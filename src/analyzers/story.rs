use crate::{
    format_currency, format_ratio, format_tokens, trim_text, weekday_from_date, AssistantEntry,
    CacheMood, HeroStat, Highlight, NamedCount, PromptRatio, Report, StoryCard, TopProject,
    TopTool, WrappedStory,
};
use std::collections::{BTreeMap, BTreeSet};

pub fn build_wrapped_story(report: &Report, entries: &[AssistantEntry]) -> WrappedStory {
    let daily_costs = &report.cost_analysis.daily_costs;
    let active_days = daily_costs
        .iter()
        .filter(|day| day.message_count > 0)
        .collect::<Vec<_>>();
    let total_messages = daily_costs
        .iter()
        .map(|day| day.message_count)
        .sum::<usize>();
    let totals = &report.cost_analysis.totals;
    let total_tokens = totals.total_tokens();
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
    let favorite_weekday = favorite_weekday(&active_days);
    let power_hour = crate::busiest_hour(entries);
    let top_tool = top_tool(entries);
    let top_project = top_project(&report.project_breakdown);
    let biggest_session = report.session_breakdown.sessions.first().cloned();
    let biggest_subagent = report.session_breakdown.costly_subagents.first().cloned();
    let prompt_ratio = prompt_ratio(&report.session_breakdown);
    let next_move = report.recommendations.first().cloned();
    let archetype = archetype(&report.model_routing, average_messages_per_active_day);
    let cache_mood = cache_mood(
        &report.cache_health.grade.letter,
        report.cache_health.efficiency_ratio,
    );
    let momentum = momentum(longest_streak, average_messages_per_active_day);

    let hero = vec![
        HeroStat {
            label: "Equivalent spend".to_string(),
            value: format_currency(report.cost_analysis.total_cost),
            note: format!(
                "{} active day{}",
                active_days.len(),
                if active_days.len() == 1 { "" } else { "s" }
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
            label: "Cache ratio".to_string(),
            value: format_ratio(report.cache_health.efficiency_ratio),
            note: format!("Grade {}", report.cache_health.grade.letter),
        },
        HeroStat {
            label: "Model mix".to_string(),
            value: model_mix_label(&report.model_routing),
            note: cache_mood.title.clone(),
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
    ];

    let mut highlights = Vec::new();
    highlights.push(Highlight {
        eyebrow: "Archetype".to_string(),
        title: archetype.title.clone(),
        note: archetype.note.clone(),
    });
    highlights.push(Highlight {
        eyebrow: "Power hour".to_string(),
        title: power_hour
            .as_ref()
            .map(|bucket| bucket.label.clone())
            .unwrap_or_else(|| "Time data still warming up".to_string()),
        note: power_hour
            .as_ref()
            .map(|bucket| {
                format!(
                    "{}% of assistant turns land around {}. {}",
                    bucket.share_pct,
                    bucket.label,
                    hour_mood(bucket.hour)
                )
            })
            .unwrap_or_else(|| "Run a few more sessions to get a reliable power hour.".to_string()),
    });
    highlights.push(Highlight {
        eyebrow: "Main character project".to_string(),
        title: top_project
            .as_ref()
            .map(|project| project.name.clone())
            .unwrap_or_else(|| "No dominant project yet".to_string()),
        note: top_project
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
    });
    highlights.push(if let Some(session) = &biggest_session {
        Highlight {
            eyebrow: "Biggest session".to_string(),
            title: format_tokens(session.total_tokens),
            note: session_note(session),
        }
    } else if let Some(peak_day) = &report.cost_analysis.peak_day {
        Highlight {
            eyebrow: "Peak day".to_string(),
            title: format_currency(peak_day.cost),
            note: format!("{} was your loudest day.", peak_day.date),
        }
    } else {
        Highlight {
            eyebrow: "Peak day".to_string(),
            title: "$0.00".to_string(),
            note: "Need more history for a peak-day read.".to_string(),
        }
    });
    highlights.push(if let Some(subagent) = &biggest_subagent {
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
            title: momentum.title.clone(),
            note: momentum.note.clone(),
        }
    });
    highlights.push(Highlight {
        eyebrow: "Next season".to_string(),
        title: next_move
            .as_ref()
            .map(|rec| rec.title.clone())
            .unwrap_or_else(|| "No obvious fixes right now".to_string()),
        note: next_move
            .as_ref()
            .map(|rec| rec.action.clone())
            .unwrap_or_else(|| {
                "Your setup looks stable. Keep the sessions clean and the cache warm.".to_string()
            }),
    });

    let summary = format!(
        "{}. {}. {} is your power hour.",
        archetype.title,
        cache_mood.title,
        power_hour
            .as_ref()
            .map(|bucket| bucket.label.clone())
            .unwrap_or_else(|| "Unknown".to_string())
    );

    WrappedStory {
        summary: summary.clone(),
        hero,
        highlights,
        archetype,
        cache_mood,
        momentum,
        power_hour,
        favorite_weekday,
        total_messages,
        total_tokens,
        average_messages_per_active_day,
        longest_streak,
        top_tool,
        top_project: top_project.clone(),
        biggest_session,
        biggest_subagent,
        prompt_ratio,
        next_move: next_move.clone(),
        share_text: if let Some(project) = top_project {
            format!(
                "{summary} {} carried {}% of your output.",
                project.name, project.share_pct
            )
        } else {
            summary
        },
    }
}

fn archetype(
    model_routing: &crate::ModelRouting,
    average_messages_per_active_day: u64,
) -> StoryCard {
    if model_routing.opus_pct >= 75 {
        StoryCard {
            title: "Precision Maximalist".to_string(),
            note: format!(
                "{}% of spend went through Opus. You prefer fewer, heavier swings over casual routing.",
                model_routing.opus_pct
            ),
        }
    } else if model_routing.sonnet_pct + model_routing.haiku_pct >= 45 {
        StoryCard {
            title: "Delegation Director".to_string(),
            note: format!(
                "{}% of spend lands on lighter models. You route work instead of brute-forcing it.",
                model_routing.sonnet_pct + model_routing.haiku_pct
            ),
        }
    } else if average_messages_per_active_day >= 120 {
        StoryCard {
            title: "Flow-State Builder".to_string(),
            note: "You keep a steady message cadence and favor momentum over ceremony.".to_string(),
        }
    } else {
        StoryCard {
            title: "Balanced Operator".to_string(),
            note: "You mix exploration and execution without leaning too hard on any one pattern."
                .to_string(),
        }
    }
}

fn cache_mood(letter: &str, ratio: u64) -> CacheMood {
    match letter {
        "A" | "B" => CacheMood {
            title: "Cache under control".to_string(),
            note: format!(
                "A {} cache ratio means the machine is mostly working with you.",
                format_ratio(ratio)
            ),
        },
        "C" => CacheMood {
            title: "Cache needs tuning".to_string(),
            note: "The setup is serviceable, but there is real slack left in session hygiene and routing."
                .to_string(),
        },
        _ => CacheMood {
            title: "Cache chaos energy".to_string(),
            note: "This run is leaking efficiency. Compact more aggressively and reset stale sessions faster."
                .to_string(),
        },
    }
}

fn momentum(longest_streak: u64, average_messages_per_active_day: u64) -> StoryCard {
    if longest_streak >= 5 {
        StoryCard {
            title: format!("{longest_streak}-day streak"),
            note: "You keep Claude Code warm across consecutive days, which is exactly what a wrapped report wants to see."
                .to_string(),
        }
    } else if average_messages_per_active_day >= 120 {
        StoryCard {
            title: "Burst-mode operator".to_string(),
            note: "You compress a lot of work into active days and keep the intensity high."
                .to_string(),
        }
    } else {
        StoryCard {
            title: "Measured tempo".to_string(),
            note: "You are selective about when you bring Claude Code in, which keeps the signal cleaner."
                .to_string(),
        }
    }
}

fn top_project(project_breakdown: &[crate::ProjectSummary]) -> Option<TopProject> {
    let pool = crate::ranked_projects(project_breakdown);
    let total_output = pool.iter().map(|p| p.output_tokens).sum::<u64>();
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
            *counts.entry(name.clone()).or_insert(0) += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|left, right| left.1.cmp(&right.1))
        .map(|(name, count)| TopTool { name, count })
}

fn favorite_weekday(active_days: &[&crate::DailyCost]) -> Option<NamedCount> {
    let mut counts = BTreeMap::new();
    for day in active_days {
        let Some(weekday) = weekday_from_date(&day.date) else {
            continue;
        };
        *counts.entry(weekday).or_insert(0usize) += day.message_count.max(1);
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
                current += 1;
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
        if timestamp_start.len() >= 10 {
            parts.push(timestamp_start[..10].to_string());
        }
    }
    if let Some(first_prompt) = &session.first_prompt {
        parts.push(trim_text(first_prompt, 86));
    }
    parts.join(" · ")
}

fn model_mix_label(model_routing: &crate::ModelRouting) -> String {
    if !model_routing.available {
        return "Model mix warming up".to_string();
    }
    if model_routing.opus_pct >= 75 {
        format!("{}% Opus", model_routing.opus_pct)
    } else if model_routing.sonnet_pct >= model_routing.opus_pct {
        format!("{}% Sonnet", model_routing.sonnet_pct)
    } else {
        format!(
            "{}% Opus / {}% Sonnet",
            model_routing.opus_pct, model_routing.sonnet_pct
        )
    }
}

fn prompt_ratio(session_breakdown: &crate::SessionBreakdown) -> PromptRatio {
    let human = session_breakdown
        .sessions
        .iter()
        .map(|session| session.prompt_count)
        .sum::<usize>();
    let tool = session_breakdown
        .sessions
        .iter()
        .map(|session| session.tool_message_count)
        .sum::<usize>();
    let total = human + tool;
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
