//! Persistent application configuration for Phaenna (`config.toml`).

use crate::error::{AppError, Result};
use directories::ProjectDirs;
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
            Quality::Best => "bestvideo[height<=?4320][vcodec^=av01]+bestaudio/bestvideo[height<=?4320][vcodec^=vp9]+bestaudio/bestvideo[height<=?4320]+bestaudio/bestvideo+bestaudio/best",
            Quality::P2160 => "bestvideo[height<=?2160][fps>=?50]+bestaudio/bestvideo[height<=?2160]+bestaudio/best",
            Quality::P1440 => "bestvideo[height<=?1440][fps>=?50]+bestaudio/bestvideo[height<=?1440]+bestaudio/best",
            Quality::P1080 => "bestvideo[height<=?1080][fps>=?50]+bestaudio/bestvideo[height<=?1080]+bestaudio/best",
            Quality::P720 => "bestvideo[height<=?720][fps>=?50]+bestaudio/bestvideo[height<=?720]+bestaudio/best",
            Quality::P480 => "bestvideo[height<=?480]+bestaudio/best",
            Quality::P360 => "bestvideo[height<=?360]+bestaudio/best",
            Quality::AudioOnly => "bestaudio/best",
        }
    }

    pub fn video_selectors(self) -> &'static [&'static str] {
        match self {
            Quality::Best => &[
                "bestvideo[height<=?4320][vcodec^=av01]",
                "bestvideo[height<=?4320][vcodec^=vp9]",
                "bestvideo[height<=?4320]",
                "bestvideo",
            ],
            Quality::P2160 => &["bestvideo[height<=?2160][fps>=?50]", "bestvideo[height<=?2160]"],
            Quality::P1440 => &["bestvideo[height<=?1440][fps>=?50]", "bestvideo[height<=?1440]"],
            Quality::P1080 => &["bestvideo[height<=?1080][fps>=?50]", "bestvideo[height<=?1080]"],
            Quality::P720 => &["bestvideo[height<=?720][fps>=?50]", "bestvideo[height<=?720]"],
            Quality::P480 => &["bestvideo[height<=?480]"],
            Quality::P360 => &["bestvideo[height<=?360]"],
            Quality::AudioOnly => &[],
        }
    }

    /// Build a `yt-dlp` format selector honoring Original vs. Dubbed audio and multi-stream loading.
    pub fn ytdl_format_with_dub(
        self,
        dub: DubPreference,
        dub_lang: &str,
        multistreams: bool,
    ) -> String {
        let lang: String = dub_lang
            .trim()
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
            .take(8)
            .collect();
        let lang = if lang.is_empty() { "en" } else { lang.as_str() };

        let primary_audio = match dub {
            DubPreference::OriginalOnly => "bestaudio[format_note!*=dubbed]".to_string(),
            DubPreference::PreferDubbed => {
                format!("bestaudio[format_note*=dubbed][language^={lang}]")
            }
        };
        let alt_audio = match dub {
            DubPreference::OriginalOnly => {
                format!("bestaudio[format_note*=dubbed][language^={lang}]")
            }
            DubPreference::PreferDubbed => "bestaudio[format_note!*=dubbed]".to_string(),
        };

        if self == Quality::AudioOnly {
            return match dub {
                DubPreference::OriginalOnly => {
                    "bestaudio[format_note!*=dubbed]/bestaudio/best".to_string()
                }
                DubPreference::PreferDubbed => format!(
                    "bestaudio[format_note*=dubbed][language^={lang}]/bestaudio[format_note*=dubbed]/bestaudio[language^={lang}]/bestaudio/best"
                ),
            };
        }

        let mut branches: Vec<String> = Vec::new();
        let vs = self.video_selectors();
        if multistreams {
            if let Some(first_v) = vs.first() {
                branches.push(format!("{first_v}+{primary_audio}+{alt_audio}"));
            }
            let primary_v = vs.last().copied().unwrap_or("bestvideo");
            if Some(&primary_v) != vs.first() {
                branches.push(format!("{primary_v}+{primary_audio}+{alt_audio}"));
            }
        }
        for v in vs {
            branches.push(format!("{v}+{primary_audio}"));
        }
        if dub == DubPreference::PreferDubbed {
            if let Some(first_v) = vs.first() {
                branches.push(format!("{first_v}+bestaudio[format_note*=dubbed]"));
                branches.push(format!("{first_v}+bestaudio[language^={lang}]"));
            }
        }
        for v in vs {
            branches.push(format!("{v}+bestaudio"));
        }
        branches.push("best".to_string());
        branches.join("/")
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Quality::Best => "4K-AV1",
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

/// Real-time GPU mastering, compression cleanup & 4K super-resolution GLSL shader profile for `mpv` / `iina`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum VideoShaderMode {
    Off,
    Balanced,
    #[default]
    #[serde(alias = "ultra", alias = "reference4k", alias = "reference_4k")]
    Reference4K,
    #[serde(alias = "neural", alias = "neural_cinema", alias = "fsrcnnx")]
    NeuralCinema,
    #[serde(alias = "anime", alias = "anime_4k")]
    Anime4K,
}

#[allow(dead_code)]
impl VideoShaderMode {
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Balanced,
            Self::Balanced => Self::Reference4K,
            Self::Reference4K => Self::NeuralCinema,
            Self::NeuralCinema => Self::Anime4K,
            Self::Anime4K => Self::Off,
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Balanced => "balanced",
            Self::Reference4K => "reference4k",
            Self::NeuralCinema => "neuralcinema",
            Self::Anime4K => "anime4k",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off (Raw Stream)",
            Self::Balanced => "Balanced (Deblock + Luma CAS + Deband)",
            Self::Reference4K => "Reference 4K Studio (AV1/VP9 Deblock + 4:4:4 Chroma + 2x RCAS 4K)",
            Self::NeuralCinema => "Neural Cinema 4K (FSRCNNX 16-0-4-1 CNN + Deblock + 4:4:4)",
            Self::Anime4K => "Anime4K Restore (Line Reconstruction + Deband + 2x Upscale)",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Off => "Off (Raw Stream)",
            Self::Balanced => "Balanced (Deblock + CAS)",
            Self::Reference4K => "Reference 4K Studio",
            Self::NeuralCinema => "Neural Cinema 4K (CNN)",
            Self::Anime4K => "Anime4K Restore (2x)",
        }
    }

    pub const fn detail_summary(self) -> &'static str {
        match self {
            Self::Off => "Native decoded stream without post-processing",
            Self::Balanced => "Bilateral deblock + Luma CAS + 16-bit deband",
            Self::Reference4K => "AV1/VP9 deblock + 4:4:4 chroma + 2x RCAS 4K",
            Self::NeuralCinema => "FSRCNNX 16-0-4-1 neural CNN + 4:4:4 chroma",
            Self::Anime4K => "Line-art contour reconstruction + 2x upscale",
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
        self.auto_skips_category_with_outro(category, false)
    }

    pub fn auto_skips_category_with_outro(self, category: &str, skip_outro: bool) -> bool {
        if category == "outro" && !skip_outro {
            return false;
        }
        match self {
            Self::AutoSkipAll => true,
            Self::SponsorOnly => category == "sponsor",
            Self::ShowChaptersOnly | Self::Off => false,
        }
    }
}

/// Controls whether YouTube multi-language AI/creator audio dubbing is used or suppressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DubPreference {
    #[default]
    OriginalOnly,
    PreferDubbed,
}

#[allow(dead_code)]
impl DubPreference {
    pub const fn toggle(self) -> Self {
        match self {
            Self::OriginalOnly => Self::PreferDubbed,
            Self::PreferDubbed => Self::OriginalOnly,
        }
    }

    pub const fn is_dub_on(self) -> bool {
        matches!(self, Self::PreferDubbed)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::OriginalOnly => "OFF (Original Audio)",
            Self::PreferDubbed => "ON (Dubbed Track)",
        }
    }

    pub const fn badge(self) -> &'static str {
        match self {
            Self::OriginalOnly => "ORIG",
            Self::PreferDubbed => "DUB",
        }
    }
}

pub const DUB_LANGUAGES: &[(&str, &str)] = &[
    ("en", "English (en)"),
    ("ar", "Arabic (ar)"),
    ("ja", "Japanese (ja)"),
    ("es", "Spanish (es)"),
    ("fr", "French (fr)"),
    ("de", "German (de)"),
    ("ko", "Korean (ko)"),
    ("pt", "Portuguese (pt)"),
    ("hi", "Hindi (hi)"),
];

pub fn dub_lang_label(code: &str) -> &'static str {
    DUB_LANGUAGES
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, l)| *l)
        .unwrap_or("English (en)")
}

pub fn next_dub_lang(current: &str) -> String {
    let idx = DUB_LANGUAGES
        .iter()
        .position(|(c, _)| *c == current)
        .map(|i| (i + 1) % DUB_LANGUAGES.len())
        .unwrap_or(0);
    DUB_LANGUAGES[idx].0.to_string()
}

/// Optional Google OAuth2 client settings for in-app Device Code & Kitty Split PKCE flows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoogleOAuthConfig {
    #[serde(default)]
    pub client_id: Option<String>,
    #[serde(default)]
    pub client_secret: Option<String>,
    #[serde(default = "default_true")]
    pub prefer_kitty_split: bool,
}

impl Default for GoogleOAuthConfig {
    fn default() -> Self {
        Self {
            client_id: None,
            client_secret: None,
            prefer_kitty_split: true,
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

fn default_dub_lang() -> String {
    "en".to_string()
}

fn default_theme_index() -> usize {
    2 // Phaenna
}

/// Resource / GPU mastering trade-off preset (`Efficient`, `Balanced`, `Quality`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceMode {
    Efficient,
    #[default]
    Balanced,
    Quality,
}

#[allow(dead_code)]
impl PerformanceMode {
    pub const ALL: [Self; 3] = [Self::Efficient, Self::Balanced, Self::Quality];

    pub const fn next(self) -> Self {
        match self {
            Self::Efficient => Self::Balanced,
            Self::Balanced => Self::Quality,
            Self::Quality => Self::Efficient,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Efficient => "Efficient",
            Self::Balanced => "Balanced",
            Self::Quality => "Quality",
        }
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Self::Efficient => "ECO",
            Self::Balanced => "BAL",
            Self::Quality => "MAX",
        }
    }

    pub const fn summary(self) -> &'static str {
        match self {
            Self::Efficient => "~350MB RAM | 48M buf | Light Luma CAS",
            Self::Balanced => "~650MB RAM | 160M buf | Deblock + 4:4:4",
            Self::Quality => "~1.4GB RAM | 512M buf | Full 4K 2x Master",
        }
    }

    pub const fn demuxer_limits(self) -> (&'static str, &'static str) {
        match self {
            Self::Efficient => ("48MiB", "16MiB"),
            Self::Balanced => ("160MiB", "48MiB"),
            Self::Quality => ("512MiB", "128MiB"),
        }
    }

    pub const fn demuxer_max_bytes_num(self) -> u64 {
        match self {
            Self::Efficient => 48 * 1024 * 1024,
            Self::Balanced => 160 * 1024 * 1024,
            Self::Quality => 512 * 1024 * 1024,
        }
    }

    pub const fn loads_multistreams(self, config_multi: bool) -> bool {
        match self {
            Self::Efficient => false,
            Self::Balanced | Self::Quality => config_multi,
        }
    }
}

/// 120Hz display refresh cadence & GPU motion interpolation profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MotionMode {
    Off,
    #[default]
    Cadence,
    Smooth120Hz,
}

#[allow(dead_code)]
impl MotionMode {
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Cadence,
            Self::Cadence => Self::Smooth120Hz,
            Self::Smooth120Hz => Self::Off,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off (Native FPS)",
            Self::Cadence => "Cadence (120Hz Match)",
            Self::Smooth120Hz => "Smooth 120Hz",
        }
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Self::Off => "NAT",
            Self::Cadence => "120C",
            Self::Smooth120Hz => "120S",
        }
    }
}

/// Persistent application configuration for Phaenna (`config.toml`).
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
    pub dub_preference: DubPreference,
    #[serde(default = "default_dub_lang")]
    pub dub_lang: String,
    #[serde(default = "default_true")]
    pub audio_multistreams: bool,
    #[serde(default)]
    pub audio_loudnorm: bool,
    #[serde(default)]
    pub sponsorblock_mode: SponsorBlockMode,
    #[serde(default)]
    pub skip_outro: bool,
    #[serde(default)]
    pub shader_mode: VideoShaderMode,
    #[serde(default)]
    pub performance_mode: PerformanceMode,
    #[serde(default)]
    pub motion_mode: MotionMode,
    #[serde(default = "default_true")]
    pub ambilight_enabled: bool,
    #[serde(default = "default_theme_index")]
    pub theme_index: usize,
    #[serde(default)]
    pub google_oauth: GoogleOAuthConfig,
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
            dub_preference: DubPreference::default(),
            dub_lang: default_dub_lang(),
            audio_multistreams: true,
            audio_loudnorm: false,
            sponsorblock_mode: SponsorBlockMode::default(),
            skip_outro: false,
            shader_mode: VideoShaderMode::default(),
            performance_mode: PerformanceMode::default(),
            motion_mode: MotionMode::default(),
            ambilight_enabled: true,
            theme_index: default_theme_index(),
            google_oauth: GoogleOAuthConfig::default(),
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

    /// Path to Phaenna's isolated OAuth & Google session vault (`<config_dir>/auth.json`).
    pub fn auth_vault_path() -> Result<PathBuf> {
        Ok(Self::config_dir()?.join("auth.json"))
    }

    /// Resolve isolated Netscape cookie jar path for optional Flow A sessions or `--cookies` CLI:
    /// 1. Explicit `custom_cookies_path` when configured
    /// 2. Phaenna's own `<config_dir>/session_cookies.txt` (zero browser SQLite scraping)
    pub fn cookies_path(&self) -> Result<PathBuf> {
        if let Some(ref custom) = self.custom_cookies_path {
            if !custom.as_os_str().is_empty() {
                return Ok(custom.clone());
            }
        }
        Ok(Self::config_dir()?.join("session_cookies.txt"))
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
            shader_mode: VideoShaderMode::Reference4K,
            theme_index: 4,
            ..Default::default()
        };
        cfg.save_to_path(&cfg_path).unwrap();

        let loaded = Config::load_from_path(&cfg_path).unwrap();
        assert_eq!(loaded.quality, Quality::P1440);
        assert_eq!(loaded.player, Player::Iina);
        assert_eq!(loaded.sponsorblock_mode, SponsorBlockMode::SponsorOnly);
        assert_eq!(loaded.shader_mode, VideoShaderMode::Reference4K);
        assert_eq!(loaded.theme_index, 4);

        // Legacy TOML alias compatibility (`"ultra"` -> `Reference4K`)
        let legacy: Config = toml::from_str("shader_mode = \"ultra\"\n").unwrap();
        assert_eq!(legacy.shader_mode, VideoShaderMode::Reference4K);
    }

    #[test]
    fn test_quality_format_selectors() {
        assert_eq!(Quality::all().len(), 8);
        assert!(Quality::Best.ytdl_format().contains("av01"));
        assert!(Quality::P1080.ytdl_format().contains("1080"));
        assert_eq!(Quality::AudioOnly.ytdl_format(), "bestaudio/best");
        assert_eq!(Quality::P2160.short_badge(), "4K60");
        let orig = Quality::Best.ytdl_format_with_dub(DubPreference::OriginalOnly, "en", true);
        assert!(orig.contains("format_note!*=dubbed"));
        let dubbed = Quality::Best.ytdl_format_with_dub(DubPreference::PreferDubbed, "ar", true);
        assert!(dubbed.contains("format_note*=dubbed"));
        assert!(dubbed.contains("language^=ar"));
    }

    #[test]
    fn test_shader_and_sponsorblock_cycling() {
        let s = VideoShaderMode::Off.next();
        assert_eq!(s, VideoShaderMode::Balanced);
        assert_eq!(s.next(), VideoShaderMode::Reference4K);
        assert_eq!(s.next().next(), VideoShaderMode::NeuralCinema);
        assert_eq!(s.next().next().next(), VideoShaderMode::Anime4K);
        assert_eq!(s.next().next().next().next(), VideoShaderMode::Off);

        let sb = SponsorBlockMode::AutoSkipAll;
        assert!(sb.auto_skips_category("sponsor"));
        assert!(sb.auto_skips_category("intro"));
        assert!(!sb.auto_skips_category("outro"));
        assert!(sb.auto_skips_category_with_outro("outro", true));
        let sb2 = sb.next();
        assert_eq!(sb2, SponsorBlockMode::SponsorOnly);
        assert!(sb2.auto_skips_category("sponsor"));
        assert!(!sb2.auto_skips_category("intro"));
        assert!(!SponsorBlockMode::Off.auto_skips_category("sponsor"));

        assert_eq!(PerformanceMode::Balanced.short_badge(), "BAL");
        assert_eq!(PerformanceMode::Efficient.demuxer_limits(), ("48MiB", "16MiB"));
        assert!(!PerformanceMode::Efficient.loads_multistreams(true));
        assert!(PerformanceMode::Quality.loads_multistreams(true));
        assert_eq!(MotionMode::Off.next(), MotionMode::Cadence);
        assert_eq!(MotionMode::Cadence.next(), MotionMode::Smooth120Hz);
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
