//! Bounded independent workers keep playback off the catalogue queue.
//! Radio feedback stays ordered with radio requests; mutations stay ordered.
//!
//! Nothing on the UI thread ever blocks on the network. A key press posts a
//! [`Job`] and returns; the worker runs it and posts the result back; `tick()`
//! picks results up. Every job carries a serial number, so an answer that has
//! been overtaken by a newer request is dropped instead of landing on screen.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::thread;
use std::time::Instant;

use crate::model::Track;
use crate::service::{MusicService, RequestContext};

/// What the worker should fetch.
#[derive(Clone, Debug, PartialEq)]
pub enum Job {
    Wave,
    TrackWave { track_id: String, title: String },
    RadioMore { view: u64, station: String, queue: String, known: Vec<String>, target: usize },
    RadioNext { generation: u64, station: String, queue: String },
    Feedback { sequence: u64, event: crate::radio::Feedback },
    OpenTrack { id: String },
    /// Backfill artwork metadata for tracks restored from an older session.
    TrackCover { id: String },
    CopyLink { url: String },
    SyncLikes,
    Likes,
    LikesMore { cursor: crate::likes::Cursor },
    Cached,
    Playlists,
    HomePlaylists,
    PlaylistTracks { id: String, title: String, personal: Option<crate::home::Kind> },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Group { Playback, Catalogue, Radio, Metadata, Actions, Downloads }

impl Group {
    const ALL: [Self; 6] = [Self::Playback, Self::Catalogue, Self::Radio, Self::Metadata, Self::Actions, Self::Downloads];
    fn name(self) -> &'static str {
        match self {
            Self::Playback => "playback", Self::Catalogue => "catalogue", Self::Radio => "radio",
            Self::Metadata => "metadata", Self::Actions => "actions", Self::Downloads => "downloads",
        }
    }
}

impl Job {
    fn group(&self) -> Group {
        match self {
            Self::Stream { .. } | Self::QualityStream { .. } | Self::OpenTrack { .. } => Group::Playback,
            Self::Wave | Self::TrackWave { .. } | Self::RadioMore { .. } | Self::RadioNext { .. } | Self::Feedback { .. } => Group::Radio,
            Self::Lyrics { .. } | Self::TrackCover { .. } => Group::Metadata,
            Self::Like { .. } | Self::CopyLink { .. } => Group::Actions,
            Self::SyncLikes | Self::SaveStream { .. } | Self::SyncStream { .. } => Group::Downloads,
            _ => Group::Catalogue,
        }
    }

    fn cancellable(&self) -> bool { !matches!(self, Self::Like { .. } | Self::Feedback { .. }) }

    // Supersession is local to a task category. Opening a list must never
    // discard playback, a like acknowledgement or a download link.
    fn lane(&self) -> String {
        match self {
            Self::LikesMore { cursor } => format!("likes-page:{}", cursor.id),
            Self::RadioMore { .. } => "radio-more".into(),
            Self::RadioNext { .. } => "radio-next".into(),
            Self::Feedback { sequence, .. } => format!("feedback:{sequence}"),
            Self::OpenTrack { .. } => "open-track".into(),
            Self::CopyLink { .. } => "clipboard".into(),
            Self::Stream { .. } | Self::QualityStream { .. } => "playback".into(),
            Self::Lyrics { .. } => "lyrics".into(),
            Self::TrackCover { .. } => "track-cover".into(),
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
    LikesPage(crate::likes::Page),
    Radio(crate::radio::Batch),
    RadioWindow(crate::radio::Window),
    Track(Option<Track>),
    Ack,
    Playlists(Vec<crate::model::Playlist>),
    HomePlaylists(Vec<crate::home::Recommendation>),
    /// A playable source, or none when the backend has no audio for the track.
    Stream(Option<crate::model::Stream>),
    Lyrics(Option<crate::model::Lyrics>),
    /// `set_liked` finished.
    Liked,
    /// A stream requested for saving, a quality switch or the sync.
    Fetched(Option<crate::model::Stream>),
}

/// Handles to the worker: post a job, or collect what it finished.
struct Task {
    serial: u64,
    job: Job,
    context: RequestContext,
    posted: Instant,
}

pub struct Worker {
    jobs: HashMap<Group, Sender<Task>>,
    done: Receiver<Done>,
    failures: VecDeque<Done>,
    next_serial: u64,
    inflight: HashMap<String, (u64, Job)>,
    contexts: HashMap<String, RequestContext>,
}

impl Worker {
    pub fn spawn(service: Box<dyn MusicService>) -> Self {
        let (done_tx, done) = mpsc::channel();
        let mut jobs = HashMap::new();
        for group in Group::ALL {
            let mut service = service.clone_boxed();
            let (tx, rx) = mpsc::channel::<Task>();
            let done_tx = done_tx.clone();
            thread::Builder::new().name(format!("yamusiq-{}", group.name())).spawn(move || {
                while let Ok(task) = rx.recv() {
                    if task.job.cancellable() && !task.context.is_current() { continue; }
                    crate::timing::record(&format!("{}.queue", group.name()), task.posted.elapsed());
                    let started = Instant::now();
                    // A failed worker task must report an error, not leave a
                    // pending stream in buffering while other workers live.
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(&mut *service, &task.job, &task.context)))
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("network worker task failed")));
                    crate::timing::record(&format!("{}.request", group.name()), started.elapsed());
                    if done_tx.send(Done { serial: task.serial, job: task.job, result }).is_err() { break; }
                }
            }).expect("spawn network worker");
            jobs.insert(group, tx);
        }
        Self { jobs, done, failures: VecDeque::new(), next_serial: 1, inflight: HashMap::new(), contexts: HashMap::new() }
    }

    fn forget(&mut self, lane: &str) {
        self.inflight.remove(lane);
        if let Some(context) = self.contexts.remove(lane) { context.cancel(); }
    }

    /// Queues a job. Only an earlier request in the same category is replaced.
    pub fn post(&mut self, job: Job) -> u64 {
        let serial = self.next_serial;
        self.next_serial += 1;
        let lane = job.lane();
        self.forget(&lane);
        let context = RequestContext::default();
        self.contexts.insert(lane.clone(), context.clone());
        self.inflight.insert(lane, (serial, job.clone()));
        let task = Task { serial, job: job.clone(), context, posted: Instant::now() };
        if self.jobs.get(&job.group()).is_none_or(|tx| tx.send(task).is_err()) {
            self.failures.push_back(Done { serial, job, result: Err(anyhow::anyhow!("network worker disconnected")) });
        }
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

    pub fn cancel_radio(&mut self) { self.forget("radio-next"); }
    pub fn loading_radio(&self) -> bool { self.inflight.contains_key("radio-next") }
    pub fn cancel_open(&mut self) { self.forget("open-track"); }

    pub fn loading_radio_more(&self) -> bool { self.inflight.contains_key("radio-more") }
    pub fn cancel_radio_more(&mut self) { self.forget("radio-more"); }

    pub fn cancel_list(&mut self) {
        self.forget("list");
    }

    /// An optimistic edit must not be overwritten by a favourites snapshot
    /// already in flight. Other explicitly requested lists remain unaffected.
    pub fn cancel_likes_list(&mut self) {
        if self.inflight.get("list").is_some_and(|(_, job)| matches!(job, Job::Likes)) {
            self.forget("list");
        }
    }

    pub fn loading_likes_page(&self, id: u64) -> bool { self.inflight.contains_key(&format!("likes-page:{id}")) }

    pub fn retain_likes_pages(&mut self, ids: &[u64]) {
        let keep: Vec<_> = ids.iter().map(|id| format!("likes-page:{id}")).collect();
        let remove: Vec<_> = self.inflight.keys().filter(|lane| lane.starts_with("likes-page:") && !keep.contains(lane)).cloned().collect();
        for lane in remove { self.forget(&lane); }
    }

    pub fn cancel_playback(&mut self) {
        self.forget("playback");
        self.forget("lyrics");
    }

    pub fn loading_stream(&self) -> bool {
        self.inflight.contains_key("playback")
    }

    pub fn loading_sync(&self) -> bool {
        self.inflight.contains_key("sync-likes")
    }

    pub fn cancel_sync(&mut self) {
        let lanes: Vec<_> = self.inflight.keys().filter(|key| *key == "sync-likes" || key.starts_with("sync:")).cloned().collect();
        for lane in lanes { self.forget(&lane); }
    }

    /// Collects one finished job, if any. Answers to a superseded request and
    /// to requests that are no longer the current one are discarded.
    pub fn poll(&mut self) -> Option<Done> {
        loop {
            match self.failures.pop_front().map(Ok).unwrap_or_else(|| self.done.try_recv()) {
                Ok(done) => {
                    let lane = done.job.lane();
                    let current = self.inflight.get(&lane).is_some_and(|(serial, _)| *serial == done.serial);
                    if !current {
                        continue; // overtaken while in flight
                    }
                    self.forget(&lane);
                    return Some(done);
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    // A dead worker must fail pending jobs, not leave the UI
                    // in buffering forever (also restores optimistic likes).
                    let lane = self.inflight.keys().next()?.clone();
                    let (serial, job) = self.inflight.get(&lane)?.clone();
                    self.forget(&lane);
                    return Some(Done { serial, job, result: Err(anyhow::anyhow!("network worker disconnected")) });
                }
            }
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) { for context in self.contexts.values() { context.cancel(); } }
}

fn run(service: &mut dyn MusicService, job: &Job, context: &RequestContext) -> anyhow::Result<Outcome> {
    Ok(match job {
        Job::Wave => Outcome::RadioWindow(crate::radio::initial_window(service, crate::radio::MY_WAVE, context)?),
        Job::TrackWave { track_id, .. } => Outcome::RadioWindow(crate::radio::initial_window(service, &format!("track:{track_id}"), context)?),
        Job::RadioMore { station, queue, known, target, .. } => Outcome::RadioWindow(crate::radio::load_window(service, station, Some(queue), known, *target, context)?),
        Job::RadioNext { station, queue, .. } => Outcome::Radio(service.radio_batch(station, Some(queue))?),
        Job::Feedback { event, .. } => { service.radio_feedback(event)?; Outcome::Ack }
        Job::OpenTrack { id } | Job::TrackCover { id } => Outcome::Track(service.track(id)?),
        Job::CopyLink { url } => { crate::links::copy(url)?; Outcome::Ack }
        Job::SyncLikes => Outcome::Tracks(service.likes_with_context(true, context)?),
        Job::Likes => Outcome::LikesPage(service.likes_page(None, 0, context)?),
        Job::LikesMore { cursor } => Outcome::LikesPage(service.likes_page(Some(&cursor.refs), cursor.next, context)?),
        Job::Cached => Outcome::Tracks(service.cached()?),
        Job::Playlists => Outcome::Playlists(service.playlists()?),
        Job::HomePlaylists => Outcome::HomePlaylists(service.home_playlists()?),
        Job::PlaylistTracks { id, .. } => Outcome::Tracks(service.playlist_tracks(id)?),
        Job::Search { query } => Outcome::Tracks(service.search(query)?),
        Job::Stream { track, quality, .. } => Outcome::Stream(service.stream_with_context(&track.id, *quality, context)?),
        Job::Artist { track, all } => Outcome::Tracks(service.artist_tracks_with_context(track, *all, context)?),
        Job::Lyrics { track_id } => Outcome::Lyrics(service.lyrics(track_id)?),
        Job::Like { track_id, liked } => {
            service.set_liked(track_id, *liked)?;
            Outcome::Liked
        }
        Job::SaveStream { track, quality }
        | Job::QualityStream { track, quality }
        | Job::SyncStream { track, quality } => Outcome::Fetched(service.stream_with_context(&track.id, *quality, context)?),
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
        let Outcome::LikesPage(page) = done.result.unwrap() else { panic!("favourites page expected") };
        assert!(!page.tracks.is_empty());
        assert!(!worker.is_busy(), "the screen stops saying buffering");
    }

    #[test]
    fn playback_and_likes_do_not_wait_for_blocked_catalogue_radio_or_lyrics() {
        let fake = FakeService::parked()
            .with_stream_gate(crate::testing::Park::default())
            .with_action_gate(crate::testing::Park::default());
        let gate = fake.gate();
        let track = fake.likes().unwrap()[0].clone();
        let before = gate.calls();
        gate.hold();
        let mut worker = Worker::spawn(Box::new(fake));
        // Ensure every slow request has actually entered I/O before posting
        // playback, rather than merely winning a scheduling race.
        for (i, job) in [Job::Likes, Job::Lyrics { track_id: track.id.clone() }, Job::SyncLikes, Job::Wave].into_iter().enumerate() {
            worker.post(job);
            let start = Instant::now();
            while gate.calls() < before + i + 1 && start.elapsed() < Duration::from_millis(500) {
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(gate.calls() > before + i);
        }
        let posted = Instant::now();
        worker.post(Job::Stream { track: track.clone(), quality: crate::model::Quality::Lossless, position: 0 });
        worker.post(Job::Like { track_id: track.id, liked: false });
        let mut playback = false;
        let mut liked = false;
        while posted.elapsed() < Duration::from_millis(500) && !(playback && liked) {
            if let Some(done) = worker.poll() {
                assert!(done.result.is_ok());
                playback |= matches!(done.job, Job::Stream { .. });
                liked |= matches!(done.job, Job::Like { .. });
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        gate.release();
        assert!(playback && liked, "playback and mutation must finish while four other queues remain blocked");
    }

    #[test]
    fn superseded_queued_catalogue_requests_are_never_executed() {
        let fake = FakeService::parked();
        let gate = fake.gate();
        gate.hold();
        let mut worker = Worker::spawn(Box::new(fake));
        worker.post(Job::Likes);
        let start = Instant::now();
        while gate.calls() == 0 && start.elapsed() < Duration::from_millis(500) { std::thread::sleep(Duration::from_millis(2)); }
        assert_eq!(gate.calls(), 1);
        worker.post(Job::Search { query: "obsolete 1".into() });
        worker.post(Job::Search { query: "obsolete 2".into() });
        worker.post(Job::Search { query: "neon".into() });
        gate.release();
        let deadline = Instant::now() + Duration::from_secs(1);
        let answer = loop {
            if let Some(done) = worker.poll() { break done; }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(2));
        };
        assert_eq!(answer.job, Job::Search { query: "neon".into() });
        assert_eq!(gate.calls(), 2, "one running call, then only the newest queued search");
    }

    #[test]
    fn radio_feedback_events_remain_in_submission_order() {
        let fake = FakeService::default();
        let log = fake.radio.clone();
        let mut worker = Worker::spawn(Box::new(fake));
        for sequence in 0..12 {
            worker.post(Job::Feedback { sequence, event: crate::radio::Feedback {
                station: crate::radio::MY_WAVE.into(), batch_id: "batch".into(), kind: crate::radio::EventKind::TrackStarted,
                track_id: Some(sequence.to_string()), played_seconds: 0.0, timestamp_ms: sequence,
            }});
        }
        let deadline = Instant::now() + Duration::from_secs(1);
        while worker.is_busy() && Instant::now() < deadline { worker.poll(); std::thread::sleep(Duration::from_millis(2)); }
        assert!(!worker.is_busy());
        assert_eq!(log.lock().unwrap().events.iter().map(|e| e.timestamp_ms).collect::<Vec<_>>(), (0..12).collect::<Vec<_>>());
    }

    #[test]
    fn a_disconnected_worker_reports_failure_and_clears_busy() {
        let (jobs, job_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        drop(job_rx);
        drop(done_tx);
        let mut worker = Worker { jobs: HashMap::from([(Group::Catalogue, jobs)]), done, failures: VecDeque::new(), next_serial: 1, inflight: HashMap::new(), contexts: HashMap::new() };
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
