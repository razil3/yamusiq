//! Background work: one worker thread owns the catalogue, the UI owns the
//! screen.
//!
//! Nothing on the UI thread ever blocks on the network. A key press posts a
//! [`Job`] and returns; the worker runs it and posts the result back; `tick()`
//! picks results up. Every job carries a serial number, so an answer that has
//! been overtaken by a newer request is dropped instead of landing on screen.

use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;

use crate::model::Track;
use crate::service::MusicService;

/// What the worker should fetch.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Wave,
    Likes,
    Cached,
    Playlists,
    PlaylistTracks { id: String, title: String },
    Search { query: String },
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
}

/// Handles to the worker: post a job, or collect what it finished.
pub struct Worker {
    jobs: Sender<(u64, Job)>,
    done: Receiver<Done>,
    next_serial: u64,
    /// The job whose answer the screen is waiting for, if any.
    inflight: Option<(u64, Job)>,
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

        Self { jobs: job_tx, done: done_rx, next_serial: 1, inflight: None }
    }

    /// Queues a job and records it as the one the screen now waits for. Any
    /// earlier answer still on its way becomes stale by comparison.
    pub fn post(&mut self, job: Job) -> u64 {
        let serial = self.next_serial;
        self.next_serial += 1;
        self.inflight = Some((serial, job.clone()));
        // If the worker is gone the app is shutting down; nothing to report.
        let _ = self.jobs.send((serial, job));
        serial
    }

    /// The job the screen is waiting for, for the "buffering" note.
    pub fn inflight(&self) -> Option<&Job> {
        self.inflight.as_ref().map(|(_, j)| j)
    }

    pub fn is_busy(&self) -> bool {
        self.inflight.is_some()
    }

    /// Collects one finished job, if any. Answers to a superseded request and
    /// to requests that are no longer the current one are discarded.
    pub fn poll(&mut self) -> Option<Done> {
        loop {
            match self.done.try_recv() {
                Ok(done) => {
                    let current = self.inflight.as_ref().is_some_and(|(s, _)| *s == done.serial);
                    if !current {
                        continue; // overtaken while in flight
                    }
                    self.inflight = None;
                    return Some(done);
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return None,
            }
        }
    }
}

fn run(service: &mut dyn MusicService, job: &Job) -> anyhow::Result<Outcome> {
    Ok(match job {
        Job::Wave => Outcome::Tracks(service.my_wave()?),
        Job::Likes => Outcome::Tracks(service.likes()?),
        Job::Cached => Outcome::Tracks(service.cached()?),
        Job::Playlists => Outcome::Playlists(service.playlists()?),
        Job::PlaylistTracks { id, .. } => Outcome::Tracks(service.playlist_tracks(id)?),
        Job::Search { query } => Outcome::Tracks(service.search(query)?),
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
