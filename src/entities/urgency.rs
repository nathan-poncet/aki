//! How much a task deserves to be looked at first. A number computed here
//! and nowhere else, from three things: which rule put it on the board, how
//! long it has been waiting, and how much the ticket behind it matters.

use super::ticket::{LinkedTicket, Priority};
use super::time::Timestamp;

/// The weights, all tunable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UrgencyRules {
    /// Points per full day of waiting.
    pub per_day: u32,
    /// The most that waiting alone can bring.
    pub waiting_cap: u32,
    /// Points for being tied to a ticket at all.
    pub linked: i32,
    pub highest: i32,
    pub high: i32,
    pub medium: i32,
    pub low: i32,
    pub lowest: i32,
    /// From this score on, a task is warm, then hot.
    pub warm: u32,
    pub hot: u32,
}

impl Default for UrgencyRules {
    fn default() -> Self {
        Self {
            per_day: 3,
            waiting_cap: 60,
            linked: 5,
            highest: 50,
            high: 30,
            medium: 10,
            low: 0,
            lowest: -10,
            warm: 100,
            hot: 130,
        }
    }
}

impl UrgencyRules {
    pub fn points(&self, priority: Priority) -> i32 {
        match priority {
            Priority::Highest => self.highest,
            Priority::High => self.high,
            Priority::Medium => self.medium,
            Priority::Low => self.low,
            Priority::Lowest => self.lowest,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Heat {
    Calm,
    Warm,
    Hot,
}

impl Heat {
    pub fn name(self) -> &'static str {
        match self {
            Heat::Calm => "calm",
            Heat::Warm => "warm",
            Heat::Hot => "hot",
        }
    }
}

/// The score, with what it is made of, so it can be explained.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Urgency {
    /// From the rule: the earlier the rule comes, the more points.
    pub rule: u32,
    /// From the days spent waiting.
    pub waiting: u32,
    /// From the ticket behind the task; can be negative.
    pub ticket: i32,
    pub heat: Heat,
}

impl Urgency {
    pub fn score(&self) -> u32 {
        (i64::from(self.rule) + i64::from(self.waiting) + i64::from(self.ticket)).max(0) as u32
    }
}

/// `rule_priority` is the headline rule's (lower comes first, 0..100).
/// `own` is the task's own priority when it is a ticket; `tickets` are the
/// tickets a change is tied to, the most important of which counts.
pub fn urgency(
    rule_priority: u32,
    since: Timestamp,
    now: Timestamp,
    own: Option<Priority>,
    tickets: &[LinkedTicket],
    rules: &UrgencyRules,
) -> Urgency {
    let rule = 100u32.saturating_sub(rule_priority);
    let waiting = since
        .days_until(now)
        .saturating_mul(rules.per_day)
        .min(rules.waiting_cap);

    let linked = tickets
        .iter()
        .filter_map(|ticket| ticket.priority)
        .max()
        .map(|priority| rules.points(priority))
        .unwrap_or(0);
    let ticket = own.map(|priority| rules.points(priority)).unwrap_or(0)
        + linked
        + if tickets.is_empty() { 0 } else { rules.linked };

    let mut urgency = Urgency {
        rule,
        waiting,
        ticket,
        heat: Heat::Calm,
    };
    urgency.heat = match urgency.score() {
        score if score >= rules.hot => Heat::Hot,
        score if score >= rules.warm => Heat::Warm,
        _ => Heat::Calm,
    };
    urgency
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn linked(key: &str, priority: Option<Priority>) -> LinkedTicket {
        LinkedTicket {
            key: key.to_string(),
            priority,
        }
    }

    fn score(rule: u32, days: i64, own: Option<Priority>, tickets: &[LinkedTicket]) -> Urgency {
        urgency(
            rule,
            Timestamp(0),
            Timestamp(days * DAY),
            own,
            tickets,
            &UrgencyRules::default(),
        )
    }

    #[test]
    fn the_rule_sets_the_base() {
        assert_eq!(score(10, 0, None, &[]).score(), 90);
        assert_eq!(score(90, 0, None, &[]).score(), 10);
        assert_eq!(score(250, 0, None, &[]).score(), 0);
    }

    #[test]
    fn waiting_adds_up_to_a_cap() {
        assert_eq!(score(10, 4, None, &[]).waiting, 12);
        assert_eq!(score(10, 400, None, &[]).waiting, 60);
        // An old draft does not outrank a fresh review request by age alone…
        assert!(score(90, 400, None, &[]).score() < score(10, 0, None, &[]).score());
        // …but an old review outranks a fresh one.
        assert!(score(10, 5, None, &[]).score() > score(10, 0, None, &[]).score());
    }

    #[test]
    fn the_most_important_linked_ticket_counts() {
        let tickets = [
            linked("A-1", Some(Priority::Low)),
            linked("A-2", Some(Priority::Highest)),
            linked("A-3", None),
        ];
        assert_eq!(score(10, 0, None, &tickets).ticket, 55);
        // Tied to a ticket nobody could read: still a little more than nothing.
        assert_eq!(score(10, 0, None, &[linked("A-3", None)]).ticket, 5);
        assert_eq!(score(10, 0, Some(Priority::High), &[]).ticket, 30);
        assert_eq!(score(95, 0, Some(Priority::Lowest), &[]).score(), 0);
    }

    #[test]
    fn heat_follows_the_score() {
        assert_eq!(score(10, 0, None, &[]).heat, Heat::Calm);
        assert_eq!(score(10, 4, None, &[]).heat, Heat::Warm);
        let urgent = [linked("A-2", Some(Priority::Highest))];
        assert_eq!(score(10, 0, None, &urgent).heat, Heat::Hot);
    }
}
