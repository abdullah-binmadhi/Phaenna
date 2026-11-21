//! Central Application State — single source of truth for screens, overlays, cache & playback.

use crate::api::dislike::DislikeInfo;
use crate::api::innertube::{VideoComment, VideoVote};
use crate::api::sponsorblock::SponsorSegment;
use crate::api::subtitles::SubtitleTrack;
use crate::api::ytdlp::ResolvedStreamLadder;
use crate::config::{
    Config, PerformanceMode, Player, Quality, SponsorBlockMode, VideoShaderMode,
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
use crate::services::updater::UpdateInfo;
use crate::ui::theme::{self, extract_ambilight_palette, ThemePalette};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::UnboundedSender;

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
}

impl Category {
    pub fn all_shelves() -> [Category; 6] {
        [
            Category::ContinueWatching,
            Category::Recommended,
            Category::Shorts,
            Category::Subscriptions,
            Category::History,
            Category::WatchLater,
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
        }
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

    // Search Studio + 120ms/350ms debounce + smart filters
    pub search_query: String,
    pub search_results: Vec<VideoItem>,
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
    pub search_hitboxes: Vec<(Rect, SearchHit)>,
    pub search_results_rect: Rect,
    pub search_last_click: Option<(usize, Instant)>,
    pub search_cursor: usize,
    pub search_generation: u64,
    pub search_last_typed: Option<Instant>,
    pub search_pending_remote: bool,
    pub is_searching_remote: bool,

    // Play options modal (6 balanced zones: 0 Stream, 1 Audio, 2 Captions/Live, 3 Mode/Shaders, 4 Manual, 5 Transport)
    pub play_modal_section: usize,
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
    pub raw_thumbs: HashMap<String, image::DynamicImage>,
    pub ambilight_cache: HashMap<String, ThemePalette>,
    pub loading_thumbs: HashSet<String>,
    pub failed_thumbs: HashMap<String, u8>,
    pub pending_thumb_requests: Vec<(String, String)>,
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
            search_hitboxes: Vec::new(),
            search_results_rect: Rect::default(),
            search_last_click: None,
            search_cursor: 0,
            search_generation: 0,
            search_last_typed: None,
            search_pending_remote: false,
            is_searching_remote: false,

            play_modal_section: 0,
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
            live_playback: LivePlaybackControlState::default(),
            stream_ladder_cache: HashMap::new(),

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
            raw_thumbs: HashMap::new(),
            ambilight_cache: HashMap::new(),
            loading_thumbs: HashSet::new(),
            failed_thumbs: HashMap::new(),
            pending_thumb_requests: Vec::new(),
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
        self.live_playback = LivePlaybackControlState {
            sub_visible: self.config.subtitles_enabled,
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
            LiveIpcEvent::Brightness(v) => self.live_playback.brightness = v,
            LiveIpcEvent::Contrast(v) => self.live_playback.contrast = v,
            LiveIpcEvent::Saturation(v) => self.live_playback.saturation = v,
            LiveIpcEvent::Gamma(v) => self.live_playback.gamma = v,
            LiveIpcEvent::CacheBytes(b) => self.live_playback.cache_bytes = b,
            LiveIpcEvent::LiveCacheTelemetry {
                cache_ahead_secs,
                seekable_end_secs,
            } => {
                self.live_playback.cache_ahead_secs = cache_ahead_secs;
                self.live_playback.seekable_end_secs = seekable_end_secs;
            }
            LiveIpcEvent::DroppedFrames(d) => self.live_playback.dropped_frames = d,
            LiveIpcEvent::PausedForCache(v) => self.live_playback.paused_for_cache = v,
            LiveIpcEvent::CacheSpeed(bps) => self.live_playback.cache_speed_bps = bps,
            LiveIpcEvent::BufferingPct(pct) => self.live_playback.buffering_pct = pct,
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
        let q_trimmed = self.search_query.trim().to_lowercase();
        let q_clean = q_trimmed
            .strip_prefix("ch:")
            .unwrap_or(&q_trimmed)
            .trim_start_matches('@')
            .to_string();
        let tokens: Vec<&str> = q_clean.split_whitespace().collect();

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
                for v in &self.search_local_results {
                    push_dedupe(v);
                }
            }
            SearchScope::LiveYt => {
                for v in &self.search_results {
                    push_dedupe(v);
                }
            }
            SearchScope::Subscribed => {
                for v in sub_items.into_iter().chain(self.shorts_subscribed.iter()) {
                    if q_clean.is_empty() || video_matches_tokens(v, &tokens) {
                        push_dedupe(v);
                    }
                }
            }
            SearchScope::Cached => {
                for v in &self.search_local_results {
                    push_dedupe(v);
                }
            }
        }

        // Apply Search filters
        let min_ts = self.search_date_filter.min_timestamp(now);
        pool.retain(|v| {
            if self.search_hide_watched && self.is_video_watched(&v.id) {
                return false;
            }
            match self.search_type_filter {
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

        // Score & sort
        let mut scored: Vec<(i64, VideoItem)> = pool
            .into_iter()
            .map(|v| {
                let s = search_relevance_score(&v, &q_clean, &tokens, &sub_uploaders, now);
                (s, v)
            })
            .collect();

        match self.search_sort_order {
            SearchSortOrder::Relevance => {
                scored.sort_by_key(|a| std::cmp::Reverse(a.0));
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
        let sort_code = match self.search_sort_order {
            SearchSortOrder::Relevance => 0,
            SearchSortOrder::Newest => 2,
            SearchSortOrder::Views => 3,
            SearchSortOrder::Duration => 0,
        };
        let date_code = match self.search_date_filter {
            SearchDateFilter::Any => 0,
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
        let videos_only = self.search_type_filter == SearchTypeFilter::Videos;
        crate::api::innertube::encode_search_params(sort_code, date_code, dur_code, videos_only)
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
        self.raw_thumbs.insert(video_id.clone(), img);
        self.image_cache.retain(|(id, _, _), _| id != &video_id);
    }

    pub fn queue_thumb_if_needed(&mut self, id: &str, url: &str) {
        if self.raw_thumbs.contains_key(id)
            || self.loading_thumbs.contains(id)
            || self.failed_thumbs.get(id).copied().unwrap_or(0) >= 2
        {
            return;
        }
        if !self
            .pending_thumb_requests
            .iter()
            .any(|(eid, _)| eid == id)
        {
            self.pending_thumb_requests
                .push((id.to_string(), url.to_string()));
        }
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

fn video_matches_tokens(v: &VideoItem, tokens: &[&str]) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let title_l = v.title.to_lowercase();
    let up_l = v.display_uploader().to_lowercase();
    tokens
        .iter()
        .all(|t| title_l.contains(t) || up_l.contains(t))
}

fn search_relevance_score(
    v: &VideoItem,
    q_clean: &str,
    tokens: &[&str],
    sub_uploaders: &HashSet<String>,
    now_unix: i64,
) -> i64 {
    if q_clean.is_empty() {
        let rec = v.timestamp.unwrap_or(0) / 86_400;
        let views = v.view_count.unwrap_or(0).min(10_000_000) as i64 / 50_000;
        return rec + views;
    }
    let title_l = v.title.to_lowercase();
    let up_l = v.display_uploader().to_lowercase();
    let mut score = 0i64;
    if title_l.contains(q_clean) {
        score += 60;
        if title_l.starts_with(q_clean) {
            score += 15;
        }
    }
    if up_l == q_clean {
        score += 55;
    } else if up_l.contains(q_clean) {
        score += 35;
    }
    let matched_tokens = tokens
        .iter()
        .filter(|t| title_l.contains(**t) || up_l.contains(**t))
        .count();
    score += (matched_tokens as i64) * 14;
    if !tokens.is_empty() && matched_tokens == tokens.len() {
        score += 22;
    }
    if sub_uploaders.contains(&up_l) {
        score += 12;
    }
    if let Some(ts) = v.timestamp {
        if now_unix.saturating_sub(ts) <= 7 * 86_400 {
            score += 8;
        }
    }
    if let Some(vw) = v.view_count {
        if vw > 0 {
            score += ((vw as f64).log10() * 2.0) as i64;
        }
    }
    if v.is_shorts() {
        score -= 4;
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
}

impl SearchTypeFilter {
    pub const ALL: [Self; 3] = [Self::All, Self::Videos, Self::Shorts];
    pub const fn next(self) -> Self {
        match self {
            Self::All => Self::Videos,
            Self::Videos => Self::Shorts,
            Self::Shorts => Self::All,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Videos => "Videos",
            Self::Shorts => "Shorts",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchDateFilter {
    Any,
    Today,
    Week,
    Month,
    Year,
}

impl SearchDateFilter {
    pub const ALL: [Self; 5] = [Self::Any, Self::Today, Self::Week, Self::Month, Self::Year];
    pub const fn next(self) -> Self {
        match self {
            Self::Any => Self::Today,
            Self::Today => Self::Week,
            Self::Week => Self::Month,
            Self::Month => Self::Year,
            Self::Year => Self::Any,
        }
    }
    pub const fn label(self) -> &'static str {
        match self {
            Self::Any => "Any",
            Self::Today => "Today",
            Self::Week => "Week",
            Self::Month => "Month",
            Self::Year => "Year",
        }
    }
    pub fn min_timestamp(self, now: i64) -> Option<i64> {
        match self {
            Self::Any => None,
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
    SelectRow(usize),
    ActionPlay,
    ActionDetail,
    ActionPinCw,
    ActionWatchLater,
    ActionChannel,
    LoadMore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManualToggle {
    SuperRes,
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
    pub const ALL: [Self; 10] = [
        Self::SuperRes,
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

#[derive(Debug, Clone, PartialEq)]
pub enum PlayOptionsHit {
    FocusSection(usize),
    SelectQuality(usize),
    SelectPlayer(Player),
    ToggleDub,
    CycleDubLang,
    ToggleMultistreams,
    ToggleLoudnorm,
    AudioDelayDelta(i32),
    SelectAudioTrack(i64),
    ToggleSubtitles,
    CycleSubStyle,
    SubDelayDelta(i32),
    SelectSubTrack(i64),
    SelectSponsorBlock(SponsorBlockMode),
    ToggleSkipOutro,
    SelectShader(VideoShaderMode),
    SetPerformanceMode(PerformanceMode),
    ToggleManual(ManualToggle),
    SeedManualFrom(PerformanceMode),
    DownloadNeuralShaders,
    CycleLiveMode,
    CycleNetworkMode,
    ToggleLiveCatchup,
    JumpToLiveEdge,
    PlayLiveFromStart,
    LaunchAudioRadio,
    ScrollAudioTracks(i8),
    TogglePause,
    SeekDelta(f64),
    SeekRatio(f64),
    ChapterStep(i32),
    SetSpeed(f64),
    VolumeDelta(i64),
    ToggleMute,
    PictureDelta(&'static str, i64),
    ResetPicture,
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
    pub loop_video: bool,
    pub brightness: i64,
    pub contrast: i64,
    pub saturation: i64,
    pub gamma: i64,
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
    pub is_local_file: bool,
    pub playback_started_at: Option<Instant>,
    pub last_abr_switch_at: Option<Instant>,
    pub audio_tracks: Vec<LiveMediaTrack>,
    pub audio_track_scroll: usize,
    pub sub_tracks: Vec<LiveMediaTrack>,
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
            loop_video: false,
            brightness: 0,
            contrast: 0,
            saturation: 0,
            gamma: 0,
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
            is_local_file: false,
            playback_started_at: None,
            last_abr_switch_at: None,
            audio_tracks: Vec::new(),
            audio_track_scroll: 0,
            sub_tracks: Vec::new(),
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

/// Content-fitted enlarged bounding box for the 6-Zone Playback Studio (`p`) modal (up to 146x33).
pub fn play_options_modal_rect(frame_area: Rect) -> Rect {
    clamped_centered_rect(frame_area, 2, 2, 96, 146, 24, 33)
}

/// Exact bounds of active modal overlay so underlying native Kitty thumbnails aren't drawn over it.
pub fn active_modal_rect(frame_area: Rect, overlay: ActiveOverlay) -> Option<Rect> {
    let (px, py) = match overlay {
        ActiveOverlay::None => return None,
        ActiveOverlay::Search => (90, 86),
        ActiveOverlay::Login => (84, 78),
        ActiveOverlay::Filter => (64, 66),
        ActiveOverlay::PlayOptions => return Some(play_options_modal_rect(frame_area)),
        ActiveOverlay::Settings => (82, 74),
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

fn comment_age_rank(published: &str) -> (u8, u64) {
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
}
