//! Central application state and event dispatch.
//!
//! [`App`] owns every pane's state, the HN client, and an MPSC channel
//! used by spawned tokio tasks to deliver results back to the main loop
//! via [`AppMessage`]. [`App::dispatch`] translates [`Action`]s from the
//! keybinding layer into state mutations and task spawns;
//! [`App::process_messages`] drains pending async results each frame.

use crate::api::client::HnClient;
use crate::api::types::{FeedKind, Item};
use crate::article::{fetch_and_extract_article, html_to_styled_lines};
use crate::keys::{Action, InputMode};
use crate::state::comment_state::CommentTreeState;
use crate::state::prior_state::PriorDiscussionsState;
use crate::state::reader_state::{ReaderState, StyledFragment};
use crate::state::search_state::SearchState;
use crate::state::story_state::StoryListState;
use std::collections::{HashMap, HashSet};
use tokio::sync::mpsc;

const MIN_PAGE_SIZE: usize = 30;
const SCROLL_PAGE: usize = 10;
const MAX_COMMENT_DEPTH: usize = 10;

/// Which content pane currently has keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pane {
    Stories,
    Comments,
}

/// Messages sent from async tasks back to the main loop.
///
/// Variants correspond to the lifecycle of each async operation: a one-shot
/// load (`StoriesLoaded`, `SearchResultsLoaded`, `ArticleLoaded`), a
/// multi-step progressive load (`CommentsLoaded` → zero or more
/// `CommentsAppended` → `CommentsDone`), or a terminal error (`Error`,
/// `ArticleError`).
pub enum AppMessage {
    /// Initial or paginated batch of stories finished loading.
    StoriesLoaded {
        stories: Vec<Item>,
        /// Only populated on initial load; subsequent paginated loads
        /// reuse the cached ID list to avoid drift when the feed changes
        /// mid-session.
        all_ids: Option<Vec<u64>>,
        append: bool,
    },
    /// Root-level comments for a story are available; deeper descendants
    /// still pending.
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
    /// All outstanding comment fetches finished; clear any "loading"
    /// spinners.
    CommentsDone,
    /// Article reader content extracted and ready to render.
    ArticleLoaded { lines: Vec<Vec<StyledFragment>> },
    /// Algolia search returned a page of results.
    SearchResultsLoaded {
        stories: Vec<Item>,
        total_pages: usize,
        total_hits: usize,
        append: bool,
    },
    /// Article fetch/extract failed; surface in the reader overlay.
    ArticleError(String),
    /// Generic error to surface in the status bar.
    Error(String),
    /// Algolia returned prior HN submissions of the selected story's URL.
    /// `story_id` is the story that triggered the query — used to drop
    /// results whose story has since been deselected.
    PriorDiscussionsLoaded {
        story_id: u64,
        submissions: Vec<Item>,
    },
}

/// Central application state — owned by the main loop.
///
/// Aggregates every pane's state, the shared [`HnClient`], and an MPSC
/// channel that async tasks use to send [`AppMessage`]s back. All input
/// flows through [`App::dispatch`]; all async results flow through
/// [`App::process_messages`].
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

    /// Prior-discussions overlay state. `Some` while the overlay is open;
    /// `None` otherwise. Contents are populated from `prior_results` when
    /// the user presses `h`.
    pub prior_state: Option<PriorDiscussionsState>,
    /// Prior-submissions query results, keyed by the story ID that was
    /// queried. Keeps each result around for the rest of the session so
    /// reopening the `h` overlay doesn't trigger a refetch.
    pub prior_results: HashMap<u64, Vec<Item>>,
    /// Story IDs whose URL queries are in flight. Prevents duplicate spawns.
    prior_in_flight: HashSet<u64>,

    last_comment_click: Option<(std::time::Instant, usize)>,

    client: HnClient,
    msg_tx: mpsc::UnboundedSender<AppMessage>,
    msg_rx: mpsc::UnboundedReceiver<AppMessage>,
}

impl App {
    /// Constructs an [`App`] sized to the given terminal dimensions, with
    /// a fresh HN client, empty state, and a brand-new message channel.
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
            prior_state: None,
            prior_results: HashMap::new(),
            prior_in_flight: HashSet::new(),
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

    /// Updates cached terminal dimensions after a resize event.
    pub fn set_terminal_size(&mut self, w: u16, h: u16) {
        self.terminal_width = w;
        self.terminal_height = h;
    }

    /// Spawns a background fetch for the first page of the current feed.
    ///
    /// Intended to be called once at startup; calling it concurrently will
    /// race two `StoriesLoaded` messages into the channel.
    pub fn load_initial_feed(&self) {
        self.spawn_load_stories(false);
    }

    /// Processes any pending async messages (non-blocking).
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
                AppMessage::PriorDiscussionsLoaded {
                    story_id,
                    submissions,
                } => {
                    self.prior_in_flight.remove(&story_id);
                    // If the user has already opened the overlay for this
                    // same story, backfill its contents now that we have data.
                    if let Some(ref mut ps) = self.prior_state {
                        if ps.story_id == story_id && ps.submissions.is_empty() {
                            ps.submissions = submissions.clone();
                            ps.selected = 0;
                        }
                    }
                    self.prior_results.insert(story_id, submissions);
                }
            }
        }
    }

    /// Applies an [`Action`] from the keybinding layer.
    ///
    /// Routing is context-sensitive: when the article reader is open, a
    /// restricted set of actions drives the reader and others are consumed.
    /// Otherwise the action mutates the focused pane's state or spawns an
    /// async task (feed switch, refresh, comment load, search).
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

        // When the prior-discussions overlay is open, route a reduced action
        // set and consume everything else.
        if self.prior_state.is_some() {
            match action {
                Action::Back => {
                    self.prior_state = None;
                    return;
                }
                Action::MoveDown => {
                    if let Some(ref mut p) = self.prior_state {
                        p.select_next();
                    }
                    return;
                }
                Action::MoveUp => {
                    if let Some(ref mut p) = self.prior_state {
                        p.select_prev();
                    }
                    return;
                }
                Action::JumpTop => {
                    if let Some(ref mut p) = self.prior_state {
                        p.jump_top();
                    }
                    return;
                }
                Action::JumpBottom => {
                    if let Some(ref mut p) = self.prior_state {
                        p.jump_bottom();
                    }
                    return;
                }
                Action::Select => {
                    self.open_selected_prior_discussion();
                    return;
                }
                Action::OpenInBrowser => {
                    if let Some(ref p) = self.prior_state {
                        if let Some(item) = p.selected_submission() {
                            if let Some(ref url) = item.url {
                                if let Ok(parsed) = url::Url::parse(url) {
                                    if parsed.scheme() == "http" || parsed.scheme() == "https" {
                                        let _ = open::that(parsed.as_str());
                                    }
                                }
                            }
                        }
                    }
                    return;
                }
                _ => return,
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
            Action::TogglePriorDiscussions => self.toggle_prior_discussions(),
            Action::None => {}
        }
    }

    /// Opens the prior-discussions overlay for the story whose comments are
    /// currently loaded, using cached results from [`App::prior_results`].
    /// No-op if no comments-loaded story has a URL-based query result. Opens
    /// an empty-state overlay if a query returned zero prior submissions.
    fn toggle_prior_discussions(&mut self) {
        if self.prior_state.is_some() {
            self.prior_state = None;
            return;
        }
        let Some(story) = self.comment_state.story.as_ref() else {
            return;
        };
        let story_id = story.id;
        if let Some(submissions) = self.prior_results.get(&story_id) {
            self.prior_state = Some(PriorDiscussionsState::new(story_id, submissions.clone()));
        } else if let Some(url) = story.url.clone() {
            // No result yet — fire the query (if not already in flight) and
            // open an empty overlay; it will populate on the next
            // process_messages tick via the overlay's view of prior_results.
            self.spawn_prior_discussions(story_id, &url);
            self.prior_state = Some(PriorDiscussionsState::new(story_id, Vec::new()));
        }
    }

    /// Loads the selected prior submission's comments as if the user had
    /// selected it from the story pane. Closes the prior-discussions overlay.
    fn open_selected_prior_discussion(&mut self) {
        let Some(item) = self
            .prior_state
            .as_ref()
            .and_then(|p| p.selected_submission().cloned())
        else {
            return;
        };
        self.prior_state = None;
        self.focus = Pane::Comments;
        self.comment_state.loading = true;

        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let story = item.clone();
        let kids = item.kids.clone().unwrap_or_default();
        let needs_full_fetch = item.kids.is_none();

        tokio::spawn(async move {
            let kids = if needs_full_fetch && story.id > 0 {
                match client.fetch_item(story.id).await {
                    Ok(Some(full_item)) => full_item.kids.unwrap_or_default(),
                    _ => kids,
                }
            } else {
                kids
            };
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
                story: Box::new(story.clone()),
                comments: root_comments.clone(),
                pending_roots,
            });
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

    /// Transitions into search-input mode, showing an empty search prompt.
    pub fn enter_search_mode(&mut self) {
        self.input_mode = InputMode::SearchInput;
        self.search_state = Some(SearchState::new());
        self.focus = Pane::Stories;
    }

    /// Commits the typed `input` as the active search query and spawns an
    /// Algolia fetch for page 0. An empty input cancels search instead.
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

    /// Exits search mode, clears the cache, and reloads the current feed.
    pub fn cancel_search(&mut self) {
        self.search_state = None;
        self.input_mode = InputMode::Normal;
        self.story_state.reset();
        self.comment_state.reset();
        self.client.clear_cache();
        self.spawn_load_stories(false);
    }

    /// Appends a typed character to the in-progress search input.
    pub fn search_input_char(&mut self, c: char) {
        if let Some(ref mut ss) = self.search_state {
            ss.input.push(c);
        }
    }

    /// Removes the last character from the in-progress search input.
    pub fn search_input_backspace(&mut self) {
        if let Some(ref mut ss) = self.search_state {
            ss.input.pop();
        }
    }

    /// Kicks off an async Algolia search. `append = true` extends the
    /// current result list (lazy pagination); `append = false` replaces it.
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

    /// Kicks off an async feed-page load. When `append` is true, reuses
    /// the cached ID list to compute a stable offset (so newly posted
    /// stories don't shift the page); otherwise fetches a fresh ID list.
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

    /// Kicks off a background Algolia query for prior HN submissions of the
    /// given URL. No-ops if the story's URL has already been queried or a
    /// query is already in flight. Failures silently no-op — prior-discussions
    /// is optional UX, not critical-path.
    fn spawn_prior_discussions(&mut self, story_id: u64, url: &str) {
        if self.prior_results.contains_key(&story_id) || self.prior_in_flight.contains(&story_id) {
            return;
        }
        self.prior_in_flight.insert(story_id);

        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let url = url.to_string();
        tokio::spawn(async move {
            let submissions = match client.search_by_url(&url).await {
                Ok(items) => items.into_iter().filter(|i| i.id != story_id).collect(),
                Err(_) => Vec::new(),
            };
            let _ = tx.send(AppMessage::PriorDiscussionsLoaded {
                story_id,
                submissions,
            });
        });
    }

    /// Kicks off a two-phase comment fetch for the currently selected story:
    /// (1) loads and displays root-level comments immediately, then
    /// (2) walks each root's subtree depth-first and appends children via
    ///     [`AppMessage::CommentsAppended`] as they arrive.
    /// For search results (kids missing), fetches the full item first.
    fn load_selected_comments(&mut self) {
        if let Some(story) = self.story_state.selected_story().cloned() {
            self.comment_state.loading = true;
            self.focus = Pane::Comments;

            // Fire a background prior-submissions query for this story's URL
            // so the `h` overlay has data ready when the user asks for it.
            if let Some(url) = story.url.as_deref() {
                self.spawn_prior_discussions(story.id, url);
            }

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

    /// Opens the reader overlay for the story in the focused pane.
    ///
    /// For text-only posts (Ask HN, etc.) renders the inline `text`
    /// locally. For URL stories, validates the http(s) scheme and then
    /// spawns a fetch + readability extraction task.
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

    /// Opens the focused story's URL in the system browser. Falls back to
    /// the HN item page for text-only stories. Only http(s) URLs are
    /// opened.
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

    /// If the selection has crossed the lazy-load threshold, kicks off
    /// the next page. Uses Algolia page-based pagination in search mode
    /// and offset-based pagination over cached IDs otherwise.
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

    /// Handles a mouse left-click at the given terminal cell.
    ///
    /// Maps the cell to a pane via `build_layout`: in the stories pane,
    /// selects the clicked story (triggering lazy load + comment fetch);
    /// in the comments pane, selects the clicked comment, treating a second
    /// click on the same row within 400ms as a double-click to toggle
    /// collapse. No-op when the reader overlay is open.
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
            let visual_index = self
                .comment_state
                .row_map
                .get(screen_row)
                .copied()
                .flatten();

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

    /// Handles a mouse wheel event in the pane under the cursor. When the
    /// reader overlay is open, scrolls the reader (3 lines per tick);
    /// otherwise moves the selected item in the hit pane.
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
