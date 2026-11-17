//! User library state: watch history, resume positions, Watch Later with provenance,
//! SponsorBlock segment cache, offline vault records, and YouTube Wrapped analytics.

use crate::db::cache::{execute_upsert_video, row_to_video, VideoItem};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisRange {
    Today,
    Days7,
    Days30,
    Days90,
    AllTime,
}

impl AnalysisRange {
    pub const ALL: [Self; 5] = [
        Self::Today,
        Self::Days7,
        Self::Days30,
        Self::Days90,
        Self::AllTime,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Today => "Today",
            Self::Days7 => "7D",
            Self::Days30 => "30D",
            Self::Days90 => "90D",
            Self::AllTime => "All",
        }
    }

    pub fn window_days(self) -> Option<i64> {
        match self {
            Self::Today => Some(1),
            Self::Days7 => Some(7),
            Self::Days30 => Some(30),
            Self::Days90 => Some(90),
            Self::AllTime => None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Today => Self::Days7,
            Self::Days7 => Self::Days30,
            Self::Days30 => Self::Days90,
            Self::Days90 => Self::AllTime,
            Self::AllTime => Self::Today,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Today => Self::AllTime,
            Self::Days7 => Self::Today,
            Self::Days30 => Self::Days7,
            Self::Days90 => Self::Days30,
            Self::AllTime => Self::Days90,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisTab {
    Overview,
    Channels,
    Habits,
}

impl AnalysisTab {
    pub const ALL: [Self; 3] = [Self::Overview, Self::Channels, Self::Habits];

    pub fn label(self) -> &'static str {
        match self {
            Self::Overview => "1 Overview",
            Self::Channels => "2 Channels",
            Self::Habits => "3 Habits & Ledger",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisChannelSort {
    Hours,
    Videos,
    FinishRate,
}

impl AnalysisChannelSort {
    pub fn label(self) -> &'static str {
        match self {
            Self::Hours => "Hours",
            Self::Videos => "Videos",
            Self::FinishRate => "Finish%",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Hours => Self::Videos,
            Self::Videos => Self::FinishRate,
            Self::FinishRate => Self::Hours,
        }
    }
}

#[derive(Debug, Clone)]
pub struct TopChannelEntry {
    pub channel_id: Option<String>,
    pub channel_name: String,
    pub video_count: usize,
    pub total_hours: f64,
    pub share_pct: f64,
    pub finish_rate_pct: f64,
}

pub fn sort_channels_by(
    channels: &[TopChannelEntry],
    sort: AnalysisChannelSort,
) -> Vec<TopChannelEntry> {
    let mut out = channels.to_vec();
    match sort {
        AnalysisChannelSort::Hours => out.sort_by(|a, b| {
            b.total_hours
                .total_cmp(&a.total_hours)
                .then_with(|| b.video_count.cmp(&a.video_count))
        }),
        AnalysisChannelSort::Videos => out.sort_by(|a, b| {
            b.video_count
                .cmp(&a.video_count)
                .then_with(|| b.total_hours.total_cmp(&a.total_hours))
        }),
        AnalysisChannelSort::FinishRate => out.sort_by(|a, b| {
            b.finish_rate_pct
                .total_cmp(&a.finish_rate_pct)
                .then_with(|| b.total_hours.total_cmp(&a.total_hours))
        }),
    }
    out
}

#[derive(Debug, Clone)]
pub struct FormatBucket {
    pub label: &'static str,
    pub video_count: usize,
    pub actual_hours: f64,
    pub share_pct: f64,
    pub finish_rate_pct: f64,
}

#[derive(Debug, Clone)]
pub struct SponsorCategoryStat {
    pub label: String,
    pub saved_secs: f64,
    pub share_pct: f64,
}

#[derive(Debug, Clone)]
pub struct DailyPacePoint {
    pub day_label: String,
    pub watch_mins: f64,
    pub is_today: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct RecentWatchItem {
    pub video_id: String,
    pub title: String,
    pub channel_id: Option<String>,
    pub channel_name: String,
    pub watched_at: i64,
    pub watched_secs: f64,
    pub video_duration_secs: f64,
    pub end_pos_secs: f64,
    pub playback_speed: f64,
    pub sb_skipped_secs: f64,
    pub completed: bool,
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

#[derive(Debug, Clone)]
pub struct WatchSessionWrite<'a> {
    pub video_id: &'a str,
    pub channel_id: Option<&'a str>,
    pub channel_name: &'a str,
    pub source: &'a str,
    pub started_at: i64,
    pub ended_at: i64,
    pub watched_secs: f64,
    pub media_secs: f64,
    pub start_pos_secs: f64,
    pub end_pos_secs: f64,
    pub video_duration_secs: f64,
    pub playback_speed: f64,
    pub sb_skipped_secs: f64,
    pub sb_categories_json: &'a str,
    pub player: &'a str,
    pub completed: bool,
}

/// Aggregated personal telemetry powering `[A] Phaenna Analysis`.
#[derive(Debug, Clone)]
pub struct UserAnalytics {
    pub include_yt_sync: bool,
    pub daily_budget_mins: u32,
    pub total_sessions: usize,
    pub total_videos_watched: usize,
    pub completed_videos: usize,
    pub partial_videos: usize,
    pub bounced_videos: usize,
    pub total_hours_watched: f64,
    pub media_hours_consumed: f64,
    pub today_watch_mins: f64,
    pub avg_daily_mins: f64,
    pub period_delta_pct: Option<f64>,
    pub avg_session_mins: f64,
    pub weighted_avg_speed: f64,
    pub speed_saved_mins: f64,
    pub in_progress_videos: usize,
    pub completion_rate: f64,
    pub deep_dive_ratio_pct: f64,
    pub active_streak_days: usize,
    pub longest_streak_days: usize,
    pub budget_hit_days: usize,
    pub budget_total_days: usize,
    pub peak_hour: usize,
    pub peak_window_label: &'static str,
    pub hourly_minutes: [f64; 24],
    pub weekday_minutes: [f64; 7],
    pub weekday_avg_mins: f64,
    pub weekend_avg_mins: f64,
    pub thermal_activity: Vec<u64>, // 84 days (12 weeks) ending today from timed sessions
    pub daily_pace: Vec<DailyPacePoint>,
    pub format_buckets: [FormatBucket; 4],
    pub top_channels: Vec<TopChannelEntry>,
    pub unique_channels_count: usize,
    pub top3_concentration_pct: f64,
    pub unresolved_channel_videos: usize,
    pub sponsorblock_saved_secs: f64,
    pub sb_categories: Vec<SponsorCategoryStat>,
    pub offline_cached_videos: usize,
    pub offline_cached_bytes: u64,
    pub watch_later_count: usize,
    pub insights: Vec<String>,
    pub recent_watch_log: Vec<RecentWatchItem>,
    pub has_yt_backlog_only: bool,
}

impl Default for UserAnalytics {
    fn default() -> Self {
        Self {
            include_yt_sync: false,
            daily_budget_mins: 60,
            total_sessions: 0,
            total_videos_watched: 0,
            completed_videos: 0,
            partial_videos: 0,
            bounced_videos: 0,
            total_hours_watched: 0.0,
            media_hours_consumed: 0.0,
            today_watch_mins: 0.0,
            avg_daily_mins: 0.0,
            period_delta_pct: None,
            avg_session_mins: 0.0,
            weighted_avg_speed: 1.0,
            speed_saved_mins: 0.0,
            in_progress_videos: 0,
            completion_rate: 0.0,
            deep_dive_ratio_pct: 0.0,
            active_streak_days: 0,
            longest_streak_days: 0,
            budget_hit_days: 0,
            budget_total_days: 0,
            peak_hour: 20,
            peak_window_label: "20:00 - 22:00",
            hourly_minutes: [0.0; 24],
            weekday_minutes: [0.0; 7],
            weekday_avg_mins: 0.0,
            weekend_avg_mins: 0.0,
            thermal_activity: vec![0; 84],
            daily_pace: Vec::new(),
            format_buckets: [
                FormatBucket { label: "<5m Clips", video_count: 0, actual_hours: 0.0, share_pct: 0.0, finish_rate_pct: 0.0 },
                FormatBucket { label: "5-20m Essay", video_count: 0, actual_hours: 0.0, share_pct: 0.0, finish_rate_pct: 0.0 },
                FormatBucket { label: "20-60m Deep", video_count: 0, actual_hours: 0.0, share_pct: 0.0, finish_rate_pct: 0.0 },
                FormatBucket { label: "60m+ Long", video_count: 0, actual_hours: 0.0, share_pct: 0.0, finish_rate_pct: 0.0 },
            ],
            top_channels: Vec::new(),
            unique_channels_count: 0,
            top3_concentration_pct: 0.0,
            unresolved_channel_videos: 0,
            sponsorblock_saved_secs: 0.0,
            sb_categories: Vec::new(),
            offline_cached_videos: 0,
            offline_cached_bytes: 0,
            watch_later_count: 0,
            insights: Vec::new(),
            recent_watch_log: Vec::new(),
            has_yt_backlog_only: false,
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
        "INSERT INTO watch_history (video_id, watched_at, origin) VALUES (?, ?, 'local')
         ON CONFLICT(video_id) DO UPDATE SET watched_at = excluded.watched_at, origin = 'local'",
    )
    .bind(clean)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    Ok(())
}

pub async fn upsert_watch_session(pool: &SqlitePool, s: &WatchSessionWrite<'_>) -> Result<()> {
    let clean = s.video_id.trim_start_matches("RD");
    sqlx::query(
        "INSERT INTO watch_sessions (
            video_id, channel_id, channel_name, source, started_at, ended_at,
            watched_secs, media_secs, start_pos_secs, end_pos_secs, video_duration_secs,
            playback_speed, sb_skipped_secs, sb_categories_json, player, completed
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
        ON CONFLICT(video_id, started_at, source) DO UPDATE SET
            channel_id = COALESCE(NULLIF(excluded.channel_id, ''), watch_sessions.channel_id),
            channel_name = CASE WHEN TRIM(excluded.channel_name) != '' AND LOWER(TRIM(excluded.channel_name)) != 'unknown'
                THEN excluded.channel_name ELSE watch_sessions.channel_name END,
            ended_at = MAX(watch_sessions.ended_at, excluded.ended_at),
            watched_secs = MAX(watch_sessions.watched_secs, excluded.watched_secs),
            media_secs = MAX(watch_sessions.media_secs, excluded.media_secs),
            end_pos_secs = MAX(watch_sessions.end_pos_secs, excluded.end_pos_secs),
            video_duration_secs = CASE WHEN excluded.video_duration_secs > 0.0
                THEN excluded.video_duration_secs ELSE watch_sessions.video_duration_secs END,
            playback_speed = excluded.playback_speed,
            sb_skipped_secs = MAX(watch_sessions.sb_skipped_secs, excluded.sb_skipped_secs),
            sb_categories_json = CASE WHEN excluded.sb_categories_json != '{}'
                THEN excluded.sb_categories_json ELSE watch_sessions.sb_categories_json END,
            completed = MAX(watch_sessions.completed, excluded.completed)",
    )
    .bind(clean)
    .bind(s.channel_id)
    .bind(s.channel_name)
    .bind(s.source)
    .bind(s.started_at)
    .bind(s.ended_at)
    .bind(s.watched_secs.max(0.0))
    .bind(s.media_secs.max(0.0))
    .bind(s.start_pos_secs.max(0.0))
    .bind(s.end_pos_secs.max(0.0))
    .bind(s.video_duration_secs.max(0.0))
    .bind(s.playback_speed.clamp(0.25, 4.0))
    .bind(s.sb_skipped_secs.max(0.0))
    .bind(s.sb_categories_json)
    .bind(s.player)
    .bind(if s.completed { 1_i64 } else { 0_i64 })
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn backfill_session_video_metadata(
    pool: &SqlitePool,
    video_id: &str,
    channel_id: Option<&str>,
    channel_name: &str,
    duration_secs: f64,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    let valid_ch = !channel_name.trim().is_empty()
        && !channel_name.trim().eq_ignore_ascii_case("unknown");
    if !valid_ch && duration_secs <= 0.0 {
        return Ok(());
    }
    sqlx::query(
        "UPDATE watch_sessions SET
            channel_id = COALESCE(NULLIF(channel_id, ''), ?2),
            channel_name = CASE WHEN (TRIM(channel_name) = '' OR LOWER(TRIM(channel_name)) = 'unknown') AND ?3 != ''
                THEN ?3 ELSE channel_name END,
            video_duration_secs = CASE WHEN video_duration_secs <= 0.0 AND ?4 > 0.0
                THEN ?4 ELSE video_duration_secs END
         WHERE video_id = ?1",
    )
    .bind(clean)
    .bind(channel_id)
    .bind(if valid_ch { channel_name.trim() } else { "" })
    .bind(duration_secs.max(0.0))
    .execute(pool)
    .await?;
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
        "INSERT INTO watch_history (video_id, watched_at, origin) VALUES (?, ?, 'local')
         ON CONFLICT(video_id) DO UPDATE SET watched_at = excluded.watched_at, origin = 'local'",
    )
    .bind(clean)
    .bind(now)
    .execute(&mut *tx)
    .await?;

    sqlx::query("DELETE FROM continue_watching WHERE video_id = ? AND pinned_manually = 0")
        .bind(clean)
        .execute(&mut *tx)
        .await?;

    // If a live playback session exists for this video within the past hour, mark it completed;
    // otherwise record an honest completed entry from known duration/progress (zero fake 600s).
    let updated_recent = sqlx::query(
        "UPDATE watch_sessions SET completed = 1, ended_at = MAX(ended_at, ?)
         WHERE video_id = ? AND ABS(ended_at - ?) < 3600",
    )
    .bind(now)
    .bind(clean)
    .bind(now)
    .execute(&mut *tx)
    .await?
    .rows_affected();

    if updated_recent == 0 {
        let row = sqlx::query(
            "SELECT v.channel_id, v.channel_name, COALESCE(v.duration_secs, 0) as d_secs,
                    COALESCE(pp.position_secs, 0.0) as pos_secs
             FROM videos v
             LEFT JOIN playback_progress pp ON pp.video_id = v.id
             WHERE v.id = ?",
        )
        .bind(clean)
        .fetch_optional(&mut *tx)
        .await?;

        let (ch_id, ch_name, d_secs, pos_secs): (Option<String>, String, f64, f64) = match row {
            Some(r) => (
                r.try_get("channel_id").ok().flatten(),
                r.try_get::<String, _>("channel_name").unwrap_or_default(),
                r.try_get::<i64, _>("d_secs").unwrap_or(0).max(0) as f64,
                r.try_get::<f64, _>("pos_secs").unwrap_or(0.0).max(0.0),
            ),
            None => (
                video.and_then(|v| v.channel_id.clone()),
                video.and_then(|v| v.uploader.clone()).unwrap_or_default(),
                video.and_then(|v| v.duration).unwrap_or(0) as f64,
                0.0,
            ),
        };
        let eff_secs = if pos_secs > 10.0 { pos_secs } else { d_secs };
        sqlx::query(
            "INSERT INTO watch_sessions (
                video_id, channel_id, channel_name, source, started_at, ended_at,
                watched_secs, media_secs, start_pos_secs, end_pos_secs, video_duration_secs,
                playback_speed, sb_skipped_secs, sb_categories_json, player, completed
            ) VALUES (?, ?, ?, 'marked_watched', ?, ?, ?, ?, 0.0, ?, ?, 1.0, 0.0, '{}', 'manual', 1)
            ON CONFLICT(video_id, started_at, source) DO UPDATE SET completed = 1",
        )
        .bind(clean)
        .bind(ch_id)
        .bind(ch_name)
        .bind(now)
        .bind(now)
        .bind(eff_secs)
        .bind(eff_secs)
        .bind(eff_secs)
        .bind(d_secs)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn get_watched_ids(pool: &SqlitePool) -> Result<HashSet<String>> {
    let rows: Vec<(String,)> = sqlx::query_as("SELECT video_id FROM watch_history")
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(|r| r.0).collect())
}

pub async fn get_local_watch_history_videos(
    pool: &SqlitePool,
    limit: usize,
) -> Result<Vec<VideoItem>> {
    let rows = sqlx::query(
        "SELECT v.* FROM watch_history wh
         JOIN videos v ON wh.video_id = v.id
         ORDER BY wh.watched_at DESC
         LIMIT ?",
    )
    .bind(limit.max(1) as i64)
    .fetch_all(pool)
    .await?;

    Ok(rows.iter().map(row_to_video).collect())
}

pub async fn sync_youtube_watched_items(
    pool: &SqlitePool,
    items: &[VideoItem],
    progress_pcts: &[(String, u8)],
    mark_all_items_watched: bool,
    now: i64,
) -> Result<()> {
    if !mark_all_items_watched && progress_pcts.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;

    if mark_all_items_watched {
        for (idx, v) in items.iter().enumerate() {
            execute_upsert_video(&mut *tx, v, now).await?;
            let ts = now.saturating_sub(idx as i64);
            sqlx::query(
                "INSERT INTO watch_history (video_id, watched_at) VALUES (?, ?)
                 ON CONFLICT(video_id) DO NOTHING",
            )
            .bind(v.clean_id())
            .bind(ts)
            .execute(&mut *tx)
            .await?;
        }
    }

    for (vid_id, pct) in progress_pcts {
        if *pct == 0 {
            continue;
        }
        let clean = vid_id.trim_start_matches("RD");
        let matched = items
            .iter()
            .find(|v| v.clean_id() == clean || v.id == *vid_id);
        if let Some(v) = matched {
            if !mark_all_items_watched {
                execute_upsert_video(&mut *tx, v, now).await?;
            }
        }
        sqlx::query(
            "INSERT INTO watch_history (video_id, watched_at) VALUES (?, ?)
             ON CONFLICT(video_id) DO NOTHING",
        )
        .bind(clean)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        if let Some(dur) = matched.and_then(|v| v.duration).filter(|&d| d > 0) {
            let pos = (dur as f64) * ((*pct as f64).min(100.0) / 100.0);
            sqlx::query(
                "INSERT INTO playback_progress (video_id, position_secs, duration_secs, updated_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(video_id) DO NOTHING",
            )
            .bind(clean)
            .bind(pos)
            .bind(dur as f64)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
    }

    tx.commit().await?;
    Ok(())
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

fn is_valid_channel_name(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty() && !t.eq_ignore_ascii_case("unknown")
}

fn sb_category_display(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "sponsor" => "Sponsor".to_string(),
        "selfpromo" | "self-promo" => "Self-Promo".to_string(),
        "intro" => "Intro".to_string(),
        "outro" => "Outro".to_string(),
        "interaction" => "Interaction".to_string(),
        "filler" => "Filler".to_string(),
        "music_offtopic" => "Off-Topic".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "Other".to_string(),
    }
}

fn weekday_short(idx: usize) -> &'static str {
    match idx % 7 {
        0 => "Mon",
        1 => "Tue",
        2 => "Wed",
        3 => "Thu",
        4 => "Fri",
        5 => "Sat",
        _ => "Sun",
    }
}

#[cfg(test)]
pub async fn compute_user_analytics(pool: &SqlitePool) -> Result<UserAnalytics> {
    compute_user_analytics_with_options(pool, AnalysisRange::AllTime, false, 60).await
}

pub async fn compute_user_analytics_with_options(
    pool: &SqlitePool,
    range: AnalysisRange,
    include_yt_sync: bool,
    daily_budget_mins: u32,
) -> Result<UserAnalytics> {
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

    compute_user_analytics_at(
        pool,
        now_unix,
        tz_offset,
        range,
        include_yt_sync,
        daily_budget_mins.max(10),
    )
    .await
}

struct ChannelAccum {
    channel_id: Option<String>,
    videos: HashSet<String>,
    hours: f64,
    completed: usize,
    total: usize,
}

impl ChannelAccum {
    fn record(
        &mut self,
        ch_id: &Option<String>,
        vid: &str,
        hours: f64,
        is_done: bool,
    ) {
        if self.channel_id.is_none() {
            self.channel_id = ch_id.clone();
        }
        self.videos.insert(vid.to_string());
        self.hours += hours;
        if is_done {
            self.completed += 1;
        }
        self.total += 1;
    }
}

pub async fn compute_user_analytics_at(
    pool: &SqlitePool,
    now_unix: i64,
    tz_offset: i64,
    range: AnalysisRange,
    include_yt_sync: bool,
    daily_budget_mins: u32,
) -> Result<UserAnalytics> {
    let mut out = UserAnalytics {
        include_yt_sync,
        daily_budget_mins,
        ..UserAnalytics::default()
    };

    let today_day = (now_unix + tz_offset).div_euclid(86_400);
    let cutoff_ts = range
        .window_days()
        .map(|d| (today_day - (d - 1)) * 86_400 - tz_offset);
    let prev_cutoff_ts = range
        .window_days()
        .map(|d| (today_day - (2 * d - 1)) * 86_400 - tz_offset);

    let in_prog: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM continue_watching")
        .fetch_one(pool)
        .await
        .unwrap_or(0);
    out.in_progress_videos = in_prog.max(0) as usize;

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

    let global_sb_secs = get_sponsorblock_saved_secs(pool).await.unwrap_or(0.0);

    // Pull all recorded local sessions ordered newest-first
    let session_rows = sqlx::query(
        "SELECT ws.video_id,
                COALESCE(NULLIF(ws.channel_id, ''), v.channel_id) as channel_id,
                CASE WHEN TRIM(ws.channel_name) != '' AND LOWER(TRIM(ws.channel_name)) != 'unknown'
                     THEN ws.channel_name
                     ELSE COALESCE(v.channel_name, '') END as channel_name,
                COALESCE(v.title, ws.video_id) as title,
                ws.started_at, ws.ended_at, ws.watched_secs, ws.media_secs,
                ws.end_pos_secs,
                CASE WHEN ws.video_duration_secs > 0.0 THEN ws.video_duration_secs
                     ELSE COALESCE(v.duration_secs, 0) * 1.0 END as dur_secs,
                ws.playback_speed, ws.sb_skipped_secs, ws.sb_categories_json, ws.completed
         FROM watch_sessions ws
         LEFT JOIN videos v ON ws.video_id = v.id
         ORDER BY ws.started_at DESC",
    )
    .fetch_all(pool)
    .await?;

    let mut session_video_ids: HashSet<String> = HashSet::new();
    let mut active_days_all: HashSet<i64> = HashSet::new();
    let mut day_mins_all: HashMap<i64, f64> = HashMap::new();
    let mut window_days_seen: HashSet<i64> = HashSet::new();
    let mut prev_window_secs = 0.0f64;

    let mut channel_stats: HashMap<String, ChannelAccum> = HashMap::new();
    let mut unresolved_vids: HashSet<String> = HashSet::new();
    let mut unique_vids_win: HashSet<String> = HashSet::new();
    let mut sb_cats: HashMap<String, f64> = HashMap::new();
    let mut speed_weighted_sum = 0.0f64;
    let mut speed_weight_secs = 0.0f64;
    let mut format_completed = [0usize; 4];

    for row in &session_rows {
        let vid: String = row.try_get("video_id").unwrap_or_default();
        let ch_id: Option<String> = row.try_get("channel_id").ok().flatten();
        let ch_name: String = row.try_get("channel_name").unwrap_or_default();
        let title: String = row.try_get("title").unwrap_or_else(|_| vid.clone());
        let started_at: i64 = row.try_get("started_at").unwrap_or(0);
        let ended_at: i64 = row.try_get("ended_at").unwrap_or(started_at);
        let watched_secs: f64 = row.try_get::<f64, _>("watched_secs").unwrap_or(0.0).max(0.0);
        let media_secs: f64 = row.try_get::<f64, _>("media_secs").unwrap_or(watched_secs).max(watched_secs);
        let end_pos_secs: f64 = row.try_get::<f64, _>("end_pos_secs").unwrap_or(0.0).max(0.0);
        let dur_secs: f64 = row.try_get::<f64, _>("dur_secs").unwrap_or(0.0).max(0.0);
        let speed: f64 = row.try_get::<f64, _>("playback_speed").unwrap_or(1.0).clamp(0.25, 4.0);
        let sb_secs: f64 = row.try_get::<f64, _>("sb_skipped_secs").unwrap_or(0.0).max(0.0);
        let sb_json: String = row.try_get("sb_categories_json").unwrap_or_else(|_| "{}".into());
        let completed_flag: i64 = row.try_get("completed").unwrap_or(0);

        session_video_ids.insert(vid.clone());

        let local_ts = started_at.max(ended_at) + tz_offset;
        let day_idx = local_ts.div_euclid(86_400);
        active_days_all.insert(day_idx);
        *day_mins_all.entry(day_idx).or_insert(0.0) += watched_secs / 60.0;

        if today_day >= day_idx && (today_day - day_idx) < 84 {
            let slot = 83 - (today_day - day_idx) as usize;
            out.thermal_activity[slot] += (watched_secs / 60.0).ceil().max(1.0) as u64;
        }

        // Check prior comparison window for delta %
        if let (Some(cur_cut), Some(prev_cut)) = (cutoff_ts, prev_cutoff_ts) {
            if started_at >= prev_cut && started_at < cur_cut {
                prev_window_secs += watched_secs;
            }
        }
        if cutoff_ts.map(|c| started_at < c).unwrap_or(false) {
            continue;
        }

        window_days_seen.insert(day_idx);
        unique_vids_win.insert(vid.clone());
        out.total_sessions += 1;

        let frac = if dur_secs > 1.0 {
            (end_pos_secs / dur_secs).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let is_done = completed_flag > 0 || frac >= 0.85;
        if is_done {
            out.completed_videos += 1;
        } else if frac >= 0.15 || watched_secs >= 45.0 {
            out.partial_videos += 1;
        } else {
            out.bounced_videos += 1;
        }

        out.total_hours_watched += watched_secs / 3600.0;
        out.media_hours_consumed += media_secs / 3600.0;
        out.sponsorblock_saved_secs += sb_secs;

        if watched_secs > 1.0 {
            speed_weighted_sum += speed * watched_secs;
            speed_weight_secs += watched_secs;
        }

        if let Ok(parsed) = serde_json::from_str::<HashMap<String, f64>>(&sb_json) {
            for (k, v) in parsed {
                if v > 0.0 {
                    *sb_cats.entry(k).or_insert(0.0) += v;
                }
            }
        }

        let hour = ((local_ts.rem_euclid(86_400)) / 3600) as usize;
        if hour < 24 {
            out.hourly_minutes[hour] += watched_secs / 60.0;
        }
        let weekday = (day_idx + 3).rem_euclid(7) as usize;
        out.weekday_minutes[weekday] += watched_secs / 60.0;

        if dur_secs > 0.0 {
            let b_idx = if dur_secs < 300.0 {
                0
            } else if dur_secs < 1200.0 {
                1
            } else if dur_secs < 3600.0 {
                2
            } else {
                3
            };
            out.format_buckets[b_idx].video_count += 1;
            out.format_buckets[b_idx].actual_hours += watched_secs / 3600.0;
            if is_done {
                format_completed[b_idx] += 1;
            }
        }

        if is_valid_channel_name(&ch_name) {
            channel_stats
                .entry(ch_name.trim().to_string())
                .or_insert_with(|| ChannelAccum {
                    channel_id: ch_id.clone(),
                    videos: HashSet::new(),
                    hours: 0.0,
                    completed: 0,
                    total: 0,
                })
                .record(&ch_id, &vid, watched_secs / 3600.0, is_done);
        } else {
            unresolved_vids.insert(vid.clone());
        }

        if out.recent_watch_log.len() < 30 {
            out.recent_watch_log.push(RecentWatchItem {
                video_id: vid,
                title,
                channel_id: ch_id,
                channel_name: if is_valid_channel_name(&ch_name) {
                    ch_name
                } else {
                    "Unresolved Creator".to_string()
                },
                watched_at: started_at,
                watched_secs,
                video_duration_secs: dur_secs,
                end_pos_secs,
                playback_speed: speed,
                sb_skipped_secs: sb_secs,
                completed: is_done,
            });
        }
    }

    // Optional synced YouTube history merge (adds backlog counts & creators WITHOUT polluting circadian/streak/pace)
    if include_yt_sync {
        let hist_rows = sqlx::query(
            "SELECT wh.video_id, wh.watched_at, v.title, v.channel_id, v.channel_name,
                    COALESCE(v.duration_secs, 0) as dur_secs,
                    COALESCE(pp.position_secs, 0.0) as pos_secs
             FROM watch_history wh
             LEFT JOIN videos v ON wh.video_id = v.id
             LEFT JOIN playback_progress pp ON wh.video_id = pp.video_id
             ORDER BY wh.watched_at DESC",
        )
        .fetch_all(pool)
        .await?;

        for row in &hist_rows {
            let vid: String = row.try_get("video_id").unwrap_or_default();
            if session_video_ids.contains(&vid) {
                continue;
            }
            let watched_at: i64 = row.try_get("watched_at").unwrap_or(0);
            if cutoff_ts.map(|c| watched_at < c).unwrap_or(false) {
                continue;
            }
            let title: String = row.try_get("title").unwrap_or_else(|_| vid.clone());
            let ch_id: Option<String> = row.try_get("channel_id").ok().flatten();
            let ch_name: String = row.try_get("channel_name").unwrap_or_default();
            let dur_secs: f64 = row.try_get::<i64, _>("dur_secs").unwrap_or(0).max(0) as f64;
            let pos_secs: f64 = row.try_get::<f64, _>("pos_secs").unwrap_or(0.0).max(0.0);
            // Only credit real known duration/position; never invent a fake 600s default.
            let est_secs = if pos_secs > 10.0 { pos_secs } else { dur_secs };

            unique_vids_win.insert(vid.clone());
            out.completed_videos += 1;
            out.total_hours_watched += est_secs / 3600.0;
            out.media_hours_consumed += est_secs / 3600.0;

            if dur_secs > 0.0 {
                let b_idx = if dur_secs < 300.0 {
                    0
                } else if dur_secs < 1200.0 {
                    1
                } else if dur_secs < 3600.0 {
                    2
                } else {
                    3
                };
                out.format_buckets[b_idx].video_count += 1;
                out.format_buckets[b_idx].actual_hours += est_secs / 3600.0;
                format_completed[b_idx] += 1;
            }

            if is_valid_channel_name(&ch_name) {
                channel_stats
                    .entry(ch_name.trim().to_string())
                    .or_insert_with(|| ChannelAccum {
                        channel_id: ch_id.clone(),
                        videos: HashSet::new(),
                        hours: 0.0,
                        completed: 0,
                        total: 0,
                    })
                    .record(&ch_id, &vid, est_secs / 3600.0, true);
            } else {
                unresolved_vids.insert(vid.clone());
            }

            if out.recent_watch_log.len() < 30 {
                out.recent_watch_log.push(RecentWatchItem {
                    video_id: vid,
                    title,
                    channel_id: ch_id,
                    channel_name: if is_valid_channel_name(&ch_name) {
                        ch_name
                    } else {
                        "Unresolved Creator".to_string()
                    },
                    watched_at,
                    watched_secs: est_secs,
                    video_duration_secs: dur_secs,
                    end_pos_secs: est_secs,
                    playback_speed: 1.0,
                    sb_skipped_secs: 0.0,
                    completed: true,
                });
            }
        }
    } else if session_rows.is_empty() {
        let hist_cnt: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM watch_history")
            .fetch_one(pool)
            .await
            .unwrap_or(0);
        out.has_yt_backlog_only = hist_cnt > 0;
    }

    if range == AnalysisRange::AllTime && global_sb_secs > out.sponsorblock_saved_secs {
        out.sponsorblock_saved_secs = global_sb_secs;
    }

    out.total_videos_watched = unique_vids_win.len();
    out.unresolved_channel_videos = unresolved_vids.len();

    let sess_outcomes = out.completed_videos + out.partial_videos + out.bounced_videos;
    out.completion_rate = if sess_outcomes > 0 {
        (out.completed_videos as f64 / sess_outcomes as f64) * 100.0
    } else {
        0.0
    };
    out.avg_session_mins = if out.total_sessions > 0 {
        (out.total_hours_watched * 60.0) / out.total_sessions as f64
    } else {
        0.0
    };
    out.weighted_avg_speed = if speed_weight_secs > 0.0 {
        speed_weighted_sum / speed_weight_secs
    } else {
        1.0
    };
    out.speed_saved_mins = ((out.media_hours_consumed - out.total_hours_watched) * 60.0).max(0.0);

    out.today_watch_mins = day_mins_all.get(&today_day).copied().unwrap_or(0.0);
    let days_denom = match range.window_days() {
        Some(d) => d.max(1) as f64,
        None => window_days_seen.len().max(1) as f64,
    };
    out.avg_daily_mins = (out.total_hours_watched * 60.0) / days_denom;
    out.period_delta_pct = if range.window_days().is_some() && prev_window_secs >= 60.0 {
        let cur_secs = out.total_hours_watched * 3600.0;
        Some(((cur_secs - prev_window_secs) / prev_window_secs) * 100.0)
    } else {
        None
    };

    // 7-day rolling daily pace + budget compliance count
    let pace_days = 7i64;
    for i in 0..pace_days {
        let d = today_day - (pace_days - 1 - i);
        let wd = (d + 3).rem_euclid(7) as usize;
        let mins = day_mins_all.get(&d).copied().unwrap_or(0.0);
        if mins > 0.0 && mins <= daily_budget_mins as f64 {
            out.budget_hit_days += 1;
        }
        out.budget_total_days += 1;
        out.daily_pace.push(DailyPacePoint {
            day_label: weekday_short(wd).to_string(),
            watch_mins: mins,
            is_today: d == today_day,
        });
    }

    // Active & longest streaks (strictly timed sessions)
    let mut active_streak = 0usize;
    let mut probe = today_day;
    if !active_days_all.contains(&probe) && active_days_all.contains(&(probe - 1)) {
        probe -= 1;
    }
    while active_days_all.contains(&probe) {
        active_streak += 1;
        probe -= 1;
    }
    out.active_streak_days = active_streak;

    let mut sorted_days: Vec<i64> = active_days_all.into_iter().collect();
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

    // Circadian peak + weekday vs weekend avg
    if let Some((hr, _)) = out
        .hourly_minutes
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| a.total_cmp(b))
    {
        out.peak_hour = hr;
        out.peak_window_label = match hr {
            0..=5 => "00:00 - 06:00 (Late Night)",
            6..=11 => "06:00 - 12:00 (Morning)",
            12..=17 => "12:00 - 18:00 (Afternoon)",
            _ => "18:00 - 24:00 (Evening)",
        };
    }
    let wk_sum: f64 = out.weekday_minutes[0..5].iter().sum();
    let we_sum: f64 = out.weekday_minutes[5..7].iter().sum();
    let weeks_est = (days_denom / 7.0).max(1.0);
    out.weekday_avg_mins = wk_sum / (5.0 * weeks_est);
    out.weekend_avg_mins = we_sum / (2.0 * weeks_est);

    // Format buckets share % (true share of watch hours) + finish rate
    let bucket_total_hrs: f64 = out.format_buckets.iter().map(|b| b.actual_hours).sum();
    let bucket_total_cnt: usize = out.format_buckets.iter().map(|b| b.video_count).sum();
    for (i, b) in out.format_buckets.iter_mut().enumerate() {
        b.share_pct = if bucket_total_hrs > 0.001 {
            (b.actual_hours / bucket_total_hrs) * 100.0
        } else if bucket_total_cnt > 0 {
            (b.video_count as f64 / bucket_total_cnt as f64) * 100.0
        } else {
            0.0
        };
        b.finish_rate_pct = if b.video_count > 0 {
            (format_completed[i] as f64 / b.video_count as f64) * 100.0
        } else {
            0.0
        };
    }
    let deep_hrs = out.format_buckets[2].actual_hours + out.format_buckets[3].actual_hours;
    out.deep_dive_ratio_pct = if bucket_total_hrs > 0.001 {
        (deep_hrs / bucket_total_hrs) * 100.0
    } else {
        0.0
    };

    // Creator leaderboard + concentration
    let total_ch_hours: f64 = channel_stats.values().map(|v| v.hours).sum();
    let mut top_ch: Vec<TopChannelEntry> = channel_stats
        .into_iter()
        .map(|(channel_name, acc)| TopChannelEntry {
            channel_id: acc.channel_id,
            channel_name,
            video_count: acc.videos.len().max(acc.total),
            total_hours: acc.hours,
            share_pct: if total_ch_hours > 0.001 {
                (acc.hours / total_ch_hours) * 100.0
            } else {
                0.0
            },
            finish_rate_pct: if acc.total > 0 {
                (acc.completed as f64 / acc.total as f64) * 100.0
            } else {
                0.0
            },
        })
        .collect();
    top_ch.sort_by(|a, b| {
        b.total_hours
            .total_cmp(&a.total_hours)
            .then_with(|| b.video_count.cmp(&a.video_count))
    });
    out.unique_channels_count = top_ch.len();
    out.top3_concentration_pct = top_ch.iter().take(3).map(|c| c.share_pct).sum();
    top_ch.truncate(24);
    out.top_channels = top_ch;

    // SponsorBlock category breakdown
    let sb_cat_total: f64 = sb_cats.values().sum();
    let mut sb_vec: Vec<SponsorCategoryStat> = sb_cats
        .into_iter()
        .map(|(category, saved_secs)| SponsorCategoryStat {
            label: sb_category_display(&category),
            saved_secs,
            share_pct: if sb_cat_total > 0.001 {
                (saved_secs / sb_cat_total) * 100.0
            } else {
                0.0
            },
        })
        .collect();
    sb_vec.sort_by(|a, b| b.saved_secs.total_cmp(&a.saved_secs));
    out.sb_categories = sb_vec;

    // Concise evidence-backed analyst insights (strictly from real SQLite data)
    if out.unique_channels_count >= 3 && out.top3_concentration_pct > 0.0 {
        out.insights.push(format!(
            "Top 3 creators make up {:.0}% of watch time across {} channels.",
            out.top3_concentration_pct, out.unique_channels_count
        ));
    }
    if out.format_buckets[2].video_count + out.format_buckets[3].video_count > 0 {
        out.insights.push(format!(
            "Deep-dive share (20m+ videos): {:.0}% of time ({:.0}% finish rate).",
            out.deep_dive_ratio_pct,
            if out.format_buckets[2].video_count >= out.format_buckets[3].video_count {
                out.format_buckets[2].finish_rate_pct
            } else {
                out.format_buckets[3].finish_rate_pct
            }
        ));
    }
    let night_mins: f64 = out.hourly_minutes[0..6].iter().sum();
    let all_hr_mins: f64 = out.hourly_minutes.iter().sum();
    if all_hr_mins > 1.0 {
        let night_pct = (night_mins / all_hr_mins) * 100.0;
        out.insights.push(format!(
            "Late-night (00h-05h) watching is {:.0}% of tracked sessions.",
            night_pct
        ));
    }

    Ok(out)
}

pub async fn save_user_video_comment(
    pool: &SqlitePool,
    video_id: &str,
    author: &str,
    content: &str,
    created_at: i64,
    parent_comment_id: Option<&str>,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    sqlx::query(
        "INSERT INTO user_video_comments (video_id, author, content, created_at, parent_comment_id) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(clean)
    .bind(author)
    .bind(content.trim())
    .bind(created_at)
    .bind(parent_comment_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_user_video_comments(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<Vec<crate::api::innertube::VideoComment>> {
    let clean = video_id.trim_start_matches("RD");
    let rows: Vec<(i64, String, String, i64, Option<String>)> = sqlx::query_as(
        "SELECT id, author, content, created_at, parent_comment_id FROM user_video_comments WHERE video_id = ? ORDER BY created_at DESC, id DESC",
    )
    .bind(clean)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|(id, author, text, _ts, parent_id)| crate::api::innertube::VideoComment {
            id: format!("local_{}", id),
            author,
            published_text: "Just now (Saved)".to_string(),
            text,
            like_count_text: "0".to_string(),
            reply_count: 0,
            is_pinned: false,
            is_self: true,
            parent_id,
            ..Default::default()
        })
        .collect())
}

pub async fn set_video_vote(
    pool: &SqlitePool,
    video_id: &str,
    vote: crate::api::innertube::VideoVote,
    updated_at: i64,
) -> Result<()> {
    let clean = video_id.trim_start_matches("RD");
    if vote == crate::api::innertube::VideoVote::None {
        sqlx::query("DELETE FROM user_video_votes WHERE video_id = ?")
            .bind(clean)
            .execute(pool)
            .await?;
    } else {
        sqlx::query(
            "INSERT INTO user_video_votes (video_id, vote, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(video_id) DO UPDATE SET vote = excluded.vote, updated_at = excluded.updated_at",
        )
        .bind(clean)
        .bind(vote.as_db_int())
        .bind(updated_at)
        .execute(pool)
        .await?;
    }
    Ok(())
}

pub async fn get_video_vote(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<crate::api::innertube::VideoVote> {
    let clean = video_id.trim_start_matches("RD");
    let val: Option<i64> = sqlx::query_scalar("SELECT vote FROM user_video_votes WHERE video_id = ?")
        .bind(clean)
        .fetch_optional(pool)
        .await?;
    Ok(val
        .map(crate::api::innertube::VideoVote::from_db_int)
        .unwrap_or_default())
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

    #[tokio::test]
    async fn test_local_and_youtube_watch_history_sync() {
        let pool = init(":memory:").await.unwrap();
        let now = 1_700_000_000i64;
        let v_local = VideoItem {
            id: "loc001".into(),
            title: "Locally Watched Talk".into(),
            uploader: Some("SysLab".into()),
            duration: Some(1200),
            ..Default::default()
        };
        let v_yt = VideoItem {
            id: "yt002".into(),
            title: "Watched On YouTube".into(),
            uploader: Some("RustConf".into()),
            duration: Some(1000),
            ..Default::default()
        };
        let v_rec = VideoItem {
            id: "rec003".into(),
            title: "Partially Watched In Recs".into(),
            uploader: Some("AudioLab".into()),
            duration: Some(200),
            ..Default::default()
        };

        touch_watch_history(&pool, Some(&v_local), &v_local.id, now + 100)
            .await
            .unwrap();
        let local_hist = get_local_watch_history_videos(&pool, 20).await.unwrap();
        assert_eq!(local_hist.len(), 1);
        assert_eq!(local_hist[0].id, "loc001");

        // Sync FEhistory items + Home overlay watched progress into SQLite
        sync_youtube_watched_items(&pool, std::slice::from_ref(&v_yt), &[], true, now + 50)
            .await
            .unwrap();
        sync_youtube_watched_items(
            &pool,
            std::slice::from_ref(&v_rec),
            &[("rec003".to_string(), 75)],
            false,
            now + 60,
        )
        .await
        .unwrap();

        let watched = get_watched_ids(&pool).await.unwrap();
        assert!(watched.contains("loc001"));
        assert!(watched.contains("yt002"));
        assert!(watched.contains("rec003"));

        let prog = get_playback_progress(&pool, "rec003").await.unwrap().unwrap();
        assert!((prog.0 - 150.0).abs() < 0.01);
        assert!((prog.1 - 200.0).abs() < 0.01);

        let full_hist = get_local_watch_history_videos(&pool, 20).await.unwrap();
        assert_eq!(full_hist[0].id, "loc001");
        assert!(full_hist.iter().any(|v| v.id == "yt002"));
    }

    #[tokio::test]
    async fn test_analysis_sessions_isolation_unknown_filtering_and_hour_math() {
        let pool = init(":memory:").await.unwrap();
        let day0 = 1_700_006_400i64; // aligned UTC day + offset testing
        let v_good = VideoItem {
            id: "sess001".into(),
            title: "Systems Engineering Deep Dive".into(),
            uploader: Some("Jon Gjengset".into()),
            channel_id: Some("UC_JON".into()),
            duration: Some(3600),
            ..Default::default()
        };
        let v_unk = VideoItem {
            id: "sess002".into(),
            title: "Clip Without Creator".into(),
            uploader: Some("Unknown".into()),
            duration: Some(120),
            ..Default::default()
        };
        let v_yt = VideoItem {
            id: "ytbulk001".into(),
            title: "Synced Backlog Video".into(),
            uploader: Some("Fireship".into()),
            channel_id: Some("UC_FS".into()),
            duration: Some(600),
            ..Default::default()
        };
        upsert_video(&pool, &v_good, day0).await.unwrap();
        upsert_video(&pool, &v_unk, day0).await.unwrap();
        sync_youtube_watched_items(&pool, std::slice::from_ref(&v_yt), &[], true, day0 + 3600)
            .await
            .unwrap();

        upsert_watch_session(
            &pool,
            &WatchSessionWrite {
                video_id: "sess001",
                channel_id: Some("UC_JON"),
                channel_name: "Jon Gjengset",
                source: "local_ipc",
                started_at: day0 + 72_000, // 20:00
                ended_at: day0 + 74_400,
                watched_secs: 2400.0,
                media_secs: 3600.0,
                start_pos_secs: 0.0,
                end_pos_secs: 3600.0,
                video_duration_secs: 3600.0,
                playback_speed: 1.5,
                sb_skipped_secs: 45.0,
                sb_categories_json: r#"{"sponsor":30.0,"intro":15.0}"#,
                player: "mpv",
                completed: true,
            },
        )
        .await
        .unwrap();
        upsert_watch_session(
            &pool,
            &WatchSessionWrite {
                video_id: "sess002",
                channel_id: None,
                channel_name: "Unknown",
                source: "local_ipc",
                started_at: day0 + 75_000,
                ended_at: day0 + 75_060,
                watched_secs: 60.0,
                media_secs: 60.0,
                start_pos_secs: 0.0,
                end_pos_secs: 60.0,
                video_duration_secs: 120.0,
                playback_speed: 1.0,
                sb_skipped_secs: 0.0,
                sb_categories_json: "{}",
                player: "mpv",
                completed: false,
            },
        )
        .await
        .unwrap();

        // Live Sessions mode excludes YouTube bulk sync & Unknown from ranked Top Channels
        let live = compute_user_analytics_at(
            &pool,
            day0 + 76_000,
            0,
            AnalysisRange::Days7,
            false,
            60,
        )
        .await
        .unwrap();
        assert_eq!(live.top_channels.len(), 1);
        assert_eq!(live.top_channels[0].channel_name, "Jon Gjengset");
        assert_eq!(live.unresolved_channel_videos, 1);
        let bucket_hrs_sum: f64 = live.format_buckets.iter().map(|b| b.actual_hours).sum();
        assert!(
            (bucket_hrs_sum - live.total_hours_watched).abs() < 0.01,
            "Format bucket hours must equal tracked watch hours"
        );
        assert!(live.speed_saved_mins > 19.0);
        assert_eq!(live.sb_categories.len(), 2);
        assert_eq!(live.sb_categories[0].label, "Sponsor");

        // Including YT history adds Fireship to channels WITHOUT creating an hourly spike at 01:00
        let merged = compute_user_analytics_at(
            &pool,
            day0 + 76_000,
            0,
            AnalysisRange::Days7,
            true,
            60,
        )
        .await
        .unwrap();
        assert_eq!(merged.top_channels.len(), 2);
        assert_eq!(
            merged.hourly_minutes[1], 0.0,
            "YouTube bulk sync hour must NOT pollute circadian rhythm"
        );
    }
}
