//! Mobile LAN Companion Remote QR Code Modal (`m`).

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph},
    Frame,
};

pub fn render_remote_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(64, 78, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " LAN MOBILE REMOTE COMPANION ",
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
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(inner);

    if let Some(ref handle) = state.remote_handle {
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("Scan with phone camera on the same Wi-Fi: ", Style::default().fg(theme.text)),
                ]),
                Line::from(Span::styled(
                    &handle.url,
                    Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
                )),
            ])
            .alignment(Alignment::Center),
            rows[0],
        );

        let qr_lines: Vec<Line> = handle
            .qr_lines
            .iter()
            .map(|r| Line::from(Span::styled(r.clone(), Style::default().fg(Color::White))))
            .collect();
        frame.render_widget(
            Paragraph::new(qr_lines).alignment(Alignment::Center),
            rows[1],
        );
    } else {
        frame.render_widget(
            Paragraph::new(
                "LAN Remote server is disabled or failed to bind.\nEnable `remote.enabled = true` in config or launch without `--no-remote`.",
            )
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
            rows[1],
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [o] Open in Local Browser ", Style::default().fg(theme.secondary)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[Esc / m] Close ", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}
