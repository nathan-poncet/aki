//! The board as plain text, for `myturn list`.

use std::fmt::Write;

use super::wording::{self, age, column, plain};
use crate::entities::board::Board;
use crate::entities::rules::Rules;
use crate::entities::task::{Bucket, Kind, Task};
use crate::entities::time::Timestamp;

pub struct Style {
    pub color: bool,
    pub width: usize,
}

impl Style {
    fn paint(&self, code: &str, text: &str) -> String {
        if self.color {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }
}

const LABEL: usize = 16;
const REFERENCE: usize = 18;

fn section(out: &mut String, title: &str, tasks: &[Task], now: Timestamp, style: &Style) {
    let heading = format!("{title} ({})", tasks.len());
    let _ = writeln!(out, "{}", style.paint("1", &heading));
    if tasks.is_empty() {
        let _ = writeln!(out, "  {}", style.paint("2", "nothing"));
    }
    let title_width = style.width.saturating_sub(LABEL + REFERENCE + 26).max(20);
    for task in tasks {
        let color = match task.bucket() {
            Bucket::Todo => "33",
            Bucket::Waiting => "36",
        };
        let who = task
            .counterpart
            .as_deref()
            .map(|name| format!("@{name}"))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "  {} {} {} {} {:>4}",
            style.paint(color, &column(wording::headline(task), LABEL)),
            style.paint("1", &column(&task.item.short(), REFERENCE)),
            column(&plain(&task.item.title), title_width),
            column(&who, 15),
            age(task.since, now),
        );
        let indent = " ".repeat(LABEL + 3);
        let _ = writeln!(out, "{indent}{}", style.paint("2", &wording::summary(task)));
    }
}

pub fn board(board: &Board, only: Option<Bucket>, now: Timestamp, style: &Style) -> String {
    let mut out = String::new();
    if only != Some(Bucket::Waiting) {
        section(&mut out, "TO DO", &board.todo, now, style);
    }
    if only.is_none() {
        out.push('\n');
    }
    if only != Some(Bucket::Todo) {
        section(&mut out, "WAITING", &board.waiting, now, style);
    }
    out
}

/// One line for a prompt or a status bar.
pub fn count(board: &Board) -> String {
    let mut line = format!(
        "{} to do · {} waiting",
        board.todo.len(),
        board.waiting.len()
    );
    if !board.failures.is_empty() {
        let _ = write!(line, " · {} source(s) down", board.failures.len());
    }
    line
}

pub fn failures(board: &Board) -> String {
    board
        .failures
        .iter()
        .map(|failure| format!("myturn: {}: {}\n", failure.source, failure.message))
        .collect()
}

/// Every rule as it applies, for `myturn rules`.
pub fn rules(rules: &Rules) -> String {
    let mut out = String::new();
    for bucket in [Bucket::Todo, Bucket::Waiting] {
        let mut kinds: Vec<Kind> = Kind::ALL
            .into_iter()
            .filter(|kind| rules.rule(*kind).bucket == bucket)
            .collect();
        kinds.sort_by_key(|kind| rules.rule(*kind).priority);
        let _ = writeln!(out, "{}", bucket.name().to_uppercase());
        for kind in kinds {
            let rule = rules.rule(kind);
            let _ = writeln!(
                out,
                "  {:>3}  {:<19} {:<4} {}",
                rule.priority,
                kind.name(),
                if rule.enabled { "on" } else { "off" },
                wording::describe(kind),
            );
        }
        out.push('\n');
    }
    let triggers: Vec<&str> = rules.re_review_triggers.iter().map(|t| t.name()).collect();
    let _ = writeln!(out, "re_review.triggers        {}", triggers.join(", "));
    let _ = writeln!(
        out,
        "re_review.after_approval  {}",
        rules.re_review_after_approval
    );
    let _ = writeln!(
        out,
        "threads.who_resolves      {}",
        rules.who_resolves.name()
    );
    let _ = writeln!(out, "threads.ignore_bots       {}", rules.ignore_bots);
    let _ = writeln!(
        out,
        "drafts.others             {}",
        rules.others_drafts.name()
    );
    let _ = writeln!(
        out,
        "tickets.waiting_statuses  {}",
        rules.ticket_waiting_statuses.join(", ")
    );
    let _ = writeln!(out, "nudge.after_days          {}", rules.nudge_after_days);
    out
}
