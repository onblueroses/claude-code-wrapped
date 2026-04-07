use crate::{
    format_hour, Anomaly, AnomalyReport, AnomalyStats, AssistantEntry, CostAnalysis, ModelRouting,
    SessionBreakdown, SessionIntel, TimeBucket, ToolCount,
};
use std::collections::{BTreeMap, HashMap};

const THROTTLED_HOURS: std::ops::RangeInclusive<usize> = 12..=18;

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
    let total_minutes = durations.iter().sum::<u64>();
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
            .sum::<u64>()
            / sessions.len() as u64
    };

    let mut assistant_messages_by_session = HashMap::new();
    let mut hour_distribution = vec![0usize; 24];
    let mut tool_totals: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        *assistant_messages_by_session
            .entry(entry.session_id.clone())
            .or_insert(0usize) += 1;
        if let Some(hour) = crate::timestamp_hour(&entry.timestamp) {
            hour_distribution[hour as usize] += 1;
        }
        for tool in &entry.tool_names {
            *tool_totals.entry(tool.clone()).or_insert(0) += 1;
        }
    }

    let avg_messages_per_session = if sessions.is_empty() {
        0
    } else {
        sessions
            .iter()
            .map(|session| {
                session.prompt_count
                    + session.tool_message_count
                    + assistant_messages_by_session
                        .get(&session.session_id)
                        .copied()
                        .unwrap_or(0)
            })
            .sum::<usize>() as u64
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
    peak_hours.sort_by(|left, right| right.count.cmp(&left.count));
    let total_hour_messages = hour_distribution.iter().sum::<usize>();
    for bucket in &mut peak_hours {
        if total_hour_messages > 0 {
            bucket.share_pct =
                ((bucket.count as f64 / total_hour_messages as f64) * 100.0).round() as u64;
        }
    }

    let peak_overlap_messages = hour_distribution[THROTTLED_HOURS.clone()]
        .iter()
        .sum::<usize>();
    let peak_overlap_pct = if total_hour_messages > 0 {
        ((peak_overlap_messages as f64 / total_hour_messages as f64) * 100.0).round() as u64
    } else {
        0
    };

    let mut top_tools = tool_totals
        .into_iter()
        .map(|(name, count)| ToolCount { name, count })
        .collect::<Vec<_>>();
    top_tools.sort_by(|left, right| right.count.cmp(&left.count));

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
    if total_cost < 0.01 {
        return ModelRouting {
            available: false,
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
        *tier_costs.entry(tier.to_string()).or_insert(0.0) += cost;
    }

    let opus_pct = ((tier_costs["opus"] / total_cost) * 100.0).round() as u64;
    let sonnet_pct = ((tier_costs["sonnet"] / total_cost) * 100.0).round() as u64;
    let haiku_pct = ((tier_costs["haiku"] / total_cost) * 100.0).round() as u64;

    let subagent_messages = entries.iter().filter(|entry| entry.is_subagent).count();
    let subagent_pct = if entries.is_empty() {
        0
    } else {
        ((subagent_messages as f64 / entries.len() as f64) * 100.0).round() as u64
    };

    let model_count = cost_analysis.model_costs.len();
    let diversity_score = if model_count >= 3 && opus_pct < 80 {
        90
    } else if model_count >= 2 && opus_pct < 90 {
        60
    } else if opus_pct > 95 {
        20
    } else {
        40
    };

    let opus_cost = tier_costs["opus"];
    let routable_to_sonnet = opus_cost * 0.4;
    let sonnet_equivalent_cost = routable_to_sonnet * 0.6;
    let estimated_savings = (routable_to_sonnet - sonnet_equivalent_cost).round() as u64;

    ModelRouting {
        available: true,
        opus_pct,
        sonnet_pct,
        haiku_pct,
        estimated_savings,
        subagent_pct,
        diversity_score,
        tier_costs,
        total_cost,
        busiest_hour,
    }
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
