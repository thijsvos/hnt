//! Application state containers, separated by UI concern.
//!
//! Each submodule owns a single pane's state and its navigation/mutation
//! methods: [`story_state::StoryListState`] for the left pane,
//! [`comment_state::CommentTreeState`] for the comment tree (with collapse
//! and plain-text caching), [`reader_state::ReaderState`] for the article
//! overlay, and [`search_state::SearchState`] for the Algolia search flow.

pub mod comment_state;
pub mod reader_state;
pub mod search_state;
pub mod story_state;
