//! What was already read, kept in one SQLite file so the next run, or the
//! next minute, does not have to ask again.
//!
//! One table of shelves: a shelf is a kind of thing (a source's last items,
//! a merge request's discussions, a ticket looked up for its priority), a
//! key names one of them, a version says what it was read at. Nothing here
//! is precious: every failure is treated as "not cached".

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rusqlite::{Connection, OptionalExtension, params};

/// Bumped when what is stored changes shape: old files are emptied.
const SCHEMA: i64 = 1;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub version: String,
    pub value: String,
    /// Seconds since the epoch.
    pub stored_at: i64,
}

impl Entry {
    pub fn age(&self) -> Duration {
        Duration::from_secs((now() - self.stored_at).max(0) as u64)
    }
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs() as i64)
        .unwrap_or(0)
}

pub struct Cache {
    connection: Mutex<Connection>,
}

impl Cache {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(folder) = path.parent() {
            std::fs::create_dir_all(folder).map_err(|error| error.to_string())?;
        }
        let connection = Connection::open(path).map_err(|error| error.to_string())?;
        // A board in a terminal and a count in a prompt may run together.
        connection
            .busy_timeout(Duration::from_secs(3))
            .and_then(|()| connection.pragma_update(None, "journal_mode", "WAL"))
            .map_err(|error| error.to_string())?;
        Self::prepare(connection)
    }

    /// A cache that forgets when the process ends.
    pub fn in_memory() -> Self {
        Connection::open_in_memory()
            .map_err(|error| error.to_string())
            .and_then(Self::prepare)
            .expect("SQLite opens in memory")
    }

    fn prepare(connection: Connection) -> Result<Self, String> {
        let found: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if found != SCHEMA {
            connection
                .execute_batch("DROP TABLE IF EXISTS entries")
                .map_err(|error| error.to_string())?;
        }
        connection
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS entries (
                     shelf     TEXT NOT NULL,
                     key       TEXT NOT NULL,
                     version   TEXT NOT NULL,
                     value     TEXT NOT NULL,
                     stored_at INTEGER NOT NULL,
                     PRIMARY KEY (shelf, key)
                 ) WITHOUT ROWID",
            )
            .and_then(|()| connection.pragma_update(None, "user_version", SCHEMA))
            .map_err(|error| error.to_string())?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn get(&self, shelf: &str, key: &str) -> Option<Entry> {
        self.connection
            .lock()
            .unwrap()
            .query_row(
                "SELECT version, value, stored_at FROM entries WHERE shelf = ?1 AND key = ?2",
                params![shelf, key],
                |row| {
                    Ok(Entry {
                        version: row.get(0)?,
                        value: row.get(1)?,
                        stored_at: row.get(2)?,
                    })
                },
            )
            .optional()
            .ok()
            .flatten()
    }

    /// Everything on a shelf, by key.
    pub fn all(&self, shelf: &str) -> Vec<(String, Entry)> {
        let connection = self.connection.lock().unwrap();
        let Ok(mut statement) = connection
            .prepare("SELECT key, version, value, stored_at FROM entries WHERE shelf = ?1")
        else {
            return Vec::new();
        };
        let rows = statement.query_map(params![shelf], |row| {
            Ok((
                row.get(0)?,
                Entry {
                    version: row.get(1)?,
                    value: row.get(2)?,
                    stored_at: row.get(3)?,
                },
            ))
        });
        match rows {
            Ok(rows) => rows.filter_map(Result::ok).collect(),
            Err(_) => Vec::new(),
        }
    }

    pub fn put(&self, shelf: &str, key: &str, version: &str, value: &str) {
        let _ = self.connection.lock().unwrap().execute(
            "INSERT INTO entries (shelf, key, version, value, stored_at)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT (shelf, key) DO UPDATE SET
                 version = excluded.version,
                 value = excluded.value,
                 stored_at = excluded.stored_at",
            params![shelf, key, version, value, now()],
        );
    }

    /// Drops from a shelf every key that is not in `keys`.
    pub fn keep_only(&self, shelf: &str, keys: &[&str]) {
        let stale: Vec<String> = self
            .all(shelf)
            .into_iter()
            .map(|(key, _)| key)
            .filter(|key| !keys.contains(&key.as_str()))
            .collect();
        let connection = self.connection.lock().unwrap();
        for key in stale {
            let _ = connection.execute(
                "DELETE FROM entries WHERE shelf = ?1 AND key = ?2",
                params![shelf, key],
            );
        }
    }

    /// Forgets everything; returns how many entries there were.
    pub fn clear(&self) -> usize {
        self.connection
            .lock()
            .unwrap()
            .execute("DELETE FROM entries", [])
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_put_comes_back_and_is_replaced() {
        let cache = Cache::in_memory();
        assert_eq!(cache.get("items", "gitlab"), None);

        cache.put("items", "gitlab", "v1", "[1]");
        cache.put("items", "github", "v1", "[2]");
        cache.put("items", "gitlab", "v2", "[3]");

        let entry = cache.get("items", "gitlab").unwrap();
        assert_eq!(
            (entry.version.as_str(), entry.value.as_str()),
            ("v2", "[3]")
        );
        assert!(entry.age() < Duration::from_secs(5));
        assert_eq!(cache.all("items").len(), 2);
        assert!(cache.all("tickets").is_empty(), "shelves do not mix");
    }

    #[test]
    fn shelves_are_pruned_and_cleared() {
        let cache = Cache::in_memory();
        for key in ["a", "b", "c"] {
            cache.put("history", key, "", "{}");
        }
        cache.put("items", "a", "", "[]");

        cache.keep_only("history", &["b"]);
        let left: Vec<String> = cache
            .all("history")
            .into_iter()
            .map(|(key, _)| key)
            .collect();
        assert_eq!(left, ["b"]);
        assert!(cache.get("items", "a").is_some());

        assert_eq!(cache.clear(), 2);
        assert!(cache.all("items").is_empty());
    }

    #[test]
    fn a_file_survives_reopening_and_an_old_schema_is_emptied() {
        let folder = std::env::temp_dir().join(format!("aki-cache-test-{}", std::process::id()));
        let path = folder.join("nested").join("cache.db");
        {
            let cache = Cache::open(&path).unwrap();
            cache.put("items", "gitlab", "v1", "[1]");
        }
        {
            let cache = Cache::open(&path).unwrap();
            assert!(cache.get("items", "gitlab").is_some());
            cache
                .connection
                .lock()
                .unwrap()
                .pragma_update(None, "user_version", SCHEMA + 1)
                .unwrap();
        }
        let cache = Cache::open(&path).unwrap();
        assert_eq!(cache.get("items", "gitlab"), None);
        drop(cache);
        let _ = std::fs::remove_dir_all(folder);
    }
}
