//! The composition root: the only place that knows every ring.

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::Duration;

use crate::adapters::config::{self, AgentConfig, Config, LauncherChoice, SourceConfig};
use crate::adapters::controllers::cli::{self, Command};
use crate::adapters::controllers::mcp::{self, Allowed, Aside, Call};
use crate::adapters::gateways::cache::Cache;
use crate::adapters::gateways::cached::CachedSource;
use crate::adapters::gateways::checkouts::LocalCheckouts;
use crate::adapters::gateways::desk_file::DeskFileStore;
use crate::adapters::gateways::github::GitHub;
use crate::adapters::gateways::gitlab::GitLab;
use crate::adapters::gateways::jira::Jira;
use crate::adapters::gateways::launch::{ChatLauncher, HerdrLauncher, Terminal, TerminalLauncher};
use crate::adapters::gateways::multiplexers::{TmuxLauncher, TuiosLauncher};
use crate::adapters::gateways::snooze_file::SnoozeFile;
use crate::adapters::gateways::sound::{self, SoundNotifier, Together};
use crate::adapters::gateways::system::{DesktopNotifier, SystemClock, TimerPacer, open_url};
use crate::adapters::presenters::tui::{self, ChannelView, Update, Wiring};
use crate::adapters::presenters::{brief, json, text, wording};
use crate::entities::board::Board;
use crate::entities::desk::Desk;
use crate::entities::filter::Filter;
use crate::entities::snooze::{Snooze, Until};
use crate::entities::task::{Bucket, Task};
use crate::entities::time::Timestamp;
use crate::use_cases::check_sources::check_sources;
use crate::use_cases::collect_board::CollectBoard;
use crate::use_cases::launch_agent::LaunchAgent;
use crate::use_cases::open_desk::open_desk;
use crate::use_cases::ports::{
    Clock, DeskStore, Launched, Launcher, Notifier, Session, WorkSource,
};
use crate::use_cases::set_aside::set_aside;
use crate::use_cases::watch_board::WatchBoard;

/// Whether a program is on the PATH. Looked up, not run: CLIs are slow to start.
fn installed(cli: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|folder| folder.join(cli).is_file()))
}

/// The SQLite file everything already read is kept in. When it cannot be
/// opened, a cache that lasts as long as the process does the job.
fn open_cache() -> Arc<Cache> {
    Arc::new(Cache::open(&config::cache_path()).unwrap_or_else(|_| Cache::in_memory()))
}

/// The sources, each remembering its last reading. `fresh_for` is how old
/// that reading may be to stand for a new one.
fn connect(
    sources: Vec<SourceConfig>,
    cache: &Arc<Cache>,
    fresh_for: Duration,
) -> Vec<Box<dyn WorkSource>> {
    sources
        .into_iter()
        .map(|source| -> Box<dyn WorkSource> {
            let source: Box<dyn WorkSource> = match source {
                SourceConfig::GitLab {
                    name,
                    host,
                    credential,
                } => Box::new(GitLab::new(name, host, credential, Some(cache.clone()))),
                SourceConfig::GitHub {
                    name,
                    host,
                    credential,
                } => Box::new(GitHub::new(name, host, credential, cache.clone())),
                SourceConfig::Jira {
                    name,
                    site,
                    email,
                    jql,
                    credential,
                } => Box::new(Jira::new(
                    name,
                    site,
                    email,
                    jql,
                    credential,
                    Some(cache.clone()),
                )),
            };
            Box::new(CachedSource::new(source, cache.clone(), fresh_for))
        })
        .collect()
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("aki: {message}");
    ExitCode::from(2)
}

pub fn run(arguments: &[String]) -> ExitCode {
    let interactive = std::io::stdout().is_terminal();
    let command = match cli::parse(arguments, interactive) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("aki: {message}\n\n{}", cli::HELP);
            return ExitCode::from(2);
        }
    };

    match command {
        Command::Help => {
            print!("{}", cli::HELP);
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("aki {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::ConfigPath => {
            println!("{}", config::path().display());
            return ExitCode::SUCCESS;
        }
        Command::ConfigInit => return init_config(),
        Command::McpSetup => {
            let program = std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "aki".to_string());
            print!("{}", mcp::setup(&program));
            return ExitCode::SUCCESS;
        }
        // An assistant started from the Dock hands down a bare PATH.
        Command::Mcp => widen_path(),
        Command::CachePath => {
            println!("{}", config::cache_path().display());
            return ExitCode::SUCCESS;
        }
        Command::CacheClear => {
            return match Cache::open(&config::cache_path()) {
                Ok(cache) => {
                    println!("forgot {} cached entries", cache.clear());
                    ExitCode::SUCCESS
                }
                Err(error) => fail(format!("{}: {error}", config::cache_path().display())),
            };
        }
        _ => {}
    }

    let config = match Config::load(&installed) {
        Ok(config) => config,
        Err(error) => return fail(format!("{}: {error}", config::path().display())),
    };
    if let Command::Sounds { play } = &command {
        return sounds(&config, play.as_deref());
    }
    if let Command::Rules = command {
        print!("{}", text::rules(&config.rules));
        return ExitCode::SUCCESS;
    }
    if config.sources.is_empty() {
        return fail(format!(
            "no source configured. Sign in with `glab auth login` or `gh auth login`, \
             or add [[sources]] to {} (`aki config init` writes a template).",
            config::path().display()
        ));
    }

    match command {
        Command::Doctor => doctor(config),
        Command::List {
            json,
            count,
            only,
            space,
            filter,
            sort,
            reversed,
            fresh,
        } => {
            let keep = match scope(&config, space.as_deref(), filter.as_deref()) {
                Ok(keep) => keep,
                Err(message) => return fail(message),
            };
            let show = Show {
                as_json: json,
                count,
                only,
                keep,
                order: text::Order { sort, reversed },
                fresh,
                color: interactive,
            };
            list(config, show)
        }
        Command::Spaces => spaces(config),
        Command::SpacesReset => reset_spaces(&config),
        Command::Mcp => serve_assistant(config),
        Command::Agent { task, dry_run } => agent(config, &task, dry_run),
        Command::Watch => watch(config),
        _ => unreachable!("handled above"),
    }
}

fn init_config() -> ExitCode {
    let path = config::path();
    if path.exists() {
        return fail(format!(
            "{} already exists; not overwriting it",
            path.display()
        ));
    }
    let written = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| std::fs::write(&path, config::TEMPLATE));
    match written {
        Ok(()) => {
            println!("wrote {}", path.display());
            ExitCode::SUCCESS
        }
        Err(error) => fail(format!("cannot write {}: {error}", path.display())),
    }
}

fn doctor(config: Config) -> ExitCode {
    let sources = connect(config.sources, &open_cache(), Duration::ZERO);
    let mut healthy = true;
    for check in check_sources(&sources) {
        match check.outcome {
            Ok(identity) => println!("✓ {:<10} signed in as {identity}", check.source),
            Err(error) => {
                healthy = false;
                println!("✗ {:<10} {error}", check.source);
            }
        }
    }
    if healthy {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn desk(config: &Config) -> Result<Desk, String> {
    let store = DeskFileStore {
        path: config::spaces_path(),
    };
    let sources: Vec<String> = config
        .sources
        .iter()
        .map(|source| source.name().to_string())
        .collect();
    open_desk(&store, &sources).map_err(|error| error.to_string())
}

/// What `--space` and `--filter` keep: the space's scope and the query.
fn scope(
    config: &Config,
    space: Option<&str>,
    filter: Option<&str>,
) -> Result<Vec<Filter>, String> {
    let mut keep = Vec::new();
    if let Some(name) = space {
        let desk = desk(config)?;
        let found = desk
            .spaces()
            .iter()
            .find(|space| space.name.eq_ignore_ascii_case(name))
            .ok_or_else(|| {
                let names: Vec<&str> = desk.spaces().iter().map(|s| s.name.as_str()).collect();
                format!("no space \"{name}\" (yours: {})", names.join(", "))
            })?;
        keep.push(found.scope.clone());
    }
    if let Some(query) = filter {
        keep.push(Filter::parse(query).map_err(|error| error.to_string())?);
    }
    Ok(keep)
}

/// Where the tools installed by hand live, for when the PATH handed down
/// does not say: the forges' CLIs hold the tokens.
fn widen_path() {
    let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
    let mut folders: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    for extra in [
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
        home.join(".local/bin"),
        home.join(".cargo/bin"),
    ] {
        if extra.is_dir() && !folders.contains(&extra) {
            folders.push(extra);
        }
    }
    if let Ok(path) = std::env::join_paths(folders) {
        // SAFETY: nothing else runs yet; no thread reads the environment.
        unsafe { std::env::set_var("PATH", path) };
    }
}

/// `aki spaces`: where the spaces come from, how each is made, and the
/// repositories it holds right now.
fn spaces(config: Config) -> ExitCode {
    let desk = match desk(&config) {
        Ok(desk) => desk,
        Err(message) => return fail(message),
    };
    let path = config::spaces_path();
    if path.exists() {
        println!(
            "Your spaces are kept in {}.\n\
             They are what you made them: every change in the board (N new space, S scope,\n\
             / filter, g group, s sort, t layout…) is saved there, and the file can be edited\n\
             by hand. `aki spaces reset` starts over from your sources.\n",
            path.display()
        );
    } else {
        println!(
            "No spaces were saved yet ({} does not exist): these are the starting ones,\n\
             \"All\" and one space per source when there are several.\n",
            path.display()
        );
    }
    let (board, _) = read_board(&config, config.cache);
    print!("{}", text::spaces(&desk, Some(&board)));
    eprint!("{}", text::failures(&board, SystemClock.now()));
    ExitCode::SUCCESS
}

/// `aki spaces reset`: the starting spaces again, the old ones kept aside.
fn reset_spaces(config: &Config) -> ExitCode {
    let path = config::spaces_path();
    if path.exists() {
        let aside = path.with_extension("toml.bak");
        if let Err(error) = std::fs::rename(&path, &aside) {
            return fail(format!("cannot keep {} aside: {error}", path.display()));
        }
        println!("kept your spaces in {}", aside.display());
    }
    let sources: Vec<String> = config
        .sources
        .iter()
        .map(|source| source.name().to_string())
        .collect();
    let desk = Desk::starting(&sources);
    let store = DeskFileStore { path };
    match store.save(&desk) {
        Ok(()) => {
            print!("{}", text::spaces(&desk, None));
            ExitCode::SUCCESS
        }
        Err(error) => fail(error),
    }
}

/// The board as it stands, with what the agents are doing, and the hand-off
/// that was used to ask.
fn read_board(config: &Config, fresh_for: Duration) -> (Board, Handoff) {
    let sources = connect(config.sources.clone(), &open_cache(), fresh_for);
    let handoff = Handoff::new(&config.agent);
    let board = collect(&sources, config, &handoff);
    (board, handoff)
}

fn collect(sources: &[Box<dyn WorkSource>], config: &Config, handoff: &Handoff) -> Board {
    let mut board = CollectBoard {
        sources,
        clock: &SystemClock,
        rules: &config.rules,
        snoozes: &snoozes(),
    }
    .run();
    // What the agents opened on these tasks are doing, where it can be told.
    let sessions = handoff.launch().sessions();
    board.mark_agents(|task| handoff.launch().state_of(task, &sessions));
    board
}

/// The board through `keep`: what is set aside or kept apart shows only
/// when one of the filters asks for it.
fn through(board: &Board, keep: &[Filter]) -> Board {
    let asked = keep.iter().any(Filter::wants_snoozed);
    let drafts = keep.iter().any(Filter::wants_drafts);
    board.only(|task| {
        keep.iter().all(|filter| filter.matches(task))
            && (asked || task.snoozed.is_none())
            && (drafts || !task.apart)
    })
}

/// The task a reference names on the board.
fn find<'a>(board: &'a Board, reference: &str) -> Result<&'a Task, String> {
    let named = |task: &&Task| {
        [
            task.item.short(),
            task.item.reference.clone(),
            task.item.id(),
            task.item.url.clone(),
        ]
        .iter()
        .chain(task.tickets.iter().map(|ticket| &ticket.key))
        .any(|name| name.eq_ignore_ascii_case(reference))
    };
    let found: Vec<&Task> = board
        .todo
        .iter()
        .chain(&board.waiting)
        .filter(named)
        .collect();
    match found.as_slice() {
        [task] => Ok(*task),
        [] => Err(format!("no task \"{reference}\" on the board")),
        several => {
            let names: Vec<String> = several.iter().map(|task| task.item.short()).collect();
            Err(format!(
                "\"{reference}\" names several tasks: {}",
                names.join(", ")
            ))
        }
    }
}

/// `aki mcp`: answers an assistant until it hangs up.
fn serve_assistant(config: Config) -> ExitCode {
    if !config.mcp.enabled {
        return fail(format!(
            "the assistant's door is closed: set `enabled = true` under [mcp] in {}",
            config::path().display()
        ));
    }
    let handoff = Handoff::new(&config.agent);
    let sources = connect(config.sources.clone(), &open_cache(), config.cache);
    let now = || SystemClock.now();

    let handle = |call: Call| -> Result<String, String> {
        let board = collect(&sources, &config, &handoff);
        match call {
            Call::ListTasks {
                side,
                space,
                filter,
                sort,
                limit,
            } => {
                let keep = scope(&config, space.as_deref(), filter.as_deref())?;
                let order = text::Order {
                    sort,
                    reversed: false,
                };
                Ok(json::board(&through(&board, &keep), side, &order, limit))
            }
            Call::Task { reference } => {
                let task = find(&board, &reference)?;
                Ok(json::detail(
                    task,
                    &wording::urgency_parts(task, &config.rules, now()),
                    &brief::write(task, &handoff.agent.settings.instructions),
                ))
            }
            Call::Spaces => Ok(text::spaces(&desk(&config)?, Some(&board))),
            Call::Rules => Ok(text::rules(&config.rules)),
            Call::SetAside { reference, until } => {
                let task = find(&board, &reference)?;
                let until = match until {
                    Aside::Days(days) => Until::At(Timestamp(now().0 + i64::from(days) * 86_400)),
                    Aside::UntilItMoves => Until::Moves,
                };
                let item = task.item.id();
                set_aside(&snoozes(), &item, Some(Snooze::of(task, until)), now())?;
                Ok(format!(
                    "{}: {}",
                    task.item.short(),
                    wording::set_aside(until, now())
                ))
            }
            Call::BringBack { reference } => {
                let task = find(&board, &reference)?;
                set_aside(&snoozes(), &task.item.id(), None, now())?;
                Ok(format!("{}: back on the board", task.item.short()))
            }
            Call::StartAgent { reference } => hand_over(&handoff, find(&board, &reference)?),
        }
    };
    let allowed = Allowed {
        set_aside: config.mcp.set_aside,
        start_agent: config.mcp.start_agent,
    };
    match mcp::serve(
        std::io::stdin().lock(),
        std::io::stdout().lock(),
        allowed,
        &handle,
    ) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}

fn snoozes() -> SnoozeFile {
    SnoozeFile {
        path: config::snoozes_path(),
    }
}

/// `aki sounds`: what can be named and what is set; with a name, plays it.
fn sounds(config: &Config, play: Option<&str>) -> ExitCode {
    if let Some(sound) = play {
        return match sound::play(sound) {
            Ok(()) => ExitCode::SUCCESS,
            Err(refused) => fail(refused),
        };
    }
    let set = &config.sounds;
    println!(
        "new task: {}{}",
        set.new_task,
        if set.enabled {
            ""
        } else {
            "  (sounds are off)"
        }
    );
    for (kind, sound) in &set.rules {
        println!("  {}: {sound}", kind.name());
    }
    println!("\navailable (or a file's path):");
    for name in sound::available() {
        println!("  {name}");
    }
    ExitCode::SUCCESS
}

/// Everything opening an agent needs, set up once.
struct Handoff {
    agent: AgentConfig,
    checkouts: LocalCheckouts,
    launcher: Box<dyn Launcher>,
}

impl Handoff {
    fn new(agent: &AgentConfig) -> Self {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap_or_default());
        // Without the user's word, the usual places, those that exist.
        let roots = agent.roots.clone().unwrap_or_else(|| {
            [
                "code",
                "src",
                "dev",
                "projects",
                "Projects",
                "work",
                "Work",
                "repos",
                "git",
                "Developer",
                "Desktop",
            ]
            .iter()
            .map(|folder| home.join(folder))
            .filter(|folder| folder.is_dir())
            .collect()
        });
        let scripts = config::cache_dir().join("launch");
        let in_herdr = std::env::var("HERDR_ENV").is_ok_and(|value| value == "1");
        let terminal = agent
            .terminal
            .clone()
            .map(Terminal::Custom)
            .or_else(|| Terminal::detect(&|name| std::env::var(name).ok()));
        let set = |name: &str| std::env::var_os(name).is_some();
        let in_tuios = set("TUIOS_PANE_ID") || set("TUIOS_SESSION");
        // Left to choose: the multiplexer aki runs in (Herdr, TUIOS, tmux),
        // else the terminal itself, else (no terminal at all) a conversation.
        let choice = match agent.launcher {
            LauncherChoice::Auto if in_herdr || agent.herdr_session.is_some() => {
                LauncherChoice::Herdr
            }
            LauncherChoice::Auto if in_tuios || agent.tuios_session.is_some() => {
                LauncherChoice::Tuios
            }
            LauncherChoice::Auto if set("TMUX") && agent.terminal.is_none() => LauncherChoice::Tmux,
            LauncherChoice::Auto if terminal.is_some() => LauncherChoice::Terminal,
            LauncherChoice::Auto => LauncherChoice::Claude,
            chosen => chosen,
        };
        let launcher: Box<dyn Launcher> = match choice {
            LauncherChoice::Herdr => Box::new(HerdrLauncher {
                scripts,
                session: agent.herdr_session.clone(),
            }),
            LauncherChoice::Tmux => Box::new(TmuxLauncher { scripts }),
            LauncherChoice::Tuios => Box::new(TuiosLauncher {
                scripts,
                session: agent.tuios_session.clone(),
            }),
            LauncherChoice::Claude => Box::new(ChatLauncher {
                url: agent.chat_url.clone(),
            }),
            LauncherChoice::Terminal | LauncherChoice::Auto => {
                Box::new(TerminalLauncher { scripts, terminal })
            }
        };
        Self {
            agent: agent.clone(),
            checkouts: LocalCheckouts::new(agent.repos.clone(), roots),
            launcher,
        }
    }

    fn launch(&self) -> LaunchAgent<'_> {
        LaunchAgent {
            settings: &self.agent.settings,
            checkouts: &self.checkouts,
            launcher: self.launcher.as_ref(),
        }
    }
}

/// Where a session is, in a few words: `web › PROJ-580 !916 in ~/code/web`.
fn place(session: &Session) -> String {
    let home = std::env::var("HOME").unwrap_or_default();
    let folder = match &session.directory {
        Some(directory) => {
            let path = directory.to_string_lossy();
            match path.strip_prefix(&home) {
                Some(rest) if !home.is_empty() => format!(" in ~{rest}"),
                _ => format!(" in {path}"),
            }
        }
        None => " (no local clone found)".to_string(),
    };
    format!("{} › {}{folder}", session.workspace, session.tab)
}

/// Opens the agent on a task, and says what happened in one line.
fn hand_over(handoff: &Handoff, task: &Task) -> Result<String, String> {
    let brief = brief::write(task, &handoff.agent.settings.instructions);
    match handoff.launch().run(task, &brief) {
        Ok((session, Launched::Opened)) => Ok(format!("agent opened: {}", place(&session))),
        Ok((session, Launched::Briefed)) => Ok(format!(
            "agent already there, told what to do: {}",
            place(&session)
        )),
        Ok((session, Launched::Restarted)) => Ok(format!(
            "agent started in its open tab: {}",
            place(&session)
        )),
        Err(error) => Err(error.to_string()),
    }
}

/// `aki agent REF`: finds the task a reference names on the board, and
/// opens the agent on it.
fn agent(config: Config, reference: &str, dry_run: bool) -> ExitCode {
    let (board, handoff) = read_board(&config, config.cache);
    eprint!("{}", text::failures(&board, SystemClock.now()));
    let task = match find(&board, reference) {
        Ok(task) => task,
        Err(message) => return fail(message),
    };

    if dry_run {
        let brief = brief::write(task, &handoff.agent.settings.instructions);
        let session = handoff.launch().plan(task, &brief);
        println!("{}", place(&session));
        println!("$ {} <brief>\n", session.command[0]);
        println!("{brief}");
        return ExitCode::SUCCESS;
    }
    match hand_over(&handoff, task) {
        Ok(done) => {
            println!("{done}");
            ExitCode::SUCCESS
        }
        Err(refused) => fail(refused),
    }
}

/// What `list` was asked to print, and how.
struct Show {
    as_json: bool,
    count: bool,
    only: Option<Bucket>,
    keep: Vec<Filter>,
    order: text::Order,
    fresh: bool,
    color: bool,
}

fn list(config: Config, show: Show) -> ExitCode {
    let Show {
        as_json,
        count,
        only,
        keep,
        order,
        fresh,
        color,
    } = show;
    let fresh_for = if fresh { Duration::ZERO } else { config.cache };
    let clock = SystemClock;
    let (board, _) = read_board(&config, fresh_for);
    // What was set aside or kept apart is left out, unless a filter asks.
    let (aside, apart) = (board.snoozed_count(), board.apart_count());
    let board = through(&board, &keep);

    if count {
        println!("{}", text::count(&board, aside, apart));
    } else if as_json {
        println!("{}", json::board(&board, only, &order, None));
    } else {
        let style = text::Style {
            color: color && std::env::var_os("NO_COLOR").is_none(),
            // Piped output has no width of its own: give titles room.
            width: if color { terminal_width() } else { 160 },
        };
        print!("{}", text::board(&board, only, &order, clock.now(), &style));
    }
    eprint!("{}", text::failures(&board, clock.now()));

    // Something was read, now or before: the list is useful even if a
    // source is down.
    let nothing = board.sources.is_empty()
        && board
            .failures
            .iter()
            .all(|failure| failure.showing_from.is_none());
    if nothing {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

fn terminal_width() -> usize {
    crossterm::terminal::size()
        .map(|(columns, _)| columns as usize)
        .unwrap_or(120)
}

fn watch(config: Config) -> ExitCode {
    if !std::io::stdout().is_terminal() {
        return fail("watch needs a terminal; use `aki list` in scripts");
    }
    let desk = match desk(&config) {
        Ok(desk) => desk,
        Err(message) => return fail(message),
    };
    let store = DeskFileStore {
        path: config::spaces_path(),
    };
    let handoff = Handoff::new(&config.agent);
    let rules = config.rules.clone();
    let (updates_in, updates) = mpsc::channel();
    let (agents_updates, config_agent) = (updates_in.clone(), config.agent.clone());
    let (requests, requests_out) = mpsc::channel();

    // The watch owns the sources and outlives nothing: when the interface
    // quits, the process ends, mid-request or not.
    thread::spawn(move || {
        // The board always asks; the cache only fills the first seconds.
        let sources = connect(config.sources, &open_cache(), Duration::ZERO);
        let mut ways: Vec<Box<dyn Notifier>> = Vec::new();
        if config.notifications {
            ways.push(Box::new(DesktopNotifier));
        }
        ways.push(Box::new(SoundNotifier {
            sounds: config.sounds.clone(),
        }));
        let notifier: &dyn Notifier = &Together(ways);
        WatchBoard {
            collect: CollectBoard {
                sources: &sources,
                clock: &SystemClock,
                rules: &config.rules,
                snoozes: &snoozes(),
            },
            notifier,
            pacer: &TimerPacer {
                interval: config.refresh,
                requests: requests_out,
            },
        }
        .run(&ChannelView(updates_in));
    });

    // What the agents are doing changes on its own clock: looked at often,
    // told only when it changed.
    let (agents_in, agent_config) = (agents_updates, config_agent);
    thread::spawn(move || {
        let handoff = Handoff::new(&agent_config);
        let mut last = None;
        loop {
            let sessions = handoff.launch().sessions();
            if last.as_ref() != Some(&sessions) {
                if agents_in.send(Update::Agents(sessions.clone())).is_err() {
                    return;
                }
                last = Some(sessions);
            }
            thread::sleep(Duration::from_secs(2));
        }
    });

    let wiring = Wiring {
        updates,
        requests,
        clock: &SystemClock,
        open: &open_url,
        save: &|desk| store.save(desk).map_err(|error| error.to_string()),
        agent: &|task| hand_over(&handoff, task),
        agent_of: &|task, sessions| handoff.launch().state_of(task, sessions),
        snooze: &|item, snooze| set_aside(&snoozes(), item, snooze, SystemClock.now()),
        rules: &rules,
        kept_in: {
            let path = config::spaces_path().display().to_string();
            match std::env::var("HOME") {
                Ok(home) if !home.is_empty() && path.starts_with(&home) => {
                    format!("~{}", &path[home.len()..])
                }
                _ => path,
            }
        },
    };
    match tui::run(desk, wiring) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}
