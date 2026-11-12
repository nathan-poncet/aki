//! Putting a task aside: it stays true, but the user said "not now". It
//! comes back by itself, at a time or when the item moves.

use super::task::Task;
use super::time::Timestamp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Until {
    /// Back at this time.
    At(Timestamp),
    /// Back when anything happens on the item.
    Moves,
}

/// What the user set aside, as it is remembered between runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snooze {
    /// The item's id.
    pub item: String,
    pub until: Until,
    /// The item's last update when it was set aside: what "moves" is
    /// measured against.
    pub seen: Timestamp,
}

impl Snooze {
    pub fn of(task: &Task, until: Until) -> Self {
        Self {
            item: task.item.id(),
            until,
            seen: task.item.updated_at,
        }
    }

    /// Whether this still keeps `task` aside.
    pub fn holds(&self, task: &Task, now: Timestamp) -> bool {
        self.item == task.item.id()
            && match self.until {
                Until::At(wake) => now < wake,
                Until::Moves => task.item.updated_at == self.seen,
            }
    }

    /// Whether this can never hold again, whatever the item does.
    pub fn is_over(&self, now: Timestamp) -> bool {
        matches!(self.until, Until::At(wake) if now >= wake)
    }
}

/// Marks the tasks that are set aside.
pub fn apply(tasks: &mut [Task], snoozes: &[Snooze], now: Timestamp) {
    for task in tasks {
        task.snoozed = snoozes
            .iter()
            .find(|snooze| snooze.holds(task, now))
            .map(|snooze| snooze.until);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::task::{Kind, samples};

    fn tasks() -> Vec<Task> {
        ["!1", "!2", "!3"]
            .iter()
            .map(|reference| {
                let mut task = samples::task("gitlab", "g/web", reference, Kind::ReplyNeeded, None);
                task.item.updated_at = Timestamp(500);
                task
            })
            .collect()
    }

    #[test]
    fn a_task_set_aside_until_a_time_comes_back_then() {
        let mut tasks = tasks();
        let snoozes = [Snooze::of(&tasks[0], Until::At(Timestamp(1_000)))];

        apply(&mut tasks, &snoozes, Timestamp(999));
        assert_eq!(tasks[0].snoozed, Some(Until::At(Timestamp(1_000))));
        assert_eq!(tasks[1].snoozed, None);

        apply(&mut tasks, &snoozes, Timestamp(1_000));
        assert_eq!(tasks[0].snoozed, None);
        assert!(snoozes[0].is_over(Timestamp(1_000)));
        assert!(!snoozes[0].is_over(Timestamp(999)));
    }

    #[test]
    fn a_task_set_aside_until_it_moves_comes_back_when_it_does() {
        let mut tasks = tasks();
        let snoozes = [Snooze::of(&tasks[1], Until::Moves)];

        apply(&mut tasks, &snoozes, Timestamp(9_999_999));
        assert_eq!(
            tasks[1].snoozed,
            Some(Until::Moves),
            "time alone does not wake it"
        );
        assert!(!snoozes[0].is_over(Timestamp(9_999_999)));

        tasks[1].item.updated_at = Timestamp(501);
        apply(&mut tasks, &snoozes, Timestamp(600));
        assert_eq!(tasks[1].snoozed, None);
    }
}
