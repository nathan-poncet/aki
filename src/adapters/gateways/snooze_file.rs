//! What the user set aside, in a small JSON file among the state files.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::entities::snooze::{Snooze, Until};
use crate::entities::time::Timestamp;
use crate::use_cases::ports::Snoozes;

#[derive(Serialize, Deserialize)]
struct Saved {
    item: String,
    /// Seconds since the epoch; absent: until the item moves.
    until: Option<i64>,
    seen: i64,
}

pub struct SnoozeFile {
    pub path: PathBuf,
}

impl Snoozes for SnoozeFile {
    /// A missing or unreadable file is nothing set aside: the worst it
    /// costs is a task showing again.
    fn load(&self) -> Vec<Snooze> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<Saved>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|saved| Snooze {
                item: saved.item,
                until: saved
                    .until
                    .map_or(Until::Moves, |at| Until::At(Timestamp(at))),
                seen: Timestamp(saved.seen),
            })
            .collect()
    }

    fn save(&self, snoozes: &[Snooze]) -> Result<(), String> {
        let saved: Vec<Saved> = snoozes
            .iter()
            .map(|snooze| Saved {
                item: snooze.item.clone(),
                until: match snooze.until {
                    Until::At(at) => Some(at.0),
                    Until::Moves => None,
                },
                seen: snooze.seen.0,
            })
            .collect();
        let text = serde_json::to_string_pretty(&saved).map_err(|error| error.to_string())?;
        // Written aside then renamed: a crash never leaves half a file.
        let aside = self.path.with_extension("json.tmp");
        self.path
            .parent()
            .map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|()| std::fs::write(&aside, text))
            .and_then(|()| std::fs::rename(&aside, &self.path))
            .map_err(|error| format!("cannot remember it: {}: {error}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_set_aside_survives_the_file() {
        let folder = std::env::temp_dir().join(format!("aki-snooze-{}", std::process::id()));
        let store = SnoozeFile {
            path: folder.join("state").join("snoozed.json"),
        };
        assert!(store.load().is_empty());

        let snoozes = vec![
            Snooze {
                item: "gitlab:g/web!1".into(),
                until: Until::At(Timestamp(2_000)),
                seen: Timestamp(100),
            },
            Snooze {
                item: "jira:PROJPROJ-7".into(),
                until: Until::Moves,
                seen: Timestamp(200),
            },
        ];
        store.save(&snoozes).unwrap();
        assert_eq!(store.load(), snoozes);

        std::fs::write(&store.path, "not json").unwrap();
        assert!(store.load().is_empty());
        let _ = std::fs::remove_dir_all(folder);
    }
}
