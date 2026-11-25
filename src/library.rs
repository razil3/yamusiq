//! Local media library: tracks saved as tagged audio files under
//! `~/Library/yamusiq/<Artist>/<Artist> — <Title>.<ext>` (deliberately out
//! of `~/Music`), with an index (`library.json`) next to them. Saving remuxes the stream with FFmpeg
//! (`-c copy`): no re-encoding, so a FLAC stream stays bit-exact.

use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::model::{Stream, Track};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Saved {
    pub track: Track,
    /// File path relative to the library root.
    pub file: String,
    /// Quality label at save time, e.g. "FLAC · lossless".
    pub quality: String,
    pub codec: String,
    pub bitrate_kbps: u32,
}

/// Progress reader state: still running, ended without ffmpeg's final
/// `progress=end`, or ended cleanly.
const RUNNING: u8 = 0;
const ENDED_BAD: u8 = 1;
const ENDED_OK: u8 = 2;

struct Job {
    proc: crate::detach::Proc,
    /// One of RUNNING / ENDED_BAD / ENDED_OK, set by the progress reader.
    state: Arc<AtomicU8>,
    track: Track,
    stream: Stream,
    part: PathBuf,
    target: PathBuf,
    /// Microseconds of audio written so far (from `ffmpeg -progress`).
    done_us: Arc<AtomicU64>,
}

/// Outcome of a finished download, reported once by [`Library::poll`].
#[derive(Debug, PartialEq, Eq)]
pub enum Finished {
    Saved { id: String, title: String },
    Failed { id: String, title: String },
}

impl Finished {
    pub fn id(&self) -> &str {
        match self {
            Self::Saved { id, .. } | Self::Failed { id, .. } => id,
        }
    }
}

pub struct Library {
    root: Option<PathBuf>,
    index: HashMap<String, Saved>,
    jobs: HashMap<String, Job>,
    /// Run ffmpeg without a controlling terminal. Set in the TUI (keeps the
    /// terminal title on yamusiq; the app kills downloads itself). Left off
    /// for command-line runs so Ctrl-C reaches ffmpeg too.
    detached: bool,
}

impl Library {
    /// A library that saves nothing (tests, or no home directory).
    pub fn disabled() -> Self {
        Self { root: None, index: HashMap::new(), jobs: HashMap::new(), detached: false }
    }

    pub fn default_root() -> Option<PathBuf> {
        std::env::var_os("YAMUSIQ_LIBRARY")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/yamusiq")))
    }

    /// Moves a library saved by earlier versions in `~/Music/yamusiq` to the
    /// current location, once. Returns the old path if something was moved.
    pub fn migrate_legacy(root: &Path) -> Option<PathBuf> {
        let old = PathBuf::from(std::env::var_os("HOME")?).join("Music/yamusiq");
        if old == root || !old.join("library.json").is_file() || root.exists() {
            return None;
        }
        std::fs::create_dir_all(root.parent()?).ok()?;
        std::fs::rename(&old, root).ok()?;
        Some(old)
    }

    /// Opens (or lazily creates) the library at `root`. Index entries whose
    /// files were deleted by hand are dropped.
    pub fn open(root: PathBuf) -> Self {
        let index: HashMap<String, Saved> = std::fs::read_to_string(root.join("library.json"))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let index = index.into_iter().filter(|(_, s)| root.join(&s.file).is_file()).collect();
        // Downloads cut short by a crash or Ctrl-C leave `.part` files behind.
        for stray in walk(&root).into_iter().filter(|p| p.to_string_lossy().ends_with(".part")) {
            let _ = std::fs::remove_file(stray);
        }
        Self { root: Some(root), index, jobs: HashMap::new(), detached: false }
    }

    pub fn set_detached(&mut self, detached: bool) {
        self.detached = detached;
    }

    pub fn enabled(&self) -> bool {
        self.root.is_some()
    }

    pub fn is_saved(&self, id: &str) -> bool {
        self.index.contains_key(id)
    }

    pub fn saved(&self, id: &str) -> Option<&Saved> {
        self.index.get(id)
    }

    /// Download progress 0..=1 while a save is running.
    pub fn progress(&self, id: &str) -> Option<f64> {
        let job = self.jobs.get(id)?;
        let total = f64::from(job.track.duration_secs.max(1)) * 1e6;
        Some((job.done_us.load(Ordering::Relaxed) as f64 / total).clamp(0.0, 0.99))
    }

    pub fn is_downloading(&self, id: &str) -> bool {
        self.jobs.contains_key(id)
    }

    /// Downloads currently running.
    pub fn active(&self) -> usize {
        self.jobs.len()
    }

    /// Saved tracks and their total size on disk.
    pub fn stats(&self) -> (usize, u64) {
        let Some(root) = &self.root else { return (0, 0) };
        let bytes = self.index.values().filter_map(|s| root.join(&s.file).metadata().ok()).map(|m| m.len()).sum();
        (self.index.len(), bytes)
    }

    /// Builds a browsable copy of the library under `dest` (same
    /// `<Artist>/<Artist> — <Title>.<ext>` layout). Uses hard links when
    /// possible (no extra space), copies otherwise; files already there with
    /// the same size are skipped. Returns (added, skipped).
    pub fn export_to(&self, dest: &Path) -> Result<(usize, usize)> {
        let Some(root) = &self.root else { bail!("медиатека недоступна") };
        let (mut added, mut skipped) = (0, 0);
        for saved in self.index.values() {
            let src = root.join(&saved.file);
            let dst = dest.join(&saved.file);
            let (Ok(meta), Some(dir)) = (src.metadata(), dst.parent()) else { continue };
            if dst.metadata().is_ok_and(|m| m.len() == meta.len()) {
                skipped += 1;
                continue;
            }
            std::fs::create_dir_all(dir).with_context(|| format!("создание {}", dir.display()))?;
            let _ = std::fs::remove_file(&dst);
            if std::fs::hard_link(&src, &dst).is_err() {
                std::fs::copy(&src, &dst).with_context(|| format!("копирование в {}", dst.display()))?;
            }
            added += 1;
        }
        Ok((added, skipped))
    }

    /// Deletes every saved file (and running downloads). Returns what was
    /// removed: number of tracks and bytes.
    pub fn remove_all(&mut self) -> Result<(usize, u64)> {
        for job in self.jobs.values_mut() {
            job.proc.kill();
            let _ = std::fs::remove_file(&job.part);
        }
        self.jobs.clear();
        let stats = self.stats();
        let ids: Vec<String> = self.index.keys().cloned().collect();
        for id in ids {
            self.remove(&id)?;
        }
        Ok(stats)
    }

    /// Saved tracks, newest first, marked `cached`.
    pub fn tracks(&self) -> Vec<Track> {
        let mut items: Vec<&Saved> = self.index.values().collect();
        items.sort_by(|a, b| (&a.track.artist, &a.track.title).cmp(&(&b.track.artist, &b.track.title)));
        items
            .into_iter()
            .map(|s| Track { cached: true, ..s.track.clone() })
            .collect()
    }

    /// Local file to play instead of streaming, if the track is saved.
    pub fn local_stream(&self, id: &str) -> Option<Stream> {
        let saved = self.index.get(id)?;
        let path = self.root.as_ref()?.join(&saved.file);
        path.is_file().then(|| Stream {
            url: path.to_string_lossy().into_owned(),
            codec: saved.codec.clone(),
            bitrate_kbps: saved.bitrate_kbps,
        })
    }

    /// Starts saving `track` from `stream` in the background.
    pub fn start(&mut self, track: &Track, stream: &Stream) -> Result<()> {
        let Some(root) = self.root.clone() else { bail!("медиатека недоступна") };
        if self.jobs.contains_key(&track.id) {
            return Ok(());
        }
        let ext = extension(&stream.codec);
        let artist = sanitize(track.artist.split(", ").next().unwrap_or(&track.artist));
        let dir = root.join(&artist);
        std::fs::create_dir_all(&dir).with_context(|| format!("создание {}", dir.display()))?;
        let target = unique(&dir.join(format!("{artist} — {}.{ext}", sanitize(&track.title))));
        let part = target.with_extension(format!("{ext}.part"));

        let mut cmd = Command::new("ffmpeg");
        cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-progress", "pipe:1", "-nostats"]);
        if stream.url.starts_with("http") {
            cmd.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "4"]);
        }
        cmd.args(["-i", &stream.url, "-map", "0:a:0", "-c", "copy"])
            .args(["-metadata", &format!("title={}", track.title)])
            .args(["-metadata", &format!("artist={}", track.artist)])
            .args(["-metadata", &format!("album={}", track.album)])
            .args(["-f", muxer(&stream.codec)])
            .arg(&part);
        cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
        let (proc, out) = crate::detach::Proc::spawn(&mut cmd, self.detached).context("запуск ffmpeg")?;

        // Completion is read from the progress stream (works for orphaned
        // processes too, whose exit status we can't collect).
        let done_us = Arc::new(AtomicU64::new(0));
        let state = Arc::new(AtomicU8::new(RUNNING));
        let (done, st) = (Arc::clone(&done_us), Arc::clone(&state));
        thread::spawn(move || {
            let mut clean = false;
            if let Some(out) = out {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(v) = line.strip_prefix("out_time_us=").and_then(|v| v.trim().parse::<u64>().ok()) {
                        done.store(v, Ordering::Relaxed);
                    }
                    if line.trim() == "progress=end" {
                        clean = true;
                    }
                }
            }
            st.store(if clean { ENDED_OK } else { ENDED_BAD }, Ordering::SeqCst);
        });
        self.jobs.insert(track.id.clone(), Job { proc, state, track: track.clone(), stream: stream.clone(), part, target, done_us });
        Ok(())
    }

    /// Collects finished downloads; call regularly from the UI loop.
    pub fn poll(&mut self) -> Vec<Finished> {
        let done: Vec<String> = self
            .jobs
            .iter_mut()
            .filter_map(|(id, job)| (job.state.load(Ordering::SeqCst) != RUNNING).then(|| id.clone()))
            .collect();
        let mut out = Vec::new();
        for id in done {
            let Some(mut job) = self.jobs.remove(&id) else { continue };
            job.proc.reap();
            // A clean end that covered (nearly) the whole track, and a file.
            let wanted_us = u64::from(job.track.duration_secs) * 1_000_000 * 9 / 10;
            let ok = job.state.load(Ordering::SeqCst) == ENDED_OK
                && job.done_us.load(Ordering::Relaxed) >= wanted_us
                && job.part.metadata().is_ok_and(|m| m.len() > 0);
            if ok && std::fs::rename(&job.part, &job.target).is_ok() {
                if let Some(root) = &self.root {
                    let file = job.target.strip_prefix(root).unwrap_or(&job.target).to_string_lossy().into_owned();
                    let mut track = job.track.clone();
                    track.cached = true;
                    self.index.insert(
                        id.clone(),
                        Saved { track, file, quality: job.stream.label(), codec: job.stream.codec.clone(), bitrate_kbps: job.stream.bitrate_kbps },
                    );
                    let _ = self.save_index();
                }
                out.push(Finished::Saved { id: job.track.id.clone(), title: job.track.title.clone() });
            } else {
                let _ = std::fs::remove_file(&job.part);
                out.push(Finished::Failed { id: job.track.id.clone(), title: job.track.title.clone() });
            }
        }
        out
    }

    /// Deletes the saved file and forgets the track.
    pub fn remove(&mut self, id: &str) -> Result<()> {
        let Some(saved) = self.index.remove(id) else { return Ok(()) };
        if let Some(root) = &self.root {
            let path = root.join(&saved.file);
            let _ = std::fs::remove_file(&path);
            // Tidy an artist folder left empty.
            if let Some(dir) = path.parent().filter(|d| *d != root.as_path()) {
                let _ = std::fs::remove_dir(dir);
            }
        }
        self.save_index()
    }

    fn save_index(&self) -> Result<()> {
        let Some(root) = &self.root else { return Ok(()) };
        std::fs::create_dir_all(root)?;
        let tmp = root.join("library.json.tmp");
        std::fs::write(&tmp, serde_json::to_string_pretty(&self.index)?)?;
        std::fs::rename(&tmp, root.join("library.json"))?;
        Ok(())
    }
}

impl Drop for Library {
    /// Unfinished downloads are abandoned, not left half-written.
    fn drop(&mut self) {
        for job in self.jobs.values_mut() {
            job.proc.kill();
            let _ = std::fs::remove_file(&job.part);
        }
    }
}

fn extension(codec: &str) -> &'static str {
    let c = codec.to_lowercase();
    if c.starts_with("flac") {
        "flac"
    } else if c.starts_with("mp3") {
        "mp3"
    } else {
        "m4a"
    }
}

fn muxer(codec: &str) -> &'static str {
    match extension(codec) {
        "flac" => "flac",
        "mp3" => "mp3",
        _ => "ipod", // .m4a
    }
}

/// File-name-safe text: no path separators or characters Finder dislikes.
fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() { '_' } else { c })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() { "—".into() } else { trimmed.chars().take(120).collect() }
}

/// `name.ext`, or `name (2).ext` … if taken (different tracks, same title).
fn unique(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    (2..)
        .map(|n| path.with_file_name(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .expect("some free name")
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                out.extend(walk(&p));
            } else {
                out.push(p);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn track(id: &str, title: &str) -> Track {
        Track {
            id: id.into(),
            title: title.into(),
            artist: "Тест: Артист".into(),
            album: "Альбом".into(),
            duration_secs: 1,
            liked: false,
            cached: false,
            album_id: None,
            artist_id: None,
        }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("yamusiq-lib-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn names_are_safe_and_unique() {
        assert_eq!(sanitize("AC/DC: Live?"), "AC_DC_ Live_");
        assert_eq!(sanitize("  .hidden "), "hidden");
        assert_eq!(sanitize(""), "—");
        assert_eq!(extension("flac-mp4"), "flac");
        assert_eq!(extension("aac-mp4"), "m4a");
        assert_eq!(extension("mp3"), "mp3");
    }

    #[test]
    fn saves_a_flac_bit_exact_indexes_and_removes_it() {
        if Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).status().is_err() {
            return; // ffmpeg missing: nothing to test
        }
        let dir = temp_dir("save");
        let src = dir.join("source.flac");
        assert!(Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .arg(&src)
            .status()
            .unwrap()
            .success());

        let root = dir.join("lib");
        let mut lib = Library::open(root.clone());
        let t = track("42", "Песня / тест");
        let stream = Stream { url: src.to_string_lossy().into_owned(), codec: "flac-mp4".into(), bitrate_kbps: 0 };
        lib.start(&t, &stream).unwrap();
        assert!(lib.is_downloading("42"));
        let mut finished = Vec::new();
        for _ in 0..300 {
            finished.extend(lib.poll());
            if !finished.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(finished, [Finished::Saved { id: "42".into(), title: "Песня / тест".into() }]);
        assert!(lib.is_saved("42") && !lib.is_downloading("42"));
        let saved = lib.saved("42").unwrap();
        assert_eq!(saved.file, "Тест_ Артист/Тест_ Артист — Песня _ тест.flac");
        assert_eq!(saved.quality, "FLAC · lossless");
        assert!(lib.local_stream("42").unwrap().url.ends_with(".flac"));
        assert!(lib.tracks()[0].cached);

        // Survives a restart via the index.
        drop(lib);
        let lib = Library::open(root.clone());
        assert!(lib.is_saved("42"));

        // The copy is bit-exact audio (FLAC → FLAC without re-encoding).
        let md5 = |p: &Path| {
            let out = Command::new("ffmpeg").args(["-nostdin", "-loglevel", "error", "-i"]).arg(p).args(["-f", "md5", "-"]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        assert_eq!(md5(&src), md5(&root.join(&lib.saved("42").unwrap().file)));

        assert_eq!(lib.stats().0, 1);
        assert!(lib.stats().1 > 0);
        // A stray partial file is cleaned up on open.
        std::fs::write(root.join("Тест_ Артист/junk.flac.part"), b"x").unwrap();
        let mut lib = Library::open(root.clone());
        assert!(!root.join("Тест_ Артист/junk.flac.part").exists());
        // Export builds the same layout elsewhere and is idempotent.
        let music = dir.join("music");
        assert_eq!(lib.export_to(&music).unwrap(), (1, 0));
        assert!(music.join("Тест_ Артист/Тест_ Артист — Песня _ тест.flac").is_file());
        assert_eq!(lib.export_to(&music).unwrap(), (0, 1));
        let (n, bytes) = lib.remove_all().unwrap();
        assert!(music.join("Тест_ Артист/Тест_ Артист — Песня _ тест.flac").is_file(), "exported copy is independent");
        assert!(n == 1 && bytes > 0);
        assert!(!lib.is_saved("42"));
        assert!(!root.join("Тест_ Артист").exists(), "empty artist folder tidied");
        assert!(Library::open(root).tracks().is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_download_leaves_nothing_behind() {
        let dir = temp_dir("fail");
        let mut lib = Library::open(dir.join("lib"));
        let stream = Stream { url: dir.join("missing.flac").to_string_lossy().into_owned(), codec: "flac".into(), bitrate_kbps: 0 };
        if lib.start(&track("1", "x"), &stream).is_err() {
            return; // ffmpeg missing
        }
        let mut finished = Vec::new();
        for _ in 0..300 {
            finished.extend(lib.poll());
            if !finished.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(finished, [Finished::Failed { id: "1".into(), title: "x".into() }]);
        assert!(!lib.is_saved("1"));
        let leftovers: Vec<_> = super::walk(&dir.join("lib"));
        assert!(leftovers.iter().all(|p| !p.to_string_lossy().ends_with(".part")), "{leftovers:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
