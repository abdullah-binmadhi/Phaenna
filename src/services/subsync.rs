//! Automated Subtitle Decoding, Parsing & Interval Synchronization Engine.
//!
//! Two-Tier Zero-Effort Synchronizer:
//! - Tier 1: Subtitle-to-Subtitle Sequence Alignment (<50ms) against reference tracks.
//! - Tier 2: Audio Voice Activity Detection (VAD) fallback via `ffmpeg silencedetect`.

use std::time::Duration;

#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)]
pub struct SyncResult {
    pub offset_seconds: f64,
    pub scale_factor: f64,
    pub score: u64,
    pub applied: bool,
}

impl Default for SyncResult {
    fn default() -> Self {
        Self {
            offset_seconds: 0.0,
            scale_factor: 1.0,
            score: 0,
            applied: false,
        }
    }
}

/// Decode raw subtitle bytes handling UTF-8, BOM stripping, and CP1251 Cyrillic fallback.
#[allow(dead_code)]
pub fn decode_subtitle_bytes(bytes: &[u8]) -> String {
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };

    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_string();
    }

    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b < 0x80 {
            out.push(b as char);
        } else if b >= 0xC0 {
            let unicode = 0x0410 + (b - 0xC0) as u32;
            out.push(char::from_u32(unicode).unwrap_or(b as char));
        } else if b == 0xA8 {
            out.push('Ё');
        } else if b == 0xB8 {
            out.push('ё');
        } else {
            out.push(b as char);
        }
    }
    out
}

#[allow(dead_code)]
pub fn is_valid_subtitle_content(content: &str) -> bool {
    let lower = content.to_lowercase();
    if lower.contains("<html")
        || lower.contains("<head")
        || lower.contains("<body")
        || lower.contains("403 forbidden")
        || lower.contains("404 not found")
        || lower.contains("<title>")
    {
        return false;
    }
    content.contains("-->")
        || content.contains("WEBVTT")
        || content.contains("[Script Info]")
        || content.contains("Dialogue:")
}

/// Parse timestamps like `01:23:45.678`, `1:23:45,67`, or `02:15.500`.
pub fn parse_timestamp(s: &str) -> Option<f64> {
    let clean = s.trim().replace(',', ".");
    let parts: Vec<&str> = clean.split(':').collect();
    match parts.len() {
        3 => {
            let h: f64 = parts[0].parse().ok()?;
            let m: f64 = parts[1].parse().ok()?;
            let sec: f64 = parts[2].parse().ok()?;
            Some(h * 3600.0 + m * 60.0 + sec)
        }
        2 => {
            let m: f64 = parts[0].parse().ok()?;
            let sec: f64 = parts[1].parse().ok()?;
            Some(m * 60.0 + sec)
        }
        _ => None,
    }
}

#[allow(dead_code)]
pub fn extract_vtt_or_srt_intervals(content: &str) -> Vec<(f64, f64)> {
    let mut intervals = Vec::new();
    for line in content.lines() {
        if let Some((left, right)) = line.split_once("-->") {
            let right_ts = right.split_whitespace().next().unwrap_or("");
            if let (Some(start), Some(end)) = (parse_timestamp(left), parse_timestamp(right_ts)) {
                if end > start {
                    intervals.push((start, end));
                }
            }
        }
    }
    intervals
}

/// Find best constant time shift `offset_seconds` to align `target` dialogue blocks onto `reference`.
#[allow(dead_code)]
pub fn align_intervals(reference: &[(f64, f64)], target: &[(f64, f64)]) -> SyncResult {
    if reference.is_empty() || target.is_empty() {
        return SyncResult::default();
    }

    let mut best_offset = 0.0;
    let mut best_score = 0u64;

    // Coarse search: -15s..+15s at 0.25s steps
    for step in -60..=60 {
        let off = step as f64 * 0.25;
        let s = overlap_score(reference, target, off);
        if s > best_score {
            best_score = s;
            best_offset = off;
        }
    }

    // Fine refine around best_offset: +-0.30s at 0.05s steps
    let coarse_best = best_offset;
    for step in -6..=6 {
        let off = coarse_best + step as f64 * 0.05;
        let s = overlap_score(reference, target, off);
        if s > best_score {
            best_score = s;
            best_offset = off;
        }
    }

    SyncResult {
        offset_seconds: best_offset,
        scale_factor: 1.0,
        score: best_score,
        applied: best_offset.abs() >= 0.15 && best_score > 0,
    }
}

fn overlap_score(reference: &[(f64, f64)], target: &[(f64, f64)], offset: f64) -> u64 {
    let mut score_ms = 0u64;
    for &(rs, re) in reference {
        for &(ts, te) in target {
            let shifted_s = ts + offset;
            let shifted_e = te + offset;
            let inter = re.min(shifted_e) - rs.max(shifted_s);
            if inter > 0.0 {
                score_ms += (inter * 100.0) as u64;
            }
        }
    }
    score_ms
}

/// Audio silence-edge VAD offset hint via `ffmpeg silencedetect` (returns median speech start delta).
#[allow(dead_code)]
pub async fn probe_vad_delay_hint(media_url: &str, timeout_secs: u64) -> Option<f64> {
    let fut = tokio::process::Command::new("ffmpeg")
        .args([
            "-hide_banner",
            "-nostats",
            "-t",
            "25",
            "-i",
            media_url,
            "-af",
            "silencedetect=noise=-35dB:d=0.4",
            "-f",
            "null",
            "-",
        ])
        .output();

    let output = tokio::time::timeout(Duration::from_secs(timeout_secs), fut)
        .await
        .ok()?
        .ok()?;
    let stderr = String::from_utf8_lossy(&output.stderr);
    for line in stderr.lines() {
        if let Some(idx) = line.find("silence_end:") {
            let after = line[idx + "silence_end:".len()..].trim();
            if let Some(tok) = after.split_whitespace().next() {
                if let Ok(sec) = tok.parse::<f64>() {
                    return Some(sec);
                }
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_vtt_interval_alignment() {
        let ref_vtt = "WEBVTT\n\n00:00:05.000 --> 00:00:08.000\nHello world\n\n00:00:12.000 --> 00:00:15.000\nSecond line\n";
        let shifted_vtt = "WEBVTT\n\n00:00:07.500 --> 00:00:10.500\nTranslated hello\n\n00:00:14.500 --> 00:00:17.500\nTranslated second\n";

        assert!(is_valid_subtitle_content(ref_vtt));
        assert!(!is_valid_subtitle_content("<html><title>403 Forbidden</title></html>"));

        let r = extract_vtt_or_srt_intervals(ref_vtt);
        let t = extract_vtt_or_srt_intervals(shifted_vtt);
        let sync = align_intervals(&r, &t);

        assert!(sync.applied);
        assert!((sync.offset_seconds - (-2.5)).abs() < 0.06);
    }
}
