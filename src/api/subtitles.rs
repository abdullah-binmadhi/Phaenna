//! Subtitle track extraction and downloading for YouTube videos.

use crate::error::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SubtitleTrack {
    pub code: String,
    pub label: String,
    pub url: Option<String>,
    pub is_auto: bool,
}

#[allow(dead_code)]
impl SubtitleTrack {
    pub fn display_label(&self) -> String {
        if self.is_auto {
            format!("{} (Auto)", self.label)
        } else {
            self.label.clone()
        }
    }
}

const PREFERRED_LANG_CODES: &[&str] = &[
    "en", "en-US", "en-GB", "ar", "ja", "es", "fr", "de", "pt", "ru", "ko", "zh-Hans", "hi", "id",
];

fn human_lang_name(code: &str, name_field: Option<&str>) -> String {
    if let Some(n) = name_field {
        let trimmed = n.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    match code {
        "en" | "en-US" | "en-GB" | "en-orig" => "English".to_string(),
        "ar" => "Arabic".to_string(),
        "ja" => "Japanese".to_string(),
        "es" => "Spanish".to_string(),
        "fr" => "French".to_string(),
        "de" => "German".to_string(),
        "ru" => "Russian".to_string(),
        "ko" => "Korean".to_string(),
        "pt" => "Portuguese".to_string(),
        "hi" => "Hindi".to_string(),
        "id" => "Indonesian".to_string(),
        other => other.to_string(),
    }
}

fn pick_best_format_url(entries: &[Value]) -> Option<(String, Option<String>)> {
    // Prefer vtt > srt > srv3 > ttml > anything with a url
    for ext_pref in ["vtt", "srt", "srv3", "ttml"] {
        for item in entries {
            if item.get("ext").and_then(|v| v.as_str()) == Some(ext_pref) {
                if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string());
                    return Some((url.to_string(), name));
                }
            }
        }
    }
    for item in entries {
        if let Some(url) = item.get("url").and_then(|v| v.as_str()) {
            let name = item
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            return Some((url.to_string(), name));
        }
    }
    None
}

/// Extract clean manual and curated auto-generated subtitle tracks from yt-dlp `-J` output.
pub fn parse_ytdlp_subtitles_json(val: &Value) -> Vec<SubtitleTrack> {
    let mut tracks = Vec::new();
    let mut seen_codes = std::collections::HashSet::new();

    if let Some(manual) = val.get("subtitles").and_then(|v| v.as_object()) {
        for (code, formats_val) in manual {
            if let Some(arr) = formats_val.as_array() {
                if let Some((url, name)) = pick_best_format_url(arr) {
                    let label = human_lang_name(code, name.as_deref());
                    seen_codes.insert(code.clone());
                    tracks.push(SubtitleTrack {
                        code: code.clone(),
                        label,
                        url: Some(url),
                        is_auto: false,
                    });
                }
            }
        }
    }

    if let Some(auto) = val.get("automatic_captions").and_then(|v| v.as_object()) {
        for &pref in PREFERRED_LANG_CODES {
            if seen_codes.contains(pref) {
                continue;
            }
            if let Some(arr) = auto.get(pref).and_then(|v| v.as_array()) {
                if let Some((url, name)) = pick_best_format_url(arr) {
                    let label = human_lang_name(pref, name.as_deref());
                    seen_codes.insert(pref.to_string());
                    tracks.push(SubtitleTrack {
                        code: pref.to_string(),
                        label,
                        url: Some(url),
                        is_auto: true,
                    });
                }
            }
        }
    }

    tracks.sort_by(|a, b| {
        a.is_auto
            .cmp(&b.is_auto)
            .then_with(|| a.code.cmp(&b.code))
    });
    tracks
}

#[allow(dead_code)]
pub async fn fetch_subtitle_text(client: &reqwest::Client, url: &str) -> Result<String> {
    let resp = client
        .get(url)
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await?;
    let bytes = resp.bytes().await?;
    Ok(crate::services::subsync::decode_subtitle_bytes(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ytdlp_subtitles_prefers_manual_and_vtt() {
        let json: Value = serde_json::json!({
            "subtitles": {
                "en": [
                    { "ext": "ttml", "url": "https://example.com/en.ttml", "name": "English" },
                    { "ext": "vtt",  "url": "https://example.com/en.vtt",  "name": "English" }
                ],
                "ja": [
                    { "ext": "vtt",  "url": "https://example.com/ja.vtt" }
                ]
            },
            "automatic_captions": {
                "en": [{ "ext": "vtt", "url": "https://example.com/en-auto.vtt" }],
                "ar": [{ "ext": "vtt", "url": "https://example.com/ar-auto.vtt" }],
                "zz": [{ "ext": "vtt", "url": "https://example.com/ignored.vtt" }]
            }
        });

        let tracks = parse_ytdlp_subtitles_json(&json);
        assert_eq!(tracks.len(), 3);
        assert_eq!(tracks[0].code, "en");
        assert!(!tracks[0].is_auto);
        assert_eq!(tracks[0].url.as_deref(), Some("https://example.com/en.vtt"));
        assert_eq!(tracks[1].code, "ja");
        assert_eq!(tracks[1].label, "Japanese");
        assert_eq!(tracks[2].code, "ar");
        assert!(tracks[2].is_auto);
    }
}
