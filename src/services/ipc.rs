//! Non-blocking mpv JSON-RPC IPC socket monitor.
//! Tracks live `time-pos` & `duration`, persists playback resume state every 4s,
//! triggers 85% completion watch logging, and performs automatic SponsorBlock segment skipping.

use crate::api::player::LOUDNORM_FILTER;
use crate::api::sponsorblock::{category_label, SponsorSegment};
use crate::config::VideoShaderMode;
use crate::db;
use crate::messages::AppMessage;
use crate::services::shaders::{prepare_video_shaders, VideoMasteringProfile};
use serde_json::{json, Value};
use sqlx::SqlitePool;
use std::collections::HashSet;
#[cfg(unix)]
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::{self, Sender};

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
    pub sock_path: String,
    pub pool: SqlitePool,
    pub tx: Sender<AppMessage>,
    pub skip_segments: Vec<SponsorSegment>,
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
        let extra = self
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| format!(" - {t}"))
            .unwrap_or_default();
        format!("[{}] {}{}{}", self.id, lang, role, extra)
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
    SetShaderMode(VideoShaderMode, VideoMasteringProfile),
    SetPictureProp { prop: &'static str, value: i64 },
    ResetPicture,
    SetOnTop(bool),
    SetLoop(bool),
    Stop,
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
            Self::SetShaderMode(mode, _profile) => {
                let mut cmds = Vec::new();
                if let Some(shaders) = prepare_video_shaders(*mode) {
                    cmds.push(
                        json!({"command": ["change-list", "glsl-shaders", "set", shaders]})
                            .to_string(),
                    );
                    cmds.push(json!({"command": ["set_property", "deband", "yes"]}).to_string());
                    cmds.push(
                        json!({"command": ["set_property", "tone-mapping", "spline"]}).to_string(),
                    );
                } else {
                    cmds.push(
                        json!({"command": ["change-list", "glsl-shaders", "clr", ""]})
                            .to_string(),
                    );
                    cmds.push(json!({"command": ["set_property", "deband", "no"]}).to_string());
                }
                cmds
            }
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
            Self::Stop => vec![json!({"command": ["quit"]}).to_string()],
        }
    }
}

/// Parse `mpv` JSON `track-list` array into `(audio_tracks, sub_tracks)`.
pub fn parse_track_list(val: &Value) -> (Vec<LiveMediaTrack>, Vec<LiveMediaTrack>) {
    let Some(arr) = val.as_array() else {
        return (Vec::new(), Vec::new());
    };
    let mut audio = Vec::new();
    let mut sub = Vec::new();
    for item in arr {
        let Some(kind_s) = item.get("type").and_then(Value::as_str) else {
            continue;
        };
        let kind = match kind_s {
            "audio" => TrackKind::Audio,
            "sub" => TrackKind::Sub,
            _ => continue,
        };
        let Some(id) = item.get("id").and_then(Value::as_i64) else {
            continue;
        };
        let lang = item.get("lang").and_then(Value::as_str).map(ToOwned::to_owned);
        let title = item.get("title").and_then(Value::as_str).map(ToOwned::to_owned);
        let selected = item.get("selected").and_then(Value::as_bool).unwrap_or(false);
        let title_low = title.as_deref().unwrap_or("").to_lowercase();
        let is_dubbed = title_low.contains("dub") || title_low.contains("auto-dub");
        let is_original = kind == TrackKind::Audio
            && !is_dubbed
            && (title_low.contains("original")
                || item.get("default").and_then(Value::as_bool).unwrap_or(false)
                || audio.is_empty());

        let track = LiveMediaTrack {
            id,
            kind,
            lang,
            title,
            is_dubbed,
            is_original,
            selected,
        };
        match kind {
            TrackKind::Audio => audio.push(track),
            TrackKind::Sub => sub.push(track),
        }
    }
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
        "track-list" => {
            let (audio, sub) = parse_track_list(data);
            Some(LiveIpcEvent::Tracks { audio, sub })
        }
        _ => None,
    }
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
        ];
        for (id, prop) in observe_props {
            let msg = format!("{{\"command\":[\"observe_property\",{id},\"{prop}\"]}}\n");
            let _ = writer.write_all(msg.as_bytes()).await;
        }

        let mut lines = BufReader::new(reader).lines();
        let mut last_pos = 0.0f64;
        let mut last_dur = 0.0f64;
        let mut last_save = std::time::Instant::now();
        let mut skipped_uuids: HashSet<String> = HashSet::new();
        let mut marked_watched = false;

        loop {
            tokio::select! {
                maybe_cmd = cmd_rx.recv() => {
                    match maybe_cmd {
                        Some(cmd) => {
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

                    if let Some((name, val)) = parse_property_change(&line) {
                        match name.as_str() {
                            "duration" => {
                                if val > 0.0 {
                                    last_dur = val;
                                }
                            }
                            "time-pos" => {
                                last_pos = val;

                                if let Some(seg) = check_skip_interval_with_duration(
                                    val,
                                    last_dur,
                                    &config.skip_segments,
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

                                if !marked_watched && is_completion_threshold_reached(last_pos, last_dur) {
                                    marked_watched = true;
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
                                    let _ = config
                                        .tx
                                        .send(AppMessage::PlaybackProgressUpdated {
                                            video_id: config.video_id.clone(),
                                            pos: last_pos,
                                            duration: last_dur,
                                        })
                                        .await;
                                }
                            }
                            _ => {}
                        }
                    }

                    if let Some(ev) = parse_live_ipc_event(&line) {
                        let _ = config
                            .tx
                            .send(AppMessage::PlaybackTelemetryUpdated {
                                session_id: config.session_id,
                                event: ev,
                            })
                            .await;
                    }
                }
            }
        }

        // Persist final position when mpv closes
        if last_pos > 3.0 {
            let _ = db::user::save_playback_progress(
                &config.pool,
                &config.video_id,
                last_pos,
                last_dur,
                now_unix(),
            )
            .await;
        }

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
            {"id": 1, "type": "audio", "lang": "en", "title": "English original", "selected": true, "default": true},
            {"id": 2, "type": "audio", "lang": "ar", "title": "Arabic - dubbed-auto", "selected": false},
            {"id": 3, "type": "sub", "lang": "en", "title": "English CC", "selected": true}
        ]);
        let (aud, subs) = parse_track_list(&track_json);
        assert_eq!(aud.len(), 2);
        assert!(aud[0].is_original);
        assert!(aud[1].is_dubbed);
        assert_eq!(subs.len(), 1);

        let cmds = PlayerCommand::SetAudioTrack(2).to_mpv_json_commands();
        assert!(cmds[0].contains("\"aid\""));
        let shader_cmds = PlayerCommand::SetShaderMode(
            VideoShaderMode::Off,
            VideoMasteringProfile::StandardHd,
        )
        .to_mpv_json_commands();
        assert!(shader_cmds[0].contains("\"clr\""));

        assert!(!is_completion_threshold_reached(40.0, 45.0));
        assert!(!is_completion_threshold_reached(80.0, 100.0));
        assert!(is_completion_threshold_reached(86.0, 100.0));
    }
}
