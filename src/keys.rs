//! Keybinding → [`Action`] translation.
//!
//! [`map_key`] is a pure, context-aware dispatch: search-input mode
//! suppresses normal keys, the help overlay eats any keypress, and
//! the reader overlay has its own reduced map. The [`InputMode`] enum
//! distinguishes character-capturing search input from normal navigation.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Whether the keyboard is in normal-navigation mode or accumulating
/// characters for the search-query input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    SearchInput,
}

/// A keybinding-independent operation the app may perform.
///
/// Produced by [`map_key`] from raw [`KeyEvent`]s, consumed by
/// [`crate::app::App::dispatch`]. `Action::None` means "unmapped — ignore."
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Quit,
    MoveUp,
    MoveDown,
    Select,
    OpenInBrowser,
    OpenReader,
    SwitchPane,
    SwitchFeed(usize),
    Refresh,
    JumpTop,
    JumpBottom,
    PageDown,
    PageUp,
    ToggleHelp,
    EnterSearch,
    Back,
    None,
}

/// Translates a [`KeyEvent`] into an [`Action`] for the current UI
/// context.
///
/// Priority order: search-input mode suppresses normal keys (returns
/// [`Action::None`] so `main.rs` can handle raw characters); a visible
/// help overlay consumes any key as [`Action::ToggleHelp`]; a visible
/// reader overlay uses its own reduced keymap; otherwise the standard
/// navigation keymap applies.
pub fn map_key(
    key: KeyEvent,
    help_visible: bool,
    reader_visible: bool,
    input_mode: InputMode,
) -> Action {
    // When in search input mode, main.rs handles raw keys — return None here
    if input_mode == InputMode::SearchInput {
        return Action::None;
    }

    // If help overlay is open, any key closes it
    if help_visible {
        return Action::ToggleHelp;
    }

    // Reader overlay has its own limited key set
    if reader_visible {
        return match key.code {
            KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
            KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
            KeyCode::Char('g') => Action::JumpTop,
            KeyCode::Char('G') => Action::JumpBottom,
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::PageDown,
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::PageUp,
            KeyCode::Char('o') => Action::OpenInBrowser,
            KeyCode::Esc | KeyCode::Char('q') => Action::Back,
            _ => Action::None,
        };
    }

    match key.code {
        KeyCode::Char('q') => Action::Quit,
        KeyCode::Esc => Action::Back,
        KeyCode::Char('j') | KeyCode::Down => Action::MoveDown,
        KeyCode::Char('k') | KeyCode::Up => Action::MoveUp,
        KeyCode::Enter => Action::Select,
        KeyCode::Char('o') => Action::OpenInBrowser,
        KeyCode::Char('p') => Action::OpenReader,
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => Action::SwitchPane,
        KeyCode::Char('1') => Action::SwitchFeed(0),
        KeyCode::Char('2') => Action::SwitchFeed(1),
        KeyCode::Char('3') => Action::SwitchFeed(2),
        KeyCode::Char('4') => Action::SwitchFeed(3),
        KeyCode::Char('5') => Action::SwitchFeed(4),
        KeyCode::Char('6') => Action::SwitchFeed(5),
        KeyCode::Char('r') => Action::Refresh,
        KeyCode::Char('g') => Action::JumpTop,
        KeyCode::Char('G') => Action::JumpBottom,
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::PageDown,
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Action::PageUp,
        KeyCode::Char('/') => Action::EnterSearch,
        KeyCode::Char('?') => Action::ToggleHelp,
        _ => Action::None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
    }

    // --- Priority chain ---

    #[test]
    fn search_input_always_returns_none() {
        assert_eq!(
            map_key(
                key(KeyCode::Char('q')),
                false,
                false,
                InputMode::SearchInput
            ),
            Action::None
        );
        assert_eq!(
            map_key(key(KeyCode::Char('q')), true, true, InputMode::SearchInput),
            Action::None
        );
    }

    #[test]
    fn help_visible_closes_on_any_key() {
        assert_eq!(
            map_key(key(KeyCode::Char('q')), true, false, InputMode::Normal),
            Action::ToggleHelp
        );
        assert_eq!(
            map_key(key(KeyCode::Enter), true, false, InputMode::Normal),
            Action::ToggleHelp
        );
    }

    #[test]
    fn help_takes_priority_over_reader() {
        assert_eq!(
            map_key(key(KeyCode::Char('j')), true, true, InputMode::Normal),
            Action::ToggleHelp
        );
    }

    // --- Reader mode ---

    #[test]
    fn reader_navigation() {
        let r = |code| map_key(key(code), false, true, InputMode::Normal);
        assert_eq!(r(KeyCode::Char('j')), Action::MoveDown);
        assert_eq!(r(KeyCode::Down), Action::MoveDown);
        assert_eq!(r(KeyCode::Char('k')), Action::MoveUp);
        assert_eq!(r(KeyCode::Up), Action::MoveUp);
        assert_eq!(r(KeyCode::Char('g')), Action::JumpTop);
        assert_eq!(r(KeyCode::Char('G')), Action::JumpBottom);
        assert_eq!(r(KeyCode::Char('o')), Action::OpenInBrowser);
        assert_eq!(r(KeyCode::Esc), Action::Back);
        assert_eq!(r(KeyCode::Char('q')), Action::Back);
    }

    #[test]
    fn reader_ctrl_keys() {
        assert_eq!(
            map_key(ctrl('d'), false, true, InputMode::Normal),
            Action::PageDown
        );
        assert_eq!(
            map_key(ctrl('u'), false, true, InputMode::Normal),
            Action::PageUp
        );
    }

    #[test]
    fn reader_unmapped_returns_none() {
        assert_eq!(
            map_key(key(KeyCode::Char('p')), false, true, InputMode::Normal),
            Action::None
        );
        assert_eq!(
            map_key(key(KeyCode::Enter), false, true, InputMode::Normal),
            Action::None
        );
    }

    // --- Normal mode ---

    #[test]
    fn normal_quit_and_back() {
        let n = |code| map_key(key(code), false, false, InputMode::Normal);
        assert_eq!(n(KeyCode::Char('q')), Action::Quit);
        assert_eq!(n(KeyCode::Esc), Action::Back);
    }

    #[test]
    fn normal_navigation() {
        let n = |code| map_key(key(code), false, false, InputMode::Normal);
        assert_eq!(n(KeyCode::Char('j')), Action::MoveDown);
        assert_eq!(n(KeyCode::Down), Action::MoveDown);
        assert_eq!(n(KeyCode::Char('k')), Action::MoveUp);
        assert_eq!(n(KeyCode::Up), Action::MoveUp);
        assert_eq!(n(KeyCode::Enter), Action::Select);
        assert_eq!(n(KeyCode::Char('g')), Action::JumpTop);
        assert_eq!(n(KeyCode::Char('G')), Action::JumpBottom);
    }

    #[test]
    fn normal_ctrl_keys() {
        assert_eq!(
            map_key(ctrl('d'), false, false, InputMode::Normal),
            Action::PageDown
        );
        assert_eq!(
            map_key(ctrl('u'), false, false, InputMode::Normal),
            Action::PageUp
        );
    }

    #[test]
    fn normal_feed_switching() {
        let n = |c: char| map_key(key(KeyCode::Char(c)), false, false, InputMode::Normal);
        for (c, idx) in [('1', 0), ('2', 1), ('3', 2), ('4', 3), ('5', 4), ('6', 5)] {
            assert_eq!(n(c), Action::SwitchFeed(idx));
        }
    }

    #[test]
    fn normal_actions() {
        let n = |code| map_key(key(code), false, false, InputMode::Normal);
        assert_eq!(n(KeyCode::Char('o')), Action::OpenInBrowser);
        assert_eq!(n(KeyCode::Char('p')), Action::OpenReader);
        assert_eq!(n(KeyCode::Tab), Action::SwitchPane);
        assert_eq!(n(KeyCode::BackTab), Action::SwitchPane);
        assert_eq!(n(KeyCode::Left), Action::SwitchPane);
        assert_eq!(n(KeyCode::Right), Action::SwitchPane);
        assert_eq!(n(KeyCode::Char('r')), Action::Refresh);
        assert_eq!(n(KeyCode::Char('/')), Action::EnterSearch);
        assert_eq!(n(KeyCode::Char('?')), Action::ToggleHelp);
    }

    #[test]
    fn normal_unmapped_returns_none() {
        assert_eq!(
            map_key(key(KeyCode::Char('z')), false, false, InputMode::Normal),
            Action::None
        );
    }
}
