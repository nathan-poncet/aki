//! Jira Cloud: the tickets a JQL query selects (by default, mine and not done).

use std::sync::Arc;
use std::thread;
use std::time::Duration;

use serde_json::{Value, json};

use super::cache::Cache;
use super::session::{Auth, Credential, Session, base64};
use crate::entities::item::{ItemRef, WorkItem};
use crate::entities::ticket::{Priority, StatusCategory, Ticket};
use crate::entities::time::Timestamp;
use crate::use_cases::ports::{SourceError, WorkSource};

pub const DEFAULT_JQL: &str =
    "assignee = currentUser() AND statusCategory != Done ORDER BY updated DESC";

/// What is asked of Jira about a ticket.
const FIELDS: [&str; 7] = [
    "summary",
    "status",
    "priority",
    "created",
    "updated",
    "issuelinks",
    "comment",
];

/// How long a looked-up ticket is trusted: priorities seldom change.
const LOOKUP_FRESH_FOR: Duration = Duration::from_secs(30 * 60);

/// How many lookups run side by side.
const PARALLEL_LOOKUPS: usize = 8;

/// Pages of 100 tickets read before giving up on the tail.
const MAX_PAGES: usize = 5;

pub struct Jira {
    name: String,
    /// `https://company.atlassian.net`, without a trailing slash.
    site: String,
    email: String,
    jql: String,
    session: Session,
    /// Where tickets looked up for their priority are kept.
    cache: Option<Arc<Cache>>,
}

impl Jira {
    pub fn new(
        name: String,
        site: String,
        email: String,
        jql: String,
        credential: Credential,
        cache: Option<Arc<Cache>>,
    ) -> Self {
        Self {
            name,
            site: site.trim_end_matches('/').to_string(),
            email,
            jql,
            session: Session::new(credential),
            cache,
        }
    }

    fn basic(&self, token: &str) -> String {
        format!(
            "Basic {}",
            base64(format!("{}:{token}", self.email).as_bytes())
        )
    }

    fn myself(&self, token: &str) -> Result<Value, SourceError> {
        let url = format!("{}/rest/api/3/myself", self.site);
        self.session.get(&url, Auth::Raw(&self.basic(token)))
    }

    fn fetch_with(&self, token: &str) -> Result<Vec<WorkItem>, SourceError> {
        let myself = self.myself(token)?;
        let me = myself["accountId"]
            .as_str()
            .ok_or_else(|| SourceError::Auth("the token belongs to no user".into()))?;

        let url = format!("{}/rest/api/3/search/jql", self.site);
        let mut items = Vec::new();
        let mut next_page = Value::Null;
        for _ in 0..MAX_PAGES {
            let mut body = json!({
                "jql": self.jql,
                "maxResults": 100,
                "fields": FIELDS,
            });
            if next_page.is_string() {
                body["nextPageToken"] = next_page.clone();
            }
            let mut page = self
                .session
                .post(&url, Auth::Raw(&self.basic(token)), &body)?;
            items.extend(
                page["issues"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|issue| read_issue(issue, me, &self.site, &self.name))
                    .map(WorkItem::Ticket),
            );
            next_page = page["nextPageToken"].take();
            if !next_page.is_string() {
                break;
            }
        }
        Ok(items)
    }
}

impl Jira {
    /// One ticket by its key, whoever it belongs to. `Ok(None)`: Jira does
    /// not know it. Both answers are kept for a while.
    fn lookup(&self, token: &str, key: &str) -> Result<Option<Ticket>, SourceError> {
        let shelf = format!("jira-ticket/1/{}", self.name);
        let cached = self
            .cache
            .as_ref()
            .and_then(|cache| cache.get(&shelf, key))
            .filter(|entry| entry.age() < LOOKUP_FRESH_FOR);
        let issue: Value = match cached {
            Some(entry) => serde_json::from_str(&entry.value).unwrap_or(Value::Null),
            None => {
                let url = format!(
                    "{}/rest/api/3/issue/{key}?fields=summary,status,priority,created,updated",
                    self.site
                );
                let issue = match self.session.get(&url, Auth::Raw(&self.basic(token))) {
                    Ok(issue) => issue,
                    Err(SourceError::Unreachable(status)) if status == "HTTP 404" => Value::Null,
                    Err(error) => return Err(error),
                };
                if let Some(cache) = &self.cache {
                    cache.put(&shelf, key, "", &issue.to_string());
                }
                issue
            }
        };
        // Not mine to answer on: mentions do not matter here.
        Ok(read_issue(&issue, "", &self.site, &self.name))
    }

    fn lookup_all(&self, token: &str, keys: &[String]) -> Result<Vec<Ticket>, SourceError> {
        let mut tickets = Vec::new();
        for batch in keys.chunks(PARALLEL_LOOKUPS) {
            let answers: Vec<_> = thread::scope(|scope| {
                let lookups: Vec<_> = batch
                    .iter()
                    .map(|key| scope.spawn(move || self.lookup(token, key)))
                    .collect();
                lookups
                    .into_iter()
                    .map(|lookup| lookup.join().expect("a lookup panicked"))
                    .collect()
            });
            for answer in answers {
                tickets.extend(answer?);
            }
        }
        Ok(tickets)
    }
}

impl WorkSource for Jira {
    fn name(&self) -> &str {
        &self.name
    }

    fn identity(&self) -> Result<String, SourceError> {
        self.session.with_token(|token| {
            let myself = self.myself(token)?;
            myself["displayName"]
                .as_str()
                .map(str::to_string)
                .ok_or_else(|| SourceError::Auth("the token belongs to no user".into()))
        })
    }

    fn fetch(&self) -> Result<Vec<WorkItem>, SourceError> {
        self.session.with_token(|token| self.fetch_with(token))
    }

    fn tickets(&self, keys: &[String]) -> Result<Vec<Ticket>, SourceError> {
        self.session
            .with_token(|token| self.lookup_all(token, keys))
    }
}

/// Whether an Atlassian Document mentions the account, at any depth.
fn mentions(document: &Value, account: &str) -> bool {
    if document["type"].as_str() == Some("mention") {
        return document["attrs"]["id"].as_str() == Some(account);
    }
    document["content"]
        .as_array()
        .is_some_and(|children| children.iter().any(|child| mentions(child, account)))
}

fn read_issue(issue: &Value, me: &str, site: &str, source: &str) -> Option<Ticket> {
    let key = issue["key"].as_str()?;
    let fields = &issue["fields"];

    // "is blocked by": the link's inward side is the blocker.
    let blocked_by = fields["issuelinks"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|link| {
            link["type"]["inward"]
                .as_str()
                .is_some_and(|inward| inward.to_lowercase().contains("blocked by"))
        })
        .filter_map(|link| {
            let blocker = &link["inwardIssue"];
            let done =
                blocker["fields"]["status"]["statusCategory"]["key"].as_str() == Some("done");
            (!done).then(|| blocker["key"].as_str().map(str::to_string))?
        })
        .collect();

    let comments: Vec<(bool, Option<Timestamp>, bool)> = fields["comment"]["comments"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|comment| {
            (
                comment["author"]["accountId"].as_str() == Some(me),
                comment["created"].as_str().and_then(Timestamp::parse),
                mentions(&comment["body"], me),
            )
        })
        .collect();
    let my_last = comments
        .iter()
        .filter(|(mine, _, _)| *mine)
        .filter_map(|(_, at, _)| *at)
        .max();
    let mention_pending = comments
        .iter()
        .any(|(mine, at, calls_me)| !mine && *calls_me && *at > my_last);

    Some(Ticket {
        item: ItemRef {
            source: source.to_string(),
            container: key.split('-').next()?.to_string(),
            reference: key.to_string(),
            title: fields["summary"].as_str()?.to_string(),
            url: format!("{site}/browse/{key}"),
            created_at: fields["created"].as_str().and_then(Timestamp::parse)?,
            updated_at: fields["updated"].as_str().and_then(Timestamp::parse)?,
        },
        status: fields["status"]["name"].as_str()?.to_string(),
        category: match fields["status"]["statusCategory"]["key"].as_str() {
            Some("done") => StatusCategory::Done,
            Some("indeterminate") => StatusCategory::InProgress,
            _ => StatusCategory::Todo,
        },
        priority: fields["priority"]["name"]
            .as_str()
            .and_then(Priority::from_name),
        blocked_by,
        mention_pending,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn comment(account: &str, at: &str, mention: Option<&str>) -> Value {
        let mut content = vec![json!({ "type": "text", "text": "ping" })];
        if let Some(account) = mention {
            content.push(json!({ "type": "mention", "attrs": { "id": account } }));
        }
        json!({
            "author": { "accountId": account }, "created": at,
            "body": { "type": "doc", "content": [{ "type": "paragraph", "content": content }] },
        })
    }

    fn issue(comments: Vec<Value>) -> Value {
        json!({
            "key": "PROJ-120",
            "fields": {
                "summary": "Add CSV export", "created": "2026-03-02T11:23:08.000+0200",
                "updated": "2026-04-02T11:23:08.000+0200", "priority": { "name": "High" },
                "status": { "name": "In Progress", "statusCategory": { "key": "indeterminate" } },
                "issuelinks": [
                    { "type": { "inward": "is blocked by", "outward": "blocks" },
                      "inwardIssue": { "key": "PROJ-99",
                        "fields": { "status": { "statusCategory": { "key": "new" } } } } },
                    { "type": { "inward": "is blocked by", "outward": "blocks" },
                      "inwardIssue": { "key": "PROJ-98",
                        "fields": { "status": { "statusCategory": { "key": "done" } } } } },
                    { "type": { "inward": "is blocked by", "outward": "blocks" },
                      "outwardIssue": { "key": "PROJ-130",
                        "fields": { "status": { "statusCategory": { "key": "new" } } } } },
                ],
                "comment": { "comments": comments },
            },
        })
    }

    fn read(comments: Vec<Value>) -> Ticket {
        read_issue(&issue(comments), "me-id", "https://x.atlassian.net", "jira").unwrap()
    }

    #[test]
    fn maps_status_links_and_reference() {
        let ticket = read(Vec::new());
        assert_eq!(ticket.item.id(), "jira:PROJPROJ-120");
        assert_eq!(ticket.item.short(), "PROJ-120");
        assert_eq!(ticket.item.url, "https://x.atlassian.net/browse/PROJ-120");
        assert_eq!(ticket.category, StatusCategory::InProgress);
        assert_eq!(ticket.priority, Some(Priority::High));
        // Only the unfinished ticket that blocks this one, not the one it blocks.
        assert_eq!(ticket.blocked_by, ["PROJ-99"]);
        assert!(!ticket.mention_pending);
    }

    #[test]
    fn a_ticket_looked_up_for_its_priority_needs_few_fields() {
        let issue = json!({
            "key": "PROJ-9",
            "fields": {
                "summary": "Carry backups", "created": "2026-03-02T11:23:08.000+0200",
                "updated": "2026-04-02T11:23:08.000+0200",
                "priority": { "name": "Blocker" },
                "status": { "name": "To Do", "statusCategory": { "key": "new" } },
            },
        });
        let ticket = read_issue(&issue, "", "https://x.atlassian.net", "jira").unwrap();
        assert_eq!(ticket.item.reference, "PROJ-9");
        assert_eq!(ticket.priority, Some(Priority::Highest));
        // What a cached "Jira does not know it" reads as.
        assert!(read_issue(&Value::Null, "", "https://x.atlassian.net", "jira").is_none());
    }

    #[test]
    fn a_mention_is_pending_until_i_comment() {
        let call = comment("ana-id", "2026-04-02T10:00:00.000+0000", Some("me-id"));
        assert!(read(vec![call.clone()]).mention_pending);

        let answer = comment("me-id", "2026-04-02T11:00:00.000+0000", None);
        assert!(!read(vec![call.clone(), answer]).mention_pending);

        let someone_else = comment("ana-id", "2026-04-02T10:00:00.000+0000", Some("bob-id"));
        assert!(!read(vec![someone_else]).mention_pending);
    }
}
