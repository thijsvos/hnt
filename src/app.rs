use crate::api::client::HnClient;
use crate::api::types::{FeedKind, Item};
use crate::keys::Action;
use crate::state::comment_state::CommentTreeState;
use crate::state::story_state::StoryListState;
use tokio::sync::mpsc;

const MIN_PAGE_SIZE: usize = 30;
const SCROLL_PAGE: usize = 10;
const MAX_COMMENT_DEPTH: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Stories,
    Comments,
}

/// Messages sent from async tasks back to the main loop.
pub enum AppMessage {
    StoriesLoaded {
        stories: Vec<Item>,
        all_ids: Vec<u64>,
        append: bool,
    },
    CommentsLoaded {
        story: Item,
        comments: Vec<(Item, usize)>,
    },
    /// Progressive update — append more child comments into the tree.
    CommentsAppended {
        parent_id: u64,
        children: Vec<(Item, usize)>,
    },
    CommentsDone,
    Error(String),
}

pub struct App {
    pub running: bool,
    pub current_feed: FeedKind,
    pub focus: Pane,
    pub story_state: StoryListState,
    pub comment_state: CommentTreeState,
    pub show_help: bool,
    pub error: Option<String>,
    pub terminal_height: u16,
    pub terminal_width: u16,

    last_comment_click: Option<(std::time::Instant, usize)>,

    client: HnClient,
    msg_tx: mpsc::UnboundedSender<AppMessage>,
    msg_rx: mpsc::UnboundedReceiver<AppMessage>,
}

impl App {
    pub fn new(terminal_width: u16, terminal_height: u16) -> Self {
        let (msg_tx, msg_rx) = mpsc::unbounded_channel();
        Self {
            running: true,
            current_feed: FeedKind::Top,
            focus: Pane::Stories,
            story_state: StoryListState::new(),
            comment_state: CommentTreeState::new(),
            show_help: false,
            error: None,
            terminal_height,
            terminal_width,
            last_comment_click: None,
            client: HnClient::new(),
            msg_tx,
            msg_rx,
        }
    }

    /// How many stories to fetch per page — enough to fill the screen.
    fn page_size(&self) -> usize {
        // terminal_height minus header(1), status(1), borders(2) = usable rows
        let visible = self.terminal_height.saturating_sub(4) as usize;
        visible.max(MIN_PAGE_SIZE)
    }

    pub fn set_terminal_size(&mut self, w: u16, h: u16) {
        self.terminal_width = w;
        self.terminal_height = h;
    }

    /// Initial load on startup.
    pub fn load_initial_feed(&self) {
        self.spawn_load_stories(false);
    }

    /// Process any pending async messages (non-blocking).
    pub fn process_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                AppMessage::StoriesLoaded {
                    stories,
                    all_ids,
                    append,
                } => {
                    if append {
                        self.story_state.stories.extend(stories);
                    } else {
                        self.story_state.stories = stories;
                    }
                    self.story_state.all_ids = all_ids;
                    self.story_state.loading = false;
                    self.error = None;
                    // Auto-load comments for the first story on initial load
                    if !append && !self.story_state.stories.is_empty() && self.comment_state.story.is_none() {
                        self.load_selected_comments();
                    }
                }
                AppMessage::CommentsLoaded { story, comments } => {
                    self.comment_state.story = Some(story);
                    self.comment_state.set_comments(comments);
                    // Still loading children in background
                    self.error = None;
                }
                AppMessage::CommentsAppended { parent_id, children } => {
                    self.comment_state.insert_children(parent_id, children);
                }
                AppMessage::CommentsDone => {
                    self.comment_state.loading = false;
                }
                AppMessage::Error(e) => {
                    self.error = Some(e);
                    self.story_state.loading = false;
                    self.comment_state.loading = false;
                }
            }
        }
    }

    /// Dispatch an action from keybindings.
    pub fn dispatch(&mut self, action: Action) {
        match action {
            Action::Quit => self.running = false,
            Action::Back => {
                if self.focus == Pane::Comments && self.comment_state.story.is_some() {
                    self.comment_state.reset();
                    self.focus = Pane::Stories;
                } else {
                    self.running = false;
                }
            }
            Action::MoveDown => match self.focus {
                Pane::Stories => {
                    self.story_state.select_next();
                    self.check_lazy_load();
                }
                Pane::Comments => self.comment_state.select_next(),
            },
            Action::MoveUp => match self.focus {
                Pane::Stories => self.story_state.select_prev(),
                Pane::Comments => self.comment_state.select_prev(),
            },
            Action::Select => match self.focus {
                Pane::Stories => self.load_selected_comments(),
                Pane::Comments => self.comment_state.toggle_collapse(),
            },
            Action::OpenInBrowser => self.open_in_browser(),
            Action::SwitchPane => {
                self.focus = match self.focus {
                    Pane::Stories => Pane::Comments,
                    Pane::Comments => Pane::Stories,
                };
            }
            Action::SwitchFeed(idx) => {
                if idx < FeedKind::ALL.len() {
                    let feed = FeedKind::ALL[idx];
                    if feed != self.current_feed {
                        self.current_feed = feed;
                        self.story_state.reset();
                        self.comment_state.reset();
                        self.client.clear_cache();
                        self.focus = Pane::Stories;
                        self.spawn_load_stories(false);
                    }
                }
            }
            Action::Refresh => {
                self.story_state.reset();
                self.comment_state.reset();
                self.client.clear_cache();
                self.spawn_load_stories(false);
            }
            Action::JumpTop => match self.focus {
                Pane::Stories => self.story_state.jump_top(),
                Pane::Comments => self.comment_state.jump_top(),
            },
            Action::JumpBottom => match self.focus {
                Pane::Stories => {
                    self.story_state.jump_bottom();
                    self.check_lazy_load();
                }
                Pane::Comments => self.comment_state.jump_bottom(),
            },
            Action::PageDown => match self.focus {
                Pane::Stories => {
                    self.story_state.page_down(SCROLL_PAGE);
                    self.check_lazy_load();
                }
                Pane::Comments => self.comment_state.page_down(SCROLL_PAGE),
            },
            Action::PageUp => match self.focus {
                Pane::Stories => self.story_state.page_up(SCROLL_PAGE),
                Pane::Comments => self.comment_state.page_up(SCROLL_PAGE),
            },
            Action::ToggleHelp => self.show_help = !self.show_help,
            Action::None => {}
        }
    }

    fn spawn_load_stories(&self, append: bool) {
        let client = self.client.clone();
        let feed = self.current_feed;
        let offset = if append {
            self.story_state.stories.len()
        } else {
            0
        };
        let tx = self.msg_tx.clone();
        let page_size = self.page_size();

        tokio::spawn(async move {
            match client.fetch_stories(feed, offset, page_size).await {
                Ok((stories, all_ids)) => {
                    let _ = tx.send(AppMessage::StoriesLoaded {
                        stories,
                        all_ids,
                        append,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(format!("Failed to load stories: {}", e)));
                }
            }
        });
    }

    fn load_selected_comments(&mut self) {
        if let Some(story) = self.story_state.selected_story().cloned() {
            self.comment_state.loading = true;
            self.focus = Pane::Comments;

            let client = self.client.clone();
            let tx = self.msg_tx.clone();
            let kids = story.kids.clone().unwrap_or_default();
            let story_clone = story.clone();

            tokio::spawn(async move {
                // Step 1: Fetch root-level comments and show them immediately
                let root_items = client.fetch_items(&kids).await;
                let root_comments: Vec<(Item, usize)> = root_items
                    .into_iter()
                    .flatten()
                    .filter(|item| !item.is_dead_or_deleted())
                    .map(|item| (item, 0))
                    .collect();

                let _ = tx.send(AppMessage::CommentsLoaded {
                    story: story_clone,
                    comments: root_comments.clone(),
                });

                // Step 2: For each root comment, fetch its children progressively
                for (root, _) in &root_comments {
                    let child_ids = root.kids.clone().unwrap_or_default();
                    if child_ids.is_empty() {
                        continue;
                    }
                    let parent_id = root.id;
                    let mut children = Vec::new();
                    client
                        .fetch_children_recursive(&child_ids, 1, MAX_COMMENT_DEPTH, &mut children)
                        .await;
                    if !children.is_empty() {
                        let _ = tx.send(AppMessage::CommentsAppended {
                            parent_id,
                            children,
                        });
                    }
                }

                let _ = tx.send(AppMessage::CommentsDone);
            });
        }
    }

    fn open_in_browser(&self) {
        let url = match self.focus {
            Pane::Stories => self
                .story_state
                .selected_story()
                .and_then(|s| s.url.clone()),
            Pane::Comments => self
                .comment_state
                .story
                .as_ref()
                .and_then(|s| s.url.clone()),
        };

        // Fall back to HN item page
        let url = url.or_else(|| {
            let id = match self.focus {
                Pane::Stories => self.story_state.selected_story().map(|s| s.id),
                Pane::Comments => self.comment_state.story.as_ref().map(|s| s.id),
            };
            id.map(|id| format!("https://news.ycombinator.com/item?id={}", id))
        });

        if let Some(url) = url {
            let _ = open::that(url);
        }
    }

    fn check_lazy_load(&mut self) {
        if self.story_state.needs_more() && !self.story_state.loading {
            self.story_state.loading = true;
            self.spawn_load_stories(true);
        }
    }

    /// Handle a mouse left-click at the given terminal position.
    pub fn handle_click(&mut self, column: u16, row: u16) {
        use crate::ui::layout::build_layout;
        use ratatui::layout::Rect;
        use ratatui::widgets::{Block, Borders};

        let area = Rect::new(0, 0, self.terminal_width, self.terminal_height);
        let layout = build_layout(area);

        if layout.stories.contains(ratatui::layout::Position::new(column, row)) {
            let inner = Block::default().borders(Borders::ALL).inner(layout.stories);
            if !inner.contains(ratatui::layout::Position::new(column, row)) {
                return;
            }

            let visible_height = inner.height as usize;
            let selected = self.story_state.selected;
            let scroll = if selected >= visible_height {
                selected - visible_height + 1
            } else {
                0
            };

            let clicked_index = scroll + (row - inner.y) as usize;
            if clicked_index < self.story_state.stories.len() {
                self.story_state.selected = clicked_index;
                self.check_lazy_load();
                // Auto-load comments for the clicked story
                self.load_selected_comments();
            }
        } else if layout.comments.contains(ratatui::layout::Position::new(column, row)) {
            let inner = Block::default().borders(Borders::ALL).inner(layout.comments);
            if !inner.contains(ratatui::layout::Position::new(column, row)) {
                return;
            }

            self.focus = Pane::Comments;

            let screen_row = (row - inner.y) as usize;
            let visual_index = {
                let row_map = self.comment_state.row_map.borrow();
                row_map.get(screen_row).copied().flatten()
            };

            if let Some(vi) = visual_index {
                let visible_len = self.comment_state.visible_comments().len();
                if vi < visible_len {
                    // Check for double-click to toggle collapse
                    let now = std::time::Instant::now();
                    if let Some((last_time, last_vi)) = self.last_comment_click {
                        if last_vi == vi && now.duration_since(last_time).as_millis() < 400 {
                            self.comment_state.selected = vi;
                            self.comment_state.toggle_collapse();
                            self.last_comment_click = None;
                            return;
                        }
                    }
                    self.comment_state.selected = vi;
                    self.last_comment_click = Some((now, vi));
                }
            }
        }
    }

    /// Handle mouse scroll in the pane under the cursor.
    pub fn handle_scroll(&mut self, column: u16, row: u16, down: bool) {
        use crate::ui::layout::build_layout;
        use ratatui::layout::Rect;

        let area = Rect::new(0, 0, self.terminal_width, self.terminal_height);
        let layout = build_layout(area);

        let pane = if layout.stories.contains(ratatui::layout::Position::new(column, row)) {
            Some(Pane::Stories)
        } else if layout.comments.contains(ratatui::layout::Position::new(column, row)) {
            Some(Pane::Comments)
        } else {
            None
        };

        if let Some(pane) = pane {
            match (pane, down) {
                (Pane::Stories, true) => {
                    self.story_state.select_next();
                    self.check_lazy_load();
                }
                (Pane::Stories, false) => {
                    self.story_state.select_prev();
                }
                (Pane::Comments, true) => {
                    self.comment_state.select_next();
                }
                (Pane::Comments, false) => {
                    self.comment_state.select_prev();
                }
            }
        }
    }
}
