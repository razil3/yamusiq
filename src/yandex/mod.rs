//! Yandex Music backend (the unofficial `api.music.yandex.net` used by the
//! official apps). Authentication is an OAuth token supplied by the user.

mod client;
mod parse;

use std::sync::Mutex;
use std::collections::HashSet;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{bail, Context, Result};
use serde_json::Value;

use crate::model::{Lyrics, Playlist, Quality, Stream, Track};
use crate::service::{normalize, MusicService};
use client::Client;

/// How many liked tracks to load at once (newest first).
const LIKES_LIMIT: usize = 300;
/// `/tracks` accepts batches; keep requests modest.
const BATCH: usize = 100;
/// Artist catalogue paging for "all tracks".
const ARTIST_PAGE: usize = 100;
const ARTIST_MAX_PAGES: usize = 10;

pub struct YandexService {
    client: Client,
    uid: u64,
    name: String,
    /// Liked track ids, loaded lazily and kept in sync by `set_liked`.
    liked: Mutex<Option<HashSet<String>>>,
}

impl YandexService {
    /// Whether `e` means the token itself is bad (as opposed to, say, no
    /// network) — then the user should be asked for a new one.
    pub fn is_auth_error(e: &anyhow::Error) -> bool {
        e.chain().any(|c| {
            let s = c.to_string();
            s.starts_with("HTTP 401") || s.starts_with("HTTP 403") || s.contains("не привязан к аккаунту")
        })
    }

    /// Validates the token by reading the account status.
    pub fn connect(token: &str) -> Result<Self> {
        let client = Client::new(token);
        let status = client.get("/account/status", &[]).context("Яндекс Музыка: вход не удался")?;
        let account = status.get("account").context("account/status: нет account")?;
        let Some(uid) = account.get("uid").and_then(Value::as_u64) else {
            bail!("токен не привязан к аккаунту (нет uid)");
        };
        let login = account.get("login").and_then(Value::as_str).unwrap_or("?");
        Ok(Self { client, uid, name: format!("YANDEX · {login}"), liked: Mutex::new(None) })
    }

    fn liked_ids(&self) -> Result<HashSet<String>> {
        // Take the hit, release the lock, then fetch: never hold it across a
        // network call, or a slow request stalls every other caller.
        let hit = self.liked.lock().unwrap().clone();
        if let Some(set) = hit {
            return Ok(set);
        }
        let ids: HashSet<String> = self.like_refs()?.into_iter().map(|(id, _)| id).collect();
        *self.liked.lock().unwrap() = Some(ids.clone());
        Ok(ids)
    }

    /// `(track_id, album_id)` of liked tracks, newest first.
    fn like_refs(&self) -> Result<Vec<(String, Option<String>)>> {
        let res = self.client.get(&format!("/users/{}/likes/tracks", self.uid), &[])?;
        let items = res.pointer("/library/tracks").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(items
            .iter()
            .filter_map(|v| Some((parse::id_str(v.get("id")?)?, v.get("albumId").and_then(parse::id_str))))
            .collect())
    }

    /// Liked tracks, newest first; `limit` caps how many are fetched.
    fn liked_tracks(&self, limit: Option<usize>) -> Result<Vec<Track>> {
        let refs = self.like_refs()?;
        *self.liked.lock().unwrap() = Some(refs.iter().map(|(id, _)| id.clone()).collect());
        let ids: Vec<String> = refs
            .into_iter()
            .take(limit.unwrap_or(usize::MAX))
            .map(|(id, album)| album.map_or(id.clone(), |a| format!("{id}:{a}")))
            .collect();
        let mut tracks = self.fetch_tracks(&ids)?;
        for t in &mut tracks {
            t.liked = true;
        }
        Ok(tracks)
    }

    fn mark(&self, mut tracks: Vec<Track>) -> Vec<Track> {
        if let Ok(liked) = self.liked_ids() {
            for t in &mut tracks {
                t.liked = liked.contains(&t.id);
            }
        }
        tracks
    }

    /// Full track objects for `id` or `id:albumId` references.
    fn fetch_tracks(&self, refs: &[String]) -> Result<Vec<Track>> {
        let mut out = Vec::with_capacity(refs.len());
        for chunk in refs.chunks(BATCH) {
            let res = self.client.post_form("/tracks", &[("track-ids", &chunk.join(",")), ("with-positions", "false")])?;
            out.extend(parse::tracks(res.as_array().into_iter().flatten()));
        }
        Ok(out)
    }

    fn file_info_stream(&self, track_id: &str, pref: Quality) -> Result<Option<Stream>> {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let (quality, codecs) = parse::file_info_params(pref);
        let transports = "raw";
        let sign = parse::file_info_sign(ts, track_id, quality, codecs, transports);
        let res = self.client.get_as(
            "/get-file-info",
            &[
                ("ts", &ts.to_string()),
                ("trackId", track_id),
                ("quality", quality),
                ("codecs", codecs),
                ("transports", transports),
                ("sign", &sign),
            ],
            client::DESKTOP,
        )?;
        Ok(parse::file_info(&res))
    }

    fn lyrics_in(&self, track_id: &str, format: &str) -> Result<Option<Lyrics>> {
        let ts = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let sign = parse::lyrics_sign(track_id, ts);
        let res = match self.client.get(
            &format!("/tracks/{track_id}/lyrics"),
            &[("format", format), ("timeStamp", &ts.to_string()), ("sign", &sign)],
        ) {
            Ok(v) => v,
            Err(e) if client::is_status(&e, 404) || client::is_status(&e, 400) => return Ok(None),
            Err(e) => return Err(e),
        };
        let Some(url) = res.get("downloadUrl").and_then(Value::as_str) else { return Ok(None) };
        let text = self.client.get_text(url, false)?;
        let lyrics = Lyrics::parse(&text);
        Ok((!lyrics.lines.is_empty()).then_some(lyrics))
    }

    /// Fallback when a track carries no artist id: search by name.
    fn artist_by_name(&self, artist: &str) -> Result<Vec<Track>> {
        let wanted = normalize(artist);
        let first = normalize(artist.split(", ").next().unwrap_or(artist));
        Ok(self
            .search(artist)?
            .into_iter()
            .filter(|t| {
                let a = normalize(&t.artist);
                a == wanted || a.split(", ").any(|x| x == first)
            })
            .collect())
    }
}

impl MusicService for YandexService {
    fn name(&self) -> &str {
        &self.name
    }

    fn search(&self, query: &str) -> Result<Vec<Track>> {
        let q = query.trim();
        if q.is_empty() {
            return Ok(Vec::new());
        }
        let res = self.client.get("/search", &[("text", q), ("type", "track"), ("page", "0"), ("nocorrect", "false")])?;
        let items = res.pointer("/tracks/results").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(self.mark(parse::tracks(&items)))
    }

    fn playlists(&self) -> Result<Vec<Playlist>> {
        let res = self.client.get(&format!("/users/{}/playlists/list", self.uid), &[])?;
        Ok(res.as_array().into_iter().flatten().filter_map(|v| parse::playlist(v, self.uid)).collect())
    }

    fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let Some((owner, kind)) = playlist_id.split_once(':') else { bail!("bad playlist id {playlist_id}") };
        let res = self.client.get(&format!("/users/{owner}/playlists/{kind}"), &[])?;
        let items = res.get("tracks").and_then(Value::as_array).cloned().unwrap_or_default();
        // Items carry a full `track` when the API inlines it; otherwise fetch.
        let mut inline = Vec::new();
        let mut missing = Vec::new();
        for item in &items {
            match item.get("track").and_then(parse::track) {
                Some(t) => inline.push(t),
                None => {
                    if let Some(id) = item.get("id").and_then(parse::id_str) {
                        missing.push(match item.get("albumId").and_then(parse::id_str) {
                            Some(a) => format!("{id}:{a}"),
                            None => id,
                        });
                    }
                }
            }
        }
        if !missing.is_empty() {
            inline.extend(self.fetch_tracks(&missing)?);
        }
        Ok(self.mark(inline))
    }

    fn my_wave(&self) -> Result<Vec<Track>> {
        let res = self.client.get("/rotor/station/user:onyourwave/tracks", &[("settings2", "true")])?;
        let seq = res.get("sequence").and_then(Value::as_array).cloned().unwrap_or_default();
        Ok(self.mark(seq.iter().filter_map(|s| s.get("track").and_then(parse::track)).collect()))
    }

    fn likes(&self) -> Result<Vec<Track>> {
        self.liked_tracks(Some(LIKES_LIMIT))
    }

    fn likes_all(&self) -> Result<Vec<Track>> {
        self.liked_tracks(None)
    }

    fn cached(&self) -> Result<Vec<Track>> {
        Ok(Vec::new()) // local cache index is not implemented yet
    }

    fn artist_tracks(&self, track: &Track, all: bool) -> Result<Vec<Track>> {
        let Some(id) = &track.artist_id else { return self.artist_by_name(&track.artist) };
        if !all {
            let res = self.client.get(&format!("/artists/{id}/brief-info"), &[])?;
            let popular = res.get("popularTracks").and_then(Value::as_array).cloned().unwrap_or_default();
            if !popular.is_empty() {
                return Ok(self.mark(parse::tracks(&popular)));
            }
        }
        // Every track, page by page (capped so huge catalogues stay responsive).
        let mut out = Vec::new();
        for page in 0..ARTIST_MAX_PAGES {
            let res = self.client.get(
                &format!("/artists/{id}/tracks"),
                &[("page", &page.to_string()), ("page-size", &ARTIST_PAGE.to_string())],
            )?;
            let items = res.get("tracks").and_then(Value::as_array).cloned().unwrap_or_default();
            let total = res.pointer("/pager/total").and_then(Value::as_u64).unwrap_or(0) as usize;
            let got = items.len();
            out.extend(parse::tracks(&items));
            if got < ARTIST_PAGE || (total > 0 && (page + 1) * ARTIST_PAGE >= total) || !all {
                break;
            }
        }
        Ok(self.mark(out))
    }
    fn set_liked(&mut self, track_id: &str, liked: bool) -> Result<()> {
        let action = if liked { "add-multiple" } else { "remove" };
        self.client.post_form(&format!("/users/{}/likes/tracks/{action}", self.uid), &[("track-ids", track_id)])?;
        if let Some(set) = self.liked.lock().unwrap().as_mut() {
            if liked {
                set.insert(track_id.to_string());
            } else {
                set.remove(track_id);
            }
        }
        Ok(())
    }

    fn lyrics(&self, track_id: &str) -> Result<Option<Lyrics>> {
        if let Some(l) = self.lyrics_in(track_id, "LRC")? {
            return Ok(Some(l));
        }
        self.lyrics_in(track_id, "TEXT")
    }

    /// Best available source: lossless/AAC via `/get-file-info` (what the
    /// current apps use), falling back to MP3 via the legacy download-info.
    fn stream(&self, track_id: &str, quality: Quality) -> Result<Option<Stream>> {
        // Any failure here (signature change, encrypted-only track) falls
        // through to the legacy path rather than breaking playback.
        let first = self.file_info_stream(track_id, quality).ok().flatten();
        let wanted_best = matches!(quality, Quality::Lossless | Quality::High);
        let good_enough = first.as_ref().is_some_and(|s| s.is_lossless() || !wanted_best || s.bitrate_kbps >= 256);
        if good_enough {
            return Ok(first);
        }
        // No lossless (or a weak lossy) for this track: when the listener
        // asked for the best, take the best lossy the API has — the "high"
        // tier and the legacy MP3 320 — rather than settling for 192.
        let mut candidates: Vec<Stream> = first.into_iter().collect();
        if quality == Quality::Lossless
            && let Ok(Some(high)) = self.file_info_stream(track_id, Quality::High)
        {
            candidates.push(high);
        }
        if candidates.iter().all(|c| c.bitrate_kbps < 320)
            && let Ok(legacy) = self.legacy_stream(track_id)
        {
            candidates.push(legacy);
        }
        // Prefer higher bitrate; on a tie prefer AAC (more efficient) over MP3.
        let best = candidates.into_iter().max_by_key(|c| (c.bitrate_kbps, c.codec.starts_with("aac")));
        match best {
            Some(s) => Ok(Some(s)),
            None => self.legacy_stream(track_id).map(Some),
        }
    }
}

impl YandexService {
    /// Legacy `download-info` → signed direct MP3 link (up to 320 kbps).
    fn legacy_stream(&self, track_id: &str) -> Result<Stream> {
        let res = self.client.get(&format!("/tracks/{track_id}/download-info"), &[])?;
        let Some((info_url, codec, bitrate_kbps)) = parse::best_download_info(&res) else {
            bail!("нет доступного потока");
        };
        let xml = self.client.get_text(&info_url, true)?;
        let info = parse::download_info_xml(&xml).context("download-info: неожиданный ответ")?;
        Ok(Stream { url: parse::direct_link(&info), codec, bitrate_kbps })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The service moves to a worker thread in 1.1.0, which only compiles if the
    /// whole value is `Send`. This test is the compile-time check for that:
    /// remove it and the guarantee quietly rots.
    #[test]
    fn service_can_move_between_threads() {
        fn assert_send<T: Send>() {}
        assert_send::<YandexService>();
    }
}

#[cfg(test)]
mod live {
    //! Live checks against the real API. Need a token in `YAMUSIQ_TOKEN` or
    //! `~/.config/yamusiq/token`; run with `cargo test live_ -- --ignored`.
    //! Output never includes the token or signed URLs.

    use super::*;

    fn service() -> YandexService {
        let token = crate::config::token().expect("no token configured");
        YandexService::connect(&token).expect("connect")
    }

    #[test]
    #[ignore]
    fn live_account_search_wave_likes_playlists() {
        let s = service();
        println!("account: {}", s.name());
        let found = s.search("Кино группа крови").unwrap();
        println!("search: {} tracks, first: {:?}", found.len(), found.first().map(|t| (&t.artist, &t.title)));
        assert!(!found.is_empty());
        let wave = s.my_wave().unwrap();
        println!("wave: {} tracks", wave.len());
        let likes = s.likes().unwrap();
        println!("likes: {} tracks", likes.len());
        let pls = s.playlists().unwrap();
        println!("playlists: {}", pls.len());
        if let Some(p) = pls.iter().find(|p| p.track_count > 0) {
            let t = s.playlist_tracks(&p.id).unwrap();
            println!("playlist {:?}: {} tracks", p.title, t.len());
        }
    }

    #[test]
    #[ignore]
    fn live_stream_decodes_and_timings() {
        use std::process::{Command, Stdio};
        use std::time::Instant;
        let s = service();
        let t0 = Instant::now();
        let t = s.search("Кино группа крови").unwrap().into_iter().next().expect("a track");
        println!("search: {:?}", t0.elapsed());
        let t1 = Instant::now();
        let stream = s.stream(&t.id, Quality::Lossless).unwrap().expect("stream");
        println!("stream: {} in {:?}", stream.label(), t1.elapsed());
        for q in Quality::ALL {
            println!("  {:8} → {}", q.id(), s.stream(&t.id, q).unwrap().map_or("none".into(), |s| s.label()));
        }
        let url = stream.url;
        // For latency experiments: write (never print) the short-lived link.
        if let Ok(path) = std::env::var("YAMUSIQ_URL_OUT") {
            std::fs::write(path, &url).unwrap();
        }
        let t2 = Instant::now();
        let lyrics = s.lyrics(&t.id).unwrap();
        println!("lyrics: {:?} ({} lines)", t2.elapsed(), lyrics.map_or(0, |l| l.lines.len()));
        // Decode 3 s of the real stream to raw PCM, as the player does.
        let t3 = Instant::now();
        let out = Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-ss", "30", "-t", "3", "-i", &url, "-f", "f32le", "-ac", "1", "-ar", "48000", "-"])
            .stderr(Stdio::piped())
            .output()
            .unwrap();
        assert!(out.status.success(), "ffmpeg failed: {}", String::from_utf8_lossy(&out.stderr).lines().next().unwrap_or(""));
        let samples: Vec<f32> = out.stdout.as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)).collect();
        println!("decoded {:.2} s of audio in {:?}", samples.len() as f64 / 48_000.0, t3.elapsed());
        assert!(samples.len() > 48_000 * 2, "at least 2 s decoded");
        let a = crate::audio::analyze_for_test(&samples, 48_000);
        let loud = a.bands.iter().filter(|&&b| b > 0.3).count();
        println!("analysis: level {:.2}, {loud}/48 bands above 0.3", a.level);
        // Band distribution over many windows, to calibrate the display range.
        let mut all: Vec<f32> = samples
            .windows(2048)
            .step_by(4096)
            .flat_map(|w| crate::audio::analyze_for_test(w, 48_000).bands)
            .collect();
        all.sort_by(f32::total_cmp);
        let q = |p: f32| all[((all.len() - 1) as f32 * p) as usize];
        println!("band values p10 {:.2} p50 {:.2} p90 {:.2} max {:.2}", q(0.1), q(0.5), q(0.9), q(1.0));
        // Time to first audio from the start of the track.
        let t4 = Instant::now();
        let mut child = Command::new("ffmpeg")
            .args(["-nostdin", "-loglevel", "error", "-i", &url, "-f", "f32le", "-ac", "2", "-ar", "48000", "-"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut first = [0u8; 4096];
        std::io::Read::read_exact(child.stdout.as_mut().unwrap(), &mut first).unwrap();
        println!("time to first audio: {:?}", t4.elapsed());
        let _ = child.kill();
        let _ = child.wait();
        assert!(a.level > 0.2, "real music is not silence");
    }

    #[test]
    #[ignore]
    fn live_lossless_availability() {
        let s = service();
        let mut lossy = 0;
        for t in s.search("Кино группа крови").unwrap().into_iter().take(6) {
            let st = s.stream(&t.id, Quality::Lossless).unwrap().expect("stream");
            if !st.is_lossless() {
                lossy += 1;
                let high = s.file_info_stream(&t.id, Quality::High).ok().flatten().map(|x| x.label());
                let legacy = s.legacy_stream(&t.id).ok().map(|x| x.label());
                println!("    alternatives: high tier {high:?}, legacy {legacy:?}");
            }
            println!("{:<28} → {}{}", t.title.chars().take(28).collect::<String>(), st.label(), if st.is_lossless() { "" } else { "  (lossless недоступен)" });
        }
        println!("{lossy} of 6 without lossless");
    }

    #[test]
    #[ignore]
    fn live_save_to_library() {
        use crate::library::{Finished, Library};
        use std::process::Command;
        use std::time::{Duration, Instant};
        let s = service();
        let t = s.search("Кино группа крови").unwrap().into_iter().next().expect("a track");
        let stream = s.stream(&t.id, Quality::Lossless).unwrap().expect("stream");
        let root = std::env::temp_dir().join(format!("yamusiq-live-lib-{}", std::process::id()));
        let mut lib = Library::open(root.clone());
        let t0 = Instant::now();
        lib.start(&t, &stream).unwrap();
        let mut done = Vec::new();
        while done.is_empty() && t0.elapsed() < Duration::from_secs(120) {
            std::thread::sleep(Duration::from_millis(200));
            done = lib.poll();
        }
        println!("saved in {:?}: {:?}", t0.elapsed(), done);
        assert!(matches!(done.as_slice(), [Finished::Saved { .. }]));
        let local = lib.local_stream(&t.id).expect("local copy");
        let probe = Command::new("ffprobe")
            .args(["-v", "error", "-show_entries", "stream=codec_name,sample_rate,bits_per_raw_sample:format=duration:format_tags=title,artist", "-of", "default=nw=1"])
            .arg(&local.url)
            .output()
            .unwrap();
        let info = String::from_utf8_lossy(&probe.stdout);
        println!("file: {}\n{}", lib.saved(&t.id).unwrap().file, info.trim());
        assert!(info.contains("codec_name=flac"));
        let secs: f64 = info.lines().find_map(|l| l.strip_prefix("duration=")).unwrap().parse().unwrap();
        assert!((secs - f64::from(t.duration_secs)).abs() < 2.0, "full track saved ({secs:.1}s)");
        let size = std::fs::metadata(&local.url).unwrap().len();
        println!("size: {:.1} MB", size as f64 / 1e6);
        lib.remove(&t.id).unwrap();
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    #[ignore]
    fn live_session_restore() {
        use crate::app::App;
        use crate::i18n::Lang;
        use crate::player::{PlayerState, SimPlayer};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let press = |a: &mut App, c| a.on_key(KeyEvent::new(c, KeyModifiers::NONE));

        let mut a = App::new(Box::new(service()), Box::new(SimPlayer::default()), Lang::Ru);
        press(&mut a, KeyCode::Char('w'));
        press(&mut a, KeyCode::Char('j'));
        press(&mut a, KeyCode::Enter);
        a.player.seek(std::time::Duration::from_secs(30));
        let json = serde_json::to_string(&a.snapshot()).unwrap();
        let title = a.current_track().unwrap().title.clone();

        let mut b = App::new(Box::new(service()), Box::new(SimPlayer::default()), Lang::Ru);
        b.restore(serde_json::from_str(&json).unwrap());
        println!("restored: {:?} at {:?}, state {:?}, quality {:?}", b.current_track().map(|t| &t.title), b.player.position(), b.player.state(), b.quality);
        assert_eq!(b.current_track().unwrap().title, title);
        assert_eq!(b.player.state(), PlayerState::Paused);
        assert!(b.player.position().as_secs() >= 30);
        assert!(b.quality.is_some(), "a fresh stream link was fetched");
    }

    #[test]
    #[ignore]
    fn live_stream_and_lyrics() {
        let s = service();
        let t = s.search("Кино группа крови").unwrap().into_iter().next().expect("a track");
        let stream = s.stream(&t.id, Quality::Lossless).unwrap().expect("stream");
        println!("stream: {} ({})", stream.label(), if stream.url.starts_with("https://") { "signed link, hidden" } else { "unexpected" });
        let popular = s.artist_tracks(&t, false).unwrap();
        let all = s.artist_tracks(&t, true).unwrap();
        println!("artist {}: popular {}, all {}", t.artist, popular.len(), all.len());
        assert!(all.len() > popular.len().min(5));
        let lyrics = s.lyrics(&t.id).unwrap();
        match &lyrics {
            Some(l) => println!("lyrics: {} lines, synced={}", l.lines.len(), l.synced),
            None => println!("lyrics: none for this track"),
        }
    }
}
