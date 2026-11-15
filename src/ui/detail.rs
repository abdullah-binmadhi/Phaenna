//! Ambilight Video Detail Screen (`Screen::Detail`).
//! Features dynamic Ambilight border extraction, ReturnYouTubeDislike bar,
//! proportional SponsorBlock skip-segment timeline bar, interactive Chapter browser (`Tab`),
//! Watch Later / Offline status badges, and direct Channel drill-down (`c`).

use crate::api::dislike::format_compact_count;
use crate::api::sponsorblock::category_label;
use crate::db::cache::format_duration_secs;
use crate::state::{occluded_by_modal, ActiveOverlay, AppState, DetailViewMode};
use crate::ui::components::cover::HalfblockCover;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Wrap},
    Frame,
};
use ratatui_image::StatefulImage;

fn sponsor_color(category: &str) -> Color {
    match category {
        "sponsor" => Color::Rgb(0, 212, 0),
        "selfpromo" => Color::Rgb(255, 255, 0),
        "interaction" => Color::Rgb(204, 0, 255),
        "intro" => Color::Rgb(0, 255, 255),
        "outro" => Color::Rgb(2, 2, 237),
        _ => Color::Rgb(255, 118, 142),
    }
}

fn render_sponsor_timeline_line(state: &AppState, total_secs: f64, width: usize) -> Line<'static> {
    if total_secs <= 1.0 || width < 10 {
        return Line::from("");
    }
    let w = width.min(72);
    let mut spans = Vec::with_capacity(w);
    for col in 0..w {
        let frac = (col as f64 + 0.5) / w as f64;
        let t = frac * total_secs;
        let mut hit = None;
        for seg in &state.detail_sponsor_segments {
            if t >= seg.start_time && t <= seg.end_time {
                hit = Some(seg.category.as_str());
                break;
            }
        }
        if let Some(cat) = hit {
            spans.push(Span::styled("━", Style::default().fg(sponsor_color(cat))));
        } else {
            spans.push(Span::styled("─", Style::default().fg(Color::Rgb(70, 78, 98))));
        }
    }
    Line::from(spans)
}

pub fn render_detail_screen(
    frame: &mut Frame,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let Some(video) = state.detail_video.clone() else {
        return;
    };
    let pal = state.active_palette_for(Some(&video.id));
    let area = frame.area();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(14),
            Constraint::Length(2),
        ])
        .split(area);

    // Header
    let hdr_blk = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.primary));
    let mode_badge = match state.detail_view_mode {
        DetailViewMode::Overview => "OVERVIEW [Tab -> Chapters]",
        DetailViewMode::Chapters => "CHAPTERS [Tab -> Overview]",
    };
    let hdr = Line::from(vec![
        Span::styled(
            format!(" ▶ {} ", video.title),
            Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  {} ", mode_badge),
            Style::default().fg(pal.secondary),
        ),
    ]);
    frame.render_widget(Paragraph::new(hdr).block(hdr_blk), rows[0]);

    // Split left artwork + telemetry | right overview / chapters
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(rows[1]);

    let left_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(14), Constraint::Min(6)])
        .split(cols[0]);

    // Left top: 16:9 HD Thumbnail Frame
    let art_blk = Block::default()
        .title(Span::styled(
            if state.config.ambilight_enabled { " ✦ Ambilight 16:9 " } else { " ▣ Preview " },
            Style::default().fg(pal.secondary).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_focus));
    let art_inner = art_blk.inner(left_rows[0]);
    frame.render_widget(art_blk, left_rows[0]);

    if !occluded_by_modal(art_inner, modal_rect) && state.overlay == ActiveOverlay::None {
        if let Some(proto) = state.banner_cache.get_mut(&video.id) {
            frame.render_stateful_widget(StatefulImage::new(None), art_inner, proto);
        } else if let Some(img) = state.raw_thumbs.get(&video.id) {
            frame.render_widget(HalfblockCover { image: img }, art_inner);
        }
    }

    // Left bottom: Stats, Sentiment (Likes vs Dislikes), Resume & Offline badges
    let stats_blk = Block::default()
        .title(Span::styled(" 📊 Telemetry ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));

    let views_str = video
        .view_count
        .map(|v| format_compact_count(v as i64))
        .unwrap_or_else(|| "—".to_string());
    let likes_str = state
        .detail_dislikes
        .as_ref()
        .map(|d| format!("👍 {} · 👎 {} ({:.1}%)", format_compact_count(d.likes), format_compact_count(d.dislikes), d.like_ratio_pct()))
        .or_else(|| video.like_count.map(|l| format!("👍 {}", format_compact_count(l as i64))))
        .unwrap_or_else(|| "Fetching likes/dislikes...".to_string());
    let resume_str = state
        .progress_map
        .get(&video.id)
        .map(|(p, d)| format!("▶ Resume at {} / {}", format_duration_secs(*p as u64), format_duration_secs(*d as u64)))
        .unwrap_or_else(|| "Start from 00:00".to_string());

    let mut stat_lines = vec![
        Line::from(vec![
            Span::styled("Channel  : ", Style::default().fg(pal.text_dim)),
            Span::styled(video.display_uploader(), Style::default().fg(pal.secondary).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled("Runtime  : ", Style::default().fg(pal.text_dim)),
            Span::styled(format!("{} · {} views", video.display_duration(), views_str), Style::default().fg(pal.text)),
        ]),
        Line::from(vec![
            Span::styled("Sentiment: ", Style::default().fg(pal.text_dim)),
            Span::styled(likes_str, Style::default().fg(pal.accent)),
        ]),
        Line::from(vec![
            Span::styled("Progress : ", Style::default().fg(pal.text_dim)),
            Span::styled(resume_str, Style::default().fg(pal.primary)),
        ]),
    ];
    if state.offline_paths.contains_key(&video.id) {
        stat_lines.push(Line::from(Span::styled(
            "✓ Cached in Offline Vault (Instant Local Playback)",
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        )));
    }
    if state.watch_later_sources.contains_key(&video.id) {
        stat_lines.push(Line::from(Span::styled(
            "★ Saved in Watch Later Queue",
            Style::default().fg(Color::Rgb(251, 191, 36)),
        )));
    }
    frame.render_widget(Paragraph::new(stat_lines).block(stats_blk).wrap(Wrap { trim: true }), left_rows[1]);

    // Right Pane: Overview OR Interactive Chapter Browser + SponsorBlock Bar
    let right_rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(8)])
        .split(cols[1]);

    let sb_blk = Block::default()
        .title(Span::styled(
            format!(" 🛡 SponsorBlock ({} segments · Mode: {}) ", state.detail_sponsor_segments.len(), state.config.sponsorblock_mode.label()),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let sb_inner = sb_blk.inner(right_rows[0]);
    frame.render_widget(sb_blk, right_rows[0]);

    let total_dur = video.duration.unwrap_or(600) as f64;
    let bar = render_sponsor_timeline_line(state, total_dur, sb_inner.width as usize);
    let summary_text = if state.detail_sponsor_segments.is_empty() {
        "No community SponsorBlock segments reported for this video.".to_string()
    } else {
        state
            .detail_sponsor_segments
            .iter()
            .take(3)
            .map(|s| format!("{} ({}-{})", category_label(&s.category), format_duration_secs(s.start_time as u64), format_duration_secs(s.end_time as u64)))
            .collect::<Vec<_>>()
            .join(" · ")
    };
    frame.render_widget(
        Paragraph::new(vec![bar, Line::from(Span::styled(summary_text, Style::default().fg(pal.text_dim)))]),
        sb_inner,
    );

    match state.detail_view_mode {
        DetailViewMode::Overview => {
            let desc_blk = Block::default()
                .title(Span::styled(" 📄 Description & Metadata ", Style::default().fg(pal.primary).add_modifier(Modifier::BOLD)))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(pal.border_idle));
            let desc = video
                .description
                .as_deref()
                .filter(|s| !s.is_empty())
                .unwrap_or("Loading full metadata, chapters, subtitles, and SponsorBlock cues...");
            frame.render_widget(
                Paragraph::new(desc)
                    .wrap(Wrap { trim: false })
                    .scroll((state.detail_scroll, 0))
                    .block(desc_blk)
                    .style(Style::default().fg(pal.text)),
                right_rows[1],
            );
        }
        DetailViewMode::Chapters => {
            let chapters = video.chapters.as_deref().unwrap_or(&[]);
            let ch_blk = Block::default()
                .title(Span::styled(
                    format!(" 📑 Video Chapters ({}) — Enter to Jump & Play ", chapters.len()),
                    Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
                ))
                .borders(Borders::ALL)
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(pal.border_focus));
            let ch_inner = ch_blk.inner(right_rows[1]);
            frame.render_widget(ch_blk, right_rows[1]);

            if chapters.is_empty() {
                frame.render_widget(
                    Paragraph::new("No native or description timestamp chapters found for this video.")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(pal.text_dim)),
                    ch_inner,
                );
            } else {
                let cur = state.detail_chapter_cursor.min(chapters.len().saturating_sub(1));
                let lines: Vec<Line> = chapters
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        let sel = i == cur;
                        let st = if sel {
                            Style::default().fg(Color::Black).bg(pal.primary).add_modifier(Modifier::BOLD)
                        } else {
                            Style::default().fg(pal.text)
                        };
                        Line::from(vec![
                            Span::styled(format!(" {:>6} ", format_duration_secs(c.start_time as u64)), Style::default().fg(pal.accent)),
                            Span::styled(c.title.clone(), st),
                        ])
                    })
                    .collect();
                frame.render_widget(Paragraph::new(lines), ch_inner);
            }
        }
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter] Play ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled("· [p] Options ", Style::default().fg(pal.primary)),
            Span::styled("· [Tab] Chapters ", Style::default().fg(pal.secondary)),
            Span::styled("· [c] Channel ", Style::default().fg(pal.primary)),
            Span::styled("· [w] WatchLater ", Style::default().fg(Color::Rgb(251, 191, 36))),
            Span::styled("· [D] Download ", Style::default().fg(pal.accent)),
            Span::styled("· [m] Remote · [Esc] Back", Style::default().fg(pal.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}
