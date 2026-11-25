//! Test fixture: an offline in-memory catalogue implementing
//! [`MusicService`], with original lyrics for a few fictional songs. Only
//! compiled for tests — the app itself always talks to Yandex Music.

pub mod lyrics;

use anyhow::{bail, Result};

use crate::model::{Lyrics, Playlist, Track};
use crate::service::{normalize, MusicService};

pub struct FakeService {
    tracks: Vec<Track>,
    /// Playlists with their member track ids.
    playlists: Vec<(Playlist, Vec<String>)>,
}

fn pl(id: &str, title: &str, ids: &[&str]) -> (Playlist, Vec<String>) {
    let ids: Vec<String> = ids.iter().map(|s| s.to_string()).collect();
    (Playlist { id: id.into(), title: title.into(), track_count: ids.len() }, ids)
}

fn t(id: &str, title: &str, artist: &str, album: &str, secs: u32, liked: bool, cached: bool) -> Track {
    Track {
        id: id.into(),
        title: title.into(),
        artist: artist.into(),
        album: album.into(),
        duration_secs: secs,
        liked,
        cached,
        album_id: None,
        artist_id: None,
    }
}

impl Default for FakeService {
    fn default() -> Self {
        let tracks = vec![
            t("1", "Night Drive", "Neon Harbor", "Coastline", 214, true, true),
            t("2", "Static Bloom", "Neon Harbor", "Coastline", 187, false, false),
            t("3", "Low Tide", "Neon Harbor", "Undertow", 242, false, true),
            t("4", "Paper Satellites", "Mira Vale", "Orbits", 198, true, false),
            t("5", "Glass Rooms", "Mira Vale", "Orbits", 231, false, false),
            t("6", "Copper Sky", "Atlas Quiet", "Field Notes", 276, true, true),
            t("7", "Winter Signal", "Atlas Quiet", "Field Notes", 205, false, false),
            t("8", "Slow Engines", "Kettle & Rail", "Junction", 189, false, false),
            t("9", "Lanterns", "Kettle & Rail", "Junction", 223, true, false),
            t("10", "Afterimage", "Pale Circuit", "Phosphor", 254, false, true),
            t("11", "Северное сияние", "Белые ночи", "Ладога", 231, true, false),
            t("12", "Ёлки в инее", "Белые ночи", "Ладога", 198, false, true),
            t("13", "Электричка", "Ретроград", "Перрон", 205, false, false),
            t("14", "Неоновый проспект", "Ретроград", "Перрон", 243, true, false),
        ];
        let playlists = vec![
            pl("p1", "Ночной город", &["1", "3", "10"]),
            pl("p2", "Фокус", &["6", "7", "4", "5"]),
            pl("p3", "По-русски", &["11", "12", "13", "14"]),
        ];
        Self { tracks, playlists }
    }
}

impl FakeService {
    fn filter(&self, pred: impl Fn(&Track) -> bool) -> Vec<Track> {
        self.tracks.iter().filter(|t| pred(t)).cloned().collect()
    }
}

impl MusicService for FakeService {
    fn name(&self) -> &str {
        "TEST"
    }

    fn search(&self, query: &str) -> Result<Vec<Track>> {
        let q = normalize(query.trim());
        if q.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.filter(|t| {
            normalize(&t.title).contains(&q) || normalize(&t.artist).contains(&q) || normalize(&t.album).contains(&q)
        }))
    }

    fn playlists(&self) -> Result<Vec<Playlist>> {
        Ok(self.playlists.iter().map(|(p, _)| p.clone()).collect())
    }

    fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>> {
        let Some((_, ids)) = self.playlists.iter().find(|(p, _)| p.id == playlist_id) else {
            bail!("unknown playlist {playlist_id}");
        };
        Ok(ids
            .iter()
            .filter_map(|id| self.tracks.iter().find(|t| &t.id == id).cloned())
            .collect())
    }

    fn my_wave(&self) -> Result<Vec<Track>> {
        // Deterministic "recommendation": everything not yet liked, then liked.
        let mut out = self.filter(|t| !t.liked);
        out.extend(self.filter(|t| t.liked));
        Ok(out)
    }

    fn likes(&self) -> Result<Vec<Track>> {
        Ok(self.filter(|t| t.liked))
    }

    fn cached(&self) -> Result<Vec<Track>> {
        Ok(self.filter(|t| t.cached))
    }

    fn artist_tracks(&self, track: &Track, _all: bool) -> Result<Vec<Track>> {
        let a = normalize(&track.artist);
        Ok(self.filter(|t| normalize(&t.artist) == a))
    }

    fn set_liked(&mut self, track_id: &str, liked: bool) -> Result<()> {
        let Some(track) = self.tracks.iter_mut().find(|t| t.id == track_id) else {
            bail!("unknown track {track_id}");
        };
        track.liked = liked;
        Ok(())
    }

    fn lyrics(&self, track_id: &str) -> Result<Option<Lyrics>> {
        Ok(crate::testing::lyrics::for_track(track_id).map(Lyrics::parse))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_lyrics_synced_and_plain() {
        let s = FakeService::default();
        assert!(s.lyrics("2").unwrap().unwrap().synced);
        assert!(s.lyrics("11").unwrap().unwrap().synced);
        assert!(!s.lyrics("13").unwrap().unwrap().synced);
        assert!(s.lyrics("5").unwrap().is_none());
    }

    #[test]
    fn search_matches_title_artist_album_case_insensitively() {
        let s = FakeService::default();
        assert_eq!(s.search("night").unwrap().len(), 1);
        assert_eq!(s.search("NEON").unwrap().len(), 3);
        assert_eq!(s.search("orbits").unwrap().len(), 2);
        assert!(s.search("   ").unwrap().is_empty());
        assert_eq!(s.search("ЕЛКИ").unwrap().len(), 1);
        assert_eq!(s.search("ёлки").unwrap().len(), 1);
        assert_eq!(s.search("ретроград").unwrap().len(), 2);
    }

    #[test]
    fn playlist_tracks_preserve_order() {
        let s = FakeService::default();
        let ids: Vec<_> = s.playlist_tracks("p2").unwrap().into_iter().map(|t| t.id).collect();
        assert_eq!(ids, ["6", "7", "4", "5"]);
        assert!(s.playlist_tracks("nope").is_err());
    }

    #[test]
    fn liking_updates_likes() {
        let mut s = FakeService::default();
        let before = s.likes().unwrap().len();
        s.set_liked("2", true).unwrap();
        assert_eq!(s.likes().unwrap().len(), before + 1);
    }
}
