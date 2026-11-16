//! Video and channel metadata SQLite cache (instant `<15ms` cold-boot feed reads).

use crate::error::Result;
use serde::{Deserialize, Serialize};
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Thumbnail {
    pub url: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct VideoChapter {
    pub title: String,
    pub start_time: f64,
    pub end_time: f64,
}

/// Canonical YouTube video model compatible with both `yt-dlp` JSON streams and SQLite cache.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VideoItem {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub uploader: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub duration: Option<u64>,
    #[serde(default)]
    pub duration_string: Option<String>,
    #[serde(default)]
    pub timestamp: Option<i64>,
    #[serde(default)]
    pub published_text: Option<String>,
    #[serde(default)]
    pub view_count: Option<u64>,
    #[serde(default)]
    pub like_count: Option<u64>,
    #[serde(default)]
    pub dislike_count: Option<u64>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub webpage_url: Option<String>,
    #[serde(default)]
    pub thumbnails: Option<Vec<Thumbnail>>,
    #[serde(default)]
    pub chapters: Option<Vec<VideoChapter>>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    pub is_live: Option<bool>,
}

#[allow(dead_code)]
impl VideoItem {
    pub fn clean_id(&self) -> &str {
        if self.id.starts_with("RD") && self.id.len() > 2 {
            &self.id[2..]
        } else {
            &self.id
        }
    }

    pub fn display_uploader(&self) -> &str {
        self.uploader
            .as_deref()
            .filter(|s| !s.is_empty())
            .or_else(|| self.channel.as_deref().filter(|s| !s.is_empty()))
            .unwrap_or("Unknown")
    }

    pub fn display_duration(&self) -> String {
        if let Some(ref s) = self.duration_string {
            if !s.is_empty() {
                return s.clone();
            }
        }
        if self.is_live == Some(true) {
            return "LIVE".to_string();
        }
        match self.duration {
            Some(secs) if secs > 0 => format_duration_secs(secs),
            _ => "LIVE".to_string(),
        }
    }

    pub fn best_thumbnail_url(&self) -> String {
        if let Some(ref t) = self.thumbnails {
            if let Some(th) = t
                .iter()
                .find(|th| th.url.contains("maxresdefault") || th.url.contains("hq720"))
            {
                return th.url.clone();
            }
            if let Some(th) = t
                .iter()
                .find(|th| th.url.contains("hqdefault") || th.url.contains("mqdefault"))
            {
                return th.url.clone();
            }
            if let Some(last) = t.last() {
                if !last.url.is_empty() {
                    return last.url.clone();
                }
            }
        }
        format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", self.clean_id())
    }

    pub fn watch_url(&self) -> String {
        self.webpage_url
            .clone()
            .filter(|u| !u.is_empty())
            .unwrap_or_else(|| format!("https://www.youtube.com/watch?v={}", self.clean_id()))
    }

    pub fn format_views(&self) -> String {
        match self.view_count {
            Some(v) if v >= 1_000_000_000 => format!("{:.1}B views", v as f64 / 1e9),
            Some(v) if v >= 1_000_000 => format!("{:.1}M views", v as f64 / 1e6),
            Some(v) if v >= 1_000 => format!("{:.1}K views", v as f64 / 1e3),
            Some(v) => format!("{} views", v),
            None => String::new(),
        }
    }

    pub fn compact_views(&self, include_label: bool) -> Option<String> {
        let v = self.view_count?;
        let num = if v >= 1_000_000_000 {
            format_trimmed_unit(v as f64 / 1e9, "B")
        } else if v >= 1_000_000 {
            format_trimmed_unit(v as f64 / 1e6, "M")
        } else if v >= 1_000 {
            format_trimmed_unit(v as f64 / 1e3, "K")
        } else {
            v.to_string()
        };
        if include_label {
            Some(format!("{} views", num))
        } else {
            Some(num)
        }
    }

    pub fn is_new_release(&self, now_unix: i64) -> bool {
        if let Some(ts) = self.timestamp {
            let age = now_unix.saturating_sub(ts);
            if ts > 0 && (0..=48 * 3600).contains(&age) {
                return true;
            }
        }
        if let Some(ref txt) = self.published_text {
            let lower = txt.to_ascii_lowercase();
            if lower.contains("just now")
                || lower.contains("second")
                || lower.contains("minute")
                || lower.contains("hour")
                || lower.contains("1 day ago")
                || lower.contains("2 days ago")
                || lower.contains("today")
                || lower == "new"
            {
                return true;
            }
        }
        false
    }

    pub fn compact_age(&self, now_unix: i64, long_suffix: bool) -> Option<String> {
        if let Some(ts) = self.timestamp {
            if ts > 0 && now_unix >= ts {
                let delta = now_unix.saturating_sub(ts);
                let s = if delta < 60 {
                    "Just now".to_string()
                } else if delta < 3600 {
                    format!("{}m", delta / 60)
                } else if delta < 86_400 {
                    format!("{}h", delta / 3600)
                } else if delta < 14 * 86_400 {
                    format!("{}d", delta / 86_400)
                } else if delta < 60 * 86_400 {
                    format!("{}w", delta / (7 * 86_400))
                } else if delta < 365 * 86_400 {
                    format!("{}mo", (delta / (30 * 86_400)).max(1))
                } else {
                    format!("{}y", delta / (365 * 86_400))
                };
                if s == "Just now" {
                    return Some(s);
                }
                return Some(if long_suffix { format!("{} ago", s) } else { s });
            }
        }
        if let Some(ref txt) = self.published_text {
            let cleaned = shorten_published_text(txt, long_suffix);
            if !cleaned.is_empty() {
                return Some(cleaned);
            }
        }
        None
    }

    pub fn effective_chapters(&self) -> Vec<VideoChapter> {
        if let Some(ref ch) = self.chapters {
            if !ch.is_empty() {
                return ch.clone();
            }
        }
        if let Some(ref desc) = self.description {
            let parsed = crate::api::ytdlp::parse_description_chapters(
                desc,
                self.duration.unwrap_or(0) as f64,
            );
            if !parsed.is_empty() {
                return parsed;
            }
        }
        Vec::new()
    }

    pub fn like_ratio_pct(&self) -> Option<f64> {
        let likes = self.like_count?;
        let dislikes = self.dislike_count?;
        let total = likes + dislikes;
        if total == 0 {
            None
        } else {
            Some((likes as f64 / total as f64) * 100.0)
        }
    }

    pub fn chapter_list(&self) -> Vec<VideoChapter> {
        self.chapters.clone().unwrap_or_default()
    }

    pub fn tag_list(&self) -> Vec<String> {
        self.tags.clone().unwrap_or_default()
    }

    pub fn is_shorts(&self) -> bool {
        self.duration.map(|d| d <= 60).unwrap_or(false)
    }
}

fn format_trimmed_unit(val: f64, unit: &str) -> String {
    if val >= 100.0 || (val - val.round()).abs() < 0.05 {
        format!("{:.0}{}", val.round(), unit)
    } else {
        format!("{:.1}{}", val, unit)
    }
}

fn shorten_published_text(raw: &str, long_suffix: bool) -> String {
    let mut s = raw
        .trim()
        .replace("Streamed ", "")
        .replace("Premiered ", "")
        .replace("Scheduled for ", "");
    let replacements = [
        (" seconds ago", "s"),
        (" second ago", "s"),
        (" minutes ago", "m"),
        (" minute ago", "m"),
        (" hours ago", "h"),
        (" hour ago", "h"),
        (" days ago", "d"),
        (" day ago", "d"),
        (" weeks ago", "w"),
        (" week ago", "w"),
        (" months ago", "mo"),
        (" month ago", "mo"),
        (" years ago", "y"),
        (" year ago", "y"),
    ];
    for (pat, short) in replacements {
        if let Some(pos) = s.to_ascii_lowercase().find(pat) {
            let prefix = s[..pos].trim();
            if long_suffix {
                return format!("{prefix}{short} ago");
            }
            return format!("{prefix}{short}");
        }
    }
    s = s.trim().to_string();
    s
}

pub fn format_duration_secs(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}:{:02}:{:02}", h, m, s)
    } else {
        format!("{}:{:02}", m, s)
    }
}

pub async fn execute_upsert_video<'e, E>(
    executor: E,
    v: &VideoItem,
    now: i64,
) -> Result<()>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let chapters_json = serde_json::to_string(&v.chapters.as_deref().unwrap_or(&[]))
        .unwrap_or_else(|_| "[]".to_string());
    let tags_json = serde_json::to_string(&v.tags.as_deref().unwrap_or(&[]))
        .unwrap_or_else(|_| "[]".to_string());
    let thumbs_json = serde_json::to_string(&v.thumbnails.as_deref().unwrap_or(&[]))
        .unwrap_or_else(|_| "[]".to_string());
    let thumb_url = v.best_thumbnail_url();

    sqlx::query(
        "INSERT INTO videos (
            id, channel_id, channel_name, title, description, duration_secs, duration_text,
            view_count, like_count, dislike_count, published_at, published_text, webpage_url,
            chapters_json, tags_json, thumbnails_json, thumb_url, is_live, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(id) DO UPDATE SET
            channel_id      = COALESCE(excluded.channel_id, videos.channel_id),
            channel_name    = CASE WHEN excluded.channel_name != 'Unknown' THEN excluded.channel_name ELSE videos.channel_name END,
            title           = CASE WHEN excluded.title != '' THEN excluded.title ELSE videos.title END,
            description     = COALESCE(excluded.description, videos.description),
            duration_secs   = COALESCE(excluded.duration_secs, videos.duration_secs),
            duration_text   = COALESCE(excluded.duration_text, videos.duration_text),
            view_count      = COALESCE(excluded.view_count, videos.view_count),
            like_count      = COALESCE(excluded.like_count, videos.like_count),
            dislike_count   = COALESCE(excluded.dislike_count, videos.dislike_count),
            published_at    = CASE WHEN excluded.published_text IS NOT NULL AND excluded.published_at IS NOT NULL THEN excluded.published_at ELSE COALESCE(excluded.published_at, videos.published_at) END,
            published_text  = COALESCE(excluded.published_text, videos.published_text),
            webpage_url     = COALESCE(excluded.webpage_url, videos.webpage_url),
            chapters_json   = CASE WHEN excluded.chapters_json != '[]' THEN excluded.chapters_json ELSE videos.chapters_json END,
            tags_json       = CASE WHEN excluded.tags_json != '[]' THEN excluded.tags_json ELSE videos.tags_json END,
            thumbnails_json = CASE WHEN excluded.thumbnails_json != '[]' THEN excluded.thumbnails_json ELSE videos.thumbnails_json END,
            thumb_url       = COALESCE(excluded.thumb_url, videos.thumb_url),
            is_live         = excluded.is_live,
            updated_at      = excluded.updated_at",
    )
    .bind(v.clean_id())
    .bind(&v.channel_id)
    .bind(v.display_uploader())
    .bind(&v.title)
    .bind(&v.description)
    .bind(v.duration.map(|d| d as i64))
    .bind(&v.duration_string)
    .bind(v.view_count.map(|c| c as i64))
    .bind(v.like_count.map(|c| c as i64))
    .bind(v.dislike_count.map(|c| c as i64))
    .bind(v.timestamp)
    .bind(&v.published_text)
    .bind(&v.webpage_url)
    .bind(chapters_json)
    .bind(tags_json)
    .bind(thumbs_json)
    .bind(thumb_url)
    .bind(if v.is_live == Some(true) { 1i64 } else { 0i64 })
    .bind(now)
    .execute(executor)
    .await?;

    Ok(())
}

pub async fn upsert_video(pool: &SqlitePool, v: &VideoItem, now: i64) -> Result<()> {
    execute_upsert_video(pool, v, now).await
}

/// Atomically store an entire feed slice into SQLite in a single transaction.
pub async fn batch_upsert_feed(
    pool: &SqlitePool,
    feed_key: &str,
    videos: &[VideoItem],
    now: i64,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM feed_entries WHERE feed_key = ?")
        .bind(feed_key)
        .execute(&mut *tx)
        .await?;

    for (pos, v) in videos.iter().enumerate() {
        execute_upsert_video(&mut *tx, v, now).await?;
        sqlx::query(
            "INSERT OR REPLACE INTO feed_entries (feed_key, video_id, position) VALUES (?, ?, ?)",
        )
        .bind(feed_key)
        .bind(v.clean_id())
        .bind(pos as i64)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub(crate) fn row_to_video(row: &sqlx::sqlite::SqliteRow) -> VideoItem {
    let id: String = row.try_get("id").unwrap_or_default();
    let channel_id: Option<String> = row.try_get("channel_id").unwrap_or(None);
    let channel_name: String = row.try_get("channel_name").unwrap_or_else(|_| "Unknown".into());
    let title: String = row.try_get("title").unwrap_or_default();
    let description: Option<String> = row.try_get("description").unwrap_or(None);
    let duration: Option<i64> = row.try_get("duration_secs").unwrap_or(None);
    let duration_string: Option<String> = row.try_get("duration_text").unwrap_or(None);
    let view_count: Option<i64> = row.try_get("view_count").unwrap_or(None);
    let like_count: Option<i64> = row.try_get("like_count").unwrap_or(None);
    let dislike_count: Option<i64> = row.try_get("dislike_count").unwrap_or(None);
    let timestamp: Option<i64> = row.try_get("published_at").unwrap_or(None);
    let published_text: Option<String> = row.try_get("published_text").unwrap_or(None);
    let webpage_url: Option<String> = row.try_get("webpage_url").unwrap_or(None);
    let chapters_raw: String = row.try_get("chapters_json").unwrap_or_else(|_| "[]".into());
    let tags_raw: String = row.try_get("tags_json").unwrap_or_else(|_| "[]".into());
    let thumbs_raw: String = row.try_get("thumbnails_json").unwrap_or_else(|_| "[]".into());
    let is_live_num: i64 = row.try_get("is_live").unwrap_or(0);

    let chapters: Option<Vec<VideoChapter>> =
        serde_json::from_str::<Vec<VideoChapter>>(&chapters_raw)
            .ok()
            .filter(|c| !c.is_empty());
    let tags: Option<Vec<String>> = serde_json::from_str::<Vec<String>>(&tags_raw)
        .ok()
        .filter(|t| !t.is_empty());
    let thumbnails: Option<Vec<Thumbnail>> = serde_json::from_str::<Vec<Thumbnail>>(&thumbs_raw)
        .ok()
        .filter(|t| !t.is_empty());

    VideoItem {
        id,
        title,
        uploader: Some(channel_name.clone()),
        channel: Some(channel_name),
        channel_id,
        duration: duration.map(|d| d.max(0) as u64),
        duration_string,
        timestamp,
        published_text,
        view_count: view_count.map(|c| c.max(0) as u64),
        like_count: like_count.map(|c| c.max(0) as u64),
        dislike_count: dislike_count.map(|c| c.max(0) as u64),
        description,
        webpage_url,
        thumbnails,
        chapters,
        tags,
        is_live: Some(is_live_num != 0),
    }
}

pub async fn get_feed_videos(pool: &SqlitePool, feed_key: &str) -> Result<Vec<VideoItem>> {
    let rows = sqlx::query(
        "SELECT v.* FROM videos v
         JOIN feed_entries f ON v.id = f.video_id
         WHERE f.feed_key = ?
         ORDER BY f.position ASC",
    )
    .bind(feed_key)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(row_to_video).collect())
}

#[allow(dead_code)]
pub async fn get_video(pool: &SqlitePool, id: &str) -> Result<Option<VideoItem>> {
    let clean = id.trim_start_matches("RD");
    let row = sqlx::query("SELECT * FROM videos WHERE id = ?")
        .bind(clean)
        .fetch_optional(pool)
        .await?;
    Ok(row.as_ref().map(row_to_video))
}

pub async fn get_all_cached_videos(pool: &SqlitePool, limit: i64) -> Result<Vec<VideoItem>> {
    let rows = sqlx::query("SELECT * FROM videos ORDER BY updated_at DESC LIMIT ?")
        .bind(limit)
        .fetch_all(pool)
        .await?;
    Ok(rows.iter().map(row_to_video).collect())
}

pub async fn store_thumb_blob(pool: &SqlitePool, id: &str, blob: &[u8]) -> Result<()> {
    let clean = id.trim_start_matches("RD");
    sqlx::query("UPDATE videos SET thumb_blob = ? WHERE id = ?")
        .bind(blob)
        .bind(clean)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_thumb_blob(pool: &SqlitePool, id: &str) -> Result<Option<Vec<u8>>> {
    let clean = id.trim_start_matches("RD");
    let blob: Option<Option<Vec<u8>>> =
        sqlx::query_scalar("SELECT thumb_blob FROM videos WHERE id = ?")
            .bind(clean)
            .fetch_optional(pool)
            .await?;
    Ok(blob.flatten().filter(|b| !b.is_empty()))
}

pub async fn search_cached_videos(pool: &SqlitePool, query: &str) -> Result<Vec<VideoItem>> {
    let pat = format!("%{}%", query.trim());
    let rows = sqlx::query(
        "SELECT * FROM videos
         WHERE title LIKE ? OR channel_name LIKE ? OR tags_json LIKE ?
         ORDER BY COALESCE(view_count, 0) DESC, updated_at DESC
         LIMIT 25",
    )
    .bind(&pat)
    .bind(&pat)
    .bind(&pat)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_video).collect())
}

pub type VideoStatsUpdate = (String, Option<String>, Option<u64>, Option<String>, Option<i64>);

pub async fn enrich_video_stats(pool: &SqlitePool, updates: &[VideoStatsUpdate]) -> Result<()> {
    if updates.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for (id, uploader, view_count, published_text, timestamp) in updates {
        let clean = id.trim_start_matches("RD");
        sqlx::query(
            "UPDATE videos SET
                channel_name = CASE WHEN ? IS NOT NULL AND (channel_name = 'Unknown' OR channel_name = '') THEN ? ELSE channel_name END,
                view_count = COALESCE(?, view_count),
                published_text = COALESCE(?, published_text),
                published_at = COALESCE(?, published_at)
             WHERE id = ?",
        )
        .bind(uploader.as_deref())
        .bind(uploader.as_deref())
        .bind(view_count.map(|c| c as i64))
        .bind(published_text.as_deref())
        .bind(*timestamp)
        .bind(clean)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init;

    fn sample_video(id: &str, title: &str, ch: &str) -> VideoItem {
        VideoItem {
            id: id.to_string(),
            title: title.to_string(),
            uploader: Some(ch.to_string()),
            channel: Some(ch.to_string()),
            duration: Some(920),
            view_count: Some(250_000),
            like_count: Some(950),
            dislike_count: Some(50),
            tags: Some(vec!["rust".to_string(), "systems".to_string()]),
            ..Default::default()
        }
    }

    #[test]
    fn test_video_item_helpers() {
        let v = sample_video("RDabc12345678", "Systems Rust", "Jon Gjengset");
        assert_eq!(v.clean_id(), "abc12345678");
        assert_eq!(v.display_uploader(), "Jon Gjengset");
        assert_eq!(v.display_duration(), "15:20");
        assert_eq!(v.format_views(), "250.0K views");
        assert_eq!(v.compact_views(true).as_deref(), Some("250K views"));
        assert!((v.like_ratio_pct().unwrap() - 95.0).abs() < 0.01);
        assert!(v.best_thumbnail_url().contains("abc12345678"));

        let now = 1_763_200_000;
        let fresh = VideoItem {
            timestamp: Some(now - 3600 * 5),
            published_text: Some("5 hours ago".to_string()),
            ..v.clone()
        };
        assert!(fresh.is_new_release(now));
        assert_eq!(fresh.compact_age(now, true).as_deref(), Some("5h ago"));
        assert_eq!(fresh.compact_age(now, false).as_deref(), Some("5h"));

        let older = VideoItem {
            timestamp: Some(now - 86_400 * 10),
            published_text: Some("1 week ago".to_string()),
            description: Some("0:00 Intro\n1:30 Setup\n5:00 Wrap".to_string()),
            ..v
        };
        assert!(!older.is_new_release(now));
        assert_eq!(older.compact_age(now, true).as_deref(), Some("10d ago"));
        let ch = older.effective_chapters();
        assert_eq!(ch.len(), 3);
        assert_eq!(ch[1].title, "Setup");
    }

    #[tokio::test]
    async fn test_feed_batch_upsert_blob_and_search() {
        let pool = init(":memory:").await.unwrap();
        let vids = vec![
            sample_video("vid00000001", "Async Rust Deep Dive", "No Boilerplate"),
            sample_video("vid00000002", "Zig vs Rust Arena Allocators", "SystemsLab"),
        ];

        batch_upsert_feed(&pool, "recommended", &vids, 1_700_000_000)
            .await
            .unwrap();
        let loaded = get_feed_videos(&pool, "recommended").await.unwrap();
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].id, "vid00000001");

        store_thumb_blob(&pool, "vid00000001", &[1, 2, 3, 4])
            .await
            .unwrap();
        assert_eq!(
            get_thumb_blob(&pool, "vid00000001").await.unwrap().unwrap(),
            vec![1, 2, 3, 4]
        );

        let hits = search_cached_videos(&pool, "Arena").await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].id, "vid00000002");
    }
}
