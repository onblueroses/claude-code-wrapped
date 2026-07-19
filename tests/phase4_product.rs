use ccwrapped::renderers::html::render_html;
use ccwrapped::renderers::markdown::render_markdown;
use ccwrapped::renderers::share_card::render_share_card;
use ccwrapped::renderers::terminal::widgets::{label_value, pad, section_header};
use ccwrapped::renderers::terminal::{render_terminal_to, try_render_terminal_to};
use ccwrapped::Report;
use serde_json::Value;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use termcolor::{ColorSpec, WriteColor};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestWorkspace {
    root: PathBuf,
}

impl TestWorkspace {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock must be after Unix epoch")
            .as_nanos();
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-phase4-{label}-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create Phase 4 test workspace");
        Self { root }
    }

    fn output_dir(&self, label: &str) -> PathBuf {
        let output = self.root.join(label);
        fs::create_dir(&output).expect("create isolated output directory");
        output
    }

    fn sensitive_transcript(&self, path_canary: &str, content: &str) -> PathBuf {
        let root = self.root.join(format!("projects-{path_canary}"));
        let project = root.join(format!("project-{path_canary}"));
        fs::create_dir_all(&project).expect("create synthetic sensitive transcript root");
        let records = [
            serde_json::json!({
                "type": "user",
                "uuid": format!("user-{path_canary}"),
                "sessionId": format!("session-{path_canary}"),
                "cwd": format!("/synthetic/{path_canary}"),
                "timestamp": "2026-04-05T09:00:00Z",
                "message": {"content": content},
                "entrypoint": content
            }),
            serde_json::json!({
                "type": "assistant",
                "sessionId": format!("session-{path_canary}"),
                "cwd": format!("/synthetic/{path_canary}"),
                "timestamp": "2026-04-05T09:01:00Z",
                "message": {
                    "id": format!("message-{path_canary}"),
                    "model": "claude-sonnet-4-6",
                    "usage": {
                        "input_tokens": 1,
                        "output_tokens": 2,
                        "cache_creation_input_tokens": 0,
                        "cache_read_input_tokens": 0
                    },
                    "content": [{
                        "type": "tool_use",
                        "name": "Read",
                        "input": {
                            "command": content,
                            "requestId": path_canary,
                            "account": content
                        }
                    }]
                }
            }),
        ];
        let contents = records
            .iter()
            .map(|record| serde_json::to_string(record).expect("serialize synthetic record"))
            .collect::<Vec<_>>()
            .join("\n");
        fs::write(project.join("session.jsonl"), format!("{contents}\n"))
            .expect("write synthetic sensitive transcript");
        root
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn install_browser_launcher(&self, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let bin = self.root.join("bin");
        fs::create_dir_all(&bin).expect("create fake browser bin directory");
        let name = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let launcher = bin.join(name);
        fs::write(&launcher, format!("#!/bin/sh\nset -eu\n{body}\n"))
            .expect("write fake browser launcher");
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700))
            .expect("make fake browser launcher executable");
        bin
    }
}

impl Drop for TestWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn fixture_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("readme-assets")
        .join("projects")
}

fn run_ccwrapped(output_dir: &Path, extra_args: &[&str]) -> Output {
    let workspace_home = output_dir.join("isolated-home");
    let workspace_claude = output_dir.join("isolated-claude");
    fs::create_dir(&workspace_home).expect("create isolated home");
    fs::create_dir(&workspace_claude).expect("create isolated Claude config");

    Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
        .args([
            "--timezone",
            "UTC",
            "--data-dir",
            fixture_root().to_str().expect("fixture path must be UTF-8"),
            "--plain",
        ])
        .args(extra_args)
        .arg("2026")
        .current_dir(output_dir)
        .env("XDG_CACHE_HOME", workspace_home.join("cache"))
        .env("HOME", workspace_home)
        .env("CLAUDE_CONFIG_DIR", workspace_claude)
        .env("NO_COLOR", "1")
        .output()
        .expect("run ccwrapped against checked synthetic fixture")
}

fn run_ccwrapped_with_data(output_dir: &Path, data_dir: &Path, extra_args: &[&str]) -> Output {
    let workspace_home = output_dir.join("isolated-home");
    let workspace_claude = output_dir.join("isolated-claude");
    fs::create_dir(&workspace_home).expect("create isolated home");
    fs::create_dir(&workspace_claude).expect("create isolated Claude config");

    Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
        .args(["--timezone", "UTC", "--data-dir"])
        .arg(data_dir)
        .arg("--plain")
        .args(extra_args)
        .arg("2026")
        .current_dir(output_dir)
        .env("XDG_CACHE_HOME", workspace_home.join("cache"))
        .env("HOME", workspace_home)
        .env("CLAUDE_CONFIG_DIR", workspace_claude)
        .env("NO_COLOR", "1")
        .output()
        .expect("run ccwrapped against synthetic Phase 4 fixture")
}

fn visible_output_entries(output_dir: &Path) -> Vec<String> {
    let mut entries = fs::read_dir(output_dir)
        .expect("read output directory")
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| {
            name != "isolated-home"
                && name != "isolated-claude"
                && name != ".ccwrapped-output.lock.sqlite3"
        })
        .collect::<Vec<_>>();
    entries.sort();
    entries
}

fn read_standard_surfaces(output_dir: &Path, output: &Output) -> String {
    let mut combined = String::from_utf8_lossy(&output.stdout).into_owned();
    combined.push_str(&String::from_utf8_lossy(&output.stderr));
    fn append_tree(path: &Path, combined: &mut String) {
        for entry in fs::read_dir(path).expect("read runtime output tree") {
            let entry = entry.expect("read runtime output entry");
            let file_type = entry.file_type().expect("read runtime output entry type");
            if file_type.is_dir() {
                append_tree(&entry.path(), combined);
            } else if file_type.is_file() {
                combined.push_str(&String::from_utf8_lossy(
                    &fs::read(entry.path()).expect("read runtime output file"),
                ));
            }
        }
    }
    append_tree(output_dir, &mut combined);
    combined
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
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = value.as_bytes();
    let mut encoded = String::new();
    for chunk in bytes.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (first << 16) | (second << 8) | third;
        encoded.push(ALPHABET[((value >> 18) & 0x3f) as usize] as char);
        encoded.push(ALPHABET[((value >> 12) & 0x3f) as usize] as char);
        encoded.push(if chunk.len() > 1 {
            ALPHABET[((value >> 6) & 0x3f) as usize] as char
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            ALPHABET[(value & 0x3f) as usize] as char
        } else {
            '='
        });
    }
    encoded
}

fn partial_report() -> Report {
    let mut report = Report {
        schema_version: "ccwrapped.report/v2".to_string(),
        generated_at: "2026-04-27T16:17:00Z".to_string(),
        year: 2026,
        ..Default::default()
    };
    report.data_coverage.selected_period = "2026".to_string();
    report.data_coverage.timezone = "UTC".to_string();
    report.data_coverage.completeness = "indeterminate".to_string();
    report.data_coverage.cost_coverage = "local-computation-with-unpriced-possibility".to_string();
    report.data_coverage.privacy_profile = "standard".to_string();
    report.data_coverage.retention_caveat =
        "Observed local history may omit earlier activity.".to_string();
    report.canonical_metrics.cost.local_api_equivalent.method_id =
        "cost/api-equivalent/v1".to_string();
    report
        .canonical_metrics
        .cost
        .local_api_equivalent
        .availability = "unavailable".to_string();
    report.methodology.pricing_registry.version = "registry-test-v1".to_string();
    report.wrapped_story.archetype.title = "Entertainment · Sample pending".to_string();
    report.wrapped_story.summary = "A bounded summary.".to_string();
    report
}

fn terminal_output(report: &Report) -> String {
    let mut output = termcolor::Buffer::no_color();
    render_terminal_to(report, &mut output);
    String::from_utf8(output.as_slice().to_vec()).expect("terminal output must be UTF-8")
}

struct BrokenTerminalWriter;

impl Write for BrokenTerminalWriter {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "synthetic terminal failure",
        ))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl WriteColor for BrokenTerminalWriter {
    fn supports_color(&self) -> bool {
        false
    }

    fn set_color(&mut self, _spec: &ColorSpec) -> io::Result<()> {
        Ok(())
    }

    fn reset(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn terminal_writer_failures_propagate() {
    let error = try_render_terminal_to(&partial_report(), &mut BrokenTerminalWriter)
        .expect_err("a broken terminal writer must surface an error");

    assert_eq!(error.kind(), io::ErrorKind::BrokenPipe);
}

fn display_width(value: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(value)
}

#[test]
fn c4_renderer_trust_facts_reconcile() {
    let report = partial_report();
    let terminal = terminal_output(&report);
    let html = render_html(&report);
    let markdown = render_markdown(&report);
    let share = render_share_card(&report);
    let json = serde_json::to_value(&report).expect("report must serialize");

    let standard_lines = [
        "Trust · profile=standard",
        "Trust · schema=ccwrapped.report/v2",
        "Trust · period=2026 · timezone=UTC",
        "Trust · completeness=indeterminate",
        "Trust · costProvenance=local API-equivalent estimate · costCoverage=local-computation-with-unpriced-possibility · method=cost/api-equivalent/v1 · registry=registry-test-v1",
        "Trust · limitations=Observed local history may omit earlier activity.",
    ];
    for line in standard_lines {
        assert!(terminal.contains(line), "terminal omitted {line}");
        assert!(html.contains(line), "HTML omitted {line}");
        assert!(markdown.contains(line), "Markdown omitted {line}");
    }
    assert!(share.contains("Trust · profile=share"));
    for line in standard_lines.into_iter().skip(1) {
        assert!(share.contains(line), "share card omitted {line}");
    }
    assert_eq!(json["schemaVersion"], "ccwrapped.report/v2");
    assert_eq!(json["dataCoverage"]["privacyProfile"], "standard");
    assert_eq!(json["dataCoverage"]["selectedPeriod"], "2026");
    assert_eq!(json["dataCoverage"]["timezone"], "UTC");
    assert_eq!(json["dataCoverage"]["completeness"], "indeterminate");
    assert_eq!(
        json["dataCoverage"]["costCoverage"],
        "local-computation-with-unpriced-possibility"
    );
}

#[test]
fn f049_partial_histories_open_as_observed_activity() {
    let report = partial_report();
    let partial_surfaces = [
        ("terminal", terminal_output(&report)),
        ("HTML", render_html(&report)),
        ("Markdown", render_markdown(&report)),
        ("share", render_share_card(&report)),
    ];
    for (surface, rendered) in &partial_surfaces {
        assert!(
            rendered.to_ascii_lowercase().contains("observed activity"),
            "{surface} did not qualify partial history"
        );
        assert!(
            !rendered.to_ascii_lowercase().contains("year in review"),
            "{surface} claimed account-complete history"
        );
    }

    let terminal = &partial_surfaces[0].1;
    assert!(
        terminal.find("Trust summary").unwrap() < terminal.find("Season stats").unwrap(),
        "terminal trust summary must precede detailed analytics"
    );
    let html = &partial_surfaces[1].1;
    assert!(
        html.find("trust-summary").unwrap() < html.find("hero-stats").unwrap(),
        "HTML trust summary must remain in the opening flow before hero analytics"
    );
    let markdown = &partial_surfaces[2].1;
    assert!(
        markdown.find("## Trust summary").unwrap() < markdown.find("## Season Summary").unwrap(),
        "Markdown trust summary must precede detailed analytics"
    );
    assert!(
        partial_surfaces[3].1.contains("Trust · profile=share"),
        "share card must carry the compact share profile"
    );

    let mut complete = partial_report();
    complete.data_coverage.completeness = "complete".to_string();
    complete.data_coverage.retention_caveat.clear();
    complete.wrapped_story.archetype.title = "Entertainment · The Specialist".to_string();
    complete.wrapped_story.total_messages = 20;
    for (surface, rendered) in [
        ("terminal", terminal_output(&complete)),
        ("HTML", render_html(&complete)),
        ("Markdown", render_markdown(&complete)),
        ("share", render_share_card(&complete)),
    ] {
        assert!(
            rendered
                .to_ascii_lowercase()
                .contains("claude code wrapped"),
            "{surface} lost the concise Wrapped treatment for complete history"
        );
        assert!(
            rendered.contains("2026"),
            "{surface} lost the selected year/period treatment for complete history"
        );
        assert!(
            !rendered.to_ascii_lowercase().contains("observed activity"),
            "{surface} qualified a complete history as partial"
        );
    }
}

#[test]
fn f046_share_projection_excludes_private_carriers() {
    const CANARY: &str = "PHASE4_SHARE_PRIVATE_CANARY_99E1";
    let mut report = partial_report();
    report.project_breakdown.push(ccwrapped::ProjectSummary {
        hash: CANARY.to_string(),
        path: Some(format!("/private/{CANARY}")),
        name: CANARY.to_string(),
        ..Default::default()
    });
    report
        .session_breakdown
        .sessions
        .push(ccwrapped::SessionSummary {
            session_id: CANARY.to_string(),
            project_hash: CANARY.to_string(),
            project_path: Some(format!("/private/{CANARY}")),
            project_name: CANARY.to_string(),
            first_prompt: Some(CANARY.to_string()),
            prompts: vec![ccwrapped::SessionPrompt {
                text: CANARY.to_string(),
                ..Default::default()
            }],
            ..Default::default()
        });

    let share = render_share_card(&report);
    assert!(!share.contains(CANARY));
    assert!(share.contains("Trust · profile=share"));
}

#[test]
fn f047_html_and_terminal_neutralize_controls_and_bidi() {
    let hostile = concat!(
        "safe",
        "\u{0000}\u{0007}\u{001b}]52;c;clipboard\u{0007}",
        "\u{009b}31m\u{007f}",
        "\u{061c}\u{200e}\u{200f}\u{202a}\u{202e}\u{2066}\u{2069}",
        "\u{2028}\u{2029}",
        "<script>alert(\"x\")</script>",
        "<img src=x onerror=alert(1)>",
        "\n# hostile-heading\n```html\n<div data-x='y'>active</div>\n```",
        "\n![image](javascript:alert(1)) [link](javascript:alert(2))",
        "\n\"quoted\" \\\\ {\"json\":[1,2]} e\u{301}界"
    );
    let escaped = ccwrapped::escape_html(hostile);
    for forbidden in [
        '\u{0000}', '\u{0007}', '\u{001b}', '\u{007f}', '\u{009b}', '\u{061c}', '\u{200e}',
        '\u{200f}', '\u{2028}', '\u{2029}', '\u{202a}', '\u{202e}', '\u{2066}', '\u{2069}',
    ] {
        assert!(!escaped.contains(forbidden));
    }
    assert!(!escaped.contains("<script>"));
    assert!(!escaped.contains("<img"));
    assert!(escaped.contains("&lt;script&gt;"));

    let mut report = partial_report();
    report.schema_version = hostile.to_string();
    report.data_coverage.selected_period = hostile.to_string();
    report.data_coverage.timezone = hostile.to_string();
    report.data_coverage.completeness = hostile.to_string();
    report.data_coverage.cost_coverage = hostile.to_string();
    report.wrapped_story.archetype.title = hostile.to_string();
    report.wrapped_story.summary = hostile.to_string();
    report.data_coverage.retention_caveat = hostile.to_string();
    report.canonical_metrics.cost.local_api_equivalent.method_id = hostile.to_string();
    report.methodology.pricing_registry.version = hostile.to_string();
    let terminal = terminal_output(&report);
    let html = render_html(&report);
    let markdown = render_markdown(&report);
    let share = render_share_card(&report);
    let json = serde_json::to_string(&report).expect("hostile report must serialize");
    let reparsed: Value = serde_json::from_str(&json).expect("hostile JSON must parse");
    assert_eq!(
        reparsed["dataCoverage"]["retentionCaveat"]
            .as_str()
            .unwrap(),
        hostile
    );
    assert_eq!(reparsed["schemaVersion"], hostile);
    assert_eq!(reparsed["dataCoverage"]["selectedPeriod"], hostile);
    assert_eq!(reparsed["dataCoverage"]["timezone"], hostile);
    assert_eq!(reparsed["dataCoverage"]["completeness"], hostile);
    assert_eq!(reparsed["dataCoverage"]["costCoverage"], hostile);
    for forbidden in [
        '\u{0000}', '\u{0007}', '\u{001b}', '\u{007f}', '\u{009b}', '\u{061c}', '\u{200e}',
        '\u{200f}', '\u{2028}', '\u{2029}', '\u{202a}', '\u{202e}', '\u{2066}', '\u{2069}',
    ] {
        assert!(!terminal.contains(forbidden));
        assert!(!html.contains(forbidden));
        assert!(!markdown.contains(forbidden));
        assert!(!share.contains(forbidden));
    }
    assert!(!html.contains("<script>"));
    assert!(!html.contains("<img"));
    assert!(!markdown.contains("<script>"));
    assert!(!markdown.contains("<img"));
    let mut in_fence = false;
    let markdown_outside_fences = markdown
        .lines()
        .filter(|line| {
            if line.starts_with("```") {
                in_fence = !in_fence;
                false
            } else {
                !in_fence
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!markdown_outside_fences.contains("![image]"));
    assert!(!markdown_outside_fences.contains("[link](javascript:"));
    assert!(!markdown_outside_fences.contains("# hostile-heading"));
    assert!(!share.contains("<script>"));
    assert!(!share.contains("<img"));
    assert!(html.contains("&lt;script&gt;"));
    assert!(markdown.contains("&lt;script&gt;"));
    for rendered in [&terminal, &html, &markdown, &share] {
        assert!(
            rendered.contains("e\u{301}界"),
            "safe combining and wide characters must survive rendering"
        );
    }

    report.data_coverage.retention_caveat = format!("{}終", "界".repeat(4_096));
    assert!(render_html(&report).len() < 200_000);
    assert!(render_markdown(&report).len() < 200_000);
    assert!(render_share_card(&report).len() < 200_000);
}

#[test]
fn c4_terminal_widgets_use_unicode_display_columns() {
    let aligned = label_value("猫", "e\u{301}", 8);
    assert_eq!(display_width(&aligned), 8);
    assert_eq!(aligned, "猫     e\u{301}");

    let header = section_header("猫", 12);
    assert_eq!(display_width(&header), 13);
    assert!(header.contains('猫'));

    assert_eq!(pad("猫a", 3), "猫a");
    assert_eq!(display_width(&pad("猫a", 3)), 3);
    assert_eq!(pad("猫a", 2), "猫");
    assert_eq!(display_width(&pad("猫a", 2)), 2);
    assert_eq!(pad("猫", 1), " ");
    assert_eq!(display_width(&pad("猫", 1)), 1);
}

#[test]
fn f043_standard_and_share_surfaces_exclude_sensitive_field_canaries() {
    const CANARY: &str = "PHASE4_STANDARD_PRIVATE_CANARY_43C2";
    let workspace = TestWorkspace::new("f043-standard-privacy");
    let source = workspace.sensitive_transcript(CANARY, CANARY);
    let output_dir = workspace.output_dir("output");
    let output = run_ccwrapped_with_data(&output_dir, &source, &["--all"]);
    assert!(
        output.status.success(),
        "F043 invocation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let surfaces = read_standard_surfaces(&output_dir, &output);
    for representation in [
        CANARY.to_string(),
        hex_encode(CANARY),
        percent_encode(CANARY),
        base64_encode(CANARY),
    ] {
        assert!(
            !surfaces.contains(&representation),
            "standard/share output leaked {representation}"
        );
    }
    assert!(String::from_utf8_lossy(&output.stdout).contains("Trust · profile=standard"));
    assert!(
        fs::read_to_string(output_dir.join("claude-code-wrapped.html"))
            .unwrap()
            .contains("Trust · profile=standard")
    );
    assert!(
        fs::read_to_string(output_dir.join("claude-code-wrapped.md"))
            .unwrap()
            .contains("Trust · profile=standard")
    );
    assert!(
        fs::read_to_string(output_dir.join("claude-code-wrapped-card.html"))
            .unwrap()
            .contains("Trust · profile=share")
    );
    assert!(!output_dir.join("wrapped-archive").exists());
}

#[test]
fn f044_encoded_sensitive_values_remain_absent_after_raw_and_decoded_scans() {
    const CANARY: &str = "PHASE4_ENCODED_PRIVATE_CANARY_44B7";
    let encoded = [
        CANARY.to_string(),
        CANARY.to_ascii_lowercase(),
        hex_encode(CANARY),
        percent_encode(CANARY),
        base64_encode(CANARY),
        format!("&#x{};", hex_encode(CANARY)),
        format!("\\u0050{}", &CANARY[1..]),
    ];
    let workspace = TestWorkspace::new("f044-encoded-privacy");
    let source =
        workspace.sensitive_transcript("PHASE4_SOURCE_PATH_CANARY_44D1", &encoded.join(" "));
    let output_dir = workspace.output_dir("output");
    let output = run_ccwrapped_with_data(&output_dir, &source, &["--all"]);
    assert!(output.status.success());
    let surfaces = read_standard_surfaces(&output_dir, &output);
    let decoded_scan = surfaces
        .replace("&amp;", "&")
        .replace("&#x50;", "P")
        .replace("%50", "P");

    for representation in encoded {
        assert!(!surfaces.contains(&representation));
        assert!(!decoded_scan.contains(&representation));
    }
    assert!(!decoded_scan.contains(CANARY));
}

#[test]
fn f045_private_profiles_require_explicit_opt_in_and_stay_isolated() {
    const PATH_CANARY: &str = "PHASE4_PRIVATE_PATH_CANARY_45A2";
    const CONTENT_CANARY: &str = "PHASE4_PRIVATE_CONTENT_CANARY_45E8";
    let workspace = TestWorkspace::new("f045-private-profiles");
    let source = workspace.sensitive_transcript(PATH_CANARY, CONTENT_CANARY);

    let standard_dir = workspace.output_dir("standard");
    let standard = run_ccwrapped_with_data(&standard_dir, &source, &["--html"]);
    assert!(standard.status.success());

    let diagnostics_dir = workspace.output_dir("diagnostics");
    let diagnostics = run_ccwrapped_with_data(
        &diagnostics_dir,
        &source,
        &["--html", "--private-diagnostics"],
    );
    assert!(diagnostics.status.success());
    assert_eq!(
        String::from_utf8_lossy(&standard.stdout)
            .split("  Wrote ")
            .next(),
        String::from_utf8_lossy(&diagnostics.stdout)
            .split("  Wrote ")
            .next()
    );
    assert_eq!(
        fs::read(standard_dir.join("claude-code-wrapped.html")).unwrap(),
        fs::read(diagnostics_dir.join("claude-code-wrapped.html")).unwrap()
    );
    let private_stderr = String::from_utf8_lossy(&diagnostics.stderr);
    assert!(private_stderr.contains("[privacy-profile: private]"));
    assert!(private_stderr.contains(PATH_CANARY));
    assert!(!private_stderr.contains(CONTENT_CANARY));
    assert!(!String::from_utf8_lossy(&diagnostics.stdout).contains(PATH_CANARY));

    let archive_dir = workspace.output_dir("archive");
    let archive_output = run_ccwrapped_with_data(&archive_dir, &source, &["--archive"]);
    assert!(archive_output.status.success());
    assert!(!String::from_utf8_lossy(&archive_output.stdout).contains(CONTENT_CANARY));
    assert!(!String::from_utf8_lossy(&archive_output.stderr).contains(CONTENT_CANARY));
    assert!(String::from_utf8_lossy(&archive_output.stderr)
        .contains("[privacy-profile: private-content]"));
    let archive = fs::read_to_string(archive_dir.join("wrapped-archive/project-1.md"))
        .expect("read private-content archive");
    assert!(archive.starts_with("<!-- privacy-profile: private-content -->"));
    assert!(archive.contains(CONTENT_CANARY));
    assert!(!archive.contains(PATH_CANARY));
    assert!(!archive_dir.join("claude-code-wrapped.html").exists());

    let json_standard_dir = workspace.output_dir("json-standard");
    let json_standard = run_ccwrapped_with_data(&json_standard_dir, &source, &["--json"]);
    let json_private_dir = workspace.output_dir("json-private");
    let json_private = run_ccwrapped_with_data(
        &json_private_dir,
        &source,
        &["--json", "--private-diagnostics"],
    );
    assert_eq!(json_standard.stdout, json_private.stdout);
    assert!(json_standard.stderr.is_empty());
    let json_private_stderr = String::from_utf8_lossy(&json_private.stderr);
    assert!(json_private_stderr.contains("[privacy-profile: private]"));
    assert!(json_private_stderr.contains(PATH_CANARY));
    assert!(!json_private_stderr.contains(CONTENT_CANARY));
}

#[test]
fn f048_json_success_and_failures_preserve_single_value_stdout_discipline() {
    let workspace = TestWorkspace::new("f048-json-discipline");

    let success_dir = workspace.output_dir("success");
    let success = run_ccwrapped(&success_dir, &["--json"]);
    assert!(success.status.success());
    assert!(success.stderr.is_empty());
    let success_json: Value = serde_json::from_slice(&success.stdout).unwrap();
    assert_eq!(success_json["dataCoverage"]["privacyProfile"], "standard");
    assert!(visible_output_entries(&success_dir).is_empty());

    let empty_source = workspace.root.join("empty-source");
    fs::create_dir(&empty_source).unwrap();
    let empty_dir = workspace.output_dir("empty");
    let empty = run_ccwrapped_with_data(&empty_dir, &empty_source, &["--json"]);
    assert_eq!(empty.status.code(), Some(1));
    assert!(empty.stderr.is_empty());
    let empty_json: Value = serde_json::from_slice(&empty.stdout).unwrap();
    assert_eq!(empty_json["code"], "E_NO_RECORDS");
    assert!(empty_json["remediation"]
        .as_str()
        .unwrap()
        .contains("--data-dir"));
    assert_eq!(empty_json["dataCoverage"]["privacyProfile"], "standard");
    assert!(visible_output_entries(&empty_dir).is_empty());

    let missing_dir = workspace.output_dir("missing");
    let missing = run_ccwrapped_with_data(
        &missing_dir,
        &workspace.root.join("missing-source"),
        &["--json"],
    );
    assert_eq!(missing.status.code(), Some(1));
    assert!(missing.stderr.is_empty());
    let missing_json: Value = serde_json::from_slice(&missing.stdout).unwrap();
    assert!(missing_json["code"]
        .as_str()
        .unwrap()
        .starts_with("E_DISCOVERY_"));
    assert!(visible_output_entries(&missing_dir).is_empty());

    let config_dir = workspace.output_dir("config");
    let config_home = config_dir.join("isolated-home");
    let config_claude = config_dir.join("isolated-claude");
    fs::create_dir(&config_home).unwrap();
    fs::create_dir(&config_claude).unwrap();
    let config = Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
        .args([
            "--json",
            "--timezone",
            "Invalid/Phase4",
            "--data-dir",
            fixture_root().to_str().unwrap(),
            "2026",
        ])
        .current_dir(&config_dir)
        .env("XDG_CACHE_HOME", config_home.join("cache"))
        .env("HOME", config_home)
        .env("CLAUDE_CONFIG_DIR", config_claude)
        .output()
        .unwrap();
    assert_eq!(config.status.code(), Some(1));
    assert!(config.stderr.is_empty());
    let config_json: Value = serde_json::from_slice(&config.stdout).unwrap();
    assert_eq!(config_json["code"], "E_TIMEZONE_INVALID");
    assert!(visible_output_entries(&config_dir).is_empty());
}

#[test]
fn f050_default_json_and_private_content_outputs_remain_isolated() {
    const CONTENT_CANARY: &str = "PHASE4_F050_PRIVATE_CONTENT_1B92";
    let workspace = TestWorkspace::new("f050-output-isolation");
    let source = workspace.sensitive_transcript("PHASE4_F050_PATH_02D1", CONTENT_CANARY);

    let default_dir = workspace.output_dir("default");
    let default = run_ccwrapped_with_data(&default_dir, &source, &[]);
    assert!(
        default.status.success(),
        "default output failed: {}",
        String::from_utf8_lossy(&default.stderr)
    );
    assert!(visible_output_entries(&default_dir).is_empty());

    let json_dir = workspace.output_dir("json");
    let json = run_ccwrapped_with_data(&json_dir, &source, &["--json"]);
    assert!(
        json.status.success(),
        "JSON output failed: {}",
        String::from_utf8_lossy(&json.stderr)
    );
    assert!(visible_output_entries(&json_dir).is_empty());

    let all_dir = workspace.output_dir("all");
    let all = run_ccwrapped_with_data(&all_dir, &source, &["--all"]);
    assert!(
        all.status.success(),
        "all-formats output failed: {}",
        String::from_utf8_lossy(&all.stderr)
    );
    assert_eq!(
        visible_output_entries(&all_dir),
        vec![
            "claude-code-wrapped-card.html".to_string(),
            "claude-code-wrapped.html".to_string(),
            "claude-code-wrapped.md".to_string(),
        ]
    );
    assert!(!read_standard_surfaces(&all_dir, &all).contains(CONTENT_CANARY));

    let archive_dir = workspace.output_dir("archive");
    let archive = run_ccwrapped_with_data(&archive_dir, &source, &["--archive"]);
    assert!(
        archive.status.success(),
        "private archive output failed: {}",
        String::from_utf8_lossy(&archive.stderr)
    );
    assert_eq!(
        visible_output_entries(&archive_dir),
        vec!["wrapped-archive".to_string()]
    );
    let private_file =
        fs::read_to_string(archive_dir.join("wrapped-archive/project-1.md")).unwrap();
    assert!(private_file.contains(CONTENT_CANARY));
    assert!(private_file.contains("privacy-profile: private-content"));
}

#[test]
fn c4_json_conflicts_with_file_and_open_flags() {
    let workspace = TestWorkspace::new("json-conflicts");
    for (index, flag) in [
        "--html",
        "--markdown",
        "--card",
        "--archive",
        "--all",
        "--open",
    ]
    .into_iter()
    .enumerate()
    {
        let output_dir = workspace.output_dir(&format!("case-{index}"));
        let output = run_ccwrapped(&output_dir, &["--json", flag]);
        assert_eq!(
            output.status.code(),
            Some(2),
            "{flag} conflict must use CLI exit 2; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            output.stderr.is_empty(),
            "{flag} conflict corrupted JSON mode stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value =
            serde_json::from_slice(&output.stdout).expect("conflict stdout must be one JSON value");
        assert_eq!(value["code"], "E_CLI_ARGUMENT_INVALID");
        assert_eq!(value["error"], "invalid configuration");
        assert_eq!(
            visible_output_entries(&output_dir),
            Vec::<String>::new(),
            "{flag} conflict performed a file side effect"
        );
    }
}

#[test]
fn f048_json_conflicts_with_file_and_open_flags() {
    c4_json_conflicts_with_file_and_open_flags();
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn c4_open_standalone_implies_standard_html_and_waits_for_success() {
    let workspace = TestWorkspace::new("open-standalone");
    let output_dir = workspace.output_dir("output");
    let browser_log = workspace.root.join("browser.log");
    let fake_bin = workspace.install_browser_launcher(
        r#"printf '%s\n' "$1" >> "$CCWRAPPED_BROWSER_LOG"
exit 0"#,
    );

    let mut command = Command::new(env!("CARGO_BIN_EXE_ccwrapped"));
    let output = command
        .args([
            "--timezone",
            "UTC",
            "--data-dir",
            fixture_root().to_str().expect("fixture path must be UTF-8"),
            "--plain",
            "--open",
            "2026",
        ])
        .current_dir(&output_dir)
        .env("HOME", workspace.root.join("home"))
        .env("XDG_CACHE_HOME", workspace.root.join("cache"))
        .env("CLAUDE_CONFIG_DIR", workspace.root.join("claude"))
        .env("NO_COLOR", "1")
        .env("PATH", fake_bin)
        .env("CCWRAPPED_BROWSER_LOG", &browser_log)
        .output()
        .expect("run standalone --open");

    assert!(
        output.status.success(),
        "standalone --open failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let html = output_dir.join("claude-code-wrapped.html");
    assert!(html.is_file(), "standalone --open did not imply HTML");
    let launches = fs::read_to_string(&browser_log).expect("launcher must record one invocation");
    assert_eq!(launches.lines().count(), 1);
    assert_eq!(Path::new(launches.trim()), html);
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn c4_open_selection_matrix_adds_html_only_when_needed() {
    type OpenSelectionCase = (
        &'static str,
        &'static [&'static str],
        &'static [&'static str],
        &'static [&'static str],
    );

    let workspace = TestWorkspace::new("open-selection");
    let fake_bin = workspace.install_browser_launcher(
        r#"printf '%s\n' "$1" >> "$CCWRAPPED_BROWSER_LOG"
exit 0"#,
    );
    let cases: &[OpenSelectionCase] = &[
        (
            "markdown",
            &["--markdown", "--open"],
            &["claude-code-wrapped.html", "claude-code-wrapped.md"],
            &["claude-code-wrapped.html"],
        ),
        (
            "archive",
            &["--archive", "--open"],
            &["claude-code-wrapped.html", "wrapped-archive"],
            &["claude-code-wrapped.html"],
        ),
        (
            "card",
            &["--card", "--open"],
            &["claude-code-wrapped-card.html"],
            &["claude-code-wrapped-card.html"],
        ),
        (
            "html",
            &["--html", "--open"],
            &["claude-code-wrapped.html"],
            &["claude-code-wrapped.html"],
        ),
        (
            "all",
            &["--all", "--open"],
            &[
                "claude-code-wrapped-card.html",
                "claude-code-wrapped.html",
                "claude-code-wrapped.md",
            ],
            &["claude-code-wrapped-card.html", "claude-code-wrapped.html"],
        ),
    ];

    for (label, flags, expected_outputs, expected_launches) in cases {
        let output_dir = workspace.output_dir(label);
        let browser_log = workspace.root.join(format!("{label}-browser.log"));
        let output = Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
            .args([
                "--timezone",
                "UTC",
                "--data-dir",
                fixture_root().to_str().expect("fixture path must be UTF-8"),
                "--plain",
            ])
            .args(*flags)
            .arg("2026")
            .current_dir(&output_dir)
            .env("HOME", workspace.root.join(format!("{label}-home")))
            .env(
                "XDG_CACHE_HOME",
                workspace.root.join(format!("{label}-cache")),
            )
            .env(
                "CLAUDE_CONFIG_DIR",
                workspace.root.join(format!("{label}-claude")),
            )
            .env("NO_COLOR", "1")
            .env("PATH", &fake_bin)
            .env("CCWRAPPED_BROWSER_LOG", &browser_log)
            .output()
            .expect("run --open selection case");
        assert!(
            output.status.success(),
            "{label} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(
            visible_output_entries(&output_dir),
            expected_outputs
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>()
        );
        let mut launches = fs::read_to_string(&browser_log)
            .expect("read browser selection log")
            .lines()
            .filter_map(|line| Path::new(line).file_name())
            .filter_map(|name| name.to_str())
            .map(str::to_string)
            .collect::<Vec<_>>();
        launches.sort();
        assert_eq!(
            launches,
            expected_launches
                .iter()
                .map(|name| (*name).to_string())
                .collect::<Vec<_>>()
        );
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn c4_nonzero_browser_launcher_status_returns_e_browser_open() {
    let workspace = TestWorkspace::new("open-nonzero");
    let output_dir = workspace.output_dir("output");
    let browser_log = workspace.root.join("browser.log");
    let fake_bin = workspace.install_browser_launcher(
        r#"printf '%s\n' "$1" >> "$CCWRAPPED_BROWSER_LOG"
exit 23"#,
    );

    let output = Command::new(env!("CARGO_BIN_EXE_ccwrapped"))
        .args([
            "--timezone",
            "UTC",
            "--data-dir",
            fixture_root().to_str().expect("fixture path must be UTF-8"),
            "--plain",
            "--html",
            "--open",
            "2026",
        ])
        .current_dir(&output_dir)
        .env("HOME", workspace.root.join("home"))
        .env("XDG_CACHE_HOME", workspace.root.join("cache"))
        .env("CLAUDE_CONFIG_DIR", workspace.root.join("claude"))
        .env("NO_COLOR", "1")
        .env("PATH", fake_bin)
        .env("CCWRAPPED_BROWSER_LOG", &browser_log)
        .output()
        .expect("run --open with non-zero launcher");

    assert_eq!(output.status.code(), Some(1));
    assert!(output_dir.join("claude-code-wrapped.html").is_file());
    assert!(browser_log.is_file(), "launcher was not invoked");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E_BROWSER_OPEN"), "{stderr}");
    assert!(stderr.contains("output files were committed"), "{stderr}");
}
