//! Background Offline Vault Downloader.
//! Downloads & muxes videos into `~/Movies/Phaenna/{Channel}/{Title}.mp4` via `yt-dlp`.

use crate::api::player::{find_binary, youtube_watch_url};
use crate::db;
use crate::messages::AppMessage;
use sqlx::SqlitePool;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Clone)]
pub struct DownloadTask {
    pub video_id: String,
    pub title: String,
    pub channel_name: String,
    pub quality_format: String,
    pub cookies_path: Option<PathBuf>,
    pub force_ipv4: bool,
    pub concurrent_fragments: u8,
    pub rate_limit: Option<String>,
}

/// Parse yt-dlp `[download]  42.3% of ... at  3.25MiB/s ETA 00:12` output.
pub fn parse_yt_dlp_progress(line: &str) -> Option<(f32, String)> {
    if !line.contains("[download]") {
        return None;
    }
    let perc_idx = line.find('%')?;
    let before_perc = line[..perc_idx].trim_end();
    let num_start = before_perc
        .rfind(|c: char| !c.is_numeric() && c != '.')
        .map(|i| i + 1)
        .unwrap_or(0);
    let percent: f32 = before_perc[num_start..].parse().ok()?;

    let speed = if let Some(at_idx) = line.find(" at ") {
        let after_at = line[at_idx + 4..].trim_start();
        if let Some(space_idx) = after_at.find(" ETA") {
            after_at[..space_idx].trim().to_string()
        } else if let Some(slash_idx) = after_at.find("/s") {
            after_at[..slash_idx + 2].trim().to_string()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    Some((percent.clamp(0.0, 100.0), speed))
}

pub fn sanitize_filename(name: &str) -> String {
    let clean: String = name
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let trimmed = clean.trim().trim_matches('.');
    if trimmed.is_empty() {
        "Video".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

pub fn get_vault_dir(channel_name: &str) -> PathBuf {
    let base = directories::UserDirs::new()
        .and_then(|u| u.video_dir().map(|p| p.to_path_buf()))
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|h| h.join("Movies"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Phaenna").join(sanitize_filename(channel_name))
}

pub fn spawn_download_task(task: DownloadTask, pool: SqlitePool, tx: Sender<AppMessage>) {
    tokio::spawn(async move {
        let dir = get_vault_dir(&task.channel_name);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            let _ = tx
                .send(AppMessage::DownloadFailed {
                    video_id: task.video_id,
                    error: format!("Cannot create directory: {}", e),
                })
                .await;
            return;
        }

        let safe_title = sanitize_filename(&task.title);
        let out_path = dir.join(format!("{} [{}].mp4", safe_title, task.video_id));
        let bin = find_binary("yt-dlp")
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| "yt-dlp".to_string());

        let mut cmd = Command::new(bin);
        if let Some(ref cpath) = task.cookies_path {
            if cpath.exists() {
                cmd.arg("--cookies").arg(cpath);
            }
        }
        cmd.args([
            "--newline",
            "--retries",
            "10",
            "--fragment-retries",
            "10",
            "--file-access-retries",
            "5",
        ]);
        if task.concurrent_fragments > 1 {
            cmd.arg("--concurrent-fragments")
                .arg(task.concurrent_fragments.to_string());
        }
        if task.force_ipv4 {
            cmd.arg("--force-ipv4");
        }
        if let Some(ref lim) = task.rate_limit {
            cmd.arg("--limit-rate").arg(lim);
        }
        cmd.args([
            "-f",
            &task.quality_format,
            "--merge-output-format",
            "mp4",
            "--embed-chapters",
            "-o",
            &out_path.to_string_lossy(),
            &youtube_watch_url(&task.video_id),
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

        let Ok(mut child) = cmd.spawn() else {
            let _ = tx
                .send(AppMessage::DownloadFailed {
                    video_id: task.video_id,
                    error: "yt-dlp not found".to_string(),
                })
                .await;
            return;
        };

        if let Some(stdout) = child.stdout.take() {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some((pct, speed)) = parse_yt_dlp_progress(&line) {
                    let _ = tx
                        .send(AppMessage::DownloadProgress {
                            video_id: task.video_id.clone(),
                            percent: pct,
                            speed,
                        })
                        .await;
                }
            }
        }

        match child.wait().await {
            Ok(status) if status.success() && out_path.exists() => {
                let bytes = std::fs::metadata(&out_path)
                    .map(|m| m.len())
                    .unwrap_or(0);
                let path_str = out_path.to_string_lossy().into_owned();
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs() as i64;
                let _ = db::user::save_offline_video(&pool, &task.video_id, &path_str, bytes, "mp4", now).await;
                let _ = tx
                    .send(AppMessage::DownloadComplete {
                        video_id: task.video_id,
                        file_path: path_str,
                    })
                    .await;
            }
            Ok(status) => {
                let _ = tx
                    .send(AppMessage::DownloadFailed {
                        video_id: task.video_id,
                        error: format!("yt-dlp exited with {}", status),
                    })
                    .await;
            }
            Err(e) => {
                let _ = tx
                    .send(AppMessage::DownloadFailed {
                        video_id: task.video_id,
                        error: e.to_string(),
                    })
                    .await;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_progress_parse_and_filename_sanitize() {
        let line = "[download]  47.8% of ~ 125.40MiB at  6.12MiB/s ETA 00:11";
        let (pct, spd) = parse_yt_dlp_progress(line).unwrap();
        assert!((pct - 47.8).abs() < 0.01);
        assert_eq!(spd, "6.12MiB/s");

        assert_eq!(sanitize_filename("Rust / Async: Deep? Dive*"), "Rust _ Async_ Deep_ Dive_");
    }
}
