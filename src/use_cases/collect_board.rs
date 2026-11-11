//! Reads every source, ties changes to their tickets, triages every item,
//! and assembles the board.

use std::collections::BTreeMap;
use std::thread;

use super::ports::{Clock, Recalled, WorkSource};
use crate::entities::board::{Board, Failure};
use crate::entities::item::WorkItem;
use crate::entities::rules::Rules;
use crate::entities::ticket::{LinkedTicket, Priority};
use crate::entities::time::Timestamp;
use crate::entities::triage::triage;

pub struct CollectBoard<'a> {
    pub sources: &'a [Box<dyn WorkSource>],
    pub clock: &'a dyn Clock,
    pub rules: &'a Rules,
}

/// What reading the sources gave, before anything is decided.
struct Reading {
    items: Vec<WorkItem>,
    reached: Vec<String>,
    failures: Vec<Failure>,
}

impl CollectBoard<'_> {
    /// Never fails: a source that cannot be read becomes a `Failure` on the
    /// board, with what it said last time when it remembers, and the others
    /// still show.
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

        let mut reading = Reading {
            items: Vec::new(),
            reached: Vec::new(),
            failures: Vec::new(),
        };
        for (source, answer) in self.sources.iter().zip(answers) {
            let name = source.name().to_string();
            match answer {
                Ok(items) => {
                    reading.reached.push(name);
                    reading.items.extend(items);
                }
                Err(error) => {
                    let recalled = source.recall();
                    reading.failures.push(Failure {
                        source: name,
                        message: error.to_string(),
                        showing_from: recalled.as_ref().map(|recalled| recalled.at),
                    });
                    reading
                        .items
                        .extend(recalled.map(|recalled| recalled.items).unwrap_or_default());
                }
            }
        }
        self.assemble(reading, self.clock.now(), true)
    }

    /// The board as the sources last saw it, without asking them anything:
    /// something to show at once while the first real reading is under way.
    /// `None` when no source remembers.
    pub fn recalled(&self) -> Option<Board> {
        let recalled: Vec<(String, Recalled)> = self
            .sources
            .iter()
            .filter_map(|source| Some((source.name().to_string(), source.recall()?)))
            .collect();
        // As old as its oldest part.
        let at = recalled.iter().map(|(_, recalled)| recalled.at).min()?;
        let mut reading = Reading {
            items: Vec::new(),
            reached: Vec::new(),
            failures: Vec::new(),
        };
        for (name, recalled) in recalled {
            reading.reached.push(name);
            reading.items.extend(recalled.items);
        }
        Some(self.assemble(reading, at, false))
    }

    /// Ties every change to the tickets it names. A ticket already read is
    /// used as is; the others are asked of the trackers when `ask` is set.
    fn link(&self, items: &mut [WorkItem], ask: bool) {
        let mut wanted: Vec<String> = Vec::new();
        let mut known: BTreeMap<String, Option<Priority>> = BTreeMap::new();
        for item in items.iter() {
            match item {
                WorkItem::Ticket(ticket) => {
                    known.insert(ticket.item.reference.clone(), ticket.priority);
                }
                WorkItem::Change(change) => {
                    for key in change.ticket_keys() {
                        if !wanted.contains(&key) {
                            wanted.push(key);
                        }
                    }
                }
            }
        }

        let missing: Vec<String> = wanted
            .into_iter()
            .filter(|key| !known.contains_key(key))
            .collect();
        // Whether somebody can tell a real ticket from a look-alike.
        let mut confirmed = false;
        if ask && !missing.is_empty() {
            for source in self.sources {
                // A tracker that cannot answer costs the priorities, not the board.
                if let Ok(tickets) = source.tickets(&missing) {
                    confirmed |= !tickets.is_empty();
                    for ticket in tickets {
                        known.insert(ticket.item.reference, ticket.priority);
                    }
                }
            }
        }

        for item in items.iter_mut() {
            if let WorkItem::Change(change) = item {
                change.tickets = change
                    .ticket_keys()
                    .into_iter()
                    .filter_map(|key| match known.get(&key) {
                        Some(priority) => Some(LinkedTicket {
                            key,
                            priority: *priority,
                        }),
                        // A tracker answered and does not know this one: it
                        // only looked like a key. With nobody to ask, the
                        // name is all there is; keep it.
                        None if confirmed => None,
                        None => Some(LinkedTicket {
                            key,
                            priority: None,
                        }),
                    })
                    .collect();
            }
        }
    }

    fn assemble(&self, mut reading: Reading, at: Timestamp, ask: bool) -> Board {
        self.link(&mut reading.items, ask);
        let tasks = reading
            .items
            .iter()
            .filter_map(|item| triage(item, self.rules, at))
            .collect();
        Board::new(tasks, reading.reached, reading.failures, at)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::task::Kind;
    use crate::entities::urgency::Heat;
    use crate::use_cases::ports::SourceError;
    use crate::use_cases::testing::{
        FixedClock, ScriptedSource, review_request, review_request_titled, ticket,
        ticket_with_priority,
    };

    fn board(sources: Vec<ScriptedSource>) -> Board {
        let sources: Vec<Box<dyn WorkSource>> = sources
            .into_iter()
            .map(|source| Box::new(source) as Box<dyn WorkSource>)
            .collect();
        CollectBoard {
            sources: &sources,
            clock: &FixedClock(1_000),
            rules: &Rules::default(),
        }
        .run()
    }

    #[test]
    fn splits_what_is_mine_from_what_waits() {
        let board = board(vec![
            ScriptedSource::always("gitlab", vec![review_request("!1")]),
            ScriptedSource::always("jira", vec![ticket("X-1", "Blocked")]),
        ]);

        assert_eq!(board.todo.len(), 1);
        assert_eq!(board.todo[0].headline().kind, Kind::ReviewRequested);
        assert_eq!(board.waiting.len(), 1);
        assert_eq!(board.waiting[0].headline().kind, Kind::TicketBlocked);
        assert_eq!(board.sources, ["gitlab", "jira"]);
        assert!(board.failures.is_empty());
    }

    #[test]
    fn a_broken_source_does_not_hide_the_others() {
        let board = board(vec![
            ScriptedSource::failing("jira", SourceError::Auth("no token".into())),
            ScriptedSource::always("gitlab", vec![review_request("!1")]),
        ]);

        assert_eq!(board.todo.len(), 1);
        assert_eq!(board.failures.len(), 1);
        assert_eq!(board.failures[0].source, "jira");
        assert_eq!(board.failures[0].message, "not signed in: no token");
        assert_eq!(board.failures[0].showing_from, None);
    }

    #[test]
    fn a_broken_source_that_remembers_still_shows_what_it_knew() {
        let source = ScriptedSource::failing("gitlab", SourceError::Unreachable("timeout".into()))
            .remembering(vec![review_request("!1")], 400);
        let board = board(vec![source]);

        assert_eq!(board.todo.len(), 1);
        assert!(board.sources.is_empty());
        assert_eq!(board.failures[0].showing_from, Some(Timestamp(400)));
    }

    #[test]
    fn the_last_known_board_needs_no_network() {
        let sources: Vec<Box<dyn WorkSource>> = vec![
            Box::new(
                ScriptedSource::failing("gitlab", SourceError::Unreachable("offline".into()))
                    .remembering(vec![review_request("!1")], 400),
            ),
            Box::new(ScriptedSource::always("jira", Vec::new()).remembering(Vec::new(), 300)),
        ];
        let collect = CollectBoard {
            sources: &sources,
            clock: &FixedClock(1_000),
            rules: &Rules::default(),
        };
        let board = collect.recalled().unwrap();
        assert_eq!(board.todo.len(), 1);
        assert_eq!(board.refreshed_at, Timestamp(300));
        assert!(board.failures.is_empty());

        let forgetful: Vec<Box<dyn WorkSource>> =
            vec![Box::new(ScriptedSource::always("gitlab", Vec::new()))];
        let collect = CollectBoard {
            sources: &forgetful,
            clock: &FixedClock(1_000),
            rules: &Rules::default(),
        };
        assert!(collect.recalled().is_none());
    }

    #[test]
    fn a_change_takes_the_priority_of_the_ticket_it_names() {
        let board = board(vec![
            ScriptedSource::always(
                "gitlab",
                vec![
                    review_request_titled("!1", "Resolve PROJ-7 \"export\""),
                    review_request_titled("!2", "Tidy the README"),
                ],
            ),
            // PROJ-7 is mine and was read with the rest: nobody is asked.
            ScriptedSource::always(
                "jira",
                vec![ticket_with_priority("PROJ-7", Priority::Highest)],
            ),
        ]);

        let linked = &board.todo[0];
        assert_eq!(
            linked.item.reference, "!1",
            "the urgent ticket's change is the more pressing"
        );
        assert_eq!(
            linked.tickets,
            [LinkedTicket {
                key: "PROJ-7".into(),
                priority: Some(Priority::Highest)
            }]
        );
        assert_eq!(linked.urgency.ticket, 55);
        assert_eq!(linked.urgency.heat, Heat::Hot);

        let plain = board
            .todo
            .iter()
            .find(|task| task.item.reference == "!2")
            .unwrap();
        assert!(plain.tickets.is_empty());
        assert_eq!(plain.urgency.ticket, 0);
    }

    #[test]
    fn someone_elses_ticket_is_asked_of_the_tracker() {
        let jira = ScriptedSource::always("jira", Vec::new())
            .knowing(vec![ticket_with_priority("PROJ-9", Priority::High)]);
        let board = board(vec![
            ScriptedSource::always(
                "gitlab",
                vec![
                    review_request_titled("!1", "PROJ-9: carry backups"),
                    review_request_titled("!2", "NOPE-1 only looks like a key"),
                ],
            ),
            jira,
        ]);

        let by_reference = |reference: &str| {
            board
                .todo
                .iter()
                .find(|task| task.item.reference == reference)
                .unwrap()
        };
        assert_eq!(by_reference("!1").tickets[0].priority, Some(Priority::High));
        assert!(
            by_reference("!2").tickets.is_empty(),
            "the tracker does not know it: not a ticket"
        );
    }

    #[test]
    fn without_a_tracker_the_key_alone_is_kept() {
        let board = board(vec![ScriptedSource::always(
            "gitlab",
            vec![review_request_titled("!1", "PROJ-9: carry backups")],
        )]);
        assert_eq!(
            board.todo[0].tickets,
            [LinkedTicket {
                key: "PROJ-9".into(),
                priority: None
            }]
        );
        assert_eq!(board.todo[0].urgency.ticket, 5);
    }
}
