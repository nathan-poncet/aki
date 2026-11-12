//! What an agent is told when it is handed a task: where the work is, why
//! it is on the board, and what is expected, most pressing first.

use std::fmt::Write;

use super::wording::{self, plain};
use crate::entities::task::{Kind, Reason, Role, Task};

/// What each rule asks of whoever picks the task up.
fn ask(kind: Kind) -> &'static str {
    match kind {
        Kind::ReviewRequested => {
            "Review it: read the description and the whole diff, check correctness, tests and \
             design, and draft your review comments."
        }
        Kind::ReReview => {
            "I already reviewed it. Look at what changed since: the commits pushed after my \
             review and the answers to my comments. Tell me whether each of my points was \
             addressed. If they all were and nothing new is wrong, say it is ready to approve."
        }
        Kind::ReplyNeeded => {
            "People are waiting for my answer. Read every open thread, and for each one propose \
             the reply, or the change in the code it calls for."
        }
        Kind::TicketMentioned => {
            "I was mentioned in a comment. Find it, and propose the answer it calls for."
        }
        Kind::ThreadToResolve => {
            "Threads I opened were answered. Check each answer against the code as it is now, \
             and tell me which threads can be resolved and which need a follow-up."
        }
        Kind::ChecksFailed => {
            "The pipeline fails. Find the failing job, read its log, find the cause and fix it."
        }
        Kind::Conflicts => {
            "It conflicts with its target branch. Rebase it and resolve the conflicts, keeping \
             both sides' intent."
        }
        Kind::Rebase => {
            "Its target branch moved on. Rebase it on the target, check it still builds and \
             passes its tests, and tell me before pushing."
        }
        Kind::ReadyToMerge => {
            "It is approved, green and quiet. Check that nothing is left pending, then it can \
             be merged."
        }
        Kind::Assigned => "It was assigned to me. Find out what is expected of me on it.",
        Kind::NoReviewer => {
            "Nobody was asked to review it. Check that it is ready, and suggest who should \
             review it from who knows this code."
        }
        Kind::TicketInProgress => {
            "This ticket is in progress. Find where the work stands, branches and changes \
             included, and what is left to do."
        }
        Kind::TicketTodo => {
            "This ticket is not started. Read it, find the code it concerns, and propose a plan."
        }
        Kind::Nudge => {
            "Nothing has moved for a while. Find who has the ball, and draft a short message to \
             get it moving."
        }
        Kind::DraftToFinish => {
            "It is still a draft. Find what is left before it can be reviewed, and do it."
        }
        Kind::AwaitingReview | Kind::AwaitingAuthor | Kind::AwaitingReply => {
            "Nothing is expected of me right now. Tell me where it stands, and whether it is \
             worth chasing."
        }
        Kind::ChecksRunning => "The pipeline is running. Watch it, and deal with a failure.",
        Kind::TicketBlocked => {
            "This ticket is blocked. Find what blocks it, and whether anything can be done \
             meanwhile."
        }
        Kind::DraftByOther => {
            "The author is still working on it. A first look is welcome, a full review is not."
        }
    }
}

fn step(number: usize, reason: &Reason) -> String {
    let mut step = format!(
        "{number}. {}: {}.\n   {}",
        wording::label(reason.kind),
        wording::explain(reason),
        ask(reason.kind)
    );
    if let Some(url) = &reason.url {
        let _ = write!(step, "\n   Start here: {url}");
    }
    step
}

/// `instructions` is the user's own word on how their agent should work.
pub fn write(task: &Task, instructions: &str) -> String {
    let item = &task.item;
    let mut brief = format!(
        "This comes from my board of things to do.\n\n{}\n{}\n",
        plain(&item.title),
        item.url
    );

    let me = match task.role {
        Role::Author => "I wrote it".to_string(),
        Role::Reviewer => "I am a reviewer".to_string(),
        Role::Assignee => "It is assigned to me".to_string(),
        Role::Participant => "I took part in it".to_string(),
    };
    let by = task
        .counterpart
        .as_deref()
        .map(|name| format!("; the author is @{name}"))
        .unwrap_or_default();
    let _ = writeln!(brief, "{} on {}. {me}{by}.", item.short(), item.source);
    if !task.tickets.is_empty() {
        let _ = writeln!(brief, "Ticket: {}.", wording::tickets(task));
    }
    if let Some(priority) = task.priority_level {
        let _ = writeln!(brief, "Priority: {}.", priority.name());
    }

    brief.push_str("\nWhat there is to do, most pressing first:\n");
    for (index, reason) in task.reasons.iter().enumerate() {
        let _ = writeln!(brief, "{}", step(index + 1, reason));
    }

    brief.push_str(
        "\nStart by reading it where it lives, with the forge's or the tracker's own tools. \
         Then tell me what you found and what you propose. Do not post, approve, merge or push \
         anything before I say so.",
    );
    if !instructions.trim().is_empty() {
        let _ = write!(brief, "\n\n{}", instructions.trim());
    }
    brief
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::task::{Detail, Trigger, samples};
    use crate::entities::ticket::{LinkedTicket, Priority};

    fn re_review() -> Task {
        let mut task = samples::task("gitlab", "acme/web", "!916", Kind::ReReview, Some("alex"));
        task.item.title = "Resolve PROJ-580 \"v1 api\" ⚠\u{fe0f}".into();
        task.item.url = "https://gitlab.test/acme/web/-/merge_requests/916".into();
        task.reasons[0].detail =
            Detail::Triggers(vec![Trigger::AuthorReplied, Trigger::NewCommits]);
        task.reasons[0].url =
            Some("https://gitlab.test/acme/web/-/merge_requests/916#note_7".into());
        let mut resolve = samples::reason(Kind::ThreadToResolve);
        resolve.detail = Detail::Threads(2);
        task.reasons.push(resolve);
        task.tickets = vec![LinkedTicket {
            key: "PROJ-580".into(),
            priority: Some(Priority::High),
        }];
        task
    }

    #[test]
    fn the_brief_says_where_why_and_what_in_order() {
        let brief = write(&re_review(), "");
        assert_eq!(
            brief,
            "This comes from my board of things to do.\n\
             \n\
             Resolve PROJ-580 \"v1 api\" ⚠\n\
             https://gitlab.test/acme/web/-/merge_requests/916\n\
             web!916 on gitlab. I am a reviewer; the author is @alex.\n\
             Ticket: PROJ-580 (high).\n\
             \n\
             What there is to do, most pressing first:\n\
             1. Re-review: the author replied, new commits.\n   \
             I already reviewed it. Look at what changed since: the commits pushed after my \
             review and the answers to my comments. Tell me whether each of my points was \
             addressed. If they all were and nothing new is wrong, say it is ready to approve.\n   \
             Start here: https://gitlab.test/acme/web/-/merge_requests/916#note_7\n\
             2. Resolve threads: 2 answered threads to check and resolve.\n   \
             Threads I opened were answered. Check each answer against the code as it is now, \
             and tell me which threads can be resolved and which need a follow-up.\n\
             \n\
             Start by reading it where it lives, with the forge's or the tracker's own tools. \
             Then tell me what you found and what you propose. Do not post, approve, merge or \
             push anything before I say so."
        );
    }

    #[test]
    fn the_users_own_instructions_close_the_brief() {
        let brief = write(&re_review(), "  Answer in French.\n");
        assert!(brief.ends_with("before I say so.\n\nAnswer in French."));
    }

    #[test]
    fn a_ticket_of_mine_is_briefed_with_its_priority() {
        let mut ticket = samples::task("jira", "PROJ", "PROJ-7", Kind::TicketTodo, None);
        ticket.role = Role::Assignee;
        ticket.priority_level = Some(Priority::Highest);
        let brief = write(&ticket, "");
        assert!(brief.contains("PROJ-7 on jira. It is assigned to me.\nPriority: highest.\n"));
        assert!(brief.contains("1. To do: "));
        assert!(brief.contains("propose a plan"));
    }

    #[test]
    fn every_rule_has_something_to_ask() {
        for kind in Kind::ALL {
            assert!(ask(kind).len() > 20, "{kind:?}");
        }
    }
}
