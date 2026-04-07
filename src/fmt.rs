use crate::{AssistantEntry, ProjectSummary, TimeBucket};
use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, Timelike};
use std::path::PathBuf;

pub fn parse_timestamp(timestamp: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(timestamp).ok()
}

pub fn timestamp_year(timestamp: &str) -> Option<i32> {
    parse_timestamp(timestamp).map(|dt| dt.year())
}

pub fn timestamp_date_key(timestamp: &str) -> Option<String> {
    if let Some(date) = timestamp.get(..10) {
        return Some(date.to_string());
    }
    parse_timestamp(timestamp).map(|dt| dt.format("%Y-%m-%d").to_string())
}

pub fn timestamp_hour(timestamp: &str) -> Option<u8> {
    // Convert to local time so power-hour reflects the user's actual working day.
    parse_timestamp(timestamp).map(|dt| dt.with_timezone(&Local).hour() as u8)
}

pub fn weekday_from_date(date: &str) -> Option<String> {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .map(|value| value.format("%A").to_string())
}

pub fn format_hour(hour: u8) -> String {
    match hour {
        0 => "12am".to_string(),
        1..=11 => format!("{hour}am"),
        12 => "12pm".to_string(),
        _ => format!("{}pm", hour - 12),
    }
}

pub fn format_currency(value: f64) -> String {
    if value >= 1000.0 {
        format!("${}", with_grouping(value.round() as u64))
    } else if value >= 100.0 {
        format!("${value:.0}")
    } else {
        format!("${value:.2}")
    }
}

pub fn format_ratio(value: u64) -> String {
    if value == 0 {
        "N/A".to_string()
    } else {
        format!("{}:1", with_grouping(value))
    }
}

pub fn round_ratio(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        0
    } else {
        (numerator as f64 / denominator as f64).round() as u64
    }
}

pub fn format_tokens(value: u64) -> String {
    match value {
        1_000_000_000.. => format!("{:.1}B", value as f64 / 1_000_000_000.0),
        1_000_000.. => format!("{:.1}M", value as f64 / 1_000_000.0),
        1_000.. => format!("{:.1}K", value as f64 / 1_000.0),
        _ => value.to_string(),
    }
}

pub fn with_grouping(value: u64) -> String {
    let text = value.to_string();
    let mut out = String::new();
    for (idx, ch) in text.chars().rev().enumerate() {
        if idx > 0 && idx % 3 == 0 {
            out.push(',');
        }
        out.push(ch);
    }
    out.chars().rev().collect()
}

pub fn trim_text(value: &str, max: usize) -> String {
    let clean = value.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return "No prompt preview available.".to_string();
    }
    if clean.chars().count() <= max {
        return clean;
    }
    let trimmed = clean
        .chars()
        .take(max.saturating_sub(1))
        .collect::<String>();
    format!("{}…", trimmed.trim_end())
}

pub fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

pub fn project_slug(name: &str) -> String {
    let mut out = String::new();
    let mut last_dash = false;
    for ch in name.chars().flat_map(|ch| ch.to_lowercase()) {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    out.trim_matches('-').to_string()
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
}

/// Returns the busiest hour bucket across all assistant entries.
pub fn busiest_hour(entries: &[AssistantEntry]) -> Option<TimeBucket> {
    let mut counts = [0usize; 24];
    for entry in entries {
        if let Some(hour) = timestamp_hour(&entry.timestamp) {
            counts[hour as usize] += 1;
        }
    }
    let total = counts.iter().sum::<usize>();
    let (hour, count) = counts
        .iter()
        .enumerate()
        .max_by(|left, right| left.1.cmp(right.1))?;
    if *count == 0 || total == 0 {
        return None;
    }
    Some(TimeBucket {
        hour: hour as u8,
        label: format_hour(hour as u8),
        count: *count,
        share_pct: ((*count as f64 / total as f64) * 100.0).round() as u64,
    })
}

/// Returns project_breakdown sorted by output tokens with workspace-root entries
/// filtered out if any named project exists.
pub fn ranked_projects(project_breakdown: &[ProjectSummary]) -> Vec<&ProjectSummary> {
    let mut sorted: Vec<&ProjectSummary> = project_breakdown.iter().collect();
    sorted.sort_by(|a, b| b.output_tokens.cmp(&a.output_tokens));
    if sorted
        .iter()
        .any(|p| !p.name.is_empty() && p.name != "workspace root")
    {
        sorted
            .into_iter()
            .filter(|p| !p.name.is_empty() && p.name != "workspace root")
            .collect()
    } else {
        sorted
    }
}
