//! High-definition 16:9 thumbnail pipeline:
//! - Automatic 4:3 baked letterbox cropping (`hqdefault`/`sddefault`)
//! - Pre-scaled cover sizing + atomic Kitty ID allocation (`>= 100_000`)
//! - Ultra-crisp Halfblock 2-rows-per-cell fallback widget for universal terminals

use image::DynamicImage;
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Color,
    widgets::Widget,
};
use ratatui_image::{picker::Picker, protocol::StatefulProtocol};
use std::sync::atomic::{AtomicU32, Ordering};

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

/// Normalize a fetched thumbnail to clean edge-to-edge 16:9 geometry at HD width.
pub fn prepare_16_9_thumbnail(img: DynamicImage) -> DynamicImage {
    if img.width() == 0 || img.height() == 0 {
        return img;
    }

    let clean = if img.width() == 480 && img.height() == 360 && is_letterboxed(&img, 30) {
        img.crop_imm(0, 45, 480, 270)
    } else if img.width() == 640 && img.height() == 480 && is_letterboxed(&img, 40) {
        img.crop_imm(0, 60, 640, 360)
    } else {
        img
    };

    // Resize slightly crisply so detail/hero banners never upscale blurrily while keeping memory bounded
    if clean.width() < 960 && clean.width() > 0 {
        let aspect = clean.height() as f32 / clean.width() as f32;
        let target_h = (960.0 * aspect).round().max(540.0) as u32;
        clean.resize_exact(960, target_h, image::imageops::FilterType::Triangle)
    } else if clean.width() > 1280 {
        let aspect = clean.height() as f32 / clean.width() as f32;
        let target_h = (1280.0 * aspect).round().max(1.0) as u32;
        clean.resize_exact(1280, target_h, image::imageops::FilterType::Triangle)
    } else {
        clean
    }
}

#[allow(dead_code)]
pub fn scale_to_frame_cover(img: &DynamicImage, area: Rect) -> DynamicImage {
    let target_w = (area.width as u32 * 14).max(160);
    let target_h = (area.height as u32 * 28).max(90);
    img.resize_to_fill(target_w, target_h, image::imageops::FilterType::Triangle)
}

pub fn build_stateful_protocol(
    picker: &mut Picker,
    img: DynamicImage,
) -> Box<dyn StatefulProtocol> {
    let _id = next_kitty_id();
    picker.new_resize_protocol(img)
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

/// Crisp 2-rows-per-cell Unicode half-block (`▀`) image widget for zero-dependency thumbnails.
pub struct HalfblockCover<'a> {
    pub image: &'a DynamicImage,
}

impl<'a> Widget for HalfblockCover<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let target_w = area.width as u32;
        let target_h = (area.height as u32) * 2;
        let resized = self
            .image
            .resize_exact(target_w, target_h, image::imageops::FilterType::Triangle)
            .to_rgb8();

        for row in 0..area.height {
            let py_top = (row as u32) * 2;
            let py_bot = py_top + 1;
            for col in 0..area.width {
                let px = col as u32;
                let top = resized.get_pixel(px, py_top);
                let bot = resized.get_pixel(px, py_bot.min(target_h - 1));
                let cell = &mut buf[(area.x + col, area.y + row)];
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
}
