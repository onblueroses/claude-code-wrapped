use std::path::{Path, PathBuf};
use std::process::Command;

const ARCHITECTURE: &str = include_str!("../docs/architecture.md");
const METHODOLOGY: &str = include_str!("../docs/methodology.md");
const FIXTURES: &str = include_str!("../docs/fixture-matrix.md");
const PUBLIC_API: &str = include_str!("../docs/baseline/public-api-v0.2.0.txt");
const BASELINE_CLI_HELP: &[u8] = include_bytes!("../docs/baseline/cli-help.txt");
const CURRENT_CLI_HELP: &[u8] = include_bytes!("../docs/current/cli-help.txt");
const CI: &str = include_str!("../.github/workflows/ci.yml");
const RELEASE: &str = include_str!("../.github/workflows/release.yml");
const MANIFEST: &str = include_str!("../Cargo.toml");

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

/// Every `uses:` reference in a workflow, e.g. `actions/checkout@3d3c42e…`.
///
/// Matches on the parsed key rather than a literal prefix, so the YAML forms
/// `- uses: x`, `uses : x`, and `"uses": x` all count. A prefix matcher lets a
/// floating ref hide behind a space and leaves the pin test green.
fn uses_references(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let line = line.strip_prefix("- ").unwrap_or(line);
            let (key, value) = line.split_once(':')?;
            if key.trim().trim_matches(['"', '\'']) != "uses" {
                return None;
            }
            value.split_whitespace().next()
        })
        .collect()
}

/// Every workflow file on disk, discovered at run time.
///
/// Reading the directory rather than a hard-coded list is the point: a policy
/// that only inspects the files someone remembered to add is not a policy.
fn workflow_files() -> Vec<(String, String)> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join(".github/workflows");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", dir.display()))
        .map(|entry| entry.expect("workflow directory entry").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("yml" | "yaml")
            )
        })
        .collect();
    paths.sort();

    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("workflow file name is UTF-8")
                .to_owned();
            let body = std::fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{name} is unreadable: {error}"));
            (name, body)
        })
        .collect()
}

/// The refs of every step that runs `action`, with the `action@` prefix removed.
fn action_pins<'a>(workflow: &'a str, action: &str) -> Vec<&'a str> {
    uses_references(workflow)
        .into_iter()
        .filter_map(|reference| reference.strip_prefix(action)?.strip_prefix('@'))
        .collect()
}

/// A pin is immutable only as a full commit id; tags and branches move under us.
fn is_immutable_commit_pin(reference: &str) -> bool {
    reference.len() == 40 && reference.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The values of every `toolchain:` action input, comments stripped.
fn toolchain_inputs(workflow: &str) -> Vec<&str> {
    workflow
        .lines()
        .filter_map(|line| line.trim().strip_prefix("toolchain:"))
        .map(|value| value.split('#').next().unwrap_or(value).trim())
        .collect()
}

fn manifest_rust_version() -> &'static str {
    MANIFEST
        .lines()
        .find_map(|line| line.trim().strip_prefix("rust-version"))
        .and_then(|rest| rest.trim().strip_prefix('='))
        .map(|rest| rest.trim().trim_matches('"'))
        .expect("Cargo.toml declares rust-version")
}

/// The separation this repo depends on: the toolchain *action* is pinned to an
/// immutable commit so a Dependabot bump cannot change what code runs, while the
/// Rust *version* is a separate `toolchain:` input so bumping the compiler never
/// touches the pin. Asserting the property, not one commit id, lets action
/// updates land without editing this test.
fn assert_toolchain_action_pin_is_separate_from_versions(name: &str, workflow: &str) {
    let pins = action_pins(workflow, "dtolnay/rust-toolchain");
    assert!(!pins.is_empty(), "{name}: no dtolnay/rust-toolchain step");

    for pin in &pins {
        assert!(
            is_immutable_commit_pin(pin),
            "{name}: dtolnay/rust-toolchain must be pinned to a full 40-hex commit, got @{pin}"
        );
    }

    let expected = pins[0];
    assert!(
        pins.iter().all(|pin| *pin == expected),
        "{name}: every dtolnay/rust-toolchain step must pin the same commit, got {pins:?}"
    );

    // One `toolchain:` input per pinned step: the version never rides the ref.
    assert_eq!(
        toolchain_inputs(workflow).len(),
        pins.len(),
        "{name}: every pinned toolchain step must take its Rust version from a separate `toolchain:` input"
    );
}

#[test]
fn compatibility_capture_ci_separates_the_action_pin_from_toolchain_versions() {
    assert_toolchain_action_pin_is_separate_from_versions("ci.yml", CI);

    let versions = toolchain_inputs(CI);
    let msrv = manifest_rust_version();
    assert_eq!(
        versions
            .iter()
            .filter(|version| version.strip_suffix(".0").unwrap_or(version) == msrv)
            .count(),
        1,
        "ci.yml must check the MSRV {msrv} that Cargo.toml advertises, exactly once, got {versions:?}"
    );
    assert_eq!(
        versions
            .iter()
            .filter(|version| **version == "1.95.0")
            .count(),
        2,
        "ci.yml must run the capture toolchain on Linux and Windows, got {versions:?}"
    );

    assert!(CI.contains("cargo test --all-targets"));

    // The regression this contract exists to catch: a floating ref in place of a pin.
    for floating in ["stable", "beta", "nightly", "master", "main", "1.95.0"] {
        assert!(
            !CI.contains(&format!("dtolnay/rust-toolchain@{floating}")),
            "ci.yml must not resolve dtolnay/rust-toolchain through the floating ref @{floating}"
        );
    }
}

#[test]
fn release_pins_the_same_toolchain_action_commit_as_ci() {
    assert_toolchain_action_pin_is_separate_from_versions("release.yml", RELEASE);
    assert_eq!(
        action_pins(RELEASE, "dtolnay/rust-toolchain").first(),
        action_pins(CI, "dtolnay/rust-toolchain").first(),
        "release.yml must build with the toolchain action commit that ci.yml tested"
    );
}

#[test]
fn every_workflow_action_is_pinned_to_an_immutable_commit() {
    let workflows = workflow_files();
    assert!(
        workflows.iter().any(|(name, _)| name == "ci.yml")
            && workflows.iter().any(|(name, _)| name == "release.yml"),
        "the workflow directory must still hold ci.yml and release.yml, found {:?}",
        workflows.iter().map(|(name, _)| name).collect::<Vec<_>>()
    );

    for (name, workflow) in &workflows {
        for reference in uses_references(workflow) {
            // Actions living in this repo are pinned by the checkout itself.
            if reference.starts_with("./") {
                continue;
            }
            let (action, pin) = reference
                .split_once('@')
                .unwrap_or_else(|| panic!("{name}: `uses: {reference}` carries no ref"));
            assert!(
                is_immutable_commit_pin(pin),
                "{name}: {action} must be pinned to a full 40-hex commit, got @{pin}"
            );
        }
    }
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
