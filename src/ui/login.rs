//! Google Account Hub & Browser-Popup Sign-In Modal (`[L]`).

use crate::services::auth::{now_unix_secs, LoginStep, GOOGLE_YOUTUBE_SERVICE_LOGIN_URL};
use crate::state::{centered_rect, ActiveOverlay, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

pub fn render_login_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = crate::state::active_modal_rect(frame.area(), ActiveOverlay::Login)
        .unwrap_or_else(|| centered_rect(84, 78, frame.area()));
    frame.render_widget(Clear, area);

    let ui = &state.login_ui;
    let status_pill = if state.auth_vault.is_authenticated() && ui.step == LoginStep::Hub {
        " SIGNED IN "
    } else if ui.busy || ui.step == LoginStep::AwaitingBrowserPopup {
        " SIGNING IN "
    } else {
        " ○ GUEST "
    };

    let outer = Block::default()
        .title(Span::styled(
            format!(" PHAENNA — GOOGLE ACCOUNT ·{status_pill}"),
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),   // Main panel
            Constraint::Length(3), // Live banner (status / error)
            Constraint::Length(2), // Action keybinds footer
        ])
        .split(inner);

    match ui.step {
        LoginStep::Hub => {
            if state.auth_vault.is_authenticated() {
                render_signed_in_hub(frame, chunks[0], state);
            } else {
                render_anonymous_popup_prompt(frame, chunks[0], state);
            }
        }
        LoginStep::AwaitingBrowserPopup => render_awaiting_browser_popup(frame, chunks[0], state),
    }

    render_status_banner(frame, chunks[1], state);
    render_footer_hints(frame, chunks[2], state);
}

fn render_anonymous_popup_prompt(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let card = Block::default()
        .title(Span::styled(
            " [Enter] Sign In with Google (Browser Popup) ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Green));

    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            "  Fast, isolated Google Sign-In via Browser Popup",
            Style::default()
                .fg(theme.text)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  Press [Enter] to open a clean Google Sign-In popup window.",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "  Once you finish logging in, Phaenna captures your YouTube session",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "  automatically and closes the popup window.",
            Style::default().fg(theme.text),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  • Passkeys, Touch ID, Security Keys & 2FA work natively",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(Span::styled(
            "  • Uses an isolated temporary browser profile (zero daily-browser scraping)",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(Span::styled(
            "  • Saved exclusively to ~/.config/phaenna/auth.json (mode 0600)",
            Style::default().fg(theme.text_dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  ▶ Press [Enter] to launch Browser Sign-In Popup",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(card)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn render_awaiting_browser_popup(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let launcher = state
        .login_ui
        .popup_launcher
        .as_deref()
        .unwrap_or("Browser popup");

    let blk = Block::default()
        .title(Span::styled(
            " BROWSER SIGN-IN POPUP ACTIVE ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Green));

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Launcher : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                launcher,
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  1. Complete Google Sign-In inside the popup window (Passkeys / 2FA supported).",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "  2. When YouTube finishes loading, Phaenna saves your session into",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "     ~/.config/phaenna/auth.json (0600) and closes the popup automatically.",
            Style::default().fg(theme.text_dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            format!("  Target   : {GOOGLE_YOUTUBE_SERVICE_LOGIN_URL}"),
            Style::default().fg(theme.text_dim),
        )),
    ];

    frame.render_widget(
        Paragraph::new(lines).block(blk).wrap(Wrap { trim: true }),
        area,
    );
}

fn render_signed_in_hub(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let Some(active) = state.auth_vault.active_account() else {
        return;
    };

    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(9), Constraint::Min(5)])
        .split(area);

    let ttl_str = match active.token_ttl_secs(now_unix_secs()) {
        Some(secs) if secs > 60 => format!("Valid ~{}m (Auto-refreshing)", secs / 60),
        Some(_) => "Refreshing soon (auto)".to_string(),
        None => "Session cookie jar active".to_string(),
    };
    let vault_str = crate::config::Config::auth_vault_path()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "~/.config/phaenna/auth.json".to_string());

    let profile_blk = Block::default()
        .title(Span::styled(
            " ACTIVE GOOGLE IDENTITY ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Green));

    let profile_lines = vec![
        Line::from(vec![
            Span::styled("  Account   : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                &active.display_name,
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                active
                    .channel_handle
                    .as_deref()
                    .map(|h| format!(" ({h})"))
                    .unwrap_or_default(),
                Style::default().fg(theme.secondary),
            ),
        ]),
        Line::from(vec![
            Span::styled("  Email     : ", Style::default().fg(theme.text_dim)),
            Span::styled(&active.email, Style::default().fg(theme.text)),
        ]),
        Line::from(vec![
            Span::styled("  Method    : ", Style::default().fg(theme.text_dim)),
            Span::styled(active.auth_kind.label(), Style::default().fg(theme.primary)),
        ]),
        Line::from(vec![
            Span::styled("  Token TTL : ", Style::default().fg(theme.text_dim)),
            Span::styled(ttl_str, Style::default().fg(theme.accent)),
        ]),
        Line::from(vec![
            Span::styled("  Vault     : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!("{vault_str} (mode 0600)"),
                Style::default().fg(theme.text_dim),
            ),
        ]),
        Line::from(vec![
            Span::styled("  InnerTube : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                "Home (FEwhat_to_watch) · Subs · History · Watch Later sync",
                Style::default().fg(theme.text),
            ),
        ]),
    ];
    frame.render_widget(Paragraph::new(profile_lines).block(profile_blk), vchunks[0]);

    let accts_blk = Block::default()
        .title(Span::styled(
            format!(
                " LINKED ACCOUNTS ({}) — [↑/↓] Select · [Enter] Switch · [+] Add via Popup · [x] Sign Out ",
                state.auth_vault.accounts.len()
            ),
            Style::default().fg(theme.secondary),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));

    let mut acct_lines = Vec::new();
    for (idx, acct) in state.auth_vault.accounts.iter().enumerate() {
        let is_selected = idx == state.login_ui.account_cursor;
        let is_active = state.auth_vault.active_account_id.as_deref() == Some(&acct.account_id);
        let marker = if is_active { "●" } else { "○" };
        let prefix = if is_selected { "▶ " } else { "  " };
        let st = if is_selected {
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        acct_lines.push(Line::from(vec![
            Span::styled(format!(" {prefix}{marker} "), st),
            Span::styled(format!("{} <{}> ", acct.display_name, acct.email), st),
            Span::styled(
                format!("[{}]", acct.auth_kind.label()),
                Style::default().fg(theme.text_dim),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(acct_lines).block(accts_blk), vchunks[1]);
}

fn render_status_banner(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let ui = &state.login_ui;
    let (msg, color) = if let Some(ref err) = ui.error_banner {
        (format!(" ✕ {err}"), Color::LightRed)
    } else if let Some(ref st) = ui.status_banner {
        (format!(" {st}"), Color::Green)
    } else {
        (
            " Vault is isolated inside ~/.config/phaenna — zero daily-browser scraping.".to_string(),
            theme.text_dim,
        )
    };

    let banner = Paragraph::new(msg)
        .alignment(Alignment::Center)
        .style(Style::default().fg(color))
        .wrap(Wrap { trim: true })
        .block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(theme.divider)),
        );
    frame.render_widget(banner, area);
}

fn render_footer_hints(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let hints = match state.login_ui.step {
        LoginStep::Hub if state.auth_vault.is_authenticated() => {
            "[↑/↓] Select · [Enter] Switch · [r] Refresh · [+] Add Account · [x] Sign Out · [Esc] Close"
        }
        LoginStep::Hub => "[Enter] Open Google Browser Sign-In Popup · [Esc] Close",
        LoginStep::AwaitingBrowserPopup => {
            "Sign in inside the browser popup — auto-connects when YouTube loads · [Esc] Cancel"
        }
    };

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            hints,
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::services::auth::{AuthKind, GoogleAccountSession};
    use ratatui::{backend::TestBackend, Terminal};
    use std::collections::HashMap;

    fn render_to_string(state: &AppState) -> String {
        let backend = TestBackend::new(110, 34);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render_login_modal(f, state)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn test_login_modal_renders_browser_popup_and_signed_in_hub() {
        let mut state = AppState::new(Config::default());
        state.auth_vault = crate::services::auth::AuthVault::default();
        state.overlay = ActiveOverlay::Login;

        // 1. Guest Hub shows single Browser Popup method
        let hub_txt = render_to_string(&state);
        assert!(hub_txt.contains("GOOGLE ACCOUNT"));
        assert!(hub_txt.contains("Sign In with Google (Browser Popup)"));
        assert!(!hub_txt.contains("Phone QR"));
        assert!(!hub_txt.contains("In-TUI Email"));

        // 2. Awaiting Browser Popup
        state.login_ui.step = LoginStep::AwaitingBrowserPopup;
        state.login_ui.popup_launcher = Some("Chromium popup".into());
        let popup_txt = render_to_string(&state);
        assert!(popup_txt.contains("BROWSER SIGN-IN POPUP ACTIVE"));
        assert!(popup_txt.contains("Chromium popup"));

        // 3. Signed-In Hub
        state.login_ui.step = LoginStep::Hub;
        state.auth_vault.upsert_session(GoogleAccountSession {
            account_id: "yt:captain@gmail.com".into(),
            email: "captain@gmail.com".into(),
            display_name: "Phaenna Captain".into(),
            channel_id: Some("UC999".into()),
            channel_handle: Some("@captain".into()),
            avatar_url: None,
            auth_kind: AuthKind::FirstParty,
            access_token: None,
            refresh_token: None,
            expires_at: None,
            cookies: HashMap::new(),
            netscape_cookies: None,
            added_at: 1,
            last_refreshed: 1,
        });
        let signed_txt = render_to_string(&state);
        assert!(signed_txt.contains("Phaenna Captain"));
        assert!(signed_txt.contains("captain@gmail.com"));
        assert!(signed_txt.contains("SIGNED IN"));
    }
}
