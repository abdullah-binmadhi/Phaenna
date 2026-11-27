//! Asynchronous Karaoke orchestration, pre-flight lyrics caching, DSP profile isolation,
//! Party 5-second intermission sequencing, Tap-to-Sync capture, and Stage/Setlist controls.

use crate::api::player::{build_karaoke_audio_filter_chain, KaraokeAudioDsp, KaraokeVocalMode};
use crate::api::ytmusic::{
    fetch_lrclib_candidates, fetch_lrclib_synced_lyrics, fetch_ytm_lyrics, fetch_ytm_search,
    fetch_ytm_up_next, search_ytm_karaoke_versions, YtmLyrics, YtmSearchFilter, YtmTrack,
};
use crate::db::karaoke::{
    add_karaoke_track, create_karaoke_playlist, delete_karaoke_item, delete_karaoke_playlist,
    ensure_default_karaoke_playlist, get_karaoke_lyrics_cache, get_karaoke_track_pref,
    increment_karaoke_sung_count, list_karaoke_items, list_karaoke_playlists, move_karaoke_item,
    swap_karaoke_item_source, update_karaoke_item_singer, upsert_karaoke_lyrics_cache,
    upsert_karaoke_track_pref, KaraokeLyricsReadiness, KaraokePlaylistEntry, KaraokeTrackPref,
};
use crate::messages::AppMessage;
use crate::services::auth::GoogleAccountSession;
use crate::services::ipc::PlayerCommand;
use crate::state::{
    AppState, KaraokeHit, KaraokeMode, KaraokeVersionPickerState, YtmSection,
};
use crossterm::event::{KeyCode, KeyModifiers};
use sqlx::SqlitePool;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

pub async fn hydrate_karaoke_from_db(state: &mut AppState, pool: &SqlitePool) {
    let now = crate::now_unix();
    let active_id = ensure_default_karaoke_playlist(pool, now)
        .await
        .unwrap_or(1);
    let playlists = list_karaoke_playlists(pool).await.unwrap_or_default();
    let eff_id = if state.music.karaoke.active_playlist_id > 0
        && playlists
            .iter()
            .any(|p| p.id == state.music.karaoke.active_playlist_id)
    {
        state.music.karaoke.active_playlist_id
    } else {
        playlists.first().map(|p| p.id).unwrap_or(active_id)
    };
    let items = list_karaoke_items(pool, eff_id).await.unwrap_or_default();
    for e in &items {
        if let Ok(Some(pref)) = get_karaoke_track_pref(pool, &e.video_id).await {
            state.music.karaoke.track_prefs.insert(e.video_id.clone(), pref);
        }
    }
    state.music.karaoke.playlists = playlists;
    state.music.karaoke.active_playlist_id = eff_id;
    state.music.karaoke.items = items;
}

pub async fn reload_active_karaoke_playlist(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let playlists = list_karaoke_playlists(pool).await.unwrap_or_default();
    if state.music.karaoke.active_playlist_id <= 0
        || !playlists
            .iter()
            .any(|p| p.id == state.music.karaoke.active_playlist_id)
    {
        state.music.karaoke.active_playlist_id = playlists
            .first()
            .map(|p| p.id)
            .unwrap_or(ensure_default_karaoke_playlist(pool, crate::now_unix()).await.unwrap_or(1));
    }
    let pl_id = state.music.karaoke.active_playlist_id;
    let items = list_karaoke_items(pool, pl_id).await.unwrap_or_default();
    state.music.karaoke.playlists = playlists;
    state.music.karaoke.items = items;
    if !state.music.karaoke.items.is_empty()
        && state.music.karaoke.cursor >= state.music.karaoke.items.len()
    {
        state.music.karaoke.cursor = state.music.karaoke.items.len() - 1;
    }

    let mut need_prefetch = Vec::new();
    let mut seen = HashSet::new();
    for e in &mut state.music.karaoke.items {
        if e.lyrics_status == KaraokeLyricsReadiness::Unknown && seen.insert(e.video_id.clone()) {
            e.lyrics_status = KaraokeLyricsReadiness::Checking;
            need_prefetch.push(e.to_ytm_track());
        }
    }
    if !need_prefetch.is_empty() {
        spawn_karaoke_prefetch(
            need_prefetch,
            pool.clone(),
            tx.clone(),
            client.clone(),
            state.auth_vault.active_account().cloned(),
        );
    }
}

pub fn spawn_karaoke_prefetch(
    tracks: Vec<YtmTrack>,
    pool: SqlitePool,
    tx: mpsc::Sender<AppMessage>,
    client: reqwest::Client,
    session_opt: Option<GoogleAccountSession>,
) {
    tokio::spawn(async move {
        for tr in tracks {
            let vid = tr.video_id.clone();
            if let Ok(Some(cached)) = get_karaoke_lyrics_cache(&pool, &vid).await {
                if !cached.lines.is_empty() {
                    let _ = tx
                        .send(AppMessage::KaraokeLyricsPrefetched {
                            video_id: vid,
                            lyrics: cached,
                        })
                        .await;
                    continue;
                }
            }

            let ytm_lrc = match fetch_ytm_up_next(&client, session_opt.as_ref(), &vid, None, None).await {
                Ok(q) => {
                    if let Some(ref lid) = q.lyrics_browse_id {
                        fetch_ytm_lyrics(&client, session_opt.as_ref(), lid).await.ok()
                    } else {
                        None
                    }
                }
                Err(_) => None,
            };

            let resolved = if ytm_lrc.as_ref().is_some_and(|l| l.is_timed) {
                ytm_lrc.unwrap()
            } else if let Some(synced) = fetch_lrclib_synced_lyrics(
                &client,
                &tr.title,
                &tr.artist,
                tr.album.as_deref(),
                tr.duration_secs,
            )
            .await
            {
                synced
            } else {
                ytm_lrc.unwrap_or_default()
            };

            let _ = upsert_karaoke_lyrics_cache(&pool, &vid, &resolved, crate::now_unix()).await;
            let _ = tx
                .send(AppMessage::KaraokeLyricsPrefetched {
                    video_id: vid,
                    lyrics: resolved,
                })
                .await;
        }
    });
}

pub async fn send_tracks_to_karaoke(
    tracks: Vec<YtmTrack>,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    if tracks.is_empty() {
        state.show_toast("Select a playable song first");
        return;
    }
    let now = crate::now_unix();
    let pl_id = if state.music.karaoke.active_playlist_id > 0 {
        state.music.karaoke.active_playlist_id
    } else {
        let id = ensure_default_karaoke_playlist(pool, now).await.unwrap_or(1);
        state.music.karaoke.active_playlist_id = id;
        id
    };

    for tr in &tracks {
        let _ = add_karaoke_track(pool, pl_id, tr, None, now).await;
    }
    reload_active_karaoke_playlist(state, pool, tx, client).await;

    let pl_name = state
        .music
        .karaoke
        .playlists
        .iter()
        .find(|p| p.id == state.music.karaoke.active_playlist_id)
        .map(|p| p.name.as_str())
        .unwrap_or("Stage Queue");

    if tracks.len() == 1 {
        state.show_toast(format!("★ Sent \"{}\" to {pl_name}", tracks[0].title));
    } else {
        state.show_toast(format!("★ Sent {} songs to {pl_name}", tracks.len()));
    }
}

pub fn apply_karaoke_dsp_chain(state: &mut AppState) {
    let dsp = KaraokeAudioDsp {
        vocal_mode: state.music.karaoke.vocal_mode,
        pitch_semitones: state.music.karaoke.pitch_semitones,
        speed_pct: state.music.karaoke.speed_pct,
    };
    let chain = build_karaoke_audio_filter_chain(&state.config.music, &dsp);
    if state.has_live_ipc()
        && state.music.active_audio_session_id == Some(state.active_session_id)
    {
        state.send_player_cmd(PlayerCommand::SetAudioFilterChain(chain.unwrap_or_default()));
    }
}

async fn persist_current_track_pref(state: &mut AppState, pool: &SqlitePool) {
    let Some(vid) = state
        .music
        .current_track()
        .map(|t| t.video_id.clone())
        .or_else(|| {
            state
                .music
                .karaoke
                .active_item_id
                .and_then(|aid| {
                    state
                        .music
                        .karaoke
                        .items
                        .iter()
                        .find(|e| e.id == aid)
                        .map(|e| e.video_id.clone())
                })
        })
    else {
        return;
    };
    let pref = KaraokeTrackPref {
        video_id: vid.clone(),
        pitch_semitones: state.music.karaoke.pitch_semitones,
        lyric_offset_ms: state.music.karaoke.lyric_offset_ms,
        vocal_mode: state.music.karaoke.vocal_mode.as_u8(),
        speed_pct: state.music.karaoke.speed_pct,
        sweep_speed_pct: state.music.karaoke.sweep_speed_pct,
        loop_start_ms: state.music.karaoke.loop_start_ms,
        loop_end_ms: state.music.karaoke.loop_end_ms,
    };
    state.music.karaoke.track_prefs.insert(vid, pref.clone());
    let _ = upsert_karaoke_track_pref(pool, &pref, crate::now_unix()).await;
}

pub async fn play_karaoke_entry_at(
    idx: usize,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let Some(entry) = state.music.karaoke.items.get(idx).cloned() else {
        return;
    };
    state.music.karaoke.session_active = true;
    state.music.karaoke.active_item_id = Some(entry.id);
    state.music.karaoke.cursor = idx;
    state.music.karaoke.intermission_next_idx = None;
    state.music.karaoke.intermission_deadline = None;
    state.music.karaoke.intermission_paused = false;
    state.music.karaoke.tap_sync_active = false;
    state.music.karaoke.tap_sync_stamps.clear();
    state.music.karaoke.lyrics_manual_scroll = false;
    state.music.karaoke.lyrics_scroll = 0;

    let pref = get_karaoke_track_pref(pool, &entry.video_id)
        .await
        .ok()
        .flatten()
        .or_else(|| state.music.karaoke.track_prefs.get(&entry.video_id).cloned())
        .unwrap_or_else(|| KaraokeTrackPref {
            video_id: entry.video_id.clone(),
            pitch_semitones: entry.pitch_semitones,
            lyric_offset_ms: entry.lyric_offset_ms,
            ..Default::default()
        });
    state.music.karaoke.pitch_semitones = pref.pitch_semitones;
    state.music.karaoke.lyric_offset_ms = pref.lyric_offset_ms;
    state.music.karaoke.vocal_mode = KaraokeVocalMode::from_u8(pref.vocal_mode);
    state.music.karaoke.speed_pct = pref.speed_pct;
    state.music.karaoke.sweep_speed_pct = if pref.sweep_speed_pct == 0 {
        100
    } else {
        pref.sweep_speed_pct.clamp(50, 200)
    };
    state.music.karaoke.loop_start_ms = pref.loop_start_ms;
    state.music.karaoke.loop_end_ms = pref.loop_end_ms;

    state.music.queue = state
        .music
        .karaoke
        .items
        .iter()
        .map(KaraokePlaylistEntry::to_ytm_track)
        .collect();

    crate::play_music_track_for_karaoke(idx, state, pool, tx, client).await;

    // Immediately load pre-fetched synced lyrics from SQLite for 0ms intro readiness!
    if let Ok(Some(cached)) = get_karaoke_lyrics_cache(pool, &entry.video_id).await {
        if !cached.lines.is_empty() {
            state.music.lyrics = Some(cached);
            state.music.lyrics_loading = false;
        }
    }
    apply_karaoke_dsp_chain(state);
}

pub fn check_karaoke_ab_loop(pos_secs: f64, state: &mut AppState) {
    if !state.music.karaoke.session_active || state.music.karaoke.mode != KaraokeMode::Practice {
        return;
    }
    let (Some(a), Some(b)) = (
        state.music.karaoke.loop_start_ms,
        state.music.karaoke.loop_end_ms,
    ) else {
        return;
    };
    if b <= a + 300 || !state.has_live_ipc() {
        return;
    }
    let pos_ms = state.music.effective_lyric_pos_ms(pos_secs);
    if pos_ms >= b {
        let target = (a as f64) / 1000.0;
        state.active_playback_pos = target;
        state.active_playback_synced_at = Some(Instant::now());
        state.send_player_cmd(PlayerCommand::SeekAbsolute(target));
    }
}

pub async fn advance_karaoke_on_eof(
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    let cur_pos = state
        .music
        .karaoke
        .active_item_id
        .and_then(|aid| state.music.karaoke.items.iter().position(|e| e.id == aid))
        .unwrap_or(state.music.queue_index);

    if let Some(finished) = state.music.karaoke.items.get(cur_pos).cloned() {
        let _ = increment_karaoke_sung_count(pool, finished.id).await;
        if state.music.karaoke.mode == KaraokeMode::Party {
            state.music.karaoke.sung_history.push(finished);
        }
    }

    let next_idx = if cur_pos + 1 < state.music.karaoke.items.len() {
        Some(cur_pos + 1)
    } else if state.music.repeat == crate::state::YtmRepeatMode::All && !state.music.karaoke.items.is_empty() {
        Some(0)
    } else {
        None
    };

    let Some(ni) = next_idx else {
        state.show_toast("◈ Karaoke Setlist finished");
        return;
    };

    if state.music.karaoke.mode == KaraokeMode::Party {
        state.music.karaoke.intermission_next_idx = Some(ni);
        state.music.karaoke.intermission_deadline = Some(Instant::now() + Duration::from_secs(5));
        state.music.karaoke.intermission_paused = false;
    } else {
        play_karaoke_entry_at(ni, state, pool, tx, client).await;
    }
}

fn current_selected_karaoke_idx(state: &AppState) -> Option<usize> {
    (!state.music.karaoke.items.is_empty())
        .then(|| state.music.karaoke.cursor.min(state.music.karaoke.items.len() - 1))
}

async fn commit_tap_sync_stamps(state: &mut AppState, pool: &SqlitePool) {
    if state.music.karaoke.tap_sync_stamps.is_empty() {
        state.music.karaoke.tap_sync_active = false;
        return;
    }
    let Some(vid) = state.active_playback_video_id.clone() else {
        state.music.karaoke.tap_sync_active = false;
        return;
    };
    if let Some(ref mut lyrics) = state.music.lyrics {
        lyrics.apply_tap_sync_stamps(&state.music.karaoke.tap_sync_stamps);
        let cloned = lyrics.clone();
        let _ = upsert_karaoke_lyrics_cache(pool, &vid, &cloned, crate::now_unix()).await;
        for e in &mut state.music.karaoke.items {
            if e.video_id == vid {
                e.lyrics_status = KaraokeLyricsReadiness::from_lyrics(&cloned);
            }
        }
        state.show_toast("★ Saved local Tap-to-Sync timings to SQLite!");
    }
    state.music.karaoke.tap_sync_active = false;
    state.music.karaoke.tap_sync_stamps.clear();
}

pub async fn handle_karaoke_hit(
    hit: KaraokeHit,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) {
    match hit {
        KaraokeHit::ToggleMode => {
            state.music.karaoke.mode = state.music.karaoke.mode.toggle();
            state.show_toast(format!("Karaoke Mode: {}", state.music.karaoke.mode.label()));
        }
        KaraokeHit::ToggleFocus => {
            state.music.karaoke.stage_focus = !state.music.karaoke.stage_focus;
            state.kitty_flush_needed = true;
        }
        KaraokeHit::ToggleFullscreen => {
            state.music.karaoke.fullscreen = !state.music.karaoke.fullscreen;
            if !state.music.karaoke.fullscreen {
                state.music.karaoke.show_compact_setlist = false;
            }
            state.kitty_flush_needed = true;
        }
        KaraokeHit::ToggleCompactSetlist => {
            state.music.karaoke.show_compact_setlist = !state.music.karaoke.show_compact_setlist;
            state.kitty_flush_needed = true;
        }
        KaraokeHit::CyclePlaylistNext => {
            if state.music.karaoke.playlists.len() > 1 {
                let pos = state
                    .music
                    .karaoke
                    .playlists
                    .iter()
                    .position(|p| p.id == state.music.karaoke.active_playlist_id)
                    .unwrap_or(0);
                let next_id = state.music.karaoke.playlists
                    [(pos + 1) % state.music.karaoke.playlists.len()]
                .id;
                state.music.karaoke.active_playlist_id = next_id;
                reload_active_karaoke_playlist(state, pool, tx, client).await;
            } else {
                state.show_toast("Press [+New] to create another Karaoke playlist");
            }
        }
        KaraokeHit::DeleteCurrentPlaylist => {
            let cur_id = state.music.karaoke.active_playlist_id;
            let _ = delete_karaoke_playlist(pool, cur_id, crate::now_unix()).await;
            state.music.karaoke.active_playlist_id = 0;
            reload_active_karaoke_playlist(state, pool, tx, client).await;
            state.show_toast("Removed Karaoke playlist");
        }
        KaraokeHit::StartNewPlaylist => {
            state.music.karaoke.singer_prompt_item_id = None;
            state.music.karaoke.inline_search_editing = false;
            state.music.karaoke.playlist_prompt_open = true;
            state.music.karaoke.playlist_prompt_input.clear();
        }
        KaraokeHit::SaveQueueAsPlaylist => {
            let tracks = state.music.queue.clone();
            if tracks.is_empty() {
                state.show_toast("Play a mix or queue tracks first");
                return;
            }
            let now = crate::now_unix();
            let name = format!("Saved Set {}", state.music.karaoke.playlists.len() + 1);
            if let Ok(new_id) = create_karaoke_playlist(pool, &name, now).await {
                state.music.karaoke.active_playlist_id = new_id;
                send_tracks_to_karaoke(tracks, state, pool, tx, client).await;
            }
        }
        KaraokeHit::ToggleInlineSearch => {
            state.music.karaoke.inline_search_open = !state.music.karaoke.inline_search_open;
            state.music.karaoke.inline_search_editing = state.music.karaoke.inline_search_open;
        }
        KaraokeHit::FocusInlineSearch => {
            state.music.karaoke.inline_search_open = true;
            state.music.karaoke.inline_search_editing = true;
            state.music.karaoke.playlist_prompt_open = false;
            state.music.karaoke.singer_prompt_item_id = None;
        }
        KaraokeHit::AddSearchTrack(idx) => {
            if let Some(tr) = state.music.karaoke.inline_search_results.get(idx).cloned() {
                send_tracks_to_karaoke(vec![tr], state, pool, tx, client).await;
            }
        }
        KaraokeHit::SelectItem(idx) => {
            let now = Instant::now();
            let dbl = state
                .music
                .karaoke
                .last_click
                .is_some_and(|(p, ts)| p == idx && now.duration_since(ts) < Duration::from_millis(550));
            state.music.karaoke.cursor = idx;
            if dbl {
                state.music.karaoke.last_click = None;
                play_karaoke_entry_at(idx, state, pool, tx, client).await;
            } else {
                state.music.karaoke.last_click = Some((idx, now));
            }
        }
        KaraokeHit::PlayItem(idx) => {
            play_karaoke_entry_at(idx, state, pool, tx, client).await;
        }
        KaraokeHit::MoveItemUp(idx) => {
            if let Some(e) = state.music.karaoke.items.get(idx).cloned() {
                let _ = move_karaoke_item(pool, state.music.karaoke.active_playlist_id, e.id, -1).await;
                if idx > 0 {
                    state.music.karaoke.cursor = idx - 1;
                }
                reload_active_karaoke_playlist(state, pool, tx, client).await;
            }
        }
        KaraokeHit::MoveItemDown(idx) => {
            if let Some(e) = state.music.karaoke.items.get(idx).cloned() {
                let _ = move_karaoke_item(pool, state.music.karaoke.active_playlist_id, e.id, 1).await;
                if idx + 1 < state.music.karaoke.items.len() {
                    state.music.karaoke.cursor = idx + 1;
                }
                reload_active_karaoke_playlist(state, pool, tx, client).await;
            }
        }
        KaraokeHit::RemoveItem(idx) => {
            if let Some(e) = state.music.karaoke.items.get(idx).cloned() {
                let _ = delete_karaoke_item(pool, e.id).await;
                reload_active_karaoke_playlist(state, pool, tx, client).await;
                state.show_toast(format!("Removed \"{}\"", e.title));
            }
        }
        KaraokeHit::StartSingerPrompt(idx) => {
            if let Some(e) = state.music.karaoke.items.get(idx).cloned() {
                state.music.karaoke.playlist_prompt_open = false;
                state.music.karaoke.inline_search_editing = false;
                state.music.karaoke.singer_prompt_item_id = Some(e.id);
                state.music.karaoke.singer_prompt_input = e.singer_name.unwrap_or_default();
            }
        }
        KaraokeHit::OpenVersionPicker(idx) => {
            if let Some(e) = state.music.karaoke.items.get(idx).cloned() {
                state.music.karaoke.version_picker = Some(KaraokeVersionPickerState {
                    item_id: e.id,
                    video_id: e.video_id.clone(),
                    track_title: e.title.clone(),
                    loading: true,
                    audio_candidates: Vec::new(),
                    lrc_candidates: Vec::new(),
                    tab_is_lrc: false,
                    cursor: 0,
                });
                let c = client.clone();
                let sess = state.auth_vault.active_account().cloned();
                let tx_v = tx.clone();
                let item_id = e.id;
                let vid = e.video_id;
                let title = e.title;
                let artist = e.artist;
                let dur = e.duration_secs;
                tokio::spawn(async move {
                    let (audio_res, lrc_cands) = tokio::join!(
                        search_ytm_karaoke_versions(&c, sess.as_ref(), &title, &artist),
                        fetch_lrclib_candidates(&c, &title, &artist, dur),
                    );
                    let _ = tx_v
                        .send(AppMessage::KaraokeVersionCandidatesLoaded {
                            item_id,
                            video_id: vid,
                            audio_candidates: audio_res.unwrap_or_default(),
                            lrc_candidates: lrc_cands,
                        })
                        .await;
                });
            }
        }
        KaraokeHit::CloseVersionPicker => {
            state.music.karaoke.version_picker = None;
        }
        KaraokeHit::PickAudioVersion(ci) => {
            let Some(vp) = state.music.karaoke.version_picker.clone() else {
                return;
            };
            let Some(tr) = vp.audio_candidates.get(ci).cloned() else {
                return;
            };
            let _ = swap_karaoke_item_source(pool, vp.item_id, &tr).await;
            state.music.karaoke.version_picker = None;
            reload_active_karaoke_playlist(state, pool, tx, client).await;
            state.show_toast(format!("◈ Swapped backing to: {}", tr.title));
            if state.music.karaoke.active_item_id == Some(vp.item_id) {
                if let Some(pos) = state
                    .music
                    .karaoke
                    .items
                    .iter()
                    .position(|e| e.id == vp.item_id)
                {
                    play_karaoke_entry_at(pos, state, pool, tx, client).await;
                }
            }
        }
        KaraokeHit::PickLrcCandidate(ci) => {
            let Some(vp) = state.music.karaoke.version_picker.clone() else {
                return;
            };
            let Some((lbl, lrc)) = vp.lrc_candidates.get(ci).cloned() else {
                return;
            };
            let _ = upsert_karaoke_lyrics_cache(pool, &vp.video_id, &lrc, crate::now_unix()).await;
            if state.active_playback_video_id.as_deref() == Some(vp.video_id.as_str()) {
                state.music.lyrics = Some(lrc.clone());
                state.music.lyrics_loading = false;
            }
            for e in &mut state.music.karaoke.items {
                if e.video_id == vp.video_id {
                    e.lyrics_status = KaraokeLyricsReadiness::from_lyrics(&lrc);
                }
            }
            state.music.karaoke.version_picker = None;
            state.show_toast(format!("★ Loaded lyrics: {lbl}"));
        }
        KaraokeHit::CycleVocalMode => {
            state.music.karaoke.vocal_mode = state.music.karaoke.vocal_mode.next();
            apply_karaoke_dsp_chain(state);
            persist_current_track_pref(state, pool).await;
            state.show_toast(format!("Vocal Guide: {}", state.music.karaoke.vocal_mode.label()));
        }
        KaraokeHit::PitchDelta(delta) => {
            state.music.karaoke.pitch_semitones =
                (state.music.karaoke.pitch_semitones + delta).clamp(-6, 6);
            apply_karaoke_dsp_chain(state);
            persist_current_track_pref(state, pool).await;
            state.show_toast(format!("Key: {:+} semitones", state.music.karaoke.pitch_semitones));
        }
        KaraokeHit::SpeedDelta(delta) => {
            let next = if state.music.karaoke.speed_pct <= 75 && delta < 0 {
                100
            } else {
                (state.music.karaoke.speed_pct as i32 + delta as i32).clamp(75, 115) as u16
            };
            state.music.karaoke.speed_pct = next;
            apply_karaoke_dsp_chain(state);
            persist_current_track_pref(state, pool).await;
            state.show_toast(format!("Practice Tempo: {:.2}x", next as f64 / 100.0));
        }
        KaraokeHit::SweepSpeedDelta(delta) => {
            let next = (state.music.karaoke.sweep_speed_pct as i32 + delta as i32).clamp(50, 200) as u16;
            state.music.karaoke.sweep_speed_pct = next;
            persist_current_track_pref(state, pool).await;
            state.show_toast(format!("Word Sweep Speed: {:.1}x", next as f64 / 100.0));
        }
        KaraokeHit::ResetSweepSpeed => {
            state.music.karaoke.sweep_speed_pct = 100;
            persist_current_track_pref(state, pool).await;
            state.show_toast("Word Sweep Speed: 1.0x (Reset)");
        }
        KaraokeHit::OffsetDelta(delta) => {
            state.music.karaoke.lyric_offset_ms =
                (state.music.karaoke.lyric_offset_ms + delta).clamp(-8_000, 8_000);
            persist_current_track_pref(state, pool).await;
            state.show_toast(format!("Lyric Sync Offset: {:+}ms", state.music.karaoke.lyric_offset_ms));
        }
        KaraokeHit::SetLoopA => {
            let ms = state
                .music
                .effective_lyric_pos_ms(state.effective_playback_pos_secs());
            state.music.karaoke.loop_start_ms = Some(ms);
            if state
                .music
                .karaoke
                .loop_end_ms
                .is_some_and(|b| b <= ms + 300)
            {
                state.music.karaoke.loop_end_ms = None;
            }
            persist_current_track_pref(state, pool).await;
            state.show_toast(format!("Chorus Loop A = {:.1}s", ms as f64 / 1000.0));
        }
        KaraokeHit::SetLoopB => {
            let ms = state
                .music
                .effective_lyric_pos_ms(state.effective_playback_pos_secs());
            state.music.karaoke.loop_end_ms = Some(ms);
            if state.music.karaoke.loop_start_ms.is_none() {
                state.music.karaoke.loop_start_ms = Some(ms.saturating_sub(15_000));
            }
            persist_current_track_pref(state, pool).await;
            state.show_toast(format!("Chorus Loop B = {:.1}s", ms as f64 / 1000.0));
        }
        KaraokeHit::ClearLoop => {
            state.music.karaoke.loop_start_ms = None;
            state.music.karaoke.loop_end_ms = None;
            persist_current_track_pref(state, pool).await;
            state.show_toast("Cleared Chorus Loop");
        }
        KaraokeHit::ToggleTapSync => {
            if state.music.karaoke.tap_sync_active {
                state.music.karaoke.tap_sync_active = false;
                state.music.karaoke.tap_sync_stamps.clear();
            } else if state.music.lyrics.is_some() {
                state.music.karaoke.tap_sync_active = true;
                state.music.karaoke.tap_sync_stamps.clear();
                state.show_toast("Tap-to-Sync ON — press [t] on each line start, [Enter] to save");
            } else {
                state.show_toast("Load a song with lyrics text first");
            }
        }
        KaraokeHit::StampTapSync => {
            if state.music.karaoke.tap_sync_active {
                let total = state
                    .music
                    .lyrics
                    .as_ref()
                    .map(|l| l.lines.len())
                    .unwrap_or(0);
                if state.music.karaoke.tap_sync_stamps.len() < total {
                    let ms = (state.effective_playback_pos_secs().max(0.0) * 1000.0) as u64;
                    state.music.karaoke.tap_sync_stamps.push(ms);
                    if state.music.karaoke.tap_sync_stamps.len() >= total {
                        commit_tap_sync_stamps(state, pool).await;
                    }
                }
            }
        }
        KaraokeHit::CommitTapSync => {
            commit_tap_sync_stamps(state, pool).await;
        }
        KaraokeHit::CancelTapSync => {
            state.music.karaoke.tap_sync_active = false;
            state.music.karaoke.tap_sync_stamps.clear();
            state.show_toast("Tap-to-Sync cancelled");
        }
        KaraokeHit::SeekLyricMs(ms) => {
            let offset_adj = (ms as i64 - state.music.karaoke.lyric_offset_ms).max(0) as f64 / 1000.0;
            state.music.karaoke.lyrics_manual_scroll = false;
            if state.has_live_ipc() {
                state.active_playback_pos = offset_adj;
                state.active_playback_synced_at = Some(Instant::now());
                state.send_player_cmd(PlayerCommand::SeekAbsolute(offset_adj));
            }
        }
        KaraokeHit::ResumeLiveLyrics => {
            state.music.karaoke.lyrics_manual_scroll = false;
        }
        KaraokeHit::IntermissionStartNow => {
            if let Some(ni) = state.music.karaoke.intermission_next_idx.take() {
                state.music.karaoke.intermission_deadline = None;
                state.music.karaoke.intermission_paused = false;
                play_karaoke_entry_at(ni, state, pool, tx, client).await;
            }
        }
        KaraokeHit::IntermissionHoldToggle => {
            state.music.karaoke.intermission_paused = !state.music.karaoke.intermission_paused;
            if !state.music.karaoke.intermission_paused {
                state.music.karaoke.intermission_deadline =
                    Some(Instant::now() + Duration::from_secs(5));
            }
        }
        KaraokeHit::IntermissionSkip => {
            if let Some(ni) = state.music.karaoke.intermission_next_idx {
                if ni + 1 < state.music.karaoke.items.len() {
                    state.music.karaoke.intermission_next_idx = Some(ni + 1);
                    state.music.karaoke.intermission_deadline =
                        Some(Instant::now() + Duration::from_secs(5));
                } else {
                    state.music.karaoke.intermission_next_idx = None;
                    state.music.karaoke.intermission_deadline = None;
                    state.show_toast("Skipped last song");
                }
            }
        }
    }
}

pub async fn handle_karaoke_key(
    code: KeyCode,
    modifiers: KeyModifiers,
    state: &mut AppState,
    pool: &SqlitePool,
    tx: &mpsc::Sender<AppMessage>,
    client: &reqwest::Client,
) -> bool {
    // 1. Version Picker Modal
    if let Some(ref mut vp) = state.music.karaoke.version_picker {
        let len = if vp.tab_is_lrc {
            vp.lrc_candidates.len()
        } else {
            vp.audio_candidates.len()
        };
        match code {
            KeyCode::Esc | KeyCode::Char('V') => {
                state.music.karaoke.version_picker = None;
            }
            KeyCode::Tab => {
                vp.tab_is_lrc = !vp.tab_is_lrc;
                vp.cursor = 0;
            }
            KeyCode::Up | KeyCode::Char('k') => vp.cursor = vp.cursor.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') if len > 0 => {
                vp.cursor = (vp.cursor + 1).min(len - 1);
            }
            KeyCode::Enter if len > 0 => {
                let hit = if vp.tab_is_lrc {
                    KaraokeHit::PickLrcCandidate(vp.cursor)
                } else {
                    KaraokeHit::PickAudioVersion(vp.cursor)
                };
                handle_karaoke_hit(hit, state, pool, tx, client).await;
            }
            _ => {}
        }
        return false;
    }

    // 2. Text input prompts (New Playlist / Singer Mic Tag / Inline Song Search)
    if state.music.karaoke.playlist_prompt_open {
        match code {
            KeyCode::Esc => state.music.karaoke.playlist_prompt_open = false,
            KeyCode::Backspace => {
                state.music.karaoke.playlist_prompt_input.pop();
            }
            KeyCode::Enter => {
                let name = state.music.karaoke.playlist_prompt_input.trim().to_string();
                state.music.karaoke.playlist_prompt_open = false;
                if !name.is_empty() {
                    if let Ok(id) = create_karaoke_playlist(pool, &name, crate::now_unix()).await {
                        state.music.karaoke.active_playlist_id = id;
                        reload_active_karaoke_playlist(state, pool, tx, client).await;
                        state.show_toast(format!("◈ Created Karaoke list: {name}"));
                    }
                }
            }
            KeyCode::Char(c) if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                state.music.karaoke.playlist_prompt_input.push(c);
            }
            _ => {}
        }
        return false;
    }

    if let Some(item_id) = state.music.karaoke.singer_prompt_item_id {
        match code {
            KeyCode::Esc => state.music.karaoke.singer_prompt_item_id = None,
            KeyCode::Backspace => {
                state.music.karaoke.singer_prompt_input.pop();
            }
            KeyCode::Enter => {
                let name = state.music.karaoke.singer_prompt_input.clone();
                state.music.karaoke.singer_prompt_item_id = None;
                let _ = update_karaoke_item_singer(pool, item_id, &name).await;
                reload_active_karaoke_playlist(state, pool, tx, client).await;
                state.show_toast(if name.trim().is_empty() {
                    "Cleared singer mic tag".to_string()
                } else {
                    format!("Mic assigned to: {}", name.trim())
                });
            }
            KeyCode::Char(c) if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                state.music.karaoke.singer_prompt_input.push(c);
            }
            _ => {}
        }
        return false;
    }

    if state.music.karaoke.inline_search_editing {
        match code {
            KeyCode::Esc => state.music.karaoke.inline_search_editing = false,
            KeyCode::Backspace => {
                state.music.karaoke.inline_search_query.pop();
            }
            KeyCode::Enter => {
                let q = state.music.karaoke.inline_search_query.trim().to_string();
                if !q.is_empty() {
                    state.music.karaoke.inline_search_loading = true;
                    state.music.karaoke.inline_search_editing = false;
                    let c = client.clone();
                    let sess = state.auth_vault.active_account().cloned();
                    let tx_s = tx.clone();
                    tokio::spawn(async move {
                        let mut tracks = Vec::new();
                        if let Ok(page) =
                            fetch_ytm_search(&c, sess.as_ref(), &q, YtmSearchFilter::Songs, None).await
                        {
                            tracks.extend(page.tracks);
                            for s in page.shelves {
                                for it in s.items {
                                    if let Some(tr) = it.to_track() {
                                        if !tracks.iter().any(|e| e.video_id == tr.video_id) {
                                            tracks.push(tr);
                                        }
                                    }
                                }
                            }
                        }
                        let _ = tx_s
                            .send(AppMessage::KaraokeInlineSearchLoaded { query: q, tracks })
                            .await;
                    });
                }
            }
            KeyCode::Char(c) if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                state.music.karaoke.inline_search_query.push(c);
            }
            _ => {}
        }
        return false;
    }

    // 3. 5-Second Party Intermission Controls take priority for Space / p / Enter
    if state.music.karaoke.intermission_deadline.is_some() {
        match code {
            KeyCode::Char(' ') | KeyCode::Enter => {
                handle_karaoke_hit(KaraokeHit::IntermissionStartNow, state, pool, tx, client).await;
                return false;
            }
            KeyCode::Char('p') => {
                handle_karaoke_hit(KaraokeHit::IntermissionHoldToggle, state, pool, tx, client).await;
                return false;
            }
            KeyCode::Char('>') => {
                handle_karaoke_hit(KaraokeHit::IntermissionSkip, state, pool, tx, client).await;
                return false;
            }
            _ => {}
        }
    }

    // 4. Dedicated Stage & Setlist Hotkeys (while keeping app navigation responsive)
    match code {
        KeyCode::Char('q') => crate::open_confirm_quit(state),
        KeyCode::Char('?') => state.overlay = crate::state::ActiveOverlay::Help,
        KeyCode::Char('s') | KeyCode::Char(',') => {
            state.music.settings_cursor = 0;
            state.overlay = crate::state::ActiveOverlay::MusicSettings;
            state.kitty_flush_needed = true;
        }
        KeyCode::Char('e') => {
            state.music.eq.save_editing = false;
            state.music.eq.dropdown_open = false;
            state.overlay = crate::state::ActiveOverlay::MusicEq;
            state.kitty_flush_needed = true;
        }
        KeyCode::Tab => {
            if state.music.karaoke.fullscreen
                || state.music.karaoke.stage_focus
                || state.music.karaoke.setlist_rect.width == 0
            {
                handle_karaoke_hit(KaraokeHit::ToggleCompactSetlist, state, pool, tx, client).await;
            } else {
                state.active_category = (state.active_category + 1) % state.categories.len();
            }
        }
        KeyCode::BackTab => {
            state.music.karaoke.fullscreen = false;
            state.active_category =
                (state.active_category + state.categories.len() - 1) % state.categories.len();
        }
        KeyCode::Char(d @ '1'..='7') => {
            state.music.karaoke.fullscreen = false;
            let idx = (d as usize) - ('1' as usize);
            state.active_category = idx.min(state.categories.len() - 1);
        }
        KeyCode::Char('m') => handle_karaoke_hit(KaraokeHit::ToggleMode, state, pool, tx, client).await,
        KeyCode::Char('f') => handle_karaoke_hit(KaraokeHit::ToggleFocus, state, pool, tx, client).await,
        KeyCode::Char('F') => handle_karaoke_hit(KaraokeHit::ToggleFullscreen, state, pool, tx, client).await,
        KeyCode::Char('v') => handle_karaoke_hit(KaraokeHit::CycleVocalMode, state, pool, tx, client).await,
        KeyCode::Char('w') => handle_karaoke_hit(KaraokeHit::SweepSpeedDelta(-10), state, pool, tx, client).await,
        KeyCode::Char('W') => handle_karaoke_hit(KaraokeHit::SweepSpeedDelta(10), state, pool, tx, client).await,
        KeyCode::Char('+') | KeyCode::Char('=') => {
            handle_karaoke_hit(KaraokeHit::PitchDelta(1), state, pool, tx, client).await;
        }
        KeyCode::Char('-') | KeyCode::Char('_') => {
            handle_karaoke_hit(KaraokeHit::PitchDelta(-1), state, pool, tx, client).await;
        }
        KeyCode::Char('9') => handle_karaoke_hit(KaraokeHit::SpeedDelta(-5), state, pool, tx, client).await,
        KeyCode::Char('0') => handle_karaoke_hit(KaraokeHit::SpeedDelta(5), state, pool, tx, client).await,
        KeyCode::Char('[') | KeyCode::Char('{') => {
            let step = if code == KeyCode::Char('{') || modifiers.contains(KeyModifiers::SHIFT) {
                -500
            } else {
                -100
            };
            handle_karaoke_hit(KaraokeHit::OffsetDelta(step), state, pool, tx, client).await;
        }
        KeyCode::Char(']') | KeyCode::Char('}') => {
            let step = if code == KeyCode::Char('}') || modifiers.contains(KeyModifiers::SHIFT) {
                500
            } else {
                100
            };
            handle_karaoke_hit(KaraokeHit::OffsetDelta(step), state, pool, tx, client).await;
        }
        KeyCode::Char('a') => handle_karaoke_hit(KaraokeHit::SetLoopA, state, pool, tx, client).await,
        KeyCode::Char('b') => handle_karaoke_hit(KaraokeHit::SetLoopB, state, pool, tx, client).await,
        KeyCode::Char('x') => handle_karaoke_hit(KaraokeHit::ClearLoop, state, pool, tx, client).await,
        KeyCode::Char('t') => {
            if state.music.karaoke.tap_sync_active {
                handle_karaoke_hit(KaraokeHit::StampTapSync, state, pool, tx, client).await;
            } else {
                handle_karaoke_hit(KaraokeHit::ToggleTapSync, state, pool, tx, client).await;
            }
        }
        KeyCode::Char('/') => {
            handle_karaoke_hit(KaraokeHit::FocusInlineSearch, state, pool, tx, client).await;
        }
        KeyCode::Char('V') => {
            if let Some(idx) = current_selected_karaoke_idx(state) {
                handle_karaoke_hit(KaraokeHit::OpenVersionPicker(idx), state, pool, tx, client).await;
            }
        }
        KeyCode::Char('M') => {
            if let Some(idx) = current_selected_karaoke_idx(state) {
                handle_karaoke_hit(KaraokeHit::StartSingerPrompt(idx), state, pool, tx, client).await;
            }
        }
        KeyCode::Char('p') => handle_karaoke_hit(KaraokeHit::CyclePlaylistNext, state, pool, tx, client).await,
        KeyCode::Char('n') => handle_karaoke_hit(KaraokeHit::StartNewPlaylist, state, pool, tx, client).await,
        KeyCode::Up | KeyCode::Char('k') => {
            state.music.karaoke.cursor = state.music.karaoke.cursor.saturating_sub(1);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            if !state.music.karaoke.items.is_empty() {
                state.music.karaoke.cursor =
                    (state.music.karaoke.cursor + 1).min(state.music.karaoke.items.len() - 1);
            }
        }
        KeyCode::Delete | KeyCode::Char('d') => {
            if let Some(idx) = current_selected_karaoke_idx(state) {
                handle_karaoke_hit(KaraokeHit::RemoveItem(idx), state, pool, tx, client).await;
            }
        }
        KeyCode::Enter => {
            if state.music.karaoke.tap_sync_active {
                handle_karaoke_hit(KaraokeHit::CommitTapSync, state, pool, tx, client).await;
            } else if let Some(idx) = current_selected_karaoke_idx(state) {
                play_karaoke_entry_at(idx, state, pool, tx, client).await;
            }
        }
        KeyCode::Esc => {
            if state.music.karaoke.tap_sync_active {
                handle_karaoke_hit(KaraokeHit::CancelTapSync, state, pool, tx, client).await;
            } else if state.music.karaoke.inline_search_open {
                state.music.karaoke.inline_search_open = false;
            } else if state.music.karaoke.fullscreen && state.music.karaoke.show_compact_setlist {
                state.music.karaoke.show_compact_setlist = false;
            } else if state.music.karaoke.fullscreen {
                state.music.karaoke.fullscreen = false;
                state.kitty_flush_needed = true;
            } else {
                state.music.section = YtmSection::Home;
            }
        }
        KeyCode::Char(' ') => {
            if state.has_live_ipc() {
                state.live_playback.paused = !state.live_playback.paused;
                state.send_player_cmd(PlayerCommand::TogglePause);
            } else if let Some(idx) = current_selected_karaoke_idx(state) {
                play_karaoke_entry_at(idx, state, pool, tx, client).await;
            }
        }
        KeyCode::Char('>') => {
            let cur = state
                .music
                .karaoke
                .active_item_id
                .and_then(|aid| state.music.karaoke.items.iter().position(|e| e.id == aid))
                .unwrap_or(0);
            if cur + 1 < state.music.karaoke.items.len() {
                play_karaoke_entry_at(cur + 1, state, pool, tx, client).await;
            }
        }
        KeyCode::Char('<') => {
            let cur = state
                .music
                .karaoke
                .active_item_id
                .and_then(|aid| state.music.karaoke.items.iter().position(|e| e.id == aid))
                .unwrap_or(0);
            play_karaoke_entry_at(cur.saturating_sub(1), state, pool, tx, client).await;
        }
        KeyCode::Right => {
            if state.has_live_ipc() {
                state.send_player_cmd(PlayerCommand::SeekRelative(5.0));
            }
        }
        KeyCode::Left if state.has_live_ipc() => {
            state.send_player_cmd(PlayerCommand::SeekRelative(-5.0));
        }
        _ => {}
    }
    false
}

pub fn handle_karaoke_message(msg: AppMessage, state: &mut AppState) -> Option<AppMessage> {
    match msg {
        AppMessage::KaraokePlaylistsLoaded {
            playlists,
            active_playlist_id,
            items,
        } => {
            state.music.karaoke.playlists = playlists;
            state.music.karaoke.active_playlist_id = active_playlist_id;
            state.music.karaoke.items = items;
            None
        }
        AppMessage::KaraokeLyricsPrefetched { video_id, lyrics } => {
            let status = KaraokeLyricsReadiness::from_lyrics(&lyrics);
            for e in &mut state.music.karaoke.items {
                if e.video_id == video_id {
                    e.lyrics_status = status;
                }
            }
            if state.music.karaoke.session_active
                && state.active_playback_video_id.as_deref() == Some(video_id.as_str())
                && !state.music.lyrics.as_ref().is_some_and(|l| l.is_timed)
                && !lyrics.lines.is_empty()
            {
                state.music.lyrics = Some(lyrics);
                state.music.lyrics_loading = false;
            }
            None
        }
        AppMessage::KaraokeInlineSearchLoaded { query: _, tracks } => {
            state.music.karaoke.inline_search_loading = false;
            state.music.karaoke.inline_search_results = tracks;
            None
        }
        AppMessage::KaraokeVersionCandidatesLoaded {
            item_id,
            video_id: _,
            audio_candidates,
            lrc_candidates,
        } => {
            if let Some(ref mut vp) = state.music.karaoke.version_picker {
                if vp.item_id == item_id {
                    vp.loading = false;
                    vp.audio_candidates = audio_candidates;
                    vp.lrc_candidates = lrc_candidates;
                }
            }
            None
        }
        other => Some(other),
    }
}

/// Format LRC back cleanly if needed for export/logging.
#[allow(dead_code)]
pub fn apply_lyrics_sync_ready_status(lyrics: &YtmLyrics) -> KaraokeLyricsReadiness {
    KaraokeLyricsReadiness::from_lyrics(lyrics)
}
