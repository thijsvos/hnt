//! `hnt` — a terminal Hacker News reader built on ratatui + tokio.
//!
//! Two-pane layout (stories/comments) with an overlay article reader,
//! Algolia-backed search, and progressive comment-tree fetching.

mod api;
mod app;
mod article;
mod clipboard;
mod command;
mod event;
mod keys;
mod sanitize;
mod state;
mod tui;
mod ui;

use anyhow::Result;
use app::App;
use crossterm::event::{KeyCode, KeyModifiers};
use event::{Event, EventHandler};
use keys::{Action, InputMode};
use std::time::Duration;

/// Process entry point — installs the panic hook, brings up the terminal,
/// constructs the [`App`] and [`EventHandler`], and drives the main loop
/// until `app.running` is cleared, then persists session state. A
/// `tui::TerminalGuard` restores the terminal on the way out — including a
/// `?` early-return from `draw`/event polling, which the panic hook (panics
/// only) does not cover.
///
/// Each loop iteration drains async results from background tasks via
/// `App::process_messages`, renders one frame, then `.await`s the next
/// [`Event`]. Key handling fans out by [`InputMode`]: `SearchInput` and
/// `HintMode` consume characters directly on [`App`]; `Normal` routes
/// through [`keys::map_key`] into an [`Action`] dispatched on [`App`].
/// Mouse, resize, and tick events go to their matching `App` handlers.
///
/// # Errors
///
/// Propagates the first error from [`tui::init`], `terminal.size`,
/// `terminal.draw`, or [`EventHandler::next`]; the `TerminalGuard` still
/// restores the terminal before the process exits non-zero.
#[tokio::main]
async fn main() -> Result<()> {
    tui::install_panic_hook();

    let mut terminal = tui::init()?;
    // Restore the terminal on every exit from here on — including a `?`
    // early-return below (a failed draw or event poll), which the panic hook
    // does not cover.
    let _guard = tui::TerminalGuard;
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
            Event::Key(key) => match app.input_mode {
                InputMode::SearchInput => match key.code {
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
                },
                InputMode::HintMode => match key.code {
                    KeyCode::Esc => app.dispatch(Action::ExitHintMode),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.dispatch(Action::ExitHintMode);
                    }
                    KeyCode::Char(c) => app.dispatch(Action::HintKey(c)),
                    _ => {}
                },
                InputMode::CommandInput => match key.code {
                    KeyCode::Enter => app.submit_command(),
                    KeyCode::Esc => app.cancel_command(),
                    KeyCode::Backspace => app.command_input_backspace(),
                    KeyCode::Tab => app.complete_command_at_cursor(),
                    KeyCode::Up => app.command_history_prev(),
                    KeyCode::Down => app.command_history_next(),
                    KeyCode::Left => app.command_cursor_left(),
                    KeyCode::Right => app.command_cursor_right(),
                    KeyCode::Home => app.command_cursor_home(),
                    KeyCode::End => app.command_cursor_end(),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.cancel_command();
                    }
                    // Only insert unmodified (or shift-modified) characters —
                    // a Ctrl/Alt chord (Ctrl+U, Ctrl+W, …) must not type its
                    // literal letter into the buffer.
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        app.command_input_char(c)
                    }
                    _ => {}
                },
                InputMode::PaletteInput => match key.code {
                    KeyCode::Enter => app.palette_submit(),
                    KeyCode::Esc => app.cancel_palette(),
                    KeyCode::Backspace => app.palette_input_backspace(),
                    KeyCode::Up => app.palette_move_up(),
                    KeyCode::Down => app.palette_move_down(),
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        app.cancel_palette();
                    }
                    // Only feed unmodified (or shift-modified) characters into
                    // the query — pressing Ctrl+P again (or any Ctrl/Alt chord)
                    // must not type its literal letter.
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        app.palette_input_char(c)
                    }
                    _ => {}
                },
                InputMode::Normal => {
                    if let Some(action) = keys::map_key(
                        key,
                        app.show_help,
                        app.reader_state.is_some(),
                        app.prior_state.is_some(),
                        app.input_mode,
                    ) {
                        app.dispatch(action);
                    }
                }
            },
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
            Event::Resize { width, height } => {
                app.set_terminal_size(width, height);
            }
            Event::Tick => {
                app.tick();
            }
        }
    }

    app.persist();
    // `_guard` is dropped as `main` returns, restoring the terminal.
    Ok(())
}
