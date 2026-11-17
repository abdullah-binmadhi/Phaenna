//! Personal Viewing Intelligence & Self-Monitoring Dashboard (`[A] Analysis`).

use crate::db::user::{sort_channels_by, AnalysisRange, AnalysisTab, UserAnalytics};
use crate::state::{centered_rect, AnalysisHit, AppState};
use ratatui::{
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap},
    Frame,
};

fn truncate_words(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    if max_chars <= 3 {
        return trimmed.chars().take(max_chars).collect();
    }
    let limit = max_chars - 3;
    let mut byte_end = 0usize;
    let mut last_space = None;
    for (i, (b_idx, ch)) in trimmed.char_indices().enumerate() {
        if i >= limit {
            break;
        }
        byte_end = b_idx + ch.len_utf8();
        if ch.is_whitespace() {
            last_space = Some(b_idx);
        }
    }
    let cut = last_space.filter(|&p| p >= byte_end / 2).unwrap_or(byte_end);
    format!("{}...", trimmed[..cut].trim_end())
}

fn format_mins_compact(mins: f64) -> String {
    if mins >= 60.0 {
        let total = mins.round() as u64;
        format!("{}h {:02}m", total / 60, total % 60)
    } else {
        format!("{:.0}m", mins.max(0.0))
    }
}

fn format_clock_utc_approx(unix_ts: i64, tz_offset_secs: i64) -> String {
    if unix_ts <= 0 {
        return "Synced".to_string();
    }
    let local = unix_ts + tz_offset_secs;
    let day_secs = local.rem_euclid(86_400);
    let hh = day_secs / 3600;
    let mm = (day_secs % 3600) / 60;
    let days = local.div_euclid(86_400);
    // Civil date from unix day count (Howard Hinnant algorithm)
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    format!("{m:02}/{d:02} {hh:02}:{mm:02}")
}

fn render_bar_spans<'a>(
    frac: f64,
    width: usize,
    fill_color: Color,
    track_color: Color,
) -> Vec<Span<'a>> {
    let w = width.max(4);
    let filled = ((frac.clamp(0.0, 1.0) * w as f64).round() as usize).min(w);
    let empty = w.saturating_sub(filled);
    vec![
        Span::styled("█".repeat(filled), Style::default().fg(fill_color)),
        Span::styled("░".repeat(empty), Style::default().fg(track_color)),
    ]
}

pub fn render_analytics_modal(frame: &mut Frame, state: &mut AppState) {
    state.analysis_hitboxes.clear();

    let theme = state.base_theme();
    let area = centered_rect(94, 90, frame.area());
    frame.render_widget(Clear, area);

    let outer = Block::default()
        .title(Span::styled(
            " PHAENNA ANALYSIS — VIEWING TELEMETRY & HABITS ",
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

    if inner.width < 40 || inner.height < 16 {
        frame.render_widget(
            Paragraph::new("Terminal too small for Analysis dashboard (need >= 40x16)")
                .alignment(Alignment::Center),
            inner,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .horizontal_margin(1)
        .constraints([
            Constraint::Length(2), // Control strip (Ranges + Source + Tabs + Budget)
            Constraint::Min(10),   // Main tab canvas
            Constraint::Length(1), // Footer keybindings
        ])
        .split(inner);

    render_control_strip(frame, state, rows[0]);

    match state.analysis_tab {
        AnalysisTab::Overview => render_overview_tab(frame, state, rows[1]),
        AnalysisTab::Channels => render_channels_tab(frame, state, rows[1]),
        AnalysisTab::Habits => render_habits_tab(frame, state, rows[1]),
    }

    render_footer(frame, state, rows[2]);
}

fn render_control_strip(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let theme = state.base_theme();
    let mut top_spans: Vec<Span> = Vec::new();
    let mut x_cur = area.x;

    let push_pill = |spans: &mut Vec<Span>,
                     hitboxes: &mut Vec<(Rect, AnalysisHit)>,
                     x: &mut u16,
                     y: u16,
                     max_x: u16,
                     label: String,
                     style: Style,
                     hit: AnalysisHit| {
        let w = label.chars().count() as u16;
        if *x + w <= max_x {
            hitboxes.push((Rect::new(*x, y, w, 1), hit));
            spans.push(Span::styled(label, style));
            *x += w;
        }
    };

    let lbl = " Range: ";
    top_spans.push(Span::styled(lbl, Style::default().fg(theme.text_dim)));
    x_cur += lbl.len() as u16;

    for r in AnalysisRange::ALL {
        let active = state.analysis_range == r;
        let text = format!(" {} ", r.label());
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(theme.primary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        push_pill(
            &mut top_spans,
            &mut state.analysis_hitboxes,
            &mut x_cur,
            area.y,
            area.x + area.width,
            text,
            style,
            AnalysisHit::SelectRange(r),
        );
    }

    let gap = "  |  ";
    if x_cur + (gap.len() as u16) < area.x + area.width {
        top_spans.push(Span::styled(gap, Style::default().fg(theme.divider)));
        x_cur += gap.len() as u16;
    }

    let src_text = if state.analysis_include_yt_sync {
        " [o] Source: Live + YT History "
    } else {
        " [o] Source: Live Sessions "
    };
    let src_style = if state.analysis_include_yt_sync {
        Style::default()
            .fg(Color::Black)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(theme.secondary)
            .add_modifier(Modifier::BOLD)
    };
    push_pill(
        &mut top_spans,
        &mut state.analysis_hitboxes,
        &mut x_cur,
        area.y,
        area.x + area.width,
        src_text.to_string(),
        src_style,
        AnalysisHit::ToggleYtSync,
    );

    // Row 1 inside control strip: Tabs on left, Budget controls on right
    let mut tab_spans: Vec<Span> = Vec::new();
    let mut tx = area.x;
    tab_spans.push(Span::raw(" "));
    tx += 1;

    for tab in AnalysisTab::ALL {
        let active = state.analysis_tab == tab;
        let txt = format!(" [{}] ", tab.label());
        let style = if active {
            Style::default()
                .fg(Color::Black)
                .bg(theme.secondary)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        push_pill(
            &mut tab_spans,
            &mut state.analysis_hitboxes,
            &mut tx,
            area.y + 1,
            area.x + area.width,
            txt,
            style,
            AnalysisHit::SelectTab(tab),
        );
        if tx + 1 < area.x + area.width {
            tab_spans.push(Span::raw(" "));
            tx += 1;
        }
    }

    let budget_lbl = format!("  Budget: {}m/d ", state.config.analysis.daily_budget_mins);
    let dec_txt = "[-b]";
    let inc_txt = "[+B]";
    let right_need = (budget_lbl.len() + dec_txt.len() + inc_txt.len() + 2) as u16;
    if area.width > (tx - area.x) + right_need {
        let pad = area.width - (tx - area.x) - right_need;
        tab_spans.push(Span::raw(" ".repeat(pad as usize)));
        tx += pad;
        tab_spans.push(Span::styled(budget_lbl, Style::default().fg(theme.text_dim)));
        tx += (format!("  Budget: {}m/d ", state.config.analysis.daily_budget_mins)).len() as u16;
        push_pill(
            &mut tab_spans,
            &mut state.analysis_hitboxes,
            &mut tx,
            area.y + 1,
            area.x + area.width,
            dec_txt.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            AnalysisHit::BudgetDelta(-15),
        );
        tab_spans.push(Span::raw(" "));
        tx += 1;
        push_pill(
            &mut tab_spans,
            &mut state.analysis_hitboxes,
            &mut tx,
            area.y + 1,
            area.x + area.width,
            inc_txt.to_string(),
            Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
            AnalysisHit::BudgetDelta(15),
        );
    }

    frame.render_widget(Paragraph::new(vec![Line::from(top_spans), Line::from(tab_spans)]), area);
}

fn render_hero_strip(frame: &mut Frame, state: &AppState, area: Rect) {
    let theme = state.base_theme();
    let a = &state.analytics;
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
            Constraint::Percentage(25),
        ])
        .split(area);

    let delta_str = match a.period_delta_pct {
        Some(d) if d >= 0.0 => format!("+{:.0}% vs prev", d),
        Some(d) => format!("{:.0}% vs prev", d),
        None => format!("Avg {}/d", format_mins_compact(a.avg_daily_mins)),
    };
    let budget_ratio = a.today_watch_mins / (a.daily_budget_mins.max(1) as f64);
    let budget_col = if budget_ratio > 1.05 {
        Color::Rgb(248, 113, 113)
    } else if budget_ratio >= 0.85 {
        Color::Rgb(251, 191, 36)
    } else {
        theme.primary
    };

    let total_saved_mins = (a.sponsorblock_saved_secs / 60.0) + a.speed_saved_mins;

    let cards = [
        (
            " WATCH TIME & PACE ",
            format!("{:.1}h · {} vids", a.total_hours_watched, a.total_videos_watched),
            format!(
                "Today {}/{}m · {}",
                format_mins_compact(a.today_watch_mins),
                a.daily_budget_mins,
                delta_str
            ),
            budget_col,
        ),
        (
            " FINISH & FOCUS ",
            format!("{:.0}% Finish Rate", a.completion_rate),
            format!(
                "{} done · {} part · {} bnc",
                a.completed_videos, a.partial_videos, a.bounced_videos
            ),
            theme.secondary,
        ),
        (
            " TIME RECLAIMED ",
            format!("{} Saved", format_mins_compact(total_saved_mins)),
            format!(
                "{:.1}m SB · {:.0}m @ {:.2}x",
                a.sponsorblock_saved_secs / 60.0,
                a.speed_saved_mins,
                a.weighted_avg_speed
            ),
            theme.accent,
        ),
        (
            " STREAK & LIBRARY ",
            format!("{}d active (best {}d)", a.active_streak_days, a.longest_streak_days),
            format!("{} CW · {} WL", a.in_progress_videos, a.watch_later_count),
            theme.primary,
        ),
    ];

    for (idx, (title, main, sub, col)) in cards.iter().enumerate() {
        let block = Block::default()
            .title(Span::styled(
                *title,
                Style::default().fg(*col).add_modifier(Modifier::BOLD),
            ))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(theme.border_idle));
        frame.render_widget(
            Paragraph::new(vec![
                Line::from(Span::styled(
                    main.clone(),
                    Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(sub.clone(), Style::default().fg(theme.text_dim))),
            ])
            .block(block),
            cols[idx],
        );
    }
}

fn render_overview_tab(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let theme = state.base_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4), // Hero Bento Cards
            Constraint::Min(8),    // Formats & Rolling Pace
            Constraint::Min(7),    // Top Channels & Insights
        ])
        .split(area);

    render_hero_strip(frame, state, sections[0]);

    // Middle Row
    let mid_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .split(sections[1]);

    render_format_allocation_panel(frame, state, mid_cols[0]);
    render_daily_pace_panel(frame, &state.analytics, theme, mid_cols[1]);

    // Bottom Row
    let bot_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(sections[2]);

    render_top_channels_compact(frame, state, bot_cols[0]);
    render_insights_panel(frame, &state.analytics, theme, bot_cols[1]);
}

fn render_format_allocation_panel(frame: &mut Frame, state: &AppState, area: Rect) {
    let theme = state.base_theme();
    let a = &state.analytics;
    let blk = Block::default()
        .title(Span::styled(
            " Format Allocation (Actual Hours & Finish Rate) ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));

    let inner_w = blk.inner(area).width as usize;
    let bar_w = inner_w.saturating_sub(42).clamp(8, 22);
    let max_share = a
        .format_buckets
        .iter()
        .map(|b| b.share_pct)
        .fold(1.0f64, f64::max);
    let colors = [
        theme.accent,
        theme.secondary,
        theme.primary,
        Color::Rgb(251, 191, 36),
    ];

    let mut lines: Vec<Line> = Vec::new();
    for (i, b) in a.format_buckets.iter().enumerate() {
        let mut spans = vec![Span::styled(
            format!(" {:<11} ", b.label),
            Style::default().fg(theme.text),
        )];
        spans.extend(render_bar_spans(
            b.share_pct / max_share,
            bar_w,
            colors[i % colors.len()],
            Color::Rgb(42, 47, 64),
        ));
        spans.push(Span::styled(
            format!(
                " {:>3}v {:>4.1}h {:>2.0}%  fin:{:>2.0}%",
                b.video_count, b.actual_hours, b.share_pct, b.finish_rate_pct
            ),
            Style::default().fg(theme.text_dim),
        ));
        lines.push(Line::from(spans));
    }

    lines.push(Line::from(vec![
        Span::styled(" Deep-Dive Share (20m+): ", Style::default().fg(theme.text_dim)),
        Span::styled(
            format!("{:.0}%", a.deep_dive_ratio_pct),
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  ·  Avg Session: {:.0}m", a.avg_session_mins),
            Style::default().fg(theme.text_dim),
        ),
    ]));

    if a.has_yt_backlog_only && !a.include_yt_sync {
        lines.push(Line::from(Span::styled(
            " No live sessions logged yet — press [o] to include synced YT history.",
            Style::default().fg(Color::Rgb(251, 191, 36)),
        )));
    }

    frame.render_widget(Paragraph::new(lines).block(blk), area);
}

fn render_daily_pace_panel(
    frame: &mut Frame,
    a: &UserAnalytics,
    theme: &crate::ui::theme::ThemePalette,
    area: Rect,
) {
    let blk = Block::default()
        .title(Span::styled(
            format!(" 7-Day Daily Watch Pace (Goal: {}m/d) ", a.daily_budget_mins),
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    if inner.height < 4 || inner.width < 24 {
        return;
    }

    let chart_rows = (inner.height as usize).saturating_sub(2).clamp(2, 5);
    let max_m = a
        .daily_pace
        .iter()
        .map(|p| p.watch_mins)
        .fold(a.daily_budget_mins as f64, f64::max)
        .max(15.0);
    let col_w = ((inner.width.saturating_sub(8)) as usize / a.daily_pace.len().max(1)).clamp(3, 7);

    let mut lines: Vec<Line> = Vec::new();
    for r in (0..chart_rows).rev() {
        let tick_val = ((r + 1) as f64 / chart_rows as f64) * max_m;
        let mut spans = vec![Span::styled(
            format!("{:>4.0}m |", tick_val),
            Style::default().fg(theme.text_dim),
        )];
        for pt in &a.daily_pace {
            let ratio = (pt.watch_mins / max_m).clamp(0.0, 1.0) * (chart_rows as f64);
            let cell_fill = ratio - (r as f64);
            let glyph = if cell_fill >= 0.85 {
                "█"
            } else if cell_fill >= 0.45 {
                "▄"
            } else if cell_fill > 0.05 {
                "▁"
            } else {
                " "
            };
            let color = if pt.watch_mins > a.daily_budget_mins as f64 {
                Color::Rgb(248, 113, 113)
            } else if pt.is_today {
                theme.primary
            } else {
                theme.secondary
            };
            let pad_l = (col_w.saturating_sub(1)) / 2;
            let pad_r = col_w.saturating_sub(1 + pad_l);
            spans.push(Span::styled(
                format!("{}{}{}", " ".repeat(pad_l), glyph, " ".repeat(pad_r)),
                Style::default().fg(color),
            ));
        }
        lines.push(Line::from(spans));
    }

    let mut axis = vec![Span::styled("   0m +", Style::default().fg(theme.text_dim))];
    for pt in &a.daily_pace {
        let lbl = pt.day_label.chars().take(3).collect::<String>();
        let pad_l = (col_w.saturating_sub(lbl.len())) / 2;
        let pad_r = col_w.saturating_sub(lbl.len() + pad_l);
        let style = if pt.is_today {
            Style::default().fg(theme.primary).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(theme.text_dim)
        };
        axis.push(Span::styled(
            format!("{}{}{}", " ".repeat(pad_l), lbl, " ".repeat(pad_r)),
            style,
        ));
    }
    lines.push(Line::from(axis));

    let peak_day = a
        .daily_pace
        .iter()
        .max_by(|x, y| x.watch_mins.total_cmp(&y.watch_mins))
        .map(|p| format!("{} ({:.0}m)", p.day_label, p.watch_mins))
        .unwrap_or_else(|| "None".into());
    lines.push(Line::from(Span::styled(
        format!(
            " Avg: {:.0}m/d · Peak: {} · Within budget: {}/{}d",
            a.avg_daily_mins, peak_day, a.budget_hit_days, a.budget_total_days
        ),
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_top_channels_compact(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let theme = state.base_theme();
    let a = &state.analytics;
    let blk = Block::default()
        .title(Span::styled(
            " Top Channels (Enter -> Open Channel) ",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    let sorted = sort_channels_by(&a.top_channels, state.analysis_channel_sort);
    if sorted.is_empty() {
        frame.render_widget(
            Paragraph::new("No ranked creator telemetry in this window yet.")
                .style(Style::default().fg(theme.text_dim)),
            inner,
        );
        return;
    }

    let max_rows = inner.height.saturating_sub(1) as usize;
    let cursor = state.analysis_cursor.min(sorted.len().saturating_sub(1));
    let name_w = (inner.width as usize).saturating_sub(32).clamp(12, 28);
    let mut lines: Vec<Line> = Vec::new();

    for (idx, c) in sorted.iter().take(max_rows).enumerate() {
        let selected = idx == cursor;
        let row_y = inner.y + idx as u16;
        state.analysis_hitboxes.push((
            Rect::new(inner.x, row_y, inner.width, 1),
            AnalysisHit::SelectChannel(idx),
        ));
        let marker = if selected { "> " } else { "  " };
        let bg = if selected {
            Color::Rgb(34, 41, 60)
        } else {
            Color::Reset
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}{:>2}. ", marker, idx + 1),
                Style::default().fg(theme.accent).bg(bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<w$} ", truncate_words(&c.channel_name, name_w), w = name_w),
                Style::default().fg(theme.text).bg(bg),
            ),
            Span::styled(
                format!(
                    "{:>2}v {:>4.1}h ({:>2.0}%) {:>2.0}%",
                    c.video_count, c.total_hours, c.share_pct, c.finish_rate_pct
                ),
                Style::default().fg(theme.text_dim).bg(bg),
            ),
        ]));
    }

    let footer_note = if a.unresolved_channel_videos > 0 {
        format!(
            " Showing {} of {} creators (+{} unresolved)",
            sorted.len().min(max_rows),
            a.unique_channels_count,
            a.unresolved_channel_videos
        )
    } else {
        format!(
            " Showing {} of {} creators · Top-3 share: {:.0}%",
            sorted.len().min(max_rows),
            a.unique_channels_count,
            a.top3_concentration_pct
        )
    };
    lines.push(Line::from(Span::styled(
        footer_note,
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_insights_panel(
    frame: &mut Frame,
    a: &UserAnalytics,
    theme: &crate::ui::theme::ThemePalette,
    area: Rect,
) {
    let blk = Block::default()
        .title(Span::styled(
            " Analyst Digest & Vault ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));

    let mut lines: Vec<Line> = Vec::new();
    if a.insights.is_empty() {
        lines.push(Line::from(Span::styled(
            " Watch a few videos across creators to unlock concentration and retention insights.",
            Style::default().fg(theme.text_dim),
        )));
    } else {
        for insight in a.insights.iter().take(4) {
            lines.push(Line::from(vec![
                Span::styled(" - ", Style::default().fg(theme.primary)),
                Span::styled(insight.clone(), Style::default().fg(theme.text)),
            ]));
        }
    }

    let mb = a.offline_cached_bytes as f64 / 1_048_576.0;
    lines.push(Line::from(Span::styled(
        format!(
            " Peak Window: {} · Vault: {} files ({:.1} MB)",
            a.peak_window_label, a.offline_cached_videos, mb
        ),
        Style::default().fg(theme.text_dim),
    )));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }).block(blk), area);
}

fn render_channels_tab(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let theme = state.base_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(10)])
        .split(area);

    // Subheader: Sort pills & concentration telemetry
    let mut sub_spans: Vec<Span> = Vec::new();
    let mut sx = sections[0].x;
    let sort_lbl = format!(" [s] Sort: {} ", state.analysis_channel_sort.label());
    let sw = sort_lbl.len() as u16;
    state.analysis_hitboxes.push((
        Rect::new(sx, sections[0].y, sw, 1),
        AnalysisHit::CycleChannelSort,
    ));
    sub_spans.push(Span::styled(
        sort_lbl,
        Style::default()
            .fg(Color::Black)
            .bg(theme.accent)
            .add_modifier(Modifier::BOLD),
    ));
    sx += sw;
    let _ = sx;
    sub_spans.push(Span::styled(
        format!(
            "   Unique Creators: {}  |  Top-3 Share: {:.0}%  |  Unresolved Stubs: {}",
            state.analytics.unique_channels_count,
            state.analytics.top3_concentration_pct,
            state.analytics.unresolved_channel_videos
        ),
        Style::default().fg(theme.text_dim),
    ));
    frame.render_widget(Paragraph::new(Line::from(sub_spans)), sections[0]);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(sections[1]);

    // Left: Full Creator Leaderboard
    let blk = Block::default()
        .title(Span::styled(
            " Creator Leaderboard (j/k Move · Enter Open Channel) ",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let inner = blk.inner(cols[0]);
    frame.render_widget(blk, cols[0]);

    let sorted = sort_channels_by(&state.analytics.top_channels, state.analysis_channel_sort);
    let name_w = (inner.width as usize).saturating_sub(34).clamp(12, 34);
    let mut lines: Vec<Line> = vec![Line::from(Span::styled(
        format!(
            "  #  {:<w$} {:>4} {:>6} {:>6} {:>6}",
            "Creator",
            "Vids",
            "Hours",
            "Share",
            "Fin%",
            w = name_w
        ),
        Style::default()
            .fg(theme.text_dim)
            .add_modifier(Modifier::BOLD),
    ))];
    let max_rows = (inner.height as usize).saturating_sub(1);
    let cursor = state.analysis_cursor.min(sorted.len().saturating_sub(1));

    for (idx, c) in sorted.iter().take(max_rows).enumerate() {
        let selected = idx == cursor;
        let row_y = inner.y + 1 + idx as u16;
        state.analysis_hitboxes.push((
            Rect::new(inner.x, row_y, inner.width, 1),
            AnalysisHit::SelectChannel(idx),
        ));
        let bg = if selected {
            Color::Rgb(34, 41, 60)
        } else {
            Color::Reset
        };
        lines.push(Line::from(vec![
            Span::styled(
                format!("{}{:>2}. ", if selected { ">" } else { " " }, idx + 1),
                Style::default().fg(theme.accent).bg(bg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<w$} ", truncate_words(&c.channel_name, name_w), w = name_w),
                Style::default().fg(theme.text).bg(bg),
            ),
            Span::styled(
                format!(
                    "{:>3}v {:>5.1}h {:>5.1}% {:>4.0}%",
                    c.video_count, c.total_hours, c.share_pct, c.finish_rate_pct
                ),
                Style::default().fg(theme.text_dim).bg(bg),
            ),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);

    // Right: SponsorBlock Category Breakdown + Efficiency Matrix
    let right_split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(cols[1]);

    let sb_blk = Block::default()
        .title(Span::styled(
            " SponsorBlock Breakdown ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let sb_w = sb_blk.inner(right_split[0]).width as usize;
    let sb_bar_w = sb_w.saturating_sub(28).clamp(6, 18);
    let mut sb_lines: Vec<Line> = Vec::new();
    if state.analytics.sb_categories.is_empty() {
        sb_lines.push(Line::from(Span::styled(
            format!(
                " Total SB saved: {:.1}m (category breakdown populates per session).",
                state.analytics.sponsorblock_saved_secs / 60.0
            ),
            Style::default().fg(theme.text_dim),
        )));
    } else {
        let max_sb = state
            .analytics
            .sb_categories
            .first()
            .map(|c| c.saved_secs)
            .unwrap_or(1.0)
            .max(1.0);
        for cat in state.analytics.sb_categories.iter().take(6) {
            let mut sp = vec![Span::styled(
                format!(" {:<11} ", truncate_words(&cat.label, 11)),
                Style::default().fg(theme.text),
            )];
            sp.extend(render_bar_spans(
                cat.saved_secs / max_sb,
                sb_bar_w,
                theme.accent,
                Color::Rgb(42, 47, 64),
            ));
            sp.push(Span::styled(
                format!(" {:>5.1}m ({:>2.0}%)", cat.saved_secs / 60.0, cat.share_pct),
                Style::default().fg(theme.text_dim),
            ));
            sb_lines.push(Line::from(sp));
        }
    }
    frame.render_widget(Paragraph::new(sb_lines).wrap(Wrap { trim: true }).block(sb_blk), right_split[0]);

    let eff_blk = Block::default()
        .title(Span::styled(
            " Playback Efficiency & Outcomes ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let a = &state.analytics;
    let eff_lines = vec![
        Line::from(Span::styled(
            format!(
                " Avg Speed: {:.2}x   Speed Saved: {}",
                a.weighted_avg_speed,
                format_mins_compact(a.speed_saved_mins)
            ),
            Style::default().fg(theme.text),
        )),
        Line::from(Span::styled(
            format!(
                " Sessions: {} total  |  Avg Len: {:.0}m  |  Finish: {:.0}%",
                a.total_sessions, a.avg_session_mins, a.completion_rate
            ),
            Style::default().fg(theme.text_dim),
        )),
        Line::from(Span::styled(
            format!(
                " Completed: {}  |  Partial: {}  |  Bounced (<15%): {}",
                a.completed_videos, a.partial_videos, a.bounced_videos
            ),
            Style::default().fg(theme.text_dim),
        )),
    ];
    frame.render_widget(Paragraph::new(eff_lines).wrap(Wrap { trim: true }).block(eff_blk), right_split[1]);
}

fn render_habits_tab(frame: &mut Frame, state: &mut AppState, area: Rect) {
    let theme = state.base_theme();
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(9), Constraint::Min(9)])
        .split(area);

    let top_cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(55), Constraint::Percentage(45)])
        .split(sections[0]);

    // Left: 24h Circadian Activity Bar Chart with axes
    let circ_blk = Block::default()
        .title(Span::styled(
            " 24h Circadian Activity (Minutes by Local Hour) ",
            Style::default()
                .fg(theme.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let circ_inner = circ_blk.inner(top_cols[0]);
    frame.render_widget(circ_blk, top_cols[0]);

    let a = &state.analytics;
    if circ_inner.height >= 5 && circ_inner.width >= 34 {
        let rows_h = (circ_inner.height as usize).saturating_sub(2).clamp(2, 6);
        let max_hr = a
            .hourly_minutes
            .iter()
            .copied()
            .fold(10.0f64, f64::max);
        let bar_cols = 24usize;
        let col_step = ((circ_inner.width.saturating_sub(8)) as usize / bar_cols).clamp(1, 2);
        let mut h_lines: Vec<Line> = Vec::new();
        for r in (0..rows_h).rev() {
            let tick = ((r + 1) as f64 / rows_h as f64) * max_hr;
            let mut spans = vec![Span::styled(
                format!("{:>4.0}m |", tick),
                Style::default().fg(theme.text_dim),
            )];
            for hr in 0..24 {
                let ratio = (a.hourly_minutes[hr] / max_hr).clamp(0.0, 1.0) * (rows_h as f64);
                let fill = ratio - (r as f64);
                let ch = if fill >= 0.8 {
                    "█"
                } else if fill >= 0.4 {
                    "▄"
                } else if fill > 0.05 {
                    "▁"
                } else {
                    " "
                };
                let col = match hr {
                    0..=5 => Color::Rgb(167, 139, 250),
                    6..=11 => Color::Rgb(56, 189, 248),
                    12..=17 => theme.primary,
                    _ => theme.accent,
                };
                spans.push(Span::styled(
                    format!("{}{}", ch, " ".repeat(col_step.saturating_sub(1))),
                    Style::default().fg(col),
                ));
            }
            h_lines.push(Line::from(spans));
        }

        let mut x_axis = String::from("   0m +");
        for hr in 0..24 {
            let ch_pair = if col_step >= 2 {
                if hr % 3 == 0 {
                    format!("{:02}", hr)
                } else {
                    "  ".to_string()
                }
            } else if hr % 6 == 0 {
                format!("{}", (hr / 10) % 10)
            } else {
                "-".to_string()
            };
            x_axis.push_str(&ch_pair);
        }
        h_lines.push(Line::from(Span::styled(
            x_axis,
            Style::default().fg(theme.text_dim),
        )));
        h_lines.push(Line::from(Span::styled(
            format!(
                " Peak: {}  |  Wkday {:.0}m/d vs Wkend {:.0}m/d",
                a.peak_window_label, a.weekday_avg_mins, a.weekend_avg_mins
            ),
            Style::default().fg(theme.text_dim),
        )));
        frame.render_widget(Paragraph::new(h_lines), circ_inner);
    }

    // Right: 12-Week Habit Heatmap with Mon..Sun labels and legend
    let heat_blk = Block::default()
        .title(Span::styled(
            " 12-Week Consistency Grid (Timed Sessions) ",
            Style::default()
                .fg(theme.primary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let heat_inner = heat_blk.inner(top_cols[1]);
    frame.render_widget(heat_blk, top_cols[1]);

    let dow_labels = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
    let mut heat_lines: Vec<Line> = Vec::new();
    for (dow, day_lbl) in dow_labels.iter().enumerate() {
        let mut spans = vec![Span::styled(
            format!(" {} ", day_lbl),
            Style::default().fg(theme.text_dim),
        )];
        for wk in 0..12 {
            let idx = wk * 7 + dow;
            let mins = a.thermal_activity.get(idx).copied().unwrap_or(0);
            let (ch, col) = match mins {
                0 => ("· ", Color::Rgb(45, 50, 66)),
                1..=20 => ("▪ ", Color::Rgb(56, 189, 248)),
                21..=59 => ("▪ ", Color::Rgb(163, 230, 53)),
                _ => ("█ ", Color::Rgb(251, 191, 36)),
            };
            spans.push(Span::styled(ch, Style::default().fg(col)));
        }
        heat_lines.push(Line::from(spans));
    }
    if heat_inner.height > 7 {
        heat_lines.push(Line::from(vec![
            Span::styled(" Less ", Style::default().fg(theme.text_dim)),
            Span::styled("· ", Style::default().fg(Color::Rgb(45, 50, 66))),
            Span::styled("▪ ", Style::default().fg(Color::Rgb(56, 189, 248))),
            Span::styled("▪ ", Style::default().fg(Color::Rgb(163, 230, 53))),
            Span::styled("█ ", Style::default().fg(Color::Rgb(251, 191, 36))),
            Span::styled("More (>60m)", Style::default().fg(theme.text_dim)),
        ]));
    }
    frame.render_widget(Paragraph::new(heat_lines), heat_inner);

    // Bottom: Interactive Session Ledger
    let log_blk = Block::default()
        .title(Span::styled(
            " Session Ledger (j/k Scroll · Enter Open Video Detail) ",
            Style::default()
                .fg(theme.accent)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let log_inner = log_blk.inner(sections[1]);
    frame.render_widget(log_blk, sections[1]);

    if a.recent_watch_log.is_empty() {
        frame.render_widget(
            Paragraph::new("No sessions in this range yet.")
                .style(Style::default().fg(theme.text_dim)),
            log_inner,
        );
        return;
    }

    let cursor = state
        .analysis_cursor
        .min(a.recent_watch_log.len().saturating_sub(1));
    let ch_w = 18usize;
    let fixed_cols = 13 + 22 + ch_w;
    let title_w = (log_inner.width as usize).saturating_sub(fixed_cols).clamp(16, 48);
    let mut rows_out: Vec<Line> = vec![Line::from(Span::styled(
        format!(
            "  {:<11}  {:<tw$}  {:<cw$} {:>9} {:>4} {:>5}",
            "Timestamp",
            "Title",
            "Creator",
            "Watch/Dur",
            "Fin",
            "Spd",
            tw = title_w,
            cw = ch_w
        ),
        Style::default()
            .fg(theme.text_dim)
            .add_modifier(Modifier::BOLD),
    ))];

    let tz = state.calendar_tz_offset;
    let max_vis = (log_inner.height as usize).saturating_sub(1);
    let start_idx = cursor.saturating_sub(max_vis.saturating_sub(1));
    for (vis_row, (idx, item)) in a
        .recent_watch_log
        .iter()
        .enumerate()
        .skip(start_idx)
        .take(max_vis)
        .enumerate()
    {
        let selected = idx == cursor;
        let row_y = log_inner.y + 1 + vis_row as u16;
        state.analysis_hitboxes.push((
            Rect::new(log_inner.x, row_y, log_inner.width, 1),
            AnalysisHit::SelectSession(idx),
        ));
        let bg = if selected {
            Color::Rgb(34, 41, 60)
        } else {
            Color::Reset
        };
        let ts_txt = format_clock_utc_approx(item.watched_at, tz);
        let dur_m = (item.video_duration_secs / 60.0).round() as u64;
        let wat_m = (item.watched_secs / 60.0).round() as u64;
        let ratio_txt = if dur_m > 0 {
            format!("{:>2}/{:>2}m", wat_m, dur_m)
        } else {
            format!("{:>2}m", wat_m)
        };
        let fin_pct = if item.completed {
            100
        } else if item.video_duration_secs > 1.0 {
            ((item.end_pos_secs / item.video_duration_secs) * 100.0).clamp(0.0, 99.0) as u64
        } else {
            0
        };

        rows_out.push(Line::from(vec![
            Span::styled(
                format!("{}{:<11}  ", if selected { ">" } else { " " }, ts_txt),
                Style::default().fg(theme.accent).bg(bg),
            ),
            Span::styled(
                format!("{:<tw$}  ", truncate_words(&item.title, title_w), tw = title_w),
                Style::default().fg(theme.text).bg(bg),
            ),
            Span::styled(
                format!(
                    "{:<cw$} {:>9} {:>3}% {:>4.2}x",
                    truncate_words(&item.channel_name, ch_w),
                    ratio_txt,
                    fin_pct,
                    item.playback_speed,
                    cw = ch_w
                ),
                Style::default().fg(theme.text_dim).bg(bg),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(rows_out), log_inner);
}

fn render_footer(frame: &mut Frame, state: &AppState, area: Rect) {
    let theme = state.base_theme();
    let extra = match state.analysis_tab {
        AnalysisTab::Channels => " [s] Sort ·",
        _ => "",
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(
                format!(" [1-3/Tab] View · [h/l] Range · [o] Source · [b/B] Budget ·{extra} [j/k] Select · [Enter] Open · "),
                Style::default().fg(theme.text_dim),
            ),
            Span::styled("[r] Refresh", Style::default().fg(theme.primary)),
            Span::styled(" · ", Style::default().fg(theme.divider)),
            Span::styled("[Esc / A] Close", Style::default().fg(theme.text_dim)),
        ]))
        .alignment(Alignment::Center),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::db::user::TopChannelEntry;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_analytics_dashboard_renders_cleanly_all_tabs() {
        let mut st = AppState::new(Config::default());
        st.analytics.total_videos_watched = 28;
        st.analytics.total_sessions = 31;
        st.analytics.total_hours_watched = 11.4;
        st.analytics.today_watch_mins = 44.0;
        st.analytics.hourly_minutes[20] = 65.0;
        st.analytics.format_buckets[2].video_count = 9;
        st.analytics.format_buckets[2].actual_hours = 5.4;
        st.analytics.format_buckets[2].share_pct = 47.4;
        st.analytics.format_buckets[2].finish_rate_pct = 82.0;
        st.analytics.top_channels.push(TopChannelEntry {
            channel_id: Some("UC123".into()),
            channel_name: "Fireship".into(),
            video_count: 4,
            total_hours: 1.9,
            share_pct: 17.0,
            finish_rate_pct: 85.0,
        });

        for tab in AnalysisTab::ALL {
            st.analysis_tab = tab;
            let backend = TestBackend::new(120, 36);
            let mut term = Terminal::new(backend).unwrap();
            term.draw(|f| render_analytics_modal(f, &mut st)).unwrap();
            let buf = term.backend().buffer();
            let txt: String = buf.content().iter().map(|c| c.symbol()).collect();
            assert!(txt.contains("PHAENNA ANALYSIS"));
            assert!(!txt.contains('◐'));
            assert!(!txt.contains('⬛'));
        }
    }

    #[test]
    fn test_word_boundary_truncation_no_midword_cut() {
        let out = truncate_words("Why Microsoft couldn't fix performance today", 24);
        assert_eq!(out, "Why Microsoft...");
        assert!(!out.ends_with("fi..."));
    }
}
