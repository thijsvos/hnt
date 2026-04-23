//! Loading-spinner frame generator.
//!
//! [`frame`] maps a monotonic tick count to one of six braille glyphs,
//! reusing a single `indicatif::ProgressStyle` via [`OnceLock`].

use indicatif::ProgressStyle;
use std::sync::OnceLock;

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
