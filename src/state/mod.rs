//! Application state containers, separated by UI concern.
//!
//! Each submodule owns a single pane's state and its navigation/mutation
//! methods: [`story_state::StoryListState`] for the left pane,
//! [`comment_state::CommentTreeState`] for the comment tree (with collapse
//! and plain-text caching), [`reader_state::ReaderState`] for the article
//! overlay, [`search_state::SearchState`] for the Algolia search flow,
//! [`prior_state::PriorDiscussionsState`] for the prior-submissions
//! overlay, [`read_store::ReadStore`] for persisted read-state tracking,
//! [`pin_store::PinStore`] for persisted pinned stories with
//! resume-position snapshots, and [`link_registry::LinkRegistry`] +
//! [`hint_state::HintState`] for the Quickjump label-hint mode.

pub mod comment_state;
pub mod hint_state;
pub mod link_registry;
pub mod pin_store;
pub mod prior_state;
pub mod read_store;
pub mod reader_state;
pub mod search_state;
pub mod story_state;
