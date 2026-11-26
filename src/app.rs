use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::style::Color;

use crate::i18n::{fill, latin_key, Lang};
use crate::library::{Finished, Library};
use crate::sync::Sync;
use crate::model::{Lyrics, Playlist, Quality, Track};
use crate::player::{Player, PlayerState};
use crate::net::{Job, Outcome, Worker};
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
    /// A catalogue request is in flight; the footer says so.
    Loading(ListKind),
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
            Self::Loading(kind) => format!("{} · {}", kind.title(lang), t.buffering),
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
/// Which list a job will fill, for the footer and for the status line.
fn job_kind(job: &Job) -> ListKind {
    match job {
        Job::Wave => ListKind::Wave,
        Job::Likes => ListKind::Likes,
        Job::Cached => ListKind::Cached,
        Job::Playlists => ListKind::Playlists,
        Job::PlaylistTracks { title, .. } => ListKind::Playlist(title.clone()),
        Job::Search { query } => ListKind::Search(query.clone()),
        Job::Artist { track, all } => {
            let name = track.artist.split(", ").next().unwrap_or(&track.artist).to_string();
            if *all { ListKind::ArtistAll(name) } else { ListKind::Artist(name) }
        }
        // These do not fill a list; the status line describes them itself.
        Job::SyncLikes
        | Job::Stream { .. }
        | Job::Lyrics { .. }
        | Job::Like { .. }
        | Job::SaveStream { .. }
        | Job::QualityStream { .. }
        | Job::SyncStream { .. } => ListKind::Overview,
    }
}

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
    /// Where to seek and pause once a restored track's stream arrives.
    pending_resume: Option<(usize, Duration)>,
    /// Track, position and pause state to restore after a quality switch.
    pending_quality: Option<(String, Duration, bool)>,
    /// Favourites sync in progress (⌥⇧D / --download-fav).
    pub sync: Option<Sync>,
    /// Save every track that starts playing (--download-all).
    pub auto_download: bool,
    pub show_lyrics: bool,
    pub frame: u64,
    last_tick: Instant,
    rng: Rng,
    /// Backend identity and shared handle. All TUI network calls go through
    /// `net`; keeping the handle also lets tests inspect the shared fake.
    pub service: Box<dyn MusicService>,
    /// Catalogue calls that must never block the screen.
    pub net: Worker,
    pub player: Box<dyn Player>,
}

impl App {
    pub fn new(service: Box<dyn MusicService>, mut player: Box<dyn Player>, lang: Lang) -> Self {
        let volume = 70;
        player.set_volume(volume);
        let net = Worker::spawn(service.clone_boxed());
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
            pending_resume: None,
            pending_quality: None,
            sync: None,
            auto_download: false,
            show_lyrics: true,
            frame: 0,
            last_tick: Instant::now(),
            rng: Rng::from_time(),
            service,
            net,
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
                let query = self.query.take().unwrap_or_default();
                self.await_job(Job::Search { query });
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
                    self.await_job(Job::PlaylistTracks { id: p.id.clone(), title: p.title.clone() });
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
        // Everything below is a request: queue it and let the frame keep coming.
        let job = match section {
            Section::Home => {
                self.net.cancel_list();
                self.set_entries(ListKind::Overview, Vec::new());
                self.status = Status::Home;
                return;
            }
            Section::Wave => Job::Wave,
            Section::Likes => Job::Likes,
            Section::Cached => Job::Cached,
            Section::Playlists => Job::Playlists,
        };
        self.await_job(job);
    }

    /// Queues a catalogue job and says so in the footer while it runs.
    fn await_job(&mut self, job: Job) {
        let kind = job_kind(&job);
        self.net.post(job);
        self.status = Status::Loading(kind);
    }

    /// Takes one finished catalogue job, if the worker has one for us.
    fn collect_job(&mut self) -> bool {
        let Some(done) = self.net.poll() else { return false };
        // Stream and lyrics answers are not list loads; hand them to their own
        // handlers so the status line keeps saying "playing" / "buffering".
        if let Job::Stream { track, position, .. } = &done.job {
            return self.finish_stream(track.clone(), *position, done.result);
        }
        if let Job::Lyrics { track_id } = &done.job {
            return self.finish_lyrics(track_id.clone(), done.result);
        }
        if let Job::Like { track_id, liked } = &done.job {
            return self.finish_like(track_id.clone(), *liked, done.result);
        }
        if let Job::SaveStream { track, .. } = &done.job {
            return self.finish_save_stream(track.clone(), done.result);
        }
        if let Job::QualityStream { track, .. } = &done.job {
            return self.finish_quality_stream(track.clone(), done.result);
        }
        if let Job::SyncStream { track, .. } = &done.job {
            return self.finish_sync_stream(track.clone(), done.result);
        }
        if done.job == Job::SyncLikes {
            match done.result {
                Ok(Outcome::Tracks(liked)) => {
                    let sync = Sync::new(liked, &self.library);
                    if sync.total == 0 {
                        self.status = Status::SyncNothing(sync.already);
                    } else {
                        self.status = Status::SyncStarted { todo: sync.total, already: sync.already };
                        self.sync = Some(sync);
                    }
                }
                Err(e) => self.status = Status::SyncFailed(format!("{e:#}")),
                _ => {}
            }
            return true;
        }
        let kind = job_kind(&done.job);
        match done.result {
            Ok(Outcome::Tracks(tracks)) => {
                // "Cached" mixes the local library with what the backend keeps.
                let tracks = if matches!(done.job, Job::Cached) {
                    let mut all = self.library.tracks();
                    all.extend(tracks.into_iter().filter(|t| !self.library.is_saved(&t.id)));
                    all
                } else {
                    tracks
                };
                self.status = Status::Loaded(kind.clone(), tracks.len());
                self.set_entries(kind, tracks.into_iter().map(Entry::Track).collect());
            }
            Ok(Outcome::Playlists(ps)) => {
                self.status = Status::Loaded(kind.clone(), ps.len());
                self.set_entries(kind, ps.into_iter().map(Entry::Playlist).collect());
            }
            Ok(Outcome::Stream(_) | Outcome::Lyrics(_) | Outcome::Liked | Outcome::Fetched(_)) => {}
            Err(e) => self.status = Status::Failed(kind, format!("{e:#}")),
        }
        true
    }

    /// The stream for a track arrived: play it, keep the queue position, and
    /// clear the buffering note.
    fn finish_stream(&mut self, track: Track, position: usize, result: anyhow::Result<Outcome>) -> bool {
        // The user may have moved on; a stale stream must not start playing.
        if self.queue_pos != Some(position) || self.current_track().is_none_or(|t| t.id != track.id) {
            return true;
        }
        let source = match result {
            Ok(Outcome::Stream(source)) => source,
            Ok(_) => None,
            Err(e) => {
                self.player.stop();
                self.status = Status::StreamFailed(format!("{e:#}"));
                return true;
            }
        };
        self.player.play(&track, source.as_ref().map(|s| s.url.as_str()));
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
        // Restoring a session resumes paused at the saved second.
        if let Some((at_pos, at)) = self.pending_resume.take()
            && at_pos == position
        {
            self.player.seek(at);
            self.player.toggle_pause();
            self.status = Status::SessionRestored(track.title.clone());
        } else {
            self.status = Status::Playing { artist: track.artist.clone(), title: track.title.clone() };
        }
        self.wave_history.clear();
        self.load_lyrics();
        true
    }

    /// Lyrics for `track_id` arrived; a failure is not worth interrupting the
    /// music, so it simply means "no lyrics".
    fn finish_lyrics(&mut self, track_id: String, result: anyhow::Result<Outcome>) -> bool {
        if self.current_track().is_none_or(|t| t.id != track_id) {
            return true;
        }
        let lyrics = match result {
            Ok(Outcome::Lyrics(l)) => l,
            _ => None,
        };
        self.lyrics = Some(LyricsView { track_id, lyrics, scroll: -1.0, manual: None });
        true
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
        if self.net.loading_stream() && self.current_track().is_some_and(|t| t.id == id) {
            return;
        }
        let pos = tracks.iter().position(|t| t.id == id);
        self.queue = tracks;
        self.play_at(pos);
    }

    fn play_at(&mut self, pos: Option<usize>) {
        self.net.cancel_playback();
        self.pending_resume = None;
        self.pending_quality = None;
        self.player.stop();
        self.lyrics = None;
        self.quality = None;
        self.queue_pos = pos;
        match pos.and_then(|i| self.queue.get(i)).cloned() {
            Some(track) => {
                // A saved copy plays from disk: instant and offline.
                let local = self.library.local_stream(&track.id);
                self.playing_local = local.is_some();
                if let Some(source) = local {
                    // A saved copy plays at once: no request, no buffering note.
                    self.finish_stream(track, pos.unwrap_or(0), Ok(Outcome::Stream(Some(source))));
                    return;
                }
                // Otherwise the link comes from the worker; the card says
                // "buffering" until it lands.
                let position = pos.unwrap_or(0);
                self.net.post(Job::Stream { track: track.clone(), quality: self.quality_pref, position });
                self.wave_history.clear();
                // Nothing fills a list here: the card carries the title and the
                // footer just needs to say that a stream is on its way.
                self.status = Status::Loading(ListKind::Overview);
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
            self.net.cancel_sync();
            self.status = Status::SyncCancelled;
            return;
        }
        if !self.library.enabled() {
            self.status = Status::LibraryDisabled;
            return;
        }
        if self.net.loading_sync() {
            self.net.cancel_sync();
            self.status = Status::SyncCancelled;
        } else {
            self.net.post(Job::SyncLikes);
            self.status = Status::Loading(ListKind::Likes);
        }
    }

    fn finish_sync_stream(&mut self, track: Track, result: anyhow::Result<Outcome>) -> bool {
        // Cancelled syncs never start new downloads, even if a link is late.
        let Some(sync) = self.sync.as_mut() else { return true };
        match result {
            Ok(Outcome::Fetched(Some(source))) if self.library.start(&track, &source).is_ok() => {}
            _ => sync.mark_failed(&track.id),
        }
        true
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
        // The link comes from the worker; the file is fetched by the library.
        self.status = Status::Saving(track.title.clone());
        self.net.post(Job::SaveStream { track, quality: self.quality_pref });
    }

    /// The link for saving arrived: start the download and report it.
    fn finish_save_stream(&mut self, track: Track, result: anyhow::Result<Outcome>) -> bool {
        let source = match result {
            Ok(Outcome::Fetched(Some(s))) => s,
            Ok(_) => {
                self.status = Status::SaveError(self.lang.t().no_audio_to_save.into());
                return true;
            }
            Err(e) => {
                self.status = Status::SaveError(format!("{e:#}"));
                return true;
            }
        };
        self.status = match self.library.start(&track, &source) {
            Ok(()) => Status::Saving(track.title),
            Err(e) => Status::SaveError(format!("{e:#}")),
        };
        true
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
            position_secs: self.pending_resume
                .filter(|(position, _)| Some(*position) == self.queue_pos)
                .map_or_else(|| self.player.position().as_secs_f64(), |(_, at)| at.as_secs_f64()),
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
        self.queue_pos = pos;
        // Sessions written by older versions may contain only the first 300
        // favourites. Keep them visible until the full list arrives, but leave
        // the playback queue intact so restoring does not change the track.
        if self.list == ListKind::Likes {
            self.await_job(Job::Likes);
        }
        let Some(i) = pos else { return };
        let track = self.queue[i].clone();
        let saved_at = if s.position_secs.is_finite() { s.position_secs.max(0.0) } else { 0.0 };
        let at = Duration::from_secs_f64(saved_at.min(track.duration().as_secs_f64()));
        let at = if at + Duration::from_secs(3) < track.duration() { at } else { Duration::ZERO };
        self.net.cancel_playback();
        self.player.stop();
        self.pending_quality = None;
        self.pending_resume = Some((i, at));
        self.playing_local = false;
        if let Some(source) = self.library.local_stream(&track.id) {
            self.playing_local = true;
            self.finish_stream(track, i, Ok(Outcome::Stream(Some(source))));
        } else {
            self.net.post(Job::Stream { track, quality: self.quality_pref, position: i });
            self.status = Status::Loading(ListKind::Overview);
        }
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
        // Remember where we were, so the new stream resumes in place.
        self.pending_quality = Some((track.id.clone(), pos, paused));
        self.net.post(Job::QualityStream { track, quality: self.quality_pref });
    }

    /// The stream in the new tier arrived: swap it in without losing the spot.
    fn finish_quality_stream(&mut self, track: Track, result: anyhow::Result<Outcome>) -> bool {
        let Some((id, _pos, _paused)) = self.pending_quality.take() else { return true };
        if id != track.id || self.current_track().is_none_or(|t| t.id != track.id) {
            return true; // the user moved on while this was in flight
        }
        match result {
            Ok(Outcome::Fetched(Some(src))) => {
                let pos = self.player.position();
                let paused = self.player.state() == PlayerState::Paused;
                self.player.play(&track, Some(&src.url));
                self.player.seek(pos);
                if paused {
                    self.player.toggle_pause();
                }
                self.set_quality_from(Some(&src));
            }
            Ok(_) => {}
            Err(e) => self.status = Status::StreamFailed(format!("{e:#}")),
        }
        true
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
        if self.net.liking(&track.id) { return; }
        let liked = !track.liked;
        // Flip the mark now and reconcile when the answer lands: a like is a
        // small action and waiting for the network would make it feel broken.
        self.apply_like(&track.id, liked);
        self.status = if liked { Status::Liked(track.title) } else { Status::Unliked(track.title) };
        self.net.post(Job::Like { track_id: track.id, liked });
    }

    /// Sets the like mark everywhere the track appears (queue and any list).
    fn apply_like(&mut self, track_id: &str, liked: bool) {
        let update = |t: &mut Track| {
            if t.id == track_id {
                t.liked = liked;
            }
        };
        self.queue.iter_mut().for_each(update);
        self.entries.iter_mut().for_each(|e| {
            if let Entry::Track(t) = e {
                update(t);
            }
        });
    }

    /// The like request came back: on failure, put the mark back as it was and
    /// say why, so the screen never claims something the server refused.
    fn finish_like(&mut self, track_id: String, liked: bool, result: anyhow::Result<Outcome>) -> bool {
        if let Err(e) = result {
            self.apply_like(&track_id, !liked);
            self.status = Status::LikeFailed(format!("{e:#}"));
        }
        true
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
        self.await_job(Job::Artist { track, all });
    }

    fn load_lyrics(&mut self) {
        let Some(track) = self.current_track() else { return };
        if self.lyrics.as_ref().is_some_and(|l| l.track_id == track.id) {
            return;
        }
        let track_id = track.id.clone();
        self.net.post(Job::Lyrics { track_id });
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
        while self.collect_job() {}
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
            let slots = sync.free_slots(&self.library);
            for track in sync.next_batch(&self.library, slots) {
                self.net.post(Job::SyncStream { track, quality: self.quality_pref });
            }
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

    /// Presses a key and waits for whatever the catalog was asked for: the
    /// worker answers off-thread, and the UI only collects on a tick.
    fn press_wait(app: &mut App, code: KeyCode) {
        press(app, code);
        settle(app);
    }

    /// Ticks until the worker's answer has landed, or the wait runs out.
    fn settle(app: &mut App) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while app.net.is_busy() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(2));
            app.tick();
        }
        app.tick();
        assert!(!app.net.is_busy(), "worker did not finish before the deadline");
    }

    fn type_str(app: &mut App, s: &str) {
        s.chars().for_each(|c| press(app, KeyCode::Char(c)));
    }

    /// The whole point of 1.1.0: opening a section must return at once, even
    /// when the catalogue is stuck, and the screen must keep answering.
    #[test]
    fn opening_a_section_does_not_wait_for_the_catalogue() {
        use crate::testing::FakeService;
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);

        let started = std::time::Instant::now();
        press(&mut a, KeyCode::Char('w'));
        assert!(started.elapsed() < std::time::Duration::from_millis(50), "the key press returns immediately");
        assert!(matches!(a.status, Status::Loading(ListKind::Wave)), "the footer says buffering: {:?}", a.status);
        assert!(a.net.is_busy());

        // Frames keep coming while the request is stuck.
        for _ in 0..10 {
            a.tick();
        }
        assert!(matches!(a.status, Status::Loading(ListKind::Wave)), "still waiting, and still drawing");

        gate.release();
        settle(&mut a);
        assert_eq!(a.list, ListKind::Wave);
        assert!(!a.entries.is_empty(), "the wave arrived");
        assert!(matches!(a.status, Status::Loaded(ListKind::Wave, _)));
    }

    /// Moving on before an answer arrives must not paint the old list.
    #[test]
    fn a_superseded_section_never_lands() {
        use crate::testing::FakeService;
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);

        press(&mut a, KeyCode::Char('w')); // wave: queued first
        press(&mut a, KeyCode::Char('l')); // likes: what the user actually wants
        gate.release();
        settle(&mut a);

        assert_eq!(a.list, ListKind::Likes, "the newer request wins");
        assert!(matches!(a.status, Status::Loaded(ListKind::Likes, _)), "no stale wave answer: {:?}", a.status);
    }

    #[test]
    fn home_cancels_a_pending_list() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press(&mut a, KeyCode::Char('w'));
        a.open_section(Section::Home);
        gate.release();
        settle(&mut a);
        assert_eq!(a.list, ListKind::Overview);
        assert_eq!(a.status, Status::Home);
        assert!(a.entries.is_empty());
    }

    #[test]
    fn navigating_during_stream_loading_does_not_discard_the_stream() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('w'));
        gate.hold();
        press(&mut a, KeyCode::Enter);
        let id = a.current_track().unwrap().id.clone();
        assert!(a.net.loading_stream());
        assert_eq!(a.player.state(), PlayerState::Stopped);
        press(&mut a, KeyCode::Enter); // repeated activation must not replace the pending stream
        press(&mut a, KeyCode::Char('l'));
        gate.release();
        settle(&mut a);
        assert_eq!(a.current_track().unwrap().id, id);
        assert_eq!(a.player.state(), PlayerState::Playing);
        assert_eq!(a.list, ListKind::Likes);
        assert_eq!(a.lyrics.as_ref().unwrap().track_id, id);
    }

    #[test]
    fn a_failed_like_rolls_back_even_when_a_list_is_requested() {
        let fake = FakeService::parked().reject_likes();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let id = a.current_track().unwrap().id.clone();
        let was = a.current_track().unwrap().liked;
        gate.hold();
        press(&mut a, KeyCode::Char('L'));
        assert_eq!(a.current_track().unwrap().liked, !was, "immediate optimistic response");
        press(&mut a, KeyCode::Char('p'));
        gate.release();
        settle(&mut a);
        assert_eq!(a.current_track().unwrap().liked, was, "failure must still be delivered");
        assert_eq!(a.current_track().unwrap().id, id);
        assert_eq!(a.list, ListKind::Playlists);
    }

    #[test]
    fn restored_zero_and_near_end_positions_stay_paused() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let mut saved = a.snapshot();
        for position in [0.0, a.current_track().unwrap().duration_secs as f64 - 1.0] {
            saved.position_secs = position;
            let mut b = app();
            b.restore(serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap());
            settle(&mut b);
            assert_eq!(b.player.state(), PlayerState::Paused);
            assert_eq!(b.player.position().as_secs(), 0);
        }
    }

    #[test]
    fn restored_pause_is_applied_before_requesting_lyrics() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let saved = a.snapshot();
        let mut b = app();
        b.queue = saved.queue.clone();
        b.queue_pos = saved.queue_pos;
        b.pending_resume = Some((saved.queue_pos.unwrap(), Duration::from_secs(42)));
        b.finish_stream(b.current_track().unwrap().clone(), saved.queue_pos.unwrap(), Ok(Outcome::Stream(None)));
        assert_eq!(b.player.state(), PlayerState::Paused);
        assert_eq!(b.player.position().as_secs(), 42);
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
        press_wait(&mut a, KeyCode::Char('f'));
        type_str(&mut a, "q");
        assert!(!a.should_quit, "'q' inside the prompt must not quit");
        press(&mut a, KeyCode::Backspace);
        type_str(&mut a, "neon");
        press_wait(&mut a, KeyCode::Enter);
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
        press_wait(&mut a, KeyCode::Enter);
        assert_eq!(a.entries.len(), 1);
        assert_eq!(a.list, ListKind::Search("елки".into()));
    }

    #[test]
    fn russian_layout_hotkeys() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('ц')); // w
        assert_eq!(a.list, ListKind::Wave);
        press_wait(&mut a, KeyCode::Enter);
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
        press_wait(&mut a, KeyCode::Char('w'));
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
        press_wait(&mut a, KeyCode::Char('w'));
        assert_eq!(a.list, ListKind::Overview);
        press(&mut a, KeyCode::Esc);
        assert!(!a.help && !a.should_quit);
    }

    #[test]
    fn play_next_prev_and_like() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        press(&mut a, KeyCode::Char('j'));
        press_wait(&mut a, KeyCode::Enter);
        assert_eq!(a.queue_pos, Some(1));
        assert_eq!(a.player.state(), PlayerState::Playing);

        press(&mut a, KeyCode::Char('n'));
        assert_eq!(a.queue_pos, Some(2));
        press(&mut a, KeyCode::Char('b'));
        press(&mut a, KeyCode::Char('b'));
        assert_eq!(a.queue_pos, Some(0));
        settle(&mut a);

        press(&mut a, KeyCode::Char(' '));
        assert_eq!(a.player.state(), PlayerState::Paused);

        let id = a.current_track().unwrap().id.clone();
        let was = a.current_track().unwrap().liked;
        press(&mut a, KeyCode::Char('L'));
        assert_eq!(a.current_track().unwrap().liked, !was);
        settle(&mut a);
        assert_eq!(a.service.likes().unwrap().iter().any(|t| t.id == id), !was);
    }

    #[test]
    fn playlist_opens_tracks_and_artist_search() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('p'));
        assert!(matches!(a.entries[0], Entry::Playlist(_)));
        press_wait(&mut a, KeyCode::Enter);
        assert_eq!(a.list, ListKind::Playlist("Ночной город".into()));
        press_wait(&mut a, KeyCode::Enter);
        press_wait(&mut a, KeyCode::Char('s'));
        assert_eq!(a.list, ListKind::Artist("Neon Harbor".into()));
        press_wait(&mut a, KeyCode::Char('S'));
        assert_eq!(a.list, ListKind::ArtistAll("Neon Harbor".into()));
        press_wait(&mut a, KeyCode::Char('Ы')); // Shift+S in the Russian layout
        assert_eq!(a.list, ListKind::ArtistAll("Neon Harbor".into()));
        press_wait(&mut a, KeyCode::Char('s'));
        assert_eq!(a.list, ListKind::Artist("Neon Harbor".into()));
        assert_eq!(a.entries.len(), 3);
    }

    #[test]
    fn lyrics_load_follow_and_scroll() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w')); // first wave track is "Static Bloom", which has LRC
        press_wait(&mut a, KeyCode::Enter);
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
        press_wait(&mut a, KeyCode::Char('w'));
        press(&mut a, KeyCode::Char('j')); // "Low Tide" has no lyrics
        press_wait(&mut a, KeyCode::Enter);
        assert!(a.lyrics.as_ref().unwrap().lyrics.is_none());
        assert_eq!(a.lyric_line(), None);
        press(&mut a, KeyCode::Char(']')); // harmless
        assert!(!a.lyrics_manual());
    }

    #[test]
    fn restored_favourites_refresh_a_truncated_saved_list() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        let full_count = a.entries.len();
        assert!(full_count > 1);
        press_wait(&mut a, KeyCode::Enter);
        let mut saved = a.snapshot();
        saved.entries.truncate(1);
        saved.queue.truncate(1);
        let track_id = saved.queue[0].id.clone();

        let mut b = app();
        b.restore(saved);
        assert_eq!(b.entries.len(), 1, "saved list stays visible during refresh");
        settle(&mut b);
        assert_eq!(b.entries.len(), full_count, "full favourites replace the saved subset");
        assert_eq!(b.queue.len(), 1, "refresh must not replace the playback queue");
        assert_eq!(b.current_track().unwrap().id, track_id);
        assert_eq!(b.player.state(), PlayerState::Paused);
        press(&mut b, KeyCode::Down);
        assert_eq!(b.selected, 1, "can navigate beyond the old saved subset");
        press_wait(&mut b, KeyCode::Enter);
        assert_eq!(b.queue.len(), full_count, "playing from the refreshed list creates a full queue");

        let mut saved = b.snapshot();
        saved.entries.truncate(1);
        saved.queue.clear();
        saved.queue_pos = None;
        let mut c = app();
        c.restore(saved);
        settle(&mut c);
        assert_eq!(c.entries.len(), full_count, "refresh also works without a playing track");
    }

    #[test]
    fn session_round_trip_resumes_paused_at_position() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('p'));
        press_wait(&mut a, KeyCode::Enter); // open "Ночной город"
        press(&mut a, KeyCode::Char('j'));
        press_wait(&mut a, KeyCode::Enter); // play its 2nd track
        press(&mut a, KeyCode::Char('r'));
        press(&mut a, KeyCode::Char('-'));
        a.player.seek(Duration::from_secs(42));
        let saved = a.snapshot();
        let json = serde_json::to_string(&saved).unwrap();

        let mut b = app();
        b.restore(serde_json::from_str(&json).unwrap());
        settle(&mut b); // the stream arrives off-thread
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
        press_wait(&mut a, KeyCode::Char('w'));
        press(&mut a, KeyCode::Char('D'));
        assert_eq!(a.status, Status::LibraryDisabled);
        let dir = std::env::temp_dir().join(format!("yamusiq-app-lib-{}", std::process::id()));
        a.library = Library::open(dir.clone());
        press_wait(&mut a, KeyCode::Char('В')); // Shift+D in the Russian layout
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
        let deadline = Instant::now() + Duration::from_secs(2);
        while a.net.loading_sync() && Instant::now() < deadline {
            std::thread::yield_now();
            a.collect_job();
        }
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
