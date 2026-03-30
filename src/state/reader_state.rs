use ratatui::style::Style;

pub struct StyledFragment {
    pub text: String,
    pub style: Style,
}

pub struct ReaderState {
    pub title: String,
    pub domain: Option<String>,
    pub url: Option<String>,
    pub lines: Vec<Vec<StyledFragment>>,
    pub scroll: usize,
    pub loading: bool,
    pub error: Option<String>,
}

impl ReaderState {
    pub fn new_loading(title: String, domain: Option<String>, url: Option<String>) -> Self {
        Self {
            title,
            domain,
            url,
            lines: Vec::new(),
            scroll: 0,
            loading: true,
            error: None,
        }
    }

    pub fn set_content(&mut self, lines: Vec<Vec<StyledFragment>>) {
        self.lines = lines;
        self.loading = false;
        self.error = None;
        self.scroll = 0;
    }

    pub fn set_error(&mut self, msg: String) {
        self.error = Some(msg);
        self.loading = false;
    }

    pub fn scroll_down(&mut self, n: usize) {
        let max = self.max_scroll();
        self.scroll = (self.scroll + n).min(max);
    }

    pub fn scroll_up(&mut self, n: usize) {
        self.scroll = self.scroll.saturating_sub(n);
    }

    pub fn page_down(&mut self, n: usize) {
        self.scroll_down(n);
    }

    pub fn page_up(&mut self, n: usize) {
        self.scroll_up(n);
    }

    pub fn jump_top(&mut self) {
        self.scroll = 0;
    }

    pub fn jump_bottom(&mut self) {
        self.scroll = self.max_scroll();
    }

    pub fn scroll_percent(&self) -> u16 {
        let max = self.max_scroll();
        if max == 0 {
            100
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
        r.set_content(make_lines(10));
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
    fn scroll_down_increments() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(20));
        r.scroll_down(5);
        assert_eq!(r.scroll, 5);
    }

    #[test]
    fn scroll_down_clamps_at_max() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(10));
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
        r.set_content(make_lines(20));
        r.scroll = 10;
        r.scroll_up(3);
        assert_eq!(r.scroll, 7);
    }

    #[test]
    fn scroll_up_clamps_at_zero() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(20));
        r.scroll = 2;
        r.scroll_up(5);
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn jump_top_and_bottom() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(20));
        r.jump_bottom();
        assert_eq!(r.scroll, 19);
        r.jump_top();
        assert_eq!(r.scroll, 0);
    }

    #[test]
    fn scroll_percent_at_top() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(100));
        assert_eq!(r.scroll_percent(), 0);
    }

    #[test]
    fn scroll_percent_at_bottom() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(100));
        r.jump_bottom();
        assert_eq!(r.scroll_percent(), 100);
    }

    #[test]
    fn scroll_percent_empty_returns_100() {
        let r = ReaderState::new_loading("T".into(), None, None);
        assert_eq!(r.scroll_percent(), 100);
    }

    #[test]
    fn scroll_percent_single_line() {
        let mut r = ReaderState::new_loading("T".into(), None, None);
        r.set_content(make_lines(1));
        assert_eq!(r.scroll_percent(), 100); // max_scroll is 0
    }
}
