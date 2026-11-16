//! Cancellation-safe `yt-dlp` JSON runner, feed pagination, detail extractor, and chapter parser.

use crate::api::innertube::{parse_iso8601_unix, parse_relative_age_to_unix, parse_rough_views};
use crate::api::player::find_binary;
use crate::api::subtitles::{parse_ytdlp_subtitles_json, SubtitleTrack};
use crate::db::cache::{VideoChapter, VideoItem};
use crate::messages::AppMessage;
use crate::services::subsync::parse_timestamp;
use serde_json::Value;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio::sync::mpsc::Sender;

fn parse_ytdlp_json_line(line: &str) -> Option<VideoItem> {
    let val: Value = serde_json::from_str(line).ok()?;
    let item: VideoItem = serde_json::from_value(val.clone()).ok()?;
    Some(enrich_ytdlp_dates(&val, item))
}

fn enrich_ytdlp_dates(val: &Value, mut item: VideoItem) -> VideoItem {
    if item.timestamp.is_none() {
        item.timestamp = val
            .get("release_timestamp")
            .and_then(|v| v.as_i64())
            .or_else(|| val.get("approximate_date").and_then(|v| v.as_i64()));
    }
    if item.timestamp.is_none() {
        let ud_opt = val
            .get("upload_date")
            .or_else(|| val.get("release_date"))
            .and_then(|v| v.as_str());
        if let Some(ud) = ud_opt {
            if ud.len() == 8 && ud.chars().all(|c| c.is_ascii_digit()) {
                let iso = format!("{}-{}-{}T12:00:00Z", &ud[0..4], &ud[4..6], &ud[6..8]);
                if let Some(ts) = parse_iso8601_unix(&iso) {
                    item.timestamp = Some(ts as i64);
                }
            }
        }
    }
    if item.published_text.is_none() {
        if let Some(rel) = val
            .get("published_time_text")
            .or_else(|| val.get("published_text"))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            item.published_text = Some(rel.to_string());
        } else if let Some(ud) = val
            .get("upload_date")
            .or_else(|| val.get("release_date"))
            .and_then(|v| v.as_str())
        {
            if ud.len() == 8 && ud.chars().all(|c| c.is_ascii_digit()) {
                item.published_text = Some(format!("{}-{}-{}", &ud[0..4], &ud[4..6], &ud[6..8]));
            }
        }
    }
    if item.timestamp.is_none() {
        if let Some(ref pub_txt) = item.published_text {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;
            item.timestamp = parse_relative_age_to_unix(pub_txt, now);
        }
    }
    if item.view_count.is_none() {
        item.view_count = val
            .get("view_count_text")
            .and_then(|v| v.as_str())
            .and_then(parse_rough_views);
    }
    if item.duration.is_none() && item.duration_string.is_none() {
        let is_short_url = item
            .webpage_url
            .as_deref()
            .map(|u| u.contains("/shorts/"))
            .unwrap_or(false)
            || val
                .get("url")
                .and_then(|v| v.as_str())
                .map(|u| u.contains("/shorts/"))
                .unwrap_or(false);
        if is_short_url {
            item.duration = Some(60);
            item.duration_string = Some("SHORT".to_string());
        }
    }
    item
}

fn ytdlp_bin() -> String {
    find_binary("yt-dlp")
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "yt-dlp".to_string())
}

fn apply_cookies(cmd: &mut Command, cookies_path: Option<&Path>) {
    if let Some(p) = cookies_path {
        if p.exists() && std::fs::metadata(p).map(|m| m.len() > 20).unwrap_or(false) {
            cmd.arg("--cookies").arg(p);
        }
    }
}

pub async fn fetch_feed_range(
    cookies_path: Option<&Path>,
    feed_url: &str,
    start: usize,
    end: usize,
) -> Vec<VideoItem> {
    let mut cmd = Command::new(ytdlp_bin());
    apply_cookies(&mut cmd, cookies_path);
    cmd.args([
        "--extractor-args",
        "youtubetab:skip=authcheck;approximate_date",
        "--flat-playlist",
        "--ignore-errors",
        "-j",
        "--playlist-start",
        &start.to_string(),
        "--playlist-end",
        &end.to_string(),
        feed_url,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null());

    let Ok(out) = cmd.output().await else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().filter_map(parse_ytdlp_json_line).collect()
}

pub async fn fetch_feed_range_stream(
    cookies_path: Option<&Path>,
    feed_url: String,
    feed_key: String,
    start: usize,
    end: usize,
    tx: Sender<AppMessage>,
) {
    let total = end.saturating_sub(start) + 1;
    let mut cmd = Command::new(ytdlp_bin());
    apply_cookies(&mut cmd, cookies_path);
    cmd.args([
        "--extractor-args",
        "youtubetab:skip=authcheck;approximate_date",
        "--flat-playlist",
        "--ignore-errors",
        "-j",
        "--playlist-start",
        &start.to_string(),
        "--playlist-end",
        &end.to_string(),
        &feed_url,
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null());

    let Ok(mut child) = cmd.spawn() else {
        let _ = tx
            .send(AppMessage::MoreFeedLoaded {
                feed_key,
                items: Vec::new(),
                continuation: None,
            })
            .await;
        return;
    };

    let mut items = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        let mut lines = BufReader::new(stdout).lines();
        let mut count = 0usize;
        while let Ok(Some(line)) = lines.next_line().await {
            if let Some(item) = parse_ytdlp_json_line(&line) {
                count += 1;
                items.push(item);
                let _ = tx
                    .send(AppMessage::FetchProgress {
                        feed_key: feed_key.clone(),
                        current: count,
                        total,
                    })
                    .await;
            }
        }
    }

    let _ = child.wait().await;
    let _ = tx
        .send(AppMessage::MoreFeedLoaded {
            feed_key,
            items,
            continuation: None,
        })
        .await;
}

pub async fn search_youtube(
    cookies_path: Option<&Path>,
    query: &str,
    limit: usize,
) -> Vec<VideoItem> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let mut cmd = Command::new(ytdlp_bin());
    apply_cookies(&mut cmd, cookies_path);
    cmd.args([
        "--extractor-args",
        "youtubetab:skip=authcheck;approximate_date",
        "--flat-playlist",
        "--ignore-errors",
        "-j",
        &format!("ytsearch{}:{}", limit, trimmed),
    ])
    .stdout(Stdio::piped())
    .stderr(Stdio::null());

    let Ok(out) = cmd.output().await else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    text.lines().filter_map(parse_ytdlp_json_line).collect()
}

pub async fn fetch_channel_videos(
    cookies_path: Option<&Path>,
    channel_id_or_handle: &str,
    limit: usize,
) -> Vec<VideoItem> {
    let target = if channel_id_or_handle.starts_with("http") {
        channel_id_or_handle.to_string()
    } else if channel_id_or_handle.starts_with('@') {
        format!("https://www.youtube.com/{}/videos", channel_id_or_handle)
    } else {
        format!(
            "https://www.youtube.com/channel/{}/videos",
            channel_id_or_handle
        )
    };
    fetch_feed_range(cookies_path, &target, 1, limit.max(1)).await
}

/// Fetch full metadata (`yt-dlp -J --skip-download --no-playlist`) including chapters, tags & subtitles.
pub async fn fetch_video_details(
    cookies_path: Option<&Path>,
    video_id: &str,
) -> Option<(VideoItem, Vec<SubtitleTrack>)> {
    let clean = video_id.trim_start_matches("RD");
    let url = format!("https://www.youtube.com/watch?v={}", clean);

    let mut cmd = Command::new(ytdlp_bin());
    apply_cookies(&mut cmd, cookies_path);
    cmd.args(["-J", "--no-playlist", "--skip-download", &url])
        .stdout(Stdio::piped())
        .stderr(Stdio::null());

    let out = cmd.output().await.ok()?;
    if !out.status.success() {
        return None;
    }

    let val: Value = serde_json::from_slice(&out.stdout).ok()?;
    let item: VideoItem = serde_json::from_value(val.clone()).ok()?;
    let mut item = enrich_ytdlp_dates(&val, item);

    // Fallback: extract chapters from description text if YouTube returned empty native chapters
    let has_chapters = item
        .chapters
        .as_ref()
        .map(|c| !c.is_empty())
        .unwrap_or(false);
    if !has_chapters {
        if let Some(ref desc) = item.description {
            let parsed = parse_description_chapters(desc, item.duration.unwrap_or(0) as f64);
            if !parsed.is_empty() {
                item.chapters = Some(parsed);
            }
        }
    }

    let subs = parse_ytdlp_subtitles_json(&val);
    Some((item, subs))
}

/// Parse timestamped chapter markers from video descriptions (e.g. `00:00 Intro`, `[1:05:22] Deep Dive`).
pub fn parse_description_chapters(description: &str, total_duration: f64) -> Vec<VideoChapter> {
    let mut markers: Vec<(f64, String)> = Vec::new();

    for line in description.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Check leading token `[HH:MM:SS]` or `MM:SS`
        let (ts_opt, rest_str) = if let Some(after_bracket) = trimmed.strip_prefix('[') {
            if let Some(close_idx) = after_bracket.find(']') {
                (
                    parse_timestamp(&after_bracket[..close_idx]),
                    after_bracket[close_idx + 1..].trim(),
                )
            } else {
                (None, "")
            }
        } else if let Some((first_tok, rest)) = trimmed.split_once(char::is_whitespace) {
            let clean_tok = first_tok.trim_matches(|c: char| !c.is_ascii_digit() && c != ':');
            if clean_tok.contains(':') {
                (parse_timestamp(clean_tok), rest.trim())
            } else {
                (None, "")
            }
        } else {
            (None, "")
        };

        if let Some(sec) = ts_opt {
            let title = rest_str
                .trim_start_matches(['-', '–', '—', ':', '|', ' '])
                .trim()
                .to_string();
            if !title.is_empty() {
                markers.push((sec, title));
            }
        }
    }

    if markers.len() < 2 {
        return Vec::new();
    }

    markers.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    markers.dedup_by(|a, b| (a.0 - b.0).abs() < 0.5);

    let mut chapters = Vec::with_capacity(markers.len());
    for i in 0..markers.len() {
        let start = markers[i].0;
        let end = if i + 1 < markers.len() {
            markers[i + 1].0
        } else if total_duration > start {
            total_duration
        } else {
            start + 60.0
        };
        chapters.push(VideoChapter {
            title: markers[i].1.clone(),
            start_time: start,
            end_time: end,
        });
    }
    chapters
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_description_chapters() {
        let desc = "\
Check out the sections below:
00:00 Intro & Setup
[02:15] - Architecture Deep Dive
1:01:30 - Final Benchmarks
";
        let chapters = parse_description_chapters(desc, 4000.0);
        assert_eq!(chapters.len(), 3);
        assert_eq!(chapters[0].title, "Intro & Setup");
        assert!((chapters[0].start_time - 0.0).abs() < 0.01);
        assert!((chapters[0].end_time - 135.0).abs() < 0.01);
        assert_eq!(chapters[1].title, "Architecture Deep Dive");
        assert_eq!(chapters[2].title, "Final Benchmarks");
        assert!((chapters[2].start_time - 3690.0).abs() < 0.01);
        assert!((chapters[2].end_time - 4000.0).abs() < 0.01);
    }
}
