//! 4-Phase In-App Self-Updater Modal (`U`).

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap},
    Frame,
};

pub fn render_update_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(66, 54, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " ⬆ PHAENNA — SELF-UPDATER ",
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

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(5),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(inner);

    if let Some(ref info) = state.available_update {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Current: ", Style::default().fg(theme.text_dim)),
                    Span::styled(format!("v{}", info.current_version), Style::default().fg(theme.text)),
                    Span::styled("  ->  Latest: ", Style::default().fg(theme.text_dim)),
                    Span::styled(&info.latest_version, Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("Asset  : ", Style::default().fg(theme.text_dim)),
                    Span::styled(&info.asset_name, Style::default().fg(theme.secondary)),
                ]),
            ]),
            rows[0],
        );

        let notes = info
            .release_notes
            .as_deref()
            .unwrap_or("Bug fixes, performance enhancements, and Solus parity upgrades.");
        frame.render_widget(
            Paragraph::new(notes)
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.text_dim)),
            rows[1],
        );

        if state.update_progress > 0.0 {
            let gauge = Gauge::default()
                .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded))
                .gauge_style(Style::default().fg(theme.primary).bg(theme.border_idle))
                .ratio((state.update_progress / 100.0).clamp(0.0, 1.0) as f64)
                .label(state.update_status.clone().unwrap_or_default());
            frame.render_widget(gauge, rows[2]);
        } else if let Some(ref st) = state.update_status {
            frame.render_widget(
                Paragraph::new(st.as_str()).style(Style::default().fg(theme.accent)),
                rows[2],
            );
        }
    } else {
        frame.render_widget(
            Paragraph::new(format!(
                "You are running v{} (SHA {}).\nPress [r] to check GitHub Releases for newer builds.",
                env!("CARGO_PKG_VERSION"),
                env!("PHAENNA_GIT_SHA")
            ))
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text)),
            rows[1],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter] Install Update ", Style::default().fg(theme.accent)),
            Span::styled("· [r] Re-Check ", Style::default().fg(theme.secondary)),
            Span::styled("· [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[3],
    );
}
