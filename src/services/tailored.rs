//! Discovery Crucible (`Tailored`): Zero-AI local YouTube Taste Constellation,
//! weighted discovery scoring, and Live Vibe Tuner.

use crate::db::cache::VideoItem;
use ratatui::style::Color;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct TailoredOption {
    pub label: &'static str,
    pub detail: &'static str,
    pub symbol: &'static str,
    pub keywords_pos: &'static [(&'static str, f32)],
    pub keywords_neg: &'static [&'static str],
    pub min_duration_secs: Option<f64>,
    pub max_duration_secs: Option<f64>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct TailoredQuestion {
    pub id: usize,
    pub title: &'static str,
    pub subtitle: &'static str,
    pub symbol: &'static str,
    pub is_multi_select: bool,
    pub options: Vec<TailoredOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TailoredAnswers {
    pub choices: [usize; 5],
    pub deal_breakers: HashSet<usize>,
}

#[derive(Debug, Clone)]
pub struct Persona {
    pub title: &'static str,
    pub epithet: &'static str,
    pub primary_color: Color,
    pub accent_color: Color,
    pub gauges: [(&'static str, u8); 5],
    pub keywords: Vec<&'static str>,
}

#[derive(Debug, Clone)]
pub struct TailoredMatch {
    pub video: VideoItem,
    pub match_percentage: u8,
    pub reason_tags: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LiveTunerConfig {
    /// 0 = All Picks, 1 = Deep Dives (20m+), 2 = Quick Hits (<12m), 3 = Fresh Only
    pub discovery_tier: usize,
    pub exclude_shorts: bool,
    pub boosted_keywords: HashSet<String>,
    pub muted_keywords: HashSet<String>,
    pub dismissed_ids: HashSet<String>,
    pub anchor_video: Option<(String, String, Vec<String>)>,
}

impl LiveTunerConfig {
    pub fn tier_label(&self) -> &'static str {
        match self.discovery_tier {
            1 => "Deep Dives",
            2 => "Quick Hits",
            3 => "Fresh Today",
            _ => "All Vibes",
        }
    }

    pub fn cycle_tier(&mut self) {
        self.discovery_tier = (self.discovery_tier + 1) % 4;
    }

    pub fn clear_adjustments(&mut self) {
        self.discovery_tier = 0;
        self.exclude_shorts = true;
        self.boosted_keywords.clear();
        self.muted_keywords.clear();
        self.dismissed_ids.clear();
        self.anchor_video = None;
    }
}

pub fn build_questions() -> Vec<TailoredQuestion> {
    vec![
        TailoredQuestion {
            id: 0,
            title: "Session Pacing & Runtime",
            subtitle: "How long do you want your ideal watch session to last?",
            symbol: "󱑎",
            is_multi_select: false,
            options: vec![
                TailoredOption {
                    label: "Espresso Shots (5–15 min)",
                    detail: "Tight essays, news breakdowns, devlogs, focused clips.",
                    symbol: "⚡",
                    keywords_pos: &[("explained", 8.0), ("devlog", 7.0), ("breakdown", 7.0)],
                    keywords_neg: &["podcast", "livestream"],
                    min_duration_secs: Some(120.0),
                    max_duration_secs: Some(960.0),
                },
                TailoredOption {
                    label: "Standard Cinema (15–45 min)",
                    detail: "Video essays, documentaries, system design & teardown videos.",
                    symbol: "◈",
                    keywords_pos: &[("documentary", 10.0), ("architecture", 9.0), ("review", 7.0)],
                    keywords_neg: &[],
                    min_duration_secs: Some(900.0),
                    max_duration_secs: Some(2700.0),
                },
                TailoredOption {
                    label: "Longform Odyssey (45+ min)",
                    detail: "Full lectures, podcasts, deep retrospectives & film analyses.",
                    symbol: "∞",
                    keywords_pos: &[("full", 9.0), ("lecture", 10.0), ("podcast", 9.0), ("retrospective", 10.0)],
                    keywords_neg: &[],
                    min_duration_secs: Some(2400.0),
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "Adaptive Mix",
                    detail: "Let content signal dictate length naturally.",
                    symbol: "❖",
                    keywords_pos: &[],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
            ],
        },
        TailoredQuestion {
            id: 1,
            title: "Core Intellectual Orbit",
            subtitle: "What universe are you tuning into tonight?",
            symbol: "✦",
            is_multi_select: false,
            options: vec![
                TailoredOption {
                    label: "Systems, Rust & Engineering",
                    detail: "Compilers, Linux, low-level performance, architecture.",
                    symbol: "🦀",
                    keywords_pos: &[("rust", 14.0), ("linux", 12.0), ("systems", 11.0), ("code", 9.0), ("tui", 12.0)],
                    keywords_neg: &["prank", "drama"],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "Science, Space & History",
                    detail: "Physics, astrophysics, investigative documentaries.",
                    symbol: "🔭",
                    keywords_pos: &[("physics", 13.0), ("space", 11.0), ("science", 12.0), ("history", 11.0), ("math", 11.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "Cinema, Design & Visual Craft",
                    detail: "Color grading, film criticism, 3D motion, sound design.",
                    symbol: "🎬",
                    keywords_pos: &[("film", 12.0), ("design", 11.0), ("cinema", 12.0), ("music", 10.0), ("art", 9.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "Gaming & Esports Analysis",
                    detail: "Speedruns, game design essays, tournament strategy.",
                    symbol: "🎮",
                    keywords_pos: &[("game", 10.0), ("speedrun", 13.0), ("lore", 11.0), ("gameplay", 8.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
            ],
        },
        TailoredQuestion {
            id: 2,
            title: "Tone & Energy Profile",
            subtitle: "Quiet late-night flow or high-tempo adrenaline?",
            symbol: "◐",
            is_multi_select: false,
            options: vec![
                TailoredOption {
                    label: "Late-Night Essay / Lo-Fi Focus",
                    detail: "Calm narration, thoughtful pacing, ambient clarity.",
                    symbol: "🌙",
                    keywords_pos: &[("essay", 11.0), ("ambient", 11.0), ("jazz", 9.0), ("story", 8.0)],
                    keywords_neg: &["challenge", "24 hours"],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "Razor-Sharp Technical Bench",
                    detail: "No fluff, fast signal density, terminal & live builds.",
                    symbol: "⚙",
                    keywords_pos: &[("benchmark", 10.0), ("tutorial", 9.0), ("build", 9.0), ("guide", 8.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "High-Production Entertainment",
                    detail: "Big ideas, visual spectacle, storytelling momentum.",
                    symbol: "★",
                    keywords_pos: &[("documentary", 9.0), ("behind", 8.0), ("Live", 7.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
            ],
        },
        TailoredQuestion {
            id: 3,
            title: "Creator Ecosystem",
            subtitle: "Indie specialists or flagship studios?",
            symbol: "◎",
            is_multi_select: false,
            options: vec![
                TailoredOption {
                    label: "Indie Artisans & Hidden Gems",
                    detail: "Solo researchers, small channels with insane craft.",
                    symbol: "💎",
                    keywords_pos: &[("indie", 10.0), ("devlog", 9.0), ("experiment", 9.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "Flagship Educational Studios",
                    detail: "Polished investigative teams and conference talks.",
                    symbol: "🏛",
                    keywords_pos: &[("talk", 9.0), ("conference", 10.0), ("veritasium", 11.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
            ],
        },
        TailoredQuestion {
            id: 4,
            title: "Sound & Focus Mode",
            subtitle: "Visual cinema or background listening companion?",
            symbol: "♫",
            is_multi_select: false,
            options: vec![
                TailoredOption {
                    label: "Full-Screen Visual Feast",
                    detail: "Cinematography, diagrams, terminal screen recordings.",
                    symbol: "▣",
                    keywords_pos: &[("4k", 8.0), ("hdr", 8.0), ("visual", 9.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "Audio-First Companion",
                    detail: "Discussions, interviews, ambient soundscapes.",
                    symbol: "🎧",
                    keywords_pos: &[("podcast", 11.0), ("interview", 10.0), ("ost", 9.0), ("radio", 8.0)],
                    keywords_neg: &[],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
            ],
        },
        TailoredQuestion {
            id: 5,
            title: "Hard Filters (Deal-Breakers)",
            subtitle: "Toggle any noise you never want surfaced.",
            symbol: "✕",
            is_multi_select: true,
            options: vec![
                TailoredOption {
                    label: "No Vertical Shorts (<=60s)",
                    detail: "Keep feeds strictly horizontal & deliberate.",
                    symbol: "⊘",
                    keywords_pos: &[],
                    keywords_neg: &["#shorts", "shorts"],
                    min_duration_secs: Some(61.0),
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "No Clickbait Drama / Reactions",
                    detail: "Suppress reaction farming and sensational thumbnails.",
                    symbol: "🚫",
                    keywords_pos: &[],
                    keywords_neg: &["reacts", "reaction", "drama", "exposed", "gone wrong"],
                    min_duration_secs: None,
                    max_duration_secs: None,
                },
                TailoredOption {
                    label: "No Multi-Hour Livestreams",
                    detail: "Skip unedited 3h+ VOD dumps.",
                    symbol: "⏱",
                    keywords_pos: &[],
                    keywords_neg: &["vod", "stream archive"],
                    min_duration_secs: None,
                    max_duration_secs: Some(9000.0),
                },
            ],
        },
    ]
}

pub fn synthesize_persona(answers: &TailoredAnswers) -> Persona {
    match answers.choices[1] {
        0 => Persona {
            title: "The Systems Architect",
            epithet: "Zero-overhead signal hunter tuned to kernels, craft & clean abstractions.",
            primary_color: Color::Rgb(122, 162, 247),
            accent_color: Color::Rgb(158, 206, 106),
            gauges: [
                ("Signal", 94),
                ("Depth", 90),
                ("Pacing", 78),
                ("Visual", 74),
                ("Indie", 86),
            ],
            keywords: vec!["rust", "linux", "systems", "architecture", "tui"],
        },
        1 => Persona {
            title: "The Cosmic Polymath",
            epithet: "Drawn to mathematical elegance, physical truth, and patient storytelling.",
            primary_color: Color::Rgb(187, 154, 247),
            accent_color: Color::Rgb(125, 207, 255),
            gauges: [
                ("Signal", 92),
                ("Depth", 96),
                ("Pacing", 68),
                ("Visual", 88),
                ("Indie", 75),
            ],
            keywords: vec!["physics", "space", "math", "science", "history"],
        },
        2 => Persona {
            title: "The Nocturnal Auteur",
            epithet: "Curates visual poetry, soundscapes, and deliberate editorial craft.",
            primary_color: Color::Rgb(247, 118, 142),
            accent_color: Color::Rgb(224, 175, 104),
            gauges: [
                ("Signal", 82),
                ("Depth", 84),
                ("Pacing", 70),
                ("Visual", 97),
                ("Indie", 91),
            ],
            keywords: vec!["cinema", "design", "film", "essay", "ambient"],
        },
        _ => Persona {
            title: "The Kinetic Analyst",
            epithet: "Dissects mechanics, speedruns, and frame-perfect design decisions.",
            primary_color: Color::Rgb(0, 240, 255),
            accent_color: Color::Rgb(255, 0, 127),
            gauges: [
                ("Signal", 85),
                ("Depth", 82),
                ("Pacing", 93),
                ("Visual", 86),
                ("Indie", 88),
            ],
            keywords: vec!["game", "speedrun", "design", "retrospective", "analysis"],
        },
    }
}

/// Score a candidate pool of YouTube videos against answers + live vibe tuner adjustments.
pub fn score_discovery_pool(
    pool_videos: &[VideoItem],
    answers: &TailoredAnswers,
    tuner: &LiveTunerConfig,
) -> Vec<TailoredMatch> {
    let questions = build_questions();
    let persona = synthesize_persona(answers);
    let mut results = Vec::new();

    // Precompute creator frequency bonus if anchor is set
    let anchor_channel = tuner.anchor_video.as_ref().map(|(_, ch, _)| ch.to_lowercase());
    let anchor_words: HashSet<String> = tuner
        .anchor_video
        .as_ref()
        .map(|(_, _, kws)| kws.iter().map(|s| s.to_lowercase()).collect())
        .unwrap_or_default();

    let mut pos_weights: HashMap<String, f32> = HashMap::new();
    let mut neg_keywords: HashSet<String> = HashSet::new();
    for kw in &persona.keywords {
        pos_weights.insert((*kw).to_string(), 8.0);
    }
    for (q_idx, &choice_idx) in answers.choices.iter().enumerate() {
        if let Some(q) = questions.get(q_idx) {
            if let Some(opt) = q.options.get(choice_idx) {
                for &(kw, w) in opt.keywords_pos {
                    *pos_weights.entry(kw.to_lowercase()).or_insert(0.0) += w;
                }
                for &nkw in opt.keywords_neg {
                    neg_keywords.insert(nkw.to_lowercase());
                }
            }
        }
    }
    for kw in &tuner.boosted_keywords {
        *pos_weights.entry(kw.to_lowercase()).or_insert(0.0) += 18.0;
    }

    for v in pool_videos {
        if tuner.dismissed_ids.contains(&v.id) {
            continue;
        }
        let dur = v.duration.unwrap_or(0) as f64;
        if (tuner.exclude_shorts || answers.deal_breakers.contains(&0)) && dur > 0.0 && dur <= 60.0
        {
            continue;
        }
        if answers.deal_breakers.contains(&2) && dur > 9000.0 {
            continue;
        }
        match tuner.discovery_tier {
            1 if dur > 0.0 && dur < 1200.0 => continue,
            2 if dur > 720.0 => continue,
            _ => {}
        }

        let hay = format!(
            "{} {} {} {}",
            v.title,
            v.display_uploader(),
            v.description.as_deref().unwrap_or(""),
            v.tags.as_deref().unwrap_or(&[]).join(" ")
        )
        .to_lowercase();

        if tuner.muted_keywords.iter().any(|m| hay.contains(&m.to_lowercase())) {
            continue;
        }
        if answers.deal_breakers.contains(&1)
            && ["reacts", "reaction", "drama", "exposed", "gone wrong"]
                .iter()
                .any(|w| hay.contains(w))
        {
            continue;
        }

        let mut score: f32 = 52.0;
        let mut reasons: Vec<String> = Vec::new();

        for nkw in &neg_keywords {
            if hay.contains(nkw) {
                score -= 14.0;
            }
        }

        for (kw, w) in &pos_weights {
            if hay.contains(kw) {
                score += *w;
                if reasons.len() < 3 {
                    reasons.push(format!("#{}", kw));
                }
            }
        }

        // Duration alignment with question 0
        if let Some(opt) = questions[0].options.get(answers.choices[0]) {
            let min_ok = opt.min_duration_secs.map(|m| dur == 0.0 || dur >= m).unwrap_or(true);
            let max_ok = opt.max_duration_secs.map(|m| dur == 0.0 || dur <= m).unwrap_or(true);
            if min_ok && max_ok && dur > 60.0 {
                score += 12.0;
                reasons.push("Ideal Runtime".to_string());
            } else if !min_ok || !max_ok {
                score -= 10.0;
            }
        }

        if let Some(ref ach) = anchor_channel {
            if v.display_uploader().to_lowercase() == *ach {
                score += 22.0;
                reasons.push("Same Creator".to_string());
            }
        }
        for akw in &anchor_words {
            if hay.contains(akw) {
                score += 10.0;
            }
        }

        let pct = score.clamp(38.0, 99.0).round() as u8;
        if reasons.is_empty() {
            reasons.push(persona.title.to_string());
        }
        results.push(TailoredMatch {
            video: v.clone(),
            match_percentage: pct,
            reason_tags: reasons,
        });
    }

    results.sort_by_key(|a| std::cmp::Reverse(a.match_percentage));
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tailored_scoring_and_tuner() {
        let vids = vec![
            VideoItem {
                id: "vid1".to_string(),
                title: "Building a Fast Rust TUI Architecture".to_string(),
                uploader: Some("LowLevelDev".to_string()),
                duration: Some(1500),
                tags: Some(vec!["rust".to_string(), "systems".to_string()]),
                ..Default::default()
            },
            VideoItem {
                id: "short1".to_string(),
                title: "Quick clip #shorts".to_string(),
                uploader: Some("Random".to_string()),
                duration: Some(45),
                ..Default::default()
            },
        ];

        let mut answers = TailoredAnswers {
            choices: [1, 0, 1, 0, 0],
            ..Default::default()
        };
        answers.deal_breakers.insert(0); // Exclude shorts

        let tuner = LiveTunerConfig {
            exclude_shorts: true,
            ..Default::default()
        };
        let scored = score_discovery_pool(&vids, &answers, &tuner);
        assert_eq!(scored.len(), 1);
        assert_eq!(scored[0].video.id, "vid1");
        assert!(scored[0].match_percentage >= 80);
    }
}
