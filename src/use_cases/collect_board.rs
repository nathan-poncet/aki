//! Reads every source, triages every item, and assembles the board.

use std::thread;

use super::ports::{Clock, WorkSource};
use crate::entities::board::{Board, Failure};
use crate::entities::rules::Rules;
use crate::entities::triage::triage;

pub struct CollectBoard<'a> {
    pub sources: &'a [Box<dyn WorkSource>],
    pub clock: &'a dyn Clock,
    pub rules: &'a Rules,
}

impl CollectBoard<'_> {
    /// Never fails: a source that cannot be read becomes a `Failure` on the
    /// board, and the others still show.
    pub fn run(&self) -> Board {
        // Sources are independent and slow; read them side by side.
        let answers: Vec<_> = thread::scope(|scope| {
            let readers: Vec<_> = self
                .sources
                .iter()
                .map(|source| scope.spawn(move || source.fetch()))
                .collect();
            readers
                .into_iter()
                .map(|reader| reader.join().expect("a source panicked"))
                .collect()
        });

        let now = self.clock.now();
        let mut tasks = Vec::new();
        let mut reached = Vec::new();
        let mut failures = Vec::new();
        for (source, answer) in self.sources.iter().zip(answers) {
            match answer {
                Ok(items) => {
                    reached.push(source.name().to_string());
                    tasks.extend(
                        items
                            .iter()
                            .filter_map(|item| triage(item, self.rules, now)),
                    );
                }
                Err(error) => failures.push(Failure {
                    source: source.name().to_string(),
                    message: error.to_string(),
                }),
            }
        }
        Board::new(tasks, reached, failures, now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::task::Kind;
    use crate::use_cases::ports::SourceError;
    use crate::use_cases::testing::{FixedClock, ScriptedSource, review_request, ticket};

    #[test]
    fn splits_what_is_mine_from_what_waits() {
        let sources: Vec<Box<dyn WorkSource>> = vec![
            Box::new(ScriptedSource::always("gitlab", vec![review_request("!1")])),
            Box::new(ScriptedSource::always(
                "jira",
                vec![ticket("X-1", "Blocked")],
            )),
        ];
        let board = CollectBoard {
            sources: &sources,
            clock: &FixedClock(1_000),
            rules: &Rules::default(),
        }
        .run();

        assert_eq!(board.todo.len(), 1);
        assert_eq!(board.todo[0].headline().kind, Kind::ReviewRequested);
        assert_eq!(board.waiting.len(), 1);
        assert_eq!(board.waiting[0].headline().kind, Kind::TicketBlocked);
        assert_eq!(board.sources, ["gitlab", "jira"]);
        assert!(board.failures.is_empty());
    }

    #[test]
    fn a_broken_source_does_not_hide_the_others() {
        let sources: Vec<Box<dyn WorkSource>> = vec![
            Box::new(ScriptedSource::failing(
                "jira",
                SourceError::Auth("no token".into()),
            )),
            Box::new(ScriptedSource::always("gitlab", vec![review_request("!1")])),
        ];
        let board = CollectBoard {
            sources: &sources,
            clock: &FixedClock(1_000),
            rules: &Rules::default(),
        }
        .run();

        assert_eq!(board.todo.len(), 1);
        assert_eq!(board.failures.len(), 1);
        assert_eq!(board.failures[0].source, "jira");
        assert_eq!(board.failures[0].message, "not signed in: no token");
    }
}
