use super::types::{FeedKind, Item, SearchResponse};
use anyhow::Result;
use futures::stream::{self, StreamExt};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const BASE_URL: &str = "https://hacker-news.firebaseio.com/v0";
const ALGOLIA_URL: &str = "https://hn.algolia.com/api/v1/search";
const CONCURRENT_REQUESTS: usize = 20;

#[derive(Clone)]
pub struct HnClient {
    client: reqwest::Client,
    cache: Arc<Mutex<HashMap<u64, Item>>>,
}

impl HnClient {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
            cache: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn clear_cache(&self) {
        self.cache.lock().unwrap().clear();
    }

    /// Fetch the list of story IDs for a given feed.
    pub async fn fetch_story_ids(&self, feed: FeedKind) -> Result<Vec<u64>> {
        let url = format!("{}/{}.json", BASE_URL, feed.endpoint());
        let ids: Vec<u64> = self.client.get(&url).send().await?.json().await?;
        Ok(ids)
    }

    /// Fetch a single item by ID (uses cache).
    pub async fn fetch_item(&self, id: u64) -> Result<Option<Item>> {
        // Check cache first
        {
            let cache = self.cache.lock().unwrap();
            if let Some(item) = cache.get(&id) {
                return Ok(Some(item.clone()));
            }
        }

        let url = format!("{}/item/{}.json", BASE_URL, id);
        let resp = self.client.get(&url).send().await?;
        let item: Option<Item> = resp.json().await?;

        if let Some(ref item) = item {
            let mut cache = self.cache.lock().unwrap();
            cache.insert(id, item.clone());
        }

        Ok(item)
    }

    /// Fetch multiple items concurrently (up to CONCURRENT_REQUESTS at a time).
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

    /// Fetch stories for a feed: get IDs, then batch fetch first `count` items.
    pub async fn fetch_stories(
        &self,
        feed: FeedKind,
        offset: usize,
        count: usize,
    ) -> Result<(Vec<Item>, Vec<u64>)> {
        let all_ids = self.fetch_story_ids(feed).await?;
        let end = (offset + count).min(all_ids.len());
        let page_ids = &all_ids[offset..end];

        let items: Vec<Item> = self
            .fetch_items(page_ids)
            .await
            .into_iter()
            .flatten()
            .collect();

        Ok((items, all_ids))
    }

    /// Search stories via the HN Algolia API.
    pub async fn search_stories(
        &self,
        query: &str,
        page: usize,
        hits_per_page: usize,
    ) -> Result<(Vec<Item>, usize, usize)> {
        let resp: SearchResponse = self
            .client
            .get(ALGOLIA_URL)
            .query(&[
                ("query", query),
                ("tags", "story"),
                ("hitsPerPage", &hits_per_page.to_string()),
                ("page", &page.to_string()),
            ])
            .send()
            .await?
            .json()
            .await?;
        let stories = resp.hits.into_iter().map(Item::from).collect();
        Ok((stories, resp.nb_pages, resp.nb_hits))
    }

    /// Recursively fetch children of a comment, depth-first.
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
