//! In-App Google Identity Hub & Sign-In Modal (`[L]`).
//! Renders interactive Email/Password + 2FA forms, RFC 8628 Device Code + QR,
//! Kitty Split Loopback PKCE status, BYO OAuth Client Setup, and Signed-In Multi-Account Hub.

use crate::services::auth::{now_unix_secs, ChallengeKind, LoginStep};
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
    } else if ui.busy
        || matches!(
            ui.step,
            LoginStep::AwaitingDeviceQr | LoginStep::AwaitingKittySso
        )
    {
        " AUTHENTICATING "
    } else {
        " ○ ANONYMOUS "
    };

    let outer = Block::default()
        .title(Span::styled(
            format!(" PHAENNA — GOOGLE IDENTITY HUB ·{status_pill}"),
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
            Constraint::Min(10),   // Main step panel
            Constraint::Length(3), // Live banner (status / error)
            Constraint::Length(2), // Action keybinds footer
        ])
        .split(inner);

    match &ui.step {
        LoginStep::Hub => {
            if state.auth_vault.is_authenticated() {
                render_signed_in_hub(frame, chunks[0], state);
            } else {
                render_anonymous_hub(frame, chunks[0], state);
            }
        }
        LoginStep::EmailInput => render_email_step(frame, chunks[0], state),
        LoginStep::PasswordInput => render_password_step(frame, chunks[0], state),
        LoginStep::TwoFactorChallenge(kind) => {
            render_two_factor_step(frame, chunks[0], state, kind);
        }
        LoginStep::AwaitingDeviceQr => render_device_qr_step(frame, chunks[0], state),
        LoginStep::AwaitingKittySso => render_kitty_sso_step(frame, chunks[0], state),
        LoginStep::ClientIdSetup => render_client_setup_step(frame, chunks[0], state),
    }

    render_status_banner(frame, chunks[1], state);
    render_footer_hints(frame, chunks[2], state);
}

fn render_anonymous_hub(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let vchunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(10),
            Constraint::Length(4),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("  All sign-in happens inside Phaenna — ", Style::default().fg(theme.text)),
            Span::styled(
                "zero browser SQLite scraping",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(".", Style::default().fg(theme.text)),
        ])),
        vchunks[0],
    );

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(vchunks[1]);

    let flow_a = Block::default()
        .title(Span::styled(
            " [1/Enter] Instant Kitty Split Sign-In ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Green));
    let flow_a_lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            " Zero setup — no OAuth Client ID needed!",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Opens Google's official YouTube login inside",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            " a Kitty right split pane (terminal-browser).",
            Style::default().fg(theme.text),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  • Passkeys, Touch ID & 2FA work natively",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(Span::styled(
            "  • Auto-imports session & closes pane",
            Style::default().fg(theme.text_dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  ▶ Press [1] or [Enter] to sign in now",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    frame.render_widget(
        Paragraph::new(flow_a_lines)
            .block(flow_a)
            .wrap(Wrap { trim: false }),
        cols[0],
    );

    let flow_b = Block::default()
        .title(Span::styled(
            " [2] Option B: In-TUI Email & 2FA ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let flow_b_lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            " Type your Google email, masked password,",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            " and 6-digit 2FA OTP directly in Phaenna.",
            Style::default().fg(theme.text),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  • Memory-scrubbed ", Style::default().fg(theme.text_dim)),
            Span::styled("Zeroize", Style::default().fg(theme.accent)),
            Span::styled(" buffer", Style::default().fg(theme.text_dim)),
        ]),
        Line::from(Span::styled(
            "  • Stays 100% inside Phaenna modal",
            Style::default().fg(theme.text_dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  ▶ Press [2] to open Email Input",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    frame.render_widget(
        Paragraph::new(flow_b_lines)
            .block(flow_b)
            .wrap(Wrap { trim: false }),
        cols[1],
    );

    let flow_c = Block::default()
        .title(Span::styled(
            " [3] Advanced: Phone QR / Device Code · [o] OAuth Client ",
            Style::default().fg(theme.secondary).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let configured = state.config.google_oauth.client_id.is_some()
        || std::env::var_os("PHAENNA_GOOGLE_CLIENT_ID").is_some();
    let oauth_badge = if configured {
        Span::styled("[OK] Custom OAuth Client Configured", Style::default().fg(Color::Green))
    } else {
        Span::styled(
            "Optional — [o] Configure Custom OAuth2 Client ID for Phone QR",
            Style::default().fg(theme.text_dim),
        )
    };
    let flow_c_lines = vec![
        Line::from(vec![
            Span::styled(
                "  [3] Generate Phone QR / 8-Char Device Code ",
                Style::default().fg(theme.text),
            ),
            Span::styled("(requires custom Google OAuth Client)", Style::default().fg(theme.text_dim)),
        ]),
        Line::from(vec![Span::raw("  "), oauth_badge]),
    ];
    frame.render_widget(Paragraph::new(flow_c_lines).block(flow_c), vchunks[2]);
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
            Span::styled(format!("{vault_str} (mode 0600)"), Style::default().fg(theme.text_dim)),
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
                " LINKED ACCOUNTS ({}) — [↑/↓] Select · [Enter] Switch · [x] Sign Out ",
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

fn render_email_step(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let blk = Block::default()
        .title(Span::styled(
            " FLOW A (STEP 1/3) — GOOGLE EMAIL ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            "  Enter your Google or Workspace email address:",
            Style::default().fg(theme.text),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Email : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!(" {}█ ", state.login_ui.email),
                Style::default()
                    .fg(theme.primary)
                    .bg(theme.divider)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  Tip: Press [Enter] to continue to password, [Ctrl+U] to clear, [Esc] to cancel.",
            Style::default().fg(theme.text_dim),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(blk).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_password_step(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let ui = &state.login_ui;
    let blk = Block::default()
        .title(Span::styled(
            " FLOW A (STEP 2/3) — PASSWORD (ZEROIZE PROTECTED) ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let shown = if ui.show_password {
        ui.password.as_str().to_string()
    } else {
        ui.password.masked()
    };
    let eye_label = if ui.show_password { "VISIBLE" } else { "MASKED" };

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Account  : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                &ui.email,
                Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Password : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!(" {shown}█ "),
                Style::default()
                    .fg(theme.primary)
                    .bg(theme.divider)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(format!("  [{eye_label}]"), Style::default().fg(theme.accent)),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  Passwords stay strictly in Zeroize-wiped RAM and are never written to disk.",
            Style::default().fg(theme.text_dim),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(blk).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_two_factor_step(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    kind: &ChallengeKind,
) {
    let theme = state.base_theme();
    let ui = &state.login_ui;
    let blk = Block::default()
        .title(Span::styled(
            " FLOW A (STEP 3/3) — 2-STEP VERIFICATION ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.accent));

    let prompt_desc = match kind {
        ChallengeKind::TotpCode => "Enter the 6-digit code from your Authenticator app:".to_string(),
        ChallengeKind::SmsCode { hint } => {
            format!("Enter the 6-digit SMS code sent to {hint}:")
        }
        ChallengeKind::GooglePrompt { number: Some(n) } => {
            format!("Tap YES and number [{n}] on your phone, or enter a 6-digit OTP below:")
        }
        ChallengeKind::GooglePrompt { number: None } => {
            "Tap YES on your Google Prompt notification, or enter a 6-digit OTP below:".to_string()
        }
        ChallengeKind::BackupCode => "Enter your 8-digit one-time Google backup code:".to_string(),
    };

    let target_len = if ui.backup_mode || matches!(kind, ChallengeKind::BackupCode) {
        8
    } else {
        6
    };
    let mut padded = String::new();
    let chars: Vec<char> = ui.two_factor_code.chars().collect();
    for i in 0..target_len {
        if i == 3 || (target_len == 8 && i == 4) {
            padded.push(' ');
        }
        let ch = chars.get(i).copied().unwrap_or('_');
        padded.push(' ');
        padded.push(ch);
    }

    let lines = vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Account : ", Style::default().fg(theme.text_dim)),
            Span::styled(&ui.email, Style::default().fg(theme.text)),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            format!("  {prompt_desc}"),
            Style::default().fg(theme.text),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::raw("    [ "),
            Span::styled(
                padded,
                Style::default()
                    .fg(theme.primary)
                    .bg(theme.divider)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(" ]"),
        ]),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(blk).wrap(Wrap { trim: false }),
        area,
    );
}

fn render_device_qr_step(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let Some(ref dev) = state.login_ui.device_session else {
        frame.render_widget(
            Paragraph::new("Requesting Google Device Code & QR matrix...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.accent)),
            area,
        );
        return;
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(36), Constraint::Length(38)])
        .split(area);

    let qr_blk = Block::default()
        .title(Span::styled(
            " SCAN WITH PHONE CAMERA ",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.secondary));

    let qr_text: Vec<Line> = dev
        .qr_lines
        .iter()
        .map(|l| Line::from(Span::styled(l.as_str(), Style::default().fg(theme.text))))
        .collect();
    frame.render_widget(
        Paragraph::new(qr_text)
            .alignment(Alignment::Center)
            .block(qr_blk),
        cols[0],
    );

    let rem = (dev.expires_at - now_unix_secs()).max(0);
    let mins = rem / 60;
    let secs = rem % 60;

    let code_blk = Block::default()
        .title(Span::styled(
            " OR ENTER CODE AT GOOGLE ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let info_lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            " 1. Visit:",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(Span::styled(
            format!("    {}", dev.verification_url),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::UNDERLINED),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            " 2. Enter code:",
            Style::default().fg(theme.text_dim),
        )),
        Line::from(Span::styled(
            format!("    {}  ", dev.user_code),
            Style::default()
                .fg(theme.primary)
                .bg(theme.divider)
                .add_modifier(Modifier::BOLD),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            format!(" Polling every {}s ({mins:02}:{secs:02} left)", dev.interval_secs),
            Style::default().fg(theme.accent),
        )),
        Line::from(Span::styled(
            " Supports Passkeys & Google Prompt!",
            Style::default().fg(Color::Green),
        )),
    ];
    frame.render_widget(
        Paragraph::new(info_lines)
            .block(code_blk)
            .wrap(Wrap { trim: false }),
        cols[1],
    );
}

fn render_kitty_sso_step(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let blk = Block::default()
        .title(Span::styled(
            " KITTY RIGHT SPLIT GOOGLE SIGN-IN (ZERO SETUP) ",
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(Color::Green));

    let url = state
        .login_ui
        .kitty_auth_url
        .as_deref()
        .unwrap_or("Opening accounts.google.com inside Kitty right split...");

    let lines = vec![
        Line::raw(""),
        Line::from(Span::styled(
            "  [OK] Official Google Sign-In opened in your Kitty right split (terminal-browser)",
            Style::default().fg(Color::Green),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            "  1. Sign into Google normally on the right side (Passkeys / 2FA supported).",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "  2. As soon as Google redirects to YouTube, Phaenna imports your session",
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            "     into ~/.config/phaenna/auth.json (0600) and closes the right pane.",
            Style::default().fg(theme.text_dim),
        )),
        Line::raw(""),
        Line::from(Span::styled(
            format!("  Target: {url}"),
            Style::default().fg(theme.text_dim),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(blk).wrap(Wrap { trim: true }),
        area,
    );
}

fn render_client_setup_step(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let ui = &state.login_ui;
    let blk = Block::default()
        .title(Span::styled(
            " GOOGLE OAUTH2 CLIENT SETUP (ONE-TIME) ",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.secondary));

    let id_focus = ui.setup_field_idx == 0;
    let id_style = if id_focus {
        Style::default()
            .fg(theme.primary)
            .bg(theme.divider)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    };
    let sec_style = if !id_focus {
        Style::default()
            .fg(theme.primary)
            .bg(theme.divider)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    };

    let masked_sec = "•".repeat(ui.client_secret_input.chars().count());
    let lines = vec![
        Line::from(Span::styled(
            "  Paste your Google Cloud OAuth Client ID (TVs/Limited Input or Desktop App):",
            Style::default().fg(theme.text),
        )),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Client ID     : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!(
                    " {}{}",
                    ui.client_id_input,
                    if id_focus { "█ " } else { " " }
                ),
                id_style,
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("  Client Secret : ", Style::default().fg(theme.text_dim)),
            Span::styled(
                format!(
                    " {}{}",
                    masked_sec,
                    if !id_focus { "█ " } else { " " }
                ),
                sec_style,
            ),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "  Saved to ~/.config/phaenna/config.toml (or set PHAENNA_GOOGLE_CLIENT_ID).",
            Style::default().fg(theme.text_dim),
        )),
    ];
    frame.render_widget(
        Paragraph::new(lines).block(blk).wrap(Wrap { trim: false }),
        area,
    );
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
            " Vault is isolated inside ~/.config/phaenna — zero browser cookie access.".to_string(),
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
    let hints = match &state.login_ui.step {
        LoginStep::Hub if state.auth_vault.is_authenticated() => {
            "[↑/↓] Select · [Enter] Switch · [r] Refresh · [+] Add Account · [x] Sign Out · [Esc] Close"
        }
        LoginStep::Hub => "[1 / Enter] Kitty Split Google Sign-In · [2] In-TUI Form · [3] Phone QR · [Esc] Close",
        LoginStep::EmailInput => "Type email · [Enter] Continue · [Ctrl+U] Clear · [Esc] Back",
        LoginStep::PasswordInput => "Type password · [Enter] Submit · [Tab] Show/Hide · [Ctrl+U] Clear · [Esc] Back",
        LoginStep::TwoFactorChallenge(_) => "[Enter] Verify · [b] Backup Code · [3] Phone QR · [Esc] Back",
        LoginStep::AwaitingDeviceQr => "[3] Regenerate QR · [Esc] Cancel & Back",
        LoginStep::AwaitingKittySso => "Sign in on right split — auto-connects when YouTube loads · [Esc] Cancel",
        LoginStep::ClientIdSetup => "[Tab] Next Field · [Enter] Save & Continue · [Ctrl+U] Clear · [Esc] Back",
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
    use crate::services::auth::{AuthKind, DeviceCodeSession, GoogleAccountSession};
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
    fn test_login_modal_renders_all_steps_cleanly() {
        let mut state = AppState::new(Config::default());
        state.auth_vault = crate::services::auth::AuthVault::default();
        state.overlay = ActiveOverlay::Login;

        // 1. Anonymous Hub
        let hub_txt = render_to_string(&state);
        assert!(hub_txt.contains("GOOGLE IDENTITY HUB"));
        assert!(hub_txt.contains("Instant Kitty Split Sign-In"));
        assert!(hub_txt.contains("In-TUI Email & 2FA"));

        // 2. Email Input
        state.login_ui.step = LoginStep::EmailInput;
        state.login_ui.email = "pilot@gmail.com".into();
        let email_txt = render_to_string(&state);
        assert!(email_txt.contains("pilot@gmail.com"));

        // 3. Password Input masks plaintext
        state.login_ui.step = LoginStep::PasswordInput;
        state.login_ui.password.push('s');
        state.login_ui.password.push('e');
        state.login_ui.password.push('c');
        let pass_txt = render_to_string(&state);
        assert!(pass_txt.contains("•••"));
        assert!(!pass_txt.contains("sec"));

        // 4. 2FA Step
        state.login_ui.step = LoginStep::TwoFactorChallenge(ChallengeKind::TotpCode);
        state.login_ui.two_factor_code = "842".into();
        let tfa_txt = render_to_string(&state);
        assert!(tfa_txt.contains("2-STEP VERIFICATION"));
        assert!(tfa_txt.contains("8 4 2"));

        // 5. Device QR Step
        state.login_ui.step = LoginStep::AwaitingDeviceQr;
        state.login_ui.device_session = Some(DeviceCodeSession {
            device_code: "dev".into(),
            user_code: "WXYZ-1234".into(),
            verification_url: "https://www.google.com/device".into(),
            verification_url_complete: "https://www.google.com/device?user_code=WXYZ-1234".into(),
            expires_at: now_unix_secs() + 900,
            interval_secs: 5,
            qr_lines: vec!["████".into(), "█  █".into()],
        });
        let qr_txt = render_to_string(&state);
        assert!(qr_txt.contains("WXYZ-1234"));
        assert!(qr_txt.contains("SCAN WITH PHONE CAMERA"));

        // 6. Client Setup Step
        state.login_ui.step = LoginStep::ClientIdSetup;
        state.login_ui.client_id_input = "my-client-id.apps.googleusercontent.com".into();
        let setup_txt = render_to_string(&state);
        assert!(setup_txt.contains("my-client-id.apps.googleusercontent.com"));

        // 7. Signed-In Hub Bento
        state.login_ui.step = LoginStep::Hub;
        state.auth_vault.upsert_session(GoogleAccountSession {
            account_id: "sub_99".into(),
            email: "captain@gmail.com".into(),
            display_name: "Phaenna Captain".into(),
            channel_id: Some("UC999".into()),
            channel_handle: Some("@captain".into()),
            avatar_url: None,
            auth_kind: AuthKind::OAuthDevice,
            access_token: Some("tok".into()),
            refresh_token: Some("ref".into()),
            expires_at: Some(now_unix_secs() + 3600),
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
