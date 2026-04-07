use chrono::{DateTime, Datelike, FixedOffset, Local, NaiveDate, Timelike};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

pub mod analyzers;
pub mod readers;
pub mod renderers;

/// Aggregated token counts across Claude Code activity.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

impl TokenUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens + self.output_tokens + self.cache_creation_tokens + self.cache_read_tokens
    }
}

impl std::ops::AddAssign<&TokenUsage> for TokenUsage {
    fn add_assign(&mut self, other: &TokenUsage) {
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cache_creation_tokens += other.cache_creation_tokens;
        self.cache_read_tokens += other.cache_read_tokens;
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelAggregate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost: f64,
    pub message_count: usize,
}

impl ModelAggregate {
    pub fn as_usage(&self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            cache_read_tokens: self.cache_read_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AssistantEntry {
    pub session_id: String,
    pub project_hash: String,
    pub is_subagent: bool,
    pub cwd: Option<String>,
    pub timestamp: String,
    pub model: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost_usd: f64,
    pub tool_names: Vec<String>,
}

impl AssistantEntry {
    pub fn usage(&self) -> TokenUsage {
        TokenUsage {
            input_tokens: self.input_tokens,
            output_tokens: self.output_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            cache_read_tokens: self.cache_read_tokens,
        }
    }
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DailyAggregate {
    pub date: String,
    pub total_cost: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub message_count: usize,
    pub session_count: usize,
    pub cache_output_ratio: u64,
    pub models: BTreeMap<String, ModelAggregate>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub hash: String,
    pub path: Option<String>,
    pub name: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub message_count: usize,
    pub session_count: usize,
    pub subagent_session_count: usize,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionPrompt {
    pub text: String,
    pub timestamp: Option<String>,
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSummary {
    pub session_id: String,
    pub timestamp_start: Option<String>,
    pub duration_minutes: u64,
    pub total_tokens: u64,
    pub usage: TokenUsage,
    pub first_prompt: Option<String>,
    pub project_path: Option<String>,
    pub project_name: Option<String>,
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub project_hash: String,
    pub project_path: Option<String>,
    pub project_name: String,
    pub timestamp_start: Option<String>,
    pub timestamp_end: Option<String>,
    pub duration_minutes: u64,
    pub usage: TokenUsage,
    pub model_totals: BTreeMap<String, TokenUsage>,
    pub total_tokens: u64,
    pub cost_usd: f64,
    pub prompt_count: usize,
    pub tool_message_count: usize,
    pub first_prompt: Option<String>,
    pub prompts: Vec<SessionPrompt>,
    pub subagents: Vec<SubagentSummary>,
}

/// Session-level summaries and notable high-cost sessions for the report.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionBreakdown {
    pub sessions: Vec<SessionSummary>,
    pub costly_sessions: Vec<SessionSummary>,
    pub costly_subagents: Vec<SubagentSummary>,
    pub total_subagent_sessions: usize,
    pub total_subagent_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CostTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelCostBreakdown {
    pub model: String,
    pub cost: f64,
    pub tokens: CostTokens,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DailyCost {
    pub date: String,
    pub cost: f64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_output_ratio: u64,
    pub message_count: usize,
    pub session_count: usize,
    pub models: Vec<ModelCostBreakdown>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionCostStats {
    pub total: usize,
    pub total_duration_minutes: u64,
    pub avg_duration_minutes: u64,
    pub longest_session_id: Option<String>,
    pub longest_session_project: Option<String>,
    pub longest_session_minutes: u64,
}

/// Aggregated cost metrics and daily usage totals for the report.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CostAnalysis {
    pub year: i32,
    pub active_days: usize,
    pub total_cost: f64,
    pub avg_daily_cost: f64,
    pub median_daily_cost: f64,
    pub peak_day: Option<DailyCost>,
    pub daily_costs: Vec<DailyCost>,
    pub model_costs: BTreeMap<String, f64>,
    pub sessions: SessionCostStats,
    pub totals: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheReason {
    pub reason: String,
    pub count: usize,
    pub percentage: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheSignals {
    pub hit_rate: u64,
    pub ratio: u64,
    pub trend: u64,
    pub breaks: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheGrade {
    pub letter: String,
    pub color: String,
    pub label: String,
    pub score: u64,
    pub signals: CacheSignals,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheSavings {
    pub from_caching: i64,
    pub wasted_from_breaks: i64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheHealth {
    pub total_cache_breaks: usize,
    pub estimated_breaks: usize,
    pub reasons_ranked: Vec<CacheReason>,
    pub cache_hit_rate: f64,
    pub efficiency_ratio: u64,
    pub grade: CacheGrade,
    pub savings: CacheSavings,
    pub totals: TokenUsage,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Anomaly {
    pub date: String,
    pub cost: f64,
    pub z_score: f64,
    pub severity: String,
    pub anomaly_type: String,
    pub avg_cost: f64,
    pub deviation: f64,
    pub cache_ratio_anomaly: bool,
    pub cache_output_ratio: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyStats {
    pub mean: f64,
    pub std_dev: f64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyReport {
    pub anomalies: Vec<Anomaly>,
    pub has_anomalies: bool,
    pub stats: AnomalyStats,
    pub trend: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TimeBucket {
    pub hour: u8,
    pub label: String,
    pub count: usize,
    pub share_pct: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionIntel {
    pub available: bool,
    pub total_sessions: usize,
    pub total_minutes: u64,
    pub avg_duration: u64,
    pub median_duration: u64,
    pub p90_duration: u64,
    pub max_duration: u64,
    pub longest_session_project: Option<String>,
    pub long_sessions: usize,
    pub long_session_pct: u64,
    pub avg_tool_messages_per_session: u64,
    pub avg_messages_per_session: u64,
    pub top_tools: Vec<ToolCount>,
    pub peak_hours: Vec<TimeBucket>,
    pub peak_overlap_pct: u64,
    pub hour_distribution: Vec<usize>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouting {
    pub available: bool,
    pub opus_pct: u64,
    pub sonnet_pct: u64,
    pub haiku_pct: u64,
    pub estimated_savings: u64,
    pub subagent_pct: u64,
    pub diversity_score: u64,
    pub tier_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub busiest_hour: Option<TimeBucket>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InflectionPoint {
    pub date: String,
    pub before_ratio: u64,
    pub after_ratio: u64,
    pub multiplier: f64,
    pub direction: String,
    pub before_days: usize,
    pub after_days: usize,
    pub summary: String,
    pub secondary: Option<Box<InflectionPoint>>,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub severity: String,
    pub title: String,
    pub savings: String,
    pub action: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeroStat {
    pub label: String,
    pub value: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoryCard {
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Highlight {
    pub eyebrow: String,
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheMood {
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptRatio {
    pub human: usize,
    pub tool: usize,
    pub total: usize,
    pub human_pct: u64,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TopTool {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TopProject {
    pub name: String,
    pub path: Option<String>,
    pub share_pct: u64,
    pub session_count: usize,
    pub output_tokens: u64,
}

/// Narrative cards and headline stats for the wrapped story view.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WrappedStory {
    pub summary: String,
    pub hero: Vec<HeroStat>,
    pub highlights: Vec<Highlight>,
    pub archetype: StoryCard,
    pub cache_mood: CacheMood,
    pub momentum: StoryCard,
    pub power_hour: Option<TimeBucket>,
    pub favorite_weekday: Option<NamedCount>,
    pub total_messages: usize,
    pub total_tokens: u64,
    pub average_messages_per_active_day: u64,
    pub longest_streak: u64,
    pub top_tool: Option<TopTool>,
    pub top_project: Option<TopProject>,
    pub biggest_session: Option<SessionSummary>,
    pub biggest_subagent: Option<SubagentSummary>,
    pub prompt_ratio: PromptRatio,
    pub next_move: Option<Recommendation>,
    pub share_text: String,
}

/// Full Claude Code wrapped output and all derived analyses.
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub generated_at: String,
    pub year: i32,
    pub cost_analysis: CostAnalysis,
    pub cache_health: CacheHealth,
    pub anomalies: AnomalyReport,
    pub inflection: Option<InflectionPoint>,
    pub session_intel: SessionIntel,
    pub session_breakdown: SessionBreakdown,
    pub model_routing: ModelRouting,
    pub project_breakdown: Vec<ProjectSummary>,
    pub recommendations: Vec<Recommendation>,
    pub wrapped_story: WrappedStory,
}

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
        format!("${:.0}", value)
    } else {
        format!("${value:.2}")
    }
}

pub fn format_currency_compact(value: f64) -> String {
    format_currency(value)
}

pub fn format_ratio(value: u64) -> String {
    if value == 0 {
        "N/A".to_string()
    } else {
        format!("{}:1", with_grouping(value))
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
