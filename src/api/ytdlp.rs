//! Cancellation-safe `yt-dlp` JSON runner, feed pagination, detail extractor, and chapter parser.

use crate::api::innertube::{parse_iso8601_unix, parse_relative_age_to_unix, parse_rough_views};
use crate::api::player::find_binary;
use crate::api::subtitles::{parse_ytdlp_subtitles_json, SubtitleTrack};
use crate::config::{CodecPriority, DubPreference, NetworkMode, Quality};
use crate::db::cache::{VideoChapter, VideoItem};
use crate::messages::AppMessage;
use crate::services::subsync::parse_timestamp;
use serde_json::Value;
use std::cmp::Ordering;
use std::path::Path;
use std::process::Stdio;
use std::time::{SystemTime, UNIX_EPOCH};
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

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct ResolvedVideoVariant {
    pub format_id: String,
    pub url: String,
    pub height: u16,
    pub fps: f32,
    pub tbr_kbps: f64,
    pub vcodec: String,
    pub has_audio: bool,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct ResolvedAudioVariant {
    pub format_id: String,
    pub url: String,
    pub abr_kbps: f64,
    pub acodec: String,
    pub lang: Option<String>,
    pub format_note: String,
    pub language_preference: i64,
    pub is_dubbed: bool,
    pub is_original: bool,
    pub is_drc: bool,
    pub user_agent: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct SelectedStreamPair {
    pub primary_url: String,
    pub audio_url: Option<String>,
    pub height: u16,
    pub fps: f32,
    pub tbr_kbps: f64,
    pub vcodec: String,
    pub user_agent: Option<String>,
    pub sub_url: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct ResolvedStreamLadder {
    pub video_id: String,
    pub resolved_at_unix: i64,
    pub video_variants: Vec<ResolvedVideoVariant>,
    pub audio_variants: Vec<ResolvedAudioVariant>,
    pub preferred_sub_url: Option<String>,
}

/// Extract the Unix `expire=` timestamp query parameter from a signed CDN stream URL.
pub fn parse_url_expire_unix(url: &str) -> Option<i64> {
    for pat in ["?expire=", "&expire="] {
        if let Some(pos) = url.find(pat) {
            let rest = &url[pos + pat.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() {
                return digits.parse::<i64>().ok();
            }
        }
    }
    None
}

fn vcodec_rank(vcodec: &str, codec: CodecPriority, congested: bool) -> u8 {
    let vc = vcodec.to_lowercase();
    let is_av1 = vc.starts_with("av01");
    let is_vp9 = vc.starts_with("vp9") || vc.starts_with("vp09");
    let is_hev = vc.starts_with("hev") || vc.starts_with("hvc");
    let is_avc = vc.starts_with("avc") || vc.starts_with("h264");

    if congested || codec.resolve_for_host() == CodecPriority::Av1First {
        if is_av1 {
            4
        } else if is_vp9 {
            3
        } else if is_hev {
            2
        } else if is_avc {
            1
        } else {
            0
        }
    } else if is_vp9 {
        4
    } else if is_hev {
        3
    } else if is_avc {
        2
    } else if is_av1 {
        1
    } else {
        0
    }
}

fn compare_kbps_pref(a: f64, b: f64, prefer_lower: bool) -> Ordering {
    if prefer_lower {
        let a_eff = if a > 0.0 { a } else { f64::MAX };
        let b_eff = if b > 0.0 { b } else { f64::MAX };
        b_eff.total_cmp(&a_eff)
    } else {
        a.total_cmp(&b)
    }
}

impl ResolvedStreamLadder {
    pub fn is_fresh(&self, now_unix: i64) -> bool {
        if now_unix.saturating_sub(self.resolved_at_unix) > 3 * 3600 {
            return false;
        }
        let sample_urls = self
            .video_variants
            .first()
            .map(|v| v.url.as_str())
            .into_iter()
            .chain(self.audio_variants.first().map(|a| a.url.as_str()));
        for url in sample_urls {
            if let Some(exp) = parse_url_expire_unix(url) {
                if exp <= now_unix + 600 {
                    return false;
                }
            }
        }
        true
    }

    fn pick_audio(
        &self,
        dub: DubPreference,
        dub_lang: &str,
        max_abr_kbps: Option<u16>,
    ) -> Option<&ResolvedAudioVariant> {
        if self.audio_variants.is_empty() {
            return None;
        }
        let pref_lang = dub_lang.trim().to_lowercase();
        let role_score = |a: &ResolvedAudioVariant| -> u8 {
            match dub {
                DubPreference::OriginalOnly => {
                    if a.is_original && !a.is_dubbed && !a.is_drc {
                        5
                    } else if a.is_original && !a.is_dubbed {
                        4
                    } else if !a.is_dubbed && a.language_preference >= 0 && !a.is_drc {
                        3
                    } else if !a.is_dubbed && !a.is_drc {
                        2
                    } else if !a.is_dubbed {
                        1
                    } else {
                        0
                    }
                }
                DubPreference::PreferDubbed => {
                    let lang_match = !pref_lang.is_empty()
                        && a.lang
                            .as_deref()
                            .map(|l| l.to_lowercase().starts_with(&pref_lang))
                            .unwrap_or(false);
                    if a.is_dubbed && lang_match {
                        4
                    } else if lang_match && !a.is_original {
                        3
                    } else if a.is_dubbed {
                        2
                    } else if lang_match {
                        1
                    } else {
                        0
                    }
                }
            }
        };

        let best_role = self.audio_variants.iter().map(role_score).max().unwrap_or(0);
        let candidates: Vec<&ResolvedAudioVariant> = self
            .audio_variants
            .iter()
            .filter(|a| role_score(a) == best_role)
            .collect();

        if let Some(cap) = max_abr_kbps {
            let ceiling = cap as f64 + 10.0;
            let within_cap: Vec<&ResolvedAudioVariant> = candidates
                .iter()
                .copied()
                .filter(|a| a.abr_kbps > 0.0 && a.abr_kbps <= ceiling)
                .collect();
            if !within_cap.is_empty() {
                return within_cap
                    .into_iter()
                    .max_by(|a, b| a.abr_kbps.total_cmp(&b.abr_kbps));
            }
            return candidates
                .into_iter()
                .min_by(|a, b| a.abr_kbps.total_cmp(&b.abr_kbps));
        }

        candidates
            .into_iter()
            .max_by(|a, b| a.abr_kbps.total_cmp(&b.abr_kbps))
    }

    fn build_pair(&self, v: &ResolvedVideoVariant, audio: Option<&ResolvedAudioVariant>) -> SelectedStreamPair {
        let separate_audio = if v.has_audio { None } else { audio };
        let audio_abr = separate_audio.map(|a| a.abr_kbps).unwrap_or(0.0);
        SelectedStreamPair {
            primary_url: v.url.clone(),
            audio_url: separate_audio.map(|a| a.url.clone()),
            height: v.height,
            fps: v.fps,
            tbr_kbps: v.tbr_kbps + audio_abr,
            vcodec: v.vcodec.clone(),
            user_agent: v
                .user_agent
                .clone()
                .or_else(|| separate_audio.and_then(|a| a.user_agent.clone())),
            sub_url: self.preferred_sub_url.clone(),
        }
    }

    pub fn select_best_pair(
        &self,
        quality: Quality,
        net: NetworkMode,
        codec: CodecPriority,
        dub: DubPreference,
        dub_lang: &str,
    ) -> Option<SelectedStreamPair> {
        let picked_audio = self.pick_audio(dub, dub_lang, net.max_audio_abr_kbps());
        if quality == Quality::AudioOnly {
            let a = picked_audio?;
            return Some(SelectedStreamPair {
                primary_url: a.url.clone(),
                audio_url: None,
                height: 0,
                fps: 0.0,
                tbr_kbps: a.abr_kbps,
                vcodec: a.acodec.clone(),
                user_agent: a.user_agent.clone(),
                sub_url: self.preferred_sub_url.clone(),
            });
        }

        if self.video_variants.is_empty() {
            return None;
        }

        let quality_max_h = quality.max_height_or_default(4320);
        let max_h = match net.max_height_cap() {
            Some(cap) => quality_max_h.min(cap),
            None => quality_max_h,
        };
        let mut pool: Vec<&ResolvedVideoVariant> = self
            .video_variants
            .iter()
            .filter(|v| v.height <= max_h)
            .collect();
        if pool.is_empty() {
            let min_h = self.video_variants.iter().map(|v| v.height).min()?;
            pool = self
                .video_variants
                .iter()
                .filter(|v| v.height == min_h)
                .collect();
        }

        if let Some(max_fps) = net.max_fps_cap() {
            let fps_pool: Vec<&ResolvedVideoVariant> = pool
                .iter()
                .copied()
                .filter(|v| v.fps <= (max_fps as f32) + 0.5)
                .collect();
            if !fps_pool.is_empty() {
                pool = fps_pool;
            }
        }

        if picked_audio.is_none() {
            let muxed: Vec<&ResolvedVideoVariant> =
                pool.iter().copied().filter(|v| v.has_audio).collect();
            if !muxed.is_empty() {
                pool = muxed;
            }
        }

        let congested = net.is_congested();
        let best_video = pool.into_iter().max_by(|a, b| {
            let a_dash = u8::from(!a.has_audio && picked_audio.is_some());
            let b_dash = u8::from(!b.has_audio && picked_audio.is_some());
            let a_fps_bucket = if congested {
                u8::from(a.fps <= 30.5)
            } else {
                u8::from(a.fps >= 49.0)
            };
            let b_fps_bucket = if congested {
                u8::from(b.fps <= 30.5)
            } else {
                u8::from(b.fps >= 49.0)
            };
            a.height
                .cmp(&b.height)
                .then_with(|| a_dash.cmp(&b_dash))
                .then_with(|| a_fps_bucket.cmp(&b_fps_bucket))
                .then_with(|| vcodec_rank(&a.vcodec, codec, congested).cmp(&vcodec_rank(&b.vcodec, codec, congested)))
                .then_with(|| compare_kbps_pref(a.tbr_kbps, b.tbr_kbps, congested))
        })?;

        Some(self.build_pair(best_video, picked_audio))
    }

    pub fn select_lower_tier(
        &self,
        current_height: u16,
        current_fps: f32,
        current_tbr_kbps: f64,
        codec: CodecPriority,
        dub: DubPreference,
        dub_lang: &str,
    ) -> Option<SelectedStreamPair> {
        if current_height == 0 || self.video_variants.is_empty() {
            return None;
        }
        let picked_audio = self.pick_audio(dub, dub_lang, Some(96));

        // Step 1: Try dropping from 60fps to <=30fps at the same resolution first.
        if current_fps > 40.0 {
            let same_h_30: Vec<&ResolvedVideoVariant> = self
                .video_variants
                .iter()
                .filter(|v| {
                    v.height == current_height
                        && v.fps <= 30.5
                        && (current_tbr_kbps <= 0.0 || v.tbr_kbps <= current_tbr_kbps * 0.85)
                        && (picked_audio.is_some() || v.has_audio)
                })
                .collect();
            if let Some(best_same) = same_h_30.into_iter().max_by(|a, b| {
                vcodec_rank(&a.vcodec, codec, true)
                    .cmp(&vcodec_rank(&b.vcodec, codec, true))
                    .then_with(|| compare_kbps_pref(a.tbr_kbps, b.tbr_kbps, true))
            }) {
                return Some(self.build_pair(best_same, picked_audio));
            }
        }

        // Step 2: Step down to the highest available resolution strictly below `current_height`.
        let mut lower: Vec<&ResolvedVideoVariant> = self
            .video_variants
            .iter()
            .filter(|v| v.height < current_height)
            .collect();
        if lower.is_empty() {
            return None;
        }
        if picked_audio.is_none() {
            let muxed: Vec<&ResolvedVideoVariant> =
                lower.iter().copied().filter(|v| v.has_audio).collect();
            if !muxed.is_empty() {
                lower = muxed;
            }
        }
        let target_h = lower.iter().map(|v| v.height).max()?;
        let mut tier: Vec<&ResolvedVideoVariant> =
            lower.into_iter().filter(|v| v.height == target_h).collect();
        let tier_30: Vec<&ResolvedVideoVariant> = tier
            .iter()
            .copied()
            .filter(|v| v.fps <= 30.5)
            .collect();
        if !tier_30.is_empty() {
            tier = tier_30;
        }

        let best = tier.into_iter().max_by(|a, b| {
            let a_dash = u8::from(!a.has_audio && picked_audio.is_some());
            let b_dash = u8::from(!b.has_audio && picked_audio.is_some());
            a_dash
                .cmp(&b_dash)
                .then_with(|| vcodec_rank(&a.vcodec, codec, true).cmp(&vcodec_rank(&b.vcodec, codec, true)))
                .then_with(|| compare_kbps_pref(a.tbr_kbps, b.tbr_kbps, true))
        })?;

        Some(self.build_pair(best, picked_audio))
    }
}

pub fn parse_stream_ladder_from_ytdlp_json(
    val: &Value,
    video_id: &str,
    now_unix: i64,
    subs: &[SubtitleTrack],
) -> Option<ResolvedStreamLadder> {
    if val.get("is_live").and_then(Value::as_bool) == Some(true)
        || val.get("live_status").and_then(Value::as_str) == Some("is_live")
    {
        return None;
    }
    let formats = val.get("formats").and_then(Value::as_array)?;
    let mut video_variants = Vec::new();
    let mut audio_variants = Vec::new();

    for fmt in formats {
        if fmt.get("has_drm").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let Some(url) = fmt.get("url").and_then(Value::as_str) else {
            continue;
        };
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        let proto = fmt
            .get("protocol")
            .and_then(Value::as_str)
            .unwrap_or("https");
        if !matches!(proto, "https" | "http" | "m3u8_native" | "m3u8") {
            continue;
        }

        let vcodec = fmt.get("vcodec").and_then(Value::as_str).unwrap_or("none");
        let acodec = fmt.get("acodec").and_then(Value::as_str).unwrap_or("none");
        let format_id = fmt
            .get("format_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let user_agent = fmt
            .get("http_headers")
            .and_then(|h| h.get("User-Agent"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        if vcodec != "none" {
            let height = fmt.get("height").and_then(Value::as_u64).unwrap_or(0) as u16;
            if height < 144 {
                continue;
            }
            let fps = fmt.get("fps").and_then(Value::as_f64).unwrap_or(30.0) as f32;
            let tbr_kbps = fmt
                .get("tbr")
                .or_else(|| fmt.get("vbr"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            video_variants.push(ResolvedVideoVariant {
                format_id,
                url: url.to_string(),
                height,
                fps,
                tbr_kbps,
                vcodec: vcodec.to_string(),
                has_audio: acodec != "none",
                user_agent,
            });
        } else if acodec != "none" {
            let abr_kbps = fmt
                .get("abr")
                .or_else(|| fmt.get("tbr"))
                .and_then(Value::as_f64)
                .unwrap_or(0.0);
            let lang = fmt
                .get("language")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let format_note = fmt
                .get("format_note")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let language_preference = fmt
                .get("language_preference")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let note_low = format_note.to_lowercase();
            let is_dubbed = note_low.contains("dub") || language_preference < 0;
            let is_original = !is_dubbed
                && (note_low.contains("original") || language_preference >= 10);
            let is_drc = note_low.contains("drc");
            audio_variants.push(ResolvedAudioVariant {
                format_id,
                url: url.to_string(),
                abr_kbps,
                acodec: acodec.to_string(),
                lang,
                format_note,
                language_preference,
                is_dubbed,
                is_original,
                is_drc,
                user_agent,
            });
        }
    }

    if video_variants.is_empty() && audio_variants.is_empty() {
        return None;
    }

    Some(ResolvedStreamLadder {
        video_id: video_id.to_string(),
        resolved_at_unix: now_unix,
        video_variants,
        audio_variants,
        preferred_sub_url: subs.first().and_then(|s| s.url.clone()),
    })
}

/// Fetch full metadata (`yt-dlp -J --skip-download --no-playlist`) including chapters, tags, subtitles, and resolved CDN stream ladder.
pub async fn fetch_video_details_with_ladder(
    cookies_path: Option<&Path>,
    video_id: &str,
    force_ipv4: bool,
) -> Option<(VideoItem, Vec<SubtitleTrack>, Option<ResolvedStreamLadder>)> {
    let clean = video_id.trim_start_matches("RD");
    let url = format!("https://www.youtube.com/watch?v={}", clean);

    let mut cmd = Command::new(ytdlp_bin());
    apply_cookies(&mut cmd, cookies_path);
    if force_ipv4 {
        cmd.arg("--force-ipv4");
    }
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
    let now_unix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let ladder = parse_stream_ladder_from_ytdlp_json(&val, clean, now_unix, &subs);
    Some((item, subs, ladder))
}

/// Fetch full metadata (`yt-dlp -J --skip-download --no-playlist`) including chapters, tags & subtitles.
#[allow(dead_code)]
pub async fn fetch_video_details(
    cookies_path: Option<&Path>,
    video_id: &str,
) -> Option<(VideoItem, Vec<SubtitleTrack>)> {
    fetch_video_details_with_ladder(cookies_path, video_id, false)
        .await
        .map(|(item, subs, _)| (item, subs))
}

/// Pre-resolve stream format ladder for direct playback and automatic ABR downshift.
pub async fn resolve_stream_ladder(
    cookies_path: Option<&Path>,
    video_id: &str,
    force_ipv4: bool,
) -> Option<ResolvedStreamLadder> {
    fetch_video_details_with_ladder(cookies_path, video_id, force_ipv4)
        .await
        .and_then(|(_, _, ladder)| ladder)
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

    #[test]
    fn test_stream_ladder_parse_freshness_selection_and_downshift() {
        let now = 1_700_000_000i64;
        let json: Value = serde_json::json!({
            "id": "vid123",
            "is_live": false,
            "formats": [
                {
                    "format_id": "sb0",
                    "protocol": "mhtml",
                    "url": "https://i.ytimg.com/sb/vid123.jpg",
                    "vcodec": "none",
                    "acodec": "none"
                },
                {
                    "format_id": "140-drc",
                    "protocol": "https",
                    "url": "https://cdn.example.com/audio140drc?expire=1700010000&itag=140",
                    "acodec": "mp4a.40.2",
                    "vcodec": "none",
                    "abr": 129.0,
                    "language": "en",
                    "format_note": "English original, DRC",
                    "language_preference": 10
                },
                {
                    "format_id": "251",
                    "protocol": "https",
                    "url": "https://cdn.example.com/audio251?expire=1700010000&itag=251",
                    "acodec": "opus",
                    "vcodec": "none",
                    "abr": 132.0,
                    "language": "en",
                    "format_note": "English original",
                    "language_preference": 10,
                    "http_headers": { "User-Agent": "PhaennaTest/1.0" }
                },
                {
                    "format_id": "249",
                    "protocol": "https",
                    "url": "https://cdn.example.com/audio249?expire=1700010000&itag=249",
                    "acodec": "opus",
                    "vcodec": "none",
                    "abr": 56.0,
                    "language": "en",
                    "format_note": "English original",
                    "language_preference": 10
                },
                {
                    "format_id": "251-dub",
                    "protocol": "https",
                    "url": "https://cdn.example.com/audiodub?expire=1700010000",
                    "acodec": "opus",
                    "vcodec": "none",
                    "abr": 135.0,
                    "language": "ar",
                    "format_note": "Arabic dubbed-auto",
                    "language_preference": -1
                },
                {
                    "format_id": "299",
                    "protocol": "https",
                    "url": "https://cdn.example.com/v1080p60?expire=1700010000",
                    "vcodec": "avc1.64002a",
                    "acodec": "none",
                    "height": 1080,
                    "fps": 60.0,
                    "tbr": 4200.0
                },
                {
                    "format_id": "399",
                    "protocol": "https",
                    "url": "https://cdn.example.com/v1080p30av1?expire=1700010000",
                    "vcodec": "av01.0.08M.08",
                    "acodec": "none",
                    "height": 1080,
                    "fps": 30.0,
                    "tbr": 1600.0
                },
                {
                    "format_id": "247",
                    "protocol": "https",
                    "url": "https://cdn.example.com/v720p30vp9?expire=1700010000",
                    "vcodec": "vp9",
                    "acodec": "none",
                    "height": 720,
                    "fps": 30.0,
                    "tbr": 950.0
                },
                {
                    "format_id": "397",
                    "protocol": "https",
                    "url": "https://cdn.example.com/v480p30av1?expire=1700010000",
                    "vcodec": "av01.0.04M.08",
                    "acodec": "none",
                    "height": 480,
                    "fps": 30.0,
                    "tbr": 480.0
                },
                {
                    "format_id": "18",
                    "protocol": "https",
                    "url": "https://cdn.example.com/v360pmux?expire=1700010000",
                    "vcodec": "avc1.42001E",
                    "acodec": "mp4a.40.2",
                    "height": 360,
                    "fps": 30.0,
                    "tbr": 380.0
                }
            ]
        });

        let ladder = parse_stream_ladder_from_ytdlp_json(&json, "vid123", now, &[])
            .expect("ladder parsed");
        assert!(ladder.is_fresh(now + 100));
        assert!(!ladder.is_fresh(now + 4 * 3600));
        assert!(!ladder.is_fresh(1_700_009_500)); // within 600s of expire=1700010000

        // Normal mode picks 1080p60 when Quality::Best + Vp9HwFirst
        let norm = ladder
            .select_best_pair(
                Quality::Best,
                NetworkMode::Normal,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "ar",
            )
            .unwrap();
        assert_eq!(norm.height, 1080);
        assert!((norm.fps - 60.0).abs() < 0.1);
        assert!(norm.audio_url.as_deref().unwrap().contains("audio251"));

        // Congested Wi-Fi caps to 720p30 and non-dubbed original audio <= 128(+10)kbps
        let wifi = ladder
            .select_best_pair(
                Quality::Best,
                NetworkMode::CongestedWifi,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "ar",
            )
            .unwrap();
        assert_eq!(wifi.height, 720);
        assert!(wifi.fps <= 30.5);

        // Ultra Saver caps to 480p30 and <= 72kbps audio (audio249 at 56kbps)
        let ultra = ladder
            .select_best_pair(
                Quality::Best,
                NetworkMode::UltraSaver,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "ar",
            )
            .unwrap();
        assert_eq!(ultra.height, 480);
        assert!(ultra.audio_url.as_deref().unwrap().contains("audio249"));

        // AudioOnly returns audio URL directly with no separate audio_url
        let audio_only = ladder
            .select_best_pair(
                Quality::AudioOnly,
                NetworkMode::CongestedWifi,
                CodecPriority::Vp9HwFirst,
                DubPreference::PreferDubbed,
                "ar",
            )
            .unwrap();
        assert_eq!(audio_only.height, 0);
        assert!(audio_only.audio_url.is_none());
        assert!(audio_only.primary_url.contains("audiodub"));

        // Downshift chain: 1080p60 -> 1080p30 -> 720p30 -> 480p30 -> 360p30 (muxed)
        let s1 = ladder
            .select_lower_tier(
                1080,
                60.0,
                4200.0,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "en",
            )
            .unwrap();
        assert_eq!(s1.height, 1080);
        assert!(s1.fps <= 30.5);

        let s2 = ladder
            .select_lower_tier(
                s1.height,
                s1.fps,
                s1.tbr_kbps,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "en",
            )
            .unwrap();
        assert_eq!(s2.height, 720);

        let s3 = ladder
            .select_lower_tier(
                s2.height,
                s2.fps,
                s2.tbr_kbps,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "en",
            )
            .unwrap();
        assert_eq!(s3.height, 480);

        let s4 = ladder
            .select_lower_tier(
                s3.height,
                s3.fps,
                s3.tbr_kbps,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "en",
            )
            .unwrap();
        assert_eq!(s4.height, 360);
        assert!(s4.audio_url.is_none()); // muxed format 18 has embedded audio

        assert!(ladder
            .select_lower_tier(
                s4.height,
                s4.fps,
                s4.tbr_kbps,
                CodecPriority::Vp9HwFirst,
                DubPreference::OriginalOnly,
                "en",
            )
            .is_none());
    }
}
