use ccwrapped::analyzers::cache::{analyze_cache_health, detect_inflection_points};
use ccwrapped::analyzers::cost::analyze_usage;
use ccwrapped::analyzers::models::{
    analyze_model_routing, analyze_session_intelligence, detect_anomalies,
};
use ccwrapped::analyzers::recommendations::generate_recommendations;
use ccwrapped::analyzers::story::build_wrapped_story;
use ccwrapped::readers::jsonl::{aggregate_by_project, aggregate_daily, read_all_jsonl};
use ccwrapped::readers::session::read_session_breakdown;
use ccwrapped::renderers::html::render_html;
use ccwrapped::renderers::markdown::render_markdown;
use ccwrapped::renderers::share_card::render_share_card;
use ccwrapped::Report;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn temp_projects_dir(name: &str) -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir()
        .join(format!("ccwrapped-rs-{name}-{unique}"))
        .join("projects");
    fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn story_builder_pipeline_matches_expected_sections() {
    let projects_dir = temp_projects_dir("story");
    let project_dir = projects_dir.join("-work-demo-app");
    fs::create_dir_all(project_dir.join("session-1/subagents")).unwrap();

    let top_level = [
        serde_json::json!({
            "type": "user",
            "userType": "external",
            "isSidechain": false,
            "timestamp": "2026-04-05T09:00:00.000Z",
            "message": { "content": "Build the demo app shell" }
        }),
        serde_json::json!({
            "type": "user",
            "userType": "external",
            "timestamp": "2026-04-05T09:01:00.000Z",
            "message": { "content": [{ "type": "tool_result", "content": "ok" }] }
        }),
        serde_json::json!({
            "type": "assistant",
            "cwd": "/work/demo-app",
            "timestamp": "2026-04-05T17:00:00.000Z",
            "sessionId": "session-1",
            "message": {
                "id": "msg_1",
                "model": "claude-opus-4-6",
                "usage": {
                    "input_tokens": 1000,
                    "output_tokens": 1800,
                    "cache_creation_input_tokens": 500,
                    "cache_read_input_tokens": 4000
                },
                "content": [{ "type": "tool_use", "name": "Bash" }]
            }
        }),
        serde_json::json!({
            "type": "assistant",
            "cwd": "/work/demo-app",
            "timestamp": "2026-04-06T17:20:00.000Z",
            "sessionId": "session-1",
            "message": {
                "id": "msg_2",
                "model": "claude-sonnet-4-6",
                "usage": {
                    "input_tokens": 500,
                    "output_tokens": 600,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 700
                },
                "content": [{ "type": "tool_use", "name": "Read" }]
            }
        }),
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    fs::write(
        project_dir.join("session-1.jsonl"),
        format!("{top_level}\n"),
    )
    .unwrap();

    let subagent = [
        serde_json::json!({
            "type": "user",
            "userType": "external",
            "isSidechain": false,
            "timestamp": "2026-04-05T10:00:00.000Z",
            "message": { "content": "Search the docs" }
        }),
        serde_json::json!({
            "type": "assistant",
            "cwd": "/work/demo-app",
            "timestamp": "2026-04-05T11:00:00.000Z",
            "sessionId": "sub-1",
            "message": {
                "id": "msg_sub",
                "model": "claude-sonnet-4-6",
                "usage": {
                    "input_tokens": 300,
                    "output_tokens": 400,
                    "cache_creation_input_tokens": 0,
                    "cache_read_input_tokens": 200
                },
                "content": [{ "type": "tool_use", "name": "Bash" }]
            }
        }),
    ]
    .iter()
    .map(|value| value.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    fs::write(
        project_dir.join("session-1/subagents/sub-1.jsonl"),
        format!("{subagent}\n"),
    )
    .unwrap();

    let entries = read_all_jsonl(&projects_dir, Some(2026));
    assert_eq!(entries.len(), 3);
    let daily = aggregate_daily(&entries);
    let session_breakdown = read_session_breakdown(&projects_dir, Some(2026));
    assert_eq!(session_breakdown.sessions[0].subagents.len(), 1);
    assert_eq!(
        session_breakdown.sessions[0].subagents[0].session_id,
        "sub-1"
    );
    let project_breakdown = aggregate_by_project(&entries);
    let cost_analysis = analyze_usage(2026, &daily, &session_breakdown);
    let cache_health = analyze_cache_health(&daily);
    let anomalies = detect_anomalies(&cost_analysis);
    let inflection = detect_inflection_points(&daily);
    let session_intel = analyze_session_intelligence(&session_breakdown, &entries);
    let model_routing = analyze_model_routing(&cost_analysis, &entries);
    let recommendations = generate_recommendations(
        &cost_analysis,
        &cache_health,
        &anomalies,
        &inflection,
        &session_intel,
        &model_routing,
        &project_breakdown,
    );

    let mut report = Report {
        generated_at: "2026-04-06T12:00:00.000Z".to_string(),
        year: 2026,
        cost_analysis,
        cache_health,
        anomalies,
        inflection,
        session_intel,
        session_breakdown,
        model_routing,
        project_breakdown,
        recommendations,
        wrapped_story: Default::default(),
    };
    report.wrapped_story = build_wrapped_story(&report, &entries);

    assert_eq!(report.wrapped_story.archetype.title, "Precision Maximalist");
    assert_eq!(report.wrapped_story.top_tool.as_ref().unwrap().name, "Bash");
    assert_eq!(
        report.wrapped_story.top_project.as_ref().unwrap().name,
        "demo-app"
    );
    assert_eq!(report.wrapped_story.prompt_ratio.human, 1);
    assert_eq!(report.wrapped_story.prompt_ratio.tool, 1);
    assert_eq!(report.wrapped_story.hero.len(), 5);

    let json = serde_json::to_value(&report).unwrap();
    assert!(json.get("generated_at").is_none());
    assert!(json.get("wrapped_story").is_none());
    assert_eq!(json["generatedAt"], "2026-04-06T12:00:00.000Z");
    assert_eq!(json["year"], 2026);
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["projectHash"],
        "-work-demo-app"
    );
    assert_eq!(json["projectBreakdown"][0]["hash"], "-work-demo-app");
    assert!(json["costAnalysis"]["dailyCosts"].is_array());
    assert!(json["cacheHealth"]["savings"]["fromCaching"].is_number());
    assert_eq!(json["wrappedStory"]["hero"].as_array().unwrap().len(), 5);

    let html = render_html(&report);
    assert!(html.contains("Claude Code Wrapped"));
    assert!(html.contains("Costliest sessions"));
    assert!(html.contains("Subagent spikes"));
    assert!(html.contains("Next season"));

    let markdown = render_markdown(&report);
    assert!(markdown.contains("## Hero Stats"));
    assert!(markdown.contains("## Highlights"));
    assert!(markdown.contains("## Top Projects"));
    assert!(markdown.contains("## Human vs Tool Prompts"));
    assert!(!markdown.contains("<div"));

    let card = render_share_card(&report);
    assert!(card.contains("Season spend"));
    assert!(card.contains("Cache grade"));
    assert!(card.contains("Power hour"));
    assert!(!card.contains("<script"));
    assert!(!card.contains("demo-app"));
}

#[test]
fn project_aggregation_prefers_cwd_and_tracks_subagents() {
    let entries = vec![
        ccwrapped::AssistantEntry {
            session_id: "top-1".to_string(),
            project_hash: "-home-user".to_string(),
            is_subagent: false,
            cwd: Some("/home/user".to_string()),
            timestamp: "2026-04-06T10:00:00.000Z".to_string(),
            model: "claude-opus-4-6".to_string(),
            input_tokens: 1,
            output_tokens: 10,
            cache_creation_tokens: 0,
            cache_read_tokens: 5,
            cost_usd: 0.0,
            tool_names: vec![],
        },
        ccwrapped::AssistantEntry {
            session_id: "sub-1".to_string(),
            project_hash: "-home-user".to_string(),
            is_subagent: true,
            cwd: Some("/home/user".to_string()),
            timestamp: "2026-04-06T11:00:00.000Z".to_string(),
            model: "claude-sonnet-4-6".to_string(),
            input_tokens: 1,
            output_tokens: 20,
            cache_creation_tokens: 0,
            cache_read_tokens: 5,
            cost_usd: 0.0,
            tool_names: vec![],
        },
    ];

    let projects = aggregate_by_project(&entries);
    assert_eq!(projects[0].name, "workspace root");
    assert_eq!(projects[0].path.as_deref(), Some("/home/user"));
    assert_eq!(projects[0].session_count, 1);
    assert_eq!(projects[0].subagent_session_count, 1);
}
