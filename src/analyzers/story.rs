use crate::{
    format_currency, format_tokens, trim_text, weekday_from_date, AssistantEntry, CacheMood,
    HeroStat, Highlight, NamedCount, PromptRatio, Report, StoryCard, TopProject, TopTool,
    WrappedStory,
};
use std::collections::{BTreeMap, BTreeSet};

struct StoryMetrics {
    active_day_count: usize,
    favorite_weekday: Option<NamedCount>,
    total_messages: usize,
    total_tokens: u64,
    average_messages_per_active_day: u64,
    longest_streak: u64,
    power_hour: Option<crate::TimeBucket>,
    top_tool: Option<TopTool>,
    top_project: Option<TopProject>,
    biggest_session: Option<crate::SessionSummary>,
    biggest_session_by_cost: Option<crate::SessionSummary>,
    biggest_session_by_tokens: Option<crate::SessionSummary>,
    biggest_subagent: Option<crate::SubagentSummary>,
    prompt_ratio: PromptRatio,
    next_move: Option<crate::Recommendation>,
    archetype: StoryCard,
    cache_mood: CacheMood,
    momentum: StoryCard,
}

pub fn build_wrapped_story(report: &Report, entries: &[AssistantEntry]) -> WrappedStory {
    let metrics = collect_story_metrics(report, entries);
    let hero = build_hero_stats(
        report,
        metrics.active_day_count,
        metrics.total_messages,
        metrics.average_messages_per_active_day,
        &metrics.prompt_ratio,
    );
    let highlights = build_highlights(report, &metrics);
    let power_hour = metrics
        .power_hour
        .as_ref()
        .map(|bucket| bucket.label.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    let summary = if !crate::canonical_evidence_is_limited(report) {
        format!(
            "{}. {}. {power_hour} is your power hour.",
            metrics.archetype.title, metrics.cache_mood.title
        )
    } else {
        format!(
            "{}. {} {power_hour} is the peak hour in the observed events.",
            metrics.archetype.title,
            crate::PARTIAL_USAGE_LIMITATION
        )
    };
    let share_text = if let Some(project) = metrics.top_project.as_ref() {
        format!(
            "{summary} {} carried {}% of your output.",
            project.name, project.share_pct
        )
    } else {
        summary.clone()
    };

    WrappedStory {
        summary: summary.clone(),
        hero,
        highlights,
        archetype: metrics.archetype,
        cache_mood: metrics.cache_mood,
        momentum: metrics.momentum,
        power_hour: metrics.power_hour,
        favorite_weekday: metrics.favorite_weekday,
        total_messages: metrics.total_messages,
        total_tokens: metrics.total_tokens,
        average_messages_per_active_day: metrics.average_messages_per_active_day,
        longest_streak: metrics.longest_streak,
        top_tool: metrics.top_tool,
        top_project: metrics.top_project,
        biggest_session: metrics.biggest_session,
        biggest_session_by_cost: metrics.biggest_session_by_cost,
        biggest_session_by_tokens: metrics.biggest_session_by_tokens,
        biggest_subagent: metrics.biggest_subagent,
        prompt_ratio: metrics.prompt_ratio,
        next_move: metrics.next_move,
        share_text,
    }
}

fn collect_story_metrics(report: &Report, entries: &[AssistantEntry]) -> StoryMetrics {
    let source_cost_available = report
        .canonical_metrics
        .cost
        .source_recorded
        .amount_usd
        .is_some();
    let usage_totals_available =
        crate::analytical_capability_available(report, "analysis_usage_totals");
    let output_available = crate::analytical_capability_available(report, "analysis_output_tokens");
    let biggest_session_by_cost = source_cost_available
        .then(|| report.session_breakdown.sessions.first().cloned())
        .flatten();
    let biggest_session_by_tokens = usage_totals_available
        .then(|| {
            let mut sessions = report.session_breakdown.sessions.clone();
            sessions.sort_by_key(|session| std::cmp::Reverse(session.total_tokens));
            sessions.first().cloned()
        })
        .flatten();

    let active_days = report
        .cost_analysis
        .daily_costs
        .iter()
        .filter(|day| day.message_count > 0)
        .collect::<Vec<_>>();
    let total_messages = report
        .cost_analysis
        .daily_costs
        .iter()
        .map(|day| day.message_count)
        .fold(0usize, usize::saturating_add);
    let average_messages_per_active_day = if active_days.is_empty() {
        0
    } else {
        (total_messages as f64 / active_days.len() as f64).round() as u64
    };
    let longest_streak = longest_active_streak(
        active_days
            .iter()
            .map(|day| day.date.clone())
            .collect::<Vec<_>>(),
    );

    StoryMetrics {
        active_day_count: active_days.len(),
        favorite_weekday: favorite_weekday(&active_days),
        total_messages,
        total_tokens: report.cost_analysis.totals.total_tokens(),
        average_messages_per_active_day,
        longest_streak,
        power_hour: report.model_routing.busiest_hour.clone(),
        top_tool: top_tool(entries),
        top_project: output_available
            .then(|| top_project(&report.project_breakdown))
            .flatten(),
        biggest_session: biggest_session_by_cost
            .clone()
            .or_else(|| biggest_session_by_tokens.clone()),
        biggest_session_by_cost,
        biggest_session_by_tokens,
        biggest_subagent: report.session_breakdown.costly_subagents.first().cloned(),
        prompt_ratio: prompt_ratio(&report.session_breakdown),
        next_move: report.recommendations.first().cloned(),
        archetype: entertainment_archetype(report),
        cache_mood: entertainment_cache_mood(report),
        momentum: entertainment_momentum(report),
    }
}

fn entertainment_archetype(report: &Report) -> StoryCard {
    report
        .insights
        .cards
        .iter()
        .find(|card| card.id == "entertainment.archetype.v1")
        .map(|card| StoryCard {
            title: card.title.clone(),
            note: card.finding.clone(),
        })
        .unwrap_or_else(|| StoryCard {
            title: "Entertainment · Not enough observed activity".to_string(),
            note: "No playful persona is assigned below the declared sample gate.".to_string(),
        })
}

fn entertainment_cache_mood(report: &Report) -> CacheMood {
    report
        .insights
        .cards
        .iter()
        .find(|card| card.id == "entertainment.cache-mood.v1")
        .map(|card| CacheMood {
            title: card.title.clone(),
            note: card.finding.clone(),
        })
        .unwrap_or_else(|| CacheMood {
            title: "Entertainment · Cache label unavailable".to_string(),
            note: "The cache-share or entertainment sample gate is unavailable.".to_string(),
        })
}

fn entertainment_momentum(report: &Report) -> StoryCard {
    report
        .insights
        .cards
        .iter()
        .find(|card| card.id == "entertainment.momentum.v1")
        .map(|card| StoryCard {
            title: card.title.clone(),
            note: card.finding.clone(),
        })
        .unwrap_or_else(|| StoryCard {
            title: "Entertainment · Momentum label unavailable".to_string(),
            note: "The trend or entertainment sample gate is unavailable.".to_string(),
        })
}

fn build_hero_stats(
    report: &Report,
    active_day_count: usize,
    total_messages: usize,
    average_messages_per_active_day: u64,
    prompt_ratio: &PromptRatio,
) -> Vec<HeroStat> {
    let local_cost = crate::canonical_local_cost(report);
    let cache_read = &report.canonical_metrics.cache.read_share;
    vec![
        HeroStat {
            label: "API-equivalent estimate".to_string(),
            value: local_cost.map_or_else(|| "Unavailable".to_string(), format_currency),
            note: format!(
                "{} active day{} · {}",
                active_day_count,
                if active_day_count == 1 { "" } else { "s" },
                report.canonical_metrics.cost.local_api_equivalent.method_id
            ),
        },
        HeroStat {
            label: "Messages".to_string(),
            value: crate::with_grouping(total_messages as u64),
            note: if average_messages_per_active_day > 0 {
                format!(
                    "{}/active day",
                    crate::with_grouping(average_messages_per_active_day)
                )
            } else {
                "Across all sessions".to_string()
            },
        },
        HeroStat {
            label: "Cache-read share".to_string(),
            value: crate::canonical_ratio_display(cache_read),
            note: cache_read.method_id.clone(),
        },
        HeroStat {
            label: "Model request mix".to_string(),
            value: model_mix_label(&report.model_routing),
            note: format!(
                "{} · {} request{}",
                report.model_routing.method_id,
                crate::with_grouping(report.model_routing.observations as u64),
                if report.model_routing.observations == 1 {
                    ""
                } else {
                    "s"
                }
            ),
        },
        HeroStat {
            label: "Human prompts".to_string(),
            value: format!("{}%", prompt_ratio.human_pct),
            note: format!(
                "{} human / {} tool",
                crate::with_grouping(prompt_ratio.human as u64),
                crate::with_grouping(prompt_ratio.tool as u64)
            ),
        },
    ]
}

fn build_highlights(report: &Report, metrics: &StoryMetrics) -> Vec<Highlight> {
    vec![
        Highlight {
            eyebrow: "Archetype".to_string(),
            title: metrics.archetype.title.clone(),
            note: metrics.archetype.note.clone(),
        },
        Highlight {
            eyebrow: "Power hour".to_string(),
            title: metrics
                .power_hour
                .as_ref()
                .map(|bucket| bucket.label.clone())
                .unwrap_or_else(|| "Time data still warming up".to_string()),
            note: metrics
                .power_hour
                .as_ref()
                .map(|bucket| {
                    format!(
                        "{}% of assistant turns land around {}. {}",
                        bucket.share_pct,
                        bucket.label,
                        hour_mood(bucket.hour)
                    )
                })
                .unwrap_or_else(|| {
                    "Run a few more sessions to get a reliable power hour.".to_string()
                }),
        },
        Highlight {
            eyebrow: "Main character project".to_string(),
            title: metrics
                .top_project
                .as_ref()
                .map(|project| project.name.clone())
                .unwrap_or_else(|| "No dominant project yet".to_string()),
            note: metrics
                .top_project
                .as_ref()
                .map(|project| {
                    format!(
                        "{}% of output tokens across {} session{}",
                        project.share_pct,
                        project.session_count,
                        if project.session_count == 1 { "" } else { "s" }
                    )
                })
                .unwrap_or_else(|| {
                    "Run a few more sessions to unlock project-level story cards.".to_string()
                }),
        },
        if let Some(session) = metrics.biggest_session.as_ref() {
            Highlight {
                eyebrow: "Biggest session".to_string(),
                title: format_tokens(session.total_tokens),
                note: session_note(session),
            }
        } else if let Some(peak_day) = report
            .canonical_metrics
            .active_time
            .days
            .iter()
            .max_by_key(|day| day.active_seconds)
        {
            Highlight {
                eyebrow: "Peak active day".to_string(),
                title: format!("{} active seconds", peak_day.active_seconds),
                note: format!(
                    "{} has the largest unioned active-time estimate.",
                    peak_day.date
                ),
            }
        } else {
            Highlight {
                eyebrow: "Peak active day".to_string(),
                title: "Unavailable".to_string(),
                note: "No active-time interval was observed in the selected period.".to_string(),
            }
        },
        if let Some(subagent) = metrics.biggest_subagent.as_ref() {
            Highlight {
                eyebrow: "Subagent cameo".to_string(),
                title: format_tokens(subagent.total_tokens),
                note: format!(
                    "{} leaned on background help. {}",
                    subagent
                        .project_name
                        .clone()
                        .unwrap_or_else(|| "A project".to_string()),
                    trim_text(
                        subagent
                            .first_prompt
                            .as_deref()
                            .unwrap_or("No prompt preview available."),
                        92
                    )
                ),
            }
        } else {
            Highlight {
                eyebrow: "Rhythm".to_string(),
                title: metrics.momentum.title.clone(),
                note: metrics.momentum.note.clone(),
            }
        },
        Highlight {
            eyebrow: "Next season".to_string(),
            title: metrics
                .next_move
                .as_ref()
                .map(|rec| rec.title.clone())
                .unwrap_or_else(|| "No obvious fixes right now".to_string()),
            note: metrics
                .next_move
                .as_ref()
                .map(|rec| rec.action.clone())
                .unwrap_or_else(|| {
                    "No evidence-backed experiment cleared the current coverage threshold."
                        .to_string()
                }),
        },
    ]
}

fn top_project(project_breakdown: &[crate::ProjectSummary]) -> Option<TopProject> {
    let pool = crate::ranked_projects(project_breakdown);
    let total_output = pool
        .iter()
        .map(|project| project.output_tokens)
        .fold(0u64, u64::saturating_add);
    let top = pool.first()?;
    let share_pct = if total_output > 0 {
        ((top.output_tokens as f64 / total_output as f64) * 100.0).round() as u64
    } else {
        0
    };
    Some(TopProject {
        name: top.name.clone(),
        path: top.path.clone(),
        share_pct,
        session_count: top.session_count,
        output_tokens: top.output_tokens,
    })
}

fn hour_mood(hour: u8) -> &'static str {
    if hour < 6 {
        "Night shift mode."
    } else if hour < 12 {
        "Morning shipping energy."
    } else if hour < 18 {
        "Afternoon builder hours."
    } else {
        "Evening closer energy."
    }
}

fn top_tool(entries: &[AssistantEntry]) -> Option<TopTool> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for entry in entries {
        for name in &entry.tool_names {
            let count = counts.entry(name.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(&left.0)))
        .map(|(name, count)| TopTool { name, count })
}

fn favorite_weekday(active_days: &[&crate::DailyCost]) -> Option<NamedCount> {
    let mut counts = BTreeMap::new();
    for day in active_days {
        let Some(weekday) = weekday_from_date(&day.date) else {
            continue;
        };
        let count = counts.entry(weekday).or_insert(0usize);
        *count = count.saturating_add(day.message_count.max(1));
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1))
        .map(|(label, count)| NamedCount { label, count })
}

fn longest_active_streak(dates: Vec<String>) -> u64 {
    if dates.is_empty() {
        return 0;
    }
    let unique = dates
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut best = 1u64;
    let mut current = 1u64;

    for pair in unique.windows(2) {
        let previous = chrono::NaiveDate::parse_from_str(&pair[0], "%Y-%m-%d").ok();
        let next = chrono::NaiveDate::parse_from_str(&pair[1], "%Y-%m-%d").ok();
        if let (Some(previous), Some(next)) = (previous, next) {
            if (next - previous).num_days() == 1 {
                current = current.saturating_add(1);
                best = best.max(current);
            } else {
                current = 1;
            }
        }
    }

    best
}

fn session_note(session: &crate::SessionSummary) -> String {
    let mut parts = Vec::new();
    if !session.project_name.is_empty() {
        parts.push(session.project_name.clone());
    }
    if let Some(timestamp_start) = &session.timestamp_start {
        if let Some(date) = crate::timestamp_date_key(timestamp_start) {
            parts.push(date);
        }
    }
    if let Some(first_prompt) = &session.first_prompt {
        parts.push(trim_text(first_prompt, 86));
    }
    parts.join(" · ")
}

fn model_mix_label(model_routing: &crate::ModelRouting) -> String {
    if !model_routing.available {
        return "Request mix unavailable".to_string();
    }
    [
        ("Opus", model_routing.opus_pct),
        ("Sonnet", model_routing.sonnet_pct),
        ("Haiku", model_routing.haiku_pct),
        ("other mapped", model_routing.other_pct),
        ("unknown", model_routing.unknown_pct),
    ]
    .into_iter()
    .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.0.cmp(left.0)))
    .map_or_else(
        || "Request mix unavailable".to_string(),
        |(label, share)| format!("{share}% {label}"),
    )
}

fn prompt_ratio(session_breakdown: &crate::SessionBreakdown) -> PromptRatio {
    let human = session_breakdown
        .sessions
        .iter()
        .map(|session| session.prompt_count)
        .fold(0usize, usize::saturating_add);
    let tool = session_breakdown
        .sessions
        .iter()
        .map(|session| session.tool_message_count)
        .fold(0usize, usize::saturating_add);
    let total = human.saturating_add(tool);
    let human_pct = if total > 0 {
        ((human as f64 / total as f64) * 100.0).round() as u64
    } else {
        0
    };
    PromptRatio {
        human,
        tool,
        total,
        human_pct,
    }
}

#[cfg(test)]
mod tests {
    use super::top_tool;
    use crate::AssistantEntry;

    #[test]
    fn story_top_tool_uses_the_same_lexical_tie_policy_as_session_intelligence() {
        let entries = [
            AssistantEntry {
                tool_names: vec!["Zulu".to_string()],
                ..AssistantEntry::default()
            },
            AssistantEntry {
                tool_names: vec!["Alpha".to_string()],
                ..AssistantEntry::default()
            },
        ];

        assert_eq!(
            top_tool(&entries).map(|tool| tool.name),
            Some("Alpha".to_string())
        );
    }
}
