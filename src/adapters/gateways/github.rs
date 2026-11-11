//! GitHub: open pull requests that involve me and open issues assigned to
//! me, read through four searches of the GraphQL API, side by side.

use std::thread;

use serde_json::{Value, json};

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
        number title url updatedAt
        repository { nameWithOwner }
        assignees(first: 20) { nodes { login } }
      }
      ... on PullRequest {
        number title url updatedAt isDraft mergeable reviewDecision
        repository { nameWithOwner viewerPermission }
        author { __typename login }
        assignees(first: 20) { nodes { login } }
        reviewRequests(first: 30) { nodes { requestedReviewer { __typename ... on User { login } } } }
        latestReviews(first: 30) { nodes { author { __typename login } state } }
        reviews(last: 50) { nodes { author { login } submittedAt } }
        reviewThreads(first: 60) { nodes { isResolved comments(first: 40) { nodes {
          author { __typename login } createdAt body url
        } } } }
        comments(last: 30) { nodes { author { __typename login } createdAt body url } }
        commits(last: 1) { nodes { commit { committedDate statusCheckRollup { state } } } }
        timelineItems(last: 30, itemTypes: [REVIEW_REQUESTED_EVENT]) { nodes {
          ... on ReviewRequestedEvent { createdAt requestedReviewer { ... on User { login } } }
        } }
      }
    }
  }
}";

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
}

impl GitHub {
    pub fn new(name: String, host: String, credential: Credential) -> Self {
        Self {
            name,
            host,
            session: Session::new(credential),
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
        let mut items = Vec::new();
        for nodes in found {
            for node in nodes {
                let url = node["url"].as_str().unwrap_or_default().to_string();
                if url.is_empty() || seen.contains(&url) {
                    continue;
                }
                seen.push(url);
                let item = match node["__typename"].as_str() {
                    Some("PullRequest") => {
                        read_pull_request(&node, &me, &self.name).map(WorkItem::Change)
                    }
                    Some("Issue") => read_issue(&node, &me, &self.name).map(WorkItem::Ticket),
                    _ => None,
                };
                items.extend(item);
            }
        }
        Ok(items)
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

fn read_pull_request(node: &Value, me: &str, source: &str) -> Option<ChangeRequest> {
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

    let mut threads: Vec<Thread> = nodes(&node["reviewThreads"])
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
    threads.extend(nodes(&node["comments"]).filter_map(|comment| {
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
        author: actor(&node["author"], me),
        assigned_to_me: assigned_to(node, me),
        reviewers,
        draft: node["isDraft"].as_bool().unwrap_or(false),
        threads,
        my_last_activity_at: nodes(&node["reviews"])
            .filter(|review| review["author"]["login"].as_str() == Some(me))
            .filter_map(|review| time(&review["submittedAt"]))
            .max(),
        last_push_at: time(&head["committedDate"]),
        review_requested_at: nodes(&node["timelineItems"])
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
            "url": "https://github.test/o/app/pull/12", "updatedAt": "2026-09-28T10:00:00Z",
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
        let change = read_pull_request(&pull_request(), "sam", "github").unwrap();

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
    fn a_repository_without_required_reviews_counts_as_approved() {
        let mut node = pull_request();
        node["reviewDecision"] = Value::Null;
        let change = read_pull_request(&node, "sam", "github").unwrap();
        assert!(change.approved);
        assert_eq!(change.approvals_missing, 0);
    }

    #[test]
    fn only_assigned_issues_are_tickets() {
        let mut issue = json!({
            "__typename": "Issue", "number": 3, "title": "Crash", "url": "https://i/3",
            "updatedAt": "2026-09-28T10:00:00Z", "repository": { "nameWithOwner": "o/tool" },
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
