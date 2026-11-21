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
        self.video_selectors_for_codec(CodecPriority::Av1First)
    }

    pub fn video_selectors_for_codec(self, codec: CodecPriority) -> &'static [&'static str] {
        match self {
            Quality::Best => match codec {
                CodecPriority::Vp9HwFirst => &[
                    "bestvideo[height<=?4320][vcodec^=vp9]",
                    "bestvideo[height<=?4320][vcodec^=hev]",
                    "bestvideo[height<=?4320][vcodec^=avc]",
                    "bestvideo[height<=?4320]",
                    "bestvideo",
                ],
                CodecPriority::Av1First => &[
                    "bestvideo[height<=?4320][vcodec^=av01]",
                    "bestvideo[height<=?4320][vcodec^=vp9]",
                    "bestvideo[height<=?4320]",
                    "bestvideo",
                ],
            },
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
        self.ytdl_format_with_dub_and_codec(dub, dub_lang, multistreams, CodecPriority::Av1First)
    }

    /// Build a `yt-dlp` format selector honoring audio dubs, multi-stream loading, and codec priority.
    pub fn ytdl_format_with_dub_and_codec(
        self,
        dub: DubPreference,
        dub_lang: &str,
        multistreams: bool,
        codec: CodecPriority,
    ) -> String {
        let lang = sanitize_dub_lang(dub_lang);

        let primary_audio = match dub {
            DubPreference::OriginalOnly => {
                "(bestaudio[format_note*=original][format_note!*=dubbed][format_note!*=DRC]/bestaudio[format_note*=original][format_note!*=dubbed]/bestaudio[language_preference>=0][format_note!*=dubbed][format_note!*=DRC]/bestaudio[language_preference>=0][format_note!*=dubbed]/bestaudio[format_note!*=dubbed][format_note!*=DRC]/bestaudio[format_note!*=dubbed]/bestaudio)"
                    .to_string()
            }
            DubPreference::PreferDubbed => format!(
                "(bestaudio[format_note*=dubbed][language^={lang}]/bestaudio[language^={lang}][format_note!*=original]/bestaudio[format_note*=dubbed]/bestaudio[language^={lang}])"
            ),
        };
        let alt_audio = match dub {
            DubPreference::OriginalOnly => format!(
                "(bestaudio[language^={lang}][format_note!*=original]/bestaudio[format_note*=dubbed][language^={lang}]/bestaudio[format_note*=dubbed])"
            ),
            DubPreference::PreferDubbed => {
                "(bestaudio[format_note*=original][format_note!*=dubbed]/bestaudio[language_preference>=0][format_note!*=dubbed]/bestaudio[format_note!*=dubbed])"
                    .to_string()
            }
        };

        if self == Quality::AudioOnly {
            return match dub {
                DubPreference::OriginalOnly => format!("{primary_audio}/best"),
                DubPreference::PreferDubbed => format!("{primary_audio}/bestaudio/best"),
            };
        }

        let vs = self.video_selectors_for_codec(codec);
        let video_chain = format!("({})", vs.join("/"));
        let mut branches: Vec<String> = Vec::new();
        if multistreams {
            branches.push(format!("{video_chain}+{primary_audio}+{alt_audio}"));
        }
        branches.push(format!("{video_chain}+{primary_audio}"));
        branches.push(format!("{video_chain}+bestaudio"));
        branches.push("best".to_string());
        branches.join("/")
    }

    /// Build a bandwidth-capped `yt-dlp` format selector for normal vs. congested/shared Wi-Fi.
    pub fn ytdl_format_for_network(
        self,
        dub: DubPreference,
        dub_lang: &str,
        multistreams: bool,
        codec: CodecPriority,
        net: NetworkMode,
    ) -> String {
        if !net.is_congested() {
            return self.ytdl_format_with_dub_and_codec(dub, dub_lang, multistreams, codec);
        }

        let lang = sanitize_dub_lang(dub_lang);
        let abr = net.max_audio_abr_kbps().unwrap_or(128);
        let primary_audio = match dub {
            DubPreference::OriginalOnly => format!(
                "(bestaudio[abr<=?{abr}][format_note*=original][format_note!*=dubbed][format_note!*=DRC]/bestaudio[abr<=?{abr}][language_preference>=0][format_note!*=dubbed]/bestaudio[abr<=?{abr}][format_note!*=dubbed]/bestaudio[format_note!*=dubbed]/bestaudio)"
            ),
            DubPreference::PreferDubbed => format!(
                "(bestaudio[abr<=?{abr}][format_note*=dubbed][language^={lang}]/bestaudio[abr<=?{abr}][language^={lang}][format_note!*=original]/bestaudio[format_note*=dubbed]/bestaudio)"
            ),
        };

        if self == Quality::AudioOnly {
            return format!("{primary_audio}/best");
        }

        let h = self.max_height_or_default(net.max_height_cap().unwrap_or(720));
        let fps = net.max_fps_cap().unwrap_or(30);
        let video_chain = format!(
            "(bestvideo[height<=?{h}][fps<=?{fps}][vcodec^=av01]/bestvideo[height<=?{h}][fps<=?{fps}][vcodec^=vp9]/bestvideo[height<=?{h}][fps<=?{fps}]/bestvideo[height<=?{h}])"
        );
        format!("{video_chain}+{primary_audio}/best[height<=?{h}]/best")
    }

    pub fn max_height_or_default(self, default_max: u16) -> u16 {
        match self {
            Quality::Best | Quality::P2160 => default_max.min(2160),
            Quality::P1440 => default_max.min(1440),
            Quality::P1080 => default_max.min(1080),
            Quality::P720 => 720,
            Quality::P480 => 480,
            Quality::P360 => 360,
            Quality::AudioOnly => 0,
        }
    }

    /// Build a fast-start `yt-dlp` format selector optimized for live broadcasts.
    pub fn ytdl_format_for_live(
        self,
        mode: LiveStreamMode,
        dub: DubPreference,
        dub_lang: &str,
    ) -> String {
        if self == Quality::AudioOnly {
            return "bestaudio[protocol^=m3u8]/bestaudio/best".to_string();
        }
        if mode == LiveStreamMode::Off {
            return self.ytdl_format_with_dub(dub, dub_lang, false);
        }
        let max_cap = match mode {
            LiveStreamMode::LowLatency => 1080,
            LiveStreamMode::BalancedLive => 1440,
            LiveStreamMode::SmoothDvr => 2160,
            LiveStreamMode::Off => 4320,
        };
        let h = self.max_height_or_default(max_cap);
        match mode {
            LiveStreamMode::LowLatency | LiveStreamMode::BalancedLive => format!(
                "best[height<=?{h}][vcodec!^=av01][protocol^=m3u8]/bestvideo[height<=?{h}][vcodec!^=av01][protocol^=m3u8]+bestaudio[protocol^=m3u8]/bestvideo[height<=?{h}][vcodec!^=av01]+bestaudio/best[height<=?{h}]/best"
            ),
            LiveStreamMode::SmoothDvr => format!(
                "bestvideo[height<=?{h}]+bestaudio/best[height<=?{h}]/best"
            ),
            LiveStreamMode::Off => unreachable!(),
        }
    }

    /// Build a live broadcast `yt-dlp` selector honoring `NetworkMode` bitrate/resolution caps.
    pub fn ytdl_format_for_live_with_network(
        self,
        mode: LiveStreamMode,
        dub: DubPreference,
        dub_lang: &str,
        net: NetworkMode,
    ) -> String {
        if !net.is_congested() || self == Quality::AudioOnly {
            return self.ytdl_format_for_live(mode, dub, dub_lang);
        }
        if mode == LiveStreamMode::Off {
            return self.ytdl_format_for_network(
                dub,
                dub_lang,
                false,
                CodecPriority::Vp9HwFirst,
                net,
            );
        }
        let h = self.max_height_or_default(net.max_height_cap().unwrap_or(720));
        format!(
            "best[height<=?{h}][vcodec!^=av01][protocol^=m3u8]/bestvideo[height<=?{h}][vcodec!^=av01][protocol^=m3u8]+bestaudio[protocol^=m3u8]/bestvideo[height<=?{h}]+bestaudio/best[height<=?{h}]/best"
        )
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

fn sanitize_dub_lang(dub_lang: &str) -> String {
    let lang: String = dub_lang
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
        .take(8)
        .collect();
    if lang.is_empty() {
        "en".to_string()
    } else {
        lang
    }
}

/// Network bandwidth & shared-Wi-Fi congestion resilience profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    #[default]
    Normal,
    CongestedWifi,
    UltraSaver,
}

#[allow(dead_code)]
impl NetworkMode {
    pub const ALL: [Self; 3] = [Self::Normal, Self::CongestedWifi, Self::UltraSaver];

    pub const fn next(self) -> Self {
        match self {
            Self::Normal => Self::CongestedWifi,
            Self::CongestedWifi => Self::UltraSaver,
            Self::UltraSaver => Self::Normal,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal (Full Speed)",
            Self::CongestedWifi => "Congested Wi-Fi (720p30 + Cushion)",
            Self::UltraSaver => "Ultra Saver (480p30 + Disk Cache)",
        }
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Self::Normal => "NET:FULL",
            Self::CongestedWifi => "NET:WIFI",
            Self::UltraSaver => "NET:ECO",
        }
    }

    pub const fn is_congested(self) -> bool {
        !matches!(self, Self::Normal)
    }

    pub const fn max_height_cap(self) -> Option<u16> {
        match self {
            Self::Normal => None,
            Self::CongestedWifi => Some(720),
            Self::UltraSaver => Some(480),
        }
    }

    pub const fn max_fps_cap(self) -> Option<u16> {
        match self {
            Self::Normal => None,
            Self::CongestedWifi | Self::UltraSaver => Some(30),
        }
    }

    pub const fn max_audio_abr_kbps(self) -> Option<u16> {
        match self {
            Self::Normal => None,
            Self::CongestedWifi => Some(128),
            Self::UltraSaver => Some(72),
        }
    }

    pub const fn vod_cache_pause_wait_secs(self) -> f64 {
        match self {
            Self::Normal => 2.5,
            Self::CongestedWifi => 4.5,
            Self::UltraSaver => 6.0,
        }
    }

    pub const fn vod_readahead_secs(self) -> u32 {
        match self {
            Self::Normal => 45,
            Self::CongestedWifi => 120,
            Self::UltraSaver => 180,
        }
    }

    pub const fn network_timeout_secs(self) -> u32 {
        match self {
            Self::Normal => 15,
            Self::CongestedWifi => 12,
            Self::UltraSaver => 10,
        }
    }

    pub const fn use_disk_cache(self) -> bool {
        !matches!(self, Self::Normal)
    }

    pub const fn vlc_network_caching_ms(self) -> u32 {
        match self {
            Self::Normal => 3000,
            Self::CongestedWifi => 8000,
            Self::UltraSaver => 12000,
        }
    }

    pub const fn live_hls_bitrate(self) -> &'static str {
        match self {
            Self::Normal => "max",
            Self::CongestedWifi => "1800000",
            Self::UltraSaver => "900000",
        }
    }

    pub const fn prefer_compact_thumbs(self) -> bool {
        !matches!(self, Self::Normal)
    }

    pub const fn effective_multistreams(self, config_multi: bool, perf: PerformanceMode) -> bool {
        if self.is_congested() {
            false
        } else {
            perf.loads_multistreams(config_multi)
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

/// Resource / GPU mastering trade-off preset (`Efficient`, `Balanced`, `Quality`, `Manual`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PerformanceMode {
    Efficient,
    #[default]
    Balanced,
    Quality,
    Manual,
}

#[allow(dead_code)]
impl PerformanceMode {
    pub const ALL: [Self; 4] = [Self::Efficient, Self::Balanced, Self::Quality, Self::Manual];

    pub const fn next(self) -> Self {
        match self {
            Self::Efficient => Self::Balanced,
            Self::Balanced => Self::Quality,
            Self::Quality => Self::Manual,
            Self::Manual => Self::Efficient,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Efficient => "Efficient",
            Self::Balanced => "Balanced",
            Self::Quality => "Quality",
            Self::Manual => "Manual",
        }
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Self::Efficient => "ECO",
            Self::Balanced => "BAL",
            Self::Quality => "MAX",
            Self::Manual => "MAN",
        }
    }

    pub const fn summary(self) -> &'static str {
        match self {
            Self::Efficient => "~350MB RAM | 48M buf | Light Luma CAS",
            Self::Balanced => "~650MB RAM | Cool HW VP9 | 1x 4:4:4 + Deblock",
            Self::Quality => "~1.4GB RAM | 512M buf | Full 4K 2x Master",
            Self::Manual => "Custom Manual GPU / Thermal / Codec Profile",
        }
    }

    pub const fn demuxer_limits(self) -> (&'static str, &'static str) {
        ManualMasteringConfig::from_preset(self).demuxer_cache.limits()
    }

    pub const fn demuxer_max_bytes_num(self) -> u64 {
        ManualMasteringConfig::from_preset(self).demuxer_cache.max_bytes_num()
    }

    pub const fn loads_multistreams(self, config_multi: bool) -> bool {
        match self {
            Self::Efficient => false,
            Self::Balanced | Self::Quality | Self::Manual => config_multi,
        }
    }
}

/// 2x Super-Resolution framebuffer scaling policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SuperResMode {
    #[default]
    Off,
    Auto1080p,
    Always2x,
}

impl SuperResMode {
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Auto1080p,
            Self::Auto1080p => Self::Always2x,
            Self::Always2x => Self::Off,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF (1x Native)",
            Self::Auto1080p => "<=1080p Only (2x)",
            Self::Always2x => "ON (Always 2x 5K)",
        }
    }
}

/// Hardware-friendly VP9/HEVC priority vs. maximum-compression AV1 priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CodecPriority {
    #[default]
    Vp9HwFirst,
    Av1First,
}

impl CodecPriority {
    pub const fn next(self) -> Self {
        match self {
            Self::Vp9HwFirst => Self::Av1First,
            Self::Av1First => Self::Vp9HwFirst,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Vp9HwFirst => "VP9/HEVC HW (Cool)",
            Self::Av1First => "AV1 First (Max Comp)",
        }
    }

    pub const fn format_sort_vcodec(self) -> &'static str {
        match self {
            Self::Vp9HwFirst => "vp9.2:vp9:h264:av01",
            Self::Av1First => "av01:vp9.2:vp9",
        }
    }
}

/// `mpv` hardware video decoder selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum HwdecMode {
    #[default]
    VideoToolbox,
    AutoSafe,
    Off,
}

impl HwdecMode {
    pub const fn next(self) -> Self {
        match self {
            Self::VideoToolbox => Self::AutoSafe,
            Self::AutoSafe => Self::Off,
            Self::Off => Self::VideoToolbox,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::VideoToolbox => "VideoToolbox HW",
            Self::AutoSafe => "Auto-Safe HW",
            Self::Off => "OFF (Software CPU)",
        }
    }

    pub const fn mpv_value(self) -> &'static str {
        match self {
            Self::VideoToolbox => {
                if cfg!(target_os = "macos") {
                    "videotoolbox"
                } else {
                    "auto-safe"
                }
            }
            Self::AutoSafe => "auto-safe",
            Self::Off => "no",
        }
    }
}

/// HDR tone-mapping curve in `vo=gpu-next`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ToneMapMode {
    #[default]
    Spline,
    Bt2446a,
    St2094_40,
}

impl ToneMapMode {
    pub const fn next(self) -> Self {
        match self {
            Self::Spline => Self::Bt2446a,
            Self::Bt2446a => Self::St2094_40,
            Self::St2094_40 => Self::Spline,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Spline => "Spline (Balanced)",
            Self::Bt2446a => "BT.2446a (Broadcast)",
            Self::St2094_40 => "ST2094-40 (Dynamic)",
        }
    }

    pub const fn mpv_value(self) -> &'static str {
        match self {
            Self::Spline => "spline",
            Self::Bt2446a => "bt.2446a",
            Self::St2094_40 => "st2094-40",
        }
    }
}

/// Active GLSL post-processing stage chain depth.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GlslStageMode {
    Off,
    LightCas,
    #[default]
    Guided444,
    FullMastering,
}

impl GlslStageMode {
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::LightCas,
            Self::LightCas => Self::Guided444,
            Self::Guided444 => Self::FullMastering,
            Self::FullMastering => Self::Off,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF (Raw Stream)",
            Self::LightCas => "Light Luma CAS",
            Self::Guided444 => "Fast Guided 4:4:4",
            Self::FullMastering => "Full 4K Pipeline",
        }
    }
}

/// Gradient debanding pass count and temporal dithering budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DebandTier {
    Off,
    #[default]
    OnePassCool,
    ThreePassStudio,
}

impl DebandTier {
    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::OnePassCool,
            Self::OnePassCool => Self::ThreePassStudio,
            Self::ThreePassStudio => Self::Off,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "OFF (Zero GPU)",
            Self::OnePassCool => "1-Pass (Cool)",
            Self::ThreePassStudio => "3-Pass (Studio)",
        }
    }
}

/// GPU luma/chroma scaler kernel preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GpuScalerMode {
    #[default]
    Spline36,
    EwaLanczos,
    EwaLanczosSharp,
}

impl GpuScalerMode {
    pub const fn next(self) -> Self {
        match self {
            Self::Spline36 => Self::EwaLanczos,
            Self::EwaLanczos => Self::EwaLanczosSharp,
            Self::EwaLanczosSharp => Self::Spline36,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Spline36 => "Spline36 + Mitchell",
            Self::EwaLanczos => "EWA Lanczos",
            Self::EwaLanczosSharp => "EWA LanczosSharp",
        }
    }
}

/// Demuxer forward/back RAM buffer capacity preset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DemuxerCachePreset {
    Small48M,
    #[default]
    Medium160M,
    Large512M,
}

impl DemuxerCachePreset {
    pub const fn next(self) -> Self {
        match self {
            Self::Small48M => Self::Medium160M,
            Self::Medium160M => Self::Large512M,
            Self::Large512M => Self::Small48M,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Small48M => "48 MiB / 16 MiB",
            Self::Medium160M => "160 MiB / 48 MiB",
            Self::Large512M => "512 MiB / 128 MiB",
        }
    }

    pub const fn limits(self) -> (&'static str, &'static str) {
        match self {
            Self::Small48M => ("48MiB", "16MiB"),
            Self::Medium160M => ("160MiB", "48MiB"),
            Self::Large512M => ("512MiB", "128MiB"),
        }
    }

    pub const fn max_bytes_num(self) -> u64 {
        match self {
            Self::Small48M => 48 * 1024 * 1024,
            Self::Medium160M => 160 * 1024 * 1024,
            Self::Large512M => 512 * 1024 * 1024,
        }
    }
}

/// Granular per-stage GPU, HDR, codec, and buffer configuration for `PerformanceMode::Manual`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualMasteringConfig {
    #[serde(default)]
    pub super_res: SuperResMode,
    #[serde(default)]
    pub codec_priority: CodecPriority,
    #[serde(default)]
    pub hwdec: HwdecMode,
    #[serde(default)]
    pub hdr_compute_peak: bool,
    #[serde(default)]
    pub tone_map: ToneMapMode,
    #[serde(default)]
    pub glsl_stage: GlslStageMode,
    #[serde(default)]
    pub deband: DebandTier,
    #[serde(default)]
    pub gpu_scaler: GpuScalerMode,
    #[serde(default)]
    pub demuxer_cache: DemuxerCachePreset,
}

impl Default for ManualMasteringConfig {
    fn default() -> Self {
        Self::from_preset(PerformanceMode::Balanced)
    }
}

impl ManualMasteringConfig {
    pub const fn from_preset(mode: PerformanceMode) -> Self {
        match mode {
            PerformanceMode::Efficient => Self {
                super_res: SuperResMode::Off,
                codec_priority: CodecPriority::Vp9HwFirst,
                hwdec: HwdecMode::VideoToolbox,
                hdr_compute_peak: false,
                tone_map: ToneMapMode::Spline,
                glsl_stage: GlslStageMode::LightCas,
                deband: DebandTier::Off,
                gpu_scaler: GpuScalerMode::Spline36,
                demuxer_cache: DemuxerCachePreset::Small48M,
            },
            PerformanceMode::Balanced | PerformanceMode::Manual => Self {
                super_res: SuperResMode::Off,
                codec_priority: CodecPriority::Vp9HwFirst,
                hwdec: HwdecMode::VideoToolbox,
                hdr_compute_peak: false,
                tone_map: ToneMapMode::Spline,
                glsl_stage: GlslStageMode::Guided444,
                deband: DebandTier::OnePassCool,
                gpu_scaler: GpuScalerMode::Spline36,
                demuxer_cache: DemuxerCachePreset::Medium160M,
            },
            PerformanceMode::Quality => Self {
                super_res: SuperResMode::Always2x,
                codec_priority: CodecPriority::Av1First,
                hwdec: HwdecMode::AutoSafe,
                hdr_compute_peak: true,
                tone_map: ToneMapMode::Spline,
                glsl_stage: GlslStageMode::FullMastering,
                deband: DebandTier::ThreePassStudio,
                gpu_scaler: GpuScalerMode::EwaLanczosSharp,
                demuxer_cache: DemuxerCachePreset::Large512M,
            },
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

/// Live broadcast optimization profile controlling buffer depth, CDN codec cap, and edge catch-up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LiveStreamMode {
    LowLatency,
    #[default]
    BalancedLive,
    SmoothDvr,
    Off,
}

impl LiveStreamMode {
    pub const fn next(self) -> Self {
        match self {
            Self::LowLatency => Self::BalancedLive,
            Self::BalancedLive => Self::SmoothDvr,
            Self::SmoothDvr => Self::Off,
            Self::Off => Self::LowLatency,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::LowLatency => "Low-Latency (3s buf)",
            Self::BalancedLive => "Balanced Live (6s buf)",
            Self::SmoothDvr => "Smooth DVR (20s buf)",
            Self::Off => "Off (VOD Pipeline)",
        }
    }

    pub const fn short_badge(self) -> &'static str {
        match self {
            Self::LowLatency => "LIVE-LL",
            Self::BalancedLive => "LIVE-BAL",
            Self::SmoothDvr => "LIVE-DVR",
            Self::Off => "LIVE-OFF",
        }
    }

    pub const fn readahead_secs(self) -> u32 {
        match self {
            Self::LowLatency => 3,
            Self::BalancedLive => 6,
            Self::SmoothDvr => 20,
            Self::Off => 10,
        }
    }

    pub const fn cache_pause_wait_secs(self) -> f64 {
        match self {
            Self::LowLatency => 1.2,
            Self::BalancedLive => 2.5,
            Self::SmoothDvr => 4.0,
            Self::Off => 1.5,
        }
    }

    pub const fn cache_pause_initial(self) -> bool {
        !matches!(self, Self::LowLatency)
    }

    pub const fn demuxer_limits(self) -> (&'static str, &'static str) {
        match self {
            Self::LowLatency => ("24MiB", "8MiB"),
            Self::BalancedLive => ("48MiB", "32MiB"),
            Self::SmoothDvr => ("96MiB", "128MiB"),
            Self::Off => ("160MiB", "48MiB"),
        }
    }

    pub const fn catchup_threshold_and_speed(self) -> Option<(f64, f64, f64)> {
        match self {
            Self::LowLatency => Some((6.0, 2.2, 1.06)),
            Self::BalancedLive => Some((12.0, 4.5, 1.04)),
            Self::SmoothDvr | Self::Off => None,
        }
    }
}

/// Persistent application configuration for Phaenna (`config.toml`).
/// Root user configuration persisted to `<config_dir>/config.toml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AnalysisConfig {
    #[serde(default = "default_daily_budget_mins")]
    pub daily_budget_mins: u32,
}

fn default_daily_budget_mins() -> u32 {
    60
}

impl Default for AnalysisConfig {
    fn default() -> Self {
        Self {
            daily_budget_mins: default_daily_budget_mins(),
        }
    }
}

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
    pub manual_mastering: ManualMasteringConfig,
    #[serde(default)]
    pub motion_mode: MotionMode,
    #[serde(default)]
    pub live_mode: LiveStreamMode,
    #[serde(default = "default_true")]
    pub live_auto_catchup: bool,
    #[serde(default)]
    pub network_mode: NetworkMode,
    #[serde(default)]
    pub force_ipv4: bool,
    #[serde(default = "default_true")]
    pub auto_abr_downshift: bool,
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
    pub analysis: AnalysisConfig,
    #[serde(default)]
    pub subtitle_style: crate::services::substyler::SubtitleStyleConfig,
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
            manual_mastering: ManualMasteringConfig::default(),
            motion_mode: MotionMode::default(),
            live_mode: LiveStreamMode::default(),
            live_auto_catchup: true,
            network_mode: NetworkMode::default(),
            force_ipv4: false,
            auto_abr_downshift: true,
            ambilight_enabled: true,
            theme_index: default_theme_index(),
            google_oauth: GoogleOAuthConfig::default(),
            custom_cookies_path: None,
            cache: CacheConfig::default(),
            remote: RemoteConfig::default(),
            cast: CastConfig::default(),
            update: UpdateConfig::default(),
            analysis: AnalysisConfig::default(),
            subtitle_style: crate::services::substyler::SubtitleStyleConfig::default(),
        }
    }
}

#[allow(dead_code)]
impl Config {
    pub fn effective_mastering(&self) -> ManualMasteringConfig {
        if self.performance_mode == PerformanceMode::Manual {
            self.manual_mastering
        } else {
            ManualMasteringConfig::from_preset(self.performance_mode)
        }
    }

    pub fn effective_demuxer_max_bytes_num(&self) -> u64 {
        self.effective_mastering().demuxer_cache.max_bytes_num()
    }

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
        assert!(orig.contains("format_note*=original"));
        assert!(orig.contains("language_preference>=0"));
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

    #[test]
    fn test_live_stream_mode_and_format_selectors() {
        assert_eq!(LiveStreamMode::BalancedLive.short_badge(), "LIVE-BAL");
        assert_eq!(LiveStreamMode::LowLatency.next(), LiveStreamMode::BalancedLive);
        assert_eq!(LiveStreamMode::LowLatency.readahead_secs(), 3);
        assert!(!LiveStreamMode::LowLatency.cache_pause_initial());
        assert!(LiveStreamMode::BalancedLive.cache_pause_initial());
        assert!(LiveStreamMode::SmoothDvr.catchup_threshold_and_speed().is_none());

        let ll = Quality::Best.ytdl_format_for_live(
            LiveStreamMode::LowLatency,
            DubPreference::OriginalOnly,
            "en",
        );
        assert!(ll.contains("height<=?1080"));
        assert!(ll.contains("vcodec!^=av01"));
        assert!(ll.contains("m3u8"));
        assert!(!ll.contains("audio-multistreams"));

        let radio = Quality::AudioOnly.ytdl_format_for_live(
            LiveStreamMode::BalancedLive,
            DubPreference::OriginalOnly,
            "en",
        );
        assert!(radio.contains("bestaudio"));
    }

    #[test]
    fn test_manual_mastering_presets_and_cool_balanced() {
        assert_eq!(PerformanceMode::Quality.next(), PerformanceMode::Manual);
        assert_eq!(PerformanceMode::Manual.next(), PerformanceMode::Efficient);
        assert_eq!(PerformanceMode::Manual.short_badge(), "MAN");

        // Balanced preset is tuned cool for fanless Apple Silicon
        let bal = ManualMasteringConfig::from_preset(PerformanceMode::Balanced);
        assert_eq!(bal.super_res, SuperResMode::Off);
        assert_eq!(bal.codec_priority, CodecPriority::Vp9HwFirst);
        assert_eq!(bal.hwdec, HwdecMode::VideoToolbox);
        assert!(!bal.hdr_compute_peak);
        assert_eq!(bal.glsl_stage, GlslStageMode::Guided444);
        assert_eq!(bal.deband, DebandTier::OnePassCool);
        assert_eq!(bal.gpu_scaler, GpuScalerMode::Spline36);

        // Quality preset stays unchanged at full mastering Fidelity
        let qual = ManualMasteringConfig::from_preset(PerformanceMode::Quality);
        assert_eq!(qual.super_res, SuperResMode::Always2x);
        assert_eq!(qual.codec_priority, CodecPriority::Av1First);
        assert!(qual.hdr_compute_peak);
        assert_eq!(qual.glsl_stage, GlslStageMode::FullMastering);
        assert_eq!(qual.deband, DebandTier::ThreePassStudio);
        assert_eq!(qual.gpu_scaler, GpuScalerMode::EwaLanczosSharp);

        let vp9_fmt = Quality::Best.ytdl_format_with_dub_and_codec(
            DubPreference::OriginalOnly,
            "en",
            true,
            CodecPriority::Vp9HwFirst,
        );
        assert!(vp9_fmt.contains("[vcodec^=vp9]"));
        assert!(!vp9_fmt.contains("[vcodec^=av01]"));

        let mut cfg = Config {
            performance_mode: PerformanceMode::Manual,
            ..Default::default()
        };
        cfg.manual_mastering.super_res = SuperResMode::Auto1080p;
        cfg.manual_mastering.demuxer_cache = DemuxerCachePreset::Large512M;
        assert_eq!(cfg.effective_mastering().super_res, SuperResMode::Auto1080p);
        assert_eq!(cfg.effective_demuxer_max_bytes_num(), 512 * 1024 * 1024);
    }

    #[test]
    fn test_network_mode_and_congested_selectors() {
        assert_eq!(NetworkMode::ALL.len(), 3);
        assert_eq!(NetworkMode::Normal.next(), NetworkMode::CongestedWifi);
        assert_eq!(NetworkMode::CongestedWifi.next(), NetworkMode::UltraSaver);
        assert_eq!(NetworkMode::UltraSaver.next(), NetworkMode::Normal);
        assert!(!NetworkMode::Normal.is_congested());
        assert!(NetworkMode::CongestedWifi.is_congested());
        assert!(NetworkMode::UltraSaver.is_congested());
        assert_eq!(NetworkMode::Normal.short_badge(), "NET:FULL");
        assert_eq!(NetworkMode::CongestedWifi.short_badge(), "NET:WIFI");
        assert_eq!(NetworkMode::UltraSaver.short_badge(), "NET:ECO");
        assert_eq!(NetworkMode::CongestedWifi.max_height_cap(), Some(720));
        assert_eq!(NetworkMode::UltraSaver.max_height_cap(), Some(480));
        assert!(!NetworkMode::CongestedWifi.effective_multistreams(true, PerformanceMode::Quality));
        assert!(NetworkMode::Normal.effective_multistreams(true, PerformanceMode::Quality));

        let dir = tempdir().unwrap();
        let cfg_path = dir.path().join("net_config.toml");
        let cfg = Config {
            network_mode: NetworkMode::CongestedWifi,
            force_ipv4: true,
            auto_abr_downshift: false,
            ..Default::default()
        };
        cfg.save_to_path(&cfg_path).unwrap();
        let loaded = Config::load_from_path(&cfg_path).unwrap();
        assert_eq!(loaded.network_mode, NetworkMode::CongestedWifi);
        assert!(loaded.force_ipv4);
        assert!(!loaded.auto_abr_downshift);

        let wifi_fmt = Quality::Best.ytdl_format_for_network(
            DubPreference::OriginalOnly,
            "en",
            true,
            CodecPriority::Vp9HwFirst,
            NetworkMode::CongestedWifi,
        );
        assert!(wifi_fmt.contains("height<=?720"));
        assert!(wifi_fmt.contains("fps<=?30"));
        assert!(wifi_fmt.contains("abr<=?128"));
        assert!(wifi_fmt.contains("vcodec^=av01"));
        assert!(!wifi_fmt.contains(")+("));

        let ultra_fmt = Quality::Best.ytdl_format_for_network(
            DubPreference::OriginalOnly,
            "en",
            true,
            CodecPriority::Av1First,
            NetworkMode::UltraSaver,
        );
        assert!(ultra_fmt.contains("height<=?480"));
        assert!(ultra_fmt.contains("fps<=?30"));
        assert!(ultra_fmt.contains("abr<=?72"));

        let low_cap_fmt = Quality::P360.ytdl_format_for_network(
            DubPreference::OriginalOnly,
            "en",
            false,
            CodecPriority::Av1First,
            NetworkMode::CongestedWifi,
        );
        assert!(low_cap_fmt.contains("height<=?360"));
        assert!(!low_cap_fmt.contains("height<=?720"));
    }
}
