//! Fakes for the ports, shared by the use-case tests.

use std::cell::Cell;
use std::sync::Mutex;

use super::ports::{BoardView, Clock, Notifier, Pace, Pacer, Recalled, SourceError, WorkSource};
use crate::entities::board::Board;
use crate::entities::change_request::{ChangeRequest, Checks, ReviewState, Reviewer};
use crate::entities::item::{Actor, ItemRef, WorkItem};
use crate::entities::task::Task;
use crate::entities::ticket::{Priority, StatusCategory, Ticket};
use crate::entities::time::Timestamp;

pub struct FixedClock(pub i64);

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        Timestamp(self.0)
    }
}

type Answer = Result<Vec<WorkItem>, SourceError>;

/// Answers each `fetch` with the next scripted answer; repeats the last one.
pub struct ScriptedSource {
    name: String,
    script: Mutex<Vec<Answer>>,
    /// What `recall` gives.
    remembered: Option<Recalled>,
    /// The tickets `tickets` can be asked about.
    known: Vec<Ticket>,
}

impl ScriptedSource {
    pub fn script(name: &str, mut answers: Vec<Answer>) -> Self {
        answers.reverse();
        Self {
            name: name.to_string(),
            script: Mutex::new(answers),
            remembered: None,
            known: Vec::new(),
        }
    }

    pub fn remembering(mut self, items: Vec<WorkItem>, at: i64) -> Self {
        self.remembered = Some(Recalled {
            items,
            at: Timestamp(at),
        });
        self
    }

    pub fn knowing(mut self, tickets: Vec<WorkItem>) -> Self {
        self.known = tickets
            .into_iter()
            .filter_map(|item| match item {
                WorkItem::Ticket(ticket) => Some(ticket),
                WorkItem::Change(_) => None,
            })
            .collect();
        self
    }

    pub fn always(name: &str, items: Vec<WorkItem>) -> Self {
        Self::script(name, vec![Ok(items)])
    }

    pub fn failing(name: &str, error: SourceError) -> Self {
        Self::script(name, vec![Err(error)])
    }
}

impl WorkSource for ScriptedSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn identity(&self) -> Result<String, SourceError> {
        Ok("me".to_string())
    }

    fn fetch(&self) -> Answer {
        let mut script = self.script.lock().unwrap();
        if script.len() > 1 {
            script.pop().unwrap()
        } else {
            script.last().cloned().unwrap_or(Ok(Vec::new()))
        }
    }

    fn tickets(&self, keys: &[String]) -> Result<Vec<Ticket>, SourceError> {
        Ok(self
            .known
            .iter()
            .filter(|ticket| keys.contains(&ticket.item.reference))
            .cloned()
            .collect())
    }

    fn recall(&self) -> Option<Recalled> {
        self.remembered.clone()
    }
}

/// Lets the watch look a fixed number of times.
pub struct Turns(Cell<usize>);

impl Turns {
    pub fn new(looks: usize) -> Self {
        Self(Cell::new(looks))
    }
}

impl Pacer for Turns {
    fn wait(&self) -> Pace {
        let left = self.0.get().saturating_sub(1);
        self.0.set(left);
        if left == 0 { Pace::Stop } else { Pace::Again }
    }
}

/// Records what the watch showed and announced.
#[derive(Default)]
pub struct Recorder {
    boards: Mutex<Vec<Board>>,
    announced: Mutex<Vec<Vec<String>>>,
}

impl Recorder {
    pub fn boards(&self) -> usize {
        self.boards.lock().unwrap().len()
    }

    /// The items to do on the nth board shown.
    pub fn shown(&self, nth: usize) -> Vec<String> {
        self.boards.lock().unwrap()[nth]
            .todo
            .iter()
            .map(|task| task.item.id())
            .collect()
    }

    pub fn announced(&self) -> Vec<Vec<String>> {
        self.announced.lock().unwrap().clone()
    }
}

impl BoardView for Recorder {
    fn refreshing(&self) {}

    fn show(&self, board: Board) {
        self.boards.lock().unwrap().push(board);
    }
}

impl Notifier for Recorder {
    fn announce(&self, news: &[&Task]) {
        let ids = news.iter().map(|task| task.item.id()).collect();
        self.announced.lock().unwrap().push(ids);
    }
}

fn item(source: &str, container: &str, reference: &str) -> ItemRef {
    ItemRef {
        source: source.to_string(),
        container: container.to_string(),
        reference: reference.to_string(),
        title: format!("Title of {reference}"),
        url: format!("https://example.test/{reference}"),
        created_at: Timestamp(100),
        updated_at: Timestamp(500),
    }
}

/// Someone else's change, waiting for my first review.
pub fn review_request(reference: &str) -> WorkItem {
    WorkItem::Change(ChangeRequest {
        item: item("gitlab", "group/web", reference),
        branch: None,
        tickets: Vec::new(),
        author: Actor::other("alex"),
        assigned_to_me: false,
        reviewers: vec![Reviewer {
            actor: Actor::me("me"),
            state: ReviewState::Pending,
        }],
        draft: false,
        threads: Vec::new(),
        my_last_activity_at: None,
        last_push_at: None,
        review_requested_at: None,
        approved: false,
        approvals_missing: 1,
        checks: Checks::Passed,
        conflicts: false,
        i_can_merge: true,
    })
}

pub fn ticket(reference: &str, status: &str) -> WorkItem {
    WorkItem::Ticket(Ticket {
        item: item("jira", "X", reference),
        status: status.to_string(),
        category: StatusCategory::InProgress,
        priority: None,
        blocked_by: Vec::new(),
        mention_pending: false,
    })
}

pub fn review_request_titled(reference: &str, title: &str) -> WorkItem {
    let mut item = review_request(reference);
    if let WorkItem::Change(change) = &mut item {
        change.item.title = title.to_string();
    }
    item
}

/// A ticket of mine, in progress, named by its key.
pub fn ticket_with_priority(key: &str, priority: Priority) -> WorkItem {
    let mut item = ticket(key, "In Progress");
    if let WorkItem::Ticket(ticket) = &mut item {
        ticket.priority = Some(priority);
    }
    item
}
