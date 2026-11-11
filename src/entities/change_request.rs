//! A merge request or pull request, reduced to what decides whose turn it is.

use super::item::{Actor, ItemRef};
use super::ticket::{LinkedTicket, keys_in};
use super::time::Timestamp;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewState {
    /// Asked to review, nothing submitted (or asked again since).
    Pending,
    /// Left a review without a verdict.
    Commented,
    ChangesRequested,
    Approved,
    /// Had approved; the approval was reset or dismissed (usually by a push).
    ApprovalReset,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reviewer {
    pub actor: Actor,
    pub state: ReviewState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checks {
    None,
    Running,
    Passed,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub author: Actor,
    pub at: Timestamp,
    pub mentions_me: bool,
}

/// A discussion. `resolvable` threads are review threads; the others are
/// plain comments, which only matter when they mention me.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thread {
    pub url: String,
    pub resolvable: bool,
    pub resolved: bool,
    pub notes: Vec<Note>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeRequest {
    pub item: ItemRef,
    /// The branch the change comes from, when the forge names it.
    pub branch: Option<String>,
    /// The tickets this change is about. Sources leave it empty: tying
    /// changes to tickets is done once everything is read.
    pub tickets: Vec<LinkedTicket>,
    pub author: Actor,
    pub assigned_to_me: bool,
    pub reviewers: Vec<Reviewer>,
    pub draft: bool,
    pub threads: Vec<Thread>,
    /// My last review act: a comment, a submitted review, an approval.
    pub my_last_activity_at: Option<Timestamp>,
    pub last_push_at: Option<Timestamp>,
    /// Last time someone asked me for a review.
    pub review_requested_at: Option<Timestamp>,
    pub approved: bool,
    pub approvals_missing: u32,
    pub checks: Checks,
    pub conflicts: bool,
    /// Whether merging is in my hands, or a maintainer's.
    pub i_can_merge: bool,
}

impl ChangeRequest {
    /// Ticket references written in the title or the branch name.
    pub fn ticket_keys(&self) -> Vec<String> {
        let mut keys = keys_in(&self.item.title);
        for key in self.branch.as_deref().map(keys_in).unwrap_or_default() {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        keys
    }

    pub fn my_review(&self) -> Option<ReviewState> {
        self.reviewers
            .iter()
            .find(|reviewer| reviewer.actor.is_me)
            .map(|reviewer| reviewer.state)
    }
}
