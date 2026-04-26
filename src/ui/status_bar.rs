//! Bottom status bar: mode indicator, keybinding hint, position counter.
//!
//! Renders three visual modes — search-input prompt, search-results
//! banner, and normal — plus a right-aligned `N/total [Pane]` counter.

use crate::api::types::FeedKind;
use crate::keys::InputMode;
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
    pub feed: FeedKind,
    pub position: &'a str,
    pub error: Option<&'a str>,
    pub focus_pane: &'static str,
    pub input_mode: InputMode,
    pub search_input: Option<&'a str>,
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
                spans.push(Span::styled(
                    format!("Error: {} ", err),
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
                spans.push(Span::styled(
                    format!("Error: {} ", err),
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
