//! Plain-text and digest renderers for headless mode.
//!
//! Output is colorless (pipe-safe) and every Hacker-News-supplied string is
//! scrubbed through [`crate::sanitize::sanitize_terminal`] first: a value
//! printed to a pipe can still reach a terminal later (via `cat`, an editor,
//! a pager), so the same escape-stripping discipline the TUI applies on
//! render applies here on write.

use crate::api::types::{CommentWithDepth, Item};
use crate::sanitize::sanitize_terminal;
use crate::state::reader_state::StyledFragment;
use std::io::{self, Write};
use std::sync::Arc;

/// Wrap width for HTML bodies flattened to plain text (comments, self-posts).
const WRAP_WIDTH: usize = 80;

/// Strips HTML to terminal-safe plain text at `width` — mirrors the TUI's
/// comment rendering (`html2text` then [`sanitize_terminal`]). `None` or an
/// empty body yields an empty string. `width` is floored at 20 because
/// `html2text` refuses very narrow widths.
pub fn html_to_plain(html: Option<&str>, width: usize) -> String {
    let Some(text) = html else {
        return String::new();
    };
    let rendered = html2text::from_read(text.as_bytes(), width.max(20)).unwrap_or_default();
    sanitize_terminal(&rendered).into_owned()
}

/// Writes a human-readable, colorless story listing — three lines per story
/// (title row, metadata row, permalink).
pub fn feed(out: &mut impl Write, items: &[Arc<Item>]) -> io::Result<()> {
    if items.is_empty() {
        writeln!(out, "No stories.")?;
        return Ok(());
    }
    for (i, item) in items.iter().enumerate() {
        let badge = item
            .badge()
            .map(|b| format!("[{}] ", b.label()))
            .unwrap_or_default();
        let title = sanitize_terminal(item.title.as_deref().unwrap_or("[no title]"));
        let domain = item
            .domain()
            .map(|d| format!("  ({})", sanitize_terminal(&d)))
            .unwrap_or_default();
        writeln!(out, "{:>3}. {badge}{title}{domain}", i + 1)?;
        writeln!(
            out,
            "     {} points · {} comments · by {} · id {}",
            item.score.unwrap_or(0),
            item.descendants.unwrap_or(0),
            sanitize_terminal(item.by.as_deref().unwrap_or("unknown")),
            item.id,
        )?;
        writeln!(out, "     https://news.ycombinator.com/item?id={}", item.id)?;
    }
    Ok(())
}

/// Writes one compact line per story — rank, title, score, comments, domain.
/// Suitable for a mailed/cron digest.
pub fn digest(out: &mut impl Write, items: &[Arc<Item>]) -> io::Result<()> {
    for (i, item) in items.iter().enumerate() {
        let title = sanitize_terminal(item.title.as_deref().unwrap_or("[no title]"));
        let domain = item
            .domain()
            .map(|d| sanitize_terminal(&d).into_owned())
            .unwrap_or_else(|| "news.ycombinator.com".to_string());
        writeln!(
            out,
            "{:>3}. {}  [{} pts · {} cmts · {}]",
            i + 1,
            title,
            item.score.unwrap_or(0),
            item.descendants.unwrap_or(0),
            domain,
        )?;
    }
    Ok(())
}

/// Writes a story header followed by its depth-indented comment tree. The
/// story self-text (Ask/Show HN body, job description) is included when
/// present; comments are indented two spaces per nesting level.
pub fn thread(out: &mut impl Write, story: &Item, comments: &[CommentWithDepth]) -> io::Result<()> {
    let title = sanitize_terminal(story.title.as_deref().unwrap_or("[no title]"));
    writeln!(out, "{title}")?;
    if let Some(url) = story.url.as_deref() {
        // Already http(s)-validated at decode; sanitize defensively anyway.
        writeln!(out, "{}", sanitize_terminal(url))?;
    }
    writeln!(
        out,
        "{} points · {} comments · by {} · id {}",
        story.score.unwrap_or(0),
        story.descendants.unwrap_or(0),
        sanitize_terminal(story.by.as_deref().unwrap_or("unknown")),
        story.id,
    )?;
    writeln!(out, "https://news.ycombinator.com/item?id={}", story.id)?;

    let body = html_to_plain(story.text.as_deref(), WRAP_WIDTH);
    if !body.trim().is_empty() {
        writeln!(out)?;
        for line in body.lines() {
            writeln!(out, "{line}")?;
        }
    }
    writeln!(out)?;

    if comments.is_empty() {
        writeln!(out, "(no comments)")?;
        return Ok(());
    }

    let now = chrono::Utc::now().timestamp();
    for c in comments {
        let indent = "  ".repeat(c.depth);
        let by = sanitize_terminal(c.item.by.as_deref().unwrap_or("unknown"));
        let age = c
            .item
            .time
            .map(|t| crate::ui::story_list::format_time_ago_since(t, now))
            .unwrap_or_default();
        writeln!(out, "{indent}— {by} ({age})")?;
        let width = WRAP_WIDTH.saturating_sub(c.depth * 2).max(20);
        for line in html_to_plain(c.item.text.as_deref(), width).lines() {
            writeln!(out, "{indent}  {line}")?;
        }
        writeln!(out)?;
    }
    Ok(())
}

/// Writes article text extracted by `crate::article`, flattening each styled
/// line to plain text. Fragment text is already terminal-safe (the extractor
/// sanitizes on construction).
pub fn article(out: &mut impl Write, lines: &[Vec<StyledFragment>]) -> io::Result<()> {
    for line in lines {
        let mut joined = String::new();
        for frag in line {
            joined.push_str(&frag.text);
        }
        writeln!(out, "{joined}")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ItemType;

    fn comment(id: u64, depth: usize, text: &str) -> CommentWithDepth {
        CommentWithDepth {
            item: Item {
                id,
                title: None,
                url: None,
                text: Some(text.into()),
                by: Some("bob".into()),
                score: None,
                time: Some(1_700_000_000),
                kids: None,
                descendants: None,
                item_type: Some(ItemType::Comment),
                dead: None,
                deleted: None,
            },
            depth,
        }
    }

    fn story_item() -> Item {
        Item {
            id: 1,
            title: Some("Show HN: Thing".into()),
            url: Some("https://example.com".into()),
            text: None,
            by: Some("alice".into()),
            score: Some(50),
            time: Some(1_700_000_000),
            kids: None,
            descendants: Some(2),
            item_type: Some(ItemType::Story),
            dead: None,
            deleted: None,
        }
    }

    #[test]
    fn html_to_plain_strips_tags() {
        let out = html_to_plain(Some("<p>hello <b>world</b></p>"), 80);
        assert!(out.contains("hello"));
        assert!(out.contains("world"));
        assert!(!out.contains('<'), "tags should be stripped: {out:?}");
    }

    #[test]
    fn html_to_plain_none_is_empty() {
        assert_eq!(html_to_plain(None, 80), "");
    }

    #[test]
    fn html_to_plain_strips_terminal_escapes() {
        // Entity-encoded ESC must not survive into the output stream.
        let out = html_to_plain(Some("a&#x1b;]0;pwned&#x07;b"), 80);
        assert!(!out.contains('\x1b'), "ESC must be stripped: {out:?}");
        assert!(!out.contains('\x07'), "BEL must be stripped: {out:?}");
    }

    #[test]
    fn feed_renders_rank_title_and_permalink() {
        let items = vec![Arc::new(story_item())];
        let mut buf = Vec::new();
        feed(&mut buf, &items).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("1. [Show HN] Show HN: Thing"));
        assert!(s.contains("50 points · 2 comments · by alice"));
        assert!(s.contains("item?id=1"));
        assert!(!s.contains('\x1b'), "output must be ANSI-free");
    }

    #[test]
    fn digest_is_one_line_per_story() {
        let items = vec![Arc::new(story_item()), Arc::new(story_item())];
        let mut buf = Vec::new();
        digest(&mut buf, &items).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert_eq!(s.lines().count(), 2);
        assert!(s.contains("[50 pts · 2 cmts · example.com]"));
    }

    #[test]
    fn thread_indents_by_depth_and_has_no_comments_marker() {
        let mut buf = Vec::new();
        thread(&mut buf, &story_item(), &[]).unwrap();
        let s = String::from_utf8(buf).unwrap();
        assert!(s.contains("Show HN: Thing"));
        assert!(s.contains("(no comments)"));

        let mut buf2 = Vec::new();
        let comments = vec![comment(2, 0, "root"), comment(3, 1, "child")];
        thread(&mut buf2, &story_item(), &comments).unwrap();
        let s2 = String::from_utf8(buf2).unwrap();
        // depth-1 comment body is indented by 2 (header) + 2 (body) spaces.
        assert!(
            s2.contains("\n    child"),
            "child should be indented: {s2:?}"
        );
    }
}
