use crate::api::types::FeedKind;
use crate::ui::theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::Widget,
};

pub struct StatusBar {
    pub feed: FeedKind,
    pub position: String,
    pub error: Option<String>,
    pub focus_pane: &'static str,
}

impl Widget for StatusBar {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Fill background
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_style(theme::status_style());
        }

        let mut spans = vec![
            Span::styled(
                format!(" [{}] ", self.feed),
                theme::accent_style().bg(theme::SURFACE),
            ),
            Span::styled(" ", theme::status_style()),
        ];

        if let Some(err) = &self.error {
            spans.push(Span::styled(
                format!("Error: {} ", err),
                ratatui::style::Style::default()
                    .fg(theme::RED)
                    .bg(theme::SURFACE),
            ));
        } else {
            spans.push(Span::styled(
                "j/k:nav \u{2190}\u{2192}/tab:switch enter:open 1-6:feed o:browser p:read r:refresh ?:help q:quit ",
                theme::status_style(),
            ));
        }

        // Right-aligned position indicator
        let right_text = format!(" {} [{}] ", self.position, self.focus_pane);
        let right_start = area.right().saturating_sub(right_text.len() as u16);

        let line = Line::from(spans);
        buf.set_line(area.left(), area.top(), &line, area.width);

        let right_span = Span::styled(right_text, theme::accent_style().bg(theme::SURFACE));
        buf.set_span(right_start, area.top(), &right_span, area.width);
    }
}
