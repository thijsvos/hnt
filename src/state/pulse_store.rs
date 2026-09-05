//! Persisted momentum samples backing the `Rising` feed.
//!
//! [`PulseStore`] owns one [`Track`] per recently-seen story and is fed
//! from two sources: the periodic Algolia sweep spawned by `App`
//! ([`PulseStore::merge_sweep`]) and every Firebase feed page that lands
//! ([`PulseStore::record_items`] — zero extra network). Ranking and
//! per-story momentum queries delegate to the pure math in
//! [`crate::pulse`].
//!
//! Backed by a JSON file at `$XDG_DATA_HOME/hnt/pulse.json` (or
//! `$HOME/.local/share/hnt/pulse.json`). Persisting the time series means
//! a restart — or the headless `hnt rising` — has momentum immediately
//! instead of waiting for two sweeps. Like the read and pin stores, a
//! missing path leaves the store in-memory only.

use crate::api::types::{Item, ItemType, StoryId};
use crate::pulse::{
    self, Momentum, Sample, SweepSample, Track, FRONT_PAGE_SIZE, SWEEP_INTERVAL, TRACK_TTL_SECS,
};
use crate::state::persist::{xdg_data_path, JsonStore, PersistedEntry};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

/// Soft cap on tracked stories. A twelve-hour sweep window holds ~600 on
/// a busy day; the cap bounds `pulse.json` if HN has a wild one.
const MAX_ENTRIES: usize = 1500;

/// On-disk schema version. Bumped only on incompatible format changes.
const SCHEMA_VERSION: u32 = 1;

/// Sweeps between mid-session flushes to disk. Unlike the read/pin
/// stores, Pulse is a time series worth saving before shutdown — a crash
/// or `SIGKILL` would otherwise discard everything sampled this session.
const FLUSH_EVERY_SWEEPS: u32 = 10;

impl PersistedEntry for Track {
    fn age_key(&self) -> i64 {
        self.latest()
            .map(|s| s.at)
            .or(self.created_at)
            .unwrap_or_default()
    }
}

/// In-memory momentum store with JSON-file persistence.
///
/// Constructed via [`PulseStore::load`] at startup. Feed it with
/// [`PulseStore::merge_sweep`] / [`PulseStore::record_items`], rank with
/// [`PulseStore::ranked`], and flush with [`PulseStore::save`].
pub struct PulseStore {
    inner: JsonStore<Track>,
    /// Current front-page IDs (first [`FRONT_PAGE_SIZE`] of `topstories`)
    /// from the latest sweep. Transient — not persisted.
    front_page: Vec<u64>,
    /// Cached [`pulse::front_page_threshold`] for the latest sweep.
    threshold: Option<f64>,
    /// Sweeps merged this session.
    sweeps: u32,
    /// Monotonic time of the latest merged sweep, for the countdown
    /// shown in the `Rising` pane title.
    last_sweep: Option<Instant>,
}

impl PulseStore {
    /// In-memory-only store with no persistence path.
    pub fn empty() -> Self {
        Self::from_inner(JsonStore::empty(MAX_ENTRIES, SCHEMA_VERSION))
    }

    /// Loads from `$XDG_DATA_HOME/hnt/pulse.json` (or the
    /// `$HOME/.local/share/hnt/pulse.json` fallback). Returns an empty
    /// in-memory store if the path can't be resolved or the file is
    /// missing/corrupt. Tracks past [`TRACK_TTL_SECS`] are pruned on load.
    pub fn load() -> Self {
        match xdg_data_path("pulse.json") {
            Some(path) => Self::load_from(path),
            None => Self::empty(),
        }
    }

    /// Loads or creates a store at `path`. A missing or corrupt file
    /// produces an empty store with `path` still set as the save target.
    pub fn load_from(path: PathBuf) -> Self {
        let mut store = Self::from_inner(JsonStore::load_from(path, MAX_ENTRIES, SCHEMA_VERSION));
        store.prune(chrono::Utc::now().timestamp());
        store
    }

    fn from_inner(inner: JsonStore<Track>) -> Self {
        Self {
            inner,
            front_page: Vec::new(),
            threshold: None,
            sweeps: 0,
            last_sweep: None,
        }
    }

    /// Writes the store to its configured path if dirty. Atomic
    /// `tmp → rename`; failures are silently swallowed — momentum data is
    /// non-critical.
    pub fn save(&mut self) {
        self.inner.save();
    }

    /// Records one observation for `id`, creating the track if needed.
    /// `created_at` fills a missing submission time on an existing track
    /// but never overwrites a known one.
    pub fn record(&mut self, id: StoryId, created_at: Option<i64>, sample: Sample) {
        match self.inner.entries.get_mut(&id) {
            Some(track) => {
                if track.created_at.is_none() {
                    track.created_at = created_at;
                }
                track.push(sample);
                self.inner.dirty = true;
            }
            None => {
                let mut track = Track::new(created_at);
                track.push(sample);
                self.inner.insert(id, track);
            }
        }
    }

    /// Piggyback sampling: records every live story in a freshly loaded
    /// feed page at wall-clock `now`. Jobs (no score) and non-story items
    /// are skipped.
    pub fn record_items(&mut self, items: &[Arc<Item>], now: i64) {
        for item in items {
            if item.is_dead_or_deleted() {
                continue;
            }
            if !matches!(item.item_type, Some(ItemType::Story) | None) {
                continue;
            }
            let Some(points) = item.score else {
                continue;
            };
            self.record(
                StoryId(item.id),
                item.time,
                Sample {
                    at: now,
                    points,
                    comments: item.descendants.unwrap_or(0),
                },
            );
        }
    }

    /// Merges one background sweep: records every sample, installs the
    /// new front page, recomputes the ETA threshold, prunes stale tracks,
    /// and bumps the sweep counter. Returns `true` when the caller should
    /// flush to disk (every [`FLUSH_EVERY_SWEEPS`] sweeps).
    pub fn merge_sweep(&mut self, samples: &[SweepSample], front_page: Vec<u64>, now: i64) -> bool {
        for s in samples {
            self.record(
                StoryId(s.id),
                s.created_at,
                Sample {
                    at: now,
                    points: s.points,
                    comments: s.comments,
                },
            );
        }
        self.front_page = front_page;
        self.front_page.truncate(FRONT_PAGE_SIZE);
        self.prune(now);
        self.recompute_threshold(now);
        self.sweeps = self.sweeps.wrapping_add(1);
        self.last_sweep = Some(Instant::now());
        self.sweeps.is_multiple_of(FLUSH_EVERY_SWEEPS)
    }

    /// Drops tracks whose latest sample is older than [`TRACK_TTL_SECS`]
    /// (or that have no samples at all).
    pub fn prune(&mut self, now: i64) {
        let before = self.inner.entries.len();
        self.inner
            .entries
            .retain(|_, t| t.latest().is_some_and(|s| now - s.at <= TRACK_TTL_SECS));
        if self.inner.entries.len() != before {
            self.inner.dirty = true;
        }
    }

    fn recompute_threshold(&mut self, now: i64) {
        let entries = &self.inner.entries;
        self.threshold =
            pulse::front_page_threshold(&self.front_page, |id| entries.get(&StoryId(id)), now);
    }

    /// The fastest-rising stories as of `now`, at most `limit`, fastest
    /// first. Empty while warming up.
    #[must_use]
    pub fn ranked(&self, now: i64, limit: usize) -> Vec<Momentum> {
        pulse::rank(
            self.inner.entries.iter().map(|(id, t)| (id.0, t)),
            &self.front_page,
            self.threshold,
            now,
            limit,
        )
    }

    /// Momentum summary for one story, or `None` when it has no live
    /// velocity. Cheap enough to call per visible row per frame — a
    /// scan of at most [`pulse::MAX_SAMPLES`] samples.
    #[must_use]
    pub fn momentum_for(&self, id: StoryId, now: i64) -> Option<Momentum> {
        let track = self.inner.entries.get(&id)?;
        pulse::momentum(id.0, track, &self.front_page, self.threshold, now)
    }

    /// Whether the store has merged at least one sweep this session.
    #[cfg(test)]
    pub fn has_swept(&self) -> bool {
        self.sweeps > 0
    }

    /// Sweeps merged this session.
    #[cfg(test)]
    pub fn sweeps(&self) -> u32 {
        self.sweeps
    }

    /// Whole seconds until the next sweep is due, based on the latest
    /// merged sweep and [`SWEEP_INTERVAL`]. `None` before the first sweep.
    #[must_use]
    pub fn seconds_until_next_sweep(&self) -> Option<u64> {
        let last = self.last_sweep?;
        let elapsed = last.elapsed();
        Some(SWEEP_INTERVAL.saturating_sub(elapsed).as_secs())
    }

    /// Number of tracked stories.
    #[must_use]
    pub fn len(&self) -> usize {
        self.inner.entries.len()
    }

    /// Whether no stories are tracked.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.inner.entries.is_empty()
    }

    /// Tracked stories' IDs in the current front page order. Exposed for
    /// tests.
    #[cfg(test)]
    pub fn front_page(&self) -> &[u64] {
        &self.front_page
    }

    /// The track for `id`, if any. Exposed for tests.
    #[cfg(test)]
    pub fn track(&self, id: StoryId) -> Option<&Track> {
        self.inner.entries.get(&id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pulse::{MIN_SAMPLE_SPACING_SECS, MIN_SPAN_SECS};

    const T0: i64 = 1_700_000_000;

    fn tmp_path(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "hnt_pulse_store_test_{}_{}.json",
            name,
            std::process::id()
        ))
    }

    fn sid(n: u64) -> StoryId {
        StoryId(n)
    }

    fn sweep(id: u64, created_at: i64, points: i64, comments: i64) -> SweepSample {
        SweepSample {
            id,
            created_at: Some(created_at),
            points,
            comments,
        }
    }

    fn story(id: u64, score: Option<i64>, item_type: Option<ItemType>) -> Arc<Item> {
        Arc::new(Item {
            id,
            title: Some(format!("story {id}")),
            url: None,
            text: None,
            by: None,
            score,
            time: Some(T0 - 600),
            kids: None,
            descendants: Some(3),
            item_type,
            dead: None,
            deleted: None,
        })
    }

    #[test]
    fn empty_store_has_nothing_ranked() {
        let s = PulseStore::empty();
        assert!(s.is_empty());
        assert!(s.ranked(T0, 10).is_empty());
        assert!(!s.has_swept());
        assert!(s.seconds_until_next_sweep().is_none());
    }

    #[test]
    fn record_creates_then_appends() {
        let mut s = PulseStore::empty();
        s.record(
            sid(1),
            Some(T0 - 100),
            Sample {
                at: T0,
                points: 1,
                comments: 0,
            },
        );
        s.record(
            sid(1),
            None,
            Sample {
                at: T0 + MIN_SAMPLE_SPACING_SECS + 1,
                points: 4,
                comments: 1,
            },
        );
        let t = s.track(sid(1)).unwrap();
        assert_eq!(t.samples.len(), 2);
        assert_eq!(t.created_at, Some(T0 - 100));
        assert_eq!(s.len(), 1);
    }

    #[test]
    fn record_backfills_missing_created_at_only() {
        let mut s = PulseStore::empty();
        let sample = Sample {
            at: T0,
            points: 1,
            comments: 0,
        };
        s.record(sid(1), None, sample);
        assert_eq!(s.track(sid(1)).unwrap().created_at, None);
        s.record(sid(1), Some(T0 - 5), sample);
        assert_eq!(s.track(sid(1)).unwrap().created_at, Some(T0 - 5));
        s.record(sid(1), Some(T0 - 999), sample);
        assert_eq!(s.track(sid(1)).unwrap().created_at, Some(T0 - 5));
    }

    #[test]
    fn record_items_skips_jobs_dead_and_scoreless() {
        let mut s = PulseStore::empty();
        let mut dead = (*story(4, Some(5), Some(ItemType::Story))).clone();
        dead.dead = Some(true);
        let items = vec![
            story(1, Some(10), Some(ItemType::Story)),
            story(2, None, Some(ItemType::Job)),
            story(3, Some(7), Some(ItemType::Comment)),
            Arc::new(dead),
            story(5, Some(2), None),
        ];
        s.record_items(&items, T0);
        assert!(s.track(sid(1)).is_some());
        assert!(s.track(sid(2)).is_none());
        assert!(s.track(sid(3)).is_none());
        assert!(s.track(sid(4)).is_none());
        assert!(s.track(sid(5)).is_some());
        assert_eq!(s.track(sid(1)).unwrap().created_at, Some(T0 - 600));
    }

    #[test]
    fn two_sweeps_produce_a_ranking_in_velocity_order() {
        let mut s = PulseStore::empty();
        let first = [
            sweep(1, T0 - 600, 5, 0),
            sweep(2, T0 - 600, 5, 0),
            sweep(3, T0 - 600, 5, 0),
        ];
        let flush = s.merge_sweep(&first, vec![], T0);
        assert!(!flush);
        assert!(s.has_swept());
        assert!(s.ranked(T0, 10).is_empty(), "single sample can't rank");

        let later = T0 + MIN_SPAN_SECS + 10;
        let second = [
            sweep(1, T0 - 600, 6, 0),
            sweep(2, T0 - 600, 30, 4),
            sweep(3, T0 - 600, 5, 0),
        ];
        s.merge_sweep(&second, vec![], later);
        let ranked = s.ranked(later, 10);
        let ids: Vec<u64> = ranked.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![2, 1], "flat story 3 must be excluded");
        assert!(ranked[0].velocity.points > ranked[1].velocity.points);
        assert_eq!(s.sweeps(), 2);
        assert!(s.seconds_until_next_sweep().is_some());
    }

    #[test]
    fn merge_sweep_installs_front_page_and_flags_flush() {
        let mut s = PulseStore::empty();
        let ids: Vec<u64> = (1..=40).collect();
        let mut flushes = 0;
        for i in 0..FLUSH_EVERY_SWEEPS {
            if s.merge_sweep(&[], ids.clone(), T0 + i64::from(i) * 60) {
                flushes += 1;
            }
        }
        assert_eq!(flushes, 1);
        assert_eq!(s.front_page().len(), FRONT_PAGE_SIZE);
        assert_eq!(s.front_page()[0], 1);
    }

    #[test]
    fn momentum_for_reports_front_page_rank() {
        let mut s = PulseStore::empty();
        s.merge_sweep(&[sweep(9, T0 - 3600, 50, 5)], vec![4, 9], T0);
        let later = T0 + MIN_SPAN_SECS + 10;
        s.merge_sweep(&[sweep(9, T0 - 3600, 60, 6)], vec![4, 9], later);
        let m = s.momentum_for(sid(9), later).unwrap();
        assert_eq!(m.front_page_rank, Some(2));
        assert!(m.velocity.points > 0.0);
        assert!(s.momentum_for(sid(4), later).is_none());
    }

    #[test]
    fn prune_drops_tracks_past_ttl() {
        let mut s = PulseStore::empty();
        s.record(
            sid(1),
            Some(T0),
            Sample {
                at: T0,
                points: 1,
                comments: 0,
            },
        );
        s.record(
            sid(2),
            Some(T0),
            Sample {
                at: T0 + TRACK_TTL_SECS,
                points: 1,
                comments: 0,
            },
        );
        s.prune(T0 + TRACK_TTL_SECS + 1);
        assert!(s.track(sid(1)).is_none());
        assert!(s.track(sid(2)).is_some());
    }

    #[test]
    fn save_and_reload_roundtrip_keeps_samples() {
        let p = tmp_path("roundtrip");
        let _ = std::fs::remove_file(&p);
        let now = chrono::Utc::now().timestamp();
        {
            let mut s = PulseStore::load_from(p.clone());
            s.record(
                sid(1),
                Some(now - 300),
                Sample {
                    at: now - 200,
                    points: 3,
                    comments: 1,
                },
            );
            s.record(
                sid(1),
                None,
                Sample {
                    at: now,
                    points: 9,
                    comments: 2,
                },
            );
            s.save();
        }
        let s2 = PulseStore::load_from(p.clone());
        let t = s2.track(sid(1)).unwrap();
        assert_eq!(t.samples.len(), 2);
        assert_eq!(t.created_at, Some(now - 300));
        assert_eq!(t.latest().unwrap().points, 9);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_prunes_expired_tracks() {
        let p = tmp_path("load_prunes");
        let _ = std::fs::remove_file(&p);
        {
            let mut s = PulseStore::load_from(p.clone());
            s.record(
                sid(1),
                Some(1000),
                Sample {
                    at: 1000,
                    points: 3,
                    comments: 1,
                },
            );
            s.save();
        }
        let s2 = PulseStore::load_from(p.clone());
        assert!(s2.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn corrupt_file_loads_as_empty() {
        let p = tmp_path("corrupt");
        std::fs::write(&p, "{not valid json").unwrap();
        let s = PulseStore::load_from(p.clone());
        assert!(s.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn eviction_bounds_size_at_max_and_removes_oldest() {
        let mut s = PulseStore::empty();
        for i in 0..(MAX_ENTRIES as u64 + 5) {
            s.record(
                sid(i),
                Some(i as i64),
                Sample {
                    at: i as i64,
                    points: 1,
                    comments: 0,
                },
            );
        }
        assert_eq!(s.len(), MAX_ENTRIES);
        for oldest in 0..5u64 {
            assert!(s.track(sid(oldest)).is_none(), "id {oldest} evicted");
        }
        assert!(s.track(sid(MAX_ENTRIES as u64 + 4)).is_some());
    }
}
