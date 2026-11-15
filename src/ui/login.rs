//! Account Cookie Sync & Authentication Modal (`L`).

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render_login_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(68, 60, frame.area());
    frame.render_widget(Clear, area);

    let block = Block::default()
        .title(Span::styled(
            " 🔑 PHAENNA — ACCOUNT & COOKIE VAULT ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(inner);

    let cookie_path = state
        .config
        .cookies_path()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| "Unavailable".to_string());

    let auth_line = if state.cookie_count > 0 {
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!("✓ Active ({} YouTube cookies)", state.cookie_count),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · Source: ", Style::default().fg(theme.text_dim)),
            Span::styled(state.config.cookie_source.label(), Style::default().fg(theme.secondary)),
        ])
    } else {
        Line::from(vec![
            Span::styled("Status: ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled(
                "✕ Anonymous Mode (Public feeds only)",
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            ),
        ])
    };

    frame.render_widget(
        Paragraph::new(vec![
            auth_line,
            Line::from(vec![
                Span::styled("Vault : ", Style::default().fg(theme.text_dim)),
                Span::styled(cookie_path, Style::default().fg(theme.text_dim)),
            ]),
        ]),
        chunks[0],
    );

    let mut opts = vec![
        Line::from(vec![
            Span::styled(" [1] ", Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD)),
            Span::styled(" Native SQLite Cookie Sync (Zen / Firefox -> Netscape)", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled("     Extracts encrypted-free SQLite session rows directly in pure Rust.", Style::default().fg(theme.text_dim))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [2] ", Style::default().fg(Color::Black).bg(theme.secondary).add_modifier(Modifier::BOLD)),
            Span::styled(" Open accounts.google.com in Default Browser", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(Span::styled("     Sign into YouTube in Zen or Firefox, then press [1] to re-sync.", Style::default().fg(theme.text_dim))),
        Line::from(""),
        Line::from(vec![
            Span::styled(" [3] ", Style::default().fg(Color::Black).bg(theme.accent).add_modifier(Modifier::BOLD)),
            Span::styled(
                format!(" Cycle Cookie Source Mode: {}", state.config.cookie_source.label()),
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ]),
    ];

    if let Some(ref msg) = state.login_status {
        opts.push(Line::from(""));
        opts.push(Line::from(Span::styled(
            msg,
            Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
        )));
    }

    frame.render_widget(Paragraph::new(opts).wrap(Wrap { trim: true }), chunks[1]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [1/2/3] Trigger Action ", Style::default().fg(theme.primary)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[Esc / L] Close ", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        chunks[2],
    );
}
