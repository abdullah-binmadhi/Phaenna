//! SQLite persistence for local Karaoke playlists, per-track vocal/pitch/sync preferences,
//! and offline/cached synced lyrics readiness.

use crate::api::ytmusic::{YtmLyrics, YtmTrack};
use crate::error::Result;
use sqlx::{Row, SqlitePool};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KaraokeLyricsReadiness {
    Unknown,
    Checking,
    SyncedWord,
    SyncedLine,
    PlainOnly,
    Unavailable,
}

impl KaraokeLyricsReadiness {
    pub fn from_lyrics(lyrics: &YtmLyrics) -> Self {
        if lyrics.lines.is_empty() {
            Self::Unavailable
        } else if !lyrics.is_timed {
            Self::PlainOnly
        } else if lyrics.has_word_timings() {
            Self::SyncedWord
        } else {
            Self::SyncedLine
        }
    }

    pub fn from_cached_flags(is_timed: i64, has_word_timings: i64, line_count_nonempty: bool) -> Self {
        if !line_count_nonempty {
            Self::Unavailable
        } else if is_timed == 0 {
            Self::PlainOnly
        } else if has_word_timings != 0 {
            Self::SyncedWord
        } else {
            Self::SyncedLine
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::Unknown => "· Check",
            Self::Checking => "… Fetch",
            Self::SyncedWord => "● Word",
            Self::SyncedLine => "● Sync",
            Self::PlainOnly => "○ Plain",
            Self::Unavailable => "× None",
        }
    }

    pub fn is_synced(self) -> bool {
        matches!(self, Self::SyncedWord | Self::SyncedLine)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaraokePlaylistSummary {
    pub id: i64,
    pub name: String,
    pub is_default: bool,
    pub item_count: i64,
    pub synced_count: i64,
    pub total_duration_secs: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaraokePlaylistEntry {
    pub id: i64,
    pub playlist_id: i64,
    pub video_id: String,
    pub ytm_playlist_id: Option<String>,
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub thumb_url: String,
    pub duration_secs: Option<u64>,
    pub singer_name: Option<String>,
    pub pitch_semitones: i8,
    pub lyric_offset_ms: i64,
    pub sort_order: i64,
    pub sung_count: i64,
    pub lyrics_status: KaraokeLyricsReadiness,
}

impl KaraokePlaylistEntry {
    pub fn to_ytm_track(&self) -> YtmTrack {
        YtmTrack {
            video_id: self.video_id.clone(),
            title: self.title.clone(),
            artist: self.artist.clone(),
            artist_browse_id: None,
            album: self.album.clone(),
            album_browse_id: None,
            duration_text: self
                .duration_secs
                .map(crate::db::cache::format_duration_secs),
            duration_secs: self.duration_secs,
            plays_text: None,
            thumb_url: self.thumb_url.clone(),
            playlist_id: self.ytm_playlist_id.clone(),
            kind: crate::api::ytmusic::YtmItemKind::Song,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KaraokeTrackPref {
    pub video_id: String,
    pub pitch_semitones: i8,
    pub lyric_offset_ms: i64,
    pub vocal_mode: u8,
    pub speed_pct: u16,
    pub loop_start_ms: Option<u64>,
    pub loop_end_ms: Option<u64>,
}

impl Default for KaraokeTrackPref {
    fn default() -> Self {
        Self {
            video_id: String::new(),
            pitch_semitones: 0,
            lyric_offset_ms: 0,
            vocal_mode: 0,
            speed_pct: 100,
            loop_start_ms: None,
            loop_end_ms: None,
        }
    }
}

pub async fn ensure_default_karaoke_playlist(pool: &SqlitePool, now: i64) -> Result<i64> {
    if let Some(existing) = sqlx::query_scalar::<_, i64>(
        "SELECT id FROM karaoke_playlists WHERE is_default = 1 ORDER BY id ASC LIMIT 1",
    )
    .fetch_optional(pool)
    .await?
    {
        return Ok(existing);
    }
    if let Some(any_id) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM karaoke_playlists ORDER BY id ASC LIMIT 1")
            .fetch_optional(pool)
            .await?
    {
        sqlx::query("UPDATE karaoke_playlists SET is_default = 1 WHERE id = ?1")
            .bind(any_id)
            .execute(pool)
            .await?;
        return Ok(any_id);
    }
    let res = sqlx::query(
        "INSERT INTO karaoke_playlists (name, is_default, created_at, updated_at)
         VALUES ('Stage Queue', 1, ?1, ?1)",
    )
    .bind(now)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

pub async fn list_karaoke_playlists(pool: &SqlitePool) -> Result<Vec<KaraokePlaylistSummary>> {
    let rows = sqlx::query(
        r#"
        SELECT
            p.id,
            p.name,
            p.is_default,
            COUNT(i.id) AS item_count,
            COALESCE(SUM(CASE WHEN c.is_timed = 1 THEN 1 ELSE 0 END), 0) AS synced_count,
            COALESCE(SUM(COALESCE(i.duration_secs, 0)), 0) AS total_duration_secs
        FROM karaoke_playlists p
        LEFT JOIN karaoke_playlist_items i ON i.playlist_id = p.id
        LEFT JOIN karaoke_lyrics_cache c ON c.video_id = i.video_id
        GROUP BY p.id
        ORDER BY p.is_default DESC, p.id ASC
        "#,
    )
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| KaraokePlaylistSummary {
            id: r.get("id"),
            name: r.get("name"),
            is_default: r.get::<i64, _>("is_default") != 0,
            item_count: r.get("item_count"),
            synced_count: r.get("synced_count"),
            total_duration_secs: r.get("total_duration_secs"),
        })
        .collect())
}

pub async fn create_karaoke_playlist(pool: &SqlitePool, name: &str, now: i64) -> Result<i64> {
    let clean = name.trim();
    let final_name = if clean.is_empty() {
        format!("Karaoke Set {now}")
    } else {
        clean.to_string()
    };
    if let Some(existing) =
        sqlx::query_scalar::<_, i64>("SELECT id FROM karaoke_playlists WHERE name = ?1")
            .bind(&final_name)
            .fetch_optional(pool)
            .await?
    {
        return Ok(existing);
    }
    let res = sqlx::query(
        "INSERT INTO karaoke_playlists (name, is_default, created_at, updated_at)
         VALUES (?1, 0, ?2, ?2)",
    )
    .bind(&final_name)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(res.last_insert_rowid())
}

#[allow(dead_code)]
pub async fn rename_karaoke_playlist(
    pool: &SqlitePool,
    playlist_id: i64,
    name: &str,
    now: i64,
) -> Result<()> {
    let clean = name.trim();
    if clean.is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE karaoke_playlists SET name = ?1, updated_at = ?2 WHERE id = ?3")
        .bind(clean)
        .bind(now)
        .bind(playlist_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete_karaoke_playlist(pool: &SqlitePool, playlist_id: i64, now: i64) -> Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM karaoke_playlists")
        .fetch_one(pool)
        .await?;
    if count <= 1 {
        sqlx::query("DELETE FROM karaoke_playlist_items WHERE playlist_id = ?1")
            .bind(playlist_id)
            .execute(pool)
            .await?;
        return Ok(());
    }
    sqlx::query("DELETE FROM karaoke_playlists WHERE id = ?1")
        .bind(playlist_id)
        .execute(pool)
        .await?;
    let _ = ensure_default_karaoke_playlist(pool, now).await?;
    Ok(())
}

pub async fn list_karaoke_items(
    pool: &SqlitePool,
    playlist_id: i64,
) -> Result<Vec<KaraokePlaylistEntry>> {
    let rows = sqlx::query(
        r#"
        SELECT
            i.id,
            i.playlist_id,
            i.video_id,
            i.ytm_playlist_id,
            i.title,
            i.artist,
            i.album,
            i.thumb_url,
            i.duration_secs,
            i.singer_name,
            COALESCE(tp.pitch_semitones, i.pitch_semitones, 0) AS eff_pitch,
            COALESCE(tp.lyric_offset_ms, i.lyric_offset_ms, 0) AS eff_offset,
            i.sort_order,
            i.sung_count,
            c.is_timed AS cache_timed,
            c.has_word_timings AS cache_word,
            c.lyrics_json AS cache_json
        FROM karaoke_playlist_items i
        LEFT JOIN karaoke_track_prefs tp ON tp.video_id = i.video_id
        LEFT JOIN karaoke_lyrics_cache c ON c.video_id = i.video_id
        WHERE i.playlist_id = ?1
        ORDER BY i.sort_order ASC, i.id ASC
        "#,
    )
    .bind(playlist_id)
    .fetch_all(pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let status = match (
                r.get::<Option<i64>, _>("cache_timed"),
                r.get::<Option<i64>, _>("cache_word"),
                r.get::<Option<String>, _>("cache_json"),
            ) {
                (Some(timed), Some(word), Some(json_s)) => {
                    let has_lines = !json_s.trim().is_empty() && json_s != "[]";
                    KaraokeLyricsReadiness::from_cached_flags(timed, word, has_lines)
                }
                _ => KaraokeLyricsReadiness::Unknown,
            };
            let dur_i: Option<i64> = r.get("duration_secs");
            let pitch_i: i64 = r.get("eff_pitch");
            KaraokePlaylistEntry {
                id: r.get("id"),
                playlist_id: r.get("playlist_id"),
                video_id: r.get("video_id"),
                ytm_playlist_id: r.get("ytm_playlist_id"),
                title: r.get("title"),
                artist: r.get("artist"),
                album: r.get("album"),
                thumb_url: r.get("thumb_url"),
                duration_secs: dur_i.filter(|&d| d >= 0).map(|d| d as u64),
                singer_name: r.get("singer_name"),
                pitch_semitones: pitch_i.clamp(-12, 12) as i8,
                lyric_offset_ms: r.get("eff_offset"),
                sort_order: r.get("sort_order"),
                sung_count: r.get("sung_count"),
                lyrics_status: status,
            }
        })
        .collect())
}

pub async fn add_karaoke_track(
    pool: &SqlitePool,
    playlist_id: i64,
    track: &YtmTrack,
    singer_name: Option<&str>,
    now: i64,
) -> Result<i64> {
    let pref = get_karaoke_track_pref(pool, &track.video_id).await?;
    let pitch = pref.as_ref().map(|p| p.pitch_semitones as i64).unwrap_or(0);
    let offset = pref.as_ref().map(|p| p.lyric_offset_ms).unwrap_or(0);

    let max_order: Option<i64> = sqlx::query_scalar(
        "SELECT MAX(sort_order) FROM karaoke_playlist_items WHERE playlist_id = ?1",
    )
    .bind(playlist_id)
    .fetch_one(pool)
    .await?;
    let next_order = max_order.unwrap_or(-1) + 1;

    let dur = track.duration_secs.map(|d| d as i64);
    let res = sqlx::query(
        r#"
        INSERT INTO karaoke_playlist_items (
            playlist_id, video_id, ytm_playlist_id, title, artist, album,
            thumb_url, duration_secs, singer_name, pitch_semitones,
            lyric_offset_ms, sort_order, sung_count, added_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, 0, ?13)
        "#,
    )
    .bind(playlist_id)
    .bind(&track.video_id)
    .bind(&track.playlist_id)
    .bind(&track.title)
    .bind(&track.artist)
    .bind(&track.album)
    .bind(track.effective_thumb_url())
    .bind(dur)
    .bind(singer_name.map(str::trim).filter(|s| !s.is_empty()))
    .bind(pitch)
    .bind(offset)
    .bind(next_order)
    .bind(now)
    .execute(pool)
    .await?;

    sqlx::query("UPDATE karaoke_playlists SET updated_at = ?1 WHERE id = ?2")
        .bind(now)
        .bind(playlist_id)
        .execute(pool)
        .await?;

    Ok(res.last_insert_rowid())
}

pub async fn move_karaoke_item(
    pool: &SqlitePool,
    playlist_id: i64,
    item_id: i64,
    direction: i32,
) -> Result<()> {
    let rows = sqlx::query_as::<_, (i64,)>(
        "SELECT id FROM karaoke_playlist_items WHERE playlist_id = ?1 ORDER BY sort_order ASC, id ASC",
    )
    .bind(playlist_id)
    .fetch_all(pool)
    .await?;
    let mut ids: Vec<i64> = rows.into_iter().map(|r| r.0).collect();
    let Some(pos) = ids.iter().position(|&id| id == item_id) else {
        return Ok(());
    };
    let target = (pos as isize + direction as isize)
        .clamp(0, ids.len().saturating_sub(1) as isize) as usize;
    if target == pos {
        return Ok(());
    }
    ids.swap(pos, target);
    for (idx, id) in ids.into_iter().enumerate() {
        sqlx::query("UPDATE karaoke_playlist_items SET sort_order = ?1 WHERE id = ?2")
            .bind(idx as i64)
            .bind(id)
            .execute(pool)
            .await?;
    }
    Ok(())
}

pub async fn delete_karaoke_item(pool: &SqlitePool, item_id: i64) -> Result<()> {
    sqlx::query("DELETE FROM karaoke_playlist_items WHERE id = ?1")
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_karaoke_item_singer(
    pool: &SqlitePool,
    item_id: i64,
    singer: &str,
) -> Result<()> {
    let clean = singer.trim();
    let val = (!clean.is_empty()).then_some(clean.to_string());
    sqlx::query("UPDATE karaoke_playlist_items SET singer_name = ?1 WHERE id = ?2")
        .bind(val)
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn increment_karaoke_sung_count(pool: &SqlitePool, item_id: i64) -> Result<()> {
    sqlx::query("UPDATE karaoke_playlist_items SET sung_count = sung_count + 1 WHERE id = ?1")
        .bind(item_id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn swap_karaoke_item_source(
    pool: &SqlitePool,
    item_id: i64,
    replacement: &YtmTrack,
) -> Result<()> {
    let dur = replacement.duration_secs.map(|d| d as i64);
    sqlx::query(
        r#"
        UPDATE karaoke_playlist_items SET
            video_id = ?1,
            ytm_playlist_id = ?2,
            title = ?3,
            artist = ?4,
            thumb_url = ?5,
            duration_secs = ?6
        WHERE id = ?7
        "#,
    )
    .bind(&replacement.video_id)
    .bind(&replacement.playlist_id)
    .bind(&replacement.title)
    .bind(&replacement.artist)
    .bind(replacement.effective_thumb_url())
    .bind(dur)
    .bind(item_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_karaoke_lyrics_cache(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<Option<YtmLyrics>> {
    let row = sqlx::query(
        "SELECT source, is_timed, lyrics_json FROM karaoke_lyrics_cache WHERE video_id = ?1",
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?;
    let Some(r) = row else {
        return Ok(None);
    };
    let source: Option<String> = r.get("source");
    let is_timed: i64 = r.get("is_timed");
    let json_s: String = r.get("lyrics_json");
    let lines = serde_json::from_str(&json_s).unwrap_or_default();
    Ok(Some(YtmLyrics {
        lines,
        source,
        is_timed: is_timed != 0,
    }))
}

pub async fn upsert_karaoke_lyrics_cache(
    pool: &SqlitePool,
    video_id: &str,
    lyrics: &YtmLyrics,
    now: i64,
) -> Result<()> {
    let json_s = serde_json::to_string(&lyrics.lines)
        .map_err(|e| crate::error::AppError::Parse(e.to_string()))?;
    let has_words = if lyrics.has_word_timings() { 1_i64 } else { 0_i64 };
    let is_timed = if lyrics.is_timed { 1_i64 } else { 0_i64 };
    sqlx::query(
        r#"
        INSERT INTO karaoke_lyrics_cache (
            video_id, source, is_timed, has_word_timings, lyrics_json, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        ON CONFLICT(video_id) DO UPDATE SET
            source           = excluded.source,
            is_timed         = excluded.is_timed,
            has_word_timings = excluded.has_word_timings,
            lyrics_json      = excluded.lyrics_json,
            updated_at       = excluded.updated_at
        "#,
    )
    .bind(video_id)
    .bind(&lyrics.source)
    .bind(is_timed)
    .bind(has_words)
    .bind(json_s)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_karaoke_track_pref(
    pool: &SqlitePool,
    video_id: &str,
) -> Result<Option<KaraokeTrackPref>> {
    let row = sqlx::query(
        r#"
        SELECT video_id, pitch_semitones, lyric_offset_ms, vocal_mode,
               speed_pct, loop_start_ms, loop_end_ms
        FROM karaoke_track_prefs WHERE video_id = ?1
        "#,
    )
    .bind(video_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|r| {
        let pitch: i64 = r.get("pitch_semitones");
        let vocal: i64 = r.get("vocal_mode");
        let spd: i64 = r.get("speed_pct");
        let la: Option<i64> = r.get("loop_start_ms");
        let lb: Option<i64> = r.get("loop_end_ms");
        KaraokeTrackPref {
            video_id: r.get("video_id"),
            pitch_semitones: pitch.clamp(-12, 12) as i8,
            lyric_offset_ms: r.get("lyric_offset_ms"),
            vocal_mode: vocal.clamp(0, 2) as u8,
            speed_pct: spd.clamp(50, 150) as u16,
            loop_start_ms: la.filter(|&v| v >= 0).map(|v| v as u64),
            loop_end_ms: lb.filter(|&v| v >= 0).map(|v| v as u64),
        }
    }))
}

pub async fn upsert_karaoke_track_pref(
    pool: &SqlitePool,
    pref: &KaraokeTrackPref,
    now: i64,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO karaoke_track_prefs (
            video_id, pitch_semitones, lyric_offset_ms, vocal_mode,
            speed_pct, loop_start_ms, loop_end_ms, updated_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        ON CONFLICT(video_id) DO UPDATE SET
            pitch_semitones = excluded.pitch_semitones,
            lyric_offset_ms = excluded.lyric_offset_ms,
            vocal_mode      = excluded.vocal_mode,
            speed_pct       = excluded.speed_pct,
            loop_start_ms   = excluded.loop_start_ms,
            loop_end_ms     = excluded.loop_end_ms,
            updated_at      = excluded.updated_at
        "#,
    )
    .bind(&pref.video_id)
    .bind(pref.pitch_semitones as i64)
    .bind(pref.lyric_offset_ms)
    .bind(pref.vocal_mode as i64)
    .bind(pref.speed_pct as i64)
    .bind(pref.loop_start_ms.map(|v| v as i64))
    .bind(pref.loop_end_ms.map(|v| v as i64))
    .bind(now)
    .execute(pool)
    .await?;

    sqlx::query(
        "UPDATE karaoke_playlist_items SET pitch_semitones = ?1, lyric_offset_ms = ?2 WHERE video_id = ?3",
    )
    .bind(pref.pitch_semitones as i64)
    .bind(pref.lyric_offset_ms)
    .bind(&pref.video_id)
    .execute(pool)
    .await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ytmusic::{YtmItemKind, YtmLyricLine, YtmLyricWord};

    #[tokio::test]
    async fn test_karaoke_playlist_crud_prefs_and_lyrics_cache() {
        let pool = crate::db::init(":memory:").await.unwrap();
        let def_id = ensure_default_karaoke_playlist(&pool, 1000).await.unwrap();
        assert_eq!(ensure_default_karaoke_playlist(&pool, 1005).await.unwrap(), def_id);

        let party_id = create_karaoke_playlist(&pool, "Friday Night", 1010)
            .await
            .unwrap();
        assert_ne!(party_id, def_id);

        let tr1 = YtmTrack {
            video_id: "vid_1".to_string(),
            title: "Starlight".to_string(),
            artist: "Muse".to_string(),
            artist_browse_id: None,
            album: None,
            album_browse_id: None,
            duration_text: Some("4:00".to_string()),
            duration_secs: Some(240),
            plays_text: None,
            thumb_url: String::new(),
            playlist_id: None,
            kind: YtmItemKind::Song,
        };
        let tr2 = YtmTrack {
            video_id: "vid_2".to_string(),
            title: "Lips".to_string(),
            artist: "Jane Remover".to_string(),
            artist_browse_id: None,
            album: None,
            album_browse_id: None,
            duration_text: Some("5:11".to_string()),
            duration_secs: Some(311),
            plays_text: None,
            thumb_url: String::new(),
            playlist_id: None,
            kind: YtmItemKind::Song,
        };

        let id1 = add_karaoke_track(&pool, party_id, &tr1, Some("Alex"), 1020)
            .await
            .unwrap();
        let id2 = add_karaoke_track(&pool, party_id, &tr2, Some("Sam"), 1021)
            .await
            .unwrap();

        let lyr = YtmLyrics {
            lines: vec![YtmLyricLine {
                text: "Far away".to_string(),
                start_ms: Some(1000),
                end_ms: Some(4000),
                words: Some(vec![
                    YtmLyricWord {
                        text: "Far ".to_string(),
                        start_ms: 1000,
                        end_ms: 2200,
                    },
                    YtmLyricWord {
                        text: "away".to_string(),
                        start_ms: 2200,
                        end_ms: 4000,
                    },
                ]),
            }],
            source: Some("LRCLIB".to_string()),
            is_timed: true,
        };
        upsert_karaoke_lyrics_cache(&pool, "vid_1", &lyr, 1030)
            .await
            .unwrap();
        upsert_karaoke_track_pref(
            &pool,
            &KaraokeTrackPref {
                video_id: "vid_1".to_string(),
                pitch_semitones: -2,
                lyric_offset_ms: 250,
                vocal_mode: 1,
                speed_pct: 95,
                loop_start_ms: Some(1000),
                loop_end_ms: Some(4000),
            },
            1035,
        )
        .await
        .unwrap();

        move_karaoke_item(&pool, party_id, id2, -1).await.unwrap();
        update_karaoke_item_singer(&pool, id1, "Maya").await.unwrap();
        increment_karaoke_sung_count(&pool, id1).await.unwrap();

        let items = list_karaoke_items(&pool, party_id).await.unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].video_id, "vid_2");
        assert_eq!(items[1].video_id, "vid_1");
        assert_eq!(items[1].singer_name.as_deref(), Some("Maya"));
        assert_eq!(items[1].pitch_semitones, -2);
        assert_eq!(items[1].lyric_offset_ms, 250);
        assert_eq!(items[1].sung_count, 1);
        assert_eq!(items[1].lyrics_status, KaraokeLyricsReadiness::SyncedWord);

        let cached = get_karaoke_lyrics_cache(&pool, "vid_1")
            .await
            .unwrap()
            .unwrap();
        assert!(cached.has_word_timings());
        assert_eq!(cached.lines[0].words.as_ref().unwrap().len(), 2);

        let pls = list_karaoke_playlists(&pool).await.unwrap();
        let friday = pls.iter().find(|p| p.id == party_id).unwrap();
        assert_eq!(friday.item_count, 2);
        assert_eq!(friday.synced_count, 1);
        assert_eq!(friday.total_duration_secs, 551);
    }
}
