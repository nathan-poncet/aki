//! GitLab: merge requests I wrote, am assigned to, or review, and the
//! issues assigned to me.
//!
//! Each refresh costs three light list calls plus one issue call. The
//! expensive part, an MR's discussions, is read again only when the MR's
//! `updatedAt` moved; what was read is kept in the cache, so a cold start
//! pays only for what changed since the last run.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::thread;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::cache::Cache;
use super::session::{Auth, Credential, Session, mentions};
use crate::entities::change_request::{ChangeRequest, Checks, Note, ReviewState, Reviewer, Thread};
use crate::entities::item::{Actor, ItemRef, WorkItem};
use crate::entities::ticket::{StatusCategory, Ticket};
use crate::entities::time::Timestamp;
use crate::use_cases::ports::{SourceError, WorkSource};

const CONNECTIONS: [&str; 3] = [
    "authoredMergeRequests",
    "assignedMergeRequests",
    "reviewRequestedMergeRequests",
];

const MERGE_REQUEST: &str = "id iid title webUrl draft createdAt updatedAt sourceBranch \
    divergedFromTargetBranch conflicts approved approvalsLeft \
    project { fullPath } author { username bot } headPipeline { status } \
    userPermissions { canMerge } \
    assignees { nodes { username } } \
    reviewers { nodes { username bot mergeRequestInteraction { reviewState } } }";

const DISCUSSIONS: &str = "query($path: ID!, $iid: String!, $after: String) {
  project(fullPath: $path) { mergeRequest(iid: $iid) {
    discussions(first: 100, after: $after) {
      pageInfo { hasNextPage endCursor }
      nodes { resolved resolvable notes(first: 100) { nodes {
        author { username bot } createdAt system body url
      } } }
    }
  } }
}";

/// How many discussion reads run side by side.
const PARALLEL_READS: usize = 16;

/// What an MR's discussions tell, beyond the list.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct History {
    threads: Vec<Thread>,
    my_last_activity_at: Option<Timestamp>,
    last_push_at: Option<Timestamp>,
    review_requested_at: Option<Timestamp>,
}

pub struct GitLab {
    name: String,
    host: String,
    session: Session,
    /// MR id → (`updatedAt` it was read at, its history).
    histories: Mutex<HashMap<String, (String, History)>>,
    /// Where `histories` is kept between runs.
    memory: Option<Arc<Cache>>,
    recalled: Once,
    /// The MR ids read from the network and not stored yet.
    unsaved: Mutex<Vec<String>>,
}

impl GitLab {
    pub fn new(
        name: String,
        host: String,
        credential: Credential,
        memory: Option<Arc<Cache>>,
    ) -> Self {
        Self {
            name,
            host,
            session: Session::new(credential),
            histories: Mutex::new(HashMap::new()),
            memory,
            recalled: Once::new(),
            unsaved: Mutex::new(Vec::new()),
        }
    }

    fn graphql(&self, token: &str, query: &str, variables: Value) -> Result<Value, SourceError> {
        let url = format!("https://{}/api/graphql", self.host);
        self.session.graphql(&url, token, query, variables)
    }

    /// One of my three lists of open MRs, every page of it.
    fn connection(
        &self,
        token: &str,
        connection: &str,
    ) -> Result<(String, Vec<Value>), SourceError> {
        let query = format!(
            "query($after: String) {{ currentUser {{ username \
             {connection}(state: opened, first: 50, after: $after) {{ \
             pageInfo {{ hasNextPage endCursor }} nodes {{ {MERGE_REQUEST} }} }} }} }}"
        );
        let mut merge_requests = Vec::new();
        let mut after = Value::Null;
        loop {
            let mut data = self.graphql(token, &query, json!({ "after": after }))?;
            let user = &mut data["currentUser"];
            let me = user["username"]
                .as_str()
                .ok_or_else(|| SourceError::Auth("the token belongs to no user".into()))?
                .to_string();
            let page = &mut user[connection];
            if let Some(nodes) = page["nodes"].as_array_mut() {
                merge_requests.append(nodes);
            }
            if page["pageInfo"]["hasNextPage"].as_bool() != Some(true) {
                return Ok((me, merge_requests));
            }
            after = page["pageInfo"]["endCursor"].take();
        }
    }

    /// My username and every open MR I am involved in, once each.
    fn list(&self, token: &str) -> Result<(String, Vec<Value>), SourceError> {
        let lists: Vec<_> = thread::scope(|scope| {
            let readers: Vec<_> = CONNECTIONS
                .iter()
                .map(|connection| scope.spawn(move || self.connection(token, connection)))
                .collect();
            readers
                .into_iter()
                .map(|reader| reader.join().expect("a list read panicked"))
                .collect()
        });

        let mut me = String::new();
        let mut seen = Vec::new();
        let mut merge_requests = Vec::new();
        for list in lists {
            let (username, nodes) = list?;
            me = username;
            for node in nodes {
                let id = node["id"].as_str().unwrap_or_default().to_string();
                if !seen.contains(&id) {
                    seen.push(id);
                    merge_requests.push(node);
                }
            }
        }
        Ok((me, merge_requests))
    }

    /// The MR's history, from memory when the MR has not moved since.
    fn history(
        &self,
        token: &str,
        me: &str,
        merge_request: &Value,
    ) -> Result<History, SourceError> {
        let id = merge_request["id"].as_str().unwrap_or_default();
        let updated_at = merge_request["updatedAt"].as_str().unwrap_or_default();
        if let Some((read_at, history)) = self.histories.lock().unwrap().get(id)
            && read_at == updated_at
        {
            return Ok(history.clone());
        }

        let mut discussions = Vec::new();
        let mut after = Value::Null;
        loop {
            let variables = json!({
                "path": merge_request["project"]["fullPath"],
                "iid": merge_request["iid"],
                "after": after,
            });
            let mut data = self.graphql(token, DISCUSSIONS, variables)?;
            let page = &mut data["project"]["mergeRequest"]["discussions"];
            if let Some(nodes) = page["nodes"].as_array_mut() {
                discussions.append(nodes);
            }
            if page["pageInfo"]["hasNextPage"].as_bool() != Some(true) {
                break;
            }
            after = page["pageInfo"]["endCursor"].take();
        }

        let history = read_history(&discussions, me);
        self.histories
            .lock()
            .unwrap()
            .insert(id.to_string(), (updated_at.to_string(), history.clone()));
        self.unsaved.lock().unwrap().push(id.to_string());
        Ok(history)
    }

    fn issues(&self, token: &str) -> Result<Vec<WorkItem>, SourceError> {
        let url = format!(
            "https://{}/api/v4/issues?scope=assigned_to_me&state=opened&per_page=100",
            self.host
        );
        let issues = self.session.get(&url, Auth::Bearer(token))?;
        Ok(issues
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|issue| read_issue(issue, &self.name))
            .map(WorkItem::Ticket)
            .collect())
    }

    fn shelf(&self, me: &str) -> String {
        // Per source and per account: "me" is baked into what is stored.
        format!("gitlab-history/1/{}/{me}", self.name)
    }

    /// Loads what earlier runs read, once. Anything unreadable only means
    /// reading it again.
    fn recall(&self, me: &str) {
        self.recalled.call_once(|| {
            let Some(cache) = &self.memory else { return };
            *self.histories.lock().unwrap() = cache
                .all(&self.shelf(me))
                .into_iter()
                .filter_map(|(id, entry)| {
                    let history: SavedHistory = serde_json::from_str(&entry.value).ok()?;
                    Some((id, (entry.version, history.into())))
                })
                .collect();
        });
    }

    /// Stores what this reading learned, and drops what is no longer open.
    fn save(&self, me: &str, prune: bool) {
        let Some(cache) = &self.memory else { return };
        let shelf = self.shelf(me);
        let read = std::mem::take(&mut *self.unsaved.lock().unwrap());
        let histories = self.histories.lock().unwrap();
        for id in read {
            if let Some((at, history)) = histories.get(&id)
                && let Ok(text) = serde_json::to_string(&SavedHistory::new(at, history))
            {
                cache.put(&shelf, &id, at, &text);
            }
        }
        if prune {
            let open: Vec<&str> = histories.keys().map(String::as_str).collect();
            cache.keep_only(&shelf, &open);
        }
    }

    fn fetch_with(&self, token: &str) -> Result<Vec<WorkItem>, SourceError> {
        let (me, merge_requests) = self.list(token)?;
        self.recall(&me);

        // A few workers share the queue; the issues are read meanwhile.
        let next = AtomicUsize::new(0);
        let slots: Vec<Mutex<Option<Result<History, SourceError>>>> =
            merge_requests.iter().map(|_| Mutex::new(None)).collect();
        let issues = thread::scope(|scope| {
            for _ in 0..PARALLEL_READS.min(merge_requests.len()) {
                scope.spawn(|| {
                    loop {
                        let index = next.fetch_add(1, Ordering::Relaxed);
                        let Some(merge_request) = merge_requests.get(index) else {
                            break;
                        };
                        let history = self.history(token, &me, merge_request);
                        *slots[index].lock().unwrap() = Some(history);
                    }
                });
            }
            self.issues(token)
        });

        // Forget MRs that left the list, so the memory follows what is open.
        let open: Vec<&str> = merge_requests
            .iter()
            .filter_map(|merge_request| merge_request["id"].as_str())
            .collect();
        let pruned = {
            let mut histories = self.histories.lock().unwrap();
            let before = histories.len();
            histories.retain(|id, _| open.contains(&id.as_str()));
            histories.len() != before
        };
        self.save(&me, pruned);

        let mut items = Vec::new();
        for (merge_request, slot) in merge_requests.iter().zip(slots) {
            let history = slot
                .into_inner()
                .unwrap()
                .expect("every merge request was read");
            if let Some(change) = read_merge_request(merge_request, history?, &me, &self.name) {
                items.push(WorkItem::Change(change));
            }
        }
        items.extend(issues?);
        Ok(items)
    }
}

/// The stored form of a history. Holds who spoke when and where, never
/// what was said.
#[derive(Serialize, Deserialize)]
struct SavedHistory {
    at: String,
    activity: Option<i64>,
    push: Option<i64>,
    requested: Option<i64>,
    threads: Vec<SavedThread>,
}

#[derive(Serialize, Deserialize)]
struct SavedThread {
    url: String,
    resolvable: bool,
    resolved: bool,
    notes: Vec<SavedNote>,
}

#[derive(Serialize, Deserialize)]
struct SavedNote {
    name: String,
    me: bool,
    bot: bool,
    at: i64,
    mention: bool,
}

impl SavedHistory {
    fn new(at: &str, history: &History) -> Self {
        let seconds = |at: Option<Timestamp>| at.map(|at| at.0);
        Self {
            at: at.to_string(),
            activity: seconds(history.my_last_activity_at),
            push: seconds(history.last_push_at),
            requested: seconds(history.review_requested_at),
            threads: history
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
                            name: note.author.name.clone(),
                            me: note.author.is_me,
                            bot: note.author.is_bot,
                            at: note.at.0,
                            mention: note.mentions_me,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

impl From<SavedHistory> for History {
    fn from(saved: SavedHistory) -> Self {
        Self {
            my_last_activity_at: saved.activity.map(Timestamp),
            last_push_at: saved.push.map(Timestamp),
            review_requested_at: saved.requested.map(Timestamp),
            threads: saved
                .threads
                .into_iter()
                .map(|thread| Thread {
                    url: thread.url,
                    resolvable: thread.resolvable,
                    resolved: thread.resolved,
                    notes: thread
                        .notes
                        .into_iter()
                        .map(|note| Note {
                            author: Actor {
                                name: note.name,
                                is_me: note.me,
                                is_bot: note.bot,
                            },
                            at: Timestamp(note.at),
                            mentions_me: note.mention,
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

impl WorkSource for GitLab {
    fn name(&self) -> &str {
        &self.name
    }

    fn identity(&self) -> Result<String, SourceError> {
        self.session.with_token(|token| {
            let data = self.graphql(token, "query { currentUser { username } }", json!({}))?;
            data["currentUser"]["username"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| SourceError::Auth("the token belongs to no user".into()))
        })
    }

    fn fetch(&self) -> Result<Vec<WorkItem>, SourceError> {
        self.session.with_token(|token| self.fetch_with(token))
    }
}

fn actor(user: &Value, me: &str) -> Actor {
    let name = user["username"].as_str().unwrap_or("unknown").to_string();
    Actor {
        is_me: name == me,
        // Project and group access tokens act as users named `…_bot_<hash>`.
        is_bot: user["bot"].as_bool().unwrap_or(false) || name.contains("_bot_"),
        name,
    }
}

fn time(value: &Value) -> Option<Timestamp> {
    value.as_str().and_then(Timestamp::parse)
}

fn read_history(discussions: &[Value], me: &str) -> History {
    let mut history = History::default();
    let latest = |slot: &mut Option<Timestamp>, at: Timestamp| *slot = (*slot).max(Some(at));

    for discussion in discussions {
        let mut notes = Vec::new();
        let mut url = String::new();
        for note in discussion["notes"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
        {
            let Some(at) = time(&note["createdAt"]) else {
                continue;
            };
            let author = actor(&note["author"], me);
            let body = note["body"].as_str().unwrap_or_default();

            if note["system"].as_bool() == Some(true) {
                // GitLab tells its history in English sentences only.
                if body.starts_with("added ") && body.contains(" commit") {
                    latest(&mut history.last_push_at, at);
                } else if body.starts_with("requested review from") && mentions(body, me) {
                    latest(&mut history.review_requested_at, at);
                } else if author.is_me
                    && [
                        "approved this merge request",
                        "left review comments",
                        "requested changes",
                    ]
                    .iter()
                    .any(|act| body.starts_with(act))
                {
                    latest(&mut history.my_last_activity_at, at);
                }
                continue;
            }

            if url.is_empty() {
                url = note["url"].as_str().unwrap_or_default().to_string();
            }
            notes.push(Note {
                mentions_me: !author.is_me && mentions(body, me),
                author,
                at,
            });
        }
        if !notes.is_empty() {
            history.threads.push(Thread {
                url,
                resolvable: discussion["resolvable"].as_bool().unwrap_or(false),
                resolved: discussion["resolved"].as_bool().unwrap_or(false),
                notes,
            });
        }
    }
    history
}

fn read_merge_request(
    merge_request: &Value,
    history: History,
    me: &str,
    source: &str,
) -> Option<ChangeRequest> {
    let reviewers = merge_request["reviewers"]["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|reviewer| Reviewer {
            actor: actor(reviewer, me),
            state: match reviewer["mergeRequestInteraction"]["reviewState"].as_str() {
                Some("REVIEWED") => ReviewState::Commented,
                Some("REQUESTED_CHANGES") => ReviewState::ChangesRequested,
                Some("APPROVED") => ReviewState::Approved,
                Some("UNAPPROVED") => ReviewState::ApprovalReset,
                // UNREVIEWED, REVIEW_STARTED: nothing submitted yet.
                _ => ReviewState::Pending,
            },
        })
        .collect();

    let checks = match merge_request["headPipeline"]["status"].as_str() {
        Some("SUCCESS") => Checks::Passed,
        Some("FAILED") => Checks::Failed,
        Some(
            "CREATED"
            | "WAITING_FOR_RESOURCE"
            | "PREPARING"
            | "WAITING_FOR_CALLBACK"
            | "PENDING"
            | "RUNNING",
        ) => Checks::Running,
        // No pipeline, or one that was skipped, canceled or awaits a click.
        _ => Checks::None,
    };

    Some(ChangeRequest {
        item: ItemRef {
            source: source.to_string(),
            container: merge_request["project"]["fullPath"].as_str()?.to_string(),
            reference: format!("!{}", merge_request["iid"].as_str()?),
            title: merge_request["title"].as_str()?.to_string(),
            url: merge_request["webUrl"].as_str()?.to_string(),
            created_at: time(&merge_request["createdAt"])?,
            updated_at: time(&merge_request["updatedAt"])?,
        },
        branch: merge_request["sourceBranch"].as_str().map(str::to_string),
        tickets: Vec::new(),
        author: actor(&merge_request["author"], me),
        assigned_to_me: merge_request["assignees"]["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|assignee| assignee["username"].as_str() == Some(me)),
        reviewers,
        draft: merge_request["draft"].as_bool().unwrap_or(false),
        threads: history.threads,
        my_last_activity_at: history.my_last_activity_at,
        last_push_at: history.last_push_at,
        review_requested_at: history.review_requested_at,
        approved: merge_request["approved"].as_bool().unwrap_or(false),
        approvals_missing: merge_request["approvalsLeft"].as_u64().unwrap_or(0) as u32,
        checks,
        conflicts: merge_request["conflicts"].as_bool().unwrap_or(false),
        behind_target: merge_request["divergedFromTargetBranch"]
            .as_bool()
            .unwrap_or(false),
        i_can_merge: merge_request["userPermissions"]["canMerge"]
            .as_bool()
            .unwrap_or(true),
    })
}

fn read_issue(issue: &Value, source: &str) -> Option<Ticket> {
    let full = issue["references"]["full"].as_str()?;
    let (container, number) = full.rsplit_once('#')?;
    Some(Ticket {
        item: ItemRef {
            source: source.to_string(),
            container: container.to_string(),
            reference: format!("#{number}"),
            title: issue["title"].as_str()?.to_string(),
            url: issue["web_url"].as_str()?.to_string(),
            created_at: time(&issue["created_at"])?,
            updated_at: time(&issue["updated_at"])?,
        },
        status: "Open".to_string(),
        category: StatusCategory::Todo,
        priority: None,
        blocked_by: Vec::new(),
        mention_pending: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn note(author: &str, at: &str, system: bool, body: &str) -> Value {
        json!({
            "author": { "username": author, "bot": false },
            "createdAt": at, "system": system, "body": body,
            "url": format!("https://gitlab.test/mr/1#note_{author}"),
        })
    }

    fn discussion(resolvable: bool, resolved: bool, notes: Vec<Value>) -> Value {
        json!({ "resolvable": resolvable, "resolved": resolved, "notes": { "nodes": notes } })
    }

    #[test]
    fn history_separates_events_from_conversations() {
        let discussions = vec![
            discussion(
                false,
                false,
                vec![note(
                    "alex",
                    "2026-04-01T07:00:00Z",
                    true,
                    "requested review from @cv, @till, and @sam",
                )],
            ),
            discussion(
                false,
                false,
                vec![note(
                    "sam",
                    "2026-04-01T09:00:00Z",
                    true,
                    "approved this merge request",
                )],
            ),
            discussion(
                true,
                false,
                vec![
                    note("sam", "2026-04-01T08:00:00Z", false, "Why not a slice?"),
                    note("alex", "2026-04-02T08:00:00Z", false, "Done, @sam"),
                ],
            ),
            discussion(
                false,
                false,
                vec![note(
                    "alex",
                    "2026-04-02T11:00:00Z",
                    true,
                    "added 2 commits <ul><li>1a2b3c4d - Fix the parser</li></ul>",
                )],
            ),
            discussion(
                false,
                false,
                vec![note(
                    "alex",
                    "2026-04-02T11:01:00Z",
                    true,
                    "changed the description",
                )],
            ),
        ];
        let history = read_history(&discussions, "sam");

        let at = |text| Timestamp::parse(text);
        assert_eq!(history.review_requested_at, at("2026-04-01T07:00:00Z"));
        assert_eq!(history.my_last_activity_at, at("2026-04-01T09:00:00Z"));
        assert_eq!(history.last_push_at, at("2026-04-02T11:00:00Z"));

        assert_eq!(history.threads.len(), 1);
        let thread = &history.threads[0];
        assert!(thread.resolvable && !thread.resolved);
        assert_eq!(thread.url, "https://gitlab.test/mr/1#note_sam");
        assert!(thread.notes[0].author.is_me);
        assert!(thread.notes[1].mentions_me);
    }

    #[test]
    fn the_memory_survives_a_round_trip() {
        let discussions = vec![discussion(
            true,
            true,
            vec![
                note("sam", "2026-04-01T08:00:00Z", false, "Why not a slice?"),
                note("alex", "2026-04-02T08:00:00Z", false, "Done, @sam"),
            ],
        )];
        let mut history = read_history(&discussions, "sam");
        history.last_push_at = Some(Timestamp(42));

        let saved = SavedHistory::new("2026-04-02T08:00:00Z", &history);
        let text = serde_json::to_string(&saved).unwrap();
        // Who and when, never what.
        assert!(!text.contains("slice"));
        let back: SavedHistory = serde_json::from_str(&text).unwrap();
        assert_eq!(back.at, "2026-04-02T08:00:00Z");
        assert_eq!(History::from(back), history);
    }

    #[test]
    fn access_tokens_are_bots() {
        let user = json!({ "username": "group_123_bot_ab12", "bot": false });
        assert!(actor(&user, "sam").is_bot);
    }

    #[test]
    fn merge_request_maps_states() {
        let merge_request = json!({
            "id": "gid://gitlab/MergeRequest/1", "iid": "741", "title": "Add CSV export",
            "webUrl": "https://gitlab.test/g/web/-/merge_requests/741",
            "draft": false, "createdAt": "2026-03-27T08:49:00Z", "sourceBranch": "PROJ-120-export", "divergedFromTargetBranch": true,
            "updatedAt": "2026-04-02T11:23:08Z", "conflicts": true,
            "approved": false, "approvalsLeft": 2,
            "project": { "fullPath": "g/web" },
            "author": { "username": "alex", "bot": false },
            "headPipeline": { "status": "FAILED" },
            "assignees": { "nodes": [{ "username": "alex" }] },
            "reviewers": { "nodes": [
                { "username": "till", "bot": false,
                  "mergeRequestInteraction": { "reviewState": "REVIEWED" } },
                { "username": "sam", "bot": false,
                  "mergeRequestInteraction": { "reviewState": "UNAPPROVED" } },
            ] },
        });
        let change =
            read_merge_request(&merge_request, History::default(), "sam", "gitlab").unwrap();

        assert_eq!(change.item.id(), "gitlab:g/web!741");
        assert_eq!(change.item.short(), "web!741");
        assert_eq!(change.my_review(), Some(ReviewState::ApprovalReset));
        assert_eq!(change.reviewers[0].state, ReviewState::Commented);
        assert_eq!(change.checks, Checks::Failed);
        assert!(change.conflicts && !change.assigned_to_me && !change.author.is_me);
        assert_eq!(change.approvals_missing, 2);
        assert!(change.behind_target);
    }

    #[test]
    fn issue_reference_splits_project_and_number() {
        let issue = json!({
            "title": "Rotate keys", "web_url": "https://gitlab.test/g/web/-/issues/12",
            "created_at": "2026-04-01T11:23:08.123Z", "updated_at": "2026-04-02T11:23:08.123Z", "references": { "full": "g/web#12" },
        });
        let ticket = read_issue(&issue, "gitlab").unwrap();
        assert_eq!(ticket.item.container, "g/web");
        assert_eq!(ticket.item.reference, "#12");
    }
}
