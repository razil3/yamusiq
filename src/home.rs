//! The three personal mixes shown on Home, independent of the playback queue.
use crate::{i18n::Lang, model::Playlist};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Kind { DejaVu, Daily, Stash }
impl Kind {
    pub const ALL: [Self; 3] = [Self::DejaVu, Self::Daily, Self::Stash];

    pub fn from_api(value: &str) -> Option<Self> {
        match value {
            "neverHeard" => Some(Self::DejaVu),
            "playlistOfTheDay" => Some(Self::Daily),
            "missedLikes" => Some(Self::Stash),
            _ => None,
        }
    }

    pub fn title(self, lang: Lang) -> &'static str {
        match (self, lang) {
            (Self::DejaVu, Lang::Ru) => "Дежавю",
            (Self::Daily, Lang::Ru) => "Плейлист дня",
            (Self::Stash, Lang::Ru) => "Тайник",
            (Self::DejaVu, Lang::En) => "Deja Vu",
            (Self::Daily, Lang::En) => "Playlist of the Day",
            (Self::Stash, Lang::En) => "Stash",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recommendation {
    pub kind: Kind,
    pub playlist: Playlist,
    pub cover_uri: Option<String>,
}

#[derive(Default)]
pub struct Home {
    pub playlists: Vec<Recommendation>,
    pub loaded: bool,
    pub loading: bool,
    pub failed: bool,
    covers: [crate::media::PreviewLoader; 3],
}
impl Home {
    pub fn playlist(&self, index: usize) -> Option<&Recommendation> {
        let kind = Kind::ALL.get(index)?;
        self.playlists.iter().find(|p| p.kind == *kind)
    }

    pub fn finish(&mut self, playlists: Vec<Recommendation>) {
        self.playlists = playlists;
        self.loaded = true;
        self.loading = false;
        self.failed = false;
        for (index, kind) in Kind::ALL.into_iter().enumerate() {
            let uri = self.playlists.iter().find(|p| p.kind == kind).and_then(|p| p.cover_uri.as_deref());
            self.covers[index].request(uri);
        }
    }

    pub fn poll_covers(&mut self) {
        for cover in &mut self.covers { cover.poll(); }
    }

    #[cfg(test)]
    pub fn set_preview(&mut self, index: usize, preview: std::sync::Arc<crate::cover::Preview>) {
        self.covers[index].preview = Some(preview);
    }

    pub fn preview(&self, index: usize) -> Option<&crate::cover::Preview> {
        self.covers.get(index)?.preview.as_deref()
    }
}
