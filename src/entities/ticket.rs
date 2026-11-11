//! An issue in a tracker (Jira, GitLab issues, GitHub issues) that is mine.

use super::item::ItemRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusCategory {
    Todo,
    InProgress,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub item: ItemRef,
    /// The tracker's own name for the status: "In Review", "Blocked"…
    pub status: String,
    pub category: StatusCategory,
    /// References of unfinished tickets that block this one.
    pub blocked_by: Vec<String>,
    /// Someone mentioned me in a comment and I have not commented since.
    pub mention_pending: bool,
}
