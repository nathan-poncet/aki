//! A sound when something new becomes the user's: one of the system's, by
//! name, or a file of their own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::entities::task::{Kind, Task};
use crate::use_cases::ports::Notifier;

/// The word that silences a sound.
const NONE: &str = "none";

/// Which sound for which news, as the user set it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sounds {
    pub enabled: bool,
    /// For anything new to do: a system sound's name, or a file's path.
    pub new_task: String,
    /// A sound of its own for the tasks a rule lists. `none`: silence.
    pub rules: BTreeMap<Kind, String>,
}

impl Default for Sounds {
    fn default() -> Self {
        Self {
            enabled: true,
            new_task: if cfg!(target_os = "macos") {
                "Glass".to_string()
            } else {
                "message-new-instant".to_string()
            },
            rules: BTreeMap::new(),
        }
    }
}

impl Sounds {
    /// The one sound for this batch of news: that of the most pressing task
    /// that has one. Ten new tasks ring once.
    pub fn for_news(&self, news: &[&Task]) -> Option<&str> {
        if !self.enabled {
            return None;
        }
        news.iter()
            .map(|task| {
                self.rules
                    .get(&task.headline().kind)
                    .unwrap_or(&self.new_task)
                    .as_str()
            })
            .find(|sound| !sound.is_empty() && *sound != NONE)
    }
}

/// Where the system keeps its sounds, the user's own first.
fn folders(home: &Path) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        vec![
            home.join("Library/Sounds"),
            PathBuf::from("/System/Library/Sounds"),
        ]
    } else {
        vec![
            home.join(".local/share/sounds"),
            PathBuf::from("/usr/share/sounds/freedesktop/stereo"),
            PathBuf::from("/usr/share/sounds"),
        ]
    }
}

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
}

/// The file a sound names: a path as it is (`~` for home), else the first
/// file of that name, whatever its extension, in `folders`.
fn resolve(sound: &str, home: &Path, folders: &[PathBuf]) -> Option<PathBuf> {
    if let Some(rest) = sound.strip_prefix("~/") {
        return Some(home.join(rest)).filter(|path| path.is_file());
    }
    if sound.contains('/') {
        return Some(PathBuf::from(sound)).filter(|path| path.is_file());
    }
    folders.iter().find_map(|folder| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                path.file_stem()
                    .is_some_and(|stem| stem.to_string_lossy().eq_ignore_ascii_case(sound))
            })
            .collect();
        files.sort();
        files.into_iter().next()
    })
}

/// The system sounds that can be named, sorted, once each.
pub fn available() -> Vec<String> {
    let mut names: Vec<String> = folders(&home())
        .iter()
        .filter_map(|folder| std::fs::read_dir(folder).ok())
        .flat_map(|entries| entries.flatten())
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| {
            entry
                .path()
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .collect();
    names.sort();
    names.dedup();
    names
}

/// Plays a sound without waiting for it to end. `Err` says why it cannot.
pub fn play(sound: &str) -> Result<(), String> {
    let home = home();
    let Some(file) = resolve(sound, &home, &folders(&home)) else {
        return Err(format!(
            "no sound \"{sound}\": give a file's path, or one of `aki sounds`"
        ));
    };
    let players: &[&str] = if cfg!(target_os = "macos") {
        &["afplay"]
    } else {
        &["paplay", "pw-play", "aplay"]
    };
    for player in players {
        let started = Command::new(player)
            .arg(&file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if started.is_ok() {
            return Ok(());
        }
    }
    Err(format!(
        "nothing here can play a sound ({})",
        players.join(", ")
    ))
}

pub struct SoundNotifier {
    pub sounds: Sounds,
}

impl Notifier for SoundNotifier {
    fn announce(&self, news: &[&Task]) {
        if let Some(sound) = self.sounds.for_news(news) {
            // A sound that cannot be played is not worth a word in the board.
            let _ = play(sound);
        }
    }
}

/// Several ways to say the same news.
pub struct Together(pub Vec<Box<dyn Notifier>>);

impl Notifier for Together {
    fn announce(&self, news: &[&Task]) {
        for notifier in &self.0 {
            notifier.announce(news);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::task::samples;

    fn task(kind: Kind) -> Task {
        samples::task("gitlab", "g/web", "!1", kind, None)
    }

    #[test]
    fn new_tasks_ring_once_with_the_most_pressing_ones_sound() {
        let sounds = Sounds {
            new_task: "Glass".into(),
            rules: BTreeMap::from([
                (Kind::ReviewRequested, "Ping".to_string()),
                (Kind::DraftToFinish, NONE.to_string()),
            ]),
            ..Sounds::default()
        };
        let (review, reply, draft) = (
            task(Kind::ReviewRequested),
            task(Kind::ReplyNeeded),
            task(Kind::DraftToFinish),
        );

        assert_eq!(sounds.for_news(&[&review, &reply]), Some("Ping"));
        assert_eq!(sounds.for_news(&[&reply, &review]), Some("Glass"));
        // A silenced rule lets the next task ring, or nothing.
        assert_eq!(sounds.for_news(&[&draft, &reply]), Some("Glass"));
        assert_eq!(sounds.for_news(&[&draft]), None);
        assert_eq!(sounds.for_news(&[]), None);

        let off = Sounds {
            enabled: false,
            ..sounds
        };
        assert_eq!(off.for_news(&[&review]), None);
    }

    #[test]
    fn a_sound_is_a_name_among_the_systems_or_a_file() {
        let root = std::env::temp_dir().join(format!("aki-sounds-{}", std::process::id()));
        let (mine, system) = (root.join("home/Library/Sounds"), root.join("system"));
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::create_dir_all(&system).unwrap();
        for file in [
            mine.join("Chime.wav"),
            system.join("Glass.aiff"),
            system.join("Chime.aiff"),
        ] {
            std::fs::write(file, b"").unwrap();
        }
        let home = root.join("home");
        let folders = [mine.clone(), system.clone()];
        let find = |sound: &str| resolve(sound, &home, &folders);

        assert_eq!(find("Glass"), Some(system.join("Glass.aiff")));
        assert_eq!(find("glass"), Some(system.join("Glass.aiff")));
        assert_eq!(
            find("Chime"),
            Some(mine.join("Chime.wav")),
            "the user's own first"
        );
        assert_eq!(
            find("~/Library/Sounds/Chime.wav"),
            Some(mine.join("Chime.wav"))
        );
        let path = system.join("Glass.aiff");
        assert_eq!(find(&path.to_string_lossy()), Some(path));
        assert_eq!(find("Nope"), None);
        assert_eq!(find("/nowhere/at/all.wav"), None);
        let _ = std::fs::remove_dir_all(root);
    }
}
