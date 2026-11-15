//! 7-Day Subscription Upload Radar & Premiere Timeline Service.
//! Buckets subscription videos into local-timezone Monday..=Sunday slots using SQLite tz math.

use crate::db::cache::VideoItem;
use sqlx::SqlitePool;
use std::collections::{HashMap, HashSet};

const MONTH_SHORT: [&str; 12] = [
    "JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC",
];
const MONTH_TITLE: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const WEEKDAY_SHORT: [&str; 7] = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
const WEEKDAY_FULL: [&str; 7] = [
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
    "Sunday",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CalendarFilter {
    #[default]
    All,
    UnwatchedOnly,
    LongformOnly,
}

impl CalendarFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::UnwatchedOnly,
            Self::UnwatchedOnly => Self::LongformOnly,
            Self::LongformOnly => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All Uploads",
            Self::UnwatchedOnly => "Unwatched",
            Self::LongformOnly => "Longform (15m+)",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekDayMeta {
    pub day_idx: usize,
    pub short_name: &'static str,
    pub full_name: &'static str,
    pub date_short: String,
    pub day_num: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeekBounds {
    pub week_offset: i32,
    pub start_unix: i64,
    pub end_unix: i64,
    pub days: Vec<WeekDayMeta>,
    pub range_label: String,
    pub today_day_idx: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct CalendarDayEntry {
    pub video: VideoItem,
    pub is_watched: bool,
    pub is_watch_later: bool,
    pub local_hhmm: String,
}

pub async fn local_tz_offset_secs(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar::<_, i64>(
        "SELECT CAST(strftime('%s', datetime('now', 'localtime')) - strftime('%s', 'now') AS INTEGER)",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0)
}

/// Convert days-since-Unix-epoch (0 = Thu Jan 1 1970) to `(year, month 1..=12, day 1..=31)`.
fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let z = days_since_epoch + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + if m <= 2 { 1 } else { 0 };
    (year, m as u32, d as u32)
}

/// Build Monday..=Sunday local-time week boundaries for `now_utc_unix + tz_offset`.
pub fn compute_week_bounds(now_utc_unix: i64, tz_offset_secs: i64, week_offset: i32) -> WeekBounds {
    let local_now = now_utc_unix + tz_offset_secs;
    let local_day = local_now.div_euclid(86_400);
    // 1970-01-01 was Thursday (day_of_week: Mon=0..Sun=6 -> Thu=3)
    let today_dow = (local_day + 3).rem_euclid(7) as usize;
    let monday_local_day = local_day - today_dow as i64 + (week_offset as i64) * 7;

    let mut days = Vec::with_capacity(7);
    let mut start_m = 1u32;
    let mut end_m = 1u32;
    let mut start_d = 1u32;
    let mut end_d = 1u32;
    let mut end_y = 1970i64;

    for i in 0..7 {
        let d_epoch = monday_local_day + i as i64;
        let (yr, mo, da) = civil_from_days(d_epoch);
        if i == 0 {
            start_m = mo;
            start_d = da;
        }
        if i == 6 {
            end_m = mo;
            end_d = da;
            end_y = yr;
        }
        let m_title = MONTH_TITLE[(mo as usize).saturating_sub(1).min(11)];
        days.push(WeekDayMeta {
            day_idx: i,
            short_name: WEEKDAY_SHORT[i],
            full_name: WEEKDAY_FULL[i],
            date_short: format!("{} {:02}", m_title, da),
            day_num: da,
        });
    }

    let start_local_unix = monday_local_day * 86_400;
    let end_local_unix = (monday_local_day + 7) * 86_400 - 1;
    let start_unix = start_local_unix - tz_offset_secs;
    let end_unix = end_local_unix - tz_offset_secs;

    let s_label = MONTH_SHORT[(start_m as usize).saturating_sub(1).min(11)];
    let e_label = MONTH_SHORT[(end_m as usize).saturating_sub(1).min(11)];
    let range_label = format!("{} {} – {} {}, {}", s_label, start_d, e_label, end_d, end_y);

    WeekBounds {
        week_offset,
        start_unix,
        end_unix,
        days,
        range_label,
        today_day_idx: if week_offset == 0 {
            Some(today_dow)
        } else {
            None
        },
    }
}

/// Bucket uploaded videos into the 7 days `[0..=6]` (Mon..=Sun) of `bounds`.
pub fn bucket_videos_by_week(
    videos: &[VideoItem],
    bounds: &WeekBounds,
    tz_offset_secs: i64,
    watched_ids: &HashSet<String>,
    watch_later_ids: &HashSet<String>,
    filter: CalendarFilter,
) -> HashMap<usize, Vec<CalendarDayEntry>> {
    let mut map: HashMap<usize, Vec<CalendarDayEntry>> = HashMap::new();
    for i in 0..7 {
        map.insert(i, Vec::new());
    }

    for v in videos {
        let Some(ts_i) = v.timestamp else {
            continue;
        };
        if ts_i < bounds.start_unix || ts_i > bounds.end_unix {
            continue;
        }

        let is_watched = watched_ids.contains(&v.id);
        let is_longform = v.duration.unwrap_or(0) >= 900;
        match filter {
            CalendarFilter::All => {}
            CalendarFilter::UnwatchedOnly if is_watched => continue,
            CalendarFilter::LongformOnly if !is_longform => continue,
            _ => {}
        }

        let local_ts = ts_i + tz_offset_secs;
        let day_offset = ((ts_i - bounds.start_unix).div_euclid(86_400) as usize).min(6);
        let sec_of_day = local_ts.rem_euclid(86_400);
        let hh = sec_of_day / 3_600;
        let mm = (sec_of_day % 3_600) / 60;

        map.entry(day_offset).or_default().push(CalendarDayEntry {
            video: v.clone(),
            is_watched,
            is_watch_later: watch_later_ids.contains(&v.id),
            local_hhmm: format!("{:02}:{:02}", hh, mm),
        });
    }

    for entries in map.values_mut() {
        entries.sort_by_key(|a| std::cmp::Reverse(a.video.timestamp));
    }
    map
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_week_bounds_and_bucketing() {
        // Sunday 2026-11-15 12:00:00 UTC = roughly 1794744000? Let's verify exact math cleanly via offset from fixed Monday.
        // 1970-01-05 is Monday -> day 4 -> 4 * 86400 = 345600
        let mon_mid = 345_600 + 43_200; // Mon Jan 5 1970 12:00 UTC
        let bounds = compute_week_bounds(mon_mid, 0, 0);
        assert_eq!(bounds.today_day_idx, Some(0));
        assert_eq!(bounds.days.len(), 7);
        assert_eq!(bounds.days[0].date_short, "Jan 05");
        assert_eq!(bounds.days[6].date_short, "Jan 11");

        let v = VideoItem {
            id: "cal1".to_string(),
            title: "Monday Drop".to_string(),
            timestamp: Some(mon_mid + 3600),
            duration: Some(1200),
            ..Default::default()
        };
        let buckets = bucket_videos_by_week(
            &[v],
            &bounds,
            0,
            &HashSet::new(),
            &HashSet::new(),
            CalendarFilter::All,
        );
        assert_eq!(buckets.get(&0).unwrap().len(), 1);
        assert_eq!(buckets.get(&0).unwrap()[0].local_hhmm, "13:00");
    }
}
