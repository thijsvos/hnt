use serde::Deserialize;
use std::fmt;

#[derive(Debug, Clone, Deserialize)]
pub struct Item {
    pub id: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub by: Option<String>,
    #[serde(default)]
    pub score: Option<i64>,
    #[serde(default)]
    pub time: Option<i64>,
    #[serde(default)]
    pub kids: Option<Vec<u64>>,
    #[serde(default)]
    pub descendants: Option<i64>,
    #[serde(rename = "type", default)]
    pub item_type: Option<String>,
    #[serde(default)]
    pub dead: Option<bool>,
    #[serde(default)]
    pub deleted: Option<bool>,
}

impl Item {
    pub fn is_dead_or_deleted(&self) -> bool {
        self.dead.unwrap_or(false) || self.deleted.unwrap_or(false)
    }

    pub fn domain(&self) -> Option<String> {
        self.url.as_ref().and_then(|u| url_domain(u))
    }

    pub fn badge(&self) -> Option<StoryBadge> {
        if self.item_type.as_deref() == Some("job") {
            return Some(StoryBadge::Job);
        }
        if self.item_type.as_deref() == Some("poll") {
            return Some(StoryBadge::Poll);
        }
        let title = self.title.as_deref()?;
        if title.starts_with("Ask HN:") {
            return Some(StoryBadge::Ask);
        }
        if title.starts_with("Show HN:") {
            return Some(StoryBadge::Show);
        }
        if title.starts_with("Tell HN:") {
            return Some(StoryBadge::Tell);
        }
        if title.starts_with("Launch HN:") {
            return Some(StoryBadge::Launch);
        }
        None
    }

    /// Title with badge prefix stripped (e.g. "Ask HN: Foo" → "Foo")
    pub fn display_title(&self) -> &str {
        let title = self.title.as_deref().unwrap_or("[no title]");
        if let Some(rest) = title.strip_prefix("Ask HN:") {
            return rest.trim_start();
        }
        if let Some(rest) = title.strip_prefix("Show HN:") {
            return rest.trim_start();
        }
        if let Some(rest) = title.strip_prefix("Tell HN:") {
            return rest.trim_start();
        }
        if let Some(rest) = title.strip_prefix("Launch HN:") {
            return rest.trim_start();
        }
        title
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoryBadge {
    Ask,
    Show,
    Tell,
    Launch,
    Job,
    Poll,
}

impl StoryBadge {
    pub fn label(self) -> &'static str {
        match self {
            StoryBadge::Ask => "Ask HN",
            StoryBadge::Show => "Show HN",
            StoryBadge::Tell => "Tell HN",
            StoryBadge::Launch => "Launch HN",
            StoryBadge::Job => "Job",
            StoryBadge::Poll => "Poll",
        }
    }
}

// --- Algolia Search types ---

#[derive(Debug, Deserialize)]
pub struct SearchHit {
    #[serde(rename = "objectID")]
    pub object_id: String,
    pub title: Option<String>,
    pub url: Option<String>,
    pub author: Option<String>,
    pub points: Option<i64>,
    pub num_comments: Option<i64>,
    pub created_at_i: Option<i64>,
    pub story_text: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    #[serde(rename = "nbPages")]
    pub nb_pages: usize,
    #[serde(rename = "nbHits")]
    pub nb_hits: usize,
}

impl From<SearchHit> for Item {
    fn from(hit: SearchHit) -> Self {
        Item {
            id: hit.object_id.parse::<u64>().unwrap_or(0),
            title: hit.title,
            url: hit.url,
            text: hit.story_text,
            by: hit.author,
            score: hit.points,
            time: hit.created_at_i,
            kids: None,
            descendants: hit.num_comments,
            item_type: Some("story".to_string()),
            dead: None,
            deleted: None,
        }
    }
}

fn url_domain(url: &str) -> Option<String> {
    // Simple domain extraction without pulling in the url crate
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let domain = without_scheme.split('/').next()?;
    let domain = domain.strip_prefix("www.").unwrap_or(domain);
    Some(domain.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FeedKind {
    Top,
    New,
    Best,
    Ask,
    Show,
    Jobs,
}

impl FeedKind {
    pub const ALL: [FeedKind; 6] = [
        FeedKind::Top,
        FeedKind::New,
        FeedKind::Best,
        FeedKind::Ask,
        FeedKind::Show,
        FeedKind::Jobs,
    ];

    pub fn endpoint(&self) -> &'static str {
        match self {
            FeedKind::Top => "topstories",
            FeedKind::New => "newstories",
            FeedKind::Best => "beststories",
            FeedKind::Ask => "askstories",
            FeedKind::Show => "showstories",
            FeedKind::Jobs => "jobstories",
        }
    }
}

impl fmt::Display for FeedKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeedKind::Top => write!(f, "Top"),
            FeedKind::New => write!(f, "New"),
            FeedKind::Best => write!(f, "Best"),
            FeedKind::Ask => write!(f, "Ask"),
            FeedKind::Show => write!(f, "Show"),
            FeedKind::Jobs => write!(f, "Jobs"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_item() -> Item {
        Item {
            id: 1,
            title: None,
            url: None,
            text: None,
            by: None,
            score: None,
            time: None,
            kids: None,
            descendants: None,
            item_type: None,
            dead: None,
            deleted: None,
        }
    }

    // --- url_domain ---

    #[test]
    fn url_domain_https() {
        assert_eq!(
            url_domain("https://example.com/path"),
            Some("example.com".into())
        );
    }

    #[test]
    fn url_domain_http() {
        assert_eq!(
            url_domain("http://example.com/path"),
            Some("example.com".into())
        );
    }

    #[test]
    fn url_domain_strips_www() {
        assert_eq!(
            url_domain("https://www.example.com/path"),
            Some("example.com".into())
        );
    }

    #[test]
    fn url_domain_no_scheme() {
        assert_eq!(url_domain("ftp://example.com"), None);
    }

    #[test]
    fn url_domain_empty_string() {
        assert_eq!(url_domain(""), None);
    }

    #[test]
    fn url_domain_no_trailing_path() {
        assert_eq!(
            url_domain("https://example.com"),
            Some("example.com".into())
        );
    }

    // --- is_dead_or_deleted ---

    #[test]
    fn is_dead_or_deleted_neither() {
        let item = make_item();
        assert!(!item.is_dead_or_deleted());
    }

    #[test]
    fn is_dead_or_deleted_dead() {
        let mut item = make_item();
        item.dead = Some(true);
        assert!(item.is_dead_or_deleted());
    }

    #[test]
    fn is_dead_or_deleted_deleted() {
        let mut item = make_item();
        item.deleted = Some(true);
        assert!(item.is_dead_or_deleted());
    }

    #[test]
    fn is_dead_or_deleted_both_true() {
        let mut item = make_item();
        item.dead = Some(true);
        item.deleted = Some(true);
        assert!(item.is_dead_or_deleted());
    }

    #[test]
    fn is_dead_or_deleted_both_false() {
        let mut item = make_item();
        item.dead = Some(false);
        item.deleted = Some(false);
        assert!(!item.is_dead_or_deleted());
    }

    // --- domain ---

    #[test]
    fn domain_some_url() {
        let mut item = make_item();
        item.url = Some("https://example.com/page".into());
        assert_eq!(item.domain(), Some("example.com".into()));
    }

    #[test]
    fn domain_none_url() {
        let item = make_item();
        assert_eq!(item.domain(), None);
    }

    // --- badge ---

    #[test]
    fn badge_job() {
        let mut item = make_item();
        item.item_type = Some("job".into());
        assert_eq!(item.badge(), Some(StoryBadge::Job));
    }

    #[test]
    fn badge_poll() {
        let mut item = make_item();
        item.item_type = Some("poll".into());
        assert_eq!(item.badge(), Some(StoryBadge::Poll));
    }

    #[test]
    fn badge_ask_hn() {
        let mut item = make_item();
        item.title = Some("Ask HN: What is Rust?".into());
        assert_eq!(item.badge(), Some(StoryBadge::Ask));
    }

    #[test]
    fn badge_show_hn() {
        let mut item = make_item();
        item.title = Some("Show HN: My project".into());
        assert_eq!(item.badge(), Some(StoryBadge::Show));
    }

    #[test]
    fn badge_tell_hn() {
        let mut item = make_item();
        item.title = Some("Tell HN: Something".into());
        assert_eq!(item.badge(), Some(StoryBadge::Tell));
    }

    #[test]
    fn badge_launch_hn() {
        let mut item = make_item();
        item.title = Some("Launch HN: New product".into());
        assert_eq!(item.badge(), Some(StoryBadge::Launch));
    }

    #[test]
    fn badge_no_badge() {
        let mut item = make_item();
        item.title = Some("Regular title".into());
        assert_eq!(item.badge(), None);
    }

    #[test]
    fn badge_no_title() {
        let item = make_item();
        assert_eq!(item.badge(), None);
    }

    #[test]
    fn badge_job_takes_priority_over_title() {
        let mut item = make_item();
        item.item_type = Some("job".into());
        item.title = Some("Ask HN: Something".into());
        assert_eq!(item.badge(), Some(StoryBadge::Job));
    }

    // --- display_title ---

    #[test]
    fn display_title_strips_ask_hn() {
        let mut item = make_item();
        item.title = Some("Ask HN: What is Rust?".into());
        assert_eq!(item.display_title(), "What is Rust?");
    }

    #[test]
    fn display_title_strips_show_hn() {
        let mut item = make_item();
        item.title = Some("Show HN: My project".into());
        assert_eq!(item.display_title(), "My project");
    }

    #[test]
    fn display_title_no_prefix() {
        let mut item = make_item();
        item.title = Some("Regular title".into());
        assert_eq!(item.display_title(), "Regular title");
    }

    #[test]
    fn display_title_none() {
        let item = make_item();
        assert_eq!(item.display_title(), "[no title]");
    }

    #[test]
    fn display_title_strips_tell_hn() {
        let mut item = make_item();
        item.title = Some("Tell HN: Something".into());
        assert_eq!(item.display_title(), "Something");
    }

    #[test]
    fn display_title_strips_launch_hn() {
        let mut item = make_item();
        item.title = Some("Launch HN: New product".into());
        assert_eq!(item.display_title(), "New product");
    }

    #[test]
    fn display_title_case_sensitive() {
        let mut item = make_item();
        item.title = Some("ask hn: lowercase".into());
        assert_eq!(item.display_title(), "ask hn: lowercase");
    }

    // --- StoryBadge::label ---

    #[test]
    fn badge_labels() {
        assert_eq!(StoryBadge::Ask.label(), "Ask HN");
        assert_eq!(StoryBadge::Show.label(), "Show HN");
        assert_eq!(StoryBadge::Tell.label(), "Tell HN");
        assert_eq!(StoryBadge::Launch.label(), "Launch HN");
        assert_eq!(StoryBadge::Job.label(), "Job");
        assert_eq!(StoryBadge::Poll.label(), "Poll");
    }

    // --- FeedKind ---

    #[test]
    fn feed_kind_endpoints() {
        assert_eq!(FeedKind::Top.endpoint(), "topstories");
        assert_eq!(FeedKind::New.endpoint(), "newstories");
        assert_eq!(FeedKind::Best.endpoint(), "beststories");
        assert_eq!(FeedKind::Ask.endpoint(), "askstories");
        assert_eq!(FeedKind::Show.endpoint(), "showstories");
        assert_eq!(FeedKind::Jobs.endpoint(), "jobstories");
    }

    #[test]
    fn feed_kind_display() {
        assert_eq!(format!("{}", FeedKind::Top), "Top");
        assert_eq!(format!("{}", FeedKind::New), "New");
        assert_eq!(format!("{}", FeedKind::Best), "Best");
        assert_eq!(format!("{}", FeedKind::Ask), "Ask");
        assert_eq!(format!("{}", FeedKind::Show), "Show");
        assert_eq!(format!("{}", FeedKind::Jobs), "Jobs");
    }

    // --- From<SearchHit> for Item ---

    #[test]
    fn search_hit_to_item() {
        let hit = SearchHit {
            object_id: "12345".into(),
            title: Some("Test".into()),
            url: Some("https://example.com".into()),
            author: Some("user".into()),
            points: Some(42),
            num_comments: Some(10),
            created_at_i: Some(1000),
            story_text: Some("body".into()),
        };
        let item = Item::from(hit);
        assert_eq!(item.id, 12345);
        assert_eq!(item.title.as_deref(), Some("Test"));
        assert_eq!(item.by.as_deref(), Some("user"));
        assert_eq!(item.score, Some(42));
        assert_eq!(item.descendants, Some(10));
        assert_eq!(item.text.as_deref(), Some("body"));
        assert_eq!(item.item_type.as_deref(), Some("story"));
    }

    #[test]
    fn search_hit_invalid_object_id() {
        let hit = SearchHit {
            object_id: "not_a_number".into(),
            title: None,
            url: None,
            author: None,
            points: None,
            num_comments: None,
            created_at_i: None,
            story_text: None,
        };
        let item = Item::from(hit);
        assert_eq!(item.id, 0);
    }
}
