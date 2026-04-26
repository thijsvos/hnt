//! Keybinding → [`Action`] translation.
//!
//! [`map_key`] is a pure, context-aware dispatch: search-input and
//! hint-mode both suppress normal keys (`main.rs` routes the raw
//! characters); the help overlay eats any keypress; the article-reader
//! and prior-discussions overlays each have their own reduced keymap,
//! with the article reader taking precedence when both are open. The
//! [`InputMode`] enum distinguishes character-capturing search input
//! and Quickjump hint selection from normal navigation.

use crate::state::hint_state::HintAction;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

/// Whether the keyboard is in normal-navigation mode or accumulating
/// characters for the search-query input or a Quickjump hint label.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    SearchInput,
    /// Active during Quickjump label selection — `main.rs` routes raw
    /// chars to [`Action::HintKey`] and Esc to [`Action::ExitHintMode`].
    HintMode,
}

/// A keybinding-independent operation the app may perform.
///
/// Produced by [`map_key`] from raw [`KeyEvent`]s, consumed by
/// [`crate::app::App::dispatch`]. Unmapped keys yield `None` from
/// [`map_key`] — there is no in-band sentinel variant.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
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
    TogglePriorDiscussions,
    EnterSearch,
    /// Cycle the comment-pane "what's new" filter: All → New since last
    /// visit → Recent 24h → All. Falls through to All when the story has
    /// never been visited (no `last_seen_at` to anchor `NewSince` to).
    CycleCommentFilter,
    /// Toggle the pinned state of the focused story (pin if not pinned,
    /// unpin if pinned). Pinned stories surface in the
    /// [`crate::api::types::FeedKind::Pinned`] virtual feed.
    TogglePin,
    /// Quickjump: enter hint-label mode; the `HintAction` decides what
    /// fires on a unique label match (open in browser / open in reader /
    /// copy URL to clipboard via OSC 52).
    EnterHintMode(HintAction),
    /// One typed character of a hint-label prefix (only emitted while
    /// [`InputMode::HintMode`] is active).
    HintKey(char),
    /// Cancel hint-label selection and return to the prior input mode.
    ExitHintMode,
    Back,
}

/// Translates a [`KeyEvent`] into an [`Action`] for the current UI
/// context.
///
/// Priority order: search-input + hint-mode suppress normal keys (return
/// `None` so `main.rs` can handle raw characters); a visible help
/// overlay consumes any key as [`Action::ToggleHelp`]; a visible reader
/// overlay uses its own reduced keymap; a visible prior-discussions
/// overlay likewise uses a reduced keymap; otherwise the standard
/// navigation keymap applies. Unmapped keys yield `None`.
pub fn map_key(
    key: KeyEvent,
    help_visible: bool,
    reader_visible: bool,
    prior_visible: bool,
    input_mode: InputMode,
) -> Option<Action> {
    // SearchInput and HintMode both consume raw chars in main.rs.
    if matches!(input_mode, InputMode::SearchInput | InputMode::HintMode) {
        return None;
    }

    // If help overlay is open, any key closes it
    if help_visible {
        return Some(Action::ToggleHelp);
    }

    // Reader overlay has its own limited key set
    if reader_visible {
        return match key.code {
            KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
            KeyCode::Char('g') => Some(Action::JumpTop),
            KeyCode::Char('G') => Some(Action::JumpBottom),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::PageDown)
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                Some(Action::PageUp)
            }
            KeyCode::Char('o') => Some(Action::OpenInBrowser),
            KeyCode::Char('f') => Some(Action::EnterHintMode(HintAction::Open)),
            KeyCode::Char('F') => Some(Action::EnterHintMode(HintAction::OpenInReader)),
            KeyCode::Char('y') => Some(Action::EnterHintMode(HintAction::CopyUrl)),
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Back),
            _ => None,
        };
    }

    // Prior-discussions overlay has its own limited key set. Note: `h` here
    // does NOT close the overlay — use Esc/q. This leaves `h` free to be the
    // global toggle in normal mode (below) without ambiguity.
    if prior_visible {
        return match key.code {
            KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
            KeyCode::Char('g') => Some(Action::JumpTop),
            KeyCode::Char('G') => Some(Action::JumpBottom),
            KeyCode::Enter => Some(Action::Select),
            KeyCode::Char('o') => Some(Action::OpenInBrowser),
            KeyCode::Esc | KeyCode::Char('q') => Some(Action::Back),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Char('q') => Some(Action::Quit),
        KeyCode::Esc => Some(Action::Back),
        KeyCode::Char('j') | KeyCode::Down => Some(Action::MoveDown),
        KeyCode::Char('k') | KeyCode::Up => Some(Action::MoveUp),
        KeyCode::Enter => Some(Action::Select),
        KeyCode::Char('o') => Some(Action::OpenInBrowser),
        KeyCode::Char('p') => Some(Action::OpenReader),
        KeyCode::Char('h') => Some(Action::TogglePriorDiscussions),
        KeyCode::Tab | KeyCode::BackTab | KeyCode::Left | KeyCode::Right => {
            Some(Action::SwitchPane)
        }
        // 1–7: Top, New, Best, Ask, Show, Jobs, Pinned (matches FeedKind::ALL order).
        KeyCode::Char(c @ '1'..='7') => Some(Action::SwitchFeed(c as usize - '1' as usize)),
        KeyCode::Char('r') => Some(Action::Refresh),
        KeyCode::Char('n') => Some(Action::CycleCommentFilter),
        KeyCode::Char('b') => Some(Action::TogglePin),
        KeyCode::Char('g') => Some(Action::JumpTop),
        KeyCode::Char('G') => Some(Action::JumpBottom),
        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(Action::PageDown)
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => Some(Action::PageUp),
        KeyCode::Char('/') => Some(Action::EnterSearch),
        KeyCode::Char('?') => Some(Action::ToggleHelp),
        // Quickjump entry — comments-pane variant. Reader-overlay variant
        // is handled in the reader_visible block above.
        KeyCode::Char('f') => Some(Action::EnterHintMode(HintAction::Open)),
        KeyCode::Char('F') => Some(Action::EnterHintMode(HintAction::OpenInReader)),
        KeyCode::Char('y') => Some(Action::EnterHintMode(HintAction::CopyUrl)),
        _ => None,
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
                false,
                InputMode::SearchInput
            ),
            None
        );
        assert_eq!(
            map_key(
                key(KeyCode::Char('q')),
                true,
                true,
                false,
                InputMode::SearchInput
            ),
            None
        );
    }

    #[test]
    fn help_visible_closes_on_any_key() {
        assert_eq!(
            map_key(
                key(KeyCode::Char('q')),
                true,
                false,
                false,
                InputMode::Normal
            ),
            Some(Action::ToggleHelp)
        );
        assert_eq!(
            map_key(key(KeyCode::Enter), true, false, false, InputMode::Normal),
            Some(Action::ToggleHelp)
        );
    }

    #[test]
    fn help_takes_priority_over_reader() {
        assert_eq!(
            map_key(
                key(KeyCode::Char('j')),
                true,
                true,
                false,
                InputMode::Normal
            ),
            Some(Action::ToggleHelp)
        );
    }

    // --- Reader mode ---

    #[test]
    fn reader_navigation() {
        let r = |code| map_key(key(code), false, true, false, InputMode::Normal);
        assert_eq!(r(KeyCode::Char('j')), Some(Action::MoveDown));
        assert_eq!(r(KeyCode::Down), Some(Action::MoveDown));
        assert_eq!(r(KeyCode::Char('k')), Some(Action::MoveUp));
        assert_eq!(r(KeyCode::Up), Some(Action::MoveUp));
        assert_eq!(r(KeyCode::Char('g')), Some(Action::JumpTop));
        assert_eq!(r(KeyCode::Char('G')), Some(Action::JumpBottom));
        assert_eq!(r(KeyCode::Char('o')), Some(Action::OpenInBrowser));
        assert_eq!(r(KeyCode::Esc), Some(Action::Back));
        assert_eq!(r(KeyCode::Char('q')), Some(Action::Back));
    }

    #[test]
    fn reader_ctrl_keys() {
        assert_eq!(
            map_key(ctrl('d'), false, true, false, InputMode::Normal),
            Some(Action::PageDown)
        );
        assert_eq!(
            map_key(ctrl('u'), false, true, false, InputMode::Normal),
            Some(Action::PageUp)
        );
    }

    #[test]
    fn reader_unmapped_returns_none() {
        assert_eq!(
            map_key(
                key(KeyCode::Char('p')),
                false,
                true,
                false,
                InputMode::Normal
            ),
            None
        );
        assert_eq!(
            map_key(key(KeyCode::Enter), false, true, false, InputMode::Normal),
            None
        );
    }

    // --- Normal mode ---

    #[test]
    fn normal_quit_and_back() {
        let n = |code| map_key(key(code), false, false, false, InputMode::Normal);
        assert_eq!(n(KeyCode::Char('q')), Some(Action::Quit));
        assert_eq!(n(KeyCode::Esc), Some(Action::Back));
    }

    #[test]
    fn normal_navigation() {
        let n = |code| map_key(key(code), false, false, false, InputMode::Normal);
        assert_eq!(n(KeyCode::Char('j')), Some(Action::MoveDown));
        assert_eq!(n(KeyCode::Down), Some(Action::MoveDown));
        assert_eq!(n(KeyCode::Char('k')), Some(Action::MoveUp));
        assert_eq!(n(KeyCode::Up), Some(Action::MoveUp));
        assert_eq!(n(KeyCode::Enter), Some(Action::Select));
        assert_eq!(n(KeyCode::Char('g')), Some(Action::JumpTop));
        assert_eq!(n(KeyCode::Char('G')), Some(Action::JumpBottom));
    }

    #[test]
    fn normal_ctrl_keys() {
        assert_eq!(
            map_key(ctrl('d'), false, false, false, InputMode::Normal),
            Some(Action::PageDown)
        );
        assert_eq!(
            map_key(ctrl('u'), false, false, false, InputMode::Normal),
            Some(Action::PageUp)
        );
    }

    #[test]
    fn normal_feed_switching() {
        let n = |c: char| {
            map_key(
                key(KeyCode::Char(c)),
                false,
                false,
                false,
                InputMode::Normal,
            )
        };
        for (c, idx) in [
            ('1', 0),
            ('2', 1),
            ('3', 2),
            ('4', 3),
            ('5', 4),
            ('6', 5),
            ('7', 6), // Pinned virtual feed
        ] {
            assert_eq!(n(c), Some(Action::SwitchFeed(idx)));
        }
    }

    #[test]
    fn normal_b_toggles_pin() {
        let n = |code| map_key(key(code), false, false, false, InputMode::Normal);
        assert_eq!(n(KeyCode::Char('b')), Some(Action::TogglePin));
    }

    #[test]
    fn reader_overlay_does_not_consume_b() {
        // Pin toggle is a story-level action; the reader overlay should
        // not emit it (the reader's focused story is already known to the
        // user — pinning belongs in the underlying pane).
        let r = |code| map_key(key(code), false, true, false, InputMode::Normal);
        assert_eq!(r(KeyCode::Char('b')), None);
    }

    #[test]
    fn prior_overlay_does_not_consume_b() {
        let p = |code| map_key(key(code), false, false, true, InputMode::Normal);
        assert_eq!(p(KeyCode::Char('b')), None);
    }

    #[test]
    fn search_input_does_not_emit_toggle_pin() {
        // `b` must be a query character when typing a search.
        assert_eq!(
            map_key(
                key(KeyCode::Char('b')),
                false,
                false,
                false,
                InputMode::SearchInput
            ),
            None
        );
    }

    #[test]
    fn normal_actions() {
        let n = |code| map_key(key(code), false, false, false, InputMode::Normal);
        assert_eq!(n(KeyCode::Char('o')), Some(Action::OpenInBrowser));
        assert_eq!(n(KeyCode::Char('p')), Some(Action::OpenReader));
        assert_eq!(n(KeyCode::Char('h')), Some(Action::TogglePriorDiscussions));
        assert_eq!(n(KeyCode::Tab), Some(Action::SwitchPane));
        assert_eq!(n(KeyCode::BackTab), Some(Action::SwitchPane));
        assert_eq!(n(KeyCode::Left), Some(Action::SwitchPane));
        assert_eq!(n(KeyCode::Right), Some(Action::SwitchPane));
        assert_eq!(n(KeyCode::Char('r')), Some(Action::Refresh));
        assert_eq!(n(KeyCode::Char('/')), Some(Action::EnterSearch));
        assert_eq!(n(KeyCode::Char('?')), Some(Action::ToggleHelp));
    }

    #[test]
    fn normal_unmapped_returns_none() {
        assert_eq!(
            map_key(
                key(KeyCode::Char('z')),
                false,
                false,
                false,
                InputMode::Normal
            ),
            None
        );
    }

    // --- Prior-discussions overlay ---

    #[test]
    fn prior_overlay_keymap() {
        let p = |code| map_key(key(code), false, false, true, InputMode::Normal);
        assert_eq!(p(KeyCode::Char('j')), Some(Action::MoveDown));
        assert_eq!(p(KeyCode::Down), Some(Action::MoveDown));
        assert_eq!(p(KeyCode::Char('k')), Some(Action::MoveUp));
        assert_eq!(p(KeyCode::Up), Some(Action::MoveUp));
        assert_eq!(p(KeyCode::Char('g')), Some(Action::JumpTop));
        assert_eq!(p(KeyCode::Char('G')), Some(Action::JumpBottom));
        assert_eq!(p(KeyCode::Enter), Some(Action::Select));
        assert_eq!(p(KeyCode::Char('o')), Some(Action::OpenInBrowser));
        assert_eq!(p(KeyCode::Esc), Some(Action::Back));
        assert_eq!(p(KeyCode::Char('q')), Some(Action::Back));
    }

    #[test]
    fn prior_overlay_consumes_unmapped_keys() {
        // Keys that would otherwise dispatch in normal mode — the overlay
        // should swallow them so e.g. `/` doesn't enter search underneath.
        let p = |code| map_key(key(code), false, false, true, InputMode::Normal);
        assert_eq!(p(KeyCode::Char('/')), None);
        assert_eq!(p(KeyCode::Char('1')), None);
        assert_eq!(p(KeyCode::Char('p')), None);
        assert_eq!(p(KeyCode::Char('h')), None);
    }

    #[test]
    fn reader_takes_priority_over_prior() {
        // If both overlays are somehow flagged simultaneously, reader wins
        // because it's strictly modal (article-reading beats context-lookup).
        assert_eq!(
            map_key(
                key(KeyCode::Char('o')),
                false,
                true,
                true,
                InputMode::Normal
            ),
            Some(Action::OpenInBrowser)
        );
    }

    #[test]
    fn help_takes_priority_over_prior() {
        assert_eq!(
            map_key(
                key(KeyCode::Char('j')),
                true,
                false,
                true,
                InputMode::Normal
            ),
            Some(Action::ToggleHelp)
        );
    }

    // --- HintMode (Quickjump) ---

    #[test]
    fn hint_mode_suppresses_all_keys() {
        // Every key returns None so main.rs can route raw chars to
        // HintKey via the input-mode shortcut.
        assert_eq!(
            map_key(
                key(KeyCode::Char('a')),
                false,
                false,
                false,
                InputMode::HintMode
            ),
            None
        );
        assert_eq!(
            map_key(key(KeyCode::Esc), false, false, false, InputMode::HintMode),
            None
        );
        assert_eq!(
            map_key(
                key(KeyCode::Char('q')),
                false,
                false,
                false,
                InputMode::HintMode
            ),
            None
        );
        // Even with overlays and help flagged, HintMode wins.
        assert_eq!(
            map_key(
                key(KeyCode::Char('a')),
                true,
                true,
                true,
                InputMode::HintMode
            ),
            None
        );
    }

    #[test]
    fn normal_f_enters_open_hint_mode() {
        let n = |code| map_key(key(code), false, false, false, InputMode::Normal);
        assert_eq!(
            n(KeyCode::Char('f')),
            Some(Action::EnterHintMode(HintAction::Open))
        );
    }

    #[test]
    fn normal_capital_f_enters_open_in_reader_hint_mode() {
        let n = |code| map_key(key(code), false, false, false, InputMode::Normal);
        assert_eq!(
            n(KeyCode::Char('F')),
            Some(Action::EnterHintMode(HintAction::OpenInReader))
        );
    }

    #[test]
    fn normal_y_enters_copy_url_hint_mode() {
        let n = |code| map_key(key(code), false, false, false, InputMode::Normal);
        assert_eq!(
            n(KeyCode::Char('y')),
            Some(Action::EnterHintMode(HintAction::CopyUrl))
        );
    }

    #[test]
    fn reader_overlay_hint_mode_keys() {
        let r = |code| map_key(key(code), false, true, false, InputMode::Normal);
        assert_eq!(
            r(KeyCode::Char('f')),
            Some(Action::EnterHintMode(HintAction::Open))
        );
        assert_eq!(
            r(KeyCode::Char('F')),
            Some(Action::EnterHintMode(HintAction::OpenInReader))
        );
        assert_eq!(
            r(KeyCode::Char('y')),
            Some(Action::EnterHintMode(HintAction::CopyUrl))
        );
    }

    #[test]
    fn prior_overlay_does_not_emit_hint_actions() {
        // Prior overlay's keymap should still consume f/F/y as None — those
        // keys only make sense in reader/comments contexts.
        let p = |code| map_key(key(code), false, false, true, InputMode::Normal);
        assert_eq!(p(KeyCode::Char('f')), None);
        assert_eq!(p(KeyCode::Char('F')), None);
        assert_eq!(p(KeyCode::Char('y')), None);
    }

    #[test]
    fn search_input_does_not_emit_hint_actions() {
        // SearchInput priority must dominate — `f`/`F`/`y` are valid query
        // characters when the user is typing.
        let s = |c: char| {
            map_key(
                key(KeyCode::Char(c)),
                false,
                false,
                false,
                InputMode::SearchInput,
            )
        };
        assert_eq!(s('f'), None);
        assert_eq!(s('F'), None);
        assert_eq!(s('y'), None);
    }
}
