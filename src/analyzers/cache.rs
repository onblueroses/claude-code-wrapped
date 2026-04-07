use crate::{
    CacheGrade, CacheHealth, CacheReason, CacheSavings, CacheSignals, DailyAggregate,
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
    // Approximate these cache savings using Sonnet pricing so they are not inflated for non-Opus users.
    let savings_from_cache = totals.cache_read_tokens as f64 / 1_000_000.0 * (3.0 - 0.30);
    let wasted_from_breaks = totals.cache_creation_tokens as f64 / 1_000_000.0 * (3.75 - 0.30);

    CacheHealth {
        total_cache_breaks: estimated_breaks,
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

    let sorted = daily_from_jsonl
        .iter()
        .filter(|day| day.output_tokens > 0)
        .cloned()
        .collect::<Vec<_>>();
    if sorted.len() < 5 {
        return None;
    }

    let mut worst_degradation: Option<InflectionPoint> = None;
    let mut worst_score = 0.0;
    let mut best_improvement: Option<InflectionPoint> = None;
    let mut best_score = 0.0;

    for index in 3..=sorted.len().saturating_sub(3) {
        let before = &sorted[index.saturating_sub(7)..index];
        let after = &sorted[index..sorted.len().min(index + 7)];
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
                    &sorted[index].date,
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
                    &sorted[index].date,
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
    let hit_rate_score: u64 = if cache_hit_rate >= 90.0 {
        100
    } else if cache_hit_rate >= 80.0 {
        85
    } else if cache_hit_rate >= 60.0 {
        65
    } else if cache_hit_rate >= 40.0 {
        40
    } else {
        15
    };

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

    let break_score = if estimated_breaks == 0 {
        100u32
    } else {
        (100u32).saturating_sub((estimated_breaks as u32).min(10) * 10)
    };
    let composite = (hit_rate_score as f64 * 0.15
        + ratio_score as f64 * 0.40
        + trend_score as f64 * 0.30
        + break_score as f64 * 0.15)
        .round() as u64;
    let min_signal = hit_rate_score
        .min(ratio_score)
        .min(trend_score)
        .min(u64::from(break_score));
    let capped = if min_signal <= 5 {
        composite.min(38)
    } else if min_signal <= 15 {
        composite.min(48)
    } else {
        composite
    };

    let (letter, color, label) = if capped >= 75 {
        ("A", "#10b981", "Excellent")
    } else if capped >= 60 {
        ("B", "#22d3ee", "Good")
    } else if capped >= 45 {
        ("C", "#f59e0b", "Fair")
    } else if capped >= 30 {
        ("D", "#f97316", "Poor")
    } else {
        ("F", "#ef4444", "Critical")
    };

    CacheGrade {
        letter: letter.to_string(),
        color: color.to_string(),
        label: label.to_string(),
        score: capped,
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

fn round_ratio(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        0
    } else {
        (numerator as f64 / denominator as f64).round() as u64
    }
}
