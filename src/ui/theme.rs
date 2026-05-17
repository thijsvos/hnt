//! Catppuccin Mocha-inspired color palette and ratatui [`Style`] helpers.
//!
//! Exposes named [`Color`] constants for base, accent, and semantic
//! (status/metadata/badge) uses, plus `*_style()` constructors that pair
//! foreground + background for a widget context. [`depth_color`] cycles
//! through [`DEPTH_COLORS`] for comment indentation.

use ratatui::style::{Color, Modifier, Style};

/// Body background (Mocha "base").
pub const BG: Color = Color::Rgb(30, 30, 46);
/// Selected-row, accent-chip, and status-bar background (Mocha
/// "surface0").
pub const SURFACE: Color = Color::Rgb(49, 50, 68);
/// Reserved for future nested-overlay backgrounds; unused today but
/// kept for parity with the upstream Mocha palette.
#[allow(dead_code)]
pub const OVERLAY: Color = Color::Rgb(69, 71, 90);
/// Primary foreground — body text (Mocha "text").
pub const TEXT: Color = Color::Rgb(205, 214, 244);
/// Secondary foreground — metadata, blockquote text (Mocha "subtext0").
pub const SUBTEXT: Color = Color::Rgb(166, 173, 200);
/// Dimmed foreground — visited-story titles, hint text (Mocha
/// "overlay2").
pub const DIM: Color = Color::Rgb(127, 132, 156);
/// HN brand orange — accents, active tab, pin glyph, depth-0 comment
/// thread bar.
pub const HN_ORANGE: Color = Color::Rgb(255, 102, 0);
/// "Show HN" badge and code-block foreground (Mocha "green").
pub const GREEN: Color = Color::Rgb(166, 227, 161);
/// Error text in the status bar (Mocha "red").
pub const RED: Color = Color::Rgb(243, 139, 168);
/// Hyperlink color and "Ask HN" badge (Mocha "blue").
pub const BLUE: Color = Color::Rgb(137, 180, 250);
/// "Job" badge and `h2` markdown headings (Mocha "yellow").
pub const YELLOW: Color = Color::Rgb(249, 226, 175);
/// "Tell HN" badge and inline-image marker (Mocha "mauve").
pub const MAUVE: Color = Color::Rgb(203, 166, 247);
/// "Poll" badge and one of the comment-depth thread bars (Mocha
/// "teal").
pub const TEAL: Color = Color::Rgb(148, 226, 213);
/// "Launch HN" badge and one of the comment-depth thread bars (Mocha
/// "peach").
pub const PEACH: Color = Color::Rgb(250, 179, 135);

/// Colors for comment depth levels (cycles through these).
pub const DEPTH_COLORS: [Color; 6] = [HN_ORANGE, BLUE, GREEN, MAUVE, TEAL, PEACH];

// Each style helper is `const fn` so the compiler can fold it into a
// compile-time constant at every call site — the per-frame render loop
// calls these helpers dozens of times.

/// Default foreground on the base background — body text.
pub const fn base_style() -> Style {
    Style::new().fg(TEXT).bg(BG)
}

/// Highlighted row: base fg on surface bg, bold.
pub const fn selected_style() -> Style {
    Style::new()
        .fg(TEXT)
        .bg(SURFACE)
        .add_modifier(Modifier::BOLD)
}

/// HN-orange bold for widget titles.
pub const fn title_style() -> Style {
    Style::new().fg(HN_ORANGE).add_modifier(Modifier::BOLD)
}

/// Default header-bar style: body fg on surface bg.
pub const fn header_style() -> Style {
    Style::new().fg(TEXT).bg(SURFACE)
}

/// Default status-bar style: muted fg on surface bg.
pub const fn status_style() -> Style {
    Style::new().fg(SUBTEXT).bg(SURFACE)
}

/// HN-orange foreground — brand accents.
pub const fn accent_style() -> Style {
    Style::new().fg(HN_ORANGE)
}

/// Dimmed foreground — hints and secondary text.
pub const fn dim_style() -> Style {
    Style::new().fg(DIM)
}

/// Secondary foreground — author/score metadata.
pub const fn meta_style() -> Style {
    Style::new().fg(SUBTEXT)
}

/// Selected feed-tab: bg swapped with HN-orange, bold.
pub const fn active_tab_style() -> Style {
    Style::new()
        .fg(BG)
        .bg(HN_ORANGE)
        .add_modifier(Modifier::BOLD)
}

/// Unselected feed-tab: muted fg on surface bg.
pub const fn inactive_tab_style() -> Style {
    Style::new().fg(SUBTEXT).bg(SURFACE)
}

/// Wraps `depth` modulo 6 into [`DEPTH_COLORS`]; used to color comment
/// indentation bars so nested replies are visually distinct.
pub const fn depth_color(depth: usize) -> Color {
    DEPTH_COLORS[depth % DEPTH_COLORS.len()]
}

/// Quickjump hint label — bold black-on-orange so it pops against the
/// underlying styled link text. Used for labels still matching the
/// active prefix.
pub const fn hint_active_style() -> Style {
    Style::new()
        .fg(BG)
        .bg(HN_ORANGE)
        .add_modifier(Modifier::BOLD)
}

/// Quickjump hint label — dimmed grey-on-surface for labels that no
/// longer match the active prefix. Lets the user see what they've
/// already ruled out without removing the labels entirely.
pub const fn hint_dim_style() -> Style {
    Style::new().fg(DIM).bg(SURFACE)
}

/// Bold HN-orange "pin" glyph for stories present in
/// [`crate::state::pin_store::PinStore`]. Matches the brand-accent
/// palette used for badges and active tabs so the marker reads as
/// "important to you" rather than as another classification badge.
pub const fn pinned_style() -> Style {
    Style::new()
        .fg(HN_ORANGE)
        .bg(BG)
        .add_modifier(Modifier::BOLD)
}

/// Bold badge color on the surface background — one color per
/// [`StoryBadge`](crate::api::types::StoryBadge) variant.
pub const fn badge_style(badge: crate::api::types::StoryBadge) -> Style {
    use crate::api::types::StoryBadge;
    let color = match badge {
        StoryBadge::Ask => BLUE,
        StoryBadge::Show => GREEN,
        StoryBadge::Tell => MAUVE,
        StoryBadge::Launch => PEACH,
        StoryBadge::Job => YELLOW,
        StoryBadge::Poll => TEAL,
    };
    Style::new()
        .fg(color)
        .bg(SURFACE)
        .add_modifier(Modifier::BOLD)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::StoryBadge;
    use std::collections::HashSet;

    // --- depth_color ---

    #[test]
    fn depth_color_first_six_match_palette_in_order() {
        assert_eq!(depth_color(0), HN_ORANGE);
        assert_eq!(depth_color(1), BLUE);
        assert_eq!(depth_color(2), GREEN);
        assert_eq!(depth_color(3), MAUVE);
        assert_eq!(depth_color(4), TEAL);
        assert_eq!(depth_color(5), PEACH);
    }

    #[test]
    fn depth_color_wraps_modulo_six() {
        assert_eq!(depth_color(6), depth_color(0));
        assert_eq!(depth_color(13), depth_color(1));
        assert_eq!(depth_color(60), depth_color(0));
    }

    #[test]
    fn depth_color_usize_max_does_not_panic() {
        let c = depth_color(usize::MAX);
        assert!(
            DEPTH_COLORS.contains(&c),
            "depth_color(usize::MAX) must return one of DEPTH_COLORS, got {c:?}"
        );
    }

    // --- badge_style ---

    const ALL_BADGES: [StoryBadge; 6] = [
        StoryBadge::Ask,
        StoryBadge::Show,
        StoryBadge::Tell,
        StoryBadge::Launch,
        StoryBadge::Job,
        StoryBadge::Poll,
    ];

    #[test]
    fn badge_style_maps_each_variant_to_its_color() {
        assert_eq!(badge_style(StoryBadge::Ask).fg, Some(BLUE));
        assert_eq!(badge_style(StoryBadge::Show).fg, Some(GREEN));
        assert_eq!(badge_style(StoryBadge::Tell).fg, Some(MAUVE));
        assert_eq!(badge_style(StoryBadge::Launch).fg, Some(PEACH));
        assert_eq!(badge_style(StoryBadge::Job).fg, Some(YELLOW));
        assert_eq!(badge_style(StoryBadge::Poll).fg, Some(TEAL));
    }

    #[test]
    fn badge_style_always_bold_on_surface() {
        for badge in ALL_BADGES {
            let s = badge_style(badge);
            assert_eq!(
                s.bg,
                Some(SURFACE),
                "badge_style({badge:?}).bg must be SURFACE"
            );
            assert!(
                s.add_modifier.contains(Modifier::BOLD),
                "badge_style({badge:?}) must set BOLD, got {:?}",
                s.add_modifier
            );
        }
    }

    #[test]
    fn badge_style_assigns_distinct_color_per_variant() {
        let colors: HashSet<_> = ALL_BADGES.into_iter().map(|b| badge_style(b).fg).collect();
        assert_eq!(
            colors.len(),
            6,
            "all six badge variants must have distinct fg colors, got {colors:?}"
        );
    }

    // --- *_style() helpers ---

    #[test]
    fn active_tab_style_swaps_fg_and_bg() {
        // The deliberate swap (fg=BG, bg=HN_ORANGE) is what visually
        // signals "selected" — assert it directly so an accidental
        // re-swap doesn't silently invert contrast.
        let s = active_tab_style();
        assert_eq!(s.fg, Some(BG));
        assert_eq!(s.bg, Some(HN_ORANGE));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn pinned_style_uses_body_background_not_surface() {
        // Pins paint in unselected story rows, so bg must be BG (not
        // SURFACE) — easy to flip by accident when refactoring palette.
        let s = pinned_style();
        assert_eq!(s.fg, Some(HN_ORANGE));
        assert_eq!(s.bg, Some(BG));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn selected_style_is_bold_text_on_surface() {
        let s = selected_style();
        assert_eq!(s.fg, Some(TEXT));
        assert_eq!(s.bg, Some(SURFACE));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn hint_active_style_matches_active_tab_shape() {
        // Two distinct call sites for the same visual intent — pin them
        // together so future divergence becomes intentional.
        let s = hint_active_style();
        assert_eq!(s.fg, Some(BG));
        assert_eq!(s.bg, Some(HN_ORANGE));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }
}
