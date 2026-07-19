use crate::{format_currency, format_tokens, Report};

fn escape_md_cell(s: &str) -> String {
    escape_md_value(s, false)
}

fn sanitize_md_paragraph(s: &str) -> String {
    escape_md_text(s)
}

fn sanitize_md_inline(s: &str) -> String {
    escape_md_text(s)
}

fn escape_md_text(value: &str) -> String {
    escape_md_value(value, true)
}

fn escape_md_value(value: &str, encode_block_prefixes: bool) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\r' | '\n') {
            escaped.push(' ');
        } else if character.is_control() || is_bidi_control(character) {
            escaped.push('\u{fffd}');
        } else if character == '&' {
            escaped.push_str("&amp;");
        } else if character == '<' {
            escaped.push_str("&lt;");
        } else if character == '>' {
            escaped.push_str("&gt;");
        } else if matches!(
            character,
            '\\' | '`'
                | '*'
                | '_'
                | '['
                | ']'
                | '('
                | ')'
                | '!'
                | '#'
                | '|'
                | '{'
                | '}'
                | '~'
                | '+'
                | '.'
                | ':'
                | '@'
        ) || (encode_block_prefixes && character == '-')
        {
            escaped.push_str("&#");
            escaped.push_str(&(character as u32).to_string());
            escaped.push(';');
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn escape_md_code(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\r' | '\n') {
            escaped.push(' ');
        } else if character.is_control() || is_bidi_control(character) {
            escaped.push('\u{fffd}');
        } else if character == '_' || character.is_ascii_alphanumeric() {
            escaped.push(character);
        } else if character == '&' {
            escaped.push_str("&amp;");
        } else if character == '<' {
            escaped.push_str("&lt;");
        } else if character == '>' {
            escaped.push_str("&gt;");
        } else if character.is_ascii_punctuation() {
            escaped.push_str("&#");
            escaped.push_str(&(character as u32).to_string());
            escaped.push(';');
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn sanitize_md_proof_line(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\r' | '\n') {
            escaped.push(' ');
        } else if character.is_control() || is_bidi_control(character) {
            escaped.push('\u{fffd}');
        } else {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '`' => escaped.push_str("&#96;"),
                _ => escaped.push(character),
            }
        }
    }
    escaped
}

fn sanitize_md_trust_line(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in sanitize_md_proof_line(value).chars() {
        if matches!(
            character,
            '\\' | '!' | '[' | ']' | '(' | ')' | '#' | '*' | '_' | '|' | '{' | '}' | '~' | ':'
        ) {
            escaped.push_str("&#");
            escaped.push_str(&(character as u32).to_string());
            escaped.push(';');
        } else {
            escaped.push(character);
        }
    }
    escaped
}

fn is_bidi_control(character: char) -> bool {
    matches!(
        character,
        '\u{061c}'
            | '\u{200e}'
            | '\u{200f}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202a}'..='\u{202e}'
            | '\u{2066}'..='\u{2069}'
    )
}

pub fn render_markdown(report: &Report) -> String {
    let wrapped = &report.wrapped_story;
    let local_cost = crate::canonical_local_cost(report);
    let cache_read = &report.canonical_metrics.cache.read_share;
    let cache_write = &report.canonical_metrics.cache.write_share;
    let summary = sanitize_md_paragraph(&wrapped.summary).replace('\n', " ");
    let mut lines = Vec::new();

    lines.push(format!(
        "# {}",
        escape_md_text(crate::experience_label(report))
    ));
    lines.push(String::new());
    lines.push(format!(
        "> {} — {}",
        escape_md_text(&wrapped.archetype.title),
        summary
    ));
    lines.push(String::new());
    lines.push("## Trust summary".to_string());
    lines.push(String::new());
    lines.push("```text".to_string());
    lines.extend(
        crate::trust_projection(report, "standard")
            .lines()
            .into_iter()
            .map(|line| sanitize_md_trust_line(&line)),
    );
    lines.push("```".to_string());
    lines.push(String::new());
    lines.push("## Season Summary".to_string());
    lines.push(String::new());
    lines.push(format!(
        "- **API-equivalent estimate:** {}",
        local_cost.map_or_else(|| "Unavailable".to_string(), format_currency)
    ));
    lines.push(format!(
        "- **Active days:** {}",
        report.cost_analysis.active_days
    ));
    lines.push(format!(
        "- **Cache-read share:** {}",
        escape_md_text(&crate::canonical_ratio_display(cache_read))
    ));
    lines.push(format!(
        "- **Cache-write share:** {}",
        escape_md_text(&crate::canonical_ratio_display(cache_write))
    ));
    if crate::canonical_evidence_is_limited(report) {
        lines.push(format!(
            "- **Analytical limitation:** {}",
            crate::PARTIAL_USAGE_LIMITATION
        ));
    }
    lines.push(String::new());

    lines.push("## Canonical method facts".to_string());
    lines.push(String::new());
    lines.push("```text".to_string());
    lines.extend(
        crate::canonical_fact_lines(report)
            .into_iter()
            .map(|line| sanitize_md_proof_line(&line)),
    );
    lines.push("```".to_string());
    lines.push(String::new());

    let insight_cards = super::insights::cards(report, false).collect::<Vec<_>>();
    if !insight_cards.is_empty() || !report.insights.families.is_empty() {
        lines.push("## Explainable insights".to_string());
        lines.push(String::new());
        for family in &report.insights.families {
            lines.push(format!(
                "- {}",
                sanitize_md_inline(&super::insights::family_line(family))
            ));
        }
        lines.push(String::new());
        for card in insight_cards {
            lines.push(format!("### {}", escape_md_text(&card.title)));
            lines.push(String::new());
            lines.push(sanitize_md_paragraph(&card.finding));
            lines.push(String::new());
            lines.push(format!(
                "- {}",
                sanitize_md_inline(&super::insights::context_line(card))
            ));
            lines.extend(
                super::insights::narrative_lines(card)
                    .into_iter()
                    .map(|line| format!("- {}", sanitize_md_inline(&line))),
            );
            if let Some(comparison) = super::insights::comparison_line(card) {
                lines.push(format!("- {}", sanitize_md_inline(&comparison)));
            }
            lines.push(format!(
                "- {}",
                sanitize_md_inline(&super::insights::limitations_line(card))
            ));
            for fact in &card.supporting_facts {
                lines.push(format!(
                    "- {}",
                    sanitize_md_inline(&super::insights::fact_line(fact))
                ));
            }
            if let Some(action) = &card.action {
                lines.push(format!(
                    "- **Experiment:** {}",
                    sanitize_md_inline(&action.experiment)
                ));
                for alternative in &action.alternative_explanations {
                    lines.push(format!(
                        "- **Alternative:** {}",
                        sanitize_md_inline(alternative)
                    ));
                }
            }
            lines.push(String::new());
        }
    }

    let coverage = &report.data_coverage;
    lines.push("## Data coverage".to_string());
    lines.push(String::new());
    lines.push(format!(
        "- **Completeness:** {}",
        sanitize_md_inline(&coverage.completeness)
    ));
    lines.push(format!(
        "- **Observed input:** {} sources · {} files · {} accepted records",
        coverage.source_root_count, coverage.files_discovered, coverage.accepted_records
    ));
    lines.push(format!(
        "- **Retention and limitations:** {}",
        sanitize_md_inline(&coverage.retention_caveat)
    ));
    if coverage.warnings.is_empty() {
        lines.push("- **Warnings:** none".to_string());
    } else {
        lines.push("- **Warnings:**".to_string());
        for warning in &coverage.warnings {
            let source = warning
                .source_alias
                .as_deref()
                .map_or(String::new(), |alias| {
                    format!(" · {}", sanitize_md_inline(alias))
                });
            lines.push(format!(
                "  - `{}`{} — {}",
                escape_md_code(&warning.code),
                source,
                sanitize_md_inline(&warning.message)
            ));
        }
    }
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
            escape_md_cell(&hero.note)
        ));
    }
    lines.push(String::new());

    lines.push("## Highlights".to_string());
    lines.push(String::new());
    for highlight in &wrapped.highlights {
        lines.push(format!("### {}", escape_md_text(&highlight.eyebrow)));
        lines.push(format!("**{}**", escape_md_text(&highlight.title)));
        lines.push(String::new());
        lines.push(sanitize_md_paragraph(&highlight.note));
        lines.push(String::new());
    }

    if report.model_routing.available {
        lines.push("## Model Request Mix".to_string());
        lines.push(String::new());
        lines.push("| Tier | Request share |".to_string());
        lines.push("|------|---------------|".to_string());
        for row in crate::model_request_mix_rows(&report.model_routing) {
            lines.push(format!(
                "| {} | {} |",
                escape_md_cell(row.label),
                escape_md_cell(&format!("{}%", row.share_pct))
            ));
        }
        lines.push(String::new());
        lines.push(format!(
            "{} across {} direct request observations. Local cost coverage: {}; {} unpriced request{}.",
            escape_md_text(&report.model_routing.method_id),
            report.model_routing.observations,
            escape_md_text(&report.canonical_metrics.cost.coverage),
            report.canonical_metrics.cost.unpriced_requests,
            if report.canonical_metrics.cost.unpriced_requests == 1 {
                ""
            } else {
                "s"
            }
        ));
        lines.push(String::new());
    }

    if !report.project_breakdown.is_empty() {
        lines.push("## Top Projects".to_string());
        lines.push(String::new());
        lines.push("| Project | Output tokens | Sessions |".to_string());
        lines.push("|---------|--------------|---------|".to_string());
        for project in crate::ranked_projects(&report.project_breakdown)
            .into_iter()
            .take(10)
        {
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
        lines.push("## Largest Sessions".to_string());
        lines.push(String::new());
        lines.push("| Project | Tokens | Active / elapsed |".to_string());
        lines.push("|---------|--------|------------------|".to_string());
        for session in report.session_breakdown.sessions.iter().take(5) {
            lines.push(format!(
                "| {} | {} | {} |",
                escape_md_cell(&session.project_name),
                escape_md_cell(&format_tokens(session.total_tokens)),
                escape_md_cell(&format!(
                    "{}s / {}s",
                    session.active_seconds, session.elapsed_seconds
                ))
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
        "_Claude Code Wrapped · data through {}_",
        escape_md_text(&report.generated_at)
    ));
    lines.push(String::new());
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::render_markdown;
    use crate::{HeroStat, Highlight, IngestionWarning, ProjectSummary, Recommendation, Report};

    #[test]
    fn hostile_report_values_remain_literal() {
        let hostile = "<img src=\"https://attacker.invalid/raw\"> ![x](https://attacker.invalid/image) [x](javascript:alert(1))\n# injected";
        let mut report = Report {
            generated_at: hostile.to_string(),
            ..Report::default()
        };
        report.data_coverage.completeness = hostile.to_string();
        report.data_coverage.retention_caveat = hostile.to_string();
        report.data_coverage.warnings.push(IngestionWarning {
            code: hostile.to_string(),
            message: hostile.to_string(),
            source_alias: Some(hostile.to_string()),
        });
        report.wrapped_story.archetype.title = hostile.to_string();
        report.wrapped_story.summary = hostile.to_string();
        report.wrapped_story.hero.push(HeroStat {
            label: hostile.to_string(),
            value: hostile.to_string(),
            note: hostile.to_string(),
        });
        report.wrapped_story.highlights.push(Highlight {
            eyebrow: hostile.to_string(),
            title: hostile.to_string(),
            note: hostile.to_string(),
        });
        report.project_breakdown.push(ProjectSummary {
            name: hostile.to_string(),
            ..ProjectSummary::default()
        });
        report.recommendations.push(Recommendation {
            title: hostile.to_string(),
            action: hostile.to_string(),
            ..Recommendation::default()
        });

        let markdown = render_markdown(&report);
        for active in ["<img", "![", "](http", "javascript:", "\n# injected"] {
            assert!(
                !markdown.contains(active),
                "hostile report value produced active Markdown: {active}"
            );
        }
        assert!(markdown.contains("attacker&#46;invalid"));
    }
}
