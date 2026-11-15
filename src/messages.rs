//! Central asynchronous message bus for background workers and TUI event loop.

use crate::api::dislike::DislikeInfo;
use crate::api::sponsorblock::SponsorSegment;
use crate::api::subtitles::SubtitleTrack;
use crate::db::cache::VideoItem;
use crate::services::cast::CastDevice;
use crate::services::updater::UpdateInfo;

#[derive(Debug)]
#[allow(dead_code, clippy::large_enum_variant)]
pub enum AppMessage {
    // Feed & SQLite SWR sync
    FeedLoaded {
        feed_key: String,
        items: Vec<VideoItem>,
        is_from_cache: bool,
    },
    MoreFeedLoaded {
        feed_key: String,
        items: Vec<VideoItem>,
    },
    FetchProgress {
        feed_key: String,
        current: usize,
        total: usize,
    },

    // Search & Drill-down
    SearchResults {
        generation: u64,
        query: String,
        items: Vec<VideoItem>,
    },
    ChannelVideosLoaded {
        channel_key: String,
        items: Vec<VideoItem>,
    },
    VideoDetailsLoaded {
        video_id: String,
        item: VideoItem,
        subtitles: Vec<SubtitleTrack>,
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

    // Thumbnail Pipeline
    ThumbnailReady {
        video_id: String,
        image: image::DynamicImage,
        raw_bytes: Option<Vec<u8>>,
    },
    ThumbnailFailed {
        video_id: String,
    },

    // Cookie Auth Sync
    SyncFinished(Result<String, String>),

    // Non-Blocking Playback & IPC Telemetry
    PlaybackStarted {
        video_id: String,
        title: String,
        session_id: u64,
        sock_path: String,
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

    // Self-Updater
    UpdateAvailable(UpdateInfo),
    UpdateProgress(f32, String),
    UpdateSuccess(String),
    UpdateFailed(String),

    // LAN Remote & TV Cast
    CastDevicesDiscovered(Vec<CastDevice>),
    CastStarted(String),
    CastFailed(String),
    RemotePlayVideo(String),
    RemotePlayFromPos(String, f64),
    RemoteToggleSubtitle(String),
}
