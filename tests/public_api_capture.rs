use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const BASELINE_REVISION: &str = "1eeec07ea37e861f489696dcb2d5b2625397413d";
static NEXT_SCRATCH_ID: AtomicU64 = AtomicU64::new(0);

struct ScratchDir(PathBuf);

impl ScratchDir {
    fn new() -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock before epoch")
            .as_nanos();
        let scratch_id = NEXT_SCRATCH_ID.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ccwrapped-public-api-{}-{nonce}-{scratch_id}",
            std::process::id()
        ));
        fs::create_dir_all(path.join("src")).expect("create scratch src");
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
    let rustc_version = Command::new("rustc")
        .arg("-vV")
        .output()
        .expect("query rustc host");
    assert!(rustc_version.status.success());
    let rustc_version = String::from_utf8(rustc_version.stdout).expect("rustc output is UTF-8");
    rustc_version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .expect("rustc host line")
        .to_owned()
}

fn ensure_lockfile(root: &Path) {
    if root.join("Cargo.lock").exists() {
        return;
    }
    let result = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .expect("generate scratch lockfile");
    assert!(
        result.status.success(),
        "lockfile generation failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

fn run_capture_with_env(root: &Path, output: &str, environment: &[(&str, &str)]) -> String {
    ensure_lockfile(root);
    let host = rustc_host();

    let mut command = Command::new("bash");
    command
        .arg("scripts/capture-public-api.sh")
        .arg(output)
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .env("CARGO_TARGET_DIR", root.join("configured-target"))
        .env("CARGO_BUILD_TARGET", &host)
        .env("CCWRAPPED_CAPTURE_REVISION", BASELINE_REVISION);
    for (key, value) in environment {
        command.env(key, value);
    }
    let result = command.output().expect("run capture script");
    assert!(
        result.status.success(),
        "capture failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::read_to_string(root.join(output)).expect("read API artifact")
}

fn run_capture(root: &Path, output: &str) -> String {
    run_capture_with_env(root, output, &[])
}

fn header_value<'a>(artifact: &'a str, key: &str) -> &'a str {
    artifact
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key} ")))
        .unwrap_or_else(|| panic!("missing artifact header: {key}"))
}

fn copy_capture_script(root: &Path) {
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("scripts/capture-public-api.sh"),
        root.join("scripts/capture-public-api.sh"),
    )
    .expect("copy capture script");
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create copied directory");
    for entry in fs::read_dir(source).expect("read copied directory") {
        let entry = entry.expect("read copied entry");
        let file_type = entry.file_type().expect("read copied entry type");
        let destination_path = destination.join(entry.file_name());
        if file_type.is_dir() {
            copy_directory(&entry.path(), &destination_path);
        } else if file_type.is_file() {
            fs::copy(entry.path(), destination_path).expect("copy regular source file");
        } else {
            panic!("current-source capture contains a non-regular input");
        }
    }
}

fn public_contract_lines(artifact: &str) -> std::collections::BTreeSet<&str> {
    artifact
        .lines()
        .filter(|line| {
            line.starts_with("item ")
                || line.starts_with("trait ")
                || line.starts_with("method ")
                || line.starts_with("associated ")
        })
        .collect()
}

#[test]
fn phase0_public_api_stays_immutable_while_current_surface_has_allowlisted_additions() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"));
    let scratch = ScratchDir::new();
    fs::copy(
        repository.join("Cargo.toml"),
        scratch.path().join("Cargo.toml"),
    )
    .expect("copy manifest");
    fs::copy(
        repository.join("Cargo.lock"),
        scratch.path().join("Cargo.lock"),
    )
    .expect("copy lockfile");
    copy_directory(&repository.join("src"), &scratch.path().join("src"));
    copy_capture_script(scratch.path());
    let actual = run_capture_with_env(
        scratch.path(),
        "current-public-api.txt",
        &[("CCWRAPPED_CAPTURE_REVISION", "unverified")],
    );
    assert_eq!(
        header_value(&actual, "source-revision-status"),
        "unverified-gitless"
    );
    let checked_current = fs::read_to_string(repository.join("docs/current/public-api-v0.3.0.txt"))
        .expect("read checked current public API");
    let baseline = fs::read_to_string(repository.join("docs/baseline/public-api-v0.2.0.txt"))
        .expect("read immutable Phase 0 public API");
    assert_eq!(
        actual, checked_current,
        "the current public API capture drifted from its checked mechanical authority"
    );

    let baseline_contract = public_contract_lines(&baseline);
    let actual_contract = public_contract_lines(&actual);
    let added = actual_contract
        .difference(&baseline_contract)
        .copied()
        .collect::<Vec<_>>();
    let removed = baseline_contract
        .difference(&actual_contract)
        .copied()
        .collect::<Vec<_>>();
    let allowed_function_paths = [
        "ccwrapped::readers::discovery::try_discover_jsonl_files",
        "ccwrapped::readers::discovery::try_discover_session_files",
        "ccwrapped::readers::jsonl::try_read_all_jsonl",
        "ccwrapped::readers::session::try_read_session_breakdown",
        "ccwrapped::renderers::terminal::widgets::terminal_text",
        "ccwrapped::renderers::terminal::try_render_terminal",
        "ccwrapped::renderers::terminal::try_render_terminal_to",
        "ccwrapped::renderers::terminal::try_render_terminal_with",
    ];
    let additive_type_paths = [
        "ccwrapped::readers::IngestionReadError",
        "ccwrapped::report::ActiveTimeMetrics",
        "ccwrapped::report::CanonicalCacheMetrics",
        "ccwrapped::report::CanonicalCostMetrics",
        "ccwrapped::report::CanonicalMetrics",
        "ccwrapped::report::CanonicalTokenMetrics",
        "ccwrapped::report::CostMetricValue",
        "ccwrapped::report::DailyActiveTime",
        "ccwrapped::report::DataCoverage",
        "ccwrapped::report::IngestionWarning",
        "ccwrapped::report::InsightAction",
        "ccwrapped::report::InsightCard",
        "ccwrapped::report::InsightComparison",
        "ccwrapped::report::InsightFact",
        "ccwrapped::report::InsightFamilyStatus",
        "ccwrapped::report::InsightReport",
        "ccwrapped::report::InsightWindow",
        "ccwrapped::report::MethodologyCatalog",
        "ccwrapped::report::MetricMethod",
        "ccwrapped::report::MetricReconciliation",
        "ccwrapped::report::ModelCostEvidence",
        "ccwrapped::report::NamedActiveTime",
        "ccwrapped::report::NamedTokenMetricSet",
        "ccwrapped::report::PricingRegistryMetadata",
        "ccwrapped::report::PricingRegistryRecordMetadata",
        "ccwrapped::report::RatioMetric",
        "ccwrapped::report::SourceCoverage",
        "ccwrapped::report::TokenMetricSet",
        "ccwrapped::report::TokenMetricValue",
        "ccwrapped::report::UnknownShapeDiagnostic",
    ];
    let replaced_struct_paths = [
        "ccwrapped::report::DailyAggregate",
        "ccwrapped::report::ModelAggregate",
        "ccwrapped::report::ModelRouting",
        "ccwrapped::report::ProjectSummary",
        "ccwrapped::report::Report",
        "ccwrapped::report::SessionBreakdown",
        "ccwrapped::report::SessionSummary",
        "ccwrapped::report::SubagentSummary",
    ];
    let cached_report_deserialization_types = [
        "ActiveTimeMetrics",
        "Anomaly",
        "AnomalyReport",
        "AnomalyStats",
        "AssistantEntry",
        "CacheGrade",
        "CacheHealth",
        "CacheMood",
        "CacheReason",
        "CacheSavings",
        "CacheSignals",
        "CanonicalCacheMetrics",
        "CanonicalCostMetrics",
        "CanonicalMetrics",
        "CanonicalTokenMetrics",
        "CostAnalysis",
        "CostMetricValue",
        "CostTokens",
        "DailyActiveTime",
        "DailyAggregate",
        "DailyCost",
        "DataCoverage",
        "HeroStat",
        "Highlight",
        "InflectionPoint",
        "IngestionWarning",
        "InsightAction",
        "InsightCard",
        "InsightComparison",
        "InsightFact",
        "InsightFamilyStatus",
        "InsightReport",
        "InsightWindow",
        "MethodologyCatalog",
        "MetricMethod",
        "MetricReconciliation",
        "ModelAggregate",
        "ModelCostBreakdown",
        "ModelCostEvidence",
        "ModelRouting",
        "NamedActiveTime",
        "NamedCount",
        "NamedTokenMetricSet",
        "PricingRegistryMetadata",
        "PricingRegistryRecordMetadata",
        "ProjectSummary",
        "PromptRatio",
        "RatioMetric",
        "Recommendation",
        "Report",
        "SessionBreakdown",
        "SessionCostStats",
        "SessionIntel",
        "SessionPrompt",
        "SessionSummary",
        "SourceCoverage",
        "StoryCard",
        "SubagentSummary",
        "TimeBucket",
        "TokenMetricSet",
        "TokenMetricValue",
        "TokenUsage",
        "ToolCount",
        "TopProject",
        "TopTool",
        "UnknownShapeDiagnostic",
        "WrappedStory",
    ];
    assert!(
        added.iter().all(|line| {
            let exact_subject = |prefix: &str| {
                line.strip_prefix(prefix)
                    .and_then(|rest| rest.split_once(" :: ").map(|(subject, _)| subject))
            };
            exact_subject("item fn ")
                .is_some_and(|subject| allowed_function_paths.contains(&subject))
                || exact_subject("item struct ").is_some_and(|subject| {
                    additive_type_paths.contains(&subject)
                        || replaced_struct_paths.contains(&subject)
                })
                || exact_subject("method ")
                    .is_some_and(|subject| subject == "ccwrapped::readers::IngestionReadError")
                || exact_subject("trait ")
                    .is_some_and(|subject| additive_type_paths.contains(&subject))
                || cached_report_deserialization_types.iter().any(|name| {
                    *line
                        == format!(
                            "trait ccwrapped::report::{name} :: impl<'de> \
                             serde_core::de::Deserialize<'de> for ccwrapped::report::{name}"
                        )
                })
        }),
        "unexpected current public additions: {added:#?}"
    );
    assert_eq!(
        removed.len(),
        replaced_struct_paths.len(),
        "only allowlisted additive struct signatures may be replaced: {removed:#?}"
    );
    assert!(
        removed.iter().all(|line| {
            line.strip_prefix("item struct ")
                .and_then(|rest| rest.split_once(" :: ").map(|(subject, _)| subject))
                .is_some_and(|subject| replaced_struct_paths.contains(&subject))
        }),
        "unexpected public removal: {removed:#?}"
    );
    assert!(
        actual.contains("item struct ccwrapped::report::DataCoverage :: pub struct DataCoverage")
    );
    assert!(actual.contains(
        "item struct ccwrapped::readers::IngestionReadError :: pub struct IngestionReadError"
    ));
    assert!(actual.contains("item fn ccwrapped::readers::jsonl::try_read_all_jsonl"));
    assert!(actual.contains("item fn ccwrapped::readers::session::try_read_session_breakdown"));
    assert!(actual.contains("item fn ccwrapped::readers::discovery::try_discover_jsonl_files"));
    assert!(actual.contains("item fn ccwrapped::readers::discovery::try_discover_session_files"));
    assert!(actual.contains("item fn ccwrapped::renderers::terminal::try_render_terminal"));
    assert!(actual.contains("item fn ccwrapped::renderers::terminal::try_render_terminal_with"));
    assert!(actual.contains("item fn ccwrapped::renderers::terminal::try_render_terminal_to"));
    assert!(actual.contains("pub schema_version: alloc::string::String, pub generated_at:"));
    assert!(baseline.contains("source-tree-sha256 63bb21a4"));
    assert!(!baseline.contains("ccwrapped::report::DataCoverage"));
}

fn force_private_rustdoc_output(root: &Path) {
    let path = root.join("scripts/capture-public-api.sh");
    let script = fs::read_to_string(&path).expect("read copied capture script");
    let forced = script.replace(
        "      RUSTDOC=\"$rustdoc_path\" \\\n      \"$cargo_path\" \"$@\"",
        "      RUSTDOC=\"$rustdoc_path\" \\\n      RUSTDOCFLAGS=--document-private-items \\\n      \"$cargo_path\" \"$@\"",
    );
    assert_ne!(script, forced, "cargo doc invocation changed unexpectedly");
    fs::write(path, forced).expect("force private rustdoc output");
}

fn write_dependency(root: &Path, package: &str, library: &str) {
    let dependency = root.join(package);
    fs::create_dir_all(dependency.join("src")).expect("create dependency src");
    fs::write(
        dependency.join("Cargo.toml"),
        format!(
            "[package]\nname = \"{package}\"\nversion = \"1.0.0\"\nedition = \"2021\"\n\n[lib]\nname = \"{library}\"\n"
        ),
    )
    .expect("write dependency manifest");
    fs::write(dependency.join("src/lib.rs"), "pub struct SameName;\n")
        .expect("write dependency source");
}

#[test]
fn capture_rejects_an_unpinned_toolchain() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    copy_capture_script(root);
    let path = root.join("scripts/capture-public-api.sh");
    let script = fs::read_to_string(&path).expect("read copied capture script");
    let mismatched = script.replace(
        "required_rustc_release=1.95.0",
        "required_rustc_release=0.0.0",
    );
    assert_ne!(script, mismatched, "toolchain pin changed unexpectedly");
    fs::write(path, mismatched).expect("write mismatched capture script");

    let result = Command::new("bash")
        .arg("scripts/capture-public-api.sh")
        .current_dir(root)
        .output()
        .expect("run mismatched capture script");
    assert!(!result.status.success(), "mismatched toolchain must fail");
    assert!(String::from_utf8_lossy(&result.stderr)
        .contains("public API capture requires the pinned Phase 0 toolchain"));
}

#[test]
fn capture_refuses_a_stale_lockfile_without_mutating_it() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"
"#,
    )
    .expect("write initial manifest");
    fs::write(root.join("src/lib.rs"), "pub fn visible() {}\n").expect("write fixture lib");
    copy_capture_script(root);
    ensure_lockfile(root);
    let lock_before = fs::read(root.join("Cargo.lock")).expect("read initial lockfile");

    write_dependency(root, "new-dependency", "new_dependency");
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"

[dependencies]
new-dependency = { path = "new-dependency" }
"#,
    )
    .expect("make manifest newer than lockfile");

    let result = Command::new("bash")
        .arg("scripts/capture-public-api.sh")
        .arg("stale.txt")
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .env("CCWRAPPED_CAPTURE_REVISION", BASELINE_REVISION)
        .output()
        .expect("run capture with stale lockfile");
    assert!(!result.status.success(), "stale lockfile must fail");
    assert_eq!(
        lock_before,
        fs::read(root.join("Cargo.lock")).expect("read unchanged lockfile"),
        "capture mutated Cargo.lock"
    );
}

#[test]
fn capture_detects_reexports_and_all_supported_item_kinds() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"
"#,
    )
    .expect("write Cargo.toml");

    let source = r#"
pub mod nested {
    pub fn nested_function(value: u64) -> u64 { value }
}
pub use nested::*;

pub fn public_function(value: u32) -> u64 { u64::from(value) }
pub struct PublicStruct(pub u8);
impl PublicStruct {
    pub const LIMIT: u8 = 7;
    pub fn value(&self) -> u8 { self.0 }
}
pub struct LargeStruct {
    pub f01: u8, pub f02: u8, pub f03: u8, pub f04: u8,
    pub f05: u8, pub f06: u8, pub f07: u8, pub f08: u8,
    pub f09: u8, pub f10: u8, pub f11: u8, pub f12: u8,
    pub f13: u8, pub f14: u8, pub f15: u8, pub f16: u8,
}
pub enum PublicEnum { One }
pub trait PublicTrait { fn call(&self); }
impl PublicTrait for PublicStruct { fn call(&self) {} }
impl PublicTrait for &PublicStruct { fn call(&self) {} }
pub type PublicAlias = u16;
pub const PUBLIC_CONST: u8 = 1;
pub static PUBLIC_STATIC: u8 = 2;
#[repr(C)] pub union PublicUnion { pub byte: u8, pub word: u16 }
#[macro_export] macro_rules! public_macro { () => { 1 }; }
"#;
    fs::write(root.join("src/lib.rs"), source).expect("write fixture lib");
    copy_capture_script(root);

    let before = run_capture(root, "before.txt");
    for required in [
        "crate-version 9.8.7",
        "module ccwrapped::nested",
        "reexport ccwrapped :: pub use ccwrapped::nested::*;",
        "item fn ccwrapped::public_function",
        "item struct ccwrapped::PublicStruct",
        "item enum ccwrapped::PublicEnum",
        "item trait ccwrapped::PublicTrait",
        "item type ccwrapped::PublicAlias",
        "item constant ccwrapped::PUBLIC_CONST",
        "item static ccwrapped::PUBLIC_STATIC",
        "item union ccwrapped::PublicUnion",
        "item macro ccwrapped::public_macro",
        "associated-constant ccwrapped::PublicStruct :: pub const LIMIT: u8 = 7",
        "trait ccwrapped::PublicStruct :: impl ccwrapped::PublicTrait for ccwrapped::PublicStruct",
        "trait ccwrapped::PublicStruct :: impl ccwrapped::PublicTrait for &ccwrapped::PublicStruct",
    ] {
        assert!(
            before.contains(required),
            "missing artifact row: {required}\n{before}"
        );
    }
    assert!(!before.contains("Show 16 fields"));

    let after_source = source
        .replace("pub use nested::*;", "")
        .replace(
            "pub fn public_function(value: u32) -> u64 { u64::from(value) }",
            "",
        )
        .replace(
            r#"impl PublicStruct {
    pub const LIMIT: u8 = 7;
    pub fn value(&self) -> u8 { self.0 }
}"#,
            "",
        );
    fs::write(root.join("src/lib.rs"), after_source).expect("remove re-export");
    let after = run_capture(root, "after.txt");
    assert_ne!(before, after, "re-export removal must change the artifact");
    assert!(!after.contains("reexport ccwrapped :: pub use ccwrapped::nested::*;"));
    assert!(!after.contains("item fn ccwrapped::public_function"));
    assert!(!after.contains("method ccwrapped::PublicStruct :: pub fn value"));
    assert!(!after.contains("associated-constant ccwrapped::PublicStruct"));
}

#[test]
fn capture_rejects_doc_only_external_input() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"
"#,
    )
    .expect("write Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        "#[cfg(doc)]\n#[path = \"../doc_only.rs\"]\npub mod doc_only;\npub fn visible() {}\n",
    )
    .expect("write doc-only module declaration");
    fs::write(
        root.join("doc_only.rs"),
        "pub fn visible_only_to_rustdoc() {}\n",
    )
    .expect("write doc-only external source");
    copy_capture_script(root);
    ensure_lockfile(root);

    let result = Command::new("bash")
        .arg("scripts/capture-public-api.sh")
        .arg("doc-only.txt")
        .current_dir(root)
        .env("CARGO_TERM_COLOR", "never")
        .env("CCWRAPPED_CAPTURE_REVISION", BASELINE_REVISION)
        .output()
        .expect("capture doc-only external input");
    assert!(
        !result.status.success(),
        "doc-only input escaped provenance"
    );
    assert!(
        String::from_utf8_lossy(&result.stderr)
            .contains("compiler dep-info contains input outside the captured closure"),
        "unexpected capture error: {}",
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn capture_detects_private_module_reexport_at_public_path() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"
"#,
    )
    .expect("write Cargo.toml");
    let source = r#"
mod hidden {
    pub struct X {
        pub value: u8,
    }

    impl X {
        pub fn value(&self) -> u8 { self.value }
    }

    pub struct Original;
}

pub use hidden::X;
pub use hidden::Original as PublicAlias;
"#;
    fs::write(root.join("src/lib.rs"), source).expect("write private re-export fixture");
    copy_capture_script(root);

    let artifact = run_capture(root, "private-reexport.txt");
    for required in [
        "item struct ccwrapped::X",
        "item struct ccwrapped::X :: pub struct X { pub value: u8, }",
        "method ccwrapped::X :: pub fn value(&self) -> u8",
        "item struct ccwrapped::PublicAlias",
    ] {
        assert!(
            artifact.contains(required),
            "missing public alias row: {required}\n{artifact}"
        );
    }
    for inaccessible in [
        "item struct ccwrapped::hidden::X",
        "method ccwrapped::hidden::X",
        "item struct ccwrapped::hidden::Original",
    ] {
        assert!(
            !artifact.contains(inaccessible),
            "private storage path leaked as public API: {inaccessible}\n{artifact}"
        );
    }

    fs::write(
        root.join("src/lib.rs"),
        source
            .replace("pub use hidden::X;", "")
            .replace("pub use hidden::Original as PublicAlias;", ""),
    )
    .expect("remove private-module re-exports");
    let after = run_capture(root, "private-reexport-removed.txt");
    assert!(!after.contains("item struct ccwrapped::X"));
    assert!(!after.contains("item struct ccwrapped::PublicAlias"));
}

#[test]
fn capture_distinguishes_same_named_external_types() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    write_dependency(root, "dep-a", "dep_a");
    write_dependency(root, "dep-b", "dep_b");
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"

[dependencies]
dep-a = { path = "dep-a" }
dep-b = { path = "dep-b" }
"#,
    )
    .expect("write Cargo.toml");
    copy_capture_script(root);

    fs::write(
        root.join("src/lib.rs"),
        "pub fn external() -> dep_a::SameName { dep_a::SameName }\n",
    )
    .expect("write dep-a API");
    let dep_a = run_capture(root, "dep-a.txt");
    assert!(
        dep_a.contains("-> dep_a::SameName"),
        "dep-a artifact:\n{dep_a}"
    );
    let dep_a_digest = header_value(&dep_a, "source-tree-sha256");
    fs::write(
        root.join("dep-a/src/lib.rs"),
        "pub struct SameName;\npub struct DependencyOnlyChange;\n",
    )
    .expect("mutate path dependency only");
    let dep_a_mutated = run_capture(root, "dep-a-mutated.txt");
    assert_ne!(
        dep_a_digest,
        header_value(&dep_a_mutated, "source-tree-sha256"),
        "path-dependency bytes must participate in product provenance"
    );

    fs::write(
        root.join("src/lib.rs"),
        "pub fn external() -> dep_b::SameName { dep_b::SameName }\n",
    )
    .expect("write dep-b API");
    let dep_b = run_capture(root, "dep-b.txt");
    assert!(
        dep_b.contains("-> dep_b::SameName"),
        "dep-b artifact:\n{dep_b}"
    );
    assert_ne!(
        dep_a, dep_b,
        "qualified dependency type must affect artifact"
    );
}

#[test]
fn capture_detects_external_dependency_reexport_signatures() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    write_dependency(root, "dep-a", "dep_a");
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"

[dependencies]
dep-a = { path = "dep-a" }
"#,
    )
    .expect("write Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        "pub use dep_a::ExternalType as PublicExternal;\n",
    )
    .expect("write external re-export");
    fs::write(
        root.join("dep-a/src/lib.rs"),
        "pub struct ExternalType { pub value: u8 }\nimpl ExternalType { pub fn value(&self) -> u8 { self.value } }\n",
    )
    .expect("write dependency API");
    copy_capture_script(root);

    let before = run_capture(root, "external-reexport-before.txt");
    for required in [
        "item struct ccwrapped::PublicExternal :: pub struct PublicExternal { pub value: u8, }",
        "method ccwrapped::PublicExternal :: pub fn value(&self) -> u8",
    ] {
        assert!(
            before.contains(required),
            "missing external re-export row: {required}\n{before}"
        );
    }

    fs::write(
        root.join("dep-a/src/lib.rs"),
        "pub struct ExternalType { pub value: u16 }\nimpl ExternalType { pub fn value(&self) -> u16 { self.value } }\n",
    )
    .expect("change dependency API");
    let after = run_capture(root, "external-reexport-after.txt");
    assert_ne!(before, after, "dependency API change must alter artifact");
    assert!(after.contains(
        "item struct ccwrapped::PublicExternal :: pub struct PublicExternal { pub value: u16, }"
    ));
    assert!(after.contains("method ccwrapped::PublicExternal :: pub fn value(&self) -> u16"));
}

#[test]
fn capture_ignores_ambient_toolchain_overrides_flags_and_private_items() {
    let scratch = ScratchDir::new();
    let root = scratch.path();
    fs::write(
        root.join("Cargo.toml"),
        r#"[package]
name = "claude-code-wrapped"
version = "9.8.7"
edition = "2021"

[lib]
name = "ccwrapped"
"#,
    )
    .expect("write Cargo.toml");
    fs::write(
        root.join("src/lib.rs"),
        r#"
pub fn visible() {}
fn private_function() {}
struct PrivateStruct;
macro_rules! private_macro { () => { 1 }; }
mod private_module {
    pub mod public_but_unreachable { pub fn nested_but_private() {} }
}
pub mod public_module {
    fn nested_private() {}
    pub fn nested_visible() {}
}
#[cfg(ambient_capture_cfg)]
pub fn ambient_only() {}
"#,
    )
    .expect("write private-item fixture");
    copy_capture_script(root);
    // Simulate an unknown future Cargo configuration channel bypassing all of
    // the script's flag neutralization. The public-page allowlist must still
    // exclude private and publicly declared-but-unreachable items.
    force_private_rustdoc_output(root);

    let ambient_cargo_home = root.join("ambient-cargo-home");
    fs::create_dir_all(&ambient_cargo_home).expect("create ambient Cargo home");
    fs::write(
        ambient_cargo_home.join("config.toml"),
        format!(
            "[build]\nrustflags = [\"--cfg\", \"ambient_capture_cfg\"]\n\n[target.{}]\nrunner = \"/ccwrapped/ambient/runner-must-not-run\"\n",
            rustc_host()
        ),
    )
    .expect("write ambient Cargo config");
    let ambient_cargo_home = ambient_cargo_home
        .to_str()
        .expect("ambient Cargo home path is UTF-8");

    let ordinary = run_capture(root, "ordinary.txt");
    let target_rustdocflags = format!(
        "CARGO_TARGET_{}_RUSTDOCFLAGS",
        rustc_host().to_uppercase().replace('-', "_")
    );
    let target_rustflags = format!(
        "CARGO_TARGET_{}_RUSTFLAGS",
        rustc_host().to_uppercase().replace('-', "_")
    );
    let captures = [
        run_capture_with_env(
            root,
            "ambient-rustdocflags.txt",
            &[("RUSTDOCFLAGS", "--document-private-items")],
        ),
        run_capture_with_env(
            root,
            "ambient-encoded-rustdocflags.txt",
            &[("CARGO_ENCODED_RUSTDOCFLAGS", "--document-private-items")],
        ),
        run_capture_with_env(
            root,
            "ambient-build-rustdocflags.txt",
            &[("CARGO_BUILD_RUSTDOCFLAGS", "--document-private-items")],
        ),
        run_capture_with_env(
            root,
            "ambient-target-rustdocflags.txt",
            &[(target_rustdocflags.as_str(), "--document-private-items")],
        ),
        run_capture_with_env(
            root,
            "ambient-build-rustflags.txt",
            &[("CARGO_BUILD_RUSTFLAGS", "--cfg ambient_capture_cfg")],
        ),
        run_capture_with_env(
            root,
            "ambient-target-rustflags.txt",
            &[(target_rustflags.as_str(), "--cfg ambient_capture_cfg")],
        ),
        run_capture_with_env(
            root,
            "ambient-toolchain-overrides.txt",
            &[
                ("RUSTC", "/ccwrapped/ambient/rustc-must-not-run"),
                ("RUSTDOC", "/ccwrapped/ambient/rustdoc-must-not-run"),
                (
                    "RUSTC_WRAPPER",
                    "/ccwrapped/ambient/rustc-wrapper-must-not-run",
                ),
                (
                    "RUSTC_WORKSPACE_WRAPPER",
                    "/ccwrapped/ambient/workspace-wrapper-must-not-run",
                ),
                (
                    "CARGO_BUILD_RUSTC",
                    "/ccwrapped/ambient/cargo-rustc-must-not-run",
                ),
                (
                    "CARGO_BUILD_RUSTDOC",
                    "/ccwrapped/ambient/cargo-rustdoc-must-not-run",
                ),
                (
                    "CARGO_BUILD_RUSTC_WRAPPER",
                    "/ccwrapped/ambient/cargo-wrapper-must-not-run",
                ),
                (
                    "CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER",
                    "/ccwrapped/ambient/cargo-workspace-wrapper-must-not-run",
                ),
            ],
        ),
        run_capture_with_env(
            root,
            "ambient-cargo-home.txt",
            &[("CARGO_HOME", ambient_cargo_home)],
        ),
    ];
    for ambient in captures {
        assert_eq!(
            ordinary, ambient,
            "ambient rustdoc flags must not alter API"
        );
        for private in [
            "private_function",
            "PrivateStruct",
            "private_macro",
            "private_module",
            "public_but_unreachable",
            "nested_but_private",
            "nested_private",
        ] {
            assert!(!ambient.contains(private), "private item leaked: {private}");
        }
        assert!(ambient.contains("module ccwrapped::public_module"));
        assert!(ambient.contains("item fn ccwrapped::public_module::nested_visible"));
        assert!(!ambient.contains("ambient_only"));
    }
}
