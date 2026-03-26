use crate::state::reader_state::ReaderState;
use crate::ui::theme;
use ratatui::{
    layout::Rect,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render_article_overlay(frame: &mut Frame, area: Rect, reader: &ReaderState) {
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

    render_content(frame, overlay_area, reader);
}

fn build_title(reader: &ReaderState) -> String {
    let title = if reader.title.chars().count() > 60 {
        let truncated: String = reader.title.chars().take(57).collect();
        format!("{}...", truncated)
    } else {
        reader.title.clone()
    };

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
        Line::from(Span::styled(error.to_string(), theme::dim_style())),
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

fn render_content(frame: &mut Frame, area: Rect, reader: &ReaderState) {
    let title = build_title(reader);
    let pct = reader.scroll_percent();
    let footer = format!(
        " j/k:scroll  Ctrl+d/u:page  g/G:top/bottom  o:browser  Esc:close  {}% ",
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

    let visible_lines = reader
        .lines
        .iter()
        .skip(scroll)
        .take(visible_height)
        .map(|fragments| {
            let spans: Vec<Span> = fragments
                .iter()
                .map(|f| Span::styled(f.text.clone(), f.style))
                .collect();
            Line::from(spans)
        })
        .collect::<Vec<_>>();

    let content = Paragraph::new(visible_lines).wrap(Wrap { trim: false });
    frame.render_widget(content, inner);
}
