//! Central Application State — single source of truth for screens, overlays, cache & playback.

use crate::api::dislike::DislikeInfo;
use crate::api::innertube::VideoComment;
use crate::api::player::{detect_dependencies, DependencyStatus};
use crate::api::sponsorblock::SponsorSegment;
use crate::api::subtitles::SubtitleTrack;
use crate::config::{Config, Quality};
use crate::db::cache::VideoItem;
use crate::db::sync::{HISTORY, RECOMMENDED, SUBSCRIPTIONS, WATCH_LATER};
use crate::db::user::{ContinueWatchingMeta, UserAnalytics, WatchLaterSource};
use crate::services::auth::{AuthVault, LoginUiState};
use crate::services::calendar::{compute_week_bounds, local_tz_offset_secs, CalendarFilter, WeekBounds};
use crate::services::cast::CastDevice;
use crate::services::remote::RemoteServerHandle;
use crate::services::tailored::{LiveTunerConfig, TailoredMatch};
use crate::services::updater::UpdateInfo;
use crate::ui::theme::{self, extract_ambilight_palette, ThemePalette};
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::collections::{HashMap, HashSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

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
    Setup,
    Analytics,
    TailoredWizard,
    TailoredSummary,
    TailoredResults,
    Calendar,
    Remote,
    Cast,
    Download,
    Update,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Category {
    Recommended,
    Subscriptions,
    History,
    WatchLater,
    ContinueWatching,
}

impl Category {
    pub fn all_shelves() -> [Category; 5] {
        [
            Category::ContinueWatching,
            Category::Recommended,
            Category::Subscriptions,
            Category::History,
            Category::WatchLater,
        ]
    }

    pub fn label(&self) -> &'static str {
        match self {
            Category::ContinueWatching => "Continue Watching",
            Category::Recommended => "Home Feed",
            Category::Subscriptions => "Subscriptions",
            Category::History => "History",
            Category::WatchLater => "Watch Later",
        }
    }

    pub fn feed_key(&self) -> &'static str {
        match self {
            Category::ContinueWatching => "continue_watching",
            Category::Recommended => RECOMMENDED,
            Category::Subscriptions => SUBSCRIPTIONS,
            Category::History => HISTORY,
            Category::WatchLater => WATCH_LATER,
        }
    }

    pub fn url(&self) -> Option<&'static str> {
        match self {
            Category::Recommended => Some("https://www.youtube.com"),
            Category::Subscriptions => Some("https://www.youtube.com/feed/subscriptions"),
            Category::History => Some("https://www.youtube.com/feed/history"),
            Category::WatchLater => Some("https://www.youtube.com/playlist?list=WL"),
            Category::ContinueWatching => None,
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
    pub detail_comments: Vec<VideoComment>,
    pub detail_comment_cursor: usize,
    pub detail_comments_loading: bool,
    pub detail_create_comment_params: Option<String>,
    pub detail_comment_composing: bool,
    pub detail_comment_input: String,

    pub channel_title: String,
    pub channel_id: Option<String>,
    pub channel_videos: Vec<VideoItem>,
    pub channel_cursor: usize,
    pub is_loading_channel: bool,

    // Search + 350ms debounce
    pub search_query: String,
    pub search_results: Vec<VideoItem>,
    pub search_local_results: Vec<VideoItem>,
    pub search_cursor: usize,
    pub search_generation: u64,
    pub search_last_typed: Option<Instant>,
    pub search_pending_remote: bool,
    pub is_searching_remote: bool,

    // Play options modal
    pub play_modal_section: usize, // 0: Quality, 1: Player, 2: SponsorBlock, 3: Subtitles
    pub play_quality_idx: usize,

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

    // Intelligence dashboards
    pub analytics: UserAnalytics,
    pub analytics_loaded: bool,
    pub wizard_step: usize,
    pub wizard_cursor: usize,
    pub tailored_matches: Vec<TailoredMatch>,
    pub tailored_cursor: usize,
    pub live_tuner: LiveTunerConfig,
    pub calendar_bounds: WeekBounds,
    pub calendar_day_cursor: usize,
    pub calendar_item_cursor: usize,
    pub calendar_filter: CalendarFilter,
    pub calendar_tz_offset: i64,

    // System Doctor & Services
    pub deps: DependencyStatus,
    pub remote_handle: Option<RemoteServerHandle>,
    pub cast_devices: Vec<CastDevice>,
    pub cast_cursor: usize,
    pub is_scanning_cast: bool,
    pub active_download: Option<DownloadJobState>,
    pub available_update: Option<UpdateInfo>,
    pub update_status: Option<String>,
    pub update_progress: f32,

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
            detail_comments: Vec::new(),
            detail_comment_cursor: 0,
            detail_comments_loading: false,
            detail_create_comment_params: None,
            detail_comment_composing: false,
            detail_comment_input: String::new(),

            channel_title: String::new(),
            channel_id: None,
            channel_videos: Vec::new(),
            channel_cursor: 0,
            is_loading_channel: false,

            search_query: String::new(),
            search_results: Vec::new(),
            search_local_results: Vec::new(),
            search_cursor: 0,
            search_generation: 0,
            search_last_typed: None,
            search_pending_remote: false,
            is_searching_remote: false,

            play_modal_section: 0,
            play_quality_idx,

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

            analytics: UserAnalytics::default(),
            analytics_loaded: false,
            wizard_step: 0,
            wizard_cursor: 0,
            tailored_matches: Vec::new(),
            tailored_cursor: 0,
            live_tuner: LiveTunerConfig {
                exclude_shorts: true,
                ..Default::default()
            },
            calendar_bounds: compute_week_bounds(now_unix, 0, 0),
            calendar_day_cursor: 0,
            calendar_item_cursor: 0,
            calendar_filter: CalendarFilter::All,
            calendar_tz_offset: 0,

            deps: detect_dependencies(),
            remote_handle: None,
            cast_devices: Vec::new(),
            cast_cursor: 0,
            is_scanning_cast: false,
            active_download: None,
            available_update: None,
            update_status: None,
            update_progress: 0.0,

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

    #[allow(dead_code)]
    pub fn active_cursor(&self) -> usize {
        self.category_cursors[self.active_category]
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
                if cat == Category::Recommended && self.is_video_watched(&v.id) {
                    return false;
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
                .retain(|v| !watched.contains(&v.id) && !watched.contains(v.clean_id()));
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
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        for v in &self.search_results {
            if seen.insert(v.id.clone()) {
                out.push(v.clone());
            }
        }
        for v in &self.search_local_results {
            if seen.insert(v.id.clone()) {
                out.push(v.clone());
            }
        }
        out
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

/// Exact bounds of active modal overlay so underlying native Kitty thumbnails aren't drawn over it.
pub fn active_modal_rect(frame_area: Rect, overlay: ActiveOverlay) -> Option<Rect> {
    let (px, py) = match overlay {
        ActiveOverlay::None => return None,
        ActiveOverlay::Search => (76, 78),
        ActiveOverlay::Login => (84, 78),
        ActiveOverlay::Filter => (60, 56),
        ActiveOverlay::PlayOptions => (86, 62),
        ActiveOverlay::Settings => (82, 74),
        ActiveOverlay::Help => (80, 74),
        ActiveOverlay::Setup => (74, 68),
        ActiveOverlay::Analytics => (92, 88),
        ActiveOverlay::TailoredWizard => (78, 72),
        ActiveOverlay::TailoredSummary => (84, 78),
        ActiveOverlay::TailoredResults => (88, 84),
        ActiveOverlay::Calendar => (90, 84),
        ActiveOverlay::Remote => (64, 78),
        ActiveOverlay::Cast => (68, 60),
        ActiveOverlay::Download => (68, 52),
        ActiveOverlay::Update => (66, 54),
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
}
