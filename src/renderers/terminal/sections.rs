use super::widgets::{
    label_value, pad, percentage_bar, ratio_bar, section_header, sparkline, terminal_text,
};
use crate::{format_tokens, ranked_projects, trim_text, with_grouping, Report};
use std::io;
use termcolor::{Color, ColorSpec, WriteColor};

fn set(
    writer: &mut impl WriteColor,
    fg: Option<Color>,
    bold: bool,
    dimmed: bool,
) -> io::Result<()> {
    let mut spec = ColorSpec::new();
    spec.set_fg(fg).set_bold(bold).set_dimmed(dimmed);
    writer.set_color(&spec)
}

// ── Header: archetype + hero stats + summary ────────────────────────────────

pub fn header(report: &Report, writer: &mut impl WriteColor, width: usize) -> io::Result<()> {
    let wrapped = &report.wrapped_story;

    set(writer, Some(Color::Green), false, true)?;
    writeln!(
        writer,
        "  {} · {}",
        crate::experience_label(report).to_uppercase(),
        report.year
    )?;
    writeln!(writer)?;
    set(writer, Some(Color::Green), true, false)?;
    write!(writer, "  • ")?;
    set(writer, Some(Color::White), true, false)?;
    writeln!(writer, "{}", terminal_text(&wrapped.archetype.title))?;
    set(writer, None, false, true)?;
    writeln!(writer, "  {}", terminal_text(&wrapped.summary))?;
    writeln!(writer)?;

    writeln!(writer, "{}", section_header("Trust summary", width))?;
    writeln!(writer)?;
    for line in crate::trust_projection(report, "standard").lines() {
        writeln!(writer, "  {}", terminal_text(&line))?;
    }
    writeln!(writer)?;

    // Hero stats in a compact grid
    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Season stats", width))?;
    writeln!(writer)?;

    for hero in &wrapped.hero {
        set(writer, Some(Color::White), true, false)?;
        write!(writer, "  {:<18}", terminal_text(&hero.label))?;
        set(writer, Some(Color::Green), true, false)?;
        write!(writer, "{:<18}", terminal_text(&hero.value))?;
        set(writer, None, false, true)?;
        writeln!(writer, "{}", terminal_text(&hero.note))?;
    }
    writeln!(writer)?;
    Ok(())
}

// ── Activity: daily unioned active-time sparkline ────────────────────────────

pub fn activity(report: &Report, writer: &mut impl WriteColor, width: usize) -> io::Result<()> {
    let daily = &report.canonical_metrics.active_time.days;
    if daily.is_empty() {
        return Ok(());
    }

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Activity", width))?;
    writeln!(writer)?;

    let values: Vec<f64> = daily.iter().map(|day| day.active_seconds as f64).collect();
    let chart_width = (width - 4).min(daily.len());
    let chart = sparkline(&values, chart_width);

    set(writer, Some(Color::Green), false, false)?;
    writeln!(writer, "  {chart}")?;

    // Date range and peak
    set(writer, None, false, true)?;
    if let (Some(first), Some(last)) = (daily.first(), daily.last()) {
        let first_date = terminal_text(&first.date);
        let last_date = terminal_text(&last.date);
        let first_label = first_date.get(5..).unwrap_or(&first_date);
        let last_label = last_date.get(5..).unwrap_or(&last_date);
        let gap = chart_width
            .saturating_sub(first_label.len() + last_label.len())
            .max(2);
        write!(writer, "  {first_label}")?;
        write!(writer, "{}", " ".repeat(gap))?;
        writeln!(writer, "{last_label}")?;
    }

    if let Some(peak) = daily.iter().max_by_key(|day| day.active_seconds) {
        set(writer, None, false, true)?;
        writeln!(
            writer,
            "  Peak: {} active seconds on {}",
            peak.active_seconds,
            terminal_text(&peak.date)
        )?;
    }
    writeln!(writer)?;
    Ok(())
}

// ── Cache: descriptive canonical shares ──────────────────────────────────────

pub fn cache(report: &Report, writer: &mut impl WriteColor, width: usize) -> io::Result<()> {
    let cache = &report.canonical_metrics.cache;

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Cache evidence", width))?;
    writeln!(writer)?;

    writeln!(
        writer,
        "  {}",
        label_value(
            "Cache-read share",
            &crate::canonical_ratio_display(&cache.read_share),
            width - 4
        )
    )?;
    writeln!(
        writer,
        "  {}",
        label_value(
            "Cache-write share",
            &crate::canonical_ratio_display(&cache.write_share),
            width - 4
        )
    )?;
    writeln!(writer, "  Read method: {}", cache.read_share.method_id)?;
    writeln!(writer, "  Write method: {}", cache.write_share.method_id)?;
    writeln!(writer, "  Direct compactions: {}", cache.direct_compactions)?;
    writeln!(
        writer,
        "  Descriptive token shares; no cause or grade is inferred."
    )?;
    writeln!(writer)?;
    Ok(())
}

// ── Model request mix + Projects ─────────────────────────────────────────────

pub fn model_mix_and_projects(
    report: &Report,
    writer: &mut impl WriteColor,
    width: usize,
) -> io::Result<()> {
    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Model request mix", width))?;
    writeln!(writer)?;

    if report.model_routing.available {
        let bar_width = 20.min(width / 4);
        for row in crate::model_request_mix_rows(&report.model_routing) {
            set(writer, Some(Color::White), true, false)?;
            write!(writer, "  {:<16}", row.label)?;
            set(writer, Some(Color::Green), false, false)?;
            write!(
                writer,
                "{} ",
                percentage_bar(row.share_pct as f64, bar_width)
            )?;
            set(writer, None, false, true)?;
            writeln!(writer, "{}%", row.share_pct)?;
        }
        writeln!(
            writer,
            "  {} · {} observations",
            report.model_routing.method_id, report.model_routing.observations
        )?;
        writeln!(
            writer,
            "  Local cost coverage: {} · {} unpriced request{}",
            terminal_text(&report.canonical_metrics.cost.coverage),
            report.canonical_metrics.cost.unpriced_requests,
            if report.canonical_metrics.cost.unpriced_requests == 1 {
                ""
            } else {
                "s"
            }
        )?;
    } else {
        set(writer, None, false, true)?;
        writeln!(writer, "  No direct request observations.")?;
    }
    writeln!(writer)?;

    // Projects
    let projects = ranked_projects(&report.project_breakdown);
    if projects.is_empty() {
        return Ok(());
    }

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Top projects", width))?;
    writeln!(writer)?;

    let max_tokens = projects.first().map(|p| p.output_tokens).unwrap_or(1);
    let bar_w = 16.min(width / 5);

    for project in projects.iter().take(8) {
        let pct = if max_tokens > 0 {
            (project.output_tokens as f64 / max_tokens as f64) * 100.0
        } else {
            0.0
        };
        set(writer, Some(Color::White), true, false)?;
        let project_name = terminal_text(&project.name);
        write!(writer, "  {:<20}", pad(&project_name, 20))?;
        set(writer, Some(Color::Green), false, false)?;
        write!(writer, "{} ", percentage_bar(pct, bar_w))?;
        set(writer, None, false, true)?;
        writeln!(
            writer,
            "{}  {} sessions",
            format_tokens(project.output_tokens),
            project.session_count
        )?;
    }
    writeln!(writer)?;
    Ok(())
}

// ── Sessions + Subagents ────────────────────────────────────────────────────

pub fn sessions_and_subagents(
    report: &Report,
    writer: &mut impl WriteColor,
    width: usize,
) -> io::Result<()> {
    if report.session_breakdown.sessions.is_empty() {
        return Ok(());
    }

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Largest sessions", width))?;
    writeln!(writer)?;

    for session in report.session_breakdown.sessions.iter().take(6) {
        set(writer, Some(Color::White), true, false)?;
        let project_name = terminal_text(&session.project_name);
        write!(writer, "  {:<20}", pad(&project_name, 20))?;
        set(writer, Some(Color::Green), false, false)?;
        write!(writer, "{:>8}", format_tokens(session.total_tokens))?;
        set(writer, None, false, true)?;
        writeln!(
            writer,
            "  {}s active / {}s elapsed",
            session.active_seconds, session.elapsed_seconds
        )?;
    }
    writeln!(writer)?;

    // Subagents
    if report.session_breakdown.costly_subagents.is_empty() {
        return Ok(());
    }

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Subagent spikes", width))?;
    writeln!(writer)?;

    for sub in report.session_breakdown.costly_subagents.iter().take(5) {
        let name = terminal_text(sub.project_name.as_deref().unwrap_or("Subagent"));
        set(writer, Some(Color::White), true, false)?;
        write!(writer, "  {:<20}", pad(&name, 20))?;
        set(writer, Some(Color::Green), false, false)?;
        write!(writer, "{:>8}", format_tokens(sub.total_tokens))?;
        set(writer, None, false, true)?;
        writeln!(
            writer,
            "  {}s active / {}s elapsed",
            sub.active_seconds, sub.elapsed_seconds
        )?;
        if let Some(prompt) = &sub.first_prompt {
            set(writer, None, false, true)?;
            let prompt = trim_text(prompt, 60);
            writeln!(writer, "    {}", terminal_text(&prompt))?;
        }
    }
    writeln!(writer)?;
    Ok(())
}

// ── Human vs tool ratio + cache savings ─────────────────────────────────────

pub fn ratio_and_savings(
    report: &Report,
    writer: &mut impl WriteColor,
    width: usize,
) -> io::Result<()> {
    let pr = &report.wrapped_story.prompt_ratio;
    if pr.total == 0 {
        return Ok(());
    }

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Human vs tool", width))?;
    writeln!(writer)?;

    let bar_width = 30.min(width / 2);
    let (human_bar, tool_bar) = ratio_bar(pr.human_pct as f64, bar_width);

    write!(writer, "  ")?;
    set(writer, Some(Color::Green), false, false)?;
    write!(writer, "{human_bar}")?;
    set(writer, None, false, true)?;
    writeln!(writer, "{tool_bar}")?;

    writeln!(
        writer,
        "  {} human ({}%)  {} tool ({}%)",
        with_grouping(pr.human as u64),
        pr.human_pct,
        with_grouping(pr.tool as u64),
        100u64.saturating_sub(pr.human_pct)
    )?;
    writeln!(writer)?;
    Ok(())
}

pub fn method_facts(report: &Report, writer: &mut impl WriteColor, width: usize) -> io::Result<()> {
    set(writer, None, false, true)?;
    writeln!(
        writer,
        "{}",
        section_header("Canonical method facts", width)
    )?;
    writeln!(writer)?;
    for line in crate::canonical_fact_lines(report) {
        writeln!(writer, "  {}", terminal_text(&line))?;
    }
    writeln!(writer)?;
    Ok(())
}

// ── Highlights: power hour, top project, top tool, biggest session ──────────

pub fn highlights(report: &Report, writer: &mut impl WriteColor, width: usize) -> io::Result<()> {
    let wrapped = &report.wrapped_story;

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Highlights", width))?;
    writeln!(writer)?;

    for highlight in wrapped.highlights.iter().take(6) {
        set(writer, Some(Color::Magenta), true, false)?;
        let eyebrow = terminal_text(&highlight.eyebrow).to_uppercase();
        write!(writer, "  {:<22}", eyebrow)?;
        set(writer, Some(Color::White), true, false)?;
        writeln!(writer, "{}", terminal_text(&highlight.title))?;
        set(writer, None, false, true)?;
        writeln!(writer, "  {:<22}{}", "", terminal_text(&highlight.note))?;
    }
    writeln!(writer)?;

    // Quick read extras
    set(writer, None, false, true)?;
    if let Some(weekday) = &wrapped.favorite_weekday {
        writeln!(
            writer,
            "  Busiest weekday: {}",
            terminal_text(&weekday.label)
        )?;
    }
    if let Some(tool) = &wrapped.top_tool {
        writeln!(
            writer,
            "  Most-called tool: {} ({})",
            terminal_text(&tool.name),
            tool.count
        )?;
    }
    if wrapped.longest_streak > 1 {
        writeln!(writer, "  Longest streak: {} days", wrapped.longest_streak)?;
    }
    if report.session_breakdown.total_subagent_sessions > 0 {
        writeln!(
            writer,
            "  Subagent sessions: {}",
            report.session_breakdown.total_subagent_sessions
        )?;
    }
    writeln!(writer)?;
    Ok(())
}

// ── Recommendations ─────────────────────────────────────────────────────────

pub fn insights(report: &Report, writer: &mut impl WriteColor, width: usize) -> io::Result<()> {
    let cards = super::super::insights::cards(report, false).collect::<Vec<_>>();
    if cards.is_empty() && report.insights.families.is_empty() {
        return Ok(());
    }
    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Explainable insights", width))?;
    writeln!(writer)?;
    for family in &report.insights.families {
        writeln!(
            writer,
            "  {}",
            terminal_text(&super::super::insights::family_line(family))
        )?;
    }
    writeln!(writer)?;
    for card in cards {
        set(writer, Some(Color::Cyan), true, false)?;
        writeln!(writer, "  {}", terminal_text(&card.title))?;
        set(writer, None, false, true)?;
        writeln!(writer, "  {}", terminal_text(&card.finding))?;
        writeln!(
            writer,
            "  {}",
            terminal_text(&super::super::insights::context_line(card))
        )?;
        for line in super::super::insights::narrative_lines(card) {
            writeln!(writer, "  {}", terminal_text(&line))?;
        }
        if let Some(comparison) = super::super::insights::comparison_line(card) {
            writeln!(writer, "  {}", terminal_text(&comparison))?;
        }
        writeln!(
            writer,
            "  {}",
            terminal_text(&super::super::insights::limitations_line(card))
        )?;
        for fact in &card.supporting_facts {
            writeln!(
                writer,
                "  {}",
                terminal_text(&super::super::insights::fact_line(fact))
            )?;
        }
        if let Some(action) = &card.action {
            writeln!(
                writer,
                "  Experiment · {}",
                terminal_text(&action.experiment)
            )?;
            for alternative in &action.alternative_explanations {
                writeln!(writer, "  Alternative · {}", terminal_text(alternative))?;
            }
        }
        writeln!(writer)?;
    }
    Ok(())
}

pub fn recommendations(
    report: &Report,
    writer: &mut impl WriteColor,
    width: usize,
) -> io::Result<()> {
    if report.recommendations.is_empty() {
        return Ok(());
    }

    set(writer, None, false, true)?;
    writeln!(writer, "{}", section_header("Recommendations", width))?;
    writeln!(writer)?;

    for rec in report.recommendations.iter().take(5) {
        let severity_color = match rec.severity.as_str() {
            "critical" => Color::Red,
            "warning" => Color::Yellow,
            "positive" => Color::Green,
            _ => Color::Cyan,
        };
        set(writer, Some(severity_color), true, false)?;
        let severity = terminal_text(&rec.severity).to_uppercase();
        write!(writer, "  {:>8}  ", severity)?;
        set(writer, Some(Color::White), true, false)?;
        writeln!(writer, "{}", terminal_text(&rec.title))?;
        set(writer, None, false, true)?;
        writeln!(writer, "  {:>8}  {}", "", terminal_text(&rec.action))?;
        writeln!(writer)?;
    }
    Ok(())
}

// ── Trend / inflection ──────────────────────────────────────────────────────

pub fn trend(report: &Report, writer: &mut impl WriteColor) -> io::Result<()> {
    let Some(inflection) = &report.inflection else {
        return Ok(());
    };
    set(writer, None, false, true)?;
    writeln!(writer, "  Trend: {}", terminal_text(&inflection.summary))?;
    writeln!(writer)?;
    Ok(())
}
