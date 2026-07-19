mod discovery;
mod insights;
mod line_reader;
mod otel;
pub(crate) mod pricing;
mod store;
mod time;
mod transcript;
mod types;
mod views;

use ccwrapped::{
    CanonicalMetrics, DailyAggregate, DataCoverage, IngestionWarning, InsightReport,
    MethodologyCatalog, ProjectSummary, Report, SessionBreakdown,
};
use discovery::{DiscoveryOptions, SourceKind};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Instant;
#[allow(unused_imports)]
// The binary consumes store inventory; the library compatibility copy does not.
pub(crate) use store::SourceFile as StoreSourceFile;
#[allow(unused_imports)]
pub(crate) use time::{TimeContext, TimeContextError};
pub(super) use transcript::CompatibilityPathScope;
pub(crate) use types::PrivatePrompt;
use types::{AliasRegistry, AliasState, DedupKey, NormalizedEvent, PrivacyHasher};
#[allow(unused_imports)]
pub(crate) use views::merge_metric_aggregates;
pub(crate) use views::AnalysisEntry;

const DEFAULT_MAXIMUM_LINE_BYTES: usize = 16 * 1024 * 1024;
const MAXIMUM_NORMALIZED_EVENTS: usize = 1_000_000;
const MAXIMUM_SOURCE_BYTES: u64 = 4 * 1024 * 1024 * 1024;
const MAXIMUM_PHYSICAL_RECORDS: usize = 16_000_000;
const REQUEST_CORRELATION_TOLERANCE_NANOS: u128 = 300_000_000_000;
const MAX_REQUEST_CORRELATION_GROUP_EVENTS: usize = 128;
const MAX_REQUEST_CORRELATION_WORK: u64 = 20_000_000;
const MAXIMUM_INGESTION_WORKERS: usize = 256;
const DEFAULT_INGESTION_WORKERS: usize = 12;
const PARALLEL_ALIAS_MINIMUM_EVENTS: usize = 4_096;

const SOURCE_WORK_LIMIT_CODE: &str = "E_SOURCE_WORK_LIMIT";

#[derive(Debug, Clone, Copy)]
enum SourceWorkLimit {
    Bytes,
    PhysicalRecords,
}

impl fmt::Display for SourceWorkLimit {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bytes => write!(
                formatter,
                "{SOURCE_WORK_LIMIT_CODE}: the invocation exceeded the source-byte safety limit"
            ),
            Self::PhysicalRecords => write!(
                formatter,
                "{SOURCE_WORK_LIMIT_CODE}: the invocation exceeded the physical-record safety limit"
            ),
        }
    }
}

impl std::error::Error for SourceWorkLimit {}

#[derive(Debug)]
struct SourceReadAccounting {
    content_bytes: AtomicU64,
    reserved_source_bytes: AtomicU64,
    physical_records: AtomicUsize,
    file_streams: AtomicUsize,
    maximum_source_bytes: u64,
    maximum_physical_records: usize,
}

impl Default for SourceReadAccounting {
    fn default() -> Self {
        Self {
            content_bytes: AtomicU64::new(0),
            reserved_source_bytes: AtomicU64::new(0),
            physical_records: AtomicUsize::new(0),
            file_streams: AtomicUsize::new(0),
            maximum_source_bytes: MAXIMUM_SOURCE_BYTES,
            maximum_physical_records: MAXIMUM_PHYSICAL_RECORDS,
        }
    }
}

impl SourceReadAccounting {
    fn start_stream(&self, maximum_stream_bytes: u64) -> io::Result<()> {
        self.reserved_source_bytes
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value
                    .checked_add(maximum_stream_bytes)
                    .filter(|next| *next <= self.maximum_source_bytes)
            })
            .map_err(|_| io::Error::other(SourceWorkLimit::Bytes))?;
        let _ = self
            .file_streams
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(1))
            });
        Ok(())
    }

    fn record_bytes(&self, bytes: usize) {
        let bytes = u64::try_from(bytes).unwrap_or(u64::MAX);
        let _ = self
            .content_bytes
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                Some(value.saturating_add(bytes))
            });
    }

    fn snapshot(&self) -> (u64, usize) {
        (
            self.content_bytes.load(Ordering::Relaxed),
            self.file_streams.load(Ordering::Relaxed),
        )
    }

    fn consume_physical_record(&self) -> io::Result<()> {
        self.physical_records
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |value| {
                value
                    .checked_add(1)
                    .filter(|next| *next <= self.maximum_physical_records)
            })
            .map(|_| ())
            .map_err(|_| io::Error::other(SourceWorkLimit::PhysicalRecords))
    }

    #[cfg(test)]
    fn with_limits(maximum_source_bytes: u64, maximum_physical_records: usize) -> Self {
        Self {
            maximum_source_bytes,
            maximum_physical_records,
            ..Self::default()
        }
    }
}

#[cfg(test)]
std::thread_local! {
    static SOURCE_CAPABILITY_EVENT_VISITS: Cell<usize> = const { Cell::new(0) };
}
const METRIC_CAPABILITIES: [(&str, &str); 8] = [
    ("metric_session_count", "session-count"),
    ("metric_lines_of_code", "lines-of-code"),
    ("metric_pull_requests", "pull-requests"),
    ("metric_commits", "commits"),
    ("metric_source_cost_estimate", "source-cost-estimate"),
    ("metric_token_usage", "token-usage"),
    ("metric_code_edit_decision", "code-edit-decision"),
    ("metric_active_time", "active-time"),
];

#[derive(Debug, Clone)]
pub(super) struct IngestionOptions {
    pub time_context: TimeContext,
    pub active_threshold_seconds: u64,
    pub timezone_fallback: bool,
    pub data_dirs: Vec<PathBuf>,
    pub otel_files: Vec<PathBuf>,
    pub claude_config_dir: Option<PathBuf>,
    pub home_dir: Option<PathBuf>,
    pub include_private_content: bool,
    pub private_diagnostics: bool,
    pub worker_count: Option<usize>,
    pub worker_delay_seed: Option<u64>,
    pub worker_panic_file: Option<usize>,
    pub store_path: Option<PathBuf>,
    pub store_salt: Option<[u8; 32]>,
}

#[derive(Debug)]
#[allow(dead_code)] // The library compatibility build does not consume binary-only report views.
pub(super) struct IngestionResult {
    pub entries: Vec<AnalysisEntry>,
    pub session_breakdown: SessionBreakdown,
    pub daily: Vec<DailyAggregate>,
    pub project_breakdown: Vec<ProjectSummary>,
    pub methodology: MethodologyCatalog,
    pub canonical_metrics: CanonicalMetrics,
    pub insights: InsightReport,
    pub hour_distribution: Vec<usize>,
    pub coverage: DataCoverage,
    #[allow(dead_code)]
    pub private_prompts: Vec<PrivatePrompt>,
    #[allow(dead_code)]
    pub private_source_paths: Vec<(String, PathBuf)>,
    pub store_files: Vec<store::SourceFile>,
    pub analysis_state: AnalysisState,
    pub encoded_analysis_state: Option<Vec<u8>>,
    pub invalidate_analysis_state: bool,
    pub store_publish_allowed: bool,
    pub fast_report_json: Option<Vec<u8>>,
    pub performance: IngestionPerformance,
}

#[derive(Debug, Serialize)]
pub(super) struct AnalysisState {
    canonical_events: Vec<NormalizedEvent>,
    diagnostics: types::Diagnostics,
    aliases: AliasState,
    alias_observations: Vec<AliasObservation>,
    observed_summary: insights::ObservedEventSummary,
    dedup_keys: Vec<DedupKey>,
    authority_keys: Vec<AppendAuthorityKey>,
    otel_request_groups: Vec<RequestCorrelationGroupKey>,
    aggregate_metrics: Vec<NormalizedEvent>,
}

struct PreparedAppend {
    canonical_events: Vec<NormalizedEvent>,
    diagnostics: types::Diagnostics,
    alias_observations: Vec<AliasObservation>,
    observed_summary: insights::ObservedEventSummary,
    dedup_keys: Vec<DedupKey>,
    authority_keys: Vec<AppendAuthorityKey>,
    otel_request_groups: Vec<RequestCorrelationGroupKey>,
    aggregate_metrics: Vec<NormalizedEvent>,
    store_files: Vec<store::SourceFile>,
    delta_records: usize,
}

struct FastPromptAppend {
    diagnostics: types::Diagnostics,
    aliases: AliasState,
    store_files: Vec<store::SourceFile>,
    prompt_sessions: Vec<String>,
    cached_report: Report,
    delta_records: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AliasObservation {
    transcript: bool,
    source_alias: String,
    file_alias: String,
    record_index: u64,
    project_key: u64,
    project_identity_present: bool,
    session_key: u64,
    parent_key: Option<u64>,
}

impl AliasObservation {
    fn from_event(event: &NormalizedEvent) -> Self {
        Self {
            transcript: event.adapter_version == types::TRANSCRIPT_ADAPTER,
            source_alias: event.source_alias.clone(),
            file_alias: event.file_alias.clone(),
            record_index: event.record_index,
            project_key: event.project_key,
            project_identity_present: event.project_identity_present,
            session_key: event.session_key,
            parent_key: event.parent_key,
        }
    }
}

struct AppendAttempt {
    prepared: Option<PreparedAppend>,
    fast_prompt: Option<FastPromptAppend>,
    full_transcript: Option<transcript::FullTranscript>,
    status: &'static str,
}

impl AppendAttempt {
    fn unavailable(status: &'static str) -> Self {
        Self {
            prepared: None,
            fast_prompt: None,
            full_transcript: None,
            status,
        }
    }

    fn used(prepared: PreparedAppend) -> Self {
        Self {
            prepared: Some(prepared),
            fast_prompt: None,
            full_transcript: None,
            status: "used",
        }
    }

    fn fast_prompt(prepared: FastPromptAppend) -> Self {
        Self {
            prepared: None,
            fast_prompt: Some(prepared),
            full_transcript: None,
            status: "used",
        }
    }

    fn full_fallback(status: &'static str, transcript: transcript::FullTranscript) -> Self {
        Self {
            prepared: None,
            fast_prompt: None,
            full_transcript: Some(transcript),
            status,
        }
    }
}

#[derive(Debug, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct IngestionPerformance {
    schema: &'static str,
    pub selected_workers: usize,
    transcript_workers: usize,
    batch_files: usize,
    result_queue_capacity: usize,
    store_load_nanos: u128,
    pub store_publish_nanos: u128,
    source_content_bytes_read: u64,
    parsed_source_files: usize,
    reused_source_files: usize,
    incremental_checkpoint_status: &'static str,
    discovery_nanos: u128,
    transcript_parse_nanos: u128,
    otel_parse_nanos: u128,
    metric_finalize_nanos: u128,
    source_dedup_nanos: u128,
    capability_aggregation_nanos: u128,
    authority_selection_nanos: u128,
    analytical_capability_nanos: u128,
    canonical_projection_nanos: u128,
    projection_activity_nanos: u128,
    projection_tokens_nanos: u128,
    projection_cost_nanos: u128,
    projection_cache_nanos: u128,
    projection_daily_nanos: u128,
    projection_projects_nanos: u128,
    projection_sessions_nanos: u128,
    projection_methodology_nanos: u128,
    projection_hour_distribution_nanos: u128,
    projection_compatibility_entries_nanos: u128,
    insight_build_nanos: u128,
    ingestion_total_nanos: u128,
    pub report_build_nanos: u128,
    pub report_serialization_nanos: u128,
    pub report_entry_projection_nanos: u128,
    pub report_cost_nanos: u128,
    pub report_cache_nanos: u128,
    pub report_session_nanos: u128,
    pub report_model_routing_nanos: u128,
    pub report_recommendation_nanos: u128,
    pub report_story_nanos: u128,
    candidate_records: usize,
    accepted_records: usize,
    canonical_records: usize,
}

#[allow(dead_code)] // The library compatibility copy does not consume binary cache counters.
impl IngestionPerformance {
    pub fn cached() -> Self {
        Self {
            schema: "ccwrapped.ingestion-performance/v1",
            incremental_checkpoint_status: "report-cache-hit",
            ..Self::default()
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct IngestionError {
    code: &'static str,
    source_alias: Option<String>,
    message: String,
    remediation: &'static str,
}

impl IngestionError {
    fn discovery(error: discovery::DiscoveryError) -> Self {
        let message = error.message().to_string();
        Self {
            code: error.code,
            source_alias: error.source_alias,
            message,
            remediation: error.remediation,
        }
    }

    fn source(
        code: &'static str,
        source_alias: String,
        error: impl fmt::Display,
        remediation: &'static str,
    ) -> Self {
        Self {
            code,
            source_alias: Some(source_alias),
            message: error.to_string(),
            remediation,
        }
    }

    fn time(error: TimeContextError) -> Self {
        Self {
            code: error.code(),
            source_alias: None,
            message: error.to_string(),
            remediation: error.remediation(),
        }
    }

    fn internal(code: &'static str, message: impl Into<String>, remediation: &'static str) -> Self {
        Self {
            code,
            source_alias: None,
            message: message.into(),
            remediation,
        }
    }

    pub fn code(&self) -> &str {
        self.code
    }

    pub fn source_alias(&self) -> Option<&str> {
        self.source_alias.as_deref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn remediation(&self) -> &str {
        self.remediation
    }
}

impl fmt::Display for IngestionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for IngestionError {}

fn transcript_ingestion_error(
    source_alias: String,
    error: transcript::TranscriptError,
) -> IngestionError {
    if error.is_source_work_limit() {
        IngestionError::source(
            SOURCE_WORK_LIMIT_CODE,
            source_alias,
            error,
            "Narrow the selected source set or period; every physical record and source byte shares one invocation-wide safety budget.",
        )
    } else {
        IngestionError::source(
            "E_TRANSCRIPT_INGESTION",
            source_alias,
            error,
            "Check the selected transcript source permissions and stability, then retry.",
        )
    }
}

fn otel_ingestion_error(error: otel::OtelBatchError) -> IngestionError {
    let code = if error.is_source_work_limit() {
        SOURCE_WORK_LIMIT_CODE
    } else {
        "E_OTEL_INGESTION"
    };
    let remediation = if error.is_source_work_limit() {
        "Narrow the selected source set or period; every physical record and source byte shares one invocation-wide safety budget."
    } else {
        "Check the selected Collector file permissions and stability, then retry."
    };
    IngestionError::source(code, error.source_alias.clone(), error, remediation)
}

#[allow(dead_code)] // The library compatibility copy does not open the binary store.
pub(super) fn lookup_cached_report(
    options: &IngestionOptions,
    path: &Path,
) -> Result<Option<Vec<u8>>, IngestionError> {
    if options.private_diagnostics {
        return Ok(None);
    }
    let discovery = discovery::discover(&DiscoveryOptions {
        data_dirs: options.data_dirs.clone(),
        otel_files: options.otel_files.clone(),
        claude_config_dir: options.claude_config_dir.clone(),
        home_dir: options.home_dir.clone(),
        private_diagnostics: false,
    })
    .map_err(IngestionError::discovery)?;
    let mut diagnostics = discovery.diagnostics.clone();
    let mut traversal_budget = transcript::TraversalBudget::default();
    let mut files = Vec::new();
    for source in &discovery.sources {
        match source.kind {
            SourceKind::Transcript => {
                let source_files = transcript::discover_store_files(
                    source,
                    &mut diagnostics,
                    &mut traversal_budget,
                );
                match source_files {
                    Ok(mut source_files) => files.append(&mut source_files),
                    Err(_) => {
                        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
                            stats.partial = true;
                        }
                        diagnostics.warning(
                            "W_TRANSCRIPT_SUBTREE_INACCESSIBLE",
                            "The selected transcript root could not be inventoried. A matching store may retain the last complete report without publishing deletions.",
                            Some(source.alias.clone()),
                        );
                    }
                }
            }
            SourceKind::Otel => files.push(store::SourceFile::metadata_only(
                source.path.clone(),
                source.path.clone(),
                source.alias.clone(),
                source.kind,
                source.discovery_snapshot.clone(),
            )),
        }
    }
    let partial_source_aliases = diagnostics
        .sources
        .values()
        .filter(|source| source.partial)
        .map(|source| source.alias.clone())
        .collect::<HashSet<_>>();
    if !partial_source_aliases.is_empty() {
        let selected_sources = discovery
            .sources
            .iter()
            .map(|source| (source.alias.clone(), source.kind, source.path.clone()))
            .collect::<Vec<_>>();
        return match store::lookup_retained_report(
            path,
            options,
            &selected_sources,
            &partial_source_aliases,
            &files,
        )
        .map_err(store_error)?
        {
            store::CacheLookup::Hit(report) => {
                patch_retained_report(report, &partial_source_aliases).map(Some)
            }
            store::CacheLookup::Miss => Ok(None),
        };
    }
    match store::lookup_report(path, options, &files).map_err(store_error)? {
        store::CacheLookup::Hit(report) => Ok(Some(report)),
        store::CacheLookup::Miss => Ok(None),
    }
}

fn patch_retained_report(
    report_json: Vec<u8>,
    partial_source_aliases: &HashSet<String>,
) -> Result<Vec<u8>, IngestionError> {
    let mut report: Report = serde_json::from_slice(&report_json).map_err(|error| {
        IngestionError::internal(
            "E_INCREMENTAL_STORE",
            format!("decode retained cached report: {error}"),
            "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
        )
    })?;
    report.data_coverage.completeness = if report.data_coverage.accepted_records == 0 {
        "indeterminate".to_string()
    } else {
        "partial".to_string()
    };
    for source in &mut report.data_coverage.sources {
        if partial_source_aliases.contains(&source.alias) {
            source.completeness = if source.accepted_records == 0 {
                "indeterminate".to_string()
            } else {
                "partial".to_string()
            };
        }
    }
    report.data_coverage.warnings.retain(|warning| {
        warning.code != "W_TRANSCRIPT_SUBTREE_INACCESSIBLE"
            || warning
                .source_alias
                .as_ref()
                .is_none_or(|alias| !partial_source_aliases.contains(alias))
    });
    for alias in partial_source_aliases {
        report.data_coverage.warnings.push(IngestionWarning {
            code: "W_TRANSCRIPT_SUBTREE_INACCESSIBLE".to_string(),
            message: "A transcript directory branch could not be read. This report retains last-known rows for that branch and does not treat absent files as deletions.".to_string(),
            source_alias: Some(alias.clone()),
        });
    }
    report.data_coverage.warnings.sort_by(|left, right| {
        left.source_alias
            .cmp(&right.source_alias)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    report.data_coverage.retention_caveat = "One or more transcript branches were inaccessible during this run. Displayed values retain last-known rows from the latest complete scan; coverage is partial until a complete scan confirms changes.".to_string();
    serde_json::to_vec_pretty(&report).map_err(|error| {
        IngestionError::internal(
            "E_INCREMENTAL_STORE",
            format!("encode retained cached report: {error}"),
            "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
        )
    })
}

#[allow(dead_code)] // The library compatibility copy does not prepare the binary store.
pub(super) struct PreparedStore {
    inner: store::PreparedStore,
}

#[allow(dead_code)] // The library compatibility copy does not prepare the binary store.
impl PreparedStore {
    pub fn path(&self) -> &Path {
        self.inner.path()
    }

    pub fn salt(&self) -> [u8; 32] {
        self.inner.salt()
    }

    pub fn commit(&mut self) -> Result<(), IngestionError> {
        self.inner.commit().map_err(store_error)
    }

    pub fn abort(&mut self) -> Result<(), IngestionError> {
        self.inner.abort().map_err(store_error)
    }
}

#[allow(dead_code)] // The library compatibility copy does not prepare the binary store.
pub(super) fn prepare_store(path: &Path, rebuild: bool) -> Result<PreparedStore, IngestionError> {
    store::prepare(path, rebuild)
        .map(|inner| PreparedStore { inner })
        .map_err(store_error)
}

#[allow(dead_code)] // The library compatibility copy does not drive rebuild publication.
pub(super) fn incomplete_rebuild_error() -> IngestionError {
    IngestionError::internal(
        "E_INCREMENTAL_STORE",
        "the complete source scan required for --rebuild-store did not finish with publishable coverage",
        "Restore access to every selected source branch and retry --rebuild-store, or omit it to keep the prior store.",
    )
}

#[allow(dead_code)] // The library compatibility copy does not publish the binary store.
pub(super) fn publish_cached_report(
    path: &Path,
    options: &IngestionOptions,
    files: &[store::SourceFile],
    analysis_state: &AnalysisState,
    encoded_analysis_state: Option<&[u8]>,
    invalidate_analysis_state: bool,
    report_json: &[u8],
) -> Result<(), IngestionError> {
    store::publish_report(
        path,
        options,
        files,
        analysis_state,
        encoded_analysis_state,
        invalidate_analysis_state,
        report_json,
    )
    .map_err(store_error)
}

#[allow(dead_code)] // The library compatibility copy does not open the binary store.
fn store_error(error: store::StoreError) -> IngestionError {
    IngestionError::internal(
        "E_INCREMENTAL_STORE",
        error.to_string(),
        "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
    )
}

pub(super) fn ingest(options: IngestionOptions) -> Result<IngestionResult, IngestionError> {
    let ingestion_started = Instant::now();
    let read_accounting = Arc::new(SourceReadAccounting::default());
    let store_load_started = Instant::now();
    let file_cache = options
        .store_path
        .as_deref()
        .map(|path| store::FileCache::open(path, &options))
        .transpose()
        .map_err(store_error)?;
    let store_load_nanos = store_load_started.elapsed().as_nanos();
    let discovery_started = Instant::now();
    let discovery = discovery::discover(&DiscoveryOptions {
        data_dirs: options.data_dirs,
        otel_files: options.otel_files,
        claude_config_dir: options.claude_config_dir,
        home_dir: options.home_dir,
        private_diagnostics: options.private_diagnostics,
    })
    .map_err(IngestionError::discovery)?;
    let discovery_nanos = discovery_started.elapsed().as_nanos();
    let private_source_paths = if options.private_diagnostics {
        discovery
            .sources
            .iter()
            .map(|source| (source.alias.clone(), source.path.clone()))
            .collect()
    } else {
        Vec::new()
    };
    let mut diagnostics = discovery.diagnostics.clone();
    let hasher = options
        .store_salt
        .map_or_else(PrivacyHasher::new, PrivacyHasher::persistent);
    let mut aliases = AliasRegistry::default();
    let mut private_prompts = Vec::new();
    let mut private_content_bytes = 0usize;
    let mut candidates = Vec::new();
    let mut metric_tracker = otel::MetricTracker::default();
    let mut transcript_traversal_budget = transcript::TraversalBudget::default();
    let mut store_files = Vec::new();
    let worker_count = select_worker_count(
        options.worker_count,
        std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get),
    )?;
    let transcript_worker_count = if options.include_private_content {
        1
    } else {
        worker_count
    };
    let mut performance = IngestionPerformance {
        schema: "ccwrapped.ingestion-performance/v1",
        selected_workers: worker_count,
        transcript_workers: transcript_worker_count,
        batch_files: 1,
        result_queue_capacity: if transcript_worker_count > 1 {
            transcript_worker_count.saturating_mul(2)
        } else {
            0
        },
        store_load_nanos,
        discovery_nanos,
        ..IngestionPerformance::default()
    };

    let append_started = Instant::now();
    let append_attempt = match file_cache.as_ref() {
        Some(cache) if cache.analysis_header().is_some() => prepare_append(
            &discovery,
            &diagnostics,
            cache,
            &transcript::TranscriptOptions {
                time_context: options.time_context.clone(),
                maximum_line_bytes: DEFAULT_MAXIMUM_LINE_BYTES,
                maximum_events: MAXIMUM_NORMALIZED_EVENTS,
                include_private_content: options.include_private_content,
                worker_count: transcript_worker_count,
                worker_delay_seed: options.worker_delay_seed,
                worker_panic_file: options.worker_panic_file,
                read_accounting: Arc::clone(&read_accounting),
            },
            &hasher,
            &mut transcript_traversal_budget,
        )?,
        _ => AppendAttempt::unavailable("unavailable"),
    };
    performance.incremental_checkpoint_status = append_attempt.status;
    if let Some(fast) = append_attempt.fast_prompt {
        performance.transcript_parse_nanos = append_started.elapsed().as_nanos();
        performance.candidate_records = fast.delta_records;
        performance.accepted_records = fast.diagnostics.accepted_records;
        performance.canonical_records = fast.diagnostics.canonical_records;
        let state_diagnostics = fast.diagnostics.clone();
        let coverage = fast
            .diagnostics
            .finalize(&options.time_context, options.timezone_fallback);
        let fast_report_json =
            patch_prompt_append_report(fast.cached_report, &coverage, &fast.prompt_sessions)?;
        (
            performance.source_content_bytes_read,
            performance.parsed_source_files,
        ) = read_accounting.snapshot();
        performance.reused_source_files =
            fast.store_files.iter().filter(|file| file.reused()).count();
        performance.ingestion_total_nanos = ingestion_started.elapsed().as_nanos();
        return Ok(IngestionResult {
            entries: Vec::new(),
            session_breakdown: SessionBreakdown::default(),
            daily: Vec::new(),
            project_breakdown: Vec::new(),
            methodology: MethodologyCatalog::default(),
            canonical_metrics: CanonicalMetrics::default(),
            insights: InsightReport::default(),
            hour_distribution: Vec::new(),
            coverage,
            private_prompts,
            private_source_paths,
            store_files: fast.store_files,
            analysis_state: AnalysisState {
                canonical_events: Vec::new(),
                diagnostics: state_diagnostics,
                aliases: fast.aliases,
                alias_observations: Vec::new(),
                observed_summary: insights::ObservedEventSummary::default(),
                dedup_keys: Vec::new(),
                authority_keys: Vec::new(),
                otel_request_groups: Vec::new(),
                aggregate_metrics: Vec::new(),
            },
            encoded_analysis_state: None,
            invalidate_analysis_state: true,
            store_publish_allowed: true,
            fast_report_json: Some(fast_report_json),
            performance,
        });
    }
    let mut prepared_append = append_attempt.prepared;
    let mut full_transcript = append_attempt.full_transcript;
    if let Some(prepared) = &prepared_append {
        performance.transcript_parse_nanos = append_started.elapsed().as_nanos();
        performance.candidate_records = prepared.delta_records;
    }

    if prepared_append.is_none() {
        for source in &discovery.sources {
            if source.kind != SourceKind::Transcript {
                continue;
            }
            let source_started = Instant::now();
            let mut source_events = if let Some(full) = full_transcript.take() {
                diagnostics = full.diagnostics;
                aliases = full.aliases;
                store_files.extend(full.store_files);
                full.events
            } else {
                transcript::ingest(
                    source,
                    &transcript::TranscriptOptions {
                        time_context: options.time_context.clone(),
                        maximum_line_bytes: DEFAULT_MAXIMUM_LINE_BYTES,
                        maximum_events: MAXIMUM_NORMALIZED_EVENTS.saturating_sub(candidates.len()),
                        include_private_content: options.include_private_content,
                        worker_count: transcript_worker_count,
                        worker_delay_seed: options.worker_delay_seed,
                        worker_panic_file: options.worker_panic_file,
                        read_accounting: Arc::clone(&read_accounting),
                    },
                    &mut diagnostics,
                    &hasher,
                    &mut aliases,
                    &mut private_prompts,
                    &mut private_content_bytes,
                    &mut transcript_traversal_budget,
                    &mut store_files,
                    file_cache.as_ref(),
                )
                .map_err(|error| transcript_ingestion_error(source.alias.clone(), error))?
            };
            performance.transcript_parse_nanos = performance
                .transcript_parse_nanos
                .saturating_add(source_started.elapsed().as_nanos());
            if candidates
                .len()
                .checked_add(source_events.len())
                .is_none_or(|count| count > MAXIMUM_NORMALIZED_EVENTS)
            {
                return Err(IngestionError::source(
                    "E_NORMALIZED_EVENT_LIMIT",
                    source.alias.clone(),
                    "the invocation exceeded the normalized-event safety limit",
                    "Narrow the selected period or split the source set into smaller invocations.",
                ));
            }
            candidates.append(&mut source_events);
        }

        let otel_sources = discovery
            .sources
            .iter()
            .filter(|source| source.kind == SourceKind::Otel)
            .collect::<Vec<_>>();
        let otel_started = Instant::now();
        let mut otel_events = otel::ingest_sources(
            &otel_sources,
            &otel::OtelOptions {
                time_context: options.time_context.clone(),
                maximum_line_bytes: DEFAULT_MAXIMUM_LINE_BYTES,
                maximum_events: MAXIMUM_NORMALIZED_EVENTS.saturating_sub(candidates.len()),
                read_accounting: Arc::clone(&read_accounting),
            },
            &mut diagnostics,
            &hasher,
            &mut aliases,
            &mut metric_tracker,
            worker_count,
            options.worker_delay_seed,
            &mut store_files,
            file_cache.as_ref(),
        )
        .map_err(otel_ingestion_error)?;
        performance.otel_parse_nanos = otel_started.elapsed().as_nanos();
        if candidates
            .len()
            .checked_add(otel_events.len())
            .is_none_or(|count| count > MAXIMUM_NORMALIZED_EVENTS)
        {
            let source_alias = otel_events
                .first()
                .map_or_else(|| "otel".to_string(), |event| event.source_alias.clone());
            return Err(IngestionError::source(
                "E_NORMALIZED_EVENT_LIMIT",
                source_alias,
                "the invocation exceeded the normalized-event safety limit",
                "Narrow the selected period or split the source set into smaller invocations.",
            ));
        }
        candidates.append(&mut otel_events);

        let metric_finalize_started = Instant::now();
        let mut metric_events = otel::finalize_metrics(
            &mut metric_tracker,
            &options.time_context,
            &mut diagnostics,
            &hasher,
            &mut aliases,
        );
        if candidates
            .len()
            .checked_add(metric_events.len())
            .is_none_or(|count| count > MAXIMUM_NORMALIZED_EVENTS)
        {
            let source_alias = metric_events
                .first()
                .map_or_else(|| "otel".to_string(), |event| event.source_alias.clone());
            return Err(IngestionError::source(
                "E_NORMALIZED_EVENT_LIMIT",
                source_alias,
                "the invocation exceeded the normalized-event safety limit",
                "Narrow the selected period or split the source set into smaller invocations.",
            ));
        }
        candidates.append(&mut metric_events);
        performance.metric_finalize_nanos = metric_finalize_started.elapsed().as_nanos();
        performance.candidate_records = candidates.len();
    }

    let source_dedup_started = Instant::now();
    let (
        mut events,
        mut canonical_events,
        alias_observations,
        prepared_observed_summary,
        prepared_dedup_keys,
        prepared_authority_keys,
        prepared_otel_request_groups,
        prepared_aggregate_metrics,
    ) = if let Some(prepared) = prepared_append.take() {
        diagnostics = prepared.diagnostics;
        store_files = prepared.store_files;
        performance.accepted_records = diagnostics.accepted_records;
        performance.canonical_records = prepared.canonical_events.len();
        (
            Vec::new(),
            prepared.canonical_events,
            prepared.alias_observations,
            Some(prepared.observed_summary),
            Some(prepared.dedup_keys),
            Some(prepared.authority_keys),
            Some(prepared.otel_request_groups),
            Some(prepared.aggregate_metrics),
        )
    } else {
        let alias_observations = candidates
            .iter()
            .map(AliasObservation::from_event)
            .collect::<Vec<_>>();
        let events = deduplicate(candidates, &mut diagnostics);
        performance.source_dedup_nanos = source_dedup_started.elapsed().as_nanos();
        performance.accepted_records = events.len();
        debug_assert!(
            events
                .iter()
                .map(|event| event.redacted_fields)
                .fold(0usize, usize::saturating_add)
                <= diagnostics.redacted_fields
        );
        let source_aliases = diagnostics.sources.keys().cloned().collect::<Vec<_>>();
        let (canonical_events, capability_nanos, authority_nanos) = if worker_count >= 2 {
            thread::scope(|scope| {
                let capability = scope.spawn(|| {
                    let started = Instant::now();
                    let observation = compute_capability_observation(&events, &source_aliases);
                    (observation, started.elapsed().as_nanos())
                });
                let authority_started = Instant::now();
                let canonical_events = select_canonical_events(
                    &events,
                    &mut diagnostics,
                    worker_count.saturating_sub(1).max(1),
                )?;
                let authority_nanos = authority_started.elapsed().as_nanos();
                let (capability, capability_nanos) = capability.join().map_err(|_| {
                IngestionError::internal(
                    "E_CAPABILITY_WORKER",
                    "a capability worker panicked; no report was published",
                    "Retry with the same inputs; if the error persists, report the tool version and error code without attaching private history.",
                )
            })?;
                apply_capability_observation(capability, &mut diagnostics);
                Ok::<_, IngestionError>((canonical_events, capability_nanos, authority_nanos))
            })?
        } else {
            let capability_started = Instant::now();
            record_capabilities(&events, &mut diagnostics);
            let capability_nanos = capability_started.elapsed().as_nanos();
            let authority_started = Instant::now();
            let canonical_events =
                select_canonical_events(&events, &mut diagnostics, worker_count)?;
            let authority_nanos = authority_started.elapsed().as_nanos();
            (canonical_events, capability_nanos, authority_nanos)
        };
        performance.capability_aggregation_nanos = capability_nanos;
        performance.authority_selection_nanos = authority_nanos;
        (
            events,
            canonical_events,
            alias_observations,
            None,
            None,
            None,
            None,
            None,
        )
    };
    aliases = rebuild_alias_registry(&alias_observations);
    assign_analysis_aliases_parallel(&mut events, &aliases, worker_count)?;
    assign_analysis_aliases_parallel(&mut canonical_events, &aliases, worker_count)?;
    sort_events(&mut canonical_events);
    let analytical_capability_started = Instant::now();
    diagnostics.canonical_records = canonical_events.len();
    record_analytical_capabilities(&canonical_events, &mut diagnostics);
    debug_assert_eq!(
        diagnostics.accepted_records,
        diagnostics
            .canonical_records
            .saturating_add(diagnostics.resolved_overlap_records)
            .saturating_add(diagnostics.unresolved_overlap_records)
            .saturating_add(diagnostics.authority_excluded_records),
        "authority accounting must reconcile source observations to canonical events"
    );
    performance.analytical_capability_nanos = analytical_capability_started.elapsed().as_nanos();
    performance.canonical_records = canonical_events.len();
    let state_diagnostics = diagnostics.clone();
    let observed_summary = prepared_observed_summary.unwrap_or_else(|| {
        insights::ObservedEventSummary::from_events(&events, &options.time_context)
    });
    let source_indices = diagnostics
        .sources
        .keys()
        .enumerate()
        .map(|(index, alias)| (alias.as_str(), index))
        .collect::<HashMap<_, _>>();
    let dedup_keys = prepared_dedup_keys.unwrap_or_else(|| {
        events
            .iter()
            .map(|event| {
                let source_index = source_indices
                    .get(event.source_alias.as_str())
                    .copied()
                    .expect("accepted event source aliases must be registered");
                event.dedup_key(source_index)
            })
            .collect()
    });
    let authority_keys = prepared_authority_keys
        .unwrap_or_else(|| events.iter().filter_map(append_authority_key).collect());
    let otel_request_groups = prepared_otel_request_groups.unwrap_or_else(|| {
        events
            .iter()
            .filter(|event| event.kind == types::EventKind::OtelApiRequest)
            .filter_map(request_correlation_group_key)
            .collect()
    });
    let aggregate_metrics = prepared_aggregate_metrics.unwrap_or_else(|| {
        events
            .iter()
            .filter(|event| {
                event.kind == types::EventKind::OtelMetric
                    && (event.tokens.richness() > 0 || event.source_cost_estimate.is_some())
            })
            .cloned()
            .collect()
    });
    let analysis_state = AnalysisState {
        canonical_events,
        diagnostics: state_diagnostics,
        aliases: aliases.snapshot(),
        alias_observations,
        observed_summary,
        dedup_keys,
        authority_keys,
        otel_request_groups,
        aggregate_metrics,
    };

    let projection_started = Instant::now();
    let mut projection = views::build_canonical_projection(
        &analysis_state.canonical_events,
        &options.time_context,
        options.active_threshold_seconds,
        worker_count,
    )
    .map_err(|error| match error {
        views::ProjectionError::Time(error) => IngestionError::time(error),
        views::ProjectionError::WorkerPanic => IngestionError::internal(
            "E_CANONICAL_PROJECTION",
            "a canonical-projection worker panicked; no report was published",
            "Retry with the same inputs; if the error persists, report the tool version and error code without attaching private history.",
        ),
    })?;
    let projection_nanos = projection_started.elapsed().as_nanos();
    let encoded_analysis_state = options
        .store_path
        .as_ref()
        .map(|_| store::encode_analysis_state(&analysis_state).map_err(store_error))
        .transpose()?;
    validate_projection(&projection)?;
    if projection.cache_ttl_composition_invalid {
        diagnostics.warning(
            "W_PRICING_CACHE_TTL_COMPOSITION",
            "Cache-creation TTL components exceeded their enclosing cache-creation total; that total remains unpriced and local API-equivalent coverage is incomplete.",
            None,
        );
    }
    if projection.metrics.cost.unpriced_tokens > 0 {
        diagnostics.warning(
            "W_PRICING_UNPRICED_USAGE",
            "Some observed usage has no exact provider/model/effective-interval/modifier price; the API-equivalent subtotal is incomplete.",
            None,
        );
    }
    private_prompts.sort_by(|left, right| {
        left.project_alias
            .cmp(&right.project_alias)
            .then_with(|| left.session_alias.cmp(&right.session_alias))
            .then_with(|| left.timestamp.cmp(&right.timestamp))
            .then_with(|| left.entrypoint.cmp(&right.entrypoint))
            .then_with(|| left.text.cmp(&right.text))
    });
    private_prompts.dedup_by(|left, right| {
        left.project_alias == right.project_alias
            && left.session_alias == right.session_alias
            && left.timestamp == right.timestamp
            && left.entrypoint == right.entrypoint
            && left.text == right.text
    });
    let store_publish_allowed = scan_is_authoritative_for_store(&diagnostics);
    let coverage = diagnostics.finalize(&options.time_context, options.timezone_fallback);
    performance.canonical_projection_nanos = projection_nanos;
    performance.projection_activity_nanos = projection.performance.activity_nanos;
    performance.projection_tokens_nanos = projection.performance.tokens_nanos;
    performance.projection_cost_nanos = projection.performance.cost_nanos;
    performance.projection_cache_nanos = projection.performance.cache_nanos;
    performance.projection_daily_nanos = projection.performance.daily_nanos;
    performance.projection_projects_nanos = projection.performance.projects_nanos;
    performance.projection_sessions_nanos = projection.performance.sessions_nanos;
    performance.projection_methodology_nanos = projection.performance.methodology_nanos;
    performance.projection_hour_distribution_nanos = projection.performance.hour_distribution_nanos;
    performance.projection_compatibility_entries_nanos =
        projection.performance.compatibility_entries_nanos;

    let insight_started = Instant::now();
    let insights = insights::build_from_summary(
        &analysis_state.observed_summary,
        &analysis_state.canonical_events,
        &projection.metrics,
        &coverage,
        &options.time_context,
        &mut projection.methodology,
    )?;
    performance.insight_build_nanos = insight_started.elapsed().as_nanos();
    (
        performance.source_content_bytes_read,
        performance.parsed_source_files,
    ) = read_accounting.snapshot();
    performance.reused_source_files = store_files.iter().filter(|file| file.reused()).count();
    performance.ingestion_total_nanos = ingestion_started.elapsed().as_nanos();
    Ok(IngestionResult {
        entries: projection.entries,
        session_breakdown: projection.session_breakdown,
        daily: projection.daily,
        project_breakdown: projection.projects,
        methodology: projection.methodology,
        canonical_metrics: projection.metrics,
        insights,
        hour_distribution: projection.hour_distribution,
        coverage,
        private_prompts,
        private_source_paths,
        store_files,
        analysis_state,
        encoded_analysis_state,
        invalidate_analysis_state: false,
        store_publish_allowed,
        fast_report_json: None,
        performance,
    })
}

fn scan_is_authoritative_for_store(diagnostics: &types::Diagnostics) -> bool {
    !diagnostics.warnings.iter().any(|warning| {
        warning.code == "W_TRANSCRIPT_SUBTREE_INACCESSIBLE"
            || warning.code == "W_TRANSCRIPT_DIRECTORY_DEPTH_LIMIT"
            || warning.code == "W_TRANSCRIPT_SYMLINK_ESCAPE"
            || warning.code.starts_with("W_DISCOVERY_")
    })
}

#[allow(clippy::too_many_arguments)]
fn prepare_append(
    discovery: &discovery::Discovery,
    current_diagnostics: &types::Diagnostics,
    file_cache: &store::FileCache,
    transcript_options: &transcript::TranscriptOptions,
    hasher: &PrivacyHasher,
    traversal_budget: &mut transcript::TraversalBudget,
) -> Result<AppendAttempt, IngestionError> {
    let Some((mut diagnostics, alias_state)) = file_cache.analysis_header() else {
        return Ok(AppendAttempt::unavailable("unavailable"));
    };
    let transcript_sources = discovery
        .sources
        .iter()
        .filter(|source| source.kind == SourceKind::Transcript)
        .collect::<Vec<_>>();
    if transcript_sources.len() != 1
        || transcript_options.include_private_content
        || transcript_options.worker_delay_seed.is_some()
        || transcript_options.worker_panic_file.is_some()
        || current_diagnostics
            .sources
            .values()
            .any(|source| source.partial)
        || !current_diagnostics.warnings.is_empty()
        || current_diagnostics
            .sources
            .keys()
            .ne(diagnostics.sources.keys())
    {
        return Ok(AppendAttempt::unavailable("ineligible-invocation"));
    }
    for source in discovery
        .sources
        .iter()
        .filter(|source| source.kind == SourceKind::Otel)
    {
        if !file_cache
            .is_unchanged(
                &source.path,
                &source.path,
                &source.alias,
                source.kind,
                &source.discovery_snapshot,
            )
            .map_err(store_error)?
        {
            return Ok(AppendAttempt::unavailable("telemetry-changed"));
        }
    }

    diagnostics.source_root_count = current_diagnostics.source_root_count;
    let mut aliases = AliasRegistry::restore(alias_state);
    let mut append_discovery_diagnostics = current_diagnostics.clone();
    let mut store_files = Vec::new();
    let prepared = transcript::ingest_prepared_append(
        transcript_sources[0],
        transcript_options,
        &mut append_discovery_diagnostics,
        &mut diagnostics,
        hasher,
        &mut aliases,
        traversal_budget,
        &mut store_files,
        file_cache,
    )
    .map_err(|error| transcript_ingestion_error(transcript_sources[0].alias.clone(), error))?;
    let mut full_fallback = Some(prepared.full_fallback);
    if !prepared.append_safe {
        return materialize_full_fallback(
            transcript_sources[0],
            "transcript-not-append-only",
            full_fallback.take().expect("full transcript fallback"),
        );
    }
    let file_alias_remap = prepared
        .file_alias_remap
        .iter()
        .map(|(previous, current)| (previous.as_str(), current.as_str()))
        .collect::<HashMap<_, _>>();
    for shape in &mut diagnostics.unknown_shapes {
        if let Some(current) = file_alias_remap.get(shape.file_alias.as_str()) {
            shape.file_alias.clear();
            shape.file_alias.push_str(current);
        }
    }
    if let Some((prompt_sessions, cached_report)) =
        fast_prompt_report(file_cache, &prepared.events)?
    {
        if !reconcile_append_inventory(discovery, file_cache, &mut store_files)? {
            return materialize_full_fallback(
                transcript_sources[0],
                "source-inventory-changed",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
        merge_append_capabilities(&prepared.events, &mut diagnostics);
        diagnostics.canonical_records = diagnostics
            .canonical_records
            .saturating_add(prepared.events.len());
        return Ok(AppendAttempt::fast_prompt(FastPromptAppend {
            diagnostics,
            aliases: aliases.snapshot(),
            store_files,
            prompt_sessions,
            cached_report,
            delta_records: prepared.events.len(),
        }));
    }
    let state = file_cache
        .take_analysis_state()
        .map_err(store_error)?
        .ok_or_else(|| {
            IngestionError::internal(
                "E_INCREMENTAL_STORE",
                "the analysis checkpoint disappeared during append preparation",
                "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
            )
        })?;
    let AnalysisState {
        mut canonical_events,
        diagnostics: _,
        aliases: _,
        mut alias_observations,
        mut observed_summary,
        mut dedup_keys,
        mut authority_keys,
        otel_request_groups,
        aggregate_metrics,
    } = state;
    for event in &mut canonical_events {
        if let Some(current) = file_alias_remap.get(event.file_alias.as_str()) {
            event.file_alias.clear();
            event.file_alias.push_str(current);
        }
    }
    for observation in &mut alias_observations {
        if let Some(current) = file_alias_remap.get(observation.file_alias.as_str()) {
            observation.file_alias.clear();
            observation.file_alias.push_str(current);
        }
    }
    alias_observations.extend(prepared.events.iter().map(AliasObservation::from_event));
    alias_observations.sort_by(|left, right| match (left.transcript, right.transcript) {
        (true, true) => left
            .source_alias
            .cmp(&right.source_alias)
            .then_with(|| {
                alias_numeric_suffix(&left.file_alias).cmp(&alias_numeric_suffix(&right.file_alias))
            })
            .then_with(|| left.record_index.cmp(&right.record_index)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => std::cmp::Ordering::Equal,
    });

    let source_indices = diagnostics
        .sources
        .keys()
        .enumerate()
        .map(|(index, alias)| (alias.as_str(), index))
        .collect::<HashMap<_, _>>();
    let existing_dedup_index = dedup_keys.iter().cloned().collect::<HashSet<_>>();
    let mut appended_dedup_index = HashSet::new();
    let existing_authority_index = authority_keys.iter().cloned().collect::<HashSet<_>>();
    let mut appended_authority_index = HashSet::new();
    let request_groups = otel_request_groups.iter().copied().collect::<HashSet<_>>();
    for event in &prepared.events {
        if event.adapter_version != types::TRANSCRIPT_ADAPTER {
            return materialize_full_fallback(
                transcript_sources[0],
                "adapter-changed",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
        let Some(source_index) = source_indices.get(event.source_alias.as_str()).copied() else {
            return materialize_full_fallback(
                transcript_sources[0],
                "source-alias-changed",
                full_fallback.take().expect("full transcript fallback"),
            );
        };
        let dedup_key = event.dedup_key(source_index);
        if existing_dedup_index.contains(&dedup_key) {
            return materialize_full_fallback(
                transcript_sources[0],
                "record-already-observed",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
        if !appended_dedup_index.insert(dedup_key.clone()) {
            return materialize_full_fallback(
                transcript_sources[0],
                "duplicate-appended-record",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
        dedup_keys.push(dedup_key);
        let Some(authority_key) = append_authority_key(event) else {
            return materialize_full_fallback(
                transcript_sources[0],
                "weak-authority",
                full_fallback.take().expect("full transcript fallback"),
            );
        };
        if existing_authority_index.contains(&authority_key) {
            return materialize_full_fallback(
                transcript_sources[0],
                "authority-already-observed",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
        if !appended_authority_index.insert(authority_key.clone()) {
            return materialize_full_fallback(
                transcript_sources[0],
                "duplicate-appended-authority",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
        authority_keys.push(authority_key);
        if request_correlation_group_key(event).is_some_and(|group| request_groups.contains(&group))
        {
            return materialize_full_fallback(
                transcript_sources[0],
                "telemetry-request-correlation",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
        if aggregate_metrics.iter().any(|metric| {
            aggregate_usage_relation(metric, event) != AggregateUsageRelation::Disjoint
        }) {
            return materialize_full_fallback(
                transcript_sources[0],
                "aggregate-metric-overlap",
                full_fallback.take().expect("full transcript fallback"),
            );
        }
    }

    if !reconcile_append_inventory(discovery, file_cache, &mut store_files)? {
        return materialize_full_fallback(
            transcript_sources[0],
            "source-inventory-changed",
            full_fallback.take().expect("full transcript fallback"),
        );
    }
    merge_append_capabilities(&prepared.events, &mut diagnostics);
    diagnostics.canonical_records = diagnostics
        .canonical_records
        .saturating_add(prepared.events.len());
    observed_summary.extend(&prepared.events, &transcript_options.time_context);
    canonical_events.extend(prepared.events.iter().cloned());
    sort_events(&mut canonical_events);

    Ok(AppendAttempt::used(PreparedAppend {
        canonical_events,
        diagnostics,
        alias_observations,
        observed_summary,
        dedup_keys,
        authority_keys,
        otel_request_groups,
        aggregate_metrics,
        store_files,
        delta_records: prepared.events.len(),
    }))
}

fn materialize_full_fallback(
    source: &discovery::Source,
    status: &'static str,
    fallback: transcript::PreparedFullTranscript,
) -> Result<AppendAttempt, IngestionError> {
    let full = fallback
        .materialize()
        .map_err(|error| transcript_ingestion_error(source.alias.clone(), error))?;
    Ok(AppendAttempt::full_fallback(status, full))
}

fn fast_prompt_report(
    file_cache: &store::FileCache,
    appended: &[NormalizedEvent],
) -> Result<Option<(Vec<String>, Report)>, IngestionError> {
    let (Some(cached_report), Some(existing_authority)) =
        (file_cache.cached_report(), file_cache.authority_keys())
    else {
        return Ok(None);
    };
    if appended.is_empty()
        || appended.iter().any(|event| {
            event.kind != types::EventKind::UserPrompt
                || event.is_subagent
                || event.is_sidechain
                || event.tokens.richness() != 0
                || event.source_cost_estimate.is_some()
                || !event.tool_names.is_empty()
        })
    {
        return Ok(None);
    }
    let report: Report = serde_json::from_slice(cached_report).map_err(|error| {
        IngestionError::internal(
            "E_INCREMENTAL_STORE",
            format!("decode cached report for append validation: {error}"),
            "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
        )
    })?;
    let mut appended_authority = HashSet::new();
    let mut prompt_sessions = Vec::with_capacity(appended.len());
    for event in appended {
        let Some(authority) = append_authority_key(event) else {
            return Ok(None);
        };
        if existing_authority.contains(&authority) || !appended_authority.insert(authority) {
            return Ok(None);
        }
        let matching_session = report.session_breakdown.sessions.iter().any(|session| {
            session.session_id == event.session_alias
                && session.project_hash == event.project_alias
                && session.timestamp_end.as_deref() == Some(event.timestamp.as_str())
        });
        if !matching_session {
            return Ok(None);
        }
        prompt_sessions.push(event.session_alias.clone());
    }
    Ok(Some((prompt_sessions, report)))
}

fn reconcile_append_inventory(
    discovery: &discovery::Discovery,
    file_cache: &store::FileCache,
    store_files: &mut Vec<store::SourceFile>,
) -> Result<bool, IngestionError> {
    for source in discovery
        .sources
        .iter()
        .filter(|source| source.kind == SourceKind::Otel)
    {
        let raw = file_cache
            .lookup_raw(
                &source.path,
                &source.path,
                &source.alias,
                source.kind,
                &source.discovery_snapshot,
            )
            .map_err(store_error)?
            .ok_or_else(|| {
                IngestionError::internal(
                    "E_INCREMENTAL_STORE",
                    "an unchanged telemetry row disappeared during append preparation",
                    "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
                )
            })?;
        store_files.push(
            store::SourceFile::reused_metadata(
                source.path.clone(),
                source.path.clone(),
                source.alias.clone(),
                source.kind,
                source.discovery_snapshot.clone(),
                raw.content_digest(),
            )
            .with_file_alias(format!("{}-file-1", source.alias)),
        );
    }
    Ok(file_cache.remaining_rows() == 0)
}

fn patch_prompt_append_report(
    mut report: Report,
    coverage: &DataCoverage,
    prompt_sessions: &[String],
) -> Result<Vec<u8>, IngestionError> {
    let mut updated_coverage = coverage.clone();
    for warning in &report.data_coverage.warnings {
        if warning.code.starts_with("W_PRICING_")
            && !updated_coverage
                .warnings
                .iter()
                .any(|current| current.code == warning.code)
        {
            updated_coverage.warnings.push(warning.clone());
        }
    }
    updated_coverage.warnings.sort_by(|left, right| {
        left.source_alias
            .cmp(&right.source_alias)
            .then_with(|| left.code.cmp(&right.code))
            .then_with(|| left.message.cmp(&right.message))
    });
    report.data_coverage = updated_coverage;
    let mut increments = HashMap::<&str, usize>::new();
    for session in prompt_sessions {
        *increments.entry(session.as_str()).or_default() += 1;
    }
    let mut matched = 0usize;
    for session in &mut report.session_breakdown.sessions {
        if let Some(increment) = increments.get(session.session_id.as_str()) {
            session.prompt_count = session.prompt_count.saturating_add(*increment);
            matched = matched.saturating_add(1);
        }
    }
    if matched != increments.len() {
        return Err(IngestionError::internal(
            "E_INCREMENTAL_STORE",
            "a prompt-only append referenced a session absent from the cached report",
            "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
        ));
    }
    let added = prompt_sessions.len();
    report.wrapped_story.prompt_ratio.human = report
        .wrapped_story
        .prompt_ratio
        .human
        .saturating_add(added);
    report.wrapped_story.prompt_ratio.total = report
        .wrapped_story
        .prompt_ratio
        .human
        .saturating_add(report.wrapped_story.prompt_ratio.tool);
    report.wrapped_story.prompt_ratio.human_pct = if report.wrapped_story.prompt_ratio.total == 0 {
        0
    } else {
        ((report.wrapped_story.prompt_ratio.human as f64
            / report.wrapped_story.prompt_ratio.total as f64)
            * 100.0)
            .round() as u64
    };
    let prompt_ratio = &report.wrapped_story.prompt_ratio;
    let Some(hero) = report
        .wrapped_story
        .hero
        .iter_mut()
        .find(|hero| hero.label == "Human prompts")
    else {
        return Err(IngestionError::internal(
            "E_INCREMENTAL_STORE",
            "the cached report has no human-prompt hero statistic",
            "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
        ));
    };
    hero.value = format!("{}%", prompt_ratio.human_pct);
    hero.note = format!(
        "{} human / {} tool",
        grouped_u64(prompt_ratio.human as u64),
        grouped_u64(prompt_ratio.tool as u64)
    );
    serde_json::to_vec_pretty(&report).map_err(|error| {
        IngestionError::internal(
            "E_INCREMENTAL_STORE",
            format!("encode cached report after prompt append: {error}"),
            "Run with --rebuild-store to replace derived state, or use --no-store to bypass it.",
        )
    })
}

fn grouped_u64(value: u64) -> String {
    let digits = value.to_string();
    let mut grouped = String::with_capacity(
        digits
            .len()
            .saturating_add(digits.len().saturating_sub(1) / 3),
    );
    for (index, character) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).checked_rem(3) == Some(0) {
            grouped.push(',');
        }
        grouped.push(character);
    }
    grouped
}

fn alias_numeric_suffix(alias: &str) -> u64 {
    alias
        .rsplit_once('-')
        .and_then(|(_, suffix)| suffix.parse().ok())
        .unwrap_or(u64::MAX)
}

fn rebuild_alias_registry(observations: &[AliasObservation]) -> AliasRegistry {
    let mut aliases = AliasRegistry::default();
    for observation in observations {
        if observation.project_identity_present {
            aliases.project(observation.project_key);
        }
        aliases.session(observation.session_key);
        if let Some(parent) = observation.parent_key {
            aliases.session(parent);
        }
    }
    aliases
}

fn assign_analysis_aliases(event: &mut NormalizedEvent, aliases: &AliasRegistry) {
    event.project_alias = if event.project_identity_present {
        aliases
            .existing_project(event.project_key)
            .expect("observed project keys must be registered before alias projection")
            .to_string()
    } else {
        types::UNATTRIBUTED_PROJECT_ALIAS.to_string()
    };
    event.session_alias = aliases
        .existing_session(event.session_key)
        .expect("observed session keys must be registered before alias projection")
        .to_string();
    event.parent_session_alias = event.parent_key.map(|parent| {
        aliases
            .existing_session(parent)
            .expect("observed parent-session keys must be registered before alias projection")
            .to_string()
    });
}

fn assign_analysis_aliases_parallel(
    events: &mut [NormalizedEvent],
    aliases: &AliasRegistry,
    worker_count: usize,
) -> Result<(), IngestionError> {
    let worker_count = worker_count.max(1).min(events.len().max(1));
    if worker_count == 1 || events.len() < PARALLEL_ALIAS_MINIMUM_EVENTS {
        for event in events {
            assign_analysis_aliases(event, aliases);
        }
        return Ok(());
    }
    let chunk_size = events.len().div_ceil(worker_count);
    thread::scope(|scope| {
        let workers = events
            .chunks_mut(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    for event in chunk {
                        assign_analysis_aliases(event, aliases);
                    }
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            worker.join().map_err(|_| {
                IngestionError::internal(
                    "E_ALIAS_WORKER",
                    "an alias-projection worker panicked; no report was published",
                    "Retry with the same inputs; if the error persists, report the tool version and error code without attaching private history.",
                )
            })?;
        }
        Ok(())
    })
}

fn merge_append_capabilities(events: &[NormalizedEvent], diagnostics: &mut types::Diagnostics) {
    diagnostics.accepted_records = diagnostics.accepted_records.saturating_add(events.len());
    for event in events {
        let Some(source) = diagnostics.sources.get_mut(&event.source_alias) else {
            continue;
        };
        source.accepted_records = source.accepted_records.saturating_add(1);
        source.observe_time(event.epoch_nanos, &event.timestamp);
        for (name, observed) in [
            ("token_usage", event.tokens.richness() > 0),
            (
                "prompt_occurrence",
                event.kind == types::EventKind::UserPrompt,
            ),
            (
                "tool_occurrence",
                event.kind == types::EventKind::AssistantUsage && !event.tool_names.is_empty(),
            ),
            ("compaction", event.compaction.is_some()),
        ] {
            if observed {
                diagnostics
                    .capabilities
                    .insert(name.to_string(), "available".to_string());
                source
                    .capabilities
                    .insert(name.to_string(), "available".to_string());
            }
        }
    }
}

fn select_worker_count(
    requested: Option<usize>,
    available: usize,
) -> Result<usize, IngestionError> {
    let available = available.clamp(1, MAXIMUM_INGESTION_WORKERS);
    match requested {
        Some(0) => Err(IngestionError::internal(
            "E_INGESTION_WORKER_COUNT",
            "the ingestion worker count must be positive",
            "Select at least one ingestion worker.",
        )),
        Some(requested) if requested > available => Err(IngestionError::internal(
            "E_INGESTION_WORKER_COUNT",
            format!(
                "the requested ingestion worker count {requested} exceeds the affinity-available count {available}"
            ),
            "Select a worker count no greater than the CPUs available to this process.",
        )),
        Some(requested) => Ok(requested),
        None => Ok(available.min(DEFAULT_INGESTION_WORKERS)),
    }
}

fn validate_projection(projection: &views::CanonicalProjection) -> Result<(), IngestionError> {
    if views::projection_reconciles(projection) {
        return Ok(());
    }
    Err(IngestionError::internal(
        "E_METRIC_RECONCILIATION",
        "canonical metric dimensions did not reconcile",
        "Retry with the same inputs; if the error persists, report the tool version and error code without attaching private history.",
    ))
}

#[allow(dead_code)] // The binary compiles this private module without the library compatibility API.
pub(super) fn discover_transcript_paths(
    projects_dir: &Path,
    scope: CompatibilityPathScope,
) -> Result<Vec<PathBuf>, IngestionError> {
    let discovery = discovery::discover(&DiscoveryOptions {
        data_dirs: vec![projects_dir.to_path_buf()],
        otel_files: Vec::new(),
        claude_config_dir: None,
        home_dir: None,
        private_diagnostics: false,
    })
    .map_err(IngestionError::discovery)?;
    let discovery::Discovery {
        sources,
        mut diagnostics,
    } = discovery;
    let source = sources.first().ok_or_else(|| IngestionError {
        code: "E_TRANSCRIPT_DISCOVERY_EMPTY",
        source_alias: None,
        message: "explicit transcript discovery produced no source".to_string(),
        remediation: "Select an existing stable Claude projects directory and retry.",
    })?;
    let paths = transcript::discover_compatibility_paths(
        source,
        &mut diagnostics,
        &mut transcript::TraversalBudget::default(),
        scope,
    )
    .map_err(|error| {
        IngestionError::source(
            "E_TRANSCRIPT_DISCOVERY",
            source.alias.clone(),
            error,
            "Check the selected transcript source permissions and stability, then retry.",
        )
    })?;
    if diagnostics
        .sources
        .get(&source.alias)
        .is_some_and(|stats| stats.partial)
    {
        return Err(IngestionError::source(
            "E_TRANSCRIPT_DISCOVERY_PARTIAL",
            source.alias.clone(),
            "bounded transcript discovery excluded an unsafe or incomplete branch",
            "Remove symlinks and narrow the selected source to a stable directory tree, then retry.",
        ));
    }
    Ok(paths)
}

fn select_canonical_events(
    events: &[NormalizedEvent],
    diagnostics: &mut types::Diagnostics,
    worker_count: usize,
) -> Result<Vec<NormalizedEvent>, IngestionError> {
    let mut transcript_candidates = Vec::new();
    let mut request_candidates = Vec::new();
    let mut metric_candidates = Vec::new();
    let mut nonusage_metric_candidates = Vec::new();
    let mut other_candidates = Vec::new();
    for event in events {
        match event.kind {
            types::EventKind::AssistantUsage => transcript_candidates.push(event),
            types::EventKind::OtelApiRequest => request_candidates.push(event),
            types::EventKind::OtelMetric
                if event.tokens.richness() > 0 || event.source_cost_estimate.is_some() =>
            {
                metric_candidates.push(event);
            }
            types::EventKind::OtelMetric => nonusage_metric_candidates.push(event),
            _ => other_candidates.push(event),
        }
    }
    let (transcript_usage, otel_requests, otel_metrics, other, resolved) = if worker_count >= 3 {
        thread::scope(|scope| {
            let transcript = scope.spawn(|| collapse_cross_source(transcript_candidates));
            let other = scope.spawn(|| collapse_cross_source(other_candidates));
            let requests = collapse_cross_source(request_candidates);
            let metrics = collapse_cross_source(metric_candidates);
            let transcript = transcript.join().map_err(|_| authority_worker_panic())?;
            let other = other.join().map_err(|_| authority_worker_panic())?;
            Ok::<_, IngestionError>((
                transcript.0,
                requests.0,
                metrics.0,
                other.0,
                transcript
                    .1
                    .saturating_add(requests.1)
                    .saturating_add(metrics.1)
                    .saturating_add(other.1),
            ))
        })?
    } else if worker_count == 2 {
        thread::scope(|scope| {
            let transcript = scope.spawn(|| collapse_cross_source(transcript_candidates));
            let other = collapse_cross_source(other_candidates);
            let requests = collapse_cross_source(request_candidates);
            let metrics = collapse_cross_source(metric_candidates);
            let transcript = transcript.join().map_err(|_| authority_worker_panic())?;
            Ok::<_, IngestionError>((
                transcript.0,
                requests.0,
                metrics.0,
                other.0,
                transcript
                    .1
                    .saturating_add(requests.1)
                    .saturating_add(metrics.1)
                    .saturating_add(other.1),
            ))
        })?
    } else {
        let transcript = collapse_cross_source(transcript_candidates);
        let requests = collapse_cross_source(request_candidates);
        let metrics = collapse_cross_source(metric_candidates);
        let other = collapse_cross_source(other_candidates);
        (
            transcript.0,
            requests.0,
            metrics.0,
            other.0,
            transcript
                .1
                .saturating_add(requests.1)
                .saturating_add(metrics.1)
                .saturating_add(other.1),
        )
    };
    if resolved > 0 {
        record_resolved_overlap(diagnostics, resolved);
    }
    let (nonusage_metrics, nonusage_resolved) = collapse_cross_source(nonusage_metric_candidates);
    if nonusage_resolved > 0 {
        record_resolved_overlap(diagnostics, nonusage_resolved);
    }
    let has_transcript = !transcript_usage.is_empty();
    let has_otel_requests = !otel_requests.is_empty();
    let mut canonical = other.into_iter().cloned().collect::<Vec<_>>();

    if !has_transcript {
        if has_otel_requests {
            canonical.extend(otel_requests.iter().map(|event| (*event).clone()));
            let mut superseded_metrics = 0usize;
            let mut unresolved_metrics = 0usize;
            for metric in otel_metrics {
                let relation = aggregate_usage_relation_to_sorted(metric, &otel_requests);
                match relation {
                    AggregateUsageRelation::Disjoint => canonical.push(metric.clone()),
                    AggregateUsageRelation::Overlap => {
                        superseded_metrics = superseded_metrics.saturating_add(1);
                    }
                    AggregateUsageRelation::Ambiguous => {
                        unresolved_metrics = unresolved_metrics.saturating_add(1);
                        note_excluded_analytical_event(diagnostics, metric);
                    }
                }
            }
            diagnostics.authority_excluded_records = diagnostics
                .authority_excluded_records
                .saturating_add(superseded_metrics);
            diagnostics.unresolved_overlap_records = diagnostics
                .unresolved_overlap_records
                .saturating_add(unresolved_metrics);
            if superseded_metrics > 0 {
                diagnostics.warning(
                    "W_AUTHORITY_AGGREGATE_METRICS_SUPERSEDED",
                    "Timestamped telemetry request events superseded compatible aggregate usage intervals that contained them rather than summing both observations.",
                    None,
                );
            }
            if unresolved_metrics > 0 {
                diagnostics.warning(
                    "W_AUTHORITY_AGGREGATE_METRICS_UNRESOLVED",
                    "A telemetry request and aggregate usage interval overlapped without enough shared context to sum or prove replacement; the aggregate observation remained unresolved.",
                    None,
                );
            }
        } else {
            canonical.extend(otel_metrics.into_iter().cloned());
        }
        canonical.extend(nonusage_metrics.into_iter().cloned());
        sort_events(&mut canonical);
        return Ok(canonical);
    }

    let correlated = correlate_requests(&transcript_usage, &otel_requests, diagnostics);
    let used_transcripts = correlated.iter().flatten().copied().collect::<HashSet<_>>();
    let authoritative_otel_requests = otel_requests
        .iter()
        .enumerate()
        .filter_map(|(index, request)| correlated[index].map(|_| *request))
        .collect::<Vec<_>>();

    for (index, event) in transcript_usage.iter().enumerate() {
        if !used_transcripts.contains(&index) {
            canonical.push((*event).clone());
        }
    }
    for (otel_index, event) in otel_requests.into_iter().enumerate() {
        if let Some(transcript) =
            correlated[otel_index].and_then(|index| transcript_usage.get(index))
        {
            record_resolved_overlap(diagnostics, 1);
            let mut selected = event.clone();
            selected.project_key = transcript.project_key;
            selected.project_identity_present = transcript.project_identity_present;
            selected.project_alias = transcript.project_alias.clone();
            selected.session_key = transcript.session_key;
            selected.session_alias = transcript.session_alias.clone();
            selected.parent_key = transcript.parent_key;
            selected.parent_session_alias = transcript.parent_session_alias.clone();
            selected.agent_key = selected.agent_key.or(transcript.agent_key);
            selected.parent_agent_key = selected.parent_agent_key.or(transcript.parent_agent_key);
            selected.is_subagent = transcript.is_subagent;
            selected.is_sidechain = transcript.is_sidechain;
            canonical.push(selected);
        } else {
            diagnostics.unresolved_overlap_records =
                diagnostics.unresolved_overlap_records.saturating_add(1);
            note_excluded_analytical_event(diagnostics, event);
        }
    }
    let mut superseded_metrics = 0usize;
    let mut unresolved_metrics = 0usize;
    for metric in otel_metrics {
        let request_relation =
            aggregate_usage_relation_to_sorted(metric, &authoritative_otel_requests);
        match request_relation {
            AggregateUsageRelation::Overlap => {
                superseded_metrics = superseded_metrics.saturating_add(1);
            }
            AggregateUsageRelation::Ambiguous => {
                unresolved_metrics = unresolved_metrics.saturating_add(1);
                note_excluded_analytical_event(diagnostics, metric);
            }
            AggregateUsageRelation::Disjoint => {
                let transcript_relation =
                    aggregate_usage_relation_to_sorted(metric, &transcript_usage);
                match transcript_relation {
                    AggregateUsageRelation::Disjoint => canonical.push(metric.clone()),
                    AggregateUsageRelation::Overlap | AggregateUsageRelation::Ambiguous => {
                        unresolved_metrics = unresolved_metrics.saturating_add(1);
                        note_excluded_analytical_event(diagnostics, metric);
                    }
                }
            }
        }
    }
    diagnostics.authority_excluded_records = diagnostics
        .authority_excluded_records
        .saturating_add(superseded_metrics);
    diagnostics.unresolved_overlap_records = diagnostics
        .unresolved_overlap_records
        .saturating_add(unresolved_metrics);
    if superseded_metrics > 0 {
        diagnostics.warning(
            "W_AUTHORITY_AGGREGATE_METRICS_SUPERSEDED",
            "Timestamped telemetry request events superseded compatible aggregate usage intervals that contained them rather than summing both observations.",
            None,
        );
    }
    if diagnostics.unresolved_overlap_records > 0 {
        diagnostics.warning(
            "W_AUTHORITY_UNRESOLVED_OVERLAP",
            "Uncorrelated transcript and telemetry usage overlapped the selected period; authority/v1 kept transcript usage rather than summing.",
            None,
        );
    }
    canonical.extend(nonusage_metrics.into_iter().cloned());
    sort_events(&mut canonical);
    Ok(canonical)
}

fn authority_worker_panic() -> IngestionError {
    IngestionError::internal(
        "E_AUTHORITY_WORKER",
        "an authority worker panicked; no report was published",
        "Retry with the same inputs; if the error persists, report the tool version and error code without attaching private history.",
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AggregateUsageRelation {
    Disjoint,
    Ambiguous,
    Overlap,
}

impl AggregateUsageRelation {
    fn combine(self, other: Self) -> Self {
        match (self, other) {
            (Self::Overlap, _) | (_, Self::Overlap) => Self::Overlap,
            (Self::Ambiguous, _) | (_, Self::Ambiguous) => Self::Ambiguous,
            _ => Self::Disjoint,
        }
    }
}

fn aggregate_usage_relation(
    metric: &NormalizedEvent,
    direct_usage: &NormalizedEvent,
) -> AggregateUsageRelation {
    let (Some(start), Some(end), Ok(usage_time)) = (
        metric.metric_interval_start_nanos,
        metric.metric_interval_end_nanos,
        u64::try_from(direct_usage.epoch_nanos),
    ) else {
        return AggregateUsageRelation::Ambiguous;
    };
    if usage_time <= start || usage_time > end {
        return AggregateUsageRelation::Disjoint;
    }
    if metric.is_subagent != direct_usage.is_subagent
        || metric.is_sidechain != direct_usage.is_sidechain
        || !correlation_context_matches(metric.agent_key, direct_usage.agent_key)
        || !correlation_context_matches(metric.parent_agent_key, direct_usage.parent_agent_key)
        || matches!(
            (metric.model.as_deref(), direct_usage.model.as_deref()),
            (Some(metric), Some(direct_usage)) if metric != direct_usage
        )
    {
        return AggregateUsageRelation::Disjoint;
    }
    match (
        metric.session_identity_present,
        direct_usage.session_identity_present,
    ) {
        (true, true) if metric.session_key == direct_usage.session_key => {
            AggregateUsageRelation::Overlap
        }
        (true, true) => AggregateUsageRelation::Disjoint,
        _ => AggregateUsageRelation::Ambiguous,
    }
}

fn aggregate_usage_relation_to_sorted(
    metric: &NormalizedEvent,
    direct_usage: &[&NormalizedEvent],
) -> AggregateUsageRelation {
    if direct_usage.is_empty() {
        return AggregateUsageRelation::Disjoint;
    }
    debug_assert!(direct_usage
        .windows(2)
        .all(|pair| pair[0].epoch_nanos <= pair[1].epoch_nanos));
    let (Some(start), Some(end)) = (
        metric.metric_interval_start_nanos,
        metric.metric_interval_end_nanos,
    ) else {
        return AggregateUsageRelation::Ambiguous;
    };
    let start = i128::from(start);
    let end = i128::from(end);
    let mut relation = if direct_usage
        .first()
        .is_some_and(|event| event.epoch_nanos < 0)
        || direct_usage
            .last()
            .is_some_and(|event| event.epoch_nanos > i128::from(u64::MAX))
    {
        AggregateUsageRelation::Ambiguous
    } else {
        AggregateUsageRelation::Disjoint
    };
    if end <= start {
        return relation;
    }
    let first = direct_usage.partition_point(|event| event.epoch_nanos <= start);
    let after_last = direct_usage.partition_point(|event| event.epoch_nanos <= end);
    for event in &direct_usage[first..after_last] {
        relation = relation.combine(aggregate_usage_relation(metric, event));
        if relation == AggregateUsageRelation::Overlap {
            break;
        }
    }
    relation
}

fn note_excluded_analytical_event(diagnostics: &mut types::Diagnostics, event: &NormalizedEvent) {
    if event.tokens.input.is_some() {
        diagnostics.excluded_analysis_token_categories |= 1 << 0;
    }
    if event.tokens.output.is_some() {
        diagnostics.excluded_analysis_token_categories |= 1 << 1;
    }
    if event.tokens.cache_creation.is_some() {
        diagnostics.excluded_analysis_token_categories |= 1 << 2;
    }
    if event.tokens.cache_read.is_some() {
        diagnostics.excluded_analysis_token_categories |= 1 << 3;
    }
    if event.source_cost_estimate.is_some() {
        diagnostics.excluded_analysis_cost = true;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct RequestCorrelationGroupKey {
    request: u64,
    session: u64,
    sidechain: bool,
    subagent: bool,
}

#[derive(Debug, Default)]
struct RequestCorrelationGroup {
    transcript_indices: Vec<usize>,
    otel_indices: Vec<usize>,
}

fn correlate_requests(
    transcript_usage: &[&NormalizedEvent],
    otel_requests: &[&NormalizedEvent],
    diagnostics: &mut types::Diagnostics,
) -> Vec<Option<usize>> {
    let mut positions = HashMap::new();
    let mut groups = Vec::<RequestCorrelationGroup>::new();

    for (index, event) in otel_requests.iter().enumerate() {
        let Some(key) = request_correlation_group_key(event) else {
            continue;
        };
        let position = correlation_group_position(key, &mut positions, &mut groups);
        groups[position].otel_indices.push(index);
    }
    for (index, event) in transcript_usage.iter().enumerate() {
        let Some(key) = request_correlation_group_key(event) else {
            continue;
        };
        let Some(position) = positions.get(&key).copied() else {
            continue;
        };
        groups[position].transcript_indices.push(index);
    }

    let bounded_work = groups
        .iter()
        .filter(|group| {
            !group.transcript_indices.is_empty()
                && !group.otel_indices.is_empty()
                && correlation_group_size(group) <= MAX_REQUEST_CORRELATION_GROUP_EVENTS
        })
        .fold(0u64, |total, group| {
            total.saturating_add(correlation_group_work(group))
        });
    let invocation_work_is_bounded = bounded_work <= MAX_REQUEST_CORRELATION_WORK;
    let mut correlated = vec![None; otel_requests.len()];
    for group in groups {
        if group.transcript_indices.is_empty() || group.otel_indices.is_empty() {
            continue;
        }
        if correlation_group_size(&group) > MAX_REQUEST_CORRELATION_GROUP_EVENTS
            || !invocation_work_is_bounded
        {
            warn_request_correlation_limit(diagnostics);
            continue;
        }
        let Some(matches) = exact_request_matches(transcript_usage, otel_requests, &group) else {
            warn_request_correlation_limit(diagnostics);
            continue;
        };
        for (otel_index, transcript_index) in matches {
            correlated[otel_index] = Some(transcript_index);
        }
    }
    correlated
}

fn correlation_group_size(group: &RequestCorrelationGroup) -> usize {
    group
        .transcript_indices
        .len()
        .saturating_add(group.otel_indices.len())
}

fn correlation_group_work(group: &RequestCorrelationGroup) -> u64 {
    u64::try_from(correlation_group_size(group))
        .unwrap_or(u64::MAX)
        .saturating_pow(3)
}

fn request_correlation_group_key(event: &NormalizedEvent) -> Option<RequestCorrelationGroupKey> {
    Some(RequestCorrelationGroupKey {
        request: event.request_key?,
        session: event.session_key,
        sidechain: event.is_sidechain,
        subagent: event.is_subagent,
    })
    .filter(|_| event.session_identity_present)
}

fn correlation_group_position(
    key: RequestCorrelationGroupKey,
    positions: &mut HashMap<RequestCorrelationGroupKey, usize>,
    groups: &mut Vec<RequestCorrelationGroup>,
) -> usize {
    *positions.entry(key).or_insert_with(|| {
        let position = groups.len();
        groups.push(RequestCorrelationGroup::default());
        position
    })
}

fn exact_request_matches(
    transcript_usage: &[&NormalizedEvent],
    otel_requests: &[&NormalizedEvent],
    group: &RequestCorrelationGroup,
) -> Option<Vec<(usize, usize)>> {
    let size = correlation_group_size(group);
    if size > MAX_REQUEST_CORRELATION_GROUP_EVENTS {
        return None;
    }

    let mut transcript_indices = group.transcript_indices.clone();
    transcript_indices.sort_by(|left, right| {
        canonical_request_observation_cmp(transcript_usage[*left], transcript_usage[*right])
    });
    let mut otel_indices = group.otel_indices.clone();
    otel_indices.sort_by(|left, right| {
        canonical_request_observation_cmp(otel_requests[*left], otel_requests[*right])
    });
    let transcript_count = transcript_indices.len();
    let otel_count = otel_indices.len();
    let size_i128 = i128::try_from(size).ok()?;
    let tie_scale = size_i128.saturating_pow(3).saturating_add(1);
    let maximum_real_cost = i128::try_from(REQUEST_CORRELATION_TOLERANCE_NANOS)
        .ok()?
        .saturating_mul(tie_scale)
        .saturating_add(size_i128.saturating_pow(2));
    let unmatched_penalty = maximum_real_cost
        .saturating_add(1)
        .saturating_mul(size_i128.saturating_add(1));
    let forbidden_cost = unmatched_penalty.saturating_mul(2);

    let assignment = minimum_cost_assignment(size, |row, column| {
        if row >= otel_count {
            return 0;
        }
        if column >= transcript_count {
            return unmatched_penalty;
        }
        let otel = otel_requests[otel_indices[row]];
        let transcript = transcript_usage[transcript_indices[column]];
        if !request_correlation_matches(transcript, otel) {
            return forbidden_cost;
        }
        let distance =
            i128::try_from(transcript.epoch_nanos.abs_diff(otel.epoch_nanos)).unwrap_or(i128::MAX);
        let tie_rank = i128::try_from(row.abs_diff(column)).unwrap_or(i128::MAX);
        distance.saturating_mul(tie_scale).saturating_add(tie_rank)
    });

    Some(
        assignment
            .into_iter()
            .enumerate()
            .filter_map(|(row, column)| {
                if row >= otel_count || column >= transcript_count {
                    return None;
                }
                let otel_index = otel_indices[row];
                let transcript_index = transcript_indices[column];
                request_correlation_matches(
                    transcript_usage[transcript_index],
                    otel_requests[otel_index],
                )
                .then_some((otel_index, transcript_index))
            })
            .collect(),
    )
}

fn canonical_request_observation_cmp(
    left: &NormalizedEvent,
    right: &NormalizedEvent,
) -> std::cmp::Ordering {
    left.epoch_nanos
        .cmp(&right.epoch_nanos)
        .then_with(|| left.project_alias.cmp(&right.project_alias))
        .then_with(|| left.parent_session_alias.cmp(&right.parent_session_alias))
        .then_with(|| left.model.cmp(&right.model))
        .then_with(|| left.pricing_modifier.cmp(&right.pricing_modifier))
        .then_with(|| left.tokens.input.cmp(&right.tokens.input))
        .then_with(|| left.tokens.output.cmp(&right.tokens.output))
        .then_with(|| left.tokens.cache_creation.cmp(&right.tokens.cache_creation))
        .then_with(|| left.tokens.cache_read.cmp(&right.tokens.cache_read))
        .then_with(|| {
            left.tokens
                .cache_creation_5m
                .cmp(&right.tokens.cache_creation_5m)
        })
        .then_with(|| {
            left.tokens
                .cache_creation_1h
                .cmp(&right.tokens.cache_creation_1h)
        })
        .then_with(|| optional_f64_total_cmp(left.source_cost_estimate, right.source_cost_estimate))
        .then_with(|| optional_f64_total_cmp(left.latency_ms, right.latency_ms))
        .then_with(|| left.error_count.cmp(&right.error_count))
        .then_with(|| left.retry_count.cmp(&right.retry_count))
        .then_with(|| left.tool_names.cmp(&right.tool_names))
        .then_with(|| left.tool_status.cmp(&right.tool_status))
        .then_with(|| left.source_alias.cmp(&right.source_alias))
        .then_with(|| left.file_alias.cmp(&right.file_alias))
        .then_with(|| left.record_index.cmp(&right.record_index))
}

fn optional_f64_total_cmp(left: Option<f64>, right: Option<f64>) -> std::cmp::Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.total_cmp(&right),
        (None, Some(_)) => std::cmp::Ordering::Less,
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    }
}

fn minimum_cost_assignment(size: usize, mut cost: impl FnMut(usize, usize) -> i128) -> Vec<usize> {
    let infinity = i128::MAX / 4;
    let mut row_potential = vec![0i128; size.saturating_add(1)];
    let mut column_potential = vec![0i128; size.saturating_add(1)];
    let mut column_row = vec![0usize; size.saturating_add(1)];
    let mut previous_column = vec![0usize; size.saturating_add(1)];

    for row in 1..=size {
        column_row[0] = row;
        let mut current_column = 0usize;
        let mut minimum = vec![infinity; size.saturating_add(1)];
        let mut used = vec![false; size.saturating_add(1)];
        loop {
            used[current_column] = true;
            let current_row = column_row[current_column];
            let mut delta = infinity;
            let mut next_column = 0usize;
            for column in 1..=size {
                if used[column] {
                    continue;
                }
                let reduced = cost(current_row - 1, column - 1)
                    .saturating_sub(row_potential[current_row])
                    .saturating_sub(column_potential[column]);
                if reduced < minimum[column] {
                    minimum[column] = reduced;
                    previous_column[column] = current_column;
                }
                if minimum[column] < delta {
                    delta = minimum[column];
                    next_column = column;
                }
            }
            for column in 0..=size {
                if used[column] {
                    row_potential[column_row[column]] =
                        row_potential[column_row[column]].saturating_add(delta);
                    column_potential[column] = column_potential[column].saturating_sub(delta);
                } else {
                    minimum[column] = minimum[column].saturating_sub(delta);
                }
            }
            current_column = next_column;
            if column_row[current_column] == 0 {
                break;
            }
        }
        loop {
            let prior = previous_column[current_column];
            column_row[current_column] = column_row[prior];
            current_column = prior;
            if current_column == 0 {
                break;
            }
        }
    }

    let mut assignment = vec![size; size];
    for column in 1..=size {
        if column_row[column] > 0 {
            assignment[column_row[column] - 1] = column - 1;
        }
    }
    assignment
}

fn request_correlation_matches(transcript: &NormalizedEvent, otel: &NormalizedEvent) -> bool {
    transcript.request_key.is_some()
        && transcript.request_key == otel.request_key
        && transcript.session_identity_present
        && otel.session_identity_present
        && transcript.session_key == otel.session_key
        && transcript.is_subagent == otel.is_subagent
        && transcript.is_sidechain == otel.is_sidechain
        && correlation_context_matches(transcript.agent_key, otel.agent_key)
        && correlation_context_matches(transcript.parent_agent_key, otel.parent_agent_key)
        && transcript.epoch_nanos.abs_diff(otel.epoch_nanos) <= REQUEST_CORRELATION_TOLERANCE_NANOS
}

fn warn_request_correlation_limit(diagnostics: &mut types::Diagnostics) {
    if diagnostics
        .warnings
        .iter()
        .any(|warning| warning.code == "W_AUTHORITY_CORRELATION_LIMIT")
    {
        return;
    }
    diagnostics.warning(
        "W_AUTHORITY_CORRELATION_LIMIT",
        "A repeated request-identity group exceeded bounded exact matching; authority/v1 kept transcript usage and reported telemetry observations as unresolved.",
        None,
    );
}

fn correlation_context_matches(left: Option<u64>, right: Option<u64>) -> bool {
    match (left, right) {
        (Some(left), Some(right)) => left == right,
        _ => true,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum CrossSourceUsageKey {
    Request {
        family: CrossSourceFactFamily,
        request: u64,
        session: u64,
        epoch_nanos: i128,
        sidechain: bool,
        subagent: bool,
        parent: Option<u64>,
        agent: Option<u64>,
    },
    Message {
        family: CrossSourceFactFamily,
        project: u64,
        session: u64,
        message: u64,
        epoch_nanos: i128,
        sidechain: bool,
        subagent: bool,
        parent: Option<u64>,
        agent: Option<u64>,
    },
    Exact(Box<CrossSourceExactKey>),
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct CrossSourceExactKey {
    schema_version: &'static str,
    adapter_version: &'static str,
    kind: types::EventKind,
    project: u64,
    project_identity_present: bool,
    session: u64,
    session_identity_present: bool,
    message: Option<u64>,
    request: Option<u64>,
    parent: Option<u64>,
    agent: Option<u64>,
    parent_agent: Option<u64>,
    skill: Option<u64>,
    plugin: Option<u64>,
    mcp_server: Option<u64>,
    mcp_tool: Option<u64>,
    timestamp: String,
    epoch_nanos: i128,
    timestamp_conversion_status: &'static str,
    model: Option<String>,
    model_mapping_status: &'static str,
    pricing_modifier: String,
    tokens: types::TokenFacts,
    cost_bits: Option<u64>,
    tools: Vec<String>,
    tool_status: Option<String>,
    latency_bits: Option<u64>,
    error_count: Option<u64>,
    retry_count: Option<u64>,
    edit_decision: Option<String>,
    compaction: Option<bool>,
    metric_name: Option<&'static str>,
    metric_value_bits: Option<u64>,
    metric_unit: Option<&'static str>,
    metric_interval_start_nanos: Option<u64>,
    metric_interval_end_nanos: Option<u64>,
    metric_temporality: Option<u64>,
    redacted_fields: usize,
    sidechain: bool,
    subagent: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum CrossSourceFactFamily {
    Usage,
    Prompt,
    ToolResult,
    Progress,
    Summary,
    System,
    Compaction,
    ApiError,
    ToolDecision,
    Metric,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum AppendAuthorityKey {
    Request {
        family: CrossSourceFactFamily,
        request: u64,
        session: u64,
        epoch_nanos: i128,
        sidechain: bool,
        subagent: bool,
        parent: Option<u64>,
        agent: Option<u64>,
    },
    Message {
        family: CrossSourceFactFamily,
        project: u64,
        session: u64,
        message: u64,
        epoch_nanos: i128,
        sidechain: bool,
        subagent: bool,
        parent: Option<u64>,
        agent: Option<u64>,
    },
}

fn append_authority_key(event: &NormalizedEvent) -> Option<AppendAuthorityKey> {
    let family = cross_source_fact_family(event.kind);
    if let Some(request) = event.request_key {
        return Some(AppendAuthorityKey::Request {
            family,
            request,
            session: event.session_key,
            epoch_nanos: event.epoch_nanos,
            sidechain: event.is_sidechain,
            subagent: event.is_subagent,
            parent: event.parent_key,
            agent: event.agent_key,
        });
    }
    (event.kind != types::EventKind::OtelMetric)
        .then_some(event.message_key)
        .flatten()
        .map(|message| AppendAuthorityKey::Message {
            family,
            project: event.project_key,
            session: event.session_key,
            message,
            epoch_nanos: event.epoch_nanos,
            sidechain: event.is_sidechain,
            subagent: event.is_subagent,
            parent: event.parent_key,
            agent: event.agent_key,
        })
}

fn cross_source_fact_family(kind: types::EventKind) -> CrossSourceFactFamily {
    match kind {
        types::EventKind::AssistantUsage | types::EventKind::OtelApiRequest => {
            CrossSourceFactFamily::Usage
        }
        types::EventKind::UserPrompt => CrossSourceFactFamily::Prompt,
        types::EventKind::ToolResult | types::EventKind::OtelToolResult => {
            CrossSourceFactFamily::ToolResult
        }
        types::EventKind::Progress => CrossSourceFactFamily::Progress,
        types::EventKind::Summary => CrossSourceFactFamily::Summary,
        types::EventKind::System => CrossSourceFactFamily::System,
        types::EventKind::Compaction => CrossSourceFactFamily::Compaction,
        types::EventKind::OtelApiError => CrossSourceFactFamily::ApiError,
        types::EventKind::OtelToolDecision => CrossSourceFactFamily::ToolDecision,
        types::EventKind::OtelMetric => CrossSourceFactFamily::Metric,
    }
}

fn cross_source_usage_key(event: &NormalizedEvent) -> CrossSourceUsageKey {
    let family = cross_source_fact_family(event.kind);
    if let Some(request) = event.request_key {
        return CrossSourceUsageKey::Request {
            family,
            request,
            session: event.session_key,
            epoch_nanos: event.epoch_nanos,
            sidechain: event.is_sidechain,
            subagent: event.is_subagent,
            parent: event.parent_key,
            agent: event.agent_key,
        };
    }
    if event.kind != types::EventKind::OtelMetric {
        if let Some(message) = event.message_key {
            return CrossSourceUsageKey::Message {
                family,
                project: event.project_key,
                session: event.session_key,
                message,
                epoch_nanos: event.epoch_nanos,
                sidechain: event.is_sidechain,
                subagent: event.is_subagent,
                parent: event.parent_key,
                agent: event.agent_key,
            };
        }
    }
    CrossSourceUsageKey::Exact(Box::new(CrossSourceExactKey {
        schema_version: event.schema_version,
        adapter_version: event.adapter_version,
        kind: event.kind,
        project: event.project_key,
        project_identity_present: event.project_identity_present,
        session: event.session_key,
        session_identity_present: event.session_identity_present,
        message: event.message_key,
        request: event.request_key,
        parent: event.parent_key,
        agent: event.agent_key,
        parent_agent: event.parent_agent_key,
        skill: event.skill_key,
        plugin: event.plugin_key,
        mcp_server: event.mcp_server_key,
        mcp_tool: event.mcp_tool_key,
        timestamp: event.timestamp.clone(),
        epoch_nanos: event.epoch_nanos,
        timestamp_conversion_status: event.timestamp_conversion_status,
        model: event.model.clone(),
        model_mapping_status: event.model_mapping_status,
        pricing_modifier: event.pricing_modifier.clone(),
        tokens: event.tokens.clone(),
        cost_bits: event.source_cost_estimate.map(f64::to_bits),
        tools: event.tool_names.clone(),
        tool_status: event.tool_status.clone(),
        latency_bits: event.latency_ms.map(f64::to_bits),
        error_count: event.error_count,
        retry_count: event.retry_count,
        edit_decision: event.edit_decision.clone(),
        compaction: event.compaction,
        metric_name: event.metric_name,
        metric_value_bits: event.metric_value.map(f64::to_bits),
        metric_unit: event.metric_unit,
        metric_interval_start_nanos: event.metric_interval_start_nanos,
        metric_interval_end_nanos: event.metric_interval_end_nanos,
        metric_temporality: event.metric_temporality,
        redacted_fields: event.redacted_fields,
        sidechain: event.is_sidechain,
        subagent: event.is_subagent,
    }))
}

fn collapse_cross_source(candidates: Vec<&NormalizedEvent>) -> (Vec<&NormalizedEvent>, usize) {
    if candidates.first().is_none_or(|first| {
        candidates
            .iter()
            .all(|candidate| candidate.source_alias == first.source_alias)
    }) {
        return (candidates, 0);
    }
    let mut positions: HashMap<CrossSourceUsageKey, usize> =
        HashMap::with_capacity(candidates.len());
    let mut selected = Vec::with_capacity(candidates.len());
    let mut resolved = 0usize;
    for candidate in candidates {
        let key = cross_source_usage_key(candidate);
        if let Some(position) = positions.get(&key).copied() {
            resolved = resolved.saturating_add(1);
            let current: &NormalizedEvent = selected[position];
            if duplicate_preference_cmp(candidate, current).is_gt() {
                selected[position] = candidate;
            }
        } else {
            positions.insert(key, selected.len());
            selected.push(candidate);
        }
    }
    (selected, resolved)
}

fn record_resolved_overlap(diagnostics: &mut types::Diagnostics, count: usize) {
    diagnostics.resolved_overlap_records =
        diagnostics.resolved_overlap_records.saturating_add(count);
    if !diagnostics
        .warnings
        .iter()
        .any(|warning| warning.code == "W_AUTHORITY_RESOLVED_OVERLAP")
    {
        diagnostics.warning(
            "W_AUTHORITY_RESOLVED_OVERLAP",
            "Strong source-native identities resolved repeated observations across selected roots without summing.",
            None,
        );
    }
}

fn sort_events(events: &mut [NormalizedEvent]) {
    events.sort_by(event_order_cmp);
}

fn deduplicate(
    candidates: Vec<NormalizedEvent>,
    diagnostics: &mut types::Diagnostics,
) -> Vec<NormalizedEvent> {
    let source_indices = diagnostics
        .sources
        .keys()
        .enumerate()
        .map(|(index, alias)| (alias.clone(), index))
        .collect::<HashMap<_, _>>();
    let mut positions: HashMap<DedupKey, usize> = HashMap::with_capacity(candidates.len());
    let mut events: Vec<NormalizedEvent> = Vec::with_capacity(candidates.len());
    let mut last_source_alias = String::new();
    let mut last_source_index = 0usize;
    for candidate in candidates {
        let source_index = if candidate.source_alias == last_source_alias {
            last_source_index
        } else {
            let index = source_indices
                .get(&candidate.source_alias)
                .copied()
                .expect("normalized event source aliases must be registered");
            last_source_alias.clone_from(&candidate.source_alias);
            last_source_index = index;
            index
        };
        let key = candidate.dedup_key(source_index);
        if let Some(position) = positions.get(&key).copied() {
            diagnostics.duplicate_records = diagnostics.duplicate_records.saturating_add(1);
            if let Some(stats) = diagnostics.sources.get_mut(&candidate.source_alias) {
                stats.duplicate_records = stats.duplicate_records.saturating_add(1);
            }
            if duplicate_preference_cmp(&candidate, &events[position]).is_gt() {
                events[position] = candidate;
            }
        } else {
            positions.insert(key, events.len());
            events.push(candidate);
        }
    }
    events.sort_by(event_order_cmp);
    events
}

fn event_order_cmp(left: &NormalizedEvent, right: &NormalizedEvent) -> std::cmp::Ordering {
    left.epoch_nanos
        .cmp(&right.epoch_nanos)
        .then_with(|| left.source_alias.cmp(&right.source_alias))
        .then_with(|| left.file_alias.cmp(&right.file_alias))
        .then_with(|| left.record_index.cmp(&right.record_index))
}

fn duplicate_preference_cmp(left: &NormalizedEvent, right: &NormalizedEvent) -> std::cmp::Ordering {
    left.richness()
        .cmp(&right.richness())
        // Preserve conservative producer-declared uncertainty when duplicate observations are
        // otherwise equally informative.
        .then_with(|| {
            left.attribute_evidence_uncertain
                .cmp(&right.attribute_evidence_uncertain)
        })
        // A smaller canonical fact vector wins when observations are equally rich. Reverse the
        // comparison here so `Greater` consistently means that `left` is preferred.
        .then_with(|| canonical_duplicate_fact_cmp(right, left))
}

fn canonical_duplicate_fact_cmp(
    left: &NormalizedEvent,
    right: &NormalizedEvent,
) -> std::cmp::Ordering {
    left.schema_version
        .cmp(right.schema_version)
        .then_with(|| left.adapter_version.cmp(right.adapter_version))
        .then_with(|| {
            left.timestamp_conversion_status
                .cmp(right.timestamp_conversion_status)
        })
        .then_with(|| left.timestamp.cmp(&right.timestamp))
        .then_with(|| {
            left.project_identity_present
                .cmp(&right.project_identity_present)
        })
        .then_with(|| {
            left.session_identity_present
                .cmp(&right.session_identity_present)
        })
        .then_with(|| left.message_key.is_some().cmp(&right.message_key.is_some()))
        .then_with(|| left.request_key.is_some().cmp(&right.request_key.is_some()))
        .then_with(|| left.model_mapping_status.cmp(right.model_mapping_status))
        .then_with(|| left.model.cmp(&right.model))
        .then_with(|| left.pricing_modifier.cmp(&right.pricing_modifier))
        .then_with(|| left.tokens.input.cmp(&right.tokens.input))
        .then_with(|| left.tokens.output.cmp(&right.tokens.output))
        .then_with(|| left.tokens.cache_creation.cmp(&right.tokens.cache_creation))
        .then_with(|| left.tokens.cache_read.cmp(&right.tokens.cache_read))
        .then_with(|| {
            left.tokens
                .cache_creation_5m
                .cmp(&right.tokens.cache_creation_5m)
        })
        .then_with(|| {
            left.tokens
                .cache_creation_1h
                .cmp(&right.tokens.cache_creation_1h)
        })
        .then_with(|| optional_f64_total_cmp(left.source_cost_estimate, right.source_cost_estimate))
        .then_with(|| left.tool_names.cmp(&right.tool_names))
        .then_with(|| left.tool_status.cmp(&right.tool_status))
        .then_with(|| optional_f64_total_cmp(left.latency_ms, right.latency_ms))
        .then_with(|| left.error_count.cmp(&right.error_count))
        .then_with(|| left.retry_count.cmp(&right.retry_count))
        .then_with(|| left.edit_decision.cmp(&right.edit_decision))
        .then_with(|| left.compaction.cmp(&right.compaction))
        .then_with(|| left.metric_name.cmp(&right.metric_name))
        .then_with(|| optional_f64_total_cmp(left.metric_value, right.metric_value))
        .then_with(|| left.metric_unit.cmp(&right.metric_unit))
        .then_with(|| {
            left.metric_interval_start_nanos
                .cmp(&right.metric_interval_start_nanos)
        })
        .then_with(|| {
            left.metric_interval_end_nanos
                .cmp(&right.metric_interval_end_nanos)
        })
        .then_with(|| left.metric_temporality.cmp(&right.metric_temporality))
        .then_with(|| left.redacted_fields.cmp(&right.redacted_fields))
}

#[derive(Debug, Clone, Copy, Default)]
struct ObservedCapabilities {
    token_usage: bool,
    cache_ttl_tokens: bool,
    prompt_occurrence: bool,
    tool_occurrence: bool,
    otel_telemetry: bool,
    api_request: bool,
    api_error: bool,
    api_request_count: usize,
    terminal_outcome_count: usize,
    retry_evidence_count: usize,
    api_request_attribute_uncertain_count: usize,
    tool_result_count: usize,
    tool_status_count: usize,
    tool_latency_count: usize,
    tool_result_attribute_uncertain_count: usize,
    tool_decision_count: usize,
    edit_decision_count: usize,
    tool_decision_attribute_uncertain_count: usize,
    compaction: bool,
    metric_bits: u8,
}

impl ObservedCapabilities {
    fn from_event(event: &NormalizedEvent) -> Self {
        let metric_bits =
            metric_capability_index(event.metric_name).map_or(0, |index| 1u8 << index);
        Self {
            token_usage: event.tokens.richness() > 0,
            cache_ttl_tokens: event.tokens.cache_creation_5m.is_some()
                || event.tokens.cache_creation_1h.is_some(),
            prompt_occurrence: event.kind == types::EventKind::UserPrompt,
            tool_occurrence: event.kind == types::EventKind::AssistantUsage
                && !event.tool_names.is_empty(),
            otel_telemetry: event.adapter_version == types::OTEL_ADAPTER,
            api_request: event.kind == types::EventKind::OtelApiRequest,
            api_error: event.kind == types::EventKind::OtelApiError,
            api_request_count: usize::from(event.kind == types::EventKind::OtelApiRequest),
            terminal_outcome_count: usize::from(matches!(
                event.kind,
                types::EventKind::OtelApiRequest | types::EventKind::OtelApiError
            )),
            retry_evidence_count: usize::from(
                event.kind == types::EventKind::OtelApiRequest && event.retry_count.is_some(),
            ),
            api_request_attribute_uncertain_count: usize::from(
                event.kind == types::EventKind::OtelApiRequest
                    && event.attribute_evidence_uncertain,
            ),
            tool_result_count: usize::from(event.kind == types::EventKind::OtelToolResult),
            tool_status_count: usize::from(
                event.kind == types::EventKind::OtelToolResult && event.tool_status.is_some(),
            ),
            tool_latency_count: usize::from(
                event.kind == types::EventKind::OtelToolResult && event.latency_ms.is_some(),
            ),
            tool_result_attribute_uncertain_count: usize::from(
                event.kind == types::EventKind::OtelToolResult
                    && event.attribute_evidence_uncertain,
            ),
            tool_decision_count: usize::from(event.kind == types::EventKind::OtelToolDecision),
            edit_decision_count: usize::from(
                event.kind == types::EventKind::OtelToolDecision && event.edit_decision.is_some(),
            ),
            tool_decision_attribute_uncertain_count: usize::from(
                event.kind == types::EventKind::OtelToolDecision
                    && event.attribute_evidence_uncertain,
            ),
            compaction: event.compaction.is_some(),
            metric_bits,
        }
    }

    fn merge(&mut self, observed: Self) {
        self.token_usage |= observed.token_usage;
        self.cache_ttl_tokens |= observed.cache_ttl_tokens;
        self.prompt_occurrence |= observed.prompt_occurrence;
        self.tool_occurrence |= observed.tool_occurrence;
        self.otel_telemetry |= observed.otel_telemetry;
        self.api_request |= observed.api_request;
        self.api_error |= observed.api_error;
        self.api_request_count = self
            .api_request_count
            .saturating_add(observed.api_request_count);
        self.terminal_outcome_count = self
            .terminal_outcome_count
            .saturating_add(observed.terminal_outcome_count);
        self.retry_evidence_count = self
            .retry_evidence_count
            .saturating_add(observed.retry_evidence_count);
        self.api_request_attribute_uncertain_count = self
            .api_request_attribute_uncertain_count
            .saturating_add(observed.api_request_attribute_uncertain_count);
        self.tool_result_count = self
            .tool_result_count
            .saturating_add(observed.tool_result_count);
        self.tool_status_count = self
            .tool_status_count
            .saturating_add(observed.tool_status_count);
        self.tool_latency_count = self
            .tool_latency_count
            .saturating_add(observed.tool_latency_count);
        self.tool_result_attribute_uncertain_count = self
            .tool_result_attribute_uncertain_count
            .saturating_add(observed.tool_result_attribute_uncertain_count);
        self.tool_decision_count = self
            .tool_decision_count
            .saturating_add(observed.tool_decision_count);
        self.edit_decision_count = self
            .edit_decision_count
            .saturating_add(observed.edit_decision_count);
        self.tool_decision_attribute_uncertain_count = self
            .tool_decision_attribute_uncertain_count
            .saturating_add(observed.tool_decision_attribute_uncertain_count);
        self.compaction |= observed.compaction;
        self.metric_bits |= observed.metric_bits;
    }

    fn write(
        self,
        capabilities: &mut std::collections::BTreeMap<String, String>,
        otel: bool,
        event_shape_uncertain: bool,
    ) {
        for (name, available) in [
            ("token_usage", self.token_usage),
            ("cache_ttl_tokens", self.cache_ttl_tokens),
            ("prompt_occurrence", self.prompt_occurrence),
            ("tool_occurrence", self.tool_occurrence),
            ("compaction", self.compaction),
        ] {
            capabilities.insert(name.to_string(), capability_status(available));
        }
        for (name, available) in [
            ("api_request", self.api_request),
            ("api_error", self.api_error),
            ("tool_result", self.tool_result_count > 0),
            ("tool_decision", self.tool_decision_count > 0),
        ] {
            capabilities.insert(
                name.to_string(),
                observed_direct_status(available, event_shape_uncertain),
            );
        }
        for (name, total, present, uncertain_count) in [
            (
                "direct_terminal_outcomes",
                self.terminal_outcome_count,
                self.terminal_outcome_count,
                0,
            ),
            (
                "retry_evidence",
                self.api_request_count,
                self.retry_evidence_count,
                self.api_request_attribute_uncertain_count,
            ),
            (
                "tool_status",
                self.tool_result_count,
                self.tool_status_count,
                self.tool_result_attribute_uncertain_count,
            ),
            (
                "tool_latency",
                self.tool_result_count,
                self.tool_latency_count,
                self.tool_result_attribute_uncertain_count,
            ),
            (
                "edit_decision",
                self.tool_decision_count,
                self.edit_decision_count,
                self.tool_decision_attribute_uncertain_count,
            ),
        ] {
            let uncertain = event_shape_uncertain || uncertain_count > 0;
            capabilities.insert(
                name.to_string(),
                direct_fact_status(total, present, uncertain),
            );
        }
        if otel {
            capabilities.insert(
                "otel_telemetry".to_string(),
                capability_status(self.otel_telemetry),
            );
        }
        capabilities.insert("content".to_string(), "excluded".to_string());
        for (index, (capability, _)) in METRIC_CAPABILITIES.into_iter().enumerate() {
            capabilities.insert(
                capability.to_string(),
                capability_status(self.metric_bits & (1u8 << index) != 0),
            );
        }
    }
}

fn metric_capability_index(metric_name: Option<&str>) -> Option<usize> {
    match metric_name {
        Some("session-count") => Some(0),
        Some("lines-of-code") => Some(1),
        Some("pull-requests") => Some(2),
        Some("commits") => Some(3),
        Some("source-cost-estimate") => Some(4),
        Some("token-usage") => Some(5),
        Some("code-edit-decision") => Some(6),
        Some("active-time") => Some(7),
        _ => None,
    }
}

fn capability_status(available: bool) -> String {
    if available {
        "available"
    } else {
        "unavailable"
    }
    .to_string()
}

fn observed_direct_status(available: bool, uncertain: bool) -> String {
    if !available {
        "unavailable"
    } else if uncertain {
        "partial"
    } else {
        "available"
    }
    .to_string()
}

fn direct_fact_status(total: usize, present: usize, uncertain: bool) -> String {
    let status = fact_status(total, present);
    if uncertain && status == "available" {
        "partial"
    } else {
        status
    }
    .to_string()
}

#[derive(Debug, Default)]
struct SourceCapabilityObservation {
    accepted_records: usize,
    earliest: Option<(i128, String)>,
    latest: Option<(i128, String)>,
    capabilities: ObservedCapabilities,
}

impl SourceCapabilityObservation {
    fn observe(&mut self, event: &NormalizedEvent, capabilities: ObservedCapabilities) {
        self.accepted_records = self.accepted_records.saturating_add(1);
        if self
            .earliest
            .as_ref()
            .is_none_or(|(epoch, _)| event.epoch_nanos < *epoch)
        {
            self.earliest = Some((event.epoch_nanos, event.timestamp.clone()));
        }
        if self
            .latest
            .as_ref()
            .is_none_or(|(epoch, _)| event.epoch_nanos > *epoch)
        {
            self.latest = Some((event.epoch_nanos, event.timestamp.clone()));
        }
        self.capabilities.merge(capabilities);
    }
}

#[derive(Debug)]
struct CapabilityObservation {
    accepted_records: usize,
    global: ObservedCapabilities,
    by_source: HashMap<String, SourceCapabilityObservation>,
}

fn compute_capability_observation(
    events: &[NormalizedEvent],
    source_aliases: &[String],
) -> CapabilityObservation {
    debug_assert!(events.iter().all(|event| {
        event.schema_version == types::NORMALIZED_SCHEMA
            && event.timestamp_conversion_status == "normalized-utc"
            && matches!(event.model_mapping_status, "missing" | "unmapped")
    }));
    let mut global = ObservedCapabilities::default();
    let mut by_source = source_aliases
        .iter()
        .map(|alias| (alias.clone(), SourceCapabilityObservation::default()))
        .collect::<HashMap<_, _>>();
    for event in events {
        #[cfg(test)]
        SOURCE_CAPABILITY_EVENT_VISITS.with(|visits| visits.set(visits.get().saturating_add(1)));
        let observed = ObservedCapabilities::from_event(event);
        global.merge(observed);
        if let Some(source) = by_source.get_mut(&event.source_alias) {
            source.observe(event, observed);
        } else {
            debug_assert!(false, "normalized event source must have diagnostics");
        }
    }
    CapabilityObservation {
        accepted_records: events.len(),
        global,
        by_source,
    }
}

fn apply_capability_observation(
    mut observation: CapabilityObservation,
    diagnostics: &mut types::Diagnostics,
) {
    diagnostics.accepted_records = observation.accepted_records;
    let event_shape_uncertain_sources = diagnostics
        .sources
        .values()
        .filter(|source| {
            source.kind == "otel" && (source.malformed_records > 0 || source.skipped_records > 0)
        })
        .map(|source| source.alias.clone())
        .collect::<HashSet<_>>();
    observation.global.write(
        &mut diagnostics.capabilities,
        true,
        !event_shape_uncertain_sources.is_empty(),
    );
    for (alias, stats) in &mut diagnostics.sources {
        let source = observation.by_source.remove(alias).unwrap_or_default();
        stats.accepted_records = source.accepted_records;
        if let Some((epoch, timestamp)) = source.earliest {
            stats.observe_time(epoch, &timestamp);
        }
        if let Some((epoch, timestamp)) = source.latest {
            stats.observe_time(epoch, &timestamp);
        }
        source.capabilities.write(
            &mut stats.capabilities,
            false,
            event_shape_uncertain_sources.contains(alias),
        );
    }
}

fn record_capabilities(events: &[NormalizedEvent], diagnostics: &mut types::Diagnostics) {
    debug_assert!(
        events
            .iter()
            .map(|event| event.redacted_fields)
            .fold(0usize, usize::saturating_add)
            <= diagnostics.redacted_fields
    );
    let source_aliases = diagnostics.sources.keys().cloned().collect::<Vec<_>>();
    let observation = compute_capability_observation(events, &source_aliases);
    apply_capability_observation(observation, diagnostics);
}

fn record_analytical_capabilities(
    canonical_events: &[NormalizedEvent],
    diagnostics: &mut types::Diagnostics,
) {
    let direct_usage_events = canonical_events
        .iter()
        .filter(|event| {
            matches!(
                event.kind,
                types::EventKind::AssistantUsage | types::EventKind::OtelApiRequest
            )
        })
        .collect::<Vec<_>>();
    let token_metric_events = canonical_events
        .iter()
        .filter(|event| event.kind == types::EventKind::OtelMetric && event.tokens.richness() > 0)
        .collect::<Vec<_>>();
    let usage_attribute_evidence_uncertain = direct_usage_events
        .iter()
        .chain(token_metric_events.iter())
        .any(|event| event.attribute_evidence_uncertain);

    let mut token_sums = [0u64; 4];
    let mut saturated_categories = 0u8;
    for event in direct_usage_events.iter().chain(token_metric_events.iter()) {
        for (index, value) in [
            event.tokens.input,
            event.tokens.output,
            event.tokens.cache_creation,
            event.tokens.cache_read,
        ]
        .into_iter()
        .enumerate()
        {
            let Some(value) = value else {
                continue;
            };
            if let Some(sum) = token_sums[index].checked_add(value) {
                token_sums[index] = sum;
            } else {
                token_sums[index] = u64::MAX;
                saturated_categories |= 1 << index;
            }
        }
    }
    if saturated_categories != 0 {
        diagnostics.excluded_analysis_token_categories |= saturated_categories;
        diagnostics.warning(
            "W_ANALYTICAL_TOKEN_SATURATED",
            "One or more canonical token totals exceeded the report integer range; observed sums were clamped and affected analytical capabilities are partial.",
            None,
        );
    }

    let mut token_statuses = [
        (
            "analysis_input_tokens",
            canonical_token_status(&direct_usage_events, &token_metric_events, |event| {
                event.tokens.input
            }),
        ),
        (
            "analysis_output_tokens",
            canonical_token_status(&direct_usage_events, &token_metric_events, |event| {
                event.tokens.output
            }),
        ),
        (
            "analysis_cache_creation_tokens",
            canonical_token_status(&direct_usage_events, &token_metric_events, |event| {
                event.tokens.cache_creation
            }),
        ),
        (
            "analysis_cache_read_tokens",
            canonical_token_status(&direct_usage_events, &token_metric_events, |event| {
                event.tokens.cache_read
            }),
        ),
    ];
    for (index, (_, status)) in token_statuses.iter_mut().enumerate() {
        if diagnostics.excluded_analysis_token_categories & (1 << index) != 0 {
            *status = "partial";
        }
    }
    if diagnostics.analytical_claims_uncertain || usage_attribute_evidence_uncertain {
        for (_, status) in &mut token_statuses {
            *status = "partial";
        }
    }
    for (capability, status) in token_statuses {
        diagnostics
            .capabilities
            .insert(capability.to_string(), status.to_string());
    }

    let token_status_values = token_statuses.map(|(_, status)| status);
    let usage_status = usage_status_from_categories(&token_status_values);
    diagnostics.capabilities.insert(
        "analysis_usage_totals".to_string(),
        usage_status.to_string(),
    );

    let direct_cost_supported = direct_usage_events
        .iter()
        .filter(|event| {
            event.source_cost_estimate.is_some_and(|cost| cost > 0.0)
                || (event.source_cost_estimate == Some(0.0) && primary_token_sum(event) == 0)
                || (event.source_cost_estimate.is_none()
                    && event.model.is_some()
                    && has_complete_primary_tokens(event))
        })
        .count();
    let source_cost_metrics = canonical_events
        .iter()
        .filter(|event| {
            event.kind == types::EventKind::OtelMetric && event.source_cost_estimate.is_some()
        })
        .collect::<Vec<_>>();
    let cost_attribute_evidence_uncertain = usage_attribute_evidence_uncertain
        || source_cost_metrics
            .iter()
            .any(|event| event.attribute_evidence_uncertain);
    let direct_cost_status = fact_status(direct_usage_events.len(), direct_cost_supported);
    let metric_token_statuses = [
        canonical_token_status(&[], &token_metric_events, |event| event.tokens.input),
        canonical_token_status(&[], &token_metric_events, |event| event.tokens.output),
        canonical_token_status(&[], &token_metric_events, |event| {
            event.tokens.cache_creation
        }),
        canonical_token_status(&[], &token_metric_events, |event| event.tokens.cache_read),
    ];
    let metric_usage_status = usage_status_from_categories(&metric_token_statuses);
    let metric_cost_status = if !source_cost_metrics.is_empty() && !token_metric_events.is_empty() {
        "unavailable"
    } else if !source_cost_metrics.is_empty()
        || (!token_metric_events.is_empty()
            && metric_usage_status == "available"
            && token_metric_events
                .iter()
                .all(|event| event.model.is_some()))
    {
        "available"
    } else if !token_metric_events.is_empty() {
        "partial"
    } else {
        "unavailable"
    };
    let metric_cost_family_present =
        !source_cost_metrics.is_empty() || !token_metric_events.is_empty();
    let mut cost_status = if !source_cost_metrics.is_empty() && !token_metric_events.is_empty() {
        "unavailable"
    } else {
        match (!direct_usage_events.is_empty(), metric_cost_family_present) {
            (true, true) => combine_fact_status(direct_cost_status, metric_cost_status),
            (true, false) => direct_cost_status,
            (false, true) => metric_cost_status,
            (false, false) => "unavailable",
        }
    };
    if (diagnostics.excluded_analysis_cost || diagnostics.excluded_analysis_token_categories != 0)
        && !(!source_cost_metrics.is_empty() && !token_metric_events.is_empty())
    {
        cost_status = "partial";
    }
    if (diagnostics.analytical_claims_uncertain || cost_attribute_evidence_uncertain)
        && !(!source_cost_metrics.is_empty() && !token_metric_events.is_empty())
    {
        cost_status = "partial";
    }
    diagnostics
        .capabilities
        .insert("analysis_cost".to_string(), cost_status.to_string());
    diagnostics.analytical_cost_coverage = Some(
        if !source_cost_metrics.is_empty() && !token_metric_events.is_empty() {
            "unavailable-conflicting-cost-bases"
        } else if cost_status == "available"
            && (direct_usage_events
                .iter()
                .any(|event| event.source_cost_estimate.is_some())
                || !source_cost_metrics.is_empty())
        {
            "source-recorded-estimate-and-local-computation"
        } else if cost_status == "available" {
            "local-computation-with-unpriced-possibility"
        } else if cost_status == "partial" {
            "partial-observed-cost-evidence"
        } else {
            "unavailable-incomplete-usage"
        },
    );

    let cache_has_denominator = direct_usage_events
        .iter()
        .chain(token_metric_events.iter())
        .any(|event| {
            event.tokens.input.unwrap_or(0) > 0
                || event.tokens.cache_creation.unwrap_or(0) > 0
                || event.tokens.cache_read.unwrap_or(0) > 0
                || event.tokens.output.unwrap_or(0) > 0
        });
    diagnostics.capabilities.insert(
        "analysis_cache_health".to_string(),
        if usage_status == "available" && cache_has_denominator {
            "available"
        } else {
            "unavailable"
        }
        .to_string(),
    );

    diagnostics.saw_source_cost = direct_usage_events
        .iter()
        .chain(source_cost_metrics.iter())
        .any(|event| event.source_cost_estimate.is_some());
}

fn has_complete_primary_tokens(event: &NormalizedEvent) -> bool {
    event.tokens.input.is_some()
        && event.tokens.output.is_some()
        && event.tokens.cache_creation.is_some()
        && event.tokens.cache_read.is_some()
}

fn primary_token_sum(event: &NormalizedEvent) -> u64 {
    [
        event.tokens.input,
        event.tokens.output,
        event.tokens.cache_creation,
        event.tokens.cache_read,
    ]
    .into_iter()
    .flatten()
    .fold(0u64, u64::saturating_add)
}

fn canonical_token_status(
    direct_events: &[&NormalizedEvent],
    metric_events: &[&NormalizedEvent],
    token: impl Fn(&NormalizedEvent) -> Option<u64>,
) -> &'static str {
    let total = direct_events
        .len()
        .saturating_add(usize::from(!metric_events.is_empty()));
    let present = direct_events
        .iter()
        .filter(|event| token(event).is_some())
        .count()
        .saturating_add(usize::from(
            metric_events.iter().any(|event| token(event).is_some()),
        ));
    fact_status(total, present)
}

fn usage_status_from_categories(statuses: &[&str; 4]) -> &'static str {
    if statuses.iter().all(|status| *status == "available") {
        "available"
    } else if statuses.iter().any(|status| *status != "unavailable") {
        "partial"
    } else {
        "unavailable"
    }
}

fn combine_fact_status(left: &'static str, right: &'static str) -> &'static str {
    match (left, right) {
        ("available", "available") => "available",
        ("unavailable", "unavailable") => "unavailable",
        _ => "partial",
    }
}

fn fact_status(total: usize, present: usize) -> &'static str {
    if total == 0 || present == 0 {
        "unavailable"
    } else if present == total {
        "available"
    } else {
        "partial"
    }
}

#[cfg(test)]
mod capability_tests {
    use super::*;
    use types::{EventKind, SourceStats, TokenFacts};

    #[test]
    fn ingestion_execution_policy_uses_the_measured_throughput_plateau_and_validates_overrides() {
        assert_eq!(select_worker_count(None, 16).unwrap(), 12);
        assert_eq!(select_worker_count(None, 12).unwrap(), 12);
        assert_eq!(select_worker_count(None, 8).unwrap(), 8);
        assert_eq!(select_worker_count(None, 2).unwrap(), 2);
        assert_eq!(select_worker_count(None, 1).unwrap(), 1);
        assert_eq!(select_worker_count(Some(8), 16).unwrap(), 8);
        assert!(select_worker_count(Some(0), 16).is_err());
        assert!(select_worker_count(Some(17), 16).is_err());
    }

    fn synthetic_event(source_alias: String, record_index: u64) -> NormalizedEvent {
        NormalizedEvent {
            schema_version: types::NORMALIZED_SCHEMA,
            adapter_version: types::TRANSCRIPT_ADAPTER,
            source_alias,
            file_alias: "file-1".to_string(),
            record_index,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            epoch_nanos: i128::from(record_index),
            timestamp_conversion_status: "normalized-utc",
            project_key: 1,
            project_identity_present: true,
            session_key: record_index,
            session_identity_present: true,
            message_key: Some(record_index),
            request_key: None,
            parent_key: None,
            agent_key: None,
            parent_agent_key: None,
            skill_key: None,
            plugin_key: None,
            mcp_server_key: None,
            mcp_tool_key: None,
            observation_key: record_index,
            project_alias: "project-1".to_string(),
            session_alias: format!("session-{record_index}"),
            parent_session_alias: None,
            is_subagent: false,
            is_sidechain: false,
            kind: EventKind::AssistantUsage,
            model: Some("claude-sonnet-4-6".to_string()),
            model_mapping_status: "unmapped",
            pricing_modifier: "standard".to_string(),
            tokens: TokenFacts {
                input: Some(1),
                output: Some(1),
                cache_creation: Some(0),
                cache_read: Some(0),
                cache_creation_5m: None,
                cache_creation_1h: None,
            },
            source_cost_estimate: None,
            tool_names: Vec::new(),
            tool_status: None,
            latency_ms: None,
            error_count: None,
            retry_count: None,
            edit_decision: None,
            compaction: None,
            metric_name: None,
            metric_value: None,
            metric_unit: None,
            metric_interval_start_nanos: None,
            metric_interval_end_nanos: None,
            metric_temporality: None,
            metric_family_key: None,
            attribute_evidence_uncertain: false,
            redacted_fields: 0,
        }
    }

    #[test]
    fn production_reconciliation_gate_rejects_a_perturbed_cost_projection() {
        let time_context = TimeContext::new("UTC", Some(2026)).expect("valid test timezone");
        let mut event = synthetic_event("transcript-1".to_string(), 1);
        event.epoch_nanos = 1_767_225_600_000_000_000;
        event.timestamp = "2026-01-01T00:00:00Z".to_string();
        let mut projection = views::build_canonical_projection(&[event], &time_context, 300, 1)
            .expect("synthetic projection");
        validate_projection(&projection).expect("unmodified projection must reconcile");

        views::perturb_cost_projection_for_test(&mut projection);
        let error = validate_projection(&projection)
            .expect_err("a perturbed production cost projection must be rejected");
        assert_eq!(error.code(), "E_METRIC_RECONCILIATION");
    }

    #[test]
    fn production_reconciliation_gate_rejects_cost_token_domain_drift() {
        let time_context = TimeContext::new("UTC", Some(2026)).expect("valid test timezone");
        let mut event = synthetic_event("transcript-1".to_string(), 1);
        event.epoch_nanos = 1_767_225_600_000_000_000;
        event.timestamp = "2026-01-01T00:00:00Z".to_string();
        let mut projection = views::build_canonical_projection(&[event], &time_context, 300, 1)
            .expect("synthetic projection");
        validate_projection(&projection).expect("unmodified projection must reconcile");

        views::perturb_cost_token_accounting_for_test(&mut projection);
        let error = validate_projection(&projection)
            .expect_err("cost-token drift must fail the production reconciliation gate");
        assert_eq!(error.code(), "E_METRIC_RECONCILIATION");
    }

    #[test]
    fn production_reconciliation_gate_rejects_a_perturbed_public_token_projection() {
        let time_context = TimeContext::new("UTC", Some(2026)).expect("valid test timezone");
        let mut event = synthetic_event("transcript-1".to_string(), 1);
        event.epoch_nanos = 1_767_225_600_000_000_000;
        event.timestamp = "2026-01-01T00:00:00Z".to_string();
        let mut projection = views::build_canonical_projection(&[event], &time_context, 300, 1)
            .expect("synthetic projection");
        validate_projection(&projection).expect("unmodified projection must reconcile");

        views::perturb_public_token_projection_for_test(&mut projection);
        let error = validate_projection(&projection)
            .expect_err("a perturbed public token projection must be rejected");
        assert_eq!(error.code(), "E_METRIC_RECONCILIATION");
    }

    #[test]
    fn production_reconciliation_gate_rejects_a_perturbed_public_activity_projection() {
        let time_context = TimeContext::new("UTC", Some(2026)).expect("valid test timezone");
        let mut event = synthetic_event("transcript-1".to_string(), 1);
        event.epoch_nanos = 1_767_225_600_000_000_000;
        event.timestamp = "2026-01-01T00:00:00Z".to_string();
        let mut projection = views::build_canonical_projection(&[event], &time_context, 300, 1)
            .expect("synthetic projection");
        validate_projection(&projection).expect("unmodified projection must reconcile");

        views::perturb_public_activity_projection_for_test(&mut projection);
        let error = validate_projection(&projection)
            .expect_err("a perturbed public activity projection must be rejected");
        assert_eq!(error.code(), "E_METRIC_RECONCILIATION");
    }

    #[test]
    fn source_capabilities_are_linear_in_events_and_sources() {
        const SOURCE_COUNT: usize = 256;
        const EVENT_COUNT: usize = 1_024;
        let mut diagnostics = types::Diagnostics::default();
        for index in 1..=SOURCE_COUNT {
            let alias = format!("transcript-{index}");
            diagnostics.sources.insert(
                alias.clone(),
                SourceStats::transcript(alias, "explicit-projects".to_string()),
            );
        }
        let events = (0..EVENT_COUNT)
            .map(|index| {
                synthetic_event(
                    format!("transcript-{}", index % SOURCE_COUNT + 1),
                    index as u64,
                )
            })
            .collect::<Vec<_>>();

        SOURCE_CAPABILITY_EVENT_VISITS.with(|visits| visits.set(0));
        record_capabilities(&events, &mut diagnostics);

        assert_eq!(
            SOURCE_CAPABILITY_EVENT_VISITS.with(Cell::get),
            EVENT_COUNT,
            "per-source capability work must visit each event once, independent of source count"
        );
        assert!(diagnostics.sources.values().all(|source| {
            source.capabilities.get("token_usage").map(String::as_str) == Some("available")
        }));
    }
}
