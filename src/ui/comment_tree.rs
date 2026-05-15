//! Comment tree widget — renders the right-hand pane.
//!
//! Two-pass rendering: `measure_comments` produces per-comment line lists
//! (header + wrapped text + gap), then the widget scrolls to keep the
//! selected comment in view and draws from the computed offset. Also
//! populates `CommentTreeState::row_map` so mouse clicks can map rows
//! back to comment indices.

use crate::api::types::{CommentId, Item};
use crate::sanitize::sanitize_terminal;
use crate::state::comment_state::{CommentFilter, CommentTreeState, FlatComment};
use crate::ui::spinner;
use crate::ui::story_list::format_time_ago_since;
use crate::ui::theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};

/// Builds the block-title label for the comments pane.
///
/// `story.display_title()` is attacker-controlled (the HN Firebase API
/// returns titles as plain strings, not HTML, so they bypass
/// `html2text`'s entity-decode path). Running it through
/// [`sanitize_terminal`] before interpolation prevents a story title
/// containing `\x1b]0;OWNED\x07` from rewriting the user's terminal
/// tab title or otherwise injecting escape sequences when the comments
/// pane is opened.
fn build_block_title_label(story: &Item) -> String {
    let safe_title = sanitize_terminal(story.display_title());
    if let Some(badge) = story.badge() {
        format!(" [{}] {} ", badge.label(), safe_title)
    } else {
        format!(" {} ", safe_title)
    }
}

/// Stateless widget that renders the right pane. Takes a mutable reference
/// to [`CommentTreeState`] because rendering populates the `row_map` and
/// may advance `scroll` to keep the selected comment visible.
///
/// `prior_count` is an optional informational badge shown in the title when
/// non-zero — the number of prior HN submissions of the loaded story's URL
/// that the [`crate::ui::prior_overlay`] overlay (bound to `h`) will surface.
pub struct CommentTree<'a> {
    /// Mutable handle to the comment-pane state — rendering populates
    /// `row_map` and may advance `scroll` to keep the selection in view.
    pub state: &'a mut CommentTreeState,
    /// Pre-walked visible-comment indices (from
    /// [`CommentTreeState::visible_indices`]); shared with the status
    /// bar so the two stay in sync within a single frame.
    pub visible: &'a [usize],
    /// True when the comments pane has keyboard focus — drives the
    /// border accent.
    pub focused: bool,
    /// Monotonic tick counter; indexes into the loading-spinner glyph
    /// sequence.
    pub tick: u64,
    /// Prior-discussions count for the loaded story; rendered as the
    /// `· N prior (h)` title suffix when non-zero.
    pub prior_count: usize,
    /// Wall-clock timestamp captured once per frame in [`crate::ui::render`]
    /// so the `Ns/Nm/Nh` ago column doesn't `clock_gettime` per visible
    /// comment.
    pub now_secs: i64,
}

/// A pre-measured comment: all the lines it will produce and its visual index.
struct MeasuredComment {
    visual_index: usize,
    lines: Vec<CommentLine>,
}

/// One rendered line in the measure pass. `Header` is the author/time
/// line, `Text` is a single wrapped body line, `Gap` is a blank row
/// between comments.
///
/// Spans are built once in the measure pass and consumed (moved) into
/// [`Line`]s in the render pass — no cloning between the two phases.
enum CommentLine {
    Header(Vec<Span<'static>>),
    Text(Vec<Span<'static>>),
    Gap,
}

impl<'a> Widget for CommentTree<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            theme::accent_style()
        } else {
            theme::dim_style()
        };

        let title_span = if let Some(story) = &self.state.story {
            Span::styled(build_block_title_label(story), theme::title_style())
        } else {
            Span::styled(" Comments ", theme::title_style())
        };

        let mut title_spans = vec![title_span];
        if self.prior_count > 0 {
            title_spans.push(Span::styled(
                format!("· {} prior (h) ", self.prior_count),
                theme::dim_style(),
            ));
        }
        match self.state.filter {
            CommentFilter::All => {}
            CommentFilter::NewSince(_) => title_spans.push(Span::styled(
                "· New since last visit (n) ",
                theme::accent_style(),
            )),
            CommentFilter::Recent(_) => {
                title_spans.push(Span::styled("· Recent 24h (n) ", theme::accent_style()));
            }
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Line::from(title_spans))
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

        // Render story meta header (fixed, not scrolled).
        // Snapshot the meta-line scalars first so the immutable borrow on
        // self.state.story can be released before story_plain_text mutably
        // borrows self.state.story_text_cache. This lets the body lines flow
        // straight from the cache as `&str`, eliminating the per-frame body
        // text clone the old two-step `.map(str::to_owned)` +
        // `.clone().unwrap_or_default()` dance needed.
        let mut header_height: u16 = 0;

        let meta_snapshot = self.state.story.as_ref().map(|s| {
            let has_text = s.text.is_some();
            // Sanitize HN-supplied fields before they enter the rendered
            // line — a malicious URL or username could otherwise embed
            // terminal escapes via entity-decoded HTML upstream.
            let by = crate::sanitize::sanitize_terminal(s.by.as_deref().unwrap_or("?"));
            let line = if has_text {
                format!(
                    "  {} pts | {} | {} comments",
                    s.score.unwrap_or(0),
                    by,
                    s.descendants.unwrap_or(0),
                )
            } else {
                let url = crate::sanitize::sanitize_terminal(s.url.as_deref().unwrap_or(""));
                format!(
                    "  {} pts | {} | {} comments | {}",
                    s.score.unwrap_or(0),
                    by,
                    s.descendants.unwrap_or(0),
                    url,
                )
            };
            (has_text, line)
        });

        let story_plain: Option<&str> = if meta_snapshot.as_ref().is_some_and(|(t, _)| *t) {
            self.state.story_plain_text(inner.width as usize)
        } else {
            None
        };

        if let Some((has_text, meta_line)) = meta_snapshot {
            buf.set_line(
                inner.left(),
                inner.top() + header_height,
                &Line::from(Span::styled(meta_line, theme::meta_style())),
                inner.width,
            );
            header_height += 1;

            if has_text {
                if let Some(plain) = story_plain {
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
                }
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
            self.now_secs,
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

        // Pass 2: consume measured and move spans into Lines — no cloning.
        let mut screen_y = header_height;
        let mut row_idx: usize = 0;

        for mc in measured {
            let visual_index = mc.visual_index;
            let is_selected = visual_index == self.state.selected;
            let bg = if is_selected {
                theme::SURFACE
            } else {
                theme::BG
            };

            for line in mc.lines {
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
                    self.state.row_map[screen_y as usize] = Some(visual_index);
                }

                match line {
                    CommentLine::Header(spans) | CommentLine::Text(spans) => {
                        buf.set_line(
                            inner.left(),
                            inner.top() + screen_y,
                            &Line::from(spans),
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
    collapsed: &std::collections::HashSet<CommentId>,
    all_comments: &mut [FlatComment],
    width: usize,
    pending_root_ids: &std::collections::HashSet<CommentId>,
    spinner_frame: &str,
    now_secs: i64,
) -> Vec<MeasuredComment> {
    let mut result = Vec::new();

    for (vi, &idx) in visible_indices.iter().enumerate() {
        let mut lines = Vec::new();
        let depth = all_comments[idx].depth;
        // `indent_bar_for` returns "<spaces>│ " — see INDENT_BARS — and
        // its char count drives the body-wrap width below.
        let text_width = width.saturating_sub(indent_bar_for(depth).chars().count() + 2);
        let is_collapsed = collapsed.contains(&CommentId(all_comments[idx].item.id));

        // Populate/refresh the plain-text cache for this comment under a
        // short-lived mutable borrow, then read everything else immutably.
        let plain_text = if is_collapsed {
            None
        } else {
            all_comments[idx].plain_text(text_width).map(str::to_owned)
        };

        let comment = &all_comments[idx];
        let depth_color = theme::depth_color(comment.depth);

        let author_sanitized =
            crate::sanitize::sanitize_terminal(comment.item.by.as_deref().unwrap_or("[deleted]"));
        let author: &str = author_sanitized.as_ref();
        let time_ago = comment
            .item
            .time
            .map(|t| format_time_ago_since(t, now_secs))
            .unwrap_or_default();
        let collapse_indicator = if is_collapsed { " [+]" } else { "" };

        // Only allocate when there's actually a hidden-children badge to
        // show — for the common "not collapsed" path this stays None and
        // skips a per-comment-per-frame `String::new()` plus its Span.
        let child_count: Option<String> = if is_collapsed {
            let count = count_hidden_children(all_comments, idx);
            (count > 0).then(|| format!(" ({} hidden)", count))
        } else {
            None
        };

        // Header line — build Spans directly. Each Span owns its String
        // via Cow::Owned, so it lives to be moved into a Line in the
        // render pass without further cloning.
        let mut header_spans: Vec<Span<'static>> = vec![Span::styled(
            indent_bar_for(depth),
            ratatui::style::Style::default().fg(depth_color),
        )];
        if comment.depth == 0 && pending_root_ids.contains(&CommentId(comment.item.id)) {
            header_spans.push(Span::styled(
                format!("{} ", spinner_frame),
                ratatui::style::Style::default().fg(theme::HN_ORANGE),
            ));
        }
        header_spans.extend([
            Span::styled(
                format!("{} ", author),
                ratatui::style::Style::default()
                    .fg(depth_color)
                    .add_modifier(ratatui::style::Modifier::BOLD),
            ),
            Span::styled(
                format!("{} ago", time_ago),
                ratatui::style::Style::default().fg(theme::DIM),
            ),
            Span::styled(
                collapse_indicator.to_string(),
                ratatui::style::Style::default().fg(theme::YELLOW),
            ),
        ]);
        if let Some(text) = child_count {
            header_spans.push(Span::styled(
                text,
                ratatui::style::Style::default().fg(theme::DIM),
            ));
        }
        lines.push(CommentLine::Header(header_spans));

        // Comment text lines
        if let Some(plain) = &plain_text {
            for line in plain.lines().take(20) {
                let text_spans: Vec<Span<'static>> = vec![
                    Span::styled(
                        indent_bar_for(depth),
                        ratatui::style::Style::default().fg(depth_color),
                    ),
                    Span::styled(
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

/// Returns the precomputed descendant count for `all[parent_idx]` — the
/// contiguous run of comments with strictly greater depth that follow it
/// in pre-order. Callers paint this as the "(N hidden)" annotation on a
/// collapsed comment, hence the legacy name; the count itself does not
/// depend on the collapse set.
///
/// O(1): reads the precomputed `descendant_count` field maintained by
/// [`crate::state::comment_state::CommentTreeState::recompute_descendant_counts`].
/// Pre-fix this re-scanned the comments tail per call, which was
/// effectively O(n²) on a thread with many collapsed deep subtrees
/// rendered every frame (closes #138).
fn count_hidden_children(all: &[FlatComment], parent_idx: usize) -> usize {
    all[parent_idx].descendant_count
}

/// Precomputed "indent + thread-bar" strings for depths 0..=MAX_COMMENT_DEPTH.
/// Used by `measure_comments` for every header row and every wrapped body
/// row, so caching the concatenation here turns the per-comment-per-frame
/// `format!("{}{}", indent, bar)` allocations into `&'static str` Span
/// content (zero allocation — `Span::styled(&'static str, ..)` produces a
/// `Cow::Borrowed`) (closes #136).
const INDENT_BARS: [&str; 11] = [
    "│ ",
    "  │ ",
    "    │ ",
    "      │ ",
    "        │ ",
    "          │ ",
    "            │ ",
    "              │ ",
    "                │ ",
    "                  │ ",
    "                    │ ",
];

/// Returns the precomputed indent-plus-thread-bar string for `depth`,
/// clamping at `INDENT_BARS.len() - 1` so deeper subtrees keep the same
/// indent rather than allocating per-depth. See [`INDENT_BARS`].
fn indent_bar_for(depth: usize) -> &'static str {
    INDENT_BARS[depth.min(INDENT_BARS.len() - 1)]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ItemType;

    fn make_story(title: Option<&str>, item_type: Option<ItemType>) -> Item {
        Item {
            id: 1,
            title: title.map(String::from),
            url: None,
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type,
            dead: None,
            deleted: None,
        }
    }

    // --- build_block_title_label ---

    #[test]
    fn block_title_neutralises_osc_window_title_escape() {
        // OSC-0 ("set window title") — the primary attack vector for
        // C2. Make sure neither ESC nor BEL survive the label build.
        let story = make_story(Some("Cool tool\x1b]0;OWNED\x07"), None);
        let label = build_block_title_label(&story);
        assert!(!label.contains('\x1b'), "ESC must not appear: {label:?}");
        assert!(!label.contains('\x07'), "BEL must not appear: {label:?}");
        assert!(label.contains("Cool tool"));
    }

    #[test]
    fn block_title_neutralises_csi_clear_screen() {
        let story = make_story(Some("Title\x1b[2Jcleared"), None);
        let label = build_block_title_label(&story);
        assert!(!label.contains('\x1b'));
        assert!(label.contains("Title"));
        assert!(label.contains("cleared"));
    }

    #[test]
    fn block_title_neutralises_c1_range_bytes() {
        // 0x9B is the 8-bit CSI introducer; rendered through a Latin-1
        // terminal it kicks off a control sequence on its own.
        let story = make_story(Some("a\u{009b}b"), None);
        let label = build_block_title_label(&story);
        assert!(!label.contains('\u{009b}'));
        assert!(label.contains('a'));
        assert!(label.contains('b'));
    }

    #[test]
    fn block_title_preserves_plain_title() {
        let story = make_story(Some("My cool tool"), None);
        let label = build_block_title_label(&story);
        assert_eq!(label, " My cool tool ");
    }

    #[test]
    fn block_title_preserves_unicode_letters() {
        let story = make_story(Some("café résumé 日本語"), None);
        let label = build_block_title_label(&story);
        assert!(label.contains("café"));
        assert!(label.contains("日本語"));
    }

    #[test]
    fn block_title_includes_badge_for_ask_hn() {
        let story = make_story(Some("Ask HN: How do I X?"), None);
        let label = build_block_title_label(&story);
        // `display_title` strips the "Ask HN:" prefix; the label
        // re-introduces the badge from `Item::badge`.
        assert!(
            label.contains("[Ask HN]"),
            "expected badge in label: {label:?}"
        );
        assert!(label.contains("How do I X?"));
        assert!(!label.contains("Ask HN: How do I X?"));
    }

    #[test]
    fn block_title_includes_badge_for_job_item_type() {
        let story = make_story(Some("We are hiring"), Some(ItemType::Job));
        let label = build_block_title_label(&story);
        assert!(label.contains("[Job]"));
        assert!(label.contains("We are hiring"));
    }

    #[test]
    fn block_title_no_badge_for_plain_story() {
        let story = make_story(Some("Just a regular story"), None);
        let label = build_block_title_label(&story);
        assert!(
            !label.contains('['),
            "plain story should not carry a badge: {label:?}"
        );
    }

    #[test]
    fn block_title_handles_missing_title() {
        // Item with `title = None` falls back to "[no title]" via
        // `display_title`. Make sure the label still builds and the
        // sanitiser doesn't choke on the literal brackets.
        let story = make_story(None, None);
        let label = build_block_title_label(&story);
        assert!(label.contains("[no title]"));
    }
}
