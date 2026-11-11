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
    CachePath,
    CacheClear,
    ConfigPath,
    ConfigInit,
    Help,
    Version,
}

pub const HELP: &str = "\
myturn: whose turn is it?

One list of what waits on you, and what waits on others, across GitLab,
GitHub and Jira.

USAGE
  myturn                 live board in a terminal, the list when piped
  myturn watch           live board: spaces, tiled panes, filters, groups;
                         refreshes by itself, notifies what is new (? for keys)
  myturn list            print the board once
      --todo | --waiting   only one side
      --space NAME         only what belongs to one of your spaces
      --filter QUERY       is: source: repo: kind: top: role: by: words, -negated
      --sort ORDER         urgency (default), rule, waiting, updated, created
      --reverse            the opposite order
      --fresh              ask the sources even if a recent reading is cached
      --json               machine-readable
      --count              one line: \"3 to do · 5 waiting\"
  myturn spaces          where your spaces are kept, and what they are
  myturn cache path      where what was read is kept (SQLite)
  myturn cache clear     forget it all
  myturn doctor          check the connection to every source
  myturn rules           print every rule as configured
  myturn config path     where the configuration file is
  myturn config init     write a commented configuration file with the defaults
  myturn help | version
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
        "spaces" => none(Command::Spaces),
        "help" | "--help" | "-h" => Ok(Command::Help),
        "version" | "--version" | "-V" => Ok(Command::Version),
        "config" => match rest.as_slice() {
            ["path"] => Ok(Command::ConfigPath),
            ["init"] => Ok(Command::ConfigInit),
            _ => Err("usage: myturn config path | init".to_string()),
        },
        "cache" => match rest.as_slice() {
            ["path"] => Ok(Command::CachePath),
            ["clear"] => Ok(Command::CacheClear),
            _ => Err("usage: myturn cache path | clear".to_string()),
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
    }

    #[test]
    fn strangers_are_refused() {
        assert!(parse_words(&["sync"], true).is_err());
        assert!(parse_words(&["doctor", "now"], true).is_err());
        assert!(parse_words(&["config"], true).is_err());
        assert_eq!(
            parse_words(&["config", "init"], true),
            Ok(Command::ConfigInit)
        );
    }
}
