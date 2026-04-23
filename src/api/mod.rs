//! Hacker News API layer.
//!
//! [`client::HnClient`] wraps the Firebase Hacker News API and the Algolia
//! search endpoint with an LRU item cache and bounded concurrency.
//! [`types`] defines the wire-level [`types::Item`] / [`types::FeedKind`] /
//! [`types::SearchHit`] structures and the [`types::StoryBadge`] prefix
//! classifier used by the UI.

pub mod client;
pub mod types;
