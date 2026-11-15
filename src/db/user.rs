//! User library state: watch history, resume positions, Watch Later with provenance,
//! SponsorBlock segment cache, offline vault records, and YouTube Wrapped analytics.

use crate::db::cache::{execute_upsert_video, VideoItem};
use crate::error::Result;
use sqlx::{Row, SqlitePool};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WatchLaterSource {
    YouTube,
    Local,
    Both,
}

#[allow(dead_code)]
impl WatchLaterSource {
    pub fn from_str_tag(raw: &str) -> Self {
        match raw {
            "youtube" => Self::YouTube,
            "both" => Self::Both,
            _ => Self::Local,
        }
    }

    pub fn pill_tag(self) -> &'static str {
        match self {
            Self::YouTube => "YT",
            Self::Local => "LOC",
            Self::Both => "YT+L",
        }
    }

    pub fn divider_label(self) -> &'static str {
        match self {
            Self::YouTube => "YOUTUBE WL",
            Self::Both => "BOTH",
            Self::Local => "LOCAL QUEUE",
        }
    }
}

#[derive(Debug, Clone)]
pub struct SponsorSegmentRow {
    pub segment_uuid: String,
    pub category: String,
    pub start_secs: f64,
    pub end_secs: f64,
}

#[derive(Debug, Clone)]
pub struct TopChannelEntry {
    pub channel_name: String,
    pub video_count: usize,
    pub total_hours: f64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RecentWatchItem {
    pub video_id: String,
    pub title: String,
    pub channel_name: String,
    pub watched_at: i64,
}

#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
pub struct ContinueWatchingMeta {
    pub video_id: String,
    pub last_pos_secs: f64,
    pub duration_secs: f64,
    pub pinned_manually: bool,
    pub queue_order: i64,
    pub last_watched_at: i64,
    pub added_at: i64,
}

/// Aggregated personal telemetry powering `[a] YouTube Wrapped & Viewing Analytics`.
#[derive(Debug, Clone)]
pub struct UserAnalytics {
    pub total_videos_watched: usize,
    pub total_hours_watched: f64,
    pub in_progress_videos: usize,
    pub completion_rate: f64,
    pub watch_archetype: &'static str,
    pub viewing_cadence: &'static str,
    pub active_streak_days: usize,
    pub longest_streak_days: usize,
    pub peak_hour: usize,
    pub hourly_distribution: [u64; 24],
    pub weekday_distribution: [u64; 7],
    pub thermal_activity: Vec<u64>,       // 182 days ending today
    pub cumulative_hours_30d: Vec<f64>,   // 30-day cumulative curve
    pub duration_buckets: [(&'static str, usize, f64); 4], // [<5m, 5-20m, 20-60m, 60m+]
    pub top_channels: Vec<TopChannelEntry>,
    pub top_tags: Vec<(String, usize, f64)>,
    pub sponsorblock_saved_secs: f64,
    pub offline_cached_videos: usize,
    pub offline_cached_bytes: u64,
    pub watch_later_count: usize,
    pub recent_watch_log: Vec<RecentWatchItem>,
}

impl Default for UserAnalytics {
    fn default() -> Self {
        Self {
            total_videos_watched: 0,
            total_hours_watched: 0.0,
            in_progress_videos: 0,
            completion_rate: 0.0,
            watch_archetype: "Zen Explorer",
            viewing_cadence: "Afternoon Flow (12:00 - 18:00)",
            active_streak_days: 0,
            longest_streak_days: 0,
            peak_hour: 20,
            hourly_distribution: [0; 24],
            weekday_distribution: [0; 7],
            thermal_activity: vec![0; 182],
            cumulative_hours_30d: vec![0.0; 30],
            duration_buckets: [
                ("<5m Shorts/Clips", 0, 0.0),
                ("5-20m Essays", 0, 0.0),
                ("20-60m Deep Dives", 0, 0.0),
                ("60m+ Longform", 0, 0.0),
            ],
            top_channels: Vec::new(),
            top_tags: Vec::new(),
            sponsorblock_saved_secs: 0.0,
            offline_cached_videos: 0,
            offline_cached_bytes: 0,
            watch_later_count: 0,
            recent_watch_log: Vec::new(),
        }
    }
}

pub async fn touch_watch_history(
    pool: &SqlitePool,
    video: Option<&VideoItem>,
    video_id: &str,
    now: i64,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    let mut tx = pool.begin().await?;
    if let Some(v) = video {
        execute_upsert_video(&mut *tx, v, now).await?;
    }
    sqlx::query(
        "INSERT INTO watch_history (video_id, watched_at) VALUES (?, ?)
         ON CONFLICT(video_id) DO UPDATE SET watched_at = excluded.watched_at",
    )
    .bind(clean)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn record_watched(
    pool: &SqlitePool,
    video: Option<&VideoItem>,
    video_id: &str,
    now: i64,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    let mut tx = pool.begin().await?;
    if let Some(v) = video {
        execute_upsert_video(&mut *tx, v, now).await?;
    }
    sqlx::query(
        "INSERT INTO watch_history (video_id, watched_at) VALUES (?, ?)
         ON CONFLICT(video_id) DO UPDATE SET watched_at = excluded.watched_at",
    )
    .bind(clean)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM continue_watching WHERE video_id = ? AND pinned_manually = 0")
        .bind(clean)
        .execute(&mut *tx)
        .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn get_watched_ids(pool: &SqlitePool) -> Result<HashSet<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT video_id FROM watch_history")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

#[allow(dead_code)]
pub async fn is_watched(pool: &SqlitePool, video_id: &str) -> Result<bool> {
    let clean = video_id.trim_start_matches("RD");
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM watch_history WHERE video_id = ?")
            .bind(clean)
            .fetch_optional(pool)
            .await?;
    Ok(exists.is_some())
}

pub async fn save_playback_progress(
    pool: &SqlitePool,
    video_id: &str,
    position_secs: f64,
    duration_secs: f64,
    now: i64,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO playback_progress (video_id, position_secs, duration_secs, updated_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT(video_id) DO UPDATE SET
            position_secs = excluded.position_secs,
            duration_secs = CASE WHEN excluded.duration_secs > 0.0 THEN excluded.duration_secs ELSE playback_progress.duration_secs END,
            updated_at    = excluded.updated_at",
    )
    .bind(clean)
    .bind(position_secs)
    .bind(duration_secs)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    if position_secs >= 10.0 && (duration_secs <= 0.0 || (position_secs / duration_secs) < 0.85) {
        sqlx::query(
            "INSERT INTO continue_watching (video_id, last_pos_secs, duration_secs, last_watched_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(video_id) DO UPDATE SET
                last_pos_secs   = excluded.last_pos_secs,
                duration_secs   = excluded.duration_secs,
                last_watched_at = excluded.last_watched_at",
        )
        .bind(clean)
        .bind(position_secs)
        .bind(duration_secs)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn get_playback_progress(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<Option<(f64, f64)>> {
    let clean = video_id.trim_start_matches("RD");
    let row: Option<(f64, f64)> = sqlx::query_as(
        "SELECT position_secs, duration_secs FROM playback_progress WHERE video_id = ?",
    )
    .bind(clean)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn get_all_progress_map(pool: &SqlitePool) -> Result<HashMap<String, (f64, f64)>> {
    let rows: Vec<(String, f64, f64)> = sqlx::query_as(
        "SELECT video_id, position_secs, duration_secs FROM playback_progress",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, p, d)| (id, (p, d)))
        .collect())
}

pub async fn get_continue_watching(pool: &SqlitePool) -> Result<Vec<VideoItem>> {
    let rows = sqlx::query(
        "SELECT v.id, v.channel_id, v.channel_name, v.title, v.description,
                v.duration_secs, v.duration_text, v.view_count, v.like_count,
                v.dislike_count, v.published_at, v.published_text, v.webpage_url,
                v.chapters_json, v.tags_json, v.thumbnails_json, v.is_live
         FROM continue_watching cw
         JOIN videos v ON cw.video_id = v.id
         ORDER BY cw.pinned_manually DESC,
                  CASE WHEN cw.queue_order > 0 THEN cw.queue_order ELSE 999999 END ASC,
                  cw.last_watched_at DESC",
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .iter()
        .map(|row| VideoItem {
            id: row.try_get("id").unwrap_or_default(),
            title: row.try_get("title").unwrap_or_default(),
            uploader: row.try_get("channel_name").ok(),
            channel: row.try_get("channel_name").ok(),
            channel_id: row.try_get("channel_id").unwrap_or(None),
            duration: row
                .try_get::<Option<i64>, _>("duration_secs")
                .unwrap_or(None)
                .map(|d| d.max(0) as u64),
            duration_string: row.try_get("duration_text").unwrap_or(None),
            view_count: row
                .try_get::<Option<i64>, _>("view_count")
                .unwrap_or(None)
                .map(|c| c.max(0) as u64),
            like_count: row
                .try_get::<Option<i64>, _>("like_count")
                .unwrap_or(None)
                .map(|c| c.max(0) as u64),
            dislike_count: row
                .try_get::<Option<i64>, _>("dislike_count")
                .unwrap_or(None)
                .map(|c| c.max(0) as u64),
            timestamp: row.try_get("published_at").unwrap_or(None),
            published_text: row.try_get("published_text").unwrap_or(None),
            description: row.try_get("description").unwrap_or(None),
            webpage_url: row.try_get("webpage_url").unwrap_or(None),
            thumbnails: serde_json::from_str(
                &row.try_get::<String, _>("thumbnails_json")
                    .unwrap_or_else(|_| "[]".into()),
            )
            .ok(),
            chapters: serde_json::from_str(
                &row.try_get::<String, _>("chapters_json")
                    .unwrap_or_else(|_| "[]".into()),
            )
            .ok(),
            tags: serde_json::from_str(
                &row.try_get::<String, _>("tags_json")
                    .unwrap_or_else(|_| "[]".into()),
            )
            .ok(),
            is_live: Some(row.try_get::<i64, _>("is_live").unwrap_or(0) != 0),
        })
        .collect())
}

pub async fn get_continue_watching_meta_map(
    pool: &SqlitePool,
) -> Result<HashMap<String, ContinueWatchingMeta>> {
    let rows = sqlx::query(
        "SELECT video_id, last_pos_secs, duration_secs, pinned_manually,
                queue_order, last_watched_at, added_at
         FROM continue_watching",
    )
    .fetch_all(pool)
    .await?;

    let mut out = HashMap::new();
    for row in rows {
        let video_id: String = row.try_get("video_id").unwrap_or_default();
        if video_id.is_empty() {
            continue;
        }
        let meta = ContinueWatchingMeta {
            video_id: video_id.clone(),
            last_pos_secs: row.try_get("last_pos_secs").unwrap_or(0.0),
            duration_secs: row.try_get("duration_secs").unwrap_or(0.0),
            pinned_manually: row.try_get::<i64, _>("pinned_manually").unwrap_or(0) != 0,
            queue_order: row.try_get("queue_order").unwrap_or(0),
            last_watched_at: row.try_get("last_watched_at").unwrap_or(0),
            added_at: row.try_get("added_at").unwrap_or(0),
        };
        out.insert(video_id, meta);
    }
    Ok(out)
}

pub async fn pin_to_continue_watching(
    pool: &SqlitePool,
    video: &VideoItem,
    now: i64,
) -> Result<()> {
    let clean = video.clean_id();
    let mut tx = pool.begin().await?;
    execute_upsert_video(&mut *tx, video, now).await?;
    let dur = video.duration.unwrap_or(0) as f64;
    let next_order: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(queue_order), 0) + 1 FROM continue_watching",
    )
    .fetch_one(&mut *tx)
    .await
    .unwrap_or(1);

    sqlx::query(
        "INSERT INTO continue_watching (
            video_id, last_pos_secs, duration_secs, last_watched_at,
            pinned_manually, queue_order, added_at
         ) VALUES (?, 0.0, ?, ?, 1, ?, ?)
         ON CONFLICT(video_id) DO UPDATE SET
            pinned_manually = 1,
            duration_secs = CASE WHEN continue_watching.duration_secs > 0.0 THEN continue_watching.duration_secs ELSE excluded.duration_secs END,
            last_watched_at = excluded.last_watched_at,
            added_at = CASE WHEN continue_watching.added_at > 0 THEN continue_watching.added_at ELSE excluded.added_at END",
    )
    .bind(clean)
    .bind(dur)
    .bind(now)
    .bind(next_order)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn move_continue_watching_item(
    pool: &SqlitePool,
    video_id: &str,
    direction: isize,
) -> Result<bool> {
    let clean = video_id.trim_start_matches("RD");
    let current = get_continue_watching(pool).await?;
    let mut ids: Vec<String> = current.into_iter().map(|v| v.clean_id().to_string()).collect();
    let Some(pos) = ids.iter().position(|id| id == clean) else {
        return Ok(false);
    };
    let target = (pos as isize + direction).clamp(0, ids.len().saturating_sub(1) as isize) as usize;
    if target == pos {
        return Ok(false);
    }
    ids.swap(pos, target);

    let mut tx = pool.begin().await?;
    for (idx, id) in ids.iter().enumerate() {
        sqlx::query("UPDATE continue_watching SET queue_order = ? WHERE video_id = ?")
            .bind((idx + 1) as i64)
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(true)
}

pub async fn clear_finished_continue_watching(pool: &SqlitePool) -> Result<usize> {
    let res = sqlx::query(
        "DELETE FROM continue_watching
         WHERE video_id IN (SELECT video_id FROM watch_history)
            OR (duration_secs > 0.0 AND (last_pos_secs / duration_secs) >= 0.85)",
    )
    .execute(pool)
    .await?;
    Ok(res.rows_affected() as usize)
}

pub async fn remove_continue_watching(pool: &SqlitePool, video_id: &str) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    sqlx::query("DELETE FROM continue_watching WHERE video_id = ?")
        .bind(clean)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn add_to_watch_later(
    pool: &SqlitePool,
    video: &VideoItem,
    source: WatchLaterSource,
    now: i64,
) -> Result<()> {
    let mut tx = pool.begin().await?;
    execute_upsert_video(&mut *tx, video, now).await?;
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT source FROM watch_later WHERE video_id = ?")
            .bind(video.clean_id())
            .fetch_optional(&mut *tx)
            .await?;

    let merged_source = match (existing.as_ref().map(|e| e.0.as_str()), source) {
        (Some("youtube"), WatchLaterSource::Local) | (Some("local"), WatchLaterSource::YouTube) | (Some("both"), _) => "both",
        (_, WatchLaterSource::Both) => "both",
        (_, WatchLaterSource::YouTube) => "youtube",
        _ => "local",
    };

    sqlx::query(
        "INSERT INTO watch_later (video_id, source, added_at) VALUES (?, ?, ?)
         ON CONFLICT(video_id) DO UPDATE SET source = excluded.source",
    )
    .bind(video.clean_id())
    .bind(merged_source)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn remove_from_watch_later(pool: &SqlitePool, video_id: &str) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    sqlx::query("DELETE FROM watch_later WHERE video_id = ?")
        .bind(clean)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn get_watch_later_sources(pool: &SqlitePool) -> Result<HashMap<String, WatchLaterSource>> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT video_id, source FROM watch_later")
        .fetch_all(pool)
        .await?;
    Ok(rows
        .into_iter()
        .map(|(id, src)| (id, WatchLaterSource::from_str_tag(&src)))
        .collect())
}

pub async fn save_sponsorblock_segments(
    pool: &SqlitePool,
    video_id: &str,
    segments: &[SponsorSegmentRow],
    now: i64,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM sponsorblock_segments WHERE video_id = ?")
        .bind(clean)
        .execute(&mut *tx)
        .await?;
    for seg in segments {
        sqlx::query(
            "INSERT INTO sponsorblock_segments
             (video_id, segment_uuid, category, start_secs, end_secs, fetched_at)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(clean)
        .bind(&seg.segment_uuid)
        .bind(&seg.category)
        .bind(seg.start_secs)
        .bind(seg.end_secs)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

pub async fn get_sponsorblock_segments(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<Vec<SponsorSegmentRow>> {
    let clean = video_id.trim_start_matches("RD");
    let rows: Vec<(String, String, f64, f64)> = sqlx::query_as(
        "SELECT segment_uuid, category, start_secs, end_secs
         FROM sponsorblock_segments WHERE video_id = ? ORDER BY start_secs ASC",
    )
    .bind(clean)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(segment_uuid, category, start_secs, end_secs)| SponsorSegmentRow {
            segment_uuid,
            category,
            start_secs,
            end_secs,
        })
        .collect())
}

pub async fn save_offline_video(
    pool: &SqlitePool,
    video_id: &str,
    file_path: &str,
    file_bytes: u64,
    format_note: &str,
    now: i64,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    sqlx::query(
        "INSERT INTO offline_videos (video_id, file_path, file_bytes, format_note, downloaded_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(video_id) DO UPDATE SET
            file_path = excluded.file_path,
            file_bytes = excluded.file_bytes,
            format_note = excluded.format_note,
            downloaded_at = excluded.downloaded_at",
    )
    .bind(clean)
    .bind(file_path)
    .bind(file_bytes as i64)
    .bind(format_note)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_offline_videos(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let rows: Vec<(String, String)> =
        sqlx::query_as("SELECT video_id, file_path FROM offline_videos")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

#[allow(dead_code)]
pub async fn delete_offline_video(pool: &SqlitePool, video_id: &str) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    sqlx::query("DELETE FROM offline_videos WHERE video_id = ?")
        .bind(clean)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn add_sponsorblock_saved_secs(pool: &SqlitePool, secs: f64) -> Result<()> {
    if secs <= 0.0 {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO telemetry_counters (key, value_f64) VALUES ('sponsorblock_saved_secs', ?)
         ON CONFLICT(key) DO UPDATE SET value_f64 = value_f64 + excluded.value_f64",
    )
    .bind(secs)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_sponsorblock_saved_secs(pool: &SqlitePool) -> Result<f64> {
    let val: Option<f64> = sqlx::query_scalar(
        "SELECT value_f64 FROM telemetry_counters WHERE key = 'sponsorblock_saved_secs'",
    )
    .fetch_optional(pool)
    .await?;
    Ok(val.unwrap_or(0.0))
}

/// Compute comprehensive personal telemetry for the `[a] YouTube Wrapped` Bento dashboard.
pub async fn compute_user_analytics(pool: &SqlitePool) -> Result<UserAnalytics> {
    let mut out = UserAnalytics::default();

    let total_watched: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM watch_history")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    out.total_videos_watched = total_watched.max(0) as usize;

    let in_prog: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM continue_watching")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    out.in_progress_videos = in_prog.max(0) as usize;

    let denom = out.total_videos_watched + out.in_progress_videos;
    out.completion_rate = if denom > 0 {
        (out.total_videos_watched as f64 / denom as f64) * 100.0
    } else {
        0.0
    };

    out.sponsorblock_saved_secs = get_sponsorblock_saved_secs(pool).await.unwrap_or(0.0);

    let (offline_cnt, offline_bytes): (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*), COALESCE(SUM(file_bytes), 0) FROM offline_videos",
    )
    .fetch_one(pool)
    .await
    .unwrap_or((0, 0));
    out.offline_cached_videos = offline_cnt.max(0) as usize;
    out.offline_cached_bytes = offline_bytes.max(0) as u64;

    let wl_cnt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM watch_later")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    out.watch_later_count = wl_cnt.max(0) as usize;

    // Fetch watched video rows paired with playback progress + timestamps
    let rows = sqlx::query(
        "SELECT wh.video_id, wh.watched_at, v.title, v.channel_name, v.duration_secs,
                v.tags_json, COALESCE(pp.position_secs, 0.0) as pos_secs
         FROM watch_history wh
         JOIN videos v ON wh.video_id = v.id
         LEFT JOIN playback_progress pp ON wh.video_id = pp.video_id
         ORDER BY wh.watched_at DESC",
    )
    .fetch_all(pool)
    .await?;

    let tz_offset: i64 = sqlx::query_scalar(
        "SELECT CAST(strftime('%s', datetime('now', 'localtime')) - strftime('%s', 'now') AS INTEGER)",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let now_unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64;
    let today_day = (now_unix + tz_offset).div_euclid(86_400);

    let mut channel_stats: HashMap<String, (usize, f64)> = HashMap::new();
    let mut tag_counts: HashMap<String, usize> = HashMap::new();
    let mut total_tags = 0usize;
    let mut dur_counts = [0usize; 4];
    let mut active_days: HashSet<i64> = HashSet::new();
    let mut day_hours: HashMap<i64, f64> = HashMap::new();
    let mut total_secs = 0.0f64;

    for row in &rows {
        let vid: String = row.try_get("video_id").unwrap_or_default();
        let watched_at: i64 = row.try_get("watched_at").unwrap_or(0);
        let title: String = row.try_get("title").unwrap_or_default();
        let ch: String = row.try_get("channel_name").unwrap_or_else(|_| "Unknown".into());
        let dur_opt: Option<i64> = row.try_get("duration_secs").unwrap_or(None);
        let pos_secs: f64 = row.try_get("pos_secs").unwrap_or(0.0);
        let tags_raw: String = row.try_get("tags_json").unwrap_or_else(|_| "[]".into());

        let effective_secs = if pos_secs > 10.0 {
            pos_secs
        } else {
            dur_opt.unwrap_or(600).max(0) as f64
        };
        total_secs += effective_secs;

        let local_ts = watched_at + tz_offset;
        let day_idx = local_ts.div_euclid(86_400);
        active_days.insert(day_idx);
        *day_hours.entry(day_idx).or_insert(0.0) += effective_secs / 3600.0;

        let hour = ((local_ts.rem_euclid(86_400)) / 3600) as usize;
        if hour < 24 {
            out.hourly_distribution[hour] += 1;
        }
        // Unix epoch 1970-01-01 was Thursday (index 3 when Mon=0)
        let weekday = (day_idx + 3).rem_euclid(7) as usize;
        out.weekday_distribution[weekday] += 1;

        if today_day >= day_idx && (today_day - day_idx) < 182 {
            let slot = 181 - (today_day - day_idx) as usize;
            out.thermal_activity[slot] += 1;
        }

        let d_secs = dur_opt.unwrap_or(600).max(0);
        if d_secs < 300 {
            dur_counts[0] += 1;
        } else if d_secs < 1200 {
            dur_counts[1] += 1;
        } else if d_secs < 3600 {
            dur_counts[2] += 1;
        } else {
            dur_counts[3] += 1;
        }

        let entry = channel_stats.entry(ch.clone()).or_insert((0, 0.0));
        entry.0 += 1;
        entry.1 += effective_secs / 3600.0;

        if let Ok(parsed_tags) = serde_json::from_str::<Vec<String>>(&tags_raw) {
            for t in parsed_tags {
                let clean_t = t.trim();
                if !clean_t.is_empty() {
                    *tag_counts.entry(clean_t.to_string()).or_insert(0) += 1;
                    total_tags += 1;
                }
            }
        }

        if out.recent_watch_log.len() < 12 {
            out.recent_watch_log.push(RecentWatchItem {
                video_id: vid,
                title,
                channel_name: ch,
                watched_at,
            });
        }
    }

    // Add hours from in-progress videos too
    let cw_secs: f64 = sqlx::query_scalar("SELECT COALESCE(SUM(last_pos_secs), 0.0) FROM continue_watching")
        .fetch_one(pool)
        .await
        .unwrap_or(0.0);
    total_secs += cw_secs;
    out.total_hours_watched = total_secs / 3600.0;

    // 30-day cumulative curve
    let mut running = 0.0f64;
    for i in 0..30 {
        let d = today_day - (29 - i as i64);
        running += day_hours.get(&d).copied().unwrap_or(0.0);
        out.cumulative_hours_30d[i] = running;
    }

    // Active & longest streaks
    let mut active_streak = 0usize;
    let mut probe = today_day;
    if !active_days.contains(&probe) && active_days.contains(&(probe - 1)) {
        probe -= 1;
    }
    while active_days.contains(&probe) {
        active_streak += 1;
        probe -= 1;
    }
    out.active_streak_days = active_streak;

    let mut sorted_days: Vec<i64> = active_days.into_iter().collect();
    sorted_days.sort_unstable();
    let mut best_streak = 0usize;
    let mut cur_streak = 0usize;
    let mut prev: Option<i64> = None;
    for d in sorted_days {
        if prev == Some(d - 1) {
            cur_streak += 1;
        } else {
            cur_streak = 1;
        }
        best_streak = best_streak.max(cur_streak);
        prev = Some(d);
    }
    out.longest_streak_days = best_streak;

    // Peak hour & cadence
    if let Some((hr, _)) = out
        .hourly_distribution
        .iter()
        .enumerate()
        .max_by_key(|(_, &c)| c)
    {
        out.peak_hour = hr;
        out.viewing_cadence = match hr {
            0..=5 => "Midnight Owl (00:00 - 06:00)",
            6..=11 => "Morning Brew (06:00 - 12:00)",
            12..=17 => "Afternoon Flow (12:00 - 18:00)",
            _ => "Evening Theatre (18:00 - 24:00)",
        };
    }

    // Duration DNA
    let dur_total = dur_counts.iter().sum::<usize>().max(1) as f64;
    out.duration_buckets = [
        ("<5m Clips", dur_counts[0], (dur_counts[0] as f64 / dur_total) * 100.0),
        ("5-20m Essays", dur_counts[1], (dur_counts[1] as f64 / dur_total) * 100.0),
        ("20-60m Deep Dives", dur_counts[2], (dur_counts[2] as f64 / dur_total) * 100.0),
        ("60m+ Longform", dur_counts[3], (dur_counts[3] as f64 / dur_total) * 100.0),
    ];

    out.watch_archetype = if dur_counts[2] + dur_counts[3] > dur_counts[0] + dur_counts[1] {
        "Deep-Dive Scholar"
    } else if dur_counts[0] > dur_counts[1] {
        "RapidPulse Clipper"
    } else if out.total_hours_watched > 25.0 {
        "Marathon Curator"
    } else {
        "Zen Explorer"
    };

    let mut top_ch: Vec<TopChannelEntry> = channel_stats
        .into_iter()
        .map(|(channel_name, (video_count, total_hours))| TopChannelEntry {
            channel_name,
            video_count,
            total_hours,
        })
        .collect();
    top_ch.sort_by(|a, b| {
        b.video_count
            .cmp(&a.video_count)
            .then_with(|| b.total_hours.total_cmp(&a.total_hours))
    });
    top_ch.truncate(8);
    out.top_channels = top_ch;

    let mut tags_vec: Vec<(String, usize, f64)> = tag_counts
        .into_iter()
        .map(|(t, c)| {
            let pct = if total_tags > 0 {
                (c as f64 / total_tags as f64) * 100.0
            } else {
                0.0
            };
            (t, c, pct)
        })
        .collect();
    tags_vec.sort_by_key(|a| std::cmp::Reverse(a.1));
    tags_vec.truncate(8);
    out.top_tags = tags_vec;

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{cache::upsert_video, init};

    #[tokio::test]
    async fn test_progress_continue_watching_and_analytics() {
        let pool = init(":memory:").await.unwrap();
        let now = 1_700_000_000i64;
        let v1 = VideoItem {
            id: "testvid0001".into(),
            title: "Understanding LLVM".into(),
            uploader: Some("CompilerLab".into()),
            duration: Some(1800),
            tags: Some(vec!["compilers".into(), "rust".into()]),
            ..Default::default()
        };
        let v2 = VideoItem {
            id: "testvid0002".into(),
            title: "Rust Async Internals".into(),
            uploader: Some("CompilerLab".into()),
            duration: Some(2400),
            tags: Some(vec!["rust".into()]),
            ..Default::default()
        };

        upsert_video(&pool, &v1, now).await.unwrap();
        upsert_video(&pool, &v2, now).await.unwrap();

        save_playback_progress(&pool, "testvid0001", 600.0, 1800.0, now)
            .await
            .unwrap();
        assert_eq!(get_continue_watching(&pool).await.unwrap().len(), 1);
        let prog = get_playback_progress(&pool, "testvid0001").await.unwrap().unwrap();
        assert!((prog.0 - 600.0).abs() < 0.01);

        record_watched(&pool, Some(&v1), "testvid0001", now)
            .await
            .unwrap();
        assert!(is_watched(&pool, "testvid0001").await.unwrap());
        assert_eq!(get_continue_watching(&pool).await.unwrap().len(), 0);

        add_to_watch_later(&pool, &v2, WatchLaterSource::YouTube, now)
            .await
            .unwrap();
        add_to_watch_later(&pool, &v2, WatchLaterSource::Local, now)
            .await
            .unwrap();
        let sources = get_watch_later_sources(&pool).await.unwrap();
        assert_eq!(sources.get("testvid0002"), Some(&WatchLaterSource::Both));

        add_sponsorblock_saved_secs(&pool, 95.5).await.unwrap();
        let analytics = compute_user_analytics(&pool).await.unwrap();
        assert_eq!(analytics.total_videos_watched, 1);
        assert_eq!(analytics.watch_later_count, 1);
        assert!((analytics.sponsorblock_saved_secs - 95.5).abs() < 0.01);
        assert_eq!(analytics.top_channels[0].channel_name, "CompilerLab");
    }

    #[tokio::test]
    async fn test_manual_continue_watching_queue_workflow() {
        let pool = init(":memory:").await.unwrap();
        let now = 1_700_000_000i64;
        let v1 = VideoItem {
            id: "queuevid001".into(),
            title: "First Queued Talk".into(),
            uploader: Some("ChanA".into()),
            duration: Some(900),
            ..Default::default()
        };
        let v2 = VideoItem {
            id: "queuevid002".into(),
            title: "Second Queued Talk".into(),
            uploader: Some("ChanB".into()),
            duration: Some(1500),
            ..Default::default()
        };

        pin_to_continue_watching(&pool, &v1, now).await.unwrap();
        pin_to_continue_watching(&pool, &v2, now + 10).await.unwrap();

        let cw = get_continue_watching(&pool).await.unwrap();
        assert_eq!(cw.len(), 2);
        assert_eq!(cw[0].id, "queuevid001");
        assert_eq!(cw[1].id, "queuevid002");

        // Pinned items stay in Continue Watching even after record_watched finishes them
        record_watched(&pool, Some(&v1), &v1.id, now + 20).await.unwrap();
        let meta = get_continue_watching_meta_map(&pool).await.unwrap();
        assert!(meta.get("queuevid001").unwrap().pinned_manually);
        assert_eq!(get_continue_watching(&pool).await.unwrap().len(), 2);

        // Reordering swaps positions persistently
        assert!(move_continue_watching_item(&pool, "queuevid002", -1).await.unwrap());
        let reordered = get_continue_watching(&pool).await.unwrap();
        assert_eq!(reordered[0].id, "queuevid002");
        assert_eq!(reordered[1].id, "queuevid001");

        // Manual removal deletes cleanly
        remove_continue_watching(&pool, "queuevid001").await.unwrap();
        let after_rm = get_continue_watching(&pool).await.unwrap();
        assert_eq!(after_rm.len(), 1);
        assert_eq!(after_rm[0].id, "queuevid002");
    }
}
