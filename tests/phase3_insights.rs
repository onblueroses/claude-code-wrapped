use chrono::{DateTime, Duration, FixedOffset, NaiveDate};
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct SyntheticHome {
    root: PathBuf,
}

impl SyntheticHome {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-phase3-{label}-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create synthetic Phase 3 home");
        Self { root }
    }

    fn transcript_root(&self) -> PathBuf {
        let root = self.root.join("config/projects");
        fs::create_dir_all(&root).expect("create transcript root");
        root
    }

    fn write_session(&self, root: &Path, lines: &[Value]) {
        self.write_project_session(root, "synthetic-project", "synthetic-session", lines);
    }

    fn write_project_session(
        &self,
        root: &Path,
        project_name: &str,
        session_name: &str,
        lines: &[Value],
    ) {
        let project = root.join(project_name);
        fs::create_dir_all(&project).expect("create synthetic project");
        let body = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            project.join(format!("{session_name}.jsonl")),
            format!("{body}\n"),
        )
        .expect("write synthetic transcript");
    }

    fn write_otel(&self, name: &str, lines: &[Value]) -> PathBuf {
        let path = self.root.join(name);
        let body = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{body}\n")).expect("write synthetic OTel file");
        path
    }

    fn run(&self, root: &Path) -> Output {
        self.run_args(&["--data-dir", root.to_str().unwrap(), "2026"])
    }

    fn run_args(&self, args: &[&str]) -> Output {
        self.command()
            .args(["--json", "--timezone", "UTC"])
            .args(args)
            .output()
            .expect("run ccwrapped")
    }

    fn run_plain_args(&self, args: &[&str]) -> Output {
        self.command()
            .args(["--plain", "--timezone", "UTC"])
            .args(args)
            .output()
            .expect("run ccwrapped")
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_ccwrapped"));
        command
            .current_dir(&self.root)
            .env("HOME", self.root.join("isolated-home"))
            .env("XDG_CACHE_HOME", self.root.join("isolated-cache"))
            .env(
                "CLAUDE_CONFIG_DIR",
                self.root.join("isolated-claude-config"),
            )
            .env("TZ", "Pacific/Honolulu")
            .env("NO_COLOR", "1");
        command
    }

    fn json(&self, root: &Path) -> Value {
        self.successful_json(self.run(root))
    }

    fn otel_json(&self, path: &Path) -> Value {
        self.successful_json(self.run_args(&["--otel-file", path.to_str().unwrap(), "2026"]))
    }

    fn successful_json(&self, output: Output) -> Value {
        assert!(
            output.status.success(),
            "status={}\nstdout={}\nstderr={}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).expect("stdout must be one JSON value")
    }
}

impl Drop for SyntheticHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn assistant(index: usize, date: NaiveDate, output_tokens: u64) -> Value {
    assistant_usage(index, date, 1, output_tokens)
}

fn assistant_usage(index: usize, date: NaiveDate, input_tokens: u64, output_tokens: u64) -> Value {
    serde_json::json!({
        "type": "assistant",
        "sessionId": "synthetic-session",
        "timestamp": format!("{date}T12:00:00Z"),
        "message": {
            "id": format!("message-{index:04}"),
            "model": "claude-sonnet-4-6",
            "usage": {
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            },
            "content": []
        }
    })
}

fn otel_attribute(key: &str, value: Value) -> Value {
    let wrapped = match value {
        Value::String(value) => serde_json::json!({"stringValue": value}),
        Value::Bool(value) => serde_json::json!({"boolValue": value}),
        Value::Number(value) if value.is_i64() || value.is_u64() => {
            serde_json::json!({"intValue": value.to_string()})
        }
        Value::Number(value) => serde_json::json!({"doubleValue": value.as_f64().unwrap()}),
        _ => panic!("synthetic OTel attributes use scalar values"),
    };
    serde_json::json!({"key": key, "value": wrapped})
}

fn unix_nanos(timestamp: &str) -> u64 {
    timestamp
        .parse::<DateTime<FixedOffset>>()
        .unwrap()
        .timestamp_nanos_opt()
        .and_then(|value| u64::try_from(value).ok())
        .unwrap()
}

fn otel_event(event_name: &str, timestamp: &str, attributes: Vec<Value>) -> Value {
    let mut all_attributes = vec![
        otel_attribute("event.timestamp", Value::String(timestamp.to_string())),
        otel_attribute("session.id", Value::String("synthetic-session".to_string())),
    ];
    all_attributes.extend(attributes);
    serde_json::json!({
        "resourceLogs": [{
            "resource": {
                "attributes": [
                    otel_attribute("service.name", Value::String("claude-code".to_string()))
                ]
            },
            "scopeLogs": [{
                "scope": {"name": "com.anthropic.claude_code.events"},
                "logRecords": [{
                    "timeUnixNano": unix_nanos(timestamp).to_string(),
                    "body": {},
                    "attributes": all_attributes,
                    "eventName": event_name
                }]
            }]
        }]
    })
}

fn with_declared_attribute_drop(mut export: Value, entity: &str) -> Value {
    let pointer = match entity {
        "resource" => "/resourceLogs/0/resource",
        "scope" => "/resourceLogs/0/scopeLogs/0/scope",
        "record" => "/resourceLogs/0/scopeLogs/0/logRecords/0",
        _ => panic!("unsupported synthetic dropped-attribute entity"),
    };
    export
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .expect("synthetic OTel entity")
        .insert("droppedAttributesCount".to_string(), Value::from(1));
    export
}

fn api_request(index: usize, attempt: u64, model: &str, output_tokens: u64) -> Value {
    api_request_with_duration(index, attempt, model, output_tokens, 100 + index as u64)
}

fn api_request_without_usage(index: usize, model: &str) -> Value {
    otel_event(
        "claude_code.api_request",
        &format!("2026-07-01T12:{index:02}:00Z"),
        vec![
            otel_attribute("request_id", Value::String(format!("request-{index}"))),
            otel_attribute("model", Value::String(model.to_string())),
            otel_attribute("duration_ms", Value::from(100 + index as u64)),
            otel_attribute("attempt", Value::from(1)),
        ],
    )
}

fn api_request_with_duration(
    index: usize,
    attempt: u64,
    model: &str,
    output_tokens: u64,
    duration_ms: u64,
) -> Value {
    let timestamp = format!("2026-07-01T12:{index:02}:00Z");
    api_request_at(
        index,
        attempt,
        model,
        output_tokens,
        duration_ms,
        &timestamp,
    )
}

fn api_request_at(
    index: usize,
    attempt: u64,
    model: &str,
    output_tokens: u64,
    duration_ms: u64,
    timestamp: &str,
) -> Value {
    otel_event(
        "claude_code.api_request",
        timestamp,
        vec![
            otel_attribute("request_id", Value::String(format!("request-{index}"))),
            otel_attribute("model", Value::String(model.to_string())),
            otel_attribute("input_tokens", Value::from(10)),
            otel_attribute("output_tokens", Value::from(output_tokens)),
            otel_attribute("cache_read_tokens", Value::from(0)),
            otel_attribute("cache_creation_tokens", Value::from(0)),
            otel_attribute("cost_usd", Value::from(0.01)),
            otel_attribute("duration_ms", Value::from(duration_ms)),
            otel_attribute("attempt", Value::from(attempt)),
        ],
    )
}

fn api_error(index: usize, attempt: u64) -> Value {
    let timestamp = format!("2026-07-01T13:{index:02}:00Z");
    otel_event(
        "claude_code.api_error",
        &timestamp,
        vec![
            otel_attribute(
                "request_id",
                Value::String(format!("failed-request-{index}")),
            ),
            otel_attribute("attempt", Value::from(attempt)),
            otel_attribute("duration_ms", Value::from(200 + index as u64)),
        ],
    )
}

fn tool_result(index: usize, tool: &str, success: bool, duration_ms: u64) -> Value {
    let day = 2 + index / (24 * 60);
    let hour = (12 + index / 60) % 24;
    let minute = index % 60;
    let timestamp = format!("2026-07-{day:02}T{hour:02}:{minute:02}:00Z");
    otel_event(
        "claude_code.tool_result",
        &timestamp,
        vec![
            otel_attribute("tool_use_id", Value::String(format!("tool-use-{index}"))),
            otel_attribute("tool_name", Value::String(tool.to_string())),
            otel_attribute("success", Value::Bool(success)),
            otel_attribute("duration_ms", Value::from(duration_ms)),
        ],
    )
}

fn tool_decision(index: usize, decision: &str) -> Value {
    let timestamp = format!("2026-07-02T13:{index:02}:00Z");
    otel_event(
        "claude_code.tool_decision",
        &timestamp,
        vec![
            otel_attribute(
                "tool_use_id",
                Value::String(format!("edit-tool-use-{index}")),
            ),
            otel_attribute("tool_name", Value::String("Edit".to_string())),
            otel_attribute("decision", Value::String(decision.to_string())),
        ],
    )
}

fn otel_token_metric(token_type: &str, start_nanos: u64, end_nanos: u64, value: u64) -> Value {
    otel_token_metric_for_model(
        token_type,
        "claude-sonnet-4-6",
        start_nanos,
        end_nanos,
        value,
    )
}

fn otel_token_metric_for_model(
    token_type: &str,
    model: &str,
    start_nanos: u64,
    end_nanos: u64,
    value: u64,
) -> Value {
    serde_json::json!({
        "resourceMetrics": [{
            "resource": {
                "attributes": [
                    otel_attribute("service.name", Value::String("claude-code".to_string()))
                ]
            },
            "scopeMetrics": [{
                "scope": {"name": "com.anthropic.claude_code"},
                "metrics": [{
                    "name": "claude_code.token.usage",
                    "unit": "tokens",
                    "sum": {
                        "aggregationTemporality": 1,
                        "isMonotonic": true,
                        "dataPoints": [{
                            "attributes": [
                                otel_attribute("type", Value::String(token_type.to_string())),
                                otel_attribute(
                                    "model",
                                    Value::String(model.to_string())
                                )
                            ],
                            "startTimeUnixNano": start_nanos.to_string(),
                            "timeUnixNano": end_nanos.to_string(),
                            "asInt": value.to_string()
                        }]
                    }
                }]
            }]
        }]
    })
}

fn card<'a>(report: &'a Value, id: &str) -> &'a Value {
    report["insights"]["cards"]
        .as_array()
        .expect("insight cards")
        .iter()
        .find(|card| card["id"] == id)
        .unwrap_or_else(|| panic!("missing insight card {id}"))
}

fn fact<'a>(card: &'a Value, metric_id: &str) -> &'a Value {
    card["supportingFacts"]
        .as_array()
        .expect("supporting facts")
        .iter()
        .find(|fact| fact["metricId"] == metric_id)
        .unwrap_or_else(|| panic!("missing supporting fact {metric_id}"))
}

fn family<'a>(report: &'a Value, name: &str) -> &'a Value {
    report["insights"]["families"]
        .as_array()
        .expect("insight families")
        .iter()
        .find(|family| family["family"] == name)
        .unwrap_or_else(|| panic!("missing insight family {name}"))
}

fn card_fact<'a>(card: &'a Value, metric_id: &str) -> &'a Value {
    card["supportingFacts"]
        .as_array()
        .expect("supporting facts")
        .iter()
        .find(|fact| fact["metricId"] == metric_id)
        .unwrap_or_else(|| panic!("missing supporting fact {metric_id}"))
}

#[test]
fn f031_adjacent_comparison_has_exact_windows_delta_and_zero_baseline_semantics() {
    let home = SyntheticHome::new("f031-adjacent");
    let root = home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let lines = (0..56)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                if index < 28 { 10 } else { 20 },
            )
        })
        .collect::<Vec<_>>();
    home.write_session(&root, &lines);

    let report = home.json(&root);
    let comparison = card(&report, "comparison.output-tokens.v1");
    assert_eq!(
        comparison["methodId"],
        "comparison/adjacent-equal-window/v1"
    );
    assert_eq!(comparison["window"]["start"], "2026-03-29T00:00:00Z");
    assert_eq!(comparison["window"]["end"], "2026-04-26T00:00:00Z");
    assert_eq!(comparison["window"]["timezone"], "UTC");
    assert_eq!(comparison["comparison"]["baselineValue"], "280");
    assert_eq!(comparison["comparison"]["currentValue"], "560");
    assert_eq!(comparison["comparison"]["absoluteDelta"], "280");
    assert_eq!(comparison["comparison"]["relativeDeltaPct"], 100.0);
    assert_eq!(comparison["sampleCount"], 56);
    assert_eq!(comparison["minimumSampleCount"], 14);
    assert_eq!(comparison["confidence"], "low");
    assert_eq!(
        comparison["limitations"],
        serde_json::json!(["retention-indeterminate-observed-activity"])
    );
    assert_eq!(family(&report, "comparison")["availability"], "partial");

    let zero_home = SyntheticHome::new("f031-zero");
    let zero_root = zero_home.transcript_root();
    let zero_lines = (0..56)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                if index < 28 { 0 } else { 20 },
            )
        })
        .collect::<Vec<_>>();
    zero_home.write_session(&zero_root, &zero_lines);
    let zero_report = zero_home.json(&zero_root);
    let zero_comparison = card(&zero_report, "comparison.output-tokens.v1");
    assert_eq!(zero_comparison["comparison"]["baselineValue"], "0");
    assert_eq!(zero_comparison["comparison"]["currentValue"], "560");
    assert_eq!(zero_comparison["comparison"]["absoluteDelta"], "560");
    assert!(
        zero_comparison["comparison"]["relativeDeltaPct"].is_null(),
        "a zero baseline cannot produce a relative change"
    );

    let direct_zero_home = SyntheticHome::new("f031-direct-zero");
    let direct_zero_root = direct_zero_home.transcript_root();
    let direct_zero_lines = (0..56)
        .map(|index| {
            assistant_usage(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                u64::from(index >= 28),
                if index < 28 { 0 } else { 20 },
            )
        })
        .collect::<Vec<_>>();
    direct_zero_home.write_session(&direct_zero_root, &direct_zero_lines);
    let direct_zero_report = direct_zero_home.json(&direct_zero_root);
    let direct_zero_comparison = card(&direct_zero_report, "comparison.output-tokens.v1");
    assert_eq!(
        card_fact(direct_zero_comparison, "comparison.prior-active-days")["value"],
        "28",
        "an observed direct all-zero tuple is still activity"
    );
    assert!(
        direct_zero_comparison["supportingFacts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|fact| fact["metricId"] != "comparison.prior-zero-coverage-days"),
        "direct observations use the ordinary active-date gate"
    );
}

#[test]
fn f031_incomplete_metric_day_intervals_cannot_waive_the_prior_active_day_gate() {
    let home = SyntheticHome::new("f031-complete-zero");
    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let mut lines = Vec::new();
    for index in 0..56 {
        let date = start.checked_add_signed(Duration::days(index)).unwrap();
        let next = date.succ_opt().unwrap();
        let interval_start = u64::try_from(
            date.and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_nanos_opt()
                .unwrap(),
        )
        .unwrap();
        let interval_end = u64::try_from(
            next.and_hms_opt(0, 0, 0)
                .unwrap()
                .and_utc()
                .timestamp_nanos_opt()
                .unwrap()
                - 1,
        )
        .unwrap();
        let model = format!("complete-day-{index:02}");
        let output = if index >= 49 { 100 } else { 0 };
        for (token_type, value) in [
            ("input", 0),
            ("output", output),
            ("cacheRead", 0),
            ("cacheCreation", 0),
        ] {
            lines.push(otel_token_metric_for_model(
                token_type,
                &model,
                interval_start,
                interval_end,
                value,
            ));
        }
    }
    let path = home.write_otel("complete-zero.jsonl", &lines);
    let report = home.otel_json(&path);
    assert_eq!(
        report["dataCoverage"]["sources"][0]["completeness"], "complete",
        "a clean file scan is not an exhaustive producer coverage declaration"
    );
    assert_eq!(
        report["dataCoverage"]["sources"][0]["capabilities"]["token_usage"],
        "available"
    );
    assert_eq!(family(&report, "comparison")["availability"], "unavailable");
    assert_eq!(
        family(&report, "comparison")["limitations"],
        serde_json::json!(["comparison-minimum-active-days"])
    );
    assert!(report["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|candidate| candidate["id"] != "comparison.output-tokens.v1"));
}

#[test]
fn f032_partial_or_incompatible_windows_suppress_comparison() {
    let start = NaiveDate::from_ymd_opt(2026, 3, 1).unwrap();
    let partial_home = SyntheticHome::new("f032-partial");
    let partial_root = partial_home.transcript_root();
    let mut partial_lines = (0..56)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                10,
            )
        })
        .collect::<Vec<_>>();
    partial_lines.push(serde_json::json!({
        "type": "future-private-shape",
        "private": "synthetic"
    }));
    partial_home.write_session(&partial_root, &partial_lines);
    let partial_report = partial_home.json(&partial_root);
    assert_eq!(
        family(&partial_report, "comparison")["availability"],
        "unavailable"
    );
    assert_eq!(
        family(&partial_report, "comparison")["limitations"],
        serde_json::json!(["comparison-partial-source"])
    );
    assert!(partial_report["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["family"] != "comparison"));

    let mixed_home = SyntheticHome::new("f032-incompatible");
    let mixed_root = mixed_home.transcript_root();
    let prior = (0..28)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                10,
            )
        })
        .collect::<Vec<_>>();
    mixed_home.write_session(&mixed_root, &prior);
    let current = (28..56)
        .map(|index| {
            let date = start
                .checked_add_signed(Duration::days(index as i64))
                .unwrap();
            let timestamp = format!("{date}T12:00:00Z");
            otel_event(
                "claude_code.api_request",
                &timestamp,
                vec![
                    otel_attribute(
                        "request_id",
                        Value::String(format!("mixed-request-{index}")),
                    ),
                    otel_attribute("model", Value::String("claude-sonnet-4-6".to_string())),
                    otel_attribute("input_tokens", Value::from(1)),
                    otel_attribute("output_tokens", Value::from(20)),
                    otel_attribute("cache_read_tokens", Value::from(0)),
                    otel_attribute("cache_creation_tokens", Value::from(0)),
                ],
            )
        })
        .collect::<Vec<_>>();
    let otel = mixed_home.write_otel("mixed.jsonl", &current);
    let mixed_report = mixed_home.successful_json(mixed_home.run_args(&[
        "--data-dir",
        mixed_root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        family(&mixed_report, "comparison")["availability"],
        "unavailable"
    );
    assert_eq!(
        family(&mixed_report, "comparison")["limitations"],
        serde_json::json!(["comparison-incompatible-coverage"])
    );

    let short_home = SyntheticHome::new("f032-short");
    let short_root = short_home.transcript_root();
    let short = (0..8)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                10,
            )
        })
        .collect::<Vec<_>>();
    short_home.write_session(&short_root, &short);
    let short_report = short_home.json(&short_root);
    assert_eq!(
        family(&short_report, "comparison")["availability"],
        "unavailable"
    );
    assert_eq!(
        family(&short_report, "comparison")["limitations"],
        serde_json::json!(["comparison-window-outside-observed-envelope"])
    );

    let outside_home = SyntheticHome::new("f032-outside-period");
    let outside_root = outside_home.transcript_root();
    outside_home.write_session(
        &outside_root,
        &[assistant(
            0,
            NaiveDate::from_ymd_opt(2026, 1, 20).unwrap(),
            10,
        )],
    );
    let outside = outside_home.json(&outside_root);
    assert_eq!(
        family(&outside, "comparison")["limitations"],
        serde_json::json!(["comparison-window-outside-period"])
    );
}

#[test]
fn f033_median_halves_trend_resists_one_extreme_point() {
    let home = SyntheticHome::new("f033-trend");
    let root = home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let lines = (0..16)
        .map(|index| {
            let output = if index < 8 {
                100
            } else if index == 15 {
                1_000_000
            } else {
                200
            };
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                output,
            )
        })
        .collect::<Vec<_>>();
    home.write_session(&root, &lines);

    let report = home.json(&root);
    let trend = card(&report, "trend.output-tokens.v1");
    assert_eq!(trend["methodId"], "trend/median-halves/v1");
    assert_eq!(trend["comparison"]["baselineValue"], "100");
    assert_eq!(trend["comparison"]["currentValue"], "200");
    assert_eq!(trend["comparison"]["absoluteDelta"], "100");
    assert_eq!(trend["comparison"]["relativeDeltaPct"], 100.0);
    assert_eq!(trend["sampleCount"], 16);
    assert_eq!(trend["minimumSampleCount"], 8);
    assert!(trend["finding"]
        .as_str()
        .unwrap()
        .contains("later daily median rose"));
    assert_eq!(family(&report, "trend")["availability"], "partial");
}

#[test]
fn f033_trend_point_samples_are_daily_points() {
    let home = SyntheticHome::new("f033-daily-point-samples");
    let root = home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let lines = (0..8)
        .flat_map(|day| {
            let output = if day < 4 { 50 } else { 100 };
            (0..2).map(move |observation| {
                assistant(
                    day * 2 + observation,
                    start
                        .checked_add_signed(Duration::days(day as i64))
                        .unwrap(),
                    output,
                )
            })
        })
        .collect::<Vec<_>>();
    home.write_session(&root, &lines);

    let report = home.json(&root);
    let trend = card(&report, "trend.output-tokens.v1");
    assert_eq!(trend["sampleCount"], 8);
    assert_eq!(trend["comparison"]["baselineValue"], "100");
    assert_eq!(trend["comparison"]["currentValue"], "200");
    assert_eq!(fact(trend, "trend.point-count")["value"], "8");
    assert_eq!(fact(trend, "trend.half-size")["value"], "4");
    assert_eq!(
        fact(trend, "trend.first-observed-date")["value"],
        "2026-05-01"
    );
    assert_eq!(
        fact(trend, "trend.last-observed-date")["value"],
        "2026-05-08"
    );
    assert_eq!(fact(trend, "trend.direction-threshold")["value"], "100");
    assert_eq!(fact(trend, "trend.direction")["value"], "rose");
    assert_eq!(fact(trend, "tokens.output.daily-median")["sampleCount"], 4);
    assert_eq!(
        trend["supportingFacts"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|fact| fact["metricId"] == "tokens.output.daily-median")
            .map(|fact| fact["sampleCount"].as_u64().unwrap())
            .collect::<Vec<_>>(),
        vec![4, 4]
    );
}

fn daily_output_report(label: &str, outputs: &[u64]) -> Value {
    let home = SyntheticHome::new(label);
    let root = home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 5, 1).unwrap();
    let lines = outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                *output,
            )
        })
        .collect::<Vec<_>>();
    home.write_session(&root, &lines);
    home.json(&root)
}

#[test]
fn f033_trend_boundaries_cover_flat_falling_zero_minimum_and_recent_cap() {
    let flat = daily_output_report("f033-flat", &[100; 8]);
    let flat_card = card(&flat, "trend.output-tokens.v1");
    assert!(flat_card["finding"]
        .as_str()
        .unwrap()
        .contains("median stable"));
    assert_eq!(flat_card["comparison"]["absoluteDelta"], "0");
    assert_eq!(flat_card["comparison"]["relativeDeltaPct"], 0.0);

    let below_rise =
        daily_output_report("f033-below-rise", &[100, 100, 100, 100, 199, 199, 199, 199]);
    assert!(card(&below_rise, "trend.output-tokens.v1")["finding"]
        .as_str()
        .unwrap()
        .contains("median stable"));

    let exact_rise =
        daily_output_report("f033-exact-rise", &[100, 100, 100, 100, 200, 200, 200, 200]);
    assert!(card(&exact_rise, "trend.output-tokens.v1")["finding"]
        .as_str()
        .unwrap()
        .contains("median rose"));

    let falling = daily_output_report("f033-falling", &[200, 200, 200, 200, 100, 100, 100, 100]);
    let falling_card = card(&falling, "trend.output-tokens.v1");
    assert!(falling_card["finding"]
        .as_str()
        .unwrap()
        .contains("median fell"));
    assert_eq!(falling_card["comparison"]["absoluteDelta"], "-100");
    assert_eq!(falling_card["comparison"]["relativeDeltaPct"], -50.0);

    let zero = daily_output_report("f033-zero", &[0, 0, 0, 0, 100, 100, 100, 100]);
    let zero_card = card(&zero, "trend.output-tokens.v1");
    assert!(zero_card["finding"]
        .as_str()
        .unwrap()
        .contains("median rose"));
    assert_eq!(zero_card["comparison"]["baselineValue"], "0");
    assert!(zero_card["comparison"]["relativeDeltaPct"].is_null());

    let short = daily_output_report("f033-short", &[100; 7]);
    assert_eq!(family(&short, "trend")["availability"], "unavailable");
    assert_eq!(
        family(&short, "trend")["limitations"],
        serde_json::json!(["trend-minimum-points"])
    );

    let capped = daily_output_report("f033-cap", &[100; 30]);
    let capped_card = card(&capped, "trend.output-tokens.v1");
    assert_eq!(capped_card["sampleCount"], 28);
    assert!(capped_card["window"]["start"]
        .as_str()
        .unwrap()
        .starts_with("2026-05-03T"));
}

#[test]
fn f034_active_efficiency_uses_unioned_active_seconds_and_observed_language() {
    let home = SyntheticHome::new("f034-active-rate");
    let root = home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 6, 1).unwrap();
    let lines = (0..6)
        .map(|index| assistant(index, start, 100))
        .enumerate()
        .map(|(index, mut value)| {
            value["timestamp"] = Value::String(format!("2026-06-01T12:{:02}:00Z", index * 5));
            value
        })
        .collect::<Vec<_>>();
    home.write_session(&root, &lines);

    let report = home.json(&root);
    assert_eq!(
        report["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        1500
    );
    let output_rate = card(&report, "efficiency.output-tokens-per-active-hour.v1");
    assert_eq!(
        output_rate["methodId"],
        "efficiency/observed-active-rate/v1"
    );
    assert_eq!(output_rate["sampleCount"], 6);
    assert_eq!(
        card_fact(output_rate, "efficiency.output-tokens-per-active-hour")["value"],
        "1440",
        "600 tokens / 1500 seconds * 3600"
    );
    let request_rate = card(&report, "efficiency.requests-per-active-hour.v1");
    assert_eq!(
        card_fact(request_rate, "efficiency.requests-per-active-hour")["value"],
        "14.4"
    );
    let cost_rate = card(
        &report,
        "efficiency.local-api-equivalent-per-active-hour.v1",
    );
    assert_eq!(cost_rate["coverage"], "available");
    assert_eq!(
        card_fact(cost_rate, "activity.active")["value"],
        "1500",
        "local estimate rate must use the same unioned active denominator"
    );
    assert!(report["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["id"] != "efficiency.terminal-errors-per-active-hour.v1"));
    assert_eq!(
        family(&report, "active-efficiency")["availability"],
        "partial"
    );

    let rendered = serde_json::to_string(&report["insights"])
        .unwrap()
        .to_ascii_lowercase();
    for forbidden in [
        "productivity",
        "productive",
        "quality",
        "efficient worker",
        "better worker",
        "worse worker",
    ] {
        assert!(
            !rendered.contains(forbidden),
            "observed active-rate insight used forbidden evaluative wording: {forbidden}"
        );
    }

    let below_home = SyntheticHome::new("f034-below-active-gate");
    let below_root = below_home.transcript_root();
    let below_lines = (0..3)
        .map(|index| {
            let mut value = assistant(index, start, 100);
            value["timestamp"] = Value::String(format!("2026-06-01T12:{:02}:00Z", index * 5));
            value
        })
        .collect::<Vec<_>>();
    below_home.write_session(&below_root, &below_lines);
    let below = below_home.json(&below_root);
    assert_eq!(
        below["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        600
    );
    assert_eq!(
        family(&below, "active-efficiency")["availability"],
        "unavailable"
    );
    assert_eq!(family(&below, "active-efficiency")["sampleCount"], 3);
    assert_eq!(family(&below, "active-efficiency")["minimumSampleCount"], 5);
    assert!(family(&below, "active-efficiency")["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "efficiency-minimum-active-seconds"));
    assert!(below["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["family"] != "active-efficiency"));
    let below_terminal = below_home.run_plain_args(&[
        "--markdown",
        "--data-dir",
        below_root.to_str().unwrap(),
        "2026",
    ]);
    assert!(below_terminal.status.success());
    let below_markdown =
        fs::read_to_string(below_home.root.join("claude-code-wrapped.md")).unwrap();
    assert!(below_markdown.contains(
        "Insight family · active&#45;efficiency=unavailable · capabilities analysis&#95;usage&#95;totals,analysis&#95;active&#95;time · samples 3/5 · limitations efficiency&#45;minimum&#45;active&#45;seconds"
    ));

    let exact_home = SyntheticHome::new("f034-exact-active-gate");
    let exact_root = exact_home.transcript_root();
    let exact_lines = (0..4)
        .map(|index| {
            let mut value = assistant(index, start, 100);
            value["timestamp"] = Value::String(format!("2026-06-01T12:{:02}:00Z", index * 5));
            value
        })
        .collect::<Vec<_>>();
    exact_home.write_session(&exact_root, &exact_lines);
    let exact = exact_home.json(&exact_root);
    assert_eq!(
        exact["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        900
    );
    assert_eq!(family(&exact, "active-efficiency")["sampleCount"], 4);
    assert_eq!(family(&exact, "active-efficiency")["minimumSampleCount"], 5);
    assert!(exact["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|card| card["id"] == "efficiency.output-tokens-per-active-hour.v1"));
    assert!(exact["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["id"] != "efficiency.requests-per-active-hour.v1"));
    let exact_cost = card(&exact, "efficiency.local-api-equivalent-per-active-hour.v1");
    assert!(
        exact_cost["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .all(|value| value != "efficiency-minimum-request-observations"),
        "an exact cost-rate card must not inherit an unrelated request-rate sample limitation"
    );

    let unpriced_home = SyntheticHome::new("f034-unpriced-cost");
    let unpriced_root = unpriced_home.transcript_root();
    let unpriced_lines = (0..6)
        .map(|index| {
            let mut value = assistant(index, start, 100);
            value["timestamp"] = Value::String(format!("2026-06-01T12:{:02}:00Z", index * 5));
            value["message"]["model"] = Value::String("claude-future-99".to_string());
            value
        })
        .collect::<Vec<_>>();
    unpriced_home.write_session(&unpriced_root, &unpriced_lines);
    let unpriced = unpriced_home.json(&unpriced_root);
    assert!(unpriced["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["id"] != "efficiency.local-api-equivalent-per-active-hour.v1"));
    assert!(family(&unpriced, "active-efficiency")["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "efficiency-local-cost-incomplete"));

    let direct_home = SyntheticHome::new("f034-direct-errors");
    let mut direct_lines = (0..10)
        .map(|index| api_request_with_duration(index, 1, "claude-sonnet-4-6", 10, 600_000))
        .collect::<Vec<_>>();
    direct_lines.push(api_error(0, 2));
    let direct_path = direct_home.write_otel("direct-errors.jsonl", &direct_lines);
    let direct = direct_home.otel_json(&direct_path);
    assert!(
        direct["canonicalMetrics"]["activeTime"]["totalActiveSeconds"]
            .as_u64()
            .unwrap()
            >= 900
    );
    let error_rate = card(&direct, "efficiency.terminal-errors-per-active-hour.v1");
    assert_eq!(card_fact(error_rate, "api.terminal-errors")["value"], "1");
    assert_eq!(
        card_fact(error_rate, "activity.active")["value"],
        direct["canonicalMetrics"]["activeTime"]["totalActiveSeconds"]
            .as_u64()
            .unwrap()
            .to_string()
    );
}

#[test]
fn f035_reliability_uses_terminal_outcomes_and_recovered_retry_evidence() {
    for total in [9usize, 10, 11] {
        let boundary_home = SyntheticHome::new(&format!("f035-terminal-boundary-{total}"));
        let mut boundary_lines = (0..total.saturating_sub(1))
            .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
            .collect::<Vec<_>>();
        boundary_lines.push(api_error(0, 2));
        let boundary_path = boundary_home.write_otel("boundary.jsonl", &boundary_lines);
        let boundary = boundary_home.otel_json(&boundary_path);
        assert_eq!(
            boundary["insights"]["cards"]
                .as_array()
                .unwrap()
                .iter()
                .any(|card| card["id"] == "reliability.api-terminal-error-rate.v1"),
            total >= 10,
            "terminal-outcome sample boundary drifted at {total}"
        );
    }

    for completed in [9usize, 10, 11] {
        let boundary_home = SyntheticHome::new(&format!("f035-retry-boundary-{completed}"));
        let boundary_lines = (0..completed)
            .map(|index| {
                api_request(
                    index,
                    if index == 0 { 2 } else { 1 },
                    "claude-sonnet-4-6",
                    10,
                )
            })
            .collect::<Vec<_>>();
        let boundary_path = boundary_home.write_otel("retry-boundary.jsonl", &boundary_lines);
        let boundary = boundary_home.otel_json(&boundary_path);
        assert_eq!(
            boundary["insights"]["cards"]
                .as_array()
                .unwrap()
                .iter()
                .any(|card| card["id"] == "reliability.api-recovered-retry-rate.v1"),
            completed >= 10,
            "recovered-retry sample boundary drifted at {completed}"
        );
    }

    let home = SyntheticHome::new("f035-terminal");
    let mut lines = (0..9)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    lines.push(api_error(0, 3));
    let path = home.write_otel("terminal.jsonl", &lines);
    let report = home.otel_json(&path);
    let terminal = card(&report, "reliability.api-terminal-error-rate.v1");
    assert_eq!(terminal["sampleCount"], 10);
    assert_eq!(card_fact(terminal, "api.terminal-outcomes")["value"], "10");
    assert_eq!(card_fact(terminal, "api.terminal-errors")["value"], "1");
    assert_eq!(
        card_fact(terminal, "reliability.api-terminal-error-rate")["value"],
        "10"
    );
    assert!(terminal["finding"]
        .as_str()
        .unwrap()
        .contains("after retries were exhausted"));
    assert!(report["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["id"] != "reliability.api-recovered-retry-rate.v1"));

    let retry_home = SyntheticHome::new("f035-retry");
    let retry_lines = (0..10)
        .map(|index| {
            api_request(
                index,
                if index == 0 { 3 } else { 1 },
                "claude-sonnet-4-6",
                10,
            )
        })
        .collect::<Vec<_>>();
    let retry_path = retry_home.write_otel("retry.jsonl", &retry_lines);
    let retry_report = retry_home.otel_json(&retry_path);
    let retry = card(&retry_report, "reliability.api-recovered-retry-rate.v1");
    assert_eq!(retry["sampleCount"], 10);
    assert_eq!(card_fact(retry, "api.recovered-requests")["value"], "1");
    assert_eq!(card_fact(retry, "api.recovered-retry-count")["value"], "2");
    assert_eq!(
        card_fact(retry, "reliability.api-recovered-retry-rate")["value"],
        "10"
    );
    let zero_terminal = card(&retry_report, "reliability.api-terminal-error-rate.v1");
    assert_eq!(
        card_fact(zero_terminal, "api.terminal-errors")["value"],
        "0"
    );
    assert_eq!(
        card_fact(zero_terminal, "reliability.api-terminal-error-rate")["value"],
        "0"
    );

    let transcript_home = SyntheticHome::new("f035-missing");
    let root = transcript_home.transcript_root();
    transcript_home.write_session(
        &root,
        &[assistant(
            0,
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            10,
        )],
    );
    let transcript_report = transcript_home.json(&root);
    assert_eq!(
        family(&transcript_report, "reliability")["availability"],
        "unavailable"
    );
    assert!(transcript_report["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["family"] != "reliability"));
}

#[test]
fn f036_tool_behavior_separates_results_latency_and_edit_decisions() {
    let occurrence_home = SyntheticHome::new("f036-occurrence-only");
    let occurrence_root = occurrence_home.transcript_root();
    let mut occurrence = assistant(0, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(), 10);
    occurrence["message"]["content"] = serde_json::json!([{"type": "tool_use", "name": "Read"}]);
    occurrence_home.write_session(&occurrence_root, &[occurrence]);
    let occurrence_report = occurrence_home.json(&occurrence_root);
    let read = card(&occurrence_report, "tool.Read.observed-outcomes.v1");
    assert_eq!(read["sampleCount"], 1);
    assert_eq!(read["minimumSampleCount"], 1);
    assert_eq!(read["confidence"], "low");
    assert!(read["supportingFacts"]
        .as_array()
        .unwrap()
        .iter()
        .any(|fact| fact["metricId"] == "tool.occurrences" && fact["value"] == "1"));
    assert!(read["supportingFacts"]
        .as_array()
        .unwrap()
        .iter()
        .all(|fact| fact["metricId"] != "tool.direct-results"));

    for results in [4usize, 5] {
        let boundary_home = SyntheticHome::new(&format!("f036-result-boundary-{results}"));
        let boundary_lines = (0..results)
            .map(|index| tool_result(index, "Bash", index > 0, 10 + index as u64))
            .collect::<Vec<_>>();
        let boundary_path = boundary_home.write_otel("results.jsonl", &boundary_lines);
        let boundary = boundary_home.otel_json(&boundary_path);
        let bash = card(&boundary, "tool.Bash.observed-outcomes.v1");
        assert_eq!(
            bash["supportingFacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|fact| fact["metricId"] == "tool.direct-failure-rate"),
            results >= 5
        );
    }

    for decisions in [4usize, 5] {
        let boundary_home = SyntheticHome::new(&format!("f036-decision-boundary-{decisions}"));
        let boundary_lines = (0..decisions)
            .map(|index| tool_decision(index, if index == 0 { "reject" } else { "accept" }))
            .collect::<Vec<_>>();
        let boundary_path = boundary_home.write_otel("decisions.jsonl", &boundary_lines);
        let boundary = boundary_home.otel_json(&boundary_path);
        let edit = card(&boundary, "tool.Edit.observed-outcomes.v1");
        assert_eq!(
            edit["supportingFacts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|fact| fact["metricId"] == "tool.edit-accept-share"),
            decisions >= 5
        );
    }

    let home = SyntheticHome::new("f036-tools");
    let mut lines = [10, 20, 30, 40, 100]
        .into_iter()
        .enumerate()
        .map(|(index, duration)| tool_result(index, "Bash", index != 4, duration))
        .collect::<Vec<_>>();
    lines.extend(
        ["accept", "accept", "accept", "reject", "reject"]
            .into_iter()
            .enumerate()
            .map(|(index, decision)| tool_decision(index, decision)),
    );
    let path = home.write_otel("tools.jsonl", &lines);
    let report = home.otel_json(&path);
    let bash = card(&report, "tool.Bash.observed-outcomes.v1");
    let bash_facts = bash["supportingFacts"].as_array().unwrap();
    let fact_value = |metric: &str| {
        bash_facts
            .iter()
            .find(|fact| fact["metricId"] == metric)
            .unwrap()["value"]
            .as_str()
            .unwrap()
    };
    assert_eq!(fact_value("tool.direct-results"), "5");
    assert_eq!(fact_value("tool.direct-failures"), "1");
    assert_eq!(fact_value("tool.direct-failure-rate"), "20");
    assert_eq!(fact_value("tool.duration-median"), "30");
    assert_eq!(fact_value("tool.duration-p95"), "100");

    let edit = card(&report, "tool.Edit.observed-outcomes.v1");
    let edit_facts = edit["supportingFacts"].as_array().unwrap();
    assert_eq!(
        edit_facts
            .iter()
            .find(|fact| fact["metricId"] == "tool.edit-decisions")
            .unwrap()["value"],
        "5"
    );
    assert_eq!(
        edit_facts
            .iter()
            .find(|fact| fact["metricId"] == "tool.edit-accept-share")
            .unwrap()["value"],
        "60"
    );
    assert_eq!(family(&report, "tool-behavior")["availability"], "partial");

    let ranked_home = SyntheticHome::new("f036-ranking");
    let ranked_tools = [
        "AskUserQuestion",
        "Bash",
        "Edit",
        "Glob",
        "Grep",
        "LS",
        "MultiEdit",
        "NotebookEdit",
        "Read",
        "Task",
        "WebFetch",
    ];
    let ranked_lines = ranked_tools
        .iter()
        .enumerate()
        .map(|(index, tool)| tool_result(index, tool, true, 10))
        .collect::<Vec<_>>();
    let ranked_path = ranked_home.write_otel("ranked.jsonl", &ranked_lines);
    let ranked = ranked_home.otel_json(&ranked_path);
    let tool_cards = ranked["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|card| card["family"] == "tool-behavior")
        .collect::<Vec<_>>();
    assert_eq!(tool_cards.len(), 10);
    assert_eq!(
        tool_cards[0]["id"],
        "tool.AskUserQuestion.observed-outcomes.v1"
    );
    assert!(tool_cards
        .iter()
        .all(|card| card["id"] != "tool.WebFetch.observed-outcomes.v1"));

    let extreme_home = SyntheticHome::new("f036-extreme-duration");
    let extreme = otel_event(
        "claude_code.tool_result",
        "2026-07-02T12:00:00Z",
        vec![
            otel_attribute("tool_use_id", Value::String("extreme-duration".to_string())),
            otel_attribute("tool_name", Value::String("Bash".to_string())),
            otel_attribute("success", Value::Bool(true)),
            otel_attribute("duration_ms", Value::from(1e308)),
        ],
    );
    let extreme_path = extreme_home.write_otel("extreme.jsonl", &[extreme]);
    let extreme_report = extreme_home.otel_json(&extreme_path);
    assert_eq!(
        extreme_report["dataCoverage"]["capabilities"]["tool_latency"],
        "unavailable"
    );
    let extreme_json = serde_json::to_string(&extreme_report)
        .unwrap()
        .to_ascii_lowercase();
    assert!(!extreme_json.contains(r#""value":"inf""#));
    assert!(!extreme_json.contains(r#""value":"-inf""#));
    assert!(!extreme_json.contains(r#""value":"nan""#));
}

#[test]
fn f036_direct_result_does_not_create_occurrence() {
    let home = SyntheticHome::new("f036-result-is-not-occurrence");
    let path = home.write_otel("result.jsonl", &[tool_result(0, "Bash", true, 10)]);
    let report = home.otel_json(&path);
    let bash = card(&report, "tool.Bash.observed-outcomes.v1");
    let facts = bash["supportingFacts"].as_array().unwrap();

    assert!(facts
        .iter()
        .any(|fact| fact["metricId"] == "tool.direct-results" && fact["value"] == "1"));
    assert!(
        facts
            .iter()
            .all(|fact| fact["metricId"] != "tool.occurrences"),
        "a direct result is outcome evidence, not transcript tool-use occurrence evidence"
    );
    assert_eq!(
        report["dataCoverage"]["capabilities"]["tool_occurrence"],
        "unavailable"
    );
    assert_eq!(
        report["dataCoverage"]["capabilities"]["tool_result"],
        "available"
    );
}

#[test]
fn f037_routing_reports_mapped_and_unknown_shares_without_quality_inference() {
    let below_home = SyntheticHome::new("f037-below-minimum");
    let below_lines = (0..4)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    let below_path = below_home.write_otel("below.jsonl", &below_lines);
    let below = below_home.otel_json(&below_path);
    assert_eq!(
        family(&below, "model-routing")["availability"],
        "unavailable"
    );
    assert!(below["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["family"] != "model-routing"));

    let home = SyntheticHome::new("f037-routing");
    let lines = (0..5)
        .map(|index| {
            api_request(
                index,
                1,
                if index == 4 {
                    "claude-sonnet-99-future"
                } else {
                    "claude-sonnet-4-6"
                },
                if index == 4 { 50 } else { 100 },
            )
        })
        .collect::<Vec<_>>();
    let path = home.write_otel("routing.jsonl", &lines);
    let report = home.otel_json(&path);
    let routing = card(&report, "routing.model-request-share.v1");
    assert_eq!(routing["sampleCount"], 5);
    assert_eq!(routing["availability"], "partial");
    let facts = routing["supportingFacts"].as_array().unwrap();
    assert_eq!(
        facts
            .iter()
            .find(|fact| fact["metricId"] == "routing.unknown-model-request-share")
            .unwrap()["value"],
        "20"
    );
    assert!(facts
        .iter()
        .any(|fact| fact["metricId"] == "routing.model-request-share" && fact["value"] == "80"));
    let text = serde_json::to_string(&routing)
        .unwrap()
        .to_ascii_lowercase();
    for forbidden in [
        "wrong model",
        "task intent",
        "model quality",
        "avoidable",
        "savings",
    ] {
        assert!(!text.contains(forbidden), "routing asserted {forbidden}");
    }

    let priced_home = SyntheticHome::new("f037-priced");
    let priced_lines = (0..5)
        .map(|index| {
            api_request(
                index,
                1,
                if index < 3 {
                    "claude-sonnet-4-6"
                } else {
                    "claude-haiku-4-5"
                },
                if index < 3 { 100 } else { 50 },
            )
        })
        .collect::<Vec<_>>();
    let priced_path = priced_home.write_otel("priced.jsonl", &priced_lines);
    let priced = priced_home.otel_json(&priced_path);
    let cost = card(&priced, "routing.model-local-cost-share.v1");
    let cost_facts = cost["supportingFacts"].as_array().unwrap();
    assert_eq!(
        cost_facts
            .iter()
            .filter(|fact| fact["metricId"] == "routing.model-local-cost-share")
            .map(|fact| fact["value"].as_str().unwrap().parse::<f64>().unwrap())
            .sum::<f64>(),
        100.0
    );
    for (metric, expected) in [
        ("cost.priced-requests", "5"),
        ("cost.unpriced-requests", "0"),
        ("cost.unpriced-tokens", "0"),
    ] {
        assert_eq!(
            cost_facts
                .iter()
                .find(|fact| fact["metricId"] == metric)
                .unwrap()["value"],
            expected
        );
    }

    let capped_home = SyntheticHome::new("f037-display-cap-denominators");
    let models = [
        "claude-sonnet-4-6",
        "claude-opus-4-8",
        "claude-opus-4-7",
        "claude-opus-4-6",
        "claude-opus-4-5",
        "claude-opus-4-1",
        "claude-fable-5",
        "claude-sonnet-5",
        "claude-sonnet-4-5",
        "claude-mythos-5",
        "claude-haiku-4-5",
    ];
    let mut capped_lines = vec![
        api_request(0, 1, models[0], 100),
        api_request(1, 1, models[0], 100),
    ];
    capped_lines.extend(
        models
            .iter()
            .enumerate()
            .skip(1)
            .map(|(index, model)| api_request(index + 1, 1, model, 100)),
    );
    let capped_path = capped_home.write_otel("capped-routing.jsonl", &capped_lines);
    let capped = capped_home.otel_json(&capped_path);
    let request = card(&capped, "routing.model-request-share.v1");
    let output = card(&capped, "routing.model-output-token-share.v1");
    let cost = card(&capped, "routing.model-local-cost-share.v1");
    for (card, tail_metric) in [
        (request, "routing.other-mapped-request-share"),
        (output, "routing.other-mapped-output-token-share"),
        (cost, "routing.other-mapped-local-cost-share"),
    ] {
        let facts = card["supportingFacts"].as_array().unwrap();
        let shares = facts
            .iter()
            .filter(|fact| {
                fact["metricId"]
                    .as_str()
                    .is_some_and(|metric| metric.ends_with("-share"))
                    && fact["unit"] == "percent"
            })
            .map(|fact| fact["value"].as_str().unwrap().parse::<f64>().unwrap())
            .sum::<f64>();
        assert!((shares - 100.0).abs() < 0.000_01, "{tail_metric} drifted");
        assert!(facts.iter().any(|fact| {
            fact["metricId"] == tail_metric
                && fact["value"].as_str().unwrap().parse::<f64>().unwrap() > 0.0
        }));
    }
}

#[test]
fn f037_model_mapping_without_token_evidence() {
    let home = SyntheticHome::new("f037-model-without-token-evidence");
    let lines = (0..20)
        .map(|index| api_request_without_usage(index, "claude-sonnet-4-6"))
        .collect::<Vec<_>>();
    let path = home.write_otel("model-only.jsonl", &lines);
    let report = home.otel_json(&path);
    let request = card(&report, "routing.model-request-share.v1");
    let facts = request["supportingFacts"].as_array().unwrap();

    assert_eq!(
        report["dataCoverage"]["capabilities"]["analysis_usage_totals"], "unavailable",
        "the fixture must isolate model identity from token availability"
    );
    assert_eq!(request["sampleCount"], 20);
    assert_eq!(request["availability"], "available");
    assert_eq!(request["coverage"], "complete-canonical-usage");
    assert!(request["limitations"].as_array().unwrap().is_empty());
    assert!(facts.iter().any(|fact| {
        fact["metricId"] == "routing.model-request-share" && fact["value"] == "100"
    }));
    assert!(facts.iter().any(|fact| {
        fact["metricId"] == "routing.unknown-model-request-share" && fact["value"] == "0"
    }));
    assert!(report["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["id"] != "routing.model-output-token-share.v1"));
    assert_eq!(family(&report, "model-routing")["availability"], "partial");
    assert!(recommendation_ids(&report).contains(&"recommendation.model-routing-experiment.v1"));
}

#[test]
fn f038_project_concentration_uses_known_output_hhi_and_safe_aliases() {
    let one_home = SyntheticHome::new("f038-one");
    let one_root = one_home.transcript_root();
    one_home.write_project_session(
        &one_root,
        "project-a",
        "session-a",
        &[assistant(
            0,
            NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            100,
        )],
    );
    let one_report = one_home.json(&one_root);
    let one = card(&one_report, "concentration.project-output-hhi.v1");
    let one_facts = one["supportingFacts"].as_array().unwrap();
    assert_eq!(one["availability"], "partial");
    assert!(one["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "retention-indeterminate-observed-activity"));
    assert_eq!(
        one_facts
            .iter()
            .find(|fact| fact["metricId"] == "concentration.project-output-hhi")
            .unwrap()["value"],
        "10000"
    );
    assert_eq!(
        one_facts
            .iter()
            .find(|fact| fact["metricId"] == "concentration.known-output-weight")
            .unwrap()["value"],
        "100"
    );
    assert!(one_facts
        .iter()
        .all(|fact| fact["metricId"] != "concentration.top-project-alias"));
    let one_alias = card(&one_report, "concentration.top-project-alias.v1");
    assert_eq!(one_alias["privacyClass"], "standard");
    assert_eq!(
        card_fact(one_alias, "concentration.top-project-alias")["value"],
        "project-1"
    );

    let many_home = SyntheticHome::new("f038-many");
    let many_root = many_home.transcript_root();
    for index in 0..10 {
        many_home.write_project_session(
            &many_root,
            &format!("project-{index:02}"),
            &format!("session-{index:02}"),
            &[serde_json::json!({
                "type": "assistant",
                "sessionId": format!("session-{index:02}"),
                "timestamp": format!("2026-07-{:02}T12:00:00Z", index + 1),
                "message": {
                    "id": format!("message-{index:02}"),
                    "model": "claude-sonnet-4-6",
                    "usage": {
                        "input_tokens": 1,
                        "output_tokens": 100,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0
                    },
                    "content": []
                }
            })],
        );
    }
    let many_report = many_home.json(&many_root);
    let many = card(&many_report, "concentration.project-output-hhi.v1");
    assert_eq!(
        many["supportingFacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|fact| fact["metricId"] == "concentration.project-output-hhi")
            .unwrap()["value"],
        "1000"
    );
    assert!(many["finding"].as_str().unwrap().contains("distributed"));

    let threshold_home = SyntheticHome::new("f038-hhi-threshold");
    let threshold_root = threshold_home.transcript_root();
    for index in 0..4 {
        threshold_home.write_project_session(
            &threshold_root,
            &format!("threshold-project-{index}"),
            &format!("threshold-session-{index}"),
            &[assistant(
                index,
                NaiveDate::from_ymd_opt(2026, 7, index as u32 + 1).unwrap(),
                100,
            )],
        );
    }
    let threshold = threshold_home.json(&threshold_root);
    let threshold_card = card(&threshold, "concentration.project-output-hhi.v1");
    assert!(threshold_card["finding"]
        .as_str()
        .unwrap()
        .contains("concentrated"));
    assert_eq!(
        threshold_card["supportingFacts"]
            .as_array()
            .unwrap()
            .iter()
            .find(|fact| fact["metricId"] == "concentration.project-output-hhi")
            .unwrap()["value"],
        "2500"
    );
}

#[test]
fn f039_anomalies_use_median_mad_and_the_mad_zero_guard() {
    let small = daily_output_report("f039-small", &[100; 6]);
    assert_eq!(family(&small, "anomaly")["availability"], "unavailable");
    assert_eq!(
        family(&small, "anomaly")["limitations"],
        serde_json::json!(["anomaly-minimum-points"])
    );

    let positive_mad =
        daily_output_report("f039-positive-mad", &[90, 95, 100, 105, 110, 115, 1000]);
    let positive_card = card(&positive_mad, "anomaly.output-tokens.2026-05-07.v1");
    let positive_facts = positive_card["supportingFacts"].as_array().unwrap();
    assert_eq!(
        positive_facts
            .iter()
            .find(|fact| fact["metricId"] == "anomaly.baseline-median")
            .unwrap()["value"],
        "105"
    );
    assert_eq!(
        positive_facts
            .iter()
            .find(|fact| fact["metricId"] == "anomaly.baseline-mad")
            .unwrap()["value"],
        "10"
    );
    assert_ne!(
        positive_facts
            .iter()
            .find(|fact| fact["metricId"] == "anomaly.robust-score")
            .unwrap()["value"],
        "unavailable"
    );

    let guarded = daily_output_report("f039-practical-guard", &[100, 100, 101, 101, 102, 102, 150]);
    assert!(guarded["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["family"] != "anomaly"));
    assert_eq!(family(&guarded, "anomaly")["availability"], "available");
    assert!(family(&guarded, "anomaly")["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "anomaly-no-point-crossed-threshold"));

    let capped = daily_output_report(
        "f039-cap",
        &[90, 95, 100, 105, 110, 115, 120, 1000, 1100, 1200, 1300],
    );
    let anomaly_ids = capped["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|card| card["family"] == "anomaly")
        .map(|card| card["id"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(anomaly_ids.len(), 3);
    assert_eq!(
        anomaly_ids,
        vec![
            "anomaly.output-tokens.2026-05-11.v1",
            "anomaly.output-tokens.2026-05-10.v1",
            "anomaly.output-tokens.2026-05-09.v1",
        ]
    );

    let home = SyntheticHome::new("f039-mad-zero");
    let root = home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let lines = (0..8)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                if index == 7 { 2_000 } else { 100 },
            )
        })
        .collect::<Vec<_>>();
    home.write_session(&root, &lines);
    let report = home.json(&root);
    let anomaly = card(&report, "anomaly.output-tokens.2026-08-08.v1");
    let facts = anomaly["supportingFacts"].as_array().unwrap();
    assert_eq!(
        facts
            .iter()
            .find(|fact| fact["metricId"] == "anomaly.baseline-median")
            .unwrap()["value"],
        "100"
    );
    assert_eq!(
        facts
            .iter()
            .find(|fact| fact["metricId"] == "anomaly.baseline-mad")
            .unwrap()["value"],
        "0"
    );
    assert_eq!(
        facts
            .iter()
            .find(|fact| fact["metricId"] == "anomaly.robust-score")
            .unwrap()["value"],
        "unavailable"
    );
    assert!(anomaly["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|value| value == "anomaly-mad-zero-fallback"));
    assert!(anomaly["finding"]
        .as_str()
        .unwrap()
        .contains("unusual within observed activity"));
}

fn recommendation_ids(report: &Value) -> Vec<&str> {
    report["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|card| card["class"] == "recommendation")
        .map(|card| card["id"].as_str().unwrap())
        .collect()
}

fn decode_numeric_entities(value: &str) -> String {
    let mut decoded = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(index) = remaining.find("&#") {
        decoded.push_str(&remaining[..index]);
        let entity = &remaining[index + 2..];
        let Some(end) = entity.find(';') else {
            decoded.push_str(&remaining[index..]);
            return decoded;
        };
        let digits = &entity[..end];
        if let Ok(codepoint) = digits.parse::<u32>() {
            if let Some(character) = char::from_u32(codepoint) {
                decoded.push(character);
                remaining = &entity[end + 1..];
                continue;
            }
        }
        decoded.push_str("&#");
        remaining = entity;
    }
    decoded.push_str(remaining);
    decoded
}

#[test]
fn f040_recommendation_rules_obey_below_at_and_above_thresholds() {
    for (errors, expected) in [(0, false), (1, true), (2, true)] {
        let home = SyntheticHome::new(&format!("f040-api-{errors}"));
        let mut lines = (0..10usize.saturating_sub(errors))
            .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
            .collect::<Vec<_>>();
        lines.extend((0..errors).map(|index| api_error(index, 2)));
        let path = home.write_otel("api.jsonl", &lines);
        let report = home.otel_json(&path);
        assert_eq!(
            recommendation_ids(&report).contains(&"recommendation.api-terminal-errors.v1"),
            expected,
            "terminal-error rule mismatch at {errors}/10"
        );
    }

    for (failures, expected) in [(1, false), (2, true), (3, true)] {
        let home = SyntheticHome::new(&format!("f040-tool-{failures}"));
        let lines = (0..10)
            .map(|index| tool_result(index, "Bash", index >= failures, 10 + index as u64))
            .collect::<Vec<_>>();
        let path = home.write_otel("tool.jsonl", &lines);
        let report = home.otel_json(&path);
        assert_eq!(
            recommendation_ids(&report).contains(&"recommendation.tool-result-errors.v1"),
            expected,
            "tool-result rule mismatch at {failures}/10"
        );
    }

    for (top_count, expected) in [(15, false), (16, true), (17, true)] {
        let home = SyntheticHome::new(&format!("f040-routing-{top_count}"));
        let lines = (0..20)
            .map(|index| {
                api_request(
                    index,
                    1,
                    if index < top_count {
                        "claude-sonnet-4-6"
                    } else {
                        "claude-haiku-4-5"
                    },
                    10,
                )
            })
            .collect::<Vec<_>>();
        let path = home.write_otel("routing.jsonl", &lines);
        let report = home.otel_json(&path);
        assert_eq!(
            recommendation_ids(&report).contains(&"recommendation.model-routing-experiment.v1"),
            expected,
            "routing rule mismatch at {top_count}/20"
        );
    }

    let home = SyntheticHome::new("f040-proof");
    let mut lines = (0..9)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    lines.push(api_error(0, 2));
    let path = home.write_otel("proof.jsonl", &lines);
    let report = home.otel_json(&path);
    let recommendation = card(&report, "recommendation.api-terminal-errors.v1");
    assert_eq!(recommendation["class"], "recommendation");
    assert_eq!(recommendation["sampleCount"], 10);
    assert_eq!(recommendation["minimumSampleCount"], 10);
    assert_eq!(recommendation["coverage"], "complete-direct-otel");
    assert_eq!(
        card_fact(recommendation, "reference.card")["value"],
        "reliability.api-terminal-error-rate.v1"
    );
    let recommendation_facts = recommendation["supportingFacts"].as_array().unwrap();
    for (metric, expected) in [
        ("api.terminal-errors", "1"),
        ("api.terminal-outcomes", "10"),
        ("reliability.api-terminal-error-rate", "10"),
        ("recommendation.threshold", "10"),
    ] {
        assert_eq!(
            recommendation_facts
                .iter()
                .find(|fact| fact["metricId"] == metric)
                .unwrap()["value"],
            expected
        );
    }
    assert!(!recommendation["action"]["experiment"]
        .as_str()
        .unwrap()
        .is_empty());
    assert_eq!(
        recommendation["action"]["alternativeExplanations"]
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let partial_home = SyntheticHome::new("f040-partial-trigger");
    let mut partial_lines = (0..20)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    partial_lines.push(otel_event(
        "claude_code.future_private_event",
        "2026-07-01T14:00:00Z",
        vec![otel_attribute(
            "private.future",
            Value::String("synthetic".to_string()),
        )],
    ));
    let partial_path = partial_home.write_otel("partial-trigger.jsonl", &partial_lines);
    let partial = partial_home.otel_json(&partial_path);
    assert_eq!(partial["dataCoverage"]["completeness"], "partial");
    assert!(!recommendation_ids(&partial).contains(&"recommendation.model-routing-experiment.v1"));

    let malformed_home = SyntheticHome::new("f040-malformed-direct-trigger");
    let mut malformed_lines = (0..9)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    malformed_lines.push(api_error(0, 2));
    let malformed_path = malformed_home.write_otel("malformed-trigger.jsonl", &malformed_lines);
    let mut body = fs::read_to_string(&malformed_path).unwrap();
    body.push_str("{malformed-synthetic-json\n");
    fs::write(&malformed_path, body).unwrap();
    let malformed = malformed_home.otel_json(&malformed_path);
    assert_eq!(
        malformed["dataCoverage"]["capabilities"]["direct_terminal_outcomes"],
        "partial"
    );
    assert!(
        !recommendation_ids(&malformed).contains(&"recommendation.api-terminal-errors.v1"),
        "a partial direct terminal-outcome denominator must suppress the recommendation"
    );
    assert_eq!(
        family(&malformed, "recommendation")["availability"],
        "unavailable"
    );

    let capped_tool_home = SyntheticHome::new("f040-tool-candidate-below-display-cap");
    let tools = [
        "AskUserQuestion",
        "Bash",
        "Edit",
        "Glob",
        "Grep",
        "LS",
        "MultiEdit",
        "NotebookEdit",
        "Read",
        "Task",
        "WebFetch",
    ];
    let mut capped_tool_lines = Vec::new();
    for (tool_index, tool) in tools.iter().enumerate() {
        for result_index in 0..10 {
            let failed = if *tool == "WebFetch" {
                true
            } else {
                result_index < 2
            };
            capped_tool_lines.push(tool_result(
                tool_index * 10 + result_index,
                tool,
                !failed,
                10,
            ));
        }
    }
    let capped_tool_path = capped_tool_home.write_otel("tool-candidate.jsonl", &capped_tool_lines);
    let capped_tool = capped_tool_home.otel_json(&capped_tool_path);
    let recommendation = card(&capped_tool, "recommendation.tool-result-errors.v1");
    assert!(recommendation["finding"]
        .as_str()
        .unwrap()
        .contains("WebFetch"));
    let reference = recommendation["supportingFacts"]
        .as_array()
        .unwrap()
        .iter()
        .find(|fact| fact["metricId"] == "reference.card")
        .unwrap()["value"]
        .as_str()
        .unwrap();
    assert!(capped_tool["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .any(|candidate| candidate["id"] == reference && candidate["class"] == "factual"));
}

#[test]
fn f041_insight_narratives_exclude_unsupported_causes_and_fixed_savings() {
    let home = SyntheticHome::new("f041-narratives");
    let mut lines = (0..16)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 100))
        .collect::<Vec<_>>();
    lines.extend((0..4).map(|index| api_error(index, 2)));
    lines.extend((0..10).map(|index| tool_result(index, "Bash", index >= 3, 10 + index as u64)));
    let path = home.write_otel("narratives.jsonl", &lines);
    let report = home.otel_json(&path);
    let forbidden = [
        "throttling",
        "throttle",
        "cache reset",
        "cache invalidation",
        "wasted spend",
        "actual spend",
        "guaranteed savings",
        "fixed savings",
        "usage reduction",
        "productivity",
        "wrongly routed",
    ];
    let assert_clean = |surface: &str, text: &str| {
        let text = text.to_ascii_lowercase();
        for forbidden in forbidden {
            assert!(
                !text.contains(forbidden),
                "{surface} emitted unsupported narrative: {forbidden}"
            );
        }
    };
    assert_clean("canonical JSON", &serde_json::to_string(&report).unwrap());

    let rendered = home.run_plain_args(&["--all", "--otel-file", path.to_str().unwrap(), "2026"]);
    assert!(
        rendered.status.success(),
        "status={}\nstdout={}\nstderr={}",
        rendered.status,
        String::from_utf8_lossy(&rendered.stdout),
        String::from_utf8_lossy(&rendered.stderr)
    );
    let terminal = String::from_utf8(rendered.stdout).unwrap();
    assert_clean("terminal", &terminal);
    assert_clean("stderr", &String::from_utf8_lossy(&rendered.stderr));
    let html = fs::read_to_string(home.root.join("claude-code-wrapped.html")).unwrap();
    let markdown = decode_numeric_entities(
        &fs::read_to_string(home.root.join("claude-code-wrapped.md")).unwrap(),
    );
    let share = fs::read_to_string(home.root.join("claude-code-wrapped-card.html")).unwrap();
    for (surface, text) in [
        ("HTML", html.as_str()),
        ("Markdown", markdown.as_str()),
        ("share card", share.as_str()),
    ] {
        assert_clean(surface, text);
    }
    for card in report["insights"]["cards"].as_array().unwrap() {
        let Some(action) = card["action"].as_object() else {
            continue;
        };
        let id = card["id"].as_str().unwrap();
        let experiment = format!(
            "Insight experiment · {id} · {}",
            action["experiment"].as_str().unwrap()
        );
        for (surface, text) in [
            ("terminal", terminal.as_str()),
            ("HTML", html.as_str()),
            ("Markdown", markdown.as_str()),
        ] {
            assert!(
                text.contains(&experiment),
                "{surface} omitted recommendation experiment for {id}"
            );
        }
        for (index, alternative) in action["alternativeExplanations"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
        {
            let line = format!(
                "Insight alternative · {id} · {} · {}",
                index + 1,
                alternative.as_str().unwrap()
            );
            for (surface, text) in [
                ("terminal", terminal.as_str()),
                ("HTML", html.as_str()),
                ("Markdown", markdown.as_str()),
            ] {
                assert!(
                    text.contains(&line),
                    "{surface} omitted recommendation alternative for {id}"
                );
            }
        }
        assert!(
            !share.contains(&experiment),
            "share exposed standard-only recommendation action for {id}"
        );
    }

    fn collect_sources(root: &Path, extension: &str, files: &mut Vec<PathBuf>) {
        let mut entries = fs::read_dir(root)
            .expect("read source directory")
            .map(|entry| entry.expect("read source entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect_sources(&path, extension, files);
            } else if path.extension().and_then(|candidate| candidate.to_str()) == Some(extension) {
                files.push(path);
            }
        }
    }

    fn unsupported_documentation_paragraph(documentation: &str) -> Option<String> {
        documentation.split("\n\n").find_map(|paragraph| {
            let paragraph = paragraph
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_ascii_lowercase()
                .replace('`', "");
            let permits_recommendation = [
                "recommendation may",
                "recommendations may",
                "recommendation can",
                "recommendations can",
                "recommendation should",
                "recommendations should",
            ]
            .iter()
            .any(|phrase| paragraph.contains(phrase));
            let names_unsupported_action_or_outcome = [
                "cache invalidation",
                "cache reset",
                "fixed savings",
                "guaranteed savings",
                "usage reduction",
                "productivity gain",
                "causal savings",
            ]
            .iter()
            .any(|phrase| paragraph.contains(phrase));
            let conflates_request_and_output_coverage = paragraph
                .contains("request and output shares use the analysis_usage_totals capability");
            let conflates_all_routing_with_token_coverage = paragraph
                .contains("comparison, trend, routing, concentration, and anomaly construction")
                && paragraph.contains("sources that actually contributed token usage");
            (permits_recommendation && names_unsupported_action_or_outcome
                || conflates_request_and_output_coverage
                || conflates_all_routing_with_token_coverage)
                .then_some(paragraph)
        })
    }

    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut source_files = Vec::new();
    collect_sources(&repository.join("src"), "rs", &mut source_files);
    assert!(!source_files.is_empty(), "source narrative scan was empty");
    for file in source_files {
        assert_clean(
            &file.display().to_string(),
            &fs::read_to_string(&file).expect("read source narrative templates"),
        );
    }

    let mut documentation_files = Vec::new();
    collect_sources(&repository.join("docs"), "md", &mut documentation_files);
    documentation_files.push(repository.join("README.md"));
    documentation_files.sort();
    documentation_files.dedup();
    assert!(
        documentation_files.contains(&repository.join("README.md")),
        "documentation narrative scan must include the repository README"
    );
    assert!(
        !documentation_files.is_empty(),
        "documentation narrative scan was empty"
    );
    assert!(
        unsupported_documentation_paragraph(
            "Recommendations may promise fixed savings from observed cache shares."
        )
        .is_some(),
        "documentation guard must reject a synthetic forbidden README claim"
    );
    for file in documentation_files {
        let documentation =
            fs::read_to_string(&file).expect("read checked methodology documentation");
        if let Some(paragraph) = unsupported_documentation_paragraph(&documentation) {
            panic!(
                "{} advertised an unsupported insight claim: {paragraph}",
                file.display()
            );
        }
    }

    let version = Command::new("tesseract")
        .arg("--version")
        .output()
        .expect("F041 requires Tesseract 5.x");
    assert!(version.status.success(), "read Tesseract version");
    let version = String::from_utf8_lossy(&version.stdout);
    assert!(
        version
            .lines()
            .next()
            .is_some_and(|line| line.starts_with("tesseract 5.")),
        "F041 requires Tesseract 5.x, observed {version}"
    );

    let manifest = fs::read_to_string(repository.join("assets/README-ASSETS.sha256"))
        .expect("read asset pins");
    let mut screenshots = manifest
        .lines()
        .filter_map(|line| line.split_once("  ").map(|(_, path)| path))
        .filter(|path| path.starts_with("assets/") && path.ends_with(".png"))
        .collect::<Vec<_>>();
    screenshots.sort_unstable();
    assert_eq!(
        screenshots,
        vec![
            "assets/cache-slide.png",
            "assets/data-slide.png",
            "assets/hero-slide.png",
            "assets/share-card.png",
            "assets/spend-slide.png",
        ],
        "F041 must scan every checked screenshot"
    );
    for screenshot in screenshots {
        let output = Command::new("tesseract")
            .arg(repository.join(screenshot))
            .arg("stdout")
            .args(["--psm", "6"])
            .env("OMP_THREAD_LIMIT", "1")
            .output()
            .expect("extract checked screenshot text");
        assert!(
            output.status.success(),
            "Tesseract failed for {screenshot}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let text = String::from_utf8_lossy(&output.stdout);
        let canary = match screenshot {
            "assets/cache-slide.png" => "cache evidence",
            "assets/data-slide.png" => "request",
            "assets/hero-slide.png" => "entertainment",
            "assets/share-card.png" => "equivalent estimate",
            "assets/spend-slide.png" => "api-equivalent",
            _ => unreachable!("checked screenshot set is exact"),
        };
        assert!(
            text.to_ascii_lowercase().contains(canary),
            "OCR canary {canary:?} was absent from {screenshot}: {text}"
        );
        assert_clean(screenshot, &text);
    }
}

#[test]
fn f042_partial_telemetry_preserves_supported_facts_and_family_absence() {
    let transcript_home = SyntheticHome::new("f042-transcript");
    let transcript_root = transcript_home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 9, 1).unwrap();
    let transcript_lines = (0..20)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                100,
            )
        })
        .collect::<Vec<_>>();
    transcript_home.write_session(&transcript_root, &transcript_lines);
    let transcript = transcript_home.json(&transcript_root);
    assert_eq!(
        transcript["canonicalMetrics"]["tokens"]["global"]["output"]["observed"],
        2_000
    );
    assert_eq!(
        family(&transcript, "reliability")["availability"],
        "unavailable"
    );
    assert_eq!(
        family(&transcript, "tool-behavior")["availability"],
        "unavailable"
    );
    assert_ne!(family(&transcript, "trend")["availability"], "unavailable");
    assert_eq!(
        family(&transcript, "model-routing")["availability"],
        "partial"
    );
    assert!(
        card(&transcript, "routing.model-request-share.v1")["limitations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "retention-indeterminate-observed-activity")
    );
    assert!(
        !recommendation_ids(&transcript).contains(&"recommendation.model-routing-experiment.v1"),
        "indeterminate retained history cannot trigger routing advice"
    );
    assert_eq!(
        family(&transcript, "recommendation")["availability"],
        "unavailable"
    );

    let event_home = SyntheticHome::new("f042-events");
    let event_lines = (0..5)
        .map(|index| tool_result(index, "Read", true, 10 + index as u64))
        .collect::<Vec<_>>();
    let event_path = event_home.write_otel("events.jsonl", &event_lines);
    let events = event_home.otel_json(&event_path);
    assert_ne!(
        family(&events, "tool-behavior")["availability"],
        "unavailable"
    );
    assert_eq!(family(&events, "trend")["availability"], "unavailable");
    assert_eq!(
        family(&events, "model-routing")["availability"],
        "unavailable"
    );

    let metric_home = SyntheticHome::new("f042-metrics");
    let start_nanos = unix_nanos("2026-10-01T00:00:00Z");
    let end_nanos = unix_nanos("2026-10-01T01:00:00Z");
    let metric_path = metric_home.write_otel(
        "metrics.jsonl",
        &[
            otel_token_metric("input", start_nanos, end_nanos, 100),
            otel_token_metric("output", start_nanos, end_nanos, 200),
            otel_token_metric("cacheRead", start_nanos, end_nanos, 0),
            otel_token_metric("cacheCreation", start_nanos, end_nanos, 0),
        ],
    );
    let metrics = metric_home.otel_json(&metric_path);
    assert_eq!(
        metrics["canonicalMetrics"]["tokens"]["global"]["output"]["observed"],
        200
    );
    assert_eq!(
        family(&metrics, "reliability")["availability"],
        "unavailable"
    );
    assert_eq!(
        family(&metrics, "tool-behavior")["availability"],
        "unavailable"
    );

    let partial_home = SyntheticHome::new("f042-partial-otel");
    let mut partial_lines = (0..10)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    partial_lines.push(otel_event(
        "claude_code.future_private_event",
        "2026-07-01T14:00:00Z",
        vec![otel_attribute(
            "private.future",
            Value::String("synthetic".to_string()),
        )],
    ));
    let partial_path = partial_home.write_otel("partial.jsonl", &partial_lines);
    let partial = partial_home.otel_json(&partial_path);
    assert_eq!(
        partial["canonicalMetrics"]["tokens"]["global"]["output"]["observed"],
        100
    );
    assert_eq!(partial["dataCoverage"]["completeness"], "partial");
    assert_ne!(
        family(&partial, "reliability")["availability"],
        "unavailable",
        "an unsupported named event cannot weaken complete direct API-event evidence"
    );
    assert_eq!(
        partial["dataCoverage"]["capabilities"]["direct_terminal_outcomes"],
        "available"
    );
    assert_ne!(
        family(&partial, "model-routing")["availability"],
        "unavailable"
    );

    let malformed_home = SyntheticHome::new("f042-malformed-otel");
    let malformed_path = malformed_home.write_otel(
        "malformed.jsonl",
        &(0..10)
            .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
            .collect::<Vec<_>>(),
    );
    let mut malformed_body = fs::read_to_string(&malformed_path).unwrap();
    malformed_body.push_str("{malformed-synthetic-json\n");
    fs::write(&malformed_path, malformed_body).unwrap();
    let malformed = malformed_home.otel_json(&malformed_path);
    assert_eq!(
        malformed["dataCoverage"]["capabilities"]["direct_terminal_outcomes"],
        "partial"
    );
    assert_eq!(
        family(&malformed, "reliability")["availability"],
        "unavailable"
    );

    let overlap_home = SyntheticHome::new("f042-mixed-overlap");
    let overlap_root = overlap_home.transcript_root();
    let mut overlap_transcript = assistant(0, NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(), 100);
    overlap_transcript["requestId"] = Value::String("request-0".to_string());
    overlap_home.write_session(&overlap_root, &[overlap_transcript]);
    let overlap_otel = overlap_home.write_otel(
        "overlap.jsonl",
        &[api_request(0, 1, "claude-sonnet-4-6", 20)],
    );
    let overlap = overlap_home.successful_json(overlap_home.run_args(&[
        "--data-dir",
        overlap_root.to_str().unwrap(),
        "--otel-file",
        overlap_otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        overlap["canonicalMetrics"]["tokens"]["global"]["output"]["observed"], 20,
        "mixed authority double-counted the correlated request"
    );
    assert_eq!(overlap["dataCoverage"]["resolvedOverlapRecords"], 1);

    let unrelated_home = SyntheticHome::new("f042-mixed-unrelated-malformed");
    let unrelated_root = unrelated_home.transcript_root();
    let unrelated_start = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
    let unrelated_transcript = (0..16)
        .map(|index| {
            assistant(
                index,
                unrelated_start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                if index < 8 { 100 } else { 200 },
            )
        })
        .collect::<Vec<_>>();
    unrelated_home.write_session(&unrelated_root, &unrelated_transcript);
    let unrelated_otel = unrelated_home.write_otel(
        "unrelated.jsonl",
        &(0..5)
            .map(|index| tool_result(index, "Read", true, 10))
            .collect::<Vec<_>>(),
    );
    let mut unrelated_body = fs::read_to_string(&unrelated_otel).unwrap();
    unrelated_body.push_str("{malformed-unrelated-json\n");
    fs::write(&unrelated_otel, unrelated_body).unwrap();
    let unrelated = unrelated_home.successful_json(unrelated_home.run_args(&[
        "--data-dir",
        unrelated_root.to_str().unwrap(),
        "--otel-file",
        unrelated_otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        unrelated["canonicalMetrics"]["tokens"]["global"]["output"]["observed"],
        2_400
    );
    assert_ne!(
        family(&unrelated, "trend")["availability"],
        "unavailable",
        "an unrelated malformed tool source suppressed exact transcript usage facts"
    );
    assert_ne!(
        family(&unrelated, "anomaly")["availability"],
        "unavailable",
        "an unrelated malformed tool source suppressed exact transcript daily facts"
    );

    for report in [
        &transcript,
        &events,
        &metrics,
        &partial,
        &overlap,
        &unrelated,
    ] {
        let families = report["insights"]["families"].as_array().unwrap();
        assert_eq!(families.len(), 10);
        assert!(families.iter().all(|family| {
            matches!(
                family["availability"].as_str(),
                Some("available" | "partial" | "unavailable")
            )
        }));
    }
}

#[test]
fn dropped_attributes_only_weaken_their_event_family() {
    let unrelated_home = SyntheticHome::new("dropped-attributes-unrelated-prompt");
    let mut unrelated_lines = (0..10)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    unrelated_lines.extend((0..5).map(|index| tool_result(index, "Read", true, 10)));
    unrelated_lines.push(with_declared_attribute_drop(
        otel_event(
            "claude_code.user_prompt",
            "2026-07-03T12:00:00Z",
            Vec::new(),
        ),
        "record",
    ));
    let unrelated_path = unrelated_home.write_otel("unrelated-prompt-drop.jsonl", &unrelated_lines);
    let unrelated = unrelated_home.otel_json(&unrelated_path);
    for capability in [
        "direct_terminal_outcomes",
        "retry_evidence",
        "tool_status",
        "tool_latency",
        "analysis_usage_totals",
        "analysis_cost",
    ] {
        assert_eq!(
            unrelated["dataCoverage"]["capabilities"][capability], "available",
            "a dropped user-prompt attribute weakened unrelated {capability} evidence"
        );
    }
    assert_ne!(
        family(&unrelated, "reliability")["availability"],
        "unavailable"
    );
    assert_ne!(
        family(&unrelated, "tool-behavior")["availability"],
        "unavailable"
    );
    assert_ne!(
        family(&unrelated, "model-routing")["availability"],
        "unavailable"
    );
    assert!(unrelated["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_UPSTREAM_DROPPED_ATTRIBUTES"));

    let api_home = SyntheticHome::new("dropped-attributes-api-scope");
    let mut api_lines = (0..10)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    let first_api = api_lines.remove(0);
    api_lines.insert(0, with_declared_attribute_drop(first_api, "scope"));
    api_lines.extend((0..5).map(|index| tool_result(index, "Read", true, 10)));
    let api_path = api_home.write_otel("api-scope-drop.jsonl", &api_lines);
    let api = api_home.otel_json(&api_path);
    assert_eq!(
        api["dataCoverage"]["capabilities"]["direct_terminal_outcomes"], "available",
        "event-family occurrence remains directly observed"
    );
    for capability in ["retry_evidence", "analysis_usage_totals", "analysis_cost"] {
        assert_eq!(
            api["dataCoverage"]["capabilities"][capability], "partial",
            "an inherited API-family drop did not weaken {capability}"
        );
    }
    for capability in ["tool_status", "tool_latency"] {
        assert_eq!(
            api["dataCoverage"]["capabilities"][capability], "available",
            "an API-family drop weakened unrelated {capability} evidence"
        );
    }

    let tool_home = SyntheticHome::new("dropped-attributes-tool-resource");
    let mut tool_lines = (0..10)
        .map(|index| api_request(index, 1, "claude-sonnet-4-6", 10))
        .collect::<Vec<_>>();
    let mut tool_events = (0..5)
        .map(|index| tool_result(index, "Read", true, 10))
        .collect::<Vec<_>>();
    let first_tool = tool_events.remove(0);
    tool_events.insert(0, with_declared_attribute_drop(first_tool, "resource"));
    tool_lines.extend(tool_events);
    let tool_path = tool_home.write_otel("tool-resource-drop.jsonl", &tool_lines);
    let tool = tool_home.otel_json(&tool_path);
    for capability in [
        "direct_terminal_outcomes",
        "retry_evidence",
        "analysis_usage_totals",
        "analysis_cost",
    ] {
        assert_eq!(
            tool["dataCoverage"]["capabilities"][capability], "available",
            "a tool-family drop weakened unrelated {capability} evidence"
        );
    }
    for capability in ["tool_status", "tool_latency"] {
        assert_eq!(
            tool["dataCoverage"]["capabilities"][capability], "partial",
            "a tool-family drop did not weaken {capability}"
        );
    }
}

#[test]
fn entertainment_labels_are_sample_gated_marked_and_deterministic() {
    let start = NaiveDate::from_ymd_opt(2026, 11, 1).unwrap();
    let sparse_home = SyntheticHome::new("entertainment-sparse");
    let sparse_root = sparse_home.transcript_root();
    let sparse_lines = (0..19)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days((index / 4) as i64))
                    .unwrap(),
                10,
            )
        })
        .collect::<Vec<_>>();
    sparse_home.write_session(&sparse_root, &sparse_lines);
    let sparse = sparse_home.json(&sparse_root);
    assert_eq!(
        family(&sparse, "entertainment")["availability"],
        "unavailable"
    );
    assert!(sparse["wrappedStory"]["archetype"]["title"]
        .as_str()
        .unwrap()
        .starts_with("Entertainment · Not enough observed activity"));
    assert!(sparse["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|card| card["class"] != "entertainment"));

    let sparse_days_home = SyntheticHome::new("entertainment-sparse-days");
    let sparse_days_root = sparse_days_home.transcript_root();
    let sparse_days_lines = (0..20)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days((index / 5) as i64))
                    .unwrap(),
                10,
            )
        })
        .collect::<Vec<_>>();
    sparse_days_home.write_session(&sparse_days_root, &sparse_days_lines);
    let sparse_days = sparse_days_home.json(&sparse_days_root);
    assert_eq!(
        family(&sparse_days, "entertainment")["availability"],
        "unavailable"
    );

    let ready_home = SyntheticHome::new("entertainment-ready");
    let ready_root = ready_home.transcript_root();
    let ready_lines = (0..20)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days((index / 4) as i64))
                    .unwrap(),
                10,
            )
        })
        .collect::<Vec<_>>();
    ready_home.write_session(&ready_root, &ready_lines);
    let ready = ready_home.json(&ready_root);
    let entertainment = card(&ready, "entertainment.archetype.v1");
    assert_eq!(entertainment["class"], "entertainment");
    assert_eq!(entertainment["sampleCount"], 20);
    assert_eq!(entertainment["minimumSampleCount"], 20);
    assert_eq!(entertainment["title"], "Entertainment · The Specialist");
    assert_eq!(
        card_fact(
            card(&ready, "entertainment.cache-mood.v1"),
            "cache.read-share"
        )["value"],
        "0",
        "an exact zero cache share must remain a zero entertainment input"
    );
    assert!(entertainment["title"]
        .as_str()
        .unwrap()
        .starts_with("Entertainment · "));
    for field in ["archetype", "cacheMood", "momentum"] {
        assert!(
            ready["wrappedStory"][field]["title"]
                .as_str()
                .unwrap()
                .starts_with("Entertainment · "),
            "{field} lacked a visible entertainment marker"
        );
    }
    let repeated = ready_home.json(&ready_root);
    assert_eq!(ready["insights"], repeated["insights"]);
    assert_eq!(
        ready["wrappedStory"]["archetype"],
        repeated["wrappedStory"]["archetype"]
    );

    let toolsmith_home = SyntheticHome::new("entertainment-toolsmith");
    let toolsmith_root = toolsmith_home.transcript_root();
    let toolsmith_lines = (0..20)
        .map(|index| {
            let mut value = assistant(
                index,
                start
                    .checked_add_signed(Duration::days((index / 2) as i64))
                    .unwrap(),
                if index < 10 { 10 } else { 200 },
            );
            value["message"]["usage"]["cache_read_input_tokens"] = Value::from(10);
            value["message"]["content"] = serde_json::json!([{"type": "tool_use", "name": "Read"}]);
            value
        })
        .collect::<Vec<_>>();
    toolsmith_home.write_session(&toolsmith_root, &toolsmith_lines);
    let toolsmith = toolsmith_home.json(&toolsmith_root);
    assert_eq!(
        card(&toolsmith, "entertainment.archetype.v1")["title"],
        "Entertainment · The Toolsmith"
    );
    assert_eq!(
        toolsmith["wrappedStory"]["cacheMood"]["title"],
        "Entertainment · Cache cartographer"
    );
    assert_eq!(
        toolsmith["wrappedStory"]["momentum"]["title"],
        "Entertainment · Momentum label unavailable",
        "a partial trend cannot activate momentum entertainment"
    );

    let partial_home = SyntheticHome::new("entertainment-partial");
    let partial_root = partial_home.transcript_root();
    let mut partial_lines = (0..20)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days((index / 4) as i64))
                    .unwrap(),
                10,
            )
        })
        .collect::<Vec<_>>();
    partial_lines.push(serde_json::json!({
        "type": "future-private-shape",
        "private": "synthetic"
    }));
    partial_home.write_session(&partial_root, &partial_lines);
    let partial = partial_home.json(&partial_root);
    assert_eq!(partial["dataCoverage"]["completeness"], "partial");
    let partial_archetype = card(&partial, "entertainment.archetype.v1");
    assert_eq!(partial_archetype["class"], "entertainment");
    assert!(partial_archetype["title"]
        .as_str()
        .unwrap()
        .starts_with("Entertainment · "));
    assert!(partial["insights"]["cards"]
        .as_array()
        .unwrap()
        .iter()
        .all(|candidate| candidate["id"] != "entertainment.momentum.v1"));

    let complete_home = SyntheticHome::new("entertainment-complete-momentum");
    let complete_lines = (0..20)
        .map(|index| {
            let date = start
                .checked_add_signed(Duration::days((index / 2) as i64))
                .unwrap();
            api_request_at(
                index,
                1,
                "claude-sonnet-4-6",
                if index < 10 { 10 } else { 200 },
                10,
                &format!("{date}T12:{:02}:00Z", index % 60),
            )
        })
        .collect::<Vec<_>>();
    let complete_path = complete_home.write_otel("complete.jsonl", &complete_lines);
    let complete = complete_home.otel_json(&complete_path);
    assert_eq!(
        card(&complete, "trend.output-tokens.v1")["availability"],
        "available"
    );
    assert_eq!(
        card(&complete, "entertainment.momentum.v1")["title"],
        "Entertainment · Observed momentum"
    );

    let explorer_home = SyntheticHome::new("entertainment-explorer");
    let explorer_root = explorer_home.transcript_root();
    for project in 0..10 {
        let lines = (0..2)
            .map(|offset| {
                let index = project * 2 + offset;
                assistant(
                    index,
                    start
                        .checked_add_signed(Duration::days(project as i64))
                        .unwrap(),
                    10,
                )
            })
            .collect::<Vec<_>>();
        explorer_home.write_project_session(
            &explorer_root,
            &format!("explorer-project-{project:02}"),
            &format!("explorer-session-{project:02}"),
            &lines,
        );
    }
    let explorer = explorer_home.json(&explorer_root);
    assert_eq!(
        card(&explorer, "entertainment.archetype.v1")["title"],
        "Entertainment · The Explorer"
    );

    let orchestrator_home = SyntheticHome::new("entertainment-orchestrator-tie");
    let orchestrator_root = orchestrator_home.transcript_root();
    let orchestrator_main = (0..14)
        .map(|index| {
            let mut value = assistant(
                index,
                start
                    .checked_add_signed(Duration::days((index / 4) as i64))
                    .unwrap(),
                10,
            );
            value["message"]["content"] = serde_json::json!([{"type": "tool_use", "name": "Read"}]);
            value
        })
        .collect::<Vec<_>>();
    orchestrator_home.write_session(&orchestrator_root, &orchestrator_main);
    let subagent_directory = orchestrator_root.join("project/session/subagents");
    fs::create_dir_all(&subagent_directory).unwrap();
    let orchestrator_lines = (14..20)
        .map(|index| {
            let mut value = assistant(
                index,
                start
                    .checked_add_signed(Duration::days((index / 4) as i64))
                    .unwrap(),
                10,
            );
            value["message"]["content"] = serde_json::json!([{"type": "tool_use", "name": "Read"}]);
            value.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        subagent_directory.join("subagent.jsonl"),
        format!("{orchestrator_lines}\n"),
    )
    .unwrap();
    let orchestrator = orchestrator_home.json(&orchestrator_root);
    assert_eq!(
        card(&orchestrator, "entertainment.archetype.v1")["title"],
        "Entertainment · The Orchestrator",
        "the exact 30% subagent threshold must win the documented tie with Toolsmith"
    );
}

#[test]
fn insight_facts_reconcile_across_terminal_html_markdown_and_share_card() {
    let home = SyntheticHome::new("renderer-insights");
    let root = home.transcript_root();
    let start = NaiveDate::from_ymd_opt(2026, 12, 1).unwrap();
    let lines = (0..16)
        .map(|index| {
            assistant(
                index,
                start
                    .checked_add_signed(Duration::days(index as i64))
                    .unwrap(),
                if index < 8 { 100 } else { 200 },
            )
        })
        .collect::<Vec<_>>();
    home.write_session(&root, &lines);
    let output = home.run_plain_args(&["--all", "--data-dir", root.to_str().unwrap(), "2026"]);
    assert!(
        output.status.success(),
        "status={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let terminal = String::from_utf8(output.stdout).unwrap();
    let html = fs::read_to_string(home.root.join("claude-code-wrapped.html")).unwrap();
    let markdown = decode_numeric_entities(
        &fs::read_to_string(home.root.join("claude-code-wrapped.md")).unwrap(),
    );
    let share = fs::read_to_string(home.root.join("claude-code-wrapped-card.html")).unwrap();
    assert!(share.contains("min-height: 1920px"));
    assert!(share.contains("overflow-y: auto"));
    assert!(
        !share.contains("overflow: hidden"),
        "share HTML must not silently clip its complete proof ledger"
    );
    let report = home.json(&root);
    let expected = "Insight fact · insight.fact.trend.output.earlier-median · tokens.output.daily-median=100 tokens · trend/median-halves/v1 · samples 8 · coverage indeterminate";
    for (renderer, text) in [
        ("terminal", terminal.as_str()),
        ("html", html.as_str()),
        ("markdown", markdown.as_str()),
        ("share", share.as_str()),
    ] {
        assert!(
            text.contains(expected),
            "{renderer} omitted or changed the common trend fact"
        );
        assert!(
            text.contains("Insight family · reliability=unavailable"),
            "{renderer} hid a missing direct-telemetry family"
        );
    }
    for card in report["insights"]["cards"].as_array().unwrap() {
        if card["availability"] == "unavailable" {
            continue;
        }
        let context = format!(
            "Insight · {} · {} · {} · samples {}/{} · availability {} · coverage {} · confidence {} · privacy {} · {} to {} ({})",
            card["id"].as_str().unwrap(),
            card["class"].as_str().unwrap(),
            card["methodId"].as_str().unwrap(),
            card["sampleCount"],
            card["minimumSampleCount"],
            card["availability"].as_str().unwrap(),
            card["coverage"].as_str().unwrap(),
            card["confidence"].as_str().unwrap(),
            card["privacyClass"].as_str().unwrap(),
            card["window"]["start"].as_str().unwrap(),
            card["window"]["end"].as_str().unwrap(),
            card["window"]["timezone"].as_str().unwrap(),
        );
        for (renderer, text) in [
            ("terminal", terminal.as_str()),
            ("html", html.as_str()),
            ("markdown", markdown.as_str()),
        ] {
            assert!(
                text.contains(&context),
                "{renderer} omitted card context for {}",
                card["id"]
            );
        }
        let title = format!(
            "Insight title · {} · {}",
            card["id"].as_str().unwrap(),
            card["title"].as_str().unwrap(),
        );
        let finding = format!(
            "Insight finding · {} · {}",
            card["id"].as_str().unwrap(),
            card["finding"].as_str().unwrap(),
        );
        for (renderer, text) in [
            ("terminal", terminal.as_str()),
            ("html", html.as_str()),
            ("markdown", markdown.as_str()),
        ] {
            assert!(
                text.contains(&title),
                "{renderer} omitted canonical title for {}",
                card["id"]
            );
            assert!(
                text.contains(&finding),
                "{renderer} omitted canonical finding for {}",
                card["id"]
            );
        }
        for fact in card["supportingFacts"].as_array().unwrap() {
            let line = format!(
                "Insight fact · {} · {}={} {} · {} · samples {} · coverage {} · source {} · {} to {} ({})",
                fact["id"].as_str().unwrap(),
                fact["metricId"].as_str().unwrap(),
                fact["value"].as_str().unwrap(),
                fact["unit"].as_str().unwrap(),
                fact["methodId"].as_str().unwrap(),
                fact["sampleCount"],
                fact["coverage"].as_str().unwrap(),
                fact["source"].as_str().unwrap(),
                fact["window"]["start"].as_str().unwrap(),
                fact["window"]["end"].as_str().unwrap(),
                fact["window"]["timezone"].as_str().unwrap(),
            );
            for (renderer, text) in [
                ("terminal", terminal.as_str()),
                ("html", html.as_str()),
                ("markdown", markdown.as_str()),
            ] {
                assert!(
                    text.contains(&line),
                    "{renderer} omitted fact {}",
                    fact["id"]
                );
            }
            if card["privacyClass"] == "share" {
                assert!(
                    share.contains(&line),
                    "share omitted privacy-eligible fact {}",
                    fact["id"]
                );
            }
        }
        if card["privacyClass"] == "share" {
            assert!(
                share.contains(&context),
                "share omitted privacy-eligible card context for {}",
                card["id"]
            );
            assert!(
                share.contains(&title),
                "share omitted privacy-eligible title for {}",
                card["id"]
            );
            assert!(
                share.contains(&finding),
                "share omitted privacy-eligible finding for {}",
                card["id"]
            );
        }
    }
    assert!(share.contains("concentration.project-output-hhi"));
    assert!(share.contains("concentration.top-known-project-share"));
    assert!(!share.contains("top-project-alias"));
    assert!(!share.contains("project-1"));
}

#[test]
fn f040_renderer_projection_is_exact_complete_and_privacy_filtered() {
    insight_facts_reconcile_across_terminal_html_markdown_and_share_card();
}
