//! What the product says about an item: why it is on the list, and on which
//! side of the line.

use super::item::ItemRef;
use super::time::Timestamp;

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
    pub const ALL: [Kind; 20] = [
        Kind::ReviewRequested,
        Kind::ReReview,
        Kind::ReplyNeeded,
        Kind::TicketMentioned,
        Kind::ThreadToResolve,
        Kind::ChecksFailed,
        Kind::Conflicts,
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
    /// When the ball last moved.
    pub since: Timestamp,
}

impl Task {
    /// Built by triage only; `reasons` must not be empty.
    pub(super) fn new(
        item: ItemRef,
        role: Role,
        counterpart: Option<String>,
        mut reasons: Vec<Reason>,
    ) -> Self {
        reasons.sort_by_key(|reason| (reason.bucket, reason.priority));
        let since = item.updated_at;
        Self {
            item,
            role,
            counterpart,
            reasons,
            since,
        }
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
