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

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::style::Color;

    fn render_header(current_feed: FeedKind, search_active: bool, area: Rect) -> Buffer {
        let mut buf = Buffer::empty(area);
        Header {
            current_feed,
            search_active,
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

    fn count_cells_with_bg(buf: &Buffer, color: Color, y: u16) -> usize {
        let mut n = 0;
        for x in 0..buf.area.width {
            if buf[(x, y)].bg == color {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn header_renders_brand_and_all_feed_labels() {
        let buf = render_header(FeedKind::Top, false, Rect::new(0, 0, 120, 1));
        let text = buffer_text(&buf);

        assert!(text.contains("[Y]"), "brand missing: {text:?}");
        assert!(
            text.contains("Hacker News Terminal"),
            "title missing: {text:?}"
        );
        for feed in FeedKind::ALL {
            let label = feed.to_string();
            assert!(
                text.contains(&label),
                "feed label {label:?} missing from header: {text:?}"
            );
        }
    }

    #[test]
    fn header_active_tab_carries_orange_highlight() {
        // For every feed, render with it as `current_feed` (no search)
        // and confirm the count of HN_ORANGE-bg cells matches the
        // active-tab label width — proving (a) the active tab is
        // highlighted and (b) no other span uses HN_ORANGE bg.
        for feed in FeedKind::ALL {
            let buf = render_header(feed, false, Rect::new(0, 0, 120, 1));
            let expected_width = format!(" {} ", feed).chars().count();
            let actual = count_cells_with_bg(&buf, theme::HN_ORANGE, 0);
            assert_eq!(
                actual, expected_width,
                "expected {expected_width} HN_ORANGE-bg cells for active tab {feed}, got {actual}"
            );
        }
    }

    #[test]
    fn header_search_active_shows_chip_and_suppresses_feed_highlight() {
        // With search_active=true and current_feed=Top, only the
        // " Search " chip (8 cells) should carry the HN_ORANGE bg —
        // the Top tab must NOT be highlighted.
        let buf = render_header(FeedKind::Top, true, Rect::new(0, 0, 120, 1));
        let text = buffer_text(&buf);
        assert!(text.contains("Search"), "Search chip missing: {text:?}");
        let orange = count_cells_with_bg(&buf, theme::HN_ORANGE, 0);
        assert_eq!(
            orange, 8,
            "search-active mode should highlight only the 8-cell ' Search ' chip, got {orange} HN_ORANGE cells"
        );
    }

    #[test]
    fn header_search_chip_absent_in_normal_mode() {
        let buf = render_header(FeedKind::Top, false, Rect::new(0, 0, 120, 1));
        let text = buffer_text(&buf);
        assert!(
            !text.contains("Search"),
            "Search chip must not appear when search_active=false: {text:?}"
        );
    }

    #[test]
    fn header_background_fill_paints_styled_bg_across_row() {
        // The leading fill loop paints SURFACE across every cell at
        // y=top; spans then override some cells to HN_ORANGE for the
        // active tab. No cell should keep the default Color::Reset bg.
        let buf = render_header(FeedKind::Top, false, Rect::new(0, 0, 120, 1));
        for x in 0..120 {
            let bg = buf[(x, 0)].bg;
            assert_ne!(
                bg,
                Color::Reset,
                "cell ({x},0) has default bg — background fill loop missed it"
            );
        }
    }

    #[test]
    fn header_paints_only_top_row_on_multi_row_area() {
        // Pins the contract that Header writes to area.top() only —
        // rows below stay at Buffer::empty defaults.
        let buf = render_header(FeedKind::Top, false, Rect::new(0, 0, 80, 3));
        for y in 1..3 {
            for x in 0..80 {
                let cell = &buf[(x, y)];
                assert_eq!(
                    cell.bg,
                    Color::Reset,
                    "cell ({x},{y}) bg should be Reset, got {:?}",
                    cell.bg
                );
                assert_eq!(
                    cell.symbol(),
                    " ",
                    "cell ({x},{y}) symbol should be default ' ', got {:?}",
                    cell.symbol()
                );
            }
        }
    }

    #[test]
    fn header_zero_size_area_does_not_panic() {
        // Width=0 height=0 and width=0 height=1 — both should survive
        // without panicking (the fill loop is empty and set_line
        // returns immediately at max_width=0).
        let _ = render_header(FeedKind::Top, false, Rect::new(0, 0, 0, 0));
        let _ = render_header(FeedKind::Top, false, Rect::new(0, 0, 0, 1));
        let _ = render_header(FeedKind::Top, true, Rect::new(0, 0, 0, 1));
    }
}
