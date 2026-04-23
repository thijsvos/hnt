use crate::api::client::HnClient;
use crate::api::types::{FeedKind, Item};
use crate::keys::{Action, InputMode};
use crate::state::comment_state::CommentTreeState;
use crate::state::reader_state::{ReaderState, StyledFragment};
use crate::state::search_state::SearchState;
use crate::state::story_state::StoryListState;
use crate::ui::theme;
use html2text::render::RichAnnotation;
use ratatui::style::{Modifier, Style};
use std::collections::HashSet;
use tokio::sync::mpsc;

const MIN_PAGE_SIZE: usize = 30;
const SCROLL_PAGE: usize = 10;
const MAX_COMMENT_DEPTH: usize = 10;
const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024; // 5MB

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Stories,
    Comments,
}

/// Messages sent from async tasks back to the main loop.
pub enum AppMessage {
    StoriesLoaded {
        stories: Vec<Item>,
        // Only populated on initial load; subsequent paginated loads reuse
        // the cached ID list to avoid drift when the feed changes mid-session.
        all_ids: Option<Vec<u64>>,
        append: bool,
    },
    CommentsLoaded {
        story: Box<Item>,
        comments: Vec<(Item, usize)>,
        pending_roots: HashSet<u64>,
    },
    /// Progressive update — append more child comments into the tree.
    CommentsAppended {
        parent_id: u64,
        children: Vec<(Item, usize)>,
    },
    CommentsDone,
    ArticleLoaded {
        lines: Vec<Vec<StyledFragment>>,
    },
    SearchResultsLoaded {
        stories: Vec<Item>,
        total_pages: usize,
        total_hits: usize,
        append: bool,
    },
    ArticleError(String),
    Error(String),
}

pub struct App {
    pub running: bool,
    pub current_feed: FeedKind,
    pub focus: Pane,
    pub story_state: StoryListState,
    pub comment_state: CommentTreeState,
    pub reader_state: Option<ReaderState>,
    pub search_state: Option<SearchState>,
    pub input_mode: InputMode,
    pub show_help: bool,
    pub error: Option<String>,
    pub terminal_height: u16,
    pub terminal_width: u16,
    pub tick_count: u64,

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
            reader_state: None,
            search_state: None,
            input_mode: InputMode::Normal,
            show_help: false,
            error: None,
            terminal_height,
            terminal_width,
            tick_count: 0,
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
                    if let Some(ids) = all_ids {
                        self.story_state.all_ids = ids;
                    }
                    self.story_state.loading = false;
                    self.error = None;
                    // Auto-load comments for the first story on initial load
                    if !append
                        && !self.story_state.stories.is_empty()
                        && self.comment_state.story.is_none()
                    {
                        self.load_selected_comments();
                        self.focus = Pane::Stories;
                    }
                }
                AppMessage::SearchResultsLoaded {
                    stories,
                    total_pages,
                    total_hits,
                    append,
                } => {
                    if append {
                        self.story_state.stories.extend(stories);
                    } else {
                        self.story_state.stories = stories;
                    }
                    self.story_state.loading = false;
                    self.error = None;
                    if let Some(ref mut ss) = self.search_state {
                        ss.total_pages = total_pages;
                        ss.total_hits = total_hits;
                    }
                    if !append && !self.story_state.stories.is_empty() {
                        self.load_selected_comments();
                        self.focus = Pane::Stories;
                    }
                }
                AppMessage::CommentsLoaded {
                    story,
                    comments,
                    pending_roots,
                } => {
                    self.comment_state.story = Some(*story);
                    self.comment_state.set_comments(comments);
                    self.comment_state.pending_root_ids = pending_roots;
                    // Still loading children in background
                    self.error = None;
                }
                AppMessage::CommentsAppended {
                    parent_id,
                    children,
                } => {
                    self.comment_state.insert_children(parent_id, children);
                    self.comment_state.pending_root_ids.remove(&parent_id);
                }
                AppMessage::CommentsDone => {
                    self.comment_state.loading = false;
                    self.comment_state.pending_root_ids.clear();
                }
                AppMessage::ArticleLoaded { lines } => {
                    if let Some(ref mut reader) = self.reader_state {
                        reader.set_content(lines);
                    }
                }
                AppMessage::ArticleError(msg) => {
                    if let Some(ref mut reader) = self.reader_state {
                        reader.set_error(msg);
                    }
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
        // When reader is open, route actions to reader
        if self.reader_state.is_some() {
            match action {
                Action::Back => {
                    self.reader_state = None;
                    return;
                }
                Action::MoveDown => {
                    if let Some(ref mut r) = self.reader_state {
                        r.scroll_down(1);
                    }
                    return;
                }
                Action::MoveUp => {
                    if let Some(ref mut r) = self.reader_state {
                        r.scroll_up(1);
                    }
                    return;
                }
                Action::PageDown => {
                    if let Some(ref mut r) = self.reader_state {
                        r.page_down(SCROLL_PAGE);
                    }
                    return;
                }
                Action::PageUp => {
                    if let Some(ref mut r) = self.reader_state {
                        r.page_up(SCROLL_PAGE);
                    }
                    return;
                }
                Action::JumpTop => {
                    if let Some(ref mut r) = self.reader_state {
                        r.jump_top();
                    }
                    return;
                }
                Action::JumpBottom => {
                    if let Some(ref mut r) = self.reader_state {
                        r.jump_bottom();
                    }
                    return;
                }
                Action::OpenInBrowser => {
                    if let Some(ref reader) = self.reader_state {
                        if let Some(ref url) = reader.url {
                            if let Ok(parsed) = url::Url::parse(url) {
                                if parsed.scheme() == "http" || parsed.scheme() == "https" {
                                    let _ = open::that(parsed.as_str());
                                }
                            }
                        }
                    }
                    return;
                }
                _ => return, // Consume all other keys
            }
        }

        match action {
            Action::Quit => self.running = false,
            Action::Back => {
                if self.focus == Pane::Comments && self.comment_state.story.is_some() {
                    self.comment_state.reset();
                    self.focus = Pane::Stories;
                } else if self.search_state.is_some() {
                    self.cancel_search();
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
            Action::OpenReader => self.open_article_reader(),
            Action::SwitchPane => {
                self.focus = match self.focus {
                    Pane::Stories => Pane::Comments,
                    Pane::Comments => Pane::Stories,
                };
            }
            Action::SwitchFeed(idx) => {
                if idx < FeedKind::ALL.len() {
                    let feed = FeedKind::ALL[idx];
                    if feed != self.current_feed || self.search_state.is_some() {
                        self.search_state = None;
                        self.input_mode = InputMode::Normal;
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
                if let Some(ref ss) = self.search_state {
                    let query = ss.query.clone();
                    if !query.is_empty() {
                        self.story_state.reset();
                        self.comment_state.reset();
                        self.spawn_search(&query, 0, false);
                    }
                } else {
                    self.story_state.reset();
                    self.comment_state.reset();
                    self.client.clear_cache();
                    self.spawn_load_stories(false);
                }
            }
            Action::EnterSearch => {
                self.enter_search_mode();
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

    pub fn enter_search_mode(&mut self) {
        self.input_mode = InputMode::SearchInput;
        self.search_state = Some(SearchState::new());
        self.focus = Pane::Stories;
    }

    pub fn submit_search(&mut self) {
        let query = if let Some(ref ss) = self.search_state {
            ss.input.trim().to_string()
        } else {
            return;
        };

        if query.is_empty() {
            self.cancel_search();
            return;
        }

        if let Some(ref mut ss) = self.search_state {
            ss.query = query.clone();
            ss.current_page = 0;
        }

        self.input_mode = InputMode::Normal;
        self.story_state.reset();
        self.comment_state.reset();
        self.spawn_search(&query, 0, false);
    }

    pub fn cancel_search(&mut self) {
        self.search_state = None;
        self.input_mode = InputMode::Normal;
        self.story_state.reset();
        self.comment_state.reset();
        self.client.clear_cache();
        self.spawn_load_stories(false);
    }

    pub fn search_input_char(&mut self, c: char) {
        if let Some(ref mut ss) = self.search_state {
            ss.input.push(c);
        }
    }

    pub fn search_input_backspace(&mut self) {
        if let Some(ref mut ss) = self.search_state {
            ss.input.pop();
        }
    }

    fn spawn_search(&mut self, query: &str, page: usize, append: bool) {
        self.story_state.loading = true;
        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let query = query.to_string();
        let page_size = self.page_size();

        tokio::spawn(async move {
            match client.search_stories(&query, page, page_size).await {
                Ok((stories, total_pages, total_hits)) => {
                    let _ = tx.send(AppMessage::SearchResultsLoaded {
                        stories,
                        total_pages,
                        total_hits,
                        append,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(format!("Search failed: {}", e)));
                }
            }
        });
    }

    fn spawn_load_stories(&self, append: bool) {
        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let page_size = self.page_size();

        if append {
            // Reuse the ID list from the initial load so offsets stay stable
            // even if new stories have been posted to the feed since.
            let cached_ids = self.story_state.all_ids.clone();
            let offset = self.story_state.stories.len();
            tokio::spawn(async move {
                match client
                    .fetch_items_page(&cached_ids, offset, page_size)
                    .await
                {
                    Ok(stories) => {
                        let _ = tx.send(AppMessage::StoriesLoaded {
                            stories,
                            all_ids: None,
                            append: true,
                        });
                    }
                    Err(e) => {
                        let _ =
                            tx.send(AppMessage::Error(format!("Failed to load stories: {}", e)));
                    }
                }
            });
        } else {
            let feed = self.current_feed;
            tokio::spawn(async move {
                match client.fetch_stories(feed, 0, page_size).await {
                    Ok((stories, all_ids)) => {
                        let _ = tx.send(AppMessage::StoriesLoaded {
                            stories,
                            all_ids: Some(all_ids),
                            append: false,
                        });
                    }
                    Err(e) => {
                        let _ =
                            tx.send(AppMessage::Error(format!("Failed to load stories: {}", e)));
                    }
                }
            });
        }
    }

    fn load_selected_comments(&mut self) {
        if let Some(story) = self.story_state.selected_story().cloned() {
            self.comment_state.loading = true;
            self.focus = Pane::Comments;

            let client = self.client.clone();
            let tx = self.msg_tx.clone();
            let story_clone = story.clone();
            let needs_full_fetch = story.kids.is_none();
            let kids = story.kids.clone().unwrap_or_default();

            tokio::spawn(async move {
                // For search results, kids is None — fetch the full item first
                let kids = if needs_full_fetch && story_clone.id > 0 {
                    match client.fetch_item(story_clone.id).await {
                        Ok(Some(full_item)) => full_item.kids.unwrap_or_default(),
                        _ => kids,
                    }
                } else {
                    kids
                };

                // Step 1: Fetch root-level comments and show them immediately
                let root_items = client.fetch_items(&kids).await;
                let root_comments: Vec<(Item, usize)> = root_items
                    .into_iter()
                    .flatten()
                    .filter(|item| !item.is_dead_or_deleted())
                    .map(|item| (item, 0))
                    .collect();

                let pending_roots: HashSet<u64> = root_comments
                    .iter()
                    .filter(|(r, _)| r.kids.as_ref().is_some_and(|k| !k.is_empty()))
                    .map(|(r, _)| r.id)
                    .collect();

                let _ = tx.send(AppMessage::CommentsLoaded {
                    story: Box::new(story_clone),
                    comments: root_comments.clone(),
                    pending_roots,
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

    fn open_article_reader(&mut self) {
        let story = match self.focus {
            Pane::Stories => self.story_state.selected_story().cloned(),
            Pane::Comments => self.comment_state.story.clone(),
        };

        let story = match story {
            Some(s) => s,
            None => return,
        };

        let title = story.title.clone().unwrap_or_default();
        let domain = story.domain();
        let url = story.url.clone();

        // For Ask HN / text-only stories: render inline text directly
        if url.is_none() {
            if let Some(ref text) = story.text {
                let width = self.terminal_width.saturating_sub(6) as usize;
                let lines = html_to_styled_lines(text.as_bytes(), width);
                let mut reader = ReaderState::new_loading(title, domain, None);
                reader.set_content(lines);
                self.reader_state = Some(reader);
            }
            return;
        }

        // Reject non-http(s) schemes (file://, javascript:, data:, etc.) before fetching.
        match url::Url::parse(url.as_deref().unwrap_or("")) {
            Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => {}
            _ => return,
        }

        self.reader_state = Some(ReaderState::new_loading(title, domain, url.clone()));

        let tx = self.msg_tx.clone();
        let width = self.terminal_width.saturating_sub(6) as usize;
        let url = url.unwrap();

        tokio::spawn(async move {
            match fetch_and_extract_article(&url, width).await {
                Ok(lines) => {
                    let _ = tx.send(AppMessage::ArticleLoaded { lines });
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::ArticleError(e));
                }
            }
        });
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
            if let Ok(parsed) = url::Url::parse(&url) {
                if parsed.scheme() == "http" || parsed.scheme() == "https" {
                    let _ = open::that(parsed.as_str());
                }
            }
        }
    }

    fn check_lazy_load(&mut self) {
        if self.story_state.loading {
            return;
        }

        if let Some(ref mut ss) = self.search_state {
            // Search mode: page-based pagination
            let threshold = (self.story_state.stories.len() as f64 * 0.8) as usize;
            if self.story_state.selected >= threshold && ss.current_page + 1 < ss.total_pages {
                ss.current_page += 1;
                let query = ss.query.clone();
                let page = ss.current_page;
                self.spawn_search(&query, page, true);
            }
        } else if self.story_state.needs_more() {
            self.story_state.loading = true;
            self.spawn_load_stories(true);
        }
    }

    /// Handle a mouse left-click at the given terminal position.
    pub fn handle_click(&mut self, column: u16, row: u16) {
        // When reader is open, consume clicks
        if self.reader_state.is_some() {
            return;
        }

        use crate::ui::layout::build_layout;
        use ratatui::layout::Rect;
        use ratatui::widgets::{Block, Borders};

        let area = Rect::new(0, 0, self.terminal_width, self.terminal_height);
        let layout = build_layout(area);

        if layout
            .stories
            .contains(ratatui::layout::Position::new(column, row))
        {
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
        } else if layout
            .comments
            .contains(ratatui::layout::Position::new(column, row))
        {
            let inner = Block::default()
                .borders(Borders::ALL)
                .inner(layout.comments);
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
    pub fn handle_scroll(&mut self, _column: u16, _row: u16, down: bool) {
        // When reader is open, scroll reader
        if self.reader_state.is_some() {
            if let Some(ref mut reader) = self.reader_state {
                if down {
                    reader.scroll_down(3);
                } else {
                    reader.scroll_up(3);
                }
            }
            return;
        }

        use crate::ui::layout::build_layout;
        use ratatui::layout::Rect;

        let area = Rect::new(0, 0, self.terminal_width, self.terminal_height);
        let layout = build_layout(area);

        let pane = if layout
            .stories
            .contains(ratatui::layout::Position::new(_column, _row))
        {
            Some(Pane::Stories)
        } else if layout
            .comments
            .contains(ratatui::layout::Position::new(_column, _row))
        {
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

/// For GitHub/GitLab repo pages, try to fetch the raw README instead of the
/// JS-heavy HTML shell (the README content is loaded dynamically by JS).
async fn try_fetch_readme(client: &reqwest::Client, url: &str) -> Option<(String, bool)> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let path_segs: Vec<&str> = parsed.path().trim_matches('/').split('/').collect();

    // Only match repo root pages (no sub-paths like /issues, /blob, etc.)
    let readme_urls: Vec<String> = if host == "github.com" || host.ends_with(".github.com") {
        if path_segs.len() != 2 || path_segs[0].is_empty() || path_segs[1].is_empty() {
            return None;
        }
        let (owner, repo) = (path_segs[0], path_segs[1]);
        vec![
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/README.md",
                owner, repo
            ),
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/readme.md",
                owner, repo
            ),
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/README.rst",
                owner, repo
            ),
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/README",
                owner, repo
            ),
        ]
    } else if host == "gitlab.com" || host.ends_with(".gitlab.com") {
        if path_segs.len() < 2 || path_segs.iter().any(|s| s.is_empty()) {
            return None;
        }
        // GitLab can have nested groups: gitlab.com/group/subgroup/repo
        let project_path = path_segs.join("/");
        vec![
            format!("https://gitlab.com/{}/-/raw/HEAD/README.md", project_path),
            format!("https://gitlab.com/{}/-/raw/HEAD/readme.md", project_path),
            format!("https://gitlab.com/{}/-/raw/HEAD/README.rst", project_path),
            format!("https://gitlab.com/{}/-/raw/HEAD/README", project_path),
        ]
    } else {
        return None;
    };

    for readme_url in readme_urls {
        if let Ok(resp) = client.get(&readme_url).send().await {
            if resp.status().is_success() {
                if let Some(len) = resp.content_length() {
                    if len > MAX_RESPONSE_BYTES as u64 {
                        continue;
                    }
                }
                if let Ok(text) = resp.text().await {
                    if text.len() > MAX_RESPONSE_BYTES {
                        continue;
                    }
                    if !text.trim().is_empty() {
                        let is_markdown = readme_url.ends_with(".md");
                        return Some((text, is_markdown));
                    }
                }
            }
        }
    }

    None
}

/// Convert markdown text to styled lines with basic formatting.
fn markdown_to_styled_lines(text: &str, width: usize) -> Vec<Vec<StyledFragment>> {
    let mut lines: Vec<Vec<StyledFragment>> = Vec::new();

    for raw_line in text.lines() {
        // Heading detection
        if let Some(rest) = raw_line.strip_prefix("# ") {
            lines.push(vec![StyledFragment {
                text: rest.to_string(),
                style: Style::default()
                    .fg(theme::HN_ORANGE)
                    .add_modifier(Modifier::BOLD),
            }]);
            lines.push(vec![]);
        } else if let Some(rest) = raw_line.strip_prefix("## ") {
            lines.push(vec![StyledFragment {
                text: rest.to_string(),
                style: Style::default()
                    .fg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            }]);
            lines.push(vec![]);
        } else if let Some(rest) = raw_line.strip_prefix("### ") {
            lines.push(vec![StyledFragment {
                text: rest.to_string(),
                style: Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            }]);
            lines.push(vec![]);
        } else if raw_line.starts_with("```") {
            // Code fence marker — just skip the marker line
            lines.push(vec![StyledFragment {
                text: raw_line.to_string(),
                style: Style::default().fg(theme::DIM),
            }]);
        } else if raw_line.starts_with("    ") || raw_line.starts_with('\t') {
            // Indented code
            lines.push(vec![StyledFragment {
                text: raw_line.to_string(),
                style: Style::default().fg(theme::GREEN).bg(theme::SURFACE),
            }]);
        } else if raw_line.starts_with("- ") || raw_line.starts_with("* ") {
            lines.push(vec![
                StyledFragment {
                    text: "  \u{2022} ".to_string(),
                    style: Style::default().fg(theme::HN_ORANGE),
                },
                StyledFragment {
                    text: raw_line[2..].to_string(),
                    style: Style::default().fg(theme::TEXT),
                },
            ]);
        } else if let Some(rest) = raw_line.strip_prefix("> ") {
            lines.push(vec![
                StyledFragment {
                    text: "\u{2502} ".to_string(),
                    style: Style::default().fg(theme::DIM),
                },
                StyledFragment {
                    text: rest.to_string(),
                    style: Style::default()
                        .fg(theme::SUBTEXT)
                        .add_modifier(Modifier::ITALIC),
                },
            ]);
        } else {
            // Word-wrap long lines
            if raw_line.chars().count() > width && width > 0 {
                let mut remaining = raw_line;
                while !remaining.is_empty() {
                    if remaining.chars().count() <= width {
                        lines.push(vec![StyledFragment {
                            text: remaining.to_string(),
                            style: Style::default().fg(theme::TEXT),
                        }]);
                        break;
                    }
                    let byte_pos = remaining
                        .char_indices()
                        .nth(width)
                        .map(|(i, _)| i)
                        .unwrap_or(remaining.len());
                    let split_at = remaining[..byte_pos]
                        .rfind(' ')
                        .map(|p| p + 1)
                        .unwrap_or(byte_pos);
                    lines.push(vec![StyledFragment {
                        text: remaining[..split_at].to_string(),
                        style: Style::default().fg(theme::TEXT),
                    }]);
                    remaining = &remaining[split_at..];
                }
            } else {
                lines.push(vec![StyledFragment {
                    text: raw_line.to_string(),
                    style: Style::default().fg(theme::TEXT),
                }]);
            }
        }
    }

    lines
}

/// Fetch article HTML, run readability extraction, convert to styled lines.
async fn fetch_and_extract_article(
    url: &str,
    width: usize,
) -> Result<Vec<Vec<StyledFragment>>, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            "Mozilla/5.0 (compatible; hnt/",
            env!("CARGO_PKG_VERSION"),
            ")"
        ))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // For GitHub/GitLab repo pages, try fetching the README directly
    if let Some((readme_text, is_markdown)) = try_fetch_readme(&client, url).await {
        return if is_markdown {
            Ok(markdown_to_styled_lines(&readme_text, width))
        } else {
            // RST / plain text — render as plain styled lines
            Ok(readme_text
                .lines()
                .map(|line| {
                    vec![StyledFragment {
                        text: line.to_string(),
                        style: Style::default().fg(theme::TEXT),
                    }]
                })
                .collect())
        };
    }

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES as u64 {
            return Err("Article too large (>5MB)".to_string());
        }
    }

    // Check content-type — reject non-HTML
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    if !content_type.is_empty()
        && !content_type.contains("text/html")
        && !content_type.contains("text/plain")
        && !content_type.contains("application/xhtml")
    {
        return Err(format!(
            "Not an article (content-type: {})",
            content_type.split(';').next().unwrap_or(&content_type)
        ));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("Article too large (>5MB)".to_string());
    }

    // Run readability extraction in a blocking task (CPU-bound)
    let url_string = url.to_string();
    let width_copy = width;
    tokio::task::spawn_blocking(move || extract_article_content(&bytes, &url_string, width_copy))
        .await
        .map_err(|e| format!("Processing error: {}", e))?
}

/// Run readability extraction + html2text rich rendering (blocking/CPU-bound).
fn extract_article_content(
    html_bytes: &[u8],
    url_str: &str,
    width: usize,
) -> Result<Vec<Vec<StyledFragment>>, String> {
    let parsed_url = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;

    // Try readability extraction first, fall back to full HTML if it produces no content
    let tagged_lines = {
        let mut cursor = std::io::Cursor::new(html_bytes);
        let readability_lines = match readability::extract(
            &mut cursor,
            &parsed_url,
            readability::ExtractOptions::default(),
        ) {
            Ok(readable) if !readable.text.trim().is_empty() => {
                html2text::from_read_rich(readable.content.as_bytes(), width).unwrap_or_default()
            }
            _ => Vec::new(),
        };

        if readability_lines
            .iter()
            .any(|l| l.tagged_strings().any(|ts| !ts.s.trim().is_empty()))
        {
            readability_lines
        } else {
            // Fallback: render the full HTML
            html2text::from_read_rich(html_bytes, width).unwrap_or_default()
        }
    };

    let lines: Vec<Vec<StyledFragment>> = tagged_lines
        .into_iter()
        .map(|tagged_line| {
            let mut fragments = Vec::new();
            for ts in tagged_line.tagged_strings() {
                let style = annotations_to_style(&ts.tag);
                fragments.push(StyledFragment {
                    text: ts.s.clone(),
                    style,
                });
                // Append URL after link text
                for ann in &ts.tag {
                    if let RichAnnotation::Link(ref url) = ann {
                        fragments.push(StyledFragment {
                            text: format!(" [{}]", url),
                            style: Style::default().fg(theme::DIM),
                        });
                    }
                }
            }
            fragments
        })
        .collect();

    Ok(lines)
}

/// Convert html2text RichAnnotation set to a ratatui Style.
fn annotations_to_style(annotations: &[RichAnnotation]) -> Style {
    let mut style = Style::default().fg(theme::TEXT);

    for ann in annotations {
        match ann {
            RichAnnotation::Strong => {
                style = style.add_modifier(Modifier::BOLD);
            }
            RichAnnotation::Emphasis => {
                style = style.add_modifier(Modifier::ITALIC);
            }
            RichAnnotation::Code | RichAnnotation::Preformat(_) => {
                style = style.fg(theme::GREEN).bg(theme::SURFACE);
            }
            RichAnnotation::Link(_) => {
                style = style.fg(theme::BLUE).add_modifier(Modifier::UNDERLINED);
            }
            RichAnnotation::Strikeout => {
                style = style.add_modifier(Modifier::CROSSED_OUT);
            }
            RichAnnotation::Image(_) => {
                style = style.fg(theme::MAUVE).add_modifier(Modifier::ITALIC);
            }
            _ => {}
        }
    }

    style
}

/// Convert raw HTML bytes to styled lines using html2text rich rendering.
fn html_to_styled_lines(html: &[u8], width: usize) -> Vec<Vec<StyledFragment>> {
    let tagged_lines = html2text::from_read_rich(html, width).unwrap_or_default();

    tagged_lines
        .into_iter()
        .map(|tagged_line| {
            let mut fragments = Vec::new();
            for ts in tagged_line.tagged_strings() {
                let style = annotations_to_style(&ts.tag);
                fragments.push(StyledFragment {
                    text: ts.s.clone(),
                    style,
                });
                for ann in &ts.tag {
                    if let RichAnnotation::Link(ref url) = ann {
                        fragments.push(StyledFragment {
                            text: format!(" [{}]", url),
                            style: Style::default().fg(theme::DIM),
                        });
                    }
                }
            }
            fragments
        })
        .collect()
}
