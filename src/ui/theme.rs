//! Theme Engine: 6 static curated palettes + Dynamic HSV Ambilight Extractor.

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThemePalette {
    pub name: &'static str,
    pub primary: Color,      // Active borders, primary titles, focus badges
    pub secondary: Color,    // Complementary tags, badges, search highlights
    pub accent: Color,       // Ratings, duration, live statuses
    pub border_idle: Color,  // Unfocused card frames
    pub border_focus: Color, // Focused card frames
    pub text: Color,         // High-contrast primary text
    pub text_dim: Color,     // Secondary metadata
    pub divider: Color,      // Horizontal rules & subtle separators
}

pub const THEMES: [ThemePalette; 6] = [
    ThemePalette {
        name: "Tokyo Night",
        primary: Color::Rgb(125, 207, 255),
        secondary: Color::Rgb(187, 154, 247),
        accent: Color::Rgb(224, 175, 104),
        border_idle: Color::Rgb(45, 52, 75),
        border_focus: Color::Rgb(125, 207, 255),
        text: Color::Rgb(240, 244, 255),
        text_dim: Color::Rgb(130, 140, 175),
        divider: Color::Rgb(55, 62, 90),
    },
    ThemePalette {
        name: "Cyberpunk Neon",
        primary: Color::Rgb(255, 0, 127),
        secondary: Color::Rgb(0, 240, 255),
        accent: Color::Rgb(255, 230, 0),
        border_idle: Color::Rgb(50, 30, 70),
        border_focus: Color::Rgb(255, 0, 127),
        text: Color::White,
        text_dim: Color::Rgb(160, 140, 190),
        divider: Color::Rgb(65, 38, 90),
    },
    ThemePalette {
        name: "Phaenna",
        primary: Color::Rgb(255, 45, 75),
        secondary: Color::Rgb(255, 110, 130),
        accent: Color::Rgb(80, 250, 120),
        border_idle: Color::Rgb(60, 42, 50),
        border_focus: Color::Rgb(255, 45, 75),
        text: Color::White,
        text_dim: Color::Rgb(175, 160, 165),
        divider: Color::Rgb(80, 48, 58),
    },
    ThemePalette {
        name: "Anime Violet",
        primary: Color::Rgb(189, 147, 249),
        secondary: Color::Rgb(255, 121, 198),
        accent: Color::Rgb(251, 191, 36),
        border_idle: Color::Rgb(55, 45, 75),
        border_focus: Color::Rgb(189, 147, 249),
        text: Color::White,
        text_dim: Color::Rgb(165, 150, 190),
        divider: Color::Rgb(75, 58, 100),
    },
    ThemePalette {
        name: "Dracula Night",
        primary: Color::Rgb(189, 147, 249),
        secondary: Color::Rgb(139, 233, 253),
        accent: Color::Rgb(80, 250, 123),
        border_idle: Color::Rgb(68, 71, 90),
        border_focus: Color::Rgb(189, 147, 249),
        text: Color::Rgb(248, 248, 242),
        text_dim: Color::Rgb(140, 150, 185),
        divider: Color::Rgb(75, 80, 105),
    },
    ThemePalette {
        name: "Nord Frost",
        primary: Color::Rgb(136, 192, 208),
        secondary: Color::Rgb(129, 161, 193),
        accent: Color::Rgb(163, 190, 140),
        border_idle: Color::Rgb(50, 60, 75),
        border_focus: Color::Rgb(136, 192, 208),
        text: Color::Rgb(236, 239, 244),
        text_dim: Color::Rgb(145, 160, 180),
        divider: Color::Rgb(65, 78, 100),
    },
];

pub fn get_theme(index: usize) -> &'static ThemePalette {
    &THEMES[index % THEMES.len()]
}

fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let rf = r as f32 / 255.0;
    let gf = g as f32 / 255.0;
    let bf = b as f32 / 255.0;
    let max = rf.max(gf).max(bf);
    let min = rf.min(gf).min(bf);
    let d = max - min;

    let h = if d < 1e-5 {
        0.0
    } else if (max - rf).abs() < 1e-5 {
        60.0 * (((gf - bf) / d).rem_euclid(6.0))
    } else if (max - gf).abs() < 1e-5 {
        60.0 * (((bf - rf) / d) + 2.0)
    } else {
        60.0 * (((rf - gf) / d) + 4.0)
    };
    let s = if max < 1e-5 { 0.0 } else { d / max };
    (h, s, max)
}

fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let x = c * (1.0 - (((h / 60.0).rem_euclid(2.0)) - 1.0).abs());
    let m = v - c;
    let (r1, g1, b1) = match (h / 60.0).floor() as i32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((g1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
        ((b1 + m).clamp(0.0, 1.0) * 255.0).round() as u8,
    )
}

pub fn boost_vibrancy(r: u8, g: u8, b: u8, min_val: f32) -> Color {
    let (h, s, v) = rgb_to_hsv(r, g, b);
    let (nr, ng, nb) = hsv_to_rgb(h, (s * 1.18).clamp(0.42, 0.92), v.max(min_val));
    Color::Rgb(nr, ng, nb)
}

/// Extract an HSV-vibrant Ambilight border & badge palette from a video thumbnail.
pub fn extract_ambilight_palette(
    img: &image::DynamicImage,
    fallback: &ThemePalette,
) -> ThemePalette {
    let thumb = img.resize_exact(16, 9, image::imageops::FilterType::Triangle);
    let rgb = thumb.to_rgb8();

    let mut hue_buckets: [Vec<(u8, u8, u8)>; 12] = Default::default();
    for px in rgb.pixels() {
        let (h, s, v) = rgb_to_hsv(px[0], px[1], px[2]);
        if v > 0.14 && s > 0.15 {
            let bucket = ((h / 30.0).floor() as usize).min(11);
            hue_buckets[bucket].push((px[0], px[1], px[2]));
        }
    }

    let mut ranked: Vec<(usize, usize)> = hue_buckets
        .iter()
        .enumerate()
        .filter(|(_, v)| !v.is_empty())
        .map(|(i, v)| (i, v.len()))
        .collect();
    ranked.sort_by_key(|a| std::cmp::Reverse(a.1));

    let avg_rgb = |bucket: &[(u8, u8, u8)]| -> (u8, u8, u8) {
        let len = bucket.len().max(1) as u32;
        let (sr, sg, sb) = bucket.iter().fold((0u32, 0u32, 0u32), |acc, c| {
            (acc.0 + c.0 as u32, acc.1 + c.1 as u32, acc.2 + c.2 as u32)
        });
        ((sr / len) as u8, (sg / len) as u8, (sb / len) as u8)
    };

    if ranked.is_empty() {
        return *fallback;
    }

    let p_rgb = avg_rgb(&hue_buckets[ranked[0].0]);
    let primary = boost_vibrancy(p_rgb.0, p_rgb.1, p_rgb.2, 0.78);

    let secondary = if ranked.len() > 1 {
        let s_rgb = avg_rgb(&hue_buckets[ranked[1].0]);
        boost_vibrancy(s_rgb.0, s_rgb.1, s_rgb.2, 0.75)
    } else {
        fallback.secondary
    };

    let mut out = *fallback;
    out.primary = primary;
    out.border_focus = primary;
    out.secondary = secondary;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ambilight_extraction_boosts_vibrancy() {
        let mut buf = image::RgbImage::new(32, 18);
        for px in buf.pixels_mut() {
            *px = image::Rgb([30, 90, 210]);
        }
        let img = image::DynamicImage::ImageRgb8(buf);
        let pal = extract_ambilight_palette(&img, &THEMES[2]);
        assert_ne!(pal.primary, Color::Rgb(0, 0, 0));
    }
}
