//! Radio protocol and session metadata, independent of the TUI and HTTP.
use serde::{Deserialize, Serialize};

use crate::model::Track;

pub const MY_WAVE: &str = "user:onyourwave";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Batch {
    pub station: String,
    pub batch_id: String,
    pub tracks: Vec<Track>,
}

/// One batch id per queue entry: feedback must use the batch that supplied
/// that track, not the id of the most recently prefetched batch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub station: String,
    pub batch_ids: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum EventKind {
    RadioStarted,
    TrackStarted,
    TrackFinished,
    Skip,
}

impl EventKind {
    pub fn api_name(self) -> &'static str {
        match self {
            Self::RadioStarted => "radioStarted",
            Self::TrackStarted => "trackStarted",
            Self::TrackFinished => "trackFinished",
            Self::Skip => "skip",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Feedback {
    pub station: String,
    pub batch_id: String,
    pub kind: EventKind,
    pub track_id: Option<String>,
    pub played_seconds: f64,
    pub timestamp_ms: u64,
}

/// A display window may combine several server batches. Batch ids remain
/// aligned with tracks so browsing ahead never fabricates listening feedback.
#[derive(Clone, Debug, PartialEq)]
pub struct Window {
    pub station: String,
    pub tracks: Vec<Track>,
    pub batch_ids: Vec<String>,
}

pub const INITIAL_TRACKS: usize = 15;
pub const MORE_TRACKS: usize = 5;

/// Publish one complete server batch immediately, without throwing away its
/// remaining tracks or waiting for further recommendations.
pub fn initial_window(service: &dyn crate::service::MusicService, station: &str, context: &crate::service::RequestContext) -> anyhow::Result<Window> {
    context.check()?;
    let batch = service.radio_batch(station, None)?;
    context.check()?;
    if batch.station != station { anyhow::bail!("radio station mismatch"); }
    let mut seen = std::collections::HashSet::new();
    let tracks: Vec<_> = batch.tracks.into_iter().filter(|t| seen.insert(t.id.clone())).collect();
    let batch_ids = vec![batch.batch_id; tracks.len()];
    Ok(Window { station: batch.station, tracks, batch_ids })
}

pub fn load_window(
    service: &dyn crate::service::MusicService,
    station: &str,
    cursor: Option<&str>,
    known: &[String],
    target: usize,
    context: &crate::service::RequestContext,
) -> anyhow::Result<Window> {
    let mut seen: std::collections::HashSet<String> = known.iter().cloned().collect();
    let mut cursor = cursor.map(str::to_string);
    let mut out = Window { station: station.into(), tracks: Vec::new(), batch_ids: Vec::new() };
    let mut unchanged = 0;
    // Rotor has no reliable 'limit' or offset parameter. Bound attempts when
    // it repeats recommendations; never send fake skips to advance its state.
    for _ in 0..6 {
        context.check()?;
        let batch = match service.radio_batch(station, cursor.as_deref()) {
            Ok(batch) => batch,
            Err(_) if !out.tracks.is_empty() => break,
            Err(e) => return Err(e),
        };
        context.check()?;
        if batch.station != station { anyhow::bail!("radio station mismatch"); }
        if let Some(last) = batch.tracks.last() { cursor = Some(last.id.clone()); }
        let before = out.tracks.len();
        for track in batch.tracks {
            if seen.insert(track.id.clone()) {
                out.tracks.push(track);
                out.batch_ids.push(batch.batch_id.clone());
                if out.tracks.len() >= target { break; }
            }
        }
        if out.tracks.len() >= target { break; }
        if before == out.tracks.len() { unchanged += 1; } else { unchanged = 0; }
        if unchanged >= 2 { break; }
    }
    Ok(out)
}
