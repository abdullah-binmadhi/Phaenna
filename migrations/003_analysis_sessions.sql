-- Phaenna Analysis: High-integrity playback session telemetry ledger

CREATE TABLE IF NOT EXISTS watch_sessions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    video_id            TEXT NOT NULL,
    channel_id          TEXT,
    channel_name        TEXT NOT NULL DEFAULT '',
    source              TEXT NOT NULL, -- 'local_ipc' | 'local_external' | 'marked_watched'
    started_at          INTEGER NOT NULL,
    ended_at            INTEGER NOT NULL,
    watched_secs        REAL NOT NULL DEFAULT 0.0,
    media_secs          REAL NOT NULL DEFAULT 0.0,
    start_pos_secs      REAL NOT NULL DEFAULT 0.0,
    end_pos_secs        REAL NOT NULL DEFAULT 0.0,
    video_duration_secs REAL NOT NULL DEFAULT 0.0,
    playback_speed      REAL NOT NULL DEFAULT 1.0,
    sb_skipped_secs     REAL NOT NULL DEFAULT 0.0,
    sb_categories_json  TEXT NOT NULL DEFAULT '{}',
    player              TEXT NOT NULL DEFAULT 'mpv',
    completed           INTEGER NOT NULL DEFAULT 0,
    UNIQUE(video_id, started_at, source)
);

CREATE INDEX IF NOT EXISTS idx_sessions_started ON watch_sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_video   ON watch_sessions(video_id);
CREATE INDEX IF NOT EXISTS idx_sessions_channel ON watch_sessions(channel_name);
