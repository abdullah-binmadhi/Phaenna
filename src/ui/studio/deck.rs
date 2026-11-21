//! Page 1 — `[1] Deck`: Overview Bento KPIs, Speed Rail, 150% Volume Fader,
//! Session Actions, and Visual Picture Equalizer + Live Tone Curve.

use crate::config::{PerformanceMode, Quality};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    format_speed_bps, mode_badge_color, push_pane_hit, render_bento_kpi_card,
    render_bipolar_slider, render_tone_curve_lines, section_block, sparkline_string,
    truncate_words,
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
            state.config.effective_mastering().codec_priority.label().to_string()
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
        format!("{spark} {}", format_speed_bps(state.live_playback.cache_speed_bps))
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

    // Card 3: GPU Mastering
    let pm = state.config.performance_mode;
    let master_head = format!(
        "{} · {}",
        pm.short_badge(),
        state.config.shader_mode.short_label()
    );
    let master_sub = format!(
        " {} · Drp:{}",
        state.config.motion_mode.short_badge(),
        state.live_playback.dropped_frames
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

    // Card 4: Audio & Guard
    let aud_head = if state.config.dub_preference.is_dub_on() {
        format!("DUB ON ({})", state.config.dub_lang.to_uppercase())
    } else {
        "Original Audio [ORIG]".to_string()
    };
    let sub_lbl = if state.live_playback.sub_visible { "ON" } else { "OFF" };
    let guard_sub = format!(
        " Subs:{} · SB:{} ({})",
        sub_lbl,
        state.config.sponsorblock_mode.label(),
        state.sponsor_skips_session
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

fn render_transport_session_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" Transport, Volume & Session ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 22 || inner.height < 8 {
        return;
    }

    let live = state.has_live_ipc();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0: Speed header
            Constraint::Length(1), // 1: Speed buttons
            Constraint::Length(1), // 2: Spacer
            Constraint::Length(1), // 3: Volume header
            Constraint::Length(1), // 4: Visual volume fader
            Constraint::Length(1), // 5: Mute + Loudnorm
            Constraint::Length(1), // 6: Spacer
            Constraint::Length(1), // 7: Window OnTop / Loop
            Constraint::Length(1), // 8: Audio Radio
            Constraint::Length(1), // 9: Launch / Relaunch
            Constraint::Min(1),    // 10: Stop Active Player
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

    // Speed rail
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Playback Speed  ( [ / ] or Click )",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[0],
    );
    let speeds = [0.50_f64, 0.75, 1.0, 1.25, 1.5, 2.0];
    let mut spd_cols = vec![Constraint::Length(1)];
    for _ in speeds {
        spd_cols.push(Constraint::Min(6));
    }
    let sp_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(spd_cols)
        .split(rows[1]);
    for (idx, spd) in speeds.iter().enumerate() {
        let active = (state.live_playback.speed - *spd).abs() < 0.04;
        let lbl = if (*spd - 2.0).abs() < 0.01 {
            "[2.0x]".to_string()
        } else {
            format!("[{spd:.2}]")
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                lbl,
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            )),
            sp_rects[idx + 1],
        );
        hits.push((sp_rects[idx + 1], PlayOptionsHit::SetSpeed(*spd)));
    }

    // Visual Volume Fader (0%..150% with unity tick at 100%)
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Master Volume   ( - / + or Click Bar )",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[3],
    );
    let vol_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(4), // " [-]"
            Constraint::Min(12),   // Bar
            Constraint::Length(7), // " 100% "
            Constraint::Length(4), // "[+] "
        ])
        .split(rows[4]);
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

    // Row 5: Mute + Night Loudnorm
    let r5_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(rows[5]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.muted {
                " [MUTE: ON ]"
            } else {
                " [MUTE: OFF]"
            },
            if state.live_playback.muted {
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(239, 68, 68))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.accent)
            },
        )),
        r5_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.config.audio_loudnorm {
                "Night Loudnorm: [ON] [n]"
            } else {
                "Night Loudnorm: [OFF] [n]"
            },
            Style::default().fg(if state.config.audio_loudnorm {
                Color::Rgb(52, 211, 153)
            } else {
                theme.text_dim
            }),
        )),
        r5_cols[1],
    );
    hits.push((r5_cols[0], PlayOptionsHit::ToggleMute));
    hits.push((r5_cols[1], PlayOptionsHit::ToggleLoudnorm));

    // Row 7: OnTop / Loop
    let r7_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[7]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.ontop {
                " [OnTop: ON]"
            } else {
                " [OnTop: OFF]"
            },
            Style::default().fg(if state.live_playback.ontop {
                theme.primary
            } else {
                theme.text
            }),
        )),
        r7_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.loop_video {
                " [Loop File: ON]"
            } else {
                " [Loop File: OFF]"
            },
            Style::default().fg(if state.live_playback.loop_video {
                theme.primary
            } else {
                theme.text
            }),
        )),
        r7_cols[1],
    );
    hits.push((r7_cols[0], PlayOptionsHit::ToggleOnTop));
    hits.push((r7_cols[1], PlayOptionsHit::ToggleLoop));

    // Row 8: Audio Radio
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [R] Audio Radio ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("(Low-Power Stream)", Style::default().fg(theme.text_dim)),
        ])),
        rows[8],
    );
    hits.push((rows[8], PlayOptionsHit::LaunchAudioRadio));

    // Row 9: Relaunch / Launch
    frame.render_widget(
        Paragraph::new(Span::styled(
            if live {
                " [Enter] Relaunch / Apply Flags"
            } else {
                " [Enter] Launch Video Stream Now"
            },
            Style::default()
                .fg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        )),
        rows[9],
    );
    hits.push((rows[9], PlayOptionsHit::LaunchOrRelaunch));

    // Row 10: Stop Active Player
    frame.render_widget(
        Paragraph::new(Span::styled(
            " [X] Stop Active Player",
            Style::default()
                .fg(Color::Rgb(239, 68, 68))
                .add_modifier(Modifier::BOLD),
        )),
        rows[10],
    );
    hits.push((rows[10], PlayOptionsHit::StopPlayer));
    push_pane_hit(hits, area, 0);
}

fn render_picture_eq_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Picture Equalizer & Live Tone Curve ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 8 {
        return;
    }

    let eq_cursor = state.studio.eq_cursor;
    let bri = state.live_playback.brightness;
    let con = state.live_playback.contrast;
    let sat = state.live_playback.saturation;
    let gam = state.live_playback.gamma;

    let curve_rows = inner.height.saturating_sub(7).clamp(3, 6);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),          // 0: Brightness
            Constraint::Length(1),          // 1: Contrast
            Constraint::Length(1),          // 2: Saturation
            Constraint::Length(1),          // 3: Gamma
            Constraint::Length(1),          // 4: Curve header
            Constraint::Length(curve_rows), // 5: Tone curve Braille canvas
            Constraint::Min(1),             // 6: Reset EQ button
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let sliders = [
        ("Brightness", "brightness", bri as i32),
        ("Contrast", "contrast", con as i32),
        ("Saturation", "saturation", sat as i32),
        ("Gamma", "gamma", gam as i32),
    ];
    for (idx, (lbl, prop, val)) in sliders.into_iter().enumerate() {
        render_bipolar_slider(
            frame,
            rows[idx],
            lbl,
            val,
            50,
            focused && eq_cursor == idx,
            PlayOptionsHit::SetPictureRatio(prop),
            PlayOptionsHit::PictureDelta(prop, -5),
            PlayOptionsHit::PictureDelta(prop, 5),
            hits,
            &theme,
        );
    }

    frame.render_widget(
        Paragraph::new(Span::styled(
            " Live Luma Transfer Curve (vs 1:1 Neutral)",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        rows[4],
    );

    let curve_lines = render_tone_curve_lines(
        bri,
        con,
        gam,
        rows[5].width as usize,
        curve_rows as usize,
        &theme,
    );
    frame.render_widget(Paragraph::new(curve_lines), rows[5]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                " [0 Reset Picture EQ to Neutral]",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("  (Right-click slider = -5)", Style::default().fg(theme.text_dim)),
        ])),
        rows[6],
    );
    hits.push((rows[6], PlayOptionsHit::ResetPicture));
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
