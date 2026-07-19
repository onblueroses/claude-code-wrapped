use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

// ── Wire Types ───────────────────────────────────────────────────────────────

/// Aggregated token counts across Claude Code activity.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

impl TokenUsage {
    pub fn total_tokens(&self) -> u64 {
        self.input_tokens
            .saturating_add(self.output_tokens)
            .saturating_add(self.cache_creation_tokens)
            .saturating_add(self.cache_read_tokens)
    }
}

impl std::ops::AddAssign<&TokenUsage> for TokenUsage {
    fn add_assign(&mut self, other: &TokenUsage) {
        self.input_tokens = self.input_tokens.saturating_add(other.input_tokens);
        self.output_tokens = self.output_tokens.saturating_add(other.output_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(other.cache_creation_tokens);
        self.cache_read_tokens = self
            .cache_read_tokens
            .saturating_add(other.cache_read_tokens);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelAggregate {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub cost: f64,
    pub message_count: usize,
    pub active_seconds: u64,
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

// ── Activity ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    pub active_seconds: u64,
    pub cache_output_ratio: u64,
    pub models: BTreeMap<String, ModelAggregate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    pub active_seconds: u64,
    pub first_seen: Option<String>,
    pub last_seen: Option<String>,
}

// ── Sessions ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionPrompt {
    pub text: String,
    pub timestamp: Option<String>,
    pub entrypoint: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SubagentSummary {
    pub session_id: String,
    pub timestamp_start: Option<String>,
    pub duration_minutes: u64,
    pub elapsed_seconds: u64,
    pub active_seconds: u64,
    pub total_tokens: u64,
    pub usage: TokenUsage,
    pub first_prompt: Option<String>,
    pub project_path: Option<String>,
    pub project_name: Option<String>,
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub session_id: String,
    pub project_hash: String,
    pub project_path: Option<String>,
    pub project_name: String,
    pub timestamp_start: Option<String>,
    pub timestamp_end: Option<String>,
    pub duration_minutes: u64,
    pub elapsed_seconds: u64,
    pub active_seconds: u64,
    pub inclusive_active_seconds: u64,
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SessionBreakdown {
    pub sessions: Vec<SessionSummary>,
    pub costly_subagents: Vec<SubagentSummary>,
    pub total_subagent_sessions: usize,
    pub total_subagent_tokens: u64,
    pub total_elapsed_seconds: u64,
    pub total_active_seconds: u64,
}

// ── Canonical methodology and metrics ──────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetricMethod {
    pub version: String,
    pub description: String,
    pub parameters: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PricingRegistryRecordMetadata {
    pub provider: String,
    pub canonical_model: String,
    pub aliases: Vec<String>,
    pub effective_start: Option<String>,
    pub effective_end: Option<String>,
    pub modifier: String,
    pub input_pico_usd_per_token: u64,
    pub output_pico_usd_per_token: u64,
    pub cache_read_pico_usd_per_token: u64,
    pub cache_write_5m_pico_usd_per_token: u64,
    pub cache_write_1h_pico_usd_per_token: u64,
    pub citation: String,
    pub access_date: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PricingRegistryMetadata {
    pub version: String,
    pub citation: String,
    pub access_date: String,
    pub selection_policy: String,
    pub records: Vec<PricingRegistryRecordMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MethodologyCatalog {
    pub timezone_database: String,
    pub methods: BTreeMap<String, MetricMethod>,
    pub pricing_registry: PricingRegistryMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenMetricValue {
    pub observed: u64,
    pub unit: String,
    pub availability: String,
    pub sample_count: usize,
    pub overflowed: bool,
    pub method_id: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenMetricSet {
    pub input: TokenMetricValue,
    pub output: TokenMetricValue,
    pub cache_creation: TokenMetricValue,
    pub cache_read: TokenMetricValue,
    pub cache_creation_5m: TokenMetricValue,
    pub cache_creation_1h: TokenMetricValue,
    pub total: TokenMetricValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamedTokenMetricSet {
    pub key: String,
    pub tokens: TokenMetricSet,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalTokenMetrics {
    pub global: TokenMetricSet,
    pub days: Vec<NamedTokenMetricSet>,
    pub models: Vec<NamedTokenMetricSet>,
    pub projects: Vec<NamedTokenMetricSet>,
    pub project_unattributed: TokenMetricSet,
    pub sessions: Vec<NamedTokenMetricSet>,
    pub unattributed: TokenMetricSet,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DailyActiveTime {
    pub date: String,
    pub active_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamedActiveTime {
    pub key: String,
    pub active_seconds: u64,
    pub inclusive_active_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ActiveTimeMetrics {
    pub method_id: String,
    pub unit: String,
    pub availability: String,
    pub interval_count: usize,
    pub threshold_seconds: u64,
    pub total_elapsed_seconds: u64,
    pub total_active_seconds: u64,
    pub main_exclusive_seconds: u64,
    pub subagent_exclusive_seconds: u64,
    pub days: Vec<DailyActiveTime>,
    pub models: Vec<NamedActiveTime>,
    pub projects: Vec<NamedActiveTime>,
    pub project_unattributed_active_seconds: u64,
    pub project_unattributed_inclusive_active_seconds: u64,
    pub sessions: Vec<NamedActiveTime>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CostMetricValue {
    pub amount_usd: Option<f64>,
    pub unit: String,
    pub availability: String,
    pub quality: String,
    pub method_id: String,
    pub source: Option<String>,
    pub sample_count: usize,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelCostEvidence {
    pub raw_model: String,
    pub provider: String,
    pub canonical_model: Option<String>,
    pub pricing_key: Option<String>,
    pub pricing_modifier: String,
    pub source_recorded_usd: Option<f64>,
    pub local_api_equivalent_usd: Option<f64>,
    pub priced_tokens: u64,
    pub unpriced_tokens: u64,
    pub priced_requests: usize,
    pub unpriced_requests: usize,
    pub requests: usize,
    pub coverage: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalCostMetrics {
    pub source_recorded: CostMetricValue,
    pub local_api_equivalent: CostMetricValue,
    pub billing_authoritative: CostMetricValue,
    pub coverage: String,
    pub priced_tokens: u64,
    pub priced_tokens_overflowed: bool,
    pub unpriced_tokens: u64,
    pub unpriced_tokens_overflowed: bool,
    pub priced_requests: usize,
    pub unpriced_requests: usize,
    pub priced_token_share_pct: Option<f64>,
    pub models: Vec<ModelCostEvidence>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RatioMetric {
    pub value_pct: Option<f64>,
    pub unit: String,
    pub numerator: u64,
    pub denominator: u64,
    pub sample_count: usize,
    pub overflowed: bool,
    pub availability: String,
    pub method_id: String,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalCacheMetrics {
    pub read_share: RatioMetric,
    pub write_share: RatioMetric,
    pub direct_compactions: u64,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MetricReconciliation {
    pub status: String,
    pub token_dimensions: BTreeMap<String, String>,
    pub active_time_dimensions: BTreeMap<String, String>,
    pub cost_domains: BTreeMap<String, String>,
    pub limitations: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CanonicalMetrics {
    pub active_time: ActiveTimeMetrics,
    pub tokens: CanonicalTokenMetrics,
    pub cost: CanonicalCostMetrics,
    pub cache: CanonicalCacheMetrics,
    pub reconciliation: MetricReconciliation,
}

// ── Explainable insights ───────────────────────────────────────────────────

/// Selected-timezone window attached to an insight fact or card.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightWindow {
    pub start: String,
    pub end: String,
    pub timezone: String,
}

/// One canonical or direct-telemetry fact supporting an insight.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightFact {
    pub id: String,
    pub metric_id: String,
    pub value: String,
    pub unit: String,
    pub method_id: String,
    pub window: InsightWindow,
    pub sample_count: usize,
    pub coverage: String,
    pub source: String,
}

/// Baseline and delta evidence for a comparative insight.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightComparison {
    pub baseline_fact_id: String,
    pub current_fact_id: String,
    pub baseline_value: String,
    pub current_value: String,
    pub absolute_delta: String,
    pub relative_delta_pct: Option<f64>,
}

/// Reversible experiment and alternatives attached to a recommendation.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightAction {
    pub experiment: String,
    pub alternative_explanations: Vec<String>,
}

/// Versioned factual, recommendation, or entertainment proof object.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightCard {
    pub id: String,
    pub version: String,
    pub family: String,
    pub class: String,
    pub title: String,
    pub finding: String,
    pub metric_id: String,
    pub comparison: Option<InsightComparison>,
    pub window: InsightWindow,
    pub sample_count: usize,
    pub minimum_sample_count: usize,
    pub method_id: String,
    pub availability: String,
    pub coverage: String,
    pub confidence: String,
    pub supporting_facts: Vec<InsightFact>,
    pub limitations: Vec<String>,
    pub action: Option<InsightAction>,
    pub privacy_class: String,
    pub renderer_priority: u32,
}

/// Availability proof for one insight family, including suppressed families.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightFamilyStatus {
    pub family: String,
    pub availability: String,
    pub required_capabilities: Vec<String>,
    pub sample_count: usize,
    pub minimum_sample_count: usize,
    pub limitations: Vec<String>,
}

/// Shared renderer input for all explainable insight families.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct InsightReport {
    pub version: String,
    pub families: Vec<InsightFamilyStatus>,
    pub cards: Vec<InsightCard>,
}

// ── Cost Analysis ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CostTokens {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelCostBreakdown {
    pub model: String,
    pub cost: f64,
    pub tokens: CostTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

// ── Cache Health ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheReason {
    pub reason: String,
    pub count: usize,
    pub percentage: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheSignals {
    pub hit_rate: u64,
    pub ratio: u64,
    pub trend: u64,
    pub breaks: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheGrade {
    pub letter: String,
    pub color: String,
    pub label: String,
    pub score: u64,
    pub signals: CacheSignals,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheSavings {
    pub from_caching: i64,
    pub wasted_from_breaks: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheHealth {
    pub estimated_breaks: usize,
    pub reasons_ranked: Vec<CacheReason>,
    pub cache_hit_rate: f64,
    pub efficiency_ratio: u64,
    pub grade: CacheGrade,
    pub savings: CacheSavings,
    pub totals: TokenUsage,
}

// ── Anomalies ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyStats {
    pub mean: f64,
    pub std_dev: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AnomalyReport {
    pub anomalies: Vec<Anomaly>,
    pub has_anomalies: bool,
    pub stats: AnomalyStats,
    pub trend: String,
}

// ── Session Intel ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TimeBucket {
    pub hour: u8,
    pub label: String,
    pub count: usize,
    pub share_pct: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ToolCount {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

// ── Model Routing ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelRouting {
    pub available: bool,
    pub method_id: String,
    pub unit: String,
    pub observations: usize,
    pub opus_pct: u64,
    pub sonnet_pct: u64,
    pub haiku_pct: u64,
    pub other_pct: u64,
    pub unknown_pct: u64,
    pub estimated_savings: f64,
    pub subagent_pct: u64,
    pub diversity_score: u64,
    pub tier_costs: BTreeMap<String, f64>,
    pub total_cost: f64,
    pub busiest_hour: Option<TimeBucket>,
}

// ── Inflection ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

// ── Recommendations ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Recommendation {
    pub severity: String,
    pub title: String,
    pub savings: String,
    pub action: String,
}

// ── Story / Narrative ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct HeroStat {
    pub label: String,
    pub value: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct StoryCard {
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Highlight {
    pub eyebrow: String,
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct NamedCount {
    pub label: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CacheMood {
    pub title: String,
    pub note: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct PromptRatio {
    pub human: usize,
    pub tool: usize,
    pub total: usize,
    pub human_pct: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TopTool {
    pub name: String,
    pub count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct TopProject {
    pub name: String,
    pub path: Option<String>,
    pub share_pct: u64,
    pub session_count: usize,
    pub output_tokens: u64,
}

/// Narrative cards and headline stats for the wrapped story view.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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
    pub biggest_session_by_cost: Option<SessionSummary>,
    pub biggest_session_by_tokens: Option<SessionSummary>,
    pub biggest_subagent: Option<SubagentSummary>,
    pub prompt_ratio: PromptRatio,
    pub next_move: Option<Recommendation>,
    pub share_text: String,
}

// ── Ingestion coverage ─────────────────────────────────────────────────────

/// A privacy-safe warning produced while discovering or ingesting local sources.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct IngestionWarning {
    pub code: String,
    pub message: String,
    pub source_alias: Option<String>,
}

/// Bounded structural evidence for an unsupported record without source values.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UnknownShapeDiagnostic {
    pub source_alias: String,
    pub adapter_version: String,
    pub file_alias: String,
    pub record_index: u64,
    pub record_kind: String,
    pub structural_fields: BTreeMap<String, String>,
    pub byte_count: usize,
}

/// Coverage for one selected source without exposing its canonical local path.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SourceCoverage {
    pub alias: String,
    pub kind: String,
    pub selection: String,
    pub files_discovered: usize,
    pub accepted_records: usize,
    pub classified_records: usize,
    pub malformed_records: usize,
    pub unsupported_records: usize,
    pub unknown_records: usize,
    pub unknown_fields: usize,
    pub filtered_records: usize,
    pub redacted_fields: usize,
    pub duplicate_records: usize,
    pub skipped_records: usize,
    pub earliest_observed_at: Option<String>,
    pub latest_observed_at: Option<String>,
    pub capabilities: BTreeMap<String, String>,
    pub completeness: String,
    pub adapter_version: String,
    pub producer_contract: Option<String>,
    pub producer_verification: Option<String>,
}

/// Privacy-safe evidence describing what the report could and could not observe.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct DataCoverage {
    pub selected_period: String,
    pub timezone: String,
    pub earliest_observed_at: Option<String>,
    pub latest_observed_at: Option<String>,
    pub observed_day_span: u64,
    pub source_root_count: usize,
    pub files_discovered: usize,
    pub accepted_records: usize,
    pub canonical_records: usize,
    pub classified_records: usize,
    pub malformed_records: usize,
    pub unsupported_records: usize,
    pub unknown_records: usize,
    pub unknown_fields: usize,
    pub filtered_records: usize,
    pub redacted_fields: usize,
    pub duplicate_records: usize,
    pub skipped_records: usize,
    pub resolved_overlap_records: usize,
    pub unresolved_overlap_records: usize,
    pub authority_excluded_records: usize,
    pub record_count_invariant: String,
    pub completeness: String,
    pub retention_caveat: String,
    pub cost_coverage: String,
    pub privacy_profile: String,
    pub authority_policy_version: String,
    pub capabilities: BTreeMap<String, String>,
    pub sources: Vec<SourceCoverage>,
    pub warnings: Vec<IngestionWarning>,
    pub unknown_shapes: Vec<UnknownShapeDiagnostic>,
}

// ── Report ───────────────────────────────────────────────────────────────────

/// Full Claude Code wrapped output and all derived analyses.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub schema_version: String,
    pub generated_at: String,
    pub year: i32,
    pub data_coverage: DataCoverage,
    pub methodology: MethodologyCatalog,
    pub canonical_metrics: CanonicalMetrics,
    pub insights: InsightReport,
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
