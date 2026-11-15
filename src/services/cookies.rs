//! Pure-Rust cross-platform browser cookie extractor for YouTube & Google sessions.
//! Reads Mozilla/Zen `cookies.sqlite` safely (copying DB + WAL into a temporary directory
//! so locked live browser instances never block) and emits Netscape cookie format.

use crate::config::CookieSource;
use crate::error::{AppError, Result};
use directories::BaseDirs;
use sqlx::{
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
    Row,
};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Domains preserved when exporting Netscape cookies for `yt-dlp`.
const RELEVANT_DOMAINS: &[&str] = &[
    "youtube.com",
    "youtu.be",
    "google.com",
    "googleusercontent.com",
    "ytimg.com",
];

/// Count active (non-comment, non-empty) Netscape cookie entries on disk.
pub fn get_cookie_count(path: &Path) -> usize {
    let Ok(content) = std::fs::read_to_string(path) else {
        return 0;
    };
    content
        .lines()
        .filter(|l| !l.starts_with('#') && !l.trim().is_empty())
        .count()
}

/// Discover candidate `cookies.sqlite` files ordered by most recent modification time.
pub fn find_browser_cookie_dbs(source: CookieSource) -> Vec<PathBuf> {
    let Some(base) = BaseDirs::new() else {
        return Vec::new();
    };
    let home = base.home_dir();
    let data_dir = base.data_dir(); // ~/Library/Application Support or ~/.local/share or AppData/Roaming

    let mut roots: Vec<PathBuf> = Vec::new();
    if matches!(source, CookieSource::Auto | CookieSource::Zen) {
        roots.push(home.join("Library/Application Support/zen/Profiles"));
        roots.push(data_dir.join("zen/Profiles"));
        roots.push(home.join(".zen"));
    }
    if matches!(source, CookieSource::Auto | CookieSource::Firefox) {
        roots.push(home.join("Library/Application Support/Firefox/Profiles"));
        roots.push(data_dir.join("Mozilla/Firefox/Profiles"));
        roots.push(home.join(".mozilla/firefox"));
    }

    let mut candidates: Vec<(SystemTime, PathBuf)> = Vec::new();
    for root in roots {
        if let Ok(entries) = std::fs::read_dir(&root) {
            for entry in entries.flatten() {
                let db_path = entry.path().join("cookies.sqlite");
                if db_path.is_file() {
                    let mtime = db_path
                        .metadata()
                        .and_then(|m| m.modified())
                        .unwrap_or(SystemTime::UNIX_EPOCH);
                    candidates.push((mtime, db_path));
                }
            }
        }
    }

    candidates.sort_by_key(|a| std::cmp::Reverse(a.0));
    candidates.into_iter().map(|(_, p)| p).collect()
}

/// Copy `cookies.sqlite` (and `-wal` / `-shm` sidecars) to a scratch dir, extract YouTube/Google
/// rows via SQLx, and write Netscape format to `dest`. Returns `(total_written, yt_specific_count)`.
pub async fn extract_sqlite_cookies_to_netscape(
    sqlite_src: &Path,
    dest: &Path,
) -> Result<(usize, usize)> {
    if !sqlite_src.is_file() {
        return Err(AppError::Auth(format!(
            "Cookie database not found: {}",
            sqlite_src.display()
        )));
    }

    let tmp_dir = tempfile::tempdir()?;
    let tmp_db = tmp_dir.path().join("cookies.sqlite");
    std::fs::copy(sqlite_src, &tmp_db)?;

    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", sqlite_src.display(), suffix));
        if sidecar.is_file() {
            let tmp_sidecar = tmp_dir.path().join(format!("cookies.sqlite{}", suffix));
            let _ = std::fs::copy(sidecar, tmp_sidecar);
        }
    }

    let opts = SqliteConnectOptions::new()
        .filename(&tmp_db)
        .read_only(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;

    let rows = sqlx::query(
        "SELECT host, name, value, path, expiry, isSecure FROM moz_cookies",
    )
    .fetch_all(&pool)
    .await?;
    pool.close().await;

    let mut out = String::from("# Netscape HTTP Cookie File\n");
    let mut total_written = 0usize;
    let mut yt_count = 0usize;

    for row in rows {
        let host: String = row.try_get("host").unwrap_or_default();
        let name: String = row.try_get("name").unwrap_or_default();
        let value: String = row.try_get("value").unwrap_or_default();
        let path: String = row.try_get("path").unwrap_or_else(|_| "/".to_string());
        let expiry: i64 = row.try_get("expiry").unwrap_or(0);
        let is_secure: i64 = row.try_get("isSecure").unwrap_or(0);

        if host.is_empty() || name.is_empty() {
            continue;
        }
        let lower_host = host.to_ascii_lowercase();
        if !RELEVANT_DOMAINS.iter().any(|d| lower_host.contains(d)) {
            continue;
        }

        let include_sub = if host.starts_with('.') { "TRUE" } else { "FALSE" };
        let secure_flag = if is_secure != 0 { "TRUE" } else { "FALSE" };
        let safe_val = value.replace(['\t', '\r', '\n'], "");
        let safe_name = name.replace(['\t', '\r', '\n'], "");

        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            host,
            include_sub,
            path,
            secure_flag,
            expiry.max(0),
            safe_name,
            safe_val
        ));
        total_written += 1;
        if lower_host.contains("youtube") || lower_host.contains("google") {
            yt_count += 1;
        }
    }

    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(dest, out)?;

    Ok((total_written, yt_count))
}

/// Synchronize cookies into `dest` from the selected browser source, falling back to the legacy
/// `/opt/homebrew/bin/sync-youtube-cookies` helper if present and native DB discovery yields nothing.
pub async fn sync_browser_cookies(source: CookieSource, dest: &Path) -> Result<(usize, String)> {
    if source == CookieSource::CustomFile {
        let count = get_cookie_count(dest);
        if count > 0 {
            return Ok((
                count,
                format!("Loaded {} cookies from {}", count, dest.display()),
            ));
        }
        return Err(AppError::Auth(format!(
            "Custom cookie file missing or empty: {}",
            dest.display()
        )));
    }

    let dbs = find_browser_cookie_dbs(source);
    let mut last_err = None;
    for db in &dbs {
        match extract_sqlite_cookies_to_netscape(db, dest).await {
            Ok((total, yt)) if total > 0 => {
                let profile_name = db
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "browser".to_string());
                return Ok((
                    total,
                    format!(
                        "Synced {} YouTube/Google cookies from {} ({})",
                        yt, source.label(), profile_name
                    ),
                ));
            }
            Ok(_) => {}
            Err(e) => {
                last_err = Some(e);
            }
        }
    }

    // Legacy fallback if installed on macOS Homebrew
    let legacy_script = Path::new("/opt/homebrew/bin/sync-youtube-cookies");
    if legacy_script.is_file() {
        let out = tokio::process::Command::new(legacy_script)
            .output()
            .await
            .map_err(|e| AppError::Auth(format!("Legacy sync helper failed: {e}")))?;
        if out.status.success() {
            let count = get_cookie_count(dest).max(1);
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            return Ok((count, stdout));
        }
    }

    Err(last_err.unwrap_or_else(|| {
        AppError::Auth(format!(
            "No active YouTube cookies found for {} ({} profile(s) scanned)",
            source.label(),
            dbs.len()
        ))
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::Executor;
    use tempfile::tempdir;

    #[tokio::test]
    async fn test_extract_sqlite_cookies_to_netscape_and_count() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("cookies.sqlite");
        let out_path = dir.path().join("out").join("cookies.txt");

        let opts = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true);
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();

        pool.execute(
            "CREATE TABLE moz_cookies (
                id INTEGER PRIMARY KEY,
                host TEXT,
                name TEXT,
                value TEXT,
                path TEXT,
                expiry INTEGER,
                isSecure INTEGER,
                isHttpOnly INTEGER
            );",
        )
        .await
        .unwrap();

        pool.execute(
            "INSERT INTO moz_cookies (host, name, value, path, expiry, isSecure, isHttpOnly) VALUES
            ('.youtube.com', 'SID', 'secret_sid', '/', 1999999999, 1, 1),
            ('.google.com', 'HSID', 'secret_hsid', '/', 1999999999, 1, 0),
            ('.example.com', 'OTHER', 'skip_me', '/', 1999999999, 0, 0);",
        )
        .await
        .unwrap();
        pool.close().await;

        let (total, yt) = extract_sqlite_cookies_to_netscape(&db_path, &out_path)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(yt, 2);
        assert_eq!(get_cookie_count(&out_path), 2);

        let body = std::fs::read_to_string(&out_path).unwrap();
        assert!(body.starts_with("# Netscape HTTP Cookie File\n"));
        assert!(body.contains(".youtube.com\tTRUE\t/\tTRUE\t1999999999\tSID\tsecret_sid"));
        assert!(!body.contains("example.com"));
    }
}
