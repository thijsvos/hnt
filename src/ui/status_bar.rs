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
    /// the normal keybinding hint line instead.
    pub error: Option<&'a str>,
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
            spans.push(Span::styled(
                " / ",
                theme::accent_style().bg(theme::SURFACE),
            ));
            spans.push(Span::styled(
                format!("{}\u{2588}", input),
                theme::status_style(),
            ));
            spans.push(Span::styled(
                " (Enter:search  Esc:cancel)",
                theme::dim_style(),
            ));
        } else if let Some(query) = self.search_query {
            // Search results mode
            spans.push(Span::styled(
                format!(" Search: \"{}\" ", query),
                theme::accent_style().bg(theme::SURFACE),
            ));
            spans.push(Span::styled(" ", theme::status_style()));

            if let Some(err) = self.error {
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

            if let Some(err) = self.error {
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
        let area = Rect::new(0, 0, 120, 1);
        let mut buf = Buffer::empty(area);
        StatusBar {
            feed: FeedKind::Top,
            position: "1/1",
            error: err,
            focus_pane: "Stories",
            input_mode: InputMode::Normal,
            search_input: None,
            search_query: None,
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
            focus_pane: "Stories",
            input_mode: InputMode::Normal,
            search_input: None,
            // Force the search-results error branch.
            search_query: Some("rust"),
        }
        .render(area, &mut buf);
        let text = buffer_text(&buf);
        assert!(!text.contains('\x1b'));
        assert!(text.contains("hit"));
        assert!(text.contains("clear"));
    }
}
