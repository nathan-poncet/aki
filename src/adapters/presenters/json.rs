//! The board as JSON, for scripts: `myturn list --json`.

use serde_json::{Value, json};

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
        "updated_at": task.since.0,
        "reasons": task.reasons.iter().map(|reason| json!({
            "kind": reason.kind.name(),
            "bucket": reason.bucket.name(),
            "priority": reason.priority,
            "why": wording::explain(reason),
            "url": reason.url,
        })).collect::<Vec<_>>(),
    })
}

pub fn board(board: &Board, only: Option<Bucket>) -> String {
    let list = |bucket: Bucket| -> Vec<Value> {
        if only.is_none_or(|only| only == bucket) {
            board.tasks(bucket).iter().map(task).collect()
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
        })).collect::<Vec<_>>(),
        "todo": list(Bucket::Todo),
        "waiting": list(Bucket::Waiting),
    });
    serde_json::to_string_pretty(&document).expect("a JSON value always serializes")
}
