//! Home is a horizontal shelf, not a track table. Small terminals show one
//! selected card at a time; left/right navigation is identical at every size.
use ratatui::{layout::{Alignment, Rect}, style::{Modifier, Style}, widgets::{Block, Borders, Paragraph}, Frame};
use crate::{app::{App, Focus, Palette}, home::Kind, i18n::Lang};
use super::parts::{ellipsize, wrap_words};

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let hint = match app.lang {
        Lang::Ru => "← → выбор · Enter открыть",
        Lang::En => "← → select · Enter open",
    };
    let inner = super::framed_panel(frame, area, &app.list_caption(), hint, app.focus == Focus::List, p);
    if inner.width == 0 || inner.height == 0 { return; }
    let selected = app.selected.min(2);
    let wide = inner.width >= 47;
    if wide {
        let available = inner.width.saturating_sub(2);
        let width = available / 3;
        for index in 0..3 {
            let x = inner.x + index as u16 * (width + 1);
            let card_width = if index == 2 { inner.right() - x } else { width };
            card(frame, Rect::new(x, inner.y, card_width, inner.height), index, app, p);
        }
    } else {
        card(frame, inner, selected, app, p);
    }
}

fn card(frame: &mut Frame, area: Rect, index: usize, app: &App, p: &Palette) {
    if area.width < 3 || area.height < 3 { return; }
    let selected = app.selected.min(2) == index;
    let focused = selected && app.focus == Focus::List;
    let border = if focused { p.text } else { p.line };
    let caption = if selected { format!(" ‹ {} / 3 › ", index + 1) } else { format!(" {} / 3 ", index + 1) };
    let block = Block::default().borders(Borders::ALL).border_style(Style::default().fg(border)).title(caption);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let title = Kind::ALL[index].title(app.lang);
    let title = wrap_words(title, usize::from(inner.width));
    let title_rows = title.len().min(2).min(usize::from(inner.height));
    let detail = match app.home.playlist(index) {
        Some(card) => app.lang.tracks(card.playlist.track_count),
        None => match (app.home.loading, app.home.failed, app.lang) {
            (true, _, Lang::Ru) => "Загрузка…".into(),
            (true, _, Lang::En) => "Loading…".into(),
            (_, true, Lang::Ru) => "Enter: повторить".into(),
            (_, true, Lang::En) => "Enter: retry".into(),
            (_, _, Lang::Ru) => "Пока недоступен".into(),
            (_, _, Lang::En) => "Not available yet".into(),
        },
    };
    let detail_rows = usize::from(usize::from(inner.height) > title_rows);
    let image_rows = usize::from(inner.height).saturating_sub(title_rows + detail_rows + 1)
        .min(usize::from(inner.width) / 2);
    // A one-row thumbnail is unreadable. Keep a clean text card instead.
    let image_rows = if image_rows >= 3 { image_rows } else { 0 };
    let image_width = image_rows * 2;
    let image_x = inner.x + inner.width.saturating_sub(image_width as u16) / 2;
    if image_rows > 0 {
        if let Some(preview) = app.home.preview(index) {
            for y in 0..image_rows {
                for x in 0..image_width {
                    let (ch, fg, _) = preview.cell(x, y, image_width, image_rows);
                    let shade = if focused { fg } else { fg.saturating_sub(18) };
                    frame.buffer_mut()[(image_x + x as u16, inner.y + y as u16)]
                        .set_char(ch).set_style(Style::default().fg(ratatui::style::Color::Rgb(shade, shade, shade)).bg(p.bg));
                }
            }
        } else {
            let placeholder = Rect::new(inner.x, inner.y + image_rows as u16 / 2, inner.width, 1);
            frame.render_widget(Paragraph::new("◌").alignment(Alignment::Center).style(Style::default().fg(p.muted)), placeholder);
        }
    }
    let text_y = if image_rows > 0 { inner.y + image_rows as u16 + 1 }
        else { inner.y + inner.height.saturating_sub((title_rows + detail_rows) as u16) / 2 };
    for (row, title) in title.iter().take(title_rows).enumerate() {
        frame.render_widget(Paragraph::new(ellipsize(title, usize::from(inner.width)))
            .alignment(Alignment::Center).style(Style::default().fg(p.text).add_modifier(if selected { Modifier::BOLD } else { Modifier::empty() })),
            Rect::new(inner.x, text_y + row as u16, inner.width, 1));
    }
    if detail_rows > 0 {
        frame.render_widget(Paragraph::new(ellipsize(&detail, usize::from(inner.width)))
            .alignment(Alignment::Center).style(Style::default().fg(p.muted)),
            Rect::new(inner.x, text_y + title_rows as u16, inner.width, 1));
    }
}
