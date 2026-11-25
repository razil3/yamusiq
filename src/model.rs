use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration_secs: u32,
    pub liked: bool,
    pub cached: bool,
    /// Album the track was fetched from; Yandex needs it for likes/streams.
    #[serde(default)]
    pub album_id: Option<String>,
    /// Main (first) artist's id, for the artist views.
    #[serde(default)]
    pub artist_id: Option<String>,
}

impl Track {
    pub fn duration(&self) -> Duration {
        Duration::from_secs(u64::from(self.duration_secs))
    }

}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub title: String,
    pub track_count: usize,
}

/// Preferred stream quality, best first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Quality {
    /// FLAC when the track has it.
    #[default]
    Lossless,
    /// Best lossy (AAC 256 on Yandex).
    High,
    /// AAC 192.
    Normal,
    /// HE-AAC 64 — for slow connections.
    Low,
}

impl Quality {
    pub const ALL: [Quality; 4] = [Self::Lossless, Self::High, Self::Normal, Self::Low];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn id(self) -> &'static str {
        ["lossless", "high", "normal", "low"][self.index()]
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.id() == id)
    }

    /// Cycles best → … → lowest → best.
    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % Self::ALL.len()]
    }
}

/// A playable source for a track.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stream {
    /// URL or file path handed to the decoder.
    pub url: String,
    /// Backend codec name, e.g. `flac-mp4`, `aac-mp4`, `mp3`.
    pub codec: String,
    /// Nominal bitrate; 0 for lossless.
    pub bitrate_kbps: u32,
}

impl Stream {
    pub fn is_lossless(&self) -> bool {
        self.codec.to_lowercase().starts_with("flac")
    }

    /// Short human label: "FLAC · lossless", "AAC 256", "MP3 320".
    pub fn label(&self) -> String {
        let c = self.codec.to_lowercase();
        let rate = |name: &str| if self.bitrate_kbps > 0 { format!("{name} {}", self.bitrate_kbps) } else { name.to_string() };
        if c.starts_with("flac") {
            "FLAC · lossless".into()
        } else if c.starts_with("he-aac") {
            rate("HE-AAC")
        } else if c.starts_with("aac") {
            rate("AAC")
        } else if c.starts_with("mp3") {
            rate("MP3")
        } else {
            rate(&c.to_uppercase())
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LyricLine {
    /// Start time for synced (LRC) lyrics; `None` for plain text.
    pub at: Option<Duration>,
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    pub synced: bool,
}

impl Lyrics {
    /// Parses LRC (`[mm:ss.xx] text`, several stamps per line allowed,
    /// `[ar:…]`-style metadata ignored) or, if no timestamps are found,
    /// plain text with one lyric line per text line.
    pub fn parse(src: &str) -> Self {
        let mut timed = Vec::new();
        for raw in src.lines() {
            let mut rest = raw.trim();
            let mut stamps = Vec::new();
            while let Some(body) = rest.strip_prefix('[') {
                let Some(end) = body.find(']') else { break };
                if let Some(t) = parse_stamp(&body[..end]) {
                    stamps.push(t);
                }
                rest = body[end + 1..].trim_start();
            }
            for t in stamps {
                timed.push(LyricLine { at: Some(t), text: rest.to_string() });
            }
        }
        if !timed.is_empty() {
            timed.sort_by_key(|l| l.at);
            return Self { lines: timed, synced: true };
        }
        let lines = src
            .lines()
            .map(str::trim)
            .skip_while(|l| l.is_empty())
            .map(|l| LyricLine { at: None, text: l.to_string() })
            .collect::<Vec<_>>();
        Self { lines, synced: false }
    }

    /// Index of the line being sung at `pos`. Plain lyrics are spread evenly
    /// over the track so they still follow along. `None` before the first
    /// synced line.
    pub fn current(&self, pos: Duration, duration: Duration) -> Option<usize> {
        if self.lines.is_empty() {
            return None;
        }
        if self.synced {
            return self.lines.iter().rposition(|l| l.at.is_some_and(|t| t <= pos));
        }
        let frac = if duration.is_zero() { 0.0 } else { pos.as_secs_f64() / duration.as_secs_f64() };
        Some(((frac * self.lines.len() as f64) as usize).min(self.lines.len() - 1))
    }
}

fn parse_stamp(s: &str) -> Option<Duration> {
    let (m, rest) = s.split_once(':')?;
    let m: u64 = m.trim().parse().ok()?;
    let secs: f64 = rest.trim().replace(',', ".").parse().ok()?;
    (0.0..60.0).contains(&secs).then(|| Duration::from_secs(m * 60) + Duration::from_secs_f64(secs))
}

pub fn format_time(d: Duration) -> String {
    let s = d.as_secs();
    format!("{}:{:02}", s / 60, s % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stream_labels() {
        let s = |codec: &str, kbps| Stream { url: String::new(), codec: codec.into(), bitrate_kbps: kbps };
        assert_eq!(s("flac-mp4", 0).label(), "FLAC · lossless");
        assert_eq!(s("aac-mp4", 256).label(), "AAC 256");
        assert_eq!(s("he-aac", 64).label(), "HE-AAC 64");
        assert_eq!(s("mp3", 320).label(), "MP3 320");
    }

    #[test]
    fn formats_minutes_and_seconds() {
        assert_eq!(format_time(Duration::from_secs(0)), "0:00");
        assert_eq!(format_time(Duration::from_secs(65)), "1:05");
        assert_eq!(format_time(Duration::from_secs(3600)), "60:00");
    }

    #[test]
    fn parses_lrc_with_metadata_and_repeated_stamps() {
        let l = Lyrics::parse("[ar:Someone]\n[00:05.50]first\n[00:01.00][00:20]chorus\n\n[00:10.00]");
        assert!(l.synced);
        let texts: Vec<_> = l.lines.iter().map(|x| x.text.as_str()).collect();
        assert_eq!(texts, ["chorus", "first", "", "chorus"]);
        let s = |x: f64| Duration::from_secs_f64(x);
        assert_eq!(l.current(s(0.5), s(30.0)), None);
        assert_eq!(l.current(s(1.0), s(30.0)), Some(0));
        assert_eq!(l.current(s(7.0), s(30.0)), Some(1));
        assert_eq!(l.current(s(25.0), s(30.0)), Some(3));
    }

    #[test]
    fn plain_lyrics_follow_track_progress() {
        let l = Lyrics::parse("\none\ntwo\nthree\nfour");
        assert!(!l.synced);
        assert_eq!(l.lines.len(), 4);
        let s = Duration::from_secs;
        assert_eq!(l.current(s(0), s(100)), Some(0));
        assert_eq!(l.current(s(60), s(100)), Some(2));
        assert_eq!(l.current(s(100), s(100)), Some(3));
    }
}
