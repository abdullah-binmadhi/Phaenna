//! Non-blocking mpv JSON-RPC IPC socket monitor.
//! Tracks live `time-pos` & `duration`, persists playback resume state every 4s,
//! triggers 85% completion watch logging, and performs automatic SponsorBlock segment skipping.

use crate::api::sponsorblock::{category_label, SponsorSegment};
use crate::db;
use crate::messages::AppMessage;
use serde_json::Value;
use sqlx::SqlitePool;
use std::collections::HashSet;
#[cfg(unix)]
use std::path::Path;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::mpsc::Sender;

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

/// Check if playback `pos` falls inside any unskipped SponsorBlock segment window.
pub fn check_skip_interval<'a>(
    pos: f64,
    segments: &'a [SponsorSegment],
    skipped_uuids: &HashSet<String>,
) -> Option<&'a SponsorSegment> {
    for seg in segments {
        if skipped_uuids.contains(&seg.uuid) {
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
        let _ = writer
            .write_all(b"{\"command\":[\"observe_property\",1,\"time-pos\"]}\n")
            .await;
        let _ = writer
            .write_all(b"{\"command\":[\"observe_property\",2,\"duration\"]}\n")
            .await;

        let mut lines = BufReader::new(reader).lines();
        let mut last_pos = 0.0f64;
        let mut last_dur = 0.0f64;
        let mut last_save = std::time::Instant::now();
        let mut skipped_uuids: HashSet<String> = HashSet::new();
        let mut marked_watched = false;

        while let Ok(Some(line)) = lines.next_line().await {
            if let Some((name, val)) = parse_property_change(&line) {
                match name.as_str() {
                    "duration" => {
                        if val > 0.0 {
                            last_dur = val;
                        }
                    }
                    "time-pos" => {
                        last_pos = val;

                        if let Some(seg) =
                            check_skip_interval(val, &config.skip_segments, &skipped_uuids)
                        {
                            skipped_uuids.insert(seg.uuid.clone());
                            let seek_cmd = format!(
                                "{{\"command\":[\"set_property\",\"time-pos\",{:.2}]}}\n",
                                seg.end_time
                            );
                            if writer.write_all(seek_cmd.as_bytes()).await.is_ok() {
                                let saved = (seg.end_time - seg.start_time).max(0.0);
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
            check_skip_interval(45.25, &segs, &skipped).map(|s| s.uuid.as_str()),
            Some("seg-1")
        );
        skipped.insert("seg-1".to_string());
        assert!(check_skip_interval(45.25, &segs, &skipped).is_none());

        assert!(!is_completion_threshold_reached(40.0, 45.0));
        assert!(!is_completion_threshold_reached(80.0, 100.0));
        assert!(is_completion_threshold_reached(86.0, 100.0));
    }
}
