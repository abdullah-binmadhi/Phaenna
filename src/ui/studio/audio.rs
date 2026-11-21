//! Page 4 — `[4] Audio & Sync`: Full-Height Audio Track Explorer Table,
//! Dubbing Engine & 3x3 Language Grid, Night Loudnorm Curve, and A/V Sync Console.

use crate::config::{dub_lang_label, DUB_LANGUAGES};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    push_line_hit, push_pane_hit, render_bipolar_slider, render_braille_dual_chart_lines,
    section_block, truncate_words,
};
use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render_audio_page(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 32 || area.height < 12 {
        return;
    }
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(area);

    render_track_explorer_pane(frame, cols[0], state);

    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
        .split(cols[1]);

    render_dub_and_loudnorm_pane(frame, right_rows[0], state);
    render_avsync_console_pane(frame, right_rows[1], state);
}

fn render_track_explorer_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let total_live = state.live_playback.audio_tracks.len();
    let title = if total_live > 0 {
        format!(" Live Audio Track Explorer ({total_live} Tracks) ")
    } else {
        " Audio Track & Stream Explorer ".to_string()
    };
    let blk = section_block(&title, focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 22 || inner.height < 5 {
        return;
    }

    let vid_opt = state
        .active_playback_video_id
        .clone()
        .or_else(|| state.active_selected_video().map(|v| v.id));
    let variants = vid_opt
        .as_ref()
        .and_then(|id| {
            let clean = id.trim_start_matches("RD");
            state
                .stream_ladder_cache
                .get(id)
                .or_else(|| state.stream_ladder_cache.get(clean))
        })
        .map(|l| l.audio_variants.clone())
        .unwrap_or_default();

    let hits = &mut state.live_playback.play_hitboxes;
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  ID   Lang    Role    Profile / Codec        State",
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        Style::default().fg(theme.divider),
    )));

    if total_live > 0 {
        let visible_cap = (inner.height as usize).saturating_sub(4).max(1);
        let max_scroll = total_live.saturating_sub(visible_cap);
        let scroll = state.live_playback.audio_track_scroll.min(max_scroll);

        for tr in state
            .live_playback
            .audio_tracks
            .iter()
            .skip(scroll)
            .take(visible_cap)
            .cloned()
        {
            let active = state.live_playback.active_aid == Some(tr.id) || tr.selected;
            let row = lines.len() as u16;
            let lang = tr.lang.as_deref().unwrap_or("und");
            let role = if tr.is_dubbed {
                "[DUB] "
            } else if tr.is_original {
                "[ORIG]"
            } else {
                "[TRK] "
            };
            let clean = tr
                .display_label()
                .replace(&format!("[{}] ", tr.id), "")
                .replace(" [DUB]", "")
                .replace(" [ORIG]", "");
            let title_w = inner.width.saturating_sub(31).max(8) as usize;
            let title_fit = truncate_words(&clean, title_w);
            let state_lbl = if active { "[ACT]" } else { "Ready" };
            let st = if active {
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            lines.push(Line::from(Span::styled(
                format!(
                    " {} {:>2}  {:<6}  {}  {:<title_w$}  {}",
                    if active { ">" } else { " " },
                    tr.id,
                    truncate_words(lang, 6),
                    role,
                    title_fit,
                    state_lbl
                ),
                st,
            )));
            push_line_hit(hits, inner, row, PlayOptionsHit::SelectAudioTrack(tr.id));
        }

        while (lines.len() as u16) + 2 < inner.height {
            lines.push(Line::from(""));
        }
        if (lines.len() as u16) + 1 < inner.height {
            let shown_end = (scroll + visible_cap).min(total_live);
            let foot_row = lines.len() as u16;
            lines.push(Line::from(Span::styled(
                format!(
                    " Showing {}-{} of {} tracks  [Click / Scroll to Browse]",
                    scroll + 1,
                    shown_end,
                    total_live
                ),
                Style::default().fg(theme.text_dim),
            )));
            push_line_hit(hits, inner, foot_row, PlayOptionsHit::ScrollAudioTracks(1));
        }
    } else if variants.is_empty() {
            lines.push(Line::from(Span::styled(
                "  Audio tracks populate automatically via mpv IPC on launch.",
                Style::default().fg(theme.text_dim),
            )));
            lines.push(Line::from(Span::styled(
                format!(
                    "  Current Policy: {} | Multi-Track: {}",
                    state.config.dub_preference.label(),
                    if state.config.audio_multistreams { "ON" } else { "OFF" }
                ),
                Style::default().fg(theme.text),
            )));
        } else {
            let cap = (inner.height as usize).saturating_sub(3).max(1);
            for (idx, av) in variants.iter().take(cap).enumerate() {
                let lang = av.lang.as_deref().unwrap_or("und");
                let role = if av.is_dubbed {
                    "[DUB] "
                } else if av.is_original {
                    "[ORIG]"
                } else {
                    "[TRK] "
                };
                let drc = if av.is_drc { "DRC " } else { "" };
                let desc = format!("{}{} {:.0}k", drc, av.acodec, av.abr_kbps);
                let desc_w = inner.width.saturating_sub(28).max(8) as usize;
                lines.push(Line::from(Span::styled(
                    format!(
                        "   {:>2}  {:<6}  {}  {:<desc_w$}  Ready",
                        idx + 1,
                        truncate_words(lang, 6),
                        role,
                        truncate_words(&desc, desc_w),
                    ),
                    Style::default().fg(theme.text),
                )));
            }
        }

    push_pane_hit(hits, area, 0);
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_dub_and_loudnorm_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Dubbing Engine & Night Loudnorm ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 6 {
        return;
    }

    let dub_on = state.config.dub_preference.is_dub_on();
    let hits = &mut state.live_playback.play_hitboxes;

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0: Dubbed Audio toggle
            Constraint::Length(1), // 1: Multi-Track toggle
            Constraint::Length(1), // 2: Lang Grid label
            Constraint::Length(3), // 3: 3x3 Language Grid
            Constraint::Length(1), // 4: Night Loudnorm toggle
            Constraint::Min(1),    // 5: Compression curve hint
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Dubbed Audio [d] : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                state.config.dub_preference.label(),
                Style::default()
                    .fg(if dub_on {
                        Color::Rgb(251, 191, 36)
                    } else {
                        Color::Rgb(52, 211, 153)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[0],
    );
    hits.push((rows[0], PlayOptionsHit::ToggleDub));

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Multi-Track Load : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.audio_multistreams {
                    "ON (Instant Hot-Switch)"
                } else {
                    "OFF (Single Stream)"
                },
                Style::default().fg(if state.config.audio_multistreams {
                    theme.primary
                } else {
                    theme.text_dim
                }),
            ),
        ])),
        rows[1],
    );
    hits.push((rows[1], PlayOptionsHit::ToggleMultistreams));

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Target Dub Lang [L]: ", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)),
            Span::styled(
                dub_lang_label(&state.config.dub_lang),
                Style::default().fg(theme.text),
            ),
        ])),
        rows[2],
    );

    // 3x3 clickable DUB_LANGUAGES grid
    let grid_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
        .split(rows[3]);
    for r in 0..3 {
        let g_cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(33),
                Constraint::Percentage(33),
                Constraint::Percentage(34),
            ])
            .split(grid_rows[r]);
        for c in 0..3 {
            let idx = r * 3 + c;
            if let Some(&(code, full_lbl)) = DUB_LANGUAGES.get(idx) {
                let active = state.config.dub_lang == code;
                let short_name = full_lbl.split(" (").next().unwrap_or(full_lbl);
                let pill = format!(" [{code}] {short_name}");
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        truncate_words(&pill, g_cols[c].width as usize),
                        if active {
                            Style::default()
                                .fg(Color::Black)
                                .bg(theme.secondary)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(theme.text_dim)
                        },
                    )),
                    g_cols[c],
                );
                hits.push((g_cols[c], PlayOptionsHit::SelectDubLang(code.to_string())));
            }
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Night Loudnorm [n]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.audio_loudnorm {
                    "ON  (dynaudnorm f=150 active)"
                } else {
                    "OFF (Bit-Exact Unprocessed)"
                },
                Style::default()
                    .fg(if state.config.audio_loudnorm {
                        Color::Rgb(52, 211, 153)
                    } else {
                        theme.text
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[4],
    );
    hits.push((rows[4], PlayOptionsHit::ToggleLoudnorm));

    let norm_curve = if state.config.audio_loudnorm {
        " Dynamics: ⣀⠔⠒⠒⠉⠉  Quiet dialogue boosted · Sudden peaks gently clamped"
    } else {
        " Dynamics: ⣀⡠⠔⠊⠉    Linear 1:1 studio dynamic range (Zero filter gain)"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_words(norm_curve, rows[5].width as usize),
            Style::default().fg(if state.config.audio_loudnorm {
                Color::Rgb(52, 211, 153)
            } else {
                theme.text_dim
            }),
        )),
        rows[5],
    );
    push_pane_hit(hits, area, 1);
}

fn render_avsync_console_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" A/V Sync & Audio Delay Console ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 4 {
        return;
    }

    let chart_h = inner.height.saturating_sub(3).clamp(2, 5);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),       // 0: Quick step buttons
            Constraint::Length(1),       // 1: Interactive bipolar delay bar
            Constraint::Length(1),       // 2: Drift status line
            Constraint::Length(chart_h), // 3: A/V sync drift chart
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

    // Row 0: Quick step buttons
    let btn_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(14),
            Constraint::Length(7),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(6),
            Constraint::Min(7),
        ])
        .split(rows[0]);
    frame.render_widget(
        Paragraph::new(Span::styled(" Quick Offset:", Style::default().fg(theme.text_dim))),
        btn_cols[0],
    );
    let steps = [
        ("[-200]", PlayOptionsHit::AudioDelayDelta(-200)),
        ("[-50]", PlayOptionsHit::AudioDelayDelta(-50)),
        ("[ 0 ]", PlayOptionsHit::SetAudioDelay(0)),
        ("[+50]", PlayOptionsHit::AudioDelayDelta(50)),
        ("[+200]", PlayOptionsHit::AudioDelayDelta(200)),
    ];
    for (idx, (lbl, hit)) in steps.into_iter().enumerate() {
        frame.render_widget(
            Paragraph::new(Span::styled(lbl, Style::default().fg(theme.secondary))),
            btn_cols[idx + 1],
        );
        hits.push((btn_cols[idx + 1], hit));
    }

    // Row 1: Bipolar Delay Slider (-500..+500ms)
    render_bipolar_slider(
        frame,
        rows[1],
        "Delay (ms)",
        state.live_playback.audio_delay_ms,
        500,
        false,
        PlayOptionsHit::SetAudioDelayRatio,
        PlayOptionsHit::AudioDelayDelta(-50),
        PlayOptionsHit::AudioDelayDelta(50),
        hits,
        &theme,
    );

    // Row 2 & 3: Live A/V Drift Sparkline
    let av_samples: Vec<f64> = state
        .live_playback
        .telemetry_history
        .avsync_ms
        .iter()
        .map(|v| v.abs())
        .collect();
    let cur_ms = state.live_playback.avsync_ms;
    let peak_ms = av_samples.iter().copied().fold(0.0_f64, f64::max);
    let sync_ok = cur_ms.abs() <= 25.0;
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Live A/V Drift (60s): ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!("Cur:{cur_ms:+.1}ms Peak:{peak_ms:.1}ms "),
                Style::default().fg(theme.text),
            ),
            Span::styled(
                if sync_ok { "[SYNC LOCK]" } else { "[DRIFT]" },
                Style::default()
                    .fg(if sync_ok {
                        Color::Rgb(52, 211, 153)
                    } else {
                        Color::Rgb(251, 191, 36)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[2],
    );

    if chart_h >= 2 {
        let lines = render_braille_dual_chart_lines(
            &av_samples,
            None,
            None,
            rows[3].width as usize,
            chart_h as usize,
            40.0,
            "m",
            if sync_ok {
                Color::Rgb(52, 211, 153)
            } else {
                Color::Rgb(251, 191, 36)
            },
            theme.secondary,
            &theme,
        );
        frame.render_widget(Paragraph::new(lines), rows[3]);
    }
}
