//! What the product says about an item: why it is on the list, and on which
//! side of the line.

use super::handoff::AgentState;
use super::item::ItemRef;
use super::snooze::Until;
use super::ticket::{LinkedTicket, Priority};
use super::time::Timestamp;
use super::urgency::{Urgency, UrgencyRules, urgency};

/// The line the whole product draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Bucket {
    /// The next move is mine.
    Todo,
    /// I cannot move it further: someone or something else has the ball.
    Waiting,
}

impl Bucket {
    pub fn name(self) -> &'static str {
        match self {
            Bucket::Todo => "todo",
            Bucket::Waiting => "waiting",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "todo" => Some(Bucket::Todo),
            "waiting" => Some(Bucket::Waiting),
            _ => None,
        }
    }
}

/// Every reason an item can be on the list. One rule per kind; each can be
/// disabled, moved to the other bucket, or reprioritized by the user.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    ReviewRequested,
    ReReview,
    ReplyNeeded,
    TicketMentioned,
    ThreadToResolve,
    ChecksFailed,
    Conflicts,
    Rebase,
    ReadyToMerge,
    Assigned,
    NoReviewer,
    TicketInProgress,
    TicketTodo,
    Nudge,
    DraftToFinish,
    AwaitingReview,
    AwaitingAuthor,
    AwaitingReply,
    ChecksRunning,
    TicketBlocked,
    DraftByOther,
}

impl Kind {
    pub const ALL: [Kind; 21] = [
        Kind::ReviewRequested,
        Kind::ReReview,
        Kind::ReplyNeeded,
        Kind::TicketMentioned,
        Kind::ThreadToResolve,
        Kind::ChecksFailed,
        Kind::Conflicts,
        Kind::Rebase,
        Kind::ReadyToMerge,
        Kind::Assigned,
        Kind::NoReviewer,
        Kind::TicketInProgress,
        Kind::TicketTodo,
        Kind::Nudge,
        Kind::DraftToFinish,
        Kind::AwaitingReview,
        Kind::AwaitingAuthor,
        Kind::AwaitingReply,
        Kind::ChecksRunning,
        Kind::TicketBlocked,
        Kind::DraftByOther,
    ];

    /// The name used in the configuration file and in JSON output.
    pub fn name(self) -> &'static str {
        match self {
            Kind::ReviewRequested => "review_requested",
            Kind::ReReview => "re_review",
            Kind::ReplyNeeded => "reply_needed",
            Kind::TicketMentioned => "ticket_mentioned",
            Kind::ThreadToResolve => "thread_to_resolve",
            Kind::ChecksFailed => "checks_failed",
            Kind::Conflicts => "conflicts",
            Kind::Rebase => "rebase",
            Kind::ReadyToMerge => "ready_to_merge",
            Kind::Assigned => "assigned",
            Kind::NoReviewer => "no_reviewer",
            Kind::TicketInProgress => "ticket_in_progress",
            Kind::TicketTodo => "ticket_todo",
            Kind::Nudge => "nudge",
            Kind::DraftToFinish => "draft_to_finish",
            Kind::AwaitingReview => "awaiting_review",
            Kind::AwaitingAuthor => "awaiting_author",
            Kind::AwaitingReply => "awaiting_reply",
            Kind::ChecksRunning => "checks_running",
            Kind::TicketBlocked => "ticket_blocked",
            Kind::DraftByOther => "draft_by_other",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Kind::ALL.into_iter().find(|kind| kind.name() == name)
    }

    pub fn default_bucket(self) -> Bucket {
        match self {
            Kind::AwaitingReview
            | Kind::AwaitingAuthor
            | Kind::AwaitingReply
            | Kind::ChecksRunning
            | Kind::TicketBlocked
            | Kind::DraftByOther => Bucket::Waiting,
            _ => Bucket::Todo,
        }
    }

    /// Lower comes first. Spaced so the user can slot rules in between.
    pub fn default_priority(self) -> u32 {
        match self {
            Kind::ReviewRequested => 10,
            Kind::ReReview => 20,
            Kind::ReplyNeeded => 30,
            Kind::TicketMentioned => 35,
            Kind::ThreadToResolve => 40,
            Kind::ChecksFailed => 50,
            Kind::Conflicts => 55,
            Kind::Rebase => 57,
            Kind::ReadyToMerge => 60,
            Kind::Assigned => 65,
            Kind::NoReviewer => 70,
            Kind::TicketInProgress => 75,
            Kind::TicketTodo => 80,
            Kind::Nudge => 85,
            Kind::DraftToFinish => 90,
            Kind::AwaitingReview => 10,
            Kind::AwaitingAuthor => 20,
            Kind::AwaitingReply => 30,
            Kind::ChecksRunning => 40,
            Kind::TicketBlocked => 50,
            Kind::DraftByOther => 60,
        }
    }
}

/// What made a re-review due.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Trigger {
    /// Someone answered in a thread I take part in.
    AuthorReplied,
    /// Commits were pushed after my last review activity.
    NewCommits,
    /// My review was requested again.
    ReRequested,
    /// My approval was reset or dismissed.
    ApprovalReset,
    /// Every thread I opened has been answered or resolved.
    AllThreadsAnswered,
}

impl Trigger {
    pub const ALL: [Trigger; 5] = [
        Trigger::AuthorReplied,
        Trigger::NewCommits,
        Trigger::ReRequested,
        Trigger::ApprovalReset,
        Trigger::AllThreadsAnswered,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Trigger::AuthorReplied => "author_replied",
            Trigger::NewCommits => "new_commits",
            Trigger::ReRequested => "re_requested",
            Trigger::ApprovalReset => "approval_reset",
            Trigger::AllThreadsAnswered => "all_threads_answered",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Trigger::ALL
            .into_iter()
            .find(|trigger| trigger.name() == name)
    }
}

/// The facts behind a reason. Presenters choose the words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Detail {
    None,
    /// How many threads are concerned.
    Threads(u32),
    Triggers(Vec<Trigger>),
    /// Approvals still missing, and who has not reviewed yet.
    Review {
        missing: u32,
        pending: Vec<String>,
    },
    /// The tracker's status name.
    Status(String),
    BlockedBy(Vec<String>),
    /// Days without any movement.
    Idle(u32),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reason {
    pub kind: Kind,
    pub bucket: Bucket,
    pub priority: u32,
    pub detail: Detail,
    /// Where to act, when it is more precise than the item itself.
    pub url: Option<String>,
    /// Since when this has been true, as well as the source lets us know.
    pub since: Timestamp,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Author,
    Reviewer,
    Assignee,
    Participant,
}

impl Role {
    pub const ALL: [Role; 4] = [
        Role::Author,
        Role::Reviewer,
        Role::Assignee,
        Role::Participant,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Role::Author => "author",
            Role::Reviewer => "reviewer",
            Role::Assignee => "assignee",
            Role::Participant => "participant",
        }
    }
}

/// One item on the list, with every reason it is there, most pressing first.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub item: ItemRef,
    pub role: Role,
    /// Who I am dealing with: the author when I review, else nobody.
    pub counterpart: Option<String>,
    pub reasons: Vec<Reason>,
    /// Since when the most pressing reason has been waiting.
    pub since: Timestamp,
    /// The tickets this task is about, with what is known of them.
    pub tickets: Vec<LinkedTicket>,
    /// Its own priority, when it is a ticket.
    pub priority_level: Option<Priority>,
    pub urgency: Urgency,
    /// Set aside by the user, and until when. It is still true; it is just
    /// not shown with the rest.
    pub snoozed: Option<Until>,
    /// The agent the user opened on it, if its terminal is still there.
    pub agent: Option<AgentState>,
    /// The item is a draft change, mine or someone else's.
    pub draft: bool,
    /// Kept apart from the rest by the user's rules: shown only where it is
    /// asked for, counted and announced nowhere.
    pub apart: bool,
}

impl Task {
    /// Built by triage only; `reasons` must not be empty.
    #[allow(clippy::too_many_arguments)] // every part of a task, said once
    pub(super) fn new(
        item: ItemRef,
        role: Role,
        counterpart: Option<String>,
        mut reasons: Vec<Reason>,
        tickets: Vec<LinkedTicket>,
        priority_level: Option<Priority>,
        now: Timestamp,
        rules: &UrgencyRules,
    ) -> Self {
        reasons.sort_by_key(|reason| (reason.bucket, reason.priority));
        let since = reasons[0].since;
        let urgency = urgency(
            reasons[0].priority,
            since,
            now,
            priority_level,
            &tickets,
            rules,
        );
        Self {
            item,
            role,
            counterpart,
            reasons,
            since,
            tickets,
            priority_level,
            urgency,
            snoozed: None,
            agent: None,
            draft: false,
            apart: false,
        }
    }

    /// Whether it is with the rest: neither set aside nor kept apart.
    pub fn in_view(&self) -> bool {
        self.snoozed.is_none() && !self.apart
    }

    /// One thing to do is enough to make the whole item mine.
    pub fn bucket(&self) -> Bucket {
        self.reasons
            .first()
            .map(|reason| reason.bucket)
            .unwrap_or(Bucket::Waiting)
    }

    pub fn priority(&self) -> u32 {
        self.reasons
            .first()
            .map(|reason| reason.priority)
            .unwrap_or(u32::MAX)
    }

    pub fn headline(&self) -> &Reason {
        &self.reasons[0]
    }

    /// Where Enter should land: the most pressing reason's place, else the item.
    pub fn target_url(&self) -> &str {
        self.headline().url.as_deref().unwrap_or(&self.item.url)
    }
}

/// Ready-made tasks for tests, here and in the outer rings.
#[cfg(test)]
pub mod samples {
    use super::*;
    use crate::entities::urgency::Heat;

    pub fn reason(kind: Kind) -> Reason {
        Reason {
            kind,
            bucket: kind.default_bucket(),
            priority: kind.default_priority(),
            detail: Detail::None,
            url: None,
            since: Timestamp(0),
        }
    }

    /// A task listed for one rule; reviewing `by` when given, else mine.
    pub fn task(
        source: &str,
        container: &str,
        reference: &str,
        kind: Kind,
        by: Option<&str>,
    ) -> Task {
        Task {
            item: ItemRef {
                source: source.into(),
                container: container.into(),
                reference: reference.into(),
                title: "Add CSV export".into(),
                url: "https://example.test/7".into(),
                created_at: Timestamp(0),
                updated_at: Timestamp(0),
            },
            role: if by.is_some() {
                Role::Reviewer
            } else {
                Role::Author
            },
            counterpart: by.map(str::to_string),
            reasons: vec![reason(kind)],
            since: Timestamp(0),
            tickets: Vec::new(),
            priority_level: None,
            snoozed: None,
            agent: None,
            draft: false,
            apart: false,
            urgency: Urgency {
                rule: 100u32.saturating_sub(kind.default_priority()),
                waiting: 0,
                ticket: 0,
                heat: Heat::Calm,
            },
        }
    }
}
