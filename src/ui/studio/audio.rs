//! Page 4 — `[4] Audio & Sync`: Track Explorer + 5-Band EQ & Signal Routing Deck,
//! Dubbing Engine & Braille Night Loudnorm Transfer Curve, and A/V Sync Console.

use crate::config::{dub_lang_label, AudioChannelMode, AudioEqPreset, DUB_LANGUAGES};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    push_line_hit, push_pane_hit, render_bipolar_slider, render_braille_dual_chart_lines,
    render_inline_pills, render_loudnorm_curve_lines, section_block, truncate_words,
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

    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(46), Constraint::Percentage(54)])
        .split(cols[0]);

    render_track_explorer_pane(frame, left_rows[0], state);
    render_audio_eq_deck(frame, left_rows[1], state);

    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(56), Constraint::Percentage(44)])
        .split(cols[1]);

    render_dub_and_loudnorm_pane(frame, right_rows[0], state);
    render_avsync_console_pane(frame, right_rows[1], state);
}

fn pill_style(active: bool, bg: Color, idle_fg: Color) -> Style {
    if active {
        Style::default()
            .fg(Color::Black)
            .bg(bg)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(idle_fg)
    }
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
    if inner.width < 22 || inner.height < 4 {
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
        let visible_cap = (inner.height as usize).saturating_sub(3).max(1);
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

        if (lines.len() as u16) < inner.height {
            let shown_end = (scroll + visible_cap).min(total_live);
            let foot_row = lines.len() as u16;
            lines.push(Line::from(Span::styled(
                format!(
                    " {}",
                    truncate_words(
                        &format!(
                            "Showing {}-{} of {} tracks [Click / Scroll]",
                            scroll + 1,
                            shown_end,
                            total_live
                        ),
                        inner.width.saturating_sub(1) as usize,
                    )
                ),
                Style::default().fg(theme.text_dim),
            )));
            push_line_hit(hits, inner, foot_row, PlayOptionsHit::ScrollAudioTracks(1));
        }
    } else if variants.is_empty() {
        lines.push(Line::from(Span::styled(
            "  Audio tracks populate via mpv IPC on playback launch.",
            Style::default().fg(theme.text_dim),
        )));
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                truncate_words(
                    &format!(
                        "Policy: {} | Multi-Track: {}",
                        state.config.dub_preference.label(),
                        if state.config.audio_multistreams { "ON" } else { "OFF" }
                    ),
                    inner.width.saturating_sub(2) as usize,
                )
            ),
            Style::default().fg(theme.text),
        )));
    } else {
        let cap = (inner.height as usize).saturating_sub(2).max(1);
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

fn render_audio_eq_deck(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let blk = section_block(" 5-Band Audio EQ & Signal Routing [e] ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 6 {
        return;
    }

    let eq = state.config.audio_eq_preset;
    let ch_mode = state.config.audio_channel_mode;
    let hp_on = state.config.audio_voice_highpass;
    let loudnorm_on = state.config.audio_loudnorm;
    let gains = eq.band_gains_db();
    let meter_rows: u16 = if inner.height >= 10 { 3 } else { 2 };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),          // 0: EQ preset pills row 1
            Constraint::Length(1),          // 1: EQ preset pills row 2
            Constraint::Length(1),          // 2: Band dB values
            Constraint::Length(meter_rows), // 3: Vertical EQ bars
            Constraint::Length(1),          // 4: Band freq labels
            Constraint::Length(1),          // 5: Routing pills (Stereo/Mono + Highpass)
            Constraint::Min(1),             // 6: Filter chain status
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

    let r0_presets = [
        AudioEqPreset::Flat,
        AudioEqPreset::VoiceClarity,
        AudioEqPreset::Podcast,
    ]
    .into_iter()
    .map(|p| {
        (
            format!("[{}]", p.short_label()),
            pill_style(eq == p, theme.primary, theme.text_dim),
            PlayOptionsHit::SelectAudioEq(p),
        )
    })
    .collect::<Vec<_>>();
    render_inline_pills(
        frame,
        rows[0],
        Some((
            " EQ [e]:",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        &r0_presets,
        hits,
    );

    let r1_presets = [
        AudioEqPreset::BassBoost,
        AudioEqPreset::TrebleCrisp,
        AudioEqPreset::LateNight,
    ]
    .into_iter()
    .map(|p| {
        (
            format!("[{}]", p.short_label()),
            pill_style(eq == p, theme.accent, theme.text_dim),
            PlayOptionsHit::SelectAudioEq(p),
        )
    })
    .collect::<Vec<_>>();
    render_inline_pills(
        frame,
        rows[1],
        Some(("        ", Style::default().fg(theme.text_dim))),
        &r1_presets,
        hits,
    );

    // 5-band Equalizer Visual Spectrum Meters
    let col_w = (rows[2].width as usize / 5).max(6);
    let mut db_spans = Vec::new();
    let mut freq_spans = Vec::new();
    const FREQS: [&str; 5] = ["60Hz", "250Hz", "1kHz", "4kHz", "12kHz"];
    for (g, f) in gains.into_iter().zip(FREQS) {
        let g_txt = format!("{g:+}dB");
        let g_col = if g > 0 {
            Color::Rgb(52, 211, 153)
        } else if g < 0 {
            Color::Rgb(251, 191, 36)
        } else {
            theme.text_dim
        };
        db_spans.push(Span::styled(
            format!("{g_txt:^col_w$}"),
            Style::default().fg(g_col).add_modifier(Modifier::BOLD),
        ));
        freq_spans.push(Span::styled(
            format!("{f:^col_w$}"),
            Style::default().fg(theme.text_dim),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(db_spans)), rows[2]);

    let bar_lines: Vec<Line<'static>> = (0..meter_rows)
        .map(|mr| {
            let level_threshold = (meter_rows - 1 - mr) as i8; // top row highest, bottom lowest
            let mut spans = Vec::new();
            for g in gains {
                // Map -6..+6dB -> height levels across `meter_rows`
                let filled_levels = (((g + 6).clamp(0, 12) as f32 / 12.0) * (meter_rows as f32))
                    .round() as i8;
                let active_cell = filled_levels > level_threshold;
                let glyph = if active_cell { "████" } else { "░░░░" };
                let fg = if !active_cell {
                    theme.divider
                } else if g > 0 {
                    Color::Rgb(52, 211, 153)
                } else if g < 0 {
                    Color::Rgb(251, 191, 36)
                } else {
                    theme.secondary
                };
                spans.push(Span::styled(format!("{glyph:^col_w$}"), Style::default().fg(fg)));
            }
            Line::from(spans)
        })
        .collect();
    frame.render_widget(Paragraph::new(bar_lines), rows[3]);
    frame.render_widget(Paragraph::new(Line::from(freq_spans)), rows[4]);

    // Channel & Voice Highpass Routing Pills
    let route_pills = vec![
        (
            format!("[{}]", ch_mode.short_label()),
            pill_style(
                ch_mode == AudioChannelMode::Mono,
                Color::Rgb(251, 191, 36),
                theme.text,
            ),
            PlayOptionsHit::ToggleAudioChannelMode,
        ),
        (
            format!("[HP 80Hz: {}]", if hp_on { "ON" } else { "OFF" }),
            pill_style(hp_on, Color::Rgb(52, 211, 153), theme.text_dim),
            PlayOptionsHit::ToggleAudioHighpass,
        ),
    ];
    render_inline_pills(
        frame,
        rows[5],
        Some((" Route :", Style::default().fg(theme.text_dim))),
        &route_pills,
        hits,
    );

    let filter_summary = if eq == AudioEqPreset::Flat
        && ch_mode == AudioChannelMode::Stereo
        && !hp_on
        && !loudnorm_on
    {
        "Filter: Bit-exact bypass (0 CPU overhead)".to_string()
    } else {
        format!(
            "Filter: {} · {} · HP:{} · Norm:{}",
            eq.short_label(),
            ch_mode.short_label(),
            if hp_on { "80Hz" } else { "OFF" },
            if loudnorm_on { "ON" } else { "OFF" }
        )
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", truncate_words(&filter_summary, rows[6].width.saturating_sub(1) as usize)),
            Style::default().fg(theme.secondary),
        )),
        rows[6],
    );

    push_pane_hit(hits, area, 0);
}

fn render_dub_and_loudnorm_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Dubbing Engine & Night Loudnorm ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 8 {
        return;
    }

    let dub_on = state.config.dub_preference.is_dub_on();
    let loudnorm_on = state.config.audio_loudnorm;
    let curve_rows = inner.height.saturating_sub(7).clamp(2, 4);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),          // 0: Dubbed Audio toggle
            Constraint::Length(1),          // 1: Multi-Track toggle
            Constraint::Length(1),          // 2: Lang Grid label
            Constraint::Length(3),          // 3: 3x3 Language Grid
            Constraint::Length(1),          // 4: Night Loudnorm toggle
            Constraint::Length(curve_rows), // 5: Braille compression transfer curve
            Constraint::Min(1),             // 6: Curve caption
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

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
            Span::styled(
                " Target Dub Lang [L]: ",
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
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
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
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
                let pill = format!("[{code}] {short_name}");
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        format!(" {}", truncate_words(&pill, g_cols[c].width.saturating_sub(1) as usize)),
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
                if loudnorm_on {
                    "ON  (dynaudnorm f=150 active)"
                } else {
                    "OFF (Bit-Exact Unprocessed)"
                },
                Style::default()
                    .fg(if loudnorm_on {
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

    if curve_rows >= 2 {
        let lines = render_loudnorm_curve_lines(
            loudnorm_on,
            rows[5].width as usize,
            curve_rows as usize,
            &theme,
        );
        frame.render_widget(Paragraph::new(lines), rows[5]);
    }

    let norm_caption = if loudnorm_on {
        "Transfer: dynaudnorm boosts soft speech & clamps explosive peaks"
    } else {
        "Transfer: Linear 1:1 studio dynamic range (Zero filter gain)"
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", truncate_words(norm_caption, rows[6].width.saturating_sub(1) as usize)),
            Style::default().fg(if loudnorm_on {
                Color::Rgb(52, 211, 153)
            } else {
                theme.text_dim
            }),
        )),
        rows[6],
    );
    push_pane_hit(hits, area, 1);
}

fn render_avsync_console_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let blk = section_block(" A/V Sync & Audio Delay Console ", false, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 5 {
        return;
    }

    let delay_ms = state.live_playback.audio_delay_ms;
    let show_device_presets = inner.height >= 6;
    let chart_h = inner
        .height
        .saturating_sub(if show_device_presets { 4 } else { 3 })
        .clamp(2, 5);

    let mut constraints = vec![Constraint::Length(1)]; // 0: Quick offset pills
    if show_device_presets {
        constraints.push(Constraint::Length(1)); // 1: Device latency presets
    }
    constraints.extend([
        Constraint::Length(1),       // Bipolar delay bar
        Constraint::Length(1),       // Drift status line
        Constraint::Length(chart_h), // A/V sync drift chart
    ]);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;
    let mut ri = 0usize;

    // Row 0: Quick step buttons via render_inline_pills (fixes `][` bug!)
    let offset_pills = vec![
        (
            "[-200]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::AudioDelayDelta(-200),
        ),
        (
            "[-50]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::AudioDelayDelta(-50),
        ),
        (
            "[0ms]".to_string(),
            pill_style(delay_ms == 0, theme.primary, theme.secondary),
            PlayOptionsHit::SetAudioDelay(0),
        ),
        (
            "[+50]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::AudioDelayDelta(50),
        ),
        (
            "[+200]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::AudioDelayDelta(200),
        ),
    ];
    render_inline_pills(
        frame,
        rows[ri],
        Some((" Quick Offset:", Style::default().fg(theme.text_dim))),
        &offset_pills,
        hits,
    );
    ri += 1;

    // Row 1: Hardware Device Latency Sync Presets
    if show_device_presets {
        let dev_pills = [
            ("Wired 0", 0),
            ("AirPods -150", -150),
            ("BT -220", -220),
            ("TV +80", 80),
        ]
        .into_iter()
        .map(|(lbl, ms)| {
            (
                format!("[{lbl}]"),
                pill_style(delay_ms == ms, theme.accent, theme.text_dim),
                PlayOptionsHit::SetAudioDelay(ms),
            )
        })
        .collect::<Vec<_>>();
        render_inline_pills(
            frame,
            rows[ri],
            Some((" Device Sync :", Style::default().fg(theme.text_dim))),
            &dev_pills,
            hits,
        );
        ri += 1;
    }

    // Bipolar Delay Slider (-500..+500ms)
    render_bipolar_slider(
        frame,
        rows[ri],
        "Delay (ms)",
        delay_ms,
        500,
        false,
        PlayOptionsHit::SetAudioDelayRatio,
        PlayOptionsHit::AudioDelayDelta(-50),
        PlayOptionsHit::AudioDelayDelta(50),
        hits,
        &theme,
    );
    ri += 1;

    // Live A/V Drift Sparkline
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
        rows[ri],
    );
    ri += 1;

    if chart_h >= 2 && ri < rows.len() {
        let lines = render_braille_dual_chart_lines(
            &av_samples,
            None,
            None,
            rows[ri].width as usize,
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
        frame.render_widget(Paragraph::new(lines), rows[ri]);
    }
}
