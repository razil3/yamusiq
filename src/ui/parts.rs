//! Smaller building blocks: header, frequency bands, thin meters, timeline,
//! signal trace and the track card.

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    symbols::Marker,
    text::{Line, Span},
    widgets::{
        canvas::{Canvas, Line as CanvasLine},
        Paragraph,
    },
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
    if app.net.loading_stream() { return t.buffering; }
    match app.player.state() {
        PlayerState::Playing => t.states[0],
        PlayerState::Paused => t.states[1],
        PlayerState::Stopped => t.states[2],
    }
}

pub(super) fn header(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let left = Rect { x: area.x + 1, width: area.width.saturating_sub(1), ..area };
    emblem(frame, left, app, p);

    // Wordmark: letter-spaced, quiet subtitle, then where the music comes from.
    let mark = Rect { x: left.x + EMBLEM_W + 2, width: left.width.saturating_sub(EMBLEM_W + 2), ..left };
    let wordmark = vec![
        Line::from(Span::styled("y a m u s i q", Style::default().fg(p.accent).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(t.tagline, Style::default().fg(p.text))),
        Line::from(Span::styled(
            format!("{} · v{}", app.service.name(), env!("CARGO_PKG_VERSION")),
            Style::default().fg(p.muted),
        )),
    ];
    frame.render_widget(Paragraph::new(wordmark), mark);

    // Right: what's on, in plain words.
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
    let summary_w = summary.iter().map(Line::width).max().unwrap_or(0) as u16 + 1;
    let right = Rect { x: (area.x + area.width).saturating_sub(summary_w), width: summary_w.min(area.width), ..area };
    frame.render_widget(Paragraph::new(summary).alignment(Alignment::Right), right);
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

/// Timeline: thin rule with a bright playhead, light ticks and time labels.
pub(super) fn timeline(buf: &mut Buffer, area: Rect, ratio: f64, ticks: &[String], p: &Palette) {
    if area.width < 10 || area.height == 0 {
        return;
    }
    thin_bar(buf, Rect { height: 1, ..area }, ratio, p);
    let head = ((f64::from(area.width - 1)) * ratio.clamp(0.0, 1.0)).round() as u16;
    set(buf, area, head, 0, "●", Style::default().fg(p.accent));
    if area.height < 2 {
        return;
    }
    let n = ticks.len().max(1) as u16;
    for (k, label) in ticks.iter().enumerate() {
        let x = area.width * k as u16 / n;
        let next = area.width * (k as u16 + 1) / n;
        set(buf, area, x, 1, "╵", Style::default().fg(p.line));
        if text_width(label) + 2 < usize::from(next - x) {
            set_str(buf, area, x + 1, 1, label, Style::default().fg(p.muted));
        }
    }
}

/// Signal history as a single smooth braille line.
pub(super) fn signal(frame: &mut Frame, inner: Rect, app: &App, p: &Palette) {
    if inner.width < 2 || inner.height == 0 {
        return;
    }
    let w = f64::from(inner.width) * 2.0;
    let h = f64::from(inner.height) * 4.0;
    let hist = &app.history;
    let samples = (w as usize).min(hist.len());
    let start = hist.len() - samples;
    let pts: Vec<(f64, f64)> = hist[start..]
        .iter()
        .enumerate()
        .map(|(i, v)| (i as f64 * w / samples.max(1) as f64, 0.5 + (*v).min(100) as f64 / 100.0 * (h - 1.5)))
        .collect();
    let (text, bg) = (p.text, p.bg);
    frame.render_widget(
        Canvas::default()
            .marker(Marker::Braille)
            .background_color(bg)
            .x_bounds([0.0, w])
            .y_bounds([0.0, h])
            .paint(move |ctx| {
                for pair in pts.windows(2) {
                    let ((x1, y1), (x2, y2)) = (pair[0], pair[1]);
                    ctx.draw(&CanvasLine { x1, y1, x2, y2, color: text });
                }
            }),
        inner,
    );
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
    match app.current_track() {
        Some(tr) => {
            let mut title = vec![Span::styled(tr.title.clone(), Style::default().fg(p.accent).add_modifier(Modifier::BOLD))];
            if app.is_saved(tr) {
                title.push(Span::styled(" ⇣", Style::default().fg(p.text)));
            }
            lines.push(Line::from(title));
            lines.push(Line::from(Span::styled(tr.artist.clone(), Style::default().fg(p.text))));
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
            lines.push(row(keys[0], tr.album.clone()));
            lines.push(row(keys[1], format_time(tr.duration())));
        }
        None => {
            lines.push(Line::from(Span::styled(t.not_playing, Style::default().fg(p.accent).add_modifier(Modifier::BOLD))));
            lines.push(Line::from(Span::styled(t.not_playing_hint, Style::default().fg(p.muted))));
        }
    }
    lines.push(row(keys[3], quiet(app.mode.label(app.lang))));
    lines.push(row(keys[4], quiet(state_label(app))));
    // A breathing line under the title block, only when there is room for it.
    if lines.len() < usize::from(inner.height) {
        lines.insert(2, Line::from(""));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::quiet;

    #[test]
    fn quiet_folds_label_but_keeps_data() {
        assert_eq!(quiet("ВИЗУАЛИЗАТОР"), "Визуализатор");
        assert_eq!(quiet("ИСПОЛНИТЕЛЬ: Neon Harbor"), "Исполнитель: Neon Harbor");
        assert_eq!(quiet("ОЧЕРЕДЬ: 1/14"), "Очередь: 1/14");
    }
}
