//! Smaller building blocks: header, frequency bands, thin meters, timeline,
//! signal trace and the track card.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

use super::{set, set_str, text_width};
use crate::app::{App, Palette};
use crate::i18n::fill;
use crate::model::format_time;
use crate::player::PlayerState;

/// "ВИЗУАЛИЗАТОР" → "Визуализатор". Only the part before ": " is folded so
/// user data (artist names, queries) keeps its case.
pub(super) fn quiet(label: &str) -> String {
    let (head, tail) = match label.split_once(": ") {
        Some((h, t)) => (h, Some(t)),
        None => (label, None),
    };
    let mut out = String::with_capacity(label.len());
    for (i, c) in head.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else {
            out.extend(c.to_lowercase());
        }
    }
    if let Some(t) = tail {
        out.push_str(": ");
        out.push_str(t);
    }
    out
}

/// Emblem size in cells.
const EMBLEM_W: u16 = 7;

/// The yamusiq mark: a bold half-block disc — a record seen from above —
/// with a spindle dot. While music plays a white glint travels round the rim
/// as if the disc were spinning, and the spindle lights up.
///
/// ```text
/// ▄█████▄
/// ██ ● ██
/// ▀█████▀
/// ```
const EMBLEM: [&str; 3] = ["▄█████▄", "██ ● ██", "▀█████▀"];

/// Rim cells in clockwise order, for the glint.
fn rim_cells() -> Vec<(u16, u16)> {
    let mut cells: Vec<(u16, u16)> = (0..EMBLEM_W).map(|x| (x, 0)).collect();
    cells.push((EMBLEM_W - 1, 1));
    cells.extend((0..EMBLEM_W).rev().map(|x| (x, 2)));
    cells.push((0, 1));
    cells
}

fn emblem(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let buf = frame.buffer_mut();
    let playing = app.player.state() == PlayerState::Playing;
    for (y, row) in EMBLEM.iter().enumerate() {
        for (x, ch) in row.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let color = if ch == '●' {
                if playing { p.accent } else { p.muted }
            } else {
                p.text
            };
            let mut tmp = [0u8; 4];
            set(buf, area, x as u16, y as u16, ch.encode_utf8(&mut tmp), Style::default().fg(color));
        }
    }
    if playing {
        let rim = rim_cells();
        let head = (app.frame / 2) as usize % rim.len();
        for (k, color) in [(0, p.accent), (1, p.muted)] {
            let (x, y) = rim[(head + rim.len() - k) % rim.len()];
            if let Some(cell) = buf.cell_mut((area.x + x, area.y + y)).filter(|_| x < area.width && y < area.height) {
                cell.set_fg(color);
            }
        }
    }
}

pub(super) fn state_label(app: &App) -> &'static str {
    let t = app.lang.t();
    if app.buffering() { return t.buffering; }
    match app.player.state() {
        PlayerState::Playing => t.states[0],
        PlayerState::Paused => t.states[1],
        PlayerState::Stopped => t.states[2],
    }
}

/// Clip a title by terminal-cell width, reserving a cell for the ellipsis.
pub(super) fn ellipsize(text: &str, width: usize) -> String {
    if width == 0 { return String::new(); }
    if text_width(text) <= width { return text.to_string(); }
    let mut out = String::new();
    for ch in text.chars() {
        out.push(ch);
        if text_width(&out) > width - 1 { out.pop(); break; }
    }
    out.push('…');
    out
}

pub(super) fn header(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let wordmark = vec![
        Line::from(Span::styled("y a m u s i q", Style::default().fg(p.accent).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(t.tagline, Style::default().fg(p.text))),
        Line::from(Span::styled(
            app.service.name(),
            Style::default().fg(p.muted),
        )),
    ];
    let (pos, dur) = match app.current_track() {
        Some(tr) => (app.player.position(), tr.duration()),
        None => (Default::default(), Default::default()),
    };
    let live = app.player.state() == PlayerState::Playing;
    let summary = vec![
        Line::from(vec![
            Span::styled(if live { "● " } else { "○ " }, Style::default().fg(if live { p.accent } else { p.muted })),
            Span::styled(quiet(state_label(app)), Style::default().fg(p.accent).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled(format!("{} / {}", format_time(pos), format_time(dur)), Style::default().fg(p.text))),
        Line::from(Span::styled(
            format!(
                "{} · {}",
                quiet(&fill(t.lbl_queue, &[&format!("{}/{}", app.queue_pos.map_or(0, |i| i + 1), app.queue.len())])),
                quiet(app.mode.label(app.lang))
            ),
            Style::default().fg(p.muted),
        )),
    ];
    // Give the brand and summary their own bounds. The centre can shrink
    // without its title ever painting over either neighbouring block.
    let brand_w = EMBLEM_W + 2 + wordmark.iter().map(Line::width).max().unwrap_or(0) as u16;
    let summary_w = summary.iter().map(Line::width).max().unwrap_or(0) as u16;
    let inside = Rect { x: area.x + u16::from(area.width > 0), width: area.width.saturating_sub(2), ..area };
    let columns = Layout::horizontal([Constraint::Length(brand_w), Constraint::Min(0), Constraint::Length(summary_w)])
        .spacing(11).split(inside);
    emblem(frame, columns[0], app, p);
    let mark_offset = (EMBLEM_W + 2).min(columns[0].width);
    let mark = Rect { x: columns[0].x + mark_offset, width: columns[0].width - mark_offset, ..columns[0] };
    frame.render_widget(Paragraph::new(wordmark), mark);
    frame.render_widget(Paragraph::new(summary).alignment(Alignment::Right), columns[2]);
    now_playing(frame, columns[1], app, p);
}

fn now_playing(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    if area.width == 0 || area.height == 0 { return; }
    let Some(track) = app.current_track() else {
        if area.height >= 2 {
            let title = ellipsize(app.lang.t().not_playing, usize::from(area.width));
            frame.render_widget(Paragraph::new(title).alignment(Alignment::Center).style(Style::default().fg(p.muted)), Rect { y: area.y + 1, height: 1, ..area });
        }
        return;
    };
    let icon = match app.player.state() {
        PlayerState::Playing => "▶",
        PlayerState::Paused => "⏸",
        PlayerState::Stopped => "",
    };
    frame.render_widget(Paragraph::new(icon).alignment(Alignment::Center).style(Style::default().fg(p.accent)), Rect { height: 1, ..area });
    if area.height < 2 { return; }
    let title = ellipsize(&track.title, usize::from(area.width));
    frame.render_widget(Paragraph::new(title).alignment(Alignment::Center).style(Style::default().fg(p.accent).add_modifier(Modifier::BOLD)), Rect { y: area.y + 1, height: 1, ..area });
    if area.height < 3 { return; }
    let pos = app.player.position().min(track.duration());
    let elapsed = format_time(pos);
    let duration = format_time(track.duration());
    let elapsed_w = text_width(&elapsed) as u16;
    let duration_w = text_width(&duration) as u16;
    let row = Rect { y: area.y + 2, height: 1, ..area };
    if area.width < elapsed_w + duration_w + 3 {
        let times = ellipsize(&format!("{elapsed} / {duration}"), usize::from(area.width));
        frame.render_widget(Paragraph::new(times).alignment(Alignment::Center).style(Style::default().fg(p.muted)), row);
        return;
    }
    let buf = frame.buffer_mut();
    set_str(buf, row, 0, 0, &elapsed, Style::default().fg(p.text));
    set_str(buf, row, area.width - duration_w, 0, &duration, Style::default().fg(p.muted));
    let bar = Rect { x: row.x + elapsed_w + 1, width: area.width - elapsed_w - duration_w - 2, ..row };
    let ratio = if track.duration().is_zero() { 0.0 } else { pos.as_secs_f64() / track.duration().as_secs_f64() };
    thin_bar(buf, bar, ratio, p);
}

/// Six labelled frequency bands drawn as thin rules with a peak-hold tick.
pub(super) fn bands(frame: &mut Frame, inner: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    if inner.width < 12 || inner.height == 0 {
        return;
    }
    let n = app.levels.len().max(1);
    let rows = usize::from(inner.height).min(t.bands.len());
    // Spread rows out when there is room so the section breathes.
    let step = (usize::from(inner.height) / rows.max(1)).clamp(1, 2);
    let label_w = t.bands.iter().map(|b| text_width(b)).max().unwrap_or(0);
    let buf = frame.buffer_mut();
    for (i, label) in t.bands.iter().take(rows).enumerate() {
        let lo = i * n / t.bands.len();
        let hi = ((i + 1) * n / t.bands.len()).max(lo + 1).min(n);
        let span = (hi - lo) as f64;
        let lvl = (app.levels[lo..hi].iter().sum::<u64>() as f64 / span / 100.0 * 1.2).min(1.0);
        let peak = (app.peaks[lo..hi].iter().cloned().fold(0.0, f64::max) * 1.2).min(1.0);
        let y = (i * step) as u16;
        set_str(buf, inner, 0, y, &format!("{label:>label_w$} "), Style::default().fg(p.muted));
        let x0 = (label_w + 1) as u16;
        let bar = Rect { x: inner.x + x0, y: inner.y + y, width: inner.width.saturating_sub(x0), height: 1 };
        thin_bar(buf, bar, lvl, p);
        if peak > lvl + 0.02 && bar.width > 0 {
            let px = ((f64::from(bar.width - 1)) * peak).round() as u16;
            set(buf, bar, px, 0, "│", Style::default().fg(p.muted));
        }
    }
}

/// `━━━━━━────────` — filled part in copy grey, remainder in the rule colour.
pub(super) fn thin_bar(buf: &mut Buffer, area: Rect, ratio: f64, p: &Palette) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let filled = (f64::from(area.width) * ratio.clamp(0.0, 1.0)).round() as u16;
    for x in 0..area.width {
        let (sym, color) = if x < filled { ("━", p.text) } else { ("─", p.line) };
        set(buf, area, x, 0, sym, Style::default().fg(color));
    }
}

/// Wrap at whitespace boundaries only; a word wider than the column stays
/// intact for the renderer to ellipsize, never split into multiple rows.
pub(super) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut rows = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && text_width(&current) + 1 + text_width(word) > width {
            rows.push(std::mem::take(&mut current));
        }
        if !current.is_empty() { current.push(' '); }
        current.push_str(word);
    }
    if !current.is_empty() || rows.is_empty() { rows.push(current); }
    rows
}

/// Track card: bright title, artist beneath, then a quiet dot-leader table.
pub(super) fn info(frame: &mut Frame, inner: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let keys = t.info_keys;
    let key_w = keys.iter().map(|k| text_width(k)).max().unwrap_or(0) + 3;
    let row = |k: &str, v: String| {
        let dots = " ·".repeat(key_w.saturating_sub(text_width(k)) / 2);
        Line::from(vec![
            Span::styled(k.to_string(), Style::default().fg(p.muted)),
            Span::styled(format!("{dots:<width$} ", width = key_w - text_width(k)), Style::default().fg(p.line)),
            Span::styled(v, Style::default().fg(p.text)),
        ])
    };
    // Rows in priority order: when the card is short, the tail is cut, so
    // what matters most (title, artist, quality) comes first.
    let mut lines = Vec::new();
    let mut heading_end = 2;
    match app.current_track() {
        Some(tr) => {
            let title = format!("{}{}", tr.title, if app.is_saved(tr) { " ⇣" } else { "" });
            for text in wrap_words(&title, usize::from(inner.width)) {
                lines.push(Line::from(Span::styled(ellipsize(&text, usize::from(inner.width)),
                    Style::default().fg(p.accent).add_modifier(Modifier::BOLD))));
            }
            lines.push(Line::from(Span::styled(tr.artist.clone(), Style::default().fg(p.text))));
            heading_end = lines.len();
            if let Some(q) = &app.quality {
                lines.push(row(keys[2], q.clone()));
                if app.lossless_missing {
                    // Aligned under the value when it fits, quiet but unmistakable.
                    let fits = key_w + 1 + text_width(t.lossless_missing) <= usize::from(inner.width);
                    let indent = if fits { " ".repeat(key_w + 1) } else { String::new() };
                    lines.push(Line::from(vec![
                        Span::raw(indent),
                        Span::styled(t.lossless_missing, Style::default().fg(p.muted).add_modifier(Modifier::ITALIC)),
                    ]));
                }
            }
            // Library state: saved (with the stored quality) or downloading.
            if let Some(f) = app.library.progress(&tr.id) {
                lines.push(row(keys[5], fill(t.file_saving, &[&format!("{:.0}", f * 100.0)])));
            } else if let Some(saved) = app.library.saved(&tr.id) {
                lines.push(row(keys[5], format!("{} · {}", t.file_saved, saved.quality)));
            }
            let indent = key_w + 1;
            let width = usize::from(inner.width);
            if width > indent {
                let value_width = width - indent;
                for (i, text) in wrap_words(&tr.album, value_width).into_iter().enumerate() {
                    let text = ellipsize(&text, value_width);
                    if i == 0 { lines.push(row(keys[0], text)); }
                    else {
                        lines.push(Line::from(vec![Span::raw(" ".repeat(indent)), Span::styled(text, Style::default().fg(p.text))]));
                    }
                }
            } else {
                // Very narrow cards have no value column; put the album below
                // its label, using the available width rather than overflowing.
                lines.push(row(keys[0], String::new()));
                for text in wrap_words(&tr.album, width) {
                    lines.push(Line::from(Span::styled(ellipsize(&text, width), Style::default().fg(p.text))));
                }
            }
        }
        None => {
            lines.push(Line::from(Span::styled(t.not_playing, Style::default().fg(p.accent).add_modifier(Modifier::BOLD))));
            lines.push(Line::from(Span::styled(t.not_playing_hint, Style::default().fg(p.muted))));
        }
    }
    lines.push(row(keys[3], quiet(app.mode.label(app.lang))));
    // Stack artwork above the complete text card. Height may limit its size,
    // but never reserve a narrow text column beside the image or crop metadata.
    if let Some(tr) = app.current_track()
        && let Some((id, preview)) = &app.cover_preview
        && id == &tr.id
    {
        let available_rows = usize::from(inner.height).saturating_sub(lines.len() + 1);
        let rows = available_rows.min(usize::from(inner.width).div_ceil(2));
        let width = usize::from(inner.width).min(rows * 2);
        if width >= 10 && rows >= 5 {
            let offset = (usize::from(inner.width) - width) / 2;
            for y in 0..rows { for x in 0..width {
                let (glyph, fg, bg) = preview.cell(x, y, width, rows);
                let mut utf8 = [0; 4];
                frame.buffer_mut()[(inner.x + (offset + x) as u16, inner.y + y as u16)]
                    .set_symbol(glyph.encode_utf8(&mut utf8)).set_style(Style::default()
                        .fg(ratatui::style::Color::Rgb(fg, fg, fg))
                        .bg(ratatui::style::Color::Rgb(bg, bg, bg)));
            } }
            let text_y = rows as u16 + 1;
            frame.render_widget(Paragraph::new(lines), Rect {
                y: inner.y + text_y, height: inner.height - text_y, ..inner
            });
            return;
        }
    }
    // A breathing line under the title block, only when there is room for it.
    if lines.len() < usize::from(inner.height) {
        lines.insert(heading_end, Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::quiet;

    #[test]
    fn album_wrap_uses_word_boundaries_and_display_width() {
        assert_eq!(super::wrap_words("один два три", 8), ["один два", "три"]);
        assert_eq!(super::wrap_words("one two three", 7), ["one two", "three"]);
        assert_eq!(super::wrap_words("夜空 星々", 4), ["夜空", "星々"]);
        assert_eq!(super::wrap_words("unbreakableword next", 5), ["unbreakableword", "next"]);
        assert_eq!(super::wrap_words("", 10), [""]);
    }

    #[test]
    fn long_titles_are_clipped_by_terminal_width() {
        assert_eq!(super::ellipsize("abcdef", 4), "abc…");
        assert_eq!(super::ellipsize("界界界", 5), "界界…");
        assert_eq!(super::ellipsize("Север", 3), "Се…");
        assert_eq!(super::ellipsize("abcdef", 1), "…");
        assert_eq!(super::ellipsize("abcdef", 0), "");
        assert_eq!(super::ellipsize("abc", 3), "abc");
    }

    #[test]
    fn quiet_folds_label_but_keeps_data() {
        assert_eq!(quiet("ВИЗУАЛИЗАТОР"), "Визуализатор");
        assert_eq!(quiet("ИСПОЛНИТЕЛЬ: Neon Harbor"), "Исполнитель: Neon Harbor");
        assert_eq!(quiet("ОЧЕРЕДЬ: 1/14"), "Очередь: 1/14");
    }
}
