use crate::{
    AnomalyReport, CacheHealth, CostAnalysis, InflectionPoint, ModelRouting, ProjectSummary,
    Recommendation, SessionIntel,
};

/// Retains the v0.2 public signature without manufacturing advice from aggregate proxies.
///
/// Standard report construction projects recommendations from typed
/// `recommendation/evidence-rule/v1` insight cards. These compatibility arguments do not
/// include direct event capabilities, proof references, coverage, or alternative
/// explanations, so the truthful result is empty.
pub fn generate_recommendations(
    cost_analysis: &CostAnalysis,
    cache_health: &CacheHealth,
    anomalies: &AnomalyReport,
    inflection: &Option<InflectionPoint>,
    session_intel: &SessionIntel,
    model_routing: &ModelRouting,
    project_breakdown: &[ProjectSummary],
) -> Vec<Recommendation> {
    let _ = (
        cost_analysis,
        cache_health,
        anomalies,
        inflection,
        session_intel,
        model_routing,
        project_breakdown,
    );
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::generate_recommendations;
    use crate::{
        AnomalyReport, CacheHealth, CostAnalysis, InflectionPoint, ModelRouting, SessionIntel,
    };

    #[test]
    fn compatibility_helper_does_not_emit_unproved_recommendations() {
        let recommendations = generate_recommendations(
            &CostAnalysis {
                total_cost: 25.0,
                ..CostAnalysis::default()
            },
            &CacheHealth {
                efficiency_ratio: u64::MAX,
                ..CacheHealth::default()
            },
            &AnomalyReport::default(),
            &Some(InflectionPoint {
                multiplier: 9.0,
                direction: "worsened".to_string(),
                ..InflectionPoint::default()
            }),
            &SessionIntel {
                available: true,
                avg_duration: u64::MAX,
                ..SessionIntel::default()
            },
            &ModelRouting {
                available: true,
                opus_pct: 100,
                ..ModelRouting::default()
            },
            &[],
        );

        assert!(recommendations.is_empty());
    }
}
