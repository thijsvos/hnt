//! Application state containers, separated by UI concern.
//!
//! Each submodule owns a single pane's state and its navigation/mutation
//! methods: [`story_state::StoryListState`] for the left pane,
//! [`comment_state::CommentTreeState`] for the comment tree (with collapse
//! and plain-text caching), [`reader_state::ReaderState`] for the article
//! overlay, [`search_state::SearchState`] for the Algolia search flow,
//! [`prior_state::PriorDiscussionsState`] for the prior-submissions
//! overlay, and [`read_store::ReadStore`] for persisted read-state tracking.

pub mod comment_state;
pub mod prior_state;
pub mod read_store;
pub mod reader_state;
pub mod search_state;
pub mod story_state;
