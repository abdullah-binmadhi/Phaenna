//! Shared animated GIF, WebP & static sticker/image decoder + premultiplied BGRA sprite cache
//! powering BOTH Phaenna's TUI comment trees (`detail.rs`) and `mpv` fullscreen overlays (`overlay-add`).

use crate::config::Config;
use image::{imageops::FilterType, AnimationDecoder, DynamicImage, GenericImageView, ImageFormat};
use std::io::Cursor;
use std::path::PathBuf;

/// Cached decoded frames for a comment attachment (animated GIF / sticker / image).
#[derive(Clone, Debug)]
pub struct DecodedCommentMedia {
    pub key: String,
    pub overlay_w: u32,
    pub overlay_h: u32,
    pub frame_delay_ms: u64,
    pub frames: Vec<DynamicImage>,
    pub bgra_paths: Vec<String>,
}

fn comment_media_cache_dir() -> PathBuf {
    Config::cache_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("phaenna"))
        .join("comment_media")
}

fn safe_file_stem(key: &str) -> String {
    key.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn scaled_sprite_dims(orig_w: u32, orig_h: u32) -> (u32, u32) {
    let w = orig_w.max(1);
    let h = orig_h.max(1);
    // Keep custom stickers/emojis crisp (~44px) and full GIFs/images sleek inside the 432px drawer.
    let (max_w, max_h) = if w <= 72 && h <= 72 {
        (44u32, 44u32)
    } else {
        (210u32, 118u32)
    };
    let sw = (w as f64) / (max_w as f64);
    let sh = (h as f64) / (max_h as f64);
    let scale = sw.max(sh).max(1.0);
    let out_w = ((w as f64 / scale).round() as u32).max(16);
    let out_h = ((h as f64 / scale).round() as u32).max(16);
    (out_w, out_h)
}

fn write_premultiplied_bgra_frame(
    dir: &std::path::Path,
    stem: &str,
    idx: usize,
    img: &DynamicImage,
    target_w: u32,
    target_h: u32,
) -> Option<String> {
    let out_path = dir.join(format!("{stem}_{idx}.bgra"));
    let expected = (target_w * target_h * 4) as u64;
    if let Ok(meta) = std::fs::metadata(&out_path) {
        if meta.len() == expected {
            return Some(out_path.to_string_lossy().into_owned());
        }
    }
    let resized = img.resize_exact(target_w, target_h, FilterType::Lanczos3).to_rgba8();
    let mut bgra = Vec::with_capacity(expected as usize);
    for px in resized.pixels() {
        let a = px[3] as u16;
        // Premultiplied alpha BGRA (required by mpv overlay-add for halo-free edges)
        let r = ((px[0] as u16 * a + 127) / 255) as u8;
        let g = ((px[1] as u16 * a + 127) / 255) as u8;
        let b = ((px[2] as u16 * a + 127) / 255) as u8;
        bgra.push(b);
        bgra.push(g);
        bgra.push(r);
        bgra.push(px[3]);
    }
    let tmp = dir.join(format!("{stem}_{idx}.tmp"));
    std::fs::write(&tmp, &bgra).ok()?;
    let _ = std::fs::rename(&tmp, &out_path);
    Some(out_path.to_string_lossy().into_owned())
}

/// Decode raw image / animated GIF / animated WebP bytes into TUI frames and `mpv` BGRA files.
pub fn decode_comment_media_bytes(key: &str, bytes: &[u8]) -> Option<DecodedCommentMedia> {
    let fmt = image::guess_format(bytes).ok()?;
    let mut frames: Vec<DynamicImage> = Vec::new();
    let mut delay_ms: u64 = 95;

    match fmt {
        ImageFormat::Gif => {
            if let Ok(dec) = image::codecs::gif::GifDecoder::new(Cursor::new(bytes)) {
                let mut iter = dec.into_frames();
                let mut idx = 0usize;
                while let Some(Ok(frame)) = iter.next() {
                    if idx == 0 {
                        let (num, den) = frame.delay().numer_denom_ms();
                        let ms = if den > 0 { (num / den) as u64 } else { 95 };
                        delay_ms = ms.clamp(45, 320);
                    }
                    frames.push(DynamicImage::ImageRgba8(frame.into_buffer()));
                    idx += 1;
                    if idx >= 24 {
                        break;
                    }
                }
            }
        }
        ImageFormat::WebP => {
            if let Ok(dec) = image::codecs::webp::WebPDecoder::new(Cursor::new(bytes)) {
                if dec.has_animation() {
                    let mut iter = dec.into_frames();
                    let mut idx = 0usize;
                    while let Some(Ok(frame)) = iter.next() {
                        if idx == 0 {
                            let (num, den) = frame.delay().numer_denom_ms();
                            let ms = if den > 0 { (num / den) as u64 } else { 95 };
                            delay_ms = ms.clamp(45, 320);
                        }
                        frames.push(DynamicImage::ImageRgba8(frame.into_buffer()));
                        idx += 1;
                        if idx >= 24 {
                            break;
                        }
                    }
                }
            }
        }
        _ => {}
    }

    if frames.is_empty() {
        let single = image::load_from_memory(bytes).ok()?;
        frames.push(single);
    }

    let (ow, oh) = frames.first()?.dimensions();
    let (overlay_w, overlay_h) = scaled_sprite_dims(ow, oh);
    let dir = comment_media_cache_dir();
    let _ = std::fs::create_dir_all(&dir);
    let stem = safe_file_stem(key);

    let mut bgra_paths = Vec::with_capacity(frames.len());
    for (idx, fr) in frames.iter().enumerate() {
        if let Some(p) = write_premultiplied_bgra_frame(&dir, &stem, idx, fr, overlay_w, overlay_h) {
            bgra_paths.push(p);
        }
    }
    if bgra_paths.is_empty() {
        return None;
    }

    Some(DecodedCommentMedia {
        key: key.to_string(),
        overlay_w,
        overlay_h,
        frame_delay_ms: delay_ms,
        frames,
        bgra_paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{codecs::gif::GifEncoder, Delay, Frame, Rgba, RgbaImage};

    #[test]
    fn test_decode_animated_gif_to_premultiplied_bgra_frames() {
        let mut buf = Vec::new();
        {
            let mut enc = GifEncoder::new(&mut buf);
            let f1 = RgbaImage::from_pixel(32, 32, Rgba([255, 0, 0, 255]));
            let f2 = RgbaImage::from_pixel(32, 32, Rgba([0, 255, 0, 128]));
            enc.encode_frame(Frame::from_parts(f1, 0, 0, Delay::from_numer_denom_ms(80, 1)))
                .unwrap();
            enc.encode_frame(Frame::from_parts(f2, 0, 0, Delay::from_numer_denom_ms(80, 1)))
                .unwrap();
        }
        let decoded = decode_comment_media_bytes("cm:test_gif_2fr", &buf).unwrap();
        assert_eq!(decoded.frames.len(), 2);
        assert_eq!(decoded.bgra_paths.len(), 2);
        assert_eq!(decoded.overlay_w, 32);
        assert_eq!(decoded.overlay_h, 32);
        let raw_len = std::fs::metadata(&decoded.bgra_paths[0]).unwrap().len();
        assert_eq!(raw_len, 32 * 32 * 4);
    }
}
