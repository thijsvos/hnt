//! Bottom status bar: mode indicator, keybinding hint, position counter.
//!
//! Renders three visual modes — search-input prompt, search-results
//! banner, and normal — plus a right-aligned `N/total [Pane]` counter.

use crate::api::types::FeedKind;
use crate::keys::InputMode;
use crate::sanitize::sanitize_terminal;
use crate::ui::theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Modifier,
    text::{Line, Span},
    widgets::Widget,
};

/// Bottom status bar. Display mode depends on `input_mode` and
/// `search_query`: a `/` prompt during input, a search-results banner
/// while search results are shown, or the normal feed/hint line.
///
/// All string fields borrow from [`crate::app::App`]. The widget is
/// rebuilt per-frame and consumed immediately, so ownership is
/// unnecessary — cloning the same strings every frame was wasted work.
pub struct StatusBar<'a> {
    /// Currently selected feed — drives the `[<Feed>]` chip on the
    /// left.
    pub feed: FeedKind,
    /// Pre-formatted "N/total" counter for the focused pane, built by
    /// `ui::render`.
    pub position: &'a str,
    /// Last error to surface (sanitised at render time). `None` paints
    /// the normal keybinding hint line instead (when `info` is also
    /// `None`). Rendered with the `Error:` prefix and red foreground.
    pub error: Option<&'a str>,
    /// Transient, non-error status message — auto-expiring toast set via
    /// [`crate::app::App::set_info`]. Takes precedence over `error` and
    /// over the keybinding hints; rendered without the `Error:` prefix
    /// and in the theme's accent style (not red). Same C0/C1 sanitisation
    /// applies — `URL copied: <url>` carries server-controlled bytes.
    pub info: Option<&'a str>,
    /// Pane label for the right-aligned `[Stories]` / `[Comments]` tag.
    pub focus_pane: &'static str,
    /// Current input mode — drives the search-input vs normal layout
    /// branch.
    pub input_mode: InputMode,
    /// In-progress search input — rendered with a block cursor when
    /// `input_mode == SearchInput`.
    pub search_input: Option<&'a str>,
    /// Committed search query — rendered as the `Search: "<q>"` chip
    /// while results are shown.
    pub search_query: Option<&'a str>,
    /// In-progress `:command` input — rendered with a `:` chip and block
    /// cursor when `input_mode == CommandInput`. Sanitised at the render
    /// boundary the same way `error` and `info` are, even though the
    /// content originates from the user — defence in depth, since a
    /// future paste path or `:source <file>` could surface server-bytes
    /// through this widget.
    pub command_input: Option<&'a str>,
    /// Byte offset of the command-input cursor within [`Self::command_input`]
    /// (always at a char boundary). Drives where the block cursor is drawn
    /// so Left/Right/Home/End line editing is visible. `None` (or out of
    /// range) falls back to end-of-line.
    pub command_cursor: Option<usize>,
}

impl<'a> Widget for StatusBar<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fill background
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_style(theme::status_style());
        }

        let mut spans = Vec::new();

        if self.input_mode == InputMode::SearchInput {
            // Search input mode
            let input = self.search_input.unwrap_or("");
            // Sanitise the echoed buffer — same defence-in-depth as the
            // CommandInput branch; a pasted query could carry C0/C1 bytes.
            let safe_input = sanitize_terminal(input);
            spans.push(Span::styled(
                " / ",
                theme::accent_style().bg(theme::SURFACE),
            ));
            spans.push(Span::styled(
                format!("{}\u{2588}", safe_input),
                theme::status_style(),
            ));
            spans.push(Span::styled(
                " (Enter:search  Esc:cancel)",
                theme::dim_style(),
            ));
        } else if self.input_mode == InputMode::CommandInput {
            // `:`-command input mode
            let input = self.command_input.unwrap_or("");
            spans.push(Span::styled(
                " : ",
                theme::accent_style().bg(theme::SURFACE),
            ));
            // Draw the block cursor at its actual byte offset so Left/Right/
            // Home/End editing is visible: split at the cursor, reverse-video
            // the char under it (or a trailing full block at end-of-line).
            // Each piece is sanitised independently — splitting on a char
            // boundary keeps that safe.
            let cursor = self.command_cursor.unwrap_or(input.len()).min(input.len());
            let cursor = if input.is_char_boundary(cursor) {
                cursor
            } else {
                input.len()
            };
            let (before, after) = input.split_at(cursor);
            spans.push(Span::styled(
                sanitize_terminal(before).into_owned(),
                theme::status_style(),
            ));
            let mut after_chars = after.chars();
            match after_chars.next() {
                Some(ch) => {
                    spans.push(Span::styled(
                        sanitize_terminal(&ch.to_string()).into_owned(),
                        theme::status_style().add_modifier(Modifier::REVERSED),
                    ));
                    let rest: String = after_chars.collect();
                    spans.push(Span::styled(
                        sanitize_terminal(&rest).into_owned(),
                        theme::status_style(),
                    ));
                }
                None => spans.push(Span::styled("\u{2588}", theme::status_style())),
            }
            // Tab-completion surfaces its candidate list via an info toast
            // (`Matches: :refresh  :reader`). Show it here when present —
            // otherwise this branch swallowed it and the feature was
            // invisible. Falls back to the key hint once the toast expires.
            if let Some(info) = self.info {
                let safe_info = sanitize_terminal(info);
                spans.push(Span::styled(
                    format!("  {} ", safe_info),
                    theme::accent_style().bg(theme::SURFACE),
                ));
            } else {
                spans.push(Span::styled(
                    " (Enter:run  Tab:complete  ↑/↓:history  Esc:cancel)",
                    theme::dim_style(),
                ));
            }
        } else if let Some(query) = self.search_query {
            // Search results mode. Sanitise the echoed query — defence-in-depth
            // matching the search-input echo and command prompt above.
            spans.push(Span::styled(
                format!(" Search: \"{}\" ", sanitize_terminal(query)),
                theme::accent_style().bg(theme::SURFACE),
            ));
            spans.push(Span::styled(" ", theme::status_style()));

            if let Some(info) = self.info {
                let safe_info = sanitize_terminal(info);
                spans.push(Span::styled(
                    format!("{} ", safe_info),
                    theme::accent_style().bg(theme::SURFACE),
                ));
            } else if let Some(err) = self.error {
                // Errors can carry server-controlled bytes (URLs from
                // Location headers, hostnames from DNS errors), so scrub
                // C0/C1/DEL controls before they reach ratatui — same
                // rationale as the C2 title sanitiser, just on a
                // lower-bandwidth attack surface.
                let safe_err = sanitize_terminal(err);
                spans.push(Span::styled(
                    format!("Error: {} ", safe_err),
                    ratatui::style::Style::default()
                        .fg(theme::RED)
                        .bg(theme::SURFACE),
                ));
            } else {
                spans.push(Span::styled(
                    "j/k:nav enter:comments o:browser p:read h:prior /:new search esc:back ?:help ",
                    theme::status_style(),
                ));
            }
        } else {
            // Normal mode
            spans.push(Span::styled(
                format!(" [{}] ", self.feed),
                theme::accent_style().bg(theme::SURFACE),
            ));
            spans.push(Span::styled(" ", theme::status_style()));

            if let Some(info) = self.info {
                let safe_info = sanitize_terminal(info);
                spans.push(Span::styled(
                    format!("{} ", safe_info),
                    theme::accent_style().bg(theme::SURFACE),
                ));
            } else if let Some(err) = self.error {
                // Errors can carry server-controlled bytes (URLs from
                // Location headers, hostnames from DNS errors), so scrub
                // C0/C1/DEL controls before they reach ratatui — same
                // rationale as the C2 title sanitiser, just on a
                // lower-bandwidth attack surface.
                let safe_err = sanitize_terminal(err);
                spans.push(Span::styled(
                    format!("Error: {} ", safe_err),
                    ratatui::style::Style::default()
                        .fg(theme::RED)
                        .bg(theme::SURFACE),
                ));
            } else {
                spans.push(Span::styled(
                    "j/k:nav tab:switch enter:open 1-7:feed b:pin /:search o:browser p:read h:prior r:refresh ?:help q:quit ",
                    theme::status_style(),
                ));
            }
        }

        // Right-aligned position indicator. Use `chars().count()` rather
        // than byte `.len()` so the alignment stays correct if any field
        // ever picks up a non-ASCII glyph.
        let right_text = format!(" {} [{}] ", self.position, self.focus_pane);
        let right_start = area
            .right()
            .saturating_sub(right_text.chars().count() as u16);

        let line = Line::from(spans);
        buf.set_line(area.left(), area.top(), &line, area.width);

        let right_span = Span::styled(right_text, theme::accent_style().bg(theme::SURFACE));
        buf.set_span(right_start, area.top(), &right_span, area.width);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;

    fn render_bar(err: Option<&str>) -> Buffer {
        render_bar_with(err, None)
    }

    fn render_bar_with(err: Option<&str>, info: Option<&str>) -> Buffer {
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: err,
            info,
            focus_pane: "Stories",
            input_mode: InputMode::Normal,
            search_input: None,
            search_query: None,
            command_input: None,
            command_cursor: None,
        }
        .render(area, &mut buf);
        buf
    }

    fn buffer_text(buf: &Buffer) -> String {
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
        }
        s
    }

    #[test]
    fn status_bar_renders_normal_mode_without_panic() {
        let buf = render_bar(None);
        assert!(buffer_text(&buf).contains("Top"));
    }

    #[test]
    fn status_bar_renders_error_message() {
        let buf = render_bar(Some("network down"));
        let text = buffer_text(&buf);
        assert!(text.contains("Error:"));
        assert!(text.contains("network down"));
    }

    #[test]
    fn status_bar_neutralises_escape_in_error_message() {
        // Reproduces the C-W1 attack: an error string carries embedded
        // OSC-0 bytes. Confirm none of \x1b, \x07, or other C0/C1
        // controls reach the rendered buffer cells.
        let buf = render_bar(Some("oops\x1b]0;OWNED\x07more"));
        let text = buffer_text(&buf);
        assert!(!text.contains('\x1b'), "ESC must not survive: {text:?}");
        assert!(!text.contains('\x07'), "BEL must not survive");
        assert!(text.contains("oops"));
        assert!(text.contains("more"));
    }

    #[test]
    fn status_bar_neutralises_csi_in_error_message_search_branch() {
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: Some("hit\x1b[2Jclear"),
            info: None,
            focus_pane: "Stories",
            input_mode: InputMode::Normal,
            search_input: None,
            // Force the search-results error branch.
            search_query: Some("rust"),
            command_input: None,
            command_cursor: None,
        }
        .render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains('\x1b'));
        assert!(text.contains("hit"));
        assert!(text.contains("clear"));
    }

    #[test]
    fn status_bar_neutralises_escape_in_search_query() {
        // The committed search query is echoed as the results-mode label; scrub
        // C0/C1/OSC bytes there too — defence-in-depth matching the search-input
        // echo and the command prompt.
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: None,
            info: None,
            focus_pane: "Stories",
            input_mode: InputMode::Normal,
            search_input: None,
            search_query: Some("rust\x1b]0;OWNED\x07lang"),
            command_input: None,
            command_cursor: None,
        }
        .render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains('\x1b'), "ESC must not survive: {text:?}");
        assert!(!text.contains('\x07'), "BEL must not survive");
        assert!(text.contains("rust"));
        assert!(text.contains("lang"));
    }

    fn render_command_bar(input: &str) -> Buffer {
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: None,
            info: None,
            focus_pane: "Stories",
            input_mode: InputMode::CommandInput,
            search_input: None,
            search_query: None,
            command_input: Some(input),
            command_cursor: None,
        }
        .render(area, &mut buf);
        buf
    }

    #[test]
    fn command_input_renders_prompt_and_buffer() {
        let buf = render_command_bar("filter author");
        let text = buffer_text(&buf);
        assert!(text.contains(':'), "command prompt missing: {text:?}");
        assert!(
            text.contains("filter author"),
            "typed line missing: {text:?}"
        );
    }

    #[test]
    fn command_input_renders_hint_line() {
        let buf = render_command_bar("");
        let text = buffer_text(&buf);
        assert!(text.contains("Enter:run"), "hint line missing: {text:?}");
        assert!(text.contains("Esc:cancel"));
    }

    #[test]
    fn command_input_renders_cursor_mid_line() {
        // With the cursor in the middle of the line, every character still
        // renders (the cursor cell carries the char under it, reverse-styled).
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: None,
            info: None,
            focus_pane: "Stories",
            input_mode: InputMode::CommandInput,
            search_input: None,
            search_query: None,
            command_input: Some("feed top"),
            command_cursor: Some(4),
        }
        .render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(text.contains("feed top"), "full line missing: {text:?}");
    }

    #[test]
    fn search_input_neutralises_terminal_escapes() {
        // The search prompt echoes a user buffer too — scrub C0/C1/OSC bytes
        // the same way the command prompt does.
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: None,
            info: None,
            focus_pane: "Stories",
            input_mode: InputMode::SearchInput,
            search_input: Some("rust\x1b]0;OWNED\x07lang"),
            search_query: None,
            command_input: None,
            command_cursor: None,
        }
        .render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains('\x1b'), "ESC must not survive: {text:?}");
        assert!(!text.contains('\x07'), "BEL must not survive");
        assert!(text.contains("rust"));
        assert!(text.contains("lang"));
    }

    #[test]
    fn command_input_renders_info_toast_over_hint() {
        // Tab-completion delivers its candidate list as an info toast; it must
        // be visible while the `:` prompt is open (previously swallowed).
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: None,
            info: Some("Matches: :refresh  :reader"),
            focus_pane: "Stories",
            input_mode: InputMode::CommandInput,
            search_input: None,
            search_query: None,
            command_input: Some("re"),
            command_cursor: None,
        }
        .render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(
            text.contains("Matches: :refresh"),
            "info toast must render in command mode: {text:?}"
        );
    }

    #[test]
    fn command_input_neutralises_terminal_escapes() {
        // Defence in depth: even though the user types this themselves
        // today, a future `:source <file>` or paste path could surface
        // server-bytes through this widget — same C0/C1 stripping as the
        // error branches.
        let buf = render_command_bar("feed \x1b]0;OWNED\x07top");
        let text = buffer_text(&buf);
        assert!(!text.contains('\x1b'), "ESC must not survive: {text:?}");
        assert!(!text.contains('\x07'), "BEL must not survive");
        assert!(text.contains("feed"));
        assert!(text.contains("top"));
    }

    #[test]
    fn info_toast_renders_without_error_prefix() {
        let buf = render_bar_with(None, Some("URL copied: https://example.com"));
        let text = buffer_text(&buf);
        assert!(text.contains("URL copied:"));
        assert!(text.contains("https://example.com"));
        assert!(
            !text.contains("Error:"),
            "info toast must not be styled/prefixed as an error: {text:?}"
        );
    }

    #[test]
    fn info_toast_takes_precedence_over_error() {
        let buf = render_bar_with(Some("network down"), Some("URL copied: https://x"));
        let text = buffer_text(&buf);
        assert!(text.contains("URL copied:"));
        assert!(
            !text.contains("Error:"),
            "info should preempt error rendering: {text:?}"
        );
        assert!(!text.contains("network down"));
    }
}
