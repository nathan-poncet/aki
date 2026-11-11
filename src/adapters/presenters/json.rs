//! The board as JSON, for scripts: `myturn list --json`.

use serde_json::{Value, json};

use super::text::Order;
use super::wording;
use crate::entities::board::Board;
use crate::entities::task::{Bucket, Task};

fn task(task: &Task) -> Value {
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
        },
        "priority": task.priority_level.map(|priority| priority.name()),
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

pub fn board(board: &Board, only: Option<Bucket>, order: &Order) -> String {
    let list = |bucket: Bucket| -> Vec<Value> {
        if only.is_none_or(|only| only == bucket) {
            let mut tasks: Vec<&Task> = board.tasks(bucket).iter().collect();
            order.sort.apply(&mut tasks, order.reversed);
            tasks.into_iter().map(task).collect()
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
