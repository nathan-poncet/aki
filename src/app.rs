//! The composition root: the only place that knows every ring.

use std::io::IsTerminal;
use std::process::ExitCode;
use std::sync::mpsc;
use std::thread;

use crate::adapters::config::{self, Config, SourceConfig};
use crate::adapters::controllers::cli::{self, Command};
use crate::adapters::gateways::desk_file::DeskFileStore;
use crate::adapters::gateways::github::GitHub;
use crate::adapters::gateways::gitlab::GitLab;
use crate::adapters::gateways::jira::Jira;
use crate::adapters::gateways::system::{
    DesktopNotifier, SilentNotifier, SystemClock, TimerPacer, open_url,
};
use crate::adapters::presenters::tui::{self, ChannelView, Wiring};
use crate::adapters::presenters::{json, text};
use crate::entities::desk::Desk;
use crate::entities::filter::Filter;
use crate::entities::task::Bucket;
use crate::use_cases::check_sources::check_sources;
use crate::use_cases::collect_board::CollectBoard;
use crate::use_cases::open_desk::open_desk;
use crate::use_cases::ports::{Clock, DeskStore, Notifier, WorkSource};
use crate::use_cases::watch_board::WatchBoard;

/// Whether a program is on the PATH. Looked up, not run: CLIs are slow to start.
fn installed(cli: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|folder| folder.join(cli).is_file()))
}

fn connect(sources: Vec<SourceConfig>) -> Vec<Box<dyn WorkSource>> {
    sources
        .into_iter()
        .map(|source| -> Box<dyn WorkSource> {
            match source {
                SourceConfig::GitLab {
                    name,
                    host,
                    credential,
                } => {
                    let memory = config::cache_dir().join(format!("{name}-{host}.json"));
                    Box::new(GitLab::new(name, host, credential, Some(memory)))
                }
                SourceConfig::GitHub {
                    name,
                    host,
                    credential,
                } => Box::new(GitHub::new(name, host, credential)),
                SourceConfig::Jira {
                    name,
                    site,
                    email,
                    jql,
                    credential,
                } => Box::new(Jira::new(name, site, email, jql, credential)),
            }
        })
        .collect()
}

fn fail(message: impl std::fmt::Display) -> ExitCode {
    eprintln!("myturn: {message}");
    ExitCode::from(2)
}

pub fn run(arguments: &[String]) -> ExitCode {
    let interactive = std::io::stdout().is_terminal();
    let command = match cli::parse(arguments, interactive) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("myturn: {message}\n\n{}", cli::HELP);
            return ExitCode::from(2);
        }
    };

    match command {
        Command::Help => {
            print!("{}", cli::HELP);
            return ExitCode::SUCCESS;
        }
        Command::Version => {
            println!("myturn {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Command::ConfigPath => {
            println!("{}", config::path().display());
            return ExitCode::SUCCESS;
        }
        Command::ConfigInit => return init_config(),
        _ => {}
    }

    let config = match Config::load(&installed) {
        Ok(config) => config,
        Err(error) => return fail(format!("{}: {error}", config::path().display())),
    };
    if let Command::Rules = command {
        print!("{}", text::rules(&config.rules));
        return ExitCode::SUCCESS;
    }
    if config.sources.is_empty() {
        return fail(format!(
            "no source configured. Sign in with `glab auth login` or `gh auth login`, \
             or add [[sources]] to {} (`myturn config init` writes a template).",
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
        } => {
            let keep = match scope(&config, space.as_deref(), filter.as_deref()) {
                Ok(keep) => keep,
                Err(message) => return fail(message),
            };
            list(config, json, count, only, keep, interactive)
        }
        Command::Spaces => spaces(&config),
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
    let sources = connect(config.sources);
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

fn spaces(config: &Config) -> ExitCode {
    let desk = match desk(config) {
        Ok(desk) => desk,
        Err(message) => return fail(message),
    };
    let path = config::spaces_path();
    let saved = if path.exists() {
        ""
    } else {
        " (not written yet: defaults)"
    };
    println!("{}{saved}\n", path.display());
    for (index, space) in desk.spaces().iter().enumerate() {
        let scope = match space.scope.query() {
            "" => "everything",
            query => query,
        };
        println!(
            "{} {:<12} {:<8} {scope}",
            index + 1,
            space.name,
            space.layout.name()
        );
        for pane in &space.panes {
            let filter = match pane.filter.query() {
                "" => "everything",
                query => query,
            };
            println!("    {filter}  · group: {}", pane.grouping.name());
        }
    }
    ExitCode::SUCCESS
}

fn list(
    config: Config,
    as_json: bool,
    count: bool,
    only: Option<Bucket>,
    keep: Vec<Filter>,
    color: bool,
) -> ExitCode {
    let sources = connect(config.sources);
    let clock = SystemClock;
    let board = CollectBoard {
        sources: &sources,
        clock: &clock,
        rules: &config.rules,
    }
    .run();
    let board = board.only(|task| keep.iter().all(|filter| filter.matches(task)));

    if count {
        println!("{}", text::count(&board));
    } else if as_json {
        println!("{}", json::board(&board, only));
    } else {
        let style = text::Style {
            color: color && std::env::var_os("NO_COLOR").is_none(),
            // Piped output has no width of its own: give titles room.
            width: if color { terminal_width() } else { 160 },
        };
        print!("{}", text::board(&board, only, clock.now(), &style));
    }
    eprint!("{}", text::failures(&board));

    // Something was read: the list is useful even if a source is down.
    if board.sources.is_empty() {
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
        return fail("watch needs a terminal; use `myturn list` in scripts");
    }
    let desk = match desk(&config) {
        Ok(desk) => desk,
        Err(message) => return fail(message),
    };
    let store = DeskFileStore {
        path: config::spaces_path(),
    };
    let (updates_in, updates) = mpsc::channel();
    let (requests, requests_out) = mpsc::channel();

    // The watch owns the sources and outlives nothing: when the interface
    // quits, the process ends, mid-request or not.
    thread::spawn(move || {
        let sources = connect(config.sources);
        let notifier: &dyn Notifier = if config.notifications {
            &DesktopNotifier
        } else {
            &SilentNotifier
        };
        WatchBoard {
            collect: CollectBoard {
                sources: &sources,
                clock: &SystemClock,
                rules: &config.rules,
            },
            notifier,
            pacer: &TimerPacer {
                interval: config.refresh,
                requests: requests_out,
            },
        }
        .run(&ChannelView(updates_in));
    });

    let wiring = Wiring {
        updates,
        requests,
        clock: &SystemClock,
        open: &open_url,
        save: &|desk| store.save(desk).map_err(|error| error.to_string()),
    };
    match tui::run(desk, wiring) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => fail(error),
    }
}
