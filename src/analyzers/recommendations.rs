use crate::{
    AnomalyReport, CacheHealth, CostAnalysis, InflectionPoint, ModelRouting, ProjectSummary,
    Recommendation, SessionIntel,
};

pub fn generate_recommendations(
    cost_analysis: &CostAnalysis,
    cache_health: &CacheHealth,
    anomalies: &AnomalyReport,
    inflection: &Option<InflectionPoint>,
    session_intel: &SessionIntel,
    model_routing: &ModelRouting,
    project_breakdown: &[ProjectSummary],
) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    if let Some(inflection) = inflection {
        if inflection.direction == "worsened" && inflection.multiplier >= 2.0 {
            recs.push(Recommendation {
                severity: "critical".to_string(),
                title: format!(
                    "Cache efficiency dropped {:.1}x on {}",
                    inflection.multiplier, inflection.date
                ),
                savings: "~40-60% usage reduction after fix".to_string(),
                action: "Audit session hygiene first: avoid reviving stale sessions, compact sooner, and update Claude Code if the regression matches a recent client upgrade.".to_string(),
            });
        } else if inflection.direction == "improved" && inflection.multiplier >= 2.0 {
            recs.push(Recommendation {
                severity: "positive".to_string(),
                title: format!(
                    "Efficiency improved {:.1}x on {}",
                    inflection.multiplier, inflection.date
                ),
                savings: "Already saving".to_string(),
                action: "Keep the workflow change that produced the improvement. That pattern is buying real cache efficiency.".to_string(),
            });
        }
    }

    if model_routing.available && model_routing.opus_pct > 80 {
        recs.push(Recommendation {
            severity: "warning".to_string(),
            title: format!(
                "{}% of spend is Opus — delegate routine work downward",
                model_routing.opus_pct
            ),
            savings: format!("~{}% usage reduction", model_routing.opus_pct * 21 / 100),
            action: "Keep Opus for main-thread synthesis. Route file search, grep-heavy exploration, and routine edit batches to Sonnet or Haiku-backed subagents.".to_string(),
        });
    }

    if session_intel.available && session_intel.avg_duration > 60 {
        recs.push(Recommendation {
            severity: "warning".to_string(),
            title: format!(
                "Average session is {} minutes — split tasks earlier",
                session_intel.avg_duration
            ),
            savings: "~15-25% usage reduction".to_string(),
            action: format!(
                "Long sessions accumulate context tax. Compact or start a fresh session before the p90 point of {} minutes.",
                session_intel.p90_duration
            ),
        });
    }

    if cache_health.efficiency_ratio > 1500 {
        recs.push(Recommendation {
            severity: "critical".to_string(),
            title: format!(
                "Cache ratio {}:1 is severely degraded",
                cache_health.efficiency_ratio
            ),
            savings: "~40-60% usage reduction".to_string(),
            action: "Restart old threads more aggressively, keep CLAUDE.md stable during a run, and avoid long idle gaps that force a full cache rebuild.".to_string(),
        });
    } else if cache_health.efficiency_ratio > 800 {
        recs.push(Recommendation {
            severity: "info".to_string(),
            title: format!(
                "Cache ratio {}:1 is elevated",
                cache_health.efficiency_ratio
            ),
            savings: "~5-10% with optimization".to_string(),
            action: "Compact earlier, trim repeated boilerplate prompts, and prefer fresh sessions over resuming deeply stale ones.".to_string(),
        });
    }

    if anomalies.has_anomalies {
        let spikes = anomalies
            .anomalies
            .iter()
            .filter(|anomaly| anomaly.anomaly_type == "spike")
            .collect::<Vec<_>>();
        if let Some(worst) = spikes.first() {
            recs.push(Recommendation {
                severity: worst.severity.clone(),
                title: format!(
                    "{} cost spike{} — worst ${:.0} on {}",
                    spikes.len(),
                    if spikes.len() == 1 { "" } else { "s" },
                    worst.cost,
                    worst.date
                ),
                savings: "Preventable with monitoring".to_string(),
                action: "Watch the first turns of new sessions. If spend jumps sharply before useful output appears, restart immediately instead of feeding the runaway context.".to_string(),
            });
        }
    }

    if project_breakdown.len() > 1 {
        let mut sorted = project_breakdown.to_vec();
        sorted.sort_by(|left, right| right.output_tokens.cmp(&left.output_tokens));
        let pool = if sorted
            .iter()
            .any(|project| !project.name.is_empty() && project.name != "workspace root")
        {
            sorted
                .iter()
                .filter(|project| !project.name.is_empty() && project.name != "workspace root")
                .cloned()
                .collect::<Vec<_>>()
        } else {
            sorted.clone()
        };
        let total_output = pool
            .iter()
            .map(|project| project.output_tokens)
            .sum::<u64>();
        if let Some(top) = pool.first() {
            if total_output > 0 {
                let share =
                    ((top.output_tokens as f64 / total_output as f64) * 100.0).round() as u64;
                if share > 30 {
                    recs.push(Recommendation {
                        severity: "info".to_string(),
                        title: format!("\"{}\" drives {}% of output", top.name, share),
                        savings: "Focus optimization here first".to_string(),
                        action: format!(
                            "{} is the dominant project this season. Review whether its workflows really need the current model mix and session length.",
                            top.name
                        ),
                    });
                }
            }
        }
    }

    recs.push(Recommendation {
        severity: "info".to_string(),
        title: "Create a .claudeignore for build artifacts".to_string(),
        savings: "~5-10% per context load".to_string(),
        action: "Exclude `node_modules/`, `dist/`, lockfiles, generated assets, and other large junk so each context load scans less irrelevant material.".to_string(),
    });

    if cache_health.efficiency_ratio > 500 {
        recs.push(Recommendation {
            severity: "info".to_string(),
            title: "Idle gaps force prompt-cache rebuilds".to_string(),
            savings: "~10-30% usage reduction".to_string(),
            action: "Anthropic cache state expires quickly. After a break, starting a fresh task can be cheaper than resuming a bloated stale thread.".to_string(),
        });
    }

    recs.push(Recommendation {
        severity: "info".to_string(),
        title: "Use sharply scoped prompts".to_string(),
        savings: "~20-40% usage reduction".to_string(),
        action: "Point Claude at the exact file, function, and failure mode instead of sending broad fix-everything prompts that trigger full-repo exploration.".to_string(),
    });

    if session_intel.available && session_intel.peak_overlap_pct > 40 {
        recs.push(Recommendation {
            severity: "info".to_string(),
            title: format!(
                "{}% of work lands during throttled hours",
                session_intel.peak_overlap_pct
            ),
            savings: "~30% longer heavy-work windows".to_string(),
            action: "Try to schedule focused work during your peak hours (local time).".to_string(),
        });
    }

    if cache_health.savings.from_caching > 100 {
        recs.push(Recommendation {
            severity: "positive".to_string(),
            title: format!(
                "Caching saved about ${}",
                cache_health.savings.from_caching
            ),
            savings: "Working as intended".to_string(),
            action: "The cache is buying meaningful efficiency. Preserve that by avoiding unnecessary prompt, tool-schema, and session-shape churn.".to_string(),
        });
    }

    if recs.is_empty() && cost_analysis.total_cost > 0.0 {
        recs.push(Recommendation {
            severity: "positive".to_string(),
            title: "No dominant inefficiency showed up".to_string(),
            savings: "Stable season".to_string(),
            action: "Keep the current workflow steady and watch for regressions in cache ratio or session length next run.".to_string(),
        });
    }

    recs.into_iter().take(10).collect()
}
