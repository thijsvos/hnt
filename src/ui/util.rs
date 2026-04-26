//! Small UI helpers shared across widgets.

/// Truncates `s` to at most `max` characters (operating on `char`s to
/// stay UTF-8-safe), appending `...` when truncation occurs. Returns
/// `s` unchanged when it already fits. `max < 3` collapses to just the
/// ellipsis (or to `max` chars of it) so callers passing tiny widths
/// still get a sensible visual.
pub fn truncate_to(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let keep = max.saturating_sub(3);
    let mut out: String = s.chars().take(keep).collect();
    out.push_str("...");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn returns_unchanged_when_fits() {
        assert_eq!(truncate_to("abc", 10), "abc");
        assert_eq!(truncate_to("abc", 3), "abc");
    }

    #[test]
    fn truncates_with_ellipsis() {
        assert_eq!(truncate_to("abcdefghij", 6), "abc...");
    }

    #[test]
    fn unicode_safe() {
        // 5 multi-byte chars; max=4 should keep 1 char + "..."
        assert_eq!(truncate_to("日本語のテスト", 4), "日...");
    }

    #[test]
    fn max_zero_returns_just_ellipsis() {
        // saturating_sub clamps `max - 3` at 0 → all that's left is "...".
        assert_eq!(truncate_to("abcdef", 0), "...");
    }
}
