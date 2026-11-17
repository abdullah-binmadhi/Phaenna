//! Multi-language comment translator supporting any language name or ISO/BCP-47 code.

use crate::error::{AppError, Result};
use serde_json::Value;

pub const PRESET_LANGS: &[(&str, &str)] = &[
    ("en", "EN"),
    ("ar", "AR"),
    ("es", "ES"),
    ("fr", "FR"),
    ("de", "DE"),
    ("ja", "JA"),
    ("ko", "KO"),
    ("zh-CN", "ZH"),
    ("tr", "TR"),
    ("hi", "HI"),
    ("pt", "PT"),
    ("ru", "RU"),
    ("it", "IT"),
];

pub fn next_preset_lang(current: &str) -> String {
    let idx = PRESET_LANGS
        .iter()
        .position(|(code, _)| code.eq_ignore_ascii_case(current.trim()))
        .unwrap_or(0);
    PRESET_LANGS[(idx + 1) % PRESET_LANGS.len()].0.to_string()
}

pub fn prev_preset_lang(current: &str) -> String {
    let idx = PRESET_LANGS
        .iter()
        .position(|(code, _)| code.eq_ignore_ascii_case(current.trim()))
        .unwrap_or(0);
    PRESET_LANGS[(idx + PRESET_LANGS.len() - 1) % PRESET_LANGS.len()]
        .0
        .to_string()
}

pub fn lang_badge(code: &str) -> String {
    let clean = code.trim();
    if let Some((_, short)) = PRESET_LANGS
        .iter()
        .find(|(c, _)| c.eq_ignore_ascii_case(clean))
    {
        return (*short).to_string();
    }
    clean.to_uppercase()
}

/// Normalize natural language names (e.g., "Arabic", "Japanese") or ISO/BCP-47 tags
/// into a canonical translation target code.
pub fn normalize_target_lang(input: &str) -> String {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return "en".to_string();
    }
    let low = trimmed.to_lowercase();
    match low.as_str() {
        "english" | "eng" => "en".to_string(),
        "arabic" | "ara" => "ar".to_string(),
        "japanese" | "jpn" | "jp" => "ja".to_string(),
        "spanish" | "esp" | "espanol" | "español" => "es".to_string(),
        "french" | "fra" | "francais" | "français" => "fr".to_string(),
        "german" | "deu" | "deutsch" => "de".to_string(),
        "korean" | "kor" | "kr" => "ko".to_string(),
        "chinese" | "mandarin" | "simplified chinese" | "zh" | "zh-hans" | "cn" => {
            "zh-CN".to_string()
        }
        "traditional chinese" | "cantonese" | "zh-hant" => "zh-TW".to_string(),
        "portuguese" | "por" | "brazilian" => "pt".to_string(),
        "russian" | "rus" => "ru".to_string(),
        "turkish" | "tur" => "tr".to_string(),
        "hindi" | "hin" => "hi".to_string(),
        "italian" | "ita" => "it".to_string(),
        "dutch" | "nld" => "nl".to_string(),
        "indonesian" | "ind" => "id".to_string(),
        "polish" | "pol" => "pl".to_string(),
        "swedish" | "swe" => "sv".to_string(),
        "persian" | "farsi" | "fas" => "fa".to_string(),
        "urdu" | "urd" => "ur".to_string(),
        "thai" | "tha" => "th".to_string(),
        "vietnamese" | "vie" => "vi".to_string(),
        "ukrainian" | "ukr" => "uk".to_string(),
        _ => {
            let filtered: String = trimmed
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '-')
                .take(12)
                .collect();
            if filtered.is_empty() {
                "en".to_string()
            } else {
                filtered
            }
        }
    }
}

pub fn parse_gtx_translate_json(root: &Value) -> Option<(String, Option<String>)> {
    let arr = root.as_array()?;
    let segments = arr.first()?.as_array()?;
    let mut translated = String::new();
    for seg in segments {
        if let Some(chunk) = seg.as_array().and_then(|s| s.first()).and_then(Value::as_str) {
            translated.push_str(chunk);
        }
    }
    let clean = translated.trim().to_string();
    if clean.is_empty() {
        return None;
    }
    let detected = arr
        .get(2)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Some((clean, detected))
}

pub async fn translate_text(
    client: &reqwest::Client,
    text: &str,
    target_lang: &str,
) -> Result<(String, Option<String>)> {
    let clean_text = text.trim();
    if clean_text.is_empty() {
        return Err(AppError::Parse("Empty comment text".to_string()));
    }
    let tl = normalize_target_lang(target_lang);
    let resp = client
        .get("https://translate.googleapis.com/translate_a/single")
        .query(&[
            ("client", "gtx"),
            ("sl", "auto"),
            ("tl", tl.as_str()),
            ("dt", "t"),
            ("q", clean_text),
        ])
        .timeout(std::time::Duration::from_secs(8))
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "Translation HTTP {}",
            resp.status()
        )));
    }
    let json: Value = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(format!("Invalid translation JSON: {e}")))?;
    parse_gtx_translate_json(&json)
        .ok_or_else(|| AppError::Parse("Translation response returned no text".to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_lang_and_parse_gtx() {
        assert_eq!(normalize_target_lang("English"), "en");
        assert_eq!(normalize_target_lang("arabic"), "ar");
        assert_eq!(normalize_target_lang("Japanese"), "ja");
        assert_eq!(normalize_target_lang("Simplified Chinese"), "zh-CN");
        assert_eq!(normalize_target_lang("ko"), "ko");
        assert_eq!(lang_badge("ja"), "JA");
        assert_eq!(next_preset_lang("en"), "ar");
        assert_eq!(prev_preset_lang("en"), "it");

        let payload = serde_json::json!([
            [
                ["The manual transmission ", "マニュアル車を", null, null, 1],
                ["is amazing.", "出す凄さよ", null, null, 1]
            ],
            null,
            "ja"
        ]);
        let (out, src) = parse_gtx_translate_json(&payload).unwrap();
        assert_eq!(out, "The manual transmission is amazing.");
        assert_eq!(src.as_deref(), Some("ja"));
    }
}
