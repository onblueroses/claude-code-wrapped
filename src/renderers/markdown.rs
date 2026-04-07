use crate::{format_currency, format_ratio, format_tokens, Report};

fn escape_md_cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

fn sanitize_md_paragraph(s: &str) -> String {
    s.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if trimmed.starts_with('#') {
                trimmed.trim_start_matches('#').trim_start().to_string()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn render_markdown(report: &Report) -> String {
    let wrapped = &report.wrapped_story;
    let summary = sanitize_md_paragraph(&wrapped.summary).replace('\n', " ");
    let mut lines = Vec::new();

    lines.push("# Claude Code Wrapped".to_string());
    lines.push(String::new());
    lines.push(format!("> {} — {}", wrapped.archetype.title, summary));
    lines.push(String::new());
    lines.push("## Season Summary".to_string());
    lines.push(String::new());
    lines.push(format!(
        "- **Total spend:** {}",
        format_currency(report.cost_analysis.total_cost)
    ));
    lines.push(format!(
        "- **Active days:** {}",
        report.cost_analysis.active_days
    ));
    lines.push(format!(
        "- **Cache grade:** {}",
        report.cache_health.grade.letter
    ));
    lines.push(format!(
        "- **Cache efficiency:** {}",
        format_ratio(report.cache_health.efficiency_ratio)
    ));
    lines.push(String::new());

    lines.push("## Hero Stats".to_string());
    lines.push(String::new());
    lines.push("| Stat | Value | Note |".to_string());
    lines.push("|------|-------|------|".to_string());
    for hero in &wrapped.hero {
        lines.push(format!(
            "| {} | **{}** | {} |",
            escape_md_cell(&hero.label),
            escape_md_cell(&hero.value),
            escape_md_cell(&sanitize_md_paragraph(&hero.note))
        ));
    }
    lines.push(String::new());

    lines.push("## Highlights".to_string());
    lines.push(String::new());
    for highlight in &wrapped.highlights {
        lines.push(format!("### {}", highlight.eyebrow));
        lines.push(format!("**{}**", highlight.title));
        lines.push(String::new());
        lines.push(sanitize_md_paragraph(&highlight.note));
        lines.push(String::new());
    }

    if report.model_routing.available {
        lines.push("## Model Mix".to_string());
        lines.push(String::new());
        lines.push("| Model | Share |".to_string());
        lines.push("|-------|-------|".to_string());
        if report.model_routing.opus_pct > 0 {
            lines.push(format!(
                "| {} | {} |",
                escape_md_cell("Opus"),
                escape_md_cell(&format!("{}%", report.model_routing.opus_pct))
            ));
        }
        if report.model_routing.sonnet_pct > 0 {
            lines.push(format!(
                "| {} | {} |",
                escape_md_cell("Sonnet"),
                escape_md_cell(&format!("{}%", report.model_routing.sonnet_pct))
            ));
        }
        if report.model_routing.haiku_pct > 0 {
            lines.push(format!(
                "| {} | {} |",
                escape_md_cell("Haiku"),
                escape_md_cell(&format!("{}%", report.model_routing.haiku_pct))
            ));
        }
        lines.push(String::new());
    }

    if !report.project_breakdown.is_empty() {
        lines.push("## Top Projects".to_string());
        lines.push(String::new());
        lines.push("| Project | Output tokens | Sessions |".to_string());
        lines.push("|---------|--------------|---------|".to_string());
        for project in report.project_breakdown.iter().take(10) {
            lines.push(format!(
                "| {} | {} | {} |",
                escape_md_cell(&project.name),
                escape_md_cell(&format_tokens(project.output_tokens)),
                escape_md_cell(&project.session_count.to_string())
            ));
        }
        lines.push(String::new());
    }

    if !report.session_breakdown.sessions.is_empty() {
        lines.push("## Costliest Sessions".to_string());
        lines.push(String::new());
        lines.push("| Project | Tokens | Date |".to_string());
        lines.push("|---------|--------|------|".to_string());
        for session in report.session_breakdown.sessions.iter().take(5) {
            let date = session
                .timestamp_start
                .as_deref()
                .map(|value| value.chars().take(10).collect::<String>())
                .unwrap_or_else(|| "-".to_string());
            lines.push(format!(
                "| {} | {} | {} |",
                escape_md_cell(&session.project_name),
                escape_md_cell(&format_tokens(session.total_tokens)),
                escape_md_cell(&date)
            ));
        }
        lines.push(String::new());
    }

    if wrapped.prompt_ratio.total > 0 {
        lines.push("## Human vs Tool Prompts".to_string());
        lines.push(String::new());
        lines.push(format!(
            "{}% of messages were typed by you ({} human / {} tool).",
            wrapped.prompt_ratio.human_pct, wrapped.prompt_ratio.human, wrapped.prompt_ratio.tool
        ));
        lines.push(String::new());
    }

    if !report.recommendations.is_empty() {
        lines.push("## Recommendations".to_string());
        lines.push(String::new());
        for recommendation in report.recommendations.iter().take(3) {
            lines.push(format!("### {}", escape_md_cell(&recommendation.title)));
            lines.push(sanitize_md_paragraph(&recommendation.action));
            lines.push(String::new());
        }
    }

    lines.push("---".to_string());
    lines.push(String::new());
    lines.push(format!(
        "_Generated by Claude Code Wrapped on {}_",
        report.generated_at
    ));
    lines.push(String::new());
    lines.join("\n")
}
