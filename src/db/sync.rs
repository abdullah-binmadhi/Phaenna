//! TTL-based sync metadata for Stale-While-Revalidate (SWR) feeds.

use crate::error::Result;
use sqlx::SqlitePool;

pub const RECOMMENDED: &str = "recommended";
pub const SUBSCRIPTIONS: &str = "subscriptions";
pub const HISTORY: &str = "history";
pub const WATCH_LATER: &str = "watch_later";

#[allow(dead_code)]
pub const ALL_FEEDS: &[&str] = &[RECOMMENDED, SUBSCRIPTIONS, HISTORY, WATCH_LATER];

/// Record the timestamp when a feed was successfully fetched.
pub async fn mark_synced(pool: &SqlitePool, key: &str, now: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO sync_meta (key, synced_at) VALUES (?, ?)
         ON CONFLICT(key) DO UPDATE SET synced_at = excluded.synced_at",
    )
    .bind(key)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

/// Read the last sync UNIX timestamp for `key`.
pub async fn last_synced(pool: &SqlitePool, key: &str) -> Result<Option<i64>> {
    let ts: Option<i64> = sqlx::query_scalar("SELECT synced_at FROM sync_meta WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await?;
    Ok(ts)
}

/// Returns `true` when never synced or elapsed time exceeds `ttl`.
pub async fn is_stale(pool: &SqlitePool, key: &str, ttl: u64, now: i64) -> Result<bool> {
    match last_synced(pool, key).await? {
        None => Ok(true),
        Some(ts) => Ok(now - ts > ttl as i64),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::init;

    #[tokio::test]
    async fn test_never_synced_is_stale() {
        let pool = init(":memory:").await.unwrap();
        assert!(is_stale(&pool, RECOMMENDED, 1800, 1_700_000_000).await.unwrap());
    }

    #[tokio::test]
    async fn test_fresh_vs_expired_ttl() {
        let pool = init(":memory:").await.unwrap();
        let t0 = 1_700_000_000i64;
        mark_synced(&pool, SUBSCRIPTIONS, t0).await.unwrap();
        assert!(!is_stale(&pool, SUBSCRIPTIONS, 900, t0 + 300).await.unwrap());
        assert!(is_stale(&pool, SUBSCRIPTIONS, 900, t0 + 1200).await.unwrap());
    }
}
