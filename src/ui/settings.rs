//! Split-Deck Fuzzy Feature Command Hub (`s` / `,`) powered by `SkimMatcherV2`.

use crate::state::{centered_rect, AppState};
use fuzzy_matcher::skim::SkimMatcherV2;
use fuzzy_matcher::FuzzyMatcher;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HubActionId {
    QualityPreset,
    PlayerBackend,
    SponsorBlockMode,
    VideoShaders,
    PerformanceMode,
    InstallNeuralShaders,
    SubtitlesEnabled,
    SubtitlePreset,
    SubtitleCollisionGuard,
    AmbilightBorders,
    ThemePalette,
    RemoteQrModal,
    CastScan,
    GoogleAccountHub,
    UploadRadar,
    CheckUpdates,
    AutoUpdateMode,
}

#[derive(Debug, Clone)]
pub struct HubItem {
    pub id: HubActionId,
    pub category: &'static str,
    pub label: &'static str,
    pub keywords: &'static str,
    pub badge: String,
    pub description: &'static str,
}

pub fn get_hub_items(state: &AppState) -> Vec<HubItem> {
    let cfg = &state.config;
    vec![
        HubItem {
            id: HubActionId::QualityPreset,
            category: "▶ PLAYBACK & STREAMING",
            label: "Default Quality & Codec",
            keywords: "quality 4k 1440p 1080p 720p audio codec format",
            badge: format!("[ {} ▾ ]", cfg.quality.label()),
            description: "Select preferred stream cap (Best/4K60 down to Audio Only).",
        },
        HubItem {
            id: HubActionId::PlayerBackend,
            category: "▶ PLAYBACK & STREAMING",
            label: "Video Player Engine",
            keywords: "player mpv iina vlc browser backend hwdec",
            badge: format!("[ {} ▾ ]", cfg.player.label()),
            description: "Choose non-blocking playback engine. mpv unlocks IPC resume & SponsorBlock.",
        },
        HubItem {
            id: HubActionId::SponsorBlockMode,
            category: "▶ PLAYBACK & STREAMING",
            label: "SponsorBlock Auto-Skip",
            keywords: "sponsorblock skip sponsor intro outro selfpromo chapters",
            badge: format!("[ {} ▾ ]", cfg.sponsorblock_mode.label()),
            description: "Automatically seek past sponsor reads, intros, outros, and interaction prompts.",
        },
        HubItem {
            id: HubActionId::VideoShaders,
            category: "VIDEO & SUBTITLE MASTERING",
            label: "GLSL 4K Mastering & Super-Resolution",
            keywords: "shader glsl sharpen rcas upscale 4k deblock chroma quality video",
            badge: format!("[ {} ▾ ]", cfg.shader_mode.label()),
            description: "Cycle Reference 4K Studio (bilateral deblock + 4:4:4 chroma + 2x RCAS), Neural Cinema, or Anime4K.",
        },
        HubItem {
            id: HubActionId::PerformanceMode,
            category: "VIDEO & SUBTITLE MASTERING",
            label: "Playback Performance Mode (RAM / GPU)",
            keywords: "performance efficient balanced quality eco ram memory cache battery",
            badge: format!("[ {} · {} ▾ ]", cfg.performance_mode.label(), cfg.performance_mode.short_badge()),
            description: "Switch Efficient (~350MB RAM), Balanced (~650MB RAM), or Quality (~1.4GB RAM 4K Master).",
        },
        HubItem {
            id: HubActionId::InstallNeuralShaders,
            category: "VIDEO & SUBTITLE MASTERING",
            label: "Download FSRCNNX 4K Neural CNN Pack",
            keywords: "fsrcnnx neural cnn download shader 4k upscale install",
            badge: if crate::services::shaders::neural_shader_installed() {
                "[ INSTALLED ]".to_string()
            } else {
                "[ Download ]".to_string()
            },
            description: "Download FSRCNNX_x2_16-0-4-1.glsl to unlock Neural Cinema 4K CNN upscaling.",
        },
        HubItem {
            id: HubActionId::SubtitlesEnabled,
            category: "VIDEO & SUBTITLE MASTERING",
            label: "Enable Subtitles / Captions",
            keywords: "subtitles captions cc enable toggle",
            badge: if cfg.subtitles_enabled { "[ ON ]".to_string() } else { "[ OFF ]".to_string() },
            description: "Automatically load preferred captions when launching playback.",
        },
        HubItem {
            id: HubActionId::SubtitlePreset,
            category: "VIDEO & SUBTITLE MASTERING",
            label: "Subtitle Typography Preset",
            keywords: "subtitle font style preset studio netflix gold oled",
            badge: format!("[ {} ▾ ]", cfg.subtitle_style.preset.label()),
            description: "Switch curated libass font/border/shadow/backdrop styling presets.",
        },
        HubItem {
            id: HubActionId::SubtitleCollisionGuard,
            category: "VIDEO & SUBTITLE MASTERING",
            label: "Subtitle Collision Guard",
            keywords: "subtitle collision signage an8 overlap",
            badge: if cfg.subtitle_style.collision_avoidance { "[ ON ]".to_string() } else { "[ OFF ]".to_string() },
            description: "Prevent dialogue lines from colliding with on-screen ASS signage.",
        },
        HubItem {
            id: HubActionId::AmbilightBorders,
            category: "VISUALS & THEMING",
            label: "Dynamic Thumbnail Ambilight",
            keywords: "ambilight glow border color extract theme",
            badge: if cfg.ambilight_enabled { "[ ON ]".to_string() } else { "[ OFF ]".to_string() },
            description: "Sample HSV dominant hues from focused thumbnails to tint active frames.",
        },
        HubItem {
            id: HubActionId::ThemePalette,
            category: "VISUALS & THEMING",
            label: "Active Theme Palette",
            keywords: "theme palette tokyo cyberpunk phaenna zen violet dracula nord",
            badge: format!("[ {} ▾ ]", state.base_theme().name),
            description: "Cycle between all 6 high-contrast curated terminal color palettes.",
        },
        HubItem {
            id: HubActionId::RemoteQrModal,
            category: "COMPANION & CAST",
            label: "Mobile LAN Remote QR",
            keywords: "remote qr phone websocket companion port",
            badge: state.remote_handle.as_ref().map(|h| format!("[ {} ]", h.url)).unwrap_or_else(|| "[ Open ]".to_string()),
            description: "Pair your phone over local Wi-Fi via WebSocket + QR code (<1ms latency).",
        },
        HubItem {
            id: HubActionId::CastScan,
            category: "COMPANION & CAST",
            label: "AirPlay / DLNA TV Cast",
            keywords: "cast tv airplay dlna upnp ssdp living room",
            badge: format!("[ {} found ]", state.cast_devices.len()),
            description: "Scan local network for AirPlay 2 & DLNA Smart TVs and hand off streams.",
        },
        HubItem {
            id: HubActionId::GoogleAccountHub,
            category: "ACCOUNT & TOOLS",
            label: "Google Account & Identity Hub",
            keywords: "google login account oauth qr 2fa auth passkey",
            badge: if let Some(acct) = state.auth_vault.active_account() {
                format!("[ OK {} ]", acct.email)
            } else {
                "[ Guest ]".to_string()
            },
            description: "Sign in inside Phaenna (Email+2FA, Instant QR Device Code, or Kitty Split SSO).",
        },
        HubItem {
            id: HubActionId::UploadRadar,
            category: "ACCOUNT & TOOLS",
            label: "7-Day Upload Radar Calendar",
            keywords: "calendar uploads schedule radar week",
            badge: "[ Open (c) ]".to_string(),
            description: "Open the local-timezone Mon-Sun upload schedule for subscriptions.",
        },
        HubItem {
            id: HubActionId::CheckUpdates,
            category: "ACCOUNT & TOOLS",
            label: "Check GitHub Releases Update",
            keywords: "update version github release semver rolling",
            badge: if state.is_checking_update {
                "[ Checking... ]".to_string()
            } else if let Some(ref u) = state.available_update {
                format!("[ Update: {} ]", u.latest_version)
            } else {
                format!(
                    "[ v{} · {} ]",
                    env!("CARGO_PKG_VERSION"),
                    crate::services::updater::short_sha(env!("PHAENNA_GIT_SHA"))
                )
            },
            description: "Check upstream GitHub Releases and self-update cleanly.",
        },
        HubItem {
            id: HubActionId::AutoUpdateMode,
            category: "ACCOUNT & TOOLS",
            label: "Auto-Update Mode",
            keywords: "auto update startup channel install check background",
            badge: if !cfg.update.check_on_startup {
                "[ Disabled ▾ ]".to_string()
            } else if cfg.update.auto_install {
                "[ Auto-Install ▾ ]".to_string()
            } else {
                "[ On Startup ▾ ]".to_string()
            },
            description: "Cycle between On Startup prompt, silent Auto-Install, or Disabled.",
        },
    ]
}

pub fn filtered_hub_items(state: &AppState) -> Vec<HubItem> {
    let all = get_hub_items(state);
    let q = state.settings_query.trim();
    if q.is_empty() {
        return all;
    }

    let matcher = SkimMatcherV2::default();
    let mut scored: Vec<(i64, HubItem)> = all
        .into_iter()
        .filter_map(|item| {
            let hay = format!("{} {} {}", item.category, item.label, item.keywords);
            matcher.fuzzy_match(&hay, q).map(|s| (s, item))
        })
        .collect();
    scored.sort_by_key(|a| std::cmp::Reverse(a.0));
    scored.into_iter().map(|(_, item)| item).collect()
}

pub fn render_settings_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(82, 74, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " PHAENNA — FUZZY COMMAND HUB & SETTINGS ",
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
            Constraint::Length(1),
        ])
        .split(inner);

    let filter_blk = Block::default()
        .title(Span::styled(" Filter Actions (Type to fuzzy-search) ", Style::default().fg(theme.secondary)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_focus));

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(" > ", Style::default().fg(theme.primary)),
            Span::styled(&state.settings_query, Style::default().fg(theme.text).add_modifier(Modifier::BOLD)),
            Span::styled("█", Style::default().fg(theme.accent)),
        ]))
        .block(filter_blk),
        rows[0],
    );

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(62), Constraint::Percentage(38)])
        .split(rows[1]);

    let items = filtered_hub_items(state);
    let cursor = state.settings_cursor.min(items.len().saturating_sub(1));

    let list_blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let list_inner = list_blk.inner(cols[0]);
    frame.render_widget(list_blk, cols[0]);

    let max_vis = list_inner.height as usize;
    let start = if cursor >= max_vis && max_vis > 0 {
        cursor + 1 - max_vis
    } else {
        0
    };

    let mut lines = Vec::new();
    for (idx, item) in items.iter().enumerate().skip(start).take(max_vis) {
        let is_sel = idx == cursor;
        let row_style = if is_sel {
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text)
        };
        let badge_style = if is_sel {
            row_style
        } else {
            Style::default().fg(theme.accent)
        };
        let prefix = if is_sel { " ▸ " } else { "   " };
        lines.push(Line::from(vec![
            Span::styled(prefix, row_style),
            Span::styled(format!("{:<29} ", item.label.chars().take(29).collect::<String>()), row_style),
            Span::styled(item.badge.clone(), badge_style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), list_inner);

    let detail_blk = Block::default()
        .title(Span::styled(" Inspector ", Style::default().fg(theme.secondary)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let inspector_lines = if let Some(sel) = items.get(cursor) {
        vec![
            Line::from(Span::styled(
                sel.category,
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                sel.label,
                Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(sel.badge.clone(), Style::default().fg(theme.secondary))),
            Line::from(""),
            Line::from(Span::styled(sel.description, Style::default().fg(theme.text))),
        ]
    } else {
        vec![Line::from(Span::styled("No matching action.", Style::default().fg(theme.text_dim)))]
    };
    frame.render_widget(
        Paragraph::new(inspector_lines)
            .wrap(Wrap { trim: true })
            .block(detail_blk),
        cols[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[↑/↓] Navigate ", Style::default().fg(theme.primary)),
            Span::styled("· [Enter] Apply / Cycle / Open ", Style::default().fg(theme.accent)),
            Span::styled("· [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_fuzzy_command_hub_filter_and_render() {
        let mut st = AppState::new(Config::default());
        st.settings_query = "sponsor".to_string();
        let filtered = filtered_hub_items(&st);
        assert!(!filtered.is_empty());
        assert_eq!(filtered[0].id, HubActionId::SponsorBlockMode);

        let backend = TestBackend::new(110, 32);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_settings_modal(f, &st)).unwrap();
    }
}
