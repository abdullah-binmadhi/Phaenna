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
const MIN_BINARY_BYTES: u64 = 1_000_000;

fn has_valid_native_magic(hdr: &[u8]) -> bool {
    if hdr.len() < 4 {
        return false;
    }
    #[cfg(target_os = "macos")]
    {
        matches!(
            [hdr[0], hdr[1], hdr[2], hdr[3]],
            [0xFE, 0xED, 0xFA, 0xCE]
                | [0xCE, 0xFA, 0xED, 0xFE]
                | [0xFE, 0xED, 0xFA, 0xCF]
                | [0xCF, 0xFA, 0xED, 0xFE]
                | [0xCA, 0xFE, 0xBA, 0xBE]
        )
    }
    #[cfg(target_os = "linux")]
    {
        hdr.starts_with(&[0x7F, b'E', b'L', b'F'])
    }
    #[cfg(target_os = "windows")]
    {
        hdr.starts_with(b"MZ")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        true
    }
}

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
    pub asset_name: String,
    pub release_notes: Option<String>,
    #[serde(default)]
    pub simple_notes: Vec<String>,
    pub is_homebrew: bool,
}

/// Persisted state tracking the last seen build SHA/version and any pending Post-Update "What's New" popup.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LastSeenBuild {
    pub last_seen_sha: String,
    pub last_seen_version: String,
    #[serde(default)]
    pub pending_whats_new: Vec<String>,
    #[serde(default)]
    pub updated_to_label: Option<String>,
}

const BUILTIN_CURRENT_RELEASE_NOTES: &[&str] = &[
    "[NEW] Two-Click Preview & Music Inspector: Click any song, album, or artist once to preview its cover art, details, and tracklist without interrupting your music, then double-click (or press Enter) to play or open. How to use: Click an item in the Music tab (7) or inside a popup to inspect it on the right drawer; press 'd' anytime to show or hide the drawer.",
    "[NEW] Watch Music Videos in Full Video Mode: Music Videos on Artist profiles and Search results now open in a dedicated mpv video window with synced audio. How to use: Switch to the Videos tab on an Artist page (or select any video) and double-click or press [Watch] in the inspector.",
    "[NEW] 18 Calibrated Equalizer Profiles & Clean Preset Menu: Choose curated profiles like Harman Target, Warm Tube, Classical, Jazz, Rock, Hip-Hop, Lo-Fi, and Equal Loudness from a tidy dropdown menu without clutter. How to use: Press 'e' in the Music tab (or click [EQ:] at the bottom), press 'p' to switch presets, and 'w' to save your custom curve.",
    "[IMPROVED] Full-Width Header, Clearer Layout & Front-Focused Videos: Placed your YouTube login handle above the top tabs so names never get cut off, enlarged Artist & Preview thumbnails, balanced the bottom player bar, and ensured videos launch in front of fullscreen terminals. How to use: Click your account badge at the very top (or press 'L') to switch accounts.",
    "[NEW] Plain-English Update & What's New Popup: Updates now summarize everything new, improved, and fixed in plain language with quick navigation tips, both after updating and anytime from Settings. How to use: Open Settings ('s' or ',') and choose 'What's New & Release Notes' whenever you want a refresher.",
    "[FIXED] Instant Playback & Reply Threads: Fixed songs pausing at 0:00 until pressing Space, removed duplicate Top Songs lists on Artist pages, and enabled double-clicking any main comment or reply to respond directly. How to use: Double-click any comment in a video detail view to reply immediately.",
];

/// Return plain-English release highlights for the currently running Phaenna build.
pub fn default_current_release_notes() -> Vec<String> {
    BUILTIN_CURRENT_RELEASE_NOTES
        .iter()
        .map(|s| (*s).to_string())
        .collect()
}

/// Return a readable display label for the currently running binary version.
pub fn current_display_version_label() -> String {
    let ver = env!("CARGO_PKG_VERSION");
    let sha = current_build_sha();
    if sha != "unknown" && !sha.is_empty() {
        format!("v{} ({})", ver, short_sha(sha))
    } else {
        format!("v{}", ver)
    }
}

/// Translate raw GitHub Release notes or commit summaries into up to 6 plain-English,
/// jargon-free bullet points prefixed with `[NEW]`, `[FIXED]`, or `[IMPROVED]`.
pub fn simplify_release_notes(raw_body: &str) -> Vec<String> {
    let mut bullets: Vec<String> = Vec::new();

    for (start_tag, end_tag) in [
        ("<!-- phaenna-notes-start -->", "<!-- phaenna-notes-end -->"),
        ("<!-- solus-notes-start -->", "<!-- solus-notes-end -->"),
    ] {
        if let (Some(start_idx), Some(end_idx)) = (raw_body.find(start_tag), raw_body.find(end_tag)) {
            let inner_start = start_idx + start_tag.len();
            if inner_start < end_idx {
                for line in raw_body[inner_start..end_idx].lines() {
                    for translated in translate_note_lines(line) {
                        push_unique_bullet(&mut bullets, translated);
                    }
                }
            }
        }
        if !bullets.is_empty() {
            break;
        }
    }

    if bullets.is_empty() {
        for line in raw_body.lines() {
            for translated in translate_note_lines(line) {
                push_unique_bullet(&mut bullets, translated);
            }
            if bullets.len() >= 6 {
                break;
            }
        }
    }

    bullets.truncate(6);
    if bullets.is_empty() {
        bullets.push(
            "[IMPROVED] Smoother video playback, cleaner YouTube Music browsing, and stability fixes."
                .to_string(),
        );
    }
    bullets
}

fn push_unique_bullet(bullets: &mut Vec<String>, candidate: String) {
    if bullets.len() >= 6 {
        return;
    }
    let norm = candidate.to_ascii_lowercase();
    if !bullets.iter().any(|b| b.to_ascii_lowercase() == norm) {
        bullets.push(candidate);
    }
}

fn translate_note_lines(raw_line: &str) -> Vec<String> {
    let trimmed = raw_line.trim();
    if trimmed.is_empty()
        || trimmed.starts_with("<!--")
        || trimmed.starts_with('#')
        || trimmed.starts_with("Full Changelog")
        || trimmed.starts_with("**Full Changelog**")
    {
        return Vec::new();
    }

    let mut line = trimmed.trim_start_matches(['-', '*', '+', '•']).trim();
    if let Some((num, rest)) = line.split_once('.') {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            line = rest.trim();
        }
    }
    if line.is_empty() {
        return Vec::new();
    }

    if let Some(rest) = line.strip_prefix("Automated build for commit") {
        if let Some((_, msg)) = rest.split_once(':') {
            line = msg.trim();
        }
    }
    if line.is_empty() {
        return Vec::new();
    }

    for prefix in ["Simple:", "simple:", "Note:", "note:"] {
        if let Some(custom) = line.strip_prefix(prefix) {
            let clean = custom.trim();
            if !clean.is_empty() {
                if clean.starts_with("[NEW]")
                    || clean.starts_with("[FIXED]")
                    || clean.starts_with("[IMPROVED]")
                {
                    return vec![ensure_period(clean)];
                }
                return vec![format!("[IMPROVED] {}", ensure_period(clean))];
            }
        }
    }

    for badge in ["[NEW]", "[FIXED]", "[IMPROVED]"] {
        if let Some(rest) = line.strip_prefix(badge) {
            let cleaned = strip_github_trailers(rest.trim());
            if !cleaned.is_empty() {
                return vec![format!("{} {}", badge, ensure_period(&cleaned))];
            }
        }
    }

    let lower = line.to_ascii_lowercase();
    if lower.starts_with("merge pull request")
        || lower.starts_with("merge branch")
        || lower.starts_with("chore")
        || lower.starts_with("ci:")
        || lower.starts_with("ci(")
        || lower.starts_with("test:")
        || lower.starts_with("test(")
        || lower.starts_with("tests:")
        || lower.starts_with("docs:")
        || lower.starts_with("docs(")
        || lower.starts_with("style:")
        || lower.starts_with("build:")
        || lower.starts_with("bump ")
    {
        return Vec::new();
    }

    let badge = if lower.starts_with("feat")
        || lower.starts_with("add ")
        || lower.starts_with("added ")
        || lower.starts_with("introduce")
    {
        "[NEW]"
    } else if lower.starts_with("fix")
        || lower.starts_with("resolve")
        || lower.starts_with("prevent")
        || lower.starts_with("repair")
        || lower.contains("bug")
        || lower.contains("crash")
    {
        "[FIXED]"
    } else {
        "[IMPROVED]"
    };

    let stripped = strip_conventional_prefix(line);
    let stripped = strip_github_trailers(&stripped);
    if stripped.is_empty() {
        return Vec::new();
    }

    let s_lower = stripped.to_ascii_lowercase();
    let mut matched = Vec::new();

    if s_lower.contains("preview") || s_lower.contains("inspector") || s_lower.contains("two-click") {
        matched.push(format!(
            "{} Clean Two-Click Preview & Inspector: Single-clicking songs or albums previews their cover and details without pausing playback; double-click (or press Enter) to play or open. How to use: Click any item in Music (7) to preview it on the right drawer, and press 'd' to toggle the drawer.",
            badge
        ));
    }
    if s_lower.contains("eq") || s_lower.contains("equalizer") || s_lower.contains("harman") {
        matched.push(format!(
            "{} 10-Band Audiophile Equalizer & Preset Menu: Switch between 18 tuned presets (like Harman Target, Warm Tube, Jazz, and Bass+) from a clean dropdown menu. How to use: Press 'e' in the Music tab (or click [EQ:] below) and press 'p' to browse presets.",
            badge
        ));
    }
    if s_lower.contains("mpv video") || s_lower.contains("music video") {
        matched.push(format!(
            "{} Music Videos Play With Full Video: Double-clicking a Music Video now opens the video window cleanly alongside lossless audio tracks. How to use: Open an Artist's Videos tab or search results and double-click any video.",
            badge
        ));
    }
    if s_lower.contains("fullscreen") || s_lower.contains("super+v") || s_lower.contains("front") {
        matched.push(format!(
            "{} Video Windows Always Open in Front: Playing a video while your terminal is maximized or fullscreen now brings the video right to the front and restores your terminal fullscreen when done. How to use: Launch any video normally—no extra shortcuts needed.",
            badge
        ));
    }
    if s_lower.contains("comment") || s_lower.contains("reply") || s_lower.contains("subcomment") {
        matched.push(format!(
            "{} Easier Comment Replies: Double-click any main comment or nested reply to respond right away, and single-click comments to expand or collapse replies. How to use: Open comments on a video or press 'C' in playback and double-click any comment.",
            badge
        ));
    }
    if (s_lower.contains("update") || s_lower.contains("what's new"))
        && (s_lower.contains("plain") || s_lower.contains("popup") || s_lower.contains("solus"))
    {
        matched.push(format!(
            "{} Plain-English Update Summaries: Updates now show clear, jargon-free notes explaining what changed and how to use new features. How to use: Open Settings ('s') and pick 'What's New & Release Notes' anytime.",
            badge
        ));
    }
    if !matched.is_empty() {
        return matched;
    }

    let humanized = humanize_free_text(&stripped);
    if humanized.len() < 4 {
        return Vec::new();
    }
    vec![format!("{} {}", badge, ensure_period(&humanized))]
}

fn strip_conventional_prefix(line: &str) -> String {
    if let Some((head, tail)) = line.split_once(':') {
        let h = head.trim().to_ascii_lowercase();
        let known = [
            "feat", "fix", "perf", "refactor", "ui", "audio", "playback", "music", "update",
            "updater", "improve", "remote", "network", "settings",
        ];
        if known
            .iter()
            .any(|k| h == *k || h.starts_with(&format!("{k}(")) || h.starts_with(&format!("{k}!")))
        {
            return tail.trim().to_string();
        }
    }
    line.trim().to_string()
}

fn strip_github_trailers(line: &str) -> String {
    let mut s = line.trim().to_string();
    if let Some(idx) = s.find(" by @") {
        s.truncate(idx);
    }
    if let Some(idx) = s.find(" in https://") {
        s.truncate(idx);
    }
    if let Some(open) = s.rfind("(#") {
        if s.ends_with(')') && s[open + 2..s.len() - 1].chars().all(|c| c.is_ascii_digit()) {
            s.truncate(open);
        }
    }
    s.trim().to_string()
}

fn humanize_free_text(input: &str) -> String {
    let mut words = Vec::new();
    for raw_word in input.split_whitespace() {
        let clean = raw_word.trim_matches(['`', '*']);
        if clean.is_empty()
            || clean.contains("src/")
            || clean.contains(".rs")
            || clean.contains("::")
            || clean.starts_with("0x")
        {
            continue;
        }
        words.push(clean);
    }
    let joined = words.join(" ");
    let mut chars = joined.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

fn ensure_period(s: &str) -> String {
    let trimmed = s.trim();
    if trimmed.ends_with('.') || trimmed.ends_with('!') || trimmed.ends_with('?') {
        trimmed.to_string()
    } else {
        format!("{trimmed}.")
    }
}

/// Path to `last_seen_build.json` used to trigger the one-time Post-Update "What's New" welcome popup.
pub fn last_seen_build_path() -> Option<PathBuf> {
    crate::config::Config::config_dir()
        .ok()
        .map(|d| d.join("last_seen_build.json"))
}

/// Save plain-English release notes before restart so the next launch shows "What's New & Fixed".
pub fn record_pending_post_update_welcome(info: &UpdateInfo) {
    if let Some(path) = last_seen_build_path() {
        let _ = record_pending_post_update_welcome_at(&path, info);
    }
}

pub fn record_pending_post_update_welcome_at(
    path: &Path,
    info: &UpdateInfo,
) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let notes = if info.simple_notes.is_empty() {
        simplify_release_notes(info.release_notes.as_deref().unwrap_or(""))
    } else {
        info.simple_notes.clone()
    };
    let state = LastSeenBuild {
        last_seen_sha: current_build_sha().to_string(),
        last_seen_version: info.latest_version.clone(),
        pending_whats_new: notes,
        updated_to_label: Some(info.latest_version.clone()),
    };
    let json = serde_json::to_string_pretty(&state)?;
    std::fs::write(path, json)
}

/// Consume pending Post-Update Welcome notes on startup, or detect an external upgrade (`cargo install` / Homebrew).
pub fn check_or_take_post_update_welcome() -> Option<(String, Vec<String>)> {
    let path = last_seen_build_path()?;
    check_or_take_post_update_welcome_at(
        &path,
        env!("CARGO_PKG_VERSION"),
        current_build_sha(),
    )
}

pub fn check_or_take_post_update_welcome_at(
    path: &Path,
    running_version: &str,
    running_sha: &str,
) -> Option<(String, Vec<String>)> {
    let running_label = if running_sha != "unknown" && !running_sha.is_empty() {
        format!(
            "v{} ({})",
            running_version.trim_start_matches(['v', 'V']),
            short_sha(running_sha)
        )
    } else {
        format!("v{}", running_version.trim_start_matches(['v', 'V']))
    };

    let Ok(raw) = std::fs::read_to_string(path) else {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let initial = LastSeenBuild {
            last_seen_sha: running_sha.to_string(),
            last_seen_version: running_version.to_string(),
            pending_whats_new: Vec::new(),
            updated_to_label: None,
        };
        if let Ok(json) = serde_json::to_string_pretty(&initial) {
            let _ = std::fs::write(path, json);
        }
        return None;
    };

    let Ok(mut state) = serde_json::from_str::<LastSeenBuild>(&raw) else {
        return None;
    };

    if !state.pending_whats_new.is_empty() {
        let notes = std::mem::take(&mut state.pending_whats_new);
        let label = state
            .updated_to_label
            .take()
            .unwrap_or_else(|| running_label.clone());
        state.last_seen_sha = running_sha.to_string();
        state.last_seen_version = running_version.to_string();
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(path, json);
        }
        return Some((label, notes));
    }

    let version_changed = !state.last_seen_version.is_empty()
        && state.last_seen_version.trim_start_matches(['v', 'V'])
            != running_version.trim_start_matches(['v', 'V']);
    let sha_changed = !state.last_seen_sha.is_empty()
        && state.last_seen_sha != "unknown"
        && running_sha != "unknown"
        && !state.last_seen_sha.eq_ignore_ascii_case(running_sha);

    if version_changed || sha_changed {
        state.last_seen_sha = running_sha.to_string();
        state.last_seen_version = running_version.to_string();
        state.updated_to_label = None;
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(path, json);
        }
        return Some((running_label, default_current_release_notes()));
    }

    None
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
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "phaenna-linux-aarch64"
    }
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        "phaenna-windows-x86_64.exe"
    }
    #[cfg(not(any(
        all(target_os = "macos", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
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
        .find(|a| a.name == target && a.size >= MIN_BINARY_BYTES)?;

    let cleaned_notes = clean_release_notes(release.body);
    let simple_notes = cleaned_notes
        .as_deref()
        .map(simplify_release_notes)
        .unwrap_or_else(default_current_release_notes);

    Some(UpdateInfo {
        current_version: display_curr,
        latest_version: display_latest,
        download_url: asset.browser_download_url.clone(),
        asset_name: asset.name.clone(),
        release_notes: cleaned_notes,
        simple_notes,
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

pub fn build_download_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(Duration::from_secs(30))
        .tcp_keepalive(Duration::from_secs(15))
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(AppError::Network)
}

fn range_header_value(downloaded: u64) -> Option<String> {
    (downloaded > 0).then(|| format!("bytes={downloaded}-"))
}

fn compute_progress_pct(downloaded: u64, expected_total: u64) -> f32 {
    if expected_total > 0 {
        ((downloaded as f32 / expected_total as f32) * 100.0).clamp(1.0, 95.0)
    } else {
        15.0
    }
}

async fn open_download_file(tmp_path: &Path, resume_offset: u64) -> Result<tokio::fs::File> {
    if resume_offset > 0 {
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(tmp_path)
            .await
            .map_err(AppError::Io)
    } else {
        tokio::fs::File::create(tmp_path).await.map_err(AppError::Io)
    }
}

async fn download_binary_with_resume(
    dl_client: &reqwest::Client,
    meta_client: &reqwest::Client,
    info: &UpdateInfo,
    tmp_path: &Path,
    tx: &Sender<AppMessage>,
) -> Result<u64> {
    const MAX_ATTEMPTS: usize = 4;
    let mut current_url = info.download_url.clone();
    let mut downloaded = 0u64;
    let mut expected_total = 0u64;
    let mut last_emit = Instant::now();
    let start_time = Instant::now();
    let mut last_err: Option<AppError> = None;

    for attempt in 1..=MAX_ATTEMPTS {
        let mut req = dl_client
            .get(&current_url)
            .header("User-Agent", USER_AGENT)
            .header("Accept", "application/octet-stream");
        if let Some(range_hdr) = range_header_value(downloaded) {
            req = req.header(reqwest::header::RANGE, range_hdr);
        }

        let resp = match req.send().await {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(AppError::Network(e));
                if attempt < MAX_ATTEMPTS {
                    let pct = compute_progress_pct(downloaded, expected_total);
                    let _ = tx
                        .send(AppMessage::UpdateProgress(
                            pct,
                            format!(
                                "Network hiccup at {:.0}% — resuming (attempt {}/{})...",
                                pct,
                                attempt + 1,
                                MAX_ATTEMPTS
                            ),
                        ))
                        .await;
                    tokio::time::sleep(Duration::from_millis(600 * attempt as u64)).await;
                    continue;
                }
                break;
            }
        };

        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND
            || status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE
            || status.is_server_error()
        {
            if let Ok(Some(fresh)) = check_for_updates(meta_client).await {
                current_url = fresh.download_url;
            }
            if status == reqwest::StatusCode::RANGE_NOT_SATISFIABLE
                || status == reqwest::StatusCode::NOT_FOUND
            {
                downloaded = 0;
                expected_total = 0;
            }
            last_err = Some(AppError::Http(format!("Download HTTP {status}")));
            if attempt < MAX_ATTEMPTS {
                tokio::time::sleep(Duration::from_millis(600 * attempt as u64)).await;
                continue;
            }
            break;
        }

        let mut resp = match resp.error_for_status() {
            Ok(r) => r,
            Err(e) => {
                last_err = Some(AppError::Http(format!("Download failed: {e}")));
                if attempt < MAX_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(600 * attempt as u64)).await;
                    continue;
                }
                break;
            }
        };

        if status == reqwest::StatusCode::PARTIAL_CONTENT {
            if expected_total == 0 {
                if let Some(rem) = resp.content_length() {
                    expected_total = downloaded + rem;
                }
            }
        } else {
            // Server returned full 200 OK instead of 206 Partial Content; reset partial file cleanly.
            downloaded = 0;
            expected_total = resp.content_length().unwrap_or(0);
        }

        let mut file = open_download_file(tmp_path, downloaded).await?;
        let mut stream_err: Option<AppError> = None;

        loop {
            match tokio::time::timeout(Duration::from_secs(30), resp.chunk()).await {
                Ok(Ok(Some(chunk))) => {
                    tokio::io::AsyncWriteExt::write_all(&mut file, &chunk).await?;
                    downloaded += chunk.len() as u64;

                    if last_emit.elapsed() >= Duration::from_millis(120) {
                        let pct = compute_progress_pct(downloaded, expected_total);
                        let elapsed_secs = start_time.elapsed().as_secs_f64().max(0.001);
                        let speed_mb = (downloaded as f64 / 1_048_576.0) / elapsed_secs;
                        let _ = tx
                            .send(AppMessage::UpdateProgress(
                                pct,
                                format!("Downloading {} ({:.1} MB/s)...", info.asset_name, speed_mb),
                            ))
                            .await;
                        last_emit = Instant::now();
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => {
                    stream_err = Some(AppError::Network(e));
                    break;
                }
                Err(_) => {
                    stream_err = Some(AppError::Http("Download stream stalled (30s timeout)".into()));
                    break;
                }
            }
        }

        tokio::io::AsyncWriteExt::flush(&mut file).await?;
        drop(file);

        if let Some(err) = stream_err {
            last_err = Some(err);
            if attempt < MAX_ATTEMPTS {
                let pct = compute_progress_pct(downloaded, expected_total);
                let _ = tx
                    .send(AppMessage::UpdateProgress(
                        pct,
                        format!(
                            "Network hiccup at {:.0}% — resuming (attempt {}/{})...",
                            pct,
                            attempt + 1,
                            MAX_ATTEMPTS
                        ),
                    ))
                    .await;
                tokio::time::sleep(Duration::from_millis(600 * attempt as u64)).await;
                continue;
            }
            break;
        }

        if expected_total > 0 && downloaded < expected_total {
            last_err = Some(AppError::Http(format!(
                "Incomplete download ({downloaded}/{expected_total} bytes)"
            )));
            if attempt < MAX_ATTEMPTS {
                let pct = compute_progress_pct(downloaded, expected_total);
                let _ = tx
                    .send(AppMessage::UpdateProgress(
                        pct,
                        format!(
                            "Incomplete stream at {:.0}% — resuming (attempt {}/{})...",
                            pct,
                            attempt + 1,
                            MAX_ATTEMPTS
                        ),
                    ))
                    .await;
                tokio::time::sleep(Duration::from_millis(600 * attempt as u64)).await;
                continue;
            }
            break;
        }

        return Ok(downloaded);
    }

    let _ = std::fs::remove_file(tmp_path);
    Err(last_err.unwrap_or_else(|| AppError::Http("Update download failed".into())))
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
    let dl_client = build_download_client().unwrap_or_else(|_| client.clone());
    let downloaded =
        download_binary_with_resume(&dl_client, client, info, &tmp_path, tx).await?;

    if downloaded < MIN_BINARY_BYTES {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(AppError::Http(format!(
            "Downloaded asset `{}` is too small ({} bytes); aborting binary replace",
            info.asset_name, downloaded
        )));
    }
    {
        use std::io::Read;
        let mut hdr = [0u8; 4];
        let valid_magic = std::fs::File::open(&tmp_path)
            .and_then(|mut f| f.read_exact(&mut hdr))
            .map(|()| has_valid_native_magic(&hdr))
            .unwrap_or(false);
        if !valid_magic {
            let _ = std::fs::remove_file(&tmp_path);
            return Err(AppError::Http(format!(
                "Downloaded asset `{}` is not a valid native executable; aborting update",
                info.asset_name
            )));
        }
    }

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
    record_pending_post_update_welcome(&info);
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
                size: 16_000_000,
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

        // 5. Never fall back to checksums or wrong-arch assets if native binary is missing/too small.
        let mut only_checksums = mk("v1.0.1-rolling");
        only_checksums.assets = vec![ReleaseAsset {
            name: "SHA256SUMS.txt".to_string(),
            browser_download_url: "https://example.com/SHA256SUMS.txt".to_string(),
            size: 355,
        }];
        assert!(evaluate_release_update(only_checksums, "1.0.0", "unknown", 0).is_none());
        assert!(!has_valid_native_magic(b"f659"));
        assert!(build_download_client().is_ok());
        assert_eq!(range_header_value(0), None);
        assert_eq!(range_header_value(4096).as_deref(), Some("bytes=4096-"));
        assert!((compute_progress_pct(8_000_000, 16_000_000) - 50.0).abs() < 0.01);
    }

    #[test]
    fn test_simplify_release_notes_and_post_update_welcome_state() {
        let raw = "<!-- phaenna-sha:1234567abcdef1234567abcdef1234567abcdef12 -->\nAutomated build for commit `1234567`: feat(music): add 10-band eq presets and harman curve";
        let notes = simplify_release_notes(raw);
        assert!(!notes.is_empty());
        assert!(notes[0].contains("[NEW]"));
        assert!(notes[0].contains("How to use:"));
        assert!(!notes[0].contains("src/"));

        let tmp = std::env::temp_dir().join(format!("phaenna_update_test_{}.json", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        // First run creates baseline without popup
        assert!(check_or_take_post_update_welcome_at(&tmp, "1.0.0", "1111111").is_none());

        let info = UpdateInfo {
            current_version: "v1.0.0 (1111111)".to_string(),
            latest_version: "main@2222222".to_string(),
            download_url: "https://example.com/phaenna".to_string(),
            asset_name: "phaenna-linux-x86_64".to_string(),
            release_notes: Some(raw.to_string()),
            simple_notes: notes.clone(),
            is_homebrew: false,
        };
        record_pending_post_update_welcome_at(&tmp, &info).unwrap();
        let welcome = check_or_take_post_update_welcome_at(&tmp, "1.0.0", "2222222").unwrap();
        assert_eq!(welcome.0, "main@2222222");
        assert_eq!(welcome.1, notes);
        // Consumed once so subsequent launch doesn't re-show unless upgraded again
        assert!(check_or_take_post_update_welcome_at(&tmp, "1.0.0", "2222222").is_none());
        let _ = std::fs::remove_file(&tmp);
    }
}
