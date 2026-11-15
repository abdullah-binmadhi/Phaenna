//! Fast HTTP/RSS client for zero-subprocess YouTube channel feed refreshes.
//! Fetches public Atom XML feeds in <150ms for the 7-day Upload Radar & channel checks.

use crate::db::cache::{Thumbnail, VideoItem};
use crate::error::{AppError, Result};

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
}
