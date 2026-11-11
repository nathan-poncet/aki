//! A filter over tasks, written as a short query:
//!
//! ```text
//! is:todo source:gitlab repo:web,api -kind:draft by:alex export
//! ```
//!
//! Keys: `is` (todo, waiting), `source`, `repo`, `kind` (any reason the task
//! is listed for), `top` (its most pressing reason), `role`, `by`, `heat`
//! (hot, warm, calm), `ticket` (part of a linked ticket's key, or `any`),
//! `priority` (of the ticket, or of the tickets a change is about).
//!
//! Terms are separated by spaces and must all hold. `key:a,b` holds when
//! any value does; a leading `-` negates the term; a bare word is searched
//! in the reference, the title, the repository and the counterpart.

use super::task::{Bucket, Kind, Role, Task};
use super::ticket::Priority;
use super::urgency::Heat;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FilterError {
    #[error(
        "unknown filter \"{0}:\" (use is, source, repo, kind, top, role, by, heat, ticket, priority)"
    )]
    UnknownKey(String),
    #[error("\"{key}:\" needs a value")]
    MissingValue { key: String },
    #[error("\"{key}:{value}\" matches nothing (expected: {expected})")]
    UnknownValue {
        key: &'static str,
        value: String,
        expected: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Test {
    Bucket(Vec<Bucket>),
    Source(Vec<String>),
    Repo(Vec<String>),
    Kind(Vec<Kind>),
    Top(Vec<Kind>),
    Role(Vec<Role>),
    By(Vec<String>),
    Heat(Vec<Heat>),
    /// Parts of ticket keys; `any` is any ticket at all.
    Ticket(Vec<String>),
    Priority(Vec<Priority>),
    Text(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
    negated: bool,
    test: Test,
}

/// Parsed once, matched many times. Keeps the query it came from, so it can
/// be shown, edited and saved as the user wrote it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    query: String,
    terms: Vec<Term>,
}

fn contains(haystack: &str, needle: &str) -> bool {
    haystack.to_lowercase().contains(needle)
}

impl Filter {
    /// Matches everything.
    pub fn all() -> Self {
        Self::default()
    }

    pub fn parse(query: &str) -> Result<Self, FilterError> {
        let mut terms = Vec::new();
        for word in query.split_whitespace() {
            let (negated, word) = match word.strip_prefix('-') {
                Some(rest) if !rest.is_empty() => (true, rest),
                _ => (false, word),
            };
            let test = match word.split_once(':') {
                // A URL or a time is text, not a key.
                Some((key, rest))
                    if rest.starts_with('/') || !key.chars().all(|c| c.is_ascii_alphabetic()) =>
                {
                    Test::Text(word.to_lowercase())
                }
                Some((key, values)) => {
                    let values: Vec<String> = values
                        .split(',')
                        .filter(|value| !value.is_empty())
                        .map(str::to_lowercase)
                        .collect();
                    if values.is_empty() {
                        return Err(FilterError::MissingValue {
                            key: key.to_string(),
                        });
                    }
                    Self::keyed(key, values)?
                }
                None => Test::Text(word.to_lowercase()),
            };
            terms.push(Term { negated, test });
        }
        Ok(Self {
            query: query.split_whitespace().collect::<Vec<_>>().join(" "),
            terms,
        })
    }

    fn keyed(key: &str, values: Vec<String>) -> Result<Test, FilterError> {
        match key {
            "is" => values
                .iter()
                .map(|value| {
                    Bucket::from_name(value).ok_or_else(|| FilterError::UnknownValue {
                        key: "is",
                        value: value.clone(),
                        expected: "todo, waiting".to_string(),
                    })
                })
                .collect::<Result<_, _>>()
                .map(Test::Bucket),
            "role" => values
                .iter()
                .map(|value| {
                    Role::ALL
                        .into_iter()
                        .find(|role| role.name() == value)
                        .ok_or_else(|| FilterError::UnknownValue {
                            key: "role",
                            value: value.clone(),
                            expected: Role::ALL.map(Role::name).join(", "),
                        })
                })
                .collect::<Result<_, _>>()
                .map(Test::Role),
            // `kind:review` is every rule whose name says "review"; `kind:` looks
            // at every reason of a task, `top:` only at its most pressing one.
            "kind" | "top" => {
                let mut kinds = Vec::new();
                for value in &values {
                    let named: Vec<Kind> = Kind::ALL
                        .into_iter()
                        .filter(|kind| kind.name().contains(value.as_str()))
                        .collect();
                    if named.is_empty() {
                        return Err(FilterError::UnknownValue {
                            key: if key == "top" { "top" } else { "kind" },
                            value: value.clone(),
                            expected: Kind::ALL.map(Kind::name).join(", "),
                        });
                    }
                    kinds.extend(named);
                }
                Ok(if key == "top" {
                    Test::Top(kinds)
                } else {
                    Test::Kind(kinds)
                })
            }
            "heat" => values
                .iter()
                .map(|value| {
                    [Heat::Hot, Heat::Warm, Heat::Calm]
                        .into_iter()
                        .find(|heat| heat.name() == value)
                        .ok_or_else(|| FilterError::UnknownValue {
                            key: "heat",
                            value: value.clone(),
                            expected: "hot, warm, calm".to_string(),
                        })
                })
                .collect::<Result<_, _>>()
                .map(Test::Heat),
            "priority" => values
                .iter()
                .map(|value| {
                    Priority::ALL
                        .into_iter()
                        .find(|priority| priority.name() == value)
                        .ok_or_else(|| FilterError::UnknownValue {
                            key: "priority",
                            value: value.clone(),
                            expected: "highest, high, medium, low, lowest".to_string(),
                        })
                })
                .collect::<Result<_, _>>()
                .map(Test::Priority),
            "ticket" => Ok(Test::Ticket(values)),
            "source" => Ok(Test::Source(values)),
            "repo" => Ok(Test::Repo(values)),
            "by" => Ok(Test::By(values)),
            other => Err(FilterError::UnknownKey(other.to_string())),
        }
    }

    /// The query as the user wrote it, spaces normalized.
    pub fn query(&self) -> &str {
        &self.query
    }

    pub fn is_all(&self) -> bool {
        self.terms.is_empty()
    }

    pub fn matches(&self, task: &Task) -> bool {
        self.terms.iter().all(|term| {
            let holds = match &term.test {
                Test::Bucket(buckets) => buckets.contains(&task.bucket()),
                Test::Source(names) => names
                    .iter()
                    .any(|name| task.item.source.to_lowercase() == *name),
                Test::Repo(parts) => parts
                    .iter()
                    .any(|part| contains(&task.item.container, part)),
                Test::Heat(heats) => heats.contains(&task.urgency.heat),
                Test::Ticket(parts) => parts.iter().any(|part| {
                    (part == "any" && !task.tickets.is_empty())
                        || task
                            .tickets
                            .iter()
                            .any(|ticket| contains(&ticket.key, part))
                }),
                Test::Priority(priorities) => {
                    task.priority_level
                        .is_some_and(|priority| priorities.contains(&priority))
                        || task
                            .tickets
                            .iter()
                            .filter_map(|ticket| ticket.priority)
                            .any(|priority| priorities.contains(&priority))
                }
                Test::Top(kinds) => kinds.contains(&task.headline().kind),
                Test::Kind(kinds) => task
                    .reasons
                    .iter()
                    .any(|reason| kinds.contains(&reason.kind)),
                Test::Role(roles) => roles.contains(&task.role),
                Test::By(parts) => task
                    .counterpart
                    .as_deref()
                    .is_some_and(|name| parts.iter().any(|part| contains(name, part))),
                Test::Text(text) => {
                    contains(&task.item.short(), text)
                        || contains(&task.item.title, text)
                        || contains(&task.item.container, text)
                        || task
                            .counterpart
                            .as_deref()
                            .is_some_and(|name| contains(name, text))
                }
            };
            holds != term.negated
        })
    }

    /// The side this filter keeps, when it keeps exactly one.
    pub fn bucket(&self) -> Option<Bucket> {
        self.terms.iter().find_map(|term| match &term.test {
            Test::Bucket(buckets) if !term.negated && buckets.len() == 1 => Some(buckets[0]),
            _ => None,
        })
    }

    /// The same filter, looking at one side, or at both (`None`).
    pub fn with_bucket(&self, bucket: Option<Bucket>) -> Self {
        let mut words: Vec<String> = bucket
            .map(|bucket| format!("is:{}", bucket.name()))
            .into_iter()
            .collect();
        words.extend(
            self.query
                .split_whitespace()
                .filter(|word| !word.trim_start_matches('-').starts_with("is:"))
                .map(str::to_string),
        );
        let mut terms: Vec<Term> = bucket
            .map(|bucket| Term {
                negated: false,
                test: Test::Bucket(vec![bucket]),
            })
            .into_iter()
            .collect();
        terms.extend(
            self.terms
                .iter()
                .filter(|term| !matches!(term.test, Test::Bucket(_)))
                .cloned(),
        );
        Self {
            query: words.join(" "),
            terms,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::task::samples;

    fn task(source: &str, container: &str, kind: Kind, by: Option<&str>) -> Task {
        samples::task(source, container, "!7", kind, by)
    }

    fn keeps(query: &str, task: &Task) -> bool {
        Filter::parse(query).unwrap().matches(task)
    }

    #[test]
    fn the_empty_filter_keeps_everything() {
        let review = task("gitlab", "group/web", Kind::ReviewRequested, Some("alex"));
        assert!(Filter::all().matches(&review));
        assert!(keeps("   ", &review));
        assert!(Filter::parse("").unwrap().is_all());
    }

    #[test]
    fn every_term_must_hold_and_any_value_is_enough() {
        let review = task("gitlab", "group/web", Kind::ReviewRequested, Some("Alex.P"));
        assert!(keeps("is:todo source:gitlab repo:api,WEB by:alex", &review));
        assert!(!keeps("is:todo source:github", &review));
        assert!(!keeps("is:waiting", &review));
        assert!(keeps("role:reviewer", &review));
        assert!(!keeps("role:author,assignee", &review));
    }

    #[test]
    fn kinds_match_by_part_of_their_name() {
        let review = task("gitlab", "group/web", Kind::ReReview, Some("alex"));
        assert!(keeps("kind:review", &review));
        assert!(keeps("kind:re_review", &review));
        assert!(!keeps("kind:checks", &review));
    }

    #[test]
    fn top_looks_only_at_the_most_pressing_reason() {
        let mut draft = task("gitlab", "group/web", Kind::ReplyNeeded, None);
        draft.reasons.push(samples::reason(Kind::DraftToFinish));
        assert!(keeps("kind:draft", &draft));
        assert!(
            !keeps("top:draft", &draft),
            "a reply comes before the draft"
        );
        assert!(keeps("top:reply", &draft));
    }

    #[test]
    fn urgency_and_tickets_can_be_filtered_on() {
        use crate::entities::ticket::LinkedTicket;

        let mut review = task("gitlab", "group/web", Kind::ReviewRequested, Some("alex"));
        assert!(keeps("heat:calm", &review));
        assert!(!keeps("ticket:any", &review));
        assert!(!keeps("priority:high", &review));

        review.urgency.heat = Heat::Hot;
        review.tickets = vec![LinkedTicket {
            key: "PROJ-12".into(),
            priority: Some(Priority::High),
        }];
        assert!(keeps("heat:hot,warm", &review));
        assert!(keeps("ticket:any", &review));
        assert!(keeps("ticket:proj", &review));
        assert!(!keeps("ticket:other", &review));
        assert!(keeps("priority:high,highest", &review));

        // A ticket's own priority counts too.
        let mut ticket = task("jira", "PROJ", Kind::TicketTodo, None);
        ticket.priority_level = Some(Priority::Lowest);
        assert!(keeps("priority:lowest", &ticket));
        assert!(matches!(
            Filter::parse("heat:tepid"),
            Err(FilterError::UnknownValue { key: "heat", .. })
        ));
    }

    #[test]
    fn a_dash_negates_and_bare_words_search() {
        let draft = task("gitlab", "group/web", Kind::DraftToFinish, None);
        assert!(!keeps("-kind:draft", &draft));
        assert!(keeps("-source:github", &draft));
        assert!(keeps("export", &draft));
        assert!(keeps("web!7", &draft));
        assert!(!keeps("-export", &draft));
        // Nobody to match: `by:` fails, `-by:` holds.
        assert!(!keeps("by:alex", &draft));
        assert!(keeps("-by:alex", &draft));
    }

    #[test]
    fn mistakes_are_named() {
        assert_eq!(
            Filter::parse("repos:web"),
            Err(FilterError::UnknownKey("repos".into()))
        );
        assert!(matches!(
            Filter::parse("is:done"),
            Err(FilterError::UnknownValue { key: "is", .. })
        ));
        assert!(matches!(
            Filter::parse("kind:nope"),
            Err(FilterError::UnknownValue { key: "kind", .. })
        ));
        assert_eq!(
            Filter::parse("repo:"),
            Err(FilterError::MissingValue { key: "repo".into() })
        );
        // Not a key: searched as text.
        assert!(Filter::parse("https://x.test 12:30").is_ok());
    }

    #[test]
    fn the_side_can_be_switched_without_touching_the_rest() {
        let filter = Filter::parse("repo:web  is:todo -kind:draft").unwrap();
        assert_eq!(filter.query(), "repo:web is:todo -kind:draft");
        assert_eq!(filter.bucket(), Some(Bucket::Todo));

        let waiting = filter.with_bucket(Some(Bucket::Waiting));
        assert_eq!(waiting.query(), "is:waiting repo:web -kind:draft");
        assert_eq!(waiting, Filter::parse(waiting.query()).unwrap());

        let both = waiting.with_bucket(None);
        assert_eq!(both.query(), "repo:web -kind:draft");
        assert_eq!(both.bucket(), None);
        assert_eq!(both, Filter::parse(both.query()).unwrap());
    }
}
