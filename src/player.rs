//! Playback abstraction.
//!
//! [`SimPlayer`] keeps a wall-clock position without producing audio (demo
//! catalogue, tests). [`crate::audio::AudioPlayer`] decodes real streams.

use std::time::{Duration, Instant};

use crate::model::Track;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerState {
    Stopped,
    Playing,
    Paused,
}

/// What the listener is hearing right now, for the visualiser.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Analysis {
    /// Log-spaced band levels, low to high, 0..=1.
    pub bands: Vec<f32>,
    /// Recent mono samples starting at a rising zero crossing, -1..=1.
    pub wave: Vec<f32>,
    /// Loudness of the recent window, 0..=1 (−60 dBFS..0 dBFS).
    pub level: f32,
}

pub trait Player {
    /// Start `track`. `source` is a URL or file path to decode; `None` means
    /// there is no audio and the player only keeps time.
    fn play(&mut self, track: &Track, source: Option<&str>);
    fn toggle_pause(&mut self);
    fn stop(&mut self);
    fn set_volume(&mut self, volume: u8);
    fn state(&self) -> PlayerState;
    fn position(&self) -> Duration;
    /// True once the current track has played to its end.
    fn finished(&self) -> bool;
    /// Jump to `to` (clamped to the track), keeping play/pause state.
    fn seek(&mut self, to: Duration);
    /// Real-time spectrum and waveform, if this player produces audio.
    fn analysis(&self, _bands: usize) -> Option<Analysis> {
        None
    }
}

#[derive(Default)]
pub struct SimPlayer {
    duration: Duration,
    /// Position accumulated before the current play segment started.
    base: Duration,
    started: Option<Instant>,
    loaded: bool,
    volume: u8,
}

impl SimPlayer {
    #[cfg(test)]
    fn advance(&mut self, by: Duration) {
        if let Some(s) = self.started.as_mut() {
            *s -= by;
        }
    }
}

impl Player for SimPlayer {
    fn play(&mut self, track: &Track, _source: Option<&str>) {
        self.duration = track.duration();
        self.base = Duration::ZERO;
        self.started = Some(Instant::now());
        self.loaded = true;
    }

    fn toggle_pause(&mut self) {
        if !self.loaded {
            return;
        }
        match self.started.take() {
            Some(s) => self.base += s.elapsed(),
            None => self.started = Some(Instant::now()),
        }
    }

    fn stop(&mut self) {
        *self = Self { volume: self.volume, ..Self::default() };
    }

    fn set_volume(&mut self, volume: u8) {
        self.volume = volume.min(100);
    }

    fn state(&self) -> PlayerState {
        match (self.loaded, self.started.is_some()) {
            (false, _) => PlayerState::Stopped,
            (true, true) => PlayerState::Playing,
            (true, false) => PlayerState::Paused,
        }
    }

    fn position(&self) -> Duration {
        let pos = self.base + self.started.map_or(Duration::ZERO, |s| s.elapsed());
        pos.min(self.duration)
    }

    fn finished(&self) -> bool {
        self.loaded && self.position() >= self.duration
    }

    fn seek(&mut self, to: Duration) {
        if !self.loaded {
            return;
        }
        self.base = to.min(self.duration);
        if self.started.is_some() {
            self.started = Some(Instant::now());
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn seek_clamps_and_keeps_pause_state() {
        let mut p = SimPlayer::default();
        p.seek(Duration::from_secs(5)); // no-op when nothing is loaded
        assert_eq!(p.position(), Duration::ZERO);
        p.play(&track(100), None);
        p.seek(Duration::from_secs(40));
        assert!(p.position() >= Duration::from_secs(40) && p.position() < Duration::from_secs(41));
        p.toggle_pause();
        p.seek(Duration::from_secs(500));
        assert_eq!(p.position(), Duration::from_secs(100));
        assert_eq!(p.state(), PlayerState::Paused);
    }

    use super::*;

    fn track(secs: u32) -> Track {
        Track {
            id: "x".into(),
            title: "x".into(),
            artist: "x".into(),
            album: "x".into(),
            duration_secs: secs,
            liked: false,
            cached: false,
            album_id: None,
            artist_id: None, cover_uri: None,
        }
    }

    #[test]
    fn lifecycle() {
        let mut p = SimPlayer::default();
        assert_eq!(p.state(), PlayerState::Stopped);
        p.toggle_pause();
        assert_eq!(p.state(), PlayerState::Stopped);

        p.play(&track(10), None);
        assert_eq!(p.state(), PlayerState::Playing);
        p.advance(Duration::from_secs(4));
        p.toggle_pause();
        assert_eq!(p.state(), PlayerState::Paused);
        let paused_at = p.position();
        assert!(paused_at >= Duration::from_secs(4));
        assert!(!p.finished());

        p.toggle_pause();
        p.advance(Duration::from_secs(20));
        assert_eq!(p.position(), Duration::from_secs(10));
        assert!(p.finished());

        p.stop();
        assert_eq!(p.state(), PlayerState::Stopped);
        assert!(!p.finished());
    }
}
