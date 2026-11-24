//! Fast HTTP/RSS client for zero-subprocess YouTube channel feed refreshes.
//! Fetches public Atom XML feeds in <150ms for the 7-day Upload Radar & channel checks.

use crate::db::cache::{is_valid_yt_video_id, normalize_thumb_url, Thumbnail, VideoChapter, VideoItem};
use crate::error::{AppError, Result};
use crate::services::auth::{compute_sapisidhash, now_unix_secs, GoogleAccountSession, PHAENNA_USER_AGENT};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, USER_AGENT};
use std::collections::HashSet;

const INNERTUBE_CLIENT_VERSION: &str = "2.20251015.01.00";

/// Map Phaenna internal feed keys onto YouTube InnerTube `browseId` targets.
pub fn browse_id_for_feed_key(feed_key: &str) -> Option<&'static str> {
    match feed_key {
        "recommended" | "shorts_suggested" => Some("FEwhat_to_watch"),
        "subscriptions" => Some("FEsubscriptions"),
        "history" => Some("FEhistory"),
        "watch_later" => Some("VLWL"),
        _ => None,
    }
}

/// Extracted InnerTube shelf page containing deduplicated videos, resume progress, and optional next continuation token.
#[derive(Debug, Clone, Default)]
pub struct InnerTubeShelfPage {
    pub videos: Vec<VideoItem>,
    pub continuation: Option<String>,
    pub watched_progress: Vec<(String, u8)>,
}

/// Spotlight channel metadata extracted from YouTube InnerTube search `channelRenderer`.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct SearchChannelCard {
    pub channel_id: String,
    pub title: String,
    pub handle: Option<String>,
    pub subscriber_text: Option<String>,
    pub video_count_text: Option<String>,
    pub description: Option<String>,
    pub verified: bool,
    pub avatar_url: Option<String>,
}

/// Structured InnerTube `/youtubei/v1/search` page preserving native `yt_rank` order and isolating Shorts & noise shelves.
#[derive(Debug, Clone, Default)]
pub struct InnerTubeSearchPage {
    pub videos: Vec<VideoItem>,
    pub shorts: Vec<VideoItem>,
    pub channels: Vec<SearchChannelCard>,
    pub did_you_mean: Option<String>,
    pub estimated_results: Option<u64>,
    pub continuation: Option<String>,
}

/// Apply either isolated `SAPISIDHASH` + cookie headers or OAuth `Bearer` authorization to an InnerTube request.
pub fn apply_innertube_auth_headers(
    mut req: reqwest::RequestBuilder,
    session: &GoogleAccountSession,
) -> reqwest::RequestBuilder {
    req = req
        .header(USER_AGENT, PHAENNA_USER_AGENT)
        .header(CONTENT_TYPE, "application/json")
        .header("Origin", "https://www.youtube.com")
        .header("X-Origin", "https://www.youtube.com")
        .header("X-YouTube-Client-Name", "1")
        .header("X-YouTube-Client-Version", INNERTUBE_CLIENT_VERSION);

    if let (Some(sapisid), Some(cookie_hdr)) = (session.sapisid(), session.cookie_header()) {
        let hash = compute_sapisidhash(sapisid, "https://www.youtube.com", now_unix_secs());
        req = req
            .header(AUTHORIZATION, format!("SAPISIDHASH {hash}"))
            .header(COOKIE, cookie_hdr);
    } else if let Some(ref token) = session.access_token {
        req = req.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    req
}

/// Fetch a personalized YouTube shelf directly over HTTP/2 InnerTube (`youtubei/v1/browse`).
pub async fn fetch_innertube_shelf(
    client: &reqwest::Client,
    session: &GoogleAccountSession,
    browse_id: &str,
    continuation: Option<&str>,
) -> Result<InnerTubeShelfPage> {
    let mut payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        }
    });

    if let Some(cont) = continuation.filter(|c| !c.is_empty()) {
        payload["continuation"] = serde_json::Value::String(cont.to_string());
    } else {
        payload["browseId"] = serde_json::Value::String(browse_id.to_string());
    }

    let req = client
        .post("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(12))
        .json(&payload);

    let resp = apply_innertube_auth_headers(req, session).send().await?;
    let status = resp.status();
    if !status.is_success() {
        return Err(AppError::Http(format!("InnerTube browse HTTP {status}")));
    }

    let root: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(format!("Invalid InnerTube JSON: {e}")))?;

    Ok(parse_innertube_browse_json(&root))
}

/// Sync a video to/from the user's real YouTube Watch Later playlist (`VLWL`).
pub async fn innertube_edit_watch_later(
    client: &reqwest::Client,
    session: &GoogleAccountSession,
    video_id: &str,
    add: bool,
) -> Result<()> {
    if session.sapisid().is_some() {
        let action = if add {
            serde_json::json!({
                "action": "ACTION_ADD_VIDEO",
                "addedVideoId": video_id
            })
        } else {
            serde_json::json!({
                "action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID",
                "removedVideoId": video_id
            })
        };

        let payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": INNERTUBE_CLIENT_VERSION
                }
            },
            "playlistId": "WL",
            "actions": [action]
        });

        let req = client
            .post("https://www.youtube.com/youtubei/v1/browse/edit_playlist")
            .timeout(std::time::Duration::from_secs(8))
            .json(&payload);
        let resp = apply_innertube_auth_headers(req, session).send().await?;
        if !resp.status().is_success() {
            return Err(AppError::Http(format!(
                "Watch Later edit HTTP {}",
                resp.status()
            )));
        }
        return Ok(());
    }

    if add {
        if let Some(ref token) = session.access_token {
            let body = serde_json::json!({
                "snippet": {
                    "playlistId": "WL",
                    "resourceId": {
                        "kind": "youtube#video",
                        "videoId": video_id
                    }
                }
            });
            let resp = client
                .post("https://www.googleapis.com/youtube/v3/playlistItems?part=snippet")
                .timeout(std::time::Duration::from_secs(8))
                .header(AUTHORIZATION, format!("Bearer {token}"))
                .json(&body)
                .send()
                .await?;
            if !resp.status().is_success() {
                return Err(AppError::Http(format!(
                    "YouTube Data API Watch Later HTTP {}",
                    resp.status()
                )));
            }
        }
    }

    Ok(())
}

/// Recursively extract normalized `VideoItem`s, resume overlays, and pagination token from any InnerTube browse JSON payload.
pub fn parse_innertube_browse_json(root: &serde_json::Value) -> InnerTubeShelfPage {
    let mut page = InnerTubeShelfPage::default();
    let mut seen = HashSet::new();
    collect_innertube_nodes(
        root,
        &mut page.videos,
        &mut page.watched_progress,
        &mut seen,
        &mut page.continuation,
    );
    page
}

fn extract_renderer_watched_pct(renderer: &serde_json::Value) -> Option<u8> {
    let overlays = renderer.get("thumbnailOverlays")?.as_array()?;
    for ov in overlays {
        if let Some(pct) = ov
            .pointer("/thumbnailOverlayResumePlaybackRenderer/percentDurationWatched")
            .and_then(|v| v.as_u64())
        {
            if pct > 0 {
                return Some(pct.min(100) as u8);
            }
        }
    }
    None
}

fn extract_lockup_watched_pct(lockup: &serde_json::Value) -> Option<u8> {
    let overlays = lockup
        .pointer("/contentImage/thumbnailViewModel/overlays")?
        .as_array()?;
    for ov in overlays {
        let pct_val = ov
            .pointer("/thumbnailOverlayProgressBarViewModel/startPercent")
            .or_else(|| {
                ov.pointer(
                    "/thumbnailBottomOverlayViewModel/progressBar/thumbnailOverlayProgressBarViewModel/startPercent",
                )
            });
        if let Some(pct_f) = pct_val.and_then(|v| v.as_f64().or_else(|| v.as_u64().map(|n| n as f64))) {
            if pct_f > 0.0 {
                return Some(pct_f.round().clamp(1.0, 100.0) as u8);
            }
        }
    }
    None
}

fn collect_innertube_nodes(
    node: &serde_json::Value,
    out: &mut Vec<VideoItem>,
    watched_progress: &mut Vec<(String, u8)>,
    seen: &mut HashSet<String>,
    continuation: &mut Option<String>,
) {
    match node {
        serde_json::Value::Object(map) => {
            for key in [
                "videoRenderer",
                "gridVideoRenderer",
                "playlistVideoRenderer",
                "compactVideoRenderer",
            ] {
                if let Some(renderer) = map.get(key) {
                    if let Some(item) = extract_video_from_renderer(renderer) {
                        if seen.insert(item.id.clone()) {
                            if let Some(pct) = extract_renderer_watched_pct(renderer) {
                                watched_progress.push((item.id.clone(), pct));
                            }
                            out.push(item);
                        }
                    }
                }
            }
            if let Some(lockup) = map.get("lockupViewModel") {
                if let Some(item) = extract_video_from_lockup(lockup) {
                    if seen.insert(item.id.clone()) {
                        if let Some(pct) = extract_lockup_watched_pct(lockup) {
                            watched_progress.push((item.id.clone(), pct));
                        }
                        out.push(item);
                    }
                }
            }
            if let Some(sl) = map.get("shortsLockupViewModel") {
                if let Some(item) = extract_video_from_shorts_lockup(sl) {
                    if seen.insert(item.id.clone()) {
                        out.push(item);
                    }
                }
            }
            if let Some(ri) = map.get("reelItemRenderer") {
                if let Some(item) = extract_video_from_reel_item(ri) {
                    if seen.insert(item.id.clone()) {
                        out.push(item);
                    }
                }
            }

            if continuation.is_none() {
                if let Some(cont_item) = map.get("continuationItemRenderer") {
                    if let Some(tok) = cont_item
                        .pointer("/continuationEndpoint/continuationCommand/token")
                        .and_then(|v| v.as_str())
                    {
                        *continuation = Some(tok.to_string());
                    }
                }
            }

            for val in map.values() {
                collect_innertube_nodes(val, out, watched_progress, seen, continuation);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_innertube_nodes(item, out, watched_progress, seen, continuation);
            }
        }
        _ => {}
    }
}

fn needs_join_space(prev: &str, next: &str) -> bool {
    let Some(lc) = prev.chars().last() else {
        return false;
    };
    let Some(fc) = next.chars().next() else {
        return false;
    };
    if lc.is_whitespace() || fc.is_whitespace() {
        return false;
    }
    if matches!(fc, ',' | '.' | '!' | '?' | ':' | ';' | ')' | ']' | '}')
        || matches!(lc, '(' | '[' | '{' | '/' | '@')
    {
        return false;
    }
    (lc.is_alphanumeric() || matches!(lc, ')' | ']' | '%' | '"' | '\''))
        && (fc.is_alphanumeric() || matches!(fc, '@' | '#' | '(' | '['))
}

fn strip_zero_width(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '\u{200B}' | '\u{FEFF}' | '\u{200E}' | '\u{200F}'))
        .collect()
}

fn extract_yt_text(val: Option<&serde_json::Value>) -> Option<String> {
    let obj = val?;
    if let Some(s) = obj.get("simpleText").and_then(|v| v.as_str()) {
        return Some(strip_zero_width(s));
    }
    if let Some(runs) = obj.get("runs").and_then(|v| v.as_array()) {
        let mut joined = String::new();
        for r in runs {
            if let Some(seg_raw) = r.get("text").and_then(|t| t.as_str()) {
                let seg = strip_zero_width(seg_raw);
                if seg.is_empty() {
                    continue;
                }
                if needs_join_space(&joined, &seg) {
                    joined.push(' ');
                }
                joined.push_str(&seg);
            }
        }
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

fn parse_duration_text(raw: &str) -> Option<u64> {
    let parts: Vec<u64> = raw
        .trim()
        .split(':')
        .map(|p| p.trim().parse::<u64>().ok())
        .collect::<Option<Vec<_>>>()?;
    match parts.as_slice() {
        [m, s] => Some(m * 60 + s),
        [h, m, s] => Some(h * 3600 + m * 60 + s),
        _ => None,
    }
}

pub fn parse_rough_views(raw: &str) -> Option<u64> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() {
        return None;
    }
    if lower.contains("no views") {
        return Some(0);
    }

    let mut num_buf = String::new();
    let mut suffix_char: Option<char> = None;
    let mut started = false;
    for ch in lower.chars() {
        if ch.is_ascii_digit() || (started && (ch == '.' || ch == ',')) {
            started = true;
            if ch != ',' {
                num_buf.push(ch);
            }
        } else if started {
            if matches!(ch, 'k' | 'm' | 'b') {
                suffix_char = Some(ch);
            }
            break;
        }
    }

    if num_buf.is_empty() {
        return None;
    }
    let val = num_buf.parse::<f64>().ok()?;
    let scale = match suffix_char {
        Some('k') => 1_000.0,
        Some('m') => 1_000_000.0,
        Some('b') => 1_000_000_000.0,
        _ if lower.contains("billion") => 1_000_000_000.0,
        _ if lower.contains("million") => 1_000_000.0,
        _ if lower.contains("thousand") => 1_000.0,
        _ => 1.0,
    };
    Some((val * scale).round() as u64)
}

/// Parse English YouTube `dateText` like `"Nov 4, 2026"` or `"Streamed live on Nov 4, 2026"` -> Unix seconds.
pub fn parse_english_short_date_to_unix(raw: &str) -> Option<i64> {
    let trimmed = raw
        .trim()
        .trim_start_matches("Premiered ")
        .trim_start_matches("Streamed live on ")
        .trim_start_matches("Published on ");
    let parts: Vec<&str> = trimmed.split_whitespace().collect();
    if parts.len() < 3 {
        return None;
    }
    let month: u32 = match parts[0].to_ascii_lowercase().as_str() {
        "jan" | "january" => 1,
        "feb" | "february" => 2,
        "mar" | "march" => 3,
        "apr" | "april" => 4,
        "may" => 5,
        "jun" | "june" => 6,
        "jul" | "july" => 7,
        "aug" | "august" => 8,
        "sep" | "sept" | "september" => 9,
        "oct" | "october" => 10,
        "nov" | "november" => 11,
        "dec" | "december" => 12,
        _ => return None,
    };
    let day: u32 = parts[1].trim_end_matches(',').parse().ok()?;
    let year: i64 = parts[2].trim_end_matches(',').parse().ok()?;
    let iso = format!("{year:04}-{month:02}-{day:02}T12:00:00Z");
    parse_iso8601_unix(&iso).map(|u| u as i64)
}

pub fn parse_relative_age_to_unix(raw: &str, now_unix: i64) -> Option<i64> {
    let lower = raw.trim().to_ascii_lowercase();
    if lower.is_empty() || lower == "new" {
        return None;
    }
    if lower.contains("just now") || lower.contains("moments ago") {
        return Some(now_unix);
    }
    let stripped = lower
        .trim_start_matches("streamed ")
        .trim_start_matches("premiered ")
        .trim_start_matches("scheduled for ");
    let mut n_opt: Option<i64> = None;
    let mut unit: Option<&str> = None;
    for tok in stripped.split_whitespace() {
        if n_opt.is_none() {
            if let Ok(n) = tok.parse::<i64>() {
                n_opt = Some(n);
                continue;
            }
        } else {
            unit = Some(tok);
            break;
        }
    }
    let n = n_opt?;
    let unit = unit?;
    let secs = if unit.starts_with("second") || unit.starts_with("sec") {
        n
    } else if unit.starts_with("minute") || unit.starts_with("min") {
        n * 60
    } else if unit.starts_with("hour") || unit.starts_with("hr") {
        n * 3_600
    } else if unit.starts_with("day") {
        n * 86_400
    } else if unit.starts_with("week") || unit.starts_with("wk") {
        n * 7 * 86_400
    } else if unit.starts_with("month") || unit.starts_with("mo") {
        n * 30 * 86_400
    } else if unit.starts_with("year") || unit.starts_with("yr") {
        n * 365 * 86_400
    } else {
        return None;
    };
    Some(now_unix.saturating_sub(secs))
}

fn extract_video_from_renderer(r: &serde_json::Value) -> Option<VideoItem> {
    let id = r.get("videoId").and_then(|v| v.as_str())?.to_string();
    if id.is_empty() {
        return None;
    }
    let title = extract_yt_text(r.get("title")).unwrap_or_else(|| "Untitled Video".to_string());

    let channel_node = r
        .get("ownerText")
        .or_else(|| r.get("shortBylineText"))
        .or_else(|| r.get("longBylineText"));
    let uploader = extract_yt_text(channel_node);
    let channel_id = channel_node
        .and_then(|n| n.pointer("/runs/0/navigationEndpoint/browseEndpoint/browseId"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let duration_string_raw = extract_yt_text(r.get("lengthText"));
    let duration = duration_string_raw.as_deref().and_then(parse_duration_text);
    let is_reel = r.pointer("/navigationEndpoint/reelWatchEndpoint").is_some();
    let duration_string = if duration_string_raw.is_none() && is_reel {
        Some("SHORT".to_string())
    } else {
        duration_string_raw
    };

    let view_count = extract_yt_text(r.get("viewCountText"))
        .or_else(|| extract_yt_text(r.get("shortViewCountText")))
        .and_then(|s| parse_rough_views(&s));

    let badges = r.get("badges").and_then(|v| v.as_array());
    let is_live = badges
        .map(|arr| {
            arr.iter().any(|b| {
                b.pointer("/metadataBadgeRenderer/style")
                    .and_then(|s| s.as_str())
                    == Some("BADGE_STYLE_TYPE_LIVE_NOW")
            })
        })
        .filter(|&b| b);
    let has_new_badge = badges
        .map(|arr| {
            arr.iter().any(|b| {
                b.pointer("/metadataBadgeRenderer/label")
                    .and_then(|s| s.as_str())
                    .map(|l| l.eq_ignore_ascii_case("new"))
                    .unwrap_or(false)
            })
        })
        .unwrap_or(false);

    let channel_verified = r
        .get("ownerBadges")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|b| {
                let st = b
                    .pointer("/metadataBadgeRenderer/style")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                let tip = b
                    .pointer("/metadataBadgeRenderer/tooltip")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                st.contains("VERIFIED") || tip.eq_ignore_ascii_case("Verified")
            })
        })
        .unwrap_or(false);

    let search_snippet = r
        .pointer("/detailedMetadataSnippets/0/snippetText")
        .and_then(|v| extract_yt_text(Some(v)))
        .or_else(|| extract_yt_text(r.get("descriptionSnippet")))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let mut published_text = extract_yt_text(r.get("publishedTimeText"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if published_text.is_none() && has_new_badge {
        published_text = Some("New".to_string());
    }
    let timestamp = published_text
        .as_deref()
        .and_then(|s| parse_relative_age_to_unix(s, now_unix_secs()));

    let thumbnails = r
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let url = normalize_thumb_url(t.get("url")?.as_str()?);
                    if url.is_empty() {
                        return None;
                    }
                    let width = t.get("width").and_then(|w| w.as_u64()).map(|n| n as u32);
                    let height = t.get("height").and_then(|h| h.as_u64()).map(|n| n as u32);
                    Some(Thumbnail { url, width, height })
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());

    let webpage_url = if is_reel {
        Some(format!("https://www.youtube.com/shorts/{id}"))
    } else {
        Some(format!("https://www.youtube.com/watch?v={id}"))
    };

    Some(VideoItem {
        id,
        title,
        uploader: uploader.clone(),
        channel: uploader,
        channel_id,
        duration,
        duration_string,
        timestamp,
        published_text,
        view_count,
        webpage_url,
        thumbnails,
        is_live,
        search_snippet,
        channel_verified,
        ..Default::default()
    })
}

fn classify_metadata_segment(
    seg: &str,
    row_idx: usize,
    uploader: &mut Option<String>,
    view_count: &mut Option<u64>,
    published_text: &mut Option<String>,
) {
    let t = seg.trim();
    if t.is_empty() {
        return;
    }
    let lower = t.to_ascii_lowercase();
    if lower.contains("view") || lower.contains("watching") {
        if view_count.is_none() {
            *view_count = parse_rough_views(t);
        }
    } else if lower.contains("ago")
        || lower.contains("just now")
        || lower.contains("streamed")
        || lower.contains("premiered")
        || lower == "new"
    {
        if published_text.is_none() {
            *published_text = Some(t.to_string());
        }
    } else if row_idx == 0
        && uploader.is_none()
        && !lower.eq_ignore_ascii_case("cc")
        && !lower.contains("4k")
        && !lower.contains("hd")
    {
        *uploader = Some(t.to_string());
    }
}

fn extract_video_from_lockup(l: &serde_json::Value) -> Option<VideoItem> {
    if let Some(ct) = l.get("contentType").and_then(|v| v.as_str()) {
        if ct != "LOCKUP_CONTENT_TYPE_VIDEO" {
            return None;
        }
    }
    let id = l.get("contentId").and_then(|v| v.as_str())?.trim().to_string();
    if id.is_empty()
        || id.starts_with("PL")
        || id.starts_with("OLAK")
        || id.starts_with("UU")
        || id.starts_with("LL")
        || id.starts_with("WL")
        || id.starts_with("FL")
        || (id.starts_with("RD") && id.len() > 13)
    {
        return None;
    }
    let title = l
        .pointer("/metadata/lockupMetadataViewModel/title/content")
        .and_then(|v| v.as_str())
        .or_else(|| {
            l.pointer("/metadata/lockupMetadataViewModel/title/simpleText")
                .and_then(|v| v.as_str())
        })
        .unwrap_or("Untitled Video")
        .to_string();

    let rows = l
        .pointer("/metadata/lockupMetadataViewModel/metadata/contentMetadataViewModel/metadataRows")
        .and_then(|v| v.as_array());

    let mut uploader: Option<String> = l
        .pointer("/metadata/lockupMetadataViewModel/image/decoratedAvatarViewModel/a11yLabel")
        .and_then(|v| v.as_str())
        .and_then(|s| s.strip_prefix("Go to channel "))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let channel_id = l
        .pointer("/metadata/lockupMetadataViewModel/image/decoratedAvatarViewModel/rendererContext/commandContext/onTap/innertubeCommand/browseEndpoint/browseId")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let mut view_count: Option<u64> = None;
    let mut published_text: Option<String> = None;
    let mut has_new_badge = false;

    if let Some(rows_arr) = rows {
        for (idx, row) in rows_arr.iter().enumerate() {
            if let Some(badges) = row.get("badges").and_then(|b| b.as_array()) {
                for b in badges {
                    if let Some(bt) = b
                        .pointer("/badgeViewModel/badgeText")
                        .or_else(|| b.pointer("/metadataBadgeRenderer/label"))
                        .and_then(|s| s.as_str())
                    {
                        if bt.trim().eq_ignore_ascii_case("new") {
                            has_new_badge = true;
                        }
                    }
                }
            }
            let Some(parts) = row.get("metadataParts").and_then(|p| p.as_array()) else {
                continue;
            };
            for part in parts {
                let raw = part
                    .pointer("/text/content")
                    .and_then(|s| s.as_str())
                    .or_else(|| part.pointer("/text/accessibilityLabel").and_then(|s| s.as_str()));
                let Some(txt) = raw else {
                    continue;
                };
                for seg in txt.split(['•', '·', '\n']) {
                    classify_metadata_segment(
                        seg,
                        idx,
                        &mut uploader,
                        &mut view_count,
                        &mut published_text,
                    );
                }
            }
        }
    }

    if let Some(a11y) = l
        .pointer("/rendererContext/accessibilityContext/label")
        .or_else(|| l.pointer("/metadata/lockupMetadataViewModel/rendererContext/accessibilityContext/label"))
        .and_then(|v| v.as_str())
    {
        for seg in a11y.split(['•', '·', '-']) {
            classify_metadata_segment(
                seg,
                1,
                &mut uploader,
                &mut view_count,
                &mut published_text,
            );
        }
    }

    if published_text.is_none() && has_new_badge {
        published_text = Some("New".to_string());
    }

    let thumbnails = l
        .pointer("/contentImage/thumbnailViewModel/image/sources")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let url = normalize_thumb_url(t.get("url")?.as_str()?);
                    if url.is_empty() {
                        return None;
                    }
                    let width = t.get("width").and_then(|w| w.as_u64()).map(|n| n as u32);
                    let height = t.get("height").and_then(|h| h.as_u64()).map(|n| n as u32);
                    Some(Thumbnail { url, width, height })
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());

    let mut duration_string: Option<String> = None;
    let mut is_live: Option<bool> = None;
    if let Some(overlays) = l
        .pointer("/contentImage/thumbnailViewModel/overlays")
        .and_then(|v| v.as_array())
    {
        for ov in overlays {
            if let Some(txt) = ov
                .pointer("/thumbnailOverlayBadgeViewModel/thumbnailBadges/0/thumbnailBadgeViewModel/text")
                .and_then(|s| s.as_str())
                .or_else(|| ov.pointer("/thumbnailBottomOverlayViewModel/badges/0/thumbnailBadgeViewModel/text").and_then(|s| s.as_str()))
            {
                let trimmed = txt.trim();
                if trimmed.eq_ignore_ascii_case("LIVE") {
                    is_live = Some(true);
                    duration_string = Some("LIVE".to_string());
                } else if trimmed.eq_ignore_ascii_case("SHORTS") {
                    duration_string = Some("SHORT".to_string());
                } else if trimmed.contains(':') {
                    duration_string = Some(trimmed.to_string());
                }
            }
        }
    }
    let duration = duration_string.as_deref().and_then(parse_duration_text);
    let timestamp = published_text
        .as_deref()
        .and_then(|s| parse_relative_age_to_unix(s, now_unix_secs()));

    Some(VideoItem {
        id: id.clone(),
        title,
        uploader: uploader.clone(),
        channel: uploader,
        channel_id,
        duration,
        duration_string,
        timestamp,
        published_text,
        view_count,
        webpage_url: Some(format!("https://www.youtube.com/watch?v={id}")),
        thumbnails,
        is_live,
        ..Default::default()
    })
}

fn extract_video_from_shorts_lockup(sl: &serde_json::Value) -> Option<VideoItem> {
    let ep = sl.pointer("/onTap/innertubeCommand/reelWatchEndpoint");
    let id = ep
        .and_then(|e| e.get("videoId"))
        .and_then(|v| v.as_str())
        .or_else(|| {
            sl.get("entityId")
                .and_then(|v| v.as_str())
                .and_then(|s| s.rsplit('-').next())
                .filter(|s| s.len() == 11)
        })?
        .trim()
        .to_string();
    if id.is_empty() {
        return None;
    }

    let title = sl
        .pointer("/overlayMetadata/primaryText/content")
        .and_then(|v| v.as_str())
        .or_else(|| {
            sl.get("accessibilityText")
                .and_then(|v| v.as_str())
                .map(|s| s.split(',').next().unwrap_or(s).trim())
        })
        .unwrap_or("YouTube Short")
        .to_string();

    let view_count = sl
        .pointer("/overlayMetadata/secondaryText/content")
        .and_then(|v| v.as_str())
        .and_then(parse_rough_views)
        .or_else(|| {
            sl.get("accessibilityText")
                .and_then(|v| v.as_str())
                .and_then(parse_rough_views)
        });

    let thumbnails = sl
        .pointer("/thumbnailViewModel/thumbnailViewModel/image/sources")
        .or_else(|| ep.and_then(|e| e.pointer("/thumbnail/thumbnails")))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let url = t.get("url")?.as_str()?.to_string();
                    let width = t.get("width").and_then(|w| w.as_u64()).map(|n| n as u32);
                    let height = t.get("height").and_then(|h| h.as_u64()).map(|n| n as u32);
                    Some(Thumbnail { url, width, height })
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());

    Some(VideoItem {
        id: id.clone(),
        title,
        duration: Some(60),
        duration_string: Some("SHORT".to_string()),
        view_count,
        webpage_url: Some(format!("https://www.youtube.com/shorts/{id}")),
        thumbnails,
        ..Default::default()
    })
}

fn extract_video_from_reel_item(r: &serde_json::Value) -> Option<VideoItem> {
    let id = r.get("videoId").and_then(|v| v.as_str())?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let title = extract_yt_text(r.get("headline")).unwrap_or_else(|| "YouTube Short".to_string());
    let view_count = extract_yt_text(r.get("viewCountText")).and_then(|s| parse_rough_views(&s));
    let published_text = extract_yt_text(r.get("publishedTimeText"));
    let timestamp = published_text
        .as_deref()
        .and_then(|s| parse_relative_age_to_unix(s, now_unix_secs()));
    let thumbnails = r
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let url = t.get("url")?.as_str()?.to_string();
                    let width = t.get("width").and_then(|w| w.as_u64()).map(|n| n as u32);
                    let height = t.get("height").and_then(|h| h.as_u64()).map(|n| n as u32);
                    Some(Thumbnail { url, width, height })
                })
                .collect::<Vec<_>>()
        })
        .filter(|v| !v.is_empty());

    Some(VideoItem {
        id: id.clone(),
        title,
        duration: Some(60),
        duration_string: Some("SHORT".to_string()),
        timestamp,
        published_text,
        view_count,
        webpage_url: Some(format!("https://www.youtube.com/shorts/{id}")),
        thumbnails,
        ..Default::default()
    })
}

/// Fetch recent uploads from a YouTube channel via Atom XML RSS (`channel_id` starts with `UC`).
pub async fn fetch_channel_rss(
    client: &reqwest::Client,
    channel_id: &str,
) -> Result<Vec<VideoItem>> {
    let url = format!(
        "https://www.youtube.com/feeds/videos.xml?channel_id={}",
        channel_id.trim()
    );
    let resp = client
        .get(&url)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(AppError::Http(format!("RSS HTTP {}", resp.status())));
    }
    let body = resp.text().await?;
    Ok(parse_youtube_rss_xml(&body))
}

fn extract_tag(slice: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = slice.find(&open)? + open.len();
    let end = slice[start..].find(&close)? + start;
    Some(xml_unescape(slice[start..end].trim()))
}

fn extract_attr(slice: &str, tag_prefix: &str, attr: &str) -> Option<String> {
    let pos = slice.find(tag_prefix)?;
    let tail = &slice[pos..];
    let tag_end = tail.find('>')?;
    let tag_slice = &tail[..tag_end];
    let key = format!("{}=\"", attr);
    let a_start = tag_slice.find(&key)? + key.len();
    let a_end = tag_slice[a_start..].find('"')? + a_start;
    Some(xml_unescape(&tag_slice[a_start..a_end]))
}

fn xml_unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
}

/// Minimal RFC 3339 timestamp parser (`2026-11-14T18:30:00+00:00` or `Z`) -> Unix seconds.
pub fn parse_iso8601_unix(ts: &str) -> Option<u64> {
    let t = ts.trim();
    if t.len() < 19 {
        return None;
    }
    let year: i64 = t.get(0..4)?.parse().ok()?;
    let month: i64 = t.get(5..7)?.parse().ok()?;
    let day: i64 = t.get(8..10)?.parse().ok()?;
    let hour: i64 = t.get(11..13)?.parse().ok()?;
    let min: i64 = t.get(14..16)?.parse().ok()?;
    let sec: i64 = t.get(17..19)?.parse().ok()?;

    // Howard Hinnant civil-from-days inverse (UTC)
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let m_adj = month + if month > 2 { -3 } else { 9 };
    let doy = (153 * m_adj + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;

    let mut epoch = days * 86_400 + hour * 3_600 + min * 60 + sec;

    // Parse optional trailing offset like +03:00 or -05:00
    let rest = &t[19..];
    if let Some(sign) = rest.chars().next() {
        if (sign == '+' || sign == '-') && rest.len() >= 6 {
            if let (Ok(oh), Ok(om)) = (rest[1..3].parse::<i64>(), rest[4..6].parse::<i64>()) {
                let offset_secs = (oh * 3_600 + om * 60) * if sign == '+' { 1 } else { -1 };
                epoch -= offset_secs;
            }
        }
    }

    Some(epoch.max(0) as u64)
}

pub fn parse_youtube_rss_xml(xml: &str) -> Vec<VideoItem> {
    let channel_name = extract_tag(xml, "name");
    let mut items = Vec::new();
    let mut cursor = xml;

    while let Some(entry_start) = cursor.find("<entry>") {
        let after_start = &cursor[entry_start + 7..];
        let Some(entry_end) = after_start.find("</entry>") else {
            break;
        };
        let entry = &after_start[..entry_end];
        cursor = &after_start[entry_end + 8..];

        let Some(id) = extract_tag(entry, "yt:videoId") else {
            continue;
        };
        let title = extract_tag(entry, "title").unwrap_or_else(|| "Untitled".to_string());
        let ch_id = extract_tag(entry, "yt:channelId");
        let entry_author = extract_tag(entry, "name").or_else(|| channel_name.clone());
        let published = extract_tag(entry, "published").and_then(|p| parse_iso8601_unix(&p));
        let description = extract_tag(entry, "media:description");
        let thumb_url = extract_attr(entry, "<media:thumbnail", "url");
        let views = extract_attr(entry, "<media:statistics", "views")
            .and_then(|v| v.parse::<u64>().ok());

        items.push(VideoItem {
            id: id.clone(),
            title,
            uploader: entry_author.clone(),
            channel: entry_author,
            channel_id: ch_id,
            duration: None,
            duration_string: None,
            timestamp: published.map(|u| u as i64),
            view_count: views,
            like_count: None,
            dislike_count: None,
            description,
            webpage_url: Some(format!("https://www.youtube.com/watch?v={}", id)),
            thumbnails: thumb_url.map(|u| {
                vec![Thumbnail {
                    url: u,
                    width: None,
                    height: None,
                }]
            }),
            chapters: None,
            tags: None,
            is_live: None,
            ..Default::default()
        });
    }

    items
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum VideoVote {
    #[default]
    None,
    Liked,
    Disliked,
}

impl VideoVote {
    pub const fn as_db_int(self) -> i64 {
        match self {
            Self::None => 0,
            Self::Liked => 1,
            Self::Disliked => -1,
        }
    }

    pub const fn from_db_int(v: i64) -> Self {
        match v {
            1 => Self::Liked,
            -1 => Self::Disliked,
            _ => Self::None,
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct VideoComment {
    pub id: String,
    pub author: String,
    pub published_text: String,
    pub text: String,
    pub like_count_text: String,
    pub reply_count: u32,
    pub is_pinned: bool,
    pub is_self: bool,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub create_reply_params: Option<String>,
    #[serde(default)]
    pub replies_continuation: Option<String>,
    #[serde(default)]
    pub replies: Vec<VideoComment>,
    #[serde(default)]
    pub replies_expanded: bool,
    #[serde(default)]
    pub replies_loading: bool,
}

#[derive(Debug, Clone, Default)]
pub struct CommentsPage {
    pub comments: Vec<VideoComment>,
    pub continuation: Option<String>,
    pub create_comment_params: Option<String>,
}

fn apply_innertube_web_headers(
    req: reqwest::RequestBuilder,
    session: Option<&GoogleAccountSession>,
) -> reqwest::RequestBuilder {
    if let Some(s) = session {
        apply_innertube_auth_headers(req, s)
    } else {
        req.header(USER_AGENT, PHAENNA_USER_AGENT)
            .header(CONTENT_TYPE, "application/json")
            .header("Origin", "https://www.youtube.com")
            .header("X-Origin", "https://www.youtube.com")
            .header("X-YouTube-Client-Name", "1")
            .header("X-YouTube-Client-Version", INNERTUBE_CLIENT_VERSION)
    }
}

fn first_string_by_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(s) = map.get(key).and_then(|val| val.as_str()) {
                if !s.is_empty() {
                    return Some(s);
                }
            }
            for val in map.values() {
                if let Some(found) = first_string_by_key(val, key) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(found) = first_string_by_key(item, key) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_comments_continuation_in_next(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(map) => {
            if let Some(sec) = map.get("itemSectionRenderer").and_then(|x| x.as_object()) {
                let is_comments = sec
                    .get("sectionIdentifier")
                    .and_then(|x| x.as_str())
                    .map(|s| s == "comment-item-section")
                    .unwrap_or(false)
                    || sec
                        .get("targetId")
                        .and_then(|x| x.as_str())
                        .map(|s| s.contains("comment"))
                        .unwrap_or(false);
                if is_comments {
                    if let Some(tok) = first_string_by_key(&serde_json::Value::Object(sec.clone()), "token") {
                        return Some(tok.to_string());
                    }
                }
            }
            if let Some(panel) = map
                .get("engagementPanelSectionListRenderer")
                .and_then(|x| x.as_object())
            {
                let is_comments = panel
                    .get("panelIdentifier")
                    .and_then(|x| x.as_str())
                    .map(|s| s.contains("comments"))
                    .unwrap_or(false);
                if is_comments {
                    if let Some(tok) =
                        first_string_by_key(&serde_json::Value::Object(panel.clone()), "token")
                    {
                        return Some(tok.to_string());
                    }
                }
            }
            for val in map.values() {
                if let Some(found) = find_comments_continuation_in_next(val) {
                    return Some(found);
                }
            }
            None
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                if let Some(found) = find_comments_continuation_in_next(item) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn clean_comment_body(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .lines()
        .map(|l| l.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn parse_reply_count_val(val: Option<&serde_json::Value>) -> u32 {
    match val {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0) as u32,
        Some(serde_json::Value::String(s)) => s
            .trim()
            .replace(',', "")
            .parse::<u32>()
            .unwrap_or(0),
        _ => 0,
    }
}

fn extract_reply_continuation_from_replies_node(replies_node: &serde_json::Value) -> Option<String> {
    let crr = replies_node.get("commentRepliesRenderer").unwrap_or(replies_node);
    if let Some(contents) = crr.get("contents").and_then(|v| v.as_array()) {
        for item in contents {
            if let Some(tok) = item
                .pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")
                .or_else(|| {
                    item.pointer("/continuationItemRenderer/button/buttonRenderer/command/continuationCommand/token")
                })
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
            {
                return Some(tok.to_string());
            }
        }
    }
    if let Some(tok) = crr
        .pointer("/viewReplies/buttonRenderer/command/continuationCommand/token")
        .or_else(|| crr.pointer("/subThreads/0/continuationItemRenderer/continuationEndpoint/continuationCommand/token"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(tok.to_string());
    }
    first_string_by_key(crr, "token").map(str::to_string)
}

fn normalize_like_count(raw: Option<&str>) -> String {
    let trimmed = raw.unwrap_or("").trim();
    if trimmed.is_empty() || trimmed == "0" {
        "0".to_string()
    } else {
        trimmed.to_string()
    }
}

fn extract_reply_params_from_node(node: &serde_json::Value) -> Option<String> {
    first_string_by_key(node, "createReplyParams")
        .or_else(|| first_string_by_key(node, "createCommentParams"))
        .map(str::to_string)
}

fn parse_comment_renderer_node(cr: &serde_json::Value) -> Option<VideoComment> {
    let id = cr.get("commentId").and_then(|x| x.as_str())?;
    if id.is_empty() {
        return None;
    }
    let author = extract_yt_text(cr.get("authorText")).unwrap_or_else(|| "Viewer".to_string());
    let published_text = extract_yt_text(cr.get("publishedTimeText")).unwrap_or_default();
    let text = extract_yt_text(cr.get("contentText"))
        .map(|s| clean_comment_body(&s))
        .unwrap_or_default();
    if text.is_empty() {
        return None;
    }
    let like_raw = extract_yt_text(cr.get("voteCount"));
    let like_count_text = normalize_like_count(like_raw.as_deref());
    let reply_count = parse_reply_count_val(cr.get("replyCount"));
    let is_pinned = cr.get("pinnedCommentBadge").is_some();
    let create_reply_params = extract_reply_params_from_node(cr);
    Some(VideoComment {
        id: id.to_string(),
        author,
        published_text,
        text,
        like_count_text,
        reply_count,
        is_pinned,
        is_self: false,
        create_reply_params,
        ..Default::default()
    })
}

#[derive(Default)]
struct CommentParseCtx {
    out: Vec<VideoComment>,
    seen: HashSet<String>,
    continuation: Option<String>,
    create_params: Option<String>,
    reply_tokens: std::collections::HashMap<String, String>,
    create_reply_params_by_id: std::collections::HashMap<String, String>,
    inline_replies: std::collections::HashMap<String, Vec<VideoComment>>,
    ordered_ids: Vec<String>,
    ordered_seen: HashSet<String>,
    pinned_ids: HashSet<String>,
}

fn collect_comments_nodes(
    v: &serde_json::Value,
    ctx: &mut CommentParseCtx,
    inside_replies_branch: bool,
) {
    match v {
        serde_json::Value::Object(map) => {
            if ctx.create_params.is_none() {
                if let Some(p) = map.get("createCommentParams").and_then(|x| x.as_str()) {
                    if !p.is_empty() {
                        ctx.create_params = Some(p.to_string());
                    }
                }
            }

            if let Some(ctr) = map.get("commentThreadRenderer") {
                let parent_id = ctr
                    .pointer("/comment/commentRenderer/commentId")
                    .or_else(|| ctr.pointer("/commentViewModel/commentViewModel/commentId"))
                    .or_else(|| ctr.pointer("/comment/commentViewModel/commentId"))
                    .and_then(|x| x.as_str())
                    .or_else(|| {
                        ctr.pointer("/replies/commentRepliesRenderer/targetId")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.strip_prefix("comment-replies-item-"))
                    })
                    .map(str::to_string);

                if let Some(ref pid) = parent_id {
                    if !pid.is_empty() && ctx.ordered_seen.insert(pid.clone()) {
                        ctx.ordered_ids.push(pid.clone());
                    }
                    if ctr
                        .pointer("/commentViewModel/commentViewModel/pinnedText")
                        .or_else(|| ctr.pointer("/comment/commentViewModel/pinnedText"))
                        .is_some()
                    {
                        ctx.pinned_ids.insert(pid.clone());
                    }
                }

                if let (Some(pid), Some(replies_node)) = (parent_id.as_ref(), ctr.get("replies")) {
                    if let Some(tok) = extract_reply_continuation_from_replies_node(replies_node) {
                        ctx.reply_tokens.insert(pid.clone(), tok);
                    }
                    if let Some(contents) = replies_node
                        .get("commentRepliesRenderer")
                        .and_then(|c| c.get("contents"))
                        .and_then(|c| c.as_array())
                    {
                        let mut extracted = Vec::new();
                        for item in contents {
                            if let Some(cr) = item.get("commentRenderer") {
                                if let Some(rc) = parse_comment_renderer_node(cr) {
                                    ctx.seen.insert(rc.id.clone());
                                    extracted.push(rc);
                                }
                            }
                        }
                        if !extracted.is_empty() {
                            ctx.inline_replies.insert(pid.clone(), extracted);
                        }
                    }
                }

                if let Some(comment_node) = ctr
                    .get("comment")
                    .or_else(|| ctr.get("commentViewModel"))
                {
                    collect_comments_nodes(comment_node, ctx, false);
                }
                return;
            }

            if let Some(cvm) = map.get("commentViewModel") {
                if let Some(cid) = cvm
                    .get("commentId")
                    .or_else(|| cvm.pointer("/commentViewModel/commentId"))
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                {
                    if ctx.ordered_seen.insert(cid.to_string()) {
                        ctx.ordered_ids.push(cid.to_string());
                    }
                    if cvm
                        .get("pinnedText")
                        .or_else(|| cvm.pointer("/commentViewModel/pinnedText"))
                        .is_some()
                    {
                        ctx.pinned_ids.insert(cid.to_string());
                    }
                    if let Some(rp) = extract_reply_params_from_node(cvm) {
                        ctx.create_reply_params_by_id
                            .insert(cid.to_string(), rp);
                    }
                }
            }

            if let Some(cr) = map.get("commentRenderer") {
                if let Some(mut cmt) = parse_comment_renderer_node(cr) {
                    if ctx.seen.insert(cmt.id.clone()) {
                        if ctx.ordered_seen.insert(cmt.id.clone()) {
                            ctx.ordered_ids.push(cmt.id.clone());
                        }
                        cmt.replies_continuation = ctx.reply_tokens.remove(&cmt.id);
                        if let Some(reps) = ctx.inline_replies.remove(&cmt.id) {
                            cmt.replies = reps;
                        }
                        ctx.out.push(cmt);
                    }
                }
            }

            if let Some(cep) = map.get("commentEntityPayload") {
                let id = cep
                    .get("properties")
                    .and_then(|p| p.get("commentId"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                if !id.is_empty() && ctx.seen.insert(id.to_string()) {
                    let author = cep
                        .get("author")
                        .and_then(|a| a.get("displayName"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("Viewer")
                        .to_string();
                    let published_text = cep
                        .get("properties")
                        .and_then(|p| p.get("publishedTime"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let text = cep
                        .get("properties")
                        .and_then(|p| p.get("content"))
                        .and_then(|c| c.get("content"))
                        .and_then(|x| x.as_str())
                        .map(clean_comment_body)
                        .unwrap_or_default();
                    let like_count_text = normalize_like_count(
                        cep.get("toolbar")
                            .and_then(|t| t.get("likeCountNotliked"))
                            .and_then(|x| x.as_str()),
                    );
                    let reply_count = parse_reply_count_val(
                        cep.get("toolbar").and_then(|t| t.get("replyCount")),
                    );
                    let create_reply_params = extract_reply_params_from_node(cep)
                        .or_else(|| ctx.create_reply_params_by_id.remove(id));
                    if !text.is_empty() {
                        let replies_continuation = ctx.reply_tokens.remove(id);
                        let replies = ctx.inline_replies.remove(id).unwrap_or_default();
                        ctx.out.push(VideoComment {
                            id: id.to_string(),
                            author,
                            published_text,
                            text,
                            like_count_text,
                            reply_count,
                            is_pinned: ctx.pinned_ids.contains(id),
                            is_self: false,
                            create_reply_params,
                            replies_continuation,
                            replies,
                            ..Default::default()
                        });
                    }
                }
            }

            if !inside_replies_branch && ctx.continuation.is_none() {
                if let Some(ci) = map.get("continuationItemRenderer") {
                    if let Some(tok) = ci
                        .pointer("/continuationEndpoint/continuationCommand/token")
                        .or_else(|| {
                            ci.pointer("/button/buttonRenderer/command/continuationCommand/token")
                        })
                        .and_then(|t| t.as_str())
                        .filter(|s| !s.is_empty())
                    {
                        ctx.continuation = Some(tok.to_string());
                    }
                }
            }

            for (k, val) in map {
                let child_in_replies =
                    inside_replies_branch || k == "replies" || k == "commentRepliesRenderer";
                collect_comments_nodes(val, ctx, child_in_replies);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_comments_nodes(item, ctx, inside_replies_branch);
            }
        }
        _ => {}
    }
}

pub fn parse_innertube_comments_json(root: &serde_json::Value) -> CommentsPage {
    let mut ctx = CommentParseCtx::default();
    if let Some(endpoints) = root.get("onResponseReceivedEndpoints") {
        collect_comments_nodes(endpoints, &mut ctx, false);
    }
    collect_comments_nodes(root, &mut ctx, false);

    let order_map: std::collections::HashMap<String, usize> = ctx
        .ordered_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (id.clone(), i))
        .collect();
    for c in &mut ctx.out {
        if let Some(tok) = ctx.reply_tokens.remove(&c.id) {
            if c.replies_continuation.is_none() {
                c.replies_continuation = Some(tok);
            }
        }
        if c.create_reply_params.is_none() {
            c.create_reply_params = ctx.create_reply_params_by_id.remove(&c.id);
        }
        if ctx.pinned_ids.contains(&c.id) {
            c.is_pinned = true;
        }
    }
    ctx.out
        .sort_by_key(|c| order_map.get(&c.id).copied().unwrap_or(usize::MAX));

    CommentsPage {
        comments: ctx.out,
        continuation: ctx.continuation,
        create_comment_params: ctx.create_params,
    }
}

/// Fetch YouTube comments for a video via InnerTube `/next` + continuation payload.
pub async fn fetch_video_comments(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    video_id: &str,
) -> Result<CommentsPage> {
    let clean_id = video_id.trim_start_matches("RD");
    let init_payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "videoId": clean_id
    });

    let init_req = client
        .post("https://www.youtube.com/youtubei/v1/next?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(10))
        .json(&init_payload);
    let init_resp = apply_innertube_web_headers(init_req, session).send().await?;
    if !init_resp.status().is_success() {
        return Err(AppError::Http(format!(
            "InnerTube comments /next HTTP {}",
            init_resp.status()
        )));
    }
    let init_json: serde_json::Value = init_resp
        .json()
        .await
        .map_err(|e| AppError::Parse(format!("Invalid /next comments JSON: {e}")))?;

    let mut page = parse_innertube_comments_json(&init_json);
    let init_create_params = first_string_by_key(&init_json, "createCommentParams").map(str::to_string);
    if page.create_comment_params.is_none() {
        page.create_comment_params = init_create_params;
    }

    if let Some(cont_tok) = find_comments_continuation_in_next(&init_json) {
        let cont_payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": INNERTUBE_CLIENT_VERSION,
                    "hl": "en",
                    "gl": "US"
                }
            },
            "continuation": cont_tok
        });
        let cont_req = client
            .post("https://www.youtube.com/youtubei/v1/next?prettyPrint=false")
            .timeout(std::time::Duration::from_secs(10))
            .json(&cont_payload);
        if let Ok(cont_resp) = apply_innertube_web_headers(cont_req, session).send().await {
            if cont_resp.status().is_success() {
                if let Ok(cont_json) = cont_resp.json::<serde_json::Value>().await {
                    let next_page = parse_innertube_comments_json(&cont_json);
                    if !next_page.comments.is_empty() {
                        page.comments = next_page.comments;
                    }
                    if next_page.create_comment_params.is_some() {
                        page.create_comment_params = next_page.create_comment_params;
                    }
                    page.continuation = next_page.continuation;
                }
            }
        }
    }

    Ok(page)
}

/// Fetch a subscribed channel's `/shorts` tab directly via InnerTube `browse` (`EgZzaG9ydHPyBgUKA5oBAA%3D%3D`).
pub async fn fetch_channel_shorts(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    channel_id: &str,
) -> Result<Vec<VideoItem>> {
    let clean_ch = channel_id.trim();
    if clean_ch.is_empty() {
        return Ok(Vec::new());
    }
    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "browseId": clean_ch,
        "params": "EgZzaG9ydHPyBgUKA5oBAA%3D%3D"
    });
    let req = client
        .post("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);
    let resp = apply_innertube_web_headers(req, session).send().await?;
    if !resp.status().is_success() {
        return Ok(Vec::new());
    }
    let root: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(format!("Invalid channel shorts JSON: {e}")))?;

    let ch_name = root
        .pointer("/metadata/channelMetadataRenderer/title")
        .or_else(|| root.pointer("/header/c4TabbedHeaderRenderer/title"))
        .or_else(|| root.pointer("/header/pageHeaderRenderer/pageTitle"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);

    let mut page = parse_innertube_browse_json(&root);
    for v in &mut page.videos {
        if v.channel_id.is_none() {
            v.channel_id = Some(clean_ch.to_string());
        }
        if v.display_uploader() == "Unknown" {
            if let Some(ref name) = ch_name {
                v.uploader = Some(name.clone());
                v.channel = Some(name.clone());
            }
        }
        if v.duration.is_none() {
            v.duration = Some(60);
        }
        if v.duration_string.is_none() {
            v.duration_string = Some("SHORT".to_string());
        }
    }
    Ok(page.videos)
}

/// Fetch algorithm-suggested Shorts from YouTube `FEwhat_to_watch` shelves / continuations.
pub async fn fetch_suggested_shorts(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    continuation: Option<&str>,
) -> Result<InnerTubeShelfPage> {
    let mut page = if let Some(sess) = session {
        fetch_innertube_shelf(client, sess, "FEwhat_to_watch", continuation).await?
    } else {
        let mut payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": INNERTUBE_CLIENT_VERSION,
                    "hl": "en",
                    "gl": "US"
                }
            }
        });
        if let Some(cont) = continuation.filter(|c| !c.is_empty()) {
            payload["continuation"] = serde_json::Value::String(cont.to_string());
        } else {
            payload["browseId"] = serde_json::Value::String("FEwhat_to_watch".to_string());
        }
        let req = client
            .post("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false")
            .timeout(std::time::Duration::from_secs(10))
            .json(&payload);
        let resp = apply_innertube_web_headers(req, None).send().await?;
        if !resp.status().is_success() {
            return Ok(InnerTubeShelfPage::default());
        }
        let root: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| AppError::Parse(format!("Invalid browse JSON: {e}")))?;
        parse_innertube_browse_json(&root)
    };

    page.videos.retain(|v| v.is_shorts());
    Ok(page)
}

/// Encode YouTube InnerTube `search` protobuf `params` token for server-side Sort / Upload Date / Duration / Feature flags.
/// `sort_code`: 0=Relevance, 1=Rating, 2=Newest, 3=Views
/// `date_code`: 0=Any, 1=Hour, 2=Today, 3=Week, 4=Month, 5=Year
/// `dur_code`: 0=Any, 1=<4m, 3=4-20m, 2=>20m
pub fn encode_search_params_ex(
    sort_code: u8,
    date_code: u8,
    dur_code: u8,
    videos_only: bool,
    is_hd: bool,
    is_4k: bool,
    is_live: bool,
) -> Option<String> {
    if sort_code == 0
        && date_code == 0
        && dur_code == 0
        && !videos_only
        && !is_hd
        && !is_4k
        && !is_live
    {
        return None;
    }
    let mut sub = Vec::new();
    if date_code > 0 {
        sub.push(1 << 3); // field 1, varint: upload date
        sub.push(date_code);
    }
    if videos_only || date_code > 0 || dur_code > 0 || is_hd || is_4k || is_live {
        sub.push(2 << 3); // field 2, varint: type = Video
        sub.push(1);
    }
    if dur_code > 0 {
        sub.push(3 << 3); // field 3, varint: duration bucket
        sub.push(dur_code);
    }
    if is_hd {
        sub.push(4 << 3); // field 4, varint: HD
        sub.push(1);
    }
    if is_live {
        sub.push(8 << 3); // field 8, varint: Live stream
        sub.push(1);
    }
    if is_4k {
        sub.push(14 << 3); // field 14, varint: 4K
        sub.push(1);
    }

    let mut msg = Vec::new();
    if sort_code > 0 {
        msg.push(1 << 3); // field 1, varint
        msg.push(sort_code);
    }
    if !sub.is_empty() {
        msg.push((2 << 3) | 2); // field 2, length-delimited
        msg.push(sub.len() as u8);
        msg.extend_from_slice(&sub);
    }
    Some(URL_SAFE_NO_PAD.encode(msg))
}

#[allow(dead_code)]
pub fn encode_search_params(
    sort_code: u8,
    date_code: u8,
    dur_code: u8,
    videos_only: bool,
) -> Option<String> {
    encode_search_params_ex(sort_code, date_code, dur_code, videos_only, false, false, false)
}

fn is_noise_search_shelf(title: &str) -> bool {
    let low = title.trim().to_ascii_lowercase();
    if low.is_empty() {
        return false;
    }
    [
        "people also watched",
        "for you",
        "previously watched",
        "channels new to you",
        "related to your search",
        "explore more",
        "from related searches",
        "others watched",
    ]
    .iter()
    .any(|pat| low.contains(pat))
}

fn extract_channel_from_renderer(c: &serde_json::Value) -> Option<SearchChannelCard> {
    let channel_id = c.get("channelId")?.as_str()?.trim().to_string();
    if channel_id.is_empty() {
        return None;
    }
    let title = extract_yt_text(c.get("title")).unwrap_or_else(|| "Channel".to_string());
    let canonical = c
        .pointer("/navigationEndpoint/browseEndpoint/canonicalBaseUrl")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_start_matches('/').to_string());

    // InnerTube swapped subscriberCountText / videoCountText across layout experiments
    let raw_sub = extract_yt_text(c.get("subscriberCountText"));
    let raw_vid = extract_yt_text(c.get("videoCountText"));
    let (handle, subscriber_text) = match (raw_sub, canonical) {
        (Some(s), can) if s.starts_with('@') => (Some(s), raw_vid.clone().or(can)),
        (sub, can) => (can, sub),
    };
    let video_count_text = if subscriber_text.as_deref() == raw_vid.as_deref() {
        None
    } else {
        raw_vid
    };
    let description = extract_yt_text(c.get("descriptionSnippet"));
    let verified = c
        .get("ownerBadges")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().any(|b| {
                let st = b
                    .pointer("/metadataBadgeRenderer/style")
                    .and_then(|s| s.as_str())
                    .unwrap_or("");
                st.contains("VERIFIED")
            })
        })
        .unwrap_or(false);
    let avatar_url = c
        .pointer("/thumbnail/thumbnails")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.last())
        .and_then(|t| t.get("url"))
        .and_then(|u| u.as_str())
        .map(normalize_thumb_url)
        .filter(|s| !s.is_empty());

    Some(SearchChannelCard {
        channel_id,
        title,
        handle,
        subscriber_text,
        video_count_text,
        description,
        verified,
        avatar_url,
    })
}

fn collect_search_container_items(
    items: &[serde_json::Value],
    page: &mut InnerTubeSearchPage,
    seen_videos: &mut HashSet<String>,
    seen_shorts: &mut HashSet<String>,
    seen_channels: &mut HashSet<String>,
) {
    for entry in items {
        let Some(obj) = entry.as_object() else {
            continue;
        };

        // Skip promoted ads / unrelated recommendation cards
        if obj.contains_key("adSlotRenderer")
            || obj.contains_key("promotedSparklesWebRenderer")
            || obj.contains_key("promotedSparklesTextSearchRenderer")
            || obj.contains_key("searchPyvRenderer")
        {
            continue;
        }

        if let Some(did) = obj.get("didYouMeanRenderer") {
            if page.did_you_mean.is_none() {
                page.did_you_mean = extract_yt_text(did.get("correctedQuery"));
            }
            continue;
        }
        if let Some(showing) = obj.get("showingResultsForRenderer") {
            if page.did_you_mean.is_none() {
                page.did_you_mean = extract_yt_text(showing.get("correctedQuery"));
            }
            continue;
        }

        if let Some(ch) = obj.get("channelRenderer").and_then(extract_channel_from_renderer) {
            if seen_channels.insert(ch.channel_id.clone()) {
                page.channels.push(ch);
            }
            continue;
        }

        if let Some(vr) = obj.get("videoRenderer") {
            if let Some(mut v) = extract_video_from_renderer(vr) {
                if is_valid_yt_video_id(v.clean_id()) {
                    if v.is_shorts() {
                        if seen_shorts.insert(v.id.clone()) {
                            page.shorts.push(v);
                        }
                    } else if seen_videos.insert(v.id.clone()) {
                        v.yt_rank = Some(page.videos.len());
                        page.videos.push(v);
                    }
                }
            }
            continue;
        }

        if let Some(lk) = obj.get("lockupViewModel") {
            if let Some(mut v) = extract_video_from_lockup(lk) {
                if is_valid_yt_video_id(v.clean_id()) {
                    if v.is_shorts() {
                        if seen_shorts.insert(v.id.clone()) {
                            page.shorts.push(v);
                        }
                    } else if seen_videos.insert(v.id.clone()) {
                        v.yt_rank = Some(page.videos.len());
                        page.videos.push(v);
                    }
                }
            }
            continue;
        }

        if let Some(sl) = obj.get("shortsLockupViewModel") {
            if let Some(v) = extract_video_from_shorts_lockup(sl) {
                if is_valid_yt_video_id(v.clean_id()) && seen_shorts.insert(v.id.clone()) {
                    page.shorts.push(v);
                }
            }
            continue;
        }

        if let Some(ri) = obj.get("reelItemRenderer") {
            if let Some(v) = extract_video_from_reel_item(ri) {
                if is_valid_yt_video_id(v.clean_id()) && seen_shorts.insert(v.id.clone()) {
                    page.shorts.push(v);
                }
            }
            continue;
        }

        // Isolate Shorts carousels (`reelShelfRenderer` / `gridShelfViewModel`) into `page.shorts`
        if let Some(reel) = obj.get("reelShelfRenderer") {
            if let Some(arr) = reel.get("items").and_then(|v| v.as_array()) {
                collect_search_container_items(arr, page, seen_videos, seen_shorts, seen_channels);
            }
            continue;
        }
        if let Some(grid_shelf) = obj.get("gridShelfViewModel") {
            if let Some(arr) = grid_shelf.pointer("/contents/gridShelfViewModel/contents")
                .or_else(|| grid_shelf.get("contents"))
                .and_then(|v| v.as_array())
            {
                collect_search_container_items(arr, page, seen_videos, seen_shorts, seen_channels);
            }
            continue;
        }

        // Filter out algorithmic noise shelves ("People also watched", "For you", "Previously watched")
        if let Some(shelf) = obj.get("shelfRenderer") {
            let shelf_title = extract_yt_text(shelf.get("title")).unwrap_or_default();
            if is_noise_search_shelf(&shelf_title) {
                continue;
            }
            if let Some(arr) = shelf
                .pointer("/content/verticalListRenderer/items")
                .or_else(|| shelf.pointer("/content/horizontalListRenderer/items"))
                .and_then(|v| v.as_array())
            {
                collect_search_container_items(arr, page, seen_videos, seen_shorts, seen_channels);
            }
            continue;
        }

        if let Some(sec) = obj.get("itemSectionRenderer") {
            if let Some(arr) = sec.get("contents").and_then(|v| v.as_array()) {
                collect_search_container_items(arr, page, seen_videos, seen_shorts, seen_channels);
            }
            continue;
        }

        if page.continuation.is_none() {
            if let Some(cont) = obj.get("continuationItemRenderer") {
                if let Some(tok) = cont
                    .pointer("/continuationEndpoint/continuationCommand/token")
                    .and_then(|v| v.as_str())
                {
                    page.continuation = Some(tok.to_string());
                }
            }
        }
    }
}

/// Structured parser for `/youtubei/v1/search` responses:
/// preserves YouTube ranking order (`yt_rank`), rejects playlists & recommendation noise,
/// isolates Shorts into `page.shorts`, and extracts channel spotlight + match snippets.
pub fn parse_innertube_search_json(root: &serde_json::Value) -> InnerTubeSearchPage {
    let mut page = InnerTubeSearchPage {
        estimated_results: root
            .get("estimatedResults")
            .and_then(|v| v.as_str().and_then(|s| s.parse::<u64>().ok()).or_else(|| v.as_u64())),
        ..Default::default()
    };
    let mut seen_videos = HashSet::new();
    let mut seen_shorts = HashSet::new();
    let mut seen_channels = HashSet::new();

    if let Some(sections) = root
        .pointer("/contents/twoColumnSearchResultsRenderer/primaryContents/sectionListRenderer/contents")
        .and_then(|v| v.as_array())
    {
        collect_search_container_items(
            sections,
            &mut page,
            &mut seen_videos,
            &mut seen_shorts,
            &mut seen_channels,
        );
    }

    if let Some(cmds) = root
        .get("onResponseReceivedCommands")
        .and_then(|v| v.as_array())
    {
        for cmd in cmds {
            if let Some(cont_items) = cmd
                .pointer("/appendContinuationItemsAction/continuationItems")
                .and_then(|v| v.as_array())
            {
                collect_search_container_items(
                    cont_items,
                    &mut page,
                    &mut seen_videos,
                    &mut seen_shorts,
                    &mut seen_channels,
                );
            }
        }
    }

    page
}

/// Fast HTTP InnerTube `/youtubei/v1/search` with server-side filters and continuation pagination (~150-350ms).
pub async fn search_innertube(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    query: &str,
    params: Option<&str>,
    continuation: Option<&str>,
) -> Result<InnerTubeSearchPage> {
    let trimmed = query.trim();
    if trimmed.is_empty() && continuation.is_none() {
        return Ok(InnerTubeSearchPage::default());
    }
    let mut payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        }
    });
    if let Some(cont) = continuation.filter(|c| !c.is_empty()) {
        payload["continuation"] = serde_json::Value::String(cont.to_string());
    } else {
        payload["query"] = serde_json::Value::String(trimmed.to_string());
        if let Some(sp) = params.filter(|p| !p.is_empty()) {
            payload["params"] = serde_json::Value::String(sp.to_string());
        }
    }

    let req = client
        .post("https://www.youtube.com/youtubei/v1/search?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);
    let resp = apply_innertube_web_headers(req, session).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!("InnerTube search HTTP {}", resp.status())));
    }
    let root: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(format!("Invalid search JSON: {e}")))?;
    Ok(parse_innertube_search_json(&root))
}

/// Fetch YouTube live search autocomplete suggestions cleanly (`<120ms`).
pub async fn fetch_search_suggestions(client: &reqwest::Client, query: &str) -> Vec<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let Ok(resp) = client
        .get("https://suggestqueries.google.com/complete/search")
        .query(&[("client", "firefox"), ("ds", "yt"), ("q", trimmed)])
        .timeout(std::time::Duration::from_secs(3))
        .send()
        .await
    else {
        return Vec::new();
    };
    let Ok(arr) = resp.json::<serde_json::Value>().await else {
        return Vec::new();
    };
    arr.get(1)
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|x| x.as_str().map(str::trim))
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case(trimmed))
                .take(6)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch missing `(uploader, view_count, published_text, timestamp)` for any video via InnerTube `/next`.
pub async fn fetch_video_card_enrichment(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    video_id: &str,
) -> Option<(Option<String>, Option<u64>, Option<String>, Option<i64>)> {
    let clean_id = video_id.trim_start_matches("RD");
    if clean_id.is_empty() {
        return None;
    }
    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "videoId": clean_id
    });
    let req = client
        .post("https://www.youtube.com/youtubei/v1/next?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(8))
        .json(&payload);
    let resp = apply_innertube_web_headers(req, session).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;

    let mut primary: Option<&serde_json::Value> = None;
    let mut secondary: Option<&serde_json::Value> = None;
    let mut reel_hdr: Option<&serde_json::Value> = None;
    if let Some(arr) = json
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .and_then(|v| v.as_array())
    {
        for item in arr {
            if primary.is_none() {
                primary = item.get("videoPrimaryInfoRenderer");
            }
            if secondary.is_none() {
                secondary = item.get("videoSecondaryInfoRenderer");
            }
            if reel_hdr.is_none() {
                reel_hdr = item.get("reelPlayerHeaderRenderer");
            }
        }
    }

    let uploader = secondary
        .and_then(|s| extract_yt_text(s.pointer("/owner/videoOwnerRenderer/title")))
        .or_else(|| reel_hdr.and_then(|r| extract_yt_text(r.get("channelTitleText"))));

    let view_count = primary
        .and_then(|p| {
            p.pointer("/viewCount/videoViewCountRenderer/originalViewCount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| {
                    extract_yt_text(p.pointer("/viewCount/videoViewCountRenderer/viewCount"))
                        .and_then(|s| parse_rough_views(&s))
                })
        })
        .or_else(|| {
            first_string_by_key(&json, "originalViewCount")
                .and_then(|s| s.parse::<u64>().ok())
        });

    let rel_text = primary
        .and_then(|p| extract_yt_text(p.get("relativeDateText")))
        .or_else(|| reel_hdr.and_then(|r| extract_yt_text(r.get("timestampText"))));
    let date_text = primary.and_then(|p| extract_yt_text(p.get("dateText")));
    let published_text = rel_text.clone().or_else(|| date_text.clone());

    let now = now_unix_secs();
    let timestamp = rel_text
        .as_deref()
        .and_then(|s| parse_relative_age_to_unix(s, now))
        .or_else(|| date_text.as_deref().and_then(parse_english_short_date_to_unix));

    if uploader.is_none() && view_count.is_none() && published_text.is_none() && timestamp.is_none()
    {
        return None;
    }
    Some((uploader, view_count, published_text, timestamp))
}

/// Fast InnerTube watch-page metadata: full synopsis description, chapters, hashtags, and channel info.
#[derive(Debug, Clone, Default)]
pub struct InnerTubeVideoDetails {
    pub description: Option<String>,
    pub chapters: Vec<VideoChapter>,
    pub tags: Vec<String>,
    pub uploader: Option<String>,
    pub channel_id: Option<String>,
    pub view_count: Option<u64>,
    pub published_text: Option<String>,
    pub timestamp: Option<i64>,
}

fn collect_macro_markers(val: &serde_json::Value, out: &mut Vec<(String, f64)>) {
    match val {
        serde_json::Value::Object(map) => {
            if let Some(m) = map.get("macroMarkersListItemRenderer") {
                if let Some(title) = extract_yt_text(m.get("title")) {
                    if let Some(time_str) = extract_yt_text(m.get("timeDescription")) {
                        if let Some(start) = crate::services::subsync::parse_timestamp(&time_str) {
                            if !title.is_empty() && start.is_finite() && start >= 0.0 {
                                out.push((title, start));
                            }
                        }
                    }
                }
            }
            for child in map.values() {
                collect_macro_markers(child, out);
            }
        }
        serde_json::Value::Array(arr) => {
            for child in arr {
                collect_macro_markers(child, out);
            }
        }
        _ => {}
    }
}

/// Parse synopsis description, chapters, hashtags, and uploader metadata from InnerTube `/youtubei/v1/next` JSON.
pub fn parse_innertube_video_details_json(
    json: &serde_json::Value,
    duration_hint: Option<u64>,
) -> Option<InnerTubeVideoDetails> {
    let mut primary: Option<&serde_json::Value> = None;
    let mut secondary: Option<&serde_json::Value> = None;
    let mut reel_hdr: Option<&serde_json::Value> = None;
    if let Some(arr) = json
        .pointer("/contents/twoColumnWatchNextResults/results/results/contents")
        .and_then(|v| v.as_array())
    {
        for item in arr {
            if primary.is_none() {
                primary = item.get("videoPrimaryInfoRenderer");
            }
            if secondary.is_none() {
                secondary = item.get("videoSecondaryInfoRenderer");
            }
            if reel_hdr.is_none() {
                reel_hdr = item.get("reelPlayerHeaderRenderer");
            }
        }
    }

    let description = secondary
        .and_then(|s| s.pointer("/attributedDescription/content"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim_end().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| {
            secondary
                .and_then(|s| extract_yt_text(s.get("description")))
                .map(|s| s.trim_end().to_string())
                .filter(|s| !s.is_empty())
        });

    let mut raw_markers: Vec<(String, f64)> = Vec::new();
    collect_macro_markers(json, &mut raw_markers);
    raw_markers.sort_by(|a, b| a.1.total_cmp(&b.1));
    raw_markers.dedup_by(|a, b| (a.1 - b.1).abs() < 0.5 && a.0 == b.0);

    let total_dur = duration_hint.unwrap_or(0) as f64;
    let mut chapters = Vec::with_capacity(raw_markers.len());
    for (idx, (title, start)) in raw_markers.iter().enumerate() {
        let next_start = raw_markers
            .get(idx + 1)
            .map(|x| x.1)
            .unwrap_or_else(|| total_dur.max(*start));
        chapters.push(VideoChapter {
            title: title.clone(),
            start_time: *start,
            end_time: next_start.max(*start),
        });
    }
    if chapters.is_empty() {
        if let Some(ref desc) = description {
            let parsed = crate::api::ytdlp::parse_description_chapters(desc, total_dur);
            if !parsed.is_empty() {
                chapters = parsed;
            }
        }
    }

    let mut tags: Vec<String> = Vec::new();
    if let Some(runs) = primary
        .and_then(|p| p.pointer("/superTitleLink/runs"))
        .and_then(|v| v.as_array())
    {
        for run in runs {
            if let Some(txt) = run.get("text").and_then(|v| v.as_str()) {
                for tok in txt.split_whitespace() {
                    if let Some(tag) = tok.strip_prefix('#') {
                        if !tag.is_empty() && !tags.iter().any(|t| t == tag) {
                            tags.push(tag.to_string());
                        }
                    }
                }
            }
        }
    }

    let owner = secondary.and_then(|s| s.pointer("/owner/videoOwnerRenderer"));
    let uploader = owner
        .and_then(|o| extract_yt_text(o.get("title")))
        .or_else(|| reel_hdr.and_then(|r| extract_yt_text(r.get("channelTitleText"))));
    let channel_id = owner
        .and_then(|o| o.pointer("/navigationEndpoint/browseEndpoint/browseId"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned);

    let view_count = primary
        .and_then(|p| {
            p.pointer("/viewCount/videoViewCountRenderer/originalViewCount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .or_else(|| {
                    extract_yt_text(p.pointer("/viewCount/videoViewCountRenderer/viewCount"))
                        .and_then(|s| parse_rough_views(&s))
                })
        })
        .or_else(|| {
            first_string_by_key(json, "originalViewCount").and_then(|s| s.parse::<u64>().ok())
        });

    let rel_text = primary
        .and_then(|p| extract_yt_text(p.get("relativeDateText")))
        .or_else(|| reel_hdr.and_then(|r| extract_yt_text(r.get("timestampText"))));
    let date_text = primary.and_then(|p| extract_yt_text(p.get("dateText")));
    let published_text = rel_text.clone().or_else(|| date_text.clone());

    let now = now_unix_secs();
    let timestamp = rel_text
        .as_deref()
        .and_then(|s| parse_relative_age_to_unix(s, now))
        .or_else(|| date_text.as_deref().and_then(parse_english_short_date_to_unix));

    if description.is_none()
        && chapters.is_empty()
        && tags.is_empty()
        && uploader.is_none()
        && channel_id.is_none()
        && view_count.is_none()
        && published_text.is_none()
    {
        return None;
    }

    Some(InnerTubeVideoDetails {
        description,
        chapters,
        tags,
        uploader,
        channel_id,
        view_count,
        published_text,
        timestamp,
    })
}

/// Fetch synopsis description, chapters, hashtags, and uploader metadata quickly (~250ms) via InnerTube `/next`.
pub async fn fetch_innertube_video_details(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    video_id: &str,
    duration_hint: Option<u64>,
) -> Option<InnerTubeVideoDetails> {
    let clean_id = video_id.trim_start_matches("RD");
    if clean_id.is_empty() {
        return None;
    }
    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "videoId": clean_id
    });
    let req = client
        .post("https://www.youtube.com/youtubei/v1/next?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);
    let resp = apply_innertube_web_headers(req, session).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    parse_innertube_video_details_json(&json, duration_hint)
}

/// Fetch replies for a parent comment thread using its InnerTube continuation token.
pub async fn fetch_comment_replies(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    continuation_token: &str,
) -> Result<CommentsPage> {
    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "continuation": continuation_token
    });
    let req = client
        .post("https://www.youtube.com/youtubei/v1/next?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);
    let resp = apply_innertube_web_headers(req, session).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "InnerTube comment replies HTTP {}",
            resp.status()
        )));
    }
    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(format!("Invalid /next replies JSON: {e}")))?;
    Ok(parse_innertube_comments_json(&json))
}

/// Post a comment to YouTube via InnerTube `/comment/create_comment`.
pub async fn post_video_comment(
    client: &reqwest::Client,
    session: &GoogleAccountSession,
    video_id: &str,
    cached_params: Option<&str>,
    text: &str,
) -> Result<()> {
    let params = if let Some(p) = cached_params.filter(|s| !s.is_empty()) {
        p.to_string()
    } else {
        let page = fetch_video_comments(client, Some(session), video_id).await?;
        page.create_comment_params.ok_or_else(|| {
            AppError::Http("YouTube createCommentParams unavailable for this session".to_string())
        })?
    };

    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "createCommentParams": params,
        "commentText": text
    });

    let req = client
        .post("https://www.youtube.com/youtubei/v1/comment/create_comment?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);
    let resp = apply_innertube_auth_headers(req, session).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "Create comment HTTP {}",
            resp.status()
        )));
    }
    Ok(())
}

/// Reply to a comment or nested sub-comment via InnerTube `/comment/create_comment_reply`.
pub async fn post_comment_reply(
    client: &reqwest::Client,
    session: &GoogleAccountSession,
    video_id: &str,
    reply_params: Option<&str>,
    text: &str,
) -> Result<()> {
    let params = match reply_params.filter(|s| !s.trim().is_empty()) {
        Some(p) => p.to_string(),
        None => {
            let page = fetch_video_comments(client, Some(session), video_id).await?;
            page.create_comment_params.ok_or_else(|| {
                AppError::Http("YouTube createReplyParams unavailable for this thread".to_string())
            })?
        }
    };

    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "createReplyParams": params,
        "commentText": text
    });

    let req = client
        .post("https://www.youtube.com/youtubei/v1/comment/create_comment_reply?prettyPrint=false")
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);
    let resp = apply_innertube_auth_headers(req, session).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "Create reply HTTP {}",
            resp.status()
        )));
    }
    Ok(())
}

/// Submit Like, Dislike, or Remove Vote to YouTube InnerTube `/like/{like,dislike,removelike}`.
pub async fn send_video_vote(
    client: &reqwest::Client,
    session: &GoogleAccountSession,
    video_id: &str,
    vote: VideoVote,
) -> Result<()> {
    let clean_id = video_id.trim_start_matches("RD");
    let action = match vote {
        VideoVote::Liked => "like",
        VideoVote::Disliked => "dislike",
        VideoVote::None => "removelike",
    };
    let endpoint = format!("https://www.youtube.com/youtubei/v1/like/{action}?prettyPrint=false");
    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": INNERTUBE_CLIENT_VERSION,
                "hl": "en",
                "gl": "US"
            }
        },
        "target": {
            "videoId": clean_id
        }
    });

    let req = client
        .post(&endpoint)
        .timeout(std::time::Duration::from_secs(10))
        .json(&payload);
    let resp = apply_innertube_auth_headers(req, session).send().await?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "InnerTube vote ({action}) HTTP {}",
            resp.status()
        )));
    }
    Ok(())
}

/// Generate a deterministic 16-character URL-safe YouTube client playback nonce (`cpn`).
pub fn generate_cpn(video_id: &str, now_unix: u64) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut state = 0x9E37_79B9_7F4A_7C15u64 ^ now_unix.rotate_left(17);
    for &b in video_id.as_bytes() {
        state = state.wrapping_mul(0xBF58_476D_1CE4_E5B9) ^ (b as u64);
    }
    let mut out = String::with_capacity(16);
    for _ in 0..16 {
        state = state.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let idx = ((state >> 32) as usize) % ALPHABET.len();
        out.push(ALPHABET[idx] as char);
    }
    out
}

fn normalize_tracking_url(raw: &str) -> String {
    if let Some(rest) = raw.strip_prefix("http://") {
        format!("https://{rest}")
    } else {
        raw.to_string()
    }
}

/// Extract `(videostatsPlaybackUrl, videostatsWatchtimeUrl, ptrackingUrl)` from InnerTube `/player` JSON.
pub fn extract_playback_tracking_urls(
    player_json: &serde_json::Value,
) -> (Option<String>, Option<String>, Option<String>) {
    let playback = player_json
        .pointer("/playbackTracking/videostatsPlaybackUrl/baseUrl")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(normalize_tracking_url);
    let watchtime = player_json
        .pointer("/playbackTracking/videostatsWatchtimeUrl/baseUrl")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(normalize_tracking_url);
    let ptracking = player_json
        .pointer("/playbackTracking/ptrackingUrl/baseUrl")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(normalize_tracking_url);
    (playback, watchtime, ptracking)
}

fn append_query(base: &str, params: &str) -> String {
    let sep = if base.contains('?') { '&' } else { '?' };
    format!("{base}{sep}{params}")
}

/// Register video view & watchtime progress directly in YouTube watch history.
#[allow(clippy::too_many_arguments)]
pub async fn report_youtube_watch_progress(
    client: &reqwest::Client,
    session: Option<&GoogleAccountSession>,
    cookies_path: Option<&std::path::Path>,
    video_id: &str,
    start_secs: f64,
    current_secs: f64,
    duration_secs: Option<f64>,
    is_initial_start: bool,
) -> Result<bool> {
    let clean_id = video_id.trim_start_matches("RD");
    if clean_id.is_empty() {
        return Ok(false);
    }

    let mut reported = false;

    if let Some(sess) = session {
        let player_payload = serde_json::json!({
            "context": {
                "client": {
                    "clientName": "WEB",
                    "clientVersion": INNERTUBE_CLIENT_VERSION,
                    "hl": "en",
                    "gl": "US"
                }
            },
            "videoId": clean_id,
            "playbackContext": {
                "contentPlaybackContext": {
                    "html5Preference": "HTML5_PREF_WANTS"
                }
            }
        });

        let player_req = client
            .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .timeout(std::time::Duration::from_secs(10))
            .json(&player_payload);

        if let Ok(resp) = apply_innertube_web_headers(player_req, Some(sess)).send().await {
            if resp.status().is_success() {
                if let Ok(player_json) = resp.json::<serde_json::Value>().await {
                    let (playback_url, watchtime_url, ptracking_url) =
                        extract_playback_tracking_urls(&player_json);
                    let cpn = generate_cpn(clean_id, now_unix_secs().max(0) as u64);
                    let referer = format!("https://www.youtube.com/watch?v={clean_id}");
                    let st = start_secs.max(0.0);
                    let et = current_secs.max(st);

                    if is_initial_start {
                        if let Some(ref p_url) = ptracking_url {
                            let url = append_query(p_url, &format!("cpn={cpn}"));
                            let r = apply_innertube_web_headers(
                                client.get(&url).timeout(std::time::Duration::from_secs(6)),
                                Some(sess),
                            )
                            .header("Referer", &referer)
                            .send()
                            .await;
                            if let Ok(ok_r) = r {
                                if ok_r.status().is_success() {
                                    reported = true;
                                }
                            }
                        }
                    }

                    if let Some(ref pb_url) = playback_url {
                        let url = append_query(
                            pb_url,
                            &format!(
                                "ver=2&c=WEB&cver={INNERTUBE_CLIENT_VERSION}&cpn={cpn}&cmt={et:.1}"
                            ),
                        );
                        let r = apply_innertube_web_headers(
                            client.get(&url).timeout(std::time::Duration::from_secs(6)),
                            Some(sess),
                        )
                        .header("Referer", &referer)
                        .send()
                        .await;
                        if let Ok(ok_r) = r {
                            if ok_r.status().is_success() {
                                reported = true;
                            }
                        }
                    }

                    if let Some(ref wt_url) = watchtime_url {
                        let mut q = format!(
                            "ver=2&c=WEB&cver={INNERTUBE_CLIENT_VERSION}&cpn={cpn}&st={st:.1}&et={et:.1}&cmt={et:.1}&state=playing"
                        );
                        if let Some(dur) = duration_secs.filter(|d| *d > 0.0) {
                            q.push_str(&format!("&len={dur:.1}"));
                        }
                        let url = append_query(wt_url, &q);
                        let r = apply_innertube_web_headers(
                            client.get(&url).timeout(std::time::Duration::from_secs(6)),
                            Some(sess),
                        )
                        .header("Referer", &referer)
                        .send()
                        .await;
                        if let Ok(ok_r) = r {
                            if ok_r.status().is_success() {
                                reported = true;
                            }
                        }
                    }
                }
            }
        }
    }

    if is_initial_start && !reported {
        if let Some(cpath) = cookies_path.filter(|p| p.exists()) {
            if let Some(ytdlp_bin) = crate::api::player::find_binary("yt-dlp") {
                let watch_url = format!("https://www.youtube.com/watch?v={clean_id}");
                let status = tokio::process::Command::new(ytdlp_bin)
                    .arg("--no-warnings")
                    .arg("--simulate")
                    .arg("--skip-download")
                    .arg("--mark-watched")
                    .arg("--cookies")
                    .arg(cpath)
                    .arg(&watch_url)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .await;
                if let Ok(st) = status {
                    if st.success() {
                        reported = true;
                    }
                }
            }
        }
    }

    Ok(reported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_youtube_rss_xml() {
        let sample = r#"<?xml version="1.0" encoding="UTF-8"?>
        <feed xmlns:yt="http://www.youtube.com/xml/schemas/2015" xmlns:media="http://search.yahoo.com/mrss/">
          <author><name>Veritasium</name></author>
          <entry>
            <yt:videoId>abc123XYZ00</yt:videoId>
            <yt:channelId>UCHnyfMqiRRG1u-2MsSQLbXA</yt:channelId>
            <title>Why No One Measured The Speed Of Light &amp; Physics</title>
            <published>2026-11-10T15:00:00+00:00</published>
            <media:group>
              <media:thumbnail url="https://i.ytimg.com/vi/abc123XYZ00/hqdefault.jpg" width="480" height="360"/>
              <media:description>Deep dive description</media:description>
              <media:community><media:statistics views="1234567"/></media:community>
            </media:group>
          </entry>
        </feed>"#;

        let vids = parse_youtube_rss_xml(sample);
        assert_eq!(vids.len(), 1);
        assert_eq!(vids[0].id, "abc123XYZ00");
        assert_eq!(
            vids[0].title,
            "Why No One Measured The Speed Of Light & Physics"
        );
        assert_eq!(vids[0].uploader.as_deref(), Some("Veritasium"));
        assert_eq!(vids[0].view_count, Some(1_234_567));
        assert!(vids[0].timestamp.unwrap_or(0) > 1_700_000_000);
    }

    #[test]
    fn test_parse_innertube_browse_json_extracts_renderers_and_continuation() {
        let sample = serde_json::json!({
            "contents": {
                "twoColumnBrowseResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "richGridRenderer": {
                                    "contents": [
                                        {
                                            "richItemRenderer": {
                                                "content": {
                                                    "videoRenderer": {
                                                        "videoId": "vid_001",
                                                        "title": { "runs": [{ "text": "Phaenna Rust 2026" }] },
                                                        "ownerText": {
                                                            "runs": [{
                                                                "text": "DevChannel",
                                                                "navigationEndpoint": {
                                                                    "browseEndpoint": { "browseId": "UCdev123" }
                                                                }
                                                            }]
                                                        },
                                                        "lengthText": { "simpleText": "12:34" },
                                                        "viewCountText": { "simpleText": "42,100 views" },
                                                        "thumbnail": {
                                                            "thumbnails": [{ "url": "https://i.ytimg.com/vi/vid_001/hqdefault.jpg", "width": 480, "height": 360 }]
                                                        }
                                                    }
                                                }
                                            }
                                        },
                                        {
                                            "gridVideoRenderer": {
                                                "videoId": "vid_001",
                                                "title": { "simpleText": "Duplicate Should Dedupe" }
                                            }
                                        },
                                        {
                                            "playlistVideoRenderer": {
                                                "videoId": "vid_002",
                                                "title": { "simpleText": "Watch Later Entry" },
                                                "lengthText": { "simpleText": "1:02:03" }
                                            }
                                        },
                                        {
                                            "continuationItemRenderer": {
                                                "continuationEndpoint": {
                                                    "continuationCommand": { "token": "CONT_TOKEN_ABC" }
                                                }
                                            }
                                        }
                                    ]
                                }
                            }
                        }
                    }]
                }
            }
        });

        let page = parse_innertube_browse_json(&sample);
        assert_eq!(page.videos.len(), 2);
        assert_eq!(page.videos[0].id, "vid_001");
        assert_eq!(page.videos[0].title, "Phaenna Rust 2026");
        assert_eq!(page.videos[0].uploader.as_deref(), Some("DevChannel"));
        assert_eq!(page.videos[0].channel_id.as_deref(), Some("UCdev123"));
        assert_eq!(page.videos[0].duration, Some(754));
        assert_eq!(page.videos[0].view_count, Some(42_100));
        assert_eq!(page.videos[1].id, "vid_002");
        assert_eq!(page.videos[1].duration, Some(3723));
        assert_eq!(page.continuation.as_deref(), Some("CONT_TOKEN_ABC"));
    }

    #[test]
    fn test_parse_rough_views_and_relative_age_and_lockup() {
        assert_eq!(parse_rough_views("1.4M views"), Some(1_400_000));
        assert_eq!(parse_rough_views("27.5K views"), Some(27_500));
        assert_eq!(parse_rough_views("42,100 views"), Some(42_100));
        assert_eq!(parse_rough_views("No views"), Some(0));

        let now = 1_763_200_000;
        assert_eq!(
            parse_relative_age_to_unix("3 hours ago", now),
            Some(now - 10_800)
        );
        assert_eq!(
            parse_relative_age_to_unix("Streamed 2 days ago", now),
            Some(now - 172_800)
        );

        let lockup = serde_json::json!({
            "lockupViewModel": {
                "contentId": "lk_001",
                "metadata": {
                    "lockupMetadataViewModel": {
                        "title": { "content": "Modern 2026 Lockup Card" },
                        "metadata": {
                            "contentMetadataViewModel": {
                                "metadataRows": [
                                    { "metadataParts": [{ "text": { "content": "Channel Four" } }] },
                                    { "metadataParts": [
                                        { "text": { "content": "128K views" } },
                                        { "text": { "content": "3 hours ago" } }
                                    ]}
                                ]
                            }
                        }
                    }
                },
                "contentImage": {
                    "thumbnailViewModel": {
                        "image": { "sources": [{ "url": "https://i.ytimg.com/vi/lk_001/hqdefault.jpg", "width": 480, "height": 360 }] },
                        "overlays": [
                            { "thumbnailOverlayBadgeViewModel": { "thumbnailBadges": [{ "thumbnailBadgeViewModel": { "text": "14:22" } }] } }
                        ]
                    }
                }
            }
        });
        let page = parse_innertube_browse_json(&lockup);
        assert_eq!(page.videos.len(), 1);
        assert_eq!(page.videos[0].id, "lk_001");
        assert_eq!(page.videos[0].view_count, Some(128_000));
        assert_eq!(page.videos[0].published_text.as_deref(), Some("3 hours ago"));
        assert_eq!(page.videos[0].duration, Some(862));
        assert!(page.videos[0].is_new_release(now_unix_secs()));
    }

    #[test]
    fn test_parse_innertube_comments_classic_and_entity_payload() {
        let sample = serde_json::json!({
            "onResponseReceivedEndpoints": [{
                "reloadContinuationItemsCommand": {
                    "continuationItems": [{
                        "commentThreadRenderer": {
                            "comment": {
                                "commentRenderer": {
                                    "commentId": "cmt_classic_1",
                                    "authorText": { "simpleText": "@audiofan" },
                                    "publishedTimeText": { "runs": [{ "text": "2 hours ago" }] },
                                    "contentText": { "runs": [
                                        { "text": "Great breakdown of the driver layout " },
                                        { "text": "at 2:47!" }
                                    ]},
                                    "voteCount": { "simpleText": "142" },
                                    "replyCount": 4,
                                    "pinnedCommentBadge": { "pinnedCommentBadgeRenderer": {} }
                                }
                            },
                            "replies": {
                                "commentRepliesRenderer": {
                                    "contents": [{
                                        "continuationItemRenderer": {
                                            "continuationEndpoint": {
                                                "continuationCommand": { "token": "reply_tok_classic_1" }
                                            }
                                        }
                                    }]
                                }
                            }
                        }
                    }, {
                        "commentThreadRenderer": {
                            "commentViewModel": {
                                "commentViewModel": {
                                    "commentId": "cmt_entity_2",
                                    "pinnedText": "Pinned"
                                }
                            },
                            "replies": {
                                "commentRepliesRenderer": {
                                    "targetId": "comment-replies-item-cmt_entity_2",
                                    "contents": [{
                                        "continuationItemRenderer": {
                                            "continuationEndpoint": {
                                                "continuationCommand": { "token": "reply_tok_entity_2" }
                                            }
                                        }
                                    }]
                                }
                            }
                        }
                    }, {
                        "continuationItemRenderer": {
                            "continuationEndpoint": {
                                "continuationCommand": { "token": "top_level_next_page" }
                            }
                        }
                    }]
                }
            }],
            "frameworkUpdates": {
                "entityBatchUpdate": {
                    "mutations": [{
                        "payload": {
                            "commentEntityPayload": {
                                "properties": {
                                    "commentId": "cmt_entity_2",
                                    "publishedTime": "18 minutes ago",
                                    "content": { "content": "Zero word concatenation on wrapped lines please." }
                                },
                                "author": { "displayName": "@rust_listener" },
                                "toolbar": {
                                    "likeCountNotliked": "29",
                                    "replyCount": "2"
                                }
                            }
                        }
                    }]
                }
            }
        });

        let parsed = parse_innertube_comments_json(&sample);
        assert_eq!(parsed.comments.len(), 2);
        assert_eq!(parsed.continuation.as_deref(), Some("top_level_next_page"));
        assert_eq!(parsed.comments[0].author, "@audiofan");
        assert!(parsed.comments[0].is_pinned);
        assert_eq!(parsed.comments[0].text, "Great breakdown of the driver layout at 2:47!");
        assert_eq!(
            parsed.comments[0].replies_continuation.as_deref(),
            Some("reply_tok_classic_1")
        );
        assert_eq!(parsed.comments[1].author, "@rust_listener");
        assert_eq!(parsed.comments[1].like_count_text, "29");
        assert_eq!(parsed.comments[1].reply_count, 2);
        assert_eq!(
            parsed.comments[1].replies_continuation.as_deref(),
            Some("reply_tok_entity_2")
        );

        let spaced_runs = serde_json::json!({
            "runs": [
                {"text": "Watch"},
                {"text": "@rust_listener"},
                {"text": "check"},
                {"text": "https://example.com"}
            ]
        });
        assert_eq!(
            extract_yt_text(Some(&spaced_runs)).as_deref(),
            Some("Watch @rust_listener check https://example.com")
        );
        assert_eq!(normalize_like_count(Some("   ")), "0");
        assert_eq!(normalize_like_count(Some(" 42 ")), "42");
    }

    #[test]
    fn test_extract_playback_tracking_urls_and_watched_overlays() {
        let player = serde_json::json!({
            "playbackTracking": {
                "videostatsPlaybackUrl": { "baseUrl": "http://s.youtube.com/api/stats/playback?cl=123" },
                "videostatsWatchtimeUrl": { "baseUrl": "https://s.youtube.com/api/stats/watchtime?cl=123" },
                "ptrackingUrl": { "baseUrl": "https://www.youtube.com/ptracking?html=1" }
            }
        });
        let (pb, wt, pt) = extract_playback_tracking_urls(&player);
        assert_eq!(pb.as_deref(), Some("https://s.youtube.com/api/stats/playback?cl=123"));
        assert_eq!(wt.as_deref(), Some("https://s.youtube.com/api/stats/watchtime?cl=123"));
        assert_eq!(pt.as_deref(), Some("https://www.youtube.com/ptracking?html=1"));

        let cpn = generate_cpn("dQw4w9WgXcQ", 1_700_000_000);
        assert_eq!(cpn.len(), 16);
        assert!(cpn
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));

        let browse = serde_json::json!({
            "items": [
                {
                    "videoRenderer": {
                        "videoId": "vid_classic_90",
                        "title": { "simpleText": "Classic Watched" },
                        "thumbnailOverlays": [
                            { "thumbnailOverlayResumePlaybackRenderer": { "percentDurationWatched": 90 } }
                        ]
                    }
                },
                {
                    "lockupViewModel": {
                        "contentId": "vid_lockup_64",
                        "metadata": { "lockupMetadataViewModel": { "title": { "content": "Lockup Watched" } } },
                        "contentImage": {
                            "thumbnailViewModel": {
                                "overlays": [
                                    { "thumbnailOverlayProgressBarViewModel": { "startPercent": 64 } }
                                ]
                            }
                        }
                    }
                }
            ]
        });
        let page = parse_innertube_browse_json(&browse);
        assert_eq!(page.videos.len(), 2);
        assert_eq!(
            page.watched_progress,
            vec![
                ("vid_classic_90".to_string(), 90),
                ("vid_lockup_64".to_string(), 64)
            ]
        );
    }

    #[test]
    fn test_lockup_combined_delimiter_and_new_badge_and_shorts_lockup() {
        let combined_lockup = serde_json::json!({
            "items": [
                {
                    "lockupViewModel": {
                        "contentId": "comb_001",
                        "metadata": {
                            "lockupMetadataViewModel": {
                                "title": { "content": "Continuation Card Title" },
                                "image": { "decoratedAvatarViewModel": { "a11yLabel": "Go to channel SystemsLab" } },
                                "metadata": {
                                    "contentMetadataViewModel": {
                                        "metadataRows": [
                                            { "metadataParts": [{ "text": { "content": "42K views • 2 hours ago" } }] },
                                            { "badges": [{ "badgeViewModel": { "badgeText": "New" } }] }
                                        ]
                                    }
                                }
                            }
                        }
                    }
                },
                {
                    "shortsLockupViewModel": {
                        "entityId": "shorts-shelf-item-V8rT6UI3Crg",
                        "accessibilityText": "Making Gunpowder From Scratch, 805 thousand views - play Short",
                        "overlayMetadata": {
                            "primaryText": { "content": "Making Gunpowder From Scratch" },
                            "secondaryText": { "content": "805K views" }
                        },
                        "onTap": {
                            "innertubeCommand": {
                                "reelWatchEndpoint": {
                                    "videoId": "V8rT6UI3Crg"
                                }
                            }
                        }
                    }
                }
            ]
        });

        let page = parse_innertube_browse_json(&combined_lockup);
        assert_eq!(page.videos.len(), 2);

        let v0 = &page.videos[0];
        assert_eq!(v0.id, "comb_001");
        assert_eq!(v0.display_uploader(), "SystemsLab");
        assert_eq!(v0.view_count, Some(42_000));
        assert_eq!(v0.published_text.as_deref(), Some("2 hours ago"));
        assert!(v0.timestamp.is_some());
        assert!(v0.is_new_release(now_unix_secs()));

        let v1 = &page.videos[1];
        assert_eq!(v1.id, "V8rT6UI3Crg");
        assert_eq!(v1.view_count, Some(805_000));
        assert!(v1.is_shorts());
        assert_eq!(v1.duration_string.as_deref(), Some("SHORT"));

        assert!(parse_english_short_date_to_unix("Nov 15, 2026").is_some());
        assert!(encode_search_params(0, 0, 0, false).is_none());
        let sp = encode_search_params(2, 3, 1, true).unwrap();
        assert!(!sp.is_empty());
        assert!(encode_search_params_ex(0, 0, 0, false, false, true, false).is_some());
    }

    #[test]
    fn test_parse_innertube_search_json_filters_playlists_shelves_and_ranks() {
        let search_json = serde_json::json!({
            "estimatedResults": "42000",
            "contents": {
                "twoColumnSearchResultsRenderer": {
                    "primaryContents": {
                        "sectionListRenderer": {
                            "contents": [
                                {
                                    "itemSectionRenderer": {
                                        "contents": [
                                            {
                                                "didYouMeanRenderer": {
                                                    "correctedQuery": { "simpleText": "system architecture patterns" }
                                                }
                                            },
                                            {
                                                "channelRenderer": {
                                                    "channelId": "UCn1f2k3m4n5p",
                                                    "title": { "simpleText": "Gaurav Sen" },
                                                    "subscriberCountText": { "simpleText": "612K subscribers" },
                                                    "ownerBadges": [{ "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_VERIFIED" } }]
                                                }
                                            },
                                            {
                                                "lockupViewModel": {
                                                    "contentId": "PLMC9KNkIncKtPzgY-5rmhvj7fax8XcWcQ",
                                                    "contentType": "LOCKUP_CONTENT_TYPE_PLAYLIST",
                                                    "metadata": { "lockupMetadataViewModel": { "title": { "content": "System Design Playlist" } } }
                                                }
                                            },
                                            {
                                                "videoRenderer": {
                                                    "videoId": "dQw4w9WgXcQ",
                                                    "title": { "simpleText": "System Architecture Patterns Explained" },
                                                    "ownerText": { "simpleText": "ByteByteGo" },
                                                    "lengthText": { "simpleText": "18:42" },
                                                    "viewCountText": { "simpleText": "1.4M views" },
                                                    "ownerBadges": [{ "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_VERIFIED" } }],
                                                    "detailedMetadataSnippets": [{
                                                        "snippetText": { "runs": [{ "text": "Compare layered and hexagonal architecture." }] }
                                                    }]
                                                }
                                            },
                                            {
                                                "reelShelfRenderer": {
                                                    "items": [{
                                                        "shortsLockupViewModel": {
                                                            "onTap": { "innertubeCommand": { "reelWatchEndpoint": { "videoId": "V8rT6UI3Crg" } } },
                                                            "overlayMetadata": {
                                                                "primaryText": { "content": "Uber Architecture in 50s" },
                                                                "secondaryText": { "content": "959K views" }
                                                            }
                                                        }
                                                    }]
                                                }
                                            },
                                            {
                                                "shelfRenderer": {
                                                    "title": { "simpleText": "People also watched" },
                                                    "content": {
                                                        "verticalListRenderer": {
                                                            "items": [{
                                                                "videoRenderer": {
                                                                    "videoId": "9bZkp7q19f0",
                                                                    "title": { "simpleText": "Unrelated Noise Video" }
                                                                }
                                                            }]
                                                        }
                                                    }
                                                }
                                            }
                                        ]
                                    }
                                },
                                {
                                    "continuationItemRenderer": {
                                        "continuationEndpoint": { "continuationCommand": { "token": "SEARCH_CONT_2" } }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let page = parse_innertube_search_json(&search_json);
        assert_eq!(page.estimated_results, Some(42_000));
        assert_eq!(page.did_you_mean.as_deref(), Some("system architecture patterns"));
        assert_eq!(page.channels.len(), 1);
        assert_eq!(page.channels[0].title, "Gaurav Sen");
        assert!(page.channels[0].verified);
        assert_eq!(page.videos.len(), 1);
        assert_eq!(page.videos[0].id, "dQw4w9WgXcQ");
        assert_eq!(page.videos[0].yt_rank, Some(0));
        assert!(page.videos[0].channel_verified);
        assert_eq!(
            page.videos[0].search_snippet.as_deref(),
            Some("Compare layered and hexagonal architecture.")
        );
        assert_eq!(page.shorts.len(), 1);
        assert_eq!(page.shorts[0].id, "V8rT6UI3Crg");
        assert_eq!(page.continuation.as_deref(), Some("SEARCH_CONT_2"));
    }

    #[test]
    fn test_parse_innertube_video_details_json() {
        let next_json = serde_json::json!({
            "contents": {
                "twoColumnWatchNextResults": {
                    "results": {
                        "results": {
                            "contents": [
                                {
                                    "videoPrimaryInfoRenderer": {
                                        "superTitleLink": {
                                            "runs": [
                                                { "text": "#Foldable" },
                                                { "text": " " },
                                                { "text": "#TechReview" }
                                            ]
                                        },
                                        "viewCount": {
                                            "videoViewCountRenderer": {
                                                "originalViewCount": "7942"
                                            }
                                        },
                                        "relativeDateText": { "simpleText": "4 hours ago" }
                                    }
                                },
                                {
                                    "videoSecondaryInfoRenderer": {
                                        "attributedDescription": {
                                            "content": "Hands-on hardware deep dive.\n00:00 Intro\n01:30 Display"
                                        },
                                        "owner": {
                                            "videoOwnerRenderer": {
                                                "title": { "runs": [{ "text": "Shane Craig" }] },
                                                "navigationEndpoint": {
                                                    "browseEndpoint": { "browseId": "UCtestChannel" }
                                                }
                                            }
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            },
            "engagementPanels": [
                {
                    "engagementPanelSectionListRenderer": {
                        "content": {
                            "macroMarkersListRenderer": {
                                "contents": [
                                    {
                                        "macroMarkersListItemRenderer": {
                                            "title": { "simpleText": "Overview" },
                                            "timeDescription": { "simpleText": "0:00" }
                                        }
                                    },
                                    {
                                        "macroMarkersListItemRenderer": {
                                            "title": { "runs": [{ "text": "Hinge Mechanics" }] },
                                            "timeDescription": { "simpleText": "2:15" }
                                        }
                                    }
                                ]
                            }
                        }
                    }
                }
            ]
        });

        let parsed = parse_innertube_video_details_json(&next_json, Some(441)).unwrap();
        assert!(parsed
            .description
            .as_deref()
            .unwrap()
            .contains("Hands-on hardware deep dive."));
        assert_eq!(parsed.chapters.len(), 2);
        assert_eq!(parsed.chapters[0].title, "Overview");
        assert_eq!(parsed.chapters[0].start_time, 0.0);
        assert_eq!(parsed.chapters[1].title, "Hinge Mechanics");
        assert_eq!(parsed.chapters[1].start_time, 135.0);
        assert_eq!(parsed.chapters[1].end_time, 441.0);
        assert_eq!(parsed.tags, vec!["Foldable", "TechReview"]);
        assert_eq!(parsed.uploader.as_deref(), Some("Shane Craig"));
        assert_eq!(parsed.channel_id.as_deref(), Some("UCtestChannel"));
        assert_eq!(parsed.view_count, Some(7942));

        // Description timestamp fallback when native macroMarkers are absent
        let fallback_json = serde_json::json!({
            "contents": {
                "twoColumnWatchNextResults": {
                    "results": {
                        "results": {
                            "contents": [
                                {
                                    "videoSecondaryInfoRenderer": {
                                        "attributedDescription": {
                                            "content": "Quick test\n00:00 Start\n01:00 End"
                                        }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });
        let fb = parse_innertube_video_details_json(&fallback_json, Some(120)).unwrap();
        assert_eq!(fb.chapters.len(), 2);
        assert_eq!(fb.chapters[0].title, "Start");
    }
}
