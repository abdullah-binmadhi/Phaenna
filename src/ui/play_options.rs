//! Unified 6-Zone (`3x2`) Playback Studio & Manual Mastering Deck (`p`).
//! Controls Quality, Player Backend, Audio Dubbing (`d`), Captions, SponsorBlock,
//! Live Stream Profiles (`v`), 4K Shaders (`s`), Performance Modes (`m`),
//! 10-Point Manual Thermal/GPU Controls (`a`), Picture EQ, and Live Transport.

use crate::config::{
    dub_lang_label, GlslStageMode, PerformanceMode, Player, Quality, SponsorBlockMode,
    SuperResMode, VideoShaderMode,
};
use crate::services::shaders::neural_shader_installed;
use crate::state::{
    active_modal_rect, play_options_modal_rect, ActiveOverlay, AppState, ManualToggle,
    NowPlayingHit, PlayOptionsHit,
};
use crate::ui::home::marquee_window;
use crate::ui::theme::ThemePalette;
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

fn truncate_words(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    if max_chars <= 3 {
        return trimmed.chars().take(max_chars).collect();
    }
    let limit = max_chars - 3;
    let mut byte_end = 0usize;
    let mut last_space = None;
    for (i, (b_idx, ch)) in trimmed.char_indices().enumerate() {
        if i >= limit {
            break;
        }
        byte_end = b_idx + ch.len_utf8();
        if ch.is_whitespace() {
            last_space = Some(b_idx);
        }
    }
    let cut = last_space.filter(|&p| p >= byte_end / 2).unwrap_or(byte_end);
    format!("{}...", trimmed[..cut].trim_end())
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

fn push_split_line_hits(
    hitboxes: &mut Vec<(Rect, PlayOptionsHit)>,
    inner: Rect,
    row_offset: u16,
    left_hit: PlayOptionsHit,
    right_hit: PlayOptionsHit,
) {
    if row_offset < inner.height && inner.width > 4 {
        let mid = inner.width / 2;
        hitboxes.push((Rect::new(inner.x, inner.y + row_offset, mid, 1), left_hit));
        hitboxes.push((
            Rect::new(inner.x + mid, inner.y + row_offset, inner.width - mid, 1),
            right_hit,
        ));
    }
}

fn push_panel_hit(hitboxes: &mut Vec<(Rect, PlayOptionsHit)>, rect: Rect, section: usize) {
    if rect.width > 0 && rect.height > 0 {
        hitboxes.push((rect, PlayOptionsHit::FocusSection(section)));
    }
}

#[allow(clippy::too_many_arguments)]
fn studio_kv_row(
    label: &str,
    val: &str,
    tag_opt: Option<&str>,
    label_w: usize,
    highlighted: bool,
    val_style: Style,
    inner_w: u16,
    theme: &ThemePalette,
) -> Line<'static> {
    let prefix = if highlighted { ">" } else { " " };
    let lbl_fit = truncate_words(label, label_w);
    let left_col = format!("{prefix}{lbl_fit:<label_w$} │ ");
    let left_chars = left_col.chars().count();

    let tag_col = tag_opt
        .filter(|_| (inner_w as usize) >= left_chars + 16)
        .map(|t| format!(" [{t}] "))
        .unwrap_or_default();
    let tag_chars = tag_col.chars().count();

    let val_budget = (inner_w as usize).saturating_sub(left_chars + tag_chars);
    let val_fit = truncate_words(val, val_budget);
    let val_padded = format!("{val_fit:<val_budget$}");

    let lbl_style = if highlighted {
        Style::default()
            .fg(Color::Black)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text_dim)
    };
    let effective_val_style = if highlighted {
        val_style.add_modifier(Modifier::BOLD)
    } else {
        val_style
    };

    let mut spans = vec![
        Span::styled(
            format!("{prefix}{lbl_fit:<label_w$}"),
            lbl_style,
        ),
        Span::styled(" │ ", Style::default().fg(theme.divider)),
        Span::styled(val_padded, effective_val_style),
    ];
    if !tag_col.is_empty() {
        spans.push(Span::styled(tag_col, Style::default().fg(theme.text_dim)));
    }
    Line::from(spans)
}

fn mode_badge_color(pm: PerformanceMode) -> Color {
    match pm {
        PerformanceMode::Efficient => Color::Rgb(52, 211, 153),
        PerformanceMode::Balanced => Color::Rgb(56, 189, 248),
        PerformanceMode::Quality => Color::Rgb(251, 191, 36),
        PerformanceMode::Manual => Color::Rgb(192, 132, 252),
    }
}

pub fn render_play_options_modal(frame: &mut Frame, state: &mut AppState) {
    let theme = *state.base_theme();
    let area = active_modal_rect(frame.area(), ActiveOverlay::PlayOptions)
        .unwrap_or_else(|| play_options_modal_rect(frame.area()));
    frame.render_widget(Clear, area);
    state.live_playback.play_hitboxes.clear();

    let live = state.has_live_ipc();
    let is_live_stream = if live {
        state.live_playback.is_live_stream
    } else {
        state
            .active_selected_video()
            .map(|v| v.is_live_stream())
            .unwrap_or(false)
    };
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
    let title_budget = area.width.saturating_sub(68).max(12) as usize;
    let display_title = marquee_window(&video_title, title_budget, state.ticker_step);
    let badge_text = if live { " [LIVE mpv IPC] " } else { " [PRE-LAUNCH] " };
    let mode_badge = format!(
        " [MODE: {}] ",
        state.config.performance_mode.label().to_uppercase()
    );
    let mode_bg = mode_badge_color(state.config.performance_mode);

    let mut title_spans = vec![
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
                .bg(if live {
                    Color::Rgb(52, 211, 153)
                } else {
                    theme.secondary
                })
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            mode_badge,
            Style::default()
                .fg(Color::Black)
                .bg(mode_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if is_live_stream {
        title_spans.push(Span::styled(
            format!(" [{}] ", state.config.live_mode.short_badge()),
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(248, 113, 113))
                .add_modifier(Modifier::BOLD),
        ));
    }

    let outer = Block::default()
        .title(Line::from(title_spans))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    if inner.width < 40 || inner.height < 16 {
        return;
    }

    let v_chunks = if inner.height >= 31 {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),  // Transport banner
                Constraint::Length(15), // Row 1: Zones 1, 2, 3 (13 inner + 2 border)
                Constraint::Length(12), // Row 2: Zones 4, 5, 6 (10 inner + 2 border)
                Constraint::Min(1),     // Interactive keybar
            ])
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Percentage(52),
                Constraint::Min(8),
                Constraint::Length(1),
            ])
            .split(inner)
    };

    render_transport_banner(frame, v_chunks[0], state);

    let row1_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(32),
            Constraint::Percentage(35),
            Constraint::Percentage(33),
        ])
        .split(v_chunks[1]);

    render_stream_engine_pane(frame, row1_cols[0], state);
    render_audio_dub_pane(frame, row1_cols[1], state);
    render_captions_sponsor_live_pane(frame, row1_cols[2], state);

    let row2_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(32),
            Constraint::Percentage(36),
            Constraint::Percentage(32),
        ])
        .split(v_chunks[2]);

    render_shaders_mode_pane(frame, row2_cols[0], state);
    render_manual_mastering_pane(frame, row2_cols[1], state);
    render_live_controls_pane(frame, row2_cols[2], state);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [<-/->/Tab] Zone ", Style::default().fg(theme.primary)),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled("[Up/Down] Row ", Style::default().fg(theme.secondary)),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled(
                "[m] Mode ",
                Style::default()
                    .fg(Color::Rgb(56, 189, 248))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled(
                "[a] Manual ",
                Style::default()
                    .fg(Color::Rgb(192, 132, 252))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled("[s] Shader ", Style::default().fg(theme.primary)),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled("[d] Dub ", Style::default().fg(theme.accent)),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled("[v] Live ", Style::default().fg(Color::Rgb(248, 113, 113))),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled(
                if live {
                    "[Enter] Relaunch/Apply "
                } else {
                    "[Enter] Launch Video "
                },
                Style::default()
                    .fg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled("[Esc] Close ", Style::default().fg(theme.text_dim)),
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
        let eff_master = state.config.effective_mastering();
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
            Span::styled(
                q.short_badge(),
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" via {} ", state.config.player.as_str()),
                Style::default().fg(theme.text),
            ),
            Span::styled("| ", Style::default().fg(theme.divider)),
            Span::styled(
                eff_master.codec_priority.label(),
                Style::default().fg(theme.secondary),
            ),
            Span::styled(" | ", Style::default().fg(theme.divider)),
            Span::styled(
                if eff_master.hdr_compute_peak {
                    "HDR Dynamic"
                } else {
                    "HDR Static Cool"
                },
                Style::default().fg(Color::Rgb(52, 211, 153)),
            ),
            Span::styled(" | ", Style::default().fg(theme.divider)),
            Span::styled(
                eff_master.super_res.label(),
                Style::default().fg(theme.accent),
            ),
        ]);
        frame.render_widget(Paragraph::new(summary), cols[0]);
        frame.render_widget(
            Paragraph::new(Span::styled(
                dub_pill,
                Style::default()
                    .fg(Color::Black)
                    .bg(if dub_on {
                        Color::Rgb(251, 191, 36)
                    } else {
                        theme.accent
                    })
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
        state
            .live_playback
            .play_hitboxes
            .push((cols[1], PlayOptionsHit::ToggleDub));
        state
            .live_playback
            .play_hitboxes
            .push((cols[2], PlayOptionsHit::LaunchOrRelaunch));
        return;
    }

    let play_lbl = if state.live_playback.paused {
        " [Play] "
    } else {
        " [Pause] "
    };
    let back_lbl = " [-10s] ";
    let fwd_lbl = " [+10s] ";
    let ch_prev_lbl = " [<Ch] ";
    let ch_next_lbl = " [Ch>] ";
    let vol_lbl = if state.live_playback.muted {
        " MUTE ".to_string()
    } else {
        format!(
            " Spd:{:.2}x Vol:{}% ",
            state.live_playback.speed, state.live_playback.volume
        )
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
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(back_lbl, Style::default().fg(theme.text))),
        cols[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(fwd_lbl, Style::default().fg(theme.text))),
        cols[2],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(ch_prev_lbl, Style::default().fg(theme.text_dim))),
        cols[3],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(ch_next_lbl, Style::default().fg(theme.text_dim))),
        cols[4],
    );

    let pos = state.active_playback_pos;
    let dur = state.active_playback_dur.max(0.0);
    let ratio = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let time_str = format!(" {} / {} ", format_hms(pos), format_hms(dur));
    let bar_w = cols[5].width.saturating_sub(time_str.len() as u16 + 2) as usize;
    let filled = ((bar_w as f64) * ratio).round() as usize;
    let track: String = (0..bar_w)
        .map(|i| {
            if i == filled.min(bar_w.saturating_sub(1)) {
                '●'
            } else if i < filled {
                '━'
            } else {
                '─'
            }
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" {track}"),
                Style::default().fg(Color::Rgb(52, 211, 153)),
            ),
            Span::styled(
                time_str,
                Style::default()
                    .fg(theme.text)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        cols[5],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(vol_lbl, Style::default().fg(theme.secondary))),
        cols[6],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            dub_pill,
            Style::default()
                .fg(Color::Black)
                .bg(if dub_on {
                    Color::Rgb(251, 191, 36)
                } else {
                    Color::Rgb(52, 211, 153)
                })
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
        hits.push((cols[5], PlayOptionsHit::SeekRatio(-1.0)));
    }
    hits.push((cols[7], PlayOptionsHit::ToggleDub));
}

fn section_block<'a>(title: &'a str, focused: bool, theme: &'a ThemePalette) -> Block<'a> {
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

/// Zone 1: Stream Quality & Player Backend (`8 + 1 + 4 = 13` rows exact).
fn render_stream_engine_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.play_modal_section == 0;
    let blk = section_block(" 1. Stream Quality & Player ", focused, &theme);
    let inner = blk.inner(area);

    let mut lines: Vec<Line> = Vec::new();
    let players = [Player::Mpv, Player::Iina, Player::Vlc, Player::Browser];
    let hitboxes = &mut state.live_playback.play_hitboxes;
    let text_budget = inner.width.saturating_sub(4) as usize;

    for (i, q) in Quality::ALL.iter().enumerate() {
        let active = i == state.play_quality_idx;
        let st = if active && focused {
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else if active {
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        let row = lines.len() as u16;
        let lbl = truncate_words(q.label(), text_budget);
        lines.push(Line::from(Span::styled(
            format!(" {} {lbl:<text_budget$} ", if active { ">" } else { " " }),
            st,
        )));
        push_line_hit(hitboxes, inner, row, PlayOptionsHit::SelectQuality(i));
    }

    lines.push(Line::from(Span::styled(
        " Player Backend:",
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
    )));
    for pl in players {
        let active = pl == state.config.player;
        let row = lines.len() as u16;
        let lbl = truncate_words(pl.label(), text_budget);
        lines.push(Line::from(Span::styled(
            format!(" {} {lbl:<text_budget$} ", if active { "*" } else { "o" }),
            if active {
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            },
        )));
        push_line_hit(hitboxes, inner, row, PlayOptionsHit::SelectPlayer(pl));
    }
    push_panel_hit(hitboxes, area, 0);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

/// Zone 2: Audio, Dubbing & Live Audio Tracks (`13` rows balanced).
fn render_audio_dub_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.play_modal_section == 1;
    let blk = section_block(" 2. Audio, Dubbing & Tracks [d] ", focused, &theme);
    let inner = blk.inner(area);

    let live = state.has_live_ipc();
    let dub_on = state.config.dub_preference.is_dub_on();
    let mut lines: Vec<Line> = Vec::new();
    let hits = &mut state.live_playback.play_hitboxes;
    let label_w = if inner.width >= 44 { 14 } else { 11 };

    let r0 = lines.len() as u16;
    lines.push(studio_kv_row(
        "Dubbed Audio",
        state.config.dub_preference.label(),
        None,
        label_w,
        false,
        Style::default()
            .fg(if dub_on {
                Color::Rgb(251, 191, 36)
            } else {
                Color::Rgb(52, 211, 153)
            })
            .add_modifier(Modifier::BOLD),
        inner.width,
        &theme,
    ));
    push_line_hit(hits, inner, r0, PlayOptionsHit::ToggleDub);

    let r1 = lines.len() as u16;
    lines.push(studio_kv_row(
        "Dub Language",
        &format!("{} [L]", dub_lang_label(&state.config.dub_lang)),
        None,
        label_w,
        false,
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
        inner.width,
        &theme,
    ));
    push_line_hit(hits, inner, r1, PlayOptionsHit::CycleDubLang);

    let r2 = lines.len() as u16;
    lines.push(studio_kv_row(
        "Multi-Track",
        if state.config.audio_multistreams {
            "ON (Instant Switch)"
        } else {
            "OFF (Single Stream)"
        },
        None,
        label_w,
        false,
        Style::default().fg(if state.config.audio_multistreams {
            theme.primary
        } else {
            theme.text_dim
        }),
        inner.width,
        &theme,
    ));
    push_line_hit(hits, inner, r2, PlayOptionsHit::ToggleMultistreams);

    let r3 = lines.len() as u16;
    lines.push(studio_kv_row(
        "Night Loudnorm",
        if state.config.audio_loudnorm {
            "ON [n]"
        } else {
            "OFF [n]"
        },
        None,
        label_w,
        false,
        Style::default().fg(if state.config.audio_loudnorm {
            Color::Rgb(52, 211, 153)
        } else {
            theme.text
        }),
        inner.width,
        &theme,
    ));
    push_line_hit(hits, inner, r3, PlayOptionsHit::ToggleLoudnorm);

    let r4 = lines.len() as u16;
    lines.push(studio_kv_row(
        "Audio Delay",
        &format!("{:+} ms  ([-] / [+])", state.live_playback.audio_delay_ms),
        None,
        label_w,
        false,
        Style::default().fg(theme.text),
        inner.width,
        &theme,
    ));
    push_split_line_hits(
        hits,
        inner,
        r4,
        PlayOptionsHit::AudioDelayDelta(-50),
        PlayOptionsHit::AudioDelayDelta(50),
    );

    let total_tracks = state.live_playback.audio_tracks.len();
    lines.push(Line::from(Span::styled(
        if total_tracks > 0 {
            format!(" Live Audio Tracks ({total_tracks}):")
        } else {
            " Audio Routing & Stream Info:".to_string()
        },
        Style::default()
            .fg(theme.primary)
            .add_modifier(Modifier::BOLD),
    )));

    if total_tracks == 0 {
        let info_rows = [
            (
                "Track Status",
                if live {
                    "Single audio stream active"
                } else {
                    "Inspects tracks upon launch"
                },
            ),
            (
                "Preferred Role",
                if dub_on {
                    "Prefer AI / Dubbed audio track"
                } else {
                    "Strictly keep Creator Original"
                },
            ),
            (
                "Dynamic Norm",
                if state.config.audio_loudnorm {
                    "dynaudnorm f=150 active"
                } else {
                    "Bit-exact unprocessed mix"
                },
            ),
            (
                "Offset Step",
                "Left half -50ms / Right +50ms",
            ),
            (
                "Live Switch",
                "Press [d] Dub or [L] Cycle Lang",
            ),
            (
                "Night Filter",
                "Press [n] Toggle Loudnorm",
            ),
            (
                "Multi-Stream",
                "Click Multi-Track row to toggle",
            ),
        ];
        for (lbl, val) in info_rows {
            if (lines.len() as u16) < inner.height {
                lines.push(studio_kv_row(
                    lbl,
                    val,
                    None,
                    label_w,
                    false,
                    Style::default().fg(theme.text_dim),
                    inner.width,
                    &theme,
                ));
            }
        }
    } else {
        let max_rows = (inner.height as usize).saturating_sub(lines.len());
        let reserve_more = usize::from(total_tracks > max_rows && max_rows > 1);
        let visible_cap = max_rows.saturating_sub(reserve_more).max(1);
        let max_scroll = total_tracks.saturating_sub(visible_cap);
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
            let st = if active {
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.primary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            let max_lbl = inner.width.saturating_sub(4) as usize;
            let lbl = truncate_words(&tr.display_label(), max_lbl);
            lines.push(Line::from(Span::styled(
                format!(" {} {lbl:<max_lbl$} ", if active { ">" } else { " " }),
                st,
            )));
            push_line_hit(hits, inner, row, PlayOptionsHit::SelectAudioTrack(tr.id));
        }
        if total_tracks > visible_cap {
            let row = lines.len() as u16;
            let hidden_above = scroll;
            let hidden_below = total_tracks.saturating_sub(scroll + visible_cap);
            lines.push(Line::from(Span::styled(
                format!("  +{hidden_above} above / +{hidden_below} below [Scroll/Click]"),
                Style::default().fg(theme.text_dim),
            )));
            push_line_hit(hits, inner, row, PlayOptionsHit::ScrollAudioTracks(1));
        } else {
            let filler = [
                ("Dub Hotkey", "Press [d] Toggle Dub / Orig"),
                ("Lang Cycle", "Press [L] Next Dub Language"),
                ("Delay Step", "Left half -50ms / Right +50ms"),
                ("Night Norm", "Press [n] Toggle Loudnorm"),
                ("Multi-Stream", "Click Multi-Track row to toggle"),
            ];
            for (lbl, val) in filler {
                if (lines.len() as u16) >= inner.height {
                    break;
                }
                lines.push(studio_kv_row(
                    lbl,
                    val,
                    None,
                    label_w,
                    false,
                    Style::default().fg(theme.text_dim),
                    inner.width,
                    &theme,
                ));
            }
        }
    }
    push_panel_hit(hits, area, 1);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

/// Zone 3: Captions, SponsorBlock & Live Stream Engine (`3 + 1 + 4 + 1 + 4 = 13` rows exact).
fn render_captions_sponsor_live_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.play_modal_section == 2;
    let blk = section_block(" 3. Captions, Sponsor & Live [c/v] ", focused, &theme);
    let inner = blk.inner(area);

    let live = state.has_live_ipc();
    let mut lines: Vec<Line> = Vec::new();
    let hits = &mut state.live_playback.play_hitboxes;
    let label_w = if inner.width >= 44 { 13 } else { 11 };

    let sub_on = if live {
        state.live_playback.sub_visible
    } else {
        state.config.subtitles_enabled
    };
    let next_sub_track = if !state.live_playback.sub_tracks.is_empty() {
        let cur_pos = state
            .live_playback
            .sub_tracks
            .iter()
            .position(|t| state.live_playback.active_sid == Some(t.id) || t.selected)
            .unwrap_or(0);
        let next = &state.live_playback.sub_tracks[(cur_pos + 1) % state.live_playback.sub_tracks.len()];
        Some((next.id, next.lang.clone().unwrap_or_else(|| format!("#{}", next.id))))
    } else {
        None
    };
    let r0 = lines.len() as u16;
    let sub_val = if let Some((_, ref next_lbl)) = next_sub_track {
        if sub_on {
            format!("ON [c] | [Sub->{next_lbl}]")
        } else {
            format!("OFF [c] | [Sub->{next_lbl}]")
        }
    } else if sub_on {
        "ON [c]".to_string()
    } else {
        "OFF [c]".to_string()
    };
    lines.push(studio_kv_row(
        "Subtitles",
        &sub_val,
        None,
        label_w,
        false,
        Style::default()
            .fg(if sub_on { theme.accent } else { theme.text_dim })
            .add_modifier(Modifier::BOLD),
        inner.width,
        &theme,
    ));
    if let Some((next_sid, _)) = next_sub_track {
        push_split_line_hits(
            hits,
            inner,
            r0,
            PlayOptionsHit::ToggleSubtitles,
            PlayOptionsHit::SelectSubTrack(next_sid),
        );
    } else {
        push_line_hit(hits, inner, r0, PlayOptionsHit::ToggleSubtitles);
    }

    let r1 = lines.len() as u16;
    lines.push(studio_kv_row(
        "Style",
        &format!("{} [y]", state.config.subtitle_style.preset.label()),
        None,
        label_w,
        false,
        Style::default().fg(theme.secondary),
        inner.width,
        &theme,
    ));
    push_line_hit(hits, inner, r1, PlayOptionsHit::CycleSubStyle);

    let r2 = lines.len() as u16;
    lines.push(studio_kv_row(
        "Sub Offset",
        &format!("{:+} ms  ([-] / [+])", state.live_playback.sub_delay_ms),
        None,
        label_w,
        false,
        Style::default().fg(theme.text),
        inner.width,
        &theme,
    ));
    push_split_line_hits(
        hits,
        inner,
        r2,
        PlayOptionsHit::SubDelayDelta(-100),
        PlayOptionsHit::SubDelayDelta(100),
    );

    lines.push(Line::from(Span::styled(
        " SponsorBlock Mode:",
        Style::default()
            .fg(theme.accent)
            .add_modifier(Modifier::BOLD),
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
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            },
        )));
        push_line_hit(hits, inner, row, PlayOptionsHit::SelectSponsorBlock(mode));
    }

    let r_outro = lines.len() as u16;
    lines.push(studio_kv_row(
        "Skip Outros",
        if state.config.skip_outro {
            "ON [o]"
        } else {
            "OFF (Protected) [o]"
        },
        None,
        label_w,
        false,
        Style::default()
            .fg(if state.config.skip_outro {
                Color::Rgb(251, 191, 36)
            } else {
                Color::Rgb(52, 211, 153)
            })
            .add_modifier(Modifier::BOLD),
        inner.width,
        &theme,
    ));
    push_line_hit(hits, inner, r_outro, PlayOptionsHit::ToggleSkipOutro);

    // 4 Live Stream Mode rows filling the bottom of Zone 3 cleanly
    if (lines.len() as u16) < inner.height {
        let row_live = lines.len() as u16;
        lines.push(studio_kv_row(
            "Live [v]",
            state.config.live_mode.label(),
            None,
            label_w,
            false,
            Style::default()
                .fg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
            inner.width,
            &theme,
        ));
        push_line_hit(hits, inner, row_live, PlayOptionsHit::CycleLiveMode);
    }
    if (lines.len() as u16) < inner.height {
        let row_cu = lines.len() as u16;
        lines.push(studio_kv_row(
            "Live Catchup",
            if state.config.live_auto_catchup {
                "ON (Auto 1.04x+)"
            } else {
                "OFF (Manual Speed)"
            },
            None,
            label_w,
            false,
            Style::default().fg(theme.secondary),
            inner.width,
            &theme,
        ));
        push_line_hit(hits, inner, row_cu, PlayOptionsHit::ToggleLiveCatchup);
    }
    if (lines.len() as u16) < inner.height {
        let row_act = lines.len() as u16;
        lines.push(Line::from(vec![
            Span::styled(
                " [E Live Edge] ",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                " [S From Start] ",
                Style::default()
                    .fg(theme.primary)
                    .add_modifier(Modifier::BOLD),
            ),
        ]));
        push_split_line_hits(
            hits,
            inner,
            row_act,
            PlayOptionsHit::JumpToLiveEdge,
            PlayOptionsHit::PlayLiveFromStart,
        );
    }
    if (lines.len() as u16) < inner.height {
        let row_rad = lines.len() as u16;
        lines.push(Line::from(vec![
            Span::styled(
                " [R Audio Radio] ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" (Low-Power Stream)", Style::default().fg(theme.text_dim)),
        ]));
        push_line_hit(hits, inner, row_rad, PlayOptionsHit::LaunchAudioRadio);
    }
    push_panel_hit(hits, area, 2);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

/// Zone 4: Performance Mode Switcher & 4K Shader Pipeline (`10` inner rows).
fn render_shaders_mode_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.play_modal_section == 3;
    let title = if state.has_live_ipc() {
        " 4. Mode & 4K Shaders [m/s] (Live) "
    } else {
        " 4. Mode & 4K Shaders [m/s] "
    };
    let blk = section_block(title, focused, &theme);
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let eff_master = state.config.effective_mastering();
    let hits = &mut state.live_playback.play_hitboxes;

    // Row 0: Horizontal clickable Mode pills [ECO] [BALANCED] [QUALITY] [MANUAL]
    let row0_rect = Rect::new(inner.x, inner.y, inner.width, 1);
    let pill_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(10), // " Mode[m]: "
            Constraint::Percentage(19),
            Constraint::Percentage(27),
            Constraint::Percentage(27),
            Constraint::Percentage(27),
        ])
        .split(row0_rect);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " Mode[m]: ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
        pill_cols[0],
    );
    for (idx, pm) in PerformanceMode::ALL.iter().enumerate() {
        let active = *pm == state.config.performance_mode;
        let col = mode_badge_color(*pm);
        let lbl = if (pill_cols[idx + 1].width as usize) >= pm.label().len() + 2 {
            format!(" {} ", pm.label())
        } else {
            format!(" {} ", pm.short_badge())
        };
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
            pill_cols[idx + 1],
        );
        hits.push((pill_cols[idx + 1], PlayOptionsHit::SetPerformanceMode(*pm)));
    }

    // Rows 1..=9 below Row 0
    let body_area = Rect::new(
        inner.x,
        inner.y.saturating_add(1),
        inner.width,
        inner.height.saturating_sub(1),
    );
    let mut lines: Vec<Line> = Vec::new();
    let sum_fit = truncate_words(
        state.config.performance_mode.summary(),
        inner.width.saturating_sub(2) as usize,
    );
    lines.push(Line::from(Span::styled(
        format!(" {sum_fit}"),
        Style::default().fg(theme.text_dim),
    )));
    lines.push(Line::from(Span::styled(
        " 4K Mastering Shader [s]:",
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD),
    )));

    let all_shaders = [
        (VideoShaderMode::Reference4K, "Reference 4K", "Deblock+4:4:4+RCAS"),
        (VideoShaderMode::NeuralCinema, "Neural Cinema", "FSRCNNX CNN+4:4:4"),
        (VideoShaderMode::Anime4K, "Anime4K", "Line restore 2x"),
        (VideoShaderMode::Balanced, "Balanced", "Deblock+CAS+Deband"),
        (VideoShaderMode::Off, "Off", "Raw native stream"),
    ];
    for (m, name, desc) in all_shaders {
        let active = m == state.config.shader_mode;
        let row = lines.len() as u16;
        let name_w = 14usize;
        let desc_budget = (inner.width as usize).saturating_sub(name_w + 6);
        let d_fit = truncate_words(desc, desc_budget);
        lines.push(Line::from(vec![
            Span::styled(
                format!(" {} {name:<name_w$}", if active { ">" } else { " " }),
                if active {
                    Style::default()
                        .fg(Color::Black)
                        .bg(theme.primary)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text)
                },
            ),
            Span::styled(" │ ", Style::default().fg(theme.divider)),
            Span::styled(d_fit, Style::default().fg(theme.text_dim)),
        ]));
        push_line_hit(hits, body_area, row, PlayOptionsHit::SelectShader(m));
    }

    if (lines.len() as u16) < body_area.height {
        let chain_desc = match (state.config.shader_mode, eff_master.glsl_stage) {
            (VideoShaderMode::Off, _) | (_, GlslStageMode::Off) => "Raw decode (0 GLSL)",
            (_, GlslStageMode::LightCas) => "00 Light Luma CAS",
            (_, GlslStageMode::Guided444) => "Deblock + Guided 4:4:4 (1x)",
            (_, GlslStageMode::FullMastering) => match eff_master.super_res {
                SuperResMode::Off => "Deblock + Guided 4:4:4 (1x)",
                SuperResMode::Auto1080p => "4:4:4 + 2x (<=1080p)",
                SuperResMode::Always2x => "Deblock + 4:4:4 + 2x",
            },
        };
        lines.push(studio_kv_row(
            "Chain",
            chain_desc,
            None,
            8,
            false,
            Style::default().fg(theme.secondary),
            body_area.width,
            &theme,
        ));
    }

    if (lines.len() as u16) < body_area.height {
        let row_cnn = lines.len() as u16;
        let installed = neural_shader_installed();
        lines.push(studio_kv_row(
            "CNN Pack",
            if installed {
                "FSRCNNX Installed"
            } else {
                "[Click to Download CNN]"
            },
            None,
            8,
            false,
            Style::default().fg(if installed {
                Color::Rgb(52, 211, 153)
            } else {
                theme.accent
            }),
            body_area.width,
            &theme,
        ));
        if !installed {
            push_line_hit(hits, body_area, row_cnn, PlayOptionsHit::DownloadNeuralShaders);
        }
    }
    push_panel_hit(hits, area, 3);

    frame.render_widget(Paragraph::new(lines), body_area);
}

/// Zone 5: Manual & Thermal Controls `[a]` (`10` inner rows exact).
fn render_manual_mastering_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.play_modal_section == 4;
    let is_manual = state.config.performance_mode == PerformanceMode::Manual;
    let title = if is_manual {
        " 5. Manual & Heat Controls [a] (ACTIVE) "
    } else {
        " 5. Manual & Heat Controls [a] (Click/Toggle) "
    };
    let blk = section_block(title, focused, &theme);
    let inner = blk.inner(area);

    let eff = state.config.effective_mastering();
    let hdr_val = if eff.hdr_compute_peak {
        "ON (Dynamic Peak)"
    } else {
        "OFF (Static Cool)"
    };
    let hdr_col = if eff.hdr_compute_peak {
        Color::Rgb(251, 191, 36)
    } else {
        Color::Rgb(52, 211, 153)
    };
    let superres_col = match eff.super_res {
        SuperResMode::Off => Color::Rgb(52, 211, 153),
        SuperResMode::Auto1080p => Color::Rgb(56, 189, 248),
        SuperResMode::Always2x => Color::Rgb(251, 191, 36),
    };
    let codec_col = match eff.codec_priority {
        crate::config::CodecPriority::Vp9HwFirst => Color::Rgb(52, 211, 153),
        crate::config::CodecPriority::Av1First => Color::Rgb(251, 191, 36),
    };

    let rows_spec: [(ManualToggle, &str, &str, &str, Color); 10] = [
        (ManualToggle::SuperRes, "5K 2x Upscale", eff.super_res.label(), "GPU", superres_col),
        (ManualToggle::CodecPriority, "Codec Prio", eff.codec_priority.label(), "CPU", codec_col),
        (ManualToggle::Hwdec, "Hardware Dec", eff.hwdec.label(), "Dec", theme.secondary),
        (ManualToggle::HdrComputePeak, "HDR Peak Comp", hdr_val, "HDR", hdr_col),
        (ManualToggle::ToneMap, "HDR Tone Map", eff.tone_map.label(), "Map", theme.text),
        (ManualToggle::GlslStage, "4:4:4 & Deblk", eff.glsl_stage.label(), "GLSL", theme.primary),
        (ManualToggle::Deband, "Deband Filter", eff.deband.label(), "Band", theme.text),
        (ManualToggle::GpuScaler, "GPU Scaler", eff.gpu_scaler.label(), "Scl", theme.text),
        (ManualToggle::Motion, "120Hz Motion", state.config.motion_mode.label(), "Hz", theme.secondary),
        (ManualToggle::DemuxerCache, "Demuxer Cache", eff.demuxer_cache.label(), "RAM", theme.accent),
    ];

    let hits = &mut state.live_playback.play_hitboxes;
    let label_w = if inner.width >= 44 { 13 } else { 11 };
    let mut lines = Vec::new();
    for (idx, (toggle, lbl, val, tag, col)) in rows_spec.into_iter().enumerate() {
        let row_off = lines.len() as u16;
        let highlighted = focused && state.manual_mastering_cursor == idx;
        lines.push(studio_kv_row(
            lbl,
            val,
            Some(tag),
            label_w,
            highlighted,
            Style::default().fg(col),
            inner.width,
            &theme,
        ));
        push_line_hit(hits, inner, row_off, PlayOptionsHit::ToggleManual(toggle));
    }
    push_panel_hit(hits, area, 4);

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

/// Zone 6: Live Transport, Picture EQ, Seed Presets & Session (`10` inner rows).
fn render_live_controls_pane(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let focused = state.play_modal_section == 5;
    let live = state.has_live_ipc();
    let is_live_stream = state.live_playback.is_live_stream;
    let live_lag = state.live_playback.live_edge_lag_secs(state.active_playback_pos);
    let cache_ahead_secs = state.live_playback.cache_ahead_secs;
    let cache_mb = state.live_playback.cache_bytes / (1024 * 1024);
    let dropped_frames = state.live_playback.dropped_frames;
    let max_mb = state.config.effective_demuxer_max_bytes_num() / (1024 * 1024);

    let blk = section_block(" 6. Transport, Picture & Session ", focused, &theme);
    let inner = blk.inner(area);
    let hits = &mut state.live_playback.play_hitboxes;
    frame.render_widget(blk, area);

    if inner.width < 20 || inner.height < 6 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0: Telemetry
            Constraint::Length(1), // 1: Speed presets
            Constraint::Length(1), // 2: Volume
            Constraint::Length(1), // 3: Bri / Con
            Constraint::Length(1), // 4: Sat / Gam
            Constraint::Length(1), // 5: Reset Picture EQ
            Constraint::Length(1), // 6: OnTop / Loop
            Constraint::Length(1), // 7: Seed Manual from Presets
            Constraint::Length(1), // 8: Launch / Relaunch
            Constraint::Min(1),    // 9: Stop Player
        ])
        .split(inner);

    // Row 0: Telemetry
    let telem = if live && is_live_stream {
        format!(" Edge: -{live_lag:.1}s | Buf: {cache_ahead_secs:.1}s ({cache_mb}M) | Drp:{dropped_frames}")
    } else if live {
        format!(" Buf: {cache_mb}/{max_mb}MB | Drp: {dropped_frames} | IPC Ready")
    } else {
        format!(" Max Buf: {max_mb}MB | Engine: {}", state.config.player.as_str())
    };
    frame.render_widget(
        Paragraph::new(Span::styled(
            truncate_words(&telem, inner.width as usize),
            Style::default().fg(theme.text_dim),
        )),
        rows[0],
    );

    // Row 1: Speed presets
    let speeds = [0.75_f64, 1.0, 1.25, 1.5, 2.0];
    let mut spd_cols = vec![Constraint::Length(8)];
    spd_cols.extend(std::iter::repeat_n(Constraint::Length(7), speeds.len()));
    spd_cols.push(Constraint::Min(0));
    let sp_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(spd_cols)
        .split(rows[1]);
    frame.render_widget(
        Paragraph::new(Span::styled(" Speed:", Style::default().fg(theme.text_dim))),
        sp_rects[0],
    );
    for (idx, spd) in speeds.iter().enumerate() {
        let active = (state.live_playback.speed - *spd).abs() < 0.04;
        let label = if (*spd - 2.0).abs() < 0.01 {
            " [2x] ".to_string()
        } else {
            format!("[{spd:.2}]")
        };
        frame.render_widget(
            Paragraph::new(Span::styled(
                label,
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

    // Row 2: Volume & Mute
    let vol_rects = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Min(8),
            Constraint::Length(5),
            Constraint::Length(9),
        ])
        .split(rows[2]);
    frame.render_widget(
        Paragraph::new(Span::styled(" Vol  :", Style::default().fg(theme.text_dim))),
        vol_rects[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(" [-] ", Style::default().fg(theme.secondary))),
        vol_rects[1],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}%", state.live_playback.volume),
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        )),
        vol_rects[2],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(" [+] ", Style::default().fg(theme.secondary))),
        vol_rects[3],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.muted {
                "[UNMUTE]"
            } else {
                " [MUTE] "
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
        vol_rects[4],
    );
    hits.push((vol_rects[1], PlayOptionsHit::VolumeDelta(-5)));
    hits.push((vol_rects[3], PlayOptionsHit::VolumeDelta(5)));
    hits.push((vol_rects[4], PlayOptionsHit::ToggleMute));

    // Rows 3 & 4: 2x2 Picture EQ (Bri/Con, Sat/Gam with generous spacing & left/right click)
    let r3_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[3]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Bri: {:+<4} [+5]", state.live_playback.brightness),
            Style::default().fg(theme.text),
        )),
        r3_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Con: {:+<4} [+5]", state.live_playback.contrast),
            Style::default().fg(theme.text),
        )),
        r3_cols[1],
    );
    hits.push((r3_cols[0], PlayOptionsHit::PictureDelta("brightness", 5)));
    hits.push((r3_cols[1], PlayOptionsHit::PictureDelta("contrast", 5)));

    let r4_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[4]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Sat: {:+<4} [+5]", state.live_playback.saturation),
            Style::default().fg(theme.text),
        )),
        r4_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" Gam: {:+<4} [+5]", state.live_playback.gamma),
            Style::default().fg(theme.text),
        )),
        r4_cols[1],
    );
    hits.push((r4_cols[0], PlayOptionsHit::PictureDelta("saturation", 5)));
    hits.push((r4_cols[1], PlayOptionsHit::PictureDelta("gamma", 5)));

    // Row 5: Reset Picture EQ
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" Pic EQ │ ", Style::default().fg(theme.text_dim)),
            Span::styled(
                "[0 Reset Picture Equalizer]",
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD),
            ),
        ])),
        rows[5],
    );
    hits.push((rows[5], PlayOptionsHit::ResetPicture));

    // Row 6: Window Pinning
    let r6_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[6]);
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
        r6_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            if state.live_playback.loop_video {
                " [Loop: ON]"
            } else {
                " [Loop: OFF]"
            },
            Style::default().fg(if state.live_playback.loop_video {
                theme.primary
            } else {
                theme.text
            }),
        )),
        r6_cols[1],
    );
    hits.push((r6_cols[0], PlayOptionsHit::ToggleOnTop));
    hits.push((r6_cols[1], PlayOptionsHit::ToggleLoop));

    // Row 7: Seed Manual preset from Balanced / Quality
    let r7_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(rows[7]);
    frame.render_widget(
        Paragraph::new(Span::styled(
            " [Seed Balanced]",
            Style::default().fg(Color::Rgb(56, 189, 248)),
        )),
        r7_cols[0],
    );
    frame.render_widget(
        Paragraph::new(Span::styled(
            " [Seed Quality]",
            Style::default().fg(Color::Rgb(251, 191, 36)),
        )),
        r7_cols[1],
    );
    hits.push((
        r7_cols[0],
        PlayOptionsHit::SeedManualFrom(PerformanceMode::Balanced),
    ));
    hits.push((
        r7_cols[1],
        PlayOptionsHit::SeedManualFrom(PerformanceMode::Quality),
    ));

    // Row 8: Relaunch / Launch
    frame.render_widget(
        Paragraph::new(Span::styled(
            if live {
                " [Enter] Relaunch / Apply Flags"
            } else {
                " [Enter] Launch Stream Now"
            },
            Style::default()
                .fg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        )),
        rows[8],
    );
    hits.push((rows[8], PlayOptionsHit::LaunchOrRelaunch));

    // Row 9: Stop Active Player
    frame.render_widget(
        Paragraph::new(Span::styled(
            " [X] Stop Active Player",
            Style::default()
                .fg(Color::Rgb(239, 68, 68))
                .add_modifier(Modifier::BOLD),
        )),
        rows[9],
    );
    hits.push((rows[9], PlayOptionsHit::StopPlayer));
    push_panel_hit(hits, area, 5);
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
    let is_live_str = state.live_playback.is_live_stream;
    let time_lbl = if is_live_str {
        let lag = state.live_playback.live_edge_lag_secs(state.active_playback_pos);
        format!("Edge:-{lag:.1}s Buf:{:.1}s", state.live_playback.cache_ahead_secs)
    } else {
        format!(
            "{}/{}",
            format_hms(state.active_playback_pos),
            format_hms(state.active_playback_dur.max(0.0))
        )
    };

    let bg = Color::Rgb(18, 22, 32);
    let mut spans: Vec<(String, Style, Option<NowPlayingHit>)> = vec![
        (
            if is_live_str { " LIVE " } else { " PLAY " }.to_string(),
            Style::default()
                .fg(Color::Black)
                .bg(if is_live_str { Color::Rgb(248, 113, 113) } else { Color::Rgb(52, 211, 153) })
                .add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::OpenStudio),
        ),
        (format!(" {time_lbl} "), Style::default().fg(theme.text).bg(bg).add_modifier(Modifier::BOLD), None),
        (format!(" {play_lbl} "), Style::default().fg(theme.primary).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::TogglePause)),
        ("[-10]".to_string(), Style::default().fg(theme.text_dim).bg(bg), Some(NowPlayingHit::SeekBack)),
        (" ".to_string(), Style::default().bg(bg), None),
        ("[+10]".to_string(), Style::default().fg(theme.text_dim).bg(bg), Some(NowPlayingHit::SeekFwd)),
        (" ".to_string(), Style::default().bg(bg), None),
    ];
    if is_live_str {
        spans.push((
            "[E Edge]".to_string(),
            Style::default().fg(Color::Rgb(52, 211, 153)).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::JumpToLiveEdge),
        ));
        spans.push((" ".to_string(), Style::default().bg(bg), None));
        spans.push((
            format!("[{}]", state.config.live_mode.short_badge()),
            Style::default().fg(Color::Rgb(248, 113, 113)).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::CycleLiveMode),
        ));
        spans.push((" ".to_string(), Style::default().bg(bg), None));
    }
    spans.extend([
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
        (
            format!("[{}]", state.config.performance_mode.short_badge()),
            Style::default().fg(Color::Rgb(56, 189, 248)).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::CyclePerfMode),
        ),
        (" ".to_string(), Style::default().bg(bg), None),
        (
            format!("[{}]", state.config.motion_mode.short_badge()),
            Style::default().fg(theme.secondary).bg(bg).add_modifier(Modifier::BOLD),
            Some(NowPlayingHit::CycleMotion),
        ),
        (" ".to_string(), Style::default().bg(bg), None),
        (shader_lbl, Style::default().fg(theme.primary).bg(bg), Some(NowPlayingHit::CycleShader)),
        (" [Studio(p)]".to_string(), Style::default().fg(theme.accent).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::OpenStudio)),
        (" [Stop] ".to_string(), Style::default().fg(Color::Rgb(239, 68, 68)).bg(bg).add_modifier(Modifier::BOLD), Some(NowPlayingHit::StopPlayer)),
    ]);

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
        assert!(all_text.contains("[MODE: BALANCED]"));
        assert!(all_text.contains("ECO"));
        assert!(all_text.contains("DUB: ON (EN)"));
        assert!(all_text.contains("[DUB]"));
    }

    #[test]
    fn test_playback_studio_6_zones_zero_dead_space_and_no_word_concat() {
        let mut st = AppState::new(Config::default());
        st.config.performance_mode = PerformanceMode::Manual;
        st.play_modal_section = 4;
        st.manual_mastering_cursor = 2;

        let full_area = Rect::new(0, 0, 150, 36);
        let modal = play_options_modal_rect(full_area);
        assert_eq!(modal.width, 146);
        assert_eq!(modal.height, 33);
        assert_eq!(active_modal_rect(full_area, ActiveOverlay::PlayOptions), Some(modal));

        let backend = TestBackend::new(150, 36);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_play_options_modal(f, &mut st)).unwrap();

        let buf = term.backend().buffer();
        let mut rows_text = Vec::new();
        for y in modal.y..modal.y + modal.height {
            let line: String = (modal.x..modal.x + modal.width)
                .map(|x| buf.cell((x, y)).map(|c| c.symbol()).unwrap_or(" "))
                .collect();
            let chars: Vec<char> = line.chars().collect();
            let n = chars.len();
            // Skip top/bottom transport border rows when testing thirds? Even border rows have corners/lines!
            let third = n / 3;
            let c1: String = chars[1..third].iter().collect();
            let c2: String = chars[third..2 * third].iter().collect();
            let c3: String = chars[2 * third..n - 1].iter().collect();
            assert!(!c1.trim().is_empty(), "Dead left column at y={y}: {line:?}");
            assert!(!c2.trim().is_empty(), "Dead middle column at y={y}: {line:?}");
            assert!(!c3.trim().is_empty(), "Dead right column at y={y}: {line:?}");
            rows_text.push(line);
        }

        let joined = rows_text.join("\n");
        for header in [
            "1. Stream Quality & Player",
            "2. Audio, Dubbing & Tracks",
            "3. Captions, Sponsor & Live",
            "4. Mode & 4K Shaders",
            "5. Manual & Heat Controls",
            "6. Transport, Picture & Session",
        ] {
            assert!(joined.contains(header), "Missing zone title: {header}");
        }

        for manual_label in [
            "5K 2x Upscale",
            "Codec Prio",
            "Hardware Dec",
            "HDR Peak Comp",
            "HDR Tone Map",
            "4:4:4 & Deblk",
            "Deband Filter",
            "GPU Scaler",
            "120Hz Motion",
            "Demuxer Cache",
        ] {
            assert!(joined.contains(manual_label), "Missing manual control: {manual_label}");
        }

        assert!(joined.contains("[Seed Balanced]"));
        assert!(joined.contains("[Seed Quality]"));
        assert!(joined.contains("[MODE: MANUAL]"));
        // Verify zero truncated ellipses inside Manual panel values at full width
        let manual_lines: Vec<&str> = rows_text
            .iter()
            .filter(|l| l.contains("5K 2x Upscale") || l.contains("HDR Peak Comp") || l.contains("Demuxer Cache"))
            .map(|s| s.as_str())
            .collect();
        for ml in manual_lines {
            assert!(ml.contains(" │ "), "Missing column divider spacing: {ml:?}");
            assert!(!ml.contains("..."), "Unexpected value truncation at full width: {ml:?}");
        }

        // Verify compact terminal fallback fits cleanly without panic
        let compact = TestBackend::new(110, 28);
        let mut compact_term = Terminal::new(compact).unwrap();
        compact_term
            .draw(|f| render_play_options_modal(f, &mut st))
            .unwrap();
    }
}
