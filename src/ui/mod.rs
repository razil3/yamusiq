//! Rendering — a calm monochrome terminal printout.
//!
//! Design rules: graphite field, light-grey copy, pure white reserved for
//! whatever is live; only the two primary screens (visualiser, track table)
//! are framed, everything else is an open section (title rule, content, key
//! hints); thin line-work instead of solid blocks; sentence-case labels.

mod parts;
mod visualizer;

use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph},
    Frame,
};

use crate::app::{App, Entry, Focus, Palette, Section, PALETTE};
use crate::i18n::{fill, Lang};
use crate::model::format_time;
use crate::player::PlayerState;

use parts::quiet;

pub fn draw(frame: &mut Frame, app: &App) {
    let p = &PALETTE;
    let area = frame.area();
    frame.render_widget(Block::default().style(Style::default().bg(p.bg).fg(p.text)), area);

    let bottom_h = match area.height {
        30.. => 8,
        22..=29 => 4,
        _ => 0,
    };
    let gap = u16::from(area.height >= 24);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Length(gap),
            Constraint::Min(8),
            Constraint::Length(bottom_h),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);
    parts::header(frame, rows[0], app, p);
    draw_main(frame, rows[2], app, p);
    if bottom_h > 0 {
        draw_bottom(frame, rows[3], app, p);
    }
    draw_status(frame, rows[4], app, p);
    draw_footer(frame, rows[5], app, p);

    if app.help {
        draw_help(frame, area, app, p);
    }
}

// ── primitives ───────────────────────────────────────────────────────────────

fn text_width(s: &str) -> usize {
    Span::raw(s).width()
}

fn set(buf: &mut Buffer, area: Rect, x: u16, y: u16, sym: &str, style: Style) {
    if x < area.width && y < area.height {
        buf[(area.x + x, area.y + y)].set_symbol(sym).set_style(style);
    }
}

/// Writes `s` at (x, y) inside `area`, clipped to the area.
fn set_str(buf: &mut Buffer, area: Rect, x: u16, y: u16, s: &str, style: Style) {
    if y >= area.height || x >= area.width {
        return;
    }
    let max = usize::from(area.width - x);
    buf.set_stringn(area.x + x, area.y + y, s, max, style);
}

/// "key desc · key desc" with keys picked out in light grey.
fn hint_line(hint: &str, p: &Palette) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    for (i, part) in hint.split(" · ").enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(p.muted)));
        }
        let (key, desc) = part.split_once(' ').unwrap_or((part, ""));
        spans.push(Span::styled(key.to_string(), Style::default().fg(p.text).add_modifier(Modifier::BOLD)));
        if !desc.is_empty() {
            spans.push(Span::styled(format!(" {desc}"), Style::default().fg(p.muted)));
        }
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

/// Rounded frame with the panel's key hints in the bottom border.
fn frame_block(hint: &str, focused: bool, p: &Palette) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused { p.muted } else { p.line }))
        .style(Style::default().bg(p.bg))
        .title_bottom(hint_line(hint, p).right_aligned())
}

/// Open section: "● Title ──────" / content / right-aligned hints.
fn hud_panel(frame: &mut Frame, area: Rect, label: &str, hint: &str, focused: bool, p: &Palette) -> Rect {
    panel(frame, area, label, hint, focused, false, p)
}

/// Framed variant — used for the primary "screens".
fn framed_panel(frame: &mut Frame, area: Rect, label: &str, hint: &str, focused: bool, p: &Palette) -> Rect {
    panel(frame, area, label, hint, focused, true, p)
}

fn panel(frame: &mut Frame, area: Rect, label: &str, hint: &str, focused: bool, framed: bool, p: &Palette) -> Rect {
    let label = &quiet(label);
    let title_style = if focused {
        Style::default().fg(p.accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(p.text)
    };
    if framed {
        let block = frame_block(hint, focused, p).title(Line::from(vec![
            Span::raw(" "),
            Span::styled(label.to_string(), title_style),
            Span::raw(" "),
        ]));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        return Rect { x: inner.x + 1, width: inner.width.saturating_sub(2), ..inner };
    }
    if area.height < 3 || area.width < 4 {
        return area;
    }
    let marker = if focused { "● " } else { "" };
    let used = text_width(marker) + text_width(label) + 1;
    let rule = "─".repeat(usize::from(area.width).saturating_sub(used));
    let head = Line::from(vec![
        Span::styled(marker, Style::default().fg(p.accent)),
        Span::styled(label.to_string(), title_style),
        Span::raw(" "),
        Span::styled(rule, Style::default().fg(p.line)),
    ]);
    frame.render_widget(Paragraph::new(head), Rect { height: 1, ..area });
    let foot = Rect { y: area.y + area.height - 1, height: 1, ..area };
    frame.render_widget(Paragraph::new(hint_line(hint, p)).alignment(Alignment::Right), foot);
    Rect { y: area.y + 1, height: area.height - 2, ..area }
}

// ── main grid ────────────────────────────────────────────────────────────────

fn draw_main(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let b = (area.width / 5).clamp(18, 28);
    let c = (area.width * 27 / 100).clamp(22, 38);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(b), Constraint::Length(c)])
        .spacing(3)
        .split(area);

    // Column A: the visualiser is the hero; the track table sits beneath.
    let a = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(if area.height >= 26 { 56 } else { 48 }), Constraint::Min(5)])
        .spacing(1)
        .split(cols[0]);
    visualizer::draw(frame, a[0], app, p);
    draw_list(frame, a[1], app, p);

    // Column B: frequency bands over three meters.
    let bcol = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(6), Constraint::Length(3), Constraint::Length(3), Constraint::Length(3)])
        .spacing(1)
        .split(cols[1]);
    let t = app.lang.t();
    let inner = hud_panel(frame, bcol[0], t.lbl_bands, t.hint_bands, false, p);
    parts::bands(frame, inner, app, p);
    draw_meters(frame, [bcol[1], bcol[2], bcol[3]], app, p);

    // Column C: library, signal trace, track card.
    let ccol = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(Section::ALL.len() as u16 + 2), Constraint::Length(6), Constraint::Min(6)])
        .spacing(1)
        .split(cols[2]);
    draw_library(frame, ccol[0], app, p);
    let inner = hud_panel(frame, ccol[1], t.lbl_signal, t.hint_signal, false, p);
    parts::signal(frame, inner, app, p);
    let inner = hud_panel(frame, ccol[2], t.lbl_info, t.hint_info, false, p);
    parts::info(frame, inner, app, p);
}

fn draw_list(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let focused = app.focus == Focus::List;
    let playlists = matches!(app.entries.first(), Some(Entry::Playlist(_)));
    let hint = if playlists {
        t.hint_list_playlists
    } else if matches!(app.list, crate::app::ListKind::Artist(_) | crate::app::ListKind::ArtistAll(_)) {
        t.hint_list_artist
    } else {
        t.hint_list
    };
    let in_playlist = matches!(app.list, crate::app::ListKind::Playlist(_));
    let inner = framed_panel(frame, area, &app.list.title(app.lang), if in_playlist { "" } else { hint }, focused, p);
    if in_playlist && area.height > 0 {
        // Back navigation has priority on narrow panels. Other hints use only
        // the remaining space, so they cannot overwrite the left-hand hint.
        let width = area.width.saturating_sub(2);
        let back_width = (text_width(t.hint_back_playlists) as u16 + 2).min(width);
        let back = Rect::new(area.x + 1, area.y + area.height - 1, back_width, 1);
        frame.render_widget(Paragraph::new(hint_line(t.hint_back_playlists, p)), back);
        let rest = Rect::new(back.x + back_width, back.y, width - back_width, 1);
        frame.render_widget(Paragraph::new(hint_line(hint, p)).alignment(Alignment::Right), rest);
    }

    if app.entries.is_empty() {
        let pad = usize::from(inner.height.saturating_sub(2) / 2);
        let mut lines = vec![Line::from(""); pad];
        for s in t.empty_list {
            lines.push(Line::from(Span::styled(s, Style::default().fg(p.muted))));
        }
        frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
        return;
    }
    if inner.height < 2 {
        return;
    }

    // Quiet column header.
    let left = format!("   №  {}", if playlists { t.tbl_playlist } else { t.tbl_track });
    let right = if playlists { String::new() } else { format!("♥  {:>5}", t.tbl_time) };
    let pad = usize::from(inner.width).saturating_sub(text_width(&left) + text_width(&right));
    let head = Line::from(Span::styled(format!("{left}{}{right}", " ".repeat(pad)), Style::default().fg(p.muted)));
    frame.render_widget(Paragraph::new(head), Rect { height: 1, ..inner });

    let width = usize::from(inner.width.saturating_sub(2));
    let playing_id = app.current_track().map(|tr| tr.id.as_str());
    // The marker follows the player, not the queue: a paused track keeps its row
    // marked but swaps ▶ for ⏸, so the list says what you are hearing right now.
    let live = app.player.state();
    let items: Vec<ListItem> = app
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| match e {
            Entry::Track(tr) => {
                let playing = Some(tr.id.as_str()) == playing_id;
                let (num, num_color) = match (playing, live) {
                    (true, PlayerState::Playing) => (" ▶  ".to_string(), p.accent),
                    (true, PlayerState::Paused) => (" ⏸  ".to_string(), p.accent),
                    (true, PlayerState::Stopped) => (format!("{:>2}  ", i + 1), p.muted),
                    (false, _) => (format!("{:>2}  ", i + 1), p.muted),
                };
                let main = format!("{} — {}", tr.artist, tr.title);
                // Saved-to-library mark right after the title; progress while saving.
                let saved = match app.library.progress(&tr.id) {
                    Some(f) => format!(" ⇣ {:.0}%", f * 100.0),
                    None if app.is_saved(tr) => " ⇣".to_string(),
                    None => String::new(),
                };
                let saving = app.library.is_downloading(&tr.id);
                let flags = format!("{}  ", if tr.liked { "♥" } else { " " });
                let dur = format_time(tr.duration());
                let used = [num.as_str(), &main, &saved, &flags, &dur].iter().map(|s| text_width(s)).sum::<usize>();
                let main_style = if playing { Style::default().fg(p.accent).add_modifier(Modifier::BOLD) } else { Style::default().fg(p.text) };
                ListItem::new(Line::from(vec![
                    Span::styled(num, Style::default().fg(num_color)),
                    Span::styled(main, main_style),
                    Span::styled(saved, Style::default().fg(if saving { p.muted } else { p.text })),
                    Span::raw(" ".repeat(width.saturating_sub(used))),
                    Span::styled(flags, Style::default().fg(p.text)),
                    Span::styled(dur, Style::default().fg(p.muted)),
                ]))
            }
            Entry::Playlist(pl) => ListItem::new(Line::from(vec![
                Span::styled(format!("{:>2}  ", i + 1), Style::default().fg(p.muted)),
                Span::styled(pl.title.clone(), Style::default().fg(p.text)),
                Span::styled(format!("  ·  {}", app.lang.tracks(pl.track_count)), Style::default().fg(p.muted)),
            ])),
        })
        .collect();
    let highlight = if focused { Style::default().bg(p.fill).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
    let mut state = ListState::default().with_selected(Some(app.selected));
    let body = Rect { y: inner.y + 1, height: inner.height - 1, ..inner };
    frame.render_stateful_widget(List::new(items).highlight_style(highlight).highlight_symbol("› "), body, &mut state);
}

fn draw_meters(frame: &mut Frame, areas: [Rect; 3], app: &App, p: &Palette) {
    let t = app.lang.t();
    let tracks: Vec<_> = app.entries.iter().filter_map(|e| if let Entry::Track(tr) = e { Some(tr) } else { None }).collect();
    let pct = |k: usize| if tracks.is_empty() { 0 } else { k * 100 / tracks.len() };
    let liked = pct(tracks.iter().filter(|tr| tr.liked).count());
    let cached = pct(tracks.iter().filter(|tr| app.is_saved(tr)).count());
    let (qpos, qlen) = (app.queue_pos.map_or(0, |i| i + 1), app.queue.len());
    let meters = [
        (fill(t.lbl_queue, &[&format!("{qpos}/{qlen}")]), t.hint_queue, if qlen == 0 { 0.0 } else { qpos as f64 / qlen as f64 }),
        (fill(t.lbl_liked, &[&liked.to_string()]), t.hint_liked, liked as f64 / 100.0),
        (fill(t.lbl_cached, &[&cached.to_string()]), t.hint_cached, cached as f64 / 100.0),
    ];
    for (area, (label, hint, ratio)) in areas.into_iter().zip(meters) {
        let inner = hud_panel(frame, area, &label, hint, false, p);
        if inner.height > 0 {
            parts::thin_bar(frame.buffer_mut(), Rect { height: 1, ..inner }, ratio, p);
        }
    }
}

fn draw_library(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let focused = app.focus == Focus::Nav;
    let inner = hud_panel(frame, area, t.lbl_library, t.hint_library, focused, p);
    let items: Vec<ListItem> = Section::ALL
        .iter()
        .map(|s| {
            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", s.icon()), Style::default().fg(p.muted)),
                Span::styled(s.label(app.lang), Style::default().fg(p.text)),
            ]))
        })
        .collect();
    let highlight = if focused { Style::default().bg(p.fill).add_modifier(Modifier::BOLD) } else { Style::default().add_modifier(Modifier::BOLD) };
    let mut state = ListState::default().with_selected(Some(app.section));
    frame.render_stateful_widget(List::new(items).highlight_style(highlight).highlight_symbol("› "), inner, &mut state);
}

// ── bottom: timeline and volume ──────────────────────────────────────────────

fn draw_bottom(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .spacing(1)
        .split(area);

    let (pos, dur) = match app.current_track() {
        Some(tr) => (app.player.position(), tr.duration()),
        None => (Default::default(), Default::default()),
    };
    let ratio = if dur.is_zero() { 0.0 } else { pos.as_secs_f64() / dur.as_secs_f64() };
    let label = format!("{}   {} / {}", t.lbl_progress, format_time(pos), format_time(dur));
    let inner = hud_panel(frame, rows[0], &label, t.hint_progress, false, p);
    let ticks: Vec<String> = (0..10).map(|k| format_time(dur.mul_f64(f64::from(k) / 10.0))).collect();
    parts::timeline(frame.buffer_mut(), inner, ratio, &ticks, p);

    if rows[1].height >= 3 {
        let label = fill(t.lbl_volume, &[&app.volume.to_string()]);
        let inner = hud_panel(frame, rows[1], &label, t.hint_volume, false, p);
        parts::thin_bar(frame.buffer_mut(), Rect { height: 1, ..inner }, f64::from(app.volume) / 100.0, p);
    }
}

// ── status / footer / help ───────────────────────────────────────────────────

fn draw_status(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    match &app.query {
        Some(q) => {
            let cursor = if (app.frame / 5).is_multiple_of(2) { "▏" } else { " " };
            let line = Line::from(vec![
                Span::styled(format!(" {} ", t.search_prompt), Style::default().fg(p.accent).add_modifier(Modifier::BOLD)),
                Span::styled(q.clone(), Style::default().fg(p.accent)),
                Span::styled(cursor, Style::default().fg(p.accent)),
            ]);
            frame.render_widget(Paragraph::new(line), area);
            frame.render_widget(Paragraph::new(hint_line(t.hint_search, p)).alignment(Alignment::Right), area);
        }
        None => {
            let line = Line::from(vec![
                Span::styled(" › ", Style::default().fg(p.muted)),
                Span::styled(app.status.text(app.lang), Style::default().fg(p.text)),
            ]);
            frame.render_widget(Paragraph::new(line), area);
        }
    }
}

fn draw_footer(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let hints = hint_line(t.hint_footer, p);
    let hints_w = hints.width() as u16;
    frame.render_widget(Paragraph::new(hints), area);

    let lang = if app.lang == Lang::Ru { "RU" } else { "EN" };
    let mut right_spans = Vec::new();
    if app.auto_download {
        right_spans.push(Span::styled(format!("{} · ", t.auto_label), Style::default().fg(p.text)));
    }
    right_spans.push(Span::styled(format!("{lang} · v{} ", env!("CARGO_PKG_VERSION")), Style::default().fg(p.muted)));
    let right = Line::from(right_spans);
    let right_w = right.width() as u16;
    frame.render_widget(Paragraph::new(right).alignment(Alignment::Right), area);

    // Sync progress lives in the gap between the hints and the language.
    let Some(sync) = &app.sync else { return };
    let gap_x = area.x + hints_w + 2;
    let gap_end = (area.x + area.width).saturating_sub(right_w + 2);
    if gap_end <= gap_x + 8 {
        return;
    }
    let gap = Rect { x: gap_x, width: gap_end - gap_x, ..area };
    let frac = sync.fraction(&app.library);
    let count = format!("⇣ {}/{} ", sync.finished_count(), sync.total);
    let pct = format!(" {:>3.0}%", frac * 100.0);
    let bar_w = usize::from(gap.width).saturating_sub(text_width(&count) + text_width(&pct)).min(32);
    let filled = (bar_w as f64 * frac).round() as usize;
    let mut spans = vec![
        Span::styled(count, Style::default().fg(p.text)),
        Span::styled("━".repeat(filled), Style::default().fg(p.accent)),
        Span::styled("─".repeat(bar_w - filled), Style::default().fg(p.line)),
        Span::styled(pct, Style::default().fg(p.text).add_modifier(Modifier::BOLD)),
    ];
    if bar_w < 6 {
        spans = vec![Span::styled(format!("⇣ {:.0}%", frac * 100.0), Style::default().fg(p.text))];
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).alignment(Alignment::Center), gap);
}

fn draw_help(frame: &mut Frame, area: Rect, app: &App, p: &Palette) {
    let t = app.lang.t();
    let w = area.width.min(64);
    let h = area.height.min(t.help.len() as u16 + 4);
    let popup = Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h };
    frame.render_widget(Clear, popup);
    let inner = framed_panel(frame, popup, t.lbl_help, t.hint_help, true, p);
    let key_w = t.help.iter().map(|(k, _)| text_width(k)).max().unwrap_or(0);
    let mut lines = vec![Line::from("")];
    lines.extend(t.help.iter().map(|(k, d)| {
        if k.is_empty() {
            return Line::from(Span::styled(format!(" {d}"), Style::default().fg(p.muted).add_modifier(Modifier::ITALIC)));
        }
        Line::from(vec![
            Span::styled(format!(" {k:<key_w$}  "), Style::default().fg(p.accent).add_modifier(Modifier::BOLD)),
            Span::styled(d.to_string(), Style::default().fg(p.text)),
        ])
    }));
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};

    use super::draw;
    use crate::app::{App, PALETTE};
    use crate::i18n::Lang;
    use crate::player::SimPlayer;
    use crate::testing::FakeService;

    fn app() -> App {
        App::new(Box::new(FakeService::default()), Box::new(SimPlayer::default()), Lang::Ru)
    }

    fn term(app: &App, w: u16, h: u16) -> Terminal<TestBackend> {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term
    }

    fn render(app: &App, w: u16, h: u16) -> String {
        term(app, w, h).backend().buffer().content().iter().map(|c| c.symbol()).collect()
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    /// Presses a key that starts a catalogue request, then settles: the worker
    /// answers off-thread and the UI only collects it on a tick.
    fn press_wait(app: &mut App, code: KeyCode) {
        press(app, code);
        settle(app);
    }

    fn settle(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.net.is_busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
            app.tick();
        }
        app.tick();
    }

    #[test]
    fn loading_stream_renders_buffering_and_keeps_keyboard_working() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('w'));
        gate.hold();
        press(&mut a, KeyCode::Enter);
        assert!(a.net.loading_stream());
        for _ in 0..5 {
            a.tick();
            assert!(render(&a, 130, 40).contains("буферизация"));
        }
        press(&mut a, KeyCode::Char('-'));
        assert_eq!(a.volume, 65);
        press(&mut a, KeyCode::Char('i'));
        assert!(render(&a, 130, 40).contains("buffering"));
        gate.release();
        settle(&mut a);
        assert!(!a.net.loading_stream());
    }

    /// Play "Static Bloom" (synced lyrics) and jump to `secs`.
    fn playing_at(secs: u64) -> App {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        a.player.seek(Duration::from_secs(secs));
        for _ in 0..40 {
            a.tick();
        }
        a
    }

    #[test]
    fn playlist_back_hint_is_on_the_bottom_left_in_both_languages() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('p'));
        press_wait(&mut a, KeyCode::Enter);
        for lang in [Lang::Ru, Lang::En] {
            a.lang = lang;
            for width in [24, 40, 80] {
                let mut term = Terminal::new(TestBackend::new(width + 6, 18)).unwrap();
                let area = ratatui::layout::Rect::new(3, 2, width, 14);
                term.draw(|f| super::draw_list(f, area, &a, &PALETTE)).unwrap();
                let buf = term.backend().buffer();
                let text = lang.t().hint_back_playlists;
                let x = area.x + 2;
                let y = area.y + area.height - 1;
                let actual: String = (x..x + super::text_width(text) as u16).map(|x| buf[(x, y)].symbol()).collect();
                assert_eq!(actual, text, "back hint at width {width}");
            }
            a.list = crate::app::ListKind::Likes;
            assert!(!render(&a, 130, 40).contains(lang.t().hint_back_playlists));
            a.list = crate::app::ListKind::Playlist("test".into());
        }
    }

    #[test]
    fn loop_modes_have_distinct_visible_labels() {
        let mut a = playing_at(10);
        for lang in [Lang::Ru, Lang::En] {
            a.lang = lang;
            for mode in [crate::app::PlaybackMode::All, crate::app::PlaybackMode::Repeat, crate::app::PlaybackMode::PlaylistLoop] {
                a.mode = mode;
                assert!(render(&a, 180, 45).contains(mode.label(lang)), "missing mode label: {}", mode.label(lang));
            }
        }
    }

    #[test]
    fn renders_every_section_in_russian_and_english() {
        let mut a = app();
        let idle = render(&a, 130, 40);
        for needle in [
            "Визуализатор", "Частоты", "16 кГц", "Уровень сигнала", "Данные трека", "Библиотека", "Прогресс трека",
            "Громкость: 70%", "нет сигнала", "Ничего не играет", "y a m u s i q", "▄█████▄", "справка", "╭",
        ] {
            assert!(idle.contains(needle), "missing {needle:?}");
        }
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let screen = render(&a, 130, 40);
        for needle in ["Играет", "Static Bloom", "Моя волна", "ИСПОЛНИТЕЛЬ — ТРЕК", "буферизация", "Альбом"] {
            assert!(screen.contains(needle), "missing {needle:?}");
        }
        press(&mut a, KeyCode::Char('?'));
        assert!(render(&a, 130, 40).contains("перемотка на 5 секунд"));
        press(&mut a, KeyCode::Esc);
        press(&mut a, KeyCode::Char('i'));
        let en = render(&a, 130, 40);
        assert!(en.contains("Library") && en.contains("Playing") && en.contains("Visualizer"));
    }

    #[test]
    fn lyrics_overlay_shows_current_line_on_background() {
        // 0:38 → "Bloom, bloom — the static in my head" (from 0:36).
        let a = playing_at(38);
        let t = term(&a, 130, 40);
        let buf = t.backend().buffer();
        let screen: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(screen.contains("Bloom, bloom — the static"), "current line visible");
        assert!(screen.contains("and something in the noise"), "previous lines stay above");
        // The current line is bright and sits on the background colour.
        let (w, h) = (buf.area.width, buf.area.height);
        let row = (0..h)
            .find(|&y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>().contains("Bloom, bloom — the static"))
            .unwrap();
        let x = (0..w).find(|&x| buf[(x, row)].symbol() == "B").unwrap();
        assert_eq!(buf[(x, row)].fg, PALETTE.accent);
        assert_eq!(buf[(x, row)].bg, PALETTE.bg);
    }

    #[test]
    fn lyrics_stay_centred_at_different_visualizer_sizes() {
        let mut a = playing_at(0);
        let view = a.lyrics.as_mut().unwrap();
        view.lyrics = Some(crate::model::Lyrics::parse(
            "[00:00.00]Short\n[00:10.00]A longer line\n[00:20.00]Тихий свет",
        ));
        view.scroll = 0.0;
        for width in [24, 51, 80, 121] {
            for height in [12, 21] {
                let area = ratatui::layout::Rect::new(5, 3, width, height);
                let mut term = Terminal::new(TestBackend::new(width + 10, height + 6)).unwrap();
                term.draw(|f| super::visualizer::draw(f, area, &a, &PALETTE)).unwrap();
                let buf = term.backend().buffer();
                let inner = ratatui::layout::Rect::new(area.x + 2, area.y + 1, width - 4, height - 2);
                for (i, text) in ["Short", "A longer line", "Тихий свет"].iter().enumerate() {
                    let y = inner.y + (f64::from(inner.height) / 2.0).round() as u16 + i as u16;
                    let x = inner.x + (inner.width - super::text_width(text) as u16) / 2;
                    let actual: String = (x..x + super::text_width(text) as u16).map(|x| buf[(x, y)].symbol()).collect();
                    assert_eq!(actual, *text, "{width}×{height}: row {i} is centred");
                }
            }
        }
    }

    #[test]
    fn lyrics_can_be_hidden_and_absent_lyrics_are_labelled() {
        let mut a = playing_at(38);
        press(&mut a, KeyCode::Char('y'));
        let s = render(&a, 130, 40);
        assert!(!s.contains("Bloom, bloom — the static"));
        assert!(s.contains("текст скрыт"));

        let mut b = app();
        press_wait(&mut b, KeyCode::Char('w'));
        press(&mut b, KeyCode::Char('j')); // "Low Tide": no lyrics
        press_wait(&mut b, KeyCode::Enter);
        assert!(render(&b, 130, 40).contains("без текста"));
    }

    #[test]
    fn real_waveform_replaces_the_model() {
        let mut a = playing_at(10);
        a.wave = Some((0..640).map(|i| (i as f32 / 640.0 * std::f32::consts::TAU * 3.0).sin() * 0.5).collect());
        a.wave_history.push_front(vec![0.0; 640]);
        let t = term(&a, 130, 40);
        let buf = t.backend().buffer();
        let screen: String = buf.content().iter().map(|c| c.symbol()).collect();
        assert!(screen.contains("спектр ·") && screen.contains("dBFS"), "real-audio readout");
        assert!(!screen.contains("буферизация"), "real audio replaces the buffering note");
        let braille_bright = buf.content().iter().filter(|c| c.fg == PALETTE.accent && ('\u{2800}'..='\u{28FF}').contains(&c.symbol().chars().next().unwrap_or(' '))).count();
        assert!(braille_bright > 20, "the live trace is drawn in the accent colour ({braille_bright} cells)");
    }

    #[test]
    fn card_says_when_lossless_is_unavailable() {
        let mut a = playing_at(10);
        a.quality = Some("MP3 320".into());
        a.lossless_missing = true;
        let s = render(&a, 130, 40);
        assert!(s.contains("MP3 320"));
        assert!(s.contains("lossless недоступен"));
        // Narrow layouts still show it (without the indent).
        assert!(render(&a, 96, 30).contains("lossless недоступен"));
        a.lossless_missing = false;
        assert!(!render(&a, 130, 40).contains("lossless недоступен"));
    }

    #[test]
    fn saved_tracks_show_the_mark_next_to_the_title() {
        let mut a = playing_at(5);
        // Mark the second wave track as saved by the backend flag.
        if let Some(crate::app::Entry::Track(t)) = a.entries.get_mut(1) {
            t.cached = true;
        }
        let s = render(&a, 130, 40);
        assert!(s.contains("Neon Harbor — Low Tide ⇣"), "mark sits right after the title");
        assert!(!s.contains("Static Bloom ⇣"), "unsaved tracks have no mark");
    }

    #[test]
    fn playing_marker_switches_between_play_and_pause() {
        let mut a = playing_at(5);
        let playing = render(&a, 130, 40);
        assert!(playing.contains("▶"), "a playing track is marked ▶");
        assert!(!playing.contains("⏸"), "no pause glyph while playing");

        press(&mut a, KeyCode::Char(' '));
        assert_eq!(a.player.state(), crate::player::PlayerState::Paused);
        let paused = render(&a, 130, 40);
        assert!(paused.contains("⏸"), "a paused track is marked ⏸");
        assert!(!paused.contains("▶"), "the play glyph goes away while paused");

        press(&mut a, KeyCode::Char(' '));
        let resumed = render(&a, 130, 40);
        assert!(resumed.contains("▶"), "resuming brings ▶ back");
        assert!(!resumed.contains("⏸"));
    }

    #[test]
    fn footer_shows_sync_progress_and_auto_mode() {
        let mut a = app();
        let liked = a.service.likes().unwrap();
        let n = liked.len();
        a.sync = Some(crate::sync::Sync::new(liked, &a.library));
        a.auto_download = true;
        let s = render(&a, 160, 40);
        let footer: String = s.chars().rev().take(160).collect::<Vec<_>>().into_iter().rev().collect();
        assert!(footer.contains(&format!("⇣ 0/{n}")), "count in the footer: {footer}");
        assert!(footer.contains("0%"));
        assert!(footer.contains("⇣ авто"));
        // The bar sits between the hints and the language label.
        let (bar, lang, quit) = (footer.find("⇣ 0/").unwrap(), footer.find("RU").unwrap(), footer.find("выход").unwrap());
        assert!(quit < bar && bar < lang);
    }

    #[test]
    fn plain_russian_lyrics_follow_progress() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        for _ in 0..7 {
            press(&mut a, KeyCode::Char('j')); // "Электричка": plain text, no timestamps
        }
        press_wait(&mut a, KeyCode::Enter);
        assert_eq!(a.current_track().unwrap().title, "Электричка");
        for _ in 0..40 {
            a.tick();
        }
        let s = render(&a, 130, 40);
        assert!(s.contains("текст без таймкодов"));
        assert!(s.contains("Электричка уходит в шесть"));
    }

    /// Debug helper: `cargo test dump_screen -- --ignored --nocapture`.
    /// `YAMUSIQ_DUMP_AT=38` picks the playback position in seconds.
    #[test]
    #[ignore]
    fn dump_screen() {
        let at = std::env::var("YAMUSIQ_DUMP_AT").ok().and_then(|s| s.parse().ok()).unwrap_or(37);
        let mut a = playing_at(at);
        // Let real time pass so the analyser, peaks and signal fill in.
        for _ in 0..24 {
            std::thread::sleep(Duration::from_millis(95));
            a.tick();
        }
        for (w, h) in [(130u16, 40u16), (96, 28)] {
            let t = term(&a, w, h);
            let buf = t.backend().buffer();
            for y in 0..h {
                println!("{}", (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>());
            }
            println!();
        }
    }

    #[test]
    fn survives_tiny_terminals() {
        let mut a = playing_at(40);
        for (w, h) in [(20, 6), (1, 1), (40, 12), (60, 20), (80, 24), (200, 60)] {
            render(&a, w, h);
            press(&mut a, KeyCode::Char('?'));
            render(&a, w, h);
            press(&mut a, KeyCode::Esc);
        }
    }
}
