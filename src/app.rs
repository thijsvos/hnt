//! Central application state and event dispatch.
//!
//! [`App`] owns every pane's state, the HN client, and an MPSC channel
//! used by spawned tokio tasks to deliver results back to the main loop
//! via [`AppMessage`]. [`App::dispatch`] translates [`Action`]s from the
//! keybinding layer into state mutations and task spawns;
//! [`App::process_messages`] drains pending async results each frame.

use crate::api::client::HnClient;
use crate::api::types::{CommentId, CommentWithDepth, FeedKind, Item, StoryId};
use crate::article::{fetch_and_extract_article, html_to_styled_lines};
use crate::clipboard;
use crate::keys::{Action, InputMode};
use crate::state::comment_state::CommentTreeState;
use crate::state::hint_state::{HintAction, HintContext, HintState};
use crate::state::link_registry::{LinkRegistry, MatchResult};
use crate::state::prior_state::PriorDiscussionsState;
use crate::state::read_store::ReadStore;
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

/// Whether a paginated load should replace the current story list or
/// append to it. Previously a `bool` in [`AppMessage`] variants — named
/// variants make call sites self-documenting (`LoadMode::Append` vs
/// `false`) and prevent flipped-arg bugs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadMode {
    Replace,
    Append,
}

/// Tri-state outcome of a hint-buffer match against the active registry.
///
/// Carries the resolved URL by value so the borrow on the registry can
/// be released before mutating `self`. See [`LinkRegistry::match_prefix`].
enum HintResolve {
    /// Multiple labels still match — keep accepting characters.
    Continue,
    /// No labels match (or no surface to label) — exit hint mode.
    Cancel,
    /// Exactly one label matches — fire the action against this URL.
    Fire(String),
}

/// Messages sent from async tasks back to the main loop.
///
/// Variants correspond to the lifecycle of each async operation: a
/// one-shot load (`StoriesLoaded`, `SearchResultsLoaded`,
/// `ArticleLoaded`, `PriorDiscussionsLoaded`), a multi-step progressive
/// load (`CommentsLoaded` → zero or more `CommentsAppended` →
/// `CommentsDone`), or a terminal error (`Error`, `ArticleError`).
#[non_exhaustive]
pub enum AppMessage {
    /// Initial or paginated batch of stories finished loading.
    StoriesLoaded {
        stories: Vec<Item>,
        /// Only populated on initial load; subsequent paginated loads
        /// reuse the cached ID list to avoid drift when the feed changes
        /// mid-session.
        all_ids: Option<Vec<u64>>,
        mode: LoadMode,
    },
    /// Root-level comments for a story are available; deeper descendants
    /// still pending.
    CommentsLoaded {
        story: Box<Item>,
        comments: Vec<CommentWithDepth>,
        pending_roots: HashSet<CommentId>,
    },
    /// Progressive update — append more child comments into the tree.
    CommentsAppended {
        parent_id: CommentId,
        children: Vec<CommentWithDepth>,
    },
    /// All outstanding comment fetches finished; clear any "loading"
    /// spinners.
    CommentsDone,
    /// Article reader content extracted and ready to render. `links`
    /// carries every hyperlink in the body (with assigned hint labels)
    /// for Quickjump.
    ArticleLoaded {
        lines: Vec<Vec<StyledFragment>>,
        links: LinkRegistry,
    },
    /// Algolia search returned a page of results.
    SearchResultsLoaded {
        stories: Vec<Item>,
        total_pages: usize,
        total_hits: usize,
        mode: LoadMode,
    },
    /// Article fetch/extract failed; surface in the reader overlay.
    ArticleError(String),
    /// Generic error to surface in the status bar.
    Error(String),
    /// Carries prior HN submissions of the selected story's URL returned
    /// by Algolia. `story_id` identifies the originating query so stale
    /// results (user has since deselected the story) can be dropped.
    PriorDiscussionsLoaded {
        story_id: StoryId,
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
    /// `None` otherwise. Contents are populated from [`App::prior_results`]
    /// when the user presses `h`.
    pub prior_state: Option<PriorDiscussionsState>,
    /// Prior-submissions query results, keyed by the story ID that was
    /// queried. Keeps each result around for the rest of the session so
    /// reopening the [`PriorDiscussionsState`] overlay doesn't trigger a refetch.
    pub prior_results: HashMap<StoryId, Vec<Item>>,
    /// Story IDs whose URL queries are in flight. Prevents duplicate spawns.
    prior_in_flight: HashSet<StoryId>,

    /// Persisted read-state — records which stories have been opened and
    /// how many comments each had at the time. Rendered by
    /// [`crate::ui::story_list::StoryList`] to dim visited stories and
    /// surface `+N` new-comments badges. Loaded from disk at startup and
    /// flushed via [`App::persist`] on shutdown.
    pub read_store: ReadStore,

    /// Quickjump hint-mode state — `Some` while the user is selecting a
    /// label, `None` otherwise. Created by [`Action::EnterHintMode`]
    /// and torn down by [`Action::ExitHintMode`] or a unique-match dispatch.
    pub hint_state: Option<HintState>,

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
            read_store: ReadStore::load(),
            hint_state: None,
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

    /// Flushes any in-memory persistent state (read-store) to disk. Call
    /// once at shutdown after the main loop exits. Silently swallows I/O
    /// errors — read-state is non-critical.
    pub fn persist(&mut self) {
        self.read_store.save();
    }

    /// Spawns a background fetch for the first page of the current feed.
    ///
    /// Intended to be called once at startup; calling it concurrently will
    /// race two `StoriesLoaded` messages into the channel.
    pub fn load_initial_feed(&self) {
        self.spawn_load_stories(LoadMode::Replace);
    }

    /// Processes any pending async messages (non-blocking).
    pub fn process_messages(&mut self) {
        while let Ok(msg) = self.msg_rx.try_recv() {
            match msg {
                AppMessage::StoriesLoaded {
                    stories,
                    all_ids,
                    mode,
                } => {
                    match mode {
                        LoadMode::Append => self.story_state.stories.extend(stories),
                        LoadMode::Replace => self.story_state.stories = stories,
                    }
                    if let Some(ids) = all_ids {
                        self.story_state.all_ids = ids;
                    }
                    self.story_state.loading = false;
                    self.error = None;
                    // Auto-load comments for the first story on initial load
                    if matches!(mode, LoadMode::Replace)
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
                    mode,
                } => {
                    match mode {
                        LoadMode::Append => self.story_state.stories.extend(stories),
                        LoadMode::Replace => self.story_state.stories = stories,
                    }
                    self.story_state.loading = false;
                    self.error = None;
                    if let Some(ref mut ss) = self.search_state {
                        ss.total_pages = total_pages;
                        ss.total_hits = total_hits;
                    }
                    if matches!(mode, LoadMode::Replace) && !self.story_state.stories.is_empty() {
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
                AppMessage::ArticleLoaded { lines, links } => {
                    if let Some(ref mut reader) = self.reader_state {
                        reader.set_content(lines, links);
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
    /// Routing is context-sensitive, in priority order:
    ///
    /// 1. Hint actions ([`Action::EnterHintMode`] / [`Action::HintKey`] /
    ///    [`Action::ExitHintMode`]) short-circuit ahead of every overlay
    ///    so a mid-selection keypress never leaks through to the
    ///    underlying pane.
    /// 2. When the article-reader overlay is open, a restricted set of
    ///    navigation actions drives the reader; others are consumed.
    /// 3. When the prior-discussions overlay is open, a reduced action
    ///    set drives the overlay; others are consumed.
    /// 4. Otherwise the action mutates the focused pane's state or
    ///    spawns an async task (feed switch, refresh, comment load,
    ///    search).
    pub fn dispatch(&mut self, action: Action) {
        // Hint-mode actions short-circuit ahead of every overlay route
        // because the user is mid-selection and any keypress should be
        // narrowing labels, not mutating panes underneath.
        match &action {
            Action::HintKey(c) => {
                self.hint_key(*c);
                return;
            }
            Action::ExitHintMode => {
                self.exit_hint_mode();
                return;
            }
            Action::EnterHintMode(hint_action) => {
                self.enter_hint_mode(*hint_action);
                return;
            }
            _ => {}
        }

        // When reader is open, route actions to reader.
        if self.reader_state.is_some() {
            // Back mutates the Option itself, so handle it before borrowing
            // the inner state.
            if matches!(action, Action::Back) {
                self.reader_state = None;
                return;
            }
            let Some(r) = self.reader_state.as_mut() else {
                return;
            };
            match action {
                Action::MoveDown => r.scroll_down(1),
                Action::MoveUp => r.scroll_up(1),
                Action::PageDown => r.page_down(SCROLL_PAGE),
                Action::PageUp => r.page_up(SCROLL_PAGE),
                Action::JumpTop => r.jump_top(),
                Action::JumpBottom => r.jump_bottom(),
                Action::OpenInBrowser => open_http_url(r.url.as_deref()),
                // Exhaustive no-op list — when new Action variants are added
                // they'll provoke a compile error here so the overlay's
                // handling is a deliberate choice, not an accident.
                Action::Quit
                | Action::Select
                | Action::OpenReader
                | Action::SwitchPane
                | Action::SwitchFeed(_)
                | Action::Refresh
                | Action::EnterSearch
                | Action::ToggleHelp
                | Action::TogglePriorDiscussions
                | Action::None => {}
                Action::EnterHintMode(_) | Action::HintKey(_) | Action::ExitHintMode => {
                    unreachable!("hint actions handled above")
                }
                Action::Back => unreachable!("Back is handled above"),
            }
            return;
        }

        // When the prior-discussions overlay is open, route a reduced action
        // set and consume everything else.
        if self.prior_state.is_some() {
            // Actions that mutate App itself (not just the overlay's inner
            // state) go first so the subsequent borrow of `p` is clean.
            if matches!(action, Action::Back) {
                self.prior_state = None;
                return;
            }
            if matches!(action, Action::Select) {
                self.open_selected_prior_discussion();
                return;
            }
            let Some(p) = self.prior_state.as_mut() else {
                return;
            };
            match action {
                Action::MoveDown => p.select_next(),
                Action::MoveUp => p.select_prev(),
                Action::JumpTop => p.jump_top(),
                Action::JumpBottom => p.jump_bottom(),
                Action::OpenInBrowser => {
                    open_http_url(p.selected_submission().and_then(|i| i.url.as_deref()));
                }
                // Exhaustive no-op list — see note in reader block above.
                Action::Quit
                | Action::OpenReader
                | Action::SwitchPane
                | Action::SwitchFeed(_)
                | Action::Refresh
                | Action::EnterSearch
                | Action::ToggleHelp
                | Action::TogglePriorDiscussions
                | Action::PageDown
                | Action::PageUp
                | Action::None => {}
                Action::EnterHintMode(_) | Action::HintKey(_) | Action::ExitHintMode => {
                    unreachable!("hint actions handled above")
                }
                Action::Back | Action::Select => unreachable!("handled above"),
            }
            return;
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
                        self.spawn_load_stories(LoadMode::Replace);
                    }
                }
            }
            Action::Refresh => {
                if let Some(ref ss) = self.search_state {
                    let query = ss.query.clone();
                    if !query.is_empty() {
                        self.story_state.reset();
                        self.comment_state.reset();
                        self.spawn_search(&query, 0, LoadMode::Replace);
                    }
                } else {
                    self.story_state.reset();
                    self.comment_state.reset();
                    self.client.clear_cache();
                    self.spawn_load_stories(LoadMode::Replace);
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
            Action::EnterHintMode(_) | Action::HintKey(_) | Action::ExitHintMode => {
                unreachable!("hint actions handled above")
            }
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
        let story_id = StoryId(story.id);
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
        self.read_store
            .mark(StoryId(item.id), item.descendants.unwrap_or(0));
        self.prior_state = None;
        self.focus = Pane::Comments;
        self.comment_state.loading = true;

        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let story = item.clone();
        let kids = item.kids.clone().unwrap_or_default();
        let needs_full_fetch = item.kids.is_none();

        tokio::spawn(async move {
            // Search results arrive with kids == None — fetch the full item to
            // populate them. TryFrom<SearchHit> filters out id=0 upstream, so
            // no sentinel guard is needed. `fetch_item` returns Arc<Item>,
            // so we clone the kids Vec instead of moving out.
            let kids = if needs_full_fetch {
                match client.fetch_item(story.id).await {
                    Ok(Some(full_item)) => full_item.kids.clone().unwrap_or_default(),
                    _ => kids,
                }
            } else {
                kids
            };
            let root_items = client.fetch_items(&kids).await;
            let root_comments: Vec<CommentWithDepth> = root_items
                .into_iter()
                .flatten()
                .filter(|item| !item.is_dead_or_deleted())
                .map(|item| CommentWithDepth { item, depth: 0 })
                .collect();
            let pending_roots: HashSet<CommentId> = root_comments
                .iter()
                .filter(|c| c.item.kids.as_ref().is_some_and(|k| !k.is_empty()))
                .map(|c| CommentId(c.item.id))
                .collect();
            let _ = tx.send(AppMessage::CommentsLoaded {
                story: Box::new(story.clone()),
                comments: root_comments.clone(),
                pending_roots,
            });
            for c in &root_comments {
                let child_ids = c.item.kids.clone().unwrap_or_default();
                if child_ids.is_empty() {
                    continue;
                }
                let parent_id = CommentId(c.item.id);
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
        self.spawn_search(&query, 0, LoadMode::Replace);
    }

    /// Exits search mode, clears the cache, and reloads the current feed.
    pub fn cancel_search(&mut self) {
        self.search_state = None;
        self.input_mode = InputMode::Normal;
        self.story_state.reset();
        self.comment_state.reset();
        self.client.clear_cache();
        self.spawn_load_stories(LoadMode::Replace);
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

    /// Kicks off an async Algolia search. [`LoadMode::Append`] extends the
    /// current result list (lazy pagination); [`LoadMode::Replace`] replaces it.
    fn spawn_search(&mut self, query: &str, page: usize, mode: LoadMode) {
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
                        mode,
                    });
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::Error(format!("Search failed: {}", e)));
                }
            }
        });
    }

    /// Kicks off an async feed-page load. [`LoadMode::Append`] reuses the
    /// cached ID list to compute a stable offset (so newly posted stories
    /// don't shift the page); [`LoadMode::Replace`] fetches a fresh ID list.
    fn spawn_load_stories(&self, mode: LoadMode) {
        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let page_size = self.page_size();

        if matches!(mode, LoadMode::Append) {
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
                            mode: LoadMode::Append,
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
                            mode: LoadMode::Replace,
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
    fn spawn_prior_discussions(&mut self, story_id: StoryId, url: &str) {
        if self.prior_results.contains_key(&story_id) || self.prior_in_flight.contains(&story_id) {
            return;
        }
        self.prior_in_flight.insert(story_id);

        let client = self.client.clone();
        let tx = self.msg_tx.clone();
        let url = url.to_string();
        tokio::spawn(async move {
            let submissions = match client.search_by_url(&url).await {
                Ok(items) => items.into_iter().filter(|i| i.id != story_id.0).collect(),
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
            self.read_store
                .mark(StoryId(story.id), story.descendants.unwrap_or(0));
            self.comment_state.loading = true;
            self.focus = Pane::Comments;

            // Fire a background prior-submissions query for this story's URL
            // so the `h` overlay has data ready when the user asks for it.
            if let Some(url) = story.url.as_deref() {
                self.spawn_prior_discussions(StoryId(story.id), url);
            }

            let client = self.client.clone();
            let tx = self.msg_tx.clone();
            let story_clone = story.clone();
            let needs_full_fetch = story.kids.is_none();
            let kids = story.kids.clone().unwrap_or_default();

            tokio::spawn(async move {
                // For search results, kids is None — fetch the full item first.
                // TryFrom<SearchHit> filters out id=0 upstream. `fetch_item`
                // returns Arc<Item>, so clone the kids Vec instead of moving.
                let kids = if needs_full_fetch {
                    match client.fetch_item(story_clone.id).await {
                        Ok(Some(full_item)) => full_item.kids.clone().unwrap_or_default(),
                        _ => kids,
                    }
                } else {
                    kids
                };

                // Step 1: Fetch root-level comments and show them immediately
                let root_items = client.fetch_items(&kids).await;
                let root_comments: Vec<CommentWithDepth> = root_items
                    .into_iter()
                    .flatten()
                    .filter(|item| !item.is_dead_or_deleted())
                    .map(|item| CommentWithDepth { item, depth: 0 })
                    .collect();

                let pending_roots: HashSet<CommentId> = root_comments
                    .iter()
                    .filter(|c| c.item.kids.as_ref().is_some_and(|k| !k.is_empty()))
                    .map(|c| CommentId(c.item.id))
                    .collect();

                let _ = tx.send(AppMessage::CommentsLoaded {
                    story: Box::new(story_clone),
                    comments: root_comments.clone(),
                    pending_roots,
                });

                // Step 2: For each root comment, fetch its children progressively
                for c in &root_comments {
                    let child_ids = c.item.kids.clone().unwrap_or_default();
                    if child_ids.is_empty() {
                        continue;
                    }
                    let parent_id = CommentId(c.item.id);
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
                let (lines, links) = html_to_styled_lines(text.as_bytes(), width);
                let mut reader = ReaderState::new_loading(title, domain, None);
                reader.set_content(lines, links);
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
                Ok((lines, links)) => {
                    let _ = tx.send(AppMessage::ArticleLoaded { lines, links });
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

        open_http_url(url.as_deref());
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
                self.spawn_search(&query, page, LoadMode::Append);
            }
        } else if self.story_state.needs_more() {
            self.story_state.loading = true;
            self.spawn_load_stories(LoadMode::Append);
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
                let visible_len = self.comment_state.visible_len();
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

    /// Enters Quickjump hint-label mode. Determines context from current
    /// app state — reader if the article-reader overlay is open, comments
    /// if the comments pane is focused. No-op when the active surface has
    /// no labeled links (currently always the case for comments — see
    /// [`Self::active_link_registry`]).
    fn enter_hint_mode(&mut self, action: HintAction) {
        // Already in hint mode? Re-entering with a different action is
        // ambiguous; treat as a re-arm with the new action but reset the
        // buffer so the user can start fresh.
        let context = if self.reader_state.is_some() {
            HintContext::Reader
        } else if self.focus == Pane::Comments {
            HintContext::Comments
        } else {
            // Nothing to label.
            return;
        };

        // Refuse to enter if the active registry has no links — silent
        // no-op (a status-bar hint could be added later).
        if self
            .active_link_registry(context)
            .is_none_or(|r| r.is_empty())
        {
            return;
        }

        self.hint_state = Some(HintState::new(action, context));
        self.input_mode = InputMode::HintMode;
    }

    /// Cancels hint-label selection without firing an action. Restores
    /// the input mode so navigation keys work again.
    fn exit_hint_mode(&mut self) {
        self.hint_state = None;
        self.input_mode = InputMode::Normal;
    }

    /// Appends `c` to the hint prefix and resolves against the active
    /// registry: a unique match fires the configured action and exits
    /// hint mode; multiple matches keep narrowing; no match cancels.
    fn hint_key(&mut self, c: char) {
        let Some(hs) = self.hint_state.as_mut() else {
            return;
        };
        hs.push(c);

        let context = hs.context;
        let action = hs.action;
        let buffer = hs.buffer().to_string();

        let resolution = self
            .active_link_registry(context)
            .map(|r| match r.match_prefix(&buffer) {
                MatchResult::Unique(link) => HintResolve::Fire(link.url.clone()),
                MatchResult::Multiple => HintResolve::Continue,
                MatchResult::None => HintResolve::Cancel,
            })
            .unwrap_or(HintResolve::Cancel);

        match resolution {
            HintResolve::Continue => {}
            HintResolve::Cancel => self.exit_hint_mode(),
            HintResolve::Fire(url) => {
                self.exit_hint_mode();
                self.execute_hint_action(action, &url);
            }
        }
    }

    /// Returns the [`LinkRegistry`] backing the current hint context.
    ///
    /// For the reader, this is the article's pre-built registry. The
    /// comments path is currently a stub returning `None` — the per-frame
    /// registry build is scoped to a follow-up PR. Hint mode entered from
    /// the comments pane therefore degrades to a no-op via
    /// [`HintResolve::Cancel`] on the first key.
    fn active_link_registry(&self, context: HintContext) -> Option<&LinkRegistry> {
        match context {
            HintContext::Reader => self.reader_state.as_ref().map(|r| &r.links),
            HintContext::Comments => None, // TODO: build registry from visible comments on hint-mode entry
        }
    }

    /// Dispatches the configured hint action against the resolved URL.
    /// Open/OpenInReader go through the same scheme-validating
    /// [`open_http_url`] used elsewhere; CopyUrl emits OSC 52.
    fn execute_hint_action(&mut self, action: HintAction, url: &str) {
        match action {
            HintAction::Open => open_http_url(Some(url)),
            HintAction::OpenInReader => self.open_url_in_reader(url),
            HintAction::CopyUrl => {
                if let Err(e) = clipboard::copy(url) {
                    self.error = Some(format!("Clipboard write failed: {}", e));
                }
            }
        }
    }

    /// Opens a hint-resolved URL in the inline article reader (the same
    /// flow as `p`-on-a-story, but seeded from a labeled link rather
    /// than the focused story's URL). Drops non-http(s) schemes.
    fn open_url_in_reader(&mut self, url: &str) {
        let parsed = match url::Url::parse(url) {
            Ok(p) if matches!(p.scheme(), "http" | "https") => p,
            _ => return,
        };
        let domain = parsed.host_str().map(|s| s.to_string());
        let title = parsed.path().trim_matches('/').to_string();
        let title = if title.is_empty() {
            domain.clone().unwrap_or_else(|| url.to_string())
        } else {
            title
        };

        self.reader_state = Some(ReaderState::new_loading(
            title,
            domain,
            Some(url.to_string()),
        ));

        let tx = self.msg_tx.clone();
        let width = self.terminal_width.saturating_sub(6) as usize;
        let url_owned = url.to_string();
        tokio::spawn(async move {
            match fetch_and_extract_article(&url_owned, width).await {
                Ok((lines, links)) => {
                    let _ = tx.send(AppMessage::ArticleLoaded { lines, links });
                }
                Err(e) => {
                    let _ = tx.send(AppMessage::ArticleError(e));
                }
            }
        });
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

/// Opens `url` in the system browser — but only when it parses as an
/// `http`/`https` URL. Silently drops `None`, parse failures, and other
/// schemes (so `file://`, `javascript:`, and `data:` can never reach
/// `open::that`). All three overlay-dispatch sites and
/// [`App::open_in_browser`] share this entry point.
fn open_http_url(url: Option<&str>) {
    let Some(raw) = url else { return };
    let Ok(parsed) = url::Url::parse(raw) else {
        return;
    };
    if matches!(parsed.scheme(), "http" | "https") {
        let _ = open::that(parsed.as_str());
    }
}
