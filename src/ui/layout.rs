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
    pub header: Rect,
    pub stories: Rect,
    pub comments: Rect,
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
