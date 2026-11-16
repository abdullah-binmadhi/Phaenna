//! Unified 5-Zone Playback Studio & Live Control Deck (`p`).
//! Controls Quality, Player Backend, Dubbed vs. Original Audio (`d`),
//! Live mpv Audio/Sub Tracks, Loudnorm, Timing Offsets, 4K Shaders (`s`),
//! Speed, Volume, Picture Tuning, and Safe Outro SponsorBlock settings.

use crate::config::{dub_lang_label, Player, Quality, SponsorBlockMode, VideoShaderMode};
use crate::state::{
    active_modal_rect, centered_rect, ActiveOverlay, AppState, NowPlayingHit, PlayOptionsHit,
};
use crate::ui::home::marquee_window;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

pub fn format_hms(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

fn push_line_hit(
    hitboxes: &mut Vec<(Rect, PlayOptionsHit)>,
    inner: Rect,
    row_offset: u16,
    hit: PlayOptionsHit,
) {
    if row_offset < inner.height && inner.width > 0 {
        hitboxes.push((Rect::new(inner.x, inner.y + row_offset, inner.width, 1), hit));
    }
}

fn push_panel_hit(hitboxes: &mut Vec<(Rect, PlayOptionsHit)>, rect: Rect, section: usize) {
    if rect.width > 0 && rect.height > 0 {
        hitboxes.push((rect, PlayOptionsHit::FocusSection(section)));
    }
}

pub fn render_play_options_modal(frame: &mut Frame, state: &mut AppState) {
    let theme = state.base_theme();
    let area = active_modal_rect(frame.area(), ActiveOverlay::PlayOptions)
        .unwrap_or_else(|| centered_rect(92, 80, frame.area()));
    frame.render_widget(Clear, area);
    state.live_playback.play_hitboxes.clear();

    let live = state.has_live_ipc();
    let video_title = if live {
        state
            .active_playback_title
            .clone()
            .unwrap_or_else(|| "Active Playback".to_string())
    } else {
        state
            .active_selected_video()
            .map(|v| v.title)
            .unwrap_or_else(|| "Selected Video".to_string())
    };
    let title_budget = area.width.saturating_sub(36).max(16) as usize;
    let display_title = marquee_window(&video_title, title_budget, state.ticker_step);
    let badge_text = if live { " [LIVE mpv IPC] " } else { " [PRE-LAUNCH] " };

    let outer = Block::default()
        .title(Line::from(vec![
            Span::styled(
                format!(" PLAYBACK STUDIO - {} ", display_title),
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                badge_text,
                Style::default()
                    .fg(Color::Black)
                    .bg(if live { Color::Rgb(52, 211, 153) } else { theme.secondary })
                    .add_modifier(Modifier::BOLD),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Percentage(55),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    render_transport_banner(frame, v_chunks[0], state);

    let row1_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(38),
            Constraint::Percentage(34),
        ])
        .split(v_chunks[1]);

    render_stream_engine_pane(frame, row1_cols[0], state);
    render_audio_dub_pane(frame, row1_cols[1], state);
    render_captions_sponsor_pane(frame, row1_cols[2], state);

    let row2_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(v_chunks[2]);

    render_shaders_pane(frame, row2_cols[0], state);
    render_live_controls_pane(frame, row2_cols[1], state);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[<-/->/Tab] Panel ", Style::default().fg(theme.primary)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[Up/Down] Select ", Style::default().fg(theme.secondary)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[d] Dub ON/OFF ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[Space] Pause/Play ", Style::default().fg(theme.text)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[s] Shader ", Style::default().fg(theme.primary)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled(
                if live { "[Enter] Relaunch/Apply " } else { "[Enter] Launch Video " },
                Style::default().fg(Color::Rgb(52, 211, 153)).add_modifier(Modifier::BOLD),
            ),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        v_chunks[3],
    );
}

fn render_transport_banner(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme();
    let live = state.has_live_ipc();
    let blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if live {
            Color::Rgb(52, 211, 153)
        } else {
            theme.border_idle
        }));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 20 || inner.height == 0 {
        return;
    }

    let dub_on = state.config.dub_preference.is_dub_on();
    let dub_pill = if dub_on {
        format!(" DUB: ON ({}) ", state.config.dub_lang.to_uppercase())
    } else {
        " DUB: OFF [ORIG] ".to_string()
    };

    if !live {
        let q = Quality::ALL
            .get(state.play_quality_idx)
            .copied()
            .unwrap_or(state.config.quality);
        let launch_label = " [Launch Video] ";
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(20),
                Constraint::Length(dub_pill.len() as u16 + 1),
                Constraint::Length(launch_label.len() as u16 + 1),
            ])
            .split(inner);

        let summary = Line::from(vec![
            Span::styled(" Ready: ", Style::default().fg(theme.text_dim)),
            Span::styled(q.short_badge(), Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" via {} ", state.config.player.as_str()), Style::default().fg(theme.text)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled(
                if state.config.audio_multistreams { "Multi-Audio Ready" } else { "Single Audio" },
                Style::default().fg(theme.secondary),
            ),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled(
                if state.config.skip_outro { "Outro Skip: ON" } else { "Outro Protected" },
                Style::default().fg(Color::Rgb(52, 211, 153)),
            ),
        ]);
        frame.render_widget(Paragraph::new(summary), cols[0]);
        frame.render_widget(
            Paragraph::new(Span::styled(
                dub_pill,
                Style::default()
                    .fg(Color::Black)
                    .bg(if dub_on { Color::Rgb(251, 191, 36) } else { theme.accent })
                    .add_modifier(Modifier::BOLD),
            )),
            cols[1],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                launch_label,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            )),
            cols[2],
        );
        state.live_playback.play_hitboxes.push((cols[1], PlayOptionsHit::ToggleDub));
        state.live_playback.play_hitboxes.push((cols[2], PlayOptionsHit::LaunchOrRelaunch));
        return;
    }

    let play_lbl = if state.live_playback.paused { " [Play] " } else { " [Pause] " };
    let back_lbl = " [-10s] ";
    let fwd_lbl = " [+10s] ";
    let ch_prev_lbl = " [<Ch] ";
    let ch_next_lbl = " [Ch>] ";
    let vol_lbl = if state.live_playback.muted {
        " MUTE ".to_string()
    } else {
        format!(" Spd:{:.2}x Vol:{}% ", state.live_playback.speed, state.live_playback.volume)
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(play_lbl.len() as u16),
            Constraint::Length(back_lbl.len() as u16),
            Constraint::Length(fwd_lbl.len() as u16),
            Constraint::Length(ch_prev_lbl.len() as u16),
            Constraint::Length(ch_next_lbl.len() as u16),
            Constraint::Min(14),
            Constraint::Length(vol_lbl.len() as u16),
            Constraint::Length(dub_pill.len() as u16),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Span::styled(
            play_lbl,
            Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD),
        )),
        cols[0],
    );
    frame.render_widget(Paragraph::new(Span::styled(back_lbl, Style::default().fg(theme.text))), cols[1]);
    frame.render_widget(Paragraph::new(Span::styled(fwd_lbl, Style::default().fg(theme.text))), cols[2]);
    frame.render_widget(Paragraph::new(Span::styled(ch_prev_lbl, Style::default().fg(theme.text_dim))), cols[3]);
    frame.render_widget(Paragraph::new(Span::styled(ch_next_lbl, Style::default().fg(theme.text_dim))), cols[4]);

    // Clickable seek scrubber bar
    let pos = state.active_playback_pos;
    let dur = state.active_playback_dur.max(0.0);
    let ratio = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 };
    let time_str = format!(" {} / {} ", format_hms(pos), format_hms(dur));
    let bar_w = cols[5].width.saturating_sub(time_str.len() as u16 + 2) as usize;
    let filled = ((bar_w as f64) * ratio).round() as usize;
    let track: String = (0..bar_w)
        .map(|i| if i == filled.min(bar_w.saturating_sub(1)) { '●' } else if i < filled { '━' } else { '─' })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(format!(" {track}"), Style::default().fg(Color::Rgb(52, 211, 153))),
            Span::styled(time_str, Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        ])),
        cols[5],
    );
    frame.render_widget(Paragraph::new(Span::styled(vol_lbl, Style::default().fg(theme.secondary))), cols[6]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            dub_pill,
            Style::default()
                .fg(Color::Black)
                .bg(if dub_on { Color::Rgb(251, 191, 36) } else { Color::Rgb(52, 211, 153) })
                .add_modifier(Modifier::BOLD),
        )),
        cols[7],
    );

    let hits = &mut state.live_playback.play_hitboxes;
    hits.push((cols[0], PlayOptionsHit::TogglePause));
    hits.push((cols[1], PlayOptionsHit::SeekDelta(-10.0)));
    hits.push((cols[2], PlayOptionsHit::SeekDelta(10.0)));
    hits.push((cols[3], PlayOptionsHit::ChapterStep(-1)));
    hits.push((cols[4], PlayOptionsHit::ChapterStep(1)));
    if dur > 0.0 {
        hits.push((cols[5], PlayOptionsHit::SeekRatio(-1.0))); // resolved via click column
    }
    hits.push((cols[7], PlayOptionsHit::ToggleDub));
}

fn section_block<'a>(title: &'a str, focused: bool, theme: &'a crate::ui::theme::ThemePalette) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(if focused { theme.primary } else { theme.text_dim })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused {
            theme.border_focus
        } else {
            theme.border_idle
        }))
}

fn render_stream_engine_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme();
    let focused = state.play_modal_section == 0;
    let blk = section_block(" 1. Stream & Player ", focused, theme);
    let inner = blk.inner(area);

    let mut lines: Vec<Line> = Vec::new();
    let players = [Player::Mpv, Player::Iina, Player::Vlc, Player::Browser];
    let hitboxes = &mut state.live_playback.play_hitboxes;

    for (i, q) in Quality::ALL.iter().enumerate() {
        let active = i == state.play_quality_idx;
        let st = if active && focused {
            Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD)
        } else if active {
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        let row = lines.len() as u16;
        lines.push(Line::from(Span::styled(
            format!(" {} {}", if active { ">" } else { " " }, q.label()),
            st,
        )));
        push_line_hit(hitboxes, inner, row, PlayOptionsHit::SelectQuality(i));
    }

    lines.push(Line::from(Span::styled(" Player Backend:", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD))));
    for pl in players {
        let active = pl == state.config.player;
        let row = lines.len() as u16;
        lines.push(Line::from(Span::styled(
            format!(" {} {}", if active { "*" } else { "o" }, pl.label()),
            if active {
                Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            },
        )));
        push_line_hit(hitboxes, inner, row, PlayOptionsHit::SelectPlayer(pl));
    }
    push_panel_hit(hitboxes, area, 0);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

fn render_audio_dub_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme();
    let focused = state.play_modal_section == 1;
    let blk = section_block(" 2. Audio & Dubbing [d] ", focused, theme);
    let inner = blk.inner(area);

    let live = state.has_live_ipc();
    let dub_on = state.config.dub_preference.is_dub_on();
    let mut lines: Vec<Line> = Vec::new();
    let hits = &mut state.live_playback.play_hitboxes;

    let r0 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Dubbed Audio  : ", Style::default().fg(theme.text_dim)),
        Span::styled(
            state.config.dub_preference.label(),
            Style::default()
                .fg(Color::Black)
                .bg(if dub_on { Color::Rgb(251, 191, 36) } else { Color::Rgb(52, 211, 153) })
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    push_line_hit(hits, inner, r0, PlayOptionsHit::ToggleDub);

    let r1 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Dub Language  : ", Style::default().fg(theme.text_dim)),
        Span::styled(
            format!("{} [L]", dub_lang_label(&state.config.dub_lang)),
            Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD),
        ),
    ]));
    push_line_hit(hits, inner, r1, PlayOptionsHit::CycleDubLang);

    let r2 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Multi-Track   : ", Style::default().fg(theme.text_dim)),
        Span::styled(
            if state.config.audio_multistreams { "ON (Instant Switch)" } else { "OFF" },
            Style::default().fg(if state.config.audio_multistreams { theme.primary } else { theme.text_dim }),
        ),
    ]));
    push_line_hit(hits, inner, r2, PlayOptionsHit::ToggleMultistreams);

    let r3 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Night Loudnorm: ", Style::default().fg(theme.text_dim)),
        Span::styled(
            if state.config.audio_loudnorm { "ON [n]" } else { "OFF [n]" },
            Style::default().fg(if state.config.audio_loudnorm { Color::Rgb(52, 211, 153) } else { theme.text }),
        ),
    ]));
    push_line_hit(hits, inner, r3, PlayOptionsHit::ToggleLoudnorm);

    let r4 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Audio Delay   : ", Style::default().fg(theme.text_dim)),
        Span::styled(
            format!("{:+} ms  ([-] / [+])", state.live_playback.audio_delay_ms),
            Style::default().fg(theme.text),
        ),
    ]));
    if inner.width > 12 && r4 < inner.height {
        let mid = inner.x + inner.width / 2;
        hits.push((Rect::new(inner.x, inner.y + r4, mid - inner.x, 1), PlayOptionsHit::AudioDelayDelta(-50)));
        hits.push((Rect::new(mid, inner.y + r4, inner.x + inner.width - mid, 1), PlayOptionsHit::AudioDelayDelta(50)));
    }

    lines.push(Line::from(Span::styled(
        " Live Audio Tracks (mpv):",
        Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
    )));
    if state.live_playback.audio_tracks.is_empty() {
        lines.push(Line::from(Span::styled(
            if live {
                "  (single audio stream loaded)"
            } else {
                "  (start playback to inspect live tracks)"
            },
            Style::default().fg(theme.text_dim),
        )));
    } else {
        for tr in state.live_playback.audio_tracks.clone() {
            let active = state.live_playback.active_aid == Some(tr.id) || tr.selected;
            let row = lines.len() as u16;
            let st = if active {
                Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            lines.push(Line::from(Span::styled(
                format!(" {} {}", if active { ">" } else { " " }, tr.display_label()),
                st,
            )));
            push_line_hit(hits, inner, row, PlayOptionsHit::SelectAudioTrack(tr.id));
        }
    }
    push_panel_hit(hits, area, 1);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

fn render_captions_sponsor_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme();
    let focused = state.play_modal_section == 2;
    let blk = section_block(" 3. Captions & SponsorBlock ", focused, theme);
    let inner = blk.inner(area);

    let live = state.has_live_ipc();
    let mut lines: Vec<Line> = Vec::new();
    let hits = &mut state.live_playback.play_hitboxes;

    let sub_on = if live {
        state.live_playback.sub_visible
    } else {
        state.config.subtitles_enabled
    };
    let r0 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Subtitles  : ", Style::default().fg(theme.text_dim)),
        Span::styled(
            if sub_on { "ON [c]" } else { "OFF [c]" },
            Style::default().fg(if sub_on { theme.accent } else { theme.text_dim }).add_modifier(Modifier::BOLD),
        ),
    ]));
    push_line_hit(hits, inner, r0, PlayOptionsHit::ToggleSubtitles);

    let r1 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Style      : ", Style::default().fg(theme.text_dim)),
        Span::styled(
            format!("{} [y]", state.config.subtitle_style.preset.label()),
            Style::default().fg(theme.secondary),
        ),
    ]));
    push_line_hit(hits, inner, r1, PlayOptionsHit::CycleSubStyle);

    let r2 = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Sub Offset : ", Style::default().fg(theme.text_dim)),
        Span::styled(format!("{:+} ms", state.live_playback.sub_delay_ms), Style::default().fg(theme.text)),
    ]));
    if inner.width > 12 && r2 < inner.height {
        let mid = inner.x + inner.width / 2;
        hits.push((Rect::new(inner.x, inner.y + r2, mid - inner.x, 1), PlayOptionsHit::SubDelayDelta(-100)));
        hits.push((Rect::new(mid, inner.y + r2, inner.x + inner.width - mid, 1), PlayOptionsHit::SubDelayDelta(100)));
    }

    lines.push(Line::from(Span::styled(
        " SponsorBlock:",
        Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
    )));
    for mode in [
        SponsorBlockMode::AutoSkipAll,
        SponsorBlockMode::SponsorOnly,
        SponsorBlockMode::ShowChaptersOnly,
        SponsorBlockMode::Off,
    ] {
        let active = mode == state.config.sponsorblock_mode;
        let row = lines.len() as u16;
        lines.push(Line::from(Span::styled(
            format!(" {} {}", if active { ">" } else { " " }, mode.label()),
            if active {
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            },
        )));
        push_line_hit(hits, inner, row, PlayOptionsHit::SelectSponsorBlock(mode));
    }

    let r_outro = lines.len() as u16;
    lines.push(Line::from(vec![
        Span::styled(" Skip Outros: ", Style::default().fg(theme.text_dim)),
        Span::styled(
            if state.config.skip_outro { "ON" } else { "OFF (Full Ending)" },
            Style::default()
                .fg(if state.config.skip_outro { Color::Rgb(251, 191, 36) } else { Color::Rgb(52, 211, 153) })
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    push_line_hit(hits, inner, r_outro, PlayOptionsHit::ToggleSkipOutro);

    for tr in state.live_playback.sub_tracks.iter().take(2).cloned() {
        let active = state.live_playback.active_sid == Some(tr.id) || tr.selected;
        let row = lines.len() as u16;
        lines.push(Line::from(Span::styled(
            format!(" Sub {} {}", if active { ">" } else { " " }, tr.display_label()),
            if active {
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_dim)
            },
        )));
        push_line_hit(hits, inner, row, PlayOptionsHit::SelectSubTrack(tr.id));
    }
    push_panel_hit(hits, area, 2);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

fn render_shaders_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme();
    let focused = state.play_modal_section == 3;
    let title = if state.has_live_ipc() {
        " 4. 4K Shaders [s] (Hot-Swap Live) "
    } else {
        " 4. 4K Mastering Shaders [s] "
    };
    let blk = section_block(title, focused, theme);
    let inner = blk.inner(area);

    let all_shaders = [
        VideoShaderMode::Reference4K,
        VideoShaderMode::NeuralCinema,
        VideoShaderMode::Anime4K,
        VideoShaderMode::Balanced,
        VideoShaderMode::Off,
    ];
    let mut lines = Vec::new();
    let hits = &mut state.live_playback.play_hitboxes;

    for m in all_shaders {
        let active = m == state.config.shader_mode;
        let row = lines.len() as u16;
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} {:<23} ", if active { ">" } else { " " }, m.short_label()),
                if active {
                    Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            ),
            Span::styled(m.detail_summary(), Style::default().fg(theme.text_dim)),
        ]));
        push_line_hit(hits, inner, row, PlayOptionsHit::SelectShader(m));
    }
    push_panel_hit(hits, area, 3);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

fn render_live_controls_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = state.base_theme();
    let focused = state.play_modal_section == 4;
    let live = state.has_live_ipc();
    let blk = section_block(" 5. Speed, Volume & Picture Controls ", focused, theme);
    let inner = blk.inner(area);
    let hits = &mut state.live_playback.play_hitboxes;

    if inner.width < 20 || inner.height < 5 {
        frame.render_widget(blk, area);
        return;
    }
    frame.render_widget(blk, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
        ])
        .split(inner);

    // Row 0: Speed presets
    let speeds = [0.75_f64, 1.0, 1.25, 1.5, 2.0];
    let mut spd_cols = vec![Constraint::Length(8)];
    spd_cols.extend(std::iter::repeat_n(Constraint::Length(7), speeds.len()));
    spd_cols.push(Constraint::Min(0));
    let sp_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(spd_cols)
        .split(rows[0]);
    frame.render_widget(Paragraph::new(Span::styled(" Speed:", Style::default().fg(theme.text_dim))), sp_rects[0]);
    for (idx, spd) in speeds.iter().enumerate() {
        let active = (state.live_playback.speed - *spd).abs() < 0.04;
        let label = if (*spd - 2.0).abs() < 0.01 { " [2x] ".to_string() } else { format!("[{spd:.2}]") };
        frame.render_widget(
            Paragraph::new(Span::styled(
                label,
                if active {
                    Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            )),
            sp_rects[idx + 1],
        );
        hits.push((sp_rects[idx + 1], PlayOptionsHit::SetSpeed(*spd)));
    }

    // Row 1: Volume slider & Mute
    let vol_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Min(10),
            Constraint::Length(5),
            Constraint::Length(9),
        ])
        .split(rows[1]);
    frame.render_widget(Paragraph::new(Span::styled(" Vol  :", Style::default().fg(theme.text_dim))), vol_rects[0]);
    frame.render_widget(Paragraph::new(Span::styled(" [-] ", Style::default().fg(theme.secondary))), vol_rects[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}%", state.live_playback.volume),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
        )),
        vol_rects[2],
    );
    frame.render_widget(Paragraph::new(Span::styled(" [+] ", Style::default().fg(theme.secondary))), vol_rects[3]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.muted { "[UNMUTE]" } else { " [MUTE] " },
            if state.live_playback.muted {
                Style::default().fg(Color::Black).bg(Color::Rgb(239, 68, 68)).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.accent)
            },
        )),
        vol_rects[4],
    );
    hits.push((vol_rects[1], PlayOptionsHit::VolumeDelta(-5)));
    hits.push((vol_rects[3], PlayOptionsHit::VolumeDelta(5)));
    hits.push((vol_rects[4], PlayOptionsHit::ToggleMute));

    // Row 2 & 3: Picture Tuner
    let pic_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(22),
            Constraint::Percentage(22),
            Constraint::Percentage(22),
            Constraint::Percentage(20),
            Constraint::Percentage(14),
        ])
        .split(rows[2]);
    let pic_items: [(&str, &str, i64); 4] = [
        ("Bri", "brightness", state.live_playback.brightness),
        ("Con", "contrast", state.live_playback.contrast),
        ("Sat", "saturation", state.live_playback.saturation),
        ("Gam", "gamma", state.live_playback.gamma),
    ];
    for (idx, (lbl, prop, val)) in pic_items.iter().enumerate() {
        frame.render_widget(
            Paragraph::new(Span::styled(format!(" {lbl}:{val:+}"), Style::default().fg(theme.text))),
            pic_rects[idx],
        );
        hits.push((pic_rects[idx], PlayOptionsHit::PictureDelta(prop, 5)));
    }
    frame.render_widget(
        Paragraph::new(Span::styled(" [Rst]", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD))),
        pic_rects[4],
    );
    hits.push((pic_rects[4], PlayOptionsHit::ResetPicture));

    // Row 3: Window & Session Buttons
    let win_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(22),
            Constraint::Percentage(30),
            Constraint::Percentage(23),
        ])
        .split(rows[3]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.ontop { " [OnTop:ON]" } else { " [OnTop]" },
            Style::default().fg(if state.live_playback.ontop { theme.primary } else { theme.text }),
        )),
        win_rects[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.loop_video { "[Loop:ON]" } else { "[Loop]" },
            Style::default().fg(if state.live_playback.loop_video { theme.primary } else { theme.text }),
        )),
        win_rects[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if live { "[Relaunch]" } else { "[Launch Video]" },
            Style::default().fg(Color::Rgb(52, 211, 153)).add_modifier(Modifier::BOLD),
        )),
        win_rects[2],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            "[Stop]",
            Style::default().fg(Color::Rgb(239, 68, 68)).add_modifier(Modifier::BOLD),
        )),
        win_rects[3],
    );
    hits.push((win_rects[0], PlayOptionsHit::ToggleOnTop));
    hits.push((win_rects[1], PlayOptionsHit::ToggleLoop));
    hits.push((win_rects[2], PlayOptionsHit::LaunchOrRelaunch));
    hits.push((win_rects[3], PlayOptionsHit::StopPlayer));
    push_panel_hit(hits, area, 4);
}

/// Docked 1-line live playback control bar rendered on Home & Detail while `mpv` IPC is active.
pub fn render_now_playing_bar(frame: &mut Frame, area: Rect, state: &mut AppState) {
    state.live_playback.now_playing_hitboxes.clear();
    if area.width < 32 || area.height == 0 || !state.has_live_ipc() {
        return;
    }
    let theme = state.base_theme();
    let title_raw = state
        .active_playback_title
        .as_deref()
        .unwrap_or("Now Playing");
    let play_lbl = if state.live_playback.paused { "[Play]" } else { "[Pause]" };
    let dub_lbl = if state.config.dub_preference.is_dub_on() {
        format!("[Dub:{}]", state.config.dub_lang.to_uppercase())
    } else {
        "[Dub:OFF]".to_string()
    };
    let spd_lbl = format!("{:.2}x", state.live_playback.speed);
    let vol_lbl = if state.live_playback.muted {
        "MUTE".to_string()
    } else {
        format!("{}%", state.live_playback.volume)
    };
    let shader_lbl = format!("[{}]", state.config.shader_mode.short_label());
    let time_lbl = format!(
        "{}/{}",
        format_hms(state.active_playback_pos),
        format_hms(state.active_playback_dur.max(0.0))
    );

    let bg = Color::Rgb(18, 22, 32);
    let mut spans: Vec<(String, Style, Option<NowPlayingHit>)> = vec![
        (" LIVE ".to_string(), Style::default().fg(Color::Black).bg(Color::Rgb(52, 211, 153)).add_modifier(Modifier::BOLD), Some(NowPlayingHit::OpenStudio)),
        (format!(" {time_lbl} "), Style::default().fg(theme.text).bg(bg).add_modifier(Modifier::BOLD), None),
        (format!(" {play_lbl} "), Style::default().fg(theme.primary).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::TogglePause)),
        ("[-10]".to_string(), Style::default().fg(theme.text_dim).bg(bg), Some(NowPlayingHit::SeekBack)),
        (" ".to_string(), Style::default().bg(bg), None),
        ("[+10]".to_string(), Style::default().fg(theme.text_dim).bg(bg), Some(NowPlayingHit::SeekFwd)),
        (" ".to_string(), Style::default().bg(bg), None),
        (
            dub_lbl,
            Style::default()
                .fg(Color::Black)
                .bg(if state.config.dub_preference.is_dub_on() { Color::Rgb(251, 191, 36) } else { theme.accent })
                .add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::ToggleDub),
        ),
        (" [<]".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::SpeedStep(-1))),
        (spd_lbl, Style::default().fg(theme.text).bg(bg), Some(NowPlayingHit::OpenStudio)),
        ("[>] ".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::SpeedStep(1))),
        ("[-]".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::VolumeDelta(-5))),
        (vol_lbl, Style::default().fg(theme.text).bg(bg), Some(NowPlayingHit::ToggleMute)),
        ("[+] ".to_string(), Style::default().fg(theme.secondary).bg(bg), Some(NowPlayingHit::VolumeDelta(5))),
        (shader_lbl, Style::default().fg(theme.primary).bg(bg), Some(NowPlayingHit::CycleShader)),
        (" [Studio(p)]".to_string(), Style::default().fg(theme.accent).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::OpenStudio)),
        (" [Stop] ".to_string(), Style::default().fg(Color::Rgb(239, 68, 68)).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::StopPlayer)),
    ];

    let fixed_w: usize = spans.iter().map(|(s, _, _)| s.chars().count()).sum();
    let title_budget = (area.width as usize).saturating_sub(fixed_w + 3);
    if title_budget >= 8 {
        let trunc = marquee_window(title_raw, title_budget, state.ticker_step);
        spans.insert(
            1,
            (
                format!(" {trunc} ·"),
                Style::default().fg(theme.secondary).bg(bg),
                Some(NowPlayingHit::OpenStudio),
            ),
        );
    }

    let mut cur_x = area.x;
    let mut rendered_spans = Vec::with_capacity(spans.len() + 1);
    for (text, style, hit) in spans {
        let len = text.chars().count() as u16;
        if cur_x + len > area.x + area.width {
            break;
        }
        if let Some(action) = hit {
            state
                .live_playback
                .now_playing_hitboxes
                .push((Rect::new(cur_x, area.y, len, 1), action));
        }
        cur_x += len;
        rendered_spans.push(Span::styled(text, style));
    }
    if cur_x < area.x + area.width {
        rendered_spans.push(Span::styled(
            " ".repeat((area.x + area.width - cur_x) as usize),
            Style::default().bg(bg),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(rendered_spans)), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, DubPreference};
    use crate::services::ipc::{LiveMediaTrack, TrackKind};
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_playback_studio_renders_prelaunch_and_live_ipc_modes() {
        let mut st = AppState::new(Config::default());
        let backend = TestBackend::new(130, 38);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_play_options_modal(f, &mut st)).unwrap();
        assert!(!st.live_playback.play_hitboxes.is_empty());

        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        st.active_playback_video_id = Some("dQw4w9WgXcQ".into());
        st.active_playback_title = Some("Multi Audio Sample".into());
        st.active_playback_pos = 125.0;
        st.active_playback_dur = 600.0;
        st.live_playback.cmd_tx = Some(tx);
        st.config.dub_preference = DubPreference::PreferDubbed;
        st.live_playback.audio_tracks = vec![
            LiveMediaTrack {
                id: 1,
                kind: TrackKind::Audio,
                lang: Some("en".into()),
                title: Some("Original".into()),
                is_dubbed: false,
                is_original: true,
                selected: false,
            },
            LiveMediaTrack {
                id: 2,
                kind: TrackKind::Audio,
                lang: Some("ar".into()),
                title: Some("Arabic dubbed".into()),
                is_dubbed: true,
                is_original: false,
                selected: true,
            },
        ];

        term.draw(|f| render_play_options_modal(f, &mut st)).unwrap();
        let all_text: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(all_text.contains("[LIVE mpv IPC]"));
        assert!(all_text.contains("DUB: ON (EN)"));
        assert!(all_text.contains("[DUB]"));
    }
}
