//! Comment tree widget — renders the right-hand pane.
//!
//! Two-pass rendering: `measure_comments` produces per-comment line lists
//! (header + wrapped text + gap), then the widget scrolls to keep the
//! selected comment in view and draws from the computed offset. Also
//! populates `CommentTreeState::row_map` so mouse clicks can map rows
//! back to comment indices.

use crate::state::comment_state::{CommentTreeState, FlatComment};
use crate::ui::spinner;
use crate::ui::story_list::format_time_ago;
use crate::ui::theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

/// Stateless widget that renders the right pane. Takes a mutable reference
/// to [`CommentTreeState`] because rendering populates the `row_map` and
/// may advance `scroll` to keep the selected comment visible.
pub struct CommentTree<'a> {
    pub state: &'a mut CommentTreeState,
    pub visible: &'a [usize],
    pub focused: bool,
    pub tick: u64,
}

/// A pre-measured comment: all the lines it will produce and its visual index.
struct MeasuredComment {
    visual_index: usize,
    lines: Vec<CommentLine>,
}

/// One rendered line in the measure pass. `Header` is the author/time
/// line, `Text` is a single wrapped body line, `Gap` is a blank row
/// between comments.
enum CommentLine {
    Header(Vec<(String, ratatui::style::Style)>),
    Text(Vec<(String, ratatui::style::Style)>),
    Gap,
}

impl<'a> Widget for CommentTree<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            theme::accent_style()
        } else {
            theme::dim_style()
        };

        let title = if let Some(story) = &self.state.story {
            if let Some(badge) = story.badge() {
                format!(" [{}] {} ", badge.label(), story.display_title())
            } else {
                format!(" {} ", story.display_title())
            }
        } else {
            " Comments ".to_string()
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Span::styled(title, theme::title_style()))
            .style(theme::base_style());

        let inner = block.inner(area);
        block.render(area, buf);

        let spinner_frame = spinner::frame(self.tick);

        if self.state.loading && self.state.story.is_none() {
            let loading = Line::from(vec![
                Span::styled(format!("  {} ", spinner_frame), theme::accent_style()),
                Span::styled("Loading comments...", theme::dim_style()),
            ]);
            buf.set_line(inner.left(), inner.top(), &loading, inner.width);
            return;
        }

        if self.state.story.is_none() {
            let hint = Line::from(Span::styled(
                "  Press Enter on a story to load comments",
                theme::dim_style(),
            ));
            buf.set_line(inner.left(), inner.top(), &hint, inner.width);
            return;
        }

        let visible = self.visible;

        if visible.is_empty() && !self.state.loading {
            let no_comments = Line::from(Span::styled("  No comments", theme::dim_style()));
            buf.set_line(inner.left(), inner.top(), &no_comments, inner.width);
            return;
        }

        // Render story meta header (fixed, not scrolled)
        let mut header_height: u16 = 0;

        let story_plain = self
            .state
            .story_plain_text(inner.width as usize)
            .map(str::to_owned);

        if let Some(story) = &self.state.story {
            if let Some(_text) = &story.text {
                let plain = story_plain.clone().unwrap_or_default();
                let meta = format!(
                    "  {} pts | {} | {} comments",
                    story.score.unwrap_or(0),
                    story.by.as_deref().unwrap_or("?"),
                    story.descendants.unwrap_or(0),
                );
                buf.set_line(
                    inner.left(),
                    inner.top() + header_height,
                    &Line::from(Span::styled(meta, theme::meta_style())),
                    inner.width,
                );
                header_height += 1;

                for line in plain.lines().take((inner.height / 4) as usize) {
                    if header_height >= inner.height {
                        break;
                    }
                    buf.set_line(
                        inner.left(),
                        inner.top() + header_height,
                        &Line::from(Span::styled(format!("  {}", line), theme::base_style())),
                        inner.width,
                    );
                    header_height += 1;
                }

                if header_height < inner.height {
                    buf.set_line(
                        inner.left(),
                        inner.top() + header_height,
                        &Line::from(Span::styled(
                            "  ────────────────────────────────────────",
                            theme::dim_style(),
                        )),
                        inner.width,
                    );
                    header_height += 1;
                }
            } else {
                let meta = format!(
                    "  {} pts | {} | {} comments | {}",
                    story.score.unwrap_or(0),
                    story.by.as_deref().unwrap_or("?"),
                    story.descendants.unwrap_or(0),
                    story.url.as_deref().unwrap_or(""),
                );
                buf.set_line(
                    inner.left(),
                    inner.top() + header_height,
                    &Line::from(Span::styled(meta, theme::meta_style())),
                    inner.width,
                );
                header_height += 1;

                if header_height < inner.height {
                    buf.set_line(
                        inner.left(),
                        inner.top() + header_height,
                        &Line::from(Span::styled(
                            "  ────────────────────────────────────────",
                            theme::dim_style(),
                        )),
                        inner.width,
                    );
                    header_height += 1;
                }
            }
        }

        if self.state.loading && visible.is_empty() {
            if header_height < inner.height {
                buf.set_line(
                    inner.left(),
                    inner.top() + header_height,
                    &Line::from(vec![
                        Span::styled(format!("  {} ", spinner_frame), theme::accent_style()),
                        Span::styled("Loading comments...", theme::dim_style()),
                    ]),
                    inner.width,
                );
            }
            return;
        }

        let available_height = inner.height.saturating_sub(header_height) as usize;
        if available_height == 0 {
            return;
        }

        // Pass 1: measure all comments into rows.
        let measured = measure_comments(
            visible,
            &self.state.collapsed,
            &mut self.state.comments,
            inner.width as usize,
            &self.state.pending_root_ids,
            spinner_frame,
        );

        // Initialize row_map for mouse click handling
        self.state.row_map.clear();
        self.state.row_map.resize(inner.height as usize, None);

        // Find the row offset where the selected comment starts
        let mut selected_row_start: usize = 0;
        let mut selected_row_end: usize = 0;
        let mut total_rows: usize = 0;

        for mc in &measured {
            let line_count = mc.lines.len();
            if mc.visual_index == self.state.selected {
                selected_row_start = total_rows;
                selected_row_end = total_rows + line_count;
            }
            total_rows += line_count;
        }

        // Scroll so selected comment is visible — prefer selected at top
        let current_scroll = self.state.scroll;
        let scroll_row = if selected_row_start < current_scroll
            || selected_row_end > current_scroll + available_height
        {
            selected_row_start
        } else {
            current_scroll
        };
        self.state.scroll = scroll_row;

        // Pass 2: render from scroll_row
        let mut screen_y = header_height;
        let mut row_idx: usize = 0;

        for mc in &measured {
            let is_selected = mc.visual_index == self.state.selected;
            let bg = if is_selected {
                theme::SURFACE
            } else {
                theme::BG
            };

            for line in &mc.lines {
                if row_idx < scroll_row {
                    row_idx += 1;
                    continue;
                }
                if screen_y >= inner.height {
                    return;
                }

                // Fill background for selected comment (skip trailing gap so the
                // highlight aligns with the left `│` bar)
                if is_selected && !matches!(line, CommentLine::Gap) {
                    for x in inner.left()..inner.right() {
                        buf[(x, inner.top() + screen_y)]
                            .set_style(ratatui::style::Style::default().bg(bg));
                    }
                }

                // Record row → comment mapping for mouse clicks
                if (screen_y as usize) < self.state.row_map.len() {
                    self.state.row_map[screen_y as usize] = Some(mc.visual_index);
                }

                match line {
                    CommentLine::Header(spans) | CommentLine::Text(spans) => {
                        let ratatui_spans: Vec<Span> = spans
                            .iter()
                            .map(|(text, style)| Span::styled(text.clone(), *style))
                            .collect();
                        buf.set_line(
                            inner.left(),
                            inner.top() + screen_y,
                            &Line::from(ratatui_spans),
                            inner.width,
                        );
                    }
                    CommentLine::Gap => {
                        // empty line, background already handled
                    }
                }

                screen_y += 1;
                row_idx += 1;
            }
        }

        // Show loading indicator at the bottom if still loading children
        if self.state.loading && screen_y < inner.height {
            buf.set_line(
                inner.left(),
                inner.top() + screen_y,
                &Line::from(vec![
                    Span::styled(format!("  {} ", spinner_frame), theme::accent_style()),
                    Span::styled("Loading more comments...", theme::dim_style()),
                ]),
                inner.width,
            );
        }
    }
}

/// Pre-renders each visible comment to a [`MeasuredComment`] (header line,
/// wrapped body lines capped at 20, trailing gap). Collapsed comments omit
/// body lines and render a `[+] (N hidden)` suffix. Root comments still
/// fetching children get a spinner glyph.
fn measure_comments(
    visible_indices: &[usize],
    collapsed: &std::collections::HashSet<u64>,
    all_comments: &mut [FlatComment],
    width: usize,
    pending_root_ids: &std::collections::HashSet<u64>,
    spinner_frame: &str,
) -> Vec<MeasuredComment> {
    let mut result = Vec::new();

    for (vi, &idx) in visible_indices.iter().enumerate() {
        let mut lines = Vec::new();
        let depth = all_comments[idx].depth;
        let indent = "  ".repeat(depth);
        let bar = "│ ";
        let text_width = width.saturating_sub(indent.len() + bar.len() + 2);
        let is_collapsed = collapsed.contains(&all_comments[idx].item.id);

        // Populate/refresh the plain-text cache for this comment under a
        // short-lived mutable borrow, then read everything else immutably.
        let plain_text = if is_collapsed {
            None
        } else {
            all_comments[idx].plain_text(text_width).map(str::to_owned)
        };

        let comment = &all_comments[idx];
        let depth_color = theme::depth_color(comment.depth);

        let author = comment.item.by.as_deref().unwrap_or("[deleted]");
        let time_ago = comment.item.time.map(format_time_ago).unwrap_or_default();
        let collapse_indicator = if is_collapsed { " [+]" } else { "" };

        let child_count = if is_collapsed {
            let count = count_hidden_children(all_comments, comment);
            if count > 0 {
                format!(" ({} hidden)", count)
            } else {
                String::new()
            }
        } else {
            String::new()
        };

        // Header line
        let mut header_spans = vec![(
            format!("{}{}", indent, bar),
            ratatui::style::Style::default().fg(depth_color),
        )];
        if comment.depth == 0 && pending_root_ids.contains(&comment.item.id) {
            header_spans.push((
                format!("{} ", spinner_frame),
                ratatui::style::Style::default().fg(theme::HN_ORANGE),
            ));
        }
        header_spans.extend([
            (
                format!("{} ", author),
                ratatui::style::Style::default()
                    .fg(depth_color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            (
                format!("{} ago", time_ago),
                ratatui::style::Style::default().fg(theme::DIM),
            ),
            (
                collapse_indicator.to_string(),
                ratatui::style::Style::default().fg(theme::YELLOW),
            ),
            (child_count, ratatui::style::Style::default().fg(theme::DIM)),
        ]);
        lines.push(CommentLine::Header(header_spans));

        // Comment text lines
        if let Some(plain) = &plain_text {
            for line in plain.lines().take(20) {
                let text_spans = vec![
                    (
                        format!("{}{}", indent, bar),
                        ratatui::style::Style::default().fg(depth_color),
                    ),
                    (
                        line.to_string(),
                        ratatui::style::Style::default().fg(theme::TEXT),
                    ),
                ];
                lines.push(CommentLine::Text(text_spans));
            }
        }

        // Blank gap after every comment — prevents the next comment's vertical
        // bar from visually bleeding into this comment's last row.
        lines.push(CommentLine::Gap);

        result.push(MeasuredComment {
            visual_index: vi,
            lines,
        });
    }

    result
}

/// Counts descendants of `parent` in the flat list — the contiguous run
/// of comments with strictly greater depth that follow it.
fn count_hidden_children(all: &[FlatComment], parent: &FlatComment) -> usize {
    let parent_idx = all.iter().position(|c| c.item.id == parent.item.id);
    match parent_idx {
        Some(idx) => {
            let mut count = 0;
            for c in &all[idx + 1..] {
                if c.depth > parent.depth {
                    count += 1;
                } else {
                    break;
                }
            }
            count
        }
        None => 0,
    }
}
