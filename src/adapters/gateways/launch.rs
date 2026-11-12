//! Opening a terminal on a session: in Herdr, or in whatever terminal the
//! user is in.
//!
//! Nothing is typed into a shell if it can be avoided. A brief is long, has
//! several lines and quotes, and would have to survive a different quoting
//! in every shell; and a shell that asks something as it starts (an update,
//! a login) swallows what is typed at it. So a session becomes two small
//! scripts, and only their paths travel:
//!
//! - the *run* script goes to the directory, starts the command with its
//!   arguments intact, and leaves the user in a shell when it ends;
//! - the *entry* script is what the terminal starts instead of a shell: it
//!   starts the user's own shell, as a login and interactive one so the
//!   command finds the same `PATH` it would at a prompt, on the run script.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use serde_json::Value;

use crate::entities::handoff::AgentState;
use crate::use_cases::ports::{LaunchError, Launched, Launcher, OpenSession, Session};

/// POSIX single quotes: everything is literal, a quote is closed, escaped,
/// reopened.
fn quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// The files of one launch.
pub(super) struct Scripts {
    pub(super) entry: PathBuf,
    pub(super) run: PathBuf,
    /// Created by the entry script as it starts: proof that it did.
    started: PathBuf,
}

fn run_script(session: &Session) -> String {
    let mut text = String::from("#!/bin/sh\n");
    if let Some(directory) = &session.directory {
        text.push_str(&format!(
            "cd {} || exit 1\n",
            quote(&directory.to_string_lossy())
        ));
    }
    let line: Vec<String> = session.command.iter().map(|word| quote(word)).collect();
    text.push_str(&line.join(" "));
    // The tab does not vanish with what it showed.
    text.push_str("\nexec \"${SHELL:-/bin/sh}\"\n");
    text
}

/// `shell` is the user's own; the terminal was told this script was it.
fn entry_script(shell: &str, scripts: &Scripts) -> String {
    format!(
        "#!/bin/sh\n\
         SHELL={}\n\
         export SHELL\n\
         : > {}\n\
         exec \"$SHELL\" -l -i -c {}\n",
        quote(shell),
        quote(&scripts.started.to_string_lossy()),
        // A bare path is a command in every shell; quoted once more for the
        // one that reads this line.
        // Read by `sh`, never run by its own path: a file just written is
        // one the system may take its time to let run.
        quote(&through_sh(&scripts.run)),
    )
}

/// The command line that has `sh` read a script.
pub(super) fn through_sh(script: &Path) -> String {
    format!("/bin/sh {}", quote(&script.to_string_lossy()))
}

fn write_executable(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // The run script holds the brief: titles and links of private work.
        options.mode(0o700);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// Writes a session's scripts where only the user can read them, and
/// forgets those of past days.
pub(super) fn write_scripts(
    folder: &Path,
    shell: &str,
    session: &Session,
) -> Result<Scripts, LaunchError> {
    let failed =
        |error: std::io::Error| LaunchError::Failed(format!("{}: {error}", folder.display()));
    std::fs::create_dir_all(folder).map_err(failed)?;
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.flatten() {
            let old = entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_secs(86_400));
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    let stamp = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let scripts = Scripts {
        entry: folder.join(format!("agent-{stamp}.sh")),
        run: folder.join(format!("agent-{stamp}.run.sh")),
        started: folder.join(format!("agent-{stamp}.started")),
    };
    write_executable(&scripts.run, &run_script(session))
        .and_then(|()| write_executable(&scripts.entry, &entry_script(shell, &scripts)))
        .map_err(failed)?;
    Ok(scripts)
}

/// The user's shell, as the environment names it.
pub(super) fn users_shell() -> String {
    std::env::var("SHELL")
        .ok()
        .filter(|shell| !shell.is_empty())
        .unwrap_or_else(|| "/bin/sh".to_string())
}

// ── Herdr ──────────────────────────────────────────────────────────────

/// Runs `herdr` with these arguments and reads its JSON answer.
type Herdr<'a> = &'a dyn Fn(&[&str]) -> Result<Value, LaunchError>;

/// Workspaces are repositories, tabs are what is being worked on, both
/// named by aki and created when missing. A tab that already exists
/// under that exact name is used again rather than doubled: its agent is
/// told what there is to do now, or started if it is not running.
pub struct HerdrLauncher {
    /// Where launch scripts are written.
    pub scripts: PathBuf,
    /// A named session to drive instead of the one aki runs in.
    pub session: Option<String>,
}

impl HerdrLauncher {
    fn herdr(&self, arguments: &[&str]) -> Result<Value, LaunchError> {
        let mut command = Command::new("herdr");
        if let Some(session) = &self.session {
            command.args(["--session", session]);
        }
        let output = command
            .args(arguments)
            .stdin(Stdio::null())
            .output()
            .map_err(|error| LaunchError::Unavailable(format!("herdr cannot be run: {error}")))?;
        // Herdr answers in JSON on stdout, and on stderr when it refuses.
        let text = if output.stdout.is_empty() {
            output.stderr
        } else {
            output.stdout
        };
        // Some commands succeed without a word.
        if text.iter().all(u8::is_ascii_whitespace) && output.status.success() {
            return Ok(Value::Null);
        }
        let answer: Value = serde_json::from_slice(&text).map_err(|_| {
            LaunchError::Failed(format!(
                "herdr {}: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&text).trim()
            ))
        })?;
        match answer["error"]["message"].as_str() {
            Some(message) => Err(LaunchError::Failed(message.to_string())),
            None => Ok(answer),
        }
    }
}

fn text<'a>(value: &'a Value, what: &str) -> Result<&'a str, LaunchError> {
    value
        .as_str()
        .ok_or_else(|| LaunchError::Failed(format!("herdr did not say {what}")))
}

/// The id of the thing in `list` whose label is exactly `name`. The names
/// are aki's own: what the user already has is used only when it is
/// named precisely as aki would name it, never guessed from a likeness.
fn labelled(list: &Value, name: &str, id: &str) -> Option<String> {
    list.as_array()?
        .iter()
        .find(|entry| entry["label"].as_str() == Some(name))
        .and_then(|entry| entry[id].as_str().map(str::to_string))
}

/// How often, and how many times, a new pane is looked at.
const POLL: Duration = Duration::from_millis(100);
const START_POLLS: usize = 30;
const SHELL_POLLS: usize = 50;

/// What the conversation with Herdr needs to know about the world.
struct World<'a> {
    herdr: Herdr<'a>,
    /// Whether the entry script has started.
    started: &'a dyn Fn() -> bool,
    pause: &'a dyn Fn(),
}

/// Waits for a pane's shell to be at its prompt. Herdr counts a pane's
/// changes; a shell that has printed something and then stays still is
/// ready. Gives up quietly: typing too early is the worst that happens.
fn wait_for_shell(world: &World, pane: &str) {
    let mut last = None;
    let mut still = 0;
    for _ in 0..SHELL_POLLS {
        let revision = (world.herdr)(&["pane", "get", pane])
            .ok()
            .and_then(|answer| answer["result"]["pane"]["revision"].as_u64());
        match revision {
            // This Herdr does not say: nothing to wait on.
            None => return,
            Some(revision) if revision > 0 && last == Some(revision) => {
                still += 1;
                if still == 3 {
                    return;
                }
            }
            Some(revision) => {
                last = Some(revision);
                still = 0;
            }
        }
        (world.pause)();
    }
}

/// The brief as a paste: an agent's prompt takes it whole, new lines
/// included, instead of reading each line as a message of its own.
pub(super) fn pasted(prompt: &str) -> String {
    let safe: String = prompt
        .chars()
        // Nothing in it may end the paste early or act as a key.
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect();
    format!("\x1b[200~{safe}\x1b[201~")
}

/// Whether a pane sits at its shell's prompt: the shell itself is what
/// runs in the foreground. A Herdr that does not say is taken at its word.
fn at_prompt(info: &Value) -> bool {
    let info = &info["result"]["process_info"];
    let Some(shell) = info["shell_pid"].as_u64() else {
        return true;
    };
    info["foreground_processes"]
        .as_array()
        .is_none_or(|running| running.iter().any(|process| process["pid"] == shell))
}

/// The tab is already there. An agent running in it is told the brief;
/// without one, the agent is started in a pane that sits at its prompt.
fn reuse_tab(
    world: &World,
    workspace: &str,
    tab: &str,
    session: &Session,
    scripts: &Scripts,
) -> Result<Launched, LaunchError> {
    let herdr = world.herdr;
    let panes = herdr(&["pane", "list", "--workspace", workspace])?;
    let here: Vec<&Value> = panes["result"]["panes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|pane| pane["tab_id"].as_str() == Some(tab))
        .collect();

    if let Some(pane) = here.iter().find(|pane| pane["agent"].is_string()) {
        let pane = text(&pane["pane_id"], "which pane the agent is in")?;
        herdr(&["pane", "send-text", pane, &pasted(&session.prompt)])?;
        // The agent takes the paste in before it is sent.
        for _ in 0..3 {
            (world.pause)();
        }
        herdr(&["pane", "send-keys", pane, "Enter"])?;
        return Ok(Launched::Briefed);
    }

    for pane in &here {
        let pane = text(&pane["pane_id"], "which pane the tab has")?;
        // Typing at anything but a prompt would land in whatever runs there.
        if at_prompt(&herdr(&["pane", "process-info", "--pane", pane])?) {
            herdr(&["pane", "run", pane, &through_sh(&scripts.run)])?;
            return Ok(Launched::Restarted);
        }
    }
    Err(LaunchError::Failed(format!(
        "the tab \"{}\" is busy with something that is not an agent: \
         close what runs in it, or the tab",
        session.tab
    )))
}

fn open_in_herdr(
    world: &World,
    session: &Session,
    scripts: &Scripts,
) -> Result<Launched, LaunchError> {
    let herdr = world.herdr;
    let focus = if session.focus {
        "--focus"
    } else {
        "--no-focus"
    };
    let directory = session
        .directory
        .as_ref()
        .map(|directory| directory.to_string_lossy().into_owned());
    // The pane starts on the entry script instead of a shell: Herdr starts
    // whatever `SHELL` names in the pane's own environment.
    let instead_of_shell = format!("SHELL={}", scripts.entry.to_string_lossy());
    let mut how: Vec<&str> = Vec::new();
    if let Some(directory) = &directory {
        how.extend(["--cwd", directory]);
    }
    how.extend(["--env", &instead_of_shell]);

    let workspaces = herdr(&["workspace", "list"])?;
    let workspace = labelled(
        &workspaces["result"]["workspaces"],
        &session.workspace,
        "workspace_id",
    );

    let pane = match workspace {
        Some(workspace) => {
            let tabs = herdr(&["tab", "list", "--workspace", &workspace])?;
            if let Some(tab) = labelled(&tabs["result"]["tabs"], &session.tab, "tab_id") {
                let launched = reuse_tab(world, &workspace, &tab, session, scripts)?;
                if session.focus {
                    herdr(&["tab", "focus", &tab])?;
                }
                return Ok(launched);
            }
            let mut create = vec!["tab", "create", "--workspace", &workspace];
            create.extend(&how);
            create.extend(["--label", &session.tab, focus]);
            let created = herdr(&create)?;
            text(
                &created["result"]["root_pane"]["pane_id"],
                "which pane it opened",
            )?
            .to_string()
        }
        None => {
            let mut create = vec!["workspace", "create"];
            create.extend(&how);
            create.extend(["--label", &session.workspace, focus]);
            let created = herdr(&create)?;
            // A new workspace comes with a first tab: it is ours.
            let tab = text(&created["result"]["tab"]["tab_id"], "which tab it opened")?;
            herdr(&["tab", "rename", tab, &session.tab])?;
            text(
                &created["result"]["root_pane"]["pane_id"],
                "which pane it opened",
            )?
            .to_string()
        }
    };

    // A Herdr set to always start the same shell ignores the pane's own
    // `SHELL`: the pane is then at a prompt, and the run script is typed.
    for _ in 0..START_POLLS {
        if (world.started)() {
            return Ok(Launched::Opened);
        }
        (world.pause)();
    }
    wait_for_shell(world, &pane);
    herdr(&["pane", "run", &pane, &through_sh(&scripts.run)])?;
    Ok(Launched::Opened)
}

/// Every tab Herdr has open, with its workspace's name and the state of
/// the agent Herdr recognized in it.
fn open_in(herdr: Herdr) -> Result<Vec<OpenSession>, LaunchError> {
    let workspaces = herdr(&["workspace", "list"])?;
    let tabs = herdr(&["tab", "list"])?;
    let label = |entry: &Value| entry["label"].as_str().map(str::to_string);
    Ok(tabs["result"]["tabs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tab| {
            let workspace = workspaces["result"]["workspaces"]
                .as_array()?
                .iter()
                .find(|workspace| workspace["workspace_id"] == tab["workspace_id"])?;
            Some(OpenSession {
                workspace: label(workspace)?,
                tab: label(tab)?,
                state: match tab["agent_status"].as_str() {
                    Some("working") => AgentState::Working,
                    Some("blocked") => AgentState::Blocked,
                    Some("done") => AgentState::Done,
                    Some("idle") => AgentState::Idle,
                    _ => AgentState::Unknown,
                },
            })
        })
        .collect())
}

impl Launcher for HerdrLauncher {
    fn sessions(&self) -> Result<Vec<OpenSession>, LaunchError> {
        open_in(&|arguments| self.herdr(arguments))
    }

    fn launch(&self, session: &Session) -> Result<Launched, LaunchError> {
        let scripts = write_scripts(&self.scripts, &users_shell(), session)?;
        let world = World {
            herdr: &|arguments| self.herdr(arguments),
            started: &|| scripts.started.exists(),
            pause: &|| std::thread::sleep(POLL),
        };
        open_in_herdr(&world, session, &scripts)
    }
}

// ── the terminal itself ────────────────────────────────────────────────

/// How a terminal is asked for a new window or tab.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Terminal {
    Tmux,
    Ghostty,
    WezTerm,
    Kitty,
    ITerm,
    /// macOS's own Terminal.
    Apple,
    /// The user's own command: `{command}` is the script to run, `{title}`
    /// its name, `{directory}` where it starts.
    Custom(Vec<String>),
}

impl Terminal {
    /// The terminal aki runs in, from what it says of itself in the
    /// environment. A multiplexer inside it wins: it is where the user is.
    pub fn detect(variable: &dyn Fn(&str) -> Option<String>) -> Option<Self> {
        if variable("TMUX").is_some() {
            return Some(Terminal::Tmux);
        }
        match variable("TERM_PROGRAM").as_deref() {
            Some("ghostty") => Some(Terminal::Ghostty),
            Some("WezTerm") => Some(Terminal::WezTerm),
            Some("iTerm.app") => Some(Terminal::ITerm),
            Some("Apple_Terminal") => Some(Terminal::Apple),
            _ if variable("KITTY_WINDOW_ID").is_some() => Some(Terminal::Kitty),
            _ => None,
        }
    }

    /// The program to run, and its arguments.
    fn invocation(&self, title: &str, directory: &str, script: &str, macos: bool) -> Vec<String> {
        let words = |words: &[&str]| words.iter().map(|word| word.to_string()).collect();
        // AppleScript strings escape the backslash and the double quote.
        let scripted = script.replace('\\', "\\\\").replace('"', "\\\"");
        match self {
            Terminal::Tmux => words(&["tmux", "new-window", "-n", title, script]),
            Terminal::Ghostty if macos => words(&[
                "open",
                "-na",
                "Ghostty",
                "--args",
                &format!("--title={title}"),
                "-e",
                script,
            ]),
            Terminal::Ghostty => words(&["ghostty", &format!("--title={title}"), "-e", script]),
            Terminal::WezTerm => words(&["wezterm", "cli", "spawn", "--", script]),
            Terminal::Kitty => words(&[
                "kitty",
                "@",
                "launch",
                "--type=tab",
                "--tab-title",
                title,
                script,
            ]),
            Terminal::ITerm => words(&[
                "osascript",
                "-e",
                &format!(
                    "tell application \"iTerm\" to create window with default profile \
                     command \"{scripted}\""
                ),
            ]),
            Terminal::Apple => words(&[
                "osascript",
                "-e",
                &format!("tell application \"Terminal\" to do script \"{scripted}\""),
                "-e",
                "tell application \"Terminal\" to activate",
            ]),
            Terminal::Custom(template) => template
                .iter()
                .map(|word| {
                    word.replace("{command}", script)
                        .replace("{title}", title)
                        .replace("{directory}", directory)
                })
                .collect(),
        }
    }
}

/// Opens a window or a tab of the terminal itself. It has no workspaces:
/// the name says both, and nothing can tell whether it is already open.
pub struct TerminalLauncher {
    pub scripts: PathBuf,
    /// `None`: no terminal was recognized, and none was configured.
    pub terminal: Option<Terminal>,
}

impl Launcher for TerminalLauncher {
    fn launch(&self, session: &Session) -> Result<Launched, LaunchError> {
        let Some(terminal) = &self.terminal else {
            return Err(LaunchError::Unavailable(
                "no known terminal to open a window in: set `terminal` under [agent], \
                 for example [\"ghostty\", \"-e\", \"{command}\"]"
                    .to_string(),
            ));
        };
        let scripts = write_scripts(&self.scripts, &users_shell(), session)?;
        let directory = session
            .directory
            .as_ref()
            .map(|directory| directory.to_string_lossy().into_owned())
            .unwrap_or_default();
        let invocation = terminal.invocation(
            &format!("{} › {}", session.workspace, session.tab),
            &directory,
            &scripts.entry.to_string_lossy(),
            cfg!(target_os = "macos"),
        );
        let Some((program, arguments)) = invocation.split_first() else {
            return Err(LaunchError::Unavailable(
                "`terminal` under [agent] is empty".into(),
            ));
        };
        let status = Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| LaunchError::Failed(format!("{program}: {error}")))?;
        if status.success() {
            Ok(Launched::Opened)
        } else {
            Err(LaunchError::Failed(format!("{program} refused ({status})")))
        }
    }
}

// ── a conversation ─────────────────────────────────────────────────────

/// Percent-encodes everything an address does not take as it is.
fn encoded(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char)
            }
            other => encoded.push_str(&format!("%{other:02X}")),
        }
    }
    encoded
}

/// The address a conversation about `session` opens at.
fn chat_address(template: &str, session: &Session) -> String {
    template
        .replace("{prompt}", &encoded(&session.prompt))
        .replace(
            "{title}",
            &encoded(&format!("{} › {}", session.workspace, session.tab)),
        )
}

/// No terminal: a new Claude conversation, with the brief as its first
/// message, opened the way the machine opens any address. For whoever
/// does not live in a terminal.
pub struct ChatLauncher {
    /// `{prompt}` is the brief, `{title}` the name the work would be given.
    pub url: String,
}

impl Launcher for ChatLauncher {
    fn launch(&self, session: &Session) -> Result<Launched, LaunchError> {
        if !self.url.contains("{prompt}") {
            return Err(LaunchError::Unavailable(
                "`chat_url` under [agent] has no {prompt} to put the brief in".to_string(),
            ));
        }
        let program = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let status = Command::new(program)
            .arg(chat_address(&self.url, session))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| LaunchError::Failed(format!("{program}: {error}")))?;
        if status.success() {
            Ok(Launched::Opened)
        } else {
            Err(LaunchError::Failed(format!("{program} refused ({status})")))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::cell::{Cell, RefCell};

    fn session(focus: bool) -> Session {
        Session {
            workspace: "web".into(),
            tab: "PROJ-580 !916".into(),
            directory: Some(PathBuf::from("/code/acme/my web")),
            command: vec!["claude".into(), "It's \"here\":\n$HOME `x`".into()],
            prompt: "It's \"here\":\n$HOME `x`".into(),
            focus,
        }
    }

    fn scripts() -> Scripts {
        Scripts {
            entry: PathBuf::from("/cache/agent-1.sh"),
            run: PathBuf::from("/cache/agent-1.run.sh"),
            started: PathBuf::from("/cache/agent-1.started"),
        }
    }

    #[test]
    fn the_run_script_carries_every_argument_untouched() {
        assert_eq!(
            run_script(&session(true)),
            "#!/bin/sh\n\
             cd '/code/acme/my web' || exit 1\n\
             'claude' 'It'\\''s \"here\":\n$HOME `x`'\n\
             exec \"${SHELL:-/bin/sh}\"\n"
        );
    }

    #[test]
    fn the_entry_script_hands_over_to_the_users_own_shell() {
        assert_eq!(
            entry_script("/opt/homebrew/bin/fish", &scripts()),
            "#!/bin/sh\n\
             SHELL='/opt/homebrew/bin/fish'\n\
             export SHELL\n\
             : > '/cache/agent-1.started'\n\
             exec \"$SHELL\" -l -i -c '/bin/sh '\\''/cache/agent-1.run.sh'\\'''\n"
        );
    }

    #[cfg(unix)]
    #[test]
    fn started_in_place_of_a_shell_the_scripts_run_what_they_were_given() {
        use std::os::unix::fs::PermissionsExt;

        let folder = std::env::temp_dir().join(format!("aki launch {}", std::process::id()));
        let mut run = session(true);
        run.directory = Some(std::env::temp_dir());
        run.command = vec![
            "printf".into(),
            "%s|".into(),
            "It's \"here\":\n$HOME `x`".into(),
        ];
        let scripts = write_scripts(&folder, "/bin/sh", &run).unwrap();
        for script in [&scripts.entry, &scripts.run] {
            let mode = std::fs::metadata(script).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700);
        }
        assert!(!scripts.started.exists());

        // No terminal here: the shell left at the end reads nothing and exits.
        let output = Command::new("/bin/sh")
            .arg(&scripts.entry)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "It's \"here\":\n$HOME `x`|"
        );
        assert!(scripts.started.exists());
        let _ = std::fs::remove_dir_all(folder);
    }

    /// A Herdr that answers from a script and writes down what it was asked.
    struct Scripted {
        asked: RefCell<Vec<String>>,
        workspaces: Value,
        tabs: Value,
        /// The panes of the workspace asked about.
        panes: Value,
        /// What runs in the foreground of every pane, beside its shell (7).
        foreground: u64,
        /// Whether this Herdr starts the pane on the entry script.
        honors_shell: bool,
        /// What `pane get` says of the pane's revision, read after read.
        revisions: RefCell<Vec<u64>>,
        pauses: Cell<usize>,
    }

    impl Scripted {
        fn new(workspaces: Value, tabs: Value) -> Self {
            Self {
                asked: RefCell::new(Vec::new()),
                workspaces,
                tabs,
                panes: json!([]),
                foreground: 7,
                honors_shell: true,
                revisions: RefCell::new(Vec::new()),
                pauses: Cell::new(0),
            }
        }

        fn answer(&self, arguments: &[&str]) -> Result<Value, LaunchError> {
            if (arguments[0], arguments[1]) == ("pane", "get") {
                let mut revisions = self.revisions.borrow_mut();
                return Ok(if revisions.is_empty() {
                    json!({ "result": {} })
                } else {
                    json!({ "result": { "pane": { "revision": revisions.remove(0) } } })
                });
            }
            self.asked.borrow_mut().push(arguments.join(" "));
            Ok(match (arguments[0], arguments[1]) {
                ("workspace", "list") => json!({ "result": { "workspaces": self.workspaces } }),
                ("tab", "list") => json!({ "result": { "tabs": self.tabs } }),
                ("pane", "list") => json!({ "result": { "panes": self.panes } }),
                ("pane", "process-info") => json!({ "result": { "process_info": {
                    "shell_pid": 7,
                    "foreground_processes": [{ "pid": self.foreground }],
                } } }),
                ("workspace", "create") => json!({ "result": {
                    "workspace": { "workspace_id": "w9" },
                    "tab": { "tab_id": "w9:t1" },
                    "root_pane": { "pane_id": "w9:p1" },
                } }),
                ("tab", "create") => json!({ "result": {
                    "tab": { "tab_id": "w2:t7" },
                    "root_pane": { "pane_id": "w2:p7" },
                } }),
                _ => Value::Null,
            })
        }

        fn try_open(&self, focus: bool) -> Result<Launched, LaunchError> {
            let world = World {
                herdr: &|arguments| self.answer(arguments),
                started: &|| self.honors_shell,
                pause: &|| self.pauses.set(self.pauses.get() + 1),
            };
            open_in_herdr(&world, &session(focus), &scripts())
        }

        fn open(&self, focus: bool) -> Launched {
            self.try_open(focus).unwrap()
        }
    }

    #[test]
    fn a_new_repository_gets_its_workspace_and_its_first_tab() {
        let herdr = Scripted::new(json!([{ "label": "api", "workspace_id": "w1" }]), json!([]));
        assert_eq!(herdr.open(true), Launched::Opened);
        assert_eq!(
            *herdr.asked.borrow(),
            [
                "workspace list",
                "workspace create --cwd /code/acme/my web --env SHELL=/cache/agent-1.sh \
                 --label web --focus",
                "tab rename w9:t1 PROJ-580 !916",
            ],
            "nothing is typed: the pane starts on the script"
        );
    }

    #[test]
    fn a_known_repository_gets_a_new_tab_in_its_workspace() {
        let herdr = Scripted::new(
            json!([{ "label": "api", "workspace_id": "w1" }, { "label": "web", "workspace_id": "w2" }]),
            json!([{ "label": "claude", "tab_id": "w2:t1" }]),
        );
        assert_eq!(herdr.open(false), Launched::Opened);
        assert_eq!(
            *herdr.asked.borrow(),
            [
                "workspace list",
                "tab list --workspace w2",
                "tab create --workspace w2 --cwd /code/acme/my web --env SHELL=/cache/agent-1.sh \
                 --label PROJ-580 !916 --no-focus",
            ]
        );
    }

    #[test]
    fn only_an_exact_name_is_reused() {
        // "Web" and "web-releases" are the user's own: left alone.
        let herdr = Scripted::new(
            json!([
                { "label": "Web", "workspace_id": "w1" },
                { "label": "web-releases", "workspace_id": "w2" }
            ]),
            json!([]),
        );
        assert_eq!(herdr.open(true), Launched::Opened);
        assert!(herdr.asked.borrow()[1].starts_with("workspace create "));
    }

    fn open_tab() -> (Value, Value) {
        (
            json!([{ "label": "web", "workspace_id": "w2" }]),
            json!([{ "label": "PROJ-580 !916", "tab_id": "w2:t4" }]),
        )
    }

    #[test]
    fn an_agent_already_at_work_is_told_the_brief_in_its_own_tab() {
        let (workspaces, tabs) = open_tab();
        let mut herdr = Scripted::new(workspaces, tabs);
        herdr.panes = json!([
            { "pane_id": "w2:p1", "tab_id": "w2:t1", "agent": "claude" },
            { "pane_id": "w2:p8", "tab_id": "w2:t4" },
            { "pane_id": "w2:p9", "tab_id": "w2:t4", "agent": "claude" }
        ]);
        assert_eq!(herdr.open(true), Launched::Briefed);
        assert_eq!(
            *herdr.asked.borrow(),
            [
                "workspace list",
                "tab list --workspace w2",
                "pane list --workspace w2",
                // One paste, whatever is in it, then sent.
                "pane send-text w2:p9 \u{1b}[200~It's \"here\":\n$HOME `x`\u{1b}[201~",
                "pane send-keys w2:p9 Enter",
                "tab focus w2:t4"
            ],
            "no second tab, no second agent"
        );

        // Asked not to take the user there: told all the same, not shown.
        let (workspaces, tabs) = open_tab();
        let mut herdr = Scripted::new(workspaces, tabs);
        herdr.panes = json!([{ "pane_id": "w2:p9", "tab_id": "w2:t4", "agent": "claude" }]);
        assert_eq!(herdr.open(false), Launched::Briefed);
        assert_eq!(
            herdr.asked.borrow().last().unwrap(),
            "pane send-keys w2:p9 Enter"
        );
    }

    #[test]
    fn a_tab_left_without_its_agent_gets_it_started_again() {
        let (workspaces, tabs) = open_tab();
        let mut herdr = Scripted::new(workspaces, tabs);
        herdr.panes = json!([{ "pane_id": "w2:p8", "tab_id": "w2:t4" }]);
        assert_eq!(herdr.open(true), Launched::Restarted);
        assert_eq!(
            herdr.asked.borrow()[3..],
            [
                "pane process-info --pane w2:p8",
                "pane run w2:p8 /bin/sh '/cache/agent-1.run.sh'",
                "tab focus w2:t4"
            ]
        );
    }

    #[test]
    fn nothing_is_typed_into_a_tab_busy_with_something_else() {
        let (workspaces, tabs) = open_tab();
        let mut herdr = Scripted::new(workspaces, tabs);
        herdr.panes = json!([{ "pane_id": "w2:p8", "tab_id": "w2:t4" }]);
        // An editor, a build: not the shell.
        herdr.foreground = 99;
        assert!(matches!(
            herdr.try_open(true),
            Err(LaunchError::Failed(message)) if message.contains("PROJ-580 !916")
        ));
        assert_eq!(
            herdr.asked.borrow().last().unwrap(),
            "pane process-info --pane w2:p8"
        );
    }

    #[test]
    fn a_paste_cannot_be_ended_from_inside() {
        assert_eq!(
            pasted("a\u{1b}[201~b\r\n\tc"),
            "\u{1b}[200~a[201~b\n\tc\u{1b}[201~"
        );
    }

    #[test]
    fn a_herdr_that_starts_its_own_shell_is_typed_at_once_the_shell_has_settled() {
        let mut herdr = Scripted::new(json!([]), json!([]));
        herdr.honors_shell = false;
        // Nothing yet, the shell prints as it starts, then stays still.
        *herdr.revisions.borrow_mut() = vec![0, 2, 5, 5, 5, 5, 9];
        assert_eq!(herdr.open(true), Launched::Opened);
        assert_eq!(
            *herdr.revisions.borrow(),
            [9],
            "not read past the stillness"
        );
        assert_eq!(herdr.pauses.get(), START_POLLS + 5);
        assert_eq!(
            herdr.asked.borrow().last().unwrap(),
            "pane run w9:p1 /bin/sh '/cache/agent-1.run.sh'"
        );
    }

    #[test]
    fn open_tabs_are_listed_with_their_workspace_and_their_agents_state() {
        let herdr = Scripted::new(
            json!([
                { "label": "web", "workspace_id": "w2" },
                { "label": "api", "workspace_id": "w3" }
            ]),
            json!([
                { "label": "PROJ-580 !916", "tab_id": "w2:t1", "workspace_id": "w2",
                  "agent_status": "blocked" },
                { "label": "!12", "tab_id": "w3:t1", "workspace_id": "w3", "agent_status": "working" },
                { "label": "notes", "tab_id": "w3:t2", "workspace_id": "w3", "agent_status": "unknown" },
                { "label": "orphan", "tab_id": "w9:t1", "workspace_id": "w9", "agent_status": "idle" }
            ]),
        );
        let open = open_in(&|arguments| herdr.answer(arguments)).unwrap();
        let seen: Vec<(&str, &str, AgentState)> = open
            .iter()
            .map(|open| (open.workspace.as_str(), open.tab.as_str(), open.state))
            .collect();
        assert_eq!(
            seen,
            [
                ("web", "PROJ-580 !916", AgentState::Blocked),
                ("api", "!12", AgentState::Working),
                ("api", "notes", AgentState::Unknown),
            ]
        );
    }

    #[test]
    fn a_refusal_from_herdr_stops_everything() {
        let world = World {
            herdr: &|_| Err(LaunchError::Failed("no herdr server is running".into())),
            started: &|| true,
            pause: &|| {},
        };
        assert_eq!(
            open_in_herdr(&world, &session(true), &scripts()),
            Err(LaunchError::Failed("no herdr server is running".into()))
        );
    }

    #[test]
    fn the_terminal_is_recognized_from_its_environment() {
        let environment = |set: &'static [(&'static str, &'static str)]| {
            move |name: &str| {
                set.iter()
                    .find(|(known, _)| *known == name)
                    .map(|(_, value)| value.to_string())
            }
        };
        assert_eq!(
            Terminal::detect(&environment(&[("TERM_PROGRAM", "ghostty")])),
            Some(Terminal::Ghostty)
        );
        assert_eq!(
            Terminal::detect(&environment(&[
                ("TERM_PROGRAM", "ghostty"),
                ("TMUX", "/tmp/t,1,0")
            ])),
            Some(Terminal::Tmux),
            "the multiplexer inside the terminal is where the user is"
        );
        assert_eq!(
            Terminal::detect(&environment(&[("KITTY_WINDOW_ID", "3")])),
            Some(Terminal::Kitty)
        );
        assert_eq!(
            Terminal::detect(&environment(&[("TERM_PROGRAM", "vscode")])),
            None
        );
    }

    #[test]
    fn each_terminal_is_asked_in_its_own_words() {
        let ask = |terminal: Terminal, macos: bool| {
            terminal.invocation("web › !916", "/code/web", "/cache/a.sh", macos)
        };
        assert_eq!(
            ask(Terminal::Tmux, true),
            ["tmux", "new-window", "-n", "web › !916", "/cache/a.sh"]
        );
        assert_eq!(
            ask(Terminal::Ghostty, true),
            [
                "open",
                "-na",
                "Ghostty",
                "--args",
                "--title=web › !916",
                "-e",
                "/cache/a.sh"
            ]
        );
        assert_eq!(
            ask(Terminal::Ghostty, false),
            ["ghostty", "--title=web › !916", "-e", "/cache/a.sh"]
        );
        assert_eq!(
            ask(Terminal::Apple, true)[2],
            "tell application \"Terminal\" to do script \"/cache/a.sh\""
        );
        let custom = Terminal::Custom(vec![
            "alacritty".into(),
            "--title".into(),
            "{title}".into(),
            "--working-directory={directory}".into(),
            "-e".into(),
            "{command}".into(),
        ]);
        assert_eq!(
            ask(custom, false),
            [
                "alacritty",
                "--title",
                "web › !916",
                "--working-directory=/code/web",
                "-e",
                "/cache/a.sh"
            ]
        );
    }

    #[test]
    fn a_conversation_opens_on_the_brief_whatever_is_in_it() {
        assert_eq!(
            chat_address("https://claude.ai/new?q={prompt}", &session(true)),
            "https://claude.ai/new?q=It%27s%20%22here%22%3A%0A%24HOME%20%60x%60"
        );
        assert_eq!(
            chat_address("app://new?t={title}", &session(true)),
            "app://new?t=web%20%E2%80%BA%20PROJ-580%20%21916"
        );
        let launcher = ChatLauncher {
            url: "https://claude.ai/new".into(),
        };
        assert!(matches!(
            launcher.launch(&session(true)),
            Err(LaunchError::Unavailable(message)) if message.contains("{prompt}")
        ));
    }

    #[test]
    fn without_a_terminal_the_user_is_told_what_to_set() {
        let launcher = TerminalLauncher {
            scripts: PathBuf::from("/nowhere"),
            terminal: None,
        };
        assert!(matches!(
            launcher.launch(&session(true)),
            Err(LaunchError::Unavailable(message)) if message.contains("[agent]")
        ));
    }
}
