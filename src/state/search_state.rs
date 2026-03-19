pub struct SearchState {
    pub query: String,
    pub input: String,
    pub current_page: usize,
    pub total_pages: usize,
    pub total_hits: usize,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            input: String::new(),
            current_page: 0,
            total_pages: 0,
            total_hits: 0,
        }
    }
}
