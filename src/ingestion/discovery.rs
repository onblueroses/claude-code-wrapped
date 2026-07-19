use super::types::{Diagnostics, FileIdentity, FileSnapshot, SourceStats};
use std::collections::HashSet;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

const MAX_SELECTED_SOURCES: usize = 256;

#[derive(Debug, Clone)]
pub(super) struct DiscoveryOptions {
    pub data_dirs: Vec<PathBuf>,
    pub otel_files: Vec<PathBuf>,
    pub claude_config_dir: Option<PathBuf>,
    pub home_dir: Option<PathBuf>,
    pub private_diagnostics: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SourceKind {
    Transcript,
    Otel,
}

#[derive(Debug, Clone)]
pub(super) struct Source {
    pub alias: String,
    pub kind: SourceKind,
    pub path: PathBuf,
    pub discovery_snapshot: FileSnapshot,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum OtelDedupKey {
    FileSystem(FileIdentity),
    CanonicalPath(PathBuf),
}

#[derive(Debug)]
pub(super) struct Discovery {
    pub sources: Vec<Source>,
    pub diagnostics: Diagnostics,
}

#[derive(Debug, Clone)]
pub(super) struct DiscoveryError {
    pub code: &'static str,
    pub source_alias: Option<String>,
    message: String,
    pub remediation: &'static str,
}

impl DiscoveryError {
    fn new(
        code: &'static str,
        source_alias: Option<String>,
        message: impl Into<String>,
        remediation: &'static str,
    ) -> Self {
        Self {
            code,
            source_alias,
            message: message.into(),
            remediation,
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DiscoveryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for DiscoveryError {}

pub(super) fn discover(options: &DiscoveryOptions) -> Result<Discovery, DiscoveryError> {
    discover_with_implicit_hook(options, |_| {})
}

fn discover_with_implicit_hook(
    options: &DiscoveryOptions,
    mut before_implicit_add: impl FnMut(&Path),
) -> Result<Discovery, DiscoveryError> {
    let implicit_transcript_slot = usize::from(options.data_dirs.is_empty());
    let requested_sources = options
        .data_dirs
        .len()
        .checked_add(options.otel_files.len())
        .and_then(|count| count.checked_add(implicit_transcript_slot));
    if requested_sources.is_none_or(|count| count > MAX_SELECTED_SOURCES) {
        return Err(DiscoveryError::new(
            "E_DISCOVERY_SOURCE_LIMIT",
            None,
            format!(
                "the invocation selected more than {MAX_SELECTED_SOURCES} local source inputs"
            ),
            "Select at most 256 transcript and telemetry inputs per invocation; split larger source sets into separate runs.",
        ));
    }

    let mut diagnostics = Diagnostics::default();
    let mut sources = Vec::new();
    let mut seen_transcript = HashSet::new();
    let mut seen_otel = HashSet::new();
    let mut implicit_errors = Vec::new();

    if options.data_dirs.is_empty() {
        let mut selected_implicit = false;
        if let Some(config_dir) = &options.claude_config_dir {
            let projects = config_dir.join("projects");
            match fs::metadata(&projects) {
                Ok(metadata) if metadata.is_dir() => {
                    selected_implicit = true;
                    before_implicit_add(&projects);
                    if let Err(error) = add_transcript(
                        &projects,
                        false,
                        Some("claude-config-env"),
                        options,
                        &mut sources,
                        &mut diagnostics,
                        &mut seen_transcript,
                    ) {
                        record_implicit_failure(
                            &mut diagnostics,
                            &error,
                            "claude-config-env",
                        );
                        implicit_errors.push(error);
                    }
                }
                Ok(_) => diagnostics.warning(
                    "W_DISCOVERY_CONFIG_DIR_MISSING",
                    "CLAUDE_CONFIG_DIR does not contain a readable projects directory; trying the supported home default.",
                    None,
                ),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    diagnostics.warning(
                        "W_DISCOVERY_CONFIG_DIR_MISSING",
                        "CLAUDE_CONFIG_DIR does not contain a readable projects directory; trying the supported home default.",
                        None,
                    );
                }
                Err(error) => {
                    selected_implicit = true;
                    let error = implicit_metadata_error(&diagnostics, error);
                    record_implicit_failure(&mut diagnostics, &error, "claude-config-env");
                    implicit_errors.push(error);
                }
            }
        }

        if !selected_implicit {
            if let Some(home) = &options.home_dir {
                let projects = home.join(".claude").join("projects");
                match fs::metadata(&projects) {
                    Ok(metadata) if metadata.is_dir() => {
                        before_implicit_add(&projects);
                        if let Err(error) = add_transcript(
                            &projects,
                            false,
                            Some("home-default"),
                            options,
                            &mut sources,
                            &mut diagnostics,
                            &mut seen_transcript,
                        ) {
                            record_implicit_failure(&mut diagnostics, &error, "home-default");
                            implicit_errors.push(error);
                        }
                    }
                    Ok(_) => diagnostics.warning(
                        "W_DISCOVERY_DEFAULT_MISSING",
                        "The supported home projects directory is unavailable.",
                        None,
                    ),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                        diagnostics.warning(
                            "W_DISCOVERY_DEFAULT_MISSING",
                            "The supported home projects directory is unavailable.",
                            None,
                        );
                    }
                    Err(error) => {
                        let error = implicit_metadata_error(&diagnostics, error);
                        record_implicit_failure(&mut diagnostics, &error, "home-default");
                        implicit_errors.push(error);
                    }
                }
            } else {
                diagnostics.warning(
                    "W_DISCOVERY_HOME_UNAVAILABLE",
                    "The home directory could not be resolved for default discovery.",
                    None,
                );
            }
        }
    } else {
        for path in &options.data_dirs {
            add_transcript(
                path,
                true,
                None,
                options,
                &mut sources,
                &mut diagnostics,
                &mut seen_transcript,
            )?;
        }
    }

    for path in &options.otel_files {
        if !path.exists() {
            let index = seen_otel.len().saturating_add(1);
            return Err(DiscoveryError::new(
                "E_DISCOVERY_OTEL_MISSING",
                Some(format!("otel-{index}")),
                format!(
                    "explicit telemetry source {index} does not exist{}",
                    private_suffix(path, options.private_diagnostics)
                ),
                "Select an existing regular file with --otel-file.",
            ));
        }
        if !path.is_file() {
            let index = seen_otel.len().saturating_add(1);
            return Err(DiscoveryError::new(
                "E_DISCOVERY_OTEL_NOT_FILE",
                Some(format!("otel-{index}")),
                format!(
                    "explicit telemetry source {index} is not a regular file{}",
                    private_suffix(path, options.private_diagnostics)
                ),
                "Select a readable uncompressed Collector JSON/JSONL file with --otel-file.",
            ));
        }
        let canonical = fs::canonicalize(path).map_err(|error| {
            let index = seen_otel.len().saturating_add(1);
            DiscoveryError::new(
                "E_DISCOVERY_OTEL_CANONICALIZE",
                Some(format!("otel-{index}")),
                format!("could not resolve explicit telemetry source {index}: {error}"),
                "Check file permissions and retry --otel-file against a stable local file.",
            )
        })?;
        let metadata = fs::metadata(&canonical).map_err(|error| {
            let index = seen_otel.len().saturating_add(1);
            DiscoveryError::new(
                "E_DISCOVERY_OTEL_METADATA",
                Some(format!("otel-{index}")),
                format!("could not inspect explicit telemetry source {index}: {error}"),
                "Check file permissions and retry --otel-file against a stable local file.",
            )
        })?;
        if !metadata.is_file() {
            let index = seen_otel.len().saturating_add(1);
            return Err(DiscoveryError::new(
                "E_DISCOVERY_OTEL_CHANGED",
                Some(format!("otel-{index}")),
                format!("explicit telemetry source {index} changed during discovery"),
                "Retry --otel-file against a stable regular file snapshot.",
            ));
        }
        let discovery_snapshot =
            FileSnapshot::capture_path(&metadata, &canonical).map_err(|error| {
                let index = seen_otel.len().saturating_add(1);
                DiscoveryError::new(
                    "E_DISCOVERY_OTEL_METADATA",
                    Some(format!("otel-{index}")),
                    format!("could not capture telemetry source identity {index}: {error}"),
                    "Check file permissions and retry --otel-file against a stable local file.",
                )
            })?;
        let dedup_key = discovery_snapshot.identity().map_or_else(
            || OtelDedupKey::CanonicalPath(canonical.clone()),
            OtelDedupKey::FileSystem,
        );
        if !seen_otel.insert(dedup_key) {
            diagnostics.warning(
                "W_DISCOVERY_DUPLICATE_OTEL",
                "A duplicate filesystem telemetry file was selected and will be imported once.",
                None,
            );
            continue;
        }
        let index = seen_otel.len();
        let alias = format!("otel-{index}");
        diagnostics
            .sources
            .insert(alias.clone(), SourceStats::otel(alias.clone()));
        sources.push(Source {
            alias,
            kind: SourceKind::Otel,
            path: canonical,
            discovery_snapshot,
        });
    }

    if sources.is_empty() {
        if let Some(error) = implicit_errors.into_iter().next() {
            return Err(error);
        }
    }

    diagnostics.source_root_count = diagnostics.sources.len();
    Ok(Discovery {
        sources,
        diagnostics,
    })
}

fn add_transcript(
    selected: &Path,
    explicit: bool,
    implicit_selection: Option<&str>,
    options: &DiscoveryOptions,
    sources: &mut Vec<Source>,
    diagnostics: &mut Diagnostics,
    seen: &mut HashSet<PathBuf>,
) -> Result<(), DiscoveryError> {
    if !selected.exists() {
        if explicit {
            let index = next_transcript_index(diagnostics);
            return Err(DiscoveryError::new(
                "E_DISCOVERY_TRANSCRIPT_MISSING",
                Some(format!("transcript-{index}")),
                format!(
                    "explicit transcript source {index} does not exist{}",
                    private_suffix(selected, options.private_diagnostics)
                ),
                "Select an existing Claude projects or configuration directory with --data-dir.",
            ));
        }
        let index = next_transcript_index(diagnostics);
        return Err(DiscoveryError::new(
            "E_DISCOVERY_TRANSCRIPT_CHANGED",
            Some(format!("transcript-{index}")),
            format!("implicit transcript source {index} changed during discovery"),
            "Retry against stable local transcript and telemetry sources.",
        ));
    }

    let selected_is_projects = selected.file_name().is_some_and(|name| name == "projects");
    let (interpreted, selection) = if let Some(selection) = implicit_selection {
        (selected.to_path_buf(), selection.to_string())
    } else if selected_is_projects {
        (selected.to_path_buf(), "explicit-projects".to_string())
    } else if selected.join("projects").is_dir() {
        (selected.join("projects"), "explicit-config".to_string())
    } else {
        (selected.to_path_buf(), "explicit-projects".to_string())
    };
    if !interpreted.is_dir() {
        let index = next_transcript_index(diagnostics);
        return Err(DiscoveryError::new(
            "E_DISCOVERY_TRANSCRIPT_NOT_DIRECTORY",
            Some(format!("transcript-{index}")),
            format!(
                "transcript source {index} is not a projects or configuration directory{}",
                private_suffix(selected, options.private_diagnostics)
            ),
            "Select a Claude projects directory or a configuration directory containing projects/ with --data-dir.",
        ));
    }
    let canonical = fs::canonicalize(&interpreted).map_err(|error| {
        let index = next_transcript_index(diagnostics);
        DiscoveryError::new(
            "E_DISCOVERY_TRANSCRIPT_CANONICALIZE",
            Some(format!("transcript-{index}")),
            format!("could not resolve transcript source {index}: {error}"),
            "Check directory permissions and retry --data-dir against a stable local path.",
        )
    })?;
    let metadata = fs::metadata(&canonical).map_err(|error| {
        let index = next_transcript_index(diagnostics);
        DiscoveryError::new(
            "E_DISCOVERY_TRANSCRIPT_METADATA",
            Some(format!("transcript-{index}")),
            format!("could not inspect transcript source {index}: {error}"),
            "Check directory permissions and retry --data-dir against a stable local path.",
        )
    })?;
    if !metadata.is_dir() {
        let index = next_transcript_index(diagnostics);
        return Err(DiscoveryError::new(
            "E_DISCOVERY_TRANSCRIPT_CHANGED",
            Some(format!("transcript-{index}")),
            format!("transcript source {index} changed during discovery"),
            "Retry --data-dir against a stable projects directory snapshot.",
        ));
    }
    let discovery_snapshot =
        FileSnapshot::capture_path(&metadata, &canonical).map_err(|error| {
            let index = next_transcript_index(diagnostics);
            DiscoveryError::new(
                "E_DISCOVERY_TRANSCRIPT_METADATA",
                Some(format!("transcript-{index}")),
                format!("could not capture transcript source identity {index}: {error}"),
                "Check directory permissions and retry --data-dir against a stable local path.",
            )
        })?;
    if !seen.insert(canonical.clone()) {
        diagnostics.warning(
            "W_DISCOVERY_DUPLICATE_TRANSCRIPT",
            "A duplicate canonical transcript root was selected and will be scanned once.",
            None,
        );
        return Ok(());
    }

    let alias = format!("transcript-{}", next_transcript_index(diagnostics));
    diagnostics.sources.insert(
        alias.clone(),
        SourceStats::transcript(alias.clone(), selection),
    );
    sources.push(Source {
        alias,
        kind: SourceKind::Transcript,
        path: canonical,
        discovery_snapshot,
    });
    Ok(())
}

fn next_transcript_index(diagnostics: &Diagnostics) -> usize {
    diagnostics
        .sources
        .values()
        .filter(|source| source.kind == "transcript")
        .count()
        .saturating_add(1)
}

fn record_implicit_failure(diagnostics: &mut Diagnostics, error: &DiscoveryError, selection: &str) {
    let alias = error
        .source_alias
        .clone()
        .unwrap_or_else(|| format!("transcript-{}", next_transcript_index(diagnostics)));
    let mut source = SourceStats::transcript(alias.clone(), selection.to_string());
    source.partial = true;
    diagnostics.sources.insert(alias.clone(), source);
    diagnostics.warning(
        "W_DISCOVERY_IMPLICIT_UNREADABLE",
        "An implicit transcript root could not be resolved or inspected; discovery continued to other declared local sources.",
        Some(alias),
    );
}

fn implicit_metadata_error(diagnostics: &Diagnostics, error: std::io::Error) -> DiscoveryError {
    let index = next_transcript_index(diagnostics);
    DiscoveryError::new(
        "E_DISCOVERY_TRANSCRIPT_METADATA",
        Some(format!("transcript-{index}")),
        format!("could not inspect implicit transcript source {index}: {error}"),
        "Check the implicit transcript directory permissions or select another usable local source.",
    )
}

fn private_suffix(path: &Path, enabled: bool) -> String {
    if enabled {
        format!(": {}", path.display())
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::{discover_with_implicit_hook, DiscoveryOptions, SourceKind};
    use crate::ingestion::{
        otel::{self, MetricTracker, OtelOptions},
        types::{AliasRegistry, PrivacyHasher},
    };
    use serde_json::{json, Value};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIRECTORY: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory {
        root: std::path::PathBuf,
    }

    impl TestDirectory {
        fn new(label: &str) -> Self {
            let sequence = NEXT_TEST_DIRECTORY.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "ccwrapped-discovery-{label}-{}-{sequence}",
                std::process::id()
            ));
            fs::create_dir(&root).unwrap();
            Self { root }
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn string_attribute(key: &str, value: &str) -> Value {
        json!({"key": key, "value": {"stringValue": value}})
    }

    fn integer_attribute(key: &str, value: u64) -> Value {
        json!({"key": key, "value": {"intValue": value.to_string()}})
    }

    #[test]
    fn unreadable_implicit_transcripts_are_visible_when_explicit_otel_is_usable() {
        let directory = TestDirectory::new("implicit-failure");
        let config_dir = directory.root.join("config");
        let config_projects = config_dir.join("projects");
        let home_dir = directory.root.join("home");
        let home_projects = home_dir.join(".claude/projects");
        fs::create_dir_all(&config_projects).unwrap();
        fs::create_dir_all(&home_projects).unwrap();
        let otel = directory.root.join("collector.jsonl");
        let export = json!({
            "resourceLogs": [{
                "resource": {
                    "attributes": [string_attribute("service.name", "claude-code")]
                },
                "scopeLogs": [{
                    "scope": {"name": "com.anthropic.claude_code.events"},
                    "logRecords": [{
                        "timeUnixNano": "1767225600000000000",
                        "body": {},
                        "attributes": [
                            string_attribute("event.timestamp", "2026-01-01T00:00:00Z"),
                            string_attribute("session.id", "synthetic-session"),
                            string_attribute("request_id", "synthetic-request"),
                            string_attribute("model", "claude-sonnet-4-6"),
                            integer_attribute("input_tokens", 1),
                            integer_attribute("output_tokens", 2),
                            integer_attribute("cache_read_tokens", 3),
                            integer_attribute("cache_creation_tokens", 4)
                        ],
                        "eventName": "claude_code.api_request"
                    }]
                }]
            }]
        });
        fs::write(&otel, format!("{export}\n")).unwrap();

        let failed_path = config_projects.clone();
        let discovery = discover_with_implicit_hook(
            &DiscoveryOptions {
                data_dirs: Vec::new(),
                otel_files: vec![otel],
                claude_config_dir: Some(config_dir),
                home_dir: Some(home_dir),
                private_diagnostics: false,
            },
            move |path| {
                if path == failed_path {
                    fs::remove_dir(path).unwrap();
                    fs::write(path, "synthetic replacement").unwrap();
                }
            },
        )
        .expect("a usable explicit OTel source should survive an implicit-root failure");

        assert_eq!(discovery.sources.len(), 1);
        assert_eq!(discovery.sources[0].kind, SourceKind::Otel);
        assert_eq!(discovery.sources[0].alias, "otel-1");
        assert_eq!(discovery.diagnostics.source_root_count, 2);
        let failed = discovery
            .diagnostics
            .sources
            .get("transcript-1")
            .expect("failed implicit source has safe coverage");
        assert!(failed.partial);
        assert_eq!(failed.selection, "claude-config-env");
        assert!(!discovery.diagnostics.sources.contains_key("transcript-2"));
        let warning = discovery
            .diagnostics
            .warnings
            .iter()
            .find(|warning| warning.code == "W_DISCOVERY_IMPLICIT_UNREADABLE")
            .expect("failed implicit source has a warning");
        assert_eq!(warning.source_alias.as_deref(), Some("transcript-1"));
        assert!(!warning
            .message
            .contains(directory.root.to_string_lossy().as_ref()));

        let mut diagnostics = discovery.diagnostics;
        let mut aliases = AliasRegistry::default();
        let mut private_prompts = Vec::new();
        let mut metric_tracker = MetricTracker::default();
        let (events, _) = otel::ingest(
            &discovery.sources[0],
            &OtelOptions {
                time_context: super::super::TimeContext::new("UTC", Some(2026)).unwrap(),
                maximum_line_bytes: 16 * 1024 * 1024,
                maximum_events: 16,
                read_accounting: std::sync::Arc::new(super::super::SourceReadAccounting::default()),
            },
            &mut diagnostics,
            &PrivacyHasher::new(),
            &mut aliases,
            &mut private_prompts,
            &mut metric_tracker,
        )
        .expect("the surviving explicit telemetry source should ingest normally");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].tokens.output, Some(2));
        assert_eq!(diagnostics.files_discovered, 1);
        assert!(!diagnostics.sources["otel-1"].partial);
        assert!(diagnostics.sources["transcript-1"].partial);
        assert!(diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "W_DISCOVERY_IMPLICIT_UNREADABLE"));
    }

    #[test]
    fn unreadable_implicit_transcript_is_fatal_without_a_usable_source() {
        let directory = TestDirectory::new("implicit-failure-only");
        let config_dir = directory.root.join("config");
        let config_projects = config_dir.join("projects");
        let home_dir = directory.root.join("home");
        fs::create_dir_all(&config_projects).unwrap();
        fs::create_dir_all(home_dir.join(".claude/projects")).unwrap();

        let failed_path = config_projects.clone();
        let error = discover_with_implicit_hook(
            &DiscoveryOptions {
                data_dirs: Vec::new(),
                otel_files: Vec::new(),
                claude_config_dir: Some(config_dir),
                home_dir: Some(home_dir),
                private_diagnostics: false,
            },
            move |path| {
                if path == failed_path {
                    fs::remove_dir(path).unwrap();
                    fs::write(path, "synthetic replacement").unwrap();
                }
            },
        )
        .expect_err("an unreadable implicit source is fatal when nothing usable survives");

        assert_eq!(error.code, "E_DISCOVERY_TRANSCRIPT_NOT_DIRECTORY");
        assert_eq!(error.source_alias.as_deref(), Some("transcript-1"));
        assert!(!error
            .message()
            .contains(directory.root.to_string_lossy().as_ref()));
    }
}
