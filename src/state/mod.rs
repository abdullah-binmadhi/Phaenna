//! Central Application State — single source of truth for screens, overlays, cache & playback.

use crate::api::dislike::DislikeInfo;
use crate::api::innertube::{SearchChannelCard, VideoComment, VideoVote};
use crate::api::sponsorblock::SponsorSegment;
use crate::api::subtitles::SubtitleTrack;
use crate::api::ytdlp::ResolvedStreamLadder;
use crate::api::ytmusic::{
    YtmLyrics, YtmPage, YtmSearchFilter, YtmShelfItem, YtmTrack,
};
use crate::config::{
    Config, PerformanceMode, PicturePreset, Player, Quality, SponsorBlockMode, VideoShaderMode,
};
use crate::db::cache::VideoItem;
use crate::db::sync::{
    HISTORY, RECOMMENDED, SHORTS_SUBSCRIBED, SHORTS_SUGGESTED, SUBSCRIPTIONS, WATCH_LATER,
};
use crate::db::user::{
    AnalysisChannelSort, AnalysisRange, AnalysisTab, ContinueWatchingMeta, UserAnalytics,
    WatchLaterSource,
};
use crate::services::auth::{AuthVault, LoginUiState};
use crate::services::calendar::{compute_week_bounds, local_tz_offset_secs, CalendarFilter, WeekBounds};
use crate::services::cast::CastDevice;
use crate::services::ipc::{LiveIpcEvent, LiveMediaTrack, PlayerCommand};
use crate::services::remote::RemoteServerHandle;
use crate::services::substyler::SubtitlePreset;
use crate::services::updater::UpdateInfo;
use crate::ui::theme::{self, extract_ambilight_palette, ThemePalette};
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum ThumbPriority {
    Focused = 0,
    #[default]
    Visible = 1,
    Prefetch = 2,
}

#[derive(Debug, Clone)]
pub struct ThumbRequest {
    pub video_id: String,
    pub url: String,
    pub priority: ThumbPriority,
}

#[derive(Debug, Clone)]
pub struct FailedThumbEntry {
    pub attempts: u8,
    pub next_retry_at: Instant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchViewMode {
    #[default]
    Grid,
    Split,
}

impl SearchViewMode {
    pub const fn toggle(self) -> Self {
        match self {
            Self::Grid => Self::Split,
            Self::Split => Self::Grid,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SearchFocus {
    #[default]
    Query,
    Results,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParsedSearchQuery {
    pub api_query: String,
    pub tokens: Vec<String>,
    pub exact_phrases: Vec<String>,
    pub excluded_terms: Vec<String>,
    pub channel_filter: Option<String>,
    pub force_type: Option<SearchTypeFilter>,
    pub only_4k: bool,
    pub only_hd: bool,
    pub only_live: bool,
    pub only_unwatched: bool,
}

pub fn parse_search_query(raw: &str) -> ParsedSearchQuery {
    let mut parsed = ParsedSearchQuery::default();
    let mut api_parts: Vec<String> = Vec::new();
    let mut i = 0;
    let bytes = raw.as_bytes();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'"' {
            i += 1;
            let start = i;
            while i < bytes.len() && bytes[i] != b'"' {
                i += 1;
            }
            let phrase = raw[start..i].trim().to_lowercase();
            if i < bytes.len() && bytes[i] == b'"' {
                i += 1;
            }
            if !phrase.is_empty() {
                api_parts.push(format!("\"{phrase}\""));
                for w in phrase.split_whitespace() {
                    parsed.tokens.push(w.to_string());
                }
                parsed.exact_phrases.push(phrase);
            }
            continue;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        let tok = &raw[start..i];
        let low = tok.to_ascii_lowercase();
        if let Some(ex) = low.strip_prefix('-').filter(|s| !s.is_empty()) {
            parsed.excluded_terms.push(ex.to_string());
            api_parts.push(format!("-{ex}"));
        } else if matches!(low.as_str(), "is:short" | "is:shorts") {
            parsed.force_type = Some(SearchTypeFilter::Shorts);
        } else if matches!(low.as_str(), "is:video" | "is:videos") {
            parsed.force_type = Some(SearchTypeFilter::Videos);
        } else if low == "is:live" {
            parsed.only_live = true;
            parsed.force_type = Some(SearchTypeFilter::Live);
        } else if low == "is:4k" {
            parsed.only_4k = true;
        } else if low == "is:hd" {
            parsed.only_hd = true;
        } else if low == "is:unwatched" {
            parsed.only_unwatched = true;
        } else if let Some(ch) = low.strip_prefix("ch:").or_else(|| low.strip_prefix("channel:")) {
            let clean = ch.trim_start_matches('@').trim();
            if !clean.is_empty() {
                parsed.channel_filter = Some(clean.to_string());
                api_parts.push(clean.to_string());
            }
        } else if let Some(handle) = low.strip_prefix('@') {
            let clean = handle.trim();
            if !clean.is_empty() {
                parsed.channel_filter = Some(clean.to_string());
                api_parts.push(clean.to_string());
            }
        } else if !low.is_empty() {
            parsed.tokens.push(low.clone());
            api_parts.push(tok.to_string());
        }
    }
    parsed.api_query = api_parts.join(" ");
    parsed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalysisHit {
    SelectTab(AnalysisTab),
    SelectRange(AnalysisRange),
    ToggleYtSync,
    CycleChannelSort,
    BudgetDelta(i32),
    SelectChannel(usize),
    SelectSession(usize),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HelpHit {
    OpenSettings,
    OpenSearch,
    OpenStudio,
    OpenAnalytics,
    OpenCalendar,
    OpenLogin,
    CycleTheme,
    Close,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Home,
    Detail,
    Channel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailViewMode {
    Overview,
    Chapters,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveOverlay {
    None,
    Search,
    Login,
    Filter,
    PlayOptions,
    Settings,
    MusicSettings,
    MusicEq,
    MusicAlbum,
    Help,
    Analytics,
    Calendar,
    Remote,
    Cast,
    Download,
    Update,
    ConfirmQuit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Recommended,
    Shorts,
    Subscriptions,
    History,
    WatchLater,
    ContinueWatching,
    Music,
}

impl Category {
    pub fn all_shelves() -> [Category; 7] {
        [
            Category::ContinueWatching,
            Category::Recommended,
            Category::Shorts,
            Category::Subscriptions,
            Category::History,
            Category::WatchLater,
            Category::Music,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Category::ContinueWatching => "Continue Watching",
            Category::Recommended => "Home Feed",
            Category::Shorts => "Shorts",
            Category::Subscriptions => "Subscriptions",
            Category::History => "History",
            Category::WatchLater => "Watch Later",
            Category::Music => "Music",
        }
    }

    pub fn feed_key(&self) -> &'static str {
        match self {
            Category::ContinueWatching => "continue_watching",
            Category::Recommended => RECOMMENDED,
            Category::Shorts => "shorts",
            Category::Subscriptions => SUBSCRIPTIONS,
            Category::History => HISTORY,
            Category::WatchLater => WATCH_LATER,
            Category::Music => "music",
        }
    }

    pub fn url(&self) -> Option<&'static str> {
        match self {
            Category::Recommended => Some("https://www.youtube.com"),
            Category::Shorts => Some("https://www.youtube.com/shorts"),
            Category::Subscriptions => Some("https://www.youtube.com/feed/subscriptions"),
            Category::History => Some("https://www.youtube.com/feed/history"),
            Category::WatchLater => Some("https://www.youtube.com/playlist?list=WL"),
            Category::ContinueWatching => None,
            Category::Music => Some("https://music.youtube.com"),
        }
    }
}

/// Top-level navigation section inside the YouTube Music tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YtmSection {
    Home,
    Library,
    Liked,
    Charts,
    Search,
    Browse,
}

impl YtmSection {
    #[allow(dead_code)]
    pub const NAV_TABS: [Self; 5] = [
        Self::Home,
        Self::Library,
        Self::Liked,
        Self::Charts,
        Self::Search,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            Self::Home => "Home",
            Self::Library => "Library",
            Self::Liked => "Liked",
            Self::Charts => "Charts",
            Self::Search => "Search",
            Self::Browse => "Browse",
        }
    }
}

/// Repeat policy for the Music player queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YtmRepeatMode {
    Off,
    All,
    One,
}

impl YtmRepeatMode {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::All => "All",
            Self::One => "One",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            Self::Off => Self::All,
            Self::All => Self::One,
            Self::One => Self::Off,
        }
    }
}

/// Right-hand auxiliary pane inside Now Playing on the Music tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum YtmRightPaneTab {
    UpNext,
    Lyrics,
}

/// Flattened display row rendered in the Music browse list.
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum YtmBrowseRow {
    ShelfHeader {
        title: String,
    },
    Item {
        item: YtmShelfItem,
        selectable_idx: usize,
        track_num: Option<usize>,
    },
}

/// Mouse click target inside the Music tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicHit {
    SelectSection(YtmSection),
    OpenSidebarPlaylist(usize),
    GoBack,
    FocusSearch,
    SelectSearchFilter(YtmSearchFilter),
    SelectItem(usize),
    TogglePlayPause,
    NextTrack,
    PrevTrack,
    ToggleShuffle,
    CycleRepeat,
    ToggleVideoMode,
    ToggleLike,
    ToggleRightDrawer,
    OpenMusicSettings,
    OpenMusicEq,
    OpenCurrentAlbum,
    VolumeDelta(i64),
    ToggleMute,
    SelectRightTab(YtmRightPaneTab),
    SelectQueueTrack(usize),
    SeekBar,
}

/// Filter tabs inside a YouTube Music Artist Profile popup.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum YtmArtistSectionTab {
    #[default]
    All,
    TopSongs,
    Albums,
    Singles,
    Videos,
    About,
}

impl YtmArtistSectionTab {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::TopSongs,
        Self::Albums,
        Self::Singles,
        Self::Videos,
        Self::About,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::TopSongs => "Top Songs",
            Self::Albums => "Albums",
            Self::Singles => "Singles",
            Self::Videos => "Videos",
            Self::About => "About",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::TopSongs,
            Self::TopSongs => Self::Albums,
            Self::Albums => Self::Singles,
            Self::Singles => Self::Videos,
            Self::Videos => Self::About,
            Self::About => Self::All,
        }
    }
}

/// Mouse click target inside the Spotify-style Album / Playlist / Artist popup modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YtmAlbumPopupHit {
    Close,
    Back,
    PlayAll,
    ShuffleAll,
    StartRadio,
    OpenArtist,
    SelectArtistTab(YtmArtistSectionTab),
    ExpandShelf(usize),
    PlayTrack(usize),
    QueueTrack(usize),
    LikeTrack(usize),
    OpenItem(usize),
}

/// Interactive state for the centered Spotify-style Album / Playlist / Artist detail popup.
#[derive(Debug, Clone, Default)]
pub struct YtmAlbumPopupState {
    pub title: String,
    pub browse_id: String,
    pub params: Option<String>,
    pub page: Option<YtmPage>,
    pub loading: bool,
    pub error: Option<String>,
    pub cursor: usize,
    pub scroll: usize,
    pub section_filter: YtmArtistSectionTab,
    pub history: Vec<(String, String, Option<String>, YtmPage)>,
    pub hitboxes: Vec<(Rect, YtmAlbumPopupHit)>,
}

/// Mouse click target inside the dedicated YouTube Music Settings modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicSettingsHit {
    Close,
    OpenEqStudio,
    ActivateRow(usize),
}

/// Mouse click target inside the dedicated YouTube Music 10-Band EQ Studio modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MusicEqHit {
    Close,
    SelectPreset(crate::config::AudioEqPreset),
    SelectSavedPreset(usize),
    DeleteSavedPreset(usize),
    StartSavePrompt,
    OverwriteCustom,
    CancelSavePrompt,
    ToggleBypass,
    ResetFlat,
    PreampDelta(i8),
    SelectBand(usize),
    BandDelta(usize, i8),
    SetBandGain(usize, i8),
}

/// Interactive state for the dedicated YouTube Music 10-Band EQ Studio modal.
#[derive(Debug, Clone, Default)]
pub struct YtmMusicEqState {
    pub band_cursor: usize,
    pub save_editing: bool,
    pub save_input: String,
    pub hitboxes: Vec<(Rect, MusicEqHit)>,
}

/// Full interactive state for the YouTube Music tab and background audio player.
#[derive(Debug, Clone)]
pub struct YtmMusicState {
    pub section: YtmSection,
    pub browse_stack: Vec<(YtmSection, String, YtmPage)>,
    pub home_page: Option<YtmPage>,
    pub library_page: Option<YtmPage>,
    pub liked_page: Option<YtmPage>,
    pub charts_page: Option<YtmPage>,
    pub search_page: Option<YtmPage>,
    pub browse_page: Option<YtmPage>,
    pub browse_title: String,
    pub search_query: String,
    pub search_editing: bool,
    pub search_filter: YtmSearchFilter,
    pub cursor: usize,
    pub scroll: usize,
    pub loading: bool,
    pub loading_label: String,
    pub error: Option<String>,
    pub queue: Vec<YtmTrack>,
    pub queue_index: usize,
    pub queue_continuation: Option<String>,
    pub shuffle: bool,
    pub repeat: YtmRepeatMode,
    pub video_mode: bool,
    pub right_drawer_open: bool,
    pub active_audio_session_id: Option<u64>,
    pub liked_ids: HashSet<String>,
    pub right_tab: YtmRightPaneTab,
    pub queue_cursor: usize,
    pub lyrics_browse_id: Option<String>,
    pub related_browse_id: Option<String>,
    pub lyrics: Option<YtmLyrics>,
    pub lyrics_loading: bool,
    pub album_popup: YtmAlbumPopupState,
    pub last_opened_collection: Option<(String, String, Option<String>)>,
    pub eq: YtmMusicEqState,
    pub settings_cursor: usize,
    pub settings_hitboxes: Vec<(Rect, MusicSettingsHit)>,
    pub last_click: Option<(usize, Instant)>,
    pub sidebar_last_click: Option<(usize, Instant)>,
    pub hitboxes: Vec<(Rect, MusicHit)>,
    pub progress_bar_rect: Rect,
}

impl Default for YtmMusicState {
    fn default() -> Self {
        Self {
            section: YtmSection::Home,
            browse_stack: Vec::new(),
            home_page: None,
            library_page: None,
            liked_page: None,
            charts_page: None,
            search_page: None,
            browse_page: None,
            browse_title: String::new(),
            search_query: String::new(),
            search_editing: false,
            search_filter: YtmSearchFilter::All,
            cursor: 0,
            scroll: 0,
            loading: false,
            loading_label: String::new(),
            error: None,
            queue: Vec::new(),
            queue_index: 0,
            queue_continuation: None,
            shuffle: false,
            repeat: YtmRepeatMode::Off,
            video_mode: false,
            right_drawer_open: true,
            active_audio_session_id: None,
            liked_ids: HashSet::new(),
            right_tab: YtmRightPaneTab::UpNext,
            queue_cursor: 0,
            lyrics_browse_id: None,
            related_browse_id: None,
            lyrics: None,
            lyrics_loading: false,
            album_popup: YtmAlbumPopupState::default(),
            last_opened_collection: None,
            eq: YtmMusicEqState::default(),
            settings_cursor: 0,
            settings_hitboxes: Vec::new(),
            last_click: None,
            sidebar_last_click: None,
            hitboxes: Vec::new(),
            progress_bar_rect: Rect::default(),
        }
    }
}

impl YtmMusicState {
    /// Active page corresponding to the current `section`.
    pub fn current_page(&self) -> Option<&YtmPage> {
        match self.section {
            YtmSection::Home => self.home_page.as_ref(),
            YtmSection::Library => self.library_page.as_ref(),
            YtmSection::Liked => self.liked_page.as_ref(),
            YtmSection::Charts => self.charts_page.as_ref(),
            YtmSection::Search => self.search_page.as_ref(),
            YtmSection::Browse => self.browse_page.as_ref(),
        }
    }

    /// Mutable slot for storing a loaded page for `section`.
    pub fn page_slot_mut(&mut self, section: YtmSection) -> &mut Option<YtmPage> {
        match section {
            YtmSection::Home => &mut self.home_page,
            YtmSection::Library => &mut self.library_page,
            YtmSection::Liked => &mut self.liked_page,
            YtmSection::Charts => &mut self.charts_page,
            YtmSection::Search => &mut self.search_page,
            YtmSection::Browse => &mut self.browse_page,
        }
    }

    /// Flatten current shelves + direct track list into navigable rows.
    pub fn build_rows(&self) -> (Vec<YtmBrowseRow>, Vec<YtmShelfItem>) {
        let Some(page) = self.current_page() else {
            return (Vec::new(), Vec::new());
        };
        let mut rows = Vec::new();
        let mut items = Vec::new();

        for shelf in &page.shelves {
            if shelf.items.is_empty() {
                continue;
            }
            if !shelf.title.trim().is_empty() {
                rows.push(YtmBrowseRow::ShelfHeader {
                    title: shelf.title.clone(),
                });
            }
            for item in &shelf.items {
                let idx = items.len();
                items.push(item.clone());
                rows.push(YtmBrowseRow::Item {
                    item: item.clone(),
                    selectable_idx: idx,
                    track_num: None,
                });
            }
        }

        if !page.tracks.is_empty() {
            if !page.shelves.is_empty() {
                rows.push(YtmBrowseRow::ShelfHeader {
                    title: "Tracks".to_string(),
                });
            }
            for (i, tr) in page.tracks.iter().enumerate() {
                let mut subtitle_parts = vec![tr.artist.clone()];
                if let Some(ref alb) = tr.album {
                    subtitle_parts.push(alb.clone());
                }
                if let Some(ref dur) = tr.duration_text {
                    subtitle_parts.push(dur.clone());
                }
                let item = YtmShelfItem {
                    kind: tr.kind,
                    title: tr.title.clone(),
                    subtitle: subtitle_parts.join(" • "),
                    video_id: Some(tr.video_id.clone()),
                    browse_id: tr.album_browse_id.clone(),
                    playlist_id: tr
                        .playlist_id
                        .clone()
                        .or_else(|| page.header_playlist_id.clone()),
                    params: None,
                    thumb_url: if !tr.thumb_url.is_empty() {
                        tr.thumb_url.clone()
                    } else if let Some(ref ht) = page.header_thumb {
                        ht.clone()
                    } else {
                        String::new()
                    },
                    duration_secs: tr.duration_secs,
                };
                let idx = items.len();
                items.push(item.clone());
                rows.push(YtmBrowseRow::Item {
                    item,
                    selectable_idx: idx,
                    track_num: Some(i + 1),
                });
            }
        }

        (rows, items)
    }

    /// Return all selectable items on the current page.
    pub fn selectable_items(&self) -> Vec<YtmShelfItem> {
        self.build_rows().1
    }

    /// Currently active track in the Music queue.
    pub fn current_track(&self) -> Option<&YtmTrack> {
        self.queue.get(self.queue_index)
    }

    /// Compute next track index honoring Shuffle and Repeat mode (`manual` overrides Repeat::One).
    pub fn next_queue_index(&self, manual: bool) -> Option<usize> {
        if self.queue.is_empty() {
            return None;
        }
        if !manual && self.repeat == YtmRepeatMode::One {
            return Some(self.queue_index);
        }
        if self.shuffle && self.queue.len() > 1 {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.subsec_nanos() as usize)
                .unwrap_or(1);
            let mut next =
                (self.queue_index + 1 + (nanos % (self.queue.len() - 1))) % self.queue.len();
            if next == self.queue_index {
                next = (self.queue_index + 1) % self.queue.len();
            }
            return Some(next);
        }
        if self.queue_index + 1 < self.queue.len() {
            Some(self.queue_index + 1)
        } else if self.repeat == YtmRepeatMode::All {
            Some(0)
        } else {
            None
        }
    }

    /// Compute previous track index honoring Repeat mode.
    pub fn prev_queue_index(&self) -> Option<usize> {
        if self.queue.is_empty() {
            return None;
        }
        if self.queue_index > 0 {
            Some(self.queue_index - 1)
        } else if self.repeat == YtmRepeatMode::All {
            Some(self.queue.len() - 1)
        } else {
            Some(0)
        }
    }

    /// Find the index of the active synced lyric line for `pos_secs`.
    pub fn active_lyric_index(&self, pos_secs: f64) -> Option<usize> {
        let lyrics = self.lyrics.as_ref()?;
        if !lyrics.is_timed || lyrics.lines.is_empty() {
            return None;
        }
        let pos_ms = (pos_secs.max(0.0) * 1000.0) as u64;
        let mut active = None;
        for (idx, line) in lyrics.lines.iter().enumerate() {
            if let Some(start) = line.start_ms {
                if pos_ms >= start {
                    active = Some(idx);
                } else {
                    break;
                }
            }
        }
        active
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortsSource {
    Suggested,
    Subscribed,
}

impl ShortsSource {
    pub fn label(self) -> &'static str {
        match self {
            ShortsSource::Suggested => "Algorithm Suggested",
            ShortsSource::Subscribed => "Subscribed",
        }
    }

    pub fn toggle(self) -> Self {
        match self {
            ShortsSource::Suggested => ShortsSource::Subscribed,
            ShortsSource::Subscribed => ShortsSource::Suggested,
        }
    }

    pub fn feed_key(self) -> &'static str {
        match self {
            ShortsSource::Suggested => SHORTS_SUGGESTED,
            ShortsSource::Subscribed => SHORTS_SUBSCRIBED,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentFilter {
    All,
    Longform,
    Shorts,
}

impl ContentFilter {
    pub fn label(&self) -> &'static str {
        match self {
            ContentFilter::All => "All Formats",
            ContentFilter::Longform => "Longform Only (5m+)",
            ContentFilter::Shorts => "Shorts / Quick (<=1m)",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Default,
    Newest,
    DurationLongest,
    DurationShortest,
    ChannelAZ,
}

impl SortOrder {
    pub fn label(&self) -> &'static str {
        match self {
            SortOrder::Default => "Default / Algorithm",
            SortOrder::Newest => "Newest Uploads",
            SortOrder::DurationLongest => "Duration (Longest)",
            SortOrder::DurationShortest => "Duration (Shortest)",
            SortOrder::ChannelAZ => "Channel (A-Z)",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DownloadJobState {
    pub video_id: String,
    pub title: String,
    pub percent: f32,
    pub speed: String,
    pub done_path: Option<String>,
    pub error: Option<String>,
}

pub struct AppState {
    pub screen: Screen,
    pub overlay: ActiveOverlay,
    pub config: Config,

    // Shelves & Home grid
    pub categories: Vec<(Category, Vec<VideoItem>)>,
    pub active_category: usize,
    pub category_cursors: Vec<usize>,
    pub grid_scroll_rows: Vec<usize>,
    pub content_filter: ContentFilter,
    pub sort_order: SortOrder,
    pub shorts_source: ShortsSource,
    pub shorts_suggested: Vec<VideoItem>,
    pub shorts_subscribed: Vec<VideoItem>,
    pub home_channel_hitboxes: Vec<(Rect, VideoItem)>,

    // User library fast-lookup caches
    pub watched_ids: HashSet<String>,
    pub progress_map: HashMap<String, (f64, f64)>,
    pub watch_later_sources: HashMap<String, WatchLaterSource>,
    pub offline_paths: HashMap<String, String>,
    pub continue_watching_meta: HashMap<String, ContinueWatchingMeta>,
    pub continue_watching_grid_mode: bool,

    // Detail & Channel drill-down stack
    pub detail_video: Option<VideoItem>,
    pub detail_history: Vec<VideoItem>,
    pub detail_view_mode: DetailViewMode,
    pub detail_focus_pane: usize, // 0: Synopsis, 1: Chapters, 2: Comments
    pub detail_chapter_cursor: usize,
    pub detail_related_cursor: usize,
    pub detail_scroll: u16,
    pub detail_subtitles: Vec<SubtitleTrack>,
    pub detail_selected_sub_idx: usize,
    pub detail_sponsor_segments: Vec<SponsorSegment>,
    pub detail_dislikes: Option<DislikeInfo>,
    pub detail_video_vote: VideoVote,
    pub detail_comments: Vec<VideoComment>,
    pub detail_comment_cursor: usize,
    pub detail_comment_sort: CommentSort,
    pub detail_comment_collapsed: HashSet<String>,
    pub detail_reply_target: Option<ReplyTarget>,
    pub detail_translate_lang: String,
    pub detail_translate_editing: bool,
    pub detail_translate_input: String,
    pub detail_comment_translations: HashMap<String, CommentTranslation>,
    pub detail_comment_translating: HashSet<String>,
    pub detail_comments_loading: bool,
    pub detail_metadata_loading: bool,
    pub detail_create_comment_params: Option<String>,
    pub detail_comment_composing: bool,
    pub detail_comment_input: String,
    pub detail_comments_panel_rect: Rect,
    pub detail_comment_composer_rect: Rect,
    pub detail_comment_hitboxes: Vec<(u16, u16, usize)>,
    pub detail_comment_node_hitboxes: Vec<CommentNodeHitbox>,
    pub detail_comment_last_click: Option<(usize, Instant)>,
    pub detail_last_node_click: Option<(String, Instant)>,
    pub detail_hitboxes: Vec<(Rect, DetailHit)>,

    pub channel_title: String,
    pub channel_id: Option<String>,
    pub channel_videos: Vec<VideoItem>,
    pub channel_cursor: usize,
    pub is_loading_channel: bool,
    pub channel_return_screen: Screen,

    // Search Studio + 120ms/350ms debounce + smart filters + hybrid RRF engine
    pub search_query: String,
    pub search_results: Vec<VideoItem>,
    pub search_shorts: Vec<VideoItem>,
    pub search_channels: Vec<SearchChannelCard>,
    pub search_did_you_mean: Option<String>,
    pub search_estimated_results: Option<u64>,
    pub search_latency_ms: Option<u64>,
    pub search_last_executed_query: String,
    pub search_local_results: Vec<VideoItem>,
    pub search_suggestions: Vec<String>,
    pub search_history: Vec<String>,
    pub search_continuation: Option<String>,
    pub search_scope: SearchScope,
    pub search_type_filter: SearchTypeFilter,
    pub search_date_filter: SearchDateFilter,
    pub search_duration_filter: SearchDurationFilter,
    pub search_sort_order: SearchSortOrder,
    pub search_hide_watched: bool,
    pub search_hd_4k_only: bool,
    pub search_view_mode: SearchViewMode,
    pub search_focus: SearchFocus,
    pub search_grid_cols: usize,
    pub return_to_search: bool,
    pub search_hitboxes: Vec<(Rect, SearchHit)>,
    pub search_results_rect: Rect,
    pub search_last_click: Option<(usize, Instant)>,
    pub search_cursor: usize,
    pub search_generation: u64,
    pub search_last_typed: Option<Instant>,
    pub search_pending_remote: bool,
    pub is_searching_remote: bool,

    // Multi-page Playback Studio state
    pub studio: StudioNavState,
    pub play_quality_idx: usize,
    pub manual_mastering_cursor: usize,

    // Filter modal
    pub filter_modal_section: usize,
    pub filter_modal_option: usize,

    // Settings Command Hub
    pub settings_query: String,
    pub settings_cursor: usize,

    // In-app Google Identity Hub & vault
    pub auth_vault: AuthVault,
    pub login_ui: LoginUiState,
    pub music: YtmMusicState,

    // Loading & pagination flags
    pub loading_feeds: HashSet<String>,
    pub is_fetching_more: bool,
    pub pagination_progress: Option<(usize, usize)>,
    pub feed_continuations: HashMap<String, String>,
    pub exhausted_feeds: HashSet<String>,
    pub last_pagination_at: Option<Instant>,

    // Playback IPC state
    pub active_playback_video_id: Option<String>,
    pub active_playback_title: Option<String>,
    pub active_playback_pos: f64,
    pub active_playback_dur: f64,
    pub active_session_id: u64,
    pub sponsor_skips_session: u32,
    pub live_playback: LivePlaybackControlState,
    pub stream_ladder_cache: HashMap<String, ResolvedStreamLadder>,
    pub overlay_comments: Vec<crate::api::innertube::VideoComment>,
    pub overlay_comment_params: Option<String>,
    pub overlay_comments_video_id: Option<String>,
    pub overlay_comments_uploader: Option<String>,

    // Intelligence & Services
    pub analytics: UserAnalytics,
    pub analytics_loaded: bool,
    pub analysis_tab: AnalysisTab,
    pub analysis_range: AnalysisRange,
    pub analysis_include_yt_sync: bool,
    pub analysis_channel_sort: AnalysisChannelSort,
    pub analysis_cursor: usize,
    pub analysis_hitboxes: Vec<(Rect, AnalysisHit)>,
    pub help_hitboxes: Vec<(Rect, HelpHit)>,
    pub calendar_bounds: WeekBounds,
    pub calendar_day_cursor: usize,
    pub calendar_item_cursor: usize,
    pub calendar_filter: CalendarFilter,
    pub calendar_tz_offset: i64,

    pub remote_handle: Option<RemoteServerHandle>,
    pub cast_devices: Vec<CastDevice>,
    pub cast_cursor: usize,
    pub is_scanning_cast: bool,
    pub active_download: Option<DownloadJobState>,
    pub available_update: Option<UpdateInfo>,
    pub update_status: Option<String>,
    pub update_progress: f32,
    pub is_checking_update: bool,
    pub is_updating: bool,
    pub update_completed_version: Option<String>,
    pub should_quit: bool,
    pub quit_confirm_yes: bool,

    // Toast & focused-card stock-ticker marquee
    pub toast: Option<(String, Instant)>,
    pub ticker_step: usize,
    pub ticker_focus_started_at: Instant,
    pub ticker_last_step_at: Instant,

    // Thumbnail & Ambilight Cache (keyed by (video_id, cell_w, cell_h) for 100% edge-to-edge cover)
    pub picker: Option<Picker>,
    pub image_cache: HashMap<(String, u16, u16), Box<dyn StatefulProtocol>>,
    pub image_cache_order: VecDeque<(String, u16, u16)>,
    pub raw_thumbs: HashMap<String, image::DynamicImage>,
    pub raw_thumbs_order: VecDeque<String>,
    pub ambilight_cache: HashMap<String, ThemePalette>,
    pub loading_thumbs: HashSet<String>,
    pub failed_thumbs: HashMap<String, FailedThumbEntry>,
    pub pending_thumb_requests: Vec<ThumbRequest>,
    pub kitty_flush_needed: bool,
    pub comment_media_cache: HashMap<String, crate::services::comment_media::DecodedCommentMedia>,
    pub loading_comment_media: HashSet<String>,
    pub pending_comment_media: Vec<(String, String)>,
    pub failed_comment_media: HashSet<String>,
}

impl AppState {
    pub fn new(config: Config) -> Self {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let categories = Category::all_shelves()
            .into_iter()
            .map(|c| (c, Vec::new()))
            .collect::<Vec<_>>();
        let n_cats = categories.len();
        let play_quality_idx = Quality::ALL
            .iter()
            .position(|&q| q == config.quality)
            .unwrap_or(0);
        let init_pic = config.picture.active_grade();
        let init_live_playback = LivePlaybackControlState {
            sub_visible: config.subtitles_enabled,
            brightness: init_pic.brightness,
            contrast: init_pic.contrast,
            saturation: init_pic.saturation,
            gamma: init_pic.gamma,
            hue: init_pic.hue,
            sharpness: init_pic.sharpness,
            ..Default::default()
        };

        let mut music = YtmMusicState::default();
        music.right_drawer_open = config.music.show_right_drawer;
        music.video_mode = config.music.default_video_mode;

        Self {
            screen: Screen::Home,
            overlay: ActiveOverlay::None,
            config,
            categories,
            active_category: 1, // Start on Recommended Home Feed (0 is ContinueWatching)
            category_cursors: vec![0; n_cats],
            grid_scroll_rows: vec![0; n_cats],
            content_filter: ContentFilter::All,
            sort_order: SortOrder::Default,
            shorts_source: ShortsSource::Suggested,
            shorts_suggested: Vec::new(),
            shorts_subscribed: Vec::new(),
            home_channel_hitboxes: Vec::new(),

            watched_ids: HashSet::new(),
            progress_map: HashMap::new(),
            watch_later_sources: HashMap::new(),
            offline_paths: HashMap::new(),
            continue_watching_meta: HashMap::new(),
            continue_watching_grid_mode: false,

            detail_video: None,
            detail_history: Vec::new(),
            detail_view_mode: DetailViewMode::Overview,
            detail_focus_pane: 0,
            detail_chapter_cursor: 0,
            detail_related_cursor: 0,
            detail_scroll: 0,
            detail_subtitles: Vec::new(),
            detail_selected_sub_idx: 0,
            detail_sponsor_segments: Vec::new(),
            detail_dislikes: None,
            detail_video_vote: VideoVote::None,
            detail_comments: Vec::new(),
            detail_comment_cursor: 0,
            detail_comment_sort: CommentSort::Top,
            detail_comment_collapsed: HashSet::new(),
            detail_reply_target: None,
            detail_translate_lang: "en".to_string(),
            detail_translate_editing: false,
            detail_translate_input: String::new(),
            detail_comment_translations: HashMap::new(),
            detail_comment_translating: HashSet::new(),
            detail_comments_loading: false,
            detail_metadata_loading: false,
            detail_create_comment_params: None,
            detail_comment_composing: false,
            detail_comment_input: String::new(),
            detail_comments_panel_rect: Rect::default(),
            detail_comment_composer_rect: Rect::default(),
            detail_comment_hitboxes: Vec::new(),
            detail_comment_node_hitboxes: Vec::new(),
            detail_comment_last_click: None,
            detail_last_node_click: None,
            detail_hitboxes: Vec::new(),

            channel_title: String::new(),
            channel_id: None,
            channel_videos: Vec::new(),
            channel_cursor: 0,
            is_loading_channel: false,
            channel_return_screen: Screen::Home,

            search_query: String::new(),
            search_results: Vec::new(),
            search_shorts: Vec::new(),
            search_channels: Vec::new(),
            search_did_you_mean: None,
            search_estimated_results: None,
            search_latency_ms: None,
            search_last_executed_query: String::new(),
            search_local_results: Vec::new(),
            search_suggestions: Vec::new(),
            search_history: Vec::new(),
            search_continuation: None,
            search_scope: SearchScope::All,
            search_type_filter: SearchTypeFilter::All,
            search_date_filter: SearchDateFilter::Any,
            search_duration_filter: SearchDurationFilter::Any,
            search_sort_order: SearchSortOrder::Relevance,
            search_hide_watched: false,
            search_hd_4k_only: false,
            search_view_mode: SearchViewMode::Grid,
            search_focus: SearchFocus::Query,
            search_grid_cols: 3,
            return_to_search: false,
            search_hitboxes: Vec::new(),
            search_results_rect: Rect::default(),
            search_last_click: None,
            search_cursor: 0,
            search_generation: 0,
            search_last_typed: None,
            search_pending_remote: false,
            is_searching_remote: false,

            studio: StudioNavState::default(),
            play_quality_idx,
            manual_mastering_cursor: 0,

            filter_modal_section: 0,
            filter_modal_option: 0,

            settings_query: String::new(),
            settings_cursor: 0,

            auth_vault: Config::auth_vault_path()
                .map(|p| AuthVault::load_or_default(&p))
                .unwrap_or_default(),
            login_ui: LoginUiState::default(),
            music,

            loading_feeds: HashSet::new(),
            is_fetching_more: false,
            pagination_progress: None,
            feed_continuations: HashMap::new(),
            exhausted_feeds: HashSet::new(),
            last_pagination_at: None,

            active_playback_video_id: None,
            active_playback_title: None,
            active_playback_pos: 0.0,
            active_playback_dur: 0.0,
            active_session_id: 0,
            sponsor_skips_session: 0,
            live_playback: init_live_playback,
            stream_ladder_cache: HashMap::new(),
            overlay_comments: Vec::new(),
            overlay_comment_params: None,
            overlay_comments_video_id: None,
            overlay_comments_uploader: None,

            analytics: UserAnalytics::default(),
            analytics_loaded: false,
            analysis_tab: AnalysisTab::Overview,
            analysis_range: AnalysisRange::Days7,
            analysis_include_yt_sync: false,
            analysis_channel_sort: AnalysisChannelSort::Hours,
            analysis_cursor: 0,
            analysis_hitboxes: Vec::new(),
            help_hitboxes: Vec::new(),
            calendar_bounds: compute_week_bounds(now_unix, 0, 0),
            calendar_day_cursor: 0,
            calendar_item_cursor: 0,
            calendar_filter: CalendarFilter::All,
            calendar_tz_offset: 0,

            remote_handle: None,
            cast_devices: Vec::new(),
            cast_cursor: 0,
            is_scanning_cast: false,
            active_download: None,
            available_update: None,
            update_status: None,
            update_progress: 0.0,
            is_checking_update: false,
            is_updating: false,
            update_completed_version: None,
            should_quit: false,
            quit_confirm_yes: false,

            toast: None,
            ticker_step: 0,
            ticker_focus_started_at: Instant::now(),
            ticker_last_step_at: Instant::now(),

            picker: None,
            image_cache: HashMap::new(),
            image_cache_order: VecDeque::new(),
            raw_thumbs: HashMap::new(),
            raw_thumbs_order: VecDeque::new(),
            ambilight_cache: HashMap::new(),
            loading_thumbs: HashSet::new(),
            failed_thumbs: HashMap::new(),
            pending_thumb_requests: Vec::new(),
            kitty_flush_needed: false,
            comment_media_cache: HashMap::new(),
            loading_comment_media: HashSet::new(),
            pending_comment_media: Vec::new(),
            failed_comment_media: HashSet::new(),
        }
    }

    pub fn base_theme(&self) -> &'static ThemePalette {
        theme::get_theme(self.config.theme_index)
    }

    pub fn active_palette_for(&self, video_id: Option<&str>) -> ThemePalette {
        let base = self.base_theme();
        if self.config.ambilight_enabled {
            if let Some(id) = video_id {
                if let Some(pal) = self.ambilight_cache.get(id) {
                    return *pal;
                }
            }
        }
        *base
    }

    pub fn show_toast(&mut self, msg: impl Into<String>) {
        self.toast = Some((msg.into(), Instant::now()));
    }

    pub fn clear_expired_toast(&mut self) {
        if let Some((_, ts)) = &self.toast {
            if ts.elapsed().as_secs() >= 4 {
                self.toast = None;
            }
        }
    }

    pub fn has_live_ipc(&self) -> bool {
        self.active_playback_video_id.is_some() && self.live_playback.cmd_tx.is_some()
    }

    pub fn send_player_cmd(&self, cmd: PlayerCommand) -> bool {
        if let Some(ref tx) = self.live_playback.cmd_tx {
            tx.send(cmd).is_ok()
        } else {
            false
        }
    }

    pub fn reset_live_playback(&mut self) {
        let pic = self.config.picture.active_grade();
        self.live_playback = LivePlaybackControlState {
            sub_visible: self.config.subtitles_enabled,
            pip_mode: self.config.auto_pip,
            ontop: self.config.auto_pip,
            fullscreen: self.config.auto_fullscreen && !self.config.auto_pip,
            brightness: pic.brightness,
            contrast: pic.contrast,
            saturation: pic.saturation,
            gamma: pic.gamma,
            hue: pic.hue,
            sharpness: pic.sharpness,
            ..Default::default()
        };
    }

    pub fn apply_live_ipc_event(&mut self, ev: LiveIpcEvent) {
        match ev {
            LiveIpcEvent::Paused(v) => self.live_playback.paused = v,
            LiveIpcEvent::Volume(v) => self.live_playback.volume = v,
            LiveIpcEvent::Muted(v) => self.live_playback.muted = v,
            LiveIpcEvent::Speed(v) => self.live_playback.speed = v,
            LiveIpcEvent::AudioId(v) => self.live_playback.active_aid = v,
            LiveIpcEvent::SubId(v) => self.live_playback.active_sid = v,
            LiveIpcEvent::SubVisible(v) => self.live_playback.sub_visible = v,
            LiveIpcEvent::SubDelay(s) => self.live_playback.sub_delay_ms = (s * 1000.0).round() as i32,
            LiveIpcEvent::AudioDelay(s) => {
                self.live_playback.audio_delay_ms = (s * 1000.0).round() as i32;
            }
            LiveIpcEvent::OnTop(v) => self.live_playback.ontop = v,
            LiveIpcEvent::LoopVideo(v) => self.live_playback.loop_video = v,
            LiveIpcEvent::Brightness(v) => {
                if !self.live_playback.picture_bypass {
                    self.live_playback.brightness = v;
                }
            }
            LiveIpcEvent::Contrast(v) => {
                if !self.live_playback.picture_bypass {
                    self.live_playback.contrast = v;
                }
            }
            LiveIpcEvent::Saturation(v) => {
                if !self.live_playback.picture_bypass {
                    self.live_playback.saturation = v;
                }
            }
            LiveIpcEvent::Gamma(v) => {
                if !self.live_playback.picture_bypass {
                    self.live_playback.gamma = v;
                }
            }
            LiveIpcEvent::Hue(v) => {
                if !self.live_playback.picture_bypass {
                    self.live_playback.hue = v;
                }
            }
            LiveIpcEvent::Sharpness(v) => {
                if !self.live_playback.picture_bypass {
                    self.live_playback.sharpness = v;
                }
            }
            LiveIpcEvent::VideoZoom(z) => self.live_playback.video_zoom = z,
            LiveIpcEvent::Panscan(p) => self.live_playback.panscan = p,
            LiveIpcEvent::AspectOverride(a) => self.live_playback.aspect_override = a,
            LiveIpcEvent::VideoRotate(deg) => self.live_playback.video_rotate = deg,
            LiveIpcEvent::MirrorX(m) => self.live_playback.mirror_x = m,
            LiveIpcEvent::Fullscreen(f) => {
                self.live_playback.fullscreen = f;
                if f {
                    self.live_playback.pip_mode = false;
                }
            }
            LiveIpcEvent::PitchCorrection(p) => self.live_playback.pitch_correction = p,
            LiveIpcEvent::AbLoopA(a) => self.live_playback.ab_loop_a = a,
            LiveIpcEvent::AbLoopB(b) => self.live_playback.ab_loop_b = b,
            LiveIpcEvent::CacheBytes(b) => self.live_playback.cache_bytes = b,
            LiveIpcEvent::LiveCacheTelemetry {
                cache_ahead_secs,
                seekable_end_secs,
            } => {
                self.live_playback.cache_ahead_secs = cache_ahead_secs;
                self.live_playback.seekable_end_secs = seekable_end_secs;
            }
            LiveIpcEvent::DroppedFrames(d) => self.live_playback.dropped_frames = d,
            LiveIpcEvent::PausedForCache(v) => {
                if v && !self.live_playback.paused_for_cache {
                    let pos = self.active_playback_pos;
                    self.live_playback
                        .telemetry_history
                        .push_event(pos, "STALL", "Rebuffering / paused for cache");
                }
                self.live_playback.paused_for_cache = v;
            }
            LiveIpcEvent::CacheSpeed(bps) => self.live_playback.cache_speed_bps = bps,
            LiveIpcEvent::BufferingPct(pct) => self.live_playback.buffering_pct = pct,
            LiveIpcEvent::VfFps(fps) => self.live_playback.estimated_vf_fps = fps,
            LiveIpcEvent::VideoBitrateBps(bps) => {
                self.live_playback.video_bitrate_kbps = bps / 1000.0;
            }
            LiveIpcEvent::AudioBitrateBps(bps) => {
                self.live_playback.audio_bitrate_kbps = bps / 1000.0;
            }
            LiveIpcEvent::AvSyncSecs(secs) => {
                self.live_playback.avsync_ms = secs * 1000.0;
            }
            LiveIpcEvent::HwdecCurrent(s) => self.live_playback.hwdec_current = s,
            LiveIpcEvent::VideoCodec(s) => self.live_playback.video_codec_live = s,
            LiveIpcEvent::VideoWidth(w) => self.live_playback.decoded_width = w,
            LiveIpcEvent::VideoHeight(h) => self.live_playback.decoded_height = h,
            LiveIpcEvent::ColorMatrix(m) => self.live_playback.colormatrix = m,
            LiveIpcEvent::HdrSigPeak(p) => self.live_playback.hdr_sig_peak = p,
            LiveIpcEvent::DelayedFrames(d) => self.live_playback.delayed_frames = d,
            LiveIpcEvent::AudioSampleRate(sr) => self.live_playback.audio_samplerate = sr,
            LiveIpcEvent::AudioChannels(ch) => self.live_playback.audio_channels = ch,
            LiveIpcEvent::Tracks { audio, sub } => {
                if let Some(sel) = audio.iter().find(|t| t.selected) {
                    self.live_playback.active_aid = Some(sel.id);
                }
                if let Some(sel) = sub.iter().find(|t| t.selected) {
                    self.live_playback.active_sid = Some(sel.id);
                }
                self.live_playback.audio_tracks = audio;
                self.live_playback.sub_tracks = sub;
            }
        }
        self.record_telemetry_sample_if_due();
    }

    pub fn log_studio_event(&mut self, kind: &'static str, summary: impl Into<String>) {
        self.live_playback
            .telemetry_history
            .push_event(self.active_playback_pos, kind, summary);
    }

    pub fn record_telemetry_sample_if_due(&mut self) {
        if !self.has_live_ipc() {
            return;
        }
        let due = self
            .live_playback
            .telemetry_history
            .last_sampled_at
            .map(|t| t.elapsed() >= Duration::from_millis(900))
            .unwrap_or(true);
        if !due {
            return;
        }
        let net_kbps = (self.live_playback.cache_speed_bps as f64 * 8.0) / 1000.0;
        let vid_kbps = if self.live_playback.video_bitrate_kbps > 0.0 {
            self.live_playback.video_bitrate_kbps
        } else {
            self.live_playback.current_stream_tbr_kbps
        };
        let ahead = self.live_playback.cache_ahead_secs;
        let fps = self.live_playback.estimated_vf_fps;
        let avsync = self.live_playback.avsync_ms;
        let lag = if self.live_playback.is_live_stream {
            self.live_playback.live_edge_lag_secs(self.active_playback_pos)
        } else {
            0.0
        };
        let stalled = self.live_playback.paused_for_cache;
        self.live_playback.telemetry_history.push_sample(
            net_kbps,
            vid_kbps,
            ahead,
            fps,
            avsync,
            lag,
            stalled,
        );
        self.live_playback.telemetry_history.last_sampled_at = Some(Instant::now());
    }

    /// Pick matching audio track ID from live `mpv` `track-list` for Original vs. Dubbed audio.
    pub fn find_audio_track_for_dub(&self, want_dub: bool) -> Option<i64> {
        let tracks = &self.live_playback.audio_tracks;
        if tracks.len() < 2 {
            return None;
        }
        if want_dub {
            let pref_lang = self.config.dub_lang.to_lowercase();
            tracks
                .iter()
                .find(|t| {
                    t.is_dubbed
                        && t.lang
                            .as_deref()
                            .map(|l| l.to_lowercase().starts_with(&pref_lang))
                            .unwrap_or(false)
                })
                .or_else(|| tracks.iter().find(|t| t.is_dubbed))
                .or_else(|| {
                    tracks.iter().find(|t| {
                        t.lang
                            .as_deref()
                            .map(|l| l.to_lowercase().starts_with(&pref_lang))
                            .unwrap_or(false)
                    })
                })
                .map(|t| t.id)
        } else {
            tracks
                .iter()
                .find(|t| t.is_original && !t.is_dubbed)
                .or_else(|| tracks.iter().find(|t| !t.is_dubbed))
                .map(|t| t.id)
        }
    }

    #[allow(dead_code)]
    pub fn active_cursor(&self) -> usize {
        self.category_cursors[self.active_category]
    }

    pub fn active_shorts_feed_key(&self) -> &'static str {
        self.shorts_source.feed_key()
    }

    pub fn active_feed_key_for(&self, cat: Category) -> &'static str {
        if cat == Category::Shorts {
            self.active_shorts_feed_key()
        } else {
            cat.feed_key()
        }
    }

    pub fn sync_active_shorts_list(&mut self) {
        let items = match self.shorts_source {
            ShortsSource::Suggested => self.shorts_suggested.clone(),
            ShortsSource::Subscribed => self.shorts_subscribed.clone(),
        };
        if let Some(idx) = self
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Shorts)
        {
            let new_len = items.len();
            self.categories[idx].1 = items;
            if self.category_cursors[idx] >= new_len {
                self.category_cursors[idx] = new_len.saturating_sub(1);
            }
            if self.grid_scroll_rows[idx] >= new_len {
                self.grid_scroll_rows[idx] = 0;
            }
        }
    }

    pub fn current_filtered_videos(&self) -> Vec<VideoItem> {
        let (cat, raw_list) = if self.active_category < self.categories.len() {
            (self.categories[self.active_category].0, &self.categories[self.active_category].1)
        } else {
            return Vec::new();
        };

        if cat == Category::ContinueWatching && self.sort_order == SortOrder::Default {
            return raw_list.clone();
        }

        let mut filtered: Vec<VideoItem> = raw_list
            .iter()
            .filter(|v| {
                if cat == Category::Recommended && (self.is_video_watched(&v.id) || v.is_shorts()) {
                    return false;
                }
                if cat == Category::Shorts {
                    return true;
                }
                match self.content_filter {
                    ContentFilter::All => true,
                    ContentFilter::Longform => v.duration.map(|d| d >= 300).unwrap_or(true),
                    ContentFilter::Shorts => v.duration.map(|d| d <= 60).unwrap_or(false),
                }
            })
            .cloned()
            .collect();

        match self.sort_order {
            SortOrder::Default => {}
            SortOrder::Newest => filtered.sort_by_key(|a| std::cmp::Reverse(a.timestamp)),
            SortOrder::DurationLongest => filtered.sort_by_key(|a| std::cmp::Reverse(a.duration)),
            SortOrder::DurationShortest => filtered.sort_by_key(|a| a.duration),
            SortOrder::ChannelAZ => filtered.sort_by(|a, b| {
                a.display_uploader()
                    .to_lowercase()
                    .cmp(&b.display_uploader().to_lowercase())
            }),
        }
        filtered
    }

    pub fn is_video_watched(&self, video_id: &str) -> bool {
        let clean = video_id.trim_start_matches("RD");
        self.watched_ids.contains(video_id) || self.watched_ids.contains(clean)
    }

    pub fn prune_watched_from_recommended(&mut self) {
        let watched = self.watched_ids.clone();
        if let Some(idx) = self
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Recommended)
        {
            self.categories[idx]
                .1
                .retain(|v| !v.is_shorts() && !watched.contains(&v.id) && !watched.contains(v.clean_id()));
            let new_len = self.categories[idx].1.len();
            if self.category_cursors[idx] >= new_len {
                self.category_cursors[idx] = new_len.saturating_sub(1);
            }
        }
    }

    pub fn mark_video_watched(&mut self, video: &VideoItem) {
        let clean = video.clean_id().to_string();
        self.watched_ids.insert(video.id.clone());
        self.watched_ids.insert(clean.clone());

        if let Some((_, hist)) = self
            .categories
            .iter_mut()
            .find(|(c, _)| *c == Category::History)
        {
            hist.retain(|v| v.clean_id() != clean);
            hist.insert(0, video.clone());
        }

        self.prune_watched_from_recommended();
    }

    pub fn is_in_continue_watching(&self, video_id: &str) -> bool {
        let clean = video_id.trim_start_matches("RD");
        self.continue_watching_meta.contains_key(clean)
            || self
                .categories
                .iter()
                .find(|(c, _)| *c == Category::ContinueWatching)
                .map(|(_, list)| list.iter().any(|v| v.clean_id() == clean))
                .unwrap_or(false)
    }

    pub fn current_home_video(&self) -> Option<VideoItem> {
        let list = self.current_filtered_videos();
        let idx = self.category_cursors[self.active_category].min(list.len().saturating_sub(1));
        list.get(idx).cloned()
    }

    pub fn reset_ticker(&mut self) {
        let now = Instant::now();
        self.ticker_step = 0;
        self.ticker_focus_started_at = now;
        self.ticker_last_step_at = now;
    }

    /// Related items helper (retained for tests / up-next resolution).
    #[allow(dead_code)]
    pub fn detail_related_entries(&self) -> Vec<(&'static str, VideoItem)> {
        let Some(ref cur) = self.detail_video else {
            return Vec::new();
        };
        let cur_id = cur.clean_id();
        let cur_uploader = cur.display_uploader();
        let mut seen = HashSet::new();
        seen.insert(cur_id.to_string());
        let mut out = Vec::new();

        if let Some((_, cw_items)) = self
            .categories
            .iter()
            .find(|(c, _)| *c == Category::ContinueWatching)
        {
            for v in cw_items {
                if seen.insert(v.clean_id().to_string()) {
                    out.push(("QUEUE", v.clone()));
                    if out.len() >= 3 {
                        break;
                    }
                }
            }
        }

        for v in self.all_known_videos() {
            let same_cid = cur.channel_id.is_some() && v.channel_id == cur.channel_id;
            let same_up = !cur_uploader.eq_ignore_ascii_case("Unknown")
                && v.display_uploader().eq_ignore_ascii_case(cur_uploader);
            if (same_cid || same_up) && seen.insert(v.clean_id().to_string()) {
                out.push(("CH", v));
                if out.len() >= 8 {
                    break;
                }
            }
        }
        out
    }

    pub fn same_creator_videos(&self, video: &VideoItem, limit: usize) -> Vec<VideoItem> {
        let cur_id = video.clean_id();
        let cur_uploader = video.display_uploader();
        let mut seen = HashSet::new();
        seen.insert(cur_id.to_string());
        let mut out = Vec::new();
        for v in self.all_known_videos() {
            let same_cid = video.channel_id.is_some() && v.channel_id == video.channel_id;
            let same_up = !cur_uploader.eq_ignore_ascii_case("Unknown")
                && v.display_uploader().eq_ignore_ascii_case(cur_uploader);
            if (same_cid || same_up) && seen.insert(v.clean_id().to_string()) {
                out.push(v);
                if out.len() >= limit {
                    break;
                }
            }
        }
        out
    }

    pub fn detail_up_next_videos(&self, limit: usize) -> Vec<VideoItem> {
        self.detail_related_entries()
            .into_iter()
            .map(|(_, v)| v)
            .take(limit)
            .collect()
    }

    pub fn build_comment_tree(&self) -> Vec<CommentTreeRow> {
        if self.detail_comments.is_empty() {
            return Vec::new();
        }
        let mut root_indices: Vec<usize> = (0..self.detail_comments.len()).collect();
        if self.detail_comment_sort == CommentSort::Newest {
            root_indices.sort_by_key(|&idx| {
                let c = &self.detail_comments[idx];
                (usize::from(!c.is_self), comment_age_rank(&c.published_text), idx)
            });
        }

        let mut rows = Vec::new();
        for r_idx in root_indices {
            let root = &self.detail_comments[r_idx];
            let has_root_children = !root.replies.is_empty()
                || root.reply_count > 0
                || root.replies_continuation.is_some();
            let root_child_cnt = subtree_descendant_count(&root.replies, root.reply_count);
            rows.push(CommentTreeRow {
                comment: root.clone(),
                depth: 0,
                parent_id: None,
                root_id: root.id.clone(),
                root_idx: r_idx,
                is_last_sibling: false,
                rail_active: Vec::new(),
                has_children: has_root_children,
                child_count: root_child_cnt,
                collapsed: !root.replies_expanded,
            });
            if !root.replies_expanded || root.replies.is_empty() {
                continue;
            }

            let children_map = build_reply_adjacency(&root.replies);
            let direct_children = &children_map.root_children;
            for (pos, &child_i) in direct_children.iter().enumerate() {
                let is_last = pos + 1 == direct_children.len();
                push_reply_subtree(
                    &root.replies,
                    &children_map.by_reply,
                    child_i,
                    root.id.clone(),
                    root.id.clone(),
                    r_idx,
                    1,
                    is_last,
                    Vec::new(),
                    &self.detail_comment_collapsed,
                    &mut rows,
                );
            }
        }
        rows
    }

    pub fn insert_comment_reply(
        &mut self,
        root_id: &str,
        parent_id: &str,
        mut reply: VideoComment,
    ) -> bool {
        reply.parent_id = Some(parent_id.to_string());
        self.detail_comment_collapsed.remove(root_id);
        self.detail_comment_collapsed.remove(parent_id);
        if let Some(root) = self.detail_comments.iter_mut().find(|c| c.id == root_id) {
            root.replies_expanded = true;
            root.reply_count = root.reply_count.saturating_add(1);
            if parent_id == root_id {
                root.replies.insert(0, reply);
            } else if let Some(pos) = root.replies.iter().position(|r| r.id == parent_id) {
                root.replies.insert(pos + 1, reply);
            } else {
                root.replies.push(reply);
            }
            return true;
        }
        false
    }

    pub fn active_selected_video(&self) -> Option<VideoItem> {
        if self.overlay == ActiveOverlay::Search {
            let combined = self.combined_search_items();
            return combined.get(self.search_cursor).cloned();
        }
        match self.screen {
            Screen::Detail => self.detail_video.clone(),
            Screen::Channel => self.channel_videos.get(self.channel_cursor).cloned(),
            Screen::Home => self.current_home_video(),
        }
    }

    pub fn combined_search_items(&self) -> Vec<VideoItem> {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        let parsed = parse_search_query(&self.search_query);
        let eff_type = parsed.force_type.unwrap_or(self.search_type_filter);
        let query_wants_shorts = eff_type == SearchTypeFilter::Shorts
            || parsed
                .tokens
                .iter()
                .any(|t| t == "short" || t == "shorts");
        let q_joined = parsed.tokens.join(" ");

        let sub_items: Vec<&VideoItem> = self
            .categories
            .iter()
            .find(|(c, _)| *c == Category::Subscriptions)
            .map(|(_, list)| list.iter().collect())
            .unwrap_or_default();
        let sub_uploaders: HashSet<String> = sub_items
            .iter()
            .map(|v| v.display_uploader().to_lowercase())
            .collect();

        let mut pool: Vec<VideoItem> = Vec::new();
        let mut seen = HashSet::new();
        let mut push_dedupe = |v: &VideoItem| {
            let cid = v.clean_id().to_string();
            if seen.insert(cid) {
                pool.push(v.clone());
            }
        };

        match self.search_scope {
            SearchScope::All => {
                for v in &self.search_results {
                    push_dedupe(v);
                }
                if eff_type == SearchTypeFilter::Shorts
                    || query_wants_shorts
                    || self.search_results.len() < 8
                {
                    for v in &self.search_shorts {
                        push_dedupe(v);
                    }
                }
                for v in &self.search_local_results {
                    push_dedupe(v);
                }
            }
            SearchScope::LiveYt => {
                for v in &self.search_results {
                    push_dedupe(v);
                }
                if eff_type == SearchTypeFilter::Shorts || query_wants_shorts {
                    for v in &self.search_shorts {
                        push_dedupe(v);
                    }
                }
            }
            SearchScope::Subscribed => {
                for v in sub_items.into_iter().chain(self.shorts_subscribed.iter()) {
                    if parsed.tokens.is_empty() || video_matches_parsed(v, &parsed) {
                        push_dedupe(v);
                    }
                }
            }
            SearchScope::Cached => {
                for v in &self.search_local_results {
                    if parsed.tokens.is_empty() || video_matches_parsed(v, &parsed) {
                        push_dedupe(v);
                    }
                }
            }
        }

        // Apply Search filters + inline operator constraints
        let min_ts = self.search_date_filter.min_timestamp(now);
        let hide_watched = self.search_hide_watched || parsed.only_unwatched;
        pool.retain(|v| {
            if hide_watched && self.is_video_watched(&v.id) {
                return false;
            }
            match eff_type {
                SearchTypeFilter::All => {}
                SearchTypeFilter::Videos => {
                    if v.is_shorts() {
                        return false;
                    }
                }
                SearchTypeFilter::Shorts => {
                    if !v.is_shorts() {
                        return false;
                    }
                }
                SearchTypeFilter::Live => {
                    if !v.is_live_stream() {
                        return false;
                    }
                }
            }
            if parsed.only_live && !v.is_live_stream() {
                return false;
            }
            if !parsed.exact_phrases.is_empty() {
                let hay = format!(
                    "{} {} {}",
                    v.title,
                    v.display_uploader(),
                    v.search_snippet.as_deref().unwrap_or("")
                )
                .to_lowercase();
                if !parsed.exact_phrases.iter().all(|p| hay.contains(p)) {
                    return false;
                }
            }
            if !parsed.excluded_terms.is_empty() {
                let hay = format!("{} {}", v.title, v.display_uploader()).to_lowercase();
                if parsed.excluded_terms.iter().any(|ex| hay.contains(ex)) {
                    return false;
                }
            }
            if let Some(ref ch_req) = parsed.channel_filter {
                if !v.display_uploader().to_lowercase().contains(ch_req) {
                    return false;
                }
            }
            if !self.search_duration_filter.matches(v.duration) {
                return false;
            }
            if let Some(cutoff) = min_ts {
                if let Some(ts) = v.timestamp {
                    if ts < cutoff {
                        return false;
                    }
                }
            }
            true
        });

        // Score with Hybrid Reciprocal Rank Fusion (YouTube rank + text/fuzzy/quality priors)
        let matcher = SkimMatcherV2::default();
        let mut scored: Vec<(i64, VideoItem)> = pool
            .into_iter()
            .map(|v| {
                let s = search_relevance_score(
                    &v,
                    &q_joined,
                    &parsed,
                    &sub_uploaders,
                    &self.progress_map,
                    query_wants_shorts,
                    &matcher,
                    now,
                );
                (s, v)
            })
            .collect();

        match self.search_sort_order {
            SearchSortOrder::Relevance => {
                scored.sort_by(|a, b| {
                    b.0.cmp(&a.0).then_with(|| {
                        a.1.yt_rank
                            .unwrap_or(usize::MAX)
                            .cmp(&b.1.yt_rank.unwrap_or(usize::MAX))
                    })
                });
            }
            SearchSortOrder::Newest => {
                scored.sort_by(|a, b| {
                    b.1.timestamp
                        .unwrap_or(0)
                        .cmp(&a.1.timestamp.unwrap_or(0))
                        .then_with(|| b.0.cmp(&a.0))
                });
            }
            SearchSortOrder::Views => {
                scored.sort_by(|a, b| {
                    b.1.view_count
                        .unwrap_or(0)
                        .cmp(&a.1.view_count.unwrap_or(0))
                        .then_with(|| b.0.cmp(&a.0))
                });
            }
            SearchSortOrder::Duration => {
                scored.sort_by(|a, b| {
                    b.1.duration
                        .unwrap_or(0)
                        .cmp(&a.1.duration.unwrap_or(0))
                        .then_with(|| b.0.cmp(&a.0))
                });
            }
        }

        scored.into_iter().map(|(_, v)| v).collect()
    }

    pub fn push_search_history(&mut self, query: &str) {
        let q = query.trim().to_string();
        if q.len() < 2 {
            return;
        }
        self.search_history.retain(|h| !h.eq_ignore_ascii_case(&q));
        self.search_history.insert(0, q);
        self.search_history.truncate(8);
    }

    pub fn search_params_token(&self) -> Option<String> {
        let parsed = parse_search_query(&self.search_query);
        let eff_type = parsed.force_type.unwrap_or(self.search_type_filter);
        let sort_code = match self.search_sort_order {
            SearchSortOrder::Relevance => 0,
            SearchSortOrder::Newest => 2,
            SearchSortOrder::Views => 3,
            SearchSortOrder::Duration => 0,
        };
        let date_code = match self.search_date_filter {
            SearchDateFilter::Any => 0,
            SearchDateFilter::Hour => 1,
            SearchDateFilter::Today => 2,
            SearchDateFilter::Week => 3,
            SearchDateFilter::Month => 4,
            SearchDateFilter::Year => 5,
        };
        let dur_code = match self.search_duration_filter {
            SearchDurationFilter::Any => 0,
            SearchDurationFilter::Short => 1,
            SearchDurationFilter::Medium => 3,
            SearchDurationFilter::Long => 2,
        };
        let videos_only = eff_type == SearchTypeFilter::Videos;
        let is_live = eff_type == SearchTypeFilter::Live || parsed.only_live;
        let is_4k = self.search_hd_4k_only || parsed.only_4k;
        let is_hd = parsed.only_hd;
        crate::api::innertube::encode_search_params_ex(
            sort_code,
            date_code,
            dur_code,
            videos_only,
            is_hd,
            is_4k,
            is_live,
        )
    }

    pub fn all_known_videos(&self) -> Vec<VideoItem> {
        let mut seen = HashSet::new();
        let mut all = Vec::new();
        for (_, items) in &self.categories {
            for v in items {
                if seen.insert(v.id.clone()) {
                    all.push(v.clone());
                }
            }
        }
        for v in self.shorts_suggested.iter().chain(self.shorts_subscribed.iter()) {
            if seen.insert(v.id.clone()) {
                all.push(v.clone());
            }
        }
        all
    }

    pub fn insert_loaded_thumbnail(
        &mut self,
        video_id: String,
        img: image::DynamicImage,
    ) {
        self.loading_thumbs.remove(&video_id);
        self.failed_thumbs.remove(&video_id);

        let ambi = extract_ambilight_palette(&img, self.base_theme());
        self.ambilight_cache.insert(video_id.clone(), ambi);
        if self.raw_thumbs.contains_key(&video_id) {
            if let Some(pos) = self.raw_thumbs_order.iter().position(|x| x == &video_id) {
                self.raw_thumbs_order.remove(pos);
            }
        } else if self.raw_thumbs.len() >= 240 {
            for _ in 0..40 {
                if let Some(evicted) = self.raw_thumbs_order.pop_front() {
                    self.raw_thumbs.remove(&evicted);
                    self.image_cache.retain(|(id, _, _), _| id != &evicted);
                    self.image_cache_order.retain(|(id, _, _)| id != &evicted);
                } else {
                    break;
                }
            }
        }
        self.raw_thumbs_order.push_back(video_id.clone());
        self.raw_thumbs.insert(video_id.clone(), img);
        self.image_cache.retain(|(id, _, _), _| id != &video_id);
        self.image_cache_order.retain(|(id, _, _)| id != &video_id);
    }

    pub fn queue_thumb_if_needed(&mut self, id: &str, url: &str) {
        self.queue_thumb_with_priority(id, url, ThumbPriority::Visible);
    }

    pub fn queue_comment_media(&mut self, key: &str, url: &str) {
        if key.is_empty()
            || url.is_empty()
            || self.comment_media_cache.contains_key(key)
            || self.loading_comment_media.contains(key)
            || self.failed_comment_media.contains(key)
            || self.pending_comment_media.iter().any(|(k, _)| k == key)
        {
            return;
        }
        self.pending_comment_media
            .push((key.to_string(), url.to_string()));
    }

    pub fn queue_thumb_with_priority(&mut self, id: &str, url: &str, priority: ThumbPriority) {
        if id.is_empty() || self.raw_thumbs.contains_key(id) || self.loading_thumbs.contains(id) {
            return;
        }
        if let Some(fail) = self.failed_thumbs.get(id) {
            if fail.attempts >= 3 || Instant::now() < fail.next_retry_at {
                return;
            }
        }
        if let Some(existing) = self
            .pending_thumb_requests
            .iter_mut()
            .find(|req| req.video_id == id)
        {
            if priority < existing.priority {
                existing.priority = priority;
            }
            if existing.url.is_empty() && !url.is_empty() {
                existing.url = url.to_string();
            }
            return;
        }
        self.pending_thumb_requests.push(ThumbRequest {
            video_id: id.to_string(),
            url: url.to_string(),
            priority,
        });
    }

    pub fn is_thumb_failed(&self, id: &str) -> bool {
        !self.raw_thumbs.contains_key(id)
            && !self.loading_thumbs.contains(id)
            && self
                .failed_thumbs
                .get(id)
                .map(|f| f.attempts >= 3)
                .unwrap_or(false)
    }

    pub fn retry_failed_thumbs(&mut self) {
        self.failed_thumbs.clear();
    }

    pub async fn sync_calendar_tz(&mut self, pool: &sqlx::SqlitePool) {
        self.calendar_tz_offset = local_tz_offset_secs(pool).await;
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs() as i64;
        self.calendar_bounds = compute_week_bounds(now_unix, self.calendar_tz_offset, 0);
        if let Some(today) = self.calendar_bounds.today_day_idx {
            self.calendar_day_cursor = today;
        }
    }
}

fn video_matches_parsed(v: &VideoItem, parsed: &ParsedSearchQuery) -> bool {
    let title_l = v.title.to_lowercase();
    let up_l = v.display_uploader().to_lowercase();
    if let Some(ref ch) = parsed.channel_filter {
        if !up_l.contains(ch) {
            return false;
        }
    }
    if parsed
        .excluded_terms
        .iter()
        .any(|ex| title_l.contains(ex) || up_l.contains(ex))
    {
        return false;
    }
    if parsed.tokens.is_empty() {
        return true;
    }
    parsed
        .tokens
        .iter()
        .all(|t| title_l.contains(t.as_str()) || up_l.contains(t.as_str()))
}

#[allow(clippy::too_many_arguments)]
fn search_relevance_score(
    v: &VideoItem,
    q_joined: &str,
    parsed: &ParsedSearchQuery,
    sub_uploaders: &HashSet<String>,
    progress_map: &HashMap<String, (f64, f64)>,
    query_wants_shorts: bool,
    matcher: &SkimMatcherV2,
    now_unix: i64,
) -> i64 {
    // 1. Preserve YouTube's ML Search ranking order via smooth Reciprocal Rank prior
    let yt_rank_prior = v
        .yt_rank
        .map(|r| (180.0 / (1.0 + 0.14 * r as f64)).round() as i64)
        .unwrap_or(0);

    if q_joined.is_empty() && parsed.channel_filter.is_none() && parsed.exact_phrases.is_empty() {
        let rec = v.timestamp.unwrap_or(0) / 86_400;
        let views = v.view_count.unwrap_or(0).min(10_000_000) as i64 / 50_000;
        return yt_rank_prior + rec + views;
    }

    let title_l = v.title.to_lowercase();
    let up_l = v.display_uploader().to_lowercase();
    let mut score = yt_rank_prior;

    if !q_joined.is_empty() {
        if title_l.contains(q_joined) {
            score += 65;
            if title_l.starts_with(q_joined) {
                score += 18;
            }
        }
        if up_l == q_joined {
            score += 55;
        } else if up_l.contains(q_joined) {
            score += 32;
        }
        if let Some(fuzzy) = matcher.fuzzy_match(&title_l, q_joined) {
            score += (fuzzy / 4).clamp(0, 42);
        }
    }

    for phrase in &parsed.exact_phrases {
        if title_l.contains(phrase.as_str()) {
            score += 80;
        }
    }

    let matched_tokens = parsed
        .tokens
        .iter()
        .filter(|t| title_l.contains(t.as_str()) || up_l.contains(t.as_str()))
        .count();
    score += (matched_tokens as i64) * 14;
    if !parsed.tokens.is_empty() && matched_tokens == parsed.tokens.len() {
        score += 30;
    }

    if let Some(ref ch) = parsed.channel_filter {
        if up_l == *ch {
            score += 70;
        } else if up_l.contains(ch.as_str()) {
            score += 45;
        }
    }

    if sub_uploaders.contains(&up_l) {
        score += 15;
    }
    if v.channel_verified {
        score += 8;
    }
    if progress_map.get(v.clean_id()).map(|(p, _)| *p > 5.0).unwrap_or(false) {
        score += 6;
    }
    if let Some(ts) = v.timestamp {
        if now_unix.saturating_sub(ts) <= 7 * 86_400 {
            score += 8;
        }
    }
    if let Some(vw) = v.view_count {
        if vw > 0 {
            score += ((vw as f64).log10() * 1.4).round() as i64;
        }
    }

    // Penalize metadata-less items & unintended Shorts hijackers
    if v.display_uploader() == "Unknown" {
        score -= 25;
    }
    if v.is_shorts() && !query_wants_shorts {
        score -= 45;
    }

    score
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchScope {
    All,
    LiveYt,
    Subscribed,
    Cached,
}

impl SearchScope {
    pub const ALL: [Self; 4] = [Self::All, Self::LiveYt, Self::Subscribed, Self::Cached];
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::LiveYt,
            Self::LiveYt => Self::Subscribed,
            Self::Subscribed => Self::Cached,
            Self::Cached => Self::All,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::LiveYt => "Live YT",
            Self::Subscribed => "Subscribed",
            Self::Cached => "Cached",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchTypeFilter {
    All,
    Videos,
    Shorts,
    Live,
}

impl SearchTypeFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Videos, Self::Shorts, Self::Live];
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Videos,
            Self::Videos => Self::Shorts,
            Self::Shorts => Self::Live,
            Self::Live => Self::All,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Videos => "Videos",
            Self::Shorts => "Shorts",
            Self::Live => "Live",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDateFilter {
    Any,
    Hour,
    Today,
    Week,
    Month,
    Year,
}

impl SearchDateFilter {
    pub const ALL: [Self; 6] = [
        Self::Any,
        Self::Hour,
        Self::Today,
        Self::Week,
        Self::Month,
        Self::Year,
    ];
    pub const fn next(self) -> Self {
        match self {
            Self::Any => Self::Hour,
            Self::Hour => Self::Today,
            Self::Today => Self::Week,
            Self::Week => Self::Month,
            Self::Month => Self::Year,
            Self::Year => Self::Any,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Any => "Any",
            Self::Hour => "Hour",
            Self::Today => "Today",
            Self::Week => "Week",
            Self::Month => "Mo",
            Self::Year => "Yr",
        }
    }
    pub fn min_timestamp(self, now: i64) -> Option<i64> {
        match self {
            Self::Any => None,
            Self::Hour => Some(now - 3_600),
            Self::Today => Some(now - 86_400),
            Self::Week => Some(now - 7 * 86_400),
            Self::Month => Some(now - 30 * 86_400),
            Self::Year => Some(now - 365 * 86_400),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDurationFilter {
    Any,
    Short,
    Medium,
    Long,
}

impl SearchDurationFilter {
    pub const ALL: [Self; 4] = [Self::Any, Self::Short, Self::Medium, Self::Long];
    pub const fn next(self) -> Self {
        match self {
            Self::Any => Self::Short,
            Self::Short => Self::Medium,
            Self::Medium => Self::Long,
            Self::Long => Self::Any,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Any => "Any",
            Self::Short => "<4m",
            Self::Medium => "4-20m",
            Self::Long => ">20m",
        }
    }
    pub fn matches(self, duration: Option<u64>) -> bool {
        let Some(d) = duration else {
            return self == Self::Any;
        };
        match self {
            Self::Any => true,
            Self::Short => d < 240,
            Self::Medium => (240..=1200).contains(&d),
            Self::Long => d > 1200,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchSortOrder {
    Relevance,
    Newest,
    Views,
    Duration,
}

impl SearchSortOrder {
    pub const ALL: [Self; 4] = [Self::Relevance, Self::Newest, Self::Views, Self::Duration];
    pub const fn next(self) -> Self {
        match self {
            Self::Relevance => Self::Newest,
            Self::Newest => Self::Views,
            Self::Views => Self::Duration,
            Self::Duration => Self::Relevance,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Relevance => "Relevance",
            Self::Newest => "Newest",
            Self::Views => "Views",
            Self::Duration => "Duration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentSort {
    Top,
    Newest,
}

#[derive(Debug, Clone)]
pub struct ReplyTarget {
    pub root_id: String,
    pub parent_id: String,
    pub author: String,
    pub reply_params: Option<String>,
}

#[derive(Debug, Clone)]
pub struct CommentTreeRow {
    pub comment: VideoComment,
    pub depth: usize,
    pub parent_id: Option<String>,
    pub root_id: String,
    pub root_idx: usize,
    pub is_last_sibling: bool,
    pub rail_active: Vec<bool>,
    pub has_children: bool,
    pub child_count: usize,
    pub collapsed: bool,
}

#[derive(Debug, Clone)]
pub struct CommentTranslation {
    pub target_lang: String,
    pub source_lang: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct CommentNodeHitbox {
    pub y0: u16,
    pub y1: u16,
    pub flat_idx: usize,
    pub node_id: String,
    pub fold_rect: Option<Rect>,
    pub reply_rect: Option<Rect>,
    pub translate_rect: Option<Rect>,
}

#[derive(Debug, Clone)]
pub enum DetailHit {
    Play,
    PlayLiveFromStart,
    PlayAudioRadio,
    CycleLiveMode,
    VoteLike,
    VoteDislike,
    PinQueue,
    WatchLater,
    Download,
    OpenChannel(VideoItem),
    SelectUpNext(VideoItem),
    SelectChapter(usize),
    SortComments(CommentSort),
    EditTranslateLang,
    TranslateNode(usize),
    FocusPane(usize),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchHit {
    ApplySuggestion(String),
    SetScope(SearchScope),
    SetType(SearchTypeFilter),
    SetDate(SearchDateFilter),
    SetDuration(SearchDurationFilter),
    SetSort(SearchSortOrder),
    ToggleHideWatched,
    ToggleHd4k,
    ToggleViewMode,
    SetViewMode(SearchViewMode),
    SelectRow(usize),
    ActionPlay,
    ActionDetail,
    ActionPinCw,
    ActionWatchLater,
    ActionChannel,
    OpenSpotlightChannel(usize),
    FilterByChannel(String),
    RetryThumbs,
    ExecuteQueryNow,
    FocusQuery,
    DirectJumpDetail(String),
    DirectJumpPlay(String),
    CloseSearch,
    LoadMore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualToggle {
    SuperRes,
    GovernorMode,
    OrganicGrain,
    CodecPriority,
    Hwdec,
    HdrComputePeak,
    ToneMap,
    GlslStage,
    Deband,
    GpuScaler,
    Motion,
    DemuxerCache,
}

impl ManualToggle {
    pub const ALL: [Self; 12] = [
        Self::SuperRes,
        Self::GovernorMode,
        Self::OrganicGrain,
        Self::CodecPriority,
        Self::Hwdec,
        Self::HdrComputePeak,
        Self::ToneMap,
        Self::GlslStage,
        Self::Deband,
        Self::GpuScaler,
        Self::Motion,
        Self::DemuxerCache,
    ];
}

pub const TELEMETRY_CAPACITY: usize = 60;
pub const SESSION_EVENT_CAPACITY: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StudioPage {
    #[default]
    Deck,
    StreamNet,
    MasteringGpu,
    AudioSync,
    CaptionsSkip,
    LiveTelemetry,
}

#[allow(dead_code)]
impl StudioPage {
    pub const ALL: [Self; 6] = [
        Self::Deck,
        Self::StreamNet,
        Self::MasteringGpu,
        Self::AudioSync,
        Self::CaptionsSkip,
        Self::LiveTelemetry,
    ];

    pub const fn index(self) -> usize {
        self as usize
    }

    pub const fn shortcut(self) -> char {
        match self {
            Self::Deck => '1',
            Self::StreamNet => '2',
            Self::MasteringGpu => '3',
            Self::AudioSync => '4',
            Self::CaptionsSkip => '5',
            Self::LiveTelemetry => '6',
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Deck => "Deck",
            Self::StreamNet => "Stream & Net",
            Self::MasteringGpu => "Mastering & GPU",
            Self::AudioSync => "Audio & EQ",
            Self::CaptionsSkip => "AdBlock & Subs",
            Self::LiveTelemetry => "Live & Telemetry",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Deck => "Deck",
            Self::StreamNet => "Stream",
            Self::MasteringGpu => "Master",
            Self::AudioSync => "AudioEQ",
            Self::CaptionsSkip => "AdBlock",
            Self::LiveTelemetry => "Live",
        }
    }

    pub const fn next(self) -> Self {
        Self::ALL[(self.index() + 1) % 6]
    }

    pub const fn prev(self) -> Self {
        Self::ALL[(self.index() + 5) % 6]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentProfile {
    Cinema24p,
    Sports60p,
    Anime2x,
    CrispCode,
    CoolBattery,
}

#[allow(dead_code)]
impl ContentProfile {
    pub const ALL: [Self; 5] = [
        Self::Cinema24p,
        Self::Sports60p,
        Self::Anime2x,
        Self::CrispCode,
        Self::CoolBattery,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Cinema24p => "Cinema 24p (Reference 4K + Cadence)",
            Self::Sports60p => "Sports 60p (VP9 + 120Hz Smooth)",
            Self::Anime2x => "Anime 2x (Anime4K Line Restore)",
            Self::CrispCode => "Crisp Code / Text (Light CAS)",
            Self::CoolBattery => "Cool Battery (Zero Shaders)",
        }
    }

    pub const fn short_label(self) -> &'static str {
        match self {
            Self::Cinema24p => "Cinema 24p",
            Self::Sports60p => "Sports 60p",
            Self::Anime2x => "Anime 2x",
            Self::CrispCode => "Crisp Code",
            Self::CoolBattery => "Cool Battery",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EventLogFilter {
    #[default]
    All,
    Stalls,
    Skips,
    Changes,
}

#[allow(dead_code)]
impl EventLogFilter {
    pub const ALL: [Self; 4] = [Self::All, Self::Stalls, Self::Skips, Self::Changes];

    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Stalls => "Stalls",
            Self::Skips => "Skips",
            Self::Changes => "Changes",
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Stalls,
            Self::Stalls => Self::Skips,
            Self::Skips => Self::Changes,
            Self::Changes => Self::All,
        }
    }

    pub fn matches(self, kind: &str) -> bool {
        match self {
            Self::All => true,
            Self::Stalls => matches!(kind, "STALL" | "ABR"),
            Self::Skips => kind == "SKIP",
            Self::Changes => !matches!(kind, "STALL" | "SKIP"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CaptionsTab {
    #[default]
    Segments,
    Chapters,
    Shield,
}

#[allow(dead_code)]
impl CaptionsTab {
    pub const fn next(self) -> Self {
        match self {
            Self::Segments => Self::Chapters,
            Self::Chapters => Self::Shield,
            Self::Shield => Self::Segments,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct StudioNavState {
    pub page: StudioPage,
    pub pane_focus: usize,
    pub eq_cursor: usize,
    pub audio_eq_band_cursor: usize,
    pub sub_track_scroll: usize,
    pub sponsor_cursor: usize,
    pub sponsor_scroll: usize,
    pub captions_tab: Option<CaptionsTab>,
    pub chapter_cursor: usize,
    pub chapter_scroll: usize,
    pub event_log_scroll: usize,
    pub event_log_filter: EventLogFilter,
}

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub media_pos_secs: f64,
    pub kind: &'static str,
    pub summary: String,
}

#[derive(Debug, Clone, Default)]
pub struct TelemetryHistory {
    pub cache_speed_kbps: VecDeque<f64>,
    pub video_bitrate_kbps: VecDeque<f64>,
    pub cache_ahead_secs: VecDeque<f64>,
    pub vf_fps: VecDeque<f64>,
    pub avsync_ms: VecDeque<f64>,
    pub live_lag_secs: VecDeque<f64>,
    pub stall_samples: VecDeque<bool>,
    pub events: VecDeque<SessionEvent>,
    pub last_sampled_at: Option<Instant>,
}

impl TelemetryHistory {
    fn push_bounded<T>(buf: &mut VecDeque<T>, val: T, cap: usize) {
        if buf.len() >= cap {
            buf.pop_front();
        }
        buf.push_back(val);
    }

    #[allow(clippy::too_many_arguments)]
    pub fn push_sample(
        &mut self,
        cache_speed_kbps: f64,
        video_bitrate_kbps: f64,
        cache_ahead_secs: f64,
        vf_fps: f64,
        avsync_ms: f64,
        live_lag_secs: f64,
        stalled: bool,
    ) {
        Self::push_bounded(&mut self.cache_speed_kbps, cache_speed_kbps.max(0.0), TELEMETRY_CAPACITY);
        Self::push_bounded(&mut self.video_bitrate_kbps, video_bitrate_kbps.max(0.0), TELEMETRY_CAPACITY);
        Self::push_bounded(&mut self.cache_ahead_secs, cache_ahead_secs.max(0.0), TELEMETRY_CAPACITY);
        Self::push_bounded(&mut self.vf_fps, vf_fps.max(0.0), TELEMETRY_CAPACITY);
        Self::push_bounded(&mut self.avsync_ms, avsync_ms, TELEMETRY_CAPACITY);
        Self::push_bounded(&mut self.live_lag_secs, live_lag_secs.max(0.0), TELEMETRY_CAPACITY);
        Self::push_bounded(&mut self.stall_samples, stalled, TELEMETRY_CAPACITY);
    }

    pub fn push_event(
        &mut self,
        media_pos_secs: f64,
        kind: &'static str,
        summary: impl Into<String>,
    ) {
        Self::push_bounded(
            &mut self.events,
            SessionEvent {
                media_pos_secs: media_pos_secs.max(0.0),
                kind,
                summary: summary.into(),
            },
            SESSION_EVENT_CAPACITY,
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SleepTimerMode {
    #[default]
    Off,
    Mins15,
    Mins30,
    Mins45,
    Mins60,
    EndOfVideo,
}

#[allow(dead_code)]
impl SleepTimerMode {
    pub const ALL: [Self; 6] = [
        Self::Off,
        Self::Mins15,
        Self::Mins30,
        Self::Mins45,
        Self::Mins60,
        Self::EndOfVideo,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Off => "Off",
            Self::Mins15 => "15m",
            Self::Mins30 => "30m",
            Self::Mins45 => "45m",
            Self::Mins60 => "60m",
            Self::EndOfVideo => "EOF",
        }
    }

    pub const fn duration_secs(self) -> Option<u64> {
        match self {
            Self::Off | Self::EndOfVideo => None,
            Self::Mins15 => Some(15 * 60),
            Self::Mins30 => Some(30 * 60),
            Self::Mins45 => Some(45 * 60),
            Self::Mins60 => Some(60 * 60),
        }
    }

    pub const fn next(self) -> Self {
        match self {
            Self::Off => Self::Mins15,
            Self::Mins15 => Self::Mins30,
            Self::Mins30 => Self::Mins45,
            Self::Mins45 => Self::Mins60,
            Self::Mins60 => Self::EndOfVideo,
            Self::EndOfVideo => Self::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub enum PlayOptionsHit {
    SelectPage(StudioPage),
    FocusPane(usize),
    FocusSection(usize),
    SelectQuality(usize),
    SelectPlayer(Player),
    CycleCodecPriority,
    SetCodecPriority(crate::config::CodecPriority),
    SetMaxFpsCap(Option<u16>),
    RefreshStreamLadder,
    SetNetworkMode(crate::config::NetworkMode),
    ToggleDub,
    CycleDubLang,
    SelectDubLang(String),
    ToggleMultistreams,
    ToggleLoudnorm,
    SelectAudioEq(crate::config::AudioEqPreset),
    CycleAudioEq,
    AudioEqBandDelta(usize, i8),
    SetAudioEqBand(usize, i8),
    AudioEqPreampDelta(i8),
    ToggleAudioEqBypass,
    SaveCustomAudioEq,
    ResetAudioEq,
    SelectAudioSpatial(crate::config::AudioChannelMode),
    ToggleAudioChannelMode,
    ToggleAudioHighpass,
    AudioDelayDelta(i32),
    SetAudioDelay(i32),
    SetAudioDelayRatio,
    SelectAudioTrack(i64),
    ToggleSubtitles,
    CycleSubStyle,
    SelectSubPreset(SubtitlePreset),
    ToggleSubCollision,
    SetDefaultSubLang(&'static str),
    SetSubFont(&'static str),
    SubFontSizeDelta(i32),
    SubBorderDelta(f32),
    SubMarginYDelta(i32),
    SubShadowDelta(f32),
    SetSubBackdrop(f32),
    SetSubColor(&'static str),
    ReloadSubtitles,
    SetCaptionsTab(CaptionsTab),
    ReScanSponsorBlock,
    UndoLastSponsorSkip,
    SkipActiveSponsorNow,
    ToggleWhitelistChannel,
    RemoveWhitelistChannel(String),
    CycleSponsorCategory(&'static str),
    SetSponsorMinSecs(u8),
    ToggleSponsorOsdToast,
    SubDelayDelta(i32),
    SetSubDelay(i32),
    SelectSubTrack(i64),
    ScrollSubTracks(i8),
    SelectSponsorBlock(SponsorBlockMode),
    ToggleSkipOutro,
    JumpSponsorSegment(f64),
    ScrollSponsorSegments(i8),
    SelectShader(VideoShaderMode),
    SetPerformanceMode(PerformanceMode),
    ApplyContentProfile(ContentProfile),
    ToggleManual(ManualToggle),
    SeedManualFrom(PerformanceMode),
    DownloadNeuralShaders,
    DownloadArtCnnShaders,
    CycleLiveMode,
    SetLiveMode(crate::config::LiveStreamMode),
    CycleNetworkMode,
    ToggleForceIpv4,
    ToggleAutoAbr,
    ToggleLiveCatchup,
    JumpToLiveEdge,
    PlayLiveFromStart,
    LaunchAudioRadio,
    ScrollAudioTracks(i8),
    ScrollEventLog(i8),
    SetEventLogFilter(EventLogFilter),
    ClearEventLog,
    TogglePause,
    SeekDelta(f64),
    SeekRatio(f64),
    FrameStep(i8),
    ChapterStep(i32),
    SetSpeed(f64),
    SpeedDelta(f64),
    TogglePitchCorrection,
    VolumeDelta(i64),
    SetVolume(i64),
    SetVolumeRatio,
    ToggleMute,
    MarkAbLoopA,
    MarkAbLoopB,
    ClearAbLoop,
    TakeScreenshot,
    SetSleepTimer(SleepTimerMode),
    CycleSleepTimer,
    ToggleFullscreen,
    TogglePipMode,
    CyclePipCorner,
    ToggleAutoFullscreen,
    ToggleInfoHud,
    PictureDelta(&'static str, i64),
    SetPictureRatio(&'static str),
    SelectPicturePreset(PicturePreset),
    SaveCustomPicturePreset,
    TogglePictureBypass,
    ResetPicture,
    ZoomDelta(f64),
    SetAspectOverride(&'static str),
    TogglePanscan,
    CycleRotate,
    ToggleMirrorX,
    ResetGeometry,
    ToggleOnTop,
    ToggleLoop,
    LaunchOrRelaunch,
    StopPlayer,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowPlayingHit {
    TogglePause,
    SeekBack,
    SeekFwd,
    ToggleDub,
    SpeedStep(i8),
    VolumeDelta(i64),
    ToggleMute,
    CycleShader,
    CyclePerfMode,
    CycleMotion,
    JumpToLiveEdge,
    CycleLiveMode,
    CycleNetworkMode,
    OpenStudio,
    StopPlayer,
}

#[derive(Debug, Clone)]
pub struct LivePlaybackControlState {
    pub paused: bool,
    pub volume: i64,
    pub muted: bool,
    pub speed: f64,
    pub audio_delay_ms: i32,
    pub sub_delay_ms: i32,
    pub sub_visible: bool,
    pub active_aid: Option<i64>,
    pub active_sid: Option<i64>,
    pub ontop: bool,
    pub fullscreen: bool,
    pub pip_mode: bool,
    pub pip_corner_idx: usize,
    pub player_pid: Option<u32>,
    pub loop_video: bool,
    pub pitch_correction: bool,
    pub ab_loop_a: Option<f64>,
    pub ab_loop_b: Option<f64>,
    pub sleep_timer_mode: SleepTimerMode,
    pub sleep_timer_deadline: Option<Instant>,
    pub brightness: i64,
    pub contrast: i64,
    pub saturation: i64,
    pub gamma: i64,
    pub hue: i64,
    pub sharpness: i64,
    pub picture_bypass: bool,
    pub audio_eq_bypass: bool,
    pub video_zoom: f64,
    pub panscan: f64,
    pub aspect_override: String,
    pub video_rotate: u16,
    pub mirror_x: bool,
    pub cache_bytes: u64,
    pub cache_ahead_secs: f64,
    pub seekable_end_secs: f64,
    pub is_live_stream: bool,
    pub dropped_frames: i64,
    pub paused_for_cache: bool,
    pub cache_speed_bps: u64,
    pub buffering_pct: i64,
    pub stall_count: u32,
    pub recent_stall_times: Vec<Instant>,
    pub current_stream_height: u16,
    pub current_stream_fps: f32,
    pub current_stream_tbr_kbps: f64,
    pub video_bitrate_kbps: f64,
    pub audio_bitrate_kbps: f64,
    pub estimated_vf_fps: f64,
    pub avsync_ms: f64,
    pub hwdec_current: String,
    pub video_codec_live: String,
    pub decoded_width: u32,
    pub decoded_height: u32,
    pub colormatrix: String,
    pub hdr_sig_peak: f64,
    pub delayed_frames: i64,
    pub audio_samplerate: u32,
    pub audio_channels: String,
    pub is_local_file: bool,
    pub playback_started_at: Option<Instant>,
    pub last_abr_switch_at: Option<Instant>,
    pub audio_tracks: Vec<LiveMediaTrack>,
    pub audio_track_scroll: usize,
    pub sub_tracks: Vec<LiveMediaTrack>,
    pub telemetry_history: TelemetryHistory,
    pub cmd_tx: Option<UnboundedSender<PlayerCommand>>,
    pub play_hitboxes: Vec<(Rect, PlayOptionsHit)>,
    pub now_playing_hitboxes: Vec<(Rect, NowPlayingHit)>,
}

impl Default for LivePlaybackControlState {
    fn default() -> Self {
        Self {
            paused: false,
            volume: 100,
            muted: false,
            speed: 1.0,
            audio_delay_ms: 0,
            sub_delay_ms: 0,
            sub_visible: true,
            active_aid: None,
            active_sid: None,
            ontop: false,
            fullscreen: false,
            pip_mode: false,
            pip_corner_idx: 0,
            player_pid: None,
            loop_video: false,
            pitch_correction: true,
            ab_loop_a: None,
            ab_loop_b: None,
            sleep_timer_mode: SleepTimerMode::Off,
            sleep_timer_deadline: None,
            brightness: 0,
            contrast: 0,
            saturation: 0,
            gamma: 0,
            hue: 0,
            sharpness: 0,
            picture_bypass: false,
            audio_eq_bypass: false,
            video_zoom: 0.0,
            panscan: 0.0,
            aspect_override: "-1".to_string(),
            video_rotate: 0,
            mirror_x: false,
            cache_bytes: 0,
            cache_ahead_secs: 0.0,
            seekable_end_secs: 0.0,
            is_live_stream: false,
            dropped_frames: 0,
            paused_for_cache: false,
            cache_speed_bps: 0,
            buffering_pct: 0,
            stall_count: 0,
            recent_stall_times: Vec::new(),
            current_stream_height: 0,
            current_stream_fps: 0.0,
            current_stream_tbr_kbps: 0.0,
            video_bitrate_kbps: 0.0,
            audio_bitrate_kbps: 0.0,
            estimated_vf_fps: 0.0,
            avsync_ms: 0.0,
            hwdec_current: String::new(),
            video_codec_live: String::new(),
            decoded_width: 0,
            decoded_height: 0,
            colormatrix: String::new(),
            hdr_sig_peak: 0.0,
            delayed_frames: 0,
            audio_samplerate: 0,
            audio_channels: String::new(),
            is_local_file: false,
            playback_started_at: None,
            last_abr_switch_at: None,
            audio_tracks: Vec::new(),
            audio_track_scroll: 0,
            sub_tracks: Vec::new(),
            telemetry_history: TelemetryHistory::default(),
            cmd_tx: None,
            play_hitboxes: Vec::new(),
            now_playing_hitboxes: Vec::new(),
        }
    }
}

impl LivePlaybackControlState {
    pub fn live_edge_lag_secs(&self, current_pos: f64) -> f64 {
        if self.seekable_end_secs > current_pos {
            (self.seekable_end_secs - current_pos).max(0.0)
        } else {
            self.cache_ahead_secs.max(0.0)
        }
    }
}

pub fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

fn clamped_centered_rect(
    frame_area: Rect,
    margin_x: u16,
    margin_y: u16,
    min_w: u16,
    max_w: u16,
    min_h: u16,
    max_h: u16,
) -> Rect {
    let target_w = frame_area
        .width
        .saturating_sub(margin_x)
        .min(max_w)
        .max(frame_area.width.min(min_w));
    let target_h = frame_area
        .height
        .saturating_sub(margin_y)
        .min(max_h)
        .max(frame_area.height.min(min_h));
    let x = frame_area.x + frame_area.width.saturating_sub(target_w) / 2;
    let y = frame_area.y + frame_area.height.saturating_sub(target_h) / 2;
    Rect::new(x, y, target_w, target_h)
}

/// Content-fitted centered bounding box for the Command Matrix (`?`) modal so large terminals
/// never stretch cards into dead vertical or horizontal void (exact 116x28 content budget).
pub fn help_modal_rect(frame_area: Rect) -> Rect {
    clamped_centered_rect(frame_area, 4, 2, 76, 116, 20, 28)
}

/// Content-fitted enlarged bounding box for the 6-Page Playback Studio (`p`) modal (up to 156x40).
pub fn play_options_modal_rect(frame_area: Rect) -> Rect {
    clamped_centered_rect(frame_area, 2, 1, 96, 156, 26, 40)
}

/// Exact bounds of active modal overlay so underlying native Kitty thumbnails aren't drawn over it.
pub fn active_modal_rect(frame_area: Rect, overlay: ActiveOverlay) -> Option<Rect> {
    let (px, py) = match overlay {
        ActiveOverlay::None => return None,
        ActiveOverlay::Search => return Some(frame_area),
        ActiveOverlay::Login => (84, 78),
        ActiveOverlay::Filter => (64, 66),
        ActiveOverlay::PlayOptions => return Some(play_options_modal_rect(frame_area)),
        ActiveOverlay::Settings => (82, 74),
        ActiveOverlay::MusicSettings => (72, 72),
        ActiveOverlay::MusicEq => (84, 78),
        ActiveOverlay::MusicAlbum => (88, 86),
        ActiveOverlay::Help => return Some(help_modal_rect(frame_area)),
        ActiveOverlay::Analytics => (94, 90),
        ActiveOverlay::Calendar => (90, 84),
        ActiveOverlay::Remote => (64, 78),
        ActiveOverlay::Cast => (68, 60),
        ActiveOverlay::Download => (68, 52),
        ActiveOverlay::Update => (66, 54),
        ActiveOverlay::ConfirmQuit => (50, 30),
    };
    Some(centered_rect(px, py, frame_area))
}

/// Checks if a card/hero Rect overlaps an open modal to prevent terminal protocol image bleed-through.
pub fn occluded_by_modal(card_area: Rect, modal_rect: Option<Rect>) -> bool {
    let Some(m) = modal_rect else {
        return false;
    };
    let x_overlap = card_area.x < m.x + m.width && m.x < card_area.x + card_area.width;
    let y_overlap = card_area.y < m.y + m.height && m.y < card_area.y + card_area.height;
    x_overlap && y_overlap
}

struct ReplyAdjacency {
    root_children: Vec<usize>,
    by_reply: Vec<Vec<usize>>,
}

fn norm_handle(raw: &str) -> String {
    raw.trim()
        .trim_start_matches('@')
        .to_lowercase()
}

fn leading_mention_handle(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    let rest = trimmed.strip_prefix('@')?;
    let tok: String = rest
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        .collect();
    if tok.len() >= 2 {
        Some(tok.to_lowercase())
    } else {
        None
    }
}

fn build_reply_adjacency(replies: &[VideoComment]) -> ReplyAdjacency {
    let mut root_children = Vec::new();
    let mut by_reply = vec![Vec::new(); replies.len()];
    let mut id_to_idx: HashMap<&str, usize> = HashMap::new();
    let mut handle_to_idx: HashMap<String, usize> = HashMap::new();

    for (idx, r) in replies.iter().enumerate() {
        id_to_idx.insert(r.id.as_str(), idx);
    }
    for (idx, r) in replies.iter().enumerate() {
        let mut attached = false;
        if let Some(ref pid) = r.parent_id {
            if let Some(&p_idx) = id_to_idx.get(pid.as_str()) {
                if p_idx != idx {
                    by_reply[p_idx].push(idx);
                    attached = true;
                }
            }
        }
        if !attached {
            if let Some(m_handle) = leading_mention_handle(&r.text) {
                if m_handle != norm_handle(&r.author) {
                    if let Some(&p_idx) = handle_to_idx.get(&m_handle) {
                        if p_idx < idx {
                            by_reply[p_idx].push(idx);
                            attached = true;
                        }
                    }
                }
            }
        }
        if !attached {
            root_children.push(idx);
        }
        let h = norm_handle(&r.author);
        if !h.is_empty() {
            handle_to_idx.insert(h, idx);
        }
    }

    ReplyAdjacency {
        root_children,
        by_reply,
    }
}

fn count_tree_descendants(by_reply: &[Vec<usize>], idx: usize) -> usize {
    let mut total = by_reply[idx].len();
    for &c in &by_reply[idx] {
        total += count_tree_descendants(by_reply, c);
    }
    total
}

fn subtree_descendant_count(replies: &[VideoComment], remote_cnt: u32) -> usize {
    (remote_cnt as usize).max(replies.len())
}

#[allow(clippy::too_many_arguments)]
fn push_reply_subtree(
    replies: &[VideoComment],
    by_reply: &[Vec<usize>],
    idx: usize,
    parent_id: String,
    root_id: String,
    root_idx: usize,
    depth: usize,
    is_last_sibling: bool,
    rail_active: Vec<bool>,
    collapsed_ids: &HashSet<String>,
    out: &mut Vec<CommentTreeRow>,
) {
    let r = &replies[idx];
    let children = &by_reply[idx];
    let has_children = !children.is_empty();
    let collapsed = has_children && collapsed_ids.contains(&r.id);
    let desc_count = count_tree_descendants(by_reply, idx);

    out.push(CommentTreeRow {
        comment: r.clone(),
        depth: depth.min(6),
        parent_id: Some(parent_id),
        root_id: root_id.clone(),
        root_idx,
        is_last_sibling,
        rail_active: rail_active.clone(),
        has_children,
        child_count: desc_count,
        collapsed,
    });

    if collapsed || children.is_empty() {
        return;
    }

    let mut next_rails = rail_active;
    if depth < 6 {
        next_rails.push(!is_last_sibling);
    }
    for (pos, &child_i) in children.iter().enumerate() {
        let child_last = pos + 1 == children.len();
        push_reply_subtree(
            replies,
            by_reply,
            child_i,
            r.id.clone(),
            root_id.clone(),
            root_idx,
            (depth + 1).min(6),
            child_last,
            next_rails.clone(),
            collapsed_ids,
            out,
        );
    }
}

pub fn comment_age_rank(published: &str) -> (u8, u64) {
    let low = published.trim().to_lowercase();
    if low.is_empty() || low.contains("just now") {
        return (0, 0);
    }
    let num = low
        .split_whitespace()
        .find_map(|w| w.parse::<u64>().ok())
        .unwrap_or(1);
    if low.contains("second") {
        (1, num)
    } else if low.contains("minute") {
        (2, num)
    } else if low.contains("hour") {
        (3, num)
    } else if low.contains("day") {
        (4, num)
    } else if low.contains("week") {
        (5, num)
    } else if low.contains("month") {
        (6, num)
    } else {
        (7, num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_app_state_defaults_and_modal_occlusion() {
        let st = AppState::new(Config::default());
        assert_eq!(st.screen, Screen::Home);
        assert_eq!(st.overlay, ActiveOverlay::None);
        assert_eq!(st.active_category, 1);
        assert!(active_modal_rect(Rect::new(0, 0, 120, 40), ActiveOverlay::None).is_none());

        let m = active_modal_rect(Rect::new(0, 0, 120, 40), ActiveOverlay::Settings);
        assert!(occluded_by_modal(Rect::new(40, 15, 24, 10), m));
        assert!(!occluded_by_modal(Rect::new(0, 0, 5, 2), m));
    }

    #[test]
    fn test_mark_video_watched_removes_from_recommended_and_adds_to_history() {
        let mut st = AppState::new(Config::default());
        let v1 = VideoItem {
            id: "home_v1".into(),
            title: "First Video".into(),
            ..Default::default()
        };
        let v2 = VideoItem {
            id: "home_v2".into(),
            title: "Second Video".into(),
            ..Default::default()
        };
        if let Some((_, rec)) = st
            .categories
            .iter_mut()
            .find(|(c, _)| *c == Category::Recommended)
        {
            *rec = vec![v1.clone(), v2.clone()];
        }
        st.active_category = st
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Recommended)
            .unwrap();

        st.mark_video_watched(&v1);

        let rec_filtered = st.current_filtered_videos();
        assert_eq!(rec_filtered.len(), 1);
        assert_eq!(rec_filtered[0].id, "home_v2");

        let hist_items = &st
            .categories
            .iter()
            .find(|(c, _)| *c == Category::History)
            .unwrap()
            .1;
        assert_eq!(hist_items.len(), 1);
        assert_eq!(hist_items[0].id, "home_v1");
    }

    #[test]
    fn test_shorts_excluded_from_home_feed_only_live_in_shorts() {
        let mut st = AppState::new(Config::default());
        let long_vid = VideoItem {
            id: "long_home_1".into(),
            title: "Deep Architecture Talk".into(),
            duration: Some(1200),
            ..Default::default()
        };
        let short_by_dur = VideoItem {
            id: "short_dur_1".into(),
            title: "45s Clip".into(),
            duration: Some(45),
            ..Default::default()
        };
        let short_by_url = VideoItem {
            id: "short_url_1".into(),
            title: "Shorts URL Clip".into(),
            webpage_url: Some("https://www.youtube.com/shorts/short_url_1".into()),
            ..Default::default()
        };
        let short_by_label = VideoItem {
            id: "short_lbl_1".into(),
            title: "Tagged Short".into(),
            duration_string: Some("SHORT".into()),
            ..Default::default()
        };

        if let Some((_, rec)) = st
            .categories
            .iter_mut()
            .find(|(c, _)| *c == Category::Recommended)
        {
            *rec = vec![
                long_vid.clone(),
                short_by_dur.clone(),
                short_by_url.clone(),
                short_by_label.clone(),
            ];
        }
        st.active_category = st
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Recommended)
            .unwrap();

        let rec_filtered = st.current_filtered_videos();
        assert_eq!(rec_filtered.len(), 1);
        assert_eq!(rec_filtered[0].id, "long_home_1");

        st.prune_watched_from_recommended();
        assert_eq!(st.categories[st.active_category].1.len(), 1);

        st.shorts_suggested = vec![short_by_dur, short_by_url, short_by_label];
        st.sync_active_shorts_list();
        st.active_category = st
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Shorts)
            .unwrap();
        assert_eq!(st.current_filtered_videos().len(), 3);
    }

    #[test]
    fn test_reddit_comment_tree_nesting_collapse_and_sub_reply() {
        let mut st = AppState::new(Config::default());
        st.detail_comments = vec![VideoComment {
            id: "root_1".into(),
            author: "@Stay_positive_Test_negative".into(),
            published_text: "17 hours ago".into(),
            text: "Truly a sad day to see Toyota go down this route".into(),
            like_count_text: "168".into(),
            reply_count: 3,
            replies_expanded: true,
            replies: vec![
                VideoComment {
                    id: "rep_1".into(),
                    author: "@audreymiller3056".into(),
                    published_text: "15 hours ago".into(),
                    text: "yep, just buy another brand at this point".into(),
                    like_count_text: "7".into(),
                    ..Default::default()
                },
                VideoComment {
                    id: "rep_2".into(),
                    author: "@toddbender3463".into(),
                    published_text: "13 hours ago".into(),
                    text: "@audreymiller3056 it is still sitting at the top".into(),
                    like_count_text: "6".into(),
                    ..Default::default()
                },
                VideoComment {
                    id: "rep_3".into(),
                    author: "@chicagotypewriter2094".into(),
                    published_text: "4 hours ago".into(),
                    text: "@toddbender3463 nah, the gap is shrinking".into(),
                    like_count_text: "0".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }];

        let rows = st.build_comment_tree();
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].depth, 0);
        assert_eq!(rows[1].depth, 1);
        assert_eq!(rows[2].depth, 2);
        assert_eq!(rows[3].depth, 3);

        // Collapse middle sub-comment `rep_2`; hides `rep_3` while keeping `rep_1` & `rep_2`
        st.detail_comment_collapsed.insert("rep_2".into());
        let collapsed_rows = st.build_comment_tree();
        assert_eq!(collapsed_rows.len(), 3);
        assert!(collapsed_rows[2].collapsed);
        assert_eq!(collapsed_rows[2].child_count, 1);

        // Reply directly to nested `rep_2`; auto-expands `rep_2` and inserts beneath it
        assert!(st.insert_comment_reply(
            "root_1",
            "rep_2",
            VideoComment {
                id: "local_sub_1".into(),
                author: "@you".into(),
                text: "@toddbender3463 fair point".into(),
                is_self: true,
                ..Default::default()
            }
        ));
        let updated = st.build_comment_tree();
        assert_eq!(updated.len(), 5);
        assert_eq!(updated[3].comment.id, "local_sub_1");
        assert_eq!(updated[3].depth, 3);
    }

    #[test]
    fn test_search_query_parser_hybrid_rrf_ranking_and_thumb_backoff() {
        let parsed = parse_search_query("distributed \"event driven\" @gaurav -shorts is:4k is:unwatched");
        assert_eq!(parsed.tokens, vec!["distributed", "event", "driven"]);
        assert_eq!(parsed.exact_phrases, vec!["event driven"]);
        assert_eq!(parsed.channel_filter.as_deref(), Some("gaurav"));
        assert_eq!(parsed.excluded_terms, vec!["shorts"]);
        assert!(parsed.only_4k);
        assert!(parsed.only_unwatched);
        assert_eq!(parsed.api_query, "distributed \"event driven\" gaurav -shorts");

        let mut st = AppState::new(Config::default());
        st.search_query = "system architecture".into();
        st.search_results = vec![
            VideoItem {
                id: "highview_sht".into(),
                title: "Architecture in 50s".into(),
                duration: Some(45),
                duration_string: Some("SHORT".into()),
                view_count: Some(5_000_000),
                yt_rank: Some(4),
                ..Default::default()
            },
            VideoItem {
                id: "top_ml_video".into(),
                title: "Designing Scalable Production Backends".into(),
                uploader: Some("ByteByteGo".into()),
                duration: Some(1400),
                view_count: Some(400_000),
                channel_verified: true,
                yt_rank: Some(0),
                ..Default::default()
            },
        ];

        let ranked = st.combined_search_items();
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].id, "top_ml_video");

        // Thumbnail backoff & manual reset
        st.queue_thumb_with_priority("vid12345678", "", ThumbPriority::Visible);
        st.queue_thumb_with_priority("vid12345678", "https://i.ytimg.com/vi/vid12345678/hq720.jpg", ThumbPriority::Focused);
        assert_eq!(st.pending_thumb_requests.len(), 1);
        assert_eq!(st.pending_thumb_requests[0].priority, ThumbPriority::Focused);
        st.failed_thumbs.insert(
            "bad12345678".into(),
            FailedThumbEntry {
                attempts: 3,
                next_retry_at: Instant::now() + Duration::from_secs(30),
            },
        );
        assert!(st.is_thumb_failed("bad12345678"));
        st.retry_failed_thumbs();
        assert!(!st.is_thumb_failed("bad12345678"));
    }
}
