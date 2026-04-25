//! Comment-tree state: a flattened list with depth, collapse tracking,
//! and an incremental-insertion API for progressive loads.
//!
//! Comments are stored as a pre-order flat [`Vec`] of [`FlatComment`]
//! (item + depth); [`CommentTreeState::visible_indices`] applies collapse
//! rules by skipping subtrees. [`CommentTreeState::insert_children`]
//! splices a root's children in-place as async fetches complete.

#[cfg(test)]
use crate::api::types::ItemType;
use crate::api::types::{CommentId, CommentWithDepth, Item};
use std::collections::HashSet;

/// One comment in the flattened, depth-tagged comment tree.
///
/// `depth == 0` is a root-level comment; children have strictly greater
/// depth and are stored contiguously after their parent in pre-order.
pub struct FlatComment {
    pub item: Item,
    pub depth: usize,
    /// Cached plain-text rendering of `item.text` at the given width.
    /// Populated on first render; invalidated by a width change.
    plain_text_cache: Option<(usize, String)>,
}

impl FlatComment {
    /// Wraps an [`Item`] at the given tree depth with an empty text cache.
    pub fn new(item: Item, depth: usize) -> Self {
        Self {
            item,
            depth,
            plain_text_cache: None,
        }
    }

    /// Returns the plain-text rendering of `item.text` at `width`, reusing
    /// the last-rendered result if the width matches. The cache is keyed
    /// only on width because each `FlatComment` wraps a single `Item`.
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

/// State for the comments pane: flattened tree, collapse set, selection,
/// and render-populated scroll/row-map.
///
/// [`CommentTreeState::set_comments`] replaces the tree;
/// [`CommentTreeState::insert_children`] splices in subtrees as progressive
/// loads complete.
#[derive(Default)]
pub struct CommentTreeState {
    pub comments: Vec<FlatComment>,
    /// Row-based scroll offset, updated by the renderer.
    pub scroll: usize,
    /// Index into `visible_comments()`, not into `comments`.
    pub selected: usize,
    /// Collapsed-subtree comment IDs; their descendants are hidden from
    /// `visible_comments()`.
    pub collapsed: HashSet<CommentId>,
    pub loading: bool,
    pub story: Option<Item>,
    /// Root-comment IDs whose subtrees are still being fetched.
    pub pending_root_ids: HashSet<CommentId>,
    /// Maps screen row (relative to inner area top) → visible comment index.
    /// Populated during render for mouse click handling.
    pub row_map: Vec<Option<usize>>,
    /// Cached plain-text rendering of the current story's text (id, width, text).
    /// Invalidated automatically when story id or width changes.
    story_text_cache: Option<(u64, usize, String)>,
}

impl CommentTreeState {
    /// Constructs an empty state with no loaded story and no pending fetches.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the plain-text rendering of the current story's text at
    /// `width`, reusing the last-rendered result if the story and width
    /// match.
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

    /// Replaces the flat list and resets selection/scroll to the top.
    /// `items` must be in pre-order (parents before their descendants).
    pub fn set_comments(&mut self, items: Vec<CommentWithDepth>) {
        self.comments = items
            .into_iter()
            .map(|c| FlatComment::new(c.item, c.depth))
            .collect();
        self.scroll = 0;
        self.selected = 0;
    }

    /// Insert child comments right after their parent in the flattened list.
    pub fn insert_children(&mut self, parent_id: CommentId, children: Vec<CommentWithDepth>) {
        let insert_pos = self
            .comments
            .iter()
            .position(|c| c.item.id == parent_id.0)
            .map(|i| i + 1);

        if let Some(pos) = insert_pos {
            let new_comments: Vec<FlatComment> = children
                .into_iter()
                .map(|c| FlatComment::new(c.item, c.depth))
                .collect();
            // Splice them in after the parent
            self.comments.splice(pos..pos, new_comments);
        }
    }

    /// Walks the comment tree, skipping subtrees rooted at a collapsed
    /// comment, and yields the indices (into `self.comments`) that should
    /// be shown. Allocation-free — prefer this for `.count()` /
    /// `.nth(...)` over the `Vec`-returning [`Self::visible_indices`].
    pub fn visible_indices_iter(&self) -> impl Iterator<Item = usize> + '_ {
        let mut skip_depth: Option<usize> = None;
        self.comments
            .iter()
            .enumerate()
            .filter_map(move |(i, comment)| {
                if let Some(sd) = skip_depth {
                    if comment.depth > sd {
                        return None;
                    }
                    skip_depth = None;
                }
                if self.collapsed.contains(&CommentId(comment.item.id)) {
                    skip_depth = Some(comment.depth);
                }
                Some(i)
            })
    }

    /// `Vec`-backed form of [`Self::visible_indices_iter`] — used by the
    /// renderer which needs to index the list as `&[usize]` for scroll
    /// calculations.
    #[must_use]
    pub fn visible_indices(&self) -> Vec<usize> {
        self.visible_indices_iter().collect()
    }

    /// Count of currently-visible comments. Replaces a `Vec`-allocating
    /// `visible_comments().len()` in navigation hot paths (every
    /// keystroke).
    #[must_use]
    pub fn visible_len(&self) -> usize {
        self.visible_indices_iter().count()
    }

    /// Resolves [`Self::visible_indices`] into borrowed comment
    /// references. Test-only — production code should prefer
    /// [`Self::visible_indices_iter`] (no allocation) or
    /// [`Self::visible_len`] (just the count).
    #[cfg(test)]
    pub fn visible_comments(&self) -> Vec<&FlatComment> {
        self.visible_indices_iter()
            .map(|i| &self.comments[i])
            .collect()
    }

    pub fn select_next(&mut self) {
        let len = self.visible_len();
        if len > 0 {
            self.selected = (self.selected + 1).min(len - 1);
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
        let len = self.visible_len();
        if len > 0 {
            self.selected = len - 1;
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        let len = self.visible_len();
        if len > 0 {
            self.selected = (self.selected + page_size).min(len - 1);
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        self.selected = self.selected.saturating_sub(page_size);
    }

    /// Flips the collapse state of the currently selected comment.
    /// Collapsing hides the subtree on the next `visible_indices` call.
    pub fn toggle_collapse(&mut self) {
        let Some(idx) = self.visible_indices_iter().nth(self.selected) else {
            return;
        };
        let id = CommentId(self.comments[idx].item.id);
        // Single-lookup toggle: `HashSet::remove` returns whether the key
        // was present, so we can skip the explicit `contains` probe.
        if !self.collapsed.remove(&id) {
            self.collapsed.insert(id);
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
            item_type: Some(ItemType::Comment),
            dead: None,
            deleted: None,
        }
    }

    /// Build a simple tree: root(1) -> child(2, depth 1) -> grandchild(3, depth 2), sibling(4, depth 0)
    fn sample_tree() -> Vec<CommentWithDepth> {
        vec![
            CommentWithDepth {
                item: make_item(1),
                depth: 0,
            },
            CommentWithDepth {
                item: make_item(2),
                depth: 1,
            },
            CommentWithDepth {
                item: make_item(3),
                depth: 2,
            },
            CommentWithDepth {
                item: make_item(4),
                depth: 0,
            },
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

    fn cwd(id: u64, depth: usize) -> CommentWithDepth {
        CommentWithDepth {
            item: make_item(id),
            depth,
        }
    }

    fn cid(n: u64) -> CommentId {
        CommentId(n)
    }

    #[test]
    fn insert_children_after_parent() {
        let mut state = CommentTreeState::new();
        state.set_comments(vec![cwd(1, 0), cwd(4, 0)]);
        state.insert_children(cid(1), vec![cwd(2, 1), cwd(3, 1)]);
        assert_eq!(state.comments.len(), 4);
        assert_eq!(state.comments[1].item.id, 2);
        assert_eq!(state.comments[2].item.id, 3);
        assert_eq!(state.comments[3].item.id, 4);
    }

    #[test]
    fn insert_children_missing_parent_noop() {
        let mut state = CommentTreeState::new();
        state.set_comments(vec![cwd(1, 0)]);
        state.insert_children(cid(999), vec![cwd(2, 1)]);
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
        state.collapsed.insert(cid(1)); // collapse root comment
        let visible = state.visible_comments();
        let ids: Vec<u64> = visible.iter().map(|c| c.item.id).collect();
        // Root 1 visible (but collapsed), children 2,3 hidden, sibling 4 visible
        assert_eq!(ids, vec![1, 4]);
    }

    #[test]
    fn visible_comments_collapse_mid_level() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(cid(2)); // collapse child at depth 1
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
        state.collapsed.insert(cid(1)); // visible: [1, 4] still works below
        state.select_next();
        assert_eq!(state.selected, 1);
        state.select_next();
        assert_eq!(state.selected, 1); // clamped at visible len - 1
    }

    #[test]
    fn jump_bottom_respects_collapse() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(cid(1)); // visible: [1, 4] still works below
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
        state.collapsed.insert(cid(1)); // visible: [1, 4] still works below
        state.page_down(5);
        assert_eq!(state.selected, 1);
    }

    #[test]
    fn toggle_collapse_adds_to_set() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.toggle_collapse();
        assert!(state.collapsed.contains(&cid(1)));
    }

    #[test]
    fn toggle_collapse_removes_from_set() {
        let mut state = CommentTreeState::new();
        state.set_comments(sample_tree());
        state.collapsed.insert(cid(1));
        state.toggle_collapse();
        assert!(!state.collapsed.contains(&cid(1)));
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
        state.collapsed.insert(cid(1));
        state.selected = 2;
        state.loading = true;
        state.story = Some(make_item(99));
        state.pending_root_ids.insert(cid(1));
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
        state.pending_root_ids.insert(cid(1));
        state.pending_root_ids.insert(cid(4));
        assert_eq!(state.pending_root_ids.len(), 2);

        // CommentsAppended for root 1 → children arrive, remove from pending
        state.insert_children(cid(1), vec![cwd(10, 1)]);
        state.pending_root_ids.remove(&cid(1));
        assert!(!state.pending_root_ids.contains(&cid(1)));
        assert!(state.pending_root_ids.contains(&cid(4)));

        // CommentsDone → clear remaining
        state.pending_root_ids.clear();
        assert!(state.pending_root_ids.is_empty());
    }
}
