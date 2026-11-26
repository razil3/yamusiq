//! Music catalogue abstraction.
//!
//! The TUI only talks to [`MusicService`]; the Yandex Music adapter
//! implements it (tests use an in-memory fake).

use anyhow::Result;

use crate::model::{Lyrics, Playlist, Quality, Stream, Track};

pub trait MusicService: Send {
    /// Human-readable backend name shown in the UI.
    fn name(&self) -> &str;
    fn search(&self, query: &str) -> Result<Vec<Track>>;
    fn playlists(&self) -> Result<Vec<Playlist>>;
    fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>>;
    fn my_wave(&self) -> Result<Vec<Track>>;
    fn likes(&self) -> Result<Vec<Track>>;
    /// Every liked track, without the display cap of [`likes`](Self::likes)
    /// (used by the favourites sync).
    fn likes_all(&self) -> Result<Vec<Track>> {
        self.likes()
    }
    fn cached(&self) -> Result<Vec<Track>>;
    /// Tracks by the main artist of `track`: the popular ones, or with
    /// `all` every track of theirs the backend knows.
    fn artist_tracks(&self, track: &Track, all: bool) -> Result<Vec<Track>>;
    fn set_liked(&mut self, track_id: &str, liked: bool) -> Result<()>;
    /// Song text if the backend has it. Synced (LRC) lyrics are preferred;
    /// `Ok(None)` means the track simply has no lyrics.
    fn lyrics(&self, track_id: &str) -> Result<Option<Lyrics>>;
    /// Playable source for the track for the real player; `Ok(None)` means
    /// the backend has no audio (the demo catalogue).
    fn stream(&self, _track_id: &str, _quality: Quality) -> Result<Option<Stream>> {
        Ok(None)
    }

    /// A second handle for the background worker. Implementations that share
    /// the HTTP agent (as the Yandex one does) clone cheaply; the ones that
    /// own mutable state rebuild themselves.
    fn clone_boxed(&self) -> Box<dyn MusicService>;
}

/// Case-folds and treats «ё» as «е», as Russian users routinely type it.
pub fn normalize(s: &str) -> String {
    s.to_lowercase().replace('ё', "е")
}
