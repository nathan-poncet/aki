//! A source that remembers: what it read last is kept in the cache, served
//! again while it is fresh enough, and still there when the source is not.

use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::cache::Cache;
use crate::entities::change_request::{ChangeRequest, Checks, Note, ReviewState, Reviewer, Thread};
use crate::entities::item::{Actor, ItemRef, WorkItem};
use crate::entities::ticket::{Priority, StatusCategory, Ticket};
use crate::entities::time::Timestamp;
use crate::use_cases::ports::{Recalled, SourceError, WorkSource};

/// The shelf of last readings; its name carries the shape's version.
const SHELF: &str = "items/2";

pub struct CachedSource {
    inner: Box<dyn WorkSource>,
    cache: Arc<Cache>,
    /// How old a reading may be and still stand for a new one. Zero: always ask.
    fresh_for: Duration,
}

impl CachedSource {
    pub fn new(inner: Box<dyn WorkSource>, cache: Arc<Cache>, fresh_for: Duration) -> Self {
        Self {
            inner,
            cache,
            fresh_for,
        }
    }
}

impl WorkSource for CachedSource {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn identity(&self) -> Result<String, SourceError> {
        self.inner.identity()
    }

    fn fetch(&self) -> Result<Vec<WorkItem>, SourceError> {
        if let Some(entry) = self.cache.get(SHELF, self.name())
            && entry.age() < self.fresh_for
            && let Some(items) = decode(&entry.value)
        {
            return Ok(items);
        }
        let items = self.inner.fetch()?;
        self.cache.put(SHELF, self.name(), "", &encode(&items));
        Ok(items)
    }

    fn tickets(&self, keys: &[String]) -> Result<Vec<Ticket>, SourceError> {
        self.inner.tickets(keys)
    }

    fn recall(&self) -> Option<Recalled> {
        let entry = self.cache.get(SHELF, self.name())?;
        Some(Recalled {
            items: decode(&entry.value)?,
            at: Timestamp(entry.stored_at),
        })
    }
}

fn encode(items: &[WorkItem]) -> String {
    let saved: Vec<Saved> = items.iter().map(Saved::from).collect();
    serde_json::to_string(&saved).expect("items always serialize")
}

fn decode(text: &str) -> Option<Vec<WorkItem>> {
    let saved: Vec<Saved> = serde_json::from_str(text).ok()?;
    saved.into_iter().map(Saved::into_item).collect()
}

// The stored form: plain data, with enums as the words the entities use.

#[derive(Serialize, Deserialize)]
struct SavedItem {
    source: String,
    container: String,
    reference: String,
    title: String,
    url: String,
    created_at: i64,
    updated_at: i64,
}

#[derive(Serialize, Deserialize)]
struct SavedActor {
    name: String,
    me: bool,
    bot: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedNote {
    author: SavedActor,
    at: i64,
    mention: bool,
}

#[derive(Serialize, Deserialize)]
struct SavedThread {
    url: String,
    resolvable: bool,
    resolved: bool,
    notes: Vec<SavedNote>,
}

#[derive(Serialize, Deserialize)]
enum Saved {
    Change {
        item: SavedItem,
        branch: Option<String>,
        author: SavedActor,
        assigned_to_me: bool,
        reviewers: Vec<(SavedActor, String)>,
        draft: bool,
        threads: Vec<SavedThread>,
        my_last_activity_at: Option<i64>,
        last_push_at: Option<i64>,
        review_requested_at: Option<i64>,
        approved: bool,
        approvals_missing: u32,
        checks: String,
        conflicts: bool,
        behind_target: bool,
        i_can_merge: bool,
    },
    Ticket {
        item: SavedItem,
        status: String,
        category: String,
        priority: Option<String>,
        blocked_by: Vec<String>,
        mention_pending: bool,
    },
}

fn review_state(state: ReviewState) -> &'static str {
    match state {
        ReviewState::Pending => "pending",
        ReviewState::Commented => "commented",
        ReviewState::ChangesRequested => "changes_requested",
        ReviewState::Approved => "approved",
        ReviewState::ApprovalReset => "approval_reset",
    }
}

fn checks(checks: Checks) -> &'static str {
    match checks {
        Checks::None => "none",
        Checks::Running => "running",
        Checks::Passed => "passed",
        Checks::Failed => "failed",
    }
}

fn category(category: StatusCategory) -> &'static str {
    match category {
        StatusCategory::Todo => "todo",
        StatusCategory::InProgress => "in_progress",
        StatusCategory::Done => "done",
    }
}

impl From<&ItemRef> for SavedItem {
    fn from(item: &ItemRef) -> Self {
        Self {
            source: item.source.clone(),
            container: item.container.clone(),
            reference: item.reference.clone(),
            title: item.title.clone(),
            url: item.url.clone(),
            created_at: item.created_at.0,
            updated_at: item.updated_at.0,
        }
    }
}

impl From<SavedItem> for ItemRef {
    fn from(saved: SavedItem) -> Self {
        Self {
            source: saved.source,
            container: saved.container,
            reference: saved.reference,
            title: saved.title,
            url: saved.url,
            created_at: Timestamp(saved.created_at),
            updated_at: Timestamp(saved.updated_at),
        }
    }
}

impl From<&Actor> for SavedActor {
    fn from(actor: &Actor) -> Self {
        Self {
            name: actor.name.clone(),
            me: actor.is_me,
            bot: actor.is_bot,
        }
    }
}

impl From<SavedActor> for Actor {
    fn from(saved: SavedActor) -> Self {
        Self {
            name: saved.name,
            is_me: saved.me,
            is_bot: saved.bot,
        }
    }
}

impl From<&WorkItem> for Saved {
    fn from(item: &WorkItem) -> Self {
        let seconds = |at: Option<Timestamp>| at.map(|at| at.0);
        match item {
            WorkItem::Change(change) => Saved::Change {
                item: (&change.item).into(),
                branch: change.branch.clone(),
                author: (&change.author).into(),
                assigned_to_me: change.assigned_to_me,
                reviewers: change
                    .reviewers
                    .iter()
                    .map(|reviewer| {
                        (
                            (&reviewer.actor).into(),
                            review_state(reviewer.state).to_string(),
                        )
                    })
                    .collect(),
                draft: change.draft,
                threads: change
                    .threads
                    .iter()
                    .map(|thread| SavedThread {
                        url: thread.url.clone(),
                        resolvable: thread.resolvable,
                        resolved: thread.resolved,
                        notes: thread
                            .notes
                            .iter()
                            .map(|note| SavedNote {
                                author: (&note.author).into(),
                                at: note.at.0,
                                mention: note.mentions_me,
                            })
                            .collect(),
                    })
                    .collect(),
                my_last_activity_at: seconds(change.my_last_activity_at),
                last_push_at: seconds(change.last_push_at),
                review_requested_at: seconds(change.review_requested_at),
                approved: change.approved,
                approvals_missing: change.approvals_missing,
                checks: checks(change.checks).to_string(),
                conflicts: change.conflicts,
                behind_target: change.behind_target,
                i_can_merge: change.i_can_merge,
            },
            WorkItem::Ticket(ticket) => Saved::Ticket {
                item: (&ticket.item).into(),
                status: ticket.status.clone(),
                category: category(ticket.category).to_string(),
                priority: ticket.priority.map(|priority| priority.name().to_string()),
                blocked_by: ticket.blocked_by.clone(),
                mention_pending: ticket.mention_pending,
            },
        }
    }
}

impl Saved {
    /// `None` when a word is not one this version knows: the reading is
    /// then simply not used.
    fn into_item(self) -> Option<WorkItem> {
        Some(match self {
            Saved::Change {
                item,
                branch,
                author,
                assigned_to_me,
                reviewers,
                draft,
                threads,
                my_last_activity_at,
                last_push_at,
                review_requested_at,
                approved,
                approvals_missing,
                checks: checks_word,
                conflicts,
                behind_target,
                i_can_merge,
            } => WorkItem::Change(ChangeRequest {
                item: item.into(),
                branch,
                // Tied again to their tickets each time a board is assembled.
                tickets: Vec::new(),
                author: author.into(),
                assigned_to_me,
                reviewers: reviewers
                    .into_iter()
                    .map(|(actor, state)| {
                        Some(Reviewer {
                            actor: actor.into(),
                            state: [
                                ReviewState::Pending,
                                ReviewState::Commented,
                                ReviewState::ChangesRequested,
                                ReviewState::Approved,
                                ReviewState::ApprovalReset,
                            ]
                            .into_iter()
                            .find(|known| review_state(*known) == state)?,
                        })
                    })
                    .collect::<Option<_>>()?,
                draft,
                threads: threads
                    .into_iter()
                    .map(|thread| Thread {
                        url: thread.url,
                        resolvable: thread.resolvable,
                        resolved: thread.resolved,
                        notes: thread
                            .notes
                            .into_iter()
                            .map(|note| Note {
                                author: note.author.into(),
                                at: Timestamp(note.at),
                                mentions_me: note.mention,
                            })
                            .collect(),
                    })
                    .collect(),
                my_last_activity_at: my_last_activity_at.map(Timestamp),
                last_push_at: last_push_at.map(Timestamp),
                review_requested_at: review_requested_at.map(Timestamp),
                approved,
                approvals_missing,
                checks: [
                    Checks::None,
                    Checks::Running,
                    Checks::Passed,
                    Checks::Failed,
                ]
                .into_iter()
                .find(|known| checks(*known) == checks_word)?,
                conflicts,
                behind_target,
                i_can_merge,
            }),
            Saved::Ticket {
                item,
                status,
                category: category_word,
                priority,
                blocked_by,
                mention_pending,
            } => WorkItem::Ticket(Ticket {
                item: item.into(),
                status,
                category: [
                    StatusCategory::Todo,
                    StatusCategory::InProgress,
                    StatusCategory::Done,
                ]
                .into_iter()
                .find(|known| category(*known) == category_word)?,
                priority: priority.as_deref().and_then(Priority::from_name),
                blocked_by,
                mention_pending,
            }),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::use_cases::testing::{ScriptedSource, review_request, ticket_with_priority};

    fn items() -> Vec<WorkItem> {
        let mut change = review_request("!1");
        if let WorkItem::Change(change) = &mut change {
            change.branch = Some("PROJ-7-export".into());
            change.last_push_at = Some(Timestamp(450));
            change.checks = Checks::Failed;
            change.threads = vec![Thread {
                url: "https://example.test/!1#note_1".into(),
                resolvable: true,
                resolved: false,
                notes: vec![Note {
                    author: Actor::bot("ci"),
                    at: Timestamp(420),
                    mentions_me: true,
                }],
            }];
        }
        vec![change, ticket_with_priority("PROJ-7", Priority::High)]
    }

    #[test]
    fn items_survive_the_cache_whole() {
        assert_eq!(decode(&encode(&items())), Some(items()));
        assert_eq!(decode("not json"), None);
        assert_eq!(
            decode(&encode(&items()).replace("failed", "exploded")),
            None
        );
    }

    fn source(script: ScriptedSource, cache: &Arc<Cache>, fresh_for: u64) -> CachedSource {
        CachedSource::new(
            Box::new(script),
            cache.clone(),
            Duration::from_secs(fresh_for),
        )
    }

    #[test]
    fn a_fresh_reading_stands_for_a_new_one() {
        let cache = Arc::new(Cache::in_memory());
        let script = || ScriptedSource::script("gitlab", vec![Ok(items()), Ok(Vec::new())]);

        // Always asking: the second answer is the second reading.
        let eager = source(script(), &cache, 0);
        assert_eq!(eager.fetch().unwrap(), items());
        assert_eq!(eager.fetch().unwrap(), Vec::new());

        // Within the minute, a new process is served what the last one read.
        let lazy = source(script(), &cache, 60);
        assert_eq!(lazy.fetch().unwrap(), Vec::new());
        assert_eq!(lazy.recall().unwrap().items, Vec::new());
    }

    #[test]
    fn a_failing_source_still_recalls_its_last_reading() {
        let cache = Arc::new(Cache::in_memory());
        assert!(
            source(ScriptedSource::always("gitlab", items()), &cache, 0)
                .recall()
                .is_none()
        );
        source(ScriptedSource::always("gitlab", items()), &cache, 0)
            .fetch()
            .unwrap();

        let down = ScriptedSource::failing("gitlab", SourceError::Unreachable("offline".into()));
        let down = source(down, &cache, 0);
        assert!(down.fetch().is_err(), "the failure is not hidden");
        assert_eq!(down.recall().unwrap().items, items());

        // Another source's reading is not this one's.
        let other = source(ScriptedSource::always("github", Vec::new()), &cache, 60);
        assert!(other.recall().is_none());
    }
}
