//! Page 4 — `[4] Audio & EQ`: Tracks & Spatial DSP, Dubbing Engine & Loudnorm Curve,
//! and Full-Width Interactive 10-Band Graphic Equalizer + 12 Sound Presets.

use crate::config::{
    dub_lang_label, AudioChannelMode, AudioEqPreset, DUB_LANGUAGES, EQ_BAND_LABELS,
};
use crate::state::{AppState, PlayOptionsHit};
use crate::ui::studio::widgets::{
    push_line_hit, push_pane_hit, render_inline_pills, render_loudnorm_curve_lines, section_block,
    truncate_words,
};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
    Frame,
};

pub fn render_audio_page(frame: &mut Frame, area: Rect, state: &mut AppState) {
    if area.width < 32 || area.height < 14 {
        return;
    }
    let top_h: u16 = if area.height >= 24 {
        10
    } else if area.height >= 19 {
        8
    } else {
        7
    };
    let v_splits = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(top_h), Constraint::Min(11)])
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
        .split(v_splits[0]);

    render_tracks_and_spatial_pane(frame, top_cols[0], state);
    render_dub_and_loudnorm_pane(frame, top_cols[1], state);
    render_10band_equalizer_deck(frame, v_splits[1], state);
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

fn render_tracks_and_spatial_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let total_live = state.live_playback.audio_tracks.len();
    let title = if total_live > 0 {
        format!(" Audio Tracks & Spatial DSP ({total_live}) ")
    } else {
        " Audio Tracks & Spatial DSP ".to_string()
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

    let spatial_now = state.config.audio_channel_mode;
    let hp_on = state.config.audio_voice_highpass;
    let preamp = state.config.audio_eq_preamp_db;
    let sr = state.live_playback.audio_samplerate;
    let kbps = state.live_playback.audio_bitrate_kbps;
    let ch_str = state.live_playback.audio_channels.clone();
    let dub_pref = state.config.dub_preference;
    let track_h = inner.height.saturating_sub(3).max(2);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(track_h), // 0: Compact audio tracks table
            Constraint::Length(1),       // 1: Stream spec
            Constraint::Length(1),       // 2: Spatial DSP mode pills
            Constraint::Min(1),          // 3: Highpass & Preamp summary
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

    // 0: Tracks
    let tr_area = chunks[0];
    let mut lines = Vec::new();
    lines.push(Line::from(Span::styled(
        "  ID  Lang   Role    Track / Profile           State",
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
    )));

    if total_live > 0 {
        let visible_cap = (tr_area.height as usize).saturating_sub(1).max(1);
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
            let title_w = tr_area.width.saturating_sub(29).max(8) as usize;
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
                    " {} {:>2} {:<5} {} {:<title_w$} {}",
                    if active { ">" } else { " " },
                    tr.id,
                    truncate_words(lang, 5),
                    role,
                    truncate_words(&clean, title_w),
                    state_lbl
                ),
                st,
            )));
            push_line_hit(hits, tr_area, row, PlayOptionsHit::SelectAudioTrack(tr.id));
        }
    } else if variants.is_empty() {
        lines.push(Line::from(Span::styled(
            format!(
                "  {}",
                truncate_words(
                    &format!("Policy: {} (Tracks sync via mpv IPC on launch)", dub_pref.label()),
                    tr_area.width.saturating_sub(2) as usize,
                )
            ),
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let cap = (tr_area.height as usize).saturating_sub(1).max(1);
        for (idx, av) in variants.iter().take(cap).enumerate() {
            let lang = av.lang.as_deref().unwrap_or("und");
            let role = if av.is_dubbed {
                "[DUB] "
            } else if av.is_original {
                "[ORIG]"
            } else {
                "[TRK] "
            };
            let desc = format!("{} {:.0}k", av.acodec, av.abr_kbps);
            let desc_w = tr_area.width.saturating_sub(26).max(8) as usize;
            lines.push(Line::from(Span::styled(
                format!(
                    "   {:>2} {:<5} {} {:<desc_w$} Ready",
                    idx + 1,
                    truncate_words(lang, 5),
                    role,
                    truncate_words(&desc, desc_w),
                ),
                Style::default().fg(theme.text),
            )));
        }
    }
    frame.render_widget(Paragraph::new(lines), tr_area);

    // 1: Stream spec
    let stream_txt = if sr > 0 {
        format!(
            "Stream : {sr} Hz · {} · {:.0} kbps",
            if ch_str.is_empty() { "Stereo" } else { ch_str.as_str() },
            kbps
        )
    } else {
        "Stream : Standby (Opus / AAC float pipeline ready)".to_string()
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", truncate_words(&stream_txt, chunks[1].width.saturating_sub(1) as usize)),
            Style::default().fg(theme.text_dim),
        )),
        chunks[1],
    );

    // 2: Spatial DSP pills
    let spatial_pills: Vec<(String, Style, PlayOptionsHit)> = AudioChannelMode::ALL
        .into_iter()
        .map(|mode| {
            (
                format!("[{}]", mode.short_label()),
                pill_style(
                    spatial_now == mode,
                    if mode == AudioChannelMode::Stereo {
                        theme.primary
                    } else {
                        theme.accent
                    },
                    theme.text_dim,
                ),
                PlayOptionsHit::SelectAudioSpatial(mode),
            )
        })
        .collect();
    render_inline_pills(
        frame,
        chunks[2],
        Some((" Spatial:", Style::default().fg(theme.text_dim))),
        &spatial_pills,
        hits,
    );

    // 3: Highpass + Preamp quick pills
    let filter_pills = vec![
        (
            format!("[HP 80Hz: {}]", if hp_on { "ON" } else { "OFF" }),
            pill_style(hp_on, Color::Rgb(52, 211, 153), theme.text_dim),
            PlayOptionsHit::ToggleAudioHighpass,
        ),
        (
            format!("Preamp {preamp:+}dB"),
            Style::default().fg(if preamp == 0 { theme.text_dim } else { theme.secondary }),
            PlayOptionsHit::AudioEqPreampDelta(0),
        ),
    ];
    render_inline_pills(
        frame,
        chunks[3],
        Some((" Filter :", Style::default().fg(theme.text_dim))),
        &filter_pills,
        hits,
    );

    push_pane_hit(hits, area, 0);
}

fn render_dub_and_loudnorm_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 1;
    let blk = section_block(" Dubbing Engine & Night Loudnorm [d/L/n] ", focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 24 || inner.height < 5 {
        return;
    }

    let dub_on = state.config.dub_preference.is_dub_on();
    let multi_on = state.config.audio_multistreams;
    let loudnorm_on = state.config.audio_loudnorm;
    let dub_lang = state.config.dub_lang.clone();
    let curve_rows = inner.height.saturating_sub(4).clamp(1, 3);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),          // 0: Dub + Multi-track
            Constraint::Length(1),          // 1: Lang pills row 1
            Constraint::Length(1),          // 2: Lang pills row 2
            Constraint::Length(1),          // 3: Night Loudnorm toggle
            Constraint::Min(curve_rows),    // 4: Braille compression transfer curve
        ])
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

    // Row 0: Dub + Multi-Track clickable halves
    let r0_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[0]);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Dub [d]: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if dub_on { "ON (Dubbed)" } else { "OFF (Original)" },
                Style::default()
                    .fg(if dub_on {
                        Color::Rgb(251, 191, 36)
                    } else {
                        Color::Rgb(52, 211, 153)
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        r0_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Multi: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if multi_on { "ON" } else { "OFF" },
                Style::default().fg(if multi_on { theme.primary } else { theme.text_dim }),
            ),
        ])),
        r0_cols[1],
    );
    hits.push((r0_cols[0], PlayOptionsHit::ToggleDub));
    hits.push((r0_cols[1], PlayOptionsHit::ToggleMultistreams));

    // Rows 1 & 2: Compact clickable language pills via render_inline_pills (zero `][`)
    let compact_langs = rows[1].width < 46;
    let lang_pill_at = |idx: usize| -> Option<(String, Style, PlayOptionsHit)> {
        DUB_LANGUAGES.get(idx).map(|&(code, full)| {
            let short = full.split(" (").next().unwrap_or(full);
            let lbl = if compact_langs {
                format!("[{code}]")
            } else {
                format!("[{code}:{short}]")
            };
            (
                lbl,
                pill_style(dub_lang == code, theme.secondary, theme.text_dim),
                PlayOptionsHit::SelectDubLang(code.to_string()),
            )
        })
    };
    let r1_langs: Vec<_> = (0..4).filter_map(lang_pill_at).collect();
    let r2_langs: Vec<_> = (4..DUB_LANGUAGES.len()).filter_map(lang_pill_at).collect();
    let lang_prefix = format!(
        " Lang({}):",
        dub_lang_label(&dub_lang)
            .split(" (")
            .next()
            .unwrap_or(&dub_lang)
    );
    render_inline_pills(
        frame,
        rows[1],
        Some((&lang_prefix, Style::default().fg(theme.secondary))),
        &r1_langs,
        hits,
    );
    render_inline_pills(
        frame,
        rows[2],
        Some(("      ", Style::default().fg(theme.text_dim))),
        &r2_langs,
        hits,
    );

    // Row 3: Night Loudnorm toggle
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
        rows[3],
    );
    hits.push((rows[3], PlayOptionsHit::ToggleLoudnorm));

    // Row 4: Compact Braille Transfer Curve
    if rows[4].height >= 2 {
        let lines = render_loudnorm_curve_lines(
            loudnorm_on,
            rows[4].width as usize,
            rows[4].height as usize,
            &theme,
        );
        frame.render_widget(Paragraph::new(lines), rows[4]);
    } else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                if loudnorm_on {
                    " Transfer: dynaudnorm boosts soft speech & clamps peaks"
                } else {
                    " Transfer: Linear 1:1 studio dynamic range"
                },
                Style::default().fg(theme.text_dim),
            )),
            rows[4],
        );
    }

    push_pane_hit(hits, area, 1);
}

fn render_10band_equalizer_deck(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.studio.pane_focus == 0;
    let preset = state.config.audio_eq_preset;
    let bypass = state.live_playback.audio_eq_bypass;
    let bands = state.config.active_audio_eq_bands();
    let preamp = state.config.audio_eq_preamp_db;
    let cursor = state.studio.audio_eq_band_cursor.min(9);
    let spatial = state.config.audio_channel_mode;
    let hp_on = state.config.audio_voice_highpass;
    let loudnorm_on = state.config.audio_loudnorm;

    let status_tag = if bypass {
        "BYPASSED".to_string()
    } else {
        preset.short_label().to_string()
    };
    let eq_title = format!(
        " 10-Band Interactive Graphic Equalizer & Sound Presets [e] ({status_tag}) "
    );
    let blk = section_block(&eq_title, focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 40 || inner.height < 8 {
        return;
    }

    let show_hint_row = inner.height >= 14;
    let mut constraints = vec![
        Constraint::Length(1), // 0: Preset pills row 1 (6 presets)
        Constraint::Length(1), // 1: Preset pills row 2 (6 presets + Custom)
        Constraint::Length(1), // 2: Bypass / Save / Reset / Preamp row
        Constraint::Min(4),    // 3: 10-Band vertical interactive faders
        Constraint::Length(1), // 4: Active DSP pipeline summary
    ];
    if show_hint_row {
        constraints.push(Constraint::Length(1));
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let hits = &mut state.live_playback.play_hitboxes;

    // Row 0: Presets 0..6
    let p_row0: Vec<(String, Style, PlayOptionsHit)> = AudioEqPreset::PRESETS_12[0..6]
        .iter()
        .map(|&p| {
            (
                format!("[{}]", p.short_label()),
                pill_style(!bypass && preset == p, theme.primary, theme.text_dim),
                PlayOptionsHit::SelectAudioEq(p),
            )
        })
        .collect();
    render_inline_pills(
        frame,
        rows[0],
        Some((
            " Presets [e]:",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        )),
        &p_row0,
        hits,
    );

    // Row 1: Presets 6..12 + [Custom]
    let mut p_row1: Vec<(String, Style, PlayOptionsHit)> = AudioEqPreset::PRESETS_12[6..12]
        .iter()
        .map(|&p| {
            (
                format!("[{}]", p.short_label()),
                pill_style(!bypass && preset == p, theme.accent, theme.text_dim),
                PlayOptionsHit::SelectAudioEq(p),
            )
        })
        .collect();
    p_row1.push((
        "[Custom*]".to_string(),
        pill_style(!bypass && preset == AudioEqPreset::Custom, theme.secondary, theme.text_dim),
        PlayOptionsHit::SelectAudioEq(AudioEqPreset::Custom),
    ));
    render_inline_pills(
        frame,
        rows[1],
        Some(("            :", Style::default().fg(theme.text_dim))),
        &p_row1,
        hits,
    );

    // Row 2: Bypass / Save / Flat Reset / Preamp Trim
    let tool_pills = vec![
        (
            if bypass { "[b Bypass: ON]" } else { "[b Bypass: OFF]" }.to_string(),
            pill_style(bypass, Color::Rgb(251, 191, 36), theme.text_dim),
            PlayOptionsHit::ToggleAudioEqBypass,
        ),
        (
            "[Save Custom]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::SaveCustomAudioEq,
        ),
        (
            "[0 Flat 0dB]".to_string(),
            Style::default().fg(theme.text_dim),
            PlayOptionsHit::ResetAudioEq,
        ),
        (
            "Preamp:".to_string(),
            Style::default().fg(theme.text_dim),
            PlayOptionsHit::AudioEqPreampDelta(0),
        ),
        (
            "[-]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::AudioEqPreampDelta(-1),
        ),
        (
            format!("{preamp:+}dB"),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            PlayOptionsHit::AudioEqPreampDelta(0),
        ),
        (
            "[+]".to_string(),
            Style::default().fg(theme.secondary),
            PlayOptionsHit::AudioEqPreampDelta(1),
        ),
    ];
    render_inline_pills(
        frame,
        rows[2],
        Some((" Tools      :", Style::default().fg(theme.text_dim))),
        &tool_pills,
        hits,
    );

    // Row 3: Interactive 10-Band Vertical Fader Console (31Hz .. 16kHz)
    let eq_area = rows[3];
    let show_steppers = eq_area.height >= 6;
    let bar_h = eq_area
        .height
        .saturating_sub(if show_steppers { 4 } else { 2 })
        .max(2);
    let band_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Ratio(1, 10); 10])
        .split(eq_area);

    for (idx, &col_rect) in band_cols.iter().enumerate() {
        let gain = bands[idx].clamp(-12, 12);
        let is_selected = idx == cursor;
        let fill_col = if bypass {
            theme.text_dim
        } else if gain >= 5 {
            Color::Rgb(251, 191, 36)
        } else if gain > 0 {
            Color::Rgb(52, 211, 153)
        } else if gain < 0 {
            Color::Rgb(56, 189, 248)
        } else {
            theme.secondary
        };

        let mut f_constraints = Vec::new();
        if show_steppers {
            f_constraints.push(Constraint::Length(1)); // [+]
        }
        f_constraints.push(Constraint::Length(1)); // +XdB
        f_constraints.push(Constraint::Length(bar_h)); // vertical fader bars
        if show_steppers {
            f_constraints.push(Constraint::Length(1)); // [-]
        }
        f_constraints.push(Constraint::Min(1)); // freq label

        let b_rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints(f_constraints)
            .split(col_rect);

        let mut bi = 0usize;
        if show_steppers {
            frame.render_widget(
                Paragraph::new(Span::styled("[+]", Style::default().fg(theme.secondary)))
                    .alignment(Alignment::Center),
                b_rows[bi],
            );
            hits.push((b_rows[bi], PlayOptionsHit::AudioEqBandDelta(idx, 1)));
            bi += 1;
        }

        // Gain dB readout
        frame.render_widget(
            Paragraph::new(Span::styled(
                format!("{gain:+}dB"),
                Style::default()
                    .fg(if is_selected { Color::White } else { fill_col })
                    .add_modifier(if is_selected || gain != 0 {
                        Modifier::BOLD
                    } else {
                        Modifier::empty()
                    }),
            ))
            .alignment(Alignment::Center),
            b_rows[bi],
        );
        bi += 1;

        // Vertical Fader Meter (interactive per row: top=+12dB, center=0dB, bottom=-12dB)
        let meter_area = b_rows[bi];
        bi += 1;
        let bh = meter_area.height as usize;
        let zero_row = bh / 2;
        let pos_steps = zero_row.max(1);
        let neg_steps = bh.saturating_sub(zero_row + 1).max(1);
        let bw = (meter_area.width as usize).saturating_sub(2).clamp(2, 4);

        for r in 0..bh {
            let row_rect = Rect::new(meter_area.x, meter_area.y + r as u16, meter_area.width, 1);
            let mapped_db: i8 = if bh <= 2 {
                if r == 0 { 6 } else { -6 }
            } else if r == zero_row {
                0
            } else if r < zero_row {
                let frac = (zero_row - r) as f64 / pos_steps as f64;
                ((frac * 12.0).round() as i8).clamp(1, 12)
            } else {
                let frac = (r - zero_row) as f64 / neg_steps as f64;
                -((frac * 12.0).round() as i8).clamp(1, 12)
            };

            let (glyph, gcol) = if r == zero_row {
                if gain == 0 {
                    ("━".repeat(bw), fill_col)
                } else {
                    ("─".repeat(bw), theme.divider)
                }
            } else if r < zero_row {
                let req_db = (((zero_row - r) as f64 - 0.5) / pos_steps as f64) * 12.0;
                if gain as f64 >= req_db.max(0.5) {
                    ("█".repeat(bw), fill_col)
                } else {
                    ("░".repeat(bw), theme.divider)
                }
            } else {
                let req_neg = -((((r - zero_row) as f64 - 0.5) / neg_steps as f64) * 12.0);
                if (gain as f64) <= req_neg.min(-0.5) {
                    ("█".repeat(bw), fill_col)
                } else {
                    ("░".repeat(bw), theme.divider)
                }
            };

            frame.render_widget(
                Paragraph::new(Span::styled(glyph, Style::default().fg(gcol)))
                    .alignment(Alignment::Center),
                row_rect,
            );
            hits.push((row_rect, PlayOptionsHit::SetAudioEqBand(idx, mapped_db)));
        }

        if show_steppers {
            frame.render_widget(
                Paragraph::new(Span::styled("[-]", Style::default().fg(theme.secondary)))
                    .alignment(Alignment::Center),
                b_rows[bi],
            );
            hits.push((b_rows[bi], PlayOptionsHit::AudioEqBandDelta(idx, -1)));
            bi += 1;
        }

        // Frequency label footer
        let flbl = EQ_BAND_LABELS[idx];
        let lbl_str = if is_selected {
            format!("[{flbl}]")
        } else {
            flbl.to_string()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                lbl_str,
                if is_selected {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            ))
            .alignment(Alignment::Center),
            b_rows[bi],
        );
    }

    // Row 4: Live DSP pipeline summary
    let eq_nonzero = !bypass && bands.iter().any(|&b| b != 0);
    let pipe_desc = if !eq_nonzero && preamp == 0 && !hp_on && !loudnorm_on && spatial == AudioChannelMode::Stereo {
        "Pipeline: Bit-exact passthrough (Zero DSP overhead · All bands at 0dB)".to_string()
    } else {
        let mut stages = Vec::new();
        if !bypass && preamp != 0 {
            stages.push(format!("Preamp {preamp:+}dB"));
        }
        if hp_on {
            stages.push("HP 80Hz".to_string());
        }
        if eq_nonzero {
            stages.push(format!("10-Band EQ ({})", preset.short_label()));
        }
        if loudnorm_on {
            stages.push("dynaudnorm".to_string());
        }
        if spatial != AudioChannelMode::Stereo {
            stages.push(spatial.short_label().to_string());
        }
        format!("Pipeline: {}", stages.join(" -> "))
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", truncate_words(&pipe_desc, rows[4].width.saturating_sub(1) as usize)),
            Style::default().fg(theme.secondary),
        )),
        rows[4],
    );

    if show_hint_row && rows.len() > 5 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Keys: [h/l] Select Band  [j/k] Gain ±1dB (or Click Meter/+/-)  [e] Cycle Preset  [b] Bypass  [0] Flat",
                Style::default().fg(theme.text_dim),
            )),
            rows[5],
        );
    }
}
