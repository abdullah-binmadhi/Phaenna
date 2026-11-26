//! Home Screen — Edge-to-Edge Bento Hero Stage, Adaptive Responsive Grid,
//! Infinite-Scroll Telemetry, and Interactive Continue Watching Queue Deck.

use crate::config::GridCardSize;
use crate::db::cache::{format_duration_secs, VideoItem};
use crate::state::{
    centered_rect, occluded_by_modal, ActiveOverlay, AppState, Category, ContentFilter,
    ShortsSource, SortOrder,
};
use crate::ui::components::cover::render_thumbnail_cover;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};
use std::time::{SystemTime, UNIX_EPOCH};

fn now_unix_home() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Smooth circular stock-ticker window over overflowing card/hero titles (UTF-8 safe).
pub fn marquee_window(text: &str, width: usize, step: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width || step == 0 {
        return chars.into_iter().take(width).collect();
    }
    let sep = [' ', ' ', ' ', '/', '/', '/', ' ', ' ', ' '];
    let mut tape = chars;
    tape.extend_from_slice(&sep);
    let start = step % tape.len();
    (0..width).map(|i| tape[(start + i) % tape.len()]).collect()
}

pub const TOP_BAR_HEIGHT: u16 = 1;
pub const HERO_HEIGHT: u16 = 8;
pub const FOOTER_HEIGHT: u16 = 0;
#[allow(dead_code)]
pub const MIN_CARD_WIDTH: u16 = 34;
#[allow(dead_code)]
pub const CARD_HEIGHT: u16 = 13;
pub const CARD_GAP: u16 = 2;

const CW_ROW_HEIGHT: u16 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeFooterAction {
    Search,
    Calendar,
    Analytics,
    Remote,
    Cast,
    Filter,
    Settings,
    Help,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GridMetrics {
    pub area: Rect,
    pub cols: usize,
    pub visible_rows: usize,
    pub base_card_width: u16,
    pub extra_width_cols: usize,
    pub card_height: u16,
    pub col_gap: u16,
    pub row_gap: u16,
    pub status_row_y: Option<u16>,
    pub scrollbar_x: Option<u16>,
}

impl GridMetrics {
    pub fn col_x_and_width(&self, c: usize) -> (u16, u16) {
        let mut x = self.area.x + 1;
        for i in 0..c {
            let w = self.base_card_width + u16::from(i < self.extra_width_cols);
            x = x.saturating_add(w).saturating_add(self.col_gap);
        }
        let width = self.base_card_width + u16::from(c < self.extra_width_cols);
        (x, width)
    }

    pub fn row_y_and_height(&self, r: usize) -> (u16, u16) {
        let y = self
            .area
            .y
            .saturating_add((r as u16) * (self.card_height + self.row_gap));
        (y, self.card_height)
    }
}

/// Compute edge-to-edge horizontal card widths and zero-void vertical row packing.
#[allow(dead_code)]
pub fn compute_grid_metrics(area: Rect) -> GridMetrics {
    compute_grid_metrics_for_size(area, GridCardSize::default())
}

/// Compute YouTube-style borderless grid dimensions adapting thumbnail height and card width cleanly to terminal size.
pub fn compute_grid_metrics_for_size(area: Rect, size_mode: GridCardSize) -> GridMetrics {
    let scrollbar_x = if area.width >= 40 {
        Some(area.x + area.width - 1)
    } else {
        None
    };
    let reserve_right: u16 = if scrollbar_x.is_some() { 2 } else { 1 };
    let usable_w = area.width.saturating_sub(1 + reserve_right).max(24);

    let status_row_y = if area.height >= 12 {
        Some(area.y + area.height - 1)
    } else {
        None
    };
    let avail_h = if status_row_y.is_some() {
        area.height.saturating_sub(1)
    } else {
        area.height
    }
    .max(8);

    let (visible_rows, card_height, best_gap, cols) = match size_mode {
        GridCardSize::CinemaLarge => {
            // YouTube.com style: 2 big 16:9 rows on compact/Hyprland windows, 3 rows on tall monitors.
            let rows: usize = if avail_h >= 35 {
                3
            } else if avail_h >= 18 {
                2
            } else {
                1
            };
            let rgap: u16 = if rows > 1 && avail_h >= 20 { 1 } else { 0 };
            let total_rgap = (rows.saturating_sub(1) as u16) * rgap;
            let ch = (avail_h.saturating_sub(total_rgap) / (rows as u16)).clamp(8, 20);

            // Borderless card: all `ch` rows go directly to thumbnail + metadata (`meta_h = 2..3`).
            let meta_est: u16 = if ch >= 14 {
                4
            } else if ch >= 10 {
                3
            } else {
                2
            };
            let thumb_est = ch.saturating_sub(meta_est).max(5);
            let ideal_card_w = ((thumb_est as f32) * 3.85).round().clamp(30.0, 48.0) as u16;
            let cols = ((usable_w + CARD_GAP) / (ideal_card_w + CARD_GAP)).clamp(1, 4) as usize;
            (rows, ch, rgap, cols)
        }
        GridCardSize::AdaptiveLarge => {
            // Compact 3-Row preset when height allows (`>= 22`), adapting width to thumbnail height.
            let rows: usize = if avail_h >= 22 {
                3
            } else if avail_h >= 15 {
                2
            } else {
                1
            };
            let rgap: u16 = if (rows == 3 && avail_h >= 29) || (rows == 2 && avail_h >= 20) {
                1
            } else {
                0
            };
            let total_rgap = (rows.saturating_sub(1) as u16) * rgap;
            let ch = (avail_h.saturating_sub(total_rgap) / (rows as u16)).clamp(7, 20);
            let meta_est: u16 = if ch >= 10 { 3 } else { 2 };
            let thumb_est = ch.saturating_sub(meta_est).max(4);
            let ideal_card_w = ((thumb_est as f32) * 3.85).round().clamp(24.0, 42.0) as u16;
            let cols = ((usable_w + CARD_GAP) / (ideal_card_w + CARD_GAP)).clamp(1, 5) as usize;
            (rows, ch, rgap, cols)
        }
    };

    let total_gaps = (cols.saturating_sub(1) as u16) * CARD_GAP;
    let avail_for_cards = usable_w.saturating_sub(total_gaps).max(cols as u16);
    let base_card_width = (avail_for_cards / (cols as u16)).max(16);
    let extra_width_cols = (avail_for_cards % (cols as u16)) as usize;

    GridMetrics {
        area,
        cols,
        visible_rows,
        base_card_width,
        extra_width_cols,
        card_height,
        col_gap: CARD_GAP,
        row_gap: best_gap,
        status_row_y,
        scrollbar_x,
    }
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

#[allow(dead_code)]
pub fn grid_columns(width: u16) -> usize {
    grid_columns_for_size(width, GridCardSize::default())
}

#[allow(dead_code)]
pub fn grid_columns_for_size(width: u16, size_mode: GridCardSize) -> usize {
    compute_grid_metrics_for_size(Rect::new(0, 0, width, 40), size_mode).cols
}

#[allow(dead_code)]
pub fn grid_card_index_at(
    grid_area: Rect,
    col: u16,
    row: u16,
    scroll_row: usize,
    total_items: usize,
) -> Option<usize> {
    grid_card_index_at_for_size(
        grid_area,
        col,
        row,
        scroll_row,
        total_items,
        GridCardSize::default(),
    )
}

pub fn grid_card_index_at_for_size(
    grid_area: Rect,
    col: u16,
    row: u16,
    scroll_row: usize,
    total_items: usize,
    size_mode: GridCardSize,
) -> Option<usize> {
    if col < grid_area.x || col >= grid_area.x + grid_area.width {
        return None;
    }
    if row < grid_area.y || row >= grid_area.y + grid_area.height {
        return None;
    }
    let m = compute_grid_metrics_for_size(grid_area, size_mode);
    if let Some(sb_x) = m.scrollbar_x {
        if col >= sb_x {
            return None;
        }
    }

    let mut matched_col = None;
    for c in 0..m.cols {
        let (cx, cw) = m.col_x_and_width(c);
        if col >= cx && col < cx + cw {
            matched_col = Some(c);
            break;
        }
    }
    let c_idx = matched_col?;

    let mut matched_row = None;
    for r in 0..m.visible_rows {
        let (ry, rh) = m.row_y_and_height(r);
        if row >= ry && row < ry + rh {
            matched_row = Some(r);
            break;
        }
    }
    let r_idx = matched_row? + scroll_row;
    let idx = r_idx * m.cols + c_idx;
    if idx < total_items {
        Some(idx)
    } else {
        None
    }
}

/// Hit-test a video row inside the Continue Watching split queue list pane.
pub fn continue_watching_row_at(
    grid_area: Rect,
    col: u16,
    row: u16,
    scroll_idx: usize,
    total_items: usize,
) -> Option<usize> {
    if total_items == 0 {
        return None;
    }
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(grid_area);
    let left_inner = Block::default().borders(Borders::ALL).inner(panes[0]);
    let list_h = left_inner.height.saturating_sub(1);
    let list_rect = Rect::new(left_inner.x, left_inner.y, left_inner.width, list_h);
    if col < list_rect.x
        || col >= list_rect.x + list_rect.width
        || row < list_rect.y
        || row >= list_rect.y + list_rect.height
    {
        return None;
    }
    let slot = ((row - list_rect.y) / CW_ROW_HEIGHT) as usize;
    let idx = scroll_idx + slot;
    if idx < total_items {
        Some(idx)
    } else {
        None
    }
}

#[allow(dead_code)]
pub fn footer_action_at(footer_area: Rect, col: u16, row: u16) -> Option<HomeFooterAction> {
    if footer_area.height == 0 || row < footer_area.y || row >= footer_area.y + footer_area.height {
        return None;
    }
    let rel_row = row - footer_area.y;
    let rel_col = col.saturating_sub(footer_area.x);
    if rel_row == 1 {
        if rel_col < 13 {
            Some(HomeFooterAction::Search)
        } else if rel_col < 28 {
            Some(HomeFooterAction::Calendar)
        } else if rel_col < 43 {
            Some(HomeFooterAction::Analytics)
        } else if rel_col < 56 {
            Some(HomeFooterAction::Remote)
        } else {
            Some(HomeFooterAction::Cast)
        }
    } else if rel_row == 2 {
        if rel_col < 19 {
            Some(HomeFooterAction::Filter)
        } else if rel_col < 34 {
            Some(HomeFooterAction::Settings)
        } else if rel_col < 47 {
            Some(HomeFooterAction::Help)
        } else {
            Some(HomeFooterAction::Quit)
        }
    } else {
        None
    }
}

pub fn home_layout(frame_area: Rect) -> [Rect; 4] {
    let hero_h = if frame_area.height <= 38 {
        6
    } else if frame_area.height <= 46 {
        7
    } else {
        HERO_HEIGHT
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(TOP_BAR_HEIGHT),
            Constraint::Length(hero_h),
            Constraint::Min(10),
            Constraint::Length(FOOTER_HEIGHT),
        ])
        .split(frame_area);
    [chunks[0], chunks[1], chunks[2], chunks[3]]
}

/// Effective grid area after reserving the 1-row Shorts source selector bar when on the Shorts shelf.
pub fn effective_grid_area(grid_area: Rect, active_cat: Category) -> Rect {
    if active_cat == Category::Shorts && grid_area.height >= 4 {
        Rect::new(
            grid_area.x,
            grid_area.y + 1,
            grid_area.width,
            grid_area.height - 1,
        )
    } else {
        grid_area
    }
}

/// Hit-test the 1-row Shorts source filter pills (`Algorithm Suggested` vs `Subscribed`).
pub fn shorts_source_at(
    grid_area: Rect,
    sug_len: usize,
    sub_len: usize,
    col: u16,
    row: u16,
) -> Option<ShortsSource> {
    if row != grid_area.y || grid_area.height < 4 {
        return None;
    }
    let sug_label = format!(" * Algorithm Suggested ({sug_len}) ");
    let sub_label = format!(" Subscribed ({sub_len}) ");
    let sug_start = grid_area.x + 1 + " Shorts Source: ".chars().count() as u16;
    let sug_end = sug_start + sug_label.chars().count() as u16;
    let sub_start = sug_end + 1;
    let sub_end = sub_start + sub_label.chars().count() as u16;
    if col >= sug_start && col < sug_end {
        Some(ShortsSource::Suggested)
    } else if col >= sub_start && col < sub_end {
        Some(ShortsSource::Subscribed)
    } else {
        None
    }
}

fn render_shorts_filter_bar(frame: &mut Frame, bar_area: Rect, state: &AppState) {
    let theme = state.base_theme();
    let sug_len = state.shorts_suggested.len();
    let sub_len = state.shorts_subscribed.len();
    let sug_active = state.shorts_source == ShortsSource::Suggested;
    let sub_active = state.shorts_source == ShortsSource::Subscribed;

    let sug_lbl = if sug_active {
        format!(" * Algorithm Suggested ({sug_len}) ")
    } else {
        format!("   Algorithm Suggested ({sug_len}) ")
    };
    let sub_lbl = if sub_active {
        format!(" * Subscribed ({sub_len}) ")
    } else {
        format!("   Subscribed ({sub_len}) ")
    };

    let active_st = Style::default()
        .fg(Color::Black)
        .bg(theme.accent)
        .add_modifier(Modifier::BOLD);
    let idle_st = Style::default()
        .fg(theme.text)
        .bg(Color::Rgb(26, 30, 42));

    let line = Line::from(vec![
        Span::styled(" Shorts Source: ", Style::default().fg(theme.text_dim)),
        Span::styled(sug_lbl, if sug_active { active_st } else { idle_st }),
        Span::raw(" "),
        Span::styled(sub_lbl, if sub_active { active_st } else { idle_st }),
        Span::styled(
            "   ·  [S] Switch Source  ·  [n] Load More  ·  [Enter/Space] Watch",
            Style::default().fg(theme.text_dim),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(line),
        Rect::new(bar_area.x + 1, bar_area.y, bar_area.width.saturating_sub(2), 1),
    );
}

pub fn render_home_screen(frame: &mut Frame, state: &mut AppState, modal_rect: Option<Rect>) {
    state.home_channel_hitboxes.clear();
    let [top_area, hero_area, mut grid_area, _footer_area] = home_layout(frame.area());
    render_top_bar(frame, top_area, state);
    render_hero_stage(frame, hero_area, state, modal_rect);

    let show_now_playing = state.has_live_ipc() && grid_area.height >= 12;
    let now_playing_area = if show_now_playing {
        let bar = Rect::new(grid_area.x, grid_area.y + grid_area.height - 1, grid_area.width, 1);
        grid_area.height -= 1;
        Some(bar)
    } else {
        state.live_playback.now_playing_hitboxes.clear();
        None
    };

    let active_cat = state
        .categories
        .get(state.active_category)
        .map(|(c, _)| *c)
        .unwrap_or(Category::Recommended);

    if active_cat == Category::ContinueWatching && !state.continue_watching_grid_mode {
        render_continue_watching_deck(frame, grid_area, state, modal_rect);
    } else {
        let card_grid_area = effective_grid_area(grid_area, active_cat);
        if active_cat == Category::Shorts && grid_area.height >= 4 {
            render_shorts_filter_bar(
                frame,
                Rect::new(grid_area.x, grid_area.y, grid_area.width, 1),
                state,
            );
        }
        render_video_grid(frame, card_grid_area, state, modal_rect);
    }

    if let Some(bar_rect) = now_playing_area {
        crate::ui::play_options::render_now_playing_bar(frame, bar_rect, state);
    }
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
            Style::default()
                .fg(theme.text_dim)
                .bg(Color::Rgb(24, 28, 38))
        };
        spans.push(Span::styled(label, st));
        spans.push(Span::raw(" "));
    }

    if state.is_fetching_more {
        if let Some((cur, tot)) = state.pagination_progress {
            spans.push(Span::styled(
                format!(" +Loading {cur}/{tot} "),
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                " +Loading more... ",
                Style::default()
                    .fg(theme.accent)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    } else if state.active_playback_video_id.is_some() {
        spans.push(Span::styled(
            format!(
                " > {} ",
                format_duration_secs(state.active_playback_pos as u64)
            ),
            Style::default()
                .fg(Color::Black)
                .bg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ));
    } else if area.width >= 96 {
        if let Some(acct) = state.auth_vault.active_account() {
            let tag = acct
                .channel_handle
                .as_deref()
                .unwrap_or(&acct.display_name);
            spans.push(Span::styled(
                format!(" [OK] {tag} "),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ));
        } else {
            spans.push(Span::styled(
                " [L] Sign In ",
                Style::default().fg(theme.text_dim),
            ));
        }
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn build_progress_spans(
    pos: f64,
    dur: f64,
    bar_width: usize,
    fill_color: Color,
) -> (Vec<Span<'static>>, u32) {
    let frac = if dur > 0.0 {
        (pos / dur).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let filled = ((frac * bar_width as f64).round() as usize).min(bar_width);
    let empty = bar_width.saturating_sub(filled);
    let pct = (frac * 100.0).round() as u32;
    let spans = vec![
        Span::styled("━".repeat(filled), Style::default().fg(fill_color)),
        Span::styled("─".repeat(empty), Style::default().fg(Color::Rgb(54, 62, 82))),
    ];
    (spans, pct)
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
                " FEATURED SPOTLIGHT (AMBILIGHT) "
            } else {
                " FEATURED SPOTLIGHT "
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
            "Hydrating feeds from SQLite cache & YouTube..."
        } else {
            "Feed is empty. Press [L] to sign in with Google, [/] to search, or [r] to refresh."
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
            Constraint::Min(32),
            Constraint::Length(34),
        ])
        .split(inner);

    // 1. Hero Left 16:9 Artwork (edge-to-edge full column width, 1-col divider gap)
    let thumb_area = Rect::new(
        cols[0].x,
        cols[0].y,
        cols[0].width.saturating_sub(1),
        cols[0].height,
    );
    if !occluded_by_modal(thumb_area, modal_rect) && state.overlay == ActiveOverlay::None {
        render_thumbnail_cover(frame, thumb_area, &video.id, state, "HD 16:9");
    }

    // 2. Hero Center Rich Metadata & Synopsis
    let active_cat_label = state
        .categories
        .get(state.active_category)
        .map(|(c, _)| {
            if *c == Category::Shorts {
                format!("SHORTS · {}", state.shorts_source.label().to_uppercase())
            } else {
                c.label().to_uppercase()
            }
        })
        .unwrap_or_else(|| "FEED".to_string());
    let now_unix = now_unix_home();
    let views_part = video.compact_views(true);
    let age_part = video.compact_age(now_unix, true);
    let telemetry_str = match (views_part, age_part) {
        (Some(v), Some(a)) => format!("{v} · {a}"),
        (Some(v), None) => v,
        (None, Some(a)) => a,
        (None, None) => String::new(),
    };

    let mut badge_offset_x: u16 = format!(" {active_cat_label} ").chars().count() as u16 + 1;
    let mut badges = vec![
        Span::styled(
            format!(" {active_cat_label} "),
            Style::default()
                .fg(Color::Black)
                .bg(pal.primary)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
    ];
    if video.is_new_release(now_unix) {
        badges.push(Span::styled(
            " NEW ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        ));
        badges.push(Span::raw(" "));
        badge_offset_x += 6;
    }
    let ch_pill = format!(" {} > ", video.display_uploader());
    let ch_pill_w = ch_pill.chars().count() as u16;
    badges.push(Span::styled(
        ch_pill,
        Style::default()
            .fg(Color::Black)
            .bg(pal.secondary)
            .add_modifier(Modifier::BOLD),
    ));
    badges.push(Span::raw(" "));
    badges.push(Span::styled(
        format!(" {} ", video.display_duration()),
        Style::default()
            .fg(Color::Black)
            .bg(pal.accent)
            .add_modifier(Modifier::BOLD),
    ));
    if !telemetry_str.is_empty() {
        badges.push(Span::styled(
            format!("  {telemetry_str} "),
            Style::default().fg(pal.text_dim),
        ));
    }
    if state.is_in_continue_watching(&video.id) {
        badges.push(Span::styled(
            " [IN QUEUE] ",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if state.watched_ids.contains(&video.id) {
        badges.push(Span::styled(
            " [WATCHED] ",
            Style::default()
                .fg(pal.accent)
                .add_modifier(Modifier::BOLD),
        ));
    }
    if let Some(src) = state.watch_later_sources.get(&video.id) {
        badges.push(Span::styled(
            format!(" [WL:{}] ", src.pill_tag()),
            Style::default()
                .fg(Color::Rgb(251, 191, 36))
                .add_modifier(Modifier::BOLD),
        ));
    }
    if state.offline_paths.contains_key(&video.id) {
        badges.push(Span::styled(" [OFFLINE] ", Style::default().fg(pal.primary)));
    }

    let desc_raw = video
        .description
        .as_deref()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            "Press [i] for chapters, dislike telemetry, SponsorBlock timeline & channel catalog."
                .to_string()
        });
    let max_desc_chars = (cols[1].width as usize).saturating_mul(2).clamp(48, 180);

    let progress_line = if let Some((pos, dur)) = state.progress_map.get(&video.id) {
        let bar_w = (cols[1].width as usize).saturating_sub(28).clamp(12, 34);
        let (mut bar_spans, pct) = build_progress_spans(*pos, *dur, bar_w, pal.accent);
        let mut line_spans = vec![Span::styled(
            format!("Resume {} ", format_duration_secs(*pos as u64)),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        )];
        line_spans.append(&mut bar_spans);
        line_spans.push(Span::styled(
            format!(" {pct}%"),
            Style::default().fg(pal.text_dim),
        ));
        Line::from(line_spans)
    } else {
        Line::from(vec![
            Span::styled("[Space] ", Style::default().fg(pal.accent).add_modifier(Modifier::BOLD)),
            Span::styled("Play   ", Style::default().fg(pal.text)),
            Span::styled("[Enter] ", Style::default().fg(pal.primary).add_modifier(Modifier::BOLD)),
            Span::styled("Inspect   ", Style::default().fg(pal.text)),
            Span::styled("[a] ", Style::default().fg(pal.secondary).add_modifier(Modifier::BOLD)),
            Span::styled("+Continue Watching   ", Style::default().fg(pal.text)),
            Span::styled("[w] ", Style::default().fg(pal.text_dim)),
            Span::styled("+WatchLater", Style::default().fg(pal.text_dim)),
        ])
    };

    let center_area = Rect::new(
        cols[1].x + 1,
        cols[1].y,
        cols[1].width.saturating_sub(2),
        cols[1].height,
    );
    let ch_x = center_area.x.saturating_add(badge_offset_x);
    let ch_max_w = (center_area.x + center_area.width).saturating_sub(ch_x);
    if ch_max_w > 0 && ch_pill_w > 0 {
        state.home_channel_hitboxes.push((
            Rect::new(ch_x, center_area.y, ch_pill_w.min(ch_max_w), 1),
            video.clone(),
        ));
    }
    let desc_snippet =
        crate::ui::detail::truncate_word_boundary(&desc_raw, max_desc_chars.min(center_area.width as usize * 2));
    let center_lines = vec![
        Line::from(badges),
        Line::from(Span::styled(
            marquee_window(&video.title, center_area.width as usize, state.ticker_step),
            Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(desc_snippet, Style::default().fg(pal.text_dim))),
        progress_line,
    ];
    frame.render_widget(
        Paragraph::new(center_lines).wrap(Wrap { trim: true }),
        center_area,
    );

    // 3. Hero Right Box: Continue Watching Up-Next Preview OR Actions
    let cw_items = state
        .categories
        .iter()
        .find(|(c, _)| *c == Category::ContinueWatching)
        .map(|(_, v)| v.as_slice())
        .unwrap_or(&[]);

    let (right_title, mut right_lines) = if !cw_items.is_empty()
        && state.categories[state.active_category].0 != Category::ContinueWatching
    {
        let title = format!(" Up Next ({}) · [1] Open ", cw_items.len());
        let mut lines = Vec::new();
        for (i, item) in cw_items.iter().take(4).enumerate() {
            let max_t = cols[2].width.saturating_sub(11) as usize;
            let short_t: String = item.title.chars().take(max_t).collect();
            let dur = item.display_duration();
            lines.push(Line::from(vec![
                Span::styled(
                    format!("{}. ", i + 1),
                    Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(short_t, Style::default().fg(pal.text)),
                Span::styled(format!(" [{dur}]"), Style::default().fg(pal.text_dim)),
            ]));
        }
        lines.push(Line::from(Span::styled(
            "[a] Pin selected video to queue",
            Style::default().fg(pal.secondary),
        )));
        (title, lines)
    } else {
        let resume_txt = state
            .progress_map
            .get(&video.id)
            .map(|(p, _)| format!("> Resume @ {}", format_duration_secs(*p as u64)))
            .unwrap_or_else(|| format!("> Play ({})", state.config.player.label()));
        let lines = vec![
            Line::from(Span::styled(
                resume_txt,
                Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled("[a] Add to Continue Watching", Style::default().fg(pal.text))),
            Line::from(Span::styled("[i] Detail & Chapters", Style::default().fg(pal.text))),
            Line::from(Span::styled("[p] Quality & Player Options", Style::default().fg(pal.text))),
            Line::from(Span::styled("[w] WatchLater · [D] Offline", Style::default().fg(pal.text_dim))),
        ];
        (" Quick Actions ".to_string(), lines)
    };

    if right_lines.len() < cols[2].height as usize {
        while right_lines.len() + 2 < cols[2].height as usize && right_lines.len() < 4 {
            right_lines.push(Line::from(""));
        }
    }

    let act_blk = Block::default()
        .title(Span::styled(right_title, Style::default().fg(pal.secondary)))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    frame.render_widget(Paragraph::new(right_lines).block(act_blk), cols[2]);
}

/// Render the dedicated Continue Watching interactive queue & inspector deck.
fn render_continue_watching_deck(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let base_theme = *state.base_theme();
    let videos = state.current_filtered_videos();

    if videos.is_empty() {
        let empty_blk = Block::default()
            .title(Span::styled(
                " CONTINUE WATCHING QUEUE (0) ",
                Style::default()
                    .fg(base_theme.primary)
                    .add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(base_theme.border_idle));
        let lines = vec![
            Line::from(""),
            Line::from(Span::styled(
                "Your Continue Watching queue is empty.",
                Style::default()
                    .fg(base_theme.primary)
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "Press [a] or [+] on any video in Home Feed [2], Subscriptions [3], History [4], Search [/], or Detail",
                Style::default().fg(base_theme.text),
            )),
            Line::from(Span::styled(
                "to queue it cleanly here. Resume timestamps are saved here and across every feed.",
                Style::default().fg(base_theme.text_dim),
            )),
            Line::from(""),
            Line::from(Span::styled(
                "When saved here: click or press [Enter]/[Space] to watch immediately, and [x]/[d] to remove when done.",
                Style::default().fg(base_theme.accent),
            )),
        ];
        frame.render_widget(
            Paragraph::new(lines)
                .alignment(Alignment::Center)
                .block(empty_blk)
                .wrap(Wrap { trim: true }),
            area,
        );
        return;
    }

    let cursor = state.category_cursors[state.active_category].min(videos.len().saturating_sub(1));
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(area);

    // 1. Left Queue List Pane
    let left_blk = Block::default()
        .title(Span::styled(
            format!(" CONTINUE WATCHING QUEUE ({} saved) ", videos.len()),
            Style::default()
                .fg(base_theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(base_theme.border_focus));
    let left_inner = left_blk.inner(panes[0]);
    frame.render_widget(left_blk, panes[0]);

    let status_h = 1u16;
    let list_h = left_inner.height.saturating_sub(status_h);
    let visible_slots = ((list_h / CW_ROW_HEIGHT).max(1)) as usize;

    let mut scroll_idx = state.grid_scroll_rows[state.active_category];
    if cursor < scroll_idx {
        scroll_idx = cursor;
    } else if cursor >= scroll_idx + visible_slots {
        scroll_idx = cursor + 1 - visible_slots;
    }
    state.grid_scroll_rows[state.active_category] = scroll_idx;

    for v in videos.iter().skip(scroll_idx).take(visible_slots + 2) {
        state.queue_thumb_if_needed(&v.id, &v.best_thumbnail_url());
    }

    let list_w = left_inner.width.saturating_sub(2);
    for (slot, idx) in (scroll_idx..(scroll_idx + visible_slots).min(videos.len())).enumerate() {
        let v = &videos[idx];
        let sel = idx == cursor;
        let row_y = left_inner.y + (slot as u16) * CW_ROW_HEIGHT;
        let slot_rect = Rect::new(left_inner.x, row_y, list_w, CW_ROW_HEIGHT);

        let pal = if sel {
            state.active_palette_for(Some(&v.id))
        } else {
            base_theme
        };
        let row_bg = if sel {
            Color::Rgb(28, 34, 48)
        } else {
            Color::Reset
        };

        let meta = state.continue_watching_meta.get(v.clean_id());
        let prog = state
            .progress_map
            .get(&v.id)
            .copied()
            .or_else(|| meta.map(|m| (m.last_pos_secs, m.duration_secs)))
            .unwrap_or((0.0, v.duration.unwrap_or(0) as f64));
        let is_finished = state.watched_ids.contains(&v.id)
            && prog.1 > 0.0
            && (prog.0 / prog.1) >= 0.85;

        let prefix = if sel { " > " } else { "   " };
        let title_budget = (list_w as usize).saturating_sub(14);
        let title_trunc: String = v.title.chars().take(title_budget).collect();

        let status_label = if is_finished {
            "FINISHED - press [x] to remove".to_string()
        } else if prog.0 >= 5.0 {
            format!("Resume @ {}", format_duration_secs(prog.0 as u64))
        } else if meta.map(|m| m.pinned_manually).unwrap_or(false) {
            "Queued manually (0:00)".to_string()
        } else {
            "In progress".to_string()
        };

        let bar_w = (list_w as usize).saturating_sub(38).clamp(10, 32);
        let (mut bar_spans, pct) = build_progress_spans(
            prog.0,
            prog.1.max(v.duration.unwrap_or(0) as f64),
            bar_w,
            if is_finished { Color::Green } else { pal.accent },
        );

        let mut line3 = vec![
            Span::styled("        ", Style::default().bg(row_bg)),
        ];
        line3.append(&mut bar_spans);
        line3.push(Span::styled(
            format!(" {:>3}%  [Enter] Watch  [x] Remove", pct),
            Style::default()
                .fg(if sel { pal.text } else { pal.text_dim })
                .bg(row_bg),
        ));

        let cw_up_label = format!("{} >", v.display_uploader());
        let cw_up_w = cw_up_label.chars().count() as u16;
        let lines = vec![
            Line::from(vec![
                Span::styled(prefix, Style::default().fg(pal.primary).bg(row_bg).add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("[{:02}] ", idx + 1),
                    Style::default().fg(pal.secondary).bg(row_bg).add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!("{title_trunc:<title_budget$}"),
                    Style::default()
                        .fg(if sel { pal.primary } else { pal.text })
                        .bg(row_bg)
                        .add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() }),
                ),
            ]),
            Line::from(vec![
                Span::styled("        ", Style::default().bg(row_bg)),
                Span::styled(
                    cw_up_label,
                    Style::default()
                        .fg(pal.secondary)
                        .bg(row_bg)
                        .add_modifier(Modifier::UNDERLINED),
                ),
                Span::styled(
                    format!("  ·  {}  ·  ", v.display_duration()),
                    Style::default().fg(pal.text_dim).bg(row_bg),
                ),
                Span::styled(
                    status_label,
                    Style::default()
                        .fg(if is_finished { Color::Green } else { pal.accent })
                        .bg(row_bg),
                ),
            ]),
            Line::from(line3),
        ];

        frame.render_widget(Paragraph::new(lines).style(Style::default().bg(row_bg)), slot_rect);
        let cw_avail = slot_rect.width.saturating_sub(8);
        if cw_avail > 0 && cw_up_w > 0 {
            state.home_channel_hitboxes.push((
                Rect::new(slot_rect.x + 8, slot_rect.y + 1, cw_up_w.min(cw_avail), 1),
                v.clone(),
            ));
        }
    }

    // Scrollbar for Continue Watching list
    if list_h > 0 && videos.len() > visible_slots {
        let sb_x = left_inner.x + left_inner.width.saturating_sub(1);
        render_vertical_scrollbar(
            frame,
            sb_x,
            left_inner.y,
            list_h,
            scroll_idx,
            visible_slots,
            videos.len(),
            base_theme.primary,
            base_theme.divider,
        );
    }

    if left_inner.height >= 2 {
        let hint_rect = Rect::new(
            left_inner.x,
            left_inner.y + left_inner.height - 1,
            left_inner.width,
            1,
        );
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled(" [Enter/Space] Watch ", Style::default().fg(base_theme.accent).add_modifier(Modifier::BOLD)),
                Span::styled("· [x/d] Remove ", Style::default().fg(base_theme.primary).add_modifier(Modifier::BOLD)),
                Span::styled("· [X] Clear Finished ", Style::default().fg(base_theme.secondary)),
                Span::styled("· [K/J] Move ", Style::default().fg(base_theme.text_dim)),
                Span::styled("· [v] Grid View", Style::default().fg(base_theme.text_dim)),
            ])),
            hint_rect,
        );
    }

    // 2. Right Preview & Controls Pane
    let sel_vid = &videos[cursor];
    let pal = state.active_palette_for(Some(&sel_vid.id));
    let right_blk = Block::default()
        .title(Span::styled(
            " QUEUE INSPECTOR ",
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_idle));
    let right_inner = right_blk.inner(panes[1]);
    frame.render_widget(right_blk, panes[1]);

    let r_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(12), Constraint::Min(5)])
        .split(right_inner);

    let thumb_rect = r_chunks[0];
    let sel_id = sel_vid.id.clone();
    if !occluded_by_modal(thumb_rect, modal_rect) && state.overlay == ActiveOverlay::None {
        render_thumbnail_cover(frame, thumb_rect, &sel_id, state, "Loading thumbnail...");
    }

    let desc = sel_vid
        .description
        .as_deref()
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "No description cached yet. Press [i] to fetch full details, subtitles, and SponsorBlock segments.".to_string());

    let info_rect = Rect::new(
        r_chunks[1].x + 1,
        r_chunks[1].y,
        r_chunks[1].width.saturating_sub(2),
        r_chunks[1].height,
    );
    let info_w = info_rect.width.max(12) as usize;
    let title_wrapped = crate::ui::detail::wrap_words(&sel_vid.title, info_w);
    let creator_row_y = info_rect.y + title_wrapped.len() as u16;
    let ch_badge = format!("[ {} > ]", sel_vid.display_uploader());
    let ch_badge_w = ch_badge.chars().count() as u16;

    let mut detail_lines: Vec<Line<'static>> = title_wrapped
        .into_iter()
        .map(|ln| {
            Line::from(Span::styled(
                ln,
                Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
            ))
        })
        .collect();
    detail_lines.push(Line::from(vec![
        Span::styled(
            ch_badge,
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        ),
        Span::styled(
            format!("  ·  {}", sel_vid.display_duration()),
            Style::default().fg(pal.text_dim),
        ),
    ]));
    detail_lines.push(Line::from(""));
    for dln in crate::ui::detail::wrap_words(&desc, info_w).into_iter().take(4) {
        detail_lines.push(Line::from(Span::styled(
            dln,
            Style::default().fg(pal.text_dim),
        )));
    }
    detail_lines.push(Line::from(""));
    detail_lines.push(Line::from(Span::styled(
        "[Enter / Space] Play & Resume",
        Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
    )));
    detail_lines.push(Line::from(Span::styled(
        "[x / d] Remove from Continue Watching",
        Style::default().fg(pal.primary),
    )));
    detail_lines.push(Line::from(Span::styled(
        "[i] Detail  ·  [w] +WatchLater  ·  Click Creator -> Channel",
        Style::default().fg(pal.text),
    )));

    frame.render_widget(Paragraph::new(detail_lines), info_rect);
    if creator_row_y < info_rect.y + info_rect.height && info_rect.width > 0 {
        state.home_channel_hitboxes.push((
            Rect::new(
                info_rect.x,
                creator_row_y,
                ch_badge_w.min(info_rect.width),
                1,
            ),
            sel_vid.clone(),
        ));
    }
}

#[allow(clippy::too_many_arguments)]
fn render_vertical_scrollbar(
    frame: &mut Frame,
    x: u16,
    y: u16,
    height: u16,
    scroll_idx: usize,
    visible: usize,
    total: usize,
    thumb_color: Color,
    track_color: Color,
) {
    if height == 0 || total <= visible || total == 0 {
        return;
    }
    let h = height as usize;
    let thumb_len = ((visible * h) / total).clamp(1, h);
    let max_scroll = total.saturating_sub(visible).max(1);
    let thumb_start = ((scroll_idx * (h.saturating_sub(thumb_len))) / max_scroll).min(h - thumb_len);

    for dy in 0..h {
        let is_thumb = dy >= thumb_start && dy < thumb_start + thumb_len;
        let glyph = if is_thumb { "┃" } else { "│" };
        let col = if is_thumb { thumb_color } else { track_color };
        frame.render_widget(
            Paragraph::new(Span::styled(glyph, Style::default().fg(col))),
            Rect::new(x, y + dy as u16, 1, 1),
        );
    }
}

fn render_video_grid(
    frame: &mut Frame,
    area: Rect,
    state: &mut AppState,
    modal_rect: Option<Rect>,
) {
    let base_theme = state.base_theme();
    let videos = state.current_filtered_videos();
    let m = compute_grid_metrics_for_size(area, state.config.grid_card_size);
    let cols = m.cols;
    let visible_rows = m.visible_rows;
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
        let (card_x, card_w) = m.col_x_and_width(c);
        let (card_y, card_h) = m.row_y_and_height(r);
        let bottom_limit = m.status_row_y.unwrap_or(area.y + area.height);
        if card_y + card_h > bottom_limit || card_x + card_w > area.x + area.width {
            continue;
        }
        let card_rect = Rect::new(card_x, card_y, card_w, card_h);
        let v = &videos[idx];
        let focused = idx == cursor;
        let card_pal = if focused {
            state.active_palette_for(Some(&v.id))
        } else {
            *base_theme
        };

        let card_inner = card_rect;

        let meta_h: u16 = if card_inner.height >= 14 {
            4
        } else if card_inner.height >= 10 {
            3
        } else {
            2
        };
        let thumb_h = card_inner.height.saturating_sub(meta_h).max(4);
        let card_chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(thumb_h), Constraint::Min(meta_h)])
            .split(card_inner);

        let vid_id = v.id.clone();
        if !occluded_by_modal(card_chunks[0], modal_rect) && state.overlay == ActiveOverlay::None {
            render_thumbnail_cover(frame, card_chunks[0], &vid_id, state, "...");
        }

        let focus_bar_w = if focused { 2 } else { 0 };
        let now_unix = now_unix_home();
        let inner_w = (card_chunks[1].width as usize).saturating_sub(focus_bar_w);
        let is_new = v.is_new_release(now_unix);
        let inline_new = meta_h == 2 && is_new && inner_w >= 18;
        let inline_new_w = if inline_new { 4 } else { 0 }; // "NEW "
        let wrapped_title = crate::ui::detail::wrap_words(&v.title, inner_w.max(8));
        let single_title = if focused {
            marquee_window(&v.title, inner_w, state.ticker_step)
        } else {
            v.title.chars().take(inner_w).collect()
        };

        let dur_badge = v.display_duration();
        let is_live_card = v.is_live == Some(true) || dur_badge == "LIVE";
        let uploader_w = inner_w
            .saturating_sub(inline_new_w + dur_badge.chars().count() + 2)
            .max(6);
        let uploader_str: String = format!("{} >", v.display_uploader())
            .chars()
            .take(uploader_w)
            .collect();
        let uploader_actual_w = uploader_str.chars().count();
        let pad_between = inner_w
            .saturating_sub(inline_new_w + uploader_actual_w + dur_badge.chars().count())
            .max(1);

        let (status_badge, status_color) = if let Some((pos, dur)) = state.progress_map.get(&v.id) {
            if *dur > 0.0 && *pos > 1.0 {
                (format!("{}%", ((pos / dur) * 100.0).round() as u32), card_pal.accent)
            } else if state.is_in_continue_watching(&v.id) {
                ("[QUEUE]".to_string(), Color::Cyan)
            } else {
                (String::new(), card_pal.text_dim)
            }
        } else if state.is_in_continue_watching(&v.id) {
            ("[QUEUE]".to_string(), Color::Cyan)
        } else if state.offline_paths.contains_key(&v.id) {
            ("[OFF]".to_string(), card_pal.primary)
        } else if state.watch_later_sources.contains_key(&v.id) {
            ("[WL]".to_string(), Color::Rgb(251, 191, 36))
        } else if state.watched_ids.contains(&v.id) {
            ("[SEEN]".to_string(), card_pal.text_dim)
        } else {
            (String::new(), card_pal.text_dim)
        };

        let meta_bg = if focused {
            Color::Rgb(24, 28, 42)
        } else {
            Color::Reset
        };
        let focus_prefix = |line_spans: Vec<Span<'static>>| -> Line<'static> {
            if focused {
                let mut out = vec![Span::styled(
                    "▌ ",
                    Style::default()
                        .fg(card_pal.primary)
                        .bg(meta_bg)
                        .add_modifier(Modifier::BOLD),
                )];
                out.extend(line_spans);
                Line::from(out)
            } else {
                Line::from(line_spans)
            }
        };

        let mut meta_lines = if meta_h >= 4 {
            let line0 = wrapped_title.first().cloned().unwrap_or_default();
            let line1 = if wrapped_title.len() > 2 {
                if focused {
                    marquee_window(&wrapped_title[1..].join(" "), inner_w, state.ticker_step)
                } else {
                    crate::ui::detail::truncate_word_boundary(
                        &wrapped_title[1..].join(" "),
                        inner_w.max(8),
                    )
                }
            } else {
                wrapped_title.get(1).cloned().unwrap_or_default()
            };
            vec![
                focus_prefix(vec![Span::styled(
                    line0,
                    Style::default()
                        .fg(if focused { card_pal.primary } else { card_pal.text })
                        .bg(meta_bg)
                        .add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() }),
                )]),
                focus_prefix(vec![Span::styled(
                    line1,
                    Style::default()
                        .fg(if focused { card_pal.primary } else { card_pal.text })
                        .bg(meta_bg),
                )]),
            ]
        } else {
            vec![focus_prefix(vec![Span::styled(
                single_title,
                Style::default()
                    .fg(if focused { card_pal.primary } else { card_pal.text })
                    .bg(meta_bg)
                    .add_modifier(if focused { Modifier::BOLD } else { Modifier::empty() }),
            )])]
        };

        let uploader_row_offset: u16 = if meta_h >= 4 { 2 } else { 1 };
        let mut uploader_spans = Vec::new();
        if inline_new {
            uploader_spans.push(Span::styled(
                "NEW",
                Style::default()
                    .fg(Color::Black)
                    .bg(Color::Rgb(52, 211, 153))
                    .add_modifier(Modifier::BOLD),
            ));
            uploader_spans.push(Span::styled(" ", Style::default().bg(meta_bg)));
        }
        uploader_spans.push(Span::styled(
            uploader_str,
            Style::default()
                .fg(card_pal.secondary)
                .bg(meta_bg)
                .add_modifier(Modifier::UNDERLINED),
        ));
        uploader_spans.push(Span::styled(" ".repeat(pad_between), Style::default().bg(meta_bg)));
        uploader_spans.push(Span::styled(
            dur_badge.clone(),
            Style::default()
                .fg(if is_live_card {
                    Color::Rgb(248, 113, 113)
                } else {
                    card_pal.accent
                })
                .bg(meta_bg)
                .add_modifier(if is_live_card {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        ));
        meta_lines.push(focus_prefix(uploader_spans));

        if meta_h >= 3 {
            let new_prefix_len = if is_new { 6 } else { 0 }; // "[NEW] "
            let right_len = status_badge.chars().count();
            let left_budget = inner_w.saturating_sub(new_prefix_len + right_len + if right_len > 0 { 1 } else { 0 });

            let mut stats_text = match (
                v.compact_views(left_budget >= 20),
                v.compact_age(now_unix, left_budget >= 18),
            ) {
                (Some(vw), Some(ag)) => format!("{vw} · {ag}"),
                (Some(vw), None) => vw,
                (None, Some(ag)) => ag,
                (None, None) => String::new(),
            };
            if stats_text.chars().count() > left_budget {
                // Retry with ultra-compact format (`128K · 3h`) before clipping
                stats_text = match (v.compact_views(false), v.compact_age(now_unix, false)) {
                    (Some(vw), Some(ag)) => format!("{vw} · {ag}"),
                    (Some(vw), None) => vw,
                    (None, Some(ag)) => ag,
                    (None, None) => String::new(),
                };
            }
            let stats_trunc: String = stats_text.chars().take(left_budget).collect();
            let stats_len = stats_trunc.chars().count();
            let pad_w = inner_w.saturating_sub(new_prefix_len + stats_len + right_len);

            let mut row3 = Vec::new();
            if is_new {
                row3.push(Span::styled(
                    "[NEW]",
                    Style::default()
                        .fg(Color::Black)
                        .bg(Color::Rgb(52, 211, 153))
                        .add_modifier(Modifier::BOLD),
                ));
                row3.push(Span::styled(" ", Style::default().bg(meta_bg)));
            }
            row3.push(Span::styled(
                stats_trunc,
                Style::default().fg(card_pal.text_dim).bg(meta_bg),
            ));
            if pad_w > 0 && right_len > 0 {
                row3.push(Span::styled(" ".repeat(pad_w), Style::default().bg(meta_bg)));
            }
            if right_len > 0 {
                row3.push(Span::styled(
                    status_badge,
                    Style::default()
                        .fg(status_color)
                        .bg(meta_bg)
                        .add_modifier(Modifier::BOLD),
                ));
            }
            meta_lines.push(focus_prefix(row3));
        }

        frame.render_widget(
            Paragraph::new(meta_lines).style(Style::default().bg(meta_bg)),
            card_chunks[1],
        );
        if card_chunks[1].height > uploader_row_offset && uploader_actual_w > 0 {
            let hitbox_x = card_chunks[1].x + (focus_bar_w + inline_new_w) as u16;
            state.home_channel_hitboxes.push((
                Rect::new(
                    hitbox_x,
                    card_chunks[1].y + uploader_row_offset,
                    (uploader_actual_w as u16)
                        .min(card_chunks[1].width.saturating_sub((focus_bar_w + inline_new_w) as u16)),
                    1,
                ),
                v.clone(),
            ));
        }
    }

    let total_rows = videos.len().div_ceil(cols);
    if let Some(sb_x) = m.scrollbar_x {
        let sb_h = m
            .status_row_y
            .map(|sy| sy.saturating_sub(area.y))
            .unwrap_or(area.height);
        render_vertical_scrollbar(
            frame,
            sb_x,
            area.y,
            sb_h,
            scroll_row,
            visible_rows,
            total_rows,
            base_theme.primary,
            base_theme.divider,
        );
    }

    if let Some(sy) = m.status_row_y {
        let active_cat = state.categories[state.active_category].0;
        let feed_key = state.active_feed_key_for(active_cat);
        let has_more = active_cat != Category::ContinueWatching
            && (!state.exhausted_feeds.contains(feed_key)
                || state.feed_continuations.contains_key(feed_key));
        let status_line = if state.is_fetching_more {
            Line::from(vec![
                Span::styled(
                    " [Loading more videos from YouTube...] ",
                    Style::default()
                        .fg(base_theme.accent)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" ({} loaded so far)", videos.len()),
                    Style::default().fg(base_theme.text_dim),
                ),
            ])
        } else if active_cat == Category::ContinueWatching {
            Line::from(vec![
                Span::styled(
                    format!(" Continue Watching Grid ({} queued) ", videos.len()),
                    Style::default().fg(base_theme.primary),
                ),
                Span::styled(
                    "· [v] Queue Deck  ·  [x/d] Remove  ·  [Enter/Space] Play",
                    Style::default().fg(base_theme.text_dim),
                ),
            ])
        } else {
            let tail = if has_more {
                "Scroll down (j/Down/PgDn) to auto-load more"
            } else {
                "All shelf videos loaded"
            };
            Line::from(vec![
                Span::styled(
                    format!(" Row {}/{}  ·  {} videos ", cursor_row + 1, total_rows.max(1), videos.len()),
                    Style::default().fg(base_theme.secondary),
                ),
                Span::styled(
                    format!("· {tail}  ·  [a] +Continue Watching"),
                    Style::default().fg(base_theme.text_dim),
                ),
            ])
        };
        frame.render_widget(
            Paragraph::new(status_line),
            Rect::new(area.x + 1, sy, area.width.saturating_sub(2), 1),
        );
    }
}

pub fn render_filter_modal(frame: &mut Frame, state: &AppState) {
    let theme = state.base_theme();
    let area = centered_rect(64, 66, frame.area());
    frame.render_widget(Clear, area);

    let blk = Block::default()
        .title(Span::styled(
            " FILTER & SORT WORKSPACE ",
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

    let shorts_sources = [ShortsSource::Suggested, ShortsSource::Subscribed];
    let shorts_spans: Vec<Span> = shorts_sources
        .iter()
        .enumerate()
        .map(|(i, ss)| {
            let active = *ss == state.shorts_source;
            let cursor = state.filter_modal_section == 2 && state.filter_modal_option == i;
            let st = if cursor {
                Style::default().fg(Color::Black).bg(theme.accent).add_modifier(Modifier::BOLD)
            } else if active {
                Style::default().fg(theme.accent).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text)
            };
            Span::styled(format!(" [{}] ", ss.label()), st)
        })
        .collect();
    frame.render_widget(
        Paragraph::new(Line::from(shorts_spans)).block(
            Block::default()
                .title(" Shorts Feed Source ")
                .borders(Borders::ALL)
                .border_style(Style::default().fg(if state.filter_modal_section == 2 {
                    theme.border_focus
                } else {
                    theme.border_idle
                })),
        ),
        chunks[2],
    );

    frame.render_widget(
        Paragraph::new("[Up/Down] Switch Row · [Left/Right] Cycle · [Enter / Esc] Apply & Close")
            .alignment(Alignment::Center)
            .style(Style::default().fg(theme.text_dim)),
        chunks[3],
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
        assert_eq!(footer.height, 0);
        assert_eq!(grid_card_index_at(grid, grid.x + 3, grid.y + 2, 0, 1), Some(0));
        assert_eq!(footer_action_at(footer, 2, footer.y), None);
        assert!(top_bar_shelf_at(&st.categories, 16).is_some());

        let backend = TestBackend::new(120, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_home_screen(f, &mut st, None)).unwrap();
        assert!(!st.home_channel_hitboxes.is_empty());
    }

    #[test]
    fn test_compute_grid_metrics_fills_width_and_height_cleanly() {
        for mode in [GridCardSize::CinemaLarge, GridCardSize::AdaptiveLarge] {
            for (w, h) in [(100u16, 30u16), (152, 34), (175, 46), (240, 65)] {
                let [_, _, grid, _] = home_layout(Rect::new(0, 0, w, h));
                let m = compute_grid_metrics_for_size(grid, mode);
                assert!(m.cols >= 2 && m.cols <= 5);
                assert!(m.visible_rows >= 1);
                if mode == GridCardSize::CinemaLarge {
                    if h <= 34 {
                        assert_eq!(m.visible_rows, 2, "expected 2 YouTube-sized rows at {w}x{h}");
                        let min_h = if h >= 34 { 12 } else { 10 };
                        assert!(m.card_height >= min_h, "expected tall 16:9 cards at {w}x{h}: {}", m.card_height);
                    } else if h >= 46 {
                        assert_eq!(m.visible_rows, 3, "expected 3 rows on tall monitor at {w}x{h}");
                    }
                }
                if mode == GridCardSize::AdaptiveLarge && h >= 34 {
                    assert_eq!(m.visible_rows, 3, "expected 3 dense rows at {w}x{h}");
                }

                let (last_x, last_w) = m.col_x_and_width(m.cols - 1);
                let right_edge = last_x + last_w;
                if let Some(sb_x) = m.scrollbar_x {
                    assert!(right_edge < sb_x);
                    assert!(sb_x - right_edge <= (m.cols as u16) + 2);
                }

                let (last_y, last_h) = m.row_y_and_height(m.visible_rows - 1);
                let bottom_edge = last_y + last_h;
                let status_y = m.status_row_y.unwrap_or(grid.y + grid.height);
                assert!(bottom_edge <= status_y);
                assert!(
                    status_y - bottom_edge <= 4,
                    "too much dead space at {w}x{h} ({mode:?}): {}",
                    status_y - bottom_edge
                );
            }
        }
    }

    #[test]
    fn test_continue_watching_deck_renders_and_hit_tests() {
        let mut st = AppState::new(Config::default());
        st.active_category = 0; // ContinueWatching
        st.categories[0].1.push(VideoItem {
            id: "cw_vid_0001".to_string(),
            title: "Systems Programming Architecture Deep Dive".to_string(),
            uploader: Some("KernelCraft".to_string()),
            duration: Some(1800),
            ..Default::default()
        });

        let area = Rect::new(0, 0, 140, 42);
        let [_, _, grid, _] = home_layout(area);
        assert_eq!(continue_watching_row_at(grid, grid.x + 6, grid.y + 2, 0, 1), Some(0));

        let backend = TestBackend::new(140, 42);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_home_screen(f, &mut st, None)).unwrap();
        assert!(st.home_channel_hitboxes.len() >= 2);
    }

    #[test]
    fn test_marquee_window_and_card_telemetry() {
        assert_eq!(marquee_window("Short Title", 20, 4), "Short Title");
        let title = "Trump, Xi and Putin plan summit in Beijing";
        assert_eq!(marquee_window(title, 16, 0), "Trump, Xi and Pu");
        let scrolled = marquee_window(title, 16, 11);
        assert_eq!(scrolled.chars().count(), 16);
        assert!(scrolled.starts_with("nd Putin plan su"));

        let cjk = "日本語タイトルのテスト動画再生";
        let cjk_win = marquee_window(cjk, 8, 3);
        assert_eq!(cjk_win.chars().count(), 8);

        let mut st = AppState::new(Config::default());
        st.ticker_step = 4;
        st.categories[1].1.push(VideoItem {
            id: "newvid001".to_string(),
            title: title.to_string(),
            uploader: Some("Channel 4 News".to_string()),
            duration: Some(443),
            view_count: Some(128_000),
            published_text: Some("3 hours ago".to_string()),
            timestamp: Some(now_unix_home() - 3600 * 3),
            ..Default::default()
        });
        let backend = TestBackend::new(140, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_home_screen(f, &mut st, None)).unwrap();

        let buf = term.backend().buffer();
        let all_text: String = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(all_text.contains("[NEW]"), "Expected [NEW] badge in card telemetry");
        assert!(all_text.contains("128K"), "Expected view count in card telemetry");
    }

    #[test]
    fn test_shorts_shelf_between_home_and_subscriptions_and_filter_bar() {
        let mut st = AppState::new(Config::default());
        assert_eq!(st.categories[1].0, Category::Recommended);
        assert_eq!(st.categories[2].0, Category::Shorts);
        assert_eq!(st.categories[3].0, Category::Subscriptions);

        st.active_category = 2;
        st.shorts_suggested = vec![VideoItem {
            id: "sh_sug_01".to_string(),
            title: "Quick Physics Fact".to_string(),
            uploader: Some("AlgoCreator".to_string()),
            duration: Some(45),
            duration_string: Some("SHORT".to_string()),
            view_count: Some(950_000),
            published_text: Some("5 hours ago".to_string()),
            timestamp: Some(now_unix_home() - 3600 * 5),
            ..Default::default()
        }];
        st.shorts_subscribed = vec![VideoItem {
            id: "sh_sub_01".to_string(),
            title: "Subscribed Short".to_string(),
            uploader: Some("Veritasium".to_string()),
            duration: Some(58),
            duration_string: Some("SHORT".to_string()),
            ..Default::default()
        }];
        st.sync_active_shorts_list();

        let area = Rect::new(0, 0, 140, 40);
        let [_, _, grid_area, _] = home_layout(area);
        assert_eq!(
            shorts_source_at(grid_area, 1, 1, grid_area.x + 20, grid_area.y),
            Some(ShortsSource::Suggested)
        );
        let eff = effective_grid_area(grid_area, Category::Shorts);
        assert_eq!(eff.y, grid_area.y + 1);

        let backend = TestBackend::new(140, 40);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render_home_screen(f, &mut st, None)).unwrap();

        let buf = term.backend().buffer();
        let all_text: String = buf
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect::<Vec<_>>()
            .join("");
        assert!(all_text.contains("Algorithm Suggested (1)"));
        assert!(all_text.contains("Subscribed (1)"));
        assert!(all_text.contains("Quick Physics Fact"));
    }
}
