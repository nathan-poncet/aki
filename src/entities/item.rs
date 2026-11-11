//! What every piece of work has in common, whatever tool it lives in.

use super::change_request::ChangeRequest;
use super::ticket::Ticket;
use super::time::Timestamp;

/// Someone who acts on an item. The gateway decides who "me" is; the rules
/// never compare names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Actor {
    pub name: String,
    pub is_me: bool,
    pub is_bot: bool,
}

#[cfg(test)]
impl Actor {
    pub fn me(name: &str) -> Self {
        Self {
            name: name.to_string(),
            is_me: true,
            is_bot: false,
        }
    }

    pub fn other(name: &str) -> Self {
        Self {
            name: name.to_string(),
            is_me: false,
            is_bot: false,
        }
    }

    pub fn bot(name: &str) -> Self {
        Self {
            name: name.to_string(),
            is_me: false,
            is_bot: true,
        }
    }
}

/// How to name and reach an item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ItemRef {
    /// The configured source it came from: `gitlab`, `github`, `jira`…
    pub source: String,
    /// Repository or project: `group/project`, `owner/repo`, `PROJ`.
    pub container: String,
    /// Reference inside the container: `!741`, `#12`, `PROJ-120`.
    pub reference: String,
    pub title: String,
    pub url: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
}

impl ItemRef {
    /// Stable across refreshes; identifies the item in diffs.
    pub fn id(&self) -> String {
        format!("{}:{}{}", self.source, self.container, self.reference)
    }

    /// Short human name: last path segment plus reference (`web!741`), or
    /// the reference alone when it already names the container (`PROJ-120`).
    pub fn short(&self) -> String {
        let tail = self.container.rsplit('/').next().unwrap_or("");
        if self.reference.starts_with(tail) && !tail.is_empty() {
            self.reference.clone()
        } else {
            format!("{tail}{}", self.reference)
        }
    }
}

/// A snapshot of one piece of work, as a source saw it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkItem {
    Change(ChangeRequest),
    Ticket(Ticket),
}
