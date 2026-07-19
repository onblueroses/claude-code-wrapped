use crate::{AssistantEntry, ProjectSummary, TimeBucket};
use chrono::{DateTime, Datelike, FixedOffset, NaiveDate, Timelike, Utc};
use std::path::PathBuf;

pub fn parse_timestamp(timestamp: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(timestamp).ok()
}

pub fn timestamp_year(timestamp: &str) -> Option<i32> {
    parse_timestamp(timestamp).map(|dt| dt.with_timezone(&Utc).year())
}

pub fn timestamp_date_key(timestamp: &str) -> Option<String> {
    parse_timestamp(timestamp).map(|dt| dt.with_timezone(&Utc).format("%Y-%m-%d").to_string())
}

pub fn timestamp_hour(timestamp: &str) -> Option<u8> {
    // Phase 1 has no explicit timezone selector, so UTC is the only reproducible
    // compatibility contract. Phase 2 replaces this with the selected IANA zone.
    parse_timestamp(timestamp).map(|dt| dt.with_timezone(&Utc).hour() as u8)
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

pub(crate) fn canonical_local_cost(report: &crate::Report) -> Option<f64> {
    report
        .canonical_metrics
        .cost
        .local_api_equivalent
        .amount_usd
        .filter(|value| value.is_finite() && *value >= 0.0)
}

pub(crate) fn canonical_evidence_is_limited(report: &crate::Report) -> bool {
    !matches!(report.data_coverage.completeness.as_str(), "complete")
        || canonical_local_cost(report).is_none()
        || report
            .canonical_metrics
            .cache
            .read_share
            .value_pct
            .is_none()
}

pub(crate) fn canonical_ratio_display(ratio: &crate::RatioMetric) -> String {
    ratio
        .value_pct
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map_or_else(|| "Unavailable".to_string(), |value| format!("{value:.1}%"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrustProjection {
    pub profile: String,
    pub schema: String,
    pub selected_period: String,
    pub timezone: String,
    pub completeness: String,
    pub cost_coverage: String,
    pub cost_method: String,
    pub pricing_registry: String,
    pub limitations: String,
}

impl TrustProjection {
    pub fn lines(&self) -> Vec<String> {
        vec![
            format!("Trust · profile={}", self.profile),
            format!("Trust · schema={}", self.schema),
            format!(
                "Trust · period={} · timezone={}",
                self.selected_period, self.timezone
            ),
            format!("Trust · completeness={}", self.completeness),
            format!(
                "Trust · costProvenance=local API-equivalent estimate · costCoverage={} · method={} · registry={}",
                self.cost_coverage, self.cost_method, self.pricing_registry
            ),
            format!("Trust · limitations={}", self.limitations),
        ]
    }
}

pub(crate) fn trust_projection(report: &crate::Report, profile: &str) -> TrustProjection {
    let limitations = if report.data_coverage.retention_caveat.is_empty() {
        if canonical_evidence_is_limited(report) {
            crate::PARTIAL_USAGE_LIMITATION.to_string()
        } else {
            "none".to_string()
        }
    } else {
        report.data_coverage.retention_caveat.clone()
    };
    TrustProjection {
        profile: profile.to_string(),
        schema: report.schema_version.clone(),
        selected_period: report.data_coverage.selected_period.clone(),
        timezone: report.data_coverage.timezone.clone(),
        completeness: report.data_coverage.completeness.clone(),
        cost_coverage: report.data_coverage.cost_coverage.clone(),
        cost_method: report
            .canonical_metrics
            .cost
            .local_api_equivalent
            .method_id
            .clone(),
        pricing_registry: report.methodology.pricing_registry.version.clone(),
        limitations,
    }
}

pub(crate) fn experience_label(report: &crate::Report) -> &'static str {
    if report.data_coverage.completeness == "complete" {
        "Claude Code Wrapped"
    } else {
        "Claude Code Wrapped · observed activity"
    }
}

pub(crate) fn canonical_fact_lines(report: &crate::Report) -> Vec<String> {
    let period = &report.data_coverage.selected_period;
    let timezone = &report.data_coverage.timezone;
    let active = &report.canonical_metrics.active_time;
    let tokens = &report.canonical_metrics.tokens.global;
    let cost = &report.canonical_metrics.cost.local_api_equivalent;
    let read = &report.canonical_metrics.cache.read_share;
    let write = &report.canonical_metrics.cache.write_share;
    let mut lines = vec![
        format!(
            "FACT method={} metric=activity.active value={} unit={} availability={} intervals={} period={} timezone={} thresholdSeconds={} limitations={}",
            active.method_id,
            active.total_active_seconds,
            active.unit,
            active.availability,
            active.interval_count,
            period,
            timezone,
            active.threshold_seconds,
            fact_limitations(&active.limitations)
        ),
    ];
    for (name, token) in [
        ("input", &tokens.input),
        ("output", &tokens.output),
        ("cacheCreation", &tokens.cache_creation),
        ("cacheRead", &tokens.cache_read),
        ("cacheCreation5m", &tokens.cache_creation_5m),
        ("cacheCreation1h", &tokens.cache_creation_1h),
    ] {
        lines.push(format!(
            "FACT method={} metric=tokens.{} value={} unit={} availability={} samples={} overflowed={} period={} timezone={} limitations={}",
            token.method_id,
            name,
            token.observed,
            token.unit,
            token.availability,
            token.sample_count,
            token.overflowed,
            period,
            timezone,
            fact_limitations(&token.limitations)
        ));
    }
    lines.extend([
        format!(
            "FACT method={} metric=tokens.total value={} unit={} availability={} samples={} overflowed={} categories=input+output+cacheCreation+cacheRead period={} timezone={} limitations={}",
            tokens.total.method_id,
            tokens.total.observed,
            tokens.total.unit,
            tokens.total.availability,
            tokens.total.sample_count,
            tokens.total.overflowed,
            period,
            timezone,
            fact_limitations(&tokens.total.limitations)
        ),
        format!(
            "FACT method={} metric=cost.localApiEquivalent value={} unit={} availability={} samples={} period={} registry={} limitations={}",
            cost.method_id,
            cost.amount_usd
                .map_or_else(|| "unavailable".to_string(), |value| format!("{value:.6}")),
            cost.unit,
            cost.availability,
            cost.sample_count,
            period,
            report.methodology.pricing_registry.version,
            fact_limitations(&cost.limitations)
        ),
        format!(
            "FACT method={} metric=cache.readShare value={} unit={} numerator={} denominator={} availability={} samples={} overflowed={} period={} limitations={}",
            read.method_id,
            read.value_pct
                .map_or_else(|| "unavailable".to_string(), |value| format!("{value:.1}")),
            read.unit,
            read.numerator,
            read.denominator,
            read.availability,
            read.sample_count,
            read.overflowed,
            period,
            fact_limitations(&read.limitations)
        ),
        format!(
            "FACT method={} metric=cache.writeShare value={} unit={} numerator={} denominator={} availability={} samples={} overflowed={} period={} limitations={}",
            write.method_id,
            write
                .value_pct
                .map_or_else(|| "unavailable".to_string(), |value| format!("{value:.1}")),
            write.unit,
            write.numerator,
            write.denominator,
            write.availability,
            write.sample_count,
            write.overflowed,
            period,
            fact_limitations(&write.limitations)
        ),
    ]);
    lines
}

fn fact_limitations(limitations: &[String]) -> String {
    if limitations.is_empty() {
        "none".to_string()
    } else {
        limitations.join("|")
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
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            character if is_unsafe_display_character(character) => escaped.push('\u{fffd}'),
            character => escaped.push(character),
        }
    }
    escaped
}

fn is_unsafe_display_character(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
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
            counts[hour as usize] = counts[hour as usize].saturating_add(1);
        }
    }
    let total = counts.iter().copied().fold(0usize, usize::saturating_add);
    let (hour, count) = counts
        .iter()
        .enumerate()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(&left.0)))?;
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
    sorted.sort_by(|left, right| {
        right
            .output_tokens
            .cmp(&left.output_tokens)
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.hash.cmp(&right.hash))
    });
    if sorted.iter().any(|project| {
        !project.name.is_empty()
            && project.name != "workspace root"
            && project.name != "unattributed"
    }) {
        sorted
            .into_iter()
            .filter(|project| {
                !project.name.is_empty()
                    && project.name != "workspace root"
                    && project.name != "unattributed"
            })
            .collect()
    } else {
        sorted
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelRequestMixRow {
    pub label: &'static str,
    pub share_pct: u64,
}

pub(crate) fn model_request_mix_rows(routing: &crate::ModelRouting) -> [ModelRequestMixRow; 5] {
    [
        ModelRequestMixRow {
            label: "Opus",
            share_pct: routing.opus_pct,
        },
        ModelRequestMixRow {
            label: "Sonnet",
            share_pct: routing.sonnet_pct,
        },
        ModelRequestMixRow {
            label: "Haiku",
            share_pct: routing.haiku_pct,
        },
        ModelRequestMixRow {
            label: "Other mapped",
            share_pct: routing.other_pct,
        },
        ModelRequestMixRow {
            label: "Unknown",
            share_pct: routing.unknown_pct,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::{busiest_hour, timestamp_date_key, timestamp_hour, timestamp_year};
    use crate::AssistantEntry;

    #[test]
    fn timestamp_date_key_validates_and_normalizes_to_utc() {
        assert_eq!(
            timestamp_date_key("2025-12-31T23:30:00-02:00").as_deref(),
            Some("2026-01-01")
        );
        assert_eq!(timestamp_date_key("2026-99-99-not-a-timestamp"), None);
        assert_eq!(timestamp_date_key("not-a-date-with-a-long-tail"), None);
        assert_eq!(timestamp_year("2025-12-31T23:30:00-02:00"), Some(2026));
        assert_eq!(timestamp_hour("2026-04-05T09:00:00-07:00"), Some(16));
    }

    #[test]
    fn busiest_hour_breaks_equal_count_ties_by_earliest_hour() {
        let entries = [
            AssistantEntry {
                timestamp: "2026-04-05T16:00:00Z".to_string(),
                ..AssistantEntry::default()
            },
            AssistantEntry {
                timestamp: "2026-04-05T09:00:00Z".to_string(),
                ..AssistantEntry::default()
            },
        ];

        assert_eq!(busiest_hour(&entries).map(|bucket| bucket.hour), Some(9));
    }
}
