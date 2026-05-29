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
    // Raw mode is now active. If a later step fails we must undo it before
    // returning the error: `main` propagates this with `?` *before* it can
    // install the `TerminalGuard`, so nothing else would restore the
    // terminal otherwise.
    if let Err(e) = execute!(io::stdout(), EnterAlternateScreen, EnableMouseCapture) {
        let _ = disable_raw_mode();
        return Err(e.into());
    }
    let backend = CrosstermBackend::new(io::stdout());
    match Terminal::new(backend) {
        Ok(terminal) => Ok(terminal),
        Err(e) => {
            let _ = restore();
            Err(e.into())
        }
    }
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

/// RAII guard that restores the terminal when dropped.
///
/// `main` constructs one right after [`init`] succeeds, so the terminal
/// leaves raw mode and the alternate screen on *every* exit path — including
/// a `?` early-return from `terminal.draw` or event polling, which the panic
/// hook does not catch (it fires on panics, not `Err` returns). A restore
/// failure is reported to stderr but cannot be propagated out of `drop`.
pub struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if let Err(e) = restore() {
            eprintln!("hnt: failed to restore terminal: {e}");
        }
    }
}

/// Installs a panic hook that restores the terminal before panicking.
pub fn install_panic_hook() {
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        // Restore first so the panic message prints on the normal screen, not
        // the alternate one. Surface a restore failure rather than dropping it
        // silently.
        if let Err(e) = restore() {
            eprintln!("hnt: failed to restore terminal after panic: {e}");
        }
        original_hook(panic_info);
    }));
}
