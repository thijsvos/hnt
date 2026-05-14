//! Article-reader overlay state.
//!
//! [`ReaderState`] tracks the pre-rendered [`StyledFragment`] lines,
//! scroll position, loading/error status, and the
//! [`LinkRegistry`] of every
//! hyperlink in the article body — populated by `crate::article` and
//! consulted by Quickjump's hint-mode dispatch. [`StyledFragment`] is the
//! shared line-fragment type used by both article extraction and HTML
//! comment rendering.

use crate::sanitize::sanitize_terminal;
use crate::state::link_registry::LinkRegistry;
use ratatui::style::Style;

/// A run of text sharing one ratatui [`Style`]. Multiple fragments compose
/// one rendered line (see `Vec<Vec<StyledFragment>>`).
pub struct StyledFragment {
    /// Visible text content of this fragment (UTF-8).
    pub text: String,
    /// Ratatui style applied uniformly across `text`.
    pub style: Style,
}

/// Article-reader overlay state: title/domain chrome, pre-rendered styled
/// lines, scroll position, and loading/error status.
pub struct ReaderState {
    /// Title for the overlay header. Truncated on render.
    pub title: String,
    /// Host of `url` (with `www.` stripped), shown after the title in
    /// parentheses. `None` for HN-native text posts.
    pub domain: Option<String>,
    /// Source URL for the article. `None` for HN-native text posts that
    /// render inline `Item::text`.
    pub url: Option<String>,
    /// Pre-rendered styled lines populated by [`Self::set_content`].
    pub lines: Vec<Vec<StyledFragment>>,
    /// Every hyperlink in `lines`, with assigned hint labels. Populated by
    /// [`Self::set_content`].
    pub links: LinkRegistry,
    /// Top-of-viewport line index. Updated by `scroll_*`/`page_*`/`jump_*`.
    pub scroll: usize,
    /// `true` between [`Self::new_loading`] and
    /// [`Self::set_content`]/[`Self::set_error`].
    pub loading: bool,
    /// `Some` after a fetch failure; rendered via the error path.
    pub error: Option<String>,
}

impl ReaderState {
    /// Starts in the loading state; the overlay renders a placeholder until
    /// [`set_content`](Self::set_content) or [`set_error`](Self::set_error)
    /// is called.
    ///
    /// `title` and `domain` originate from untrusted HN content
    /// (`Item::title`, the host of `Item::url`). Both are scrubbed through
    /// [`sanitize_terminal`] here so the overlay header cannot smuggle
    /// terminal control bytes through ratatui to the user's terminal. The
    /// `url` itself is not sanitized because it is only used for the
    /// HTTP fetch and OSC-52 copy paths, both of which apply their own
    /// safeguards (the SSRF guard for the fetch, base64 for OSC 52).
    pub fn new_loading(title: String, domain: Option<String>, url: Option<String>) -> Self {
        let title = sanitize_terminal(&title).into_owned();
        let domain = domain.map(|d| sanitize_terminal(&d).into_owned());
        Self {
            title,
            domain,
            url,
            lines: Vec::new(),
            links: LinkRegistry::new(),
            scroll: 0,
            loading: true,
            error: None,
        }
    }

    /// Installs loaded content + extracted hyperlinks, clears the loading
    /// flag and any prior error, and resets scroll to the top.
    pub fn set_content(&mut self, lines: Vec<Vec<StyledFragment>>, links: LinkRegistry) {
        self.lines = lines;
        self.links = links;
        self.loading = false;
        self.error = None;
        self.scroll = 0;
    }

    /// Transitions from loading to error state with the given message.
    ///
    /// `msg` is scrubbed through [`sanitize_terminal`] before storage —
    /// fetch errors can carry server-controlled bytes (Location-header
    /// URLs, DNS-resolved hostnames, response bodies surfaced via
    /// `anyhow` context) so we treat the string as untrusted at this
    /// boundary.
    pub fn set_error(&mut self, msg: String) {
        self.error = Some(sanitize_terminal(&msg).into_owned());
        self.loading = false;
    }

    /// Scrolls the viewport down by `n` lines, clamped to `max_scroll`.
    pub fn scroll_down(&mut self, n: usize) {
        let max = self.max_scroll();
        self.scroll = (self.scroll + n).min(max);
    }

    /// Scrolls the viewport up by `n` lines, saturating at zero.
    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    /// Pages the viewport down by `n` lines (currently identical to
    /// [`Self::scroll_down`]; kept as a separate entry point so the
    /// keybinding layer can stay symmetric with [`Self::page_up`]).
    pub fn page_down(&mut self, n: usize) {
        self.scroll_down(n);
    }

    /// Pages the viewport up by `n` lines (currently identical to
    /// [`Self::scroll_up`]).
    pub fn page_up(&mut self, n: usize) {
        self.scroll_up(n);
    }

    /// Jumps to the top of the article.
    pub fn jump_top(&mut self) {
        self.scroll = 0;
    }

    /// Jumps to the bottom of the article (last line at the top of the
    /// viewport).
    pub fn jump_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    /// Position as a percentage of `max_scroll`, clamped 0..=100.
    ///
    /// Returns 0 when the content is empty or fits on a single line —
    /// reporting 100% in that case is misleading UX (the entire article
    /// is already in view, "scrolling" is a no-op). A perfectly accurate
    /// reading would need the viewport height plumbed in to compute
    /// `lines.len().saturating_sub(viewport_height)`; the 0 fallback is
    /// the cheap version that fixes the obvious wrong reading without
    /// the wider refactor.
    pub fn scroll_percent(&self) -> u16 {
        let max = self.max_scroll();
        if max == 0 {
            0
        } else {
            ((self.scroll as f64 / max as f64) * 100.0) as u16
        }
    }

    fn max_scroll(&self) -> usize {
        self.lines.len().saturating_sub(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_lines(n: usize) -> Vec<Vec<StyledFragment>> {
        (0..n)
            .map(|i| {
                vec![StyledFragment {
                    text: format!("line {}", i),
                    style: Style::default(),
                }]
            })
            .collect()
    }

    #[test]
    fn new_loading_state() {
        let r = ReaderState::new_loading("Title".into(), Some("example.com".into()), None);
        assert_eq!(r.title, "Title");
        assert_eq!(r.domain.as_deref(), Some("example.com"));
        assert!(r.loading);
        assert!(r.lines.is_empty());
        assert_eq!(r.scroll, 0);
        assert!(r.error.is_none());
    }

    #[test]
    fn set_content_clears_loading() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.error = Some("old error".into());
        r.scroll = 5;
        r.set_content(make_lines(10), LinkRegistry::new());
        assert_eq!(r.lines.len(), 10);
        assert!(!r.loading);
        assert!(r.error.is_none());
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn set_error_clears_loading() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_error("fail".into());
        assert_eq!(r.error.as_deref(), Some("fail"));
        assert!(!r.loading);
    }

    #[test]
    fn set_error_sanitises_escape_bytes() {
        // Article fetch errors can carry server-supplied bytes (e.g. a
        // malicious `Location:` header with embedded ESC). Verify they
        // don't survive into the rendered overlay.
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_error("fail\x1b]0;owned\x07after".into());
        let stored = r.error.expect("error stored");
        assert!(!stored.contains('\x1b'));
        assert!(!stored.contains('\x07'));
        assert!(stored.contains("fail"));
        assert!(stored.contains("after"));
    }

    #[test]
    fn scroll_down_increments() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(20), LinkRegistry::new());
        r.scroll_down(5);
        assert_eq!(r.scroll, 5);
    }

    #[test]
    fn scroll_down_clamps_at_max() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(10), LinkRegistry::new());
        r.scroll_down(100);
        assert_eq!(r.scroll, 9); // max_scroll = 10 - 1
    }

    #[test]
    fn scroll_down_empty_lines() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.scroll_down(5);
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn scroll_up_decrements() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(20), LinkRegistry::new());
        r.scroll = 10;
        r.scroll_up(3);
        assert_eq!(r.scroll, 7);
    }

    #[test]
    fn scroll_up_clamps_at_zero() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(20), LinkRegistry::new());
        r.scroll = 2;
        r.scroll_up(5);
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn jump_top_and_bottom() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(20), LinkRegistry::new());
        r.jump_bottom();
        assert_eq!(r.scroll, 19);
        r.jump_top();
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn scroll_percent_at_top() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(100), LinkRegistry::new());
        assert_eq!(r.scroll_percent(), 0);
    }

    #[test]
    fn scroll_percent_at_bottom() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(100), LinkRegistry::new());
        r.jump_bottom();
        assert_eq!(r.scroll_percent(), 100);
    }

    #[test]
    fn scroll_percent_empty_returns_0() {
        // No content → no scroll position → 0%, not 100% (closes #141).
        let r = ReaderState::new_loading("T".into(), None, None);
        assert_eq!(r.scroll_percent(), 0);
    }

    #[test]
    fn scroll_percent_single_line_returns_0() {
        // A one-line article fits in any viewport, so the footer
        // showing "100%" was misleading. We now return 0 in this
        // unscrollable-content case.
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(1), LinkRegistry::new());
        assert_eq!(r.scroll_percent(), 0);
    }

    // --- new_loading sanitises untrusted title / domain inputs ---

    #[test]
    fn new_loading_sanitises_escape_in_title() {
        // OSC-0 ("set window title") plus its terminator — the most
        // damaging escape an HN submitter can land in a title.
        let r = ReaderState::new_loading("\x1b]0;OWNED\x07hello".into(), None, None);
        assert!(!r.title.contains('\x1b'));
        assert!(!r.title.contains('\x07'));
        assert!(r.title.contains("hello"));
    }

    #[test]
    fn new_loading_sanitises_csi_in_title() {
        let r = ReaderState::new_loading("clear\x1b[2Jhere".into(), None, None);
        assert!(!r.title.contains('\x1b'));
        assert!(r.title.contains("clear"));
        assert!(r.title.contains("here"));
    }

    #[test]
    fn new_loading_sanitises_domain() {
        // Domains come from `url::Url::host_str` so this is defence in
        // depth — but we still scrub since a future code path could
        // route a less-trusted value in.
        let r = ReaderState::new_loading(
            "Title".into(),
            Some("example.com\x1b]0;owned\x07".into()),
            None,
        );
        let domain = r.domain.expect("domain set");
        assert!(!domain.contains('\x1b'));
        assert!(!domain.contains('\x07'));
        assert!(domain.contains("example.com"));
    }

    #[test]
    fn new_loading_preserves_safe_title_unchanged() {
        let r = ReaderState::new_loading("Just a normal title".into(), None, None);
        assert_eq!(r.title, "Just a normal title");
    }

    #[test]
    fn new_loading_preserves_unicode_letters_in_title() {
        let r = ReaderState::new_loading("café résumé 日本語".into(), None, None);
        assert_eq!(r.title, "café résumé 日本語");
    }

    #[test]
    fn new_loading_does_not_sanitise_url() {
        // The URL field feeds the HTTP fetch (which has its own SSRF
        // guard) and OSC 52 copy (base64-encoded); it is never spliced
        // back into a `Span`. Verify it is preserved verbatim so escape
        // characters in a percent-decoded URL don't trigger surprising
        // mutation here.
        let url = "https://example.com/path?q=1".to_string();
        let r = ReaderState::new_loading("T".into(), None, Some(url.clone()));
        assert_eq!(r.url.as_deref(), Some(url.as_str()));
    }
}
