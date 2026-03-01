use crate::api::types::Item;
use crate::ui::theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

pub struct StoryList<'a> {
    pub stories: &'a [Item],
    pub selected: usize,
    pub offset: usize,
    pub focused: bool,
    pub loading: bool,
}

impl<'a> Widget for StoryList<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            theme::accent_style()
        } else {
            theme::dim_style()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(" Stories ", theme::title_style()))
            .style(theme::base_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if self.loading && self.stories.is_empty() {
            let loading_line = Line::from(Span::styled(
                "  Loading stories...",
                theme::dim_style(),
            ));
            buf.set_line(inner.left(), inner.top(), &loading_line, inner.width);
            return;
        }

        if self.stories.is_empty() {
            let empty_line = Line::from(Span::styled(
                "  No stories loaded",
                theme::dim_style(),
            ));
            buf.set_line(inner.left(), inner.top(), &empty_line, inner.width);
            return;
        }

        let visible_height = inner.height as usize;

        // Calculate scroll offset to keep selected visible
        let scroll = if self.selected >= self.offset + visible_height {
            self.selected - visible_height + 1
        } else if self.selected < self.offset {
            self.selected
        } else {
            self.offset
        };

        for (i, story) in self.stories.iter().enumerate().skip(scroll).take(visible_height) {
            let y = inner.top() + (i - scroll) as u16;
            let is_selected = i == self.selected;

            let title = story.title.as_deref().unwrap_or("[no title]");
            let domain = story
                .domain()
                .map(|d| format!(" ({})", d))
                .unwrap_or_default();

            let num = format!("{:>3}. ", i + 1);
            let max_title_width = (inner.width as usize).saturating_sub(num.len() + domain.len() + 2);
            let truncated_title: String = if title.chars().count() > max_title_width {
                let truncated: String = title.chars().take(max_title_width.saturating_sub(3)).collect();
                format!("{}...", truncated)
            } else {
                title.to_string()
            };

            let style = if is_selected {
                theme::selected_style()
            } else {
                theme::base_style()
            };

            // Fill line background
            for x in inner.left()..inner.right() {
                buf[(x, y)].set_style(style);
            }

            let line = Line::from(vec![
                Span::styled(
                    num,
                    if is_selected {
                        theme::accent_style().bg(theme::SURFACE)
                    } else {
                        theme::dim_style()
                    },
                ),
                Span::styled(truncated_title, style),
                Span::styled(domain, theme::dim_style().bg(if is_selected { theme::SURFACE } else { theme::BG })),
            ]);
            buf.set_line(inner.left(), y, &line, inner.width);

            // Meta line (if space allows: every other row)
            if visible_height > self.stories.len() || i == self.selected {
                // We only show meta inline with compact display
            }

            // Show score/author/time on the same concept but we'll keep it compact
            // Actually let's use 2 lines per story if there's room, otherwise 1
        }
    }
}

pub fn format_time_ago(timestamp: i64) -> String {
    let now = chrono::Utc::now().timestamp();
    let diff = now - timestamp;

    if diff < 60 {
        format!("{}s", diff)
    } else if diff < 3600 {
        format!("{}m", diff / 60)
    } else if diff < 86400 {
        format!("{}h", diff / 3600)
    } else {
        format!("{}d", diff / 86400)
    }
}
