//! Page 1 — `[1] Deck`: Overview Bento KPIs, Precision Speed/Scrub/A-B/Sleep/Session Console,
//! 6-Channel Picture Equalizer w/ Grade Presets, Live Tone Curve + Scope, & Zero-Copy GPU Geometry.

use crate::config::{PerformanceMode, PicturePreset, Quality};
use crate::state::{AppState, PlayOptionsHit, SleepTimerMode};
use crate::ui::studio::widgets::{
    compute_luma_scope, format_hms, format_speed_bps, mode_badge_color, push_pane_hit,
    render_bento_kpi_card, render_bipolar_slider_range, render_inline_pills,
    render_tone_curve_lines, section_block, sparkline_string, truncate_words,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render_deck_page(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 30 || area.height < 12 {
        return;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Top 4 Bento KPI cards
            Constraint::Min(10),   // Main 2-column Deck + Picture EQ
            Constraint::Length(3), // Bottom Quick Engine Presets strip
        ])
        .split(area);

    render_bento_strip(frame, rows[0], state);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(rows[1]);

    render_transport_session_pane(frame, cols[0], state);
    render_picture_eq_pane(frame, cols[1], state);
    render_quick_presets_strip(frame, rows[2], state);
}

fn sleep_countdown_label(state: &AppState) -> Option<String> {
    match state.live_playback.sleep_timer_mode {
        SleepTimerMode::Off => None,
        SleepTimerMode::EndOfVideo => Some("EOF".to_string()),
        _ => {
            if let Some(deadline) = state.live_playback.sleep_timer_deadline {
                let rem = deadline
                    .saturating_duration_since(std::time::Instant::now())
                    .as_secs();
                Some(format_hms(rem as f64))
            } else {
                Some(state.live_playback.sleep_timer_mode.label().to_string())
            }
        }
    }
}

fn render_bento_strip(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = *state.base_theme();
    let live = state.has_live_ipc();
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    // Card 1: Stream & Decode
    let q = Quality::ALL
        .get(state.play_quality_idx)
        .copied()
        .unwrap_or(state.config.quality);
    let stream_head = if live && state.live_playback.current_stream_height > 0 {
        let codec = if !state.live_playback.video_codec_live.is_empty() {
            state.live_playback.video_codec_live.to_uppercase()
        } else {
            state
                .config
                .effective_mastering()
                .codec_priority
                .label()
                .to_string()
        };
        format!(
            "{}p{:.0} · {}",
            state.live_playback.current_stream_height,
            state.live_playback.current_stream_fps.max(30.0),
            codec
        )
    } else {
        format!("{} · {}", q.short_badge(), state.config.player.as_str())
    };
    let hw = if live && !state.live_playback.hwdec_current.is_empty() {
        state.live_playback.hwdec_current.clone()
    } else {
        state.config.effective_mastering().hwdec.label().to_string()
    };
    let fps_txt = if live && state.live_playback.estimated_vf_fps > 0.1 {
        format!(" · {:.1} fps", state.live_playback.estimated_vf_fps)
    } else {
        String::new()
    };
    render_bento_kpi_card(
        frame,
        cols[0],
        " STREAM & DECODE ",
        &stream_head,
        vec![Span::styled(
            format!(" {}{}", truncate_words(&hw, 18), fps_txt),
            Style::default().fg(theme.text_dim),
        )],
        theme.primary,
        &theme,
    );

    // Card 2: Buffer & Network
    let spd_samples: Vec<f64> = state
        .live_playback
        .telemetry_history
        .cache_speed_kbps
        .iter()
        .copied()
        .collect();
    let spark = sparkline_string(&spd_samples, 8);
    let net_head = if live {
        format!(
            "{spark} {}",
            format_speed_bps(state.live_playback.cache_speed_bps)
        )
    } else {
        state.config.network_mode.label().to_string()
    };
    let cache_mb = state.live_playback.cache_bytes / (1024 * 1024);
    let max_mb = state.config.effective_demuxer_max_bytes_num() / (1024 * 1024);
    let buf_sub = if live {
        format!(
            " Buf {:.1}s ({}/{}M) · Stl:{}",
            state.live_playback.cache_ahead_secs,
            cache_mb,
            max_mb,
            state.live_playback.stall_count
        )
    } else {
        format!(
            " Max Buf {}M · Readahead {}s",
            max_mb,
            state.config.network_mode.vod_readahead_secs()
        )
    };
    render_bento_kpi_card(
        frame,
        cols[1],
        " BUFFER & NETWORK ",
        &net_head,
        vec![Span::styled(buf_sub, Style::default().fg(theme.text_dim))],
        Color::Rgb(56, 189, 248),
        &theme,
    );

    // Card 3: GPU Mastering + Sleep indicator
    let pm = state.config.performance_mode;
    let master_head = format!(
        "{} · {}",
        pm.short_badge(),
        state.config.shader_mode.short_label()
    );
    let sleep_part = sleep_countdown_label(state)
        .map(|s| format!(" · Slp:{s}"))
        .unwrap_or_default();
    let master_sub = format!(
        " {} · Drp:{}{}",
        state.config.motion_mode.short_badge(),
        state.live_playback.dropped_frames,
        sleep_part
    );
    render_bento_kpi_card(
        frame,
        cols[2],
        " GPU MASTERING ",
        &master_head,
        vec![Span::styled(master_sub, Style::default().fg(theme.text_dim))],
        mode_badge_color(pm),
        &theme,
    );

    // Card 4: Audio, Grade & Guard
    let aud_head = if state.config.dub_preference.is_dub_on() {
        format!("DUB ON ({})", state.config.dub_lang.to_uppercase())
    } else {
        "Original Audio [ORIG]".to_string()
    };
    let sub_lbl = if state.live_playback.sub_visible {
        "ON"
    } else {
        "OFF"
    };
    let grade_lbl = if state.live_playback.picture_bypass {
        "BYP"
    } else {
        state.config.picture.preset.short_label()
    };
    let guard_sub = format!(
        " Subs:{} · Eq:{} · SB:{}",
        sub_lbl, grade_lbl, state.sponsor_skips_session
    );
    render_bento_kpi_card(
        frame,
        cols[3],
        " AUDIO & GUARD ",
        &aud_head,
        vec![Span::styled(guard_sub, Style::default().fg(theme.text_dim))],
        theme.accent,
        &theme,
    );
}

fn active_chapter_label(state: &AppState) -> Option<String> {
    let vid = state.active_selected_video()?;
    let chs = vid.effective_chapters();
    if chs.is_empty() {
        return None;
    }
    let pos = state.active_playback_pos;
    let cur = chs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, c)| pos >= c.start_time)
        .or_else(|| chs.first().map(|c| (0, c)));
    cur.map(|(idx, c)| format!("{:02} · {}", idx + 1, c.title))
}

fn render_divider_row(frame: &mut Frame, area: Rect, theme: &crate::ui::theme::ThemePalette) {
    if area.width > 2 && area.height > 0 {
        let line = "─".repeat(area.width.saturating_sub(2) as usize);
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!(" {line}"),
                Style::default().fg(theme.divider),
            )),
            area,
        );
    }
}

fn render_transport_session_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Transport, Volume & Session ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 28 || inner.height < 9 {
        return;
    }

    let live = state.has_live_ipc();
    let h = inner.height;
    let compact = h < 15;
    let show_dividers = h >= 19;
    let show_separate_shot = h >= 19;
    let show_separate_pitch = h >= 21;

    // Build exact row slots so bottom session controls sit cleanly with zero dead trailing block
    let mut constraints: Vec<Constraint> = vec![
        Constraint::Length(1), // Speed header + ETA
        Constraint::Length(1), // Speed pills
    ];
    if show_separate_pitch {
        constraints.push(Constraint::Length(1)); // Pitch lock row
    }
    constraints.push(Constraint::Length(1)); // Scrub & frame header
    constraints.push(Constraint::Length(1)); // Scrub & frame pills
    if show_dividers {
        constraints.push(Constraint::Length(1)); // Divider
    }
    constraints.push(Constraint::Length(1)); // Volume header + MUTE
    constraints.push(Constraint::Length(1)); // Visual volume fader
    if !compact {
        constraints.push(Constraint::Length(1)); // Volume quick level pills + Loudnorm
    }
    if show_dividers {
        constraints.push(Constraint::Length(1)); // Divider
    }
    if !compact {
        constraints.push(Constraint::Length(1)); // A-B loop header + live bounds
    }
    constraints.push(Constraint::Length(1)); // A-B pills (+ Shot when not separate)
    if show_separate_shot {
        constraints.push(Constraint::Length(1)); // Dedicated Screenshot PNG row
    }
    if show_dividers {
        constraints.push(Constraint::Length(1)); // Divider
    }
    if h >= 13 {
        constraints.push(Constraint::Length(1)); // Sleep timer header + countdown
        constraints.push(Constraint::Length(1)); // Sleep pills
    }
    if h >= 16 {
        constraints.push(Constraint::Length(1)); // Chapter / Resume anchor
    }
    // Absorb any remaining extra rows (only on extra-tall screens >= 22) above the bottom session footer
    constraints.push(Constraint::Min(0));
    constraints.push(Constraint::Length(1)); // Window toggles: OnTop, Fullscreen, Loop
    if h >= 12 {
        constraints.push(Constraint::Length(1)); // Audio Radio
    }
    constraints.push(Constraint::Length(1)); // Launch/Relaunch + Stop Player

    let rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let w = inner.width as usize;
    let speed = state.live_playback.speed.max(0.25);
    let dur = if state.active_playback_dur > 1.0 {
        state.active_playback_dur
    } else {
        state
            .active_selected_video()
            .and_then(|v| v.duration)
            .unwrap_or(0) as f64
    };
    let rem_media = (dur - state.active_playback_pos).max(0.0);
    let rem_wall = rem_media / speed;
    let pitch_on = state.live_playback.pitch_correction;

    let mut r_idx = 0usize;
    let mut next_rect = || -> Rect {
        let r = rects[r_idx];
        r_idx += 1;
        r
    };

    // Snapshot immutable context BEFORE borrowing `play_hitboxes` mutably!
    let ch_lbl = active_chapter_label(state);
    let sleep_rem_lbl = sleep_countdown_label(state);
    let active_preset_q = Quality::ALL
        .get(state.play_quality_idx)
        .copied()
        .unwrap_or(state.config.quality);

    let hits = &mut state.live_playback.play_hitboxes;

    // 1. Speed header + ETA
    let speed_hdr_r = next_rect();
    let eta_txt = if dur > 1.0 {
        if w >= 54 {
            format!(
                "ETA {} -> {} @ {:.2}x",
                format_hms(rem_media),
                format_hms(rem_wall),
                speed
            )
        } else {
            format!("{}->{} @{:.2}x", format_hms(rem_media), format_hms(rem_wall), speed)
        }
    } else {
        format!("Speed {:.2}x", speed)
    };
    let hdr_left = if w >= 50 {
        " Playback Speed ([ / ]):"
    } else {
        " Speed ([ / ]):"
    };
    let pad = w.saturating_sub(hdr_left.len() + eta_txt.len() + 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                hdr_left,
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(pad)),
            Span::styled(
                truncate_words(&eta_txt, w.saturating_sub(hdr_left.len() + 2)),
                Style::default().fg(theme.text_dim),
            ),
        ])),
        speed_hdr_r,
    );

    // 2. Speed pills
    let speed_pills_r = next_rect();
    let speeds = [0.50_f64, 0.75, 1.0, 1.25, 1.5, 2.0];
    let mut sp_pills: Vec<(String, Style, PlayOptionsHit)> = speeds
        .iter()
        .map(|&s| {
            let active = (speed - s).abs() < 0.03;
            let lbl = if (s - 2.0).abs() < 0.01 {
                "[2.0x]".to_string()
            } else {
                format!("[{s:.2}]")
            };
            let st = if active {
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            (lbl, st, PlayOptionsHit::SetSpeed(s))
        })
        .collect();
    if w >= 56 {
        sp_pills.push((
            "[-.05]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::SpeedDelta(-0.05),
        ));
        sp_pills.push((
            "[+.05]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::SpeedDelta(0.05),
        ));
    }
    render_inline_pills(
        frame,
        speed_pills_r,
        Some((" ", Style::default())),
        &sp_pills,
        hits,
    );

    // Optional separate Pitch Lock row
    if show_separate_pitch {
        let pitch_r = next_rect();
        let p_pills = vec![(
            if pitch_on {
                "[Pitch Lock: ON (Natural Voice)]".to_string()
            } else {
                "[Pitch Lock: OFF (Raw Tempo)]".to_string()
            },
            Style::default()
                .fg(if pitch_on {
                    Color::Rgb(52, 211, 153)
                } else {
                    Color::Rgb(251, 191, 36)
                })
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::TogglePitchCorrection,
        )];
        render_inline_pills(
            frame,
            pitch_r,
            Some((" [P] Audio Pitch:", Style::default().fg(theme.text_dim))),
            &p_pills,
            hits,
        );
    }

    // 3. Scrub & Frame step header + pills
    let scrub_hdr_r = next_rect();
    let scrub_hdr_right = if !show_separate_pitch {
        format!("[P] Pitch:{}", if pitch_on { "ON" } else { "OFF" })
    } else {
        "Frame-Accurate Seek".to_string()
    };
    let s_left = " Scrub & Frame Step (, / .):";
    let s_pad = w.saturating_sub(s_left.len() + scrub_hdr_right.len() + 1);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                s_left,
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ".repeat(s_pad)),
            Span::styled(scrub_hdr_right, Style::default().fg(theme.text_dim)),
        ])),
        scrub_hdr_r,
    );

    let scrub_pills_r = next_rect();
    let scrub_pills = vec![
        ("[-60s]".to_string(), Style::default().fg(theme.text), PlayOptionsHit::SeekDelta(-60.0)),
        ("[-5s]".to_string(), Style::default().fg(theme.text), PlayOptionsHit::SeekDelta(-5.0)),
        ("[<Frm]".to_string(), Style::default().fg(theme.accent), PlayOptionsHit::FrameStep(-1)),
        ("[Frm>]".to_string(), Style::default().fg(theme.accent), PlayOptionsHit::FrameStep(1)),
        ("[+5s]".to_string(), Style::default().fg(theme.text), PlayOptionsHit::SeekDelta(5.0)),
        ("[+60s]".to_string(), Style::default().fg(theme.text), PlayOptionsHit::SeekDelta(60.0)),
    ];
    render_inline_pills(
        frame,
        scrub_pills_r,
        Some((" ", Style::default())),
        &scrub_pills,
        hits,
    );

    if show_dividers {
        render_divider_row(frame, next_rect(), &theme);
    }

    // 4. Master Volume header + visual fader + quick levels
    let vol_hdr_r = next_rect();
    let vol_hdr_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(18), Constraint::Length(12)])
        .split(vol_hdr_r);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Master Volume (-/+):  {}%", state.live_playback.volume),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        vol_hdr_cols[0],
    );
    let mute_pill = vec![(
        if state.live_playback.muted {
            "[MUTE:ON]".to_string()
        } else {
            "[MUTE:OFF]".to_string()
        },
        if state.live_playback.muted {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(239, 68, 68))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.accent)
        },
        PlayOptionsHit::ToggleMute,
    )];
    render_inline_pills(frame, vol_hdr_cols[1], None, &mute_pill, hits);

    let fader_r = next_rect();
    let vol_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(4), // " [-]"
            Constraint::Min(10),   // Bar
            Constraint::Length(7), // " 100% "
            Constraint::Length(4), // "[+] "
        ])
        .split(fader_r);
    frame.render_widget(
        Paragraph::new(Span::styled(" [-]", Style::default().fg(theme.secondary))),
        vol_cols[0],
    );
    let bar_w = vol_cols[1].width.saturating_sub(2).max(6) as usize;
    let vol = state.live_playback.volume.clamp(0, 150) as usize;
    let filled = ((vol * bar_w) + 75) / 150;
    let unity_idx = ((100 * bar_w) + 75) / 150;
    let mut bar_spans = vec![Span::raw(" ")];
    for i in 0..bar_w {
        if i == unity_idx && i >= filled {
            bar_spans.push(Span::styled("│", Style::default().fg(theme.text_dim)));
        } else if i < filled {
            let col = if i > unity_idx {
                Color::Rgb(251, 191, 36)
            } else {
                Color::Rgb(52, 211, 153)
            };
            bar_spans.push(Span::styled("█", Style::default().fg(col)));
        } else {
            bar_spans.push(Span::styled("░", Style::default().fg(theme.divider)));
        }
    }
    frame.render_widget(Paragraph::new(Line::from(bar_spans)), vol_cols[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {:>3}% ", state.live_playback.volume),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )),
        vol_cols[2],
    );
    frame.render_widget(
        Paragraph::new(Span::styled("[+] ", Style::default().fg(theme.secondary))),
        vol_cols[3],
    );
    hits.push((vol_cols[0], PlayOptionsHit::VolumeDelta(-5)));
    hits.push((vol_cols[1], PlayOptionsHit::SetVolumeRatio));
    hits.push((vol_cols[3], PlayOptionsHit::VolumeDelta(5)));

    if !compact {
        let lvl_r = next_rect();
        let mut lvl_pills: Vec<(String, Style, PlayOptionsHit)> = [25_i64, 50, 75, 100, 125]
            .into_iter()
            .map(|lvl| {
                let active = (state.live_playback.volume - lvl).abs() <= 2;
                (
                    format!("[{lvl}%]"),
                    if active {
                        Style::default()
                            .fg(Color::Black)
                            .bg(theme.primary)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    },
                    PlayOptionsHit::SetVolume(lvl),
                )
            })
            .collect();
        lvl_pills.push((
            if state.config.audio_loudnorm {
                "[Norm:ON]".to_string()
            } else {
                "[Norm:OFF]".to_string()
            },
            Style::default().fg(if state.config.audio_loudnorm {
                Color::Rgb(52, 211, 153)
            } else {
                theme.text_dim
            }),
            PlayOptionsHit::ToggleLoudnorm,
        ));
        render_inline_pills(frame, lvl_r, Some((" ", Style::default())), &lvl_pills, hits);
    }

    if show_dividers {
        render_divider_row(frame, next_rect(), &theme);
    }

    // 5. A-B Clip Looper & Screenshot
    let a_str = state
        .live_playback
        .ab_loop_a
        .map(format_hms)
        .unwrap_or_else(|| "--:--".to_string());
    let b_str = state
        .live_playback
        .ab_loop_b
        .map(format_hms)
        .unwrap_or_else(|| "--:--".to_string());
    let ab_active =
        state.live_playback.ab_loop_a.is_some() && state.live_playback.ab_loop_b.is_some();

    if !compact {
        let ab_hdr_r = next_rect();
        let ab_status = format!("A:{a_str} B:{b_str}");
        let ab_left = if w >= 48 {
            " A-B Clip Looper ({ / } / \\):"
        } else {
            " A-B Loop ({ / } / \\):"
        };
        let ab_pad = w.saturating_sub(ab_left.len() + ab_status.len() + 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    ab_left,
                    Style::default()
                        .fg(theme.secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(ab_pad)),
                Span::styled(
                    ab_status,
                    Style::default().fg(if ab_active {
                        Color::Rgb(52, 211, 153)
                    } else {
                        theme.text_dim
                    }),
                ),
            ])),
            ab_hdr_r,
        );
    }

    let ab_pills_r = next_rect();
    let mut ab_pills = vec![
        (
            if state.live_playback.ab_loop_a.is_some() {
                format!("[A:{a_str}]")
            } else {
                "[Set A]".to_string()
            },
            Style::default().fg(if state.live_playback.ab_loop_a.is_some() {
                theme.primary
            } else {
                theme.text
            }),
            PlayOptionsHit::MarkAbLoopA,
        ),
        (
            if state.live_playback.ab_loop_b.is_some() {
                format!("[B:{b_str}]")
            } else {
                "[Set B]".to_string()
            },
            Style::default().fg(if state.live_playback.ab_loop_b.is_some() {
                theme.primary
            } else {
                theme.text
            }),
            PlayOptionsHit::MarkAbLoopB,
        ),
        (
            "[Clear]".to_string(),
            Style::default().fg(theme.text_dim),
            PlayOptionsHit::ClearAbLoop,
        ),
    ];
    if !show_separate_shot {
        ab_pills.push((
            "[K Shot]".to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::TakeScreenshot,
        ));
    }
    render_inline_pills(frame, ab_pills_r, Some((" ", Style::default())), &ab_pills, hits);

    if show_separate_shot {
        let shot_r = next_rect();
        let shot_pills = vec![(
            "[K Screenshot PNG]".to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::TakeScreenshot,
        )];
        render_inline_pills(
            frame,
            shot_r,
            Some((" ", Style::default())),
            &shot_pills,
            hits,
        );
    }

    if show_dividers {
        render_divider_row(frame, next_rect(), &theme);
    }

    // 6. Sleep Timer
    if h >= 13 {
        let slp_hdr_r = next_rect();
        let slp_right = sleep_rem_lbl
            .map(|s| format!("Active: {s}"))
            .unwrap_or_else(|| "Inactive".to_string());
        let slp_left = " Sleep Timer [T]:";
        let slp_pad = w.saturating_sub(slp_left.len() + slp_right.len() + 1);
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(
                    slp_left,
                    Style::default()
                        .fg(theme.secondary)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::raw(" ".repeat(slp_pad)),
                Span::styled(
                    slp_right,
                    Style::default().fg(if state.live_playback.sleep_timer_mode == SleepTimerMode::Off {
                        theme.text_dim
                    } else {
                        Color::Rgb(251, 191, 36)
                    }),
                ),
            ])),
            slp_hdr_r,
        );

        let slp_pills_r = next_rect();
        let slp_pills: Vec<(String, Style, PlayOptionsHit)> = SleepTimerMode::ALL
            .iter()
            .map(|&mode| {
                let active = state.live_playback.sleep_timer_mode == mode;
                (
                    format!("[{}]", mode.label()),
                    if active {
                        Style::default()
                            .fg(Color::Black)
                            .bg(if mode == SleepTimerMode::Off {
                                theme.secondary
                            } else {
                                Color::Rgb(251, 191, 36)
                            })
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(theme.text)
                    },
                    PlayOptionsHit::SetSleepTimer(mode),
                )
            })
            .collect();
        render_inline_pills(frame, slp_pills_r, Some((" ", Style::default())), &slp_pills, hits);
    }

    // 7. Chapter / Anchor Strip
    if h >= 16 {
        let ch_r = next_rect();
        let ch_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Min(16), Constraint::Length(12)])
            .split(ch_r);
        let ch_txt = if let Some(lbl) = ch_lbl {
            format!(" Ch: {}", truncate_words(&lbl, ch_cols[0].width.saturating_sub(6) as usize))
        } else if state.active_playback_pos > 3.0 {
            format!(" Anchor: Resumed @ {}", format_hms(state.active_playback_pos))
        } else {
            format!(
                " Target: {} ({})",
                active_preset_q.short_badge(),
                state.config.player.as_str()
            )
        };
        frame.render_widget(
            Paragraph::new(Span::styled(ch_txt, Style::default().fg(theme.text_dim))),
            ch_cols[0],
        );
        let ch_pills = vec![
            ("[<Ch]".to_string(), Style::default().fg(theme.text), PlayOptionsHit::ChapterStep(-1)),
            ("[Ch>]".to_string(), Style::default().fg(theme.text), PlayOptionsHit::ChapterStep(1)),
        ];
        render_inline_pills(frame, ch_cols[1], None, &ch_pills, hits);
    }

    // Flex spacer above bottom session controls (draws crisp divider if >=1 row available)
    let gap_r = next_rect();
    if gap_r.height >= 1 {
        render_divider_row(
            frame,
            Rect::new(gap_r.x, gap_r.y + gap_r.height - 1, gap_r.width, 1),
            &theme,
        );
    }

    // 8. Window Toggles (OnTop / Fullscreen / Loop)
    let win_r = next_rect();
    let full_lbl = if w >= 52 { "Fullscreen" } else { "Full" };
    let win_pills = vec![
        (
            format!("[OnTop:{}]", if state.live_playback.ontop { "ON" } else { "OFF" }),
            Style::default().fg(if state.live_playback.ontop { theme.primary } else { theme.text }),
            PlayOptionsHit::ToggleOnTop,
        ),
        (
            format!("[{full_lbl}:{}]", if state.live_playback.fullscreen { "ON" } else { "OFF" }),
            Style::default().fg(if state.live_playback.fullscreen { theme.primary } else { theme.text }),
            PlayOptionsHit::ToggleFullscreen,
        ),
        (
            format!("[Loop:{}]", if state.live_playback.loop_video { "ON" } else { "OFF" }),
            Style::default().fg(if state.live_playback.loop_video { theme.primary } else { theme.text }),
            PlayOptionsHit::ToggleLoop,
        ),
    ];
    render_inline_pills(frame, win_r, Some((" ", Style::default())), &win_pills, hits);

    // 9. Audio Radio
    if h >= 12 {
        let rad_r = next_rect();
        let rad_pills = vec![(
            "[R Audio Radio]".to_string(),
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::LaunchAudioRadio,
        )];
        render_inline_pills(frame, rad_r, Some((" ", Style::default())), &rad_pills, hits);
    }

    // 10. Launch / Relaunch + Stop Player (flush at bottom)
    let sess_r = next_rect();
    let sess_pills = vec![
        (
            if live {
                "[Enter Relaunch / Apply]".to_string()
            } else {
                "[Enter Launch Video]".to_string()
            },
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::LaunchOrRelaunch,
        ),
        (
            "[X Stop Player]".to_string(),
            Style::default()
                .fg(Color::Rgb(239, 68, 68))
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::StopPlayer,
        ),
    ];
    render_inline_pills(frame, sess_r, Some((" ", Style::default())), &sess_pills, hits);

    push_pane_hit(hits, area, 0);
}

fn render_picture_eq_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Picture Equalizer, Presets & Geometry ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 30 || inner.height < 10 {
        return;
    }

    let eq_cursor = state.studio.eq_cursor;
    let bri = state.live_playback.brightness;
    let con = state.live_playback.contrast;
    let sat = state.live_playback.saturation;
    let gam = state.live_playback.gamma;
    let hue = state.live_playback.hue;
    let shp = state.live_playback.sharpness;
    let bypass = state.live_playback.picture_bypass;

    let h = inner.height;
    let show_all_6 = h >= 14;
    let show_dividers = h >= 20;
    let show_geometry = h >= 17;

    let mut constraints = vec![
        Constraint::Length(1), // 0: Preset header + description
        Constraint::Length(1), // 1: Preset pills Row 1
        Constraint::Length(1), // 2: Preset pills Row 2 + Bypass
    ];
    if show_dividers {
        constraints.push(Constraint::Length(1)); // Divider
    }
    // EQ sliders (4 or 6)
    constraints.push(Constraint::Length(1)); // Brightness
    constraints.push(Constraint::Length(1)); // Contrast
    constraints.push(Constraint::Length(1)); // Saturation
    constraints.push(Constraint::Length(1)); // Gamma
    if show_all_6 {
        constraints.push(Constraint::Length(1)); // Hue / Tint
        constraints.push(Constraint::Length(1)); // Sharpness
    }
    if show_dividers {
        constraints.push(Constraint::Length(1)); // Divider
    }
    constraints.push(Constraint::Length(1)); // Curve & Scope Header
    constraints.push(Constraint::Min(3));    // Split Braille Curve + Grade Scope (absorbs remaining height!)
    if show_geometry {
        if show_dividers {
            constraints.push(Constraint::Length(1)); // Divider
        }
        constraints.push(Constraint::Length(1)); // Geometry Row 1: Zoom + Aspect
        constraints.push(Constraint::Length(1)); // Geometry Row 2: Panscan, Rotate, Mirror, Reset
    }
    constraints.push(Constraint::Length(1)); // Footer actions: Save Custom, A/B Bypass, Reset EQ

    let rects = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let active_preset = state.config.picture.preset;
    let zoom_factor = 2.0_f64.powf(state.live_playback.video_zoom);
    let panscan_on = state.live_playback.panscan > 0.25;
    let aspect_cur = state.live_playback.aspect_override.clone();
    let rotate_deg = state.live_playback.video_rotate;
    let mirror_on = state.live_playback.mirror_x;
    let w = inner.width as usize;

    let mut r_idx = 0usize;
    let mut next_rect = || -> Rect {
        let r = rects[r_idx];
        r_idx += 1;
        r
    };

    let hits = &mut state.live_playback.play_hitboxes;

    // 1. Preset Header + Description
    let p_hdr_r = next_rect();
    let p_title = format!(" Presets [g]: {}", active_preset.label());
    let p_desc_budget = w.saturating_sub(p_title.len() + 3);
    let p_desc = truncate_words(active_preset.description(), p_desc_budget);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                p_title,
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!(" · {p_desc}"), Style::default().fg(theme.text_dim)),
        ])),
        p_hdr_r,
    );

    // Preset Pills Row 1: Neutral, Cinema, OLED, Daylight
    let p_r1 = next_rect();
    let row1_presets = [
        PicturePreset::Neutral,
        PicturePreset::Cinema,
        PicturePreset::OledPunch,
        PicturePreset::Daylight,
    ];
    let r1_pills: Vec<(String, Style, PlayOptionsHit)> = row1_presets
        .into_iter()
        .map(|p| {
            let act = active_preset == p;
            (
                format!("[{}]", p.short_label()),
                if act {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
                PlayOptionsHit::SelectPicturePreset(p),
            )
        })
        .collect();
    render_inline_pills(frame, p_r1, Some((" ", Style::default())), &r1_pills, hits);

    // Preset Pills Row 2: Night, Noir, Custom + Bypass
    let p_r2 = next_rect();
    let row2_presets = [
        PicturePreset::NightWarm,
        PicturePreset::NoirBw,
        PicturePreset::Custom,
    ];
    let mut r2_pills: Vec<(String, Style, PlayOptionsHit)> = row2_presets
        .into_iter()
        .map(|p| {
            let act = active_preset == p;
            (
                format!("[{}]", p.short_label()),
                if act {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
                PlayOptionsHit::SelectPicturePreset(p),
            )
        })
        .collect();
    r2_pills.push((
        if bypass {
            "[b Bypass:ON]".to_string()
        } else {
            "[b Bypass:OFF]".to_string()
        },
        if bypass {
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(251, 191, 36))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.accent)
        },
        PlayOptionsHit::TogglePictureBypass,
    ));
    render_inline_pills(frame, p_r2, Some((" ", Style::default())), &r2_pills, hits);

    if show_dividers {
        render_divider_row(frame, next_rect(), &theme);
    }

    // 2. 6-Channel Equalizer Sliders
    let sliders: Vec<(&str, &'static str, i32, i32, i32)> = if show_all_6 {
        vec![
            ("Brightness", "brightness", bri as i32, -50, 50),
            ("Contrast", "contrast", con as i32, -50, 50),
            ("Saturation", "saturation", sat as i32, -100, 50),
            ("Gamma", "gamma", gam as i32, -50, 50),
            ("Hue / Tint", "hue", hue as i32, -50, 50),
            ("Sharpness", "sharpen", shp as i32, -50, 50),
        ]
    } else {
        vec![
            ("Brightness", "brightness", bri as i32, -50, 50),
            ("Contrast", "contrast", con as i32, -50, 50),
            ("Saturation", "saturation", sat as i32, -100, 50),
            ("Gamma", "gamma", gam as i32, -50, 50),
        ]
    };

    for (idx, (lbl, prop, val, min_v, max_v)) in sliders.into_iter().enumerate() {
        let sl_r = next_rect();
        render_bipolar_slider_range(
            frame,
            sl_r,
            lbl,
            val,
            min_v,
            max_v,
            focused && eq_cursor == idx,
            PlayOptionsHit::SetPictureRatio(prop),
            PlayOptionsHit::PictureDelta(prop, -5),
            PlayOptionsHit::PictureDelta(prop, 5),
            hits,
            &theme,
        );
    }

    if show_dividers {
        render_divider_row(frame, next_rect(), &theme);
    }

    // 3. Split Live Luma Transfer Curve + Grade Scope
    let scope_hdr_r = next_rect();
    let scope_cols_hdr = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(scope_hdr_r);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Live Luma Transfer Curve",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        scope_cols_hdr[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            "│ Grade Scope",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        scope_cols_hdr[1],
    );

    let curve_area = next_rect();
    let curve_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(curve_area);

    let (eff_bri, eff_con, eff_gam) = if bypass { (0, 0, 0) } else { (bri, con, gam) };
    let curve_lines = render_tone_curve_lines(
        eff_bri,
        eff_con,
        eff_gam,
        curve_cols[0].width as usize,
        curve_cols[0].height as usize,
        &theme,
    );
    frame.render_widget(Paragraph::new(curve_lines), curve_cols[0]);

    let (blk_pct, mid_pct, wht_pct) = compute_luma_scope(eff_bri, eff_con, eff_gam);
    let wide_scope = curve_cols[1].width >= 21;
    let mut scope_lines = vec![
        Line::from(vec![
            Span::styled(
                if wide_scope { "│ Blk Floor: " } else { "│ Blk : " },
                Style::default().fg(theme.text_dim),
            ),
            Span::styled(format!("{blk_pct:>4.1}%"), Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled(
                if wide_scope { "│ Midtone  : " } else { "│ Mid : " },
                Style::default().fg(theme.text_dim),
            ),
            Span::styled(format!("{mid_pct:>4.1}%"), Style::default().fg(theme.primary)),
        ]),
        Line::from(vec![
            Span::styled(
                if wide_scope { "│ Wht Clip : " } else { "│ Wht : " },
                Style::default().fg(theme.text_dim),
            ),
            Span::styled(
                format!("{wht_pct:>4.1}%"),
                Style::default().fg(if wht_pct >= 99.9 {
                    Color::Rgb(251, 191, 36)
                } else {
                    Color::Rgb(52, 211, 153)
                }),
            ),
        ]),
    ];
    if curve_cols[1].height >= 4 {
        scope_lines.push(Line::from(vec![
            Span::styled(
                if wide_scope { "│ Hue/Shp  : " } else { "│ H/S : " },
                Style::default().fg(theme.text_dim),
            ),
            Span::styled(
                format!("{hue:+} / {shp:+}"),
                Style::default().fg(theme.secondary),
            ),
        ]));
    }
    if curve_cols[1].height >= 5 {
        scope_lines.push(Line::from(vec![
            Span::styled(
                if wide_scope { "│ State    : " } else { "│ Eq  : " },
                Style::default().fg(theme.text_dim),
            ),
            Span::styled(
                if bypass {
                    "BYPASSED"
                } else {
                    active_preset.short_label()
                },
                Style::default()
                    .fg(if bypass {
                        Color::Rgb(251, 191, 36)
                    } else {
                        Color::Rgb(52, 211, 153)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(scope_lines), curve_cols[1]);

    // 4. Zero-Copy GPU Framing & Geometry Deck
    if show_geometry {
        if show_dividers {
            render_divider_row(frame, next_rect(), &theme);
        }

        let geom_r1 = next_rect();
        let zoom_lbl = format!("{zoom_factor:.2}x");
        let mut g1_pills = vec![
            ("[-]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::ZoomDelta(-0.10)),
            (zoom_lbl, Style::default().fg(theme.text).add_modifier(Modifier::BOLD), PlayOptionsHit::ResetGeometry),
            ("[+]".to_string(), Style::default().fg(theme.secondary), PlayOptionsHit::ZoomDelta(0.10)),
        ];
        for (lbl, val) in [("Auto", "-1"), ("16:9", "16:9"), ("21:9", "21:9"), ("4:3", "4:3")] {
            let act = aspect_cur == val;
            g1_pills.push((
                format!("[{lbl}]"),
                if act {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.secondary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim)
                },
                PlayOptionsHit::SetAspectOverride(val),
            ));
        }
        let g1_pfx = if w >= 48 { " Zoom:" } else { " Zm:" };
        render_inline_pills(
            frame,
            geom_r1,
            Some((g1_pfx, Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD))),
            &g1_pills,
            hits,
        );

        let geom_r2 = next_rect();
        let g2_pills = vec![
            (
                if panscan_on {
                    "[Pan:Fill]".to_string()
                } else {
                    "[Pan:OFF]".to_string()
                },
                Style::default().fg(if panscan_on { theme.primary } else { theme.text }),
                PlayOptionsHit::TogglePanscan,
            ),
            (
                format!("[Rot:{}°]", rotate_deg),
                Style::default().fg(if rotate_deg != 0 { theme.primary } else { theme.text }),
                PlayOptionsHit::CycleRotate,
            ),
            (
                format!("[Flip:{}]", if mirror_on { "ON" } else { "OFF" }),
                Style::default().fg(if mirror_on { theme.primary } else { theme.text }),
                PlayOptionsHit::ToggleMirrorX,
            ),
            (
                "[Reset Geom]".to_string(),
                Style::default().fg(theme.accent),
                PlayOptionsHit::ResetGeometry,
            ),
        ];
        render_inline_pills(frame, geom_r2, Some((" ", Style::default())), &g2_pills, hits);
    }

    // 5. Footer Actions (Save Custom, Bypass, Reset EQ)
    let foot_r = next_rect();
    let foot_pills = vec![
        (
            "[Save Custom]".to_string(),
            Style::default()
                .fg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::SaveCustomPicturePreset,
        ),
        (
            if bypass {
                "[b Resume Grade]".to_string()
            } else {
                "[b A/B Bypass]".to_string()
            },
            Style::default().fg(theme.secondary),
            PlayOptionsHit::TogglePictureBypass,
        ),
        (
            "[0 Reset EQ]".to_string(),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
            PlayOptionsHit::ResetPicture,
        ),
    ];
    render_inline_pills(frame, foot_r, Some((" ", Style::default())), &foot_pills, hits);

    push_pane_hit(hits, area, 1);
}

fn render_quick_presets_strip(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" Quick Engine Mode & Pipeline Presets ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 30 || inner.height == 0 {
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10), // " Mode[m]: "
            Constraint::Length(13), // ECO
            Constraint::Length(13), // BAL
            Constraint::Length(13), // MAX
            Constraint::Length(13), // MAN
            Constraint::Min(14),    // Shader + Net pills
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Span::styled(
            " Mode[m]: ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        cols[0],
    );

    let hints = ["ECO ~350M", "BAL ~650M", "MAX ~1.4G", "MAN Custom"];
    let hits = &mut state.live_playback.play_hitboxes;
    for (idx, pm) in PerformanceMode::ALL.iter().enumerate() {
        let active = *pm == state.config.performance_mode;
        let col = mode_badge_color(*pm);
        let lbl = format!(" {} ", hints[idx]);
        frame.render_widget(
            Paragraph::new(Span::styled(
                lbl,
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(col)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim)
                },
            )),
            cols[idx + 1],
        );
        hits.push((cols[idx + 1], PlayOptionsHit::SetPerformanceMode(*pm)));
    }

    let right_txt = format!(
        " Shader[s]: {} | {}",
        state.config.shader_mode.short_label(),
        state.config.network_mode.short_badge()
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_words(&right_txt, cols[5].width as usize),
            Style::default().fg(theme.secondary),
        )),
        cols[5],
    );
}
