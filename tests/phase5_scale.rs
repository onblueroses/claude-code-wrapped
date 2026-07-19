#[path = "support/phase5-bench/src/generator.rs"]
mod generator;

use generator::{
    append_incremental_tail, byte_identity, generate, relative_source_files, CorpusClass,
};
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

struct Scratch {
    path: PathBuf,
}

impl Scratch {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock follows epoch")
            .as_nanos();
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        #[cfg(windows)]
        let base = std::env::var_os("CCWRAPPED_WINDOWS_TEST_ROOT")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .expect("Windows store tests require a test root or USERPROFILE");
        #[cfg(not(windows))]
        let base = std::env::temp_dir();
        let path = base.join(format!(
            "ccwrapped-phase5-{label}-{}-{nonce}-{id}",
            std::process::id()
        ));
        fs::create_dir(&path).expect("create Phase 5 scratch");
        Self { path }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        if std::env::var_os("CCWRAPPED_KEEP_PHASE5_SCRATCH").is_some() {
            eprintln!("preserved Phase 5 scratch: {}", self.path.display());
        } else {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn otel_files(root: &Path) -> Vec<PathBuf> {
    let mut paths = fs::read_dir(root.join("otel"))
        .expect("read generated OTel directory")
        .map(|entry| entry.expect("read OTel entry").path())
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn run_generated(root: &Path) -> std::process::Output {
    run_generated_with(root, None, None, None)
}

fn run_generated_with(
    root: &Path,
    workers: Option<usize>,
    delay_seed: Option<u64>,
    panic_file: Option<usize>,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ccwrapped"));
    command
        .args(["--timezone", "UTC", "--data-dir"])
        .arg(root.join("projects"))
        .arg("--no-store");
    if let Some(workers) = workers {
        command.arg("--ingestion-workers").arg(workers.to_string());
    }
    if let Some(delay_seed) = delay_seed {
        command
            .arg("--ingestion-delay-seed")
            .arg(delay_seed.to_string());
    }
    if let Some(panic_file) = panic_file {
        command
            .arg("--ingestion-panic-file")
            .arg(panic_file.to_string());
    }
    for path in otel_files(root) {
        command.arg("--otel-file").arg(path);
    }
    command
        .args(["--json", "2026"])
        .env("HOME", root.join("isolated-home"))
        .env("XDG_CACHE_HOME", root.join("isolated-cache"))
        .env("CLAUDE_CONFIG_DIR", root.join("isolated-config"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run ccwrapped against generated corpus")
}

fn run_generated_store(root: &Path, store: &Path, rebuild: bool) -> std::process::Output {
    run_generated_store_with_extra(root, store, rebuild, &[])
}

fn run_generated_store_with_extra(
    root: &Path,
    store: &Path,
    rebuild: bool,
    extra_args: &[&str],
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ccwrapped"));
    command
        .args(["--timezone", "UTC", "--data-dir"])
        .arg(root.join("projects"))
        .arg("--store-path")
        .arg(store);
    if rebuild {
        command.arg("--rebuild-store");
    }
    for path in otel_files(root) {
        command.arg("--otel-file").arg(path);
    }
    command
        .args(extra_args)
        .args(["--json", "2026"])
        .env("HOME", root.join("isolated-home"))
        .env("XDG_CACHE_HOME", root.join("isolated-cache"))
        .env("CLAUDE_CONFIG_DIR", root.join("isolated-config"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run ccwrapped with an explicit store")
}

fn run_generated_store_with_counters(
    root: &Path,
    store: &Path,
    counters: &Path,
) -> std::process::Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ccwrapped"));
    command
        .args(["--timezone", "UTC", "--data-dir"])
        .arg(root.join("projects"))
        .arg("--store-path")
        .arg(store)
        .arg("--benchmark-counters")
        .arg(counters);
    for path in otel_files(root) {
        command.arg("--otel-file").arg(path);
    }
    command
        .args(["--json", "2026"])
        .env("HOME", root.join("isolated-home"))
        .env("XDG_CACHE_HOME", root.join("isolated-cache"))
        .env("CLAUDE_CONFIG_DIR", root.join("isolated-config"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run incremental import with counters")
}

fn run_generated_serial_private_archive(
    root: &Path,
    output: &Path,
    standard_worker_override: usize,
) -> std::process::Output {
    fs::create_dir(output).expect("create archive output directory");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ccwrapped"));
    command
        .args(["--timezone", "UTC", "--data-dir"])
        .arg(root.join("projects"))
        .arg("--ingestion-workers")
        .arg(standard_worker_override.to_string());
    for path in otel_files(root) {
        command.arg("--otel-file").arg(path);
    }
    command
        .args(["--archive", "--plain", "2026"])
        .current_dir(output)
        .env("HOME", output.join("home"))
        .env("XDG_CACHE_HOME", output.join("home").join("cache"))
        .env("CLAUDE_CONFIG_DIR", output.join("config"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run private archive against generated corpus")
}

fn run_generated_archive_with_store(
    root: &Path,
    output: &Path,
    store: &Path,
) -> std::process::Output {
    fs::create_dir(output).expect("create archive/store output directory");
    let mut command = Command::new(env!("CARGO_BIN_EXE_ccwrapped"));
    command
        .args(["--timezone", "UTC", "--data-dir"])
        .arg(root.join("projects"))
        .arg("--store-path")
        .arg(store);
    for path in otel_files(root) {
        command.arg("--otel-file").arg(path);
    }
    command
        .args(["--archive", "--plain", "2026"])
        .current_dir(output)
        .env("HOME", output.join("home"))
        .env("XDG_CACHE_HOME", output.join("home").join("cache"))
        .env("CLAUDE_CONFIG_DIR", output.join("config"))
        .env("NO_COLOR", "1")
        .output()
        .expect("run private archive with an explicit standard store")
}

fn json_u64(value: &Value, path: &[&str]) -> u64 {
    let mut cursor = value;
    for segment in path {
        cursor = &cursor[*segment];
    }
    cursor
        .as_u64()
        .unwrap_or_else(|| panic!("{} is not a u64", path.join(".")))
}

fn replace_store_with_legacy_schema(path: &Path, version: i64) {
    assert!((1..=8).contains(&version));
    let mut connection =
        rusqlite::Connection::open(path).expect("open protected store for legacy fixture");
    let transaction = connection
        .transaction()
        .expect("begin legacy schema replacement");
    transaction
        .execute_batch(
            "
            DROP TABLE analysis_state;
            DROP TABLE cached_report;
            DROP TABLE source_file;
            DROP TABLE meta;
            CREATE TABLE meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            ) WITHOUT ROWID;
            CREATE TABLE source_file (
                legacy_key BLOB PRIMARY KEY,
                legacy_payload BLOB NOT NULL
            ) WITHOUT ROWID;
            CREATE TABLE cached_report (
                singleton INTEGER PRIMARY KEY,
                report_json BLOB NOT NULL,
                report_digest BLOB NOT NULL
            );
            CREATE TABLE packed_file_cache (
                legacy_key BLOB PRIMARY KEY,
                legacy_payload BLOB NOT NULL
            ) WITHOUT ROWID;
            ",
        )
        .expect("create genuine legacy table shapes");
    transaction
        .execute_batch(
            "
            CREATE TABLE analysis_state (
                singleton INTEGER PRIMARY KEY,
                legacy_payload BLOB NOT NULL
            );
            INSERT INTO analysis_state (singleton, legacy_payload) VALUES (1, X'01');
            ",
        )
        .expect("create legacy analysis state");
    let format = format!("ccwrapped.incremental-store/v{version}");
    let salt = [version as u8; 32];
    transaction
        .execute(
            "INSERT INTO meta (key, value) VALUES ('format', ?1), ('salt', ?2)",
            rusqlite::params![format.as_bytes(), salt.as_slice()],
        )
        .expect("write legacy metadata");
    transaction
        .execute(
            "INSERT INTO source_file (legacy_key, legacy_payload) VALUES (X'01', X'02')",
            [],
        )
        .expect("write legacy source row");
    transaction
        .execute(
            "INSERT INTO cached_report (singleton, report_json, report_digest)
             VALUES (1, X'01', zeroblob(32))",
            [],
        )
        .expect("write legacy report row");
    transaction
        .execute(
            "INSERT INTO packed_file_cache (legacy_key, legacy_payload) VALUES (X'01', X'02')",
            [],
        )
        .expect("write legacy packed row");
    transaction
        .pragma_update(None, "user_version", version)
        .expect("set legacy schema version");
    transaction.commit().expect("commit legacy schema fixture");
}

#[test]
fn f051_generator_is_byte_deterministic_and_manifest_bounded() {
    let _incremental_tail_contract: fn(&Path, &Path) -> Result<String, String> =
        append_incremental_tail;
    assert_eq!(
        CorpusClass::parse("decision").expect("decision class"),
        CorpusClass::Decision
    );
    assert_eq!(
        CorpusClass::parse("saturation-large").expect("large class"),
        CorpusClass::SaturationLarge
    );
    let scratch = Scratch::new("generator-determinism");
    let first = scratch.path.join("first");
    let second = scratch.path.join("second");
    let first_summary =
        generate(CorpusClass::OracleSmall, &first, None).expect("generate first corpus");
    let second_summary =
        generate(CorpusClass::OracleSmall, &second, None).expect("generate second corpus");

    assert_eq!(first_summary, second_summary);
    assert!(byte_identity(&first, &second).expect("compare generated corpora"));
    assert!((10_000..=20_000).contains(&first_summary.physical_records));
    assert!(first_summary.source_bytes <= 16 * 1024 * 1024);
    assert_eq!(
        first_summary.normalized_candidates,
        first_summary
            .accepted_records
            .saturating_add(first_summary.duplicate_records)
    );
    assert_eq!(first_summary.transcript_files, 32);
    assert_eq!(first_summary.otel_files, 4);
    assert_eq!(first_summary.metric_points, 12);
    assert_eq!(first_summary.metric_accepted_points, 10);
    assert_eq!(first_summary.metric_filtered_points, 2);
    assert_eq!(first_summary.metric_delta_points, 6);
    assert_eq!(first_summary.metric_cumulative_points, 6);
    assert_eq!(first_summary.metric_reset_points, 2);
    assert_eq!(first_summary.metric_gap_points, 4);
    assert_eq!(first_summary.metric_overlap_points, 2);
    assert!(first_summary.active_time_oracle.is_some());
    assert_eq!(first_summary.insight_eligibility.len(), 10);
    let manifest: Value =
        serde_json::from_str(&first_summary.manifest_json()).expect("parse corpus manifest");
    assert_eq!(manifest["schema"], "ccwrapped.phase5-corpus/v2");
    assert_eq!(manifest["metricOracle"]["points"], 12);
    assert_eq!(manifest["activeTimeOracle"]["intervalCount"], 4_196);
    assert_eq!(
        manifest["insightEligibility"].as_array().map(Vec::len),
        Some(10)
    );
    assert_eq!(
        relative_source_files(&first)
            .expect("enumerate generated corpus")
            .len(),
        32 + 4 + 1
    );
}

#[test]
fn f051_oracle_matches_real_ingestion_and_excludes_canaries() {
    let scratch = Scratch::new("oracle");
    let corpus = scratch.path.join("corpus");
    let expected =
        generate(CorpusClass::OracleSmall, &corpus, None).expect("generate oracle corpus");
    let output = run_generated(&corpus);
    assert!(
        output.status.success(),
        "generated corpus failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("JSON output is UTF-8");
    for canary in [
        "SYNTHETIC_PHASE5_PROMPT_CANARY",
        "SYNTHETIC_PHASE5_PATH_CANARY",
        "SYNTHETIC_PHASE5_EMAIL_CANARY",
    ] {
        assert!(!stdout.contains(canary), "{canary} leaked to standard JSON");
    }
    let report: Value = serde_json::from_str(&stdout).expect("parse generated report");
    let coverage = &report["dataCoverage"];
    let expected_classified = expected
        .accepted_records
        .saturating_add(expected.malformed_records)
        .saturating_add(expected.unsupported_records)
        .saturating_add(expected.filtered_records)
        .saturating_add(expected.duplicate_records);
    assert_eq!(
        coverage["acceptedRecords"].as_u64(),
        Some(expected.accepted_records)
    );
    assert_eq!(
        coverage["classifiedRecords"].as_u64(),
        Some(expected_classified)
    );
    assert_eq!(
        coverage["canonicalRecords"].as_u64(),
        Some(expected.canonical_records)
    );
    assert_eq!(
        coverage["malformedRecords"].as_u64(),
        Some(expected.malformed_records)
    );
    assert_eq!(
        coverage["unsupportedRecords"].as_u64(),
        Some(expected.unsupported_records)
    );
    assert_eq!(
        coverage["unknownRecords"].as_u64(),
        Some(expected.unknown_records)
    );
    assert_eq!(
        coverage["filteredRecords"].as_u64(),
        Some(expected.filtered_records)
    );
    assert_eq!(
        coverage["duplicateRecords"].as_u64(),
        Some(expected.duplicate_records)
    );
    assert_eq!(
        coverage["resolvedOverlapRecords"].as_u64(),
        Some(expected.resolved_overlap_records)
    );
    assert_eq!(
        coverage["unresolvedOverlapRecords"].as_u64(),
        Some(expected.unresolved_overlap_records)
    );

    assert_eq!(
        json_u64(
            &report,
            &["canonicalMetrics", "tokens", "global", "input", "observed"]
        ),
        expected.input_tokens
    );
    assert_eq!(
        json_u64(
            &report,
            &["canonicalMetrics", "tokens", "global", "output", "observed",]
        ),
        expected.output_tokens
    );
    assert_eq!(
        json_u64(
            &report,
            &[
                "canonicalMetrics",
                "tokens",
                "global",
                "cacheCreation",
                "observed",
            ]
        ),
        expected.cache_creation_tokens
    );
    assert_eq!(
        json_u64(
            &report,
            &[
                "canonicalMetrics",
                "tokens",
                "global",
                "cacheRead",
                "observed",
            ]
        ),
        expected.cache_read_tokens
    );

    let expected_active = expected
        .active_time_oracle
        .as_ref()
        .expect("oracle-small active-time oracle");
    let active = &report["canonicalMetrics"]["activeTime"];
    assert_eq!(
        active["intervalCount"].as_u64(),
        Some(expected_active.interval_count)
    );
    assert_eq!(
        active["totalElapsedSeconds"].as_u64(),
        Some(expected_active.total_elapsed_seconds)
    );
    assert_eq!(
        active["totalActiveSeconds"].as_u64(),
        Some(expected_active.total_active_seconds)
    );
    assert_eq!(
        active["mainExclusiveSeconds"].as_u64(),
        Some(expected_active.main_exclusive_seconds)
    );
    assert_eq!(
        active["subagentExclusiveSeconds"].as_u64(),
        Some(expected_active.subagent_exclusive_seconds)
    );

    let families = report["insights"]["families"]
        .as_array()
        .expect("insight family array");
    for expected_family in &expected.insight_eligibility {
        let actual = families
            .iter()
            .find(|family| family["family"] == expected_family.family)
            .unwrap_or_else(|| panic!("missing {} insight family", expected_family.family));
        assert_eq!(
            actual["availability"].as_str(),
            Some(expected_family.availability),
            "{} availability",
            expected_family.family
        );
        assert_eq!(
            actual["sampleCount"].as_u64(),
            Some(expected_family.sample_count),
            "{} sample count",
            expected_family.family
        );
        assert_eq!(
            actual["minimumSampleCount"].as_u64(),
            Some(expected_family.minimum_sample_count),
            "{} minimum sample count",
            expected_family.family
        );
    }
}

#[test]
fn f052_generated_otel_metrics_cover_temporality_reset_gap_and_overlap() {
    let scratch = Scratch::new("otel-metric-oracle");
    let corpus = scratch.path.join("corpus");
    let expected =
        generate(CorpusClass::OracleSmall, &corpus, None).expect("generate OTel metric corpus");
    assert_eq!(
        (
            expected.metric_points,
            expected.metric_accepted_points,
            expected.metric_filtered_points,
            expected.metric_delta_points,
            expected.metric_cumulative_points,
            expected.metric_reset_points,
            expected.metric_gap_points,
            expected.metric_overlap_points,
        ),
        (12, 10, 2, 6, 6, 2, 4, 2)
    );

    let output = run_generated(&corpus);
    assert!(
        output.status.success(),
        "generated metric corpus failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty());
    let report: Value =
        serde_json::from_slice(&output.stdout).expect("parse generated metric report");
    assert_eq!(
        report["dataCoverage"]["capabilities"]["metric_token_usage"].as_str(),
        Some("available")
    );
    assert_eq!(
        json_u64(
            &report,
            &["canonicalMetrics", "tokens", "global", "output", "observed"]
        ),
        expected.output_tokens
    );
    let warnings = report["dataCoverage"]["warnings"]
        .as_array()
        .expect("coverage warning array");
    for code in [
        "W_OTEL_METRIC_GAP",
        "W_OTEL_METRIC_OVERLAP",
        "W_OTEL_METRIC_RESET",
    ] {
        assert!(
            warnings.iter().any(|warning| warning["code"] == code),
            "generated metric corpus did not exercise {code}"
        );
    }
}

#[test]
fn f053_cold_first_warm_and_no_store_json_are_byte_identical() {
    let scratch = Scratch::new("store-equality");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate store corpus");
    let store = scratch.path.join("state/store.sqlite3");

    let no_store = run_generated(&corpus);
    let first = run_generated_store(&corpus, &store, false);
    let warm = run_generated_store(&corpus, &store, false);
    for (mode, output) in [("no-store", &no_store), ("first", &first), ("warm", &warm)] {
        assert!(
            output.status.success(),
            "{mode} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty(), "{mode} wrote stderr");
    }
    assert_eq!(first.stdout, no_store.stdout);
    assert_eq!(warm.stdout, no_store.stdout);

    let private_diagnostics =
        run_generated_store_with_extra(&corpus, &store, false, &["--private-diagnostics"]);
    assert!(private_diagnostics.status.success());
    assert_eq!(private_diagnostics.stdout, no_store.stdout);
    let diagnostic_stderr = String::from_utf8_lossy(&private_diagnostics.stderr);
    assert!(diagnostic_stderr.contains("[privacy-profile: private]"));
    assert!(diagnostic_stderr.contains(&corpus.join("projects").to_string_lossy().to_string()));
}

#[test]
fn f053_cached_report_rejects_recomputed_private_output_carriers() {
    let scratch = Scratch::new("store-report-trust-boundary");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate store corpus");
    let store = scratch.path.join("state/store.sqlite3");

    let first = run_generated_store(&corpus, &store, false);
    assert!(
        first.status.success(),
        "initial stored report failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let private_value = "/private/recomputed-cache-injection";
    let mut report: Value =
        serde_json::from_slice(&first.stdout).expect("parse initial cached report");
    let project = report["projectBreakdown"]
        .as_array_mut()
        .and_then(|projects| projects.first_mut())
        .expect("generated report has a project summary");
    project["path"] = Value::String(private_value.to_string());
    let encoded = serde_json::to_vec_pretty(&report).expect("encode modified cached report");
    let compressed =
        zstd::stream::encode_all(encoded.as_slice(), -5).expect("compress modified cached report");
    let digest = blake3::hash(&compressed);
    let connection = rusqlite::Connection::open(&store).expect("open stored report");
    connection
        .execute(
            "UPDATE cached_report SET report_json = ?1, report_digest = ?2 WHERE singleton = 1",
            rusqlite::params![compressed, digest.as_bytes().as_slice()],
        )
        .expect("replace report with recomputed-digest fixture");
    drop(connection);

    let rejected = run_generated_store(&corpus, &store, false);
    assert!(
        !rejected.status.success(),
        "a recomputed digest bypassed typed cached-report validation"
    );
    let error: Value =
        serde_json::from_slice(&rejected.stdout).expect("parse cached-report validation error");
    assert_eq!(error["code"], "E_INCREMENTAL_STORE");
    assert!(
        !String::from_utf8_lossy(&rejected.stdout).contains(private_value),
        "private cached value leaked through the error response"
    );
}

#[test]
fn f054_incremental_append_reads_only_changed_and_new_source_files() {
    let scratch = Scratch::new("store-incremental");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate incremental corpus");
    let store = scratch.path.join("state/store.sqlite3");
    let first = run_generated_store(&corpus, &store, false);
    assert!(first.status.success());

    let mut transcript_files = relative_source_files(&corpus)
        .expect("enumerate corpus")
        .into_iter()
        .filter(|path| {
            path.starts_with("projects")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
        })
        .collect::<Vec<_>>();
    transcript_files.sort();
    let changed = corpus.join(transcript_files.first().expect("generated transcript file"));
    let appended = concat!(
        "{\"type\":\"assistant\",\"uuid\":\"incremental-message-1\",",
        "\"timestamp\":\"2026-06-30T12:00:00Z\",",
        "\"message\":{\"model\":\"claude-sonnet-4-20250514\",",
        "\"usage\":{\"input_tokens\":7,\"output_tokens\":11,",
        "\"cache_creation_input_tokens\":13,\"cache_read_input_tokens\":17}}}\n"
    );
    let mut changed_file = fs::OpenOptions::new()
        .append(true)
        .open(&changed)
        .expect("open changed transcript");
    changed_file
        .write_all(appended.as_bytes())
        .expect("append incremental record");
    changed_file.sync_all().expect("sync incremental append");

    let new_file = corpus
        .join("projects")
        .join("project-00000")
        .join("incremental-new-session.jsonl");
    fs::write(
        &new_file,
        concat!(
            "{\"type\":\"assistant\",\"uuid\":\"incremental-message-2\",",
            "\"timestamp\":\"2026-07-01T12:00:00Z\",",
            "\"message\":{\"model\":\"claude-sonnet-4-20250514\",",
            "\"usage\":{\"input_tokens\":19,\"output_tokens\":23,",
            "\"cache_creation_input_tokens\":29,\"cache_read_input_tokens\":31}}}\n"
        ),
    )
    .expect("write new incremental transcript");

    let counters_path = scratch.path.join("incremental-counters.json");
    let incremental = run_generated_store_with_counters(&corpus, &store, &counters_path);
    assert!(
        incremental.status.success(),
        "incremental import failed: {}",
        String::from_utf8_lossy(&incremental.stderr)
    );
    let clean = run_generated(&corpus);
    assert!(clean.status.success());
    assert_eq!(incremental.stdout, clean.stdout);

    let counters: Value =
        serde_json::from_slice(&fs::read(&counters_path).expect("read incremental counters"))
            .expect("parse incremental counters");
    assert_eq!(
        counters["parsedSourceFiles"].as_u64(),
        Some(2),
        "incremental counters: {counters}"
    );
    assert!(
        counters["reusedSourceFiles"].as_u64().unwrap_or_default() > 0,
        "unchanged files were not reused"
    );
    let changed_bytes = fs::metadata(&changed).expect("changed metadata").len()
        + fs::metadata(&new_file).expect("new metadata").len();
    assert_eq!(
        counters["sourceContentBytesRead"].as_u64(),
        Some(changed_bytes)
    );
    let warm = run_generated_store(&corpus, &store, false);
    assert!(warm.status.success());
    assert_eq!(warm.stdout, clean.stdout);
}

#[test]
fn f055_store_reconciles_mutations_and_preserves_rows_on_root_failure() {
    let scratch = Scratch::new("store-mutations");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate mutation corpus");
    let store = scratch.path.join("state/store.sqlite3");
    let first = run_generated_store(&corpus, &store, false);
    assert!(first.status.success());

    let mut transcript_files = relative_source_files(&corpus)
        .expect("enumerate mutation corpus")
        .into_iter()
        .filter(|path| {
            path.starts_with("projects")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
        })
        .collect::<Vec<_>>();
    transcript_files.sort();
    assert!(transcript_files.len() >= 4);

    fs::remove_file(corpus.join(&transcript_files[0])).expect("delete transcript");
    let deleted = run_generated_store(&corpus, &store, false);
    let deleted_clean = run_generated(&corpus);
    assert!(deleted.status.success());
    assert_eq!(deleted.stdout, deleted_clean.stdout);

    fs::write(corpus.join(&transcript_files[1]), b"").expect("truncate transcript");
    let truncated = run_generated_store(&corpus, &store, false);
    let truncated_clean = run_generated(&corpus);
    assert!(truncated.status.success());
    assert_eq!(truncated.stdout, truncated_clean.stdout);

    let replacement = corpus.join("replacement.jsonl");
    fs::write(
        &replacement,
        concat!(
            "{\"type\":\"assistant\",\"uuid\":\"replacement-message\",",
            "\"timestamp\":\"2026-08-01T12:00:00Z\",",
            "\"message\":{\"model\":\"claude-sonnet-4-20250514\",",
            "\"usage\":{\"input_tokens\":2,\"output_tokens\":3}}}\n"
        ),
    )
    .expect("write replacement");
    fs::rename(&replacement, corpus.join(&transcript_files[2]))
        .expect("replace transcript identity");
    let replaced = run_generated_store(&corpus, &store, false);
    let replaced_clean = run_generated(&corpus);
    assert!(replaced.status.success());
    assert_eq!(replaced.stdout, replaced_clean.stdout);

    let renamed = corpus
        .join(&transcript_files[3])
        .with_file_name("renamed-session.jsonl");
    fs::rename(corpus.join(&transcript_files[3]), &renamed).expect("rename transcript");
    let renamed_store = run_generated_store(&corpus, &store, false);
    let renamed_clean = run_generated(&corpus);
    assert!(renamed_store.status.success());
    assert_eq!(renamed_store.stdout, renamed_clean.stdout);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let projects = corpus.join("projects");
        let database_before = fs::read(&store).expect("read store before root failure");
        fs::set_permissions(&projects, fs::Permissions::from_mode(0o000))
            .expect("make root inaccessible");
        let retained = run_generated_store(&corpus, &store, false);
        fs::set_permissions(&projects, fs::Permissions::from_mode(0o700))
            .expect("restore root permissions");
        assert!(
            retained.status.success(),
            "an inaccessible root did not return the retained report: {}",
            String::from_utf8_lossy(&retained.stderr)
        );
        let retained_json: Value =
            serde_json::from_slice(&retained.stdout).expect("parse root-retained report JSON");
        let authoritative_json: Value =
            serde_json::from_slice(&renamed_store.stdout).expect("parse authoritative report JSON");
        assert_eq!(
            retained_json["canonicalMetrics"], authoritative_json["canonicalMetrics"],
            "the inaccessible-root report discarded last-known analytical facts"
        );
        assert_eq!(
            retained_json["dataCoverage"]["completeness"], "partial",
            "retained root facts must be labeled as partial coverage"
        );
        assert!(
            retained_json["dataCoverage"]["warnings"]
                .as_array()
                .is_some_and(|warnings| warnings.iter().any(|warning| {
                    warning["code"] == "W_TRANSCRIPT_SUBTREE_INACCESSIBLE"
                        && warning["sourceAlias"] == "transcript-1"
                })),
            "the retained report omitted the inaccessible-root warning"
        );
        assert_eq!(
            fs::read(&store).expect("read store after root failure"),
            database_before,
            "an indeterminate root scan mutated authoritative store rows"
        );
        let recovered = run_generated_store(&corpus, &store, false);
        assert!(recovered.status.success());
        assert_eq!(recovered.stdout, renamed_clean.stdout);
    }
}

#[test]
fn f055_changed_transcript_is_streamed_once_for_replace_growth_and_truncate() {
    let scratch = Scratch::new("store-one-pass-mutations");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate one-pass corpus");
    let store = scratch.path.join("state/store.sqlite3");
    let first = run_generated_store(&corpus, &store, false);
    assert!(first.status.success());

    let mut transcript_files = relative_source_files(&corpus)
        .expect("enumerate one-pass corpus")
        .into_iter()
        .filter(|path| {
            path.starts_with("projects")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
        })
        .collect::<Vec<_>>();
    transcript_files.sort();
    let changed = corpus.join(transcript_files.first().expect("generated transcript"));

    let assert_one_pass = |label: &str| {
        let counters_path = scratch.path.join(format!("{label}-counters.json"));
        let stored = run_generated_store_with_counters(&corpus, &store, &counters_path);
        assert!(
            stored.status.success(),
            "{label} store import failed: {}",
            String::from_utf8_lossy(&stored.stderr)
        );
        let clean = run_generated(&corpus);
        assert!(clean.status.success(), "{label} clean import failed");
        assert_eq!(stored.stdout, clean.stdout, "{label} output drifted");
        let counters: Value =
            serde_json::from_slice(&fs::read(counters_path).expect("read one-pass counters"))
                .expect("parse one-pass counters");
        assert_eq!(
            counters["parsedSourceFiles"].as_u64(),
            Some(1),
            "{label} streamed a changed source more than once: {counters}"
        );
        assert_eq!(
            counters["sourceContentBytesRead"].as_u64(),
            Some(fs::metadata(&changed).expect("changed metadata").len()),
            "{label} physical read accounting drifted: {counters}"
        );
        assert!(
            counters["reusedSourceFiles"].as_u64().unwrap_or_default() > 0,
            "{label} reparsed unchanged files instead of using normalized payloads"
        );
    };

    let mut same_size = fs::read(&changed).expect("read same-size source");
    let identity = same_size
        .windows(b"\"id\":\"".len())
        .position(|window| window == b"\"id\":\"")
        .expect("generated message identity field")
        + b"\"id\":\"".len();
    same_size[identity] = if same_size[identity] == b'x' {
        b'y'
    } else {
        b'x'
    };
    fs::write(&changed, &same_size).expect("write same-size replacement");
    assert_one_pass("same-size");

    let mut prefix_changed_growth = fs::read(&changed).expect("read growth source");
    prefix_changed_growth[identity] = if prefix_changed_growth[identity] == b'z' {
        b'w'
    } else {
        b'z'
    };
    prefix_changed_growth.extend_from_slice(
        concat!(
            "{\"type\":\"assistant\",\"uuid\":\"one-pass-growth\",",
            "\"timestamp\":\"2026-08-02T12:00:00Z\",",
            "\"message\":{\"model\":\"claude-sonnet-4-20250514\",",
            "\"usage\":{\"input_tokens\":5,\"output_tokens\":7}}}\n"
        )
        .as_bytes(),
    );
    fs::write(&changed, prefix_changed_growth).expect("write prefix-changing growth");
    assert_one_pass("prefix-growth");

    let truncated_len = fs::metadata(&changed)
        .expect("growth metadata")
        .len()
        .saturating_div(2);
    fs::OpenOptions::new()
        .write(true)
        .open(&changed)
        .expect("open source for truncate")
        .set_len(truncated_len)
        .expect("truncate source");
    assert_one_pass("truncate");
}

#[cfg(windows)]
#[test]
fn f055_windows_timestamp_preserving_rewrite_invalidates_store() {
    let scratch = Scratch::new("store-windows-change-time");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate Windows mutation corpus");
    let store = scratch.path.join("state/store.sqlite3");
    let first = run_generated_store(&corpus, &store, false);
    assert!(first.status.success());

    let changed = relative_source_files(&corpus)
        .expect("enumerate Windows mutation corpus")
        .into_iter()
        .find(|path| {
            path.starts_with("projects")
                && path
                    .extension()
                    .is_some_and(|extension| extension == "jsonl")
        })
        .map(|path| corpus.join(path))
        .expect("generated Windows transcript");
    let original_modified = fs::metadata(&changed)
        .expect("read original Windows metadata")
        .modified()
        .expect("read original Windows last-write time");
    let mut replacement = fs::read(&changed).expect("read Windows transcript");
    let token_digit = replacement
        .windows(b"\"input_tokens\":1".len())
        .position(|window| window == b"\"input_tokens\":1")
        .expect("generated input token")
        + b"\"input_tokens\":".len();
    replacement[token_digit] = b'9';
    fs::write(&changed, replacement).expect("write equal-length Windows replacement");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&changed)
        .expect("open Windows replacement for timestamp restore");
    file.set_times(std::fs::FileTimes::new().set_modified(original_modified))
        .expect("restore Windows last-write time");
    assert_eq!(
        fs::metadata(&changed)
            .expect("read restored Windows metadata")
            .modified()
            .expect("read restored Windows last-write time"),
        original_modified
    );

    let counters_path = scratch.path.join("windows-change-time-counters.json");
    let stored = run_generated_store_with_counters(&corpus, &store, &counters_path);
    let clean = run_generated(&corpus);
    assert!(
        stored.status.success(),
        "timestamp-preserving stored import failed: {}",
        String::from_utf8_lossy(&stored.stderr)
    );
    assert!(clean.status.success());
    assert_ne!(
        stored.stdout, first.stdout,
        "the rewritten token did not change the stored report"
    );
    assert_eq!(
        stored.stdout, clean.stdout,
        "Windows change time did not invalidate the stale store row"
    );
    let counters: Value =
        serde_json::from_slice(&fs::read(counters_path).expect("read Windows counters"))
            .expect("parse Windows counters");
    assert_eq!(counters["parsedSourceFiles"].as_u64(), Some(1));
    assert_eq!(
        counters["sourceContentBytesRead"].as_u64(),
        Some(
            fs::metadata(changed)
                .expect("changed Windows metadata")
                .len()
        )
    );
}

#[cfg(unix)]
#[test]
fn f055_store_retains_last_known_report_when_one_transcript_subtree_is_inaccessible() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = Scratch::new("store-partial-subtree");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate partial-subtree corpus");
    let store = scratch.path.join("state/store.sqlite3");
    let first = run_generated_store(&corpus, &store, false);
    assert!(
        first.status.success(),
        "initial store import failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_json: Value =
        serde_json::from_slice(&first.stdout).expect("parse initial report JSON");
    let database_before = fs::read(&store).expect("read store before partial scan");

    let inaccessible = corpus.join("projects/project-00000");
    let original_mode = fs::metadata(&inaccessible)
        .expect("read subtree permissions")
        .permissions()
        .mode();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o000))
        .expect("make transcript subtree inaccessible");
    let retained = run_generated_store(&corpus, &store, false);
    fs::set_permissions(
        &inaccessible,
        fs::Permissions::from_mode(original_mode & 0o777),
    )
    .expect("restore transcript subtree permissions");

    assert!(
        retained.status.success(),
        "partial scan failed instead of returning retained coverage: {}",
        String::from_utf8_lossy(&retained.stderr)
    );
    let retained_json: Value =
        serde_json::from_slice(&retained.stdout).expect("parse retained report JSON");
    assert_eq!(
        retained_json["dataCoverage"]["acceptedRecords"],
        first_json["dataCoverage"]["acceptedRecords"],
        "the partial report must retain last-known source rows"
    );
    assert_eq!(
        retained_json["canonicalMetrics"], first_json["canonicalMetrics"],
        "last-known analytical facts changed while a subtree was inaccessible"
    );
    assert_eq!(
        retained_json["dataCoverage"]["completeness"], "partial",
        "retained facts must be labeled as partial coverage"
    );
    assert!(
        retained_json["dataCoverage"]["warnings"]
            .as_array()
            .is_some_and(|warnings| warnings.iter().any(|warning| {
                warning["code"] == "W_TRANSCRIPT_SUBTREE_INACCESSIBLE"
                    && warning["sourceAlias"] == "transcript-1"
            })),
        "the report omitted the inaccessible-subtree warning"
    );
    assert_eq!(
        fs::read(&store).expect("read store after partial scan"),
        database_before,
        "a partial subtree scan mutated authoritative store rows"
    );

    let recovered = run_generated_store(&corpus, &store, false);
    assert!(recovered.status.success());
    assert_eq!(
        recovered.stdout, first.stdout,
        "restoring the subtree did not restore the authoritative cached report"
    );
}

#[cfg(unix)]
#[test]
fn f055_partial_subtree_rejects_stale_readable_inventory() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = Scratch::new("store-partial-readable-change");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None)
        .expect("generate partial-readable-change corpus");
    let store = scratch.path.join("state/store.sqlite3");
    let first = run_generated_store(&corpus, &store, false);
    assert!(
        first.status.success(),
        "initial store import failed: {}",
        String::from_utf8_lossy(&first.stderr)
    );
    let database_before = fs::read(&store).expect("read store before partial changed scan");

    let readable_file = corpus
        .join("projects/project-00001")
        .join("session-00001.jsonl");
    let mut readable_file = fs::OpenOptions::new()
        .append(true)
        .open(&readable_file)
        .expect("open readable transcript for mutation");
    writeln!(readable_file).expect("mutate readable transcript");
    drop(readable_file);

    let inaccessible = corpus.join("projects/project-00000");
    let original_mode = fs::metadata(&inaccessible)
        .expect("read subtree permissions")
        .permissions()
        .mode();
    fs::set_permissions(&inaccessible, fs::Permissions::from_mode(0o000))
        .expect("make a different transcript subtree inaccessible");
    let rejected = run_generated_store(&corpus, &store, false);
    fs::set_permissions(
        &inaccessible,
        fs::Permissions::from_mode(original_mode & 0o777),
    )
    .expect("restore transcript subtree permissions");

    assert!(
        !rejected.status.success(),
        "a retained partial report silently ignored a readable source mutation"
    );
    let error: Value =
        serde_json::from_slice(&rejected.stdout).expect("parse partial inventory error JSON");
    assert_eq!(error["code"], "E_INCREMENTAL_STORE");
    assert_eq!(
        fs::read(&store).expect("read store after rejected partial changed scan"),
        database_before,
        "a rejected partial changed scan mutated authoritative store rows"
    );
}

#[cfg(unix)]
#[test]
fn f055_inaccessible_root_cannot_reuse_a_different_sources_retained_report() {
    use std::os::unix::fs::PermissionsExt;

    let scratch = Scratch::new("store-root-identity");
    let first_corpus = scratch.path.join("first-corpus");
    let second_corpus = scratch.path.join("second-corpus");
    generate(CorpusClass::OracleSmall, &first_corpus, None)
        .expect("generate first root-identity corpus");
    generate(CorpusClass::OracleSmall, &second_corpus, None)
        .expect("generate second root-identity corpus");
    let store = scratch.path.join("state/store.sqlite3");
    let initialized = run_generated_store(&first_corpus, &store, false);
    assert!(initialized.status.success());
    let database_before = fs::read(&store).expect("read store before mismatched root");

    let projects = second_corpus.join("projects");
    fs::set_permissions(&projects, fs::Permissions::from_mode(0o000))
        .expect("make mismatched root inaccessible");
    let rejected = run_generated_store(&second_corpus, &store, false);
    fs::set_permissions(&projects, fs::Permissions::from_mode(0o700))
        .expect("restore mismatched root permissions");

    assert!(
        !rejected.status.success(),
        "an inaccessible source reused a retained report from a different root"
    );
    assert_eq!(
        fs::read(&store).expect("read store after mismatched root"),
        database_before,
        "a rejected source selection mutated the retained store"
    );
}

#[test]
fn f056_store_is_private_corruption_is_explicit_and_rebuild_is_source_safe() {
    let scratch = Scratch::new("store-recovery");
    let corpus = scratch.path.join("corpus");
    let source_reference = scratch.path.join("source-reference");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate recovery corpus");
    generate(CorpusClass::OracleSmall, &source_reference, None).expect("generate source reference");
    assert!(byte_identity(&corpus, &source_reference).expect("measure source identity"));
    let store = scratch.path.join("state/store.sqlite3");

    let first = run_generated_store(&corpus, &store, false);
    assert!(first.status.success());
    let database = fs::read(&store).expect("read store for privacy scan");
    for canary in [
        b"SYNTHETIC_PHASE5_PROMPT_CANARY".as_slice(),
        b"SYNTHETIC_PHASE5_PATH_CANARY".as_slice(),
        b"SYNTHETIC_PHASE5_EMAIL_CANARY".as_slice(),
        corpus.as_os_str().as_encoded_bytes(),
    ] {
        assert!(
            !database
                .windows(canary.len())
                .any(|window| window == canary),
            "private source material leaked into the database"
        );
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&store)
                .expect("store metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(store.parent().expect("store parent"))
                .expect("store parent metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );

        let existing_parent = scratch.path.join("existing-store-parent");
        fs::create_dir(&existing_parent).expect("create existing store parent");
        fs::set_permissions(&existing_parent, fs::Permissions::from_mode(0o755))
            .expect("set existing parent permissions");
        let existing_parent_store = existing_parent.join("store.sqlite3");
        let existing_parent_run = run_generated_store(&corpus, &existing_parent_store, false);
        assert!(
            existing_parent_run.status.success(),
            "safe existing store parent failed: {}",
            String::from_utf8_lossy(&existing_parent_run.stderr)
        );
        assert_eq!(
            fs::metadata(&existing_parent)
                .expect("existing parent metadata")
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "ccwrapped changed permissions on an existing store parent"
        );
        assert_eq!(
            fs::metadata(&existing_parent_store)
                .expect("existing-parent store metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        let writable_parent = scratch.path.join("writable-store-parent");
        fs::create_dir(&writable_parent).expect("create writable store parent");
        fs::set_permissions(&writable_parent, fs::Permissions::from_mode(0o777))
            .expect("set writable parent permissions");
        let rejected = run_generated_store(&corpus, &writable_parent.join("store.sqlite3"), false);
        assert!(
            !rejected.status.success(),
            "a group/world-writable store parent was accepted"
        );
        let rejected_error: Value =
            serde_json::from_slice(&rejected.stdout).expect("unsafe-parent JSON error");
        assert_eq!(rejected_error["code"], "E_INCREMENTAL_STORE");
        assert_eq!(
            fs::metadata(&writable_parent)
                .expect("writable parent metadata")
                .permissions()
                .mode()
                & 0o777,
            0o777,
            "ccwrapped changed permissions on a rejected store parent"
        );
    }

    for version in 1..=8 {
        let migration_store = scratch
            .path
            .join(format!("state/migration-v{version}.sqlite3"));
        let initialized = run_generated_store(&corpus, &migration_store, false);
        assert!(initialized.status.success());
        replace_store_with_legacy_schema(&migration_store, version);

        let migrated = run_generated_store(&corpus, &migration_store, false);
        assert!(
            migrated.status.success(),
            "v{version} migration failed: {}",
            String::from_utf8_lossy(&migrated.stderr)
        );
        assert_eq!(migrated.stdout, first.stdout);
        let connection =
            rusqlite::Connection::open(&migration_store).expect("inspect migrated store");
        let actual_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read migrated schema version");
        assert_eq!(actual_version, 9);
        let cached_report_columns = connection
            .prepare("SELECT name FROM pragma_table_info('cached_report') ORDER BY cid")
            .expect("prepare cached report column query")
            .query_map([], |row| row.get::<_, String>(0))
            .expect("query cached report columns")
            .collect::<Result<Vec<_>, _>>()
            .expect("collect cached report columns");
        assert_eq!(
            cached_report_columns,
            ["singleton", "options_key", "report_json", "report_digest"]
        );
    }

    let interrupted_store = scratch.path.join("state/interrupted-v1.sqlite3");
    let initialized = run_generated_store(&corpus, &interrupted_store, false);
    assert!(initialized.status.success());
    replace_store_with_legacy_schema(&interrupted_store, 1);
    let legacy_before_failure =
        fs::read(&interrupted_store).expect("read legacy store before failed staged migration");
    let projects = corpus.join("projects");
    let projects_backup = corpus.join("projects-before-failed-migration");
    fs::rename(&projects, &projects_backup).expect("move selected source before migration failure");
    fs::write(&projects, b"not a directory").expect("replace selected source with a file");
    let interrupted = run_generated_store(&corpus, &interrupted_store, false);
    fs::remove_file(&projects).expect("remove invalid selected source");
    fs::rename(&projects_backup, &projects)
        .expect("restore selected source after migration failure");
    assert!(!interrupted.status.success());
    let interrupted_error: Value =
        serde_json::from_slice(&interrupted.stdout).expect("migration failure JSON error");
    assert_eq!(
        interrupted_error["code"],
        "E_DISCOVERY_TRANSCRIPT_NOT_DIRECTORY"
    );
    assert_eq!(
        fs::read(&interrupted_store).expect("read legacy store after failed staged migration"),
        legacy_before_failure,
        "a failed scan modified the authoritative legacy store"
    );
    {
        let connection =
            rusqlite::Connection::open(&interrupted_store).expect("inspect interrupted migration");
        let actual_version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .expect("read rolled-back schema version");
        assert_eq!(actual_version, 1);
        let format: Vec<u8> = connection
            .query_row("SELECT value FROM meta WHERE key = 'format'", [], |row| {
                row.get(0)
            })
            .expect("read rolled-back format");
        assert_eq!(format, b"ccwrapped.incremental-store/v1");
        let source_rows: usize = connection
            .query_row("SELECT count(*) FROM source_file", [], |row| row.get(0))
            .expect("count rolled-back source rows");
        let cached_rows: usize = connection
            .query_row("SELECT count(*) FROM cached_report", [], |row| row.get(0))
            .expect("count rolled-back cached rows");
        assert_eq!((source_rows, cached_rows), (1, 1));
    }
    let after_interruption = run_generated_store(&corpus, &interrupted_store, false);
    assert!(
        after_interruption.status.success(),
        "migration retry failed: {}",
        String::from_utf8_lossy(&after_interruption.stderr)
    );
    assert_eq!(after_interruption.stdout, first.stdout);

    let database_before_archive = fs::read(&store).expect("read store before private archive");
    let archive =
        run_generated_archive_with_store(&corpus, &scratch.path.join("private-archive"), &store);
    assert!(
        !archive.status.success(),
        "private archive unexpectedly accepted a conflicting store path: {}",
        String::from_utf8_lossy(&archive.stderr)
    );
    assert!(String::from_utf8_lossy(&archive.stderr).contains("cannot be used with '--archive'"));
    assert_eq!(
        fs::read(&store).expect("read store after private archive"),
        database_before_archive,
        "rejected private-content mode accessed or rewrote the standard store"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let database_before_failed_rebuild =
            fs::read(&store).expect("read valid store before failed rebuild");
        let projects = corpus.join("projects");
        fs::set_permissions(&projects, fs::Permissions::from_mode(0o000))
            .expect("make a selected rebuild branch inaccessible");
        let failed_rebuild = run_generated_store(&corpus, &store, true);
        fs::set_permissions(&projects, fs::Permissions::from_mode(0o700))
            .expect("restore selected rebuild branch");
        assert!(
            !failed_rebuild.status.success(),
            "an incomplete rebuild scan replaced the prior store"
        );
        let failed_error: Value =
            serde_json::from_slice(&failed_rebuild.stdout).expect("failed rebuild JSON error");
        assert_eq!(failed_error["code"], "E_TRANSCRIPT_INGESTION");
        assert_eq!(
            fs::read(&store).expect("read store after failed rebuild"),
            database_before_failed_rebuild,
            "a failed complete scan destroyed or changed the prior valid store"
        );
        assert!(
            fs::read_dir(store.parent().unwrap())
                .expect("read store parent after failed rebuild")
                .map(|entry| entry.expect("read store parent entry").file_name())
                .all(|name| !name
                    .to_string_lossy()
                    .starts_with(".ccwrapped-store-rebuild-")),
            "a failed rebuild left a private staging database behind"
        );
        let recovered = run_generated_store(&corpus, &store, false);
        assert!(
            recovered.status.success(),
            "prior store was unusable after failed rebuild: {}",
            String::from_utf8_lossy(&recovered.stderr)
        );
        assert_eq!(recovered.stdout, first.stdout);
    }

    fs::write(&store, b"not a sqlite database").expect("corrupt derived store");
    let corrupt = run_generated_store(&corpus, &store, false);
    assert!(!corrupt.status.success());
    let error: Value = serde_json::from_slice(&corrupt.stdout).expect("corruption JSON error");
    assert_eq!(error["code"], "E_INCREMENTAL_STORE");
    assert!(error["remediation"]
        .as_str()
        .is_some_and(|value| value.contains("--rebuild-store")));

    let rebuilt = run_generated_store(&corpus, &store, true);
    assert!(
        rebuilt.status.success(),
        "rebuild failed: {}",
        String::from_utf8_lossy(&rebuilt.stderr)
    );
    assert_eq!(rebuilt.stdout, first.stdout);
    assert!(byte_identity(&corpus, &source_reference).expect("source remains byte-identical"));
}

#[test]
fn f057_parallel_standard_workers_preserve_json_serial_private_policy_is_invariant_and_panics_fail_closed(
) {
    let scratch = Scratch::new("parallel-determinism");
    let corpus = scratch.path.join("corpus");
    generate(CorpusClass::OracleSmall, &corpus, None).expect("generate parallel corpus");

    let one = run_generated_with(&corpus, Some(1), None, None);
    assert!(
        one.status.success(),
        "single-worker run failed: {}",
        String::from_utf8_lossy(&one.stderr)
    );
    assert!(one.stderr.is_empty());

    let available = std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get);
    let mut worker_counts = [2, 4, 8, 12, 15]
        .into_iter()
        .map(|workers| workers.min(available))
        .collect::<Vec<_>>();
    worker_counts.sort_unstable();
    worker_counts.dedup();
    worker_counts.retain(|workers| *workers > 1);
    for workers in worker_counts.iter().copied() {
        for delay_seed in [7, 91] {
            let parallel = run_generated_with(&corpus, Some(workers), Some(delay_seed), None);
            assert!(
                parallel.status.success(),
                "{workers}-worker run failed: {}",
                String::from_utf8_lossy(&parallel.stderr)
            );
            assert!(parallel.stderr.is_empty());
            assert!(
                parallel.stdout == one.stdout,
                "worker scheduling changed canonical JSON (single={} bytes, parallel={} bytes)",
                one.stdout.len(),
                parallel.stdout.len()
            );
        }
    }

    let archive_one = scratch.path.join("archive-one");
    let one_archive = run_generated_serial_private_archive(&corpus, &archive_one, 1);
    assert!(one_archive.status.success());
    for workers in worker_counts {
        let archive_many = scratch.path.join(format!("archive-{workers}"));
        let many_archive = run_generated_serial_private_archive(&corpus, &archive_many, workers);
        assert!(
            many_archive.status.success(),
            "private archive with a {workers}-worker standard-policy override failed: {}",
            String::from_utf8_lossy(&many_archive.stderr)
        );
        assert!(
            byte_identity(
                &archive_one.join("wrapped-archive"),
                &archive_many.join("wrapped-archive"),
            )
            .expect("compare private archive bytes"),
            "a {workers}-worker standard-policy override changed serial private archive order"
        );
    }

    if available > 1 {
        let panic = run_generated_with(&corpus, Some(2), Some(13), Some(0));
        assert!(!panic.status.success());
        let error: Value =
            serde_json::from_slice(&panic.stdout).expect("worker panic returns JSON error");
        assert_eq!(error["code"], "E_TRANSCRIPT_INGESTION");
        assert!(!String::from_utf8_lossy(&panic.stdout).contains("SYNTHETIC_PHASE5_PROMPT_CANARY"));
    }
}

#[cfg(unix)]
#[test]
fn f057_utilization_verifier_rejects_smaller_plateau_and_unstable_selection() {
    let scratch = Scratch::new("utilization-verifier");
    let record_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/benchmarks/phase5-record.json");
    let record_bytes = fs::read(record_path).expect("read Phase 5 benchmark record");

    let mut smaller_plateau: Value =
        serde_json::from_slice(&record_bytes).expect("parse Phase 5 benchmark record");
    let points = smaller_plateau["measurements"]["scaling"]["points"]
        .as_array_mut()
        .expect("scaling points");
    let production_walls = points
        .iter()
        .find(|point| point["workerCount"] == 12)
        .expect("production scaling point")["rawAggregates"]
        .as_array()
        .expect("production raw aggregates")
        .iter()
        .map(|sample| sample["wallNanos"].clone())
        .collect::<Vec<_>>();
    let smaller = points
        .iter_mut()
        .find(|point| point["workerCount"] == 8)
        .expect("smaller scaling point")["rawAggregates"]
        .as_array_mut()
        .expect("smaller raw aggregates");
    for (sample, production_wall) in smaller.iter_mut().zip(production_walls) {
        sample["wallNanos"] = production_wall;
    }
    let smaller_path = scratch.path.join("smaller-plateau.json");
    fs::write(
        &smaller_path,
        serde_json::to_vec(&smaller_plateau).expect("encode smaller plateau mutation"),
    )
    .expect("write smaller plateau mutation");
    let smaller_result =
        Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/phase5-benchmark.sh"))
            .arg("verify-utilization")
            .env("CCWRAPPED_PHASE5_RECORD", &smaller_path)
            .output()
            .expect("run verifier against smaller plateau");
    assert!(
        !smaller_result.status.success(),
        "verifier accepted an 8-worker plateau: {}",
        String::from_utf8_lossy(&smaller_result.stdout)
    );

    let mut unstable: Value =
        serde_json::from_slice(&record_bytes).expect("parse unstable benchmark record");
    let production = unstable["measurements"]["scaling"]["points"]
        .as_array_mut()
        .expect("unstable scaling points")
        .iter_mut()
        .find(|point| point["workerCount"] == 12)
        .expect("unstable production point")["rawAggregates"]
        .as_array_mut()
        .expect("unstable raw aggregates");
    let baseline = production[0]["wallNanos"]
        .as_u64()
        .expect("production wall sample");
    production.last_mut().expect("last production sample")["wallNanos"] =
        Value::from(baseline.saturating_mul(3));
    let unstable_path = scratch.path.join("unstable-production.json");
    fs::write(
        &unstable_path,
        serde_json::to_vec(&unstable).expect("encode unstable selection mutation"),
    )
    .expect("write unstable selection mutation");
    let unstable_result =
        Command::new(Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/phase5-benchmark.sh"))
            .arg("verify-utilization")
            .env("CCWRAPPED_PHASE5_RECORD", &unstable_path)
            .output()
            .expect("run verifier against unstable selection");
    assert!(
        !unstable_result.status.success(),
        "verifier accepted an unstable production point: {}",
        String::from_utf8_lossy(&unstable_result.stdout)
    );
}
