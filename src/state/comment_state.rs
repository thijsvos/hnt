use crate::api::types::Item;
use std::collections::HashSet;

pub struct FlatComment {
    pub item: Item,
    pub depth: usize,
    /// Cached plain-text rendering of `item.text` at the given width.
    /// Populated on first render; invalidated by a width change.
    plain_text_cache: Option<(usize, String)>,
}

impl FlatComment {
    pub fn new(item: Item, depth: usize) -> Self {
        Self {
            item,
            depth,
            plain_text_cache: None,
        }
    }

    /// Returns the plain-text rendering of `item.text` at `width`, reusing
    /// the last-rendered result if the width matches.
    pub fn plain_text(&mut self, width: usize) -> Option<&str> {
        let text = self.item.text.as_deref()?;
        let needs_refresh = !matches!(&self.plain_text_cache, Some((w, _)) if *w == width);
        if needs_refresh {
            let rendered = html2text::from_read(text.as_bytes(), width.max(20)).unwrap_or_default();
            self.plain_text_cache = Some((width, rendered));
        }
        self.plain_text_cache.as_ref().map(|(_, s)| s.as_str())
    }
}

pub struct CommentTreeState {
    pub comments: Vec<FlatComment>,
    /// Row-based scroll offset, updated by the renderer.
    pub scroll: usize,
    pub selected: usize,
    pub collapsed: HashSet<u64>,
    pub loading: bool,
    pub story: Option<Item>,
    /// Root-comment IDs whose subtrees are still being fetched.
    pub pending_root_ids: HashSet<u64>,
    /// Maps screen row (relative to inner area top) → visible comment index.
    /// Populated during render for mouse click handling.
    pub row_map: Vec<Option<usize>>,
    /// Cached plain-text rendering of the current story's text (id, width, text).
    /// Invalidated automatically when story id or width changes.
    story_text_cache: Option<(u64, usize, String)>,
}

impl CommentTreeState {
    pub fn new() -> Self {
        Self {
            comments: Vec::new(),
            scroll: 0,
            selected: 0,
            collapsed: HashSet::new(),
            loading: false,
            story: None,
            pending_root_ids: HashSet::new(),
            row_map: Vec::new(),
            story_text_cache: None,
        }
    }

    /// Return the plain-text rendering of the current story's text at `width`,
    /// reusing the last-rendered result if the story and width match.
    pub fn story_plain_text(&mut self, width: usize) -> Option<&str> {
        let story = self.story.as_ref()?;
        let text = story.text.as_deref()?;
        let id = story.id;
        let needs_refresh =
            !matches!(&self.story_text_cache, Some((cid, w, _)) if *cid == id && *w == width);
        if needs_refresh {
            let rendered = html2text::from_read(text.as_bytes(), width.max(20)).unwrap_or_default();
            self.story_text_cache = Some((id, width, rendered));
        }
        self.story_text_cache.as_ref().map(|(_, _, s)| s.as_str())
    }

    pub fn set_comments(&mut self, items: Vec<(Item, usize)>) {
        self.comments = items
            .into_iter()
            .map(|(item, depth)| FlatComment::new(item, depth))
            .collect();
        self.scroll = 0;
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
                .map(|(item, depth)| FlatComment::new(item, depth))
                .collect();
            // Splice them in after the parent
            self.comments.splice(pos..pos, new_comments);
        }
    }

    /// Walk the comment tree, skipping subtrees rooted at a collapsed comment.
    /// Returns the indices (into `self.comments`) that should be shown.
    pub fn visible_indices(&self) -> Vec<usize> {
        let mut indices = Vec::with_capacity(self.comments.len());
        let mut skip_depth: Option<usize> = None;
        for (i, comment) in self.comments.iter().enumerate() {
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
            indices.push(i);
        }
        indices
    }

    pub fn visible_comments(&self) -> Vec<&FlatComment> {
        self.visible_indices()
            .into_iter()
            .map(|i| &self.comments[i])
            .collect()
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
        self.scroll = 0;
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
        self.scroll = 0;
        self.selected = 0;
        self.collapsed.clear();
        self.loading = false;
        self.story = None;
        self.pending_root_ids.clear();
        self.row_map.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item(id: u64) -> Item {
        Item {
            id,
            title: None,
            url: None,
            text: Some(format!("Comment {}", id)),
            by: Some("user".into()),
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: Some("comment".into()),
            dead: None,
            deleted: None,
        }
    }

    /// Build a simple tree: root(1) -> child(2, depth 1) -> grandchild(3, depth 2), sibling(4, depth 0)
    fn sample_tree() -> Vec<(Item, usize)> {
        vec![
            (make_item(1), 0),
            (make_item(2), 1),
            (make_item(3), 2),
            (make_item(4), 0),
        ]
    }

    #[test]
    fn set_comments_populates() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        assert_eq!(state.comments.len(), 4);
        assert_eq!(state.comments[0].depth, 0);
        assert_eq!(state.comments[1].depth, 1);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn set_comments_resets_scroll() {
        let mut state = CommentTreeState::new();
        state.scroll = 10;
        state.selected = 5;
        state.set_comments(sample_tree());
        assert_eq!(state.scroll, 0);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn insert_children_after_parent() {
        let mut state = CommentTreeState::new();
        state.set_comments(vec![(make_item(1), 0), (make_item(4), 0)]);
        state.insert_children(1, vec![(make_item(2), 1), (make_item(3), 1)]);
        assert_eq!(state.comments.len(), 4);
        assert_eq!(state.comments[1].item.id, 2);
        assert_eq!(state.comments[2].item.id, 3);
        assert_eq!(state.comments[3].item.id, 4);
    }

    #[test]
    fn insert_children_missing_parent_noop() {
        let mut state = CommentTreeState::new();
        state.set_comments(vec![(make_item(1), 0)]);
        state.insert_children(999, vec![(make_item(2), 1)]);
        assert_eq!(state.comments.len(), 1);
    }

    #[test]
    fn visible_comments_no_collapse() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        assert_eq!(state.visible_comments().len(), 4);
    }

    #[test]
    fn visible_comments_collapse_root_hides_children() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(1); // collapse root comment
        let visible = state.visible_comments();
        let ids: Vec<u64> = visible.iter().map(|c| c.item.id).collect();
        // Root 1 visible (but collapsed), children 2,3 hidden, sibling 4 visible
        assert_eq!(ids, vec![1, 4]);
    }

    #[test]
    fn visible_comments_collapse_mid_level() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(2); // collapse child at depth 1
        let visible = state.visible_comments();
        let ids: Vec<u64> = visible.iter().map(|c| c.item.id).collect();
        // 1 visible, 2 visible (collapsed), 3 hidden (child of 2), 4 visible
        assert_eq!(ids, vec![1, 2, 4]);
    }

    #[test]
    fn visible_comments_empty() {
        let state = CommentTreeState::new();
        assert!(state.visible_comments().is_empty());
    }

    #[test]
    fn select_next_increments() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.select_next();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn select_next_clamps_at_end() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.selected = 3;
        state.select_next();
        assert_eq!(state.selected, 3);
    }

    #[test]
    fn select_next_empty() {
        let mut state = CommentTreeState::new();
        state.select_next();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn select_prev_decrements() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.selected = 2;
        state.select_prev();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn select_prev_clamps_at_zero() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.select_prev();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn jump_top_resets() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.selected = 3;
        state.scroll = 5;
        state.jump_top();
        assert_eq!(state.selected, 0);
        assert_eq!(state.scroll, 0);
    }

    #[test]
    fn jump_bottom_selects_last() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.jump_bottom();
        assert_eq!(state.selected, 3);
    }

    #[test]
    fn page_down_moves_by_page() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.page_down(2);
        assert_eq!(state.selected, 2);
    }

    #[test]
    fn page_up_moves_by_page() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.selected = 3;
        state.page_up(2);
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn select_next_clamps_at_visible_end_when_collapsed() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(1); // visible: [1, 4]
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 1); // clamped at visible len - 1
    }

    #[test]
    fn jump_bottom_respects_collapse() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(1); // visible: [1, 4]
        state.jump_bottom();
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn jump_bottom_empty() {
        let mut state = CommentTreeState::new();
        state.jump_bottom();
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn page_down_clamps_at_end() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.selected = 2;
        state.page_down(5);
        assert_eq!(state.selected, 3);
    }

    #[test]
    fn page_down_empty() {
        let mut state = CommentTreeState::new();
        state.page_down(5);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn page_up_clamps_at_zero() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.selected = 1;
        state.page_up(5);
        assert_eq!(state.selected, 0);
    }

    #[test]
    fn page_down_respects_collapse() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(1); // visible: [1, 4]
        state.page_down(5);
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn toggle_collapse_adds_to_set() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.toggle_collapse();
        assert!(state.collapsed.contains(&1));
    }

    #[test]
    fn toggle_collapse_removes_from_set() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(1);
        state.toggle_collapse();
        assert!(!state.collapsed.contains(&1));
    }

    #[test]
    fn toggle_collapse_empty_noop() {
        let mut state = CommentTreeState::new();
        state.toggle_collapse(); // should not panic
        assert!(state.collapsed.is_empty());
    }

    #[test]
    fn reset_clears_all() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(1);
        state.selected = 2;
        state.loading = true;
        state.story = Some(make_item(99));
        state.pending_root_ids.insert(1);
        state.reset();
        assert!(state.comments.is_empty());
        assert_eq!(state.scroll, 0);
        assert_eq!(state.selected, 0);
        assert!(state.collapsed.is_empty());
        assert!(!state.loading);
        assert!(state.story.is_none());
        assert!(state.pending_root_ids.is_empty());
    }

    #[test]
    fn pending_root_ids_lifecycle() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        // Populate — simulating app.rs inserting after CommentsLoaded
        state.pending_root_ids.insert(1);
        state.pending_root_ids.insert(4);
        assert_eq!(state.pending_root_ids.len(), 2);

        // CommentsAppended for root 1 → children arrive, remove from pending
        state.insert_children(1, vec![(make_item(10), 1)]);
        state.pending_root_ids.remove(&1);
        assert!(!state.pending_root_ids.contains(&1));
        assert!(state.pending_root_ids.contains(&4));

        // CommentsDone → clear remaining
        state.pending_root_ids.clear();
        assert!(state.pending_root_ids.is_empty());
    }
}
