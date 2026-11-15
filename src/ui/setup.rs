//! System Doctor & Onboarding Checklist (`!`).
//! Audits runtime dependencies (`yt-dlp`, `mpv`, `iina`, `vlc`, `ffmpeg`, cookies)
//! with clear copy-paste install commands.

use crate::state::{centered_rect, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render_setup_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(74, 68, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " PHAENNA — SYSTEM DOCTOR & RUNTIME CHECK ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.secondary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([Constraint::Min(10), Constraint::Length(2)])
        .split(inner);

    let badge = |ok: bool| -> Span<'static> {
        if ok {
            Span::styled(" [OK] READY ", Style::default().fg(theme.accent).add_modifier(Modifier::BOLD))
        } else {
            Span::styled(" ✕ MISSING ", Style::default().fg(Color::Rgb(247, 118, 142)).add_modifier(Modifier::BOLD))
        }
    };

    let d = &state.deps;
    let lines = vec![
        Line::from(Span::styled("Core Engine & Video Backends:", Style::default().fg(theme.primary).add_modifier(Modifier::BOLD))),
        Line::from(""),
        Line::from(vec![
            badge(d.has_ytdlp),
            Span::styled("yt-dlp       ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled(
                d.ytdlp_path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "Install: brew install yt-dlp / pipx install yt-dlp".to_string()),
                Style::default().fg(theme.text_dim),
            ),
        ]),
        Line::from(vec![
            badge(d.has_mpv),
            Span::styled("mpv          ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled(
                d.mpv_path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| "Install: brew install mpv (enables IPC + SponsorBlock + GLSL)".to_string()),
                Style::default().fg(theme.text_dim),
            ),
        ]),
        Line::from(vec![
            badge(d.has_iina),
            Span::styled("IINA         ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled("macOS hardware player (optional alternative)", Style::default().fg(theme.text_dim)),
        ]),
        Line::from(vec![
            badge(d.has_vlc),
            Span::styled("VLC          ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled("Cross-platform fallback media player", Style::default().fg(theme.text_dim)),
        ]),
        Line::from(vec![
            badge(d.has_ffmpeg),
            Span::styled("ffmpeg       ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled("Muxing offline vault downloads + VAD subtitle alignment", Style::default().fg(theme.text_dim)),
        ]),
        Line::from(vec![
            badge(state.auth_vault.is_authenticated()),
            Span::styled("Google Auth  ", Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled(
                if let Some(acct) = state.auth_vault.active_account() {
                    format!(
                        "Signed in as {} ({}) · Zero browser scraping",
                        acct.display_name, acct.email
                    )
                } else {
                    "Press [L] to sign in directly inside Phaenna (Email/2FA, Device QR, or Kitty SSO)".to_string()
                },
                Style::default().fg(theme.text_dim),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled("Platform & Binary Info:", Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD))),
        Line::from(Span::styled(
            format!("  Version: v{} · Git SHA: {} · OS: {}", env!("CARGO_PKG_VERSION"), env!("PHAENNA_GIT_SHA"), std::env::consts::OS),
            Style::default().fg(theme.text_dim),
        )),
    ];

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), rows[0]);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" [r] Re-Scan Binaries ", Style::default().fg(theme.primary)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[L] Google Account ", Style::default().fg(theme.secondary)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[Esc] Close ", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[1],
    );
}
