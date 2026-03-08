use crate::api::types::Item;
use std::cell::{Cell, RefCell};
use std::collections::HashSet;

pub struct FlatComment {
    pub item: Item,
    pub depth: usize,
}

pub struct CommentTreeState {
    pub comments: Vec<FlatComment>,
    /// Row-based scroll offset, updated by the renderer via interior mutability.
    pub scroll: Cell<usize>,
    pub selected: usize,
    pub collapsed: HashSet<u64>,
    pub loading: bool,
    pub story: Option<Item>,
    /// Maps screen row (relative to inner area top) → visible comment index.
    /// Populated during render for mouse click handling.
    pub row_map: RefCell<Vec<Option<usize>>>,
}

impl CommentTreeState {
    pub fn new() -> Self {
        Self {
            comments: Vec::new(),
            scroll: Cell::new(0),
            selected: 0,
            collapsed: HashSet::new(),
            loading: false,
            story: None,
            row_map: RefCell::new(Vec::new()),
        }
    }

    pub fn set_comments(&mut self, items: Vec<(Item, usize)>) {
        self.comments = items
            .into_iter()
            .map(|(item, depth)| FlatComment { item, depth })
            .collect();
        self.scroll.set(0);
        self.selected = 0;
    }

    /// Insert child comments right after their parent in the flattened list.
    pub fn insert_children(&mut self, parent_id: u64, children: Vec<(Item, usize)>) {
        let insert_pos = self
            .comments
            .iter()
            .position(|c| c.item.id == parent_id)
            .map(|i| i + 1);

        if let Some(pos) = insert_pos {
            let new_comments: Vec<FlatComment> = children
                .into_iter()
                .map(|(item, depth)| FlatComment { item, depth })
                .collect();
            // Splice them in after the parent
            self.comments.splice(pos..pos, new_comments);
        }
    }

    pub fn visible_comments(&self) -> Vec<&FlatComment> {
        let mut result = Vec::new();
        let mut skip_depth: Option<usize> = None;

        for comment in &self.comments {
            if let Some(sd) = skip_depth {
                if comment.depth > sd {
                    continue;
                } else {
                    skip_depth = None;
                }
            }

            if self.collapsed.contains(&comment.item.id) {
                skip_depth = Some(comment.depth);
            }

            result.push(comment);
        }

        result
    }

    pub fn select_next(&mut self) {
        let visible_len = self.visible_comments().len();
        if visible_len > 0 {
            self.selected = (self.selected + 1).min(visible_len - 1);
        }
    }

    pub fn select_prev(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn jump_top(&mut self) {
        self.selected = 0;
        self.scroll.set(0);
    }

    pub fn jump_bottom(&mut self) {
        let visible_len = self.visible_comments().len();
        if visible_len > 0 {
            self.selected = visible_len - 1;
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        let visible_len = self.visible_comments().len();
        if visible_len > 0 {
            self.selected = (self.selected + page_size).min(visible_len - 1);
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
    }

    pub fn toggle_collapse(&mut self) {
        let visible = self.visible_comments();
        if let Some(comment) = visible.get(self.selected) {
            let id = comment.item.id;
            if self.collapsed.contains(&id) {
                self.collapsed.remove(&id);
            } else {
                self.collapsed.insert(id);
            }
        }
    }

    pub fn reset(&mut self) {
        self.comments.clear();
        self.scroll.set(0);
        self.selected = 0;
        self.collapsed.clear();
        self.loading = false;
        self.story = None;
        self.row_map.borrow_mut().clear();
    }
}
