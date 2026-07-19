pub mod discovery;
pub mod jsonl;
pub mod session;
pub mod wire;

use crate::ingestion::{self, IngestionOptions, IngestionResult};
use crate::DataCoverage;
use std::fmt;
use std::path::{Path, PathBuf};

/// A privacy-safe failure returned by the bounded compatibility readers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestionReadError {
    code: String,
    source_alias: Option<String>,
    message: String,
    remediation: String,
}

impl IngestionReadError {
    pub fn code(&self) -> &str {
        &self.code
    }

    pub fn source_alias(&self) -> Option<&str> {
        self.source_alias.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn remediation(&self) -> &str {
        &self.remediation
    }
}

impl fmt::Display for IngestionReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "[{}] {}", self.code, self.message)
    }
}

impl std::error::Error for IngestionReadError {}

fn compatibility_error(error: ingestion::IngestionError) -> IngestionReadError {
    IngestionReadError {
        code: error.code().to_string(),
        source_alias: error.source_alias().map(str::to_string),
        message: error.message().to_string(),
        remediation: error.remediation().to_string(),
    }
}

fn compatibility_time_error(error: ingestion::TimeContextError) -> IngestionReadError {
    IngestionReadError {
        code: error.code().to_string(),
        source_alias: None,
        message: error.to_string(),
        remediation: error.remediation().to_string(),
    }
}

fn compatibility_ingest(
    projects_dir: &Path,
    year: Option<i32>,
) -> Result<IngestionResult, IngestionReadError> {
    let time_context =
        ingestion::TimeContext::new("UTC", year).map_err(compatibility_time_error)?;
    ingestion::ingest(IngestionOptions {
        time_context,
        active_threshold_seconds: 300,
        timezone_fallback: false,
        data_dirs: vec![PathBuf::from(projects_dir)],
        otel_files: Vec::new(),
        claude_config_dir: None,
        home_dir: None,
        include_private_content: false,
        private_diagnostics: false,
        worker_count: None,
        worker_delay_seed: None,
        worker_panic_file: None,
        store_path: None,
        store_salt: None,
    })
    .map_err(compatibility_error)
}

fn compatibility_discover(
    projects_dir: &Path,
    scope: ingestion::CompatibilityPathScope,
) -> Result<Vec<PathBuf>, IngestionReadError> {
    ingestion::discover_transcript_paths(projects_dir, scope).map_err(compatibility_error)
}

fn emit_compatibility_coverage(coverage: &DataCoverage, reader: &str) {
    let excluded = coverage
        .malformed_records
        .saturating_add(coverage.unsupported_records)
        .saturating_add(coverage.skipped_records);
    let usage_is_limited = coverage
        .capabilities
        .get("analysis_usage_totals")
        .is_some_and(|status| status != "available");
    if excluded > 0 || !coverage.warnings.is_empty() || usage_is_limited {
        eprintln!(
            "warning [W_COMPATIBILITY_INGESTION_PARTIAL]: {reader} returned limited observed evidence and excluded {excluded} record(s); use its try_ variant to inspect DataCoverage"
        );
    }
}

fn emit_compatibility_error(error: &IngestionReadError, reader: &str) {
    eprintln!(
        "error [{}]: {reader} could not process {}; remediation: {}",
        error.code(),
        error.source_alias().unwrap_or("selected source"),
        error.remediation()
    );
}
