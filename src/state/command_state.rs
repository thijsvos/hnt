//! Command-line (`:`) UI state and the command palette overlay state.
//!
//! [`CommandState`] mirrors [`crate::state::search_state::SearchState`]:
//! a single struct holding the in-progress typed line plus
//! history-navigation bookkeeping. The persisted history list lives on
//! [`crate::app::App`] directly so
//! [`crate::state::command_history_store`] can round-trip it
//! independently of whether the user is currently in command mode.
//!
//! [`PaletteState`] is the fuzzy-searchable popup variant — opened via
//! `Ctrl+P`, it holds the query, the filtered-and-ranked indices into
//! [`crate::command::CommandRegistry`], and the selected row.

/// State for an active `:command` prompt.
///
/// `input` is the line typed so far; `cursor` is the byte offset of
/// the insertion point (always at a UTF-8 char boundary —
/// [`Self::set_input`] enforces this). `history_idx` is `Some(i)`
/// while the user is walking through prior commands via `Up`/`Down`;
/// `pre_history` snapshots the in-progress line so cancelling history
/// navigation restores it verbatim.
#[derive(Debug, Default, Clone)]
pub struct CommandState {
    /// Current typed line (without the leading `:`).
    pub input: String,
    /// Byte offset of the insertion point in [`Self::input`]. Always at
    /// a char boundary.
    pub cursor: usize,
    /// Index into the persisted history list (newest=last) while the
    /// user is walking through prior commands; `None` when typing fresh.
    pub history_idx: Option<usize>,
    /// In-progress line saved when entering history navigation so the
    /// user can return to it without losing what they had typed.
    pub pre_history: String,
}

impl CommandState {
    /// Fresh empty state — used when [`crate::app::App::enter_command_mode`]
    /// installs the prompt.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replace the line and pin the cursor to the end. Used by history
    /// navigation and tab-completion.
    pub fn set_input(&mut self, s: String) {
        self.cursor = s.len();
        self.input = s;
    }

    /// Append `c` at the cursor and advance.
    pub fn insert_char(&mut self, c: char) {
        let mut buf = [0u8; 4];
        let s = c.encode_utf8(&mut buf);
        self.input.insert_str(self.cursor, s);
        self.cursor += s.len();
        // Typing exits history-navigation mode — anything past this is
        // the user's own edit, not a recalled command.
        self.history_idx = None;
    }

    /// Remove the char before the cursor. No-op if cursor is at 0.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        // Walk back to the previous char boundary.
        let mut new_cursor = self.cursor - 1;
        while new_cursor > 0 && !self.input.is_char_boundary(new_cursor) {
            new_cursor -= 1;
        }
        self.input.replace_range(new_cursor..self.cursor, "");
        self.cursor = new_cursor;
        self.history_idx = None;
    }

    /// Moves the cursor one char left, stopping at column 0. UTF-8 aware.
    pub fn move_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let mut c = self.cursor - 1;
        while c > 0 && !self.input.is_char_boundary(c) {
            c -= 1;
        }
        self.cursor = c;
    }

    /// Moves the cursor one char right, stopping at the end. UTF-8 aware.
    pub fn move_right(&mut self) {
        if self.cursor >= self.input.len() {
            return;
        }
        let mut c = self.cursor + 1;
        while c < self.input.len() && !self.input.is_char_boundary(c) {
            c += 1;
        }
        self.cursor = c;
    }

    /// Moves the cursor to the start of the line.
    pub fn move_home(&mut self) {
        self.cursor = 0;
    }

    /// Moves the cursor to the end of the line.
    pub fn move_end(&mut self) {
        self.cursor = self.input.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insert_char_advances_cursor() {
        let mut s = CommandState::new();
        s.insert_char('q');
        s.insert_char('u');
        assert_eq!(s.input, "qu");
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn backspace_removes_previous_char() {
        let mut s = CommandState::new();
        s.set_input("quit".to_string());
        s.backspace();
        assert_eq!(s.input, "qui");
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn backspace_at_zero_is_noop() {
        let mut s = CommandState::new();
        s.backspace();
        assert_eq!(s.input, "");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn backspace_respects_utf8_boundaries() {
        let mut s = CommandState::new();
        s.insert_char('é'); // 2 bytes
        s.insert_char('a');
        assert_eq!(s.cursor, 3);
        s.backspace();
        assert_eq!(s.input, "é");
        assert_eq!(s.cursor, 2);
        s.backspace();
        assert_eq!(s.input, "");
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn typing_after_recall_clears_history_index() {
        let mut s = CommandState::new();
        s.history_idx = Some(3);
        s.insert_char('x');
        assert_eq!(s.history_idx, None);
    }

    #[test]
    fn set_input_pins_cursor_to_end() {
        let mut s = CommandState::new();
        s.set_input("filter author dang".to_string());
        assert_eq!(s.cursor, 18);
    }

    #[test]
    fn move_left_right_walk_one_char() {
        let mut s = CommandState::new();
        s.set_input("feed".to_string());
        assert_eq!(s.cursor, 4);
        s.move_left();
        assert_eq!(s.cursor, 3);
        s.move_left();
        s.move_left();
        s.move_left();
        s.move_left(); // saturates at 0
        assert_eq!(s.cursor, 0);
        s.move_right();
        assert_eq!(s.cursor, 1);
    }

    #[test]
    fn move_right_saturates_at_end() {
        let mut s = CommandState::new();
        s.set_input("hi".to_string());
        s.move_home();
        assert_eq!(s.cursor, 0);
        s.move_right();
        s.move_right();
        s.move_right(); // past end → clamps
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn move_left_right_respect_utf8_boundaries() {
        let mut s = CommandState::new();
        s.set_input("éb".to_string()); // 'é' is 2 bytes, cursor at 3
        assert_eq!(s.cursor, 3);
        s.move_left(); // onto 'b' boundary at byte 2
        assert_eq!(s.cursor, 2);
        s.move_left(); // skip over the 2-byte 'é' to byte 0
        assert_eq!(s.cursor, 0);
        s.move_right(); // forward across 'é' to byte 2
        assert_eq!(s.cursor, 2);
    }

    #[test]
    fn insert_at_cursor_after_move_left() {
        // The cursor must drive insertion, not just sit at the end.
        let mut s = CommandState::new();
        s.set_input("fed".to_string());
        s.move_left(); // between 'e' and 'd'
        s.insert_char('e');
        assert_eq!(s.input, "feed");
        assert_eq!(s.cursor, 3);
    }

    #[test]
    fn home_and_end_jump_to_extremes() {
        let mut s = CommandState::new();
        s.set_input("filter".to_string());
        s.move_home();
        assert_eq!(s.cursor, 0);
        s.move_end();
        assert_eq!(s.cursor, 6);
    }
}

/// State for the command-palette overlay (`Ctrl+P`).
///
/// `query` is the in-progress filter string; `matches` is the
/// registry-index list emitted by
/// [`crate::command::CommandRegistry::fuzzy`], rebuilt on every keystroke
/// via [`crate::app::App::recompute_palette_matches`]; `selected` is the
/// row within `matches` the user is about to run.
#[derive(Debug, Default, Clone)]
pub struct PaletteState {
    /// Typed query — filters the matches list.
    pub query: String,
    /// Indices into the [`crate::command::CommandRegistry`] command list
    /// that pass the current fuzzy filter, sorted by descending score.
    pub matches: Vec<usize>,
    /// Selected position within `matches`. Reset to 0 by
    /// [`crate::app::App::recompute_palette_matches`] on every query change,
    /// so it always indexes the current top match.
    pub selected: usize,
    /// Whether the user has typed a query or moved the selection since the
    /// palette opened. A bare Enter on an *untouched* palette must not run
    /// the pre-highlighted row-0 command (see
    /// [`crate::app::App::palette_submit`]).
    pub interacted: bool,
}

impl PaletteState {
    /// Empty palette — no query, no matches, no selection. Populated by
    /// [`crate::app::App::open_command_palette`].
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the currently-selected match's registry index, if any.
    pub fn selected_index(&self) -> Option<usize> {
        self.matches.get(self.selected).copied()
    }

    /// Moves the selection cursor down one row, saturating at the last
    /// match.
    pub fn move_down(&mut self) {
        self.interacted = true;
        if self.matches.is_empty() {
            return;
        }
        if self.selected + 1 < self.matches.len() {
            self.selected += 1;
        }
    }

    /// Moves the selection cursor up one row, saturating at 0.
    pub fn move_up(&mut self) {
        self.interacted = true;
        self.selected = self.selected.saturating_sub(1);
    }

    /// Appends `c` to the query string.
    pub fn push_query(&mut self, c: char) {
        self.interacted = true;
        self.query.push(c);
    }

    /// Removes the last char of the query string.
    pub fn pop_query(&mut self) {
        self.interacted = true;
        self.query.pop();
    }
}

#[cfg(test)]
mod palette_tests {
    use super::*;

    #[test]
    fn empty_palette_has_no_selection() {
        let p = PaletteState::new();
        assert!(p.selected_index().is_none());
    }

    #[test]
    fn move_down_saturates_at_end() {
        let mut p = PaletteState::new();
        p.matches = vec![0, 1, 2];
        p.move_down();
        p.move_down();
        p.move_down();
        p.move_down();
        assert_eq!(p.selected, 2, "selection must clamp at len-1");
    }

    #[test]
    fn move_up_saturates_at_zero() {
        let mut p = PaletteState::new();
        p.matches = vec![0, 1, 2];
        p.selected = 1;
        p.move_up();
        p.move_up();
        p.move_up();
        assert_eq!(p.selected, 0);
    }

    #[test]
    fn move_on_empty_list_is_noop() {
        let mut p = PaletteState::new();
        p.move_down();
        p.move_up();
        assert_eq!(p.selected, 0);
    }
}
