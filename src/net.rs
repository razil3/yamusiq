//! Background work: one worker thread owns the catalogue, the UI owns the
//! screen.
//!
//! Nothing on the UI thread ever blocks on the network. A key press posts a
//! [`Job`] and returns; the worker runs it and posts the result back; `tick()`
//! picks results up. Every job carries a serial number, so an answer that has
//! been overtaken by a newer request is dropped instead of landing on screen.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use crate::model::Track;
use crate::service::MusicService;

/// What the worker should fetch.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Wave,
    SyncLikes,
    Likes,
    Cached,
    Playlists,
    PlaylistTracks { id: String, title: String },
    Search { query: String },
    /// Play `track`; the worker resolves the stream (or plays a saved copy).
    Stream { track: Track, quality: crate::model::Quality, position: usize },
    Artist { track: Track, all: bool },
    Lyrics { track_id: String },
    /// Set or clear the like on a track.
    Like { track_id: String, liked: bool },
    /// The link that saving a track needs, fetched on demand.
    SaveStream { track: Track, quality: crate::model::Quality },
    /// A stream of the same track in another quality tier.
    QualityStream { track: Track, quality: crate::model::Quality },
    /// A stream for the favourites sync (needs its own handle).
    SyncStream { track: Track, quality: crate::model::Quality },
}

impl Job {
    // Supersession is local to a task category. Opening a list must never
    // discard playback, a like acknowledgement or a download link.
    fn lane(&self) -> String {
        match self {
            Self::Stream { .. } | Self::QualityStream { .. } => "playback".into(),
            Self::Lyrics { .. } => "lyrics".into(),
            Self::Like { track_id, .. } => format!("like:{track_id}"),
            Self::SaveStream { track, .. } => format!("save:{}", track.id),
            Self::SyncStream { track, .. } => format!("sync:{}", track.id),
            Self::SyncLikes => "sync-likes".into(),
            _ => "list".into(),
        }
    }
}

/// What came back, tagged with the serial of the job that asked for it.
pub struct Done {
    pub serial: u64,
    pub job: Job,
    pub result: anyhow::Result<Outcome>,
}

/// A successful result, shaped for the screen.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    Tracks(Vec<Track>),
    Playlists(Vec<crate::model::Playlist>),
    /// A playable source, or none when the backend has no audio for the track.
    Stream(Option<crate::model::Stream>),
    Lyrics(Option<crate::model::Lyrics>),
    /// `set_liked` finished.
    Liked,
    /// A stream requested for saving, a quality switch or the sync.
    Fetched(Option<crate::model::Stream>),
}

/// Handles to the worker: post a job, or collect what it finished.
pub struct Worker {
    jobs: Sender<(u64, Job)>,
    done: Receiver<Done>,
    next_serial: u64,
    /// Latest request per independent operation category.
    inflight: HashMap<String, (u64, Job)>,
}

impl Worker {
    /// Sends jobs to `service`, which moves to the worker thread for good.
    pub fn spawn(mut service: Box<dyn MusicService>) -> Self {
        let (job_tx, job_rx) = mpsc::channel::<(u64, Job)>();
        let (done_tx, done_rx) = mpsc::channel::<Done>();

        thread::Builder::new()
            .name("yamusiq-net".into())
            .spawn(move || {
                while let Ok((serial, job)) = job_rx.recv() {
                    let result = run(&mut *service, &job);
                    // A closed receiver means the app is gone; stop quietly.
                    if done_tx.send(Done { serial, job, result }).is_err() {
                        break;
                    }
                }
            })
            .expect("spawn the network worker");

        Self { jobs: job_tx, done: done_rx, next_serial: 1, inflight: HashMap::new() }
    }

    /// Queues a job. Only an earlier request in the same category is replaced.
    pub fn post(&mut self, job: Job) -> u64 {
        let serial = self.next_serial;
        self.next_serial += 1;
        self.inflight.insert(job.lane(), (serial, job.clone()));
        // poll() reports a disconnected worker as an error for pending jobs.
        let _ = self.jobs.send((serial, job));
        serial
    }

    /// Whether the screen is waiting for an answer. The UI reads it for tests
    /// and diagnostics; the footer is driven by `Status::Loading`.
    #[cfg(test)]
    pub fn is_busy(&self) -> bool {
        !self.inflight.is_empty()
    }

    pub fn liking(&self, id: &str) -> bool {
        self.inflight.contains_key(&format!("like:{id}"))
    }

    pub fn cancel_list(&mut self) {
        self.inflight.remove("list");
    }

    pub fn cancel_playback(&mut self) {
        self.inflight.remove("playback");
        self.inflight.remove("lyrics");
    }

    pub fn loading_stream(&self) -> bool {
        self.inflight.contains_key("playback")
    }

    pub fn loading_sync(&self) -> bool {
        self.inflight.contains_key("sync-likes")
    }

    pub fn cancel_sync(&mut self) {
        self.inflight.retain(|key, _| key != "sync-likes" && !key.starts_with("sync:"));
    }

    /// Collects one finished job, if any. Answers to a superseded request and
    /// to requests that are no longer the current one are discarded.
    pub fn poll(&mut self) -> Option<Done> {
        loop {
            match self.done.try_recv() {
                Ok(done) => {
                    let lane = done.job.lane();
                    let current = self.inflight.get(&lane).is_some_and(|(serial, _)| *serial == done.serial);
                    if !current {
                        continue; // overtaken while in flight
                    }
                    self.inflight.remove(&lane);
                    return Some(done);
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    // A dead worker must fail pending jobs, not leave the UI
                    // in buffering forever (also restores optimistic likes).
                    let lane = self.inflight.keys().next()?.clone();
                    let (serial, job) = self.inflight.remove(&lane)?;
                    return Some(Done { serial, job, result: Err(anyhow::anyhow!("network worker disconnected")) });
                }
            }
        }
    }
}

fn run(service: &mut dyn MusicService, job: &Job) -> anyhow::Result<Outcome> {
    Ok(match job {
        Job::Wave => Outcome::Tracks(service.my_wave()?),
        Job::SyncLikes => Outcome::Tracks(service.likes_all()?),
        Job::Likes => Outcome::Tracks(service.likes()?),
        Job::Cached => Outcome::Tracks(service.cached()?),
        Job::Playlists => Outcome::Playlists(service.playlists()?),
        Job::PlaylistTracks { id, .. } => Outcome::Tracks(service.playlist_tracks(id)?),
        Job::Search { query } => Outcome::Tracks(service.search(query)?),
        Job::Stream { track, quality, .. } => Outcome::Stream(service.stream(&track.id, *quality)?),
        Job::Artist { track, all } => Outcome::Tracks(service.artist_tracks(track, *all)?),
        Job::Lyrics { track_id } => Outcome::Lyrics(service.lyrics(track_id)?),
        Job::Like { track_id, liked } => {
            service.set_liked(track_id, *liked)?;
            Outcome::Liked
        }
        Job::SaveStream { track, quality }
        | Job::QualityStream { track, quality }
        | Job::SyncStream { track, quality } => Outcome::Fetched(service.stream(&track.id, *quality)?),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::FakeService;
    use std::time::{Duration, Instant};

    #[test]
    fn a_held_catalogue_does_not_block_the_caller() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut worker = Worker::spawn(Box::new(fake));

        let posted = Instant::now();
        worker.post(Job::Likes);
        assert!(posted.elapsed() < Duration::from_millis(50), "posting is instant");
        assert!(worker.is_busy(), "the screen knows a request is in flight");
        // The worker thread may not have picked the job up yet; wait for it
        // briefly rather than assuming the scheduler is instantaneous.
        let start = Instant::now();
        while gate.calls() == 0 && start.elapsed() < Duration::from_millis(500) {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(gate.calls(), 1, "the worker picked the job up");

        // While the catalogue is held, polling keeps saying "nothing yet" —
        // which is what lets the UI keep drawing frames.
        let spin = Instant::now();
        while spin.elapsed() < Duration::from_millis(80) {
            assert!(worker.poll().is_none());
        }

        gate.release();
        let done = (0..200).find_map(|_| {
            std::thread::sleep(Duration::from_millis(5));
            worker.poll()
        });
        let done = done.expect("the answer eventually arrives");
        assert_eq!(done.job, Job::Likes);
        let Outcome::Tracks(tracks) = done.result.unwrap() else { panic!("tracks expected") };
        assert!(!tracks.is_empty());
        assert!(!worker.is_busy(), "the screen stops saying buffering");
    }

    #[test]
    fn a_disconnected_worker_reports_failure_and_clears_busy() {
        let (jobs, job_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        drop(job_rx);
        drop(done_tx);
        let mut worker = Worker { jobs, done, next_serial: 1, inflight: HashMap::new() };
        worker.post(Job::Likes);
        assert!(worker.is_busy());
        let answer = worker.poll().expect("failure for the pending request");
        assert_eq!(answer.job, Job::Likes);
        assert!(answer.result.is_err());
        assert!(!worker.is_busy());
        assert!(worker.poll().is_none());
    }

    #[test]
    fn an_overtaken_answer_never_reaches_the_screen() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut worker = Worker::spawn(Box::new(fake));

        worker.post(Job::Likes); // queued first
        worker.post(Job::Wave); // the user moved on before the first answered

        gate.release();

        // Only the newest request may come back.
        let mut seen = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline && seen.len() < 2 {
            if let Some(done) = worker.poll() {
                seen.push(done.job);
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(seen, vec![Job::Wave], "the stale likes answer was dropped");
    }
}
