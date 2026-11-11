//! The user's rules, typed. Every decision triage takes can be tuned here;
//! nothing below this ring ever sees a raw configuration string.

use std::collections::BTreeMap;

use super::task::{Bucket, Kind, Trigger};

/// Who is expected to resolve a review thread once it is answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resolver {
    /// Whoever opened it checks the answer and closes it.
    Reviewer,
    /// The author closes threads once addressed.
    Author,
    /// No convention: only "who spoke last" matters.
    Anyone,
}

impl Resolver {
    pub fn name(self) -> &'static str {
        match self {
            Resolver::Reviewer => "reviewer",
            Resolver::Author => "author",
            Resolver::Anyone => "anyone",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "reviewer" => Some(Resolver::Reviewer),
            "author" => Some(Resolver::Author),
            "anyone" => Some(Resolver::Anyone),
            _ => None,
        }
    }
}

/// What to do with someone else's draft I am a reviewer of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OthersDrafts {
    /// Treat it like any other review.
    Review,
    /// Show it as waiting on its author.
    Wait,
    /// Leave it off the list.
    Ignore,
}

impl OthersDrafts {
    pub fn name(self) -> &'static str {
        match self {
            OthersDrafts::Review => "review",
            OthersDrafts::Wait => "wait",
            OthersDrafts::Ignore => "ignore",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "review" => Some(OthersDrafts::Review),
            "wait" => Some(OthersDrafts::Wait),
            "ignore" => Some(OthersDrafts::Ignore),
            _ => None,
        }
    }
}

/// A user's override of one rule. `None` keeps the default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RuleOverride {
    pub enabled: Option<bool>,
    pub bucket: Option<Bucket>,
    pub priority: Option<u32>,
}

/// A rule after overrides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rule {
    pub enabled: bool,
    pub bucket: Bucket,
    pub priority: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rules {
    pub overrides: BTreeMap<Kind, RuleOverride>,
    /// Any of these makes a re-review due.
    pub re_review_triggers: Vec<Trigger>,
    /// Whether a re-review can be due on something I already approved.
    pub re_review_after_approval: bool,
    pub who_resolves: Resolver,
    /// Bots' comments never give anyone the ball.
    pub ignore_bots: bool,
    pub others_drafts: OthersDrafts,
    /// A waiting item idle this many days becomes a "nudge" to do. 0: never.
    pub nudge_after_days: u32,
    /// Ticket statuses that mean "not mine to move", compared case-insensitively.
    pub ticket_waiting_statuses: Vec<String>,
}

impl Default for Rules {
    fn default() -> Self {
        Self {
            overrides: BTreeMap::new(),
            re_review_triggers: vec![
                Trigger::AuthorReplied,
                Trigger::NewCommits,
                Trigger::ReRequested,
                Trigger::ApprovalReset,
            ],
            re_review_after_approval: false,
            who_resolves: Resolver::Reviewer,
            ignore_bots: true,
            others_drafts: OthersDrafts::Wait,
            nudge_after_days: 0,
            ticket_waiting_statuses: ["blocked", "on hold", "in review", "waiting"]
                .map(String::from)
                .to_vec(),
        }
    }
}

impl Rules {
    pub fn rule(&self, kind: Kind) -> Rule {
        let custom = self.overrides.get(&kind).copied().unwrap_or_default();
        Rule {
            enabled: custom.enabled.unwrap_or(true),
            bucket: custom.bucket.unwrap_or(kind.default_bucket()),
            priority: custom.priority.unwrap_or(kind.default_priority()),
        }
    }

    pub fn is_waiting_status(&self, status: &str) -> bool {
        let status = status.to_lowercase();
        self.ticket_waiting_statuses
            .iter()
            .any(|waiting| waiting.to_lowercase() == status)
    }
}
