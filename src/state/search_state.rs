//! Algolia-search UI state.
//!
//! Separates the in-progress typed `input` from the submitted `query`
//! so pagination (`current_page` / `total_pages`) anchors to the
//! submitted value, not whatever the user is currently typing.

/// State for an active Algolia search.
///
/// `input` is the in-progress typed text; `query` is the submitted
/// (committed) query — the two differ while the user is editing, which is
/// why pagination anchors to `query`.
#[derive(Default)]
pub struct SearchState {
    /// Committed search query — what pagination spawns send to Algolia.
    /// Empty until `App::submit_search` copies from `input`.
    pub query: String,
    /// In-progress typed input buffer. Driven by
    /// `App::search_input_char` / `_backspace` while
    /// [`crate::keys::InputMode::SearchInput`] is active.
    pub input: String,
    /// Last-fetched Algolia page index (0-based). Bumped by
    /// `App::check_lazy_load` when the user nears the loaded tail.
    pub current_page: usize,
    /// Total pages reported by the most recent Algolia response —
    /// drives the lazy-pagination cap.
    pub total_pages: usize,
    /// Total hit count from Algolia — surfaced in the status bar.
    pub total_hits: usize,
}

impl SearchState {
    /// Starts with empty input/query and zeroed pagination.
    pub fn new() -> Self {
        Self::default()
    }
}
