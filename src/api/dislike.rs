//! Return YouTube Dislike REST API client.
//! Queries https://returnyoutubedislikeapi.com/votes?videoId={id} asynchronously.

use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};

const RYD_ENDPOINT: &str = "https://returnyoutubedislikeapi.com/votes";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DislikeInfo {
    pub id: String,
    #[serde(default)]
    pub likes: i64,
    #[serde(default)]
    pub dislikes: i64,
    #[serde(rename = "viewCount", default)]
    pub view_count: i64,
    #[serde(default)]
    pub rating: f64,
    #[serde(default)]
    pub deleted: bool,
}

impl DislikeInfo {
    pub fn like_ratio_pct(&self) -> f64 {
        let total = self.likes + self.dislikes;
        if total <= 0 {
            return 100.0;
        }
        (self.likes as f64 / total as f64) * 100.0
    }
}

pub async fn fetch_dislikes(
    client: &reqwest::Client,
    video_id: &str,
) -> Result<Option<DislikeInfo>> {
    let clean_id = video_id.trim_start_matches("RD");
    if clean_id.is_empty() {
        return Ok(None);
    }

    let resp = client
        .get(RYD_ENDPOINT)
        .query(&[("videoId", clean_id)])
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?;

    if resp.status().as_u16() == 404 {
        return Ok(None);
    }
    if !resp.status().is_success() {
        return Err(AppError::Http(format!("RYD HTTP {}", resp.status())));
    }

    let info: DislikeInfo = resp
        .json()
        .await
        .map_err(|e| AppError::Parse(e.to_string()))?;
    Ok(Some(info))
}

pub fn format_compact_count(count: i64) -> String {
    let abs = count.unsigned_abs();
    let sign = if count < 0 { "-" } else { "" };
    if abs >= 1_000_000_000 {
        format!("{}{:.1}B", sign, abs as f64 / 1_000_000_000.0)
    } else if abs >= 1_000_000 {
        format!("{}{:.1}M", sign, abs as f64 / 1_000_000.0)
    } else if abs >= 1_000 {
        format!("{}{:.1}K", sign, abs as f64 / 1_000.0)
    } else {
        format!("{}{}", sign, abs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dislike_json_and_compact_format() {
        let raw = r#"{
            "id": "dQw4w9WgXcQ",
            "likes": 18200000,
            "dislikes": 450000,
            "viewCount": 1600000000,
            "rating": 4.9035,
            "deleted": false
        }"#;
        let info: DislikeInfo = serde_json::from_str(raw).expect("parse RYD");
        assert_eq!(info.likes, 18_200_000);
        assert_eq!(info.dislikes, 450_000);
        assert!(info.like_ratio_pct() > 97.0);
        assert_eq!(format_compact_count(info.likes), "18.2M");
        assert_eq!(format_compact_count(info.dislikes), "450.0K");
        assert_eq!(format_compact_count(info.view_count), "1.6B");
        assert_eq!(format_compact_count(42), "42");
    }
}
