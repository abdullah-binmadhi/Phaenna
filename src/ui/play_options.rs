//! 4-Column Play Options & Launch Matrix (`p`).
//! Configure Quality Preset (8 tiers), Player Backend, SponsorBlock Mode, Subtitles, and 4K Shaders.

use crate::config::{Player, Quality, SponsorBlockMode, VideoShaderMode};
use crate::state::{centered_rect, AppState};
use crate::ui::home::marquee_window;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render_play_options_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(90, 68, frame.area());
    frame.render_widget(Clear, area);

    let video_title = state
        .active_selected_video()
        .map(|v| v.title)
        .unwrap_or_else(|| "Selected Video".to_string());
    let title_budget = area.width.saturating_sub(24).max(16) as usize;
    let display_title = marquee_window(&video_title, title_budget, state.ticker_step);

    let outer = Block::default()
        .title(Span::styled(
            format!(" PLAY OPTIONS — {} ", display_title),
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(12), Constraint::Length(2)])
        .split(inner);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(28),
            Constraint::Percentage(23),
            Constraint::Percentage(21),
            Constraint::Percentage(28),
        ])
        .split(v_chunks[0]);

    let sec = state.play_modal_section;

    // Col 0: Quality Presets
    let q_block = Block::default()
        .title(Span::styled(
            " 1. Quality / Codec ",
            Style::default()
                .fg(if sec == 0 { theme.primary } else { theme.text_dim })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if sec == 0 {
            theme.border_focus
        } else {
            theme.border_idle
        }));

    let q_lines: Vec<Line> = Quality::ALL
        .iter()
        .enumerate()
        .map(|(i, q)| {
            let active = i == state.play_quality_idx;
            let st = if active && sec == 0 {
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
            Line::from(Span::styled(
                format!(" {} {}", if active { "▸" } else { " " }, q.label()),
                st,
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(q_lines).block(q_block), cols[0]);

    // Col 1: Player Backend
    let p_block = Block::default()
        .title(Span::styled(
            " 2. Player ",
            Style::default()
                .fg(if sec == 1 { theme.primary } else { theme.text_dim })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if sec == 1 {
            theme.border_focus
        } else {
            theme.border_idle
        }));

    let players = [Player::Mpv, Player::Iina, Player::Vlc, Player::Browser];
    let p_lines: Vec<Line> = players
        .iter()
        .map(|pl| {
            let active = *pl == state.config.player;
            let st = if active && sec == 1 {
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.secondary)
                    .add_modifier(Modifier::BOLD)
            } else if active {
                Style::default()
                    .fg(theme.secondary)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            Line::from(Span::styled(
                format!(" {} {}", if active { "●" } else { "○" }, pl.label()),
                st,
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(p_lines).block(p_block), cols[1]);

    // Col 2: SponsorBlock
    let sb_block = Block::default()
        .title(Span::styled(
            " 3. SponsorBlock ",
            Style::default()
                .fg(if sec == 2 { theme.primary } else { theme.text_dim })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if sec == 2 {
            theme.border_focus
        } else {
            theme.border_idle
        }));

    let sb_modes = [
        SponsorBlockMode::AutoSkipAll,
        SponsorBlockMode::SponsorOnly,
        SponsorBlockMode::ShowChaptersOnly,
        SponsorBlockMode::Off,
    ];
    let sb_lines: Vec<Line> = sb_modes
        .iter()
        .map(|m| {
            let active = *m == state.config.sponsorblock_mode;
            let st = if active && sec == 2 {
                Style::default()
                    .fg(Color::Black)
                    .bg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else if active {
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            Line::from(Span::styled(
                format!(" {} {}", if active { "▸" } else { " " }, m.label()),
                st,
            ))
        })
        .collect();
    frame.render_widget(Paragraph::new(sb_lines).block(sb_block), cols[2]);

    // Col 3: Captions & Reference 4K Shaders
    let sub_block = Block::default()
        .title(Span::styled(
            " 4. Captions & 4K FX ",
            Style::default()
                .fg(if sec == 3 { theme.primary } else { theme.text_dim })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if sec == 3 {
            theme.border_focus
        } else {
            theme.border_idle
        }));

    let all_shaders = [
        VideoShaderMode::Reference4K,
        VideoShaderMode::NeuralCinema,
        VideoShaderMode::Anime4K,
        VideoShaderMode::Balanced,
        VideoShaderMode::Off,
    ];

    let mut sub_lines = vec![
        Line::from(vec![
            Span::styled(" Subtitles: ", Style::default().fg(theme.text_dim)),
            Span::styled(
                if state.config.subtitles_enabled { "ON" } else { "OFF" },
                Style::default()
                    .fg(if state.config.subtitles_enabled {
                        theme.accent
                    } else {
                        theme.text_dim
                    })
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled(" Style    : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                state.config.subtitle_style.preset.label(),
                Style::default().fg(theme.secondary),
            ),
        ]),
        Line::from(Span::styled("", Style::default())),
        Line::from(Span::styled(
            " 4K Mastering Shader [s]:",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    for m in all_shaders {
        let active = m == state.config.shader_mode;
        let st = if active {
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        sub_lines.push(Line::from(Span::styled(
            format!(" {} {}", if active { "▸" } else { " " }, m.short_label()),
            st,
        )));
    }
    sub_lines.push(Line::from(Span::styled("", Style::default())));
    sub_lines.push(Line::from(Span::styled(
        format!(" {}", state.config.shader_mode.detail_summary()),
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(
        Paragraph::new(sub_lines)
            .wrap(Wrap { trim: true })
            .block(sub_block),
        cols[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [←/→] Column ", Style::default().fg(theme.primary)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[↑/↓] Option ", Style::default().fg(theme.secondary)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[s] Cycle 4K Shader ", Style::default().fg(theme.primary)),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled(
                "[Enter] Launch Video ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("· ", Style::default().fg(theme.divider)),
            Span::styled("[Esc] Cancel ", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        v_chunks[1],
    );
}
