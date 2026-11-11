//! The words. The rules decide what is true; this decides how to say it.

use crate::entities::task::{Bucket, Detail, Kind, Reason, Task, Trigger};
use unicode_width::UnicodeWidthStr;

use crate::entities::desk::{GroupKey, Pane, Sort};
use crate::entities::time::Timestamp;
use crate::entities::urgency::Heat;

pub fn label(kind: Kind) -> &'static str {
    match kind {
        Kind::ReviewRequested => "Review",
        Kind::ReReview => "Re-review",
        Kind::ReplyNeeded => "Reply",
        Kind::TicketMentioned => "Mentioned",
        Kind::ThreadToResolve => "Resolve threads",
        Kind::ChecksFailed => "Fix checks",
        Kind::Conflicts => "Fix conflicts",
        Kind::ReadyToMerge => "Merge",
        Kind::Assigned => "Assigned",
        Kind::NoReviewer => "Ask a review",
        Kind::TicketInProgress => "In progress",
        Kind::TicketTodo => "To do",
        Kind::Nudge => "Nudge",
        Kind::DraftToFinish => "Finish draft",
        Kind::AwaitingReview => "Awaiting review",
        Kind::AwaitingAuthor => "Awaiting author",
        Kind::AwaitingReply => "Awaiting reply",
        Kind::ChecksRunning => "Checks running",
        Kind::TicketBlocked => "Blocked",
        Kind::DraftByOther => "Draft",
    }
}

/// The rule in one line, for `myturn rules`.
pub fn describe(kind: Kind) -> &'static str {
    match kind {
        Kind::ReviewRequested => "your review is requested and you have not started",
        Kind::ReReview => "something moved since your review (see [re_review] triggers)",
        Kind::ReplyNeeded => "someone is waiting for your answer in a thread, or mentioned you",
        Kind::TicketMentioned => {
            "someone mentioned you on a ticket and you have not commented since"
        }
        Kind::ThreadToResolve => "an answered thread is yours to check and resolve",
        Kind::ChecksFailed => "the pipeline of your change failed",
        Kind::Conflicts => "your change conflicts with its target branch",
        Kind::ReadyToMerge => "your change is approved, green and quiet",
        Kind::Assigned => "a change you neither wrote nor review is assigned to you",
        Kind::NoReviewer => "your change is ready but nobody was asked to review it",
        Kind::TicketInProgress => "a ticket of yours is in progress",
        Kind::TicketTodo => "a ticket of yours is not started",
        Kind::Nudge => "a waiting item has not moved for [nudge] after_days",
        Kind::DraftToFinish => "your change is still a draft",
        Kind::AwaitingReview => "your change waits for reviewers",
        Kind::AwaitingAuthor => "you reviewed; the author has the ball",
        Kind::AwaitingReply => "you answered; the other side has to close the thread",
        Kind::ChecksRunning => "the pipeline of your change is running",
        Kind::TicketBlocked => "a ticket of yours is blocked or in a waiting status",
        Kind::DraftByOther => "a change you review is still a draft",
    }
}

fn plural(count: u32, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

fn trigger(trigger: Trigger) -> &'static str {
    match trigger {
        Trigger::AuthorReplied => "the author replied",
        Trigger::NewCommits => "new commits",
        Trigger::ReRequested => "review requested again",
        Trigger::ApprovalReset => "your approval was reset",
        Trigger::AllThreadsAnswered => "all your threads are answered",
    }
}

/// Why, in a few words.
pub fn explain(reason: &Reason) -> String {
    match (&reason.kind, &reason.detail) {
        (_, Detail::Triggers(triggers)) => triggers
            .iter()
            .map(|fired| trigger(*fired))
            .collect::<Vec<_>>()
            .join(", "),
        (Kind::ReplyNeeded, Detail::Threads(count)) => {
            format!(
                "{} for your answer",
                plural(*count, "thread waits", "threads wait")
            )
        }
        (Kind::ThreadToResolve, Detail::Threads(count)) => {
            format!(
                "{} to check and resolve",
                plural(*count, "answered thread", "answered threads")
            )
        }
        (Kind::AwaitingAuthor, Detail::Threads(0)) => "nothing new since your review".to_string(),
        (Kind::AwaitingAuthor, Detail::Threads(count)) => {
            format!(
                "{} unanswered",
                plural(*count, "thread of yours", "threads of yours")
            )
        }
        (Kind::AwaitingReply, Detail::Threads(count)) => {
            format!(
                "you answered {}, not resolved yet",
                plural(*count, "thread", "threads")
            )
        }
        (_, Detail::Threads(count)) => plural(*count, "thread", "threads"),
        (_, Detail::Review { missing, pending }) => {
            let mut parts = Vec::new();
            if *missing > 0 {
                parts.push(plural(*missing, "approval missing", "approvals missing"));
            }
            if !pending.is_empty() {
                let mut names = pending
                    .iter()
                    .take(3)
                    .cloned()
                    .collect::<Vec<_>>()
                    .join(", ");
                if pending.len() > 3 {
                    names.push_str(&format!(" +{}", pending.len() - 3));
                }
                parts.push(format!("not reviewed yet: {names}"));
            }
            if parts.is_empty() {
                "waiting for a maintainer".to_string()
            } else {
                parts.join(" · ")
            }
        }
        (_, Detail::Status(status)) => format!("status: {status}"),
        (_, Detail::BlockedBy(blockers)) => format!("blocked by {}", blockers.join(", ")),
        (_, Detail::Idle(days)) => format!("no movement for {}", plural(*days, "day", "days")),
        (kind, Detail::None) => match kind {
            Kind::ReviewRequested => "your review is requested",
            Kind::ChecksFailed => "the pipeline failed",
            Kind::ChecksRunning => "the pipeline is running",
            Kind::Conflicts => "conflicts with the target branch",
            Kind::ReadyToMerge => "approved, green, no open thread",
            Kind::Assigned => "assigned to you",
            Kind::NoReviewer => "nobody was asked to review",
            Kind::DraftToFinish => "still a draft",
            Kind::DraftByOther => "the author is still working on it",
            Kind::TicketMentioned => "you were mentioned in a comment",
            other => describe(*other),
        }
        .to_string(),
    }
}

/// How a pane is ordered, with the direction: the natural one points down.
pub fn sort(sort: Sort, reversed: bool) -> String {
    format!("{} {}", sort.name(), if reversed { "↑" } else { "↓" })
}

/// The mark of a task that should not wait any longer.
pub fn heat(heat: Heat) -> &'static str {
    match heat {
        Heat::Hot => "▲",
        Heat::Warm => "▴",
        Heat::Calm => " ",
    }
}

/// The tickets a task is about, with what is known of them: `PROJ-7 (high)`.
pub fn tickets(task: &Task) -> String {
    task.tickets
        .iter()
        .map(|ticket| match ticket.priority {
            Some(priority) => format!("{} ({})", ticket.key, priority.name()),
            None => ticket.key.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// The score, and what it is made of.
pub fn urgency(task: &Task) -> String {
    let urgency = &task.urgency;
    let mut parts = vec![format!("rule {}", urgency.rule)];
    if urgency.waiting > 0 {
        parts.push(format!("waiting +{}", urgency.waiting));
    }
    if urgency.ticket != 0 {
        let what = match (task.priority_level, task.tickets.is_empty()) {
            (Some(priority), _) => format!("priority {}", priority.name()),
            (None, false) => tickets(task),
            (None, true) => "ticket".to_string(),
        };
        parts.push(format!("{what} {:+}", urgency.ticket));
    }
    format!("urgency {} = {}", urgency.score(), parts.join(", "))
}

/// A group's heading.
pub fn group(key: &GroupKey) -> String {
    match key {
        GroupKey::All => String::new(),
        GroupKey::Repo(path) => path.clone(),
        GroupKey::Kind(kind) => label(*kind).to_string(),
        GroupKey::Source(name) => name.clone(),
        GroupKey::Person(Some(name)) => format!("@{name}"),
        GroupKey::Person(None) => "yours".to_string(),
        GroupKey::Role(role) => format!("as {}", role.name()),
    }
}

/// A pane's name: the one it was given, else what its filter says.
pub fn pane_title(pane: &Pane) -> String {
    if !pane.title.is_empty() {
        return pane.title.clone();
    }
    let side = match pane.filter.bucket() {
        Some(Bucket::Todo) => "To do",
        Some(Bucket::Waiting) => "Waiting",
        None => "Everything",
    };
    let rest = pane.filter.with_bucket(None);
    if rest.is_all() {
        side.to_string()
    } else {
        format!("{side} · {}", rest.query())
    }
}

pub fn headline(task: &Task) -> &'static str {
    label(task.headline().kind)
}

/// Every reason of a task on one line: the headline's why, then the others.
pub fn summary(task: &Task) -> String {
    let mut parts = vec![explain(task.headline())];
    parts.extend(
        task.reasons[1..]
            .iter()
            .map(|reason| format!("{}: {}", label(reason.kind).to_lowercase(), explain(reason))),
    );
    parts.join(" · ")
}

/// `now`, `12m`, `5h`, `3d`, `6w`.
pub fn age(since: Timestamp, now: Timestamp) -> String {
    let seconds = since.until(now);
    match seconds {
        0..60 => "now".to_string(),
        60..3_600 => format!("{}m", seconds / 60),
        3_600..86_400 => format!("{}h", seconds / 3_600),
        86_400..1_209_600 => format!("{}d", seconds / 86_400),
        _ => format!("{}w", seconds / 604_800),
    }
}

/// A title safe to lay out: without the invisible characters (emoji
/// variation selectors, joiners, controls) that terminals disagree on the
/// width of, which would shift everything drawn after them.
pub fn plain(text: &str) -> String {
    text.chars()
        .filter(|character| {
            !matches!(character, '\u{fe0e}' | '\u{fe0f}' | '\u{200d}') && !character.is_control()
        })
        .collect()
}

/// Cuts to `width` terminal columns, marking the cut. Emoji and CJK take
/// two columns; counting characters would push the next column around.
pub fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_string();
    }
    let mut kept = String::new();
    for character in text.chars() {
        kept.push(character);
        if kept.width() >= width {
            kept.pop();
            break;
        }
    }
    format!("{kept}…")
}

/// `fit`, then padded: exactly `width` columns wide.
pub fn column(text: &str, width: usize) -> String {
    let fitted = fit(text, width);
    let padding = width.saturating_sub(fitted.width());
    format!("{fitted}{}", " ".repeat(padding))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reason(kind: Kind, detail: Detail) -> Reason {
        Reason {
            detail,
            ..crate::entities::task::samples::reason(kind)
        }
    }

    #[test]
    fn explains_with_the_right_number() {
        assert_eq!(
            explain(&reason(Kind::ReplyNeeded, Detail::Threads(1))),
            "1 thread waits for your answer"
        );
        assert_eq!(
            explain(&reason(Kind::ThreadToResolve, Detail::Threads(3))),
            "3 answered threads to check and resolve"
        );
        assert_eq!(
            explain(&reason(
                Kind::ReReview,
                Detail::Triggers(vec![Trigger::AuthorReplied, Trigger::NewCommits])
            )),
            "the author replied, new commits"
        );
        assert_eq!(
            explain(&reason(
                Kind::AwaitingReview,
                Detail::Review {
                    missing: 2,
                    pending: vec!["till".into()]
                }
            )),
            "2 approvals missing · not reviewed yet: till"
        );
    }

    #[test]
    fn the_score_is_explained() {
        use crate::entities::task::samples;
        use crate::entities::ticket::{LinkedTicket, Priority};

        let mut task = samples::task("gitlab", "g/web", "!7", Kind::ReviewRequested, Some("alex"));
        assert_eq!(urgency(&task), "urgency 90 = rule 90");

        task.urgency.waiting = 12;
        task.urgency.ticket = 35;
        task.tickets = vec![LinkedTicket {
            key: "PROJ-7".into(),
            priority: Some(Priority::High),
        }];
        assert_eq!(
            urgency(&task),
            "urgency 137 = rule 90, waiting +12, PROJ-7 (high) +35"
        );
        assert_eq!(sort(Sort::Urgency, false), "urgency ↓");
        assert_eq!(sort(Sort::Created, true), "created ↑");
    }

    #[test]
    fn ages_are_short() {
        let now = Timestamp(10_000_000);
        let ago = |seconds| age(Timestamp(now.0 - seconds), now);
        assert_eq!(ago(5), "now");
        assert_eq!(ago(125), "2m");
        assert_eq!(ago(7_300), "2h");
        assert_eq!(ago(3 * 86_400), "3d");
        assert_eq!(ago(30 * 86_400), "4w");
    }

    #[test]
    fn fit_marks_the_cut() {
        assert_eq!(fit("short", 10), "short");
        assert_eq!(fit("a long title", 6), "a lon…");
        // Two columns each: three fit in six, with the mark only two.
        assert_eq!(fit("🐛🐛🐛🐛", 6), "🐛🐛…");
        assert_eq!(column("🐛 fix", 8), "🐛 fix  ");
        assert_eq!(plain("⚠\u{fe0f}⚠\u{fe0f} ok\t"), "⚠⚠ ok");
    }
}
