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
pub struct SearchState {
    pub query: String,
    pub input: String,
    pub current_page: usize,
    pub total_pages: usize,
    pub total_hits: usize,
}

impl SearchState {
    /// Starts with empty input/query and zeroed pagination.
    pub fn new() -> Self {
        Self {
            query: String::new(),
            input: String::new(),
            current_page: 0,
            total_pages: 0,
            total_hits: 0,
        }
    }
}
