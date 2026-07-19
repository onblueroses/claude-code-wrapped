use crate::{
    CacheGrade, CacheHealth, CacheSavings, CacheSignals, DailyAggregate, InflectionPoint,
    TokenUsage,
};

pub fn analyze_cache_health(daily_from_jsonl: &[DailyAggregate]) -> CacheHealth {
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

    // This legacy public API cannot prove the eligibility denominator needed for a
    // cache share, nor can it distinguish cache creation from invalidation. Retain
    // only the directly observed token totals and make every derived field neutral.
    CacheHealth {
        estimated_breaks: 0,
        reasons_ranked: Vec::new(),
        cache_hit_rate: 0.0,
        efficiency_ratio: 0,
        grade: CacheGrade {
            letter: "N/A".to_string(),
            color: "#94a3b8".to_string(),
            label: "Unavailable — use canonical cache shares".to_string(),
            score: 0,
            signals: CacheSignals::default(),
        },
        savings: CacheSavings::default(),
        totals,
    }
}

pub fn detect_inflection_points(daily_from_jsonl: &[DailyAggregate]) -> Option<InflectionPoint> {
    let _ = daily_from_jsonl;
    None
}

#[cfg(test)]
mod tests {
    use super::{analyze_cache_health, detect_inflection_points};
    use crate::DailyAggregate;
    use std::collections::BTreeMap;

    fn day(
        date: &str,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
    ) -> DailyAggregate {
        DailyAggregate {
            date: date.to_string(),
            total_cost: 0.0,
            input_tokens,
            output_tokens,
            cache_creation_tokens,
            cache_read_tokens,
            message_count: 1,
            session_count: 1,
            active_seconds: 0,
            cache_output_ratio: crate::round_ratio(cache_read_tokens, output_tokens),
            models: BTreeMap::new(),
        }
    }

    #[test]
    fn analyze_cache_health_retains_totals_but_neutralizes_unsupported_derivations() {
        let daily = vec![
            day("2026-01-01", 50, 40, 0, 450),
            day("2026-01-02", 50, 60, 0, 450),
        ];

        let health = analyze_cache_health(&daily);

        assert_eq!(health.totals.input_tokens, 100);
        assert_eq!(health.totals.output_tokens, 100);
        assert_eq!(health.totals.cache_read_tokens, 900);
        assert_eq!(health.cache_hit_rate, 0.0);
        assert_eq!(health.efficiency_ratio, 0);
        assert_eq!(health.estimated_breaks, 0);
        assert!(health.reasons_ranked.is_empty());
        assert_eq!(health.grade.letter, "N/A");
        assert_eq!(health.savings.from_caching, 0);
        assert_eq!(health.savings.wasted_from_breaks, 0);
    }

    #[test]
    fn analyze_cache_health_never_turns_a_zero_or_low_share_into_a_grade() {
        let health = analyze_cache_health(&[day("2026-01-01", 800, 100, 0, 200)]);

        assert_eq!(health.cache_hit_rate, 0.0);
        assert_eq!(health.grade.letter, "N/A");
    }

    #[test]
    fn detect_inflection_points_requires_at_least_six_active_days() {
        let daily = (1..=5)
            .map(|day_index| day(&format!("2026-01-0{day_index}"), 10, 10, 0, 10))
            .collect::<Vec<_>>();

        assert!(detect_inflection_points(&daily).is_none());
    }

    #[test]
    fn detect_inflection_points_is_a_neutral_compatibility_adapter() {
        let mut daily = Vec::new();
        for day_index in 1..=4 {
            daily.push(day(&format!("2026-01-0{day_index}"), 10, 10, 0, 10));
        }
        for day_index in 5..=8 {
            daily.push(day(&format!("2026-01-0{day_index}"), 10, 10, 0, 30));
        }

        assert!(detect_inflection_points(&daily).is_none());
    }
}
