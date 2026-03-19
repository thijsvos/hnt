use ratatui::style::{Color, Modifier, Style};

// Catppuccin Mocha-inspired dark theme with HN orange accent
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

/// Colors for comment depth levels (cycles through these)
pub const DEPTH_COLORS: [Color; 6] = [HN_ORANGE, BLUE, GREEN, MAUVE, TEAL, PEACH];

pub fn base_style() -> Style {
    Style::default().fg(TEXT).bg(BG)
}

pub fn selected_style() -> Style {
    Style::default()
        .fg(TEXT)
        .bg(SURFACE)
        .add_modifier(Modifier::BOLD)
}

pub fn title_style() -> Style {
    Style::default().fg(HN_ORANGE).add_modifier(Modifier::BOLD)
}

pub fn header_style() -> Style {
    Style::default().fg(TEXT).bg(SURFACE)
}

pub fn status_style() -> Style {
    Style::default().fg(SUBTEXT).bg(SURFACE)
}

pub fn accent_style() -> Style {
    Style::default().fg(HN_ORANGE)
}

pub fn dim_style() -> Style {
    Style::default().fg(DIM)
}

pub fn meta_style() -> Style {
    Style::default().fg(SUBTEXT)
}

pub fn active_tab_style() -> Style {
    Style::default()
        .fg(BG)
        .bg(HN_ORANGE)
        .add_modifier(Modifier::BOLD)
}

pub fn inactive_tab_style() -> Style {
    Style::default().fg(SUBTEXT).bg(SURFACE)
}

pub fn depth_color(depth: usize) -> Color {
    DEPTH_COLORS[depth % DEPTH_COLORS.len()]
}

pub fn badge_style(badge: crate::api::types::StoryBadge) -> Style {
    use crate::api::types::StoryBadge;
    let color = match badge {
        StoryBadge::Ask => BLUE,
        StoryBadge::Show => GREEN,
        StoryBadge::Tell => MAUVE,
        StoryBadge::Launch => PEACH,
        StoryBadge::Job => YELLOW,
        StoryBadge::Poll => TEAL,
    };
    Style::default()
        .fg(color)
        .bg(SURFACE)
        .add_modifier(Modifier::BOLD)
}
