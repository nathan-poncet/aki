//! What the use cases need from the outside world. Adapters implement these;
//! nothing here knows about HTTP, terminals or files.

use std::path::PathBuf;

use crate::entities::board::Board;
use crate::entities::desk::Desk;
use crate::entities::handoff::AgentState;
use crate::entities::item::{ItemRef, WorkItem};
use crate::entities::snooze::Snooze;
use crate::entities::task::Task;
use crate::entities::ticket::Ticket;
use crate::entities::time::Timestamp;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SourceError {
    /// No usable credentials, or they were refused.
    #[error("not signed in: {0}")]
    Auth(String),
    /// The source could not be reached or answered with an error.
    #[error("unreachable: {0}")]
    Unreachable(String),
    /// The source answered something we do not understand.
    #[error("unexpected answer: {0}")]
    Unexpected(String),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum DeskError {
    #[error("cannot read your spaces: {0}")]
    Unreadable(String),
    #[error("cannot save your spaces: {0}")]
    Unwritable(String),
}

/// Where the user's spaces are kept between runs.
pub trait DeskStore {
    /// `None` when nothing was ever saved.
    fn load(&self) -> Result<Option<Desk>, DeskError>;
    fn save(&self, desk: &Desk) -> Result<(), DeskError>;
}

/// What the user set aside, between runs.
pub trait Snoozes: Send + Sync {
    fn load(&self) -> Vec<Snooze>;
    fn save(&self, snoozes: &[Snooze]) -> Result<(), String>;
}

/// A place work lives in: a forge, a tracker.
pub trait WorkSource: Send + Sync {
    /// The name the user gave it; ends up in `ItemRef::source`.
    fn name(&self) -> &str;

    /// Who the source thinks I am. Used to check the connection.
    fn identity(&self) -> Result<String, SourceError>;

    /// Every open item I am involved in, as it stands right now.
    fn fetch(&self) -> Result<Vec<WorkItem>, SourceError>;

    /// The tickets with these keys that this source knows, whoever they
    /// belong to. Only trackers answer; keys they do not know are left out.
    fn tickets(&self, _keys: &[String]) -> Result<Vec<Ticket>, SourceError> {
        Ok(Vec::new())
    }

    /// What the last successful `fetch` returned, and when, if this source
    /// remembers: something to show before, or instead of, a fresh answer.
    fn recall(&self) -> Option<Recalled> {
        None
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recalled {
    pub items: Vec<WorkItem>,
    pub at: Timestamp,
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// Interrupts the user, outside the board, for what just became theirs.
pub trait Notifier: Send + Sync {
    fn announce(&self, news: &[&Task]);
}

/// Where a fresh board goes.
pub trait BoardView {
    /// A refresh has started.
    fn refreshing(&self);
    fn show(&self, board: Board);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pace {
    /// Time for another look.
    Again,
    Stop,
}

/// Decides when the watch looks again: a timer, a key press, a test script.
pub trait Pacer {
    fn wait(&self) -> Pace;
}

/// One terminal to open: where it goes, where it starts, what it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// The group it belongs to: a repository, usually.
    pub workspace: String,
    /// Its own name within the group: a ticket, a change.
    pub tab: String,
    pub directory: Option<PathBuf>,
    /// The command and its arguments, each exactly as it must arrive.
    pub command: Vec<String>,
    /// What the agent is told: on its own, for an agent that is already
    /// running and only needs to hear it.
    pub prompt: String,
    /// Whether the user is taken to it.
    pub focus: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Launched {
    Opened,
    /// A terminal with that name was already there with its agent running:
    /// the agent was told what there is to do now.
    Briefed,
    /// A terminal with that name was already there without an agent: the
    /// agent was started in it.
    Restarted,
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LaunchError {
    /// Nothing here can open a terminal.
    #[error("{0}")]
    Unavailable(String),
    #[error("could not open the agent: {0}")]
    Failed(String),
}

/// Whatever can open a named terminal running a command: a multiplexer, a
/// terminal emulator.
pub trait Launcher {
    fn launch(&self, session: &Session) -> Result<Launched, LaunchError>;

    /// The terminals open right now, by name, with what runs in them.
    /// A launcher that cannot tell says there are none.
    fn sessions(&self) -> Result<Vec<OpenSession>, LaunchError> {
        Ok(Vec::new())
    }
}

/// A terminal that is open: where, and what its agent is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenSession {
    pub workspace: String,
    pub tab: String,
    pub state: AgentState,
}

/// Where the repositories are cloned on this machine.
pub trait Checkouts {
    fn find(&self, item: &ItemRef) -> Option<PathBuf>;
}
