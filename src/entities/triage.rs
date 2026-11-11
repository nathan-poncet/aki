//! The heart of the product: given a snapshot of an item and the user's
//! rules, whose turn is it, and why?

use super::change_request::{ChangeRequest, Checks, ReviewState, Thread};
use super::item::WorkItem;
use super::rules::{OthersDrafts, Resolver, Rules};
use super::task::{Bucket, Detail, Kind, Reason, Role, Task, Trigger};
use super::ticket::{StatusCategory, Ticket};
use super::time::Timestamp;

/// `None` when the item asks nothing of anyone that concerns me.
pub fn triage(item: &WorkItem, rules: &Rules, now: Timestamp) -> Option<Task> {
    match item {
        WorkItem::Change(change) => triage_change(change, rules, now),
        WorkItem::Ticket(ticket) => triage_ticket(ticket, rules, now),
    }
}

/// Collects reasons, applying the user's per-rule overrides.
struct Reasons<'a> {
    rules: &'a Rules,
    found: Vec<Reason>,
}

impl<'a> Reasons<'a> {
    fn new(rules: &'a Rules) -> Self {
        Self {
            rules,
            found: Vec::new(),
        }
    }

    fn add(&mut self, kind: Kind, detail: Detail, url: Option<String>) {
        let rule = self.rules.rule(kind);
        if rule.enabled {
            self.found.push(Reason {
                kind,
                bucket: rule.bucket,
                priority: rule.priority,
                detail,
                url,
            });
        }
    }

    fn has_todo(&self) -> bool {
        self.found
            .iter()
            .any(|reason| reason.bucket == Bucket::Todo)
    }

    /// A waiting item nobody touched for too long becomes mine again: chase it.
    fn nudge_if_idle(&mut self, updated_at: Timestamp, now: Timestamp) {
        let limit = self.rules.nudge_after_days;
        let idle = updated_at.days_until(now);
        if limit > 0 && !self.found.is_empty() && !self.has_todo() && idle >= limit {
            self.add(Kind::Nudge, Detail::Idle(idle), None);
        }
    }
}

/// A thread seen from my side, once bots are set aside.
struct ThreadView<'a> {
    thread: &'a Thread,
    opened_by_me: bool,
    last_by_me: bool,
    i_took_part: bool,
    /// Someone mentioned me after the last thing I did.
    mention_pending: bool,
}

impl ThreadView<'_> {
    /// An open review thread.
    fn open(&self) -> bool {
        self.thread.resolvable && !self.thread.resolved
    }
}

fn view<'a>(
    thread: &'a Thread,
    ignore_bots: bool,
    my_last_act: Option<Timestamp>,
) -> Option<ThreadView<'a>> {
    let notes: Vec<_> = thread
        .notes
        .iter()
        .filter(|note| !(ignore_bots && note.author.is_bot))
        .collect();
    let first = notes.first()?;
    let last = notes.last()?;
    // I answered a bot and no human joined: nobody is waiting on anybody.
    let opened_by_ignored_bot = ignore_bots && thread.notes[0].author.is_bot;
    if opened_by_ignored_bot && notes.iter().all(|note| note.author.is_me) {
        return None;
    }
    let my_last_here = notes
        .iter()
        .filter(|note| note.author.is_me)
        .map(|note| note.at)
        .max();
    let answered_at = my_last_here.max(my_last_act);
    Some(ThreadView {
        thread,
        opened_by_me: first.author.is_me,
        last_by_me: last.author.is_me,
        i_took_part: my_last_here.is_some(),
        mention_pending: notes.iter().any(|note| {
            !note.author.is_me && note.mentions_me && answered_at.is_none_or(|at| note.at > at)
        }),
    })
}

fn count(threads: &[&ThreadView]) -> Detail {
    Detail::Threads(threads.len() as u32)
}

fn first_url(threads: &[&ThreadView]) -> Option<String> {
    threads.first().map(|view| view.thread.url.clone())
}

fn triage_change(change: &ChangeRequest, rules: &Rules, now: Timestamp) -> Option<Task> {
    // My last act on the item: a review act, or any comment of mine.
    let my_last_act = change
        .threads
        .iter()
        .flat_map(|thread| &thread.notes)
        .filter(|note| note.author.is_me)
        .map(|note| note.at)
        .max()
        .max(change.my_last_activity_at);

    let threads: Vec<ThreadView> = change
        .threads
        .iter()
        .filter_map(|thread| view(thread, rules.ignore_bots, my_last_act))
        .collect();

    let mut reasons = Reasons::new(rules);
    let role = if change.author.is_me {
        as_author(change, &threads, &mut reasons);
        Role::Author
    } else if let Some(state) = change.my_review() {
        if !as_reviewer(change, state, my_last_act, &threads, &mut reasons) {
            return None;
        }
        Role::Reviewer
    } else {
        if change.assigned_to_me {
            reasons.add(Kind::Assigned, Detail::None, None);
        }
        let waiting_on_me: Vec<_> = threads.iter().filter(|view| owes_reply(view)).collect();
        if !waiting_on_me.is_empty() {
            reasons.add(
                Kind::ReplyNeeded,
                count(&waiting_on_me),
                first_url(&waiting_on_me),
            );
        }
        if change.assigned_to_me {
            Role::Assignee
        } else {
            Role::Participant
        }
    };

    reasons.nudge_if_idle(change.item.updated_at, now);
    if reasons.found.is_empty() {
        return None;
    }
    let counterpart = (!change.author.is_me).then(|| change.author.name.clone());
    Some(Task::new(
        change.item.clone(),
        role,
        counterpart,
        reasons.found,
    ))
}

/// On someone else's thread: do I owe an answer?
fn owes_reply(view: &ThreadView) -> bool {
    if view.thread.resolvable {
        view.open() && !view.last_by_me && (view.i_took_part || view.mention_pending)
    } else {
        view.mention_pending
    }
}

fn as_author(change: &ChangeRequest, threads: &[ThreadView], reasons: &mut Reasons) {
    let rules = reasons.rules;

    // Open threads where a reviewer spoke last, and comments that call me.
    let to_answer: Vec<_> = threads
        .iter()
        .filter(|view| {
            if view.thread.resolvable {
                view.open() && !view.last_by_me
            } else {
                view.mention_pending
            }
        })
        .collect();
    if !to_answer.is_empty() {
        reasons.add(Kind::ReplyNeeded, count(&to_answer), first_url(&to_answer));
    }

    // Open threads where I spoke last: mine to close, or the reviewer's.
    let answered: Vec<_> = threads
        .iter()
        .filter(|view| view.open() && view.last_by_me)
        .collect();
    if !answered.is_empty() {
        let kind = match rules.who_resolves {
            Resolver::Author => Kind::ThreadToResolve,
            Resolver::Reviewer | Resolver::Anyone => Kind::AwaitingReply,
        };
        reasons.add(kind, count(&answered), first_url(&answered));
    }

    match change.checks {
        Checks::Failed => reasons.add(Kind::ChecksFailed, Detail::None, None),
        Checks::Running => reasons.add(Kind::ChecksRunning, Detail::None, None),
        Checks::Passed | Checks::None => {}
    }
    if change.conflicts {
        reasons.add(Kind::Conflicts, Detail::None, None);
    }

    if change.draft {
        reasons.add(Kind::DraftToFinish, Detail::None, None);
        return;
    }

    let any_open = threads.iter().any(ThreadView::open);
    let checks_green = matches!(change.checks, Checks::Passed | Checks::None);
    let fully_approved = change.approved && change.approvals_missing == 0;
    let others: Vec<_> = change
        .reviewers
        .iter()
        .filter(|reviewer| !reviewer.actor.is_me)
        .collect();

    let mergeable = fully_approved && checks_green && !change.conflicts && !any_open;
    if mergeable && change.i_can_merge {
        reasons.add(Kind::ReadyToMerge, Detail::None, None);
    } else if !fully_approved && others.is_empty() && change.i_can_merge {
        reasons.add(Kind::NoReviewer, Detail::None, None);
    } else if !fully_approved || !change.i_can_merge {
        // Either reviewers owe an approval, or only a maintainer can merge.
        let pending = others
            .iter()
            .filter(|reviewer| reviewer.state == ReviewState::Pending)
            .map(|reviewer| reviewer.actor.name.clone())
            .collect();
        reasons.add(
            Kind::AwaitingReview,
            Detail::Review {
                missing: change.approvals_missing,
                pending,
            },
            None,
        );
    }
}

/// Returns false when the item must stay off the list altogether.
fn as_reviewer(
    change: &ChangeRequest,
    state: ReviewState,
    my_last_act: Option<Timestamp>,
    threads: &[ThreadView],
    reasons: &mut Reasons,
) -> bool {
    let rules = reasons.rules;

    if change.draft {
        match rules.others_drafts {
            OthersDrafts::Review => {}
            OthersDrafts::Ignore => return false,
            OthersDrafts::Wait => {
                reasons.add(Kind::DraftByOther, Detail::None, None);
                return true;
            }
        }
    }

    let reviewed_before = my_last_act.is_some() || state != ReviewState::Pending;
    let after_my_act =
        |at: Option<Timestamp>| matches!((at, my_last_act), (Some(at), Some(mine)) if at > mine);

    // Threads I take part in where someone else spoke last.
    let replies: Vec<_> = threads
        .iter()
        .filter(|view| view.open() && view.i_took_part && !view.last_by_me)
        .collect();
    let mine_answered: Vec<_> = replies
        .iter()
        .copied()
        .filter(|view| view.opened_by_me)
        .collect();
    let mine_unanswered: Vec<_> = threads
        .iter()
        .filter(|view| view.open() && view.opened_by_me && view.last_by_me)
        .collect();
    let new_commits = after_my_act(change.last_push_at);

    if !reviewed_before {
        reasons.add(Kind::ReviewRequested, Detail::None, None);
    } else if state != ReviewState::Approved || rules.re_review_after_approval {
        let opened_any = threads.iter().any(|view| view.opened_by_me);
        let fired = |trigger: &Trigger| match trigger {
            Trigger::AuthorReplied => !replies.is_empty(),
            Trigger::NewCommits => new_commits,
            Trigger::ReRequested => {
                state == ReviewState::Pending && after_my_act(change.review_requested_at)
            }
            Trigger::ApprovalReset => state == ReviewState::ApprovalReset,
            Trigger::AllThreadsAnswered => {
                opened_any
                    && mine_unanswered.is_empty()
                    && (!mine_answered.is_empty() || new_commits)
            }
        };
        let triggers: Vec<Trigger> = rules
            .re_review_triggers
            .iter()
            .copied()
            .filter(|trigger| fired(trigger))
            .collect();
        if !triggers.is_empty() {
            reasons.add(
                Kind::ReReview,
                Detail::Triggers(triggers),
                first_url(&replies),
            );
        }
    }

    // Answered threads: mine to verify and close, unless the author closes them.
    let (to_resolve, to_answer): (Vec<_>, Vec<_>) = match rules.who_resolves {
        Resolver::Author => (
            Vec::new(),
            threads.iter().filter(|v| owes_reply(v)).collect(),
        ),
        Resolver::Reviewer | Resolver::Anyone => (
            mine_answered.clone(),
            threads
                .iter()
                .filter(|view| !view.opened_by_me && owes_reply(view))
                .collect(),
        ),
    };
    if !to_resolve.is_empty() {
        reasons.add(
            Kind::ThreadToResolve,
            count(&to_resolve),
            first_url(&to_resolve),
        );
    }
    if !to_answer.is_empty() {
        reasons.add(Kind::ReplyNeeded, count(&to_answer), first_url(&to_answer));
    }

    // Nothing for me: the author has the ball, unless I am done with it.
    if !reasons.has_todo() && reviewed_before && state != ReviewState::Approved {
        reasons.add(Kind::AwaitingAuthor, count(&mine_unanswered), None);
    }
    true
}

fn triage_ticket(ticket: &Ticket, rules: &Rules, now: Timestamp) -> Option<Task> {
    if ticket.category == StatusCategory::Done {
        return None;
    }
    let mut reasons = Reasons::new(rules);

    if ticket.mention_pending {
        reasons.add(Kind::TicketMentioned, Detail::None, None);
    }
    if !ticket.blocked_by.is_empty() {
        reasons.add(
            Kind::TicketBlocked,
            Detail::BlockedBy(ticket.blocked_by.clone()),
            None,
        );
    } else if rules.is_waiting_status(&ticket.status) {
        reasons.add(
            Kind::TicketBlocked,
            Detail::Status(ticket.status.clone()),
            None,
        );
    } else {
        let kind = match ticket.category {
            StatusCategory::InProgress => Kind::TicketInProgress,
            _ => Kind::TicketTodo,
        };
        reasons.add(kind, Detail::Status(ticket.status.clone()), None);
    }

    reasons.nudge_if_idle(ticket.item.updated_at, now);
    if reasons.found.is_empty() {
        return None;
    }
    Some(Task::new(
        ticket.item.clone(),
        Role::Assignee,
        None,
        reasons.found,
    ))
}
