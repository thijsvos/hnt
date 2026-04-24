//! Prior-discussions overlay state.
//!
//! When a story with a URL is selected, the app fires an Algolia query
//! for other HN submissions of the same URL (see
//! [`crate::api::client::HnClient::search_by_url`]). Results are stashed
//! here and displayed via the prior-discussions overlay when the user
//! presses `h`.

use crate::api::types::Item;
#[cfg(test)]
use crate::api::types::ItemType;

/// State backing the prior-discussions overlay.
///
/// Populated from [`crate::api::client::HnClient::search_by_url`] results
/// and displayed by [`crate::ui::prior_overlay::render_prior_overlay`].
/// [`PriorDiscussionsState::new`] constructs a fresh instance positioned
/// at the first entry; the `select_*` / `jump_*` methods drive the cursor.
pub struct PriorDiscussionsState {
    /// Story whose URL was queried. Checked against the currently selected
    /// story so stale results from a prior selection are dropped.
    pub story_id: u64,
    /// Prior HN submissions of `story_id`'s URL, in Algolia default order
    /// (most recent first). May be empty when the URL has no prior submissions.
    pub submissions: Vec<Item>,
    /// Cursor into `submissions`. Clamps at `submissions.len() - 1`.
    pub selected: usize,
}

impl PriorDiscussionsState {
    /// Starts a fresh overlay positioned at the first entry.
    pub fn new(story_id: u64, submissions: Vec<Item>) -> Self {
        Self {
            story_id,
            submissions,
            selected: 0,
        }
    }

    /// Advances selection by one, saturating at the last entry. No-op when
    /// `submissions` is empty.
    pub fn select_next(&mut self) {
        if !self.submissions.is_empty() {
            self.selected = (self.selected + 1).min(self.submissions.len() - 1);
        }
    }

    /// Moves selection up one row, clamping at zero.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Jumps selection to the first entry.
    pub fn jump_top(&mut self) {
        self.selected = 0;
    }

    /// Jumps selection to the last entry. No-op when `submissions` is empty.
    pub fn jump_bottom(&mut self) {
        if !self.submissions.is_empty() {
            self.selected = self.submissions.len() - 1;
        }
    }

    /// Returns the currently-selected submission, if any.
    pub fn selected_submission(&self) -> Option<&Item> {
        self.submissions.get(self.selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: u64) -> Item {
        Item {
            id,
            title: Some(format!("Story {}", id)),
            url: Some(format!("https://example.com/{}", id)),
            text: None,
            by: Some("user".into()),
            score: Some(100),
            time: Some(1000),
            kids: None,
            descendants: Some(10),
            item_type: Some(ItemType::Story),
            dead: None,
            deleted: None,
        }
    }

    #[test]
    fn new_defaults_selected_to_zero() {
        let s = PriorDiscussionsState::new(1, vec![item(1), item(2)]);
        assert_eq!(s.selected, 0);
        assert_eq!(s.story_id, 1);
        assert_eq!(s.submissions.len(), 2);
    }

    #[test]
    fn select_next_clamps_at_end() {
        let mut s = PriorDiscussionsState::new(1, vec![item(1), item(2)]);
        s.select_next();
        s.select_next();
        s.select_next();
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn select_prev_clamps_at_zero() {
        let mut s = PriorDiscussionsState::new(1, vec![item(1), item(2)]);
        s.select_prev();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn empty_submissions_nav_is_noop() {
        let mut s = PriorDiscussionsState::new(1, Vec::new());
        s.select_next();
        s.jump_bottom();
        assert_eq!(s.selected, 0);
        assert!(s.selected_submission().is_none());
    }

    #[test]
    fn jump_bottom_selects_last() {
        let mut s = PriorDiscussionsState::new(1, vec![item(1), item(2), item(3)]);
        s.jump_bottom();
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn selected_submission_returns_current() {
        let mut s = PriorDiscussionsState::new(1, vec![item(10), item(20)]);
        assert_eq!(s.selected_submission().unwrap().id, 10);
        s.select_next();
        assert_eq!(s.selected_submission().unwrap().id, 20);
    }
}
