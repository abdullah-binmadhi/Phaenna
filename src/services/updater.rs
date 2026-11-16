//! Non-blocking GitHub Releases SemVer + Rolling Main Commit Auto-Updater for `phaenna`.
//!
//! Mirrors Solus's release & auto-update architecture:
//! - Evaluates both tagged SemVer releases (`v1.0.1`) and rolling `main` commit releases (`v1.0.0-rolling`).
//! - Compares embedded `<!-- phaenna-sha:... -->` & `<!-- phaenna-ts:... -->` against `PHAENNA_GIT_SHA` & `PHAENNA_COMMIT_TS`.
//! - Streams binary downloads with live speed/percentage progress and atomically replaces the executable.

use crate::error::{AppError, Result};
use crate::messages::AppMessage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::Sender;

const GITHUB_RELEASES_URL: &str =
    "https://api.github.com/repos/abdullah-binmadhi/Phaenna/releases/latest";
const USER_AGENT: &str = concat!("phaenna/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseAsset {
    pub name: String,
    pub browser_download_url: String,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubRelease {
    pub tag_name: String,
    pub name: Option<String>,
    pub body: Option<String>,
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
    pub asset_name: String,
    pub release_notes: Option<String>,
    pub is_homebrew: bool,
}

pub fn parse_semver(ver: &str) -> Option<(u32, u32, u32)> {
    let clean = ver.trim().trim_start_matches(['v', 'V']);
    let mut parts = clean.split('.');
    let major: u32 = parts.next()?.parse().ok()?;
    let minor: u32 = parts.next()?.parse().ok()?;
    let patch_raw = parts.next().unwrap_or("0");
    let patch_num = patch_raw
        .split(|c: char| !c.is_ascii_digit())
        .next()
        .unwrap_or("0");
    let patch: u32 = patch_num.parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer_version(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

pub fn target_asset_name() -> &'static str {
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "phaenna-macos-aarch64"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "phaenna-macos-x86_64"
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "phaenna-linux-x86_64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "phaenna-windows-x86_64.exe"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "windows", target_arch = "x86_64"),
    )))]
    {
        "phaenna"
    }
}

fn resolved_current_exe() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(std::fs::canonicalize(&exe).unwrap_or(exe))
}

pub fn is_homebrew_binary_path(path: &Path) -> bool {
    let norm = path.to_string_lossy().replace('\\', "/");
    norm.contains("/Cellar/") || norm.contains("/Caskroom/")
}

pub fn is_homebrew_install() -> bool {
    resolved_current_exe()
        .map(|p| is_homebrew_binary_path(&p))
        .unwrap_or(false)
}

/// Git SHA baked in at compile time (`build.rs`), or `"unknown"`.
pub fn current_build_sha() -> &'static str {
    option_env!("PHAENNA_GIT_SHA").unwrap_or("unknown")
}

/// Commit Unix timestamp baked in at compile time (`build.rs`), preventing downgrades of newer local dev builds.
pub fn current_commit_timestamp() -> i64 {
    option_env!("PHAENNA_COMMIT_TS")
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

pub fn short_sha(sha: &str) -> &str {
    sha.get(..7).unwrap_or(sha)
}

pub fn extract_release_sha(body: Option<&str>) -> Option<String> {
    extract_marker(body, "phaenna-sha:")
        .filter(|s| s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit()))
}

pub fn extract_release_timestamp(body: Option<&str>) -> Option<i64> {
    extract_marker(body, "phaenna-ts:")?.parse::<i64>().ok()
}

fn extract_marker(body: Option<&str>, prefix: &str) -> Option<String> {
    let text = body?;
    let start = text.find(prefix)? + prefix.len();
    let rest = &text[start..];
    let end = rest.find("-->").unwrap_or(rest.len());
    let val = rest[..end].trim();
    if val.is_empty() {
        None
    } else {
        Some(val.to_string())
    }
}

fn clean_release_notes(body: Option<String>) -> Option<String> {
    let text = body?;
    let cleaned: Vec<&str> = text
        .lines()
        .filter(|line| {
            let trimmed = line.trim();
            !(trimmed.starts_with("<!-- phaenna-") && trimmed.ends_with("-->"))
        })
        .collect();
    let joined = cleaned.join("\n").trim().to_string();
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

/// Pure update decision logic supporting both tagged SemVer releases and rolling `main` commit releases.
pub fn evaluate_release_update(
    release: GitHubRelease,
    current_version: &str,
    local_sha: &str,
    local_ts: i64,
) -> Option<UpdateInfo> {
    let semver_newer = is_newer_version(current_version, &release.tag_name);
    let is_rolling = release.tag_name.ends_with("-rolling") || release.tag_name == "latest";
    let remote_sha = extract_release_sha(release.body.as_deref());
    let remote_ts = extract_release_timestamp(release.body.as_deref()).unwrap_or(0);

    let semver_not_older = !is_newer_version(&release.tag_name, current_version);
    let rolling_sha_newer = is_rolling
        && semver_not_older
        && local_sha != "unknown"
        && matches!(
            remote_sha.as_deref(),
            Some(rsha) if rsha != local_sha && (remote_ts == 0 || local_ts == 0 || remote_ts >= local_ts)
        );

    if !semver_newer && !rolling_sha_newer {
        return None;
    }

    let (display_curr, display_latest) = if !semver_newer && rolling_sha_newer {
        (
            format!(
                "v{} ({})",
                current_version.trim_start_matches(['v', 'V']),
                short_sha(local_sha)
            ),
            format!(
                "main@{}",
                short_sha(remote_sha.as_deref().unwrap_or("latest"))
            ),
        )
    } else {
        (current_version.to_string(), release.tag_name.clone())
    };

    let target = target_asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == target)
        .or_else(|| release.assets.iter().find(|a| a.name.contains("phaenna")))
        .or_else(|| release.assets.first())?;

    Some(UpdateInfo {
        current_version: display_curr,
        latest_version: display_latest,
        download_url: asset.browser_download_url.clone(),
        asset_name: asset.name.clone(),
        release_notes: clean_release_notes(release.body),
        is_homebrew: is_homebrew_install(),
    })
}

pub async fn check_for_updates(client: &reqwest::Client) -> Result<Option<UpdateInfo>> {
    let resp = client
        .get(GITHUB_RELEASES_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(6))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(e.to_string()))?;

    Ok(evaluate_release_update(
        release,
        env!("CARGO_PKG_VERSION"),
        current_build_sha(),
        current_commit_timestamp(),
    ))
}

async fn download_and_install_inner(
    client: &reqwest::Client,
    info: &UpdateInfo,
    tx: &Sender<AppMessage>,
) -> Result<()> {
    if info.is_homebrew {
        return Err(AppError::Player(
            "Homebrew install detected: run `brew upgrade phaenna`.".to_string(),
        ));
    }

    let exe_path = resolved_current_exe()
        .ok_or_else(|| AppError::Player("Unable to resolve executable path".to_string()))?;
    let parent_dir = exe_path
        .parent()
        .ok_or_else(|| AppError::Player("Unable to resolve binary parent directory".to_string()))?;

    let tmp_path = parent_dir.join(format!(".phaenna-update-{}.tmp", std::process::id()));

    let mut resp = client
        .get(&info.download_url)
        .header("User-Agent", USER_AGENT)
        .send()
        .await?
        .error_for_status()
        .map_err(|e| AppError::Http(format!("Download failed: {e}")))?;

    let total_size = resp.content_length().unwrap_or(0);
    let mut downloaded = 0u64;
    let mut last_emit = Instant::now();
    let start_time = Instant::now();

    let mut file = tokio::fs::File::create(&tmp_path).await?;
    while let Some(chunk) = resp.chunk().await? {
        tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
        downloaded += chunk.len() as u64;

        if last_emit.elapsed() >= Duration::from_millis(120) {
            let pct = if total_size > 0 {
                (downloaded as f32 / total_size as f32) * 100.0
            } else {
                15.0
            };
            let elapsed_secs = start_time.elapsed().as_secs_f64().max(0.001);
            let speed_mb = (downloaded as f64 / 1_048_576.0) / elapsed_secs;
            let _ = tx
                .send(AppMessage::UpdateProgress(
                    pct.clamp(1.0, 95.0),
                    format!("Downloading {} ({:.1} MB/s)...", info.asset_name, speed_mb),
                ))
                .await;
            last_emit = Instant::now();
        }
    }

    tokio::io::AsyncWriteExt::flush(&mut file).await?;
    drop(file);

    let _ = tx
        .send(AppMessage::UpdateProgress(
            96.0,
            "Replacing binary atomically...".to_string(),
        ))
        .await;

    #[cfg(windows)]
    {
        let old_exe = exe_path.with_extension("exe.old");
        let _ = std::fs::remove_file(&old_exe);
        if let Err(e) = std::fs::rename(&exe_path, &old_exe) {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(AppError::Player(format!("Rename running exe failed: {e}")));
        }
        if let Err(e) = std::fs::rename(&tmp_path, &exe_path) {
            let _ = std::fs::rename(&old_exe, &exe_path);
            let _ = std::fs::remove_file(&tmp_path);
            return Err(AppError::Player(format!("Install new exe failed: {e}")));
        }
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        if let Err(e) = std::fs::rename(&tmp_path, &exe_path) {
            let _ = std::fs::remove_file(&exe_path);
            if let Err(e2) = std::fs::copy(&tmp_path, &exe_path) {
                let _ = std::fs::remove_file(&tmp_path);
                return Err(AppError::Player(format!(
                    "Replace binary failed: {e} (copy fallback: {e2})"
                )));
            }
            let _ = std::fs::remove_file(&tmp_path);
        }
        #[cfg(target_os = "macos")]
        {
            let _ = std::process::Command::new("codesign")
                .args(["-s", "-", "-f"])
                .arg(&exe_path)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }

    Ok(())
}

pub fn spawn_self_update(client: reqwest::Client, info: UpdateInfo, tx: Sender<AppMessage>) {
    tokio::spawn(async move {
        let target_version = info.latest_version.clone();
        match download_and_install_inner(&client, &info, &tx).await {
            Ok(()) => {
                let _ = tx.send(AppMessage::UpdateSuccess(target_version)).await;
            }
            Err(e) => {
                let _ = tx.send(AppMessage::UpdateFailed(e.to_string())).await;
            }
        }
    });
}

/// Clean up leftover temporary update files or `.exe.old` backups from previous updates.
pub fn cleanup_legacy_update_files() {
    if let Some(current_exe) = resolved_current_exe() {
        if let Some(parent) = current_exe.parent() {
            #[cfg(windows)]
            {
                let old_exe = current_exe.with_extension("exe.old");
                let _ = std::fs::remove_file(old_exe);
            }
            if let Ok(entries) = std::fs::read_dir(parent) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with(".phaenna-update-") && name_str.ends_with(".tmp") {
                        let _ = std::fs::remove_file(entry.path());
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_semver_and_homebrew_detection() {
        assert_eq!(parse_semver("v1.2.3-rc1"), Some((1, 2, 3)));
        assert!(is_newer_version("1.0.0", "v1.0.1"));
        assert!(!is_newer_version("1.2.0", "1.1.9"));
        assert!(is_homebrew_binary_path(Path::new(
            "/opt/homebrew/Cellar/phaenna/1.0.0/bin/phaenna"
        )));
        assert!(!is_homebrew_binary_path(Path::new(
            "/Users/me/.cargo/bin/phaenna"
        )));
    }

    #[test]
    fn test_evaluate_release_update_rolling_sha_and_semver() {
        let body = "<!-- phaenna-sha:abcdef1234567890abcdef1234567890abcdef12 -->\n<!-- phaenna-ts:2000 -->\nAutomated build for commit `abcdef1`: Fix reply threads";
        let mk = |tag: &str| GitHubRelease {
            tag_name: tag.to_string(),
            name: Some(tag.to_string()),
            body: Some(body.to_string()),
            html_url: "https://github.com/abdullah-binmadhi/Phaenna/releases".to_string(),
            assets: vec![ReleaseAsset {
                name: target_asset_name().to_string(),
                browser_download_url: "https://example.com/phaenna".to_string(),
                size: 1024,
            }],
        };

        // 1. Older SemVer upgrades to newer rolling tag.
        let info1 = evaluate_release_update(mk("v1.0.1-rolling"), "1.0.0", "unknown", 0).unwrap();
        assert_eq!(info1.latest_version, "v1.0.1-rolling");
        assert!(!info1.release_notes.as_deref().unwrap().contains("phaenna-sha"));

        // 2. Same SemVer 1.0.0 with older commit SHA upgrades to newer commit SHA.
        let info2 = evaluate_release_update(
            mk("v1.0.0-rolling"),
            "1.0.0",
            "1111111222222233333334444444555555566666",
            1500,
        )
        .unwrap();
        assert_eq!(info2.latest_version, "main@abcdef1");
        assert_eq!(info2.current_version, "v1.0.0 (1111111)");

        // 3. Matching SHA reports no update.
        assert!(evaluate_release_update(
            mk("v1.0.0-rolling"),
            "1.0.0",
            "abcdef1234567890abcdef1234567890abcdef12",
            2000,
        )
        .is_none());

        // 4. Newer local dev build (local_ts > remote_ts) is never downgraded.
        assert!(evaluate_release_update(
            mk("v1.0.0-rolling"),
            "1.0.0",
            "9999999888888877777776666666555555544444",
            2500,
        )
        .is_none());
    }
}
