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
    let schema = include_str!("../../migrations/001_initial.sql");
    for stmt in schema.split(';') {
        let trimmed = stmt.trim();
        if trimmed.is_empty() || trimmed.starts_with("--") && !trimmed.contains('\n') {
            continue;
        }
        sqlx::query(trimmed).execute(pool).await?;
    }
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
        ] {
            assert!(
                names.contains(&expected.to_string()),
                "Missing table: {}",
                expected
            );
        }
    }
}
