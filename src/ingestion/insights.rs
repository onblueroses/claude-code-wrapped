use super::time::{epoch_datetime, TimeContext};
use super::types::{EventKind, NormalizedEvent};
use ccwrapped::{
    CanonicalMetrics, DataCoverage, InsightAction, InsightCard, InsightComparison, InsightFact,
    InsightFamilyStatus, InsightReport, InsightWindow, MethodologyCatalog, MetricMethod,
    NamedTokenMetricSet,
};
use chrono::{Duration, NaiveDate, SecondsFormat};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::thread;

const REPORT_VERSION: &str = "ccwrapped.insights/v1";
const COMPARISON_METHOD: &str = "comparison/adjacent-equal-window/v1";
const TREND_METHOD: &str = "trend/median-halves/v1";
const EFFICIENCY_METHOD: &str = "efficiency/observed-active-rate/v1";
const RELIABILITY_METHOD: &str = "reliability/event-rate/v1";
const TOOL_METHOD: &str = "tool/observed-outcomes/v1";
const ROUTING_METHOD: &str = "routing/model-share/v1";
const CONCENTRATION_METHOD: &str = "concentration/project-hhi/v1";
const ANOMALY_METHOD: &str = "anomaly/median-mad/v1";
const RECOMMENDATION_METHOD: &str = "recommendation/evidence-rule/v1";
const ENTERTAINMENT_METHOD: &str = "entertainment/sample-gated/v1";

const MAX_CARDS: usize = 32;
const MAX_FACTS_PER_CARD: usize = 16;
const COMPARISON_DAYS: i64 = 28;
const COMPARISON_MINIMUM_ACTIVE_DAYS: usize = 7;
const TREND_MAXIMUM_POINTS: usize = 28;
const TREND_MINIMUM_POINTS: usize = 8;
const EFFICIENCY_MINIMUM_ACTIVE_SECONDS: u64 = 900;
const EFFICIENCY_MINIMUM_REQUESTS: usize = 5;

#[derive(Debug, Clone, Copy)]
pub(super) struct ValidationEvidence {
    active_efficiency_sample_count: usize,
    active_time_available: bool,
    active_seconds: u64,
}

impl ValidationEvidence {
    pub(super) fn new(metrics: &CanonicalMetrics, active_efficiency_sample_count: usize) -> Self {
        Self {
            active_efficiency_sample_count,
            active_time_available: metrics.active_time.availability == "available",
            active_seconds: metrics.active_time.total_active_seconds,
        }
    }
}

const FAMILIES: [(&str, &[&str]); 10] = [
    ("comparison", &["analysis_usage_totals"]),
    ("trend", &["analysis_usage_totals"]),
    (
        "active-efficiency",
        &["analysis_usage_totals", "analysis_active_time"],
    ),
    (
        "reliability",
        &["direct_terminal_outcomes", "retry_evidence"],
    ),
    (
        "tool-behavior",
        &[
            "tool_occurrence",
            "tool_result",
            "tool_status",
            "tool_latency",
            "edit_decision",
        ],
    ),
    ("model-routing", &["analysis_usage_totals"]),
    ("project-concentration", &["analysis_usage_totals"]),
    ("anomaly", &["analysis_usage_totals"]),
    ("recommendation", &[]),
    ("entertainment", &[]),
];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub(super) struct ObservedEventSummary {
    minimum_date: Option<String>,
    maximum_date: Option<String>,
    output_signatures: BTreeMap<String, BTreeSet<String>>,
}

impl ObservedEventSummary {
    pub fn from_events(events: &[NormalizedEvent], time_context: &TimeContext) -> Self {
        let mut summary = Self::default();
        summary.extend(events, time_context);
        summary
    }

    pub fn extend(&mut self, events: &[NormalizedEvent], time_context: &TimeContext) {
        for event in events {
            let Some(date) = time_context.local_date_epoch(event.epoch_nanos) else {
                continue;
            };
            let date = date.format("%Y-%m-%d").to_string();
            if self
                .minimum_date
                .as_ref()
                .is_none_or(|current| date < *current)
            {
                self.minimum_date = Some(date.clone());
            }
            if self
                .maximum_date
                .as_ref()
                .is_none_or(|current| date > *current)
            {
                self.maximum_date = Some(date.clone());
            }
            if event.tokens.output.is_some() {
                self.output_signatures
                    .entry(date)
                    .or_default()
                    .insert(format!("{}:{:?}", event.adapter_version, event.kind));
            }
        }
    }

    fn minimum(&self) -> Option<NaiveDate> {
        self.minimum_date
            .as_deref()
            .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
    }

    fn maximum(&self) -> Option<NaiveDate> {
        self.maximum_date
            .as_deref()
            .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
    }

    fn window_signature(&self, start: NaiveDate, end: NaiveDate) -> BTreeSet<String> {
        self.output_signatures
            .range(start.format("%Y-%m-%d").to_string()..end.format("%Y-%m-%d").to_string())
            .flat_map(|(_, signatures)| signatures.iter().cloned())
            .collect()
    }
}

pub(super) fn build_from_summary(
    observed_summary: &ObservedEventSummary,
    events: &[NormalizedEvent],
    metrics: &CanonicalMetrics,
    coverage: &DataCoverage,
    time_context: &TimeContext,
    methodology: &mut MethodologyCatalog,
) -> Result<InsightReport, super::IngestionError> {
    install_methods(methodology);
    let (
        comparison,
        trend,
        efficiency,
        (reliability_report, reliability),
        (tool_report, tools),
        (routing_report, routing),
        concentration,
        anomalies,
    ) = thread::scope(|scope| {
        let comparison = scope.spawn(|| {
            let mut report = empty_report();
            build_comparison(
                &metrics.tokens.days,
                observed_summary,
                events,
                coverage,
                time_context,
                &mut report,
            )?;
            Ok::<_, super::IngestionError>(report)
        });
        let trend = scope.spawn(|| {
            let mut report = empty_report();
            build_trend(
                &metrics.tokens.days,
                events,
                coverage,
                time_context,
                &mut report,
            );
            report
        });
        let efficiency = scope.spawn(|| {
            let mut report = empty_report();
            build_active_efficiency(events, metrics, coverage, time_context, &mut report);
            report
        });
        let reliability = scope.spawn(|| {
            let mut report = empty_report();
            let summary = build_reliability(events, coverage, time_context, &mut report);
            (report, summary)
        });
        let tools = scope.spawn(|| {
            let mut report = empty_report();
            let summary = build_tool_behavior(events, coverage, time_context, &mut report);
            (report, summary)
        });
        let routing = scope.spawn(|| {
            let mut report = empty_report();
            let summary = build_model_routing(events, metrics, coverage, time_context, &mut report);
            (report, summary)
        });
        let concentration = scope.spawn(|| {
            let mut report = empty_report();
            build_project_concentration(metrics, events, coverage, time_context, &mut report);
            report
        });
        let anomalies = scope.spawn(|| {
            let mut report = empty_report();
            build_anomalies(
                &metrics.tokens.days,
                events,
                coverage,
                time_context,
                &mut report,
            );
            report
        });
        Ok::<_, super::IngestionError>((
            join_worker(comparison, "comparison")??,
            join_worker(trend, "trend")?,
            join_worker(efficiency, "active-efficiency")?,
            join_worker(reliability, "reliability")?,
            join_worker(tools, "tool-behavior")?,
            join_worker(routing, "model-routing")?,
            join_worker(concentration, "project-concentration")?,
            join_worker(anomalies, "anomaly")?,
        ))
    })?;

    let mut report = empty_report();
    merge_family(&mut report, comparison, "comparison");
    merge_family(&mut report, trend, "trend");
    merge_family(&mut report, efficiency, "active-efficiency");
    merge_family(&mut report, reliability_report, "reliability");
    merge_family(&mut report, tool_report, "tool-behavior");
    merge_family(&mut report, routing_report, "model-routing");
    merge_family(&mut report, concentration, "project-concentration");
    merge_family(&mut report, anomalies, "anomaly");
    build_recommendations(
        events,
        &reliability,
        &tools,
        &routing,
        time_context,
        &mut report,
    );
    build_entertainment(events, metrics, coverage, time_context, &mut report);

    for card in &mut report.cards {
        card.supporting_facts
            .sort_by(|left, right| left.id.cmp(&right.id));
    }
    report.cards.sort_by(|left, right| {
        left.renderer_priority
            .cmp(&right.renderer_priority)
            .then_with(|| left.family.cmp(&right.family))
            .then_with(|| left.id.cmp(&right.id))
    });
    report.families.sort_by(|left, right| {
        family_rank(&left.family)
            .cmp(&family_rank(&right.family))
            .then_with(|| left.family.cmp(&right.family))
    });
    let request_count = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::AssistantUsage | EventKind::OtelApiRequest
            )
        })
        .count();
    validate(
        &report,
        methodology,
        ValidationEvidence::new(metrics, request_count),
    )?;
    Ok(report)
}

fn empty_report() -> InsightReport {
    InsightReport {
        version: REPORT_VERSION.to_string(),
        families: FAMILIES
            .iter()
            .map(|(family, capabilities)| unavailable_family(family, capabilities))
            .collect(),
        cards: Vec::new(),
    }
}

fn merge_family(target: &mut InsightReport, mut source: InsightReport, family: &str) {
    if let Some(updated) = source
        .families
        .into_iter()
        .find(|status| status.family == family)
    {
        if let Some(current) = target
            .families
            .iter_mut()
            .find(|status| status.family == family)
        {
            *current = updated;
        }
    }
    target
        .cards
        .extend(source.cards.drain(..).filter(|card| card.family == family));
}

fn join_worker<T>(
    worker: thread::ScopedJoinHandle<'_, T>,
    family: &'static str,
) -> Result<T, super::IngestionError> {
    worker.join().map_err(|_| {
        super::IngestionError::internal(
            "E_INSIGHT_WORKER",
            format!("the {family} insight worker panicked"),
            "Retry with the same inputs; if the error persists, report the tool version and error code without attaching private history.",
        )
    })
}

fn install_methods(methodology: &mut MethodologyCatalog) {
    for (id, description, parameters) in [
        (
            COMPARISON_METHOD,
            "Compare canonical output tokens across two adjacent equal selected-zone calendar windows.",
            BTreeMap::from([
                ("windowDays".to_string(), COMPARISON_DAYS.to_string()),
                (
                    "minimumActiveDaysPerWindow".to_string(),
                    COMPARISON_MINIMUM_ACTIVE_DAYS.to_string(),
                ),
                (
                    "zeroBaselineException".to_string(),
                    "requires-explicit-exhaustive-producer-coverage-unavailable-under-v1-adapters"
                        .to_string(),
                ),
                ("anchor".to_string(), "latest-observed-local-date".to_string()),
            ]),
        ),
        (
            TREND_METHOD,
            "Compare medians of equal chronological halves of exact observed daily output-token points.",
            BTreeMap::from([
                (
                    "minimumPoints".to_string(),
                    TREND_MINIMUM_POINTS.to_string(),
                ),
                (
                    "maximumPoints".to_string(),
                    TREND_MAXIMUM_POINTS.to_string(),
                ),
                ("relativeThresholdPct".to_string(), "10".to_string()),
                ("absoluteThresholdTokens".to_string(), "100".to_string()),
            ]),
        ),
        (
            EFFICIENCY_METHOD,
            "Describe canonical output, request, error, and complete local-cost observations per unioned active hour.",
            BTreeMap::from([
                (
                    "minimumActiveSeconds".to_string(),
                    EFFICIENCY_MINIMUM_ACTIVE_SECONDS.to_string(),
                ),
                (
                    "minimumRequestObservations".to_string(),
                    EFFICIENCY_MINIMUM_REQUESTS.to_string(),
                ),
            ]),
        ),
        (
            RELIABILITY_METHOD,
            "Compute direct terminal API outcome and recovered-retry rates from supported Claude Code OTel events.",
            BTreeMap::from([
                ("minimumTerminalOutcomes".to_string(), "10".to_string()),
                (
                    "minimumCompletedRequestsWithAttemptEvidence".to_string(),
                    "10".to_string(),
                ),
                (
                    "terminalOutcomeDenominator".to_string(),
                    "api_request+api_error".to_string(),
                ),
            ]),
        ),
        (
            TOOL_METHOD,
            "Keep transcript occurrence separate from direct OTel result, latency, and edit-decision facts.",
            BTreeMap::from([
                ("minimumResultsPerTool".to_string(), "5".to_string()),
                ("minimumEditDecisions".to_string(), "5".to_string()),
                ("latencyP95".to_string(), "nearest-rank".to_string()),
                (
                    "maximumDirectDurationMs".to_string(),
                    decimal(super::types::MAX_DIRECT_DURATION_MS),
                ),
                ("maximumRankedTools".to_string(), "10".to_string()),
                (
                    "recommendationCandidatePopulation".to_string(),
                    "all-classified-tools".to_string(),
                ),
                (
                    "recommendationTriggerCardBudget".to_string(),
                    "top-9-ranked-plus-one-trigger-when-trigger-rank-exceeds-10".to_string(),
                ),
            ]),
        ),
        (
            ROUTING_METHOD,
            "Report exact mapped-model request, output-token, and local API-equivalent cost shares with unknown coverage.",
            BTreeMap::from([
                ("minimumObservations".to_string(), "5".to_string()),
                (
                    "localCostDenominator".to_string(),
                    "priced-local-api-equivalent-only".to_string(),
                ),
                (
                    "shareDenominatorPopulation".to_string(),
                    "all-mapped-plus-unknown".to_string(),
                ),
                ("maximumRankedModels".to_string(), "10".to_string()),
                (
                    "omittedMappedBucket".to_string(),
                    "other-mapped".to_string(),
                ),
            ]),
        ),
        (
            CONCENTRATION_METHOD,
            "Compute project concentration from exact known output-token weights and report unattributed coverage separately.",
            BTreeMap::from([
                ("concentratedHhiMinimum".to_string(), "2500".to_string()),
                ("distributedHhiMaximum".to_string(), "1500".to_string()),
                ("concentratedTopSharePct".to_string(), "70".to_string()),
            ]),
        ),
        (
            ANOMALY_METHOD,
            "Detect unusual observed daily output-token values with a median/MAD score and practical-change guard.",
            BTreeMap::from([
                ("minimumPoints".to_string(), "7".to_string()),
                ("robustScoreThreshold".to_string(), "3.5".to_string()),
                (
                    "practicalDifference".to_string(),
                    "max(100,25%-of-median)".to_string(),
                ),
                (
                    "madZeroDifference".to_string(),
                    "max(1000,median)".to_string(),
                ),
                ("maximumAnomalies".to_string(), "3".to_string()),
            ]),
        ),
        (
            RECOMMENDATION_METHOD,
            "Attach bounded reversible experiments only to direct facts that cross frozen sample and rate thresholds.",
            BTreeMap::from([
                (
                    "rules".to_string(),
                    "api-terminal-errors,tool-result-errors,model-routing-experiment".to_string(),
                ),
                ("apiTerminalMinimumOutcomes".to_string(), "10".to_string()),
                (
                    "apiTerminalErrorRateMinimumPct".to_string(),
                    "10".to_string(),
                ),
                ("toolMinimumResults".to_string(), "10".to_string()),
                ("toolFailureRateMinimumPct".to_string(), "20".to_string()),
                (
                    "toolCandidatePopulation".to_string(),
                    "all-classified-tools".to_string(),
                ),
                ("routingMinimumObservations".to_string(), "20".to_string()),
                ("routingTopShareMinimumPct".to_string(), "80".to_string()),
                ("routingUnknownShareMaximumPct".to_string(), "10".to_string()),
            ]),
        ),
        (
            ENTERTAINMENT_METHOD,
            "Assign visibly marked deterministic entertainment labels only after their factual sample gates pass.",
            BTreeMap::from([
                ("minimumObservations".to_string(), "20".to_string()),
                ("minimumActiveDays".to_string(), "5".to_string()),
                (
                    "archetypeTieOrder".to_string(),
                    "orchestrator,toolsmith,specialist,explorer".to_string(),
                ),
                (
                    "orchestratorSubagentMinimumPct".to_string(),
                    "30".to_string(),
                ),
                (
                    "toolsmithRule".to_string(),
                    "every-canonical-request-message-observation-has-classified-tool-occurrence"
                        .to_string(),
                ),
                (
                    "specialistProjectHhiMinimum".to_string(),
                    "2500".to_string(),
                ),
            ]),
        ),
    ] {
        methodology.methods.insert(
            id.to_string(),
            MetricMethod {
                version: "1".to_string(),
                description: description.to_string(),
                parameters,
            },
        );
    }
}

fn build_comparison(
    days: &[NamedTokenMetricSet],
    observed_summary: &ObservedEventSummary,
    events: &[NormalizedEvent],
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) -> Result<(), super::IngestionError> {
    let Some(anchor) = observed_summary.maximum() else {
        set_family(
            report,
            "comparison",
            "unavailable",
            0,
            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
            vec!["comparison-window-outside-observed-envelope"],
        );
        return Ok(());
    };
    let Some(current_start) = anchor.checked_sub_signed(Duration::days(COMPARISON_DAYS - 1)) else {
        return Ok(());
    };
    let Some(current_end) = anchor.succ_opt() else {
        return Ok(());
    };
    let Some(prior_start) = current_start.checked_sub_signed(Duration::days(COMPARISON_DAYS))
    else {
        return Ok(());
    };

    let prior_window = insight_window(time_context, prior_start, current_start)?;
    let current_window = insight_window(time_context, current_start, current_end)?;
    let window_start_epoch = time_context
        .local_date_start_epoch(prior_start)
        .map_err(super::IngestionError::time)?;
    let window_end_epoch = time_context
        .local_date_start_epoch(current_end)
        .map_err(super::IngestionError::time)?;
    if time_context
        .period_bounds()
        .is_some_and(|(start, end)| window_start_epoch < start || window_end_epoch > end)
    {
        set_family(
            report,
            "comparison",
            "unavailable",
            0,
            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
            vec!["comparison-window-outside-period"],
        );
        return Ok(());
    }
    let observed_min = observed_summary.minimum();
    if observed_min.is_none_or(|date| date > prior_start) {
        set_family(
            report,
            "comparison",
            "unavailable",
            0,
            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
            vec!["comparison-window-outside-observed-envelope"],
        );
        return Ok(());
    }
    let prior_signature = observed_summary.window_signature(prior_start, current_start);
    let current_signature = observed_summary.window_signature(current_start, current_end);
    if prior_signature.is_empty()
        || current_signature.is_empty()
        || prior_signature != current_signature
    {
        set_family(
            report,
            "comparison",
            "unavailable",
            0,
            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
            vec!["comparison-incompatible-coverage"],
        );
        return Ok(());
    }
    let signature = prior_signature
        .iter()
        .cloned()
        .collect::<Vec<_>>()
        .join("|");
    let evidence_coverage = usage_evidence_coverage(coverage, events);
    if matches!(
        evidence_coverage.as_str(),
        "partial-canonical-usage" | "unavailable-canonical-usage"
    ) {
        set_family(
            report,
            "comparison",
            "unavailable",
            0,
            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
            vec!["comparison-partial-source"],
        );
        return Ok(());
    }

    let mut prior = daily_window(days, prior_start, current_start);
    let mut current = daily_window(days, current_start, current_end);
    prior.active_days = active_usage_days(events, time_context, prior_start, current_start);
    current.active_days = active_usage_days(events, time_context, current_start, current_end);
    let sample_count = prior.sample_count.saturating_add(current.sample_count);
    if !prior.exact || !current.exact {
        set_family(
            report,
            "comparison",
            "unavailable",
            sample_count,
            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
            vec!["comparison-incompatible-coverage"],
        );
        return Ok(());
    }
    if prior.active_days < COMPARISON_MINIMUM_ACTIVE_DAYS
        || current.active_days < COMPARISON_MINIMUM_ACTIVE_DAYS
    {
        set_family(
            report,
            "comparison",
            "unavailable",
            sample_count,
            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
            vec!["comparison-minimum-active-days"],
        );
        return Ok(());
    }

    let limitations = usage_evidence_limitations(coverage, events);
    let confidence = evidence_confidence(
        &evidence_coverage,
        sample_count,
        COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
    );
    let prior_fact = exact_fact(
        "insight.fact.comparison.output.prior",
        "tokens.output",
        prior.value.to_string(),
        "tokens",
        COMPARISON_METHOD,
        prior_window.clone(),
        prior.sample_count,
        evidence_coverage.clone(),
        "canonical",
    );
    let prior_active_days_fact = exact_fact(
        "insight.fact.comparison.output.prior-active-days",
        "comparison.prior-active-days",
        prior.active_days.to_string(),
        "days",
        COMPARISON_METHOD,
        prior_window.clone(),
        prior.sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let prior_signature_fact = exact_fact(
        "insight.fact.comparison.output.prior-coverage-signature",
        "comparison.prior-coverage-signature",
        signature.clone(),
        "coverage-signature",
        COMPARISON_METHOD,
        prior_window.clone(),
        prior.sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let current_fact = exact_fact(
        "insight.fact.comparison.output.current",
        "tokens.output",
        current.value.to_string(),
        "tokens",
        COMPARISON_METHOD,
        current_window.clone(),
        current.sample_count,
        evidence_coverage.clone(),
        "canonical",
    );
    let current_active_days_fact = exact_fact(
        "insight.fact.comparison.output.current-active-days",
        "comparison.current-active-days",
        current.active_days.to_string(),
        "days",
        COMPARISON_METHOD,
        current_window.clone(),
        current.sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let current_signature_fact = exact_fact(
        "insight.fact.comparison.output.current-coverage-signature",
        "comparison.current-coverage-signature",
        signature,
        "coverage-signature",
        COMPARISON_METHOD,
        current_window.clone(),
        current.sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let supporting_facts = vec![
        prior_fact.clone(),
        prior_active_days_fact,
        prior_signature_fact,
        current_fact.clone(),
        current_active_days_fact,
        current_signature_fact,
    ];
    let absolute_delta = signed_delta(current.value, prior.value);
    let relative_delta_pct = (prior.value > 0)
        .then(|| round6((current.value as f64 / prior.value as f64 - 1.0) * 100.0));
    report.cards.push(InsightCard {
        id: "comparison.output-tokens.v1".to_string(),
        version: "1".to_string(),
        family: "comparison".to_string(),
        class: "factual".to_string(),
        title: "Observed output tokens · adjacent windows".to_string(),
        finding: format!(
            "Observed output tokens changed by {absolute_delta} across adjacent 28-day windows."
        ),
        metric_id: "tokens.output".to_string(),
        comparison: Some(InsightComparison {
            baseline_fact_id: prior_fact.id.clone(),
            current_fact_id: current_fact.id.clone(),
            baseline_value: prior_fact.value.clone(),
            current_value: current_fact.value.clone(),
            absolute_delta,
            relative_delta_pct,
        }),
        window: current_window,
        sample_count,
        minimum_sample_count: COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
        method_id: COMPARISON_METHOD.to_string(),
        availability: if limitations.is_empty() {
            "available"
        } else {
            "partial"
        }
        .to_string(),
        coverage: evidence_coverage,
        confidence,
        supporting_facts,
        limitations: limitations.clone(),
        action: None,
        privacy_class: "standard".to_string(),
        renderer_priority: 100,
    });
    set_family(
        report,
        "comparison",
        if limitations.is_empty() {
            "available"
        } else {
            "partial"
        },
        sample_count,
        COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
        limitations.iter().map(String::as_str).collect(),
    );
    Ok(())
}

fn build_trend(
    days: &[NamedTokenMetricSet],
    events: &[NormalizedEvent],
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) {
    let mut points = exact_daily_points(days);
    let evidence_coverage = usage_evidence_coverage(coverage, events);
    if matches!(
        evidence_coverage.as_str(),
        "partial-canonical-usage" | "unavailable-canonical-usage"
    ) || points.iter().any(|point| !point.exact)
    {
        set_family(
            report,
            "trend",
            "unavailable",
            points.len(),
            TREND_MINIMUM_POINTS,
            vec!["trend-partial-daily-facts"],
        );
        return;
    }
    points.retain(|point| point.exact);
    if points.len() > TREND_MAXIMUM_POINTS {
        points.drain(..points.len() - TREND_MAXIMUM_POINTS);
    }
    if points.len() % 2 == 1 {
        points.remove(0);
    }
    if points.len() < TREND_MINIMUM_POINTS {
        set_family(
            report,
            "trend",
            "unavailable",
            points.len(),
            TREND_MINIMUM_POINTS,
            vec!["trend-minimum-points"],
        );
        return;
    }
    let half = points.len() / 2;
    let earlier = median(&points[..half]);
    let later = median(&points[half..]);
    let threshold = if earlier == 0 {
        100
    } else {
        100u128.max(div_ceil(earlier.saturating_mul(10), 100))
    };
    let delta = signed_delta(later, earlier);
    let direction = if later >= earlier.saturating_add(threshold) {
        "rose"
    } else if earlier >= later.saturating_add(threshold) {
        "fell"
    } else {
        "stable"
    };
    let start = points.first().map(|point| point.date).unwrap();
    let end = points
        .last()
        .and_then(|point| point.date.succ_opt())
        .unwrap();
    let window = date_window(time_context, start, end);
    let limitations = usage_evidence_limitations(coverage, events);
    let sample_count = points.len();
    let first_date = start.format("%Y-%m-%d").to_string();
    let last_date = points
        .last()
        .map(|point| point.date.format("%Y-%m-%d").to_string())
        .unwrap();
    let direction_fact = exact_fact(
        "insight.fact.trend.output.direction",
        "trend.direction",
        direction.to_string(),
        "direction",
        TREND_METHOD,
        window.clone(),
        sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let earlier_fact = exact_fact(
        "insight.fact.trend.output.earlier-median",
        "tokens.output.daily-median",
        earlier.to_string(),
        "tokens",
        TREND_METHOD,
        date_window(time_context, start, points[half].date),
        half,
        evidence_coverage.clone(),
        "canonical",
    );
    let first_date_fact = exact_fact(
        "insight.fact.trend.output.first-observed-date",
        "trend.first-observed-date",
        first_date,
        "local-date",
        TREND_METHOD,
        window.clone(),
        sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let half_size_fact = exact_fact(
        "insight.fact.trend.output.half-size",
        "trend.half-size",
        half.to_string(),
        "points",
        TREND_METHOD,
        window.clone(),
        sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let last_date_fact = exact_fact(
        "insight.fact.trend.output.last-observed-date",
        "trend.last-observed-date",
        last_date,
        "local-date",
        TREND_METHOD,
        window.clone(),
        sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let later_fact = exact_fact(
        "insight.fact.trend.output.later-median",
        "tokens.output.daily-median",
        later.to_string(),
        "tokens",
        TREND_METHOD,
        date_window(time_context, points[half].date, end),
        half,
        evidence_coverage.clone(),
        "canonical",
    );
    let point_count_fact = exact_fact(
        "insight.fact.trend.output.point-count",
        "trend.point-count",
        sample_count.to_string(),
        "points",
        TREND_METHOD,
        window.clone(),
        sample_count,
        evidence_coverage.clone(),
        "derived",
    );
    let threshold_fact = exact_fact(
        "insight.fact.trend.output.threshold",
        "trend.direction-threshold",
        threshold.to_string(),
        "tokens",
        TREND_METHOD,
        window.clone(),
        half,
        evidence_coverage.clone(),
        "derived",
    );
    report.cards.push(InsightCard {
        id: "trend.output-tokens.v1".to_string(),
        version: "1".to_string(),
        family: "trend".to_string(),
        class: "factual".to_string(),
        title: "Observed output-token trend".to_string(),
        finding: format!(
            "The later daily median {direction} relative to the earlier observed half."
        ),
        metric_id: "tokens.output.daily-median".to_string(),
        comparison: Some(InsightComparison {
            baseline_fact_id: earlier_fact.id.clone(),
            current_fact_id: later_fact.id.clone(),
            baseline_value: earlier_fact.value.clone(),
            current_value: later_fact.value.clone(),
            absolute_delta: delta,
            relative_delta_pct: (earlier > 0)
                .then(|| round6((later as f64 / earlier as f64 - 1.0) * 100.0)),
        }),
        window,
        sample_count,
        minimum_sample_count: TREND_MINIMUM_POINTS,
        method_id: TREND_METHOD.to_string(),
        availability: if limitations.is_empty() {
            "available"
        } else {
            "partial"
        }
        .to_string(),
        coverage: evidence_coverage.clone(),
        confidence: evidence_confidence(&evidence_coverage, points.len(), TREND_MINIMUM_POINTS),
        supporting_facts: vec![
            direction_fact,
            earlier_fact,
            first_date_fact,
            half_size_fact,
            last_date_fact,
            later_fact,
            point_count_fact,
            threshold_fact,
        ],
        limitations: limitations.clone(),
        action: None,
        privacy_class: "share".to_string(),
        renderer_priority: 110,
    });
    set_family(
        report,
        "trend",
        if limitations.is_empty() {
            "available"
        } else {
            "partial"
        },
        points.len(),
        TREND_MINIMUM_POINTS,
        limitations.iter().map(String::as_str).collect(),
    );
}

fn build_active_efficiency(
    events: &[NormalizedEvent],
    metrics: &CanonicalMetrics,
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) {
    let active = &metrics.active_time;
    let output = &metrics.tokens.global.output;
    let request_count = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::AssistantUsage | EventKind::OtelApiRequest
            )
        })
        .count();
    let start = events
        .iter()
        .filter_map(|event| time_context.local_date_epoch(event.epoch_nanos))
        .min();
    let end = events
        .iter()
        .filter_map(|event| time_context.local_date_epoch(event.epoch_nanos))
        .max()
        .and_then(|date| date.succ_opt());
    let window = match (start, end) {
        (Some(start), Some(end)) => date_window(time_context, start, end),
        _ => empty_window(time_context),
    };
    let exact_active = active.availability == "available"
        && active.total_active_seconds >= EFFICIENCY_MINIMUM_ACTIVE_SECONDS;
    if !exact_active {
        set_family(
            report,
            "active-efficiency",
            "unavailable",
            request_count,
            EFFICIENCY_MINIMUM_REQUESTS,
            vec!["efficiency-minimum-active-seconds"],
        );
        return;
    }
    let mut cards = 0usize;
    let base_limitations = observed_limitations(coverage);
    let mut family_limitations = base_limitations.clone();
    if output.availability == "available" && !output.overflowed && output.sample_count > 0 {
        let card_limitations = base_limitations.clone();
        let rate = rate_per_hour(output.observed, active.total_active_seconds);
        let numerator = exact_fact(
            "insight.fact.efficiency.output-tokens",
            "tokens.output",
            output.observed.to_string(),
            "tokens",
            &output.method_id,
            window.clone(),
            output.sample_count,
            output.availability.clone(),
            "canonical",
        );
        let denominator = exact_fact(
            "insight.fact.efficiency.active-seconds",
            "activity.active",
            active.total_active_seconds.to_string(),
            "seconds",
            &active.method_id,
            window.clone(),
            active.interval_count,
            active.availability.clone(),
            "canonical",
        );
        report.cards.push(InsightCard {
            id: "efficiency.output-tokens-per-active-hour.v1".to_string(),
            version: "1".to_string(),
            family: "active-efficiency".to_string(),
            class: "factual".to_string(),
            title: "Observed output per active hour".to_string(),
            finding: format!(
                "{} output tokens per observed unioned active hour.",
                decimal(rate)
            ),
            metric_id: "efficiency.output-tokens-per-active-hour".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: output.sample_count,
            minimum_sample_count: 1,
            method_id: EFFICIENCY_METHOD.to_string(),
            availability: if card_limitations.is_empty() {
                "available"
            } else {
                "partial"
            }
            .to_string(),
            coverage: coverage.completeness.clone(),
            confidence: confidence(coverage, output.sample_count, 1),
            supporting_facts: vec![
                numerator,
                denominator,
                exact_fact(
                    "insight.fact.efficiency.output-rate",
                    "efficiency.output-tokens-per-active-hour",
                    decimal(rate),
                    "tokens/hour",
                    EFFICIENCY_METHOD,
                    window.clone(),
                    output.sample_count,
                    coverage.completeness.clone(),
                    "derived",
                ),
            ],
            limitations: card_limitations,
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 120,
        });
        cards = cards.saturating_add(1);
    } else {
        family_limitations.push("efficiency-output-unavailable".to_string());
    }
    if request_count >= EFFICIENCY_MINIMUM_REQUESTS {
        let card_limitations = base_limitations.clone();
        let rate = rate_per_hour(request_count as u64, active.total_active_seconds);
        report.cards.push(InsightCard {
            id: "efficiency.requests-per-active-hour.v1".to_string(),
            version: "1".to_string(),
            family: "active-efficiency".to_string(),
            class: "factual".to_string(),
            title: "Observed requests per active hour".to_string(),
            finding: format!(
                "{} canonical request/message observations per unioned active hour.",
                decimal(rate)
            ),
            metric_id: "efficiency.requests-per-active-hour".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: request_count,
            minimum_sample_count: EFFICIENCY_MINIMUM_REQUESTS,
            method_id: EFFICIENCY_METHOD.to_string(),
            availability: if card_limitations.is_empty() {
                "available"
            } else {
                "partial"
            }
            .to_string(),
            coverage: coverage.completeness.clone(),
            confidence: confidence(coverage, request_count, EFFICIENCY_MINIMUM_REQUESTS),
            supporting_facts: vec![
                exact_fact(
                    "insight.fact.efficiency.requests",
                    "request.canonical-count",
                    request_count.to_string(),
                    "observations",
                    EFFICIENCY_METHOD,
                    window.clone(),
                    request_count,
                    coverage.completeness.clone(),
                    "canonical",
                ),
                exact_fact(
                    "insight.fact.efficiency.request-active-seconds",
                    "activity.active",
                    active.total_active_seconds.to_string(),
                    "seconds",
                    &active.method_id,
                    window.clone(),
                    active.interval_count,
                    active.availability.clone(),
                    "canonical",
                ),
                exact_fact(
                    "insight.fact.efficiency.request-rate",
                    "efficiency.requests-per-active-hour",
                    decimal(rate),
                    "observations/hour",
                    EFFICIENCY_METHOD,
                    window.clone(),
                    request_count,
                    coverage.completeness.clone(),
                    "derived",
                ),
            ],
            limitations: card_limitations,
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 121,
        });
        cards = cards.saturating_add(1);
    } else {
        family_limitations.push("efficiency-minimum-request-observations".to_string());
    }
    if metrics.cost.coverage == "available" {
        if let Some(amount) = metrics
            .cost
            .local_api_equivalent
            .amount_usd
            .filter(|amount| amount.is_finite())
        {
            let card_limitations = base_limitations.clone();
            let rate = round6(amount * 3_600.0 / active.total_active_seconds as f64);
            report.cards.push(InsightCard {
                id: "efficiency.local-api-equivalent-per-active-hour.v1".to_string(),
                version: "1".to_string(),
                family: "active-efficiency".to_string(),
                class: "factual".to_string(),
                title: "Observed local API-equivalent estimate per active hour".to_string(),
                finding: format!(
                    "${} local API-equivalent estimate per observed unioned active hour.",
                    decimal(rate)
                ),
                metric_id: "efficiency.local-api-equivalent-per-active-hour".to_string(),
                comparison: None,
                window: window.clone(),
                sample_count: metrics.cost.local_api_equivalent.sample_count,
                minimum_sample_count: 1,
                method_id: EFFICIENCY_METHOD.to_string(),
                availability: if card_limitations.is_empty() {
                    "available"
                } else {
                    "partial"
                }
                .to_string(),
                coverage: metrics.cost.coverage.clone(),
                confidence: confidence(coverage, metrics.cost.local_api_equivalent.sample_count, 1),
                supporting_facts: vec![
                    exact_fact(
                        "insight.fact.efficiency.local-api-equivalent",
                        "cost.local-api-equivalent",
                        decimal(amount),
                        "USD",
                        &metrics.cost.local_api_equivalent.method_id,
                        window.clone(),
                        metrics.cost.local_api_equivalent.sample_count,
                        metrics.cost.coverage.clone(),
                        "local-pricing-registry",
                    ),
                    exact_fact(
                        "insight.fact.efficiency.cost-active-seconds",
                        "activity.active",
                        active.total_active_seconds.to_string(),
                        "seconds",
                        &active.method_id,
                        window.clone(),
                        active.interval_count,
                        active.availability.clone(),
                        "canonical",
                    ),
                    exact_fact(
                        "insight.fact.efficiency.local-cost-rate",
                        "efficiency.local-api-equivalent-per-active-hour",
                        decimal(rate),
                        "USD/hour",
                        EFFICIENCY_METHOD,
                        window.clone(),
                        metrics.cost.local_api_equivalent.sample_count,
                        metrics.cost.coverage.clone(),
                        "derived",
                    ),
                ],
                limitations: card_limitations,
                action: None,
                privacy_class: "share".to_string(),
                renderer_priority: 122,
            });
            cards = cards.saturating_add(1);
        } else {
            family_limitations.push("efficiency-local-cost-unavailable".to_string());
        }
    } else {
        family_limitations.push("efficiency-local-cost-incomplete".to_string());
    }
    let terminal_outcomes = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::OtelApiRequest | EventKind::OtelApiError
            )
        })
        .count();
    if capability_available(coverage, "direct_terminal_outcomes") && terminal_outcomes > 0 {
        let terminal_errors = events
            .iter()
            .filter(|event| event.kind == EventKind::OtelApiError)
            .count();
        let rate = rate_per_hour(terminal_errors as u64, active.total_active_seconds);
        report.cards.push(InsightCard {
            id: "efficiency.terminal-errors-per-active-hour.v1".to_string(),
            version: "1".to_string(),
            family: "active-efficiency".to_string(),
            class: "factual".to_string(),
            title: "Observed terminal API errors per active hour".to_string(),
            finding: format!(
                "{} direct terminal API errors per observed unioned active hour.",
                decimal(rate)
            ),
            metric_id: "efficiency.terminal-errors-per-active-hour".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: terminal_outcomes,
            minimum_sample_count: 1,
            method_id: EFFICIENCY_METHOD.to_string(),
            availability: "available".to_string(),
            coverage: "complete-direct-otel".to_string(),
            confidence: sample_confidence(terminal_outcomes, 1),
            supporting_facts: vec![
                exact_fact(
                    "insight.fact.efficiency.terminal-errors",
                    "api.terminal-errors",
                    terminal_errors.to_string(),
                    "errors",
                    EFFICIENCY_METHOD,
                    window.clone(),
                    terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.efficiency.error-active-seconds",
                    "activity.active",
                    active.total_active_seconds.to_string(),
                    "seconds",
                    &active.method_id,
                    window.clone(),
                    active.interval_count,
                    active.availability.clone(),
                    "canonical",
                ),
                exact_fact(
                    "insight.fact.efficiency.terminal-error-rate",
                    "efficiency.terminal-errors-per-active-hour",
                    decimal(rate),
                    "errors/hour",
                    EFFICIENCY_METHOD,
                    window,
                    terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
            ],
            limitations: Vec::new(),
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 123,
        });
        cards = cards.saturating_add(1);
    } else {
        family_limitations.push("efficiency-direct-terminal-errors-unavailable".to_string());
    }
    set_family(
        report,
        "active-efficiency",
        if cards == 0 {
            "unavailable"
        } else if family_limitations.is_empty() {
            "available"
        } else {
            "partial"
        },
        request_count,
        EFFICIENCY_MINIMUM_REQUESTS,
        family_limitations.iter().map(String::as_str).collect(),
    );
}

#[derive(Debug, Default)]
struct ReliabilitySummary {
    terminal_outcomes: usize,
    terminal_errors: usize,
    terminal_error_rate_pct: Option<f64>,
}

fn build_reliability(
    events: &[NormalizedEvent],
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) -> ReliabilitySummary {
    const MINIMUM: usize = 10;
    let terminal_outcomes_complete = capability_available(coverage, "direct_terminal_outcomes");
    let retry_evidence_complete = capability_available(coverage, "retry_evidence");
    let requests = events
        .iter()
        .filter(|event| event.kind == EventKind::OtelApiRequest)
        .collect::<Vec<_>>();
    let errors = events
        .iter()
        .filter(|event| event.kind == EventKind::OtelApiError)
        .count();
    let terminal_outcomes = requests.len().saturating_add(errors);
    let terminal_error_rate_pct = (terminal_outcomes_complete && terminal_outcomes >= MINIMUM)
        .then(|| percent(errors as u128, terminal_outcomes as u128));
    let mut summary = ReliabilitySummary {
        terminal_outcomes,
        terminal_errors: errors,
        terminal_error_rate_pct,
    };
    let window = event_window(events, time_context);
    let mut card_count = 0usize;
    let mut limitations = Vec::<String>::new();

    if let Some(rate) = terminal_error_rate_pct {
        report.cards.push(InsightCard {
            id: "reliability.api-terminal-error-rate.v1".to_string(),
            version: "1".to_string(),
            family: "reliability".to_string(),
            class: "factual".to_string(),
            title: "Terminal API outcome rate".to_string(),
            finding: format!(
                "{}% of {terminal_outcomes} direct terminal API outcomes were errors emitted after retries were exhausted.",
                decimal(rate)
            ),
            metric_id: "reliability.api-terminal-error-rate".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: terminal_outcomes,
            minimum_sample_count: MINIMUM,
            method_id: RELIABILITY_METHOD.to_string(),
            availability: "available".to_string(),
            coverage: "complete-direct-otel".to_string(),
            confidence: sample_confidence(terminal_outcomes, MINIMUM),
            supporting_facts: vec![
                exact_fact(
                    "insight.fact.reliability.api-terminal-outcomes",
                    "api.terminal-outcomes",
                    terminal_outcomes.to_string(),
                    "outcomes",
                    RELIABILITY_METHOD,
                    window.clone(),
                    terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.reliability.api-terminal-errors",
                    "api.terminal-errors",
                    errors.to_string(),
                    "errors",
                    RELIABILITY_METHOD,
                    window.clone(),
                    errors,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.reliability.api-terminal-error-rate",
                    "reliability.api-terminal-error-rate",
                    decimal(rate),
                    "percent",
                    RELIABILITY_METHOD,
                    window.clone(),
                    terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
            ],
            limitations: Vec::new(),
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 130,
        });
        card_count = card_count.saturating_add(1);
    } else if !terminal_outcomes_complete {
        limitations.push("reliability-direct-otel-unavailable".to_string());
    } else {
        limitations.push("reliability-minimum-terminal-outcomes".to_string());
    }

    let attempts = requests
        .iter()
        .filter_map(|event| event.retry_count.map(|retries| (*event, retries)))
        .collect::<Vec<_>>();
    if retry_evidence_complete && attempts.len() >= MINIMUM {
        let recovered = attempts.iter().filter(|(_, retries)| *retries > 0).count();
        let total_retries = attempts
            .iter()
            .fold(0u64, |sum, (_, retries)| sum.saturating_add(*retries));
        let rate = percent(recovered as u128, attempts.len() as u128);
        report.cards.push(InsightCard {
            id: "reliability.api-recovered-retry-rate.v1".to_string(),
            version: "1".to_string(),
            family: "reliability".to_string(),
            class: "factual".to_string(),
            title: "Recovered retries on completed requests".to_string(),
            finding: format!(
                "{}% of {} completed direct requests with attempt evidence recovered after at least one retry.",
                decimal(rate),
                attempts.len()
            ),
            metric_id: "reliability.api-recovered-retry-rate".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: attempts.len(),
            minimum_sample_count: MINIMUM,
            method_id: RELIABILITY_METHOD.to_string(),
            availability: "available".to_string(),
            coverage: "complete-direct-otel".to_string(),
            confidence: sample_confidence(attempts.len(), MINIMUM),
            supporting_facts: vec![
                exact_fact(
                    "insight.fact.reliability.api-attempt-evidence",
                    "api.completed-with-attempt-evidence",
                    attempts.len().to_string(),
                    "requests",
                    RELIABILITY_METHOD,
                    window.clone(),
                    attempts.len(),
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.reliability.api-recovered-requests",
                    "api.recovered-requests",
                    recovered.to_string(),
                    "requests",
                    RELIABILITY_METHOD,
                    window.clone(),
                    recovered,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.reliability.api-retry-count",
                    "api.recovered-retry-count",
                    total_retries.to_string(),
                    "retries",
                    RELIABILITY_METHOD,
                    window.clone(),
                    attempts.len(),
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.reliability.api-recovered-retry-rate",
                    "reliability.api-recovered-retry-rate",
                    decimal(rate),
                    "percent",
                    RELIABILITY_METHOD,
                    window,
                    attempts.len(),
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
            ],
            limitations: Vec::new(),
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 131,
        });
        card_count = card_count.saturating_add(1);
    } else if retry_evidence_complete {
        limitations.push("reliability-minimum-attempt-evidence".to_string());
    } else {
        limitations.push("reliability-direct-attempt-evidence-unavailable".to_string());
    }
    set_family_owned(
        report,
        "reliability",
        if card_count == 0 {
            "unavailable"
        } else if limitations.is_empty() {
            "available"
        } else {
            "partial"
        },
        terminal_outcomes.max(attempts.len()),
        MINIMUM,
        limitations,
    );
    if terminal_error_rate_pct.is_none() {
        summary.terminal_error_rate_pct = None;
    }
    summary
}

#[derive(Debug, Clone, Default)]
struct ToolSummary {
    name: String,
    card_id: String,
    results: usize,
    failures: usize,
    failure_rate_pct: Option<f64>,
    recommendation_eligible: bool,
}

#[derive(Debug, Default)]
struct ToolAccumulator {
    occurrences: usize,
    results: usize,
    successes: usize,
    failures: usize,
    latencies: Vec<f64>,
    decisions: usize,
    accepts: usize,
}

fn build_tool_behavior(
    events: &[NormalizedEvent],
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) -> Vec<ToolSummary> {
    const MINIMUM: usize = 5;
    const MAXIMUM: usize = 10;
    let direct_results_complete = capability_available(coverage, "tool_result");
    let result_status_complete = capability_available(coverage, "tool_status");
    let result_latency_complete = capability_available(coverage, "tool_latency");
    let edit_decisions_complete = capability_available(coverage, "edit_decision");
    let direct_result_coverage = if direct_results_complete {
        "complete-direct-otel"
    } else {
        "partial-direct-otel"
    };
    let mut tools = BTreeMap::<String, ToolAccumulator>::new();
    for event in events {
        for name in &event.tool_names {
            let tool = tools.entry(name.clone()).or_default();
            if event.kind == EventKind::AssistantUsage {
                tool.occurrences = tool.occurrences.saturating_add(1);
            }
            if event.kind == EventKind::OtelToolResult {
                tool.results = tool.results.saturating_add(1);
                match event.tool_status.as_deref() {
                    Some("success") => tool.successes = tool.successes.saturating_add(1),
                    Some("error") => tool.failures = tool.failures.saturating_add(1),
                    _ => {}
                }
                if let Some(latency) = event.latency_ms.filter(|value| value.is_finite()) {
                    tool.latencies.push(latency);
                }
            }
            if event.kind == EventKind::OtelToolDecision {
                if let Some(decision) = event.edit_decision.as_deref() {
                    tool.decisions = tool.decisions.saturating_add(1);
                    if decision == "accept" {
                        tool.accepts = tool.accepts.saturating_add(1);
                    }
                }
            }
        }
    }
    let mut ranked = tools.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|(left_name, left), (right_name, right)| {
        right
            .results
            .cmp(&left.results)
            .then_with(|| right.occurrences.cmp(&left.occurrences))
            .then_with(|| left_name.cmp(right_name))
    });
    let total_samples = ranked.iter().fold(0usize, |sum, (_, tool)| {
        sum.saturating_add(tool.results)
            .saturating_add(tool.occurrences)
            .saturating_add(tool.decisions)
    });
    let mut summaries = ranked
        .iter()
        .map(|(name, tool)| {
            let result_rate_available = direct_results_complete
                && result_status_complete
                && tool.results >= MINIMUM
                && tool.successes.saturating_add(tool.failures) == tool.results;
            ToolSummary {
                name: name.clone(),
                card_id: format!("tool.{name}.observed-outcomes.v1"),
                results: tool.results,
                failures: tool.failures,
                failure_rate_pct: result_rate_available
                    .then(|| percent(tool.failures as u128, tool.results as u128)),
                recommendation_eligible: result_rate_available,
            }
        })
        .collect::<Vec<_>>();
    let mut recommendation_candidates = summaries
        .iter()
        .enumerate()
        .filter(|(_, tool)| {
            tool.recommendation_eligible
                && tool.results >= 10
                && tool.failure_rate_pct.is_some_and(|rate| rate >= 20.0)
        })
        .collect::<Vec<_>>();
    recommendation_candidates.sort_by(|(_, left), (_, right)| {
        right
            .failure_rate_pct
            .unwrap()
            .total_cmp(&left.failure_rate_pct.unwrap())
            .then_with(|| right.failures.cmp(&left.failures))
            .then_with(|| right.results.cmp(&left.results))
            .then_with(|| left.name.cmp(&right.name))
    });
    let trigger_summary = recommendation_candidates
        .first()
        .map(|(index, _)| *index)
        .filter(|index| *index >= MAXIMUM);
    ranked.truncate(if trigger_summary.is_some() {
        MAXIMUM.saturating_sub(1)
    } else {
        MAXIMUM
    });
    let window = event_window(events, time_context);
    let mut available_cards = 0usize;
    for (rank, (name, mut tool)) in ranked.into_iter().enumerate() {
        let result_rate_available = direct_results_complete
            && result_status_complete
            && tool.results >= MINIMUM
            && tool.successes.saturating_add(tool.failures) == tool.results;
        let failure_rate_pct =
            result_rate_available.then(|| percent(tool.failures as u128, tool.results as u128));
        let decision_available = edit_decisions_complete && tool.decisions >= MINIMUM;
        let latency_available = result_latency_complete && tool.latencies.len() >= MINIMUM;
        if tool.occurrences == 0 && tool.results == 0 && tool.decisions == 0 {
            continue;
        }
        tool.latencies.sort_by(f64::total_cmp);
        let median_latency = latency_available.then(|| median_f64(&tool.latencies));
        let p95_latency = latency_available.then(|| nearest_rank(&tool.latencies, 95));
        let mut facts = Vec::new();
        if tool.occurrences > 0 {
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.occurrences"),
                "tool.occurrences",
                tool.occurrences.to_string(),
                "occurrences",
                TOOL_METHOD,
                window.clone(),
                tool.occurrences,
                coverage.completeness.clone(),
                "normalized-event",
            ));
        }
        if tool.results > 0 {
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.results"),
                "tool.direct-results",
                tool.results.to_string(),
                "results",
                TOOL_METHOD,
                window.clone(),
                tool.results,
                direct_result_coverage.to_string(),
                "otel-event",
            ));
        }
        if let Some(rate) = failure_rate_pct {
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.failures"),
                "tool.direct-failures",
                tool.failures.to_string(),
                "errors",
                TOOL_METHOD,
                window.clone(),
                tool.results,
                "complete-direct-otel".to_string(),
                "otel-event",
            ));
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.failure-rate"),
                "tool.direct-failure-rate",
                decimal(rate),
                "percent",
                TOOL_METHOD,
                window.clone(),
                tool.results,
                "complete-direct-otel".to_string(),
                "derived",
            ));
        }
        if let (Some(median), Some(p95)) = (median_latency, p95_latency) {
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.latency-median"),
                "tool.duration-median",
                decimal(median),
                "milliseconds",
                TOOL_METHOD,
                window.clone(),
                tool.latencies.len(),
                "complete-direct-otel".to_string(),
                "derived",
            ));
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.latency-p95"),
                "tool.duration-p95",
                decimal(p95),
                "milliseconds",
                TOOL_METHOD,
                window.clone(),
                tool.latencies.len(),
                "complete-direct-otel".to_string(),
                "derived",
            ));
        }
        if tool.decisions > 0 {
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.edit-decisions"),
                "tool.edit-decisions",
                tool.decisions.to_string(),
                "decisions",
                TOOL_METHOD,
                window.clone(),
                tool.decisions,
                if edit_decisions_complete {
                    "complete-direct-otel"
                } else {
                    "partial-direct-otel"
                }
                .to_string(),
                "otel-event",
            ));
        }
        if decision_available {
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.edit-accepts"),
                "tool.edit-accepts",
                tool.accepts.to_string(),
                "decisions",
                TOOL_METHOD,
                window.clone(),
                tool.decisions,
                "complete-direct-otel".to_string(),
                "otel-event",
            ));
            facts.push(exact_fact(
                &format!("insight.fact.tool.{name}.edit-accept-share"),
                "tool.edit-accept-share",
                decimal(percent(tool.accepts as u128, tool.decisions as u128)),
                "percent",
                TOOL_METHOD,
                window.clone(),
                tool.decisions,
                "complete-direct-otel".to_string(),
                "derived",
            ));
        }
        let mut limitations = Vec::new();
        if !result_rate_available {
            limitations.push("tool-result-rate-unavailable".to_string());
        }
        if !latency_available {
            limitations.push("tool-latency-minimum-results".to_string());
        }
        if !decision_available {
            limitations.push("tool-edit-decision-minimum".to_string());
        }
        if tool.results == 0 {
            limitations.push("tool-direct-results-unavailable".to_string());
        }
        if tool.decisions == 0 {
            limitations.push("tool-edit-decisions-unavailable".to_string());
        }
        let sample_count = tool
            .results
            .max(tool.decisions)
            .max(tool.latencies.len())
            .max(tool.occurrences);
        let minimum_sample_count =
            if result_rate_available || decision_available || latency_available {
                MINIMUM
            } else {
                1
            };
        report.cards.push(InsightCard {
            id: format!("tool.{name}.observed-outcomes.v1"),
            version: "1".to_string(),
            family: "tool-behavior".to_string(),
            class: "factual".to_string(),
            title: format!("{name} · observed tool evidence"),
            finding: match failure_rate_pct {
                Some(rate) => format!(
                    "{}% of {} direct {name} results were errors.",
                    decimal(rate),
                    tool.results
                ),
                None if tool.results > 0 => {
                    format!("{name} has {} observed direct result(s).", tool.results)
                }
                None if tool.decisions > 0 => {
                    format!("{name} has {} observed edit decision(s).", tool.decisions)
                }
                None => format!("{name} has {} observed occurrence(s).", tool.occurrences),
            },
            metric_id: "tool.observed-outcomes".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count,
            minimum_sample_count,
            method_id: TOOL_METHOD.to_string(),
            availability: if limitations.is_empty() {
                "available"
            } else {
                "partial"
            }
            .to_string(),
            coverage: "capability-specific".to_string(),
            confidence: if limitations.is_empty() {
                sample_confidence(sample_count, minimum_sample_count)
            } else {
                "low".to_string()
            },
            supporting_facts: facts,
            limitations,
            action: None,
            privacy_class: "standard".to_string(),
            renderer_priority: 140u32.saturating_add(rank as u32),
        });
        available_cards = available_cards.saturating_add(1);
    }
    if let Some(index) = trigger_summary {
        let summary = &mut summaries[index];
        let rate = summary.failure_rate_pct.unwrap();
        summary.card_id = format!("tool.{}.recommendation-trigger.v1", summary.name);
        report.cards.push(InsightCard {
            id: summary.card_id.clone(),
            version: "1".to_string(),
            family: "tool-behavior".to_string(),
            class: "factual".to_string(),
            title: format!("{} · recommendation trigger evidence", summary.name),
            finding: format!(
                "{}% of {} direct {} results were errors.",
                decimal(rate),
                summary.results,
                summary.name
            ),
            metric_id: "tool.observed-outcomes".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: summary.results,
            minimum_sample_count: 10,
            method_id: TOOL_METHOD.to_string(),
            availability: "available".to_string(),
            coverage: "complete-direct-otel".to_string(),
            confidence: sample_confidence(summary.results, 10),
            supporting_facts: vec![
                exact_fact(
                    &format!(
                        "insight.fact.tool.{}.recommendation-trigger.results",
                        summary.name
                    ),
                    "tool.direct-results",
                    summary.results.to_string(),
                    "results",
                    TOOL_METHOD,
                    window.clone(),
                    summary.results,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    &format!(
                        "insight.fact.tool.{}.recommendation-trigger.failures",
                        summary.name
                    ),
                    "tool.direct-failures",
                    summary.failures.to_string(),
                    "errors",
                    TOOL_METHOD,
                    window.clone(),
                    summary.results,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    &format!(
                        "insight.fact.tool.{}.recommendation-trigger.failure-rate",
                        summary.name
                    ),
                    "tool.direct-failure-rate",
                    decimal(rate),
                    "percent",
                    TOOL_METHOD,
                    window,
                    summary.results,
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
            ],
            limitations: vec!["tool-trigger-outside-ranked-display".to_string()],
            action: None,
            privacy_class: "standard".to_string(),
            renderer_priority: 199,
        });
        available_cards = available_cards.saturating_add(1);
    }
    set_family_owned(
        report,
        "tool-behavior",
        if available_cards == 0 {
            "unavailable"
        } else {
            "partial"
        },
        total_samples,
        MINIMUM,
        if available_cards == 0 {
            vec!["tool-direct-outcome-evidence-unavailable".to_string()]
        } else {
            vec!["tool-capabilities-are-independent".to_string()]
        },
    );
    summaries
}

#[derive(Debug, Clone, Default)]
struct RoutingSummary {
    observations: usize,
    top_model: Option<String>,
    top_requests: usize,
    top_request_share_pct: Option<f64>,
    unknown_request_share_pct: Option<f64>,
    trigger_coverage: String,
    recommendation_eligible: bool,
}

#[derive(Debug, Clone, Default)]
struct ModelShare {
    requests: usize,
    output_tokens: u128,
    local_cost_usd: f64,
}

fn build_model_routing(
    events: &[NormalizedEvent],
    metrics: &CanonicalMetrics,
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) -> RoutingSummary {
    const MINIMUM: usize = 5;
    const MAXIMUM: usize = 10;
    let request_events = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::AssistantUsage | EventKind::OtelApiRequest
            )
        })
        .collect::<Vec<_>>();
    let request_evidence_coverage = routing_request_evidence_coverage(coverage, &request_events);
    let output_evidence_coverage = usage_evidence_coverage(coverage, events);
    let mut mapped = BTreeMap::<String, ModelShare>::new();
    let mut unknown_requests = 0usize;
    let mut unknown_output = 0u128;
    for event in &request_events {
        let Some(raw) = event.model.as_deref() else {
            unknown_requests = unknown_requests.saturating_add(1);
            unknown_output =
                unknown_output.saturating_add(u128::from(event.tokens.output.unwrap_or(0)));
            continue;
        };
        let Some(canonical) = super::pricing::canonical_model(raw) else {
            unknown_requests = unknown_requests.saturating_add(1);
            unknown_output =
                unknown_output.saturating_add(u128::from(event.tokens.output.unwrap_or(0)));
            continue;
        };
        let share = mapped.entry(canonical.to_string()).or_default();
        share.requests = share.requests.saturating_add(1);
        share.output_tokens = share
            .output_tokens
            .saturating_add(u128::from(event.tokens.output.unwrap_or(0)));
    }
    for model in &metrics.cost.models {
        let Some(canonical) = model.canonical_model.as_ref() else {
            continue;
        };
        mapped.entry(canonical.clone()).or_default().local_cost_usd +=
            model.local_api_equivalent_usd.unwrap_or(0.0);
    }
    let observations = request_events.len();
    let unknown_request_share_pct =
        (observations > 0).then(|| percent(unknown_requests as u128, observations as u128));
    if observations < MINIMUM {
        set_family_owned(
            report,
            "model-routing",
            "unavailable",
            observations,
            MINIMUM,
            vec!["routing-minimum-observations".to_string()],
        );
        return RoutingSummary {
            observations,
            unknown_request_share_pct,
            ..RoutingSummary::default()
        };
    }
    let mut ranked = mapped.into_iter().collect::<Vec<_>>();
    ranked.sort_by(|(left_name, left), (right_name, right)| {
        right
            .requests
            .cmp(&left.requests)
            .then_with(|| right.output_tokens.cmp(&left.output_tokens))
            .then_with(|| left_name.cmp(right_name))
    });
    let total_output = ranked.iter().fold(unknown_output, |sum, (_, model)| {
        sum.saturating_add(model.output_tokens)
    });
    let total_local_cost = ranked
        .iter()
        .map(|(_, model)| model.local_cost_usd)
        .sum::<f64>();
    let omitted = if ranked.len() > MAXIMUM {
        ranked.split_off(MAXIMUM)
    } else {
        Vec::new()
    };
    let omitted_requests = omitted
        .iter()
        .fold(0usize, |sum, (_, model)| sum.saturating_add(model.requests));
    let omitted_output = omitted.iter().fold(0u128, |sum, (_, model)| {
        sum.saturating_add(model.output_tokens)
    });
    let omitted_local_cost = omitted
        .iter()
        .map(|(_, model)| model.local_cost_usd)
        .sum::<f64>();
    let top_model = ranked.first().map(|(name, _)| name.clone());
    let top_requests = ranked.first().map(|(_, model)| model.requests).unwrap_or(0);
    let top_request_share_pct = ranked
        .first()
        .map(|(_, model)| percent(model.requests as u128, observations as u128));
    let window = event_window(events, time_context);
    let mut request_limitations = routing_request_evidence_limitations(&request_evidence_coverage);
    let mut family_limitations = request_limitations.clone();
    family_limitations.extend(usage_evidence_limitations(coverage, events));
    if unknown_requests > 0 {
        request_limitations.push("routing-unknown-model-observations".to_string());
        family_limitations.push("routing-unknown-model-observations".to_string());
    }
    let mut request_facts = ranked
        .iter()
        .map(|(name, model)| {
            exact_fact(
                &format!("insight.fact.routing.{name}.request-share"),
                "routing.model-request-share",
                decimal(percent(model.requests as u128, observations as u128)),
                "percent",
                ROUTING_METHOD,
                window.clone(),
                observations,
                request_evidence_coverage.clone(),
                "canonical",
            )
        })
        .chain(std::iter::once(exact_fact(
            "insight.fact.routing.unknown.request-share",
            "routing.unknown-model-request-share",
            decimal(unknown_request_share_pct.unwrap_or(0.0)),
            "percent",
            ROUTING_METHOD,
            window.clone(),
            observations,
            request_evidence_coverage.clone(),
            "canonical",
        )))
        .collect::<Vec<_>>();
    if omitted_requests > 0 {
        request_facts.push(exact_fact(
            "insight.fact.routing.other-mapped.request-share",
            "routing.other-mapped-request-share",
            decimal(percent(omitted_requests as u128, observations as u128)),
            "percent",
            ROUTING_METHOD,
            window.clone(),
            observations,
            request_evidence_coverage.clone(),
            "canonical",
        ));
    }
    report.cards.push(InsightCard {
        id: "routing.model-request-share.v1".to_string(),
        version: "1".to_string(),
        family: "model-routing".to_string(),
        class: "factual".to_string(),
        title: "Observed model request share".to_string(),
        finding: match (&top_model, top_request_share_pct) {
            (Some(model), Some(share)) => format!(
                "{model} represented {}% of canonical request/message observations.",
                decimal(share)
            ),
            _ => "Mapped model observations were unavailable.".to_string(),
        },
        metric_id: "routing.model-request-share".to_string(),
        comparison: None,
        window: window.clone(),
        sample_count: observations,
        minimum_sample_count: MINIMUM,
        method_id: ROUTING_METHOD.to_string(),
        availability: if request_limitations.is_empty() {
            "available"
        } else {
            "partial"
        }
        .to_string(),
        coverage: request_evidence_coverage.clone(),
        confidence: evidence_confidence(&request_evidence_coverage, observations, MINIMUM),
        supporting_facts: request_facts,
        limitations: request_limitations,
        action: None,
        privacy_class: "share".to_string(),
        renderer_priority: 150,
    });
    if total_output > 0 {
        let mut output_limitations = usage_evidence_limitations(coverage, events);
        if unknown_output > 0 {
            output_limitations.push("routing-unknown-model-output-tokens".to_string());
            family_limitations.push("routing-unknown-model-output-tokens".to_string());
        }
        let mut output_facts = ranked
            .iter()
            .map(|(name, model)| {
                exact_fact(
                    &format!("insight.fact.routing.{name}.output-share"),
                    "routing.model-output-token-share",
                    decimal(percent(model.output_tokens, total_output)),
                    "percent",
                    ROUTING_METHOD,
                    window.clone(),
                    observations,
                    output_evidence_coverage.clone(),
                    "canonical",
                )
            })
            .chain(std::iter::once(exact_fact(
                "insight.fact.routing.unknown.output-share",
                "routing.unknown-model-output-token-share",
                decimal(percent(unknown_output, total_output)),
                "percent",
                ROUTING_METHOD,
                window.clone(),
                observations,
                output_evidence_coverage.clone(),
                "canonical",
            )))
            .collect::<Vec<_>>();
        if omitted_output > 0 {
            output_facts.push(exact_fact(
                "insight.fact.routing.other-mapped.output-share",
                "routing.other-mapped-output-token-share",
                decimal(percent(omitted_output, total_output)),
                "percent",
                ROUTING_METHOD,
                window.clone(),
                observations,
                output_evidence_coverage.clone(),
                "canonical",
            ));
        }
        report.cards.push(InsightCard {
            id: "routing.model-output-token-share.v1".to_string(),
            version: "1".to_string(),
            family: "model-routing".to_string(),
            class: "factual".to_string(),
            title: "Observed model output-token share".to_string(),
            finding:
                "Canonical output-token shares are descriptive and include unknown-model coverage."
                    .to_string(),
            metric_id: "routing.model-output-token-share".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: observations,
            minimum_sample_count: MINIMUM,
            method_id: ROUTING_METHOD.to_string(),
            availability: if output_limitations.is_empty() {
                "available"
            } else {
                "partial"
            }
            .to_string(),
            coverage: output_evidence_coverage.clone(),
            confidence: evidence_confidence(&output_evidence_coverage, observations, MINIMUM),
            supporting_facts: output_facts,
            limitations: output_limitations,
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 151,
        });
    }
    if total_local_cost.is_finite()
        && total_local_cost > 0.0
        && metrics.cost.priced_requests >= MINIMUM
    {
        let mut cost_facts = ranked
            .iter()
            .filter(|(_, model)| model.local_cost_usd > 0.0)
            .map(|(name, model)| {
                exact_fact(
                    &format!("insight.fact.routing.{name}.local-cost-share"),
                    "routing.model-local-cost-share",
                    decimal(round6(model.local_cost_usd / total_local_cost * 100.0)),
                    "percent",
                    ROUTING_METHOD,
                    window.clone(),
                    observations,
                    metrics.cost.coverage.clone(),
                    "local-api-equivalent",
                )
            })
            .collect::<Vec<_>>();
        if omitted_local_cost.is_finite() && omitted_local_cost > 0.0 {
            cost_facts.push(exact_fact(
                "insight.fact.routing.other-mapped.local-cost-share",
                "routing.other-mapped-local-cost-share",
                decimal(round6(omitted_local_cost / total_local_cost * 100.0)),
                "percent",
                ROUTING_METHOD,
                window.clone(),
                observations,
                metrics.cost.coverage.clone(),
                "local-api-equivalent",
            ));
        }
        cost_facts.extend([
            exact_fact(
                "insight.fact.routing.priced-requests",
                "cost.priced-requests",
                metrics.cost.priced_requests.to_string(),
                "requests",
                ROUTING_METHOD,
                window.clone(),
                metrics.cost.priced_requests,
                metrics.cost.coverage.clone(),
                "local-api-equivalent",
            ),
            exact_fact(
                "insight.fact.routing.unpriced-requests",
                "cost.unpriced-requests",
                metrics.cost.unpriced_requests.to_string(),
                "requests",
                ROUTING_METHOD,
                window.clone(),
                metrics
                    .cost
                    .priced_requests
                    .saturating_add(metrics.cost.unpriced_requests),
                metrics.cost.coverage.clone(),
                "local-api-equivalent",
            ),
            exact_fact(
                "insight.fact.routing.priced-tokens",
                "cost.priced-tokens",
                metrics.cost.priced_tokens.to_string(),
                "tokens",
                ROUTING_METHOD,
                window.clone(),
                metrics.cost.priced_requests,
                metrics.cost.coverage.clone(),
                "local-api-equivalent",
            ),
            exact_fact(
                "insight.fact.routing.unpriced-tokens",
                "cost.unpriced-tokens",
                metrics.cost.unpriced_tokens.to_string(),
                "tokens",
                ROUTING_METHOD,
                window.clone(),
                metrics.cost.unpriced_requests,
                metrics.cost.coverage.clone(),
                "local-api-equivalent",
            ),
        ]);
        if !cost_facts.is_empty() {
            report.cards.push(InsightCard {
                id: "routing.model-local-cost-share.v1".to_string(),
                version: "1".to_string(),
                family: "model-routing".to_string(),
                class: "factual".to_string(),
                title: "Local API-equivalent model share".to_string(),
                finding: "Priced local API-equivalent estimate shares exclude source-recorded and billing domains.".to_string(),
                metric_id: "routing.model-local-cost-share".to_string(),
                comparison: None,
                window,
                sample_count: metrics.cost.priced_requests,
                minimum_sample_count: MINIMUM,
                method_id: ROUTING_METHOD.to_string(),
                availability: if metrics.cost.unpriced_requests == 0 {
                    "available"
                } else {
                    "partial"
                }
                .to_string(),
                coverage: metrics.cost.coverage.clone(),
                confidence: sample_confidence(metrics.cost.priced_requests, MINIMUM),
                supporting_facts: cost_facts,
                limitations: if metrics.cost.unpriced_requests == 0 {
                    Vec::new()
                } else {
                    vec!["routing-local-cost-unpriced-coverage".to_string()]
                },
                action: None,
                privacy_class: "share".to_string(),
                renderer_priority: 152,
            });
        }
    }
    family_limitations.sort();
    family_limitations.dedup();
    set_family_owned(
        report,
        "model-routing",
        if family_limitations.is_empty() {
            "available"
        } else {
            "partial"
        },
        observations,
        MINIMUM,
        family_limitations,
    );
    RoutingSummary {
        observations,
        top_model,
        top_requests,
        top_request_share_pct,
        unknown_request_share_pct,
        trigger_coverage: request_evidence_coverage.clone(),
        recommendation_eligible: request_evidence_coverage == "complete-canonical-usage",
    }
}

fn build_project_concentration(
    metrics: &CanonicalMetrics,
    events: &[NormalizedEvent],
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) {
    let mut projects = metrics
        .tokens
        .projects
        .iter()
        .filter(|project| {
            project.tokens.output.availability == "available"
                && !project.tokens.output.overflowed
                && project.tokens.output.observed > 0
        })
        .map(|project| {
            (
                project.key.clone(),
                u128::from(project.tokens.output.observed),
            )
        })
        .collect::<Vec<_>>();
    projects.sort_by(|(left_name, left), (right_name, right)| {
        right.cmp(left).then_with(|| left_name.cmp(right_name))
    });
    let known = projects
        .iter()
        .fold(0u128, |sum, (_, value)| sum.saturating_add(*value));
    let global = u128::from(metrics.tokens.global.output.observed);
    let unattributed = u128::from(metrics.tokens.project_unattributed.output.observed);
    if known == 0
        || metrics.tokens.global.output.availability != "available"
        || metrics.tokens.global.output.overflowed
    {
        set_family_owned(
            report,
            "project-concentration",
            "unavailable",
            projects.len(),
            1,
            vec!["concentration-positive-known-weight-required".to_string()],
        );
        return;
    }
    let hhi = projects
        .iter()
        .map(|(_, value)| {
            let share = *value as f64 / known as f64;
            share * share * 10_000.0
        })
        .sum::<f64>();
    let (top_alias, top_weight) = projects.first().cloned().unwrap();
    let top_share = percent(top_weight, known);
    let known_share = if global > 0 {
        percent(known, global)
    } else {
        0.0
    };
    let unattributed_share = if global > 0 {
        percent(unattributed, global)
    } else {
        0.0
    };
    let label = if hhi >= 2_500.0 || top_share >= 70.0 {
        "concentrated"
    } else if hhi <= 1_500.0 && projects.len() >= 4 && top_share < 50.0 {
        "distributed"
    } else {
        "mixed"
    };
    let observed_window = daily_points_window(&metrics.tokens.days, time_context);
    let window = period_window(time_context, &observed_window);
    let evidence_coverage = usage_evidence_coverage(coverage, events);
    let mut limitations = usage_evidence_limitations(coverage, events);
    if known_share < 90.0 {
        limitations.push("concentration-unattributed-output-coverage".to_string());
    }
    report.cards.push(InsightCard {
        id: "concentration.project-output-hhi.v1".to_string(),
        version: "1".to_string(),
        family: "project-concentration".to_string(),
        class: "factual".to_string(),
        title: "Observed project concentration".to_string(),
        finding: format!(
            "Known project output-token weights were {label} under the declared HHI thresholds."
        ),
        metric_id: "concentration.project-output-hhi".to_string(),
        comparison: None,
        window: window.clone(),
        sample_count: projects.len(),
        minimum_sample_count: 1,
        method_id: CONCENTRATION_METHOD.to_string(),
        availability: if limitations.is_empty() {
            "available"
        } else {
            "partial"
        }
        .to_string(),
        coverage: evidence_coverage.clone(),
        confidence: if known_share >= 90.0 {
            evidence_confidence(&evidence_coverage, projects.len(), 1)
        } else {
            "low".to_string()
        },
        supporting_facts: vec![
            exact_fact(
                "insight.fact.concentration.project-hhi",
                "concentration.project-output-hhi",
                decimal(round6(hhi)),
                "hhi-0-10000",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.concentration.project-count",
                "concentration.known-project-count",
                projects.len().to_string(),
                "projects",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.concentration.known-output-weight",
                "concentration.known-output-weight",
                known.to_string(),
                "tokens",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.concentration.unattributed-output-weight",
                "concentration.unattributed-output-weight",
                unattributed.to_string(),
                "tokens",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.concentration.top-project-share",
                "concentration.top-known-project-share",
                decimal(top_share),
                "percent",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "derived",
            ),
            exact_fact(
                "insight.fact.concentration.known-output-share",
                "concentration.known-output-share",
                decimal(known_share),
                "percent",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "derived",
            ),
            exact_fact(
                "insight.fact.concentration.unattributed-output-share",
                "concentration.unattributed-output-share",
                decimal(unattributed_share),
                "percent",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "derived",
            ),
        ],
        limitations: limitations.clone(),
        action: None,
        privacy_class: "share".to_string(),
        renderer_priority: 160,
    });
    report.cards.push(InsightCard {
        id: "concentration.top-project-alias.v1".to_string(),
        version: "1".to_string(),
        family: "project-concentration".to_string(),
        class: "factual".to_string(),
        title: "Observed top project alias".to_string(),
        finding: format!(
            "{top_alias} represented {}% of known attributed output tokens.",
            decimal(top_share)
        ),
        metric_id: "concentration.top-project-alias".to_string(),
        comparison: None,
        window: window.clone(),
        sample_count: projects.len(),
        minimum_sample_count: 1,
        method_id: CONCENTRATION_METHOD.to_string(),
        availability: if limitations.is_empty() {
            "available"
        } else {
            "partial"
        }
        .to_string(),
        coverage: evidence_coverage.clone(),
        confidence: if known_share >= 90.0 {
            evidence_confidence(&evidence_coverage, projects.len(), 1)
        } else {
            "low".to_string()
        },
        supporting_facts: vec![
            exact_fact(
                "insight.fact.concentration.top-project-alias",
                "concentration.top-project-alias",
                top_alias,
                "alias",
                CONCENTRATION_METHOD,
                window.clone(),
                projects.len(),
                evidence_coverage.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.concentration.alias-card-top-share",
                "concentration.top-known-project-share",
                decimal(top_share),
                "percent",
                CONCENTRATION_METHOD,
                window,
                projects.len(),
                evidence_coverage.clone(),
                "derived",
            ),
        ],
        limitations: limitations.clone(),
        action: None,
        privacy_class: "standard".to_string(),
        renderer_priority: 161,
    });
    set_family_owned(
        report,
        "project-concentration",
        if limitations.is_empty() {
            "available"
        } else {
            "partial"
        },
        projects.len(),
        1,
        limitations,
    );
}

fn build_anomalies(
    days: &[NamedTokenMetricSet],
    events: &[NormalizedEvent],
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) {
    const MINIMUM: usize = 7;
    const MAXIMUM: usize = 3;
    let points = exact_daily_points(days);
    let evidence_coverage = usage_evidence_coverage(coverage, events);
    if matches!(
        evidence_coverage.as_str(),
        "partial-canonical-usage" | "unavailable-canonical-usage"
    ) || points.len() < MINIMUM
        || points.iter().any(|point| !point.exact)
    {
        set_family_owned(
            report,
            "anomaly",
            "unavailable",
            points.len(),
            MINIMUM,
            vec![if points.len() < MINIMUM {
                "anomaly-minimum-points".to_string()
            } else {
                "anomaly-partial-daily-facts".to_string()
            }],
        );
        return;
    }
    let median_value = median(&points);
    let mut deviations = points
        .iter()
        .map(|point| absolute_difference(point.value, median_value))
        .collect::<Vec<_>>();
    deviations.sort_unstable();
    let mad = median_values(&deviations);
    let practical = 100u128.max(div_ceil(median_value.saturating_mul(25), 100));
    let mad_zero_practical = 1_000u128.max(median_value);
    let mut anomalies = points
        .iter()
        .filter_map(|point| {
            let deviation = absolute_difference(point.value, median_value);
            if mad == 0 {
                (deviation >= mad_zero_practical).then_some((
                    point,
                    None,
                    deviation,
                    mad_zero_practical,
                ))
            } else {
                let score = round6(0.67448975 * deviation as f64 / mad as f64);
                (score >= 3.5 && deviation >= practical).then_some((
                    point,
                    Some(score),
                    deviation,
                    practical,
                ))
            }
        })
        .collect::<Vec<_>>();
    anomalies.sort_by(
        |(left, left_score, left_deviation, _), (right, right_score, right_deviation, _)| {
            match (left_score, right_score) {
                (None, Some(_)) => std::cmp::Ordering::Less,
                (Some(_), None) => std::cmp::Ordering::Greater,
                (Some(left), Some(right)) => right.total_cmp(left),
                (None, None) => right_deviation.cmp(left_deviation),
            }
            .then_with(|| right_deviation.cmp(left_deviation))
            .then_with(|| left.date.cmp(&right.date))
        },
    );
    anomalies.truncate(MAXIMUM);
    let window = date_window(
        time_context,
        points.first().unwrap().date,
        points.last().unwrap().date.succ_opt().unwrap(),
    );
    let limitations = usage_evidence_limitations(coverage, events);
    for (index, (point, score, deviation, threshold)) in anomalies.iter().enumerate() {
        let score_value = score
            .map(decimal)
            .unwrap_or_else(|| "unavailable".to_string());
        report.cards.push(InsightCard {
            id: format!("anomaly.output-tokens.{}.v1", point.date),
            version: "1".to_string(),
            family: "anomaly".to_string(),
            class: "factual".to_string(),
            title: format!("Unusual observed output · {}", point.date),
            finding: format!(
                "{} output tokens were unusual within observed activity under the declared robust baseline.",
                point.value
            ),
            metric_id: "anomaly.daily-output-tokens".to_string(),
            comparison: None,
            window: date_window(
                time_context,
                point.date,
                point.date.succ_opt().unwrap(),
            ),
            sample_count: points.len(),
            minimum_sample_count: MINIMUM,
            method_id: ANOMALY_METHOD.to_string(),
            availability: if limitations.is_empty() {
                "available"
            } else {
                "partial"
            }
            .to_string(),
            coverage: evidence_coverage.clone(),
            confidence: evidence_confidence(&evidence_coverage, points.len(), MINIMUM),
            supporting_facts: vec![
                exact_fact(
                    &format!("insight.fact.anomaly.{}.value", point.date),
                    "tokens.output.daily",
                    point.value.to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    date_window(
                        time_context,
                        point.date,
                        point.date.succ_opt().unwrap(),
                    ),
                    point.sample_count,
                    evidence_coverage.clone(),
                    "canonical",
                ),
                exact_fact(
                    &format!("insight.fact.anomaly.{}.median", point.date),
                    "anomaly.baseline-median",
                    median_value.to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    window.clone(),
                    points.len(),
                    evidence_coverage.clone(),
                    "derived",
                ),
                exact_fact(
                    &format!("insight.fact.anomaly.{}.mad", point.date),
                    "anomaly.baseline-mad",
                    mad.to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    window.clone(),
                    points.len(),
                    evidence_coverage.clone(),
                    "derived",
                ),
                exact_fact(
                    &format!("insight.fact.anomaly.{}.robust-score", point.date),
                    "anomaly.robust-score",
                    score_value,
                    "score",
                    ANOMALY_METHOD,
                    window.clone(),
                    points.len(),
                    evidence_coverage.clone(),
                    "derived",
                ),
                exact_fact(
                    &format!("insight.fact.anomaly.{}.absolute-deviation", point.date),
                    "anomaly.absolute-deviation",
                    deviation.to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    window.clone(),
                    points.len(),
                    evidence_coverage.clone(),
                    "derived",
                ),
                exact_fact(
                    &format!("insight.fact.anomaly.{}.practical-threshold", point.date),
                    "anomaly.practical-threshold",
                    threshold.to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    window.clone(),
                    points.len(),
                    evidence_coverage.clone(),
                    "method-parameter",
                ),
            ],
            limitations: if mad == 0 {
                let mut values = limitations.clone();
                values.push("anomaly-mad-zero-fallback".to_string());
                values
            } else {
                limitations.clone()
            },
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 170u32.saturating_add(index as u32),
        });
    }
    set_family_owned(
        report,
        "anomaly",
        if anomalies.is_empty() || limitations.is_empty() {
            "available"
        } else {
            "partial"
        },
        points.len(),
        MINIMUM,
        if anomalies.is_empty() {
            vec!["anomaly-no-point-crossed-threshold".to_string()]
        } else {
            limitations
        },
    );
}

fn build_recommendations(
    events: &[NormalizedEvent],
    reliability: &ReliabilitySummary,
    tools: &[ToolSummary],
    routing: &RoutingSummary,
    time_context: &TimeContext,
    report: &mut InsightReport,
) {
    let observed_window = event_window(events, time_context);
    let window = period_window(time_context, &observed_window);
    let mut emitted = 0usize;
    if reliability.terminal_outcomes >= 10
        && reliability
            .terminal_error_rate_pct
            .is_some_and(|rate| rate >= 10.0)
    {
        let rate = reliability.terminal_error_rate_pct.unwrap();
        report.cards.push(recommendation_card(
            "recommendation.api-terminal-errors.v1",
            "Review terminal API errors with a controlled rerun",
            format!(
                "{} of {} direct terminal outcomes were errors ({}%).",
                reliability.terminal_errors,
                reliability.terminal_outcomes,
                decimal(rate)
            ),
            "recommendation.api-terminal-errors",
            reliability.terminal_outcomes,
            10,
            window.clone(),
            "complete-direct-otel",
            vec![
                exact_fact(
                    "insight.fact.recommendation.api-terminal-errors.trigger-card",
                    "reference.card",
                    "reliability.api-terminal-error-rate.v1".to_string(),
                    "card-id",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    reliability.terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "reference",
                ),
                exact_fact(
                    "insight.fact.recommendation.api-terminal-errors.numerator",
                    "api.terminal-errors",
                    reliability.terminal_errors.to_string(),
                    "errors",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    reliability.terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.recommendation.api-terminal-errors.denominator",
                    "api.terminal-outcomes",
                    reliability.terminal_outcomes.to_string(),
                    "outcomes",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    reliability.terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.recommendation.api-terminal-errors.rate",
                    "reliability.api-terminal-error-rate",
                    decimal(rate),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    reliability.terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
                exact_fact(
                    "insight.fact.recommendation.api-terminal-errors.threshold",
                    "recommendation.threshold",
                    "10".to_string(),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    reliability.terminal_outcomes,
                    "complete-direct-otel".to_string(),
                    "method-parameter",
                ),
            ],
            "Repeat a controlled 10-request sample after checking local configuration and connectivity, then compare the same terminal-outcome rate.",
            vec![
                "A transient service or network condition may explain the observed errors.",
                "The task or input mix may differ between the observed and controlled samples.",
            ],
            200,
        ));
        emitted = emitted.saturating_add(1);
    }
    let mut eligible_tools = tools
        .iter()
        .filter(|tool| {
            tool.recommendation_eligible
                && tool.results >= 10
                && tool.failure_rate_pct.is_some_and(|rate| rate >= 20.0)
        })
        .collect::<Vec<_>>();
    eligible_tools.sort_by(|left, right| {
        right
            .failure_rate_pct
            .unwrap()
            .total_cmp(&left.failure_rate_pct.unwrap())
            .then_with(|| right.failures.cmp(&left.failures))
            .then_with(|| right.results.cmp(&left.results))
            .then_with(|| left.name.cmp(&right.name))
    });
    if let Some(tool) = eligible_tools.first() {
        let rate = tool.failure_rate_pct.unwrap();
        report.cards.push(recommendation_card(
            "recommendation.tool-result-errors.v1",
            "Test the highest observed tool-result error rate",
            format!(
                "{} of {} direct {} results were errors ({}%).",
                tool.failures,
                tool.results,
                tool.name,
                decimal(rate)
            ),
            "recommendation.tool-result-errors",
            tool.results,
            10,
            window.clone(),
            "complete-direct-otel",
            vec![
                exact_fact(
                    "insight.fact.recommendation.tool-result-errors.trigger-card",
                    "reference.card",
                    tool.card_id.clone(),
                    "card-id",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    tool.results,
                    "complete-direct-otel".to_string(),
                    "reference",
                ),
                exact_fact(
                    "insight.fact.recommendation.tool-result-errors.numerator",
                    "tool.direct-failures",
                    tool.failures.to_string(),
                    "errors",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    tool.results,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.recommendation.tool-result-errors.denominator",
                    "tool.direct-results",
                    tool.results.to_string(),
                    "results",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    tool.results,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "insight.fact.recommendation.tool-result-errors.rate",
                    "tool.direct-failure-rate",
                    decimal(rate),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    tool.results,
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
                exact_fact(
                    "insight.fact.recommendation.tool-result-errors.threshold",
                    "recommendation.threshold",
                    "20".to_string(),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    tool.results,
                    "complete-direct-otel".to_string(),
                    "method-parameter",
                ),
            ],
            "Repeat a small controlled workflow for the classified tool after checking inputs and permissions, then compare the same direct-result rate.",
            vec![
                "The observed task mix or invalid inputs may explain the result rate.",
                "Permissions, tool versions, or environment changes may explain the result rate.",
            ],
            201,
        ));
        emitted = emitted.saturating_add(1);
    }
    if routing.recommendation_eligible
        && routing.observations >= 20
        && routing
            .top_request_share_pct
            .is_some_and(|share| share >= 80.0)
        && routing
            .unknown_request_share_pct
            .is_some_and(|share| share <= 10.0)
    {
        let share = routing.top_request_share_pct.unwrap();
        let model = routing.top_model.as_deref().unwrap_or("mapped model");
        report.cards.push(recommendation_card(
            "recommendation.model-routing-experiment.v1",
            "Run a bounded model-routing experiment",
            format!(
                "{model} represented {}% of {} canonical request/message observations.",
                decimal(share),
                routing.observations
            ),
            "recommendation.model-routing-experiment",
            routing.observations,
            20,
            window.clone(),
            &routing.trigger_coverage,
            vec![
                exact_fact(
                    "insight.fact.recommendation.model-routing.trigger-card",
                    "reference.card",
                    "routing.model-request-share.v1".to_string(),
                    "card-id",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    routing.observations,
                    routing.trigger_coverage.clone(),
                    "reference",
                ),
                exact_fact(
                    "insight.fact.recommendation.model-routing.numerator",
                    "routing.top-model-requests",
                    routing.top_requests.to_string(),
                    "observations",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    routing.observations,
                    routing.trigger_coverage.clone(),
                    "canonical",
                ),
                exact_fact(
                    "insight.fact.recommendation.model-routing.denominator",
                    "routing.total-model-observations",
                    routing.observations.to_string(),
                    "observations",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    routing.observations,
                    routing.trigger_coverage.clone(),
                    "canonical",
                ),
                exact_fact(
                    "insight.fact.recommendation.model-routing.top-share",
                    "routing.top-model-request-share",
                    decimal(share),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    routing.observations,
                    routing.trigger_coverage.clone(),
                    "derived",
                ),
                exact_fact(
                    "insight.fact.recommendation.model-routing.top-share-threshold",
                    "recommendation.top-share-threshold",
                    "80".to_string(),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    routing.observations,
                    routing.trigger_coverage.clone(),
                    "method-parameter",
                ),
                exact_fact(
                    "insight.fact.recommendation.model-routing.unknown-share",
                    "routing.unknown-model-request-share",
                    decimal(routing.unknown_request_share_pct.unwrap_or(0.0)),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    routing.observations,
                    routing.trigger_coverage.clone(),
                    "derived",
                ),
                exact_fact(
                    "insight.fact.recommendation.model-routing.unknown-share-threshold",
                    "recommendation.unknown-share-maximum",
                    "10".to_string(),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    routing.observations,
                    routing.trigger_coverage.clone(),
                    "method-parameter",
                ),
            ],
            "Review 10 already-known interchangeable tasks, try your chosen alternative model on that bounded set, and compare task-defined outcomes plus canonical request and local-cost evidence.",
            vec![
                "Task complexity or a deliberate user policy may explain the observed concentration.",
                "Missing model evidence or deliberate specialization may explain the observed concentration.",
            ],
            202,
        ));
        emitted = emitted.saturating_add(1);
    }
    let reliability_rule_eligible =
        reliability.terminal_outcomes >= 10 && reliability.terminal_error_rate_pct.is_some();
    let tool_rule_eligible = tools.iter().any(|tool| {
        tool.recommendation_eligible && tool.results >= 10 && tool.failure_rate_pct.is_some()
    });
    let routing_rule_eligible = routing.recommendation_eligible
        && routing.observations >= 20
        && routing.top_request_share_pct.is_some()
        && routing.unknown_request_share_pct.is_some();
    let exact_trigger_evidence =
        reliability_rule_eligible || tool_rule_eligible || routing_rule_eligible;
    set_family_owned(
        report,
        "recommendation",
        if exact_trigger_evidence {
            "available"
        } else {
            "unavailable"
        },
        reliability
            .terminal_outcomes
            .max(routing.observations)
            .max(tools.iter().map(|tool| tool.results).max().unwrap_or(0)),
        10,
        if !exact_trigger_evidence {
            vec!["recommendation-exact-trigger-evidence-unavailable".to_string()]
        } else if emitted == 0 {
            vec!["recommendation-no-rule-threshold-met".to_string()]
        } else {
            Vec::new()
        },
    );
}

#[allow(clippy::too_many_arguments)]
fn recommendation_card(
    id: &str,
    title: &str,
    finding: String,
    metric_id: &str,
    sample_count: usize,
    minimum_sample_count: usize,
    window: InsightWindow,
    coverage: &str,
    supporting_facts: Vec<InsightFact>,
    experiment: &str,
    alternatives: Vec<&str>,
    renderer_priority: u32,
) -> InsightCard {
    InsightCard {
        id: id.to_string(),
        version: "1".to_string(),
        family: "recommendation".to_string(),
        class: "recommendation".to_string(),
        title: title.to_string(),
        finding,
        metric_id: metric_id.to_string(),
        comparison: None,
        window,
        sample_count,
        minimum_sample_count,
        method_id: RECOMMENDATION_METHOD.to_string(),
        availability: "available".to_string(),
        coverage: coverage.to_string(),
        confidence: sample_confidence(sample_count, minimum_sample_count),
        supporting_facts,
        limitations: Vec::new(),
        action: Some(InsightAction {
            experiment: experiment.to_string(),
            alternative_explanations: alternatives.into_iter().map(str::to_string).collect(),
        }),
        privacy_class: "standard".to_string(),
        renderer_priority,
    }
}

fn build_entertainment(
    events: &[NormalizedEvent],
    metrics: &CanonicalMetrics,
    coverage: &DataCoverage,
    time_context: &TimeContext,
    report: &mut InsightReport,
) {
    const MINIMUM_OBSERVATIONS: usize = 20;
    const MINIMUM_ACTIVE_DAYS: usize = 5;
    let observations = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::AssistantUsage | EventKind::OtelApiRequest
            )
        })
        .count();
    let active_days = metrics
        .tokens
        .days
        .iter()
        .filter(|day| day.tokens.output.sample_count > 0)
        .count();
    if observations < MINIMUM_OBSERVATIONS || active_days < MINIMUM_ACTIVE_DAYS {
        set_family_owned(
            report,
            "entertainment",
            "unavailable",
            observations,
            MINIMUM_OBSERVATIONS,
            vec!["entertainment-not-enough-observed-activity".to_string()],
        );
        return;
    }
    let subagent_count = events
        .iter()
        .filter(|event| {
            event.is_subagent
                && matches!(
                    event.kind,
                    EventKind::AssistantUsage | EventKind::OtelApiRequest
                )
        })
        .count();
    let tool_bearing_observations = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::AssistantUsage | EventKind::OtelApiRequest
            ) && !event.tool_names.is_empty()
        })
        .count();
    let concentration_hhi = report
        .cards
        .iter()
        .find(|card| card.id == "concentration.project-output-hhi.v1")
        .and_then(|card| {
            card.supporting_facts
                .iter()
                .find(|fact| fact.metric_id == "concentration.project-output-hhi")
        })
        .and_then(|fact| fact.value.parse::<f64>().ok())
        .unwrap_or(0.0);
    let label = if subagent_count.saturating_mul(100) >= observations.saturating_mul(30) {
        "The Orchestrator"
    } else if tool_bearing_observations >= observations {
        "The Toolsmith"
    } else if concentration_hhi >= 2_500.0 {
        "The Specialist"
    } else {
        "The Explorer"
    };
    let observed_window = event_window(events, time_context);
    let window = period_window(time_context, &observed_window);
    report.cards.push(InsightCard {
        id: "entertainment.archetype.v1".to_string(),
        version: "1".to_string(),
        family: "entertainment".to_string(),
        class: "entertainment".to_string(),
        title: format!("Entertainment · {label}"),
        finding: "A deterministic playful label based on sample-gated aggregate activity; it is not a factual assessment.".to_string(),
        metric_id: "entertainment.archetype".to_string(),
        comparison: None,
        window: window.clone(),
        sample_count: observations,
        minimum_sample_count: MINIMUM_OBSERVATIONS,
        method_id: ENTERTAINMENT_METHOD.to_string(),
        availability: "available".to_string(),
        coverage: coverage.completeness.clone(),
        confidence: "unavailable".to_string(),
        supporting_facts: vec![
            exact_fact(
                "insight.fact.entertainment.observations",
                "request.canonical-count",
                observations.to_string(),
                "observations",
                ENTERTAINMENT_METHOD,
                window.clone(),
                observations,
                coverage.completeness.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.entertainment.active-days",
                "activity.observed-active-days",
                active_days.to_string(),
                "days",
                ENTERTAINMENT_METHOD,
                window.clone(),
                active_days,
                coverage.completeness.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.entertainment.subagent-observations",
                "entertainment.subagent-observations",
                subagent_count.to_string(),
                "observations",
                ENTERTAINMENT_METHOD,
                window.clone(),
                observations,
                coverage.completeness.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.entertainment.tool-bearing-observations",
                "entertainment.tool-bearing-observations",
                tool_bearing_observations.to_string(),
                "observations",
                ENTERTAINMENT_METHOD,
                window.clone(),
                observations,
                coverage.completeness.clone(),
                "canonical",
            ),
            exact_fact(
                "insight.fact.entertainment.project-hhi",
                "concentration.project-output-hhi",
                decimal(concentration_hhi),
                "hhi-0-10000",
                ENTERTAINMENT_METHOD,
                window.clone(),
                observations,
                coverage.completeness.clone(),
                "derived",
            ),
        ],
        limitations: vec!["entertainment-not-a-factual-assessment".to_string()],
        action: None,
        privacy_class: "share".to_string(),
        renderer_priority: 300,
    });
    let mut family_limitations = vec!["entertainment-not-a-factual-assessment".to_string()];
    if metrics.cache.read_share.availability == "available" {
        if let Some(read_share) = metrics
            .cache
            .read_share
            .value_pct
            .filter(|value| value.is_finite())
        {
            report.cards.push(InsightCard {
                id: "entertainment.cache-mood.v1".to_string(),
                version: "1".to_string(),
                family: "entertainment".to_string(),
                class: "entertainment".to_string(),
                title: "Entertainment · Cache cartographer".to_string(),
                finding: "A playful label based only on the available canonical cache-read share; it does not infer a cause or monetary effect.".to_string(),
                metric_id: "entertainment.cache-mood".to_string(),
                comparison: None,
                window: window.clone(),
                sample_count: metrics.cache.read_share.sample_count,
                minimum_sample_count: 1,
                method_id: ENTERTAINMENT_METHOD.to_string(),
                availability: "available".to_string(),
                coverage: coverage.completeness.clone(),
                confidence: "unavailable".to_string(),
                supporting_facts: vec![exact_fact(
                    "insight.fact.entertainment.cache-read-share",
                    "cache.read-share",
                    decimal(read_share),
                    "percent",
                    ENTERTAINMENT_METHOD,
                    window.clone(),
                    metrics.cache.read_share.sample_count,
                    metrics.cache.read_share.availability.clone(),
                    "canonical",
                )],
                limitations: vec!["entertainment-not-a-factual-assessment".to_string()],
                action: None,
                privacy_class: "share".to_string(),
                renderer_priority: 301,
            });
        } else {
            family_limitations.push("entertainment-cache-label-unavailable".to_string());
        }
    } else {
        family_limitations.push("entertainment-cache-label-unavailable".to_string());
    }
    let trend_reference = report
        .cards
        .iter()
        .find(|card| card.id == "trend.output-tokens.v1" && card.availability == "available")
        .map(|card| (card.id.clone(), card.sample_count));
    if let Some((trend_id, trend_samples)) = trend_reference {
        report.cards.push(InsightCard {
            id: "entertainment.momentum.v1".to_string(),
            version: "1".to_string(),
            family: "entertainment".to_string(),
            class: "entertainment".to_string(),
            title: "Entertainment · Observed momentum".to_string(),
            finding: "A playful presentation of the available declared trend card; it is not a factual assessment.".to_string(),
            metric_id: "entertainment.momentum".to_string(),
            comparison: None,
            window: window.clone(),
            sample_count: trend_samples,
            minimum_sample_count: TREND_MINIMUM_POINTS,
            method_id: ENTERTAINMENT_METHOD.to_string(),
            availability: "available".to_string(),
            coverage: coverage.completeness.clone(),
            confidence: "unavailable".to_string(),
            supporting_facts: vec![exact_fact(
                "insight.fact.entertainment.trend-reference",
                "reference.card",
                trend_id,
                "card-id",
                ENTERTAINMENT_METHOD,
                window,
                trend_samples,
                coverage.completeness.clone(),
                "reference",
            )],
            limitations: vec!["entertainment-not-a-factual-assessment".to_string()],
            action: None,
            privacy_class: "share".to_string(),
            renderer_priority: 302,
        });
    } else {
        family_limitations.push("entertainment-momentum-label-unavailable".to_string());
    }
    set_family_owned(
        report,
        "entertainment",
        "available",
        observations,
        MINIMUM_OBSERVATIONS,
        family_limitations,
    );
}

#[derive(Debug)]
struct WindowTotal {
    value: u128,
    sample_count: usize,
    active_days: usize,
    exact: bool,
}

fn daily_window(days: &[NamedTokenMetricSet], start: NaiveDate, end: NaiveDate) -> WindowTotal {
    days.iter()
        .filter_map(|day| {
            let date = NaiveDate::parse_from_str(&day.key, "%Y-%m-%d").ok()?;
            (date >= start && date < end).then_some(day)
        })
        .fold(
            WindowTotal {
                value: 0,
                sample_count: 0,
                active_days: 0,
                exact: true,
            },
            |mut total, day| {
                let output = &day.tokens.output;
                total.value = total.value.saturating_add(u128::from(output.observed));
                total.sample_count = total.sample_count.saturating_add(output.sample_count);
                total.exact &= output.availability == "available" && !output.overflowed;
                total
            },
        )
}

fn active_usage_days(
    events: &[NormalizedEvent],
    time_context: &TimeContext,
    start: NaiveDate,
    end: NaiveDate,
) -> usize {
    events
        .iter()
        .filter(|event| match event.kind {
            EventKind::AssistantUsage | EventKind::OtelApiRequest => event.tokens.richness() > 0,
            EventKind::OtelMetric => [
                event.tokens.input,
                event.tokens.output,
                event.tokens.cache_creation,
                event.tokens.cache_read,
            ]
            .into_iter()
            .flatten()
            .any(|value| value > 0),
            _ => false,
        })
        .filter_map(|event| time_context.local_date_epoch(event.epoch_nanos))
        .filter(|date| *date >= start && *date < end)
        .collect::<BTreeSet<_>>()
        .len()
}

#[derive(Debug)]
struct DailyPoint {
    date: NaiveDate,
    value: u128,
    sample_count: usize,
    exact: bool,
}

fn exact_daily_points(days: &[NamedTokenMetricSet]) -> Vec<DailyPoint> {
    let mut points = days
        .iter()
        .filter_map(|day| {
            Some(DailyPoint {
                date: NaiveDate::parse_from_str(&day.key, "%Y-%m-%d").ok()?,
                value: u128::from(day.tokens.output.observed),
                sample_count: day.tokens.output.sample_count,
                exact: day.tokens.output.availability == "available"
                    && !day.tokens.output.overflowed,
            })
        })
        .collect::<Vec<_>>();
    points.sort_by_key(|point| point.date);
    points
}

fn median(points: &[DailyPoint]) -> u128 {
    let mut values = points.iter().map(|point| point.value).collect::<Vec<_>>();
    values.sort_unstable();
    let middle = values.len() / 2;
    if values.len() & 1 == 0 {
        values[middle - 1]
            .saturating_add(values[middle])
            .checked_div(2)
            .unwrap_or(0)
    } else {
        values[middle]
    }
}

fn insight_window(
    time_context: &TimeContext,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<InsightWindow, super::IngestionError> {
    let start = time_context
        .local_date_start_epoch(start)
        .map_err(super::IngestionError::time)?;
    let end = time_context
        .local_date_start_epoch(end)
        .map_err(super::IngestionError::time)?;
    Ok(InsightWindow {
        start: epoch_datetime(start)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
            .unwrap_or_default(),
        end: epoch_datetime(end)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
            .unwrap_or_default(),
        timezone: time_context.name().to_string(),
    })
}

fn date_window(time_context: &TimeContext, start: NaiveDate, end: NaiveDate) -> InsightWindow {
    insight_window(time_context, start, end).unwrap_or_else(|_| empty_window(time_context))
}

fn empty_window(time_context: &TimeContext) -> InsightWindow {
    InsightWindow {
        timezone: time_context.name().to_string(),
        ..InsightWindow::default()
    }
}

fn event_window(events: &[NormalizedEvent], time_context: &TimeContext) -> InsightWindow {
    let start = events
        .iter()
        .filter_map(|event| time_context.local_date_epoch(event.epoch_nanos))
        .min();
    let end = events
        .iter()
        .filter_map(|event| time_context.local_date_epoch(event.epoch_nanos))
        .max()
        .and_then(|date| date.succ_opt());
    match (start, end) {
        (Some(start), Some(end)) => date_window(time_context, start, end),
        _ => empty_window(time_context),
    }
}

fn daily_points_window(days: &[NamedTokenMetricSet], time_context: &TimeContext) -> InsightWindow {
    let dates = days
        .iter()
        .filter_map(|day| NaiveDate::parse_from_str(&day.key, "%Y-%m-%d").ok())
        .collect::<Vec<_>>();
    let start = dates.iter().min().copied();
    let end = dates.iter().max().and_then(|date| date.succ_opt());
    match (start, end) {
        (Some(start), Some(end)) => date_window(time_context, start, end),
        _ => empty_window(time_context),
    }
}

fn period_window(time_context: &TimeContext, observed_window: &InsightWindow) -> InsightWindow {
    let Some((start, end)) = time_context.period_bounds() else {
        return observed_window.clone();
    };
    InsightWindow {
        start: epoch_datetime(start)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
            .unwrap_or_default(),
        end: epoch_datetime(end)
            .map(|value| value.to_rfc3339_opts(SecondsFormat::Secs, true))
            .unwrap_or_default(),
        timezone: time_context.name().to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn exact_fact(
    id: &str,
    metric_id: &str,
    value: String,
    unit: &str,
    method_id: &str,
    window: InsightWindow,
    sample_count: usize,
    coverage: String,
    source: &str,
) -> InsightFact {
    InsightFact {
        id: id.to_string(),
        metric_id: metric_id.to_string(),
        value,
        unit: unit.to_string(),
        method_id: method_id.to_string(),
        window,
        sample_count,
        coverage,
        source: source.to_string(),
    }
}

fn unavailable_family(family: &str, capabilities: &[&str]) -> InsightFamilyStatus {
    let minimum_sample_count = family_minimum_sample_count(family);
    InsightFamilyStatus {
        family: family.to_string(),
        availability: "unavailable".to_string(),
        required_capabilities: capabilities
            .iter()
            .map(|capability| (*capability).to_string())
            .collect(),
        sample_count: 0,
        minimum_sample_count,
        limitations: vec![if family == "active-efficiency" {
            "efficiency-minimum-active-seconds".to_string()
        } else {
            format!("{family}-evidence-unavailable")
        }],
    }
}

fn family_minimum_sample_count(family: &str) -> usize {
    match family {
        "comparison" => COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
        "trend" => TREND_MINIMUM_POINTS,
        "active-efficiency" => EFFICIENCY_MINIMUM_REQUESTS,
        "reliability" => 10,
        "tool-behavior" => 5,
        "model-routing" => 5,
        "project-concentration" => 1,
        "anomaly" => 7,
        "recommendation" => 10,
        "entertainment" => 20,
        _ => 1,
    }
}

fn set_family(
    report: &mut InsightReport,
    family: &str,
    availability: &str,
    sample_count: usize,
    minimum_sample_count: usize,
    limitations: Vec<&str>,
) {
    if let Some(status) = report
        .families
        .iter_mut()
        .find(|status| status.family == family)
    {
        status.availability = availability.to_string();
        status.sample_count = sample_count;
        status.minimum_sample_count = minimum_sample_count;
        status.limitations = limitations.into_iter().map(str::to_string).collect();
    }
}

fn set_family_owned(
    report: &mut InsightReport,
    family: &str,
    availability: &str,
    sample_count: usize,
    minimum_sample_count: usize,
    limitations: Vec<String>,
) {
    if let Some(status) = report
        .families
        .iter_mut()
        .find(|status| status.family == family)
    {
        status.availability = availability.to_string();
        status.sample_count = sample_count;
        status.minimum_sample_count = minimum_sample_count;
        status.limitations = limitations;
    }
}

fn observed_limitations(coverage: &DataCoverage) -> Vec<String> {
    match coverage.completeness.as_str() {
        "complete" => Vec::new(),
        "indeterminate" => vec!["retention-indeterminate-observed-activity".to_string()],
        "partial" => vec!["partial-observed-activity".to_string()],
        "empty" => vec!["empty-observed-activity".to_string()],
        _ => vec!["unknown-coverage".to_string()],
    }
}

fn usage_evidence_coverage(coverage: &DataCoverage, events: &[NormalizedEvent]) -> String {
    let contributing_sources = events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                EventKind::AssistantUsage | EventKind::OtelApiRequest
            ) || (event.kind == EventKind::OtelMetric && event.tokens.richness() > 0)
        })
        .map(|event| event.source_alias.as_str())
        .collect::<BTreeSet<_>>();
    let contributing_coverage = coverage
        .sources
        .iter()
        .filter(|source| contributing_sources.contains(source.alias.as_str()))
        .collect::<Vec<_>>();
    let has_transcript = contributing_coverage
        .iter()
        .any(|source| source.kind == "transcript");
    let contributing_source_is_partial = contributing_coverage
        .iter()
        .any(|source| source.completeness == "partial");
    let unrelated_source_is_partial = coverage.sources.iter().any(|source| {
        !contributing_sources.contains(source.alias.as_str()) && source.completeness == "partial"
    });
    let global_usage_status = coverage
        .capabilities
        .get("analysis_usage_totals")
        .map(String::as_str);
    match global_usage_status {
        Some("available") if has_transcript => "indeterminate-retained-history",
        Some("available") => "complete-canonical-usage",
        Some("partial") if contributing_source_is_partial || !unrelated_source_is_partial => {
            "partial-canonical-usage"
        }
        Some("partial") if has_transcript => "indeterminate-retained-history",
        Some("partial") => "complete-canonical-usage",
        _ => "unavailable-canonical-usage",
    }
    .to_string()
}

fn usage_evidence_limitations(coverage: &DataCoverage, events: &[NormalizedEvent]) -> Vec<String> {
    match usage_evidence_coverage(coverage, events).as_str() {
        "complete-canonical-usage" => Vec::new(),
        "indeterminate-retained-history" => {
            vec!["retention-indeterminate-observed-activity".to_string()]
        }
        "partial-canonical-usage" => vec!["partial-canonical-usage-evidence".to_string()],
        _ => vec!["canonical-usage-evidence-unavailable".to_string()],
    }
}

fn routing_request_evidence_coverage(
    coverage: &DataCoverage,
    events: &[&NormalizedEvent],
) -> String {
    let mut has_transcript = false;
    let mut partial = false;
    for event in events {
        if event.attribute_evidence_uncertain {
            partial = true;
        }
        match event.kind {
            EventKind::AssistantUsage => has_transcript = true,
            EventKind::OtelApiRequest => {
                let source = coverage
                    .sources
                    .iter()
                    .find(|source| source.alias == event.source_alias);
                let status = source
                    .and_then(|source| source.capabilities.get("api_request"))
                    .map(String::as_str);
                if status != Some("available")
                    || source.is_some_and(|source| source.completeness == "partial")
                {
                    partial = true;
                }
            }
            _ => partial = true,
        }
    }
    if partial {
        "partial-canonical-usage"
    } else if has_transcript {
        "indeterminate-retained-history"
    } else if events.is_empty() {
        "unavailable-canonical-usage"
    } else {
        "complete-canonical-usage"
    }
    .to_string()
}

fn routing_request_evidence_limitations(coverage: &str) -> Vec<String> {
    match coverage {
        "complete-canonical-usage" => Vec::new(),
        "indeterminate-retained-history" => {
            vec!["retention-indeterminate-observed-activity".to_string()]
        }
        "partial-canonical-usage" => vec!["partial-canonical-model-evidence".to_string()],
        _ => vec!["canonical-model-evidence-unavailable".to_string()],
    }
}

fn evidence_confidence(coverage: &str, sample_count: usize, minimum: usize) -> String {
    match coverage {
        "complete-canonical-usage" => sample_confidence(sample_count, minimum),
        "indeterminate-retained-history" if sample_count >= minimum => "low".to_string(),
        _ => "unavailable".to_string(),
    }
}

fn capability_available(coverage: &DataCoverage, capability: &str) -> bool {
    coverage.capabilities.get(capability).map(String::as_str) == Some("available")
}

fn confidence(coverage: &DataCoverage, sample_count: usize, minimum: usize) -> String {
    match coverage.completeness.as_str() {
        "complete" if sample_count >= minimum.saturating_mul(2) => "high",
        "complete" if sample_count >= minimum => "medium",
        "indeterminate" if sample_count >= minimum => "low",
        _ => "unavailable",
    }
    .to_string()
}

fn sample_confidence(sample_count: usize, minimum: usize) -> String {
    if sample_count >= minimum.saturating_mul(2) {
        "high"
    } else if sample_count >= minimum {
        "medium"
    } else {
        "unavailable"
    }
    .to_string()
}

fn signed_delta(current: u128, baseline: u128) -> String {
    if current >= baseline {
        current.saturating_sub(baseline).to_string()
    } else {
        format!("-{}", baseline.saturating_sub(current))
    }
}

fn absolute_difference(left: u128, right: u128) -> u128 {
    left.max(right).saturating_sub(left.min(right))
}

fn median_values(values: &[u128]) -> u128 {
    let middle = values.len() / 2;
    if values.len() & 1 == 0 {
        values[middle - 1]
            .saturating_add(values[middle])
            .checked_div(2)
            .unwrap_or(0)
    } else {
        values[middle]
    }
}

fn median_f64(values: &[f64]) -> f64 {
    let middle = values.len() / 2;
    if values.len() & 1 == 0 {
        round6((values[middle - 1] + values[middle]) / 2.0)
    } else {
        round6(values[middle])
    }
}

fn nearest_rank(values: &[f64], percentile: usize) -> f64 {
    let rank = values
        .len()
        .saturating_mul(percentile)
        .saturating_add(99)
        .checked_div(100)
        .unwrap_or(values.len())
        .max(1)
        .saturating_sub(1)
        .min(values.len().saturating_sub(1));
    round6(values[rank])
}

fn percent(numerator: u128, denominator: u128) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        round6(numerator as f64 / denominator as f64 * 100.0)
    }
}

fn div_ceil(numerator: u128, denominator: u128) -> u128 {
    numerator
        .checked_add(denominator.saturating_sub(1))
        .and_then(|value| value.checked_div(denominator))
        .unwrap_or(u128::MAX)
}

fn rate_per_hour(numerator: u64, active_seconds: u64) -> f64 {
    round6(numerator as f64 * 3600.0 / active_seconds as f64)
}

fn round6(value: f64) -> f64 {
    let scaled = value * 1_000_000.0;
    if !value.is_finite() || !scaled.is_finite() {
        return 0.0;
    }
    scaled.round() / 1_000_000.0
}

fn decimal(value: f64) -> String {
    let rendered = format!("{value:.6}");
    rendered
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

fn family_rank(family: &str) -> usize {
    FAMILIES
        .iter()
        .position(|(candidate, _)| *candidate == family)
        .unwrap_or(usize::MAX)
}

fn unique_strings(values: &[String]) -> bool {
    values.iter().collect::<BTreeSet<_>>().len() == values.len()
}

fn valid_window(window: &InsightWindow) -> bool {
    let (Ok(start), Ok(end)) = (
        chrono::DateTime::parse_from_rfc3339(&window.start),
        chrono::DateTime::parse_from_rfc3339(&window.end),
    ) else {
        return false;
    };
    !window.timezone.is_empty() && start < end
}

fn window_contains(outer: &InsightWindow, inner: &InsightWindow) -> bool {
    let (Ok(outer_start), Ok(outer_end), Ok(inner_start), Ok(inner_end)) = (
        chrono::DateTime::parse_from_rfc3339(&outer.start),
        chrono::DateTime::parse_from_rfc3339(&outer.end),
        chrono::DateTime::parse_from_rfc3339(&inner.start),
        chrono::DateTime::parse_from_rfc3339(&inner.end),
    ) else {
        return false;
    };
    outer.timezone == inner.timezone && outer_start <= inner_start && outer_end >= inner_end
}

fn single_fact<'a>(card: &'a InsightCard, metric_id: &str) -> Option<&'a InsightFact> {
    let mut facts = card
        .supporting_facts
        .iter()
        .filter(|fact| fact.metric_id == metric_id);
    let fact = facts.next()?;
    facts.next().is_none().then_some(fact)
}

fn numeric_fact(card: &InsightCard, metric_id: &str) -> Option<f64> {
    single_fact(card, metric_id)?
        .value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
}

fn count_fact(card: &InsightCard, metric_id: &str) -> Option<u128> {
    single_fact(card, metric_id)?.value.parse::<u128>().ok()
}

fn approximately_equal(left: f64, right: f64) -> bool {
    left.is_finite() && right.is_finite() && (left - right).abs() <= 0.000_001
}

fn valid_numeric_fact(fact: &InsightFact) -> bool {
    if matches!(
        fact.unit.as_str(),
        "alias" | "card-id" | "coverage-signature"
    ) {
        return !fact.value.is_empty();
    }
    if fact.unit == "direction" {
        return matches!(fact.value.as_str(), "rose" | "fell" | "stable");
    }
    if fact.unit == "local-date" {
        return NaiveDate::parse_from_str(&fact.value, "%Y-%m-%d").is_ok();
    }
    if fact.unit == "score" && fact.value == "unavailable" {
        return true;
    }
    if fact.value.contains(['e', 'E']) {
        return false;
    }
    let Some(value) = fact
        .value
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
    else {
        return false;
    };
    if fact
        .value
        .split_once('.')
        .is_some_and(|(_, fraction)| fraction.len() > 6)
    {
        return false;
    }
    match fact.unit.as_str() {
        "percent" => (0.0..=100.0).contains(&value),
        "hhi-0-10000" => (0.0..=10_000.0).contains(&value),
        "milliseconds" => (0.0..=super::types::MAX_DIRECT_DURATION_MS).contains(&value),
        _ => value >= 0.0,
    }
}

fn all_facts_use_card_window(card: &InsightCard) -> bool {
    card.supporting_facts
        .iter()
        .all(|fact| same_window(&card.window, &fact.window))
}

fn exact_local_date_bounds(window: &InsightWindow) -> Option<(NaiveDate, NaiveDate)> {
    let timezone = window.timezone.parse::<chrono_tz::Tz>().ok()?;
    let start = chrono::DateTime::parse_from_rfc3339(&window.start)
        .ok()?
        .with_timezone(&timezone)
        .date_naive();
    let end = chrono::DateTime::parse_from_rfc3339(&window.end)
        .ok()?
        .with_timezone(&timezone)
        .date_naive();
    let last = end.pred_opt()?;
    let context = TimeContext::new(&window.timezone, None).ok()?;
    same_window(window, &date_window(&context, start, end)).then_some((start, last))
}

fn trend_proof_valid(
    card: &InsightCard,
    baseline: &InsightFact,
    current: &InsightFact,
    baseline_value: u128,
    current_value: u128,
) -> bool {
    let (
        Some(direction),
        Some(first_date),
        Some(half_size),
        Some(last_date),
        Some(point_count),
        Some(threshold),
    ) = (
        single_fact(card, "trend.direction"),
        single_fact(card, "trend.first-observed-date"),
        count_fact(card, "trend.half-size"),
        single_fact(card, "trend.last-observed-date"),
        count_fact(card, "trend.point-count"),
        count_fact(card, "trend.direction-threshold"),
    )
    else {
        return false;
    };
    let (Ok(point_count), Ok(half_size)) =
        (usize::try_from(point_count), usize::try_from(half_size))
    else {
        return false;
    };
    let Some((expected_first_date, expected_last_date)) = exact_local_date_bounds(&card.window)
    else {
        return false;
    };
    let expected_threshold = if baseline_value == 0 {
        100
    } else {
        100u128.max(div_ceil(baseline_value.saturating_mul(10), 100))
    };
    let expected_direction = if current_value >= baseline_value.saturating_add(expected_threshold) {
        "rose"
    } else if baseline_value >= current_value.saturating_add(expected_threshold) {
        "fell"
    } else {
        "stable"
    };
    let expected_limitations = match card.coverage.as_str() {
        "complete-canonical-usage" => Vec::new(),
        "indeterminate-retained-history" => {
            vec!["retention-indeterminate-observed-activity".to_string()]
        }
        _ => return false,
    };
    card.id == "trend.output-tokens.v1"
        && card.metric_id == "tokens.output.daily-median"
        && card.supporting_facts.len() == 8
        && card.sample_count == point_count
        && (TREND_MINIMUM_POINTS..=TREND_MAXIMUM_POINTS).contains(&point_count)
        && point_count % 2 == 0
        && half_size == point_count / 2
        && baseline.id == "insight.fact.trend.output.earlier-median"
        && current.id == "insight.fact.trend.output.later-median"
        && baseline.unit == "tokens"
        && current.unit == "tokens"
        && baseline.source == "canonical"
        && current.source == "canonical"
        && baseline.sample_count == half_size
        && current.sample_count == half_size
        && direction.id == "insight.fact.trend.output.direction"
        && direction.unit == "direction"
        && direction.value == expected_direction
        && direction.sample_count == point_count
        && first_date.id == "insight.fact.trend.output.first-observed-date"
        && first_date.unit == "local-date"
        && first_date.value == expected_first_date.format("%Y-%m-%d").to_string()
        && first_date.sample_count == point_count
        && last_date.id == "insight.fact.trend.output.last-observed-date"
        && last_date.unit == "local-date"
        && last_date.value == expected_last_date.format("%Y-%m-%d").to_string()
        && last_date.sample_count == point_count
        && single_fact(card, "trend.half-size").is_some_and(|fact| {
            fact.id == "insight.fact.trend.output.half-size"
                && fact.unit == "points"
                && fact.sample_count == point_count
        })
        && single_fact(card, "trend.point-count").is_some_and(|fact| {
            fact.id == "insight.fact.trend.output.point-count"
                && fact.unit == "points"
                && fact.sample_count == point_count
        })
        && threshold == expected_threshold
        && single_fact(card, "trend.direction-threshold").is_some_and(|fact| {
            fact.id == "insight.fact.trend.output.threshold"
                && fact.unit == "tokens"
                && fact.sample_count == half_size
        })
        && card.finding
            == format!(
                "The later daily median {expected_direction} relative to the earlier observed half."
            )
        && card.window.start == baseline.window.start
        && card.window.end == current.window.end
        && baseline.window.end == current.window.start
        && card.window.timezone == baseline.window.timezone
        && card.window.timezone == current.window.timezone
        && card
            .supporting_facts
            .iter()
            .filter(|fact| {
                !matches!(
                    fact.id.as_str(),
                    "insight.fact.trend.output.earlier-median"
                        | "insight.fact.trend.output.later-median"
                )
            })
            .all(|fact| same_window(&card.window, &fact.window))
        && card
            .supporting_facts
            .iter()
            .all(|fact| fact.method_id == card.method_id && fact.coverage == card.coverage)
        && card.limitations == expected_limitations
        && card.availability
            == if card.limitations.is_empty() {
                "available"
            } else {
                "partial"
            }
        && card.confidence == evidence_confidence(&card.coverage, point_count, TREND_MINIMUM_POINTS)
}

fn comparison_proof_valid(card: &InsightCard) -> bool {
    let Some(comparison) = &card.comparison else {
        return false;
    };
    let baseline = card
        .supporting_facts
        .iter()
        .find(|fact| fact.id == comparison.baseline_fact_id);
    let current = card
        .supporting_facts
        .iter()
        .find(|fact| fact.id == comparison.current_fact_id);
    if baseline.map(|fact| fact.value.as_str()) != Some(comparison.baseline_value.as_str())
        || current.map(|fact| fact.value.as_str()) != Some(comparison.current_value.as_str())
    {
        return false;
    }
    let (Some(baseline), Some(current)) = (baseline, current) else {
        return false;
    };
    let (Ok(baseline_value), Ok(current_value)) = (
        comparison.baseline_value.parse::<u128>(),
        comparison.current_value.parse::<u128>(),
    ) else {
        return false;
    };
    let expected_relative = (baseline_value > 0)
        .then(|| round6((current_value as f64 / baseline_value as f64 - 1.0) * 100.0));
    comparison.absolute_delta == signed_delta(current_value, baseline_value)
        && comparison.relative_delta_pct == expected_relative
        && baseline.method_id == card.method_id
        && current.method_id == card.method_id
        && match card.family.as_str() {
            "comparison" => {
                let prior_active = single_fact(card, "comparison.prior-active-days");
                let current_active = single_fact(card, "comparison.current-active-days");
                let prior_signature = single_fact(card, "comparison.prior-coverage-signature");
                let current_signature = single_fact(card, "comparison.current-coverage-signature");
                let (
                    Some(prior_active),
                    Some(current_active),
                    Some(prior_signature),
                    Some(current_signature),
                ) = (
                    prior_active,
                    current_active,
                    prior_signature,
                    current_signature,
                )
                else {
                    return false;
                };
                let (Ok(prior_active_days), Ok(current_active_days)) = (
                    prior_active.value.parse::<usize>(),
                    current_active.value.parse::<usize>(),
                ) else {
                    return false;
                };
                let expected_days = usize::try_from(COMPARISON_DAYS).unwrap();
                let exact_windows = exact_local_date_bounds(&baseline.window)
                    .zip(exact_local_date_bounds(&current.window))
                    .is_some_and(
                        |((prior_start, prior_last), (current_start, current_last))| {
                            prior_last.succ_opt() == Some(current_start)
                                && prior_last
                                    .signed_duration_since(prior_start)
                                    .num_days()
                                    .saturating_add(1)
                                    == COMPARISON_DAYS
                                && current_last
                                    .signed_duration_since(current_start)
                                    .num_days()
                                    .saturating_add(1)
                                    == COMPARISON_DAYS
                        },
                    );
                card.id == "comparison.output-tokens.v1"
                    && card.metric_id == "tokens.output"
                    && card.title == "Observed output tokens · adjacent windows"
                    && card.finding
                        == format!(
                            "Observed output tokens changed by {} across adjacent 28-day windows.",
                            comparison.absolute_delta
                        )
                    && card.supporting_facts.len() == 6
                    && card.minimum_sample_count == COMPARISON_MINIMUM_ACTIVE_DAYS * 2
                    && card.sample_count
                        == baseline.sample_count.saturating_add(current.sample_count)
                    && same_window(&card.window, &current.window)
                    && baseline.window.end == current.window.start
                    && baseline.window.timezone == current.window.timezone
                    && exact_windows
                    && baseline.id == "insight.fact.comparison.output.prior"
                    && current.id == "insight.fact.comparison.output.current"
                    && baseline.source == "canonical"
                    && current.source == "canonical"
                    && (COMPARISON_MINIMUM_ACTIVE_DAYS..=expected_days).contains(&prior_active_days)
                    && (COMPARISON_MINIMUM_ACTIVE_DAYS..=expected_days)
                        .contains(&current_active_days)
                    && prior_active.id == "insight.fact.comparison.output.prior-active-days"
                    && current_active.id == "insight.fact.comparison.output.current-active-days"
                    && prior_active.unit == "days"
                    && current_active.unit == "days"
                    && prior_active.source == "derived"
                    && current_active.source == "derived"
                    && prior_active.sample_count == baseline.sample_count
                    && current_active.sample_count == current.sample_count
                    && same_window(&prior_active.window, &baseline.window)
                    && same_window(&current_active.window, &current.window)
                    && prior_signature.id
                        == "insight.fact.comparison.output.prior-coverage-signature"
                    && current_signature.id
                        == "insight.fact.comparison.output.current-coverage-signature"
                    && prior_signature.unit == "coverage-signature"
                    && current_signature.unit == "coverage-signature"
                    && prior_signature.source == "derived"
                    && current_signature.source == "derived"
                    && prior_signature.value == current_signature.value
                    && prior_signature.sample_count == baseline.sample_count
                    && current_signature.sample_count == current.sample_count
                    && same_window(&prior_signature.window, &baseline.window)
                    && same_window(&current_signature.window, &current.window)
                    && matches!(
                        card.coverage.as_str(),
                        "complete-canonical-usage" | "indeterminate-retained-history"
                    )
                    && card.limitations
                        == if card.coverage == "complete-canonical-usage" {
                            Vec::new()
                        } else {
                            vec!["retention-indeterminate-observed-activity".to_string()]
                        }
                    && card.confidence
                        == evidence_confidence(
                            &card.coverage,
                            card.sample_count,
                            COMPARISON_MINIMUM_ACTIVE_DAYS * 2,
                        )
                    && card.supporting_facts.iter().all(|fact| {
                        fact.method_id == card.method_id && fact.coverage == card.coverage
                    })
            }
            "trend" => trend_proof_valid(card, baseline, current, baseline_value, current_value),
            _ => false,
        }
}

fn efficiency_proof_valid(card: &InsightCard) -> bool {
    let (numerator_metric, expected_minimum) = match card.id.as_str() {
        "efficiency.output-tokens-per-active-hour.v1" => ("tokens.output", 1),
        "efficiency.requests-per-active-hour.v1" => {
            ("request.canonical-count", EFFICIENCY_MINIMUM_REQUESTS)
        }
        "efficiency.local-api-equivalent-per-active-hour.v1" => ("cost.local-api-equivalent", 1),
        "efficiency.terminal-errors-per-active-hour.v1" => ("api.terminal-errors", 1),
        _ => return false,
    };
    let Some(numerator) = single_fact(card, numerator_metric) else {
        return false;
    };
    let Some(denominator) = single_fact(card, "activity.active") else {
        return false;
    };
    let Some(rate) = single_fact(card, &card.metric_id) else {
        return false;
    };
    let (Ok(numerator_value), Ok(active_seconds), Ok(rate_value)) = (
        numerator.value.parse::<f64>(),
        denominator.value.parse::<u64>(),
        rate.value.parse::<f64>(),
    ) else {
        return false;
    };
    active_seconds >= EFFICIENCY_MINIMUM_ACTIVE_SECONDS
        && approximately_equal(
            rate_value,
            round6(numerator_value * 3_600.0 / active_seconds as f64),
        )
        && rate.method_id == EFFICIENCY_METHOD
        && rate.sample_count == numerator.sample_count
        && card.sample_count == numerator.sample_count
        && card.minimum_sample_count == expected_minimum
        && all_facts_use_card_window(card)
}

fn reliability_proof_valid(card: &InsightCard) -> bool {
    let (denominator_metric, numerator_metric, rate_metric, expected_minimum) =
        match card.id.as_str() {
            "reliability.api-terminal-error-rate.v1" => (
                "api.terminal-outcomes",
                "api.terminal-errors",
                "reliability.api-terminal-error-rate",
                10,
            ),
            "reliability.api-recovered-retry-rate.v1" => (
                "api.completed-with-attempt-evidence",
                "api.recovered-requests",
                "reliability.api-recovered-retry-rate",
                10,
            ),
            _ => return false,
        };
    let (Some(denominator), Some(numerator), Some(rate)) = (
        count_fact(card, denominator_metric),
        count_fact(card, numerator_metric),
        numeric_fact(card, rate_metric),
    ) else {
        return false;
    };
    if numerator > denominator {
        return false;
    }
    if card.id == "reliability.api-recovered-retry-rate.v1" {
        let Some(retries) = count_fact(card, "api.recovered-retry-count") else {
            return false;
        };
        if retries < numerator {
            return false;
        }
    }
    card.metric_id == rate_metric
        && card.sample_count == usize::try_from(denominator).unwrap_or(usize::MAX)
        && card.minimum_sample_count == expected_minimum
        && approximately_equal(rate, percent(numerator, denominator))
        && card
            .supporting_facts
            .iter()
            .all(|fact| fact.method_id == RELIABILITY_METHOD)
        && all_facts_use_card_window(card)
}

fn tool_name_from_card(card: &InsightCard, suffix: &str) -> Option<String> {
    let name = card.id.strip_prefix("tool.")?.strip_suffix(suffix)?;
    let (classified, transformed) = super::types::classified_tool_name(name);
    (transformed == 0 || matches!(name, "mcp" | "other"))
        .then_some(classified?)
        .filter(|classified| classified == name)
}

fn tool_proof_valid(card: &InsightCard) -> bool {
    let trigger = card.id.ends_with(".recommendation-trigger.v1");
    let suffix = if trigger {
        ".recommendation-trigger.v1"
    } else {
        ".observed-outcomes.v1"
    };
    if tool_name_from_card(card, suffix).is_none()
        || card.metric_id != "tool.observed-outcomes"
        || !all_facts_use_card_window(card)
        || card
            .supporting_facts
            .iter()
            .any(|fact| fact.method_id != TOOL_METHOD)
    {
        return false;
    }
    let results = count_fact(card, "tool.direct-results");
    let failures = count_fact(card, "tool.direct-failures");
    let failure_rate = numeric_fact(card, "tool.direct-failure-rate");
    if failures.is_some() || failure_rate.is_some() || trigger {
        let (Some(results), Some(failures), Some(rate)) = (results, failures, failure_rate) else {
            return false;
        };
        if failures > results || !approximately_equal(rate, percent(failures, results)) {
            return false;
        }
        if trigger
            && (card.sample_count != usize::try_from(results).unwrap_or(usize::MAX)
                || card.minimum_sample_count != 10
                || results < 10
                || rate < 20.0)
        {
            return false;
        }
    } else if trigger {
        return false;
    }
    let median = numeric_fact(card, "tool.duration-median");
    let p95 = numeric_fact(card, "tool.duration-p95");
    if median.is_some() != p95.is_some()
        || median.zip(p95).is_some_and(|(median, p95)| median > p95)
    {
        return false;
    }
    let decisions = count_fact(card, "tool.edit-decisions");
    let accepts = count_fact(card, "tool.edit-accepts");
    let accept_share = numeric_fact(card, "tool.edit-accept-share");
    if accepts.is_some() || accept_share.is_some() {
        let (Some(decisions), Some(accepts), Some(share)) = (decisions, accepts, accept_share)
        else {
            return false;
        };
        if accepts > decisions || !approximately_equal(share, percent(accepts, decisions)) {
            return false;
        }
    }
    let expected_sample = card
        .supporting_facts
        .iter()
        .map(|fact| fact.sample_count)
        .max()
        .unwrap_or(0);
    let derived_available = failure_rate.is_some() || median.is_some() || accept_share.is_some();
    let expected_minimum = if trigger {
        10
    } else if derived_available {
        5
    } else {
        1
    };
    card.sample_count == expected_sample && card.minimum_sample_count == expected_minimum
}

fn share_sum_valid(card: &InsightCard, metric_ids: &[&str], require_unknown: &str) -> bool {
    let facts = card
        .supporting_facts
        .iter()
        .filter(|fact| metric_ids.contains(&fact.metric_id.as_str()))
        .collect::<Vec<_>>();
    if facts.is_empty()
        || !facts.iter().any(|fact| fact.metric_id == require_unknown)
        || facts.iter().any(|fact| {
            fact.value
                .parse::<f64>()
                .ok()
                .is_none_or(|value| !value.is_finite())
        })
    {
        return false;
    }
    let sum = facts
        .iter()
        .filter_map(|fact| fact.value.parse::<f64>().ok())
        .sum::<f64>();
    (sum - 100.0).abs() <= facts.len() as f64 * 0.000_001 + 0.000_001
}

fn routing_proof_valid(card: &InsightCard) -> bool {
    if !all_facts_use_card_window(card)
        || card
            .supporting_facts
            .iter()
            .any(|fact| fact.method_id != ROUTING_METHOD)
    {
        return false;
    }
    match card.id.as_str() {
        "routing.model-request-share.v1" => {
            card.metric_id == "routing.model-request-share"
                && card.minimum_sample_count == 5
                && card
                    .supporting_facts
                    .iter()
                    .all(|fact| fact.sample_count == card.sample_count)
                && share_sum_valid(
                    card,
                    &[
                        "routing.model-request-share",
                        "routing.unknown-model-request-share",
                        "routing.other-mapped-request-share",
                    ],
                    "routing.unknown-model-request-share",
                )
        }
        "routing.model-output-token-share.v1" => {
            card.metric_id == "routing.model-output-token-share"
                && card.minimum_sample_count == 5
                && card
                    .supporting_facts
                    .iter()
                    .all(|fact| fact.sample_count == card.sample_count)
                && share_sum_valid(
                    card,
                    &[
                        "routing.model-output-token-share",
                        "routing.unknown-model-output-token-share",
                        "routing.other-mapped-output-token-share",
                    ],
                    "routing.unknown-model-output-token-share",
                )
        }
        "routing.model-local-cost-share.v1" => {
            let shares = card
                .supporting_facts
                .iter()
                .filter(|fact| {
                    matches!(
                        fact.metric_id.as_str(),
                        "routing.model-local-cost-share" | "routing.other-mapped-local-cost-share"
                    )
                })
                .filter_map(|fact| fact.value.parse::<f64>().ok())
                .collect::<Vec<_>>();
            let (Some(priced), Some(unpriced)) = (
                count_fact(card, "cost.priced-requests"),
                count_fact(card, "cost.unpriced-requests"),
            ) else {
                return false;
            };
            !shares.is_empty()
                && shares.iter().all(|value| value.is_finite())
                && (shares.iter().sum::<f64>() - 100.0).abs()
                    <= shares.len() as f64 * 0.000_001 + 0.000_001
                && card.metric_id == "routing.model-local-cost-share"
                && card.sample_count == usize::try_from(priced).unwrap_or(usize::MAX)
                && card.minimum_sample_count == 5
                && priced >= 5
                && priced.saturating_add(unpriced) >= priced
                && count_fact(card, "cost.priced-tokens").is_some()
                && count_fact(card, "cost.unpriced-tokens").is_some()
        }
        _ => false,
    }
}

fn concentration_proof_valid(card: &InsightCard, report: &InsightReport) -> bool {
    if !all_facts_use_card_window(card)
        || card
            .supporting_facts
            .iter()
            .any(|fact| fact.method_id != CONCENTRATION_METHOD)
    {
        return false;
    }
    match card.id.as_str() {
        "concentration.project-output-hhi.v1" => {
            let (
                Some(hhi),
                Some(projects),
                Some(known),
                Some(unattributed),
                Some(top_share),
                Some(known_share),
                Some(unattributed_share),
            ) = (
                numeric_fact(card, "concentration.project-output-hhi"),
                count_fact(card, "concentration.known-project-count"),
                count_fact(card, "concentration.known-output-weight"),
                count_fact(card, "concentration.unattributed-output-weight"),
                numeric_fact(card, "concentration.top-known-project-share"),
                numeric_fact(card, "concentration.known-output-share"),
                numeric_fact(card, "concentration.unattributed-output-share"),
            )
            else {
                return false;
            };
            let global = known.saturating_add(unattributed);
            projects > 0
                && known > 0
                && card.sample_count == usize::try_from(projects).unwrap_or(usize::MAX)
                && card.minimum_sample_count == 1
                && hhi >= 10_000.0 / projects as f64 - 0.000_001
                && hhi <= 10_000.0
                && top_share >= 100.0 / projects as f64 - 0.000_001
                && approximately_equal(known_share, percent(known, global))
                && approximately_equal(unattributed_share, percent(unattributed, global))
                && approximately_equal(known_share + unattributed_share, 100.0)
        }
        "concentration.top-project-alias.v1" => {
            let (Some(alias), Some(alias_share), Some(hhi_card)) = (
                single_fact(card, "concentration.top-project-alias"),
                numeric_fact(card, "concentration.top-known-project-share"),
                report
                    .cards
                    .iter()
                    .find(|candidate| candidate.id == "concentration.project-output-hhi.v1"),
            ) else {
                return false;
            };
            !alias.value.is_empty()
                && alias.unit == "alias"
                && approximately_equal(
                    alias_share,
                    numeric_fact(hhi_card, "concentration.top-known-project-share")
                        .unwrap_or(f64::NAN),
                )
                && card.sample_count == hhi_card.sample_count
                && card.minimum_sample_count == 1
                && card.privacy_class == "standard"
        }
        _ => false,
    }
}

fn anomaly_proof_valid(card: &InsightCard) -> bool {
    if !card.id.starts_with("anomaly.output-tokens.")
        || !card.id.ends_with(".v1")
        || card.metric_id != "anomaly.daily-output-tokens"
        || card.minimum_sample_count != 7
        || card
            .supporting_facts
            .iter()
            .any(|fact| fact.method_id != ANOMALY_METHOD)
    {
        return false;
    }
    let (
        Some(value),
        Some(median),
        Some(mad),
        Some(deviation),
        Some(threshold),
        Some(value_fact),
        Some(median_fact),
    ) = (
        count_fact(card, "tokens.output.daily"),
        count_fact(card, "anomaly.baseline-median"),
        count_fact(card, "anomaly.baseline-mad"),
        count_fact(card, "anomaly.absolute-deviation"),
        count_fact(card, "anomaly.practical-threshold"),
        single_fact(card, "tokens.output.daily"),
        single_fact(card, "anomaly.baseline-median"),
    )
    else {
        return false;
    };
    if !same_window(&card.window, &value_fact.window)
        || !window_contains(&median_fact.window, &card.window)
        || card.sample_count != median_fact.sample_count
        || deviation != absolute_difference(value, median)
    {
        return false;
    }
    let Some(score_fact) = single_fact(card, "anomaly.robust-score") else {
        return false;
    };
    if mad == 0 {
        score_fact.value == "unavailable"
            && threshold == 1_000u128.max(median)
            && deviation >= threshold
            && card
                .limitations
                .iter()
                .any(|limitation| limitation == "anomaly-mad-zero-fallback")
    } else {
        let Some(score) = score_fact
            .value
            .parse::<f64>()
            .ok()
            .filter(|score| score.is_finite())
        else {
            return false;
        };
        approximately_equal(score, round6(0.67448975 * deviation as f64 / mad as f64))
            && threshold == 100u128.max(div_ceil(median.saturating_mul(25), 100))
            && score >= 3.5
            && deviation >= threshold
    }
}

fn recommendation_proof_valid(card: &InsightCard, report: &InsightReport) -> bool {
    if !all_facts_use_card_window(card)
        || card
            .supporting_facts
            .iter()
            .any(|fact| fact.method_id != RECOMMENDATION_METHOD)
    {
        return false;
    }
    let Some(reference) = single_fact(card, "reference.card") else {
        return false;
    };
    let Some(target) = report
        .cards
        .iter()
        .find(|candidate| candidate.id == reference.value)
    else {
        return false;
    };
    if target.class != "factual" {
        return false;
    }
    let (numerator_metric, denominator_metric, rate_metric, threshold_metric, threshold) =
        match card.id.as_str() {
            "recommendation.api-terminal-errors.v1" => (
                "api.terminal-errors",
                "api.terminal-outcomes",
                "reliability.api-terminal-error-rate",
                "recommendation.threshold",
                10.0,
            ),
            "recommendation.tool-result-errors.v1" => (
                "tool.direct-failures",
                "tool.direct-results",
                "tool.direct-failure-rate",
                "recommendation.threshold",
                20.0,
            ),
            "recommendation.model-routing-experiment.v1" => {
                let (
                    Some(numerator),
                    Some(denominator),
                    Some(top_share),
                    Some(top_threshold),
                    Some(unknown_share),
                    Some(unknown_threshold),
                ) = (
                    count_fact(card, "routing.top-model-requests"),
                    count_fact(card, "routing.total-model-observations"),
                    numeric_fact(card, "routing.top-model-request-share"),
                    numeric_fact(card, "recommendation.top-share-threshold"),
                    numeric_fact(card, "routing.unknown-model-request-share"),
                    numeric_fact(card, "recommendation.unknown-share-maximum"),
                )
                else {
                    return false;
                };
                return target.id == "routing.model-request-share.v1"
                    && denominator >= 20
                    && numerator <= denominator
                    && card.sample_count == usize::try_from(denominator).unwrap_or(usize::MAX)
                    && card.minimum_sample_count == 20
                    && approximately_equal(top_share, percent(numerator, denominator))
                    && approximately_equal(top_threshold, 80.0)
                    && approximately_equal(unknown_threshold, 10.0)
                    && top_share >= top_threshold
                    && unknown_share <= unknown_threshold
                    && target.supporting_facts.iter().any(|fact| {
                        fact.metric_id == "routing.unknown-model-request-share"
                            && fact.value == decimal(unknown_share)
                    })
                    && target.supporting_facts.iter().any(|fact| {
                        fact.metric_id == "routing.model-request-share"
                            && fact.value == decimal(top_share)
                    });
            }
            _ => return false,
        };
    let (Some(numerator), Some(denominator), Some(rate), Some(actual_threshold)) = (
        count_fact(card, numerator_metric),
        count_fact(card, denominator_metric),
        numeric_fact(card, rate_metric),
        numeric_fact(card, threshold_metric),
    ) else {
        return false;
    };
    denominator >= 10
        && numerator <= denominator
        && card.sample_count == usize::try_from(denominator).unwrap_or(usize::MAX)
        && card.minimum_sample_count == 10
        && approximately_equal(rate, percent(numerator, denominator))
        && approximately_equal(actual_threshold, threshold)
        && rate >= threshold
        && count_fact(target, numerator_metric) == Some(numerator)
        && count_fact(target, denominator_metric) == Some(denominator)
        && numeric_fact(target, rate_metric).is_some_and(|value| approximately_equal(value, rate))
}

fn entertainment_proof_valid(card: &InsightCard, report: &InsightReport) -> bool {
    if !all_facts_use_card_window(card)
        || card
            .supporting_facts
            .iter()
            .any(|fact| fact.method_id != ENTERTAINMENT_METHOD)
        || !card
            .limitations
            .iter()
            .any(|limitation| limitation == "entertainment-not-a-factual-assessment")
    {
        return false;
    }
    match card.id.as_str() {
        "entertainment.archetype.v1" => {
            let (
                Some(observations),
                Some(active_days),
                Some(subagents),
                Some(tool_bearing),
                Some(hhi),
            ) = (
                count_fact(card, "request.canonical-count"),
                count_fact(card, "activity.observed-active-days"),
                count_fact(card, "entertainment.subagent-observations"),
                count_fact(card, "entertainment.tool-bearing-observations"),
                numeric_fact(card, "concentration.project-output-hhi"),
            )
            else {
                return false;
            };
            let label = if subagents.saturating_mul(100) >= observations.saturating_mul(30) {
                "The Orchestrator"
            } else if tool_bearing >= observations {
                "The Toolsmith"
            } else if hhi >= 2_500.0 {
                "The Specialist"
            } else {
                "The Explorer"
            };
            observations >= 20
                && active_days >= 5
                && subagents <= observations
                && tool_bearing <= observations
                && card.sample_count == usize::try_from(observations).unwrap_or(usize::MAX)
                && card.minimum_sample_count == 20
                && card.title == format!("Entertainment · {label}")
        }
        "entertainment.cache-mood.v1" => {
            let Some(fact) = single_fact(card, "cache.read-share") else {
                return false;
            };
            fact.coverage == "available"
                && numeric_fact(card, "cache.read-share").is_some()
                && card.sample_count == fact.sample_count
                && card.minimum_sample_count == 1
        }
        "entertainment.momentum.v1" => {
            let Some(reference) = single_fact(card, "reference.card") else {
                return false;
            };
            let Some(trend) = report.cards.iter().find(|target| {
                target.id == reference.value
                    && target.id == "trend.output-tokens.v1"
                    && target.availability == "available"
            }) else {
                return false;
            };
            card.sample_count == trend.sample_count
                && reference.sample_count == trend.sample_count
                && card.minimum_sample_count == TREND_MINIMUM_POINTS
        }
        _ => false,
    }
}

fn narrative_matches(card: &InsightCard, title: &str, finding: String) -> bool {
    card.title == title && card.finding == finding
}

fn exact_action(card: &InsightCard, experiment: &str, alternatives: &[&str]) -> bool {
    card.action.as_ref().is_some_and(|action| {
        action.experiment == experiment
            && action
                .alternative_explanations
                .iter()
                .map(String::as_str)
                .eq(alternatives.iter().copied())
    })
}

fn routing_model_from_fact_id(fact: &InsightFact, suffix: &str) -> Option<String> {
    (fact.metric_id == "routing.model-request-share")
        .then(|| {
            fact.id
                .strip_prefix("insight.fact.routing.")?
                .strip_suffix(suffix)
                .map(str::to_string)
        })
        .flatten()
}

fn top_routing_model(report: &InsightReport) -> Option<String> {
    let request_card = report
        .cards
        .iter()
        .find(|card| card.id == "routing.model-request-share.v1")?;
    let output_card = report
        .cards
        .iter()
        .find(|card| card.id == "routing.model-output-token-share.v1");
    let mut candidates = request_card
        .supporting_facts
        .iter()
        .filter_map(|fact| {
            let name = routing_model_from_fact_id(fact, ".request-share")?;
            let request_share = fact.value.parse::<f64>().ok()?;
            if !request_share.is_finite() {
                return None;
            }
            let output_share = output_card
                .and_then(|card| {
                    card.supporting_facts.iter().find(|candidate| {
                        candidate.id == format!("insight.fact.routing.{name}.output-share")
                    })
                })
                .and_then(|fact| fact.value.parse::<f64>().ok())
                .filter(|value| value.is_finite())
                .unwrap_or(0.0);
            Some((name, request_share, output_share))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(
        |(left_name, left_requests, left_output), (right_name, right_requests, right_output)| {
            right_requests
                .total_cmp(left_requests)
                .then_with(|| right_output.total_cmp(left_output))
                .then_with(|| left_name.cmp(right_name))
        },
    );
    candidates.into_iter().next().map(|(name, _, _)| name)
}

fn card_narrative_valid(card: &InsightCard, report: &InsightReport) -> bool {
    let no_action = card.action.is_none();
    match card.id.as_str() {
        "comparison.output-tokens.v1" => {
            let Some(comparison) = &card.comparison else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed output tokens · adjacent windows",
                    format!(
                        "Observed output tokens changed by {} across adjacent 28-day windows.",
                        comparison.absolute_delta
                    ),
                )
        }
        "trend.output-tokens.v1" => {
            let Some(direction) = single_fact(card, "trend.direction") else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed output-token trend",
                    format!(
                        "The later daily median {} relative to the earlier observed half.",
                        direction.value
                    ),
                )
        }
        "efficiency.output-tokens-per-active-hour.v1" => {
            let Some(rate) = single_fact(card, &card.metric_id) else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed output per active hour",
                    format!(
                        "{} output tokens per observed unioned active hour.",
                        rate.value
                    ),
                )
        }
        "efficiency.requests-per-active-hour.v1" => {
            let Some(rate) = single_fact(card, &card.metric_id) else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed requests per active hour",
                    format!(
                        "{} canonical request/message observations per unioned active hour.",
                        rate.value
                    ),
                )
        }
        "efficiency.local-api-equivalent-per-active-hour.v1" => {
            let Some(rate) = single_fact(card, &card.metric_id) else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed local API-equivalent estimate per active hour",
                    format!(
                        "${} local API-equivalent estimate per observed unioned active hour.",
                        rate.value
                    ),
                )
        }
        "efficiency.terminal-errors-per-active-hour.v1" => {
            let Some(rate) = single_fact(card, &card.metric_id) else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed terminal API errors per active hour",
                    format!(
                        "{} direct terminal API errors per observed unioned active hour.",
                        rate.value
                    ),
                )
        }
        "reliability.api-terminal-error-rate.v1" => {
            let (Some(outcomes), Some(rate)) = (
                count_fact(card, "api.terminal-outcomes"),
                single_fact(card, "reliability.api-terminal-error-rate"),
            ) else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Terminal API outcome rate",
                    format!(
                        "{}% of {outcomes} direct terminal API outcomes were errors emitted after retries were exhausted.",
                        rate.value
                    ),
                )
        }
        "reliability.api-recovered-retry-rate.v1" => {
            let (Some(outcomes), Some(rate)) = (
                count_fact(card, "api.completed-with-attempt-evidence"),
                single_fact(card, "reliability.api-recovered-retry-rate"),
            ) else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Recovered retries on completed requests",
                    format!(
                        "{}% of {outcomes} completed direct requests with attempt evidence recovered after at least one retry.",
                        rate.value
                    ),
                )
        }
        "routing.model-request-share.v1" => {
            let finding = top_routing_model(report)
                .and_then(|model| {
                    card.supporting_facts
                        .iter()
                        .find(|fact| {
                            fact.id == format!("insight.fact.routing.{model}.request-share")
                        })
                        .map(|fact| {
                            format!(
                                "{model} represented {}% of canonical request/message observations.",
                                fact.value
                            )
                        })
                })
                .unwrap_or_else(|| "Mapped model observations were unavailable.".to_string());
            no_action && narrative_matches(card, "Observed model request share", finding)
        }
        "routing.model-output-token-share.v1" => {
            no_action
                && narrative_matches(
                    card,
                    "Observed model output-token share",
                    "Canonical output-token shares are descriptive and include unknown-model coverage."
                        .to_string(),
                )
        }
        "routing.model-local-cost-share.v1" => {
            no_action
                && narrative_matches(
                    card,
                    "Local API-equivalent model share",
                    "Priced local API-equivalent estimate shares exclude source-recorded and billing domains."
                        .to_string(),
                )
        }
        "concentration.project-output-hhi.v1" => {
            let (Some(hhi), Some(projects), Some(top_share)) = (
                numeric_fact(card, "concentration.project-output-hhi"),
                count_fact(card, "concentration.known-project-count"),
                numeric_fact(card, "concentration.top-known-project-share"),
            ) else {
                return false;
            };
            let label = if hhi >= 2_500.0 || top_share >= 70.0 {
                "concentrated"
            } else if hhi <= 1_500.0 && projects >= 4 && top_share < 50.0 {
                "distributed"
            } else {
                "mixed"
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed project concentration",
                    format!(
                        "Known project output-token weights were {label} under the declared HHI thresholds."
                    ),
                )
        }
        "concentration.top-project-alias.v1" => {
            let (Some(alias), Some(share)) = (
                single_fact(card, "concentration.top-project-alias"),
                single_fact(card, "concentration.top-known-project-share"),
            ) else {
                return false;
            };
            no_action
                && narrative_matches(
                    card,
                    "Observed top project alias",
                    format!(
                        "{} represented {}% of known attributed output tokens.",
                        alias.value, share.value
                    ),
                )
        }
        "recommendation.api-terminal-errors.v1" => {
            let (Some(errors), Some(outcomes), Some(rate)) = (
                count_fact(card, "api.terminal-errors"),
                count_fact(card, "api.terminal-outcomes"),
                single_fact(card, "reliability.api-terminal-error-rate"),
            ) else {
                return false;
            };
            narrative_matches(
                card,
                "Review terminal API errors with a controlled rerun",
                format!(
                    "{errors} of {outcomes} direct terminal outcomes were errors ({}%).",
                    rate.value
                ),
            ) && exact_action(
                card,
                "Repeat a controlled 10-request sample after checking local configuration and connectivity, then compare the same terminal-outcome rate.",
                &[
                    "A transient service or network condition may explain the observed errors.",
                    "The task or input mix may differ between the observed and controlled samples.",
                ],
            )
        }
        "recommendation.tool-result-errors.v1" => {
            let (Some(reference), Some(errors), Some(results), Some(rate)) = (
                single_fact(card, "reference.card"),
                count_fact(card, "tool.direct-failures"),
                count_fact(card, "tool.direct-results"),
                single_fact(card, "tool.direct-failure-rate"),
            ) else {
                return false;
            };
            let Some(tool) = report
                .cards
                .iter()
                .find(|candidate| candidate.id == reference.value)
                .and_then(|target| {
                    tool_name_from_card(target, ".observed-outcomes.v1").or_else(|| {
                        tool_name_from_card(target, ".recommendation-trigger.v1")
                    })
                })
            else {
                return false;
            };
            narrative_matches(
                card,
                "Test the highest observed tool-result error rate",
                format!(
                    "{errors} of {results} direct {tool} results were errors ({}%).",
                    rate.value
                ),
            ) && exact_action(
                card,
                "Repeat a small controlled workflow for the classified tool after checking inputs and permissions, then compare the same direct-result rate.",
                &[
                    "The observed task mix or invalid inputs may explain the result rate.",
                    "Permissions, tool versions, or environment changes may explain the result rate.",
                ],
            )
        }
        "recommendation.model-routing-experiment.v1" => {
            let (Some(observations), Some(share), Some(model)) = (
                count_fact(card, "routing.total-model-observations"),
                single_fact(card, "routing.top-model-request-share"),
                top_routing_model(report),
            ) else {
                return false;
            };
            narrative_matches(
                card,
                "Run a bounded model-routing experiment",
                format!(
                    "{model} represented {}% of {observations} canonical request/message observations.",
                    share.value
                ),
            ) && exact_action(
                card,
                "Review 10 already-known interchangeable tasks, try your chosen alternative model on that bounded set, and compare task-defined outcomes plus canonical request and local-cost evidence.",
                &[
                    "Task complexity or a deliberate user policy may explain the observed concentration.",
                    "Missing model evidence or deliberate specialization may explain the observed concentration.",
                ],
            )
        }
        "entertainment.cache-mood.v1" => {
            no_action
                && narrative_matches(
                    card,
                    "Entertainment · Cache cartographer",
                    "A playful label based only on the available canonical cache-read share; it does not infer a cause or monetary effect."
                        .to_string(),
                )
        }
        "entertainment.momentum.v1" => {
            no_action
                && narrative_matches(
                    card,
                    "Entertainment · Observed momentum",
                    "A playful presentation of the available declared trend card; it is not a factual assessment."
                        .to_string(),
                )
        }
        "entertainment.archetype.v1" => {
            no_action
                && card.finding
                    == "A deterministic playful label based on sample-gated aggregate activity; it is not a factual assessment."
                && card.title.starts_with("Entertainment · ")
        }
        _ if card.id.starts_with("tool.") => {
            let trigger = card.id.ends_with(".recommendation-trigger.v1");
            let suffix = if trigger {
                ".recommendation-trigger.v1"
            } else {
                ".observed-outcomes.v1"
            };
            let Some(name) = tool_name_from_card(card, suffix) else {
                return false;
            };
            let finding = if let (Some(rate), Some(results)) = (
                single_fact(card, "tool.direct-failure-rate"),
                count_fact(card, "tool.direct-results"),
            ) {
                format!(
                    "{}% of {results} direct {name} results were errors.",
                    rate.value
                )
            } else if let Some(results) = count_fact(card, "tool.direct-results") {
                format!("{name} has {results} observed direct result(s).")
            } else if let Some(decisions) = count_fact(card, "tool.edit-decisions") {
                format!("{name} has {decisions} observed edit decision(s).")
            } else if let Some(occurrences) = count_fact(card, "tool.occurrences") {
                format!("{name} has {occurrences} observed occurrence(s).")
            } else {
                return false;
            };
            let title = if trigger {
                format!("{name} · recommendation trigger evidence")
            } else {
                format!("{name} · observed tool evidence")
            };
            no_action && card.title == title && card.finding == finding
        }
        _ if card.id.starts_with("anomaly.output-tokens.") && card.id.ends_with(".v1") => {
            let Some(date) = card
                .id
                .strip_prefix("anomaly.output-tokens.")
                .and_then(|id| id.strip_suffix(".v1"))
                .filter(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").is_ok())
            else {
                return false;
            };
            let Some(value) = single_fact(card, "tokens.output.daily") else {
                return false;
            };
            no_action
                && card.title == format!("Unusual observed output · {date}")
                && card.finding
                    == format!(
                        "{} output tokens were unusual within observed activity under the declared robust baseline.",
                        value.value
                    )
        }
        _ => false,
    }
}

fn card_proof_valid(card: &InsightCard, report: &InsightReport) -> bool {
    let family_valid = match card.family.as_str() {
        "comparison" | "trend" => comparison_proof_valid(card),
        "active-efficiency" => efficiency_proof_valid(card),
        "reliability" => reliability_proof_valid(card),
        "tool-behavior" => tool_proof_valid(card),
        "model-routing" => routing_proof_valid(card),
        "project-concentration" => concentration_proof_valid(card, report),
        "anomaly" => anomaly_proof_valid(card),
        "recommendation" => recommendation_proof_valid(card, report),
        "entertainment" => entertainment_proof_valid(card, report),
        _ => false,
    };
    family_valid && card_narrative_valid(card, report)
}

fn active_efficiency_family_valid(report: &InsightReport, evidence: ValidationEvidence) -> bool {
    let Some(family) = report
        .families
        .iter()
        .find(|family| family.family == "active-efficiency")
    else {
        return false;
    };
    if family.sample_count != evidence.active_efficiency_sample_count {
        return false;
    }

    let cards = report
        .cards
        .iter()
        .filter(|card| card.family == "active-efficiency")
        .collect::<Vec<_>>();
    let exact_active = evidence.active_time_available
        && evidence.active_seconds >= EFFICIENCY_MINIMUM_ACTIVE_SECONDS;
    if !exact_active {
        return cards.is_empty()
            && family.availability == "unavailable"
            && family.limitations == ["efficiency-minimum-active-seconds"];
    }

    if family
        .limitations
        .iter()
        .any(|limitation| limitation == "efficiency-minimum-active-seconds")
    {
        return false;
    }
    let request_card = cards
        .iter()
        .find(|card| card.id == "efficiency.requests-per-active-hour.v1");
    let request_gate_satisfied =
        evidence.active_efficiency_sample_count >= EFFICIENCY_MINIMUM_REQUESTS;
    let request_limitation_present = family
        .limitations
        .iter()
        .any(|limitation| limitation == "efficiency-minimum-request-observations");
    if request_card.is_some() != request_gate_satisfied
        || request_limitation_present == request_gate_satisfied
    {
        return false;
    }
    if let Some(card) = request_card {
        let expected = evidence.active_efficiency_sample_count.to_string();
        if card.sample_count != evidence.active_efficiency_sample_count
            || !card.supporting_facts.iter().any(|fact| {
                fact.metric_id == "request.canonical-count"
                    && fact.value == expected
                    && fact.sample_count == evidence.active_efficiency_sample_count
                    && fact.unit == "observations"
            })
        {
            return false;
        }
    }

    family.availability
        == if cards.is_empty() {
            "unavailable"
        } else if family.limitations.is_empty() {
            "available"
        } else {
            "partial"
        }
}

pub(super) fn validate(
    report: &InsightReport,
    methodology: &MethodologyCatalog,
    evidence: ValidationEvidence,
) -> Result<(), super::IngestionError> {
    if report.version != REPORT_VERSION
        || report.cards.len() > MAX_CARDS
        || report
            .cards
            .iter()
            .any(|card| card.supporting_facts.len() > MAX_FACTS_PER_CARD)
    {
        return Err(invalid_insights());
    }
    let card_ids = report
        .cards
        .iter()
        .map(|card| card.id.as_str())
        .collect::<BTreeSet<_>>();
    if card_ids.len() != report.cards.len()
        || report.cards.windows(2).any(|cards| {
            cards[0]
                .renderer_priority
                .cmp(&cards[1].renderer_priority)
                .then_with(|| cards[0].family.cmp(&cards[1].family))
                .then_with(|| cards[0].id.cmp(&cards[1].id))
                .is_gt()
        })
        || report
            .cards
            .iter()
            .filter(|card| card.family == "tool-behavior")
            .count()
            > 10
        || report
            .cards
            .iter()
            .filter(|card| card.family == "anomaly")
            .count()
            > 3
        || report
            .cards
            .iter()
            .filter(|card| card.family == "recommendation")
            .count()
            > 3
    {
        return Err(invalid_insights());
    }
    let mut fact_ids = BTreeSet::new();
    let family_names = report
        .families
        .iter()
        .map(|family| family.family.as_str())
        .collect::<BTreeSet<_>>();
    if report.families.len() != FAMILIES.len()
        || family_names.len() != report.families.len()
        || report
            .families
            .iter()
            .zip(FAMILIES)
            .any(|(actual, (family, capabilities))| {
                actual.family != family
                    || actual.minimum_sample_count != family_minimum_sample_count(family)
                    || actual.required_capabilities
                        != capabilities
                            .iter()
                            .map(|capability| (*capability).to_string())
                            .collect::<Vec<_>>()
            })
        || report.families.iter().any(|family| {
            !matches!(
                family.availability.as_str(),
                "available" | "partial" | "unavailable"
            ) || !unique_strings(&family.required_capabilities)
                || !unique_strings(&family.limitations)
                || family.minimum_sample_count == 0
                || (family.availability == "partial" && family.limitations.is_empty())
                || (family.availability == "unavailable" && family.limitations.is_empty())
        })
    {
        return Err(invalid_insights());
    }
    if !active_efficiency_family_valid(report, evidence)
        || report.families.iter().any(|family| {
            family.availability == "unavailable"
                && report.cards.iter().any(|card| card.family == family.family)
        })
    {
        return Err(invalid_insights());
    }
    for card in &report.cards {
        let proof_valid = card_proof_valid(card, report);
        let expected_method = match card.family.as_str() {
            "comparison" => COMPARISON_METHOD,
            "trend" => TREND_METHOD,
            "active-efficiency" => EFFICIENCY_METHOD,
            "reliability" => RELIABILITY_METHOD,
            "tool-behavior" => TOOL_METHOD,
            "model-routing" => ROUTING_METHOD,
            "project-concentration" => CONCENTRATION_METHOD,
            "anomaly" => ANOMALY_METHOD,
            "recommendation" => RECOMMENDATION_METHOD,
            "entertainment" => ENTERTAINMENT_METHOD,
            _ => return Err(invalid_insights()),
        };
        let expected_class = match card.family.as_str() {
            "recommendation" => "recommendation",
            "entertainment" => "entertainment",
            _ => "factual",
        };
        let action_valid = match (expected_class, card.action.as_ref()) {
            ("recommendation", Some(action)) => {
                !action.experiment.is_empty()
                    && !action.alternative_explanations.is_empty()
                    && action
                        .alternative_explanations
                        .iter()
                        .all(|alternative| !alternative.is_empty())
            }
            ("recommendation", None) => false,
            (_, None) => true,
            (_, Some(_)) => false,
        };
        if !family_names.contains(card.family.as_str())
            || !methodology.methods.contains_key(&card.method_id)
            || card.method_id != expected_method
            || card.version != "1"
            || card.class != expected_class
            || card.title.is_empty()
            || card.finding.is_empty()
            || card.metric_id.is_empty()
            || !valid_window(&card.window)
            || card.coverage.is_empty()
            || !matches!(card.availability.as_str(), "available" | "partial")
            || !matches!(
                card.confidence.as_str(),
                "high" | "medium" | "low" | "unavailable"
            )
            || (card.availability == "partial" && card.limitations.is_empty())
            || !unique_strings(&card.limitations)
            || !matches!(card.privacy_class.as_str(), "share" | "standard")
            || !action_valid
            || card.sample_count < card.minimum_sample_count
            || card.minimum_sample_count == 0
            || card.supporting_facts.is_empty()
            || card
                .supporting_facts
                .windows(2)
                .any(|facts| facts[0].id >= facts[1].id)
            || card
                .comparison
                .as_ref()
                .and_then(|comparison| comparison.relative_delta_pct)
                .is_some_and(|value| !value.is_finite())
            || !proof_valid
        {
            return Err(invalid_insights());
        }
        if card.class == "recommendation" {
            let Some(reference) = card
                .supporting_facts
                .iter()
                .find(|fact| fact.metric_id == "reference.card")
            else {
                return Err(invalid_insights());
            };
            if reference.value == card.id || !card_ids.contains(reference.value.as_str()) {
                return Err(invalid_insights());
            }
        }
        if card.privacy_class == "share"
            && card
                .supporting_facts
                .iter()
                .any(|fact| fact.metric_id == "concentration.top-project-alias")
        {
            return Err(invalid_insights());
        }
        for fact in &card.supporting_facts {
            if !fact_ids.insert(fact.id.as_str())
                || !methodology.methods.contains_key(&fact.method_id)
                || fact.id.is_empty()
                || fact.metric_id.is_empty()
                || fact.unit.is_empty()
                || fact.coverage.is_empty()
                || fact.source.is_empty()
                || !valid_window(&fact.window)
                || !valid_numeric_fact(fact)
            {
                return Err(invalid_insights());
            }
        }
    }
    Ok(())
}

fn same_window(left: &InsightWindow, right: &InsightWindow) -> bool {
    left.start == right.start && left.end == right.end && left.timezone == right.timezone
}

fn invalid_insights() -> super::IngestionError {
    super::IngestionError::internal(
        "E_INSIGHT_RECONCILIATION",
        "explainable insight proof objects did not reconcile",
        "Retry with the same inputs; if the error persists, report the tool version and error code without attaching private history.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn validate(
        report: &InsightReport,
        methodology: &MethodologyCatalog,
    ) -> Result<(), super::super::IngestionError> {
        let active_family = report
            .families
            .iter()
            .find(|family| family.family == "active-efficiency")
            .unwrap();
        let has_active_cards = report
            .cards
            .iter()
            .any(|card| card.family == "active-efficiency");
        super::validate(
            report,
            methodology,
            ValidationEvidence {
                active_efficiency_sample_count: active_family.sample_count,
                active_time_available: has_active_cards,
                active_seconds: if has_active_cards {
                    EFFICIENCY_MINIMUM_ACTIVE_SECONDS
                } else {
                    0
                },
            },
        )
    }

    fn valid_comparison_report() -> (InsightReport, MethodologyCatalog) {
        let mut methodology = MethodologyCatalog::default();
        install_methods(&mut methodology);
        let prior_window = InsightWindow {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-01-29T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let current_window = InsightWindow {
            start: "2026-01-29T00:00:00Z".to_string(),
            end: "2026-02-26T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let prior = exact_fact(
            "insight.fact.comparison.output.prior",
            "tokens.output",
            "100".to_string(),
            "tokens",
            COMPARISON_METHOD,
            prior_window.clone(),
            7,
            "complete-canonical-usage".to_string(),
            "canonical",
        );
        let prior_active = exact_fact(
            "insight.fact.comparison.output.prior-active-days",
            "comparison.prior-active-days",
            "7".to_string(),
            "days",
            COMPARISON_METHOD,
            prior_window.clone(),
            7,
            "complete-canonical-usage".to_string(),
            "derived",
        );
        let prior_signature = exact_fact(
            "insight.fact.comparison.output.prior-coverage-signature",
            "comparison.prior-coverage-signature",
            "transcript/v1:AssistantUsage".to_string(),
            "coverage-signature",
            COMPARISON_METHOD,
            prior_window,
            7,
            "complete-canonical-usage".to_string(),
            "derived",
        );
        let current = exact_fact(
            "insight.fact.comparison.output.current",
            "tokens.output",
            "200".to_string(),
            "tokens",
            COMPARISON_METHOD,
            current_window.clone(),
            7,
            "complete-canonical-usage".to_string(),
            "canonical",
        );
        let current_active = exact_fact(
            "insight.fact.comparison.output.current-active-days",
            "comparison.current-active-days",
            "7".to_string(),
            "days",
            COMPARISON_METHOD,
            current_window.clone(),
            7,
            "complete-canonical-usage".to_string(),
            "derived",
        );
        let current_signature = exact_fact(
            "insight.fact.comparison.output.current-coverage-signature",
            "comparison.current-coverage-signature",
            "transcript/v1:AssistantUsage".to_string(),
            "coverage-signature",
            COMPARISON_METHOD,
            current_window.clone(),
            7,
            "complete-canonical-usage".to_string(),
            "derived",
        );
        let mut report = InsightReport {
            version: REPORT_VERSION.to_string(),
            families: FAMILIES
                .iter()
                .map(|(family, capabilities)| unavailable_family(family, capabilities))
                .collect(),
            cards: vec![InsightCard {
                id: "comparison.output-tokens.v1".to_string(),
                version: "1".to_string(),
                family: "comparison".to_string(),
                class: "factual".to_string(),
                title: "Observed output tokens · adjacent windows".to_string(),
                finding: "Observed output tokens changed by 100 across adjacent 28-day windows."
                    .to_string(),
                metric_id: "tokens.output".to_string(),
                comparison: Some(InsightComparison {
                    baseline_fact_id: prior.id.clone(),
                    current_fact_id: current.id.clone(),
                    baseline_value: "100".to_string(),
                    current_value: "200".to_string(),
                    absolute_delta: "100".to_string(),
                    relative_delta_pct: Some(100.0),
                }),
                window: current_window,
                sample_count: 14,
                minimum_sample_count: 14,
                method_id: COMPARISON_METHOD.to_string(),
                availability: "available".to_string(),
                coverage: "complete-canonical-usage".to_string(),
                confidence: "medium".to_string(),
                supporting_facts: vec![
                    prior,
                    prior_active,
                    prior_signature,
                    current,
                    current_active,
                    current_signature,
                ],
                limitations: Vec::new(),
                action: None,
                privacy_class: "standard".to_string(),
                renderer_priority: 100,
            }],
        };
        if let Some(family) = report
            .families
            .iter_mut()
            .find(|family| family.family == "comparison")
        {
            family.availability = "available".to_string();
            family.sample_count = 14;
            family.minimum_sample_count = 14;
            family.limitations.clear();
        }
        report.families.sort_by(|left, right| {
            family_rank(&left.family)
                .cmp(&family_rank(&right.family))
                .then_with(|| left.family.cmp(&right.family))
        });
        for card in &mut report.cards {
            card.supporting_facts
                .sort_by(|left, right| left.id.cmp(&right.id));
        }
        (report, methodology)
    }

    fn fabricated_zero_exception_comparison_report() -> (InsightReport, MethodologyCatalog) {
        let (mut report, methodology) = valid_comparison_report();
        let card = &mut report.cards[0];
        let prior_window = card
            .supporting_facts
            .iter()
            .find(|fact| {
                fact.metric_id == "tokens.output" && !same_window(&fact.window, &card.window)
            })
            .unwrap()
            .window
            .clone();
        card.comparison.as_mut().unwrap().baseline_value = "0".to_string();
        card.comparison.as_mut().unwrap().absolute_delta = "200".to_string();
        card.comparison.as_mut().unwrap().relative_delta_pct = None;
        card.finding =
            "Observed output tokens changed by 200 across adjacent 28-day windows.".to_string();
        card.supporting_facts
            .iter_mut()
            .find(|fact| fact.id == "insight.fact.comparison.output.prior")
            .unwrap()
            .value = "0".to_string();
        card.supporting_facts
            .iter_mut()
            .find(|fact| fact.metric_id == "comparison.prior-active-days")
            .unwrap()
            .value = "0".to_string();
        card.supporting_facts.push(exact_fact(
            "insight.fact.comparison.output.prior-zero-coverage-days",
            "comparison.prior-zero-coverage-days",
            COMPARISON_DAYS.to_string(),
            "days",
            COMPARISON_METHOD,
            prior_window,
            7,
            "complete-canonical-usage".to_string(),
            "otel-metric",
        ));
        card.supporting_facts
            .sort_by(|left, right| left.id.cmp(&right.id));
        (report, methodology)
    }

    fn valid_reliability_report() -> (InsightReport, MethodologyCatalog) {
        let mut methodology = MethodologyCatalog::default();
        install_methods(&mut methodology);
        let window = InsightWindow {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-02-01T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let mut report = InsightReport {
            version: REPORT_VERSION.to_string(),
            families: FAMILIES
                .iter()
                .map(|(family, capabilities)| unavailable_family(family, capabilities))
                .collect(),
            cards: vec![InsightCard {
                id: "reliability.api-terminal-error-rate.v1".to_string(),
                version: "1".to_string(),
                family: "reliability".to_string(),
                class: "factual".to_string(),
                title: "Terminal API outcome rate".to_string(),
                finding: "20% of 10 direct terminal API outcomes were errors emitted after retries were exhausted.".to_string(),
                metric_id: "reliability.api-terminal-error-rate".to_string(),
                comparison: None,
                window: window.clone(),
                sample_count: 10,
                minimum_sample_count: 10,
                method_id: RELIABILITY_METHOD.to_string(),
                availability: "available".to_string(),
                coverage: "complete-direct-otel".to_string(),
                confidence: "medium".to_string(),
                supporting_facts: vec![
                    exact_fact(
                        "insight.fact.reliability.api-terminal-outcomes",
                        "api.terminal-outcomes",
                        "10".to_string(),
                        "outcomes",
                        RELIABILITY_METHOD,
                        window.clone(),
                        10,
                        "complete-direct-otel".to_string(),
                        "otel-event",
                    ),
                    exact_fact(
                        "insight.fact.reliability.api-terminal-errors",
                        "api.terminal-errors",
                        "2".to_string(),
                        "errors",
                        RELIABILITY_METHOD,
                        window.clone(),
                        2,
                        "complete-direct-otel".to_string(),
                        "otel-event",
                    ),
                    exact_fact(
                        "insight.fact.reliability.api-terminal-error-rate",
                        "reliability.api-terminal-error-rate",
                        "20".to_string(),
                        "percent",
                        RELIABILITY_METHOD,
                        window,
                        10,
                        "complete-direct-otel".to_string(),
                        "derived",
                    ),
                ],
                limitations: Vec::new(),
                action: None,
                privacy_class: "share".to_string(),
                renderer_priority: 130,
            }],
        };
        if let Some(family) = report
            .families
            .iter_mut()
            .find(|family| family.family == "reliability")
        {
            family.availability = "available".to_string();
            family.sample_count = 10;
            family.minimum_sample_count = 10;
            family.limitations.clear();
        }
        report.families.sort_by(|left, right| {
            family_rank(&left.family)
                .cmp(&family_rank(&right.family))
                .then_with(|| left.family.cmp(&right.family))
        });
        for card in &mut report.cards {
            card.supporting_facts
                .sort_by(|left, right| left.id.cmp(&right.id));
        }
        (report, methodology)
    }

    fn proof_card(
        id: &str,
        family: &str,
        metric_id: &str,
        window: InsightWindow,
        sample_count: usize,
        minimum_sample_count: usize,
        facts: Vec<InsightFact>,
    ) -> InsightCard {
        let (method_id, renderer_priority) = match family {
            "trend" => (TREND_METHOD, 110),
            "active-efficiency" => (EFFICIENCY_METHOD, 120),
            "tool-behavior" => (TOOL_METHOD, 140),
            "model-routing" => (ROUTING_METHOD, 150),
            "project-concentration" => (CONCENTRATION_METHOD, 160),
            "anomaly" => (ANOMALY_METHOD, 170),
            "recommendation" => (RECOMMENDATION_METHOD, 200),
            "entertainment" => (ENTERTAINMENT_METHOD, 300),
            _ => panic!("unsupported proof-card fixture family"),
        };
        InsightCard {
            id: id.to_string(),
            version: "1".to_string(),
            family: family.to_string(),
            class: match family {
                "recommendation" => "recommendation",
                "entertainment" => "entertainment",
                _ => "factual",
            }
            .to_string(),
            title: "proof fixture".to_string(),
            finding: "proof fixture".to_string(),
            metric_id: metric_id.to_string(),
            comparison: None,
            window,
            sample_count,
            minimum_sample_count,
            method_id: method_id.to_string(),
            availability: "available".to_string(),
            coverage: "complete".to_string(),
            confidence: if family == "entertainment" {
                "unavailable"
            } else {
                "medium"
            }
            .to_string(),
            supporting_facts: facts,
            limitations: if family == "entertainment" {
                vec!["entertainment-not-a-factual-assessment".to_string()]
            } else {
                Vec::new()
            },
            action: None,
            privacy_class: if matches!(family, "tool-behavior" | "recommendation") {
                "standard"
            } else {
                "share"
            }
            .to_string(),
            renderer_priority,
        }
    }

    fn full_family_proof_report() -> (InsightReport, MethodologyCatalog) {
        let mut methodology = MethodologyCatalog::default();
        install_methods(&mut methodology);
        let window = InsightWindow {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-02-01T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let mut cards = Vec::new();
        cards.push(valid_comparison_report().0.cards.remove(0));

        let earlier_window = InsightWindow {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-01-05T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let later_window = InsightWindow {
            start: "2026-01-05T00:00:00Z".to_string(),
            end: "2026-01-09T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let trend_window = InsightWindow {
            start: earlier_window.start.clone(),
            end: later_window.end.clone(),
            timezone: "UTC".to_string(),
        };
        let earlier = exact_fact(
            "insight.fact.trend.output.earlier-median",
            "tokens.output.daily-median",
            "100".to_string(),
            "tokens",
            TREND_METHOD,
            earlier_window,
            4,
            "complete-canonical-usage".to_string(),
            "canonical",
        );
        let later = exact_fact(
            "insight.fact.trend.output.later-median",
            "tokens.output.daily-median",
            "200".to_string(),
            "tokens",
            TREND_METHOD,
            later_window,
            4,
            "complete-canonical-usage".to_string(),
            "canonical",
        );
        let mut trend = proof_card(
            "trend.output-tokens.v1",
            "trend",
            "tokens.output.daily-median",
            trend_window.clone(),
            8,
            8,
            vec![
                exact_fact(
                    "insight.fact.trend.output.direction",
                    "trend.direction",
                    "rose".to_string(),
                    "direction",
                    TREND_METHOD,
                    trend_window.clone(),
                    8,
                    "complete-canonical-usage".to_string(),
                    "derived",
                ),
                earlier.clone(),
                exact_fact(
                    "insight.fact.trend.output.first-observed-date",
                    "trend.first-observed-date",
                    "2026-01-01".to_string(),
                    "local-date",
                    TREND_METHOD,
                    trend_window.clone(),
                    8,
                    "complete-canonical-usage".to_string(),
                    "derived",
                ),
                exact_fact(
                    "insight.fact.trend.output.half-size",
                    "trend.half-size",
                    "4".to_string(),
                    "points",
                    TREND_METHOD,
                    trend_window.clone(),
                    8,
                    "complete-canonical-usage".to_string(),
                    "derived",
                ),
                exact_fact(
                    "insight.fact.trend.output.last-observed-date",
                    "trend.last-observed-date",
                    "2026-01-08".to_string(),
                    "local-date",
                    TREND_METHOD,
                    trend_window.clone(),
                    8,
                    "complete-canonical-usage".to_string(),
                    "derived",
                ),
                later.clone(),
                exact_fact(
                    "insight.fact.trend.output.point-count",
                    "trend.point-count",
                    "8".to_string(),
                    "points",
                    TREND_METHOD,
                    trend_window.clone(),
                    8,
                    "complete-canonical-usage".to_string(),
                    "derived",
                ),
                exact_fact(
                    "insight.fact.trend.output.threshold",
                    "trend.direction-threshold",
                    "100".to_string(),
                    "tokens",
                    TREND_METHOD,
                    trend_window,
                    4,
                    "complete-canonical-usage".to_string(),
                    "derived",
                ),
            ],
        );
        trend.finding =
            "The later daily median rose relative to the earlier observed half.".to_string();
        trend.title = "Observed output-token trend".to_string();
        trend.coverage = "complete-canonical-usage".to_string();
        trend.comparison = Some(InsightComparison {
            baseline_fact_id: earlier.id,
            current_fact_id: later.id,
            baseline_value: "100".to_string(),
            current_value: "200".to_string(),
            absolute_delta: "100".to_string(),
            relative_delta_pct: Some(100.0),
        });
        cards.push(trend);

        let mut efficiency = proof_card(
            "efficiency.output-tokens-per-active-hour.v1",
            "active-efficiency",
            "efficiency.output-tokens-per-active-hour",
            window.clone(),
            6,
            1,
            vec![
                exact_fact(
                    "fixture.efficiency.active",
                    "activity.active",
                    "1500".to_string(),
                    "seconds",
                    EFFICIENCY_METHOD,
                    window.clone(),
                    5,
                    "available".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.efficiency.output",
                    "tokens.output",
                    "600".to_string(),
                    "tokens",
                    EFFICIENCY_METHOD,
                    window.clone(),
                    6,
                    "available".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.efficiency.rate",
                    "efficiency.output-tokens-per-active-hour",
                    "1440".to_string(),
                    "tokens/hour",
                    EFFICIENCY_METHOD,
                    window.clone(),
                    6,
                    "available".to_string(),
                    "derived",
                ),
            ],
        );
        efficiency.title = "Observed output per active hour".to_string();
        efficiency.finding = "1440 output tokens per observed unioned active hour.".to_string();
        cards.push(efficiency);

        cards.push(valid_reliability_report().0.cards.remove(0));
        let mut tool = proof_card(
            "tool.Bash.observed-outcomes.v1",
            "tool-behavior",
            "tool.observed-outcomes",
            window.clone(),
            10,
            5,
            vec![
                exact_fact(
                    "fixture.tool.failures",
                    "tool.direct-failures",
                    "2".to_string(),
                    "errors",
                    TOOL_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "fixture.tool.rate",
                    "tool.direct-failure-rate",
                    "20".to_string(),
                    "percent",
                    TOOL_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.tool.results",
                    "tool.direct-results",
                    "10".to_string(),
                    "results",
                    TOOL_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
            ],
        );
        tool.title = "Bash · observed tool evidence".to_string();
        tool.finding = "20% of 10 direct Bash results were errors.".to_string();
        cards.push(tool);
        let mut routing = proof_card(
            "routing.model-request-share.v1",
            "model-routing",
            "routing.model-request-share",
            window.clone(),
            20,
            5,
            vec![
                exact_fact(
                    "insight.fact.routing.claude-sonnet-4-6.request-share",
                    "routing.model-request-share",
                    "80".to_string(),
                    "percent",
                    ROUTING_METHOD,
                    window.clone(),
                    20,
                    "complete-canonical-usage".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "insight.fact.routing.unknown.request-share",
                    "routing.unknown-model-request-share",
                    "20".to_string(),
                    "percent",
                    ROUTING_METHOD,
                    window.clone(),
                    20,
                    "complete-canonical-usage".to_string(),
                    "canonical",
                ),
            ],
        );
        routing.title = "Observed model request share".to_string();
        routing.finding =
            "claude-sonnet-4-6 represented 80% of canonical request/message observations."
                .to_string();
        cards.push(routing);
        let mut concentration = proof_card(
            "concentration.project-output-hhi.v1",
            "project-concentration",
            "concentration.project-output-hhi",
            window.clone(),
            1,
            1,
            vec![
                exact_fact(
                    "fixture.concentration.hhi",
                    "concentration.project-output-hhi",
                    "10000".to_string(),
                    "hhi-0-10000",
                    CONCENTRATION_METHOD,
                    window.clone(),
                    1,
                    "complete".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.concentration.known-share",
                    "concentration.known-output-share",
                    "100".to_string(),
                    "percent",
                    CONCENTRATION_METHOD,
                    window.clone(),
                    1,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.concentration.known-weight",
                    "concentration.known-output-weight",
                    "100".to_string(),
                    "tokens",
                    CONCENTRATION_METHOD,
                    window.clone(),
                    1,
                    "complete".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.concentration.projects",
                    "concentration.known-project-count",
                    "1".to_string(),
                    "projects",
                    CONCENTRATION_METHOD,
                    window.clone(),
                    1,
                    "complete".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.concentration.top-share",
                    "concentration.top-known-project-share",
                    "100".to_string(),
                    "percent",
                    CONCENTRATION_METHOD,
                    window.clone(),
                    1,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.concentration.unattributed-share",
                    "concentration.unattributed-output-share",
                    "0".to_string(),
                    "percent",
                    CONCENTRATION_METHOD,
                    window.clone(),
                    1,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.concentration.unattributed-weight",
                    "concentration.unattributed-output-weight",
                    "0".to_string(),
                    "tokens",
                    CONCENTRATION_METHOD,
                    window.clone(),
                    1,
                    "complete".to_string(),
                    "canonical",
                ),
            ],
        );
        concentration.title = "Observed project concentration".to_string();
        concentration.finding =
            "Known project output-token weights were concentrated under the declared HHI thresholds."
                .to_string();
        cards.push(concentration);

        let anomaly_day = InsightWindow {
            start: "2026-01-05T00:00:00Z".to_string(),
            end: "2026-01-06T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let anomaly_baseline = InsightWindow {
            start: "2026-01-01T00:00:00Z".to_string(),
            end: "2026-01-08T00:00:00Z".to_string(),
            timezone: "UTC".to_string(),
        };
        let mut anomaly = proof_card(
            "anomaly.output-tokens.2026-01-05.v1",
            "anomaly",
            "anomaly.daily-output-tokens",
            anomaly_day.clone(),
            7,
            7,
            vec![
                exact_fact(
                    "fixture.anomaly.deviation",
                    "anomaly.absolute-deviation",
                    "900".to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    anomaly_baseline.clone(),
                    7,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.anomaly.mad",
                    "anomaly.baseline-mad",
                    "100".to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    anomaly_baseline.clone(),
                    7,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.anomaly.median",
                    "anomaly.baseline-median",
                    "100".to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    anomaly_baseline.clone(),
                    7,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.anomaly.threshold",
                    "anomaly.practical-threshold",
                    "100".to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    anomaly_baseline.clone(),
                    7,
                    "complete".to_string(),
                    "method-parameter",
                ),
                exact_fact(
                    "fixture.anomaly.score",
                    "anomaly.robust-score",
                    "6.070408".to_string(),
                    "score",
                    ANOMALY_METHOD,
                    anomaly_baseline,
                    7,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.anomaly.value",
                    "tokens.output.daily",
                    "1000".to_string(),
                    "tokens",
                    ANOMALY_METHOD,
                    anomaly_day,
                    1,
                    "complete".to_string(),
                    "canonical",
                ),
            ],
        );
        anomaly.title = "Unusual observed output · 2026-01-05".to_string();
        anomaly.finding =
            "1000 output tokens were unusual within observed activity under the declared robust baseline."
                .to_string();
        cards.push(anomaly);

        let mut recommendation = proof_card(
            "recommendation.api-terminal-errors.v1",
            "recommendation",
            "recommendation.api-terminal-errors",
            window.clone(),
            10,
            10,
            vec![
                exact_fact(
                    "fixture.recommendation.denominator",
                    "api.terminal-outcomes",
                    "10".to_string(),
                    "outcomes",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "fixture.recommendation.numerator",
                    "api.terminal-errors",
                    "2".to_string(),
                    "errors",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "otel-event",
                ),
                exact_fact(
                    "fixture.recommendation.rate",
                    "reliability.api-terminal-error-rate",
                    "20".to_string(),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.recommendation.reference",
                    "reference.card",
                    "reliability.api-terminal-error-rate.v1".to_string(),
                    "card-id",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "reference",
                ),
                exact_fact(
                    "fixture.recommendation.threshold",
                    "recommendation.threshold",
                    "10".to_string(),
                    "percent",
                    RECOMMENDATION_METHOD,
                    window.clone(),
                    10,
                    "complete-direct-otel".to_string(),
                    "method-parameter",
                ),
            ],
        );
        recommendation.title = "Review terminal API errors with a controlled rerun".to_string();
        recommendation.finding = "2 of 10 direct terminal outcomes were errors (20%).".to_string();
        recommendation.action = Some(InsightAction {
            experiment: "Repeat a controlled 10-request sample after checking local configuration and connectivity, then compare the same terminal-outcome rate.".to_string(),
            alternative_explanations: vec![
                "A transient service or network condition may explain the observed errors."
                    .to_string(),
                "The task or input mix may differ between the observed and controlled samples."
                    .to_string(),
            ],
        });
        cards.push(recommendation);

        let mut entertainment = proof_card(
            "entertainment.archetype.v1",
            "entertainment",
            "entertainment.archetype",
            window.clone(),
            20,
            20,
            vec![
                exact_fact(
                    "fixture.entertainment.active-days",
                    "activity.observed-active-days",
                    "5".to_string(),
                    "days",
                    ENTERTAINMENT_METHOD,
                    window.clone(),
                    5,
                    "complete".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.entertainment.hhi",
                    "concentration.project-output-hhi",
                    "0".to_string(),
                    "hhi-0-10000",
                    ENTERTAINMENT_METHOD,
                    window.clone(),
                    20,
                    "complete".to_string(),
                    "derived",
                ),
                exact_fact(
                    "fixture.entertainment.observations",
                    "request.canonical-count",
                    "20".to_string(),
                    "observations",
                    ENTERTAINMENT_METHOD,
                    window.clone(),
                    20,
                    "complete".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.entertainment.subagents",
                    "entertainment.subagent-observations",
                    "6".to_string(),
                    "observations",
                    ENTERTAINMENT_METHOD,
                    window.clone(),
                    20,
                    "complete".to_string(),
                    "canonical",
                ),
                exact_fact(
                    "fixture.entertainment.tools",
                    "entertainment.tool-bearing-observations",
                    "20".to_string(),
                    "observations",
                    ENTERTAINMENT_METHOD,
                    window,
                    20,
                    "complete".to_string(),
                    "canonical",
                ),
            ],
        );
        entertainment.title = "Entertainment · The Orchestrator".to_string();
        entertainment.finding = "A deterministic playful label based on sample-gated aggregate activity; it is not a factual assessment.".to_string();
        cards.push(entertainment);

        let mut report = InsightReport {
            version: REPORT_VERSION.to_string(),
            families: FAMILIES
                .iter()
                .map(|(family, capabilities)| unavailable_family(family, capabilities))
                .collect(),
            cards,
        };
        for family in &mut report.families {
            let family_cards = report
                .cards
                .iter()
                .filter(|card| card.family == family.family)
                .collect::<Vec<_>>();
            if family_cards.is_empty() {
                continue;
            }
            family.availability = "available".to_string();
            family.sample_count = family_cards
                .iter()
                .map(|card| card.sample_count)
                .max()
                .unwrap_or(0);
            family.minimum_sample_count = if family.family == "active-efficiency" {
                EFFICIENCY_MINIMUM_REQUESTS
            } else {
                family_cards
                    .iter()
                    .map(|card| card.minimum_sample_count)
                    .min()
                    .unwrap_or(1)
            };
            family.limitations = if family.family == "entertainment" {
                vec!["entertainment-not-a-factual-assessment".to_string()]
            } else if family.family == "active-efficiency" {
                family.sample_count = EFFICIENCY_MINIMUM_REQUESTS - 1;
                family.availability = "partial".to_string();
                vec!["efficiency-minimum-request-observations".to_string()]
            } else {
                Vec::new()
            };
        }
        for card in &mut report.cards {
            card.supporting_facts
                .sort_by(|left, right| left.id.cmp(&right.id));
        }
        report.cards.sort_by(|left, right| {
            left.renderer_priority
                .cmp(&right.renderer_priority)
                .then_with(|| left.family.cmp(&right.family))
                .then_with(|| left.id.cmp(&right.id))
        });
        (report, methodology)
    }

    #[test]
    fn production_insight_reconciliation_rejects_fact_method_sample_and_window_mutations() {
        let (report, methodology) = valid_comparison_report();
        assert!(validate(&report, &methodology).is_ok());

        let mut fact = report.clone();
        fact.cards[0]
            .supporting_facts
            .iter_mut()
            .find(|fact| fact.id == "insight.fact.comparison.output.current")
            .unwrap()
            .value = "201".to_string();
        assert_eq!(
            validate(&fact, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut method = report.clone();
        method.cards[0].method_id = TREND_METHOD.to_string();
        assert_eq!(
            validate(&method, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut sample = report.clone();
        sample.cards[0].sample_count = 15;
        assert_eq!(
            validate(&sample, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut window = report;
        window.cards[0].window.end = "2026-02-27T00:00:00Z".to_string();
        assert_eq!(
            validate(&window, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );
    }

    #[test]
    fn production_insight_reconciliation_rejects_a_fabricated_zero_baseline_waiver() {
        let (report, methodology) = fabricated_zero_exception_comparison_report();
        assert_eq!(
            validate(&report, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );
    }

    #[test]
    fn production_insight_reconciliation_rejects_noncomparison_arithmetic_mutations() {
        let (report, methodology) = valid_reliability_report();
        assert!(validate(&report, &methodology).is_ok());

        let mut value = report.clone();
        value.cards[0].supporting_facts[2].value = "21".to_string();
        assert_eq!(
            validate(&value, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut fact_method = report.clone();
        fact_method.cards[0].supporting_facts[2].method_id = TREND_METHOD.to_string();
        assert_eq!(
            validate(&fact_method, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut sample = report.clone();
        sample.cards[0].sample_count = 11;
        assert_eq!(
            validate(&sample, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut window = report;
        window.cards[0].window.end = "2026-02-02T00:00:00Z".to_string();
        assert_eq!(
            validate(&window, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );
    }

    #[test]
    fn production_insight_reconciliation_rejects_active_efficiency_family_mutations() {
        let (report, methodology) = full_family_proof_report();
        let active_family_index = report
            .families
            .iter()
            .position(|family| family.family == "active-efficiency")
            .unwrap();
        let evidence = ValidationEvidence {
            active_efficiency_sample_count: report.families[active_family_index].sample_count,
            active_time_available: true,
            active_seconds: EFFICIENCY_MINIMUM_ACTIVE_SECONDS,
        };
        assert!(super::validate(&report, &methodology, evidence).is_ok());

        let mut sample_count = report.clone();
        sample_count.families[active_family_index].sample_count = sample_count.families
            [active_family_index]
            .sample_count
            .saturating_add(1);
        assert_eq!(
            super::validate(&sample_count, &methodology, evidence)
                .unwrap_err()
                .code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut availability = report.clone();
        availability.families[active_family_index].availability = "available".to_string();
        assert_eq!(
            super::validate(&availability, &methodology, evidence)
                .unwrap_err()
                .code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut gate_limitation = report;
        gate_limitation.families[active_family_index]
            .limitations
            .clear();
        assert_eq!(
            super::validate(&gate_limitation, &methodology, evidence)
                .unwrap_err()
                .code(),
            "E_INSIGHT_RECONCILIATION"
        );
    }

    #[test]
    fn production_insight_reconciliation_rejects_narrative_and_action_mutations() {
        let (report, methodology) = full_family_proof_report();
        assert!(validate(&report, &methodology).is_ok());

        for (index, card) in report.cards.iter().enumerate() {
            for field in ["title", "finding"] {
                let mut mutation = report.clone();
                match field {
                    "title" => {
                        mutation.cards[index].title = "Forged authoritative title".to_string();
                    }
                    "finding" => {
                        mutation.cards[index].finding = "Forged authoritative finding.".to_string();
                    }
                    _ => unreachable!(),
                }
                assert_eq!(
                    validate(&mutation, &methodology).unwrap_err().code(),
                    "E_INSIGHT_RECONCILIATION",
                    "mutation to {field} survived for {}",
                    card.id
                );
            }
        }

        let recommendation_index = report
            .cards
            .iter()
            .position(|card| card.id == "recommendation.api-terminal-errors.v1")
            .unwrap();
        let mut experiment = report.clone();
        experiment.cards[recommendation_index]
            .action
            .as_mut()
            .unwrap()
            .experiment = "Trust the forged recommendation.".to_string();
        assert_eq!(
            validate(&experiment, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut alternative = report.clone();
        alternative.cards[recommendation_index]
            .action
            .as_mut()
            .unwrap()
            .alternative_explanations[0] = "The evidence proves a single cause.".to_string();
        assert_eq!(
            validate(&alternative, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );

        let mut order = report;
        order.cards[recommendation_index]
            .action
            .as_mut()
            .unwrap()
            .alternative_explanations
            .swap(0, 1);
        assert_eq!(
            validate(&order, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );
    }

    #[test]
    fn production_insight_reconciliation_rejects_reliability_narrative_mutations() {
        let (report, methodology) = valid_reliability_report();
        assert!(validate(&report, &methodology).is_ok());

        for field in ["title", "finding"] {
            let mut mutation = report.clone();
            match field {
                "title" => mutation.cards[0].title = "Forged authoritative title".to_string(),
                "finding" => {
                    mutation.cards[0].finding =
                        "All direct terminal outcomes succeeded.".to_string();
                }
                _ => unreachable!(),
            }
            assert_eq!(
                validate(&mutation, &methodology).unwrap_err().code(),
                "E_INSIGHT_RECONCILIATION",
                "mutation to {field} must fail closed"
            );
        }
    }

    #[test]
    fn production_insight_reconciliation_rejects_arithmetic_mutations_across_every_family() {
        let (report, methodology) = full_family_proof_report();
        assert!(validate(&report, &methodology).is_ok());

        for (card_id, fact_id, invalid_value) in [
            (
                "comparison.output-tokens.v1",
                "insight.fact.comparison.output.prior",
                "101",
            ),
            (
                "trend.output-tokens.v1",
                "insight.fact.trend.output.earlier-median",
                "101",
            ),
            (
                "efficiency.output-tokens-per-active-hour.v1",
                "fixture.efficiency.rate",
                "1441",
            ),
            (
                "reliability.api-terminal-error-rate.v1",
                "insight.fact.reliability.api-terminal-error-rate",
                "21",
            ),
            ("tool.Bash.observed-outcomes.v1", "fixture.tool.rate", "21"),
            (
                "routing.model-request-share.v1",
                "insight.fact.routing.claude-sonnet-4-6.request-share",
                "79",
            ),
            (
                "concentration.project-output-hhi.v1",
                "fixture.concentration.known-share",
                "99",
            ),
            (
                "anomaly.output-tokens.2026-01-05.v1",
                "fixture.anomaly.score",
                "6",
            ),
            (
                "recommendation.api-terminal-errors.v1",
                "fixture.recommendation.threshold",
                "11",
            ),
            (
                "entertainment.archetype.v1",
                "fixture.entertainment.subagents",
                "0",
            ),
        ] {
            let mut mutation = report.clone();
            mutation
                .cards
                .iter_mut()
                .find(|card| card.id == card_id)
                .and_then(|card| {
                    card.supporting_facts
                        .iter_mut()
                        .find(|fact| fact.id == fact_id)
                })
                .unwrap()
                .value = invalid_value.to_string();
            assert_eq!(
                validate(&mutation, &methodology).unwrap_err().code(),
                "E_INSIGHT_RECONCILIATION",
                "mutation survived for {card_id}/{fact_id}"
            );
        }
    }

    #[test]
    fn production_insight_reconciliation_rejects_trend_method_mutations() {
        let (report, methodology) = full_family_proof_report();
        assert!(validate(&report, &methodology).is_ok());

        for (metric_id, invalid_value) in [
            ("trend.direction", "stable"),
            ("trend.first-observed-date", "2026-01-02"),
            ("trend.half-size", "3"),
            ("trend.last-observed-date", "2026-01-07"),
            ("trend.point-count", "10"),
            ("trend.direction-threshold", "101"),
        ] {
            let mut mutation = report.clone();
            mutation
                .cards
                .iter_mut()
                .find(|card| card.id == "trend.output-tokens.v1")
                .and_then(|card| {
                    card.supporting_facts
                        .iter_mut()
                        .find(|fact| fact.metric_id == metric_id)
                })
                .unwrap()
                .value = invalid_value.to_string();
            assert_eq!(
                validate(&mutation, &methodology).unwrap_err().code(),
                "E_INSIGHT_RECONCILIATION",
                "trend mutation survived for {metric_id}"
            );
        }

        let mut finding = report.clone();
        finding
            .cards
            .iter_mut()
            .find(|card| card.id == "trend.output-tokens.v1")
            .unwrap()
            .finding =
            "The later daily median stable relative to the earlier observed half.".to_string();
        assert_eq!(
            validate(&finding, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );
    }

    #[test]
    fn production_insight_reconciliation_enforces_the_cardinality_bound() {
        let (template, methodology) = valid_comparison_report();
        let template = template.cards[0].clone();
        let (mut report, _) = valid_comparison_report();
        report.cards = vec![template; MAX_CARDS + 1];
        assert_eq!(
            validate(&report, &methodology).unwrap_err().code(),
            "E_INSIGHT_RECONCILIATION"
        );
    }

    #[test]
    fn project_concentration_keeps_unattributed_weight_outside_hhi_and_aliases_out_of_share() {
        let token_value = |observed| ccwrapped::TokenMetricValue {
            observed,
            availability: "available".to_string(),
            sample_count: 1,
            method_id: "tokens/canonical-sum/v1".to_string(),
            unit: "tokens".to_string(),
            ..ccwrapped::TokenMetricValue::default()
        };
        let mut metrics = CanonicalMetrics::default();
        metrics.tokens.global.output = token_value(1_000);
        metrics.tokens.projects = vec![NamedTokenMetricSet {
            key: "project-1".to_string(),
            tokens: ccwrapped::TokenMetricSet {
                output: token_value(400),
                ..ccwrapped::TokenMetricSet::default()
            },
        }];
        metrics.tokens.project_unattributed.output = token_value(600);
        let mut coverage = DataCoverage {
            completeness: "complete".to_string(),
            ..DataCoverage::default()
        };
        coverage
            .capabilities
            .insert("analysis_usage_totals".to_string(), "available".to_string());
        let time = TimeContext::new("UTC", Some(2026)).unwrap();
        let mut report = InsightReport::default();
        build_project_concentration(&metrics, &[], &coverage, &time, &mut report);

        let shared = report
            .cards
            .iter()
            .find(|card| card.id == "concentration.project-output-hhi.v1")
            .unwrap();
        let value = |metric: &str| {
            shared
                .supporting_facts
                .iter()
                .find(|fact| fact.metric_id == metric)
                .unwrap()
                .value
                .as_str()
        };
        assert_eq!(value("concentration.project-output-hhi"), "10000");
        assert_eq!(value("concentration.known-output-weight"), "400");
        assert_eq!(value("concentration.unattributed-output-weight"), "600");
        assert_eq!(value("concentration.known-output-share"), "40");
        assert_eq!(value("concentration.unattributed-output-share"), "60");
        assert_eq!(shared.privacy_class, "share");
        assert!(shared
            .supporting_facts
            .iter()
            .all(|fact| fact.metric_id != "concentration.top-project-alias"));

        let alias = report
            .cards
            .iter()
            .find(|card| card.id == "concentration.top-project-alias.v1")
            .unwrap();
        assert_eq!(alias.privacy_class, "standard");
        assert_eq!(
            single_fact(alias, "concentration.top-project-alias")
                .unwrap()
                .value,
            "project-1"
        );
    }
}
