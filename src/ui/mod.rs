//! Terminal UI widgets and top-level frame renderer.
//!
//! [`render`] composes one ratatui frame from [`App`] state: header tab bar,
//! story list, comment tree, status bar, plus the help and article-reader
//! overlays when active. Submodules expose individual widgets and the
//! shared [`theme`] palette.

pub mod article_reader;
pub mod comment_tree;
pub mod header;
pub mod layout;
pub mod prior_overlay;
pub mod spinner;
pub mod status_bar;
pub mod story_list;
pub mod theme;

use crate::app::App;
use layout::build_layout;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// Composes one ratatui frame from [`App`] state: header, story list,
/// comment tree (with optional `· N prior (h)` badge), status bar, and
/// the help, article-reader, and prior-discussions overlays when active.
/// The article reader takes precedence over the prior-discussions overlay
/// when both are somehow open.
pub fn render(app: &mut App, frame: &mut Frame) {
    let area = frame.area();

    // Fill background
    frame.render_widget(Block::default().style(theme::base_style()), area);

    let layout = build_layout(area);

    let search_active = app.search_state.is_some();
    let search_query = app.search_state.as_ref().map(|ss| ss.query.as_str());

    // Header
    frame.render_widget(
        header::Header {
            current_feed: app.current_feed,
            search_active,
        },
        layout.header,
    );

    // Story list
    frame.render_widget(
        story_list::StoryList {
            stories: &app.story_state.stories,
            selected: app.story_state.selected,
            offset: 0,
            focused: app.focus == crate::app::Pane::Stories,
            loading: app.story_state.loading,
            search_query: if search_active { search_query } else { None },
            read_store: &app.read_store,
        },
        layout.stories,
    );

    // Walk the comment tree once per frame and share the result between the
    // comment tree widget and the status bar.
    let visible_indices = app.comment_state.visible_indices();

    // Comment tree
    let prior_count = app
        .comment_state
        .story
        .as_ref()
        .and_then(|s| app.prior_results.get(&crate::api::types::StoryId(s.id)))
        .map(|v| v.len())
        .unwrap_or(0);
    frame.render_widget(
        comment_tree::CommentTree {
            state: &mut app.comment_state,
            visible: &visible_indices,
            focused: app.focus == crate::app::Pane::Comments,
            tick: app.tick_count,
            prior_count,
        },
        layout.comments,
    );

    // Status bar
    let position = if app.focus == crate::app::Pane::Stories {
        if app.story_state.stories.is_empty() {
            "0/0".to_string()
        } else {
            format!(
                "{}/{}",
                app.story_state.selected + 1,
                app.story_state.stories.len()
            )
        }
    } else {
        let total = app
            .comment_state
            .story
            .as_ref()
            .and_then(|s| s.descendants)
            .unwrap_or(visible_indices.len() as i64);
        if visible_indices.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", app.comment_state.selected + 1, total)
        }
    };

    frame.render_widget(
        status_bar::StatusBar {
            feed: app.current_feed,
            position: &position,
            error: app.error.as_deref(),
            focus_pane: match app.focus {
                crate::app::Pane::Stories => "Stories",
                crate::app::Pane::Comments => "Comments",
            },
            input_mode: app.input_mode,
            search_input: app.search_state.as_ref().map(|ss| ss.input.as_str()),
            search_query: if search_active { search_query } else { None },
        },
        layout.status,
    );

    // Help overlay
    if app.show_help {
        render_help_overlay(frame, area);
    }

    // Article reader overlay
    if let Some(ref reader_state) = app.reader_state {
        article_reader::render_article_overlay(frame, area, reader_state, app.hint_state.as_ref());
    }

    // Prior-discussions overlay (takes precedence over comment pane focus but
    // sits below the article reader — a user who has the reader open is busy
    // with the article).
    if app.reader_state.is_none() {
        if let Some(ref prior_state) = app.prior_state {
            prior_overlay::render_prior_overlay(frame, area, prior_state);
        }
    }
}

/// Draws the centered modal help overlay listing every keybinding.
/// Bounded to at most 56×28 cells; auto-shrinks on small terminals.
fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let width = 56u16.min(area.width.saturating_sub(4));
    let height = 28u16.min(area.height.saturating_sub(4));
    let x = (area.width.saturating_sub(width)) / 2;
    let y = (area.height.saturating_sub(height)) / 2;
    let popup_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, popup_area);

    let help_text = vec![
        Line::from(Span::styled("Keybindings", theme::title_style())),
        Line::from(""),
        Line::from(vec![
            Span::styled("  j/k, arrows  ", theme::accent_style()),
            Span::styled("Navigate up/down", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  Enter        ", theme::accent_style()),
            Span::styled("Select / Toggle collapse", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  o            ", theme::accent_style()),
            Span::styled("Open URL in browser", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  p            ", theme::accent_style()),
            Span::styled("Read article inline", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  f / F / y    ", theme::accent_style()),
            Span::styled(
                "(in reader) Quickjump: open / read inline / copy",
                theme::base_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  h            ", theme::accent_style()),
            Span::styled("Show prior HN submissions of this URL", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  Tab          ", theme::accent_style()),
            Span::styled("Switch pane focus", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  1-6          ", theme::accent_style()),
            Span::styled(
                "Switch feed (Top/New/Best/Ask/Show/Jobs)",
                theme::base_style(),
            ),
        ]),
        Line::from(vec![
            Span::styled("  /            ", theme::accent_style()),
            Span::styled("Search stories", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  r            ", theme::accent_style()),
            Span::styled("Refresh", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  g/G          ", theme::accent_style()),
            Span::styled("Jump to top/bottom", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  Ctrl+d/u     ", theme::accent_style()),
            Span::styled("Page down/up", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  q            ", theme::accent_style()),
            Span::styled("Quit", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  Esc          ", theme::accent_style()),
            Span::styled("Back / Close", theme::base_style()),
        ]),
        Line::from(vec![
            Span::styled("  ?            ", theme::accent_style()),
            Span::styled("Toggle this help", theme::base_style()),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "  Visited stories are dimmed; +N marks new comments",
            theme::dim_style(),
        )),
        Line::from(""),
        Line::from(Span::styled("  Press any key to close", theme::dim_style())),
    ];

    let help = Paragraph::new(help_text)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(theme::accent_style())
                .title(Span::styled(" Help ", theme::title_style()))
                .style(theme::base_style()),
        )
        .wrap(Wrap { trim: false });

    frame.render_widget(help, popup_area);
}
