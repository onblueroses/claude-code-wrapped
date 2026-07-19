use ccwrapped::analyzers::cache::{analyze_cache_health, detect_inflection_points};
use ccwrapped::analyzers::cost::analyze_usage;
use ccwrapped::analyzers::models::{
    analyze_model_routing, analyze_session_intelligence, detect_anomalies,
};
use ccwrapped::analyzers::recommendations::generate_recommendations;
use ccwrapped::analyzers::story::build_wrapped_story;
use ccwrapped::renderers::html::render_html;
use ccwrapped::renderers::markdown::render_markdown;
use ccwrapped::renderers::share_card::render_share_card;
use ccwrapped::renderers::terminal::try_render_terminal_to;
use ccwrapped::{DailyAggregate, Report, SessionBreakdown};

fn main() {
    let daily = vec![DailyAggregate {
        date: "2026-01-01".to_string(),
        ..DailyAggregate::default()
    }];
    let sessions = SessionBreakdown::default();
    let entries = Vec::new();
    let cost = analyze_usage(2026, &daily, &sessions);
    let cache = analyze_cache_health(&daily);
    let anomalies = detect_anomalies(&cost);
    let inflection = detect_inflection_points(&daily);
    let session_intel = analyze_session_intelligence(&sessions, &entries);
    let routing = analyze_model_routing(&cost, &entries);
    let projects = Vec::new();
    let recommendations = generate_recommendations(
        &cost,
        &cache,
        &anomalies,
        &inflection,
        &session_intel,
        &routing,
        &projects,
    );
    let mut report = Report {
        schema_version: "ccwrapped.report/v2".to_string(),
        year: 2026,
        cost_analysis: cost,
        cache_health: cache,
        anomalies,
        inflection,
        session_intel,
        session_breakdown: sessions,
        model_routing: routing,
        project_breakdown: projects,
        recommendations,
        ..Report::default()
    };
    report.wrapped_story = build_wrapped_story(&report, &entries);

    let _json = serde_json::to_string(&report).expect("serialize public report");
    let _html = render_html(&report);
    let _markdown = render_markdown(&report);
    let _card = render_share_card(&report);
    let mut terminal = termcolor::NoColor::new(Vec::new());
    try_render_terminal_to(&report, &mut terminal).expect("render public report");
}
