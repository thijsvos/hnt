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
