//! Self-contained In-App Google Sign-In, OAuth2 (Device Code + Kitty Split PKCE),
//! isolated session vault (`~/.config/phaenna/auth.json`, mode `0600`), and `SAPISIDHASH`
//! signing — zero browser SQLite cookie scraping.

use crate::config::GoogleOAuthConfig;
use crate::db::AccountRow;
use crate::error::{AppError, Result};
use crate::services::remote::generate_terminal_qr;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use futures_util::{SinkExt, StreamExt};
use rand::RngCore;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, SET_COOKIE, USER_AGENT};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio_tungstenite::tungstenite::Message;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub const GOOGLE_YOUTUBE_SERVICE_LOGIN_URL: &str =
    "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F";

pub const PKCE_CALLBACK_PORT: u16 = 41933;
pub const PKCE_REDIRECT_URI: &str = "http://127.0.0.1:41933/google/callback";
pub const GOOGLE_DEVICE_CODE_ENDPOINT: &str = "https://oauth2.googleapis.com/device/code";
pub const GOOGLE_TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";
pub const GOOGLE_AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const GOOGLE_USERINFO_ENDPOINT: &str = "https://www.googleapis.com/oauth2/v3/userinfo";
pub const YOUTUBE_CHANNELS_MINE_ENDPOINT: &str =
    "https://www.googleapis.com/youtube/v3/channels?part=snippet&mine=true";
pub const YOUTUBE_OAUTH_SCOPE: &str = "https://www.googleapis.com/auth/youtube openid email profile";
pub const PHAENNA_USER_AGENT: &str =
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

/// Returns current UNIX epoch timestamp in seconds.
pub fn now_unix_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Zeroize-backed password buffer whose memory is scrubbed on clear and drop.
#[derive(Clone, Default, Zeroize, ZeroizeOnDrop)]
pub struct SecurePassword(String);

impl std::fmt::Debug for SecurePassword {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SecurePassword([REDACTED])")
    }
}

#[allow(dead_code)]
impl SecurePassword {
    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn push(&mut self, c: char) {
        self.0.push(c);
    }

    pub fn pop(&mut self) {
        self.0.pop();
    }

    pub fn clear(&mut self) {
        self.0.zeroize();
    }

    pub fn len(&self) -> usize {
        self.0.chars().count()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn masked(&self) -> String {
        "•".repeat(self.len())
    }
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
            Self::FirstParty => "Google Session (In-App Vault)",
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
    /// True when the OAuth2 access token expires within 90 seconds (or is already expired).
    pub fn is_token_expiring(&self, now_unix: i64) -> bool {
        matches!(self.expires_at, Some(exp) if now_unix >= exp - 90)
    }

    /// Remaining validity window in seconds (None if session uses persistent cookies without expiry).
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

    /// Deterministic `Cookie:` header built from isolated first-party session cookies.
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

    /// Format isolated session cookies into a temporary Netscape cookie jar when needed by `yt-dlp`.
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

/// 2FA verification challenge kind presented inline inside `[L]`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ChallengeKind {
    TotpCode,
    SmsCode { hint: String },
    GooglePrompt { number: Option<u8> },
    BackupCode,
}

/// OAuth2 flow waiting to resume once the user supplies a Google Cloud OAuth client ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PendingOAuthFlow {
    DeviceCode,
    KittyPkce,
}

/// Multi-stage state machine driving the `[L]` Google Identity Hub overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginStep {
    Hub,
    EmailInput,
    PasswordInput,
    TwoFactorChallenge(ChallengeKind),
    AwaitingDeviceQr,
    AwaitingKittySso,
    ClientIdSetup,
}

/// Active RFC 8628 Device Code authorization payload rendered inside `[L]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceCodeSession {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub verification_url_complete: String,
    pub expires_at: i64,
    pub interval_secs: u64,
    pub qr_lines: Vec<String>,
}

/// Interactive state for the `[L]` Google Identity Hub modal.
#[derive(Debug)]
pub struct LoginUiState {
    pub step: LoginStep,
    pub email: String,
    pub password: SecurePassword,
    pub show_password: bool,
    pub two_factor_code: String,
    pub backup_mode: bool,
    pub client_id_input: String,
    pub client_secret_input: String,
    pub setup_field_idx: usize,
    pub setup_after: Option<PendingOAuthFlow>,
    pub account_cursor: usize,
    pub status_banner: Option<String>,
    pub error_banner: Option<String>,
    pub busy: bool,
    pub device_session: Option<DeviceCodeSession>,
    pub kitty_auth_url: Option<String>,
    pub pending_flow_cookies: HashMap<String, String>,
    pub pending_challenge_token: Option<String>,
    pub cancel_tx: Option<watch::Sender<bool>>,
}

impl Default for LoginUiState {
    fn default() -> Self {
        Self {
            step: LoginStep::Hub,
            email: String::new(),
            password: SecurePassword::default(),
            show_password: false,
            two_factor_code: String::new(),
            backup_mode: false,
            client_id_input: String::new(),
            client_secret_input: String::new(),
            setup_field_idx: 0,
            setup_after: None,
            account_cursor: 0,
            status_banner: None,
            error_banner: None,
            busy: false,
            device_session: None,
            kitty_auth_url: None,
            pending_flow_cookies: HashMap::new(),
            pending_challenge_token: None,
            cancel_tx: None,
        }
    }
}

impl LoginUiState {
    /// Cancel any background pollers/listeners and zeroize sensitive transient buffers.
    pub fn reset_transient(&mut self) {
        if let Some(tx) = self.cancel_tx.take() {
            let _ = tx.send(true);
        }
        self.password.clear();
        self.show_password = false;
        self.two_factor_code.zeroize();
        self.backup_mode = false;
        self.busy = false;
        self.device_session = None;
        self.kitty_auth_url = None;
        self.pending_flow_cookies.clear();
        self.pending_challenge_token = None;
        self.setup_after = None;
    }

    pub fn back_to_hub(&mut self) {
        self.reset_transient();
        self.step = LoginStep::Hub;
    }
}

/// Resolved Google OAuth2 client credentials for Device Code / Loopback PKCE.
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

/// Generate a 43-character URL-safe PKCE `code_verifier` (RFC 7636).
pub fn generate_pkce_verifier() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}

/// Compute S256 PKCE `code_challenge = BASE64URL(SHA256(verifier))`.
pub fn compute_pkce_s256_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

/// Generate a cryptographic OAuth2 `state` anti-CSRF nonce.
pub fn generate_oauth_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
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

fn decode_query_component(s: &str) -> String {
    let plus_spaces = s.replace('+', " ");
    let bytes = plus_spaces.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(hex_str) = std::str::from_utf8(&bytes[i + 1..i + 3]) {
                if let Ok(val) = u8::from_str_radix(hex_str, 16) {
                    decoded.push(val);
                    i += 3;
                    continue;
                }
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&decoded).into_owned()
}

/// Parse an HTTP request line `/google/callback?code=...&state=...` into `(code, state)`.
pub fn parse_oauth_callback_query(request_line: &str) -> Result<(String, String)> {
    let path_and_query = request_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| AppError::Auth("Malformed HTTP callback request line".into()))?;

    let (path, query) = path_and_query
        .split_once('?')
        .ok_or_else(|| AppError::Auth("OAuth callback missing query parameters".into()))?;
    if !path.starts_with("/google/callback") {
        return Err(AppError::Auth(format!("Unexpected callback path: {path}")));
    }

    let mut code = None;
    let mut state = None;
    let mut error = None;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        let val = decode_query_component(v);
        match k {
            "code" => code = Some(val),
            "state" => state = Some(val),
            "error" => error = Some(val),
            _ => {}
        }
    }

    if let Some(err) = error {
        return Err(AppError::Auth(format!("Google returned OAuth error: {err}")));
    }
    match (code, state) {
        (Some(c), Some(s)) if !c.is_empty() && !s.is_empty() => Ok((c, s)),
        _ => Err(AppError::Auth(
            "OAuth callback did not include authorization code and state".into(),
        )),
    }
}

#[derive(Debug, Deserialize)]
struct DeviceCodeResponse {
    device_code: String,
    user_code: String,
    #[serde(default = "default_verification_url")]
    verification_url: String,
    #[serde(default)]
    verification_url_complete: Option<String>,
    #[serde(default = "default_expires_in")]
    expires_in: i64,
    #[serde(default = "default_poll_interval")]
    interval: u64,
}

fn default_verification_url() -> String {
    "https://www.google.com/device".to_string()
}

fn default_expires_in() -> i64 {
    1800
}

fn default_poll_interval() -> u64 {
    5
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    #[serde(default)]
    pub refresh_token: Option<String>,
    #[serde(default = "default_token_ttl")]
    pub expires_in: i64,
    #[serde(default)]
    pub token_type: Option<String>,
}

fn default_token_ttl() -> i64 {
    3600
}

#[derive(Debug, Deserialize)]
struct OAuthErrorBody {
    error: String,
    #[serde(default)]
    error_description: Option<String>,
}

/// Classified poll status for RFC 8628 Google Device Code token polling.
#[derive(Debug, PartialEq, Eq)]
pub enum DevicePollStatus {
    Pending,
    SlowDown,
    Approved(OAuthTokenResponse),
    Failed(String),
}

/// Interpret Google's `/token` response during Device Code polling.
pub fn parse_device_token_response(status_code: u16, body: &str) -> DevicePollStatus {
    if (200..300).contains(&status_code) {
        if let Ok(tok) = serde_json::from_str::<OAuthTokenResponse>(body) {
            return DevicePollStatus::Approved(tok);
        }
    }
    if let Ok(err) = serde_json::from_str::<OAuthErrorBody>(body) {
        return match err.error.as_str() {
            "authorization_pending" => DevicePollStatus::Pending,
            "slow_down" => DevicePollStatus::SlowDown,
            "access_denied" => {
                DevicePollStatus::Failed("Sign-in declined on approval device".to_string())
            }
            "expired_token" => {
                DevicePollStatus::Failed("Device code expired — press [2] to generate a new QR".to_string())
            }
            other => DevicePollStatus::Failed(
                err.error_description
                    .unwrap_or_else(|| format!("Google OAuth error: {other}")),
            ),
        };
    }
    DevicePollStatus::Failed(format!(
        "Unexpected token endpoint response (HTTP {status_code})"
    ))
}

/// Flow B — Request a Google Device Code + terminal half-block QR matrix.
pub async fn request_device_code(
    client: &reqwest::Client,
    oauth: &OAuthClient,
) -> Result<DeviceCodeSession> {
    let body = encode_form(&[
        ("client_id", &oauth.client_id),
        ("scope", YOUTUBE_OAUTH_SCOPE),
    ]);
    let resp = client
        .post(GOOGLE_DEVICE_CODE_ENDPOINT)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await?;

    let status = resp.status();
    let text = resp.text().await?;
    if !status.is_success() {
        let msg = serde_json::from_str::<OAuthErrorBody>(&text)
            .map(|e| e.error_description.unwrap_or(e.error))
            .unwrap_or(text);
        return Err(AppError::Auth(format!(
            "Google Device Code request failed (HTTP {}): {msg}",
            status.as_u16()
        )));
    }

    let parsed: DeviceCodeResponse = serde_json::from_str(&text)
        .map_err(|e| AppError::Parse(format!("Invalid Device Code JSON: {e}")))?;

    let full_url = parsed.verification_url_complete.clone().unwrap_or_else(|| {
        format!(
            "{}?user_code={}",
            parsed.verification_url,
            percent_encode(&parsed.user_code)
        )
    });
    let qr_lines = generate_terminal_qr(&full_url);

    Ok(DeviceCodeSession {
        device_code: parsed.device_code,
        user_code: parsed.user_code,
        verification_url: parsed.verification_url,
        verification_url_complete: full_url,
        expires_at: now_unix_secs() + parsed.expires_in,
        interval_secs: parsed.interval.max(3),
        qr_lines,
    })
}

/// Flow B — Background poller resolving `GoogleAccountSession` once the user approves on phone/device.
pub async fn poll_device_code_until_done(
    client: reqwest::Client,
    oauth: OAuthClient,
    session: DeviceCodeSession,
    mut cancel_rx: watch::Receiver<bool>,
) -> std::result::Result<GoogleAccountSession, String> {
    let mut interval = session.interval_secs.max(3);

    loop {
        if *cancel_rx.borrow() {
            return Err("Cancelled by user".to_string());
        }
        if now_unix_secs() >= session.expires_at {
            return Err("Device code expired — press [2] to generate a fresh QR".to_string());
        }

        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(interval)) => {}
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    return Err("Cancelled by user".to_string());
                }
            }
        }

        let mut params = vec![
            ("client_id", oauth.client_id.as_str()),
            ("device_code", session.device_code.as_str()),
            ("grant_type", "urn:ietf:params:oauth:grant-type:device_code"),
        ];
        if !oauth.client_secret.is_empty() {
            params.push(("client_secret", oauth.client_secret.as_str()));
        }

        let resp = match client
            .post(GOOGLE_TOKEN_ENDPOINT)
            .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(encode_form(&params))
            .send()
            .await
        {
            Ok(r) => r,
            Err(_) => continue, // transient network jitter while waiting
        };

        let status_code = resp.status().as_u16();
        let body = resp.text().await.unwrap_or_default();
        match parse_device_token_response(status_code, &body) {
            DevicePollStatus::Pending => continue,
            DevicePollStatus::SlowDown => {
                interval = (interval + 5).min(30);
            }
            DevicePollStatus::Failed(reason) => return Err(reason),
            DevicePollStatus::Approved(tok) => {
                return build_oauth_session(&client, tok, AuthKind::OAuthDevice)
                    .await
                    .map_err(|e| e.to_string());
            }
        }
    }
}

/// Build Google's OAuth2 authorization URL with S256 PKCE for loopback `127.0.0.1:41933`.
pub fn build_pkce_authorize_url(client_id: &str, code_challenge: &str, state: &str) -> String {
    let params = [
        ("client_id", client_id),
        ("redirect_uri", PKCE_REDIRECT_URI),
        ("response_type", "code"),
        ("scope", YOUTUBE_OAUTH_SCOPE),
        ("code_challenge", code_challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "select_account consent"),
    ];
    format!("{GOOGLE_AUTH_ENDPOINT}?{}", encode_form(&params))
}

/// Launch the login page inside Kitty's right split via `terminal-browser` when running inside Kitty,
/// or fall back cleanly to the platform opener if not inside Kitty.
pub fn open_url_in_kitty_or_system(url: &str, prefer_kitty_split: bool) -> &'static str {
    let in_kitty = std::env::var_os("KITTY_PID").is_some();
    if prefer_kitty_split && in_kitty {
        if let Ok(child) = std::process::Command::new("terminal-browser")
            .args(["open", url, "--split", "right", "--size", "0.35"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            let _ = child;
            return "Kitty right split (terminal-browser)";
        }
    }

    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open").arg(url).spawn();
    #[cfg(target_os = "linux")]
    let _ = std::process::Command::new("xdg-open").arg(url).spawn();
    #[cfg(target_os = "windows")]
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn();

    "local loopback SSO pane"
}

fn build_dark_callback_html(title: &str, detail: &str, is_error: bool) -> String {
    let accent = if is_error { "#f87171" } else { "#4ade80" };
    let icon = if is_error { "!" } else { "OK" };
    format!(
        "<!doctype html><html><head><meta charset='utf-8'><title>Phaenna Auth</title>\
<style>body{{margin:0;height:100vh;display:flex;align-items:center;justify-content:center;\
background:#0b0b12;color:#f4f4f6;font-family:-apple-system,BlinkMacSystemFont,sans-serif;}}\
.card{{border:1px solid #27273a;background:#141422;padding:32px 36px;border-radius:14px;\
max-width:440px;text-align:center;}}.badge{{font-size:36px;color:{accent};margin-bottom:8px;}}\
h1{{font-size:20px;margin:0 0 8px 0;}}p{{color:#a1a1b5;font-size:14px;line-height:1.5;margin:0;}}\
</style></head><body><div class='card'><div class='badge'>{icon}</div>\
<h1>{title}</h1><p>{detail}</p></div></body></html>"
    )
}

async fn write_http_response(stream: &mut tokio::net::TcpStream, status: &str, body: &str) {
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Flow C — Single-shot loopback PKCE OAuth2 listener on `127.0.0.1:41933` with Kitty split support.
pub async fn run_loopback_pkce_flow(
    client: reqwest::Client,
    oauth: OAuthClient,
    prefer_kitty_split: bool,
    mut cancel_rx: watch::Receiver<bool>,
    on_url_ready: impl FnOnce(String, &'static str),
) -> std::result::Result<GoogleAccountSession, String> {
    let listener = TcpListener::bind(("127.0.0.1", PKCE_CALLBACK_PORT))
        .await
        .map_err(|e| {
            format!(
                "Port {PKCE_CALLBACK_PORT} busy or unavailable for OAuth callback: {e}"
            )
        })?;

    let verifier = generate_pkce_verifier();
    let challenge = compute_pkce_s256_challenge(&verifier);
    let state_nonce = generate_oauth_state();
    let auth_url = build_pkce_authorize_url(&oauth.client_id, &challenge, &state_nonce);
    let launcher_label = open_url_in_kitty_or_system(&auth_url, prefer_kitty_split);
    on_url_ready(auth_url, launcher_label);

    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);

    let code = loop {
        tokio::select! {
            _ = &mut deadline => {
                return Err("Loopback OAuth timed out after 5 minutes".to_string());
            }
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    return Err("OAuth sign-in cancelled".to_string());
                }
            }
            accepted = listener.accept() => {
                let (mut stream, _) = accepted.map_err(|e| format!("Callback accept failed: {e}"))?;
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");

                if first_line.contains("/favicon.ico") {
                    write_http_response(&mut stream, "404 Not Found", "").await;
                    continue;
                }

                match parse_oauth_callback_query(first_line) {
                    Ok((auth_code, recv_state)) if recv_state == state_nonce => {
                        let html = build_dark_callback_html(
                            "Connected to Phaenna",
                            "Google authorization succeeded. You can close this split pane and return to Phaenna.",
                            false,
                        );
                        write_http_response(&mut stream, "200 OK", &html).await;
                        break auth_code;
                    }
                    Ok(_) => {
                        let html = build_dark_callback_html(
                            "Security Check Failed",
                            "OAuth state nonce mismatch. Please restart sign-in inside Phaenna.",
                            true,
                        );
                        write_http_response(&mut stream, "400 Bad Request", &html).await;
                        return Err("OAuth state mismatch (possible CSRF interruption)".to_string());
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        let html = build_dark_callback_html("Google Sign-In Cancelled", &msg, true);
                        write_http_response(&mut stream, "400 Bad Request", &html).await;
                        return Err(msg);
                    }
                }
            }
        }
    };

    let mut params = vec![
        ("client_id", oauth.client_id.as_str()),
        ("code", code.as_str()),
        ("code_verifier", verifier.as_str()),
        ("grant_type", "authorization_code"),
        ("redirect_uri", PKCE_REDIRECT_URI),
    ];
    if !oauth.client_secret.is_empty() {
        params.push(("client_secret", oauth.client_secret.as_str()));
    }

    let resp = client
        .post(GOOGLE_TOKEN_ENDPOINT)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .body(encode_form(&params))
        .send()
        .await
        .map_err(|e| format!("Token exchange network error: {e}"))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        return Err(format!(
            "Token exchange failed (HTTP {}): {text}",
            status.as_u16()
        ));
    }

    let tok: OAuthTokenResponse = serde_json::from_str(&text)
        .map_err(|e| format!("Invalid OAuth token JSON: {e}"))?;
    build_oauth_session(&client, tok, AuthKind::OAuthPkce)
        .await
        .map_err(|e| e.to_string())
}

/// Outcome of Flow A in-TUI Google Credential & 2FA submission.
#[derive(Debug)]
#[allow(dead_code)]
pub enum DirectAuthOutcome {
    SignedIn(Box<GoogleAccountSession>),
    NeedsTwoFactor {
        kind: ChallengeKind,
        cookies: HashMap<String, String>,
        challenge_token: Option<String>,
    },
    RequiresModernSso {
        reason: String,
    },
}

fn absorb_set_cookies(headers: &reqwest::header::HeaderMap, jar: &mut HashMap<String, String>) {
    for val in headers.get_all(SET_COOKIE) {
        if let Ok(raw) = val.to_str() {
            if let Some((kv, _)) = raw.split_once(';') {
                if let Some((name, value)) = kv.split_once('=') {
                    let k = name.trim();
                    let v = value.trim();
                    if !k.is_empty() && !v.is_empty() && !v.eq_ignore_ascii_case("deleted") {
                        jar.insert(k.to_string(), v.to_string());
                    }
                }
            }
        }
    }
}

fn cookie_header_from_map(jar: &HashMap<String, String>) -> String {
    let mut pairs: Vec<(&String, &String)> = jar.iter().collect();
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .into_iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect::<Vec<_>>()
        .join("; ")
}

fn has_google_auth_cookies(jar: &HashMap<String, String>) -> bool {
    let has_sid = jar.contains_key("SID") || jar.contains_key("__Secure-3PSID");
    let has_sapisid = jar.contains_key("SAPISID") || jar.contains_key("__Secure-3PAPISID");
    has_sid && has_sapisid
}

/// Flow A — Submit email & password directly inside Phaenna's TUI without external browsers.
/// Handles classic cookie issuance, 2-Step Verification detection, and clean Passkey/BotGuard fallback guidance.
pub async fn submit_direct_credentials(
    client: &reqwest::Client,
    email: &str,
    password: &SecurePassword,
) -> Result<DirectAuthOutcome> {
    let trimmed_email = email.trim();
    if trimmed_email.is_empty() || !trimmed_email.contains('@') {
        return Err(AppError::Auth("Enter a valid Google email address.".into()));
    }
    if password.is_empty() {
        return Err(AppError::Auth("Password cannot be empty.".into()));
    }

    let mut jar: HashMap<String, String> = HashMap::new();

    // 1. Prime initial Google ServiceLogin session cookies
    let init_url = "https://accounts.google.com/ServiceLogin?service=youtube&continue=https%3A%2F%2Fwww.youtube.com%2F";
    let init_resp = client
        .get(init_url)
        .header(USER_AGENT, PHAENNA_USER_AGENT)
        .send()
        .await?;
    absorb_set_cookies(init_resp.headers(), &mut jar);
    let init_html = init_resp.text().await.unwrap_or_default();

    let gxf = extract_hidden_input(&init_html, "gxf").unwrap_or_default();
    let dsh = extract_hidden_input(&init_html, "dsh").unwrap_or_default();

    // 2. Submit credentials to Google's ServiceLoginAuth endpoint with active jar
    let mut form_fields = vec![
        ("Email", trimmed_email),
        ("Passwd", password.as_str()),
        ("service", "youtube"),
        ("continue", "https://www.youtube.com/"),
    ];
    if !gxf.is_empty() {
        form_fields.push(("gxf", gxf.as_str()));
    }
    if !dsh.is_empty() {
        form_fields.push(("dsh", dsh.as_str()));
    }

    let auth_resp = client
        .post("https://accounts.google.com/signin/v1/lookup")
        .header(USER_AGENT, PHAENNA_USER_AGENT)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(COOKIE, cookie_header_from_map(&jar))
        .body(encode_form(&form_fields))
        .send()
        .await?;
    absorb_set_cookies(auth_resp.headers(), &mut jar);
    let auth_body = auth_resp.text().await.unwrap_or_default();

    if has_google_auth_cookies(&jar) {
        let mut session = build_first_party_session(trimmed_email, jar, None);
        hydrate_innertube_identity(client, &mut session).await;
        return Ok(DirectAuthOutcome::SignedIn(Box::new(session)));
    }

    // Check for interactive 2FA challenge hints returned by Google
    let lower = auth_body.to_ascii_lowercase();
    if lower.contains("totp") || lower.contains("authenticator") {
        return Ok(DirectAuthOutcome::NeedsTwoFactor {
            kind: ChallengeKind::TotpCode,
            cookies: jar,
            challenge_token: extract_hidden_input(&auth_body, "TL"),
        });
    }
    if lower.contains("sms") || lower.contains("phone") && lower.contains("code") {
        return Ok(DirectAuthOutcome::NeedsTwoFactor {
            kind: ChallengeKind::SmsCode {
                hint: "your recovery phone".to_string(),
            },
            cookies: jar,
            challenge_token: extract_hidden_input(&auth_body, "TL"),
        });
    }
    if lower.contains("prompt") || lower.contains("tap yes") {
        return Ok(DirectAuthOutcome::NeedsTwoFactor {
            kind: ChallengeKind::GooglePrompt { number: None },
            cookies: jar,
            challenge_token: extract_hidden_input(&auth_body, "TL"),
        });
    }

    Ok(DirectAuthOutcome::RequiresModernSso {
        reason: "Google requires Passkey / JS BotGuard verification for direct password sign-in. Press [1] for Instant Kitty Split Sign-In or [3] for Phone QR.".to_string(),
    })
}

/// Flow A — Submit inline 2FA OTP / SMS / Backup code against an active challenge session.
pub async fn submit_two_factor_challenge(
    client: &reqwest::Client,
    email: &str,
    code: &str,
    mut jar: HashMap<String, String>,
    challenge_token: Option<String>,
) -> Result<DirectAuthOutcome> {
    let cleaned: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
    if cleaned.len() < 6 {
        return Err(AppError::Auth(
            "Enter at least 6 digits for 2-Step Verification.".into(),
        ));
    }

    let mut fields = vec![("Pin", cleaned.as_str()), ("service", "youtube")];
    let tl = challenge_token.unwrap_or_default();
    if !tl.is_empty() {
        fields.push(("TL", tl.as_str()));
    }

    let resp = client
        .post("https://accounts.google.com/signin/challenge/totp")
        .header(USER_AGENT, PHAENNA_USER_AGENT)
        .header(CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(COOKIE, cookie_header_from_map(&jar))
        .body(encode_form(&fields))
        .send()
        .await?;
    absorb_set_cookies(resp.headers(), &mut jar);

    if has_google_auth_cookies(&jar) {
        let mut session = build_first_party_session(email, jar, None);
        hydrate_innertube_identity(client, &mut session).await;
        return Ok(DirectAuthOutcome::SignedIn(Box::new(session)));
    }

    Ok(DirectAuthOutcome::RequiresModernSso {
        reason: "Google rejected direct 2FA submission (WebAuthn / JS challenge required). Press [1] for Instant Kitty Split Sign-In or [3] for Phone QR.".to_string(),
    })
}

fn extract_hidden_input(html: &str, field_name: &str) -> Option<String> {
    let needle = format!("name=\"{field_name}\"");
    let pos = html.find(&needle)?;
    let window = &html[pos..html.len().min(pos + 300)];
    let val_pos = window.find("value=\"")?;
    let rest = &window[val_pos + 7..];
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
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
                .send(Message::Text(r#"{"id":2,"method":"Browser.close","params":{}}"#.into()))
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

/// Zero-Setup Flow: Open Google's official YouTube sign-in inside Kitty's right split
/// (`terminal-browser --split right --no-merge`) and bridge authenticated session cookies via CDP.
pub async fn run_kitty_split_google_signin(
    client: reqwest::Client,
    mut cancel_rx: watch::Receiver<bool>,
    on_status: impl Fn(String),
) -> std::result::Result<GoogleAccountSession, String> {
    let pre_existing_ports: HashSet<u16> = discover_terminal_browser_instances()
        .await
        .into_iter()
        .map(|(p, _)| p)
        .collect();

    std::process::Command::new("terminal-browser")
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
        .map_err(|e| {
            format!(
                "Could not launch terminal-browser ({e}). Use [2] In-TUI Form or ensure terminal-browser is in PATH."
            )
        })?;

    on_status(
        "Opened Google Sign-In in Kitty right split — sign in there to connect automatically"
            .to_string(),
    );

    let deadline = tokio::time::sleep(Duration::from_secs(300));
    tokio::pin!(deadline);

    loop {
        tokio::select! {
            _ = &mut deadline => {
                return Err("Kitty split sign-in timed out after 5 minutes".to_string());
            }
            _ = cancel_rx.changed() => {
                if *cancel_rx.borrow() {
                    return Err("OAuth sign-in cancelled".to_string());
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(1200)) => {}
        }

        if *cancel_rx.borrow() {
            return Err("OAuth sign-in cancelled".to_string());
        }

        let instances = discover_terminal_browser_instances().await;
        // Prefer newly spawned split pane before any pre-existing terminal-browser windows.
        let mut ordered = instances.clone();
        ordered.sort_by_key(|(port, _)| usize::from(pre_existing_ports.contains(port)));

        for (port, ls_urls) in ordered {
            let Some((jar, netscape, target_urls)) =
                cdp_fetch_cookies_and_optionally_close(&client, port, false, false).await
            else {
                continue;
            };

            let all_urls: Vec<&String> = if target_urls.is_empty() {
                ls_urls.iter().collect()
            } else {
                target_urls.iter().collect()
            };
            let still_on_accounts = all_urls.iter().any(|u| u.contains("accounts.google.com"));
            let landed_on_youtube = all_urls.iter().any(|u| u.contains("youtube.com"));

            if !still_on_accounts
                && (landed_on_youtube || jar.contains_key("LOGIN_INFO"))
                && has_google_auth_cookies(&jar)
                && jar.contains_key("LOGIN_INFO")
            {
                let is_dedicated = !pre_existing_ports.contains(&port);
                let _ =
                    cdp_fetch_cookies_and_optionally_close(&client, port, true, is_dedicated).await;

                let mut session =
                    build_first_party_session("google-account@youtube", jar, Some(netscape));
                hydrate_innertube_identity(&client, &mut session).await;
                return Ok(session);
            }
        }
    }
}

/// Silently refresh an expiring OAuth2 access token using its stored `refresh_token`.
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

#[derive(Debug, Deserialize)]
struct GoogleUserInfo {
    #[serde(default)]
    sub: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    picture: Option<String>,
}

async fn build_oauth_session(
    client: &reqwest::Client,
    tok: OAuthTokenResponse,
    auth_kind: AuthKind,
) -> Result<GoogleAccountSession> {
    let now = now_unix_secs();
    let mut session = GoogleAccountSession {
        account_id: format!("oauth:{now}"),
        email: "google-account@youtube".to_string(),
        display_name: "YouTube Member".to_string(),
        channel_id: None,
        channel_handle: None,
        avatar_url: None,
        auth_kind,
        access_token: Some(tok.access_token.clone()),
        refresh_token: tok.refresh_token,
        expires_at: Some(now + tok.expires_in),
        cookies: HashMap::new(),
        netscape_cookies: None,
        added_at: now,
        last_refreshed: now,
    };
    hydrate_google_and_youtube_profile(client, &tok.access_token, &mut session).await;
    Ok(session)
}

/// Populate email, avatar, and YouTube channel ID/handle using Google OAuth userinfo & YouTube API v3.
pub async fn hydrate_google_and_youtube_profile(
    client: &reqwest::Client,
    access_token: &str,
    session: &mut GoogleAccountSession,
) {
    if let Ok(resp) = client
        .get(GOOGLE_USERINFO_ENDPOINT)
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(info) = resp.json::<GoogleUserInfo>().await {
                if !info.sub.is_empty() {
                    session.account_id = info.sub;
                }
                if !info.email.is_empty() {
                    session.email = info.email;
                }
                if let Some(name) = info.name.filter(|s| !s.is_empty()) {
                    session.display_name = name;
                }
                if info.picture.is_some() {
                    session.avatar_url = info.picture;
                }
            }
        }
    }

    if let Ok(resp) = client
        .get(YOUTUBE_CHANNELS_MINE_ENDPOINT)
        .header(AUTHORIZATION, format!("Bearer {access_token}"))
        .send()
        .await
    {
        if resp.status().is_success() {
            if let Ok(val) = resp.json::<serde_json::Value>().await {
                if let Some(first) = val
                    .get("items")
                    .and_then(|v| v.as_array())
                    .and_then(|arr| arr.first())
                {
                    if let Some(cid) = first.get("id").and_then(|v| v.as_str()) {
                        session.channel_id = Some(cid.to_string());
                    }
                    if let Some(snip) = first.get("snippet") {
                        if let Some(title) = snip.get("title").and_then(|v| v.as_str()) {
                            session.display_name = title.to_string();
                        }
                        if let Some(handle) = snip.get("customUrl").and_then(|v| v.as_str()) {
                            session.channel_handle = Some(handle.to_string());
                        }
                    }
                }
            }
        }
    }
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
                } else if email.is_none() && s.contains('@') && !s.starts_with('@') && s.contains('.') && !s.contains(' ') {
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
                    if session.display_name == "Google User" || session.display_name == "google-account" {
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
    fn test_pkce_s256_challenge_and_sapisidhash_determinism() {
        // RFC 7636 Appendix B test vector
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let expected = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert_eq!(compute_pkce_s256_challenge(verifier), expected);

        let gen = generate_pkce_verifier();
        assert_eq!(gen.len(), 43);

        let h1 = compute_sapisidhash("test_sapisid", "https://www.youtube.com", 1_700_000_000);
        let h2 = compute_sapisidhash("test_sapisid", "https://www.youtube.com", 1_700_000_000);
        assert_eq!(h1, h2);
        assert!(h1.starts_with("1700000000_"));
        assert_eq!(h1.len(), "1700000000_".len() + 40);
    }

    #[test]
    fn test_parse_device_token_responses() {
        assert_eq!(
            parse_device_token_response(428, r#"{"error":"authorization_pending"}"#),
            DevicePollStatus::Pending
        );
        assert_eq!(
            parse_device_token_response(400, r#"{"error":"slow_down"}"#),
            DevicePollStatus::SlowDown
        );
        assert!(matches!(
            parse_device_token_response(403, r#"{"error":"access_denied"}"#),
            DevicePollStatus::Failed(_)
        ));
        assert!(matches!(
            parse_device_token_response(400, r#"{"error":"expired_token"}"#),
            DevicePollStatus::Failed(_)
        ));
        let ok_json = r#"{"access_token":"ya29.abc","refresh_token":"1//xyz","expires_in":3600,"token_type":"Bearer"}"#;
        match parse_device_token_response(200, ok_json) {
            DevicePollStatus::Approved(tok) => {
                assert_eq!(tok.access_token, "ya29.abc");
                assert_eq!(tok.refresh_token.as_deref(), Some("1//xyz"));
                assert_eq!(tok.expires_in, 3600);
            }
            other => panic!("Expected Approved, got {other:?}"),
        }
    }

    #[test]
    fn test_parse_oauth_callback_query_and_state_validation() {
        let (code, state) = parse_oauth_callback_query(
            "GET /google/callback?state=nonce_123&code=4%2F0AfJohX HTTP/1.1",
        )
        .unwrap();
        assert_eq!(code, "4/0AfJohX");
        assert_eq!(state, "nonce_123");

        assert!(parse_oauth_callback_query(
            "GET /google/callback?error=access_denied&state=nonce_123 HTTP/1.1"
        )
        .is_err());
        assert!(parse_oauth_callback_query("GET /other?code=a&state=b HTTP/1.1").is_err());
    }

    #[test]
    fn test_auth_vault_save_load_0600_and_multi_account_rotation() {
        let dir = tempdir().unwrap();
        let vault_path = dir.path().join("auth.json");

        let mut vault = AuthVault::default();
        vault.upsert_session(GoogleAccountSession {
            account_id: "sub_1".into(),
            email: "one@gmail.com".into(),
            display_name: "Account One".into(),
            channel_id: Some("UC111".into()),
            channel_handle: Some("@one".into()),
            avatar_url: None,
            auth_kind: AuthKind::OAuthDevice,
            access_token: Some("tok1".into()),
            refresh_token: Some("ref1".into()),
            expires_at: Some(9_999_999_999),
            cookies: HashMap::new(),
            netscape_cookies: None,
            added_at: 100,
            last_refreshed: 100,
        });
        vault.upsert_session(GoogleAccountSession {
            account_id: "sub_2".into(),
            email: "two@gmail.com".into(),
            display_name: "Account Two".into(),
            channel_id: Some("UC222".into()),
            channel_handle: Some("@two".into()),
            avatar_url: None,
            auth_kind: AuthKind::OAuthPkce,
            access_token: Some("tok2".into()),
            refresh_token: Some("ref2".into()),
            expires_at: Some(9_999_999_999),
            cookies: HashMap::new(),
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
        assert_eq!(loaded.active_account().unwrap().account_id, "sub_2");

        assert!(loaded.set_active("sub_1"));
        assert_eq!(loaded.active_account().unwrap().email, "one@gmail.com");

        assert!(loaded.remove_account("sub_1"));
        assert_eq!(loaded.active_account().unwrap().account_id, "sub_2");
    }

    #[test]
    fn test_secure_password_zeroize_and_redacted_debug() {
        let mut pw = SecurePassword::default();
        pw.push('s');
        pw.push('e');
        pw.push('c');
        assert_eq!(pw.masked(), "•••");
        assert_eq!(format!("{pw:?}"), "SecurePassword([REDACTED])");
        pw.clear();
        assert!(pw.is_empty());
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
