//! Strip terminal control bytes from untrusted HN content.
//!
//! HN comments and titles flow through `html2text`, which decodes HTML
//! entities — so a comment containing `&#x1b;[2J` re-emerges as a raw
//! `0x1B` byte. ratatui forwards bytes to the terminal verbatim, which
//! lets a malicious submitter rewrite the user's window title, palette,
//! scroll region, or (on some terminals) trigger query responses that
//! could cause command injection. The fix: replace C0/C1 controls and
//! DEL with the Unicode replacement glyph before they reach a `Span`.
//!
//! `\t` and `\n` are preserved because ratatui itself handles those
//! glyphs; everything else in the C0 range, plus `0x7F` (DEL) and the
//! C1 range (`0x80..=0x9F`), is replaced.

use std::borrow::Cow;

/// Returns `s` unchanged if it contains no control bytes, otherwise an
/// owned [`String`] with `\u{FFFD}` (REPLACEMENT CHARACTER) substituted
/// for any C0 / C1 / DEL byte. `\t` (0x09) and `\n` (0x0A) are kept.
#[must_use]
pub fn sanitize_terminal(s: &str) -> Cow<'_, str> {
    if !s.chars().any(is_control_to_strip) {
        return Cow::Borrowed(s);
    }
    Cow::Owned(
        s.chars()
            .map(|c| {
                if is_control_to_strip(c) {
                    '\u{FFFD}'
                } else {
                    c
                }
            })
            .collect(),
    )
}

#[inline]
fn is_control_to_strip(c: char) -> bool {
    match c {
        '\t' | '\n' => false,
        '\u{0000}'..='\u{001F}' => true,
        '\u{007F}' => true,
        '\u{0080}'..='\u{009F}' => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_is_borrowed_unchanged() {
        let s = "hello world";
        let out = sanitize_terminal(s);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, "hello world");
    }

    #[test]
    fn unicode_letters_are_borrowed() {
        let s = "café résumé 日本語";
        let out = sanitize_terminal(s);
        assert!(matches!(out, Cow::Borrowed(_)));
        assert_eq!(out, s);
    }

    #[test]
    fn esc_is_replaced() {
        let s = "before\x1b[2Jafter";
        let out = sanitize_terminal(s);
        assert_eq!(out, "before\u{FFFD}[2Jafter");
    }

    #[test]
    fn osc_window_title_is_neutralised() {
        // OSC 0; sets terminal title — most damaging in practice.
        let s = "\x1b]0;owned\x07";
        let out = sanitize_terminal(s);
        assert!(!out.contains('\x1b'));
        assert!(!out.contains('\x07'));
    }

    #[test]
    fn tab_and_newline_are_preserved() {
        let s = "a\tb\nc";
        let out = sanitize_terminal(s);
        assert_eq!(out, "a\tb\nc");
    }

    #[test]
    fn del_byte_is_replaced() {
        let s = "x\x7fy";
        let out = sanitize_terminal(s);
        assert_eq!(out, "x\u{FFFD}y");
    }

    #[test]
    fn c1_range_is_replaced() {
        let s = "x\u{0085}y\u{0090}z";
        let out = sanitize_terminal(s);
        assert_eq!(out, "x\u{FFFD}y\u{FFFD}z");
    }
}
