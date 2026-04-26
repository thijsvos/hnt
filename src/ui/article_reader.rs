//! Full-screen article-reader overlay rendering.
//!
//! [`render_article_overlay`] dispatches to loading, error, or content
//! renderers based on [`ReaderState`]. Content is drawn as wrapped
//! styled lines with a title, domain, and keybinding footer. When
//! Quickjump's [`HintState`] is active, a post-pass paints label glyphs
//! over each visible hyperlink, dimming labels that no longer match the
//! typed prefix.

use crate::state::hint_state::{HintContext, HintState};
use crate::state::reader_state::ReaderState;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

/// Draws the full-screen reader overlay for `reader`'s current state
/// (loading, error, or content) into `area` with a small margin. No-op if
/// the available space is too small. When `hint` is `Some` and targets the
/// reader, a label-overlay pass paints Quickjump labels atop visible links.
pub fn render_article_overlay(
    frame: &mut Frame,
    area: Rect,
    reader: &ReaderState,
    hint: Option<&HintState>,
) {
    // Fullscreen overlay with 2-cell margin on each side
    let margin = 2u16;
    let x = margin.min(area.width / 2);
    let y = 1u16.min(area.height / 2);
    let width = area.width.saturating_sub(x * 2);
    let height = area.height.saturating_sub(y * 2);

    if width < 10 || height < 5 {
        return;
    }

    let overlay_area = Rect::new(x, y, width, height);

    frame.render_widget(Clear, overlay_area);

    if reader.loading {
        render_loading(frame, overlay_area, reader);
        return;
    }

    if let Some(ref err) = reader.error {
        render_error(frame, overlay_area, reader, err);
        return;
    }

    let inner = render_content(frame, overlay_area, reader);

    // Hint-mode label overlay — draws over the just-rendered content.
    if let Some(hint) = hint {
        if matches!(hint.context, HintContext::Reader) {
            paint_hint_labels(frame, inner, reader, hint);
        }
    }
}

fn build_title(reader: &ReaderState) -> String {
    let title = crate::ui::util::truncate_to(&reader.title, 60);
    match &reader.domain {
        Some(domain) => format!(" {} ({}) ", title, domain),
        None => format!(" {} ", title),
    }
}

fn render_loading(frame: &mut Frame, area: Rect, reader: &ReaderState) {
    let title = build_title(reader);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent_style())
        .title(Span::styled(title, theme::title_style()))
        .style(theme::base_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let loading_text = Paragraph::new(Line::from(Span::styled(
        "Loading article...",
        theme::dim_style(),
    )))
    .alignment(ratatui::layout::Alignment::Center);

    // Center vertically
    let y_offset = inner.height / 2;
    if y_offset > 0 && inner.height > 0 {
        let centered = Rect::new(inner.x, inner.y + y_offset, inner.width, 1);
        frame.render_widget(loading_text, centered);
    }
}

fn render_error(frame: &mut Frame, area: Rect, reader: &ReaderState, error: &str) {
    let title = build_title(reader);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent_style())
        .title(Span::styled(title, theme::title_style()))
        .style(theme::base_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let error_lines = vec![
        Line::from(Span::styled(
            "Failed to load article",
            ratatui::style::Style::default()
                .fg(theme::RED)
                .add_modifier(ratatui::style::Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(Span::styled(error, theme::dim_style())),
        Line::from(""),
        Line::from(Span::styled(
            "Press 'o' to open in browser, Esc to close",
            theme::dim_style(),
        )),
    ];

    let error_para = Paragraph::new(error_lines)
        .alignment(ratatui::layout::Alignment::Center)
        .wrap(Wrap { trim: false });

    let y_offset = inner.height / 3;
    if inner.height > y_offset + 5 {
        let centered = Rect::new(
            inner.x,
            inner.y + y_offset,
            inner.width,
            inner.height - y_offset,
        );
        frame.render_widget(error_para, centered);
    }
}

fn render_content(frame: &mut Frame, area: Rect, reader: &ReaderState) -> Rect {
    let title = build_title(reader);
    let pct = reader.scroll_percent();
    let footer = format!(
        " j/k:scroll  Ctrl+d/u:page  g/G:top/bottom  o:browser  f:hint  y:copy  Esc:close  {}% ",
        pct
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme::accent_style())
        .title(Span::styled(title, theme::title_style()))
        .title_bottom(Line::from(Span::styled(footer, theme::dim_style())).centered())
        .style(theme::base_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    let scroll = reader.scroll;

    // Borrow fragment text instead of cloning — Span<'_> holds a Cow<str>
    // and happily borrows from `reader.lines` for the frame's lifetime.
    let visible_lines = reader
        .lines
        .iter()
        .skip(scroll)
        .take(visible_height)
        .map(|fragments| {
            let spans: Vec<Span> = fragments
                .iter()
                .map(|f| Span::styled(f.text.as_str(), f.style))
                .collect();
            Line::from(spans)
        })
        .collect::<Vec<_>>();

    let content = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
    frame.render_widget(content, inner);

    inner
}

/// Paints Quickjump hint labels atop visible hyperlinks. Each label
/// renders at the link's first column with a high-contrast highlight
/// (HN-orange background); labels whose full text no longer starts with
/// the typed prefix dim instead, so the user can see what they've
/// ruled out without having labels disappear.
///
/// Column positioning uses `chars().count()` rather than full unicode
/// width — adequate for the URLs and ASCII link text typical of news
/// articles; wide-char link anchors (CJK, emoji) may be off by a
/// column. Labels paint at the html2text logical row index, so any
/// further re-wrap by ratatui's `Paragraph` (e.g. when the terminal is
/// narrower than the width html2text was given) misplaces labels by
/// the number of intervening wraps.
fn paint_hint_labels(frame: &mut Frame, inner: Rect, reader: &ReaderState, hint: &HintState) {
    let buf = frame.buffer_mut();
    let scroll = reader.scroll;
    let visible_height = inner.height as usize;
    let prefix = hint.buffer();

    for link in &reader.links.links {
        if link.line < scroll || link.line >= scroll + visible_height {
            continue;
        }
        // `link.col` is pre-computed at registry-build time
        // (`tagged_lines_to_styled_with_links`) — no per-keystroke
        // chars().count() summing across preceding fragments here.
        let label_x = inner.x.saturating_add(link.col as u16);
        if label_x >= inner.right() {
            continue;
        }
        let row_y = inner.y.saturating_add((link.line - scroll) as u16);
        if row_y >= inner.bottom() {
            continue;
        }

        let style = if link.label.starts_with(prefix) {
            theme::hint_active_style()
        } else {
            theme::hint_dim_style()
        };

        for (i, ch) in link.label.chars().enumerate() {
            let cell_x = label_x.saturating_add(i as u16);
            if cell_x >= inner.right() {
                break;
            }
            let cell = &mut buf[(cell_x, row_y)];
            cell.set_char(ch);
            cell.set_style(style);
        }
    }
}
