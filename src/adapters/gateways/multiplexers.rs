//! Opening an agent in tmux or in TUIOS, the way it is done in Herdr: a
//! place per repository, a named terminal per piece of work, both created
//! when missing and used again when they bear exactly that name.
//!
//! |            | repository       | piece of work |
//! |------------|------------------|---------------|
//! | tmux       | a session        | a window      |
//! | TUIOS      | a named workspace | a named window |
//!
//! A terminal that is already there is never doubled: its agent is told
//! the new brief, or started again in it if it is gone, and nothing is
//! typed into a terminal busy with something else.

use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

use super::launch::{Scripts, pasted, through_sh, users_shell, write_scripts};
use crate::entities::handoff::AgentState;
use crate::use_cases::ports::{LaunchError, Launched, Launcher, OpenSession, Session};

/// Runs the multiplexer's own command with these arguments, and returns
/// what it printed.
type Run<'a> = &'a dyn Fn(&[&str]) -> Result<String, LaunchError>;

fn run(program: &str, arguments: &[&str]) -> Result<String, LaunchError> {
    let output = Command::new(program)
        .args(arguments)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| LaunchError::Unavailable(format!("{program} cannot be run: {error}")))?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let said = if output.stderr.is_empty() {
            &output.stdout
        } else {
            &output.stderr
        };
        Err(LaunchError::Failed(format!(
            "{program} {}: {}",
            arguments.first().copied().unwrap_or_default(),
            String::from_utf8_lossy(said).trim()
        )))
    }
}

/// Whether a program's name is a shell's: a terminal showing one sits at
/// its prompt.
fn is_shell(program: &str) -> bool {
    let name = program.rsplit('/').next().unwrap_or(program);
    let name = name.trim_start_matches('-');
    matches!(
        name,
        "sh" | "bash" | "zsh" | "fish" | "dash" | "ksh" | "nu" | "elvish" | "xonsh" | "tcsh"
    )
}

/// The agent's own name: the last part of the first word of its command.
fn agent_name(session: &Session) -> &str {
    session
        .command
        .first()
        .map(|program| program.rsplit('/').next().unwrap_or(program))
        .unwrap_or_default()
}

fn busy(tab: &str) -> LaunchError {
    LaunchError::Failed(format!(
        "\"{tab}\" is busy with something that is not an agent: close what runs in it, or the \
         terminal itself"
    ))
}

// ── tmux ───────────────────────────────────────────────────────────────

/// One pane, as `list-panes` tells it.
struct Pane {
    session: String,
    window: String,
    window_id: String,
    pane_id: String,
    /// What runs in the foreground.
    command: String,
    tty: String,
}

const PANES: &str = "#{session_name}\t#{window_name}\t#{window_id}\t#{pane_id}\t#{pane_current_command}\t#{pane_tty}";

fn panes(tmux: Run) -> Vec<Pane> {
    // No server: nothing is open.
    let listed = tmux(&["list-panes", "-a", "-F", PANES]).unwrap_or_default();
    listed
        .lines()
        .filter_map(|line| {
            let mut fields = line.split('\t').map(str::to_string);
            Some(Pane {
                session: fields.next()?,
                window: fields.next()?,
                window_id: fields.next()?,
                pane_id: fields.next()?,
                command: fields.next()?,
                tty: fields.next()?,
            })
        })
        .collect()
}

/// tmux does not take a dot or a colon in a session's name.
fn session_name(workspace: &str) -> String {
    workspace.replace(['.', ':'], "_")
}

/// Given as two words, a window's command is run as it is. Given as one,
/// tmux hands it to the user's shell first, and a login shell started under
/// that one never gets to its prompt.
const DIRECT: [&str; 1] = ["/bin/sh"];

/// What tmux needs to know about the world besides itself.
struct TmuxWorld<'a> {
    tmux: Run<'a>,
    /// The command lines of what runs on a terminal.
    running_on: &'a dyn Fn(&str) -> String,
    /// Whether aki itself runs inside tmux: only then can it take the user
    /// to another session.
    inside: bool,
}

fn open_in_tmux(
    world: &TmuxWorld,
    session: &Session,
    scripts: &Scripts,
) -> Result<Launched, LaunchError> {
    let tmux = world.tmux;
    let name = session_name(&session.workspace);
    let all = panes(tmux);
    let entry = scripts.entry.to_string_lossy();
    let directory = session
        .directory
        .as_ref()
        .map(|directory| directory.to_string_lossy().into_owned());
    let mut place: Vec<&str> = vec!["-n", &session.tab];
    if let Some(directory) = &directory {
        place.extend(["-c", directory]);
    }

    let here: Vec<&Pane> = all
        .iter()
        .filter(|pane| pane.session == name && pane.window == session.tab)
        .collect();
    let (window, launched) = if let Some(first) = here.first() {
        let agent = agent_name(session);
        let with_agent = here.iter().find(|pane| {
            !agent.is_empty()
                && (world.running_on)(&pane.tty).lines().any(|line| {
                    line.split_whitespace()
                        .any(|word| word.rsplit('/').next() == Some(agent))
                })
        });
        if let Some(pane) = with_agent {
            // One paste, bracketed when the agent asks for it, then sent.
            tmux(&["set-buffer", "-b", "aki-brief", "--", &session.prompt])?;
            tmux(&[
                "paste-buffer",
                "-p",
                "-d",
                "-b",
                "aki-brief",
                "-t",
                &pane.pane_id,
            ])?;
            tmux(&["send-keys", "-t", &pane.pane_id, "Enter"])?;
            (first.window_id.clone(), Launched::Briefed)
        } else if let Some(pane) = here.iter().find(|pane| is_shell(&pane.command)) {
            tmux(&[
                "send-keys",
                "-t",
                &pane.pane_id,
                "-l",
                &through_sh(&scripts.run),
            ])?;
            tmux(&["send-keys", "-t", &pane.pane_id, "Enter"])?;
            (first.window_id.clone(), Launched::Restarted)
        } else {
            return Err(busy(&session.tab));
        }
    } else if all.iter().any(|pane| pane.session == name) {
        // `=name:` is that session exactly, never one that starts alike.
        let target = format!("={name}:");
        let mut create = vec![
            "new-window",
            "-d",
            "-P",
            "-F",
            "#{window_id}",
            "-t",
            &target,
        ];
        create.extend(&place);
        create.extend(DIRECT);
        create.push(&entry);
        (tmux(&create)?.trim().to_string(), Launched::Opened)
    } else {
        let mut create = vec!["new-session", "-d", "-P", "-F", "#{window_id}", "-s", &name];
        create.extend(&place);
        create.extend(DIRECT);
        create.push(&entry);
        (tmux(&create)?.trim().to_string(), Launched::Opened)
    };

    if session.focus {
        tmux(&["select-window", "-t", &window])?;
        if world.inside {
            tmux(&["switch-client", "-t", &format!("={name}")])?;
        }
    }
    Ok(launched)
}

/// tmux: a session per repository, a window per piece of work.
pub struct TmuxLauncher {
    /// Where launch scripts are written.
    pub scripts: PathBuf,
}

impl Launcher for TmuxLauncher {
    fn launch(&self, session: &Session) -> Result<Launched, LaunchError> {
        let scripts = write_scripts(&self.scripts, &users_shell(), session)?;
        let world = TmuxWorld {
            tmux: &|arguments| run("tmux", arguments),
            running_on: &|tty| {
                let tty = tty.trim_start_matches("/dev/");
                run("ps", &["-t", tty, "-o", "command="]).unwrap_or_default()
            },
            inside: std::env::var_os("TMUX").is_some(),
        };
        open_in_tmux(&world, session, &scripts)
    }

    fn sessions(&self) -> Result<Vec<OpenSession>, LaunchError> {
        let mut open: Vec<OpenSession> = Vec::new();
        for pane in panes(&|arguments| run("tmux", arguments)) {
            let known = open
                .iter()
                .any(|session| session.workspace == pane.session && session.tab == pane.window);
            if !known {
                open.push(OpenSession {
                    workspace: pane.session,
                    tab: pane.window,
                    // tmux knows a window is there, not what its agent does.
                    state: AgentState::Unknown,
                });
            }
        }
        Ok(open)
    }
}

// ── TUIOS ──────────────────────────────────────────────────────────────

fn json(text: &str, what: &str) -> Result<Value, LaunchError> {
    let answer: Value = serde_json::from_str(text)
        .map_err(|_| LaunchError::Failed(format!("tuios {what}: {}", text.trim())))?;
    if answer["success"] == false {
        let message = answer["message"].as_str().unwrap_or("refused");
        return Err(LaunchError::Failed(format!("tuios {what}: {message}")));
    }
    Ok(answer)
}

fn state(name: &str) -> AgentState {
    match name {
        "working" => AgentState::Working,
        // An agent that failed waits for someone as much as one that asks.
        "needs_input" | "errored" => AgentState::Blocked,
        "done" => AgentState::Done,
        "idle" => AgentState::Idle,
        _ => AgentState::Unknown,
    }
}

fn open_in_tuios(
    tuios: Run,
    pause: &dyn Fn(),
    session: &Session,
    scripts: &Scripts,
) -> Result<Launched, LaunchError> {
    // The workspace named after the repository, or a free one given its name.
    let listed = json(&tuios(&["list-workspaces", "--json"])?, "list-workspaces")?;
    let workspaces = listed["workspaces"].as_array().cloned().unwrap_or_default();
    let number = |workspace: &Value| workspace["workspace"].as_u64().map(|n| n.to_string());
    let named = workspaces
        .iter()
        .find(|workspace| workspace["name"].as_str() == Some(&session.workspace))
        .and_then(number);
    let workspace = match named {
        Some(workspace) => workspace,
        None => {
            let free = workspaces
                .iter()
                .find(|workspace| {
                    workspace["name"].as_str().unwrap_or_default().is_empty()
                        && workspace["window_count"].as_u64() == Some(0)
                })
                .and_then(number)
                .ok_or_else(|| {
                    LaunchError::Failed(format!(
                        "no free TUIOS workspace for \"{}\": empty one, or clear its name",
                        session.workspace
                    ))
                })?;
            tuios(&["set-workspace-name", &free, &session.workspace])?;
            free
        }
    };

    let windows = json(&tuios(&["list-windows", "--json"])?, "list-windows")?;
    let existing = windows["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|window| {
            window["custom_name"].as_str() == Some(&session.tab)
                && window["workspace"]
                    .as_u64()
                    .map(|n| n.to_string())
                    .as_deref()
                    == Some(&workspace)
        })
        .and_then(|window| window["window_id"].as_str().map(str::to_string));

    let (window, launched) = match existing {
        Some(window) => {
            let seen = json(
                &tuios(&["explain-agent-detect", "-w", &window, "--json"])?,
                "explain-agent-detect",
            )?;
            let agent = seen["matched"] == true
                || seen["state"].as_str().is_some_and(|state| state != "none");
            let at_prompt = seen["process"]["comm"].as_str().is_some_and(is_shell);
            if agent {
                tuios(&["send-text", "-w", &window, &pasted(&session.prompt)])?;
                // The agent takes the paste in before it is sent.
                for _ in 0..3 {
                    pause();
                }
                tuios(&["send-text", "-w", &window, "\r"])?;
                (window, Launched::Briefed)
            } else if at_prompt {
                let line = format!("{}\n", through_sh(&scripts.run));
                tuios(&["send-text", "-w", &window, &line])?;
                (window, Launched::Restarted)
            } else {
                return Err(busy(&session.tab));
            }
        }
        None => {
            let directory = session
                .directory
                .as_ref()
                .map(|directory| directory.to_string_lossy().into_owned());
            let entry = scripts.entry.to_string_lossy();
            let mut create = vec!["new-window", &session.tab, "--workspace", &workspace];
            if let Some(directory) = &directory {
                create.extend(["--cwd", directory]);
            }
            // Focus is given below, once, when asked for.
            create.extend(["--no-focus", "--json", "--", "/bin/sh", &entry]);
            let created = json(&tuios(&create)?, "new-window")?;
            let window = created["window_id"].as_str().ok_or_else(|| {
                LaunchError::Failed("tuios did not say which window it opened".into())
            })?;
            (window.to_string(), Launched::Opened)
        }
    };

    if session.focus {
        tuios(&["select-workspace", &workspace])?;
        tuios(&["focus-window", &window])?;
    }
    Ok(launched)
}

/// Every named window on a named workspace, with its agent's state.
fn open_in(tuios: Run) -> Result<Vec<OpenSession>, LaunchError> {
    let workspaces = json(&tuios(&["list-workspaces", "--json"])?, "list-workspaces")?;
    let windows = json(&tuios(&["list-windows", "--json"])?, "list-windows")?;
    Ok(windows["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|window| {
            let tab = window["custom_name"]
                .as_str()
                .filter(|name| !name.is_empty())?;
            let workspace = workspaces["workspaces"]
                .as_array()?
                .iter()
                .find(|workspace| workspace["workspace"] == window["workspace"])?["name"]
                .as_str()
                .filter(|name| !name.is_empty())?;
            Some(OpenSession {
                workspace: workspace.to_string(),
                tab: tab.to_string(),
                state: state(window["agent_state"].as_str().unwrap_or_default()),
            })
        })
        .collect())
}

/// TUIOS: a named workspace per repository, a named window per piece of work.
pub struct TuiosLauncher {
    pub scripts: PathBuf,
    /// The session to open agents in; without it, the one TUIOS says aki
    /// runs in, else its most recently active one.
    pub session: Option<String>,
}

impl TuiosLauncher {
    fn tuios(&self, arguments: &[&str]) -> Result<String, LaunchError> {
        let named = self
            .session
            .clone()
            .or_else(|| std::env::var("TUIOS_SESSION").ok());
        let mut all: Vec<&str> = arguments.to_vec();
        if let Some(session) = &named {
            // Before `--`, after which everything is the window's command.
            let at = all
                .iter()
                .position(|word| *word == "--")
                .unwrap_or(all.len());
            all.splice(at..at, ["-s", session.as_str()]);
        }
        run("tuios", &all)
    }
}

impl Launcher for TuiosLauncher {
    fn launch(&self, session: &Session) -> Result<Launched, LaunchError> {
        let scripts = write_scripts(&self.scripts, &users_shell(), session)?;
        open_in_tuios(
            &|arguments| self.tuios(arguments),
            &|| std::thread::sleep(std::time::Duration::from_millis(100)),
            session,
            &scripts,
        )
    }

    fn sessions(&self) -> Result<Vec<OpenSession>, LaunchError> {
        open_in(&|arguments| self.tuios(arguments))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json as value;
    use std::cell::RefCell;

    fn session(focus: bool) -> Session {
        Session {
            workspace: "web".into(),
            tab: "PROJ-580 !916".into(),
            directory: Some(PathBuf::from("/code/acme/web")),
            command: vec!["/opt/bin/claude".into(), "Review it".into()],
            prompt: "Review it.\nStart with the diff.".into(),
            focus,
        }
    }

    static CALLS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

    fn scripts() -> Scripts {
        write_scripts(
            // A folder of its own for each call: tests run side by side.
            &std::env::temp_dir().join(format!(
                "aki mux {} {}",
                std::process::id(),
                CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )),
            "/bin/sh",
            &session(false),
        )
        .unwrap()
    }

    /// A tmux that lists these panes and writes down what it is asked.
    struct Tmux {
        panes: &'static str,
        on_tty: &'static str,
        asked: RefCell<Vec<String>>,
    }

    impl Tmux {
        fn open(&self, focus: bool, inside: bool) -> Result<Launched, LaunchError> {
            let scripts = scripts();
            let world = TmuxWorld {
                tmux: &|arguments| {
                    if arguments[0] == "list-panes" {
                        return Ok(self.panes.to_string());
                    }
                    // Paths differ from run to run: keep the words that do not.
                    let said: Vec<&str> = arguments
                        .iter()
                        .map(|word| {
                            if word.contains("agent-") {
                                "<script>"
                            } else {
                                word
                            }
                        })
                        .collect();
                    self.asked.borrow_mut().push(said.join(" "));
                    Ok("@7\n".to_string())
                },
                running_on: &|_| self.on_tty.to_string(),
                inside,
            };
            open_in_tmux(&world, &session(focus), &scripts)
        }
    }

    fn tmux(panes: &'static str, on_tty: &'static str) -> Tmux {
        Tmux {
            panes,
            on_tty,
            asked: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn in_tmux_a_repository_is_a_session_and_a_piece_of_work_a_window() {
        // Nothing yet: the session is made with its first window.
        let fresh = tmux("", "");
        assert_eq!(fresh.open(false, true), Ok(Launched::Opened));
        assert_eq!(
            *fresh.asked.borrow(),
            [
                "new-session -d -P -F #{window_id} -s web -n PROJ-580 !916 -c /code/acme/web /bin/sh <script>"
            ]
        );

        // The session is there: one more window in it, and the user is taken to it.
        let known = tmux("web\tclaude\t@1\t%1\tzsh\t/dev/ttys001\n", "");
        assert_eq!(known.open(true, true), Ok(Launched::Opened));
        assert_eq!(
            *known.asked.borrow(),
            [
                "new-window -d -P -F #{window_id} -t =web: -n PROJ-580 !916 -c /code/acme/web /bin/sh <script>",
                "select-window -t @7",
                "switch-client -t =web"
            ]
        );

        // "web-releases" is the user's own: left alone.
        let alike = tmux(
            "web-releases\tPROJ-580 !916\t@1\t%1\tzsh\t/dev/ttys001\n",
            "",
        );
        alike.open(false, false).unwrap();
        assert!(alike.asked.borrow()[0].starts_with("new-session "));
        assert_eq!(session_name("my.repo:x"), "my_repo_x");
    }

    #[test]
    fn in_tmux_a_window_already_there_is_told_or_restarted_never_doubled() {
        const THERE: &str = "web\tPROJ-580 !916\t@4\t%9\tnode\t/dev/ttys004\n";
        let working = tmux(THERE, "-zsh\n/opt/bin/claude --resume\n");
        assert_eq!(working.open(true, false), Ok(Launched::Briefed));
        assert_eq!(
            *working.asked.borrow(),
            [
                "set-buffer -b aki-brief -- Review it.\nStart with the diff.",
                "paste-buffer -p -d -b aki-brief -t %9",
                "send-keys -t %9 Enter",
                // Outside tmux, the window is selected; nobody can be taken there.
                "select-window -t @4"
            ]
        );

        let left = tmux(
            "web\tPROJ-580 !916\t@4\t%9\tfish\t/dev/ttys004\n",
            "-fish\n",
        );
        assert_eq!(left.open(false, true), Ok(Launched::Restarted));
        assert_eq!(
            *left.asked.borrow(),
            ["send-keys -t %9 -l <script>", "send-keys -t %9 Enter"]
        );

        let editing = tmux(
            "web\tPROJ-580 !916\t@4\t%9\tvim\t/dev/ttys004\n",
            "vim notes.md\n",
        );
        assert!(matches!(
            editing.open(true, true),
            Err(LaunchError::Failed(message)) if message.contains("PROJ-580 !916")
        ));
        assert!(editing.asked.borrow().is_empty(), "nothing is typed");
    }

    /// A TUIOS that answers from what it is given.
    struct Tuios {
        workspaces: Value,
        windows: Value,
        detect: Value,
        asked: RefCell<Vec<String>>,
    }

    impl Tuios {
        fn answer(&self, arguments: &[&str]) -> Result<String, LaunchError> {
            let reply = match arguments[0] {
                "list-workspaces" => value!({ "success": true, "workspaces": self.workspaces }),
                "list-windows" => value!({ "success": true, "windows": self.windows }),
                "explain-agent-detect" => self.detect.clone(),
                other => {
                    let said: Vec<&str> = arguments
                        .iter()
                        .map(
                            |word| match (word.contains("agent-"), word.ends_with('\n')) {
                                (true, true) => "<script>\n",
                                (true, false) => "<script>",
                                _ => word,
                            },
                        )
                        .collect();
                    self.asked.borrow_mut().push(said.join(" "));
                    if other == "new-window" {
                        value!({ "success": true, "window_id": "w-new" })
                    } else {
                        return Ok(String::new());
                    }
                }
            };
            Ok(reply.to_string())
        }

        fn open(&self, focus: bool) -> Result<Launched, LaunchError> {
            open_in_tuios(
                &|arguments| self.answer(arguments),
                &|| {},
                &session(focus),
                &scripts(),
            )
        }
    }

    fn workspaces() -> Value {
        value!([
            { "workspace": 1, "name": "", "window_count": 2 },
            { "workspace": 2, "name": "api", "window_count": 0 },
            { "workspace": 3, "name": "", "window_count": 0 }
        ])
    }

    #[test]
    fn in_tuios_a_repository_takes_a_free_workspace_and_names_it() {
        let tuios = Tuios {
            workspaces: workspaces(),
            windows: value!([]),
            detect: Value::Null,
            asked: RefCell::new(Vec::new()),
        };
        assert_eq!(tuios.open(true), Ok(Launched::Opened));
        assert_eq!(
            *tuios.asked.borrow(),
            [
                // Not 1 (in use), not 2 (someone else's name): the first free one.
                "set-workspace-name 3 web",
                "new-window PROJ-580 !916 --workspace 3 --cwd /code/acme/web --no-focus --json -- /bin/sh <script>",
                "select-workspace 3",
                "focus-window w-new"
            ]
        );

        let full = Tuios {
            workspaces: value!([{ "workspace": 1, "name": "", "window_count": 2 }]),
            windows: value!([]),
            detect: Value::Null,
            asked: RefCell::new(Vec::new()),
        };
        assert!(matches!(
            full.open(true),
            Err(LaunchError::Failed(message)) if message.contains("no free TUIOS workspace")
        ));
    }

    #[test]
    fn in_tuios_a_window_already_there_is_told_or_restarted_never_doubled() {
        let there = |detect: Value| Tuios {
            workspaces: value!([{ "workspace": 4, "name": "web", "window_count": 1 }]),
            windows: value!([
                { "window_id": "w-other", "custom_name": "PROJ-580 !916", "workspace": 1 },
                { "window_id": "w-9", "custom_name": "PROJ-580 !916", "workspace": 4 }
            ]),
            detect,
            asked: RefCell::new(Vec::new()),
        };

        let working = there(
            value!({ "success": true, "matched": true, "state": "working",
            "process": { "comm": "node" } }),
        );
        assert_eq!(working.open(false), Ok(Launched::Briefed));
        assert_eq!(
            *working.asked.borrow(),
            [
                "send-text -w w-9 \u{1b}[200~Review it.\nStart with the diff.\u{1b}[201~",
                "send-text -w w-9 \r"
            ]
        );

        let left = there(value!({ "success": true, "matched": false, "state": "none",
            "process": { "comm": "zsh" } }));
        assert_eq!(left.open(false), Ok(Launched::Restarted));
        assert_eq!(*left.asked.borrow(), ["send-text -w w-9 <script>\n"]);

        let editing = there(value!({ "success": true, "matched": false, "state": "none",
            "process": { "comm": "vim" } }));
        assert!(editing.open(true).is_err());
        assert!(editing.asked.borrow().is_empty());
    }

    #[test]
    fn open_windows_are_listed_by_the_names_aki_gave() {
        let tuios = Tuios {
            workspaces: value!([
                { "workspace": 1, "name": "" },
                { "workspace": 4, "name": "web" }
            ]),
            windows: value!([
                { "custom_name": "PROJ-580 !916", "workspace": 4, "agent_state": "needs_input" },
                { "custom_name": "!12", "workspace": 4, "agent_state": "none" },
                { "custom_name": "notes", "workspace": 1, "agent_state": "working" },
                { "custom_name": "", "workspace": 4, "agent_state": "idle" }
            ]),
            detect: Value::Null,
            asked: RefCell::new(Vec::new()),
        };
        let open = open_in(&|arguments| tuios.answer(arguments)).unwrap();
        let seen: Vec<(&str, &str, AgentState)> = open
            .iter()
            .map(|open| (open.workspace.as_str(), open.tab.as_str(), open.state))
            .collect();
        assert_eq!(
            seen,
            [
                ("web", "PROJ-580 !916", AgentState::Blocked),
                ("web", "!12", AgentState::Unknown)
            ]
        );
        assert_eq!(state("done"), AgentState::Done);
        assert!(is_shell("-zsh") && is_shell("/bin/bash") && !is_shell("node"));
    }
}
