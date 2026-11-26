//! Isolated Google Browser-Popup Sign-In, `auth.json` (`0600`) multi-account vault,
//! and InnerTube `SAPISIDHASH` signing — zero daily-browser SQLite scraping.

use crate::api::player::find_binary;
use crate::config::GoogleOAuthConfig;
use crate::db::AccountRow;
use crate::error::{AppError, Result};
use futures_util::{SinkExt, StreamExt};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;

pub const GOOGLE_YOUTUBE_SERVICE_LOGIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F";

pub const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const PHAENNA_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Returns current UNIX epoch timestamp in seconds.
pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Authentication mechanism that issued a Google session in Phaenna's vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    OAuthDevice,
    OAuthPkce,
    FirstParty,
}

impl AuthKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OAuthDevice => "oauth_device",
            Self::OAuthPkce => "oauth_pkce",
            Self::FirstParty => "first_party",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::OAuthDevice => "Google Device Code (Auto-Refresh)",
            Self::OAuthPkce => "Google OAuth2 PKCE (Auto-Refresh)",
            Self::FirstParty => "Browser Popup Session (Isolated Vault)",
        }
    }
}

/// Persistent Google + YouTube account credentials stored solely inside Phaenna's vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GoogleAccountSession {
    pub account_id: String,
    pub email: String,
    pub display_name: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub channel_handle: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    pub auth_kind: AuthKind,
    #[serde(default)]
    pub access_token: Option<String>,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default)]
    pub expires_at: Option<i64>,
    #[serde(default)]
    pub cookies: HashMap<String, String>,
    #[serde(default)]
    pub netscape_cookies: Option<String>,
    pub added_at: i64,
    pub last_refreshed: i64,
}

impl GoogleAccountSession {
    /// True when an OAuth2 access token expires within 90 seconds (or is already expired).
    pub fn is_token_expiring(&self, now_unix: i64) -> bool {
        matches!(self.expires_at, Some(exp) if now_unix >= exp - 90)
    }

    /// Remaining validity window in seconds (None for persistent browser-popup cookie sessions).
    pub fn token_ttl_secs(&self, now_unix: i64) -> Option<i64> {
        self.expires_at.map(|exp| (exp - now_unix).max(0))
    }

    /// Extract the primary SAPISID cookie value needed for InnerTube `SAPISIDHASH` headers.
    pub fn sapisid(&self) -> Option<&str> {
        self.cookies
            .get("SAPISID")
            .or_else(|| self.cookies.get("__Secure-3PAPISID"))
            .or_else(|| self.cookies.get("__Secure-1PAPISID"))
            .map(String::as_str)
    }

    /// Deterministic `Cookie:` header built from isolated session cookies.
    pub fn cookie_header(&self) -> Option<String> {
        if self.cookies.is_empty() {
            return None;
        }
        let mut pairs: Vec<(&String, &String)> = self.cookies.iter().collect();
        pairs.sort_by(|a, b| a.0.cmp(b.0));
        let joined = pairs
            .into_iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("; ");
        Some(joined)
    }

    /// Format isolated session cookies into a Netscape cookie jar for `yt-dlp` / `mpv`.
    pub fn to_netscape_cookies(&self) -> Option<String> {
        if let Some(ref raw) = self.netscape_cookies {
            if !raw.trim().is_empty() {
                return Some(raw.clone());
            }
        }
        if self.cookies.is_empty() {
            return None;
        }
        let expiry = now_unix_secs() + 86_400 * 30;
        let mut keys: Vec<&String> = self.cookies.keys().collect();
        keys.sort();

        let mut out = String::from("# Netscape HTTP Cookie File\n");
        for k in keys {
            if let Some(v) = self.cookies.get(k) {
                for domain in [".youtube.com", ".google.com"] {
                    out.push_str(&format!(
                        "{domain}\tTRUE\t/\tTRUE\t{expiry}\t{k}\t{v}\n"
                    ));
                }
            }
        }
        Some(out)
    }

    pub fn to_account_row(&self, is_active: bool) -> AccountRow {
        AccountRow {
            account_id: self.account_id.clone(),
            email: self.email.clone(),
            display_name: self.display_name.clone(),
            channel_id: self.channel_id.clone(),
            channel_handle: self.channel_handle.clone(),
            avatar_url: self.avatar_url.clone(),
            auth_kind: self.auth_kind.as_str().to_string(),
            is_active,
            added_at: self.added_at,
            last_refreshed: self.last_refreshed,
        }
    }
}

/// Atomic multi-account vault saved at `~/.config/phaenna/auth.json` with `0600` permissions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AuthVault {
    pub version: u32,
    #[serde(default)]
    pub active_account_id: Option<String>,
    #[serde(default)]
    pub accounts: Vec<GoogleAccountSession>,
}

impl Default for AuthVault {
    fn default() -> Self {
        Self {
            version: 1,
            active_account_id: None,
            accounts: Vec::new(),
        }
    }
}

impl AuthVault {
    pub fn load_from_path(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let raw = std::fs::read_to_string(path)?;
        serde_json::from_str::<Self>(&raw)
            .map_err(|e| AppError::Auth(format!("Corrupt auth vault {}: {e}", path.display())))
    }

    pub fn load_or_default(path: &Path) -> Self {
        Self::load_from_path(path).unwrap_or_default()
    }

    /// Write vault atomically via temp file + chmod `0600` + rename so credentials never hit world-readable disk.
    pub fn save_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let payload = serde_json::to_string_pretty(self)
            .map_err(|e| AppError::Auth(format!("Failed to serialize auth vault: {e}")))?;

        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, payload.as_bytes())?;
        set_owner_only_perms(&tmp);
        std::fs::rename(&tmp, path)?;
        set_owner_only_perms(path);
        Ok(())
    }

    /// Sync Phaenna's isolated `session_cookies.txt` exclusively from the active vault session.
    pub fn sync_isolated_cookie_jar(&self, jar_path: &Path) -> Result<()> {
        if let Some(netscape) = self
            .active_account()
            .and_then(GoogleAccountSession::to_netscape_cookies)
        {
            if let Some(parent) = jar_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(jar_path, netscape.as_bytes())?;
            set_owner_only_perms(jar_path);
        } else if jar_path.exists() {
            let _ = std::fs::remove_file(jar_path);
        }
        Ok(())
    }

    pub fn active_account(&self) -> Option<&GoogleAccountSession> {
        let active_id = self.active_account_id.as_deref()?;
        self.accounts.iter().find(|a| a.account_id == active_id)
    }

    pub fn is_authenticated(&self) -> bool {
        self.active_account().is_some()
    }

    pub fn upsert_session(&mut self, session: GoogleAccountSession) {
        let id = session.account_id.clone();
        if let Some(existing) = self.accounts.iter_mut().find(|a| a.account_id == id) {
            *existing = session;
        } else {
            self.accounts.push(session);
        }
        self.active_account_id = Some(id);
    }

    pub fn set_active(&mut self, account_id: &str) -> bool {
        if self.accounts.iter().any(|a| a.account_id == account_id) {
            self.active_account_id = Some(account_id.to_string());
            true
        } else {
            false
        }
    }

    pub fn remove_account(&mut self, account_id: &str) -> bool {
        let before = self.accounts.len();
        self.accounts.retain(|a| a.account_id != account_id);
        let removed = self.accounts.len() != before;
        if self.active_account_id.as_deref() == Some(account_id) {
            self.active_account_id = self.accounts.first().map(|a| a.account_id.clone());
        }
        removed
    }
}

fn set_owner_only_perms(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

/// Interactive state machine for the `[L]` Google Account Hub modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LoginStep {
    #[default]
    Hub,
    AwaitingBrowserPopup,
}

/// Interactive state for the `[L]` Google Account Hub modal.
#[derive(Debug, Default)]
pub struct LoginUiState {
    pub step: LoginStep,
    pub account_cursor: usize,
    pub status_banner: Option<String>,
    pub error_banner: Option<String>,
    pub busy: bool,
    pub popup_launcher: Option<String>,
    pub cancel_tx: Option<watch::Sender<bool>>,
}

impl LoginUiState {
    /// Cancel any active popup watcher and clear transient UI flags.
    pub fn reset_transient(&mut self) {
        if let Some(tx) = self.cancel_tx.take() {
            let _ = tx.send(true);
        }
        self.busy = false;
        self.popup_launcher = None;
    }

    pub fn back_to_hub(&mut self) {
        self.reset_transient();
        self.step = LoginStep::Hub;
    }
}

/// Resolved Google OAuth2 client credentials (used for silent refresh of legacy OAuth sessions).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub client_id: String,
    pub client_secret: String,
}

/// Resolve OAuth client credentials from `config.toml` or `PHAENNA_GOOGLE_CLIENT_*` env vars.
pub fn resolve_oauth_client(cfg: &GoogleOAuthConfig) -> Option<OAuthClient> {
    if let Some(id) = cfg
        .client_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(OAuthClient {
            client_id: id.to_string(),
            client_secret: cfg
                .client_secret
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .to_string(),
        });
    }

    if let Ok(env_id) = std::env::var("PHAENNA_GOOGLE_CLIENT_ID") {
        let trimmed = env_id.trim();
        if !trimmed.is_empty() {
            let env_secret = std::env::var("PHAENNA_GOOGLE_CLIENT_SECRET")
                .map(|s| s.trim().to_string())
                .unwrap_or_default();
            return Some(OAuthClient {
                client_id: trimmed.to_string(),
                client_secret: env_secret,
            });
        }
    }

    None
}

/// Compute `SAPISIDHASH <ts>_<sha1_hex>` header value used by authenticated InnerTube requests.
pub fn compute_sapisidhash(sapisid: &str, origin: &str, unix_ts: i64) -> String {
    let payload = format!("{unix_ts} {sapisid} {origin}");
    let digest = Sha1::digest(payload.as_bytes());
    let mut hex = String::with_capacity(40);
    for byte in digest {
        use std::fmt::Write;
        let _ = write!(hex, "{byte:02x}");
    }
    format!("{unix_ts}_{hex}")
}

fn percent_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len() * 2);
    for byte in input.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(*byte as char);
            }
            b' ' => out.push_str("%20"),
            other => {
                use std::fmt::Write;
                let _ = write!(out, "%{other:02X}");
            }
        }
    }
    out
}

fn encode_form(params: &[(&str, &str)]) -> String {
    params
        .iter()
        .map(|(k, v)| format!("{}={}", percent_encode(k), percent_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default = "default_token_ttl")]
    expires_in: i64,
}

fn default_token_ttl() -> i64 {
    3600
}

fn has_google_auth_cookies(jar: &HashMap<String, String>) -> bool {
    let has_sid = jar.contains_key("SID") || jar.contains_key("__Secure-3PSID");
    let has_sapisid = jar.contains_key("SAPISID") || jar.contains_key("__Secure-3PAPISID");
    has_sid && has_sapisid
}

fn build_first_party_session(
    email: &str,
    cookies: HashMap<String, String>,
    netscape_cookies: Option<String>,
) -> GoogleAccountSession {
    let now = now_unix_secs();
    let display = email
        .split('@')
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or("Google User")
        .to_string();
    GoogleAccountSession {
        account_id: format!("fp:{email}"),
        email: email.to_string(),
        display_name: display,
        channel_id: None,
        channel_handle: None,
        avatar_url: None,
        auth_kind: AuthKind::FirstParty,
        access_token: None,
        refresh_token: None,
        expires_at: None,
        cookies,
        netscape_cookies,
        added_at: now,
        last_refreshed: now,
    }
}

#[derive(Debug, Deserialize)]
struct TbLsOutput {
    #[serde(default)]
    browsers: Vec<TbBrowserInstance>,
}

#[derive(Debug, Deserialize)]
struct TbBrowserInstance {
    #[serde(rename = "cdpPort")]
    cdp_port: Option<u16>,
    #[serde(default)]
    tabs: Vec<TbBrowserTab>,
}

#[derive(Debug, Deserialize)]
struct TbBrowserTab {
    #[serde(default)]
    url: String,
}

async fn discover_terminal_browser_instances() -> Vec<(u16, Vec<String>)> {
    let mut found: Vec<(u16, Vec<String>)> = Vec::new();
    let mut seen_ports: HashSet<u16> = HashSet::new();

    if let Ok(output) = tokio::process::Command::new("terminal-browser")
        .args(["ls", "--all", "--json"])
        .stdin(Stdio::null())
        .output()
        .await
    {
        let raw = String::from_utf8_lossy(&output.stdout);
        // terminal-browser may emit a Kitty APC graphics capability probe before the JSON object.
        if let Some(start) = raw.find('{') {
            if let Ok(parsed) = serde_json::from_str::<TbLsOutput>(&raw[start..]) {
                for b in parsed.browsers {
                    if let Some(port) = b.cdp_port {
                        if seen_ports.insert(port) {
                            let urls = b.tabs.into_iter().map(|t| t.url).collect();
                            found.push((port, urls));
                        }
                    }
                }
            }
        }
    }

    if let Some(home) = directories::BaseDirs::new().map(|b| b.home_dir().to_path_buf()) {
        let dir = home.join(".local/state/pixel/instances");
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for entry in entries.flatten() {
                if let Ok(text) = std::fs::read_to_string(entry.path()) {
                    if let Ok(val) = serde_json::from_str::<serde_json::Value>(&text) {
                        if let Some(port) = val
                            .get("cdpPort")
                            .and_then(|v| v.as_u64())
                            .and_then(|p| u16::try_from(p).ok())
                        {
                            if seen_ports.insert(port) {
                                found.push((port, Vec::new()));
                            }
                        }
                    }
                }
            }
        }
    }

    found
}

/// Convert CDP `Storage.getCookies` entries into Phaenna's cookie map + isolated Netscape jar.
pub fn parse_cdp_cookies(
    cdp_cookies: &[serde_json::Value],
    default_expiry: i64,
) -> (HashMap<String, String>, String) {
    let mut jar: HashMap<String, String> = HashMap::new();
    let mut lines = String::from("# Netscape HTTP Cookie File\n");

    for c in cdp_cookies {
        let domain = c.get("domain").and_then(|v| v.as_str()).unwrap_or("");
        let lower_dom = domain.to_ascii_lowercase();
        if !(lower_dom.ends_with("youtube.com") || lower_dom.ends_with("google.com")) {
            continue;
        }
        let name = c.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let value = c.get("value").and_then(|v| v.as_str()).unwrap_or("");
        if name.is_empty() || value.is_empty() {
            continue;
        }
        let path = c.get("path").and_then(|v| v.as_str()).unwrap_or("/");
        let secure = c.get("secure").and_then(|v| v.as_bool()).unwrap_or(true);
        let sub_flag = if domain.starts_with('.') {
            "TRUE"
        } else {
            "FALSE"
        };
        let sec_flag = if secure { "TRUE" } else { "FALSE" };
        let exp = c
            .get("expires")
            .and_then(|v| v.as_f64())
            .map(|e| e as i64)
            .filter(|&e| e > 0)
            .unwrap_or(default_expiry);

        jar.insert(name.to_string(), value.to_string());
        lines.push_str(&format!(
            "{domain}\t{sub_flag}\t{path}\t{sec_flag}\t{exp}\t{name}\t{value}\n"
        ));
    }

    (jar, lines)
}

async fn cdp_fetch_cookies_and_optionally_close(
    client: &reqwest::Client,
    cdp_port: u16,
    close_after_capture: bool,
    is_dedicated_instance: bool,
) -> Option<(HashMap<String, String>, String, Vec<String>)> {
    let version_url = format!("http://127.0.0.1:{cdp_port}/json/version");
    let ver_val: serde_json::Value = client
        .get(&version_url)
        .timeout(Duration::from_secs(2))
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    let ws_url = ver_val.get("webSocketDebuggerUrl")?.as_str()?;

    let mut target_urls: Vec<String> = Vec::new();
    let mut auth_target_ids: Vec<String> = Vec::new();
    if let Ok(resp) = client
        .get(format!("http://127.0.0.1:{cdp_port}/json/list"))
        .timeout(Duration::from_secs(2))
        .send()
        .await
    {
        if let Ok(targets) = resp.json::<Vec<serde_json::Value>>().await {
            for t in targets {
                if t.get("type").and_then(|v| v.as_str()) == Some("page") {
                    if let Some(u) = t.get("url").and_then(|v| v.as_str()) {
                        target_urls.push(u.to_string());
                        if u.contains("youtube.com") || u.contains("accounts.google.com") {
                            if let Some(tid) = t.get("id").and_then(|v| v.as_str()) {
                                auth_target_ids.push(tid.to_string());
                            }
                        }
                    }
                }
            }
        }
    }

    let (ws, _) = tokio_tungstenite::connect_async(ws_url).await.ok()?;
    let (mut ws_tx, mut ws_rx) = ws.split();
    let req_json = r#"{"id":1,"method":"Storage.getCookies","params":{}}"#;
    ws_tx.send(Message::Text(req_json.into())).await.ok()?;

    let cookies_arr = loop {
        let next = tokio::time::timeout(Duration::from_secs(2), ws_rx.next())
            .await
            .ok()??;
        let Ok(Message::Text(txt)) = next else {
            continue;
        };
        if let Ok(val) = serde_json::from_str::<serde_json::Value>(&txt) {
            if val.get("id").and_then(|v| v.as_u64()) == Some(1) {
                let arr = val
                    .pointer("/result/cookies")
                    .and_then(|v| v.as_array())
                    .cloned()
                    .unwrap_or_default();
                break arr;
            }
        }
    };

    let (jar, netscape) = parse_cdp_cookies(&cookies_arr, now_unix_secs() + 86_400 * 30);
    if close_after_capture && has_google_auth_cookies(&jar) && jar.contains_key("LOGIN_INFO") {
        if is_dedicated_instance {
            let _ = ws_tx
                .send(Message::Text(
                    r#"{"id":2,"method":"Browser.close","params":{}}"#.into(),
                ))
                .await;
        } else {
            for tid in auth_target_ids {
                let cmd = serde_json::json!({
                    "id": 3,
                    "method": "Target.closeTarget",
                    "params": { "targetId": tid }
                })
                .to_string();
                let _ = ws_tx.send(Message::Text(cmd.into())).await;
            }
        }
    }

    Some((jar, netscape, target_urls))
}

fn find_chromium_popup_browser() -> Option<(PathBuf, &'static str)> {
    let candidates: &[(&str, &str)] = &[
        ("chromium", "Chromium"),
        ("chromium-browser", "Chromium"),
        ("google-chrome", "Google Chrome"),
        ("google-chrome-stable", "Google Chrome"),
        ("brave", "Brave"),
        ("brave-browser", "Brave"),
        ("microsoft-edge", "Microsoft Edge"),
        ("microsoft-edge-stable", "Microsoft Edge"),
        ("vivaldi", "Vivaldi"),
        ("vivaldi-stable", "Vivaldi"),
    ];
    for (bin, label) in candidates {
        if let Some(path) = find_binary(bin) {
            return Some((path, label));
        }
    }

    #[cfg(target_os = "macos")]
    {
        let mac_apps: &[(&str, &str)] = &[
            (
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
                "Google Chrome",
            ),
            (
                "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
                "Brave",
            ),
            (
                "/Applications/Chromium.app/Contents/MacOS/Chromium",
                "Chromium",
            ),
            (
                "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
                "Microsoft Edge",
            ),
        ];
        for (p, label) in mac_apps {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some((path, label));
            }
        }
    }

    #[cfg(windows)]
    {
        let win_bins: &[(&str, &str)] = &[
            (r"C:\Program Files\Google\Chrome\Application\chrome.exe", "Google Chrome"),
            (r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe", "Google Chrome"),
            (r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe", "Brave"),
            (r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe", "Microsoft Edge"),
        ];
        for (p, label) in win_bins {
            let path = PathBuf::from(p);
            if path.is_file() {
                return Some((path, label));
            }
        }
    }

    None
}

fn allocate_ephemeral_cdp_port() -> std::result::Result<u16, String> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))
        .map_err(|e| format!("Failed to allocate local CDP port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Failed reading local CDP port: {e}"))?
        .port();
    Ok(port)
}

/// RAII cleanup guard ensuring an isolated popup browser process and temporary profile dir never leak.
struct PopupBrowserGuard {
    child: Option<Child>,
    profile_dir: PathBuf,
}

impl PopupBrowserGuard {
    fn shutdown(&mut self) {
        if let Some(ref mut ch) = self.child {
            match ch.try_wait() {
                Ok(Some(_)) => {}
                _ => {
                    let _ = ch.kill();
                    let _ = ch.wait();
                }
            }
        }
        self.child = None;
        let _ = std::fs::remove_dir_all(&self.profile_dir);
    }
}

impl Drop for PopupBrowserGuard {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Build the Chromium `--app` popup launch argument vector (pure helper for unit tests).
pub fn build_chromium_popup_args(port: u16, profile_dir: &Path, url: &str) -> Vec<String> {
    vec![
        format!("--remote-debugging-port={port}"),
        format!("--user-data-dir={}", profile_dir.to_string_lossy()),
        format!("--app={url}"),
        "--window-size=500,720".to_string(),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
        "--new-window".to_string(),
    ]
}

fn session_ready_from_cdp(
    jar: &HashMap<String, String>,
    urls: &[String],
) -> bool {
    let still_on_accounts = urls.iter().any(|u| u.contains("accounts.google.com"));
    let landed_on_youtube = urls.iter().any(|u| u.contains("youtube.com"));
    !still_on_accounts
        && (landed_on_youtube || jar.contains_key("LOGIN_INFO"))
        && has_google_auth_cookies(jar)
        && jar.contains_key("LOGIN_INFO")
}

async fn run_chromium_popup_signin(
    client: &reqwest::Client,
    browser_path: &Path,
    browser_label: &str,
    mut cancel_rx: watch::Receiver<bool>,
    on_ready: impl FnOnce(String),
) -> std::result::Result<GoogleAccountSession, String> {
    let port = allocate_ephemeral_cdp_port()?;
    let profile_dir = std::env::temp_dir().join(format!(
        "phaenna-auth-popup-{}-{}",
        std::process::id(),
        port
    ));
    let _ = std::fs::remove_dir_all(&profile_dir);
    std::fs::create_dir_all(&profile_dir)
        .map_err(|e| format!("Failed creating temporary browser profile: {e}"))?;

    let args = build_chromium_popup_args(port, &profile_dir, GOOGLE_YOUTUBE_SERVICE_LOGIN_URL);
    let child = Command::new(browser_path)
        .args(&args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed launching {browser_label} popup: {e}"))?;

    let mut guard = PopupBrowserGuard {
        child: Some(child),
        profile_dir,
    };

    on_ready(format!("{browser_label} popup"));

    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => {
                guard.shutdown();
                return Err("Browser sign-in timed out after 5 minutes".to_string());
            }
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    guard.shutdown();
                    return Err("Sign-in cancelled".to_string());
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(900)) => {}
        }

        if *cancel_rx.borrow() {
            guard.shutdown();
            return Err("Sign-in cancelled".to_string());
        }

        if let Some(ref mut ch) = guard.child {
            if let Ok(Some(_)) = ch.try_wait() {
                guard.shutdown();
                return Err(
                    "Browser sign-in window was closed before completing login.".to_string(),
                );
            }
        }

        let Some((jar, netscape, target_urls)) =
            cdp_fetch_cookies_and_optionally_close(client, port, false, true).await
        else {
            continue;
        };

        if session_ready_from_cdp(&jar, &target_urls) {
            let _ = cdp_fetch_cookies_and_optionally_close(client, port, true, true).await;
            tokio::time::sleep(Duration::from_millis(200)).await;
            guard.shutdown();

            let mut session =
                build_first_party_session("google-account@youtube", jar, Some(netscape));
            hydrate_innertube_identity(client, &mut session).await;
            return Ok(session);
        }
    }
}

async fn run_terminal_browser_popup_signin(
    client: &reqwest::Client,
    mut cancel_rx: watch::Receiver<bool>,
    on_ready: impl FnOnce(String),
) -> std::result::Result<GoogleAccountSession, String> {
    let pre_existing_ports: HashSet<u16> = discover_terminal_browser_instances()
        .await
        .into_iter()
        .map(|(p, _)| p)
        .collect();

    Command::new("terminal-browser")
        .args([
            "open",
            GOOGLE_YOUTUBE_SERVICE_LOGIN_URL,
            "--split",
            "right",
            "--size",
            "0.38",
            "--no-merge",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("Could not launch terminal-browser ({e})"))?;

    on_ready("Kitty split (terminal-browser)".to_string());

    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => {
                return Err("Browser sign-in timed out after 5 minutes".to_string());
            }
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    return Err("Sign-in cancelled".to_string());
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(1200)) => {}
        }

        if *cancel_rx.borrow() {
            return Err("Sign-in cancelled".to_string());
        }

        let mut ordered = discover_terminal_browser_instances().await;
        ordered.sort_by_key(|(port, _)| usize::from(pre_existing_ports.contains(port)));

        for (port, ls_urls) in ordered {
            let Some((jar, netscape, target_urls)) =
                cdp_fetch_cookies_and_optionally_close(client, port, false, false).await
            else {
                continue;
            };

            let all_urls: Vec<String> = if target_urls.is_empty() {
                ls_urls
            } else {
                target_urls
            };

            if session_ready_from_cdp(&jar, &all_urls) {
                let is_dedicated = !pre_existing_ports.contains(&port);
                let _ =
                    cdp_fetch_cookies_and_optionally_close(client, port, true, is_dedicated).await;

                let mut session =
                    build_first_party_session("google-account@youtube", jar, Some(netscape));
                hydrate_innertube_identity(client, &mut session).await;
                return Ok(session);
            }
        }
    }
}

/// Launch an isolated Browser Popup for official Google Sign-In and auto-capture YouTube cookies via CDP.
pub async fn run_browser_popup_google_signin(
    client: reqwest::Client,
    cancel_rx: watch::Receiver<bool>,
    on_ready: impl FnOnce(String),
) -> std::result::Result<GoogleAccountSession, String> {
    if let Some((path, label)) = find_chromium_popup_browser() {
        return run_chromium_popup_signin(&client, &path, label, cancel_rx, on_ready).await;
    }

    if find_binary("terminal-browser").is_some() {
        return run_terminal_browser_popup_signin(&client, cancel_rx, on_ready).await;
    }

    #[cfg(target_os = "linux")]
    let msg = "No Chromium-based browser found (chromium, google-chrome, brave, edge). Install Chromium (e.g. `sudo pacman -S chromium`) and press [Enter] to sign in.";
    #[cfg(target_os = "macos")]
    let msg = "No Chromium-based browser found (Google Chrome, Chromium, Brave, Edge). Install Chrome or Chromium and press [Enter] to sign in.";
    #[cfg(target_os = "windows")]
    let msg = "No Chromium-based browser found (Edge, Chrome, Brave). Install Edge or Chrome and press [Enter] to sign in.";
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    let msg = "No Chromium-based browser found for popup sign-in.";

    Err(msg.to_string())
}

/// Silently refresh an expiring OAuth2 access token if a legacy OAuth session is stored.
pub async fn refresh_session_token(
    client: &reqwest::Client,
    oauth: Option<&OAuthClient>,
    session: &mut GoogleAccountSession,
) -> Result<bool> {
    let Some(refresh_tok) = session.refresh_token.clone() else {
        return Ok(false);
    };
    let Some(creds) = oauth else {
        return Ok(false);
    };

    let mut params = vec![
        ("client_id", creds.client_id.as_str()),
        ("refresh_token", refresh_tok.as_str()),
        ("grant_type", "refresh_token"),
    ];
    if !creds.client_secret.is_empty() {
        params.push(("client_secret", creds.client_secret.as_str()));
    }

    let resp = client
        .post(GOOGLE_TOKEN_ENDPOINT)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(encode_form(&params))
        .send()
        .await?;

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(AppError::Auth(format!(
            "Token refresh failed (HTTP {}): {body}",
            status.as_u16()
        )));
    }

    let tok: OAuthTokenResponse = serde_json::from_str(&body)
        .map_err(|e| AppError::Parse(format!("Invalid refresh token JSON: {e}")))?;

    let now = now_unix_secs();
    session.access_token = Some(tok.access_token);
    if let Some(new_rt) = tok.refresh_token {
        session.refresh_token = Some(new_rt);
    }
    session.expires_at = Some(now + tok.expires_in);
    session.last_refreshed = now;
    Ok(true)
}

fn extract_menu_text(val: &serde_json::Value) -> Option<String> {
    if let Some(s) = val.get("simpleText").and_then(|v| v.as_str()) {
        return Some(s.to_string());
    }
    if let Some(runs) = val.get("runs").and_then(|v| v.as_array()) {
        let joined: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(|t| t.as_str()))
            .collect();
        if !joined.is_empty() {
            return Some(joined);
        }
    }
    None
}

/// Extract `(display_name, channel_handle, email)` from InnerTube `account_menu` JSON.
pub fn extract_account_menu_identity(
    root: &serde_json::Value,
) -> (Option<String>, Option<String>, Option<String>) {
    fn walk(
        node: &serde_json::Value,
        name: &mut Option<String>,
        handle: &mut Option<String>,
        email: &mut Option<String>,
    ) {
        match node {
            serde_json::Value::Object(map) => {
                if name.is_none() {
                    if let Some(v) = map.get("accountName").and_then(extract_menu_text) {
                        *name = Some(v);
                    }
                }
                if handle.is_none() {
                    if let Some(v) = map.get("channelHandle").and_then(extract_menu_text) {
                        *handle = Some(v);
                    }
                }
                if email.is_none() {
                    if let Some(v) = map
                        .get("email")
                        .and_then(extract_menu_text)
                        .filter(|s| s.contains('@'))
                    {
                        *email = Some(v);
                    }
                }
                for val in map.values() {
                    walk(val, name, handle, email);
                }
            }
            serde_json::Value::Array(arr) => {
                for item in arr {
                    walk(item, name, handle, email);
                }
            }
            serde_json::Value::String(s) => {
                if handle.is_none() && s.starts_with('@') && s.len() > 2 && !s.contains(' ') {
                    *handle = Some(s.clone());
                } else if email.is_none()
                    && s.contains('@')
                    && !s.starts_with('@')
                    && s.contains('.')
                    && !s.contains(' ')
                {
                    *email = Some(s.clone());
                }
            }
            _ => {}
        }
    }

    let mut name = None;
    let mut handle = None;
    let mut email = None;
    walk(root, &mut name, &mut handle, &mut email);
    (name, handle, email)
}

async fn hydrate_innertube_identity(client: &reqwest::Client, session: &mut GoogleAccountSession) {
    let Some(sapisid) = session.sapisid().map(str::to_string) else {
        return;
    };
    let Some(cookie_hdr) = session.cookie_header() else {
        return;
    };
    let hash = compute_sapisidhash(&sapisid, "https://www.youtube.com", now_unix_secs());
    let payload = serde_json::json!({
        "context": {
            "client": {
                "clientName": "WEB",
                "clientVersion": "2.20251015.01.00"
            }
        }
    });

    if let Ok(resp) = client
        .post("https://www.youtube.com/youtubei/v1/account/account_menu")
        .header(USER_AGENT, PHAENNA_USER_AGENT)
        .header(AUTHORIZATION, format!("SAPISIDHASH {hash}"))
        .header(COOKIE, cookie_hdr)
        .header("Origin", "https://www.youtube.com")
        .header("X-Origin", "https://www.youtube.com")
        .header("X-YouTube-Client-Name", "1")
        .header("X-YouTube-Client-Version", "2.20251015.01.00")
        .json(&payload)
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(val) = resp.json::<serde_json::Value>().await {
                let (name, handle, email) = extract_account_menu_identity(&val);
                if let Some(n) = name {
                    session.display_name = n;
                }
                if let Some(h) = handle {
                    session.channel_handle = Some(h.clone());
                    if session.display_name == "Google User"
                        || session.display_name == "google-account"
                    {
                        session.display_name = h.trim_start_matches('@').to_string();
                    }
                }
                if let Some(e) = email {
                    session.email = e.clone();
                    session.account_id = format!("yt:{e}");
                } else if let Some(ref h) = session.channel_handle {
                    session.account_id = format!("yt:{h}");
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_sapisidhash_determinism() {
        let h1 = compute_sapisidhash("test_sapisid", "https://www.youtube.com", 1_700_000_000);
        let h2 = compute_sapisidhash("test_sapisid", "https://www.youtube.com", 1_700_000_000);
        assert_eq!(h1, h2);
        assert!(h1.starts_with("1700000000_"));
        assert_eq!(h1.len(), "1700000000_".len() + 40);
    }

    #[test]
    fn test_auth_vault_save_load_0600_and_multi_account_rotation() {
        let dir = tempdir().unwrap();
        let vault_path = dir.path().join("auth.json");

        let mut vault = AuthVault::default();
        vault.upsert_session(GoogleAccountSession {
            account_id: "yt:one@gmail.com".into(),
            email: "one@gmail.com".into(),
            display_name: "Account One".into(),
            channel_id: Some("UC111".into()),
            channel_handle: Some("@one".into()),
            avatar_url: None,
            auth_kind: AuthKind::FirstParty,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            cookies: HashMap::from([("SAPISID".into(), "s1".into())]),
            netscape_cookies: None,
            added_at: 100,
            last_refreshed: 100,
        });
        vault.upsert_session(GoogleAccountSession {
            account_id: "yt:two@gmail.com".into(),
            email: "two@gmail.com".into(),
            display_name: "Account Two".into(),
            channel_id: Some("UC222".into()),
            channel_handle: Some("@two".into()),
            avatar_url: None,
            auth_kind: AuthKind::FirstParty,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            cookies: HashMap::from([("SAPISID".into(), "s2".into())]),
            netscape_cookies: None,
            added_at: 200,
            last_refreshed: 200,
        });

        vault.save_to_path(&vault_path).unwrap();

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&vault_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }

        let mut loaded = AuthVault::load_from_path(&vault_path).unwrap();
        assert_eq!(loaded.accounts.len(), 2);
        assert_eq!(loaded.active_account().unwrap().account_id, "yt:two@gmail.com");

        assert!(loaded.set_active("yt:one@gmail.com"));
        assert_eq!(loaded.active_account().unwrap().email, "one@gmail.com");

        assert!(loaded.remove_account("yt:one@gmail.com"));
        assert_eq!(loaded.active_account().unwrap().account_id, "yt:two@gmail.com");
    }

    #[test]
    fn test_chromium_popup_args_and_cdp_session_readiness() {
        let dir = PathBuf::from("/tmp/phaenna-popup-test");
        let args = build_chromium_popup_args(49152, &dir, GOOGLE_YOUTUBE_SERVICE_LOGIN_URL);
        assert!(args.contains(&"--remote-debugging-port=49152".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/phaenna-popup-test".to_string()));
        assert!(args.iter().any(|a| a.starts_with("--app=https://accounts.google.com/")));
        assert!(args.contains(&"--window-size=500,720".to_string()));

        let mut jar = HashMap::from([
            ("SAPISID".into(), "sapi".into()),
            ("SID".into(), "sid".into()),
            ("LOGIN_INFO".into(), "li".into()),
        ]);
        assert!(!session_ready_from_cdp(
            &jar,
            &["https://accounts.google.com/v3/signin".to_string()]
        ));
        assert!(session_ready_from_cdp(
            &jar,
            &["https://www.youtube.com/".to_string()]
        ));
        jar.remove("LOGIN_INFO");
        assert!(!session_ready_from_cdp(
            &jar,
            &["https://www.youtube.com/".to_string()]
        ));
    }

    #[test]
    fn test_cdp_cookie_extraction_and_account_menu_identity() {
        let cdp_cookies = vec![
            serde_json::json!({
                "name": "SAPISID",
                "value": "sapi_val",
                "domain": ".youtube.com",
                "path": "/",
                "secure": true,
                "expires": -1
            }),
            serde_json::json!({
                "name": "__Secure-3PSID",
                "value": "psid_val",
                "domain": ".google.com",
                "path": "/",
                "secure": true,
                "expires": 1_800_000_000.0
            }),
            serde_json::json!({
                "name": "UNRELATED",
                "value": "ignore_me",
                "domain": ".example.com",
                "path": "/"
            }),
        ];

        let (jar, netscape) = parse_cdp_cookies(&cdp_cookies, 1_900_000_000);
        assert_eq!(jar.get("SAPISID").map(String::as_str), Some("sapi_val"));
        assert!(!jar.contains_key("UNRELATED"));
        assert!(netscape.contains(".youtube.com\tTRUE\t/\tTRUE\t1900000000\tSAPISID\tsapi_val"));
        assert!(netscape.contains(".google.com\tTRUE\t/\tTRUE\t1800000000\t__Secure-3PSID\tpsid_val"));
        assert!(!netscape.contains("example.com"));

        let menu = serde_json::json!({
            "actions": [{
                "openPopupAction": {
                    "popup": {
                        "multiPageMenuRenderer": {
                            "header": {
                                "activeAccountHeaderRenderer": {
                                    "accountName": { "simpleText": "Abdullah" },
                                    "channelHandle": { "simpleText": "@abdullah_yt" },
                                    "email": { "simpleText": "abdullah@gmail.com" }
                                }
                            }
                        }
                    }
                }
            }]
        });
        let (name, handle, email) = extract_account_menu_identity(&menu);
        assert_eq!(name.as_deref(), Some("Abdullah"));
        assert_eq!(handle.as_deref(), Some("@abdullah_yt"));
        assert_eq!(email.as_deref(), Some("abdullah@gmail.com"));
    }
}
