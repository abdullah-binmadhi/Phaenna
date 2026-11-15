//! Fast HTTP/RSS client for zero-subprocess YouTube channel feed refreshes.
//! Fetches public Atom XML feeds in <150ms for the 7-day Upload Radar & channel checks.

use crate::db::cache::{Thumbnail, VideoItem};
use crate::error::{AppError, Result};
use crate::services::auth::{compute_sapisidhash, now_unix_secs, GoogleAccountSession, PHAENNA_USER_AGENT};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, USER_AGENT};
use std::collections::HashSet;

const INNERTUBE_CLIENT_VERSION: &str = "2.20251015.01.00";

/// Map Phaenna internal feed keys onto YouTube InnerTube `browseId` targets.
pub fn browse_id_for_feed_key(feed_key: &str) -> Option<&'static str> {
    match feed_key {
        "recommended" => Some("FEwhat_to_watch"),
        "subscriptions" => Some("FEsubscriptions"),
        "history" => Some("FEhistory"),
        "watch_later" => Some("VLWL"),
        _ => None,
    }
}

/// Extracted InnerTube shelf page containing deduplicated videos and optional next continuation token.
#[derive(Debug, Clone, Default)]
pub struct InnerTubeShelfPage {
    pub videos: Vec<VideoItem>,
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

/// Recursively extract normalized `VideoItem`s and pagination token from any InnerTube browse JSON payload.
pub fn parse_innertube_browse_json(root: &serde_json::Value) -> InnerTubeShelfPage {
    let mut page = InnerTubeShelfPage::default();
    let mut seen = HashSet::new();
    collect_innertube_nodes(root, &mut page.videos, &mut seen, &mut page.continuation);
    page
}

fn collect_innertube_nodes(
    node: &serde_json::Value,
    out: &mut Vec<VideoItem>,
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
                            out.push(item);
                        }
                    }
                }
            }
            if let Some(lockup) = map.get("lockupViewModel") {
                if let Some(item) = extract_video_from_lockup(lockup) {
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
                collect_innertube_nodes(val, out, seen, continuation);
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                collect_innertube_nodes(item, out, seen, continuation);
            }
        }
        _ => {}
    }
}

fn extract_yt_text(val: Option<&serde_json::Value>) -> Option<String> {
    let obj = val?;
    if let Some(s) = obj.get("simpleText").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    if let Some(runs) = obj.get("runs").and_then(|v| v.as_array()) {
        let joined: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .collect();
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

fn parse_rough_views(raw: &str) -> Option<u64> {
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
        _ => 1.0,
    };
    Some((val * scale).round() as u64)
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

    let duration_string = extract_yt_text(r.get("lengthText"));
    let duration = duration_string.as_deref().and_then(parse_duration_text);

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

fn extract_video_from_lockup(l: &serde_json::Value) -> Option<VideoItem> {
    let id = l.get("contentId").and_then(|v| v.as_str())?.trim().to_string();
    if id.is_empty() {
        return None;
    }
    let title = l
        .pointer("/metadata/lockupMetadataViewModel/title/content")
        .and_then(|v| v.as_str())
        .unwrap_or("Untitled Video")
        .to_string();

    let rows = l
        .pointer("/metadata/lockupMetadataViewModel/metadata/contentMetadataViewModel/metadataRows")
        .and_then(|v| v.as_array());

    let mut uploader: Option<String> = None;
    let mut view_count: Option<u64> = None;
    let mut published_text: Option<String> = None;

    if let Some(rows_arr) = rows {
        for (idx, row) in rows_arr.iter().enumerate() {
            let Some(parts) = row.get("metadataParts").and_then(|p| p.as_array()) else {
                continue;
            };
            for part in parts {
                let Some(txt) = part.pointer("/text/content").and_then(|s| s.as_str()) else {
                    continue;
                };
                let t = txt.trim();
                if t.is_empty() {
                    continue;
                }
                let lower = t.to_ascii_lowercase();
                if lower.contains("view") || lower.contains("watching") {
                    if view_count.is_none() {
                        view_count = parse_rough_views(t);
                    }
                } else if lower.contains("ago")
                    || lower.contains("streamed")
                    || lower.contains("premiered")
                    || lower == "new"
                {
                    if published_text.is_none() {
                        published_text = Some(t.to_string());
                    }
                } else if idx == 0 && uploader.is_none() {
                    uploader = Some(t.to_string());
                }
            }
        }
    }

    let thumbnails = l
        .pointer("/contentImage/thumbnailViewModel/image/sources")
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
}
