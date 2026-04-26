//! Persistent read-state store.
//!
//! Tracks which stories the user has opened and how many comments each had
//! at that moment. [`crate::ui::story_list::StoryList`] uses this to dim
//! visited stories and badge stories with new comments since last visit.
//!
//! Backed by a JSON file at `$XDG_DATA_HOME/hnt/read.json` (or
//! `$HOME/.local/share/hnt/read.json` as a fallback). Failures to resolve
//! the path or read the file leave the store in-memory only — the feature
//! still works within the session but is not persisted across restarts.

use crate::api::types::StoryId;
use crate::state::persist::{xdg_data_path, JsonStore, PersistedEntry};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Soft cap on stored entries. Oldest (lowest `last_seen_at`) entries are
/// evicted on overflow so the file stays bounded.
const MAX_ENTRIES: usize = 5000;

/// On-disk schema version. Bumped only on incompatible format changes.
const SCHEMA_VERSION: u32 = 1;

/// One visited story's persisted state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadEntry {
    /// Wall-clock timestamp (Unix seconds) of the most recent visit.
    pub last_seen_at: i64,
    /// Total comment count (`descendants`) at the time of the visit. A
    /// later render can subtract this from the current count to surface a
    /// `+N` "new comments" badge.
    pub last_comment_count: i64,
}

impl PersistedEntry for ReadEntry {
    fn age_key(&self) -> i64 {
        self.last_seen_at
    }
}

/// In-memory read-state store with JSON-file persistence.
///
/// Constructed via [`ReadStore::load`] at startup. Mark a story as visited
/// with [`ReadStore::mark`] and flush with [`ReadStore::save`]. Reads
/// ([`ReadStore::is_read`], [`ReadStore::new_comments_since`]) are cheap.
///
/// Keyed by [`StoryId`] so the compiler catches attempts to mix in
/// comment IDs. The JSON on disk still uses stringified-u64 keys —
/// conversion happens at the serde boundary.
pub struct ReadStore {
    inner: JsonStore<ReadEntry>,
}

impl ReadStore {
    /// In-memory-only store with no persistence path. Used when the
    /// default XDG path can't be resolved.
    pub fn empty() -> Self {
        Self {
            inner: JsonStore::empty(MAX_ENTRIES, SCHEMA_VERSION),
        }
    }

    /// Loads from `$XDG_DATA_HOME/hnt/read.json` (or the
    /// `$HOME/.local/share/hnt/read.json` fallback). Returns an empty
    /// in-memory store if the path can't be resolved or the file is
    /// missing/corrupt.
    pub fn load() -> Self {
        match xdg_data_path("read.json") {
            Some(path) => Self::load_from(path),
            None => Self::empty(),
        }
    }

    /// Loads or creates a store at `path`. A missing or corrupt file
    /// produces an empty store with `path` still set as the save target —
    /// the next [`ReadStore::save`] will replace it.
    pub fn load_from(path: PathBuf) -> Self {
        Self {
            inner: JsonStore::load_from(path, MAX_ENTRIES, SCHEMA_VERSION),
        }
    }

    /// Writes the store to its configured path if dirty. No-op for
    /// in-memory-only stores and for clean stores. Uses an atomic
    /// `tmp → rename` to avoid leaving half-written files on crash.
    /// Failures (permissions, missing parent, disk full) are silently
    /// swallowed — read-state is non-critical.
    pub fn save(&mut self) {
        self.inner.save();
    }

    /// Records or refreshes the entry for `id` with the current wall-clock
    /// timestamp and comment count. Evicts oldest entries if the store
    /// would overflow [`MAX_ENTRIES`].
    pub fn mark(&mut self, id: StoryId, current_comment_count: i64) {
        self.mark_at(id, current_comment_count, chrono::Utc::now().timestamp());
    }

    /// Variant of [`ReadStore::mark`] that uses an explicit timestamp —
    /// used by tests to keep behavior deterministic.
    pub fn mark_at(&mut self, id: StoryId, current_comment_count: i64, now: i64) {
        self.inner.insert(
            id,
            ReadEntry {
                last_seen_at: now,
                last_comment_count: current_comment_count,
            },
        );
    }

    /// Returns whether `id` has ever been visited.
    #[must_use]
    pub fn is_read(&self, id: StoryId) -> bool {
        self.inner.entries.contains_key(&id)
    }

    /// Wall-clock timestamp (Unix seconds) of the last visit to `id`, if
    /// any. Used by the "what's new" comment filter to mark comments
    /// older than the user's previous visit as already-seen.
    #[must_use]
    pub fn last_seen_at(&self, id: StoryId) -> Option<i64> {
        self.inner.entries.get(&id).map(|e| e.last_seen_at)
    }

    /// Persisted entry for `id`, if any.
    #[cfg(test)]
    pub fn entry(&self, id: StoryId) -> Option<&ReadEntry> {
        self.inner.entries.get(&id)
    }

    /// New comments since the last visit, if any. Returns `Some(n)` when
    /// `n > 0`; `None` when the story was never visited or has no new
    /// comments. A shrinking count (rare — deletions) is clamped to `None`.
    #[must_use]
    pub fn new_comments_since(&self, id: StoryId, current_count: i64) -> Option<i64> {
        let entry = self.inner.entries.get(&id)?;
        Some(current_count - entry.last_comment_count).filter(|&d| d > 0)
    }

    /// Number of persisted entries. Exposed for tests and diagnostics.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hnt_read_store_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    fn fresh_store(name: &str) -> ReadStore {
        let p = tmp_path(name);
        let _ = std::fs::remove_file(&p);
        ReadStore::load_from(p)
    }

    fn sid(n: u64) -> StoryId {
        StoryId(n)
    }

    #[test]
    fn empty_store_has_no_entries() {
        let s = ReadStore::empty();
        assert!(!s.is_read(sid(42)));
        assert!(s.new_comments_since(sid(42), 10).is_none());
    }

    #[test]
    fn mark_then_is_read() {
        let mut s = fresh_store("mark_then_is_read");
        s.mark_at(sid(42), 10, 1_700_000_000);
        assert!(s.is_read(sid(42)));
        assert!(!s.is_read(sid(99)));
    }

    #[test]
    fn new_comments_since_returns_positive_delta() {
        let mut s = fresh_store("new_comments_delta");
        s.mark_at(sid(42), 10, 1_700_000_000);
        assert_eq!(s.new_comments_since(sid(42), 15), Some(5));
    }

    #[test]
    fn new_comments_since_returns_none_when_unchanged() {
        let mut s = fresh_store("new_comments_unchanged");
        s.mark_at(sid(42), 10, 1_700_000_000);
        assert_eq!(s.new_comments_since(sid(42), 10), None);
    }

    #[test]
    fn new_comments_since_returns_none_when_shrunk() {
        let mut s = fresh_store("new_comments_shrunk");
        s.mark_at(sid(42), 10, 1_700_000_000);
        assert_eq!(s.new_comments_since(sid(42), 5), None);
    }

    #[test]
    fn new_comments_since_none_for_unknown_id() {
        let s = ReadStore::empty();
        assert_eq!(s.new_comments_since(sid(99), 10), None);
    }

    #[test]
    fn mark_updates_existing_entry_in_place() {
        let mut s = fresh_store("mark_updates");
        s.mark_at(sid(42), 10, 1_700_000_000);
        s.mark_at(sid(42), 25, 1_700_000_100);
        let e = s.entry(sid(42)).unwrap();
        assert_eq!(e.last_seen_at, 1_700_000_100);
        assert_eq!(e.last_comment_count, 25);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let p = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&p);
        {
            let mut s = ReadStore::load_from(p.clone());
            s.mark_at(sid(1), 10, 1_700_000_000);
            s.mark_at(sid(2), 20, 1_700_000_100);
            s.save();
        }
        let s2 = ReadStore::load_from(p.clone());
        assert!(s2.is_read(sid(1)));
        assert!(s2.is_read(sid(2)));
        assert_eq!(s2.entry(sid(1)).unwrap().last_comment_count, 10);
        assert_eq!(s2.entry(sid(2)).unwrap().last_comment_count, 20);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn save_is_noop_when_not_dirty() {
        let p = tmp_path("save_noop");
        let _ = std::fs::remove_file(&p);
        let mut s = ReadStore::load_from(p.clone());
        s.save();
        assert!(!p.exists());
    }

    #[test]
    fn corrupt_file_loads_as_empty() {
        let p = tmp_path("corrupt");
        std::fs::write(&p, "{not valid json").unwrap();
        let s = ReadStore::load_from(p.clone());
        assert_eq!(s.len(), 0);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn in_memory_only_store_silently_drops_save() {
        let mut s = ReadStore::empty();
        s.mark_at(sid(1), 10, 1000);
        s.save();
        assert!(s.is_read(sid(1)));
    }

    #[test]
    fn eviction_bounds_size_at_max_and_removes_oldest() {
        let mut s = ReadStore::empty();
        for i in 0..(MAX_ENTRIES as u64 + 5) {
            s.mark_at(sid(i), 0, i as i64);
        }
        assert_eq!(s.len(), MAX_ENTRIES);
        for oldest in 0..5u64 {
            assert!(
                !s.is_read(sid(oldest)),
                "oldest id {oldest} should be evicted"
            );
        }
        assert!(s.is_read(sid(MAX_ENTRIES as u64 + 4)));
    }
}
