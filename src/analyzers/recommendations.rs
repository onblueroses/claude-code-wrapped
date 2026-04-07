use crate::{
    AnomalyReport, CacheHealth, CostAnalysis, InflectionPoint, ModelRouting, ProjectSummary,
    Recommendation, SessionIntel,
};

struct RuleContext<'a> {
    cost_analysis: &'a CostAnalysis,
    cache_health: &'a CacheHealth,
    anomalies: &'a AnomalyReport,
    inflection: &'a Option<InflectionPoint>,
    session_intel: &'a SessionIntel,
    model_routing: &'a ModelRouting,
    project_breakdown: &'a [ProjectSummary],
}

type RuleFn = fn(&RuleContext<'_>) -> Option<Recommendation>;

const FALLBACK_RULE: RuleFn = rule_no_dominant_inefficiency;

const RULES: &[RuleFn] = &[
    rule_inflection_worsened,
    rule_inflection_improved,
    rule_opus_heavy,
    rule_long_sessions,
    rule_cache_ratio_severe,
    rule_cache_ratio_elevated,
    rule_cost_spikes,
    rule_dominant_project,
    rule_claudeignore,
    rule_prompt_cache_idle_gaps,
    rule_scoped_prompts,
    rule_throttled_hours,
    rule_cache_savings,
];

pub fn generate_recommendations(
    cost_analysis: &CostAnalysis,
    cache_health: &CacheHealth,
    anomalies: &AnomalyReport,
    inflection: &Option<InflectionPoint>,
    session_intel: &SessionIntel,
    model_routing: &ModelRouting,
    project_breakdown: &[ProjectSummary],
) -> Vec<Recommendation> {
    let context = RuleContext {
        cost_analysis,
        cache_health,
        anomalies,
        inflection,
        session_intel,
        model_routing,
        project_breakdown,
    };
    let mut recs = RULES
        .iter()
        .filter_map(|rule| rule(&context))
        .collect::<Vec<_>>();
    if recs.is_empty() {
        recs.extend(FALLBACK_RULE(&context));
    }
    recs.truncate(10);
    recs
}

fn rule_inflection_worsened(context: &RuleContext<'_>) -> Option<Recommendation> {
    let inflection = context.inflection.as_ref()?;
    (inflection.direction == "worsened" && inflection.multiplier >= 2.0).then(|| Recommendation {
        severity: "critical".to_string(),
        title: format!(
            "Cache efficiency dropped {:.1}x on {}",
            inflection.multiplier, inflection.date
        ),
        savings: "~40-60% usage reduction after fix".to_string(),
        action: "Audit session hygiene first: avoid reviving stale sessions, compact sooner, and update Claude Code if the regression matches a recent client upgrade.".to_string(),
    })
}

fn rule_inflection_improved(context: &RuleContext<'_>) -> Option<Recommendation> {
    let inflection = context.inflection.as_ref()?;
    (inflection.direction == "improved" && inflection.multiplier >= 2.0).then(|| Recommendation {
        severity: "positive".to_string(),
        title: format!(
            "Efficiency improved {:.1}x on {}",
            inflection.multiplier, inflection.date
        ),
        savings: "Already saving".to_string(),
        action: "Keep the workflow change that produced the improvement. That pattern is buying real cache efficiency.".to_string(),
    })
}

fn rule_opus_heavy(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.model_routing.available && context.model_routing.opus_pct > 80).then(|| {
        Recommendation {
            severity: "warning".to_string(),
            title: format!(
                "{}% of spend is Opus — delegate routine work downward",
                context.model_routing.opus_pct
            ),
            savings: format!("~{}% usage reduction", context.model_routing.opus_pct * 21 / 100),
            action: "Keep Opus for main-thread synthesis. Route file search, grep-heavy exploration, and routine edit batches to Sonnet or Haiku-backed subagents.".to_string(),
        }
    })
}

fn rule_long_sessions(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.session_intel.available && context.session_intel.avg_duration > 60).then(|| {
        Recommendation {
            severity: "warning".to_string(),
            title: format!(
                "Average session is {} minutes — split tasks earlier",
                context.session_intel.avg_duration
            ),
            savings: "~15-25% usage reduction".to_string(),
            action: format!(
                "Long sessions accumulate context tax. Compact or start a fresh session before the p90 point of {} minutes.",
                context.session_intel.p90_duration
            ),
        }
    })
}

fn rule_cache_ratio_severe(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.cache_health.efficiency_ratio > 1500).then(|| Recommendation {
        severity: "critical".to_string(),
        title: format!(
            "Cache ratio {}:1 is severely degraded",
            context.cache_health.efficiency_ratio
        ),
        savings: "~40-60% usage reduction".to_string(),
        action: "Restart old threads more aggressively, keep CLAUDE.md stable during a run, and avoid long idle gaps that force a full cache rebuild.".to_string(),
    })
}

fn rule_cache_ratio_elevated(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.cache_health.efficiency_ratio > 800 && context.cache_health.efficiency_ratio <= 1500)
        .then(|| Recommendation {
            severity: "info".to_string(),
            title: format!(
                "Cache ratio {}:1 is elevated",
                context.cache_health.efficiency_ratio
            ),
            savings: "~5-10% with optimization".to_string(),
            action: "Compact earlier, trim repeated boilerplate prompts, and prefer fresh sessions over resuming deeply stale ones.".to_string(),
        })
}

fn rule_cost_spikes(context: &RuleContext<'_>) -> Option<Recommendation> {
    let spikes = context
        .anomalies
        .anomalies
        .iter()
        .filter(|anomaly| anomaly.anomaly_type == "spike")
        .collect::<Vec<_>>();
    let worst = spikes.first()?;
    Some(Recommendation {
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
    })
}

fn rule_dominant_project(context: &RuleContext<'_>) -> Option<Recommendation> {
    if context.project_breakdown.len() <= 1 {
        return None;
    }

    let pool = crate::ranked_projects(context.project_breakdown);
    let top = pool.first()?;
    let total_output = pool
        .iter()
        .map(|project| project.output_tokens)
        .sum::<u64>();
    if total_output == 0 {
        return None;
    }

    let share = ((top.output_tokens as f64 / total_output as f64) * 100.0).round() as u64;
    (share > 30).then(|| Recommendation {
        severity: "info".to_string(),
        title: format!("\"{}\" drives {}% of output", top.name, share),
        savings: "Focus optimization here first".to_string(),
        action: format!(
            "{} is the dominant project this season. Review whether its workflows really need the current model mix and session length.",
            top.name
        ),
    })
}

fn rule_claudeignore(_: &RuleContext<'_>) -> Option<Recommendation> {
    Some(Recommendation {
        severity: "info".to_string(),
        title: "Create a .claudeignore for build artifacts".to_string(),
        savings: "~5-10% per context load".to_string(),
        action: "Exclude `node_modules/`, `dist/`, lockfiles, generated assets, and other large junk so each context load scans less irrelevant material.".to_string(),
    })
}

fn rule_prompt_cache_idle_gaps(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.cache_health.efficiency_ratio > 500).then(|| Recommendation {
        severity: "info".to_string(),
        title: "Idle gaps force prompt-cache rebuilds".to_string(),
        savings: "~10-30% usage reduction".to_string(),
        action: "Anthropic cache state expires quickly. After a break, starting a fresh task can be cheaper than resuming a bloated stale thread.".to_string(),
    })
}

fn rule_scoped_prompts(_: &RuleContext<'_>) -> Option<Recommendation> {
    Some(Recommendation {
        severity: "info".to_string(),
        title: "Use sharply scoped prompts".to_string(),
        savings: "~20-40% usage reduction".to_string(),
        action: "Point Claude at the exact file, function, and failure mode instead of sending broad fix-everything prompts that trigger full-repo exploration.".to_string(),
    })
}

fn rule_throttled_hours(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.session_intel.available && context.session_intel.peak_overlap_pct > 40).then(|| {
        Recommendation {
            severity: "info".to_string(),
            title: format!(
                "{}% of work lands during throttled hours",
                context.session_intel.peak_overlap_pct
            ),
            savings: "~30% longer heavy-work windows".to_string(),
            action: "Try to schedule focused work during your peak hours (local time).".to_string(),
        }
    })
}

fn rule_cache_savings(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.cache_health.savings.from_caching > 100).then(|| Recommendation {
        severity: "positive".to_string(),
        title: format!(
            "Caching saved about ${}",
            context.cache_health.savings.from_caching
        ),
        savings: "Working as intended".to_string(),
        action: "The cache is buying meaningful efficiency. Preserve that by avoiding unnecessary prompt, tool-schema, and session-shape churn.".to_string(),
    })
}

fn rule_no_dominant_inefficiency(context: &RuleContext<'_>) -> Option<Recommendation> {
    (context.cost_analysis.total_cost > 0.0).then(|| Recommendation {
        severity: "positive".to_string(),
        title: "No dominant inefficiency showed up".to_string(),
        savings: "Stable season".to_string(),
        action: "Keep the current workflow steady and watch for regressions in cache ratio or session length next run.".to_string(),
    })
}
