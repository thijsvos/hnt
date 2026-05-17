//! Top-level terminal layout: header, two content panes, status bar.
//!
//! Produces an [`AppLayout`] with fixed 1-row header and status rows
//! and a 35/65 horizontal split for stories/comments. Shared by
//! `ui::render` and by click/scroll hit-testing in `app.rs`.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Rects for the four top-level regions of the screen, in draw order
/// top-to-bottom: [`header`](Self::header),
/// [`stories`](Self::stories) / [`comments`](Self::comments) side-by-side,
/// [`status`](Self::status).
pub struct AppLayout {
    /// 1-row top header bar.
    pub header: Rect,
    /// Left content pane — 35% of the body width.
    pub stories: Rect,
    /// Right content pane — 65% of the body width.
    pub comments: Rect,
    /// 1-row bottom status bar.
    pub status: Rect,
}

/// Splits `area` into header/body/status (1/flex/1 rows), then splits the
/// body 35/65 into stories/comments.
pub fn build_layout(area: Rect) -> AppLayout {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(0),    // main content
            Constraint::Length(1), // status bar
        ])
        .split(area);

    let main = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(35), // stories
            Constraint::Percentage(65), // comments
        ])
        .split(outer[1]);

    AppLayout {
        header: outer[0],
        stories: main[0],
        comments: main[1],
        status: outer[2],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_layout_standard_80x24() {
        let layout = build_layout(Rect::new(0, 0, 80, 24));

        assert_eq!(layout.header, Rect::new(0, 0, 80, 1));
        assert_eq!(layout.status, Rect::new(0, 23, 80, 1));
        assert_eq!(layout.stories.y, 1);
        assert_eq!(layout.stories.height, 22);
        assert_eq!(layout.comments.y, 1);
        assert_eq!(layout.comments.height, 22);
        assert_eq!(layout.stories.width + layout.comments.width, 80);
        assert_eq!(layout.stories.x, 0);
        assert_eq!(layout.comments.right(), 80);
    }

    #[test]
    fn build_layout_wide_200x60() {
        let layout = build_layout(Rect::new(0, 0, 200, 60));

        assert_eq!(layout.header.height, 1);
        assert_eq!(layout.status.height, 1);
        assert_eq!(layout.stories.width + layout.comments.width, 200);
        assert_eq!(
            layout.stories.width, 70,
            "stories should be 35% of 200 = 70, got {}",
            layout.stories.width
        );
        assert_eq!(
            layout.comments.width, 130,
            "comments should be 65% of 200 = 130, got {}",
            layout.comments.width
        );
    }

    #[test]
    fn build_layout_non_zero_origin() {
        let layout = build_layout(Rect::new(5, 5, 20, 10));

        assert_eq!(layout.header.x, 5);
        assert_eq!(layout.header.y, 5);
        assert_eq!(layout.header.width, 20);
        assert_eq!(layout.status.y, 14);
        assert_eq!(layout.stories.x, 5);
        assert_eq!(layout.comments.right(), 25);
        assert_eq!(layout.stories.width + layout.comments.width, 20);
    }

    #[test]
    fn build_layout_no_left_right_gap() {
        for width in [80u16, 100, 137] {
            let area = Rect::new(0, 0, width, 24);
            let layout = build_layout(area);
            assert_eq!(
                layout.stories.x, area.x,
                "stories.x must equal area.x at width {width}"
            );
            assert_eq!(
                layout.comments.right(),
                area.right(),
                "comments.right() must equal area.right() at width {width}"
            );
        }
    }

    #[test]
    fn build_layout_height_two_collapses_body() {
        let layout = build_layout(Rect::new(0, 0, 80, 2));

        assert_eq!(layout.header.height, 1);
        assert_eq!(layout.status.height, 1);
        assert_eq!(layout.stories.height, 0);
        assert_eq!(layout.comments.height, 0);
    }

    #[test]
    fn build_layout_height_one_does_not_panic() {
        // With three constraints (Length(1) + Min(0) + Length(1)) but only
        // one row available, ratatui's solver cannot satisfy both Length(1)
        // constraints — it compromises. We don't pin a specific assignment
        // (that's ratatui's prerogative); we just assert (a) no panic and
        // (b) the row heights stay within the available area.
        let layout = build_layout(Rect::new(0, 0, 80, 1));
        let total = u32::from(layout.header.height)
            + u32::from(layout.stories.height)
            + u32::from(layout.status.height);
        assert!(
            total <= 1,
            "total assigned vertical height ({total}) must not exceed area height (1)"
        );
        assert_eq!(layout.stories.y, layout.comments.y);
        assert_eq!(layout.stories.height, layout.comments.height);
    }

    #[test]
    fn build_layout_zero_area_does_not_panic() {
        let layout = build_layout(Rect::new(0, 0, 0, 0));

        assert_eq!(layout.header.width, 0);
        assert_eq!(layout.header.height, 0);
        assert_eq!(layout.stories.width, 0);
        assert_eq!(layout.stories.height, 0);
        assert_eq!(layout.comments.width, 0);
        assert_eq!(layout.comments.height, 0);
        assert_eq!(layout.status.width, 0);
        assert_eq!(layout.status.height, 0);
    }
}
