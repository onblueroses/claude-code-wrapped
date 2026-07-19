use std::process::Command;

const ARCHITECTURE: &str = include_str!("../docs/architecture.md");
const METHODOLOGY: &str = include_str!("../docs/methodology.md");
const FIXTURES: &str = include_str!("../docs/fixture-matrix.md");
const PUBLIC_API: &str = include_str!("../docs/baseline/public-api-v0.2.0.txt");
const BASELINE_CLI_HELP: &[u8] = include_bytes!("../docs/baseline/cli-help.txt");
const CURRENT_CLI_HELP: &[u8] = include_bytes!("../docs/current/cli-help.txt");
const CI: &str = include_str!("../.github/workflows/ci.yml");

#[test]
fn cli_help_matches_current_contract_without_mutating_the_phase0_baseline() {
    let output = Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
        .arg("--help")
        .env("NO_COLOR", "1")
        .output()
        .expect("run ccwrapped --help");
    assert!(
        output.status.success(),
        "--help failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stderr.is_empty(), "--help wrote to stderr");
    assert_eq!(
        CURRENT_CLI_HELP, output.stdout,
        "refresh docs/current/cli-help.txt"
    );
    assert_eq!(BASELINE_CLI_HELP.len(), 665, "Phase 0 help bytes changed");
    assert!(!String::from_utf8_lossy(BASELINE_CLI_HELP).contains("--data-dir"));
    assert_ne!(
        BASELINE_CLI_HELP, CURRENT_CLI_HELP,
        "Phase 1 CLI additions must remain an explicit diff from the Phase 0 baseline"
    );
}

#[test]
fn otlp_contract_is_bounded() {
    for required in [
        "16 MiB physical line",
        "100,000 records/points per export object",
        "128 attributes",
        "1,000,000 distinct metric streams",
        "Oversized lines are drained without buffering their tail",
    ] {
        assert!(ARCHITECTURE.contains(required), "missing: {required}");
    }
    assert!(FIXTURES.contains("OTLP events, producer contract, and resource limits"));
}

#[test]
fn otlp_contract_pins_the_wire_producer_and_schema() {
    for required in [
        "OpenTelemetry Collector Contrib `v0.148.0`",
        "file exporter module `v0.148.0`",
        "Collector pdata `v1.54.0`",
        "`go.opentelemetry.io/proto/slim/otlp` `v1.10.0`",
        "producer verification `unverified`",
        "incompatible required path/type is a bounded\nunsupported shape",
    ] {
        assert!(ARCHITECTURE.contains(required), "missing: {required}");
    }
    assert!(FIXTURES.contains("alternate/incompatible exporter shape"));
}

#[test]
fn partial_otel_only_is_canonical() {
    assert!(ARCHITECTURE.contains("A sole available source family is canonical"));
    assert!(FIXTURES.contains("sole partial OTel remains canonical with limitation"));
}

#[test]
fn session_count_is_distinct_per_query() {
    assert!(METHODOLOGY.contains("session/distinct-observed/v1"));
    assert!(METHODOLOGY.contains("not summed to produce\na month/year session count"));
    assert!(FIXTURES.contains("distinct once per query/day and non-additive wider count"));
}

#[test]
fn mixed_source_discovery_is_defined() {
    for selector in ["--data-dir PATH", "--otel-file PATH"] {
        assert!(ARCHITECTURE.contains(selector), "missing: {selector}");
    }
    assert!(ARCHITECTURE.contains("Telemetry enrichment is never discovered implicitly"));
    assert!(ARCHITECTURE.contains("There is no `--git-repo` option"));
}

#[test]
fn public_api_baseline_has_signatures_and_traits() {
    assert!(PUBLIC_API.starts_with("artifact-format public-api-signatures/v7\n"));
    for required in [
        "extractor-version rustdoc-html/v11",
        "rustc-release 1.95.0",
        "rustdoc-release 1.95.0",
        "cargo-release 1.95.0",
        "target x86_64-unknown-linux-gnu",
        "feature-surface default-no-package-features",
        "crate-version 0.2.0",
        "source-revision-claim 1eeec07ea37e861f489696dcb2d5b2625397413d",
        "source-tree-sha256 ",
        "extractor-tree-sha256 ",
        "dependency-build-scripts-sha256 ",
        "dependency-sources-sha256 ",
    ] {
        assert!(PUBLIC_API.contains(required), "missing: {required}");
    }
    assert!(!PUBLIC_API.contains("core::marker::Freeze"));
    assert!(!PUBLIC_API.contains("core::marker::UnsafeUnpin"));
    assert!(!PUBLIC_API.contains("Show 16 fields"));
    assert!(!PUBLIC_API.contains("Show 21 fields"));
    assert!(PUBLIC_API
        .contains("item fn ccwrapped::analyzers::cost::analyze_usage :: pub fn analyze_usage("));
    assert!(PUBLIC_API.contains(
        "trait ccwrapped::report::TokenUsage :: impl core::ops::arith::AddAssign<&ccwrapped::report::TokenUsage> for ccwrapped::report::TokenUsage"
    ));
    assert!(PUBLIC_API
        .contains("method ccwrapped::report::TokenUsage :: pub fn total_tokens(&self) -> u64"));
}

#[test]
fn compatibility_capture_ci_uses_the_pinned_toolchain() {
    assert!(CI.contains("dtolnay/rust-toolchain@f133eefe930d61f0d9371efd474daf0125ed3dd1"));
    assert!(CI.contains("cargo test --all-targets"));
    assert!(!CI.contains("dtolnay/rust-toolchain@stable"));
}

#[test]
fn store_salt_is_not_canonical() {
    assert!(ARCHITECTURE.contains(
        "The salt and salted values never enter standard output, public aliases,\ncanonical sort keys, or the canonical payload"
    ));
    assert!(ARCHITECTURE.contains(
        "Random salts, store identities, file metadata, and canonical path values do\nnot participate"
    ));
}
