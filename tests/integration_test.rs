use ccwrapped::analyzers::cache::{analyze_cache_health, detect_inflection_points};
use ccwrapped::analyzers::cost::analyze_usage;
use ccwrapped::analyzers::models::{
    analyze_model_routing, analyze_session_intelligence, detect_anomalies,
};
use ccwrapped::analyzers::recommendations::generate_recommendations;
use ccwrapped::analyzers::story::build_wrapped_story;
use ccwrapped::readers::discovery::{
    discover_jsonl_files, discover_session_files, try_discover_jsonl_files,
    try_discover_session_files,
};
use ccwrapped::readers::jsonl::{
    aggregate_by_project, aggregate_daily, read_all_jsonl, try_read_all_jsonl,
};
use ccwrapped::readers::session::{read_session_breakdown, try_read_session_breakdown};
use ccwrapped::renderers::html::render_html;
use ccwrapped::renderers::markdown::render_markdown;
use ccwrapped::renderers::share_card::render_share_card;
use ccwrapped::renderers::terminal::render_terminal_to;
use ccwrapped::{IngestionWarning, Report};
use std::fs;
use std::path::PathBuf;
use std::process::Command;
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
fn readme_assets_match_current_synthetic_exports() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let output = Command::new("bash")
        .arg("scripts/generate-readme-assets.sh")
        .arg("--verify-manifest")
        .current_dir(&root)
        .output()
        .expect("run README asset manifest guard");
    assert!(
        output.status.success(),
        "README assets or their renderer inputs drifted:\nstdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    for (name, width, height) in [
        ("hero-slide.png", 1200u32, 900u32),
        ("spend-slide.png", 1200, 900),
        ("cache-slide.png", 1200, 900),
        ("data-slide.png", 1200, 900),
        ("share-card.png", 540, 960),
    ] {
        let bytes = fs::read(root.join("assets").join(name)).expect("read pinned README PNG");
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{name} is not a PNG");
        assert_eq!(
            u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
            width,
            "{name} width drifted"
        );
        assert_eq!(
            u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
            height,
            "{name} height drifted"
        );
    }
}

#[cfg(unix)]
#[test]
fn legacy_discovery_rejects_out_of_root_symlinks() {
    use std::os::unix::fs::symlink;

    let projects_dir = temp_projects_dir("legacy-discovery-symlink-escape");
    let project_dir = projects_dir.join("project");
    let external_dir = projects_dir.parent().unwrap().join("external");
    fs::create_dir_all(&project_dir).unwrap();
    fs::create_dir_all(&external_dir).unwrap();
    fs::write(external_dir.join("secret.jsonl"), "{}\n").unwrap();
    symlink(&external_dir, project_dir.join("nested-escape")).unwrap();
    symlink(&external_dir, projects_dir.join("project-escape")).unwrap();

    let all_error = try_discover_jsonl_files(&projects_dir).unwrap_err();
    let session_error = try_discover_session_files(&projects_dir).unwrap_err();
    assert_eq!(all_error.code(), "E_TRANSCRIPT_DISCOVERY_PARTIAL");
    assert_eq!(session_error.code(), "E_TRANSCRIPT_DISCOVERY_PARTIAL");
    assert!(!all_error
        .message()
        .contains(&external_dir.to_string_lossy()[..]));
    assert!(!session_error
        .message()
        .contains(&external_dir.to_string_lossy()[..]));

    assert!(
        discover_jsonl_files(&projects_dir).is_empty(),
        "recursive compatibility discovery followed an out-of-root symlink"
    );
    assert!(
        discover_session_files(&projects_dir).is_empty(),
        "session compatibility discovery followed an out-of-root project symlink"
    );
}

#[test]
fn compatibility_discovery_is_bounded_fallible_and_scope_accurate() {
    let projects_dir = temp_projects_dir("compatibility-discovery-contract");
    let project_dir = projects_dir.join("project");
    let nested_dir = project_dir.join("session/subagents");
    fs::create_dir_all(&nested_dir).unwrap();
    let direct = project_dir.join("direct.jsonl");
    let nested = nested_dir.join("nested.jsonl");
    fs::write(&direct, "{}\n").unwrap();
    fs::write(&nested, "{}\n").unwrap();
    fs::write(project_dir.join("ignored.txt"), "{}\n").unwrap();

    let all = try_discover_jsonl_files(&projects_dir).unwrap();
    let sessions = try_discover_session_files(&projects_dir).unwrap();
    assert_eq!(
        all,
        vec![
            direct.canonicalize().unwrap(),
            nested.canonicalize().unwrap()
        ]
    );
    assert_eq!(sessions, vec![direct.canonicalize().unwrap()]);
    assert_eq!(discover_jsonl_files(&projects_dir), all);
    assert_eq!(discover_session_files(&projects_dir), sessions);

    let missing = projects_dir
        .parent()
        .unwrap()
        .join("PRIVATE_DISCOVERY_PATH_CANARY");
    let error = try_discover_jsonl_files(&missing).unwrap_err();
    assert_eq!(error.code(), "E_DISCOVERY_TRANSCRIPT_MISSING");
    assert_eq!(error.source_alias(), Some("transcript-1"));
    assert!(!error.message().contains("PRIVATE_DISCOVERY_PATH_CANARY"));
}

#[test]
fn compatibility_discovery_surfaces_the_depth_limit() {
    let projects_dir = temp_projects_dir("compatibility-discovery-depth");
    let mut directory = projects_dir.join("project");
    fs::create_dir_all(&directory).unwrap();
    for index in 0..130 {
        directory = directory.join(format!("depth-{index}"));
        fs::create_dir(&directory).unwrap();
    }
    fs::write(directory.join("excluded.jsonl"), "{}\n").unwrap();

    let error = try_discover_jsonl_files(&projects_dir).unwrap_err();
    assert_eq!(error.code(), "E_TRANSCRIPT_DISCOVERY_PARTIAL");
}

#[test]
fn public_daily_aggregation_uses_validated_utc_dates() {
    let entries = vec![
        ccwrapped::AssistantEntry {
            timestamp: "2025-12-31T23:30:00-02:00".to_string(),
            output_tokens: 7,
            ..Default::default()
        },
        ccwrapped::AssistantEntry {
            timestamp: "2026-99-99-not-a-timestamp".to_string(),
            output_tokens: 100,
            ..Default::default()
        },
    ];

    let daily = aggregate_daily(&entries);
    assert_eq!(daily.len(), 1);
    assert_eq!(daily[0].date, "2026-01-01");
    assert_eq!(daily[0].output_tokens, 7);
}

#[test]
fn resolve_project_path_breaks_equal_count_ties_lexically() {
    for _ in 0..32 {
        let mut counts = std::collections::HashMap::new();
        for index in 0..64 {
            counts.insert(format!("/work/path-{index:03}"), 1usize);
        }
        let (path, name) = ccwrapped::readers::jsonl::resolve_project_path(&counts, "fallback");
        assert_eq!(path.as_deref(), Some("/work/path-000"));
        assert_eq!(name, "path-000");
    }
}

#[test]
fn terminal_hostile_public_report_strings_are_inert() {
    let hostile = "safe\u{1b}]52;c;U0VDUkVU\u{7}\rnewline\nspoof\u{202e}\u{2028}end";
    let mut report = Report::default();
    report.wrapped_story.summary = hostile.to_string();
    report.wrapped_story.archetype.title = hostile.to_string();
    report.wrapped_story.hero = vec![ccwrapped::HeroStat {
        label: hostile.to_string(),
        value: hostile.to_string(),
        note: hostile.to_string(),
    }];
    report.wrapped_story.highlights = vec![ccwrapped::Highlight {
        eyebrow: hostile.to_string(),
        title: hostile.to_string(),
        note: hostile.to_string(),
    }];
    report.wrapped_story.favorite_weekday = Some(ccwrapped::NamedCount {
        label: hostile.to_string(),
        count: 1,
    });
    report.wrapped_story.top_tool = Some(ccwrapped::TopTool {
        name: hostile.to_string(),
        count: 1,
    });
    report.cache_health.grade.letter = hostile.to_string();
    report.cache_health.grade.label = hostile.to_string();
    report.cost_analysis.total_cost = 1.0;
    report.cost_analysis.daily_costs = vec![ccwrapped::DailyCost {
        date: hostile.to_string(),
        cost: 1.0,
        ..Default::default()
    }];
    report.cost_analysis.peak_day = report.cost_analysis.daily_costs.first().cloned();
    report
        .cost_analysis
        .model_costs
        .insert(hostile.to_string(), 1.0);
    report.project_breakdown = vec![ccwrapped::ProjectSummary {
        name: hostile.to_string(),
        output_tokens: 1,
        session_count: 1,
        ..Default::default()
    }];
    report.session_breakdown.sessions = vec![ccwrapped::SessionSummary {
        project_name: hostile.to_string(),
        timestamp_start: Some("2026-01-01T00:00:00Z".to_string()),
        total_tokens: 1,
        ..Default::default()
    }];
    report.session_breakdown.costly_subagents = vec![ccwrapped::SubagentSummary {
        project_name: Some(hostile.to_string()),
        timestamp_start: Some("2026-01-01T00:00:00Z".to_string()),
        total_tokens: 1,
        first_prompt: Some(hostile.to_string()),
        ..Default::default()
    }];
    report.recommendations = vec![ccwrapped::Recommendation {
        severity: hostile.to_string(),
        title: hostile.to_string(),
        action: hostile.to_string(),
        ..Default::default()
    }];
    report.insights.families = vec![ccwrapped::InsightFamilyStatus {
        family: hostile.to_string(),
        availability: "available".to_string(),
        limitations: vec![hostile.to_string()],
        ..Default::default()
    }];
    report.insights.cards = vec![ccwrapped::InsightCard {
        id: hostile.to_string(),
        class: hostile.to_string(),
        title: hostile.to_string(),
        finding: hostile.to_string(),
        availability: "available".to_string(),
        supporting_facts: vec![ccwrapped::InsightFact {
            id: hostile.to_string(),
            metric_id: hostile.to_string(),
            value: hostile.to_string(),
            unit: hostile.to_string(),
            method_id: hostile.to_string(),
            coverage: hostile.to_string(),
            source: hostile.to_string(),
            ..Default::default()
        }],
        action: Some(ccwrapped::InsightAction {
            experiment: hostile.to_string(),
            alternative_explanations: vec![hostile.to_string()],
        }),
        ..Default::default()
    }];
    report.inflection = Some(ccwrapped::InflectionPoint {
        summary: hostile.to_string(),
        ..Default::default()
    });
    for capability in ["analysis_cost", "analysis_cache_health"] {
        report
            .data_coverage
            .capabilities
            .insert(capability.to_string(), "available".to_string());
    }

    let mut output = termcolor::Buffer::no_color();
    render_terminal_to(&report, &mut output);
    let rendered = String::from_utf8(output.as_slice().to_vec()).unwrap();

    for forbidden in ['\u{1b}', '\u{7}', '\r', '\u{202e}', '\u{2028}'] {
        assert!(
            !rendered.contains(forbidden),
            "terminal output retained hostile control U+{:04X}",
            forbidden as u32
        );
    }
    assert!(rendered.contains("safe�]52;c;U0VDUkVU��newline�spoof��end"));
}

#[test]
fn hostile_multibyte_public_timestamps_do_not_panic() {
    let malformed = "123456789é".to_string();
    let session = ccwrapped::SessionSummary {
        timestamp_start: Some(malformed.clone()),
        total_tokens: 1,
        cost_usd: 1.0,
        ..Default::default()
    };
    let subagent = ccwrapped::SubagentSummary {
        timestamp_start: Some(malformed),
        total_tokens: 1,
        ..Default::default()
    };
    let mut report = Report::default();
    report.session_breakdown.sessions = vec![session];
    report.session_breakdown.costly_subagents = vec![subagent];
    for capability in ["analysis_cost", "analysis_usage_totals"] {
        report
            .data_coverage
            .capabilities
            .insert(capability.to_string(), "available".to_string());
    }

    assert!(
        std::panic::catch_unwind(|| build_wrapped_story(&report, &[])).is_ok(),
        "story construction sliced a malformed public timestamp at a byte boundary"
    );
    assert!(
        std::panic::catch_unwind(|| render_html(&report)).is_ok(),
        "HTML rendering sliced a malformed public timestamp at a byte boundary"
    );
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut output = termcolor::Buffer::no_color();
            render_terminal_to(&report, &mut output);
        }))
        .is_ok(),
        "terminal rendering sliced a malformed public timestamp at a byte boundary"
    );
}

#[test]
fn html_project_bars_handle_maximum_public_token_values() {
    let report = Report {
        project_breakdown: vec![ccwrapped::ProjectSummary {
            hash: "project-1".to_string(),
            name: "project-1".to_string(),
            output_tokens: u64::MAX,
            ..Default::default()
        }],
        ..Default::default()
    };

    assert!(
        std::panic::catch_unwind(|| render_html(&report)).is_ok(),
        "HTML project percentage overflowed for a legal public u64 value"
    );
}

#[test]
fn public_compatibility_readers_use_the_privacy_safe_normalized_projection() {
    const CANARY: &str = "PRIVATE_COMPATIBILITY_CANARY_7F21";
    const PERCENT_CANARY: &str = "%50%52%49%56%41%54%45%5f%43%4f%4d%50%41%54%49%42%49%4c%49%54%59%5f%43%41%4e%41%52%59%5f%37%46%32%31";
    const BASE64_CANARY: &str = "UFJJVkFURV9DT01QQVRJQklMSVRZX0NBTkFSWV83RjIx";
    const JSON_ESCAPED_CANARY: &str = r"\u0050\u0052\u0049\u0056\u0041\u0054\u0045\u005f\u0043\u004f\u004d\u0050\u0041\u0054\u0049\u0042\u0049\u004c\u0049\u0054\u0059\u005f\u0043\u0041\u004e\u0041\u0052\u0059\u005f\u0037\u0046\u0032\u0031";
    let projects_dir = temp_projects_dir("public-normalized-privacy");
    let project_dir = projects_dir.join(CANARY);
    fs::create_dir_all(&project_dir).unwrap();
    let record = serde_json::json!({
        "type": "assistant",
        "cwd": format!("/tmp/{PERCENT_CANARY}"),
        "timestamp": "2026-04-05T09:00:00Z",
        "sessionId": format!("session-{BASE64_CANARY}"),
        "message": {
            "id": format!("message-{JSON_ESCAPED_CANARY}"),
            "model": format!("model-{CANARY}"),
            "usage": {
                "input_tokens": 1,
                "output_tokens": 2,
                "cache_creation_input_tokens": 3,
                "cache_read_input_tokens": 4
            },
            "content": [{
                "type": "tool_use",
                "name": format!("tool-{BASE64_CANARY}"),
                "input": {
                    "raw": CANARY,
                    "percent": PERCENT_CANARY,
                    "base64": BASE64_CANARY,
                    "jsonEscaped": JSON_ESCAPED_CANARY
                }
            }]
        }
    });
    fs::write(project_dir.join("session.jsonl"), format!("{record}\n")).unwrap();

    let (fallible_entries, coverage) = try_read_all_jsonl(&projects_dir, Some(2026)).unwrap();
    let (fallible_sessions, session_coverage) =
        try_read_session_breakdown(&projects_dir, Some(2026)).unwrap();
    let entries = read_all_jsonl(&projects_dir, Some(2026));
    let sessions = read_session_breakdown(&projects_dir, Some(2026));
    let serialized = serde_json::to_string(&(
        &entries,
        &sessions,
        &fallible_entries,
        &fallible_sessions,
        &coverage,
        &session_coverage,
    ))
    .unwrap();

    for canary in [CANARY, PERCENT_CANARY, BASE64_CANARY, JSON_ESCAPED_CANARY] {
        assert!(
            !serialized.contains(canary),
            "a public compatibility reader retained raw or encoded private content"
        );
    }
    assert_eq!(
        serde_json::to_value(&entries).unwrap(),
        serde_json::to_value(&fallible_entries).unwrap(),
        "the infallible compatibility projection diverged from its fallible counterpart"
    );
    assert_eq!(
        serde_json::to_value(&sessions).unwrap(),
        serde_json::to_value(&fallible_sessions).unwrap(),
        "the infallible session projection diverged from its fallible counterpart"
    );
    assert_eq!(entries[0].project_hash, "project-1");
    assert!(entries[0].session_id.starts_with("session-"));
    assert_eq!(entries[0].cwd, None);
    assert_eq!(entries[0].model, "unknown");
    assert_eq!(entries[0].tool_names, vec!["other"]);
    assert_eq!(coverage.selected_period, "2026");
    fs::remove_dir_all(projects_dir.parent().unwrap()).unwrap();
}

#[test]
fn public_compatibility_reader_sources_have_no_independent_raw_adapter() {
    for (name, source) in [
        ("jsonl", include_str!("../src/readers/jsonl.rs")),
        ("session", include_str!("../src/readers/session.rs")),
    ] {
        assert!(
            source.contains("compatibility_ingest"),
            "{name} reader does not delegate to normalized ingestion"
        );
        for forbidden in ["readers::wire", "read_to_string", "serde_json::from"] {
            assert!(
                !source.contains(forbidden),
                "{name} reader retained an independent raw adapter through {forbidden}"
            );
        }
    }

    let discovery = include_str!("../src/readers/discovery.rs");
    assert!(
        discovery.contains("compatibility_discover"),
        "public discovery does not delegate to normalized transcript discovery"
    );
    for forbidden in ["std::fs", "read_dir", "collect_jsonl_files"] {
        assert!(
            !discovery.contains(forbidden),
            "public discovery retained an independent filesystem walker through {forbidden}"
        );
    }
}

#[test]
fn public_fallible_readers_surface_bounds_coverage_and_safe_errors() {
    const CANARY: &str = "PRIVATE_ERROR_PATH_CANARY_A411";
    let projects_dir = temp_projects_dir("public-normalized-bounds");
    let project_dir = projects_dir.join("private-project");
    fs::create_dir_all(&project_dir).unwrap();

    let mut input = format!(
        "{{\"type\":\"assistant\",\"padding\":\"{}\"}}\n",
        "x".repeat(16 * 1024 * 1024)
    );
    let valid = serde_json::json!({
        "type": "assistant",
        "timestamp": "2026-05-01T10:00:00Z",
        "sessionId": "private-session",
        "message": {
            "id": "bounded-message",
            "model": "claude-sonnet-4-6",
            "usage": { "input_tokens": 3, "output_tokens": 5 }
        }
    });
    input.push_str(&valid.to_string());
    input.push('\n');
    fs::write(project_dir.join("bounded.jsonl"), input).unwrap();

    let (entries, coverage) = try_read_all_jsonl(&projects_dir, Some(2026)).unwrap();
    assert_eq!(
        entries.len(),
        1,
        "later valid records must survive an oversized line"
    );
    assert_eq!(coverage.malformed_records, 1);
    assert!(coverage.warnings.iter().any(|warning| {
        warning.code == "W_TRANSCRIPT_LINE_OVERSIZED"
            && warning.source_alias.as_deref() == Some("transcript-1")
    }));
    assert_eq!(coverage.completeness, "partial");

    let missing = projects_dir.parent().unwrap().join(CANARY);
    let error = try_read_all_jsonl(&missing, Some(2026)).unwrap_err();
    assert_eq!(error.code(), "E_DISCOVERY_TRANSCRIPT_MISSING");
    assert_eq!(error.source_alias(), Some("transcript-1"));
    assert!(!error.message().contains(CANARY));
    assert!(!error.remediation().contains(CANARY));
    assert!(!error.to_string().contains(CANARY));
    assert!(!error.remediation().is_empty());

    fs::remove_dir_all(projects_dir.parent().unwrap()).unwrap();
}

#[test]
fn public_compatibility_readers_reject_invalid_years_without_panicking() {
    let projects_dir = temp_projects_dir("public-invalid-year");
    fs::create_dir_all(&projects_dir).unwrap();

    let jsonl_error = try_read_all_jsonl(&projects_dir, Some(i32::MAX)).unwrap_err();
    assert_eq!(jsonl_error.code(), "E_PERIOD_INVALID");
    assert_eq!(jsonl_error.source_alias(), None);
    assert!(!jsonl_error.message().is_empty());
    assert!(!jsonl_error.remediation().is_empty());

    let session_error = try_read_session_breakdown(&projects_dir, Some(i32::MAX)).unwrap_err();
    assert_eq!(session_error.code(), "E_PERIOD_INVALID");
    assert_eq!(session_error.source_alias(), None);
    assert!(!session_error.message().is_empty());
    assert!(!session_error.remediation().is_empty());

    assert!(read_all_jsonl(&projects_dir, Some(i32::MAX)).is_empty());
    let fallback = read_session_breakdown(&projects_dir, Some(i32::MAX));
    assert!(fallback.sessions.is_empty());
    assert!(fallback.costly_subagents.is_empty());
    assert_eq!(fallback.total_subagent_sessions, 0);

    fs::remove_dir_all(projects_dir.parent().unwrap()).unwrap();
}

#[test]
fn public_compatibility_readers_preserve_all_years_semantics() {
    let projects_dir = temp_projects_dir("public-normalized-all-years");
    let project_dir = projects_dir.join("private-project");
    fs::create_dir_all(&project_dir).unwrap();
    let records = [
        serde_json::json!({
            "type": "assistant",
            "timestamp": "2025-12-31T23:00:00Z",
            "sessionId": "private-session",
            "message": {
                "id": "message-2025",
                "model": "claude-haiku-4-5",
                "usage": { "input_tokens": 1, "output_tokens": 2 }
            }
        }),
        serde_json::json!({
            "type": "assistant",
            "timestamp": "2026-01-01T01:00:00Z",
            "sessionId": "private-session",
            "message": {
                "id": "message-2026",
                "model": "claude-haiku-4-5",
                "usage": { "input_tokens": 3, "output_tokens": 4 }
            }
        }),
    ];
    fs::write(
        project_dir.join("years.jsonl"),
        records
            .iter()
            .map(serde_json::Value::to_string)
            .collect::<Vec<_>>()
            .join("\n"),
    )
    .unwrap();

    let (all_entries, all_coverage) = try_read_all_jsonl(&projects_dir, None).unwrap();
    let (selected_entries, selected_coverage) =
        try_read_all_jsonl(&projects_dir, Some(2026)).unwrap();
    assert_eq!(all_entries.len(), 2);
    assert_eq!(all_coverage.selected_period, "all");
    assert_eq!(selected_entries.len(), 1);
    assert_eq!(selected_entries[0].output_tokens, 4);
    assert_eq!(selected_coverage.selected_period, "2026");
    assert_eq!(selected_coverage.filtered_records, 1);

    fs::remove_dir_all(projects_dir.parent().unwrap()).unwrap();
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
    assert!(session_breakdown.sessions[0]
        .session_id
        .starts_with("session-"));
    assert!(session_breakdown.sessions[0].subagents[0]
        .session_id
        .starts_with("session-"));
    assert_ne!(
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
        schema_version: "ccwrapped.report/v2".to_string(),
        generated_at: "2026-04-06T12:00:00.000Z".to_string(),
        year: 2026,
        data_coverage: Default::default(),
        methodology: Default::default(),
        canonical_metrics: Default::default(),
        insights: Default::default(),
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
    report.canonical_metrics.active_time.days = vec![
        ccwrapped::DailyActiveTime {
            date: "2026-04-05".to_string(),
            active_seconds: 300,
        },
        ccwrapped::DailyActiveTime {
            date: "2026-04-06".to_string(),
            active_seconds: 600,
        },
    ];
    report.wrapped_story = build_wrapped_story(&report, &entries);

    assert_eq!(
        report.wrapped_story.archetype.title,
        "Entertainment · Not enough observed activity"
    );
    assert_eq!(report.wrapped_story.top_tool.as_ref().unwrap().name, "Bash");
    assert_eq!(
        report.wrapped_story.top_project.as_ref().unwrap().name,
        "project-1"
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
        "project-1"
    );
    assert_eq!(json["projectBreakdown"][0]["hash"], "project-1");
    assert!(json["costAnalysis"]["dailyCosts"].is_array());
    assert!(json["cacheHealth"]["savings"]["fromCaching"].is_number());
    assert_eq!(json["wrappedStory"]["hero"].as_array().unwrap().len(), 5);

    let html = render_html(&report);
    assert!(html.contains("Claude Code Wrapped"));
    assert!(html.contains("Largest sessions"));
    assert!(html.contains("Subagent spikes"));
    assert!(html.contains("Next season"));

    let markdown = render_markdown(&report);
    assert!(markdown.contains("## Hero Stats"));
    assert!(markdown.contains("## Highlights"));
    assert!(markdown.contains("## Top Projects"));
    assert!(markdown.contains("## Human vs Tool Prompts"));
    assert!(!markdown.contains("<div"));

    let card = render_share_card(&report);
    assert!(card.contains("API-equivalent estimate"));
    assert!(card.contains("Cache-read share"));
    assert!(card.contains("Power hour"));
    assert!(!card.contains("<script"));
    assert!(!card.contains("demo-app"));

    // Terminal output covers all major sections
    let mut buf = termcolor::Buffer::no_color();
    render_terminal_to(&report, &mut buf);
    let terminal = String::from_utf8_lossy(buf.as_slice()).to_string();
    assert!(terminal.contains("CLAUDE CODE WRAPPED"));
    assert!(terminal.contains("Season stats"));
    assert!(terminal.contains("Activity"));
    assert!(terminal.contains("Cache evidence"));
    assert!(terminal.contains("Model request mix"));
    assert!(terminal.contains("Top projects"));
    assert!(terminal.contains("Largest sessions"));
    assert!(terminal.contains("Human vs tool"));
    assert!(terminal.contains("Highlights"));
    assert!(
        !terminal.contains("Recommendations"),
        "the aggregate-only compatibility helper must not invent advice"
    );
    // Verify sparkline characters are present
    assert!(terminal.chars().any(|c| "▁▂▃▄▅▆▇█".contains(c)));
    // Verify percentage bar characters are present
    assert!(terminal.contains('█'));
}

#[test]
fn renderer_exports_include_coverage_and_limitations() {
    let mut report = Report {
        schema_version: "ccwrapped.report/v2".to_string(),
        generated_at: "2026-04-06T12:00:00Z".to_string(),
        year: 2026,
        ..Default::default()
    };
    report.data_coverage.completeness = "partial".to_string();
    report.data_coverage.source_root_count = 2;
    report.data_coverage.files_discovered = 3;
    report.data_coverage.accepted_records = 4;
    report.data_coverage.retention_caveat =
        "Synthetic retention is incomplete; do not treat this as billing truth.".to_string();
    report.data_coverage.warnings = vec![IngestionWarning {
        code: "W_SYNTHETIC_PARTIAL".to_string(),
        message: "A <script>synthetic</script> source was only partially observed.".to_string(),
        source_alias: Some("transcript-1".to_string()),
    }];

    let partial_html = render_html(&report);
    let partial_markdown = render_markdown(&report);
    for rendered in [&partial_html, &partial_markdown] {
        assert!(rendered.contains("Data coverage"));
        assert!(rendered.contains("partial"));
        assert!(rendered.contains("2 sources"));
        assert!(rendered.contains("3 files"));
        assert!(rendered.contains("4 accepted records"));
        assert!(rendered.contains("Synthetic retention is incomplete"));
        assert!(rendered.contains("W_SYNTHETIC_PARTIAL"));
        assert!(!rendered.contains("<script>synthetic</script>"));
        assert!(rendered.contains("&lt;script&gt;synthetic&lt;/script&gt;"));
    }

    report.data_coverage.completeness = "indeterminate".to_string();
    let indeterminate_html = render_html(&report);
    let indeterminate_markdown = render_markdown(&report);
    assert!(indeterminate_html.contains("indeterminate"));
    assert!(indeterminate_markdown.contains("indeterminate"));
}

#[test]
fn exported_renderers_surface_partial_and_indeterminate_coverage() {
    renderer_exports_include_coverage_and_limitations();
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
