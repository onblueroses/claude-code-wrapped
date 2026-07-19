use super::pricing::{
    pico_usd_to_dollars, price_usage, registry_records, PriceResult, REGISTRY_ACCESS_DATE,
    REGISTRY_CITATION, REGISTRY_VERSION, SELECTION_POLICY,
};
use super::types::{EventKind, NormalizedEvent, TokenFacts};
use super::{TimeContext, TimeContextError};
use ccwrapped::{
    ActiveTimeMetrics, AssistantEntry, CanonicalCacheMetrics, CanonicalCostMetrics,
    CanonicalMetrics, CanonicalTokenMetrics, CostMetricValue, DailyActiveTime, DailyAggregate,
    MethodologyCatalog, MetricMethod, MetricReconciliation, ModelAggregate, ModelCostEvidence,
    NamedActiveTime, NamedTokenMetricSet, PricingRegistryMetadata, ProjectSummary, RatioMetric,
    SessionBreakdown, SessionSummary, SubagentSummary, TokenMetricSet, TokenMetricValue,
    TokenUsage,
};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Instant;

const NANOS_PER_SECOND: i128 = 1_000_000_000;
const TOKEN_METHOD: &str = "tokens/canonical-sum/v1";
const ACTIVE_METHOD: &str = "activity/capped-interval-union/v1";
const ELAPSED_METHOD: &str = "session/elapsed-span/v1";
const PERIOD_METHOD: &str = "period/local-calendar/v1";
const CACHE_READ_METHOD: &str = "cache/read-share/v1";
const CACHE_WRITE_METHOD: &str = "cache/write-share/v1";
const SOURCE_COST_METHOD: &str = "cost/source-estimate/v1";
const LOCAL_COST_METHOD: &str = "cost/api-equivalent/v1";
const BILLING_COST_METHOD: &str = "cost/billing/v1";

#[derive(Debug, Clone)]
pub(crate) struct AnalysisEntry {
    accumulator: AssistantEntry,
}

#[allow(dead_code)] // Library compatibility borrows entries; the binary consumes them.
impl AnalysisEntry {
    pub(crate) fn is_message_occurrence(&self) -> bool {
        true
    }

    /// Compatibility projection only. Canonical metrics consume optional facts before this
    /// legacy accumulator converts absent categories to numeric zero.
    pub(crate) fn observed_accumulator(&self) -> AssistantEntry {
        self.accumulator.clone()
    }

    pub(crate) fn into_observed_accumulator(self) -> AssistantEntry {
        self.accumulator
    }
}

#[derive(Debug)]
pub(super) struct CanonicalProjection {
    pub entries: Vec<AnalysisEntry>,
    pub session_breakdown: SessionBreakdown,
    pub daily: Vec<DailyAggregate>,
    pub projects: Vec<ProjectSummary>,
    pub methodology: MethodologyCatalog,
    pub metrics: CanonicalMetrics,
    pub hour_distribution: Vec<usize>,
    pub cache_ttl_composition_invalid: bool,
    pub performance: ProjectionPerformance,
    token_reconciliation: TokenReconciliationProof,
    activity_reconciliation: ActivityReconciliationProof,
    cost_reconciliation: CostReconciliationProof,
}

#[derive(Debug, Default)]
pub(super) struct ProjectionPerformance {
    pub activity_nanos: u128,
    pub tokens_nanos: u128,
    pub cost_nanos: u128,
    pub cache_nanos: u128,
    pub daily_nanos: u128,
    pub projects_nanos: u128,
    pub sessions_nanos: u128,
    pub methodology_nanos: u128,
    pub hour_distribution_nanos: u128,
    pub compatibility_entries_nanos: u128,
}

#[derive(Debug, Default)]
struct ProjectionTimers {
    activity: AtomicU64,
    tokens: AtomicU64,
    cost: AtomicU64,
    cache: AtomicU64,
    daily: AtomicU64,
    projects: AtomicU64,
    sessions: AtomicU64,
    methodology: AtomicU64,
    hour_distribution: AtomicU64,
    compatibility_entries: AtomicU64,
}

impl ProjectionTimers {
    fn record<T>(&self, timer: &AtomicU64, operation: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = operation();
        timer.store(elapsed_nanos_u64(started), Ordering::Relaxed);
        result
    }

    fn finish(&self) -> ProjectionPerformance {
        ProjectionPerformance {
            activity_nanos: u128::from(self.activity.load(Ordering::Relaxed)),
            tokens_nanos: u128::from(self.tokens.load(Ordering::Relaxed)),
            cost_nanos: u128::from(self.cost.load(Ordering::Relaxed)),
            cache_nanos: u128::from(self.cache.load(Ordering::Relaxed)),
            daily_nanos: u128::from(self.daily.load(Ordering::Relaxed)),
            projects_nanos: u128::from(self.projects.load(Ordering::Relaxed)),
            sessions_nanos: u128::from(self.sessions.load(Ordering::Relaxed)),
            methodology_nanos: u128::from(self.methodology.load(Ordering::Relaxed)),
            hour_distribution_nanos: u128::from(self.hour_distribution.load(Ordering::Relaxed)),
            compatibility_entries_nanos: u128::from(
                self.compatibility_entries.load(Ordering::Relaxed),
            ),
        }
    }
}

fn elapsed_nanos_u64(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX)
}

#[derive(Debug)]
pub(super) enum ProjectionError {
    Time(TimeContextError),
    WorkerPanic,
}

impl fmt::Display for ProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Time(error) => error.fmt(formatter),
            Self::WorkerPanic => formatter.write_str("a canonical-projection worker panicked"),
        }
    }
}

pub(super) fn build_canonical_projection(
    events: &[NormalizedEvent],
    time_context: &TimeContext,
    active_threshold_seconds: u64,
    worker_count: usize,
) -> Result<CanonicalProjection, ProjectionError> {
    let worker_count = worker_count.max(1);
    let timers = ProjectionTimers::default();
    let (activity, tokens, mut cost, early_auxiliary) = if worker_count >= 4 {
        thread::scope(|scope| {
            let activity = scope.spawn(|| {
                timers.record(&timers.activity, || {
                    analyze_activity(events, time_context, active_threshold_seconds)
                        .map_err(ProjectionError::Time)
                })
            });
            let tokens = scope
                .spawn(|| timers.record(&timers.tokens, || analyze_tokens(events, time_context)));
            let entries = scope.spawn(|| {
                timers.record(&timers.compatibility_entries, || assistant_entries(events))
            });
            let cost = timers.record(&timers.cost, || analyze_cost(events, time_context));
            let methodology = timers.record(&timers.methodology, || {
                methodology(time_context, active_threshold_seconds)
            });
            let hour_distribution = timers.record(&timers.hour_distribution, || {
                hour_distribution(events, time_context)
            });
            let activity = activity
                .join()
                .map_err(|_| ProjectionError::WorkerPanic)??;
            let tokens = tokens.join().map_err(|_| ProjectionError::WorkerPanic)?;
            let entries = entries.join().map_err(|_| ProjectionError::WorkerPanic)?;
            Ok((
                activity,
                tokens,
                cost,
                Some((methodology, hour_distribution, entries)),
            ))
        })?
    } else if worker_count == 2 {
        thread::scope(|scope| {
            let activity = scope.spawn(|| {
                timers.record(&timers.activity, || {
                    analyze_activity(events, time_context, active_threshold_seconds)
                        .map_err(ProjectionError::Time)
                })
            });
            let tokens = timers.record(&timers.tokens, || analyze_tokens(events, time_context));
            let cost = timers.record(&timers.cost, || analyze_cost(events, time_context));
            let methodology = timers.record(&timers.methodology, || {
                methodology(time_context, active_threshold_seconds)
            });
            let hour_distribution = timers.record(&timers.hour_distribution, || {
                hour_distribution(events, time_context)
            });
            let entries =
                timers.record(&timers.compatibility_entries, || assistant_entries(events));
            let activity = activity
                .join()
                .map_err(|_| ProjectionError::WorkerPanic)??;
            Ok((
                activity,
                tokens,
                cost,
                Some((methodology, hour_distribution, entries)),
            ))
        })?
    } else {
        (
            timers.record(&timers.activity, || {
                analyze_activity(events, time_context, active_threshold_seconds)
                    .map_err(ProjectionError::Time)
            })?,
            timers.record(&timers.tokens, || analyze_tokens(events, time_context)),
            timers.record(&timers.cost, || analyze_cost(events, time_context)),
            None,
        )
    };
    cost.proof.bind_canonical_tokens(&tokens);
    let (cache, daily, projects, session_breakdown) = if worker_count >= 4 {
        thread::scope(|scope| {
            let daily = scope.spawn(|| {
                timers.record(&timers.daily, || {
                    compatibility_daily(events, time_context, &activity)
                })
            });
            let projects = scope.spawn(|| {
                timers.record(&timers.projects, || {
                    compatibility_projects(events, &activity)
                })
            });
            let session_breakdown = scope
                .spawn(|| timers.record(&timers.sessions, || session_breakdown(events, &activity)));
            let cache = timers.record(&timers.cache, || analyze_cache(events, &tokens.global));
            Ok((
                cache,
                daily.join().map_err(|_| ProjectionError::WorkerPanic)?,
                projects.join().map_err(|_| ProjectionError::WorkerPanic)?,
                session_breakdown
                    .join()
                    .map_err(|_| ProjectionError::WorkerPanic)?,
            ))
        })?
    } else if worker_count >= 2 {
        thread::scope(|scope| {
            let daily = scope.spawn(|| {
                timers.record(&timers.daily, || {
                    compatibility_daily(events, time_context, &activity)
                })
            });
            let cache = timers.record(&timers.cache, || analyze_cache(events, &tokens.global));
            let projects = timers.record(&timers.projects, || {
                compatibility_projects(events, &activity)
            });
            let session_breakdown =
                timers.record(&timers.sessions, || session_breakdown(events, &activity));
            Ok((
                cache,
                daily.join().map_err(|_| ProjectionError::WorkerPanic)?,
                projects,
                session_breakdown,
            ))
        })?
    } else {
        (
            timers.record(&timers.cache, || analyze_cache(events, &tokens.global)),
            timers.record(&timers.daily, || {
                compatibility_daily(events, time_context, &activity)
            }),
            timers.record(&timers.projects, || {
                compatibility_projects(events, &activity)
            }),
            timers.record(&timers.sessions, || session_breakdown(events, &activity)),
        )
    };
    let token_reconciliation = TokenReconciliationProof::from_analysis(&tokens);
    let activity_reconciliation = ActivityReconciliationProof::from_analysis(&activity);
    let reconciliation = reconcile(&token_reconciliation, &activity_reconciliation, &cost.proof);
    let CostAnalysis {
        public: cost_metrics,
        proof: cost_reconciliation,
    } = cost;
    let cache_ttl_composition_invalid = cost_reconciliation.global.cache_ttl_composition_invalid;
    let (methodology, hour_distribution, entries) = early_auxiliary.unwrap_or_else(|| {
        (
            timers.record(&timers.methodology, || {
                methodology(time_context, active_threshold_seconds)
            }),
            timers.record(&timers.hour_distribution, || {
                hour_distribution(events, time_context)
            }),
            timers.record(&timers.compatibility_entries, || assistant_entries(events)),
        )
    });
    let performance = timers.finish();

    Ok(CanonicalProjection {
        entries,
        session_breakdown,
        daily,
        projects,
        methodology,
        metrics: CanonicalMetrics {
            active_time: activity.public,
            tokens: tokens.public,
            cost: cost_metrics,
            cache,
            reconciliation,
        },
        hour_distribution,
        cache_ttl_composition_invalid,
        performance,
        token_reconciliation,
        activity_reconciliation,
        cost_reconciliation,
    })
}

pub(super) fn assistant_entries(events: &[NormalizedEvent]) -> Vec<AnalysisEntry> {
    events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::AssistantUsage | EventKind::OtelApiRequest
            )
        })
        .map(|event| AnalysisEntry {
            accumulator: AssistantEntry {
                session_id: event.session_alias.clone(),
                project_hash: event.project_alias.clone(),
                is_subagent: event.is_subagent,
                cwd: None,
                timestamp: event.timestamp.clone(),
                model: event.model.clone().unwrap_or_else(|| "unknown".to_string()),
                input_tokens: event.tokens.input.unwrap_or_default(),
                output_tokens: event.tokens.output.unwrap_or_default(),
                cache_creation_tokens: event.tokens.cache_creation.unwrap_or_default(),
                cache_read_tokens: event.tokens.cache_read.unwrap_or_default(),
                cost_usd: event.source_cost_estimate.unwrap_or_default(),
                tool_names: event.tool_names.clone(),
            },
        })
        .collect()
}

#[derive(Debug, Clone, Default)]
struct CategoryAccumulator {
    observed: u128,
    present: usize,
    eligible: usize,
}

impl CategoryAccumulator {
    fn observe(&mut self, value: Option<u64>, eligible: bool) {
        if eligible {
            self.eligible = self.eligible.saturating_add(1);
        }
        if let Some(value) = value {
            self.present = self.present.saturating_add(1);
            self.observed = self.observed.saturating_add(u128::from(value));
        }
    }

    fn public(&self) -> TokenMetricValue {
        let overflowed = self.observed > u128::from(u64::MAX);
        let availability = if self.present == 0 {
            "unavailable"
        } else if self.present == self.eligible {
            "available"
        } else {
            "partial"
        };
        TokenMetricValue {
            observed: u64::try_from(self.observed).unwrap_or(u64::MAX),
            unit: "tokens".to_string(),
            availability: availability.to_string(),
            sample_count: self.present,
            overflowed,
            method_id: TOKEN_METHOD.to_string(),
            limitations: token_limitations(availability, self.present, self.eligible, overflowed),
        }
    }
}

fn token_limitations(
    availability: &str,
    present: usize,
    eligible: usize,
    overflowed: bool,
) -> Vec<String> {
    let mut limitations = Vec::new();
    if overflowed {
        limitations.push(
            "The observed category sum saturated at u64::MAX and is a lower bound.".to_string(),
        );
    }
    match availability {
        "unavailable" => limitations
            .push("No supported observation supplied this token category.".to_string()),
        "partial" => limitations.push(format!(
            "Only {present} of {eligible} eligible observations supplied this token category; the observed value is a partial sum."
        )),
        _ => {}
    }
    limitations
}

#[derive(Debug, Clone, Default)]
struct TokenAccumulator {
    input: CategoryAccumulator,
    output: CategoryAccumulator,
    cache_creation: CategoryAccumulator,
    cache_read: CategoryAccumulator,
    cache_creation_5m: CategoryAccumulator,
    cache_creation_1h: CategoryAccumulator,
    eligible_events: usize,
}

impl TokenAccumulator {
    fn observe(&mut self, tokens: &TokenFacts) {
        self.eligible_events = self.eligible_events.saturating_add(1);
        self.input.observe(tokens.input, true);
        self.output.observe(tokens.output, true);
        self.cache_creation.observe(tokens.cache_creation, true);
        self.cache_read.observe(tokens.cache_read, true);
        let ttl_eligible = tokens.cache_creation.is_some_and(|value| value > 0)
            || tokens.cache_creation_5m.is_some()
            || tokens.cache_creation_1h.is_some();
        self.cache_creation_5m
            .observe(tokens.cache_creation_5m, ttl_eligible);
        self.cache_creation_1h
            .observe(tokens.cache_creation_1h, ttl_eligible);
    }

    fn merge(&mut self, other: &Self) {
        merge_category(&mut self.input, &other.input);
        merge_category(&mut self.output, &other.output);
        merge_category(&mut self.cache_creation, &other.cache_creation);
        merge_category(&mut self.cache_read, &other.cache_read);
        merge_category(&mut self.cache_creation_5m, &other.cache_creation_5m);
        merge_category(&mut self.cache_creation_1h, &other.cache_creation_1h);
        self.eligible_events = self.eligible_events.saturating_add(other.eligible_events);
    }

    fn public(&self) -> TokenMetricSet {
        let input = self.input.public();
        let output = self.output.public();
        let cache_creation = self.cache_creation.public();
        let cache_read = self.cache_read.public();
        let total_observed = self
            .input
            .observed
            .saturating_add(self.output.observed)
            .saturating_add(self.cache_creation.observed)
            .saturating_add(self.cache_read.observed);
        let total_availability = if input.availability == "available"
            && output.availability == "available"
            && cache_creation.availability == "available"
            && cache_read.availability == "available"
        {
            "available"
        } else if input.availability != "unavailable"
            || output.availability != "unavailable"
            || cache_creation.availability != "unavailable"
            || cache_read.availability != "unavailable"
        {
            "partial"
        } else {
            "unavailable"
        };
        let total_overflowed = total_observed > u128::from(u64::MAX);
        let total_limitations = token_total_limitations(total_availability, total_overflowed);
        TokenMetricSet {
            input,
            output,
            cache_creation,
            cache_read,
            cache_creation_5m: self.cache_creation_5m.public(),
            cache_creation_1h: self.cache_creation_1h.public(),
            total: TokenMetricValue {
                observed: u64::try_from(total_observed).unwrap_or(u64::MAX),
                unit: "tokens".to_string(),
                availability: total_availability.to_string(),
                sample_count: self.eligible_events,
                overflowed: total_overflowed,
                method_id: TOKEN_METHOD.to_string(),
                limitations: total_limitations,
            },
        }
    }

    fn priceable_tokens(&self) -> u128 {
        [
            self.input.observed,
            self.output.observed,
            self.cache_creation.observed,
            self.cache_read.observed,
        ]
        .into_iter()
        .fold(0u128, u128::saturating_add)
    }
}

fn token_total_limitations(availability: &str, overflowed: bool) -> Vec<String> {
    let mut limitations = Vec::new();
    if overflowed {
        limitations.push(
            "The combined observed token sum saturated at u64::MAX and is a lower bound."
                .to_string(),
        );
    }
    match availability {
        "unavailable" => limitations.push(
            "No supported observation supplied any required total-token category.".to_string(),
        ),
        "partial" => limitations.push(
            "At least one required total-token category is unavailable or partial; the combined observed value is a partial sum."
                .to_string(),
        ),
        _ => {}
    }
    limitations
}

fn merge_category(target: &mut CategoryAccumulator, source: &CategoryAccumulator) {
    target.observed = target.observed.saturating_add(source.observed);
    target.present = target.present.saturating_add(source.present);
    target.eligible = target.eligible.saturating_add(source.eligible);
}

#[derive(Debug)]
struct TokenAnalysis {
    public: CanonicalTokenMetrics,
    global: TokenAccumulator,
    days: BTreeMap<String, TokenAccumulator>,
    models: BTreeMap<String, TokenAccumulator>,
    projects: BTreeMap<String, TokenAccumulator>,
    project_unattributed: TokenAccumulator,
    sessions: BTreeMap<String, TokenAccumulator>,
    unattributed: TokenAccumulator,
}

#[derive(Debug, Clone)]
struct TokenReconciliationProof {
    global: TokenAccumulator,
    days: BTreeMap<String, TokenAccumulator>,
    models: BTreeMap<String, TokenAccumulator>,
    projects: BTreeMap<String, TokenAccumulator>,
    project_unattributed: TokenAccumulator,
    sessions: BTreeMap<String, TokenAccumulator>,
    unattributed: TokenAccumulator,
}

impl TokenReconciliationProof {
    fn from_analysis(analysis: &TokenAnalysis) -> Self {
        Self {
            global: analysis.global.clone(),
            days: analysis.days.clone(),
            models: analysis.models.clone(),
            projects: analysis.projects.clone(),
            project_unattributed: analysis.project_unattributed.clone(),
            sessions: analysis.sessions.clone(),
            unattributed: analysis.unattributed.clone(),
        }
    }

    fn public(&self) -> CanonicalTokenMetrics {
        CanonicalTokenMetrics {
            global: self.global.public(),
            days: named_token_sets(&self.days),
            models: named_token_sets(&self.models),
            projects: named_token_sets(&self.projects),
            project_unattributed: self.project_unattributed.public(),
            sessions: named_token_sets(&self.sessions),
            unattributed: self.unattributed.public(),
        }
    }

    fn statuses(&self) -> BTreeMap<String, String> {
        let mut projects_plus_unattributed = TokenAccumulator::default();
        for value in self.projects.values() {
            projects_plus_unattributed.merge(value);
        }
        projects_plus_unattributed.merge(&self.project_unattributed);
        let mut sessions_plus_unattributed = TokenAccumulator::default();
        for value in self.sessions.values() {
            sessions_plus_unattributed.merge(value);
        }
        sessions_plus_unattributed.merge(&self.unattributed);
        BTreeMap::from([
            (
                "days".to_string(),
                token_dimension_status(&self.global, self.days.values()),
            ),
            (
                "models".to_string(),
                token_dimension_status(&self.global, self.models.values()),
            ),
            (
                "projects".to_string(),
                if token_accumulators_equal(&self.global, &projects_plus_unattributed) {
                    "pass"
                } else {
                    "fail"
                }
                .to_string(),
            ),
            (
                "sessionsPlusUnattributed".to_string(),
                if token_accumulators_equal(&self.global, &sessions_plus_unattributed) {
                    "pass"
                } else {
                    "fail"
                }
                .to_string(),
            ),
        ])
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct AggregateTokenFamilyKey {
    source_alias: String,
    family_key: u64,
    interval_start_nanos: Option<u64>,
    interval_end_nanos: Option<u64>,
    temporality: Option<u64>,
    project_alias: String,
    session_alias: String,
    session_identity_present: bool,
    model: Option<String>,
    is_subagent: bool,
}

#[derive(Debug, Clone)]
struct TokenObservation {
    tokens: TokenFacts,
    date: Option<String>,
    model: String,
    project: Option<String>,
    session: Option<String>,
}

impl TokenObservation {
    fn from_event(event: &NormalizedEvent, time_context: &TimeContext) -> Self {
        Self {
            tokens: event.tokens.clone(),
            date: time_context.date_key_epoch(event.epoch_nanos),
            model: event.model.as_deref().unwrap_or("unknown").to_string(),
            project: event
                .project_identity_present
                .then(|| event.project_alias.clone()),
            session: event
                .session_identity_present
                .then(|| event.session_alias.clone()),
        }
    }

    fn merge_tokens(&mut self, tokens: &TokenFacts) {
        merge_optional_token(&mut self.tokens.input, tokens.input);
        merge_optional_token(&mut self.tokens.output, tokens.output);
        merge_optional_token(&mut self.tokens.cache_creation, tokens.cache_creation);
        merge_optional_token(&mut self.tokens.cache_read, tokens.cache_read);
        merge_optional_token(&mut self.tokens.cache_creation_5m, tokens.cache_creation_5m);
        merge_optional_token(&mut self.tokens.cache_creation_1h, tokens.cache_creation_1h);
    }
}

fn merge_optional_token(target: &mut Option<u64>, value: Option<u64>) {
    if let Some(value) = value {
        *target = Some(target.unwrap_or(0).saturating_add(value));
    }
}

fn analyze_tokens(events: &[NormalizedEvent], time_context: &TimeContext) -> TokenAnalysis {
    let mut global = TokenAccumulator::default();
    let mut days = BTreeMap::<String, TokenAccumulator>::new();
    let mut models = BTreeMap::<String, TokenAccumulator>::new();
    let mut projects = BTreeMap::<String, TokenAccumulator>::new();
    let mut project_unattributed = TokenAccumulator::default();
    let mut sessions = BTreeMap::<String, TokenAccumulator>::new();
    let mut unattributed = TokenAccumulator::default();
    let mut aggregate_families = BTreeMap::<AggregateTokenFamilyKey, TokenObservation>::new();

    for event in events.iter().filter(|event| usage_event(event)) {
        if event.kind == EventKind::OtelMetric {
            let key = AggregateTokenFamilyKey {
                source_alias: event.source_alias.clone(),
                family_key: event.metric_family_key.unwrap_or(event.observation_key),
                interval_start_nanos: event.metric_interval_start_nanos,
                interval_end_nanos: event.metric_interval_end_nanos,
                temporality: event.metric_temporality,
                project_alias: event.project_alias.clone(),
                session_alias: if event.session_identity_present {
                    event.session_alias.clone()
                } else {
                    "unattributed".to_string()
                },
                session_identity_present: event.session_identity_present,
                model: event.model.clone(),
                is_subagent: event.is_subagent,
            };
            aggregate_families
                .entry(key)
                .and_modify(|observation| observation.merge_tokens(&event.tokens))
                .or_insert_with(|| TokenObservation::from_event(event, time_context));
        } else {
            observe_token_observation(
                TokenObservation::from_event(event, time_context),
                &mut global,
                &mut days,
                &mut models,
                &mut projects,
                &mut project_unattributed,
                &mut sessions,
                &mut unattributed,
            );
        }
    }
    for observation in aggregate_families.into_values() {
        observe_token_observation(
            observation,
            &mut global,
            &mut days,
            &mut models,
            &mut projects,
            &mut project_unattributed,
            &mut sessions,
            &mut unattributed,
        );
    }

    let public = CanonicalTokenMetrics {
        global: global.public(),
        days: named_token_sets(&days),
        models: named_token_sets(&models),
        projects: named_token_sets(&projects),
        project_unattributed: project_unattributed.public(),
        sessions: named_token_sets(&sessions),
        unattributed: unattributed.public(),
    };
    TokenAnalysis {
        public,
        global,
        days,
        models,
        projects,
        project_unattributed,
        sessions,
        unattributed,
    }
}

#[allow(clippy::too_many_arguments)]
fn observe_token_observation(
    observation: TokenObservation,
    global: &mut TokenAccumulator,
    days: &mut BTreeMap<String, TokenAccumulator>,
    models: &mut BTreeMap<String, TokenAccumulator>,
    projects: &mut BTreeMap<String, TokenAccumulator>,
    project_unattributed: &mut TokenAccumulator,
    sessions: &mut BTreeMap<String, TokenAccumulator>,
    unattributed: &mut TokenAccumulator,
) {
    global.observe(&observation.tokens);
    if let Some(date) = observation.date {
        days.entry(date).or_default().observe(&observation.tokens);
    }
    models
        .entry(observation.model)
        .or_default()
        .observe(&observation.tokens);
    if let Some(project) = observation.project {
        projects
            .entry(project)
            .or_default()
            .observe(&observation.tokens);
    } else {
        project_unattributed.observe(&observation.tokens);
    }
    if let Some(session) = observation.session {
        sessions
            .entry(session)
            .or_default()
            .observe(&observation.tokens);
    } else {
        unattributed.observe(&observation.tokens);
    }
}

fn named_token_sets(values: &BTreeMap<String, TokenAccumulator>) -> Vec<NamedTokenMetricSet> {
    values
        .iter()
        .map(|(key, tokens)| NamedTokenMetricSet {
            key: key.clone(),
            tokens: tokens.public(),
        })
        .collect()
}

fn usage_event(event: &NormalizedEvent) -> bool {
    matches!(
        event.kind,
        EventKind::AssistantUsage | EventKind::OtelApiRequest
    ) || (event.kind == EventKind::OtelMetric && event.tokens.richness() > 0)
}

#[derive(Debug, Clone, Copy)]
struct ActivityInterval<'a> {
    start: i128,
    end: i128,
    project: Option<&'a str>,
    session: &'a str,
    inclusive_group: &'a str,
    model: &'a str,
    is_subagent: bool,
}

#[derive(Debug, Default)]
struct ActivityNanos<'a> {
    total: u128,
    main_exclusive: u128,
    subagent_exclusive: u128,
    days: BTreeMap<String, u128>,
    models_partitioned: HashMap<&'a str, u128>,
    models_inclusive: HashMap<&'a str, u128>,
    projects_partitioned: HashMap<&'a str, u128>,
    projects_inclusive: HashMap<&'a str, u128>,
    project_unattributed_partitioned: u128,
    project_unattributed_inclusive: u128,
    sessions_partitioned: HashMap<&'a str, u128>,
    sessions_inclusive: HashMap<&'a str, u128>,
    groups_inclusive: HashMap<&'a str, u128>,
}

#[derive(Debug)]
struct ActivityAnalysis {
    public: ActiveTimeMetrics,
    session_partitioned: BTreeMap<String, u64>,
    session_inclusive: BTreeMap<String, u64>,
    group_inclusive: BTreeMap<String, u64>,
    project_partitioned: BTreeMap<String, u64>,
    days: BTreeMap<String, u64>,
    total_active_seconds: u64,
    partition_nanos: BTreeMap<&'static str, u128>,
}

#[derive(Debug, Clone)]
struct ActivityReconciliationProof {
    expected_public: ActiveTimeMetrics,
    partition_nanos: BTreeMap<&'static str, u128>,
}

impl ActivityReconciliationProof {
    fn from_analysis(analysis: &ActivityAnalysis) -> Self {
        Self {
            expected_public: analysis.public.clone(),
            partition_nanos: analysis.partition_nanos.clone(),
        }
    }

    fn statuses(&self) -> BTreeMap<String, String> {
        let global = self.partition_nanos.get("global").copied().unwrap_or(0);
        ["days", "models", "projects", "sessions"]
            .into_iter()
            .map(|key| {
                let total = self.partition_nanos.get(key).copied().unwrap_or(0);
                (
                    key.to_string(),
                    if total == global { "pass" } else { "fail" }.to_string(),
                )
            })
            .collect()
    }
}

fn analyze_activity(
    events: &[NormalizedEvent],
    time_context: &TimeContext,
    threshold_seconds: u64,
) -> Result<ActivityAnalysis, TimeContextError> {
    let mut grouped = HashMap::<(&str, &str), Vec<&NormalizedEvent>>::new();
    for event in events
        .iter()
        .filter(|event| event.kind != EventKind::OtelMetric && event.session_identity_present)
    {
        grouped
            .entry((&event.project_alias, &event.session_alias))
            .or_default()
            .push(event);
    }

    let threshold_nanos = i128::from(threshold_seconds).saturating_mul(NANOS_PER_SECOND);
    let activity_observed = events
        .iter()
        .any(|event| event.kind != EventKind::OtelMetric);
    let mut intervals = Vec::<ActivityInterval<'_>>::new();
    for values in grouped.values_mut() {
        values.sort_by(|left, right| {
            left.epoch_nanos
                .cmp(&right.epoch_nanos)
                .then_with(|| left.record_index.cmp(&right.record_index))
                .then_with(|| left.observation_key.cmp(&right.observation_key))
        });
        for pair in values.windows(2) {
            let earlier = pair[0];
            let later = pair[1];
            let end = later
                .epoch_nanos
                .min(earlier.epoch_nanos.saturating_add(threshold_nanos));
            push_activity_interval(
                &mut intervals,
                earlier.epoch_nanos,
                end,
                earlier,
                time_context,
            );
        }
    }
    for event in events
        .iter()
        .filter(|event| event.kind != EventKind::OtelMetric)
    {
        if let Some(latency_ms) = event.latency_ms.filter(|value| *value > 0.0) {
            let duration_nanos = (latency_ms.min(super::types::MAX_DIRECT_DURATION_MS)
                * 1_000_000.0)
                .round() as i128;
            push_activity_interval(
                &mut intervals,
                event.epoch_nanos.saturating_sub(duration_nanos),
                event.epoch_nanos,
                event,
                time_context,
            );
        }
    }

    let day_boundaries = time_context.local_day_boundaries()?;
    let mut edges = Vec::with_capacity(
        intervals
            .len()
            .saturating_mul(2)
            .saturating_add(day_boundaries.len()),
    );
    for (index, interval) in intervals.iter().enumerate() {
        edges.push((interval.start, true, index));
        edges.push((interval.end, false, index));
    }
    for boundary in day_boundaries {
        edges.push((boundary, false, usize::MAX));
    }
    edges.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });

    let mut active = HashSet::<usize>::new();
    let mut previous = None;
    let mut totals = ActivityNanos::default();
    let mut edge_index = 0usize;
    while edge_index < edges.len() {
        let time = edges[edge_index].0;
        if let Some(start) = previous {
            if time > start && !active.is_empty() {
                attribute_active_segment(
                    start,
                    time,
                    &active,
                    &intervals,
                    time_context,
                    &mut totals,
                );
            }
        }
        while edge_index < edges.len() && edges[edge_index].0 == time {
            let (_, starts, interval_index) = edges[edge_index];
            if interval_index != usize::MAX {
                if starts {
                    active.insert(interval_index);
                } else {
                    active.remove(&interval_index);
                }
            }
            edge_index = edge_index.saturating_add(1);
        }
        previous = Some(time);
    }

    let session_partitioned = borrowed_seconds_map(&totals.sessions_partitioned);
    let session_inclusive = borrowed_seconds_map(&totals.sessions_inclusive);
    let group_inclusive = borrowed_seconds_map(&totals.groups_inclusive);
    let project_partitioned = borrowed_seconds_map(&totals.projects_partitioned);
    let model_partitioned = borrowed_seconds_map(&totals.models_partitioned);
    let days = seconds_map(&totals.days);
    let total_active_seconds = nanos_to_seconds(totals.total);
    let public = ActiveTimeMetrics {
        method_id: ACTIVE_METHOD.to_string(),
        unit: "seconds".to_string(),
        availability: if activity_observed {
            "available"
        } else {
            "unavailable"
        }
        .to_string(),
        interval_count: intervals.len(),
        threshold_seconds,
        total_elapsed_seconds: elapsed_total(events),
        total_active_seconds,
        main_exclusive_seconds: nanos_to_seconds(totals.main_exclusive),
        subagent_exclusive_seconds: nanos_to_seconds(totals.subagent_exclusive),
        days: days
            .iter()
            .map(|(date, active_seconds)| DailyActiveTime {
                date: date.clone(),
                active_seconds: *active_seconds,
            })
            .collect(),
        models: named_active(
            &model_partitioned,
            &borrowed_seconds_map(&totals.models_inclusive),
        ),
        projects: named_active(
            &project_partitioned,
            &borrowed_seconds_map(&totals.projects_inclusive),
        ),
        project_unattributed_active_seconds: nanos_to_seconds(
            totals.project_unattributed_partitioned,
        ),
        project_unattributed_inclusive_active_seconds: nanos_to_seconds(
            totals.project_unattributed_inclusive,
        ),
        sessions: named_active(&session_partitioned, &session_inclusive),
        limitations: vec![
            "Active time is an observed-event estimate and does not measure human outcomes or attention."
                .to_string(),
            "Concurrent intervals are unioned globally; partitioned dimension totals use top-level-first then lexical ownership while inclusive values may overlap."
                .to_string(),
        ],
    };

    Ok(ActivityAnalysis {
        public,
        session_partitioned,
        session_inclusive,
        group_inclusive,
        project_partitioned,
        days,
        total_active_seconds,
        partition_nanos: BTreeMap::from([
            (
                "days",
                totals
                    .days
                    .values()
                    .copied()
                    .fold(0u128, u128::saturating_add),
            ),
            (
                "models",
                totals
                    .models_partitioned
                    .values()
                    .copied()
                    .fold(0u128, u128::saturating_add),
            ),
            (
                "projects",
                totals
                    .projects_partitioned
                    .values()
                    .copied()
                    .fold(0u128, u128::saturating_add)
                    .saturating_add(totals.project_unattributed_partitioned),
            ),
            (
                "sessions",
                totals
                    .sessions_partitioned
                    .values()
                    .copied()
                    .fold(0u128, u128::saturating_add),
            ),
            ("global", totals.total),
        ]),
    })
}

fn push_activity_interval<'a>(
    intervals: &mut Vec<ActivityInterval<'a>>,
    start: i128,
    end: i128,
    event: &'a NormalizedEvent,
    time_context: &TimeContext,
) {
    let Some((start, end)) = time_context.clip_interval(start, end) else {
        return;
    };
    intervals.push(ActivityInterval {
        start,
        end,
        project: event
            .project_identity_present
            .then_some(event.project_alias.as_str()),
        session: if event.session_identity_present {
            &event.session_alias
        } else {
            "unattributed"
        },
        inclusive_group: if event.session_identity_present {
            event
                .parent_session_alias
                .as_deref()
                .unwrap_or(&event.session_alias)
        } else {
            "unattributed"
        },
        model: event.model.as_deref().unwrap_or("unknown"),
        is_subagent: event.is_subagent,
    });
}

fn attribute_active_segment<'a>(
    start: i128,
    end: i128,
    active: &HashSet<usize>,
    intervals: &[ActivityInterval<'a>],
    time_context: &TimeContext,
    totals: &mut ActivityNanos<'a>,
) {
    let duration = u128::try_from(end.saturating_sub(start)).unwrap_or(u128::MAX);
    totals.total = totals.total.saturating_add(duration);
    if let Some(date) = time_context.date_key_epoch(start) {
        add_nanos(&mut totals.days, date, duration);
    }

    let mut models = HashSet::new();
    let mut projects = HashSet::new();
    let mut project_unattributed = false;
    let mut sessions = HashSet::new();
    let mut groups = HashSet::new();
    for index in active {
        let interval = &intervals[*index];
        models.insert(interval.model);
        if let Some(project) = interval.project {
            projects.insert(project);
        } else {
            project_unattributed = true;
        }
        sessions.insert(interval.session);
        groups.insert(interval.inclusive_group);
    }
    for model in models {
        add_borrowed_nanos(&mut totals.models_inclusive, model, duration);
    }
    for project in projects {
        add_borrowed_nanos(&mut totals.projects_inclusive, project, duration);
    }
    if project_unattributed {
        totals.project_unattributed_inclusive = totals
            .project_unattributed_inclusive
            .saturating_add(duration);
    }
    for session in sessions {
        add_borrowed_nanos(&mut totals.sessions_inclusive, session, duration);
    }
    for group in groups {
        add_borrowed_nanos(&mut totals.groups_inclusive, group, duration);
    }

    if let Some(owner) = active
        .iter()
        .map(|index| &intervals[*index])
        .min_by(|left, right| {
            left.is_subagent
                .cmp(&right.is_subagent)
                .then_with(|| left.session.cmp(right.session))
                .then_with(|| left.project.cmp(&right.project))
                .then_with(|| left.model.cmp(right.model))
        })
    {
        add_borrowed_nanos(&mut totals.models_partitioned, owner.model, duration);
        if let Some(project) = owner.project {
            add_borrowed_nanos(&mut totals.projects_partitioned, project, duration);
        } else {
            totals.project_unattributed_partitioned = totals
                .project_unattributed_partitioned
                .saturating_add(duration);
        }
        add_borrowed_nanos(&mut totals.sessions_partitioned, owner.session, duration);
        if owner.is_subagent {
            totals.subagent_exclusive = totals.subagent_exclusive.saturating_add(duration);
        } else {
            totals.main_exclusive = totals.main_exclusive.saturating_add(duration);
        }
    }
}

fn add_nanos(values: &mut BTreeMap<String, u128>, key: String, duration: u128) {
    let value = values.entry(key).or_default();
    *value = value.saturating_add(duration);
}

fn add_borrowed_nanos<'a>(values: &mut HashMap<&'a str, u128>, key: &'a str, duration: u128) {
    let value = values.entry(key).or_default();
    *value = value.saturating_add(duration);
}

fn nanos_to_seconds(value: u128) -> u64 {
    u64::try_from(value / 1_000_000_000u128).unwrap_or(u64::MAX)
}

fn seconds_map(values: &BTreeMap<String, u128>) -> BTreeMap<String, u64> {
    values
        .iter()
        .map(|(key, value)| (key.clone(), nanos_to_seconds(*value)))
        .collect()
}

fn borrowed_seconds_map(values: &HashMap<&str, u128>) -> BTreeMap<String, u64> {
    values
        .iter()
        .map(|(key, value)| ((*key).to_string(), nanos_to_seconds(*value)))
        .collect()
}

fn named_active(
    partitioned: &BTreeMap<String, u64>,
    inclusive: &BTreeMap<String, u64>,
) -> Vec<NamedActiveTime> {
    let keys = partitioned
        .keys()
        .chain(inclusive.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    keys.into_iter()
        .map(|key| NamedActiveTime {
            active_seconds: partitioned.get(&key).copied().unwrap_or(0),
            inclusive_active_seconds: inclusive.get(&key).copied().unwrap_or(0),
            key,
        })
        .collect()
}

fn elapsed_total(events: &[NormalizedEvent]) -> u64 {
    let mut bounds = HashMap::<&str, (i128, i128)>::new();
    for event in events.iter().filter(|event| {
        event.kind != EventKind::OtelMetric && event.session_identity_present && !event.is_subagent
    }) {
        bounds
            .entry(&event.session_alias)
            .and_modify(|(first, last)| {
                *first = (*first).min(event.epoch_nanos);
                *last = (*last).max(event.epoch_nanos);
            })
            .or_insert((event.epoch_nanos, event.epoch_nanos));
    }
    bounds.into_values().fold(0u64, |total, (first, last)| {
        let seconds =
            u64::try_from(last.saturating_sub(first) / NANOS_PER_SECOND).unwrap_or(u64::MAX);
        total.saturating_add(seconds)
    })
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct CostDomainAccumulator {
    source_pico: u128,
    source_samples: usize,
    local_pico: u128,
    canonical_priceable_tokens: u128,
    priceable_tokens: u128,
    priced_tokens: u128,
    unpriced_tokens: u128,
    priced_requests: usize,
    unpriced_requests: usize,
    priced_observations: usize,
    source_overflowed: bool,
    local_overflowed: bool,
    unpriced_overflowed: bool,
    token_accounting_invalid: bool,
    cache_ttl_composition_invalid: bool,
}

impl CostDomainAccumulator {
    fn observe(
        &mut self,
        source_pico: Option<u128>,
        price: Option<&PriceResult>,
        request_observation: bool,
    ) {
        if let Some(source_pico) = source_pico {
            checked_add_u128(
                &mut self.source_pico,
                source_pico,
                &mut self.source_overflowed,
            );
            checked_add_usize(&mut self.source_samples, 1, &mut self.source_overflowed);
        }
        let Some(price) = price else {
            return;
        };
        checked_add_u128(
            &mut self.local_pico,
            price.cost_pico_usd,
            &mut self.local_overflowed,
        );
        checked_add_u128(
            &mut self.priceable_tokens,
            price.priceable_tokens,
            &mut self.local_overflowed,
        );
        checked_add_u128(
            &mut self.priced_tokens,
            price.priced_tokens,
            &mut self.local_overflowed,
        );
        checked_add_u128(
            &mut self.unpriced_tokens,
            price.unpriced_tokens,
            &mut self.unpriced_overflowed,
        );
        self.token_accounting_invalid |=
            price.priced_tokens.checked_add(price.unpriced_tokens) != Some(price.priceable_tokens);
        self.cache_ttl_composition_invalid |= price.cache_ttl_composition_invalid;
        if price.priced_tokens > 0 {
            checked_add_usize(&mut self.priced_observations, 1, &mut self.local_overflowed);
        }
        if request_observation {
            if price.request_priced {
                checked_add_usize(&mut self.priced_requests, 1, &mut self.local_overflowed);
            } else {
                checked_add_usize(
                    &mut self.unpriced_requests,
                    1,
                    &mut self.unpriced_overflowed,
                );
            }
        }
    }

    fn merge(&mut self, other: &Self) {
        checked_add_u128(
            &mut self.source_pico,
            other.source_pico,
            &mut self.source_overflowed,
        );
        checked_add_usize(
            &mut self.source_samples,
            other.source_samples,
            &mut self.source_overflowed,
        );
        checked_add_u128(
            &mut self.local_pico,
            other.local_pico,
            &mut self.local_overflowed,
        );
        checked_add_u128(
            &mut self.priceable_tokens,
            other.priceable_tokens,
            &mut self.local_overflowed,
        );
        checked_add_u128(
            &mut self.priced_tokens,
            other.priced_tokens,
            &mut self.local_overflowed,
        );
        checked_add_usize(
            &mut self.priced_requests,
            other.priced_requests,
            &mut self.local_overflowed,
        );
        checked_add_usize(
            &mut self.priced_observations,
            other.priced_observations,
            &mut self.local_overflowed,
        );
        checked_add_u128(
            &mut self.unpriced_tokens,
            other.unpriced_tokens,
            &mut self.unpriced_overflowed,
        );
        checked_add_usize(
            &mut self.unpriced_requests,
            other.unpriced_requests,
            &mut self.unpriced_overflowed,
        );
        self.source_overflowed |= other.source_overflowed;
        self.local_overflowed |= other.local_overflowed;
        self.unpriced_overflowed |= other.unpriced_overflowed;
        self.token_accounting_invalid |= other.token_accounting_invalid;
        self.cache_ttl_composition_invalid |= other.cache_ttl_composition_invalid;
    }
}

fn checked_add_u128(target: &mut u128, value: u128, overflowed: &mut bool) {
    if let Some(sum) = target.checked_add(value) {
        *target = sum;
    } else {
        *target = u128::MAX;
        *overflowed = true;
    }
}

fn checked_add_usize(target: &mut usize, value: usize, overflowed: &mut bool) {
    if let Some(sum) = target.checked_add(value) {
        *target = sum;
    } else {
        *target = usize::MAX;
        *overflowed = true;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct ModelCostKey {
    raw_model: String,
    provider: String,
    canonical_model: Option<String>,
    pricing_key: Option<String>,
    pricing_modifier: String,
}

#[derive(Debug, Clone, Default)]
struct ModelCostAccumulator {
    domain: CostDomainAccumulator,
}

#[derive(Debug, Clone, Default)]
struct CostReconciliationProof {
    global: CostDomainAccumulator,
    days: CostDomainAccumulator,
    models: CostDomainAccumulator,
    projects: CostDomainAccumulator,
    sessions_plus_unattributed: CostDomainAccumulator,
}

impl CostReconciliationProof {
    fn bind_canonical_tokens(&mut self, tokens: &TokenAnalysis) {
        self.global.canonical_priceable_tokens = tokens.global.priceable_tokens();
        self.days.canonical_priceable_tokens = sum_priceable_tokens(tokens.days.values());
        self.models.canonical_priceable_tokens = sum_priceable_tokens(tokens.models.values());
        self.projects.canonical_priceable_tokens = sum_priceable_tokens(tokens.projects.values())
            .saturating_add(tokens.project_unattributed.priceable_tokens());
        self.sessions_plus_unattributed.canonical_priceable_tokens = {
            let session_tokens = sum_priceable_tokens(tokens.sessions.values());
            session_tokens.saturating_add(tokens.unattributed.priceable_tokens())
        };
    }
}

fn sum_priceable_tokens<'a>(values: impl Iterator<Item = &'a TokenAccumulator>) -> u128 {
    values.fold(0u128, |total, value| {
        total.saturating_add(value.priceable_tokens())
    })
}

#[derive(Debug)]
struct CostAnalysis {
    public: CanonicalCostMetrics,
    proof: CostReconciliationProof,
}

fn analyze_cost(events: &[NormalizedEvent], time_context: &TimeContext) -> CostAnalysis {
    let mut global = CostDomainAccumulator::default();
    let mut days = BTreeMap::<String, CostDomainAccumulator>::new();
    let mut projects = BTreeMap::<String, CostDomainAccumulator>::new();
    let mut project_unattributed = CostDomainAccumulator::default();
    let mut sessions = BTreeMap::<String, CostDomainAccumulator>::new();
    let mut unattributed = CostDomainAccumulator::default();
    let mut source_interfaces = BTreeSet::new();
    let mut models = BTreeMap::<ModelCostKey, ModelCostAccumulator>::new();

    for event in events
        .iter()
        .filter(|event| usage_event(event) || event.source_cost_estimate.is_some())
    {
        let raw_model = event.model.as_deref().unwrap_or("unknown");
        let source_pico = event.source_cost_estimate.map(dollars_to_pico_usd);
        if event.source_cost_estimate.is_some() {
            source_interfaces.insert(event.adapter_version);
        }
        let price = price_usage(
            raw_model,
            event.epoch_nanos,
            &event.pricing_modifier,
            &event.tokens,
        );
        let request_observation = matches!(
            event.kind,
            EventKind::AssistantUsage | EventKind::OtelApiRequest
        );

        global.observe(source_pico, Some(&price), request_observation);
        if let Some(date) = time_context.date_key_epoch(event.epoch_nanos) {
            days.entry(date)
                .or_default()
                .observe(source_pico, Some(&price), request_observation);
        }
        if event.project_identity_present {
            projects
                .entry(event.project_alias.clone())
                .or_default()
                .observe(source_pico, Some(&price), request_observation);
        } else {
            project_unattributed.observe(source_pico, Some(&price), request_observation);
        }
        if event.session_identity_present {
            sessions
                .entry(event.session_alias.clone())
                .or_default()
                .observe(source_pico, Some(&price), request_observation);
        } else {
            unattributed.observe(source_pico, Some(&price), request_observation);
        }

        let model = models
            .entry(ModelCostKey {
                raw_model: raw_model.to_string(),
                provider: price.provider.clone(),
                canonical_model: price.canonical_model.clone(),
                pricing_key: price.pricing_key.clone(),
                pricing_modifier: event.pricing_modifier.clone(),
            })
            .or_default();
        model
            .domain
            .observe(source_pico, Some(&price), request_observation);
    }

    let total_tokens = global.priced_tokens.saturating_add(global.unpriced_tokens);
    let local_amount =
        (global.priced_tokens > 0).then(|| round_money(pico_usd_to_dollars(global.local_pico)));
    let coverage = if total_tokens == 0 {
        "unavailable"
    } else if global.unpriced_tokens == 0 && global.unpriced_requests == 0 {
        "available"
    } else if global.priced_tokens == 0 {
        "unavailable"
    } else {
        "partial"
    };
    let public = CanonicalCostMetrics {
        source_recorded: CostMetricValue {
            amount_usd: (global.source_samples > 0)
                .then(|| round_money(pico_usd_to_dollars(global.source_pico))),
            unit: "USD".to_string(),
            availability: if global.source_samples > 0 {
                "available"
            } else {
                "unavailable"
            }
                .to_string(),
            quality: if global.source_samples > 0 {
                "direct"
            } else {
                "unavailable"
            }
            .to_string(),
            method_id: SOURCE_COST_METHOD.to_string(),
            source: (global.source_samples > 0).then(|| {
                source_interfaces
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(",")
            }),
            sample_count: global.source_samples,
            limitations: vec![
                "Source-recorded estimates are interface estimates and are not billing-authoritative."
                    .to_string(),
            ],
        },
        local_api_equivalent: CostMetricValue {
            amount_usd: local_amount,
            unit: "USD".to_string(),
            availability: coverage.to_string(),
            quality: if local_amount.is_some() { "modeled" } else { "unavailable" }.to_string(),
            method_id: LOCAL_COST_METHOD.to_string(),
            source: local_amount.map(|_| REGISTRY_VERSION.to_string()),
            sample_count: global.priced_observations,
            limitations: vec![
                "This is an API-equivalent estimate from exact evidenced prices, not a bill or subscription charge."
                    .to_string(),
            ],
        },
        billing_authoritative: CostMetricValue {
            amount_usd: None,
            unit: "USD".to_string(),
            availability: "unavailable".to_string(),
            quality: "unavailable".to_string(),
            method_id: BILLING_COST_METHOD.to_string(),
            source: None,
            sample_count: 0,
            limitations: vec![
                "No selected source supplies billing-authoritative charges.".to_string(),
            ],
        },
        coverage: coverage.to_string(),
        priced_tokens: u64::try_from(global.priced_tokens).unwrap_or(u64::MAX),
        priced_tokens_overflowed: global.priced_tokens > u128::from(u64::MAX),
        unpriced_tokens: u64::try_from(global.unpriced_tokens).unwrap_or(u64::MAX),
        unpriced_tokens_overflowed: global.unpriced_tokens > u128::from(u64::MAX),
        priced_requests: global.priced_requests,
        unpriced_requests: global.unpriced_requests,
        priced_token_share_pct: (total_tokens > 0).then(|| {
            ((global.priced_tokens as f64 / total_tokens as f64) * 1000.0).round() / 10.0
        }),
        models: models
            .iter()
            .map(|(key, value)| {
                let total_tokens = value
                    .domain
                    .priced_tokens
                    .saturating_add(value.domain.unpriced_tokens);
                let coverage = if total_tokens == 0 {
                    "unavailable"
                } else if value.domain.unpriced_tokens == 0
                    && value.domain.unpriced_requests == 0
                {
                    "available"
                } else if value.domain.priced_tokens == 0 {
                    "unavailable"
                } else {
                    "partial"
                };
                ModelCostEvidence {
                    raw_model: key.raw_model.clone(),
                    provider: key.provider.clone(),
                    canonical_model: key.canonical_model.clone(),
                    pricing_key: key.pricing_key.clone(),
                    pricing_modifier: key.pricing_modifier.clone(),
                    source_recorded_usd: (value.domain.source_samples > 0)
                        .then(|| round_money(pico_usd_to_dollars(value.domain.source_pico))),
                    local_api_equivalent_usd: (value.domain.priced_tokens > 0)
                        .then(|| round_money(pico_usd_to_dollars(value.domain.local_pico))),
                    priced_tokens: u64::try_from(value.domain.priced_tokens).unwrap_or(u64::MAX),
                    unpriced_tokens: u64::try_from(value.domain.unpriced_tokens)
                        .unwrap_or(u64::MAX),
                    priced_requests: value.domain.priced_requests,
                    unpriced_requests: value.domain.unpriced_requests,
                    requests: value
                        .domain
                        .priced_requests
                        .saturating_add(value.domain.unpriced_requests),
                    coverage: coverage.to_string(),
                }
            })
            .collect(),
    };
    let proof = CostReconciliationProof {
        global,
        days: sum_cost_domains(days.values()),
        models: sum_cost_domains(models.values().map(|value| &value.domain)),
        projects: {
            let mut total = sum_cost_domains(projects.values());
            total.merge(&project_unattributed);
            total
        },
        sessions_plus_unattributed: {
            let mut total = sum_cost_domains(sessions.values());
            total.merge(&unattributed);
            total
        },
    };
    CostAnalysis { public, proof }
}

fn sum_cost_domains<'a>(
    values: impl Iterator<Item = &'a CostDomainAccumulator>,
) -> CostDomainAccumulator {
    let mut total = CostDomainAccumulator::default();
    for value in values {
        total.merge(value);
    }
    total
}

fn round_money(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn analyze_cache(events: &[NormalizedEvent], tokens: &TokenAccumulator) -> CanonicalCacheMetrics {
    let direct_compactions = events
        .iter()
        .filter(|event| {
            event.kind == EventKind::Compaction && event.compaction.is_some_and(|value| value)
        })
        .fold(0u64, |count, _| count.saturating_add(1));
    CanonicalCacheMetrics {
        read_share: ratio(
            &tokens.cache_read,
            &tokens.input,
            tokens
                .input
                .observed
                .saturating_add(tokens.cache_read.observed),
            CACHE_READ_METHOD,
        ),
        write_share: ratio(
            &tokens.cache_creation,
            &tokens.input,
            tokens
                .input
                .observed
                .saturating_add(tokens.cache_creation.observed),
            CACHE_WRITE_METHOD,
        ),
        direct_compactions,
        limitations: vec![
            "Token shares establish no monetary effect, causal mechanism, or qualitative health judgment."
                .to_string(),
        ],
    }
}

fn ratio(
    numerator: &CategoryAccumulator,
    input: &CategoryAccumulator,
    denominator: u128,
    method_id: &str,
) -> RatioMetric {
    let complete = numerator.present > 0
        && numerator.present == numerator.eligible
        && input.present > 0
        && input.present == input.eligible;
    let overflowed = numerator.observed > u128::from(u64::MAX)
        || input.observed > u128::from(u64::MAX)
        || denominator > u128::from(u64::MAX);
    let value = (complete && !overflowed && denominator > 0)
        .then(|| ((numerator.observed as f64 / denominator as f64) * 1000.0).round() / 10.0);
    RatioMetric {
        value_pct: value,
        unit: "percent".to_string(),
        numerator: u64::try_from(numerator.observed).unwrap_or(u64::MAX),
        denominator: u64::try_from(denominator).unwrap_or(u64::MAX),
        sample_count: numerator.present.min(input.present),
        overflowed,
        availability: if value.is_some() {
            "available"
        } else {
            "unavailable"
        }
        .to_string(),
        method_id: method_id.to_string(),
        limitations: if overflowed {
            vec!["A required token category saturated, so the ratio is unavailable.".to_string()]
        } else if denominator == 0 {
            vec!["The documented denominator is zero, so the ratio is unavailable.".to_string()]
        } else if !complete {
            vec!["Required token-category coverage is incomplete.".to_string()]
        } else {
            Vec::new()
        },
    }
}

fn methodology(time_context: &TimeContext, threshold_seconds: u64) -> MethodologyCatalog {
    let mut methods = BTreeMap::new();
    methods.insert(
        PERIOD_METHOD.to_string(),
        MetricMethod {
            version: "1".to_string(),
            description: "Convert source instants through one selected IANA timezone before period and label attribution.".to_string(),
            parameters: BTreeMap::from([
                ("timezone".to_string(), time_context.name().to_string()),
                ("tzdb".to_string(), time_context.database_version().to_string()),
            ]),
        },
    );
    methods.insert(
        ELAPSED_METHOD.to_string(),
        MetricMethod {
            version: "1".to_string(),
            description: "Last ordered valid event instant minus first ordered valid event instant per session.".to_string(),
            parameters: BTreeMap::new(),
        },
    );
    methods.insert(
        ACTIVE_METHOD.to_string(),
        MetricMethod {
            version: "1".to_string(),
            description:
                "Union clipped half-open source-duration and capped adjacent-event intervals."
                    .to_string(),
            parameters: BTreeMap::from([
                (
                    "thresholdSeconds".to_string(),
                    threshold_seconds.to_string(),
                ),
                (
                    "partitionOwner".to_string(),
                    "top-level-first-then-lexical".to_string(),
                ),
                (
                    "directTimestampConvention".to_string(),
                    "source-timestamp-is-interval-end".to_string(),
                ),
                ("maxDirectDurationSeconds".to_string(), "86400".to_string()),
            ]),
        },
    );
    for (id, description) in [
        (TOKEN_METHOD, "Sum independent present token categories after canonical authority selection."),
        (CACHE_READ_METHOD, "cache_read / (input + cache_read), unavailable on zero/incomplete denominator."),
        (CACHE_WRITE_METHOD, "cache_creation / (input + cache_creation), unavailable on zero/incomplete denominator."),
        (SOURCE_COST_METHOD, "Keep source-emitted estimates separate and name their interface evidence."),
        (LOCAL_COST_METHOD, "Apply exact provider/model/effective-interval/modifier prices to observed token facts."),
        (BILLING_COST_METHOD, "Expose billing cost only from a documented billing-authoritative source."),
    ] {
        methods.insert(
            id.to_string(),
            MetricMethod {
                version: "1".to_string(),
                description: description.to_string(),
                parameters: BTreeMap::new(),
            },
        );
    }
    MethodologyCatalog {
        timezone_database: format!(
            "IANA {} via chrono-tz 0.10.4",
            time_context.database_version()
        ),
        methods,
        pricing_registry: PricingRegistryMetadata {
            version: REGISTRY_VERSION.to_string(),
            citation: REGISTRY_CITATION.to_string(),
            access_date: REGISTRY_ACCESS_DATE.to_string(),
            selection_policy: SELECTION_POLICY.to_string(),
            records: registry_records(),
        },
    }
}

fn compatibility_daily(
    events: &[NormalizedEvent],
    time_context: &TimeContext,
    activity: &ActivityAnalysis,
) -> Vec<DailyAggregate> {
    #[derive(Default)]
    struct Day {
        tokens: TokenUsage,
        cost_pico: u128,
        messages: usize,
        sessions: BTreeSet<String>,
        models: BTreeMap<String, ModelAggregate>,
    }
    let mut days = BTreeMap::<String, Day>::new();
    for event in events.iter().filter(|event| usage_event(event)) {
        let Some(date) = time_context.date_key_epoch(event.epoch_nanos) else {
            continue;
        };
        let day = days.entry(date).or_default();
        add_token_facts(&mut day.tokens, &event.tokens);
        let price = price_usage(
            event.model.as_deref().unwrap_or("unknown"),
            event.epoch_nanos,
            &event.pricing_modifier,
            &event.tokens,
        );
        let event_cost = pico_usd_to_dollars(price.cost_pico_usd);
        day.cost_pico = day
            .cost_pico
            .saturating_add(dollars_to_pico_usd(event_cost));
        let model_name = event.model.as_deref().unwrap_or("unknown").to_string();
        let model = day.models.entry(model_name).or_default();
        add_model_facts(model, &event.tokens);
        model.cost = (model.cost + event_cost).min(f64::MAX);
        if matches!(
            event.kind,
            EventKind::AssistantUsage | EventKind::OtelApiRequest
        ) {
            day.messages = day.messages.saturating_add(1);
            model.message_count = model.message_count.saturating_add(1);
            if event.session_identity_present {
                day.sessions.insert(event.session_alias.clone());
            }
        }
    }
    days.into_iter()
        .map(|(date, day)| DailyAggregate {
            date: date.clone(),
            total_cost: round_money(pico_usd_to_dollars(day.cost_pico)),
            input_tokens: day.tokens.input_tokens,
            output_tokens: day.tokens.output_tokens,
            cache_creation_tokens: day.tokens.cache_creation_tokens,
            cache_read_tokens: day.tokens.cache_read_tokens,
            message_count: day.messages,
            session_count: day.sessions.len(),
            active_seconds: activity.days.get(&date).copied().unwrap_or(0),
            cache_output_ratio: ccwrapped::round_ratio(
                day.tokens.cache_read_tokens,
                day.tokens.output_tokens,
            ),
            models: day.models,
        })
        .collect()
}

fn compatibility_projects(
    events: &[NormalizedEvent],
    activity: &ActivityAnalysis,
) -> Vec<ProjectSummary> {
    #[derive(Default)]
    struct Project {
        tokens: TokenUsage,
        messages: usize,
        sessions: BTreeSet<String>,
        top_level_sessions: BTreeSet<String>,
        subagent_sessions: BTreeSet<String>,
        first: Option<(i128, String)>,
        last: Option<(i128, String)>,
    }
    let mut projects = BTreeMap::<String, Project>::new();
    for event in events.iter().filter(|event| usage_event(event)) {
        if !event.project_identity_present {
            continue;
        }
        let project = projects.entry(event.project_alias.clone()).or_default();
        add_token_facts(&mut project.tokens, &event.tokens);
        if matches!(
            event.kind,
            EventKind::AssistantUsage | EventKind::OtelApiRequest
        ) {
            match &project.first {
                Some((epoch, _)) if *epoch <= event.epoch_nanos => {}
                _ => project.first = Some((event.epoch_nanos, event.timestamp.clone())),
            }
            match &project.last {
                Some((epoch, _)) if *epoch >= event.epoch_nanos => {}
                _ => project.last = Some((event.epoch_nanos, event.timestamp.clone())),
            }
            project.messages = project.messages.saturating_add(1);
            if event.session_identity_present {
                project.sessions.insert(event.session_alias.clone());
                if event.is_subagent {
                    project
                        .subagent_sessions
                        .insert(event.session_alias.clone());
                } else {
                    project
                        .top_level_sessions
                        .insert(event.session_alias.clone());
                }
            }
        }
    }
    let mut output = projects
        .into_iter()
        .map(|(alias, project)| ProjectSummary {
            hash: alias.clone(),
            path: None,
            name: alias.clone(),
            input_tokens: project.tokens.input_tokens,
            output_tokens: project.tokens.output_tokens,
            cache_creation_tokens: project.tokens.cache_creation_tokens,
            cache_read_tokens: project.tokens.cache_read_tokens,
            message_count: project.messages,
            session_count: if project.top_level_sessions.is_empty() {
                project.sessions.len()
            } else {
                project.top_level_sessions.len()
            },
            subagent_session_count: project.subagent_sessions.len(),
            active_seconds: activity
                .project_partitioned
                .get(&alias)
                .copied()
                .unwrap_or(0),
            first_seen: project.first.map(|(_, value)| value),
            last_seen: project.last.map(|(_, value)| value),
        })
        .collect::<Vec<_>>();
    output.sort_by(|left, right| {
        right
            .output_tokens
            .cmp(&left.output_tokens)
            .then_with(|| left.hash.cmp(&right.hash))
    });
    output
}

fn add_token_facts(total: &mut TokenUsage, tokens: &TokenFacts) {
    total.input_tokens = total.input_tokens.saturating_add(tokens.input.unwrap_or(0));
    total.output_tokens = total
        .output_tokens
        .saturating_add(tokens.output.unwrap_or(0));
    total.cache_creation_tokens = total
        .cache_creation_tokens
        .saturating_add(tokens.cache_creation.unwrap_or(0));
    total.cache_read_tokens = total
        .cache_read_tokens
        .saturating_add(tokens.cache_read.unwrap_or(0));
}

fn add_model_facts(total: &mut ModelAggregate, tokens: &TokenFacts) {
    total.input_tokens = total.input_tokens.saturating_add(tokens.input.unwrap_or(0));
    total.output_tokens = total
        .output_tokens
        .saturating_add(tokens.output.unwrap_or(0));
    total.cache_creation_tokens = total
        .cache_creation_tokens
        .saturating_add(tokens.cache_creation.unwrap_or(0));
    total.cache_read_tokens = total
        .cache_read_tokens
        .saturating_add(tokens.cache_read.unwrap_or(0));
}

fn dollars_to_pico_usd(value: f64) -> u128 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    (value.min(u128::MAX as f64 / 1_000_000_000_000.0) * 1_000_000_000_000.0).round() as u128
}

#[derive(Debug, Default)]
struct SessionAccumulator {
    project_alias: String,
    session_alias: String,
    parent_alias: Option<String>,
    is_subagent: bool,
    timestamp_start: Option<(i128, String)>,
    timestamp_end: Option<(i128, String)>,
    usage: TokenUsage,
    model_totals: BTreeMap<String, TokenUsage>,
    cost_usd: f64,
    prompt_count: usize,
    tool_message_count: usize,
}

impl SessionAccumulator {
    fn observe(&mut self, event: &NormalizedEvent) {
        if self
            .timestamp_start
            .as_ref()
            .is_none_or(|(epoch, _)| event.epoch_nanos < *epoch)
        {
            self.timestamp_start = Some((event.epoch_nanos, event.timestamp.clone()));
        }
        if self
            .timestamp_end
            .as_ref()
            .is_none_or(|(epoch, _)| event.epoch_nanos > *epoch)
        {
            self.timestamp_end = Some((event.epoch_nanos, event.timestamp.clone()));
        }
        match event.kind {
            EventKind::AssistantUsage | EventKind::OtelApiRequest => {
                add_token_facts(&mut self.usage, &event.tokens);
                let model = event.model.as_deref().unwrap_or("unknown").to_string();
                add_token_facts(self.model_totals.entry(model).or_default(), &event.tokens);
                if let Some(cost) = event.source_cost_estimate {
                    self.cost_usd = (self.cost_usd + cost).min(f64::MAX);
                }
            }
            EventKind::UserPrompt => {
                self.prompt_count = self.prompt_count.saturating_add(1);
            }
            EventKind::ToolResult | EventKind::OtelToolResult => {
                self.tool_message_count = self.tool_message_count.saturating_add(1);
            }
            _ => {}
        }
    }

    fn elapsed_seconds(&self) -> u64 {
        match (&self.timestamp_start, &self.timestamp_end) {
            (Some((start, _)), Some((end, _))) if end >= start => {
                u64::try_from((*end - *start) / NANOS_PER_SECOND).unwrap_or(u64::MAX)
            }
            _ => 0,
        }
    }

    fn into_summary(
        self,
        subagents: Vec<SubagentSummary>,
        activity: &ActivityAnalysis,
    ) -> SessionSummary {
        let total_tokens = self.usage.total_tokens();
        let elapsed_seconds = self.elapsed_seconds();
        let active_seconds = activity
            .session_partitioned
            .get(&self.session_alias)
            .copied()
            .unwrap_or(0);
        let inclusive_active_seconds = activity
            .group_inclusive
            .get(&self.session_alias)
            .copied()
            .unwrap_or_else(|| {
                activity
                    .session_inclusive
                    .get(&self.session_alias)
                    .copied()
                    .unwrap_or(0)
            });
        SessionSummary {
            session_id: self.session_alias,
            project_hash: self.project_alias.clone(),
            project_path: None,
            project_name: self.project_alias,
            timestamp_start: self.timestamp_start.map(|(_, timestamp)| timestamp),
            timestamp_end: self.timestamp_end.map(|(_, timestamp)| timestamp),
            duration_minutes: elapsed_seconds / 60,
            elapsed_seconds,
            active_seconds,
            inclusive_active_seconds,
            usage: self.usage,
            model_totals: self.model_totals,
            total_tokens,
            cost_usd: self.cost_usd,
            prompt_count: self.prompt_count,
            tool_message_count: self.tool_message_count,
            first_prompt: None,
            prompts: Vec::new(),
            subagents,
        }
    }

    fn to_subagent(&self, activity: &ActivityAnalysis) -> SubagentSummary {
        let elapsed_seconds = self.elapsed_seconds();
        SubagentSummary {
            session_id: self.session_alias.clone(),
            timestamp_start: self
                .timestamp_start
                .as_ref()
                .map(|(_, timestamp)| timestamp.clone()),
            duration_minutes: elapsed_seconds / 60,
            elapsed_seconds,
            active_seconds: activity
                .session_partitioned
                .get(&self.session_alias)
                .copied()
                .unwrap_or(0),
            total_tokens: self.usage.total_tokens(),
            usage: self.usage.clone(),
            first_prompt: None,
            project_path: None,
            project_name: Some(self.project_alias.clone()),
            parent_session_id: self.parent_alias.clone(),
        }
    }
}

fn session_breakdown(events: &[NormalizedEvent], activity: &ActivityAnalysis) -> SessionBreakdown {
    let mut grouped: BTreeMap<(String, String), SessionAccumulator> = BTreeMap::new();
    for event in events
        .iter()
        .filter(|event| event.kind != EventKind::OtelMetric && event.session_identity_present)
    {
        let key = (event.project_alias.clone(), event.session_alias.clone());
        let session = grouped.entry(key).or_insert_with(|| SessionAccumulator {
            project_alias: event.project_alias.clone(),
            session_alias: event.session_alias.clone(),
            parent_alias: event.parent_session_alias.clone(),
            is_subagent: event.is_subagent,
            ..SessionAccumulator::default()
        });
        session.observe(event);
    }

    let mut subagents_by_parent: BTreeMap<String, Vec<SubagentSummary>> = BTreeMap::new();
    let mut all_subagents = Vec::new();
    for session in grouped.values().filter(|session| session.is_subagent) {
        let summary = session.to_subagent(activity);
        if let Some(parent) = &session.parent_alias {
            subagents_by_parent
                .entry(parent.clone())
                .or_default()
                .push(summary.clone());
        }
        all_subagents.push(summary);
    }
    for subagents in subagents_by_parent.values_mut() {
        subagents.sort_by(|left, right| {
            right
                .total_tokens
                .cmp(&left.total_tokens)
                .then_with(|| left.session_id.cmp(&right.session_id))
        });
    }

    let mut sessions = grouped
        .into_values()
        .filter(|session| !session.is_subagent)
        .map(|session| {
            let subagents = subagents_by_parent
                .remove(&session.session_alias)
                .unwrap_or_default();
            session.into_summary(subagents, activity)
        })
        .collect::<Vec<_>>();
    sessions.sort_by(|left, right| {
        right
            .cost_usd
            .total_cmp(&left.cost_usd)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    all_subagents.sort_by(|left, right| {
        right
            .total_tokens
            .cmp(&left.total_tokens)
            .then_with(|| left.session_id.cmp(&right.session_id))
    });
    let total_elapsed_seconds = sessions.iter().fold(0u64, |total, session| {
        total.saturating_add(session.elapsed_seconds)
    });

    SessionBreakdown {
        costly_subagents: all_subagents.iter().take(20).cloned().collect(),
        total_subagent_sessions: all_subagents.len(),
        total_subagent_tokens: all_subagents
            .iter()
            .fold(0u64, |total, item| total.saturating_add(item.total_tokens)),
        total_elapsed_seconds,
        total_active_seconds: activity.total_active_seconds,
        sessions,
    }
}

fn hour_distribution(events: &[NormalizedEvent], time_context: &TimeContext) -> Vec<usize> {
    let mut counts = vec![0usize; 24];
    for event in events.iter().filter(|event| {
        matches!(
            event.kind,
            EventKind::AssistantUsage | EventKind::OtelApiRequest
        )
    }) {
        if let Some(hour) = time_context.hour_epoch(event.epoch_nanos) {
            counts[usize::from(hour)] = counts[usize::from(hour)].saturating_add(1);
        }
    }
    counts
}

fn reconcile(
    tokens: &TokenReconciliationProof,
    activity: &ActivityReconciliationProof,
    cost: &CostReconciliationProof,
) -> MetricReconciliation {
    let token_dimensions = tokens.statuses();
    let active_time_dimensions = activity.statuses();
    let cost_domains = cost_reconciliation_statuses(cost);
    let status = if token_dimensions.values().all(|value| value == "pass")
        && active_time_dimensions.values().all(|value| value == "pass")
        && cost_domains
            .values()
            .all(|value| value == "pass" || value == "unavailable")
    {
        "pass"
    } else {
        "fail"
    };
    MetricReconciliation {
        status: status.to_string(),
        token_dimensions,
        active_time_dimensions,
        cost_domains,
        limitations: vec![
            "Daily distinct session counts are non-additive; the wider period performs its own distinct count."
                .to_string(),
            "Inclusive active-time values may overlap; partitioned active-time values reconcile to the global union."
                .to_string(),
        ],
    }
}

pub(super) fn reconciliation_passes(reconciliation: &MetricReconciliation) -> bool {
    reconciliation.status == "pass"
        && reconciliation
            .token_dimensions
            .values()
            .all(|status| status == "pass")
        && reconciliation
            .active_time_dimensions
            .values()
            .all(|status| status == "pass")
        && reconciliation
            .cost_domains
            .values()
            .all(|status| status == "pass" || status == "unavailable")
}

pub(super) fn projection_reconciles(projection: &CanonicalProjection) -> bool {
    let expected_reconciliation = reconcile(
        &projection.token_reconciliation,
        &projection.activity_reconciliation,
        &projection.cost_reconciliation,
    );
    metric_reconciliation_matches(&projection.metrics.reconciliation, &expected_reconciliation)
        && canonical_token_metrics_match(
            &projection.metrics.tokens,
            &projection.token_reconciliation.public(),
        )
        && active_time_metrics_match(
            &projection.metrics.active_time,
            &projection.activity_reconciliation.expected_public,
        )
        && reconciliation_passes(&projection.metrics.reconciliation)
        && cost_public_matches_proof(
            &projection.metrics.cost,
            &projection.cost_reconciliation.global,
        )
}

fn metric_reconciliation_matches(
    observed: &MetricReconciliation,
    expected: &MetricReconciliation,
) -> bool {
    observed.status == expected.status
        && observed.token_dimensions == expected.token_dimensions
        && observed.active_time_dimensions == expected.active_time_dimensions
        && observed.cost_domains == expected.cost_domains
        && observed.limitations == expected.limitations
}

fn canonical_token_metrics_match(
    observed: &CanonicalTokenMetrics,
    expected: &CanonicalTokenMetrics,
) -> bool {
    token_metric_set_matches(&observed.global, &expected.global)
        && named_token_metric_sets_match(&observed.days, &expected.days)
        && named_token_metric_sets_match(&observed.models, &expected.models)
        && named_token_metric_sets_match(&observed.projects, &expected.projects)
        && token_metric_set_matches(
            &observed.project_unattributed,
            &expected.project_unattributed,
        )
        && named_token_metric_sets_match(&observed.sessions, &expected.sessions)
        && token_metric_set_matches(&observed.unattributed, &expected.unattributed)
}

fn named_token_metric_sets_match(
    observed: &[NamedTokenMetricSet],
    expected: &[NamedTokenMetricSet],
) -> bool {
    observed.len() == expected.len()
        && observed.iter().zip(expected).all(|(observed, expected)| {
            observed.key == expected.key
                && token_metric_set_matches(&observed.tokens, &expected.tokens)
        })
}

fn token_metric_set_matches(observed: &TokenMetricSet, expected: &TokenMetricSet) -> bool {
    token_metric_value_matches(&observed.input, &expected.input)
        && token_metric_value_matches(&observed.output, &expected.output)
        && token_metric_value_matches(&observed.cache_creation, &expected.cache_creation)
        && token_metric_value_matches(&observed.cache_read, &expected.cache_read)
        && token_metric_value_matches(&observed.cache_creation_5m, &expected.cache_creation_5m)
        && token_metric_value_matches(&observed.cache_creation_1h, &expected.cache_creation_1h)
        && token_metric_value_matches(&observed.total, &expected.total)
}

fn token_metric_value_matches(observed: &TokenMetricValue, expected: &TokenMetricValue) -> bool {
    observed.observed == expected.observed
        && observed.unit == expected.unit
        && observed.availability == expected.availability
        && observed.sample_count == expected.sample_count
        && observed.overflowed == expected.overflowed
        && observed.method_id == expected.method_id
        && observed.limitations == expected.limitations
}

fn active_time_metrics_match(observed: &ActiveTimeMetrics, expected: &ActiveTimeMetrics) -> bool {
    observed.method_id == expected.method_id
        && observed.unit == expected.unit
        && observed.availability == expected.availability
        && observed.interval_count == expected.interval_count
        && observed.threshold_seconds == expected.threshold_seconds
        && observed.total_elapsed_seconds == expected.total_elapsed_seconds
        && observed.total_active_seconds == expected.total_active_seconds
        && observed.main_exclusive_seconds == expected.main_exclusive_seconds
        && observed.subagent_exclusive_seconds == expected.subagent_exclusive_seconds
        && daily_active_times_match(&observed.days, &expected.days)
        && named_active_times_match(&observed.models, &expected.models)
        && named_active_times_match(&observed.projects, &expected.projects)
        && observed.project_unattributed_active_seconds
            == expected.project_unattributed_active_seconds
        && observed.project_unattributed_inclusive_active_seconds
            == expected.project_unattributed_inclusive_active_seconds
        && named_active_times_match(&observed.sessions, &expected.sessions)
        && observed.limitations == expected.limitations
}

fn daily_active_times_match(observed: &[DailyActiveTime], expected: &[DailyActiveTime]) -> bool {
    observed.len() == expected.len()
        && observed.iter().zip(expected).all(|(observed, expected)| {
            observed.date == expected.date && observed.active_seconds == expected.active_seconds
        })
}

fn named_active_times_match(observed: &[NamedActiveTime], expected: &[NamedActiveTime]) -> bool {
    observed.len() == expected.len()
        && observed.iter().zip(expected).all(|(observed, expected)| {
            observed.key == expected.key
                && observed.active_seconds == expected.active_seconds
                && observed.inclusive_active_seconds == expected.inclusive_active_seconds
        })
}

fn cost_reconciliation_statuses(proof: &CostReconciliationProof) -> BTreeMap<String, String> {
    let dimensions = [
        &proof.days,
        &proof.models,
        &proof.projects,
        &proof.sessions_plus_unattributed,
    ];
    BTreeMap::from([
        (
            "sourceRecordedSeparate".to_string(),
            cost_domain_status(&proof.global, &dimensions, source_domain_matches),
        ),
        (
            "localApiEquivalentSeparate".to_string(),
            cost_domain_status(&proof.global, &dimensions, local_domain_matches),
        ),
        (
            "unpricedUsage".to_string(),
            cost_domain_status(&proof.global, &dimensions, unpriced_domain_matches),
        ),
        (
            "billingAuthoritative".to_string(),
            "unavailable".to_string(),
        ),
    ])
}

fn cost_domain_status(
    global: &CostDomainAccumulator,
    dimensions: &[&CostDomainAccumulator; 4],
    matches: fn(&CostDomainAccumulator, &CostDomainAccumulator) -> bool,
) -> String {
    if dimensions
        .iter()
        .all(|dimension| matches(global, dimension))
    {
        "pass"
    } else {
        "fail"
    }
    .to_string()
}

fn source_domain_matches(left: &CostDomainAccumulator, right: &CostDomainAccumulator) -> bool {
    !left.source_overflowed
        && !right.source_overflowed
        && left.source_pico == right.source_pico
        && left.source_samples == right.source_samples
}

fn local_domain_matches(left: &CostDomainAccumulator, right: &CostDomainAccumulator) -> bool {
    !left.local_overflowed
        && !right.local_overflowed
        && cost_token_accounting_matches(left)
        && cost_token_accounting_matches(right)
        && left.local_pico == right.local_pico
        && left.priceable_tokens == right.priceable_tokens
        && left.priced_tokens == right.priced_tokens
        && left.priced_requests == right.priced_requests
        && left.priced_observations == right.priced_observations
}

fn unpriced_domain_matches(left: &CostDomainAccumulator, right: &CostDomainAccumulator) -> bool {
    !left.unpriced_overflowed
        && !right.unpriced_overflowed
        && cost_token_accounting_matches(left)
        && cost_token_accounting_matches(right)
        && left.unpriced_tokens == right.unpriced_tokens
        && left.unpriced_requests == right.unpriced_requests
}

fn cost_token_accounting_matches(domain: &CostDomainAccumulator) -> bool {
    !domain.token_accounting_invalid
        && domain.priceable_tokens == domain.canonical_priceable_tokens
        && domain.priced_tokens.checked_add(domain.unpriced_tokens) == Some(domain.priceable_tokens)
}

fn cost_public_matches_proof(public: &CanonicalCostMetrics, proof: &CostDomainAccumulator) -> bool {
    let source_amount =
        (proof.source_samples > 0).then(|| round_money(pico_usd_to_dollars(proof.source_pico)));
    let local_amount =
        (proof.priced_tokens > 0).then(|| round_money(pico_usd_to_dollars(proof.local_pico)));
    cost_token_accounting_matches(proof)
        && public.source_recorded.amount_usd == source_amount
        && public.source_recorded.sample_count == proof.source_samples
        && public.local_api_equivalent.amount_usd == local_amount
        && public.local_api_equivalent.sample_count == proof.priced_observations
        && public.priced_tokens == u64::try_from(proof.priced_tokens).unwrap_or(u64::MAX)
        && public.priced_tokens_overflowed == (proof.priced_tokens > u128::from(u64::MAX))
        && public.unpriced_tokens == u64::try_from(proof.unpriced_tokens).unwrap_or(u64::MAX)
        && public.unpriced_tokens_overflowed == (proof.unpriced_tokens > u128::from(u64::MAX))
        && public.priced_requests == proof.priced_requests
        && public.unpriced_requests == proof.unpriced_requests
}

#[cfg(test)]
pub(super) fn perturb_cost_projection_for_test(projection: &mut CanonicalProjection) {
    projection.cost_reconciliation.models.local_pico = projection
        .cost_reconciliation
        .models
        .local_pico
        .saturating_add(1);
}

#[cfg(test)]
pub(super) fn perturb_cost_token_accounting_for_test(projection: &mut CanonicalProjection) {
    projection.cost_reconciliation.global.priceable_tokens = projection
        .cost_reconciliation
        .global
        .priceable_tokens
        .saturating_add(1);
}

#[cfg(test)]
pub(super) fn perturb_public_token_projection_for_test(projection: &mut CanonicalProjection) {
    projection.metrics.tokens.global.input.observed = projection
        .metrics
        .tokens
        .global
        .input
        .observed
        .saturating_add(1);
}

#[cfg(test)]
pub(super) fn perturb_public_activity_projection_for_test(projection: &mut CanonicalProjection) {
    projection.metrics.active_time.total_active_seconds = projection
        .metrics
        .active_time
        .total_active_seconds
        .saturating_add(1);
}

fn token_dimension_status<'a>(
    global: &TokenAccumulator,
    values: impl Iterator<Item = &'a TokenAccumulator>,
) -> String {
    let mut total = TokenAccumulator::default();
    for value in values {
        total.merge(value);
    }
    if token_accumulators_equal(global, &total) {
        "pass"
    } else {
        "fail"
    }
    .to_string()
}

fn token_accumulators_equal(left: &TokenAccumulator, right: &TokenAccumulator) -> bool {
    [
        (&left.input, &right.input),
        (&left.output, &right.output),
        (&left.cache_creation, &right.cache_creation),
        (&left.cache_read, &right.cache_read),
        (&left.cache_creation_5m, &right.cache_creation_5m),
        (&left.cache_creation_1h, &right.cache_creation_1h),
    ]
    .into_iter()
    .all(|(left, right)| {
        left.observed == right.observed
            && left.present == right.present
            && left.eligible == right.eligible
    })
}

/// Compatibility helper retained for callers inside the current binary while the canonical
/// projection replaces legacy metric merging. It keeps the prior public-free symbol available.
#[allow(dead_code)]
pub(crate) fn merge_metric_aggregates(
    daily: &mut Vec<DailyAggregate>,
    metric_daily: Vec<DailyAggregate>,
    projects: &mut Vec<ProjectSummary>,
    metric_projects: Vec<ProjectSummary>,
) {
    let mut daily_positions = daily
        .iter()
        .enumerate()
        .map(|(index, day)| (day.date.clone(), index))
        .collect::<HashMap<_, _>>();
    for mut metric_day in metric_daily {
        if let Some(index) = daily_positions.get(&metric_day.date).copied() {
            let day = &mut daily[index];
            day.total_cost = (day.total_cost + metric_day.total_cost).min(f64::MAX);
            day.input_tokens = day.input_tokens.saturating_add(metric_day.input_tokens);
            day.output_tokens = day.output_tokens.saturating_add(metric_day.output_tokens);
            day.cache_creation_tokens = day
                .cache_creation_tokens
                .saturating_add(metric_day.cache_creation_tokens);
            day.cache_read_tokens = day
                .cache_read_tokens
                .saturating_add(metric_day.cache_read_tokens);
            day.active_seconds = day.active_seconds.saturating_add(metric_day.active_seconds);
            for (model_name, metric_model) in metric_day.models {
                let model = day.models.entry(model_name).or_default();
                model.input_tokens = model.input_tokens.saturating_add(metric_model.input_tokens);
                model.output_tokens = model
                    .output_tokens
                    .saturating_add(metric_model.output_tokens);
                model.cache_creation_tokens = model
                    .cache_creation_tokens
                    .saturating_add(metric_model.cache_creation_tokens);
                model.cache_read_tokens = model
                    .cache_read_tokens
                    .saturating_add(metric_model.cache_read_tokens);
                model.cost = (model.cost + metric_model.cost).min(f64::MAX);
                model.active_seconds = model
                    .active_seconds
                    .saturating_add(metric_model.active_seconds);
            }
            day.cache_output_ratio =
                ccwrapped::round_ratio(day.cache_read_tokens, day.output_tokens);
        } else {
            metric_day.message_count = 0;
            metric_day.session_count = 0;
            for model in metric_day.models.values_mut() {
                model.message_count = 0;
            }
            daily_positions.insert(metric_day.date.clone(), daily.len());
            daily.push(metric_day);
        }
    }
    daily.sort_by(|left, right| left.date.cmp(&right.date));

    let mut project_positions = projects
        .iter()
        .enumerate()
        .map(|(index, project)| (project.hash.clone(), index))
        .collect::<HashMap<_, _>>();
    for mut metric_project in metric_projects {
        if let Some(index) = project_positions.get(&metric_project.hash).copied() {
            let project = &mut projects[index];
            project.input_tokens = project
                .input_tokens
                .saturating_add(metric_project.input_tokens);
            project.output_tokens = project
                .output_tokens
                .saturating_add(metric_project.output_tokens);
            project.cache_creation_tokens = project
                .cache_creation_tokens
                .saturating_add(metric_project.cache_creation_tokens);
            project.cache_read_tokens = project
                .cache_read_tokens
                .saturating_add(metric_project.cache_read_tokens);
            project.active_seconds = project
                .active_seconds
                .saturating_add(metric_project.active_seconds);
        } else {
            metric_project.message_count = 0;
            metric_project.session_count = 0;
            metric_project.subagent_session_count = 0;
            metric_project.first_seen = None;
            metric_project.last_seen = None;
            project_positions.insert(metric_project.hash.clone(), projects.len());
            projects.push(metric_project);
        }
    }
    projects.sort_by(|left, right| {
        right
            .output_tokens
            .cmp(&left.output_tokens)
            .then_with(|| left.hash.cmp(&right.hash))
    });
}
