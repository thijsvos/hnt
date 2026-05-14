# Changelog

All notable changes to `hnt` are documented in this file.

The format is loosely based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.4.0] — 2026-05-14

A security- and stability-focused release. A code-review pass surfaced
four critical issues and fourteen warnings; this release closes all
eighteen and adds ~150 new tests along the way.

### Security

- **SSRF guard on the initial article fetch.** The article-reader's
  redirect callback already blocked redirects into private/loopback IP
  ranges, but the *initial* `client.get(url).send()` accepted any
  literal host. A malicious HN submission with URL
  `http://169.254.169.254/...` (AWS IMDS) or `http://192.168.1.1/admin`
  could probe the user's internal network from inside their machine and
  render the response in the reader. Now mirrors the redirect check
  before the first request, with `tokio::net::lookup_host` resolving
  hostnames so a domain pointing at a private IP is also rejected.
  Best-effort against DNS rebinding (`reqwest` re-resolves at connect
  time). ([#128](https://github.com/thijsvos/hnt/issues/128),
  [#146](https://github.com/thijsvos/hnt/pull/146))
- **Terminal-escape sanitisation of story titles and error messages.**
  The comments-pane block title and the article-reader overlay title
  were interpolating `story.title` directly into a `Span`, bypassing the
  C0/C1/DEL scrub the rest of the codebase applied. Status-bar error
  text and article-fetch error messages were also raw. A malicious HN
  title containing `\x1b]0;OWNED\x07` would rewrite the user's terminal
  tab; a malicious error response would do the same via DNS or
  `Location:` header. All five sinks now run through `sanitize_terminal`
  at the rendering boundary.
  ([#129](https://github.com/thijsvos/hnt/issues/129),
  [#147](https://github.com/thijsvos/hnt/pull/147),
  [#132](https://github.com/thijsvos/hnt/issues/132),
  [#154](https://github.com/thijsvos/hnt/pull/154))
- **README/markdown content sanitisation.** Markdown READMEs from
  `raw.githubusercontent.com` and the gitlab.com `-/raw/HEAD/...`
  endpoint were not run through `sanitize_terminal`. A malicious
  project README could embed OSC/CSI escapes in any heading, body, or
  list item and have them ride through the reader. All markdown
  fragments are now scrubbed.
  ([#133](https://github.com/thijsvos/hnt/issues/133),
  [#155](https://github.com/thijsvos/hnt/pull/155))

### Stability

- **Generation-counter gating for async results.** Every `tokio::spawn`
  for feed loads, search, comment-tree walks, article fetches, and
  prior-discussions queries was fire-and-forget. A feed switch
  (1 → 2 → 3) while feed-1's fetch was still in flight would land
  feed-1's stories on top of feed-3's pane; story A's late
  `CommentsDone` would clear B's loading spinner; search A's
  `total_pages` would write into B's state. Three monotonic
  generation counters now tag every async message; stale results are
  silently dropped by `process_messages`.
  ([#130](https://github.com/thijsvos/hnt/issues/130),
  [#148](https://github.com/thijsvos/hnt/pull/148))
- **Refresh actually refetches prior discussions.** Pressing `r` on a
  story already in the `prior_results` cache short-circuited at the
  `contains(&story_id)` guard and served stale data. The cache now
  clears on every `reset_panes_and_reload` (covers Refresh, feed
  switch, and search cancel).
  ([#131](https://github.com/thijsvos/hnt/issues/131),
  [#149](https://github.com/thijsvos/hnt/pull/149))
- **Pinned-feed pagination refetches the pin list.** The
  `LoadMode::Append` branch for `FeedKind::Pinned` was reusing the
  cached `all_ids`, so stories pinned (or unpinned) since the initial
  load were silently missing (or still fetched). The Pinned-feed
  Append branch now re-reads `pin_store.pinned_ids_newest_first()` and
  ships the refreshed list as `all_ids` so subsequent pages stay
  consistent.
  ([#135](https://github.com/thijsvos/hnt/issues/135),
  [#156](https://github.com/thijsvos/hnt/pull/156))
- **Startup loading placeholder appears.** `load_initial_feed` never
  set `story_state.loading = true`, so the window between `App::new`
  and the first `StoriesLoaded` rendered "No stories loaded" instead
  of "Loading stories…". The flip moved into `spawn_load_stories`
  itself — the single chokepoint for every story-list spawn.
  ([#134](https://github.com/thijsvos/hnt/issues/134),
  [#150](https://github.com/thijsvos/hnt/pull/150))
- **Bounded `process_messages` drain per call.** Under a comment-thread
  flood (`buffer_unordered(20)` × an O(n) `Vec::splice` per
  `CommentsAppended`) the render loop could spend a whole tick draining
  without painting a frame. Capped to 32 messages per call; the rest
  pick up on the next tick.
  ([#139](https://github.com/thijsvos/hnt/issues/139),
  [#158](https://github.com/thijsvos/hnt/pull/158))
- **Reader `scroll_percent` 0% for unscrollable content.** A one-line
  article or empty reader was reporting "100%" in the footer; the
  guard now returns 0 when there's nothing to scroll.
  ([#141](https://github.com/thijsvos/hnt/issues/141),
  [#151](https://github.com/thijsvos/hnt/pull/151))
- **`markdown_to_styled_lines` on width == 0 emits a placeholder.** The
  else branch fell through, caching unwrapped full-width fragments
  that rendered broken layout after a resize.
  ([#142](https://github.com/thijsvos/hnt/issues/142),
  [#152](https://github.com/thijsvos/hnt/pull/152))

### Performance

- **Generation-counter gating ([#148](https://github.com/thijsvos/hnt/pull/148))**
  also avoids applying `Vec::splice` and `set_content` for results the
  user will never see.
- **`filter_visible_set` memoised** by `(filter, len)`, with the result
  held in an `Rc<HashSet<usize>>` so `visible_indices_iter` clones the
  pointer (O(1)) instead of rebuilding the O(n) path-stack walk every
  frame.
  ([#137](https://github.com/thijsvos/hnt/issues/137),
  [#157](https://github.com/thijsvos/hnt/pull/157))
- **`descendant_count` precomputed per `FlatComment`.**
  `count_hidden_children` was re-scanning the comments-slice tail per
  call — effectively O(n²) on threads with many widely-collapsed
  subtrees rendered every frame. Now O(1) via a precomputed field
  filled on `set_comments` / `insert_children`.
  ([#138](https://github.com/thijsvos/hnt/issues/138),
  [#159](https://github.com/thijsvos/hnt/pull/159))
- **Per-frame `indent+bar` allocations eliminated.** The comment-tree
  renderer was `format!`ing the depth-prefixed thread bar once per
  header row and once per wrapped body line — ~80k allocations/sec on
  an idle 50-comment pane at 4 Hz. Replaced with a constant
  `INDENT_BARS: [&str; 11]` lookup so the Spans take `&'static str`
  content (`Cow::Borrowed`, zero allocation). The remaining per-comment
  allocations (author/time/body) are tracked for a future
  `Span<'a>`-lifetime refactor.
  ([#136](https://github.com/thijsvos/hnt/issues/136),
  [#162](https://github.com/thijsvos/hnt/pull/162))
- **`Arc<Item>` propagates through the story-load path.** `fetch_items`
  used to deref-clone the cached `Arc<Item>` at its boundary into
  `Vec<Option<Item>>`, undoing the cache work. The `Arc` now travels
  end-to-end through `AppMessage::StoriesLoaded`, `apply_loaded_stories`,
  and into `StoryListState::stories: Vec<Arc<Item>>` with zero deep
  clones on the boundary. `FlatComment::item` still owns `Item`
  directly — that conversion is the remaining follow-up.
  ([#140](https://github.com/thijsvos/hnt/issues/140),
  [#163](https://github.com/thijsvos/hnt/pull/163))

### Documentation

- **README keybindings table updated.** Feed switch row now reads
  `` `1`-`7` `` (added Pinned), and a new `b` row covers pin/unpin.
  Features list bumps "6 feeds" to "7 feeds".
  ([#143](https://github.com/thijsvos/hnt/issues/143),
  [#153](https://github.com/thijsvos/hnt/pull/153))
- **`App` field docs filled in.** The 14 fields at the top of the
  struct (running, current_feed, focus, story_state, …, tick_count)
  were left bare after the rustdoc push in #123; every field now has a
  one-liner.
  ([#144](https://github.com/thijsvos/hnt/issues/144),
  [#160](https://github.com/thijsvos/hnt/pull/160))

### Tests

- **`app.rs` state-machine test module.** Started in #148 and rounded
  out in #161: 32 tests covering `apply_loaded_stories` for both
  `LoadMode`s, the `cycle_comment_filter` four-arm matrix, `pane_at`
  rect math and boundary checks, `PriorInFlightGuard` Drop including
  the poisoned-mutex path, plus the generation-gating invariants for
  every gated `AppMessage` variant.
  ([#145](https://github.com/thijsvos/hnt/issues/145),
  [#161](https://github.com/thijsvos/hnt/pull/161))
- Suite grew from ~300 tests pre-release to **368** at the end of the
  cycle.

## [0.3.13] — 2026-04-25

Final 0.3.x release. See `git log` for individual commits; the 0.3.x
series predates this changelog file.

[0.4.0]: https://github.com/thijsvos/hnt/releases/tag/v0.4.0
[0.3.13]: https://github.com/thijsvos/hnt/releases/tag/v0.3.13
