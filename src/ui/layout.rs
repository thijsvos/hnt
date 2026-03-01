use ratatui::layout::{Constraint, Direction, Layout, Rect};

pub struct AppLayout {
    pub header: Rect,
    pub stories: Rect,
    pub comments: Rect,
    pub status: Rect,
}

pub fn build_layout(area: Rect) -> AppLayout {
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header
            Constraint::Min(0),   // main content
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
