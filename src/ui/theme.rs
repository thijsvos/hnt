//! Catppuccin Mocha-inspired color palette and ratatui [`Style`] helpers.
//!
//! Exposes named [`Color`] constants for base, accent, and semantic
//! (status/metadata/badge) uses, plus `*_style()` constructors that pair
//! foreground + background for a widget context. [`depth_color`] cycles
//! through [`DEPTH_COLORS`] for comment indentation.

use ratatui::style::{Color, Modifier, Style};

pub const BG: Color = Color::Rgb(30, 30, 46);
pub const SURFACE: Color = Color::Rgb(49, 50, 68);
#[allow(dead_code)]
pub const OVERLAY: Color = Color::Rgb(69, 71, 90);
pub const TEXT: Color = Color::Rgb(205, 214, 244);
pub const SUBTEXT: Color = Color::Rgb(166, 173, 200);
pub const DIM: Color = Color::Rgb(127, 132, 156);
pub const HN_ORANGE: Color = Color::Rgb(255, 102, 0);
pub const GREEN: Color = Color::Rgb(166, 227, 161);
pub const RED: Color = Color::Rgb(243, 139, 168);
pub const BLUE: Color = Color::Rgb(137, 180, 250);
pub const YELLOW: Color = Color::Rgb(249, 226, 175);
pub const MAUVE: Color = Color::Rgb(203, 166, 247);
pub const TEAL: Color = Color::Rgb(148, 226, 213);
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
