use crate::api::types::Item;

pub struct StoryListState {
    pub stories: Vec<Item>,
    pub all_ids: Vec<u64>,
    pub selected: usize,
    pub offset: usize,
    pub loading: bool,
}

impl StoryListState {
    pub fn new() -> Self {
        Self {
            stories: Vec::new(),
            all_ids: Vec::new(),
            selected: 0,
            offset: 0,
            loading: false,
        }
    }

    pub fn select_next(&mut self) {
        if !self.stories.is_empty() {
            self.selected = (self.selected + 1).min(self.stories.len() - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn jump_top(&mut self) {
        self.selected = 0;
    }

    pub fn jump_bottom(&mut self) {
        if !self.stories.is_empty() {
            self.selected = self.stories.len() - 1;
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        if !self.stories.is_empty() {
            self.selected = (self.selected + page_size).min(self.stories.len() - 1);
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
    }

    pub fn selected_story(&self) -> Option<&Item> {
        self.stories.get(self.selected)
    }

    pub fn needs_more(&self) -> bool {
        // Load more when within 80% of loaded stories
        if self.stories.is_empty() {
            return false;
        }
        let threshold = (self.stories.len() as f64 * 0.8) as usize;
        self.selected >= threshold && self.stories.len() < self.all_ids.len()
    }

    pub fn reset(&mut self) {
        self.stories.clear();
        self.all_ids.clear();
        self.selected = 0;
        self.offset = 0;
        self.loading = false;
    }
}
