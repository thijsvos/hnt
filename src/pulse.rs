//! Pulse — the momentum engine behind the `Rising` feed.
//!
//! Hacker News (and every client for it) shows a static ranked list; Pulse
//! shows the *derivative*. It keeps a short time series of `(points,
//! comments)` observations per story — a [`Track`] of [`Sample`]s — and
//! derives from it a trailing-window [`Velocity`], a fixed-width
//! [`sparkline`], and an estimate of when the story will cross onto the
//! front page ([`front_page_eta`]) using the well-known HN ranking
//! approximation ([`rank_score`]).
//!
//! Everything in this module is pure and clock-free (callers pass `now`),
//! so the ranking math is unit-testable with synthetic samples. Sampling
//! sources and persistence live in
//! [`crate::state::pulse_store::PulseStore`]; the periodic Algolia sweep
//! that feeds it is spawned by `App`.

use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Pause between two background sweeps. Sixty seconds keeps a cold start
/// under two minutes before the first velocities appear (see
/// [`MIN_SPAN_SECS`]) while staying far below Algolia's 10 000 req/hour
/// allowance (~60 sweeps/hour, two small requests each).
pub const SWEEP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);
/// How far back one Algolia `search_by_date` sweep reaches. Twelve hours
/// comfortably fits in a single 1000-hit page (a busy six hours is ~300
/// stories) and covers everything that could still be climbing.
pub const SWEEP_WINDOW_SECS: i64 = 12 * 3600;
/// Trailing window a [`Velocity`] is expressed over. `+18/30m` in the UI
/// means "eighteen points over the last thirty minutes at the current
/// pace".
pub const VELOCITY_WINDOW_SECS: i64 = 30 * 60;
/// Minimum time span between the reference and latest sample before a
/// velocity is reported. Two sweeps at [`SWEEP_INTERVAL`] are 120 s apart,
/// so 110 s means the second sweep already yields a number — a shorter
/// span would extrapolate a single point gained into a wild rate.
pub const MIN_SPAN_SECS: i64 = 110;
/// A velocity's latest sample must be at most this old for the track to
/// count as live. Prevents a store reloaded after hours away from
/// reporting ancient momentum as current.
pub const STALE_AFTER_SECS: i64 = VELOCITY_WINDOW_SECS;
/// Retained samples per story. 32 sweeps at 60 s covers the full
/// [`VELOCITY_WINDOW_SECS`] with headroom, and keeps `pulse.json` small.
pub const MAX_SAMPLES: usize = 32;
/// Samples closer together than this replace the previous one instead of
/// appending — feed pages piggyback samples on every load, and paging
/// through a feed shouldn't burn the ring on near-duplicate readings.
pub const MIN_SAMPLE_SPACING_SECS: i64 = 30;
/// Tracks whose latest sample is older than this are pruned. Momentum
/// data has no value past the velocity window; six hours is generous.
pub const TRACK_TTL_SECS: i64 = 6 * 3600;
/// Number of front-page slots considered for rank badges and the ETA
/// threshold — HN's front page is 30 stories.
pub const FRONT_PAGE_SIZE: usize = 30;
/// Minimum number of front-page stories with known points before an ETA
/// threshold is computed. With fewer, the threshold would be dominated
/// by the few (usually top-ranked) stories we happen to know and every
/// ETA would be far too pessimistic.
pub const MIN_KNOWN_FOR_THRESHOLD: usize = 20;
/// Columns in a [`sparkline`]. Each column is one eighth of
/// [`VELOCITY_WINDOW_SECS`] (3 m 45 s).
pub const SPARKLINE_WIDTH: usize = 8;
/// Furthest into the future [`front_page_eta`] will look.
pub const ETA_HORIZON_MIN: u32 = 180;
/// Step size of the ETA search.
pub const ETA_STEP_MIN: u32 = 5;
/// Points-per-window at or above which a story gets the `↗` glyph in
/// non-Rising feeds.
pub const HOT_VELOCITY: f64 = 20.0;

/// The eight block-element levels of a sparkline, lowest first. Every
/// glyph occupies exactly one terminal column, so the existing
/// `chars().count()` width math in the story list stays correct.
const SPARK_LEVELS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// One observation of a story's score and comment count.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    /// Wall-clock time of the observation, Unix seconds.
    pub at: i64,
    /// Net score at `at`.
    pub points: i64,
    /// Total comment count at `at`.
    pub comments: i64,
}

/// A story's retained time series plus the one identity fact the math
/// needs (its submission time, for age in [`rank_score`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    /// Submission time in Unix seconds — `None` when the sampling source
    /// didn't carry it (never the case for Algolia or Firebase stories,
    /// but the wire types make it optional).
    pub created_at: Option<i64>,
    /// Oldest first. Bounded by [`MAX_SAMPLES`] via [`Track::push`].
    pub samples: VecDeque<Sample>,
}

/// Points and comments gained per [`VELOCITY_WINDOW_SECS`], extrapolated
/// from the trailing span of samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity {
    /// Points gained per window.
    pub points: f64,
    /// Comments gained per window.
    pub comments: f64,
}

/// One Algolia sweep row — the subset of a search hit Pulse cares about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepSample {
    /// HN item ID.
    pub id: u64,
    /// Submission time, Unix seconds.
    pub created_at: Option<i64>,
    /// Net score at sweep time.
    pub points: i64,
    /// Comment count at sweep time.
    pub comments: i64,
}

/// A story's momentum summary — what the `Rising` rows, the `↗` glyph,
/// and the headless `hnt rising` output render.
#[derive(Debug, Clone, PartialEq)]
pub struct Momentum {
    /// HN item ID.
    pub id: u64,
    /// Points at the latest sample.
    pub points: i64,
    /// Comments at the latest sample.
    pub comments: i64,
    /// Trailing-window velocity.
    pub velocity: Velocity,
    /// Fixed-width [`sparkline`] of the trailing window.
    pub sparkline: String,
    /// 1-based position on the current front page, if the story is on it.
    pub front_page_rank: Option<usize>,
    /// Minutes until the story is projected to enter the front page at its
    /// current pace; `None` when it is already there, when no threshold is
    /// known, or when it won't make it within [`ETA_HORIZON_MIN`].
    pub eta_minutes: Option<u32>,
}

impl Track {
    /// Empty track for a story submitted at `created_at`.
    pub fn new(created_at: Option<i64>) -> Self {
        Self {
            created_at,
            samples: VecDeque::new(),
        }
    }

    /// Appends a sample, enforcing the ring size and the minimum spacing.
    /// A sample within [`MIN_SAMPLE_SPACING_SECS`] of the latest one
    /// *replaces* it (fresher reading, same slot); a sample older than the
    /// latest is ignored (clock skew between sources — keep the series
    /// monotonic rather than reorder it).
    pub fn push(&mut self, sample: Sample) {
        if let Some(last) = self.samples.back_mut() {
            if sample.at < last.at {
                return;
            }
            if sample.at - last.at < MIN_SAMPLE_SPACING_SECS {
                *last = sample;
                return;
            }
        }
        self.samples.push_back(sample);
        while self.samples.len() > MAX_SAMPLES {
            self.samples.pop_front();
        }
    }

    /// Most recent observation, if any.
    #[must_use]
    pub fn latest(&self) -> Option<Sample> {
        self.samples.back().copied()
    }

    /// Trailing-window velocity as of `now`, or `None` when the track is
    /// stale (latest sample older than [`STALE_AFTER_SECS`]), has fewer
    /// than two samples inside the window, or the span between the oldest
    /// in-window sample and the latest is shorter than [`MIN_SPAN_SECS`].
    ///
    /// The delta over the observed span is scaled up to a full window, so
    /// a story two minutes old that gained 3 points reports `+45/30m` —
    /// that *is* the current pace, and the ranking wants it.
    #[must_use]
    pub fn velocity(&self, now: i64) -> Option<Velocity> {
        let latest = self.latest()?;
        if now - latest.at > STALE_AFTER_SECS {
            return None;
        }
        let cutoff = latest.at - VELOCITY_WINDOW_SECS;
        let reference = self.samples.iter().find(|s| s.at >= cutoff)?;
        let span = latest.at - reference.at;
        if span < MIN_SPAN_SECS {
            return None;
        }
        let scale = VELOCITY_WINDOW_SECS as f64 / span as f64;
        Some(Velocity {
            points: (latest.points - reference.points) as f64 * scale,
            comments: (latest.comments - reference.comments) as f64 * scale,
        })
    }

    /// Points trajectory over the trailing window as a [`SPARKLINE_WIDTH`]
    /// glyph string. Columns are equal time buckets ending at the latest
    /// sample; a bucket with no observation at or before its end renders
    /// as a space, so a young track visibly "fills in" from the right as
    /// sweeps land.
    #[must_use]
    pub fn sparkline(&self) -> String {
        let Some(latest) = self.latest() else {
            return " ".repeat(SPARKLINE_WIDTH);
        };
        let first_at = self.samples.front().map_or(latest.at, |s| s.at);
        let bucket = VELOCITY_WINDOW_SECS / SPARKLINE_WIDTH as i64;
        let start = latest.at - VELOCITY_WINDOW_SECS;
        let values: Vec<Option<i64>> = (1..=SPARKLINE_WIDTH as i64)
            .map(|i| {
                let end = start + i * bucket;
                if end < first_at {
                    return None;
                }
                // Carry the last observation at or before the bucket end.
                self.samples
                    .iter()
                    .rev()
                    .find(|s| s.at <= end)
                    .map(|s| s.points)
            })
            .collect();
        sparkline(&values)
    }
}

impl Velocity {
    /// The number the `Rising` feed sorts by: points velocity plus half
    /// the comment velocity. Comments are a leading indicator of a thread
    /// catching fire but are noisier than votes, hence the half weight.
    #[must_use]
    pub fn rising_score(&self) -> f64 {
        self.points + 0.5 * self.comments
    }

    /// Whether this velocity earns the `↗` glyph in non-Rising feeds.
    #[must_use]
    pub fn is_hot(&self) -> bool {
        self.points >= HOT_VELOCITY
    }
}

/// Renders `values` as block-element glyphs, normalised between the
/// minimum and maximum *known* value. `None` slots render as a space. A
/// flat series (or a single known value) renders at the lowest level.
#[must_use]
pub fn sparkline(values: &[Option<i64>]) -> String {
    let known: Vec<i64> = values.iter().flatten().copied().collect();
    let (min, max) = match (known.iter().min(), known.iter().max()) {
        (Some(&min), Some(&max)) => (min, max),
        _ => return " ".repeat(values.len()),
    };
    let range = (max - min).max(1) as f64;
    let top = (SPARK_LEVELS.len() - 1) as f64;
    values
        .iter()
        .map(|v| match v {
            None => ' ',
            Some(v) => {
                let level = ((*v - min) as f64 / range * top).round() as usize;
                SPARK_LEVELS[level.min(SPARK_LEVELS.len() - 1)]
            }
        })
        .collect()
}

/// The widely documented approximation of HN's front-page ranking:
/// `(points − 1)^0.8 / (age_hours + 2)^1.8`. Only relative values matter
/// here — a story enters the front page when its score exceeds the
/// lowest-ranked story already on it.
#[must_use]
pub fn rank_score(points: i64, age_hours: f64) -> f64 {
    let p = (points - 1).max(0) as f64;
    p.powf(0.8) / (age_hours.max(0.0) + 2.0).powf(1.8)
}

/// Minutes until a story with `points` (as of `now`) and submission time
/// `created_at`, gaining `points_per_window` per [`VELOCITY_WINDOW_SECS`],
/// first has a [`rank_score`] at or above `threshold`. Searches forward in
/// [`ETA_STEP_MIN`] steps up to [`ETA_HORIZON_MIN`]; `None` if it never
/// crosses within the horizon. `Some(0)` means it already qualifies.
#[must_use]
pub fn front_page_eta(
    points: i64,
    created_at: i64,
    points_per_window: f64,
    threshold: f64,
    now: i64,
) -> Option<u32> {
    let per_minute = points_per_window / (VELOCITY_WINDOW_SECS as f64 / 60.0);
    (0..=ETA_HORIZON_MIN)
        .step_by(ETA_STEP_MIN as usize)
        .find(|&t| {
            let projected = points + (per_minute * t as f64).round() as i64;
            let age_hours = (now + i64::from(t) * 60 - created_at) as f64 / 3600.0;
            rank_score(projected, age_hours) >= threshold
        })
}

/// The [`rank_score`] a story must beat to enter the front page: the
/// lowest score among the first [`FRONT_PAGE_SIZE`] entries of
/// `front_page` whose track has a sample and a submission time. Returns
/// `None` until at least [`MIN_KNOWN_FOR_THRESHOLD`] of them are known.
#[must_use]
pub fn front_page_threshold<'a>(
    front_page: &[u64],
    lookup: impl Fn(u64) -> Option<&'a Track>,
    now: i64,
) -> Option<f64> {
    let known: Vec<f64> = front_page
        .iter()
        .take(FRONT_PAGE_SIZE)
        .filter_map(|&id| {
            let track = lookup(id)?;
            let latest = track.latest()?;
            let created = track.created_at?;
            Some(rank_score(latest.points, (now - created) as f64 / 3600.0))
        })
        .collect();
    if known.len() < MIN_KNOWN_FOR_THRESHOLD {
        return None;
    }
    known.into_iter().reduce(f64::min)
}

/// Builds the [`Momentum`] summary for one track, or `None` when the
/// track has no live velocity. `threshold` is the output of
/// [`front_page_threshold`]; pass `None` to skip the ETA.
#[must_use]
pub fn momentum(
    id: u64,
    track: &Track,
    front_page: &[u64],
    threshold: Option<f64>,
    now: i64,
) -> Option<Momentum> {
    let velocity = track.velocity(now)?;
    let latest = track.latest()?;
    let front_page_rank = front_page
        .iter()
        .take(FRONT_PAGE_SIZE)
        .position(|&fp| fp == id)
        .map(|i| i + 1);
    let eta_minutes = match (front_page_rank, threshold, track.created_at) {
        (None, Some(threshold), Some(created)) => {
            front_page_eta(latest.points, created, velocity.points, threshold, now)
        }
        _ => None,
    };
    Some(Momentum {
        id,
        points: latest.points,
        comments: latest.comments,
        velocity,
        sparkline: track.sparkline(),
        front_page_rank,
        eta_minutes,
    })
}

/// Ranks every track that is actually gaining points, fastest
/// [`Velocity::rising_score`] first (ties broken by current points),
/// truncated to `limit`. Comment velocity influences the order but can't
/// get a story in on its own — a thread that's only being argued about
/// isn't "rising" in the HN sense.
#[must_use]
pub fn rank<'a>(
    tracks: impl Iterator<Item = (u64, &'a Track)>,
    front_page: &[u64],
    threshold: Option<f64>,
    now: i64,
    limit: usize,
) -> Vec<Momentum> {
    let mut ranked: Vec<Momentum> = tracks
        .filter_map(|(id, track)| momentum(id, track, front_page, threshold, now))
        .filter(|m| m.velocity.points > 0.0 && m.velocity.rising_score() > 0.0)
        .collect();
    ranked.sort_by(|a, b| {
        b.velocity
            .rising_score()
            .partial_cmp(&a.velocity.rising_score())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.points.cmp(&a.points))
    });
    ranked.truncate(limit);
    ranked
}

/// Formats a velocity for a row: `+18/30m`, `+0/30m`, or `-3/30m`. Rounded
/// to whole points.
#[must_use]
pub fn format_velocity(velocity: &Velocity) -> String {
    let n = velocity.points.round() as i64;
    let minutes = VELOCITY_WINDOW_SECS / 60;
    if n >= 0 {
        format!("+{n}/{minutes}m")
    } else {
        format!("{n}/{minutes}m")
    }
}

/// Formats the front-page chip: `on FP #9`, `FP ~25m`, or an empty string
/// when neither applies.
#[must_use]
pub fn format_front_page(m: &Momentum) -> String {
    match (m.front_page_rank, m.eta_minutes) {
        (Some(rank), _) => format!("on FP #{rank}"),
        (None, Some(0)) => "FP now".to_string(),
        (None, Some(eta)) => format!("FP ~{eta}m"),
        (None, None) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: i64 = 1_700_000_000;

    fn s(at: i64, points: i64, comments: i64) -> Sample {
        Sample {
            at,
            points,
            comments,
        }
    }

    fn track(created_at: i64, samples: &[Sample]) -> Track {
        let mut t = Track::new(Some(created_at));
        for &sample in samples {
            t.push(sample);
        }
        t
    }

    // --- Track::push -------------------------------------------------

    #[test]
    fn push_appends_in_order_and_caps_ring() {
        let mut t = Track::new(Some(T0));
        for i in 0..(MAX_SAMPLES as i64 + 10) {
            t.push(s(T0 + i * 60, i, 0));
        }
        assert_eq!(t.samples.len(), MAX_SAMPLES);
        assert_eq!(t.samples.front().unwrap().points, 10);
        assert_eq!(t.latest().unwrap().points, MAX_SAMPLES as i64 + 9);
    }

    #[test]
    fn push_replaces_sample_within_min_spacing() {
        let mut t = Track::new(Some(T0));
        t.push(s(T0, 5, 1));
        t.push(s(T0 + MIN_SAMPLE_SPACING_SECS - 1, 7, 2));
        assert_eq!(t.samples.len(), 1);
        assert_eq!(t.latest(), Some(s(T0 + MIN_SAMPLE_SPACING_SECS - 1, 7, 2)));
        t.push(s(T0 + MIN_SAMPLE_SPACING_SECS + 30, 9, 3));
        assert_eq!(t.samples.len(), 2);
    }

    #[test]
    fn push_ignores_out_of_order_sample() {
        let mut t = Track::new(Some(T0));
        t.push(s(T0 + 600, 10, 0));
        t.push(s(T0, 1, 0));
        assert_eq!(t.samples.len(), 1);
        assert_eq!(t.latest().unwrap().points, 10);
    }

    // --- velocity ----------------------------------------------------

    #[test]
    fn velocity_none_with_single_sample() {
        let t = track(T0, &[s(T0, 10, 2)]);
        assert!(t.velocity(T0 + 60).is_none());
    }

    #[test]
    fn velocity_none_when_span_too_short() {
        let t = track(T0, &[s(T0, 10, 2), s(T0 + MIN_SPAN_SECS - 1, 14, 3)]);
        assert!(t.velocity(T0 + MIN_SPAN_SECS).is_none());
    }

    #[test]
    fn velocity_scales_span_to_window() {
        // +10 points over 15 minutes → +20 per 30-minute window.
        let t = track(T0, &[s(T0, 10, 0), s(T0 + 900, 20, 3)]);
        let v = t.velocity(T0 + 900).unwrap();
        assert!((v.points - 20.0).abs() < 1e-9, "{v:?}");
        assert!((v.comments - 6.0).abs() < 1e-9, "{v:?}");
    }

    #[test]
    fn velocity_uses_oldest_sample_inside_window() {
        // A sample 45 min old is outside the 30-min window and must be
        // ignored; the reference becomes the 20-min-old sample.
        let t = track(
            T0,
            &[s(T0, 0, 0), s(T0 + 25 * 60, 50, 0), s(T0 + 45 * 60, 60, 0)],
        );
        let v = t.velocity(T0 + 45 * 60).unwrap();
        // +10 over 20 min → +15 per 30 min.
        assert!((v.points - 15.0).abs() < 1e-9, "{v:?}");
    }

    #[test]
    fn velocity_none_when_latest_sample_is_stale() {
        let t = track(T0, &[s(T0, 10, 0), s(T0 + 900, 20, 0)]);
        assert!(t.velocity(T0 + 900 + STALE_AFTER_SECS + 1).is_none());
        assert!(t.velocity(T0 + 900 + STALE_AFTER_SECS).is_some());
    }

    #[test]
    fn velocity_can_be_negative_and_is_not_ranked() {
        let t = track(T0, &[s(T0, 30, 5), s(T0 + 900, 20, 5)]);
        let v = t.velocity(T0 + 900).unwrap();
        assert!(v.points < 0.0);
        assert!(v.rising_score() < 0.0);
        let ranked = rank(std::iter::once((1u64, &t)), &[], None, T0 + 900, 10);
        assert!(ranked.is_empty());
    }

    #[test]
    fn rising_score_weights_comments_half() {
        let v = Velocity {
            points: 10.0,
            comments: 4.0,
        };
        assert!((v.rising_score() - 12.0).abs() < 1e-9);
    }

    #[test]
    fn is_hot_threshold() {
        assert!(Velocity {
            points: HOT_VELOCITY,
            comments: 0.0
        }
        .is_hot());
        assert!(!Velocity {
            points: HOT_VELOCITY - 0.1,
            comments: 100.0
        }
        .is_hot());
    }

    // --- sparkline ---------------------------------------------------

    #[test]
    fn sparkline_normalises_min_to_max() {
        // 0/7/14/21 over a range of 21 → levels 0, 2.33→2, 4.67→5, 7.
        let out = sparkline(&[Some(0), Some(7), Some(14), Some(21)]);
        assert_eq!(out, "▁▃▆█");
    }

    #[test]
    fn sparkline_flat_series_is_lowest_level() {
        assert_eq!(sparkline(&[Some(5), Some(5), Some(5)]), "▁▁▁");
    }

    #[test]
    fn sparkline_unknown_slots_are_spaces() {
        assert_eq!(sparkline(&[None, None, Some(1), Some(9)]), "  ▁█");
        assert_eq!(sparkline(&[None, None]), "  ");
    }

    #[test]
    fn track_sparkline_is_fixed_width_and_fills_from_right() {
        let empty = Track::new(Some(T0));
        assert_eq!(empty.sparkline().chars().count(), SPARKLINE_WIDTH);
        assert!(empty.sparkline().chars().all(|c| c == ' '));

        // Two samples 5 minutes apart: only the trailing buckets are known.
        let young = track(T0, &[s(T0, 1, 0), s(T0 + 300, 9, 0)]);
        let line = young.sparkline();
        assert_eq!(line.chars().count(), SPARKLINE_WIDTH);
        assert!(line.starts_with(' '), "{line:?}");
        assert!(line.ends_with('█'), "{line:?}");

        // A full window of linear growth is a monotone ramp.
        let samples: Vec<Sample> = (0..=30).map(|m| s(T0 + m * 60, m * 2, 0)).collect();
        let full = track(T0, &samples);
        let ramp = full.sparkline();
        assert_eq!(ramp.chars().count(), SPARKLINE_WIDTH);
        assert!(!ramp.contains(' '), "{ramp:?}");
        let levels: Vec<usize> = ramp
            .chars()
            .map(|c| SPARK_LEVELS.iter().position(|&l| l == c).unwrap())
            .collect();
        assert!(levels.windows(2).all(|w| w[0] <= w[1]), "{ramp:?}");
        assert_eq!(*levels.last().unwrap(), SPARK_LEVELS.len() - 1);
    }

    // --- rank_score / ETA -------------------------------------------

    #[test]
    fn rank_score_decays_with_age_and_grows_with_points() {
        assert!(rank_score(100, 1.0) > rank_score(100, 5.0));
        assert!(rank_score(200, 1.0) > rank_score(100, 1.0));
        assert_eq!(rank_score(1, 1.0), 0.0);
        assert_eq!(rank_score(0, 1.0), 0.0);
    }

    #[test]
    fn front_page_eta_zero_when_already_qualifying() {
        let threshold = rank_score(50, 1.0);
        assert_eq!(front_page_eta(100, T0 - 3600, 0.0, threshold, T0), Some(0));
    }

    #[test]
    fn front_page_eta_finds_crossing_time() {
        // Story is 30 min old with 10 points gaining 30/30m. Threshold is
        // what a 40-point 1-hour-old story scores.
        let threshold = rank_score(40, 1.0);
        let eta = front_page_eta(10, T0 - 1800, 30.0, threshold, T0).unwrap();
        assert!(eta > 0 && eta <= ETA_HORIZON_MIN, "{eta}");
        assert_eq!(eta % ETA_STEP_MIN, 0);
        // Verify it really crosses at `eta` and not before.
        let per_min = 1.0;
        let score_at = |t: u32| {
            rank_score(
                10 + (per_min * t as f64).round() as i64,
                (1800 + i64::from(t) * 60) as f64 / 3600.0,
            )
        };
        assert!(score_at(eta) >= threshold);
        if eta >= ETA_STEP_MIN {
            assert!(score_at(eta - ETA_STEP_MIN) < threshold);
        }
    }

    #[test]
    fn front_page_eta_none_when_never_crossing() {
        let threshold = rank_score(500, 1.0);
        assert_eq!(front_page_eta(5, T0 - 600, 1.0, threshold, T0), None);
    }

    #[test]
    fn threshold_requires_enough_known_stories() {
        let tracks: Vec<(u64, Track)> = (1..=FRONT_PAGE_SIZE as u64)
            .map(|id| {
                (
                    id,
                    track(T0 - 3600 * id as i64, &[s(T0, 200 - id as i64 * 5, 0)]),
                )
            })
            .collect();
        let ids: Vec<u64> = tracks.iter().map(|(id, _)| *id).collect();
        let lookup = |id: u64| tracks.iter().find(|(i, _)| *i == id).map(|(_, t)| t);

        // Only the first few known → None.
        let few = &ids[..MIN_KNOWN_FOR_THRESHOLD - 1];
        assert!(front_page_threshold(few, lookup, T0).is_none());

        // All known → the minimum rank score across the page.
        let t = front_page_threshold(&ids, lookup, T0).unwrap();
        let expected = tracks
            .iter()
            .map(|(_, tr)| {
                rank_score(
                    tr.latest().unwrap().points,
                    (T0 - tr.created_at.unwrap()) as f64 / 3600.0,
                )
            })
            .fold(f64::INFINITY, f64::min);
        assert!((t - expected).abs() < 1e-12);
    }

    // --- momentum / rank --------------------------------------------

    #[test]
    fn momentum_marks_front_page_rank_and_skips_eta() {
        let t = track(T0 - 3600, &[s(T0 - 900, 50, 10), s(T0, 70, 14)]);
        let m = momentum(7, &t, &[3, 9, 7], Some(0.0), T0).unwrap();
        assert_eq!(m.front_page_rank, Some(3));
        assert_eq!(m.eta_minutes, None);
        assert_eq!(m.points, 70);
        assert_eq!(m.comments, 14);
        assert_eq!(format_front_page(&m), "on FP #3");
    }

    #[test]
    fn momentum_computes_eta_off_front_page() {
        // 30 minutes old, 20 points, gaining +30/30m. The bar is what a
        // 30-point, 1-hour-old story scores — just above where this story
        // sits now, so it crosses within the first few steps.
        let t = track(T0 - 1800, &[s(T0 - 900, 5, 0), s(T0, 20, 3)]);
        let threshold = rank_score(30, 1.0);
        assert!(rank_score(20, 0.5) < threshold, "fixture: not yet on FP");
        let m = momentum(7, &t, &[1, 2, 3], Some(threshold), T0).unwrap();
        assert_eq!(m.front_page_rank, None);
        let eta = m.eta_minutes.expect("should cross within the horizon");
        assert!(eta > 0 && eta <= 30, "{m:?}");
        assert!(format_front_page(&m).starts_with("FP ~"));
    }

    #[test]
    fn momentum_none_without_velocity() {
        let t = track(T0, &[s(T0, 5, 0)]);
        assert!(momentum(1, &t, &[], None, T0).is_none());
    }

    #[test]
    fn rank_sorts_fastest_first_and_truncates() {
        let slow = track(T0, &[s(T0 - 900, 0, 0), s(T0, 5, 0)]);
        let fast = track(T0, &[s(T0 - 900, 0, 0), s(T0, 50, 0)]);
        let mid = track(T0, &[s(T0 - 900, 0, 0), s(T0, 20, 0)]);
        let flat = track(T0, &[s(T0 - 900, 9, 0), s(T0, 9, 0)]);
        let tracks = [(1u64, &slow), (2, &fast), (3, &mid), (4, &flat)];
        let ranked = rank(tracks.iter().copied(), &[], None, T0, 2);
        let ids: Vec<u64> = ranked.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![2, 3]);
    }

    #[test]
    fn rank_excludes_comment_only_movement() {
        // Comments climbing, points flat: positive rising score but not
        // "rising" — must be excluded. Comments help the order otherwise.
        let argued = track(T0, &[s(T0 - 900, 10, 0), s(T0, 10, 40)]);
        let quiet = track(T0, &[s(T0 - 900, 10, 0), s(T0, 14, 0)]);
        let lively = track(T0, &[s(T0 - 900, 10, 0), s(T0, 14, 20)]);
        let ranked = rank(
            [(1u64, &argued), (2, &quiet), (3, &lively)].into_iter(),
            &[],
            None,
            T0,
            10,
        );
        let ids: Vec<u64> = ranked.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![3, 2]);
    }

    #[test]
    fn rank_breaks_ties_by_points() {
        let a = track(T0, &[s(T0 - 900, 0, 0), s(T0, 10, 0)]);
        let b = track(T0, &[s(T0 - 900, 90, 0), s(T0, 100, 0)]);
        let ranked = rank([(1u64, &a), (2, &b)].into_iter(), &[], None, T0, 10);
        assert_eq!(ranked[0].id, 2);
    }

    // --- formatting --------------------------------------------------

    #[test]
    fn format_velocity_rounds_and_signs() {
        let v = |p: f64| Velocity {
            points: p,
            comments: 0.0,
        };
        assert_eq!(format_velocity(&v(17.6)), "+18/30m");
        assert_eq!(format_velocity(&v(0.2)), "+0/30m");
        assert_eq!(format_velocity(&v(-2.5)), "-3/30m");
    }

    #[test]
    fn format_front_page_variants() {
        let base = Momentum {
            id: 1,
            points: 1,
            comments: 0,
            velocity: Velocity {
                points: 0.0,
                comments: 0.0,
            },
            sparkline: String::new(),
            front_page_rank: None,
            eta_minutes: None,
        };
        assert_eq!(format_front_page(&base), "");
        let now = Momentum {
            eta_minutes: Some(0),
            ..base.clone()
        };
        assert_eq!(format_front_page(&now), "FP now");
        let soon = Momentum {
            eta_minutes: Some(25),
            ..base
        };
        assert_eq!(format_front_page(&soon), "FP ~25m");
    }
}
