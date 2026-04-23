pub mod article_reader;
pub mod comment_tree;
pub mod header;
pub mod layout;
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
        },
        layout.stories,
    );

    // Comment tree
    frame.render_widget(
        comment_tree::CommentTree {
            state: &mut app.comment_state,
            focused: app.focus == crate::app::Pane::Comments,
            tick: app.tick_count,
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
        let visible = app.comment_state.visible_comments();
        let total = app
            .comment_state
            .story
            .as_ref()
            .and_then(|s| s.descendants)
            .unwrap_or(visible.len() as i64);
        if visible.is_empty() {
            "0/0".to_string()
        } else {
            format!("{}/{}", app.comment_state.selected + 1, total)
        }
    };

    frame.render_widget(
        status_bar::StatusBar {
            feed: app.current_feed,
            position,
            error: app.error.clone(),
            focus_pane: match app.focus {
                crate::app::Pane::Stories => "Stories",
                crate::app::Pane::Comments => "Comments",
            },
            input_mode: app.input_mode,
            search_input: app.search_state.as_ref().map(|ss| ss.input.clone()),
            search_query: if search_active {
                search_query.map(|s| s.to_string())
            } else {
                None
            },
        },
        layout.status,
    );

    // Help overlay
    if app.show_help {
        render_help_overlay(frame, area);
    }

    // Article reader overlay
    if let Some(ref reader_state) = app.reader_state {
        article_reader::render_article_overlay(frame, area, reader_state);
    }
}

fn render_help_overlay(frame: &mut Frame, area: Rect) {
    let width = 50u16.min(area.width.saturating_sub(4));
    let height = 21u16.min(area.height.saturating_sub(4));
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
