pub mod cache;
pub mod sync;
pub mod user;

use crate::error::Result;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    SqlitePool,
};
use std::str::FromStr;

/// Initialize the high-performance SQLite WAL connection pool and run idempotent migrations.
pub async fn init(db_path: &str) -> Result<SqlitePool> {
    if db_path != ":memory:" {
        if let Some(parent) = std::path::Path::new(db_path).parent() {
            std::fs::create_dir_all(parent)?;
        }
    }

    let url = if db_path == ":memory:" {
        "sqlite::memory:".to_string()
    } else {
        format!("sqlite://{}?mode=rwc", db_path)
    };

    let opts = SqliteConnectOptions::from_str(&url)?.create_if_missing(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(if db_path == ":memory:" { 1 } else { 5 })
        .connect_with(opts)
        .await?;

    let _ = sqlx::query("PRAGMA journal_mode = WAL;").execute(&pool).await;
    let _ = sqlx::query("PRAGMA synchronous = NORMAL;").execute(&pool).await;
    let _ = sqlx::query("PRAGMA busy_timeout = 5000;").execute(&pool).await;
    let _ = sqlx::query("PRAGMA temp_store = MEMORY;").execute(&pool).await;

    run_migrations(&pool).await?;
    Ok(pool)
}

async fn run_migrations(pool: &SqlitePool) -> Result<()> {
    for schema in [
        include_str!("../../migrations/001_initial.sql"),
        include_str!("../../migrations/002_google_auth.sql"),
        include_str!("../../migrations/003_analysis_sessions.sql"),
    ] {
        for stmt in schema.split(';') {
            let trimmed = stmt.trim();
            if trimmed.is_empty() || trimmed.starts_with("--") && !trimmed.contains('\n') {
                continue;
            }
            sqlx::query(trimmed).execute(pool).await?;
        }
    }
    ensure_column_exists(
        pool,
        "continue_watching",
        "pinned_manually",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_exists(
        pool,
        "continue_watching",
        "queue_order",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_exists(
        pool,
        "continue_watching",
        "added_at",
        "INTEGER NOT NULL DEFAULT 0",
    )
    .await?;
    ensure_column_exists(pool, "videos", "published_text", "TEXT").await?;
    ensure_column_exists(
        pool,
        "watch_history",
        "origin",
        "TEXT NOT NULL DEFAULT 'youtube_sync'",
    )
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_video_comments (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            video_id TEXT NOT NULL,
            author TEXT NOT NULL,
            content TEXT NOT NULL,
            created_at INTEGER NOT NULL,
            parent_comment_id TEXT
        )",
    )
    .execute(pool)
    .await?;
    ensure_column_exists(pool, "user_video_comments", "parent_comment_id", "TEXT").await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS user_video_votes (
            video_id TEXT PRIMARY KEY,
            vote INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS thumb_blobs (
            video_id TEXT PRIMARY KEY,
            blob BLOB NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_column_exists(
    pool: &SqlitePool,
    table: &str,
    column: &str,
    col_def: &str,
) -> Result<()> {
    let exists: Option<i64> =
        sqlx::query_scalar("SELECT 1 FROM pragma_table_info(?) WHERE name = ?")
            .bind(table)
            .bind(column)
            .fetch_optional(pool)
            .await?;
    if exists.is_none() {
        let sql = format!("ALTER TABLE {} ADD COLUMN {} {}", table, column, col_def);
        sqlx::query(&sql).execute(pool).await?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub struct AccountRow {
    pub account_id: String,
    pub email: String,
    pub display_name: String,
    pub channel_id: Option<String>,
    pub channel_handle: Option<String>,
    pub avatar_url: Option<String>,
    pub auth_kind: String,
    pub is_active: bool,
    pub added_at: i64,
    pub last_refreshed: i64,
}

#[allow(dead_code)]
pub async fn upsert_account(pool: &SqlitePool, acct: &AccountRow) -> Result<()> {
    if acct.is_active {
        sqlx::query("UPDATE accounts SET is_active = 0")
            .execute(pool)
            .await?;
    }
    sqlx::query(
        r#"
        INSERT INTO accounts (
            account_id, email, display_name, channel_id, channel_handle,
            avatar_url, auth_kind, is_active, added_at, last_refreshed
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(account_id) DO UPDATE SET
            email          = excluded.email,
            display_name   = excluded.display_name,
            channel_id     = excluded.channel_id,
            channel_handle = excluded.channel_handle,
            avatar_url     = excluded.avatar_url,
            auth_kind      = excluded.auth_kind,
            is_active      = excluded.is_active,
            last_refreshed = excluded.last_refreshed
        "#,
    )
    .bind(&acct.account_id)
    .bind(&acct.email)
    .bind(&acct.display_name)
    .bind(&acct.channel_id)
    .bind(&acct.channel_handle)
    .bind(&acct.avatar_url)
    .bind(&acct.auth_kind)
    .bind(if acct.is_active { 1_i64 } else { 0_i64 })
    .bind(acct.added_at)
    .bind(acct.last_refreshed)
    .execute(pool)
    .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn set_active_account(pool: &SqlitePool, account_id: &str) -> Result<()> {
    sqlx::query("UPDATE accounts SET is_active = 0")
        .execute(pool)
        .await?;
    sqlx::query("UPDATE accounts SET is_active = 1 WHERE account_id = ?1")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[allow(dead_code)]
pub async fn delete_account(pool: &SqlitePool, account_id: &str) -> Result<()> {
    sqlx::query("DELETE FROM accounts WHERE account_id = ?1")
        .bind(account_id)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_init_and_idempotent_migrations() {
        let pool = init(":memory:").await.unwrap();
        // Run migrations a second time to verify idempotency
        run_migrations(&pool).await.unwrap();

        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        let names: Vec<String> = tables.into_iter().map(|t| t.0).collect();

        for expected in [
            "accounts",
            "channels",
            "continue_watching",
            "feed_entries",
            "offline_videos",
            "playback_prefs",
            "playback_progress",
            "sponsorblock_segments",
            "sync_meta",
            "telemetry_counters",
            "videos",
            "watch_history",
            "watch_later",
            "watch_sessions",
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "Missing table: {}",
                expected
            );
        }
    }
}
