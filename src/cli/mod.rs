//! Headless / scriptable command-line mode.
//!
//! When `hnt` is invoked with a subcommand (`hnt top --json`, `hnt thread
//! 123`, `hnt article 123`, …) it runs **headless**: it prints to stdout and
//! exits without ever entering raw mode or the alternate screen. With no
//! arguments, [`parse`] returns `Ok(None)` and `main` falls through to the
//! interactive TUI.
//!
//! The surface deliberately mirrors the in-app `:`-commands (`feed`, `open`,
//! `search`, `reader`/`article`) so the two interfaces stay consistent. All
//! network access reuses [`crate::api::client::HnClient`] and
//! `crate::article` — headless mode adds no new endpoints and stays
//! local-only, exactly like the TUI.

mod output;
mod render;

use crate::api::client::HnClient;
use crate::api::types::{CommentWithDepth, FeedKind, Item};
use anyhow::{Context, Result};
use std::io::Write;
use std::sync::Arc;

/// Default item cap for feed listings and search results.
const DEFAULT_LIMIT: usize = 30;
/// Default maximum comment nesting fetched by `thread` — generous enough for
/// a one-shot print without unbounded fan-out on pathological threads.
const DEFAULT_MAX_DEPTH: usize = 12;
/// Render width for comment/article bodies in the text output.
const TEXT_WIDTH: usize = 80;

/// `--help` / `--version` body and the short usage line shown on errors.
const HELP_TEXT: &str = "\
hnt — a dark-themed terminal Hacker News reader

USAGE:
    hnt                          Launch the interactive TUI (default)
    hnt <feed> [options]         List a feed: top new best ask show jobs pinned
    hnt feed <name> [options]    Same, explicit form
    hnt thread <id> [options]    Print a story's comment thread (alias: comments)
    hnt open <id> [options]      Print a single item (alias: item)
    hnt search <query…> [opts]   Search Hacker News via Algolia
    hnt article <id|url>         Print extracted article text (alias: read)
    hnt --help | --version

OPTIONS:
    --json            Emit JSON (a stable contract) instead of text
    --digest          One compact line per story (feed listings only)
    --limit N         Max items (feeds/search default 30; thread: top-level cap)
    --max-depth N     Max comment nesting for `thread` (default 12)

EXAMPLES:
    hnt top --limit 10 --json | jq -r '.[].title'
    hnt thread 38911 > thread.txt
    hnt article 38911 | less
    hnt search rust async --json
    hnt top --digest | mail -s 'HN today' me

No arguments launches the full-screen reader. Output is local-only — hnt
talks only to the Hacker News API and Algolia, exactly like the TUI.
";

/// A user-facing argument/usage error. Rendered to stderr by `main`, which
/// then exits with code 2.
#[derive(Debug, PartialEq, Eq)]
pub struct UsageError(pub String);

impl std::fmt::Display for UsageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Builds a [`UsageError`] from anything string-like.
fn ue(msg: impl Into<String>) -> UsageError {
    UsageError(msg.into())
}

/// Output format for a listing command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Multi-line human-readable text (default).
    Text,
    /// JSON via the [`output`] contract.
    Json,
    /// One compact line per story.
    Digest,
}

/// A fully-parsed headless invocation. `main` runs it via [`run`].
#[derive(Debug, PartialEq, Eq)]
pub enum Invocation {
    /// Print `--help`.
    Help,
    /// Print the version.
    Version,
    /// List a feed.
    Feed {
        /// Which feed to list.
        kind: FeedKind,
        /// Maximum rows.
        limit: usize,
        /// Output format.
        format: Format,
    },
    /// Print a story's comment thread.
    Thread {
        /// HN story id.
        id: u64,
        /// Maximum comment nesting depth.
        max_depth: usize,
        /// Optional cap on top-level comments.
        limit: Option<usize>,
        /// Emit JSON instead of text.
        json: bool,
    },
    /// Print a single item (story or comment).
    Item {
        /// HN item id.
        id: u64,
        /// Emit JSON instead of text.
        json: bool,
    },
    /// Search Hacker News.
    Search {
        /// Space-joined query.
        query: String,
        /// Maximum results.
        limit: usize,
        /// Emit JSON instead of text.
        json: bool,
    },
    /// Extract and print article text for an id or URL.
    Article {
        /// An HN item id (numeric) or a direct URL.
        target: String,
    },
}

/// Collected flags + positionals from a subcommand's argument tail.
#[derive(Default)]
struct Flags {
    json: bool,
    digest: bool,
    limit: Option<usize>,
    max_depth: Option<usize>,
    positionals: Vec<String>,
}

impl Flags {
    /// Scans `tokens`, recognizing `--json`, `--digest`, `--limit[ =]N`,
    /// `--max-depth[ =]N`; anything else starting with `-` is an error and
    /// bare tokens are collected as positionals.
    fn parse(tokens: &[String]) -> Result<Self, UsageError> {
        let mut f = Flags::default();
        let mut i = 0;
        while i < tokens.len() {
            let t = tokens[i].as_str();
            match t {
                "--json" => f.json = true,
                "--digest" => f.digest = true,
                "--limit" => {
                    i += 1;
                    f.limit = Some(num_from_next(tokens.get(i), "--limit", false)?);
                }
                "--max-depth" => {
                    i += 1;
                    f.max_depth = Some(num_from_next(tokens.get(i), "--max-depth", true)?);
                }
                _ if t.starts_with("--limit=") => {
                    f.limit = Some(parse_num(&t["--limit=".len()..], "--limit", false)?);
                }
                _ if t.starts_with("--max-depth=") => {
                    f.max_depth = Some(parse_num(&t["--max-depth=".len()..], "--max-depth", true)?);
                }
                _ if t.starts_with('-') => return Err(ue(format!("unknown flag: {t}"))),
                _ => f.positionals.push(tokens[i].clone()),
            }
            i += 1;
        }
        Ok(f)
    }

    /// Resolves `--json`/`--digest` into a [`Format`], rejecting the
    /// combination.
    fn format(&self) -> Result<Format, UsageError> {
        match (self.json, self.digest) {
            (true, true) => Err(ue("--json and --digest are mutually exclusive")),
            (true, false) => Ok(Format::Json),
            (false, true) => Ok(Format::Digest),
            (false, false) => Ok(Format::Text),
        }
    }
}

/// Parses a numeric flag value supplied as the next token.
fn num_from_next(tok: Option<&String>, flag: &str, allow_zero: bool) -> Result<usize, UsageError> {
    match tok {
        Some(v) => parse_num(v, flag, allow_zero),
        None => Err(ue(format!("{flag} requires a number"))),
    }
}

/// Parses a numeric flag value. Always rejects non-numeric input; rejects
/// `0` unless `allow_zero` — true for `--max-depth` (0 means "root comments
/// only"), false for `--limit` (0 would request an empty listing, almost
/// always a mistake).
fn parse_num(v: &str, flag: &str, allow_zero: bool) -> Result<usize, UsageError> {
    let n = v
        .parse::<usize>()
        .map_err(|_| ue(format!("{flag} expects a number, got {v:?}")))?;
    if n == 0 && !allow_zero {
        return Err(ue(format!("{flag} must be greater than 0")));
    }
    Ok(n)
}

/// Extracts exactly one numeric id from a command's positionals.
fn single_id(positionals: &[String], cmd: &str) -> Result<u64, UsageError> {
    match positionals {
        [one] => one
            .parse::<u64>()
            .map_err(|_| ue(format!("{cmd}: invalid id {one:?}"))),
        [] => Err(ue(format!("{cmd} requires an item id"))),
        _ => Err(ue(format!("{cmd} takes a single id"))),
    }
}

/// Parses the process arguments (everything after the program name) into an
/// [`Invocation`].
///
/// Returns `Ok(None)` when there are no arguments — the signal for `main` to
/// launch the interactive TUI. Returns `Err` with a user-facing message for
/// unknown commands, bad flags, or malformed values. This function performs
/// no I/O so it is fully unit-testable.
pub fn parse(args: &[String]) -> Result<Option<Invocation>, UsageError> {
    let Some(first) = args.first() else {
        return Ok(None);
    };
    let rest = &args[1..];
    match first.as_str() {
        "-h" | "--help" | "help" => Ok(Some(Invocation::Help)),
        "-V" | "--version" | "version" => Ok(Some(Invocation::Version)),
        "feed" => match rest.first() {
            Some(name) => parse_feed(name, &rest[1..]),
            None => Err(ue(
                "feed requires a name (top, new, best, ask, show, jobs, pinned)",
            )),
        },
        "thread" | "comments" => parse_thread(rest),
        "open" | "item" => parse_item(rest),
        "search" => parse_search(rest),
        "article" | "read" => parse_article(rest),
        other if FeedKind::from_name(other).is_some() => parse_feed(other, rest),
        other if other.starts_with('-') => {
            Err(ue(format!("unknown option: {other} (no subcommand given)")))
        }
        other => Err(ue(format!("unknown command: {other}"))),
    }
}

/// Parses a feed listing (`hnt top …` / `hnt feed top …`).
fn parse_feed(name: &str, flags: &[String]) -> Result<Option<Invocation>, UsageError> {
    let kind = FeedKind::from_name(name).ok_or_else(|| {
        ue(format!(
            "unknown feed: {name} (try top, new, best, ask, show, jobs, pinned)"
        ))
    })?;
    let f = Flags::parse(flags)?;
    if let Some(extra) = f.positionals.first() {
        return Err(ue(format!("unexpected argument: {extra}")));
    }
    if f.max_depth.is_some() {
        return Err(ue("--max-depth only applies to `thread`"));
    }
    let format = f.format()?;
    Ok(Some(Invocation::Feed {
        kind,
        limit: f.limit.unwrap_or(DEFAULT_LIMIT),
        format,
    }))
}

/// Parses `hnt thread <id> …`.
fn parse_thread(tokens: &[String]) -> Result<Option<Invocation>, UsageError> {
    let f = Flags::parse(tokens)?;
    if f.digest {
        return Err(ue("--digest applies only to feed listings"));
    }
    let id = single_id(&f.positionals, "thread")?;
    Ok(Some(Invocation::Thread {
        id,
        max_depth: f.max_depth.unwrap_or(DEFAULT_MAX_DEPTH),
        limit: f.limit,
        json: f.json,
    }))
}

/// Parses `hnt open <id> …`.
fn parse_item(tokens: &[String]) -> Result<Option<Invocation>, UsageError> {
    let f = Flags::parse(tokens)?;
    if f.digest {
        return Err(ue("--digest applies only to feed listings"));
    }
    let id = single_id(&f.positionals, "open")?;
    Ok(Some(Invocation::Item { id, json: f.json }))
}

/// Parses `hnt search <query…> …`.
fn parse_search(tokens: &[String]) -> Result<Option<Invocation>, UsageError> {
    let f = Flags::parse(tokens)?;
    if f.digest {
        return Err(ue("--digest applies only to feed listings"));
    }
    if f.positionals.is_empty() {
        return Err(ue("search requires a query"));
    }
    Ok(Some(Invocation::Search {
        query: f.positionals.join(" "),
        limit: f.limit.unwrap_or(DEFAULT_LIMIT),
        json: f.json,
    }))
}

/// Parses `hnt article <id|url>`.
fn parse_article(tokens: &[String]) -> Result<Option<Invocation>, UsageError> {
    let f = Flags::parse(tokens)?;
    if f.json || f.digest || f.limit.is_some() || f.max_depth.is_some() {
        return Err(ue("article takes no options, just an id or URL"));
    }
    match f.positionals.as_slice() {
        [target] => Ok(Some(Invocation::Article {
            target: target.clone(),
        })),
        [] => Err(ue("article requires an id or URL")),
        _ => Err(ue("article takes a single id or URL")),
    }
}

/// Runs a parsed [`Invocation`], returning the process exit code (`0` on
/// success, `1` when a requested item could not be found). Network/IO
/// failures propagate as `Err` for `main` to print.
pub async fn run(inv: Invocation) -> Result<i32> {
    match inv {
        Invocation::Help => {
            print!("{HELP_TEXT}");
            Ok(0)
        }
        Invocation::Version => {
            println!("hnt {}", env!("CARGO_PKG_VERSION"));
            Ok(0)
        }
        Invocation::Feed {
            kind,
            limit,
            format,
        } => run_feed(kind, limit, format).await,
        Invocation::Thread {
            id,
            max_depth,
            limit,
            json,
        } => run_thread(id, max_depth, limit, json).await,
        Invocation::Item { id, json } => run_item(id, json).await,
        Invocation::Search { query, limit, json } => run_search(&query, limit, json).await,
        Invocation::Article { target } => run_article(&target).await,
    }
}

/// Fetches and prints a feed listing.
async fn run_feed(kind: FeedKind, limit: usize, format: Format) -> Result<i32> {
    let client = HnClient::new();
    let items: Vec<Arc<Item>> = if kind == FeedKind::Pinned {
        // Pinned is a virtual feed backed by the local pin store — no remote
        // endpoint. Load ids from disk, then hydrate via the same item fetch.
        let ids = crate::state::pin_store::PinStore::load().pinned_ids_newest_first();
        let ids: Vec<u64> = ids.into_iter().take(limit).collect();
        client
            .fetch_items(&ids)
            .await
            .into_iter()
            .flatten()
            .filter(|i| !i.is_dead_or_deleted())
            .collect()
    } else {
        let (items, _all_ids) = client
            .fetch_stories(kind, 0, limit)
            .await
            .with_context(|| format!("failed to fetch {kind} feed"))?;
        items
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    match format {
        Format::Json => {
            serde_json::to_writer_pretty(&mut out, &output::stories(&items))?;
            writeln!(out)?;
        }
        Format::Digest => render::digest(&mut out, &items)?,
        Format::Text => render::feed(&mut out, &items)?,
    }
    Ok(0)
}

/// Fetches and prints a story's comment thread.
async fn run_thread(id: u64, max_depth: usize, limit: Option<usize>, json: bool) -> Result<i32> {
    let client = HnClient::new();
    let Some(story) = client
        .fetch_item(id)
        .await
        .with_context(|| format!("failed to fetch story {id}"))?
    else {
        eprintln!("hnt: no item with id {id}");
        return Ok(1);
    };

    let mut roots: Vec<u64> = story.kids.as_deref().unwrap_or(&[]).to_vec();
    if let Some(n) = limit {
        roots.truncate(n);
    }
    let mut flat: Vec<CommentWithDepth> = Vec::new();
    client
        .fetch_children_recursive(&roots, 0, max_depth, &mut flat)
        .await;

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if json {
        let payload = output::OutThread {
            story: output::OutStory::from_item(story.as_ref()),
            comments: flat.iter().map(output::OutComment::from_comment).collect(),
        };
        serde_json::to_writer_pretty(&mut out, &payload)?;
        writeln!(out)?;
    } else {
        render::thread(&mut out, story.as_ref(), &flat)?;
    }
    Ok(0)
}

/// Fetches and prints a single item.
async fn run_item(id: u64, json: bool) -> Result<i32> {
    let client = HnClient::new();
    let Some(item) = client
        .fetch_item(id)
        .await
        .with_context(|| format!("failed to fetch item {id}"))?
    else {
        eprintln!("hnt: no item with id {id}");
        return Ok(1);
    };

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if json {
        serde_json::to_writer_pretty(&mut out, &output::OutStory::from_item(item.as_ref()))?;
        writeln!(out)?;
    } else if item.title.is_some() {
        render::feed(&mut out, std::slice::from_ref(&item))?;
    } else {
        // A comment (no title) — print author + body.
        writeln!(
            out,
            "by {} · id {}",
            crate::sanitize::sanitize_terminal(item.by.as_deref().unwrap_or("unknown")),
            item.id,
        )?;
        for line in render::html_to_plain(item.text.as_deref(), TEXT_WIDTH).lines() {
            writeln!(out, "{line}")?;
        }
    }
    Ok(0)
}

/// Runs an Algolia search and prints the results.
async fn run_search(query: &str, limit: usize, json: bool) -> Result<i32> {
    let client = HnClient::new();
    let (hits, _pages, _total) = client
        .search_stories(query, 0, limit)
        .await
        .with_context(|| format!("search failed for {query:?}"))?;
    let hits: Vec<Arc<Item>> = hits.into_iter().take(limit).map(Arc::new).collect();

    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    if json {
        serde_json::to_writer_pretty(&mut out, &output::stories(&hits))?;
        writeln!(out)?;
    } else {
        render::feed(&mut out, &hits)?;
    }
    Ok(0)
}

/// Extracts and prints article text for an id or URL.
async fn run_article(target: &str) -> Result<i32> {
    // A numeric target is an HN item id; resolve it to the story's external
    // URL (or fall back to its text body for an HN-native post).
    let url = if !target.is_empty() && target.bytes().all(|b| b.is_ascii_digit()) {
        let id: u64 = target
            .parse()
            .with_context(|| format!("invalid id {target}"))?;
        let client = HnClient::new();
        let Some(item) = client
            .fetch_item(id)
            .await
            .with_context(|| format!("failed to fetch item {id}"))?
        else {
            eprintln!("hnt: no item with id {id}");
            return Ok(1);
        };
        match item.url.clone() {
            Some(u) => u,
            None => {
                let body = render::html_to_plain(item.text.as_deref(), TEXT_WIDTH);
                if body.trim().is_empty() {
                    eprintln!("hnt: item {id} has no article URL or text body");
                    return Ok(1);
                }
                let stdout = std::io::stdout();
                let mut out = stdout.lock();
                for line in body.lines() {
                    writeln!(out, "{line}")?;
                }
                return Ok(0);
            }
        }
    } else {
        target.to_string()
    };

    let (lines, _links) = crate::article::fetch_and_extract_article(&url, TEXT_WIDTH)
        .await
        .with_context(|| format!("failed to extract article from {url}"))?;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    render::article(&mut out, &lines)?;
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Parse helper taking string literals.
    fn p(args: &[&str]) -> Result<Option<Invocation>, UsageError> {
        let owned: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        parse(&owned)
    }

    #[test]
    fn no_args_falls_through_to_tui() {
        assert_eq!(p(&[]), Ok(None));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(p(&["--help"]), Ok(Some(Invocation::Help)));
        assert_eq!(p(&["-h"]), Ok(Some(Invocation::Help)));
        assert_eq!(p(&["help"]), Ok(Some(Invocation::Help)));
        assert_eq!(p(&["--version"]), Ok(Some(Invocation::Version)));
        assert_eq!(p(&["-V"]), Ok(Some(Invocation::Version)));
    }

    #[test]
    fn feed_shorthand_defaults() {
        assert_eq!(
            p(&["top"]),
            Ok(Some(Invocation::Feed {
                kind: FeedKind::Top,
                limit: DEFAULT_LIMIT,
                format: Format::Text,
            }))
        );
    }

    #[test]
    fn feed_explicit_with_flags() {
        assert_eq!(
            p(&["feed", "best", "--limit", "5", "--json"]),
            Ok(Some(Invocation::Feed {
                kind: FeedKind::Best,
                limit: 5,
                format: Format::Json,
            }))
        );
    }

    #[test]
    fn feed_digest_and_equals_limit() {
        assert_eq!(
            p(&["new", "--limit=3", "--digest"]),
            Ok(Some(Invocation::Feed {
                kind: FeedKind::New,
                limit: 3,
                format: Format::Digest,
            }))
        );
    }

    #[test]
    fn feed_json_and_digest_conflict() {
        assert!(p(&["top", "--json", "--digest"]).is_err());
    }

    #[test]
    fn feed_rejects_max_depth() {
        assert!(p(&["top", "--max-depth", "3"]).is_err());
    }

    #[test]
    fn feed_unknown_name_errors() {
        assert!(p(&["bogusfeed"]).is_err());
        assert!(p(&["feed", "bogus"]).is_err());
        assert!(p(&["feed"]).is_err());
    }

    #[test]
    fn pinned_feed_parses() {
        assert_eq!(
            p(&["pinned"]),
            Ok(Some(Invocation::Feed {
                kind: FeedKind::Pinned,
                limit: DEFAULT_LIMIT,
                format: Format::Text,
            }))
        );
    }

    #[test]
    fn thread_defaults_and_flags() {
        assert_eq!(
            p(&["thread", "123"]),
            Ok(Some(Invocation::Thread {
                id: 123,
                max_depth: DEFAULT_MAX_DEPTH,
                limit: None,
                json: false,
            }))
        );
        assert_eq!(
            p(&[
                "thread",
                "123",
                "--json",
                "--max-depth",
                "3",
                "--limit",
                "10"
            ]),
            Ok(Some(Invocation::Thread {
                id: 123,
                max_depth: 3,
                limit: Some(10),
                json: true,
            }))
        );
    }

    #[test]
    fn thread_allows_max_depth_zero_root_only() {
        // depth 0 is a legitimate request (root comments, no recursion);
        // only --limit rejects 0.
        assert_eq!(
            p(&["thread", "5", "--max-depth", "0"]),
            Ok(Some(Invocation::Thread {
                id: 5,
                max_depth: 0,
                limit: None,
                json: false,
            }))
        );
        assert!(p(&["thread", "5", "--limit", "0"]).is_err());
    }

    #[test]
    fn thread_requires_numeric_id() {
        assert!(p(&["thread", "abc"]).is_err());
        assert!(p(&["thread"]).is_err());
        assert!(p(&["comments", "1", "2"]).is_err());
    }

    #[test]
    fn open_item_aliases() {
        assert_eq!(
            p(&["open", "42", "--json"]),
            Ok(Some(Invocation::Item { id: 42, json: true }))
        );
        assert_eq!(
            p(&["item", "42"]),
            Ok(Some(Invocation::Item {
                id: 42,
                json: false,
            }))
        );
    }

    #[test]
    fn search_joins_query() {
        assert_eq!(
            p(&["search", "rust", "async", "--limit", "5"]),
            Ok(Some(Invocation::Search {
                query: "rust async".into(),
                limit: 5,
                json: false,
            }))
        );
        assert!(p(&["search"]).is_err());
    }

    #[test]
    fn article_id_or_url() {
        assert_eq!(
            p(&["article", "99"]),
            Ok(Some(Invocation::Article {
                target: "99".into(),
            }))
        );
        assert_eq!(
            p(&["read", "https://example.com"]),
            Ok(Some(Invocation::Article {
                target: "https://example.com".into(),
            }))
        );
        assert!(p(&["article"]).is_err());
        assert!(p(&["article", "1", "2"]).is_err());
        assert!(p(&["article", "1", "--json"]).is_err());
    }

    #[test]
    fn unknown_command_and_leading_flag() {
        assert!(p(&["frobnicate"]).is_err());
        assert!(p(&["--json"]).is_err());
    }

    #[test]
    fn bad_limit_value() {
        assert!(p(&["top", "--limit", "x"]).is_err());
        assert!(p(&["top", "--limit", "0"]).is_err());
        assert!(p(&["top", "--limit"]).is_err());
    }
}
