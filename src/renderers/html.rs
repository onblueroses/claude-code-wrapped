use super::html_css::STYLE_BLOCK;
use super::html_slides::{
    slide_activity, slide_biggest_session, slide_cache_grade, slide_highlights, slide_insights,
    slide_model_and_projects, slide_opening, slide_power_hour, slide_prompts_and_savings,
    slide_recommendations, slide_sessions_and_subagents, slide_spend, slide_top_project,
    slide_top_tool,
};
use crate::{escape_html, Report};

pub fn render_html(report: &Report) -> String {
    [
        format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Claude Code Wrapped {}</title>
  {}
</head>
<body>
"#,
            report.year, STYLE_BLOCK
        ),
        slide_opening(report),
        slide_spend(report),
        slide_power_hour(report),
        slide_top_project(report),
        slide_cache_grade(report),
        slide_top_tool(report),
        slide_biggest_session(report),
        slide_activity(report),
        slide_model_and_projects(report),
        slide_sessions_and_subagents(report),
        slide_prompts_and_savings(report),
        slide_highlights(report),
        slide_insights(report),
        slide_recommendations(report),
        coverage_section(report),
        "</body>\n</html>".to_string(),
    ]
    .join("\n")
}

fn coverage_section(report: &Report) -> String {
    let coverage = &report.data_coverage;
    let analytical_limitation = if !crate::canonical_evidence_is_limited(report) {
        String::new()
    } else {
        format!(
            "<p class=\"slide-sub\">{}</p>",
            escape_html(crate::PARTIAL_USAGE_LIMITATION)
        )
    };
    let warnings = if coverage.warnings.is_empty() {
        "<p class=\"slide-sub\">No ingestion warnings were recorded.</p>".to_string()
    } else {
        coverage
            .warnings
            .iter()
            .map(|warning| {
                let source = warning
                    .source_alias
                    .as_deref()
                    .map_or(String::new(), |alias| format!(" · {}", escape_html(alias)));
                format!(
                    "<div class=\"card\"><div class=\"eyebrow\">{}{}</div><p>{}</p></div>",
                    escape_html(&warning.code),
                    source,
                    escape_html(&warning.message)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let facts = crate::canonical_fact_lines(report)
        .into_iter()
        .map(|line| escape_html(&line))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"  <!-- ── DATA COVERAGE / LIMITATIONS ── -->
  <section class="slide s-dark data-slide coverage-slide">
    <div class="slide-inner">
      <div class="slide-label">Data coverage</div>
      <h2 class="slide-hero-med">{completeness}</h2>
      <p class="slide-sub">{sources} sources · {files} files · {accepted} accepted records</p>
      <p class="slide-sub">{retention}</p>
      {analytical_limitation}
      <pre class="slide-sub" style="white-space:pre-wrap;text-align:left">{facts}</pre>
      <div class="card-grid">{warnings}</div>
    </div>
  </section>"#,
        completeness = escape_html(&coverage.completeness),
        sources = coverage.source_root_count,
        files = coverage.files_discovered,
        accepted = coverage.accepted_records,
        retention = escape_html(&coverage.retention_caveat),
        analytical_limitation = analytical_limitation,
        facts = facts,
    )
}
