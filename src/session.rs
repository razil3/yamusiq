//! Last session (`~/.config/yamusiq/session.json`): the open list, the queue,
//! the current track and where it stopped, so the next launch picks up there.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::app::{Entry, Focus, ListKind, PlaybackMode};
use crate::config::Config;
use crate::model::Track;

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    /// Backend name incl. account (e.g. "YANDEX · login"); a session is only
    /// restored into the same backend so demo and real tracks never mix.
    pub backend: String,
    pub list: ListKind,
    pub entries: Vec<Entry>,
    pub selected: usize,
    #[serde(default)]
    pub playlist_parent: Option<(Vec<Entry>, usize)>,
    pub section: usize,
    pub focus: Focus,
    pub queue: Vec<Track>,
    pub queue_pos: Option<usize>,
    pub position_secs: f64,
    pub mode: PlaybackMode,
    pub volume: u8,
}

impl Session {
    pub fn path() -> Option<PathBuf> {
        Some(Config::path()?.with_file_name("session.json"))
    }

    /// Missing or unreadable (e.g. from an older version) → start fresh.
    pub fn load() -> Option<Self> {
        let text = std::fs::read_to_string(Self::path()?).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("no HOME to store the session in")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Write-then-rename so a crash mid-write never leaves a broken file.
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_string(self)?)?;
        std::fs::rename(&tmp, &path).with_context(|| format!("writing {}", path.display()))
    }
}
