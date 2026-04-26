//! Prior-discussions overlay rendering.
//!
//! [`render_prior_overlay`] draws a full-screen list of prior HN
//! submissions of the currently-selected story's URL. Follows the same
//! visual pattern as [`crate::ui::article_reader`] for consistency.

use crate::api::types::Item;
use crate::state::prior_state::PriorDiscussionsState;
use crate::ui::theme;
use crate::ui::util::truncate_to;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
    Frame,
};

/// Draws the prior-discussions overlay for `state` into `area`.
///
/// Leaves a small margin and no-ops if the available space is too small.
pub fn render_prior_overlay(frame: &mut Frame, area: Rect, state: &PriorDiscussionsState) {
    let margin = 2u16;
    let x = margin.min(area.width / 2);
    let y = 1u16.min(area.height / 2);
    let width = area.width.saturating_sub(x * 2);
    let height = area.height.saturating_sub(y * 2);

    if width < 20 || height < 5 {
        return;
    }

    let overlay_area = Rect::new(x, y, width, height);
    frame.render_widget(Clear, overlay_area);

    let title = format!(
        " Prior Discussions ({}) ",
        match state.submissions.len() {
            1 => "1 prior submission".to_string(),
            n => format!("{} prior submissions", n),
        }
    );

    let footer = Line::from(Span::styled(
        " j/k:navigate  Enter:load comments  o:browser  Esc:close ",
        theme::dim_style(),
    ))
    .centered();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent_style())
        .title(Span::styled(title, theme::title_style()))
        .title_bottom(footer)
        .style(theme::base_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.submissions.is_empty() {
        let empty = Paragraph::new(Line::from(Span::styled(
            "No prior HN submissions found for this URL.",
            theme::dim_style(),
        )))
        .alignment(ratatui::layout::Alignment::Center);
        let y_offset = inner.height / 2;
        if y_offset > 0 && inner.height > 0 {
            let centered = Rect::new(inner.x, inner.y + y_offset, inner.width, 1);
            frame.render_widget(empty, centered);
        }
        return;
    }

    // Each submission takes two rows (header + byline) + a blank gap, so
    // we can render up to `inner.height / 3` comfortably.
    let rows_per_item = 3u16;
    let visible_count = (inner.height / rows_per_item) as usize;
    if visible_count == 0 {
        return;
    }

    // Scroll so the selected row is always in view.
    let scroll = if state.selected >= visible_count {
        state.selected - visible_count + 1
    } else {
        0
    };

    let mut lines: Vec<Line> = Vec::with_capacity(visible_count * 3);
    for (i, item) in state
        .submissions
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_count)
    {
        let is_selected = i == state.selected;
        lines.extend(format_submission(item, is_selected, inner.width as usize));
        lines.push(Line::from(""));
    }

    let list = Paragraph::new(lines);
    frame.render_widget(list, inner);
}

/// Formats one submission as two styled lines (main + byline).
fn format_submission(item: &Item, selected: bool, width: usize) -> [Line<'static>; 2] {
    let points = item.score.unwrap_or(0);
    let comments = item.descendants.unwrap_or(0);
    let date = item
        .time
        .and_then(|t| chrono::DateTime::<chrono::Utc>::from_timestamp(t, 0))
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".into());
    // Sanitize HN-supplied strings before they enter a Span — a comment
    // submitter could otherwise inject ANSI escapes that retarget the
    // terminal's title/palette/scroll region.
    let title_sanitized = crate::sanitize::sanitize_terminal(item.display_title());
    let title: &str = title_sanitized.as_ref();
    let author_sanitized =
        crate::sanitize::sanitize_terminal(item.by.as_deref().unwrap_or("[deleted]"));
    let author: &str = author_sanitized.as_ref();

    let cursor = if selected { "> " } else { "  " };
    let title_style = if selected {
        theme::title_style().bg(theme::SURFACE)
    } else {
        ratatui::style::Style::default().fg(theme::TEXT)
    };
    let meta_style = if selected {
        theme::accent_style().bg(theme::SURFACE)
    } else {
        theme::accent_style()
    };
    let dim_style = if selected {
        theme::dim_style().bg(theme::SURFACE)
    } else {
        theme::dim_style()
    };
    let row_bg = if selected {
        theme::selected_style()
    } else {
        theme::base_style()
    };

    // First line: `>  847 pts  2023-04-02  Title here`
    let prefix = format!("{}{:>4} pts  {}  ", cursor, points, date);
    let title_truncated = truncate_to(title, width.saturating_sub(prefix.chars().count() + 2));

    let line1 = Line::from(vec![
        Span::styled(format!("{}{:>4} pts  ", cursor, points), meta_style),
        Span::styled(format!("{}  ", date), dim_style),
        Span::styled(title_truncated, title_style),
    ])
    .style(row_bg);

    // Second line: `       ({N} comments) by {author}`
    let byline = format!(
        "     ({} comment{}) by {}",
        comments,
        if comments == 1 { "" } else { "s" },
        author
    );
    let line2 = Line::from(Span::styled(byline, dim_style)).style(row_bg);

    [line1, line2]
}
