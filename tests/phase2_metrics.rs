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
            "ccwrapped-phase2-{label}-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create synthetic Phase 2 home");
        Self { root }
    }

    fn transcript_root(&self) -> PathBuf {
        let root = self.root.join("config/projects");
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

    fn write_otel(&self, filename: &str, values: &[Value]) -> PathBuf {
        let path = self.root.join(filename);
        let body = values
            .iter()
            .map(Value::to_string)
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(&path, format!("{body}\n")).expect("write synthetic OTel artifact");
        path
    }

    fn run(&self, args: &[&str]) -> Output {
        self.run_with_tz(args, "Pacific/Honolulu")
    }

    fn run_with_tz(&self, args: &[&str], timezone: &str) -> Output {
        Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
            .args(args)
            .current_dir(&self.root)
            .env("HOME", self.root.join("isolated-home"))
            .env("XDG_CACHE_HOME", self.root.join("isolated-cache"))
            .env(
                "CLAUDE_CONFIG_DIR",
                self.root.join("isolated-claude-config"),
            )
            .env("TZ", timezone)
            .env("NO_COLOR", "1")
            .output()
            .expect("run ccwrapped")
    }

    fn json(&self, root: &Path, timezone: &str, year: i32) -> Value {
        successful_json(self.run(&[
            "--json",
            "--timezone",
            timezone,
            "--data-dir",
            root.to_str().unwrap(),
            &year.to_string(),
        ]))
    }
}

impl Drop for SyntheticHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
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

struct AssistantRecord<'a> {
    session: &'a str,
    message: &'a str,
    timestamp: &'a str,
    model: &'a str,
    input: Option<u64>,
    output: Option<u64>,
    cache_creation: Option<u64>,
    cache_read: Option<u64>,
    source_cost: Option<f64>,
}

macro_rules! assistant {
    (
        $session:expr,
        $message:expr,
        $timestamp:expr,
        $model:expr,
        $input:expr,
        $output:expr,
        $cache_creation:expr,
        $cache_read:expr,
        $source_cost:expr $(,)?
    ) => {
        assistant_record(AssistantRecord {
            session: $session,
            message: $message,
            timestamp: $timestamp,
            model: $model,
            input: $input,
            output: $output,
            cache_creation: $cache_creation,
            cache_read: $cache_read,
            source_cost: $source_cost,
        })
    };
}

fn assistant_record(spec: AssistantRecord<'_>) -> Value {
    let mut usage = serde_json::Map::new();
    if let Some(value) = spec.input {
        usage.insert("input_tokens".to_string(), Value::from(value));
    }
    if let Some(value) = spec.output {
        usage.insert("output_tokens".to_string(), Value::from(value));
    }
    if let Some(value) = spec.cache_creation {
        usage.insert(
            "cache_creation_input_tokens".to_string(),
            Value::from(value),
        );
    }
    if let Some(value) = spec.cache_read {
        usage.insert("cache_read_input_tokens".to_string(), Value::from(value));
    }
    let mut record = serde_json::json!({
        "type": "assistant",
        "sessionId": spec.session,
        "timestamp": spec.timestamp,
        "message": {
            "id": spec.message,
            "model": spec.model,
            "usage": usage,
            "content": []
        }
    });
    if let Some(cost) = spec.source_cost {
        record["costUSD"] = Value::from(cost);
    }
    record
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

fn otel_api_request(
    session: &str,
    request: &str,
    timestamp: &str,
    unix_nanos: u64,
    duration_ms: u64,
    subagent: bool,
) -> Value {
    let mut attributes = vec![
        otel_attribute("event.timestamp", Value::String(timestamp.to_string())),
        otel_attribute("session.id", Value::String(session.to_string())),
        otel_attribute("request_id", Value::String(request.to_string())),
        otel_attribute("model", Value::String("claude-sonnet-4-6".to_string())),
        otel_attribute("input_tokens", Value::from(1)),
        otel_attribute("output_tokens", Value::from(1)),
        otel_attribute("cache_read_tokens", Value::from(0)),
        otel_attribute("cache_creation_tokens", Value::from(0)),
        otel_attribute("duration_ms", Value::from(duration_ms)),
    ];
    if subagent {
        attributes.push(otel_attribute(
            "agent_id",
            Value::String(format!("agent-{session}")),
        ));
    }
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
                    "timeUnixNano": unix_nanos.to_string(),
                    "body": {},
                    "attributes": attributes,
                    "eventName": "claude_code.api_request"
                }]
            }]
        }]
    })
}

fn otel_api_request_with_pricing_modifier(
    session: &str,
    request: &str,
    timestamp: &str,
    unix_nanos: u64,
    model: &str,
    speed: &str,
) -> Value {
    let mut value = otel_api_request(session, request, timestamp, unix_nanos, 10, false);
    let attributes = value["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["attributes"]
        .as_array_mut()
        .expect("synthetic API event attributes");
    let model_attribute = attributes
        .iter_mut()
        .find(|attribute| attribute["key"] == "model")
        .expect("synthetic API event model");
    model_attribute["value"]["stringValue"] = Value::String(model.to_string());
    attributes.push(otel_attribute("speed", Value::String(speed.to_string())));
    value
}

fn otel_compaction(session: &str, timestamp: &str, unix_nanos: u64) -> Value {
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
                    "timeUnixNano": unix_nanos.to_string(),
                    "body": {},
                    "attributes": [
                        otel_attribute("event.timestamp", Value::String(timestamp.to_string())),
                        otel_attribute("session.id", Value::String(session.to_string())),
                        otel_attribute("success", Value::Bool(true))
                    ],
                    "eventName": "claude_code.compaction"
                }]
            }]
        }]
    })
}

fn otel_token_metric(
    session: Option<&str>,
    token_type: &str,
    start_nanos: u64,
    end_nanos: u64,
    value: u64,
) -> Value {
    let mut attributes = vec![
        otel_attribute("type", Value::String(token_type.to_string())),
        otel_attribute("model", Value::String("claude-sonnet-4-6".to_string())),
    ];
    if let Some(session) = session {
        attributes.push(otel_attribute(
            "session.id",
            Value::String(session.to_string()),
        ));
    }
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
                            "attributes": attributes,
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

fn assert_cost_domains_reconcile(json: &Value) {
    let reconciliation = &json["canonicalMetrics"]["reconciliation"];
    assert_eq!(reconciliation["status"], "pass");
    let domains = &reconciliation["costDomains"];
    assert_eq!(domains["sourceRecordedSeparate"], "pass");
    assert_eq!(domains["localApiEquivalentSeparate"], "pass");
    assert_eq!(domains["unpricedUsage"], "pass");
    assert_eq!(domains["billingAuthoritative"], "unavailable");
}

fn assert_public_token_dimensions_reconcile(json: &Value) {
    let tokens = &json["canonicalMetrics"]["tokens"];
    let global = &tokens["global"];
    for (dimension_name, dimension) in [
        ("days", tokens["days"].as_array().unwrap()),
        ("models", tokens["models"].as_array().unwrap()),
        ("projects", tokens["projects"].as_array().unwrap()),
        ("sessions", tokens["sessions"].as_array().unwrap()),
    ] {
        let mut sets = dimension
            .iter()
            .map(|entry| &entry["tokens"])
            .collect::<Vec<_>>();
        if dimension_name == "sessions" {
            sets.push(&tokens["unattributed"]);
        } else if dimension_name == "projects" {
            sets.push(&tokens["projectUnattributed"]);
        }
        for category in [
            "input",
            "output",
            "cacheCreation",
            "cacheRead",
            "cacheCreation5m",
            "cacheCreation1h",
            "total",
        ] {
            let observed_sum = sets.iter().fold(0u128, |sum, set| {
                sum.saturating_add(u128::from(set[category]["observed"].as_u64().unwrap()))
            });
            let sample_sum = sets.iter().fold(0u128, |sum, set| {
                sum.saturating_add(u128::from(set[category]["sampleCount"].as_u64().unwrap()))
            });
            let any_overflow = sets
                .iter()
                .any(|set| set[category]["overflowed"].as_bool().unwrap());
            let expected_overflow = any_overflow || observed_sum > u128::from(u64::MAX);
            let expected_observed = if expected_overflow {
                u64::MAX
            } else {
                observed_sum as u64
            };
            assert_eq!(
                global[category]["observed"].as_u64().unwrap(),
                expected_observed,
                "{dimension_name}.{category} observed values must reconcile independently"
            );
            assert_eq!(
                global[category]["sampleCount"].as_u64().unwrap(),
                u64::try_from(sample_sum).unwrap(),
                "{dimension_name}.{category} samples must reconcile independently"
            );
            assert_eq!(
                global[category]["overflowed"].as_bool().unwrap(),
                expected_overflow,
                "{dimension_name}.{category} overflow must reconcile independently"
            );
        }
    }
}

fn assert_public_active_dimensions_reconcile(json: &Value) {
    let active = &json["canonicalMetrics"]["activeTime"];
    let global = active["totalActiveSeconds"].as_u64().unwrap();
    assert_eq!(
        active["mainExclusiveSeconds"].as_u64().unwrap()
            + active["subagentExclusiveSeconds"].as_u64().unwrap(),
        global,
        "main and subagent exclusive seconds must reconcile independently"
    );
    for dimension in ["days", "models", "projects", "sessions"] {
        let mut sum = active[dimension]
            .as_array()
            .unwrap()
            .iter()
            .map(|entry| entry["activeSeconds"].as_u64().unwrap())
            .sum::<u64>();
        if dimension == "projects" {
            sum = sum.saturating_add(active["projectUnattributedActiveSeconds"].as_u64().unwrap());
        }
        assert_eq!(
            sum, global,
            "{dimension} active seconds must reconcile independently"
        );
    }
}

fn canonical_fact_lines(json: &Value) -> Vec<String> {
    let period = json["dataCoverage"]["selectedPeriod"].as_str().unwrap();
    let timezone = json["dataCoverage"]["timezone"].as_str().unwrap();
    let active = &json["canonicalMetrics"]["activeTime"];
    let tokens = &json["canonicalMetrics"]["tokens"]["global"];
    let cost = &json["canonicalMetrics"]["cost"]["localApiEquivalent"];
    let read = &json["canonicalMetrics"]["cache"]["readShare"];
    let write = &json["canonicalMetrics"]["cache"]["writeShare"];
    let limitations = |value: &Value| {
        let values = value["limitations"]
            .as_array()
            .expect("canonical fact exposes limitations");
        if values.is_empty() {
            "none".to_string()
        } else {
            values
                .iter()
                .map(|value| value.as_str().expect("limitation is text"))
                .collect::<Vec<_>>()
                .join("|")
        }
    };
    let money = cost["amountUsd"]
        .as_f64()
        .map_or_else(|| "unavailable".to_string(), |value| format!("{value:.6}"));
    let ratio_value = |ratio: &Value| {
        ratio["valuePct"]
            .as_f64()
            .map_or_else(|| "unavailable".to_string(), |value| format!("{value:.1}"))
    };
    let mut lines = vec![
        format!(
            "FACT method={} metric=activity.active value={} unit={} availability={} intervals={} period={} timezone={} thresholdSeconds={} limitations={}",
            active["methodId"].as_str().unwrap(),
            active["totalActiveSeconds"].as_u64().unwrap(),
            active["unit"].as_str().unwrap(),
            active["availability"].as_str().unwrap(),
            active["intervalCount"].as_u64().unwrap(),
            period,
            timezone,
            active["thresholdSeconds"].as_u64().unwrap(),
            limitations(active)
        ),
    ];
    for (json_name, fact_name) in [
        ("input", "input"),
        ("output", "output"),
        ("cacheCreation", "cacheCreation"),
        ("cacheRead", "cacheRead"),
        ("cacheCreation5m", "cacheCreation5m"),
        ("cacheCreation1h", "cacheCreation1h"),
    ] {
        let token = &tokens[json_name];
        lines.push(format!(
            "FACT method={} metric=tokens.{} value={} unit={} availability={} samples={} overflowed={} period={} timezone={} limitations={}",
            token["methodId"].as_str().unwrap(),
            fact_name,
            token["observed"].as_u64().unwrap(),
            token["unit"].as_str().unwrap(),
            token["availability"].as_str().unwrap(),
            token["sampleCount"].as_u64().unwrap(),
            token["overflowed"].as_bool().unwrap(),
            period,
            timezone,
            limitations(token)
        ));
    }
    let total = &tokens["total"];
    lines.extend([
        format!(
            "FACT method={} metric=tokens.total value={} unit={} availability={} samples={} overflowed={} categories=input+output+cacheCreation+cacheRead period={} timezone={} limitations={}",
            total["methodId"].as_str().unwrap(),
            total["observed"].as_u64().unwrap(),
            total["unit"].as_str().unwrap(),
            total["availability"].as_str().unwrap(),
            total["sampleCount"].as_u64().unwrap(),
            total["overflowed"].as_bool().unwrap(),
            period,
            timezone,
            limitations(total)
        ),
        format!(
            "FACT method={} metric=cost.localApiEquivalent value={} unit={} availability={} samples={} period={} registry={} limitations={}",
            cost["methodId"].as_str().unwrap(),
            money,
            cost["unit"].as_str().unwrap(),
            cost["availability"].as_str().unwrap(),
            cost["sampleCount"].as_u64().unwrap(),
            period,
            json["methodology"]["pricingRegistry"]["version"]
                .as_str()
                .unwrap(),
            limitations(cost)
        ),
        format!(
            "FACT method={} metric=cache.readShare value={} unit={} numerator={} denominator={} availability={} samples={} overflowed={} period={} limitations={}",
            read["methodId"].as_str().unwrap(),
            ratio_value(read),
            read["unit"].as_str().unwrap(),
            read["numerator"].as_u64().unwrap(),
            read["denominator"].as_u64().unwrap(),
            read["availability"].as_str().unwrap(),
            read["sampleCount"].as_u64().unwrap(),
            read["overflowed"].as_bool().unwrap(),
            period,
            limitations(read)
        ),
        format!(
            "FACT method={} metric=cache.writeShare value={} unit={} numerator={} denominator={} availability={} samples={} overflowed={} period={} limitations={}",
            write["methodId"].as_str().unwrap(),
            ratio_value(write),
            write["unit"].as_str().unwrap(),
            write["numerator"].as_u64().unwrap(),
            write["denominator"].as_u64().unwrap(),
            write["availability"].as_str().unwrap(),
            write["sampleCount"].as_u64().unwrap(),
            write["overflowed"].as_bool().unwrap(),
            period,
            limitations(write)
        ),
    ]);
    lines
}

fn renderers_containing_facts(
    home: &SyntheticHome,
    args: &[&str],
    json: &Value,
) -> Vec<(&'static str, String)> {
    let mut render_args = vec!["--all", "--plain"];
    render_args.extend_from_slice(args);
    let rendered = home.run(&render_args);
    assert!(
        rendered.status.success(),
        "status={}\nstdout={}\nstderr={}",
        rendered.status,
        String::from_utf8_lossy(&rendered.stdout),
        String::from_utf8_lossy(&rendered.stderr)
    );
    let outputs = vec![
        (
            "terminal",
            String::from_utf8(rendered.stdout).expect("terminal is UTF-8"),
        ),
        (
            "html",
            fs::read_to_string(home.root.join("claude-code-wrapped.html"))
                .expect("read HTML report"),
        ),
        (
            "markdown",
            fs::read_to_string(home.root.join("claude-code-wrapped.md"))
                .expect("read Markdown report"),
        ),
        (
            "card",
            fs::read_to_string(home.root.join("claude-code-wrapped-card.html"))
                .expect("read card report"),
        ),
    ];
    for fact in canonical_fact_lines(json) {
        for (label, output) in &outputs {
            assert!(
                output.contains(&fact),
                "{label} omitted canonical fact {fact}"
            );
        }
    }
    outputs
}

fn collect_json_strings(value: &Value, output: &mut Vec<String>) {
    match value {
        Value::String(value) => output.push(value.clone()),
        Value::Array(values) => {
            for value in values {
                collect_json_strings(value, output);
            }
        }
        Value::Object(values) => {
            for value in values.values() {
                collect_json_strings(value, output);
            }
        }
        _ => {}
    }
}

#[test]
fn f019_selected_iana_timezone_controls_year_day_hour_and_labels() {
    let home = SyntheticHome::new("f019-timezone");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-a",
                "2025-12-31T23:30:00-02:00",
                "claude-sonnet-4-6",
                Some(1),
                Some(7),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-b",
                "2026-01-01T23:30:00-02:00",
                "claude-sonnet-4-6",
                Some(1),
                Some(7),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );

    let new_york = home.json(&root, "America/New_York", 2025);
    assert_eq!(new_york["dataCoverage"]["timezone"], "America/New_York");
    assert_eq!(
        new_york["costAnalysis"]["dailyCosts"][0]["date"],
        "2025-12-31"
    );
    assert_eq!(new_york["sessionIntel"]["hourDistribution"][20], 1);
    assert_eq!(
        new_york["wrappedStory"]["favoriteWeekday"]["label"],
        "Wednesday"
    );
    assert_eq!(new_york["wrappedStory"]["longestStreak"], 1);
    assert!(
        new_york["inflection"].is_null(),
        "legacy comparison-shaped labels stay absent until Phase 3 supplies a selected-zone proof object"
    );

    let utc = home.json(&root, "UTC", 2026);
    assert_eq!(utc["dataCoverage"]["timezone"], "UTC");
    assert_eq!(utc["costAnalysis"]["dailyCosts"][0]["date"], "2026-01-01");
    assert_eq!(utc["sessionIntel"]["hourDistribution"][1], 2);
    assert_eq!(utc["costAnalysis"]["dailyCosts"][1]["date"], "2026-01-02");
    assert_eq!(utc["wrappedStory"]["favoriteWeekday"]["label"], "Thursday");
    assert_eq!(utc["wrappedStory"]["longestStreak"], 2);
    assert!(
        utc["inflection"].is_null(),
        "legacy comparison-shaped labels stay absent until Phase 3 supplies a selected-zone proof object"
    );
}

#[test]
fn f020_dst_gap_fold_and_leap_day_attribute_real_instants_only() {
    let home = SyntheticHome::new("f020-dst");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-gap-before",
                "2026-03-08T06:30:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-gap-after",
                "2026-03-08T07:30:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-fold-first",
                "2026-11-01T05:30:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-fold-second",
                "2026-11-01T06:30:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );
    let json = home.json(&root, "America/New_York", 2026);
    assert_eq!(json["sessionIntel"]["hourDistribution"][1], 3);
    assert_eq!(json["sessionIntel"]["hourDistribution"][2], 0);
    assert_eq!(json["sessionIntel"]["hourDistribution"][3], 1);

    let leap_home = SyntheticHome::new("f020-leap");
    let leap_root = leap_home.transcript_root();
    leap_home.write_session(
        &leap_root,
        "project-leap",
        "session-leap",
        &[assistant!(
            "session-leap",
            "message-leap",
            "2024-02-28T10:30:00Z",
            "claude-sonnet-4-6",
            Some(1),
            Some(1),
            Some(0),
            Some(0),
            None,
        )],
    );
    let leap = leap_home.json(&leap_root, "Pacific/Kiritimati", 2024);
    assert_eq!(leap["costAnalysis"]["dailyCosts"][0]["date"], "2024-02-29");
}

#[test]
fn f020_skipped_local_date_uses_next_real_instant() {
    let home = SyntheticHome::new("f020-skipped-local-date");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-apia",
        "session-apia",
        &[
            assistant!(
                "session-apia",
                "message-before",
                "2011-12-30T09:58:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-apia",
                "message-after",
                "2011-12-30T10:02:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );

    let json = home.json(&root, "Pacific/Apia", 2011);
    let daily_dates = json["costAnalysis"]["dailyCosts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|day| day["date"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(daily_dates, ["2011-12-29", "2011-12-31"]);
    assert!(!daily_dates.contains(&"2011-12-30"));
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["elapsedSeconds"],
        240
    );
    assert_eq!(
        json["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        240
    );
    assert_eq!(
        json["canonicalMetrics"]["activeTime"]["days"][0]["activeSeconds"],
        120
    );
    assert_eq!(
        json["canonicalMetrics"]["activeTime"]["days"][1]["activeSeconds"],
        120
    );
}

#[test]
fn observed_day_span_counts_inclusive_local_calendar_dates() {
    let home = SyntheticHome::new("observed-day-span");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-a",
                "2026-04-01T23:59:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-b",
                "2026-04-02T00:01:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );
    let json = home.json(&root, "UTC", 2026);
    assert_eq!(json["dataCoverage"]["observedDaySpan"], 2);
}

#[test]
fn f021_resumed_session_keeps_elapsed_and_capped_active_time_separate() {
    let home = SyntheticHome::new("f021-resume");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-a",
                "2026-01-01T10:00:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-b",
                "2026-01-03T10:00:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );
    let json = home.json(&root, "UTC", 2026);
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["elapsedSeconds"],
        172_800
    );
    assert_eq!(
        json["sessionBreakdown"]["sessions"][0]["activeSeconds"],
        300
    );
    assert_eq!(
        json["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        300
    );
    assert_eq!(
        json["canonicalMetrics"]["activeTime"]["thresholdSeconds"],
        300
    );
}

#[test]
fn f022_overlapping_main_subagent_and_direct_intervals_union_once() {
    let home = SyntheticHome::new("f022-union");
    let otel = home.write_otel(
        "activity.jsonl",
        &[
            otel_api_request(
                "main-session",
                "main-a",
                "2026-04-05T10:00:00Z",
                1_775_383_200_000_000_000,
                240_000,
                false,
            ),
            otel_api_request(
                "main-session",
                "main-b",
                "2026-04-05T10:10:00Z",
                1_775_383_800_000_000_000,
                0,
                false,
            ),
            otel_api_request(
                "subagent-session",
                "subagent-a",
                "2026-04-05T10:02:00Z",
                1_775_383_320_000_000_000,
                0,
                true,
            ),
            otel_api_request(
                "subagent-session",
                "subagent-b",
                "2026-04-05T10:07:00Z",
                1_775_383_620_000_000_000,
                0,
                true,
            ),
        ],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));
    let active = &json["canonicalMetrics"]["activeTime"];
    assert_eq!(active["totalActiveSeconds"], 660);
    assert_eq!(active["mainExclusiveSeconds"], 540);
    assert_eq!(active["subagentExclusiveSeconds"], 120);
    assert_eq!(active["days"][0]["activeSeconds"], 660);
    assert_eq!(active["intervalCount"], 3);
    assert_eq!(active["unit"], "seconds");
    assert_eq!(active["availability"], "available");
    assert_public_token_dimensions_reconcile(&json);
    assert_public_active_dimensions_reconcile(&json);
    assert_eq!(
        json["canonicalMetrics"]["reconciliation"]["activeTimeDimensions"]["days"],
        "pass"
    );
    assert_eq!(
        json["canonicalMetrics"]["reconciliation"]["activeTimeDimensions"]["sessions"],
        "pass"
    );
    assert_cost_domains_reconcile(&json);
    assert_eq!(
        json["methodology"]["methods"]["activity/capped-interval-union/v1"]["parameters"]
            ["directTimestampConvention"],
        "source-timestamp-is-interval-end"
    );
}

#[test]
fn f022_parent_group_and_dimension_inclusive_values_are_non_additive() {
    let home = SyntheticHome::new("f022-parent-group");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-main",
        &[
            assistant!(
                "session-main",
                "main-a",
                "2026-04-05T10:00:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-main",
                "main-b",
                "2026-04-05T10:10:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );
    home.write_session(
        &root,
        "project-alpha/session-main/subagents",
        "session-child",
        &[
            assistant!(
                "session-child",
                "child-a",
                "2026-04-05T10:02:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-child",
                "child-b",
                "2026-04-05T10:07:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );

    let json = home.json(&root, "UTC", 2026);
    let active = &json["canonicalMetrics"]["activeTime"];
    assert_eq!(active["totalActiveSeconds"], 420);
    assert_eq!(active["mainExclusiveSeconds"], 300);
    assert_eq!(active["subagentExclusiveSeconds"], 120);
    assert_eq!(active["projects"][0]["activeSeconds"], 420);
    assert_eq!(active["projects"][0]["inclusiveActiveSeconds"], 420);
    assert_eq!(active["models"][0]["activeSeconds"], 420);
    assert_eq!(active["models"][0]["inclusiveActiveSeconds"], 420);

    let mut session_values = active["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|session| {
            (
                session["activeSeconds"].as_u64().unwrap(),
                session["inclusiveActiveSeconds"].as_u64().unwrap(),
            )
        })
        .collect::<Vec<_>>();
    session_values.sort_unstable();
    assert_eq!(session_values, vec![(120, 300), (300, 300)]);
    assert!(
        session_values
            .iter()
            .map(|(_, inclusive)| inclusive)
            .sum::<u64>()
            > active["totalActiveSeconds"].as_u64().unwrap(),
        "inclusive session projections intentionally overlap"
    );

    let parent = &json["sessionBreakdown"]["sessions"][0];
    assert_eq!(parent["activeSeconds"], 300);
    assert_eq!(parent["inclusiveActiveSeconds"], 420);
    assert_eq!(parent["subagents"][0]["activeSeconds"], 120);
    assert_eq!(
        parent["subagents"][0]["parentSessionId"],
        parent["sessionId"]
    );
    assert_eq!(
        json["canonicalMetrics"]["reconciliation"]["activeTimeDimensions"]["models"],
        "pass"
    );
    assert_eq!(
        json["canonicalMetrics"]["reconciliation"]["activeTimeDimensions"]["projects"],
        "pass"
    );
    assert_eq!(
        json["canonicalMetrics"]["reconciliation"]["activeTimeDimensions"]["sessions"],
        "pass"
    );
    assert_public_token_dimensions_reconcile(&json);
    assert_public_active_dimensions_reconcile(&json);
}

#[test]
fn f022_period_clipping_local_midnight_and_dst_use_real_instant_durations() {
    let clip_home = SyntheticHome::new("f022-period-clip");
    let clipped = clip_home.write_otel(
        "clip.jsonl",
        &[otel_api_request(
            "clip-session",
            "clip-request",
            "2026-01-01T00:02:00Z",
            1_767_225_720_000_000_000,
            300_000,
            false,
        )],
    );
    let clipped_json = successful_json(clip_home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--otel-file",
        clipped.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        clipped_json["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        120
    );
    assert_eq!(
        clipped_json["canonicalMetrics"]["activeTime"]["days"][0]["activeSeconds"],
        120
    );
    assert_public_token_dimensions_reconcile(&clipped_json);
    assert_public_active_dimensions_reconcile(&clipped_json);
    assert_cost_domains_reconcile(&clipped_json);

    let midnight_home = SyntheticHome::new("f022-midnight-dst");
    let midnight_root = midnight_home.transcript_root();
    midnight_home.write_session(
        &midnight_root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-a",
                "2026-01-02T04:58:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-b",
                "2026-01-02T05:02:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );
    let midnight = midnight_home.json(&midnight_root, "America/New_York", 2026);
    assert_eq!(
        midnight["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        240
    );
    assert_eq!(
        midnight["canonicalMetrics"]["activeTime"]["days"][0]["activeSeconds"],
        120
    );
    assert_eq!(
        midnight["canonicalMetrics"]["activeTime"]["days"][1]["activeSeconds"],
        120
    );
    assert_eq!(midnight["costAnalysis"]["dailyCosts"][0]["sessionCount"], 1);
    assert_eq!(midnight["costAnalysis"]["dailyCosts"][1]["sessionCount"], 1);
    assert_eq!(
        midnight["sessionBreakdown"]["sessions"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_public_token_dimensions_reconcile(&midnight);
    assert_public_active_dimensions_reconcile(&midnight);

    let dst_home = SyntheticHome::new("f022-dst-duration");
    let dst = dst_home.write_otel(
        "dst.jsonl",
        &[
            otel_api_request(
                "spring-session",
                "spring-request",
                "2026-03-08T07:05:00Z",
                1_772_953_500_000_000_000,
                600_000,
                false,
            ),
            otel_api_request(
                "fall-session",
                "fall-request",
                "2026-11-01T06:05:00Z",
                1_793_513_100_000_000_000,
                600_000,
                false,
            ),
        ],
    );
    let dst_json = successful_json(dst_home.run(&[
        "--json",
        "--timezone",
        "America/New_York",
        "--otel-file",
        dst.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        dst_json["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        1_200
    );
    assert_public_token_dimensions_reconcile(&dst_json);
    assert_public_active_dimensions_reconcile(&dst_json);
}

#[test]
fn f023_singleton_out_of_order_and_threshold_boundaries_are_deterministic() {
    let singleton_home = SyntheticHome::new("f023-singleton");
    let singleton_root = singleton_home.transcript_root();
    singleton_home.write_session(
        &singleton_root,
        "project-alpha",
        "session-a",
        &[assistant!(
            "session-a",
            "message-a",
            "2026-04-05T10:00:00Z",
            "claude-sonnet-4-6",
            Some(1),
            Some(1),
            Some(0),
            Some(0),
            None,
        )],
    );
    let singleton = singleton_home.json(&singleton_root, "UTC", 2026);
    assert_eq!(
        singleton["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        0
    );
    assert_eq!(
        singleton["canonicalMetrics"]["activeTime"]["intervalCount"],
        0
    );
    assert_eq!(
        singleton["sessionBreakdown"]["sessions"][0]["elapsedSeconds"],
        0
    );

    let ordered_home = SyntheticHome::new("f023-ordered");
    let ordered_root = ordered_home.transcript_root();
    let earlier = assistant!(
        "session-a",
        "message-a",
        "2026-04-05T10:00:00Z",
        "claude-sonnet-4-6",
        Some(1),
        Some(1),
        Some(0),
        Some(0),
        None,
    );
    let later = assistant!(
        "session-a",
        "message-b",
        "2026-04-05T10:10:00Z",
        "claude-sonnet-4-6",
        Some(1),
        Some(1),
        Some(0),
        Some(0),
        None,
    );
    ordered_home.write_session(
        &ordered_root,
        "project-alpha",
        "session-a",
        &[earlier.clone(), later.clone()],
    );
    let reversed_home = SyntheticHome::new("f023-reversed");
    let reversed_root = reversed_home.transcript_root();
    reversed_home.write_session(
        &reversed_root,
        "project-alpha",
        "session-a",
        &[later, earlier],
    );
    let ordered = successful_json(ordered_home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--active-threshold-minutes",
        "2",
        "--data-dir",
        ordered_root.to_str().unwrap(),
        "2026",
    ]));
    let reversed = successful_json(reversed_home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--active-threshold-minutes",
        "2",
        "--data-dir",
        reversed_root.to_str().unwrap(),
        "2026",
    ]));
    assert_eq!(
        ordered["canonicalMetrics"]["activeTime"],
        reversed["canonicalMetrics"]["activeTime"]
    );
    assert_eq!(
        ordered["canonicalMetrics"]["activeTime"]["totalActiveSeconds"],
        120
    );
    assert_eq!(
        ordered["canonicalMetrics"]["activeTime"]["thresholdSeconds"],
        120
    );

    for invalid in ["0", "1441"] {
        let output = ordered_home.run(&[
            "--json",
            "--timezone",
            "UTC",
            "--active-threshold-minutes",
            invalid,
            "--data-dir",
            ordered_root.to_str().unwrap(),
            "2026",
        ]);
        assert!(!output.status.success());
        let error: Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(error["code"], "E_ACTIVE_THRESHOLD_INVALID");
    }
}

#[test]
fn f024_explicit_zero_remains_distinct_from_an_absent_token_category() {
    let home = SyntheticHome::new("f024-token-presence");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant!(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            "claude-sonnet-4-6",
            Some(2),
            Some(0),
            None,
            None,
            None,
        )],
    );
    let json = home.json(&root, "UTC", 2026);
    let tokens = &json["canonicalMetrics"]["tokens"]["global"];
    assert_eq!(tokens["output"]["observed"], 0);
    assert_eq!(tokens["output"]["availability"], "available");
    assert_eq!(tokens["output"]["sampleCount"], 1);
    assert_eq!(tokens["cacheRead"]["availability"], "unavailable");
    assert_eq!(tokens["cacheRead"]["sampleCount"], 0);
    assert!(tokens["output"]["limitations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!tokens["cacheRead"]["limitations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_public_token_dimensions_reconcile(&json);
    renderers_containing_facts(
        &home,
        &[
            "--timezone",
            "UTC",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &json,
    );
}

#[test]
fn tokenless_direct_requests_remain_in_request_mix_and_cost_completeness() {
    let home = SyntheticHome::new("tokenless-direct-request");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-priced",
                "2026-04-05T09:00:00Z",
                "claude-sonnet-4-6",
                Some(10),
                Some(20),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-tokenless",
                "2026-04-05T09:01:00Z",
                "claude-sonnet-4-6",
                None,
                None,
                None,
                None,
                None,
            ),
        ],
    );

    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["coverage"], "partial");
    assert_eq!(cost["pricedRequests"], 1);
    assert_eq!(cost["unpricedRequests"], 1);
    assert_eq!(cost["pricedTokens"], 30);
    assert_eq!(cost["unpricedTokens"], 0);
    assert_eq!(cost["models"].as_array().unwrap().len(), 1);
    assert_eq!(cost["models"][0]["pricedRequests"], 1);
    assert_eq!(cost["models"][0]["unpricedRequests"], 1);
    assert_eq!(cost["models"][0]["coverage"], "partial");

    assert_eq!(json["modelRouting"]["observations"], 2);
    assert_eq!(json["modelRouting"]["sonnetPct"], 100);
    assert_eq!(
        json["modelRouting"]["opusPct"].as_u64().unwrap()
            + json["modelRouting"]["sonnetPct"].as_u64().unwrap()
            + json["modelRouting"]["haikuPct"].as_u64().unwrap()
            + json["modelRouting"]["otherPct"].as_u64().unwrap()
            + json["modelRouting"]["unknownPct"].as_u64().unwrap(),
        100
    );
    assert_eq!(
        json["canonicalMetrics"]["tokens"]["global"]["output"]["sampleCount"],
        1
    );
    assert_public_token_dimensions_reconcile(&json);
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f024_aggregate_metric_categories_form_one_complete_observation() {
    let home = SyntheticHome::new("f024-aggregate-family");
    let start = 1_775_379_000_000_000_000;
    let end = 1_775_379_600_000_000_000;
    let metrics = [
        ("input", 10),
        ("output", 20),
        ("cacheRead", 30),
        ("cacheCreation", 40),
    ]
    .into_iter()
    .map(|(category, value)| otel_token_metric(Some("metric-session"), category, start, end, value))
    .collect::<Vec<_>>();
    let otel = home.write_otel("aggregate-family.jsonl", &metrics);
    let args = [
        "--timezone",
        "UTC",
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ];
    let json = successful_json(home.run(&["--json"].into_iter().chain(args).collect::<Vec<_>>()));
    let global = &json["canonicalMetrics"]["tokens"]["global"];
    for category in ["input", "output", "cacheCreation", "cacheRead"] {
        assert_eq!(
            global[category]["availability"], "available",
            "aggregate category {category} must be complete"
        );
        assert_eq!(global[category]["sampleCount"], 1);
        assert!(global[category]["limitations"]
            .as_array()
            .unwrap()
            .is_empty());
    }
    assert_eq!(global["total"]["availability"], "available");
    assert_eq!(global["total"]["sampleCount"], 1);
    assert_eq!(
        json["canonicalMetrics"]["cache"]["readShare"]["valuePct"],
        75.0
    );
    assert_eq!(
        json["canonicalMetrics"]["cache"]["writeShare"]["valuePct"],
        80.0
    );
    assert_public_token_dimensions_reconcile(&json);
    for dimension in ["days", "models", "projects", "sessionsPlusUnattributed"] {
        assert_eq!(
            json["canonicalMetrics"]["reconciliation"]["tokenDimensions"][dimension],
            "pass"
        );
    }
    renderers_containing_facts(&home, &args, &json);
}

#[test]
fn f024_ttl_composition_overflow_and_dimensions_reconcile() {
    let home = SyntheticHome::new("f024-ttl-overflow");
    let root = home.transcript_root();
    let mut first = assistant!(
        "session-a",
        "message-a",
        "2026-04-05T09:00:00Z",
        "claude-sonnet-4-6",
        Some(u64::MAX),
        Some(u64::MAX),
        Some(3_000_000),
        Some(0),
        None,
    );
    first["message"]["usage"]["cache_creation"] = serde_json::json!({
        "ephemeral_5m_input_tokens": 1_000_000,
        "ephemeral_1h_input_tokens": 2_000_000
    });
    let second = assistant!(
        "session-a",
        "message-b",
        "2026-04-05T09:01:00Z",
        "claude-sonnet-4-6",
        Some(u64::MAX),
        Some(u64::MAX),
        Some(0),
        Some(0),
        None,
    );
    home.write_session(&root, "project-alpha", "session-a", &[first, second]);
    let json = home.json(&root, "UTC", 2026);
    let global = &json["canonicalMetrics"]["tokens"]["global"];
    assert_eq!(global["input"]["observed"], u64::MAX);
    assert_eq!(global["input"]["overflowed"], true);
    assert_eq!(global["cacheCreation"]["observed"], 3_000_000);
    assert_eq!(global["cacheCreation5m"]["observed"], 1_000_000);
    assert_eq!(global["cacheCreation1h"]["observed"], 2_000_000);
    assert_eq!(global["total"]["overflowed"], true);
    assert_eq!(global["total"]["unit"], "tokens");
    assert!(!global["input"]["limitations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(!global["total"]["limitations"]
        .as_array()
        .unwrap()
        .is_empty());
    assert_public_token_dimensions_reconcile(&json);
    assert_cost_domains_reconcile(&json);
    for dimension in ["days", "models", "projects", "sessionsPlusUnattributed"] {
        assert_eq!(
            json["canonicalMetrics"]["reconciliation"]["tokenDimensions"][dimension],
            "pass"
        );
    }
    renderers_containing_facts(
        &home,
        &[
            "--timezone",
            "UTC",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &json,
    );
}

#[test]
fn f025_cache_shares_use_documented_denominators_and_zero_is_unavailable() {
    let home = SyntheticHome::new("f025-cache");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant!(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            "claude-sonnet-4-6",
            Some(100),
            Some(10),
            Some(100),
            Some(900),
            None,
        )],
    );
    let json = home.json(&root, "UTC", 2026);
    let cache = &json["canonicalMetrics"]["cache"];
    assert_cost_domains_reconcile(&json);
    assert_eq!(cache["readShare"]["numerator"], 900);
    assert_eq!(cache["readShare"]["denominator"], 1000);
    assert_eq!(cache["readShare"]["valuePct"], 90.0);
    assert_eq!(cache["writeShare"]["numerator"], 100);
    assert_eq!(cache["writeShare"]["denominator"], 200);
    assert_eq!(cache["writeShare"]["valuePct"], 50.0);
    assert_eq!(json["cacheHealth"]["estimatedBreaks"], 0);
    assert_eq!(json["cacheHealth"]["grade"]["letter"], "N/A");
    renderers_containing_facts(
        &home,
        &[
            "--timezone",
            "UTC",
            "--data-dir",
            root.to_str().unwrap(),
            "2026",
        ],
        &json,
    );

    let zero_home = SyntheticHome::new("f025-cache-zero");
    let zero_root = zero_home.transcript_root();
    zero_home.write_session(
        &zero_root,
        "project-zero",
        "session-zero",
        &[assistant!(
            "session-zero",
            "message-zero",
            "2026-04-05T09:00:00Z",
            "claude-sonnet-4-6",
            Some(0),
            Some(1),
            Some(0),
            Some(0),
            None,
        )],
    );
    let zero = zero_home.json(&zero_root, "UTC", 2026);
    assert!(zero["canonicalMetrics"]["cache"]["readShare"]["valuePct"].is_null());
    assert_eq!(
        zero["canonicalMetrics"]["cache"]["readShare"]["availability"],
        "unavailable"
    );
    renderers_containing_facts(
        &zero_home,
        &[
            "--timezone",
            "UTC",
            "--data-dir",
            zero_root.to_str().unwrap(),
            "2026",
        ],
        &zero,
    );

    let partial_home = SyntheticHome::new("f025-cache-partial");
    let partial_root = partial_home.transcript_root();
    partial_home.write_session(
        &partial_root,
        "project-partial",
        "session-partial",
        &[
            assistant!(
                "session-partial",
                "message-present",
                "2026-04-05T09:00:00Z",
                "claude-sonnet-4-6",
                Some(100),
                Some(1),
                Some(0),
                Some(100),
                None,
            ),
            assistant!(
                "session-partial",
                "message-missing",
                "2026-04-05T09:01:00Z",
                "claude-sonnet-4-6",
                Some(100),
                Some(1),
                Some(0),
                None,
                None,
            ),
        ],
    );
    let partial = partial_home.json(&partial_root, "UTC", 2026);
    assert_eq!(
        partial["canonicalMetrics"]["tokens"]["global"]["cacheRead"]["availability"],
        "partial"
    );
    assert!(partial["canonicalMetrics"]["cache"]["readShare"]["valuePct"].is_null());
    assert_eq!(
        partial["canonicalMetrics"]["cache"]["readShare"]["availability"],
        "unavailable"
    );
    renderers_containing_facts(
        &partial_home,
        &[
            "--timezone",
            "UTC",
            "--data-dir",
            partial_root.to_str().unwrap(),
            "2026",
        ],
        &partial,
    );

    let saturated_home = SyntheticHome::new("f025-cache-saturated");
    let saturated_root = saturated_home.transcript_root();
    saturated_home.write_session(
        &saturated_root,
        "project-saturated",
        "session-saturated",
        &[
            assistant!(
                "session-saturated",
                "message-a",
                "2026-04-05T09:00:00Z",
                "claude-sonnet-4-6",
                Some(u64::MAX),
                Some(1),
                Some(0),
                Some(u64::MAX),
                None,
            ),
            assistant!(
                "session-saturated",
                "message-b",
                "2026-04-05T09:01:00Z",
                "claude-sonnet-4-6",
                Some(1),
                Some(1),
                Some(0),
                Some(1),
                None,
            ),
        ],
    );
    let saturated = saturated_home.json(&saturated_root, "UTC", 2026);
    let saturated_read = &saturated["canonicalMetrics"]["cache"]["readShare"];
    assert!(saturated_read["valuePct"].is_null());
    assert_eq!(saturated_read["availability"], "unavailable");
    assert_eq!(saturated_read["overflowed"], true);
    assert!(!saturated_read["limitations"].as_array().unwrap().is_empty());
    renderers_containing_facts(
        &saturated_home,
        &[
            "--timezone",
            "UTC",
            "--data-dir",
            saturated_root.to_str().unwrap(),
            "2026",
        ],
        &saturated,
    );
}

#[test]
fn f026_f027_f028_exact_model_prices_effective_dates_and_unknown_coverage() {
    let home = SyntheticHome::new("f026-f028-pricing");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-prelaunch",
                "2026-06-29T12:00:00Z",
                "claude-sonnet-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-intro",
                "2026-08-31T12:00:00Z",
                "claude-sonnet-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                Some(7.25),
            ),
            assistant!(
                "session-a",
                "message-standard",
                "2026-09-01T12:00:00Z",
                "claude-sonnet-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-future",
                "2026-09-02T12:00:00Z",
                "claude-sonnet-99-9",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-retired",
                "2026-02-19T12:00:00Z",
                "claude-3-5-haiku-20241022",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );
    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["sourceRecorded"]["amountUsd"], 7.25);
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], 30.0);
    assert!(cost["billingAuthoritative"]["amountUsd"].is_null());
    assert_eq!(cost["pricedTokens"], 4_000_000);
    assert_eq!(cost["unpricedTokens"], 6_000_000);
    assert_eq!(cost["pricedRequests"], 2);
    assert_eq!(cost["unpricedRequests"], 3);
    let sonnet_rows = cost["models"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|row| row["canonicalModel"] == "claude-sonnet-5")
        .collect::<Vec<_>>();
    assert_eq!(sonnet_rows.len(), 3);
    assert!(sonnet_rows.iter().any(|row| {
        row["pricingKey"].is_null()
            && row["coverage"] == "unavailable"
            && row["unpricedRequests"] == 1
    }));
    assert!(sonnet_rows.iter().any(|row| {
        row["pricingKey"] == "anthropic-api/claude-sonnet-5/2026-06-30/2026-08-31/standard"
            && row["localApiEquivalentUsd"] == 12.0
            && row["pricedRequests"] == 1
    }));
    assert!(sonnet_rows.iter().any(|row| {
        row["pricingKey"] == "anthropic-api/claude-sonnet-5/2026-09-01/open/standard"
            && row["localApiEquivalentUsd"] == 18.0
            && row["pricedRequests"] == 1
    }));
    assert_eq!(json["costAnalysis"]["totalCost"], 30.0);
    assert_eq!(
        json["costAnalysis"]["dailyCosts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|day| day["cost"].as_f64().unwrap())
            .sum::<f64>(),
        30.0
    );
    assert_eq!(
        json["methodology"]["pricingRegistry"]["version"],
        "anthropic-api-2026-07-19"
    );
    assert_eq!(
        json["methodology"]["pricingRegistry"]["accessDate"],
        "2026-07-19"
    );
    assert_eq!(
        json["methodology"]["pricingRegistry"]["selectionPolicy"],
        "pricing/exact-provider-model-interval-modifier/v1"
    );
    let records = json["methodology"]["pricingRegistry"]["records"]
        .as_array()
        .expect("report pins the complete registry inventory");
    assert_eq!(records.len(), 17);
    assert!(records.windows(2).all(|pair| {
        let key = |record: &Value| {
            format!(
                "{}\0{}\0{}\0{}\0{}",
                record["provider"].as_str().unwrap(),
                record["canonicalModel"].as_str().unwrap(),
                record["effectiveStart"].as_str().unwrap_or(""),
                record["effectiveEnd"].as_str().unwrap_or(""),
                record["modifier"].as_str().unwrap()
            )
        };
        key(&pair[0]) <= key(&pair[1])
    }));
    assert!(
        records
            .iter()
            .all(|record| record["effectiveStart"].is_string()),
        "every model interval must reject observations before the model existed"
    );
    let sonnet = records
        .iter()
        .filter(|record| record["canonicalModel"] == "claude-sonnet-5")
        .collect::<Vec<_>>();
    assert_eq!(sonnet.len(), 2);
    assert_eq!(sonnet[0]["provider"], "anthropic-api");
    assert_eq!(sonnet[0]["aliases"], serde_json::json!(["claude-sonnet-5"]));
    assert_eq!(sonnet[0]["effectiveStart"], "2026-06-30");
    assert_eq!(sonnet[0]["effectiveEnd"], "2026-08-31");
    assert_eq!(sonnet[0]["modifier"], "standard");
    assert_eq!(sonnet[0]["inputPicoUsdPerToken"], 2_000_000);
    assert_eq!(sonnet[0]["outputPicoUsdPerToken"], 10_000_000);
    assert_eq!(sonnet[0]["cacheReadPicoUsdPerToken"], 200_000);
    assert_eq!(sonnet[0]["cacheWrite5mPicoUsdPerToken"], 2_500_000);
    assert_eq!(sonnet[0]["cacheWrite1hPicoUsdPerToken"], 4_000_000);
    assert_eq!(sonnet[1]["effectiveStart"], "2026-09-01");
    assert_eq!(sonnet[1]["effectiveEnd"], Value::Null);
    assert_eq!(sonnet[1]["inputPicoUsdPerToken"], 3_000_000);
    assert_eq!(sonnet[1]["outputPicoUsdPerToken"], 15_000_000);
    assert_eq!(sonnet[1]["cacheReadPicoUsdPerToken"], 300_000);
    assert_eq!(sonnet[1]["cacheWrite5mPicoUsdPerToken"], 3_750_000);
    assert_eq!(sonnet[1]["cacheWrite1hPicoUsdPerToken"], 6_000_000);
    let retired_haiku = records
        .iter()
        .find(|record| record["canonicalModel"] == "claude-haiku-3-5")
        .expect("retired Haiku interval is embedded");
    assert_eq!(retired_haiku["effectiveStart"], "2024-11-04");
    assert_eq!(retired_haiku["effectiveEnd"], "2026-02-18");
    for record in records {
        assert_eq!(
            record["citation"],
            "https://platform.claude.com/docs/en/about-claude/pricing"
        );
        assert_eq!(record["accessDate"], "2026-07-19");
    }
    assert_eq!(cost["sourceRecorded"]["source"], "claude-transcript/v1");
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_PRICING_UNPRICED_USAGE"));
    assert_public_token_dimensions_reconcile(&json);
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f026_fast_pricing_modifier_stays_unpriced_without_an_exact_registry_record() {
    let home = SyntheticHome::new("f026-fast-modifier");
    let otel = home.write_otel(
        "fast.jsonl",
        &[otel_api_request_with_pricing_modifier(
            "session-fast",
            "request-fast",
            "2026-06-01T12:00:00Z",
            1_780_315_200_000_000_000,
            "claude-opus-4-8",
            "fast",
        )],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--otel-file",
        otel.to_str().unwrap(),
        "--no-store",
        "2026",
    ]));
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], Value::Null);
    assert_eq!(cost["localApiEquivalent"]["availability"], "unavailable");
    assert_eq!(cost["pricedTokens"], 0);
    assert_eq!(cost["unpricedTokens"], 2);
    assert_eq!(cost["pricedRequests"], 0);
    assert_eq!(cost["unpricedRequests"], 1);
    assert_eq!(cost["models"][0]["canonicalModel"], "claude-opus-4-8");
    assert!(cost["models"][0]["pricingKey"].is_null());
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_PRICING_UNPRICED_USAGE"));
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f026_opus_45_pinned_api_id_uses_its_exact_registry_record() {
    let home = SyntheticHome::new("f026-opus-45-pinned");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant!(
            "session-a",
            "message-pinned",
            "2026-01-01T12:00:00Z",
            "claude-opus-4-5-20251101",
            Some(1_000_000),
            Some(1_000_000),
            Some(0),
            Some(0),
            None,
        )],
    );

    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], 30.0);
    assert_eq!(cost["pricedTokens"], 2_000_000);
    assert_eq!(cost["unpricedTokens"], 0);
    assert_eq!(cost["models"][0]["canonicalModel"], "claude-opus-4-5");
    assert!(json["methodology"]["pricingRegistry"]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["canonicalModel"] == "claude-opus-4-5")
        .unwrap()["aliases"]
        .as_array()
        .unwrap()
        .iter()
        .any(|alias| alias == "claude-opus-4-5-20251101"));
}

#[test]
fn f026_current_fable_and_mythos_ids_survive_classification_and_price_exactly() {
    let home = SyntheticHome::new("f026-current-fable-mythos");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-fable",
                "2026-07-01T12:00:00Z",
                "claude-fable-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-mythos",
                "2026-07-01T12:01:00Z",
                "claude-mythos-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );

    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], 120.0);
    assert_eq!(cost["pricedTokens"], 4_000_000);
    assert_eq!(cost["unpricedTokens"], 0);
    assert_eq!(cost["pricedRequests"], 2);
    assert_eq!(cost["unpricedRequests"], 0);
    assert_eq!(
        cost["models"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|model| model["canonicalModel"].as_str())
            .collect::<Vec<_>>(),
        ["claude-fable-5", "claude-mythos-5"]
    );
}

#[test]
fn f026_model_availability_boundaries_and_suspensions_remain_unpriced() {
    let home = SyntheticHome::new("f026-availability-boundaries");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "haiku-prelaunch",
                "2024-11-03T12:00:00Z",
                "claude-3-5-haiku-20241022",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "haiku-launch",
                "2024-11-04T12:00:00Z",
                "claude-3-5-haiku-20241022",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "opus4-prelaunch",
                "2025-05-21T12:00:00Z",
                "claude-opus-4-20250514",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "opus4-launch",
                "2025-05-22T12:00:00Z",
                "claude-opus-4-20250514",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "sonnet4-prelaunch",
                "2025-05-21T12:01:00Z",
                "claude-sonnet-4-20250514",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "sonnet4-launch",
                "2025-05-22T12:01:00Z",
                "claude-sonnet-4-20250514",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "fable-before-suspension",
                "2026-06-11T12:00:00Z",
                "claude-fable-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "mythos-before-suspension",
                "2026-06-11T12:01:00Z",
                "claude-mythos-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "fable-suspended",
                "2026-06-12T12:00:00Z",
                "claude-fable-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "mythos-suspended",
                "2026-06-12T12:01:00Z",
                "claude-mythos-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "fable-before-restoration",
                "2026-06-30T12:00:00Z",
                "claude-fable-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "mythos-before-restoration",
                "2026-06-30T12:01:00Z",
                "claude-mythos-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "fable-restored",
                "2026-07-01T12:00:00Z",
                "claude-fable-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "mythos-restored",
                "2026-07-01T12:01:00Z",
                "claude-mythos-5",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );

    let report_2024 = home.json(&root, "UTC", 2024);
    let cost_2024 = &report_2024["canonicalMetrics"]["cost"];
    assert_eq!(cost_2024["localApiEquivalent"]["amountUsd"], 4.8);
    assert_eq!(cost_2024["pricedTokens"], 2_000_000);
    assert_eq!(cost_2024["unpricedTokens"], 2_000_000);
    assert_eq!(cost_2024["pricedRequests"], 1);
    assert_eq!(cost_2024["unpricedRequests"], 1);

    let report_2025 = home.json(&root, "UTC", 2025);
    let cost_2025 = &report_2025["canonicalMetrics"]["cost"];
    assert_eq!(cost_2025["localApiEquivalent"]["amountUsd"], 108.0);
    assert_eq!(cost_2025["pricedTokens"], 4_000_000);
    assert_eq!(cost_2025["unpricedTokens"], 4_000_000);
    assert_eq!(cost_2025["pricedRequests"], 2);
    assert_eq!(cost_2025["unpricedRequests"], 2);

    let report_2026 = home.json(&root, "UTC", 2026);
    let cost_2026 = &report_2026["canonicalMetrics"]["cost"];
    assert_eq!(cost_2026["localApiEquivalent"]["amountUsd"], 240.0);
    assert_eq!(cost_2026["pricedTokens"], 8_000_000);
    assert_eq!(cost_2026["unpricedTokens"], 8_000_000);
    assert_eq!(cost_2026["pricedRequests"], 4);
    assert_eq!(cost_2026["unpricedRequests"], 4);
}

#[test]
fn f028_legacy_cost_and_routing_use_only_the_local_api_equivalent_domain() {
    let home = SyntheticHome::new("f028-legacy-local-domain");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-opus",
                "2026-04-05T09:00:00Z",
                "claude-opus-4-6",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                Some(0.01),
            ),
            assistant!(
                "session-a",
                "message-sonnet",
                "2026-04-05T09:01:00Z",
                "claude-sonnet-4-6",
                Some(1_000_000),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );

    let json = home.json(&root, "UTC", 2026);
    assert_eq!(
        json["canonicalMetrics"]["cost"]["sourceRecorded"]["amountUsd"],
        0.01
    );
    assert_eq!(
        json["canonicalMetrics"]["cost"]["localApiEquivalent"]["amountUsd"],
        48.0
    );
    assert_eq!(json["costAnalysis"]["totalCost"], 48.0);
    assert_eq!(json["modelRouting"]["totalCost"], 48.0);
    assert_eq!(
        json["modelRouting"]["methodId"],
        "routing/model-tier-request-share/v1"
    );
    assert_eq!(json["modelRouting"]["unit"], "request-share");
    assert_eq!(json["modelRouting"]["observations"], 2);
    assert_eq!(json["modelRouting"]["opusPct"], 50);
    assert_eq!(json["modelRouting"]["sonnetPct"], 50);
    assert_public_token_dimensions_reconcile(&json);
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f027_cache_ttl_prices_once_and_partner_provider_stays_unpriced() {
    let home = SyntheticHome::new("f027-ttl-provider");
    let root = home.transcript_root();
    let mut direct = assistant!(
        "session-a",
        "message-direct",
        "2026-04-05T09:00:00Z",
        "claude-sonnet-4-6",
        Some(0),
        Some(0),
        Some(3_000_000),
        Some(0),
        None,
    );
    direct["message"]["usage"]["cache_creation"] = serde_json::json!({
        "ephemeral_5m_input_tokens": 1_000_000,
        "ephemeral_1h_input_tokens": 2_000_000
    });
    let partner = assistant!(
        "session-a",
        "message-partner",
        "2026-04-05T09:01:00Z",
        "us.anthropic.claude-sonnet-4-6-v1:0",
        Some(1_000_000),
        Some(1_000_000),
        Some(0),
        Some(0),
        None,
    );
    home.write_session(&root, "project-alpha", "session-a", &[direct, partner]);
    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], 15.75);
    assert_eq!(cost["localApiEquivalent"]["unit"], "USD");
    assert_eq!(cost["pricedTokens"], 3_000_000);
    assert_eq!(cost["unpricedTokens"], 2_000_000);
    let partner = cost["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["provider"] == "aws-bedrock")
        .expect("partner model evidence");
    assert!(partner["canonicalModel"].is_null());
    assert!(partner["pricingKey"].is_null());
    assert_eq!(partner["localApiEquivalentUsd"], Value::Null);
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f027_cache_ttl_components_exceeding_generic_total_stay_unpriced() {
    let home = SyntheticHome::new("f027-ttl-over-composition");
    let root = home.transcript_root();
    let mut direct = assistant!(
        "session-a",
        "message-direct",
        "2026-04-05T09:00:00Z",
        "claude-sonnet-4-6",
        Some(0),
        Some(0),
        Some(1_000_000),
        Some(0),
        None,
    );
    direct["message"]["usage"]["cache_creation"] = serde_json::json!({
        "ephemeral_5m_input_tokens": 2_000_000,
        "ephemeral_1h_input_tokens": 3_000_000
    });
    home.write_session(&root, "project-alpha", "session-a", &[direct]);

    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], Value::Null);
    assert_eq!(cost["localApiEquivalent"]["availability"], "unavailable");
    assert_eq!(cost["pricedTokens"], 0);
    assert_eq!(cost["unpricedTokens"], 1_000_000);
    assert_eq!(cost["coverage"], "unavailable");
    assert_eq!(cost["pricedRequests"], 0);
    assert_eq!(cost["unpricedRequests"], 1);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_PRICING_UNPRICED_USAGE"));
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_PRICING_CACHE_TTL_COMPOSITION"));
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f027_cache_ttl_component_sum_overflow_stays_unpriced() {
    let home = SyntheticHome::new("f027-ttl-component-overflow");
    let root = home.transcript_root();
    let mut direct = assistant!(
        "session-a",
        "message-direct",
        "2026-04-05T09:00:00Z",
        "claude-sonnet-4-6",
        Some(0),
        Some(0),
        Some(u64::MAX),
        Some(0),
        None,
    );
    direct["message"]["usage"]["cache_creation"] = serde_json::json!({
        "ephemeral_5m_input_tokens": u64::MAX,
        "ephemeral_1h_input_tokens": 1
    });
    home.write_session(&root, "project-alpha", "session-a", &[direct]);

    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], Value::Null);
    assert_eq!(cost["pricedTokens"], 0);
    assert_eq!(cost["unpricedTokens"], u64::MAX);
    assert_eq!(cost["unpricedTokensOverflowed"], false);
    assert_eq!(cost["pricedRequests"], 0);
    assert_eq!(cost["unpricedRequests"], 1);
    assert!(json["dataCoverage"]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .any(|warning| warning["code"] == "W_PRICING_CACHE_TTL_COMPOSITION"));
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f027_aggregate_metric_without_session_reconciles_as_unattributed_cost() {
    let home = SyntheticHome::new("f027-unattributed-metric");
    let otel = home.write_otel(
        "unattributed-metric.jsonl",
        &[otel_token_metric(
            None,
            "input",
            1_775_383_200_000_000_000,
            1_775_383_260_000_000_000,
            1_000_000,
        )],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]));

    assert_eq!(
        json["canonicalMetrics"]["tokens"]["unattributed"]["input"]["observed"],
        1_000_000
    );
    assert_eq!(
        json["canonicalMetrics"]["cost"]["localApiEquivalent"]["amountUsd"],
        3.0
    );
    assert_public_token_dimensions_reconcile(&json);
    assert_cost_domains_reconcile(&json);
}

#[test]
fn otel_without_project_identity_stays_explicitly_unattributed() {
    let home = SyntheticHome::new("otel-project-unattributed");
    let otel = home.write_otel(
        "unattributed.jsonl",
        &[otel_api_request(
            "session-a",
            "request-a",
            "2026-04-05T09:00:00Z",
            1_775_376_000_000_000_000,
            10,
            false,
        )],
    );
    let json = successful_json(home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--otel-file",
        otel.to_str().unwrap(),
        "--no-store",
        "2026",
    ]));

    let tokens = &json["canonicalMetrics"]["tokens"];
    assert!(tokens["projects"].as_array().unwrap().is_empty());
    assert_eq!(tokens["projectUnattributed"]["input"]["observed"], 1);
    assert_eq!(tokens["projectUnattributed"]["output"]["observed"], 1);
    assert!(json["projectBreakdown"].as_array().unwrap().is_empty());
    assert_eq!(
        json["canonicalMetrics"]["activeTime"]["projectUnattributedActiveSeconds"],
        0
    );
    assert_public_token_dimensions_reconcile(&json);
    assert_public_active_dimensions_reconcile(&json);
    assert_cost_domains_reconcile(&json);
}

#[test]
fn f026_exact_first_party_provider_prefixes_map_without_tier_guessing() {
    let home = SyntheticHome::new("f026-prefixes");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-anthropic-prefix",
                "2026-04-05T09:00:00Z",
                "anthropic/claude-sonnet-4-6",
                Some(1_000_000),
                Some(0),
                Some(0),
                Some(0),
                None,
            ),
            assistant!(
                "session-a",
                "message-claude-prefix",
                "2026-04-05T09:01:00Z",
                "claude/claude-sonnet-4-6",
                Some(0),
                Some(1_000_000),
                Some(0),
                Some(0),
                None,
            ),
        ],
    );
    let json = home.json(&root, "UTC", 2026);
    let cost = &json["canonicalMetrics"]["cost"];
    assert_eq!(cost["localApiEquivalent"]["amountUsd"], 18.0);
    assert_eq!(cost["pricedTokens"], 2_000_000);
    assert_eq!(cost["unpricedTokens"], 0);
    for model in cost["models"].as_array().unwrap() {
        assert_eq!(model["provider"], "anthropic-api");
        assert_eq!(model["canonicalModel"], "claude-sonnet-4-6");
        assert!(model["pricingKey"].as_str().unwrap().contains("standard"));
    }
}

#[test]
fn f029_explicit_zone_json_is_byte_deterministic_across_runs_and_ambient_tz() {
    let home = SyntheticHome::new("f029-determinism");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-a",
                "2026-04-05T09:00:00Z",
                "claude-sonnet-4-6",
                Some(100),
                Some(20),
                Some(10),
                Some(90),
                None,
            ),
            assistant!(
                "session-a",
                "message-b",
                "2026-04-05T09:03:00Z",
                "claude-sonnet-4-6",
                Some(100),
                Some(20),
                Some(10),
                Some(90),
                None,
            ),
        ],
    );
    let args = [
        "--json",
        "--timezone",
        "UTC",
        "--active-threshold-minutes",
        "5",
        "--data-dir",
        root.to_str().unwrap(),
        "2026",
    ];
    let first = home.run_with_tz(&args, "Pacific/Honolulu");
    let second = home.run_with_tz(&args, "Pacific/Honolulu");
    let different_ambient = home.run_with_tz(&args, "Europe/Berlin");
    assert!(first.status.success());
    assert_eq!(first.stdout, second.stdout);
    assert_eq!(first.stdout, different_ambient.stdout);
}

#[test]
fn f030_every_renderer_exposes_the_same_canonical_fact_lines_without_causal_cache_text() {
    let home = SyntheticHome::new("f030-renderers");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[
            assistant!(
                "session-a",
                "message-a",
                "2026-04-05T09:00:00Z",
                "claude-sonnet-4-6",
                Some(100),
                Some(10),
                Some(20),
                Some(80),
                Some(0.01),
            ),
            assistant!(
                "session-a",
                "message-b",
                "2026-04-05T09:02:00Z",
                "claude-sonnet-4-6",
                Some(100),
                Some(10),
                Some(20),
                Some(80),
                None,
            ),
        ],
    );
    let otel = home.write_otel(
        "compaction.jsonl",
        &[otel_compaction(
            "session-a",
            "2026-04-05T09:03:00Z",
            1_775_379_780_000_000_000,
        )],
    );
    let json_output = home.run(&[
        "--json",
        "--timezone",
        "UTC",
        "--data-dir",
        root.to_str().unwrap(),
        "--otel-file",
        otel.to_str().unwrap(),
        "2026",
    ]);
    let json = successful_json(json_output);
    let outputs = renderers_containing_facts(
        &home,
        &[
            "--timezone",
            "UTC",
            "--data-dir",
            root.to_str().unwrap(),
            "--otel-file",
            otel.to_str().unwrap(),
            "2026",
        ],
        &json,
    );
    assert_eq!(json["canonicalMetrics"]["activeTime"]["unit"], "seconds");
    assert_eq!(
        json["canonicalMetrics"]["tokens"]["global"]["total"]["unit"],
        "tokens"
    );
    assert_eq!(
        json["canonicalMetrics"]["cost"]["localApiEquivalent"]["unit"],
        "USD"
    );
    assert_eq!(
        json["canonicalMetrics"]["cache"]["readShare"]["unit"],
        "percent"
    );
    assert_eq!(json["canonicalMetrics"]["cache"]["directCompactions"], 1);

    let forbidden = [
        "season spend",
        "cache grade",
        "cache health",
        "saved from caching",
        "overhead from breaks",
        "cache chaos",
        "reset stale",
        "wasted spend",
        "actual cost",
        "total spend",
    ];
    for (label, output) in outputs {
        let output = output.to_lowercase();
        for phrase in forbidden {
            assert!(!output.contains(phrase), "{label} emitted {phrase}");
        }
    }
    let mut json_strings = Vec::new();
    collect_json_strings(&json, &mut json_strings);
    let json_values = json_strings.join("\n").to_lowercase();
    for phrase in forbidden {
        assert!(!json_values.contains(phrase), "JSON value emitted {phrase}");
    }
}

#[test]
fn invalid_timezone_is_actionable_and_json_safe() {
    let home = SyntheticHome::new("invalid-timezone");
    let root = home.transcript_root();
    home.write_session(
        &root,
        "project-alpha",
        "session-a",
        &[assistant!(
            "session-a",
            "message-a",
            "2026-04-05T09:00:00Z",
            "claude-sonnet-4-6",
            Some(1),
            Some(1),
            Some(0),
            Some(0),
            None,
        )],
    );
    let output = home.run(&[
        "--json",
        "--timezone",
        "Not/A_Real_Zone",
        "--data-dir",
        root.to_str().unwrap(),
        "2026",
    ]);
    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON error payload");
    assert_eq!(json["code"], "E_TIMEZONE_INVALID");
    assert!(!String::from_utf8_lossy(&output.stderr).contains(root.to_str().unwrap()));
}

#[test]
fn nonnumeric_active_threshold_is_actionable_and_json_safe() {
    let home = SyntheticHome::new("invalid-threshold-text");
    let output = home.run(&[
        "--json",
        "--active-threshold-minutes",
        "private-invalid-threshold",
        "2026",
    ]);
    assert!(!output.status.success());
    let json: Value = serde_json::from_slice(&output.stdout).expect("JSON error payload");
    assert_eq!(json["code"], "E_CLI_ARGUMENT_INVALID");
    assert_eq!(json["error"], "invalid configuration");
    assert!(output.stderr.is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("private-invalid-threshold"));
}
