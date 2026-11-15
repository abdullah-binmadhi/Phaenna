//! SponsorBlock community segment fetcher.
//! Queries https://sponsor.ajay.app/api/skipSegments for intro, outro, sponsor,
//! self-promotion, interaction reminders, and filler segments.

use crate::config::SponsorBlockMode;
use crate::db::user::SponsorSegmentRow;
use crate::error::{AppError, Result};
use serde::{Deserialize, Serialize};

const SPONSORBLOCK_API: &str = "https://sponsor.ajay.app/api/skipSegments";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SponsorSegment {
    pub uuid: String,
    pub category: String,
    pub action_type: String,
    pub start_time: f64,
    pub end_time: f64,
    pub video_duration: f64,
}

#[derive(Debug, Deserialize)]
struct ApiSegmentPayload {
    #[serde(rename = "UUID")]
    uuid: String,
    category: String,
    #[serde(rename = "actionType", default = "default_action")]
    action_type: String,
    segment: [f64; 2],
    #[serde(rename = "videoDuration", default)]
    video_duration: f64,
}

fn default_action() -> String {
    "skip".to_string()
}

pub fn parse_sponsor_segments_json(json_str: &str) -> Result<Vec<SponsorSegment>> {
    let raw: Vec<ApiSegmentPayload> =
        serde_json::from_str(json_str).map_err(|e| AppError::Parse(e.to_string()))?;

    let mut out: Vec<SponsorSegment> = raw
        .into_iter()
        .filter_map(|p| {
            let start = p.segment[0].max(0.0);
            let end = p.segment[1];
            if !start.is_finite() || !end.is_finite() || end <= start + 0.25 {
                return None;
            }
            Some(SponsorSegment {
                uuid: p.uuid,
                category: p.category,
                action_type: p.action_type,
                start_time: start,
                end_time: end,
                video_duration: p.video_duration,
            })
        })
        .collect();

    out.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(out)
}

pub async fn fetch_sponsor_segments(
    client: &reqwest::Client,
    video_id: &str,
) -> Result<Vec<SponsorSegment>> {
    let clean_id = video_id.trim_start_matches("RD");
    if clean_id.is_empty() {
        return Ok(Vec::new());
    }

    let cats_json = r#"["sponsor","selfpromo","interaction","intro","outro","preview","music_offtopic","filler"]"#;
    let resp = client
        .get(SPONSORBLOCK_API)
        .query(&[("videoID", clean_id), ("categories", cats_json)])
        .timeout(std::time::Duration::from_secs(6))
        .send()
        .await?;

    if resp.status().as_u16() == 404 {
        return Ok(Vec::new());
    }
    if !resp.status().is_success() {
        return Err(AppError::Http(format!(
            "SponsorBlock HTTP {}",
            resp.status()
        )));
    }

    let body = resp.text().await?;
    parse_sponsor_segments_json(&body)
}

pub fn filter_segments_for_mode(
    segments: &[SponsorSegment],
    mode: SponsorBlockMode,
) -> Vec<SponsorSegment> {
    segments
        .iter()
        .filter(|seg| seg.action_type == "skip" && mode.auto_skips_category(&seg.category))
        .cloned()
        .collect()
}

pub fn to_db_rows(_video_id: &str, segments: &[SponsorSegment]) -> Vec<SponsorSegmentRow> {
    segments
        .iter()
        .map(|s| SponsorSegmentRow {
            segment_uuid: s.uuid.clone(),
            category: s.category.clone(),
            start_secs: s.start_time,
            end_secs: s.end_time,
        })
        .collect()
}

pub fn from_db_rows(rows: &[SponsorSegmentRow]) -> Vec<SponsorSegment> {
    rows.iter()
        .map(|r| SponsorSegment {
            uuid: r.segment_uuid.clone(),
            category: r.category.clone(),
            action_type: "skip".to_string(),
            start_time: r.start_secs,
            end_time: r.end_secs,
            video_duration: 0.0,
        })
        .collect()
}

pub fn category_label(category: &str) -> &'static str {
    match category {
        "sponsor" => "Sponsor",
        "selfpromo" => "Self-Promo",
        "interaction" => "Interaction Reminder",
        "intro" => "Intro / Intermission",
        "outro" => "Outro / Endcards",
        "preview" => "Preview / Recap",
        "music_offtopic" => "Non-Music Section",
        "filler" => "Filler Tangent",
        _ => "Segment",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_sponsorblock_payload_and_filter_mode() {
        let json = r#"[
            {
                "segment": [120.5, 165.0],
                "UUID": "uuid-2",
                "category": "selfpromo",
                "actionType": "skip",
                "videoDuration": 600.0
            },
            {
                "segment": [10.0, 42.2],
                "UUID": "uuid-1",
                "category": "sponsor",
                "actionType": "skip",
                "videoDuration": 600.0
            },
            {
                "segment": [50.0, 50.1],
                "UUID": "uuid-too-short",
                "category": "intro",
                "actionType": "skip"
            },
            {
                "segment": [300.0, 320.0],
                "UUID": "uuid-poi",
                "category": "poi_highlight",
                "actionType": "poi"
            }
        ]"#;

        let segs = parse_sponsor_segments_json(json).expect("should parse valid json");
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].uuid, "uuid-1"); // Sorted by start_time
        assert_eq!(segs[1].uuid, "uuid-2");

        let sponsor_only = filter_segments_for_mode(&segs, SponsorBlockMode::SponsorOnly);
        assert_eq!(sponsor_only.len(), 1);
        assert_eq!(sponsor_only[0].category, "sponsor");

        let all_skips = filter_segments_for_mode(&segs, SponsorBlockMode::AutoSkipAll);
        assert_eq!(all_skips.len(), 2); // Excludes actionType="poi"

        let none = filter_segments_for_mode(&segs, SponsorBlockMode::Off);
        assert!(none.is_empty());
    }
}
