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
use crate::api::innertube::{
    browse_id_for_feed_key, fetch_channel_rss, fetch_channel_shorts, fetch_comment_replies,
    fetch_innertube_shelf, fetch_search_suggestions, fetch_suggested_shorts,
    fetch_video_card_enrichment, fetch_video_comments, innertube_edit_watch_later,
    post_video_comment, report_youtube_watch_progress, search_innertube, VideoComment,
};
use crate::api::player::{
    detect_dependencies, ipc_socket_path, spawn_player, youtube_watch_url, PlayOptions,
};
use crate::api::sponsorblock::{
    fetch_sponsor_segments, filter_segments_for_mode_with_outro, from_db_rows, to_db_rows,
};
use crate::api::ytdlp::{
    fetch_channel_videos, fetch_feed_range, fetch_feed_range_stream, fetch_video_details,
    search_youtube,
};
use crate::config::{
    next_dub_lang, Config, MotionMode, PerformanceMode, Player, Quality, VideoShaderMode,
};
use crate::db::{delete_account, set_active_account, upsert_account};
use crate::db::cache::{
    batch_upsert_feed, enrich_video_stats, get_all_cached_videos, get_feed_videos, get_thumb_blob,
    search_cached_videos, store_thumb_blob, upsert_video, VideoItem,
};
use crate::db::sync::{
    is_stale, last_synced, mark_synced, HISTORY, RECOMMENDED, SHORTS_SUBSCRIBED, SHORTS_SUGGESTED,
    SUBSCRIPTIONS, WATCH_LATER,
};
use crate::db::user::{
    add_to_watch_later, clear_finished_continue_watching, compute_user_analytics,
    get_all_progress_map, get_continue_watching, get_continue_watching_meta_map,
    get_local_watch_history_videos, get_offline_videos, get_sponsorblock_segments,
    get_user_video_comments, get_watch_later_sources, get_watched_ids,
    move_continue_watching_item, pin_to_continue_watching, remove_continue_watching,
    remove_from_watch_later, save_sponsorblock_segments, save_user_video_comment,
    sync_youtube_watched_items, touch_watch_history, WatchLaterSource,
};
use crate::messages::AppMessage;
use crate::services::auth::{
    now_unix_secs, poll_device_code_until_done, refresh_session_token, request_device_code,
    resolve_oauth_client, run_kitty_split_google_signin, run_loopback_pkce_flow,
    submit_direct_credentials, submit_two_factor_challenge, ChallengeKind, DirectAuthOutcome,
    LoginStep, PendingOAuthFlow, GOOGLE_YOUTUBE_SERVICE_LOGIN_URL,
};
use crate::services::calendar::{bucket_videos_by_week, compute_week_bounds};
use crate::services::cast::{discover_devices, resolve_tv_stream_url, start_casting};
use crate::services::downloader::{spawn_download_task, DownloadTask};
use crate::services::ipc::{
    spawn_mpv_ipc_monitor, IpcMonitorConfig, LiveIpcEvent, PlayerCommand,
};
use crate::services::remote::start_remote_server;
use crate::services::shaders::{download_neural_shader_pack, VideoMasteringProfile};
use crate::services::tailored::{build_questions, score_discovery_pool};
use crate::services::updater::{
    check_for_updates, cleanup_legacy_update_files, short_sha, spawn_self_update,
};
use crate::state::{
    ActiveOverlay, AppState, Category, ContentFilter, DetailViewMode, DownloadJobState,
    NowPlayingHit, PlayOptionsHit, Screen, SearchHit, SearchScope, SearchSortOrder,
    SearchTypeFilter, ShortsSource, SortOrder,
};
use crate::ui::components::cover::prepare_16_9_thumbnail;
use crate::ui::help::confirm_quit_button_rects;
use crate::ui::home::{
    compute_grid_metrics, continue_watching_row_at, effective_grid_area, footer_action_at,
    grid_card_index_at, grid_columns, home_layout, shorts_source_at, top_bar_shelf_at,
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

    // 0. Clean up leftover update files from previous self-updates
    cleanup_legacy_update_files();

    // 1. Initialize SQLite WAL Pool (<15ms cold boot)
    let db_path = Config::db_path()?;
    let pool = db::init(&db_path.to_string_lossy()).await?;
    if cli.clear_cache {
        let _ = sqlx::query("DELETE FROM feed_entries").execute(&pool).await;
        let _ = sqlx::query("DELETE FROM sync_meta").execute(&pool).await;
        let _ = sqlx::query("UPDATE videos SET thumb_blob = NULL").execute(&pool).await;
    }

    // 2. Setup AppState and hydrate immediately from SQLite & isolated auth vault
    let mut state = AppState::new(cfg.clone());
    if let Ok(cpath) = cfg.cookies_path() {
        let _ = state.auth_vault.sync_isolated_cookie_jar(&cpath);
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

    // 5. Silent token refresh if active OAuth2 session is expiring, plus SWR feed hydration
    if let Some(mut acct) = state.auth_vault.active_account().cloned() {
        if acct.is_token_expiring(now_unix_secs()) && acct.refresh_token.is_some() {
            let oauth = resolve_oauth_client(&state.config.google_oauth);
            let c_auth = client.clone();
            let tx_auth = tx.clone();
            tokio::spawn(async move {
                match refresh_session_token(&c_auth, oauth.as_ref(), &mut acct).await {
                    Ok(true) => {
                        let _ = tx_auth.send(AppMessage::AuthRefreshed(acct)).await;
                    }
                    Ok(false) => {}
                    Err(e) => {
                        let _ = tx_auth.send(AppMessage::AuthFailed(e.to_string())).await;
                    }
                }
            });
        }
    }
    trigger_swr_feed_refreshes(&mut state, &pool, &tx, &client, false);

    // 6. Optional non-blocking GitHub Releases check (supports SemVer & rolling main@sha)
    if state.config.update.check_on_startup {
        spawn_update_check(
            &pool,
            &tx,
            &client,
            state.config.update.auto_install,
            state.config.update.check_interval,
            true,
        );
    }

    // ── Main Event Loop ───────────────────────────────────────────────────────
    let mut needs_redraw = true;
    let mut last_suggest_gen: u64 = 0;
    loop {
        // Handle 120ms autocomplete suggestions & 350ms InnerTube remote search
        if state.search_pending_remote {
            if let Some(ts) = state.search_last_typed {
                let q = state.search_query.trim().to_string();
                if !q.is_empty()
                    && ts.elapsed() >= Duration::from_millis(120)
                    && last_suggest_gen != state.search_generation
                {
                    last_suggest_gen = state.search_generation;
                    spawn_search_suggestions(&client, q.clone(), state.search_generation, &tx);
                }
                if ts.elapsed() >= Duration::from_millis(350) {
                    state.search_pending_remote = false;
                    if !q.is_empty()
                        && matches!(state.search_scope, SearchScope::All | SearchScope::LiveYt)
                    {
                        state.push_search_history(&q);
                        spawn_remote_search(
                            &client,
                            state.auth_vault.active_account().cloned(),
                            &state.config,
                            q,
                            state.search_params_token(),
                            state.search_generation,
                            &tx,
                        );
                        state.is_searching_remote = true;
                        needs_redraw = true;
                    }
                }
            }
        }

        state.clear_expired_toast();

        if state.screen == Screen::Home && state.overlay == ActiveOverlay::None {
            if let Some(v) = state.current_home_video() {
                let card_inner_w = crossterm::terminal::size()
                    .map(|(w, h)| {
                        let [_, _, grid, _] = home_layout(ratatui::layout::Rect::new(0, 0, w, h));
                        let m = compute_grid_metrics(grid);
                        m.base_card_width.saturating_sub(2) as usize
                    })
                    .unwrap_or(28);
                if v.title.chars().count() > card_inner_w.max(12)
                    && state.ticker_focus_started_at.elapsed() >= Duration::from_millis(640)
                    && state.ticker_last_step_at.elapsed() >= Duration::from_millis(160)
                {
                    state.ticker_step = state.ticker_step.wrapping_add(1);
                    state.ticker_last_step_at = Instant::now();
                    needs_redraw = true;
                }
            }
        }

        while let Ok(msg) = rx.try_recv() {
            needs_redraw = true;
            handle_app_message(msg, &mut state, &pool, &tx, &client).await;
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

        let is_animating = state.toast.is_some()
            || state.has_live_ipc()
            || !state.loading_feeds.is_empty()
            || !state.loading_thumbs.is_empty()
            || state.is_searching_remote
            || state.is_loading_channel
            || state.search_pending_remote;
        let poll_ms = if is_animating { 8 } else { 45 };
        if event::poll(Duration::from_millis(poll_ms))? {
            let prev_focus = (
                state.screen,
                state.active_category,
                state.category_cursors[state.active_category],
            );
            match event::read()? {
                Event::Resize(_, _) => needs_redraw = true,
                Event::Mouse(mouse) => {
                    if handle_mouse_event(mouse, &mut state, &pool, &tx, &client).await {
                        needs_redraw = true;
                    }
                    if state.should_quit {
                        break;
                    }
                }
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    needs_redraw = true;
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        && key.code == KeyCode::Char('c')
                    {
                        break;
                    }
                    if handle_key_event(key.code, key.modifiers, &mut state, &pool, &tx, &client).await
                        || state.should_quit
                    {
                        break;
                    }
                }
                _ => {}
            }
            let new_focus = (
                state.screen,
                state.active_category,
                state.category_cursors[state.active_category],
            );
            if new_focus != prev_focus {
                state.reset_ticker();
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
    if let Ok(cw_meta) = get_continue_watching_meta_map(pool).await {
        state.continue_watching_meta = cw_meta;
    }

    for (cat, items) in &mut state.categories {
        if *cat == Category::ContinueWatching || *cat == Category::Shorts {
            continue;
        }
        if let Ok(mut cached) = get_feed_videos(pool, cat.feed_key()).await {
            if *cat == Category::Recommended {
                cached.retain(|v| !v.is_shorts());
            }
            *items = cached;
        }
    }

    if let Ok(cached_sug) = get_feed_videos(pool, SHORTS_SUGGESTED).await {
        state.shorts_suggested = cached_sug;
    }
    if let Ok(cached_sub) = get_feed_videos(pool, SHORTS_SUBSCRIBED).await {
        state.shorts_subscribed = cached_sub;
    }
    state.sync_active_shorts_list();

    if let Ok(local_hist) = get_local_watch_history_videos(pool, 200).await {
        if let Some((_, hist_list)) = state
            .categories
            .iter_mut()
            .find(|(c, _)| *c == Category::History)
        {
            let mut merged = Vec::new();
            let mut seen = HashSet::new();
            for v in local_hist.into_iter().chain(std::mem::take(hist_list)) {
                let cid = v.clean_id().to_string();
                state.watched_ids.insert(v.id.clone());
                state.watched_ids.insert(cid.clone());
                if seen.insert(cid) {
                    merged.push(v);
                }
            }
            *hist_list = merged;
        }
    } else if let Some((_, hist_list)) = state
        .categories
        .iter()
        .find(|(c, _)| *c == Category::History)
    {
        for v in hist_list {
            state.watched_ids.insert(v.id.clone());
            state.watched_ids.insert(v.clean_id().to_string());
        }
    }

    state.prune_watched_from_recommended();
}

fn trigger_swr_feed_refreshes(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
    force: bool,
) {
    let cookies = state.config.cookies_path().ok();
    let cfg_cache = state.config.cache.clone();
    let session_opt = state.auth_vault.active_account().cloned();

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
        let client_clone = client.clone();
        let session_clone = session_opt.clone();
        let key_s = key.to_string();
        let url_s = url.to_string();

        tokio::spawn(async move {
            let force_history = key_s == HISTORY && session_clone.is_some();
            let stale = force
                || empty
                || force_history
                || is_stale(&p_clone, &key_s, ttl, now_unix()).await.unwrap_or(true);
            if !stale {
                let _ = tx_clone
                    .send(AppMessage::FeedLoaded {
                        feed_key: key_s,
                        items: Vec::new(),
                        continuation: None,
                        is_from_cache: true,
                    })
                    .await;
                return;
            }

            let mut fetched = Vec::new();
            let mut next_continuation: Option<String> = None;
            let mut all_watched_progress: Vec<(String, u8)> = Vec::new();
            let watched_set = if key_s == RECOMMENDED {
                get_watched_ids(&p_clone).await.unwrap_or_default()
            } else {
                HashSet::new()
            };

            if let Some(ref session) = session_clone {
                if let Some(browse_id) = browse_id_for_feed_key(&key_s) {
                    if let Ok(page) =
                        fetch_innertube_shelf(&client_clone, session, browse_id, None).await
                    {
                        all_watched_progress.extend(page.watched_progress);
                        next_continuation = page.continuation;
                        if key_s == RECOMMENDED {
                            let overlay_watched: HashSet<&str> = all_watched_progress
                                .iter()
                                .filter(|(_, pct)| *pct > 0)
                                .map(|(id, _)| id.as_str())
                                .collect();
                            fetched = page
                                .videos
                                .into_iter()
                                .filter(|v| {
                                    !v.is_shorts()
                                        && !watched_set.contains(&v.id)
                                        && !watched_set.contains(v.clean_id())
                                        && !overlay_watched.contains(v.id.as_str())
                                        && !overlay_watched.contains(v.clean_id())
                                })
                                .collect();
                        } else {
                            fetched = page.videos;
                        }

                        // Automatically chain up to 2 pages if initial shelf has fewer than target items
                        let target_min = if key_s == RECOMMENDED { 24 } else { 30 };
                        for _ in 0..2 {
                            if fetched.len() >= target_min {
                                break;
                            }
                            let Some(tok) = next_continuation.take() else {
                                break;
                            };
                            if let Ok(page2) = fetch_innertube_shelf(
                                &client_clone,
                                session,
                                browse_id,
                                Some(&tok),
                            )
                            .await
                            {
                                all_watched_progress.extend(page2.watched_progress);
                                let overlay_watched: HashSet<&str> = all_watched_progress
                                    .iter()
                                    .filter(|(_, pct)| *pct > 0)
                                    .map(|(id, _)| id.as_str())
                                    .collect();
                                let mut seen: HashSet<String> =
                                    fetched.iter().map(|v| v.id.clone()).collect();
                                for v in page2.videos {
                                    if key_s == RECOMMENDED
                                        && (v.is_shorts()
                                            || watched_set.contains(&v.id)
                                            || watched_set.contains(v.clean_id())
                                            || overlay_watched.contains(v.id.as_str())
                                            || overlay_watched.contains(v.clean_id()))
                                    {
                                        continue;
                                    }
                                    if seen.insert(v.id.clone()) {
                                        fetched.push(v);
                                    }
                                }
                                next_continuation = page2.continuation;
                            } else {
                                next_continuation = Some(tok);
                                break;
                            }
                        }
                    }
                }
            }
            if fetched.is_empty() && next_continuation.is_none() {
                let raw = fetch_feed_range(c_clone.as_deref(), &url_s, 1, 28).await;
                if key_s == RECOMMENDED {
                    fetched = raw
                        .into_iter()
                        .filter(|v| {
                            !v.is_shorts()
                                && !watched_set.contains(&v.id)
                                && !watched_set.contains(v.clean_id())
                        })
                        .collect();
                } else {
                    fetched = raw;
                }
            }

            let ts = now_unix();
            if !fetched.is_empty() || !all_watched_progress.is_empty() {
                let _ = sync_youtube_watched_items(
                    &p_clone,
                    &fetched,
                    &all_watched_progress,
                    key_s == HISTORY,
                    ts,
                )
                .await;
            }

            if key_s == HISTORY {
                if let Ok(local_hist) = get_local_watch_history_videos(&p_clone, 200).await {
                    let mut merged = Vec::new();
                    let mut seen = HashSet::new();
                    for v in local_hist.into_iter().chain(fetched) {
                        if seen.insert(v.clean_id().to_string()) {
                            merged.push(v);
                        }
                    }
                    fetched = merged;
                }
            }

            if !fetched.is_empty() {
                let _ = batch_upsert_feed(&p_clone, &key_s, &fetched, ts).await;
                let _ = mark_synced(&p_clone, &key_s, ts).await;
            }
            let _ = tx_clone
                .send(AppMessage::FeedLoaded {
                    feed_key: key_s,
                    items: fetched,
                    continuation: next_continuation,
                    is_from_cache: false,
                })
                .await;
        });
    }

    // Refresh Shorts (Algorithm Suggested + Subscribed) concurrently
    let sub_channel_ids: Vec<String> = state
        .categories
        .iter()
        .find(|(c, _)| *c == Category::Subscriptions)
        .map(|(_, list)| {
            let mut ch_seen = HashSet::new();
            let mut ch_ids = Vec::new();
            for v in list {
                if let Some(ref cid) = v.channel_id {
                    if cid.starts_with("UC") && ch_seen.insert(cid.clone()) {
                        ch_ids.push(cid.clone());
                        if ch_ids.len() >= 10 {
                            break;
                        }
                    }
                }
            }
            ch_ids
        })
        .unwrap_or_default();

    let sug_empty = state.shorts_suggested.is_empty();
    state.loading_feeds.insert(SHORTS_SUGGESTED.to_string());
    {
        let p_sh = pool.clone();
        let tx_sh = tx.clone();
        let c_sh = client.clone();
        let sess_sh = session_opt.clone();
        let ck_sh = cookies.clone();
        tokio::spawn(async move {
            let stale = force
                || sug_empty
                || is_stale(&p_sh, SHORTS_SUGGESTED, cfg_cache.recommended_ttl, now_unix())
                    .await
                    .unwrap_or(true);
            if !stale {
                let _ = tx_sh
                    .send(AppMessage::FeedLoaded {
                        feed_key: SHORTS_SUGGESTED.to_string(),
                        items: Vec::new(),
                        continuation: None,
                        is_from_cache: true,
                    })
                    .await;
                return;
            }
            let mut fetched = Vec::new();
            let mut next_cont = None;
            if let Ok(mut p1) = fetch_suggested_shorts(&c_sh, sess_sh.as_ref(), None).await {
                next_cont = p1.continuation.take();
                fetched.append(&mut p1.videos);
                for _ in 0..2 {
                    if fetched.len() >= 18 {
                        break;
                    }
                    let Some(tok) = next_cont.take() else {
                        break;
                    };
                    if let Ok(p2) = fetch_suggested_shorts(&c_sh, sess_sh.as_ref(), Some(&tok)).await {
                        let mut seen: HashSet<String> = fetched.iter().map(|v| v.id.clone()).collect();
                        for v in p2.videos {
                            if seen.insert(v.id.clone()) {
                                fetched.push(v);
                            }
                        }
                        next_cont = p2.continuation;
                    } else {
                        next_cont = Some(tok);
                        break;
                    }
                }
            }
            if fetched.is_empty() {
                let fallback = search_youtube(ck_sh.as_deref(), "#shorts", 20).await;
                fetched = filter_short_items(fallback);
            }
            let ts = now_unix();
            if !fetched.is_empty() {
                let _ = batch_upsert_feed(&p_sh, SHORTS_SUGGESTED, &fetched, ts).await;
                let _ = mark_synced(&p_sh, SHORTS_SUGGESTED, ts).await;
            }
            let _ = tx_sh
                .send(AppMessage::FeedLoaded {
                    feed_key: SHORTS_SUGGESTED.to_string(),
                    items: fetched,
                    continuation: next_cont,
                    is_from_cache: false,
                })
                .await;
        });
    }

    let sub_shorts_empty = state.shorts_subscribed.is_empty();
    state.loading_feeds.insert(SHORTS_SUBSCRIBED.to_string());
    {
        let p_sub = pool.clone();
        let tx_sub = tx.clone();
        let c_sub = client.clone();
        let sess_sub = session_opt.clone();
        let sub_ttl = cfg_cache.subscriptions_ttl;
        tokio::spawn(async move {
            let stale = force
                || sub_shorts_empty
                || is_stale(&p_sub, SHORTS_SUBSCRIBED, sub_ttl, now_unix())
                    .await
                    .unwrap_or(true);
            if !stale {
                let _ = tx_sub
                    .send(AppMessage::FeedLoaded {
                        feed_key: SHORTS_SUBSCRIBED.to_string(),
                        items: Vec::new(),
                        continuation: None,
                        is_from_cache: true,
                    })
                    .await;
                return;
            }
            let fetched = fetch_subscribed_shorts_merged(
                &p_sub,
                &c_sub,
                sess_sub.as_ref(),
                sub_channel_ids,
                8,
            )
            .await;
            let ts = now_unix();
            if !fetched.is_empty() {
                let _ = batch_upsert_feed(&p_sub, SHORTS_SUBSCRIBED, &fetched, ts).await;
                let _ = mark_synced(&p_sub, SHORTS_SUBSCRIBED, ts).await;
            }
            let _ = tx_sub
                .send(AppMessage::FeedLoaded {
                    feed_key: SHORTS_SUBSCRIBED.to_string(),
                    items: fetched,
                    continuation: None,
                    is_from_cache: false,
                })
                .await;
        });
    }
}

fn filter_short_items(items: Vec<VideoItem>) -> Vec<VideoItem> {
    items
        .into_iter()
        .filter_map(|mut v| {
            if !v.is_shorts() {
                return None;
            }
            if v.duration_string.is_none() {
                v.duration_string = Some("SHORT".to_string());
            }
            Some(v)
        })
        .collect()
}

async fn fetch_subscribed_shorts_merged(
    pool: &SqlitePool,
    client: &reqwest::Client,
    session: Option<&crate::services::auth::GoogleAccountSession>,
    seed_channel_ids: Vec<String>,
    max_channels: usize,
) -> Vec<VideoItem> {
    let mut sub_items = get_feed_videos(pool, SUBSCRIPTIONS)
        .await
        .unwrap_or_default();
    if sub_items.is_empty() {
        if let Some(sess) = session {
            if let Ok(page) = fetch_innertube_shelf(client, sess, "FEsubscriptions", None).await {
                sub_items = page.videos;
            }
        }
    }

    let mut ch_seen = HashSet::new();
    let mut ch_ids = Vec::new();
    for cid in seed_channel_ids {
        if cid.starts_with("UC") && ch_seen.insert(cid.clone()) {
            ch_ids.push(cid);
        }
    }
    for v in &sub_items {
        if let Some(ref cid) = v.channel_id {
            if cid.starts_with("UC") && ch_seen.insert(cid.clone()) {
                ch_ids.push(cid.clone());
                if ch_ids.len() >= max_channels {
                    break;
                }
            }
        }
    }
    ch_ids.truncate(max_channels);

    let mut seen_vids = HashSet::new();
    let mut out = Vec::new();

    for mut v in sub_items {
        if v.is_shorts() && seen_vids.insert(v.clean_id().to_string()) {
            if v.duration_string.is_none() {
                v.duration_string = Some("SHORT".to_string());
            }
            out.push(v);
        }
    }

    let mut handles = Vec::with_capacity(ch_ids.len());
    for cid in ch_ids {
        let c = client.clone();
        let s = session.cloned();
        handles.push(tokio::spawn(async move {
            fetch_channel_shorts(&c, s.as_ref(), &cid)
                .await
                .unwrap_or_default()
        }));
    }

    let mut per_channel: Vec<Vec<VideoItem>> = Vec::new();
    for h in handles {
        if let Ok(vids) = h.await {
            if !vids.is_empty() {
                per_channel.push(vids);
            }
        }
    }

    let max_depth = per_channel.iter().map(|v| v.len()).max().unwrap_or(0).min(8);
    for round in 0..max_depth {
        for ch_vids in &per_channel {
            if let Some(v) = ch_vids.get(round) {
                if seen_vids.insert(v.clean_id().to_string()) {
                    out.push(v.clone());
                }
            }
        }
    }

    out
}

fn spawn_card_metadata_enrichment(
    items: &[VideoItem],
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
    session: Option<crate::services::auth::GoogleAccountSession>,
) {
    let missing_ids: Vec<String> = items
        .iter()
        .filter(|v| {
            v.view_count.is_none()
                || (v.timestamp.is_none() && v.published_text.is_none())
                || v.display_uploader() == "Unknown"
        })
        .take(18)
        .map(|v| v.id.clone())
        .collect();

    if missing_ids.is_empty() {
        return;
    }

    let p_e = pool.clone();
    let tx_e = tx.clone();
    let c_e = client.clone();
    tokio::spawn(async move {
        let sem = Arc::new(Semaphore::new(6));
        let mut handles = Vec::with_capacity(missing_ids.len());
        for vid in missing_ids {
            let permit = sem.clone().acquire_owned().await;
            let c_i = c_e.clone();
            let s_i = session.clone();
            handles.push(tokio::spawn(async move {
                let _p = permit;
                fetch_video_card_enrichment(&c_i, s_i.as_ref(), &vid)
                    .await
                    .map(|(up, vw, pt, ts)| (vid, up, vw, pt, ts))
            }));
        }

        let mut updates = Vec::new();
        for h in handles {
            if let Ok(Some(upd)) = h.await {
                updates.push(upd);
            }
        }
        if !updates.is_empty() {
            let _ = enrich_video_stats(&p_e, &updates).await;
            let _ = tx_e
                .send(AppMessage::VideosMetadataEnriched { updates })
                .await;
        }
    });
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
    client: &reqwest::Client,
    session: Option<crate::services::auth::GoogleAccountSession>,
    cfg: &Config,
    query: String,
    params: Option<String>,
    generation: u64,
    tx: &mpsc::Sender<AppMessage>,
) {
    let cookies = cfg.cookies_path().ok();
    let tx_clone = tx.clone();
    let client_clone = client.clone();
    tokio::spawn(async move {
        let mut continuation = None;
        let items = match search_innertube(
            &client_clone,
            session.as_ref(),
            &query,
            params.as_deref(),
            None,
        )
        .await
        {
            Ok(page) if !page.videos.is_empty() => {
                continuation = page.continuation;
                page.videos
            }
            _ => search_youtube(cookies.as_deref(), &query, 18).await,
        };
        let _ = tx_clone
            .send(AppMessage::SearchResults {
                generation,
                query,
                items,
                continuation,
            })
            .await;
    });
}

fn spawn_more_search_results(
    client: &reqwest::Client,
    session: Option<crate::services::auth::GoogleAccountSession>,
    query: String,
    params: Option<String>,
    continuation: String,
    generation: u64,
    tx: &mpsc::Sender<AppMessage>,
) {
    let tx_clone = tx.clone();
    let client_clone = client.clone();
    tokio::spawn(async move {
        let page = search_innertube(
            &client_clone,
            session.as_ref(),
            &query,
            params.as_deref(),
            Some(&continuation),
        )
        .await
        .unwrap_or_default();
        let _ = tx_clone
            .send(AppMessage::MoreSearchResults {
                generation,
                items: page.videos,
                continuation: page.continuation,
            })
            .await;
    });
}

fn spawn_search_suggestions(
    client: &reqwest::Client,
    query: String,
    generation: u64,
    tx: &mpsc::Sender<AppMessage>,
) {
    let tx_clone = tx.clone();
    let client_clone = client.clone();
    tokio::spawn(async move {
        let suggestions = fetch_search_suggestions(&client_clone, &query).await;
        let _ = tx_clone
            .send(AppMessage::SearchSuggestionsReady {
                generation,
                suggestions,
            })
            .await;
    });
}

fn open_channel_for_video(
    v: &VideoItem,
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
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

fn extract_youtube_video_id(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if !(trimmed.contains("youtube.com") || trimmed.contains("youtu.be")) {
        return None;
    }
    if let Some(pos) = trimmed.find("v=") {
        let cand: String = trimmed[pos + 2..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect();
        if cand.len() == 11 {
            return Some(cand);
        }
    }
    for prefix in ["youtu.be/", "/shorts/", "/live/", "/embed/"] {
        if let Some(pos) = trimmed.find(prefix) {
            let cand: String = trimmed[pos + prefix.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            if cand.len() == 11 {
                return Some(cand);
            }
        }
    }
    None
}

async fn handle_app_message(
    msg: AppMessage,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match msg {
        AppMessage::FeedLoaded {
            feed_key,
            items,
            continuation,
            is_from_cache,
        } => {
            state.loading_feeds.remove(&feed_key);
            if !is_from_cache {
                state.exhausted_feeds.remove(&feed_key);
                if let Some(tok) = continuation {
                    state.feed_continuations.insert(feed_key.clone(), tok);
                } else {
                    state.feed_continuations.remove(&feed_key);
                }
                if let Ok(watched) = get_watched_ids(pool).await {
                    state.watched_ids.extend(watched);
                }
                if let Ok(prog) = get_all_progress_map(pool).await {
                    for (k, v) in prog {
                        state.progress_map.entry(k).or_insert(v);
                    }
                }
            }
            if !is_from_cache && feed_key == HISTORY {
                for v in &items {
                    state.watched_ids.insert(v.id.clone());
                    state.watched_ids.insert(v.clean_id().to_string());
                }
                let local_hist = get_local_watch_history_videos(pool, 200)
                    .await
                    .unwrap_or_default();
                let mut merged = Vec::new();
                let mut seen = HashSet::new();
                for v in local_hist.into_iter().chain(items) {
                    let cid = v.clean_id().to_string();
                    state.watched_ids.insert(v.id.clone());
                    state.watched_ids.insert(cid.clone());
                    if seen.insert(cid) {
                        merged.push(v);
                    }
                }
                if !merged.is_empty() {
                    if let Some((_, list)) = state
                        .categories
                        .iter_mut()
                        .find(|(c, _)| *c == Category::History)
                    {
                        *list = merged;
                    }
                }
            } else if !is_from_cache && (feed_key == SHORTS_SUGGESTED || feed_key == SHORTS_SUBSCRIBED) {
                let enrich_slice = items.clone();
                if feed_key == SHORTS_SUGGESTED {
                    state.shorts_suggested = items;
                } else {
                    state.shorts_subscribed = items;
                }
                state.sync_active_shorts_list();
                spawn_card_metadata_enrichment(
                    &enrich_slice,
                    pool,
                    tx,
                    client,
                    state.auth_vault.active_account().cloned(),
                );
            } else if !is_from_cache && !items.is_empty() {
                let filtered_items: Vec<VideoItem> = if feed_key == RECOMMENDED {
                    items
                        .into_iter()
                        .filter(|v| !v.is_shorts() && !state.is_video_watched(&v.id))
                        .collect()
                } else {
                    items
                };
                spawn_card_metadata_enrichment(
                    &filtered_items,
                    pool,
                    tx,
                    client,
                    state.auth_vault.active_account().cloned(),
                );
                if let Some((_, list)) = state
                    .categories
                    .iter_mut()
                    .find(|(c, _)| c.feed_key() == feed_key)
                {
                    *list = filtered_items;
                }
            }
            if !is_from_cache {
                state.prune_watched_from_recommended();
            }
        }
        AppMessage::FetchProgress {
            feed_key,
            current,
            total,
        } => {
            if state.active_feed_key_for(state.categories[state.active_category].0) == feed_key {
                state.pagination_progress = Some((current, total));
            }
        }
        AppMessage::VideosMetadataEnriched { updates } => {
            let apply_updates = |list: &mut Vec<VideoItem>| {
                for (vid, up, vw, pt, ts) in &updates {
                    let clean = vid.trim_start_matches("RD");
                    if let Some(item) = list
                        .iter_mut()
                        .find(|v| v.id == *vid || v.clean_id() == clean)
                    {
                        if let Some(u) = up {
                            if item.display_uploader() == "Unknown" {
                                item.uploader = Some(u.clone());
                                item.channel = Some(u.clone());
                            }
                        }
                        if item.view_count.is_none() {
                            item.view_count = *vw;
                        }
                        if item.published_text.is_none() {
                            item.published_text = pt.clone();
                        }
                        if item.timestamp.is_none() {
                            item.timestamp = *ts;
                        }
                    }
                }
            };
            for (_, list) in &mut state.categories {
                apply_updates(list);
            }
            apply_updates(&mut state.shorts_suggested);
            apply_updates(&mut state.shorts_subscribed);
        }
        AppMessage::MoreFeedLoaded {
            feed_key,
            items,
            continuation,
        } => {
            state.is_fetching_more = false;
            state.pagination_progress = None;
            if let Some(tok) = continuation {
                state.feed_continuations.insert(feed_key.clone(), tok);
                state.exhausted_feeds.remove(&feed_key);
            } else {
                state.feed_continuations.remove(&feed_key);
            }
            if let Ok(watched) = get_watched_ids(pool).await {
                state.watched_ids.extend(watched);
            }
            if let Ok(prog) = get_all_progress_map(pool).await {
                for (k, v) in prog {
                    state.progress_map.entry(k).or_insert(v);
                }
            }

            let target_cat = if feed_key == SHORTS_SUGGESTED || feed_key == SHORTS_SUBSCRIBED {
                Some(Category::Shorts)
            } else {
                state
                    .categories
                    .iter()
                    .find(|(c, _)| c.feed_key() == feed_key)
                    .map(|(c, _)| *c)
            };
            let mut added_items: Vec<VideoItem> = Vec::new();
            if feed_key == SHORTS_SUGGESTED || feed_key == SHORTS_SUBSCRIBED {
                let target_buf = if feed_key == SHORTS_SUGGESTED {
                    &mut state.shorts_suggested
                } else {
                    &mut state.shorts_subscribed
                };
                let mut seen: HashSet<String> = target_buf.iter().map(|v| v.id.clone()).collect();
                for v in &items {
                    if seen.insert(v.id.clone()) {
                        target_buf.push(v.clone());
                        added_items.push(v.clone());
                    }
                }
                if !items.is_empty() {
                    let _ = batch_upsert_feed(pool, &feed_key, target_buf, now_unix()).await;
                }
                state.sync_active_shorts_list();
            } else if let Some((_, list)) = state
                .categories
                .iter_mut()
                .find(|(c, _)| c.feed_key() == feed_key)
            {
                let mut seen: HashSet<String> = list.iter().map(|v| v.id.clone()).collect();
                for v in &items {
                    if feed_key == RECOMMENDED
                        && (v.is_shorts()
                            || state.watched_ids.contains(&v.id)
                            || state.watched_ids.contains(v.clean_id()))
                    {
                        continue;
                    }
                    if seen.insert(v.id.clone()) {
                        list.push(v.clone());
                        added_items.push(v.clone());
                    }
                }
                if !items.is_empty() {
                    let _ = batch_upsert_feed(pool, &feed_key, list, now_unix()).await;
                }
            }
            if !added_items.is_empty() {
                spawn_card_metadata_enrichment(
                    &added_items,
                    pool,
                    tx,
                    client,
                    state.auth_vault.active_account().cloned(),
                );
                state.show_toast(format!("Loaded +{} videos", added_items.len()));
            } else if state.feed_continuations.contains_key(&feed_key) {
                if let Some(cat) = target_cat {
                    maybe_trigger_infinite_scroll_for(cat, state, pool, tx, client, true);
                }
            } else {
                state.exhausted_feeds.insert(feed_key);
            }
        }
        AppMessage::SearchResults {
            generation,
            query: _,
            items,
            continuation,
        } => {
            if generation == state.search_generation {
                state.is_searching_remote = false;
                state.search_results = items;
                state.search_continuation = continuation;
                state.search_cursor = 0;
            }
        }
        AppMessage::MoreSearchResults {
            generation,
            items,
            continuation,
        } => {
            if generation == state.search_generation {
                state.is_searching_remote = false;
                state.search_continuation = continuation;
                let mut seen: std::collections::HashSet<String> = state
                    .search_results
                    .iter()
                    .map(|v| v.clean_id().to_string())
                    .collect();
                for item in items {
                    if seen.insert(item.clean_id().to_string()) {
                        state.search_results.push(item);
                    }
                }
            }
        }
        AppMessage::SearchSuggestionsReady {
            generation,
            suggestions,
        } => {
            if generation == state.search_generation {
                state.search_suggestions = suggestions;
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
        AppMessage::CommentsLoaded {
            video_id,
            comments,
            create_params,
        } => {
            if state
                .detail_video
                .as_ref()
                .map(|v| v.id == video_id)
                .unwrap_or(false)
            {
                state.detail_comments_loading = false;
                if create_params.is_some() {
                    state.detail_create_comment_params = create_params;
                }
                let mut merged: Vec<VideoComment> = state
                    .detail_comments
                    .iter()
                    .filter(|c| c.is_self)
                    .cloned()
                    .collect();
                for c in comments {
                    if !merged.iter().any(|m| m.id == c.id) {
                        merged.push(c);
                    }
                }
                state.detail_comments = merged;
            }
        }
        AppMessage::CommentRepliesLoaded {
            video_id,
            parent_comment_id,
            replies,
            continuation,
        } => {
            if state
                .detail_video
                .as_ref()
                .map(|v| v.id == video_id)
                .unwrap_or(false)
            {
                if let Some(parent) = state
                    .detail_comments
                    .iter_mut()
                    .find(|c| c.id == parent_comment_id)
                {
                    parent.replies_loading = false;
                    parent.replies_continuation = continuation;
                    for r in replies {
                        if !parent.replies.iter().any(|existing| existing.id == r.id) {
                            parent.replies.push(r);
                        }
                    }
                }
            }
        }
        AppMessage::CommentPosted {
            video_id,
            comment,
            synced_remote,
        } => {
            if state
                .detail_video
                .as_ref()
                .map(|v| v.id == video_id)
                .unwrap_or(false)
                && !state.detail_comments.iter().any(|c| c.id == comment.id)
            {
                state.detail_comments.insert(0, comment);
                state.detail_comment_cursor = 0;
            }
            if synced_remote {
                state.show_toast("Comment posted to YouTube!");
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
        AppMessage::AuthDeviceCodeReady(dev) => {
            state.login_ui.busy = false;
            state.login_ui.error_banner = None;
            state.login_ui.status_banner = Some(format!(
                "QR Ready! Scan with phone camera or enter {} at google.com/device",
                dev.user_code
            ));
            state.login_ui.device_session = Some(dev);
            state.login_ui.step = LoginStep::AwaitingDeviceQr;
        }
        AppMessage::AuthKittyUrlReady { url, launcher } => {
            state.login_ui.kitty_auth_url = Some(url);
            state.login_ui.status_banner = Some(format!(
                "Opened Google SSO in {launcher} — complete login to return"
            ));
        }
        AppMessage::AuthChallengeRequired {
            kind,
            cookies,
            challenge_token,
        } => {
            state.login_ui.busy = false;
            state.login_ui.password.clear();
            state.login_ui.two_factor_code.clear();
            state.login_ui.pending_flow_cookies = cookies;
            state.login_ui.pending_challenge_token = challenge_token;
            state.login_ui.error_banner = None;
            state.login_ui.status_banner = Some(
                "2-Step Verification requested — enter your 6-digit code inside Phaenna."
                    .to_string(),
            );
            state.login_ui.step = LoginStep::TwoFactorChallenge(kind);
        }
        AppMessage::AuthNeedsModernSso { reason } => {
            state.login_ui.busy = false;
            state.login_ui.password.clear();
            state.login_ui.pending_flow_cookies.clear();
            state.login_ui.step = LoginStep::Hub;
            state.login_ui.error_banner = Some(reason);
        }
        AppMessage::AuthSignedIn(session) => {
            let label = format!("Signed in as {} ({})", session.display_name, session.email);
            state.auth_vault.upsert_session(session);
            persist_auth_vault_and_cookies(state, pool).await;
            state.login_ui.reset_transient();
            state.login_ui.step = LoginStep::Hub;
            state.login_ui.error_banner = None;
            state.login_ui.status_banner = Some(label.clone());
            state.show_toast(format!("{label} · Refreshing YouTube shelves..."));
            trigger_swr_feed_refreshes(state, pool, tx, client, true);
        }
        AppMessage::AuthRefreshed(session) => {
            state.auth_vault.upsert_session(session);
            persist_auth_vault_and_cookies(state, pool).await;
            state.login_ui.status_banner = Some("Google OAuth2 token silently refreshed".into());
        }
        AppMessage::AuthFailed(err) => {
            state.login_ui.busy = false;
            state.login_ui.error_banner = Some(err.clone());
            state.show_toast(format!("✕ Auth error: {err}"));
        }
        AppMessage::PlaybackStarted {
            video_id,
            title,
            session_id,
            sock_path,
        } => {
            state.reset_live_playback();
            state.active_playback_video_id = Some(video_id.clone());
            state.active_playback_title = Some(title.clone());
            state.active_session_id = session_id;
            state.active_playback_pos = 0.0;
            state.active_playback_dur = 0.0;
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
        AppMessage::PlaybackCommandReady { session_id, cmd_tx } => {
            if session_id == state.active_session_id {
                state.live_playback.cmd_tx = Some(cmd_tx);
            }
        }
        AppMessage::PlaybackTelemetryUpdated { session_id, event } => {
            if session_id == state.active_session_id {
                let ev_copy = event.clone();
                state.apply_live_ipc_event(event);
                if matches!(ev_copy, LiveIpcEvent::Tracks { .. })
                    && state.config.dub_preference == crate::config::DubPreference::OriginalOnly
                {
                    let selected_is_dubbed = state
                        .live_playback
                        .audio_tracks
                        .iter()
                        .any(|t| t.selected && t.is_dubbed);
                    if selected_is_dubbed {
                        if let Some(orig_aid) = state.find_audio_track_for_dub(false) {
                            if state.live_playback.active_aid != Some(orig_aid) {
                                state.live_playback.active_aid = Some(orig_aid);
                                state.send_player_cmd(PlayerCommand::SetAudioTrack(orig_aid));
                            }
                        }
                    }
                }
                if let Some(ref handle) = state.remote_handle {
                    let mut rst = handle.state.write().await;
                    match ev_copy {
                        LiveIpcEvent::Paused(p) => rst.is_playing = !p,
                        LiveIpcEvent::Volume(v) => rst.volume = v as i32,
                        LiveIpcEvent::Speed(s) => rst.speed = s,
                        LiveIpcEvent::SubVisible(sv) => rst.sub_active = sv,
                        _ => {}
                    }
                    let _ = handle
                        .state_tx
                        .send(serde_json::to_string(&*rst).unwrap_or_default());
                }
            }
        }
        AppMessage::PlaybackProgressUpdated {
            video_id,
            pos,
            duration,
        } => {
            let need_refresh = !state.is_in_continue_watching(&video_id) && pos >= 10.0;
            state.active_playback_pos = pos;
            state.active_playback_dur = duration;
            state.progress_map.insert(video_id.clone(), (pos, duration));
            if let Some(meta) = state.continue_watching_meta.get_mut(video_id.trim_start_matches("RD")) {
                meta.last_pos_secs = pos;
                if duration > 0.0 {
                    meta.duration_secs = duration;
                }
            }
            if need_refresh {
                refresh_continue_watching_state(state, pool).await;
            }
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
            state.show_toast(format!("Skipped {} (+{:.0}s)", category, saved_secs));
        }
        AppMessage::VideoAutoCompleted { video_id } => {
            let clean = video_id.trim_start_matches("RD").to_string();
            state.watched_ids.insert(video_id.clone());
            state.watched_ids.insert(clean.clone());
            state.prune_watched_from_recommended();
            refresh_continue_watching_state(state, pool).await;

            let (pos, dur) = if state.active_playback_video_id.as_deref() == Some(video_id.as_str()) {
                (state.active_playback_pos, state.active_playback_dur)
            } else {
                state
                    .progress_map
                    .get(&video_id)
                    .or_else(|| state.progress_map.get(&clean))
                    .copied()
                    .unwrap_or((0.0, 0.0))
            };
            let sess_opt = state.auth_vault.active_account().cloned();
            let cpath = state.config.cookies_path().ok();
            let c_sync = client.clone();
            tokio::spawn(async move {
                let _ = report_youtube_watch_progress(
                    &c_sync,
                    sess_opt.as_ref(),
                    cpath.as_deref(),
                    &video_id,
                    0.0,
                    pos.max(1.0),
                    (dur > 0.0).then_some(dur),
                    false,
                )
                .await;
            });

            state.show_toast("Marked watched (85%) + synced to YouTube");
        }
        AppMessage::PlaybackDone(sess) => {
            refresh_continue_watching_state(state, pool).await;
            if sess == state.active_session_id {
                let finished_vid = state.active_playback_video_id.take();
                let finished_pos = state.active_playback_pos;
                let finished_dur = state.active_playback_dur;
                state.active_playback_title = None;
                state.active_playback_pos = 0.0;
                state.active_playback_dur = 0.0;
                state.reset_live_playback();

                if let Some(vid) = finished_vid {
                    if finished_pos > 3.0 {
                        let sess_opt = state.auth_vault.active_account().cloned();
                        let cpath = state.config.cookies_path().ok();
                        let c_sync = client.clone();
                        tokio::spawn(async move {
                            let _ = report_youtube_watch_progress(
                                &c_sync,
                                sess_opt.as_ref(),
                                cpath.as_deref(),
                                &vid,
                                0.0,
                                finished_pos,
                                (finished_dur > 0.0).then_some(finished_dur),
                                false,
                            )
                            .await;
                        });
                    }
                }

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
            state.show_toast("Offline download complete!");
        }
        AppMessage::DownloadFailed { video_id, error } => {
            if let Some(ref mut job) = state.active_download {
                if job.video_id == video_id {
                    job.error = Some(error.clone());
                }
            }
            state.show_toast(format!("✕ Download failed: {}", error));
        }
        AppMessage::UpdateCheckResult(update_opt) => {
            let was_checking = state.is_checking_update;
            state.is_checking_update = false;
            if let Some(info) = update_opt {
                let new_ver = info.latest_version.clone();
                let is_brew = info.is_homebrew;
                if is_brew {
                    state.available_update = Some(info);
                    state.show_toast(format!("Phaenna {new_ver} available: brew upgrade phaenna"));
                } else if state.config.update.auto_install && !was_checking {
                    state.is_updating = true;
                    state.available_update = Some(info);
                    state.show_toast(format!("Updating Phaenna to {new_ver} in background..."));
                } else {
                    state.available_update = Some(info);
                    if state.overlay == ActiveOverlay::None {
                        state.overlay = ActiveOverlay::Update;
                    }
                    state.show_toast(format!("Update {new_ver} available! Press [U]"));
                }
            } else if was_checking {
                state.show_toast(format!(
                    "Phaenna is up to date (v{} · {})",
                    env!("CARGO_PKG_VERSION"),
                    short_sha(env!("PHAENNA_GIT_SHA"))
                ));
            }
        }
        AppMessage::UpdateProgress(pct, msg) => {
            state.is_updating = true;
            state.update_progress = pct;
            state.update_status = Some(msg);
        }
        AppMessage::UpdateSuccess(ver) => {
            state.is_updating = false;
            state.update_progress = 100.0;
            state.update_completed_version = Some(ver.clone());
            state.update_status = Some(format!("Updated to {} — press [Enter] to exit!", ver));
            state.overlay = ActiveOverlay::Update;
            state.show_toast(format!("Phaenna updated to {}! Press [Enter] to exit.", ver));
        }
        AppMessage::UpdateFailed(err) => {
            state.is_updating = false;
            state.update_progress = 0.0;
            state.update_status = Some(format!("✕ {}", err));
            state.show_toast(format!("Update failed: {err}"));
        }
        AppMessage::CastDevicesDiscovered(devs) => {
            state.is_scanning_cast = false;
            state.cast_devices = devs;
            state.cast_cursor = 0;
        }
        AppMessage::CastStarted(name) => {
            state.show_toast(format!("Casting to {}", name));
        }
        AppMessage::CastFailed(err) => {
            state.show_toast(format!("✕ Cast error: {}", err));
        }
        AppMessage::ShowToast(msg) => {
            state.show_toast(msg);
        }
        _ => {}
    }
}

fn spawn_fetch_comment_replies(
    video_id: String,
    parent_comment_id: String,
    token: String,
    session: Option<crate::services::auth::GoogleAccountSession>,
    tx: mpsc::Sender<AppMessage>,
    client: reqwest::Client,
) {
    tokio::spawn(async move {
        match fetch_comment_replies(&client, session.as_ref(), &token).await {
            Ok(page) => {
                let _ = tx
                    .send(AppMessage::CommentRepliesLoaded {
                        video_id,
                        parent_comment_id,
                        replies: page.comments,
                        continuation: page.continuation,
                    })
                    .await;
            }
            Err(_) => {
                let _ = tx
                    .send(AppMessage::CommentRepliesLoaded {
                        video_id,
                        parent_comment_id,
                        replies: Vec::new(),
                        continuation: None,
                    })
                    .await;
            }
        }
    });
}

fn toggle_selected_comment_replies(
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let Some(video_id) = state.detail_video.as_ref().map(|v| v.id.clone()) else {
        return;
    };
    let idx = state.detail_comment_cursor;
    let sess = state.auth_vault.active_account().cloned();
    let Some(c) = state.detail_comments.get_mut(idx) else {
        return;
    };
    if c.replies_expanded {
        c.replies_expanded = false;
        return;
    }
    c.replies_expanded = true;
    if c.replies.is_empty() && !c.replies_loading {
        if let Some(tok) = c.replies_continuation.take() {
            c.replies_loading = true;
            let pid = c.id.clone();
            spawn_fetch_comment_replies(video_id, pid, tok, sess, tx.clone(), client.clone());
        }
    }
}

fn load_more_selected_comment_replies(
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let Some(video_id) = state.detail_video.as_ref().map(|v| v.id.clone()) else {
        return;
    };
    let idx = state.detail_comment_cursor;
    let sess = state.auth_vault.active_account().cloned();
    let Some(c) = state.detail_comments.get_mut(idx) else {
        return;
    };
    if !c.replies_loading {
        if let Some(tok) = c.replies_continuation.take() {
            c.replies_expanded = true;
            c.replies_loading = true;
            let pid = c.id.clone();
            spawn_fetch_comment_replies(video_id, pid, tok, sess, tx.clone(), client.clone());
        }
    }
}

fn spawn_update_check(
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
    auto_install: bool,
    check_interval: u64,
    respect_interval: bool,
) {
    let tx_u = tx.clone();
    let pool_u = pool.clone();
    let client_u = client.clone();
    tokio::spawn(async move {
        let now = now_unix();
        let should_check = if respect_interval && check_interval > 0 {
            match last_synced(&pool_u, "update_check").await {
                Ok(Some(last)) => (now - last) >= check_interval as i64,
                _ => true,
            }
        } else {
            true
        };
        if !should_check {
            return;
        }

        match check_for_updates(&client_u).await {
            Ok(update_opt) => {
                let _ = mark_synced(&pool_u, "update_check", now).await;
                if let Some(ref info) = update_opt {
                    if auto_install && !info.is_homebrew {
                        spawn_self_update(client_u.clone(), info.clone(), tx_u.clone());
                    }
                }
                let _ = tx_u.send(AppMessage::UpdateCheckResult(update_opt)).await;
            }
            Err(_) => {
                if !respect_interval {
                    let _ = tx_u.send(AppMessage::UpdateCheckResult(None)).await;
                }
            }
        }
    });
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
    state.detail_focus_pane = 0;
    state.detail_chapter_cursor = 0;
    state.detail_related_cursor = 0;
    state.detail_scroll = 0;
    state.detail_subtitles.clear();
    state.detail_sponsor_segments.clear();
    state.detail_dislikes = None;
    state.detail_comments.clear();
    state.detail_comment_cursor = 0;
    state.detail_comments_loading = true;
    state.detail_create_comment_params = None;
    state.detail_comment_composing = false;
    state.detail_comment_input.clear();
    state.detail_comment_hitboxes.clear();
    state.detail_comment_last_click = None;
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
    let id_ryd = vid_id.clone();
    tokio::spawn(async move {
        if let Ok(Some(info)) = fetch_dislikes(&c_ryd, &id_ryd).await {
            let _ = tx_ryd
                .send(AppMessage::DislikesLoaded {
                    video_id: id_ryd,
                    info,
                })
                .await;
        }
    });

    let c_cm = client.clone();
    let p_cm = pool.clone();
    let tx_cm = tx.clone();
    let sess_cm = state.auth_vault.active_account().cloned();
    tokio::spawn(async move {
        let mut combined = get_user_video_comments(&p_cm, &vid_id)
            .await
            .unwrap_or_default();
        let mut create_params = None;
        if let Ok(page) = fetch_video_comments(&c_cm, sess_cm.as_ref(), &vid_id).await {
            create_params = page.create_comment_params;
            for c in page.comments {
                if !combined.iter().any(|existing| existing.id == c.id) {
                    combined.push(c);
                }
            }
        }
        let _ = tx_cm
            .send(AppMessage::CommentsLoaded {
                video_id: vid_id,
                comments: combined,
                create_params,
            })
            .await;
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
    let start_pos = start_override.or_else(|| {
        state.progress_map.get(&video.id).and_then(|(p, dur)| {
            if *dur > 10.0 && *p >= *dur - 8.0 {
                None
            } else {
                Some(*p)
            }
        })
    });
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

    let cookies_path = state.config.cookies_path().ok();
    if let Some(ref cpath) = cookies_path {
        let _ = state.auth_vault.sync_isolated_cookie_jar(cpath);
    }

    let ytdl_format = quality.ytdl_format_with_dub(
        state.config.dub_preference,
        &state.config.dub_lang,
        state.config.performance_mode.loads_multistreams(state.config.audio_multistreams),
    );
    let opts = PlayOptions {
        video_id: video.id.clone(),
        title: video.title.clone(),
        local_file,
        quality_format: ytdl_format,
        player: state.config.player,
        cookies_path: cookies_path.clone(),
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
        performance_mode: state.config.performance_mode,
        motion_mode: state.config.motion_mode,
        dub_preference: state.config.dub_preference,
        dub_lang: state.config.dub_lang.clone(),
        audio_multistreams: state.config.audio_multistreams,
        audio_loudnorm: state.config.audio_loudnorm,
    };

    match spawn_player(&opts) {
        Ok(Some(_child)) => {
            let ts = now_unix();
            let _ = touch_watch_history(pool, Some(video), &video.id, ts).await;
            state.mark_video_watched(video);

            if let Some((_, hist_list)) = state
                .categories
                .iter()
                .find(|(c, _)| *c == Category::History)
            {
                let _ = batch_upsert_feed(pool, HISTORY, hist_list, ts).await;
            }
            let rec_len = if let Some((_, rec_list)) = state
                .categories
                .iter()
                .find(|(c, _)| *c == Category::Recommended)
            {
                let _ = batch_upsert_feed(pool, RECOMMENDED, rec_list, ts).await;
                rec_list.len()
            } else {
                usize::MAX
            };
            if rec_len < 12 && !state.is_fetching_more {
                maybe_trigger_infinite_scroll_for(
                    Category::Recommended,
                    state,
                    pool,
                    tx,
                    client,
                    false,
                );
            }

            let sess_sync = state.auth_vault.active_account().cloned();
            let vid_sync = video.id.clone();
            let dur_sync = video.duration.map(|d| d as f64);
            let start_sync = start_pos.unwrap_or(0.0);
            let c_sync = client.clone();
            let cpath_sync = cookies_path.clone();
            tokio::spawn(async move {
                let _ = report_youtube_watch_progress(
                    &c_sync,
                    sess_sync.as_ref(),
                    cpath_sync.as_deref(),
                    &vid_sync,
                    start_sync,
                    start_sync,
                    dur_sync,
                    true,
                )
                .await;
            });

            state.show_toast(format!(
                "Playing via {} ({})",
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
                sb_segs = filter_segments_for_mode_with_outro(
                    &sb_segs,
                    state.config.sponsorblock_mode,
                    state.config.skip_outro,
                );

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

async fn refresh_continue_watching_state(state: &mut AppState, pool: &SqlitePool) {
    if let Ok(cw) = get_continue_watching(pool).await {
        if let Some(idx) = state
            .categories
            .iter()
            .position(|(c, _)| *c == Category::ContinueWatching)
        {
            let new_len = cw.len();
            state.categories[idx].1 = cw;
            if state.category_cursors[idx] >= new_len {
                state.category_cursors[idx] = new_len.saturating_sub(1);
            }
        }
    }
    if let Ok(meta) = get_continue_watching_meta_map(pool).await {
        state.continue_watching_meta = meta;
    }
}

async fn pin_video_to_continue_watching(
    video: &VideoItem,
    state: &mut AppState,
    pool: &SqlitePool,
) {
    let _ = pin_to_continue_watching(pool, video, now_unix()).await;
    refresh_continue_watching_state(state, pool).await;
    let count = state
        .categories
        .iter()
        .find(|(c, _)| *c == Category::ContinueWatching)
        .map(|(_, v)| v.len())
        .unwrap_or(0);
    let short_title: String = video.title.chars().take(36).collect();
    state.show_toast(format!(
        "Added \"{short_title}\" to Continue Watching ({count})"
    ));
}

async fn remove_video_from_continue_watching(
    video: &VideoItem,
    state: &mut AppState,
    pool: &SqlitePool,
) {
    let _ = remove_continue_watching(pool, &video.id).await;
    refresh_continue_watching_state(state, pool).await;
    let short_title: String = video.title.chars().take(36).collect();
    state.show_toast(format!("Removed \"{short_title}\" from Continue Watching"));
}

fn maybe_trigger_infinite_scroll(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
    force: bool,
) {
    let cat = state.categories[state.active_category].0;
    maybe_trigger_infinite_scroll_for(cat, state, pool, tx, client, force);
}

fn set_shorts_source(
    src: ShortsSource,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    state.shorts_source = src;
    state.sync_active_shorts_list();
    let is_empty = match src {
        ShortsSource::Suggested => state.shorts_suggested.is_empty(),
        ShortsSource::Subscribed => state.shorts_subscribed.is_empty(),
    };
    state.show_toast(format!("Shorts: {}", src.label()));
    if is_empty {
        maybe_trigger_infinite_scroll_for(Category::Shorts, state, pool, tx, client, true);
    }
}

fn maybe_trigger_infinite_scroll_for(
    cat: Category,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
    force: bool,
) {
    let Some(url) = cat.url() else {
        return;
    };
    let key = state.active_feed_key_for(cat).to_string();
    if state.is_fetching_more {
        return;
    }
    if !force {
        if state.exhausted_feeds.contains(&key) {
            return;
        }
        if let Some(ts) = state.last_pagination_at {
            if ts.elapsed() < Duration::from_millis(400) {
                return;
            }
        }
    }

    let current_len = state
        .categories
        .iter()
        .find(|(c, _)| *c == cat)
        .map(|(_, l)| l.len())
        .unwrap_or(0);
    state.is_fetching_more = true;
    state.last_pagination_at = Some(Instant::now());

    // Dedicated Shorts pagination path (Suggested via InnerTube / Subscribed across channels)
    if key == SHORTS_SUGGESTED {
        let client_clone = client.clone();
        let tx_m = tx.clone();
        let sess_opt = state.auth_vault.active_account().cloned();
        let tok_opt = state.feed_continuations.get(&key).cloned();
        let ck = state.config.cookies_path().ok();
        tokio::spawn(async move {
            let mut items = Vec::new();
            let mut next_cont = tok_opt;
            for _ in 0..2 {
                if let Ok(page) =
                    fetch_suggested_shorts(&client_clone, sess_opt.as_ref(), next_cont.as_deref())
                        .await
                {
                    let mut seen: HashSet<String> = items.iter().map(|v: &VideoItem| v.id.clone()).collect();
                    for v in page.videos {
                        if seen.insert(v.id.clone()) {
                            items.push(v);
                        }
                    }
                    next_cont = page.continuation;
                    if items.len() >= 8 || next_cont.is_none() {
                        break;
                    }
                } else {
                    break;
                }
            }
            if items.is_empty() && next_cont.is_none() {
                let raw = search_youtube(ck.as_deref(), "#shorts", 18).await;
                items = filter_short_items(raw);
            }
            let _ = tx_m
                .send(AppMessage::MoreFeedLoaded {
                    feed_key: key,
                    items,
                    continuation: next_cont,
                })
                .await;
        });
        return;
    }

    if key == SHORTS_SUBSCRIBED {
        let client_clone = client.clone();
        let tx_m = tx.clone();
        let p_inf = pool.clone();
        let sess_opt = state.auth_vault.active_account().cloned();
        tokio::spawn(async move {
            let items =
                fetch_subscribed_shorts_merged(&p_inf, &client_clone, sess_opt.as_ref(), Vec::new(), 20)
                    .await;
            let _ = tx_m
                .send(AppMessage::MoreFeedLoaded {
                    feed_key: key,
                    items,
                    continuation: None,
                })
                .await;
        });
        return;
    }

    // Fast path: signed-in InnerTube shelf (works even if continuation token was None after cached boot)
    if let (Some(session), Some(browse_id)) = (
        state.auth_vault.active_account().cloned(),
        browse_id_for_feed_key(&key),
    ) {
        let tok_opt = state.feed_continuations.get(&key).cloned();
        let client_clone = client.clone();
        let tx_m = tx.clone();
        let p_inf = pool.clone();
        tokio::spawn(async move {
            match fetch_innertube_shelf(&client_clone, &session, browse_id, tok_opt.as_deref())
                .await
            {
                Ok(page) => {
                    let _ = sync_youtube_watched_items(
                        &p_inf,
                        &page.videos,
                        &page.watched_progress,
                        key == HISTORY,
                        now_unix(),
                    )
                    .await;
                    let _ = tx_m
                        .send(AppMessage::MoreFeedLoaded {
                            feed_key: key,
                            items: page.videos,
                            continuation: page.continuation,
                        })
                        .await;
                }
                Err(_) => {
                    let _ = tx_m
                        .send(AppMessage::MoreFeedLoaded {
                            feed_key: key,
                            items: Vec::new(),
                            continuation: None,
                        })
                        .await;
                }
            }
        });
        return;
    }

    // Fallback path: yt-dlp playlist slice (with approximate_date enabled)
    let start = current_len + 1;
    let end = start + 23;
    let cookies = state.config.cookies_path().ok();
    let tx_m = tx.clone();
    let u = url.to_string();
    tokio::spawn(async move {
        fetch_feed_range_stream(cookies.as_deref(), u, key, start, end, tx_m).await;
    });
}

fn scroll_down_grid_and_prefetch(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
    cols: usize,
    rows_step: usize,
    list_len: usize,
) {
    if list_len == 0 {
        maybe_trigger_infinite_scroll(state, pool, tx, client, false);
        return;
    }
    let cols = cols.max(1);
    let step = cols * rows_step.max(1);
    let c = &mut state.category_cursors[state.active_category];
    if *c + step < list_len {
        *c += step;
    } else {
        // Allow stepping down onto the incomplete final row cleanly instead of switching tabs.
        let cur_row = *c / cols;
        let last_row = (list_len - 1) / cols;
        if cur_row < last_row {
            *c = list_len - 1;
        }
    }

    let cur_row = state.category_cursors[state.active_category] / cols;
    let total_rows = list_len.div_ceil(cols);
    if cur_row + 2 >= total_rows {
        maybe_trigger_infinite_scroll(state, pool, tx, client, false);
    }
}

async fn toggle_watch_later(
    video: &VideoItem,
    state: &mut AppState,
    pool: &SqlitePool,
    client: &reqwest::Client,
) {
    let active_session = state.auth_vault.active_account().cloned();
    if state.watch_later_sources.contains_key(&video.id) {
        let _ = remove_from_watch_later(pool, &video.id).await;
        state.watch_later_sources.remove(&video.id);
        if let Some(sess) = active_session {
            let c_clone = client.clone();
            let vid = video.id.clone();
            tokio::spawn(async move {
                let _ = innertube_edit_watch_later(&c_clone, &sess, &vid, false).await;
            });
            state.show_toast("Removed from Watch Later (Local + YouTube)");
        } else {
            state.show_toast("Removed from Watch Later");
        }
    } else {
        let src = if active_session.is_some() {
            WatchLaterSource::Both
        } else {
            WatchLaterSource::Local
        };
        let _ = add_to_watch_later(pool, video, src, now_unix()).await;
        state.watch_later_sources.insert(video.id.clone(), src);
        if let Some(sess) = active_session {
            let c_clone = client.clone();
            let vid = video.id.clone();
            tokio::spawn(async move {
                let _ = innertube_edit_watch_later(&c_clone, &sess, &vid, true).await;
            });
            state.show_toast("Added to Watch Later (Synced to YouTube)");
        } else {
            state.show_toast("Added to Watch Later");
        }
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
            let next = state.config.shader_mode.next();
            apply_shader_mode(state, next);
        }
        HubActionId::PerformanceMode => {
            let next = state.config.performance_mode.next();
            apply_performance_mode(state, next);
        }
        HubActionId::InstallNeuralShaders => {
            state.show_toast("Downloading FSRCNNX 4K neural CNN shader pack...");
            let c_s = client.clone();
            let tx_s = tx.clone();
            tokio::spawn(async move {
                let msg = match download_neural_shader_pack(&c_s).await {
                    Ok(path) => format!("FSRCNNX Neural Cinema 4K installed ({})", path.display()),
                    Err(e) => format!("Neural shader download failed: {}", e),
                };
                let _ = tx_s.send(AppMessage::ShowToast(msg)).await;
            });
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
        HubActionId::GoogleAccountHub => {
            state.login_ui.step = LoginStep::Hub;
            state.login_ui.error_banner = None;
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
            if !state.is_checking_update {
                state.is_checking_update = true;
                spawn_update_check(pool, tx, client, false, 0, false);
            }
        }
        HubActionId::AutoUpdateMode => {
            if !state.config.update.check_on_startup {
                state.config.update.check_on_startup = true;
                state.config.update.auto_install = false;
                state.show_toast("Auto-Update: Check on Startup");
            } else if !state.config.update.auto_install {
                state.config.update.auto_install = true;
                state.show_toast("Auto-Update: Silent Auto-Install");
            } else {
                state.config.update.check_on_startup = false;
                state.config.update.auto_install = false;
                state.show_toast("Auto-Update: Disabled");
            }
            let _ = state.config.save();
        }
    }
}

async fn toggle_dub_live_or_config(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    state.config.dub_preference = state.config.dub_preference.toggle();
    let want_dub = state.config.dub_preference.is_dub_on();
    let _ = state.config.save();

    if state.has_live_ipc() {
        if let Some(aid) = state.find_audio_track_for_dub(want_dub) {
            state.send_player_cmd(PlayerCommand::SetAudioTrack(aid));
            state.show_toast(if want_dub {
                format!("Switched live to Dubbed Audio (track #{aid})")
            } else {
                format!("Switched live to Original Audio (track #{aid})")
            });
        } else {
            // Single audio stream loaded; seamlessly reload at current timestamp with updated dub filter
            relaunch_live_video_from_pos(state, pool, tx, client).await;
            state.show_toast(format!("Audio Dub: {} (reloading at timestamp)", state.config.dub_preference.label()));
        }
    } else {
        state.show_toast(format!("Audio Dub: {}", state.config.dub_preference.label()));
    }
}

async fn relaunch_live_video_from_pos(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let resume_at = if state.active_playback_pos > 2.0 {
        Some(state.active_playback_pos)
    } else {
        None
    };
    let target_vid = if let Some(ref vid_id) = state.active_playback_video_id {
        state
            .all_known_videos()
            .into_iter()
            .find(|v| v.id == *vid_id)
            .or_else(|| state.active_selected_video())
    } else {
        state.active_selected_video()
    };
    if state.has_live_ipc() {
        state.send_player_cmd(PlayerCommand::Stop);
    }
    if let Some(v) = target_vid {
        start_nonblocking_playback(&v, resume_at, state, pool, tx, client).await;
    }
}

fn apply_shader_mode(state: &mut AppState, mode: VideoShaderMode) {
    state.config.shader_mode = mode;
    let _ = state.config.save();
    if state.has_live_ipc() {
        let prof = state
            .active_selected_video()
            .map(|v| VideoMasteringProfile::from_metadata(v.duration, &v.title))
            .unwrap_or(VideoMasteringProfile::StandardHd);
        state.send_player_cmd(PlayerCommand::SetShaderMode(
            mode,
            prof,
            state.config.performance_mode,
        ));
        state.show_toast(format!("Live 4K Shader: {}", mode.short_label()));
    } else {
        state.show_toast(format!("4K Shader: {}", mode.short_label()));
    }
}

fn apply_performance_mode(state: &mut AppState, mode: PerformanceMode) {
    state.config.performance_mode = mode;
    let _ = state.config.save();
    if state.has_live_ipc() {
        let prof = state
            .active_selected_video()
            .map(|v| VideoMasteringProfile::from_metadata(v.duration, &v.title))
            .unwrap_or(VideoMasteringProfile::StandardHd);
        state.send_player_cmd(PlayerCommand::SetPerformanceMode(
            mode,
            state.config.shader_mode,
            prof,
        ));
        state.show_toast(format!("Live Mode: {} ({})", mode.label(), mode.summary()));
    } else {
        state.show_toast(format!("Performance Mode: {} ({})", mode.label(), mode.summary()));
    }
}

fn apply_motion_mode(state: &mut AppState, mode: MotionMode) {
    state.config.motion_mode = mode;
    let _ = state.config.save();
    if state.has_live_ipc() {
        state.send_player_cmd(PlayerCommand::SetMotionMode(mode));
        state.show_toast(format!("Live Motion: {}", mode.label()));
    } else {
        state.show_toast(format!("Motion Mode: {}", mode.label()));
    }
}

async fn apply_play_options_hit(
    hit: PlayOptionsHit,
    click_col: u16,
    hit_rect: ratatui::layout::Rect,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match hit {
        PlayOptionsHit::FocusSection(sec) => state.play_modal_section = sec.min(4),
        PlayOptionsHit::SelectQuality(i) => {
            if let Some(q) = Quality::ALL.get(i).copied() {
                state.play_quality_idx = i;
                state.config.quality = q;
                let _ = state.config.save();
            }
        }
        PlayOptionsHit::SelectPlayer(pl) => {
            state.config.player = pl;
            let _ = state.config.save();
        }
        PlayOptionsHit::ToggleDub => {
            toggle_dub_live_or_config(state, pool, tx, client).await;
        }
        PlayOptionsHit::CycleDubLang => {
            state.config.dub_lang = next_dub_lang(&state.config.dub_lang);
            let _ = state.config.save();
            if state.has_live_ipc() && state.config.dub_preference.is_dub_on() {
                if let Some(aid) = state.find_audio_track_for_dub(true) {
                    state.send_player_cmd(PlayerCommand::SetAudioTrack(aid));
                }
            }
        }
        PlayOptionsHit::ToggleMultistreams => {
            state.config.audio_multistreams = !state.config.audio_multistreams;
            let _ = state.config.save();
        }
        PlayOptionsHit::ToggleLoudnorm => {
            state.config.audio_loudnorm = !state.config.audio_loudnorm;
            let _ = state.config.save();
            if state.has_live_ipc() {
                state.send_player_cmd(PlayerCommand::SetLoudnorm(state.config.audio_loudnorm));
            }
        }
        PlayOptionsHit::AudioDelayDelta(delta_ms) => {
            state.live_playback.audio_delay_ms =
                (state.live_playback.audio_delay_ms + delta_ms).clamp(-5000, 5000);
            state.send_player_cmd(PlayerCommand::SetAudioDelay(
                state.live_playback.audio_delay_ms as f64 / 1000.0,
            ));
        }
        PlayOptionsHit::SelectAudioTrack(aid) => {
            state.live_playback.active_aid = Some(aid);
            state.send_player_cmd(PlayerCommand::SetAudioTrack(aid));
            if let Some(tr) = state.live_playback.audio_tracks.iter().find(|t| t.id == aid) {
                state.config.dub_preference = if tr.is_dubbed {
                    crate::config::DubPreference::PreferDubbed
                } else {
                    crate::config::DubPreference::OriginalOnly
                };
                let _ = state.config.save();
            }
        }
        PlayOptionsHit::ToggleSubtitles => {
            if state.has_live_ipc() {
                state.live_playback.sub_visible = !state.live_playback.sub_visible;
                state.config.subtitles_enabled = state.live_playback.sub_visible;
                state.send_player_cmd(PlayerCommand::SetSubVisible(state.live_playback.sub_visible));
            } else {
                state.config.subtitles_enabled = !state.config.subtitles_enabled;
            }
            let _ = state.config.save();
        }
        PlayOptionsHit::CycleSubStyle => {
            state.config.subtitle_style.cycle_preset();
            let _ = state.config.save();
        }
        PlayOptionsHit::SubDelayDelta(delta_ms) => {
            state.live_playback.sub_delay_ms =
                (state.live_playback.sub_delay_ms + delta_ms).clamp(-10000, 10000);
            state.send_player_cmd(PlayerCommand::SetSubDelay(
                state.live_playback.sub_delay_ms as f64 / 1000.0,
            ));
        }
        PlayOptionsHit::SelectSubTrack(sid) => {
            state.live_playback.active_sid = Some(sid);
            state.live_playback.sub_visible = true;
            state.send_player_cmd(PlayerCommand::SetSubTrack(sid));
        }
        PlayOptionsHit::SelectSponsorBlock(mode) => {
            state.config.sponsorblock_mode = mode;
            let _ = state.config.save();
        }
        PlayOptionsHit::ToggleSkipOutro => {
            state.config.skip_outro = !state.config.skip_outro;
            let _ = state.config.save();
        }
        PlayOptionsHit::SelectShader(mode) => {
            apply_shader_mode(state, mode);
        }
        PlayOptionsHit::SetPerformanceMode(mode) => {
            apply_performance_mode(state, mode);
        }
        PlayOptionsHit::CycleMotion => {
            let next = state.config.motion_mode.next();
            apply_motion_mode(state, next);
        }
        PlayOptionsHit::ScrollAudioTracks(delta) => {
            let total = state.live_playback.audio_tracks.len();
            if total > 0 {
                let next = if delta > 0 {
                    let cur = state.live_playback.audio_track_scroll + (delta as usize);
                    if cur + 1 >= total { 0 } else { cur }
                } else {
                    state.live_playback.audio_track_scroll.saturating_sub((-delta) as usize)
                };
                state.live_playback.audio_track_scroll = next;
            }
        }
        PlayOptionsHit::TogglePause => {
            state.live_playback.paused = !state.live_playback.paused;
            state.send_player_cmd(PlayerCommand::TogglePause);
        }
        PlayOptionsHit::SeekDelta(delta) => {
            state.send_player_cmd(PlayerCommand::SeekRelative(delta));
        }
        PlayOptionsHit::SeekRatio(_) => {
            if state.active_playback_dur > 0.0 && hit_rect.width > 12 {
                let bar_span = hit_rect.width.saturating_sub(14).max(1) as f64;
                let rel = (click_col.saturating_sub(hit_rect.x + 1) as f64 / bar_span).clamp(0.0, 0.99);
                let target = state.active_playback_dur * rel;
                state.active_playback_pos = target;
                state.send_player_cmd(PlayerCommand::SeekAbsolute(target));
            }
        }
        PlayOptionsHit::ChapterStep(step) => {
            state.send_player_cmd(PlayerCommand::ChapterStep(step));
        }
        PlayOptionsHit::SetSpeed(spd) => {
            state.live_playback.speed = spd;
            state.send_player_cmd(PlayerCommand::SetSpeed(spd));
        }
        PlayOptionsHit::VolumeDelta(delta) => {
            state.live_playback.volume = (state.live_playback.volume + delta).clamp(0, 150);
            state.send_player_cmd(PlayerCommand::SetVolume(state.live_playback.volume));
        }
        PlayOptionsHit::ToggleMute => {
            state.live_playback.muted = !state.live_playback.muted;
            state.send_player_cmd(PlayerCommand::ToggleMute);
        }
        PlayOptionsHit::PictureDelta(prop, delta) => {
            let next_val = match prop {
                "brightness" => {
                    state.live_playback.brightness =
                        (state.live_playback.brightness + delta).clamp(-50, 50);
                    state.live_playback.brightness
                }
                "contrast" => {
                    state.live_playback.contrast =
                        (state.live_playback.contrast + delta).clamp(-50, 50);
                    state.live_playback.contrast
                }
                "saturation" => {
                    state.live_playback.saturation =
                        (state.live_playback.saturation + delta).clamp(-50, 50);
                    state.live_playback.saturation
                }
                _ => {
                    state.live_playback.gamma =
                        (state.live_playback.gamma + delta).clamp(-50, 50);
                    state.live_playback.gamma
                }
            };
            state.send_player_cmd(PlayerCommand::SetPictureProp { prop, value: next_val });
        }
        PlayOptionsHit::ResetPicture => {
            state.live_playback.brightness = 0;
            state.live_playback.contrast = 0;
            state.live_playback.saturation = 0;
            state.live_playback.gamma = 0;
            state.send_player_cmd(PlayerCommand::ResetPicture);
        }
        PlayOptionsHit::ToggleOnTop => {
            state.live_playback.ontop = !state.live_playback.ontop;
            state.send_player_cmd(PlayerCommand::SetOnTop(state.live_playback.ontop));
        }
        PlayOptionsHit::ToggleLoop => {
            state.live_playback.loop_video = !state.live_playback.loop_video;
            state.send_player_cmd(PlayerCommand::SetLoop(state.live_playback.loop_video));
        }
        PlayOptionsHit::LaunchOrRelaunch => {
            state.overlay = ActiveOverlay::None;
            if state.has_live_ipc() {
                relaunch_live_video_from_pos(state, pool, tx, client).await;
            } else if let Some(v) = state.active_selected_video() {
                start_nonblocking_playback(&v, None, state, pool, tx, client).await;
            }
        }
        PlayOptionsHit::StopPlayer => {
            state.send_player_cmd(PlayerCommand::Stop);
            state.overlay = ActiveOverlay::None;
            state.show_toast("Stopped playback");
        }
    }
}

async fn apply_now_playing_hit(
    hit: NowPlayingHit,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match hit {
        NowPlayingHit::TogglePause => {
            state.live_playback.paused = !state.live_playback.paused;
            state.send_player_cmd(PlayerCommand::TogglePause);
        }
        NowPlayingHit::SeekBack => {
            state.send_player_cmd(PlayerCommand::SeekRelative(-10.0));
        }
        NowPlayingHit::SeekFwd => {
            state.send_player_cmd(PlayerCommand::SeekRelative(10.0));
        }
        NowPlayingHit::ToggleDub => {
            toggle_dub_live_or_config(state, pool, tx, client).await;
        }
        NowPlayingHit::SpeedStep(dir) => {
            let next = (state.live_playback.speed + (dir as f64) * 0.25).clamp(0.5, 2.5);
            state.live_playback.speed = next;
            state.send_player_cmd(PlayerCommand::SetSpeed(next));
        }
        NowPlayingHit::VolumeDelta(delta) => {
            state.live_playback.volume = (state.live_playback.volume + delta).clamp(0, 150);
            state.send_player_cmd(PlayerCommand::SetVolume(state.live_playback.volume));
        }
        NowPlayingHit::ToggleMute => {
            state.live_playback.muted = !state.live_playback.muted;
            state.send_player_cmd(PlayerCommand::ToggleMute);
        }
        NowPlayingHit::CycleShader => {
            let next = state.config.shader_mode.next();
            apply_shader_mode(state, next);
        }
        NowPlayingHit::CyclePerfMode => {
            let next = state.config.performance_mode.next();
            apply_performance_mode(state, next);
        }
        NowPlayingHit::CycleMotion => {
            let next = state.config.motion_mode.next();
            apply_motion_mode(state, next);
        }
        NowPlayingHit::OpenStudio => {
            state.overlay = ActiveOverlay::PlayOptions;
        }
        NowPlayingHit::StopPlayer => {
            state.send_player_cmd(PlayerCommand::Stop);
            state.show_toast("Stopped playback");
        }
    }
}

async fn open_search_overlay(state: &mut AppState, pool: &SqlitePool) {
    state.overlay = ActiveOverlay::Search;
    state.search_query.clear();
    state.search_results.clear();
    state.search_suggestions.clear();
    state.search_continuation = None;
    state.search_local_results = get_all_cached_videos(pool, 16).await.unwrap_or_default();
    state.search_cursor = 0;
}

async fn trigger_search_update(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
    remote_now: bool,
) {
    state.search_generation += 1;
    state.search_continuation = None;
    state.search_local_results = search_cached_videos(pool, &state.search_query)
        .await
        .unwrap_or_default();
    state.search_cursor = 0;

    let q = state.search_query.trim().to_string();
    if q.is_empty() {
        state.search_results.clear();
        state.search_suggestions.clear();
        state.search_pending_remote = false;
        state.is_searching_remote = false;
        return;
    }

    if remote_now {
        state.search_pending_remote = false;
        spawn_search_suggestions(client, q.clone(), state.search_generation, tx);
        if matches!(state.search_scope, SearchScope::All | SearchScope::LiveYt) {
            state.push_search_history(&q);
            state.is_searching_remote = true;
            spawn_remote_search(
                client,
                state.auth_vault.active_account().cloned(),
                &state.config,
                q,
                state.search_params_token(),
                state.search_generation,
                tx,
            );
        } else {
            state.is_searching_remote = false;
        }
    } else {
        state.search_last_typed = Some(Instant::now());
        state.search_pending_remote = true;
    }
}

async fn apply_search_hit(
    hit: SearchHit,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match hit {
        SearchHit::ApplySuggestion(s) => {
            state.search_query = s;
            trigger_search_update(state, pool, tx, client, true).await;
        }
        SearchHit::SetScope(sc) => {
            if state.search_scope != sc {
                state.search_scope = sc;
                state.search_cursor = 0;
                if matches!(sc, SearchScope::All | SearchScope::LiveYt)
                    && !state.search_query.trim().is_empty()
                    && state.search_results.is_empty()
                {
                    trigger_search_update(state, pool, tx, client, true).await;
                }
            }
        }
        SearchHit::SetType(tf) => {
            if state.search_type_filter != tf {
                let server_changed =
                    matches!(state.search_type_filter, SearchTypeFilter::Videos)
                        || matches!(tf, SearchTypeFilter::Videos);
                state.search_type_filter = tf;
                state.search_cursor = 0;
                if server_changed && !state.search_query.trim().is_empty() {
                    trigger_search_update(state, pool, tx, client, true).await;
                }
            }
        }
        SearchHit::SetDate(df) => {
            if state.search_date_filter != df {
                state.search_date_filter = df;
                state.search_cursor = 0;
                if !state.search_query.trim().is_empty() {
                    trigger_search_update(state, pool, tx, client, true).await;
                }
            }
        }
        SearchHit::SetDuration(df) => {
            if state.search_duration_filter != df {
                state.search_duration_filter = df;
                state.search_cursor = 0;
                if !state.search_query.trim().is_empty() {
                    trigger_search_update(state, pool, tx, client, true).await;
                }
            }
        }
        SearchHit::SetSort(so) => {
            if state.search_sort_order != so {
                let server_changed =
                    matches!(so, SearchSortOrder::Newest | SearchSortOrder::Views)
                        || matches!(
                            state.search_sort_order,
                            SearchSortOrder::Newest | SearchSortOrder::Views
                        );
                state.search_sort_order = so;
                state.search_cursor = 0;
                if server_changed && !state.search_query.trim().is_empty() {
                    trigger_search_update(state, pool, tx, client, true).await;
                }
            }
        }
        SearchHit::ToggleHideWatched => {
            state.search_hide_watched = !state.search_hide_watched;
            state.search_cursor = 0;
        }
        SearchHit::SelectRow(idx) => {
            let total = state.combined_search_items().len();
            if idx < total {
                let now = Instant::now();
                let is_double = state
                    .search_last_click
                    .map(|(prev_idx, ts)| {
                        prev_idx == idx && now.duration_since(ts) <= Duration::from_millis(450)
                    })
                    .unwrap_or(false);
                state.search_cursor = idx;
                state.search_last_click = Some((idx, now));
                if is_double {
                    if let Some(v) = state.combined_search_items().get(idx).cloned() {
                        open_detail_for_video(v, state, pool, tx, client);
                    }
                }
            }
        }
        SearchHit::ActionPlay => {
            if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                state.overlay = ActiveOverlay::None;
                start_nonblocking_playback(&v, None, state, pool, tx, client).await;
            }
        }
        SearchHit::ActionDetail => {
            if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                open_detail_for_video(v, state, pool, tx, client);
            }
        }
        SearchHit::ActionPinCw => {
            if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                pin_video_to_continue_watching(&v, state, pool).await;
            }
        }
        SearchHit::ActionWatchLater => {
            if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                toggle_watch_later(&v, state, pool, client).await;
            }
        }
        SearchHit::ActionChannel => {
            if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                state.overlay = ActiveOverlay::None;
                open_channel_for_video(&v, state, tx, client);
            }
        }
        SearchHit::LoadMore => {
            if let Some(cont) = state.search_continuation.clone() {
                if !state.is_searching_remote {
                    state.is_searching_remote = true;
                    spawn_more_search_results(
                        client,
                        state.auth_vault.active_account().cloned(),
                        state.search_query.trim().to_string(),
                        state.search_params_token(),
                        cont,
                        state.search_generation,
                        tx,
                    );
                }
            }
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
        if state.overlay == ActiveOverlay::ConfirmQuit {
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                let (exit_btn, cancel_btn) = confirm_quit_button_rects(frame_area);
                let in_rect = |r: ratatui::layout::Rect| -> bool {
                    r.width > 0
                        && r.height > 0
                        && mouse.column >= r.x
                        && mouse.column < r.x + r.width
                        && mouse.row >= r.y
                        && mouse.row < r.y + r.height
                };
                if in_rect(exit_btn) {
                    state.should_quit = true;
                } else if in_rect(cancel_btn)
                    || crate::state::active_modal_rect(frame_area, ActiveOverlay::ConfirmQuit)
                        .map(|m| !in_rect(m))
                        .unwrap_or(false)
                {
                    state.overlay = ActiveOverlay::None;
                }
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
                state.overlay = ActiveOverlay::None;
                return true;
            }
            return false;
        }
        if state.overlay == ActiveOverlay::PlayOptions {
            let in_rect = |r: ratatui::layout::Rect| -> bool {
                r.width > 0
                    && r.height > 0
                    && mouse.column >= r.x
                    && mouse.column < r.x + r.width
                    && mouse.row >= r.y
                    && mouse.row < r.y + r.height
            };
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                if let Some((rect, hit)) = state
                    .live_playback
                    .play_hitboxes
                    .iter()
                    .find(|(r, h)| in_rect(*r) && !matches!(h, PlayOptionsHit::FocusSection(_)))
                    .or_else(|| {
                        state
                            .live_playback
                            .play_hitboxes
                            .iter()
                            .find(|(r, _)| in_rect(*r))
                    })
                    .cloned()
                {
                    apply_play_options_hit(hit, mouse.column, rect, state, pool, tx, client).await;
                    return true;
                }
            }
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
                if let Some((rect, PlayOptionsHit::PictureDelta(prop, _))) = state
                    .live_playback
                    .play_hitboxes
                    .iter()
                    .find(|(r, h)| {
                        in_rect(*r) && matches!(h, PlayOptionsHit::PictureDelta(_, _))
                    })
                    .cloned()
                {
                    apply_play_options_hit(
                        PlayOptionsHit::PictureDelta(prop, -5),
                        mouse.column,
                        rect,
                        state,
                        pool,
                        tx,
                        client,
                    )
                    .await;
                    return true;
                }
                state.overlay = ActiveOverlay::None;
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
                let over_audio_panel = state.live_playback.play_hitboxes.iter().any(|(r, h)| {
                    in_rect(*r) && matches!(h, PlayOptionsHit::FocusSection(1))
                });
                if over_audio_panel {
                    let delta = if matches!(mouse.kind, MouseEventKind::ScrollDown) {
                        1
                    } else {
                        -1
                    };
                    apply_play_options_hit(
                        PlayOptionsHit::ScrollAudioTracks(delta),
                        mouse.column,
                        ratatui::layout::Rect::default(),
                        state,
                        pool,
                        tx,
                        client,
                    )
                    .await;
                    return true;
                }
            }
            return false;
        }
        if state.overlay == ActiveOverlay::Search {
            let in_rect = |r: ratatui::layout::Rect| -> bool {
                r.width > 0
                    && r.height > 0
                    && mouse.column >= r.x
                    && mouse.column < r.x + r.width
                    && mouse.row >= r.y
                    && mouse.row < r.y + r.height
            };
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                if let Some((_, hit)) = state
                    .search_hitboxes
                    .iter()
                    .rev()
                    .find(|(r, _)| in_rect(*r))
                    .cloned()
                {
                    apply_search_hit(hit, state, pool, tx, client).await;
                    return true;
                }
                if let Some(modal) =
                    crate::state::active_modal_rect(frame_area, ActiveOverlay::Search)
                {
                    if !in_rect(modal) {
                        state.overlay = ActiveOverlay::None;
                        return true;
                    }
                }
            }
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Right)) {
                state.overlay = ActiveOverlay::None;
                return true;
            }
            if matches!(mouse.kind, MouseEventKind::ScrollDown | MouseEventKind::ScrollUp) {
                let total = state.combined_search_items().len();
                if total > 0 {
                    if matches!(mouse.kind, MouseEventKind::ScrollDown) {
                        state.search_cursor = (state.search_cursor + 1).min(total - 1);
                        if state.search_cursor + 3 >= total
                            && state.search_continuation.is_some()
                            && !state.is_searching_remote
                        {
                            apply_search_hit(SearchHit::LoadMore, state, pool, tx, client).await;
                        }
                    } else {
                        state.search_cursor = state.search_cursor.saturating_sub(1);
                    }
                    return true;
                }
            }
            return false;
        }
        if state.overlay == ActiveOverlay::None && state.screen == Screen::Detail {
            let in_rect = |r: ratatui::layout::Rect| -> bool {
                r.width > 0
                    && r.height > 0
                    && mouse.column >= r.x
                    && mouse.column < r.x + r.width
                    && mouse.row >= r.y
                    && mouse.row < r.y + r.height
            };
            if matches!(mouse.kind, MouseEventKind::Down(MouseButton::Left)) {
                if let Some((_, hit)) = state
                    .live_playback
                    .now_playing_hitboxes
                    .iter()
                    .find(|(r, _)| in_rect(*r))
                    .copied()
                {
                    apply_now_playing_hit(hit, state, pool, tx, client).await;
                    return true;
                }
            }

            match mouse.kind {
                MouseEventKind::Down(MouseButton::Left) => {
                    if in_rect(state.detail_comment_composer_rect) {
                        state.detail_focus_pane = 2;
                        state.detail_comment_composing = true;
                        state.detail_comment_last_click = None;
                        return true;
                    }
                    if in_rect(state.detail_comments_panel_rect) {
                        state.detail_focus_pane = 2;
                        state.detail_comment_composing = false;
                        if let Some(&(_, _, idx)) = state
                            .detail_comment_hitboxes
                            .iter()
                            .find(|&&(y0, y1, _)| mouse.row >= y0 && mouse.row <= y1)
                        {
                            let is_dbl = matches!(
                                state.detail_comment_last_click,
                                Some((prev, ts))
                                    if prev == idx && ts.elapsed() <= Duration::from_millis(450)
                            );
                            state.detail_comment_cursor = idx;
                            if is_dbl {
                                state.detail_comment_last_click = None;
                                toggle_selected_comment_replies(state, tx, client);
                            } else {
                                state.detail_comment_last_click = Some((idx, Instant::now()));
                            }
                        }
                        return true;
                    }
                }
                MouseEventKind::ScrollDown if in_rect(state.detail_comments_panel_rect) => {
                    let n = state.detail_comments.len();
                    if n > 0 {
                        state.detail_comment_cursor = (state.detail_comment_cursor + 1).min(n - 1);
                    }
                    return true;
                }
                MouseEventKind::ScrollUp if in_rect(state.detail_comments_panel_rect) => {
                    state.detail_comment_cursor = state.detail_comment_cursor.saturating_sub(1);
                    return true;
                }
                _ => {}
            }
        }

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
    let active_cat = state.categories[state.active_category].0;
    let cw_deck = active_cat == Category::ContinueWatching && !state.continue_watching_grid_mode;

    match mouse.kind {
        MouseEventKind::Down(MouseButton::Left) => {
            if let Some((_, hit)) = state
                .live_playback
                .now_playing_hitboxes
                .iter()
                .find(|(r, _)| {
                    r.width > 0
                        && r.height > 0
                        && mouse.column >= r.x
                        && mouse.column < r.x + r.width
                        && mouse.row >= r.y
                        && mouse.row < r.y + r.height
                })
                .copied()
            {
                apply_now_playing_hit(hit, state, pool, tx, client).await;
                return true;
            }
            if mouse.row == top_area.y {
                if let Some(idx) = top_bar_shelf_at(&state.categories, mouse.column) {
                    state.active_category = idx;
                    return true;
                }
            }
            if mouse.row >= grid_area.y && mouse.row < grid_area.y + grid_area.height {
                if active_cat == Category::Shorts {
                    if let Some(src) = shorts_source_at(
                        grid_area,
                        state.shorts_suggested.len(),
                        state.shorts_subscribed.len(),
                        mouse.column,
                        mouse.row,
                    ) {
                        set_shorts_source(src, state, pool, tx, client);
                        return true;
                    }
                }
                let card_grid = effective_grid_area(grid_area, active_cat);
                let total = state.current_filtered_videos().len();
                let scroll = state.grid_scroll_rows[state.active_category];
                let hit = if cw_deck {
                    continue_watching_row_at(grid_area, mouse.column, mouse.row, scroll, total)
                } else {
                    grid_card_index_at(card_grid, mouse.column, mouse.row, scroll, total)
                };
                if let Some(idx) = hit {
                    if state.category_cursors[state.active_category] == idx {
                        if let Some(v) = state.current_home_video() {
                            if cw_deck {
                                start_nonblocking_playback(&v, None, state, pool, tx, client).await;
                            } else {
                                open_detail_for_video(v, state, pool, tx, client);
                            }
                        }
                    } else {
                        state.category_cursors[state.active_category] = idx;
                    }
                    return true;
                }
            }
            if let Some(act) = footer_action_at(footer_area, mouse.column, mouse.row) {
                match act {
                    HomeFooterAction::Search => open_search_overlay(state, pool).await,
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
                    HomeFooterAction::Quit => open_confirm_quit(state),
                }
                return true;
            }
        }
        MouseEventKind::ScrollDown => {
            let len = state.current_filtered_videos().len();
            if cw_deck {
                if len > 0 {
                    let cur = &mut state.category_cursors[state.active_category];
                    *cur = (*cur + 1).min(len - 1);
                    return true;
                }
            } else {
                let cols = grid_columns(grid_area.width);
                scroll_down_grid_and_prefetch(state, pool, tx, client, cols, 1, len);
                return true;
            }
        }
        MouseEventKind::ScrollUp => {
            if cw_deck {
                let cur = &mut state.category_cursors[state.active_category];
                *cur = cur.saturating_sub(1);
            } else {
                let cols = grid_columns(grid_area.width);
                let cur = &mut state.category_cursors[state.active_category];
                *cur = cur.saturating_sub(cols);
            }
            return true;
        }
        _ => {}
    }
    false
}

fn open_confirm_quit(state: &mut AppState) {
    state.quit_confirm_yes = false;
    state.overlay = ActiveOverlay::ConfirmQuit;
}

/// Returns true only when application should quit.
async fn handle_key_event(
    code: KeyCode,
    modifiers: KeyModifiers,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) -> bool {
    // 1. Handle active modal overlay first
    if state.overlay != ActiveOverlay::None {
        handle_overlay_key(code, modifiers, state, pool, tx, client).await;
        return false;
    }

    // 2. Detail / Channel screens
    match state.screen {
        Screen::Detail => {
            if state.detail_comment_composing {
                match code {
                    KeyCode::Esc => {
                        state.detail_comment_composing = false;
                    }
                    KeyCode::Backspace => {
                        state.detail_comment_input.pop();
                    }
                    KeyCode::Enter => {
                        let body = state.detail_comment_input.trim().to_string();
                        if !body.is_empty() {
                            if let Some(vid_id) = state.detail_video.as_ref().map(|v| v.id.clone()) {
                                let author = state
                                    .auth_vault
                                    .active_account()
                                    .map(|a| a.display_name.clone())
                                    .unwrap_or_else(|| "You (Local)".to_string());
                                let now = now_unix();
                                let _ = save_user_video_comment(pool, &vid_id, &author, &body, now).await;
                                let local_cmt = VideoComment {
                                    id: format!("local_{now}"),
                                    author,
                                    published_text: "Just now".to_string(),
                                    text: body.clone(),
                                    like_count_text: "1".to_string(),
                                    reply_count: 0,
                                    is_pinned: false,
                                    is_self: true,
                                    ..Default::default()
                                };
                                state.detail_comments.insert(0, local_cmt);
                                state.detail_comment_cursor = 0;

                                if let Some(sess) = state.auth_vault.active_account().cloned() {
                                    let c_post = client.clone();
                                    let tx_post = tx.clone();
                                    let params = state.detail_create_comment_params.clone();
                                    let v_id_p = vid_id.clone();
                                    state.show_toast("Posting comment to YouTube...");
                                    tokio::spawn(async move {
                                        let msg = match post_video_comment(
                                            &c_post,
                                            &sess,
                                            &v_id_p,
                                            params.as_deref(),
                                            &body,
                                        )
                                        .await
                                        {
                                            Ok(()) => "Comment posted to YouTube!".to_string(),
                                            Err(e) => {
                                                format!("Saved locally (YouTube sync failed: {e})")
                                            }
                                        };
                                        let _ = tx_post.send(AppMessage::ShowToast(msg)).await;
                                    });
                                } else {
                                    state.show_toast(
                                        "Comment saved locally (Sign in [L] to post to YouTube)",
                                    );
                                }
                            }
                        }
                        state.detail_comment_input.clear();
                        state.detail_comment_composing = false;
                    }
                    KeyCode::Char(c) => {
                        state.detail_comment_input.push(c);
                    }
                    _ => {}
                }
                return false;
            }

            match code {
                KeyCode::Esc | KeyCode::Backspace => {
                    if let Some(prev) = state.detail_history.pop() {
                        open_detail_for_video(prev, state, pool, tx, client);
                    } else {
                        state.screen = Screen::Home;
                    }
                }
                KeyCode::Tab => {
                    state.detail_focus_pane = (state.detail_focus_pane + 1) % 3;
                    state.detail_view_mode = if state.detail_focus_pane == 1 {
                        DetailViewMode::Chapters
                    } else {
                        DetailViewMode::Overview
                    };
                }
                KeyCode::BackTab => {
                    state.detail_focus_pane = (state.detail_focus_pane + 2) % 3;
                    state.detail_view_mode = if state.detail_focus_pane == 1 {
                        DetailViewMode::Chapters
                    } else {
                        DetailViewMode::Overview
                    };
                }
                KeyCode::Char('m') => {
                    state.detail_focus_pane = 2;
                    state.detail_comment_composing = true;
                }
                KeyCode::Char('j') | KeyCode::Down => {
                    match state.detail_focus_pane {
                        1 => {
                            let n = state
                                .detail_video
                                .as_ref()
                                .map(|v| v.effective_chapters().len())
                                .unwrap_or(0);
                            if n > 0 {
                                state.detail_chapter_cursor =
                                    (state.detail_chapter_cursor + 1).min(n - 1);
                            }
                        }
                        2 => {
                            let n = state.detail_comments.len();
                            if n > 0 {
                                state.detail_comment_cursor =
                                    (state.detail_comment_cursor + 1).min(n - 1);
                            }
                        }
                        _ => {
                            state.detail_scroll = state.detail_scroll.saturating_add(2);
                        }
                    }
                }
                KeyCode::Char('k') | KeyCode::Up => {
                    match state.detail_focus_pane {
                        1 => {
                            state.detail_chapter_cursor =
                                state.detail_chapter_cursor.saturating_sub(1);
                        }
                        2 => {
                            state.detail_comment_cursor =
                                state.detail_comment_cursor.saturating_sub(1);
                        }
                        _ => {
                            state.detail_scroll = state.detail_scroll.saturating_sub(2);
                        }
                    }
                }
                KeyCode::Char('r') | KeyCode::Char('l') | KeyCode::Right
                    if state.detail_focus_pane == 2 =>
                {
                    toggle_selected_comment_replies(state, tx, client);
                }
                KeyCode::Char('h') | KeyCode::Left if state.detail_focus_pane == 2 => {
                    if let Some(c) = state.detail_comments.get_mut(state.detail_comment_cursor) {
                        c.replies_expanded = false;
                    }
                }
                KeyCode::Char('n') if state.detail_focus_pane == 2 => {
                    load_more_selected_comment_replies(state, tx, client);
                }
                KeyCode::Enter | KeyCode::Char(' ') => {
                    if state.detail_focus_pane == 2 && code == KeyCode::Enter {
                        let has_replies = state
                            .detail_comments
                            .get(state.detail_comment_cursor)
                            .map(|c| {
                                c.reply_count > 0
                                    || !c.replies.is_empty()
                                    || c.replies_continuation.is_some()
                            })
                            .unwrap_or(false);
                        if has_replies {
                            toggle_selected_comment_replies(state, tx, client);
                        } else {
                            state.detail_comment_composing = true;
                        }
                    } else if let Some(v) = state.detail_video.clone() {
                        let start_ch = if state.detail_focus_pane == 1 {
                            v.effective_chapters()
                                .get(state.detail_chapter_cursor)
                                .map(|c| c.start_time)
                        } else {
                            None
                        };
                        start_nonblocking_playback(&v, start_ch, state, pool, tx, client).await;
                    }
                }
                KeyCode::Char('c') => {
                    if let Some(v) = state.detail_video.clone() {
                        open_channel_for_video(&v, state, tx, client);
                    }
                }
                KeyCode::Char('p') => state.overlay = ActiveOverlay::PlayOptions,
                KeyCode::Char('a') | KeyCode::Char('+') => {
                    if let Some(v) = state.detail_video.clone() {
                        pin_video_to_continue_watching(&v, state, pool).await;
                    }
                }
                KeyCode::Char('w') => {
                    if let Some(v) = state.detail_video.clone() {
                        toggle_watch_later(&v, state, pool, client).await;
                    }
                }
                KeyCode::Char('D') => {
                    if let Some(v) = state.detail_video.clone() {
                        spawn_offline_download(&v, state, pool, tx);
                    }
                }
                KeyCode::Char('M') => state.overlay = ActiveOverlay::Remote,
                KeyCode::Char('C') => execute_hub_action(HubActionId::CastScan, state, pool, tx, client).await,
                KeyCode::Char('s') | KeyCode::Char(',') => state.overlay = ActiveOverlay::Settings,
                KeyCode::Char('?') => state.overlay = ActiveOverlay::Help,
                KeyCode::Char('q') => open_confirm_quit(state),
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
                KeyCode::Char('a') | KeyCode::Char('+') => {
                    if let Some(v) = state.channel_videos.get(state.channel_cursor).cloned() {
                        pin_video_to_continue_watching(&v, state, pool).await;
                    }
                }
                KeyCode::Char('q') => open_confirm_quit(state),
                _ => {}
            }
            return false;
        }
        Screen::Home => {}
    }

    // 3. Home Global Keybindings
    let list_len = state.current_filtered_videos().len();
    let active_cat = state.categories[state.active_category].0;
    let cw_deck = active_cat == Category::ContinueWatching && !state.continue_watching_grid_mode;
    let (cols, visible_rows) = crossterm::terminal::size()
        .map(|(w, h)| {
            let [_, _, grid_area, _] = home_layout(ratatui::layout::Rect::new(0, 0, w, h));
            let card_grid = effective_grid_area(grid_area, active_cat);
            let m = compute_grid_metrics(card_grid);
            (m.cols, m.visible_rows)
        })
        .unwrap_or((3, 2));

    match code {
        KeyCode::Char('q') => open_confirm_quit(state),
        KeyCode::Char('?') => state.overlay = ActiveOverlay::Help,
        KeyCode::Char('/') => {
            open_search_overlay(state, pool).await;
        }
        KeyCode::Char('s') | KeyCode::Char(',') => {
            state.settings_query.clear();
            state.settings_cursor = 0;
            state.overlay = ActiveOverlay::Settings;
        }
        KeyCode::Char('L') => {
            state.login_ui.step = LoginStep::Hub;
            state.login_ui.error_banner = None;
            state.overlay = ActiveOverlay::Login;
        }
        KeyCode::Char('!') => {
            state.deps = detect_dependencies();
            state.overlay = ActiveOverlay::Setup;
        }
        KeyCode::Char('f') => state.overlay = ActiveOverlay::Filter,
        KeyCode::Char('p') => state.overlay = ActiveOverlay::PlayOptions,
        KeyCode::Char('A') => open_analytics(state, pool).await,
        KeyCode::Char('a') | KeyCode::Char('+') => {
            if let Some(v) = state.current_home_video() {
                pin_video_to_continue_watching(&v, state, pool).await;
            }
        }
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
            state.show_toast(format!("Theme: {}", state.base_theme().name));
        }
        KeyCode::Char('r') | KeyCode::Char('R') => {
            trigger_swr_feed_refreshes(state, pool, tx, client, true);
            state.show_toast("Refreshing all YouTube shelves...");
        }
        KeyCode::Char('n') | KeyCode::Char(']') => {
            maybe_trigger_infinite_scroll(state, pool, tx, client, true);
        }
        KeyCode::Char('S') => {
            if let Some(shorts_idx) = state.categories.iter().position(|(c, _)| *c == Category::Shorts)
            {
                if state.active_category != shorts_idx {
                    state.active_category = shorts_idx;
                } else {
                    let next = state.shorts_source.toggle();
                    set_shorts_source(next, state, pool, tx, client);
                }
            }
        }
        KeyCode::Char('w') => {
            if let Some(v) = state.current_home_video() {
                toggle_watch_later(&v, state, pool, client).await;
            }
        }
        KeyCode::Char('D') => {
            if let Some(v) = state.current_home_video() {
                spawn_offline_download(&v, state, pool, tx);
            }
        }
        KeyCode::Char('v') if active_cat == Category::ContinueWatching => {
            state.continue_watching_grid_mode = !state.continue_watching_grid_mode;
            state.show_toast(if state.continue_watching_grid_mode {
                "Continue Watching: Grid View"
            } else {
                "Continue Watching: Queue Deck View"
            });
        }
        KeyCode::Char('x') | KeyCode::Char('d') | KeyCode::Delete
            if active_cat == Category::ContinueWatching =>
        {
            if let Some(v) = state.current_home_video() {
                remove_video_from_continue_watching(&v, state, pool).await;
            }
        }
        KeyCode::Backspace if cw_deck => {
            if let Some(v) = state.current_home_video() {
                remove_video_from_continue_watching(&v, state, pool).await;
            }
        }
        KeyCode::Char('X') if active_cat == Category::ContinueWatching => {
            if let Ok(cleared) = clear_finished_continue_watching(pool).await {
                refresh_continue_watching_state(state, pool).await;
                state.show_toast(format!("Cleared {cleared} finished videos from Continue Watching"));
            }
        }
        KeyCode::Char('K') if active_cat == Category::ContinueWatching => {
            if let Some(v) = state.current_home_video() {
                if move_continue_watching_item(pool, &v.id, -1).await.unwrap_or(false) {
                    refresh_continue_watching_state(state, pool).await;
                    let c = &mut state.category_cursors[state.active_category];
                    *c = c.saturating_sub(1);
                }
            }
        }
        KeyCode::Char('J') if active_cat == Category::ContinueWatching => {
            if let Some(v) = state.current_home_video() {
                if move_continue_watching_item(pool, &v.id, 1).await.unwrap_or(false) {
                    refresh_continue_watching_state(state, pool).await;
                    let len = state.current_filtered_videos().len();
                    if len > 0 {
                        let c = &mut state.category_cursors[state.active_category];
                        *c = (*c + 1).min(len - 1);
                    }
                }
            }
        }
        KeyCode::Char('i') => {
            if let Some(v) = state.current_home_video() {
                open_detail_for_video(v, state, pool, tx, client);
            }
        }
        KeyCode::Enter => {
            if let Some(v) = state.current_home_video() {
                if active_cat == Category::ContinueWatching {
                    start_nonblocking_playback(&v, None, state, pool, tx, client).await;
                } else {
                    open_detail_for_video(v, state, pool, tx, client);
                }
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
        KeyCode::Char('6') => state.active_category = 5.min(state.categories.len() - 1),
        KeyCode::Tab => {
            state.active_category = (state.active_category + 1) % state.categories.len();
        }
        KeyCode::BackTab => {
            state.active_category =
                (state.active_category + state.categories.len() - 1) % state.categories.len();
        }
        KeyCode::Char('l') | KeyCode::Right => {
            if list_len > 0 && !cw_deck {
                let c = &mut state.category_cursors[state.active_category];
                *c = (*c + 1).min(list_len - 1);
            }
        }
        KeyCode::Char('h') | KeyCode::Left => {
            if !cw_deck {
                let c = &mut state.category_cursors[state.active_category];
                *c = c.saturating_sub(1);
            }
        }
        KeyCode::Char('j') | KeyCode::Down => {
            if cw_deck {
                if list_len > 0 {
                    let c = &mut state.category_cursors[state.active_category];
                    *c = (*c + 1).min(list_len - 1);
                }
            } else {
                scroll_down_grid_and_prefetch(state, pool, tx, client, cols, 1, list_len);
            }
        }
        KeyCode::Char('k') | KeyCode::Up => {
            if cw_deck {
                let c = &mut state.category_cursors[state.active_category];
                *c = c.saturating_sub(1);
            } else {
                let c = &mut state.category_cursors[state.active_category];
                *c = c.saturating_sub(cols);
            }
        }
        KeyCode::PageDown => {
            if cw_deck {
                if list_len > 0 {
                    let c = &mut state.category_cursors[state.active_category];
                    *c = (*c + 5).min(list_len - 1);
                }
            } else {
                scroll_down_grid_and_prefetch(state, pool, tx, client, cols, visible_rows, list_len);
            }
        }
        KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
            if cw_deck {
                if list_len > 0 {
                    let c = &mut state.category_cursors[state.active_category];
                    *c = (*c + 5).min(list_len - 1);
                }
            } else {
                scroll_down_grid_and_prefetch(state, pool, tx, client, cols, visible_rows, list_len);
            }
        }
        KeyCode::PageUp => {
            let step = if cw_deck { 5 } else { cols * visible_rows.max(1) };
            let c = &mut state.category_cursors[state.active_category];
            *c = c.saturating_sub(step);
        }
        KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
            let step = if cw_deck { 5 } else { cols * visible_rows.max(1) };
            let c = &mut state.category_cursors[state.active_category];
            *c = c.saturating_sub(step);
        }
        KeyCode::Home | KeyCode::Char('g') => {
            state.category_cursors[state.active_category] = 0;
        }
        KeyCode::End | KeyCode::Char('G') => {
            if list_len > 0 {
                state.category_cursors[state.active_category] = list_len - 1;
            }
            if !cw_deck {
                maybe_trigger_infinite_scroll(state, pool, tx, client, false);
            }
        }
        _ => {}
    }

    false
}

async fn handle_overlay_key(
    code: KeyCode,
    modifiers: KeyModifiers,
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
                state.show_toast("Re-scanned runtime binaries");
            }
            KeyCode::Char('L') => {
            state.login_ui.step = LoginStep::Hub;
            state.login_ui.error_banner = None;
            state.overlay = ActiveOverlay::Login;
        }
            _ => {}
        },
        ActiveOverlay::Login => {
            handle_login_overlay_key(code, modifiers, state, pool, tx, client).await;
        }
        ActiveOverlay::Filter => match code {
            KeyCode::Esc | KeyCode::Char('f') | KeyCode::Enter => {
                state.overlay = ActiveOverlay::None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.filter_modal_section = (state.filter_modal_section + 2) % 3;
                state.filter_modal_option = 0;
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.filter_modal_section = (state.filter_modal_section + 1) % 3;
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
                } else if state.filter_modal_section == 1 {
                    let next = (state.filter_modal_option as isize + dir).rem_euclid(5) as usize;
                    state.filter_modal_option = next;
                    state.sort_order = match next {
                        1 => SortOrder::Newest,
                        2 => SortOrder::DurationLongest,
                        3 => SortOrder::DurationShortest,
                        4 => SortOrder::ChannelAZ,
                        _ => SortOrder::Default,
                    };
                } else {
                    let next = (state.filter_modal_option as isize + dir).rem_euclid(2) as usize;
                    state.filter_modal_option = next;
                    let src = if next == 1 {
                        ShortsSource::Subscribed
                    } else {
                        ShortsSource::Suggested
                    };
                    set_shorts_source(src, state, pool, tx, client);
                }
            }
            _ => {}
        },
        ActiveOverlay::PlayOptions => match code {
            KeyCode::Esc | KeyCode::Char('p') => state.overlay = ActiveOverlay::None,
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => {
                state.play_modal_section = if state.play_modal_section == 0 {
                    4
                } else {
                    state.play_modal_section - 1
                };
            }
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => {
                state.play_modal_section = (state.play_modal_section + 1) % 5;
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
                    1 => {
                        toggle_dub_live_or_config(state, pool, tx, client).await;
                        return;
                    }
                    2 => state.config.sponsorblock_mode = state.config.sponsorblock_mode.next(),
                    3 => {
                        let next = if down {
                            state.config.shader_mode.next()
                        } else {
                            let mut cur = state.config.shader_mode;
                            for _ in 0..4 {
                                cur = cur.next();
                            }
                            cur
                        };
                        apply_shader_mode(state, next);
                        return;
                    }
                    _ => {
                        let delta = if down { -5 } else { 5 };
                        state.live_playback.volume =
                            (state.live_playback.volume + delta).clamp(0, 150);
                        state.send_player_cmd(PlayerCommand::SetVolume(state.live_playback.volume));
                        return;
                    }
                }
                let _ = state.config.save();
            }
            KeyCode::Char('d') => toggle_dub_live_or_config(state, pool, tx, client).await,
            KeyCode::Char('L') => {
                state.config.dub_lang = next_dub_lang(&state.config.dub_lang);
                let _ = state.config.save();
                if state.has_live_ipc() && state.config.dub_preference.is_dub_on() {
                    if let Some(aid) = state.find_audio_track_for_dub(true) {
                        state.send_player_cmd(PlayerCommand::SetAudioTrack(aid));
                    }
                }
            }
            KeyCode::Char('n') => {
                state.config.audio_loudnorm = !state.config.audio_loudnorm;
                let _ = state.config.save();
                state.send_player_cmd(PlayerCommand::SetLoudnorm(state.config.audio_loudnorm));
            }
            KeyCode::Char('c') => {
                if state.has_live_ipc() {
                    state.live_playback.sub_visible = !state.live_playback.sub_visible;
                    state.config.subtitles_enabled = state.live_playback.sub_visible;
                    state.send_player_cmd(PlayerCommand::SetSubVisible(
                        state.live_playback.sub_visible,
                    ));
                } else {
                    state.config.subtitles_enabled = !state.config.subtitles_enabled;
                }
                let _ = state.config.save();
            }
            KeyCode::Char('y') => {
                state.config.subtitle_style.cycle_preset();
                let _ = state.config.save();
            }
            KeyCode::Char('o') => {
                state.config.skip_outro = !state.config.skip_outro;
                let _ = state.config.save();
            }
            KeyCode::Char('s') => {
                let next = state.config.shader_mode.next();
                apply_shader_mode(state, next);
            }
            KeyCode::Char('m') => {
                let next = state.config.performance_mode.next();
                apply_performance_mode(state, next);
            }
            KeyCode::Char('t') => {
                let next = state.config.motion_mode.next();
                apply_motion_mode(state, next);
            }
            KeyCode::Char(' ') => {
                if state.has_live_ipc() {
                    state.live_playback.paused = !state.live_playback.paused;
                    state.send_player_cmd(PlayerCommand::TogglePause);
                } else {
                    toggle_dub_live_or_config(state, pool, tx, client).await;
                }
            }
            KeyCode::Char('-') => {
                state.live_playback.volume = (state.live_playback.volume - 5).clamp(0, 150);
                state.send_player_cmd(PlayerCommand::SetVolume(state.live_playback.volume));
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                state.live_playback.volume = (state.live_playback.volume + 5).clamp(0, 150);
                state.send_player_cmd(PlayerCommand::SetVolume(state.live_playback.volume));
            }
            KeyCode::Char('[') => {
                let next = (state.live_playback.speed - 0.25).clamp(0.5, 2.5);
                state.live_playback.speed = next;
                state.send_player_cmd(PlayerCommand::SetSpeed(next));
            }
            KeyCode::Char(']') => {
                let next = (state.live_playback.speed + 0.25).clamp(0.5, 2.5);
                state.live_playback.speed = next;
                state.send_player_cmd(PlayerCommand::SetSpeed(next));
            }
            KeyCode::Char('M') => {
                state.live_playback.muted = !state.live_playback.muted;
                state.send_player_cmd(PlayerCommand::ToggleMute);
            }
            KeyCode::Char('0') => {
                state.live_playback.brightness = 0;
                state.live_playback.contrast = 0;
                state.live_playback.saturation = 0;
                state.live_playback.gamma = 0;
                state.send_player_cmd(PlayerCommand::ResetPicture);
            }
            KeyCode::Char('X') => {
                state.send_player_cmd(PlayerCommand::Stop);
                state.overlay = ActiveOverlay::None;
            }
            KeyCode::Enter => {
                state.overlay = ActiveOverlay::None;
                if state.has_live_ipc() {
                    relaunch_live_video_from_pos(state, pool, tx, client).await;
                } else if let Some(v) = state.active_selected_video() {
                    start_nonblocking_playback(&v, None, state, pool, tx, client).await;
                }
            }
            _ => {}
        },
        ActiveOverlay::Search => match code {
            KeyCode::Esc => state.overlay = ActiveOverlay::None,
            KeyCode::Tab => {
                let candidate = state
                    .search_suggestions
                    .first()
                    .or_else(|| state.search_history.first())
                    .cloned();
                if let Some(s) = candidate {
                    apply_search_hit(SearchHit::ApplySuggestion(s), state, pool, tx, client).await;
                }
            }
            KeyCode::Up => state.search_cursor = state.search_cursor.saturating_sub(1),
            KeyCode::Down => {
                let total = state.combined_search_items().len();
                if total > 0 {
                    state.search_cursor = (state.search_cursor + 1).min(total - 1);
                    if state.search_cursor + 3 >= total
                        && state.search_continuation.is_some()
                        && !state.is_searching_remote
                    {
                        apply_search_hit(SearchHit::LoadMore, state, pool, tx, client).await;
                    }
                }
            }
            KeyCode::PageUp => {
                state.search_cursor = state.search_cursor.saturating_sub(5);
            }
            KeyCode::PageDown => {
                let total = state.combined_search_items().len();
                if total > 0 {
                    state.search_cursor = (state.search_cursor + 5).min(total - 1);
                    if state.search_cursor + 3 >= total
                        && state.search_continuation.is_some()
                        && !state.is_searching_remote
                    {
                        apply_search_hit(SearchHit::LoadMore, state, pool, tx, client).await;
                    }
                }
            }
            KeyCode::Enter => {
                let q = state.search_query.trim().to_string();
                if let Some(vid) = extract_youtube_video_id(&q) {
                    let stub = VideoItem {
                        id: vid.clone(),
                        title: format!("YouTube Video ({vid})"),
                        webpage_url: Some(format!("https://www.youtube.com/watch?v={vid}")),
                        ..VideoItem::default()
                    };
                    open_detail_for_video(stub, state, pool, tx, client);
                } else if let Some(v) = state.combined_search_items().get(state.search_cursor).cloned() {
                    if !q.is_empty() {
                        state.push_search_history(&q);
                    }
                    open_detail_for_video(v, state, pool, tx, client);
                } else if !q.is_empty() {
                    trigger_search_update(state, pool, tx, client, true).await;
                }
            }
            KeyCode::Char('t') if modifiers.contains(KeyModifiers::CONTROL) => {
                let next = state.search_type_filter.next();
                apply_search_hit(SearchHit::SetType(next), state, pool, tx, client).await;
            }
            KeyCode::Char('d') if modifiers.contains(KeyModifiers::CONTROL) => {
                let next = state.search_date_filter.next();
                apply_search_hit(SearchHit::SetDate(next), state, pool, tx, client).await;
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                let next = state.search_duration_filter.next();
                apply_search_hit(SearchHit::SetDuration(next), state, pool, tx, client).await;
            }
            KeyCode::Char('s') if modifiers.contains(KeyModifiers::CONTROL) => {
                let next = state.search_sort_order.next();
                apply_search_hit(SearchHit::SetSort(next), state, pool, tx, client).await;
            }
            KeyCode::Char('w') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_search_hit(SearchHit::ToggleHideWatched, state, pool, tx, client).await;
            }
            KeyCode::Char('g') if modifiers.contains(KeyModifiers::CONTROL) => {
                let next = state.search_scope.next();
                apply_search_hit(SearchHit::SetScope(next), state, pool, tx, client).await;
            }
            KeyCode::Char('n') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_search_hit(SearchHit::LoadMore, state, pool, tx, client).await;
            }
            KeyCode::Char('a') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_search_hit(SearchHit::ActionPinCw, state, pool, tx, client).await;
            }
            KeyCode::Char('l') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_search_hit(SearchHit::ActionWatchLater, state, pool, tx, client).await;
            }
            KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_search_hit(SearchHit::ActionChannel, state, pool, tx, client).await;
            }
            KeyCode::Char('p') if modifiers.contains(KeyModifiers::CONTROL) => {
                apply_search_hit(SearchHit::ActionPlay, state, pool, tx, client).await;
            }
            KeyCode::Char('p') if state.search_query.is_empty() => {
                apply_search_hit(SearchHit::ActionPlay, state, pool, tx, client).await;
            }
            KeyCode::Backspace => {
                state.search_query.pop();
                trigger_search_update(state, pool, tx, client, false).await;
            }
            KeyCode::Char(c) => {
                state.search_query.push(c);
                trigger_search_update(state, pool, tx, client, false).await;
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
            KeyCode::Esc | KeyCode::Char('a') | KeyCode::Char('A') => state.overlay = ActiveOverlay::None,
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
            KeyCode::Char('w') | KeyCode::Enter | KeyCode::Char(' ') | KeyCode::Char('p') | KeyCode::Char('a') | KeyCode::Char('+') => {
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
                        match code {
                            KeyCode::Char('w') => toggle_watch_later(&vid, state, pool, client).await,
                            KeyCode::Char('a') | KeyCode::Char('+') => pin_video_to_continue_watching(&vid, state, pool).await,
                            KeyCode::Char(' ') | KeyCode::Char('p') => {
                                state.overlay = ActiveOverlay::None;
                                start_nonblocking_playback(&vid, None, state, pool, tx, client).await;
                            }
                            _ => open_detail_for_video(vid, state, pool, tx, client),
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
                    state.show_toast(format!("Resolving stream for {}...", name));
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
                if state.update_completed_version.is_some() {
                    state.should_quit = true;
                } else if !state.is_updating {
                    if let Some(info) = state.available_update.clone() {
                        state.is_updating = true;
                        spawn_self_update(client.clone(), info, tx.clone());
                    }
                }
            }
            _ => {}
        },
        ActiveOverlay::ConfirmQuit => match code {
            KeyCode::Char('y') | KeyCode::Char('Y') => state.should_quit = true,
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc | KeyCode::Char('q') => {
                state.overlay = ActiveOverlay::None;
            }
            KeyCode::Left
            | KeyCode::Right
            | KeyCode::Up
            | KeyCode::Down
            | KeyCode::Tab
            | KeyCode::BackTab
            | KeyCode::Char('h')
            | KeyCode::Char('l') => {
                state.quit_confirm_yes = !state.quit_confirm_yes;
            }
            KeyCode::Enter | KeyCode::Char(' ') => {
                if state.quit_confirm_yes {
                    state.should_quit = true;
                } else {
                    state.overlay = ActiveOverlay::None;
                }
            }
            _ => {}
        },
    }
}

async fn persist_auth_vault_and_cookies(state: &AppState, pool: &SqlitePool) {
    if let Ok(vpath) = Config::auth_vault_path() {
        let _ = state.auth_vault.save_to_path(&vpath);
    }
    if let Ok(cpath) = state.config.cookies_path() {
        let _ = state.auth_vault.sync_isolated_cookie_jar(&cpath);
    }
    if let Some(active) = state.auth_vault.active_account() {
        let _ = upsert_account(pool, &active.to_account_row(true)).await;
    }
}

fn open_client_id_setup(state: &mut AppState, after: Option<PendingOAuthFlow>) {
    state.login_ui.reset_transient();
    state.login_ui.error_banner = None;
    state.login_ui.client_id_input = state
        .config
        .google_oauth
        .client_id
        .clone()
        .unwrap_or_default();
    state.login_ui.client_secret_input = state
        .config
        .google_oauth
        .client_secret
        .clone()
        .unwrap_or_default();
    state.login_ui.setup_field_idx = 0;
    state.login_ui.setup_after = after;
    state.login_ui.status_banner = Some(
        "Enter your Google Cloud OAuth Client ID (TVs / Desktop App) to enable QR & Kitty SSO."
            .to_string(),
    );
    state.login_ui.step = LoginStep::ClientIdSetup;
}

fn start_device_code_flow(
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let Some(oauth) = resolve_oauth_client(&state.config.google_oauth) else {
        open_client_id_setup(state, Some(PendingOAuthFlow::DeviceCode));
        return;
    };

    state.login_ui.reset_transient();
    state.login_ui.error_banner = None;
    state.login_ui.busy = true;
    state.login_ui.step = LoginStep::AwaitingDeviceQr;
    state.login_ui.status_banner = Some("Requesting Google Device Code & terminal QR...".into());

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    state.login_ui.cancel_tx = Some(cancel_tx);

    let c_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        match request_device_code(&c_clone, &oauth).await {
            Ok(dev) => {
                let dev_for_poll = dev.clone();
                let _ = tx_clone.send(AppMessage::AuthDeviceCodeReady(dev)).await;
                match poll_device_code_until_done(c_clone, oauth, dev_for_poll, cancel_rx).await {
                    Ok(session) => {
                        let _ = tx_clone.send(AppMessage::AuthSignedIn(session)).await;
                    }
                    Err(e) if e != "Cancelled by user" => {
                        let _ = tx_clone.send(AppMessage::AuthFailed(e)).await;
                    }
                    Err(_) => {}
                }
            }
            Err(e) => {
                let _ = tx_clone.send(AppMessage::AuthFailed(e.to_string())).await;
            }
        }
    });
}

fn start_kitty_split_signin(
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    state.login_ui.reset_transient();
    state.login_ui.error_banner = None;
    state.login_ui.busy = true;
    state.login_ui.step = LoginStep::AwaitingKittySso;
    state.login_ui.kitty_auth_url = Some(GOOGLE_YOUTUBE_SERVICE_LOGIN_URL.to_string());
    state.login_ui.status_banner =
        Some("Launching official Google Sign-In in Kitty right split...".into());

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    state.login_ui.cancel_tx = Some(cancel_tx);

    let c_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let tx_status = tx_clone.clone();
        let res = run_kitty_split_google_signin(c_clone, cancel_rx, move |_status| {
            let _ = tx_status.try_send(AppMessage::AuthKittyUrlReady {
                url: GOOGLE_YOUTUBE_SERVICE_LOGIN_URL.to_string(),
                launcher: "Kitty right split (terminal-browser)",
            });
        })
        .await;
        match res {
            Ok(session) => {
                let _ = tx_clone.send(AppMessage::AuthSignedIn(session)).await;
            }
            Err(e) if e != "OAuth sign-in cancelled" => {
                let _ = tx_clone.send(AppMessage::AuthFailed(e)).await;
            }
            Err(_) => {}
        }
    });
}

fn start_kitty_pkce_flow(
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let Some(oauth) = resolve_oauth_client(&state.config.google_oauth) else {
        open_client_id_setup(state, Some(PendingOAuthFlow::KittyPkce));
        return;
    };

    state.login_ui.reset_transient();
    state.login_ui.error_banner = None;
    state.login_ui.busy = true;
    state.login_ui.step = LoginStep::AwaitingKittySso;
    state.login_ui.status_banner =
        Some("Starting loopback callback listener on 127.0.0.1:41933...".into());

    let (cancel_tx, cancel_rx) = tokio::sync::watch::channel(false);
    state.login_ui.cancel_tx = Some(cancel_tx);

    let c_clone = client.clone();
    let tx_clone = tx.clone();
    let prefer_kitty = state.config.google_oauth.prefer_kitty_split;
    tokio::spawn(async move {
        let tx_url = tx_clone.clone();
        let res = run_loopback_pkce_flow(
            c_clone,
            oauth,
            prefer_kitty,
            cancel_rx,
            move |url, launcher| {
                let _ = tx_url.try_send(AppMessage::AuthKittyUrlReady { url, launcher });
            },
        )
        .await;
        match res {
            Ok(session) => {
                let _ = tx_clone.send(AppMessage::AuthSignedIn(session)).await;
            }
            Err(e) if e != "OAuth sign-in cancelled" => {
                let _ = tx_clone.send(AppMessage::AuthFailed(e)).await;
            }
            Err(_) => {}
        }
    });
}

fn launch_direct_login(
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let email = state.login_ui.email.trim().to_string();
    let pw = state.login_ui.password.clone();
    state.login_ui.password.clear();
    state.login_ui.busy = true;
    state.login_ui.error_banner = None;
    state.login_ui.status_banner = Some("Authenticating with Google inside Phaenna...".into());

    let c_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let msg = match submit_direct_credentials(&c_clone, &email, &pw).await {
            Ok(DirectAuthOutcome::SignedIn(sess)) => AppMessage::AuthSignedIn(*sess),
            Ok(DirectAuthOutcome::NeedsTwoFactor {
                kind,
                cookies,
                challenge_token,
            }) => AppMessage::AuthChallengeRequired {
                kind,
                cookies,
                challenge_token,
            },
            Ok(DirectAuthOutcome::RequiresModernSso { reason }) => {
                AppMessage::AuthNeedsModernSso { reason }
            }
            Err(e) => AppMessage::AuthFailed(e.to_string()),
        };
        let _ = tx_clone.send(msg).await;
    });
}

fn launch_two_factor_submit(
    state: &mut AppState,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let email = state.login_ui.email.trim().to_string();
    let code = state.login_ui.two_factor_code.clone();
    let cookies = state.login_ui.pending_flow_cookies.clone();
    let token = state.login_ui.pending_challenge_token.clone();

    state.login_ui.busy = true;
    state.login_ui.error_banner = None;
    state.login_ui.status_banner = Some("Verifying 2-Step Verification code...".into());

    let c_clone = client.clone();
    let tx_clone = tx.clone();
    tokio::spawn(async move {
        let msg = match submit_two_factor_challenge(&c_clone, &email, &code, cookies, token).await {
            Ok(DirectAuthOutcome::SignedIn(sess)) => AppMessage::AuthSignedIn(*sess),
            Ok(DirectAuthOutcome::NeedsTwoFactor {
                kind,
                cookies,
                challenge_token,
            }) => AppMessage::AuthChallengeRequired {
                kind,
                cookies,
                challenge_token,
            },
            Ok(DirectAuthOutcome::RequiresModernSso { reason }) => {
                AppMessage::AuthNeedsModernSso { reason }
            }
            Err(e) => AppMessage::AuthFailed(e.to_string()),
        };
        let _ = tx_clone.send(msg).await;
    });
}

fn is_plain_text_char(modifiers: KeyModifiers) -> bool {
    !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
}

async fn handle_login_overlay_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match state.login_ui.step.clone() {
        LoginStep::Hub => match code {
            KeyCode::Esc | KeyCode::Char('L') => {
                state.login_ui.reset_transient();
                state.overlay = ActiveOverlay::None;
            }
            KeyCode::Char('1') | KeyCode::Char('+') => {
                start_kitty_split_signin(state, tx, client);
            }
            KeyCode::Enter if !state.auth_vault.is_authenticated() => {
                start_kitty_split_signin(state, tx, client);
            }
            KeyCode::Char('2') => {
                state.login_ui.reset_transient();
                state.login_ui.error_banner = None;
                state.login_ui.status_banner =
                    Some("Step 1/3: Enter your Google or Workspace email address".into());
                state.login_ui.step = LoginStep::EmailInput;
            }
            KeyCode::Char('3') => start_device_code_flow(state, tx, client),
            KeyCode::Char('o') => open_client_id_setup(state, None),
            KeyCode::Char('r') if state.auth_vault.is_authenticated() => {
                trigger_swr_feed_refreshes(state, pool, tx, client, true);
                state.show_toast("Refreshing personalized YouTube shelves...");
            }
            KeyCode::Up | KeyCode::Char('k') if !state.auth_vault.accounts.is_empty() => {
                state.login_ui.account_cursor = state.login_ui.account_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') if !state.auth_vault.accounts.is_empty() => {
                let max = state.auth_vault.accounts.len().saturating_sub(1);
                state.login_ui.account_cursor = (state.login_ui.account_cursor + 1).min(max);
            }
            KeyCode::Enter if state.auth_vault.is_authenticated() => {
                let idx = state
                    .login_ui
                    .account_cursor
                    .min(state.auth_vault.accounts.len().saturating_sub(1));
                if let Some(acct) = state.auth_vault.accounts.get(idx).cloned() {
                    if state.auth_vault.set_active(&acct.account_id) {
                        let _ = set_active_account(pool, &acct.account_id).await;
                        persist_auth_vault_and_cookies(state, pool).await;
                        state.login_ui.status_banner =
                            Some(format!("Switched active account to {}", acct.email));
                        trigger_swr_feed_refreshes(state, pool, tx, client, true);
                    }
                }
            }
            KeyCode::Char('x') | KeyCode::Char('d') if state.auth_vault.is_authenticated() => {
                let idx = state
                    .login_ui
                    .account_cursor
                    .min(state.auth_vault.accounts.len().saturating_sub(1));
                if let Some(acct) = state.auth_vault.accounts.get(idx).cloned() {
                    state.auth_vault.remove_account(&acct.account_id);
                    let _ = delete_account(pool, &acct.account_id).await;
                    state.login_ui.account_cursor = state
                        .login_ui
                        .account_cursor
                        .min(state.auth_vault.accounts.len().saturating_sub(1));
                    persist_auth_vault_and_cookies(state, pool).await;
                    state.login_ui.status_banner =
                        Some(format!("Signed out and purged credentials for {}", acct.email));
                    state.show_toast("Signed out & wiped Phaenna vault entry");
                }
            }
            _ => {}
        },
        LoginStep::EmailInput => match code {
            KeyCode::Esc => state.login_ui.back_to_hub(),
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                state.login_ui.email.clear();
            }
            KeyCode::Backspace => {
                state.login_ui.email.pop();
            }
            KeyCode::Enter => {
                let trimmed = state.login_ui.email.trim();
                if trimmed.is_empty() || !trimmed.contains('@') {
                    state.login_ui.error_banner =
                        Some("Enter a valid Google email address (e.g. name@gmail.com).".into());
                } else {
                    state.login_ui.error_banner = None;
                    state.login_ui.status_banner =
                        Some("Step 2/3: Password held strictly in Zeroize RAM".into());
                    state.login_ui.step = LoginStep::PasswordInput;
                }
            }
            KeyCode::Char(c) if is_plain_text_char(modifiers) => {
                state.login_ui.email.push(c);
            }
            _ => {}
        },
        LoginStep::PasswordInput => match code {
            KeyCode::Esc => {
                state.login_ui.password.clear();
                state.login_ui.error_banner = None;
                state.login_ui.step = LoginStep::EmailInput;
            }
            KeyCode::Tab => {
                state.login_ui.show_password = !state.login_ui.show_password;
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                state.login_ui.password.clear();
            }
            KeyCode::Backspace => {
                state.login_ui.password.pop();
            }
            KeyCode::Enter => {
                if state.login_ui.password.is_empty() {
                    state.login_ui.error_banner = Some("Password cannot be empty.".into());
                } else {
                    launch_direct_login(state, tx, client);
                }
            }
            KeyCode::Char(c) if is_plain_text_char(modifiers) => {
                state.login_ui.password.push(c);
            }
            _ => {}
        },
        LoginStep::TwoFactorChallenge(ref kind) => match code {
            KeyCode::Esc => state.login_ui.back_to_hub(),
            KeyCode::Char('b') => {
                state.login_ui.backup_mode = !state.login_ui.backup_mode;
                state.login_ui.two_factor_code.clear();
            }
            KeyCode::Char('3') => start_device_code_flow(state, tx, client),
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                state.login_ui.two_factor_code.clear();
            }
            KeyCode::Backspace => {
                state.login_ui.two_factor_code.pop();
            }
            KeyCode::Enter => launch_two_factor_submit(state, tx, client),
            KeyCode::Char(c) if is_plain_text_char(modifiers) && c.is_ascii_alphanumeric() => {
                let target_len = if state.login_ui.backup_mode
                    || matches!(kind, ChallengeKind::BackupCode)
                {
                    8
                } else {
                    6
                };
                if state.login_ui.two_factor_code.len() < target_len {
                    state.login_ui.two_factor_code.push(c);
                    if !state.login_ui.backup_mode
                        && !matches!(kind, ChallengeKind::BackupCode)
                        && state.login_ui.two_factor_code.len() == 6
                    {
                        launch_two_factor_submit(state, tx, client);
                    }
                }
            }
            _ => {}
        },
        LoginStep::AwaitingDeviceQr => match code {
            KeyCode::Esc => state.login_ui.back_to_hub(),
            KeyCode::Char('L') => {
                state.login_ui.back_to_hub();
                state.overlay = ActiveOverlay::None;
            }
            KeyCode::Char('3') => start_device_code_flow(state, tx, client),
            _ => {}
        },
        LoginStep::AwaitingKittySso => match code {
            KeyCode::Esc => state.login_ui.back_to_hub(),
            KeyCode::Char('L') => {
                state.login_ui.back_to_hub();
                state.overlay = ActiveOverlay::None;
            }
            _ => {}
        },
        LoginStep::ClientIdSetup => match code {
            KeyCode::Esc => state.login_ui.back_to_hub(),
            KeyCode::Tab | KeyCode::BackTab | KeyCode::Up | KeyCode::Down => {
                state.login_ui.setup_field_idx = 1 - state.login_ui.setup_field_idx;
            }
            KeyCode::Char('u') if modifiers.contains(KeyModifiers::CONTROL) => {
                if state.login_ui.setup_field_idx == 0 {
                    state.login_ui.client_id_input.clear();
                } else {
                    state.login_ui.client_secret_input.clear();
                }
            }
            KeyCode::Backspace => {
                if state.login_ui.setup_field_idx == 0 {
                    state.login_ui.client_id_input.pop();
                } else {
                    state.login_ui.client_secret_input.pop();
                }
            }
            KeyCode::Enter => {
                let cid = state.login_ui.client_id_input.trim().to_string();
                let csec = state.login_ui.client_secret_input.trim().to_string();
                if cid.is_empty() {
                    state.login_ui.error_banner =
                        Some("OAuth Client ID cannot be empty.".into());
                } else {
                    state.config.google_oauth.client_id = Some(cid);
                    state.config.google_oauth.client_secret =
                        if csec.is_empty() { None } else { Some(csec) };
                    let _ = state.config.save();
                    state.login_ui.error_banner = None;
                    state.login_ui.status_banner =
                        Some("Saved OAuth Client ID to ~/.config/phaenna/config.toml".into());
                    match state.login_ui.setup_after.take() {
                        Some(PendingOAuthFlow::DeviceCode) => {
                            start_device_code_flow(state, tx, client);
                        }
                        Some(PendingOAuthFlow::KittyPkce) => {
                            start_kitty_pkce_flow(state, tx, client);
                        }
                        None => {
                            state.login_ui.step = LoginStep::Hub;
                        }
                    }
                }
            }
            KeyCode::Char(c) if is_plain_text_char(modifiers) => {
                if state.login_ui.setup_field_idx == 0 {
                    state.login_ui.client_id_input.push(c);
                } else {
                    state.login_ui.client_secret_input.push(c);
                }
            }
            _ => {}
        },
    }
}
