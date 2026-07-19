use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const BASELINE_REVISION: &str = "1eeec07ea37e861f489696dcb2d5b2625397413d";

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("ccwrapped-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(path.join("scripts")).expect("create scratch scripts");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for ScratchDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn rustc_host() -> String {
    let output = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("query rustc host");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("rustc output is UTF-8")
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("rustc host line")
        .to_owned()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut child = Command::new("sha256sum")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start sha256sum");
    child
        .stdin
        .take()
        .expect("sha256sum stdin")
        .write_all(bytes)
        .expect("hash input");
    let output = child.wait_with_output().expect("finish sha256sum");
    assert!(output.status.success());
    String::from_utf8(output.stdout)
        .expect("sha256sum output is UTF-8")
        .split_whitespace()
        .next()
        .expect("sha256sum digest")
        .to_owned()
}

fn tree_digest(entries: &[(&str, Vec<u8>)]) -> String {
    let mut framed = Vec::new();
    for (path, bytes) in entries {
        framed.extend_from_slice(path.as_bytes());
        framed.push(0);
        framed.extend_from_slice(sha256_bytes(bytes).as_bytes());
        framed.push(0);
    }
    sha256_bytes(&framed)
}

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied directory");
    for entry in fs::read_dir(source).expect("read copied directory") {
        let entry = entry.expect("read copied entry");
        let file_type = entry.file_type().expect("read copied entry type");
        let destination_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_tree(&entry.path(), &destination_path);
        } else if file_type.is_file() {
            fs::copy(entry.path(), destination_path).expect("copy regular fixture file");
        } else {
            panic!("source-only fixture contains a non-regular entry");
        }
    }
}

fn serializable_default_report_types(public_api: &str) -> BTreeSet<String> {
    let trait_names = |marker: &str| {
        public_api
            .lines()
            .filter(|line| line.contains(marker))
            .filter_map(|line| {
                line.strip_prefix("trait ccwrapped::report::")?
                    .split_once(" :: ")
                    .map(|(name, _)| name.to_owned())
            })
            .collect::<BTreeSet<_>>()
    };
    let default_types = trait_names(" :: impl core::default::Default for ccwrapped::report::");
    let serializable_types =
        trait_names(" :: impl serde_core::ser::Serialize for ccwrapped::report::");
    default_types
        .intersection(&serializable_types)
        .cloned()
        .collect()
}

fn assert_report_fixture_inventory_complete(public_api: &str, report_inventory: &str) {
    let expected = serializable_default_report_types(public_api);
    let captured = report_inventory
        .lines()
        .filter_map(|line| line.strip_prefix("struct ccwrapped::"))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();

    assert!(
        !expected.is_empty(),
        "no serializable report types discovered"
    );
    assert_eq!(
        expected, captured,
        "fixture inventory omitted a report type"
    );
}

fn audit_report_source(root: &Path, report_source: &Path) -> Result<BTreeSet<String>, String> {
    let manifest = root.join("tests/support/report-source-audit/Cargo.toml");
    let output = Command::new("cargo")
        .args(["run", "--quiet", "--locked", "--offline", "--manifest-path"])
        .arg(&manifest)
        .arg("--")
        .arg(report_source)
        .env(
            "CARGO_TARGET_DIR",
            root.join("target/phase0-report-source-audit"),
        )
        .output()
        .map_err(|error| format!("run report source audit: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let stdout = String::from_utf8(output.stdout)
        .map_err(|error| format!("report source audit output is not UTF-8: {error}"))?;
    Ok(stdout.lines().map(str::to_owned).collect())
}

#[test]
fn phase0_report_artifact_remains_bound_to_its_frozen_extractor() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let script = fs::read(root.join("scripts/capture-report-schema.sh")).expect("read script");
    let example =
        fs::read(root.join("examples/capture_report_schema.rs")).expect("read schema example");
    let audit_manifest = fs::read(root.join("tests/support/report-source-audit/Cargo.toml"))
        .expect("read source audit manifest");
    let audit_lock = fs::read(root.join("tests/support/report-source-audit/Cargo.lock"))
        .expect("read source audit lockfile");
    let audit_source = fs::read(root.join("tests/support/report-source-audit/src/main.rs"))
        .expect("read source audit implementation");
    let expected = tree_digest(&[
        ("examples/capture_report_schema.rs", example.clone()),
        ("scripts/capture-report-schema.sh", script),
        (
            "tests/support/report-source-audit/Cargo.toml",
            audit_manifest.clone(),
        ),
        (
            "tests/support/report-source-audit/Cargo.lock",
            audit_lock.clone(),
        ),
        (
            "tests/support/report-source-audit/src/main.rs",
            audit_source.clone(),
        ),
    ]);
    let artifact = fs::read_to_string(root.join("docs/baseline/report-v1-fields.txt"))
        .expect("read report artifact");
    assert!(artifact.contains(
        "extractor-tree-sha256 fa8f2b721703d1fcf6ac5e6085497fc61095b47d3d4cf573141c49ec8bad8afb"
    ));
    assert_ne!(
        expected, "fa8f2b721703d1fcf6ac5e6085497fc61095b47d3d4cf573141c49ec8bad8afb",
        "the Phase 1 schema fixture must remain distinct from the frozen Phase 0 extractor"
    );

    let mut changed_example = example;
    changed_example.extend_from_slice(b"\n// provenance mutation\n");
    let changed = tree_digest(&[
        ("examples/capture_report_schema.rs", changed_example),
        (
            "scripts/capture-report-schema.sh",
            fs::read(root.join("scripts/capture-report-schema.sh")).expect("reread script"),
        ),
        (
            "tests/support/report-source-audit/Cargo.toml",
            audit_manifest,
        ),
        ("tests/support/report-source-audit/Cargo.lock", audit_lock),
        (
            "tests/support/report-source-audit/src/main.rs",
            audit_source,
        ),
    ]);
    assert_ne!(expected, changed, "extractor edits must change provenance");
}

#[test]
fn report_fixture_inventory_covers_the_bounded_report_domain() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let public_api = fs::read_to_string(root.join("docs/baseline/public-api-v0.2.0.txt"))
        .expect("read public API artifact");
    let report_inventory = fs::read_to_string(root.join("docs/baseline/report-v1-fields.txt"))
        .expect("read report fixture inventory");

    assert_report_fixture_inventory_complete(&public_api, &report_inventory);
    assert!(report_inventory.starts_with("artifact-format report-v1-serde-fixture-shapes/v7\n"));
    assert!(report_inventory.contains("extractor-version serde-bounded-fixture-json/v6\n"));
    assert!(report_inventory.contains("feature-surface default-no-package-features\n"));
    assert!(report_inventory.contains("crate-version 0.2.0\n"));
    assert!(report_inventory.contains("dependency-build-scripts-sha256 "));
    assert!(report_inventory.contains("dependency-sources-sha256 "));
    assert!(
        !public_api.contains("item enum ccwrapped::report::"),
        "public report enums require an exhaustive variant fixture matrix"
    );

    let expected = serializable_default_report_types(&public_api);
    let audited = audit_report_source(root, &root.join("src/report.rs"))
        .unwrap_or_else(|error| panic!("{error}"));
    let added = audited
        .difference(&expected)
        .cloned()
        .collect::<BTreeSet<_>>();
    let removed = expected
        .difference(&audited)
        .cloned()
        .collect::<BTreeSet<_>>();
    assert!(
        removed.is_empty(),
        "Phase 1 removed report types: {removed:?}"
    );
    assert_eq!(
        added,
        BTreeSet::from([
            "ActiveTimeMetrics".to_string(),
            "CanonicalCacheMetrics".to_string(),
            "CanonicalCostMetrics".to_string(),
            "CanonicalMetrics".to_string(),
            "CanonicalTokenMetrics".to_string(),
            "CostMetricValue".to_string(),
            "DailyActiveTime".to_string(),
            "DataCoverage".to_string(),
            "IngestionWarning".to_string(),
            "InsightAction".to_string(),
            "InsightCard".to_string(),
            "InsightComparison".to_string(),
            "InsightFact".to_string(),
            "InsightFamilyStatus".to_string(),
            "InsightReport".to_string(),
            "InsightWindow".to_string(),
            "MethodologyCatalog".to_string(),
            "MetricMethod".to_string(),
            "MetricReconciliation".to_string(),
            "ModelCostEvidence".to_string(),
            "NamedActiveTime".to_string(),
            "NamedTokenMetricSet".to_string(),
            "PricingRegistryMetadata".to_string(),
            "PricingRegistryRecordMetadata".to_string(),
            "RatioMetric".to_string(),
            "SourceCoverage".to_string(),
            "TokenMetricSet".to_string(),
            "TokenMetricValue".to_string(),
            "UnknownShapeDiagnostic".to_string(),
        ]),
        "live report-domain additions must remain the allowlisted Phase 1-3 types"
    );
}

#[test]
fn source_audit_rejects_hidden_serde_behavior() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = ScratchDir::new("report-source-audit");
    let cases = [
        (
            "cfg-attr.rs",
            r#"
                use serde::Serialize;

                #[derive(Default, Serialize)]
                #[serde(rename_all = "camelCase")]
                pub struct Probe {
                    #[cfg_attr(unix, serde(skip_serializing_if = "Option::is_none"))]
                    pub optional: Option<String>,
                }
            "#,
            "field-level Serde behavior",
        ),
        (
            "cfg-field.rs",
            r#"
                use serde::Serialize;

                #[derive(Default, Serialize)]
                #[serde(rename_all = "camelCase")]
                pub struct Probe {
                    #[cfg(target_os = "linux")]
                    pub linux_only: String,
                }
            "#,
            "unsupported attribute",
        ),
        (
            "manual-serialize.rs",
            r#"
                pub struct Probe;

                impl serde::Serialize for Probe {
                    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
                    where
                        S: serde::Serializer,
                    {
                        serializer.serialize_unit_struct("Probe")
                    }
                }
            "#,
            "manual Serde implementation",
        ),
        (
            "aliased-manual-serialize.rs",
            r#"
                use serde::Serialize as S;

                #[derive(Default, serde::Serialize)]
                #[serde(rename_all = "camelCase")]
                pub struct Probe {
                    pub helper: PrivateHelper,
                }

                #[derive(Default)]
                pub struct PrivateHelper;

                impl S for PrivateHelper {
                    fn serialize<T>(&self, serializer: T) -> Result<T::Ok, T::Error>
                    where
                        T: serde::Serializer,
                    {
                        serializer.serialize_unit_struct("PrivateHelper")
                    }
                }
            "#,
            "manual trait implementation `S`",
        ),
    ];

    for (name, source, expected_error) in cases {
        let path = scratch.path().join(name);
        fs::write(&path, source).expect("write adversarial report source");
        let error = audit_report_source(root, &path).expect_err("unsafe Serde source must fail");
        assert!(
            error.contains(expected_error),
            "expected `{expected_error}` in `{error}`"
        );
    }
}

#[test]
fn aliased_serialize_derive_is_rejected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = ScratchDir::new("aliased-serialize-derive");
    let path = scratch.path().join("aliased-derive.rs");
    fs::write(
        &path,
        r#"
            use serde::Serialize as S;

            #[derive(Default, S)]
            #[serde(rename_all = "camelCase")]
            pub struct Probe {
                pub value: String,
            }
        "#,
    )
    .expect("write aliased derive source");

    let error = audit_report_source(root, &path).expect_err("aliased derive must fail");
    assert!(
        error.contains("renamed or glob import"),
        "unexpected alias error: {error}"
    );
}

#[test]
fn conditional_serialize_derive_is_rejected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = ScratchDir::new("conditional-serialize-derive");
    let path = scratch.path().join("conditional-derive.rs");
    fs::write(
        &path,
        r#"
            use serde::Serialize;

            #[cfg_attr(target_os = "linux", derive(Default, Serialize))]
            #[serde(rename_all = "camelCase")]
            pub struct Probe {
                pub value: String,
            }
        "#,
    )
    .expect("write conditional derive source");

    let error = audit_report_source(root, &path).expect_err("conditional derive must fail");
    assert!(
        error.contains("uses cfg_attr"),
        "unexpected conditional derive error: {error}"
    );
}

#[test]
fn private_module_public_reexport_is_rejected() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = ScratchDir::new("private-module-reexport");
    let path = scratch.path().join("private-reexport.rs");
    fs::write(
        &path,
        r#"
            mod hidden {
                #[derive(Default, serde::Serialize)]
                #[serde(rename_all = "camelCase")]
                pub struct Added {
                    pub value: String,
                }
            }

            pub use hidden::Added;
        "#,
    )
    .expect("write private-module re-export source");

    let error = audit_report_source(root, &path).expect_err("public re-export must fail");
    assert!(
        error.contains("public re-export"),
        "unexpected re-export error: {error}"
    );
}

#[test]
fn report_generator_rejects_hidden_serde_behavior() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = ScratchDir::new("report-generator-source-audit");
    fs::copy(root.join("Cargo.toml"), scratch.path().join("Cargo.toml"))
        .expect("copy root manifest");
    fs::copy(root.join("Cargo.lock"), scratch.path().join("Cargo.lock"))
        .expect("copy root lockfile");
    copy_tree(&root.join("src"), &scratch.path().join("src"));
    fs::create_dir_all(scratch.path().join("examples")).expect("create scratch examples");
    fs::copy(
        root.join("examples/capture_report_schema.rs"),
        scratch.path().join("examples/capture_report_schema.rs"),
    )
    .expect("copy report schema example");
    fs::copy(
        root.join("scripts/capture-report-schema.sh"),
        scratch.path().join("scripts/capture-report-schema.sh"),
    )
    .expect("copy report generator");
    copy_tree(
        &root.join("tests/support/report-source-audit"),
        &scratch.path().join("tests/support/report-source-audit"),
    );

    let report_path = scratch.path().join("src/report.rs");
    let report_source = fs::read_to_string(&report_path).expect("read copied report source");
    let hidden_serde = report_source.replacen(
        "    pub project_hash: String,",
        concat!(
            "    #[cfg_attr(unix, serde(skip_serializing_if = \"String::is_empty\"))]\n",
            "    pub project_hash: String,"
        ),
        1,
    );
    assert_ne!(report_source, hidden_serde, "report fixture field moved");
    fs::write(report_path, hidden_serde).expect("inject hidden Serde behavior");

    let result = Command::new("bash")
        .arg("scripts/capture-report-schema.sh")
        .arg("direct-report.txt")
        .current_dir(scratch.path())
        .env("CCWRAPPED_CAPTURE_REVISION", BASELINE_REVISION)
        .output()
        .expect("run structurally unsafe report generator");
    assert!(!result.status.success(), "generator bypassed source audit");
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("field-level Serde behavior"),
        "unexpected generator error: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
#[should_panic(expected = "fixture inventory omitted a report type")]
fn uncaptured_serializable_report_type_is_rejected() {
    let public_api = concat!(
        "trait ccwrapped::report::Existing :: impl core::default::Default for ccwrapped::report::Existing\n",
        "trait ccwrapped::report::Existing :: impl serde_core::ser::Serialize for ccwrapped::report::Existing\n",
        "trait ccwrapped::report::Added :: impl core::default::Default for ccwrapped::report::Added\n",
        "trait ccwrapped::report::Added :: impl serde_core::ser::Serialize for ccwrapped::report::Added\n",
    );
    let report_inventory = "struct ccwrapped::Existing\n";

    assert_report_fixture_inventory_complete(public_api, report_inventory);
}

#[test]
fn capture_rejects_an_unpinned_cargo() {
    let scratch = ScratchDir::new("report-schema-toolchain");
    let script_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/capture-report-schema.sh");
    let script = fs::read_to_string(script_path).expect("read report capture script");
    let mismatched = script.replace(
        "required_cargo_release=1.95.0",
        "required_cargo_release=0.0.0",
    );
    assert_ne!(script, mismatched, "Cargo pin changed unexpectedly");
    fs::write(
        scratch.path().join("scripts/capture-report-schema.sh"),
        mismatched,
    )
    .expect("write mismatched report capture script");

    let result = Command::new("bash")
        .arg("scripts/capture-report-schema.sh")
        .current_dir(scratch.path())
        .output()
        .expect("run mismatched report capture script");
    assert!(!result.status.success(), "mismatched Cargo must fail");
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("report schema capture requires the pinned Phase 0 toolchain"));
}

#[test]
fn phase0_report_schema_stays_immutable_while_current_v2_shape_is_captured() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = ScratchDir::new("current-report-schema");
    fs::copy(root.join("Cargo.toml"), scratch.path().join("Cargo.toml")).expect("copy manifest");
    fs::copy(root.join("Cargo.lock"), scratch.path().join("Cargo.lock")).expect("copy lockfile");
    copy_tree(&root.join("src"), &scratch.path().join("src"));
    copy_tree(&root.join("examples"), &scratch.path().join("examples"));
    copy_tree(
        &root.join("tests/support"),
        &scratch.path().join("tests/support"),
    );
    fs::copy(
        root.join("scripts/capture-report-schema.sh"),
        scratch.path().join("scripts/capture-report-schema.sh"),
    )
    .expect("copy report capture script");
    let output = scratch.path().join("current-report-schema.txt");
    let result = Command::new("bash")
        .arg("scripts/capture-report-schema.sh")
        .arg(&output)
        .current_dir(scratch.path())
        .env("CARGO_TERM_COLOR", "never")
        .env("CCWRAPPED_CAPTURE_REVISION", "unverified")
        .env("RUSTC", "/ccwrapped/ambient/rustc-must-not-run")
        .env(
            "RUSTC_WRAPPER",
            "/ccwrapped/ambient/rustc-wrapper-must-not-run",
        )
        .env(
            "RUSTC_WORKSPACE_WRAPPER",
            "/ccwrapped/ambient/workspace-wrapper-must-not-run",
        )
        .env(
            "CARGO_BUILD_RUSTC",
            "/ccwrapped/ambient/cargo-rustc-must-not-run",
        )
        .env(
            "CARGO_BUILD_RUSTC_WRAPPER",
            "/ccwrapped/ambient/cargo-wrapper-must-not-run",
        )
        .env(
            "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
            "/ccwrapped/ambient/cargo-workspace-wrapper-must-not-run",
        )
        .env("RUSTFLAGS", "--ccwrapped-invalid-rust-flag")
        .env(
            "CARGO_ENCODED_RUSTFLAGS",
            "--ccwrapped-invalid-encoded-flag",
        )
        .env("CARGO_BUILD_RUSTFLAGS", "--ccwrapped-invalid-build-flag")
        .env("CARGO_BUILD_TARGET", "ccwrapped-invalid-target")
        .env(
            format!(
                "CARGO_TARGET_{}_RUNNER",
                rustc_host().to_uppercase().replace('-', "_")
            ),
            "/ccwrapped/ambient/runner-must-not-run",
        )
        .env(
            format!(
                "CARGO_TARGET_{}_RUSTFLAGS",
                rustc_host().to_uppercase().replace('-', "_")
            ),
            "--ccwrapped-invalid-target-flag",
        )
        .output()
        .expect("run report schema capture");
    assert!(
        result.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );

    let actual = fs::read(&output).expect("read generated report schema");
    let checked_current = fs::read(root.join("docs/current/report-v2-fields.txt"))
        .expect("read checked current report schema");
    assert_eq!(
        actual, checked_current,
        "the current report schema capture drifted from its checked mechanical authority"
    );
    let actual_text = String::from_utf8(actual.clone()).expect("schema is UTF-8");
    assert!(actual_text.contains("source-revision-status unverified-gitless"));
    let current_extractor = tree_digest(&[
        (
            "examples/capture_report_schema.rs",
            fs::read(root.join("examples/capture_report_schema.rs")).unwrap(),
        ),
        (
            "scripts/capture-report-schema.sh",
            fs::read(root.join("scripts/capture-report-schema.sh")).unwrap(),
        ),
        (
            "tests/support/report-source-audit/Cargo.toml",
            fs::read(root.join("tests/support/report-source-audit/Cargo.toml")).unwrap(),
        ),
        (
            "tests/support/report-source-audit/Cargo.lock",
            fs::read(root.join("tests/support/report-source-audit/Cargo.lock")).unwrap(),
        ),
        (
            "tests/support/report-source-audit/src/main.rs",
            fs::read(root.join("tests/support/report-source-audit/src/main.rs")).unwrap(),
        ),
    ]);
    assert!(actual_text.contains(&format!("extractor-tree-sha256 {current_extractor}")));
    for required in [
        "fixture report-default\njson-path $: object",
        "json-path $.inflection: null",
        "fixture report-populated\njson-path $: object",
        "json-path $.schemaVersion: string",
        "json-path $.dataCoverage: object",
        "json-path $.dataCoverage.sources[0].adapterVersion: string",
        "json-path $.dataCoverage.warnings[0].code: string",
        "json-path $.dataCoverage.unknownShapes[0].structuralFields.type: string",
        "json-path $.methodology.pricingRegistry.version: string",
        "json-path $.methodology.pricingRegistry.records[0].provider: string",
        "json-path $.methodology.pricingRegistry.records[0].aliases[0]: string",
        "json-path $.methodology.pricingRegistry.records[0].effectiveStart: string",
        "json-path $.methodology.pricingRegistry.records[0].effectiveEnd: null",
        "json-path $.methodology.pricingRegistry.records[0].modifier: string",
        "json-path $.methodology.pricingRegistry.records[0].inputPicoUsdPerToken: number",
        "json-path $.methodology.pricingRegistry.records[0].outputPicoUsdPerToken: number",
        "json-path $.methodology.pricingRegistry.records[0].cacheReadPicoUsdPerToken: number",
        "json-path $.methodology.pricingRegistry.records[0].cacheWrite5mPicoUsdPerToken: number",
        "json-path $.methodology.pricingRegistry.records[0].cacheWrite1hPicoUsdPerToken: number",
        "json-path $.canonicalMetrics.tokens.global.input.limitations[0]: string",
        "json-path $.canonicalMetrics.activeTime.totalActiveSeconds: number",
        "json-path $.canonicalMetrics.tokens.global.input.availability: string",
        "json-path $.canonicalMetrics.cost.localApiEquivalent.amountUsd: number",
        "json-path $.canonicalMetrics.cost.billingAuthoritative.amountUsd: null",
        "json-path $.canonicalMetrics.cache.readShare.valuePct: number",
        "json-path $.canonicalMetrics.reconciliation.status: string",
        "json-path $.inflection: object",
        "json-path $.projectBreakdown[0].path: string",
        "json-path $.sessionBreakdown.sessions[0].prompts[0].text: string",
        "fixture serde-probe-default",
        "json-path $.custom: string",
        "fixture serde-probe-populated",
        "json-path $.flattened: number",
        "json-path $.optional: string",
        "json-path $.values[0]: number",
    ] {
        assert!(actual_text.contains(required), "missing probe: {required}");
    }
    for report_type in [
        "struct ccwrapped::IngestionWarning",
        "struct ccwrapped::SourceCoverage",
        "struct ccwrapped::DataCoverage",
        "struct ccwrapped::UnknownShapeDiagnostic",
        "struct ccwrapped::MethodologyCatalog",
        "struct ccwrapped::CanonicalMetrics",
        "struct ccwrapped::TokenMetricValue",
        "struct ccwrapped::PricingRegistryRecordMetadata",
        "struct ccwrapped::RatioMetric",
    ] {
        assert!(
            actual_text.contains(report_type),
            "current fixture inventory omitted {report_type}"
        );
    }
    let baseline = fs::read_to_string(root.join("docs/baseline/report-v1-fields.txt"))
        .expect("read immutable Phase 0 report schema");
    assert!(baseline.starts_with("artifact-format report-v1-serde-fixture-shapes/v7\n"));
    assert!(!baseline.contains("json-field ccwrapped::Report::schemaVersion"));
    assert!(!baseline.contains("struct ccwrapped::DataCoverage"));
}
