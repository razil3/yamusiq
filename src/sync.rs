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
    cancelled: bool,
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
        Self { total: pending.len(), pending, in_flight: HashSet::new(), cancelled: false, already, saved: 0, failed: 0 }
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

    /// Takes the next batch of tracks to fetch. The stream links themselves come
    /// from the background worker, one job per track; `finish_stream` starts the
    /// download when a link arrives.
    pub fn next_batch(&mut self, lib: &Library, want: usize) -> Vec<Track> {
        let mut out = Vec::new();
        if self.cancelled { return out; }
        while out.len() < want {
            let Some(track) = self.pending.pop_front() else { break };
            if lib.is_saved(&track.id) {
                self.saved += 1; // saved meanwhile (e.g. by hand)
                continue;
            }
            if lib.is_downloading(&track.id) {
                self.in_flight.insert(track.id.clone()); // already running (auto-download)
                continue;
            }
            self.in_flight.insert(track.id.clone());
            out.push(track);
        }
        out
    }

    /// A track the sync wanted could not be fetched.
    pub fn mark_failed(&mut self, track_id: &str) {
        if self.in_flight.remove(track_id) {
            self.failed += 1;
        }
    }

    /// How many slots are free for new downloads.
    pub fn free_slots(&self, lib: &Library) -> usize {
        PARALLEL.saturating_sub(lib.active().max(self.in_flight.len()))
    }

    /// CLI callers have no UI thread and may fetch links synchronously.
    pub fn pump(&mut self, lib: &mut Library, service: &dyn MusicService, quality: Quality) {
        while !self.cancelled && self.free_slots(lib) > 0 && !self.pending.is_empty() {
            let tracks = self.next_batch(lib, self.free_slots(lib));
            for track in tracks {
                match service.stream(&track.id, quality) {
                    Ok(Some(stream)) if lib.start(&track, &stream).is_ok() => {}
                    _ => self.mark_failed(&track.id),
                }
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
        self.cancelled = true;
        self.pending.clear();
        self.in_flight.clear();
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
    fn link_requests_count_towards_the_parallel_limit() {
        let lib = Library::disabled();
        let mut sync = Sync::new(FakeService::default().likes().unwrap(), &lib);
        let batch = sync.next_batch(&lib, sync.free_slots(&lib));
        assert_eq!(batch.len(), PARALLEL);
        assert_eq!(sync.free_slots(&lib), 0, "links awaiting a response occupy slots too");
        sync.mark_failed(&batch[0].id);
        assert_eq!(sync.free_slots(&lib), 1);
        sync.cancel();
        assert!(sync.next_batch(&lib, PARALLEL).is_empty());
        assert!(sync.is_done());
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
