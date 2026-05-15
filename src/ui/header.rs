//! Top header bar: app brand, feed tabs, and search-mode indicator.
//!
//! Renders one row spanning the terminal width; tab order mirrors
//! [`crate::api::types::FeedKind::ALL`] so the `1`-`7` keymap stays
//! consistent with what the user sees. A `search_active` flag suppresses
//! the feed-tab highlight in favour of a "Search" chip.

use crate::api::types::FeedKind;
use crate::ui::theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};

/// Top header: brand, feed tabs (highlighting `current_feed` unless a
/// search is active), and a "Search" chip when `search_active`.
pub struct Header {
    /// Highlighted feed tab — ignored when `search_active` is true so
    /// the "Search" chip wins the active highlight.
    pub current_feed: FeedKind,
    /// Whether to paint the "Search" indicator and suppress the feed
    /// tab highlight.
    pub search_active: bool,
}

impl Widget for Header {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fill background
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_style(theme::header_style());
        }

        let mut spans = vec![
            Span::styled(" [Y] ", theme::accent_style().bg(theme::SURFACE)),
            Span::styled(
                "Hacker News Terminal  ",
                theme::title_style().bg(theme::SURFACE),
            ),
        ];

        for (i, feed) in FeedKind::ALL.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ", theme::header_style()));
            }
            let label = format!(" {} ", feed);
            if !self.search_active && *feed == self.current_feed {
                spans.push(Span::styled(label, theme::active_tab_style()));
            } else {
                spans.push(Span::styled(label, theme::inactive_tab_style()));
            }
        }

        if self.search_active {
            spans.push(Span::styled(" ", theme::header_style()));
            spans.push(Span::styled(" Search ", theme::active_tab_style()));
        }

        let line = Line::from(spans);
        buf.set_line(area.left(), area.top(), &line, area.width);
    }
}
