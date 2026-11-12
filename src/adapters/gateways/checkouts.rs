//! Where the repositories are cloned on this machine: the ones the user
//! named, then the ones found under the folders where they keep their code.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use crate::entities::item::ItemRef;
use crate::use_cases::ports::Checkouts;

/// How deep a clone may be under a root: `root/group/sub/sub/sub/repo`.
const DEPTH: usize = 6;

/// Folders that hold no clone worth the walk.
const SKIPPED: [&str; 6] = [
    "node_modules",
    "target",
    "vendor",
    "Library",
    "dist",
    "build",
];

pub struct LocalCheckouts {
    /// Repository path, as the forge names it → its folder.
    pub named: BTreeMap<String, PathBuf>,
    pub roots: Vec<PathBuf>,
    /// The clones under the roots, by repository path; walked once, when
    /// first needed.
    found: OnceLock<BTreeMap<String, PathBuf>>,
}

/// The repository a remote URL points at: `group/sub/repo`, in lower case.
/// Understands `git@host:group/repo.git`, `https://host/group/repo.git` and
/// `ssh://git@host:22/group/repo`.
fn repository(url: &str) -> Option<String> {
    let url = url.trim().trim_end_matches('/');
    let url = url.strip_suffix(".git").unwrap_or(url);
    let path = match url.split_once("://") {
        Some((_, rest)) => rest.split_once('/')?.1,
        None => url.split_once(':')?.1,
    };
    let path = path.trim_matches('/').to_lowercase();
    (!path.is_empty()).then_some(path)
}

/// The repositories a clone's `.git/config` names as remotes.
fn remotes(config: &str) -> Vec<String> {
    config
        .lines()
        .filter_map(|line| line.trim().strip_prefix("url"))
        .filter_map(|rest| rest.trim_start().strip_prefix('='))
        .filter_map(repository)
        .collect()
}

fn walk(folder: &Path, depth: usize, found: &mut BTreeMap<String, PathBuf>) {
    let git = folder.join(".git");
    if git.is_dir() {
        if let Ok(config) = std::fs::read_to_string(git.join("config")) {
            for repository in remotes(&config) {
                // The first clone found is the one kept: roots are in the
                // user's order.
                found
                    .entry(repository)
                    .or_insert_with(|| folder.to_path_buf());
            }
        }
        // A clone is not searched for clones.
        return;
    }
    if depth == 0 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_dir()))
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            !name.starts_with('.') && !SKIPPED.contains(&name.as_ref())
        })
        .map(|entry| entry.path())
        .collect();
    folders.sort();
    for folder in folders {
        walk(&folder, depth - 1, found);
    }
}

impl LocalCheckouts {
    pub fn new(named: BTreeMap<String, PathBuf>, roots: Vec<PathBuf>) -> Self {
        Self {
            named: named
                .into_iter()
                .map(|(repository, folder)| (repository.to_lowercase(), folder))
                .collect(),
            roots,
            found: OnceLock::new(),
        }
    }
}

impl Checkouts for LocalCheckouts {
    fn find(&self, item: &ItemRef) -> Option<PathBuf> {
        let repository = item.container.to_lowercase();
        if let Some(folder) = self.named.get(&repository) {
            return Some(folder.clone());
        }
        self.found
            .get_or_init(|| {
                let mut found = BTreeMap::new();
                for root in &self.roots {
                    walk(root, DEPTH, &mut found);
                }
                found
            })
            .get(&repository)
            .cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entities::time::Timestamp;

    fn item(container: &str) -> ItemRef {
        ItemRef {
            source: "gitlab".into(),
            container: container.into(),
            reference: "!1".into(),
            title: "Title".into(),
            url: "https://example.test".into(),
            created_at: Timestamp(0),
            updated_at: Timestamp(0),
        }
    }

    #[test]
    fn remote_urls_name_their_repository() {
        assert_eq!(
            repository("git@gitlab.com:Acme/Platform/web.git").as_deref(),
            Some("acme/platform/web")
        );
        assert_eq!(
            repository("https://github.com/sam-doe/notes.git").as_deref(),
            Some("sam-doe/notes")
        );
        assert_eq!(
            repository("ssh://git@gitlab.acme.io:2222/acme/web/").as_deref(),
            Some("acme/web")
        );
        assert_eq!(repository("/local/path"), None);

        let config = "[remote \"origin\"]\n\turl = git@gitlab.com:acme/web.git\n\tfetch = +refs\n\
                      [remote \"fork\"]\n\turl=https://github.com/sam/web\n";
        assert_eq!(remotes(config), ["acme/web", "sam/web"]);
    }

    fn clone_at(folder: &Path, url: &str) {
        std::fs::create_dir_all(folder.join(".git")).unwrap();
        std::fs::write(
            folder.join(".git/config"),
            format!("[remote \"origin\"]\n\turl = {url}\n"),
        )
        .unwrap();
    }

    #[test]
    fn clones_are_found_under_the_roots_and_named_ones_win() {
        let root = std::env::temp_dir().join(format!("aki-checkouts-{}", std::process::id()));
        clone_at(
            &root.join("Acme/Platform/web"),
            "git@gitlab.com:acme/platform/web.git",
        );
        clone_at(
            &root.join("perso/notes"),
            "https://github.com/sam-doe/notes.git",
        );
        // Not searched: hidden, a dependency, inside another clone.
        clone_at(&root.join(".cache/api"), "git@gitlab.com:acme/api.git");
        clone_at(
            &root.join("node_modules/dep"),
            "git@gitlab.com:acme/dep.git",
        );
        clone_at(&root.join("perso/notes/sub"), "git@gitlab.com:acme/sub.git");

        let checkouts = LocalCheckouts::new(BTreeMap::new(), vec![root.clone()]);
        assert_eq!(
            checkouts.find(&item("acme/platform/web")),
            Some(root.join("Acme/Platform/web"))
        );
        assert_eq!(
            checkouts.find(&item("Sam-Doe/notes")),
            Some(root.join("perso/notes"))
        );
        for hidden in ["acme/api", "acme/dep", "acme/sub", "acme/unknown"] {
            assert_eq!(checkouts.find(&item(hidden)), None, "{hidden}");
        }

        let named =
            BTreeMap::from([("Acme/Platform/web".to_string(), PathBuf::from("/elsewhere"))]);
        let checkouts = LocalCheckouts::new(named, vec![root.clone()]);
        assert_eq!(
            checkouts.find(&item("acme/platform/web")),
            Some(PathBuf::from("/elsewhere"))
        );
        let _ = std::fs::remove_dir_all(root);
    }
}
