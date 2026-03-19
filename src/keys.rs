use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    SearchInput,
}

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
