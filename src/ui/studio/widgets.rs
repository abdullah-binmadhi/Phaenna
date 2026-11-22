//! Reusable visual primitives for the 6-Page Playback Studio:
//! Braille charts, sparklines, bipolar sliders, segmented gauges,
//! SponsorBlock timeline maps, and live Gamma/Contrast tone curves.

use crate::api::sponsorblock::SponsorSegment;
use crate::config::PerformanceMode;
use crate::state::PlayOptionsHit;
use crate::ui::theme::ThemePalette;
use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph},
    Frame,
};

pub fn format_hms(secs: f64) -> String {
    let total = secs.max(0.0).round() as u64;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn format_speed_bps(bps: u64) -> String {
    if bps >= 1_048_576 {
        format!("{:.1}M/s", bps as f64 / 1_048_576.0)
    } else if bps >= 1024 {
        format!("{}K/s", bps / 1024)
    } else {
        "--".to_string()
    }
}

pub fn truncate_words(s: &str, max_chars: usize) -> String {
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

pub fn mode_badge_color(pm: PerformanceMode) -> Color {
    match pm {
        PerformanceMode::Efficient => Color::Rgb(52, 211, 153),
        PerformanceMode::Balanced => Color::Rgb(56, 189, 248),
        PerformanceMode::Quality => Color::Rgb(251, 191, 36),
        PerformanceMode::Manual => Color::Rgb(192, 132, 252),
    }
}

pub fn sponsor_color(category: &str) -> Color {
    match category {
        "sponsor" => Color::Rgb(251, 191, 36),
        "selfpromo" => Color::Rgb(244, 114, 182),
        "interaction" => Color::Rgb(204, 120, 255),
        "intro" => Color::Rgb(56, 189, 248),
        "outro" => Color::Rgb(96, 165, 250),
        "preview" => Color::Rgb(167, 139, 250),
        _ => Color::Rgb(248, 113, 113),
    }
}

pub fn push_line_hit(
    hitboxes: &mut Vec<(Rect, PlayOptionsHit)>,
    inner: Rect,
    row_offset: u16,
    hit: PlayOptionsHit,
) {
    if row_offset < inner.height && inner.width > 0 {
        hitboxes.push((Rect::new(inner.x, inner.y + row_offset, inner.width, 1), hit));
    }
}

pub fn push_pane_hit(hitboxes: &mut Vec<(Rect, PlayOptionsHit)>, rect: Rect, pane: usize) {
    if rect.width > 0 && rect.height > 0 {
        hitboxes.push((rect, PlayOptionsHit::FocusPane(pane)));
    }
}

pub fn section_block<'a>(title: &'a str, focused: bool, theme: &'a ThemePalette) -> Block<'a> {
    Block::default()
        .title(Span::styled(
            title,
            Style::default()
                .fg(if focused { theme.primary } else { theme.text_dim })
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(if focused {
            theme.border_focus
        } else {
            theme.border_idle
        }))
}

#[allow(clippy::too_many_arguments)]
pub fn studio_kv_row(
    label: &str,
    val: &str,
    tag_opt: Option<&str>,
    label_w: usize,
    highlighted: bool,
    val_style: Style,
    inner_w: u16,
    theme: &ThemePalette,
) -> Line<'static> {
    let prefix = if highlighted { ">" } else { " " };
    let lbl_fit = truncate_words(label, label_w);
    let left_col = format!("{prefix}{lbl_fit:<label_w$} │ ");
    let left_chars = left_col.chars().count();

    let tag_col = tag_opt
        .filter(|_| (inner_w as usize) >= left_chars + 16)
        .map(|t| format!(" [{t}] "))
        .unwrap_or_default();
    let tag_chars = tag_col.chars().count();

    let val_budget = (inner_w as usize).saturating_sub(left_chars + tag_chars);
    let val_fit = truncate_words(val, val_budget);
    let val_padded = format!("{val_fit:<val_budget$}");

    let lbl_style = if highlighted {
        Style::default()
            .fg(Color::Black)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text_dim)
    };
    let effective_val_style = if highlighted {
        val_style.add_modifier(Modifier::BOLD)
    } else {
        val_style
    };

    let mut spans = vec![
        Span::styled(format!("{prefix}{lbl_fit:<label_w$}"), lbl_style),
        Span::styled(" │ ", Style::default().fg(theme.divider)),
        Span::styled(val_padded, effective_val_style),
    ];
    if !tag_col.is_empty() {
        spans.push(Span::styled(tag_col, Style::default().fg(theme.text_dim)));
    }
    Line::from(spans)
}

pub fn render_bento_kpi_card(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    headline: &str,
    sub_spans: Vec<Span<'static>>,
    accent: Color,
    theme: &ThemePalette,
) {
    let blk = Block::default()
        .title(Span::styled(
            title.to_string(),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.border_idle));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);
    if inner.width < 6 || inner.height == 0 {
        return;
    }
    let head_fit = truncate_words(headline, inner.width.saturating_sub(1) as usize);
    let mut lines = vec![Line::from(Span::styled(
        format!(" {head_fit}"),
        Style::default().fg(theme.text).add_modifier(Modifier::BOLD),
    ))];
    if inner.height >= 2 {
        lines.push(Line::from(sub_spans));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

pub fn sparkline_string(values: &[f64], width: usize) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if width == 0 {
        return String::new();
    }
    if values.is_empty() {
        return "▁".repeat(width.min(8));
    }
    let slice = if values.len() > width {
        &values[values.len() - width..]
    } else {
        values
    };
    let max_v = slice.iter().copied().fold(0.0_f64, f64::max).max(1e-6);
    let min_v = slice.iter().copied().fold(f64::INFINITY, f64::min).min(max_v);
    let span = (max_v - min_v).max(1e-6);

    let mut out = String::with_capacity(width);
    if slice.len() < width {
        out.push_str(&" ".repeat(width - slice.len()));
    }
    for &v in slice {
        let norm = if max_v <= 1e-6 {
            0.0
        } else if (max_v - min_v).abs() < 1e-6 {
            0.5
        } else {
            ((v - min_v) / span).clamp(0.0, 1.0)
        };
        let idx = ((norm * 7.0).round() as usize).min(7);
        out.push(BARS[idx]);
    }
    out
}

/// Braille 2×4 pixel-grid dual-series line plotter.
/// Dot bit layout per Unicode Braille cell (`0x2800`):
/// left col `(dx=0)`: rows `0,1,2,3` => bits `0x01, 0x02, 0x04, 0x40`
/// right col `(dx=1)`: rows `0,1,2,3` => bits `0x08, 0x10, 0x20, 0x80`
const BRAILLE_BITS: [[u8; 4]; 2] = [[0x01, 0x02, 0x04, 0x40], [0x08, 0x10, 0x20, 0x80]];

fn resample_series(values: &[f64], target_len: usize) -> Vec<f64> {
    if target_len == 0 {
        return Vec::new();
    }
    if values.is_empty() {
        return vec![0.0; target_len];
    }
    if values.len() == 1 {
        return vec![values[0]; target_len];
    }
    (0..target_len)
        .map(|i| {
            let pos = (i as f64 / (target_len.saturating_sub(1).max(1)) as f64)
                * (values.len().saturating_sub(1) as f64);
            let lo = pos.floor() as usize;
            let hi = (lo + 1).min(values.len() - 1);
            let frac = pos - lo as f64;
            values[lo] * (1.0 - frac) + values[hi] * frac
        })
        .collect()
}

struct BrailleCanvas {
    bits: Vec<Vec<u8>>,
    tags: Vec<Vec<u8>>,
    px_w: usize,
    px_h: usize,
}

impl BrailleCanvas {
    fn new(plot_cols: usize, plot_rows: usize) -> Self {
        Self {
            bits: vec![vec![0u8; plot_cols]; plot_rows],
            tags: vec![vec![0u8; plot_cols]; plot_rows],
            px_w: plot_cols * 2,
            px_h: plot_rows * 4,
        }
    }

    fn plot_point(&mut self, x: usize, y: usize, tag: u8) {
        if x < self.px_w && y < self.px_h {
            self.bits[y / 4][x / 2] |= BRAILLE_BITS[x % 2][y % 4];
            self.tags[y / 4][x / 2] |= tag;
        }
    }

    fn draw_segment(&mut self, x0: i32, y0: i32, x1: i32, y1: i32, tag: u8) {
        let steps = (x1 - x0).abs().max((y1 - y0).abs()).max(1);
        for s in 0..=steps {
            let t = s as f64 / steps as f64;
            let x = (x0 as f64 + (x1 - x0) as f64 * t).round() as isize;
            let y = (y0 as f64 + (y1 - y0) as f64 * t).round() as isize;
            if x >= 0 && y >= 0 {
                self.plot_point(x as usize, y as usize, tag);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn render_braille_dual_chart_lines(
    primary: &[f64],
    secondary: Option<&[f64]>,
    stalls: Option<&[bool]>,
    cols: usize,
    rows: usize,
    y_max_hint: f64,
    unit_suffix: &str,
    c1: Color,
    c2: Color,
    theme: &ThemePalette,
) -> Vec<Line<'static>> {
    let label_w = 6usize;
    let plot_cols = cols.saturating_sub(label_w + 1).max(6);
    let plot_rows = rows.max(2);
    let mut canvas = BrailleCanvas::new(plot_cols, plot_rows);

    let mut max_v = y_max_hint.max(1.0);
    for &v in primary {
        if v.is_finite() {
            max_v = max_v.max(v * 1.12);
        }
    }
    if let Some(sec) = secondary {
        for &v in sec {
            if v.is_finite() {
                max_v = max_v.max(v * 1.12);
            }
        }
    }

    if let Some(sec) = secondary {
        let s2 = resample_series(sec, canvas.px_w);
        for x in 1..canvas.px_w {
            let y0 = ((1.0 - (s2[x - 1] / max_v).clamp(0.0, 1.0)) * (canvas.px_h - 1) as f64).round() as i32;
            let y1 = ((1.0 - (s2[x] / max_v).clamp(0.0, 1.0)) * (canvas.px_h - 1) as f64).round() as i32;
            canvas.draw_segment((x - 1) as i32, y0, x as i32, y1, 2);
        }
    }

    if !primary.is_empty() {
        let s1 = resample_series(primary, canvas.px_w);
        for x in 1..canvas.px_w {
            let y0 = ((1.0 - (s1[x - 1] / max_v).clamp(0.0, 1.0)) * (canvas.px_h - 1) as f64).round() as i32;
            let y1 = ((1.0 - (s1[x] / max_v).clamp(0.0, 1.0)) * (canvas.px_h - 1) as f64).round() as i32;
            canvas.draw_segment((x - 1) as i32, y0, x as i32, y1, 1);
        }
    }

    let mut stall_cols = vec![false; plot_cols];
    if let Some(st) = stalls {
        if !st.is_empty() {
            for (c, slot) in stall_cols.iter_mut().enumerate().take(plot_cols) {
                let idx = (c * st.len()) / plot_cols.max(1);
                if st.get(idx).copied().unwrap_or(false) {
                    *slot = true;
                }
            }
        }
    }

    let mut out = Vec::with_capacity(plot_rows + 1);
    for r in 0..plot_rows {
        let tick_lbl = if r == 0 {
            format!("{:>4.0}{}", max_v, unit_suffix)
        } else if r == plot_rows / 2 && plot_rows >= 3 {
            format!("{:>4.0}{}", max_v * 0.5, unit_suffix)
        } else if r == plot_rows - 1 {
            format!("{:>4}{}", 0, unit_suffix)
        } else {
            String::new()
        };
        let axis_char = if r == plot_rows - 1 { '└' } else { '│' };
        let mut spans = vec![Span::styled(
            format!("{tick_lbl:>label_w$}{axis_char}"),
            Style::default().fg(theme.text_dim),
        )];
        for (c, &has_stall) in stall_cols.iter().enumerate().take(plot_cols) {
            if has_stall && r == 0 {
                spans.push(Span::styled(
                    "!",
                    Style::default()
                        .fg(Color::Rgb(248, 113, 113))
                        .add_modifier(Modifier::BOLD),
                ));
                continue;
            }
            let mask = canvas.bits[r][c];
            let ch = char::from_u32(0x2800 + mask as u32).unwrap_or(' ');
            let color = if mask == 0 {
                theme.divider
            } else if (canvas.tags[r][c] & 1) != 0 {
                c1
            } else {
                c2
            };
            spans.push(Span::styled(ch.to_string(), Style::default().fg(color)));
        }
        out.push(Line::from(spans));
    }
    out
}

/// Live Luma Transfer Curve plotting Brightness + Contrast + Gamma against 1:1 diagonal.
pub fn render_tone_curve_lines(
    brightness: i64,
    contrast: i64,
    gamma: i64,
    cols: usize,
    rows: usize,
    theme: &ThemePalette,
) -> Vec<Line<'static>> {
    let plot_cols = cols.saturating_sub(6).max(8);
    let plot_rows = rows.clamp(3, 7);
    let mut canvas = BrailleCanvas::new(plot_cols, plot_rows);

    // 1:1 neutral reference diagonal (dim tag 2)
    for x in 0..canvas.px_w {
        let norm = x as f64 / (canvas.px_w.saturating_sub(1).max(1)) as f64;
        let y = ((1.0 - norm) * (canvas.px_h - 1) as f64).round() as usize;
        canvas.plot_point(x, y, 2);
    }

    let g_exp = (1.0 - (gamma as f64) / 100.0).clamp(0.35, 2.2);
    let c_mul = (1.0 + (contrast as f64) / 100.0).clamp(0.35, 2.0);
    let b_off = (brightness as f64) / 160.0;

    let mut prev_y = None;
    for x in 0..canvas.px_w {
        let nx = x as f64 / (canvas.px_w.saturating_sub(1).max(1)) as f64;
        let curved = ((nx.powf(g_exp) - 0.5) * c_mul + 0.5 + b_off).clamp(0.0, 1.0);
        let y = ((1.0 - curved) * (canvas.px_h - 1) as f64).round() as i32;
        if let Some(py) = prev_y {
            canvas.draw_segment((x - 1) as i32, py, x as i32, y, 1);
        } else {
            canvas.plot_point(x, (y as usize).min(canvas.px_h - 1), 1);
        }
        prev_y = Some(y);
    }

    let neutral = brightness == 0 && contrast == 0 && gamma == 0;
    let curve_col = if neutral {
        Color::Rgb(52, 211, 153)
    } else {
        theme.accent
    };

    let mut lines = Vec::with_capacity(plot_rows);
    for r in 0..plot_rows {
        let lbl = if r == 0 {
            "1.0│"
        } else if r == plot_rows / 2 {
            "0.5│"
        } else if r == plot_rows - 1 {
            "0.0└"
        } else {
            "   │"
        };
        let mut spans = vec![Span::styled(
            format!(" {lbl}"),
            Style::default().fg(theme.text_dim),
        )];
        for c in 0..plot_cols {
            let ch = char::from_u32(0x2800 + canvas.bits[r][c] as u32).unwrap_or(' ');
            let fg = if (canvas.tags[r][c] & 1) != 0 {
                curve_col
            } else if (canvas.tags[r][c] & 2) != 0 {
                theme.divider
            } else {
                theme.text_dim
            };
            spans.push(Span::styled(ch.to_string(), Style::default().fg(fg)));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Braille compression transfer curve comparing 1:1 studio dynamic range vs `dynaudnorm`.
pub fn render_loudnorm_curve_lines(
    enabled: bool,
    cols: usize,
    rows: usize,
    theme: &ThemePalette,
) -> Vec<Line<'static>> {
    let plot_cols = cols.saturating_sub(7).max(8);
    let plot_rows = rows.clamp(3, 5);
    let mut canvas = BrailleCanvas::new(plot_cols, plot_rows);

    // 1:1 linear diagonal reference (dim tag 2)
    for x in 0..canvas.px_w {
        let nx = x as f64 / (canvas.px_w.saturating_sub(1).max(1)) as f64;
        let y = ((1.0 - nx) * (canvas.px_h - 1) as f64).round() as usize;
        canvas.plot_point(x, y, 2);
    }

    if enabled {
        let mut prev_y = None;
        for x in 0..canvas.px_w {
            let nx = x as f64 / (canvas.px_w.saturating_sub(1).max(1)) as f64;
            let comp = (nx.powf(0.58) * 0.90 + 0.06).clamp(0.0, 0.96);
            let y = ((1.0 - comp) * (canvas.px_h - 1) as f64).round() as i32;
            if let Some(py) = prev_y {
                canvas.draw_segment((x - 1) as i32, py, x as i32, y, 1);
            } else {
                canvas.plot_point(x, (y as usize).min(canvas.px_h - 1), 1);
            }
            prev_y = Some(y);
        }
    }

    let curve_col = if enabled {
        Color::Rgb(52, 211, 153)
    } else {
        theme.text_dim
    };

    let mut lines = Vec::with_capacity(plot_rows);
    for r in 0..plot_rows {
        let lbl = if r == 0 {
            "  0dB│"
        } else if r == plot_rows / 2 {
            "-18dB│"
        } else if r == plot_rows - 1 {
            "-36dB└"
        } else {
            "     │"
        };
        let mut spans = vec![Span::styled(
            format!(" {lbl}"),
            Style::default().fg(theme.text_dim),
        )];
        for c in 0..plot_cols {
            let ch = char::from_u32(0x2800 + canvas.bits[r][c] as u32).unwrap_or(' ');
            let fg = if (canvas.tags[r][c] & 1) != 0 {
                curve_col
            } else if (canvas.tags[r][c] & 2) != 0 {
                if enabled {
                    theme.divider
                } else {
                    theme.secondary
                }
            } else {
                theme.divider
            };
            spans.push(Span::styled(ch.to_string(), Style::default().fg(fg)));
        }
        lines.push(Line::from(spans));
    }
    lines
}

/// Pre-launch Braille baseline chart displaying configured target ceiling instead of an empty void.
#[allow(clippy::too_many_arguments)]
pub fn render_braille_standby_chart_lines(
    cols: usize,
    rows: usize,
    y_max: f64,
    target_val: f64,
    unit_suffix: &str,
    target_caption: &str,
    col: Color,
    theme: &ThemePalette,
) -> Vec<Line<'static>> {
    let label_w = 6usize;
    let plot_cols = cols.saturating_sub(label_w + 1).max(8);
    let plot_rows = rows.max(2);
    let max_v = y_max.max(target_val * 1.25).max(1.0);
    let mut canvas = BrailleCanvas::new(plot_cols, plot_rows);

    let y_px = ((1.0 - (target_val / max_v).clamp(0.05, 0.95)) * (canvas.px_h - 1) as f64).round()
        as usize;
    for x in 0..canvas.px_w {
        if x % 4 < 2 {
            canvas.plot_point(x, y_px, 1);
        }
    }

    let caption_row = plot_rows / 2;
    let cap_fit = truncate_words(target_caption, plot_cols.saturating_sub(2));
    let mut out = Vec::with_capacity(plot_rows);
    for r in 0..plot_rows {
        let tick_lbl = if r == 0 {
            format!("{:>4.0}{}", max_v, unit_suffix)
        } else if r == plot_rows / 2 && plot_rows >= 3 {
            format!("{:>4.0}{}", max_v * 0.5, unit_suffix)
        } else if r == plot_rows - 1 {
            format!("{:>4}{}", 0, unit_suffix)
        } else {
            String::new()
        };
        let axis_char = if r == plot_rows - 1 { '└' } else { '│' };
        let mut spans = vec![Span::styled(
            format!("{tick_lbl:>label_w$}{axis_char}"),
            Style::default().fg(theme.text_dim),
        )];
        if r == caption_row && plot_cols > 12 {
            spans.push(Span::styled(
                format!(" {cap_fit}"),
                Style::default().fg(col).add_modifier(Modifier::BOLD),
            ));
        } else {
            for c in 0..plot_cols {
                let mask = canvas.bits[r][c];
                let ch = char::from_u32(0x2800 + mask as u32).unwrap_or(' ');
                let fg = if mask == 0 { theme.divider } else { col };
                spans.push(Span::styled(ch.to_string(), Style::default().fg(fg)));
            }
        }
        out.push(Line::from(spans));
    }
    out
}

/// Interactive slider (`min_val ..= max_val`) with center neutral mark and exact hitboxes.
#[allow(clippy::too_many_arguments)]
pub fn render_bipolar_slider_range(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: i32,
    min_val: i32,
    max_val: i32,
    focused: bool,
    bar_hit: PlayOptionsHit,
    dec_hit: PlayOptionsHit,
    inc_hit: PlayOptionsHit,
    hitboxes: &mut Vec<(Rect, PlayOptionsHit)>,
    theme: &ThemePalette,
) {
    if area.width < 28 || area.height == 0 {
        return;
    }
    let btn_w = 3usize; // "[-]" "[+]"
    // Prefix: " " (1) + label (10) + " " (1) + value (4) + "  " (2) = 18 chars
    let prefix_w = 18usize;
    let rail_w = (area.width as usize)
        .saturating_sub(prefix_w + btn_w * 2 + 3)
        .max(5);

    let x_dec = area.x + prefix_w as u16;
    let x_rail = x_dec + btn_w as u16 + 1;
    let x_inc = x_rail + rail_w as u16 + 1;

    let span = (max_val - min_val).max(1) as f64;
    let norm = ((value.clamp(min_val, max_val) - min_val) as f64 / span).clamp(0.0, 1.0);
    let zero_norm = ((0 - min_val) as f64 / span).clamp(0.0, 1.0);
    let handle_pos = ((norm * (rail_w.saturating_sub(1)) as f64).round() as usize)
        .min(rail_w.saturating_sub(1));
    let mid = ((zero_norm * (rail_w.saturating_sub(1)) as f64).round() as usize)
        .min(rail_w.saturating_sub(1));

    let mut rail_spans = Vec::with_capacity(rail_w + 2);
    for i in 0..rail_w {
        let active_span = (i >= mid.min(handle_pos) && i <= mid.max(handle_pos)) && value != 0;
        let ch = if i == handle_pos {
            '●'
        } else if i == mid {
            '│'
        } else if active_span {
            '━'
        } else {
            '─'
        };
        let col = if i == handle_pos {
            if focused {
                theme.primary
            } else {
                theme.accent
            }
        } else if active_span {
            theme.secondary
        } else {
            theme.text_dim
        };
        rail_spans.push(Span::styled(ch.to_string(), Style::default().fg(col)));
    }

    let lbl_style = if focused {
        Style::default()
            .fg(Color::Black)
            .bg(theme.primary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text)
    };
    let val_col = if value == 0 {
        theme.text_dim
    } else if value > 0 {
        Color::Rgb(52, 211, 153)
    } else {
        Color::Rgb(251, 191, 36)
    };
    let lbl_fit = truncate_words(label, 10);

    let mut spans = vec![
        Span::styled(format!(" {lbl_fit:<10}"), lbl_style),
        Span::styled(
            format!(" {value:>+4}  "),
            Style::default().fg(val_col).add_modifier(Modifier::BOLD),
        ),
        Span::styled("[-]", Style::default().fg(theme.secondary)),
        Span::raw(" "),
    ];
    spans.extend(rail_spans);
    spans.push(Span::raw(" "));
    spans.push(Span::styled("[+]", Style::default().fg(theme.secondary)));

    frame.render_widget(Paragraph::new(Line::from(spans)), area);

    hitboxes.push((Rect::new(x_dec, area.y, btn_w as u16, 1), dec_hit));
    hitboxes.push((Rect::new(x_rail, area.y, rail_w as u16, 1), bar_hit));
    if x_inc + btn_w as u16 <= area.x + area.width {
        hitboxes.push((Rect::new(x_inc, area.y, btn_w as u16, 1), inc_hit));
    }
}

/// Interactive centered bipolar slider (`-max_abs .. 0 .. +max_abs`) rendered inside `area`.
#[allow(dead_code, clippy::too_many_arguments)]
pub fn render_bipolar_slider(
    frame: &mut Frame,
    area: Rect,
    label: &str,
    value: i32,
    max_abs: i32,
    focused: bool,
    bar_hit: PlayOptionsHit,
    dec_hit: PlayOptionsHit,
    inc_hit: PlayOptionsHit,
    hitboxes: &mut Vec<(Rect, PlayOptionsHit)>,
    theme: &ThemePalette,
) {
    render_bipolar_slider_range(
        frame,
        area,
        label,
        value,
        -max_abs,
        max_abs,
        focused,
        bar_hit,
        dec_hit,
        inc_hit,
        hitboxes,
        theme,
    );
}

/// Render clickable `[pill]` items on a single row with guaranteed 1-space separation
/// and zero mid-word cutoff or `][` concatenation.
pub fn render_inline_pills(
    frame: &mut Frame,
    area: Rect,
    prefix: Option<(&str, Style)>,
    pills: &[(String, Style, PlayOptionsHit)],
    hitboxes: &mut Vec<(Rect, PlayOptionsHit)>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let max_x = area.x + area.width;
    let mut x_cur = area.x;
    let mut spans: Vec<Span<'static>> = Vec::new();

    if let Some((pfx, pfx_style)) = prefix {
        let p_w = pfx.chars().count() as u16;
        if x_cur + p_w <= max_x {
            spans.push(Span::styled(pfx.to_string(), pfx_style));
            x_cur += p_w;
        }
    }

    for (idx, (label, style, hit)) in pills.iter().enumerate() {
        if idx > 0 || prefix.is_some() {
            if x_cur + 1 > max_x {
                break;
            }
            spans.push(Span::raw(" "));
            x_cur += 1;
        }
        let w = label.chars().count() as u16;
        if x_cur + w > max_x {
            break;
        }
        hitboxes.push((Rect::new(x_cur, area.y, w, 1), hit.clone()));
        spans.push(Span::styled(label.clone(), *style));
        x_cur += w;
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// Compute normalized `(black_floor_pct, midtone_pct, white_clip_pct)` from Luma EQ params.
pub fn compute_luma_scope(brightness: i64, contrast: i64, gamma: i64) -> (f64, f64, f64) {
    let g_exp = (1.0 - (gamma as f64) / 100.0).clamp(0.35, 2.2);
    let c_mul = (1.0 + (contrast as f64) / 100.0).clamp(0.35, 2.0);
    let b_off = (brightness as f64) / 160.0;
    let eval = |x: f64| -> f64 {
        (((x.powf(g_exp) - 0.5) * c_mul + 0.5 + b_off).clamp(0.0, 1.0)) * 100.0
    };
    (eval(0.0), eval(0.5), eval(1.0))
}

pub fn segmented_gauge_line(
    label: &str,
    pct: u8,
    right_text: &str,
    width: usize,
    theme: &ThemePalette,
) -> Line<'static> {
    let lbl_w = 10usize;
    let right_w = right_text.chars().count();
    let bar_w = width.saturating_sub(lbl_w + right_w + 4).clamp(8, 28);
    let filled = (((pct.min(100) as usize) * bar_w) + 50) / 100;
    let empty = bar_w.saturating_sub(filled);
    let fill_col = if pct >= 80 {
        Color::Rgb(248, 113, 113)
    } else if pct >= 55 {
        Color::Rgb(251, 191, 36)
    } else {
        Color::Rgb(52, 211, 153)
    };

    Line::from(vec![
        Span::styled(format!(" {label:<lbl_w$}"), Style::default().fg(theme.text_dim)),
        Span::styled("█".repeat(filled), Style::default().fg(fill_col)),
        Span::styled("░".repeat(empty), Style::default().fg(theme.divider)),
        Span::styled(
            format!(" {right_text}"),
            Style::default().fg(fill_col).add_modifier(Modifier::BOLD),
        ),
    ])
}

pub fn sponsor_timeline_with_chapters_spans(
    segments: &[SponsorSegment],
    chapter_starts: &[f64],
    pos_secs: f64,
    dur_secs: f64,
    width: usize,
    theme: &ThemePalette,
) -> Vec<Span<'static>> {
    if width < 6 || dur_secs <= 1.0 {
        return vec![Span::styled(
            "─".repeat(width),
            Style::default().fg(theme.divider),
        )];
    }
    let playhead_col = ((pos_secs / dur_secs).clamp(0.0, 1.0) * (width.saturating_sub(1)) as f64)
        .round() as usize;

    let mut chapter_cols = vec![false; width];
    for &st in chapter_starts {
        if st > 1.0 && st < dur_secs {
            let c = ((st / dur_secs).clamp(0.0, 1.0) * (width.saturating_sub(1)) as f64).round()
                as usize;
            if let Some(slot) = chapter_cols.get_mut(c) {
                *slot = true;
            }
        }
    }

    let mut out = Vec::with_capacity(width);
    for (col, &has_chapter) in chapter_cols.iter().enumerate().take(width) {
        if col == playhead_col {
            out.push(Span::styled(
                "▼",
                Style::default()
                    .fg(theme.text)
                    .add_modifier(Modifier::BOLD),
            ));
            continue;
        }
        let t = ((col as f64 + 0.5) / width as f64) * dur_secs;
        let seg_cat = segments
            .iter()
            .find(|s| t >= s.start_time && t <= s.end_time)
            .map(|s| s.category.as_str());
        if let Some(cat) = seg_cat {
            out.push(Span::styled("█", Style::default().fg(sponsor_color(cat))));
        } else if has_chapter {
            out.push(Span::styled("┆", Style::default().fg(theme.secondary)));
        } else {
            out.push(Span::styled("─", Style::default().fg(theme.divider)));
        }
    }
    out
}
