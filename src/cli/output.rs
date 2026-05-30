//! Stable JSON output contract for headless mode.
//!
//! These [`serde::Serialize`] structs are intentionally decoupled from the
//! wire-level `crate::api::types::Item` so the public `--json` shape stays
//! stable even if the internal decode type evolves. Each carries a couple of
//! computed conveniences (`hn_url`, `domain`) that scripts would otherwise
//! have to derive themselves, and omits unset optional fields rather than
//! emitting `null` so `jq` filters stay simple.

use crate::api::types::{CommentWithDepth, Item, ItemType};
use serde::Serialize;

/// Render width for comment bodies that are flattened to plain text inside
/// the JSON payload. Matches the headless text renderer's default.
const PLAIN_TEXT_WIDTH: usize = 80;

/// Canonical Hacker News permalink for an item id.
fn hn_url(id: u64) -> String {
    format!("https://news.ycombinator.com/item?id={id}")
}

/// Wire string for an item's type (`story`, `comment`, `job`, …), or `None`
/// when the source item left `type` unset. Kept as an explicit match (rather
/// than re-deriving `Serialize` on `ItemType`) so the JSON contract is
/// independent of the internal enum's serde attributes.
fn kind_str(item: &Item) -> Option<&'static str> {
    item.item_type.map(|t| match t {
        ItemType::Story => "story",
        ItemType::Comment => "comment",
        ItemType::Job => "job",
        ItemType::Poll => "poll",
        ItemType::Pollopt => "pollopt",
        ItemType::Unknown => "unknown",
    })
}

/// JSON view of a story / job / listing row.
#[derive(Debug, Serialize)]
pub struct OutStory {
    /// HN item id.
    pub id: u64,
    /// Full title as posted (badge prefix preserved, e.g. `Ask HN: …`).
    pub title: String,
    /// Submitter username, omitted when unknown/deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Net score, omitted for unscored items.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<i64>,
    /// Total comment count (HN `descendants`), omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comments: Option<i64>,
    /// Submission time in Unix seconds, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
    /// External link, omitted for HN-native text posts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Host of `url` (with `www.` stripped), omitted for text posts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// HN discussion permalink — always present.
    pub hn_url: String,
    /// Item type string, omitted when the source left it unset.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub kind: Option<&'static str>,
}

impl OutStory {
    /// Projects an [`Item`] into the public story shape, computing `domain`
    /// and `hn_url`.
    pub fn from_item(item: &Item) -> Self {
        Self {
            id: item.id,
            title: item.title.clone().unwrap_or_default(),
            by: item.by.clone(),
            score: item.score,
            comments: item.descendants,
            time: item.time,
            url: item.url.clone(),
            domain: item.domain(),
            hn_url: hn_url(item.id),
            kind: kind_str(item),
        }
    }
}

/// JSON view of a single comment in a flattened thread.
#[derive(Debug, Serialize)]
pub struct OutComment {
    /// HN item id of the comment.
    pub id: u64,
    /// Tree depth — `0` is a top-level reply to the story.
    pub depth: usize,
    /// Author username, omitted when unknown/deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by: Option<String>,
    /// Comment time in Unix seconds, omitted when absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
    /// Body rendered to terminal-safe plain text (HTML stripped).
    pub text: String,
}

impl OutComment {
    /// Projects a [`CommentWithDepth`] into the public comment shape,
    /// rendering its HTML body to plain text.
    pub fn from_comment(c: &CommentWithDepth) -> Self {
        Self {
            id: c.item.id,
            depth: c.depth,
            by: c.item.by.clone(),
            time: c.item.time,
            text: crate::cli::render::html_to_plain(c.item.text.as_deref(), PLAIN_TEXT_WIDTH),
        }
    }
}

/// JSON view of a story plus its flattened comment tree.
#[derive(Debug, Serialize)]
pub struct OutThread {
    /// The story being discussed.
    pub story: OutStory,
    /// Comments in pre-order (parents before descendants); `depth` encodes
    /// nesting.
    pub comments: Vec<OutComment>,
}

/// Builds the JSON listing payload for a slice of stories.
pub fn stories(items: &[std::sync::Arc<Item>]) -> Vec<OutStory> {
    items
        .iter()
        .map(|i| OutStory::from_item(i.as_ref()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::Item;

    fn story() -> Item {
        Item {
            id: 42,
            title: Some("Ask HN: Test?".into()),
            url: Some("https://example.com/p".into()),
            text: None,
            by: Some("alice".into()),
            score: Some(100),
            time: Some(1_700_000_000),
            kids: None,
            descendants: Some(7),
            item_type: Some(ItemType::Story),
            dead: None,
            deleted: None,
        }
    }

    #[test]
    fn out_story_computes_hn_url_and_domain() {
        let s = OutStory::from_item(&story());
        assert_eq!(s.id, 42);
        assert_eq!(s.hn_url, "https://news.ycombinator.com/item?id=42");
        assert_eq!(s.domain.as_deref(), Some("example.com"));
        assert_eq!(s.kind, Some("story"));
        assert_eq!(s.comments, Some(7));
    }

    #[test]
    fn out_story_serializes_without_null_optionals() {
        // A text post (no url) should omit `url`/`domain` entirely.
        let mut item = story();
        item.url = None;
        let json = serde_json::to_string(&OutStory::from_item(&item)).unwrap();
        assert!(!json.contains("\"url\""), "url should be omitted: {json}");
        assert!(
            !json.contains("\"domain\""),
            "domain should be omitted: {json}"
        );
        assert!(json.contains("\"hn_url\""), "hn_url must always be present");
        // Type field is renamed to `type`.
        assert!(json.contains("\"type\":\"story\""), "got {json}");
    }

    #[test]
    fn out_story_missing_title_is_empty_string() {
        let mut item = story();
        item.title = None;
        assert_eq!(OutStory::from_item(&item).title, "");
    }
}
