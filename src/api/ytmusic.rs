//! Native YouTube Music (`music.youtube.com` / InnerTube `WEB_REMIX`) HTTP/2 API client.
//! Provides personalized Home shelves, Library, Liked Music, Charts, Albums, Playlists, Artists,
//! filtered Search, Up Next radio/album queues, real-time timed & plain Lyrics, and Likes.

use crate::db::cache::normalize_thumb_url;
use crate::error::{AppError, Result};
use crate::services::auth::{
    compute_sapisidhash, now_unix_secs, GoogleAccountSession, PHAENNA_USER_AGENT,
};
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, USER_AGENT};
use serde_json::Value;
use std::collections::HashSet;

const YTM_ORIGIN: &str = "https://music.youtube.com";
const YTM_CLIENT_NAME: &str = "WEB_REMIX";
const YTM_CLIENT_ID: &str = "67";
const YTM_CLIENT_VERSION: &str = "1.20251015.01.00";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YtmItemKind {
    Song,
    Video,
    Album,
    Playlist,
    Artist,
    Radio,
}

impl YtmItemKind {
    pub fn badge(self) -> &'static str {
        match self {
            Self::Song => "SONG",
            Self::Video => "VIDEO",
            Self::Album => "ALBUM",
            Self::Playlist => "PLAYLIST",
            Self::Artist => "ARTIST",
            Self::Radio => "RADIO",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtmTrack {
    pub video_id: String,
    pub title: String,
    pub artist: String,
    pub artist_browse_id: Option<String>,
    pub album: Option<String>,
    pub album_browse_id: Option<String>,
    pub duration_text: Option<String>,
    pub duration_secs: Option<u64>,
    pub plays_text: Option<String>,
    pub thumb_url: String,
    pub playlist_id: Option<String>,
    pub kind: YtmItemKind,
}

impl YtmTrack {
    /// Distinct thumbnail cache key (`ytm:<video_id>`) so square album art never collides with 16:9 video thumbs.
    pub fn thumb_cache_key(&self) -> String {
        format!("ytm:{}", self.video_id)
    }

    /// Best cover URL for this track, falling back to YouTube high-quality thumbnail when absent.
    pub fn effective_thumb_url(&self) -> String {
        if !self.thumb_url.trim().is_empty() {
            self.thumb_url.clone()
        } else if !self.video_id.trim().is_empty() {
            format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", self.video_id)
        } else {
            String::new()
        }
    }

    #[allow(dead_code)]
    pub fn subtitle_line(&self) -> String {
        let mut parts = Vec::new();
        if !self.artist.trim().is_empty() {
            parts.push(self.artist.clone());
        }
        if let Some(ref alb) = self.album {
            if !alb.trim().is_empty() && !alb.eq_ignore_ascii_case(&self.artist) {
                parts.push(alb.clone());
            }
        }
        if let Some(ref plays) = self.plays_text {
            if !plays.trim().is_empty() {
                parts.push(plays.clone());
            }
        }
        parts.join(" · ")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtmShelfItem {
    pub kind: YtmItemKind,
    pub title: String,
    pub subtitle: String,
    pub video_id: Option<String>,
    pub browse_id: Option<String>,
    pub playlist_id: Option<String>,
    pub params: Option<String>,
    pub thumb_url: String,
    pub duration_secs: Option<u64>,
}

impl YtmShelfItem {
    pub fn thumb_cache_key(&self) -> String {
        if let Some(ref vid) = self.video_id {
            format!("ytm:{vid}")
        } else if let Some(ref bid) = self.browse_id {
            format!("ytm:b:{bid}")
        } else if let Some(ref pid) = self.playlist_id {
            format!("ytm:p:{pid}")
        } else {
            format!("ytm:t:{}", self.title)
        }
    }

    pub fn to_track(&self) -> Option<YtmTrack> {
        let vid = self.video_id.clone()?;
        let mut artist = self.subtitle.clone();
        if let Some((first, _)) = self.subtitle.split_once(" • ") {
            if !first.trim().is_empty() && !first.eq_ignore_ascii_case("song") && !first.eq_ignore_ascii_case("video") {
                artist = first.trim().to_string();
            } else if let Some(second) = self.subtitle.split(" • ").nth(1) {
                artist = second.trim().to_string();
            }
        }
        Some(YtmTrack {
            video_id: vid,
            title: self.title.clone(),
            artist,
            artist_browse_id: None,
            album: None,
            album_browse_id: self.browse_id.clone().filter(|b| b.starts_with("MPREb")),
            duration_text: self.duration_secs.map(fmt_mmss),
            duration_secs: self.duration_secs,
            plays_text: None,
            thumb_url: self.thumb_url.clone(),
            playlist_id: self.playlist_id.clone(),
            kind: if self.kind == YtmItemKind::Video {
                YtmItemKind::Video
            } else {
                YtmItemKind::Song
            },
        })
    }
}

#[derive(Debug, Clone, Default)]
pub struct YtmShelf {
    pub title: String,
    pub items: Vec<YtmShelfItem>,
    pub more_browse_id: Option<String>,
    pub more_params: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct YtmPage {
    pub header_title: Option<String>,
    pub header_subtitle: Option<String>,
    pub header_artist: Option<String>,
    pub header_artist_browse_id: Option<String>,
    pub header_year: Option<String>,
    pub header_track_summary: Option<String>,
    pub header_thumb: Option<String>,
    pub header_playlist_id: Option<String>,
    pub header_radio_playlist_id: Option<String>,
    pub header_subscribers: Option<String>,
    pub header_description: Option<String>,
    pub is_artist_profile: bool,
    pub shelves: Vec<YtmShelf>,
    pub tracks: Vec<YtmTrack>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YtmSearchFilter {
    #[default]
    All,
    Songs,
    Videos,
    Albums,
    Playlists,
    Artists,
}

impl YtmSearchFilter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Songs,
        Self::Videos,
        Self::Albums,
        Self::Playlists,
        Self::Artists,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "Top",
            Self::Songs => "Songs",
            Self::Videos => "Videos",
            Self::Albums => "Albums",
            Self::Playlists => "Playlists",
            Self::Artists => "Artists",
        }
    }

    pub fn params_token(self) -> Option<&'static str> {
        match self {
            Self::All => None,
            Self::Songs => Some("EgWKAQIIAWoKEAMQBBAJEAoQBQ%3D%3D"),
            Self::Videos => Some("EgWKAQIQAWoKEAMQBBAJEAoQBQ%3D%3D"),
            Self::Albums => Some("EgWKAQIYAWoKEAMQBBAJEAoQBQ%3D%3D"),
            Self::Playlists => Some("EgeKAQQoAEABagoQAxAEEAkQChAF"),
            Self::Artists => Some("EgWKAQIgAWoKEAMQBBAJEAoQBQ%3D%3D"),
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Songs,
            Self::Songs => Self::Videos,
            Self::Videos => Self::Albums,
            Self::Albums => Self::Playlists,
            Self::Playlists => Self::Artists,
            Self::Artists => Self::All,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct YtmUpNextQueue {
    pub tracks: Vec<YtmTrack>,
    pub lyrics_browse_id: Option<String>,
    pub related_browse_id: Option<String>,
    pub continuation: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct YtmLyricLine {
    pub text: String,
    pub start_ms: Option<u64>,
    pub end_ms: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct YtmLyrics {
    pub lines: Vec<YtmLyricLine>,
    pub source: Option<String>,
    pub is_timed: bool,
}

fn ytm_context() -> Value {
    serde_json::json!({
        "client": {
            "clientName": YTM_CLIENT_NAME,
            "clientVersion": YTM_CLIENT_VERSION,
            "hl": "en",
            "gl": "US"
        }
    })
}

pub fn apply_ytm_headers(
    mut req: reqwest::RequestBuilder,
    session_opt: Option<&GoogleAccountSession>,
) -> reqwest::RequestBuilder {
    req = req
        .header(USER_AGENT, PHAENNA_USER_AGENT)
        .header(CONTENT_TYPE, "application/json")
        .header("Origin", YTM_ORIGIN)
        .header("X-Origin", YTM_ORIGIN)
        .header("Referer", format!("{YTM_ORIGIN}/"))
        .header("X-YouTube-Client-Name", YTM_CLIENT_ID)
        .header("X-YouTube-Client-Version", YTM_CLIENT_VERSION);

    if let Some(session) = session_opt {
        if let (Some(sapisid), Some(cookie_hdr)) = (session.sapisid(), session.cookie_header()) {
            let hash = compute_sapisidhash(sapisid, YTM_ORIGIN, now_unix_secs());
            req = req
                .header(AUTHORIZATION, format!("SAPISIDHASH {hash}"))
                .header(COOKIE, cookie_hdr);
        } else if let Some(ref token) = session.access_token {
            req = req.header(AUTHORIZATION, format!("Bearer {token}"));
        }
    }
    req
}

async fn post_ytm_json(
    client: &reqwest::Client,
    session_opt: Option<&GoogleAccountSession>,
    endpoint: &str,
    payload: Value,
) -> Result<Value> {
    let url = format!("{YTM_ORIGIN}/youtubei/v1/{endpoint}?prettyPrint=false");
    let req = apply_ytm_headers(client.post(url), session_opt).json(&payload);
    let resp = req
        .send()
        .await
        .map_err(|e| AppError::Http(format!("YTM {endpoint} request failed: {e}")))?;
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "YTM {endpoint} HTTP {}",
            resp.status()
        )));
    }
    resp.json::<Value>()
        .await
        .map_err(|e| AppError::Parse(format!("Invalid YTM {endpoint} JSON: {e}")))
}

fn runs_text(val: Option<&Value>) -> String {
    let Some(v) = val else {
        return String::new();
    };
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return s.trim().to_string();
    }
    if let Some(runs) = v.get("runs").and_then(Value::as_array) {
        let joined: String = runs
            .iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect();
        return joined.trim().to_string();
    }
    String::new()
}

fn upgrade_ytm_thumb_url(raw: &str) -> String {
    let url = normalize_thumb_url(raw);
    if url.contains("googleusercontent.com") || url.contains("yt3.ggpht.com") {
        if let Some(eq_idx) = url.rfind('=') {
            let suffix = &url[eq_idx + 1..];
            if let Some(rest) = suffix.strip_prefix('w') {
                if let Some(dash_h) = rest.find("-h") {
                    let w_str = &rest[..dash_h];
                    let after_h = &rest[dash_h + 2..];
                    let h_len = after_h
                        .bytes()
                        .take_while(|b| b.is_ascii_digit())
                        .count();
                    if let (Ok(w), Ok(_h)) =
                        (w_str.parse::<u32>(), after_h[..h_len].parse::<u32>())
                    {
                        if w > 0 && w < 400 {
                            let tail = &after_h[h_len..];
                            return format!("{}=w544-h544{tail}", &url[..eq_idx]);
                        }
                    }
                }
            }
        }
    }
    url
}

fn best_thumb(val: Option<&Value>) -> String {
    let Some(arr) = val
        .and_then(|t| t.get("thumbnails").or_else(|| t.pointer("/musicThumbnailRenderer/thumbnail/thumbnails")))
        .and_then(Value::as_array)
    else {
        return String::new();
    };
    arr.iter()
        .max_by_key(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|t| t.get("url").and_then(Value::as_str))
        .map(upgrade_ytm_thumb_url)
        .unwrap_or_default()
}

pub fn parse_duration_secs(text: &str) -> Option<u64> {
    let t = text.trim();
    if t.is_empty() || !t.contains(':') {
        return None;
    }
    let parts: Vec<u64> = t
        .split(':')
        .map(|p| p.trim().parse::<u64>())
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;
    match parts.as_slice() {
        [m, s] => Some(m * 60 + s),
        [h, m, s] => Some(h * 3600 + m * 60 + s),
        _ => None,
    }
}

fn fmt_mmss(secs: u64) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn classify_browse_kind(browse_id: &str, page_type: Option<&str>, subtitle: &str) -> YtmItemKind {
    if let Some(pt) = page_type {
        if pt.contains("ALBUM") {
            return YtmItemKind::Album;
        }
        if pt.contains("ARTIST") || pt.contains("USER_CHANNEL") {
            return YtmItemKind::Artist;
        }
        if pt.contains("PLAYLIST") {
            return YtmItemKind::Playlist;
        }
    }
    if browse_id.starts_with("MPREb") || browse_id.starts_with("OLAK") {
        YtmItemKind::Album
    } else if browse_id.starts_with("UC") {
        YtmItemKind::Artist
    } else if browse_id.starts_with("VL") || browse_id.starts_with("FE") {
        YtmItemKind::Playlist
    } else if subtitle.to_ascii_lowercase().contains("album") {
        YtmItemKind::Album
    } else if subtitle.to_ascii_lowercase().contains("artist") {
        YtmItemKind::Artist
    } else {
        YtmItemKind::Playlist
    }
}

fn extract_watch_endpoint(ep: Option<&Value>) -> (Option<String>, Option<String>) {
    let Some(ep) = ep else {
        return (None, None);
    };
    if let Some(we) = ep.get("watchEndpoint") {
        let vid = we
            .get("videoId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let pid = we
            .get("playlistId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        return (vid, pid);
    }
    if let Some(wpe) = ep.get("watchPlaylistEndpoint") {
        let pid = wpe
            .get("playlistId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        return (None, pid);
    }
    (None, None)
}

fn extract_browse_endpoint(ep: Option<&Value>) -> (Option<String>, Option<String>, Option<&str>) {
    let Some(be) = ep.and_then(|e| e.get("browseEndpoint")) else {
        return (None, None, None);
    };
    let bid = be
        .get("browseId")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let params = be
        .get("params")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let page_type = be
        .pointer("/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
        .and_then(Value::as_str);
    (bid, params, page_type)
}

fn extract_continuation(val: &Value) -> Option<String> {
    if let Some(conts) = val.get("continuations").and_then(Value::as_array) {
        for c in conts {
            if let Some(tok) = c
                .pointer("/nextContinuationData/continuation")
                .or_else(|| c.pointer("/nextRadioContinuationData/continuation"))
                .and_then(Value::as_str)
            {
                if !tok.is_empty() {
                    return Some(tok.to_string());
                }
            }
        }
    }
    if let Some(arr) = val.get("contents").and_then(Value::as_array) {
        for item in arr {
            if let Some(tok) = item
                .pointer("/continuationItemRenderer/continuationEndpoint/continuationCommand/token")
                .and_then(Value::as_str)
            {
                if !tok.is_empty() {
                    return Some(tok.to_string());
                }
            }
        }
    }
    None
}

fn parse_two_row_item(r: &Value) -> Option<YtmShelfItem> {
    parse_two_row_item_with_hint(r, None)
}

fn parse_two_row_item_with_hint(r: &Value, shelf_title: Option<&str>) -> Option<YtmShelfItem> {
    let title = runs_text(r.get("title"));
    if title.is_empty() {
        return None;
    }
    let subtitle = runs_text(r.get("subtitle"));
    let thumb_url = best_thumb(r.get("thumbnailRenderer"));

    let nav = r
        .get("navigationEndpoint")
        .or_else(|| r.pointer("/title/runs/0/navigationEndpoint"));
    let play_ep = r.pointer(
        "/thumbnailOverlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint",
    );

    let (mut vid, mut pid) = extract_watch_endpoint(nav);
    if vid.is_none() && pid.is_none() {
        let (ov_vid, ov_pid) = extract_watch_endpoint(play_ep);
        vid = ov_vid;
        pid = ov_pid;
    }
    let (bid, params, page_type) = extract_browse_endpoint(nav);

    let shelf_is_video = shelf_title.is_some_and(|s| s.to_ascii_lowercase().contains("video"));
    let vtype = play_ep
        .or(nav)
        .and_then(|e| {
            e.pointer("/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        })
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = if let Some(ref b) = bid {
        classify_browse_kind(b, page_type, &subtitle)
    } else if vid.is_some() {
        let sub_low = subtitle.to_ascii_lowercase();
        if shelf_is_video
            || sub_low.contains("video")
            || vtype == "MUSIC_VIDEO_TYPE_OMV"
            || vtype == "MUSIC_VIDEO_TYPE_UGC"
        {
            YtmItemKind::Video
        } else {
            YtmItemKind::Song
        }
    } else if pid.is_some() {
        if pid.as_deref().unwrap_or("").starts_with("RD") {
            YtmItemKind::Radio
        } else {
            YtmItemKind::Playlist
        }
    } else {
        return None;
    };

    // Never attach an overlay child track video_id onto Album / Playlist / Artist collection cards,
    // otherwise clicking another album to inspect it would hijack/stop active playback!
    if matches!(
        kind,
        YtmItemKind::Album | YtmItemKind::Playlist | YtmItemKind::Artist
    ) {
        vid = None;
    }

    Some(YtmShelfItem {
        kind,
        title,
        subtitle,
        video_id: vid,
        browse_id: bid,
        playlist_id: pid,
        params,
        thumb_url,
        duration_secs: None,
    })
}

fn parse_responsive_item(r: &Value, default_playlist_id: Option<&str>) -> Option<YtmShelfItem> {
    let cols = r.get("flexColumns").and_then(Value::as_array)?;
    let title_col = cols.first()?.get("musicResponsiveListItemFlexColumnRenderer")?;
    let title = runs_text(title_col.get("text"));
    if title.is_empty() || title.eq_ignore_ascii_case("Shuffle all") {
        return None;
    }

    let mut sub_parts = Vec::new();
    let mut duration_secs: Option<u64> = None;
    let mut artist_bid: Option<String> = None;
    let mut album_bid: Option<String> = None;

    for col in cols.iter().skip(1) {
        let Some(fc) = col.get("musicResponsiveListItemFlexColumnRenderer") else {
            continue;
        };
        let txt = runs_text(fc.get("text"));
        if !txt.is_empty() {
            for seg in txt.split(" • ") {
                if let Some(d) = parse_duration_secs(seg) {
                    duration_secs = Some(d);
                }
            }
            sub_parts.push(txt);
        }
        if let Some(runs) = fc.pointer("/text/runs").and_then(Value::as_array) {
            for run in runs {
                let (bid, _, page_type) = extract_browse_endpoint(run.get("navigationEndpoint"));
                if let (Some(b), Some(pt)) = (bid, page_type) {
                    if pt.contains("ARTIST") && artist_bid.is_none() {
                        artist_bid = Some(b);
                    } else if pt.contains("ALBUM") && album_bid.is_none() {
                        album_bid = Some(b);
                    }
                }
            }
        }
    }

    if duration_secs.is_none() {
        if let Some(fixed) = r.get("fixedColumns").and_then(Value::as_array) {
            for fc in fixed {
                let txt = runs_text(fc.pointer("/musicResponsiveListItemFixedColumnRenderer/text"));
                if let Some(d) = parse_duration_secs(&txt) {
                    duration_secs = Some(d);
                    break;
                }
            }
        }
    }

    let play_ep = r.pointer(
        "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint",
    );
    let nav_ep = r.get("navigationEndpoint");
    let title_nav = title_col.pointer("/text/runs/0/navigationEndpoint");

    let (mut vid, mut pid) = extract_watch_endpoint(play_ep);
    if vid.is_none() {
        let (nv, np) = extract_watch_endpoint(nav_ep.or(title_nav));
        if nv.is_some() {
            vid = nv;
        }
        if pid.is_none() {
            pid = np;
        }
    }
    if vid.is_none() {
        vid = r
            .pointer("/playlistItemData/videoId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    if pid.is_none() {
        pid = default_playlist_id.map(ToOwned::to_owned);
    }

    let (bid, params, page_type) = extract_browse_endpoint(nav_ep.or(title_nav));
    let subtitle = sub_parts.join(" • ");
    let thumb_url = best_thumb(r.get("thumbnail"));

    let nav_is_collection = bid.as_ref().is_some_and(|b| {
        let k = classify_browse_kind(b, page_type, &subtitle);
        matches!(
            k,
            YtmItemKind::Album | YtmItemKind::Playlist | YtmItemKind::Artist
        )
    });

    let kind = if nav_is_collection {
        classify_browse_kind(bid.as_deref().unwrap_or(""), page_type, &subtitle)
    } else if vid.is_some() {
        let vtype = play_ep
            .or(nav_ep)
            .or(title_nav)
            .and_then(|e| e.pointer("/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if vtype == "MUSIC_VIDEO_TYPE_OMV"
            || vtype == "MUSIC_VIDEO_TYPE_UGC"
            || subtitle.to_ascii_lowercase().contains("video")
            || subtitle.to_ascii_lowercase().contains("views")
        {
            YtmItemKind::Video
        } else {
            YtmItemKind::Song
        }
    } else if let Some(ref b) = bid {
        classify_browse_kind(b, page_type, &subtitle)
    } else if pid.is_some() {
        YtmItemKind::Playlist
    } else {
        return None;
    };

    if matches!(
        kind,
        YtmItemKind::Album | YtmItemKind::Playlist | YtmItemKind::Artist
    ) {
        vid = None;
    }

    Some(YtmShelfItem {
        kind,
        title,
        subtitle,
        video_id: vid,
        browse_id: bid.or(album_bid).or(artist_bid),
        playlist_id: pid,
        params,
        thumb_url,
        duration_secs,
    })
}

fn responsive_to_track(r: &Value, fallback_thumb: &str, default_playlist_id: Option<&str>) -> Option<YtmTrack> {
    let cols = r.get("flexColumns").and_then(Value::as_array)?;
    let title_col = cols.first()?.get("musicResponsiveListItemFlexColumnRenderer")?;
    let title = runs_text(title_col.get("text"));
    if title.is_empty() || title.eq_ignore_ascii_case("Shuffle all") {
        return None;
    }

    let play_ep = r.pointer(
        "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint",
    );
    let title_nav = title_col.pointer("/text/runs/0/navigationEndpoint");
    let (mut vid, mut pid) = extract_watch_endpoint(play_ep.or(title_nav).or_else(|| r.get("navigationEndpoint")));
    if vid.is_none() {
        vid = r
            .pointer("/playlistItemData/videoId")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
    }
    let video_id = vid?;
    if pid.is_none() {
        pid = default_playlist_id.map(ToOwned::to_owned);
    }

    let mut artist = String::new();
    let mut artist_browse_id = None;
    let mut album = None;
    let mut album_browse_id = None;
    let mut duration_text = None;
    let mut duration_secs = None;
    let mut plays_text = None;

    for (idx, col) in cols.iter().enumerate().skip(1) {
        let Some(fc) = col.get("musicResponsiveListItemFlexColumnRenderer") else {
            continue;
        };
        if let Some(runs) = fc.pointer("/text/runs").and_then(Value::as_array) {
            for run in runs {
                let txt = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
                if txt.is_empty() || txt == "•" || txt == "," || txt == "&" {
                    continue;
                }
                let (bid, _, pt) = extract_browse_endpoint(run.get("navigationEndpoint"));
                if let Some(pt_s) = pt {
                    if pt_s.contains("ARTIST") {
                        if artist.is_empty() {
                            artist = txt.to_string();
                            artist_browse_id = bid;
                        }
                        continue;
                    } else if pt_s.contains("ALBUM") {
                        if album.is_none() {
                            album = Some(txt.to_string());
                            album_browse_id = bid;
                        }
                        continue;
                    }
                }
                if let Some(d) = parse_duration_secs(txt) {
                    duration_secs = Some(d);
                    duration_text = Some(txt.to_string());
                } else if txt.to_ascii_lowercase().contains("play") || txt.to_ascii_lowercase().contains("view") {
                    plays_text = Some(txt.to_string());
                } else if idx == 1 && artist.is_empty() && !txt.eq_ignore_ascii_case("song") && !txt.eq_ignore_ascii_case("video") {
                    artist = txt.to_string();
                } else if idx == 1 && !artist.is_empty() && album.is_none() && !txt.eq_ignore_ascii_case("song") && !txt.eq_ignore_ascii_case("video") {
                    album = Some(txt.to_string());
                }
            }
        }
    }

    if duration_secs.is_none() {
        if let Some(fixed) = r.get("fixedColumns").and_then(Value::as_array) {
            for fc in fixed {
                let txt = runs_text(fc.pointer("/musicResponsiveListItemFixedColumnRenderer/text"));
                if let Some(d) = parse_duration_secs(&txt) {
                    duration_secs = Some(d);
                    duration_text = Some(txt);
                    break;
                }
            }
        }
    }

    let mut thumb_url = best_thumb(r.get("thumbnail"));
    if thumb_url.is_empty() {
        if !fallback_thumb.is_empty() {
            thumb_url = fallback_thumb.to_string();
        } else if !video_id.is_empty() {
            thumb_url = format!("https://i.ytimg.com/vi/{video_id}/hqdefault.jpg");
        }
    }

    let vtype = play_ep
        .or(title_nav)
        .and_then(|e| e.pointer("/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let kind = if vtype == "MUSIC_VIDEO_TYPE_OMV" || vtype == "MUSIC_VIDEO_TYPE_UGC" {
        YtmItemKind::Video
    } else {
        YtmItemKind::Song
    };

    Some(YtmTrack {
        video_id,
        title,
        artist,
        artist_browse_id,
        album,
        album_browse_id,
        duration_text,
        duration_secs,
        plays_text,
        thumb_url,
        playlist_id: pid,
        kind,
    })
}

pub fn parse_ytm_browse_json(root: &Value) -> YtmPage {
    let mut page = YtmPage::default();

    // Header (albums, personal/editable playlists, immersive/visual artist headers)
    for hdr_ptr in [
        "/header/musicDetailHeaderRenderer",
        "/header/musicEditablePlaylistDetailHeaderRenderer/header/musicDetailHeaderRenderer",
        "/header/musicEditablePlaylistDetailHeaderRenderer/header/musicResponsiveHeaderRenderer",
        "/header/musicImmersiveHeaderRenderer",
        "/header/musicVisualHeaderRenderer",
        "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer",
        "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicEditablePlaylistDetailHeaderRenderer/header/musicResponsiveHeaderRenderer",
        "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicEditablePlaylistDetailHeaderRenderer/header/musicDetailHeaderRenderer",
    ] {
        if let Some(hdr) = root.pointer(hdr_ptr) {
            if hdr_ptr.contains("ImmersiveHeaderRenderer")
                || hdr_ptr.contains("VisualHeaderRenderer")
                || hdr.get("subscriptionButton").is_some()
                || hdr.get("startRadioButton").is_some()
            {
                page.is_artist_profile = true;
            }
            let t = runs_text(hdr.get("title"));
            if !t.is_empty() {
                page.header_title = Some(t);
            }
            let sub = runs_text(hdr.get("subtitle").or_else(|| hdr.get("straplineTextOne")));
            if !sub.is_empty() {
                page.header_subtitle = Some(sub.clone());
                for part in sub.split("•").map(str::trim) {
                    if part.len() == 4 && part.chars().all(|c| c.is_ascii_digit()) {
                        page.header_year = Some(part.to_string());
                    }
                }
            }
            if let Some(runs) = hdr
                .pointer("/straplineTextOne/runs")
                .or_else(|| hdr.pointer("/subtitle/runs"))
                .and_then(Value::as_array)
            {
                for r in runs {
                    let rt = r.get("text").and_then(Value::as_str).unwrap_or("").trim();
                    let (bid, _, pt) = extract_browse_endpoint(r.get("navigationEndpoint"));
                    if bid.is_some()
                        && (pt.unwrap_or("").contains("ARTIST")
                            || bid.as_deref().unwrap_or("").starts_with("UC"))
                    {
                        page.header_artist = (!rt.is_empty()).then(|| rt.to_string());
                        page.header_artist_browse_id = bid;
                        break;
                    }
                }
            }
            if page.header_artist.is_none() {
                let strap = runs_text(hdr.get("straplineTextOne"));
                if !strap.is_empty() {
                    page.header_artist = Some(strap);
                }
            }
            let summary = runs_text(hdr.get("secondSubtitle"));
            if !summary.is_empty() {
                page.header_track_summary = Some(summary);
            }
            let desc = runs_text(hdr.get("description"));
            if !desc.is_empty() {
                page.header_description = Some(desc.clone());
                if page.header_track_summary.is_none() {
                    page.header_track_summary = Some(desc);
                }
            }
            let subs = runs_text(
                hdr.pointer("/subscriptionButton/subscribeButtonRenderer/longSubscriberCountText")
                    .or_else(|| {
                        hdr.pointer(
                            "/subscriptionButton/subscribeButtonRenderer/subscriberCountText",
                        )
                    })
                    .or_else(|| {
                        hdr.pointer(
                            "/subscriptionButton/subscribeButtonRenderer/shortSubscriberCountText",
                        )
                    }),
            );
            if !subs.is_empty() {
                page.header_subscribers = Some(subs);
            }
            let th = best_thumb(hdr.get("thumbnail"));
            if !th.is_empty() {
                page.header_thumb = Some(th);
            }
            let (_, play_pid) = extract_watch_endpoint(
                hdr.pointer("/playButton/buttonRenderer/navigationEndpoint"),
            );
            if let Some(pid) = play_pid {
                page.header_playlist_id = Some(pid);
            }
            let (_, radio_pid) = extract_watch_endpoint(
                hdr.pointer("/startRadioButton/buttonRenderer/navigationEndpoint"),
            );
            if let Some(rpid) = radio_pid {
                page.header_radio_playlist_id = Some(rpid);
            }
            if let Some(btns) = hdr.get("buttons").and_then(Value::as_array) {
                for b in btns {
                    let ep = b
                        .pointer("/musicPlayButtonRenderer/playNavigationEndpoint")
                        .or_else(|| b.pointer("/buttonRenderer/navigationEndpoint"));
                    let (_, pid) = extract_watch_endpoint(ep);
                    if let Some(p) = pid {
                        if p.starts_with("RD") {
                            if page.header_radio_playlist_id.is_none() {
                                page.header_radio_playlist_id = Some(p);
                            }
                        } else if page.header_playlist_id.is_none() {
                            page.header_playlist_id = Some(p);
                        }
                    }
                }
            }
            break;
        }
    }

    let fallback_thumb = page.header_thumb.as_deref().unwrap_or("");
    let default_pid = page.header_playlist_id.as_deref();

    // Locate main section list across singleColumn, twoColumn (album/playlist secondary), or continuation
    let section_opt = root
        .pointer("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer")
        .or_else(|| {
            root.pointer(
                "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer",
            )
        })
        .or_else(|| root.pointer("/continuationContents/sectionListContinuation"));

    if let Some(sec) = section_opt {
        if page.continuation.is_none() {
            page.continuation = extract_continuation(sec);
        }
        if let Some(contents) = sec.get("contents").and_then(Value::as_array) {
            for block in contents {
                if let Some(car) = block.get("musicCarouselShelfRenderer") {
                    let title = runs_text(
                        car.pointer("/header/musicCarouselShelfBasicHeaderRenderer/title"),
                    );
                    let mut items = Vec::new();
                    if let Some(arr) = car.get("contents").and_then(Value::as_array) {
                        for entry in arr {
                            if let Some(two) = entry.get("musicTwoRowItemRenderer") {
                                if let Some(it) = parse_two_row_item_with_hint(two, Some(&title)) {
                                    items.push(it);
                                }
                            } else if let Some(resp) = entry.get("musicResponsiveListItemRenderer") {
                                if let Some(it) = parse_responsive_item(resp, default_pid) {
                                    if let Some(tr) = responsive_to_track(resp, fallback_thumb, default_pid) {
                                        page.tracks.push(tr);
                                    }
                                    items.push(it);
                                }
                            }
                        }
                    }
                    if !items.is_empty() {
                        let (more_bid, more_p, _) = extract_browse_endpoint(
                            car.pointer("/header/musicCarouselShelfBasicHeaderRenderer/moreContentButton/buttonRenderer/navigationEndpoint"),
                        );
                        page.shelves.push(YtmShelf {
                            title: if title.is_empty() {
                                "For You".to_string()
                            } else {
                                title
                            },
                            items,
                            more_browse_id: more_bid,
                            more_params: more_p,
                        });
                    }
                } else if let Some(ms) = block
                    .get("musicShelfRenderer")
                    .or_else(|| block.get("musicPlaylistShelfRenderer"))
                {
                    if page.continuation.is_none() {
                        page.continuation = extract_continuation(ms);
                    }
                    let title = runs_text(ms.get("title"));
                    let mut items = Vec::new();
                    if let Some(arr) = ms.get("contents").and_then(Value::as_array) {
                        for entry in arr {
                            if let Some(resp) = entry.get("musicResponsiveListItemRenderer") {
                                if let Some(tr) = responsive_to_track(resp, fallback_thumb, default_pid) {
                                    page.tracks.push(tr);
                                }
                                if let Some(it) = parse_responsive_item(resp, default_pid) {
                                    items.push(it);
                                }
                            }
                        }
                    }
                    if !items.is_empty() {
                        let (more_bid, more_p, _) =
                            extract_browse_endpoint(ms.get("bottomEndpoint"));
                        page.shelves.push(YtmShelf {
                            title: if title.is_empty() {
                                page.header_title
                                    .clone()
                                    .unwrap_or_else(|| "Tracks".to_string())
                            } else {
                                title
                            },
                            items,
                            more_browse_id: more_bid,
                            more_params: more_p,
                        });
                    }
                } else if let Some(desc_blk) = block.get("musicDescriptionShelfRenderer") {
                    let bio = runs_text(desc_blk.get("description"));
                    if !bio.is_empty() {
                        page.is_artist_profile = true;
                        match page.header_description {
                            Some(ref existing) if !existing.contains(&bio) => {
                                page.header_description = Some(format!("{existing}\n{bio}"));
                            }
                            None => page.header_description = Some(bio),
                            _ => {}
                        }
                    }
                    let sub_info = runs_text(desc_blk.get("subheader"));
                    if !sub_info.is_empty() && page.header_subscribers.is_none() {
                        page.header_subscribers = Some(sub_info);
                    }
                } else if let Some(grid) = block.get("gridRenderer") {
                    if page.continuation.is_none() {
                        page.continuation = extract_continuation(grid);
                    }
                    let mut items = Vec::new();
                    if let Some(arr) = grid.get("items").and_then(Value::as_array) {
                        for entry in arr {
                            if let Some(two) = entry.get("musicTwoRowItemRenderer") {
                                if let Some(it) = parse_two_row_item(two) {
                                    items.push(it);
                                }
                            }
                        }
                    }
                    if !items.is_empty() {
                        page.shelves.push(YtmShelf {
                            title: "Your Library".to_string(),
                            items,
                            more_browse_id: None,
                            more_params: None,
                        });
                    }
                }
            }
        }
    }

    // Continuation append for playlists/liked songs
    if let Some(cont_ms) = root
        .pointer("/continuationContents/musicPlaylistShelfContinuation")
        .or_else(|| root.pointer("/continuationContents/musicShelfContinuation"))
    {
        page.continuation = extract_continuation(cont_ms);
        if let Some(arr) = cont_ms.get("contents").and_then(Value::as_array) {
            let mut items = Vec::new();
            for entry in arr {
                if let Some(resp) = entry.get("musicResponsiveListItemRenderer") {
                    if let Some(tr) = responsive_to_track(resp, fallback_thumb, default_pid) {
                        page.tracks.push(tr);
                    }
                    if let Some(it) = parse_responsive_item(resp, default_pid) {
                        items.push(it);
                    }
                }
            }
            if !items.is_empty() {
                page.shelves.push(YtmShelf {
                    title: "More Tracks".to_string(),
                    items,
                    more_browse_id: None,
                    more_params: None,
                });
            }
        }
    }

    // Inherit album / artist header metadata for tracks inside Album / Artist popups
    let fallback_artist = if page.is_artist_profile {
        page.header_title.as_deref()
    } else {
        page.header_artist.as_deref()
    };
    let is_album_or_ep = !page.is_artist_profile
        && (page.header_year.is_some()
            || page.header_subtitle.as_deref().is_some_and(|s| {
                let low = s.to_ascii_lowercase();
                low.starts_with("album") || low.starts_with("ep") || low.starts_with("single")
            }));
    for tr in &mut page.tracks {
        if tr.artist.trim().is_empty() {
            if let Some(art) = fallback_artist {
                tr.artist = art.to_string();
                if tr.artist_browse_id.is_none() {
                    tr.artist_browse_id = page.header_artist_browse_id.clone();
                }
            }
        }
        if is_album_or_ep && tr.album.is_none() {
            tr.album = page.header_title.clone();
        }
    }

    page
}

pub fn parse_ytm_search_json(root: &Value) -> YtmPage {
    let mut page = YtmPage::default();
    let sec_opt = root
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer")
        .or_else(|| root.pointer("/continuationContents/musicShelfContinuation"));

    let Some(sec) = sec_opt else {
        return page;
    };

    if let Some(arr) = sec.get("contents").and_then(Value::as_array) {
        if sec.get("sectionListRenderer").is_none() && arr.first().and_then(|v| v.get("musicResponsiveListItemRenderer")).is_some() {
            // Direct continuation items from musicShelfContinuation
            page.continuation = extract_continuation(sec);
            let mut items = Vec::new();
            for entry in arr {
                if let Some(resp) = entry.get("musicResponsiveListItemRenderer") {
                    if let Some(tr) = responsive_to_track(resp, "", None) {
                        page.tracks.push(tr);
                    }
                    if let Some(it) = parse_responsive_item(resp, None) {
                        items.push(it);
                    }
                }
            }
            if !items.is_empty() {
                page.shelves.push(YtmShelf {
                    title: "Results".to_string(),
                    items,
                    ..Default::default()
                });
            }
            return page;
        }

        for block in arr {
            if let Some(card) = block.get("musicCardShelfRenderer") {
                let title = runs_text(card.get("title"));
                let subtitle = runs_text(card.get("subtitle"));
                let thumb_url = best_thumb(card.get("thumbnail"));
                let on_tap = card.get("onTap");
                let (vid, pid) = extract_watch_endpoint(on_tap);
                let (bid, params, pt) = extract_browse_endpoint(on_tap);
                if !title.is_empty() && (vid.is_some() || bid.is_some()) {
                    let kind = if let Some(ref b) = bid {
                        classify_browse_kind(b, pt, &subtitle)
                    } else {
                        YtmItemKind::Song
                    };
                    let keep_vid = if matches!(
                        kind,
                        YtmItemKind::Album | YtmItemKind::Playlist | YtmItemKind::Artist
                    ) {
                        None
                    } else {
                        vid
                    };
                    let it = YtmShelfItem {
                        kind,
                        title: title.clone(),
                        subtitle: subtitle.clone(),
                        video_id: keep_vid,
                        browse_id: bid,
                        playlist_id: pid.clone(),
                        params,
                        thumb_url: thumb_url.clone(),
                        duration_secs: None,
                    };
                    if let Some(tr) = it.to_track() {
                        page.tracks.push(tr);
                    }
                    page.shelves.push(YtmShelf {
                        title: "Top result".to_string(),
                        items: vec![it],
                        ..Default::default()
                    });
                }
            } else if let Some(ms) = block.get("musicShelfRenderer") {
                if page.continuation.is_none() {
                    page.continuation = extract_continuation(ms);
                }
                let title = runs_text(ms.get("title"));
                let is_video_shelf = title.to_ascii_lowercase().contains("video");
                let mut items = Vec::new();
                if let Some(list) = ms.get("contents").and_then(Value::as_array) {
                    for entry in list {
                        if let Some(resp) = entry.get("musicResponsiveListItemRenderer") {
                            if let Some(mut tr) = responsive_to_track(resp, "", None) {
                                if is_video_shelf {
                                    tr.kind = YtmItemKind::Video;
                                }
                                page.tracks.push(tr);
                            }
                            if let Some(mut it) = parse_responsive_item(resp, None) {
                                if is_video_shelf
                                    && matches!(it.kind, YtmItemKind::Song | YtmItemKind::Video)
                                {
                                    it.kind = YtmItemKind::Video;
                                }
                                items.push(it);
                            }
                        }
                    }
                }
                if !items.is_empty() {
                    page.shelves.push(YtmShelf {
                        title: if title.is_empty() {
                            "Results".to_string()
                        } else {
                            title
                        },
                        items,
                        ..Default::default()
                    });
                }
            }
        }
    }
    page
}

fn parse_playlist_panel_video(r: &Value) -> Option<YtmTrack> {
    let vid = r.get("videoId").and_then(Value::as_str)?.to_string();
    let title = runs_text(r.get("title"));
    if title.is_empty() {
        return None;
    }

    let mut artist = String::new();
    let mut artist_browse_id = None;
    let mut album = None;
    let mut album_browse_id = None;
    if let Some(runs) = r
        .pointer("/longBylineText/runs")
        .or_else(|| r.pointer("/shortBylineText/runs"))
        .and_then(Value::as_array)
    {
        for run in runs {
            let txt = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
            if txt.is_empty() || txt == "•" {
                continue;
            }
            let (bid, _, pt) = extract_browse_endpoint(run.get("navigationEndpoint"));
            if let Some(pt_s) = pt {
                if pt_s.contains("ARTIST") && artist.is_empty() {
                    artist = txt.to_string();
                    artist_browse_id = bid;
                    continue;
                } else if pt_s.contains("ALBUM") && album.is_none() {
                    album = Some(txt.to_string());
                    album_browse_id = bid;
                    continue;
                }
            }
            if artist.is_empty() && !txt.contains("view") && !txt.contains("like") {
                artist = txt.to_string();
            }
        }
    }

    let duration_text = {
        let s = runs_text(r.get("lengthText"));
        (!s.is_empty()).then_some(s)
    };
    let duration_secs = duration_text.as_deref().and_then(parse_duration_secs);
    let thumb_url = best_thumb(r.get("thumbnail"));
    let (_, pid) = extract_watch_endpoint(r.get("navigationEndpoint"));

    Some(YtmTrack {
        video_id: vid,
        title,
        artist,
        artist_browse_id,
        album,
        album_browse_id,
        duration_text,
        duration_secs,
        plays_text: None,
        thumb_url,
        playlist_id: pid,
        kind: YtmItemKind::Song,
    })
}

pub fn parse_ytm_next_json(root: &Value) -> YtmUpNextQueue {
    let mut q = YtmUpNextQueue::default();

    if let Some(tabs) = root
        .pointer("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs")
        .and_then(Value::as_array)
    {
        for tab in tabs {
            let Some(tr) = tab.get("tabRenderer") else {
                continue;
            };
            let title = tr.get("title").and_then(Value::as_str).unwrap_or("");
            let (bid, _, pt) = extract_browse_endpoint(tr.get("endpoint"));
            if let Some(b) = bid {
                if pt.unwrap_or("").contains("LYRICS") || title.eq_ignore_ascii_case("Lyrics") {
                    q.lyrics_browse_id = Some(b);
                } else if pt.unwrap_or("").contains("RELATED") || title.eq_ignore_ascii_case("Related") {
                    q.related_browse_id = Some(b);
                }
            }
            if let Some(panel) = tr.pointer("/content/musicQueueRenderer/content/playlistPanelRenderer") {
                q.continuation = extract_continuation(panel);
                if let Some(arr) = panel.get("contents").and_then(Value::as_array) {
                    let mut seen = HashSet::new();
                    for item in arr {
                        let candidate = item
                            .pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer")
                            .or_else(|| item.get("playlistPanelVideoRenderer"));
                        if let Some(pv) = candidate {
                            if let Some(track) = parse_playlist_panel_video(pv) {
                                if seen.insert(track.video_id.clone()) {
                                    q.tracks.push(track);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(panel) = root.pointer("/continuationContents/playlistPanelContinuation") {
        q.continuation = extract_continuation(panel);
        if let Some(arr) = panel.get("contents").and_then(Value::as_array) {
            for item in arr {
                let candidate = item
                    .pointer("/playlistPanelVideoWrapperRenderer/primaryRenderer/playlistPanelVideoRenderer")
                    .or_else(|| item.get("playlistPanelVideoRenderer"));
                if let Some(pv) = candidate {
                    if let Some(track) = parse_playlist_panel_video(pv) {
                        q.tracks.push(track);
                    }
                }
            }
        }
    }

    q
}

pub fn parse_ytm_lyrics_json(root: &Value) -> YtmLyrics {
    let mut out = YtmLyrics::default();

    fn walk_lyrics(val: &Value, out: &mut YtmLyrics) {
        match val {
            Value::Object(map) => {
                if let Some(timed) = map.get("musicTimedLyricsRenderer") {
                    if let Some(arr) = timed.get("timedLyricsData").and_then(Value::as_array) {
                        out.is_timed = true;
                        for entry in arr {
                            let text = entry
                                .get("lyricLine")
                                .and_then(Value::as_str)
                                .unwrap_or("")
                                .trim()
                                .to_string();
                            let start_ms = entry
                                .pointer("/cueRange/startTimeMilliseconds")
                                .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64()));
                            let end_ms = entry
                                .pointer("/cueRange/endTimeMilliseconds")
                                .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_u64()));
                            out.lines.push(YtmLyricLine {
                                text,
                                start_ms,
                                end_ms,
                            });
                        }
                    }
                    let src = timed
                        .get("sourceMessage")
                        .and_then(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .unwrap_or_else(|| runs_text(timed.get("sourceMessage")));
                    if !src.is_empty() {
                        out.source = Some(src);
                    }
                    return;
                }
                if let Some(desc) = map.get("musicDescriptionShelfRenderer") {
                    let text = runs_text(desc.get("description"));
                    if !text.is_empty() && out.lines.is_empty() {
                        for line in text.lines() {
                            out.lines.push(YtmLyricLine {
                                text: line.to_string(),
                                start_ms: None,
                                end_ms: None,
                            });
                        }
                    }
                    let src = runs_text(desc.get("footer"));
                    if !src.is_empty() && out.source.is_none() {
                        out.source = Some(src);
                    }
                    return;
                }
                for v in map.values() {
                    walk_lyrics(v, out);
                }
            }
            Value::Array(arr) => {
                for v in arr {
                    walk_lyrics(v, out);
                }
            }
            _ => {}
        }
    }

    walk_lyrics(root, &mut out);
    out
}

pub async fn fetch_ytm_home(
    client: &reqwest::Client,
    session_opt: Option<&GoogleAccountSession>,
    continuation: Option<&str>,
) -> Result<YtmPage> {
    fetch_ytm_browse(client, session_opt, "FEmusic_home", None, continuation).await
}

pub async fn fetch_ytm_browse(
    client: &reqwest::Client,
    session_opt: Option<&GoogleAccountSession>,
    browse_id: &str,
    params: Option<&str>,
    continuation: Option<&str>,
) -> Result<YtmPage> {
    let mut payload = serde_json::json!({ "context": ytm_context() });
    if let Some(cont) = continuation.filter(|c| !c.is_empty()) {
        payload["continuation"] = Value::String(cont.to_string());
    } else {
        payload["browseId"] = Value::String(browse_id.to_string());
        if let Some(p) = params.filter(|s| !s.is_empty()) {
            payload["params"] = Value::String(p.to_string());
        }
    }
    let val = post_ytm_json(client, session_opt, "browse", payload).await?;
    Ok(parse_ytm_browse_json(&val))
}

pub async fn fetch_ytm_search(
    client: &reqwest::Client,
    session_opt: Option<&GoogleAccountSession>,
    query: &str,
    filter: YtmSearchFilter,
    continuation: Option<&str>,
) -> Result<YtmPage> {
    let mut payload = serde_json::json!({ "context": ytm_context() });
    if let Some(cont) = continuation.filter(|c| !c.is_empty()) {
        payload["continuation"] = Value::String(cont.to_string());
    } else {
        payload["query"] = Value::String(query.to_string());
        if let Some(p) = filter.params_token() {
            payload["params"] = Value::String(p.to_string());
        }
    }
    let val = post_ytm_json(client, session_opt, "search", payload).await?;
    Ok(parse_ytm_search_json(&val))
}

pub async fn fetch_ytm_up_next(
    client: &reqwest::Client,
    session_opt: Option<&GoogleAccountSession>,
    video_id: &str,
    playlist_id: Option<&str>,
    continuation: Option<&str>,
) -> Result<YtmUpNextQueue> {
    let clean_vid = video_id.trim_start_matches("RD");
    let pid = playlist_id
        .filter(|p| !p.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| format!("RDAMVM{clean_vid}"));
    let mut payload = serde_json::json!({
        "context": ytm_context(),
        "videoId": clean_vid,
        "playlistId": pid,
        "isAudioOnly": true
    });
    if let Some(cont) = continuation.filter(|c| !c.is_empty()) {
        payload["continuation"] = Value::String(cont.to_string());
    }
    let val = post_ytm_json(client, session_opt, "next", payload).await?;
    Ok(parse_ytm_next_json(&val))
}

pub async fn fetch_ytm_lyrics(
    client: &reqwest::Client,
    session_opt: Option<&GoogleAccountSession>,
    lyrics_browse_id: &str,
) -> Result<YtmLyrics> {
    let payload = serde_json::json!({
        "context": ytm_context(),
        "browseId": lyrics_browse_id,
    });
    let val = post_ytm_json(client, session_opt, "browse", payload).await?;
    Ok(parse_ytm_lyrics_json(&val))
}

fn parse_lrc_timestamp_ms(tag: &str) -> Option<u64> {
    let parts: Vec<&str> = tag.split(':').collect();
    let (mins, secs_part) = match parts.as_slice() {
        [m, s] => (m.trim().parse::<u64>().ok()?, *s),
        [h, m, s] => {
            let hh = h.trim().parse::<u64>().ok()?;
            let mm = m.trim().parse::<u64>().ok()?;
            (hh * 60 + mm, *s)
        }
        _ => return None,
    };
    let (sec_str, frac_str) = secs_part.split_once('.').unwrap_or((secs_part, "0"));
    let secs = sec_str.trim().parse::<u64>().ok()?;
    let frac_digits: String = frac_str
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .take(3)
        .collect();
    let ms = match frac_digits.len() {
        0 => 0,
        1 => frac_digits.parse::<u64>().ok()? * 100,
        2 => frac_digits.parse::<u64>().ok()? * 10,
        _ => frac_digits.parse::<u64>().ok()?,
    };
    Some((mins * 60 + secs) * 1000 + ms)
}

/// Parse standard `[mm:ss.xx]` synced LRC text into millisecond-accurate `YtmLyrics`.
pub fn parse_lrc_lyrics(lrc_text: &str, source: Option<String>) -> YtmLyrics {
    let mut offset_ms: i64 = 0;
    let mut raw_entries: Vec<(u64, String)> = Vec::new();

    for raw_line in lrc_text.lines() {
        let mut rest = raw_line.trim();
        if rest.is_empty() {
            continue;
        }
        let mut timestamps: Vec<u64> = Vec::new();
        while rest.starts_with('[') {
            let Some(close_idx) = rest.find(']') else {
                break;
            };
            let tag = &rest[1..close_idx];
            if let Some(off_str) = tag.strip_prefix("offset:") {
                if let Ok(v) = off_str.trim().parse::<i64>() {
                    offset_ms = v;
                }
                rest = rest[close_idx + 1..].trim();
                continue;
            }
            if let Some(ms) = parse_lrc_timestamp_ms(tag) {
                timestamps.push(ms);
                rest = rest[close_idx + 1..].trim();
            } else {
                // Metadata tag such as [ar:...], [ti:...], [al:...]
                rest = "";
                break;
            }
        }
        if timestamps.is_empty() {
            continue;
        }
        for ts in timestamps {
            let adj = (ts as i64 - offset_ms).max(0) as u64;
            raw_entries.push((adj, rest.to_string()));
        }
    }

    raw_entries.sort_by_key(|(ts, _)| *ts);

    let mut lines: Vec<YtmLyricLine> = Vec::new();
    for i in 0..raw_entries.len() {
        let (start_ms, ref text) = raw_entries[i];
        if text.trim().is_empty() {
            // Blank timestamp marks an instrumental break / end of previous sung line
            if let Some(prev) = lines.last_mut() {
                if prev.end_ms.is_none() && start_ms > prev.start_ms.unwrap_or(0) {
                    prev.end_ms = Some(start_ms);
                }
            }
            continue;
        }
        let next_ts = raw_entries.get(i + 1).map(|(t, _)| *t);
        let end_ms = next_ts
            .filter(|&nt| nt > start_ms)
            .unwrap_or_else(|| start_ms.saturating_add(4500));
        lines.push(YtmLyricLine {
            text: text.clone(),
            start_ms: Some(start_ms),
            end_ms: Some(end_ms),
        });
    }

    let is_timed = !lines.is_empty();
    YtmLyrics {
        lines,
        source: if is_timed { source } else { None },
        is_timed,
    }
}

fn clean_track_title_for_lyrics(raw: &str) -> String {
    let mut s = raw.trim().to_string();
    for (open, close) in [('(', ')'), ('[', ']')] {
        loop {
            let Some(i) = s.rfind(open) else { break };
            let Some(j_rel) = s[i..].find(close) else { break };
            let inner = s[i + 1..i + j_rel].to_ascii_lowercase();
            let strip = inner.contains("official")
                || inner.contains("lyric")
                || inner.contains("audio")
                || inner.contains("video")
                || inner.contains("visualizer")
                || inner.contains("remaster")
                || inner.contains("feat.")
                || inner.contains("ft.");
            if strip {
                s.replace_range(i..=i + j_rel, "");
                s = s.trim().to_string();
            } else {
                break;
            }
        }
    }
    s
}

/// Fetch real-time millisecond-synced LRC lyrics from `lrclib.net` as fallback when YouTube Music
/// only provides plain unsynced lyrics.
pub async fn fetch_lrclib_synced_lyrics(
    client: &reqwest::Client,
    title: &str,
    artist: &str,
    album: Option<&str>,
    duration_secs: Option<u64>,
) -> Option<YtmLyrics> {
    let clean_title = clean_track_title_for_lyrics(title);
    let clean_artist = artist
        .split([',', '&', '•'])
        .next()
        .map(str::trim)
        .filter(|a| !a.is_empty() && !a.eq_ignore_ascii_case("YouTube Music"))
        .unwrap_or("");
    if clean_title.is_empty() {
        return None;
    }

    if !clean_artist.is_empty() {
        let mut params = vec![
            ("track_name", clean_title.clone()),
            ("artist_name", clean_artist.to_string()),
        ];
        if let Some(alb) = album.map(str::trim).filter(|a| !a.is_empty()) {
            params.push(("album_name", alb.to_string()));
        }
        if let Some(dur) = duration_secs.filter(|&d| d > 0) {
            params.push(("duration", dur.to_string()));
        }
        if let Ok(resp) = client
            .get("https://lrclib.net/api/get")
            .header(USER_AGENT, "Phaenna/1.0")
            .query(&params)
            .timeout(std::time::Duration::from_secs(6))
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(val) = resp.json::<Value>().await {
                    if let Some(synced) = val
                        .get("syncedLyrics")
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty())
                    {
                        let parsed =
                            parse_lrc_lyrics(synced, Some("Source: LRCLIB (Synced)".to_string()));
                        if parsed.is_timed {
                            return Some(parsed);
                        }
                    }
                }
            }
        }
    }

    // Fallback search endpoint when exact album/duration match didn't hit
    let search_params: Vec<(&str, String)> = if !clean_artist.is_empty() {
        vec![
            ("track_name", clean_title.clone()),
            ("artist_name", clean_artist.to_string()),
        ]
    } else {
        vec![("q", clean_title.clone())]
    };
    let resp = client
        .get("https://lrclib.net/api/search")
        .header(USER_AGENT, "Phaenna/1.0")
        .query(&search_params)
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let arr = resp.json::<Value>().await.ok()?;
    let items = arr.as_array()?;
    let mut best: Option<(u64, &str)> = None;
    for entry in items {
        let Some(synced) = entry
            .get("syncedLyrics")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        else {
            continue;
        };
        let entry_dur = entry.get("duration").and_then(Value::as_f64).map(|d| d.round() as u64);
        let diff = match (duration_secs, entry_dur) {
            (Some(target), Some(cand)) => target.abs_diff(cand),
            _ => 0,
        };
        if duration_secs.is_some() && entry_dur.is_some() && diff > 12 {
            continue;
        }
        if best.is_none_or(|(b_diff, _)| diff < b_diff) {
            best = Some((diff, synced));
        }
    }
    let (_, best_lrc) = best?;
    let parsed = parse_lrc_lyrics(best_lrc, Some("Source: LRCLIB (Synced)".to_string()));
    parsed.is_timed.then_some(parsed)
}

pub async fn submit_ytm_like(
    client: &reqwest::Client,
    session: &GoogleAccountSession,
    video_id: &str,
    like: bool,
) -> Result<()> {
    let endpoint = if like { "like/like" } else { "like/removelike" };
    let payload = serde_json::json!({
        "context": ytm_context(),
        "target": { "videoId": video_id.trim_start_matches("RD") }
    });
    let _ = post_ytm_json(client, Some(session), endpoint, payload).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ytm_home_library_and_search() {
        let home_json = serde_json::json!({
            "contents": {
                "singleColumnBrowseResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicCarouselShelfRenderer": {
                                            "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [{"text": "Listen again"}] } } },
                                            "contents": [{
                                                "musicTwoRowItemRenderer": {
                                                    "title": { "runs": [{ "text": "Midnight City", "navigationEndpoint": { "watchEndpoint": { "videoId": "dX3k_QDnzHE", "playlistId": "RDAMVMdX3k_QDnzHE" } } }] },
                                                    "subtitle": { "runs": [{ "text": "Song • M83" }] },
                                                    "thumbnailRenderer": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/art=w544-h544", "width": 544, "height": 544 }] } } }
                                                }
                                            }, {
                                                "musicTwoRowItemRenderer": {
                                                    "title": { "runs": [{ "text": "Hurry Up, We're Dreaming", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_12345", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" } } } } }] },
                                                    "subtitle": { "runs": [{ "text": "Album • M83" }] },
                                                    "thumbnailRenderer": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/alb=w544-h544", "width": 544, "height": 544 }] } } }
                                                }
                                            }]
                                        }
                                    }],
                                    "continuations": [{ "nextContinuationData": { "continuation": "tok123" } }]
                                }
                            }
                        }
                    }]
                }
            }
        });

        let home = parse_ytm_browse_json(&home_json);
        assert_eq!(home.shelves.len(), 1);
        assert_eq!(home.shelves[0].title, "Listen again");
        assert_eq!(home.shelves[0].items.len(), 2);
        assert_eq!(home.shelves[0].items[0].kind, YtmItemKind::Song);
        assert_eq!(home.shelves[0].items[0].video_id.as_deref(), Some("dX3k_QDnzHE"));
        assert_eq!(home.shelves[0].items[1].kind, YtmItemKind::Album);
        assert_eq!(home.shelves[0].items[1].browse_id.as_deref(), Some("MPREb_12345"));
        assert_eq!(home.continuation.as_deref(), Some("tok123"));

        let search_json = serde_json::json!({
            "contents": {
                "tabbedSearchResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicShelfRenderer": {
                                            "title": { "runs": [{"text": "Songs"}] },
                                            "contents": [{
                                                "musicResponsiveListItemRenderer": {
                                                    "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/s=w120-h120", "width": 120, "height": 120 }] } } },
                                                    "flexColumns": [
                                                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Instant Crush" }] } } },
                                                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [
                                                            { "text": "Daft Punk", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdaft", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" } } } } },
                                                            { "text": " • " },
                                                            { "text": "Random Access Memories", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_ram", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" } } } } },
                                                            { "text": " • " },
                                                            { "text": "5:37" }
                                                        ] } } },
                                                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "980M plays" }] } } }
                                                    ],
                                                    "overlay": { "musicItemThumbnailOverlayRenderer": { "content": { "musicPlayButtonRenderer": { "playNavigationEndpoint": { "watchEndpoint": { "videoId": "a5uQMwRMHcs" } } } } } }
                                                }
                                            }]
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });

        let sr = parse_ytm_search_json(&search_json);
        assert_eq!(sr.tracks.len(), 1);
        assert_eq!(sr.tracks[0].video_id, "a5uQMwRMHcs");
        assert_eq!(sr.tracks[0].artist, "Daft Punk");
        assert_eq!(sr.tracks[0].album.as_deref(), Some("Random Access Memories"));
        assert_eq!(sr.tracks[0].duration_secs, Some(337));
        assert_eq!(sr.tracks[0].plays_text.as_deref(), Some("980M plays"));
    }

    #[test]
    fn test_parse_ytm_next_and_lyrics() {
        let next_json = serde_json::json!({
            "contents": {
                "singleColumnMusicWatchNextResultsRenderer": {
                    "tabbedRenderer": {
                        "watchNextTabbedResultsRenderer": {
                            "tabs": [
                                {
                                    "tabRenderer": {
                                        "title": "Up next",
                                        "content": {
                                            "musicQueueRenderer": {
                                                "content": {
                                                    "playlistPanelRenderer": {
                                                        "contents": [{
                                                            "playlistPanelVideoWrapperRenderer": {
                                                                "primaryRenderer": {
                                                                    "playlistPanelVideoRenderer": {
                                                                        "title": { "runs": [{ "text": "Get Lucky" }] },
                                                                        "longBylineText": { "runs": [
                                                                            { "text": "Daft Punk", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCdaft", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" } } } } },
                                                                            { "text": " • " },
                                                                            { "text": "Random Access Memories", "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_ram", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" } } } } }
                                                                        ] },
                                                                        "lengthText": { "runs": [{ "text": "4:08" }] },
                                                                        "videoId": "5NV6Rdv1a3I",
                                                                        "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/gl=w544-h544", "width": 544, "height": 544 }] }
                                                                    }
                                                                }
                                                            }
                                                        }]
                                                    }
                                                }
                                            }
                                        }
                                    }
                                },
                                {
                                    "tabRenderer": {
                                        "title": "Lyrics",
                                        "endpoint": { "browseEndpoint": { "browseId": "MPLYt_test123", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_TRACK_LYRICS" } } } }
                                    }
                                },
                                {
                                    "tabRenderer": {
                                        "title": "Related",
                                        "endpoint": { "browseEndpoint": { "browseId": "MPTRt_test123", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_TRACK_RELATED" } } } }
                                    }
                                }
                            ]
                        }
                    }
                }
            }
        });

        let q = parse_ytm_next_json(&next_json);
        assert_eq!(q.tracks.len(), 1);
        assert_eq!(q.tracks[0].video_id, "5NV6Rdv1a3I");
        assert_eq!(q.tracks[0].duration_secs, Some(248));
        assert_eq!(q.lyrics_browse_id.as_deref(), Some("MPLYt_test123"));
        assert_eq!(q.related_browse_id.as_deref(), Some("MPTRt_test123"));

        let timed_json = serde_json::json!({
            "contents": {
                "sectionListRenderer": {
                    "contents": [{
                        "musicTimedLyricsRenderer": {
                            "timedLyricsData": [
                                { "lyricLine": "Like the legend of the phoenix", "cueRange": { "startTimeMilliseconds": "12000", "endTimeMilliseconds": "16000" } }
                            ],
                            "sourceMessage": "Source: LyricFind"
                        }
                    }]
                }
            }
        });
        let lyrics = parse_ytm_lyrics_json(&timed_json);
        assert!(lyrics.is_timed);
        assert_eq!(lyrics.lines.len(), 1);
        assert_eq!(lyrics.lines[0].start_ms, Some(12000));
        assert_eq!(lyrics.source.as_deref(), Some("Source: LyricFind"));

        let lrc = "[ar:Jane Remover]\n[00:18.91]Teeth stained yellow\n[00:23.77]For a glass love so hard\n[00:27.83]\n[00:30.88]Oh, and I'll be the one";
        let lrc_parsed = parse_lrc_lyrics(lrc, Some("Source: LRCLIB (Synced)".to_string()));
        assert!(lrc_parsed.is_timed);
        assert_eq!(lrc_parsed.lines.len(), 3);
        assert_eq!(lrc_parsed.lines[0].start_ms, Some(18910));
        assert_eq!(lrc_parsed.lines[0].end_ms, Some(23770));
        assert_eq!(lrc_parsed.lines[1].end_ms, Some(27830));
        assert_eq!(lrc_parsed.lines[2].start_ms, Some(30880));
    }

    #[test]
    fn test_parse_ytm_album_popup_metadata() {
        let album_json = serde_json::json!({
            "contents": {
                "twoColumnBrowseResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicResponsiveHeaderRenderer": {
                                            "title": { "runs": [{ "text": "Black Holes and Revelations" }] },
                                            "subtitle": { "runs": [
                                                { "text": "Album" },
                                                { "text": " • " },
                                                { "text": "Muse", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCmuse", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" } } } } },
                                                { "text": " • " },
                                                { "text": "2006" }
                                            ] },
                                            "secondSubtitle": { "runs": [{ "text": "11 songs · 45 minutes" }] },
                                            "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/bh=w544-h544", "width": 544, "height": 544 }] } } }
                                        }
                                    }]
                                }
                            }
                        }
                    }]
                }
            }
        });
        let page = parse_ytm_browse_json(&album_json);
        assert_eq!(
            page.header_title.as_deref(),
            Some("Black Holes and Revelations")
        );
        assert_eq!(page.header_artist.as_deref(), Some("Muse"));
        assert_eq!(page.header_artist_browse_id.as_deref(), Some("UCmuse"));
        assert_eq!(page.header_year.as_deref(), Some("2006"));
        assert_eq!(
            page.header_track_summary.as_deref(),
            Some("11 songs · 45 minutes")
        );

        // Album page with track rows omitting artist/album inherits header_artist & header_title
        let alb_tracks_json = serde_json::json!({
            "contents": {
                "twoColumnBrowseResultsRenderer": {
                    "tabs": [{
                        "tabRenderer": {
                            "content": {
                                "sectionListRenderer": {
                                    "contents": [{
                                        "musicResponsiveHeaderRenderer": {
                                            "title": { "runs": [{ "text": "Census Designated" }] },
                                            "subtitle": { "runs": [
                                                { "text": "Album" },
                                                { "text": " • " },
                                                { "text": "2023" }
                                            ] },
                                            "straplineTextOne": { "runs": [
                                                { "text": "Jane Remover", "navigationEndpoint": { "browseEndpoint": { "browseId": "UCjane", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ARTIST" } } } } }
                                            ] }
                                        }
                                    }]
                                }
                            }
                        }
                    }],
                    "secondaryContents": {
                        "sectionListRenderer": {
                            "contents": [{
                                "musicShelfRenderer": {
                                    "contents": [{
                                        "musicResponsiveListItemRenderer": {
                                            "flexColumns": [
                                                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Lips" }] } } }
                                            ],
                                            "fixedColumns": [
                                                { "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [{ "text": "5:11" }] } } }
                                            ],
                                            "playlistItemData": { "videoId": "lips_vid" }
                                        }
                                    }]
                                }
                            }]
                        }
                    }
                }
            }
        });
        let alb_tracks_page = parse_ytm_browse_json(&alb_tracks_json);
        assert_eq!(alb_tracks_page.tracks.len(), 1);
        assert_eq!(alb_tracks_page.tracks[0].artist, "Jane Remover");
        assert_eq!(
            alb_tracks_page.tracks[0].album.as_deref(),
            Some("Census Designated")
        );

        // Collection card with overlay videoId must drop video_id so clicking never interrupts playback
        let album_card = serde_json::json!({
            "title": { "runs": [{ "text": "Absolution" }] },
            "subtitle": { "runs": [{ "text": "Album • Muse" }] },
            "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_abso", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" } } } },
            "thumbnailOverlay": { "musicItemThumbnailOverlayRenderer": { "content": { "musicPlayButtonRenderer": { "playNavigationEndpoint": { "watchEndpoint": { "videoId": "first_track_vid", "playlistId": "OLAK_abso" } } } } } },
            "thumbnailRenderer": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/ab=w60-h60-l90-rj", "width": 60, "height": 60 }] } } }
        });
        let parsed_card = parse_two_row_item(&album_card).unwrap();
        assert_eq!(parsed_card.kind, YtmItemKind::Album);
        assert_eq!(parsed_card.browse_id.as_deref(), Some("MPREb_abso"));
        assert!(parsed_card.video_id.is_none());
        assert!(parsed_card.thumb_url.contains("=w544-h544-l90-rj"));

        // Personal editable playlist header + track cover fallback
        let pl_json = serde_json::json!({
            "header": {
                "musicEditablePlaylistDetailHeaderRenderer": {
                    "header": {
                        "musicDetailHeaderRenderer": {
                            "title": { "runs": [{ "text": "Still listening" }] },
                            "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/pl=w120-h120", "width": 120, "height": 120 }] }
                        }
                    }
                }
            },
            "contents": {
                "singleColumnBrowseResultsRenderer": {
                    "tabs": [{ "tabRenderer": { "content": { "sectionListRenderer": { "contents": [{
                        "musicPlaylistShelfRenderer": {
                            "contents": [{
                                "musicResponsiveListItemRenderer": {
                                    "flexColumns": [
                                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Into Darkness" }] } } },
                                        { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Thomas Bergersen" }] } } }
                                    ],
                                    "playlistItemData": { "videoId": "trk123" }
                                }
                            }]
                        }
                    }] } } } }]
                }
            }
        });
        let pl_page = parse_ytm_browse_json(&pl_json);
        assert_eq!(pl_page.header_title.as_deref(), Some("Still listening"));
        assert!(pl_page.header_thumb.as_deref().unwrap().contains("=w544-h544"));
        assert_eq!(pl_page.tracks.len(), 1);

        // Rich Artist Profile (immersive header + Top Songs + Albums + Description shelf)
        let artist_json = serde_json::json!({
            "header": {
                "musicImmersiveHeaderRenderer": {
                    "title": { "runs": [{ "text": "Muse" }] },
                    "description": { "runs": [{ "text": "English rock band formed in 1994." }] },
                    "subscriptionButton": { "subscribeButtonRenderer": { "subscriberCountText": { "runs": [{ "text": "14.2M subscribers" }] } } },
                    "playButton": { "buttonRenderer": { "navigationEndpoint": { "watchEndpoint": { "playlistId": "OLAK_top" } } } },
                    "startRadioButton": { "buttonRenderer": { "navigationEndpoint": { "watchPlaylistEndpoint": { "playlistId": "RDEM_muse" } } } },
                    "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "https://lh3.googleusercontent.com/muse=w544-h544", "width": 544, "height": 544 }] } } }
                }
            },
            "contents": {
                "singleColumnBrowseResultsRenderer": {
                    "tabs": [{ "tabRenderer": { "content": { "sectionListRenderer": { "contents": [
                        {
                            "musicShelfRenderer": {
                                "title": { "runs": [{ "text": "Songs" }] },
                                "contents": [{
                                    "musicResponsiveListItemRenderer": {
                                        "flexColumns": [
                                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Starlight" }] } } },
                                            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "Muse" }] } } }
                                        ],
                                        "overlay": { "musicItemThumbnailOverlayRenderer": { "content": { "musicPlayButtonRenderer": { "playNavigationEndpoint": { "watchEndpoint": { "videoId": "Pgum6OT_VH8" } } } } } }
                                    }
                                }]
                            }
                        },
                        {
                            "musicCarouselShelfRenderer": {
                                "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [{ "text": "Albums" }] } } },
                                "contents": [{
                                    "musicTwoRowItemRenderer": {
                                        "title": { "runs": [{ "text": "Black Holes and Revelations" }] },
                                        "subtitle": { "runs": [{ "text": "2006" }] },
                                        "navigationEndpoint": { "browseEndpoint": { "browseId": "MPREb_bh", "browseEndpointContextSupportedConfigs": { "browseEndpointContextMusicConfig": { "pageType": "MUSIC_PAGE_TYPE_ALBUM" } } } }
                                    }
                                }]
                            }
                        },
                        {
                            "musicDescriptionShelfRenderer": {
                                "description": { "runs": [{ "text": "Full Teignmouth biography." }] }
                            }
                        }
                    ] } } } }]
                }
            }
        });
        let art_page = parse_ytm_browse_json(&artist_json);
        assert!(art_page.is_artist_profile);
        assert_eq!(art_page.header_subscribers.as_deref(), Some("14.2M subscribers"));
        assert_eq!(art_page.header_radio_playlist_id.as_deref(), Some("RDEM_muse"));
        assert!(art_page.header_description.as_deref().unwrap().contains("Teignmouth"));
        assert_eq!(art_page.tracks.len(), 1);
        assert!(art_page.shelves.iter().any(|s| s.title == "Albums"));
    }
}
