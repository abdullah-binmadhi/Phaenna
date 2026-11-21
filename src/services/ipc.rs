//! Non-blocking mpv JSON-RPC IPC socket monitor.
//! Tracks live `time-pos` & `duration`, persists playback resume state every 4s,
//! triggers 85% completion watch logging, and performs automatic SponsorBlock segment skipping.

use crate::api::player::LOUDNORM_FILTER;
use crate::api::sponsorblock::{
    category_label, fetch_sponsor_segments, filter_segments_for_mode_with_outro, from_db_rows,
    SponsorSegment,
};
use crate::config::{
    DebandTier, GpuScalerMode, ManualMasteringConfig, MotionMode, NetworkMode, PerformanceMode,
    SponsorBlockMode, VideoShaderMode,
};
use crate::db;
use crate::messages::AppMessage;
use crate::services::shaders::{prepare_video_shaders_with_mastering, VideoMasteringProfile};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};
#[cfg(unix)]
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{
    mpsc::{self, Sender},
    oneshot,
};

#[cfg(unix)]
type IpcStream = tokio::net::UnixStream;

#[cfg(windows)]
type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;

#[cfg(not(any(unix, windows)))]
type IpcStream = tokio::io::DuplexStream;

async fn connect_ipc(sock_path: &str) -> Option<IpcStream> {
    #[cfg(unix)]
    {
        if Path::new(sock_path).exists() {
            tokio::net::UnixStream::connect(sock_path).await.ok()
        } else {
            None
        }
    }
    #[cfg(windows)]
    {
        tokio::net::windows::named_pipe::ClientOptions::new()
            .open(sock_path)
            .ok()
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = sock_path;
        None
    }
}

pub struct IpcMonitorConfig {
    pub session_id: u64,
    pub video_id: String,
    pub channel_id: Option<String>,
    pub channel_name: String,
    pub is_live: bool,
    pub duration_hint_secs: f64,
    pub initial_pos_secs: f64,
    pub sock_path: String,
    pub pool: SqlitePool,
    pub tx: Sender<AppMessage>,
    pub skip_segments: Vec<SponsorSegment>,
    pub sponsorblock_fetch: Option<(reqwest::Client, SponsorBlockMode, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackKind {
    Audio,
    Sub,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiveMediaTrack {
    pub id: i64,
    pub kind: TrackKind,
    pub lang: Option<String>,
    pub title: Option<String>,
    pub is_dubbed: bool,
    pub is_original: bool,
    pub selected: bool,
}

impl LiveMediaTrack {
    pub fn display_label(&self) -> String {
        let lang = self.lang.as_deref().unwrap_or("und");
        let raw_title = self
            .title
            .as_deref()
            .map(str::trim)
            .unwrap_or("");
        // Strip verbose YouTube prefix/suffix like "251-drc - audio only (English (US) original)"
        let clean_title = if let Some(start) = raw_title.find("audio only (") {
            let inner = &raw_title[start + "audio only (".len()..];
            inner.trim_end_matches(')').trim()
        } else {
            raw_title
        };
        let role = if self.kind == TrackKind::Audio {
            if self.is_dubbed {
                " [DUB]"
            } else if self.is_original {
                " [ORIG]"
            } else {
                ""
            }
        } else {
            ""
        };
        let label = if clean_title.is_empty() {
            lang.to_string()
        } else {
            clean_title.replace(" original", "").trim().to_string()
        };
        format!("[{}] {}{}", self.id, label, role)
    }
}

#[derive(Debug, Clone)]
pub enum LiveIpcEvent {
    Paused(bool),
    Volume(i64),
    Muted(bool),
    Speed(f64),
    AudioId(Option<i64>),
    SubId(Option<i64>),
    SubVisible(bool),
    SubDelay(f64),
    AudioDelay(f64),
    OnTop(bool),
    LoopVideo(bool),
    Brightness(i64),
    Contrast(i64),
    Saturation(i64),
    Gamma(i64),
    CacheBytes(u64),
    LiveCacheTelemetry {
        cache_ahead_secs: f64,
        seekable_end_secs: f64,
    },
    PausedForCache(bool),
    CacheSpeed(u64),
    BufferingPct(i64),
    DroppedFrames(i64),
    VfFps(f64),
    VideoBitrateBps(f64),
    AudioBitrateBps(f64),
    AvSyncSecs(f64),
    HwdecCurrent(String),
    VideoCodec(String),
    VideoWidth(u32),
    VideoHeight(u32),
    ColorMatrix(String),
    HdrSigPeak(f64),
    DelayedFrames(i64),
    AudioSampleRate(u32),
    AudioChannels(String),
    Tracks {
        audio: Vec<LiveMediaTrack>,
        sub: Vec<LiveMediaTrack>,
    },
}

#[derive(Debug, Clone)]
pub enum PlayerCommand {
    TogglePause,
    SeekRelative(f64),
    SeekAbsolute(f64),
    ChapterStep(i32),
    SetVolume(i64),
    ToggleMute,
    SetSpeed(f64),
    SetAudioTrack(i64),
    SetSubTrack(i64),
    SetSubVisible(bool),
    SetSubDelay(f64),
    SetAudioDelay(f64),
    SetLoudnorm(bool),
    SetShaderMode(VideoShaderMode, VideoMasteringProfile, PerformanceMode),
    SetPerformanceMode(PerformanceMode, VideoShaderMode, VideoMasteringProfile),
    ApplyManualMastering(ManualMasteringConfig, VideoShaderMode, VideoMasteringProfile),
    SetMotionMode(MotionMode),
    SetPictureProp { prop: &'static str, value: i64 },
    ResetPicture,
    SetOnTop(bool),
    SetLoop(bool),
    JumpToLiveEdge,
    SetNetworkMode(NetworkMode),
    HotSwitchStream {
        primary_url: String,
        audio_url: Option<String>,
        user_agent: Option<String>,
        resume_pos: f64,
        osd_message: String,
    },
    Stop,
}

fn mastering_ipc_commands(
    perf: PerformanceMode,
    shader: VideoShaderMode,
    manual: ManualMasteringConfig,
) -> Vec<String> {
    let eff = if perf == PerformanceMode::Manual {
        manual
    } else {
        ManualMasteringConfig::from_preset(perf)
    };
    let (max_b, back_b) = eff.demuxer_cache.limits();
    let mut cmds = vec![
        json!({"command": ["set_property", "demuxer-max-bytes", max_b]}).to_string(),
        json!({"command": ["set_property", "demuxer-max-back-bytes", back_b]}).to_string(),
        json!({"command": ["set_property", "tone-mapping", eff.tone_map.mpv_value()]}).to_string(),
        json!({"command": ["set_property", "hdr-compute-peak", if eff.hdr_compute_peak { "yes" } else { "no" }]}).to_string(),
        json!({"command": ["set_property", "hwdec", eff.hwdec.mpv_value()]}).to_string(),
    ];

    if perf == PerformanceMode::Efficient {
        cmds.extend([
            json!({"command": ["set_property", "scale", "spline36"]}).to_string(),
            json!({"command": ["set_property", "cscale", "bilinear"]}).to_string(),
            json!({"command": ["set_property", "deband", "no"]}).to_string(),
            json!({"command": ["set_property", "temporal-dither", "no"]}).to_string(),
            json!({"command": ["set_property", "vd-lavc-fast", "yes"]}).to_string(),
        ]);
    } else {
        let (scale, cscale) = match eff.gpu_scaler {
            GpuScalerMode::Spline36 => ("spline36", "mitchell"),
            GpuScalerMode::EwaLanczos => ("ewa_lanczos", "ewa_lanczos"),
            GpuScalerMode::EwaLanczosSharp => ("ewa_lanczossharp", "ewa_lanczossharp"),
        };
        cmds.push(json!({"command": ["set_property", "scale", scale]}).to_string());
        cmds.push(json!({"command": ["set_property", "cscale", cscale]}).to_string());
        match eff.deband {
            DebandTier::Off => {
                cmds.push(json!({"command": ["set_property", "deband", "no"]}).to_string());
                cmds.push(json!({"command": ["set_property", "temporal-dither", "no"]}).to_string());
            }
            DebandTier::OnePassCool => {
                cmds.push(json!({"command": ["set_property", "deband", "yes"]}).to_string());
                cmds.push(json!({"command": ["set_property", "deband-iterations", 1]}).to_string());
                cmds.push(json!({"command": ["set_property", "temporal-dither", "no"]}).to_string());
            }
            DebandTier::ThreePassStudio => {
                cmds.push(json!({"command": ["set_property", "deband", "yes"]}).to_string());
                cmds.push(json!({"command": ["set_property", "deband-iterations", 3]}).to_string());
                cmds.push(json!({"command": ["set_property", "temporal-dither", "yes"]}).to_string());
            }
        }
        cmds.push(json!({"command": ["set_property", "vd-lavc-fast", "no"]}).to_string());
    }

    if let Some(shaders) = prepare_video_shaders_with_mastering(shader, perf, Some(eff)) {
        cmds.push(json!({"command": ["change-list", "glsl-shaders", "set", shaders]}).to_string());
    } else {
        cmds.push(json!({"command": ["change-list", "glsl-shaders", "clr", ""]}).to_string());
    }
    cmds
}

impl PlayerCommand {
    /// Serialize into newline-free `mpv` JSON-RPC command strings.
    pub fn to_mpv_json_commands(&self) -> Vec<String> {
        match self {
            Self::TogglePause => vec![json!({"command": ["cycle", "pause"]}).to_string()],
            Self::SeekRelative(sec) => {
                vec![json!({"command": ["seek", sec, "relative"]}).to_string()]
            }
            Self::SeekAbsolute(pos) => {
                vec![json!({"command": ["seek", pos.max(0.0), "absolute"]}).to_string()]
            }
            Self::ChapterStep(step) => {
                vec![json!({"command": ["add", "chapter", step]}).to_string()]
            }
            Self::SetVolume(vol) => {
                vec![json!({"command": ["set_property", "volume", (*vol).clamp(0, 150)]}).to_string()]
            }
            Self::ToggleMute => vec![json!({"command": ["cycle", "mute"]}).to_string()],
            Self::SetSpeed(spd) => {
                let s = (spd.clamp(0.25, 4.0) * 100.0).round() / 100.0;
                vec![json!({"command": ["set_property", "speed", s]}).to_string()]
            }
            Self::SetAudioTrack(aid) => {
                vec![json!({"command": ["set_property", "aid", aid]}).to_string()]
            }
            Self::SetSubTrack(sid) => vec![
                json!({"command": ["set_property", "sid", sid]}).to_string(),
                json!({"command": ["set_property", "sub-visibility", true]}).to_string(),
            ],
            Self::SetSubVisible(vis) => {
                vec![json!({"command": ["set_property", "sub-visibility", vis]}).to_string()]
            }
            Self::SetSubDelay(secs) => {
                vec![json!({"command": ["set_property", "sub-delay", secs]}).to_string()]
            }
            Self::SetAudioDelay(secs) => {
                vec![json!({"command": ["set_property", "audio-delay", secs]}).to_string()]
            }
            Self::SetLoudnorm(enabled) => {
                let val = if *enabled { LOUDNORM_FILTER } else { "" };
                vec![json!({"command": ["set_property", "af", val]}).to_string()]
            }
            Self::SetShaderMode(mode, _profile, perf) => {
                let mut cmds = Vec::new();
                if let Some(shaders) = prepare_video_shaders_with_mastering(*mode, *perf, None) {
                    cmds.push(
                        json!({"command": ["change-list", "glsl-shaders", "set", shaders]})
                            .to_string(),
                    );
                    if *perf != PerformanceMode::Efficient {
                        let eff = ManualMasteringConfig::from_preset(*perf);
                        cmds.push(json!({"command": ["set_property", "deband", "yes"]}).to_string());
                        cmds.push(
                            json!({"command": ["set_property", "tone-mapping", eff.tone_map.mpv_value()]}).to_string(),
                        );
                    }
                } else {
                    cmds.push(
                        json!({"command": ["change-list", "glsl-shaders", "clr", ""]})
                            .to_string(),
                    );
                    cmds.push(json!({"command": ["set_property", "deband", "no"]}).to_string());
                }
                cmds
            }
            Self::SetPerformanceMode(perf, shader, _profile) => {
                mastering_ipc_commands(*perf, *shader, ManualMasteringConfig::from_preset(*perf))
            }
            Self::ApplyManualMastering(manual, shader, _profile) => {
                mastering_ipc_commands(PerformanceMode::Manual, *shader, *manual)
            }
            Self::SetMotionMode(motion) => match motion {
                MotionMode::Off => vec![
                    json!({"command": ["set_property", "interpolation", "no"]}).to_string(),
                    json!({"command": ["set_property", "video-sync", "audio"]}).to_string(),
                ],
                MotionMode::Cadence => vec![
                    json!({"command": ["set_property", "video-sync", "display-resample"]})
                        .to_string(),
                    json!({"command": ["set_property", "interpolation", "yes"]}).to_string(),
                    json!({"command": ["set_property", "tscale", "oversample"]}).to_string(),
                ],
                MotionMode::Smooth120Hz => vec![
                    json!({"command": ["set_property", "video-sync", "display-resample"]})
                        .to_string(),
                    json!({"command": ["set_property", "interpolation", "yes"]}).to_string(),
                    json!({"command": ["set_property", "tscale", "catmull_rom"]}).to_string(),
                    json!({"command": ["set_property", "override-display-fps", 120]}).to_string(),
                ],
            },
            Self::SetPictureProp { prop, value } => {
                vec![json!({"command": ["set_property", prop, (*value).clamp(-100, 100)]}).to_string()]
            }
            Self::ResetPicture => ["brightness", "contrast", "saturation", "gamma"]
                .iter()
                .map(|p| json!({"command": ["set_property", p, 0]}).to_string())
                .collect(),
            Self::SetOnTop(on) => {
                vec![json!({"command": ["set_property", "ontop", on]}).to_string()]
            }
            Self::SetLoop(on) => {
                let v = if *on { "inf" } else { "no" };
                vec![json!({"command": ["set_property", "loop-file", v]}).to_string()]
            }
            Self::JumpToLiveEdge => vec![
                json!({"command": ["script-binding", "phaenna-snap-live-edge"]}).to_string(),
                json!({"command": ["set_property", "speed", 1.0]}).to_string(),
            ],
            Self::SetNetworkMode(mode) => vec![
                json!({"command": ["set_property", "cache-pause-wait", mode.vod_cache_pause_wait_secs()]}).to_string(),
                json!({"command": ["set_property", "demuxer-readahead-secs", mode.vod_readahead_secs()]}).to_string(),
                json!({"command": ["set_property", "network-timeout", mode.network_timeout_secs()]}).to_string(),
                json!({"command": ["show-text", format!("Phaenna Network: {}", mode.label()), 2500]}).to_string(),
            ],
            Self::HotSwitchStream {
                primary_url,
                audio_url,
                user_agent,
                resume_pos,
                osd_message,
            } => {
                let mut cmds = Vec::new();
                if *resume_pos > 1.0 {
                    cmds.push(
                        json!({"command": ["set_property", "start", format!("{:.2}", resume_pos)]})
                            .to_string(),
                    );
                }
                if let Some(ref ua) = user_agent {
                    if !ua.trim().is_empty() {
                        cmds.push(json!({"command": ["set_property", "user-agent", ua]}).to_string());
                    }
                }
                if let Some(ref aurl) = audio_url {
                    cmds.push(json!({"command": ["set_property", "audio-files", [aurl]]}).to_string());
                } else {
                    cmds.push(json!({"command": ["set_property", "audio-files", []]}).to_string());
                }
                cmds.push(json!({"command": ["loadfile", primary_url, "replace"]}).to_string());
                cmds.push(json!({"command": ["show-text", osd_message, 3500]}).to_string());
                cmds
            }
            Self::Stop => vec![json!({"command": ["quit"]}).to_string()],
        }
    }
}

/// Parse `mpv` JSON `track-list` array into `(audio_tracks, sub_tracks)`.
pub fn parse_track_list(val: &Value) -> (Vec<LiveMediaTrack>, Vec<LiveMediaTrack>) {
    let Some(arr) = val.as_array() else {
        return (Vec::new(), Vec::new());
    };
    struct RawAudio {
        id: i64,
        lang: Option<String>,
        title: Option<String>,
        selected: bool,
        explicit_dub: bool,
        explicit_orig: bool,
    }
    let mut raw_audio = Vec::new();
    let mut sub = Vec::new();
    for item in arr {
        let Some(kind_s) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        let Some(id) = item.get("id").and_then(Value::as_i64) else {
            continue;
        };
        let lang = item.get("lang").and_then(Value::as_str).map(ToOwned::to_owned);
        let title = item.get("title").and_then(Value::as_str).map(ToOwned::to_owned);
        let selected = item.get("selected").and_then(Value::as_bool).unwrap_or(false);
        let title_low = title.as_deref().unwrap_or("").to_lowercase();

        match kind_s {
            "audio" => {
                let explicit_dub = title_low.contains("dub") || title_low.contains("auto-dub");
                let explicit_orig = !explicit_dub
                    && (title_low.contains("original")
                        || item.get("default").and_then(Value::as_bool).unwrap_or(false));
                raw_audio.push(RawAudio {
                    id,
                    lang,
                    title,
                    selected,
                    explicit_dub,
                    explicit_orig,
                });
            }
            "sub" => {
                sub.push(LiveMediaTrack {
                    id,
                    kind: TrackKind::Sub,
                    lang,
                    title,
                    is_dubbed: false,
                    is_original: false,
                    selected,
                });
            }
            _ => {}
        }
    }

    let any_explicit_orig = raw_audio.iter().any(|t| t.explicit_orig);
    let audio = raw_audio
        .into_iter()
        .enumerate()
        .map(|(idx, t)| {
            let is_original = t.explicit_orig || (!any_explicit_orig && !t.explicit_dub && idx == 0);
            let is_dubbed = t.explicit_dub || (any_explicit_orig && !is_original);
            LiveMediaTrack {
                id: t.id,
                kind: TrackKind::Audio,
                lang: t.lang,
                title: t.title,
                is_dubbed,
                is_original,
                selected: t.selected,
            }
        })
        .collect();
    (audio, sub)
}

/// Parse mpv JSON-RPC `property-change` event line -> `(property_name, float_value)`.
pub fn parse_property_change(line: &str) -> Option<(String, f64)> {
    let parsed: Value = serde_json::from_str(line).ok()?;
    if parsed.get("event")?.as_str()? == "property-change" {
        let name = parsed.get("name")?.as_str()?.to_string();
        let data = parsed.get("data")?.as_f64()?;
        return Some((name, data));
    }
    None
}

/// Parse any observed mpv property-change line into a typed `LiveIpcEvent`.
pub fn parse_live_ipc_event(line: &str) -> Option<LiveIpcEvent> {
    let parsed: Value = serde_json::from_str(line).ok()?;
    if parsed.get("event")?.as_str()? != "property-change" {
        return None;
    }
    let name = parsed.get("name")?.as_str()?;
    let data = parsed.get("data")?;
    match name {
        "pause" => Some(LiveIpcEvent::Paused(data.as_bool()?)),
        "volume" => Some(LiveIpcEvent::Volume(data.as_f64()?.round() as i64)),
        "mute" => Some(LiveIpcEvent::Muted(data.as_bool()?)),
        "speed" => Some(LiveIpcEvent::Speed(data.as_f64()?)),
        "aid" => Some(LiveIpcEvent::AudioId(data.as_i64())),
        "sid" => Some(LiveIpcEvent::SubId(data.as_i64())),
        "sub-visibility" => Some(LiveIpcEvent::SubVisible(data.as_bool()?)),
        "sub-delay" => Some(LiveIpcEvent::SubDelay(data.as_f64()?)),
        "audio-delay" => Some(LiveIpcEvent::AudioDelay(data.as_f64()?)),
        "ontop" => Some(LiveIpcEvent::OnTop(data.as_bool()?)),
        "loop-file" => {
            let on = data.as_bool().unwrap_or_else(|| {
                data.as_str()
                    .map(|s| s == "inf" || s == "yes")
                    .unwrap_or(false)
            });
            Some(LiveIpcEvent::LoopVideo(on))
        }
        "brightness" => Some(LiveIpcEvent::Brightness(data.as_f64()?.round() as i64)),
        "contrast" => Some(LiveIpcEvent::Contrast(data.as_f64()?.round() as i64)),
        "saturation" => Some(LiveIpcEvent::Saturation(data.as_f64()?.round() as i64)),
        "gamma" => Some(LiveIpcEvent::Gamma(data.as_f64()?.round() as i64)),
        "frame-drop-count" => Some(LiveIpcEvent::DroppedFrames(data.as_i64()?)),
        "paused-for-cache" => Some(LiveIpcEvent::PausedForCache(data.as_bool()?)),
        "cache-speed" => Some(LiveIpcEvent::CacheSpeed(data.as_f64()?.max(0.0).round() as u64)),
        "cache-buffering-state" => Some(LiveIpcEvent::BufferingPct(data.as_f64()?.round() as i64)),
        "estimated-vf-fps" => Some(LiveIpcEvent::VfFps(data.as_f64()?.max(0.0))),
        "video-bitrate" => Some(LiveIpcEvent::VideoBitrateBps(data.as_f64()?.max(0.0))),
        "audio-bitrate" => Some(LiveIpcEvent::AudioBitrateBps(data.as_f64()?.max(0.0))),
        "avsync" => Some(LiveIpcEvent::AvSyncSecs(data.as_f64()?)),
        "hwdec-current" => {
            let s = data.as_str()?.trim();
            (!s.is_empty()).then(|| LiveIpcEvent::HwdecCurrent(s.to_string()))
        }
        "video-codec" => {
            let s = data.as_str()?.trim();
            (!s.is_empty()).then(|| LiveIpcEvent::VideoCodec(s.to_string()))
        }
        "video-params/w" => Some(LiveIpcEvent::VideoWidth(data.as_u64()? as u32)),
        "video-params/h" => Some(LiveIpcEvent::VideoHeight(data.as_u64()? as u32)),
        "video-params/colormatrix" => {
            let s = data.as_str()?.trim();
            (!s.is_empty()).then(|| LiveIpcEvent::ColorMatrix(s.to_string()))
        }
        "video-params/sig-peak" => Some(LiveIpcEvent::HdrSigPeak(data.as_f64()?.max(0.0))),
        "vo-delayed-frame-count" => Some(LiveIpcEvent::DelayedFrames(data.as_i64()?)),
        "audio-params/samplerate" => Some(LiveIpcEvent::AudioSampleRate(data.as_u64()? as u32)),
        "audio-params/hr-channels" => {
            let s = data.as_str()?.trim();
            (!s.is_empty()).then(|| LiveIpcEvent::AudioChannels(s.to_string()))
        }
        "demuxer-cache-state" => {
            let bytes = data
                .get("fw-bytes")
                .or_else(|| data.get("total-bytes"))
                .and_then(Value::as_u64)?;
            Some(LiveIpcEvent::CacheBytes(bytes))
        }
        "track-list" => {
            let (mut audio, sub) = parse_track_list(data);
            // Put selected and Original tracks at the top ahead of secondary languages
            audio.sort_by_key(|t| (!t.selected, !t.is_original, t.is_dubbed, t.id));
            Some(LiveIpcEvent::Tracks { audio, sub })
        }
        _ => None,
    }
}

/// Parse optional buffer-duration & seekable-ranges end seconds from `demuxer-cache-state`.
pub fn parse_live_cache_telemetry(line: &str) -> Option<(f64, f64)> {
    let parsed: Value = serde_json::from_str(line).ok()?;
    if parsed.get("event")?.as_str()? != "property-change" {
        return None;
    }
    if parsed.get("name")?.as_str()? != "demuxer-cache-state" {
        return None;
    }
    let data = parsed.get("data")?;
    let ahead = data
        .get("cache-duration")
        .and_then(Value::as_f64)
        .unwrap_or(0.0)
        .max(0.0);
    let end = data
        .get("seekable-ranges")
        .and_then(Value::as_array)
        .and_then(|arr| arr.iter().filter_map(|r| r.get("end").and_then(Value::as_f64)).reduce(f64::max))
        .or_else(|| data.get("cache-end").and_then(Value::as_f64))
        .unwrap_or(0.0)
        .max(0.0);
    (ahead > 0.0 || end > 0.0).then_some((ahead, end))
}

/// EOF-safe SponsorBlock matcher: never seeks past `duration - 2.0s` so video endings aren't cut short.
pub fn check_skip_interval_with_duration<'a>(
    pos: f64,
    duration: f64,
    segments: &'a [SponsorSegment],
    skipped_uuids: &HashSet<String>,
) -> Option<&'a SponsorSegment> {
    for seg in segments {
        if skipped_uuids.contains(&seg.uuid) {
            continue;
        }
        if duration > 0.0 && seg.end_time >= (duration - 2.0).max(0.0) {
            continue;
        }
        if pos >= seg.start_time - 0.35 && pos < seg.end_time - 0.8 {
            return Some(seg);
        }
    }
    None
}

/// Return true once watched fraction reaches 85% (for videos >= 60s).
pub fn is_completion_threshold_reached(pos: f64, duration: f64) -> bool {
    if duration < 60.0 {
        return false;
    }
    (pos / duration) >= 0.85
}

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn is_file_loaded_event(line: &str) -> bool {
    serde_json::from_str::<Value>(line)
        .ok()
        .and_then(|v| {
            v.get("event")
                .and_then(Value::as_str)
                .map(|e| e == "file-loaded")
        })
        .unwrap_or(false)
}

pub fn spawn_mpv_ipc_monitor(config: IpcMonitorConfig) {
    tokio::spawn(async move {
        let mut stream = None;
        for _ in 0..45 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Some(s) = connect_ipc(&config.sock_path).await {
                stream = Some(s);
                break;
            }
        }

        let Some(s) = stream else {
            let _ = config.tx.send(AppMessage::PlaybackDone(config.session_id)).await;
            return;
        };

        let (reader, mut writer) = tokio::io::split(s);
        let (cmd_tx, mut cmd_rx) = mpsc::unbounded_channel::<PlayerCommand>();
        let _ = config
            .tx
            .send(AppMessage::PlaybackCommandReady {
                session_id: config.session_id,
                cmd_tx,
            })
            .await;

        let observe_props = [
            (1, "time-pos"),
            (2, "duration"),
            (3, "pause"),
            (4, "volume"),
            (5, "mute"),
            (6, "speed"),
            (7, "aid"),
            (8, "sid"),
            (9, "sub-visibility"),
            (10, "sub-delay"),
            (11, "audio-delay"),
            (12, "ontop"),
            (13, "loop-file"),
            (14, "brightness"),
            (15, "contrast"),
            (16, "saturation"),
            (17, "gamma"),
            (18, "track-list"),
            (19, "demuxer-cache-state"),
            (20, "frame-drop-count"),
            (21, "paused-for-cache"),
            (22, "cache-speed"),
            (23, "cache-buffering-state"),
            (24, "estimated-vf-fps"),
            (25, "video-bitrate"),
            (26, "audio-bitrate"),
            (27, "avsync"),
            (28, "hwdec-current"),
            (29, "video-codec"),
            (30, "video-params/w"),
            (31, "video-params/h"),
            (32, "video-params/colormatrix"),
            (33, "video-params/sig-peak"),
            (34, "vo-delayed-frame-count"),
            (35, "audio-params/samplerate"),
            (36, "audio-params/hr-channels"),
        ];
        for (id, prop) in observe_props {
            let msg = format!("{{\"command\":[\"observe_property\",{id},\"{prop}\"]}}\n");
            let _ = writer.write_all(msg.as_bytes()).await;
        }

        struct SessionTracker {
            started_at: i64,
            watched_wall_secs: f64,
            watched_media_secs: f64,
            current_speed: f64,
            is_paused: bool,
            sb_skipped_session: f64,
            sb_cats: HashMap<String, f64>,
        }

        impl SessionTracker {
            async fn flush(
                &self,
                config: &IpcMonitorConfig,
                last_pos: f64,
                last_dur: f64,
                completed: bool,
            ) {
                if self.watched_wall_secs < 4.0 && !completed {
                    return;
                }
                let cats_json = if self.sb_cats.is_empty() {
                    "{}".to_string()
                } else {
                    serde_json::to_string(&self.sb_cats).unwrap_or_else(|_| "{}".into())
                };
                let _ = db::user::upsert_watch_session(
                    &config.pool,
                    &db::user::WatchSessionWrite {
                        video_id: &config.video_id,
                        channel_id: config.channel_id.as_deref(),
                        channel_name: &config.channel_name,
                        source: "local_ipc",
                        started_at: self.started_at,
                        ended_at: now_unix().max(self.started_at),
                        watched_secs: self.watched_wall_secs,
                        media_secs: self.watched_media_secs,
                        start_pos_secs: config.initial_pos_secs,
                        end_pos_secs: last_pos,
                        video_duration_secs: last_dur,
                        playback_speed: self.current_speed,
                        sb_skipped_secs: self.sb_skipped_session,
                        sb_categories_json: &cats_json,
                        player: "mpv",
                        completed,
                    },
                )
                .await;
            }
        }

        let mut lines = BufReader::new(reader).lines();
        let mut last_pos = config.initial_pos_secs;
        let mut prev_pos = config.initial_pos_secs;
        let mut last_dur = config.duration_hint_secs;
        let mut tracker = SessionTracker {
            started_at: now_unix(),
            watched_wall_secs: 0.0,
            watched_media_secs: 0.0,
            current_speed: 1.0,
            is_paused: false,
            sb_skipped_session: 0.0,
            sb_cats: HashMap::new(),
        };
        let mut last_save = std::time::Instant::now();
        let mut last_session_save = std::time::Instant::now();
        let mut last_ui_tick = std::time::Instant::now();
        let mut skipped_uuids: HashSet<String> = HashSet::new();
        let mut marked_watched = false;
        let mut skip_segments = config.skip_segments.clone();
        let mut pending_reset_start = false;
        let mut sb_rx: Option<oneshot::Receiver<Vec<SponsorSegment>>> = None;

        if skip_segments.is_empty() && !config.is_live {
            if let Some((sb_client, sb_mode, skip_outro)) = config.sponsorblock_fetch.clone() {
                if sb_mode != SponsorBlockMode::Off {
                    let (sb_tx, rx) = oneshot::channel();
                    let sb_vid = config.video_id.clone();
                    let sb_pool = config.pool.clone();
                    tokio::spawn(async move {
                        let raw = match db::user::get_sponsorblock_segments(&sb_pool, &sb_vid).await {
                            Ok(cached) if !cached.is_empty() => from_db_rows(&cached),
                            _ => fetch_sponsor_segments(&sb_client, &sb_vid)
                                .await
                                .unwrap_or_default(),
                        };
                        if !raw.is_empty() {
                            let filtered =
                                filter_segments_for_mode_with_outro(&raw, sb_mode, skip_outro);
                            let _ = sb_tx.send(filtered);
                        }
                    });
                    sb_rx = Some(rx);
                }
            }
        }

        loop {
            tokio::select! {
                sb_res = async {
                    match sb_rx.as_mut() {
                        Some(rx) => rx.await.ok(),
                        None => std::future::pending().await,
                    }
                }, if sb_rx.is_some() => {
                    sb_rx = None;
                    if let Some(segs) = sb_res {
                        skip_segments = segs;
                    }
                }
                maybe_cmd = cmd_rx.recv() => {
                    match maybe_cmd {
                        Some(cmd) => {
                            if matches!(cmd, PlayerCommand::HotSwitchStream { .. }) {
                                pending_reset_start = true;
                            }
                            for raw in cmd.to_mpv_json_commands() {
                                let line = format!("{raw}\n");
                                if writer.write_all(line.as_bytes()).await.is_err() {
                                    break;
                                }
                            }
                        }
                        None => break,
                    }
                }
                next_line = lines.next_line() => {
                    let Ok(Some(line)) = next_line else {
                        break;
                    };

                    if pending_reset_start && is_file_loaded_event(&line) {
                        pending_reset_start = false;
                        let _ = writer
                            .write_all(b"{\"command\":[\"set_property\",\"start\",\"none\"]}\n")
                            .await;
                    }

                    if let Some((name, val)) = parse_property_change(&line) {
                        match name.as_str() {
                            "duration" => {
                                if val > 0.0 {
                                    last_dur = val;
                                }
                            }
                            "time-pos" => {
                                if !tracker.is_paused && prev_pos > 0.0 {
                                    let dt = val - prev_pos;
                                    if dt > 0.0 && dt <= 6.0 {
                                        tracker.watched_media_secs += dt;
                                        tracker.watched_wall_secs +=
                                            dt / tracker.current_speed.clamp(0.25, 4.0);
                                    }
                                }
                                prev_pos = val;
                                last_pos = val;

                                if let Some(seg) = check_skip_interval_with_duration(
                                    val,
                                    last_dur,
                                    &skip_segments,
                                    &skipped_uuids,
                                ) {
                                    skipped_uuids.insert(seg.uuid.clone());
                                    let target_end = if last_dur > 2.5 {
                                        seg.end_time.min(last_dur - 2.0)
                                    } else {
                                        seg.end_time
                                    };
                                    let seek_cmd = format!(
                                        "{{\"command\":[\"set_property\",\"time-pos\",{:.2}]}}\n",
                                        target_end
                                    );
                                    if writer.write_all(seek_cmd.as_bytes()).await.is_ok() {
                                        let saved = (target_end - seg.start_time).max(0.0);
                                        tracker.sb_skipped_session += saved;
                                        *tracker.sb_cats.entry(seg.category.clone()).or_insert(0.0) +=
                                            saved;
                                        let _ = db::user::add_sponsorblock_saved_secs(&config.pool, saved).await;
                                        let _ = config
                                            .tx
                                            .send(AppMessage::SponsorSkipped {
                                                video_id: config.video_id.clone(),
                                                category: category_label(&seg.category).to_string(),
                                                saved_secs: saved,
                                            })
                                            .await;
                                    }
                                }

                                if !config.is_live
                                    && !marked_watched
                                    && is_completion_threshold_reached(last_pos, last_dur)
                                {
                                    marked_watched = true;
                                    tracker.flush(&config, last_pos, last_dur, true).await;
                                    let _ = db::user::record_watched(
                                        &config.pool,
                                        None,
                                        &config.video_id,
                                        now_unix(),
                                    )
                                    .await;
                                    let _ = config
                                        .tx
                                        .send(AppMessage::VideoAutoCompleted {
                                            video_id: config.video_id.clone(),
                                        })
                                        .await;
                                }

                                if last_ui_tick.elapsed() >= Duration::from_millis(900) {
                                    last_ui_tick = std::time::Instant::now();
                                    let _ = config
                                        .tx
                                        .send(AppMessage::PlaybackProgressUpdated {
                                            video_id: config.video_id.clone(),
                                            pos: last_pos,
                                            duration: last_dur,
                                        })
                                        .await;
                                }

                                if last_save.elapsed() >= Duration::from_secs(4) && last_pos > 1.0 {
                                    last_save = std::time::Instant::now();
                                    let _ = db::user::save_playback_progress(
                                        &config.pool,
                                        &config.video_id,
                                        last_pos,
                                        last_dur,
                                        now_unix(),
                                    )
                                    .await;
                                }

                                if last_session_save.elapsed() >= Duration::from_secs(10) {
                                    last_session_save = std::time::Instant::now();
                                    tracker
                                        .flush(&config, last_pos, last_dur, marked_watched)
                                        .await;
                                }
                            }
                            _ => {}
                        }
                    }

                    if let Some(ev) = parse_live_ipc_event(&line) {
                        match ev {
                            LiveIpcEvent::Paused(p) => tracker.is_paused = p,
                            LiveIpcEvent::Speed(s) => tracker.current_speed = s.clamp(0.25, 4.0),
                            _ => {}
                        }
                        let _ = config
                            .tx
                            .send(AppMessage::PlaybackTelemetryUpdated {
                                session_id: config.session_id,
                                event: ev,
                            })
                            .await;
                    }
                    if let Some((cache_ahead_secs, seekable_end_secs)) =
                        parse_live_cache_telemetry(&line)
                    {
                        let _ = config
                            .tx
                            .send(AppMessage::PlaybackTelemetryUpdated {
                                session_id: config.session_id,
                                event: LiveIpcEvent::LiveCacheTelemetry {
                                    cache_ahead_secs,
                                    seekable_end_secs,
                                },
                            })
                            .await;
                    }
                }
            }
        }

        // Persist final position and session telemetry when mpv closes
        if !config.is_live && last_pos > 3.0 {
            let _ = db::user::save_playback_progress(
                &config.pool,
                &config.video_id,
                last_pos,
                last_dur,
                now_unix(),
            )
            .await;
        }
        tracker
            .flush(
                &config,
                last_pos,
                last_dur,
                marked_watched || (!config.is_live && is_completion_threshold_reached(last_pos, last_dur)),
            )
            .await;

        #[cfg(unix)]
        let _ = std::fs::remove_file(&config.sock_path);

        let _ = config.tx.send(AppMessage::PlaybackDone(config.session_id)).await;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_property_parse_skip_and_completion() {
        let line = r#"{"event":"property-change","id":1,"name":"time-pos","data":45.25}"#;
        let parsed = parse_property_change(line).unwrap();
        assert_eq!(parsed.0, "time-pos");
        assert!((parsed.1 - 45.25).abs() < 0.001);

        let segs = vec![SponsorSegment {
            uuid: "seg-1".to_string(),
            category: "sponsor".to_string(),
            action_type: "skip".to_string(),
            start_time: 40.0,
            end_time: 75.0,
            video_duration: 600.0,
        }];
        let mut skipped = HashSet::new();
        assert_eq!(
            check_skip_interval_with_duration(45.25, 600.0, &segs, &skipped).map(|s| s.uuid.as_str()),
            Some("seg-1")
        );
        skipped.insert("seg-1".to_string());
        assert!(check_skip_interval_with_duration(45.25, 600.0, &segs, &skipped).is_none());

        // End-of-video SponsorBlock outro/selfpromo near EOF must NOT trigger premature exit
        let ending_segs = vec![SponsorSegment {
            uuid: "seg-outro".to_string(),
            category: "outro".to_string(),
            action_type: "skip".to_string(),
            start_time: 585.0,
            end_time: 599.5,
            video_duration: 600.0,
        }];
        assert!(check_skip_interval_with_duration(586.0, 600.0, &ending_segs, &HashSet::new()).is_none());

        let track_json = serde_json::json!([
            {"id": 1, "type": "video", "selected": true},
            {"id": 1, "type": "audio", "lang": "it", "title": "Italian, medium", "selected": false},
            {"id": 2, "type": "audio", "lang": "en-US", "title": "English (US) original (default), medium", "selected": true, "default": true},
            {"id": 3, "type": "audio", "lang": "ar", "title": "Arabic - dubbed-auto", "selected": false},
            {"id": 4, "type": "sub", "lang": "en", "title": "English CC", "selected": true}
        ]);
        let (aud, subs) = parse_track_list(&track_json);
        assert_eq!(aud.len(), 3);
        assert!(aud[0].is_dubbed);
        assert!(!aud[0].is_original);
        assert!(aud[1].is_original);
        assert!(!aud[1].is_dubbed);
        assert!(aud[2].is_dubbed);
        assert_eq!(subs.len(), 1);

        let cmds = PlayerCommand::SetAudioTrack(2).to_mpv_json_commands();
        assert!(cmds[0].contains("\"aid\""));
        let shader_cmds = PlayerCommand::SetShaderMode(
            VideoShaderMode::Off,
            VideoMasteringProfile::StandardHd,
            PerformanceMode::Balanced,
        )
        .to_mpv_json_commands();
        assert!(shader_cmds[0].contains("\"clr\""));

        let eco_cmds = PlayerCommand::SetPerformanceMode(
            PerformanceMode::Efficient,
            VideoShaderMode::Reference4K,
            VideoMasteringProfile::StandardHd,
        )
        .to_mpv_json_commands();
        assert!(eco_cmds.iter().any(|c| c.contains("\"48MiB\"")));
        assert!(eco_cmds.iter().any(|c| c.contains("00_light_cas.glsl")));

        let mut custom = ManualMasteringConfig::from_preset(PerformanceMode::Balanced);
        custom.hdr_compute_peak = false;
        custom.tone_map = crate::config::ToneMapMode::St2094_40;
        let man_cmds = PlayerCommand::ApplyManualMastering(
            custom,
            VideoShaderMode::Reference4K,
            VideoMasteringProfile::StandardHd,
        )
        .to_mpv_json_commands();
        assert!(man_cmds.iter().any(|c| c.contains("\"st2094-40\"")));
        assert!(man_cmds.iter().any(|c| c.contains("\"hdr-compute-peak\",\"no\"")));

        let cache_line = r#"{"event":"property-change","id":19,"name":"demuxer-cache-state","data":{"fw-bytes":52428800,"cache-duration":6.25,"seekable-ranges":[{"start":120.0,"end":485.5}]}}"#;
        assert!(matches!(parse_live_ipc_event(cache_line), Some(LiveIpcEvent::CacheBytes(52428800))));
        let (ahead, seek_end) = parse_live_cache_telemetry(cache_line).unwrap();
        assert!((ahead - 6.25).abs() < 0.01);
        assert!((seek_end - 485.5).abs() < 0.01);
        let jump = PlayerCommand::JumpToLiveEdge.to_mpv_json_commands();
        assert!(jump[0].contains("phaenna-snap-live-edge"));

        assert!(!is_completion_threshold_reached(40.0, 45.0));
        assert!(!is_completion_threshold_reached(80.0, 100.0));
        assert!(is_completion_threshold_reached(86.0, 100.0));
    }

    #[test]
    fn test_buffer_telemetry_and_abr_hot_switch_commands() {
        let pfc_line = r#"{"event":"property-change","id":21,"name":"paused-for-cache","data":true}"#;
        assert!(matches!(
            parse_live_ipc_event(pfc_line),
            Some(LiveIpcEvent::PausedForCache(true))
        ));

        let spd_line = r#"{"event":"property-change","id":22,"name":"cache-speed","data":524288.4}"#;
        assert!(matches!(
            parse_live_ipc_event(spd_line),
            Some(LiveIpcEvent::CacheSpeed(524288))
        ));

        let pct_line = r#"{"event":"property-change","id":23,"name":"cache-buffering-state","data":67.8}"#;
        assert!(matches!(
            parse_live_ipc_event(pct_line),
            Some(LiveIpcEvent::BufferingPct(68))
        ));

        let net_cmds = PlayerCommand::SetNetworkMode(NetworkMode::CongestedWifi).to_mpv_json_commands();
        assert!(net_cmds.iter().any(|c| c.contains("\"cache-pause-wait\",4.5")));
        assert!(net_cmds.iter().any(|c| c.contains("\"demuxer-readahead-secs\",120")));
        assert!(net_cmds.iter().any(|c| c.contains("\"network-timeout\",12")));

        let sw_cmds = PlayerCommand::HotSwitchStream {
            primary_url: "https://cdn.example.com/v480.webm".to_string(),
            audio_url: Some("https://cdn.example.com/a96.webm".to_string()),
            user_agent: Some("PhaennaTest/1.0".to_string()),
            resume_pos: 142.35,
            osd_message: "Auto-ABR -> 480p30".to_string(),
        }
        .to_mpv_json_commands();
        assert!(sw_cmds.iter().any(|c| c.contains("\"start\",\"142.35\"")));
        assert!(sw_cmds.iter().any(|c| c.contains("\"audio-files\",[\"https://cdn.example.com/a96.webm\"]")));
        assert!(sw_cmds.iter().any(|c| c.contains("\"loadfile\",\"https://cdn.example.com/v480.webm\",\"replace\"")));
        assert!(is_file_loaded_event(r#"{"event":"file-loaded"}"#));
        assert!(!is_file_loaded_event(r#"{"event":"end-file"}"#));

        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":24,"name":"estimated-vf-fps","data":59.94}"#),
            Some(LiveIpcEvent::VfFps(f)) if (f - 59.94).abs() < 0.01
        ));
        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":25,"name":"video-bitrate","data":12800000.0}"#),
            Some(LiveIpcEvent::VideoBitrateBps(b)) if (b - 12_800_000.0).abs() < 1.0
        ));
        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":27,"name":"avsync","data":-0.0042}"#),
            Some(LiveIpcEvent::AvSyncSecs(s)) if (s + 0.0042).abs() < 0.0001
        ));
        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":28,"name":"hwdec-current","data":"videotoolbox"}"#),
            Some(LiveIpcEvent::HwdecCurrent(ref s)) if s == "videotoolbox"
        ));
        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":29,"name":"video-codec","data":"av1"}"#),
            Some(LiveIpcEvent::VideoCodec(ref s)) if s == "av1"
        ));
        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":32,"name":"video-params/colormatrix","data":"bt.709"}"#),
            Some(LiveIpcEvent::ColorMatrix(ref s)) if s == "bt.709"
        ));
        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":34,"name":"vo-delayed-frame-count","data":2}"#),
            Some(LiveIpcEvent::DelayedFrames(2))
        ));
        assert!(matches!(
            parse_live_ipc_event(r#"{"event":"property-change","id":36,"name":"audio-params/hr-channels","data":"stereo"}"#),
            Some(LiveIpcEvent::AudioChannels(ref s)) if s == "stereo"
        ));
        assert!(parse_live_ipc_event(r#"{"event":"property-change","id":28,"name":"hwdec-current","data":null}"#).is_none());
    }
}
