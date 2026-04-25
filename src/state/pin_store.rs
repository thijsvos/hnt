//! Persistent pinned-story store.
//!
//! Tracks stories the user has explicitly pinned via `b`, plus the
//! reading-position snapshot (selected comment index, collapsed-subtree
//! IDs) at the time they last left the story. The Pinned feed
//! ([`crate::api::types::FeedKind::Pinned`]) reads from this store, and
//! [`crate::ui::story_list::StoryList`] renders a `★` glyph for any story
//! present here.
//!
//! Backed by a JSON file at `$XDG_DATA_HOME/hnt/pinned.json` (or
//! `$HOME/.local/share/hnt/pinned.json` as a fallback). Failures to
//! resolve the path or read the file leave the store in-memory only —
//! the feature still works within the session but is not persisted across
//! restarts. Mirrors [`crate::state::read_store::ReadStore`]'s atomic
//! tmp+rename write, version field, and corrupt-file recovery.

use crate::api::types::StoryId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Soft cap on stored pins. Oldest (lowest `pinned_at`) entries are
/// evicted on overflow so the file stays bounded. Set lower than
/// [`crate::state::read_store`]'s cap because pinning is a deliberate
/// user action, not a side effect of every visit.
const MAX_ENTRIES: usize = 1000;

/// On-disk schema version. Bumped only on incompatible format changes.
const SCHEMA_VERSION: u32 = 1;

/// One pinned story's persisted state, including the reading-position
/// snapshot used to resume the user mid-thread when they reopen it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct PinEntry {
    /// Wall-clock timestamp (Unix seconds) at which the story was first
    /// pinned. Used for newest-first ordering in the Pinned feed and as
    /// the LRU eviction key. Re-pinning an existing entry leaves this
    /// untouched — the position in the list is stable.
    pub pinned_at: i64,
    /// Selected comment index into `visible_comments()` at the moment of
    /// the last snapshot. Clamped to the visible length on restore so a
    /// shrinking thread (rare — moderation deletions) can't strand the
    /// cursor past the end.
    #[serde(default)]
    pub selected: usize,
    /// Comment IDs the user collapsed before leaving. Restored as the
    /// initial collapse set when reopening. IDs that no longer exist in
    /// the loaded tree are harmlessly retained — the visibility check
    /// just never matches them.
    #[serde(default)]
    pub collapsed: Vec<u64>,
}

/// In-memory pinned-store with JSON-file persistence.
///
/// Constructed via [`PinStore::load`] at startup. Add a pin with
/// [`PinStore::pin`], remove with [`PinStore::unpin`], snapshot reading
/// position with [`PinStore::update_resume`], flush with
/// [`PinStore::save`]. Reads ([`PinStore::is_pinned`],
/// [`PinStore::resume_for`], [`PinStore::pinned_ids_newest_first`]) are
/// cheap.
pub struct PinStore {
    entries: HashMap<StoryId, PinEntry>,
    path: Option<PathBuf>,
    dirty: bool,
}

#[derive(Serialize, Deserialize)]
struct DiskStore {
    version: u32,
    #[serde(default)]
    entries: HashMap<String, PinEntry>,
}

impl PinStore {
    /// In-memory-only store with no persistence path. Used when the
    /// default XDG path can't be resolved.
    pub fn empty() -> Self {
        Self {
            entries: HashMap::new(),
            path: None,
            dirty: false,
        }
    }

    /// Loads from `$XDG_DATA_HOME/hnt/pinned.json` (or the
    /// `$HOME/.local/share/hnt/pinned.json` fallback). Returns an empty
    /// in-memory store if the path can't be resolved or the file is
    /// missing/corrupt.
    pub fn load() -> Self {
        match default_path() {
            Some(path) => Self::load_from(path),
            None => Self::empty(),
        }
    }

    /// Loads or creates a store at `path`. A missing or corrupt file
    /// produces an empty store with `path` still set as the save target —
    /// the next [`PinStore::save`] will replace it.
    pub fn load_from(path: PathBuf) -> Self {
        let entries = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str::<DiskStore>(&raw).ok())
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
        }
    }

    /// Writes the store to its configured path if dirty. No-op for
    /// in-memory-only stores and for clean stores. Uses an atomic
    /// `tmp → rename` to avoid leaving half-written files on crash.
    /// Failures (permissions, missing parent, disk full) are silently
    /// swallowed — pin-state is non-critical.
    pub fn save(&mut self) {
        if !self.dirty {
            return;
        }
        let Some(path) = self.path.as_ref() else {
            return;
        };
        let disk = DiskStore {
            version: SCHEMA_VERSION,
            entries: self
                .entries
                .iter()
                .map(|(&id, entry)| (id.0.to_string(), entry.clone()))
                .collect(),
        };
        let Ok(json) = serde_json::to_string(&disk) else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let tmp = path.with_extension("json.tmp");
        if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, path).is_ok() {
            self.dirty = false;
        }
    }

    /// Pins `id` with the current wall-clock timestamp and a default
    /// (top-of-thread) resume state. No-op if already pinned — re-pinning
    /// must not bump `pinned_at`, otherwise the Pinned-feed ordering
    /// would shuffle when the user opens an existing pin.
    pub fn pin(&mut self, id: StoryId) {
        self.pin_at(id, chrono::Utc::now().timestamp());
    }

    /// Variant of [`PinStore::pin`] that uses an explicit timestamp —
    /// used by tests to keep behavior deterministic.
    pub fn pin_at(&mut self, id: StoryId, now: i64) {
        if self.entries.contains_key(&id) {
            return;
        }
        self.entries.insert(
            id,
            PinEntry {
                pinned_at: now,
                selected: 0,
                collapsed: Vec::new(),
            },
        );
        if self.entries.len() > MAX_ENTRIES {
            self.evict_oldest();
        }
        self.dirty = true;
    }

    /// Unpins `id`. No-op if not pinned. Returns whether anything was
    /// removed (useful for callers that want to flag the store dirty
    /// only on a real change).
    pub fn unpin(&mut self, id: StoryId) -> bool {
        if self.entries.remove(&id).is_some() {
            self.dirty = true;
            true
        } else {
            false
        }
    }

    /// Whether `id` is currently pinned.
    #[must_use]
    pub fn is_pinned(&self, id: StoryId) -> bool {
        self.entries.contains_key(&id)
    }

    /// Snapshot the user's reading position for `id`. No-op if `id` isn't
    /// pinned — only deliberately-curated stories get their position
    /// remembered.
    pub fn update_resume(&mut self, id: StoryId, selected: usize, collapsed: Vec<u64>) {
        let Some(entry) = self.entries.get_mut(&id) else {
            return;
        };
        // Skip a write (and `dirty`) when nothing actually changed —
        // typical idle-then-quit shouldn't re-flush the file.
        if entry.selected == selected && entry.collapsed == collapsed {
            return;
        }
        entry.selected = selected;
        entry.collapsed = collapsed;
        self.dirty = true;
    }

    /// Persisted entry for `id`, if any. Used by the comment-load path to
    /// restore reading position once the thread has finished loading.
    #[must_use]
    pub fn resume_for(&self, id: StoryId) -> Option<&PinEntry> {
        self.entries.get(&id)
    }

    /// Pinned story IDs in newest-first order — the listing the Pinned
    /// virtual feed renders. Returns owned `u64`s (matching
    /// [`crate::state::story_state::StoryListState::all_ids`]) so the
    /// caller can stash the slice for stable pagination without holding a
    /// borrow into the store.
    #[must_use]
    pub fn pinned_ids_newest_first(&self) -> Vec<u64> {
        let mut entries: Vec<(i64, StoryId)> = self
            .entries
            .iter()
            .map(|(&id, e)| (e.pinned_at, id))
            .collect();
        // Newest first → descending by timestamp, with ID as tiebreaker
        // for determinism when two pins land in the same second.
        entries.sort_unstable_by(|a, b| b.cmp(a));
        entries.into_iter().map(|(_, id)| id.0).collect()
    }

    /// Drops entries with the lowest `pinned_at` until the store is
    /// back within [`MAX_ENTRIES`].
    fn evict_oldest(&mut self) {
        let mut ages: Vec<(i64, StoryId)> = self
            .entries
            .iter()
            .map(|(&id, e)| (e.pinned_at, id))
            .collect();
        ages.sort_unstable();
        let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
        for (_, id) in ages.into_iter().take(excess) {
            self.entries.remove(&id);
        }
    }

    /// Number of pinned entries. Exposed for tests and diagnostics.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

/// Resolves the default persistence path: `$XDG_DATA_HOME/hnt/pinned.json`,
/// falling back to `$HOME/.local/share/hnt/pinned.json`. Returns `None` if
/// neither variable is set (rare — containers with no `HOME`).
fn default_path() -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("hnt").join("pinned.json"));
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
            .join("pinned.json"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hnt_pin_store_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    fn fresh_store(name: &str) -> PinStore {
        let p = tmp_path(name);
        let _ = std::fs::remove_file(&p);
        PinStore::load_from(p)
    }

    fn sid(n: u64) -> StoryId {
        StoryId(n)
    }

    #[test]
    fn empty_store_has_no_pins() {
        let s = PinStore::empty();
        assert!(!s.is_pinned(sid(42)));
        assert!(s.resume_for(sid(42)).is_none());
        assert!(s.pinned_ids_newest_first().is_empty());
    }

    #[test]
    fn pin_then_is_pinned() {
        let mut s = fresh_store("pin_then_is_pinned");
        s.pin_at(sid(42), 1_700_000_000);
        assert!(s.is_pinned(sid(42)));
        assert!(!s.is_pinned(sid(99)));
    }

    #[test]
    fn unpin_removes_entry() {
        let mut s = fresh_store("unpin_removes");
        s.pin_at(sid(42), 1_700_000_000);
        assert!(s.unpin(sid(42)));
        assert!(!s.is_pinned(sid(42)));
    }

    #[test]
    fn unpin_unknown_returns_false() {
        let mut s = fresh_store("unpin_unknown");
        assert!(!s.unpin(sid(42)));
    }

    #[test]
    fn re_pinning_is_idempotent_and_does_not_bump_timestamp() {
        let mut s = fresh_store("idempotent_pin");
        s.pin_at(sid(42), 1_700_000_000);
        s.pin_at(sid(42), 1_700_000_500);
        assert_eq!(s.resume_for(sid(42)).unwrap().pinned_at, 1_700_000_000);
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn pinned_ids_newest_first_orders_by_timestamp_descending() {
        let mut s = fresh_store("newest_first");
        s.pin_at(sid(1), 1_700_000_100);
        s.pin_at(sid(2), 1_700_000_300);
        s.pin_at(sid(3), 1_700_000_200);
        assert_eq!(s.pinned_ids_newest_first(), vec![2, 3, 1]);
    }

    #[test]
    fn pinned_ids_newest_first_tiebreak_is_deterministic() {
        let mut s = fresh_store("tiebreak");
        // Two pins land in the same second — order must still be stable
        // across runs (HashMap iteration is not).
        s.pin_at(sid(5), 1_700_000_000);
        s.pin_at(sid(7), 1_700_000_000);
        s.pin_at(sid(3), 1_700_000_000);
        let ids = s.pinned_ids_newest_first();
        assert_eq!(ids, vec![7, 5, 3]);
    }

    #[test]
    fn update_resume_only_applies_to_pinned() {
        let mut s = fresh_store("update_unpinned");
        s.update_resume(sid(42), 5, vec![1, 2, 3]);
        assert!(s.resume_for(sid(42)).is_none());
    }

    #[test]
    fn update_resume_stores_position() {
        let mut s = fresh_store("update_resume");
        s.pin_at(sid(42), 1_700_000_000);
        s.update_resume(sid(42), 17, vec![10, 20, 30]);
        let entry = s.resume_for(sid(42)).unwrap();
        assert_eq!(entry.selected, 17);
        assert_eq!(entry.collapsed, vec![10, 20, 30]);
    }

    #[test]
    fn update_resume_no_change_does_not_dirty() {
        let mut s = fresh_store("no_change");
        s.pin_at(sid(42), 1_700_000_000);
        s.dirty = false; // simulate "just saved"
        s.update_resume(sid(42), 0, Vec::new());
        assert!(
            !s.dirty,
            "equal-value resume update must not dirty the store"
        );
    }

    #[test]
    fn save_and_reload_roundtrip() {
        let p = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&p);
        {
            let mut s = PinStore::load_from(p.clone());
            s.pin_at(sid(1), 1_700_000_000);
            s.pin_at(sid(2), 1_700_000_100);
            s.update_resume(sid(2), 7, vec![100, 200]);
            s.save();
        }
        let s2 = PinStore::load_from(p.clone());
        assert!(s2.is_pinned(sid(1)));
        assert!(s2.is_pinned(sid(2)));
        let e2 = s2.resume_for(sid(2)).unwrap();
        assert_eq!(e2.selected, 7);
        assert_eq!(e2.collapsed, vec![100, 200]);
        assert_eq!(e2.pinned_at, 1_700_000_100);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn save_is_noop_when_not_dirty() {
        let p = tmp_path("save_noop");
        let _ = std::fs::remove_file(&p);
        let mut s = PinStore::load_from(p.clone());
        s.save();
        assert!(!p.exists());
    }

    #[test]
    fn corrupt_file_loads_as_empty() {
        let p = tmp_path("corrupt");
        std::fs::write(&p, "{not valid json").unwrap();
        let s = PinStore::load_from(p.clone());
        assert_eq!(s.len(), 0);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn version_mismatch_is_tolerated_via_default_path_field() {
        // We don't currently reject on version mismatch — instead the
        // shape of `entries` is what matters. A future v2 could add an
        // explicit check here. Ensure today's behavior for v=99 is "load
        // anyway" so a forward-compat downgrade doesn't lose data.
        let p = tmp_path("version_99");
        std::fs::write(
            &p,
            r#"{"version": 99, "entries": {"42": {"pinned_at": 1, "selected": 0, "collapsed": []}}}"#,
        )
        .unwrap();
        let s = PinStore::load_from(p.clone());
        assert!(s.is_pinned(sid(42)));
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn in_memory_only_store_silently_drops_save() {
        let mut s = PinStore::empty();
        s.pin_at(sid(1), 1000);
        s.save();
        assert!(s.is_pinned(sid(1)));
    }

    #[test]
    fn eviction_bounds_size_at_max_and_removes_oldest() {
        let mut s = PinStore::empty();
        for i in 0..(MAX_ENTRIES as u64 + 5) {
            s.pin_at(sid(i), i as i64);
        }
        assert_eq!(s.len(), MAX_ENTRIES);
        for oldest in 0..5u64 {
            assert!(
                !s.is_pinned(sid(oldest)),
                "oldest id {oldest} should be evicted"
            );
        }
        assert!(s.is_pinned(sid(MAX_ENTRIES as u64 + 4)));
    }
}
