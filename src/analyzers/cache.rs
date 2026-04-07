use crate::{
    round_ratio, CacheGrade, CacheHealth, CacheReason, CacheSavings, CacheSignals, DailyAggregate,
    InflectionPoint, TokenUsage,
};
use chrono::NaiveDate;

pub fn analyze_cache_health(daily_from_jsonl: &[DailyAggregate]) -> CacheHealth {
    let totals = daily_from_jsonl
        .iter()
        .fold(TokenUsage::default(), |mut totals, day| {
            totals.input_tokens += day.input_tokens;
            totals.output_tokens += day.output_tokens;
            totals.cache_creation_tokens += day.cache_creation_tokens;
            totals.cache_read_tokens += day.cache_read_tokens;
            totals
        });

    let total_input_attempts =
        totals.cache_read_tokens + totals.cache_creation_tokens + totals.input_tokens;
    let cache_hit_rate = if total_input_attempts > 0 {
        totals.cache_read_tokens as f64 / total_input_attempts as f64 * 100.0
    } else {
        0.0
    };
    let estimated_breaks = if totals.cache_creation_tokens > 0 {
        ((totals.cache_creation_tokens as f64) / 300_000.0).round() as usize
    } else {
        0
    };
    let efficiency_ratio = round_ratio(totals.cache_read_tokens, totals.output_tokens);
    let grade = calculate_grade(
        efficiency_ratio,
        daily_from_jsonl,
        cache_hit_rate,
        estimated_breaks,
    );
    // Compute savings using the actual per-model token mix rather than a flat Sonnet rate.
    let (savings_from_cache, wasted_from_breaks) = model_weighted_savings(daily_from_jsonl);

    CacheHealth {
        estimated_breaks,
        reasons_ranked: vec![CacheReason {
            reason: "Unknown / Server-side".to_string(),
            count: estimated_breaks,
            percentage: 100,
        }]
        .into_iter()
        .filter(|reason| reason.count > 0)
        .collect(),
        cache_hit_rate: (cache_hit_rate * 10.0).round() / 10.0,
        efficiency_ratio,
        grade,
        savings: CacheSavings {
            from_caching: savings_from_cache.round() as i64,
            wasted_from_breaks: wasted_from_breaks.round() as i64,
        },
        totals,
    }
}

pub fn detect_inflection_points(daily_from_jsonl: &[DailyAggregate]) -> Option<InflectionPoint> {
    if daily_from_jsonl.len() < 5 {
        return None;
    }

    // Only days with real output can participate in the before/after efficiency windows.
    let active_days = daily_from_jsonl
        .iter()
        .filter(|day| day.output_tokens > 0)
        .cloned()
        .collect::<Vec<_>>();
    if active_days.len() < 5 {
        return None;
    }

    let mut worst_degradation: Option<InflectionPoint> = None;
    let mut worst_score = 0.0;
    let mut best_improvement: Option<InflectionPoint> = None;
    let mut best_score = 0.0;

    for index in 3..=active_days.len().saturating_sub(3) {
        let before = &active_days[index.saturating_sub(7)..index];
        let after = &active_days[index..active_days.len().min(index + 7)];
        let before_ratio = compute_ratio(before);
        let after_ratio = compute_ratio(after);

        if before_ratio == 0 || after_ratio == 0 {
            continue;
        }

        if after_ratio > before_ratio {
            let multiplier = after_ratio as f64 / before_ratio as f64;
            if multiplier >= 1.5 && multiplier > worst_score {
                worst_score = multiplier;
                worst_degradation = Some(build_result(
                    &active_days[index].date,
                    before_ratio,
                    after_ratio,
                    multiplier,
                    "worsened",
                    before.len(),
                    after.len(),
                ));
            }
        } else {
            let multiplier = before_ratio as f64 / after_ratio as f64;
            if multiplier >= 1.5 && multiplier > best_score {
                best_score = multiplier;
                best_improvement = Some(build_result(
                    &active_days[index].date,
                    before_ratio,
                    after_ratio,
                    multiplier,
                    "improved",
                    before.len(),
                    after.len(),
                ));
            }
        }
    }

    let mut primary = worst_degradation.or(best_improvement.clone())?;
    if primary.direction == "worsened" {
        primary.secondary = best_improvement.map(Box::new);
    }
    Some(primary)
}

fn calculate_grade(
    all_time_ratio: u64,
    daily_from_jsonl: &[DailyAggregate],
    cache_hit_rate: f64,
    estimated_breaks: usize,
) -> CacheGrade {
    // Grade is based on three signals. The efficiency ratio (cache_read / output)
    // is intentionally excluded: it scales with context length and agentic session
    // depth, so high values aren't a quality problem — they just mean long sessions.

    // Hit rate: what fraction of context came from cache.
    let hit_rate_score: u64 = if cache_hit_rate >= 85.0 {
        100
    } else if cache_hit_rate >= 70.0 {
        85
    } else if cache_hit_rate >= 50.0 {
        65
    } else if cache_hit_rate >= 30.0 {
        40
    } else {
        15
    };

    // Trend: is cache efficiency improving or declining over recent days?
    let mut trend_score: u64 = 70;
    if daily_from_jsonl.len() >= 7 {
        let split = daily_from_jsonl.len().saturating_sub(7);
        let recent = &daily_from_jsonl[split..];
        let older = &daily_from_jsonl[..split];
        let recent_ratio = compute_ratio(recent);
        let older_ratio = compute_ratio(older);
        if older_ratio > 0 {
            let change = recent_ratio as f64 / older_ratio as f64;
            trend_score = if change <= 0.5 {
                100
            } else if change <= 0.8 {
                85
            } else if change <= 1.2 {
                70
            } else if change <= 2.0 {
                40
            } else {
                10
            };
        }
    }

    // Break score: cache breaks (context resets) hurt efficiency.
    let break_score = if estimated_breaks == 0 {
        100u32
    } else {
        (100u32).saturating_sub((estimated_breaks as u32).min(10) * 10)
    };

    // hit_rate is the primary quality signal; trend and breaks are modifiers.
    let composite =
        (hit_rate_score as f64 * 0.60 + trend_score as f64 * 0.25 + break_score as f64 * 0.15)
            .round() as u64;

    // Cap grades when the efficiency ratio is pathologically high. Ratios above 1500:1
    // cap the grade at C; ratios above 2000:1 cap it at D. Normal agentic sessions
    // (e.g. 700:1) are not capped; only genuinely extreme values trigger this override.
    let composite = if all_time_ratio > 2000 {
        composite.min(49) // D at most
    } else if all_time_ratio > 1500 {
        composite.min(64) // C at most
    } else {
        composite
    };

    let (letter, color, label) = if composite >= 80 {
        ("A", "#7ec49a", "Excellent")
    } else if composite >= 65 {
        ("B", "#d4c5b0", "Good")
    } else if composite >= 50 {
        ("C", "#c4a96e", "Fair")
    } else if composite >= 35 {
        ("D", "#b08060", "Poor")
    } else {
        ("F", "#a07060", "Critical")
    };

    // ratio_score kept for JSON output / debugging; not used in grade calculation.
    let ratio_score: u64 = if all_time_ratio <= 200 {
        100
    } else if all_time_ratio <= 400 {
        85
    } else if all_time_ratio <= 600 {
        70
    } else if all_time_ratio <= 800 {
        55
    } else if all_time_ratio <= 1000 {
        40
    } else if all_time_ratio <= 1500 {
        25
    } else if all_time_ratio <= 2000 {
        15
    } else {
        5
    };

    CacheGrade {
        letter: letter.to_string(),
        color: color.to_string(),
        label: label.to_string(),
        score: composite,
        signals: CacheSignals {
            hit_rate: hit_rate_score,
            ratio: ratio_score,
            trend: trend_score,
            breaks: u64::from(break_score),
        },
    }
}

fn build_result(
    date: &str,
    before_ratio: u64,
    after_ratio: u64,
    multiplier: f64,
    direction: &str,
    before_days: usize,
    after_days: usize,
) -> InflectionPoint {
    let rounded = (multiplier * 10.0).round() / 10.0;
    let direction_label = if direction == "worsened" {
        "dropped"
    } else {
        "improved"
    };

    InflectionPoint {
        date: date.to_string(),
        before_ratio,
        after_ratio,
        multiplier: rounded,
        direction: direction.to_string(),
        before_days,
        after_days,
        summary: format!(
            "Your cache efficiency {direction_label} {rounded}x starting {}. Before: {}:1. After: {}:1.",
            format_date(date),
            before_ratio,
            after_ratio
        ),
        secondary: None,
    }
}

fn compute_ratio(days: &[DailyAggregate]) -> u64 {
    let total_output = days.iter().map(|day| day.output_tokens).sum::<u64>();
    let total_cache_read = days.iter().map(|day| day.cache_read_tokens).sum::<u64>();
    round_ratio(total_cache_read, total_output)
}

fn format_date(date: &str) -> String {
    NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map(|value| value.format("%b %-d").to_string())
        .unwrap_or_else(|_| date.to_string())
}

/// Returns `(savings_from_caching, overhead_from_breaks)` in dollars,
/// weighted by the actual model mix rather than a flat Sonnet rate.
fn model_weighted_savings(daily: &[DailyAggregate]) -> (f64, f64) {
    let mut savings = 0.0f64;
    let mut overhead = 0.0f64;
    for day in daily {
        for (model_name, model_data) in &day.models {
            let lower = model_name.to_lowercase();
            // (input_price, cache_read_price, cache_write_price) per million tokens
            let (input, cache_read, cache_write) = if lower.contains("haiku") {
                (1.0, 0.10, 1.25)
            } else if lower.contains("sonnet") {
                (3.0, 0.30, 3.75)
            } else {
                // Opus or unknown — default to Opus pricing
                (5.0, 0.50, 6.25)
            };
            savings += model_data.cache_read_tokens as f64 / 1_000_000.0 * (input - cache_read);
            overhead +=
                model_data.cache_creation_tokens as f64 / 1_000_000.0 * (cache_write - cache_read);
        }
    }
    (savings, overhead)
}
