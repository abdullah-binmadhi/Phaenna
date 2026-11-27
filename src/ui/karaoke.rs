//! Dedicated Karaoke Workspace UI (`YtmSection::Karaoke`):
//! - Responsive 2-zone stage + setlist companion below Phaenna's Bottom Player Bar
//! - Bounded centered lyric theatre (`max_w = 82`) preventing claustrophobia on small panes and barren voids on ultrawide monitors
//! - Unified Practice (3-step vocal guide, key/speed, sync offset, A/B loop, Tap-to-Sync) & Party (singer tags, 5s Up Next intermission, clean lock) workflows

use crate::api::ytmusic::YtmLyricLine;
use crate::db::cache::format_duration_secs;
use crate::state::{
    AppState, KaraokeHit, KaraokeLyricsReadiness, KaraokeMode, KaraokePlaylistEntry, MusicHit,
    ThumbPriority,
};
use crate::ui::components::cover::render_thumbnail_cover;
use crate::ui::music::{
    fit_joined_parts, karaoke_chunk_parts, karaoke_progress, render_vertical_scrollbar,
    truncate_str, truncate_words_clean, wrap_lyric_line_with_offsets,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KaraokeLayoutDims {
    pub compact_only_one_pane: bool,
    pub show_stage: bool,
    pub show_setlist: bool,
    pub stage_w: u16,
    pub setlist_w: u16,
    pub corridor_w: u16,
}

/// Compute balanced Karaoke body widths so neither the Lyric Stage nor the Setlist Companion
/// feels cramped (<30 cols) or overly stretched across an ultrawide canvas.
pub fn karaoke_layout_dims(
    body_w: u16,
    stage_focus: bool,
    fullscreen: bool,
    show_compact_setlist: bool,
) -> KaraokeLayoutDims {
    if fullscreen {
        return KaraokeLayoutDims {
            compact_only_one_pane: true,
            show_stage: true,
            show_setlist: false,
            stage_w: body_w,
            setlist_w: 0,
            corridor_w: body_w.saturating_sub(4).min(114).max(body_w.min(28)),
        };
    }

    if stage_focus {
        return KaraokeLayoutDims {
            compact_only_one_pane: true,
            show_stage: true,
            show_setlist: false,
            stage_w: body_w,
            setlist_w: 0,
            corridor_w: body_w.saturating_sub(4).min(96).max(body_w.min(24)),
        };
    }

    if body_w < 84 {
        let show_set = show_compact_setlist;
        return KaraokeLayoutDims {
            compact_only_one_pane: true,
            show_stage: !show_set,
            show_setlist: show_set,
            stage_w: if show_set { 0 } else { body_w },
            setlist_w: if show_set { body_w } else { 0 },
            corridor_w: body_w.saturating_sub(2).min(88),
        };
    }

    let setlist_w = ((body_w as u32 * 30) / 100).clamp(30, 38) as u16;
    let stage_w = body_w.saturating_sub(setlist_w);
    let corridor_w = stage_w.saturating_sub(4).min(92).max(stage_w.min(28));
    KaraokeLayoutDims {
        compact_only_one_pane: false,
        show_stage: true,
        show_setlist: true,
        stage_w,
        setlist_w,
        corridor_w,
    }
}

/// Compute strictly linear word-sweep progress (`sung_chars`, normalized `0.0..=1.0`) scaled manually
/// by `sweep_speed_pct` (`50..=200`, default `100`).
pub fn compute_lyric_sung_chars_and_frac(
    line: &YtmLyricLine,
    pos_ms: u64,
    next_start_ms: Option<u64>,
    sweep_speed_pct: u16,
) -> (usize, f64) {
    let total_chars = line.text.chars().count();
    if total_chars == 0 {
        return (0, 0.0);
    }
    let scale = (sweep_speed_pct.clamp(50, 200) as f64) / 100.0;

    if let Some(ref words) = line.words {
        if !words.is_empty() {
            let mut count = 0usize;
            for w in words {
                let wlen = w.text.chars().count();
                let raw_dur = w.end_ms.saturating_sub(w.start_ms).max(80) as f64;
                let eff_dur = (raw_dur / scale).max(40.0);
                if pos_ms <= w.start_ms {
                    break;
                }
                let elapsed = (pos_ms.saturating_sub(w.start_ms)) as f64;
                if elapsed >= eff_dur {
                    count += wlen;
                } else {
                    let frac = (elapsed / eff_dur).clamp(0.0, 1.0);
                    count += ((frac * wlen as f64).round() as usize).min(wlen);
                    break;
                }
            }
            let clamped = count.min(total_chars);
            return (clamped, (clamped as f64 / total_chars as f64).clamp(0.0, 1.0));
        }
    }

    if let Some(t0) = line.start_ms {
        let base_p = if (scale - 1.0).abs() < f64::EPSILON {
            karaoke_progress(pos_ms, t0, line.end_ms, next_start_ms)
        } else {
            let t1 = line
                .end_ms
                .filter(|&e| e > t0)
                .or_else(|| next_start_ms.filter(|&ns| ns > t0))
                .unwrap_or(t0.saturating_add(4000));
            let raw_dur = t1.saturating_sub(t0).max(250) as f64;
            let eff_dur = (raw_dur / scale).max(150.0);
            ((pos_ms.saturating_sub(t0)) as f64 / eff_dur).clamp(0.0, 1.0)
        };
        let chars = ((base_p * total_chars as f64).round() as usize).min(total_chars);
        (chars, base_p)
    } else {
        (total_chars, 1.0)
    }
}

pub fn compute_lyric_sung_chars(
    line: &YtmLyricLine,
    pos_ms: u64,
    next_start_ms: Option<u64>,
    sweep_speed_pct: u16,
) -> usize {
    compute_lyric_sung_chars_and_frac(line, pos_ms, next_start_ms, sweep_speed_pct).0
}

/// Render the full dedicated Karaoke workspace cleanly inside `area`.
pub fn render_karaoke_workspace(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    state.music.karaoke.hitboxes.clear();
    state.music.karaoke.stage_rect = Rect::default();
    state.music.karaoke.setlist_rect = Rect::default();

    if area.width < 28 || area.height < 10 {
        return;
    }

    let is_full = state.music.karaoke.fullscreen;
    let dims = karaoke_layout_dims(
        area.width,
        state.music.karaoke.stage_focus,
        is_full,
        state.music.karaoke.show_compact_setlist,
    );

    if dims.show_stage && dims.show_setlist {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(36), Constraint::Length(dims.setlist_w)])
            .split(area);
        render_karaoke_stage(frame, cols[0], state, modal_rect, dims);
        render_karaoke_setlist(frame, cols[1], state, modal_rect);
    } else if dims.show_stage {
        render_karaoke_stage(frame, area, state, modal_rect, dims);
        if is_full && state.music.karaoke.show_compact_setlist && modal_rect.is_none() && area.width >= 56 && area.height >= 14 {
            let qw = ((area.width as u32 * 32) / 100).clamp(32, 42) as u16;
            let qh = area.height.saturating_sub(5);
            let q_rect = Rect::new(area.x + area.width.saturating_sub(qw + 1), area.y + 1, qw, qh);
            frame.render_widget(Clear, q_rect);
            render_karaoke_setlist(frame, q_rect, state, modal_rect);
        }
    } else if dims.show_setlist {
        render_karaoke_setlist(frame, area, state, modal_rect);
    }

    if modal_rect.is_none() && state.music.karaoke.version_picker.is_some() {
        render_karaoke_version_picker(frame, area, state);
    }
}

fn active_or_selected_entry(state: &AppState) -> Option<&KaraokePlaylistEntry> {
    let k = &state.music.karaoke;
    if let Some(aid) = k.active_item_id {
        if let Some(found) = k.items.iter().find(|e| e.id == aid) {
            return Some(found);
        }
    }
    if let Some(vid) = state.active_playback_video_id.as_deref() {
        if let Some(found) = k.items.iter().find(|e| e.video_id == vid) {
            return Some(found);
        }
    }
    k.items.get(k.cursor)
}

fn push_stage_pill(
    hits: &mut Vec<(Rect, KaraokeHit)>,
    spans: &mut Vec<Span<'static>>,
    cursor_x: &mut u16,
    bounds: (u16, u16), // (y, max_x)
    label: &str,
    hit: KaraokeHit,
    style_colors: (Color, Color),
) {
    let (y, max_x) = bounds;
    let (fg, bg) = style_colors;
    let w = label.chars().count() as u16;
    if *cursor_x + w > max_x {
        return;
    }
    hits.push((Rect::new(*cursor_x, y, w, 1), hit));
    spans.push(Span::styled(
        label.to_string(),
        Style::default()
            .fg(fg)
            .bg(bg)
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::raw(" "));
    *cursor_x += w + 1;
}

fn render_karaoke_stage(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
    dims: KaraokeLayoutDims,
) {
    state.music.karaoke.stage_rect = area;
    let theme = *state.base_theme();
    let spotify_green = Color::Rgb(30, 215, 96);

    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if state.music.karaoke.session_active {
            spotify_green
        } else {
            theme.primary
        }))
        .style(Style::default().bg(Color::Reset));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 20 || inner.height < 8 {
        return;
    }

    let corr_w = dims.corridor_w.min(inner.width).max(20);
    let corr_x = inner.x + inner.width.saturating_sub(corr_w) / 2;
    let corr = Rect::new(corr_x, inner.y, corr_w, inner.height);

    let is_full = state.music.karaoke.fullscreen;
    let show_hero = !is_full && corr.height >= 16;
    let footer_h: u16 = if is_full {
        if corr.height >= 14 { 3 } else { 2 }
    } else if corr.height >= 12 {
        2
    } else {
        1
    };
    let hero_h: u16 = if show_hero { 3 } else { 0 };
    let tap_h: u16 = if state.music.karaoke.tap_sync_active { 1 } else { 0 };

    let parts = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),        // Top bar
            Constraint::Length(hero_h),   // Thumbnail + song/mic telemetry
            Constraint::Length(1),        // Separator
            Constraint::Length(tap_h),    // Tap-to-Sync strip when active
            Constraint::Min(4),           // Lyric theatre
            Constraint::Length(1),        // Separator
            Constraint::Length(footer_h), // Controls & always-visible keybind bar
        ])
        .split(corr);

    // 1. Top Stage Header Bar
    let top = parts[0];
    let compact_hdr = top.width < 68;
    let mode_lbl = match (compact_hdr, state.music.karaoke.mode) {
        (true, KaraokeMode::Practice) => " [Practice] ",
        (true, KaraokeMode::Party) => " [Party] ",
        (false, KaraokeMode::Practice) => " [Mode: Practice] ",
        (false, KaraokeMode::Party) => " [Mode: Party] ",
    };
    let focus_lbl = if state.music.karaoke.stage_focus {
        " [Split] "
    } else if compact_hdr {
        " [Focus] "
    } else {
        " [Focus (f)] "
    };
    let full_lbl = if is_full {
        " [Exit Full (F)] "
    } else if compact_hdr {
        " [Full] "
    } else {
        " [Full (F)] "
    };
    let setlist_lbl = if is_full { " [Queue] " } else { " [Setlist] " };
    let mode_w = mode_lbl.chars().count() as u16;
    let full_w = full_lbl.chars().count() as u16;
    let focus_w = if is_full {
        0
    } else {
        focus_lbl.chars().count() as u16 + 1
    };
    let set_w = if dims.compact_only_one_pane {
        setlist_lbl.chars().count() as u16 + 1
    } else {
        0
    };
    let right_total = mode_w + 1 + full_w + focus_w + set_w;
    let left_w = top.width.saturating_sub(right_total + 1) as usize;
    let pl_name = state
        .music
        .karaoke
        .playlists
        .iter()
        .find(|p| p.id == state.music.karaoke.active_playlist_id)
        .map(|p| p.name.as_str())
        .unwrap_or("Stage Queue");

    let left_header_text = if is_full {
        let entry = active_or_selected_entry(state);
        let track_tag = state
            .music
            .current_track()
            .map(|t| {
                if t.artist.is_empty() {
                    t.title.clone()
                } else {
                    format!("{} — {}", t.title, t.artist)
                }
            })
            .or_else(|| entry.map(|e| format!("{} — {}", e.title, e.artist)))
            .unwrap_or_else(|| pl_name.to_string());
        let mic_tag = entry
            .and_then(|e| e.singer_name.as_deref())
            .map(|s| format!(" · Mic:{s}"))
            .unwrap_or_default();
        let swp_tag = if state.music.karaoke.sweep_speed_pct != 100 {
            format!(" · Swp:{:.1}x", state.music.karaoke.sweep_speed_pct as f64 / 100.0)
        } else {
            String::new()
        };
        format!("◈ FULL KARAOKE · {track_tag}{mic_tag}{swp_tag}")
    } else {
        format!("◈ KARAOKE · {pl_name}")
    };

    let title_str = truncate_str(&left_header_text, left_w.max(8));
    let mut top_spans = vec![Span::styled(
        format!("{title_str:<w$}", w = left_w.max(8)),
        Style::default()
            .fg(spotify_green)
            .add_modifier(Modifier::BOLD),
    )];
    let mut rx = top.x + top.width.saturating_sub(right_total);
    top_spans.push(Span::raw(" "));
    let mode_col = if state.music.karaoke.mode == KaraokeMode::Party {
        Color::Rgb(245, 190, 75)
    } else {
        theme.accent
    };
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut top_spans,
        &mut rx,
        (top.y, top.x + top.width),
        mode_lbl,
        KaraokeHit::ToggleMode,
        (Color::Black, mode_col),
    );
    if !is_full {
        push_stage_pill(
            &mut state.music.karaoke.hitboxes,
            &mut top_spans,
            &mut rx,
            (top.y, top.x + top.width),
            focus_lbl,
            KaraokeHit::ToggleFocus,
            (Color::White, Color::Rgb(46, 52, 68)),
        );
    }
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut top_spans,
        &mut rx,
        (top.y, top.x + top.width),
        full_lbl,
        KaraokeHit::ToggleFullscreen,
        if is_full {
            (Color::Black, Color::Rgb(245, 190, 75))
        } else {
            (Color::Black, theme.accent)
        },
    );
    if dims.compact_only_one_pane {
        push_stage_pill(
            &mut state.music.karaoke.hitboxes,
            &mut top_spans,
            &mut rx,
            (top.y, top.x + top.width),
            setlist_lbl,
            KaraokeHit::ToggleCompactSetlist,
            (Color::Black, spotify_green),
        );
    }
    frame.render_widget(Paragraph::new(Line::from(top_spans)), top);

    // 2. Song Anchor Card (6x3 cover + Singer / Key / Vocal / Sync metadata)
    let rect_intersects_any = |r: Rect| {
        modal_rect.is_some_and(|m| {
            r.x < m.x + m.width && m.x < r.x + r.width && r.y < m.y + m.height && m.y < r.y + r.height
        })
    };

    if show_hero {
        let hero = parts[1];
        let cov_w: u16 = if hero.width >= 30 { 7 } else { 0 };
        let (cov_rect, info_r) = if cov_w > 0 {
            let sp = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(cov_w), Constraint::Min(12)])
                .split(hero);
            (Some(Rect::new(sp[0].x, sp[0].y, sp[0].width.saturating_sub(1), sp[0].height)), sp[1])
        } else {
            (None, hero)
        };

        let cur_tr = state.music.current_track().cloned();
        let entry_opt = active_or_selected_entry(state).cloned();
        let (title_s, artist_s, singer_s, thumb_key, thumb_url) = if let Some(ref tr) = cur_tr {
            let singer = entry_opt
                .as_ref()
                .filter(|e| e.video_id == tr.video_id)
                .and_then(|e| e.singer_name.clone());
            (
                tr.title.clone(),
                tr.artist.clone(),
                singer,
                tr.thumb_cache_key(),
                tr.effective_thumb_url(),
            )
        } else if let Some(ref e) = entry_opt {
            (
                e.title.clone(),
                e.artist.clone(),
                e.singer_name.clone(),
                format!("ytm:{}", e.video_id),
                e.thumb_url.clone(),
            )
        } else {
            (
                "Ready for Karaoke".to_string(),
                "Send songs with [K] or search in Setlist".to_string(),
                None,
                String::new(),
                String::new(),
            )
        };

        if let Some(cr) = cov_rect {
            if !thumb_url.is_empty() && !thumb_key.is_empty() {
                state
                    .queue_thumb_with_priority(&thumb_key, &thumb_url, ThumbPriority::Focused);
            }
            if !thumb_key.is_empty() && !rect_intersects_any(cr) {
                render_thumbnail_cover(frame, cr, &thumb_key, state, "◈");
            } else {
                frame.render_widget(
                    Paragraph::new("\n◈")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(spotify_green).bg(Color::Reset)),
                    cr,
                );
            }
        }

        let iw = info_r.width as usize;
        let line1 = if artist_s.is_empty() {
            title_s
        } else {
            format!("{title_s} — {artist_s}")
        };
        let key_s = match state.music.karaoke.pitch_semitones {
            0 => "Key: 0st".to_string(),
            p if p > 0 => format!("Key: +{p}st"),
            p => format!("Key: {p}st"),
        };
        let off_ms = state.music.karaoke.lyric_offset_ms;
        let off_s = if off_ms == 0 {
            "Sync: 0ms".to_string()
        } else if off_ms > 0 {
            format!("Sync: +{off_ms}ms")
        } else {
            format!("Sync: {off_ms}ms")
        };
        let mic_s = singer_s
            .map(|s| format!("Mic: {s}"))
            .unwrap_or_else(|| format!("Mode: {}", state.music.karaoke.mode.label()));
        let spd_s = if state.music.karaoke.speed_pct == 100 {
            String::new()
        } else {
            format!(" · Spd: {:.2}x", state.music.karaoke.speed_pct as f64 / 100.0)
        };
        let line2 = format!(
            "{mic_s} · {key_s} · Vocal: {}{spd_s} · {off_s}",
            state.music.karaoke.vocal_mode.label()
        );
        let src_s = state
            .music
            .lyrics
            .as_ref()
            .and_then(|l| l.source.as_deref())
            .unwrap_or("Pre-fetched SQLite + LRCLIB Karaoke Engine");
        let ab_s = match (
            state.music.karaoke.loop_start_ms,
            state.music.karaoke.loop_end_ms,
        ) {
            (Some(a), Some(b)) => format!(" · Loop {:.1}s-{:.1}s", a as f64 / 1000.0, b as f64 / 1000.0),
            (Some(a), None) => format!(" · Loop A={:.1}s (press b)", a as f64 / 1000.0),
            _ => String::new(),
        };
        let line3 = format!("{src_s}{ab_s}");

        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    truncate_words_clean(&line1, iw),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(
                    truncate_words_clean(&line2, iw),
                    Style::default().fg(theme.accent),
                )),
                Line::from(Span::styled(
                    truncate_words_clean(&line3, iw),
                    Style::default().fg(theme.text_dim),
                )),
            ]),
            info_r,
        );
    }

    // Separators
    let sep = "─".repeat(corr.width as usize);
    frame.render_widget(
        Paragraph::new(sep.as_str()).style(Style::default().fg(theme.border_idle)),
        parts[2],
    );
    frame.render_widget(
        Paragraph::new(sep.as_str()).style(Style::default().fg(theme.border_idle)),
        parts[5],
    );

    // 3. Tap-to-Sync interactive prompt bar when active
    if state.music.karaoke.tap_sync_active {
        let tr = parts[3];
        let stamped = state.music.karaoke.tap_sync_stamps.len();
        let total = state
            .music
            .lyrics
            .as_ref()
            .map(|l| l.lines.len())
            .unwrap_or(0);
        let msg = format!("TAP-SYNC · Stamped {stamped}/{total} · [t] Stamp Line ");
        let mut tspans = vec![Span::styled(
            msg,
            Style::default()
                .fg(Color::Rgb(255, 200, 90))
                .add_modifier(Modifier::BOLD),
        )];
        let mut bx = tr.x + tr.width.min(48);
        push_stage_pill(
            &mut state.music.karaoke.hitboxes,
            &mut tspans,
            &mut bx,
            (tr.y, tr.x + tr.width),
            " [Stamp (t)] ",
            KaraokeHit::StampTapSync,
            (Color::Black, spotify_green),
        );
        push_stage_pill(
            &mut state.music.karaoke.hitboxes,
            &mut tspans,
            &mut bx,
            (tr.y, tr.x + tr.width),
            " [Save] ",
            KaraokeHit::CommitTapSync,
            (Color::Black, theme.accent),
        );
        push_stage_pill(
            &mut state.music.karaoke.hitboxes,
            &mut tspans,
            &mut bx,
            (tr.y, tr.x + tr.width),
            " [Cancel] ",
            KaraokeHit::CancelTapSync,
            (Color::White, Color::Rgb(70, 48, 52)),
        );
        frame.render_widget(Paragraph::new(Line::from(tspans)), tr);
    }

    // 4. Center Lyric Theatre
    let theatre = parts[4];
    render_karaoke_theatre(frame, theatre, state, spotify_green, &theme);

    // 5. Conditional Stage Controls Footer
    render_karaoke_stage_footer(frame, parts[6], state, spotify_green, &theme);

    // 6. Optional 5-second Party Intermission Overlay centered inside Lyric Theatre
    if state.music.karaoke.intermission_deadline.is_some() && modal_rect.is_none() {
        render_karaoke_intermission_card(frame, theatre, state, spotify_green, &theme);
    }
}

fn render_karaoke_theatre(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    spotify_green: Color,
    theme: &crate::ui::theme::ThemePalette,
) {
    if area.height < 3 || area.width < 12 {
        return;
    }

    if state.music.lyrics_loading {
        frame.render_widget(
            Paragraph::new("\n◈ Loading synced karaoke lyrics…")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            area,
        );
        return;
    }

    let Some(lyrics) = state.music.lyrics.clone() else {
        let has_items = !state.music.karaoke.items.is_empty();
        let top_y = area.y + area.height.saturating_sub(5) / 2;
        if has_items {
            let first = &state.music.karaoke.items[0];
            let hint = format!(
                "Queue Ready: {} track(s) · First up: {}",
                state.music.karaoke.items.len(),
                truncate_words_clean(&first.title, area.width.saturating_sub(36) as usize)
            );
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        "KARAOKE STAGE READY",
                        Style::default()
                            .fg(spotify_green)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(hint, Style::default().fg(Color::White))),
                    Line::from(Span::styled(
                        "Press [Enter] or click a track in the Setlist to start singing",
                        Style::default().fg(theme.text_dim),
                    )),
                ])
                .alignment(Alignment::Center),
                Rect::new(area.x, top_y, area.width, 3.min(area.height)),
            );
            let pill = " [▶ Start Track 1] ";
            let pw = pill.chars().count() as u16;
            if top_y + 4 < area.y + area.height && area.width >= pw {
                let pr = Rect::new(area.x + (area.width - pw) / 2, top_y + 4, pw, 1);
                state
                    .music
                    .karaoke
                    .hitboxes
                    .push((pr, KaraokeHit::PlayItem(0)));
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        pill,
                        Style::default()
                            .fg(Color::Black)
                            .bg(spotify_green)
                            .add_modifier(Modifier::BOLD),
                    )),
                    pr,
                );
            }
        } else {
            frame.render_widget(
                Paragraph::new(vec![
                    Line::from(Span::styled(
                        "BUILD YOUR KARAOKE SETLIST",
                        Style::default()
                            .fg(spotify_green)
                            .add_modifier(Modifier::BOLD),
                    )),
                    Line::from(Span::styled(
                        "Press [K] on any song in Home / Search / Albums to send it here,",
                        Style::default().fg(Color::White),
                    )),
                    Line::from(Span::styled(
                        "or press [/] to search and add songs right into this playlist.",
                        Style::default().fg(theme.text_dim),
                    )),
                ])
                .alignment(Alignment::Center),
                Rect::new(area.x, top_y, area.width, 3.min(area.height)),
            );
            let pill = " [⌕ Add First Song (/)] ";
            let pw = pill.chars().count() as u16;
            if top_y + 4 < area.y + area.height && area.width >= pw {
                let pr = Rect::new(area.x + (area.width - pw) / 2, top_y + 4, pw, 1);
                state
                    .music
                    .karaoke
                    .hitboxes
                    .push((pr, KaraokeHit::FocusInlineSearch));
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        pill,
                        Style::default()
                            .fg(Color::Black)
                            .bg(theme.accent)
                            .add_modifier(Modifier::BOLD),
                    )),
                    pr,
                );
            }
        }
        return;
    };

    if lyrics.lines.is_empty() {
        frame.render_widget(
            Paragraph::new("\nInstrumental / No lyrics returned — click [Ver] in Setlist to try another match.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            area,
        );
        return;
    }

    let status_rect = Rect::new(area.x, area.y, area.width, 1);
    let mut top_spans = Vec::new();
    let badge = if state.music.karaoke.tap_sync_active {
        "TAP-SYNC"
    } else if lyrics.has_word_timings() {
        "LIVE KARAOKE · Word-Synced"
    } else if lyrics.is_timed {
        "LIVE KARAOKE · Line-Synced"
    } else {
        "LYRICS · Unsynced (press [t] in Practice to Tap-Sync)"
    };
    top_spans.push(Span::styled(
        badge,
        Style::default()
            .fg(if lyrics.is_timed { spotify_green } else { theme.accent })
            .add_modifier(Modifier::BOLD),
    ));
    if state.music.karaoke.lyrics_manual_scroll {
        let fl = " [Follow Live] ";
        let fw = fl.chars().count() as u16;
        if area.width > badge.chars().count() as u16 + fw + 2 {
            let fx = area.x + area.width - fw;
            state
                .music
                .karaoke
                .hitboxes
                .push((Rect::new(fx, area.y, fw, 1), KaraokeHit::ResumeLiveLyrics));
            top_spans.push(Span::raw(" ".repeat(
                area.width.saturating_sub(badge.chars().count() as u16 + fw) as usize,
            )));
            top_spans.push(Span::styled(
                fl,
                Style::default()
                    .fg(Color::Black)
                    .bg(spotify_green)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(top_spans)), status_rect);

    let body = Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1));
    let view_h = body.height as usize;
    if view_h == 0 {
        return;
    }

    let effective_pos = state.effective_playback_pos_secs();
    let pos_ms = state.music.effective_lyric_pos_ms(effective_pos);
    let active_idx = state.music.active_lyric_index(effective_pos);
    let sweep_speed_pct = state.music.karaoke.sweep_speed_pct;
    let wide_breathing = view_h >= 12 && !state.music.karaoke.tap_sync_active;

    struct StageVisualRow {
        spans: Vec<Span<'static>>,
        seek_ms: Option<u64>,
        is_focus: bool,
    }

    let text_w = (body.width.saturating_sub(4) as usize).clamp(
        12,
        if state.music.karaoke.fullscreen { 108 } else { 80 },
    );
    let tap_cursor = state.music.karaoke.tap_sync_stamps.len();
    let mut rows: Vec<StageVisualRow> = Vec::new();

    if !state.music.karaoke.tap_sync_active
        && lyrics.is_timed
        && lyrics
            .lines
            .first()
            .and_then(|l| l.start_ms)
            .is_some_and(|s| s > 1500 && pos_ms < s)
    {
        let first_s = lyrics.lines.first().and_then(|l| l.start_ms).unwrap_or(0);
        let rem_secs = (first_s.saturating_sub(pos_ms) as f64) / 1000.0;
        let pulse = match ((pos_ms / 350) % 4) as usize {
            0 => "● ○ ○ ○",
            1 => "● ● ○ ○",
            2 => "● ● ● ○",
            _ => "◈ ● ● ●",
        };
        rows.push(StageVisualRow {
            spans: vec![
                Span::styled(
                    format!("▸  {pulse}  Vocals in {rem_secs:.1}s  ◂"),
                    Style::default()
                        .fg(spotify_green)
                        .add_modifier(Modifier::BOLD),
                ),
            ],
            seek_ms: Some(first_s),
            is_focus: true,
        });
        if wide_breathing {
            rows.push(StageVisualRow {
                spans: Vec::new(),
                seek_ms: None,
                is_focus: false,
            });
        }
    }

    for (idx, line) in lyrics.lines.iter().enumerate() {
        let is_active = if state.music.karaoke.tap_sync_active {
            idx == tap_cursor
        } else {
            active_idx == Some(idx)
        };
        let is_past = if state.music.karaoke.tap_sync_active {
            idx < tap_cursor
        } else {
            active_idx.is_some_and(|a| idx < a)
        };
        let is_next = if state.music.karaoke.tap_sync_active {
            idx == tap_cursor + 1
        } else {
            active_idx.is_some_and(|a| idx == a + 1 || idx == a + 2)
        };
        let next_start = lyrics.lines.get(idx + 1).and_then(|n| n.start_ms);

        let in_post_interlude = !state.music.karaoke.tap_sync_active
            && is_active
            && line.end_ms.is_some_and(|e| {
                next_start.is_some_and(|ns| ns > e + 2800) && pos_ms > e + 800
            });

        if line.text.trim().is_empty() {
            rows.push(StageVisualRow {
                spans: vec![Span::styled("◈ • • •", Style::default().fg(theme.text_dim))],
                seek_ms: line.start_ms,
                is_focus: is_active && !in_post_interlude,
            });
            continue;
        }

        let chunks = wrap_lyric_line_with_offsets(&line.text, text_w);
        let total_chars = line.text.chars().count();
        let (sung_chars, sweep_frac) = if state.music.karaoke.tap_sync_active {
            if is_past { (total_chars, 1.0) } else { (0, 0.0) }
        } else if is_active {
            compute_lyric_sung_chars_and_frac(line, pos_ms, next_start, sweep_speed_pct)
        } else {
            (0, 0.0)
        };

        let spotlight_cushion = view_h >= 14 && is_active && !in_post_interlude && !state.music.karaoke.tap_sync_active;
        if spotlight_cushion {
            rows.push(StageVisualRow {
                spans: Vec::new(),
                seek_ms: None,
                is_focus: false,
            });
        }

        for (ci, (cs, _ce, chunk)) in chunks.into_iter().enumerate() {
            let mut spans = Vec::new();
            let focus_line = is_active && !in_post_interlude;
            if focus_line && ci == 0 {
                spans.push(Span::styled(
                    "▸ ",
                    Style::default()
                        .fg(spotify_green)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            if state.music.karaoke.tap_sync_active {
                let col = if is_active {
                    Color::Rgb(255, 210, 90)
                } else if is_past {
                    spotify_green
                } else {
                    Color::Rgb(190, 200, 215)
                };
                let mut st = Style::default().fg(col);
                if is_active {
                    st = st.add_modifier(Modifier::BOLD);
                }
                spans.push(Span::styled(chunk, st));
            } else if is_active {
                let (sung, tip, unsung) =
                    karaoke_chunk_parts(&chunk, cs, total_chars, sung_chars);
                if !sung.is_empty() {
                    spans.push(Span::styled(
                        sung,
                        Style::default()
                            .fg(spotify_green)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
                if !tip.is_empty() {
                    spans.push(Span::styled(
                        tip,
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
                    ));
                }
                if !unsung.is_empty() {
                    spans.push(Span::styled(
                        unsung,
                        Style::default()
                            .fg(Color::Rgb(215, 222, 235))
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            } else if is_past {
                spans.push(Span::styled(chunk, Style::default().fg(theme.text_dim)));
            } else if is_next {
                spans.push(Span::styled(
                    chunk,
                    Style::default().fg(Color::Rgb(205, 214, 226)),
                ));
            } else {
                spans.push(Span::styled(
                    chunk,
                    Style::default().fg(Color::Rgb(115, 125, 140)),
                ));
            }

            if focus_line && ci == 0 {
                spans.push(Span::styled(
                    " ◂",
                    Style::default()
                        .fg(spotify_green)
                        .add_modifier(Modifier::BOLD),
                ));
            }

            rows.push(StageVisualRow {
                spans,
                seek_ms: line.start_ms,
                is_focus: focus_line && ci == 0,
            });
        }

        if spotlight_cushion && lyrics.is_timed && body.width >= 20 {
            let beam_w = total_chars.clamp(18, text_w.min(54));
            let filled = ((beam_w.saturating_sub(1)) as f64 * sweep_frac.clamp(0.0, 1.0))
                .round() as usize;
            let rem = beam_w.saturating_sub(filled + 1);
            rows.push(StageVisualRow {
                spans: vec![
                    Span::styled("━".repeat(filled), Style::default().fg(spotify_green)),
                    Span::styled("●", Style::default().fg(Color::White)),
                    Span::styled("─".repeat(rem), Style::default().fg(Color::Rgb(65, 72, 88))),
                ],
                seek_ms: line.start_ms,
                is_focus: false,
            });
            rows.push(StageVisualRow {
                spans: Vec::new(),
                seek_ms: None,
                is_focus: false,
            });
        }

        if in_post_interlude {
            let rem_s = next_start
                .map(|ns| (ns.saturating_sub(pos_ms) as f64) / 1000.0)
                .unwrap_or(0.0);
            rows.push(StageVisualRow {
                spans: vec![Span::styled(
                    format!("◈ • • • Next verse in {rem_s:.1}s • • •"),
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                )],
                seek_ms: next_start,
                is_focus: true,
            });
        }

        if wide_breathing {
            rows.push(StageVisualRow {
                spans: Vec::new(),
                seek_ms: None,
                is_focus: false,
            });
        }
    }

    let total_vis = rows.len();
    let max_scroll = total_vis.saturating_sub(view_h);
    state.music.karaoke.lyrics_max_scroll = max_scroll;
    if !state.music.karaoke.lyrics_manual_scroll {
        let focus_row = rows.iter().position(|r| r.is_focus);
        state.music.karaoke.lyrics_scroll = if let Some(f) = focus_row {
            if total_vis > view_h {
                f.saturating_sub((view_h * 2) / 5).min(max_scroll)
            } else {
                0
            }
        } else {
            0
        };
    } else {
        state.music.karaoke.lyrics_scroll = state.music.karaoke.lyrics_scroll.min(max_scroll);
    }

    let has_sb = total_vis > view_h && view_h >= 2 && body.width >= 12;
    let draw_w = if has_sb { body.width - 1 } else { body.width };
    for (vi, vr) in rows
        .into_iter()
        .skip(state.music.karaoke.lyrics_scroll)
        .take(view_h)
        .enumerate()
    {
        let r = Rect::new(body.x, body.y + vi as u16, draw_w, 1);
        if let Some(ms) = vr.seek_ms {
            state
                .music
                .karaoke
                .hitboxes
                .push((r, KaraokeHit::SeekLyricMs(ms)));
        }
        frame.render_widget(
            Paragraph::new(Line::from(vr.spans))
                .alignment(Alignment::Center)
                .style(Style::default().bg(Color::Reset)),
            r,
        );
    }
    if has_sb {
        render_vertical_scrollbar(
            frame,
            Rect::new(body.x + body.width - 1, body.y, 1, body.height),
            state.music.karaoke.lyrics_scroll,
            view_h,
            total_vis,
            theme.border_idle,
            spotify_green,
        );
    }
}

fn render_karaoke_stage_footer(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    spotify_green: Color,
    theme: &crate::ui::theme::ThemePalette,
) {
    if area.height == 0 || area.width < 12 {
        return;
    }

    let is_full = state.music.karaoke.fullscreen;
    let mut row_idx = 0u16;

    // In Fullscreen mode, reserve top footer row for compact transport + clickable Seek Bar
    if is_full && area.height >= 3 {
        let tr_row = Rect::new(area.x, area.y, area.width, 1);
        let cur_playing = state.music.current_track().is_some();
        let play_lbl = if cur_playing && !state.live_playback.paused && state.has_live_ipc() {
            " || "
        } else {
            " ▶ "
        };
        let mut bx = tr_row.x;
        let max_x = tr_row.x + tr_row.width;
        let dark_pill = Color::Rgb(42, 48, 62);
        let mut t_spans = Vec::new();
        for (lbl, mhit, fg, bg) in [
            (" |< ", MusicHit::PrevTrack, Color::White, dark_pill),
            (play_lbl, MusicHit::TogglePlayPause, Color::Black, Color::White),
            (" >| ", MusicHit::NextTrack, Color::White, dark_pill),
        ] {
            let w = lbl.chars().count() as u16;
            if bx + w <= max_x {
                state
                    .music
                    .hitboxes
                    .push((Rect::new(bx, tr_row.y, w, 1), mhit));
                t_spans.push(Span::styled(
                    lbl,
                    Style::default()
                        .fg(fg)
                        .bg(bg)
                        .add_modifier(Modifier::BOLD),
                ));
                t_spans.push(Span::raw(" "));
                bx += w + 1;
            }
        }
        let dur_secs = if state.active_playback_dur > 0.0 {
            state.active_playback_dur
        } else {
            state
                .music
                .current_track()
                .and_then(|t| t.duration_secs)
                .map(|d| d as f64)
                .unwrap_or(0.0)
        };
        let pos_secs = if cur_playing {
            state.effective_playback_pos_secs().clamp(0.0, dur_secs.max(state.active_playback_pos))
        } else {
            0.0
        };
        let t_left = format_duration_secs(pos_secs as u64);
        let t_right = format_duration_secs(dur_secs as u64);
        let rem_w = max_x.saturating_sub(bx) as usize;
        let bar_space = rem_w.saturating_sub(t_left.len() + t_right.len() + 3);
        if bar_space >= 8 {
            let ratio = if dur_secs > 0.0 {
                (pos_secs / dur_secs).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let filled = ((bar_space - 1) as f64 * ratio).round() as usize;
            let rem = bar_space.saturating_sub(filled + 1);
            let seek_rect = Rect::new(bx + t_left.len() as u16 + 1, tr_row.y, bar_space as u16, 1);
            state.music.progress_bar_rect = seek_rect;
            state.music.hitboxes.push((seek_rect, MusicHit::SeekBar));
            t_spans.push(Span::styled(format!("{t_left} "), Style::default().fg(theme.text_dim)));
            t_spans.push(Span::styled("━".repeat(filled), Style::default().fg(spotify_green)));
            t_spans.push(Span::styled("●", Style::default().fg(Color::White)));
            t_spans.push(Span::styled("─".repeat(rem), Style::default().fg(Color::Rgb(70, 76, 92))));
            t_spans.push(Span::styled(format!(" {t_right}"), Style::default().fg(theme.text_dim)));
        }
        frame.render_widget(Paragraph::new(Line::from(t_spans)), tr_row);
        row_idx += 1;
    }

    let r0 = Rect::new(area.x, area.y + row_idx, area.width, 1);
    let mut spans = Vec::new();
    let mut bx = r0.x;
    let max_x = r0.x + r0.width;
    let dark_pill = Color::Rgb(42, 48, 62);
    let swp_col = if state.music.karaoke.sweep_speed_pct != 100 {
        spotify_green
    } else {
        theme.accent
    };
    let swp_lbl = format!(
        "{:.1}x",
        state.music.karaoke.sweep_speed_pct as f64 / 100.0
    );

    match state.music.karaoke.mode {
        KaraokeMode::Practice => {
            let v_lbl = format!(" [Voc: {}] ", state.music.karaoke.vocal_mode.label());
            let v_bg = if state.music.karaoke.vocal_mode == crate::state::KaraokeVocalMode::Original {
                dark_pill
            } else {
                spotify_green
            };
            let v_fg = if state.music.karaoke.vocal_mode == crate::state::KaraokeVocalMode::Original {
                Color::White
            } else {
                Color::Black
            };
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                &v_lbl,
                KaraokeHit::CycleVocalMode,
                (v_fg, v_bg),
            );

            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                " Swp- ",
                KaraokeHit::SweepSpeedDelta(-10),
                (Color::White, dark_pill),
            );
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                &swp_lbl,
                KaraokeHit::ResetSweepSpeed,
                (swp_col, Color::Reset),
            );
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                "+ ",
                KaraokeHit::SweepSpeedDelta(10),
                (Color::White, dark_pill),
            );

            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                " Key- ",
                KaraokeHit::PitchDelta(-1),
                (Color::White, dark_pill),
            );
            let k_val = format!("{:+}st ", state.music.karaoke.pitch_semitones);
            let k_w = k_val.chars().count() as u16;
            if bx + k_w <= max_x {
                spans.push(Span::styled(
                    k_val,
                    Style::default()
                        .fg(if state.music.karaoke.pitch_semitones != 0 {
                            spotify_green
                        } else {
                            theme.text_dim
                        })
                        .add_modifier(Modifier::BOLD),
                ));
                bx += k_w;
            }
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                "+ ",
                KaraokeHit::PitchDelta(1),
                (Color::White, dark_pill),
            );

            let spd_lbl = format!(" {:.2}x ", state.music.karaoke.speed_pct as f64 / 100.0);
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                &spd_lbl,
                KaraokeHit::SpeedDelta(-5),
                (theme.accent, dark_pill),
            );

            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                " Sync- ",
                KaraokeHit::OffsetDelta(-100),
                (Color::White, dark_pill),
            );
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                "+ ",
                KaraokeHit::OffsetDelta(100),
                (Color::White, dark_pill),
            );

            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                if state.music.karaoke.loop_start_ms.is_some() {
                    " [A*] "
                } else {
                    " [A] "
                },
                KaraokeHit::SetLoopA,
                (
                    Color::Black,
                    if state.music.karaoke.loop_start_ms.is_some() {
                        spotify_green
                    } else {
                        theme.accent
                    },
                ),
            );
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                if state.music.karaoke.loop_end_ms.is_some() {
                    " [B*] "
                } else {
                    " [B] "
                },
                KaraokeHit::SetLoopB,
                (
                    Color::Black,
                    if state.music.karaoke.loop_end_ms.is_some() {
                        spotify_green
                    } else {
                        theme.accent
                    },
                ),
            );
            if state.music.karaoke.loop_start_ms.is_some()
                || state.music.karaoke.loop_end_ms.is_some()
            {
                push_stage_pill(
                    &mut state.music.karaoke.hitboxes,
                    &mut spans,
                    &mut bx,
                    (r0.y, max_x),
                    " [Clr] ",
                    KaraokeHit::ClearLoop,
                    (Color::White, Color::Rgb(75, 48, 55)),
                );
            }
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                if state.music.karaoke.tap_sync_active {
                    " [Tap ON] "
                } else {
                    " [Tap-Sync] "
                },
                KaraokeHit::ToggleTapSync,
                (
                    Color::Black,
                    if state.music.karaoke.tap_sync_active {
                        Color::Rgb(255, 200, 90)
                    } else {
                        Color::Rgb(165, 180, 210)
                    },
                ),
            );
        }
        KaraokeMode::Party => {
            let v_lbl = format!(" [Vocal: {}] ", state.music.karaoke.vocal_mode.label());
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                &v_lbl,
                KaraokeHit::CycleVocalMode,
                (Color::Black, spotify_green),
            );
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                " Swp- ",
                KaraokeHit::SweepSpeedDelta(-10),
                (Color::White, dark_pill),
            );
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                &swp_lbl,
                KaraokeHit::ResetSweepSpeed,
                (swp_col, Color::Reset),
            );
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                "+ ",
                KaraokeHit::SweepSpeedDelta(10),
                (Color::White, dark_pill),
            );
            let k_lbl = format!(" Key {:+}st ", state.music.karaoke.pitch_semitones);
            push_stage_pill(
                &mut state.music.karaoke.hitboxes,
                &mut spans,
                &mut bx,
                (r0.y, max_x),
                &k_lbl,
                KaraokeHit::PitchDelta(1),
                (Color::White, dark_pill),
            );

            let next_str = next_karaoke_index(&state.music.karaoke)
                .and_then(|ni| state.music.karaoke.items.get(ni))
                .map(|e| {
                    let singer = e
                        .singer_name
                        .as_deref()
                        .map(|s| format!(" · Mic:{s}"))
                        .unwrap_or_default();
                    format!("Up Next: {} — {}{singer}", e.title, e.artist)
                })
                .unwrap_or_else(|| "Party Host Ready · Queue songs with [K] or [/]".to_string());
            let rem_w = max_x.saturating_sub(bx) as usize;
            if rem_w >= 8 {
                spans.push(Span::styled(
                    truncate_words_clean(&next_str, rem_w),
                    Style::default().fg(theme.text_dim),
                ));
            }
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), r0);
    row_idx += 1;

    // Always-visible Keybind Reminder Bar at the bottom!
    if row_idx < area.height {
        let r_hint = Rect::new(area.x, area.y + row_idx, area.width, 1);
        let hint = match (is_full, state.music.karaoke.mode) {
            (true, KaraokeMode::Practice) => fit_joined_parts(
                &[
                    "[F/Esc] Exit Fullscreen",
                    "[w/W] Word Sweep Speed",
                    "[+/-] Key",
                    "[[ / ]] Sync Offset",
                    "[v] Vocal Cut",
                    "[9/0] Tempo",
                    "[a/b/x] Loop",
                    "[t] Tap-Sync",
                    "[Tab] Queue",
                    "[Space] Play/Pause",
                ],
                " · ",
                r_hint.width as usize,
            ),
            (true, KaraokeMode::Party) => fit_joined_parts(
                &[
                    "[F/Esc] Exit Fullscreen",
                    "[w/W] Word Sweep Speed",
                    "[+/-] Key",
                    "[v] Vocal Cut",
                    "[Tab] Queue",
                    "[>] Next (5s Handoff)",
                    "[m] Practice Mode",
                    "[Space] Play/Pause",
                ],
                " · ",
                r_hint.width as usize,
            ),
            (false, KaraokeMode::Practice) => fit_joined_parts(
                &[
                    "[F] Fullscreen",
                    "[w/W] Sweep",
                    "[+/-] Key",
                    "[9/0] Speed",
                    "[[ / ]] Sync",
                    "[v] Vocal",
                    "[a/b/x] Loop",
                    "[t] Tap-Sync",
                    "[f] Focus",
                    "[m] Party",
                ],
                " · ",
                r_hint.width as usize,
            ),
            (false, KaraokeMode::Party) => fit_joined_parts(
                &[
                    "[F] Fullscreen",
                    "[w/W] Sweep",
                    "[Space] Play/Pause",
                    "[>] Next (5s Handoff)",
                    "[/] Add Song",
                    "[m] Practice",
                    "[f] Focus",
                ],
                " · ",
                r_hint.width as usize,
            ),
        };
        frame.render_widget(
            Paragraph::new(hint).style(Style::default().fg(theme.text_dim)),
            r_hint,
        );
    }
}

fn next_karaoke_index(k: &crate::state::KaraokeStageState) -> Option<usize> {
    if k.items.is_empty() {
        return None;
    }
    if let Some(aid) = k.active_item_id {
        if let Some(pos) = k.items.iter().position(|e| e.id == aid) {
            if pos + 1 < k.items.len() {
                return Some(pos + 1);
            }
            return None;
        }
    }
    Some(0)
}

fn render_karaoke_intermission_card(
    frame: &mut Frame,
    theatre: Rect,
    state: &mut AppState,
    spotify_green: Color,
    theme: &crate::ui::theme::ThemePalette,
) {
    let Some(next_idx) = state.music.karaoke.intermission_next_idx else {
        return;
    };
    let Some(entry) = state.music.karaoke.items.get(next_idx).cloned() else {
        return;
    };
    let cw = (theatre.width.saturating_sub(6)).clamp(26, 56);
    let ch = (theatre.height.saturating_sub(2)).clamp(7, 9);
    let box_rect = Rect::new(
        theatre.x + theatre.width.saturating_sub(cw) / 2,
        theatre.y + theatre.height.saturating_sub(ch) / 2,
        cw,
        ch,
    );
    frame.render_widget(Clear, box_rect);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(spotify_green))
        .title(Span::styled(
            " ◈ UP NEXT · MIC HANDOFF ",
            Style::default()
                .fg(spotify_green)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Reset));
    let inner = blk.inner(box_rect);
    frame.render_widget(blk, box_rect);
    if inner.width < 18 || inner.height < 5 {
        return;
    }

    let rem_txt = if state.music.karaoke.intermission_paused {
        "HELD — press [p] or [Space] to start".to_string()
    } else if let Some(dl) = state.music.karaoke.intermission_deadline {
        let secs = dl.saturating_duration_since(std::time::Instant::now()).as_secs_f64();
        format!("Starting in {secs:.1}s…")
    } else {
        "Starting now…".to_string()
    };
    let singer_line = entry
        .singer_name
        .as_deref()
        .map(|s| format!("Singer: {s} · Key {:+}st", entry.pitch_semitones))
        .unwrap_or_else(|| format!("Key {:+}st · Vocal: {}", entry.pitch_semitones, state.music.karaoke.vocal_mode.label()));
    let w = inner.width as usize;

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(Span::styled(
                truncate_words_clean(&format!("{} — {}", entry.title, entry.artist), w),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                truncate_words_clean(&singer_line, w),
                Style::default().fg(theme.accent),
            )),
            Line::from(Span::styled(
                rem_txt,
                Style::default()
                    .fg(Color::Rgb(255, 200, 90))
                    .add_modifier(Modifier::BOLD),
            )),
        ])
        .alignment(Alignment::Center),
        Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(2)),
    );

    let btn_y = inner.y + inner.height.saturating_sub(1);
    let mut bspans = Vec::new();
    let mut bx = inner.x + 1;
    let max_x = inner.x + inner.width;
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut bspans,
        &mut bx,
        (btn_y, max_x),
        " [▶ Start Now (Space)] ",
        KaraokeHit::IntermissionStartNow,
        (Color::Black, spotify_green),
    );
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut bspans,
        &mut bx,
        (btn_y, max_x),
        if state.music.karaoke.intermission_paused {
            " [Resume] "
        } else {
            " [Hold (p)] "
        },
        KaraokeHit::IntermissionHoldToggle,
        (Color::Black, theme.accent),
    );
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut bspans,
        &mut bx,
        (btn_y, max_x),
        " [Skip] ",
        KaraokeHit::IntermissionSkip,
        (Color::White, Color::Rgb(50, 56, 72)),
    );
    frame.render_widget(
        Paragraph::new(Line::from(bspans)).alignment(Alignment::Center),
        Rect::new(inner.x, btn_y, inner.width, 1),
    );
}

fn render_karaoke_setlist(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    _modal_rect: Option<Rect>,
) {
    state.music.karaoke.setlist_rect = area;
    let theme = *state.base_theme();
    let spotify_green = Color::Rgb(30, 215, 96);

    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle))
        .style(Style::default().bg(Color::Reset));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 18 || inner.height < 6 {
        return;
    }

    let search_h: u16 = if state.music.karaoke.inline_search_open {
        let res_rows = state.music.karaoke.inline_search_results.len().min(5) as u16;
        2 + res_rows.max(1)
    } else {
        1
    };
    let prompt_h: u16 = if state.music.karaoke.playlist_prompt_open
        || state.music.karaoke.singer_prompt_item_id.is_some()
    {
        1
    } else {
        0
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),        // Playlist header row
            Constraint::Length(prompt_h), // Optional New Playlist / Singer prompt
            Constraint::Length(search_h), // Inline quick add search
            Constraint::Length(1),        // Divider
            Constraint::Min(3),           // Scrollable Setlist entries
            Constraint::Length(2),        // Summary + Sung History / Version button
        ])
        .split(inner);

    // 1. Playlist Switcher Row
    let pl_row = chunks[0];
    let cur_pl_name = state
        .music
        .karaoke
        .playlists
        .iter()
        .find(|p| p.id == state.music.karaoke.active_playlist_id)
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "Stage Queue".to_string());
    let has_multi_pl = state.music.karaoke.playlists.len() > 1;
    let compact_set_hdr = pl_row.width < 36;
    let new_lbl = if compact_set_hdr { " [+] " } else { " [+New] " };
    let save_lbl = if compact_set_hdr { " [Sv] " } else { " [SaveQ] " };
    let del_lbl = if compact_set_hdr { " [x] " } else { " [Del] " };
    let btn_reserve = (new_lbl.chars().count()
        + save_lbl.chars().count()
        + 2
        + if has_multi_pl {
            del_lbl.chars().count() + 1
        } else {
            0
        }) as u16;
    let pl_pill = format!(
        " ▾ {} ",
        truncate_str(&cur_pl_name, pl_row.width.saturating_sub(btn_reserve + 3) as usize)
    );
    let mut p_spans = Vec::new();
    let mut bx = pl_row.x;
    let max_x = pl_row.x + pl_row.width;
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut p_spans,
        &mut bx,
        (pl_row.y, max_x),
        &pl_pill,
        KaraokeHit::CyclePlaylistNext,
        (Color::Black, spotify_green),
    );
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut p_spans,
        &mut bx,
        (pl_row.y, max_x),
        new_lbl,
        KaraokeHit::StartNewPlaylist,
        (Color::White, Color::Rgb(46, 52, 68)),
    );
    push_stage_pill(
        &mut state.music.karaoke.hitboxes,
        &mut p_spans,
        &mut bx,
        (pl_row.y, max_x),
        save_lbl,
        KaraokeHit::SaveQueueAsPlaylist,
        (Color::Black, theme.accent),
    );
    if has_multi_pl {
        push_stage_pill(
            &mut state.music.karaoke.hitboxes,
            &mut p_spans,
            &mut bx,
            (pl_row.y, max_x),
            del_lbl,
            KaraokeHit::DeleteCurrentPlaylist,
            (Color::Rgb(255, 130, 130), Color::Rgb(46, 52, 68)),
        );
    }
    frame.render_widget(Paragraph::new(Line::from(p_spans)), pl_row);

    // 2. Prompt row (Playlist Name / Singer Tag)
    if prompt_h > 0 {
        let pr = chunks[1];
        let txt = if state.music.karaoke.playlist_prompt_open {
            format!(" New List: {}▏ [Enter]", state.music.karaoke.playlist_prompt_input)
        } else {
            format!(" Singer Mic: {}▏ [Enter]", state.music.karaoke.singer_prompt_input)
        };
        frame.render_widget(
            Paragraph::new(truncate_str(&txt, pr.width as usize)).style(
                Style::default()
                    .fg(Color::Rgb(255, 200, 90))
                    .add_modifier(Modifier::BOLD),
            ),
            pr,
        );
    }

    // 3. Inline Quick-Search Drawer
    let sr = chunks[2];
    if !state.music.karaoke.inline_search_open {
        state
            .music
            .karaoke
            .hitboxes
            .push((sr, KaraokeHit::FocusInlineSearch));
        frame.render_widget(
            Paragraph::new(truncate_str(" ⌕ [/] Search & add song here…", sr.width as usize))
                .style(Style::default().fg(theme.text_dim)),
            sr,
        );
    } else {
        let q_row = Rect::new(sr.x, sr.y, sr.width, 1);
        let close_w: u16 = 4;
        let q_w = sr.width.saturating_sub(close_w);
        state
            .music
            .karaoke
            .hitboxes
            .push((Rect::new(q_row.x, q_row.y, q_w, 1), KaraokeHit::FocusInlineSearch));
        state.music.karaoke.hitboxes.push((
            Rect::new(q_row.x + q_w, q_row.y, close_w, 1),
            KaraokeHit::ToggleInlineSearch,
        ));
        let cur_g = if state.music.karaoke.inline_search_editing {
            "▏"
        } else {
            ""
        };
        let q_disp = if state.music.karaoke.inline_search_query.is_empty() {
            format!(" ⌕ Type song + Enter{cur_g}")
        } else {
            format!(" ⌕ {}{cur_g}", state.music.karaoke.inline_search_query)
        };
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    format!("{:<w$}", truncate_str(&q_disp, q_w as usize), w = q_w as usize),
                    Style::default()
                        .fg(Color::White)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(
                    " [x]",
                    Style::default()
                        .fg(theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
            ])),
            q_row,
        );

        if state.music.karaoke.inline_search_loading {
            frame.render_widget(
                Paragraph::new("   Searching YouTube Music…")
                    .style(Style::default().fg(theme.accent)),
                Rect::new(sr.x, sr.y + 1, sr.width, 1),
            );
        } else if state.music.karaoke.inline_search_results.is_empty() {
            frame.render_widget(
                Paragraph::new("   Enter query to append songs cleanly.")
                    .style(Style::default().fg(theme.text_dim)),
                Rect::new(sr.x, sr.y + 1, sr.width, 1),
            );
        } else {
            let max_res = (sr.height.saturating_sub(1) as usize).min(5);
            for (ri, tr) in state
                .music
                .karaoke
                .inline_search_results
                .iter()
                .take(max_res)
                .enumerate()
            {
                let rr = Rect::new(sr.x, sr.y + 1 + ri as u16, sr.width, 1);
                state
                    .music
                    .karaoke
                    .hitboxes
                    .push((rr, KaraokeHit::AddSearchTrack(ri)));
                let dur = tr
                    .duration_text
                    .as_deref()
                    .unwrap_or("");
                let line = format!(" + {} — {} {dur}", tr.title, tr.artist);
                frame.render_widget(
                    Paragraph::new(truncate_str(&line, rr.width as usize))
                        .style(Style::default().fg(spotify_green)),
                    rr,
                );
            }
        }
    }

    frame.render_widget(
        Paragraph::new("─".repeat(inner.width as usize))
            .style(Style::default().fg(theme.border_idle)),
        chunks[3],
    );

    // 4. Scrollable Setlist Track Cards (2 rows per track)
    let list = chunks[4];
    let items_len = state.music.karaoke.items.len();
    if items_len == 0 {
        frame.render_widget(
            Paragraph::new("\nSetlist empty.\nPress [K] on any song or [/] above to add.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim))
                .wrap(Wrap { trim: true }),
            list,
        );
    } else {
        if state.music.karaoke.cursor >= items_len {
            state.music.karaoke.cursor = items_len - 1;
        }
        let vis_cards = (list.height as usize / 2).max(1);
        let max_scroll = items_len.saturating_sub(vis_cards);
        if state.music.karaoke.cursor < state.music.karaoke.scroll {
            state.music.karaoke.scroll = state.music.karaoke.cursor;
        } else if state.music.karaoke.cursor >= state.music.karaoke.scroll + vis_cards {
            state.music.karaoke.scroll = state.music.karaoke.cursor + 1 - vis_cards;
        }
        state.music.karaoke.scroll = state.music.karaoke.scroll.min(max_scroll);

        let has_sb = items_len > vis_cards && list.height >= 4 && list.width >= 10;
        let row_w = if has_sb { list.width - 1 } else { list.width };

        for (vi, (idx, entry)) in state
            .music
            .karaoke
            .items
            .iter()
            .enumerate()
            .skip(state.music.karaoke.scroll)
            .take(vis_cards)
            .enumerate()
        {
            let ry = list.y + (vi as u16) * 2;
            if ry + 1 >= list.y + list.height {
                break;
            }
            let card_rect = Rect::new(list.x, ry, row_w, 2);
            let is_playing = state.music.karaoke.active_item_id == Some(entry.id)
                || (state.music.karaoke.session_active
                    && state.active_playback_video_id.as_deref() == Some(entry.video_id.as_str()));
            let is_sel = idx == state.music.karaoke.cursor;

            let badge = entry.lyrics_status.badge();
            let badge_w = badge.chars().count() as u16 + 1;
            let lead = if is_playing {
                "▶ ".to_string()
            } else if is_sel {
                "▸ ".to_string()
            } else {
                format!("{:>2}.", idx + 1)
            };
            let title_w = (row_w.saturating_sub(lead.chars().count() as u16 + badge_w)) as usize;
            let title_part = truncate_words_clean(
                &format!("{} · {}", entry.title, entry.artist),
                title_w.max(6),
            );
            let b_col = match entry.lyrics_status {
                KaraokeLyricsReadiness::SyncedWord | KaraokeLyricsReadiness::SyncedLine => {
                    spotify_green
                }
                KaraokeLyricsReadiness::Checking | KaraokeLyricsReadiness::Unknown => theme.accent,
                KaraokeLyricsReadiness::PlainOnly => Color::Rgb(245, 195, 80),
                KaraokeLyricsReadiness::Unavailable => Color::Rgb(230, 110, 110),
            };

            let row1 = Rect::new(list.x, ry, row_w, 1);
            let row2 = Rect::new(list.x, ry + 1, row_w, 1);
            state
                .music
                .karaoke
                .hitboxes
                .push((card_rect, KaraokeHit::SelectItem(idx)));

            frame.render_widget(
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        format!("{lead:<3}"),
                        Style::default()
                            .fg(if is_playing { spotify_green } else if is_sel { theme.accent } else { theme.text_dim })
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{title_part:<w$}", w = title_w.max(6)),
                        Style::default()
                            .fg(if is_playing {
                                spotify_green
                            } else if is_sel {
                                theme.accent
                            } else {
                                Color::White
                            })
                            .add_modifier(if is_sel || is_playing { Modifier::BOLD } else { Modifier::empty() }),
                    ),
                    Span::styled(format!(" {badge}"), Style::default().fg(b_col)),
                ])),
                row1,
            );

            let mic_str = entry
                .singer_name
                .as_deref()
                .map(|s| format!("Mic:{s}"))
                .unwrap_or_else(|| "Unassigned".to_string());
            let key_str = if entry.pitch_semitones != 0 {
                format!(" · {:+}st", entry.pitch_semitones)
            } else {
                String::new()
            };
            let dur_str = entry
                .duration_secs
                .map(|d| format!(" · {}", format_duration_secs(d)))
                .unwrap_or_default();

            let mut r2_spans = Vec::new();
            if is_sel {
                let meta_w = (row_w.saturating_sub(21)) as usize;
                let m_txt = truncate_str(&format!("   {mic_str}{key_str}{dur_str}"), meta_w.max(6));
                r2_spans.push(Span::styled(
                    format!("{m_txt:<w$}", w = meta_w.max(6)),
                    Style::default().fg(theme.text_dim),
                ));
                let mut bx = row2.x + meta_w.max(6) as u16;
                let max_rx = row2.x + row_w;
                let pill_bg = Color::Rgb(42, 48, 62);
                push_stage_pill(&mut state.music.karaoke.hitboxes, &mut r2_spans, &mut bx, (row2.y, max_rx), "↑", KaraokeHit::MoveItemUp(idx), (Color::White, pill_bg));
                push_stage_pill(&mut state.music.karaoke.hitboxes, &mut r2_spans, &mut bx, (row2.y, max_rx), "↓", KaraokeHit::MoveItemDown(idx), (Color::White, pill_bg));
                push_stage_pill(&mut state.music.karaoke.hitboxes, &mut r2_spans, &mut bx, (row2.y, max_rx), "Mic", KaraokeHit::StartSingerPrompt(idx), (Color::Black, theme.accent));
                push_stage_pill(&mut state.music.karaoke.hitboxes, &mut r2_spans, &mut bx, (row2.y, max_rx), "Ver", KaraokeHit::OpenVersionPicker(idx), (Color::Black, spotify_green));
                push_stage_pill(&mut state.music.karaoke.hitboxes, &mut r2_spans, &mut bx, (row2.y, max_rx), "x", KaraokeHit::RemoveItem(idx), (Color::Rgb(255, 130, 130), pill_bg));
            } else {
                r2_spans.push(Span::styled(
                    truncate_str(&format!("   {mic_str}{key_str}{dur_str}"), row_w as usize),
                    Style::default().fg(theme.text_dim),
                ));
            }
            frame.render_widget(Paragraph::new(Line::from(r2_spans)), row2);
        }

        if has_sb {
            render_vertical_scrollbar(
                frame,
                Rect::new(list.x + list.width - 1, list.y, 1, list.height),
                state.music.karaoke.scroll,
                vis_cards,
                items_len,
                theme.border_idle,
                theme.accent,
            );
        }
    }

    // 5. Summary + Actions Footer
    let foot = chunks[5];
    let synced_n = state
        .music
        .karaoke
        .items
        .iter()
        .filter(|e| e.lyrics_status.is_synced())
        .count();
    let total_dur: u64 = state
        .music
        .karaoke
        .items
        .iter()
        .filter_map(|e| e.duration_secs)
        .sum();
    let sung_n = state.music.karaoke.sung_history.len();
    let stat_line = format!(
        "{items_len} queued · {synced_n} synced · {} · Sung:{sung_n}",
        format_duration_secs(total_dur)
    );
    frame.render_widget(
        Paragraph::new(truncate_str(&stat_line, foot.width as usize))
            .style(Style::default().fg(spotify_green)),
        Rect::new(foot.x, foot.y, foot.width, 1),
    );
    let hint_line = fit_joined_parts(
        &["[2x/Enter] Sing", "[V] Backing/LRC", "[M] Mic Tag", "[Del] Remove"],
        " · ",
        foot.width as usize,
    );
    frame.render_widget(
        Paragraph::new(hint_line).style(Style::default().fg(theme.text_dim)),
        Rect::new(foot.x, foot.y + 1, foot.width, 1),
    );
}

fn render_karaoke_version_picker(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let Some(vp) = state.music.karaoke.version_picker.clone() else {
        return;
    };
    let theme = *state.base_theme();
    let spotify_green = Color::Rgb(30, 215, 96);

    let mw = (area.width.saturating_sub(6)).clamp(38, 76);
    let mh = (area.height.saturating_sub(4)).clamp(12, 20);
    let modal = Rect::new(
        area.x + area.width.saturating_sub(mw) / 2,
        area.y + area.height.saturating_sub(mh) / 2,
        mw,
        mh,
    );
    frame.render_widget(Clear, modal);
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(spotify_green))
        .title(Span::styled(
            format!(" Swap Track / Lyrics · {} ", truncate_str(&vp.track_title, 28)),
            Style::default()
                .fg(spotify_green)
                .add_modifier(Modifier::BOLD),
        ))
        .style(Style::default().bg(Color::Reset));
    let inner = blk.inner(modal);
    frame.render_widget(blk, modal);
    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
        ])
        .split(inner);

    // Tab Header
    let tab_lbl = if vp.tab_is_lrc {
        " Audio Backing (YTM)   [Synced Lyrics (LRCLIB)] "
    } else {
        " [Audio Backing (YTM)]   Synced Lyrics (LRCLIB) "
    };
    let mut tspans = vec![Span::styled(
        truncate_str(tab_lbl, chunks[0].width.saturating_sub(6) as usize),
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
    )];
    let close_x = chunks[0].x + chunks[0].width.saturating_sub(4);
    state.music.karaoke.hitboxes.push((
        Rect::new(close_x, chunks[0].y, 4, 1),
        KaraokeHit::CloseVersionPicker,
    ));
    tspans.push(Span::raw(" "));
    tspans.push(Span::styled(
        " [x]",
        Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
    ));
    frame.render_widget(Paragraph::new(Line::from(tspans)), chunks[0]);

    let body = chunks[1];
    if vp.loading {
        frame.render_widget(
            Paragraph::new("\nFetching karaoke backing tracks & alternate LRCLIB matches…")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.accent)),
            body,
        );
    } else if vp.tab_is_lrc {
        if vp.lrc_candidates.is_empty() {
            frame.render_widget(
                Paragraph::new("\nNo alternate LRCLIB synced candidates found.")
                    .alignment(Alignment::Center)
                    .style(Style::default().fg(theme.text_dim)),
                body,
            );
        } else {
            for (ci, (lbl, _)) in vp.lrc_candidates.iter().take(body.height as usize).enumerate() {
                let r = Rect::new(body.x, body.y + ci as u16, body.width, 1);
                state
                    .music
                    .karaoke
                    .hitboxes
                    .push((r, KaraokeHit::PickLrcCandidate(ci)));
                let sel = ci == vp.cursor;
                frame.render_widget(
                    Paragraph::new(truncate_str(
                        &format!("{} {lbl}", if sel { "▸" } else { " " }),
                        r.width as usize,
                    ))
                    .style(Style::default().fg(if sel { spotify_green } else { Color::White })),
                    r,
                );
            }
        }
    } else if vp.audio_candidates.is_empty() {
        frame.render_widget(
            Paragraph::new("\nNo alternate YTM Karaoke / Instrumental audio tracks found.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            body,
        );
    } else {
        for (ci, tr) in vp.audio_candidates.iter().take(body.height as usize).enumerate() {
            let r = Rect::new(body.x, body.y + ci as u16, body.width, 1);
            state
                .music
                .karaoke
                .hitboxes
                .push((r, KaraokeHit::PickAudioVersion(ci)));
            let sel = ci == vp.cursor;
            let dur = tr.duration_text.as_deref().unwrap_or("");
            frame.render_widget(
                Paragraph::new(truncate_str(
                    &format!("{} {} — {} {dur}", if sel { "▸" } else { " " }, tr.title, tr.artist),
                    r.width as usize,
                ))
                .style(Style::default().fg(if sel { spotify_green } else { Color::White })),
                r,
            );
        }
    }

    frame.render_widget(
        Paragraph::new("[Tab] Switch Audio/Lyrics · [Enter/Click] Apply · [Esc] Close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ytmusic::{YtmLyricLine, YtmLyricWord, YtmLyrics};
    use crate::state::{
        AppState, Category, KaraokeLyricsReadiness, KaraokePlaylistEntry, KaraokePlaylistSummary,
        YtmSection,
    };
    use ratatui::{backend::TestBackend, Terminal};

    fn make_karaoke_state() -> AppState {
        let mut st = AppState::new(crate::config::Config::default());
        let music_idx = st
            .categories
            .iter()
            .position(|(c, _)| *c == Category::Music)
            .unwrap();
        st.active_category = music_idx;
        st.music.section = YtmSection::Karaoke;
        st.music.karaoke.playlists = vec![KaraokePlaylistSummary {
            id: 1,
            name: "Friday Night".to_string(),
            is_default: true,
            item_count: 2,
            synced_count: 2,
            total_duration_secs: 450,
        }];
        st.music.karaoke.active_playlist_id = 1;
        st.music.karaoke.items = vec![
            KaraokePlaylistEntry {
                id: 10,
                playlist_id: 1,
                video_id: "vid_1".to_string(),
                ytm_playlist_id: None,
                title: "Starlight".to_string(),
                artist: "Muse".to_string(),
                album: Some("Black Holes".to_string()),
                thumb_url: String::new(),
                duration_secs: Some(240),
                singer_name: Some("Alex".to_string()),
                pitch_semitones: 2,
                lyric_offset_ms: 100,
                sort_order: 0,
                sung_count: 0,
                lyrics_status: KaraokeLyricsReadiness::SyncedWord,
            },
            KaraokePlaylistEntry {
                id: 11,
                playlist_id: 1,
                video_id: "vid_2".to_string(),
                ytm_playlist_id: None,
                title: "Lips".to_string(),
                artist: "Jane Remover".to_string(),
                album: None,
                thumb_url: String::new(),
                duration_secs: Some(210),
                singer_name: Some("Sam".to_string()),
                pitch_semitones: 0,
                lyric_offset_ms: 0,
                sort_order: 1,
                sung_count: 0,
                lyrics_status: KaraokeLyricsReadiness::SyncedLine,
            },
        ];
        st.music.karaoke.active_item_id = Some(10);
        st.music.queue = vec![st.music.karaoke.items[0].to_ytm_track()];
        st.active_playback_video_id = Some("vid_1".to_string());
        st.active_playback_pos = 11.4;
        st.active_playback_dur = 240.0;
        st.music.lyrics = Some(YtmLyrics {
            lines: vec![
                YtmLyricLine {
                    text: "Far away".to_string(),
                    start_ms: Some(10_000),
                    end_ms: Some(14_000),
                    words: Some(vec![
                        YtmLyricWord {
                            text: "Far ".to_string(),
                            start_ms: 10_000,
                            end_ms: 11_200,
                        },
                        YtmLyricWord {
                            text: "away".to_string(),
                            start_ms: 11_200,
                            end_ms: 14_000,
                        },
                    ]),
                },
                YtmLyricLine {
                    text: "Hold you in my arms".to_string(),
                    start_ms: Some(14_500),
                    end_ms: Some(18_500),
                    words: None,
                },
            ],
            source: Some("Source: LRCLIB (Synced)".to_string()),
            is_timed: true,
        });
        st
    }

    #[test]
    fn test_karaoke_layout_dims_avoid_claustrophobia_and_barren_ultrawide() {
        let narrow = karaoke_layout_dims(80, false, false, false);
        assert!(narrow.show_stage && !narrow.show_setlist);

        let desk = karaoke_layout_dims(118, false, false, false);
        assert!(desk.show_stage && desk.show_setlist);
        assert!(desk.stage_w >= 80 && desk.setlist_w >= 34);

        let ultra = karaoke_layout_dims(200, false, false, false);
        assert!(ultra.corridor_w <= 92);

        let full = karaoke_layout_dims(140, false, true, false);
        assert!(full.show_stage && !full.show_setlist);
        assert_eq!(full.stage_w, 140);
        assert!(full.corridor_w >= 110);
    }

    #[test]
    fn test_word_level_sung_chars_uses_word_spans_and_sweep_speed() {
        let line = YtmLyricLine {
            text: "Far away".to_string(),
            start_ms: Some(10_000),
            end_ms: Some(14_000),
            words: Some(vec![
                YtmLyricWord {
                    text: "Far ".to_string(),
                    start_ms: 10_000,
                    end_ms: 11_000,
                },
                YtmLyricWord {
                    text: "away".to_string(),
                    start_ms: 11_000,
                    end_ms: 14_000,
                },
            ]),
        };
        assert_eq!(compute_lyric_sung_chars(&line, 11_000, None, 100), 4);
        assert_eq!(compute_lyric_sung_chars(&line, 12_500, None, 100), 6);

        // Faster manual sweep (150%) finishes ahead of 100%, slower (50%) trails smoothly
        let plain_line = YtmLyricLine {
            text: "Fast vocals ahead of sweep".to_string(),
            start_ms: Some(10_000),
            end_ms: Some(16_000),
            words: None,
        };
        let c_slow = compute_lyric_sung_chars(&plain_line, 13_000, None, 50);
        let c_norm = compute_lyric_sung_chars(&plain_line, 13_000, None, 100);
        let c_fast = compute_lyric_sung_chars(&plain_line, 13_000, None, 150);
        assert!(c_slow < c_norm && c_norm < c_fast);
        assert_eq!(compute_lyric_sung_chars(&plain_line, 14_000, None, 150), 26);
    }

    #[test]
    fn test_karaoke_fullscreen_renders_big_lyrics_sweep_beam_and_bottom_keybinds() {
        let mut st = make_karaoke_state();
        st.music.karaoke.fullscreen = true;
        st.music.karaoke.sweep_speed_pct = 130;
        let backend = TestBackend::new(140, 42);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| crate::ui::home::render_home_screen(f, &mut st, None))
            .unwrap();
        let text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(text.contains("FULL KARAOKE"));
        assert!(text.contains("Swp:1.3x"));
        assert!(text.contains("[F/Esc] Exit Fullscreen"));
        assert!(text.contains("[w/W] Word Sweep Speed"));
        assert!(text.contains("Far away"));
        // Active sweep progress beam rendered beneath active line
        assert!(text.contains("●"));
        assert_eq!(term.backend().buffer()[(70, 20)].bg, Color::Reset);
        assert!(st
            .music
            .karaoke
            .hitboxes
            .iter()
            .any(|(_, h)| *h == KaraokeHit::SweepSpeedDelta(10)));
        assert!(st
            .music
            .karaoke
            .hitboxes
            .iter()
            .any(|(_, h)| *h == KaraokeHit::ToggleFullscreen));

        // Floating Setlist Queue drawer in Fullscreen
        st.music.karaoke.show_compact_setlist = true;
        term.draw(|f| crate::ui::home::render_home_screen(f, &mut st, None))
            .unwrap();
        let q_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(q_text.contains("Friday Night"));
        assert!(q_text.contains("Jane Remover"));
    }

    #[test]
    fn test_karaoke_workspace_renders_practice_party_focus_and_intermission_cleanly() {
        for (w, h) in [(104, 32), (140, 42), (200, 50)] {
            let mut st = make_karaoke_state();
            let backend = TestBackend::new(w, h);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| crate::ui::home::render_home_screen(f, &mut st, None))
                .unwrap();

            let text: String = term
                .backend()
                .buffer()
                .content()
                .iter()
                .map(|c| c.symbol())
                .collect();
            assert!(text.contains("KARAOKE"));
            assert!(text.contains("Friday Night"));
            assert!(text.contains("Far away"));
            assert!(text.contains("Mic: Alex"));
            assert_eq!(term.backend().buffer()[(w / 2, h / 2)].bg, Color::Reset);
            for bad in ["🔀", "📻", "💾", "⚙", "➕", "❌", "♪"] {
                assert!(!text.contains(bad), "unexpected emoji {bad}");
            }
        }

        // Party mode + 5s Up Next Intermission Card
        let mut st = make_karaoke_state();
        st.music.karaoke.mode = KaraokeMode::Party;
        st.music.karaoke.intermission_next_idx = Some(1);
        st.music.karaoke.intermission_deadline =
            Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
        let backend = TestBackend::new(140, 42);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| crate::ui::home::render_home_screen(f, &mut st, None))
            .unwrap();
        let party_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(party_text.contains("UP NEXT"));
        assert!(party_text.contains("Jane Remover"));
        assert!(st
            .music
            .karaoke
            .hitboxes
            .iter()
            .any(|(_, h)| *h == KaraokeHit::IntermissionStartNow));
    }
}
