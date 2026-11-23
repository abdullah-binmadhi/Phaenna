//! Y2K-era anime portrait pack generator for `mpv` `overlay-add` OSD loading & buffering screens.
//! Generates 11 unique 460x460 premultiplied BGRA CRT-styled anime portraits once into cache so
//! initial launch and every mid-video buffer event cycle cleanly through all 11 unique cards.

use image::{imageops, Rgba, RgbaImage};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

/// Side length in pixels for each square Y2K portrait overlay (`460x460` BGRA).
pub const Y2K_PORTRAIT_DIM: u32 = 460;

const Y2K_ASSETS: [(&[u8], [u8; 3], &str); 11] = [
    (
        include_bytes!("../../assets/loading/y2k_0.jpg"),
        [110, 231, 135],
        "CRT PHOSPHOR",
    ),
    (
        include_bytes!("../../assets/loading/y2k_1.jpg"),
        [96, 165, 250],
        "CYBER AUDIO",
    ),
    (
        include_bytes!("../../assets/loading/y2k_2.jpg"),
        [229, 231, 235],
        "INK SKETCH",
    ),
    (
        include_bytes!("../../assets/loading/y2k_3.jpg"),
        [165, 180, 252],
        "MIDNIGHT DECK",
    ),
    (
        include_bytes!("../../assets/loading/y2k_4.jpg"),
        [134, 239, 172],
        "MINT STAR",
    ),
    (
        include_bytes!("../../assets/loading/y2k_5.jpg"),
        [129, 140, 248],
        "NEON CAT",
    ),
    (
        include_bytes!("../../assets/loading/y2k_6.jpg"),
        [203, 213, 225],
        "HALFTONE STAR",
    ),
    (
        include_bytes!("../../assets/loading/y2k_7.jpg"),
        [192, 132, 252],
        "CYBER VIOLET",
    ),
    (
        include_bytes!("../../assets/loading/y2k_8.jpg"),
        [94, 234, 212],
        "CYAN HALO",
    ),
    (
        include_bytes!("../../assets/loading/y2k_9.jpg"),
        [147, 197, 253],
        "ICE MATRIX",
    ),
    (
        include_bytes!("../../assets/loading/y2k_10.jpg"),
        [45, 212, 191],
        "TEAL SIGNAL",
    ),
];

static LOADING_VARIANT_SEQ: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Total number of bundled Y2K anime showcase portraits.
pub const Y2K_ART_COUNT: usize = Y2K_ASSETS.len();

/// Returns the next Y2K loading card variant index (`0..11`), rotating on every launch.
pub fn next_loading_variant_index() -> usize {
    let cur = LOADING_VARIANT_SEQ.load(Ordering::Relaxed);
    if cur == usize::MAX {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos() as usize)
            .unwrap_or(0)
            % Y2K_ASSETS.len();
        let _ = LOADING_VARIANT_SEQ.compare_exchange(
            usize::MAX,
            seed,
            Ordering::Relaxed,
            Ordering::Relaxed,
        );
    }
    LOADING_VARIANT_SEQ.fetch_add(1, Ordering::Relaxed) % Y2K_ASSETS.len()
}

/// Ensures all 11 unique `460x460` premultiplied BGRA Y2K anime portraits exist in cache
/// (`y2k_0.bgra` .. `y2k_10.bgra`) and returns their parent directory for `mpv` `overlay-add`.
pub fn ensure_y2k_portrait_pack() -> Option<PathBuf> {
    let dir = crate::config::Config::cache_dir()
        .map(|d| d.join("loading").join("portraits_v1"))
        .unwrap_or_else(|_| {
            std::env::temp_dir()
                .join("phaenna-loading")
                .join("portraits_v1")
        });
    std::fs::create_dir_all(&dir).ok()?;

    let expected_bytes = (Y2K_PORTRAIT_DIM * Y2K_PORTRAIT_DIM * 4) as u64;
    for (idx, (bytes, accent_rgb, _tag)) in Y2K_ASSETS.iter().enumerate() {
        let out_path = dir.join(format!("y2k_{idx}.bgra"));
        let already_valid = std::fs::metadata(&out_path)
            .map(|m| m.is_file() && m.len() == expected_bytes)
            .unwrap_or(false);
        if already_valid {
            continue;
        }

        let src = image::load_from_memory(bytes).ok()?;
        let mut portrait = src
            .resize_to_fill(
                Y2K_PORTRAIT_DIM,
                Y2K_PORTRAIT_DIM,
                imageops::FilterType::CatmullRom,
            )
            .to_rgba8();

        // Subtle every-3-pixel Y2K CRT scanline pass
        for y in (0..Y2K_PORTRAIT_DIM).step_by(3) {
            for x in 0..Y2K_PORTRAIT_DIM {
                let px = portrait.get_pixel_mut(x, y);
                px[0] = ((px[0] as u16 * 84) / 100) as u8;
                px[1] = ((px[1] as u16 * 84) / 100) as u8;
                px[2] = ((px[2] as u16 * 84) / 100) as u8;
                px[3] = 255;
            }
        }

        let accent = Rgba([accent_rgb[0], accent_rgb[1], accent_rgb[2], 255]);
        let inner_dark = Rgba([12, 14, 22, 255]);
        let white = Rgba([244, 246, 252, 255]);

        draw_rect_outline(
            &mut portrait,
            0,
            0,
            Y2K_PORTRAIT_DIM,
            Y2K_PORTRAIT_DIM,
            4,
            accent,
        );
        draw_rect_outline(
            &mut portrait,
            4,
            4,
            Y2K_PORTRAIT_DIM - 8,
            Y2K_PORTRAIT_DIM - 8,
            2,
            inner_dark,
        );
        draw_y2k_star(&mut portrait, Y2K_PORTRAIT_DIM - 24, 24, 12, accent);
        draw_y2k_star(&mut portrait, 24, Y2K_PORTRAIT_DIM - 24, 9, white);

        let mut bgra = Vec::with_capacity(expected_bytes as usize);
        for px in portrait.pixels() {
            bgra.push(px[2]);
            bgra.push(px[1]);
            bgra.push(px[0]);
            bgra.push(255);
        }

        let tmp_path = dir.join(format!("y2k_{idx}.bgra.tmp"));
        std::fs::write(&tmp_path, &bgra).ok()?;
        let _ = std::fs::rename(&tmp_path, &out_path);
    }

    Some(dir)
}

fn fill_rect(img: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, color: Rgba<u8>) {
    let (mw, mh) = img.dimensions();
    for py in y..y.saturating_add(h).min(mh) {
        for px in x..x.saturating_add(w).min(mw) {
            img.put_pixel(px, py, color);
        }
    }
}

fn draw_rect_outline(
    img: &mut RgbaImage,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    thickness: u32,
    color: Rgba<u8>,
) {
    fill_rect(img, x, y, w, thickness, color);
    fill_rect(img, x, y + h.saturating_sub(thickness), w, thickness, color);
    fill_rect(img, x, y, thickness, h, color);
    fill_rect(img, x + w.saturating_sub(thickness), y, thickness, h, color);
}

fn draw_y2k_star(img: &mut RgbaImage, cx: u32, cy: u32, radius: i32, color: Rgba<u8>) {
    let (mw, mh) = img.dimensions();
    for dy in -radius..=radius {
        for dx in -radius..=radius {
            let ax = dx.abs();
            let ay = dy.abs();
            if (ax * ay <= (radius / 2).max(1)) && (ax + ay <= radius) {
                let px = cx as i32 + dx;
                let py = cy as i32 + dy;
                if px >= 0 && py >= 0 && (px as u32) < mw && (py as u32) < mh {
                    img.put_pixel(px as u32, py as u32, color);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_next_loading_variant_cycles_all_eleven_themes() {
        let mut seen = Vec::new();
        for _ in 0..Y2K_ASSETS.len() {
            seen.push(next_loading_variant_index());
        }
        seen.sort();
        assert_eq!(seen, (0..Y2K_ASSETS.len()).collect::<Vec<_>>());
    }

    #[test]
    fn test_ensure_y2k_portrait_pack_creates_all_eleven_bgra_portraits() {
        let dir = ensure_y2k_portrait_pack().expect("should create BGRA Y2K portrait pack");
        let expected_len = (Y2K_PORTRAIT_DIM * Y2K_PORTRAIT_DIM * 4) as usize;
        for idx in 0..Y2K_ART_COUNT {
            let path = dir.join(format!("y2k_{idx}.bgra"));
            assert!(path.is_file(), "missing {}", path.display());
            let data = std::fs::read(&path).expect("should read BGRA portrait");
            assert_eq!(data.len(), expected_len);
            assert_eq!(data[3], 255);
            assert_eq!(data[expected_len - 1], 255);
        }
    }
}
