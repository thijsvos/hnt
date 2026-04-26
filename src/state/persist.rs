//! Generic JSON-backed entry store shared by [`crate::state::read_store`]
//! and [`crate::state::pin_store`].
//!
//! Both stores persist a `HashMap<StoryId, E>` to a single JSON file with
//! atomic tmp+rename writes, corrupt-file recovery, and an LRU-style cap
//! by an entry-defined age key. Splitting them out keeps the per-store
//! modules focused on their domain APIs.

use crate::api::types::StoryId;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// One persisted entry — must be cheaply clonable, JSON-serializable, and
/// expose an age key (Unix seconds) used as the LRU eviction order.
pub(crate) trait PersistedEntry: Clone + Serialize + DeserializeOwned {
    fn age_key(&self) -> i64;
}

#[derive(Serialize, Deserialize)]
struct DiskStore<E> {
    version: u32,
    #[serde(default = "HashMap::new")]
    entries: HashMap<String, E>,
}

/// Generic on-disk store keyed by [`StoryId`] with a soft-capped entry
/// count and atomic disk writes. Domain wrappers ([`crate::state::read_store::ReadStore`],
/// [`crate::state::pin_store::PinStore`]) compose around this.
pub(crate) struct JsonStore<E: PersistedEntry> {
    pub entries: HashMap<StoryId, E>,
    pub path: Option<PathBuf>,
    pub dirty: bool,
    max_entries: usize,
    schema_version: u32,
}

impl<E: PersistedEntry> JsonStore<E> {
    /// In-memory-only store with no persistence path.
    pub fn empty(max_entries: usize, schema_version: u32) -> Self {
        Self {
            entries: HashMap::new(),
            path: None,
            dirty: false,
            max_entries,
            schema_version,
        }
    }

    /// Loads from `path`, returning an empty (but path-bound) store on
    /// missing or corrupt files. Subsequent [`Self::save`] writes the path
    /// regardless.
    pub fn load_from(path: PathBuf, max_entries: usize, schema_version: u32) -> Self {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<DiskStore<E>>(&raw).ok())
            .map(|disk| {
                disk.entries
                    .into_iter()
                    .filter_map(|(k, v)| k.parse::<u64>().ok().map(|id| (StoryId(id), v)))
                    .collect()
            })
            .unwrap_or_default();
        Self {
            entries,
            path: Some(path),
            dirty: false,
            max_entries,
            schema_version,
        }
    }

    /// Inserts or replaces `id`'s entry, then evicts oldest entries if
    /// the store has overflowed `max_entries`. Marks dirty.
    pub fn insert(&mut self, id: StoryId, entry: E) {
        self.entries.insert(id, entry);
        if self.entries.len() > self.max_entries {
            self.evict_oldest();
        }
        self.dirty = true;
    }

    /// Removes `id`. Returns whether anything was removed and marks dirty
    /// only on a real change.
    pub fn remove(&mut self, id: StoryId) -> bool {
        if self.entries.remove(&id).is_some() {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Writes the store to disk if dirty. No-op for in-memory-only or
    /// clean stores. Atomic tmp+rename; failures are silently swallowed
    /// because both consumers are non-critical.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let disk = DiskStore {
            version: self.schema_version,
            entries: self
                .entries
                .iter()
                .map(|(&id, entry)| (id.0.to_string(), entry.clone()))
                .collect(),
        };
        let Ok(json) = serde_json::to_string(&disk) else {
            return;
        };

        // Best-effort dir creation. On Unix tighten to 0700 so per-user
        // history isn't world-readable on shared hosts.
        if let Some(parent) = path.parent() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                let _ = std::fs::DirBuilder::new()
                    .recursive(true)
                    .mode(0o700)
                    .create(parent);
            }
            #[cfg(not(unix))]
            {
                let _ = std::fs::create_dir_all(parent);
            }
        }

        let tmp = path.with_extension("json.tmp");
        if write_atomic(&tmp, &json).is_ok() && std::fs::rename(&tmp, path).is_ok() {
            self.dirty = false;
        } else {
            // Clean up a stranded tmp on failure so we don't leak files.
            let _ = std::fs::remove_file(&tmp);
        }
    }

    /// Drops entries with the lowest [`PersistedEntry::age_key`] until the
    /// store is back within `max_entries`.
    fn evict_oldest(&mut self) {
        let mut ages: Vec<(i64, StoryId)> = self
            .entries
            .iter()
            .map(|(&id, e)| (e.age_key(), id))
            .collect();
        ages.sort_unstable();
        let excess = self.entries.len().saturating_sub(self.max_entries);
        for (_, id) in ages.into_iter().take(excess) {
            self.entries.remove(&id);
        }
    }
}

/// Writes `json` to `path` with mode 0600 on Unix (so other users on a
/// shared host can't read pinned/read history). Existing files are
/// truncated.
fn write_atomic(path: &std::path::Path, json: &str) -> std::io::Result<()> {
    use std::io::Write;
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(json.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, json)?;
    }
    Ok(())
}

/// Resolves an XDG data path under `hnt/`. Tries `$XDG_DATA_HOME` first
/// (rejecting non-absolute values per the XDG spec), falls back to
/// `$HOME/.local/share`, and returns `None` if neither is usable (rare —
/// containers without a `HOME`).
pub(crate) fn xdg_data_path(filename: &str) -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        let p = PathBuf::from(&xdg);
        if !xdg.is_empty() && p.is_absolute() {
            return Some(p.join("hnt").join(filename));
        }
    }
    let home = std::env::var("HOME").ok()?;
    if home.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(home)
            .join(".local")
            .join("share")
            .join("hnt")
            .join(filename),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
    struct TestEntry {
        ts: i64,
        n: u32,
    }
    impl PersistedEntry for TestEntry {
        fn age_key(&self) -> i64 {
            self.ts
        }
    }

    fn tmp(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hnt_persist_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    #[test]
    fn roundtrip_through_disk() {
        let p = tmp("roundtrip");
        let _ = std::fs::remove_file(&p);
        {
            let mut s = JsonStore::<TestEntry>::load_from(p.clone(), 100, 1);
            s.insert(StoryId(1), TestEntry { ts: 10, n: 1 });
            s.insert(StoryId(2), TestEntry { ts: 20, n: 2 });
            s.save();
        }
        let s2 = JsonStore::<TestEntry>::load_from(p.clone(), 100, 1);
        assert_eq!(s2.entries.len(), 2);
        assert_eq!(s2.entries[&StoryId(1)].n, 1);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupt_file_loads_as_empty() {
        let p = tmp("corrupt");
        std::fs::write(&p, "{not valid json").unwrap();
        let s = JsonStore::<TestEntry>::load_from(p.clone(), 100, 1);
        assert!(s.entries.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn evicts_oldest_when_over_cap() {
        let mut s = JsonStore::<TestEntry>::empty(3, 1);
        for i in 0..5 {
            s.insert(StoryId(i), TestEntry { ts: i as i64, n: 0 });
        }
        assert_eq!(s.entries.len(), 3);
        // oldest two (ts=0,1) should be gone; ts=2,3,4 retained
        assert!(!s.entries.contains_key(&StoryId(0)));
        assert!(!s.entries.contains_key(&StoryId(1)));
        assert!(s.entries.contains_key(&StoryId(4)));
    }

    #[test]
    fn save_is_noop_when_clean() {
        let p = tmp("save_noop");
        let _ = std::fs::remove_file(&p);
        let mut s = JsonStore::<TestEntry>::load_from(p.clone(), 100, 1);
        s.save();
        assert!(!p.exists());
    }

    #[test]
    fn xdg_path_rejects_non_absolute() {
        // Save and clear environment to avoid polluting other tests.
        let prev_xdg = std::env::var("XDG_DATA_HOME").ok();
        let prev_home = std::env::var("HOME").ok();
        // SAFETY: Tests run single-threaded by default (Cargo serializes
        // env tests via `--test-threads`), and we restore both vars before
        // returning.
        unsafe {
            std::env::set_var("XDG_DATA_HOME", "relative/path");
            std::env::set_var("HOME", "/home/test");
            let p = xdg_data_path("read.json").unwrap();
            assert!(p.starts_with("/home/test/.local/share/hnt"));
            // Now an absolute XDG should win.
            std::env::set_var("XDG_DATA_HOME", "/var/data");
            let p = xdg_data_path("read.json").unwrap();
            assert!(p.starts_with("/var/data/hnt"));
            // Restore.
            match prev_xdg {
                Some(v) => std::env::set_var("XDG_DATA_HOME", v),
                None => std::env::remove_var("XDG_DATA_HOME"),
            }
            match prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
        }
    }

    #[cfg(unix)]
    #[test]
    fn save_creates_file_with_user_only_perms() {
        use std::os::unix::fs::PermissionsExt;
        let p = tmp("perms");
        let _ = std::fs::remove_file(&p);
        let mut s = JsonStore::<TestEntry>::load_from(p.clone(), 100, 1);
        s.insert(StoryId(1), TestEntry { ts: 1, n: 1 });
        s.save();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "saved file mode should be 0o600, got {:o}", mode);
        let _ = std::fs::remove_file(&p);
    }
}
