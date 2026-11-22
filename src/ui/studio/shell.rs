//! Persistent Studio Shell:
//! Outer frame, marquee video title + engine pills, clickable 6-page tab bar,
//! persistent mini transport + SponsorBlock-annotated scrubber, and contextual footer.

use crate::config::NetworkMode;
use crate::state::{
    active_modal_rect, play_options_modal_rect, ActiveOverlay, AppState, PlayOptionsHit, StudioPage,
};
use crate::ui::home::marquee_window;
use crate::ui::studio::widgets::{format_hms, mode_badge_color, sponsor_color};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

/// Render the persistent Playback Studio chrome and return the inner canvas `Rect`
/// for the active `StudioPage`.
pub fn render_studio_shell(frame: &mut Frame, state: &mut AppState) -> Option<Rect> {
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
    let title_budget = area.width.saturating_sub(74).max(10) as usize;
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

    if live && state.live_playback.current_stream_height > 0 {
        let codec_tag = if !state.live_playback.video_codec_live.is_empty() {
            format!(" {}", state.live_playback.video_codec_live.to_uppercase())
        } else {
            String::new()
        };
        title_spans.push(Span::styled(
            format!(
                " [{}p{:.0}{}] ",
                state.live_playback.current_stream_height,
                state.live_playback.current_stream_fps.max(30.0),
                codec_tag
            ),
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }

    if state.config.picture.preset != crate::config::PicturePreset::Neutral
        || state.live_playback.picture_bypass
    {
        let lbl = if state.live_playback.picture_bypass {
            " [EQ:BYPASS] ".to_string()
        } else {
            format!(" [EQ:{}] ", state.config.picture.preset.short_label().to_uppercase())
        };
        title_spans.push(Span::styled(
            lbl,
            Style::default()
                .fg(Color::Black)
                .bg(if state.live_playback.picture_bypass {
                    Color::Rgb(148, 163, 184)
                } else {
                    theme.secondary
                })
                .add_modifier(Modifier::BOLD),
        ));
    }

    if state.config.network_mode.is_congested() {
        let net_bg = match state.config.network_mode {
            NetworkMode::CongestedWifi => Color::Rgb(251, 191, 36),
            NetworkMode::UltraSaver => Color::Rgb(52, 211, 153),
            NetworkMode::Normal => theme.primary,
        };
        title_spans.push(Span::styled(
            format!(" [{}] ", state.config.network_mode.short_badge()),
            Style::default()
                .fg(Color::Black)
                .bg(net_bg)
                .add_modifier(Modifier::BOLD),
        ));
    }
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
        return None;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // 0: 6-page clickable tab bar
            Constraint::Length(3), // 1: Persistent Mini Transport Strip
            Constraint::Min(10),   // 2: Active Page Canvas
            Constraint::Length(1), // 3: Contextual keybar
        ])
        .split(inner);

    render_page_tabs(frame, rows[0], state);
    render_transport_banner(frame, rows[1], state);
    render_footer_keybar(frame, rows[3], state);

    Some(rows[2])
}

fn render_page_tabs(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
    let active_page = state.studio.page;
    let compact = area.width < 110;

    let mut spans = Vec::new();
    let mut x_cur = area.x;
    for (idx, page) in StudioPage::ALL.iter().enumerate() {
        let is_active = *page == active_page;
        let lbl = if compact {
            format!(" [{}] {} ", page.shortcut(), page.short_label())
        } else {
            format!(" [{}] {} ", page.shortcut(), page.label())
        };
        let w = lbl.chars().count() as u16;
        if x_cur + w <= area.x + area.width {
            state
                .live_playback
                .play_hitboxes
                .push((Rect::new(x_cur, area.y, w, 1), PlayOptionsHit::SelectPage(*page)));
            x_cur += w;
        }
        let st = if is_active {
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        spans.push(Span::styled(lbl, st));
        if idx + 1 < StudioPage::ALL.len() && x_cur + 1 < area.x + area.width {
            spans.push(Span::styled("│", Style::default().fg(theme.divider)));
            x_cur += 1;
        }
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_transport_banner(frame: &mut Frame, area: Rect, state: &mut AppState) {
    let theme = *state.base_theme();
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
        let q = crate::config::Quality::ALL
            .get(state.play_quality_idx)
            .copied()
            .unwrap_or(state.config.quality);
        let eff_master = state.config.effective_mastering();
        let net_pill = format!(" {} ", state.config.network_mode.short_badge());
        let net_bg = match state.config.network_mode {
            NetworkMode::Normal => theme.border_idle,
            NetworkMode::CongestedWifi => Color::Rgb(251, 191, 36),
            NetworkMode::UltraSaver => Color::Rgb(52, 211, 153),
        };
        let net_fg = if state.config.network_mode.is_congested() {
            Color::Black
        } else {
            theme.text
        };
        let launch_label = " [Launch Video] ";
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(20),
                Constraint::Length(net_pill.len() as u16 + 1),
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
                net_pill,
                Style::default()
                    .fg(net_fg)
                    .bg(net_bg)
                    .add_modifier(Modifier::BOLD),
            )),
            cols[1],
        );
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
            cols[2],
        );
        frame.render_widget(
            Paragraph::new(Span::styled(
                launch_label,
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            )),
            cols[3],
        );
        state
            .live_playback
            .play_hitboxes
            .push((cols[1], PlayOptionsHit::CycleNetworkMode));
        state
            .live_playback
            .play_hitboxes
            .push((cols[2], PlayOptionsHit::ToggleDub));
        state
            .live_playback
            .play_hitboxes
            .push((cols[3], PlayOptionsHit::LaunchOrRelaunch));
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
    let ahead_end = (pos + state.live_playback.cache_ahead_secs).min(dur.max(pos));
    let time_str = format!(" {} / {} ", format_hms(pos), format_hms(dur));
    let bar_w = (cols[5].width.saturating_sub(time_str.len() as u16 + 2) as usize).max(4);
    let ratio = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((bar_w as f64) * ratio).round() as usize;
    let head_idx = filled.min(bar_w.saturating_sub(1));
    let buf_end_idx = if dur > 0.0 {
        (((ahead_end / dur).clamp(0.0, 1.0)) * (bar_w as f64)).round() as usize
    } else {
        head_idx
    };

    let mut bar_spans = vec![Span::raw(" ")];
    for i in 0..bar_w {
        if i == head_idx {
            bar_spans.push(Span::styled(
                "●",
                Style::default()
                    .fg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        let seg_cat = if dur > 1.0 {
            let t = ((i as f64 + 0.5) / bar_w as f64) * dur;
            state
                .detail_sponsor_segments
                .iter()
                .find(|s| t >= s.start_time && t <= s.end_time)
                .map(|s| s.category.as_str())
        } else {
            None
        };
        if let Some(cat) = seg_cat {
            bar_spans.push(Span::styled("■", Style::default().fg(sponsor_color(cat))));
        } else if i < head_idx {
            bar_spans.push(Span::styled(
                "━",
                Style::default().fg(Color::Rgb(52, 211, 153)),
            ));
        } else if i <= buf_end_idx {
            bar_spans.push(Span::styled(
                "═",
                Style::default().fg(Color::Rgb(56, 189, 248)),
            ));
        } else {
            bar_spans.push(Span::styled("─", Style::default().fg(theme.divider)));
        }
    }
    bar_spans.push(Span::styled(
        time_str,
        Style::default()
            .fg(theme.text)
            .add_modifier(Modifier::BOLD),
    ));

    frame.render_widget(Paragraph::new(Line::from(bar_spans)), cols[5]);
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

fn render_footer_keybar(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let live = state.has_live_ipc();
    let wide = area.width >= 118;
    let page_hint = match state.studio.page {
        StudioPage::Deck => {
            if wide {
                "[g] Grade | [b] Bypass | [f] Full | [,/.] Frame | [T] Sleep | [0] Reset"
            } else {
                "[g] Grade | [b] Bypass | [f] Full | [T] Sleep"
            }
        }
        StudioPage::StreamNet => "[j/k] Tier | [C] Codec | [w] Net | [r] Probe | [A] ABR",
        StudioPage::MasteringGpu => "[m] Mode | [s] Shader | [t] 120Hz | [j/k+Space] Matrix",
        StudioPage::AudioSync => "[d] Dub | [L] Lang | [n] Norm | [e] EQ | [h/l] Delay",
        StudioPage::CaptionsSkip => "[c] Subs | [y] Style | [o] Outros | [u] Ch/SB | [h/l] Offset",
        StudioPage::LiveTelemetry => "[v] Live | [E] Edge | [S] Start | [R] Radio | [F] Filter",
    };

    let mut spans = vec![
        Span::styled(" [1-6] Page ", Style::default().fg(theme.primary)),
        Span::styled("| ", Style::default().fg(theme.divider)),
        Span::styled("[Tab] Next ", Style::default().fg(theme.secondary)),
        Span::styled("| ", Style::default().fg(theme.divider)),
        Span::styled(page_hint, Style::default().fg(theme.text)),
        Span::styled(" | ", Style::default().fg(theme.divider)),
    ];
    if wide {
        spans.push(Span::styled(
            "[a] Manual ",
            Style::default().fg(Color::Rgb(192, 132, 252)),
        ));
        spans.push(Span::styled("| ", Style::default().fg(theme.divider)));
    }
    spans.push(Span::styled(
        if live {
            "[Enter] Relaunch/Apply "
        } else {
            "[Enter] Launch Video "
        },
        Style::default()
            .fg(Color::Rgb(52, 211, 153))
            .add_modifier(Modifier::BOLD),
    ));
    spans.push(Span::styled("| ", Style::default().fg(theme.divider)));
    spans.push(Span::styled("[Esc] Close ", Style::default().fg(theme.text_dim)));

    frame.render_widget(
        Paragraph::new(Line::from(spans)).alignment(Alignment::Center),
        area,
    );
}
