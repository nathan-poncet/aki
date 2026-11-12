//! The whole picture at one instant, and what changed between two of them.

use std::collections::BTreeSet;

use super::handoff::AgentState;
use super::snooze::{Snooze, Until};
use super::task::{Bucket, Kind, Task};
use super::time::Timestamp;

/// A source that could not be read. The rest of the board is still valid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub source: String,
    pub message: String,
    /// When set, the board still shows this source as it was then.
    pub showing_from: Option<Timestamp>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Board {
    pub todo: Vec<Task>,
    pub waiting: Vec<Task>,
    pub failures: Vec<Failure>,
    /// Sources that answered.
    pub sources: Vec<String>,
    pub refreshed_at: Timestamp,
}

impl Board {
    pub fn new(
        tasks: Vec<Task>,
        sources: Vec<String>,
        failures: Vec<Failure>,
        refreshed_at: Timestamp,
    ) -> Self {
        let (mut todo, mut waiting): (Vec<Task>, Vec<Task>) = tasks
            .into_iter()
            .partition(|task| task.bucket() == Bucket::Todo);
        // Most pressing first; among equals, the one that moved most recently.
        let order = |task: &Task| {
            (
                task.priority(),
                std::cmp::Reverse(task.since),
                task.item.id(),
            )
        };
        todo.sort_by_key(order);
        waiting.sort_by_key(order);
        Self {
            todo,
            waiting,
            failures,
            sources,
            refreshed_at,
        }
    }

    /// The same board, with only the tasks `keep` accepts.
    pub fn only(&self, keep: impl Fn(&Task) -> bool) -> Board {
        Board {
            todo: self
                .todo
                .iter()
                .filter(|task| keep(task))
                .cloned()
                .collect(),
            waiting: self
                .waiting
                .iter()
                .filter(|task| keep(task))
                .cloned()
                .collect(),
            failures: self.failures.clone(),
            sources: self.sources.clone(),
            refreshed_at: self.refreshed_at,
        }
    }

    pub fn tasks(&self, bucket: Bucket) -> &[Task] {
        match bucket {
            Bucket::Todo => &self.todo,
            Bucket::Waiting => &self.waiting,
        }
    }

    /// Sets a task aside, or brings it back (`None`). Returns what to
    /// remember, when there is something to remember.
    pub fn snooze(&mut self, item: &str, until: Option<Until>) -> Option<Snooze> {
        let task = self
            .todo
            .iter_mut()
            .chain(&mut self.waiting)
            .find(|task| task.item.id() == item)?;
        task.snoozed = until;
        until.map(|until| Snooze::of(task, until))
    }

    /// Says of every task what its agent is doing, when it has one.
    pub fn mark_agents(&mut self, of: impl Fn(&Task) -> Option<AgentState>) {
        for task in self.todo.iter_mut().chain(&mut self.waiting) {
            task.agent = of(task);
        }
    }

    /// How many tasks are set aside.
    pub fn snoozed_count(&self) -> usize {
        self.todo
            .iter()
            .chain(&self.waiting)
            .filter(|task| task.snoozed.is_some())
            .count()
    }

    /// How many tasks the rules keep apart.
    pub fn apart_count(&self) -> usize {
        self.todo
            .iter()
            .chain(&self.waiting)
            .filter(|task| task.apart && task.snoozed.is_none())
            .count()
    }

    /// Every (item, reason) that asks something of me.
    fn demands(&self) -> BTreeSet<(String, Kind)> {
        self.todo
            .iter()
            .filter(|task| task.in_view())
            .flat_map(|task| {
                task.reasons
                    .iter()
                    .filter(|reason| reason.bucket == Bucket::Todo)
                    .map(|reason| (task.item.id(), reason.kind))
            })
            .collect()
    }

    /// Tasks that ask something of me that `previous` did not: what is worth
    /// interrupting for. Items from sources that failed last time are not
    /// "new": they were only hidden.
    pub fn news_since(&self, previous: &Board) -> Vec<&Task> {
        let known = previous.demands();
        let hidden: BTreeSet<&str> = previous
            .failures
            .iter()
            .map(|failure| failure.source.as_str())
            .collect();
        self.todo
            .iter()
            // What was set aside comes back quietly; what is apart stays quiet.
            .filter(|task| task.in_view())
            .filter(|task| !hidden.contains(task.item.source.as_str()))
            .filter(|task| {
                task.reasons
                    .iter()
                    .filter(|reason| reason.bucket == Bucket::Todo)
                    .any(|reason| !known.contains(&(task.item.id(), reason.kind)))
            })
            .collect()
    }
}
