//! Music catalogue abstraction.
//!
//! The TUI only talks to [`MusicService`]; the Yandex Music adapter
//! implements it (tests use an in-memory fake).

use anyhow::Result;

use crate::model::{Lyrics, Playlist, Quality, Stream, Track};

/// Cooperative cancellation: checked between HTTP batches, never held as a lock
/// across network I/O. In-flight HTTP calls still obey the client's timeout.
#[derive(Clone)]
pub struct RequestContext(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Default for RequestContext {
    fn default() -> Self { Self(std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true))) }
}

impl RequestContext {
    pub fn cancel(&self) { self.0.store(false, std::sync::atomic::Ordering::Release); }
    pub fn is_current(&self) -> bool { self.0.load(std::sync::atomic::Ordering::Acquire) }
    pub fn check(&self) -> Result<()> {
        if !self.is_current() { anyhow::bail!("request cancelled"); }
        Ok(())
    }
}

pub trait MusicService: Send {
    /// Human-readable backend name shown in the UI.
    fn name(&self) -> &str;
    fn search(&self, query: &str) -> Result<Vec<Track>>;
    fn playlists(&self) -> Result<Vec<Playlist>>;
    fn home_playlists(&self) -> Result<Vec<crate::home::Recommendation>> { Ok(Vec::new()) }
    fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>>;
    fn my_wave(&self) -> Result<Vec<Track>>;
    fn radio_batch(&self, station: &str, _queue: Option<&str>) -> Result<crate::radio::Batch> {
        Ok(crate::radio::Batch { station: station.into(), batch_id: String::new(), tracks: self.my_wave()? })
    }
    fn radio_feedback(&self, _event: &crate::radio::Feedback) -> Result<()> { Ok(()) }
    fn track(&self, _id: &str) -> Result<Option<Track>> { Ok(None) }
    /// Liked tracks without a display cap.
    fn likes(&self) -> Result<Vec<Track>>;
    /// Every liked track for the favourites sync; defaults to the same full list.
    fn likes_all(&self) -> Result<Vec<Track>> {
        self.likes()
    }
    fn likes_with_context(&self, all: bool, context: &RequestContext) -> Result<Vec<Track>> {
        context.check()?;
        let tracks = if all { self.likes_all()? } else { self.likes()? };
        context.check()?;
        Ok(tracks)
    }
    /// The display uses stable ID snapshots and at most 100 detailed tracks.
    /// Full-library sync continues to use `likes_all` / `likes_with_context`.
    fn likes_page(&self, refs: Option<&[String]>, offset: usize, context: &RequestContext) -> Result<crate::likes::Page> {
        let tracks = self.likes_with_context(false, context)?;
        let initial = refs.is_none();
        let owned: Vec<_> = tracks.iter().map(|t| t.id.clone()).collect();
        let refs = refs.unwrap_or(&owned);
        let end = offset.saturating_add(crate::likes::PAGE_SIZE).min(refs.len());
        let rows = refs.get(offset..end).unwrap_or_default().iter().filter_map(|id| tracks.iter().find(|t| &t.id == id).cloned()).collect();
        Ok(crate::likes::Page { tracks: rows, refs: initial.then(|| refs.to_vec()), next: end })
    }

    fn stream_with_context(&self, id: &str, quality: Quality, context: &RequestContext) -> Result<Option<Stream>> {
        context.check()?;
        let stream = self.stream(id, quality)?;
        context.check()?;
        Ok(stream)
    }
    fn cached(&self) -> Result<Vec<Track>>;
    /// Tracks by the main artist of `track`: the popular ones, or with
    /// `all` every track of theirs the backend knows.
    fn artist_tracks(&self, track: &Track, all: bool) -> Result<Vec<Track>>;
    fn artist_tracks_with_context(&self, track: &Track, all: bool, context: &RequestContext) -> Result<Vec<Track>> {
        context.check()?;
        let tracks = self.artist_tracks(track, all)?;
        context.check()?;
        Ok(tracks)
    }
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
