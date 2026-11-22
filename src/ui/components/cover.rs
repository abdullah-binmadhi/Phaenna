//! High-definition aspect-accurate thumbnail pipeline:
//! - Automatic 4:3 letterbox + solid vertical pillar trimming
//! - 100% full-frame proportional scaling (`FilterType::Lanczos3`) with centered cell fit (zero crop!)
//! - Atomic Kitty ID allocation (`>= 100_000`)
//! - Distortion-free Halfblock (`▀`) fallback cover widget for universal terminals

use crate::db::cache::VideoItem;
use crate::ui::home::marquee_window;
use image::{imageops::FilterType, DynamicImage};
use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, BorderType, Borders, Paragraph, Widget, Wrap},
    Frame,
};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol, Resize, StatefulImage};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_KITTY_IMAGE_ID: AtomicU32 = AtomicU32::new(100_000);

pub fn next_kitty_id() -> u32 {
    NEXT_KITTY_IMAGE_ID.fetch_add(1, Ordering::Relaxed)
}

/// Detect baked black letterbox bars at top & bottom of legacy 4:3 YouTube thumbnails.
pub fn is_letterboxed(img: &DynamicImage, sample_y: u32) -> bool {
    let rgb = img.to_rgb8();
    if rgb.width() == 0 || rgb.height() <= sample_y {
        return false;
    }
    let mid_x = rgb.width() / 2;
    let top_px = rgb.get_pixel(mid_x, sample_y / 2);
    let bot_px = rgb.get_pixel(mid_x, rgb.height() - 1 - (sample_y / 2));
    top_px[0] < 20
        && top_px[1] < 20
        && top_px[2] < 20
        && bot_px[0] < 20
        && bot_px[1] < 20
        && bot_px[2] < 20
}

/// Detect uniform solid grey/black side pillars often baked into vertical Shorts/news thumbnails.
fn detect_solid_side_pillars(img: &DynamicImage) -> Option<(u32, u32)> {
    let rgb = img.to_rgb8();
    let (w, h) = (rgb.width(), rgb.height());
    if w < 120 || h < 80 {
        return None;
    }
    let y_samples = [h / 6, h / 3, h / 2, (h * 2) / 3, (h * 5) / 6];
    let left_ref = *rgb.get_pixel(4, h / 2);
    let right_ref = *rgb.get_pixel(w - 5, h / 2);

    let max_diff = |p: &image::Rgb<u8>, q: &image::Rgb<u8>| -> u8 {
        p[0].abs_diff(q[0])
            .max(p[1].abs_diff(q[1]))
            .max(p[2].abs_diff(q[2]))
    };

    let is_neutral = |p: &image::Rgb<u8>| -> bool {
        let hi = p[0].max(p[1]).max(p[2]);
        let lo = p[0].min(p[1]).min(p[2]);
        hi - lo <= 16
    };
    if !is_neutral(&left_ref) || !is_neutral(&right_ref) || max_diff(&left_ref, &right_ref) > 20 {
        return None;
    }

    let mut left_edge = 0u32;
    while left_edge < w / 3 {
        if y_samples
            .iter()
            .any(|&y| max_diff(rgb.get_pixel(left_edge, y), &left_ref) > 12)
        {
            break;
        }
        left_edge += 1;
    }

    let mut right_edge = w.saturating_sub(1);
    while right_edge > (w * 2) / 3 {
        if y_samples
            .iter()
            .any(|&y| max_diff(rgb.get_pixel(right_edge, y), &right_ref) > 12)
        {
            break;
        }
        right_edge -= 1;
    }

    let right_pillar_w = w.saturating_sub(1).saturating_sub(right_edge);
    let symmetric = left_edge.abs_diff(right_pillar_w) <= 16;
    let center_differs = y_samples
        .iter()
        .any(|&y| max_diff(rgb.get_pixel(w / 2, y), &left_ref) > 15);

    if symmetric
        && center_differs
        && left_edge >= w / 12
        && right_edge <= w - w / 12
        && right_edge > left_edge + w / 4
    {
        Some((left_edge, right_edge - left_edge + 1))
    } else {
        None
    }
}

/// Strip genuine dead letterbox/pillar bars and normalize thumbnail resolution proportionally (100% uncropped).
pub fn prepare_16_9_thumbnail(img: DynamicImage) -> DynamicImage {
    if img.width() == 0 || img.height() == 0 {
        return img;
    }

    let mut clean = if img.width() == 480 && img.height() == 360 && is_letterboxed(&img, 30) {
        img.crop_imm(0, 45, 480, 270)
    } else if img.width() == 640 && img.height() == 480 && is_letterboxed(&img, 40) {
        img.crop_imm(0, 60, 640, 360)
    } else {
        img
    };

    if let Some((start_x, active_w)) = detect_solid_side_pillars(&clean) {
        let h = clean.height();
        clean = clean.crop_imm(start_x, 0, active_w, h);
    }

    // Preserve exact aspect ratio so 100% of the thumbnail stays visible (HD cap: 1280x720).
    let (max_w, max_h) = if clean.width() >= 1200 {
        (1280, 720)
    } else {
        (960, 720)
    };
    if clean.width() > max_w || clean.height() > max_h || clean.width() < 640 {
        clean.resize(max_w, max_h, FilterType::Lanczos3)
    } else {
        clean
    }
}

/// Compute a centered sub-`Rect` inside `area` matching `img_w:img_h` under cell pixel size `font_size`.
pub fn fit_centered_rect(area: Rect, img_w: u32, img_h: u32, font_size: (u16, u16)) -> Rect {
    if area.width == 0 || area.height == 0 || img_w == 0 || img_h == 0 {
        return area;
    }
    let fw = font_size.0.max(1) as f64;
    let fh = font_size.1.max(1) as f64;
    let box_w = area.width as f64 * fw;
    let box_h = area.height as f64 * fh;
    let scale = (box_w / img_w as f64).min(box_h / img_h as f64);

    let fit_cols = ((img_w as f64 * scale / fw).round() as u16).clamp(1, area.width);
    let fit_rows = ((img_h as f64 * scale / fh).round() as u16).clamp(1, area.height);
    let fit_x = area.x + (area.width.saturating_sub(fit_cols)) / 2;
    let fit_y = area.y + (area.height.saturating_sub(fit_rows)) / 2;
    Rect::new(fit_x, fit_y, fit_cols, fit_rows)
}

pub fn build_stateful_protocol(
    picker: &mut Picker,
    img: DynamicImage,
) -> Box<dyn StatefulProtocol> {
    let _id = next_kitty_id();
    picker.new_resize_protocol(img)
}

/// Detect YouTube's tiny 120x90 flat-grey 404 placeholder so the loader tries fallback CDNs instead of caching it.
pub fn is_yt_grey_placeholder(img: &DynamicImage) -> bool {
    if img.width() != 120 || img.height() != 90 {
        return false;
    }
    let rgb = img.to_rgb8();
    let mut min_luma = u8::MAX;
    let mut max_luma = 0u8;
    let mut max_chroma = 0u8;
    for y in (20..70).step_by(5) {
        for x in (24..96).step_by(6) {
            let p = rgb.get_pixel(x, y);
            let hi = p[0].max(p[1]).max(p[2]);
            let lo = p[0].min(p[1]).min(p[2]);
            max_chroma = max_chroma.max(hi - lo);
            let luma = ((p[0] as u16 + p[1] as u16 + p[2] as u16) / 3) as u8;
            min_luma = min_luma.min(luma);
            max_luma = max_luma.max(luma);
        }
    }
    max_chroma <= 12 && max_luma.saturating_sub(min_luma) <= 18
}

/// Render 100% of the full thumbnail at accurate aspect scale, centered inside `area`.
pub fn render_thumbnail_cover(
    frame: &mut Frame,
    area: Rect,
    video_id: &str,
    state: &mut crate::state::AppState,
    fallback_label: &str,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }

    let (fit_rect, font_size) = match (&state.picker, state.raw_thumbs.get(video_id)) {
        (Some(picker), Some(raw_img)) => (
            fit_centered_rect(area, raw_img.width(), raw_img.height(), picker.font_size),
            picker.font_size,
        ),
        _ => (area, (8, 16)),
    };

    let key = (video_id.to_string(), fit_rect.width, fit_rect.height);
    if state.image_cache.contains_key(&key) {
        // Refresh LRU position on cache hit
        if let Some(pos) = state.image_cache_order.iter().position(|k| k == &key) {
            state.image_cache_order.remove(pos);
        }
        state.image_cache_order.push_back(key.clone());
    } else if let (Some(ref mut picker), Some(raw_img)) =
        (&mut state.picker, state.raw_thumbs.get(video_id))
    {
        if state.image_cache.len() >= 220 {
            for _ in 0..40 {
                if let Some(old_k) = state.image_cache_order.pop_front() {
                    state.image_cache.remove(&old_k);
                } else {
                    break;
                }
            }
        }
        let (fw, fh) = font_size;
        let px_w = (fit_rect.width as u32 * fw.max(6) as u32).max(32);
        let px_h = (fit_rect.height as u32 * fh.max(12) as u32).max(32);
        let scaled = raw_img.resize(px_w, px_h, FilterType::Lanczos3);
        let proto = build_stateful_protocol(picker, scaled);
        state.image_cache.insert(key.clone(), proto);
        state.image_cache_order.push_back(key.clone());
    }

    if let Some(proto) = state.image_cache.get_mut(&key) {
        frame.render_stateful_widget(
            StatefulImage::new(None).resize(Resize::Fit(Some(FilterType::Lanczos3))),
            fit_rect,
            proto,
        );
    } else if let Some(img) = state.raw_thumbs.get(video_id) {
        frame.render_widget(HalfblockCover { image: img }, area);
    } else if state.is_thumb_failed(video_id) {
        let accent = color_from_id(video_id);
        let label = if area.height >= 4 && area.width >= 16 {
            "\nNO COVER\n[R] Retry"
        } else if area.width >= 8 {
            "[R]"
        } else {
            "—"
        };
        frame.render_widget(
            Paragraph::new(label)
                .alignment(Alignment::Center)
                .style(
                    Style::default()
                        .fg(accent)
                        .bg(Color::Rgb(20, 24, 34))
                        .add_modifier(Modifier::BOLD),
                ),
            area,
        );
    } else if !fallback_label.is_empty() {
        frame.render_widget(
            Paragraph::new(fallback_label)
                .alignment(Alignment::Center)
                .style(ratatui::style::Style::default().fg(color_from_id(video_id))),
            area,
        );
    }
}

/// Returns current Unix timestamp in seconds.
pub fn unix_now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// Shared right-hand Live Preview & Telemetry Inspector for split-pane views
/// (Upload Radar, Channel Catalog, and Search), eliminating empty right-side voids
/// while keeping thumbnails edge-to-edge 16:9 with marquee title and quick stats.
#[allow(clippy::too_many_arguments)]
pub fn render_video_preview_inspector(
    frame: &mut Frame,
    area: Rect,
    state: &mut crate::state::AppState,
    video: &VideoItem,
    now_unix: i64,
    status_badges: &[(&str, Color)],
    extra_line: Option<&str>,
    actions_hint: &str,
) {
    if area.width < 12 || area.height < 8 {
        return;
    }

    state.queue_thumb_with_priority(
        &video.id,
        &video.best_thumbnail_url(),
        crate::state::ThumbPriority::Focused,
    );
    let pal = state.active_palette_for(Some(&video.id));

    let blk = Block::default()
        .title(Span::styled(
            " LIVE PREVIEW ",
            Style::default()
                .fg(pal.secondary)
                .add_modifier(Modifier::BOLD),
        ))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(pal.border_focus));
    let inner = blk.inner(area);
    frame.render_widget(blk, area);

    if inner.width < 8 || inner.height < 6 {
        return;
    }

    let max_thumb_h = (inner.height * 46 / 100).max(5);
    let thumb_h = ((inner.width as f32 / 3.55).round() as u16).clamp(5, max_thumb_h);
    let sections = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(thumb_h),
            Constraint::Min(5),
            Constraint::Length(1),
        ])
        .split(inner);

    render_thumbnail_cover(frame, sections[0], &video.id, state, "Loading 16:9...");

    let inner_w = sections[1].width.max(8) as usize;
    let title_str = marquee_window(&video.title, inner_w, state.ticker_step);

    let uploader_lbl = if video.channel_verified {
        format!("{} ✓", video.display_uploader())
    } else {
        video.display_uploader().to_string()
    };
    let mut ch_spans = vec![
        Span::styled(
            uploader_lbl,
            Style::default().fg(pal.secondary).add_modifier(Modifier::BOLD),
        ),
        Span::styled("  ·  ", Style::default().fg(pal.text_dim)),
        Span::styled(
            video.display_duration(),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ),
    ];
    if video.is_new_release(now_unix) {
        ch_spans.push(Span::styled("  ", Style::default()));
        ch_spans.push(Span::styled(
            " NEW ",
            Style::default()
                .fg(Color::Black)
                .bg(Color::Rgb(52, 211, 153))
                .add_modifier(Modifier::BOLD),
        ));
    }
    for (badge_lbl, badge_col) in status_badges {
        ch_spans.push(Span::styled(" ", Style::default()));
        ch_spans.push(Span::styled(
            format!("[{}]", badge_lbl),
            Style::default().fg(*badge_col).add_modifier(Modifier::BOLD),
        ));
    }

    let mut tel_parts: Vec<String> = Vec::new();
    if let Some(v) = video.compact_views(true) {
        tel_parts.push(v);
    }
    if let Some(a) = video.compact_age(now_unix, true) {
        tel_parts.push(a);
    }
    if let Some(ext) = extra_line {
        if !ext.trim().is_empty() {
            tel_parts.push(ext.trim().to_string());
        }
    }
    let tel_str = if tel_parts.is_empty() {
        "Metadata synced · Press [Enter] for full telemetry & chapters".to_string()
    } else {
        tel_parts.join(" · ")
    };

    let desc = video
        .description
        .as_deref()
        .or(video.search_snippet.as_deref())
        .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            "Press [Enter] for full Detail Deck (Dislike sentiment, SponsorBlock segments, chapters) or [Space] to stream immediately.".to_string()
        });

    let meta_lines = vec![
        Line::from(Span::styled(
            title_str,
            Style::default().fg(pal.primary).add_modifier(Modifier::BOLD),
        )),
        Line::from(ch_spans),
        Line::from(Span::styled(tel_str, Style::default().fg(pal.text_dim))),
        Line::from(Span::styled(
            "─".repeat(inner_w.min(64)),
            Style::default().fg(pal.divider),
        )),
        Line::from(Span::styled(desc, Style::default().fg(pal.text))),
    ];
    frame.render_widget(
        Paragraph::new(meta_lines).wrap(Wrap { trim: true }),
        sections[1],
    );

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            actions_hint.to_string(),
            Style::default().fg(pal.accent).add_modifier(Modifier::BOLD),
        ))),
        sections[2],
    );
}

/// Deterministic pastel accent color from a video/channel ID for crisp card frames.
pub fn color_from_id(id: &str) -> Color {
    let hash: u32 = id.bytes().fold(2166136261u32, |acc, b| {
        acc.wrapping_mul(16777619) ^ (b as u32)
    });
    let palette = [
        Color::Rgb(125, 207, 255),
        Color::Rgb(255, 118, 142),
        Color::Rgb(158, 206, 106),
        Color::Rgb(187, 154, 247),
        Color::Rgb(224, 175, 104),
        Color::Rgb(115, 218, 202),
    ];
    palette[(hash as usize) % palette.len()]
}

/// Crisp aspect-accurate Unicode half-block (`▀`) thumbnail widget for zero-dependency terminals.
pub struct HalfblockCover<'a> {
    pub image: &'a DynamicImage,
}

impl<'a> Widget for HalfblockCover<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 || self.image.width() == 0 || self.image.height() == 0
        {
            return;
        }
        let fit = fit_centered_rect(area, self.image.width(), self.image.height(), (1, 2));
        let target_w = fit.width as u32;
        let target_h = (fit.height as u32) * 2;
        let resized = self
            .image
            .resize(target_w, target_h, FilterType::Lanczos3)
            .to_rgb8();

        let render_rows = resized.height().div_ceil(2).min(fit.height as u32) as u16;
        let render_cols = resized.width().min(fit.width as u32) as u16;
        let start_x = fit.x + (fit.width.saturating_sub(render_cols)) / 2;
        let start_y = fit.y + (fit.height.saturating_sub(render_rows)) / 2;

        for row in 0..render_rows {
            let py_top = (row as u32) * 2;
            let py_bot = (py_top + 1).min(resized.height().saturating_sub(1));
            for col in 0..render_cols {
                let px = col as u32;
                let top = resized.get_pixel(px, py_top);
                let bot = resized.get_pixel(px, py_bot);
                let cell = &mut buf[(start_x + col, start_y + row)];
                cell.set_char('▀')
                    .set_fg(Color::Rgb(top[0], top[1], top[2]))
                    .set_bg(Color::Rgb(bot[0], bot[1], bot[2]));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    #[test]
    fn test_letterbox_crop_and_halfblock_render() {
        let mut raw = image::RgbImage::new(480, 360);
        for y in 45..315 {
            for x in 0..480 {
                raw.put_pixel(x, y, image::Rgb([180, 60, 90]));
            }
        }
        let img = DynamicImage::ImageRgb8(raw);
        assert!(is_letterboxed(&img, 30));
        let prepped = prepare_16_9_thumbnail(img);
        assert_eq!(prepped.width(), 960);
        assert_eq!(prepped.height(), 540);

        let backend = TestBackend::new(20, 6);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            f.render_widget(HalfblockCover { image: &prepped }, Rect::new(0, 0, 20, 6));
        })
        .unwrap();
    }

    #[test]
    fn test_solid_side_pillar_crop_and_full_thumbnail_aspect_preserved() {
        let mut raw = image::RgbImage::new(480, 360);
        for y in 0..360 {
            for x in 0..480 {
                if (120..360).contains(&x) {
                    raw.put_pixel(x, y, image::Rgb([(x % 250) as u8, 120, 200]));
                } else {
                    raw.put_pixel(x, y, image::Rgb([96, 96, 96]));
                }
            }
        }
        let prepped = prepare_16_9_thumbnail(DynamicImage::ImageRgb8(raw));
        // Middle 240x360 (2:3 aspect) scales cleanly to 480x720 with zero vertical crop.
        assert_eq!(prepped.width(), 480);
        assert_eq!(prepped.height(), 720);
        let px = prepped.to_rgb8().get_pixel(5, 360).0;
        assert_ne!(px, [96, 96, 96], "Grey side pillars should be cropped out");

        // Verify wide card box (36x7 @ 10x22 cells -> 360x154px = 2.34:1) keeps 16:9 image uncropped & centered.
        let area = Rect::new(2, 1, 36, 7);
        let fit_16_9 = fit_centered_rect(area, 1280, 720, (10, 22));
        assert_eq!(fit_16_9.height, 7);
        assert_eq!(fit_16_9.width, 27);
        assert!(fit_16_9.x > area.x);

        // Verify vertical 9:16 Short thumbnail scales cleanly inside card height without cropping.
        let fit_9_16 = fit_centered_rect(area, 405, 720, (10, 22));
        assert_eq!(fit_9_16.height, 7);
        assert_eq!(fit_9_16.width, 9);
    }

    #[test]
    fn test_is_yt_grey_placeholder() {
        let mut grey = image::RgbImage::new(120, 90);
        for p in grey.pixels_mut() {
            *p = image::Rgb([196, 196, 196]);
        }
        assert!(is_yt_grey_placeholder(&DynamicImage::ImageRgb8(grey)));

        let mut real_cover = image::RgbImage::new(120, 90);
        for (x, y, p) in real_cover.enumerate_pixels_mut() {
            *p = image::Rgb([(x * 2) as u8, (y * 2) as u8, 220]);
        }
        assert!(!is_yt_grey_placeholder(&DynamicImage::ImageRgb8(real_cover)));
    }
}
