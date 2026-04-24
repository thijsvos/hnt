//! HTTP client for the Hacker News Firebase API and Algolia search.
//!
//! [`HnClient`] is cheaply [`Clone`]able (shared `reqwest::Client` plus
//! `Arc<Mutex<LruCache>>`) so async tasks can each take their own handle.
//! Item fetches are cached up to `CACHE_CAPACITY` entries and fan out
//! with `CONCURRENT_REQUESTS` in flight.

use super::types::{FeedKind, Item, SearchResponse};
use anyhow::Result;
use futures::stream::{self, StreamExt};
use lru::LruCache;
use std::collections::HashMap;
use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

const BASE_URL: &str = "https://hacker-news.firebaseio.com/v0";
const ALGOLIA_URL: &str = "https://hn.algolia.com/api/v1/search";
const CONCURRENT_REQUESTS: usize = 20;
const CACHE_CAPACITY: usize = 2000;

/// Shared HTTP client for the Hacker News API.
///
/// Cheaply [`Clone`]able — both fields are `Arc`-backed — so each spawned
/// task can take its own handle. Item fetches hit the internal LRU cache
/// before the network.
#[derive(Clone)]
pub struct HnClient {
    client: reqwest::Client,
    cache: Arc<Mutex<LruCache<u64, Item>>>,
}

impl HnClient {
    /// Builds a fresh client with an empty LRU cache of up to
    /// `CACHE_CAPACITY` items.
    pub fn new() -> Self {
        let capacity = NonZeroUsize::new(CACHE_CAPACITY).expect("cache capacity > 0");
        Self {
            client: reqwest::Client::new(),
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
        }
    }

    /// Drops every cached item. Called on feed switch and refresh to avoid
    /// serving stale data.
    pub fn clear_cache(&self) {
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Fetches the list of story IDs for a given feed.
    pub async fn fetch_story_ids(&self, feed: FeedKind) -> Result<Vec<u64>> {
        let url = format!("{}/{}.json", BASE_URL, feed.endpoint());
        let ids: Vec<u64> = self.client.get(&url).send().await?.json().await?;
        Ok(ids)
    }

    /// Fetches a single item by ID, consulting the LRU cache first.
    pub async fn fetch_item(&self, id: u64) -> Result<Option<Item>> {
        // Check cache first
        {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(item) = cache.get(&id) {
                return Ok(Some(item.clone()));
            }
        }

        let url = format!("{}/item/{}.json", BASE_URL, id);
        let resp = self.client.get(&url).send().await?;
        let item: Option<Item> = resp.json().await?;

        if let Some(ref item) = item {
            let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            cache.put(id, item.clone());
        }

        Ok(item)
    }

    /// Fetches multiple items concurrently (up to `CONCURRENT_REQUESTS`
    /// in flight) and returns them in the order of `ids` — `result[i]`
    /// corresponds to `ids[i]`, and is `None` when the item was missing
    /// or its fetch failed.
    pub async fn fetch_items(&self, ids: &[u64]) -> Vec<Option<Item>> {
        let results: Vec<Option<Item>> = stream::iter(ids.iter().copied())
            .map(|id| {
                let client = self.clone();
                async move { client.fetch_item(id).await.ok().flatten() }
            })
            .buffer_unordered(CONCURRENT_REQUESTS)
            .collect()
            .await;

        // buffer_unordered doesn't preserve order, so re-order by input IDs
        let result_map: HashMap<u64, Item> = results
            .into_iter()
            .flatten()
            .map(|item| (item.id, item))
            .collect();

        ids.iter().map(|id| result_map.get(id).cloned()).collect()
    }

    /// Fetches a page of items from a pre-fetched ID list. Used for pagination
    /// so callers can reuse the initial ID list instead of re-fetching it —
    /// avoiding drift when new stories have been posted since the last fetch.
    pub async fn fetch_items_page(
        &self,
        ids: &[u64],
        offset: usize,
        count: usize,
    ) -> Result<Vec<Item>> {
        if offset >= ids.len() {
            return Ok(Vec::new());
        }
        let end = (offset + count).min(ids.len());
        let page_ids = &ids[offset..end];

        Ok(self
            .fetch_items(page_ids)
            .await
            .into_iter()
            .flatten()
            .collect())
    }

    /// Fetches a page of a feed. Returns `(items, all_ids)` where `all_ids`
    /// is the complete ID list from the feed endpoint — callers should stash
    /// it for stable subsequent pagination.
    pub async fn fetch_stories(
        &self,
        feed: FeedKind,
        offset: usize,
        count: usize,
    ) -> Result<(Vec<Item>, Vec<u64>)> {
        let all_ids = self.fetch_story_ids(feed).await?;
        let items = self.fetch_items_page(&all_ids, offset, count).await?;
        Ok((items, all_ids))
    }

    /// Finds prior HN submissions of the same URL.
    ///
    /// Queries Algolia with a scheme-stripped form of the URL (which gets
    /// tokenized against the indexed `url` field), then filters client-side
    /// to exact URL matches after normalization (lowercased host, stripped
    /// `www.` and trailing slash, scheme-insensitive). Returns up to 50
    /// matches, most-recent first by Algolia default.
    ///
    /// Empty result is not an error — a novel URL legitimately has no prior
    /// submissions.
    pub async fn search_by_url(&self, url: &str) -> Result<Vec<Item>> {
        let target = normalize_url(url);
        if target.is_empty() {
            return Ok(Vec::new());
        }
        let encoded = url_encode(&target);
        let api = format!(
            "{}?query={}&tags=story&hitsPerPage=50",
            ALGOLIA_URL, encoded
        );
        let resp: SearchResponse = self.client.get(&api).send().await?.json().await?;
        Ok(resp
            .hits
            .into_iter()
            .map(Item::from)
            .filter(|item| {
                item.url
                    .as_deref()
                    .map(normalize_url)
                    .is_some_and(|n| n == target)
            })
            .collect())
    }

    /// Searches stories via the HN Algolia API.
    ///
    /// Returns `(stories, total_pages, total_hits)`. `page` is 0-indexed.
    /// Story [`Item::kids`] is always `None` in results — fetch the full
    /// item if comment IDs are needed.
    pub async fn search_stories(
        &self,
        query: &str,
        page: usize,
        hits_per_page: usize,
    ) -> Result<(Vec<Item>, usize, usize)> {
        let encoded_query = url_encode(query);
        let url = format!(
            "{}?query={}&tags=story&hitsPerPage={}&page={}",
            ALGOLIA_URL, encoded_query, hits_per_page, page
        );
        let resp: SearchResponse = self.client.get(&url).send().await?.json().await?;
        let stories = resp.hits.into_iter().map(Item::from).collect();
        Ok((stories, resp.nb_pages, resp.nb_hits))
    }

    /// Walks a comment subtree depth-first, appending `(Item, depth)` into
    /// `result` for every live descendant up to `max_depth`. Dead/deleted
    /// comments are skipped. Returns a boxed future so the recursion can
    /// cross `async` boundaries.
    pub fn fetch_children_recursive<'a>(
        &'a self,
        ids: &'a [u64],
        depth: usize,
        max_depth: usize,
        result: &'a mut Vec<(Item, usize)>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
        Box::pin(async move {
            if depth > max_depth || ids.is_empty() {
                return;
            }

            let items = self.fetch_items(ids).await;

            for item in items.into_iter().flatten() {
                if item.is_dead_or_deleted() {
                    continue;
                }
                let kids = item.kids.clone().unwrap_or_default();
                result.push((item, depth));

                if !kids.is_empty() {
                    self.fetch_children_recursive(&kids, depth + 1, max_depth, result)
                        .await;
                }
            }
        })
    }
}

/// Normalizes a URL for cross-submission comparison.
///
/// Lowercases the host, strips a leading `www.`, drops the scheme, strips
/// a trailing slash, and drops the fragment. Preserves the query string
/// because different query strings usually represent different resources.
/// Returns the original input unchanged if it doesn't parse as a URL.
fn normalize_url(u: &str) -> String {
    let Ok(parsed) = url::Url::parse(u) else {
        return u.to_string();
    };
    let Some(host) = parsed.host_str() else {
        return u.to_string();
    };
    let host_lower = host.to_lowercase();
    let host = host_lower.strip_prefix("www.").unwrap_or(&host_lower);
    let path = parsed.path().trim_end_matches('/');
    let query = parsed
        .query()
        .map(|q| format!("?{}", q))
        .unwrap_or_default();
    format!("{}{}{}", host, path, query)
}

/// Percent-encodes a query-string value. Preserves unreserved characters
/// (`A-Z a-z 0-9 -_.~`), encodes space as `+`, and percent-encodes every
/// other byte.
fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            b' ' => out.push('+'),
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0x0F) as usize]));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- normalize_url ---

    #[test]
    fn normalize_url_strips_scheme() {
        assert_eq!(
            normalize_url("https://example.com/path"),
            "example.com/path"
        );
        assert_eq!(normalize_url("http://example.com/path"), "example.com/path");
    }

    #[test]
    fn normalize_url_strips_www() {
        assert_eq!(
            normalize_url("https://www.example.com/path"),
            "example.com/path"
        );
    }

    #[test]
    fn normalize_url_strips_trailing_slash() {
        assert_eq!(normalize_url("https://example.com/foo/"), "example.com/foo");
        assert_eq!(normalize_url("https://example.com/"), "example.com");
    }

    #[test]
    fn normalize_url_lowercases_host_preserves_path_case() {
        assert_eq!(
            normalize_url("https://EXAMPLE.com/Rust-1.0.html"),
            "example.com/Rust-1.0.html"
        );
    }

    #[test]
    fn normalize_url_preserves_query() {
        assert_eq!(
            normalize_url("https://example.com/search?q=foo"),
            "example.com/search?q=foo"
        );
    }

    #[test]
    fn normalize_url_drops_fragment() {
        assert_eq!(
            normalize_url("https://example.com/foo#section"),
            "example.com/foo"
        );
    }

    #[test]
    fn normalize_url_invalid_returns_input() {
        assert_eq!(normalize_url("not a url"), "not a url");
    }

    #[test]
    fn normalize_url_http_and_https_collapse() {
        assert_eq!(
            normalize_url("http://example.com/foo"),
            normalize_url("https://www.example.com/foo/")
        );
    }

    #[test]
    fn normalize_url_empty_host_returns_input() {
        // file:// URLs have no host — treat as unnormalizable to avoid
        // false-positive cross-submission matches.
        assert_eq!(normalize_url("file:///tmp/foo"), "file:///tmp/foo");
    }

    // --- url_encode ---

    #[test]
    fn url_encode_space() {
        assert_eq!(url_encode("hello world"), "hello+world");
    }

    #[test]
    fn url_encode_unreserved() {
        assert_eq!(url_encode("rust-lang_0.9~"), "rust-lang_0.9~");
    }

    #[test]
    fn url_encode_special_chars() {
        assert_eq!(url_encode("foo@bar.com"), "foo%40bar.com");
    }

    #[test]
    fn url_encode_empty() {
        assert_eq!(url_encode(""), "");
    }

    #[test]
    fn url_encode_alphanumeric() {
        assert_eq!(url_encode("ABCxyz012"), "ABCxyz012");
    }

    #[test]
    fn url_encode_percent_sign() {
        assert_eq!(url_encode("100%"), "100%25");
    }

    #[test]
    fn url_encode_ampersand_and_equals() {
        assert_eq!(url_encode("q=a&b=c"), "q%3Da%26b%3Dc");
    }

    #[test]
    fn url_encode_multiple_spaces() {
        assert_eq!(url_encode("a b c"), "a+b+c");
    }

    #[test]
    fn url_encode_slash() {
        assert_eq!(url_encode("path/to"), "path%2Fto");
    }
}
