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
            "ccwrapped-phase1-{label}-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create synthetic home");
        Self { root }
    }

    fn transcript_root(&self, name: &str) -> PathBuf {
        let root = self.root.join(name).join("projects");
        fs::create_dir_all(&root).expect("create transcript root");
        root
    }

    fn write_session(&self, root: &Path, project: &str, session: &str, lines: &[Value]) {
        let project_dir = root.join(project);
        fs::create_dir_all(&project_dir).expect("create synthetic project");
        let body = lines
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(
            project_dir.join(format!("{session}.jsonl")),
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
        fs::write(&path, format!("{body}\n")).expect("write synthetic OTel artifact");
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_in(args, &self.root)
    }

    fn run_in(&self, args: &[&str], current_dir: &Path) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
            .args(["--timezone", "UTC"])
            .args(args)
            .current_dir(current_dir)
            .env("HOME", self.root.join("isolated-home"))
            .env("XDG_CACHE_HOME", self.root.join("isolated-cache"))
            .env(
                "CLAUDE_CONFIG_DIR",
                self.root.join("isolated-claude-config"),
            )
            .env("NO_COLOR", "1")
            .output()
            .expect("run ccwrapped")
    }

    fn config_dir(&self) -> PathBuf {
        self.root.join("isolated-claude-config")
    }

    fn default_projects_dir(&self) -> PathBuf {
        self.root
            .join("isolated-home")
            .join(".claude")
            .join("projects")
    }
}

impl Drop for SyntheticHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(target_os = "linux")]
#[test]
fn linux_binary_does_not_import_glibc_renameat2_wrapper() {
    let output = Command::new("objdump")
        .args(["-T", env!("CARGO_BIN_EXE_ccwrapped")])
        .output()
        .expect("objdump must inspect the Linux test binary");
    assert!(output.status.success());
    let symbols = String::from_utf8(output.stdout).expect("objdump output must be UTF-8");
    assert!(
        !symbols
            .lines()
            .any(|line| line.split_whitespace().last() == Some("renameat2")),
        "Linux binary imported glibc's versioned renameat2 wrapper"
    );
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn browser_launch_failure_is_actionable_after_outputs_commit() {
    const PATH_CANARY: &str = "PRIVATE_BROWSER_SOURCE_CANARY_5E21";
    let home = SyntheticHome::new("browser-launch-failure");
    let root = home.transcript_root(PATH_CANARY);
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:01:00Z",
            2,
        )],
    );
    let output_dir = home.root.join("browser-output");
    let empty_path = home.root.join("empty-path");
    fs::create_dir(&output_dir).unwrap();
    fs::create_dir(&empty_path).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
        .args([
            "--html",
            "--open",
            "--plain",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ])
        .current_dir(&output_dir)
        .env("HOME", home.root.join("isolated-home"))
        .env("XDG_CACHE_HOME", home.root.join("isolated-cache"))
        .env(
            "CLAUDE_CONFIG_DIR",
            home.root.join("isolated-claude-config"),
        )
        .env("NO_COLOR", "1")
        .env("PATH", &empty_path)
        .output()
        .expect("run ccwrapped with no browser launcher");

    assert!(
        !output.status.success(),
        "an explicitly requested browser launch failure returned success"
    );
    assert!(output_dir.join("claude-code-wrapped.html").is_file());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E_BROWSER_OPEN"));
    assert!(stderr.contains("output files were committed"));
    assert!(!stderr.contains(PATH_CANARY));
}

fn assistant(session: &str, message: &str, timestamp: &str, output_tokens: u64) -> Value {
    serde_json::json!({
        "type": "assistant",
        "sessionId": session,
        "timestamp": timestamp,
        "message": {
            "id": message,
            "model": "claude-sonnet-4-6",
            "usage": {
                "input_tokens": 1,
                "output_tokens": output_tokens,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0
            },
            "content": []
        }
    })
}

fn user_prompt(session: &str, message: &str, timestamp: &str, content: &str) -> Value {
    serde_json::json!({
        "type": "user",
        "sessionId": session,
        "timestamp": timestamp,
        "message": {
            "id": message,
            "content": content
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
        _ => panic!("test attributes use scalar OTLP values"),
    };
    serde_json::json!({"key": key, "value": wrapped})
}

fn otel_api_request(
    session: &str,
    request: &str,
    timestamp: &str,
    unix_nanos: u64,
    output_tokens: u64,
    extra_attributes: Vec<Value>,
) -> Value {
    let mut attributes = vec![
        otel_attribute("event.timestamp", Value::String(timestamp.to_string())),
        otel_attribute("session.id", Value::String(session.to_string())),
        otel_attribute("request_id", Value::String(request.to_string())),
        otel_attribute("model", Value::String("claude-sonnet-4-6".to_string())),
        otel_attribute("input_tokens", serde_json::json!(2)),
        otel_attribute("output_tokens", serde_json::json!(output_tokens)),
        otel_attribute("cache_read_tokens", serde_json::json!(3)),
        otel_attribute("cache_creation_tokens", serde_json::json!(4)),
        otel_attribute("cost_usd", serde_json::json!(0.02)),
        otel_attribute("duration_ms", serde_json::json!(125)),
    ];
    attributes.extend(extra_attributes);
    serde_json::json!({
        "resourceLogs": [{
            "resource": {
                "attributes": [
                    otel_attribute("service.name", Value::String("claude-code".to_string())),
                    otel_attribute("user.email", Value::String("PRIVATE_EMAIL_CANARY@example.test".to_string()))
                ]
            },
            "scopeLogs": [{
                "scope": {"name": "com.anthropic.claude_code.events"},
                "logRecords": [{
                    "timeUnixNano": unix_nanos.to_string(),
                    "body": {},
                    "attributes": attributes,
                    "eventName": "claude_code.api_request"
                }]
            }]
        }]
    })
}

fn otel_token_metric(
    session: &str,
    token_type: &str,
    temporality: u64,
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
                        "aggregationTemporality": temporality,
                        "isMonotonic": true,
                        "dataPoints": [{
                            "attributes": [
                                otel_attribute("session.id", Value::String(session.to_string())),
                                otel_attribute("type", Value::String(token_type.to_string())),
                                otel_attribute("model", Value::String("claude-sonnet-4-6".to_string()))
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

fn otel_metric_wire_unit(name: &str) -> &'static str {
    match name {
        "claude_code.session.count"
        | "claude_code.lines_of_code.count"
        | "claude_code.pull_request.count"
        | "claude_code.commit.count"
        | "claude_code.code_edit_tool.decision" => "count",
        "claude_code.cost.usage" => "USD",
        "claude_code.token.usage" => "tokens",
        "claude_code.active_time.total" => "s",
        _ => panic!("test metric must use a pinned Claude metric name"),
    }
}

fn use_pinned_otel_integer_strings(value: &mut Value) {
    match value {
        Value::Array(values) => {
            for value in values {
                use_pinned_otel_integer_strings(value);
            }
        }
        Value::Object(values) => {
            for (key, value) in values {
                if matches!(
                    key.as_str(),
                    "intValue"
                        | "asInt"
                        | "timeUnixNano"
                        | "observedTimeUnixNano"
                        | "startTimeUnixNano"
                ) {
                    if let Value::Number(number) = value {
                        *value = Value::String(number.to_string());
                    }
                } else {
                    use_pinned_otel_integer_strings(value);
                }
            }
        }
        _ => {}
    }
}

fn successful_json(output: Output) -> Value {
    assert!(
        output.status.success(),
        "status={}\nstdout={}\nstderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout must be one JSON value")
}

fn hex_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(value.len().saturating_mul(2));
    for byte in value.bytes() {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn percent_encode(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::with_capacity(value.len().saturating_mul(3));
    for byte in value.bytes() {
        encoded.push('%');
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    encoded
}

fn base64_encode(value: &str) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::new();
    for chunk in value.as_bytes().chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0x03) << 4) | (second >> 4)) as usize] as char);
        output.push(if chunk.len() > 1 {
            TABLE[(((second & 0x0f) << 2) | (third >> 6)) as usize] as char
        } else {
            '='
        });
        output.push(if chunk.len() > 2 {
            TABLE[(third & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    output
}

#[test]
fn repeatable_explicit_roots_feed_one_report_in_command_order() {
    let home = SyntheticHome::new("multi-root");
    let first = home.transcript_root("first-config");
    let second = home.transcript_root("second-config");
    home.write_session(
        &first,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    home.write_session(
        &second,
        "project-beta",
        "session-b",
        &[assistant(
            "session-b",
            "message-b",
            "2026-04-05T10:00:00Z",
            20,
        )],
    );

    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        first.to_str().unwrap(),
        "--data-dir",
        second.to_str().unwrap(),
        "2026",
    ]));

    assert_eq!(json["schemaVersion"], "ccwrapped.report/v2");
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 30);
    assert_eq!(json["dataCoverage"]["sourceRootCount"], 2);
    assert_eq!(json["dataCoverage"]["filesDiscovered"], 2);
    assert_eq!(
        json["dataCoverage"]["sources"][0]["selection"],
        "explicit-projects"
    );
    let aliases = json["dataCoverage"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|source| source["alias"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(aliases, ["transcript-1", "transcript-2"]);
}

#[test]
fn standard_report_excludes_content_paths_and_raw_identifiers() {
    const PATH_CANARY: &str = "PRIVATE_PATH_CANARY_91B8";
    const SESSION_CANARY: &str = "PRIVATE_SESSION_CANARY_47C2";
    const MESSAGE_CANARY: &str = "PRIVATE_MESSAGE_CANARY_A9D4";
    const PROMPT_CANARY: &str = "PRIVATE_PROMPT_CANARY_E37A";
    const TOOL_CANARY: &str = "PRIVATE_TOOL_ARGUMENT_CANARY_F611";

    let home = SyntheticHome::new("privacy");
    let root = home.transcript_root("privacy-config");
    home.write_session(
        &root,
        &format!("project-{PATH_CANARY}"),
        "session-file",
        &[
            serde_json::json!({
                "type": "user",
                "sessionId": SESSION_CANARY,
                "cwd": format!("/synthetic/{PATH_CANARY}"),
                "timestamp": "2026-04-05T09:00:00Z",
                "message": {"content": PROMPT_CANARY}
            }),
            serde_json::json!({
                "type": "assistant",
                "sessionId": SESSION_CANARY,
                "cwd": format!("/synthetic/{PATH_CANARY}"),
                "timestamp": "2026-04-05T09:01:00Z",
                "message": {
                    "id": MESSAGE_CANARY,
                    "model": "claude-sonnet-4-6",
                    "usage": {"input_tokens": 1, "output_tokens": 2},
                    "content": [{
                        "type": "tool_use",
                        "name": "Bash",
                        "input": {"command": TOOL_CANARY}
                    }]
                }
            }),
        ],
    );

    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);

    for canary in [
        PATH_CANARY,
        SESSION_CANARY,
        MESSAGE_CANARY,
        PROMPT_CANARY,
        TOOL_CANARY,
    ] {
        assert!(
            !combined.contains(canary),
            "standard output leaked {canary}"
        );
    }
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["sessionId"],
        "session-1"
    );
    assert_eq!(json["projectBreakdown"][0]["name"], "project-1");
    assert!(json["projectBreakdown"][0]["path"].is_null());
    assert_eq!(json["dataCoverage"]["redactedFields"], 13);
}

#[test]
fn rejected_transcript_record_does_not_consume_public_aliases() {
    let home = SyntheticHome::new("rejected-transcript-aliases");
    let root = home.transcript_root("config");
    let mut rejected = assistant(
        "rejected-session",
        "rejected-message",
        "2026-04-05T08:00:00Z",
        99,
    );
    rejected["message"].as_object_mut().unwrap().remove("usage");
    home.write_session(&root, "a-rejected-project", "rejected-session", &[rejected]);
    home.write_session(
        &root,
        "b-valid-project",
        "valid-session",
        &[assistant(
            "valid-session",
            "valid-message",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["sessionId"],
        "session-1"
    );
    assert_eq!(json["projectBreakdown"][0]["name"], "project-1");
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["skippedRecords"], 1);
}

#[test]
fn arbitrary_model_and_tool_names_are_classified_before_standard_storage() {
    const MODEL_CANARY: &str = "PRIVATE_MODEL_NAME_CANARY_71A2";
    const TOOL_CANARY: &str = "PRIVATE_TOOL_NAME_CANARY_C8E4";
    let home = SyntheticHome::new("name-classification");
    let root = home.transcript_root("config");
    let mut record = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 10);
    record["message"]["model"] = Value::String(MODEL_CANARY.to_string());
    record["message"]["content"] = serde_json::json!([{
        "type": "tool_use",
        "name": TOOL_CANARY,
        "input": {"safe": true}
    }]);
    home.write_session(&root, "project-alpha", "session-a", &[record]);

    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);
    assert!(!combined.contains(MODEL_CANARY));
    assert!(!combined.contains(TOOL_CANARY));
    assert_eq!(json["dataCoverage"]["redactedFields"], 9);
    assert_eq!(
        json["dataCoverage"]["capabilities"]["tool_occurrence"],
        "available"
    );
}

#[test]
fn otel_invalid_optional_usage_does_not_become_zero_or_grade() {
    let home = SyntheticHome::new("invalid-optional-usage");
    let unix_nanos = 1_775_379_600_000_000_000u64;
    let otel = home.write_otel(
        "partial-request.jsonl",
        &[serde_json::json!({
            "resourceLogs": [{
                "resource": {
                    "attributes": [
                        otel_attribute(
                            "service.name",
                            Value::String("claude-code".to_string())
                        )
                    ]
                },
                "scopeLogs": [{
                    "scope": {"name": "com.anthropic.claude_code.events"},
                    "logRecords": [{
                        "timeUnixNano": unix_nanos.to_string(),
                        "body": {},
                        "attributes": [
                            otel_attribute(
                                "event.timestamp",
                                Value::String("2026-04-05T09:00:00Z".to_string())
                            ),
                            otel_attribute(
                                "session.id",
                                Value::String("partial-session".to_string())
                            ),
                            otel_attribute(
                                "request_id",
                                Value::String("partial-request".to_string())
                            ),
                            otel_attribute(
                                "model",
                                Value::String("claude-sonnet-4-6".to_string())
                            ),
                            otel_attribute(
                                "input_tokens",
                                Value::String("not-an-integer".to_string())
                            ),
                            otel_attribute("output_tokens", serde_json::json!(1))
                        ],
                        "eventName": "claude_code.api_request"
                    }]
                }]
            }]
        })],
    );

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 1);
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_input_tokens"],
        "unavailable"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_output_tokens"],
        "available"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cost"],
        "unavailable"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cache_health"],
        "unavailable"
    );
    assert_eq!(
        json["dataCoverage"]["costCoverage"],
        "unavailable-incomplete-usage"
    );
    assert!(json["recommendations"].as_array().unwrap().is_empty());
    assert_eq!(json["cacheHealth"]["grade"]["letter"], "N/A");

    let card_output = home.run(&["--card", "--otel-file", otel.to_str().unwrap(), "2026"]);
    assert!(
        card_output.status.success(),
        "status={}\nstdout={}\nstderr={}",
        card_output.status,
        String::from_utf8_lossy(&card_output.stdout),
        String::from_utf8_lossy(&card_output.stderr)
    );
    let card = fs::read_to_string(home.root.join("claude-code-wrapped-card.html"))
        .expect("read synthetic share card");
    assert!(!card.contains("Grade F"));
    assert!(!card.contains(">Season spend</span><span class=\"stat-value\">$0.00"));
    assert!(card.contains("Unavailable"));
    assert!(card.contains("Usage evidence is partial"));
}

#[test]
fn otel_explicit_zero_usage_remains_observed() {
    let home = SyntheticHome::new("explicit-zero-usage");
    let mut request = otel_api_request(
        "zero-session",
        "zero-request",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        0,
        Vec::new(),
    );
    let attributes = request["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"]
        .as_array_mut()
        .unwrap();
    for attribute in attributes {
        match attribute["key"].as_str() {
            Some(
                "input_tokens" | "output_tokens" | "cache_read_tokens" | "cache_creation_tokens",
            ) => attribute["value"] = serde_json::json!({"intValue": "0"}),
            Some("cost_usd") => attribute["value"] = serde_json::json!({"doubleValue": 0.0}),
            _ => {}
        }
    }
    let otel = home.write_otel("zero-request.jsonl", &[request]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    for capability in [
        "analysis_input_tokens",
        "analysis_output_tokens",
        "analysis_cache_creation_tokens",
        "analysis_cache_read_tokens",
        "analysis_usage_totals",
        "analysis_cost",
    ] {
        assert_eq!(
            json["dataCoverage"]["capabilities"][capability], "available",
            "explicit zero was not observed for {capability}"
        );
    }
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cache_health"],
        "unavailable"
    );
    assert_eq!(
        json["dataCoverage"]["costCoverage"],
        "source-recorded-estimate-and-local-computation"
    );
    assert_eq!(json["costAnalysis"]["totalCost"], 0.0);
    assert_eq!(json["cacheHealth"]["grade"]["letter"], "N/A");
}

#[test]
fn otel_token_metric_categories_are_collectively_available() {
    let home = SyntheticHome::new("metric-category-availability");
    let start = 1_775_379_000_000_000_000;
    let end = 1_775_379_600_000_000_000;
    let metrics = [
        ("input", 10),
        ("output", 20),
        ("cacheRead", 30),
        ("cacheCreation", 40),
    ]
    .into_iter()
    .map(|(category, value)| otel_token_metric("metric-session", category, 1, start, end, value))
    .collect::<Vec<_>>();
    let otel = home.write_otel("token-metrics.jsonl", &metrics);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    for capability in [
        "analysis_input_tokens",
        "analysis_output_tokens",
        "analysis_cache_creation_tokens",
        "analysis_cache_read_tokens",
        "analysis_usage_totals",
        "analysis_cost",
        "analysis_cache_health",
    ] {
        assert_eq!(
            json["dataCoverage"]["capabilities"][capability], "available",
            "separate metric category was treated as a missing per-request field for {capability}"
        );
    }
    assert_eq!(json["costAnalysis"]["totals"]["inputTokens"], 10);
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(json["costAnalysis"]["dailyCosts"][0]["messageCount"], 0);
    assert_eq!(json["costAnalysis"]["dailyCosts"][0]["sessionCount"], 0);
    assert_eq!(json["costAnalysis"]["activeDays"], 0);
    assert_eq!(json["wrappedStory"]["totalMessages"], 0);
    assert!(json["projectBreakdown"].as_array().unwrap().is_empty());
    assert_eq!(json["sessionIntel"]["available"], false);
    assert_eq!(json["modelRouting"]["available"], false);
    assert!(json["sessionBreakdown"]["sessions"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_eq!(json["cacheHealth"]["grade"]["letter"], "N/A");

    let card_output = home.run(&["--card", "--otel-file", otel.to_str().unwrap(), "2026"]);
    assert!(card_output.status.success());
    let card = fs::read_to_string(home.root.join("claude-code-wrapped-card.html")).unwrap();
    assert!(card.contains(r#">Observed messages</span><span class="stat-value">-</span>"#));
}

#[test]
fn disjoint_otel_requests_do_not_suppress_aggregate_metrics() {
    let home = SyntheticHome::new("disjoint-request-and-metric");
    let request = otel_api_request(
        "request-session",
        "request-a",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        5,
        Vec::new(),
    );
    let metric = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_777_885_200_000_000_000,
        1_777_885_800_000_000_000,
        20,
    );
    let request_file = home.write_otel("request.jsonl", &[request]);
    let metric_file = home.write_otel("metric.jsonl", &[metric]);

    let forward = successful_json(home.run(&[
        "--json",
        "--otel-file",
        request_file.to_str().unwrap(),
        "--otel-file",
        metric_file.to_str().unwrap(),
        "2026",
    ]));
    let reverse = successful_json(home.run(&[
        "--json",
        "--otel-file",
        metric_file.to_str().unwrap(),
        "--otel-file",
        request_file.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(forward["costAnalysis"]["totals"]["outputTokens"], 25);
    assert_eq!(forward["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(forward["dataCoverage"]["canonicalRecords"], 2);
    assert_eq!(forward["dataCoverage"]["authorityExcludedRecords"], 0);
    assert_eq!(forward["dataCoverage"]["unresolvedOverlapRecords"], 0);
    assert_eq!(
        forward["dataCoverage"]["capabilities"]["analysis_usage_totals"],
        "partial"
    );
    assert_eq!(
        forward["dataCoverage"]["capabilities"]["analysis_cost"],
        "partial"
    );
    assert_eq!(
        forward["dataCoverage"]["capabilities"]["analysis_cache_health"],
        "unavailable"
    );
    assert!(!forward["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_AUTHORITY_AGGREGATE_METRICS_SUPERSEDED"));

    let mut forward_without_sources = forward.clone();
    let mut reverse_without_sources = reverse;
    forward_without_sources["dataCoverage"]
        .as_object_mut()
        .unwrap()
        .remove("sources");
    reverse_without_sources["dataCoverage"]
        .as_object_mut()
        .unwrap()
        .remove("sources");
    assert_eq!(forward_without_sources, reverse_without_sources);

    let markdown_output = home.run(&[
        "--markdown",
        "--otel-file",
        request_file.to_str().unwrap(),
        "--otel-file",
        metric_file.to_str().unwrap(),
        "2026",
    ]);
    assert!(markdown_output.status.success());
    let markdown = fs::read_to_string(home.root.join("claude-code-wrapped.md")).unwrap();
    assert_eq!(
        forward["canonicalMetrics"]["tokens"]["projectUnattributed"]["output"]["observed"],
        25
    );
    assert!(forward["projectBreakdown"].as_array().unwrap().is_empty());
    assert!(!markdown.contains("## Top Projects"));
}

#[test]
fn overlapping_and_ambiguous_otel_metrics_are_not_summed_with_requests() {
    let home = SyntheticHome::new("overlapping-request-and-metrics");
    let request_time = 1_775_379_600_000_000_000;
    let request = otel_api_request(
        "request-session",
        "request-a",
        "2026-04-05T09:00:00Z",
        request_time,
        5,
        Vec::new(),
    );
    let overlapping = otel_token_metric(
        "request-session",
        "output",
        1,
        1_775_379_000_000_000_000,
        request_time,
        20,
    );
    let mut ambiguous = otel_token_metric(
        "unused-session",
        "output",
        1,
        1_775_379_000_000_000_000,
        request_time,
        30,
    );
    ambiguous["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"][0]
        ["attributes"]
        .as_array_mut()
        .unwrap()
        .retain(|attribute| attribute["key"] != "session.id");
    let otel = home.write_otel("collector.jsonl", &[request, overlapping, ambiguous]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 5);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 3);
    assert_eq!(json["dataCoverage"]["canonicalRecords"], 1);
    assert_eq!(json["dataCoverage"]["authorityExcludedRecords"], 1);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 1);
    for code in [
        "W_AUTHORITY_AGGREGATE_METRICS_SUPERSEDED",
        "W_AUTHORITY_AGGREGATE_METRICS_UNRESOLVED",
    ] {
        assert!(json["dataCoverage"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|warning| warning["code"] == code));
    }
}

#[test]
fn otel_source_cost_and_token_metrics_do_not_form_one_cost_claim() {
    let home = SyntheticHome::new("metric-cost-basis-conflict");
    let start = 1_775_379_000_000_000_000;
    let end = 1_775_379_600_000_000_000;
    let mut metrics = [
        ("input", 10),
        ("output", 20),
        ("cacheRead", 30),
        ("cacheCreation", 40),
    ]
    .into_iter()
    .map(|(category, value)| otel_token_metric("metric-session", category, 1, start, end, value))
    .collect::<Vec<_>>();
    let mut source_cost = otel_token_metric("metric-session", "output", 1, start, end, 7);
    source_cost["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["name"] =
        Value::String("claude_code.cost.usage".to_string());
    source_cost["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["unit"] =
        Value::String("USD".to_string());
    metrics.push(source_cost);
    let otel = home.write_otel("mixed-cost-bases.jsonl", &metrics);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cost"],
        "unavailable"
    );
    assert_eq!(
        json["dataCoverage"]["costCoverage"],
        "unavailable-conflicting-cost-bases"
    );
    assert_eq!(json["costAnalysis"]["totalCost"], 0.0);
    assert!(json["recommendations"].as_array().unwrap().is_empty());
}

#[test]
fn duplicate_identity_is_scoped_by_session_context() {
    let home = SyntheticHome::new("dedup-context");
    let root = home.transcript_root("dedup-config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "same-message-id",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    home.write_session(
        &root,
        "project-alpha",
        "session-b",
        &[assistant(
            "session-b",
            "same-message-id",
            "2026-04-05T10:00:00Z",
            20,
        )],
    );

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 30);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 0);
}

#[test]
fn malformed_unknown_and_duplicate_records_are_separately_counted() {
    let home = SyntheticHome::new("diagnostics");
    let root = home.transcript_root("diagnostics-config");
    let project = root.join("project-alpha");
    fs::create_dir_all(&project).unwrap();
    let accepted = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 10);
    let body = format!(
        "{}\n{}\n{{\"type\":\n{}\n",
        accepted,
        accepted,
        serde_json::json!({
            "type": "PRIVATE_UNKNOWN_KIND_CANARY_91D2",
            "timestamp": "2026-04-05T10:00:00Z",
            "payload": "UNKNOWN_VALUE_CANARY_8AB1"
        })
    );
    fs::write(project.join("session-a.jsonl"), body).unwrap();

    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);
    assert!(!combined.contains("UNKNOWN_VALUE_CANARY_8AB1"));
    assert!(!combined.contains("PRIVATE_UNKNOWN_KIND_CANARY_91D2"));
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 1);
    assert_eq!(json["dataCoverage"]["malformedRecords"], 1);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
    assert_eq!(json["dataCoverage"]["unknownRecords"], 1);
    assert_eq!(json["dataCoverage"]["classifiedRecords"], 4);
    assert_eq!(
        json["dataCoverage"]["recordCountInvariant"],
        "classifiedRecords = acceptedRecords + malformedRecords + unsupportedRecords + filteredRecords + skippedRecords + duplicateRecords; unknown/redacted/overlap counts are orthogonal"
    );
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
    assert_eq!(
        json["dataCoverage"]["unknownShapes"][0]["recordKind"],
        "unsupported-transcript-variant"
    );
    assert_eq!(
        json["dataCoverage"]["unknownShapes"][0]["structuralFields"]["type"],
        "string"
    );
    assert!(json["dataCoverage"]["unknownShapes"][0]["structuralFields"]
        .get("payload")
        .is_none());
}

#[test]
fn aggregate_token_totals_saturate_at_u64_boundary() {
    let home = SyntheticHome::new("token-saturation");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant("session-a", "message-a", "2026-04-05T09:00:00Z", u64::MAX),
            assistant("session-a", "message-b", "2026-04-05T10:00:00Z", u64::MAX),
            assistant("session-a", "message-c", "2026-04-06T10:00:00Z", u64::MAX),
        ],
    );

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], u64::MAX);
    assert_eq!(
        json["costAnalysis"]["dailyCosts"][0]["outputTokens"],
        u64::MAX
    );
    assert_eq!(
        json["costAnalysis"]["dailyCosts"][1]["outputTokens"],
        u64::MAX
    );
    assert_eq!(
        json["costAnalysis"]["dailyCosts"][0]["models"][0]["tokens"]["output"],
        u64::MAX
    );
    assert_eq!(json["projectBreakdown"][0]["outputTokens"], u64::MAX);
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["totalTokens"],
        u64::MAX
    );
    assert_eq!(json["wrappedStory"]["totalTokens"], u64::MAX);
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_input_tokens"],
        "available"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_output_tokens"],
        "partial"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_usage_totals"],
        "partial"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cost"],
        "partial"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cache_health"],
        "unavailable"
    );
    assert_eq!(
        json["dataCoverage"]["costCoverage"],
        "partial-observed-cost-evidence"
    );
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_ANALYTICAL_TOKEN_SATURATED"));
    assert_eq!(json["modelRouting"]["available"], true);
    assert_eq!(json["modelRouting"]["sonnetPct"], 100);
    assert!(json["anomalies"]["anomalies"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(json["recommendations"].as_array().unwrap().is_empty());
}

#[test]
fn partial_cost_coverage_separates_local_and_source_cost_sums() {
    let home = SyntheticHome::new("partial-observed-cost");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir_all(&project).unwrap();

    let mut observed = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 10);
    observed["costUSD"] = serde_json::json!(1.25);
    fs::write(
        project.join("session-a.jsonl"),
        format!("{}\n{{\"type\":\"#\n", observed),
    )
    .unwrap();

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cost"],
        "partial"
    );
    assert_eq!(
        json["dataCoverage"]["costCoverage"],
        "partial-observed-cost-evidence"
    );
    assert_eq!(
        json["canonicalMetrics"]["cost"]["sourceRecorded"]["amountUsd"],
        1.25
    );
    assert_eq!(
        json["canonicalMetrics"]["cost"]["localApiEquivalent"]["amountUsd"],
        0.000153
    );
    assert_eq!(json["sessionBreakdown"]["sessions"][0]["costUsd"], 1.25);
    assert_eq!(json["costAnalysis"]["totalCost"], 0.000153);
    assert_eq!(json["costAnalysis"]["dailyCosts"][0]["cost"], 0.000153);
    assert_eq!(
        json["costAnalysis"]["dailyCosts"][0]["models"][0]["cost"],
        0.000153
    );
    assert_eq!(json["costAnalysis"]["modelCosts"]["Sonnet 4.6"], 0.000153);
    assert_eq!(json["costAnalysis"]["avgDailyCost"], 0.0);
    assert_eq!(json["costAnalysis"]["medianDailyCost"], 0.0);
    assert!(json["costAnalysis"]["peakDay"].is_null());
    assert_eq!(json["modelRouting"]["available"], true);
    assert_eq!(json["modelRouting"]["sonnetPct"], 100);
    assert!(json["anomalies"]["anomalies"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(json["recommendations"].as_array().unwrap().is_empty());
}

#[test]
fn degradation_unknown_records_downgrade_analytical_claims() {
    let valid = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 10).to_string();
    let malformed = r#"{"type":"#.to_string();
    let mut same_source_reports = Vec::new();

    for (label, body) in [
        ("before", format!("{malformed}\n{valid}\n")),
        ("after", format!("{valid}\n{malformed}\n")),
    ] {
        let home = SyntheticHome::new(&format!("degradation-{label}"));
        let root = home.transcript_root("config");
        let project = root.join("project-alpha");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("session-a.jsonl"), body).unwrap();

        let json =
            successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
        assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
        assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
        assert_eq!(json["dataCoverage"]["malformedRecords"], 1);
        assert_eq!(json["dataCoverage"]["classifiedRecords"], 2);
        assert_eq!(json["dataCoverage"]["completeness"], "partial");
        for capability in [
            "analysis_input_tokens",
            "analysis_output_tokens",
            "analysis_cache_creation_tokens",
            "analysis_cache_read_tokens",
            "analysis_usage_totals",
            "analysis_cost",
        ] {
            assert_eq!(
                json["dataCoverage"]["capabilities"][capability], "partial",
                "unknowable rejected record left {capability} overconfident"
            );
        }
        assert_eq!(
            json["dataCoverage"]["capabilities"]["analysis_cache_health"],
            "unavailable"
        );
        assert_eq!(
            json["dataCoverage"]["costCoverage"],
            "partial-observed-cost-evidence"
        );
        assert!(json["costAnalysis"]["totalCost"].as_f64().unwrap() > 0.0);
        assert_eq!(json["modelRouting"]["available"], true);
        assert_eq!(json["modelRouting"]["sonnetPct"], 100);
        assert_eq!(json["cacheHealth"]["grade"]["letter"], "N/A");
        assert!(json["recommendations"].as_array().unwrap().is_empty());

        if label == "after" {
            let markdown_output =
                home.run(&["--markdown", "--data-dir", root.to_str().unwrap(), "2026"]);
            assert!(markdown_output.status.success());
            let markdown = fs::read_to_string(home.root.join("claude-code-wrapped.md")).unwrap();
            assert!(markdown.contains("Usage evidence is partial"));
            assert!(markdown.contains("**API-equivalent estimate:**"));
        }
        same_source_reports.push(json);
    }
    assert_eq!(same_source_reports[0], same_source_reports[1]);

    let mixed_home = SyntheticHome::new("degradation-mixed-sources");
    let root = mixed_home.transcript_root("config");
    mixed_home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let otel = mixed_home.root.join("malformed-otel.jsonl");
    fs::write(&otel, "{\"resourceLogs\":\n").unwrap();
    let mixed = successful_json(mixed_home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(mixed["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(mixed["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(mixed["dataCoverage"]["malformedRecords"], 1);
    assert_eq!(mixed["dataCoverage"]["completeness"], "partial");
    assert_eq!(
        mixed["dataCoverage"]["capabilities"]["analysis_usage_totals"],
        "partial"
    );
    assert_eq!(
        mixed["dataCoverage"]["capabilities"]["analysis_cost"],
        "partial"
    );
    assert_eq!(
        mixed["dataCoverage"]["capabilities"]["analysis_cache_health"],
        "unavailable"
    );
    let otel_source = mixed["dataCoverage"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["kind"] == "otel")
        .unwrap();
    assert_eq!(otel_source["acceptedRecords"], 0);
    assert_eq!(otel_source["malformedRecords"], 1);
    assert_eq!(otel_source["completeness"], "indeterminate");
}

#[test]
fn provably_out_of_period_records_do_not_downgrade_analytical_claims() {
    let home = SyntheticHome::new("out-of-period-analytical-claims");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant("session-a", "message-selected", "2026-04-05T09:00:00Z", 10),
            assistant(
                "session-a",
                "message-prior-year",
                "2025-04-05T09:00:00Z",
                1_000,
            ),
        ],
    );

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["filteredRecords"], 1);
    assert_eq!(json["dataCoverage"]["classifiedRecords"], 2);
    for capability in [
        "analysis_input_tokens",
        "analysis_output_tokens",
        "analysis_cache_creation_tokens",
        "analysis_cache_read_tokens",
        "analysis_usage_totals",
        "analysis_cost",
        "analysis_cache_health",
    ] {
        assert_eq!(
            json["dataCoverage"]["capabilities"][capability], "available",
            "provably irrelevant record downgraded {capability}"
        );
    }
    assert_eq!(json["cacheHealth"]["grade"]["letter"], "N/A");

    let metric_home = SyntheticHome::new("out-of-period-metric-claims");
    let selected_request = otel_api_request(
        "session-a",
        "request-selected",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        10,
        Vec::new(),
    );
    let prior_year_metric = otel_token_metric(
        "session-a",
        "output",
        1,
        1_743_843_600_000_000_000,
        1_743_847_200_000_000_000,
        1_000,
    );
    let otel = metric_home.write_otel(
        "out-of-period.jsonl",
        &[selected_request, prior_year_metric],
    );
    let metric_json = successful_json(metric_home.run(&[
        "--json",
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(metric_json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(metric_json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(metric_json["dataCoverage"]["filteredRecords"], 1);
    for capability in [
        "analysis_input_tokens",
        "analysis_output_tokens",
        "analysis_cache_creation_tokens",
        "analysis_cache_read_tokens",
        "analysis_usage_totals",
        "analysis_cost",
        "analysis_cache_health",
    ] {
        assert_eq!(
            metric_json["dataCoverage"]["capabilities"][capability], "available",
            "provably irrelevant metric downgraded {capability}"
        );
    }
}

#[test]
fn transcript_rejections_count_nested_and_early_exit_redactions() {
    let home = SyntheticHome::new("rejected-redactions");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir_all(&project).unwrap();
    let body = [
        serde_json::json!({
            "type": "future_variant",
            "timestamp": "2026-04-05T08:00:00Z",
            "message": {
                "id": "unknown-id",
                "content": {"secret": "UNKNOWN_NESTED_SECRET"}
            }
        }),
        serde_json::json!({
            "type": "user",
            "sessionId": "missing-time-session",
            "message": {"id": "missing-time-id", "content": "MISSING_TIME_SECRET"}
        }),
        serde_json::json!({
            "type": "user",
            "sessionId": "invalid-time-session",
            "timestamp": "not-a-timestamp",
            "message": {"id": "invalid-time-id", "content": "INVALID_TIME_SECRET"}
        }),
        serde_json::json!({
            "type": "user",
            "sessionId": "filtered-session",
            "timestamp": "2025-04-05T08:00:00Z",
            "message": {"id": "filtered-id", "content": "FILTERED_SECRET"}
        }),
        assistant(
            "accepted-session",
            "accepted-message",
            "2026-04-05T09:00:00Z",
            10,
        ),
    ]
    .into_iter()
    .map(|record| record.to_string())
    .collect::<Vec<_>>()
    .join("\n");
    fs::write(project.join("session.jsonl"), format!("{body}\n")).unwrap();

    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);
    for canary in [
        "UNKNOWN_NESTED_SECRET",
        "MISSING_TIME_SECRET",
        "INVALID_TIME_SECRET",
        "FILTERED_SECRET",
    ] {
        assert!(!combined.contains(canary));
    }
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
    assert_eq!(json["dataCoverage"]["skippedRecords"], 1);
    assert_eq!(json["dataCoverage"]["malformedRecords"], 1);
    assert_eq!(json["dataCoverage"]["filteredRecords"], 1);
    assert_eq!(json["dataCoverage"]["redactedFields"], 25);
}

#[test]
fn timestamps_are_canonicalized_to_utc_before_analysis_and_output() {
    let home = SyntheticHome::new("utc-normalization");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T11:00:00+02:00",
            10,
        )],
    );

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["generatedAt"], "2026-04-05T09:00:00Z");
    assert_eq!(
        json["dataCoverage"]["earliestObservedAt"],
        "2026-04-05T09:00:00Z"
    );
    assert_eq!(json["costAnalysis"]["dailyCosts"][0]["date"], "2026-04-05");
}

#[cfg(unix)]
#[test]
fn report_is_invariant_to_ambient_tz() {
    let home = SyntheticHome::new("ambient-tz-invariance");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );

    let run = |timezone: &str| {
        Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
            .args([
                "--json",
                "--timezone",
                "UTC",
                "--data-dir",
                root.to_str().unwrap(),
                "2026",
            ])
            .env("HOME", home.root.join("isolated-home"))
            .env("XDG_CACHE_HOME", home.root.join("isolated-cache"))
            .env(
                "CLAUDE_CONFIG_DIR",
                home.root.join("isolated-claude-config"),
            )
            .env("NO_COLOR", "1")
            .env("TZ", timezone)
            .output()
            .expect("run ccwrapped with a synthetic ambient timezone")
    };

    let utc = successful_json(run("UTC"));
    let los_angeles = successful_json(run("America/Los_Angeles"));
    assert_eq!(utc, los_angeles);
    assert_eq!(utc["sessionIntel"]["hourDistribution"][9], 1);
    assert_eq!(utc["modelRouting"]["busiestHour"]["hour"], 9);
    assert_eq!(utc["wrappedStory"]["powerHour"]["hour"], 9);
}

#[test]
fn pinned_otel_api_request_is_accepted_without_sensitive_attributes() {
    let home = SyntheticHome::new("otel-api-request");
    let otel = home.write_otel(
        "collector.jsonl",
        &[otel_api_request(
            "otel-session-private",
            "otel-request-private",
            "2026-04-05T09:00:00Z",
            1_775_379_600_000_000_000,
            20,
            vec![
                otel_attribute(
                    "tool_input",
                    Value::String("PRIVATE_OTEL_TOOL_INPUT_CANARY".to_string()),
                ),
                otel_attribute(
                    "organization.id",
                    Value::String("PRIVATE_ORGANIZATION_CANARY".to_string()),
                ),
            ],
        )],
    );
    let output = home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);
    for canary in [
        "PRIVATE_EMAIL_CANARY",
        "PRIVATE_OTEL_TOOL_INPUT_CANARY",
        "PRIVATE_ORGANIZATION_CANARY",
        "otel-session-private",
        "otel-request-private",
    ] {
        assert!(!combined.contains(canary), "OTel output leaked {canary}");
    }
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["redactedFields"], 5);
    assert_eq!(json["dataCoverage"]["sources"][0]["alias"], "otel-1");
    assert_eq!(
        json["dataCoverage"]["sources"][0]["producerContract"],
        "otelcol-contrib/file/v0.148.0+pdata/v1.54.0+slim-otlp/v1.10.0"
    );
    assert_eq!(
        json["dataCoverage"]["sources"][0]["producerVerification"],
        "unverified"
    );
}

#[test]
fn otel_pinned_integer_wire_encoding() {
    let home = SyntheticHome::new("otel-pinned-integers");
    let start = 1_775_379_000_000_000_000;
    let end = 1_775_379_600_000_000_000;
    let exact_tokens = 9_007_199_254_740_993u64;
    let mut exact_metric =
        otel_token_metric("metric-session", "output", 1, start, end, exact_tokens);
    use_pinned_otel_integer_strings(&mut exact_metric);
    let exact_file = home.write_otel("exact-integer.jsonl", &[exact_metric]);
    let exact_json = successful_json(home.run(&[
        "--json",
        "--otel-file",
        exact_file.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        exact_json["costAnalysis"]["totals"]["outputTokens"],
        exact_tokens
    );

    let mut pinned_request = otel_api_request(
        "request-session",
        "request-a",
        "2026-04-05T09:00:00Z",
        end,
        17,
        Vec::new(),
    );
    use_pinned_otel_integer_strings(&mut pinned_request);
    let request_file = home.write_otel("integer-attributes.jsonl", &[pinned_request]);
    let request_json = successful_json(home.run(&[
        "--json",
        "--otel-file",
        request_file.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(request_json["costAnalysis"]["totals"]["outputTokens"], 17);

    let mut late_invalid = otel_token_metric("metric-session", "output", 2, start, end, 10);
    use_pinned_otel_integer_strings(&mut late_invalid);
    let mut malformed_point = late_invalid["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]
        ["sum"]["dataPoints"][0]
        .clone();
    malformed_point["asInt"] = Value::String("not-an-integer".to_string());
    late_invalid["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"]
        .as_array_mut()
        .unwrap()
        .push(malformed_point);

    let mut conflict = otel_token_metric("conflict-session", "output", 1, start, end, 1);
    use_pinned_otel_integer_strings(&mut conflict);
    conflict["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"][0]
        ["asDouble"] = serde_json::json!(1.0);

    let mut out_of_range = otel_api_request(
        "request-session",
        "request-b",
        "2026-04-05T09:00:00Z",
        end,
        1,
        Vec::new(),
    );
    use_pinned_otel_integer_strings(&mut out_of_range);
    out_of_range["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"][5]["value"]
        ["intValue"] = Value::String("9223372036854775808".to_string());

    let later = end + 600_000_000_000;
    let mut accepted = otel_token_metric("metric-session", "output", 2, start, later, 16);
    use_pinned_otel_integer_strings(&mut accepted);
    let invalid_file = home.write_otel(
        "invalid-integers.jsonl",
        &[late_invalid, conflict, out_of_range, accepted],
    );
    let invalid_json = successful_json(home.run(&[
        "--json",
        "--otel-file",
        invalid_file.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(invalid_json["costAnalysis"]["totals"]["outputTokens"], 16);
    assert_eq!(invalid_json["dataCoverage"]["unsupportedRecords"], 3);
    let warning_codes = invalid_json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    for code in [
        "W_OTEL_INTEGER_STRING_INVALID",
        "W_OTEL_INTEGER_RANGE",
        "W_OTEL_POINT_VALUE_CONFLICT",
    ] {
        assert!(warning_codes.contains(&code), "missing {code}");
    }
}

#[test]
fn every_supported_otel_log_event_surfaces_its_direct_capability() {
    let home = SyntheticHome::new("otel-log-event-matrix");
    let mut export = otel_api_request(
        "session-a",
        "request-a",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        2,
        vec![otel_attribute("attempt", Value::from(1))],
    );
    let base = export["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0].clone();
    let records = export["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
        .as_array_mut()
        .unwrap();
    for (event_name, extras) in [
        ("claude_code.api_error", vec![]),
        (
            "claude_code.tool_result",
            vec![
                otel_attribute("tool_name", Value::String("Read".to_string())),
                otel_attribute("success", Value::Bool(false)),
            ],
        ),
        (
            "claude_code.tool_decision",
            vec![
                otel_attribute("tool_name", Value::String("Edit".to_string())),
                otel_attribute("decision", Value::String("accept".to_string())),
            ],
        ),
        ("claude_code.user_prompt", vec![]),
        (
            "claude_code.compaction",
            vec![otel_attribute("success", Value::Bool(true))],
        ),
    ] {
        let mut record = base.clone();
        record["eventName"] = Value::String(event_name.to_string());
        record["attributes"].as_array_mut().unwrap().extend(extras);
        records.push(record);
    }
    let otel = home.write_otel("events.jsonl", &[export]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 6);
    for capability in [
        "api_request",
        "api_error",
        "direct_terminal_outcomes",
        "retry_evidence",
        "prompt_occurrence",
        "tool_result",
        "tool_decision",
        "tool_status",
        "tool_latency",
        "edit_decision",
        "compaction",
    ] {
        assert_eq!(
            json["dataCoverage"]["capabilities"][capability], "available",
            "missing OTel event capability {capability}"
        );
    }
    assert_eq!(
        json["dataCoverage"]["capabilities"]["tool_occurrence"], "unavailable",
        "a direct OTel result must not invent a transcript tool-use occurrence"
    );
}

#[test]
fn fallback_cross_source_identity_preserves_distinct_facts() {
    let home = SyntheticHome::new("otel-fallback-identity");
    let mut export = otel_api_request(
        "session-a",
        "request-a",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        2,
        Vec::new(),
    );
    let base = export["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0].clone();
    let record = |event_name: &str, extras: Vec<Value>| {
        let mut record = base.clone();
        record["eventName"] = Value::String(event_name.to_string());
        let attributes = record["attributes"].as_array_mut().unwrap();
        attributes.retain(|attribute| attribute["key"] != "request_id");
        attributes.extend(extras);
        record
    };
    let paired = |event_name: &str, key: &str, left: Value, right: Value| {
        [
            record(event_name, vec![otel_attribute(key, left)]),
            record(event_name, vec![otel_attribute(key, right)]),
        ]
    };

    // Retain one usage event so the CLI can render a complete report while the request-less
    // records exercise the fallback overlap identity.
    let mut records = vec![base.clone()];
    records.extend(paired(
        "claude_code.api_error",
        "attempt",
        serde_json::json!(1),
        serde_json::json!(2),
    ));
    records.extend([
        record(
            "claude_code.tool_result",
            vec![
                otel_attribute("tool_name", Value::String("Read".to_string())),
                otel_attribute("success", Value::Bool(true)),
            ],
        ),
        record(
            "claude_code.tool_result",
            vec![
                otel_attribute("tool_name", Value::String("Read".to_string())),
                otel_attribute("success", Value::Bool(false)),
            ],
        ),
    ]);
    records.extend([
        record(
            "claude_code.tool_decision",
            vec![
                otel_attribute("tool_name", Value::String("Edit".to_string())),
                otel_attribute("decision", Value::String("accept".to_string())),
            ],
        ),
        record(
            "claude_code.tool_decision",
            vec![
                otel_attribute("tool_name", Value::String("Edit".to_string())),
                otel_attribute("decision", Value::String("reject".to_string())),
            ],
        ),
    ]);
    records.extend(paired(
        "claude_code.compaction",
        "success",
        Value::Bool(true),
        Value::Bool(false),
    ));
    for key in [
        "agent_id",
        "parent_agent_id",
        "skill.name",
        "plugin.name",
        "mcp_server.name",
        "mcp_tool.name",
    ] {
        records.extend(paired(
            "claude_code.user_prompt",
            key,
            Value::String(format!("{key}-a")),
            Value::String(format!("{key}-b")),
        ));
    }
    let expected_records = records.len();
    export["resourceLogs"][0]["scopeLogs"][0]["logRecords"] = Value::Array(records);
    let otel = home.write_otel("fallback-identity.jsonl", &[export]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["dataCoverage"]["acceptedRecords"], expected_records);
    assert_eq!(
        json["dataCoverage"]["resolvedOverlapRecords"], 0,
        "distinct fallback facts or identity contexts collapsed as repeated observations"
    );
    for capability in [
        "api_error",
        "prompt_occurrence",
        "tool_result",
        "tool_status",
        "edit_decision",
        "compaction",
    ] {
        assert_eq!(
            json["dataCoverage"]["capabilities"][capability],
            "available"
        );
    }
    assert_eq!(
        json["dataCoverage"]["capabilities"]["tool_occurrence"], "unavailable",
        "direct OTel result identities remain distinct without becoming occurrence evidence"
    );
}

#[test]
fn content_bearing_otel_events_are_excluded_without_copying_bodies() {
    const CANARY: &str = "PRIVATE_OTEL_RESPONSE_BODY_CANARY_447A";
    let home = SyntheticHome::new("otel-content-event");
    let mut export = otel_api_request(
        "session-a",
        "request-a",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        2,
        Vec::new(),
    );
    let mut content = export["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0].clone();
    content["eventName"] = Value::String("claude_code.api_response_body".to_string());
    content["body"] = Value::String(CANARY.to_string());
    export["resourceLogs"][0]["scopeLogs"][0]["logRecords"]
        .as_array_mut()
        .unwrap()
        .push(content);
    let otel = home.write_otel("content.jsonl", &[export]);
    let output = home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);
    assert!(!combined.contains(CANARY));
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_CONTENT_EVENT_EXCLUDED"));
}

#[test]
fn every_supported_otel_metric_retains_a_named_capability() {
    let home = SyntheticHome::new("otel-metric-matrix");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let names = [
        "claude_code.session.count",
        "claude_code.lines_of_code.count",
        "claude_code.pull_request.count",
        "claude_code.commit.count",
        "claude_code.cost.usage",
        "claude_code.token.usage",
        "claude_code.code_edit_tool.decision",
        "claude_code.active_time.total",
    ];
    let mut objects = Vec::new();
    for name in names {
        let mut metric = otel_token_metric(
            "metric-session",
            "output",
            1,
            1_775_379_000_000_000_000,
            1_775_379_600_000_000_000,
            1,
        );
        metric["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["name"] =
            Value::String(name.to_string());
        metric["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["unit"] =
            Value::String(otel_metric_wire_unit(name).to_string());
        if name == "claude_code.code_edit_tool.decision" {
            metric["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"][0]
                ["attributes"]
                .as_array_mut()
                .unwrap()
                .push(otel_attribute(
                    "decision",
                    Value::String("accept".to_string()),
                ));
        }
        objects.push(metric);
    }
    let otel = home.write_otel("metrics.jsonl", &objects);
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    for capability in [
        "metric_session_count",
        "metric_lines_of_code",
        "metric_pull_requests",
        "metric_commits",
        "metric_source_cost_estimate",
        "metric_token_usage",
        "metric_code_edit_decision",
        "metric_active_time",
    ] {
        assert_eq!(
            json["dataCoverage"]["capabilities"][capability], "available",
            "missing OTel metric capability {capability}"
        );
    }
}

#[test]
fn otel_rejects_conflicting_metric_units() {
    let home = SyntheticHome::new("otel-metric-unit");
    let mut wrong_unit = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_775_379_000_000_000_000,
        1_775_379_600_000_000_000,
        1_000,
    );
    wrong_unit["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["name"] =
        Value::String("claude_code.active_time.total".to_string());
    wrong_unit["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["unit"] =
        Value::String("ms".to_string());
    let request = otel_api_request(
        "request-session",
        "request-a",
        "2026-04-05T09:01:00Z",
        1_775_379_660_000_000_000,
        2,
        Vec::new(),
    );
    let otel = home.write_otel("collector.jsonl", &[wrong_unit, request]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
    assert_ne!(
        json["dataCoverage"]["capabilities"]["metric_active_time"],
        "available"
    );
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_METRIC_UNIT_UNSUPPORTED"));
}

#[test]
fn strong_request_identity_prefers_otel_without_double_counting() {
    let home = SyntheticHome::new("otel-correlated");
    let root = home.transcript_root("transcript-config");
    let mut transcript = assistant(
        "shared-session",
        "transcript-message",
        "2026-04-05T09:00:00Z",
        10,
    );
    transcript["requestId"] = Value::String("shared-request".to_string());
    home.write_session(&root, "project-alpha", "shared-session", &[transcript]);
    let otel = home.write_otel(
        "collector.jsonl",
        &[otel_api_request(
            "shared-session",
            "shared-request",
            "2026-04-05T09:00:00Z",
            1_775_379_600_000_000_000,
            20,
            vec![otel_attribute("speed", Value::String("fast".to_string()))],
        )],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 1);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
    assert_eq!(json["canonicalMetrics"]["cost"]["pricedTokens"], 0);
    assert_eq!(
        json["canonicalMetrics"]["cost"]["unpricedTokens"], 29,
        "the authoritative OTel modifier must survive transcript correlation"
    );
}

#[test]
fn repeated_request_ids_use_maximum_cross_source_matching() {
    let home = SyntheticHome::new("otel-maximum-matching");
    let root = home.transcript_root("transcript-config");
    let mut transcript_early = assistant(
        "shared-session",
        "transcript-early",
        "2026-04-05T09:00:00Z",
        10,
    );
    transcript_early["requestId"] = Value::String("shared-request".to_string());
    let mut transcript_late = assistant(
        "shared-session",
        "transcript-late",
        "2026-04-05T09:04:00Z",
        20,
    );
    transcript_late["requestId"] = Value::String("shared-request".to_string());
    home.write_session(
        &root,
        "project-alpha",
        "shared-session",
        &[transcript_early, transcript_late],
    );
    let otel = home.write_otel(
        "collector.jsonl",
        &[
            otel_api_request(
                "shared-session",
                "shared-request",
                "2026-04-05T09:03:00Z",
                1_775_379_780_000_000_000,
                30,
                Vec::new(),
            ),
            otel_api_request(
                "shared-session",
                "shared-request",
                "2026-04-05T09:06:00Z",
                1_775_379_960_000_000_000,
                40,
                Vec::new(),
            ),
        ],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        json["dataCoverage"]["resolvedOverlapRecords"], 2,
        "{json:#}"
    );
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 70);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 4);
}

#[test]
fn maximum_cross_source_matching_minimizes_total_timestamp_distance() {
    let home = SyntheticHome::new("otel-minimum-distance-matching");
    let root = home.transcript_root("transcript-config");
    let mut transcript_early = assistant(
        "shared-session",
        "transcript-early",
        "2026-04-05T09:00:00Z",
        10,
    );
    transcript_early["requestId"] = Value::String("shared-request".to_string());
    let mut transcript_late = assistant(
        "shared-session",
        "transcript-late",
        "2026-04-05T09:04:00Z",
        20,
    );
    transcript_late["requestId"] = Value::String("shared-request".to_string());
    home.write_session(
        &root,
        "project-alpha",
        "shared-session",
        &[transcript_early],
    );
    home.write_session(&root, "project-beta", "shared-session", &[transcript_late]);
    let otel = home.write_otel(
        "collector.jsonl",
        &[
            otel_api_request(
                "shared-session",
                "shared-request",
                "2026-04-05T09:01:00Z",
                1_775_379_660_000_000_000,
                30,
                Vec::new(),
            ),
            otel_api_request(
                "shared-session",
                "shared-request",
                "2026-04-05T09:03:00Z",
                1_775_379_780_000_000_000,
                40,
                Vec::new(),
            ),
        ],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    let projects = json["projectBreakdown"].as_array().unwrap();
    let project_alpha = projects
        .iter()
        .find(|project| project["hash"] == "project-1")
        .unwrap();
    let project_beta = projects
        .iter()
        .find(|project| project["hash"] == "project-2")
        .unwrap();
    assert_eq!(project_alpha["outputTokens"], 30);
    assert_eq!(project_beta["outputTokens"], 40);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 2);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
}

#[test]
fn subagent_request_identity_correlates_across_sources() {
    let home = SyntheticHome::new("otel-subagent-request-correlation");
    let root = home.transcript_root("transcript-config");
    let mut transcript = assistant(
        "shared-subagent-session",
        "transcript-subagent",
        "2026-04-05T09:00:00Z",
        10,
    );
    transcript["requestId"] = Value::String("shared-subagent-request".to_string());
    home.write_session(
        &root,
        "project-alpha/parent-session/subagents",
        "subagent-session",
        &[transcript],
    );
    let otel = home.write_otel(
        "collector.jsonl",
        &[otel_api_request(
            "shared-subagent-session",
            "shared-subagent-request",
            "2026-04-05T09:01:00Z",
            1_775_379_660_000_000_000,
            30,
            vec![otel_attribute(
                "agent_id",
                Value::String("synthetic-agent".to_string()),
            )],
        )],
    );

    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["canonicalRecords"], 1);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 1);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 30);
}

#[test]
fn equal_distance_request_matching_is_record_order_independent() {
    let home = SyntheticHome::new("otel-equal-distance-matching");
    let root = home.transcript_root("transcript-config");
    let mut transcript_early = assistant(
        "shared-session",
        "transcript-early",
        "2026-04-05T09:00:00Z",
        10,
    );
    transcript_early["requestId"] = Value::String("shared-request".to_string());
    let mut transcript_late = assistant(
        "shared-session",
        "transcript-late",
        "2026-04-05T09:02:00Z",
        20,
    );
    transcript_late["requestId"] = Value::String("shared-request".to_string());
    home.write_session(
        &root,
        "project-alpha/parent-alpha/subagents",
        "shared-session-alpha",
        &[transcript_early],
    );
    home.write_session(
        &root,
        "project-beta/parent-beta/subagents",
        "shared-session-beta",
        &[transcript_late],
    );
    let request_30 = otel_api_request(
        "shared-session",
        "shared-request",
        "2026-04-05T09:01:00Z",
        1_775_379_660_000_000_000,
        30,
        vec![otel_attribute(
            "agent_id",
            Value::String("synthetic-agent-30".to_string()),
        )],
    );
    let request_40 = otel_api_request(
        "shared-session",
        "shared-request",
        "2026-04-05T09:01:00Z",
        1_775_379_660_000_000_000,
        40,
        vec![otel_attribute(
            "agent_id",
            Value::String("synthetic-agent-40".to_string()),
        )],
    );
    let forward_file = home.write_otel("forward.jsonl", &[request_30.clone(), request_40.clone()]);
    let reverse_file = home.write_otel("reverse.jsonl", &[request_40, request_30]);

    let forward = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        forward_file.to_str().unwrap(),
        "2026",
    ]));
    let reverse = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        reverse_file.to_str().unwrap(),
        "2026",
    ]));

    assert_eq!(forward["dataCoverage"]["resolvedOverlapRecords"], 2);
    assert_eq!(forward["dataCoverage"]["unresolvedOverlapRecords"], 0);
    assert_eq!(
        forward, reverse,
        "equal-cost matching followed record order"
    );
}

#[test]
fn oversized_request_correlation_groups_degrade_with_a_bounded_warning() {
    let home = SyntheticHome::new("otel-correlation-limit");
    let root = home.transcript_root("transcript-config");
    let mut transcripts = Vec::new();
    for index in 0..128 {
        let timestamp = format!("2026-04-05T09:{:02}:{:02}Z", index / 60, index % 60);
        let mut transcript = assistant(
            "shared-session",
            &format!("transcript-{index}"),
            &timestamp,
            1,
        );
        transcript["requestId"] = Value::String("shared-request".to_string());
        transcripts.push(transcript);
    }
    home.write_session(&root, "project-alpha", "shared-session", &transcripts);
    let otel = home.write_otel(
        "collector.jsonl",
        &[otel_api_request(
            "shared-session",
            "shared-request",
            "2026-04-05T09:01:00Z",
            1_775_379_660_000_000_000,
            999,
            Vec::new(),
        )],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 128);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 129);
    assert_eq!(json["dataCoverage"]["canonicalRecords"], 128);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 0);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 1);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_AUTHORITY_CORRELATION_LIMIT"));
}

#[test]
fn aggregate_request_matching_budget_degrades_independently_of_record_order() {
    let home = SyntheticHome::new("otel-correlation-work-budget");
    let root = home.transcript_root("transcript-config");
    let mut transcripts = Vec::new();
    let mut telemetry = Vec::new();
    for group in 0..10 {
        transcripts.push(assistant(
            &format!("session-{group}"),
            &format!("transcript-{group}"),
            "2026-04-05T09:01:00Z",
            1,
        ));
        transcripts.last_mut().unwrap()["requestId"] = Value::String(format!("request-{group}"));
        for second in 0..127u64 {
            let timestamp = format!("2026-04-05T09:{:02}:{:02}Z", second / 60, second % 60);
            telemetry.push(otel_api_request(
                &format!("session-{group}"),
                &format!("request-{group}"),
                &timestamp,
                1_775_379_600_000_000_000 + second * 1_000_000_000,
                999,
                Vec::new(),
            ));
        }
    }
    home.write_session(&root, "project-alpha", "transcript-container", &transcripts);
    let forward = home.write_otel("forward.jsonl", &telemetry);
    telemetry.reverse();
    let reverse = home.write_otel("reverse.jsonl", &telemetry);

    let forward_output = home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        forward.to_str().unwrap(),
        "2026",
    ]);
    let reverse_output = home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        reverse.to_str().unwrap(),
        "2026",
    ]);
    assert_eq!(forward_output.stdout, reverse_output.stdout);
    let json = successful_json(forward_output);
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1_280);
    assert_eq!(json["dataCoverage"]["canonicalRecords"], 10);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 0);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 1_270);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_AUTHORITY_CORRELATION_LIMIT"));
}

#[test]
fn repeated_request_at_the_same_source_timestamp_remains_a_duplicate() {
    let home = SyntheticHome::new("same-source-request-duplicate");
    let root = home.transcript_root("transcript-config");
    let mut first = assistant(
        "shared-session",
        "transcript-first",
        "2026-04-05T09:00:00Z",
        10,
    );
    first["requestId"] = Value::String("shared-request".to_string());
    let mut repeated = assistant(
        "shared-session",
        "transcript-repeated",
        "2026-04-05T09:00:00Z",
        20,
    );
    repeated["requestId"] = Value::String("shared-request".to_string());
    home.write_session(&root, "project-alpha", "shared-session", &[first, repeated]);
    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 1);
}

#[test]
fn request_identity_never_correlates_across_sessions_or_incompatible_times() {
    let home = SyntheticHome::new("request-context");
    let root = home.transcript_root("config");
    let mut transcript = assistant(
        "transcript-session",
        "transcript-message",
        "2026-04-05T09:00:00Z",
        10,
    );
    transcript["requestId"] = Value::String("reused-request".to_string());
    home.write_session(&root, "project-alpha", "transcript-session", &[transcript]);
    let otel = home.write_otel(
        "collector.jsonl",
        &[
            otel_api_request(
                "different-session",
                "reused-request",
                "2026-04-05T09:00:00Z",
                1_775_379_600_000_000_000,
                20,
                Vec::new(),
            ),
            otel_api_request(
                "transcript-session",
                "reused-request",
                "2026-04-08T09:00:00Z",
                1_775_638_800_000_000_000,
                30,
                Vec::new(),
            ),
        ],
    );

    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 0);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 2);
}

#[test]
fn repeated_request_ids_in_distinct_otel_sessions_remain_distinct() {
    let home = SyntheticHome::new("request-session-scope");
    let first = otel_api_request(
        "session-a",
        "reused-request",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        10,
        Vec::new(),
    );
    let second = otel_api_request(
        "session-b",
        "reused-request",
        "2026-04-05T09:01:00Z",
        1_775_379_660_000_000_000,
        20,
        Vec::new(),
    );
    let first_file = home.write_otel("first.jsonl", &[first]);
    let second_file = home.write_otel("second.jsonl", &[second]);

    let json = successful_json(home.run(&[
        "--json",
        "--otel-file",
        first_file.to_str().unwrap(),
        "--otel-file",
        second_file.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 30);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 0);
}

#[test]
fn unresolved_cross_source_overlap_keeps_transcript_authority() {
    let home = SyntheticHome::new("otel-unresolved");
    let root = home.transcript_root("transcript-config");
    home.write_session(
        &root,
        "project-alpha",
        "transcript-session",
        &[assistant(
            "transcript-session",
            "transcript-message",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let otel = home.write_otel(
        "collector.jsonl",
        &[otel_api_request(
            "other-session",
            "other-request",
            "2026-04-05T09:00:01Z",
            1_775_379_601_000_000_000,
            20,
            Vec::new(),
        )],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 1);
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
}

#[test]
fn disjoint_transcript_and_otel_metric_are_both_canonical() {
    let home = SyntheticHome::new("disjoint-transcript-and-metric");
    let root = home.transcript_root("transcript-config");
    home.write_session(
        &root,
        "project-alpha",
        "transcript-session",
        &[assistant(
            "transcript-session",
            "transcript-message",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let metric = home.write_otel(
        "metric.jsonl",
        &[otel_token_metric(
            "metric-session",
            "output",
            1,
            1_777_885_200_000_000_000,
            1_777_885_800_000_000_000,
            20,
        )],
    );

    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        metric.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 30);
    assert_eq!(
        json["costAnalysis"]["dailyCosts"].as_array().unwrap().len(),
        2
    );
    assert_eq!(json["wrappedStory"]["totalMessages"], 1);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["canonicalRecords"], 2);
    assert_eq!(json["dataCoverage"]["authorityExcludedRecords"], 0);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
    assert!(!json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_AUTHORITY_UNRESOLVED_OVERLAP"));
}

#[test]
fn overlapping_transcript_and_otel_metric_remains_unresolved() {
    let home = SyntheticHome::new("overlapping-transcript-and-metric");
    let root = home.transcript_root("transcript-config");
    home.write_session(
        &root,
        "project-alpha",
        "shared-session",
        &[assistant(
            "shared-session",
            "transcript-message",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let metric = home.write_otel(
        "metric.jsonl",
        &[otel_token_metric(
            "shared-session",
            "output",
            1,
            1_775_379_000_000_000_000,
            1_775_379_600_000_000_000,
            20,
        )],
    );

    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        metric.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["canonicalRecords"], 1);
    assert_eq!(json["dataCoverage"]["authorityExcludedRecords"], 0);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 1);
}

#[test]
fn correlated_otel_request_supersedes_metric_with_transcript_present() {
    let home = SyntheticHome::new("correlated-request-supersedes-metric");
    let root = home.transcript_root("transcript-config");
    let mut transcript = assistant(
        "shared-session",
        "transcript-message",
        "2026-04-05T09:00:00Z",
        10,
    );
    transcript["requestId"] = Value::String("shared-request".to_string());
    home.write_session(&root, "project-alpha", "shared-session", &[transcript]);
    let request = otel_api_request(
        "shared-session",
        "shared-request",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        20,
        Vec::new(),
    );
    let metric = otel_token_metric(
        "shared-session",
        "output",
        1,
        1_775_379_000_000_000_000,
        1_775_379_600_000_000_000,
        30,
    );
    let otel = home.write_otel("collector.jsonl", &[request, metric]);

    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 3);
    assert_eq!(json["dataCoverage"]["canonicalRecords"], 1);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 1);
    assert_eq!(json["dataCoverage"]["authorityExcludedRecords"], 1);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_AUTHORITY_AGGREGATE_METRICS_SUPERSEDED"));
}

#[test]
fn strong_message_identity_collapses_repeated_transcript_roots() {
    let home = SyntheticHome::new("repeated-transcript-roots");
    let first = home.transcript_root("first");
    let second = home.transcript_root("second");
    let record = assistant(
        "shared-session",
        "shared-message",
        "2026-04-05T09:00:00Z",
        10,
    );
    home.write_session(
        &first,
        "project-alpha",
        "shared-session",
        std::slice::from_ref(&record),
    );
    home.write_session(
        &second,
        "project-alpha",
        "shared-session",
        std::slice::from_ref(&record),
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        first.to_str().unwrap(),
        "--data-dir",
        second.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 0);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 1);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
}

#[test]
fn repeated_transcript_roots_collapse_prompt_occurrences_as_well_as_usage() {
    let home = SyntheticHome::new("repeated-prompt-roots");
    let first = home.transcript_root("first");
    let second = home.transcript_root("second");
    let prompt = user_prompt(
        "shared-session",
        "shared-user-message",
        "2026-04-05T08:59:00Z",
        "PRIVATE_REPEATED_PROMPT_CANARY",
    );
    let usage = assistant(
        "shared-session",
        "shared-assistant-message",
        "2026-04-05T09:00:00Z",
        10,
    );
    for root in [&first, &second] {
        home.write_session(
            root,
            "project-alpha",
            "shared-session",
            &[prompt.clone(), usage.clone()],
        );
    }

    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        first.to_str().unwrap(),
        "--data-dir",
        second.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["sessionBreakdown"]["sessions"][0]["promptCount"], 1);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 2);
}

#[test]
fn strong_request_identity_collapses_repeated_otel_files() {
    let home = SyntheticHome::new("repeated-otel-files");
    let record = otel_api_request(
        "shared-session",
        "shared-request",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        20,
        Vec::new(),
    );
    let first = home.write_otel("first.jsonl", std::slice::from_ref(&record));
    let second = home.write_otel("second.jsonl", std::slice::from_ref(&record));
    let json = successful_json(home.run(&[
        "--json",
        "--otel-file",
        first.to_str().unwrap(),
        "--otel-file",
        second.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["resolvedOverlapRecords"], 1);
    assert_eq!(json["dataCoverage"]["unresolvedOverlapRecords"], 0);
}

#[test]
fn otel_hardlinks_import_physical_file_once() {
    let home = SyntheticHome::new("hard-linked-otel");
    let mut session_count = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_775_379_000_000_000_000,
        1_775_379_600_000_000_000,
        7,
    );
    session_count["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["name"] =
        Value::String("claude_code.session.count".to_string());
    session_count["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["unit"] =
        Value::String("count".to_string());
    let original = home.write_otel("original.jsonl", &[session_count]);
    let alias = home.root.join("alias.jsonl");
    fs::hard_link(&original, &alias).expect("create OTel hard link");

    let json = successful_json(home.run(&[
        "--json",
        "--otel-file",
        original.to_str().unwrap(),
        "--otel-file",
        alias.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["dataCoverage"]["sourceRootCount"], 1);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(
        json["dataCoverage"]["capabilities"]["metric_session_count"],
        "available"
    );
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_DISCOVERY_DUPLICATE_OTEL"));
}

#[test]
fn claude_config_dir_precedes_the_supported_home_default() {
    let home = SyntheticHome::new("implicit-precedence");
    let config_projects = home.config_dir().join("projects");
    let default_projects = home.default_projects_dir();
    fs::create_dir_all(&config_projects).unwrap();
    fs::create_dir_all(&default_projects).unwrap();
    home.write_session(
        &config_projects,
        "config-project",
        "config-session",
        &[assistant(
            "config-session",
            "config-message",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    home.write_session(
        &default_projects,
        "default-project",
        "default-session",
        &[assistant(
            "default-session",
            "default-message",
            "2026-04-05T10:00:00Z",
            20,
        )],
    );

    let json = successful_json(home.run(&["--json", "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["sourceRootCount"], 1);
    assert_eq!(json["dataCoverage"]["sources"][0]["alias"], "transcript-1");
    assert_eq!(
        json["dataCoverage"]["sources"][0]["selection"],
        "claude-config-env"
    );
}

#[cfg(unix)]
#[test]
fn canonical_duplicate_roots_import_once_and_keep_command_order_aliases() {
    use std::os::unix::fs::symlink;

    let home = SyntheticHome::new("duplicate-root");
    let root = home.transcript_root("real-config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let link = home.root.join("projects-link");
    symlink(&root, &link).unwrap();
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        link.to_str().unwrap(),
        "--data-dir",
        root.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["sourceRootCount"], 1);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_DISCOVERY_DUPLICATE_TRANSCRIPT"));
}

#[test]
fn otel_many_source_checkpointing_is_bounded() {
    let home = SyntheticHome::new("otel-source-limit");
    let private_path_canary = home.root.join("PRIVATE_SOURCE_LIMIT_CANARY.jsonl");
    let mut args = vec!["--json".to_string()];
    for _ in 0..257 {
        args.push("--otel-file".to_string());
        args.push(private_path_canary.to_string_lossy().into_owned());
    }
    args.push("2026".to_string());
    let borrowed = args.iter().map(String::as_str).collect::<Vec<_>>();

    let output = home.run(&borrowed);
    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["code"], "E_DISCOVERY_SOURCE_LIMIT");
    assert_eq!(json["sourceAlias"], Value::Null);
    assert!(json["remediation"].as_str().unwrap().contains("256"));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("PRIVATE_SOURCE_LIMIT_CANARY"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("PRIVATE_SOURCE_LIMIT_CANARY"));
}

#[test]
fn richer_duplicate_wins_and_decision_is_salt_independent() {
    let home = SyntheticHome::new("richer-duplicate");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir_all(&project).unwrap();
    let sparse = serde_json::json!({
        "type": "assistant",
        "sessionId": "session-a",
        "timestamp": "2026-04-05T09:00:00Z",
        "message": {
            "id": "message-a",
            "usage": {"output_tokens": 10},
            "content": []
        }
    });
    let rich = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 20);
    fs::write(
        project.join("session-a.jsonl"),
        format!("{}\n{}\n", sparse, rich),
    )
    .unwrap();
    let args = ["--json", "--data-dir", root.to_str().unwrap(), "2026"];
    let first = home.run(&args);
    let second = home.run(&args);
    assert!(first.status.success());
    assert!(second.status.success());
    assert_eq!(
        first.stdout, second.stdout,
        "fresh salts changed report bytes"
    );
    let json: Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 1);
}

#[test]
fn equal_richness_duplicate_order_is_deterministic() {
    let home = SyntheticHome::new("equal-richness-duplicate-order");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir_all(&project).unwrap();
    let lower = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 10);
    let higher = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 20);
    let first_path = project.join("a.jsonl");
    let second_path = project.join("b.jsonl");
    let run = || home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);

    fs::write(&first_path, format!("{lower}\n")).unwrap();
    fs::write(&second_path, format!("{higher}\n")).unwrap();
    let forward = run();
    fs::write(&first_path, format!("{higher}\n")).unwrap();
    fs::write(&second_path, format!("{lower}\n")).unwrap();
    let reversed = run();

    assert!(forward.status.success());
    assert!(reversed.status.success());
    assert_eq!(
        forward.stdout, reversed.stdout,
        "equal-richness duplicate facts followed physical file enumeration"
    );
    let json: Value = serde_json::from_slice(&forward.stdout).unwrap();
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 1);
}

#[test]
fn empty_explicit_history_returns_machine_readable_empty_coverage() {
    let home = SyntheticHome::new("empty");
    let root = home.transcript_root("empty-config");
    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"], "no records found");
    assert_eq!(json["dataCoverage"]["completeness"], "empty");
    assert_eq!(json["dataCoverage"]["sourceRootCount"], 1);
    assert_eq!(json["dataCoverage"]["filesDiscovered"], 0);
}

#[test]
fn exact_source_paths_require_explicit_private_diagnostics() {
    let home = SyntheticHome::new("private-diagnostics");
    let root = home.transcript_root("PRIVATE_DIAGNOSTIC_PATH_CANARY");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let standard = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    assert!(standard.status.success());
    assert!(!String::from_utf8_lossy(&standard.stderr).contains("PRIVATE_DIAGNOSTIC_PATH_CANARY"));
    assert!(!String::from_utf8_lossy(&standard.stdout).contains("PRIVATE_DIAGNOSTIC_PATH_CANARY"));

    let private = home.run(&[
        "--json",
        "--private-diagnostics",
        "--data-dir",
        root.to_str().unwrap(),
        "2026",
    ]);
    assert!(private.status.success());
    assert!(String::from_utf8_lossy(&private.stderr).contains("PRIVATE_DIAGNOSTIC_PATH_CANARY"));
    assert!(!String::from_utf8_lossy(&private.stdout).contains("PRIVATE_DIAGNOSTIC_PATH_CANARY"));
}

#[test]
fn missing_implicit_transcripts_are_visible_when_explicit_otel_is_usable() {
    let home = SyntheticHome::new("implicit-missing-with-otel");
    let otel = home.write_otel(
        "collector.jsonl",
        &[otel_api_request(
            "session-a",
            "request-a",
            "2026-04-05T09:00:00Z",
            1_775_379_600_000_000_000,
            2,
            Vec::new(),
        )],
    );
    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
    let codes = json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"W_DISCOVERY_CONFIG_DIR_MISSING"));
    assert!(codes.contains(&"W_DISCOVERY_DEFAULT_MISSING"));
}

#[cfg(unix)]
#[test]
fn transcript_symlink_escape_is_excluded_and_marks_coverage_partial() {
    use std::os::unix::fs::symlink;

    let home = SyntheticHome::new("symlink-escape");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let outside = home.root.join("outside.jsonl");
    fs::write(
        &outside,
        format!(
            "{}\n",
            assistant(
                "outside-session",
                "outside-message",
                "2026-04-05T09:01:00Z",
                999,
            )
        ),
    )
    .unwrap();
    symlink(&outside, root.join("project-alpha/escape.jsonl")).unwrap();

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_TRANSCRIPT_SYMLINK_ESCAPE"));
}

#[cfg(unix)]
#[test]
fn non_utf8_project_names_remain_distinct() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;

    let home = SyntheticHome::new("non-utf8-project-identity");
    let root = home.transcript_root("config");
    for byte in [0x80, 0x81] {
        let mut name = b"project-".to_vec();
        name.push(byte);
        let project = root.join(OsString::from_vec(name));
        fs::create_dir(&project).unwrap();
        fs::write(
            project.join("session-a.jsonl"),
            format!(
                "{}\n",
                assistant(
                    "shared-session",
                    "shared-message",
                    "2026-04-05T09:00:00Z",
                    10,
                )
            ),
        )
        .unwrap();
    }

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 0);
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(json["projectBreakdown"].as_array().unwrap().len(), 2);
}

#[cfg(unix)]
#[test]
fn hard_linked_transcript_is_scanned_once() {
    let home = SyntheticHome::new("hard-linked-transcript");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir(&project).unwrap();
    let original = project.join("a.jsonl");
    fs::write(
        &original,
        format!(
            "{}\n",
            serde_json::json!({
                "type": "assistant",
                "timestamp": "2026-04-05T09:00:00Z",
                "message": {
                    "model": "claude-sonnet-4-6",
                    "usage": {"input_tokens": 1, "output_tokens": 10},
                    "content": []
                }
            })
        ),
    )
    .unwrap();
    fs::hard_link(&original, project.join("b.jsonl")).unwrap();

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["dataCoverage"]["filesDiscovered"], 1);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_TRANSCRIPT_DUPLICATE_FILE"));
}

#[cfg(unix)]
#[test]
fn unreadable_explicit_file_is_an_actionable_error_not_empty_history() {
    use std::os::unix::fs::PermissionsExt;

    let home = SyntheticHome::new("unreadable-file");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let file = root.join("project-alpha/session-a.jsonl");
    fs::set_permissions(&file, fs::Permissions::from_mode(0o000)).unwrap();
    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    fs::set_permissions(&file, fs::Permissions::from_mode(0o600)).unwrap();
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.contains(file.to_str().unwrap()));
    assert!(!stdout.contains("no records found"));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"], "ingestion failed");
    assert_eq!(json["code"], "E_TRANSCRIPT_INGESTION");
    assert_eq!(json["sourceAlias"], "transcript-1");
}

#[test]
fn cumulative_otel_metrics_use_differences_and_repeat_import_is_idempotent() {
    let home = SyntheticHome::new("otel-cumulative");
    let start = 1_775_379_000_000_000_000;
    let middle = 1_775_379_600_000_000_000;
    let end = 1_775_380_200_000_000_000;
    let first = otel_token_metric("metric-session", "output", 2, start, middle, 10);
    let second = otel_token_metric("metric-session", "output", 2, start, end, 16);
    let otel = home.write_otel("metrics.jsonl", &[first.clone(), first, second]);
    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 16);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 2);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 1);
    assert_eq!(
        json["dataCoverage"]["capabilities"]["otel_telemetry"],
        "available"
    );
}

#[test]
fn cumulative_metric_identity_is_attribute_order_independent() {
    let home = SyntheticHome::new("otel-attribute-order");
    let start = 1_775_379_000_000_000_000;
    let middle = 1_775_379_600_000_000_000;
    let end = 1_775_380_200_000_000_000;
    let first = otel_token_metric("metric-session", "output", 2, start, middle, 10);
    let mut second = otel_token_metric("metric-session", "output", 2, start, end, 16);
    second["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"][0]
        ["attributes"]
        .as_array_mut()
        .unwrap()
        .reverse();
    let otel = home.write_otel("metrics.jsonl", &[first, second]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 16);
}

#[test]
fn cumulative_metric_state_continues_across_selected_files() {
    let home = SyntheticHome::new("otel-multi-file-cumulative");
    let start = 1_775_379_000_000_000_000;
    let middle = 1_775_379_600_000_000_000;
    let end = 1_775_380_200_000_000_000;
    let first = home.write_otel(
        "first.jsonl",
        &[otel_token_metric(
            "metric-session",
            "output",
            2,
            start,
            middle,
            10,
        )],
    );
    let second = home.write_otel(
        "second.jsonl",
        &[otel_token_metric(
            "metric-session",
            "output",
            2,
            start,
            end,
            16,
        )],
    );

    let json = successful_json(home.run(&[
        "--json",
        "--otel-file",
        first.to_str().unwrap(),
        "--otel-file",
        second.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 16);
}

#[test]
fn cumulative_metric_state_is_selected_file_order_independent() {
    let home = SyntheticHome::new("otel-multi-file-order");
    let start = 1_775_379_000_000_000_000;
    let middle = 1_775_379_600_000_000_000;
    let end = 1_775_380_200_000_000_000;
    let earlier = home.write_otel(
        "earlier.jsonl",
        &[otel_token_metric(
            "metric-session",
            "output",
            2,
            start,
            middle,
            10,
        )],
    );
    let later = home.write_otel(
        "later.jsonl",
        &[otel_token_metric(
            "metric-session",
            "output",
            2,
            start,
            end,
            16,
        )],
    );

    let forward = successful_json(home.run(&[
        "--json",
        "--otel-file",
        earlier.to_str().unwrap(),
        "--otel-file",
        later.to_str().unwrap(),
        "2026",
    ]));
    let reverse = successful_json(home.run(&[
        "--json",
        "--otel-file",
        later.to_str().unwrap(),
        "--otel-file",
        earlier.to_str().unwrap(),
        "2026",
    ]));

    let mut forward_without_selector_attribution = forward;
    let mut reverse_without_selector_attribution = reverse;
    forward_without_selector_attribution["dataCoverage"]
        .as_object_mut()
        .unwrap()
        .remove("sources");
    reverse_without_selector_attribution["dataCoverage"]
        .as_object_mut()
        .unwrap()
        .remove("sources");
    assert_eq!(
        forward_without_selector_attribution, reverse_without_selector_attribution,
        "selector order changed analytical facts or global accounting"
    );
}

#[test]
fn a_rejected_otel_object_cannot_mutate_later_cumulative_state() {
    let home = SyntheticHome::new("otel-atomic-object");
    let start = 1_775_379_000_000_000_000;
    let middle = 1_775_379_600_000_000_000;
    let end = 1_775_380_200_000_000_000;
    let mut rejected = otel_token_metric("metric-session", "output", 2, start, middle, 10);
    let mut invalid_point = rejected["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]
        ["dataPoints"][0]
        .clone();
    invalid_point
        .as_object_mut()
        .unwrap()
        .remove("timeUnixNano");
    rejected["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["sum"]["dataPoints"]
        .as_array_mut()
        .unwrap()
        .push(invalid_point);
    let accepted = otel_token_metric("metric-session", "output", 2, start, end, 16);
    let otel = home.write_otel("metrics.jsonl", &[rejected, accepted]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 16);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
}

#[test]
fn non_usage_metrics_do_not_invent_assistant_messages() {
    let home = SyntheticHome::new("otel-non-usage-metric");
    let request = otel_api_request(
        "event-session",
        "event-request",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        2,
        Vec::new(),
    );
    let mut session_count = otel_token_metric(
        "event-session",
        "output",
        1,
        1_775_379_000_000_000_000,
        1_775_379_600_000_000_000,
        1,
    );
    session_count["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["name"] =
        Value::String("claude_code.session.count".to_string());
    session_count["resourceMetrics"][0]["scopeMetrics"][0]["metrics"][0]["unit"] =
        Value::String("count".to_string());
    let otel = home.write_otel("collector.jsonl", &[request, session_count]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["dailyCosts"][0]["messageCount"], 1);
    assert_eq!(
        json["dataCoverage"]["capabilities"]["metric_session_count"],
        "available"
    );
}

#[test]
fn json_ingestion_failures_have_a_stable_safe_actionable_shape() {
    let home = SyntheticHome::new("structured-ingestion-error");
    let missing = home.root.join("PRIVATE_MISSING_PATH_CANARY");
    let output = home.run(&["--json", "--data-dir", missing.to_str().unwrap(), "2026"]);
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let combined = String::from_utf8_lossy(&output.stdout);
    assert!(!combined.contains("PRIVATE_MISSING_PATH_CANARY"));
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(json["error"], "ingestion failed");
    assert_eq!(json["code"], "E_DISCOVERY_TRANSCRIPT_MISSING");
    assert_eq!(json["sourceAlias"], "transcript-1");
    assert!(json["remediation"].as_str().unwrap().contains("--data-dir"));
}

#[test]
fn otel_skipped_record_marks_global_and_source_partial() {
    let home = SyntheticHome::new("otel-skipped-completeness");
    let request = otel_api_request(
        "session-a",
        "request-a",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        2,
        Vec::new(),
    );
    let otel = home.root.join("collector.jsonl");
    fs::write(&otel, format!("{request}\n\n")).unwrap();

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    let source = json["dataCoverage"]["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["kind"] == "otel")
        .unwrap();

    assert_eq!(json["dataCoverage"]["skippedRecords"], 1);
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
    assert_eq!(source["skippedRecords"], 1);
    assert_eq!(source["completeness"], "partial");
}

#[test]
fn transcript_skipped_record_marks_global_and_source_indeterminate() {
    let home = SyntheticHome::new("transcript-skipped-completeness");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir(&project).unwrap();
    fs::write(project.join("session-a.jsonl"), "\n").unwrap();

    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    assert!(!output.status.success());
    assert!(output.stderr.is_empty());
    let json: Value = serde_json::from_slice(&output.stdout).unwrap();
    let source = &json["dataCoverage"]["sources"][0];

    assert_eq!(json["dataCoverage"]["skippedRecords"], 1);
    assert_eq!(json["dataCoverage"]["completeness"], "indeterminate");
    assert_eq!(source["skippedRecords"], 1);
    assert_eq!(source["completeness"], "indeterminate");
}

#[test]
fn metric_boundary_straddle_is_partial_and_never_prorated() {
    let home = SyntheticHome::new("otel-boundary");
    let request = otel_api_request(
        "event-session",
        "event-request",
        "2026-01-01T00:00:01Z",
        1_767_225_601_000_000_000,
        2,
        Vec::new(),
    );
    let straddling = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_767_225_599_000_000_000,
        1_767_225_601_000_000_000,
        1_000,
    );
    let otel = home.write_otel("boundary.jsonl", &[request, straddling]);
    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 2);
    assert_eq!(json["dataCoverage"]["filteredRecords"], 1);
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_PERIOD_BOUNDARY_STRADDLE"));
}

#[test]
fn metric_same_year_day_boundary_straddle_is_filtered() {
    let home = SyntheticHome::new("otel-day-boundary");
    let request = otel_api_request(
        "event-session",
        "event-request",
        "2026-04-05T12:00:00Z",
        1_775_390_400_000_000_000,
        2,
        Vec::new(),
    );
    let straddling = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_775_433_540_000_000_000,
        1_775_433_660_000_000_000,
        1_000,
    );
    let otel = home.write_otel("day-boundary.jsonl", &[request, straddling]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 2);
    assert_eq!(
        json["costAnalysis"]["dailyCosts"].as_array().unwrap().len(),
        1
    );
    assert_eq!(json["costAnalysis"]["dailyCosts"][0]["date"], "2026-04-05");
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["filteredRecords"], 1);
    assert_eq!(json["dataCoverage"]["classifiedRecords"], 2);
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_output_tokens"],
        "partial"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_usage_totals"],
        "partial"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cost"],
        "partial"
    );
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_cache_health"],
        "unavailable"
    );
    let source = &json["dataCoverage"]["sources"][0];
    assert_eq!(source["acceptedRecords"], 1);
    assert_eq!(source["filteredRecords"], 1);
    assert_eq!(source["classifiedRecords"], 2);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_PERIOD_BOUNDARY_STRADDLE"));

    let terminal = home.run(&["--plain", "--otel-file", otel.to_str().unwrap(), "2026"]);
    assert!(terminal.status.success());
    let stdout = String::from_utf8(terminal.stdout).unwrap();
    assert!(!stdout.contains("1.0K"));
    assert!(stdout.contains("W_OTEL_PERIOD_BOUNDARY_STRADDLE"));
}

#[test]
fn metric_midnight_endpoint_obeys_the_half_open_day_contract() {
    let ending_home = SyntheticHome::new("otel-ending-midnight");
    let request = otel_api_request(
        "event-session",
        "event-request",
        "2026-04-05T12:00:00Z",
        1_775_390_400_000_000_000,
        2,
        Vec::new(),
    );
    let ending_at_midnight = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_775_433_540_000_000_000,
        1_775_433_600_000_000_000,
        1_000,
    );
    let ending_file =
        ending_home.write_otel("ending-midnight.jsonl", &[request, ending_at_midnight]);
    let ending_json = successful_json(ending_home.run(&[
        "--json",
        "--otel-file",
        ending_file.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(ending_json["costAnalysis"]["totals"]["outputTokens"], 2);
    assert_eq!(ending_json["dataCoverage"]["filteredRecords"], 1);

    let starting_home = SyntheticHome::new("otel-starting-midnight");
    let starting_at_midnight = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_775_433_600_000_000_000,
        1_775_433_660_000_000_000,
        10,
    );
    let starting_file =
        starting_home.write_otel("starting-midnight.jsonl", &[starting_at_midnight]);
    let starting_json = successful_json(starting_home.run(&[
        "--json",
        "--otel-file",
        starting_file.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(starting_json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(
        starting_json["costAnalysis"]["dailyCosts"][0]["date"],
        "2026-04-06"
    );
    assert_eq!(starting_json["dataCoverage"]["filteredRecords"], 0);
}

#[test]
fn overlapping_delta_points_reconcile_physical_records() {
    let home = SyntheticHome::new("otel-overlap-accounting");
    let first = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_775_376_000_000_000_000,
        1_775_383_200_000_000_000,
        10,
    );
    let second = otel_token_metric(
        "metric-session",
        "output",
        1,
        1_775_379_600_000_000_000,
        1_775_386_800_000_000_000,
        5,
    );
    let otel = home.write_otel("overlap.jsonl", &[first, second]);

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 1);
    assert_eq!(json["dataCoverage"]["filteredRecords"], 1);
    assert_eq!(json["dataCoverage"]["classifiedRecords"], 2);
    assert_eq!(
        json["dataCoverage"]["capabilities"]["analysis_output_tokens"],
        "partial"
    );
    let source = &json["dataCoverage"]["sources"][0];
    assert_eq!(source["acceptedRecords"], 1);
    assert_eq!(source["filteredRecords"], 1);
    assert_eq!(source["classifiedRecords"], 2);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_METRIC_OVERLAP"));

    let terminal = home.run(&["--plain", "--otel-file", otel.to_str().unwrap(), "2026"]);
    assert!(terminal.status.success());
    let stdout = String::from_utf8(terminal.stdout).unwrap();
    assert!(stdout.contains("1 accepted; partial"));
    assert!(stdout.contains("W_OTEL_METRIC_OVERLAP"));
}

#[test]
fn incompatible_otel_shape_is_counted_without_guessing_partial_facts() {
    let home = SyntheticHome::new("otel-incompatible");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let otel = home.write_otel(
        "incompatible.jsonl",
        &[serde_json::json!({
            "resourceSpans": [{"private": "INCOMPATIBLE_SHAPE_CANARY"}]
        })],
    );
    let output = home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);
    assert!(!combined.contains("INCOMPATIBLE_SHAPE_CANARY"));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
    assert_eq!(json["dataCoverage"]["completeness"], "partial");
}

#[test]
fn known_transcript_variants_and_sidechain_context_share_the_normalized_stream() {
    let home = SyntheticHome::new("known-variants");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir_all(project.join("session-main/subagents")).unwrap();
    let main_lines = [
        serde_json::json!({
            "type": "progress",
            "sessionId": "session-main",
            "timestamp": "2026-04-05T08:58:00Z",
            "data": {"private": "PROGRESS_VALUE_CANARY"}
        }),
        serde_json::json!({
            "type": "system",
            "sessionId": "session-main",
            "timestamp": "2026-04-05T08:59:00Z",
            "message": {"content": "SYSTEM_VALUE_CANARY"}
        }),
        serde_json::json!({
            "type": "summary",
            "sessionId": "session-main",
            "timestamp": "2026-04-05T09:00:00Z",
            "summary": "SUMMARY_VALUE_CANARY"
        }),
        assistant("session-main", "shared-message", "2026-04-05T09:01:00Z", 10),
    ];
    home.write_session(&root, "project-alpha", "session-main", &main_lines);
    let mut sidechain = assistant("session-sub", "shared-message", "2026-04-05T09:02:00Z", 20);
    sidechain["isSidechain"] = Value::Bool(true);
    fs::write(
        project.join("session-main/subagents/session-sub.jsonl"),
        format!("{sidechain}\n"),
    )
    .unwrap();

    let output = home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]);
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let json = successful_json(output);
    for canary in [
        "PROGRESS_VALUE_CANARY",
        "SYSTEM_VALUE_CANARY",
        "SUMMARY_VALUE_CANARY",
    ] {
        assert!(!combined.contains(canary));
    }
    assert_eq!(json["dataCoverage"]["acceptedRecords"], 5);
    assert_eq!(json["dataCoverage"]["duplicateRecords"], 0);
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 30);
    assert_eq!(json["sessionBreakdown"]["totalSubagentSessions"], 1);
}

#[test]
fn deleted_source_file_cannot_survive_a_fresh_no_store_scan() {
    let home = SyntheticHome::new("deleted-source");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let args = ["--json", "--data-dir", root.to_str().unwrap(), "2026"];
    let first = successful_json(home.run(&args));
    assert_eq!(first["costAnalysis"]["totals"]["outputTokens"], 10);
    fs::remove_file(root.join("project-alpha/session-a.jsonl")).unwrap();
    let second = home.run(&args);
    assert!(!second.status.success());
    let second: Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second["error"], "no records found");
    assert_eq!(second["dataCoverage"]["acceptedRecords"], 0);
    assert_eq!(second["dataCoverage"]["filesDiscovered"], 0);
}

#[test]
fn truncation_replacement_and_rename_are_reconciled_by_each_fresh_scan() {
    let home = SyntheticHome::new("fresh-source-reconciliation");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let file = root.join("project-alpha/session-a.jsonl");
    let args = ["--json", "--data-dir", root.to_str().unwrap(), "2026"];
    assert_eq!(
        successful_json(home.run(&args))["costAnalysis"]["totals"]["outputTokens"],
        10
    );

    fs::write(&file, "").unwrap();
    let truncated = home.run(&args);
    assert!(!truncated.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&truncated.stdout).unwrap()["dataCoverage"]
            ["acceptedRecords"],
        0
    );

    let replacement = root.join("project-alpha/replacement.tmp");
    fs::write(
        &replacement,
        format!(
            "{}\n",
            assistant("session-a", "message-b", "2026-04-05T10:00:00Z", 20,)
        ),
    )
    .unwrap();
    fs::rename(&replacement, &file).unwrap();
    assert_eq!(
        successful_json(home.run(&args))["costAnalysis"]["totals"]["outputTokens"],
        20
    );

    fs::rename(&file, root.join("project-alpha/renamed-session.jsonl")).unwrap();
    let renamed = successful_json(home.run(&args));
    assert_eq!(renamed["costAnalysis"]["totals"]["outputTokens"], 20);
    assert_eq!(renamed["dataCoverage"]["filesDiscovered"], 1);
}

#[test]
fn otel_attribute_limit_rejects_the_export_object_before_partial_acceptance() {
    let home = SyntheticHome::new("otel-attribute-limit");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let mut too_many = Vec::new();
    for index in 0..129 {
        too_many.push(otel_attribute(
            &format!("custom_{index}"),
            Value::String("synthetic".to_string()),
        ));
    }
    let otel = home.write_otel(
        "too-many-attributes.jsonl",
        &[serde_json::json!({
            "resourceLogs": [{
                "resource": {"attributes": too_many},
                "scopeLogs": [{
                    "scope": {"name": "com.anthropic.claude_code.events"},
                    "logRecords": [{
                        "timeUnixNano": 1_775_379_600_000_000_000u64,
                        "body": {},
                        "attributes": [],
                        "eventName": "claude_code.api_request"
                    }]
                }]
            }]
        })],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_ATTRIBUTE_LIMIT"));
}

#[test]
fn otel_scope_limit_is_enforced_before_resource_filtering() {
    let home = SyntheticHome::new("otel-scope-limit");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );
    let scopes = (0..257)
        .map(|_| {
            serde_json::json!({
                "scope": {"name": "not.claude"},
                "logRecords": []
            })
        })
        .collect::<Vec<_>>();
    let otel = home.write_otel(
        "too-many-scopes.jsonl",
        &[serde_json::json!({
            "resourceLogs": [{
                "resource": {"attributes": [
                    otel_attribute("service.name", Value::String("other-service".to_string()))
                ]},
                "scopeLogs": scopes
            }]
        })],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 1);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_OTEL_SCOPE_LIMIT"));
}

#[test]
fn oversized_and_deep_transcript_lines_are_bounded_and_later_records_survive() {
    let home = SyntheticHome::new("transcript-physical-limits");
    let root = home.transcript_root("config");
    let project = root.join("project-alpha");
    fs::create_dir_all(&project).unwrap();
    let oversized = "x".repeat(16 * 1024 * 1024 + 1);
    let deeply_nested = format!("{}0{}", "[".repeat(140), "]".repeat(140));
    let valid = assistant("session-a", "message-a", "2026-04-05T09:00:00Z", 10);
    fs::write(
        project.join("session-a.jsonl"),
        format!("{oversized}\n{deeply_nested}\n{valid}\n"),
    )
    .unwrap();

    let json = successful_json(home.run(&["--json", "--data-dir", root.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["malformedRecords"], 2);
    let codes = json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"W_TRANSCRIPT_LINE_OVERSIZED"));
    assert!(codes.contains(&"W_TRANSCRIPT_MALFORMED_JSON"));
}

#[test]
fn oversized_and_deep_otel_lines_are_bounded_and_later_exports_survive() {
    let home = SyntheticHome::new("otel-physical-limits");
    let otel = home.root.join("collector.jsonl");
    let oversized = "x".repeat(16 * 1024 * 1024 + 1);
    let deeply_nested = format!("{}0{}", "[".repeat(140), "]".repeat(140));
    let valid = otel_api_request(
        "session-a",
        "request-a",
        "2026-04-05T09:00:00Z",
        1_775_379_600_000_000_000,
        2,
        Vec::new(),
    );
    fs::write(&otel, format!("{oversized}\n{deeply_nested}\n{valid}\n")).unwrap();

    let json =
        successful_json(home.run(&["--json", "--otel-file", otel.to_str().unwrap(), "2026"]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 2);
    assert_eq!(json["dataCoverage"]["malformedRecords"], 2);
    let codes = json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"W_OTEL_LINE_OVERSIZED"));
    assert!(codes.contains(&"W_OTEL_MALFORMED_JSON"));
}

#[test]
fn otel_resource_record_point_and_text_limits_reject_whole_objects() {
    let home = SyntheticHome::new("otel-structural-limits");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            10,
        )],
    );

    let resources = (0..257)
        .map(|_| serde_json::json!({"scopeLogs": []}))
        .collect::<Vec<_>>();
    let records = (0..100_001)
        .map(|_| serde_json::json!({}))
        .collect::<Vec<_>>();
    let points = (0..100_001)
        .map(|_| serde_json::json!({}))
        .collect::<Vec<_>>();
    let objects = vec![
        serde_json::json!({"resourceLogs": resources}),
        serde_json::json!({
            "resourceLogs": [{
                "resource": {"attributes": []},
                "scopeLogs": [{
                    "scope": {"name": "com.anthropic.claude_code.events"},
                    "logRecords": records
                }]
            }]
        }),
        serde_json::json!({
            "resourceMetrics": [{
                "resource": {"attributes": []},
                "scopeMetrics": [{
                    "scope": {"name": "com.anthropic.claude_code"},
                    "metrics": [{
                        "name": "claude_code.token.usage",
                        "sum": {
                            "aggregationTemporality": 1,
                            "isMonotonic": true,
                            "dataPoints": points
                        }
                    }]
                }]
            }]
        }),
        serde_json::json!({
            "resourceLogs": [{
                "resource": {"attributes": [
                    otel_attribute(
                        "custom.private",
                        Value::String("x".repeat(65_537))
                    )
                ]},
                "scopeLogs": []
            }]
        }),
    ];
    let otel = home.write_otel("limits.jsonl", &objects);
    let json = successful_json(home.run(&[
        "--json",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(json["costAnalysis"]["totals"]["outputTokens"], 10);
    assert_eq!(json["dataCoverage"]["unsupportedRecords"], 4);
    let codes = json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|warning| warning["code"].as_str().unwrap())
        .collect::<Vec<_>>();
    for code in [
        "W_OTEL_RESOURCE_LIMIT",
        "W_OTEL_RECORD_LIMIT",
        "W_OTEL_POINT_LIMIT",
        "W_OTEL_ATTRIBUTE_TEXT_LIMIT",
    ] {
        assert!(
            codes.contains(&code),
            "missing structural limit code {code}"
        );
    }
}

#[test]
fn out_of_order_transcript_records_produce_stable_chronological_facts() {
    let home = SyntheticHome::new("out-of-order-transcript");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant("session-a", "message-late", "2026-04-05T10:00:00Z", 20),
            assistant("session-a", "message-early", "2026-04-05T09:00:00Z", 10),
        ],
    );
    let args = ["--json", "--data-dir", root.to_str().unwrap(), "2026"];
    let first = home.run(&args);
    let second = home.run(&args);
    assert_eq!(first.stdout, second.stdout);
    let json = successful_json(first);
    assert_eq!(json["generatedAt"], "2026-04-05T10:00:00Z");
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["timestampStart"],
        "2026-04-05T09:00:00Z"
    );
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["timestampEnd"],
        "2026-04-05T10:00:00Z"
    );
}

#[test]
fn every_standard_renderer_excludes_sensitive_canaries() {
    const CANARY: &str = "STANDARD_RENDERER_SECRET_CANARY_7E12";
    let home = SyntheticHome::new("renderer-privacy");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        &format!("project-{CANARY}"),
        "session-a",
        &[
            serde_json::json!({
                "type": "user",
                "sessionId": format!("session-{CANARY}"),
                "cwd": format!("/synthetic/{CANARY}"),
                "timestamp": "2026-04-05T09:00:00Z",
                "message": {"content": CANARY}
            }),
            serde_json::json!({
                "type": "assistant",
                "sessionId": format!("session-{CANARY}"),
                "cwd": format!("/synthetic/{CANARY}"),
                "timestamp": "2026-04-05T09:01:00Z",
                "message": {
                    "id": format!("message-{CANARY}"),
                    "model": "claude-sonnet-4-6",
                    "usage": {"input_tokens": 1, "output_tokens": 2},
                    "content": [{"type": "tool_use", "name": "Read", "input": CANARY}]
                }
            }),
        ],
    );
    let output_dir = home.root.join("standard-outputs");
    fs::create_dir_all(&output_dir).unwrap();
    let output = home.run_in(
        &[
            "--all",
            "--plain",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &output_dir,
    );
    assert!(output.status.success());
    let mut surfaces = vec![output.stdout, output.stderr];
    for name in [
        "claude-code-wrapped.html",
        "claude-code-wrapped.md",
        "claude-code-wrapped-card.html",
    ] {
        surfaces.push(fs::read(output_dir.join(name)).unwrap());
    }
    for surface in surfaces {
        let surface = String::from_utf8_lossy(&surface);
        for representation in [
            CANARY.to_string(),
            hex_encode(CANARY),
            percent_encode(CANARY),
            base64_encode(CANARY),
        ] {
            assert!(
                !surface.contains(&representation),
                "standard output leaked an encoded canary representation"
            );
        }
    }
    assert!(!output_dir.join("wrapped-archive").exists());
}

#[test]
fn archive_is_the_only_explicit_content_bearing_sidecar() {
    const CANARY: &str = "PRIVATE_ARCHIVE_CONTENT_CANARY_C811";
    let home = SyntheticHome::new("archive-privacy");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            serde_json::json!({
                "type": "user",
                "sessionId": "session-a",
                "timestamp": "2026-04-05T09:00:00Z",
                "message": {"content": CANARY}
            }),
            assistant("session-a", "message-a", "2026-04-05T09:01:00Z", 2),
        ],
    );
    let output_dir = home.root.join("private-output");
    fs::create_dir_all(&output_dir).unwrap();
    let output = home.run_in(
        &[
            "--archive",
            "--plain",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &output_dir,
    );
    assert!(output.status.success());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(CANARY));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(CANARY));
    assert!(String::from_utf8_lossy(&output.stderr).contains("private prompt content"));
    let archive = fs::read_to_string(output_dir.join("wrapped-archive/project-1.md")).unwrap();
    assert!(archive.contains(CANARY));
    assert!(!output_dir.join("claude-code-wrapped.html").exists());
}

#[test]
fn private_archive_entrypoints_are_bounded() {
    const TAIL_CANARY: &str = "PRIVATE_ENTRYPOINT_TAIL_CANARY_6F42";
    let home = SyntheticHome::new("archive-entrypoint-bound");
    let root = home.transcript_root("config");
    let mut prompt = user_prompt(
        "session-a",
        "user-a",
        "2026-04-05T09:00:00Z",
        "bounded private prompt",
    );
    prompt["entrypoint"] = Value::String(format!("{}{TAIL_CANARY}", "e".repeat(600)));
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            prompt,
            assistant("session-a", "message-a", "2026-04-05T09:01:00Z", 2),
        ],
    );

    let output = home.run(&[
        "--archive",
        "--plain",
        "--data-dir",
        root.to_str().unwrap(),
        "2026",
    ]);
    assert!(output.status.success());
    let archive = fs::read_to_string(home.root.join("wrapped-archive/project-1.md")).unwrap();
    assert!(!archive.contains(TAIL_CANARY));
    assert!(archive.len() < 2_000);
}

#[cfg(unix)]
#[test]
fn archive_rejects_symlinked_roots_without_writing_private_content() {
    use std::os::unix::fs::symlink;

    const CANARY: &str = "PRIVATE_ARCHIVE_SYMLINK_CANARY_9721";
    let home = SyntheticHome::new("archive-root-symlink");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            user_prompt("session-a", "user-a", "2026-04-05T09:00:00Z", CANARY),
            assistant("session-a", "message-a", "2026-04-05T09:01:00Z", 2),
        ],
    );
    let output_dir = home.root.join("symlinked-output");
    let outside = home.root.join("outside");
    fs::create_dir_all(&output_dir).unwrap();
    fs::create_dir_all(&outside).unwrap();
    symlink(&outside, output_dir.join("wrapped-archive")).unwrap();

    let output = home.run_in(
        &[
            "--archive",
            "--plain",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &output_dir,
    );

    assert!(!output.status.success());
    assert!(!outside.join("project-1.md").exists());
    assert!(!String::from_utf8_lossy(&output.stdout).contains(CANARY));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(CANARY));
}

#[cfg(unix)]
#[test]
fn archive_rejects_file_symlinks_and_uses_private_permissions() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    const CANARY: &str = "PRIVATE_ARCHIVE_FILE_CANARY_3842";
    let home = SyntheticHome::new("archive-file-symlink");
    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            user_prompt("session-a", "user-a", "2026-04-05T09:00:00Z", CANARY),
            assistant("session-a", "message-a", "2026-04-05T09:01:00Z", 2),
        ],
    );
    let output_dir = home.root.join("file-symlink-output");
    let archive_dir = output_dir.join("wrapped-archive");
    let outside = home.root.join("outside-private-target.md");
    fs::create_dir_all(&archive_dir).unwrap();
    fs::write(&outside, "must remain unchanged").unwrap();
    symlink(&outside, archive_dir.join("project-1.md")).unwrap();

    let rejected = home.run_in(
        &[
            "--archive",
            "--plain",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &output_dir,
    );
    assert!(!rejected.status.success());
    assert_eq!(
        fs::read_to_string(&outside).unwrap(),
        "must remain unchanged"
    );

    fs::remove_dir_all(&archive_dir).unwrap();
    let accepted = home.run_in(
        &[
            "--archive",
            "--plain",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &output_dir,
    );
    assert!(accepted.status.success());
    let archive_path = archive_dir.join("project-1.md");
    assert!(fs::read_to_string(&archive_path).unwrap().contains(CANARY));
    assert_eq!(
        fs::metadata(&archive_dir).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        fs::metadata(archive_path).unwrap().permissions().mode() & 0o777,
        0o600
    );
}

#[cfg(target_os = "linux")]
#[test]
fn archive_succeeds_without_hard_link_support() {
    const CANARY: &str = "PRIVATE_ARCHIVE_NO_HARD_LINK_CANARY_5106";
    let home = SyntheticHome::new("archive-no-hard-links");
    let shim_source = home.root.join("deny_hard_links.rs");
    let shim = home.root.join("deny_hard_links.so");
    fs::write(
        &shim_source,
        r#"
use std::ffi::{c_char, c_int};

extern "C" {
    fn __errno_location() -> *mut c_int;
}

fn unsupported() -> c_int {
    unsafe { *__errno_location() = 95; }
    -1
}

#[no_mangle]
pub unsafe extern "C" fn link(_old: *const c_char, _new: *const c_char) -> c_int {
    unsupported()
}

#[no_mangle]
pub unsafe extern "C" fn linkat(
    _old_dir: c_int,
    _old: *const c_char,
    _new_dir: c_int,
    _new: *const c_char,
    _flags: c_int,
) -> c_int {
    unsupported()
}
"#,
    )
    .unwrap();
    let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let compiled = Command::new(rustc)
        .args(["--crate-type", "cdylib"])
        .arg(&shim_source)
        .arg("-o")
        .arg(&shim)
        .output()
        .expect("compile hard-link denial shim");
    assert!(
        compiled.status.success(),
        "hard-link denial shim failed to compile: {}",
        String::from_utf8_lossy(&compiled.stderr)
    );

    let root = home.transcript_root("config");
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            user_prompt("session-a", "user-a", "2026-04-05T09:00:00Z", CANARY),
            assistant("session-a", "message-a", "2026-04-05T09:01:00Z", 2),
        ],
    );
    let output_dir = home.root.join("no-hard-link-output");
    fs::create_dir(&output_dir).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
        .args([
            "--archive",
            "--plain",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ])
        .current_dir(&output_dir)
        .env("HOME", home.root.join("isolated-home"))
        .env("XDG_CACHE_HOME", home.root.join("isolated-cache"))
        .env(
            "CLAUDE_CONFIG_DIR",
            home.root.join("isolated-claude-config"),
        )
        .env("NO_COLOR", "1")
        .env("LD_PRELOAD", &shim)
        .output()
        .expect("run ccwrapped without hard-link support");

    assert!(
        output.status.success(),
        "archive required hard links: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        fs::read_to_string(output_dir.join("wrapped-archive/project-1.md"))
            .unwrap()
            .contains(CANARY)
    );
}
