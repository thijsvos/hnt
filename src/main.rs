mod api;
mod app;
mod event;
mod keys;
mod state;
mod tui;
mod ui;

use anyhow::Result;
use app::App;
use event::{Event, EventHandler};
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
            ui::render(&app, frame);
        })?;

        // Handle events
        match events.next().await? {
            Event::Key(key) => {
                let action = keys::map_key(key, app.show_help);
                app.dispatch(action);
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
                // Tick — just triggers redraw above
            }
        }
    }

    tui::restore()?;
    Ok(())
}
