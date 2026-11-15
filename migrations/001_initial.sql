-- Phaenna SQLite Initial Schema

CREATE TABLE IF NOT EXISTS channels (
    id              TEXT PRIMARY KEY,
    handle          TEXT,
    name            TEXT NOT NULL,
    avatar_url      TEXT,
    avatar_blob     BLOB,
    subscriber_text TEXT,
    is_subscribed   INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS videos (
    id              TEXT PRIMARY KEY,
    channel_id      TEXT,
    channel_name    TEXT NOT NULL,
    title           TEXT NOT NULL,
    description     TEXT,
    duration_secs   INTEGER,
    duration_text   TEXT,
    view_count      INTEGER,
    like_count      INTEGER,
    dislike_count   INTEGER,
    published_at    INTEGER,
    webpage_url     TEXT,
    chapters_json   TEXT NOT NULL DEFAULT '[]',
    tags_json       TEXT NOT NULL DEFAULT '[]',
    thumbnails_json TEXT NOT NULL DEFAULT '[]',
    thumb_url       TEXT,
    thumb_blob      BLOB,
    is_live         INTEGER NOT NULL DEFAULT 0,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS feed_entries (
    feed_key        TEXT NOT NULL,
    video_id        TEXT NOT NULL REFERENCES videos(id),
    position        INTEGER NOT NULL,
    PRIMARY KEY (feed_key, video_id)
);

CREATE TABLE IF NOT EXISTS watch_history (
    video_id        TEXT PRIMARY KEY REFERENCES videos(id),
    watched_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS continue_watching (
    video_id        TEXT PRIMARY KEY REFERENCES videos(id),
    last_pos_secs   REAL NOT NULL DEFAULT 0.0,
    duration_secs   REAL NOT NULL DEFAULT 0.0,
    last_watched_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS watch_later (
    video_id        TEXT PRIMARY KEY REFERENCES videos(id),
    source          TEXT NOT NULL DEFAULT 'local', -- 'youtube' | 'local' | 'both'
    added_at        INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS playback_progress (
    video_id        TEXT PRIMARY KEY REFERENCES videos(id),
    position_secs   REAL NOT NULL DEFAULT 0.0,
    duration_secs   REAL NOT NULL DEFAULT 0.0,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sponsorblock_segments (
    video_id        TEXT NOT NULL REFERENCES videos(id),
    segment_uuid    TEXT NOT NULL,
    category        TEXT NOT NULL,
    start_secs      REAL NOT NULL,
    end_secs        REAL NOT NULL,
    fetched_at      INTEGER NOT NULL,
    PRIMARY KEY (video_id, segment_uuid)
);

CREATE TABLE IF NOT EXISTS offline_videos (
    video_id        TEXT PRIMARY KEY REFERENCES videos(id),
    file_path       TEXT NOT NULL,
    file_bytes      INTEGER NOT NULL DEFAULT 0,
    format_note     TEXT,
    downloaded_at   INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS playback_prefs (
    video_id        TEXT PRIMARY KEY REFERENCES videos(id),
    preferred_lang  TEXT,
    quality         TEXT,
    shader_mode     TEXT,
    updated_at      INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_meta (
    key             TEXT PRIMARY KEY,
    synced_at       INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS telemetry_counters (
    key             TEXT PRIMARY KEY,
    value_f64       REAL NOT NULL DEFAULT 0.0
);

CREATE INDEX IF NOT EXISTS idx_videos_channel ON videos(channel_id);
CREATE INDEX IF NOT EXISTS idx_videos_published ON videos(published_at DESC);
CREATE INDEX IF NOT EXISTS idx_feed_pos ON feed_entries(feed_key, position);
CREATE INDEX IF NOT EXISTS idx_history_ts ON watch_history(watched_at DESC);
CREATE INDEX IF NOT EXISTS idx_cw_ts ON continue_watching(last_watched_at DESC);
