//! The board as plain text, for `aki list`.

use std::fmt::Write;

use super::wording::{self, age, column, plain};
use crate::entities::board::Board;
use crate::entities::desk::{Desk, Sort};
use crate::entities::rules::Rules;
use crate::entities::task::{Bucket, Kind, Task};
use crate::entities::time::Timestamp;
use crate::entities::urgency::Heat;

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

fn section(out: &mut String, title: &str, tasks: &[&Task], now: Timestamp, style: &Style) {
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
        let heat = match task.urgency.heat {
            Heat::Hot => style.paint("31", wording::heat(Heat::Hot)),
            Heat::Warm => style.paint("33", wording::heat(Heat::Warm)),
            Heat::Calm => " ".to_string(),
        };
        let _ = writeln!(
            out,
            "{heat} {} {} {} {} {:>4}",
            style.paint(color, &column(wording::headline(task), LABEL)),
            style.paint("1", &column(&task.item.short(), REFERENCE)),
            column(&plain(&task.item.title), title_width),
            column(&who, 15),
            age(task.since, now),
        );
        let indent = " ".repeat(LABEL + 3);
        let mut why = wording::summary(task);
        if let Some(until) = task.snoozed {
            why = format!("{} · {why}", wording::set_aside(until, now));
        }
        if !task.tickets.is_empty() {
            let _ = write!(why, " · {}", wording::tickets(task));
        }
        let _ = writeln!(out, "{indent}{}", style.paint("2", &why));
    }
}

/// How the list is ordered.
pub struct Order {
    pub sort: Sort,
    pub reversed: bool,
}

impl Order {
    fn of<'a>(&self, tasks: &'a [Task]) -> Vec<&'a Task> {
        let mut tasks: Vec<&Task> = tasks.iter().collect();
        self.sort.apply(&mut tasks, self.reversed);
        tasks
    }
}

pub fn board(
    board: &Board,
    only: Option<Bucket>,
    order: &Order,
    now: Timestamp,
    style: &Style,
) -> String {
    let mut out = String::new();
    if only != Some(Bucket::Waiting) {
        section(&mut out, "TO DO", &order.of(&board.todo), now, style);
    }
    if only.is_none() {
        out.push('\n');
    }
    if only != Some(Bucket::Todo) {
        section(&mut out, "WAITING", &order.of(&board.waiting), now, style);
    }
    out
}

/// One line for a prompt or a status bar. `aside`: how many tasks the
/// user set aside, `apart`: how many drafts the rules keep apart; the
/// counts leave both out.
pub fn count(board: &Board, aside: usize, apart: usize) -> String {
    let mut line = format!(
        "{} to do · {} waiting",
        board.todo.len(),
        board.waiting.len()
    );
    if aside > 0 {
        let _ = write!(line, " · {aside} set aside");
    }
    if apart > 0 {
        let _ = write!(line, " · {apart} drafts apart");
    }
    if !board.failures.is_empty() {
        let _ = write!(line, " · {} source(s) down", board.failures.len());
    }
    line
}

pub fn failures(board: &Board, now: Timestamp) -> String {
    board
        .failures
        .iter()
        .map(|failure| {
            let shown = match failure.showing_from {
                Some(at) => format!(" (showing what was read {} ago)", age(at, now)),
                None => String::new(),
            };
            format!("aki: {}: {}{shown}\n", failure.source, failure.message)
        })
        .collect()
}

/// The spaces, for `aki spaces`: how each is made and what it holds.
/// Without a board, only how they are made.
pub fn spaces(desk: &Desk, board: Option<&Board>) -> String {
    let or_all = |query: &str| match query {
        "" => "everything".to_string(),
        query => query.to_string(),
    };
    let mut out = String::new();
    for (index, space) in desk.spaces().iter().enumerate() {
        let _ = writeln!(
            out,
            "{} {}\n    scope   {}\n    layout  {}",
            index + 1,
            space.name,
            or_all(space.scope.query()),
            space.layout.name()
        );
        for (number, pane) in space.panes.iter().enumerate() {
            let _ = writeln!(
                out,
                "    pane {}  {} · filter: {} · group: {} · sort: {}",
                number + 1,
                wording::pane_title(pane),
                or_all(pane.filter.query()),
                pane.grouping.name(),
                wording::sort(pane.sort, pane.reversed),
            );
        }
        if let Some(board) = board {
            let holdings = space.holdings(board);
            if holdings.is_empty() {
                let _ = writeln!(out, "    holds   nothing right now");
            }
            for (number, held) in holdings.iter().enumerate() {
                let _ = writeln!(
                    out,
                    "    {}   {} · {}  ({} to do, {} waiting)",
                    if number == 0 { "holds" } else { "     " },
                    held.source,
                    held.container,
                    held.todo,
                    held.waiting
                );
            }
        }
        out.push('\n');
    }
    out
}

/// Every rule as it applies, for `aki rules`.
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
    let _ = writeln!(out, "drafts.mine               {}", rules.own_drafts.name());
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
    let urgency = &rules.urgency;
    let _ = writeln!(
        out,
        "\nurgency = (100 - rule priority) + waiting + ticket\n\
         urgency.per_day           {} per day waiting, up to {}\n\
         urgency.linked            {:+} for naming a ticket\n\
         urgency ticket priority   highest {:+}, high {:+}, medium {:+}, low {:+}, lowest {:+}\n\
         urgency.warm / hot        from {} / from {}",
        urgency.per_day,
        urgency.waiting_cap,
        urgency.linked,
        urgency.highest,
        urgency.high,
        urgency.medium,
        urgency.low,
        urgency.lowest,
        urgency.warm,
        urgency.hot,
    );
    out
}
