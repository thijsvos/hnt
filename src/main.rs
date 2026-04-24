//! `hnt` — a terminal Hacker News reader built on ratatui + tokio.
//!
//! Two-pane layout (stories/comments) with an overlay article reader,
//! Algolia-backed search, and progressive comment-tree fetching.

mod api;
mod app;
mod article;
mod event;
mod keys;
mod state;
mod tui;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::event::{KeyCode, KeyModifiers};
use event::{Event, EventHandler};
use keys::InputMode;
use std::time::Duration;

#[tokio::main]
async fn main() -> Result<()> {
    tui::install_panic_hook();

    let mut terminal = tui::init()?;
    let mut events = EventHandler::new(Duration::from_millis(250));
    let size = terminal.size()?;
    let mut app = App::new(size.width, size.height);

    // Kick off initial data load
    app.load_initial_feed();

    // Main loop
    while app.running {
        // Process any pending async results
        app.process_messages();

        // Draw
        terminal.draw(|frame| {
            ui::render(&mut app, frame);
        })?;

        // Handle events
        match events.next().await? {
            Event::Key(key) => {
                if app.input_mode == InputMode::SearchInput {
                    match key.code {
                        KeyCode::Enter => app.submit_search(),
                        KeyCode::Esc => {
                            if app
                                .search_state
                                .as_ref()
                                .is_some_and(|ss| ss.query.is_empty())
                            {
                                app.cancel_search();
                            } else {
                                // Exit input mode but keep search results
                                app.input_mode = InputMode::Normal;
                            }
                        }
                        KeyCode::Backspace => app.search_input_backspace(),
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            app.cancel_search();
                        }
                        KeyCode::Char(c) => app.search_input_char(c),
                        _ => {}
                    }
                } else {
                    let action = keys::map_key(
                        key,
                        app.show_help,
                        app.reader_state.is_some(),
                        app.prior_state.is_some(),
                        app.input_mode,
                    );
                    app.dispatch(action);
                }
            }
            Event::Mouse(mouse) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                match mouse.kind {
                    MouseEventKind::Down(MouseButton::Left) => {
                        app.handle_click(mouse.column, mouse.row);
                    }
                    MouseEventKind::ScrollDown => {
                        app.handle_scroll(mouse.column, mouse.row, true);
                    }
                    MouseEventKind::ScrollUp => {
                        app.handle_scroll(mouse.column, mouse.row, false);
                    }
                    _ => {}
                }
            }
            Event::Resize(w, h) => {
                app.set_terminal_size(w, h);
            }
            Event::Tick => {
                app.tick_count = app.tick_count.wrapping_add(1);
            }
        }
    }

    tui::restore()?;
    Ok(())
}
