//! GitHub: open pull requests that involve me and open issues assigned to
//! me, read through four searches of the GraphQL API, side by side.
//!
//! GitHub bills a query by the nodes it may return, and a refresh every
//! thirty seconds adds up fast. So the searches ask only for what is light
//! and can change silently (checks, review decision, requests); a pull
//! request's conversations, the expensive part, are read again only when
//! its `updatedAt` moved, and kept in the cache meanwhile.

use std::sync::Arc;
use std::thread;

use serde_json::{Value, json};

use super::cache::Cache;
use super::session::{Credential, Session, mentions};
use crate::entities::change_request::{ChangeRequest, Checks, Note, ReviewState, Reviewer, Thread};
use crate::entities::item::{Actor, ItemRef, WorkItem};
use crate::entities::ticket::{StatusCategory, Ticket};
use crate::entities::time::Timestamp;
use crate::use_cases::ports::{SourceError, WorkSource};

const SEARCH: &str = "query($q: String!, $after: String) {
  viewer { login }
  search(query: $q, type: ISSUE, first: 50, after: $after) {
    pageInfo { hasNextPage endCursor }
    nodes {
      __typename
      ... on Issue {
        number title url createdAt updatedAt
        repository { nameWithOwner }
        assignees(first: 20) { nodes { login } }
      }
      ... on PullRequest {
        id number title url createdAt updatedAt isDraft mergeable reviewDecision headRefName
        repository { nameWithOwner viewerPermission }
        author { __typename login }
        assignees(first: 20) { nodes { login } }
        reviewRequests(first: 30) { nodes { requestedReviewer { __typename ... on User { login } } } }
        latestReviews(first: 30) { nodes { author { __typename login } state } }
        commits(last: 1) { nodes { commit { committedDate statusCheckRollup { state } } } }
      }
    }
  }
}";

/// The conversations of a few pull requests, by id.
const CONVERSATIONS: &str = "query($ids: [ID!]!) {
  nodes(ids: $ids) {
    ... on PullRequest {
      id
      reviews(last: 50) { nodes { author { login } submittedAt } }
      reviewThreads(first: 60) { nodes { isResolved comments(first: 40) { nodes {
        author { __typename login } createdAt body url
      } } } }
      comments(last: 30) { nodes { author { __typename login } createdAt body url } }
      timelineItems(last: 30, itemTypes: [REVIEW_REQUESTED_EVENT]) { nodes {
        ... on ReviewRequestedEvent { createdAt requestedReviewer { ... on User { login } } }
      } }
    }
  }
}";

/// How many pull requests' conversations are asked for in one query.
const CONVERSATIONS_PER_QUERY: usize = 10;

/// What to look for. `@me` is the token's owner.
const SEARCHES: [&str; 4] = [
    "is:open archived:false is:pr involves:@me",
    "is:open archived:false is:pr review-requested:@me",
    "is:open archived:false is:pr reviewed-by:@me",
    "is:open archived:false is:issue assignee:@me",
];

/// Pages of 50 read per search before giving up on the tail.
const MAX_PAGES: usize = 4;

pub struct GitHub {
    name: String,
    host: String,
    session: Session,
    /// Where conversations already read are kept.
    cache: Arc<Cache>,
}

impl GitHub {
    pub fn new(name: String, host: String, credential: Credential, cache: Arc<Cache>) -> Self {
        Self {
            name,
            host,
            session: Session::new(credential),
            cache,
        }
    }

    fn graphql(&self, token: &str, query: &str, variables: Value) -> Result<Value, SourceError> {
        let url = if self.host == "github.com" {
            "https://api.github.com/graphql".to_string()
        } else {
            format!("https://{}/api/graphql", self.host)
        };
        self.session.graphql(&url, token, query, variables)
    }

    fn login(&self, token: &str) -> Result<String, SourceError> {
        let data = self.graphql(token, "query { viewer { login } }", json!({}))?;
        data["viewer"]["login"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| SourceError::Auth("the token belongs to no user".into()))
    }

    fn fetch_with(&self, token: &str) -> Result<Vec<WorkItem>, SourceError> {
        let answers: Vec<_> = thread::scope(|scope| {
            let readers: Vec<_> = SEARCHES
                .iter()
                .map(|search| scope.spawn(move || self.search(token, search)))
                .collect();
            readers
                .into_iter()
                .map(|reader| reader.join().expect("a search panicked"))
                .collect()
        });
        let mut me = String::new();
        let mut found = Vec::new();
        for answer in answers {
            let (login, nodes) = answer?;
            me = login;
            found.push(nodes);
        }

        let mut seen = Vec::new();
        let mut nodes = Vec::new();
        for node in found.into_iter().flatten() {
            let url = node["url"].as_str().unwrap_or_default().to_string();
            if !url.is_empty() && !seen.contains(&url) {
                seen.push(url);
                nodes.push(node);
            }
        }

        let pull_requests: Vec<&Value> = nodes
            .iter()
            .filter(|node| node["__typename"].as_str() == Some("PullRequest"))
            .collect();
        let conversations = self.conversations(token, &me, &pull_requests)?;

        Ok(nodes
            .iter()
            .filter_map(|node| match node["__typename"].as_str() {
                Some("PullRequest") => {
                    let talk = conversations
                        .iter()
                        .find(|(id, _)| Some(id.as_str()) == node["id"].as_str())
                        .map(|(_, talk)| talk)
                        .unwrap_or(&Value::Null);
                    read_pull_request(node, talk, &me, &self.name).map(WorkItem::Change)
                }
                Some("Issue") => read_issue(node, &me, &self.name).map(WorkItem::Ticket),
                _ => None,
            })
            .collect())
    }

    /// The conversations of these pull requests, by id: from the cache for
    /// those that have not moved, asked for in batches for the others.
    fn conversations(
        &self,
        token: &str,
        me: &str,
        pull_requests: &[&Value],
    ) -> Result<Vec<(String, Value)>, SourceError> {
        // Per account: who "me" is does not change what is stored, but
        // what another account may see does.
        let shelf = format!("github-conversation/1/{}/{me}", self.name);
        let mut known = Vec::new();
        let mut moved: Vec<(&str, &str)> = Vec::new();
        for pull_request in pull_requests {
            let (Some(id), Some(updated_at)) = (
                pull_request["id"].as_str(),
                pull_request["updatedAt"].as_str(),
            ) else {
                continue;
            };
            let cached = self
                .cache
                .get(&shelf, id)
                .filter(|entry| entry.version == updated_at)
                .and_then(|entry| serde_json::from_str(&entry.value).ok());
            match cached {
                Some(talk) => known.push((id.to_string(), talk)),
                None => moved.push((id, updated_at)),
            }
        }

        for batch in moved.chunks(CONVERSATIONS_PER_QUERY) {
            let ids: Vec<&str> = batch.iter().map(|(id, _)| *id).collect();
            let mut data = self.graphql(token, CONVERSATIONS, json!({ "ids": ids }))?;
            for talk in data["nodes"].as_array_mut().into_iter().flatten() {
                let Some(id) = talk["id"].as_str().map(str::to_string) else {
                    continue;
                };
                if let Some((_, updated_at)) = batch.iter().find(|(wanted, _)| *wanted == id) {
                    self.cache.put(&shelf, &id, updated_at, &talk.to_string());
                }
                known.push((id, talk.take()));
            }
        }

        let open: Vec<&str> = pull_requests
            .iter()
            .filter_map(|pull_request| pull_request["id"].as_str())
            .collect();
        if self.cache.all(&shelf).len() > open.len() {
            self.cache.keep_only(&shelf, &open);
        }
        Ok(known)
    }

    /// My login and every result of one search, up to `MAX_PAGES` pages.
    fn search(&self, token: &str, search: &str) -> Result<(String, Vec<Value>), SourceError> {
        let mut me = String::new();
        let mut nodes = Vec::new();
        let mut after = Value::Null;
        for _ in 0..MAX_PAGES {
            let mut data = self.graphql(token, SEARCH, json!({ "q": search, "after": after }))?;
            me = data["viewer"]["login"]
                .as_str()
                .ok_or_else(|| SourceError::Auth("the token belongs to no user".into()))?
                .to_string();
            let page = &mut data["search"];
            if let Some(found) = page["nodes"].as_array_mut() {
                nodes.append(found);
            }
            if page["pageInfo"]["hasNextPage"].as_bool() != Some(true) {
                break;
            }
            after = page["pageInfo"]["endCursor"].take();
        }
        Ok((me, nodes))
    }
}

impl WorkSource for GitHub {
    fn name(&self) -> &str {
        &self.name
    }

    fn identity(&self) -> Result<String, SourceError> {
        self.session.with_token(|token| self.login(token))
    }

    fn fetch(&self) -> Result<Vec<WorkItem>, SourceError> {
        self.session.with_token(|token| self.fetch_with(token))
    }
}

fn actor(user: &Value, me: &str) -> Actor {
    let name = user["login"].as_str().unwrap_or("ghost").to_string();
    Actor {
        is_me: name == me,
        is_bot: user["__typename"].as_str() == Some("Bot"),
        name,
    }
}

fn time(value: &Value) -> Option<Timestamp> {
    value.as_str().and_then(Timestamp::parse)
}

fn nodes(value: &Value) -> impl Iterator<Item = &Value> {
    value["nodes"].as_array().into_iter().flatten()
}

fn item(node: &Value, source: &str) -> Option<ItemRef> {
    Some(ItemRef {
        source: source.to_string(),
        container: node["repository"]["nameWithOwner"].as_str()?.to_string(),
        reference: format!("#{}", node["number"].as_u64()?),
        title: node["title"].as_str()?.to_string(),
        url: node["url"].as_str()?.to_string(),
        created_at: time(&node["createdAt"])?,
        updated_at: time(&node["updatedAt"])?,
    })
}

fn assigned_to(node: &Value, me: &str) -> bool {
    nodes(&node["assignees"]).any(|assignee| assignee["login"].as_str() == Some(me))
}

fn note(comment: &Value, me: &str) -> Option<Note> {
    let author = actor(&comment["author"], me);
    Some(Note {
        mentions_me: !author.is_me && mentions(comment["body"].as_str().unwrap_or_default(), me),
        author,
        at: time(&comment["createdAt"])?,
    })
}

/// `node` is what the search says of the pull request; `talk` is its
/// conversations, read apart.
fn read_pull_request(node: &Value, talk: &Value, me: &str, source: &str) -> Option<ChangeRequest> {
    // A pending request overrides whatever that person submitted before.
    let mut reviewers: Vec<Reviewer> = nodes(&node["reviewRequests"])
        .filter(|request| request["requestedReviewer"]["login"].is_string())
        .map(|request| Reviewer {
            actor: actor(&request["requestedReviewer"], me),
            state: ReviewState::Pending,
        })
        .collect();
    for review in nodes(&node["latestReviews"]) {
        let author = actor(&review["author"], me);
        if reviewers
            .iter()
            .any(|reviewer| reviewer.actor.name == author.name)
        {
            continue;
        }
        let state = match review["state"].as_str() {
            Some("APPROVED") => ReviewState::Approved,
            Some("CHANGES_REQUESTED") => ReviewState::ChangesRequested,
            Some("DISMISSED") => ReviewState::ApprovalReset,
            Some("COMMENTED") => ReviewState::Commented,
            // PENDING: a draft review only its author sees.
            _ => continue,
        };
        reviewers.push(Reviewer {
            actor: author,
            state,
        });
    }

    let mut threads: Vec<Thread> = nodes(&talk["reviewThreads"])
        .filter_map(|thread| {
            let comments = &thread["comments"];
            Some(Thread {
                url: nodes(comments).next()?["url"].as_str()?.to_string(),
                resolvable: true,
                resolved: thread["isResolved"].as_bool().unwrap_or(false),
                notes: nodes(comments)
                    .filter_map(|comment| note(comment, me))
                    .collect(),
            })
        })
        .collect();
    // Conversation comments are not threads: each stands alone and only
    // matters when it calls me.
    threads.extend(nodes(&talk["comments"]).filter_map(|comment| {
        Some(Thread {
            url: comment["url"].as_str()?.to_string(),
            resolvable: false,
            resolved: false,
            notes: vec![note(comment, me)?],
        })
    }));

    let head = &node["commits"]["nodes"][0]["commit"];
    let decision = node["reviewDecision"].as_str();

    Some(ChangeRequest {
        item: item(node, source)?,
        branch: node["headRefName"].as_str().map(str::to_string),
        tickets: Vec::new(),
        author: actor(&node["author"], me),
        assigned_to_me: assigned_to(node, me),
        reviewers,
        draft: node["isDraft"].as_bool().unwrap_or(false),
        threads,
        my_last_activity_at: nodes(&talk["reviews"])
            .filter(|review| review["author"]["login"].as_str() == Some(me))
            .filter_map(|review| time(&review["submittedAt"]))
            .max(),
        last_push_at: time(&head["committedDate"]),
        review_requested_at: nodes(&talk["timelineItems"])
            .filter(|event| event["requestedReviewer"]["login"].as_str() == Some(me))
            .filter_map(|event| time(&event["createdAt"]))
            .max(),
        // No decision means the repository requires no review at all.
        approved: matches!(decision, Some("APPROVED") | None),
        approvals_missing: match decision {
            Some("REVIEW_REQUIRED" | "CHANGES_REQUESTED") => 1,
            _ => 0,
        },
        checks: match head["statusCheckRollup"]["state"].as_str() {
            Some("SUCCESS") => Checks::Passed,
            Some("FAILURE" | "ERROR") => Checks::Failed,
            Some("PENDING" | "EXPECTED") => Checks::Running,
            _ => Checks::None,
        },
        conflicts: node["mergeable"].as_str() == Some("CONFLICTING"),
        i_can_merge: matches!(
            node["repository"]["viewerPermission"].as_str(),
            Some("ADMIN" | "MAINTAIN" | "WRITE")
        ),
    })
}

/// Only issues assigned to me are work; the others I merely talked on.
fn read_issue(node: &Value, me: &str, source: &str) -> Option<Ticket> {
    if !assigned_to(node, me) {
        return None;
    }
    Some(Ticket {
        item: item(node, source)?,
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

    fn pull_request() -> Value {
        json!({
            "__typename": "PullRequest", "number": 12, "title": "Fix chords",
            "url": "https://github.test/o/app/pull/12", "createdAt": "2026-09-18T10:00:00Z", "updatedAt": "2026-09-28T10:00:00Z",
            "isDraft": false, "mergeable": "CONFLICTING", "reviewDecision": "REVIEW_REQUIRED",
            "repository": { "nameWithOwner": "o/app", "viewerPermission": "READ" },
            "author": { "__typename": "User", "login": "ben" },
            "assignees": { "nodes": [] },
            "reviewRequests": { "nodes": [
                { "requestedReviewer": { "__typename": "User", "login": "sam" } },
                { "requestedReviewer": { "__typename": "Team" } },
            ] },
            "latestReviews": { "nodes": [
                { "author": { "__typename": "User", "login": "sam" }, "state": "COMMENTED" },
                { "author": { "__typename": "User", "login": "zoe" }, "state": "APPROVED" },
            ] },
            "reviews": { "nodes": [
                { "author": { "login": "sam" }, "submittedAt": "2026-09-20T10:00:00Z" },
                { "author": { "login": "zoe" }, "submittedAt": "2026-09-27T10:00:00Z" },
            ] },
            "reviewThreads": { "nodes": [ { "isResolved": false, "comments": { "nodes": [
                { "author": { "__typename": "User", "login": "sam" },
                  "createdAt": "2026-09-20T10:00:00Z", "body": "BaseCode?", "url": "https://t/1" },
                { "author": { "__typename": "User", "login": "ben" },
                  "createdAt": "2026-09-21T10:00:00Z", "body": "Fixed", "url": "https://t/2" },
            ] } } ] },
            "comments": { "nodes": [
                { "author": { "__typename": "Bot", "login": "ci" },
                  "createdAt": "2026-09-22T10:00:00Z", "body": "@sam coverage", "url": "https://c/1" },
            ] },
            "commits": { "nodes": [ { "commit": {
                "committedDate": "2026-09-25T10:00:00Z", "statusCheckRollup": { "state": "FAILURE" }
            } } ] },
            "timelineItems": { "nodes": [
                { "createdAt": "2026-09-19T10:00:00Z", "requestedReviewer": { "login": "sam" } },
                { "createdAt": "2026-09-26T10:00:00Z", "requestedReviewer": { "login": "sam" } },
                { "createdAt": "2026-09-27T10:00:00Z", "requestedReviewer": { "login": "zoe" } },
            ] },
        })
    }

    #[test]
    fn pull_request_maps_review_state_and_history() {
        let change = read_pull_request(&pull_request(), &pull_request(), "sam", "github").unwrap();

        assert_eq!(change.item.id(), "github:o/app#12");
        // Requested again: pending, even though I commented before.
        assert_eq!(change.my_review(), Some(ReviewState::Pending));
        assert_eq!(change.reviewers.len(), 2);
        assert_eq!(change.reviewers[1].state, ReviewState::Approved);

        let at = |text| Timestamp::parse(text);
        assert_eq!(change.my_last_activity_at, at("2026-09-20T10:00:00Z"));
        assert_eq!(change.review_requested_at, at("2026-09-26T10:00:00Z"));
        assert_eq!(change.last_push_at, at("2026-09-25T10:00:00Z"));

        assert_eq!(change.checks, Checks::Failed);
        assert!(change.conflicts && !change.approved && !change.i_can_merge);
        assert_eq!(change.approvals_missing, 1);

        assert_eq!(change.threads.len(), 2);
        assert!(change.threads[0].resolvable);
        assert_eq!(change.threads[0].url, "https://t/1");
        assert!(!change.threads[1].resolvable);
        assert!(change.threads[1].notes[0].author.is_bot);
        assert!(change.threads[1].notes[0].mentions_me);
    }

    #[test]
    fn a_pull_request_whose_conversations_are_unknown_still_shows() {
        let change = read_pull_request(&pull_request(), &Value::Null, "sam", "github").unwrap();
        assert_eq!(change.my_review(), Some(ReviewState::Pending));
        assert_eq!(change.checks, Checks::Failed);
        assert!(change.threads.is_empty());
        assert_eq!(change.my_last_activity_at, None);
    }

    #[test]
    fn a_repository_without_required_reviews_counts_as_approved() {
        let mut node = pull_request();
        node["reviewDecision"] = Value::Null;
        let change = read_pull_request(&node, &node, "sam", "github").unwrap();
        assert!(change.approved);
        assert_eq!(change.approvals_missing, 0);
    }

    #[test]
    fn only_assigned_issues_are_tickets() {
        let mut issue = json!({
            "__typename": "Issue", "number": 3, "title": "Crash", "url": "https://i/3",
            "createdAt": "2026-09-18T10:00:00Z", "updatedAt": "2026-09-28T10:00:00Z", "repository": { "nameWithOwner": "o/tool" },
            "assignees": { "nodes": [] },
        });
        assert!(read_issue(&issue, "sam", "github").is_none());
        issue["assignees"]["nodes"] = json!([{ "login": "sam" }]);
        assert_eq!(
            read_issue(&issue, "sam", "github").unwrap().item.short(),
            "tool#3"
        );
    }
}
