//! Public track links only. Never fetch an arbitrary pasted URL or invoke a shell.
use anyhow::{bail, Context, Result};
use crate::model::Track;

fn numeric(s: &str) -> bool {
    !s.is_empty() && s.len() <= 30 && s.bytes().all(|c| c.is_ascii_digit())
}

pub fn track_id(input: &str) -> Result<String> {
    let input = input.trim();
    if numeric(input) { return Ok(input.to_string()); }
    if let Some((track, album)) = input.split_once(':')
        && numeric(track) && numeric(album)
    {
        return Ok(format!("{track}:{album}"));
    }
    if input.len() > 2048 { bail!("invalid track link"); }
    let url = url::Url::parse(input).context("expected a track ID or Yandex Music link")?;
    let host = url.host_str().unwrap_or_default();
    if !matches!(url.scheme(), "https" | "http")
        || !matches!(host, "music.yandex.ru" | "music.yandex.com" | "music.yandex.kz" | "music.yandex.by" | "music.yandex.uz" | "next.music.yandex.ru")
        || !url.username().is_empty() || url.password().is_some() || url.port().is_some()
    {
        bail!("expected a Yandex Music track link");
    }
    let path: Vec<_> = url.path().trim_matches('/').split('/').collect();
    match path.as_slice() {
        ["track", id] if numeric(id) => Ok((*id).to_string()),
        ["album", album, "track", id] if numeric(id) && numeric(album) => Ok(format!("{id}:{album}")),
        _ => bail!("expected /track/ID or /album/ID/track/ID"),
    }
}

pub fn track_url(track: &Track) -> Result<String> {
    if !numeric(&track.id) { bail!("invalid track ID"); }
    Ok(match track.album_id.as_deref().filter(|id| numeric(id)) {
        Some(album) => format!("https://music.yandex.ru/album/{album}/track/{}", track.id),
        None => format!("https://music.yandex.ru/track/{}", track.id),
    })
}

pub fn copy(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("/usr/bin/pbcopy")
        .env("LC_CTYPE", "UTF-8")
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null())
        .spawn().context("cannot start pbcopy")?;
    let write_result = child.stdin.take().context("pbcopy stdin")?.write_all(text.as_bytes());
    let status = child.wait().context("waiting for pbcopy")?;
    write_result.context("writing clipboard")?;
    if !status.success() { bail!("pbcopy failed"); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_url_is_public_and_round_trips() {
        let track = crate::model::Track {
            id: "123".into(), album_id: Some("456".into()), title: "test".into(), artist: "test".into(),
            album: "test".into(), duration_secs: 1, liked: false, cached: false, artist_id: None, cover_uri: None,
        };
        let public = track_url(&track).unwrap();
        assert_eq!(public, "https://music.yandex.ru/album/456/track/123");
        assert_eq!(track_id(&public).unwrap(), "123:456");
        assert!(!public.contains('?'));
        let mut track = track;
        track.album_id = None;
        assert_eq!(track_url(&track).unwrap(), "https://music.yandex.ru/track/123");
    }

    #[test]
    fn accepts_ids_and_public_track_links_only() {
        for (input, expected) in [
            (" 123 ", "123"), ("123:45", "123:45"),
            ("https://music.yandex.ru/album/45/track/123?utm_source=share", "123:45"),
            ("https://music.yandex.com/track/123/", "123"),
        ] { assert_eq!(track_id(input).unwrap(), expected); }
        for input in ["", "abc", "123;whoami", "https://evil.test/track/123", "https://music.yandex.ru.evil.test/track/123", "https://music.yandex.ru@evil.test/track/123", "file:///track/123", "https://music.yandex.ru/album/45", "https://music.yandex.ru/track/%31", "https://music.yandex.ru/track/123/extra"] {
            assert!(track_id(input).is_err(), "accepted {input}");
        }
    }
}
