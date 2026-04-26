//! State for the left-hand story list pane.
//!
//! Holds the loaded [`Item`]s, the full ID list from the initial feed
//! fetch (used to compute stable pagination offsets), selection index,
//! and the loading flag. [`StoryListState::needs_more`] drives lazy
//! pagination when the cursor approaches the end.

use crate::api::types::Item;

/// State for the story-list pane.
///
/// `stories` is the currently loaded window; `all_ids` is the full ID list
/// from the initial feed fetch, used as a stable index for pagination so
/// appended pages don't drift when new stories are posted mid-session.
///
/// `domains` is a parallel cache of `Item::domain()` results, populated
/// once when stories load so per-frame rendering doesn't re-parse URLs.
/// Mutate `stories` only through [`Self::replace_stories`] /
/// [`Self::append_stories`] so the cache stays in sync.
#[derive(Default)]
pub struct StoryListState {
    pub stories: Vec<Item>,
    /// Pre-computed `story.domain()` for each story at the same index.
    /// Avoids the per-frame `url::Url::parse` that
    /// [`StoryList`](crate::ui::story_list::StoryList) used to do.
    pub domains: Vec<Option<String>>,
    pub all_ids: Vec<u64>,
    pub selected: usize,
    pub loading: bool,
}

impl StoryListState {
    /// Constructs an empty state with no loaded stories, no cached IDs,
    /// selection at zero, and not loading.
    pub fn new() -> Self {
        Self::default()
    }

    /// Replaces the loaded stories and refreshes the parallel domain
    /// cache. Use instead of `state.stories = ...` so per-frame rendering
    /// can keep using the cache.
    pub fn replace_stories(&mut self, stories: Vec<Item>) {
        self.domains = stories.iter().map(Item::domain).collect();
        self.stories = stories;
    }

    /// Appends a paginated batch and extends the parallel domain cache.
    pub fn append_stories(&mut self, stories: Vec<Item>) {
        self.domains.extend(stories.iter().map(Item::domain));
        self.stories.extend(stories);
    }

    /// Advances the cursor by one row, saturating at the last loaded
    /// story. No-op when `stories` is empty.
    pub fn select_next(&mut self) {
        if !self.stories.is_empty() {
            self.selected = (self.selected + 1).min(self.stories.len() - 1);
        }
    }

    /// Moves the cursor up by one row, saturating at zero.
    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Jumps the cursor to the first row.
    pub fn jump_top(&mut self) {
        self.selected = 0;
    }

    /// Jumps the cursor to the last loaded story. No-op when `stories` is
    /// empty.
    pub fn jump_bottom(&mut self) {
        if !self.stories.is_empty() {
            self.selected = self.stories.len() - 1;
        }
    }

    /// Advances the cursor by `page_size` rows, saturating at the last
    /// loaded story. No-op when `stories` is empty.
    pub fn page_down(&mut self, page_size: usize) {
        if !self.stories.is_empty() {
            self.selected = (self.selected + page_size).min(self.stories.len() - 1);
        }
    }

    /// Moves the cursor up by `page_size` rows, saturating at zero.
    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
    }

    #[must_use]
    pub fn selected_story(&self) -> Option<&Item> {
        self.stories.get(self.selected)
    }

    /// Whether the selected story is within 80% of the loaded window and
    /// more IDs remain to be fetched — signals lazy-pagination time.
    #[must_use]
    pub fn needs_more(&self) -> bool {
        // Load more when within 80% of loaded stories
        if self.stories.is_empty() {
            return false;
        }
        let threshold = (self.stories.len() as f64 * 0.8) as usize;
        self.selected >= threshold && self.stories.len() < self.all_ids.len()
    }

    /// Clears the loaded stories, the parallel domain cache, the cached
    /// ID list, and resets selection and the loading flag. Used on feed
    /// switch and refresh.
    pub fn reset(&mut self) {
        self.stories.clear();
        self.domains.clear();
        self.all_ids.clear();
        self.selected = 0;
        self.loading = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(id: u64) -> Item {
        Item {
            id,
            title: Some(format!("Story {}", id)),
            url: None,
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: None,
            dead: None,
            deleted: None,
        }
    }

    fn state_with_stories(n: usize) -> StoryListState {
        let mut s = StoryListState::new();
        for i in 0..n {
            s.stories.push(make_item(i as u64));
        }
        s
    }

    #[test]
    fn new_defaults() {
        let s = StoryListState::new();
        assert!(s.stories.is_empty());
        assert!(s.all_ids.is_empty());
        assert_eq!(s.selected, 0);
        assert!(!s.loading);
    }

    #[test]
    fn select_next_increments() {
        let mut s = state_with_stories(5);
        s.select_next();
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn select_next_clamps_at_end() {
        let mut s = state_with_stories(3);
        s.selected = 2;
        s.select_next();
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn select_next_empty_noop() {
        let mut s = StoryListState::new();
        s.select_next();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn select_prev_decrements() {
        let mut s = state_with_stories(5);
        s.selected = 3;
        s.select_prev();
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn select_prev_clamps_at_zero() {
        let mut s = state_with_stories(5);
        s.select_prev();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn jump_top() {
        let mut s = state_with_stories(10);
        s.selected = 7;
        s.jump_top();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn jump_bottom() {
        let mut s = state_with_stories(10);
        s.jump_bottom();
        assert_eq!(s.selected, 9);
    }

    #[test]
    fn jump_bottom_empty() {
        let mut s = StoryListState::new();
        s.jump_bottom();
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn page_down() {
        let mut s = state_with_stories(20);
        s.page_down(5);
        assert_eq!(s.selected, 5);
    }

    #[test]
    fn page_down_clamps() {
        let mut s = state_with_stories(10);
        s.selected = 7;
        s.page_down(5);
        assert_eq!(s.selected, 9);
    }

    #[test]
    fn page_down_empty() {
        let mut s = StoryListState::new();
        s.page_down(5);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn page_up() {
        let mut s = state_with_stories(20);
        s.selected = 10;
        s.page_up(5);
        assert_eq!(s.selected, 5);
    }

    #[test]
    fn page_up_clamps_at_zero() {
        let mut s = state_with_stories(10);
        s.selected = 2;
        s.page_up(5);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn page_up_empty() {
        let mut s = StoryListState::new();
        s.page_up(5);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn selected_story_returns_item() {
        let s = state_with_stories(3);
        assert_eq!(s.selected_story().unwrap().id, 0);
    }

    #[test]
    fn selected_story_empty() {
        let s = StoryListState::new();
        assert!(s.selected_story().is_none());
    }

    #[test]
    fn needs_more_below_threshold() {
        let mut s = state_with_stories(10);
        s.all_ids = (0..50).collect();
        s.selected = 2;
        assert!(!s.needs_more());
    }

    #[test]
    fn needs_more_at_threshold() {
        let mut s = state_with_stories(10);
        s.all_ids = (0..50).collect();
        s.selected = 8; // 80% of 10
        assert!(s.needs_more());
    }

    #[test]
    fn needs_more_all_loaded() {
        let mut s = state_with_stories(10);
        s.all_ids = (0..10).collect();
        s.selected = 9;
        assert!(!s.needs_more());
    }

    #[test]
    fn needs_more_empty() {
        let s = StoryListState::new();
        assert!(!s.needs_more());
    }

    #[test]
    fn reset_clears_all() {
        let mut s = state_with_stories(5);
        s.all_ids = vec![1, 2, 3];
        s.selected = 3;
        s.loading = true;
        s.reset();
        assert!(s.stories.is_empty());
        assert!(s.all_ids.is_empty());
        assert_eq!(s.selected, 0);
        assert!(!s.loading);
    }
}
