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
use crate::radio::{self, EventKind};
use crate::likes;
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
    PlaylistLoop,
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
    TrackWave(String),
    Playlist(String),
    HomePlaylist(crate::home::Kind),
    Artist(String),
    ArtistAll(String),
}

impl ListKind {
    pub fn title(&self, lang: Lang) -> String {
        let t = lang.t();
        match self {
            Self::Overview => t.list_overview.into(),
            Self::Wave => t.list_wave.into(),
            Self::TrackWave(id) => fill(t.list_track_wave, &[id]),
            Self::Likes => t.list_likes.into(),
            Self::Cached => t.list_cached.into(),
            Self::Playlists => t.list_playlists.into(),
            Self::Search(q) => fill(t.list_search, &[q]),
            Self::Playlist(n) => fill(t.list_playlist, &[n]),
            Self::HomePlaylist(kind) => fill(t.list_playlist, &[kind.title(lang)]),
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
    LinkCopied,
    LinkFailed(String),
    RadioFailed(String),
    NeedTrack,
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
            Self::LinkCopied => t.st_link_copied.into(),
            Self::LinkFailed(e) => fill(t.st_link_failed, &[e]),
            Self::RadioFailed(e) => fill(t.st_radio_failed, &[e]),
            Self::NeedTrack => t.st_need_track.into(),
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

/// Which list a job will fill, for the footer and for the status line.
fn job_kind(job: &Job) -> ListKind {
    match job {
        Job::Wave => ListKind::Wave,
        Job::TrackWave { title, .. } => ListKind::TrackWave(title.clone()),
        Job::Likes | Job::LikesMore { .. } => ListKind::Likes,
        Job::Cached => ListKind::Cached,
        Job::Playlists => ListKind::Playlists,
        Job::HomePlaylists => ListKind::Overview,
        Job::PlaylistTracks { title, personal, .. } => personal.map_or_else(|| ListKind::Playlist(title.clone()), ListKind::HomePlaylist),
        Job::Search { query } => ListKind::Search(query.clone()),
        Job::Artist { track, all } => {
            let name = track.artist.split(", ").next().unwrap_or(&track.artist).to_string();
            if *all { ListKind::ArtistAll(name) } else { ListKind::Artist(name) }
        }
        // These do not fill a list; the status line describes them itself.
        Job::RadioMore { .. } | Job::RadioNext { .. } | Job::Feedback { .. } | Job::OpenTrack { .. } | Job::CopyLink { .. }
        | Job::SyncLikes | Job::TrackCover { .. }
        | Job::Stream { .. }
        | Job::Lyrics { .. }
        | Job::Like { .. }
        | Job::SaveStream { .. }
        | Job::QualityStream { .. }
        | Job::SyncStream { .. } => ListKind::Overview,
    }
}

/// Natural completion stops at the end without a loop; manual next still
/// advances (and wraps) even when the current track is set to repeat.
pub fn next_index(mode: PlaybackMode, len: usize, current: usize, manual: bool, rng: &mut Rng) -> Option<usize> {
    if len == 0 {
        return None;
    }
    match mode {
        PlaybackMode::All if !manual && current + 1 >= len => None,
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

pub struct LinkDialog {
    pub input: String,
    pub error: Option<String>,
    pub loading: bool,
}

struct LikesParent {
    entries: Vec<Entry>,
    selected: usize,
    cursor: Option<likes::Cursor>,
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
    pub link_dialog: Option<LinkDialog>,
    likes_list: Option<likes::Cursor>,
    likes_queue: Option<likes::Cursor>,
    next_likes_id: u64,
    likes_retry: std::collections::HashMap<u64, Instant>,
    likes_waiting: bool,
    likes_wait_manual: bool,
    radio_list: Option<radio::Session>,
    radio_list_view: u64,
    radio_initial_target: usize,
    radio_list_retry_at: Instant,
    radio: Option<radio::Session>,
    radio_generation: u64,
    radio_view_generation: Option<u64>,
    radio_waiting: bool,
    radio_cursor: Option<String>,
    radio_retry_at: Instant,
    radio_started_track: bool,
    radio_announced: bool,
    radio_heard: Duration,
    radio_since: Option<Instant>,
    feedback_sequence: u64,
    pub list: ListKind,
    pub entries: Vec<Entry>,
    pub home: crate::home::Home,
    pub selected: usize,
    playlist_parent: Option<(Vec<Entry>, usize)>,
    playlist_parent_kind: Option<ListKind>,
    artist_likes_parent: Option<LikesParent>,
    playlist_return_title: Option<String>,
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
    /// Experimental, transient TUI cover; never serialized into sessions.
    pub cover_preview: Option<(String, std::sync::Arc<crate::cover::Preview>)>,
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
    /// Space can request playback before the restored stream has arrived.
    restore_should_play: bool,
    /// A system Pause during buffering must survive until audio is ready.
    pause_when_ready: bool,
    play_requested_at: Option<Instant>,
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
            link_dialog: None,
            likes_list: None,
            likes_queue: None,
            next_likes_id: 1,
            likes_retry: std::collections::HashMap::new(),
            likes_waiting: false,
            likes_wait_manual: false,
            radio_list: None,
            radio_list_view: 0,
            radio_initial_target: 0,
            radio_list_retry_at: Instant::now(),
            radio: None,
            radio_generation: 0,
            radio_view_generation: None,
            radio_waiting: false,
            radio_cursor: None,
            radio_retry_at: Instant::now(),
            radio_started_track: false,
            radio_announced: false,
            radio_heard: Duration::ZERO,
            radio_since: None,
            feedback_sequence: 0,
            list: ListKind::Overview,
            entries: Vec::new(),
            home: crate::home::Home::default(),
            selected: 0,
            playlist_parent: None,
            playlist_parent_kind: None,
            artist_likes_parent: None,
            playlist_return_title: None,
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
            cover_preview: None,
            quality_pref: Quality::default(),
            lossless_missing: false,
            playing_local: false,
            library: Library::disabled(),
            pending_remove: None,
            pending_resume: None,
            restore_should_play: false,
            pause_when_ready: false,
            play_requested_at: None,
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

    /// System media commands bypass search/help/link prompts, never move the
    /// list cursor, and reuse the same queue/radio operations as the keyboard.
    pub fn on_media_command(&mut self, command: crate::media::Command) {
        use crate::media::{Command, State};
        if !self.media_snapshot().accepts(command) { return; }
        match command {
            Command::Seek(position) => {
                if let Some(track) = self.current_track() {
                    self.player.seek(position.min(track.duration()));
                    if let Some(view) = self.lyrics.as_mut() { view.manual = None; }
                }
            }
            Command::Next => { self.pause_when_ready = false; self.advance(true); }
            Command::Previous => { self.pause_when_ready = false; self.previous(); }
            Command::Play | Command::Pause | Command::Toggle => {
                let playing = self.media_state() == State::Playing;
                let play = match command {
                    Command::Play => true,
                    Command::Pause => false,
                    _ => !playing,
                };
                if self.pending_resume.is_some() && self.net.loading_stream() {
                    self.restore_should_play = play;
                } else if self.net.loading_stream() || self.likes_waiting || self.radio_waiting {
                    self.pause_when_ready = !play;
                } else if (play && self.player.state() != PlayerState::Playing)
                    || (!play && self.player.state() == PlayerState::Playing)
                {
                    self.toggle_pause();
                }
            }
        }
    }

    fn media_state(&self) -> crate::media::State {
        use crate::media::State;
        if self.pending_resume.is_some() && self.net.loading_stream() {
            return if self.restore_should_play { State::Playing } else { State::Paused };
        }
        if self.net.loading_stream() || self.likes_waiting || self.radio_waiting {
            return if self.pause_when_ready { State::Paused } else { State::Playing };
        }
        match self.player.state() {
            PlayerState::Playing => State::Playing,
            PlayerState::Paused => State::Paused,
            PlayerState::Stopped => State::Stopped,
        }
    }

    pub fn media_snapshot(&self) -> crate::media::Snapshot {
        let track = self.current_track();
        crate::media::Snapshot {
            track: track.map(|t| crate::media::Metadata {
                id: t.id.clone(), title: t.title.clone(), artist: t.artist.clone(),
                album: t.album.clone(), duration: t.duration(), cover_uri: t.cover_uri.clone(),
            }),
            state: self.media_state(),
            position: self.pending_resume.map(|(_, at)| at).unwrap_or_else(|| self.player.position()),
            can_play: track.is_some() || self.selected_track().is_some(),
            can_navigate: track.is_some(),
            can_seek: track.is_some_and(|t| !t.duration().is_zero())
                && self.player.state() != PlayerState::Stopped
                && !self.net.loading_stream() && !self.likes_waiting && !self.radio_waiting,
        }
    }

    pub fn on_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        let code = match key.code {
            KeyCode::Char(c) if self.query.is_none() && self.link_dialog.is_none() => KeyCode::Char(latin_key(c)),
            other => other,
        };
        if key.modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }
        if self.link_dialog.is_some() {
            self.on_link_key(code);
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
            KeyCode::Char('c') => {
                let mode = match self.mode {
                    PlaybackMode::All | PlaybackMode::Random => PlaybackMode::Repeat,
                    PlaybackMode::Repeat => PlaybackMode::PlaylistLoop,
                    PlaybackMode::PlaylistLoop => PlaybackMode::All,
                };
                self.set_mode(mode);
            }
            KeyCode::Char('a') => self.set_mode(PlaybackMode::All),
            KeyCode::Char('r') => self.set_mode(PlaybackMode::Random),
            KeyCode::Char('v') => self.visualizer = !self.visualizer,
            KeyCode::Char('y') => {
                self.show_lyrics = !self.show_lyrics;
                self.status = Status::Lyrics(self.show_lyrics);
            }
            KeyCode::Char('o') => self.cycle_quality(),
            KeyCode::Left if key.modifiers.contains(KeyModifiers::SHIFT) => self.back_to_parent(),
            KeyCode::Left if self.home_focused() => self.move_home(-1),
            KeyCode::Right if self.home_focused() => self.move_home(1),
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
                    Focus::Nav if !self.entries.is_empty() || self.list == ListKind::Overview => Focus::List,
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
            KeyCode::Char('W') => {
                if let Some(track) = self.action_track().cloned() {
                    self.await_job(Job::TrackWave { title: format!("{} — {}", track.artist, track.title), track_id: track.id });
                } else { self.status = Status::NeedTrack; }
            }
            KeyCode::Char('u') => self.copy_track_link(),
            KeyCode::Char('O') => {
                self.link_dialog = Some(LinkDialog { input: String::new(), error: None, loading: false });
            }
            KeyCode::Char('d') => self.open_section(Section::Cached),
            KeyCode::Char('l') => self.open_section(Section::Likes),
            KeyCode::Char('s') => self.artist_search(false),
            KeyCode::Char('S') => self.artist_search(true),
            _ => {}
        }
    }

    /// List actions refer to the highlighted track, not an unrelated playing one.
    fn action_track(&self) -> Option<&Track> {
        if self.focus == Focus::List { self.selected_track().or_else(|| self.current_track()) }
        else { self.current_track().or_else(|| self.selected_track()) }
    }

    fn copy_track_link(&mut self) {
        let Some(track) = self.action_track() else { self.status = Status::NeedTrack; return };
        match crate::links::track_url(track) {
            Ok(url) => { self.net.post(Job::CopyLink { url }); }
            Err(e) => self.status = Status::LinkFailed(e.to_string()),
        }
    }

    fn on_link_key(&mut self, code: KeyCode) {
        if code == KeyCode::Esc {
            self.net.cancel_open();
            self.link_dialog = None;
            return;
        }
        let Some(dialog) = self.link_dialog.as_mut() else { return };
        match code {
            KeyCode::Enter if !dialog.loading => {
                match crate::links::track_id(&dialog.input) {
                    Ok(id) => {
                        dialog.loading = true;
                        dialog.error = None;
                        self.net.post(Job::OpenTrack { id });
                    }
                    Err(_) => dialog.error = Some(self.lang.t().link_invalid.into()),
                }
            }
            KeyCode::Backspace | KeyCode::Char(_) => {
                self.net.cancel_open();
                dialog.loading = false;
                dialog.error = None;
                match code {
                    KeyCode::Backspace => { dialog.input.pop(); }
                    KeyCode::Char(c) if !c.is_control() && dialog.input.len() < 2048 => dialog.input.push(c),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    pub fn on_paste(&mut self, text: &str) {
        if self.link_dialog.is_none() && self.query.is_none() { return; }
        for c in text.chars().filter(|c| !c.is_control()).take(2048) {
            if self.link_dialog.is_some() { self.on_link_key(KeyCode::Char(c)); }
            else if let Some(query) = self.query.as_mut() { query.push(c); }
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

    fn home_focused(&self) -> bool {
        self.focus == Focus::List && self.list == ListKind::Overview
    }

    fn move_home(&mut self, delta: isize) {
        self.selected = self.selected.saturating_add_signed(delta).min(2);
    }

    pub fn playlist_parent_is_home(&self) -> bool {
        matches!(self.list, ListKind::HomePlaylist(_)) || self.playlist_parent_kind == Some(ListKind::Overview)
    }

    fn move_selection(&mut self, delta: isize) {
        if self.home_focused() { return; }

        let (cursor, len) = match self.focus {
            Focus::Nav => (&mut self.section, Section::ALL.len()),
            Focus::List => (&mut self.selected, self.entries.len()),
        };
        if len == 0 {
            return;
        }
        *cursor = cursor.saturating_add_signed(delta).min(len - 1);
        if self.focus == Focus::List && self.list == ListKind::Likes
            && self.selected.saturating_add(likes::PREFETCH + 1) >= self.entries.len()
            && let Some(cursor) = self.likes_list.clone()
        { self.request_likes_page(cursor); }
        if self.focus == Focus::List && self.selected >= self.entries.len().saturating_sub(2) {
            self.request_radio_list_more();
        }
    }

    fn activate(&mut self) {
        if self.home_focused() {
            if let Some(card) = self.home.playlist(self.selected) {
                let playlist = card.playlist.clone();
                let personal = Some(card.kind);
                self.playlist_parent = Some((Vec::new(), self.selected));
                self.playlist_parent_kind = Some(ListKind::Overview);
                self.await_job(Job::PlaylistTracks { id: playlist.id, title: playlist.title, personal });
            } else if self.home.failed {
                self.open_section(Section::Home);
                self.focus = Focus::List;
            }
            return;
        }
        match self.focus {
            Focus::Nav => self.open_section(Section::ALL[self.section]),
            Focus::List => match self.entries.get(self.selected).cloned() {
                Some(Entry::Playlist(p)) => {
                    self.playlist_parent = Some((self.entries.clone(), self.selected));
                    self.playlist_parent_kind = Some(self.list.clone());
                    self.await_job(Job::PlaylistTracks { id: p.id.clone(), title: p.title.clone(), personal: None });
                }
                Some(Entry::Track(_)) => self.play_from_list(),
                None => {}
            },
        }
    }

    pub fn can_back_to_likes(&self) -> bool {
        self.artist_likes_parent.is_some()
    }

    fn back_to_parent(&mut self) {
        if let Some(parent) = self.artist_likes_parent.take() {
            self.net.cancel_list();
            self.set_entries(ListKind::Likes, parent.entries);
            self.likes_list = parent.cursor;
            self.selected = parent.selected.min(self.entries.len().saturating_sub(1));
            self.section = Section::ALL.iter().position(|s| *s == Section::Likes).unwrap();
            self.status = Status::Loaded(ListKind::Likes, self.entries.len());
            return;
        }
        self.back_to_playlists();
    }

    fn back_to_playlists(&mut self) {
        if let ListKind::HomePlaylist(kind) = self.list {
            let fallback = crate::home::Kind::ALL.iter().position(|k| *k == kind).unwrap_or(0);
            let selected = self.playlist_parent.take().map_or(fallback, |(_, selected)| selected);
            self.playlist_parent_kind = None;
            self.open_section(Section::Home);
            self.selected = selected.min(2);
            self.focus = Focus::List;
            return;
        }
        let ListKind::Playlist(title) = &self.list else { return };
        self.net.cancel_list();
        if let Some((entries, selected)) = self.playlist_parent.take() {
            if self.playlist_parent_kind.take() == Some(ListKind::Overview) {
                self.open_section(Section::Home);
                self.selected = selected.min(2);
                self.focus = Focus::List;
                return;
            }
            self.set_entries(ListKind::Playlists, entries);
            self.selected = selected.min(self.entries.len().saturating_sub(1));
            self.status = Status::Loaded(ListKind::Playlists, self.entries.len());
        } else {
            // Legacy sessions have no cached parent; fetch it and select the
            // playlist we came from rather than jumping to the first one.
            self.playlist_return_title = Some(title.clone());
            self.await_job(Job::Playlists);
        }
        self.section = Section::ALL.iter().position(|s| *s == Section::Playlists).unwrap();
    }

    fn open_section(&mut self, section: Section) {
        if let Some(i) = Section::ALL.iter().position(|s| *s == section) {
            self.section = i;
        }
        self.playlist_return_title = None;
        // Everything below is a request: queue it and let the frame keep coming.
        let job = match section {
            Section::Home => {
                self.net.cancel_list();
                self.set_entries(ListKind::Overview, Vec::new());
                self.home.loading = false;
                self.home.failed = false;
                self.status = Status::Home;
                self.focus = Focus::List;
                if !self.home.loaded {
                    self.home.loading = true;
                    self.await_job(Job::HomePlaylists);
                }
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
        if job != Job::HomePlaylists { self.home.loading = false; }
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
        if let Job::TrackCover { id } = &done.job {
            return self.finish_track_cover(id, done.result);
        }
        if let Job::Lyrics { track_id } = &done.job {
            return self.finish_lyrics(track_id.clone(), done.result);
        }
        if let Job::Like { track_id, liked } = &done.job {
            return self.finish_like(track_id.clone(), *liked, done.result);
        }
        if let Job::LikesMore { cursor } = &done.job {
            self.finish_likes_page(cursor, done.result);
            return true;
        }
        if let Job::RadioMore { view, .. } = &done.job {
            if *view == self.radio_list_view { self.finish_radio_list_more(done.result); }
            return true;
        }
        if let Job::RadioNext { generation, .. } = &done.job {
            if *generation == self.radio_generation { self.finish_radio_next(done.result); }
            return true;
        }
        if matches!(done.job, Job::Feedback { .. }) {
            if let Err(e) = done.result { self.status = Status::RadioFailed(e.to_string()); }
            return true;
        }
        if matches!(done.job, Job::CopyLink { .. }) {
            self.status = match done.result {
                Ok(_) => Status::LinkCopied,
                Err(e) => Status::LinkFailed(e.to_string()),
            };
            return true;
        }
        if matches!(done.job, Job::OpenTrack { .. }) {
            if self.link_dialog.is_none() { return true; }
            match done.result {
                Ok(Outcome::Track(Some(track))) => {
                    self.link_dialog = None;
                    self.net.cancel_list();
                    self.end_radio_track(EventKind::Skip);
                    self.detach_radio();
                    self.set_entries(ListKind::Search(track.title.clone()), vec![Entry::Track(track)]);
                    self.play_from_list();
                }
                result => {
                    let error = match result { Err(e) => e.to_string(), _ => self.lang.t().link_not_found.into() };
                    if let Some(dialog) = self.link_dialog.as_mut() { dialog.loading = false; dialog.error = Some(error); }
                }
            }
            return true;
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
        if done.job == Job::HomePlaylists {
            self.home.loading = false;
            match done.result {
                Ok(Outcome::HomePlaylists(playlists)) => {
                    self.home.finish(playlists);
                    self.status = Status::Home;
                }
                Err(e) => {
                    self.home.failed = true;
                    self.status = Status::Failed(ListKind::Overview, format!("{e:#}"));
                }
                _ => {}
            }
            return true;
        }
        match done.result {
            Ok(Outcome::LikesPage(page)) => {
                let Some(refs) = page.refs else { return true };
                let count = page.tracks.len();
                let cursor = likes::Cursor { id: self.next_likes_id, refs, next: page.next };
                self.next_likes_id = self.next_likes_id.wrapping_add(1);
                self.set_entries(ListKind::Likes, page.tracks.into_iter().map(Entry::Track).collect());
                self.likes_list = Some(cursor);
                self.status = Status::Loaded(ListKind::Likes, count);
            }
            Ok(Outcome::RadioWindow(window)) => {
                let count = window.tracks.len();
                let context = radio::Session { station: window.station, batch_ids: window.batch_ids };
                self.set_entries(kind.clone(), window.tracks.into_iter().map(Entry::Track).collect());
                self.radio_list = Some(context);
                self.radio_initial_target = if count > 0 { radio::INITIAL_TRACKS } else { 0 };
                self.status = Status::Loaded(kind, count);
                if matches!(done.job, Job::TrackWave { .. }) {
                    self.section = Section::ALL.iter().position(|s| *s == Section::Wave).unwrap();
                    if count > 0 { self.play_from_list(); }
                }
            }
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
                let selected = self.playlist_return_title.take().and_then(|title| ps.iter().position(|p| p.title == title));
                self.set_entries(kind, ps.into_iter().map(Entry::Playlist).collect());
                if let Some(selected) = selected { self.selected = selected; }
            }
            Ok(Outcome::HomePlaylists(_) | Outcome::Stream(_) | Outcome::Lyrics(_) | Outcome::Liked | Outcome::Fetched(_) | Outcome::Track(_) | Outcome::Ack | Outcome::Radio(_)) => {}
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
            if self.restore_should_play {
                self.status = Status::Playing { artist: track.artist.clone(), title: track.title.clone() };
            } else {
                self.player.toggle_pause();
                self.status = Status::SessionRestored(track.title.clone());
            }
            self.restore_should_play = false;
        } else if self.pause_when_ready {
            self.player.toggle_pause();
            self.status = Status::Paused;
        } else {
            self.status = Status::Playing { artist: track.artist.clone(), title: track.title.clone() };
        }
        self.pause_when_ready = false;
        if let Some(started) = self.play_requested_at.take() { crate::timing::record("playback.ready", started.elapsed()); }
        if self.player.state() == PlayerState::Playing { self.begin_radio_track(); }
        self.wave_history.clear();
        self.load_lyrics();
        if self.current_track().is_some_and(|t| t.cover_uri.is_none()) {
            self.net.post(Job::TrackCover { id: track.id.clone() });
        }
        true
    }

    fn finish_track_cover(&mut self, id: &str, result: anyhow::Result<Outcome>) -> bool {
        if self.current_track().is_none_or(|t| t.id != id) { return true; }
        if let Ok(Outcome::Track(Some(track))) = result
            && track.id == id && track.cover_uri.is_some()
        {
            for queued in &mut self.queue {
                if queued.id == id { queued.cover_uri = track.cover_uri.clone(); }
            }
            for entry in &mut self.entries {
                if let Entry::Track(t) = entry && t.id == id { t.cover_uri = track.cover_uri.clone(); }
            }
        }
        // Optional artwork failures never overwrite playback status.
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
        if !matches!(kind, ListKind::Artist(_) | ListKind::ArtistAll(_)) {
            self.artist_likes_parent = None;
        }
        self.list = kind;
        self.likes_list = None;
        self.retain_likes_pages();
        self.radio_list_view = self.radio_list_view.wrapping_add(1);
        self.radio_initial_target = 0;
        self.radio_list_retry_at = Instant::now();
        self.net.cancel_radio_more();
        self.radio_list = None;
        self.radio_view_generation = None;
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
        self.end_radio_track(EventKind::Skip);
        self.detach_radio();
        if matches!(self.list, ListKind::Wave | ListKind::TrackWave(_)) {
            self.radio = self.radio_list.clone();
            self.radio_view_generation = Some(self.radio_generation);
        }
        self.likes_queue = if self.list == ListKind::Likes { self.likes_list.clone() } else { None };
        self.likes_waiting = false;
        self.retain_likes_pages();
        self.queue = tracks;
        self.play_at(pos);
    }

    fn play_at(&mut self, pos: Option<usize>) {
        self.pause_when_ready &= self.likes_waiting || self.radio_waiting;
        self.play_requested_at = Some(Instant::now());
        self.likes_waiting = false;
        self.end_radio_track(EventKind::Skip);
        self.radio_heard = Duration::ZERO;
        self.radio_since = None;
        self.radio_waiting = false;
        self.net.cancel_playback();
        self.pending_resume = None;
        self.restore_should_play = false;
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
            playlist_parent: self.playlist_parent.clone(),
            playlist_parent_kind: self.playlist_parent_kind.clone(),
            section: self.section,
            focus: self.focus,
            queue: self.queue.clone(),
            queue_pos: self.queue_pos,
            position_secs: self.pending_resume
                .filter(|(position, _)| Some(*position) == self.queue_pos)
                .map_or_else(|| self.player.position().as_secs_f64(), |(_, at)| at.as_secs_f64()),
            radio: self.radio.clone(),
            radio_list: self.radio_list.clone(),
            likes_list: self.likes_list.clone(),
            likes_queue: self.likes_queue.clone(),
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
        self.playlist_parent = s.playlist_parent;
        self.playlist_parent_kind = s.playlist_parent_kind;
        // Older Home sessions stored a raw API title. Their cached Home card
        // index identifies the mix without translating user playlist names.
        if matches!(self.list, ListKind::Playlist(_))
            && self.playlist_parent_is_home()
            && let Some((_, index)) = &self.playlist_parent
            && let Some(kind) = crate::home::Kind::ALL.get(*index)
        {
            self.list = ListKind::HomePlaylist(*kind);
        }
        self.selected = s.selected.min(if self.list == ListKind::Overview { 2 } else { self.entries.len().saturating_sub(1) });
        self.section = s.section.min(Section::ALL.len() - 1);
        self.focus = if self.entries.is_empty() && self.list != ListKind::Overview { Focus::Nav } else { s.focus };
        self.queue = s.queue;
        self.likes_list = s.likes_list.filter(|c| self.list == ListKind::Likes && c.valid() && c.next >= self.entries.len());
        self.likes_queue = s.likes_queue.filter(|c| c.valid() && c.next >= self.queue.len());
        self.next_likes_id = self.likes_list.iter().chain(self.likes_queue.iter()).map(|c| c.id).max().unwrap_or(0).wrapping_add(1);
        self.radio = s.radio.filter(|r| r.batch_ids.len() == self.queue.len());
        self.radio_list = s.radio_list.filter(|r| r.batch_ids.len() == self.entries.len());
        if self.radio.is_some() && self.radio == self.radio_list { self.radio_view_generation = Some(self.radio_generation); }
        let pos = s.queue_pos.filter(|&i| i < self.queue.len());
        self.queue_pos = pos;
        // Sessions written by older versions may contain only the first 300
        // favourites. Keep them visible until the full list arrives, but leave
        // the playback queue intact so restoring does not change the track.
        if self.list == ListKind::Likes && self.likes_list.is_none() {
            self.await_job(Job::Likes);
        }
        if self.radio_list.is_none() && self.list == ListKind::Wave {
            self.await_job(Job::Wave);
        }
        let Some(i) = pos else { return };
        let track = self.queue[i].clone();
        let saved_at = if s.position_secs.is_finite() { s.position_secs.max(0.0) } else { 0.0 };
        let at = Duration::from_secs_f64(saved_at.min(track.duration().as_secs_f64()));
        let at = if at + Duration::from_secs(3) < track.duration() { at } else { Duration::ZERO };
        self.net.cancel_playback();
        self.player.stop();
        self.pending_quality = None;
        self.play_requested_at = Some(Instant::now());
        self.pending_resume = Some((i, at));
        self.restore_should_play = false;
        self.pause_when_ready = false;
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
        if self.likes_waiting { return; }
        if self.pending_resume.is_some() && self.net.loading_stream() {
            // The saved queue/position are already restored, but there is no
            // playable source yet. Do not restart the highlighted list row.
            self.restore_should_play = !self.restore_should_play;
            return;
        }
        if self.player.state() == PlayerState::Stopped && self.net.loading_stream() {
            return; // a normal track request is pending; preserve that queue too
        }
        if self.player.state() == PlayerState::Stopped {
            if self.selected_track().is_some() {
                self.play_from_list();
            }
            return;
        }
        self.update_radio_clock();
        self.player.toggle_pause();
        self.radio_since = None;
        if self.player.state() == PlayerState::Playing {
            if !self.radio_started_track { self.begin_radio_track(); }
            else { self.radio_since = Some(Instant::now()); }
        }
        self.status = match self.player.state() {
            PlayerState::Paused => Status::Paused,
            _ => Status::Resumed,
        };
    }

    fn advance(&mut self, manual: bool) {
        let Some(cur) = self.queue_pos else { return };
        self.end_radio_track(if manual { EventKind::Skip } else { EventKind::TrackFinished });
        if cur + 1 >= self.queue.len() && (manual || self.mode != PlaybackMode::Repeat)
            && let Some(cursor) = self.likes_queue.clone().filter(|c| c.has_more())
        {
            self.player.stop();
            self.likes_waiting = true;
            self.likes_wait_manual = manual;
            self.status = Status::Loading(ListKind::Likes);
            self.request_likes_page(cursor);
            return;
        }
        if self.radio.is_some() {
            if !manual && self.mode == PlaybackMode::Repeat { self.play_at(Some(cur)); }
            else if cur + 1 < self.queue.len() { self.play_at(Some(cur + 1)); }
            else {
                self.player.stop();
                self.radio_waiting = true;
                self.status = Status::Loading(ListKind::Wave);
                self.request_radio_next();
            }
        } else {
            let next = next_index(self.mode, self.queue.len(), cur, manual, &mut self.rng);
            self.play_at(next);
        }
    }

    fn previous(&mut self) {
        let Some(cur) = self.queue_pos else { return };
        self.play_at(prev_index(self.queue.len(), cur));
    }

    fn toggle_like(&mut self) {
        let source = self.selected_track().or_else(|| self.current_track());
        let Some(track) = source.cloned() else {
            self.status = Status::NothingToLike;
            return;
        };
        if self.net.liking(&track.id) { return; }
        if self.list == ListKind::Likes { self.net.cancel_likes_list(); }
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
        if let Some(parent) = self.artist_likes_parent.as_mut() {
            for entry in &mut parent.entries {
                if let Entry::Track(track) = entry { update(track); }
            }
        }
        self.entries.iter_mut().for_each(|e| {
            if let Entry::Track(t) = e {
                update(t);
            }
        });
    }

    /// The like request came back: on failure, put the mark back as it was and
    /// say why, so the screen never claims something the server refused.
    fn finish_like(&mut self, track_id: String, liked: bool, result: anyhow::Result<Outcome>) -> bool {
        match result {
            Ok(_) => self.apply_like(&track_id, liked),
            Err(e) => {
                self.apply_like(&track_id, !liked);
                self.status = Status::LikeFailed(format!("{e:#}"));
            }
        }
        true
    }

    /// `s` / `S` act on the selected track, never an unrelated playing one.
    /// Playback is only the fallback when the current list has no track row.
    fn artist_search(&mut self, all: bool) {
        let source = self.selected_track().or_else(|| self.current_track());
        let Some(track) = source.cloned() else {
            self.status = Status::ArtistNeedTrack;
            return;
        };
        if self.list == ListKind::Likes {
            self.artist_likes_parent = Some(LikesParent {
                entries: self.entries.clone(),
                selected: self.selected,
                cursor: self.likes_list.clone(),
            });
        }
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

    pub fn list_caption(&self) -> String {
        let title = self.list.title(self.lang);
        if self.list == ListKind::Likes && let Some(cursor) = &self.likes_list {
            return format!("{title} · {}", fill(self.lang.t().likes_loaded, &[&self.entries.len().to_string(), &cursor.refs.len().to_string()]));
        }
        title
    }

    fn retain_likes_pages(&mut self) {
        let ids: Vec<_> = self.likes_list.iter().chain(self.likes_queue.iter())
            .chain(self.artist_likes_parent.iter().filter_map(|p| p.cursor.as_ref())).map(|c| c.id).collect();
        self.net.retain_likes_pages(&ids);
        self.likes_retry.retain(|id, _| ids.contains(id));
    }

    fn request_likes_page(&mut self, cursor: likes::Cursor) {
        if !cursor.has_more() || self.net.loading_likes_page(cursor.id)
            || self.likes_retry.get(&cursor.id).is_some_and(|at| Instant::now() < *at)
        { return; }
        self.net.post(Job::LikesMore { cursor });
    }

    fn finish_likes_page(&mut self, request: &likes::Cursor, result: anyhow::Result<Outcome>) {
        let list_matches = self.likes_list.as_ref().is_some_and(|c| c.id == request.id && c.next == request.next);
        let queue_matches = self.likes_queue.as_ref().is_some_and(|c| c.id == request.id && c.next == request.next);
        let parent_matches = self.artist_likes_parent.as_ref().and_then(|p| p.cursor.as_ref())
            .is_some_and(|c| c.id == request.id && c.next == request.next);
        if !list_matches && !queue_matches && !parent_matches { return; }
        let page = match result {
            Ok(Outcome::LikesPage(page)) if page.next > request.next && page.next <= request.refs.len() => page,
            other => {
                self.likes_retry.insert(request.id, Instant::now() + Duration::from_secs(10));
                let error = match other { Err(e) => e.to_string(), _ => self.lang.t().likes_page_error.into() };
                self.status = Status::Failed(ListKind::Likes, error);
                return;
            }
        };
        self.likes_retry.remove(&request.id);
        let clear_error = matches!(&self.status, Status::Failed(ListKind::Likes, _));
        let old_queue_len = self.queue.len();
        if list_matches {
            self.entries.extend(page.tracks.iter().cloned().map(Entry::Track));
            if let Some(cursor) = self.likes_list.as_mut() { cursor.next = page.next; }
            if clear_error { self.status = Status::Loaded(ListKind::Likes, self.entries.len()); }
            if self.entries.is_empty() && page.next < request.refs.len() {
                if let Some(cursor) = self.likes_list.clone() { self.request_likes_page(cursor); }
            } else if self.entries.is_empty() { self.focus = Focus::Nav; }
        }
        if parent_matches && let Some(parent) = self.artist_likes_parent.as_mut() {
            parent.entries.extend(page.tracks.iter().cloned().map(Entry::Track));
            if let Some(cursor) = parent.cursor.as_mut() { cursor.next = page.next; }
        }
        if queue_matches {
            self.queue.extend(page.tracks);
            if let Some(cursor) = self.likes_queue.as_mut() { cursor.next = page.next; }
            if self.likes_waiting {
                if self.queue.len() > old_queue_len { self.play_at(Some(old_queue_len)); }
                else if page.next == request.refs.len() {
                    let next = self.queue_pos.and_then(|cur| next_index(self.mode, self.queue.len(), cur, self.likes_wait_manual, &mut self.rng));
                    self.play_at(next);
                }
            }
        }
    }

    fn request_radio_list_more(&mut self) { self.request_radio_list_more_with_status(true); }

    fn request_radio_list_more_with_status(&mut self, show_status: bool) {
        if !matches!(self.list, ListKind::Wave | ListKind::TrackWave(_))
            || self.net.loading_radio_more() || Instant::now() < self.radio_list_retry_at
        { return; }
        let (Some(radio), Some(Entry::Track(last))) = (&self.radio_list, self.entries.last()) else { return };
        let known = self.entries.iter().filter_map(|e| match e { Entry::Track(t) => Some(t.id.clone()), _ => None }).collect();
        let target = if self.entries.len() < self.radio_initial_target {
            (self.radio_initial_target - self.entries.len()).min(radio::MORE_TRACKS)
        } else { radio::MORE_TRACKS };
        self.net.post(Job::RadioMore { view: self.radio_list_view, station: radio.station.clone(), queue: last.id.clone(), known, target });
        if show_status { self.status = Status::Loading(self.list.clone()); }
    }

    fn finish_radio_list_more(&mut self, result: anyhow::Result<Outcome>) {
        let window = match result {
            Ok(Outcome::RadioWindow(window)) => window,
            other => {
                self.radio_list_retry_at = Instant::now() + Duration::from_secs(10);
                self.radio_initial_target = 0;
                self.status = Status::RadioFailed(match other { Err(e) => e.to_string(), _ => self.lang.t().radio_empty.into() });
                return;
            }
        };
        let Some(context) = self.radio_list.as_mut() else { return };
        if context.station != window.station { return; }
        let old_len = self.entries.len();
        let old_queue_len = self.queue.len();
        let mut seen: std::collections::HashSet<String> = self.entries.iter().filter_map(|e| match e { Entry::Track(t) => Some(t.id.clone()), _ => None }).collect();
        for (track, batch_id) in window.tracks.into_iter().zip(window.batch_ids) {
            if seen.insert(track.id.clone()) {
                self.entries.push(Entry::Track(track));
                context.batch_ids.push(batch_id);
            }
        }
        if self.entries.len() == old_len {
            self.radio_initial_target = 0;
            self.radio_list_retry_at = Instant::now() + Duration::from_secs(10);
            self.status = Status::RadioFailed(self.lang.t().radio_empty.into());
            return;
        }
        if matches!(&self.status, Status::Loading(kind) if *kind == self.list) {
            self.status = Status::Loaded(self.list.clone(), self.entries.len());
        }
        if self.radio_view_generation == Some(self.radio_generation) {
            self.queue.extend(self.entries[old_len..].iter().filter_map(|e| match e { Entry::Track(t) => Some(t.clone()), _ => None }));
            self.radio = Some(context.clone());
            if self.radio_waiting { self.play_at(Some(old_queue_len)); }
        }
    }

    pub fn buffering(&self) -> bool {
        self.net.loading_stream() || self.radio_waiting || self.likes_waiting
    }

    fn detach_radio(&mut self) {
        self.radio_generation = self.radio_generation.wrapping_add(1);
        self.net.cancel_radio();
        self.radio = None;
        self.radio_waiting = false;
        self.radio_cursor = None;
        self.radio_retry_at = Instant::now();
        self.radio_started_track = false;
        self.radio_announced = false;
        self.radio_since = None;
        self.radio_heard = Duration::ZERO;
    }

    fn update_radio_clock(&mut self) {
        if let Some(since) = self.radio_since.take()
            && self.player.state() == PlayerState::Playing
        {
            self.radio_heard += since.elapsed();
            if let Some(track) = self.current_track() { self.radio_heard = self.radio_heard.min(track.duration()); }
            self.radio_since = Some(Instant::now());
        }
    }

    fn send_radio_event(&mut self, kind: EventKind, track: bool) {
        let Some(radio) = &self.radio else { return };
        let Some(pos) = self.queue_pos else { return };
        let Some(batch_id) = radio.batch_ids.get(pos) else { return };
        let track_id = if track { self.current_track().map(|t| t.id.clone()) } else { None };
        self.feedback_sequence += 1;
        let event = radio::Feedback {
            station: radio.station.clone(), batch_id: batch_id.clone(), kind, track_id,
            played_seconds: self.radio_heard.as_secs_f64(),
            timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        };
        self.net.post(Job::Feedback { sequence: self.feedback_sequence, event });
    }

    fn begin_radio_track(&mut self) {
        if self.radio.is_none() || self.radio_started_track { return; }
        if !self.radio_announced {
            self.send_radio_event(EventKind::RadioStarted, false);
            self.radio_announced = true;
        }
        self.send_radio_event(EventKind::TrackStarted, true);
        self.radio_started_track = true;
        self.radio_since = Some(Instant::now());
    }

    fn end_radio_track(&mut self, kind: EventKind) {
        if !self.radio_started_track { return; }
        self.update_radio_clock();
        self.send_radio_event(kind, true);
        self.radio_cursor = self.current_track().map(|t| t.id.clone());
        self.radio_started_track = false;
        self.radio_since = None;
    }

    fn request_radio_next(&mut self) {
        if self.net.loading_radio() || Instant::now() < self.radio_retry_at || (!self.radio_started_track && !self.radio_waiting) { return; }
        let (Some(radio), Some(track)) = (&self.radio, self.current_track()) else { return };
        // Rotor advances after feedback for the preceding (finished/skipped)
        // track. Sending the new playing track here can repeat the old batch.
        if self.radio_cursor.is_none() && !self.radio_waiting && self.queue.len() > 1 { return; }
        let queue = self.radio_cursor.clone().unwrap_or_else(|| track.id.clone());
        self.net.post(Job::RadioNext { generation: self.radio_generation, station: radio.station.clone(), queue });
    }

    fn finish_radio_next(&mut self, result: anyhow::Result<Outcome>) {
        let batch = match result {
            Ok(Outcome::Radio(batch)) => batch,
            other => {
                self.radio_retry_at = Instant::now() + Duration::from_secs(10);
                self.status = Status::RadioFailed(match other { Err(e) => e.to_string(), _ => self.lang.t().radio_empty.into() });
                return;
            }
        };
        let Some(radio) = self.radio.as_mut() else { return };
        if radio.station != batch.station { return; }
        let old_len = self.queue.len();
        let mut seen: std::collections::HashSet<String> = self.queue.iter().rev().take(50).map(|t| t.id.clone()).collect();
        for track in batch.tracks {
            if seen.insert(track.id.clone()) {
                self.queue.push(track);
                radio.batch_ids.push(batch.batch_id.clone());
            }
        }
        if self.queue.len() == old_len {
            self.radio_retry_at = Instant::now() + Duration::from_secs(10);
            if self.radio_waiting { self.status = Status::RadioFailed(self.lang.t().radio_empty.into()); }
            return;
        }
        if self.radio_view_generation == Some(self.radio_generation) {
            self.entries.extend(self.queue[old_len..].iter().cloned().map(Entry::Track));
            self.radio_list = Some(radio.clone());
        }
        if self.radio_waiting { self.play_at(Some(old_len)); }
    }

    pub fn load_home(&mut self) {
        if self.list == ListKind::Overview && !self.home.loaded && !self.home.loading {
            self.home.loading = true;
            self.await_job(Job::HomePlaylists);
        }
    }

    pub fn tick(&mut self) {
        while self.collect_job() {}
        self.home.poll_covers();
        if self.list == ListKind::Likes
            && (self.entries.is_empty() || (self.focus == Focus::List && self.selected.saturating_add(likes::PREFETCH + 1) >= self.entries.len()))
            && let Some(cursor) = self.likes_list.clone()
        { self.request_likes_page(cursor); }
        if (self.likes_waiting || self.queue_pos.is_some_and(|i| i.saturating_add(likes::PREFETCH + 1) >= self.queue.len()))
            && let Some(cursor) = self.likes_queue.clone()
        { self.request_likes_page(cursor); }
        if self.entries.len() < self.radio_initial_target && !self.net.loading_stream() {
            self.request_radio_list_more_with_status(false);
        }
        self.update_radio_clock();
        if self.player.finished() {
            self.advance(false);
        }
        if self.radio.is_some() && self.queue_pos.is_some_and(|i| self.queue.len().saturating_sub(i + 1) <= 3) {
            self.request_radio_next();
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
    fn home_cards_open_and_return_to_the_same_card_including_saved_sessions() {
        let mut a = app();
        a.open_section(Section::Home);
        settle(&mut a);
        assert_eq!(a.home.playlists.len(), 3);
        assert_eq!(a.focus, Focus::List);
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Right);
        assert_eq!(a.selected, 2);
        press_wait(&mut a, KeyCode::Enter);
        assert_eq!(a.list, ListKind::HomePlaylist(crate::home::Kind::Stash));
        assert!(a.entries.iter().all(|e| matches!(e, Entry::Track(_))));
        assert!(!a.entries.is_empty());
        assert!(a.current_track().is_none(), "opening a card never autoplays");
        let session = serde_json::to_string(&a.snapshot()).unwrap();
        let mut restored = app();
        restored.restore(serde_json::from_str(&session).unwrap());
        for a in [&mut a, &mut restored] {
            a.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
            settle(a);
            assert_eq!(a.list, ListKind::Overview);
            assert_eq!(a.section, 0);
            assert_eq!(a.selected, 2);
            assert_eq!(a.focus, Focus::List);
            assert_eq!(a.home.playlists.len(), 3);
        }
    }

    #[test]
    fn personal_playlist_titles_translate_while_loading_loaded_and_restored() {
        use crate::home::Kind;
        let mut a = app();
        a.open_section(Section::Home);
        settle(&mut a);
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            a.lang = Lang::En;
            a.selected = index;
            press(&mut a, KeyCode::Enter);
            assert_eq!(a.status, Status::Loading(ListKind::HomePlaylist(kind)));
            assert!(a.status.text(Lang::En).contains(kind.title(Lang::En)));
            assert!(!a.status.text(Lang::En).contains(kind.title(Lang::Ru)));
            settle(&mut a);
            assert_eq!(a.list, ListKind::HomePlaylist(kind));
            let loaded = a.status.clone();
            assert!(loaded.text(Lang::En).contains(kind.title(Lang::En)));
            assert!(loaded.text(Lang::En).contains("tracks"));
            assert!(!loaded.text(Lang::En).contains(kind.title(Lang::Ru)));
            press(&mut a, KeyCode::Char('i'));
            assert_eq!(a.lang, Lang::Ru);
            assert!(a.list_caption().contains(kind.title(Lang::Ru)));
            assert!(loaded.text(a.lang).contains(kind.title(Lang::Ru)));
            press(&mut a, KeyCode::Char('i'));
            assert!(a.list_caption().contains(kind.title(Lang::En)));
            let json = serde_json::to_string(&a.snapshot()).unwrap();
            let mut restored = app();
            restored.lang = Lang::En;
            restored.restore(serde_json::from_str(&json).unwrap());
            assert_eq!(restored.list, ListKind::HomePlaylist(kind));
            assert!(restored.list_caption().contains(kind.title(Lang::En)));
            // Upgrade sessions written before Home kept a stable mix type.
            let mut legacy = a.snapshot();
            legacy.list = ListKind::Playlist(kind.title(Lang::Ru).into());
            restored.restore(legacy);
            assert_eq!(restored.list, ListKind::HomePlaylist(kind));
            assert!(restored.list_caption().contains(kind.title(Lang::En)));
            a.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
            settle(&mut a);
            assert_eq!(a.list, ListKind::Overview);
            assert_eq!(a.selected, index);
        }
        // Matching a generated playlist's name is not enough: ordinary user
        // playlists must never be translated or migrated based on title alone.
        a.list = ListKind::Playlist("Плейлист дня".into());
        a.playlist_parent_kind = Some(ListKind::Playlists);
        let mut restored = app();
        restored.lang = Lang::En;
        restored.restore(a.snapshot());
        assert_eq!(restored.list, ListKind::Playlist("Плейлист дня".into()));
        assert!(restored.list_caption().contains("Плейлист дня"));
    }

    #[test]
    fn home_arrows_change_cards_only_with_list_focus_and_never_seek() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        a.player.seek(Duration::from_secs(20));
        a.open_section(Section::Home);
        settle(&mut a);
        let queue = a.queue.clone();
        let at = a.player.position();
        press(&mut a, KeyCode::Right);
        assert_eq!(a.selected, 1);
        assert!(a.player.position().abs_diff(at) < Duration::from_secs(1));
        press(&mut a, KeyCode::Down);
        assert_eq!(a.selected, 1, "cards are horizontal");
        press(&mut a, KeyCode::Left);
        assert_eq!(a.selected, 0);
        press(&mut a, KeyCode::Left);
        assert_eq!(a.selected, 0);
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.focus, Focus::Nav);
        press(&mut a, KeyCode::Right);
        assert!(a.player.position() >= at + Duration::from_secs(4));
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.focus, Focus::List);
        assert_eq!(a.queue, queue);
    }

    #[test]
    fn cancelled_home_load_cannot_replace_a_newer_section() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        a.open_section(Section::Home);
        assert!(a.home.loading);
        press(&mut a, KeyCode::Right);
        press(&mut a, KeyCode::Enter);
        assert!(a.current_track().is_none());
        a.open_section(Section::Likes);
        gate.release();
        settle(&mut a);
        assert_eq!(a.list, ListKind::Likes);
        assert!(!a.home.loading);
        a.open_section(Section::Home);
        settle(&mut a);
        assert_eq!(a.home.playlists.len(), 3);
    }

    #[test]
    fn home_error_stops_loading_and_enter_retries_without_autoplay() {
        let fake = FakeService::default();
        let failed = fake.home_fail.clone();
        failed.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        a.open_section(Section::Home);
        settle(&mut a);
        assert!(a.home.failed && !a.home.loading);
        assert!(matches!(a.status, Status::Failed(ListKind::Overview, _)));
        for _ in 0..10 { a.tick(); }
        assert!(!a.net.is_busy(), "no automatic retry loop");
        failed.store(false, std::sync::atomic::Ordering::SeqCst);
        press_wait(&mut a, KeyCode::Enter);
        assert!(a.home.loaded && !a.home.failed);
        assert_eq!(a.home.playlists.len(), 3);
        assert!(a.queue.is_empty());
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
    fn favourites_load_hundreds_at_rows_90_190_290_without_cursor_reset() {
        let fake = FakeService::default().with_liked_tracks(350);
        let log = fake.likes_log.clone();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        assert_eq!(a.entries.len(), 100);
        assert_eq!(log.lock().unwrap().offsets, [0]);
        assert!(a.list_caption().contains("100/350"));
        a.selected = 88;
        a.tick();
        assert!(!a.net.is_busy());
        for (row, count) in [(89, 200), (189, 300), (289, 350)] {
            a.selected = row - 1;
            press_wait(&mut a, KeyCode::Down);
            assert_eq!(a.selected, row);
            assert_eq!(a.entries.len(), count);
        }
        assert_eq!(log.lock().unwrap().offsets, [0, 100, 200, 300]);
        let ids: Vec<_> = a.entries.iter().filter_map(|e| match e { Entry::Track(t) => Some(t.id.clone()), _ => None }).collect();
        assert_eq!(ids, (1..=350).map(|id| id.to_string()).collect::<Vec<_>>());
        a.selected = 349;
        press_wait(&mut a, KeyCode::Down);
        assert_eq!(log.lock().unwrap().offsets, [0, 100, 200, 300]);
    }

    #[test]
    fn playback_prefetches_favourites_without_moving_selection() {
        let fake = FakeService::default().with_liked_tracks(350);
        let log = fake.likes_log.clone();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        a.play_at(Some(88));
        settle(&mut a);
        assert_eq!(a.queue.len(), 100);
        press_wait(&mut a, KeyCode::Char('n'));
        assert_eq!(a.queue_pos, Some(89));
        assert_eq!(a.selected, 0);
        assert_eq!(a.queue.len(), 200);
        assert_eq!(a.entries.len(), 200);
        assert_eq!(log.lock().unwrap().offsets, [0, 100]);
    }

    #[test]
    fn end_of_loaded_favourites_waits_for_the_next_page_without_wrapping() {
        for mode in [PlaybackMode::All, PlaybackMode::PlaylistLoop] {
            let fake = FakeService::default().with_liked_tracks(350);
            let log = fake.likes_log.clone();
            let gate = crate::testing::Park::default();
            gate.hold();
            log.lock().unwrap().next_gate = Some(gate.clone());
            let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
            press_wait(&mut a, KeyCode::Char('l'));
            press_wait(&mut a, KeyCode::Enter);
            a.mode = mode;
            a.play_at(Some(99));
            let deadline = Instant::now() + Duration::from_millis(500);
            while (a.player.state() != PlayerState::Playing || gate.calls() == 0) && Instant::now() < deadline {
                a.tick(); std::thread::sleep(Duration::from_millis(2));
            }
            assert_eq!(a.player.state(), PlayerState::Playing);
            a.player.seek(a.current_track().unwrap().duration());
            a.tick();
            assert_eq!(a.queue_pos, Some(99));
            assert!(a.likes_waiting);
            assert!(a.buffering());
            for _ in 0..5 { a.tick(); }
            gate.release();
            settle(&mut a);
            assert_eq!(a.current_track().unwrap().id, "101");
            assert_eq!(a.queue.len(), 200);
            assert_eq!(a.queue_pos, Some(100));
            assert_eq!(a.player.state(), PlayerState::Playing);
            assert_eq!(log.lock().unwrap().offsets, [0, 100], "a pending page is never duplicated");
        }
    }

    #[test]
    fn unavailable_final_page_stops_or_loops_only_after_all_references_are_consumed() {
        for (mode, expected) in [(PlaybackMode::All, None), (PlaybackMode::PlaylistLoop, Some(0))] {
            let fake = FakeService::default().with_liked_tracks(150);
            let gate = crate::testing::Park::default();
            gate.hold();
            {
                let mut log = fake.likes_log.lock().unwrap();
                log.empty_at.insert(100);
                log.next_gate = Some(gate.clone());
            }
            let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
            press_wait(&mut a, KeyCode::Char('l'));
            press_wait(&mut a, KeyCode::Enter);
            a.mode = mode;
            a.play_at(Some(99));
            let deadline = Instant::now() + Duration::from_millis(500);
            while a.player.state() != PlayerState::Playing && Instant::now() < deadline { a.tick(); std::thread::sleep(Duration::from_millis(2)); }
            a.advance(false);
            assert!(a.likes_waiting);
            gate.release();
            settle(&mut a);
            assert_eq!(a.queue_pos, expected);
            assert!(!a.likes_waiting);
            assert!(!a.likes_queue.as_ref().unwrap().has_more());
        }
    }

    #[test]
    fn favourites_paging_continues_while_browsing_another_section() {
        let fake = FakeService::default().with_liked_tracks(350);
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        press_wait(&mut a, KeyCode::Char('p'));
        let playlists = a.entries.clone();
        a.play_at(Some(89));
        settle(&mut a);
        assert_eq!(a.queue.len(), 200);
        assert_eq!(a.entries, playlists);
        assert_eq!(a.list, ListKind::Playlists);
        let queue_id = a.likes_queue.as_ref().unwrap().id;
        press_wait(&mut a, KeyCode::Char('l'));
        assert_ne!(a.likes_list.as_ref().unwrap().id, queue_id);
        assert_eq!(a.entries.len(), 100);
        a.selected = 88;
        press_wait(&mut a, KeyCode::Down);
        assert_eq!(a.entries.len(), 200);
        assert_eq!(a.queue.len(), 200, "the newly opened snapshot must not duplicate the old queue");
        assert_eq!(a.current_track().unwrap().id, "90");
    }

    #[test]
    fn paged_favourites_restore_cursor_queue_and_playback_position() {
        let fake = FakeService::default().with_liked_tracks(350);
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        a.player.seek(Duration::from_secs(42));
        a.selected = 3;
        let fake = FakeService::default().with_liked_tracks(350);
        let log = fake.likes_log.clone();
        let mut b = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        b.restore(serde_json::from_str(&serde_json::to_string(&a.snapshot()).unwrap()).unwrap());
        settle(&mut b);
        assert!(log.lock().unwrap().offsets.is_empty(), "restored pages are not reloaded eagerly");
        assert_eq!(b.entries.len(), 100);
        assert_eq!(b.selected, 3);
        assert_eq!(b.current_track().unwrap().id, "1");
        assert_eq!(b.player.position().as_secs(), 42);
        assert_eq!(b.player.state(), PlayerState::Paused);
        b.selected = 88;
        press_wait(&mut b, KeyCode::Down);
        assert_eq!(b.entries.len(), 200);
        assert_eq!(b.queue.len(), 200);
        assert_eq!(log.lock().unwrap().offsets, [100]);
        assert_eq!(b.player.position().as_secs(), 42);
    }

    #[test]
    fn favourites_page_failure_retries_without_resetting_loaded_rows() {
        let fake = FakeService::default().with_liked_tracks(350);
        let log = fake.likes_log.clone();
        log.lock().unwrap().fail_next = true;
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        a.selected = 88;
        press_wait(&mut a, KeyCode::Down);
        assert_eq!(a.entries.len(), 100);
        assert_eq!(a.selected, 89);
        assert!(matches!(a.status, Status::Failed(ListKind::Likes, _)));
        let calls = log.lock().unwrap().offsets.len();
        for _ in 0..10 { a.tick(); }
        assert_eq!(log.lock().unwrap().offsets.len(), calls);
        log.lock().unwrap().fail_next = false;
        a.likes_retry.clear();
        a.tick();
        settle(&mut a);
        assert_eq!(a.entries.len(), 200);
        assert_eq!(a.selected, 89);
    }

    #[test]
    fn unavailable_favourites_pages_advance_by_references_not_row_count() {
        let fake = FakeService::default().with_liked_tracks(350);
        let log = fake.likes_log.clone();
        log.lock().unwrap().empty_at.insert(100);
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        a.selected = 88;
        press_wait(&mut a, KeyCode::Down);
        assert_eq!(log.lock().unwrap().offsets, [0, 100, 200]);
        assert_eq!(a.entries.len(), 200);
        assert_eq!(a.likes_list.as_ref().unwrap().next, 300);
        assert!(matches!(&a.entries[100], Entry::Track(t) if t.id == "201"));
    }

    #[test]
    fn like_edits_survive_pagination_and_only_reopening_removes_the_row() {
        let fake = FakeService::default().with_liked_tracks(350);
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Char('L'));
        assert!(!a.selected_track().unwrap().liked);
        press_wait(&mut a, KeyCode::Char('L'));
        assert!(a.selected_track().unwrap().liked);
        press_wait(&mut a, KeyCode::Char('L'));
        a.selected = 88;
        press_wait(&mut a, KeyCode::Down);
        assert_eq!(a.entries.len(), 200);
        assert!(matches!(&a.entries[0], Entry::Track(t) if t.id == "1" && !t.liked));
        press_wait(&mut a, KeyCode::Char('l'));
        assert_eq!(a.entries.len(), 100);
        assert_eq!(a.likes_list.as_ref().unwrap().refs.len(), 349);
        assert!(matches!(&a.entries[0], Entry::Track(t) if t.id == "2"));
    }

    #[test]
    fn unliking_keeps_favourites_rows_until_explicitly_reopened() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Down);
        let id = a.selected_track().unwrap().id.clone();
        let ids = |entries: &[Entry]| entries.iter().filter_map(|e| match e { Entry::Track(t) => Some(t.id.clone()), _ => None }).collect::<Vec<_>>();
        let rows = ids(&a.entries);
        let queue = a.queue.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        let current = a.current_track().unwrap().id.clone();
        let calls = gate.calls();
        gate.hold();
        press(&mut a, KeyCode::Char('L'));
        assert!(!a.selected_track().unwrap().liked, "heart disappears immediately on the selected row");
        assert_eq!(ids(&a.entries), rows);
        assert_eq!(a.selected, 1);
        gate.release();
        settle(&mut a);
        assert_eq!(gate.calls(), calls + 1, "only the like mutation, no catalogue reload");
        assert_eq!(ids(&a.entries), rows);
        assert_eq!(a.selected, 1);
        assert!(!a.selected_track().unwrap().liked);
        assert_eq!(a.queue.iter().map(|t| t.id.clone()).collect::<Vec<_>>(), queue);
        assert_eq!(a.current_track().unwrap().id, current);
        assert_eq!(a.player.state(), PlayerState::Playing);
        // Re-enter the section from the Library block, not by refreshing a row.
        press(&mut a, KeyCode::Tab);
        assert_eq!(a.focus, Focus::Nav);
        press_wait(&mut a, KeyCode::Enter);
        assert_eq!(a.entries.len(), rows.len() - 1);
        assert!(!ids(&a.entries).contains(&id));
        assert_eq!(a.current_track().unwrap().id, current);
    }

    #[test]
    fn an_inflight_favourites_snapshot_cannot_replace_an_optimistic_unlike() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press(&mut a, KeyCode::Down);
        let rows = a.entries.len();
        gate.hold();
        a.await_job(Job::Likes);
        press(&mut a, KeyCode::Char('L'));
        assert!(!a.selected_track().unwrap().liked);
        gate.release();
        settle(&mut a);
        assert_eq!(a.entries.len(), rows);
        assert_eq!(a.selected, 1, "late snapshot cannot reset the cursor");
        assert!(!a.selected_track().unwrap().liked, "late snapshot cannot restore the heart");
    }

    #[test]
    fn failed_unlike_restores_the_heart_without_reloading_favourites() {
        let fake = FakeService::parked().reject_likes();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press(&mut a, KeyCode::Down);
        let rows = a.entries.len();
        let calls = gate.calls();
        press_wait(&mut a, KeyCode::Char('L'));
        assert!(a.selected_track().unwrap().liked);
        assert_eq!(a.entries.len(), rows);
        assert_eq!(a.selected, 1);
        assert_eq!(gate.calls(), calls + 1);
        assert!(matches!(a.status, Status::LikeFailed(_)));
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
    fn space_during_restore_keeps_the_auto_advanced_track_and_position() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        // Auto-advance changes playback, not the user's highlighted row.
        a.player.seek(a.current_track().unwrap().duration());
        a.tick();
        settle(&mut a);
        assert_eq!(a.queue_pos, Some(1));
        assert_eq!(a.selected, 0);
        a.player.seek(Duration::from_secs(42));
        let saved = a.snapshot();
        let id = a.current_track().unwrap().id.clone();
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut b = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        b.restore(saved);
        press(&mut b, KeyCode::Char(' '));
        assert_eq!(b.current_track().unwrap().id, id, "Space must not play the old selected row");
        assert!(b.pending_resume.is_some(), "saved seek survives the pending request");
        gate.release();
        settle(&mut b);
        assert_eq!(b.current_track().unwrap().id, id);
        assert_eq!(b.player.state(), PlayerState::Playing);
        assert!((42..=43).contains(&b.player.position().as_secs()));
    }

    #[test]
    fn space_twice_during_restore_still_restores_paused_at_saved_position() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        a.player.seek(Duration::from_secs(55));
        let saved = a.snapshot();
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut b = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        b.restore(saved);
        press(&mut b, KeyCode::Char(' '));
        press(&mut b, KeyCode::Char(' '));
        gate.release();
        settle(&mut b);
        assert_eq!(b.player.state(), PlayerState::Paused);
        assert_eq!(b.player.position().as_secs(), 55);
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
    fn track_radio_starts_playing_before_the_fifteen_track_fill_finishes() {
        let fake = FakeService::default();
        let gate = crate::testing::Park::default();
        gate.hold();
        {
            let mut log = fake.radio.lock().unwrap();
            log.initial_size = Some(5);
            log.next_gate = Some(gate.clone());
        }
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press(&mut a, KeyCode::Char('W'));
        let deadline = Instant::now() + Duration::from_millis(500);
        while (a.player.state() != PlayerState::Playing || gate.calls() == 0) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(2)); a.tick();
        }
        let started = a.player.state() == PlayerState::Playing;
        let first_len = a.entries.len();
        let fetching = a.net.loading_radio_more();
        gate.release();
        settle(&mut a);
        assert!(started, "audio starts while subsequent radio batches are blocked");
        assert_eq!(first_len, 5, "first batch is already visible");
        assert!(fetching);
        assert_eq!(a.entries.len(), 15);
    }

    #[test]
    fn both_radio_lists_start_with_fifteen_and_load_five_at_the_penultimate_row() {
        for track_based in [false, true] {
            let fake = FakeService::default();
            let log = fake.radio.clone();
            log.lock().unwrap().initial_size = Some(5);
            let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
            if track_based {
                press_wait(&mut a, KeyCode::Char('l'));
                press_wait(&mut a, KeyCode::Char('W'));
            } else {
                // Browse ahead while an unrelated liked track is playing.
                press_wait(&mut a, KeyCode::Char('l'));
                press_wait(&mut a, KeyCode::Enter);
                press_wait(&mut a, KeyCode::Char('w'));
            }
            assert_eq!(a.entries.len(), 15);
            assert_eq!(a.radio_list.as_ref().unwrap().batch_ids.len(), 15);
            let pos = a.queue_pos;
            let current = a.current_track().unwrap().id.clone();
            let events = log.lock().unwrap().events.len();
            let calls = log.lock().unwrap().requests.len();
            for _ in 0..12 { press_wait(&mut a, KeyCode::Down); }
            assert_eq!(a.selected, 12);
            assert_eq!(log.lock().unwrap().requests.len(), calls, "not near the end yet");
            press_wait(&mut a, KeyCode::Down);
            assert_eq!(a.selected, 13, "pagination preserves the cursor");
            assert_eq!(a.entries.len(), 20, "penultimate row requests five more tracks");
            assert_eq!(a.radio_list.as_ref().unwrap().batch_ids.len(), 20);
            assert_eq!(a.current_track().unwrap().id, current);
            assert_eq!(a.queue_pos, pos);
            assert_eq!(a.player.state(), PlayerState::Playing);
            assert_eq!(log.lock().unwrap().events.len(), events, "browsing never sends playback feedback");
            let ids: std::collections::HashSet<_> = a.entries.iter().filter_map(|e| match e { Entry::Track(t) => Some(&t.id), _ => None }).collect();
            assert_eq!(ids.len(), a.entries.len(), "no duplicates");
            if track_based { assert_eq!(a.queue.len(), 20); }
            else { assert_ne!(a.queue.len(), 20, "unrelated playback queue is untouched"); }
            for _ in 0..5 { press_wait(&mut a, KeyCode::Down); }
            assert_eq!(a.selected, 18);
            assert_eq!(a.entries.len(), 25);
        }
    }

    #[test]
    fn repeated_radio_batches_do_not_create_fake_tracks_or_feedback() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        {
            let mut log = log.lock().unwrap();
            log.initial_size = Some(5);
            log.repeat_next = true;
        }
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('w'));
        assert_eq!(a.entries.len(), 5, "show the real available tracks, not repeated rows");
        assert_eq!(log.lock().unwrap().requests.len(), 3, "stop after two unchanged responses");
        for _ in 0..3 { press_wait(&mut a, KeyCode::Down); }
        assert_eq!(a.entries.len(), 5);
        let calls = log.lock().unwrap().requests.len();
        for _ in 0..10 { press(&mut a, KeyCode::Down); a.tick(); }
        assert_eq!(log.lock().unwrap().requests.len(), calls, "empty pages are throttled");
        assert!(log.lock().unwrap().events.is_empty());
    }

    #[test]
    fn pending_radio_list_pagination_is_not_duplicated_and_cannot_reopen_home() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('w'));
        a.selected = a.entries.len() - 3;
        gate.hold();
        press(&mut a, KeyCode::Down);
        assert!(a.net.loading_radio_more());
        let pending = a.net.is_busy();
        press(&mut a, KeyCode::Down);
        assert_eq!(a.net.is_busy(), pending);
        a.open_section(Section::Home);
        assert!(!a.net.loading_radio_more());
        // Put another request behind the cancelled one to wait until the old
        // response really has passed through the worker before asserting.
        press(&mut a, KeyCode::Char('O'));
        a.on_paste("1");
        press(&mut a, KeyCode::Enter);
        gate.release();
        settle(&mut a);
        assert!(!matches!(a.list, ListKind::Wave));
        assert_eq!(a.entries.len(), 1);
    }

    #[test]
    fn radio_prefetches_keeps_advancing_and_uses_original_batch_ids() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let first_len = a.queue.len();
        for _ in 0..first_len + 8 { press_wait(&mut a, KeyCode::Char('n')); }
        assert!(a.queue_pos.unwrap() >= first_len + 8, "radio never wraps to the first batch");
        assert!(a.queue.len() > first_len);
        assert_eq!(a.entries.len(), a.queue.len());
        let log = log.lock().unwrap();
        assert_eq!(log.events[0].kind, EventKind::RadioStarted);
        assert_eq!(log.events[1].kind, EventKind::TrackStarted);
        assert_eq!(log.events[2].kind, EventKind::Skip);
        assert_eq!(log.events[2].batch_id, "batch-1");
        assert!(log.events.iter().any(|e| e.kind == EventKind::TrackStarted && e.batch_id != "batch-1"));
        assert!(log.requests.iter().skip(1).all(|(_, queue)| queue.is_some()));
    }

    #[test]
    fn natural_radio_completion_reports_finished_and_not_skip() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let id = a.current_track().unwrap().id.clone();
        a.radio_since = Some(Instant::now() - Duration::from_secs(4));
        a.player.seek(a.current_track().unwrap().duration());
        a.tick();
        settle(&mut a);
        let log = log.lock().unwrap();
        let ended = log.events.iter().find(|e| e.kind == EventKind::TrackFinished).unwrap();
        assert_eq!(ended.track_id.as_deref(), Some(id.as_str()));
        assert!(ended.played_seconds >= 4.0 && ended.played_seconds < 5.0);
        assert!(!log.events.iter().any(|e| e.kind == EventKind::Skip));
    }

    #[test]
    fn radio_refill_does_not_replace_another_browsed_list() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        press_wait(&mut a, KeyCode::Char('l'));
        let liked = a.entries.clone();
        let n = a.queue.len();
        for _ in 0..n { press_wait(&mut a, KeyCode::Char('n')); }
        assert!(a.queue.len() > n);
        assert_eq!(a.list, ListKind::Likes);
        assert_eq!(a.entries, liked);
    }

    #[test]
    fn empty_radio_waits_without_looping_or_retrying_every_tick() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        log.lock().unwrap().empty_next = true;
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let last = a.queue.len() - 1;
        a.play_at(Some(last));
        settle(&mut a);
        a.advance(false);
        settle(&mut a);
        assert_eq!(a.queue_pos, Some(last));
        assert!(a.radio_waiting);
        assert_eq!(a.player.state(), PlayerState::Stopped);
        let calls = log.lock().unwrap().requests.len();
        for _ in 0..20 { a.tick(); }
        assert_eq!(log.lock().unwrap().requests.len(), calls);
        log.lock().unwrap().empty_next = false;
        a.radio_retry_at = Instant::now();
        a.tick();
        settle(&mut a);
        assert_eq!(a.queue_pos, Some(last + 1));
        assert_eq!(a.player.state(), PlayerState::Playing);
    }

    #[test]
    fn changing_playback_queue_discards_a_stale_radio_batch() {
        let fake = FakeService::parked();
        let liked = fake.likes().unwrap();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let generation = a.radio_generation;
        gate.hold();
        a.radio_cursor = a.current_track().map(|t| t.id.clone());
        a.request_radio_next();
        assert!(a.net.loading_radio());
        a.set_entries(ListKind::Likes, liked.into_iter().map(Entry::Track).collect());
        a.play_from_list();
        let queue = a.queue.clone();
        assert_ne!(a.radio_generation, generation);
        assert!(!a.net.loading_radio());
        gate.release();
        settle(&mut a);
        assert!(a.radio.is_none());
        assert_eq!(a.queue, queue, "late radio result cannot append into another queue");
        assert_eq!(a.player.state(), PlayerState::Playing);
    }

    #[test]
    fn failed_radio_refill_preserves_playback_and_retries_later() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        log.lock().unwrap().fail_next = true;
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let n = a.queue.len();
        a.play_at(Some(n - 2));
        settle(&mut a);
        assert_eq!(a.player.state(), PlayerState::Playing);
        assert_eq!(a.queue.len(), n);
        assert!(matches!(a.status, Status::RadioFailed(_)));
        let calls = log.lock().unwrap().requests.len();
        for _ in 0..10 { a.tick(); }
        assert_eq!(log.lock().unwrap().requests.len(), calls);
        log.lock().unwrap().fail_next = false;
        a.radio_retry_at = Instant::now();
        a.tick();
        settle(&mut a);
        assert!(a.queue.len() > n);
    }

    #[test]
    fn radio_listening_time_excludes_seek_and_pause() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        a.radio_since = Some(Instant::now() - Duration::from_secs(3));
        a.update_radio_clock();
        a.seek_by(100);
        press(&mut a, KeyCode::Char(' '));
        let heard = a.radio_heard;
        assert!(a.radio_since.is_none());
        a.tick();
        assert_eq!(a.radio_heard, heard);
        press_wait(&mut a, KeyCode::Char('n'));
        let log = log.lock().unwrap();
        let skip = log.events.iter().find(|e| e.kind == EventKind::Skip).unwrap();
        assert!(skip.played_seconds >= 3.0 && skip.played_seconds < 4.0);
    }

    #[test]
    fn track_wave_uses_the_selection_and_starts_playing() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Down);
        let selected = a.selected_track().unwrap().id.clone();
        assert_ne!(a.current_track().unwrap().id, selected);
        press_wait(&mut a, KeyCode::Char('W'));
        assert_eq!(log.lock().unwrap().requests[0].0, format!("track:{selected}"));
        assert!(matches!(a.list, ListKind::TrackWave(_)));
        assert_eq!(a.player.state(), PlayerState::Playing);
    }

    #[test]
    fn link_dialog_opens_shared_track_independently_of_radio() {
        let mut a = app();
        press(&mut a, KeyCode::Char('O'));
        a.on_paste("https://music.yandex.ru/album/99/track/4?from=share");
        press_wait(&mut a, KeyCode::Enter);
        assert!(a.link_dialog.is_none());
        assert_eq!(a.current_track().unwrap().id, "4");
        assert_eq!(a.queue.len(), 1);
        assert!(a.radio.is_none());
        assert_eq!(a.player.state(), PlayerState::Playing);
        press(&mut a, KeyCode::Char('O'));
        type_str(&mut a, "not a link");
        press(&mut a, KeyCode::Enter);
        assert!(a.link_dialog.as_ref().unwrap().error.is_some());
        assert!(!a.should_quit);
        press(&mut a, KeyCode::Esc);
        assert!(a.link_dialog.is_none());
        assert_eq!(a.current_track().unwrap().id, "4");
    }

    #[test]
    fn cancelled_track_lookup_cannot_start_playback() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        gate.hold();
        press(&mut a, KeyCode::Char('O'));
        a.on_paste("2");
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Esc);
        gate.release();
        settle(&mut a);
        assert!(a.current_track().is_none());
        assert!(a.link_dialog.is_none());
    }

    #[test]
    fn a_new_shared_link_supersedes_a_cancelled_lookup() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::En);
        gate.hold();
        press(&mut a, KeyCode::Char('O'));
        a.on_paste("2");
        press(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Esc);
        press(&mut a, KeyCode::Char('O'));
        a.on_paste("4");
        press(&mut a, KeyCode::Enter);
        gate.release();
        settle(&mut a);
        assert_eq!(a.current_track().unwrap().id, "4");
        assert!(a.link_dialog.is_none());
        assert_eq!(a.queue.len(), 1);
    }

    #[test]
    fn restored_radio_stays_paused_and_resumes_without_losing_context() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('w'));
        press_wait(&mut a, KeyCode::Enter);
        let fake = FakeService::default();
        let log = fake.radio.clone();
        let mut b = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        b.restore(serde_json::from_str(&serde_json::to_string(&a.snapshot()).unwrap()).unwrap());
        settle(&mut b);
        assert_eq!(b.radio, a.radio);
        assert_eq!(b.player.state(), PlayerState::Paused);
        assert!(log.lock().unwrap().events.is_empty(), "no listening feedback for a paused restore");
        press_wait(&mut b, KeyCode::Char(' '));
        assert_eq!(log.lock().unwrap().events[0].kind, EventKind::RadioStarted);
        assert_eq!(log.lock().unwrap().events[1].kind, EventKind::TrackStarted);
    }

    #[test]
    fn loop_key_cycles_modes_and_persists_them() {
        let mut a = app();
        for mode in [PlaybackMode::Repeat, PlaybackMode::PlaylistLoop, PlaybackMode::All] {
            press(&mut a, KeyCode::Char('c'));
            assert_eq!(a.mode, mode);
            let mut b = app();
            b.restore(serde_json::from_str(&serde_json::to_string(&a.snapshot()).unwrap()).unwrap());
            assert_eq!(b.mode, mode);
        }
        press(&mut a, KeyCode::Char('r'));
        assert_eq!(a.mode, PlaybackMode::Random);
        press(&mut a, KeyCode::Char('c'));
        assert_eq!(a.mode, PlaybackMode::Repeat);
        press(&mut a, KeyCode::Char('a'));
        assert_eq!(a.mode, PlaybackMode::All);
    }

    #[test]
    fn natural_completion_stops_or_repeats_the_queue() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        a.selected = a.entries.len() - 1;
        press_wait(&mut a, KeyCode::Enter);
        let last = a.queue_pos;
        a.set_mode(PlaybackMode::Repeat);
        a.advance(false);
        settle(&mut a);
        assert_eq!(a.queue_pos, last);
        a.set_mode(PlaybackMode::PlaylistLoop);
        a.advance(false);
        settle(&mut a);
        assert_eq!(a.queue_pos, Some(0));
        a.play_at(last);
        settle(&mut a);
        a.set_mode(PlaybackMode::All);
        a.advance(false);
        assert_eq!(a.player.state(), PlayerState::Stopped);
        assert_eq!(a.queue_pos, None);
    }

    #[test]
    fn shift_left_returns_to_the_selected_playlist_without_stopping() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('p'));
        assert!(a.entries.len() > 1);
        press(&mut a, KeyCode::Down);
        let parent = a.entries.clone();
        press_wait(&mut a, KeyCode::Enter);
        assert!(matches!(a.list, ListKind::Playlist(_)));
        press_wait(&mut a, KeyCode::Enter);
        let queue = a.queue.clone();
        let position = a.queue_pos;
        gate.hold();
        a.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
        assert_eq!(a.list, ListKind::Playlists);
        assert_eq!(a.entries, parent);
        assert_eq!(a.selected, 1);
        assert!(!a.net.is_busy(), "cached return does not fetch anything");
        assert_eq!(a.queue, queue);
        assert_eq!(a.queue_pos, position);
        assert_eq!(a.player.state(), PlayerState::Playing);
        gate.release();
    }

    #[test]
    fn restored_playlist_supports_back_navigation_including_legacy_sessions() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('p'));
        press(&mut a, KeyCode::Down);
        let parent = a.entries.clone();
        press_wait(&mut a, KeyCode::Enter);
        let saved = serde_json::to_value(a.snapshot()).unwrap();
        for legacy in [false, true] {
            let mut saved = saved.clone();
            if legacy { saved.as_object_mut().unwrap().remove("playlist_parent"); }
            let mut b = app();
            b.restore(serde_json::from_value(saved).unwrap());
            b.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
            settle(&mut b);
            assert_eq!(b.list, ListKind::Playlists);
            assert_eq!(b.entries, parent);
            assert_eq!(b.selected, 1);
        }
    }

    #[test]
    fn next_index_modes() {
        let mut rng = Rng::seeded(42);
        assert_eq!(next_index(PlaybackMode::All, 3, 2, false, &mut rng), None);
        assert_eq!(next_index(PlaybackMode::All, 3, 1, false, &mut rng), Some(2));
        assert_eq!(next_index(PlaybackMode::All, 3, 2, true, &mut rng), Some(0));
        assert_eq!(next_index(PlaybackMode::PlaylistLoop, 3, 2, false, &mut rng), Some(0));
        assert_eq!(next_index(PlaybackMode::PlaylistLoop, 1, 0, false, &mut rng), Some(0));
        assert_eq!(next_index(PlaybackMode::All, 1, 0, false, &mut rng), None);
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

        let id = a.selected_track().unwrap().id.clone();
        let was = a.selected_track().unwrap().liked;
        press(&mut a, KeyCode::Char('L'));
        assert_eq!(a.selected_track().unwrap().liked, !was);
        settle(&mut a);
        assert_eq!(a.service.likes().unwrap().iter().any(|t| t.id == id), !was);
    }

    #[test]
    fn like_shortcut_targets_the_highlight_in_every_track_list() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        let playing = a.current_track().unwrap().clone();
        for kind in [ListKind::Likes, ListKind::Wave, ListKind::Search("test".into()),
            ListKind::Artist("test".into()), ListKind::ArtistAll("test".into()),
            ListKind::Playlist("test".into()), ListKind::Cached, ListKind::TrackWave("test".into())]
        {
            press_wait(&mut a, KeyCode::Char('l'));
            a.list = kind;
            press(&mut a, KeyCode::Down);
            let selected = a.selected_track().unwrap().clone();
            assert_ne!(selected.id, playing.id);
            // Like targeting does not depend on which panel has focus.
            a.focus = Focus::Nav;
            let entries = a.entries.len();
            a.on_key(KeyEvent::new(KeyCode::Char('L'), KeyModifiers::SHIFT));
            assert_eq!(a.selected_track().unwrap().liked, !selected.liked);
            settle(&mut a);
            assert_eq!(a.selected_track().unwrap().liked, !selected.liked);
            assert_eq!(a.current_track().unwrap().liked, playing.liked);
            assert_eq!(a.current_track().unwrap().id, playing.id);
            assert_eq!(a.player.state(), PlayerState::Playing);
            assert_eq!(a.selected, 1);
            assert_eq!(a.entries.len(), entries);
            assert_eq!(a.service.likes().unwrap().iter().any(|t| t.id == selected.id), !selected.liked);
            // Toggling again acts on the same row.
            press_wait(&mut a, KeyCode::Char('L'));
            assert_eq!(a.selected_track().unwrap().liked, selected.liked);
        }
        press_wait(&mut a, KeyCode::Char('p'));
        assert!(a.selected_track().is_none());
        press_wait(&mut a, KeyCode::Char('L'));
        assert_eq!(a.current_track().unwrap().liked, !playing.liked);
    }

    #[test]
    fn cover_backfill_does_not_change_playback_or_apply_to_a_new_track() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        let mut first = a.current_track().unwrap().clone();
        let status = format!("{:?}", a.status);
        first.cover_uri = Some("avatars.yandex.net/get-music-content/first/%%".into());
        a.finish_track_cover(&first.id.clone(), Ok(Outcome::Track(Some(first.clone()))));
        assert_eq!(a.media_snapshot().track.unwrap().cover_uri, first.cover_uri);
        assert_eq!(format!("{:?}", a.status), status);
        assert_eq!(a.player.state(), PlayerState::Playing);
        a.finish_track_cover(&first.id, Err(anyhow::anyhow!("offline")));
        assert_eq!(format!("{:?}", a.status), status);
        press_wait(&mut a, KeyCode::Char('n'));
        let second = a.current_track().unwrap().clone();
        assert_ne!(second.id, first.id);
        a.finish_track_cover(&first.id.clone(), Ok(Outcome::Track(Some(first))));
        assert_eq!(a.current_track().unwrap().cover_uri, second.cover_uri);
        assert_eq!(a.media_snapshot().track.unwrap().id, second.id);
    }

    #[test]
    fn system_seek_preserves_pause_queue_and_prompt_and_clamps() {
        use crate::media::Command;
        let mut a = app();
        assert!(!a.media_snapshot().can_seek);
        a.on_media_command(Command::Seek(Duration::from_secs(42)));
        assert_eq!(a.player.position(), Duration::ZERO);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Down);
        let queue = a.queue.clone();
        let cursor = a.selected;
        a.query = Some("still searching".into());
        a.on_media_command(Command::Seek(Duration::from_millis(42500)));
        assert!(a.player.position() >= Duration::from_millis(42500));
        assert!(a.player.position() < Duration::from_secs(43));
        assert_eq!(a.player.state(), PlayerState::Playing);
        a.on_media_command(Command::Pause);
        a.on_media_command(Command::Seek(Duration::from_secs(17)));
        assert_eq!(a.player.position(), Duration::from_secs(17));
        assert_eq!(a.player.state(), PlayerState::Paused);
        let duration = a.current_track().unwrap().duration();
        a.on_media_command(Command::Seek(Duration::MAX));
        assert_eq!(a.player.position(), duration);
        a.on_media_command(Command::Seek(Duration::ZERO));
        assert_eq!(a.player.position(), Duration::ZERO);
        assert_eq!(a.queue, queue);
        assert_eq!(a.selected, cursor);
        assert_eq!(a.query.as_deref(), Some("still searching"));
        a.likes_waiting = true;
        assert!(!a.media_snapshot().can_seek);
        a.on_media_command(Command::Seek(Duration::from_secs(42)));
        assert_eq!(a.player.position(), Duration::ZERO);
    }

    #[test]
    fn system_play_pause_are_idempotent_and_work_inside_prompts() {
        use crate::media::Command;
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Down);
        let queue = a.queue.clone();
        let entries = a.entries.clone();
        a.query = Some("unfinished search".into());
        a.help = true;
        a.link_dialog = Some(LinkDialog { input: "123".into(), error: None, loading: false });
        for _ in 0..2 { a.on_media_command(Command::Play); }
        assert_eq!(a.player.state(), PlayerState::Playing);
        for _ in 0..2 { a.on_media_command(Command::Pause); }
        assert_eq!(a.player.state(), PlayerState::Paused);
        a.on_media_command(Command::Toggle);
        assert_eq!(a.player.state(), PlayerState::Playing);
        a.on_media_command(Command::Toggle);
        assert_eq!(a.player.state(), PlayerState::Paused);
        a.on_media_command(Command::Play);
        assert_eq!(a.player.state(), PlayerState::Playing);
        assert_eq!(a.queue, queue);
        assert_eq!(a.entries, entries);
        assert_eq!(a.selected, 1);
        assert_eq!(a.query.as_deref(), Some("unfinished search"));
        assert_eq!(a.link_dialog.as_ref().unwrap().input, "123");
        assert!(a.help);
    }

    #[test]
    fn system_next_previous_use_the_playing_queue_without_moving_selection() {
        use crate::media::Command;
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        press(&mut a, KeyCode::Down);
        a.query = Some("search".into());
        a.on_media_command(Command::Next);
        settle(&mut a);
        assert_eq!(a.queue_pos, Some(1));
        a.on_media_command(Command::Previous);
        settle(&mut a);
        assert_eq!(a.queue_pos, Some(0));
        assert_eq!(a.selected, 1);
        assert_eq!(a.query.as_deref(), Some("search"));
        assert_eq!(a.player.state(), PlayerState::Playing);
    }

    #[test]
    fn system_pause_during_buffering_is_applied_when_the_stream_arrives() {
        use crate::media::{Command, State};
        for play in [false, true] {
            let fake = FakeService::parked();
            let gate = fake.gate();
            let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
            press_wait(&mut a, KeyCode::Char('l'));
            gate.hold();
            press(&mut a, KeyCode::Enter);
            assert!(a.net.loading_stream());
            a.on_media_command(Command::Pause);
            a.on_media_command(Command::Pause);
            assert_eq!(a.media_snapshot().state, State::Paused);
            if play {
                a.on_media_command(Command::Play);
                a.on_media_command(Command::Play);
                assert_eq!(a.media_snapshot().state, State::Playing);
            }
            gate.release();
            settle(&mut a);
            assert_eq!(a.player.state(), if play { PlayerState::Playing } else { PlayerState::Paused });
            assert_eq!(a.queue_pos, Some(0));
        }
    }

    #[test]
    fn system_play_during_restore_preserves_saved_track_seek_and_pause_intent() {
        use crate::media::Command;
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        press_wait(&mut a, KeyCode::Char('n'));
        a.player.seek(Duration::from_secs(42));
        for play in [false, true] {
            let fake = FakeService::parked();
            let gate = fake.gate();
            gate.hold();
            let mut b = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
            b.restore(a.snapshot());
            b.on_media_command(Command::Play);
            b.on_media_command(Command::Play);
            if !play {
                b.on_media_command(Command::Pause);
                b.on_media_command(Command::Pause);
            }
            gate.release();
            settle(&mut b);
            assert_eq!(b.queue_pos, Some(1));
            assert_eq!(b.selected, 0);
            assert_eq!(b.player.position().as_secs(), 42);
            assert_eq!(b.player.state(), if play { PlayerState::Playing } else { PlayerState::Paused });
        }
    }

    #[test]
    fn now_playing_metadata_follows_audio_not_the_selected_row() {
        use crate::media::State;
        let mut a = app();
        assert_eq!(a.media_snapshot().state, State::Stopped);
        a.on_media_command(crate::media::Command::Pause);
        assert!(a.media_snapshot().track.is_none());
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        let playing = a.current_track().unwrap().clone();
        press(&mut a, KeyCode::Down);
        a.player.seek(Duration::from_secs(42));
        let snapshot = a.media_snapshot();
        let metadata = snapshot.track.unwrap();
        assert_eq!(metadata.id, playing.id);
        assert_eq!(metadata.title, playing.title);
        assert_eq!(metadata.artist, playing.artist);
        assert_eq!(metadata.duration, playing.duration());
        assert_eq!(snapshot.position.as_secs(), 42);
        assert_eq!(snapshot.state, State::Playing);
        a.on_media_command(crate::media::Command::Pause);
        assert_eq!(a.media_snapshot().state, State::Paused);
        a.play_at(None);
        let stopped = a.media_snapshot();
        assert_eq!(stopped.state, State::Stopped);
        assert!(stopped.track.is_none());
        assert!(!stopped.can_navigate);
    }

    #[test]
    fn system_pause_survives_waiting_for_the_next_likes_page() {
        use crate::media::Command;
        let fake = FakeService::default().with_liked_tracks(350);
        let gate = crate::testing::Park::default();
        gate.hold();
        fake.likes_log.lock().unwrap().next_gate = Some(gate.clone());
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter);
        a.play_at(Some(99));
        let deadline = Instant::now() + Duration::from_millis(500);
        while (a.player.state() != PlayerState::Playing || gate.calls() == 0) && Instant::now() < deadline {
            a.tick(); std::thread::sleep(Duration::from_millis(2));
        }
        a.player.seek(a.current_track().unwrap().duration());
        a.tick();
        assert!(a.likes_waiting);
        a.on_media_command(Command::Pause);
        gate.release();
        settle(&mut a);
        assert_eq!(a.current_track().unwrap().id, "101");
        assert_eq!(a.player.state(), PlayerState::Paused);
    }

    #[test]
    fn artist_back_restores_likes_selection_cursor_and_playback() {
        for key in ['s', 'S'] {
            let mut a = app();
            press_wait(&mut a, KeyCode::Char('l'));
            press_wait(&mut a, KeyCode::Enter);
            press(&mut a, KeyCode::Down);
            let parent = a.entries.clone();
            let cursor = a.likes_list.clone();
            let queue = a.queue.clone();
            let position = a.queue_pos;
            press_wait(&mut a, KeyCode::Char(key));
            assert!(a.can_back_to_likes());
            // Switching popular/all tracks keeps the same return destination.
            press_wait(&mut a, KeyCode::Char('S'));
            a.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
            assert_eq!(a.list, ListKind::Likes);
            assert_eq!(a.entries, parent);
            assert_eq!(a.selected, 1);
            assert_eq!(a.likes_list, cursor);
            assert_eq!(a.queue, queue);
            assert_eq!(a.queue_pos, position);
            assert_eq!(a.player.state(), PlayerState::Playing);
            assert_eq!(a.focus, Focus::List);
            assert!(!a.net.is_busy(), "return does not reload Likes");
            assert!(!a.can_back_to_likes());
        }
    }

    #[test]
    fn artist_back_cancels_an_unfinished_request() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        let mut a = App::new(Box::new(fake), Box::new(SimPlayer::default()), Lang::Ru);
        press_wait(&mut a, KeyCode::Char('l'));
        press(&mut a, KeyCode::Down);
        let parent = a.entries.clone();
        gate.hold();
        press(&mut a, KeyCode::Char('s'));
        a.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
        assert!(!a.net.is_busy());
        gate.release();
        settle(&mut a);
        assert_eq!(a.list, ListKind::Likes);
        assert_eq!(a.entries, parent);
        assert_eq!(a.selected, 1);
    }

    #[test]
    fn artist_return_snapshot_receives_pages_and_heart_changes() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        let mut extra = a.selected_track().unwrap().clone();
        extra.id = "extra".into();
        let next = a.entries.len();
        let cursor = likes::Cursor { id: 99, refs: vec!["ref".into(); next + 1], next };
        a.likes_list = Some(cursor.clone());
        press_wait(&mut a, KeyCode::Char('s'));
        a.finish_likes_page(&cursor, Ok(Outcome::LikesPage(likes::Page {
            tracks: vec![extra.clone()], refs: None, next: next + 1,
        })));
        a.apply_like(&extra.id, false);
        a.on_key(KeyEvent::new(KeyCode::Left, KeyModifiers::SHIFT));
        assert_eq!(a.entries.len(), next + 1);
        assert_eq!(a.likes_list.as_ref().unwrap().next, next + 1);
        assert!(matches!(a.entries.last(), Some(Entry::Track(t)) if t.id == extra.id && !t.liked));
        press_wait(&mut a, KeyCode::Char('s'));
        press_wait(&mut a, KeyCode::Char('p'));
        assert!(!a.can_back_to_likes(), "opening another list discards the artist origin");
    }

    #[test]
    fn artist_shortcuts_follow_the_highlight_not_the_playing_track() {
        let mut a = app();
        press_wait(&mut a, KeyCode::Char('l'));
        press_wait(&mut a, KeyCode::Enter); // Neon Harbor is playing
        let playing = a.current_track().unwrap().id.clone();
        for (key, all) in [(KeyCode::Char('s'), false), (KeyCode::Char('S'), true)] {
            press_wait(&mut a, KeyCode::Char('l'));
            press(&mut a, KeyCode::Down); // Mira Vale is highlighted
            assert_eq!(a.selected_track().unwrap().artist, "Mira Vale");
            press_wait(&mut a, key);
            assert_eq!(a.list, if all { ListKind::ArtistAll("Mira Vale".into()) } else { ListKind::Artist("Mira Vale".into()) });
            assert!(a.entries.iter().all(|e| matches!(e, Entry::Track(t) if t.artist == "Mira Vale")));
            assert_eq!(a.current_track().unwrap().id, playing);
            assert_eq!(a.player.state(), PlayerState::Playing);
        }
        // With no selected track (only playlist rows), playback is a fallback.
        press_wait(&mut a, KeyCode::Char('p'));
        press_wait(&mut a, KeyCode::Char('s'));
        assert_eq!(a.list, ListKind::Artist("Neon Harbor".into()));
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
        saved.likes_list = None; // emulate a legacy session without pagination
        saved.likes_queue = None;
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
        saved.likes_list = None;
        saved.likes_queue = None;
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
