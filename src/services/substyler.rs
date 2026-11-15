//! Cinema-Grade Subtitle Styler & Signage Collision Avoidance for mpv/libass.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SubtitlePreset {
    #[default]
    StudioMinimal,
    NetflixClean,
    CinemaGold,
    OledPlate,
    Custom,
}

#[allow(dead_code)]
impl SubtitlePreset {
    pub fn label(&self) -> &'static str {
        match self {
            SubtitlePreset::StudioMinimal => "Studio Minimal",
            SubtitlePreset::NetflixClean => "Netflix Clean",
            SubtitlePreset::CinemaGold => "Cinema Gold",
            SubtitlePreset::OledPlate => "OLED Plate",
            SubtitlePreset::Custom => "Custom",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            SubtitlePreset::StudioMinimal => "Crisp sans-serif with clean border & subtle shadow.",
            SubtitlePreset::NetflixClean => "Rounded soft stroke and generous bottom margin.",
            SubtitlePreset::CinemaGold => "Warm golden subtitle ink with bold black outline.",
            SubtitlePreset::OledPlate => "Translucent dark backdrop pill for bright daytime scenes.",
            SubtitlePreset::Custom => "Custom typography settings.",
        }
    }

    pub fn cycle_next(self) -> Self {
        match self {
            SubtitlePreset::StudioMinimal => SubtitlePreset::NetflixClean,
            SubtitlePreset::NetflixClean => SubtitlePreset::CinemaGold,
            SubtitlePreset::CinemaGold => SubtitlePreset::OledPlate,
            SubtitlePreset::OledPlate => SubtitlePreset::StudioMinimal,
            SubtitlePreset::Custom => SubtitlePreset::StudioMinimal,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SubtitleStyleConfig {
    #[serde(default)]
    pub preset: SubtitlePreset,
    #[serde(default = "default_sub_font")]
    pub font_family: String,
    #[serde(default = "default_sub_font_size")]
    pub font_size: u32,
    #[serde(default = "default_sub_border_size")]
    pub border_size: f32,
    #[serde(default = "default_sub_shadow_offset")]
    pub shadow_offset: f32,
    #[serde(default = "default_sub_margin_y")]
    pub margin_y: u32,
    #[serde(default = "default_sub_color")]
    pub primary_color: String,
    #[serde(default = "default_sub_border_color")]
    pub border_color: String,
    #[serde(default = "default_sub_backdrop_opacity")]
    pub backdrop_opacity: f32,
    #[serde(default = "default_true")]
    pub collision_avoidance: bool,
}

fn default_sub_font() -> String {
    "Trebuchet MS".to_string()
}
fn default_sub_font_size() -> u32 {
    46
}
fn default_sub_border_size() -> f32 {
    2.4
}
fn default_sub_shadow_offset() -> f32 {
    1.2
}
fn default_sub_margin_y() -> u32 {
    38
}
fn default_sub_color() -> String {
    "#FFFFFF".to_string()
}
fn default_sub_border_color() -> String {
    "#000000".to_string()
}
fn default_sub_backdrop_opacity() -> f32 {
    0.0
}
fn default_true() -> bool {
    true
}

impl Default for SubtitleStyleConfig {
    fn default() -> Self {
        Self::from_preset(SubtitlePreset::StudioMinimal)
    }
}

#[allow(dead_code)]
impl SubtitleStyleConfig {
    pub fn from_preset(preset: SubtitlePreset) -> Self {
        match preset {
            SubtitlePreset::StudioMinimal => Self {
                preset,
                font_family: "Trebuchet MS".to_string(),
                font_size: 46,
                border_size: 2.4,
                shadow_offset: 1.2,
                margin_y: 38,
                primary_color: "#FFFFFF".to_string(),
                border_color: "#000000".to_string(),
                backdrop_opacity: 0.0,
                collision_avoidance: true,
            },
            SubtitlePreset::NetflixClean => Self {
                preset,
                font_family: "Helvetica Neue".to_string(),
                font_size: 44,
                border_size: 1.8,
                shadow_offset: 1.5,
                margin_y: 48,
                primary_color: "#F3F4F6".to_string(),
                border_color: "#111827".to_string(),
                backdrop_opacity: 0.0,
                collision_avoidance: true,
            },
            SubtitlePreset::CinemaGold => Self {
                preset,
                font_family: "Arial Rounded MT Bold".to_string(),
                font_size: 48,
                border_size: 3.0,
                shadow_offset: 1.5,
                margin_y: 42,
                primary_color: "#FCD34D".to_string(),
                border_color: "#000000".to_string(),
                backdrop_opacity: 0.0,
                collision_avoidance: true,
            },
            SubtitlePreset::OledPlate => Self {
                preset,
                font_family: "Inter".to_string(),
                font_size: 44,
                border_size: 1.0,
                shadow_offset: 0.0,
                margin_y: 40,
                primary_color: "#FFFFFF".to_string(),
                border_color: "#000000".to_string(),
                backdrop_opacity: 0.65,
                collision_avoidance: true,
            },
            SubtitlePreset::Custom => Self::from_preset(SubtitlePreset::StudioMinimal),
        }
    }

    pub fn cycle_preset(&mut self) {
        *self = Self::from_preset(self.preset.cycle_next());
    }

    pub fn to_mpv_flags(&self) -> Vec<String> {
        let mut flags = vec![
            format!("--sub-font={}", self.font_family),
            format!("--sub-font-size={}", self.font_size),
            format!("--sub-border-size={:.1}", self.border_size),
            format!("--sub-shadow-offset={:.1}", self.shadow_offset),
            format!("--sub-margin-y={}", self.margin_y),
            format!("--sub-color={}", self.primary_color),
            format!("--sub-border-color={}", self.border_color),
            "--sub-shadow-color=#80000000".to_string(),
        ];
        if self.backdrop_opacity > 0.05 {
            let alpha_u8 = (self.backdrop_opacity.clamp(0.0, 1.0) * 255.0).round() as u8;
            flags.push(format!("--sub-back-color=#{:02X}000000", alpha_u8));
            flags.push("--sub-border-style=background-box".to_string());
        }
        flags
    }
}

/// Apply vertical `\an8` collision avoidance to ASS events when sign text overlaps dialogue.
#[allow(dead_code)]
pub fn apply_signage_collision_avoidance(content: &str) -> String {
    if !content.contains("Dialogue:") {
        return content.to_string();
    }

    let mut sign_intervals = Vec::new();
    let mut in_events = false;
    for line in content.lines() {
        if line.starts_with("[Events]") {
            in_events = true;
            continue;
        }
        if in_events && line.starts_with("Dialogue:") {
            let parts: Vec<&str> = line.splitn(10, ',').collect();
            if parts.len() >= 10 {
                let style = parts[3].to_lowercase();
                let text = parts[9].to_lowercase();
                let is_sign = style.contains("sign")
                    || style.contains("title")
                    || text.contains("\\pos(");
                if is_sign {
                    if let (Some(s), Some(e)) = (
                        crate::services::subsync::parse_timestamp(parts[1]),
                        crate::services::subsync::parse_timestamp(parts[2]),
                    ) {
                        sign_intervals.push((s, e));
                    }
                }
            }
        }
    }

    if sign_intervals.is_empty() {
        return content.to_string();
    }

    let mut out = Vec::new();
    in_events = false;
    for line in content.lines() {
        if line.starts_with("[Events]") {
            in_events = true;
            out.push(line.to_string());
            continue;
        }
        if in_events && line.starts_with("Dialogue:") {
            let parts: Vec<&str> = line.splitn(10, ',').collect();
            if parts.len() >= 10 {
                let style = parts[3].to_lowercase();
                let text = parts[9];
                let is_dialogue =
                    !style.contains("sign") && !style.contains("title") && !text.contains("\\pos(");
                if is_dialogue {
                    if let (Some(s), Some(e)) = (
                        crate::services::subsync::parse_timestamp(parts[1]),
                        crate::services::subsync::parse_timestamp(parts[2]),
                    ) {
                        let collides = sign_intervals
                            .iter()
                            .any(|(ss, se)| s.max(*ss) < e.min(*se) && (e - s) > 0.2);
                        if collides {
                            let lifted = format!("{{\\an8}}{}", text);
                            let mut new_parts = parts;
                            new_parts[9] = &lifted;
                            out.push(new_parts.join(","));
                            continue;
                        }
                    }
                }
            }
        }
        out.push(line.to_string());
    }
    out.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_subtitle_presets_and_mpv_flags() {
        let mut cfg = SubtitleStyleConfig::default();
        assert_eq!(cfg.preset, SubtitlePreset::StudioMinimal);
        cfg.cycle_preset();
        assert_eq!(cfg.preset, SubtitlePreset::NetflixClean);
        let oled = SubtitleStyleConfig::from_preset(SubtitlePreset::OledPlate);
        let flags = oled.to_mpv_flags();
        assert!(flags.iter().any(|f| f == "--sub-border-style=background-box"));
    }
}
