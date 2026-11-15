//! Phaenna v1.0.0 — Zero-Lag Native Rust Terminal Video Theatre & Discovery Crucible.

mod api;
mod config;
mod db;
mod error;
mod messages;
mod services;
mod state;
mod ui;

use crate::api::dislike::fetch_dislikes;
use crate::api::innertube::fetch_channel_rss;
use crate::api::player::{
    detect_dependencies, ipc_socket_path, spawn_player, youtube_watch_url, PlayOptions,
};
use crate::api::sponsorblock::{
    fetch_sponsor_segments, filter_segments_for_mode, from_db_rows, to_db_rows,
};
use crate::api::ytdlp::{
    fetch_channel_videos, fetch_feed_range, fetch_feed_range_stream, fetch_video_details,
    search_youtube,
};
use crate::config::{Config, Player, Quality};
use crate::db::cache::{
    batch_upsert_feed, get_all_cached_videos, get_feed_videos, get_thumb_blob,
    search_cached_videos, store_thumb_blob, upsert_video, VideoItem,
};
use crate::db::sync::{is_stale, mark_synced, HISTORY, RECOMMENDED, SUBSCRIPTIONS, WATCH_LATER};
use crate::db::user::{
    add_to_watch_later, compute_user_analytics, get_all_progress_map, get_continue_watching,
    get_offline_videos, get_sponsorblock_segments, get_watch_later_sources, get_watched_ids,
    record_watched, remove_from_watch_later, save_sponsorblock_segments, WatchLaterSource,
};
use crate::messages::AppMessage;
use crate::services::calendar::{bucket_videos_by_week, compute_week_bounds};
use crate::services::cast::{discover_devices, resolve_tv_stream_url, start_casting};
use crate::services::cookies::{get_cookie_count, sync_browser_cookies};
use crate::services::downloader::{spawn_download_task, DownloadTask};
use crate::services::ipc::{spawn_mpv_ipc_monitor, IpcMonitorConfig};
use crate::services::remote::start_remote_server;
use crate::services::shaders::VideoMasteringProfile;
use crate::services::tailored::{build_questions, score_discovery_pool};
use crate::services::updater::{check_for_updates, spawn_self_update};
use crate::state::{
    ActiveOverlay, AppState, Category, ContentFilter, DetailViewMode, DownloadJobState, Screen,
    SortOrder,
};
use crate::ui::components::cover::prepare_16_9_thumbnail;
use crate::ui::home::{
    footer_action_at, grid_card_index_at, grid_columns, home_layout, top_bar_shelf_at,
    HomeFooterAction,
};
use crate::ui::settings::{filtered_hub_items, HubActionId};
use clap::{Parser, Subcommand};
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, BeginSynchronizedUpdate, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use ratatui::{backend::CrosstermBackend, Terminal};
use ratatui_image::picker::Picker;
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, Semaphore};

#[derive(Parser)]
#[command(
    name = "phaenna",
    version,
    about = "Phaenna — Cinema-Grade Terminal YouTube Client"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Clear cached feeds & thumbnails on startup
    #[arg(long)]
    clear_cache: bool,

    /// Override Netscape cookies.txt path for this session
    #[arg(long)]
    cookies: Option<PathBuf>,

    /// Disable launching the LAN Mobile Companion Remote server
    #[arg(long)]
    no_remote: bool,
}

#[derive(Subcommand)]
enum Commands {
    /// Start standalone LAN Mobile Companion Remote QR server
    Remote {
        #[arg(short, long, default_value_t = 8485)]
        port: u16,
    },
    /// Run System Doctor runtime dependency diagnostic
    Doctor,
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::load().unwrap_or_default();

    if let Some(ref cpath) = cli.cookies {
        cfg.custom_cookies_path = Some(cpath.clone());
    }
    if cli.no_remote {
        cfg.remote.enabled = false;
    }

    if let Some(cmd) = cli.command {
        match cmd {
            Commands::Doctor => {
                let d = detect_dependencies();
                println!("Phaenna v{} ({})", env!("CARGO_PKG_VERSION"), env!("PHAENNA_GIT_SHA"));
                println!("  yt-dlp : {}", if d.has_ytdlp { "READY" } else { "MISSING" });
                println!("  mpv    : {}", if d.has_mpv { "READY" } else { "MISSING" });
                println!("  IINA   : {}", if d.has_iina { "READY" } else { "MISSING" });
                println!("  VLC    : {}", if d.has_vlc { "READY" } else { "MISSING" });
                println!("  ffmpeg : {}", if d.has_ffmpeg { "READY" } else { "MISSING" });
                return Ok(());
            }
            Commands::Remote { port } => {
                if let Some(handle) = start_remote_server(port).await {
                    println!("Phaenna LAN Remote active at {}", handle.url);
                    for line in &handle.qr_lines {
                        println!("{}", line);
                    }
                    println!("Press Ctrl+C to stop.");
                    tokio::signal::ctrl_c().await?;
                } else {
                    eprintln!("Failed to bind LAN remote on port {}", port);
                }
                return Ok(());
            }
        }
    }

    // 1. Initialize SQLite WAL Pool (<15ms cold boot)
    let db_path = Config::db_path()?;
    let pool = db::init(&db_path.to_string_lossy()).await?;
    if cli.clear_cache {
        let _ = sqlx::query("DELETE FROM feed_entries").execute(&pool).await;
        let _ = sqlx::query("DELETE FROM sync_meta").execute(&pool).await;
        let _ = sqlx::query("UPDATE videos SET thumb_blob = NULL").execute(&pool).await;
    }

    // 2. Setup AppState and hydrate immediately from SQLite
    let mut state = AppState::new(cfg.clone());
    if let Ok(cpath) = cfg.cookies_path() {
        state.cookie_count = get_cookie_count(&cpath);
    }
    hydrate_state_from_db(&mut state, &pool).await;
    state.sync_calendar_tz(&pool).await;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // 3. Setup Protocol Picker AFTER AlternateScreen (Kitty/iTerm2/Sixel/Halfblocks)
    #[cfg(unix)]
    let mut picker = Picker::from_termios().unwrap_or_else(|_| Picker::new((8, 16)));
    #[cfg(not(unix))]
    let mut picker = Picker::new((8, 16));
    picker.guess_protocol();
    state.picker = Some(picker);

    let (tx, mut rx) = mpsc::channel::<AppMessage>(256);
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(12))
        .user_agent(concat!("phaenna/", env!("CARGO_PKG_VERSION")))
        .build()?;
    let thumb_sem = Arc::new(Semaphore::new(12));

    // 4. Start LAN Companion Remote Server if configured
    if state.config.remote.enabled {
        state.remote_handle = start_remote_server(state.config.remote.port).await;
    }

    // 5. SWR Background Refresh: Refresh stale or empty shelves immediately without blocking render
    trigger_swr_feed_refreshes(&mut state, &pool, &tx, false);

    // 6. Optional non-blocking GitHub Releases check
    if state.config.update.check_on_startup {
        let c_clone = client.clone();
        let tx_u = tx.clone();
        tokio::spawn(async move {
            if let Ok(Some(info)) = check_for_updates(&c_clone).await {
                let _ = tx_u.send(AppMessage::UpdateAvailable(info)).await;
            }
        });
    }

    // ── Main Event Loop ───────────────────────────────────────────────────────
    let mut needs_redraw = true;
    loop {
        // Handle 350ms debounced remote search trigger
        if state.search_pending_remote {
            if let Some(ts) = state.search_last_typed {
                if ts.elapsed() >= Duration::from_millis(350) {
                    state.search_pending_remote = false;
                    let q = state.search_query.trim().to_string();
                    if !q.is_empty() {
                        spawn_remote_search(&state.config, q, state.search_generation, &tx);
                        state.is_searching_remote = true;
                        needs_redraw = true;
                    }
                }
            }
        }

        state.clear_expired_toast();

        while let Ok(msg) = rx.try_recv() {
            needs_redraw = true;
            handle_app_message(msg, &mut state, &pool, &tx).await;
        }

        if needs_redraw {
            let mut out_handle = io::stdout();
            let _ = execute!(out_handle, BeginSynchronizedUpdate);
            terminal.draw(|frame| ui::render_app(frame, &mut state))?;
            let _ = execute!(out_handle, EndSynchronizedUpdate);
            needs_redraw = false;

            // Dispatch pending lazy thumbnail fetches via 12-permit Semaphore
            let pending = std::mem::take(&mut state.pending_thumb_requests);
            for (vid, url) in pending {
                if state.loading_thumbs.contains(&vid) {
                    continue;
                }
                state.loading_thumbs.insert(vid.clone());
                spawn_thumb_loader(
                    vid,
                    url,
                    pool.clone(),
                    client.clone(),
                    thumb_sem.clone(),
                    tx.clone(),
                );
            }
        }

        if event::poll(Duration::from_millis(40))? {
            match event::read()? {
                Event::Resize(_, _) => needs_redraw = true,
                Event::Mouse(mouse) => {
                    if handle_mouse_event(mouse, &mut state, &pool, &tx, &client).await {
                        needs_redraw = true;
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    needs_redraw = true;
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        break;
                    }
                    if handle_key_event(key.code, &mut state, &pool, &tx, &client).await {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

async fn hydrate_state_from_db(state: &mut AppState, pool: &SqlitePool) {
    if let Ok(watched) = get_watched_ids(pool).await {
        state.watched_ids = watched;
    }
    if let Ok(prog) = get_all_progress_map(pool).await {
        state.progress_map = prog;
    }
    if let Ok(wl) = get_watch_later_sources(pool).await {
        state.watch_later_sources = wl;
    }
    if let Ok(off) = get_offline_videos(pool).await {
        state.offline_paths = off;
    }

    if let Ok(cw) = get_continue_watching(pool).await {
        if let Some(entry) = state
            .categories
            .iter_mut()
            .find(|(c, _)| *c == Category::ContinueWatching)
        {
            entry.1 = cw;
        }
    }

    for (cat, items) in &mut state.categories {
        if *cat == Category::ContinueWatching {
            continue;
        }
        if let Ok(cached) = get_feed_videos(pool, cat.feed_key()).await {
            *items = cached;
        }
    }
}

fn trigger_swr_feed_refreshes(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    force: bool,
) {
    let cookies = state.config.cookies_path().ok();
    let cfg_cache = state.config.cache.clone();

    let jobs = [
        (Category::Recommended, RECOMMENDED, cfg_cache.recommended_ttl),
        (Category::Subscriptions, SUBSCRIPTIONS, cfg_cache.subscriptions_ttl),
        (Category::History, HISTORY, cfg_cache.history_ttl),
        (Category::WatchLater, WATCH_LATER, cfg_cache.watch_later_ttl),
    ];

    for (cat, key, ttl) in jobs {
        let Some(url) = cat.url() else { continue };
        let empty = state
            .categories
            .iter()
            .find(|(c, _)| *c == cat)
            .map(|(_, v)| v.is_empty())
            .unwrap_or(true);

        state.loading_feeds.insert(key.to_string());
        let p_clone = pool.clone();
        let tx_clone = tx.clone();
        let c_clone = cookies.clone();
        let key_s = key.to_string();
        let url_s = url.to_string();

        tokio::spawn(async move {
            let stale = force || empty || is_stale(&p_clone, &key_s, ttl, now_unix()).await.unwrap_or(true);
            if !stale {
                let _ = tx_clone
                    .send(AppMessage::FeedLoaded {
                        feed_key: key_s,
                        items: Vec::new(),
                        is_from_cache: true,
                    })
                    .await;
                return;
            }

            let fetched = fetch_feed_range(c_clone.as_deref(), &url_s, 1, 16).await;
            if !fetched.is_empty() {
                let ts = now_unix();
                let _ = batch_upsert_feed(&p_clone, &key_s, &fetched, ts).await;
                let _ = mark_synced(&p_clone, &key_s, ts).await;
            }
            let _ = tx_clone
                .send(AppMessage::FeedLoaded {
                    feed_key: key_s,
                    items: fetched,
                    is_from_cache: false,
                })
                .await;
        });
    }
}

fn spawn_thumb_loader(
    video_id: String,
    url: String,
    pool: SqlitePool,
    client: reqwest::Client,
    sem: Arc<Semaphore>,
    tx: mpsc::Sender<AppMessage>,
) {
    tokio::spawn(async move {
        let Ok(_permit) = sem.acquire().await else {
            return;
        };

        // 1. Instant SQLite thumb_blob check before hitting network
        if let Ok(Some(bytes)) = get_thumb_blob(&pool, &video_id).await {
            let dec = tokio::task::spawn_blocking(move || {
                image::load_from_memory(&bytes).ok().map(prepare_16_9_thumbnail)
            })
            .await
            .ok()
            .flatten();
            if let Some(img) = dec {
                let _ = tx
                    .send(AppMessage::ThumbnailReady {
                        video_id,
                        image: img,
                        raw_bytes: None,
                    })
                    .await;
                return;
            }
        }

        // 2. Network fetch + fallback chain
        let clean_id = video_id.trim_start_matches("RD");
        let hq = format!("https://i.ytimg.com/vi/{}/hqdefault.jpg", clean_id);
        let mq = format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", clean_id);
        let candidates: Vec<String> = [url, hq, mq]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect();

        for cand in candidates {
            if let Ok(resp) = client.get(&cand).send().await {
                if resp.status().is_success() {
                    if let Ok(bytes) = resp.bytes().await {
                        let raw_vec = bytes.to_vec();
                        let dec = tokio::task::spawn_blocking(move || {
                            image::load_from_memory(&raw_vec)
                                .ok()
                                .map(|img| (prepare_16_9_thumbnail(img), raw_vec))
                        })
                        .await
                        .ok()
                        .flatten();

                        if let Some((img, raw)) = dec {
                            let _ = store_thumb_blob(&pool, &video_id, &raw).await;
                            let _ = tx
                                .send(AppMessage::ThumbnailReady {
                                    video_id,
                                    image: img,
                                    raw_bytes: Some(raw),
                                })
                                .await;
                            return;
                        }
                    }
                }
            }
        }

        let _ = tx.send(AppMessage::ThumbnailFailed { video_id }).await;
    });
}

fn spawn_remote_search(
    cfg: &Config,
    query: String,
    generation: u64,
    tx: &mpsc::Sender<AppMessage>,
) {
    let cookies = cfg.cookies_path().ok();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let items = search_youtube(cookies.as_deref(), &query, 18).await;
        let _ = tx_clone
            .send(AppMessage::SearchResults {
                generation,
                query,
                items,
            })
            .await;
    });
}

async fn handle_app_message(
    msg: AppMessage,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
) {
    match msg {
        AppMessage::FeedLoaded {
            feed_key,
            items,
            is_from_cache,
        } => {
            state.loading_feeds.remove(&feed_key);
            if !is_from_cache && !items.is_empty() {
                if let Some((_, list)) = state
                    .categories
                    .iter_mut()
                    .find(|(c, _)| c.feed_key() == feed_key)
                {
                    *list = items;
                }
            }
        }
        AppMessage::FetchProgress {
            feed_key,
            current,
            total,
        } => {
            if state.categories[state.active_category].0.feed_key() == feed_key {
                state.pagination_progress = Some((current, total));
            }
        }
        AppMessage::MoreFeedLoaded { feed_key, items } => {
            state.is_fetching_more = false;
            state.pagination_progress = None;
            if let Some((_, list)) = state
                .categories
                .iter_mut()
                .find(|(c, _)| c.feed_key() == feed_key)
            {
                let mut seen: HashSet<String> = list.iter().map(|v| v.id.clone()).collect();
                let mut added = 0usize;
                for v in &items {
                    if seen.insert(v.id.clone()) {
                        list.push(v.clone());
                        added += 1;
                    }
                }
                if !items.is_empty() {
                    let _ = batch_upsert_feed(pool, &feed_key, list, now_unix()).await;
                }
                if added > 0 {
                    state.show_toast(format!("Loaded +{} videos into {}", added, feed_key));
                }
            }
        }
        AppMessage::SearchResults {
            generation,
            query: _,
            items,
        } => {
            if generation == state.search_generation {
                state.is_searching_remote = false;
                state.search_results = items;
                state.search_cursor = 0;
            }
        }
        AppMessage::ChannelVideosLoaded { channel_key: _, items } => {
            state.is_loading_channel = false;
            state.channel_videos = items;
            state.channel_cursor = 0;
        }
        AppMessage::VideoDetailsLoaded {
            video_id,
            item,
            subtitles,
        } => {
            let _ = upsert_video(pool, &item, now_unix()).await;
            if state
                .detail_video
                .as_ref()
                .map(|v| v.id == video_id)
                .unwrap_or(false)
            {
                state.detail_video = Some(item);
                state.detail_subtitles = subtitles;
                state.detail_selected_sub_idx = 0;
            }
        }
        AppMessage::SponsorSegmentsLoaded { video_id, segments } => {
            let rows = to_db_rows(&video_id, &segments);
            let _ = save_sponsorblock_segments(pool, &video_id, &rows, now_unix()).await;
            if state
                .detail_video
                .as_ref()
                .map(|v| v.id == video_id)
                .unwrap_or(false)
            {
                state.detail_sponsor_segments = segments;
            }
        }
        AppMessage::DislikesLoaded { video_id, info } => {
            if state
                .detail_video
                .as_ref()
                .map(|v| v.id == video_id)
                .unwrap_or(false)
            {
                state.detail_dislikes = Some(info);
            }
        }
        AppMessage::ThumbnailReady {
            video_id, image, ..
        } => {
            state.insert_loaded_thumbnail(video_id, image);
        }
        AppMessage::ThumbnailFailed { video_id } => {
            state.loading_thumbs.remove(&video_id);
            *state.failed_thumbs.entry(video_id).or_insert(0) += 1;
        }
        AppMessage::SyncFinished(res) => match res {
            Ok(msg) => {
                if let Ok(cpath) = state.config.cookies_path() {
                    state.cookie_count = get_cookie_count(&cpath);
                }
                state.login_status = Some(format!("✓ {}", msg));
                state.show_toast("Cookies synced! Refreshing feeds...");
                trigger_swr_feed_refreshes(state, pool, tx, true);
            }
            Err(e) => {
                state.login_status = Some(format!("✕ Sync error: {}", e));
                state.show_toast("Cookie sync failed");
            }
        },
        AppMessage::PlaybackStarted {
            video_id,
            title,
            session_id,
            sock_path,
        } => {
            state.active_playback_video_id = Some(video_id.clone());
            state.active_playback_title = Some(title.clone());
            state.active_session_id = session_id;
            state.sponsor_skips_session = 0;
            if let Some(ref handle) = state.remote_handle {
                *handle.active_sock.write().await = Some(sock_path);
                let mut rst = handle.state.write().await;
                rst.is_playing = true;
                rst.video_id = video_id;
                rst.title = title;
                let _ = handle
                    .state_tx
                    .send(serde_json::to_string(&*rst).unwrap_or_default());
            }
        }
        AppMessage::PlaybackProgressUpdated {
            video_id,
            pos,
            duration,
        } => {
            state.active_playback_pos = pos;
            state.active_playback_dur = duration;
            state.progress_map.insert(video_id, (pos, duration));
            if let Some(ref handle) = state.remote_handle {
                let mut rst = handle.state.write().await;
                rst.time_pos = pos;
                rst.duration = duration;
                let _ = handle
                    .state_tx
                    .send(serde_json::to_string(&*rst).unwrap_or_default());
            }
        }
        AppMessage::SponsorSkipped {
            category,
            saved_secs,
            ..
        } => {
            state.sponsor_skips_session += 1;
            state.show_toast(format!("🛡 Skipped {} (+{:.0}s)", category, saved_secs));
        }
        AppMessage::VideoAutoCompleted { video_id } => {
            state.watched_ids.insert(video_id);
            if let Ok(cw) = get_continue_watching(pool).await {
                if let Some(entry) = state
                    .categories
                    .iter_mut()
                    .find(|(c, _)| *c == Category::ContinueWatching)
                {
                    entry.1 = cw;
                }
            }
            state.show_toast("✓ Marked watched (85%)");
        }
        AppMessage::PlaybackDone(sess) => {
            if sess == state.active_session_id {
                state.active_playback_video_id = None;
                state.active_playback_title = None;
                if let Some(ref handle) = state.remote_handle {
                    *handle.active_sock.write().await = None;
                    let mut rst = handle.state.write().await;
                    rst.is_playing = false;
                }
            }
        }
        AppMessage::DownloadProgress {
            video_id,
            percent,
            speed,
        } => {
            if let Some(ref mut job) = state.active_download {
                if job.video_id == video_id {
                    job.percent = percent;
                    job.speed = speed;
                }
            }
        }
        AppMessage::DownloadComplete {
            video_id,
            file_path,
        } => {
            state.offline_paths.insert(video_id.clone(), file_path.clone());
            if let Some(ref mut job) = state.active_download {
                if job.video_id == video_id {
                    job.percent = 100.0;
                    job.done_path = Some(file_path);
                }
            }
            state.show_toast("✓ Offline download complete!");
        }
        AppMessage::DownloadFailed { video_id, error } => {
            if let Some(ref mut job) = state.active_download {
                if job.video_id == video_id {
                    job.error = Some(error.clone());
                }
            }
            state.show_toast(format!("✕ Download failed: {}", error));
        }
        AppMessage::UpdateAvailable(info) => {
            let v = info.latest_version.clone();
            state.available_update = Some(info);
            state.show_toast(format!("⬆ Update {} available! Press [U]", v));
        }
        AppMessage::UpdateProgress(pct, msg) => {
            state.update_progress = pct;
            state.update_status = Some(msg);
        }
        AppMessage::UpdateSuccess(ver) => {
            state.update_progress = 100.0;
            state.update_status = Some(format!("✓ Updated to {} — restart phaenna!", ver));
            state.show_toast(format!("✓ Updated to {}!", ver));
        }
        AppMessage::UpdateFailed(err) => {
            state.update_progress = 0.0;
            state.update_status = Some(format!("✕ {}", err));
        }
        AppMessage::CastDevicesDiscovered(devs) => {
            state.is_scanning_cast = false;
            state.cast_devices = devs;
            state.cast_cursor = 0;
        }
        AppMessage::CastStarted(name) => {
            state.show_toast(format!("📺 Casting to {}", name));
        }
        AppMessage::CastFailed(err) => {
            state.show_toast(format!("✕ Cast error: {}", err));
        }
        _ => {}
    }
}

fn open_detail_for_video(
    video: VideoItem,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    if state.screen == Screen::Detail {
        if let Some(curr) = state.detail_video.take() {
            state.detail_history.push(curr);
        }
    }
    let vid_id = video.id.clone();
    state.queue_thumb_if_needed(&vid_id, &video.best_thumbnail_url());
    state.detail_video = Some(video);
    state.detail_view_mode = DetailViewMode::Overview;
    state.detail_chapter_cursor = 0;
    state.detail_scroll = 0;
    state.detail_subtitles.clear();
    state.detail_sponsor_segments.clear();
    state.detail_dislikes = None;
    state.overlay = ActiveOverlay::None;
    state.screen = Screen::Detail;

    let cookies = state.config.cookies_path().ok();
    let tx_d = tx.clone();
    let id_d = vid_id.clone();
    tokio::spawn(async move {
        if let Some((full_item, subs)) = fetch_video_details(cookies.as_deref(), &id_d).await {
            let _ = tx_d
                .send(AppMessage::VideoDetailsLoaded {
                    video_id: id_d,
                    item: full_item,
                    subtitles: subs,
                })
                .await;
        }
    });

    let c_sb = client.clone();
    let p_sb = pool.clone();
    let tx_sb = tx.clone();
    let id_sb = vid_id.clone();
    tokio::spawn(async move {
        if let Ok(cached) = get_sponsorblock_segments(&p_sb, &id_sb).await {
            if !cached.is_empty() {
                let _ = tx_sb
                    .send(AppMessage::SponsorSegmentsLoaded {
                        video_id: id_sb.clone(),
                        segments: from_db_rows(&cached),
                    })
                    .await;
                return;
            }
        }
        if let Ok(segs) = fetch_sponsor_segments(&c_sb, &id_sb).await {
            let _ = tx_sb
                .send(AppMessage::SponsorSegmentsLoaded {
                    video_id: id_sb,
                    segments: segs,
                })
                .await;
        }
    });

    let c_ryd = client.clone();
    let tx_ryd = tx.clone();
    tokio::spawn(async move {
        if let Ok(Some(info)) = fetch_dislikes(&c_ryd, &vid_id).await {
            let _ = tx_ryd
                .send(AppMessage::DislikesLoaded {
                    video_id: vid_id,
                    info,
                })
                .await;
        }
    });
}

async fn start_nonblocking_playback(
    video: &VideoItem,
    start_override: Option<f64>,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let quality = Quality::ALL
        .get(state.play_quality_idx)
        .copied()
        .unwrap_or(state.config.quality);
    let session_id = state.active_session_id.wrapping_add(1).max(1);
    let sock_path = ipc_socket_path(video.clean_id(), session_id);
    let start_pos = start_override.or_else(|| state.progress_map.get(&video.id).map(|(p, _)| *p));
    let local_file = state
        .offline_paths
        .get(&video.id)
        .map(PathBuf::from)
        .filter(|p| p.exists());

    let sub_url = if state.config.subtitles_enabled {
        state
            .detail_subtitles
            .get(state.detail_selected_sub_idx)
            .and_then(|s| s.url.clone())
    } else {
        None
    };

    let opts = PlayOptions {
        video_id: video.id.clone(),
        title: video.title.clone(),
        local_file,
        quality_format: quality.ytdl_format().to_string(),
        player: state.config.player,
        cookies_path: state.config.cookies_path().ok(),
        start_pos,
        ipc_socket: if state.config.player == Player::Mpv {
            Some(sock_path.clone())
        } else {
            None
        },
        subtitles_enabled: state.config.subtitles_enabled,
        sub_lang: Some(state.config.default_sub_lang.clone()),
        sub_url,
        sub_style: state.config.subtitle_style.clone(),
        shader_mode: state.config.shader_mode,
        mastering_profile: VideoMasteringProfile::from_metadata(video.duration, &video.title),
    };

    match spawn_player(&opts) {
        Ok(Some(_child)) => {
            let _ = record_watched(pool, Some(video), &video.id, now_unix()).await;
            state.watched_ids.insert(video.id.clone());
            state.show_toast(format!(
                "▶ Playing via {} ({})",
                state.config.player.label(),
                quality.label()
            ));

            if state.config.player == Player::Mpv {
                let mut sb_segs = if !state.detail_sponsor_segments.is_empty()
                    && state
                        .detail_video
                        .as_ref()
                        .map(|v| v.id == video.id)
                        .unwrap_or(false)
                {
                    state.detail_sponsor_segments.clone()
                } else {
                    fetch_sponsor_segments(client, &video.id)
                        .await
                        .unwrap_or_default()
                };
                sb_segs = filter_segments_for_mode(&sb_segs, state.config.sponsorblock_mode);

                let _ = tx
                    .send(AppMessage::PlaybackStarted {
                        video_id: video.id.clone(),
                        title: video.title.clone(),
                        session_id,
                        sock_path: sock_path.clone(),
                    })
                    .await;

                spawn_mpv_ipc_monitor(IpcMonitorConfig {
                    session_id,
                    video_id: video.id.clone(),
                    sock_path,
                    pool: pool.clone(),
                    tx: tx.clone(),
                    skip_segments: sb_segs,
                });
            }
        }
        Ok(None) => {}
        Err(e) => {
            state.show_toast(format!("✕ {}", e));
        }
    }
}

async fn toggle_watch_later(video: &VideoItem, state: &mut AppState, pool: &SqlitePool) {
    if state.watch_later_sources.contains_key(&video.id) {
        let _ = remove_from_watch_later(pool, &video.id).await;
        state.watch_later_sources.remove(&video.id);
        state.show_toast("Removed from Watch Later");
    } else {
        let _ = add_to_watch_later(pool, video, WatchLaterSource::Local, now_unix()).await;
        state
            .watch_later_sources
            .insert(video.id.clone(), WatchLaterSource::Local);
        state.show_toast("★ Added to Watch Later");
    }
    if let Ok(wl_items) = get_feed_videos(pool, WATCH_LATER).await {
        if let Some((_, list)) = state
            .categories
            .iter_mut()
            .find(|(c, _)| *c == Category::WatchLater)
        {
            *list = wl_items;
        }
    }
}

async fn open_analytics(state: &mut AppState, pool: &SqlitePool) {
    if let Ok(a) = compute_user_analytics(pool).await {
        state.analytics = a;
        state.analytics_loaded = true;
    }
    state.overlay = ActiveOverlay::Analytics;
}

fn recompute_tailored_pool(state: &mut AppState) {
    let pool_videos = state.all_known_videos();
    state.tailored_matches =
        score_discovery_pool(&pool_videos, &state.config.tailored_answers, &state.live_tuner);
    state.tailored_cursor = 0;
}

fn spawn_offline_download(video: &VideoItem, state: &mut AppState, pool: &SqlitePool, tx: &mpsc::Sender<AppMessage>) {
    let quality = Quality::ALL
        .get(state.play_quality_idx)
        .copied()
        .unwrap_or(state.config.quality);
    state.active_download = Some(DownloadJobState {
        video_id: video.id.clone(),
        title: video.title.clone(),
        percent: 0.0,
        speed: String::new(),
        done_path: None,
        error: None,
    });
    state.overlay = ActiveOverlay::Download;
    spawn_download_task(
        DownloadTask {
            video_id: video.id.clone(),
            title: video.title.clone(),
            channel_name: video.display_uploader().to_string(),
            quality_format: quality.ytdl_format().to_string(),
            cookies_path: state.config.cookies_path().ok(),
        },
        pool.clone(),
        tx.clone(),
    );
}

async fn execute_hub_action(
    id: HubActionId,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match id {
        HubActionId::QualityPreset => {
            state.config.quality = state.config.quality.next();
            state.play_quality_idx = Quality::ALL
                .iter()
                .position(|&q| q == state.config.quality)
                .unwrap_or(0);
            let _ = state.config.save();
        }
        HubActionId::PlayerBackend => {
            state.config.player = state.config.player.next();
            let _ = state.config.save();
        }
        HubActionId::SponsorBlockMode => {
            state.config.sponsorblock_mode = state.config.sponsorblock_mode.next();
            let _ = state.config.save();
        }
        HubActionId::VideoShaders => {
            state.config.shader_mode = state.config.shader_mode.next();
            let _ = state.config.save();
        }
        HubActionId::SubtitlesEnabled => {
            state.config.subtitles_enabled = !state.config.subtitles_enabled;
            let _ = state.config.save();
        }
        HubActionId::SubtitlePreset => {
            state.config.subtitle_style.cycle_preset();
            let _ = state.config.save();
        }
        HubActionId::SubtitleCollisionGuard => {
            state.config.subtitle_style.collision_avoidance =
                !state.config.subtitle_style.collision_avoidance;
            let _ = state.config.save();
        }
        HubActionId::AmbilightBorders => {
            state.config.ambilight_enabled = !state.config.ambilight_enabled;
            let _ = state.config.save();
        }
        HubActionId::ThemePalette => {
            state.config.theme_index = (state.config.theme_index + 1) % ui::theme::THEMES.len();
            let _ = state.config.save();
        }
        HubActionId::RemoteQrModal => {
            state.overlay = ActiveOverlay::Remote;
        }
        HubActionId::CastScan => {
            state.overlay = ActiveOverlay::Cast;
            state.is_scanning_cast = true;
            let tx_c = tx.clone();
            let t_ms = state.config.cast.scan_timeout_ms;
            tokio::spawn(async move {
                let devs = discover_devices(t_ms).await;
                let _ = tx_c.send(AppMessage::CastDevicesDiscovered(devs)).await;
            });
        }
        HubActionId::CookieSyncModal => {
            state.overlay = ActiveOverlay::Login;
        }
        HubActionId::UploadRadar => {
            state.sync_calendar_tz(pool).await;
            state.overlay = ActiveOverlay::Calendar;
        }
        HubActionId::SystemDoctor => {
            state.deps = detect_dependencies();
            state.overlay = ActiveOverlay::Setup;
        }
        HubActionId::CheckUpdates => {
            state.overlay = ActiveOverlay::Update;
            let c_u = client.clone();
            let tx_u = tx.clone();
            tokio::spawn(async move {
                if let Ok(Some(info)) = check_for_updates(&c_u).await {
                    let _ = tx_u.send(AppMessage::UpdateAvailable(info)).await;
                }
            });
        }
    }
}

async fn handle_mouse_event(
    mouse: MouseEvent,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) -> bool {
    let Ok((w, h)) = crossterm::terminal::size() else {
        return false;
    };
    let frame_area = ratatui::layout::Rect::new(0, 0, w, h);

    if state.overlay != ActiveOverlay::None || state.screen != Screen::Home {
        if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
            if state.overlay != ActiveOverlay::None {
                state.overlay = ActiveOverlay::None;
                return true;
            }
            if state.screen != Screen::Home {
                state.screen = Screen::Home;
                return true;
            }
        }
        return false;
    }

    let [top_area, _, grid_area, footer_area] = home_layout(frame_area);
    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if mouse.row == top_area.y {
                if let Some(idx) = top_bar_shelf_at(&state.categories, mouse.column) {
                    state.active_category = idx;
                    return true;
                }
            }
            if mouse.row >= grid_area.y && mouse.row < grid_area.y + grid_area.height {
                let total = state.current_filtered_videos().len();
                let scroll = state.grid_scroll_rows[state.active_category];
                if let Some(idx) =
                    grid_card_index_at(grid_area, mouse.column, mouse.row, scroll, total)
                {
                    if state.category_cursors[state.active_category] == idx {
                        if let Some(v) = state.current_home_video() {
                            open_detail_for_video(v, state, pool, tx, client);
                        }
                    } else {
                        state.category_cursors[state.active_category] = idx;
                    }
                    return true;
                }
            }
            if let Some(act) = footer_action_at(footer_area, mouse.column, mouse.row) {
                match act {
                    HomeFooterAction::Search => state.overlay = ActiveOverlay::Search,
                    HomeFooterAction::Calendar => {
                        state.sync_calendar_tz(pool).await;
                        state.overlay = ActiveOverlay::Calendar;
                    }
                    HomeFooterAction::Tailored => state.overlay = ActiveOverlay::TailoredWizard,
                    HomeFooterAction::Analytics => open_analytics(state, pool).await,
                    HomeFooterAction::Remote => state.overlay = ActiveOverlay::Remote,
                    HomeFooterAction::Cast => execute_hub_action(HubActionId::CastScan, state, pool, tx, client).await,
                    HomeFooterAction::Filter => state.overlay = ActiveOverlay::Filter,
                    HomeFooterAction::Settings => state.overlay = ActiveOverlay::Settings,
                    HomeFooterAction::Setup => state.overlay = ActiveOverlay::Setup,
                    HomeFooterAction::Help => state.overlay = ActiveOverlay::Help,
                    HomeFooterAction::Quit => {}
                }
                return true;
            }
        }
        MouseEventKind::ScrollDown => {
            let len = state.current_filtered_videos().len();
            if len > 0 {
                let cols = grid_columns(grid_area.width);
                let cur = &mut state.category_cursors[state.active_category];
                *cur = (*cur + cols).min(len - 1);
                return true;
            }
        }
        MouseEventKind::ScrollUp => {
            let cols = grid_columns(grid_area.width);
            let cur = &mut state.category_cursors[state.active_category];
            *cur = cur.saturating_sub(cols);
            return true;
        }
        _ => {}
    }
    false
}

/// Returns true only when application should quit.
async fn handle_key_event(
    code: KeyCode,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) -> bool {
    // 1. Handle active modal overlay first
    if state.overlay != ActiveOverlay::None {
        handle_overlay_key(code, state, pool, tx, client).await;
        return false;
    }

    // 2. Detail / Channel screens
    match state.screen {
        Screen::Detail => {
            match code {
                KeyCode::Esc | KeyCode::Backspace => {
                    if let Some(prev) = state.detail_history.pop() {
                        open_detail_for_video(prev, state, pool, tx, client);
                    } else {
                        state.screen = Screen::Home;
                    }
                }
                KeyCode::Tab => {
                    state.detail_view_mode = match state.detail_view_mode {
                        DetailViewMode::Overview => DetailViewMode::Chapters,
                        DetailViewMode::Chapters => DetailViewMode::Overview,
                    };
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    if state.detail_view_mode == DetailViewMode::Chapters {
                        let n = state
                            .detail_video
                            .as_ref()
                            .and_then(|v| v.chapters.as_ref())
                            .map(|c| c.len())
                            .unwrap_or(0);
                        if n > 0 {
                            state.detail_chapter_cursor = (state.detail_chapter_cursor + 1).min(n - 1);
                        }
                    } else {
                        state.detail_scroll = state.detail_scroll.saturating_add(2);
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    if state.detail_view_mode == DetailViewMode::Chapters {
                        state.detail_chapter_cursor = state.detail_chapter_cursor.saturating_sub(1);
                    } else {
                        state.detail_scroll = state.detail_scroll.saturating_sub(2);
                    }
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if let Some(v) = state.detail_video.clone() {
                        let start_ch = if state.detail_view_mode == DetailViewMode::Chapters {
                            v.chapters
                                .as_ref()
                                .and_then(|c| c.get(state.detail_chapter_cursor))
                                .map(|c| c.start_time)
                        } else {
                            None
                        };
                        start_nonblocking_playback(&v, start_ch, state, pool, tx, client).await;
                    }
                }
                KeyCode::Char('c') => {
                    if let Some(ref v) = state.detail_video {
                        let ch_name = v.display_uploader().to_string();
                        let ch_id = v.channel_id.clone();
                        state.channel_title = ch_name.clone();
                        state.channel_id = ch_id.clone();
                        state.channel_videos.clear();
                        state.channel_cursor = 0;
                        state.is_loading_channel = true;
                        state.screen = Screen::Channel;

                        let target = ch_id.unwrap_or_else(|| format!("@{}", ch_name.replace(' ', "")));
                        let cookies = state.config.cookies_path().ok();
                        let tx_ch = tx.clone();
                        let c_ch = client.clone();
                        tokio::spawn(async move {
                            let mut vids = if target.starts_with("UC") {
                                fetch_channel_rss(&c_ch, &target).await.unwrap_or_default()
                            } else {
                                Vec::new()
                            };
                            if vids.is_empty() {
                                vids = fetch_channel_videos(cookies.as_deref(), &target, 20).await;
                            }
                            let _ = tx_ch
                                .send(AppMessage::ChannelVideosLoaded {
                                    channel_key: target,
                                    items: vids,
                                })
                                .await;
                        });
                    }
                }
                KeyCode::Char('p') => state.overlay = ActiveOverlay::PlayOptions,
                KeyCode::Char('w') => {
                    if let Some(v) = state.detail_video.clone() {
                        toggle_watch_later(&v, state, pool).await;
                    }
                }
                KeyCode::Char('D') => {
                    if let Some(v) = state.detail_video.clone() {
                        spawn_offline_download(&v, state, pool, tx);
                    }
                }
                KeyCode::Char('m') => state.overlay = ActiveOverlay::Remote,
                KeyCode::Char('C') => execute_hub_action(HubActionId::CastScan, state, pool, tx, client).await,
                KeyCode::Char('s') | KeyCode::Char(',') => state.overlay = ActiveOverlay::Settings,
                KeyCode::Char('?') => state.overlay = ActiveOverlay::Help,
                KeyCode::Char('q') => return true,
                _ => {}
            }
            return false;
        }
        Screen::Channel => {
            match code {
                KeyCode::Esc | KeyCode::Backspace => state.screen = Screen::Detail,
                KeyCode::Char('j') | KeyCode::Down => {
                    if !state.channel_videos.is_empty() {
                        state.channel_cursor =
                            (state.channel_cursor + 1).min(state.channel_videos.len() - 1);
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    state.channel_cursor = state.channel_cursor.saturating_sub(1);
                }
                KeyCode::Enter => {
                    if let Some(v) = state.channel_videos.get(state.channel_cursor).cloned() {
                        open_detail_for_video(v, state, pool, tx, client);
                    }
                }
                KeyCode::Char('p') | KeyCode::Char(' ') => {
                    if let Some(v) = state.channel_videos.get(state.channel_cursor).cloned() {
                        start_nonblocking_playback(&v, None, state, pool, tx, client).await;
                    }
                }
                KeyCode::Char('q') => return true,
                _ => {}
            }
            return false;
        }
        Screen::Home => {}
    }

    // 3. Home Grid Global Keybindings
    let list_len = state.current_filtered_videos().len();
    let cols = crossterm::terminal::size()
        .map(|(w, _)| grid_columns(w))
        .unwrap_or(3);

    match code {
        KeyCode::Char('q') => return true,
        KeyCode::Char('?') => state.overlay = ActiveOverlay::Help,
        KeyCode::Char('/') => {
            state.overlay = ActiveOverlay::Search;
            state.search_query.clear();
            state.search_results.clear();
            state.search_local_results = get_all_cached_videos(pool, 16).await.unwrap_or_default();
            state.search_cursor = 0;
        }
        KeyCode::Char('s') | KeyCode::Char(',') => {
            state.settings_query.clear();
            state.settings_cursor = 0;
            state.overlay = ActiveOverlay::Settings;
        }
        KeyCode::Char('L') => state.overlay = ActiveOverlay::Login,
        KeyCode::Char('!') => {
            state.deps = detect_dependencies();
            state.overlay = ActiveOverlay::Setup;
        }
        KeyCode::Char('f') => state.overlay = ActiveOverlay::Filter,
        KeyCode::Char('p') => state.overlay = ActiveOverlay::PlayOptions,
        KeyCode::Char('a') => open_analytics(state, pool).await,
        KeyCode::Char('t') => {
            recompute_tailored_pool(state);
            state.overlay = ActiveOverlay::TailoredWizard;
        }
        KeyCode::Char('c') => {
            state.sync_calendar_tz(pool).await;
            state.overlay = ActiveOverlay::Calendar;
        }
        KeyCode::Char('m') => state.overlay = ActiveOverlay::Remote,
        KeyCode::Char('C') => execute_hub_action(HubActionId::CastScan, state, pool, tx, client).await,
        KeyCode::Char('U') => state.overlay = ActiveOverlay::Update,
        KeyCode::Char('T') => {
            state.config.theme_index = (state.config.theme_index + 1) % ui::theme::THEMES.len();
            let _ = state.config.save();
            state.show_toast(format!("🎨 Theme: {}", state.base_theme().name));
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            trigger_swr_feed_refreshes(state, pool, tx, true);
            state.show_toast("⟳ Refreshing all YouTube shelves...");
        }
        KeyCode::Char('n') | KeyCode::Char(']') => {
            let cat = state.categories[state.active_category].0;
            if let Some(url) = cat.url() {
                if !state.is_fetching_more {
                    state.is_fetching_more = true;
                    let current_len = state.categories[state.active_category].1.len();
                    let start = current_len + 1;
                    let end = start + 15;
                    let cookies = state.config.cookies_path().ok();
                    let key = cat.feed_key().to_string();
                    let tx_m = tx.clone();
                    let u = url.to_string();
                    tokio::spawn(async move {
                        fetch_feed_range_stream(cookies.as_deref(), u, key, start, end, tx_m).await;
                    });
                }
            }
        }
        KeyCode::Char('w') => {
            if let Some(v) = state.current_home_video() {
                toggle_watch_later(&v, state, pool).await;
            }
        }
        KeyCode::Char('D') => {
            if let Some(v) = state.current_home_video() {
                spawn_offline_download(&v, state, pool, tx);
            }
        }
        KeyCode::Char('i') => {
            if let Some(v) = state.current_home_video() {
                open_detail_for_video(v, state, pool, tx, client);
            }
        }
        KeyCode::Enter => {
            if let Some(v) = state.current_home_video() {
                open_detail_for_video(v, state, pool, tx, client);
            }
        }
        KeyCode::Char(' ') => {
            if let Some(v) = state.current_home_video() {
                start_nonblocking_playback(&v, None, state, pool, tx, client).await;
            }
        }
        KeyCode::Char('1') => state.active_category = 0,
        KeyCode::Char('2') => state.active_category = 1.min(state.categories.len() - 1),
        KeyCode::Char('3') => state.active_category = 2.min(state.categories.len() - 1),
        KeyCode::Char('4') => state.active_category = 3.min(state.categories.len() - 1),
        KeyCode::Char('5') => state.active_category = 4.min(state.categories.len() - 1),
        KeyCode::Tab => {
            state.active_category = (state.active_category + 1) % state.categories.len();
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if list_len > 0 {
                let c = &mut state.category_cursors[state.active_category];
                *c = (*c + 1).min(list_len - 1);
            }
        }
        KeyCode::Char('h') | KeyCode::Left => {
            let c = &mut state.category_cursors[state.active_category];
            *c = c.saturating_sub(1);
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if list_len > 0 {
                let c = &mut state.category_cursors[state.active_category];
                if *c + cols < list_len {
                    *c += cols;
                } else {
                    state.active_category = (state.active_category + 1) % state.categories.len();
                }
            } else {
                state.active_category = (state.active_category + 1) % state.categories.len();
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            let c = &mut state.category_cursors[state.active_category];
            if *c >= cols {
                *c -= cols;
            } else if state.active_category > 0 {
                state.active_category -= 1;
            }
        }
        _ => {}
    }

    false
}

async fn handle_overlay_key(
    code: KeyCode,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match state.overlay {
        ActiveOverlay::None => {}
        ActiveOverlay::Help => {
            if matches!(code, KeyCode::Esc | KeyCode::Char('?') | KeyCode::Char('q')) {
                state.overlay = ActiveOverlay::None;
            } else if code == KeyCode::Char('s') {
                state.overlay = ActiveOverlay::Settings;
            }
        }
        ActiveOverlay::Setup => match code {
            KeyCode::Esc | KeyCode::Char('!') => state.overlay = ActiveOverlay::None,
            KeyCode::Char('r') => {
                state.deps = detect_dependencies();
                state.show_toast("🩺 Re-scanned runtime binaries");
            }
            KeyCode::Char('L') => state.overlay = ActiveOverlay::Login,
            _ => {}
        },
        ActiveOverlay::Login => match code {
            KeyCode::Esc | KeyCode::Char('L') => state.overlay = ActiveOverlay::None,
            KeyCode::Char('1') => {
                let cpath = match state.config.cookies_path() {
                    Ok(p) => p,
                    Err(e) => {
                        state.login_status = Some(format!("Path error: {}", e));
                        return;
                    }
                };
                let src = state.config.cookie_source;
                let tx_s = tx.clone();
                state.login_status = Some("⟳ Syncing cookies...".to_string());
                tokio::spawn(async move {
                    let res = sync_browser_cookies(src, &cpath)
                        .await
                        .map(|(_, msg)| msg)
                        .map_err(|e| e.to_string());
                    let _ = tx_s.send(AppMessage::SyncFinished(res)).await;
                });
            }
            KeyCode::Char('2') => {
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open")
                    .arg("https://accounts.google.com")
                    .spawn();
                #[cfg(target_os = "linux")]
                let _ = std::process::Command::new("xdg-open")
                    .arg("https://accounts.google.com")
                    .spawn();
                state.login_status = Some("Opened accounts.google.com in browser.".to_string());
            }
            KeyCode::Char('3') => {
                state.config.cookie_source = state.config.cookie_source.next();
                let _ = state.config.save();
            }
            _ => {}
        },
        ActiveOverlay::Filter => match code {
            KeyCode::Esc | KeyCode::Char('f') | KeyCode::Enter => {
                state.overlay = ActiveOverlay::None;
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
                state.filter_modal_section = 1 - state.filter_modal_section;
                state.filter_modal_option = 0;
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Left | KeyCode::Char('h') => {
                let dir: isize = if matches!(code, KeyCode::Right | KeyCode::Char('l')) { 1 } else { -1 };
                if state.filter_modal_section == 0 {
                    let next = (state.filter_modal_option as isize + dir).rem_euclid(3) as usize;
                    state.filter_modal_option = next;
                    state.content_filter = match next {
                        1 => ContentFilter::Longform,
                        2 => ContentFilter::Shorts,
                        _ => ContentFilter::All,
                    };
                } else {
                    let next = (state.filter_modal_option as isize + dir).rem_euclid(5) as usize;
                    state.filter_modal_option = next;
                    state.sort_order = match next {
                        1 => SortOrder::Newest,
                        2 => SortOrder::DurationLongest,
                        3 => SortOrder::DurationShortest,
                        4 => SortOrder::ChannelAZ,
                        _ => SortOrder::Default,
                    };
                }
            }
            _ => {}
        },
        ActiveOverlay::PlayOptions => match code {
            KeyCode::Esc | KeyCode::Char('p') => state.overlay = ActiveOverlay::None,
            KeyCode::Left | KeyCode::Char('h') => {
                state.play_modal_section = state.play_modal_section.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Char('l') => {
                state.play_modal_section = (state.play_modal_section + 1).min(3);
            }
            KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
                let down = matches!(code, KeyCode::Down | KeyCode::Char('j'));
                match state.play_modal_section {
                    0 => {
                        let len = Quality::ALL.len();
                        state.play_quality_idx = if down {
                            (state.play_quality_idx + 1) % len
                        } else {
                            (state.play_quality_idx + len - 1) % len
                        };
                        state.config.quality = Quality::ALL[state.play_quality_idx];
                    }
                    1 => state.config.player = state.config.player.next(),
                    2 => state.config.sponsorblock_mode = state.config.sponsorblock_mode.next(),
                    _ => state.config.subtitles_enabled = !state.config.subtitles_enabled,
                }
                let _ = state.config.save();
            }
            KeyCode::Enter => {
                state.overlay = ActiveOverlay::None;
                if let Some(v) = state.active_selected_video() {
                    start_nonblocking_playback(&v, None, state, pool, tx, client).await;
                }
            }
            _ => {}
        },
        ActiveOverlay::Search => match code {
            KeyCode::Esc => state.overlay = ActiveOverlay::None,
            KeyCode::Up => state.search_cursor = state.search_cursor.saturating_sub(1),
            KeyCode::Down => {
                let total = state.combined_search_items().len();
                if total > 0 {
                    state.search_cursor = (state.search_cursor + 1).min(total - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                    open_detail_for_video(v, state, pool, tx, client);
                } else {
                    let q = state.search_query.trim().to_string();
                    if !q.is_empty() {
                        state.search_generation += 1;
                        state.search_pending_remote = false;
                        state.is_searching_remote = true;
                        spawn_remote_search(&state.config, q, state.search_generation, tx);
                    }
                }
            }
            KeyCode::Char('p') => {
                if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                    state.overlay = ActiveOverlay::None;
                    start_nonblocking_playback(&v, None, state, pool, tx, client).await;
                }
            }
            KeyCode::Backspace => {
                state.search_query.pop();
                state.search_generation += 1;
                state.search_local_results =
                    search_cached_videos(pool, &state.search_query)
                        .await
                        .unwrap_or_default();
                state.search_last_typed = Some(Instant::now());
                state.search_pending_remote = !state.search_query.trim().is_empty();
                state.search_cursor = 0;
            }
            KeyCode::Char(c) => {
                state.search_query.push(c);
                state.search_generation += 1;
                state.search_local_results =
                    search_cached_videos(pool, &state.search_query)
                        .await
                        .unwrap_or_default();
                state.search_last_typed = Some(Instant::now());
                state.search_pending_remote = true;
                state.search_cursor = 0;
            }
            _ => {}
        },
        ActiveOverlay::Settings => match code {
            KeyCode::Esc => state.overlay = ActiveOverlay::None,
            KeyCode::Up => state.settings_cursor = state.settings_cursor.saturating_sub(1),
            KeyCode::Down => {
                let len = filtered_hub_items(state).len();
                if len > 0 {
                    state.settings_cursor = (state.settings_cursor + 1).min(len - 1);
                }
            }
            KeyCode::Enter => {
                let items = filtered_hub_items(state);
                if let Some(sel) = items.get(state.settings_cursor) {
                    execute_hub_action(sel.id, state, pool, tx, client).await;
                }
            }
            KeyCode::Backspace => {
                state.settings_query.pop();
                state.settings_cursor = 0;
            }
            KeyCode::Char(c) => {
                state.settings_query.push(c);
                state.settings_cursor = 0;
            }
            _ => {}
        },
        ActiveOverlay::Analytics => match code {
            KeyCode::Esc | KeyCode::Char('a') => state.overlay = ActiveOverlay::None,
            KeyCode::Char('r') => open_analytics(state, pool).await,
            _ => {}
        },
        ActiveOverlay::TailoredWizard => {
            let questions = build_questions();
            let step = state.wizard_step.min(questions.len() - 1);
            let q = &questions[step];
            match code {
                KeyCode::Esc => state.overlay = ActiveOverlay::None,
                KeyCode::Up | KeyCode::Char('k') => {
                    state.wizard_cursor = state.wizard_cursor.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    state.wizard_cursor = (state.wizard_cursor + 1).min(q.options.len() - 1);
                }
                KeyCode::Char(' ') if q.is_multi_select => {
                    if state.config.tailored_answers.deal_breakers.contains(&state.wizard_cursor) {
                        state.config.tailored_answers.deal_breakers.remove(&state.wizard_cursor);
                    } else {
                        state.config.tailored_answers.deal_breakers.insert(state.wizard_cursor);
                    }
                    let _ = state.config.save();
                }
                KeyCode::Char('s') => {
                    recompute_tailored_pool(state);
                    state.overlay = ActiveOverlay::TailoredResults;
                }
                KeyCode::Enter => {
                    if !q.is_multi_select && step < 5 {
                        state.config.tailored_answers.choices[step] = state.wizard_cursor;
                        let _ = state.config.save();
                    }
                    if step + 1 < questions.len() {
                        state.wizard_step += 1;
                        state.wizard_cursor = 0;
                    } else {
                        recompute_tailored_pool(state);
                        state.overlay = ActiveOverlay::TailoredSummary;
                    }
                }
                _ => {}
            }
        }
        ActiveOverlay::TailoredSummary => match code {
            KeyCode::Esc => state.overlay = ActiveOverlay::None,
            KeyCode::Char('r') => {
                state.wizard_step = 0;
                state.wizard_cursor = 0;
                state.overlay = ActiveOverlay::TailoredWizard;
            }
            KeyCode::Enter => {
                recompute_tailored_pool(state);
                state.overlay = ActiveOverlay::TailoredResults;
            }
            _ => {}
        },
        ActiveOverlay::TailoredResults => match code {
            KeyCode::Esc | KeyCode::Char('t') => state.overlay = ActiveOverlay::None,
            KeyCode::Up | KeyCode::Char('k') => {
                state.tailored_cursor = state.tailored_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !state.tailored_matches.is_empty() {
                    state.tailored_cursor =
                        (state.tailored_cursor + 1).min(state.tailored_matches.len() - 1);
                }
            }
            KeyCode::Char('v') => {
                state.live_tuner.cycle_tier();
                recompute_tailored_pool(state);
            }
            KeyCode::Char('s') => {
                state.live_tuner.exclude_shorts = !state.live_tuner.exclude_shorts;
                recompute_tailored_pool(state);
            }
            KeyCode::Char('x') => {
                if let Some(m) = state.tailored_matches.get(state.tailored_cursor) {
                    state.live_tuner.dismissed_ids.insert(m.video.id.clone());
                    recompute_tailored_pool(state);
                }
            }
            KeyCode::Char('0') => {
                state.live_tuner.clear_adjustments();
                recompute_tailored_pool(state);
            }
            KeyCode::Char('z') => {
                state.wizard_step = 0;
                state.wizard_cursor = 0;
                state.overlay = ActiveOverlay::TailoredWizard;
            }
            KeyCode::Enter => {
                if let Some(m) = state.tailored_matches.get(state.tailored_cursor).cloned() {
                    open_detail_for_video(m.video, state, pool, tx, client);
                }
            }
            _ => {}
        },
        ActiveOverlay::Calendar => match code {
            KeyCode::Esc | KeyCode::Char('c') => state.overlay = ActiveOverlay::None,
            KeyCode::Left | KeyCode::Char('h') => {
                state.calendar_day_cursor = (state.calendar_day_cursor + 6) % 7;
                state.calendar_item_cursor = 0;
            }
            KeyCode::Right | KeyCode::Char('l') => {
                state.calendar_day_cursor = (state.calendar_day_cursor + 1) % 7;
                state.calendar_item_cursor = 0;
            }
            KeyCode::Char('[') | KeyCode::Char(']') => {
                let delta = if code == KeyCode::Char(']') { 1 } else { -1 };
                state.calendar_bounds = compute_week_bounds(
                    now_unix(),
                    state.calendar_tz_offset,
                    state.calendar_bounds.week_offset + delta,
                );
                state.calendar_item_cursor = 0;
            }
            KeyCode::Char('f') => {
                state.calendar_filter = state.calendar_filter.next();
                state.calendar_item_cursor = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.calendar_item_cursor = state.calendar_item_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.calendar_item_cursor = state.calendar_item_cursor.saturating_add(1);
            }
            KeyCode::Char('w') | KeyCode::Enter => {
                let pool_v = state.all_known_videos();
                let wl_ids: HashSet<String> = state.watch_later_sources.keys().cloned().collect();
                let buckets = bucket_videos_by_week(
                    &pool_v,
                    &state.calendar_bounds,
                    state.calendar_tz_offset,
                    &state.watched_ids,
                    &wl_ids,
                    state.calendar_filter,
                );
                if let Some(day_list) = buckets.get(&state.calendar_day_cursor) {
                    if let Some(entry) = day_list.get(state.calendar_item_cursor.min(day_list.len().saturating_sub(1))) {
                        let vid = entry.video.clone();
                        if code == KeyCode::Char('w') {
                            toggle_watch_later(&vid, state, pool).await;
                        } else {
                            open_detail_for_video(vid, state, pool, tx, client);
                        }
                    }
                }
            }
            _ => {}
        },
        ActiveOverlay::Remote => match code {
            KeyCode::Esc | KeyCode::Char('m') => state.overlay = ActiveOverlay::None,
            KeyCode::Char('o') => {
                if let Some(ref handle) = state.remote_handle {
                    #[cfg(target_os = "macos")]
                    let _ = std::process::Command::new("open").arg(&handle.url).spawn();
                    #[cfg(target_os = "linux")]
                    let _ = std::process::Command::new("xdg-open").arg(&handle.url).spawn();
                }
            }
            _ => {}
        },
        ActiveOverlay::Cast => match code {
            KeyCode::Esc | KeyCode::Char('C') => state.overlay = ActiveOverlay::None,
            KeyCode::Char('s') => {
                execute_hub_action(HubActionId::CastScan, state, pool, tx, client).await;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.cast_cursor = state.cast_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if !state.cast_devices.is_empty() {
                    state.cast_cursor = (state.cast_cursor + 1).min(state.cast_devices.len() - 1);
                }
            }
            KeyCode::Enter => {
                if let (Some(dev), Some(vid)) = (
                    state.cast_devices.get(state.cast_cursor).cloned(),
                    state.active_selected_video(),
                ) {
                    let tx_cast = tx.clone();
                    let name = dev.name.clone();
                    let start_pos = state.progress_map.get(&vid.id).map(|(p, _)| *p).unwrap_or(0.0);
                    state.show_toast(format!("📺 Resolving stream for {}...", name));
                    tokio::spawn(async move {
                        match resolve_tv_stream_url(&vid.id).await {
                            Ok(url) => match start_casting(&dev, &url, start_pos).await {
                                Ok(_) => {
                                    let _ = tx_cast.send(AppMessage::CastStarted(name)).await;
                                }
                                Err(e) => {
                                    let _ = tx_cast.send(AppMessage::CastFailed(e)).await;
                                }
                            },
                            Err(e) => {
                                let _ = tx_cast.send(AppMessage::CastFailed(e)).await;
                            }
                        }
                    });
                } else if state.cast_devices.is_empty() {
                    // Fallback: check youtube_watch_url diagnostic cleanly
                    let _ = youtube_watch_url("dQw4w9WgXcQ");
                }
            }
            _ => {}
        },
        ActiveOverlay::Download => {
            if matches!(code, KeyCode::Esc | KeyCode::Char('D')) {
                state.overlay = ActiveOverlay::None;
            }
        }
        ActiveOverlay::Update => match code {
            KeyCode::Esc | KeyCode::Char('U') => state.overlay = ActiveOverlay::None,
            KeyCode::Char('r') => {
                execute_hub_action(HubActionId::CheckUpdates, state, pool, tx, client).await;
            }
            KeyCode::Enter => {
                if let Some(info) = state.available_update.clone() {
                    spawn_self_update(client.clone(), info, tx.clone());
                }
            }
            _ => {}
        },
    }
}
