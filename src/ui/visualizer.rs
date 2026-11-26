//! The visualiser screen and the lyrics that float over it.
//!
//! Layers, back to front:
//! 1. spectrum skyline along the floor — fine braille bars with peak caps;
//! 2. phosphor afterglow — the waveform as it was 1–3 frames ago, fading;
//! 3. the live waveform, tapered at the edges like a scope sweep;
//! 4. centred lyrics: each line sits on the background colour only
//!    under its own glyphs, so the scene shows through between lines.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    symbols::Marker,
    widgets::canvas::{Canvas, Line as CanvasLine, Points},
    Frame,
};

use super::{framed_panel, set_str, text_width};
use crate::app::{App, Palette};
use crate::model::format_time;
use crate::player::PlayerState;

/// Afterglow tone between the rule colour and muted grey.
const GLOW_MID: Color = Color::Rgb(78, 78, 84);

pub(super) fn draw(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let inner = framed_panel(frame, area, t.lbl_visualizer, t.hint_visualizer, false, p);
    if inner.width < 8 || inner.height < 3 {
        return;
    }
    scene(frame, inner, app, p);
    lyrics(frame, inner, app, p);
}

fn scene(frame: &mut Frame, inner: Rect, app: &App, p: &Palette) {
    let w = f64::from(inner.width) * 2.0;
    let h = f64::from(inner.height) * 4.0;
    let state = app.player.state();
    let floor_h = h * 0.30;
    let mid = floor_h + (h - floor_h) * 0.52;
    let max_amp = (h - floor_h) * 0.44;

    // 1. Skyline: one fine bar per band with a gap, plus a peak cap.
    let n = app.levels.len().max(1);
    let band_w = w / n as f64;
    let mut bars = Vec::new();
    let mut caps = Vec::new();
    for (i, lvl) in app.levels.iter().enumerate() {
        let x = (i as f64 + 0.5) * band_w;
        let top = (*lvl as f64 / 100.0) * floor_h;
        let mut y = 0.0;
        while y <= top {
            bars.push((x, y));
            y += 1.0;
        }
        let peak = app.peaks.get(i).copied().unwrap_or(0.0) * floor_h;
        if peak > top + 1.0 {
            caps.push((x, peak));
        }
    }

    // 2–3. The triggered samples the listener is hearing, with the previous
    // frames as afterglow. Nothing is drawn that the audio didn't produce.
    let real = app.wave.as_ref().filter(|w| w.len() > 1 && state != PlayerState::Stopped);
    let (lo, hi) = (floor_h + 0.5, h - 0.5);
    let scope = |samples: &[f32]| -> Vec<(f64, f64, f64, f64)> {
        let steps = (w as usize).max(2);
        let last = (samples.len() - 1) as f64;
        let y = |x: f64| (mid + f64::from(samples[(x / w * last).round() as usize]) * max_amp * 1.6).clamp(lo, hi);
        (0..steps)
            .map(|i| {
                let (x1, x2) = (w * i as f64 / steps as f64, w * (i + 1) as f64 / steps as f64);
                (x1, y(x1), x2, y(x2))
            })
            .collect()
    };
    let glow_colors = [p.muted, GLOW_MID, p.line];
    let (live, glow): (Vec<_>, Vec<_>) = match real {
        Some(wave) => (
            scope(wave),
            app.wave_history.iter().zip(glow_colors).rev().filter(|(w, _)| w.len() > 1).map(|(w, c)| (scope(w), c)).collect(),
        ),
        None => (Vec::new(), Vec::new()),
    };

    // No signal: a barely-breathing flat line.
    let idle: Vec<(f64, f64)> = if real.is_none() {
        let phase = app.frame as f64 * 0.05;
        (0..(w as usize)).map(|i| (i as f64, mid + 0.6 * (i as f64 * 0.08 + phase).sin())).collect()
    } else {
        Vec::new()
    };

    let trace_color = if state == PlayerState::Playing { p.accent } else { p.text };
    let (line, muted, bg) = (p.line, p.muted, p.bg);
    frame.render_widget(
        Canvas::default()
            .marker(Marker::Braille)
            .background_color(bg)
            .x_bounds([0.0, w])
            .y_bounds([0.0, h])
            .paint(move |ctx| {
                ctx.draw(&Points { coords: &bars, color: line });
                ctx.draw(&Points { coords: &caps, color: muted });
                ctx.draw(&Points { coords: &idle, color: line });
                for (segs, color) in &glow {
                    ctx.layer();
                    for &(x1, y1, x2, y2) in segs {
                        ctx.draw(&CanvasLine { x1, y1, x2, y2, color: *color });
                    }
                }
                ctx.layer();
                for &(x1, y1, x2, y2) in &live {
                    ctx.draw(&CanvasLine { x1, y1, x2, y2, color: trace_color });
                }
            }),
        inner,
    );

    // Read-outs, top-left. Drawn after the canvas, on the background colour.
    let t_txt = app.lang.t();
    let readout = match (real, app.current_track()) {
        (Some(_), Some(tr)) => {
            let level = app.history.last().copied().unwrap_or(0) as f64;
            format!(
                "{} · {:.0} dBFS · {} / {}",
                t_txt.fft_label,
                level * 0.6 - 60.0,
                format_time(app.player.position()),
                format_time(tr.duration())
            )
        }
        (None, Some(tr)) if state == PlayerState::Playing || app.net.loading_stream() => {
            format!("{} · {} / {}", t_txt.buffering, format_time(app.player.position()), format_time(tr.duration()))
        }
        _ => t_txt.no_signal.to_string(),
    };
    let buf = frame.buffer_mut();
    set_str(buf, inner, 0, 0, &readout, Style::default().fg(p.muted).bg(p.bg));
}

/// Greedy word wrap by display width; over-long words are split.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(4);
    let mut out = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while text_width(&word) > width {
            let head: String = word.chars().take(width).collect();
            word = word.chars().skip(width).collect();
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            out.push(head);
        }
        let need = if cur.is_empty() { text_width(&word) } else { text_width(&cur) + 1 + text_width(&word) };
        if need > width && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(&word);
    }
    if !cur.is_empty() || out.is_empty() {
        out.push(cur);
    }
    out
}

/// Centred lyrics over the visualiser. The current line sits at the vertical
/// centre; the text glides up as `scroll` eases towards the next line.
/// Distance from the current line sets the tone:
/// bright now, soft grey next, fading into the background behind.
fn lyrics(frame: &mut Frame, inner: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let Some(view) = app.lyrics.as_ref().filter(|v| app.current_track().is_some_and(|tr| tr.id == v.track_id)) else {
        return;
    };
    let buf = frame.buffer_mut();
    let corner = |buf: &mut ratatui::buffer::Buffer, s: &str| {
        let x = inner.width.saturating_sub(text_width(s) as u16 + 1);
        set_str(buf, inner, x, 0, &format!(" {s}"), Style::default().fg(p.muted).bg(p.bg));
    };
    let Some(lyr) = view.lyrics.as_ref() else {
        corner(buf, t.lyrics_none);
        return;
    };
    if !app.show_lyrics {
        corner(buf, t.lyrics_hidden);
        return;
    }
    if app.lyrics_manual() {
        corner(buf, t.lyrics_manual);
    } else if !lyr.synced {
        corner(buf, t.lyrics_plain);
    }

    // Keep a readable column width, but centre every wrapped row in the
    // entire visualiser rather than anchoring the column to its right edge.
    let col_w = if inner.width >= 52 {
        (inner.width * 64 / 100).max(36).min(inner.width)
    } else {
        inner.width
    };
    let text_w = usize::from(col_w.saturating_sub(4));

    // Wrapped rows and the first row of each lyric line.
    let mut rows: Vec<(usize, String)> = Vec::new();
    let mut starts = Vec::with_capacity(lyr.lines.len() + 1);
    for (i, line) in lyr.lines.iter().enumerate() {
        starts.push(rows.len() as f64);
        let text = if line.text.trim().is_empty() { "♪".to_string() } else { line.text.clone() };
        for r in wrap(&text, text_w) {
            rows.push((i, r));
        }
    }
    starts.push(rows.len() as f64);
    // Fractional line index → fractional row, with a virtual line at -1.
    let row_of = |s: f64| -> f64 {
        if s < 0.0 {
            return s * 2.0;
        }
        let i = (s.floor() as usize).min(starts.len() - 2);
        let f = s - i as f64;
        starts[i] + f * (starts[i + 1] - starts[i])
    };
    let anchor = f64::from(inner.height) / 2.0;
    let offset = row_of(view.scroll);
    let current = app.lyric_line().map(|c| c.round() as i64);

    for (r, (li, text)) in rows.iter().enumerate() {
        let y = (anchor + r as f64 - offset).round();
        if y < 1.0 || y >= f64::from(inner.height) {
            continue; // row 0 belongs to the read-outs
        }
        let y = y as u16;
        let d = current.map_or(1, |c| *li as i64 - c);
        let (mut fg, bold) = match d {
            0 => (p.accent, true),
            1..=2 => (p.text, false),
            d if d > 2 => (p.muted, false),
            -1 => (p.muted, false),
            _ => (p.line, false),
        };
        // Soft fade into the frame at the top and bottom edges.
        let edge = y.min(inner.height - 1 - y);
        if edge <= 1 && d != 0 {
            fg = fade(fg, p);
        }
        let mut style = Style::default().fg(fg).bg(p.bg);
        if bold {
            style = style.add_modifier(Modifier::BOLD);
        }
        // Centre the text itself; the current-line marker must not shift it.
        let x = inner.width.saturating_sub(text_width(text) as u16) / 2;
        let marker = if d == 0 && r as f64 == starts[*li] { "› " } else { "  " };
        let marker_style = Style::default().fg(p.muted).bg(p.bg);
        if x >= 2 {
            set_str(buf, inner, x - 2, y, marker, marker_style);
        }
        set_str(buf, inner, x, y, text, style);
        set_str(buf, inner, x + text_width(text) as u16, y, " ", Style::default().bg(p.bg));
    }
}

fn fade(c: Color, p: &Palette) -> Color {
    if c == p.accent || c == p.text { p.muted } else { p.line }
}

#[cfg(test)]
mod tests {
    use super::wrap;

    #[test]
    fn wraps_by_words_and_splits_long_words() {
        assert_eq!(wrap("один два три", 8), ["один два", "три"]);
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 10), [""]);
    }
}
