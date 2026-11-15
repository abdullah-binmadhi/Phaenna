//! Offline Vault Downloader Modal (`D`).

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph},
    Frame,
};

pub fn render_download_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(68, 52, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " ⬇ OFFLINE VAULT DOWNLOADER ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(3),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(inner);

    if let Some(ref job) = state.active_download {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Target : ", Style::default().fg(theme.text_dim)),
                    Span::styled(&job.title, Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
                ]),
                Line::from(vec![
                    Span::styled("Vault  : ", Style::default().fg(theme.text_dim)),
                    Span::styled(
                        format!(
                            "Speed: {} · Saved entries: {}",
                            if job.speed.is_empty() { "muxing..." } else { &job.speed },
                            state.offline_paths.len()
                        ),
                        Style::default().fg(theme.secondary),
                    ),
                ]),
            ]),
            rows[0],
        );

        let ratio = (job.percent / 100.0).clamp(0.0, 1.0) as f64;
        let gauge = Gauge::default()
            .block(Block::default().borders(Borders::ALL).border_type(BorderType::Rounded))
            .gauge_style(Style::default().fg(theme.accent).bg(theme.border_idle))
            .ratio(ratio)
            .label(format!("{:.1}%", job.percent));
        frame.render_widget(gauge, rows[1]);

        let status = if let Some(ref p) = job.done_path {
            Span::styled(format!("[OK] Saved to {}", p), Style::default().fg(theme.accent))
        } else if let Some(ref e) = job.error {
            Span::styled(format!("✕ Failed: {}", e), Style::default().fg(Color::Rgb(247, 118, 142)))
        } else {
            Span::styled("Downloading & embedding chapters in background...", Style::default().fg(theme.text_dim))
        };
        frame.render_widget(Paragraph::new(status), rows[2]);
    } else {
        frame.render_widget(
            Paragraph::new("No active download job. Select any video and press [D] to download to ~/Movies/Phaenna/.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            rows[2],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            " [Esc] Minimize to background & continue browsing ",
            Style::default().fg(theme.primary),
        )))
        .alignment(Alignment::Center),
        rows[3],
    );
}
