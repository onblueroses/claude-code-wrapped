use crate::{
    CostAnalysis, CostTokens, DailyAggregate, DailyCost, ModelCostBreakdown, SessionBreakdown,
    SessionCostStats, TokenUsage,
};
use std::collections::BTreeMap;

pub fn analyze_usage(
    year: i32,
    daily_from_jsonl: &[DailyAggregate],
    session_breakdown: &SessionBreakdown,
) -> CostAnalysis {
    let daily_costs = daily_from_jsonl
        .iter()
        .map(|day| {
            let mut day_cost = 0.0;
            let mut model_breakdowns = Vec::new();

            for (model_name, model_usage) in &day.models {
                let cost = calculate_cost(model_name, &model_usage.as_usage(), model_usage.cost);
                day_cost = (day_cost + cost).min(f64::MAX);
                model_breakdowns.push(ModelCostBreakdown {
                    model: model_name.clone(),
                    cost,
                    tokens: CostTokens {
                        input: model_usage.input_tokens,
                        output: model_usage.output_tokens,
                        cache_read: model_usage.cache_read_tokens,
                        cache_write: model_usage.cache_creation_tokens,
                    },
                });
            }

            DailyCost {
                date: day.date.clone(),
                cost: day_cost,
                output_tokens: day.output_tokens,
                cache_read_tokens: day.cache_read_tokens,
                cache_output_ratio: day.cache_output_ratio,
                message_count: day.message_count,
                session_count: day.session_count,
                models: model_breakdowns,
            }
        })
        .collect::<Vec<_>>();

    let active_days = daily_costs
        .iter()
        .filter(|day| day.message_count > 0)
        .count();
    let total_cost = daily_costs.iter().map(|day| day.cost).sum::<f64>();
    let avg_daily_cost = if active_days > 0 {
        total_cost / active_days as f64
    } else {
        0.0
    };
    let median_daily_cost = median(
        daily_costs
            .iter()
            .filter(|day| day.cost > 0.01)
            .map(|day| day.cost)
            .collect::<Vec<_>>(),
    );
    let peak_day = daily_costs
        .iter()
        .cloned()
        .max_by(|left, right| left.cost.total_cmp(&right.cost));

    let mut model_costs = BTreeMap::new();
    for day in &daily_costs {
        for model in &day.models {
            let total = model_costs
                .entry(clean_model_name(&model.model))
                .or_insert(0.0);
            *total = (*total + model.cost).min(f64::MAX);
        }
    }

    let totals = daily_from_jsonl
        .iter()
        .fold(TokenUsage::default(), |mut totals, day| {
            totals.input_tokens = totals.input_tokens.saturating_add(day.input_tokens);
            totals.output_tokens = totals.output_tokens.saturating_add(day.output_tokens);
            totals.cache_creation_tokens = totals
                .cache_creation_tokens
                .saturating_add(day.cache_creation_tokens);
            totals.cache_read_tokens = totals
                .cache_read_tokens
                .saturating_add(day.cache_read_tokens);
            totals
        });

    let total_duration_minutes = session_breakdown
        .sessions
        .iter()
        .map(|session| session.duration_minutes)
        .fold(0u64, u64::saturating_add);
    let longest_session = session_breakdown
        .sessions
        .iter()
        .max_by_key(|session| session.duration_minutes);

    CostAnalysis {
        year,
        active_days,
        total_cost,
        avg_daily_cost,
        median_daily_cost,
        peak_day,
        daily_costs,
        model_costs,
        sessions: SessionCostStats {
            total: session_breakdown.sessions.len(),
            total_duration_minutes,
            avg_duration_minutes: if session_breakdown.sessions.is_empty() {
                0
            } else {
                total_duration_minutes / session_breakdown.sessions.len() as u64
            },
            longest_session_id: longest_session.map(|session| session.session_id.clone()),
            longest_session_project: longest_session.map(|session| session.project_name.clone()),
            longest_session_minutes: longest_session
                .map(|session| session.duration_minutes)
                .unwrap_or(0),
        },
        totals,
    }
}

pub fn clean_model_name(name: &str) -> String {
    let s = name.trim();
    let s = s.strip_prefix("anthropic/").unwrap_or(s);
    let s = s.strip_prefix("claude/").unwrap_or(s);
    let s = s.strip_prefix("claude-").unwrap_or(s);
    let collapsed = s
        .split('-')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();

    let cleaned = if collapsed.len() >= 3
        && collapsed[collapsed.len() - 1].len() == 8
        && collapsed[collapsed.len() - 1]
            .chars()
            .all(|ch| ch.is_ascii_digit())
    {
        collapsed[..collapsed.len() - 1].join("-")
    } else {
        collapsed.join("-")
    };

    if cleaned.starts_with("opus-") {
        cleaned.replacen("opus-", "Opus ", 1).replace('-', ".")
    } else if cleaned.starts_with("sonnet-") {
        cleaned.replacen("sonnet-", "Sonnet ", 1).replace('-', ".")
    } else if cleaned.starts_with("haiku-") {
        cleaned.replacen("haiku-", "Haiku ", 1).replace('-', ".")
    } else if cleaned.is_empty() {
        "Unknown".to_string()
    } else {
        let mut chars = cleaned.chars();
        match chars.next() {
            Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            None => "Unknown".to_string(),
        }
    }
}

fn calculate_cost(model_name: &str, tokens: &TokenUsage, recorded_cost_usd: f64) -> f64 {
    let _ = (model_name, tokens);
    recorded_cost_usd
}

fn median(mut values: Vec<f64>) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.sort_by(|left, right| left.total_cmp(right));
    let mid = values.len() / 2;
    if values.len() % 2 == 1 {
        values[mid]
    } else {
        (values[mid - 1] + values[mid]) / 2.0
    }
}
