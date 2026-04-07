use super::html_css::STYLE_BLOCK;
use super::html_slides::{
    slide_activity, slide_biggest_session, slide_cache_grade, slide_highlights,
    slide_model_and_projects, slide_opening, slide_power_hour, slide_prompts_and_savings,
    slide_recommendations, slide_sessions_and_subagents, slide_spend, slide_top_project,
    slide_top_tool,
};
use crate::Report;

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
        slide_recommendations(report),
        "</body>\n</html>".to_string(),
    ]
    .join("\n")
}
