//! Home Screen — Cinematic Glass Top Bar, 3-Zone Hero Stage,
//! Occlusion-Safe HD 16:9 Card Grid, Filter Modal, and 2-Row Command Dock.

use crate::api::dislike::format_compact_count;
use crate::db::cache::{format_duration_secs, VideoItem};
use crate::state::{
    centered_rect, occluded_by_modal, ActiveOverlay, AppState, Category, ContentFilter, SortOrder,
};
use crate::ui::components::cover::{color_from_id, HalfblockCover};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use ratatui_image::StatefulImage;

pub const TOP_BAR_HEIGHT: u16 = 1;
pub const HERO_HEIGHT: u16 = 10;
pub const FOOTER_HEIGHT: u16 = 3;
pub const CARD_WIDTH: u16 = 28;
pub const CARD_HEIGHT: u16 = 11;
pub const CARD_GAP: u16 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeFooterAction {
    Search,
    Calendar,
    Tailored,
    Analytics,
    Remote,
    Cast,
    Filter,
    Settings,
    Help,
    Setup,
    Quit,
}

/// Compute top bar shelf pill hitboxes `(start_x, end_x, category_index)`.
pub fn top_bar_shelf_rects(categories: &[(Category, Vec<VideoItem>)]) -> Vec<(u16, u16, usize)> {
    let mut out = Vec::with_capacity(categories.len());
    let mut x = 10u16; // After " PHAENNA " brand chip
    for (idx, (cat, items)) in categories.iter().enumerate() {
        let label = format!(" {} ({}) ", cat.label(), items.len());
        let w = label.chars().count() as u16;
        out.push((x, x + w, idx));
        x = x + w + 1;
    }
    out
}

pub fn top_bar_shelf_at(categories: &[(Category, Vec<VideoItem>)], col: u16) -> Option<usize> {
    for (sx, ex, idx) in top_bar_shelf_rects(categories) {
        if col >= sx && col < ex {
            return Some(idx);
        }
    }
    None
}

pub fn grid_columns(width: u16) -> usize {
    let usable = width.saturating_sub(2);
    ((usable + CARD_GAP) / (CARD_WIDTH + CARD_GAP)).max(1) as usize
}

pub fn grid_card_index_at(
    grid_area: Rect,
    col: u16,
    row: u16,
    scroll_row: usize,
    total_items: usize,
) -> Option<usize> {
    if col < grid_area.x || col >= grid_area.x + grid_area.width {
        return None;
    }
    if row < grid_area.y || row >= grid_area.y + grid_area.height {
        return None;
    }
    let cols = grid_columns(grid_area.width);
    let rel_x = col.saturating_sub(grid_area.x + 1);
    let rel_y = row.saturating_sub(grid_area.y);
    let c_idx = (rel_x / (CARD_WIDTH + CARD_GAP)) as usize;
    let in_card_x = rel_x % (CARD_WIDTH + CARD_GAP);
    if c_idx >= cols || in_card_x >= CARD_WIDTH {
        return None;
    }
    let r_idx = (rel_y / (CARD_HEIGHT + 1)) as usize + scroll_row;
    let idx = r_idx * cols + c_idx;
    if idx < total_items {
        Some(idx)
    } else {
        None
    }
}

pub fn footer_action_at(footer_area: Rect, col: u16, row: u16) -> Option<HomeFooterAction> {
    if row < footer_area.y || row >= footer_area.y + footer_area.height {
        return None;
    }
    let rel_row = row - footer_area.y;
    let rel_col = col.saturating_sub(footer_area.x);
    if rel_row == 1 {
        if rel_col < 14 {
            Some(HomeFooterAction::Search)
        } else if rel_col < 29 {
            Some(HomeFooterAction::Calendar)
        } else if rel_col < 45 {
            Some(HomeFooterAction::Tailored)
        } else if rel_col < 61 {
            Some(HomeFooterAction::Analytics)
        } else if rel_col < 76 {
            Some(HomeFooterAction::Remote)
        } else {
            Some(HomeFooterAction::Cast)
        }
    } else if rel_row == 2 {
        if rel_col < 15 {
            Some(HomeFooterAction::Filter)
        } else if rel_col < 31 {
            Some(HomeFooterAction::Settings)
        } else if rel_col < 46 {
            Some(HomeFooterAction::Setup)
        } else if rel_col < 60 {
            Some(HomeFooterAction::Help)
        } else {
            Some(HomeFooterAction::Quit)
        }
    } else {
        None
    }
}

pub fn home_layout(frame_area: Rect) -> [Rect; 4] {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TOP_BAR_HEIGHT),
            Constraint::Length(HERO_HEIGHT),
            Constraint::Min(CARD_HEIGHT + 2),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame_area);
    [chunks[0], chunks[1], chunks[2], chunks[3]]
}

pub fn render_home_screen(frame: &mut Frame, state: &mut AppState, modal_rect: Option<Rect>) {
    let [top_area, hero_area, grid_area, footer_area] = home_layout(frame.area());
    render_top_bar(frame, top_area, state);
    render_hero_stage(frame, hero_area, state, modal_rect);
    render_video_grid(frame, grid_area, state, modal_rect);
    render_footer_dock(frame, footer_area, state);
}

fn render_top_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let mut spans = vec![
        Span::styled(
            " PHAENNA ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];

    for (idx, (cat, items)) in state.categories.iter().enumerate() {
        let active = idx == state.active_category;
        let label = format!(" {} ({}) ", cat.label(), items.len());
        let st = if active {
            Style::default()
                .fg(Color::Black)
                .bg(theme.secondary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim).bg(Color::Rgb(24, 28, 38))
        };
        spans.push(Span::styled(label, st));
        spans.push(Span::raw(" "));
    }

    if state.is_fetching_more {
        if let Some((cur, tot)) = state.pagination_progress {
            spans.push(Span::styled(
                format!(" ⟳ +{}/{}", cur, tot),
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD),
            ));
        }
    } else if state.active_playback_video_id.is_some() {
        spans.push(Span::styled(
            format!(
                " ▶ {} ",
                format_duration_secs(state.active_playback_pos as u64)
            ),
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_hero_stage(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let selected = state.current_home_video();
    let pal = state.active_palette_for(selected.as_ref().map(|v| v.id.as_str()));

    let hero_blk = Block::default()
        .title(Span::styled(
            if selected.is_some() && state.config.ambilight_enabled {
                " ✦ FEATURED SPOTLIGHT (AMBILIGHT) "
            } else {
                " ✦ FEATURED SPOTLIGHT "
            },
            Style::default()
                .fg(pal.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_focus));

    let inner = hero_blk.inner(area);
    frame.render_widget(hero_blk, area);

    let Some(video) = selected else {
        let msg = if !state.loading_feeds.is_empty() {
            "⟳ Hydrating feeds from SQLite & YouTube..."
        } else {
            "Feed is empty. Press [L] to sync browser cookies, [/] to search, or [r] to refresh."
        };
        frame.render_widget(
            Paragraph::new(msg)
                .alignment(Alignment::Center)
                .style(Style::default().fg(pal.text_dim)),
            inner,
        );
        return;
    };

    state.queue_thumb_if_needed(&video.id, &video.best_thumbnail_url());

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(32),
            Constraint::Min(30),
            Constraint::Length(28),
        ])
        .split(inner);

    // 1. Hero Left 16:9 Artwork
    let thumb_area = Rect::new(
        cols[0].x + 1,
        cols[0].y,
        cols[0].width.saturating_sub(2),
        cols[0].height,
    );
    if !occluded_by_modal(thumb_area, modal_rect) && state.overlay == ActiveOverlay::None {
        if let Some(proto) = state.banner_cache.get_mut(&video.id) {
            frame.render_stateful_widget(StatefulImage::new(None), thumb_area, proto);
        } else if let Some(img) = state.raw_thumbs.get(&video.id) {
            frame.render_widget(HalfblockCover { image: img }, thumb_area);
        } else {
            let ph = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(color_from_id(&video.id)));
            frame.render_widget(
                Paragraph::new("HD 16:9").alignment(Alignment::Center).block(ph),
                thumb_area,
            );
        }
    }

    // 2. Hero Center Metadata
    let views = video
        .view_count
        .map(|v| format!("{} views", format_compact_count(v as i64)))
        .unwrap_or_else(|| "Phaenna".to_string());
    let mut badges = vec![
        Span::styled(
            format!(" {} ", video.display_uploader()),
            Style::default()
                .fg(Color::Black)
                .bg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            format!(" {} ", video.display_duration()),
            Style::default()
                .fg(Color::Black)
                .bg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(format!("  {} ", views), Style::default().fg(pal.text_dim)),
    ];
    if state.watched_ids.contains(&video.id) {
        badges.push(Span::styled(" ✓ Watched ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)));
    }
    if let Some(src) = state.watch_later_sources.get(&video.id) {
        badges.push(Span::styled(
            format!(" ★ {} ", src.pill_tag()),
            Style::default().fg(Color::Rgb(251, 191, 36)).add_modifier(Modifier::BOLD),
        ));
    }
    if state.offline_paths.contains_key(&video.id) {
        badges.push(Span::styled(" ⬇ Offline ", Style::default().fg(pal.primary)));
    }

    let desc_snippet = video
        .description
        .as_deref()
        .map(|s| s.replace('\n', " "))
        .unwrap_or_else(|| "Press [i] for full chapters, ReturnYouTubeDislike sentiment, SponsorBlock timeline & channel feed.".to_string());

    let center_lines = vec![
        Line::from(Span::styled(
            video.title.clone(),
            Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
        )),
        Line::from(""),
        Line::from(badges),
        Line::from(""),
        Line::from(Span::styled(desc_snippet, Style::default().fg(pal.text_dim))),
    ];
    frame.render_widget(
        Paragraph::new(center_lines).wrap(Wrap { trim: true }),
        cols[1],
    );

    // 3. Hero Right Telemetry & Action Box
    let act_blk = Block::default()
        .title(Span::styled(" Actions ", Style::default().fg(pal.secondary)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let resume_txt = state
        .progress_map
        .get(&video.id)
        .map(|(p, _)| format!("▶ Resume @ {}", format_duration_secs(*p as u64)))
        .unwrap_or_else(|| format!("▶ Play ({})", state.config.player.label()));
    let act_lines = vec![
        Line::from(Span::styled(
            resume_txt,
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled("[i] Detail & Chapters", Style::default().fg(pal.text))),
        Line::from(Span::styled("[p] Quality & Options", Style::default().fg(pal.text))),
        Line::from(Span::styled("[w] Toggle WatchLater", Style::default().fg(pal.text))),
        Line::from(Span::styled("[D] Offline Download", Style::default().fg(pal.text_dim))),
    ];
    frame.render_widget(Paragraph::new(act_lines).block(act_blk), cols[2]);
}

fn render_video_grid(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let base_theme = state.base_theme();
    let videos = state.current_filtered_videos();
    let cols = grid_columns(area.width);
    let visible_rows = ((area.height / (CARD_HEIGHT + 1)).max(1)) as usize;
    let cursor = state.category_cursors[state.active_category].min(videos.len().saturating_sub(1));
    let cursor_row = cursor / cols;

    let mut scroll_row = state.grid_scroll_rows[state.active_category];
    if cursor_row < scroll_row {
        scroll_row = cursor_row;
    } else if cursor_row >= scroll_row + visible_rows {
        scroll_row = cursor_row + 1 - visible_rows;
    }
    state.grid_scroll_rows[state.active_category] = scroll_row;

    if videos.is_empty() {
        frame.render_widget(
            Paragraph::new("No videos match current filter.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(base_theme.text_dim)),
            area,
        );
        return;
    }

    let start_idx = scroll_row * cols;
    let end_idx = (start_idx + visible_rows * cols).min(videos.len());

    // Queue next row thumbnails ahead cleanly
    for v in videos.iter().skip(start_idx).take(visible_rows * cols + cols) {
        state.queue_thumb_if_needed(&v.id, &v.best_thumbnail_url());
    }

    for (slot, idx) in (start_idx..end_idx).enumerate() {
        let r = slot / cols;
        let c = slot % cols;
        let card_x = area.x + 1 + (c as u16) * (CARD_WIDTH + CARD_GAP);
        let card_y = area.y + (r as u16) * (CARD_HEIGHT + 1);
        if card_y + CARD_HEIGHT > area.y + area.height || card_x + CARD_WIDTH > area.x + area.width {
            continue;
        }
        let card_rect = Rect::new(card_x, card_y, CARD_WIDTH, CARD_HEIGHT);
        let v = &videos[idx];
        let focused = idx == cursor;
        let card_pal = if focused {
            state.active_palette_for(Some(&v.id))
        } else {
            *base_theme
        };

        let border_col = if focused {
            card_pal.border_focus
        } else {
            card_pal.border_idle
        };

        let card_blk = Block::default()
            .borders(Borders::ALL)
            .border_type(if focused { BorderType::Thick } else { BorderType::Rounded })
            .border_style(Style::default().fg(border_col));
        let card_inner = card_blk.inner(card_rect);
        frame.render_widget(card_blk, card_rect);

        let card_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(6), Constraint::Min(3)])
            .split(card_inner);

        if !occluded_by_modal(card_chunks[0], modal_rect) && state.overlay == ActiveOverlay::None {
            if let Some(proto) = state.image_cache.get_mut(&v.id) {
                frame.render_stateful_widget(StatefulImage::new(None), card_chunks[0], proto);
            } else if let Some(img) = state.raw_thumbs.get(&v.id) {
                frame.render_widget(HalfblockCover { image: img }, card_chunks[0]);
            } else {
                frame.render_widget(
                    Paragraph::new("...")
                        .alignment(Alignment::Center)
                        .style(Style::default().fg(color_from_id(&v.id))),
                    card_chunks[0],
                );
            }
        }

        let title_str: String = v.title.chars().take((CARD_WIDTH as usize).saturating_sub(3)).collect();
        let dur_badge = v.display_duration();
        let meta_badge = if let Some((pos, dur)) = state.progress_map.get(&v.id) {
            if *dur > 0.0 {
                format!("{} · {}%", dur_badge, ((pos / dur) * 100.0).round() as u32)
            } else {
                dur_badge
            }
        } else if state.watched_ids.contains(&v.id) {
            format!("{} · ✓", dur_badge)
        } else if state.watch_later_sources.contains_key(&v.id) {
            format!("{} · ★", dur_badge)
        } else {
            dur_badge
        };

        let meta_lines = vec![
            Line::from(Span::styled(
                title_str,
                Style::default()
                    .fg(if focused { card_pal.primary } else { card_pal.text })
                    .add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() }),
            )),
            Line::from(vec![
                Span::styled(
                    format!("{:<14}", v.display_uploader().chars().take(14).collect::<String>()),
                    Style::default().fg(card_pal.text_dim),
                ),
                Span::styled(meta_badge, Style::default().fg(card_pal.accent)),
            ]),
        ];
        frame.render_widget(Paragraph::new(meta_lines), card_chunks[1]);
    }
}

fn render_footer_dock(frame: &mut Frame, area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    frame.render_widget(
        Paragraph::new("─".repeat(area.width as usize)).style(Style::default().fg(theme.divider)),
        rows[0],
    );

    let key = Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD);
    let sec = Style::default().fg(Color::Black).bg(theme.secondary).add_modifier(Modifier::BOLD);
    let txt = Style::default().fg(theme.text);

    let row1 = Line::from(vec![
        Span::styled(" / ", key), Span::styled(" Search   ", txt),
        Span::styled(" c ", sec), Span::styled(" Calendar   ", txt),
        Span::styled(" t ", key), Span::styled(" Discover   ", txt),
        Span::styled(" a ", sec), Span::styled(" Wrapped   ", txt),
        Span::styled(" m ", key), Span::styled(" Remote   ", txt),
        Span::styled(" C ", sec), Span::styled(" TV Cast ", txt),
    ]);
    let row2 = Line::from(vec![
        Span::styled(" f ", sec), Span::styled(format!(" {} ", state.content_filter.label()), txt),
        Span::styled(" s ", key), Span::styled(" Fuzzy Hub  ", txt),
        Span::styled(" ! ", sec), Span::styled(" Doctor    ", txt),
        Span::styled(" ? ", key), Span::styled(" Matrix   ", txt),
        Span::styled(" q ", sec), Span::styled(" Quit     ", txt),
    ]);

    frame.render_widget(Paragraph::new(row1), rows[1]);
    frame.render_widget(Paragraph::new(row2), rows[2]);
}

pub fn render_filter_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(60, 56, frame.area());
    frame.render_widget(Clear, area);

    let blk = Block::default()
        .title(Span::styled(
            " 󰈲 FILTER & SORT WORKSPACE ",
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(4),
            Constraint::Length(4),
            Constraint::Length(2),
        ])
        .split(inner);

    let formats = [ContentFilter::All, ContentFilter::Longform, ContentFilter::Shorts];
    let fmt_spans: Vec<Span> = formats
        .iter()
        .enumerate()
        .map(|(i, cf)| {
            let active = *cf == state.content_filter;
            let cursor = state.filter_modal_section == 0 && state.filter_modal_option == i;
            let st = if cursor {
                Style::default().fg(Color::Black).bg(theme.primary).add_modifier(Modifier::BOLD)
            } else if active {
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            Span::styled(format!(" [{}] ", cf.label()), st)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(fmt_spans)).block(
            Block::default()
                .title(" Content Format ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if state.filter_modal_section == 0 {
                    theme.border_focus
                } else {
                    theme.border_idle
                })),
        ),
        chunks[0],
    );

    let sorts = [
        SortOrder::Default,
        SortOrder::Newest,
        SortOrder::DurationLongest,
        SortOrder::DurationShortest,
        SortOrder::ChannelAZ,
    ];
    let sort_spans: Vec<Span> = sorts
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let active = *s == state.sort_order;
            let cursor = state.filter_modal_section == 1 && state.filter_modal_option == i;
            let st = if cursor {
                Style::default().fg(Color::Black).bg(theme.secondary).add_modifier(Modifier::BOLD)
            } else if active {
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            Span::styled(format!(" [{}] ", s.label()), st)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(sort_spans)).block(
            Block::default()
                .title(" Sort Order ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if state.filter_modal_section == 1 {
                    theme.border_focus
                } else {
                    theme.border_idle
                })),
        ),
        chunks[1],
    );

    frame.render_widget(
        Paragraph::new("[↑/↓] Switch Row · [←/→] Cycle · [Enter / Esc] Apply & Close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
        chunks[2],
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_home_render_and_hit_testing() {
        let mut st = AppState::new(Config::default());
        st.categories[1].1.push(VideoItem {
            id: "dQw4w9WgXcQ".to_string(),
            title: "Rust Async TUI Engineering Masterclass".to_string(),
            uploader: Some("ZenChannel".to_string()),
            duration: Some(920),
            view_count: Some(125_000),
            ..Default::default()
        });

        let area = Rect::new(0, 0, 120, 40);
        let [_, _, grid, footer] = home_layout(area);
        assert_eq!(grid_card_index_at(grid, grid.x + 3, grid.y + 2, 0, 1), Some(0));
        assert_eq!(footer_action_at(footer, 2, footer.y + 1), Some(HomeFooterAction::Search));
        assert!(top_bar_shelf_at(&st.categories, 16).is_some());

        let backend = TestBackend::new(120, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_home_screen(f, &mut st, None)).unwrap();
    }
}
