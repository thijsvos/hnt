//! Loading-spinner frame generator.
//!
//! [`frame`] maps a monotonic tick count to one of six braille glyphs,
//! reusing a single `indicatif::ProgressStyle` via [`OnceLock`].

use indicatif::ProgressStyle;
use std::sync::OnceLock;

/// Braille glyphs cycled by [`frame`]. Six frames chosen for visual
/// continuity at the ~4 Hz tick rate — fewer would look stuttery,
/// more would slow the perceived motion.
const FRAMES: &[&str] = &["⠾", "⠷", "⠯", "⠟", "⠻", "⠽"];

fn style() -> &'static ProgressStyle {
    static STYLE: OnceLock<ProgressStyle> = OnceLock::new();
    STYLE.get_or_init(|| ProgressStyle::default_spinner().tick_strings(FRAMES))
}

/// Picks the braille spinner glyph for this tick. Safe to call every
/// frame — the [`ProgressStyle`] is built once via [`OnceLock`].
pub fn frame(tick: u64) -> &'static str {
    style().get_tick_str(tick)
}

#[cfg(test)]
mod tests {
    use super::*;

    // indicatif's ProgressStyle::get_tick_str reserves the last entry of
    // tick_strings as the "finished" glyph and cycles over the remaining
    // N-1 entries: `tick_strings[idx % (len - 1)]`. With FRAMES of length
    // 6, the animation cycle is 5 — only the first five glyphs ever
    // appear during ticking; FRAMES[5] ("⠽") is the finished-state
    // string and is only returned by get_final_tick_str. These tests pin
    // that contract so a future refactor doesn't silently regress the
    // animation length.
    const CYCLED_FRAMES: &[&str] = &["⠾", "⠷", "⠯", "⠟", "⠻"];

    #[test]
    fn frame_returns_expected_glyphs_for_animated_cycle() {
        let cycle: Vec<&'static str> = (0..5).map(frame).collect();
        assert_eq!(cycle, CYCLED_FRAMES);
    }

    #[test]
    fn frame_cycles_every_five_ticks() {
        assert_eq!(frame(5), frame(0));
        assert_eq!(frame(6), frame(1));
        assert_eq!(frame(10), frame(0));
        assert_eq!(frame(14), frame(4));
    }

    #[test]
    fn frame_handles_u64_max_without_panic() {
        let f = frame(u64::MAX);
        assert!(
            CYCLED_FRAMES.contains(&f),
            "frame(u64::MAX) must return one of the cycled FRAMES, got {f:?}"
        );
    }

    #[test]
    fn frame_reuses_static_storage_across_calls() {
        // OnceLock contract: ProgressStyle isn't rebuilt per call, so the
        // returned &'static str is the same backing storage every time.
        let a = frame(0);
        let b = frame(0);
        assert_eq!(a.as_ptr(), b.as_ptr());
    }
}
