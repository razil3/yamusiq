//! Favourites sync: downloads every liked track that is not yet in the local
//! library, a couple at a time, and reports overall progress.

use std::collections::{HashSet, VecDeque};

use crate::library::{Finished, Library};
use crate::model::{Quality, Track};
use crate::service::MusicService;

/// Parallel downloads; more would mostly fight over bandwidth.
pub const PARALLEL: usize = 2;

pub struct Sync {
    pending: VecDeque<Track>,
    in_flight: HashSet<String>,
    /// Tracks this run has to fetch (already-saved ones are not counted).
    pub total: usize,
    /// Liked tracks that were already in the library at the start.
    pub already: usize,
    pub saved: usize,
    pub failed: usize,
}

impl Sync {
    pub fn new(liked: Vec<Track>, lib: &Library) -> Self {
        let mut seen = HashSet::new();
        let unique: Vec<Track> = liked.into_iter().filter(|t| seen.insert(t.id.clone())).collect();
        let already = unique.iter().filter(|t| lib.is_saved(&t.id)).count();
        let pending: VecDeque<Track> = unique.into_iter().filter(|t| !lib.is_saved(&t.id)).collect();
        Self { total: pending.len(), pending, in_flight: HashSet::new(), already, saved: 0, failed: 0 }
    }

    pub fn is_done(&self) -> bool {
        self.pending.is_empty() && self.in_flight.is_empty()
    }

    pub fn finished_count(&self) -> usize {
        self.saved + self.failed
    }

    /// Overall progress 0..=1, counting partial progress of running downloads.
    pub fn fraction(&self, lib: &Library) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        let partial: f64 = self.in_flight.iter().filter_map(|id| lib.progress(id)).sum();
        ((self.finished_count() as f64 + partial) / self.total as f64).clamp(0.0, 1.0)
    }

    /// Start downloads while there is room. Stream links are fetched just in
    /// time because they expire.
    pub fn pump(&mut self, lib: &mut Library, service: &dyn MusicService, quality: Quality) {
        while lib.active() < PARALLEL {
            let Some(track) = self.pending.pop_front() else { return };
            if lib.is_saved(&track.id) {
                self.saved += 1; // saved meanwhile (e.g. by hand)
                continue;
            }
            if lib.is_downloading(&track.id) {
                self.in_flight.insert(track.id.clone()); // already running (auto-download)
                continue;
            }
            match service.stream(&track.id, quality) {
                Ok(Some(stream)) if lib.start(&track, &stream).is_ok() => {
                    self.in_flight.insert(track.id.clone());
                }
                _ => self.failed += 1,
            }
        }
    }

    /// Account for a finished download. Returns whether it belonged to the sync.
    pub fn on_finished(&mut self, f: &Finished) -> bool {
        if !self.in_flight.remove(f.id()) {
            return false;
        }
        match f {
            Finished::Saved { .. } => self.saved += 1,
            Finished::Failed { .. } => self.failed += 1,
        }
        true
    }

    /// Stop queueing new downloads; running ones finish on their own.
    pub fn cancel(&mut self) {
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeService;

    #[test]
    fn counts_skip_saved_and_track_failures() {
        let svc = FakeService::default();
        let lib = Library::disabled();
        let liked = svc.likes().unwrap();
        let n = liked.len();
        let mut doubled = liked.clone();
        doubled.extend(liked); // duplicates collapse
        let mut sync = Sync::new(doubled, &lib);
        assert_eq!((sync.total, sync.already), (n, 0));
        assert_eq!(sync.fraction(&lib), 0.0);

        // The fake service has no audio and the library is disabled: every start fails.
        let mut lib = lib;
        sync.pump(&mut lib, &svc, Quality::Lossless);
        assert!(sync.is_done());
        assert_eq!(sync.failed, n);
        assert_eq!(sync.fraction(&lib), 1.0);
    }

    #[test]
    fn finished_events_are_matched_by_id() {
        let mut sync = Sync::new(Vec::new(), &Library::disabled());
        sync.in_flight.insert("7".into());
        sync.total = 1;
        assert!(!sync.on_finished(&Finished::Saved { id: "8".into(), title: "x".into() }), "not ours");
        assert!(sync.on_finished(&Finished::Saved { id: "7".into(), title: "x".into() }));
        assert_eq!(sync.saved, 1);
        assert!(sync.is_done());
    }
}
