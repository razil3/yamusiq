//! Real playback: FFmpeg decodes the source to raw float PCM, cpal plays it,
//! and the output callback taps exactly what is sent to the speakers for the
//! visualiser — so spectrum, waveform, position and lyrics stay in sync with
//! what the listener hears.

mod analysis;

/// Analyse arbitrary mono samples (used by live tests outside this module).
#[cfg(test)]
pub fn analyze_for_test(samples: &[f32], rate: u32) -> crate::player::Analysis {
    analysis::Analyzer::default().analyze(samples, rate, 48)
}

use std::collections::VecDeque;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::model::Track;
use crate::player::{Analysis, Player, PlayerState, SimPlayer};
use analysis::{Analyzer, WINDOW};

/// Decoded audio kept ahead of the speakers.
const BUFFER_SECS: usize = 4;
/// Mono samples kept for analysis.
const TAP_LEN: usize = WINDOW * 2;

/// State shared between the UI thread, the decoder thread and the audio
/// callback.
#[derive(Default)]
struct Shared {
    /// Interleaved samples waiting to be played.
    queue: Mutex<VecDeque<f32>>,
    /// Mono copy of what was just played (pre-volume), newest last.
    tap: Mutex<VecDeque<f32>>,
    /// Frames played since the current decode started.
    played: AtomicU64,
    paused: AtomicBool,
    /// Output gain as f32 bits.
    gain: AtomicU32,
    /// The current decoder reached end of input.
    eof: AtomicBool,
    /// Bumped on every (re)start; stale decoder threads notice and exit.
    generation: AtomicU64,
}

pub struct AudioPlayer {
    shared: Arc<Shared>,
    _stream: cpal::Stream,
    rate: u32,
    channels: u16,
    child: Option<crate::detach::Proc>,
    source: Option<String>,
    start: Duration,
    duration: Duration,
    loaded: bool,
    /// Tracks without a source (shouldn't happen with a real backend) are
    /// timed by a simulated clock instead of failing outright.
    sim: SimPlayer,
    simulated: bool,
    analyzer: Analyzer,
}

impl AudioPlayer {
    pub fn new() -> Result<Self> {
        let ffmpeg_ok = Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).stderr(Stdio::null()).status();
        if !ffmpeg_ok.is_ok_and(|s| s.success()) {
            bail!("ffmpeg не найден в PATH (brew install ffmpeg)");
        }
        let device = cpal::default_host().default_output_device().context("нет устройства вывода звука")?;
        let supported = device.default_output_config().context("устройство вывода не отдаёт конфигурацию")?;
        if supported.sample_format() != cpal::SampleFormat::F32 {
            bail!("неподдерживаемый формат вывода {:?} (нужен f32)", supported.sample_format());
        }
        let config = supported.config();
        let (rate, channels) = (config.sample_rate, config.channels);
        let shared = Arc::new(Shared::default());
        shared.gain.store(1.0f32.to_bits(), Ordering::Relaxed);

        let cb = Arc::clone(&shared);
        let ch = usize::from(channels);
        let stream = device
            .build_output_stream::<f32, _, _>(
                config,
                move |out: &mut [f32], _| fill_output(&cb, out, ch),
                |_err| {}, // device errors surface as silence; the UI keeps running
                None,
            )
            .context("не удалось открыть аудиопоток")?;
        stream.play().context("не удалось запустить аудиопоток")?;

        Ok(Self {
            shared,
            _stream: stream,
            rate,
            channels,
            child: None,
            source: None,
            start: Duration::ZERO,
            duration: Duration::ZERO,
            loaded: false,
            sim: SimPlayer::default(),
            simulated: false,
            analyzer: Analyzer::default(),
        })
    }

    fn kill_decoder(&mut self) {
        self.shared.generation.fetch_add(1, Ordering::SeqCst);
        if let Some(mut child) = self.child.take() {
            child.kill();
        }
        self.shared.queue.lock().unwrap().clear();
    }

    /// (Re)start decoding the current source at `at`.
    fn restart(&mut self, at: Duration) {
        self.kill_decoder();
        let Some(source) = self.source.clone() else { return };
        let generation = self.shared.generation.load(Ordering::SeqCst);
        self.shared.eof.store(false, Ordering::SeqCst);
        self.shared.played.store(0, Ordering::SeqCst);
        self.shared.tap.lock().unwrap().clear();
        self.start = at;
        match spawn_decoder(&source, at, self.rate, self.channels, Arc::clone(&self.shared), generation) {
            Ok(child) => self.child = Some(child),
            Err(_) => self.shared.eof.store(true, Ordering::SeqCst),
        }
    }
}

impl Drop for AudioPlayer {
    fn drop(&mut self) {
        self.kill_decoder();
    }
}

/// Audio callback: pull from the queue, apply gain, tap a mono copy.
fn fill_output(shared: &Shared, out: &mut [f32], ch: usize) {
    if shared.paused.load(Ordering::Relaxed) {
        out.fill(0.0);
        return;
    }
    let gain = f32::from_bits(shared.gain.load(Ordering::Relaxed));
    let mut queue = shared.queue.lock().unwrap();
    let frames = (out.len() / ch).min(queue.len() / ch);
    let mut tap = shared.tap.lock().unwrap();
    for f in 0..frames {
        let mut mono = 0.0;
        for c in 0..ch {
            let s = queue.pop_front().unwrap_or(0.0);
            out[f * ch + c] = s * gain;
            mono += s;
        }
        tap.push_back(mono / ch as f32);
    }
    out[frames * ch..].fill(0.0);
    let excess = tap.len().saturating_sub(TAP_LEN);
    tap.drain(..excess);
    shared.played.fetch_add(frames as u64, Ordering::Relaxed);
}

/// Runs `ffmpeg … -f f32le -` and feeds its output into `shared.queue` from a
/// background thread, with back-pressure so only a few seconds are buffered.
fn spawn_decoder(source: &str, at: Duration, rate: u32, channels: u16, shared: Arc<Shared>, generation: u64) -> Result<crate::detach::Proc> {
    let started = Instant::now();
    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error"]);
    if source.starts_with("http://") || source.starts_with("https://") {
        cmd.args(["-reconnect", "1", "-reconnect_streamed", "1", "-reconnect_delay_max", "4"]);
    }
    if !at.is_zero() {
        cmd.args(["-ss", &format!("{:.3}", at.as_secs_f64())]);
    }
    cmd.args(["-i", source, "-vn", "-f", "f32le", "-ac", &channels.to_string(), "-ar", &rate.to_string(), "-"]);
    // Orphaned, without a terminal: the window title keeps showing yamusiq,
    // not "yamusiq ▸ ffmpeg"; the player kills the decoder itself.
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    let (child, stdout) = crate::detach::Proc::spawn(&mut cmd, true).context("запуск ffmpeg")?;
    let mut stdout = stdout.context("ffmpeg без stdout")?;
    let max_queued = rate as usize * usize::from(channels) * BUFFER_SECS;

    thread::spawn(move || {
        let mut buf = vec![0u8; 32 * 1024];
        let mut carry: Vec<u8> = Vec::with_capacity(4);
        let mut first_pcm = true;
        let current = || shared.generation.load(Ordering::SeqCst) == generation;
        loop {
            if !current() {
                return;
            }
            if shared.queue.lock().unwrap().len() >= max_queued {
                thread::sleep(Duration::from_millis(15));
                continue;
            }
            match stdout.read(&mut buf) {
                Ok(0) | Err(_) => {
                    if current() {
                        shared.eof.store(true, Ordering::SeqCst);
                    }
                    return;
                }
                Ok(n) => {
                    carry.extend_from_slice(&buf[..n]);
                    let whole = carry.len() / 4 * 4;
                    let samples = carry[..whole].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b));
                    if !current() {
                        return;
                    }
                    shared.queue.lock().unwrap().extend(samples);
                    if whole > 0 && first_pcm {
                        first_pcm = false;
                        crate::timing::record("audio.first_pcm", started.elapsed());
                    }
                    carry.drain(..whole);
                }
            }
        }
    });
    Ok(child)
}

impl Player for AudioPlayer {
    fn play(&mut self, track: &Track, source: Option<&str>) {
        self.duration = track.duration();
        self.loaded = true;
        self.shared.paused.store(false, Ordering::SeqCst);
        match source {
            Some(src) => {
                self.simulated = false;
                self.sim.stop();
                self.source = Some(src.to_string());
                self.restart(Duration::ZERO);
            }
            None => {
                self.kill_decoder();
                self.source = None;
                self.simulated = true;
                self.sim.play(track, None);
            }
        }
    }

    fn toggle_pause(&mut self) {
        if self.simulated {
            self.sim.toggle_pause();
        } else if self.loaded {
            let p = self.shared.paused.load(Ordering::SeqCst);
            self.shared.paused.store(!p, Ordering::SeqCst);
        }
    }

    fn stop(&mut self) {
        self.kill_decoder();
        self.sim.stop();
        self.loaded = false;
        self.simulated = false;
        self.source = None;
        self.shared.tap.lock().unwrap().clear();
    }

    fn set_volume(&mut self, volume: u8) {
        // Perceptual curve: halfway on the slider sounds roughly half as loud.
        let gain = (f32::from(volume.min(100)) / 100.0).powf(1.7);
        self.shared.gain.store(gain.to_bits(), Ordering::Relaxed);
        self.sim.set_volume(volume);
    }

    fn state(&self) -> PlayerState {
        if self.simulated {
            return self.sim.state();
        }
        match (self.loaded, self.shared.paused.load(Ordering::SeqCst)) {
            (false, _) => PlayerState::Stopped,
            (true, true) => PlayerState::Paused,
            (true, false) => PlayerState::Playing,
        }
    }

    fn position(&self) -> Duration {
        if self.simulated {
            return self.sim.position();
        }
        let played = self.shared.played.load(Ordering::Relaxed) as f64 / f64::from(self.rate.max(1));
        (self.start + Duration::from_secs_f64(played)).min(self.duration.max(self.start))
    }

    fn finished(&self) -> bool {
        if self.simulated {
            return self.sim.finished();
        }
        self.loaded && self.shared.eof.load(Ordering::SeqCst) && self.shared.queue.lock().unwrap().is_empty()
    }

    fn seek(&mut self, to: Duration) {
        if self.simulated {
            self.sim.seek(to);
        } else if self.loaded {
            let limit = self.duration.saturating_sub(Duration::from_secs(1));
            self.restart(to.min(limit));
        }
    }

    fn analysis(&self, bands: usize) -> Option<Analysis> {
        if self.simulated || !self.loaded {
            return None;
        }
        let tap: Vec<f32> = self.shared.tap.lock().unwrap().iter().copied().collect();
        if tap.is_empty() {
            return None; // still buffering
        }
        Some(self.analyzer.analyze(&tap, self.rate, bands))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decodes a generated 440 Hz tone through the same ffmpeg pipeline the
    /// player uses (no audio device needed) and checks the analyser sees it.
    #[test]
    fn decoder_pipeline_feeds_queue_and_analysis_finds_the_tone() {
        if Command::new("ffmpeg").arg("-version").stdout(Stdio::null()).status().is_err() {
            eprintln!("ffmpeg not installed; skipping");
            return;
        }
        let dir = std::env::temp_dir().join(format!("yamusiq-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("tone.wav");
        let ok = Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-y", "-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
            .arg(&wav)
            .status()
            .unwrap();
        assert!(ok.success());

        let shared = Arc::new(Shared::default());
        let mut child = spawn_decoder(wav.to_str().unwrap(), Duration::from_millis(200), 48_000, 2, Arc::clone(&shared), 0).unwrap();
        // The decoder is not our descendant and has no controlling terminal,
        // so the terminal doesn't list it ("yamusiq ▸ ffmpeg").
        let ppid = {
            let out = Command::new("ps").args(["-o", "ppid=", "-p", &child.id().to_string()]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        if !ppid.is_empty() {
            assert_ne!(ppid, std::process::id().to_string(), "ffmpeg must not be our child");
        }
        let ps = |field: &str, pid: u32| {
            let out = Command::new("ps").args(["-o", &format!("{field}="), "-p", &pid.to_string()]).output().unwrap();
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let child_group = ps("pgid", child.id());
        if !child_group.is_empty() {
            assert_ne!(child_group, ps("pgid", std::process::id()), "ffmpeg is in another session");
            let (ours, theirs) = (ps("tty", std::process::id()), ps("tty", child.id()));
            // Only meaningful when the test itself runs on a terminal.
            if ours != "??" && !ours.is_empty() {
                assert_eq!(theirs, "??", "ffmpeg has no controlling terminal (ours: {ours})");
            }
        }
        for _ in 0..200 {
            if shared.eof.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        child.reap();
        assert!(shared.eof.load(Ordering::SeqCst), "decoder reaches end of input");
        let queued: Vec<f32> = shared.queue.lock().unwrap().iter().copied().collect();
        // 0.8 s of stereo at 48 kHz (seeked 0.2 s into a 1 s tone).
        let frames = queued.len() / 2;
        assert!((37_000..=40_000).contains(&frames), "got {frames} frames");

        let mono: Vec<f32> = queued.as_chunks::<2>().0.iter().map(|[l, r]| (l + r) / 2.0).collect();
        let a = Analyzer::default().analyze(&mono, 48_000, 48);
        let peak = a.bands.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap().0;
        assert!((20..=28).contains(&peak), "440 Hz should sit mid-low, got band {peak}");
        assert!(a.level > 0.3);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn output_callback_applies_gain_taps_and_counts() {
        let shared = Shared::default();
        shared.gain.store(0.5f32.to_bits(), Ordering::Relaxed);
        shared.queue.lock().unwrap().extend([1.0, -1.0, 0.5, 0.5, 0.2, 0.2]);
        let mut out = [9.0f32; 8];
        fill_output(&shared, &mut out, 2);
        assert_eq!(out, [0.5, -0.5, 0.25, 0.25, 0.1, 0.1, 0.0, 0.0]);
        assert_eq!(shared.played.load(Ordering::Relaxed), 3);
        assert_eq!(shared.tap.lock().unwrap().iter().copied().collect::<Vec<_>>(), [0.0, 0.5, 0.2]);

        shared.paused.store(true, Ordering::Relaxed);
        shared.queue.lock().unwrap().extend([1.0, 1.0]);
        let mut out = [9.0f32; 2];
        fill_output(&shared, &mut out, 2);
        assert_eq!(out, [0.0, 0.0], "paused outputs silence and keeps the queue");
        assert_eq!(shared.queue.lock().unwrap().len(), 2);
    }
}
