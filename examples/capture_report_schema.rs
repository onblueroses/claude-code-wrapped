use ccwrapped::*;
use serde::{Serialize, Serializer};
use serde_json::Value;
use std::collections::BTreeMap;

fn value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn capture<T>(name: &str)
where
    T: Default + Serialize,
{
    let value = serde_json::to_value(T::default()).expect("default value must serialize");
    let object = value
        .as_object()
        .unwrap_or_else(|| panic!("{name} must serialize as an object"));

    println!("struct {name}");
    for (field, value) in object {
        println!("json-field {name}::{field}: {}", value_kind(value));
    }
}

fn emit_paths(path: &str, value: &Value) {
    println!("json-path {path}: {}", value_kind(value));
    match value {
        Value::Object(object) => {
            for (field, child) in object {
                emit_paths(&format!("{path}.{field}"), child);
            }
        }
        Value::Array(items) => {
            for (index, child) in items.iter().enumerate() {
                emit_paths(&format!("{path}[{index}]"), child);
            }
        }
        _ => {}
    }
}

fn capture_fixture<T>(name: &str, value: &T)
where
    T: Serialize,
{
    println!("fixture {name}");
    let value = serde_json::to_value(value).expect("fixture must serialize");
    emit_paths("$", &value);
}

fn sample_map<T>(value: T) -> BTreeMap<String, T> {
    BTreeMap::from([("sample".to_string(), value)])
}

fn sample_usage() -> TokenUsage {
    TokenUsage {
        input_tokens: 1,
        output_tokens: 2,
        cache_creation_tokens: 3,
        cache_read_tokens: 4,
    }
}

fn sample_token_metric(observed: u64) -> TokenMetricValue {
    TokenMetricValue {
        observed,
        unit: "tokens".to_string(),
        availability: "available".to_string(),
        sample_count: 1,
        overflowed: false,
        method_id: "tokens/canonical-sum/v1".to_string(),
        limitations: vec!["synthetic token limitation".to_string()],
    }
}

fn sample_token_metrics() -> TokenMetricSet {
    TokenMetricSet {
        input: sample_token_metric(1),
        output: sample_token_metric(2),
        cache_creation: sample_token_metric(3),
        cache_read: sample_token_metric(4),
        cache_creation_5m: sample_token_metric(3),
        cache_creation_1h: sample_token_metric(0),
        total: sample_token_metric(10),
    }
}

fn sample_cost_metric(method_id: &str, amount_usd: Option<f64>) -> CostMetricValue {
    CostMetricValue {
        amount_usd,
        unit: "USD".to_string(),
        availability: if amount_usd.is_some() {
            "available".to_string()
        } else {
            "unavailable".to_string()
        },
        quality: "modeled".to_string(),
        method_id: method_id.to_string(),
        source: Some("synthetic-registry".to_string()),
        sample_count: 1,
        limitations: vec!["synthetic limitation".to_string()],
    }
}

fn sample_methodology() -> MethodologyCatalog {
    MethodologyCatalog {
        timezone_database: "IANA 2025b via chrono-tz 0.10.4".to_string(),
        methods: sample_map(MetricMethod {
            version: "1".to_string(),
            description: "synthetic method".to_string(),
            parameters: sample_map("synthetic parameter".to_string()),
        }),
        pricing_registry: PricingRegistryMetadata {
            version: "synthetic-registry".to_string(),
            citation: "https://example.invalid/pricing".to_string(),
            access_date: "2025-01-02".to_string(),
            selection_policy: "pricing/exact/v1".to_string(),
            records: vec![PricingRegistryRecordMetadata {
                provider: "synthetic-provider".to_string(),
                canonical_model: "synthetic-model".to_string(),
                aliases: vec!["synthetic-alias".to_string()],
                effective_start: Some("2025-01-01".to_string()),
                effective_end: None,
                modifier: "standard".to_string(),
                input_pico_usd_per_token: 1,
                output_pico_usd_per_token: 2,
                cache_read_pico_usd_per_token: 3,
                cache_write_5m_pico_usd_per_token: 4,
                cache_write_1h_pico_usd_per_token: 5,
                citation: "https://example.invalid/pricing".to_string(),
                access_date: "2025-01-02".to_string(),
            }],
        },
    }
}

fn sample_canonical_metrics() -> CanonicalMetrics {
    let tokens = sample_token_metrics();
    CanonicalMetrics {
        active_time: ActiveTimeMetrics {
            method_id: "activity/capped-interval-union/v1".to_string(),
            unit: "seconds".to_string(),
            availability: "available".to_string(),
            interval_count: 1,
            threshold_seconds: 300,
            total_elapsed_seconds: 360,
            total_active_seconds: 300,
            main_exclusive_seconds: 240,
            subagent_exclusive_seconds: 60,
            days: vec![DailyActiveTime {
                date: "2025-01-02".to_string(),
                active_seconds: 300,
            }],
            models: vec![NamedActiveTime {
                key: "claude-sonnet-4-6".to_string(),
                active_seconds: 300,
                inclusive_active_seconds: 300,
            }],
            projects: vec![NamedActiveTime {
                key: "project-hash".to_string(),
                active_seconds: 300,
                inclusive_active_seconds: 300,
            }],
            project_unattributed_active_seconds: 0,
            project_unattributed_inclusive_active_seconds: 0,
            sessions: vec![NamedActiveTime {
                key: "session-main".to_string(),
                active_seconds: 300,
                inclusive_active_seconds: 360,
            }],
            limitations: vec!["synthetic active-time limitation".to_string()],
        },
        tokens: CanonicalTokenMetrics {
            global: tokens.clone(),
            days: vec![NamedTokenMetricSet {
                key: "2025-01-02".to_string(),
                tokens: tokens.clone(),
            }],
            models: vec![NamedTokenMetricSet {
                key: "claude-sonnet-4-6".to_string(),
                tokens: tokens.clone(),
            }],
            projects: vec![NamedTokenMetricSet {
                key: "project-hash".to_string(),
                tokens: tokens.clone(),
            }],
            project_unattributed: TokenMetricSet::default(),
            sessions: vec![NamedTokenMetricSet {
                key: "session-main".to_string(),
                tokens,
            }],
            unattributed: TokenMetricSet::default(),
        },
        cost: CanonicalCostMetrics {
            source_recorded: sample_cost_metric("cost/source-estimate/v1", Some(0.25)),
            local_api_equivalent: sample_cost_metric("cost/api-equivalent/v1", Some(0.20)),
            billing_authoritative: sample_cost_metric("cost/billing/v1", None),
            coverage: "partial".to_string(),
            priced_tokens: 10,
            priced_tokens_overflowed: false,
            unpriced_tokens: 2,
            unpriced_tokens_overflowed: false,
            priced_requests: 1,
            unpriced_requests: 1,
            priced_token_share_pct: Some(83.3),
            models: vec![ModelCostEvidence {
                raw_model: "claude-sonnet-4-6".to_string(),
                provider: "anthropic-api".to_string(),
                canonical_model: Some("claude-sonnet-4-6".to_string()),
                pricing_key: Some("synthetic-key".to_string()),
                pricing_modifier: "standard".to_string(),
                source_recorded_usd: Some(0.25),
                local_api_equivalent_usd: Some(0.20),
                priced_tokens: 10,
                unpriced_tokens: 2,
                priced_requests: 1,
                unpriced_requests: 1,
                requests: 2,
                coverage: "partial".to_string(),
            }],
        },
        cache: CanonicalCacheMetrics {
            read_share: RatioMetric {
                value_pct: Some(80.0),
                unit: "percent".to_string(),
                numerator: 4,
                denominator: 5,
                sample_count: 1,
                overflowed: false,
                availability: "available".to_string(),
                method_id: "cache/read-share/v1".to_string(),
                limitations: vec!["synthetic cache limitation".to_string()],
            },
            write_share: RatioMetric {
                value_pct: Some(75.0),
                unit: "percent".to_string(),
                numerator: 3,
                denominator: 4,
                sample_count: 1,
                overflowed: false,
                availability: "available".to_string(),
                method_id: "cache/write-share/v1".to_string(),
                limitations: vec!["synthetic cache limitation".to_string()],
            },
            direct_compactions: 1,
            limitations: vec!["synthetic cache limitation".to_string()],
        },
        reconciliation: MetricReconciliation {
            status: "pass".to_string(),
            token_dimensions: sample_map("pass".to_string()),
            active_time_dimensions: sample_map("pass".to_string()),
            cost_domains: sample_map("pass".to_string()),
            limitations: vec!["synthetic reconciliation limitation".to_string()],
        },
    }
}

fn sample_insights() -> InsightReport {
    let prior_window = InsightWindow {
        start: "2024-12-31".to_string(),
        end: "2025-01-01".to_string(),
        timezone: "UTC".to_string(),
    };
    let current_window = InsightWindow {
        start: "2025-01-01".to_string(),
        end: "2025-01-02".to_string(),
        timezone: "UTC".to_string(),
    };
    InsightReport {
        version: "insights/v1".to_string(),
        families: vec![InsightFamilyStatus {
            family: "comparison".to_string(),
            availability: "available".to_string(),
            required_capabilities: vec!["token_usage".to_string()],
            sample_count: 2,
            minimum_sample_count: 2,
            limitations: vec!["synthetic insight limitation".to_string()],
        }],
        cards: vec![InsightCard {
            id: "comparison.output-tokens.v1".to_string(),
            version: "1".to_string(),
            family: "comparison".to_string(),
            class: "descriptive".to_string(),
            title: "Synthetic comparison".to_string(),
            finding: "Synthetic output rose in the observed window.".to_string(),
            metric_id: "tokens.output".to_string(),
            comparison: Some(InsightComparison {
                baseline_fact_id: "comparison.output.prior".to_string(),
                current_fact_id: "comparison.output.current".to_string(),
                baseline_value: "1".to_string(),
                current_value: "2".to_string(),
                absolute_delta: "+1".to_string(),
                relative_delta_pct: Some(100.0),
            }),
            window: current_window.clone(),
            sample_count: 2,
            minimum_sample_count: 2,
            method_id: "comparison/adjacent-output-tokens/v1".to_string(),
            availability: "available".to_string(),
            coverage: "complete".to_string(),
            confidence: "high".to_string(),
            supporting_facts: vec![
                InsightFact {
                    id: "comparison.output.prior".to_string(),
                    metric_id: "tokens.output".to_string(),
                    value: "1".to_string(),
                    unit: "tokens".to_string(),
                    method_id: "comparison/adjacent-output-tokens/v1".to_string(),
                    window: prior_window,
                    sample_count: 1,
                    coverage: "complete".to_string(),
                    source: "canonical transcript".to_string(),
                },
                InsightFact {
                    id: "comparison.output.current".to_string(),
                    metric_id: "tokens.output".to_string(),
                    value: "2".to_string(),
                    unit: "tokens".to_string(),
                    method_id: "comparison/adjacent-output-tokens/v1".to_string(),
                    window: current_window.clone(),
                    sample_count: 1,
                    coverage: "complete".to_string(),
                    source: "canonical transcript".to_string(),
                },
            ],
            limitations: vec!["synthetic insight limitation".to_string()],
            action: Some(InsightAction {
                experiment: "Repeat the observed workflow for one comparable window.".to_string(),
                alternative_explanations: vec!["The selected tasks may differ.".to_string()],
            }),
            privacy_class: "share".to_string(),
            renderer_priority: 10,
        }],
    }
}

fn sample_time_bucket() -> TimeBucket {
    TimeBucket {
        hour: 7,
        label: "07:00".to_string(),
        count: 2,
        share_pct: 50,
    }
}

fn sample_recommendation() -> Recommendation {
    Recommendation {
        severity: "info".to_string(),
        title: "Synthetic recommendation".to_string(),
        savings: "unavailable".to_string(),
        action: "Inspect the evidence".to_string(),
    }
}

fn sample_subagent() -> SubagentSummary {
    SubagentSummary {
        session_id: "session-subagent".to_string(),
        timestamp_start: Some("2025-01-02T03:04:05Z".to_string()),
        duration_minutes: 6,
        elapsed_seconds: 360,
        active_seconds: 300,
        total_tokens: 10,
        usage: sample_usage(),
        first_prompt: Some("synthetic prompt".to_string()),
        project_path: Some("/synthetic/project".to_string()),
        project_name: Some("synthetic-project".to_string()),
        parent_session_id: Some("session-main".to_string()),
    }
}

fn sample_session() -> SessionSummary {
    SessionSummary {
        session_id: "session-main".to_string(),
        project_hash: "project-hash".to_string(),
        project_path: Some("/synthetic/project".to_string()),
        project_name: "synthetic-project".to_string(),
        timestamp_start: Some("2025-01-02T03:04:05Z".to_string()),
        timestamp_end: Some("2025-01-02T03:10:05Z".to_string()),
        duration_minutes: 6,
        elapsed_seconds: 360,
        active_seconds: 300,
        inclusive_active_seconds: 360,
        usage: sample_usage(),
        model_totals: sample_map(sample_usage()),
        total_tokens: 10,
        cost_usd: 0.25,
        prompt_count: 1,
        tool_message_count: 1,
        first_prompt: Some("synthetic prompt".to_string()),
        prompts: vec![SessionPrompt {
            text: "synthetic prompt".to_string(),
            timestamp: Some("2025-01-02T03:04:05Z".to_string()),
            entrypoint: Some("cli".to_string()),
        }],
        subagents: vec![sample_subagent()],
    }
}

fn sample_daily_cost() -> DailyCost {
    DailyCost {
        date: "2025-01-02".to_string(),
        cost: 0.25,
        output_tokens: 2,
        cache_read_tokens: 4,
        cache_output_ratio: 200,
        message_count: 1,
        session_count: 1,
        models: vec![ModelCostBreakdown {
            model: "claude-synthetic".to_string(),
            cost: 0.25,
            tokens: CostTokens {
                input: 1,
                output: 2,
                cache_read: 4,
                cache_write: 3,
            },
        }],
    }
}

fn sample_report() -> Report {
    let session = sample_session();
    let subagent = sample_subagent();
    let daily_cost = sample_daily_cost();
    let recommendation = sample_recommendation();
    Report {
        schema_version: "ccwrapped.report/v2".to_string(),
        generated_at: "2025-01-02T03:10:05Z".to_string(),
        year: 2025,
        data_coverage: DataCoverage {
            selected_period: "2025".to_string(),
            timezone: "UTC".to_string(),
            earliest_observed_at: Some("2025-01-02T03:04:05Z".to_string()),
            latest_observed_at: Some("2025-01-02T03:10:05Z".to_string()),
            observed_day_span: 1,
            source_root_count: 1,
            files_discovered: 1,
            accepted_records: 4,
            canonical_records: 1,
            classified_records: 9,
            malformed_records: 1,
            unsupported_records: 1,
            unknown_records: 1,
            unknown_fields: 1,
            filtered_records: 1,
            redacted_fields: 3,
            duplicate_records: 1,
            skipped_records: 1,
            resolved_overlap_records: 1,
            unresolved_overlap_records: 1,
            authority_excluded_records: 1,
            record_count_invariant: "classifiedRecords = acceptedRecords + malformedRecords + unsupportedRecords + filteredRecords + skippedRecords + duplicateRecords; unknown/redacted/overlap counts are orthogonal".to_string(),
            completeness: "partial".to_string(),
            retention_caveat: "synthetic retention caveat".to_string(),
            cost_coverage: "source-recorded-estimate".to_string(),
            privacy_profile: "standard".to_string(),
            authority_policy_version: "authority/v1".to_string(),
            capabilities: BTreeMap::from([("token_usage".to_string(), "available".to_string())]),
            sources: vec![SourceCoverage {
                alias: "transcript-1".to_string(),
                kind: "transcript".to_string(),
                selection: "explicit-projects".to_string(),
                files_discovered: 1,
                accepted_records: 4,
                classified_records: 9,
                malformed_records: 1,
                unsupported_records: 1,
                unknown_records: 1,
                unknown_fields: 1,
                filtered_records: 1,
                redacted_fields: 3,
                duplicate_records: 1,
                skipped_records: 1,
                earliest_observed_at: Some("2025-01-02T03:04:05Z".to_string()),
                latest_observed_at: Some("2025-01-02T03:10:05Z".to_string()),
                capabilities: BTreeMap::from([(
                    "token_usage".to_string(),
                    "available".to_string(),
                )]),
                completeness: "partial".to_string(),
                adapter_version: "claude-transcript/v1".to_string(),
                producer_contract: None,
                producer_verification: None,
            }],
            warnings: vec![IngestionWarning {
                code: "W_SYNTHETIC".to_string(),
                message: "synthetic warning".to_string(),
                source_alias: Some("transcript-1".to_string()),
            }],
            unknown_shapes: vec![UnknownShapeDiagnostic {
                source_alias: "transcript-1".to_string(),
                adapter_version: "claude-transcript/v1".to_string(),
                file_alias: "transcript-1-file-1".to_string(),
                record_index: 1,
                record_kind: "synthetic".to_string(),
                structural_fields: BTreeMap::from([("type".to_string(), "string".to_string())]),
                byte_count: 42,
            }],
        },
        methodology: sample_methodology(),
        canonical_metrics: sample_canonical_metrics(),
        insights: sample_insights(),
        cost_analysis: CostAnalysis {
            year: 2025,
            active_days: 1,
            total_cost: 0.25,
            avg_daily_cost: 0.25,
            median_daily_cost: 0.25,
            peak_day: Some(daily_cost.clone()),
            daily_costs: vec![daily_cost],
            model_costs: sample_map(0.25),
            sessions: SessionCostStats {
                total: 1,
                total_duration_minutes: 6,
                avg_duration_minutes: 6,
                longest_session_id: Some("session-main".to_string()),
                longest_session_project: Some("synthetic-project".to_string()),
                longest_session_minutes: 6,
            },
            totals: sample_usage(),
        },
        cache_health: CacheHealth {
            estimated_breaks: 1,
            reasons_ranked: vec![CacheReason {
                reason: "synthetic observation".to_string(),
                count: 1,
                percentage: 100,
            }],
            cache_hit_rate: 0.5,
            efficiency_ratio: 50,
            grade: CacheGrade {
                letter: "A".to_string(),
                color: "green".to_string(),
                label: "synthetic".to_string(),
                score: 90,
                signals: CacheSignals {
                    hit_rate: 50,
                    ratio: 200,
                    trend: 1,
                    breaks: 1,
                },
            },
            savings: CacheSavings {
                from_caching: 1,
                wasted_from_breaks: 2,
            },
            totals: sample_usage(),
        },
        anomalies: AnomalyReport {
            anomalies: vec![Anomaly {
                date: "2025-01-02".to_string(),
                cost: 0.25,
                z_score: 1.5,
                severity: "low".to_string(),
                anomaly_type: "synthetic".to_string(),
                avg_cost: 0.20,
                deviation: 0.05,
                cache_ratio_anomaly: true,
                cache_output_ratio: 200,
            }],
            has_anomalies: true,
            stats: AnomalyStats {
                mean: 0.20,
                std_dev: 0.05,
            },
            trend: "stable".to_string(),
        },
        inflection: Some(InflectionPoint {
            date: "2025-01-02".to_string(),
            before_ratio: 10,
            after_ratio: 20,
            multiplier: 2.0,
            direction: "up".to_string(),
            before_days: 2,
            after_days: 2,
            summary: "synthetic change".to_string(),
            secondary: Some(Box::new(InflectionPoint {
                date: "2025-01-03".to_string(),
                before_ratio: 20,
                after_ratio: 10,
                multiplier: 0.5,
                direction: "down".to_string(),
                before_days: 2,
                after_days: 2,
                summary: "synthetic secondary".to_string(),
                secondary: None,
            })),
        }),
        session_intel: SessionIntel {
            available: true,
            total_sessions: 1,
            total_minutes: 6,
            avg_duration: 6,
            median_duration: 6,
            p90_duration: 6,
            max_duration: 6,
            longest_session_project: Some("synthetic-project".to_string()),
            long_sessions: 1,
            long_session_pct: 100,
            avg_tool_messages_per_session: 1,
            avg_messages_per_session: 1,
            top_tools: vec![ToolCount {
                name: "SyntheticTool".to_string(),
                count: 1,
            }],
            peak_hours: vec![sample_time_bucket()],
            peak_overlap_pct: 50,
            hour_distribution: vec![1],
        },
        session_breakdown: SessionBreakdown {
            sessions: vec![session.clone()],
            costly_subagents: vec![subagent.clone()],
            total_subagent_sessions: 1,
            total_subagent_tokens: 10,
            total_elapsed_seconds: 360,
            total_active_seconds: 300,
        },
        model_routing: ModelRouting {
            available: true,
            method_id: "routing/model-tier-request-share/v1".to_string(),
            unit: "request-share".to_string(),
            observations: 10,
            opus_pct: 10,
            sonnet_pct: 80,
            haiku_pct: 10,
            other_pct: 0,
            unknown_pct: 0,
            estimated_savings: 0.10,
            subagent_pct: 20,
            diversity_score: 3,
            tier_costs: sample_map(0.25),
            total_cost: 0.25,
            busiest_hour: Some(sample_time_bucket()),
        },
        project_breakdown: vec![ProjectSummary {
            hash: "project-hash".to_string(),
            path: Some("/synthetic/project".to_string()),
            name: "synthetic-project".to_string(),
            input_tokens: 1,
            output_tokens: 2,
            cache_creation_tokens: 3,
            cache_read_tokens: 4,
            message_count: 1,
            session_count: 1,
            subagent_session_count: 1,
            active_seconds: 300,
            first_seen: Some("2025-01-02T03:04:05Z".to_string()),
            last_seen: Some("2025-01-02T03:10:05Z".to_string()),
        }],
        recommendations: vec![recommendation.clone()],
        wrapped_story: WrappedStory {
            summary: "synthetic summary".to_string(),
            hero: vec![HeroStat {
                label: "Synthetic".to_string(),
                value: "1".to_string(),
                note: "fixture".to_string(),
            }],
            highlights: vec![Highlight {
                eyebrow: "Synthetic".to_string(),
                title: "Highlight".to_string(),
                note: "fixture".to_string(),
            }],
            archetype: StoryCard {
                title: "Synthetic archetype".to_string(),
                note: "fixture".to_string(),
            },
            cache_mood: CacheMood {
                title: "Synthetic cache".to_string(),
                note: "fixture".to_string(),
            },
            momentum: StoryCard {
                title: "Synthetic momentum".to_string(),
                note: "fixture".to_string(),
            },
            power_hour: Some(sample_time_bucket()),
            favorite_weekday: Some(NamedCount {
                label: "Thursday".to_string(),
                count: 1,
            }),
            total_messages: 1,
            total_tokens: 10,
            average_messages_per_active_day: 1,
            longest_streak: 1,
            top_tool: Some(TopTool {
                name: "SyntheticTool".to_string(),
                count: 1,
            }),
            top_project: Some(TopProject {
                name: "synthetic-project".to_string(),
                path: Some("/synthetic/project".to_string()),
                share_pct: 100,
                session_count: 1,
                output_tokens: 2,
            }),
            biggest_session: Some(session.clone()),
            biggest_session_by_cost: Some(session.clone()),
            biggest_session_by_tokens: Some(session),
            biggest_subagent: Some(subagent),
            prompt_ratio: PromptRatio {
                human: 1,
                tool: 1,
                total: 2,
                human_pct: 50,
            },
            next_move: Some(recommendation),
            share_text: "synthetic share text".to_string(),
        },
    }
}

fn sample_assistant_entry() -> AssistantEntry {
    AssistantEntry {
        session_id: "session-main".to_string(),
        project_hash: "project-hash".to_string(),
        is_subagent: true,
        cwd: Some("/synthetic/project".to_string()),
        timestamp: "2025-01-02T03:04:05Z".to_string(),
        model: "claude-synthetic".to_string(),
        input_tokens: 1,
        output_tokens: 2,
        cache_creation_tokens: 3,
        cache_read_tokens: 4,
        cost_usd: 0.25,
        tool_names: vec!["SyntheticTool".to_string()],
    }
}

fn sample_daily_aggregate() -> DailyAggregate {
    DailyAggregate {
        date: "2025-01-02".to_string(),
        total_cost: 0.25,
        input_tokens: 1,
        output_tokens: 2,
        cache_creation_tokens: 3,
        cache_read_tokens: 4,
        message_count: 1,
        session_count: 1,
        active_seconds: 300,
        cache_output_ratio: 200,
        models: sample_map(ModelAggregate {
            input_tokens: 1,
            output_tokens: 2,
            cache_creation_tokens: 3,
            cache_read_tokens: 4,
            cost: 0.25,
            message_count: 1,
            active_seconds: 300,
        }),
    }
}

fn serialize_number_as_string<S>(value: &u64, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&value.to_string())
}

#[derive(Default, Serialize)]
struct SerdeProbe {
    #[serde(flatten)]
    flattened: BTreeMap<String, u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    optional: Option<String>,
    #[serde(serialize_with = "serialize_number_as_string")]
    custom: u64,
    values: Vec<u64>,
}

fn populated_probe() -> SerdeProbe {
    SerdeProbe {
        flattened: BTreeMap::from([("flattened".to_string(), 1)]),
        optional: Some("present".to_string()),
        custom: 7,
        values: vec![1],
    }
}

fn main() {
    capture::<TokenUsage>("ccwrapped::TokenUsage");
    capture::<ModelAggregate>("ccwrapped::ModelAggregate");
    capture::<AssistantEntry>("ccwrapped::AssistantEntry");
    capture::<DailyAggregate>("ccwrapped::DailyAggregate");
    capture::<ProjectSummary>("ccwrapped::ProjectSummary");
    capture::<SessionPrompt>("ccwrapped::SessionPrompt");
    capture::<SubagentSummary>("ccwrapped::SubagentSummary");
    capture::<SessionSummary>("ccwrapped::SessionSummary");
    capture::<SessionBreakdown>("ccwrapped::SessionBreakdown");
    capture::<MetricMethod>("ccwrapped::MetricMethod");
    capture::<PricingRegistryMetadata>("ccwrapped::PricingRegistryMetadata");
    capture::<PricingRegistryRecordMetadata>("ccwrapped::PricingRegistryRecordMetadata");
    capture::<MethodologyCatalog>("ccwrapped::MethodologyCatalog");
    capture::<TokenMetricValue>("ccwrapped::TokenMetricValue");
    capture::<TokenMetricSet>("ccwrapped::TokenMetricSet");
    capture::<NamedTokenMetricSet>("ccwrapped::NamedTokenMetricSet");
    capture::<CanonicalTokenMetrics>("ccwrapped::CanonicalTokenMetrics");
    capture::<DailyActiveTime>("ccwrapped::DailyActiveTime");
    capture::<NamedActiveTime>("ccwrapped::NamedActiveTime");
    capture::<ActiveTimeMetrics>("ccwrapped::ActiveTimeMetrics");
    capture::<CostMetricValue>("ccwrapped::CostMetricValue");
    capture::<ModelCostEvidence>("ccwrapped::ModelCostEvidence");
    capture::<CanonicalCostMetrics>("ccwrapped::CanonicalCostMetrics");
    capture::<RatioMetric>("ccwrapped::RatioMetric");
    capture::<CanonicalCacheMetrics>("ccwrapped::CanonicalCacheMetrics");
    capture::<MetricReconciliation>("ccwrapped::MetricReconciliation");
    capture::<CanonicalMetrics>("ccwrapped::CanonicalMetrics");
    capture::<InsightWindow>("ccwrapped::InsightWindow");
    capture::<InsightFact>("ccwrapped::InsightFact");
    capture::<InsightComparison>("ccwrapped::InsightComparison");
    capture::<InsightAction>("ccwrapped::InsightAction");
    capture::<InsightCard>("ccwrapped::InsightCard");
    capture::<InsightFamilyStatus>("ccwrapped::InsightFamilyStatus");
    capture::<InsightReport>("ccwrapped::InsightReport");
    capture::<CostTokens>("ccwrapped::CostTokens");
    capture::<ModelCostBreakdown>("ccwrapped::ModelCostBreakdown");
    capture::<DailyCost>("ccwrapped::DailyCost");
    capture::<SessionCostStats>("ccwrapped::SessionCostStats");
    capture::<CostAnalysis>("ccwrapped::CostAnalysis");
    capture::<CacheReason>("ccwrapped::CacheReason");
    capture::<CacheSignals>("ccwrapped::CacheSignals");
    capture::<CacheGrade>("ccwrapped::CacheGrade");
    capture::<CacheSavings>("ccwrapped::CacheSavings");
    capture::<CacheHealth>("ccwrapped::CacheHealth");
    capture::<Anomaly>("ccwrapped::Anomaly");
    capture::<AnomalyStats>("ccwrapped::AnomalyStats");
    capture::<AnomalyReport>("ccwrapped::AnomalyReport");
    capture::<TimeBucket>("ccwrapped::TimeBucket");
    capture::<ToolCount>("ccwrapped::ToolCount");
    capture::<SessionIntel>("ccwrapped::SessionIntel");
    capture::<ModelRouting>("ccwrapped::ModelRouting");
    capture::<InflectionPoint>("ccwrapped::InflectionPoint");
    capture::<Recommendation>("ccwrapped::Recommendation");
    capture::<HeroStat>("ccwrapped::HeroStat");
    capture::<StoryCard>("ccwrapped::StoryCard");
    capture::<Highlight>("ccwrapped::Highlight");
    capture::<NamedCount>("ccwrapped::NamedCount");
    capture::<CacheMood>("ccwrapped::CacheMood");
    capture::<PromptRatio>("ccwrapped::PromptRatio");
    capture::<TopTool>("ccwrapped::TopTool");
    capture::<TopProject>("ccwrapped::TopProject");
    capture::<WrappedStory>("ccwrapped::WrappedStory");
    capture::<IngestionWarning>("ccwrapped::IngestionWarning");
    capture::<UnknownShapeDiagnostic>("ccwrapped::UnknownShapeDiagnostic");
    capture::<SourceCoverage>("ccwrapped::SourceCoverage");
    capture::<DataCoverage>("ccwrapped::DataCoverage");
    capture::<Report>("ccwrapped::Report");

    capture_fixture("report-default", &Report::default());
    capture_fixture("report-populated", &sample_report());
    capture_fixture("assistant-entry-populated", &sample_assistant_entry());
    capture_fixture("daily-aggregate-populated", &sample_daily_aggregate());
    capture_fixture("serde-probe-default", &SerdeProbe::default());
    capture_fixture("serde-probe-populated", &populated_probe());
}
