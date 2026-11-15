//! Persistent application configuration for Phaenna (`config.toml`).

use crate::error::{AppError, Result};
use directories::{BaseDirs, ProjectDirs};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Stream quality preset translated into `yt-dlp --ytdl-format` selectors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    #[default]
    Best,
    P2160,
    P1440,
    P1080,
    P720,
    P480,
    P360,
    AudioOnly,
}

#[allow(dead_code)]
impl Quality {
    pub const ALL: [Quality; 8] = Self::all();

    pub const fn all() -> [Quality; 8] {
        [
            Quality::Best,
            Quality::P2160,
            Quality::P1440,
            Quality::P1080,
            Quality::P720,
            Quality::P480,
            Quality::P360,
            Quality::AudioOnly,
        ]
    }

    pub const fn next(self) -> Self {
        match self {
            Quality::Best => Quality::P2160,
            Quality::P2160 => Quality::P1440,
            Quality::P1440 => Quality::P1080,
            Quality::P1080 => Quality::P720,
            Quality::P720 => Quality::P480,
            Quality::P480 => Quality::P360,
            Quality::P360 => Quality::AudioOnly,
            Quality::AudioOnly => Quality::Best,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Quality::Best => "Auto / Best (4K / 60fps)",
            Quality::P2160 => "4K (2160p 60fps UHD)",
            Quality::P1440 => "1440p (2K 60fps QHD)",
            Quality::P1080 => "1080p (FHD 60fps)",
            Quality::P720 => "720p (HD 60fps)",
            Quality::P480 => "480p (SD 30/60fps)",
            Quality::P360 => "360p (Data Saver)",
            Quality::AudioOnly => "Audio Only (Podcast)",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Quality::Best => "Highest available resolution & frame rate",
            Quality::P2160 => "Ultra HD 4K 60fps (maximum visual detail)",
            Quality::P1440 => "Quad HD 1440p 60fps (retina desktop standard)",
            Quality::P1080 => "Full HD 1080p 60fps (smooth high frame rate)",
            Quality::P720 => "Standard HD 720p 60fps (fast buffer streaming)",
            Quality::P480 => "Standard Definition (low bandwidth / mobile)",
            Quality::P360 => "Minimal data consumption",
            Quality::AudioOnly => "Listen to audio stream without video",
        }
    }

    pub const fn ytdl_format(self) -> &'static str {
        match self {
            Quality::Best => "bestvideo+bestaudio/best",
            Quality::P2160 => "bestvideo[height<=?2160][fps>=?50]+bestaudio/bestvideo[height<=?2160]+bestaudio/best",
            Quality::P1440 => "bestvideo[height<=?1440][fps>=?50]+bestaudio/bestvideo[height<=?1440]+bestaudio/best",
            Quality::P1080 => "bestvideo[height<=?1080][fps>=?50]+bestaudio/bestvideo[height<=?1080]+bestaudio/best",
            Quality::P720 => "bestvideo[height<=?720][fps>=?50]+bestaudio/bestvideo[height<=?720]+bestaudio/best",
            Quality::P480 => "bestvideo[height<=?480]+bestaudio/best",
            Quality::P360 => "bestvideo[height<=?360]+bestaudio/best",
            Quality::AudioOnly => "bestaudio/best",
        }
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Quality::Best => "BEST",
            Quality::P2160 => "4K60",
            Quality::P1440 => "2K60",
            Quality::P1080 => "1080p",
            Quality::P720 => "720p",
            Quality::P480 => "480p",
            Quality::P360 => "360p",
            Quality::AudioOnly => "AUDIO",
        }
    }
}

/// Supported playback engines across macOS, Linux, and Windows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Player {
    #[default]
    Mpv,
    Iina,
    Vlc,
    Browser,
}

#[allow(dead_code)]
impl Player {
    pub const fn all() -> [Player; 4] {
        [Player::Mpv, Player::Iina, Player::Vlc, Player::Browser]
    }

    pub const fn next(self) -> Self {
        match self {
            Player::Mpv => Player::Iina,
            Player::Iina => Player::Vlc,
            Player::Vlc => Player::Browser,
            Player::Browser => Player::Mpv,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Player::Mpv => "MPV (IPC + Hardware Accel)",
            Player::Iina => "IINA (macOS Native)",
            Player::Vlc => "VLC Media Player",
            Player::Browser => "System Web Browser",
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Player::Mpv => "mpv",
            Player::Iina => "iina",
            Player::Vlc => "vlc",
            Player::Browser => "browser",
        }
    }
}

/// Real-time GPU mastering & upscaling GLSL shader profile for `mpv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VideoShaderMode {
    Off,
    #[default]
    Balanced,
    Ultra,
}

#[allow(dead_code)]
impl VideoShaderMode {
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Balanced,
            Self::Balanced => Self::Ultra,
            Self::Ultra => Self::Off,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Balanced => "balanced",
            Self::Ultra => "ultra",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off (Native)",
            Self::Balanced => "Balanced (Lanczos + Deband + Crisp)",
            Self::Ultra => "Ultra (x2 CNN Upscale + Mastering)",
        }
    }
}

/// SponsorBlock auto-skip behavior when watching in `mpv`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SponsorBlockMode {
    #[default]
    AutoSkipAll,
    SponsorOnly,
    ShowChaptersOnly,
    Off,
}

#[allow(dead_code)]
impl SponsorBlockMode {
    pub const fn next(self) -> Self {
        match self {
            Self::AutoSkipAll => Self::SponsorOnly,
            Self::SponsorOnly => Self::ShowChaptersOnly,
            Self::ShowChaptersOnly => Self::Off,
            Self::Off => Self::AutoSkipAll,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::AutoSkipAll => "Auto-Skip All",
            Self::SponsorOnly => "Sponsors Only",
            Self::ShowChaptersOnly => "Show Badges Only",
            Self::Off => "Disabled",
        }
    }

    pub fn categories(self) -> &'static [&'static str] {
        match self {
            Self::AutoSkipAll | Self::ShowChaptersOnly => &[
                "sponsor",
                "selfpromo",
                "interaction",
                "intro",
                "outro",
                "preview",
                "music_offtopic",
            ],
            Self::SponsorOnly => &["sponsor"],
            Self::Off => &[],
        }
    }

    pub fn auto_skips_category(self, category: &str) -> bool {
        match self {
            Self::AutoSkipAll => true,
            Self::SponsorOnly => category == "sponsor",
            Self::ShowChaptersOnly | Self::Off => false,
        }
    }
}

/// Browser profile source for Netscape cookie extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CookieSource {
    #[default]
    Auto,
    Zen,
    Firefox,
    CustomFile,
}

impl CookieSource {
    pub const fn next(self) -> Self {
        match self {
            Self::Auto => Self::Zen,
            Self::Zen => Self::Firefox,
            Self::Firefox => Self::CustomFile,
            Self::CustomFile => Self::Auto,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Auto => "Auto (Zen -> Firefox -> File)",
            Self::Zen => "Zen Browser",
            Self::Firefox => "Mozilla Firefox",
            Self::CustomFile => "Custom Netscape File",
        }
    }
}

/// Cache TTLs (in seconds) controlling SWR background refresh cadence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheConfig {
    pub recommended_ttl: u64,
    pub subscriptions_ttl: u64,
    pub history_ttl: u64,
    pub watch_later_ttl: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            recommended_ttl: 60 * 30,      // 30 minutes
            subscriptions_ttl: 60 * 15,    // 15 minutes
            history_ttl: 60 * 60,          // 1 hour
            watch_later_ttl: 60 * 60 * 6,  // 6 hours
        }
    }
}

/// Local LAN mobile remote WebSocket/HTTP server settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_remote_port")]
    pub port: u16,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            port: default_remote_port(),
        }
    }
}

/// AirPlay & DLNA TV cast discovery configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CastConfig {
    #[serde(default = "default_true")]
    pub auto_discover: bool,
    #[serde(default = "default_cast_timeout")]
    pub scan_timeout_ms: u64,
}

impl Default for CastConfig {
    fn default() -> Self {
        Self {
            auto_discover: true,
            scan_timeout_ms: default_cast_timeout(),
        }
    }
}

/// GitHub Release self-update preferences.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateConfig {
    #[serde(default = "default_true")]
    pub check_on_startup: bool,
    #[serde(default)]
    pub auto_install: bool,
    #[serde(default)]
    pub check_interval: u64,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_on_startup: true,
            auto_install: false,
            check_interval: 0,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_remote_port() -> u16 {
    8485
}

fn default_cast_timeout() -> u64 {
    500
}

fn default_sub_lang() -> String {
    "English".to_string()
}

fn default_theme_index() -> usize {
    2 // Phaenna
}

/// Root user configuration persisted to `<config_dir>/config.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub quality: Quality,
    #[serde(default)]
    pub player: Player,
    #[serde(default = "default_true")]
    pub subtitles_enabled: bool,
    #[serde(default = "default_sub_lang")]
    pub default_sub_lang: String,
    #[serde(default)]
    pub sponsorblock_mode: SponsorBlockMode,
    #[serde(default)]
    pub shader_mode: VideoShaderMode,
    #[serde(default = "default_true")]
    pub ambilight_enabled: bool,
    #[serde(default = "default_theme_index")]
    pub theme_index: usize,
    #[serde(default)]
    pub cookie_source: CookieSource,
    #[serde(default)]
    pub custom_cookies_path: Option<PathBuf>,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub remote: RemoteConfig,
    #[serde(default)]
    pub cast: CastConfig,
    #[serde(default)]
    pub update: UpdateConfig,
    #[serde(default)]
    pub subtitle_style: crate::services::substyler::SubtitleStyleConfig,
    #[serde(default)]
    pub tailored_answers: crate::services::tailored::TailoredAnswers,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            quality: Quality::default(),
            player: Player::default(),
            subtitles_enabled: true,
            default_sub_lang: default_sub_lang(),
            sponsorblock_mode: SponsorBlockMode::default(),
            shader_mode: VideoShaderMode::default(),
            ambilight_enabled: true,
            theme_index: default_theme_index(),
            cookie_source: CookieSource::default(),
            custom_cookies_path: None,
            cache: CacheConfig::default(),
            remote: RemoteConfig::default(),
            cast: CastConfig::default(),
            update: UpdateConfig::default(),
            subtitle_style: crate::services::substyler::SubtitleStyleConfig::default(),
            tailored_answers: crate::services::tailored::TailoredAnswers::default(),
        }
    }
}

#[allow(dead_code)]
impl Config {
    pub fn config_dir() -> Result<PathBuf> {
        let proj = ProjectDirs::from("", "", "phaenna")
            .ok_or_else(|| AppError::Config("Could not determine config directory".into()))?;
        Ok(proj.config_dir().to_path_buf())
    }

    pub fn cache_dir() -> Result<PathBuf> {
        ProjectDirs::from("", "", "phaenna")
            .map(|p| p.cache_dir().to_path_buf())
            .ok_or_else(|| AppError::Config("Could not determine cache directory".into()))
    }

    pub fn config_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("config.toml"))
    }

    pub fn db_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("phaenna.db"))
    }

    /// Resolve cookie file path portably:
    /// 1. Explicit `custom_cookies_path` when configured
    /// 2. Existing non-empty `~/.config/yt-dlp/cookies.txt` (preserves prior setups)
    /// 3. `<config_dir>/cookies.txt`
    pub fn cookies_path(&self) -> Result<PathBuf> {
        if let Some(ref custom) = self.custom_cookies_path {
            if !custom.as_os_str().is_empty() {
                return Ok(custom.clone());
            }
        }
        if let Some(base) = BaseDirs::new() {
            let legacy_ytdlp = base.home_dir().join(".config").join("yt-dlp").join("cookies.txt");
            if is_non_empty_file(&legacy_ytdlp) {
                return Ok(legacy_ytdlp);
            }
        }
        Ok(Self::config_dir()?.join("cookies.txt"))
    }

    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;
        Self::load_from_path(&path)
    }

    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            let cfg = Self::default();
            let _ = cfg.save_to_path(path);
            return Ok(cfg);
        }
        let raw = std::fs::read_to_string(path)?;
        toml::from_str::<Self>(&raw).map_err(|e| AppError::Parse(format!("Invalid config.toml: {e}")))
    }

    pub fn save(&self) -> Result<()> {
        let path = Self::config_path()?;
        self.save_to_path(&path)
    }

    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)
            .map_err(|e| AppError::Config(format!("Failed to serialize config: {e}")))?;
        std::fs::write(path, toml_str)?;
        Ok(())
    }
}

fn is_non_empty_file(path: &Path) -> bool {
    std::fs::metadata(path).map(|m| m.is_file() && m.len() > 0).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_config_roundtrip_toml() {
        let dir = tempdir().unwrap();
        let cfg_path = dir.path().join("config.toml");

        let cfg = Config {
            quality: Quality::P1440,
            player: Player::Iina,
            sponsorblock_mode: SponsorBlockMode::SponsorOnly,
            shader_mode: VideoShaderMode::Ultra,
            theme_index: 4,
            ..Default::default()
        };
        cfg.save_to_path(&cfg_path).unwrap();

        let loaded = Config::load_from_path(&cfg_path).unwrap();
        assert_eq!(loaded.quality, Quality::P1440);
        assert_eq!(loaded.player, Player::Iina);
        assert_eq!(loaded.sponsorblock_mode, SponsorBlockMode::SponsorOnly);
        assert_eq!(loaded.shader_mode, VideoShaderMode::Ultra);
        assert_eq!(loaded.theme_index, 4);
    }

    #[test]
    fn test_quality_format_selectors() {
        assert_eq!(Quality::all().len(), 8);
        assert!(Quality::P1080.ytdl_format().contains("1080"));
        assert_eq!(Quality::AudioOnly.ytdl_format(), "bestaudio/best");
        assert_eq!(Quality::P2160.short_badge(), "4K60");
    }

    #[test]
    fn test_shader_and_sponsorblock_cycling() {
        let s = VideoShaderMode::Off.next();
        assert_eq!(s, VideoShaderMode::Balanced);
        assert_eq!(s.next(), VideoShaderMode::Ultra);
        assert_eq!(s.next().next(), VideoShaderMode::Off);

        let sb = SponsorBlockMode::AutoSkipAll;
        assert!(sb.auto_skips_category("sponsor"));
        assert!(sb.auto_skips_category("intro"));
        let sb2 = sb.next();
        assert_eq!(sb2, SponsorBlockMode::SponsorOnly);
        assert!(sb2.auto_skips_category("sponsor"));
        assert!(!sb2.auto_skips_category("intro"));
        assert!(!SponsorBlockMode::Off.auto_skips_category("sponsor"));
    }

    #[test]
    fn test_custom_cookies_path_precedence() {
        let cfg = Config {
            custom_cookies_path: Some(PathBuf::from("/tmp/custom-yt-cookies.txt")),
            ..Default::default()
        };
        assert_eq!(cfg.cookies_path().unwrap(), PathBuf::from("/tmp/custom-yt-cookies.txt"));
    }
}
