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
    pub search_query: Option<&'a str>,
}

impl<'a> Widget for StoryList<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            theme::accent_style()
        } else {
            theme::dim_style()
        };

        let title = if let Some(q) = &self.search_query {
            format!(" Search: {} ", q)
        } else {
            " Stories ".to_string()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(title, theme::title_style()))
            .style(theme::base_style());

        let inner = block.inner(area);
        block.render(area, buf);

        if self.loading && self.stories.is_empty() {
            let msg = if self.search_query.is_some() {
                "  Searching..."
            } else {
                "  Loading stories..."
            };
            let loading_line = Line::from(Span::styled(msg, theme::dim_style()));
            buf.set_line(inner.left(), inner.top(), &loading_line, inner.width);
            return;
        }

        if self.stories.is_empty() {
            let msg = if self.search_query.is_some() {
                "  No results found"
            } else {
                "  No stories loaded"
            };
            let empty_line = Line::from(Span::styled(msg, theme::dim_style()));
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

        for (i, story) in self
            .stories
            .iter()
            .enumerate()
            .skip(scroll)
            .take(visible_height)
        {
            let y = inner.top() + (i - scroll) as u16;
            let is_selected = i == self.selected;

            let title = story.display_title();
            let badge = story.badge();
            let domain = story
                .domain()
                .map(|d| format!(" ({})", d))
                .unwrap_or_default();

            let num = format!("{:>3}. ", i + 1);
            let badge_text = badge.map(|b| format!("[{}] ", b.label()));
            let badge_width = badge_text.as_ref().map_or(0, |t| t.len());
            let max_title_width =
                (inner.width as usize).saturating_sub(num.len() + badge_width + domain.len() + 2);
            let truncated_title: String = if title.chars().count() > max_title_width {
                let truncated: String = title
                    .chars()
                    .take(max_title_width.saturating_sub(3))
                    .collect();
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

            let mut spans = vec![Span::styled(
                num,
                if is_selected {
                    theme::accent_style().bg(theme::SURFACE)
                } else {
                    theme::dim_style()
                },
            )];
            if let Some((text, b)) = badge_text.zip(badge) {
                spans.push(Span::styled(text, theme::badge_style(b)));
            }
            spans.push(Span::styled(truncated_title, style));
            spans.push(Span::styled(
                domain,
                theme::dim_style().bg(if is_selected {
                    theme::SURFACE
                } else {
                    theme::BG
                }),
            ));

            let line = Line::from(spans);
            buf.set_line(inner.left(), y, &line, inner.width);
        }
    }
}

pub fn format_time_ago(timestamp: i64) -> String {
    format_time_ago_since(timestamp, chrono::Utc::now().timestamp())
}

fn format_time_ago_since(timestamp: i64, now: i64) -> String {
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

#[cfg(test)]
mod tests {
    use super::format_time_ago_since;

    #[test]
    fn zero_seconds_is_0s() {
        assert_eq!(format_time_ago_since(1_000, 1_000), "0s");
    }

    #[test]
    fn fifty_nine_seconds_is_seconds() {
        assert_eq!(format_time_ago_since(0, 59), "59s");
    }

    #[test]
    fn sixty_seconds_rolls_over_to_minutes() {
        assert_eq!(format_time_ago_since(0, 60), "1m");
    }

    #[test]
    fn just_under_an_hour_is_minutes() {
        assert_eq!(format_time_ago_since(0, 3599), "59m");
    }

    #[test]
    fn one_hour_rolls_over_to_hours() {
        assert_eq!(format_time_ago_since(0, 3600), "1h");
    }

    #[test]
    fn just_under_a_day_is_hours() {
        assert_eq!(format_time_ago_since(0, 86_399), "23h");
    }

    #[test]
    fn one_day_rolls_over_to_days() {
        assert_eq!(format_time_ago_since(0, 86_400), "1d");
    }

    #[test]
    fn large_diff_counts_days() {
        // ~30 days
        assert_eq!(format_time_ago_since(0, 86_400 * 30), "30d");
    }
}
