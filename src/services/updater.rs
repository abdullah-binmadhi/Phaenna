//! Non-blocking GitHub Releases SemVer & atomic self-updater for `phaenna`.

use crate::error::{AppError, Result};
use crate::messages::AppMessage;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
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

pub async fn check_for_updates(client: &reqwest::Client) -> Result<Option<UpdateInfo>> {
    let resp = client
        .get(GITHUB_RELEASES_URL)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Ok(None);
    }

    let release: GitHubRelease = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(e.to_string()))?;

    let current = env!("CARGO_PKG_VERSION").to_string();
    if !is_newer_version(&current, &release.tag_name) {
        return Ok(None);
    }

    let target = target_asset_name();
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == target)
        .or_else(|| release.assets.first());

    let Some(asset) = asset else {
        return Ok(None);
    };

    let is_brew = resolved_current_exe()
        .map(|p| is_homebrew_binary_path(&p))
        .unwrap_or(false);

    Ok(Some(UpdateInfo {
        current_version: current,
        latest_version: release.tag_name,
        download_url: asset.browser_download_url.clone(),
        asset_name: asset.name.clone(),
        release_notes: release.body,
        is_homebrew: is_brew,
    }))
}

pub fn spawn_self_update(client: reqwest::Client, info: UpdateInfo, tx: Sender<AppMessage>) {
    tokio::spawn(async move {
        if info.is_homebrew {
            let _ = tx
                .send(AppMessage::UpdateFailed(
                    "Homebrew install detected: run `brew upgrade phaenna`.".to_string(),
                ))
                .await;
            return;
        }

        let Some(exe_path) = resolved_current_exe() else {
            let _ = tx
                .send(AppMessage::UpdateFailed(
                    "Unable to resolve executable path".to_string(),
                ))
                .await;
            return;
        };

        let _ = tx
            .send(AppMessage::UpdateProgress(
                10.0,
                format!("Downloading {}...", info.asset_name),
            ))
            .await;

        let resp = match client.get(&info.download_url).send().await {
            Ok(r) if r.status().is_success() => r,
            Ok(r) => {
                let _ = tx
                    .send(AppMessage::UpdateFailed(format!(
                        "Download failed: HTTP {}",
                        r.status()
                    )))
                    .await;
                return;
            }
            Err(e) => {
                let _ = tx
                    .send(AppMessage::UpdateFailed(format!("Download error: {}", e)))
                    .await;
                return;
            }
        };

        let bytes = match resp.bytes().await {
            Ok(b) => b,
            Err(e) => {
                let _ = tx
                    .send(AppMessage::UpdateFailed(format!("Read error: {}", e)))
                    .await;
                return;
            }
        };

        let _ = tx
            .send(AppMessage::UpdateProgress(
                85.0,
                "Replacing binary atomically...".to_string(),
            ))
            .await;

        let tmp_path = exe_path.with_extension("update.tmp");
        if let Err(e) = tokio::fs::write(&tmp_path, &bytes).await {
            let _ = tx
                .send(AppMessage::UpdateFailed(format!("Write error: {}", e)))
                .await;
            return;
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o755));
        }

        match tokio::fs::rename(&tmp_path, &exe_path).await {
            Ok(_) => {
                let _ = tx
                    .send(AppMessage::UpdateSuccess(info.latest_version))
                    .await;
            }
            Err(e) => {
                let _ = tokio::fs::remove_file(&tmp_path).await;
                let _ = tx
                    .send(AppMessage::UpdateFailed(format!("Replace error: {}", e)))
                    .await;
            }
        }
    });
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
}
