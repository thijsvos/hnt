//! Terminal lifecycle: enter/leave alternate screen, raw mode, mouse capture.
//!
//! [`init`] and [`restore`] bracket the app's runtime; [`install_panic_hook`]
//! ensures the terminal is restored even on panic so the user's shell
//! isn't left in raw mode.

use anyhow::Result;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{backend::CrosstermBackend, Terminal};
use std::io::{self, Stdout};

/// Concrete ratatui terminal type used by the app.
pub type Tui = Terminal<CrosstermBackend<Stdout>>;

/// Enters raw mode, switches to the alternate screen, and enables mouse
/// capture; returns a ready-to-draw [`Tui`].
///
/// # Errors
///
/// Returns the underlying `crossterm`/[`std::io::Error`] if
/// `enable_raw_mode`, the alternate-screen / mouse-capture sequences, or
/// terminal construction fail.
pub fn init() -> Result<Tui> {
    enable_raw_mode()?;
    execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let terminal = Terminal::new(backend)?;
    Ok(terminal)
}

/// Undoes [`init`]: disables raw mode, leaves the alternate screen, and
/// disables mouse capture. Safe to call from a panic hook.
///
/// # Errors
///
/// Returns the underlying `crossterm`/[`std::io::Error`] if
/// `disable_raw_mode` or the alternate-screen / mouse-capture teardown
/// sequences fail.
pub fn restore() -> Result<()> {
    disable_raw_mode()?;
    execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    Ok(())
}

/// Installs a panic hook that restores the terminal before panicking.
pub fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = restore();
        original_hook(panic_info);
    }));
}
