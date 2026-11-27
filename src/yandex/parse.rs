//! Pure helpers: JSON → model conversion, request signing, download-info XML.
//! Kept free of I/O so they can be tested against recorded response shapes.

use base64::Engine as _;
use hmac::{Hmac, KeyInit, Mac};
use md5::{Digest, Md5};
use serde_json::Value;
use sha2::Sha256;

use crate::model::{Playlist, Stream, Track};

/// Key used by the official clients to sign lyrics requests.
const LYRICS_SIGN_KEY: &[u8] = b"p93jhgh689SBReK6ghtw62";
/// Key used by the desktop client to sign `/get-file-info`.
const FILE_INFO_SIGN_KEY: &[u8] = b"kzqU4XhfCaY6B6JTHODeq5";
/// Codecs we can decode without decryption, best first.
pub const FILE_INFO_CODECS: &str = "flac-mp4,flac,aac-mp4,aac,he-aac-mp4,he-aac,mp3";
const LOSSY_CODECS: &str = "aac-mp4,aac,he-aac-mp4,he-aac,mp3";

/// `/get-file-info` `quality` and `codecs` for a preference. Verified live:
/// lossless+all → FLAC, lossless+lossy → AAC 256, nq → AAC 192, lq → HE-AAC 64.
pub fn file_info_params(q: crate::model::Quality) -> (&'static str, &'static str) {
    use crate::model::Quality::*;
    match q {
        Lossless => ("lossless", FILE_INFO_CODECS),
        High => ("lossless", LOSSY_CODECS),
        Normal => ("nq", LOSSY_CODECS),
        Low => ("lq", LOSSY_CODECS),
    }
}
/// Salt used to build direct MP3 links from download-info.
const DOWNLOAD_SALT: &str = "XGRlBW9FXlekgbPrRHuSiA";

/// IDs arrive as strings or numbers depending on the endpoint.
pub fn id_str(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

pub fn track(v: &Value) -> Option<Track> {
    if v.get("available").and_then(Value::as_bool) == Some(false) {
        return None; // region-locked or removed: cannot be played
    }
    let id = id_str(v.get("id")?)?;
    let mut title = v.get("title")?.as_str()?.to_string();
    if let Some(version) = v.get("version").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        title = format!("{title} ({version})");
    }
    let artist = v
        .get("artists")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|x| x.get("name")?.as_str()).collect::<Vec<_>>().join(", "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "—".into());
    let artist_id = v.get("artists").and_then(Value::as_array).and_then(|a| a.first()).and_then(|a| id_str(a.get("id")?));
    let album = v.get("albums").and_then(Value::as_array).and_then(|a| a.first());
    let album_id = album.and_then(|a| id_str(a.get("id")?));
    let album_title = album.and_then(|a| a.get("title")?.as_str()).unwrap_or("").to_string();
    let duration_ms = v.get("durationMs").and_then(Value::as_u64).unwrap_or(0);
    Some(Track {
        id,
        title,
        artist,
        album: album_title,
        duration_secs: ((duration_ms + 500) / 1000) as u32,
        liked: false,
        cached: false,
        album_id,
        artist_id,
        cover_uri: v.get("coverUri").and_then(Value::as_str).filter(|s| !s.is_empty())
            .or_else(|| album.and_then(|a| a.get("coverUri")?.as_str()).filter(|s| !s.is_empty()))
            .map(str::to_owned),
    })
}

pub fn tracks<'a>(items: impl IntoIterator<Item = &'a Value>) -> Vec<Track> {
    items.into_iter().filter_map(track).collect()
}

/// `/users/{uid}/playlists/list` item → playlist with id `"{owner}:{kind}"`.
pub fn playlist(v: &Value, fallback_owner: u64) -> Option<Playlist> {
    let kind = id_str(v.get("kind")?)?;
    let owner = v
        .get("owner")
        .and_then(|o| o.get("uid"))
        .or_else(|| v.get("uid"))
        .and_then(id_str)
        .unwrap_or_else(|| fallback_owner.to_string());
    Some(Playlist {
        id: format!("{owner}:{kind}"),
        title: v.get("title")?.as_str()?.to_string(),
        track_count: v.get("trackCount").and_then(Value::as_u64).unwrap_or(0) as usize,
    })
}

/// Personal landing entities are wrappers around ordinary playlist metadata.
/// Match stable generated types, never translated or user-editable titles.
pub fn home_playlists(result: &Value, owner: u64) -> Vec<crate::home::Recommendation> {
    use crate::home::{Kind, Recommendation};
    let mut found = Vec::new();
    for block in result.get("blocks").and_then(Value::as_array).into_iter().flatten() {
        if block.get("type").and_then(Value::as_str) != Some("personal-playlists") { continue; }
        for entity in block.get("entities").and_then(Value::as_array).into_iter().flatten() {
            if entity.get("type").and_then(Value::as_str) != Some("personal-playlist") { continue; }
            let Some(wrapper) = entity.get("data") else { continue };
            if wrapper.get("ready").and_then(Value::as_bool) == Some(false) { continue; }
            let Some(data) = wrapper.get("data") else { continue };
            let Some(kind) = data.get("generatedPlaylistType").or_else(|| wrapper.get("type"))
                .and_then(Value::as_str).and_then(Kind::from_api) else { continue };
            if found.iter().any(|p: &Recommendation| p.kind == kind) { continue; }
            let Some(playlist) = playlist(data, owner) else { continue };
            let cover_uri = data.pointer("/coverWithoutText/uri").and_then(Value::as_str)
                .or_else(|| data.pointer("/cover/uri").and_then(Value::as_str))
                .filter(|s| !s.is_empty()).map(str::to_owned);
            found.push(Recommendation { kind, playlist, cover_uri });
        }
    }
    found.sort_by_key(|p| Kind::ALL.iter().position(|k| *k == p.kind).unwrap_or(3));
    found
}

/// Lyrics request signature: base64(HMAC-SHA256(key, "{track_id}{timestamp}")).
pub fn lyrics_sign(track_id: &str, timestamp: u64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(LYRICS_SIGN_KEY).expect("HMAC accepts any key length");
    mac.update(format!("{track_id}{timestamp}").as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

/// `/get-file-info` signature: base64(HMAC-SHA256(key, ts + trackId +
/// quality + codecs-without-commas + transports)) without `=` padding.
pub fn file_info_sign(ts: u64, track_id: &str, quality: &str, codecs: &str, transports: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(FILE_INFO_SIGN_KEY).expect("HMAC accepts any key length");
    mac.update(format!("{ts}{track_id}{quality}{}{transports}", codecs.replace(',', "")).as_bytes());
    base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()).trim_end_matches('=').to_string()
}

/// `/get-file-info` result → stream, if it is a plain (unencrypted) source.
pub fn file_info(result: &Value) -> Option<Stream> {
    let info = result.get("downloadInfo")?;
    if info.get("transport").and_then(Value::as_str) != Some("raw") || info.get("key").is_some_and(|k| !k.is_null()) {
        return None;
    }
    let url = info
        .get("url")
        .and_then(Value::as_str)
        .or_else(|| info.get("urls")?.as_array()?.first()?.as_str())?
        .to_string();
    Some(Stream {
        url,
        codec: info.get("codec").and_then(Value::as_str).unwrap_or("?").to_string(),
        bitrate_kbps: info.get("bitrate").and_then(Value::as_u64).unwrap_or(0) as u32,
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct DownloadInfo {
    pub host: String,
    pub path: String,
    pub ts: String,
    pub s: String,
}

/// Extracts `<host>`, `<path>`, `<ts>`, `<s>` from the download-info XML.
pub fn download_info_xml(xml: &str) -> Option<DownloadInfo> {
    let tag = |name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let start = xml.find(&open)? + open.len();
        let end = start + xml[start..].find(&format!("</{name}>"))?;
        Some(xml[start..end].trim().to_string())
    };
    Some(DownloadInfo { host: tag("host")?, path: tag("path")?, ts: tag("ts")?, s: tag("s")? })
}

pub fn direct_link(d: &DownloadInfo) -> String {
    let path = d.path.strip_prefix('/').unwrap_or(&d.path);
    let sign = Md5::digest(format!("{DOWNLOAD_SALT}{path}{}", d.s).as_bytes());
    let hex: String = sign.iter().map(|b| format!("{b:02x}")).collect();
    format!("https://{}/get-mp3/{hex}/{}/{path}", d.host, d.ts)
}

/// Picks the best MP3 variant from `/tracks/{id}/download-info`, falling back
/// to AAC; returns its `downloadInfoUrl`, codec and bitrate.
pub fn best_download_info(result: &Value) -> Option<(String, String, u32)> {
    let items = result.as_array()?;
    let score = |v: &Value| {
        let codec = v.get("codec").and_then(Value::as_str).unwrap_or("");
        let bitrate = v.get("bitrateInKbps").and_then(Value::as_u64).unwrap_or(0);
        let rank = match codec {
            "mp3" => 2,
            "aac" | "he-aac" => 1,
            _ => 0,
        };
        (rank, bitrate)
    };
    items
        .iter()
        .filter(|v| score(v).0 > 0 && v.get("preview").and_then(Value::as_bool) != Some(true))
        .max_by_key(|v| score(v))
        .and_then(|v| {
            let url = v.get("downloadInfoUrl")?.as_str()?.to_string();
            let codec = v.get("codec")?.as_str()?.to_string();
            Some((url, codec, v.get("bitrateInKbps").and_then(Value::as_u64).unwrap_or(0) as u32))
        })
}

#[cfg(test)]
mod tests {

    #[test]
    fn home_personal_playlists_are_typed_ordered_and_keep_cover_metadata() {
        use serde_json::json;
        use crate::home::Kind;
        let entity = |kind: &str, id: u64| json!({"type": "personal-playlist", "data": {
            "type": kind, "ready": true, "data": {
                "kind": id, "uid": 42, "title": "A translated title", "trackCount": 25,
                "generatedPlaylistType": kind,
                "coverWithoutText": {"uri": "music-avatars.music.yandex.net/get-music-user-playlist/1/a/%%?123"}
            }
        }});
        let stash = entity("missedLikes", 3);
        let result = json!({"blocks": [{"type": "personal-playlists", "entities": [
            entity("recentTracks", 9), stash.clone(), entity("playlistOfTheDay", 2), entity("neverHeard", 1), stash
        ]}]});
        let cards = super::home_playlists(&result, 999);
        assert_eq!(cards.iter().map(|p| p.kind).collect::<Vec<_>>(), Kind::ALL);
        assert_eq!(cards[0].playlist.id, "42:1");
        assert_eq!(cards[1].playlist.track_count, 25);
        assert!(cards.iter().all(|p| p.cover_uri.as_deref().unwrap().ends_with("?123")));
    }

    #[test]
    fn home_ignores_unready_unknown_and_malformed_entities() {
        use serde_json::json;
        let result = json!({"blocks": [{"type": "personal-playlists", "entities": [
            {"type": "personal-playlist", "data": {"type": "neverHeard", "ready": false, "data": {"kind": 1, "title": "Not ready"}}},
            {"type": "personal-playlist", "data": {"type": "unknown", "data": {"kind": 2, "title": "Other"}}},
            {"type": "personal-playlist", "data": {"type": "playlistOfTheDay", "data": {"title": "No id"}}},
            {"type": "other", "data": {"type": "missedLikes", "data": {"kind": 3, "title": "Not a playlist"}}}
        ]}]});
        assert!(super::home_playlists(&result, 1).is_empty());
        assert!(super::home_playlists(&json!({}), 1).is_empty());
    }
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_track_variants() {
        let v = json!({
            "id": "123", "title": "Song", "version": "Live", "durationMs": 187_400,
            "artists": [{"id": 9, "name": "A"}, {"name": "B"}],
            "albums": [{"id": 555, "title": "Album"}], "available": true
        });
        let t = track(&v).unwrap();
        assert_eq!((t.id.as_str(), t.title.as_str(), t.artist.as_str(), t.album.as_str()), ("123", "Song (Live)", "A, B", "Album"));
        assert_eq!(t.duration_secs, 187);
        assert_eq!(t.album_id.as_deref(), Some("555"));
        assert_eq!(t.artist_id.as_deref(), Some("9"));

        assert!(track(&json!({"id": 1, "title": "x", "available": false})).is_none());
        let bare = track(&json!({"id": 7, "title": "x"})).unwrap();
        assert_eq!((bare.id.as_str(), bare.artist.as_str(), bare.album_id), ("7", "—", None));
    }

    #[test]
    fn cover_metadata_prefers_track_then_album_and_old_tracks_still_load() {
        let mut v = json!({"id": 1, "title": "Song", "coverUri": "track/%%",
            "albums": [{"id": 2, "coverUri": "album/%%"}]});
        assert_eq!(track(&v).unwrap().cover_uri.as_deref(), Some("track/%%"));
        v["coverUri"] = "".into();
        assert_eq!(track(&v).unwrap().cover_uri.as_deref(), Some("album/%%"));
        v.as_object_mut().unwrap().remove("coverUri");
        assert_eq!(track(&v).unwrap().cover_uri.as_deref(), Some("album/%%"));
        let old = track(&json!({"id": 3, "title": "Old"})).unwrap();
        let mut saved = serde_json::to_value(&old).unwrap();
        saved.as_object_mut().unwrap().remove("cover_uri");
        assert!(serde_json::from_value::<Track>(saved).unwrap().cover_uri.is_none());
        let restored: Track = serde_json::from_value(serde_json::to_value(track(&v).unwrap()).unwrap()).unwrap();
        assert_eq!(restored.cover_uri.as_deref(), Some("album/%%"));
    }

    #[test]
    fn parses_playlists() {
        let p = playlist(&json!({"kind": 3, "title": "Mix", "trackCount": 12, "owner": {"uid": 99}}), 1).unwrap();
        assert_eq!((p.id.as_str(), p.title.as_str(), p.track_count), ("99:3", "Mix", 12));
        let p = playlist(&json!({"kind": 1000, "title": "Mine"}), 42).unwrap();
        assert_eq!(p.id, "42:1000");
    }

    #[test]
    fn signs_like_the_reference_implementation() {
        // Vectors computed independently with Python's hmac/hashlib.
        assert_eq!(lyrics_sign("12345", 1_700_000_000), "Ucx7ny+5EZuyvj1G6+agFdud6MV8nQXkvdrNs95cyEM=");
        let d = DownloadInfo { host: "s1.example".into(), path: "/abc/def.mp3".into(), ts: "0005f".into(), s: "xyz".into() };
        assert_eq!(direct_link(&d), "https://s1.example/get-mp3/018bd052f726715baee932ed27d907fc/0005f/abc/def.mp3");
    }

    #[test]
    fn signs_file_info_and_parses_it() {
        // Vector computed independently with Python's hmac.
        assert_eq!(file_info_sign(1_700_000_000, "12345", "lossless", FILE_INFO_CODECS, "raw"), "O0VvWI2JxeDGla1FGaupHqZR/fauZDHc8LkR/OKU/D4");
        let ok = json!({"downloadInfo": {"codec": "flac-mp4", "bitrate": 0, "transport": "raw", "url": "https://x/a", "urls": ["https://x/a"]}});
        assert_eq!(file_info(&ok), Some(Stream { url: "https://x/a".into(), codec: "flac-mp4".into(), bitrate_kbps: 0 }));
        let only_urls = json!({"downloadInfo": {"codec": "mp3", "bitrate": 320, "transport": "raw", "urls": ["https://y"]}});
        assert_eq!(file_info(&only_urls).unwrap().url, "https://y");
        let encrypted = json!({"downloadInfo": {"codec": "flac-mp4", "transport": "encraw", "key": "abc", "url": "https://z"}});
        assert_eq!(file_info(&encrypted), None);
    }

    #[test]
    fn parses_download_info() {
        let xml = r#"<?xml version="1.0"?><download-info><host>h.example</host><path>/p/x.mp3</path><ts>1a</ts><region>0</region><s>sig</s></download-info>"#;
        assert_eq!(
            download_info_xml(xml),
            Some(DownloadInfo { host: "h.example".into(), path: "/p/x.mp3".into(), ts: "1a".into(), s: "sig".into() })
        );
        assert!(download_info_xml("<nope/>").is_none());

        let variants = json!([
            {"codec": "aac", "bitrateInKbps": 192, "downloadInfoUrl": "aac192"},
            {"codec": "mp3", "bitrateInKbps": 192, "downloadInfoUrl": "mp3-192"},
            {"codec": "mp3", "bitrateInKbps": 320, "downloadInfoUrl": "mp3-320"},
            {"codec": "mp3", "bitrateInKbps": 320, "preview": true, "downloadInfoUrl": "preview"},
        ]);
        assert_eq!(best_download_info(&variants), Some(("mp3-320".into(), "mp3".into(), 320)));
    }
}
