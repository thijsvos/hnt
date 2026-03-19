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
