//! Channel Drill-Down Screen (`Screen::Channel`) — Split-Pane Catalog + Live 16:9 Preview.
//! Launched from `[c]` inside Video Detail; pushes cleanly so `Esc` pops back.

use crate::state::{occluded_by_modal, ActiveOverlay, AppState};
use crate::ui::components::cover::{render_video_preview_inspector, unix_now_secs};
use crate::ui::home::marquee_window;
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub fn render_channel_screen(frame: &mut Frame, state: &mut AppState, modal_rect: Option<Rect>) {
    let theme = state.base_theme();
    let area = frame.area();
    let now_unix = unix_now_secs();

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(10),
            Constraint::Length(2),
        ])
        .split(area);

    let header_blk = Block::default()
        .title(Span::styled(
            format!(" CHANNEL UPLOADS — {} ", state.channel_title),
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.primary));

    let subtitle = if state.is_loading_channel {
        "Syncing latest uploads & Atom feed...".to_string()
    } else {
        format!(
            "{} videos indexed · ID: {} · Press [Space/p] to play in Reference 4K or [a] to add to Continue Watching",
            state.channel_videos.len(),
            state.channel_id.as_deref().unwrap_or("handle")
        )
    };
    frame.render_widget(
        Paragraph::new(Span::styled(subtitle, Style::default().fg(theme.accent))).block(header_blk),
        rows[0],
    );

    // 58% Left Catalog List / 42% Right Live 16:9 Preview Inspector
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(rows[1]);

    let list_blk = Block::default()
        .title(Span::styled(
            " UPLOADS CATALOG ",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let list_inner = list_blk.inner(panes[0]);
    frame.render_widget(list_blk, panes[0]);

    let selected_video = if state.channel_videos.is_empty() {
        frame.render_widget(
            Paragraph::new("Loading channel catalog...")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            list_inner,
        );
        None
    } else {
        let max_vis = list_inner.height.max(1) as usize;
        let cur = state
            .channel_cursor
            .min(state.channel_videos.len().saturating_sub(1));
        let start = if cur >= max_vis { cur + 1 - max_vis } else { 0 };

        let to_queue: Vec<(String, String)> = state
            .channel_videos
            .iter()
            .skip(start)
            .take(max_vis + 2)
            .map(|v| (v.id.clone(), v.best_thumbnail_url()))
            .collect();
        for (id, url) in to_queue {
            state.queue_thumb_if_needed(&id, &url);
        }

        let total_w = list_inner.width as usize;
        let dur_col_w = 10usize;
        let meta_col_w = if total_w >= 68 { 16 } else { 0 };
        let prefix_w = 8usize; // " ▸ [OK] "
        let title_w = total_w
            .saturating_sub(prefix_w + dur_col_w + meta_col_w + 2)
            .max(12);

        let mut lines = Vec::new();
        for (idx, v) in state
            .channel_videos
            .iter()
            .enumerate()
            .skip(start)
            .take(max_vis)
        {
            let sel = idx == cur;
            let bg = if sel { Color::Rgb(32, 38, 54) } else { Color::Reset };
            let badge = if state.watched_ids.contains(&v.id) {
                Span::styled("[OK] ", Style::default().fg(theme.accent).bg(bg))
            } else if v.is_new_release(now_unix) {
                Span::styled("NEW  ", Style::default().fg(Color::Rgb(52, 211, 153)).bg(bg).add_modifier(Modifier::BOLD))
            } else {
                Span::styled("  ·  ", Style::default().fg(theme.text_dim).bg(bg))
            };
            let title_text = if sel {
                marquee_window(&v.title, title_w, state.ticker_step)
            } else {
                v.title.chars().take(title_w).collect::<String>()
            };

            let mut row_spans = vec![
                Span::styled(
                    if sel { " ▸ " } else { "   " },
                    Style::default().fg(theme.primary).bg(bg),
                ),
                badge,
                Span::styled(
                    format!("{:<width$} ", title_text, width = title_w),
                    Style::default()
                        .fg(if sel { theme.primary } else { theme.text })
                        .bg(bg)
                        .add_modifier(if sel { Modifier::BOLD } else { Modifier::empty() }),
                ),
            ];
            if meta_col_w > 0 {
                let views_age = match (v.compact_views(false), v.compact_age(now_unix, false)) {
                    (Some(vw), Some(ag)) => format!("{vw} · {ag}"),
                    (Some(vw), None) => vw,
                    (None, Some(ag)) => ag,
                    (None, None) => String::new(),
                };
                let trimmed: String = views_age.chars().take(meta_col_w - 1).collect();
                row_spans.push(Span::styled(
                    format!("{:<width$} ", trimmed, width = meta_col_w - 1),
                    Style::default().fg(theme.text_dim).bg(bg),
                ));
            }
            row_spans.push(Span::styled(
                format!("[{}]", v.display_duration()),
                Style::default().fg(theme.accent).bg(bg),
            ));
            lines.push(Line::from(row_spans));
        }
        frame.render_widget(Paragraph::new(lines), list_inner);
        state.channel_videos.get(cur).cloned()
    };

    if let Some(sel_vid) = selected_video {
        if state.overlay == ActiveOverlay::None && !occluded_by_modal(panes[1], modal_rect) {
            let mut badges: Vec<(&str, Color)> = Vec::new();
            if state.watched_ids.contains(&sel_vid.id) {
                badges.push(("WATCHED", theme.accent));
            }
            if state.watch_later_sources.contains_key(&sel_vid.id) {
                badges.push(("WL", Color::Rgb(251, 191, 36)));
            }
            render_video_preview_inspector(
                frame,
                panes[1],
                state,
                &sel_vid,
                now_unix,
                &badges,
                None,
                "[Space/p] Play · [Enter] Detail · [a/+] +CW · [Esc] Back",
            );
        }
    } else {
        let empty_right = Block::default()
            .title(Span::styled(" LIVE PREVIEW ", Style::default().fg(theme.secondary)))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_idle));
        let empty_inner = empty_right.inner(panes[1]);
        frame.render_widget(empty_right, panes[1]);
        frame.render_widget(
            Paragraph::new("Select an upload to preview thumbnail & metadata.")
                .alignment(Alignment::Center)
                .style(Style::default().fg(theme.text_dim)),
            empty_inner,
        );
    }

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[j/k] Navigate ", Style::default().fg(theme.primary)),
            Span::styled("· [Enter] Open Detail ", Style::default().fg(theme.accent)),
            Span::styled("· [Space/p] Play Immediate ", Style::default().fg(theme.secondary)),
            Span::styled("· [a] +Continue Watching ", Style::default().fg(theme.primary)),
            Span::styled("· [Esc] Back to Detail", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        rows[2],
    );
}
