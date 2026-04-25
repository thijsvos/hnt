//! Hint-mode transient state.
//!
//! While the user is selecting a hint label from the overlay, the app
//! holds a [`HintState`] that records (a) which action will fire on a
//! unique match, (b) which surface the labels live on, and (c) the
//! prefix typed so far.

/// What to do once the user narrows down to a single labeled link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintAction {
    /// Hand the URL to the system browser via `open::that`.
    Open,
    /// Open the URL in HNT's own inline article reader.
    OpenInReader,
    /// Copy the URL to the clipboard (via OSC 52).
    CopyUrl,
}

/// Which surface holds the labeled links.
///
/// The article reader carries its own [`LinkRegistry`](crate::state::link_registry::LinkRegistry)
/// alongside its content; the comment-tree registry is built on demand
/// when the user enters hint mode there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HintContext {
    Reader,
    Comments,
}

/// Active hint-mode state. Lives on `App` as `Option<HintState>` and is
/// `Some` only while the user is mid-selection.
#[derive(Debug, Clone)]
pub struct HintState {
    pub action: HintAction,
    pub context: HintContext,
    pub buffer: String,
}

impl HintState {
    pub fn new(action: HintAction, context: HintContext) -> Self {
        Self {
            action,
            context,
            buffer: String::new(),
        }
    }

    /// Appends a typed character to the prefix buffer.
    pub fn push(&mut self, c: char) {
        self.buffer.push(c);
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_starts_empty() {
        let h = HintState::new(HintAction::Open, HintContext::Reader);
        assert_eq!(h.action, HintAction::Open);
        assert_eq!(h.context, HintContext::Reader);
        assert!(h.buffer().is_empty());
    }

    #[test]
    fn push_appends() {
        let mut h = HintState::new(HintAction::CopyUrl, HintContext::Comments);
        h.push('a');
        h.push('s');
        assert_eq!(h.buffer(), "as");
    }
}
