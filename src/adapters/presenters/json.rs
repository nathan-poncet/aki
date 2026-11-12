//! The board as JSON, for scripts: `aki list --json`.

use serde_json::{Value, json};

use super::text::Order;
use super::wording;
use crate::entities::board::Board;
use crate::entities::snooze::Until;
use crate::entities::task::{Bucket, Task};

pub fn task(task: &Task) -> Value {
    json!({
        "id": task.item.id(),
        "source": task.item.source,
        "container": task.item.container,
        "reference": task.item.reference,
        "title": task.item.title,
        "url": task.item.url,
        "target_url": task.target_url(),
        "role": task.role.name(),
        "counterpart": task.counterpart,
        "bucket": task.bucket().name(),
        "created_at": task.item.created_at.0,
        "updated_at": task.item.updated_at.0,
        "waiting_since": task.since.0,
        "urgency": {
            "score": task.urgency.score(),
            "rule": task.urgency.rule,
            "waiting": task.urgency.waiting,
            "ticket": task.urgency.ticket,
            "heat": task.urgency.heat.name(),
            "explained": wording::urgency(task),
        },
        "draft": task.draft,
        "kept_apart": task.apart,
        "priority": task.priority_level.map(|priority| priority.name()),
        "agent": task.agent.map(|state| state.name()),
        "snoozed": task.snoozed.map(|until| match until {
            Until::Moves => json!("until_it_moves"),
            Until::At(wake) => json!(wake.0),
        }),
        "tickets": task.tickets.iter().map(|ticket| json!({
            "key": ticket.key,
            "priority": ticket.priority.map(|priority| priority.name()),
        })).collect::<Vec<_>>(),
        "reasons": task.reasons.iter().map(|reason| json!({
            "kind": reason.kind.name(),
            "bucket": reason.bucket.name(),
            "priority": reason.priority,
            "why": wording::explain(reason),
            "url": reason.url,
            "since": reason.since.0,
        })).collect::<Vec<_>>(),
    })
}

/// One task with everything there is to say about it: how its score adds
/// up, and the brief of what is expected.
pub fn detail(subject: &Task, parts: &[wording::Part], brief: &str) -> String {
    let mut document = task(subject);
    document["urgency"]["computed"] = parts
        .iter()
        .map(|part| json!({ "what": part.what, "how": part.how, "points": part.points }))
        .collect();
    document["brief"] = json!(brief);
    serde_json::to_string_pretty(&document).expect("a JSON value always serializes")
}

/// `limit`: at most this many tasks per side.
pub fn board(board: &Board, only: Option<Bucket>, order: &Order, limit: Option<usize>) -> String {
    let list = |bucket: Bucket| -> Vec<Value> {
        if only.is_none_or(|only| only == bucket) {
            let mut tasks: Vec<&Task> = board.tasks(bucket).iter().collect();
            order.sort.apply(&mut tasks, order.reversed);
            tasks
                .into_iter()
                .take(limit.unwrap_or(usize::MAX))
                .map(task)
                .collect()
        } else {
            Vec::new()
        }
    };
    let document = json!({
        "refreshed_at": board.refreshed_at.0,
        "sources": board.sources,
        "failures": board.failures.iter().map(|failure| json!({
            "source": failure.source,
            "message": failure.message,
            "showing_from": failure.showing_from.map(|at| at.0),
        })).collect::<Vec<_>>(),
        "todo": list(Bucket::Todo),
        "waiting": list(Bucket::Waiting),
    });
    serde_json::to_string_pretty(&document).expect("a JSON value always serializes")
}
