use indicatif::ProgressStyle;
use std::sync::OnceLock;

const FRAMES: &[&str] = &["⠾", "⠷", "⠯", "⠟", "⠻", "⠽"];

fn style() -> &'static ProgressStyle {
    static STYLE: OnceLock<ProgressStyle> = OnceLock::new();
    STYLE.get_or_init(|| ProgressStyle::default_spinner().tick_strings(FRAMES))
}

pub fn frame(tick: u64) -> &'static str {
    style().get_tick_str(tick)
}
