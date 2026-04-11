use super::widgets::{label_value, pad, percentage_bar, ratio_bar, section_header, sparkline};
use crate::{
    format_currency, format_ratio, format_tokens, ranked_projects, trim_text, with_grouping, Report,
};
use termcolor::{Color, ColorSpec, WriteColor};

fn set(writer: &mut impl WriteColor, fg: Option<Color>, bold: bool, dimmed: bool) {
    let mut spec = ColorSpec::new();
    spec.set_fg(fg).set_bold(bold).set_dimmed(dimmed);
    let _ = writer.set_color(&spec);
}

// ── Header: archetype + hero stats + summary ────────────────────────────────

pub fn header(report: &Report, writer: &mut impl WriteColor, width: usize) {
    let wrapped = &report.wrapped_story;

    let grade_color = match report.cache_health.grade.letter.as_str() {
        "A" => Color::Green,
        "B" => Color::Cyan,
        "C" => Color::Yellow,
        _ => Color::Red,
    };

    set(writer, Some(Color::Green), false, true);
    let _ = writeln!(writer, "  CLAUDE CODE WRAPPED · {}", report.year);
    let _ = writeln!(writer);
    set(writer, Some(grade_color), true, false);
    let _ = write!(writer, "  {} ", report.cache_health.grade.letter);
    set(writer, Some(Color::White), true, false);
    let _ = writeln!(writer, "{}", wrapped.archetype.title);
    set(writer, None, false, true);
    let _ = writeln!(writer, "  {}", wrapped.summary);
    let _ = writeln!(writer);

    // Hero stats in a compact grid
    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Season stats", width));
    let _ = writeln!(writer);

    for hero in &wrapped.hero {
        set(writer, Some(Color::White), true, false);
        let _ = write!(writer, "  {:<18}", hero.label);
        set(writer, Some(Color::Green), true, false);
        let _ = write!(writer, "{:<18}", hero.value);
        set(writer, None, false, true);
        let _ = writeln!(writer, "{}", hero.note);
    }
    let _ = writeln!(writer);
}

// ── Activity: daily spend sparkline ─────────────────────────────────────────

pub fn activity(report: &Report, writer: &mut impl WriteColor, width: usize) {
    let daily = &report.cost_analysis.daily_costs;
    if daily.is_empty() {
        return;
    }

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Activity", width));
    let _ = writeln!(writer);

    let values: Vec<f64> = daily.iter().map(|d| d.cost).collect();
    let chart_width = (width - 4).min(daily.len());
    let chart = sparkline(&values, chart_width);

    set(writer, Some(Color::Green), false, false);
    let _ = writeln!(writer, "  {chart}");

    // Date range and peak
    set(writer, None, false, true);
    if let (Some(first), Some(last)) = (daily.first(), daily.last()) {
        let first_label = first.date.get(5..).unwrap_or(&first.date);
        let last_label = last.date.get(5..).unwrap_or(&last.date);
        let gap = chart_width
            .saturating_sub(first_label.len() + last_label.len())
            .max(2);
        let _ = write!(writer, "  {first_label}");
        let _ = write!(writer, "{}", " ".repeat(gap));
        let _ = writeln!(writer, "{last_label}");
    }

    if let Some(peak) = &report.cost_analysis.peak_day {
        set(writer, None, false, true);
        let _ = writeln!(
            writer,
            "  Peak: {} on {}",
            format_currency(peak.cost),
            peak.date
        );
    }
    let _ = writeln!(writer);
}

// ── Cache: grade, hit rate, ratio, savings ───────────────────────────────────

pub fn cache(report: &Report, writer: &mut impl WriteColor, width: usize) {
    let health = &report.cache_health;
    let grade_color = match health.grade.letter.as_str() {
        "A" => Color::Green,
        "B" => Color::Cyan,
        "C" => Color::Yellow,
        _ => Color::Red,
    };

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Cache health", width));
    let _ = writeln!(writer);

    set(writer, Some(grade_color), true, false);
    let _ = write!(writer, "  Grade {}", health.grade.letter);
    set(writer, None, false, true);
    let _ = writeln!(writer, "  {}", health.grade.label);

    let _ = writeln!(
        writer,
        "  {}",
        label_value(
            "Hit rate",
            &format!("{:.1}%", health.cache_hit_rate),
            width - 4
        )
    );
    let _ = writeln!(
        writer,
        "  {}",
        label_value(
            "Cache ratio",
            &format_ratio(health.efficiency_ratio),
            width - 4
        )
    );

    set(writer, Some(Color::Green), false, false);
    let _ = writeln!(
        writer,
        "  {}",
        label_value(
            "Saved from caching",
            &format!("+${}", health.savings.from_caching),
            width - 4
        )
    );
    set(writer, Some(Color::Red), false, false);
    let _ = writeln!(
        writer,
        "  {}",
        label_value(
            "Overhead from breaks",
            &format!("-${}", health.savings.wasted_from_breaks),
            width - 4
        )
    );
    let _ = writer.reset();
    let _ = writeln!(writer);
}

// ── Model mix + Projects (side-by-side at wide terminals) ───────────────────

pub fn model_mix_and_projects(report: &Report, writer: &mut impl WriteColor, width: usize) {
    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Model mix", width));
    let _ = writeln!(writer);

    let total_cost = report.cost_analysis.total_cost;
    let bar_width = 20.min(width / 4);

    for (model, cost) in &report.cost_analysis.model_costs {
        let share = if total_cost > 0.0 {
            (cost / total_cost) * 100.0
        } else {
            0.0
        };
        set(writer, Some(Color::White), true, false);
        let _ = write!(writer, "  {:<16}", model);
        set(writer, Some(Color::Green), false, false);
        let _ = write!(writer, "{} ", percentage_bar(share, bar_width));
        set(writer, None, false, true);
        let _ = writeln!(writer, "{} ({:.0}%)", format_currency(*cost), share);
    }
    let _ = writeln!(writer);

    // Projects
    let projects = ranked_projects(&report.project_breakdown);
    if projects.is_empty() {
        return;
    }

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Top projects", width));
    let _ = writeln!(writer);

    let max_tokens = projects.first().map(|p| p.output_tokens).unwrap_or(1);
    let bar_w = 16.min(width / 5);

    for project in projects.iter().take(8) {
        let pct = if max_tokens > 0 {
            (project.output_tokens as f64 / max_tokens as f64) * 100.0
        } else {
            0.0
        };
        set(writer, Some(Color::White), true, false);
        let _ = write!(writer, "  {:<20}", pad(&project.name, 20));
        set(writer, Some(Color::Green), false, false);
        let _ = write!(writer, "{} ", percentage_bar(pct, bar_w));
        set(writer, None, false, true);
        let _ = writeln!(
            writer,
            "{}  {} sessions",
            format_tokens(project.output_tokens),
            project.session_count
        );
    }
    let _ = writeln!(writer);
}

// ── Sessions + Subagents ────────────────────────────────────────────────────

pub fn sessions_and_subagents(report: &Report, writer: &mut impl WriteColor, width: usize) {
    if report.session_breakdown.sessions.is_empty() {
        return;
    }

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Costliest sessions", width));
    let _ = writeln!(writer);

    for session in report.session_breakdown.sessions.iter().take(6) {
        let date = session
            .timestamp_start
            .as_deref()
            .map(|v| &v[..v.len().min(10)])
            .unwrap_or("-");
        set(writer, Some(Color::White), true, false);
        let _ = write!(writer, "  {:<20}", pad(&session.project_name, 20));
        set(writer, Some(Color::Green), false, false);
        let _ = write!(writer, "{:>8}", format_tokens(session.total_tokens));
        set(writer, None, false, true);
        let _ = writeln!(writer, "  {date}");
    }
    let _ = writeln!(writer);

    // Subagents
    if report.session_breakdown.costly_subagents.is_empty() {
        return;
    }

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Subagent spikes", width));
    let _ = writeln!(writer);

    for sub in report.session_breakdown.costly_subagents.iter().take(5) {
        let date = sub
            .timestamp_start
            .as_deref()
            .map(|v| &v[..v.len().min(10)])
            .unwrap_or("-");
        let name = sub.project_name.as_deref().unwrap_or("Subagent");
        set(writer, Some(Color::White), true, false);
        let _ = write!(writer, "  {:<20}", pad(name, 20));
        set(writer, Some(Color::Green), false, false);
        let _ = write!(writer, "{:>8}", format_tokens(sub.total_tokens));
        set(writer, None, false, true);
        let _ = writeln!(writer, "  {date}");
        if let Some(prompt) = &sub.first_prompt {
            set(writer, None, false, true);
            let _ = writeln!(writer, "    {}", trim_text(prompt, 60));
        }
    }
    let _ = writeln!(writer);
}

// ── Human vs tool ratio + cache savings ─────────────────────────────────────

pub fn ratio_and_savings(report: &Report, writer: &mut impl WriteColor, width: usize) {
    let pr = &report.wrapped_story.prompt_ratio;
    if pr.total == 0 {
        return;
    }

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Human vs tool", width));
    let _ = writeln!(writer);

    let bar_width = 30.min(width / 2);
    let (human_bar, tool_bar) = ratio_bar(pr.human_pct as f64, bar_width);

    let _ = write!(writer, "  ");
    set(writer, Some(Color::Green), false, false);
    let _ = write!(writer, "{human_bar}");
    set(writer, None, false, true);
    let _ = writeln!(writer, "{tool_bar}");

    let _ = writeln!(
        writer,
        "  {} human ({}%)  {} tool ({}%)",
        with_grouping(pr.human as u64),
        pr.human_pct,
        with_grouping(pr.tool as u64),
        100u64.saturating_sub(pr.human_pct)
    );
    let _ = writeln!(writer);
}

// ── Highlights: power hour, top project, top tool, biggest session ──────────

pub fn highlights(report: &Report, writer: &mut impl WriteColor, width: usize) {
    let wrapped = &report.wrapped_story;

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Highlights", width));
    let _ = writeln!(writer);

    for highlight in wrapped.highlights.iter().take(6) {
        set(writer, Some(Color::Magenta), true, false);
        let _ = write!(writer, "  {:<22}", highlight.eyebrow.to_uppercase());
        set(writer, Some(Color::White), true, false);
        let _ = writeln!(writer, "{}", highlight.title);
        set(writer, None, false, true);
        let _ = writeln!(writer, "  {:<22}{}", "", highlight.note);
    }
    let _ = writeln!(writer);

    // Quick read extras
    set(writer, None, false, true);
    if let Some(weekday) = &wrapped.favorite_weekday {
        let _ = writeln!(writer, "  Busiest weekday: {}", weekday.label);
    }
    if let Some(tool) = &wrapped.top_tool {
        let _ = writeln!(writer, "  Most-called tool: {} ({})", tool.name, tool.count);
    }
    if wrapped.longest_streak > 1 {
        let _ = writeln!(writer, "  Longest streak: {} days", wrapped.longest_streak);
    }
    if report.session_breakdown.total_subagent_sessions > 0 {
        let _ = writeln!(
            writer,
            "  Subagent sessions: {}",
            report.session_breakdown.total_subagent_sessions
        );
    }
    let _ = writeln!(writer);
}

// ── Recommendations ─────────────────────────────────────────────────────────

pub fn recommendations(report: &Report, writer: &mut impl WriteColor, width: usize) {
    if report.recommendations.is_empty() {
        return;
    }

    set(writer, None, false, true);
    let _ = writeln!(writer, "{}", section_header("Recommendations", width));
    let _ = writeln!(writer);

    for rec in report.recommendations.iter().take(5) {
        let severity_color = match rec.severity.as_str() {
            "critical" => Color::Red,
            "warning" => Color::Yellow,
            "positive" => Color::Green,
            _ => Color::Cyan,
        };
        set(writer, Some(severity_color), true, false);
        let _ = write!(writer, "  {:>8}  ", rec.severity.to_uppercase());
        set(writer, Some(Color::White), true, false);
        let _ = writeln!(writer, "{}", rec.title);
        set(writer, None, false, true);
        let _ = writeln!(writer, "  {:>8}  {}", "", rec.action);
        let _ = writeln!(writer);
    }
}

// ── Trend / inflection ──────────────────────────────────────────────────────

pub fn trend(report: &Report, writer: &mut impl WriteColor) {
    let Some(inflection) = &report.inflection else {
        return;
    };
    set(writer, None, false, true);
    let _ = writeln!(writer, "  Trend: {}", inflection.summary);
    let _ = writeln!(writer);
}
