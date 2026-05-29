//! Command-palette overlay rendering.
//!
//! [`render_command_palette`] draws a centred popup listing every
//! command in [`crate::command::CommandRegistry`] (filtered by the typed
//! query, ranked by [`crate::command::CommandRegistry::fuzzy`]). Follows
//! the same visual pattern as [`crate::ui::prior_overlay`] for
//! consistency: `Clear` over a centred [`Rect`], `Block` with title
//! chrome, dim footer hint line.
//!
//! Every dynamic string (the query echo, command names, descriptions)
//! is sanitised at the render boundary via
//! [`crate::sanitize::sanitize_terminal`]. Command metadata is
//! `'static` today — but the palette also echoes the user's query, and
//! future paste / `:source` paths could surface server bytes; the
//! defence-in-depth pass keeps the boundary uniform.

use crate::command::CommandRegistry;
use crate::sanitize::sanitize_terminal;
use crate::state::command_state::PaletteState;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem, Paragraph},
    Frame,
};

/// Renders the command palette overlay.
///
/// The overlay occupies up to 70% of the area's width and ~60% of its
/// height, centred. Layout from top to bottom:
/// - 1-row title bar ("Command Palette · N matches")
/// - 1-row query echo with block cursor
/// - n-row match list, selected row highlighted
/// - 1-row footer hint
///
/// Renders nothing when the available area is too small to be usable.
pub fn render_command_palette(
    frame: &mut Frame,
    area: Rect,
    palette: &PaletteState,
    registry: &CommandRegistry,
) {
    // Early-return for terminals too small to fit the minimum palette
    // (40 col × 8 row) with a 2-col / 2-row margin — clamping below
    // would otherwise underflow when the area is smaller than the
    // declared minimum.
    if area.width < 44 || area.height < 12 {
        return;
    }
    // Centred rect, capped at 70%x60% with absolute floors so a tall
    // narrow terminal still gets a usable popup.
    let max_w = (area.width as u32 * 70 / 100) as u16;
    let max_h = (area.height as u32 * 60 / 100) as u16;
    let width = max_w.max(40).min(area.width.saturating_sub(4));
    let height = max_h.max(8).min(area.height.saturating_sub(4));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let overlay = Rect::new(x, y, width, height);

    frame.render_widget(Clear, overlay);

    let title = format!(
        " Command Palette · {} match{} ",
        palette.matches.len(),
        if palette.matches.len() == 1 { "" } else { "es" }
    );
    let footer = Line::from(Span::styled(
        " ↑/↓:nav  Enter:run  Esc:close ",
        theme::dim_style(),
    ))
    .centered();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent_style())
        .title(Span::styled(title, theme::title_style()))
        .title_bottom(footer)
        .style(theme::base_style());

    let inner = block.inner(overlay);
    frame.render_widget(block, overlay);

    if inner.height < 3 || inner.width < 10 {
        return;
    }

    // Row 0: query echo with block cursor. Sanitised because future
    // paste / `:source` paths could surface server bytes through this
    // widget; today the typed chars come straight from the user but the
    // boundary is kept uniform with `status_bar`'s CommandInput branch.
    let query_area = Rect::new(inner.x, inner.y, inner.width, 1);
    let safe_query = sanitize_terminal(&palette.query);
    let query_line = Line::from(vec![
        Span::styled("❯ ", theme::accent_style()),
        Span::styled(format!("{}\u{2588}", safe_query), theme::base_style()),
    ]);
    frame.render_widget(Paragraph::new(query_line), query_area);

    // Row 1: separator.
    let sep_area = Rect::new(inner.x, inner.y + 1, inner.width, 1);
    let sep = "─".repeat(inner.width as usize);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(sep, theme::dim_style()))),
        sep_area,
    );

    // Remaining rows: match list.
    let list_area = Rect::new(
        inner.x,
        inner.y + 2,
        inner.width,
        inner.height.saturating_sub(2),
    );
    if list_area.height == 0 {
        return;
    }

    // Compute the visible window — keep the selected row in view via a
    // simple top-bias scroll. The list itself is short (≤ ~15 commands)
    // so a smarter scroll isn't needed yet.
    let visible_rows = list_area.height as usize;
    let scroll = palette
        .selected
        .saturating_sub(visible_rows.saturating_sub(1));

    let commands = registry.all();
    let items: Vec<ListItem> = palette
        .matches
        .iter()
        .enumerate()
        .skip(scroll)
        .take(visible_rows)
        .map(|(i, &cmd_idx)| {
            let cmd = &commands[cmd_idx];
            let is_selected = i == palette.selected;
            let name = sanitize_terminal(cmd.name);
            let desc = sanitize_terminal(cmd.description);
            let style = if is_selected {
                theme::accent_style().add_modifier(Modifier::BOLD)
            } else {
                theme::base_style()
            };
            let prefix = if is_selected { "▌ " } else { "  " };
            let line = Line::from(vec![
                Span::styled(prefix, style),
                Span::styled(format!(":{name}"), style),
                Span::raw("  "),
                Span::styled(desc, dim_or_active(is_selected)),
            ]);
            ListItem::new(line)
        })
        .collect();

    if items.is_empty() {
        // Empty-state placeholder
        let empty = Paragraph::new(Line::from(Span::styled(
            "  No commands match this query.",
            theme::dim_style(),
        )));
        frame.render_widget(empty, list_area);
    } else {
        frame.render_widget(List::new(items), list_area);
    }
}

fn dim_or_active(selected: bool) -> Style {
    if selected {
        theme::accent_style()
    } else {
        theme::dim_style()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn render(palette: &PaletteState, registry: &CommandRegistry, w: u16, h: u16) -> String {
        let backend = TestBackend::new(w, h);
        let mut terminal = Terminal::new(backend).expect("terminal");
        terminal
            .draw(|f| {
                render_command_palette(f, f.area(), palette, registry);
            })
            .expect("draw");
        let buf = terminal.backend().buffer().clone();
        let mut s = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                s.push_str(buf[(x, y)].symbol());
            }
            s.push('\n');
        }
        s
    }

    #[test]
    fn palette_renders_with_no_query_lists_all_commands() {
        let registry = CommandRegistry::with_builtins();
        let mut p = PaletteState::new();
        p.matches = (0..registry.all().len()).collect();
        let text = render(&p, &registry, 80, 24);
        assert!(text.contains("Command Palette"));
        assert!(text.contains(":quit"));
        assert!(text.contains(":feed"));
        assert!(text.contains(":filter"));
    }

    #[test]
    fn palette_highlights_selected_row() {
        let registry = CommandRegistry::with_builtins();
        let mut p = PaletteState::new();
        p.matches = (0..registry.all().len()).collect();
        p.selected = 2;
        let text = render(&p, &registry, 80, 24);
        // The selected row has the ▌ marker before its name.
        assert!(text.contains("\u{258c}"));
    }

    #[test]
    fn palette_empty_matches_shows_placeholder() {
        let registry = CommandRegistry::with_builtins();
        let p = PaletteState::new();
        let text = render(&p, &registry, 80, 24);
        assert!(text.contains("No commands match"));
    }

    #[test]
    fn palette_renders_match_count_in_title() {
        let registry = CommandRegistry::with_builtins();
        let mut p = PaletteState::new();
        p.matches = vec![0];
        let text = render(&p, &registry, 80, 24);
        assert!(text.contains("1 match "), "singular: {text:?}");
        p.matches = vec![0, 1, 2];
        let text = render(&p, &registry, 80, 24);
        assert!(text.contains("3 matches"), "plural: {text:?}");
    }

    #[test]
    fn palette_sanitises_query_echo() {
        // Defence in depth: even though `query` is normally typed by the
        // user today, a future paste / `:source` path could surface
        // server bytes through this widget. The boundary must stay
        // uniform with the status_bar's CommandInput branch.
        let registry = CommandRegistry::with_builtins();
        let mut p = PaletteState::new();
        p.query = "feed\x1b]0;OWNED\x07top".to_string();
        p.matches = vec![0];
        let text = render(&p, &registry, 80, 24);
        assert!(!text.contains('\x1b'), "ESC must not survive: {text:?}");
        assert!(!text.contains('\x07'), "BEL must not survive: {text:?}");
    }

    #[test]
    fn palette_skips_render_when_area_too_small() {
        let registry = CommandRegistry::with_builtins();
        let mut p = PaletteState::new();
        p.matches = vec![0];
        // 20x4 is below the 40x8 minimum.
        let text = render(&p, &registry, 20, 4);
        assert!(
            !text.contains("Command Palette"),
            "tiny area should suppress overlay: {text:?}"
        );
    }
}
