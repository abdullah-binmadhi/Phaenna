//! Central asynchronous message bus for background workers and TUI event loop.

use crate::api::dislike::DislikeInfo;
use crate::api::innertube::{SearchChannelCard, VideoComment, VideoVote};
use crate::api::sponsorblock::SponsorSegment;
use crate::api::subtitles::SubtitleTrack;
use crate::api::ytdlp::ResolvedStreamLadder;
use crate::db::cache::{VideoItem, VideoStatsUpdate};
use crate::services::auth::GoogleAccountSession;
use crate::services::cast::CastDevice;
use crate::services::ipc::{LiveIpcEvent, PlayerCommand};
use crate::services::updater::UpdateInfo;
use tokio::sync::mpsc::UnboundedSender;

#[derive(Debug)]
#[allow(dead_code, clippy::large_enum_variant)]
pub enum AppMessage {
    // Feed & SQLite SWR sync
    FeedLoaded {
        feed_key: String,
        items: Vec<VideoItem>,
        continuation: Option<String>,
        is_from_cache: bool,
    },
    MoreFeedLoaded {
        feed_key: String,
        items: Vec<VideoItem>,
        continuation: Option<String>,
    },
    FetchProgress {
        feed_key: String,
        current: usize,
        total: usize,
    },
    VideosMetadataEnriched {
        updates: Vec<VideoStatsUpdate>,
    },

    // Search & Drill-down
    SearchResults {
        generation: u64,
        query: String,
        items: Vec<VideoItem>,
        shorts: Vec<VideoItem>,
        channels: Vec<SearchChannelCard>,
        did_you_mean: Option<String>,
        estimated_results: Option<u64>,
        latency_ms: u64,
        continuation: Option<String>,
    },
    MoreSearchResults {
        generation: u64,
        items: Vec<VideoItem>,
        shorts: Vec<VideoItem>,
        continuation: Option<String>,
    },
    SearchSuggestionsReady {
        generation: u64,
        suggestions: Vec<String>,
    },
    ChannelVideosLoaded {
        channel_key: String,
        items: Vec<VideoItem>,
    },
    VideoDetailsLoaded {
        video_id: String,
        item: VideoItem,
        subtitles: Vec<SubtitleTrack>,
        ladder: Option<ResolvedStreamLadder>,
    },
    VideoSynopsisLoaded {
        video_id: String,
        details: Option<crate::api::innertube::InnerTubeVideoDetails>,
    },
    VideoDetailsFailed {
        video_id: String,
    },
    StreamLadderResolved {
        video_id: String,
        ladder: ResolvedStreamLadder,
    },

    // Community Enrichment
    SponsorSegmentsLoaded {
        video_id: String,
        segments: Vec<SponsorSegment>,
    },
    DislikesLoaded {
        video_id: String,
        info: DislikeInfo,
    },
    VideoVoteLoaded {
        video_id: String,
        vote: VideoVote,
    },
    CommentsLoaded {
        video_id: String,
        comments: Vec<VideoComment>,
        create_params: Option<String>,
    },
    CommentRepliesLoaded {
        video_id: String,
        parent_comment_id: String,
        replies: Vec<VideoComment>,
        continuation: Option<String>,
    },
    CommentPosted {
        video_id: String,
        comment: VideoComment,
        parent_comment_id: Option<String>,
        synced_remote: bool,
    },
    CommentTranslated {
        comment_id: String,
        target_lang: String,
        source_lang: Option<String>,
        translated_text: Option<String>,
        error: Option<String>,
    },

    // Thumbnail Pipeline
    ThumbnailReady {
        video_id: String,
        image: image::DynamicImage,
        raw_bytes: Option<Vec<u8>>,
    },
    ThumbnailFailed {
        video_id: String,
    },

    // In-App Google Auth & Identity Hub
    AuthPopupReady {
        launcher: String,
    },
    AuthSignedIn(GoogleAccountSession),
    AuthRefreshed(GoogleAccountSession),
    AuthFailed(String),

    // Non-Blocking Playback & IPC Telemetry
    PlaybackStarted {
        video_id: String,
        title: String,
        session_id: u64,
        sock_path: String,
        stream_hint: Option<(u16, f32, f64)>,
        is_local: bool,
        player_pid: Option<u32>,
    },
    PipToggleFromPlayer {
        enabled: bool,
        corner_idx: Option<usize>,
    },
    PipCornerFromPlayer {
        corner_idx: usize,
    },
    FullscreenToggleFromPlayer {
        enabled: bool,
    },
    LinuxPipOutcome(crate::services::pip::PipApplyOutcome),
    PlaybackCommandReady {
        session_id: u64,
        cmd_tx: UnboundedSender<PlayerCommand>,
    },
    PlaybackTelemetryUpdated {
        session_id: u64,
        event: LiveIpcEvent,
    },
    PlaybackProgressUpdated {
        video_id: String,
        pos: f64,
        duration: f64,
    },
    SponsorSkipped {
        video_id: String,
        category: String,
        saved_secs: f64,
    },
    VideoAutoCompleted {
        video_id: String,
    },
    PlaybackDone(u64),

    // Offline Vault Downloader
    DownloadProgress {
        video_id: String,
        percent: f32,
        speed: String,
    },
    DownloadComplete {
        video_id: String,
        file_path: String,
    },
    DownloadFailed {
        video_id: String,
        error: String,
    },

    // Self-Updater & UI Notifications
    UpdateCheckResult(Option<UpdateInfo>),
    UpdateProgress(f32, String),
    UpdateSuccess(String),
    UpdateFailed(String),
    ShowToast(String),

    // LAN Remote & TV Cast
    CastDevicesDiscovered(Vec<CastDevice>),
    CastStarted(String),
    CastFailed(String),
    RemotePlayVideo(String),
    RemotePlayFromPos(String, f64),
    RemoteToggleSubtitle(String),
}
