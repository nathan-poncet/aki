//! An issue in a tracker (Jira, GitLab issues, GitHub issues), and how
//! changes point at one.

use super::item::ItemRef;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusCategory {
    Todo,
    InProgress,
    Done,
}

/// How much a ticket matters, least first so that `max` is the most important.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Lowest,
    Low,
    Medium,
    High,
    Highest,
}

impl Priority {
    pub const ALL: [Priority; 5] = [
        Priority::Lowest,
        Priority::Low,
        Priority::Medium,
        Priority::High,
        Priority::Highest,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Priority::Lowest => "lowest",
            Priority::Low => "low",
            Priority::Medium => "medium",
            Priority::High => "high",
            Priority::Highest => "highest",
        }
    }

    /// Reads a tracker's own word for it. Trackers disagree on the scale;
    /// the usual names of Jira's old and new schemes are understood.
    pub fn from_name(name: &str) -> Option<Self> {
        match name.trim().to_lowercase().as_str() {
            "highest" | "blocker" | "critical" | "urgent" | "p0" | "p1" => Some(Priority::Highest),
            "high" | "major" | "p2" => Some(Priority::High),
            "medium" | "normal" | "p3" => Some(Priority::Medium),
            "low" | "minor" | "p4" => Some(Priority::Low),
            "lowest" | "trivial" | "p5" => Some(Priority::Lowest),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub item: ItemRef,
    /// The tracker's own name for the status: "In Review", "Blocked"…
    pub status: String,
    pub category: StatusCategory,
    pub priority: Option<Priority>,
    /// References of unfinished tickets that block this one.
    pub blocked_by: Vec<String>,
    /// Someone mentioned me in a comment and I have not commented since.
    pub mention_pending: bool,
}

/// A ticket a change is about. The priority is known only when a tracker
/// could be asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LinkedTicket {
    pub key: String,
    pub priority: Option<Priority>,
}

/// Things that look like ticket keys and are not.
const NOT_PROJECTS: [&str; 24] = [
    "UTF", "SHA", "CVE", "ISO", "RFC", "TLS", "SSL", "AES", "RSA", "HTTP", "GPL", "MD", "PR", "MR",
    "ED", "EC", "HS", "RS", "ES", "BLS", "COVID", "UTC", "GMT", "IEEE",
];

/// The ticket keys written in a text: `PROJ-120`, `AB2-7`. A key is two to
/// ten capitals or digits starting with a capital, a dash, and a number,
/// standing on its own.
pub fn keys_in(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let word = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'_';
    let mut keys: Vec<String> = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let standalone = start == 0 || !(word(bytes[start - 1]) || bytes[start - 1] == b'-');
        if !(standalone && bytes[start].is_ascii_uppercase()) {
            start += 1;
            continue;
        }
        let mut dash = start;
        while dash < bytes.len()
            && (bytes[dash].is_ascii_uppercase() || bytes[dash].is_ascii_digit())
        {
            dash += 1;
        }
        let mut end = dash + 1;
        while end < bytes.len() && bytes[end].is_ascii_digit() {
            end += 1;
        }
        let project = &text[start..dash];
        let is_key = (2..=10).contains(&project.len())
            && bytes.get(dash) == Some(&b'-')
            && end > dash + 1
            && end - dash <= 8
            && !bytes.get(end).is_some_and(|next| word(*next))
            && !NOT_PROJECTS.contains(&project);
        if is_key {
            let key = text[start..end].to_string();
            if !keys.contains(&key) {
                keys.push(key);
            }
            start = end;
        } else {
            start = dash.max(start + 1);
        }
    }
    keys
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_found_wherever_they_stand() {
        assert_eq!(
            keys_in("Resolve PROJ-120 \"Feature/ export\""),
            ["PROJ-120"]
        );
        assert_eq!(
            keys_in("[AB2-7] fix, see PROJ-1 and PROJ-1"),
            ["AB2-7", "PROJ-1"]
        );
        assert_eq!(keys_in("feature/PROJ-42-export-account"), ["PROJ-42"]);
        assert_eq!(keys_in("✨ PROJ-719 - [web] - retry"), ["PROJ-719"]);
    }

    #[test]
    fn look_alikes_are_left_alone() {
        assert!(keys_in("Encode as UTF-8 with SHA-256, fix CVE-2024").is_empty());
        assert!(keys_in("lowercase proj-12, xPROJ-12, PROJ-12x, A-1, PROJ-").is_empty());
        assert!(keys_in("my-PROJ-12 is part of a longer name").is_empty());
        assert!(keys_in("").is_empty());
    }

    #[test]
    fn priorities_are_read_from_the_usual_names() {
        assert_eq!(Priority::from_name("Highest"), Some(Priority::Highest));
        assert_eq!(Priority::from_name("Blocker"), Some(Priority::Highest));
        assert_eq!(Priority::from_name(" major "), Some(Priority::High));
        assert_eq!(Priority::from_name("Trivial"), Some(Priority::Lowest));
        assert_eq!(Priority::from_name("Whenever"), None);
        assert!(Priority::Highest > Priority::High);
        for priority in Priority::ALL {
            assert_eq!(Priority::from_name(priority.name()), Some(priority));
        }
    }
}
