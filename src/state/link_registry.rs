//! URL hint registry for keyboard-driven link navigation.
//!
//! Collects every hyperlink in a rendered surface (article reader or
//! comment pane), assigns each one a short alphabetic label, and answers
//! prefix lookups for the [`crate::keys::InputMode::HintMode`] dispatch.
//! Used by `Quickjump` to let the user open, copy, or reader-load a link
//! by typing its overlay label.

/// Alphabet used for hint labels. Home-row + top-row characters that don't
/// collide with vim navigation keys (`j`/`k`/`h`/`g`) — chosen so that a
/// label glyph never resembles a navigation press.
const ALPHABET: &[u8] = b"asdfweiou";

/// One labeled hyperlink within a rendered surface.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkRef {
    pub url: String,
    /// Line index within the rendered `Vec<Vec<StyledFragment>>`.
    pub line: usize,
    /// Fragment index within that line; the label paints at this fragment's
    /// first column, so the column offset = sum of preceding fragment widths.
    pub fragment: usize,
    /// Assigned by [`LinkRegistry::assign_labels`]. Empty until then.
    pub label: String,
}

/// Result of a [`LinkRegistry::match_prefix`] lookup.
#[derive(Debug, PartialEq)]
pub enum MatchResult<'a> {
    /// No links match the prefix — caller should exit hint mode.
    None,
    /// More than one link starts with the prefix — keep narrowing.
    Multiple,
    /// Exactly one link matches — caller should fire the action.
    Unique(&'a LinkRef),
}

/// Collected hyperlinks for one rendered surface. Built once when content
/// arrives (article reader) or once per `f`-press (comments) and consulted
/// by the input-mode dispatch on every keypress.
#[derive(Debug, Default)]
pub struct LinkRegistry {
    pub links: Vec<LinkRef>,
}

impl LinkRegistry {
    /// Empty registry. Use [`Self::push`] to add links, then
    /// [`Self::assign_labels`] to populate the `label` field.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.links.is_empty()
    }

    /// Records a new link at `(line, fragment)`. The `label` is left empty
    /// until [`Self::assign_labels`] is called once all links are pushed.
    pub fn push(&mut self, url: String, line: usize, fragment: usize) {
        self.links.push(LinkRef {
            url,
            line,
            fragment,
            label: String::new(),
        });
    }

    /// Assigns uniform-length alphabetic labels to every link in insertion
    /// order. Length is the smallest k such that `ALPHABET.len()^k >= n`,
    /// so 1-9 links get 1-char labels, 10-81 get 2-char labels, etc.
    pub fn assign_labels(&mut self) {
        let n = self.links.len();
        if n == 0 {
            return;
        }
        let alphabet_len = ALPHABET.len();
        let mut k: usize = 1;
        let mut capacity = alphabet_len;
        while capacity < n {
            k += 1;
            capacity *= alphabet_len;
        }
        for (i, link) in self.links.iter_mut().enumerate() {
            link.label = generate_label(i, k);
        }
    }

    /// Looks up `prefix` against every link's label. See [`MatchResult`].
    pub fn match_prefix(&self, prefix: &str) -> MatchResult<'_> {
        let mut iter = self.links.iter().filter(|l| l.label.starts_with(prefix));
        match (iter.next(), iter.next()) {
            (None, _) => MatchResult::None,
            (Some(link), None) => MatchResult::Unique(link),
            (Some(_), Some(_)) => MatchResult::Multiple,
        }
    }
}

/// Encodes `index` in base `ALPHABET.len()` as a fixed-width string of
/// length `length`, big-endian. Index 0 → "aa…a", index 1 → "aa…s", etc.
fn generate_label(index: usize, length: usize) -> String {
    let alphabet_len = ALPHABET.len();
    let mut chars = Vec::with_capacity(length);
    let mut n = index;
    for _ in 0..length {
        chars.push(ALPHABET[n % alphabet_len]);
        n /= alphabet_len;
    }
    chars.reverse();
    String::from_utf8(chars).expect("ALPHABET is valid ASCII")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry_with(n: usize) -> LinkRegistry {
        let mut r = LinkRegistry::new();
        for i in 0..n {
            r.push(format!("https://example.com/{}", i), i, 0);
        }
        r.assign_labels();
        r
    }

    #[test]
    fn new_registry_is_empty() {
        let r = LinkRegistry::new();
        assert!(r.is_empty());
        assert_eq!(r.links.len(), 0);
    }

    #[test]
    fn push_adds_link_without_label() {
        let mut r = LinkRegistry::new();
        r.push("https://x.com".into(), 5, 2);
        assert_eq!(r.links.len(), 1);
        assert_eq!(r.links[0].url, "https://x.com");
        assert_eq!(r.links[0].line, 5);
        assert_eq!(r.links[0].fragment, 2);
        assert!(r.links[0].label.is_empty());
    }

    #[test]
    fn assign_labels_empty_is_noop() {
        let mut r = LinkRegistry::new();
        r.assign_labels();
        assert!(r.is_empty());
    }

    #[test]
    fn assign_labels_single_uses_one_char() {
        let r = registry_with(1);
        assert_eq!(r.links[0].label.len(), 1);
        assert_eq!(r.links[0].label, "a");
    }

    #[test]
    fn assign_labels_under_alphabet_size_uses_one_char() {
        let r = registry_with(ALPHABET.len());
        for link in &r.links {
            assert_eq!(link.label.len(), 1, "label {} should be 1 char", link.label);
        }
    }

    #[test]
    fn assign_labels_over_alphabet_size_uses_two_chars() {
        let r = registry_with(ALPHABET.len() + 1);
        for link in &r.links {
            assert_eq!(
                link.label.len(),
                2,
                "label {} should be 2 chars",
                link.label
            );
        }
    }

    #[test]
    fn assign_labels_all_unique() {
        let r = registry_with(50);
        let mut seen = std::collections::HashSet::new();
        for link in &r.links {
            assert!(
                seen.insert(link.label.clone()),
                "duplicate label: {}",
                link.label
            );
        }
    }

    #[test]
    fn assign_labels_uses_only_alphabet_chars() {
        let r = registry_with(50);
        for link in &r.links {
            for c in link.label.chars() {
                assert!(
                    ALPHABET.contains(&(c as u8)),
                    "label char {:?} not in alphabet",
                    c
                );
            }
        }
    }

    #[test]
    fn assign_labels_deterministic_in_insertion_order() {
        let r1 = registry_with(15);
        let r2 = registry_with(15);
        for (a, b) in r1.links.iter().zip(r2.links.iter()) {
            assert_eq!(a.label, b.label);
        }
    }

    #[test]
    fn match_prefix_empty_registry_is_none() {
        let r = LinkRegistry::new();
        assert_eq!(r.match_prefix(""), MatchResult::None);
        assert_eq!(r.match_prefix("a"), MatchResult::None);
    }

    #[test]
    fn match_prefix_unique_with_full_label() {
        let r = registry_with(3);
        let label = r.links[1].label.clone();
        match r.match_prefix(&label) {
            MatchResult::Unique(link) => {
                assert_eq!(link.url, "https://example.com/1");
            }
            other => panic!("expected unique match, got {:?}", other),
        }
    }

    #[test]
    fn match_prefix_empty_with_multiple_links_is_multiple() {
        let r = registry_with(5);
        assert_eq!(r.match_prefix(""), MatchResult::Multiple);
    }

    #[test]
    fn match_prefix_empty_with_one_link_is_unique() {
        let r = registry_with(1);
        match r.match_prefix("") {
            MatchResult::Unique(_) => {}
            other => panic!("expected unique, got {:?}", other),
        }
    }

    #[test]
    fn match_prefix_partial_two_char_labels() {
        // Force 2-char labels by having more links than ALPHABET.len().
        let r = registry_with(ALPHABET.len() + 5);
        // Every label starts with 'a' for the first ALPHABET.len() entries
        // (because index 0..ALPHABET.len() encode as "a?" in base-N).
        match r.match_prefix("a") {
            MatchResult::Multiple => {}
            other => panic!("expected multiple, got {:?}", other),
        }
    }

    #[test]
    fn match_prefix_no_match_returns_none() {
        let r = registry_with(5);
        assert_eq!(r.match_prefix("z"), MatchResult::None);
    }

    #[test]
    fn generate_label_index_zero_is_all_a() {
        assert_eq!(generate_label(0, 1), "a");
        assert_eq!(generate_label(0, 2), "aa");
        assert_eq!(generate_label(0, 3), "aaa");
    }

    #[test]
    fn generate_label_index_one_is_second_alphabet_char() {
        // ALPHABET[1] is 's'.
        assert_eq!(generate_label(1, 1), "s");
        assert_eq!(generate_label(1, 2), "as");
    }

    #[test]
    fn generate_label_alphabet_size_is_first_two_char_carry() {
        let n = ALPHABET.len();
        // Index n in length-2 = "sa" (first carry).
        assert_eq!(generate_label(n, 2), "sa");
    }
}
