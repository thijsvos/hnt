//! Story-list widget for the left pane.
//!
//! [`StoryList`] renders numbered story rows with badge, title, and
//! domain, scrolling to keep the selected row in view. In the `Rising`
//! feed each row also carries a momentum column (sparkline, points
//! gained, front-page chip) from the Pulse store; in every other feed a
//! fast-moving story gets a `↗` glyph. Also exposes
//! [`format_time_ago_since`], used by the comment-tree widget for author
//! timestamps.

use crate::api::types::{Item, StoryId};
use crate::pulse::{self, Momentum, SPARKLINE_WIDTH};
use crate::sanitize::sanitize_terminal;
use crate::state::pin_store::PinStore;
use crate::state::pulse_store::PulseStore;
use crate::state::read_store::ReadStore;
use crate::ui::theme;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Widget},
};
use std::sync::Arc;

/// Minimum inner width at which the `Rising` momentum column shows the
/// front-page chip (`FP ~25m` / `on FP #9`) in addition to the sparkline
/// and points figure. The stories pane is 35 % of the terminal, so this
/// is roughly a 230-column terminal — below it the chip is dropped
/// rather than squeezing the title.
const WIDE_MOMENTUM_MIN_WIDTH: u16 = 80;
/// Minimum inner width for the sparkline + points column at all. Below
/// this the `Rising` feed falls back to the single `↗` glyph.
const MOMENTUM_MIN_WIDTH: u16 = 48;
/// Width of the `+NN` points figure (`+123` fits).
const VELOCITY_WIDTH: usize = 4;
/// Width of the front-page chip (`on FP #30` is the longest form).
const FRONT_PAGE_WIDTH: usize = 9;

/// Stateless widget that renders the left pane. Composed from borrowed
/// app state; rebuilt each frame.
pub struct StoryList<'a> {
    /// Loaded stories — borrowed for the frame's lifetime; rebuilt per
    /// draw. `Arc<Item>` matches the cache shape so no deep clone is
    /// needed to render.
    pub stories: &'a [Arc<Item>],
    /// Pre-computed `Item::domain()` results, parallel to `stories`.
    /// Avoids a per-frame `url::Url::parse` per visible row.
    pub domains: &'a [Option<String>],
    /// Index into `stories` of the highlighted row.
    pub selected: usize,
    /// Whether the pane currently has keyboard focus (drives border
    /// accent).
    pub focused: bool,
    /// True while the first batch is still in flight — paints
    /// "Loading stories..." instead of the empty-list placeholder.
    pub loading: bool,
    /// Active search query, when one is set — used in the pane title
    /// (`" Search: <q> "`) in place of `" Stories "`.
    pub search_query: Option<&'a str>,
    /// Persisted read-state: stories present here render dimmed, and those
    /// whose comment count has grown since the last visit get a `+N` badge.
    pub read_store: &'a ReadStore,
    /// Persisted pin-store: stories present here render with a leading
    /// `★` glyph in any feed.
    pub pin_store: &'a PinStore,
    /// Momentum store: drives the sparkline column in the `Rising` feed
    /// and the `↗` glyph elsewhere.
    pub pulse_store: &'a PulseStore,
    /// Whether the `Rising` feed is on screen — switches on the momentum
    /// column and the warming-up placeholder.
    pub rising: bool,
    /// Pane title override (e.g. `Rising · next sweep 43s`). Ignored while
    /// a search query is active; falls back to `Stories` when `None`.
    pub pane_title: Option<&'a str>,
    /// Wall-clock time (Unix seconds) captured once per frame — every
    /// momentum query is evaluated against it.
    pub now_secs: i64,
}

/// The momentum column layout chosen for a frame, from the pane width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MomentumLayout {
    /// `↗ ` only (2 columns) — also the non-Rising form.
    Glyph,
    /// Sparkline + points: `▂▃▅▆▇█ +42 ` (SPARKLINE_WIDTH + 1 + 4 + 1).
    Compact,
    /// Compact plus the front-page chip: `… FP ~25m ` (+ 9 + 1).
    Wide,
}

impl MomentumLayout {
    fn for_width(rising: bool, inner_width: u16) -> Self {
        if !rising || inner_width < MOMENTUM_MIN_WIDTH {
            Self::Glyph
        } else if inner_width < WIDE_MOMENTUM_MIN_WIDTH {
            Self::Compact
        } else {
            Self::Wide
        }
    }

    /// Columns the momentum column occupies, including trailing spaces.
    /// In the `Glyph` layout that's 2 only when the glyph is shown, so
    /// callers pass `shown`.
    fn width(self, shown: bool) -> usize {
        match self {
            Self::Glyph => {
                if shown {
                    2
                } else {
                    0
                }
            }
            Self::Compact => SPARKLINE_WIDTH + 1 + VELOCITY_WIDTH + 1,
            Self::Wide => SPARKLINE_WIDTH + 1 + VELOCITY_WIDTH + 1 + FRONT_PAGE_WIDTH + 1,
        }
    }
}

/// Builds the momentum spans for one row. Returns the spans and their
/// total visible width. Rows without live momentum in the `Rising` feed
/// get a blank column of the same width so titles stay aligned; in other
/// feeds they get nothing.
fn momentum_spans(
    momentum: Option<&Momentum>,
    layout: MomentumLayout,
    row_bg: ratatui::style::Color,
) -> (Vec<Span<'static>>, usize) {
    let spark_style = theme::momentum_style().bg(row_bg);
    let meta_style = theme::momentum_meta_style().bg(row_bg);
    match layout {
        MomentumLayout::Glyph => match momentum {
            Some(m) if m.velocity.is_hot() => (vec![Span::styled("\u{2197} ", spark_style)], 2),
            _ => (Vec::new(), 0),
        },
        MomentumLayout::Compact | MomentumLayout::Wide => {
            let width = layout.width(true);
            let Some(m) = momentum else {
                return (vec![Span::styled(" ".repeat(width), meta_style)], width);
            };
            let points = m.velocity.points.round() as i64;
            let velocity = if points >= 0 {
                format!("+{points}")
            } else {
                points.to_string()
            };
            let mut spans = vec![
                Span::styled(m.sparkline.clone(), spark_style),
                Span::styled(format!(" {velocity:>VELOCITY_WIDTH$} "), meta_style),
            ];
            if layout == MomentumLayout::Wide {
                let chip = pulse::format_front_page(m);
                spans.push(Span::styled(
                    format!("{chip:<FRONT_PAGE_WIDTH$} "),
                    meta_style,
                ));
            }
            (spans, width)
        }
    }
}

impl<'a> Widget for StoryList<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_style = if self.focused {
            theme::accent_style()
        } else {
            theme::dim_style()
        };

        let title_span = if let Some(q) = self.search_query {
            // Sanitise the echoed query — same defence-in-depth as the status
            // bar; a pasted query could carry C0/C1 bytes.
            Span::styled(
                format!(" Search: {} ", sanitize_terminal(q)),
                theme::title_style(),
            )
        } else if let Some(t) = self.pane_title {
            Span::styled(format!(" {t} "), theme::title_style())
        } else {
            Span::styled(" Stories ", theme::title_style())
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(title_span)
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
            } else if self.rising {
                "  Warming up \u{2014} momentum needs two sweeps (~2 min)"
            } else {
                "  No stories loaded"
            };
            let empty_line = Line::from(Span::styled(msg, theme::dim_style()));
            buf.set_line(inner.left(), inner.top(), &empty_line, inner.width);
            return;
        }

        let momentum_layout = MomentumLayout::for_width(self.rising, inner.width);
        let visible_height = inner.height as usize;

        // Calculate scroll offset to keep selected visible.
        let scroll = if self.selected >= visible_height {
            self.selected - visible_height + 1
        } else {
            0
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
            let sid = StoryId(story.id);
            let is_read = self.read_store.is_read(sid);
            let is_pinned = self.pin_store.is_pinned(sid);
            let new_comments = self
                .read_store
                .new_comments_since(sid, story.descendants.unwrap_or(0));

            // Sanitize untrusted HN-supplied strings before they reach a
            // ratatui Span — terminal escapes embedded in titles/domains
            // would otherwise be forwarded straight to crossterm.
            let title_sanitized = crate::sanitize::sanitize_terminal(story.display_title());
            let title: &str = title_sanitized.as_ref();
            let badge = story.badge();
            // Domain is pre-parsed by `StoryListState::replace_stories` /
            // `append_stories`; rendering only formats the cached value.
            let domain = self
                .domains
                .get(i)
                .and_then(|d| d.as_deref())
                .map(|d| format!(" ({})", crate::sanitize::sanitize_terminal(d)))
                .unwrap_or_default();
            let new_badge_text = new_comments.map(|n| format!(" +{}", n));

            let num = format!("{:>3}. ", i + 1);
            let badge_text = badge.map(|b| format!("[{}] ", b.label()));
            // Visible-column math: `chars().count()` not `.len()` so a
            // future non-ASCII badge or `+N` glyph doesn't misalign the
            // truncation boundary.
            let badge_width = badge_text.as_ref().map_or(0, |t| t.chars().count());
            let new_badge_width = new_badge_text.as_ref().map_or(0, |t| t.chars().count());
            // ★ + space = 2 visual columns. Reserved before the badge so a
            // pinned Ask HN story stays aligned: "  1. ★ [Ask HN] Title".
            let pin_width = if is_pinned { 2 } else { 0 };

            let row_bg = if is_selected {
                theme::SURFACE
            } else {
                theme::BG
            };

            // Momentum column (Rising) or `↗` glyph (elsewhere). Every
            // sparkline glyph is one column wide, so `chars().count()`
            // stays valid for the width reservation.
            let momentum = self.pulse_store.momentum_for(sid, self.now_secs);
            let (momentum_spans, momentum_width) =
                momentum_spans(momentum.as_ref(), momentum_layout, row_bg);

            let max_title_width = (inner.width as usize).saturating_sub(
                num.chars().count()
                    + pin_width
                    + momentum_width
                    + badge_width
                    + new_badge_width
                    + domain.chars().count()
                    + 2,
            );
            let truncated_title = crate::ui::util::truncate_to(title, max_title_width);
            let row_style = if is_selected {
                theme::selected_style()
            } else {
                theme::base_style()
            };
            // Visited stories use the dim foreground so they recede visually,
            // while still keeping the selection highlight (bold + surface bg)
            // so the cursor row remains obvious.
            let title_style = match (is_selected, is_read) {
                (true, true) => theme::dim_style()
                    .bg(theme::SURFACE)
                    .add_modifier(ratatui::style::Modifier::BOLD),
                (true, false) => theme::selected_style(),
                (false, true) => theme::dim_style(),
                (false, false) => theme::base_style(),
            };

            // Fill line background
            for x in inner.left()..inner.right() {
                buf[(x, y)].set_style(row_style);
            }

            let mut spans = vec![Span::styled(
                num,
                if is_selected {
                    theme::accent_style().bg(theme::SURFACE)
                } else {
                    theme::dim_style()
                },
            )];
            if is_pinned {
                spans.push(Span::styled(
                    "\u{2605} ",
                    if is_selected {
                        theme::pinned_style().bg(theme::SURFACE)
                    } else {
                        theme::pinned_style()
                    },
                ));
            }
            spans.extend(momentum_spans);
            if let Some((text, b)) = badge_text.zip(badge) {
                spans.push(Span::styled(text, theme::badge_style(b)));
            }
            spans.push(Span::styled(truncated_title, title_style));
            if let Some(text) = new_badge_text {
                spans.push(Span::styled(text, theme::accent_style().bg(row_bg)));
            }
            spans.push(Span::styled(domain, theme::dim_style().bg(row_bg)));

            let line = Line::from(spans);
            buf.set_line(inner.left(), y, &line, inner.width);
        }
    }
}

/// Renders a Unix timestamp as `"Ns"`/`"Nm"`/`"Nh"`/`"Nd"` relative to
/// `now`. Per-frame callers should hoist `now` out of any visible-rows
/// loop so they only `clock_gettime` once per render.
pub fn format_time_ago_since(timestamp: i64, now: i64) -> String {
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

    /// Renders `stories` through [`StoryList`] into a `width`×6 buffer
    /// and returns the first row as text.
    fn render_first_row(
        stories: &[std::sync::Arc<crate::api::types::Item>],
        pulse_store: &crate::state::pulse_store::PulseStore,
        rising: bool,
        width: u16,
        now: i64,
    ) -> String {
        use super::StoryList;
        use crate::state::pin_store::PinStore;
        use crate::state::read_store::ReadStore;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;

        let read_store = ReadStore::empty();
        let pin_store = PinStore::empty();
        let domains: Vec<Option<String>> = stories.iter().map(|s| s.domain()).collect();
        let area = Rect::new(0, 0, width, 6);
        let mut buf = Buffer::empty(area);
        StoryList {
            stories,
            domains: &domains,
            selected: 0,
            focused: false,
            loading: false,
            search_query: None,
            read_store: &read_store,
            pin_store: &pin_store,
            pulse_store,
            rising,
            pane_title: None,
            now_secs: now,
        }
        .render(area, &mut buf);
        (0..buf.area.width)
            .map(|x| buf[(x, 1)].symbol().to_string())
            .collect()
    }

    fn hot_story(id: u64, now: i64) -> std::sync::Arc<crate::api::types::Item> {
        std::sync::Arc::new(crate::api::types::Item {
            id,
            title: Some("Rising story".into()),
            url: Some("https://example.com/x".into()),
            text: None,
            by: Some("alice".into()),
            score: Some(60),
            time: Some(now - 1800),
            kids: None,
            descendants: Some(9),
            item_type: Some(crate::api::types::ItemType::Story),
            dead: None,
            deleted: None,
        })
    }

    /// A store where story 1 gained 40 points over the last 15 minutes
    /// (→ +80/30m, well past the `↗` threshold) and story 2 has no
    /// samples.
    fn hot_store(now: i64) -> crate::state::pulse_store::PulseStore {
        use crate::api::types::StoryId;
        use crate::pulse::Sample;
        use crate::state::pulse_store::PulseStore;
        let mut store = PulseStore::empty();
        store.record(
            StoryId(1),
            Some(now - 1800),
            Sample {
                at: now - 900,
                points: 20,
                comments: 2,
            },
        );
        store.record(
            StoryId(1),
            None,
            Sample {
                at: now,
                points: 60,
                comments: 9,
            },
        );
        store
    }

    #[test]
    fn hot_story_gets_arrow_glyph_in_regular_feeds() {
        let now = 1_700_000_000;
        let store = hot_store(now);
        let row = render_first_row(&[hot_story(1, now)], &store, false, 80, now);
        assert!(row.contains("\u{2197} Rising story"), "{row:?}");
        assert!(!row.contains('▁'), "no sparkline outside Rising: {row:?}");

        let cold = render_first_row(&[hot_story(2, now)], &store, false, 80, now);
        assert!(!cold.contains('\u{2197}'), "{cold:?}");
    }

    #[test]
    fn rising_feed_renders_sparkline_and_velocity_column() {
        let now = 1_700_000_000;
        let store = hot_store(now);
        // 80 wide → inner 78 → Compact layout (no front-page chip).
        let row = render_first_row(&[hot_story(1, now)], &store, true, 80, now);
        assert!(row.contains('█'), "sparkline expected: {row:?}");
        assert!(row.contains(" +80 Rising story"), "{row:?}");
        assert!(!row.contains("FP"), "compact layout has no chip: {row:?}");

        // A Rising row without momentum keeps the column blank so titles
        // stay aligned with their neighbours.
        let blank = render_first_row(&[hot_story(2, now)], &store, true, 80, now);
        // Compare visual columns, not byte offsets — the sparkline glyphs
        // are three bytes each.
        let title_col = |r: &str| r[..r.find("Rising story").unwrap()].chars().count();
        assert_eq!(title_col(&row), title_col(&blank), "{row:?} vs {blank:?}");
    }

    #[test]
    fn rising_feed_wide_layout_shows_front_page_chip() {
        let now = 1_700_000_000;
        let mut store = hot_store(now);
        // A sweep installing story 1 at front-page rank 4.
        store.merge_sweep(&[], vec![7, 8, 9, 1], now);
        let row = render_first_row(&[hot_story(1, now)], &store, true, 100, now);
        assert!(row.contains("on FP #4"), "{row:?}");
    }

    #[test]
    fn rising_feed_narrow_pane_falls_back_to_glyph() {
        let now = 1_700_000_000;
        let store = hot_store(now);
        let row = render_first_row(&[hot_story(1, now)], &store, true, 40, now);
        assert!(row.contains('\u{2197}'), "{row:?}");
        assert!(!row.contains('█'), "{row:?}");
    }

    #[test]
    fn rising_feed_empty_shows_warming_up() {
        let now = 1_700_000_000;
        let store = crate::state::pulse_store::PulseStore::empty();
        use super::StoryList;
        use crate::state::pin_store::PinStore;
        use crate::state::read_store::ReadStore;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;
        let read_store = ReadStore::empty();
        let pin_store = PinStore::empty();
        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        StoryList {
            stories: &[],
            domains: &[],
            selected: 0,
            focused: false,
            loading: false,
            search_query: None,
            read_store: &read_store,
            pin_store: &pin_store,
            pulse_store: &store,
            rising: true,
            pane_title: Some("Rising · warming up…"),
            now_secs: now,
        }
        .render(area, &mut buf);
        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(text.contains("Rising · warming up…"), "{text:?}");
        assert!(text.contains("Warming up"), "{text:?}");
    }

    #[test]
    fn search_title_sanitises_terminal_escapes() {
        use super::StoryList;
        use crate::state::pin_store::PinStore;
        use crate::state::pulse_store::PulseStore;
        use crate::state::read_store::ReadStore;
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;

        let read_store = ReadStore::empty();
        let pin_store = PinStore::empty();
        let pulse_store = PulseStore::empty();
        let area = Rect::new(0, 0, 80, 6);
        let mut buf = Buffer::empty(area);
        StoryList {
            stories: &[],
            domains: &[],
            selected: 0,
            focused: false,
            loading: false,
            // Committed query carries an OSC-0 title-rewrite sequence.
            search_query: Some("rust\x1b]0;OWNED\x07lang"),
            read_store: &read_store,
            pin_store: &pin_store,
            pulse_store: &pulse_store,
            rising: false,
            pane_title: None,
            now_secs: 0,
        }
        .render(area, &mut buf);

        let mut text = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                text.push_str(buf[(x, y)].symbol());
            }
        }
        assert!(
            !text.contains('\x1b'),
            "ESC must not survive in search title: {text:?}"
        );
        assert!(
            !text.contains('\x07'),
            "BEL must not survive in search title"
        );
        assert!(
            text.contains("rust"),
            "query text should still render: {text:?}"
        );
    }
}
