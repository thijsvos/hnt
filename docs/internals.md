# Internals

Notes on the load-bearing-but-non-obvious mechanisms in the codebase, for
maintainers and curious users.

## Async architecture

The app is a single tokio runtime hosting a `crossterm` event loop in the
main task plus N spawned tasks for HTTP fetches. They communicate via an
unbounded MPSC channel:

```
keys/mouse ──► main loop ──► dispatch ──► tokio::spawn(fetch)
                  ▲                            │
                  └───────── AppMessage ◄──────┘
```

`process_messages` drains the channel each tick with a per-frame budget
(`PROCESS_MESSAGES_BUDGET = 32`) so a `CommentsAppended` flood from a deep
thread can't starve render.

## Generation-counter result gating

Every `tokio::spawn` for feed loads, search, comment-tree walks, and
article fetches used to be fire-and-forget — a stale result could land
on top of post-action state (feed switch race, story-select race, search
pagination race). Three monotonic `u64` counters on `App` (`feed_gen`,
`story_gen`, `article_gen`) tag every spawned task at spawn time. Each
gated `AppMessage` variant carries the snapshot, and `process_messages`
drops the message when the snapshot doesn't match the current counter.

The state-change points that bump counters: `reset_panes_and_reload`,
`submit_search`, `load_selected_comments`, `open_article_reader`,
`open_url_in_reader`, the comments-pane `Back` arm, the reader-overlay
`Back` arm. `Error` and `PriorDiscussionsLoaded` are intentionally
ungated.

## SSRF guard

The article reader (`p`) fetches user-supplied URLs. To prevent abuse
(e.g. an HN submission of `http://192.168.1.1/admin` probing the user's
LAN), `check_host_is_public` runs before the initial `client.get()`:

1. If the host is a literal IP (parsed via `url::Url::host()`), reject if
   it's in any private / loopback / link-local / unique-local range
   (`is_private_ip`).
2. If the host is a hostname, resolve via `tokio::net::lookup_host` and
   reject if any resolved address is private.

This is best-effort against DNS rebinding (`reqwest` re-resolves at
connect time), but closes the literal-IP and obvious hostname cases.

The redirect callback inside the `reqwest::Client` builder applies the
same `is_private_ip` check to every redirect target as a second layer.

## Terminal-escape sanitisation

Every untrusted-content boundary that flows into a ratatui `Span` is
scrubbed through `sanitize::sanitize_terminal` — it replaces C0 / C1 /
DEL bytes with the Unicode replacement character. Sanitised paths:

- Story titles in the comments-pane block title (`ui::comment_tree::build_block_title_label`).
- Reader overlay title + domain (sanitised inside `ReaderState::new_loading`).
- Status-bar error display (`ui::status_bar::StatusBar::render`).
- Article body fragments (`article.rs::tagged_lines_to_styled_with_links`).
- Markdown README content (`article.rs::markdown_to_styled_lines`).
- Plain-text README fallback (`article.rs::fetch_and_extract_article`'s RST/plain path).
- Comment HTML body cache (`state::comment_state::FlatComment::plain_text`).

A malicious HN title containing `\x1b]0;OWNED\x07` will not rewrite the
user's terminal tab title.

## Quickjump (`f` / `F` / `y` in reader)

When the user presses `f` (or `F`, or `y`) in the article reader, the
reader's `LinkRegistry` is consulted. Each hyperlink gets a 1- or 2-
character label drawn from a curated alphabet (`asdfweiou` — home-row,
non-collisional with vim navigation keys). Labels are assigned uniformly
in `LinkRegistry::assign_labels` once at content-set time, then painted
over the reader by `paint_hint_labels` on every render while
`HintState` is active.

Typing the label characters narrows the match. A unique match fires
the configured `HintAction` (`Open` browser / `OpenInReader` / `CopyUrl`
via OSC-52). A non-matching prefix cancels.

OSC-52 clipboard works through SSH because the escape sequence is
interpreted by the user's *local* terminal emulator (iTerm2, kitty,
WezTerm, etc.), not by the remote machine.

## Comment-tree memoisation

Two caches make the comments pane O(1)-per-frame for the common case:

- `CommentTreeState::filter_cache` — memoises the "what's new" filter's
  visible-index set, keyed by `(filter, comments.len())`. `Rc<HashSet>`
  so each `visible_indices_iter` call clones a pointer rather than
  rebuilding the O(n) path-stack walk.
- `FlatComment::descendant_count` — precomputed at `set_comments` /
  `insert_children` time. `count_hidden_children` reads the field in
  O(1) instead of scanning the comments-slice tail per call.

## Persistence layer

`state::persist::JsonStore<E>` is the shared atomic-write + LRU-bounded
JSON store used by both `read_store.rs` and `pin_store.rs`. Generic over
an entry type that implements `PersistedEntry` (provides an `age_key`
used for eviction).

See `docs/configuration.md` for the per-platform paths.

## Where to look first

- New feature: `keys.rs` (keymap) + `app.rs` (`dispatch_normal` /
  `dispatch_reader` / `dispatch_prior`) + the relevant pane state.
- New bug: `app.rs` `process_messages` is the choke point; check
  whether the affected variant is gated and whether the gen invariant
  matches.
- New panic: `tui.rs` panic hook restores the terminal on crash.
- New flaky test: `app.rs` `#[cfg(test)] mod tests` covers the gating
  + state machine; `comment_state.rs` covers the comment tree.
