//! Persisted UI preferences (`~/.config/yamusiq/config.json`).

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::i18n::Lang;
use crate::model::Quality;

#[derive(Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// Save every track that starts playing (`--download-all`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_download: Option<bool>,
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(dir).join("yamusiq/config.json"));
        }
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config/yamusiq/config.json"))
    }

    /// Missing or unreadable config falls back to defaults: preferences must
    /// never stop the player from starting. Unknown keys (e.g. the retired
    /// `theme`) are ignored.
    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::path().context("no HOME to store config in")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, serde_json::to_string_pretty(self)? + "\n")
            .with_context(|| format!("writing {}", path.display()))
    }

    /// `YAMUSIQ_LANG` overrides the stored language; Russian is the default.
    pub fn resolve(&self) -> Lang {
        std::env::var("YAMUSIQ_LANG")
            .ok()
            .or_else(|| self.lang.clone())
            .and_then(|id| Lang::from_id(&id))
            .unwrap_or(Lang::Ru)
    }

    pub fn quality(&self) -> Quality {
        self.quality.as_deref().and_then(Quality::from_id).unwrap_or_default()
    }

    pub fn from_state(lang: Lang, quality: Quality, auto_download: bool) -> Self {
        Self { lang: Some(lang.id().into()), quality: Some(quality.id().into()), auto_download: Some(auto_download) }
    }
}

/// Yandex Music OAuth token: `YAMUSIQ_TOKEN`, else the `token` file next to
/// the config (`~/.config/yamusiq/token`). Never logged.
pub fn token() -> Option<String> {
    let from_env = std::env::var("YAMUSIQ_TOKEN").ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
    from_env.or_else(|| std::fs::read_to_string(token_path()?).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty()))
}

pub fn token_path() -> Option<PathBuf> {
    Some(Config::path()?.with_file_name("token"))
}

/// Stores the token readable only by the user (mode 600).
pub fn save_token(token: &str) -> Result<PathBuf> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = token_path().context("no HOME to store the token in")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut f = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&path)?;
    f.write_all(token.trim().as_bytes())?;
    f.write_all(b"\n")?;
    // An existing file keeps its old mode on open; enforce 600 explicitly.
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o600))?;
    Ok(path)
}

/// Forgets the saved token. Returns whether there was one.
pub fn delete_token() -> Result<bool> {
    let Some(path) = token_path() else { return Ok(false) };
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_old_keys() {
        let c = Config::from_state(Lang::En, Quality::Normal, true);
        assert_eq!(c.auto_download, Some(true));
        assert_eq!(c.quality(), Quality::Normal);
        let json = serde_json::to_string(&c).unwrap();
        assert_eq!(serde_json::from_str::<Config>(&json).unwrap(), c);
        let old: Config = serde_json::from_str(r#"{"theme":"mars","lang":"ru"}"#).unwrap();
        assert_eq!(old.lang.as_deref(), Some("ru"));
        assert_eq!(serde_json::from_str::<Config>("{}").unwrap(), Config::default());
    }
}
