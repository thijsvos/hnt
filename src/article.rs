use crate::state::reader_state::StyledFragment;
use crate::ui::theme;
use html2text::render::RichAnnotation;
use ratatui::style::{Modifier, Style};

const MAX_RESPONSE_BYTES: usize = 5 * 1024 * 1024; // 5MB

/// For GitHub/GitLab repo pages, try to fetch the raw README instead of the
/// JS-heavy HTML shell (the README content is loaded dynamically by JS).
async fn try_fetch_readme(client: &reqwest::Client, url: &str) -> Option<(String, bool)> {
    let parsed = url::Url::parse(url).ok()?;
    let host = parsed.host_str()?;
    let path_segs: Vec<&str> = parsed.path().trim_matches('/').split('/').collect();

    // Only match repo root pages (no sub-paths like /issues, /blob, etc.)
    let readme_urls: Vec<String> = if host == "github.com" || host.ends_with(".github.com") {
        if path_segs.len() != 2 || path_segs[0].is_empty() || path_segs[1].is_empty() {
            return None;
        }
        let (owner, repo) = (path_segs[0], path_segs[1]);
        vec![
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/README.md",
                owner, repo
            ),
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/readme.md",
                owner, repo
            ),
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/README.rst",
                owner, repo
            ),
            format!(
                "https://raw.githubusercontent.com/{}/{}/HEAD/README",
                owner, repo
            ),
        ]
    } else if host == "gitlab.com" || host.ends_with(".gitlab.com") {
        if path_segs.len() < 2 || path_segs.iter().any(|s| s.is_empty()) {
            return None;
        }
        // GitLab can have nested groups: gitlab.com/group/subgroup/repo
        let project_path = path_segs.join("/");
        vec![
            format!("https://gitlab.com/{}/-/raw/HEAD/README.md", project_path),
            format!("https://gitlab.com/{}/-/raw/HEAD/readme.md", project_path),
            format!("https://gitlab.com/{}/-/raw/HEAD/README.rst", project_path),
            format!("https://gitlab.com/{}/-/raw/HEAD/README", project_path),
        ]
    } else {
        return None;
    };

    for readme_url in readme_urls {
        if let Ok(resp) = client.get(&readme_url).send().await {
            if resp.status().is_success() {
                if let Some(len) = resp.content_length() {
                    if len > MAX_RESPONSE_BYTES as u64 {
                        continue;
                    }
                }
                if let Ok(text) = resp.text().await {
                    if text.len() > MAX_RESPONSE_BYTES {
                        continue;
                    }
                    if !text.trim().is_empty() {
                        let is_markdown = readme_url.ends_with(".md");
                        return Some((text, is_markdown));
                    }
                }
            }
        }
    }

    None
}

/// Convert markdown text to styled lines with basic formatting.
fn markdown_to_styled_lines(text: &str, width: usize) -> Vec<Vec<StyledFragment>> {
    let mut lines: Vec<Vec<StyledFragment>> = Vec::new();

    for raw_line in text.lines() {
        // Heading detection
        if let Some(rest) = raw_line.strip_prefix("# ") {
            lines.push(vec![StyledFragment {
                text: rest.to_string(),
                style: Style::default()
                    .fg(theme::HN_ORANGE)
                    .add_modifier(Modifier::BOLD),
            }]);
            lines.push(vec![]);
        } else if let Some(rest) = raw_line.strip_prefix("## ") {
            lines.push(vec![StyledFragment {
                text: rest.to_string(),
                style: Style::default()
                    .fg(theme::YELLOW)
                    .add_modifier(Modifier::BOLD),
            }]);
            lines.push(vec![]);
        } else if let Some(rest) = raw_line.strip_prefix("### ") {
            lines.push(vec![StyledFragment {
                text: rest.to_string(),
                style: Style::default()
                    .fg(theme::GREEN)
                    .add_modifier(Modifier::BOLD),
            }]);
            lines.push(vec![]);
        } else if raw_line.starts_with("```") {
            // Code fence marker — just skip the marker line
            lines.push(vec![StyledFragment {
                text: raw_line.to_string(),
                style: Style::default().fg(theme::DIM),
            }]);
        } else if raw_line.starts_with("    ") || raw_line.starts_with('\t') {
            // Indented code
            lines.push(vec![StyledFragment {
                text: raw_line.to_string(),
                style: Style::default().fg(theme::GREEN).bg(theme::SURFACE),
            }]);
        } else if raw_line.starts_with("- ") || raw_line.starts_with("* ") {
            lines.push(vec![
                StyledFragment {
                    text: "  \u{2022} ".to_string(),
                    style: Style::default().fg(theme::HN_ORANGE),
                },
                StyledFragment {
                    text: raw_line[2..].to_string(),
                    style: Style::default().fg(theme::TEXT),
                },
            ]);
        } else if let Some(rest) = raw_line.strip_prefix("> ") {
            lines.push(vec![
                StyledFragment {
                    text: "\u{2502} ".to_string(),
                    style: Style::default().fg(theme::DIM),
                },
                StyledFragment {
                    text: rest.to_string(),
                    style: Style::default()
                        .fg(theme::SUBTEXT)
                        .add_modifier(Modifier::ITALIC),
                },
            ]);
        } else {
            // Word-wrap long lines
            if raw_line.chars().count() > width && width > 0 {
                let mut remaining = raw_line;
                while !remaining.is_empty() {
                    if remaining.chars().count() <= width {
                        lines.push(vec![StyledFragment {
                            text: remaining.to_string(),
                            style: Style::default().fg(theme::TEXT),
                        }]);
                        break;
                    }
                    let byte_pos = remaining
                        .char_indices()
                        .nth(width)
                        .map(|(i, _)| i)
                        .unwrap_or(remaining.len());
                    let split_at = remaining[..byte_pos]
                        .rfind(' ')
                        .map(|p| p + 1)
                        .unwrap_or(byte_pos);
                    lines.push(vec![StyledFragment {
                        text: remaining[..split_at].to_string(),
                        style: Style::default().fg(theme::TEXT),
                    }]);
                    remaining = &remaining[split_at..];
                }
            } else {
                lines.push(vec![StyledFragment {
                    text: raw_line.to_string(),
                    style: Style::default().fg(theme::TEXT),
                }]);
            }
        }
    }

    lines
}

/// Fetch article HTML, run readability extraction, convert to styled lines.
pub async fn fetch_and_extract_article(
    url: &str,
    width: usize,
) -> Result<Vec<Vec<StyledFragment>>, String> {
    let client = reqwest::Client::builder()
        .user_agent(concat!(
            "Mozilla/5.0 (compatible; hnt/",
            env!("CARGO_PKG_VERSION"),
            ")"
        ))
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| format!("HTTP client error: {}", e))?;

    // For GitHub/GitLab repo pages, try fetching the README directly
    if let Some((readme_text, is_markdown)) = try_fetch_readme(&client, url).await {
        return if is_markdown {
            Ok(markdown_to_styled_lines(&readme_text, width))
        } else {
            // RST / plain text — render as plain styled lines
            Ok(readme_text
                .lines()
                .map(|line| {
                    vec![StyledFragment {
                        text: line.to_string(),
                        style: Style::default().fg(theme::TEXT),
                    }]
                })
                .collect())
        };
    }

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Failed to fetch: {}", e))?;

    if !resp.status().is_success() {
        return Err(format!("HTTP {}", resp.status()));
    }

    if let Some(len) = resp.content_length() {
        if len > MAX_RESPONSE_BYTES as u64 {
            return Err("Article too large (>5MB)".to_string());
        }
    }

    // Check content-type — reject non-HTML
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_lowercase();

    if !content_type.is_empty()
        && !content_type.contains("text/html")
        && !content_type.contains("text/plain")
        && !content_type.contains("application/xhtml")
    {
        return Err(format!(
            "Not an article (content-type: {})",
            content_type.split(';').next().unwrap_or(&content_type)
        ));
    }

    let bytes = resp
        .bytes()
        .await
        .map_err(|e| format!("Failed to read response: {}", e))?;

    if bytes.len() > MAX_RESPONSE_BYTES {
        return Err("Article too large (>5MB)".to_string());
    }

    // Run readability extraction in a blocking task (CPU-bound)
    let url_string = url.to_string();
    let width_copy = width;
    tokio::task::spawn_blocking(move || extract_article_content(&bytes, &url_string, width_copy))
        .await
        .map_err(|e| format!("Processing error: {}", e))?
}

/// Run readability extraction + html2text rich rendering (blocking/CPU-bound).
fn extract_article_content(
    html_bytes: &[u8],
    url_str: &str,
    width: usize,
) -> Result<Vec<Vec<StyledFragment>>, String> {
    let parsed_url = url::Url::parse(url_str).map_err(|e| format!("Invalid URL: {}", e))?;

    // Try readability extraction first, fall back to full HTML if it produces no content
    let tagged_lines = {
        let mut cursor = std::io::Cursor::new(html_bytes);
        let readability_lines = match readability::extract(
            &mut cursor,
            &parsed_url,
            readability::ExtractOptions::default(),
        ) {
            Ok(readable) if !readable.text.trim().is_empty() => {
                html2text::from_read_rich(readable.content.as_bytes(), width).unwrap_or_default()
            }
            _ => Vec::new(),
        };

        if readability_lines
            .iter()
            .any(|l| l.tagged_strings().any(|ts| !ts.s.trim().is_empty()))
        {
            readability_lines
        } else {
            // Fallback: render the full HTML
            html2text::from_read_rich(html_bytes, width).unwrap_or_default()
        }
    };

    let lines: Vec<Vec<StyledFragment>> = tagged_lines
        .into_iter()
        .map(|tagged_line| {
            let mut fragments = Vec::new();
            for ts in tagged_line.tagged_strings() {
                let style = annotations_to_style(&ts.tag);
                fragments.push(StyledFragment {
                    text: ts.s.clone(),
                    style,
                });
                // Append URL after link text
                for ann in &ts.tag {
                    if let RichAnnotation::Link(ref url) = ann {
                        fragments.push(StyledFragment {
                            text: format!(" [{}]", url),
                            style: Style::default().fg(theme::DIM),
                        });
                    }
                }
            }
            fragments
        })
        .collect();

    Ok(lines)
}

/// Convert html2text RichAnnotation set to a ratatui Style.
fn annotations_to_style(annotations: &[RichAnnotation]) -> Style {
    let mut style = Style::default().fg(theme::TEXT);

    for ann in annotations {
        match ann {
            RichAnnotation::Strong => {
                style = style.add_modifier(Modifier::BOLD);
            }
            RichAnnotation::Emphasis => {
                style = style.add_modifier(Modifier::ITALIC);
            }
            RichAnnotation::Code | RichAnnotation::Preformat(_) => {
                style = style.fg(theme::GREEN).bg(theme::SURFACE);
            }
            RichAnnotation::Link(_) => {
                style = style.fg(theme::BLUE).add_modifier(Modifier::UNDERLINED);
            }
            RichAnnotation::Strikeout => {
                style = style.add_modifier(Modifier::CROSSED_OUT);
            }
            RichAnnotation::Image(_) => {
                style = style.fg(theme::MAUVE).add_modifier(Modifier::ITALIC);
            }
            _ => {}
        }
    }

    style
}

/// Convert raw HTML bytes to styled lines using html2text rich rendering.
pub fn html_to_styled_lines(html: &[u8], width: usize) -> Vec<Vec<StyledFragment>> {
    let tagged_lines = html2text::from_read_rich(html, width).unwrap_or_default();

    tagged_lines
        .into_iter()
        .map(|tagged_line| {
            let mut fragments = Vec::new();
            for ts in tagged_line.tagged_strings() {
                let style = annotations_to_style(&ts.tag);
                fragments.push(StyledFragment {
                    text: ts.s.clone(),
                    style,
                });
                for ann in &ts.tag {
                    if let RichAnnotation::Link(ref url) = ann {
                        fragments.push(StyledFragment {
                            text: format!(" [{}]", url),
                            style: Style::default().fg(theme::DIM),
                        });
                    }
                }
            }
            fragments
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- markdown_to_styled_lines ---

    #[test]
    fn markdown_empty_input_produces_no_lines() {
        let lines = markdown_to_styled_lines("", 80);
        assert!(lines.is_empty());
    }

    #[test]
    fn markdown_h1_is_bold_orange_followed_by_blank() {
        let lines = markdown_to_styled_lines("# Title", 80);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0][0].text, "Title");
        assert_eq!(lines[0][0].style.fg, Some(theme::HN_ORANGE));
        assert!(lines[0][0].style.add_modifier.contains(Modifier::BOLD));
        assert!(lines[1].is_empty());
    }

    #[test]
    fn markdown_h2_is_bold_yellow() {
        let lines = markdown_to_styled_lines("## Subtitle", 80);
        assert_eq!(lines[0][0].text, "Subtitle");
        assert_eq!(lines[0][0].style.fg, Some(theme::YELLOW));
        assert!(lines[0][0].style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn markdown_h3_is_bold_green() {
        let lines = markdown_to_styled_lines("### Small heading", 80);
        assert_eq!(lines[0][0].text, "Small heading");
        assert_eq!(lines[0][0].style.fg, Some(theme::GREEN));
    }

    #[test]
    fn markdown_bullet_dash_renders_bullet_glyph() {
        let lines = markdown_to_styled_lines("- first\n- second", 80);
        assert_eq!(lines.len(), 2);
        assert!(lines[0][0].text.contains('\u{2022}'));
        assert_eq!(lines[0][1].text, "first");
        assert_eq!(lines[1][1].text, "second");
    }

    #[test]
    fn markdown_bullet_asterisk_also_renders_bullet_glyph() {
        let lines = markdown_to_styled_lines("* one", 80);
        assert!(lines[0][0].text.contains('\u{2022}'));
        assert_eq!(lines[0][1].text, "one");
    }

    #[test]
    fn markdown_blockquote_is_italic_subtext_with_bar() {
        let lines = markdown_to_styled_lines("> quoted", 80);
        assert_eq!(lines[0][0].text, "\u{2502} ");
        assert_eq!(lines[0][1].text, "quoted");
        assert!(lines[0][1].style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn markdown_indented_four_spaces_is_code_style() {
        let lines = markdown_to_styled_lines("    let x = 1;", 80);
        assert_eq!(lines[0][0].text, "    let x = 1;");
        assert_eq!(lines[0][0].style.fg, Some(theme::GREEN));
        assert_eq!(lines[0][0].style.bg, Some(theme::SURFACE));
    }

    #[test]
    fn markdown_code_fence_keeps_marker_line() {
        let lines = markdown_to_styled_lines("```rust", 80);
        assert_eq!(lines[0][0].text, "```rust");
        assert_eq!(lines[0][0].style.fg, Some(theme::DIM));
    }

    #[test]
    fn markdown_long_line_is_word_wrapped() {
        // 4-char word repeated; width 10 should wrap every 2-3 words.
        let input = "word word word word word word word word";
        let lines = markdown_to_styled_lines(input, 10);
        assert!(lines.len() > 1);
        for line in &lines {
            let total: usize = line.iter().map(|f| f.text.chars().count()).sum();
            assert!(total <= 10, "wrapped line exceeds width: len={}", total);
        }
    }

    #[test]
    fn markdown_plain_short_line_is_single_fragment() {
        let lines = markdown_to_styled_lines("hello", 80);
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0][0].text, "hello");
        assert_eq!(lines[0][0].style.fg, Some(theme::TEXT));
    }

    // --- annotations_to_style ---

    #[test]
    fn annotations_empty_gives_default_text_color() {
        let style = annotations_to_style(&[]);
        assert_eq!(style.fg, Some(theme::TEXT));
        assert!(style.add_modifier.is_empty());
    }

    #[test]
    fn annotations_strong_adds_bold() {
        let style = annotations_to_style(&[RichAnnotation::Strong]);
        assert!(style.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn annotations_emphasis_adds_italic() {
        let style = annotations_to_style(&[RichAnnotation::Emphasis]);
        assert!(style.add_modifier.contains(Modifier::ITALIC));
    }

    #[test]
    fn annotations_code_is_green_on_surface() {
        let style = annotations_to_style(&[RichAnnotation::Code]);
        assert_eq!(style.fg, Some(theme::GREEN));
        assert_eq!(style.bg, Some(theme::SURFACE));
    }

    #[test]
    fn annotations_link_is_blue_underlined() {
        let style = annotations_to_style(&[RichAnnotation::Link("https://example.com".into())]);
        assert_eq!(style.fg, Some(theme::BLUE));
        assert!(style.add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn annotations_strikeout_adds_crossed_out() {
        let style = annotations_to_style(&[RichAnnotation::Strikeout]);
        assert!(style.add_modifier.contains(Modifier::CROSSED_OUT));
    }

    #[test]
    fn annotations_combined_strong_and_emphasis() {
        let style = annotations_to_style(&[RichAnnotation::Strong, RichAnnotation::Emphasis]);
        assert!(style.add_modifier.contains(Modifier::BOLD));
        assert!(style.add_modifier.contains(Modifier::ITALIC));
    }

    // --- html_to_styled_lines ---

    #[test]
    fn html_plain_paragraph_produces_fragments() {
        let html = b"<p>hello world</p>";
        let lines = html_to_styled_lines(html, 80);
        assert!(!lines.is_empty());
        let joined: String = lines[0].iter().map(|f| f.text.as_str()).collect();
        assert!(joined.contains("hello world"));
    }

    #[test]
    fn html_link_appends_url_in_dim_fragment() {
        let html = b"<a href=\"https://example.com\">click</a>";
        let lines = html_to_styled_lines(html, 80);
        let joined: String = lines
            .iter()
            .flat_map(|l| l.iter().map(|f| f.text.as_str()))
            .collect();
        assert!(joined.contains("https://example.com"));
    }

    #[test]
    fn html_empty_input_is_empty_output() {
        let lines = html_to_styled_lines(b"", 80);
        assert!(lines.is_empty());
    }
}
