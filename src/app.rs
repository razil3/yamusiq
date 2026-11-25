use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Color;

use crate::i18n::{fill, latin_key, Lang};
use crate::library::{Finished, Library};
use crate::sync::Sync;
use crate::model::{Lyrics, Playlist, Quality, Track};
use crate::player::{Player, PlayerState};
use crate::service::MusicService;

/// The one palette: graphite field, light-grey copy, thin dim line-work and
/// warm white reserved for whatever is live.
pub struct Palette {
    pub bg: Color,
    pub line: Color,
    pub accent: Color,
    pub fill: Color,
    pub text: Color,
    pub muted: Color,
}

pub const PALETTE: Palette = Palette {
    bg: Color::Rgb(17, 17, 19),
    line: Color::Rgb(54, 54, 60),
    accent: Color::Rgb(240, 238, 232),
    fill: Color::Rgb(36, 36, 41),
    text: Color::Rgb(172, 172, 168),
    muted: Color::Rgb(104, 104, 110),
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlaybackMode {
    All,
    Repeat,
    Random,
}

impl PlaybackMode {
    pub fn label(self, lang: Lang) -> &'static str {
        lang.t().modes[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Home,
    Wave,
    Playlists,
    Likes,
    Cached,
}

impl Section {
    pub const ALL: [Section; 5] = [Self::Home, Self::Wave, Self::Playlists, Self::Likes, Self::Cached];

    pub fn icon(self) -> &'static str {
        match self {
            Self::Home => "⌂",
            Self::Wave => "≋",
            Self::Playlists => "▤",
            Self::Likes => "♥",
            Self::Cached => "⇣",
        }
    }

    pub fn label(self, lang: Lang) -> &'static str {
        lang.t().sections[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Focus {
    Nav,
    List,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    Track(Track),
    Playlist(Playlist),
}

/// What the centre list currently shows; translated at render time.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ListKind {
    Overview,
    Wave,
    Likes,
    Cached,
    Playlists,
    Search(String),
    Playlist(String),
    Artist(String),
    ArtistAll(String),
}

impl ListKind {
    pub fn title(&self, lang: Lang) -> String {
        let t = lang.t();
        match self {
            Self::Overview => t.list_overview.into(),
            Self::Wave => t.list_wave.into(),
            Self::Likes => t.list_likes.into(),
            Self::Cached => t.list_cached.into(),
            Self::Playlists => t.list_playlists.into(),
            Self::Search(q) => fill(t.list_search, &[q]),
            Self::Playlist(n) => fill(t.list_playlist, &[n]),
            Self::Artist(n) => fill(t.list_artist, &[n]),
            Self::ArtistAll(n) => fill(t.list_artist_all, &[n]),
        }
    }
}

/// Status-line message; translated at render time so a language switch
/// re-translates whatever is currently shown.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Ready,
    SearchOpen,
    SearchCancelled,
    Mode(PlaybackMode),
    Home,
    Loaded(ListKind, usize),
    Failed(ListKind, String),
    Playing { artist: String, title: String },
    Stopped,
    Paused,
    Resumed,
    NothingToLike,
    LikeFailed(String),
    Liked(String),
    Unliked(String),
    ArtistNeedTrack,
    Lang,
    Lyrics(bool),
    Quality(Quality),
    Saving(String),
    Saved(String),
    SaveFailed(String),
    SaveError(String),
    ConfirmRemove(String),
    Removed(String),
    LibraryDisabled,
    SyncStarted { todo: usize, already: usize },
    SyncNothing(usize),
    SyncCancelled,
    SyncDone { saved: usize, failed: usize },
    SyncFailed(String),
    StreamFailed(String),
    /// Previous session restored; the track waits paused.
    SessionRestored(String),
    /// No audio output: browsing works, playback only keeps time.
    AudioDown(String),
}

impl Status {
    pub fn text(&self, lang: Lang) -> String {
        let t = lang.t();
        match self {
            Self::Ready => t.st_ready.into(),
            Self::SearchOpen => t.st_search_open.into(),
            Self::SearchCancelled => t.st_search_cancelled.into(),
            Self::Mode(m) => fill(t.st_mode, &[m.label(lang)]),
            Self::Home => t.st_home.into(),
            Self::Loaded(ListKind::Playlists, n) => format!("{} · {}", ListKind::Playlists.title(lang), lang.playlists(*n)),
            Self::Loaded(kind, n) => format!("{} · {}", kind.title(lang), lang.tracks(*n)),
            Self::Failed(kind, e) => fill(t.st_failed, &[&kind.title(lang), e]),
            Self::Playing { artist, title } => fill(t.st_playing, &[artist, title]),
            Self::Stopped => t.st_stopped.into(),
            Self::Paused => t.st_paused.into(),
            Self::Resumed => t.st_resumed.into(),
            Self::NothingToLike => t.st_nothing_to_like.into(),
            Self::LikeFailed(e) => fill(t.st_like_failed, &[e]),
            Self::Liked(s) => fill(t.st_liked, &[s]),
            Self::Unliked(s) => fill(t.st_unliked, &[s]),
            Self::ArtistNeedTrack => t.st_artist_need_track.into(),
            Self::Lang => fill(t.st_lang, &[t.lang_name]),
            Self::Lyrics(on) => if *on { t.st_lyrics_on.into() } else { t.st_lyrics_off.into() },
            Self::StreamFailed(e) => fill(t.st_stream_failed, &[e]),
            Self::SessionRestored(title) => fill(t.st_session_restored, &[title]),
            Self::Quality(q) => fill(t.st_quality, &[t.qualities[q.index()]]),
            Self::Saving(s) => fill(t.st_saving, &[s]),
            Self::Saved(s) => fill(t.st_saved, &[s]),
            Self::SaveFailed(s) => fill(t.st_save_failed, &[s]),
            Self::SaveError(e) => fill(t.st_save_error, &[e]),
            Self::ConfirmRemove(s) => fill(t.st_confirm_remove, &[s]),
            Self::Removed(s) => fill(t.st_removed, &[s]),
            Self::LibraryDisabled => t.st_library_disabled.into(),
            Self::SyncStarted { todo, already } => fill(t.st_sync_started, &[&todo.to_string(), &already.to_string()]),
            Self::SyncNothing(n) => fill(t.st_sync_nothing, &[&n.to_string()]),
            Self::SyncCancelled => t.st_sync_cancelled.into(),
            Self::SyncDone { saved, failed } => fill(t.st_sync_done, &[&saved.to_string(), &failed.to_string()]),
            Self::SyncFailed(e) => fill(t.st_sync_failed, &[e]),
            Self::AudioDown(e) => fill(t.st_audio_down, &[e]),
        }
    }
}

/// Small xorshift PRNG; good enough for shuffle and avoids a dependency.
pub struct Rng(u64);

impl Rng {
    pub fn seeded(seed: u64) -> Self {
        Self(seed.max(1))
    }

    fn from_time() -> Self {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64);
        Self::seeded(nanos)
    }

    pub fn below(&mut self, n: usize) -> usize {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % n as u64) as usize
    }
}

/// Pick the next queue index. `manual` is true when the user pressed "next",
/// in which case LOOP still advances instead of repeating the same track.
pub fn next_index(mode: PlaybackMode, len: usize, current: usize, manual: bool, rng: &mut Rng) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match mode {
        PlaybackMode::Repeat if !manual => Some(current),
        PlaybackMode::Random if len > 1 => {
            let step = 1 + rng.below(len - 1);
            Some((current + step) % len)
        }
        _ => Some((current + 1) % len),
    }
}

pub fn prev_index(len: usize, current: usize) -> Option<usize> {
    (len > 0).then(|| (current + len - 1) % len)
}

pub const SPECTRUM_BANDS: usize = 48;
pub const HISTORY_LEN: usize = 160;
/// Manual lyric scrolling hands control back to playback after this long.
const MANUAL_SCROLL_TIMEOUT: Duration = Duration::from_secs(6);

/// Lyrics for the current track plus their scroll state.
pub struct LyricsView {
    pub track_id: String,
    pub lyrics: Option<Lyrics>,
    /// Eased scroll position in lines; the renderer anchors this line.
    pub scroll: f64,
    /// Manual offset (in lines) from the followed line, and when it was last
    /// touched. `None` means the view follows playback.
    pub manual: Option<(f64, Instant)>,
}

pub struct App {
    pub should_quit: bool,
    pub lang: Lang,
    pub mode: PlaybackMode,
    pub volume: u8,
    pub visualizer: bool,
    pub help: bool,
    pub focus: Focus,
    pub section: usize,
    /// `Some` while the search prompt is open.
    pub query: Option<String>,
    pub list: ListKind,
    pub entries: Vec<Entry>,
    pub selected: usize,
    pub queue: Vec<Track>,
    pub queue_pos: Option<usize>,
    pub status: Status,
    /// Spectrum bands, 0..=100.
    pub levels: Vec<u64>,
    /// Peak-hold per band, 0..=1, decaying slowly.
    pub peaks: Vec<f64>,
    /// Overall signal level over time (oldest first), 0..=100.
    pub history: Vec<u64>,
    pub lyrics: Option<LyricsView>,
    /// Latest real waveform (triggered), when the player produces audio.
    pub wave: Option<Vec<f32>>,
    /// Previous waveforms, newest first, for the phosphor afterglow.
    pub wave_history: VecDeque<Vec<f32>>,
    /// Quality label of the current stream ("FLAC · lossless"), if any.
    pub quality: Option<String>,
    /// Preferred stream quality (persisted).
    pub quality_pref: Quality,
    /// Lossless was asked for but this track only has a lossy stream.
    pub lossless_missing: bool,
    /// The current track plays from the local library.
    pub playing_local: bool,
    /// Saved tracks on disk.
    pub library: Library,
    /// Track awaiting a second `D` to delete its file, and when asked.
    pending_remove: Option<(String, Instant)>,
    /// Favourites sync in progress (⌥⇧D / --download-fav).
    pub sync: Option<Sync>,
    /// Save every track that starts playing (--download-all).
    pub auto_download: bool,
    pub show_lyrics: bool,
    pub frame: u64,
    last_tick: Instant,
    rng: Rng,
    pub service: Box<dyn MusicService>,
    pub player: Box<dyn Player>,
}

impl App {
    pub fn new(service: Box<dyn MusicService>, mut player: Box<dyn Player>, lang: Lang) -> Self {
        let volume = 70;
        player.set_volume(volume);
        Self {
            should_quit: false,
            lang,
            mode: PlaybackMode::All,
            volume,
            visualizer: true,
            help: false,
            focus: Focus::Nav,
            section: 0,
            query: None,
            list: ListKind::Overview,
            entries: Vec::new(),
            selected: 0,
            queue: Vec::new(),
            queue_pos: None,
            status: Status::Ready,
            levels: vec![0; SPECTRUM_BANDS],
            peaks: vec![0.0; SPECTRUM_BANDS],
            history: vec![0; HISTORY_LEN],
            lyrics: None,
            wave: None,
            wave_history: VecDeque::new(),
            quality: None,
            quality_pref: Quality::default(),
            lossless_missing: false,
            playing_local: false,
            library: Library::disabled(),
            pending_remove: None,
            sync: None,
            auto_download: false,
            show_lyrics: true,
            frame: 0,
            last_tick: Instant::now(),
            rng: Rng::from_time(),
            service,
            player,
        }
    }

    pub fn current_track(&self) -> Option<&Track> {
        self.queue_pos.and_then(|i| self.queue.get(i))
    }

    fn selected_track(&self) -> Option<&Track> {
        match self.entries.get(self.selected) {
            Some(Entry::Track(t)) => Some(t),
            _ => None,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        let code = match key.code {
            KeyCode::Char(c) if self.query.is_none() => KeyCode::Char(latin_key(c)),
            other => other,
        };
        if key.modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        // ⌥⇧D: with "Option as Meta" it arrives as Alt+D; without, macOS
        // turns Option+Shift+D into 'Î' (US layout).
        let alt_d = key.modifiers.contains(KeyModifiers::ALT) && matches!(code, KeyCode::Char('D' | 'd'));
        if self.query.is_none() && !self.help && (alt_d || key.code == KeyCode::Char('Î')) {
            self.toggle_sync();
            return;
        }
        if self.query.is_some() {
            self.on_query_key(code);
            return;
        }
        if self.help {
            if matches!(code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) | KeyCode::Enter) {
                self.help = false;
            } else if code == KeyCode::Char('q') {
                self.should_quit = true;
            }
            return;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('+') | KeyCode::Char('=') => self.change_volume(5),
            KeyCode::Char('-') => self.change_volume(-5),
            KeyCode::Char('c') => self.set_mode(PlaybackMode::Repeat),
            KeyCode::Char('a') => self.set_mode(PlaybackMode::All),
            KeyCode::Char('r') => self.set_mode(PlaybackMode::Random),
            KeyCode::Char('v') => self.visualizer = !self.visualizer,
            KeyCode::Char('y') => {
                self.show_lyrics = !self.show_lyrics;
                self.status = Status::Lyrics(self.show_lyrics);
            }
            KeyCode::Char('o') => self.cycle_quality(),
            KeyCode::Left => self.seek_by(-5),
            KeyCode::Right => self.seek_by(5),
            KeyCode::Char('[') => self.scroll_lyrics(-1.0),
            KeyCode::Char(']') => self.scroll_lyrics(1.0),
            KeyCode::Char('i') => {
                self.lang = self.lang.toggle();
                self.status = Status::Lang;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                self.focus = match self.focus {
                    Focus::Nav if !self.entries.is_empty() => Focus::List,
                    _ => Focus::Nav,
                }
            }
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Enter => self.activate(),
            KeyCode::Char(' ') => self.toggle_pause(),
            KeyCode::Char('n') => self.advance(true),
            KeyCode::Char('b') => self.previous(),
            KeyCode::Char('L') => self.toggle_like(),
            KeyCode::Char('D') => self.save_or_remove(),
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) => self.help = true,
            KeyCode::Char('f') | KeyCode::Char('/') => {
                self.query = Some(String::new());
                self.status = Status::SearchOpen;
            }
            KeyCode::Char('p') => self.open_section(Section::Playlists),
            KeyCode::Char('w') => self.open_section(Section::Wave),
            KeyCode::Char('d') => self.open_section(Section::Cached),
            KeyCode::Char('l') => self.open_section(Section::Likes),
            KeyCode::Char('s') => self.artist_search(false),
            KeyCode::Char('S') => self.artist_search(true),
            _ => {}
        }
    }

    fn on_query_key(&mut self, code: KeyCode) {
        let Some(query) = self.query.as_mut() else { return };
        match code {
            KeyCode::Esc => {
                self.query = None;
                self.status = Status::SearchCancelled;
            }
            KeyCode::Enter => {
                let q = self.query.take().unwrap_or_default();
                let result = self.service.search(&q);
                self.show_tracks(ListKind::Search(q), result);
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char(c) => query.push(c),
            _ => {}
        }
    }

    fn change_volume(&mut self, delta: i16) {
        self.volume = (i16::from(self.volume) + delta).clamp(0, 100) as u8;
        self.player.set_volume(self.volume);
    }

    fn set_mode(&mut self, mode: PlaybackMode) {
        self.mode = mode;
        self.status = Status::Mode(mode);
    }

    fn move_selection(&mut self, delta: isize) {
        let (cursor, len) = match self.focus {
            Focus::Nav => (&mut self.section, Section::ALL.len()),
            Focus::List => (&mut self.selected, self.entries.len()),
        };
        if len == 0 {
            return;
        }
        *cursor = cursor.saturating_add_signed(delta).min(len - 1);
    }

    fn activate(&mut self) {
        match self.focus {
            Focus::Nav => self.open_section(Section::ALL[self.section]),
            Focus::List => match self.entries.get(self.selected).cloned() {
                Some(Entry::Playlist(p)) => {
                    let result = self.service.playlist_tracks(&p.id);
                    self.show_tracks(ListKind::Playlist(p.title), result);
                }
                Some(Entry::Track(_)) => self.play_from_list(),
                None => {}
            },
        }
    }

    fn open_section(&mut self, section: Section) {
        if let Some(i) = Section::ALL.iter().position(|s| *s == section) {
            self.section = i;
        }
        match section {
            Section::Home => {
                self.set_entries(ListKind::Overview, Vec::new());
                self.status = Status::Home;
            }
            Section::Wave => {
                let r = self.service.my_wave();
                self.show_tracks(ListKind::Wave, r);
            }
            Section::Likes => {
                let r = self.service.likes();
                self.show_tracks(ListKind::Likes, r);
            }
            Section::Cached => {
                // The local library first, then anything the backend marks cached.
                let r = self.service.cached().map(|backend| {
                    let mut all = self.library.tracks();
                    all.extend(backend.into_iter().filter(|t| !self.library.is_saved(&t.id)));
                    all
                });
                self.show_tracks(ListKind::Cached, r);
            }
            Section::Playlists => match self.service.playlists() {
                Ok(ps) => {
                    self.status = Status::Loaded(ListKind::Playlists, ps.len());
                    self.set_entries(ListKind::Playlists, ps.into_iter().map(Entry::Playlist).collect());
                }
                Err(e) => self.status = Status::Failed(ListKind::Playlists, format!("{e:#}")),
            },
        }
    }

    fn show_tracks(&mut self, kind: ListKind, result: Result<Vec<Track>>) {
        match result {
            Ok(tracks) => {
                self.status = Status::Loaded(kind.clone(), tracks.len());
                self.set_entries(kind, tracks.into_iter().map(Entry::Track).collect());
            }
            Err(e) => self.status = Status::Failed(kind, format!("{e:#}")),
        }
    }

    fn set_entries(&mut self, kind: ListKind, entries: Vec<Entry>) {
        self.list = kind;
        self.entries = entries;
        self.selected = 0;
        self.focus = if self.entries.is_empty() { Focus::Nav } else { Focus::List };
    }

    fn play_from_list(&mut self) {
        let tracks: Vec<Track> = self
            .entries
            .iter()
            .filter_map(|e| match e {
                Entry::Track(t) => Some(t.clone()),
                Entry::Playlist(_) => None,
            })
            .collect();
        let Some(id) = self.selected_track().map(|t| t.id.clone()) else { return };
        let pos = tracks.iter().position(|t| t.id == id);
        self.queue = tracks;
        self.play_at(pos);
    }

    fn play_at(&mut self, pos: Option<usize>) {
        self.queue_pos = pos;
        match pos.and_then(|i| self.queue.get(i)).cloned() {
            Some(track) => {
                // A saved copy plays from disk: instant and offline.
                let local = self.library.local_stream(&track.id);
                self.playing_local = local.is_some();
                let fetched = match local {
                    Some(s) => Ok(Some(s)),
                    None => self.service.stream(&track.id, self.quality_pref),
                };
                let source = match fetched {
                    Ok(source) => source,
                    Err(e) => {
                        self.player.stop();
                        self.status = Status::StreamFailed(format!("{e:#}"));
                        return;
                    }
                };
                self.player.play(&track, source.as_ref().map(|s| s.url.as_str()));
                // --download-all: keep a copy of everything that gets played.
                if self.auto_download
                    && !self.playing_local
                    && !self.library.is_saved(&track.id)
                    && let Some(src) = &source
                {
                    let _ = self.library.start(&track, src);
                }
                self.set_quality_from(source.as_ref());
                if self.playing_local
                    && let Some(q) = self.quality.as_mut()
                {
                    q.push_str(" · ");
                    q.push_str(self.lang.t().from_disk);
                }
                self.status = Status::Playing { artist: track.artist.clone(), title: track.title.clone() };
                self.wave_history.clear();
                self.load_lyrics();
            }
            None => {
                self.player.stop();
                self.status = Status::Stopped;
            }
        }
    }

    fn set_quality_from(&mut self, stream: Option<&crate::model::Stream>) {
        self.quality = stream.map(|s| s.label());
        self.lossless_missing = stream.is_some_and(|s| self.quality_pref == Quality::Lossless && !s.is_lossless());
    }

    /// ⌥⇧D: start syncing all favourites into the library, or stop a running sync.
    pub fn toggle_sync(&mut self) {
        if let Some(sync) = self.sync.as_mut() {
            sync.cancel();
            self.status = Status::SyncCancelled;
            return;
        }
        if !self.library.enabled() {
            self.status = Status::LibraryDisabled;
            return;
        }
        match self.service.likes_all() {
            Ok(liked) => {
                let sync = Sync::new(liked, &self.library);
                if sync.total == 0 {
                    self.status = Status::SyncNothing(sync.already);
                } else {
                    self.status = Status::SyncStarted { todo: sync.total, already: sync.already };
                    self.sync = Some(sync);
                }
            }
            Err(e) => self.status = Status::SyncFailed(format!("{e:#}")),
        }
    }

    /// Whether a track is in the local library (or flagged cached by the backend).
    pub fn is_saved(&self, track: &Track) -> bool {
        track.cached || self.library.is_saved(&track.id)
    }

    /// `D`: save the track (selected in the list, else playing) to the
    /// library; on a saved track, a second `D` within a few seconds deletes it.
    fn save_or_remove(&mut self) {
        let target = match self.focus {
            Focus::List => self.selected_track().or_else(|| self.current_track()),
            Focus::Nav => self.current_track().or_else(|| self.selected_track()),
        }
        .cloned();
        let Some(track) = target else {
            self.status = Status::NothingToLike;
            return;
        };
        if !self.library.enabled() {
            self.status = Status::LibraryDisabled;
            return;
        }
        if self.library.is_saved(&track.id) {
            let confirmed = self.pending_remove.as_ref().is_some_and(|(id, at)| *id == track.id && at.elapsed() < Duration::from_secs(4));
            if confirmed {
                self.pending_remove = None;
                self.status = match self.library.remove(&track.id) {
                    Ok(()) => Status::Removed(track.title.clone()),
                    Err(e) => Status::SaveError(format!("{e:#}")),
                };
                for t in self.entries.iter_mut().filter_map(|e| if let Entry::Track(t) = e { Some(t) } else { None }) {
                    if t.id == track.id {
                        t.cached = false;
                    }
                }
                if self.list == ListKind::Cached {
                    self.entries.retain(|e| !matches!(e, Entry::Track(t) if t.id == track.id));
                    self.selected = self.selected.min(self.entries.len().saturating_sub(1));
                }
            } else {
                self.pending_remove = Some((track.id.clone(), Instant::now()));
                self.status = Status::ConfirmRemove(track.title.clone());
            }
            return;
        }
        if self.library.is_downloading(&track.id) {
            self.status = Status::Saving(track.title.clone());
            return;
        }
        let stream = match self.service.stream(&track.id, self.quality_pref) {
            Ok(Some(s)) => s,
            Ok(None) => {
                self.status = Status::SaveError(self.lang.t().no_audio_to_save.into());
                return;
            }
            Err(e) => {
                self.status = Status::SaveError(format!("{e:#}"));
                return;
            }
        };
        self.status = match self.library.start(&track, &stream) {
            Ok(()) => Status::Saving(track.title.clone()),
            Err(e) => Status::SaveError(format!("{e:#}")),
        };
    }

    /// Everything needed to pick up where the listener left off.
    pub fn snapshot(&self) -> crate::session::Session {
        crate::session::Session {
            backend: self.service.name().to_string(),
            list: self.list.clone(),
            entries: self.entries.clone(),
            selected: self.selected,
            section: self.section,
            focus: self.focus,
            queue: self.queue.clone(),
            queue_pos: self.queue_pos,
            position_secs: self.player.position().as_secs_f64(),
            mode: self.mode,
            volume: self.volume,
        }
    }

    /// Restore a saved session from the same backend/account. The current
    /// track is reopened (fresh stream link) and left paused at its position.
    pub fn restore(&mut self, s: crate::session::Session) {
        if s.backend != self.service.name() {
            return;
        }
        self.mode = s.mode;
        self.volume = s.volume.min(100);
        self.player.set_volume(self.volume);
        self.list = s.list;
        self.entries = s.entries;
        self.selected = s.selected.min(self.entries.len().saturating_sub(1));
        self.section = s.section.min(Section::ALL.len() - 1);
        self.focus = if self.entries.is_empty() { Focus::Nav } else { s.focus };
        self.queue = s.queue;
        let pos = s.queue_pos.filter(|&i| i < self.queue.len());
        let Some(i) = pos else { return };
        self.play_at(Some(i));
        if self.player.state() == PlayerState::Stopped {
            return; // stream unavailable: status already explains why
        }
        let track = self.queue[i].clone();
        let at = Duration::from_secs_f64(s.position_secs.max(0.0));
        // Near the very end, start the track over rather than ending at once.
        if at + Duration::from_secs(3) < track.duration() {
            self.player.seek(at);
        }
        self.player.toggle_pause();
        if let Some(view) = self.lyrics.as_mut() {
            view.scroll = -1.0;
        }
        self.status = Status::SessionRestored(track.title);
    }

    /// Switch to the next quality tier and, if a real stream is playing,
    /// reopen it at the same position so the change is heard immediately.
    fn cycle_quality(&mut self) {
        self.quality_pref = self.quality_pref.next();
        self.status = Status::Quality(self.quality_pref);
        let Some(track) = self.current_track().cloned() else { return };
        if self.quality.is_none() || self.player.state() == PlayerState::Stopped {
            return; // nothing streaming: applies to the next track
        }
        let (pos, paused) = (self.player.position(), self.player.state() == PlayerState::Paused);
        match self.service.stream(&track.id, self.quality_pref) {
            Ok(Some(src)) => {
                self.player.play(&track, Some(&src.url));
                self.player.seek(pos);
                if paused {
                    self.player.toggle_pause();
                }
                self.set_quality_from(Some(&src));
            }
            Ok(None) => {}
            Err(e) => self.status = Status::StreamFailed(format!("{e:#}")),
        }
    }

    fn seek_by(&mut self, secs: i64) {
        if self.current_track().is_none() {
            return;
        }
        let pos = self.player.position().as_secs() as i64 + secs;
        self.player.seek(Duration::from_secs(pos.max(0) as u64));
        // Seeking means "take me there": drop any manual lyric offset.
        if let Some(view) = self.lyrics.as_mut() {
            view.manual = None;
        }
    }

    fn toggle_pause(&mut self) {
        if self.player.state() == PlayerState::Stopped {
            if self.selected_track().is_some() {
                self.play_from_list();
            }
            return;
        }
        self.player.toggle_pause();
        self.status = match self.player.state() {
            PlayerState::Paused => Status::Paused,
            _ => Status::Resumed,
        };
    }

    fn advance(&mut self, manual: bool) {
        let Some(cur) = self.queue_pos else { return };
        let next = next_index(self.mode, self.queue.len(), cur, manual, &mut self.rng);
        self.play_at(next);
    }

    fn previous(&mut self) {
        let Some(cur) = self.queue_pos else { return };
        self.play_at(prev_index(self.queue.len(), cur));
    }

    fn toggle_like(&mut self) {
        let Some(track) = self.current_track().or_else(|| self.selected_track()).cloned() else {
            self.status = Status::NothingToLike;
            return;
        };
        let liked = !track.liked;
        if let Err(e) = self.service.set_liked(&track.id, liked) {
            self.status = Status::LikeFailed(format!("{e:#}"));
            return;
        }
        let update = |t: &mut Track| {
            if t.id == track.id {
                t.liked = liked;
            }
        };
        self.queue.iter_mut().for_each(update);
        self.entries.iter_mut().for_each(|e| {
            if let Entry::Track(t) = e {
                update(t);
            }
        });
        self.status = if liked { Status::Liked(track.title) } else { Status::Unliked(track.title) };
    }

    /// `s`: popular tracks of the playing (or selected) track's artist;
    /// `S`: all of that artist's tracks.
    fn artist_search(&mut self, all: bool) {
        // In an artist view, S/s switch between popular and all for that artist.
        let from_list = matches!(self.list, ListKind::Artist(_) | ListKind::ArtistAll(_));
        let source = if from_list { self.selected_track().or_else(|| self.current_track()) } else { self.current_track().or_else(|| self.selected_track()) };
        let Some(track) = source.cloned() else {
            self.status = Status::ArtistNeedTrack;
            return;
        };
        let r = self.service.artist_tracks(&track, all);
        let name = track.artist.split(", ").next().unwrap_or(&track.artist).to_string();
        self.show_tracks(if all { ListKind::ArtistAll(name) } else { ListKind::Artist(name) }, r);
    }

    fn load_lyrics(&mut self) {
        let Some(track) = self.current_track() else { return };
        if self.lyrics.as_ref().is_some_and(|l| l.track_id == track.id) {
            return;
        }
        let id = track.id.clone();
        // A lyrics failure must never interrupt playback: treat it as "no lyrics".
        let lyrics = self.service.lyrics(&id).ok().flatten();
        self.lyrics = Some(LyricsView { track_id: id, lyrics, scroll: -1.0, manual: None });
    }

    /// Line the lyric view follows (before manual offset): the sung line, or
    /// -1 before the first synced line so line 0 waits just below the anchor.
    pub fn lyric_line(&self) -> Option<f64> {
        let track = self.current_track()?;
        let lyrics = self.lyrics.as_ref()?.lyrics.as_ref()?;
        let cur = lyrics.current(self.player.position(), track.duration());
        Some(cur.map_or(-1.0, |i| i as f64))
    }

    fn lyric_target(&self) -> Option<f64> {
        let follow = self.lyric_line()?;
        let view = self.lyrics.as_ref()?;
        let len = view.lyrics.as_ref()?.lines.len() as f64;
        let offset = view.manual.map_or(0.0, |(o, _)| o);
        Some((follow + offset).clamp(-1.0, (len - 1.0).max(0.0)))
    }

    fn scroll_lyrics(&mut self, delta: f64) {
        let (Some(follow), Some(view)) = (self.lyric_line(), self.lyrics.as_ref()) else { return };
        let Some(len) = view.lyrics.as_ref().map(|l| l.lines.len() as f64) else { return };
        let offset = view.manual.map_or(0.0, |(o, _)| o) + delta;
        // Keep the offset within the text so scrolling back feels immediate.
        let offset = (follow + offset).clamp(-1.0, (len - 1.0).max(0.0)) - follow;
        if let Some(view) = self.lyrics.as_mut() {
            view.manual = Some((offset, Instant::now()));
        }
        self.show_lyrics = true;
    }

    pub fn lyrics_manual(&self) -> bool {
        self.lyrics.as_ref().is_some_and(|v| v.manual.is_some())
    }

    pub fn tick(&mut self) {
        if self.player.finished() {
            self.advance(false);
        }
        for done in self.library.poll() {
            // Sync downloads report through the footer bar, not the status line.
            if self.sync.as_mut().is_some_and(|s| s.on_finished(&done)) {
                continue;
            }
            self.status = match done {
                Finished::Saved { title, .. } => Status::Saved(title),
                Finished::Failed { title, .. } => Status::SaveFailed(title),
            };
        }
        if let Some(sync) = self.sync.as_mut() {
            sync.pump(&mut self.library, self.service.as_ref(), self.quality_pref);
            if sync.is_done() {
                self.status = Status::SyncDone { saved: sync.saved, failed: sync.failed };
                self.sync = None;
            }
        }
        if let Some(view) = self.lyrics.as_mut()
            && view.manual.is_some_and(|(_, at)| at.elapsed() >= MANUAL_SCROLL_TIMEOUT)
        {
            view.manual = None;
        }
        if let Some(target) = self.lyric_target()
            && let Some(view) = self.lyrics.as_mut()
        {
            // Ease towards the target line so the text glides upwards.
            let d = target - view.scroll;
            view.scroll = if d.abs() < 0.02 { target } else { view.scroll + d * 0.35 };
        }
        if self.last_tick.elapsed() >= Duration::from_millis(90) {
            self.last_tick = Instant::now();
            self.frame = self.frame.wrapping_add(1);
            let state = self.player.state();
            if state == PlayerState::Paused {
                return; // freeze the picture while paused
            }
            let live = self.visualizer && state == PlayerState::Playing;
            let n = self.levels.len();
            // Only real analysis of what is playing drives the visuals.
            let analysis = if live { self.player.analysis(n) } else { None };
            let targets: Vec<f64> = match &analysis {
                Some(a) => a.bands.iter().map(|&b| f64::from(b)).collect(),
                None => vec![0.0; n],
            };
            for (i, target) in targets.iter().enumerate().take(n) {
                let target = (target * 100.0).round() as u64;
                let cur = self.levels[i];
                // Fast attack, slower release — like a real analyser.
                self.levels[i] = if target > cur { target } else { cur.saturating_sub(9).max(target) };
                let lvl = self.levels[i] as f64 / 100.0;
                self.peaks[i] = if lvl > self.peaks[i] { lvl } else { (self.peaks[i] - 0.02).max(lvl) };
            }
            let energy = match &analysis {
                Some(a) => (a.level * 100.0) as u64,
                None => self.history.last().copied().unwrap_or(0).saturating_sub(6),
            };
            match analysis {
                Some(a) => {
                    if let Some(prev) = self.wave.replace(a.wave) {
                        self.wave_history.push_front(prev);
                        self.wave_history.truncate(3);
                    }
                }
                None if state == PlayerState::Stopped || !self.visualizer => {
                    self.wave = None;
                    self.wave_history.clear();
                }
                None => {}
            }
            self.history.remove(0);
            self.history.push(energy.min(100));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::SimPlayer;
    use crate::testing::FakeService;

    fn app() -> App {
        App::new(Box::new(FakeService::default()), Box::new(SimPlayer::default()), Lang::Ru)
    }

    fn press(app: &mut App, code: KeyCode) {
        app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
    }

    fn type_str(app: &mut App, s: &str) {
        s.chars().for_each(|c| press(app, KeyCode::Char(c)));
    }

    #[test]
    fn next_index_modes() {
        let mut rng = Rng::seeded(42);
        assert_eq!(next_index(PlaybackMode::All, 3, 2, false, &mut rng), Some(0));
        assert_eq!(next_index(PlaybackMode::Repeat, 3, 1, false, &mut rng), Some(1));
        assert_eq!(next_index(PlaybackMode::Repeat, 3, 1, true, &mut rng), Some(2));
        assert_eq!(next_index(PlaybackMode::All, 0, 0, true, &mut rng), None);
        for _ in 0..100 {
            let n = next_index(PlaybackMode::Random, 5, 3, false, &mut rng).unwrap();
            assert!(n < 5 && n != 3);
        }
        assert_eq!(next_index(PlaybackMode::Random, 1, 0, false, &mut rng), Some(0));
        assert_eq!(prev_index(3, 0), Some(2));
    }

    #[test]
    fn search_keys_go_to_query_not_commands() {
        let mut a = app();
        press(&mut a, KeyCode::Char('f'));
        type_str(&mut a, "q");
        assert!(!a.should_quit, "'q' inside the prompt must not quit");
        press(&mut a, KeyCode::Backspace);
        type_str(&mut a, "neon");
        press(&mut a, KeyCode::Enter);
        assert!(a.query.is_none());
        assert_eq!(a.entries.len(), 3);
        assert_eq!(a.focus, Focus::List);
    }

    #[test]
    fn cyrillic_search_is_typed_verbatim_and_matches() {
        let mut a = app();
        press(&mut a, KeyCode::Char('а')); // 'f' in the Russian layout opens search
        assert!(a.query.is_some());
        type_str(&mut a, "елки"); // must not be remapped to Latin, ё == е
        assert_eq!(a.query.as_deref(), Some("елки"));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.entries.len(), 1);
        assert_eq!(a.list, ListKind::Search("елки".into()));
    }

    #[test]
    fn russian_layout_hotkeys() {
        let mut a = app();
        press(&mut a, KeyCode::Char('ц')); // w
        assert_eq!(a.list, ListKind::Wave);
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.player.state(), PlayerState::Playing);
        press(&mut a, KeyCode::Char('т')); // n
        assert_eq!(a.queue_pos, Some(1));
        press(&mut a, KeyCode::Char('Д')); // L
        assert!(matches!(a.status, Status::Liked(_) | Status::Unliked(_)));
        press(&mut a, KeyCode::Char('ш')); // i
        assert_eq!(a.lang, Lang::En);
        press(&mut a, KeyCode::Char('й')); // q
        assert!(a.should_quit);
    }

    #[test]
    fn status_retranslates_on_language_switch() {
        let mut a = app();
        press(&mut a, KeyCode::Char('w'));
        assert!(a.status.text(a.lang).starts_with("МОЯ ВОЛНА · "));
        let loaded = a.status.clone();
        press(&mut a, KeyCode::Char('i'));
        assert_eq!(a.status.text(a.lang), "Language: English");
        assert!(loaded.text(a.lang).starts_with("MY WAVE · "));
        assert_eq!(a.list.title(a.lang), "MY WAVE");
    }

    #[test]
    fn help_overlay_swallows_keys() {
        let mut a = app();
        press(&mut a, KeyCode::Char('?'));
        assert!(a.help);
        press(&mut a, KeyCode::Char('w'));
        assert_eq!(a.list, ListKind::Overview);
        press(&mut a, KeyCode::Esc);
        assert!(!a.help && !a.should_quit);
    }

    #[test]
    fn play_next_prev_and_like() {
        let mut a = app();
        press(&mut a, KeyCode::Char('w'));
        press(&mut a, KeyCode::Char('j'));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.queue_pos, Some(1));
        assert_eq!(a.player.state(), PlayerState::Playing);

        press(&mut a, KeyCode::Char('n'));
        assert_eq!(a.queue_pos, Some(2));
        press(&mut a, KeyCode::Char('b'));
        press(&mut a, KeyCode::Char('b'));
        assert_eq!(a.queue_pos, Some(0));

        press(&mut a, KeyCode::Char(' '));
        assert_eq!(a.player.state(), PlayerState::Paused);

        let id = a.current_track().unwrap().id.clone();
        let was = a.current_track().unwrap().liked;
        press(&mut a, KeyCode::Char('L'));
        assert_eq!(a.current_track().unwrap().liked, !was);
        assert_eq!(a.service.likes().unwrap().iter().any(|t| t.id == id), !was);
    }

    #[test]
    fn playlist_opens_tracks_and_artist_search() {
        let mut a = app();
        press(&mut a, KeyCode::Char('p'));
        assert!(matches!(a.entries[0], Entry::Playlist(_)));
        press(&mut a, KeyCode::Enter);
        assert_eq!(a.list, ListKind::Playlist("Ночной город".into()));
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Char('s'));
        assert_eq!(a.list, ListKind::Artist("Neon Harbor".into()));
        press(&mut a, KeyCode::Char('S'));
        assert_eq!(a.list, ListKind::ArtistAll("Neon Harbor".into()));
        press(&mut a, KeyCode::Char('Ы')); // Shift+S in the Russian layout
        assert_eq!(a.list, ListKind::ArtistAll("Neon Harbor".into()));
        press(&mut a, KeyCode::Char('s'));
        assert_eq!(a.list, ListKind::Artist("Neon Harbor".into()));
        assert_eq!(a.entries.len(), 3);
    }

    #[test]
    fn lyrics_load_follow_and_scroll() {
        let mut a = app();
        press(&mut a, KeyCode::Char('w')); // first wave track is "Static Bloom", which has LRC
        press(&mut a, KeyCode::Enter);
        let view = a.lyrics.as_ref().unwrap();
        assert_eq!(view.track_id, "2");
        assert!(view.lyrics.as_ref().unwrap().synced);
        assert_eq!(a.lyric_line(), Some(-1.0), "before the first timestamp");

        press(&mut a, KeyCode::Char(']'));
        press(&mut a, KeyCode::Char('ъ')); // ']' in the Russian layout
        assert!(a.lyrics_manual());
        assert_eq!(a.lyric_target(), Some(1.0));
        for _ in 0..30 {
            a.last_tick -= Duration::from_millis(100);
            a.tick();
        }
        assert!((a.lyrics.as_ref().unwrap().scroll - 1.0).abs() < 0.05, "scroll eases to the target");

        press(&mut a, KeyCode::Char('['));
        press(&mut a, KeyCode::Char('['));
        press(&mut a, KeyCode::Char('['));
        assert_eq!(a.lyric_target(), Some(-1.0), "clamped at the top");

        press(&mut a, KeyCode::Char('y'));
        assert!(!a.show_lyrics);
        assert_eq!(a.status, Status::Lyrics(false));
    }

    #[test]
    fn track_without_lyrics() {
        let mut a = app();
        press(&mut a, KeyCode::Char('w'));
        press(&mut a, KeyCode::Char('j')); // "Low Tide" has no lyrics
        press(&mut a, KeyCode::Enter);
        assert!(a.lyrics.as_ref().unwrap().lyrics.is_none());
        assert_eq!(a.lyric_line(), None);
        press(&mut a, KeyCode::Char(']')); // harmless
        assert!(!a.lyrics_manual());
    }

    #[test]
    fn session_round_trip_resumes_paused_at_position() {
        let mut a = app();
        press(&mut a, KeyCode::Char('p'));
        press(&mut a, KeyCode::Enter); // open "Ночной город"
        press(&mut a, KeyCode::Char('j'));
        press(&mut a, KeyCode::Enter); // play its 2nd track
        press(&mut a, KeyCode::Char('r'));
        press(&mut a, KeyCode::Char('-'));
        a.player.seek(Duration::from_secs(42));
        let saved = a.snapshot();
        let json = serde_json::to_string(&saved).unwrap();

        let mut b = app();
        b.restore(serde_json::from_str(&json).unwrap());
        assert_eq!(b.list, ListKind::Playlist("Ночной город".into()));
        assert_eq!(b.entries.len(), 3);
        assert_eq!(b.selected, 1);
        assert_eq!(b.queue_pos, Some(1));
        assert_eq!(b.mode, PlaybackMode::Random);
        assert_eq!(b.volume, 65);
        assert_eq!(b.player.state(), PlayerState::Paused);
        let pos = b.player.position().as_secs();
        assert!((42..=43).contains(&pos), "resumes at the saved position, got {pos}");
        assert!(matches!(b.status, Status::SessionRestored(_)));
        press(&mut b, KeyCode::Char(' '));
        assert_eq!(b.player.state(), PlayerState::Playing);
    }

    #[test]
    fn session_from_another_backend_is_ignored() {
        let mut a = app();
        let mut s = a.snapshot();
        s.backend = "YANDEX · someone".into();
        s.queue = vec![];
        s.list = ListKind::Wave;
        a.restore(s);
        assert_eq!(a.list, ListKind::Overview);
    }

    #[test]
    fn lossless_missing_only_when_lossless_was_asked_for() {
        use crate::model::Stream;
        let mut a = app();
        let mp3 = Stream { url: String::new(), codec: "mp3".into(), bitrate_kbps: 320 };
        let flac = Stream { url: String::new(), codec: "flac-mp4".into(), bitrate_kbps: 0 };
        a.set_quality_from(Some(&mp3));
        assert!(a.lossless_missing);
        a.set_quality_from(Some(&flac));
        assert!(!a.lossless_missing);
        a.quality_pref = Quality::High;
        a.set_quality_from(Some(&mp3));
        assert!(!a.lossless_missing, "lossless was not requested");
        a.set_quality_from(None);
        assert!(!a.lossless_missing && a.quality.is_none());
    }

    #[test]
    fn save_needs_a_library_and_audio() {
        let mut a = app();
        press(&mut a, KeyCode::Char('w'));
        press(&mut a, KeyCode::Char('D'));
        assert_eq!(a.status, Status::LibraryDisabled);
        let dir = std::env::temp_dir().join(format!("yamusiq-app-lib-{}", std::process::id()));
        a.library = Library::open(dir.clone());
        press(&mut a, KeyCode::Char('В')); // Shift+D in the Russian layout
        assert!(matches!(a.status, Status::SaveError(_)), "fake tracks have no audio to save");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn alt_shift_d_toggles_sync() {
        let mut a = app();
        a.on_key(KeyEvent::new(KeyCode::Char('D'), KeyModifiers::ALT | KeyModifiers::SHIFT));
        assert_eq!(a.status, Status::LibraryDisabled, "needs a library");
        let dir = std::env::temp_dir().join(format!("yamusiq-sync-{}", std::process::id()));
        a.library = Library::open(dir.clone());
        press(&mut a, KeyCode::Char('Î')); // Option+Shift+D without Meta
        assert!(matches!(a.status, Status::SyncStarted { .. }));
        assert!(a.sync.is_some());
        press(&mut a, KeyCode::Char('Î'));
        assert_eq!(a.status, Status::SyncCancelled);
        // Demo likes have no audio: the remaining run fails fast and reports.
        a.tick();
        assert!(a.sync.is_none());
        assert!(matches!(a.status, Status::SyncDone { saved: 0, .. }));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn volume_is_clamped() {
        let mut a = app();
        for _ in 0..10 {
            press(&mut a, KeyCode::Char('+'));
        }
        assert_eq!(a.volume, 100);
        for _ in 0..30 {
            press(&mut a, KeyCode::Char('-'));
        }
        assert_eq!(a.volume, 0);
    }
}
