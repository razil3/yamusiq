//! System media controls. Native callbacks only enqueue commands; the TUI
//! owns playback, queue changes and radio feedback on its existing thread.
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
mod artwork;
#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::Controls;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Play,
    Pause,
    Toggle,
    Previous,
    Next,
    Seek(Duration),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Stopped,
    Paused,
    Playing,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Metadata {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: Duration,
    pub cover_uri: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub track: Option<Metadata>,
    pub state: State,
    pub position: Duration,
    pub can_play: bool,
    pub can_navigate: bool,
    pub can_seek: bool,
}

impl Command {
    #[cfg(target_os = "macos")]
    fn bit(self) -> u8 {
        1 << match self {
            Self::Play => 0,
            Self::Pause => 1,
            Self::Toggle => 2,
            Self::Previous => 3,
            Self::Next => 4,
            Self::Seek(_) => 5,
        }
    }
}

impl Snapshot {
    pub fn accepts(&self, command: Command) -> bool {
        match command {
            Command::Play | Command::Toggle => self.can_play,
            Command::Pause => self.track.is_some(),
            Command::Previous | Command::Next => self.can_navigate,
            Command::Seek(_) => self.can_seek,
        }
    }
}

/// Publish state changes/seeks immediately, but do not flood the framework
/// with wall-clock progress every TUI frame. macOS extrapolates at playbackRate.
#[derive(Default)]
struct Publication {
    last: Option<(Snapshot, Instant)>,
}

impl Publication {
    fn due(&mut self, snapshot: &Snapshot, now: Instant) -> bool {
        let publish = self.last.as_ref().is_none_or(|(last, at)| {
            snapshot.track != last.track
                || snapshot.state != last.state
                || snapshot.can_play != last.can_play
                || snapshot.can_navigate != last.can_navigate
                || snapshot.can_seek != last.can_seek
                || snapshot.position.abs_diff(last.position) >= Duration::from_secs(2)
                || (snapshot != last && now.duration_since(*at) >= Duration::from_secs(1))
        });
        if publish {
            self.last = Some((snapshot.clone(), now));
        }
        publish
    }
}

#[cfg(not(target_os = "macos"))]
pub struct Controls;
#[cfg(not(target_os = "macos"))]
impl Controls {
    pub fn new() -> Self {
        Self
    }
    pub fn poll(&self) -> Vec<Command> {
        Vec::new()
    }
    pub fn update(&mut self, _: &Snapshot) {}
    pub fn preview(&self) -> Option<std::sync::Arc<crate::cover::Preview>> {
        None
    }
}

/// Optional artwork for non-playing cards. Each slot has its own latest-only
/// worker, so three Home requests cannot supersede one another or Now Playing.
#[derive(Default)]
pub struct PreviewLoader {
    pub preview: Option<std::sync::Arc<crate::cover::Preview>>,
    #[cfg(target_os = "macos")]
    loader: Option<artwork::Loader>,
}
impl PreviewLoader {
    pub fn request(&mut self, uri: Option<&str>) {
        #[cfg(target_os = "macos")]
        {
            if uri.is_none() && self.loader.is_none() { return; }
            let loader = self.loader.get_or_insert_with(artwork::Loader::new);
            if loader.request(uri) { self.preview = None; }
        }
        #[cfg(not(target_os = "macos"))]
        let _ = uri;
    }

    pub fn poll(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(bytes) = self.loader.as_ref().and_then(artwork::Loader::poll) {
            self.preview = objc2::rc::autoreleasepool(|_| bytes.as_deref().and_then(macos::make_preview));
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn preview_for_test(bytes: &[u8]) -> Option<std::sync::Arc<crate::cover::Preview>> {
    macos::make_preview(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Snapshot {
        Snapshot {
            track: Some(Metadata {
                id: "1".into(),
                title: "Title".into(),
                artist: "Artist".into(),
                album: "Album".into(),
                duration: Duration::from_secs(180),
                cover_uri: None,
            }),
            state: State::Playing,
            position: Duration::ZERO,
            can_play: true,
            can_navigate: true,
            can_seek: true,
        }
    }

    #[test]
    fn publication_throttles_progress_but_not_pause_track_changes_or_seeks() {
        let mut publication = Publication::default();
        let mut s = snapshot();
        let now = Instant::now();
        assert!(publication.due(&s, now));
        assert!(!publication.due(&s, now + Duration::from_secs(1)));
        s.position = Duration::from_millis(80);
        assert!(!publication.due(&s, now + Duration::from_millis(80)));
        s.position = Duration::from_secs(1);
        assert!(publication.due(&s, now + Duration::from_secs(1)));
        s.state = State::Paused;
        assert!(publication.due(&s, now + Duration::from_millis(1010)));
        s.position = Duration::from_secs(20);
        assert!(publication.due(&s, now + Duration::from_millis(1020)));
        s.track.as_mut().unwrap().id = "2".into();
        assert!(publication.due(&s, now + Duration::from_millis(1030)));
        s.can_navigate = false;
        assert!(publication.due(&s, now + Duration::from_millis(1040)));
        s.track = None;
        s.state = State::Stopped;
        assert!(publication.due(&s, now + Duration::from_millis(1050)));
    }

    #[test]
    fn seek_availability_is_published_immediately() {
        let mut publication = Publication::default();
        let mut s = snapshot();
        let now = Instant::now();
        assert!(s.accepts(Command::Seek(Duration::from_secs(42))));
        assert!(publication.due(&s, now));
        s.can_seek = false;
        assert!(!s.accepts(Command::Seek(Duration::ZERO)));
        assert!(publication.due(&s, now + Duration::from_millis(1)));
    }

    #[test]
    fn commands_require_available_content() {
        let mut s = snapshot();
        for command in [
            Command::Play,
            Command::Pause,
            Command::Toggle,
            Command::Previous,
            Command::Next,
        ] {
            assert!(s.accepts(command));
        }
        s.track = None;
        s.can_navigate = false;
        assert!(s.accepts(Command::Play)); // a selected row can start playback
        assert!(!s.accepts(Command::Pause));
        assert!(!s.accepts(Command::Next));
        s.can_play = false;
        assert!(!s.accepts(Command::Toggle));
    }
}
