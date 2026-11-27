//! 4-Phase Self-Updater & Plain-English "What's New & Fixed" Modal (`U`),
//! matching Solus's transparent, jargon-free changelog UX:
//! 1. **Available**: Shows version diff + plain-English "What's New & Fixed" highlights.
//! 2. **Downloading**: Live progress gauge + speed alongside release highlights.
//! 3. **RestartRequired**: Confirms upgrade & prompts `[Enter]` clean exit / `[Esc]` continue.
//! 4. **PostUpdateWelcome**: Greets the user after updating (and accessible anytime from Settings).

use crate::services::updater::{default_current_release_notes, short_sha, simplify_release_notes};
use crate::state::{active_modal_rect, ActiveOverlay, AppState, UpdateModalPhase};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Gauge, Paragraph, Wrap},
    Frame,
};

const EMERALD: Color = Color::Rgb(52, 211, 153);
const CYAN: Color = Color::Rgb(56, 189, 248);
const AMBER: Color = Color::Rgb(251, 191, 36);
const CORAL: Color = Color::Rgb(248, 113, 113);

fn notes_for_info(info: &crate::services::updater::UpdateInfo) -> Vec<String> {
    let mut notes = if !info.simple_notes.is_empty() {
        info.simple_notes.clone()
    } else if let Some(ref raw) = info.release_notes {
        simplify_release_notes(raw)
    } else {
        default_current_release_notes()
    };

    if notes.len() < 6 {
        for fallback in default_current_release_notes() {
            if notes.len() >= 6 {
                break;
            }
            let prefix = fallback
                .split_once(':')
                .map(|(head, _)| head.to_ascii_lowercase())
                .unwrap_or_else(|| fallback.to_ascii_lowercase());
            let already_present = notes.iter().any(|existing| {
                let ex_prefix = existing
                    .split_once(':')
                    .map(|(head, _)| head.to_ascii_lowercase())
                    .unwrap_or_else(|| existing.to_ascii_lowercase());
                ex_prefix == prefix || existing.eq_ignore_ascii_case(&fallback)
            });
            if !already_present {
                notes.push(fallback);
            }
        }
    }
    notes
}

fn append_body_with_how_to_use(spans: &mut Vec<Span<'static>>, body: &str) {
    if let Some((desc, guide)) = body.split_once("How to use:") {
        if !desc.is_empty() {
            spans.push(Span::styled(
                desc.to_string(),
                Style::default().fg(Color::White),
            ));
        }
        spans.push(Span::styled(
            "How to use:",
            Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            guide.to_string(),
            Style::default().fg(Color::Rgb(215, 225, 240)),
        ));
    } else {
        spans.push(Span::styled(
            body.to_string(),
            Style::default().fg(Color::White),
        ));
    }
}

fn format_plain_english_bullet(note: &str) -> Line<'static> {
    let trimmed = note.trim();
    for (badge, color) in [("[NEW]", EMERALD), ("[FIXED]", CYAN), ("[IMPROVED]", AMBER)] {
        if let Some(rest) = trimmed.strip_prefix(badge) {
            let mut spans = vec![
                Span::styled("  • ", Style::default().fg(color)),
                Span::styled(
                    format!("{badge} "),
                    Style::default().fg(color).add_modifier(Modifier::BOLD),
                ),
            ];
            append_body_with_how_to_use(&mut spans, rest.trim());
            return Line::from(spans);
        }
    }

    let mut spans = vec![Span::styled("  • ", Style::default().fg(CYAN))];
    append_body_with_how_to_use(&mut spans, trimmed);
    Line::from(spans)
}

fn push_plain_english_section(lines: &mut Vec<Line<'static>>, notes: &[String], header: &str) {
    lines.push(Line::from(Span::styled(
        header.to_string(),
        Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(""));
    for (idx, note) in notes.iter().take(6).enumerate() {
        lines.push(format_plain_english_bullet(note));
        if idx + 1 < notes.len().min(6) {
            lines.push(Line::from(""));
        }
    }
}

pub fn render_update_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = active_modal_rect(frame.area(), ActiveOverlay::Update)
        .unwrap_or_else(|| crate::state::centered_rect(78, 74, frame.area()));
    frame.render_widget(Clear, area);

    let derived_phase = state.update_modal_phase.clone().or_else(|| {
        if let Some(ref ver) = state.update_completed_version {
            let notes = state
                .available_update
                .as_ref()
                .map(notes_for_info)
                .unwrap_or_else(default_current_release_notes);
            Some(UpdateModalPhase::RestartRequired {
                new_version: ver.clone(),
                simple_notes: notes,
            })
        } else if state.is_updating || state.update_progress > 0.0 {
            Some(UpdateModalPhase::Downloading {
                info: state.available_update.clone(),
                progress_pct: state.update_progress,
                speed: state
                    .update_status
                    .clone()
                    .unwrap_or_else(|| "Updating...".to_string()),
            })
        } else {
            state
                .available_update
                .clone()
                .map(|info| UpdateModalPhase::Available { info })
        }
    });

    let (title, border_color) = match derived_phase {
        Some(UpdateModalPhase::Available { .. }) => {
            (" ◈ PHAENNA UPDATE AVAILABLE ◈ ".to_string(), theme.primary)
        }
        Some(UpdateModalPhase::Downloading { .. }) => {
            (" ◈ DOWNLOADING PHAENNA UPDATE ◈ ".to_string(), CYAN)
        }
        Some(UpdateModalPhase::RestartRequired { .. }) => {
            (" ◈ UPDATE SUCCESSFULLY APPLIED ◈ ".to_string(), EMERALD)
        }
        Some(UpdateModalPhase::PostUpdateWelcome {
            ref version_label, ..
        }) => (
            format!(" ◈ Phaenna Just Got Better! ({version_label}) ◈ "),
            EMERALD,
        ),
        Some(UpdateModalPhase::Failed { .. }) => {
            (" [!] PHAENNA UPDATE NOTICE ".to_string(), CORAL)
        }
        None => (" ◈ PHAENNA — UPDATES & WHAT'S NEW ◈ ".to_string(), theme.primary),
    };

    let outer = Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(border_color)
                .add_modifier(Modifier::BOLD),
        ))
        .title_alignment(Alignment::Center)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color))
        .style(Style::default().bg(Color::Reset));

    let inner = outer.inner(area).inner(Margin {
        horizontal: 1,
        vertical: 0,
    });
    frame.render_widget(outer, area);

    match derived_phase {
        Some(UpdateModalPhase::Available { ref info }) => {
            render_available(frame, inner, info, border_color);
        }
        Some(UpdateModalPhase::Downloading {
            ref info,
            progress_pct,
            ref speed,
        }) => {
            render_downloading(frame, inner, info.as_ref(), progress_pct, speed, border_color);
        }
        Some(UpdateModalPhase::RestartRequired {
            ref new_version,
            ref simple_notes,
        }) => {
            render_restart_required(frame, inner, new_version, simple_notes, border_color);
        }
        Some(UpdateModalPhase::PostUpdateWelcome {
            ref version_label,
            ref simple_notes,
        }) => {
            render_post_update_welcome(frame, inner, version_label, simple_notes);
        }
        Some(UpdateModalPhase::Failed { ref error }) => {
            render_failed(frame, inner, error, border_color);
        }
        None => {
            render_idle_status(frame, inner, state);
        }
    }
}

fn render_available(
    frame: &mut Frame,
    area: Rect,
    info: &crate::services::updater::UpdateInfo,
    accent: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(area);

    let version_lines = vec![Line::from(vec![
        Span::styled(
            "Current Version: ",
            Style::default().fg(Color::Rgb(160, 160, 180)),
        ),
        Span::styled(
            &info.current_version,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "   ──►   ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            "New Version: ",
            Style::default().fg(Color::Rgb(160, 160, 180)),
        ),
        Span::styled(
            &info.latest_version,
            Style::default().fg(EMERALD).add_modifier(Modifier::BOLD),
        ),
    ])];
    frame.render_widget(
        Paragraph::new(version_lines).alignment(Alignment::Center),
        chunks[0],
    );

    let notes = notes_for_info(info);
    let mut notes_lines = Vec::new();
    push_plain_english_section(&mut notes_lines, &notes, "What's New & Fixed:");
    frame.render_widget(
        Paragraph::new(notes_lines).wrap(Wrap { trim: false }),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Your watch history, queues, playlists, and settings stay safely preserved.",
            Style::default().fg(Color::Rgb(150, 155, 175)),
        )))
        .alignment(Alignment::Center),
        chunks[2],
    );

    let action_line = if info.is_homebrew {
        Line::from(vec![
            Span::styled(
                "[Esc] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Close  (Run 'brew upgrade phaenna' in your terminal)",
                Style::default().fg(Color::Rgb(200, 200, 220)),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(
                " [Enter / u] Update Now ",
                Style::default()
                    .fg(Color::Black)
                    .bg(EMERALD)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("   "),
            Span::styled(
                "[b] ",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Background   ", Style::default().fg(Color::White)),
            Span::styled(
                "[Esc] ",
                Style::default().fg(AMBER).add_modifier(Modifier::BOLD),
            ),
            Span::styled("Remind Me Later", Style::default().fg(Color::Rgb(180, 180, 195))),
        ])
    };
    frame.render_widget(
        Paragraph::new(vec![action_line]).alignment(Alignment::Center),
        chunks[3],
    );
}

fn render_downloading(
    frame: &mut Frame,
    area: Rect,
    info: Option<&crate::services::updater::UpdateInfo>,
    pct: f32,
    speed: &str,
    accent: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(2),
        ])
        .split(area);

    let target_label = info
        .map(|i| format!("Updating Phaenna: {}  ──►  {}", i.current_version, i.latest_version))
        .unwrap_or_else(|| "Downloading & installing latest Phaenna update...".to_string());
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            target_label,
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        chunks[0],
    );

    let clamped_pct = pct.clamp(0.0, 100.0);
    let gauge = Gauge::default()
        .block(
            Block::default()
                .title(" Download Progress ")
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(accent)),
        )
        .gauge_style(Style::default().fg(accent).bg(Color::Rgb(30, 30, 45)))
        .percent(clamped_pct as u16)
        .label(Span::styled(
            format!("{clamped_pct:.1}%"),
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        ));
    frame.render_widget(gauge, chunks[1]);

    frame.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::styled("Status: ", Style::default().fg(Color::Rgb(140, 140, 160))),
            Span::styled(speed, Style::default().fg(EMERALD).add_modifier(Modifier::BOLD)),
            Span::styled("   ·   ", Style::default().fg(Color::Rgb(80, 80, 100))),
            Span::styled(
                "Your history and playlists stay safe during update",
                Style::default().fg(Color::Rgb(180, 180, 200)),
            ),
        ])])
        .alignment(Alignment::Center),
        chunks[2],
    );

    let notes = info
        .map(notes_for_info)
        .unwrap_or_else(default_current_release_notes);
    let mut notes_lines = Vec::new();
    push_plain_english_section(
        &mut notes_lines,
        &notes,
        "What's New & Fixed in This Update:",
    );
    frame.render_widget(
        Paragraph::new(notes_lines).wrap(Wrap { trim: false }),
        chunks[3],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                "[b / Esc] ",
                Style::default().fg(CYAN).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Finish Quietly in Background",
                Style::default().fg(Color::Rgb(170, 175, 195)),
            ),
        ]))
        .alignment(Alignment::Center),
        chunks[4],
    );
}

fn render_restart_required(
    frame: &mut Frame,
    area: Rect,
    new_version: &str,
    simple_notes: &[String],
    accent: Color,
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(5),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(vec![Line::from(Span::styled(
            format!("◈ Phaenna has been updated to {new_version} and is ready!"),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))])
        .alignment(Alignment::Center),
        chunks[0],
    );

    let notes = if simple_notes.is_empty() {
        default_current_release_notes()
    } else {
        simple_notes.to_vec()
    };
    let mut notes_lines = Vec::new();
    push_plain_english_section(
        &mut notes_lines,
        &notes,
        "What's Included in This Update:",
    );
    frame.render_widget(
        Paragraph::new(notes_lines).wrap(Wrap { trim: false }),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Your watch history, queues, playlists, and settings are safely preserved.",
            Style::default().fg(Color::Rgb(160, 160, 180)),
        )))
        .alignment(Alignment::Center),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::styled(
                " [Enter] Exit Phaenna to Relaunch ",
                Style::default()
                    .fg(Color::Black)
                    .bg(EMERALD)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw("    "),
            Span::styled(
                "[Esc] ",
                Style::default()
                    .fg(Color::Rgb(160, 160, 180))
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                "Continue Current Session",
                Style::default().fg(Color::Rgb(160, 160, 180)),
            ),
        ])])
        .alignment(Alignment::Center),
        chunks[3],
    );
}

fn render_post_update_welcome(
    frame: &mut Frame,
    area: Rect,
    _version_label: &str,
    simple_notes: &[String],
) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(2),
            Constraint::Length(2),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "You are running the latest version of Phaenna! Here is what changed:",
            Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center),
        chunks[0],
    );

    let notes = if simple_notes.is_empty() {
        default_current_release_notes()
    } else {
        simple_notes.to_vec()
    };
    let mut notes_lines = Vec::new();
    push_plain_english_section(&mut notes_lines, &notes, "What's New & Fixed:");
    frame.render_widget(
        Paragraph::new(notes_lines).wrap(Wrap { trim: false }),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "Tip: Re-open these release notes anytime from Settings [s] -> What's New & Release Notes.",
            Style::default().fg(Color::Rgb(150, 155, 175)),
        )))
        .alignment(Alignment::Center),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![Span::styled(
            " [Enter / Space / Esc] Start Watching ",
            Style::default()
                .fg(Color::Black)
                .bg(EMERALD)
                .add_modifier(Modifier::BOLD),
        )]))
        .alignment(Alignment::Center),
        chunks[3],
    );
}

fn render_failed(frame: &mut Frame, area: Rect, error: &str, accent: Color) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(4),
            Constraint::Length(2),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(vec![Line::from(Span::styled(
            "Automatic update could not be completed right now.",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))])
        .alignment(Alignment::Center),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(vec![
            Line::from(""),
            Line::from(Span::styled(
                format!("Details: {error}"),
                Style::default().fg(Color::Rgb(255, 160, 160)),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Your current Phaenna installation was not modified.",
                Style::default().fg(Color::Rgb(170, 175, 195)),
            )),
        ])
        .alignment(Alignment::Center)
        .wrap(Wrap { trim: false }),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(vec![Line::from(vec![
            Span::styled(
                "[Enter / Esc] ",
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("Dismiss", Style::default().fg(Color::Rgb(180, 180, 180))),
        ])])
        .alignment(Alignment::Center),
        chunks[2],
    );
}

fn render_idle_status(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Min(6),
            Constraint::Length(2),
        ])
        .split(area);

    let status_text = if state.is_checking_update {
        format!(
            "Checking GitHub Releases... Running v{} ({})",
            env!("CARGO_PKG_VERSION"),
            short_sha(env!("PHAENNA_GIT_SHA"))
        )
    } else {
        format!(
            "You are running Phaenna v{} ({}). Here is what changed recently:",
            env!("CARGO_PKG_VERSION"),
            short_sha(env!("PHAENNA_GIT_SHA"))
        )
    };
    frame.render_widget(
        Paragraph::new(status_text)
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)),
        chunks[0],
    );

    let mut notes_lines = Vec::new();
    push_plain_english_section(
        &mut notes_lines,
        &default_current_release_notes(),
        "What's New & Fixed:",
    );
    frame.render_widget(
        Paragraph::new(notes_lines).wrap(Wrap { trim: false }),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[r] Re-Check Updates ", Style::default().fg(EMERALD)),
            Span::styled("· [Esc] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::services::updater::UpdateInfo;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_update_modal_shows_plain_english_notes_across_all_phases() {
        let mut st = AppState::new(Config::default());
        let info = UpdateInfo {
            current_version: "v1.0.0 (1111111)".to_string(),
            latest_version: "main@abcdef1".to_string(),
            download_url: "https://example.com/phaenna".to_string(),
            asset_name: "phaenna-linux-x86_64".to_string(),
            release_notes: Some("Automated build for commit `abcdef1`: feat: add eq presets".to_string()),
            simple_notes: vec![
                "[NEW] Added 18 audiophile equalizer profiles. How to use: Press 'e' in Music.".to_string(),
                "[FIXED] Videos now open cleanly in front of fullscreen terminals.".to_string(),
            ],
            is_homebrew: false,
        };

        st.open_update_modal(info.clone());
        let mut term = Terminal::new(TestBackend::new(130, 42)).unwrap();
        term.draw(|f| render_update_modal(f, &st)).unwrap();
        let avail_text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(avail_text.contains("Update Now"));
        assert!(avail_text.contains("What's New & Fixed:"));
        assert!(avail_text.contains("How to use:"));

        st.open_post_update_welcome("main@abcdef1".to_string(), info.simple_notes.clone());
        term.draw(|f| render_update_modal(f, &st)).unwrap();
        let welcome_text: String = term.backend().buffer().content().iter().map(|c| c.symbol()).collect();
        assert!(welcome_text.contains("Phaenna Just Got Better! (main@abcdef1)"));
        assert!(welcome_text.contains("Start Watching"));
        assert!(welcome_text.contains("What's New & Release Notes"));
    }
}
