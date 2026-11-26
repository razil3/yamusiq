//! Stable favourites snapshots: IDs are cheap, full track details are paged.
use serde::{Deserialize, Serialize};
use crate::model::Track;

pub const PAGE_SIZE: usize = 100;
pub const PREFETCH: usize = 10;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Cursor {
    pub id: u64,
    pub refs: Vec<String>,
    /// References consumed, not playable rows returned. Removed/unavailable
    /// tracks must not cause the same page to be requested indefinitely.
    pub next: usize,
}

impl Cursor {
    pub fn has_more(&self) -> bool { self.next < self.refs.len() }
    pub fn valid(&self) -> bool { self.next <= self.refs.len() }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Page {
    pub tracks: Vec<Track>,
    /// Only the first page carries the complete ordered ID snapshot.
    pub refs: Option<Vec<String>>,
    pub next: usize,
}
