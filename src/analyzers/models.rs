use crate::{
    format_hour, Anomaly, AnomalyReport, AnomalyStats, AssistantEntry, CostAnalysis, ModelRouting,
    SessionBreakdown, SessionIntel, TimeBucket, ToolCount,
};
use std::collections::{BTreeMap, HashMap};

const LEGACY_MIDDAY_HOURS: std::ops::RangeInclusive<usize> = 12..=18;

pub fn detect_anomalies(cost_analysis: &CostAnalysis) -> AnomalyReport {
    let daily_costs = &cost_analysis.daily_costs;
    if daily_costs.len() < 3 {
        return AnomalyReport {
            anomalies: Vec::new(),
            has_anomalies: false,
            stats: AnomalyStats::default(),
            trend: "stable".to_string(),
        };
    }

    let costs = daily_costs
        .iter()
        .filter(|day| day.cost > 0.01)
        .map(|day| day.cost)
        .collect::<Vec<_>>();
    if costs.len() < 3 {
        return AnomalyReport {
            anomalies: Vec::new(),
            has_anomalies: false,
            stats: AnomalyStats::default(),
            trend: "stable".to_string(),
        };
    }

    let mean = costs.iter().sum::<f64>() / costs.len() as f64;
    let variance = costs.iter().map(|cost| (cost - mean).powi(2)).sum::<f64>() / costs.len() as f64;
    let std_dev = variance.sqrt();

    let mut anomalies = daily_costs
        .iter()
        .filter(|day| day.cost > 0.01)
        .filter_map(|day| {
            let z_score = if std_dev > 0.0 {
                (day.cost - mean) / std_dev
            } else {
                0.0
            };
            if z_score.abs() <= 2.0 {
                return None;
            }

            Some(Anomaly {
                date: day.date.clone(),
                cost: day.cost,
                z_score: (z_score * 100.0).round() / 100.0,
                severity: if z_score.abs() > 3.0 {
                    "critical".to_string()
                } else {
                    "warning".to_string()
                },
                anomaly_type: if z_score > 0.0 {
                    "spike".to_string()
                } else {
                    "dip".to_string()
                },
                avg_cost: (mean * 100.0).round() / 100.0,
                deviation: ((day.cost - mean) * 100.0).round() / 100.0,
                cache_ratio_anomaly: day.cache_output_ratio > 2000,
                cache_output_ratio: day.cache_output_ratio,
            })
        })
        .collect::<Vec<_>>();

    anomalies.sort_by(|left, right| right.cost.total_cmp(&left.cost));

    AnomalyReport {
        has_anomalies: !anomalies.is_empty(),
        anomalies,
        stats: AnomalyStats {
            mean: (mean * 100.0).round() / 100.0,
            std_dev: (std_dev * 100.0).round() / 100.0,
        },
        trend: cost_trend(daily_costs),
    }
}

pub fn analyze_session_intelligence(
    session_breakdown: &SessionBreakdown,
    entries: &[AssistantEntry],
) -> SessionIntel {
    if session_breakdown.sessions.is_empty() {
        return SessionIntel {
            available: false,
            ..SessionIntel::default()
        };
    }

    let sessions = &session_breakdown.sessions;
    let durations = sessions
        .iter()
        .map(|session| session.duration_minutes)
        .collect::<Vec<_>>();
    let total_minutes = durations.iter().copied().fold(0u64, u64::saturating_add);
    let avg_duration = if durations.is_empty() {
        0
    } else {
        total_minutes / durations.len() as u64
    };
    let sorted = {
        let mut values = durations.clone();
        values.sort_unstable();
        values
    };
    let median_duration = percentile(&sorted, 0.50);
    let p90_duration = percentile(&sorted, 0.90);
    let max_duration = sorted.last().copied().unwrap_or(0);
    let longest_session_project = sessions
        .iter()
        .find(|session| session.duration_minutes == max_duration)
        .map(|session| session.project_name.clone());
    let long_sessions = sessions
        .iter()
        .filter(|session| session.duration_minutes > 60)
        .count();
    let long_session_pct = if sessions.is_empty() {
        0
    } else {
        ((long_sessions as f64 / sessions.len() as f64) * 100.0).round() as u64
    };
    let avg_tool_messages_per_session = if sessions.is_empty() {
        0
    } else {
        sessions
            .iter()
            .map(|session| session.tool_message_count as u64)
            .fold(0u64, u64::saturating_add)
            / sessions.len() as u64
    };

    let mut assistant_messages_by_session = HashMap::new();
    let mut hour_distribution = vec![0usize; 24];
    let mut tool_totals: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        let message_count = assistant_messages_by_session
            .entry(entry.session_id.clone())
            .or_insert(0usize);
        *message_count = message_count.saturating_add(1);
        if let Some(hour) = crate::timestamp_hour(&entry.timestamp) {
            hour_distribution[hour as usize] = hour_distribution[hour as usize].saturating_add(1);
        }
        for tool in &entry.tool_names {
            let count = tool_totals.entry(tool.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
    }

    let avg_messages_per_session = if sessions.is_empty() {
        0
    } else {
        sessions
            .iter()
            .map(|session| {
                session
                    .prompt_count
                    .saturating_add(session.tool_message_count)
                    .saturating_add(
                        assistant_messages_by_session
                            .get(&session.session_id)
                            .copied()
                            .unwrap_or(0),
                    )
            })
            .fold(0usize, usize::saturating_add) as u64
            / sessions.len() as u64
    };

    let mut peak_hours = hour_distribution
        .iter()
        .enumerate()
        .map(|(hour, count)| TimeBucket {
            hour: hour as u8,
            label: format_hour(hour as u8),
            count: *count,
            share_pct: 0,
        })
        .collect::<Vec<_>>();
    peak_hours.sort_by_key(|hour| std::cmp::Reverse(hour.count));
    let total_hour_messages = hour_distribution
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add);
    for bucket in &mut peak_hours {
        if total_hour_messages > 0 {
            bucket.share_pct =
                ((bucket.count as f64 / total_hour_messages as f64) * 100.0).round() as u64;
        }
    }

    let peak_overlap_messages = hour_distribution[LEGACY_MIDDAY_HOURS.clone()]
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add);
    let peak_overlap_pct = if total_hour_messages > 0 {
        ((peak_overlap_messages as f64 / total_hour_messages as f64) * 100.0).round() as u64
    } else {
        0
    };

    let mut top_tools = tool_totals
        .into_iter()
        .map(|(name, count)| ToolCount { name, count })
        .collect::<Vec<_>>();
    top_tools.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.name.cmp(&right.name))
    });

    SessionIntel {
        available: true,
        total_sessions: sessions.len(),
        total_minutes,
        avg_duration,
        median_duration,
        p90_duration,
        max_duration,
        longest_session_project,
        long_sessions,
        long_session_pct,
        avg_tool_messages_per_session,
        avg_messages_per_session,
        top_tools: top_tools.into_iter().take(8).collect(),
        peak_hours: peak_hours.into_iter().take(3).collect(),
        peak_overlap_pct,
        hour_distribution,
    }
}

pub fn analyze_model_routing(
    cost_analysis: &CostAnalysis,
    entries: &[AssistantEntry],
) -> ModelRouting {
    let total_cost = cost_analysis.model_costs.values().sum::<f64>();
    let busiest_hour = crate::busiest_hour(entries);
    if entries.is_empty() {
        return ModelRouting {
            available: false,
            method_id: "routing/model-tier-request-share/v1".to_string(),
            unit: "request-share".to_string(),
            total_cost,
            busiest_hour,
            ..ModelRouting::default()
        };
    }

    let mut tier_costs = BTreeMap::from([
        ("opus".to_string(), 0.0),
        ("sonnet".to_string(), 0.0),
        ("haiku".to_string(), 0.0),
        ("other".to_string(), 0.0),
    ]);

    for (name, cost) in &cost_analysis.model_costs {
        let lower = name.to_lowercase();
        let tier = if lower.contains("opus") {
            "opus"
        } else if lower.contains("sonnet") {
            "sonnet"
        } else if lower.contains("haiku") {
            "haiku"
        } else {
            "other"
        };
        let total = tier_costs.entry(tier.to_string()).or_insert(0.0);
        *total = (*total + cost).min(f64::MAX);
    }

    let mut request_counts = [0usize; 5];
    for entry in entries {
        let tier = match crate::ingestion::pricing::canonical_model(&entry.model) {
            Some(model) if model.contains("opus") => 0,
            Some(model) if model.contains("sonnet") => 1,
            Some(model) if model.contains("haiku") => 2,
            Some(_) => 3,
            None => 4,
        };
        request_counts[tier] = request_counts[tier].saturating_add(1);
    }
    let [opus_pct, sonnet_pct, haiku_pct, other_pct, unknown_pct] =
        apportioned_percentages(request_counts);

    let subagent_messages = entries.iter().filter(|entry| entry.is_subagent).count();
    let subagent_pct = if entries.is_empty() {
        0
    } else {
        ((subagent_messages as f64 / entries.len() as f64) * 100.0).round() as u64
    };

    let model_count = request_counts.iter().filter(|count| **count > 0).count();
    let diversity_score = if model_count >= 3 && opus_pct < 80 {
        90
    } else if model_count >= 2 && opus_pct < 90 {
        60
    } else if opus_pct > 95 {
        20
    } else {
        40
    };

    ModelRouting {
        available: true,
        method_id: "routing/model-tier-request-share/v1".to_string(),
        unit: "request-share".to_string(),
        observations: entries.len(),
        opus_pct,
        sonnet_pct,
        haiku_pct,
        other_pct,
        unknown_pct,
        estimated_savings: 0.0,
        subagent_pct,
        diversity_score,
        tier_costs,
        total_cost,
        busiest_hour,
    }
}

fn apportioned_percentages(counts: [usize; 5]) -> [u64; 5] {
    let total = counts.iter().copied().fold(0usize, usize::saturating_add);
    if total == 0 {
        return [0; 5];
    }
    let total = total as u128;
    let mut shares = [0u64; 5];
    let mut remainders = [(0u128, 0usize); 5];
    for (index, count) in counts.into_iter().enumerate() {
        let scaled = (count as u128).saturating_mul(100);
        shares[index] = u64::try_from(scaled / total).unwrap_or(100);
        remainders[index] = (scaled % total, index);
    }
    remainders.sort_by(|left, right| right.0.cmp(&left.0).then_with(|| left.1.cmp(&right.1)));
    let assigned = shares.iter().copied().sum::<u64>();
    for (_, index) in remainders
        .into_iter()
        .take(usize::try_from(100u64.saturating_sub(assigned)).unwrap_or(0))
    {
        shares[index] = shares[index].saturating_add(1);
    }
    shares
}

fn percentile(sorted: &[u64], pct: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let index = (sorted.len() as f64 * pct).floor() as usize;
    sorted[index.min(sorted.len() - 1)]
}

fn cost_trend(daily_costs: &[crate::DailyCost]) -> String {
    if daily_costs.len() < 7 {
        return "stable".to_string();
    }

    let split = daily_costs.len().saturating_sub(7);
    let recent = &daily_costs[split..];
    let older = &daily_costs[..split];
    let recent = recent
        .iter()
        .filter(|day| day.cost > 0.01)
        .collect::<Vec<_>>();
    let older = older
        .iter()
        .filter(|day| day.cost > 0.01)
        .collect::<Vec<_>>();
    if recent.is_empty() || older.is_empty() {
        return "stable".to_string();
    }

    let recent_avg = recent.iter().map(|day| day.cost).sum::<f64>() / recent.len() as f64;
    let older_avg = older.iter().map(|day| day.cost).sum::<f64>() / older.len() as f64;
    if older_avg <= 0.0 {
        return "stable".to_string();
    }
    let change = (recent_avg - older_avg) / older_avg * 100.0;
    if change > 50.0 {
        "rising_fast".to_string()
    } else if change > 20.0 {
        "rising".to_string()
    } else if change < -50.0 {
        "dropping_fast".to_string()
    } else if change < -20.0 {
        "dropping".to_string()
    } else {
        "stable".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{analyze_model_routing, analyze_session_intelligence};
    use crate::{AssistantEntry, CostAnalysis, SessionBreakdown, SessionSummary};
    use std::collections::BTreeMap;

    fn cost_analysis(model_costs: BTreeMap<String, f64>) -> CostAnalysis {
        CostAnalysis {
            model_costs,
            ..CostAnalysis::default()
        }
    }

    fn entry_with_model(session_id: &str, model: &str, is_subagent: bool) -> AssistantEntry {
        AssistantEntry {
            session_id: session_id.to_string(),
            project_hash: "project".to_string(),
            is_subagent,
            cwd: None,
            timestamp: "2026-01-01T12:00:00.000Z".to_string(),
            model: model.to_string(),
            input_tokens: 0,
            output_tokens: 0,
            cache_creation_tokens: 0,
            cache_read_tokens: 0,
            cost_usd: 0.0,
            tool_names: Vec::new(),
        }
    }

    fn entry(session_id: &str, is_subagent: bool) -> AssistantEntry {
        entry_with_model(session_id, "claude-opus-4-1", is_subagent)
    }

    #[test]
    fn analyze_model_routing_calculates_request_percentages_independently_of_cost() {
        let routing = analyze_model_routing(
            &cost_analysis(BTreeMap::from([
                ("Claude Opus".to_string(), 80.0),
                ("Claude Sonnet".to_string(), 20.0),
                ("Claude Haiku".to_string(), 0.0),
            ])),
            &[
                entry_with_model("session-1", "claude-opus-4-1", false),
                entry_with_model("session-2", "claude-sonnet-4-6", false),
            ],
        );

        assert!(routing.available);
        assert_eq!(routing.observations, 2);
        assert_eq!(routing.opus_pct, 50);
        assert_eq!(routing.sonnet_pct, 50);
        assert_eq!(routing.haiku_pct, 0);
        assert_eq!(
            routing.opus_pct
                + routing.sonnet_pct
                + routing.haiku_pct
                + routing.other_pct
                + routing.unknown_pct,
            100
        );
    }

    #[test]
    fn analyze_model_routing_does_not_fabricate_savings_from_request_mix() {
        let routing = analyze_model_routing(
            &cost_analysis(BTreeMap::from([
                ("Claude Opus".to_string(), 4.0),
                ("Claude Sonnet".to_string(), 1.0),
            ])),
            &[
                entry("session-1", false),
                entry("session-2", true),
                entry("session-3", false),
            ],
        );

        assert_eq!(routing.estimated_savings, 0.0);
        assert_eq!(routing.subagent_pct, 33);
    }

    #[test]
    fn session_top_tools_break_equal_count_ties_lexically() {
        let mut entries = Vec::new();
        for index in 0..64 {
            let mut entry = entry("session-1", false);
            entry.tool_names = vec![format!("tool-{index:03}")];
            entries.push(entry);
        }
        let breakdown = SessionBreakdown {
            sessions: vec![SessionSummary {
                session_id: "session-1".to_string(),
                ..SessionSummary::default()
            }],
            ..SessionBreakdown::default()
        };

        let intelligence = analyze_session_intelligence(&breakdown, &entries);
        assert_eq!(
            intelligence
                .top_tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            (0..8)
                .map(|index| format!("tool-{index:03}"))
                .collect::<Vec<_>>()
        );
    }
}
