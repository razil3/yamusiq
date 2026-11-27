//! Independent, latest-request-only artwork worker. Never sends account tokens.
//! Only compressed JPEG/PNG bytes cross threads; AppKit stays on the TUI thread.
use std::{
    collections::VecDeque,
    io::Read,
    sync::{Arc, Condvar, Mutex},
    time::{Duration, Instant},
};

pub(super) type Bytes = Arc<[u8]>;
const MAX_BYTES: usize = 2 * 1024 * 1024;
const CACHE_ENTRIES: usize = 8;
const FAILURE_TTL: Duration = Duration::from_secs(60);

fn image_url(uri: &str) -> Option<String> {
    if uri.len() > 2048 || uri.chars().any(char::is_control) {
        return None;
    }
    let uri = uri.trim().replace("%%", "600x600");
    let uri = if uri.starts_with("https://") {
        uri
    } else {
        format!("https://{uri}")
    };
    let url = url::Url::parse(&uri).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
        || url.query().is_some_and(|q| q.is_empty() || q.len() > 20 || !q.bytes().all(|b| b.is_ascii_digit()))
        || url.fragment().is_some()
        || !matches!(
            url.host_str()?,
            "avatars.yandex.net"
                | "avatars.mds.yandex.net"
                | "avatars.mdst.yandex.net"
                | "music-avatars.music.yandex.net"
        )
        || !(url.path().starts_with("/get-music-content/")
            || url.path().starts_with("/get-music-user-playlist/"))
    {
        return None;
    }
    Some(url.into())
}

/// Inspect dimensions before asking AppKit to decode. Covers are requested at
/// 600px; reject unexpected large images and non-raster formats (e.g. SVG).
fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        if bytes.get(8..16)? != b"\0\0\0\rIHDR" {
            return None;
        }
        return Some((
            u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?),
            u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?),
        ));
    }
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut at = 2;
    while at < bytes.len() {
        if *bytes.get(at)? != 0xff {
            return None;
        }
        while *bytes.get(at)? == 0xff {
            at += 1;
        }
        let marker = *bytes.get(at)?;
        at += 1;
        if matches!(marker, 0xda | 0xd9) {
            return None;
        }
        if matches!(marker, 0x01 | 0xd0..=0xd8) {
            continue;
        }
        let len = u16::from_be_bytes(bytes.get(at..at + 2)?.try_into().ok()?) as usize;
        if len < 2 {
            return None;
        }
        let segment = bytes.get(at..at + len)?;
        if matches!(marker, 0xc0..=0xc3 | 0xc5..=0xc7 | 0xc9..=0xcb | 0xcd..=0xcf) {
            let height = u16::from_be_bytes(segment.get(3..5)?.try_into().ok()?);
            let width = u16::from_be_bytes(segment.get(5..7)?.try_into().ok()?);
            return Some((width.into(), height.into()));
        }
        at += len;
    }
    None
}

fn bounded_image(bytes: Vec<u8>) -> Option<Bytes> {
    if bytes.len() > MAX_BYTES {
        return None;
    }
    let (width, height) = dimensions(&bytes)?;
    if !(1..=1200).contains(&width) || !(1..=1200).contains(&height) {
        return None;
    }
    Some(bytes.into())
}

fn download(agent: &ureq::Agent, url: &str) -> Option<Bytes> {
    let response = agent.get(url).call().ok()?;
    // Redirects are disabled: never follow a CDN response to an arbitrary host.
    if response.status() != 200
        || response
            .header("Content-Length")
            .and_then(|s| s.parse::<usize>().ok())
            .is_some_and(|n| n > MAX_BYTES)
    {
        return None;
    }
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take((MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .ok()?;
    bounded_image(bytes)
}

#[derive(Default)]
struct Cache(VecDeque<(String, Option<Bytes>, Instant)>);
impl Cache {
    fn get(&mut self, key: &str) -> Option<Option<Bytes>> {
        let at = self.0.iter().position(|(k, _, _)| k == key)?;
        let entry = self.0.remove(at)?;
        if entry.1.is_none() && entry.2.elapsed() >= FAILURE_TTL {
            return None;
        }
        let bytes = entry.1.clone();
        self.0.push_back(entry);
        Some(bytes)
    }
    fn insert(&mut self, key: String, bytes: Option<Bytes>) {
        self.0.retain(|(k, _, _)| k != &key);
        self.0.push_back((key, bytes, Instant::now()));
        while self.0.len() > CACHE_ENTRIES {
            self.0.pop_front();
        }
    }
}

#[derive(Default)]
struct State {
    generation: u64,
    wanted: Option<String>,
    pending: bool,
    ready: Option<Option<Bytes>>,
    stopped: bool,
}
#[derive(Default)]
struct Shared {
    state: Mutex<State>,
    wake: Condvar,
}

pub(super) struct Loader {
    shared: Arc<Shared>,
    current: Option<String>,
}
impl Loader {
    pub fn new() -> Self {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(3))
            .timeout(Duration::from_secs(6))
            .build();
        Self::with_fetch(move |url| download(&agent, url))
    }
    fn with_fetch(fetch: impl Fn(&str) -> Option<Bytes> + Send + 'static) -> Self {
        let shared = Arc::new(Shared::default());
        let worker = shared.clone();
        // Failure to start optional artwork must not stop audio or the TUI.
        let _ = std::thread::Builder::new()
            .name("yamusiq-artwork".into())
            .spawn(move || {
                let mut cache = Cache::default();
                loop {
                    let (generation, wanted) = {
                        let mut state = worker.state.lock().unwrap();
                        while !state.pending && !state.stopped {
                            state = worker.wake.wait(state).unwrap();
                        }
                        if state.stopped {
                            return;
                        }
                        state.pending = false;
                        (state.generation, state.wanted.clone())
                    };
                    let bytes = wanted.as_deref().and_then(|key| {
                        cache.get(key).unwrap_or_else(|| {
                            let bytes = fetch(key);
                            cache.insert(key.to_owned(), bytes.clone());
                            bytes
                        })
                    });
                    let mut state = worker.state.lock().unwrap();
                    if state.stopped {
                        return;
                    }
                    if state.generation == generation {
                        state.ready = Some(bytes);
                    }
                }
            });
        Self {
            shared,
            current: None,
        }
    }
    /// True on a source change, so the caller removes the preceding cover now.
    pub fn request(&mut self, uri: Option<&str>) -> bool {
        let wanted = uri.and_then(image_url);
        if wanted == self.current {
            return false;
        }
        self.current = wanted.clone();
        let mut state = self.shared.state.lock().unwrap();
        state.generation = state.generation.wrapping_add(1);
        state.wanted = wanted;
        state.ready = None;
        state.pending = true;
        self.shared.wake.notify_one();
        true
    }
    pub fn poll(&self) -> Option<Option<Bytes>> {
        self.shared.state.lock().unwrap().ready.take()
    }
}
impl Drop for Loader {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock().unwrap();
        state.stopped = true;
        self.shared.wake.notify_one();
        // Do not join an in-flight HTTP request on the UI/exit path.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn only_https_yandex_music_images_are_allowed() {
        assert_eq!(
            image_url("avatars.yandex.net/get-music-content/1/abc/%%").as_deref(),
            Some("https://avatars.yandex.net/get-music-content/1/abc/600x600")
        );
        assert!(image_url("https://avatars.mds.yandex.net/get-music-content/1/%%").is_some());
        assert!(image_url("music-avatars.music.yandex.net/get-music-content/1/%%").is_some());
        for url in [
            "https://evil.test/a",
            "http://avatars.yandex.net/get-music-content/1/%%",
            "https://avatars.yandex.net.evil.test/get-music-content/1/%%",
            "https://music-avatars.music.yandex.net.evil.test/get-music-content/1/%%",
            "https://user:password@avatars.yandex.net/get-music-content/1/%%",
            "https://avatars.yandex.net:444/get-music-content/1/%%",
            "https://avatars.yandex.net/get-music-content/1/%%?url=http://localhost",
            "https://avatars.yandex.net/get-music-content/1/%%#x",
            "file:///etc/passwd",
            "https://avatars.yandex.net/other/%%",
            "\n",
        ] {
            assert!(image_url(url).is_none(), "accepted {url}");
        }
    }

    #[test]
    fn playlist_artwork_accepts_only_numeric_cache_versions_on_the_same_cdn() {
        assert_eq!(image_url("music-avatars.music.yandex.net/get-music-user-playlist/1/a/%%?1730453320815").as_deref(),
            Some("https://music-avatars.music.yandex.net/get-music-user-playlist/1/a/600x600?1730453320815"));
        for uri in [
            "music-avatars.music.yandex.net/get-music-user-playlist/1/a/%%?redirect=https://evil.test",
            "music-avatars.music.yandex.net/get-music-user-playlist/1/a/%%?123&host=evil.test",
            "evil.test/get-music-user-playlist/1/a/%%?123",
            "music-avatars.music.yandex.net/other/1/a/%%?123",
        ] { assert!(image_url(uri).is_none()); }
    }

    #[test]
    fn image_size_and_format_are_bounded_before_decode() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend(600u32.to_be_bytes());
        png.extend(600u32.to_be_bytes());
        assert!(bounded_image(png.clone()).is_some());
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(bounded_image(png).is_none());
        assert!(bounded_image(vec![0; MAX_BYTES + 1]).is_none());
        assert!(bounded_image(b"<svg/>".to_vec()).is_none());
        assert!(bounded_image(vec![0xff, 0xd8, 0xff, 0xe0, 0, 0]).is_none());
        let jpeg = vec![0xff, 0xd8, 0xff, 0xc0, 0, 8, 8, 2, 88, 2, 88, 0];
        assert_eq!(dimensions(&jpeg), Some((600, 600)));
        for len in 0..jpeg.len() {
            assert!(dimensions(&jpeg[..len]).is_none());
        }
    }

    #[test]
    fn http_download_has_no_auth_and_rejects_redirects_and_oversize() {
        use std::io::Write;
        use std::net::TcpListener;
        let image = include_bytes!("../../assets/macos/AppIcon.png");
        for status in [200, 302, 413] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}/cover", listener.local_addr().unwrap());
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    request.push(byte[0]);
                    assert!(request.len() < 8192);
                }
                let request = String::from_utf8(request).unwrap().to_lowercase();
                assert!(!request.contains("authorization:"));
                assert!(!request.contains("cookie:"));
                let response = match status {
                    200 => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", image.len()),
                    302 => "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".into(),
                    _ => format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", MAX_BYTES + 1),
                };
                stream.write_all(response.as_bytes()).unwrap();
                if status == 200 {
                    stream.write_all(image).unwrap();
                }
            });
            let agent = ureq::AgentBuilder::new()
                .redirects(0)
                .timeout(Duration::from_secs(3))
                .build();
            assert_eq!(download(&agent, &url).is_some(), status == 200);
            server.join().unwrap();
        }
    }

    #[test]
    fn cache_is_lru_bounded_and_failures_expire() {
        let mut cache = Cache::default();
        for i in 0..CACHE_ENTRIES {
            cache.insert(i.to_string(), Some(Arc::from([1u8])));
        }
        assert!(cache.get("0").is_some());
        cache.insert("new".into(), None);
        assert!(cache.get("1").is_none());
        assert!(cache.get("0").is_some());
        assert_eq!(cache.get("new"), Some(None));
        cache.0.back_mut().unwrap().2 -= FAILURE_TTL;
        assert_eq!(cache.get("new"), None);
        assert!(cache.0.len() <= CACHE_ENTRIES);
    }

    #[test]
    fn obsolete_downloads_never_replace_the_latest_cover() {
        let (called, calls) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut loader = Loader::with_fetch(move |url| {
            called.send(url.to_owned()).unwrap();
            gate.recv_timeout(Duration::from_secs(3)).unwrap();
            Some(Arc::from([url.as_bytes()[url.len() - 1]]))
        });
        let a = "avatars.yandex.net/get-music-content/a";
        let b = "avatars.yandex.net/get-music-content/b";
        let c = "avatars.yandex.net/get-music-content/c";
        assert!(loader.request(Some(a)));
        assert!(
            calls
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .ends_with("/a")
        );
        assert!(!loader.request(Some(a)));
        assert!(loader.poll().is_none()); // request never waits for HTTP
        loader.request(Some(b));
        loader.request(Some(c));
        release.send(()).unwrap();
        assert!(
            calls
                .recv_timeout(Duration::from_secs(3))
                .unwrap()
                .ends_with("/c")
        );
        assert!(loader.poll().is_none()); // completed A was discarded
        loader.request(None);
        release.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(bytes) = loader.poll() {
                assert!(bytes.is_none());
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        loader.request(Some(c)); // the stale download is still useful in cache
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(bytes) = loader.poll() {
                assert_eq!(&*bytes.unwrap(), b"c");
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert!(calls.try_recv().is_err());
    }
}
