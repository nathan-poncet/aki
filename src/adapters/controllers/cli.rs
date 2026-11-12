//! The command line: argv in, a typed command out.

use crate::entities::desk::Sort;
use crate::entities::task::Bucket;

#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    /// The live board.
    Watch,
    List {
        json: bool,
        count: bool,
        only: Option<Bucket>,
        /// The name of one of the user's spaces.
        space: Option<String>,
        /// A filter query, as in the board.
        filter: Option<String>,
        sort: Sort,
        reversed: bool,
        /// Ask the sources even if a recent reading is at hand.
        fresh: bool,
    },
    Doctor,
    Rules,
    Spaces,
    /// Start the spaces over from the sources, keeping the old file aside.
    SpacesReset,
    /// Serve the board to an assistant, over standard input and output.
    Mcp,
    /// Say how to plug the board into an assistant.
    McpSetup,
    /// Open the agent on the task this names.
    Agent {
        task: String,
        /// Say what would be opened, and the brief, without opening it.
        dry_run: bool,
    },
    CachePath,
    CacheClear,
    /// List the sounds that can be named, or play one.
    Sounds {
        play: Option<String>,
    },
    ConfigPath,
    ConfigInit,
    Help,
    Version,
}

pub const HELP: &str = "\
aki: whose turn is it?

One list of what waits on you, and what waits on others, across GitLab,
GitHub and Jira.

USAGE
  aki                    live board in a terminal, the list when piped
  aki watch              live board: spaces, tiled panes, filters, groups;
                         refreshes by itself, notifies what is new (? for keys)
  aki list               print the board once
      --todo | --waiting   only one side
      --space NAME         only what belongs to one of your spaces
      --filter QUERY       is:todo|waiting|draft|snoozed source: repo: kind: top:
                           role: by: heat: ticket: priority: agent: words, -negated
      --sort ORDER         urgency (default), rule, waiting, updated, created
      --reverse            the opposite order
      --fresh              ask the sources even if a recent reading is cached
      --json               machine-readable
      --count              one line: \"3 to do · 5 waiting\"
  aki agent REF          open your coding agent on a task (web!916, PROJ-7…),
                         briefed, in its clone; `a` in the board does the same
      --dry-run            only say where it would open, and the brief
  aki spaces             where your spaces come from, what each one shows and
                         which repositories it holds right now
  aki spaces reset       start them over from your sources (the old file is kept
                         beside, as spaces.toml.bak)
  aki mcp                serve the board to an assistant (Claude Desktop, Claude
                         Code…) over the Model Context Protocol
  aki mcp setup          what to paste where to plug it in
  aki sounds [NAME]      list the sounds a new task can ring with, or play one
  aki cache path         where what was read is kept (SQLite)
  aki cache clear        forget it all
  aki doctor             check the connection to every source
  aki rules              print every rule as configured
  aki config path        where the configuration file is
  aki config init        write a commented configuration file with the defaults
  aki help | version
";

/// `interactive`: whether output goes to a terminal.
pub fn parse(arguments: &[String], interactive: bool) -> Result<Command, String> {
    let mut words = arguments.iter().map(String::as_str);
    let Some(command) = words.next() else {
        return Ok(if interactive {
            Command::Watch
        } else {
            Command::List {
                json: false,
                count: false,
                only: None,
                space: None,
                filter: None,
                sort: Sort::Urgency,
                reversed: false,
                fresh: false,
            }
        });
    };
    let rest: Vec<&str> = words.collect();
    let none = |command: Command| match rest.first() {
        None => Ok(command),
        Some(extra) => Err(format!("unexpected argument \"{extra}\"")),
    };

    match command {
        "watch" => none(Command::Watch),
        "doctor" => none(Command::Doctor),
        "rules" => none(Command::Rules),
        "spaces" => match rest.as_slice() {
            [] => Ok(Command::Spaces),
            ["reset"] => Ok(Command::SpacesReset),
            _ => Err("usage: aki spaces [reset]".to_string()),
        },
        "mcp" => match rest.as_slice() {
            [] => Ok(Command::Mcp),
            ["setup"] => Ok(Command::McpSetup),
            _ => Err("usage: aki mcp [setup]".to_string()),
        },
        "agent" => match rest.as_slice() {
            [task] if !task.starts_with("--") => Ok(Command::Agent {
                task: task.to_string(),
                dry_run: false,
            }),
            [task, "--dry-run"] | ["--dry-run", task] if !task.starts_with("--") => {
                Ok(Command::Agent {
                    task: task.to_string(),
                    dry_run: true,
                })
            }
            _ => Err("usage: aki agent REFERENCE [--dry-run]".to_string()),
        },
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-V" => Ok(Command::Version),
        "config" => match rest.as_slice() {
            ["path"] => Ok(Command::ConfigPath),
            ["init"] => Ok(Command::ConfigInit),
            _ => Err("usage: aki config path | init".to_string()),
        },
        "sounds" => match rest.as_slice() {
            [] => Ok(Command::Sounds { play: None }),
            [sound] => Ok(Command::Sounds {
                play: Some(sound.to_string()),
            }),
            _ => Err("usage: aki sounds [NAME or FILE]".to_string()),
        },
        "cache" => match rest.as_slice() {
            ["path"] => Ok(Command::CachePath),
            ["clear"] => Ok(Command::CacheClear),
            _ => Err("usage: aki cache path | clear".to_string()),
        },
        "list" => {
            let (mut json, mut count, mut only) = (false, false, None);
            let (mut space, mut filter) = (None, None);
            let (mut sort, mut reversed, mut fresh) = (Sort::Urgency, false, false);
            let mut flags = rest.into_iter();
            while let Some(flag) = flags.next() {
                let mut value = || {
                    flags
                        .next()
                        .map(str::to_string)
                        .ok_or_else(|| format!("{flag} needs a value"))
                };
                match flag {
                    "--json" => json = true,
                    "--count" => count = true,
                    "--todo" => only = Some(Bucket::Todo),
                    "--waiting" => only = Some(Bucket::Waiting),
                    "--space" => space = Some(value()?),
                    "--filter" => filter = Some(value()?),
                    "--sort" => {
                        let name = value()?;
                        sort = Sort::from_name(&name).ok_or_else(|| {
                            format!(
                                "unknown order \"{name}\" (expected: {})",
                                Sort::ALL.map(Sort::name).join(", ")
                            )
                        })?;
                    }
                    "--reverse" => reversed = true,
                    "--fresh" => fresh = true,
                    other => return Err(format!("unknown option \"{other}\" for list")),
                }
            }
            Ok(Command::List {
                json,
                count,
                only,
                space,
                filter,
                sort,
                reversed,
                fresh,
            })
        }
        other => Err(format!("unknown command \"{other}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_words(words: &[&str], interactive: bool) -> Result<Command, String> {
        let arguments: Vec<String> = words.iter().map(|word| word.to_string()).collect();
        parse(&arguments, interactive)
    }

    #[test]
    fn bare_invocation_depends_on_the_terminal() {
        assert_eq!(parse_words(&[], true), Ok(Command::Watch));
        assert!(matches!(parse_words(&[], false), Ok(Command::List { .. })));
    }

    #[test]
    fn list_takes_its_flags() {
        assert_eq!(
            parse_words(&["list", "--json", "--waiting"], true),
            Ok(Command::List {
                json: true,
                count: false,
                only: Some(Bucket::Waiting),
                space: None,
                filter: None,
                sort: Sort::Urgency,
                reversed: false,
                fresh: false,
            })
        );
        assert!(parse_words(&["list", "--yaml"], true).is_err());
        assert!(matches!(
            parse_words(&["list", "--space", "Pro", "--filter", "repo:web -kind:draft"], true),
            Ok(Command::List { space: Some(space), filter: Some(filter), .. })
                if space == "Pro" && filter == "repo:web -kind:draft"
        ));
        assert!(parse_words(&["list", "--space"], true).is_err());
        assert!(matches!(
            parse_words(&["list", "--sort", "waiting", "--reverse", "--fresh"], true),
            Ok(Command::List {
                sort: Sort::Waiting,
                reversed: true,
                fresh: true,
                ..
            })
        ));
        assert!(parse_words(&["list", "--sort", "luck"], true).is_err());
        assert_eq!(
            parse_words(&["cache", "clear"], true),
            Ok(Command::CacheClear)
        );
        assert_eq!(
            parse_words(&["agent", "web!916", "--dry-run"], true),
            Ok(Command::Agent {
                task: "web!916".into(),
                dry_run: true
            })
        );
        assert!(parse_words(&["agent"], true).is_err());
        assert!(parse_words(&["agent", "--dry-run"], true).is_err());
    }

    #[test]
    fn strangers_are_refused() {
        assert!(parse_words(&["sync"], true).is_err());
        assert!(parse_words(&["doctor", "now"], true).is_err());
        assert!(parse_words(&["config"], true).is_err());
        assert_eq!(parse_words(&["mcp"], false), Ok(Command::Mcp));
        assert_eq!(parse_words(&["mcp", "setup"], true), Ok(Command::McpSetup));
        assert!(parse_words(&["mcp", "serve"], true).is_err());
        assert_eq!(
            parse_words(&["spaces", "reset"], true),
            Ok(Command::SpacesReset)
        );
        assert_eq!(
            parse_words(&["config", "init"], true),
            Ok(Command::ConfigInit)
        );
    }
}
