#![allow(dead_code)] // The library builds this shared module without the binary store entry points.

use super::discovery::SourceKind;
use super::types::{
    AliasState, DedupKey, Diagnostics, EventKind, FileSnapshot, NormalizedEvent, SourceStats,
    TokenFacts, NORMALIZED_SCHEMA, OTEL_ADAPTER, TRANSCRIPT_ADAPTER,
};
use super::IngestionOptions;
use bincode::Options;
use blake3::Hasher;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
#[cfg(not(windows))]
use std::fs::OpenOptions;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

const STORE_SCHEMA_VERSION: i64 = 9;
const STORE_FORMAT: &str = "ccwrapped.incremental-store/v9";
const REPORT_SINGLETON: i64 = 1;
const MAXIMUM_PAYLOAD_BYTES: u64 = 512 * 1024 * 1024;
const MAXIMUM_COMPRESSED_PAYLOAD_BYTES: u64 = MAXIMUM_PAYLOAD_BYTES + 16 * 1024 * 1024;
const MAXIMUM_STORED_SOURCE_FILES: usize = 100_256;
const MAXIMUM_STORED_ALIAS_CHARACTERS: i64 = 256;
const CACHE_COMPRESSION_LEVEL: i32 = -5;
const MAXIMUM_REBUILD_STAGE_ATTEMPTS: usize = 16;

#[derive(Debug)]
pub(super) struct DecodedByteBudget {
    remaining: AtomicU64,
}

impl DecodedByteBudget {
    fn new() -> Self {
        Self::with_limit(MAXIMUM_PAYLOAD_BYTES)
    }

    fn with_limit(limit: u64) -> Self {
        Self {
            remaining: AtomicU64::new(limit),
        }
    }

    fn reserve(&self, bytes: usize, label: &'static str) -> Result<(), StoreError> {
        let bytes = u64::try_from(bytes).map_err(|_| {
            StoreError::new(
                "decompress payload",
                format!("{label} exceeds the aggregate decoded-size budget"),
            )
        })?;
        self.remaining
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |remaining| {
                remaining.checked_sub(bytes)
            })
            .map(|_| ())
            .map_err(|_| {
                StoreError::new(
                    "decompress payload",
                    format!("{label} exceeds the aggregate decoded-size budget"),
                )
            })
    }
}

pub(super) struct PreparedStore {
    path: PathBuf,
    salt: [u8; 32],
    rebuild_destination: Option<PathBuf>,
    _lock: StoreLock,
}

struct StoreLock {
    connection: Connection,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = self.connection.execute_batch("ROLLBACK;");
    }
}

impl PreparedStore {
    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn salt(&self) -> [u8; 32] {
        self.salt
    }

    pub fn commit(&mut self) -> Result<(), StoreError> {
        let Some(destination) = self.rebuild_destination.as_ref() else {
            return Ok(());
        };
        validate_store_file(&self.path)?;
        remove_completed_rebuild_journal(&journal_path(&self.path))?;
        match fs::symlink_metadata(destination) {
            Ok(_) => validate_store_file(destination)?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::new("publish rebuild", error.to_string())),
        }
        remove_regular_store_artifact(&journal_path(destination))?;
        replace_store_artifact(&self.path, destination)
            .map_err(|error| StoreError::new("publish rebuild", error.to_string()))?;
        self.path = destination.clone();
        self.rebuild_destination = None;
        Ok(())
    }

    pub fn abort(&mut self) -> Result<(), StoreError> {
        if self.rebuild_destination.is_none() {
            return Ok(());
        }
        remove_regular_store_artifact(&journal_path(&self.path))?;
        remove_regular_store_artifact(&self.path)?;
        self.rebuild_destination = None;
        Ok(())
    }
}

impl Drop for PreparedStore {
    fn drop(&mut self) {
        if self.rebuild_destination.is_some() {
            let _ = remove_regular_store_artifact(&journal_path(&self.path));
            let _ = remove_regular_store_artifact(&self.path);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SourceFile {
    path: PathBuf,
    source_root: PathBuf,
    source_alias: String,
    kind: SourceKind,
    snapshot: FileSnapshot,
    content_digest: [u8; 32],
    event_count: usize,
    events: Option<Vec<u8>>,
    diagnostics: Option<Vec<u8>>,
    metric_state: Option<Vec<u8>>,
    file_alias: String,
    reused: bool,
}

impl SourceFile {
    pub(super) fn metadata_only(
        path: PathBuf,
        source_root: PathBuf,
        source_alias: String,
        kind: SourceKind,
        snapshot: FileSnapshot,
    ) -> Self {
        let mut evidence = Hasher::new();
        evidence.update(b"ccwrapped-metadata-evidence/v1\0");
        evidence.update(&snapshot.len().to_le_bytes());
        let (device, inode, modified_s, modified_ns, changed_s, changed_ns) =
            snapshot.store_identity();
        evidence.update(&device.to_le_bytes());
        evidence.update(&inode.to_le_bytes());
        evidence.update(&modified_s.to_le_bytes());
        evidence.update(&modified_ns.to_le_bytes());
        evidence.update(&changed_s.to_le_bytes());
        evidence.update(&changed_ns.to_le_bytes());
        Self {
            path,
            source_root,
            source_alias,
            kind,
            snapshot,
            content_digest: *evidence.finalize().as_bytes(),
            event_count: 0,
            events: None,
            diagnostics: None,
            metric_state: None,
            file_alias: String::new(),
            reused: false,
        }
    }

    pub(super) fn reused_metadata(
        path: PathBuf,
        source_root: PathBuf,
        source_alias: String,
        kind: SourceKind,
        snapshot: FileSnapshot,
        content_digest: [u8; 32],
    ) -> Self {
        let mut file = Self::with_content_digest(
            path,
            source_root,
            source_alias,
            kind,
            snapshot,
            content_digest,
        );
        file.reused = true;
        file
    }

    pub(super) fn with_content_digest(
        path: PathBuf,
        source_root: PathBuf,
        source_alias: String,
        kind: SourceKind,
        snapshot: FileSnapshot,
        content_digest: [u8; 32],
    ) -> Self {
        Self {
            path,
            source_root,
            source_alias,
            kind,
            snapshot,
            content_digest,
            event_count: 0,
            events: None,
            diagnostics: None,
            metric_state: None,
            file_alias: String::new(),
            reused: false,
        }
    }

    pub(super) fn with_file_alias(mut self, file_alias: impl Into<String>) -> Self {
        self.file_alias = file_alias.into();
        self
    }

    pub(super) fn with_payload(
        mut self,
        events: &[NormalizedEvent],
        diagnostics: &Diagnostics,
        metric_state: Option<Vec<u8>>,
    ) -> Result<Self, StoreError> {
        self.event_count = events.len();
        self.events = Some(encode(events, "normalized events")?);
        self.diagnostics = Some(encode_uncompressed(diagnostics, "diagnostics")?);
        self.metric_state = metric_state;
        Ok(self)
    }

    pub(super) fn with_encoded_payload(
        mut self,
        event_count: usize,
        events: Vec<u8>,
        diagnostics: Vec<u8>,
        metric_state: Option<Vec<u8>>,
    ) -> Self {
        self.event_count = event_count;
        self.events = Some(events);
        self.diagnostics = Some(diagnostics);
        self.metric_state = metric_state;
        self.reused = true;
        self
    }

    pub(super) fn source_bytes(&self) -> u64 {
        self.snapshot.len()
    }

    pub(super) fn reused(&self) -> bool {
        self.reused
    }
}

#[derive(Debug)]
pub(super) struct CachedFile {
    pub events: Vec<NormalizedEvent>,
    pub diagnostics: Diagnostics,
    pub metric_state: Option<Vec<u8>>,
    pub content_digest: [u8; 32],
    pub event_payload: Vec<u8>,
    pub diagnostics_payload: Vec<u8>,
    pub file_alias: String,
    pub event_count: usize,
    pub decode_budget: Arc<DecodedByteBudget>,
}

#[derive(Debug)]
pub(super) struct RawCachedFile {
    content_digest: [u8; 32],
    event_payload: Vec<u8>,
    diagnostics_payload: Vec<u8>,
    metric_state: Option<Vec<u8>>,
    deferred_payload: Option<DeferredPayload>,
    events_available: bool,
    file_alias: String,
    event_count: usize,
    decode_budget: Arc<DecodedByteBudget>,
}

impl RawCachedFile {
    pub fn content_digest(&self) -> [u8; 32] {
        self.content_digest
    }

    pub fn file_alias(&self) -> &str {
        &self.file_alias
    }

    pub fn events_available(&self) -> bool {
        self.events_available
    }

    pub fn event_count(&self) -> usize {
        self.event_count
    }
}

#[derive(Debug)]
struct DeferredPayload {
    connection: Arc<Mutex<Connection>>,
    path_key: [u8; 32],
}

pub(super) struct PreviousCachedFile {
    pub raw: RawCachedFile,
    pub source_bytes: u64,
}

#[derive(Debug, Deserialize)]
struct StoredEvent {
    schema_version: String,
    adapter_version: String,
    source_alias: String,
    file_alias: String,
    record_index: u64,
    timestamp: String,
    epoch_nanos: i128,
    timestamp_conversion_status: String,
    project_key: u64,
    project_identity_present: bool,
    session_key: u64,
    session_identity_present: bool,
    message_key: Option<u64>,
    request_key: Option<u64>,
    parent_key: Option<u64>,
    agent_key: Option<u64>,
    parent_agent_key: Option<u64>,
    skill_key: Option<u64>,
    plugin_key: Option<u64>,
    mcp_server_key: Option<u64>,
    mcp_tool_key: Option<u64>,
    observation_key: u64,
    project_alias: String,
    session_alias: String,
    parent_session_alias: Option<String>,
    is_subagent: bool,
    is_sidechain: bool,
    kind: EventKind,
    model: Option<String>,
    model_mapping_status: String,
    pricing_modifier: String,
    tokens: TokenFacts,
    source_cost_estimate: Option<f64>,
    tool_names: Vec<String>,
    tool_status: Option<String>,
    latency_ms: Option<f64>,
    error_count: Option<u64>,
    retry_count: Option<u64>,
    edit_decision: Option<String>,
    compaction: Option<bool>,
    metric_name: Option<String>,
    metric_value: Option<f64>,
    metric_unit: Option<String>,
    metric_interval_start_nanos: Option<u64>,
    metric_interval_end_nanos: Option<u64>,
    metric_temporality: Option<u64>,
    metric_family_key: Option<u64>,
    attribute_evidence_uncertain: bool,
    redacted_fields: usize,
}

impl StoredEvent {
    fn into_runtime(self) -> Result<NormalizedEvent, StoreError> {
        Ok(NormalizedEvent {
            schema_version: known(
                &self.schema_version,
                &[NORMALIZED_SCHEMA],
                "normalized schema",
            )?,
            adapter_version: known(
                &self.adapter_version,
                &[TRANSCRIPT_ADAPTER, OTEL_ADAPTER],
                "adapter",
            )?,
            source_alias: self.source_alias,
            file_alias: self.file_alias,
            record_index: self.record_index,
            timestamp: self.timestamp,
            epoch_nanos: self.epoch_nanos,
            timestamp_conversion_status: known(
                &self.timestamp_conversion_status,
                &["normalized-utc"],
                "timestamp conversion",
            )?,
            project_key: self.project_key,
            project_identity_present: self.project_identity_present,
            session_key: self.session_key,
            session_identity_present: self.session_identity_present,
            message_key: self.message_key,
            request_key: self.request_key,
            parent_key: self.parent_key,
            agent_key: self.agent_key,
            parent_agent_key: self.parent_agent_key,
            skill_key: self.skill_key,
            plugin_key: self.plugin_key,
            mcp_server_key: self.mcp_server_key,
            mcp_tool_key: self.mcp_tool_key,
            observation_key: self.observation_key,
            project_alias: self.project_alias,
            session_alias: self.session_alias,
            parent_session_alias: self.parent_session_alias,
            is_subagent: self.is_subagent,
            is_sidechain: self.is_sidechain,
            kind: self.kind,
            model: self.model,
            model_mapping_status: known(
                &self.model_mapping_status,
                &["missing", "unmapped"],
                "model mapping",
            )?,
            pricing_modifier: known(
                &self.pricing_modifier,
                &["standard", "fast", "unknown"],
                "pricing modifier",
            )?
            .to_string(),
            tokens: self.tokens,
            source_cost_estimate: self.source_cost_estimate,
            tool_names: self.tool_names,
            tool_status: self.tool_status,
            latency_ms: self.latency_ms,
            error_count: self.error_count,
            retry_count: self.retry_count,
            edit_decision: self.edit_decision,
            compaction: self.compaction,
            metric_name: self
                .metric_name
                .as_deref()
                .map(|value| {
                    known(
                        value,
                        &[
                            "session-count",
                            "lines-of-code",
                            "pull-requests",
                            "commits",
                            "source-cost-estimate",
                            "token-usage",
                            "code-edit-decision",
                            "active-time",
                        ],
                        "metric name",
                    )
                })
                .transpose()?,
            metric_value: self.metric_value,
            metric_unit: self
                .metric_unit
                .as_deref()
                .map(|value| {
                    known(
                        value,
                        &[
                            "sessions",
                            "lines",
                            "pull-requests",
                            "commits",
                            "usd",
                            "tokens",
                            "decisions",
                            "seconds",
                        ],
                        "metric unit",
                    )
                })
                .transpose()?,
            metric_interval_start_nanos: self.metric_interval_start_nanos,
            metric_interval_end_nanos: self.metric_interval_end_nanos,
            metric_temporality: self.metric_temporality,
            metric_family_key: self.metric_family_key,
            attribute_evidence_uncertain: self.attribute_evidence_uncertain,
            redacted_fields: self.redacted_fields,
        })
    }
}

#[derive(Debug, Deserialize)]
struct StoredSourceStats {
    alias: String,
    kind: String,
    selection: String,
    adapter_version: String,
    files_discovered: usize,
    accepted_records: usize,
    malformed_records: usize,
    unsupported_records: usize,
    unknown_records: usize,
    unknown_fields: usize,
    filtered_records: usize,
    redacted_fields: usize,
    duplicate_records: usize,
    skipped_records: usize,
    earliest: Option<(i128, String)>,
    latest: Option<(i128, String)>,
    capabilities: BTreeMap<String, String>,
    partial: bool,
    producer_contract: Option<String>,
    producer_verification: Option<String>,
}

impl StoredSourceStats {
    fn into_runtime(self, map_key: &str) -> Result<SourceStats, StoreError> {
        let invalid_identity =
            self.alias != map_key || !valid_source_alias(&self.alias, &self.kind);
        let invalid_timestamps = self
            .earliest
            .as_ref()
            .is_some_and(|(_, timestamp)| !valid_inert_text(timestamp, 64))
            || self
                .latest
                .as_ref()
                .is_some_and(|(_, timestamp)| !valid_inert_text(timestamp, 64));
        let invalid_selection = !matches!(
            self.selection.as_str(),
            "explicit-projects"
                | "explicit-config"
                | "claude-config-env"
                | "home-default"
                | "explicit-file"
                | "parallel-file"
        );
        let invalid_adapter = (self.kind == "transcript"
            && (self.adapter_version != TRANSCRIPT_ADAPTER
                || self.producer_contract.is_some()
                || self.producer_verification.is_some()))
            || (self.kind == "otel"
                && (self.adapter_version != OTEL_ADAPTER
                    || self.producer_contract.as_deref() != Some(super::types::OTEL_CONTRACT)
                    || self.producer_verification.as_deref() != Some("unverified")));
        let invalid_capabilities = self.capabilities.len() > 256
            || self.capabilities.iter().any(|(name, status)| {
                !valid_safe_token(name, 128)
                    || !matches!(
                        status.as_str(),
                        "available" | "partial" | "unavailable" | "excluded"
                    )
            });
        let invalid_field = [
            (invalid_identity, "identity"),
            (invalid_timestamps, "timestamp"),
            (invalid_selection, "selection"),
            (invalid_adapter, "adapter provenance"),
            (invalid_capabilities, "capabilities"),
        ]
        .into_iter()
        .find_map(|(invalid, field)| invalid.then_some(field));
        if let Some(field) = invalid_field {
            return Err(StoreError::new(
                "validate stored diagnostics",
                format!(
                    "cached source diagnostic {field} is outside the finite contract; run with --rebuild-store"
                ),
            ));
        }
        Ok(SourceStats {
            alias: self.alias,
            kind: self.kind,
            selection: self.selection,
            adapter_version: self.adapter_version,
            files_discovered: self.files_discovered,
            accepted_records: self.accepted_records,
            malformed_records: self.malformed_records,
            unsupported_records: self.unsupported_records,
            unknown_records: self.unknown_records,
            unknown_fields: self.unknown_fields,
            filtered_records: self.filtered_records,
            redacted_fields: self.redacted_fields,
            duplicate_records: self.duplicate_records,
            skipped_records: self.skipped_records,
            earliest: self.earliest,
            latest: self.latest,
            capabilities: self.capabilities,
            partial: self.partial,
            producer_contract: self.producer_contract,
            producer_verification: self.producer_verification,
        })
    }
}

#[derive(Debug, Deserialize)]
struct StoredWarning {
    code: String,
    message: String,
    source_alias: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StoredUnknownShape {
    source_alias: String,
    adapter_version: String,
    file_alias: String,
    record_index: u64,
    record_kind: String,
    structural_fields: BTreeMap<String, String>,
    byte_count: usize,
}

#[derive(Debug, Deserialize)]
struct StoredDiagnostics {
    source_root_count: usize,
    files_discovered: usize,
    accepted_records: usize,
    canonical_records: usize,
    malformed_records: usize,
    unsupported_records: usize,
    unknown_records: usize,
    unknown_fields: usize,
    filtered_records: usize,
    redacted_fields: usize,
    duplicate_records: usize,
    skipped_records: usize,
    resolved_overlap_records: usize,
    unresolved_overlap_records: usize,
    authority_excluded_records: usize,
    earliest: Option<(i128, String)>,
    latest: Option<(i128, String)>,
    sources: BTreeMap<String, StoredSourceStats>,
    warnings: Vec<StoredWarning>,
    unknown_shapes: Vec<StoredUnknownShape>,
    capabilities: BTreeMap<String, String>,
    saw_source_cost: bool,
    analytical_cost_coverage: Option<String>,
    excluded_analysis_token_categories: u8,
    excluded_analysis_cost: bool,
    analytical_claims_uncertain: bool,
}

impl StoredDiagnostics {
    fn into_runtime(self) -> Result<Diagnostics, StoreError> {
        if self.sources.len() > 256
            || self.warnings.len() > 4096
            || self.unknown_shapes.len() > super::types::MAX_UNKNOWN_SHAPE_DIAGNOSTICS
            || self.capabilities.len() > 256
            || self.capabilities.iter().any(|(name, status)| {
                !valid_safe_token(name, 128)
                    || !matches!(
                        status.as_str(),
                        "available" | "partial" | "unavailable" | "excluded"
                    )
            })
            || self
                .earliest
                .as_ref()
                .is_some_and(|(_, timestamp)| !valid_inert_text(timestamp, 64))
            || self
                .latest
                .as_ref()
                .is_some_and(|(_, timestamp)| !valid_inert_text(timestamp, 64))
        {
            return Err(StoreError::new(
                "validate stored diagnostics",
                "cached diagnostic cardinality or capability data exceeds its contract; run with --rebuild-store",
            ));
        }
        for warning in &self.warnings {
            if !valid_warning_code(&warning.code)
                || !valid_inert_text(&warning.message, 1024)
                || warning
                    .source_alias
                    .as_deref()
                    .is_some_and(|alias| !self.sources.contains_key(alias))
            {
                return Err(StoreError::new(
                    "validate stored diagnostics",
                    "a cached warning is outside the finite diagnostic contract; run with --rebuild-store",
                ));
            }
        }
        for shape in &self.unknown_shapes {
            if !self.sources.contains_key(&shape.source_alias)
                || !matches!(
                    shape.adapter_version.as_str(),
                    TRANSCRIPT_ADAPTER | OTEL_ADAPTER
                )
                || !valid_file_alias(&shape.file_alias)
                || !valid_safe_token(&shape.record_kind, 128)
                || shape.structural_fields.len() > 128
                || shape.structural_fields.iter().any(|(field, value_type)| {
                    !valid_safe_token(field, 128) || !valid_safe_token(value_type, 64)
                })
            {
                return Err(StoreError::new(
                    "validate stored diagnostics",
                    "a cached unknown-shape diagnostic is outside the bounded structural contract; run with --rebuild-store",
                ));
            }
        }
        let analytical_cost_coverage = self
            .analytical_cost_coverage
            .as_deref()
            .map(|value| {
                known(
                    value,
                    &[
                        "unavailable-conflicting-cost-bases",
                        "source-recorded-estimate-and-local-computation",
                        "local-computation-with-unpriced-possibility",
                        "partial-observed-cost-evidence",
                        "unavailable-incomplete-usage",
                    ],
                    "cost coverage",
                )
            })
            .transpose()?;
        Ok(Diagnostics {
            source_root_count: self.source_root_count,
            files_discovered: self.files_discovered,
            accepted_records: self.accepted_records,
            canonical_records: self.canonical_records,
            malformed_records: self.malformed_records,
            unsupported_records: self.unsupported_records,
            unknown_records: self.unknown_records,
            unknown_fields: self.unknown_fields,
            filtered_records: self.filtered_records,
            redacted_fields: self.redacted_fields,
            duplicate_records: self.duplicate_records,
            skipped_records: self.skipped_records,
            resolved_overlap_records: self.resolved_overlap_records,
            unresolved_overlap_records: self.unresolved_overlap_records,
            authority_excluded_records: self.authority_excluded_records,
            earliest: self.earliest,
            latest: self.latest,
            sources: self
                .sources
                .into_iter()
                .map(|(key, value)| {
                    let runtime = value.into_runtime(&key)?;
                    Ok((key, runtime))
                })
                .collect::<Result<BTreeMap<_, _>, StoreError>>()?,
            warnings: self
                .warnings
                .into_iter()
                .map(|warning| ccwrapped::IngestionWarning {
                    code: warning.code,
                    message: warning.message,
                    source_alias: warning.source_alias,
                })
                .collect(),
            unknown_shapes: self
                .unknown_shapes
                .into_iter()
                .map(|shape| ccwrapped::UnknownShapeDiagnostic {
                    source_alias: shape.source_alias,
                    adapter_version: shape.adapter_version,
                    file_alias: shape.file_alias,
                    record_index: shape.record_index,
                    record_kind: shape.record_kind,
                    structural_fields: shape.structural_fields,
                    byte_count: shape.byte_count,
                })
                .collect(),
            capabilities: self.capabilities,
            saw_source_cost: self.saw_source_cost,
            analytical_cost_coverage,
            excluded_analysis_token_categories: self.excluded_analysis_token_categories,
            excluded_analysis_cost: self.excluded_analysis_cost,
            analytical_claims_uncertain: self.analytical_claims_uncertain,
        })
    }
}

#[derive(Debug)]
struct StoredAnalysisState {
    canonical_events: Vec<StoredEvent>,
    diagnostics: StoredDiagnostics,
    aliases: AliasState,
    alias_observations: Vec<super::AliasObservation>,
    observed_summary: super::insights::ObservedEventSummary,
    dedup_keys: Vec<DedupKey>,
    authority_keys: Vec<super::AppendAuthorityKey>,
    otel_request_groups: Vec<super::RequestCorrelationGroupKey>,
    aggregate_metrics: Vec<StoredEvent>,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredAnalysisEnvelope {
    canonical_events: Vec<u8>,
    diagnostics: Vec<u8>,
    aliases: Vec<u8>,
    alias_observations: Vec<u8>,
    observed_summary: Vec<u8>,
    dedup_keys: Vec<u8>,
    authority_keys: Vec<u8>,
    otel_request_groups: Vec<u8>,
    aggregate_metrics: Vec<u8>,
}

impl StoredAnalysisState {
    fn into_runtime(self) -> Result<super::AnalysisState, StoreError> {
        let canonical_events = self
            .canonical_events
            .into_iter()
            .map(StoredEvent::into_runtime)
            .collect::<Result<Vec<_>, _>>()?;
        let aggregate_metrics = self
            .aggregate_metrics
            .into_iter()
            .map(StoredEvent::into_runtime)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(super::AnalysisState {
            canonical_events,
            diagnostics: self.diagnostics.into_runtime()?,
            aliases: self.aliases,
            alias_observations: self.alias_observations,
            observed_summary: self.observed_summary,
            dedup_keys: self.dedup_keys,
            authority_keys: self.authority_keys,
            otel_request_groups: self.otel_request_groups,
            aggregate_metrics,
        })
    }
}

pub(super) fn encode_analysis_state(state: &super::AnalysisState) -> Result<Vec<u8>, StoreError> {
    let envelope = StoredAnalysisEnvelope {
        canonical_events: encode(&state.canonical_events, "canonical events")?,
        diagnostics: encode(&state.diagnostics, "analysis diagnostics")?,
        aliases: encode(&state.aliases, "analysis aliases")?,
        alias_observations: encode(&state.alias_observations, "alias observations")?,
        observed_summary: encode(&state.observed_summary, "observed summary")?,
        dedup_keys: encode(&state.dedup_keys, "dedup keys")?,
        authority_keys: encode(&state.authority_keys, "authority keys")?,
        otel_request_groups: encode(&state.otel_request_groups, "request groups")?,
        aggregate_metrics: encode(&state.aggregate_metrics, "aggregate metrics")?,
    };
    validate_analysis_component_budget(&envelope)?;
    encode_uncompressed(&envelope, "analysis state envelope")
}

fn validate_analysis_component_budget(envelope: &StoredAnalysisEnvelope) -> Result<(), StoreError> {
    validate_analysis_component_lengths([
        envelope.canonical_events.len(),
        envelope.diagnostics.len(),
        envelope.aliases.len(),
        envelope.alias_observations.len(),
        envelope.observed_summary.len(),
        envelope.dedup_keys.len(),
        envelope.authority_keys.len(),
        envelope.otel_request_groups.len(),
        envelope.aggregate_metrics.len(),
    ])
}

fn validate_analysis_component_lengths(lengths: [usize; 9]) -> Result<(), StoreError> {
    let bytes = lengths
        .into_iter()
        .try_fold(0u64, |total, bytes| {
            total.checked_add(u64::try_from(bytes).ok()?)
        })
        .ok_or_else(|| {
            StoreError::new(
                "encode analysis state",
                "analysis components exceed the aggregate encoded-size budget",
            )
        })?;
    if bytes > MAXIMUM_PAYLOAD_BYTES {
        return Err(StoreError::new(
            "encode analysis state",
            "analysis components exceed the aggregate encoded-size budget",
        ));
    }
    Ok(())
}

fn decode_analysis_state_envelope(
    envelope: StoredAnalysisEnvelope,
    budget: &DecodedByteBudget,
) -> Result<super::AnalysisState, StoreError> {
    StoredAnalysisState {
        canonical_events: decode_with_shared_budget(
            &envelope.canonical_events,
            "canonical events",
            budget,
        )?,
        diagnostics: decode_with_shared_budget(
            &envelope.diagnostics,
            "analysis diagnostics",
            budget,
        )?,
        aliases: decode_with_shared_budget(&envelope.aliases, "analysis aliases", budget)?,
        alias_observations: decode_with_shared_budget(
            &envelope.alias_observations,
            "alias observations",
            budget,
        )?,
        observed_summary: decode_with_shared_budget(
            &envelope.observed_summary,
            "observed summary",
            budget,
        )?,
        dedup_keys: decode_with_shared_budget(&envelope.dedup_keys, "dedup keys", budget)?,
        authority_keys: decode_with_shared_budget(
            &envelope.authority_keys,
            "authority keys",
            budget,
        )?,
        otel_request_groups: decode_with_shared_budget(
            &envelope.otel_request_groups,
            "request groups",
            budget,
        )?,
        aggregate_metrics: decode_with_shared_budget(
            &envelope.aggregate_metrics,
            "aggregate metrics",
            budget,
        )?,
    }
    .into_runtime()
}

fn decode_analysis_header(
    envelope: &StoredAnalysisEnvelope,
    budget: &DecodedByteBudget,
) -> Result<(Diagnostics, AliasState), StoreError> {
    let diagnostics: StoredDiagnostics =
        decode_with_shared_budget(&envelope.diagnostics, "analysis diagnostics", budget)?;
    let aliases = decode_with_shared_budget(&envelope.aliases, "analysis aliases", budget)?;
    Ok((diagnostics.into_runtime()?, aliases))
}

#[derive(Debug)]
pub(super) enum CacheLookup {
    Hit(Vec<u8>),
    Miss,
}

#[derive(Debug)]
struct StoredFileRow {
    normalization_key: Vec<u8>,
    source_key: Vec<u8>,
    source_alias: String,
    source_kind: i64,
    file_alias: String,
    event_count: i64,
    device: Vec<u8>,
    inode: Vec<u8>,
    size: i64,
    modified_s: i64,
    modified_ns: i64,
    changed_s: i64,
    changed_ns: i64,
    content_digest: Vec<u8>,
    event_payload_bytes: i64,
    diagnostics_payload_bytes: i64,
    metric_state_payload_bytes: Option<i64>,
}

struct RawStoredFileRow {
    normalization_key: Option<Vec<u8>>,
    source_key: Option<Vec<u8>>,
    source_alias: Option<String>,
    source_kind: i64,
    file_alias: Option<String>,
    event_count: i64,
    device: Option<Vec<u8>>,
    inode: Option<Vec<u8>>,
    size: i64,
    modified_s: i64,
    modified_ns: i64,
    changed_s: i64,
    changed_ns: i64,
    content_digest: Option<Vec<u8>>,
    event_payload_bytes: Option<i64>,
    diagnostics_payload_bytes: Option<i64>,
    metric_state_payload_bytes: Option<i64>,
}

fn load_file_rows(connection: &Connection) -> Result<HashMap<[u8; 32], StoredFileRow>, StoreError> {
    let stored_count = connection
        .query_row("SELECT count(*) FROM source_file", [], |row| {
            row.get::<_, usize>(0)
        })
        .map_err(|error| StoreError::new("count file cache", error.to_string()))?;
    if stored_count > MAXIMUM_STORED_SOURCE_FILES {
        return Err(StoreError::new(
            "verify file cache",
            format!(
                "the store contains more than {MAXIMUM_STORED_SOURCE_FILES} source rows; run with --rebuild-store"
            ),
        ));
    }
    let mut statement = connection
        .prepare(
            "
            SELECT
                   CASE WHEN typeof(path_key) = 'blob' AND length(path_key) = 32
                        THEN path_key END,
                   CASE WHEN typeof(normalization_key) = 'blob'
                                  AND length(normalization_key) = 32
                        THEN normalization_key END,
                   CASE WHEN typeof(source_key) = 'blob' AND length(source_key) = 32
                        THEN source_key END,
                   CASE WHEN typeof(source_alias) = 'text' AND length(source_alias) <= ?1
                        THEN source_alias END,
                   source_kind,
                   CASE WHEN typeof(file_alias) = 'text' AND length(file_alias) <= ?1
                        THEN file_alias END,
                   event_count,
                   CASE WHEN typeof(device) = 'blob' AND length(device) = 8
                        THEN device END,
                   CASE WHEN typeof(inode) = 'blob' AND length(inode) = 8
                        THEN inode END,
                   size, modified_seconds, modified_nanoseconds,
                   changed_seconds, changed_nanoseconds,
                   CASE WHEN typeof(content_digest) = 'blob' AND length(content_digest) = 32
                        THEN content_digest END,
                   CASE WHEN typeof(normalized_events) = 'blob'
                        THEN length(normalized_events) END,
                   CASE WHEN typeof(diagnostics) = 'blob'
                        THEN length(diagnostics) END,
                   CASE
                       WHEN metric_state IS NULL THEN NULL
                       WHEN typeof(metric_state) = 'blob' THEN length(metric_state)
                       ELSE -1
                   END
            FROM source_file
            ",
        )
        .map_err(|error| StoreError::new("prepare file cache scan", error.to_string()))?;
    let records = statement
        .query_map(params![MAXIMUM_STORED_ALIAS_CHARACTERS], |row| {
            Ok((
                row.get::<_, Option<Vec<u8>>>(0)?,
                RawStoredFileRow {
                    normalization_key: row.get(1)?,
                    source_key: row.get(2)?,
                    source_alias: row.get(3)?,
                    source_kind: row.get(4)?,
                    file_alias: row.get(5)?,
                    event_count: row.get(6)?,
                    device: row.get(7)?,
                    inode: row.get(8)?,
                    size: row.get(9)?,
                    modified_s: row.get(10)?,
                    modified_ns: row.get(11)?,
                    changed_s: row.get(12)?,
                    changed_ns: row.get(13)?,
                    content_digest: row.get(14)?,
                    event_payload_bytes: row.get(15)?,
                    diagnostics_payload_bytes: row.get(16)?,
                    metric_state_payload_bytes: row.get(17)?,
                },
            ))
        })
        .map_err(|error| StoreError::new("scan file cache", error.to_string()))?;
    let mut rows = HashMap::new();
    let mut aggregate_event_count = 0usize;
    let mut aggregate_stored_payload_bytes = 0u64;
    for record in records {
        let (key, raw) =
            record.map_err(|error| StoreError::new("read file cache row", error.to_string()))?;
        let key = required_bounded_store_value("verify file cache row", key, "path key")?;
        let row = StoredFileRow {
            normalization_key: required_bounded_store_value(
                "verify file cache row",
                raw.normalization_key,
                "normalization key",
            )?,
            source_key: required_bounded_store_value(
                "verify file cache row",
                raw.source_key,
                "source key",
            )?,
            source_alias: required_bounded_store_value(
                "verify file cache row",
                raw.source_alias,
                "source alias",
            )?,
            source_kind: raw.source_kind,
            file_alias: required_bounded_store_value(
                "verify file cache row",
                raw.file_alias,
                "file alias",
            )?,
            event_count: raw.event_count,
            device: required_bounded_store_value(
                "verify file cache row",
                raw.device,
                "device identity",
            )?,
            inode: required_bounded_store_value(
                "verify file cache row",
                raw.inode,
                "inode identity",
            )?,
            size: raw.size,
            modified_s: raw.modified_s,
            modified_ns: raw.modified_ns,
            changed_s: raw.changed_s,
            changed_ns: raw.changed_ns,
            content_digest: required_bounded_store_value(
                "verify file cache row",
                raw.content_digest,
                "content digest",
            )?,
            event_payload_bytes: required_bounded_store_value(
                "verify file cache row",
                raw.event_payload_bytes,
                "normalized-event payload",
            )?,
            diagnostics_payload_bytes: required_bounded_store_value(
                "verify file cache row",
                raw.diagnostics_payload_bytes,
                "diagnostics payload",
            )?,
            metric_state_payload_bytes: raw.metric_state_payload_bytes,
        };
        if !matches!(row.source_kind, 1 | 2) {
            return Err(StoreError::new(
                "verify file cache row",
                "a stored source kind is invalid; run with --rebuild-store",
            ));
        }
        if row.event_count < 0
            || usize::try_from(row.event_count)
                .ok()
                .is_none_or(|count| count > super::MAXIMUM_NORMALIZED_EVENTS)
        {
            return Err(StoreError::new(
                "verify file cache row",
                "a stored event count is outside its bounded range; run with --rebuild-store",
            ));
        }
        let event_count = usize::try_from(row.event_count).map_err(|_| {
            StoreError::new(
                "verify file cache row",
                "a stored event count is outside its bounded range; run with --rebuild-store",
            )
        })?;
        aggregate_event_count = aggregate_event_count
            .checked_add(event_count)
            .filter(|count| *count <= super::MAXIMUM_NORMALIZED_EVENTS)
            .ok_or_else(|| {
                StoreError::new(
                    "verify file cache",
                    "the aggregate cached event count exceeds its bounded range; run with --rebuild-store",
                )
            })?;
        if row.size < 0 {
            return Err(StoreError::new(
                "verify file cache row",
                "a stored source size is negative; run with --rebuild-store",
            ));
        }
        validate_stored_blob_length(
            "verify file cache row",
            "normalized-event payload",
            row.event_payload_bytes,
            MAXIMUM_COMPRESSED_PAYLOAD_BYTES,
        )?;
        validate_stored_blob_length(
            "verify file cache row",
            "diagnostics payload",
            row.diagnostics_payload_bytes,
            MAXIMUM_PAYLOAD_BYTES,
        )?;
        if let Some(length) = row.metric_state_payload_bytes {
            validate_stored_blob_length(
                "verify file cache row",
                "metric-state payload",
                length,
                MAXIMUM_COMPRESSED_PAYLOAD_BYTES,
            )?;
        }
        for length in [
            Some(row.event_payload_bytes),
            Some(row.diagnostics_payload_bytes),
            row.metric_state_payload_bytes,
        ]
        .into_iter()
        .flatten()
        {
            let length = u64::try_from(length).map_err(|_| {
                StoreError::new(
                    "verify file cache",
                    "a cached payload has a negative aggregate length; run with --rebuild-store",
                )
            })?;
            aggregate_stored_payload_bytes = aggregate_stored_payload_bytes
                .checked_add(length)
                .filter(|total| *total <= MAXIMUM_COMPRESSED_PAYLOAD_BYTES)
                .ok_or_else(|| {
                    StoreError::new(
                        "verify file cache",
                        "the aggregate cached payload size exceeds its bounded range; run with --rebuild-store",
                    )
                })?;
        }
        let key: [u8; 32] = key.try_into().map_err(|_| {
            StoreError::new(
                "verify file cache row",
                "a stored path key is not 32 bytes; run with --rebuild-store",
            )
        })?;
        if rows.insert(key, row).is_some() {
            return Err(StoreError::new(
                "verify file cache row",
                "the store contains a duplicate path key; run with --rebuild-store",
            ));
        }
    }
    Ok(rows)
}

fn required_bounded_store_value<T>(
    action: &'static str,
    value: Option<T>,
    label: &'static str,
) -> Result<T, StoreError> {
    value.ok_or_else(|| {
        StoreError::new(
            action,
            format!("a stored {label} has an invalid type or size; run with --rebuild-store"),
        )
    })
}

type StoredFilePayload = (Vec<u8>, Vec<u8>, Option<Vec<u8>>);

fn load_file_payload(
    connection: &Connection,
    path_key: &[u8; 32],
) -> Result<StoredFilePayload, StoreError> {
    let maximum_compressed = i64::try_from(MAXIMUM_COMPRESSED_PAYLOAD_BYTES)
        .expect("compressed payload limit fits SQLite INTEGER");
    let maximum_uncompressed =
        i64::try_from(MAXIMUM_PAYLOAD_BYTES).expect("payload limit fits SQLite INTEGER");
    let stored = connection
        .query_row(
            "
            SELECT
                CASE WHEN typeof(normalized_events) = 'blob'
                               AND length(normalized_events) <= ?2
                     THEN normalized_events END,
                CASE WHEN typeof(normalized_events) = 'blob'
                     THEN length(normalized_events) ELSE -1 END,
                CASE WHEN typeof(diagnostics) = 'blob' AND length(diagnostics) <= ?3
                     THEN diagnostics END,
                CASE WHEN typeof(diagnostics) = 'blob'
                     THEN length(diagnostics) ELSE -1 END,
                CASE
                    WHEN metric_state IS NULL THEN NULL
                    WHEN typeof(metric_state) = 'blob' AND length(metric_state) <= ?2
                    THEN metric_state
                END,
                CASE
                    WHEN metric_state IS NULL THEN NULL
                    WHEN typeof(metric_state) = 'blob' THEN length(metric_state)
                    ELSE -1
                END
            FROM source_file
            WHERE path_key = ?1
            ",
            params![
                path_key.as_slice(),
                maximum_compressed,
                maximum_uncompressed
            ],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                ))
            },
        )
        .optional()
        .map_err(|error| StoreError::new("read file payload", error.to_string()))?
        .ok_or_else(|| {
            StoreError::new(
                "read file payload",
                "the indexed source row disappeared; run with --rebuild-store",
            )
        })?;
    let (events, events_len, diagnostics, diagnostics_len, metric_state, metric_state_len) = stored;
    validate_stored_blob_length(
        "read file payload",
        "normalized-event payload",
        events_len,
        MAXIMUM_COMPRESSED_PAYLOAD_BYTES,
    )?;
    validate_stored_blob_length(
        "read file payload",
        "diagnostics payload",
        diagnostics_len,
        MAXIMUM_PAYLOAD_BYTES,
    )?;
    if let Some(length) = metric_state_len {
        validate_stored_blob_length(
            "read file payload",
            "metric-state payload",
            length,
            MAXIMUM_COMPRESSED_PAYLOAD_BYTES,
        )?;
    }
    Ok((
        events.ok_or_else(|| {
            StoreError::new(
                "read file payload",
                "the normalized-event payload exceeded its storage bound",
            )
        })?,
        diagnostics.ok_or_else(|| {
            StoreError::new(
                "read file payload",
                "the diagnostics payload exceeded its storage bound",
            )
        })?,
        metric_state,
    ))
}

type LoadedAnalysisState = (
    Option<(Diagnostics, AliasState)>,
    Option<HashSet<super::AppendAuthorityKey>>,
    Option<StoredAnalysisEnvelope>,
);

fn load_analysis_state(
    connection: &Connection,
    options_key: &[u8; 32],
    decode_budget: &DecodedByteBudget,
) -> Result<LoadedAnalysisState, StoreError> {
    let maximum = i64::try_from(MAXIMUM_PAYLOAD_BYTES).expect("payload limit fits SQLite INTEGER");
    let stored = connection
        .query_row(
            "
            SELECT CASE WHEN typeof(options_key) = 'blob' AND length(options_key) = 32
                        THEN options_key END,
                   CASE WHEN typeof(payload) = 'blob' AND length(payload) <= ?2
                        THEN payload END,
                   CASE WHEN typeof(payload) = 'blob' THEN length(payload) ELSE -1 END,
                   CASE WHEN typeof(payload_digest) = 'blob' AND length(payload_digest) = 32
                        THEN payload_digest END
            FROM analysis_state
            WHERE singleton = ?1
            ",
            params![REPORT_SINGLETON, maximum],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, Option<Vec<u8>>>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                ))
            },
        )
        .optional()
        .map_err(|error| StoreError::new("read analysis state", error.to_string()))?;
    let Some((stored_key, payload, payload_len, digest)) = stored else {
        return Ok((None, None, None));
    };
    let stored_key =
        required_bounded_store_value("read analysis state", stored_key, "analysis options key")?;
    let digest =
        required_bounded_store_value("read analysis state", digest, "analysis payload digest")?;
    if stored_key.as_slice() != options_key {
        return Ok((None, None, None));
    }
    validate_stored_blob_length(
        "read analysis state",
        "analysis-state envelope",
        payload_len,
        MAXIMUM_PAYLOAD_BYTES,
    )?;
    let payload = payload.ok_or_else(|| {
        StoreError::new(
            "read analysis state",
            "the analysis-state envelope exceeded its storage bound",
        )
    })?;
    if digest.as_slice() != blake3::hash(&payload).as_bytes() {
        return Err(StoreError::new(
            "verify analysis state",
            "the payload digest does not match; run with --rebuild-store",
        ));
    }
    decode_budget.reserve(payload.len(), "analysis-state envelope")?;
    let envelope: StoredAnalysisEnvelope =
        decode_uncompressed(&payload, "analysis state envelope")?;
    let header = decode_analysis_header(&envelope, decode_budget)?;
    let authority_keys = decode_with_shared_budget::<Vec<super::AppendAuthorityKey>>(
        &envelope.authority_keys,
        "authority keys",
        decode_budget,
    )?
    .into_iter()
    .collect();
    Ok((Some(header), Some(authority_keys), Some(envelope)))
}

fn load_cached_report(
    connection: &Connection,
    options_key: &[u8; 32],
    decode_budget: &DecodedByteBudget,
) -> Result<Option<Vec<u8>>, StoreError> {
    let maximum = i64::try_from(MAXIMUM_COMPRESSED_PAYLOAD_BYTES)
        .expect("compressed payload limit fits SQLite INTEGER");
    let stored = connection
        .query_row(
            "
            SELECT CASE WHEN typeof(report_json) = 'blob' AND length(report_json) <= ?3
                        THEN report_json END,
                   CASE WHEN typeof(report_json) = 'blob'
                        THEN length(report_json) ELSE -1 END,
                   CASE WHEN typeof(report_digest) = 'blob' AND length(report_digest) = 32
                        THEN report_digest END
            FROM cached_report
            WHERE singleton = ?1 AND options_key = ?2
            ",
            params![REPORT_SINGLETON, options_key.as_slice(), maximum],
            |row| {
                Ok((
                    row.get::<_, Option<Vec<u8>>>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, Option<Vec<u8>>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| StoreError::new("read cached report", error.to_string()))?;
    let Some((compressed, compressed_len, digest)) = stored else {
        return Ok(None);
    };
    let digest =
        required_bounded_store_value("read cached report", digest, "cached-report digest")?;
    validate_stored_blob_length(
        "read cached report",
        "cached-report payload",
        compressed_len,
        MAXIMUM_COMPRESSED_PAYLOAD_BYTES,
    )?;
    let compressed = compressed.ok_or_else(|| {
        StoreError::new(
            "read cached report",
            "the cached-report payload exceeded its storage bound",
        )
    })?;
    if digest.as_slice() != blake3::hash(&compressed).as_bytes() {
        return Err(StoreError::new(
            "verify cached report",
            "the payload digest does not match; run with --rebuild-store",
        ));
    }
    let decoded = decompress_bytes_with_shared_budget(&compressed, "cached report", decode_budget)?;
    let report: ccwrapped::Report = serde_json::from_slice(&decoded).map_err(|error| {
        StoreError::new(
            "decode cached report",
            format!("the cached report is not one valid typed JSON report: {error}"),
        )
    })?;
    validate_cached_standard_report(&report)?;
    serde_json::to_vec_pretty(&report)
        .map(Some)
        .map_err(|error| StoreError::new("encode cached report", error.to_string()))
}

fn validate_cached_standard_report(report: &ccwrapped::Report) -> Result<(), StoreError> {
    if report.schema_version != "ccwrapped.report/v2"
        || report.data_coverage.privacy_profile != "standard"
        || report.methodology.pricing_registry.version != super::pricing::REGISTRY_VERSION
        || report.methodology.pricing_registry.citation != super::pricing::REGISTRY_CITATION
        || report.methodology.pricing_registry.access_date != super::pricing::REGISTRY_ACCESS_DATE
        || report.methodology.pricing_registry.selection_policy != super::pricing::SELECTION_POLICY
        || !super::views::reconciliation_passes(&report.canonical_metrics.reconciliation)
    {
        return Err(StoreError::new(
            "validate cached report",
            "the cached report contract does not reconcile; run with --rebuild-store",
        ));
    }
    super::insights::validate(
        &report.insights,
        &report.methodology,
        super::insights::ValidationEvidence::new(
            &report.canonical_metrics,
            report.wrapped_story.total_messages,
        ),
    )
    .map_err(|_| {
        StoreError::new(
            "validate cached report",
            "the cached insight proof objects do not reconcile; run with --rebuild-store",
        )
    })?;

    if report
        .project_breakdown
        .iter()
        .any(|project| project.path.is_some())
        || report
            .session_breakdown
            .sessions
            .iter()
            .any(|session| !standard_session_is_private_free(session))
        || report
            .session_breakdown
            .costly_subagents
            .iter()
            .any(|subagent| !standard_subagent_is_private_free(subagent))
        || report
            .wrapped_story
            .top_project
            .as_ref()
            .is_some_and(|project| project.path.is_some())
        || [
            report.wrapped_story.biggest_session.as_ref(),
            report.wrapped_story.biggest_session_by_cost.as_ref(),
            report.wrapped_story.biggest_session_by_tokens.as_ref(),
        ]
        .into_iter()
        .flatten()
        .any(|session| !standard_session_is_private_free(session))
        || report
            .wrapped_story
            .biggest_subagent
            .as_ref()
            .is_some_and(|subagent| !standard_subagent_is_private_free(subagent))
    {
        return Err(StoreError::new(
            "validate cached report",
            "the cached standard report contains a private-output carrier; run with --rebuild-store",
        ));
    }
    Ok(())
}

fn standard_session_is_private_free(session: &ccwrapped::SessionSummary) -> bool {
    session.project_path.is_none()
        && session.first_prompt.is_none()
        && session.prompts.is_empty()
        && session
            .subagents
            .iter()
            .all(standard_subagent_is_private_free)
}

fn standard_subagent_is_private_free(subagent: &ccwrapped::SubagentSummary) -> bool {
    subagent.first_prompt.is_none() && subagent.project_path.is_none()
}

pub(super) struct FileCache {
    salt: [u8; 32],
    normalization_key: [u8; 32],
    rows: RefCell<HashMap<[u8; 32], StoredFileRow>>,
    payload_connection: Arc<Mutex<Connection>>,
    analysis_header: Option<(Diagnostics, AliasState)>,
    authority_keys: Option<HashSet<super::AppendAuthorityKey>>,
    analysis_state: RefCell<Option<StoredAnalysisEnvelope>>,
    cached_report: Option<Vec<u8>>,
    decode_budget: Arc<DecodedByteBudget>,
}

impl FileCache {
    pub fn open(path: &Path, options: &IngestionOptions) -> Result<Self, StoreError> {
        validate_store_file(path)?;
        let connection = open_connection(path, false)?;
        validate_schema(&connection)?;
        let salt = read_salt(&connection)?;
        let normalization_key = *options_key(options).as_bytes();
        let rows = load_file_rows(&connection)?;
        let decode_budget = Arc::new(DecodedByteBudget::new());
        let (analysis_header, authority_keys, analysis_state) =
            load_analysis_state(&connection, &normalization_key, &decode_budget)?;
        let cached_report = load_cached_report(&connection, &normalization_key, &decode_budget)?;
        Ok(Self {
            salt,
            normalization_key,
            rows: RefCell::new(rows),
            payload_connection: Arc::new(Mutex::new(connection)),
            analysis_header,
            authority_keys,
            analysis_state: RefCell::new(analysis_state),
            cached_report,
            decode_budget,
        })
    }

    pub fn analysis_header(&self) -> Option<(Diagnostics, AliasState)> {
        self.analysis_header.clone()
    }

    pub fn take_analysis_state(&self) -> Result<Option<super::AnalysisState>, StoreError> {
        self.analysis_state
            .borrow_mut()
            .take()
            .map(|envelope| decode_analysis_state_envelope(envelope, &self.decode_budget))
            .transpose()
    }

    pub fn authority_keys(&self) -> Option<&HashSet<super::AppendAuthorityKey>> {
        self.authority_keys.as_ref()
    }

    pub fn cached_report(&self) -> Option<&[u8]> {
        self.cached_report.as_deref()
    }

    pub fn lookup_raw(
        &self,
        path: &Path,
        source_root: &Path,
        source_alias: &str,
        kind: SourceKind,
        snapshot: &FileSnapshot,
    ) -> Result<Option<RawCachedFile>, StoreError> {
        self.lookup_raw_inner(path, source_root, source_alias, kind, snapshot, true)
    }

    pub fn lookup_raw_deferred(
        &self,
        path: &Path,
        source_root: &Path,
        source_alias: &str,
        kind: SourceKind,
        snapshot: &FileSnapshot,
    ) -> Result<Option<RawCachedFile>, StoreError> {
        self.lookup_raw_inner(path, source_root, source_alias, kind, snapshot, false)
    }

    fn lookup_raw_inner(
        &self,
        path: &Path,
        source_root: &Path,
        source_alias: &str,
        kind: SourceKind,
        snapshot: &FileSnapshot,
        load_payload: bool,
    ) -> Result<Option<RawCachedFile>, StoreError> {
        let key = path_key(&self.salt, path);
        let Some(stored) = self.rows.borrow_mut().remove(key.as_bytes()) else {
            return Ok(None);
        };
        let candidate = SourceFile::metadata_only(
            path.to_path_buf(),
            source_root.to_path_buf(),
            source_alias.to_string(),
            kind,
            snapshot.clone(),
        );
        let expected = stored_snapshot(&candidate)?;
        let expected_normalization_key = self.normalization_key.to_vec();
        let expected_source_key = source_key(&self.salt, &candidate).as_bytes().to_vec();
        if (
            &stored.normalization_key,
            &stored.source_key,
            &stored.source_alias,
            stored.source_kind,
            &stored.device,
            &stored.inode,
            stored.size,
            stored.modified_s,
            stored.modified_ns,
            stored.changed_s,
            stored.changed_ns,
        ) != (
            &expected_normalization_key,
            &expected_source_key,
            &expected.0,
            expected.1,
            &expected.2,
            &expected.3,
            expected.4,
            expected.5,
            expected.6,
            expected.7,
            expected.8,
        ) {
            self.rows.borrow_mut().insert(*key.as_bytes(), stored);
            return Ok(None);
        }
        let content_digest: [u8; 32] = stored.content_digest.try_into().map_err(|_| {
            StoreError::new("verify file payload", "content digest is not 32 bytes")
        })?;
        let event_count = usize::try_from(stored.event_count)
            .map_err(|_| StoreError::new("verify file payload", "event count is outside usize"))?;
        let events_available = stored.event_payload_bytes > 0;
        let (event_payload, diagnostics_payload, metric_state, deferred_payload) = if load_payload {
            let connection = self.payload_connection.lock().map_err(|_| {
                StoreError::new(
                    "read file payload",
                    "the payload connection lock was poisoned",
                )
            })?;
            let (events, diagnostics, metric_state) =
                load_file_payload(&connection, key.as_bytes())?;
            (events, diagnostics, metric_state, None)
        } else {
            (
                Vec::new(),
                Vec::new(),
                None,
                Some(DeferredPayload {
                    connection: Arc::clone(&self.payload_connection),
                    path_key: *key.as_bytes(),
                }),
            )
        };
        Ok(Some(RawCachedFile {
            metric_state,
            content_digest,
            event_payload,
            diagnostics_payload,
            deferred_payload,
            events_available,
            file_alias: stored.file_alias,
            event_count,
            decode_budget: Arc::clone(&self.decode_budget),
        }))
    }

    pub fn is_unchanged(
        &self,
        path: &Path,
        source_root: &Path,
        source_alias: &str,
        kind: SourceKind,
        snapshot: &FileSnapshot,
    ) -> Result<bool, StoreError> {
        let key = path_key(&self.salt, path);
        let rows = self.rows.borrow();
        let Some(stored) = rows.get(key.as_bytes()) else {
            return Ok(false);
        };
        let candidate = SourceFile::metadata_only(
            path.to_path_buf(),
            source_root.to_path_buf(),
            source_alias.to_string(),
            kind,
            snapshot.clone(),
        );
        let expected = stored_snapshot(&candidate)?;
        Ok(stored.normalization_key == self.normalization_key
            && stored.source_key == source_key(&self.salt, &candidate).as_bytes()
            && stored.source_alias == source_alias
            && stored.source_kind == expected.1
            && stored.device == expected.2
            && stored.inode == expected.3
            && stored.size == expected.4
            && stored.modified_s == expected.5
            && stored.modified_ns == expected.6
            && stored.changed_s == expected.7
            && stored.changed_ns == expected.8)
    }

    pub fn lookup(
        &self,
        path: &Path,
        source_root: &Path,
        source_alias: &str,
        kind: SourceKind,
        snapshot: &FileSnapshot,
    ) -> Result<Option<CachedFile>, StoreError> {
        let Some(raw) = self.lookup_raw(path, source_root, source_alias, kind, snapshot)? else {
            return Ok(None);
        };
        if !raw.events_available() {
            return Ok(None);
        }
        decode_cached_file(raw).map(Some)
    }

    pub fn take_previous_raw(
        &self,
        path: &Path,
        source_root: &Path,
        source_alias: &str,
        kind: SourceKind,
        snapshot: &FileSnapshot,
    ) -> Result<Option<PreviousCachedFile>, StoreError> {
        let key = path_key(&self.salt, path);
        let Some(stored) = self.rows.borrow_mut().remove(key.as_bytes()) else {
            return Ok(None);
        };
        let candidate = SourceFile::metadata_only(
            path.to_path_buf(),
            source_root.to_path_buf(),
            source_alias.to_string(),
            kind,
            snapshot.clone(),
        );
        let expected_source_key = source_key(&self.salt, &candidate).as_bytes().to_vec();
        if stored.normalization_key != self.normalization_key
            || stored.source_key != expected_source_key
            || stored.source_alias != source_alias
            || stored.source_kind != source_kind(kind)
        {
            self.rows.borrow_mut().insert(*key.as_bytes(), stored);
            return Ok(None);
        }
        let content_digest: [u8; 32] = stored.content_digest.try_into().map_err(|_| {
            StoreError::new(
                "verify prior file payload",
                "content digest is not 32 bytes",
            )
        })?;
        let source_bytes = u64::try_from(stored.size)
            .map_err(|_| StoreError::new("verify prior file payload", "source size is negative"))?;
        let event_count = usize::try_from(stored.event_count).map_err(|_| {
            StoreError::new("verify prior file payload", "event count is outside usize")
        })?;
        let connection = self.payload_connection.lock().map_err(|_| {
            StoreError::new(
                "read prior file payload",
                "the payload connection lock was poisoned",
            )
        })?;
        let (event_payload, diagnostics_payload, metric_state) =
            load_file_payload(&connection, key.as_bytes())?;
        let events_available = !event_payload.is_empty();
        Ok(Some(PreviousCachedFile {
            raw: RawCachedFile {
                content_digest,
                event_payload,
                diagnostics_payload,
                metric_state,
                deferred_payload: None,
                events_available,
                file_alias: stored.file_alias,
                event_count,
                decode_budget: Arc::clone(&self.decode_budget),
            },
            source_bytes,
        }))
    }

    pub fn remaining_rows(&self) -> usize {
        self.rows.borrow().len()
    }

    pub fn has_remaining_source(&self, source_alias: &str, kind: SourceKind) -> bool {
        let stored_kind = source_kind(kind);
        self.rows
            .borrow()
            .values()
            .any(|row| row.source_alias == source_alias && row.source_kind == stored_kind)
    }
}

pub(super) fn decode_cached_file(mut raw: RawCachedFile) -> Result<CachedFile, StoreError> {
    if let Some(deferred) = raw.deferred_payload.take() {
        let connection = deferred.connection.lock().map_err(|_| {
            StoreError::new(
                "read file payload",
                "the payload connection lock was poisoned",
            )
        })?;
        let (events, diagnostics, metric_state) =
            load_file_payload(&connection, &deferred.path_key)?;
        raw.event_payload = events;
        raw.diagnostics_payload = diagnostics;
        raw.metric_state = metric_state;
    }
    let events = if raw.events_available() {
        let stored_events: Vec<StoredEvent> =
            decode_with_shared_budget(&raw.event_payload, "normalized events", &raw.decode_budget)?;
        let events = stored_events
            .into_iter()
            .map(StoredEvent::into_runtime)
            .collect::<Result<Vec<_>, _>>()?;
        if events.len() != raw.event_count {
            return Err(StoreError::new(
                "verify normalized events",
                "the cached event count does not match its payload",
            ));
        }
        events
    } else {
        Vec::new()
    };
    raw.decode_budget
        .reserve(raw.diagnostics_payload.len(), "diagnostics")?;
    let diagnostics: StoredDiagnostics =
        decode_uncompressed(&raw.diagnostics_payload, "diagnostics")?;
    Ok(CachedFile {
        events,
        diagnostics: diagnostics.into_runtime()?,
        metric_state: raw.metric_state,
        content_digest: raw.content_digest,
        event_payload: raw.event_payload,
        diagnostics_payload: raw.diagnostics_payload,
        file_alias: raw.file_alias,
        event_count: raw.event_count,
        decode_budget: raw.decode_budget,
    })
}

#[derive(Debug)]
pub(super) struct StoreError {
    action: &'static str,
    detail: String,
}

impl StoreError {
    fn new(action: &'static str, detail: impl Into<String>) -> Self {
        Self {
            action,
            detail: detail.into(),
        }
    }
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} failed for the local incremental store: {}",
            self.action, self.detail
        )
    }
}

impl std::error::Error for StoreError {}

fn encode<T: serde::Serialize + ?Sized>(
    value: &T,
    label: &'static str,
) -> Result<Vec<u8>, StoreError> {
    let encoded = encode_uncompressed(value, label)?;
    compress_bytes(&encoded, label)
}

fn compress_bytes(encoded: &[u8], label: &'static str) -> Result<Vec<u8>, StoreError> {
    if encoded.len() as u64 > MAXIMUM_PAYLOAD_BYTES {
        return Err(StoreError::new(
            "compress payload",
            format!("{label} exceeds the bounded decoded size"),
        ));
    }
    zstd::stream::encode_all(encoded, CACHE_COMPRESSION_LEVEL)
        .map_err(|error| StoreError::new("compress payload", format!("{label}: {error}")))
}

fn encode_uncompressed<T: serde::Serialize + ?Sized>(
    value: &T,
    label: &'static str,
) -> Result<Vec<u8>, StoreError> {
    bincode::options()
        .with_limit(MAXIMUM_PAYLOAD_BYTES)
        .serialize(value)
        .map_err(|error| StoreError::new("encode payload", format!("{label}: {error}")))
}

pub(super) fn encode_metric_state<T: serde::Serialize + ?Sized>(
    value: &T,
) -> Result<Vec<u8>, StoreError> {
    encode(value, "metric state")
}

fn decode<T: serde::de::DeserializeOwned>(
    compressed: &[u8],
    label: &'static str,
) -> Result<T, StoreError> {
    let encoded = decompress_bytes(compressed, label)?;
    decode_uncompressed(&encoded, label)
}

fn decode_with_budget<T: serde::de::DeserializeOwned>(
    compressed: &[u8],
    label: &'static str,
    remaining: &mut u64,
) -> Result<T, StoreError> {
    let encoded = decompress_bytes_with_limit(compressed, label, *remaining)?;
    *remaining = remaining.saturating_sub(encoded.len() as u64);
    decode_uncompressed(&encoded, label)
}

fn decode_with_shared_budget<T: serde::de::DeserializeOwned>(
    compressed: &[u8],
    label: &'static str,
    budget: &DecodedByteBudget,
) -> Result<T, StoreError> {
    let encoded = decompress_bytes_with_shared_budget(compressed, label, budget)?;
    decode_uncompressed(&encoded, label)
}

fn decompress_bytes(compressed: &[u8], label: &'static str) -> Result<Vec<u8>, StoreError> {
    decompress_bytes_with_limit(compressed, label, MAXIMUM_PAYLOAD_BYTES)
}

fn decompress_bytes_with_shared_budget(
    compressed: &[u8],
    label: &'static str,
    budget: &DecodedByteBudget,
) -> Result<Vec<u8>, StoreError> {
    let mut decoder = zstd::stream::read::Decoder::new(compressed)
        .map_err(|error| StoreError::new("open compressed payload", format!("{label}: {error}")))?;
    let mut encoded = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let read = decoder
            .read(&mut chunk)
            .map_err(|error| StoreError::new("decompress payload", format!("{label}: {error}")))?;
        if read == 0 {
            break;
        }
        let next_len = encoded.len().checked_add(read).ok_or_else(|| {
            StoreError::new(
                "decompress payload",
                format!("{label} exceeds the bounded decoded size"),
            )
        })?;
        if next_len as u64 > MAXIMUM_PAYLOAD_BYTES {
            return Err(StoreError::new(
                "decompress payload",
                format!("{label} exceeds the bounded decoded size"),
            ));
        }
        budget.reserve(read, label)?;
        encoded.extend_from_slice(&chunk[..read]);
    }
    Ok(encoded)
}

fn decompress_bytes_with_limit(
    compressed: &[u8],
    label: &'static str,
    maximum_bytes: u64,
) -> Result<Vec<u8>, StoreError> {
    let decoder = zstd::stream::read::Decoder::new(compressed)
        .map_err(|error| StoreError::new("open compressed payload", format!("{label}: {error}")))?;
    let mut bounded = decoder.take(maximum_bytes.saturating_add(1));
    let mut encoded = Vec::new();
    bounded
        .read_to_end(&mut encoded)
        .map_err(|error| StoreError::new("decompress payload", format!("{label}: {error}")))?;
    if encoded.len() as u64 > maximum_bytes {
        return Err(StoreError::new(
            "decompress payload",
            format!("{label} exceeds the aggregate decoded-size budget"),
        ));
    }
    Ok(encoded)
}

fn decode_uncompressed<T: serde::de::DeserializeOwned>(
    encoded: &[u8],
    label: &'static str,
) -> Result<T, StoreError> {
    if encoded.len() as u64 > MAXIMUM_PAYLOAD_BYTES {
        return Err(StoreError::new(
            "decode payload",
            format!("{label} exceeds the bounded decoded size"),
        ));
    }
    bincode::options()
        .with_limit(MAXIMUM_PAYLOAD_BYTES)
        .deserialize(encoded)
        .map_err(|error| StoreError::new("decode payload", format!("{label}: {error}")))
}

pub(super) fn decode_metric_state<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
    budget: &DecodedByteBudget,
) -> Result<T, StoreError> {
    decode_with_shared_budget(bytes, "metric state", budget)
}

fn known(
    value: &str,
    choices: &[&'static str],
    label: &'static str,
) -> Result<&'static str, StoreError> {
    choices
        .iter()
        .copied()
        .find(|choice| *choice == value)
        .ok_or_else(|| StoreError::new("decode payload", format!("unknown {label} value")))
}

fn valid_source_alias(alias: &str, kind: &str) -> bool {
    let prefix = match kind {
        "transcript" => "transcript-",
        "otel" => "otel-",
        _ => return false,
    };
    alias
        .strip_prefix(prefix)
        .is_some_and(valid_positive_decimal)
}

fn valid_file_alias(alias: &str) -> bool {
    if alias.len() > MAXIMUM_STORED_ALIAS_CHARACTERS as usize {
        return false;
    }
    let Some((source_alias, file_number)) = alias.rsplit_once("-file-") else {
        return false;
    };
    let kind = if source_alias.starts_with("transcript-") {
        "transcript"
    } else if source_alias.starts_with("otel-") {
        "otel"
    } else {
        return false;
    };
    valid_source_alias(source_alias, kind) && valid_positive_decimal(file_number)
}

fn valid_positive_decimal(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('0')
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && value.parse::<usize>().is_ok_and(|number| number > 0)
}

fn valid_warning_code(code: &str) -> bool {
    let Some(suffix) = code.strip_prefix("W_") else {
        return false;
    };
    !suffix.is_empty()
        && code.len() <= 96
        && suffix
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
}

fn valid_safe_token(value: &str, maximum_characters: usize) -> bool {
    !value.is_empty()
        && value.len() <= maximum_characters
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/' | b'[' | b']')
        })
}

fn valid_inert_text(value: &str, maximum_characters: usize) -> bool {
    let mut characters = 0usize;
    value.chars().all(|character| {
        characters = characters.saturating_add(1);
        characters <= maximum_characters
            && !character.is_control()
            && !matches!(
                character,
                '\u{061c}'
                    | '\u{200e}'
                    | '\u{200f}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202a}'..='\u{202e}'
                    | '\u{2066}'..='\u{2069}'
            )
    })
}

fn validate_stored_blob_length(
    action: &'static str,
    label: &'static str,
    length: i64,
    maximum: u64,
) -> Result<(), StoreError> {
    let length = u64::try_from(length).map_err(|_| {
        StoreError::new(
            action,
            format!("the stored {label} has an invalid negative length"),
        )
    })?;
    if length > maximum {
        return Err(StoreError::new(
            action,
            format!("the stored {label} exceeds its bounded size"),
        ));
    }
    Ok(())
}

pub(super) fn lookup_report(
    path: &Path,
    options: &IngestionOptions,
    files: &[SourceFile],
) -> Result<CacheLookup, StoreError> {
    if !path.exists() {
        return Ok(CacheLookup::Miss);
    }
    validate_store_file(path)?;
    let connection = open_connection(path, false)?;
    validate_schema(&connection)?;
    let salt = read_salt(&connection)?;
    if !inventory_matches(&connection, &salt, &options_key(options), files)? {
        return Ok(CacheLookup::Miss);
    }
    let options_key = options_key(options);
    let decode_budget = DecodedByteBudget::new();
    match load_cached_report(&connection, options_key.as_bytes(), &decode_budget)? {
        Some(report) => Ok(CacheLookup::Hit(report)),
        None => Ok(CacheLookup::Miss),
    }
}

pub(super) fn lookup_retained_report(
    path: &Path,
    options: &IngestionOptions,
    selected_sources: &[(String, SourceKind, PathBuf)],
    partial_source_aliases: &HashSet<String>,
    readable_files: &[SourceFile],
) -> Result<CacheLookup, StoreError> {
    if partial_source_aliases.is_empty() || !path.exists() {
        return Ok(CacheLookup::Miss);
    }
    validate_store_file(path)?;
    let connection = open_connection(path, false)?;
    validate_schema(&connection)?;
    let salt = read_salt(&connection)?;
    let normalization_key = options_key(options);

    let rows = load_file_rows(&connection)?;
    let mut stored_counts: HashMap<(String, i64), (Vec<u8>, usize)> = HashMap::new();
    for row in rows.values() {
        if row.normalization_key.as_slice() != normalization_key.as_bytes() {
            return Ok(CacheLookup::Miss);
        }
        let identity = (row.source_alias.clone(), row.source_kind);
        if let Some((stored_source_key, count)) = stored_counts.get_mut(&identity) {
            if stored_source_key != &row.source_key {
                return Err(StoreError::new(
                    "verify retained inventory",
                    "the store contains duplicate source inventory groups",
                ));
            }
            *count = count.saturating_add(1);
        } else {
            stored_counts.insert(identity, (row.source_key.clone(), 1usize));
        }
    }

    let expected_sources = selected_sources
        .iter()
        .map(|(alias, kind, root)| {
            (
                (alias.clone(), source_kind(*kind)),
                source_root_key(&salt, alias, *kind, root)
                    .as_bytes()
                    .to_vec(),
            )
        })
        .collect::<HashMap<_, _>>();
    if stored_counts.len() != expected_sources.len()
        || stored_counts
            .iter()
            .any(|(identity, (stored_key, _))| expected_sources.get(identity) != Some(stored_key))
    {
        return Ok(CacheLookup::Miss);
    }
    if !retained_readable_inventory_matches(
        &connection,
        &salt,
        &normalization_key,
        readable_files,
        &stored_counts,
        partial_source_aliases,
    )? {
        return Err(StoreError::new(
            "verify retained inventory",
            "a readable source changed while another transcript branch was inaccessible; restore complete source access and retry before reusing retained facts",
        ));
    }

    let decode_budget = DecodedByteBudget::new();
    match load_cached_report(&connection, normalization_key.as_bytes(), &decode_budget)? {
        Some(report) => Ok(CacheLookup::Hit(report)),
        None => Ok(CacheLookup::Miss),
    }
}

pub(super) fn prepare(path: &Path, rebuild: bool) -> Result<PreparedStore, StoreError> {
    prepare_store_path(path)?;
    let store_lock = acquire_store_lock(path)?;
    if rebuild {
        return prepare_staged_store(path, None, store_lock);
    }
    if path.exists() {
        match inspect_existing_store(path)? {
            ExistingStore::Current(salt) => {
                prepare_current_store_artifact(path)?;
                return Ok(PreparedStore {
                    path: path.to_path_buf(),
                    salt,
                    rebuild_destination: None,
                    _lock: store_lock,
                });
            }
            ExistingStore::Legacy(salt) => {
                return prepare_staged_store(path, Some(salt), store_lock);
            }
        }
    }
    let salt = prepare_current_store_artifact(path)?;
    Ok(PreparedStore {
        path: path.to_path_buf(),
        salt,
        rebuild_destination: None,
        _lock: store_lock,
    })
}

fn prepare_staged_store(
    destination: &Path,
    salt: Option<[u8; 32]>,
    store_lock: StoreLock,
) -> Result<PreparedStore, StoreError> {
    for _ in 0..MAXIMUM_REBUILD_STAGE_ATTEMPTS {
        let staging = rebuild_staging_path(destination)?;
        match fs::symlink_metadata(&staging) {
            Ok(_) => continue,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::new("prepare rebuild", error.to_string())),
        }
        match prepare_new_store_artifact(&staging, salt) {
            Ok(staged_salt) => {
                return Ok(PreparedStore {
                    path: staging,
                    salt: staged_salt,
                    rebuild_destination: Some(destination.to_path_buf()),
                    _lock: store_lock,
                });
            }
            Err(error) if error.action == "create" => {
                let _ = remove_regular_store_artifact(&journal_path(&staging));
                let _ = remove_regular_store_artifact(&staging);
                continue;
            }
            Err(error) => {
                let _ = remove_regular_store_artifact(&journal_path(&staging));
                let _ = remove_regular_store_artifact(&staging);
                return Err(error);
            }
        }
    }
    Err(StoreError::new(
        "prepare rebuild",
        "could not allocate a unique private staging database",
    ))
}

fn prepare_current_store_artifact(path: &Path) -> Result<[u8; 32], StoreError> {
    let is_new = !path.exists();
    if is_new {
        return prepare_new_store_artifact(path, None);
    } else {
        validate_store_file(path)?;
    }
    prepare_private_journal(path)?;
    let connection = open_connection(path, true)?;
    configure_write(&connection)?;
    validate_schema(&connection)?;
    let salt = read_salt(&connection)?;
    drop(connection);
    enforce_private_file(path)?;
    enforce_private_journal(path)?;
    Ok(salt)
}

fn prepare_new_store_artifact(path: &Path, salt: Option<[u8; 32]>) -> Result<[u8; 32], StoreError> {
    create_private_file(path)?;
    prepare_private_journal(path)?;
    let connection = open_connection(path, true)?;
    configure_write(&connection)?;
    let salt = match salt {
        Some(salt) => salt,
        None => {
            let mut salt = [0u8; 32];
            getrandom::fill(&mut salt)
                .map_err(|error| StoreError::new("generate salt", error.to_string()))?;
            salt
        }
    };
    initialize_schema_with_salt(&connection, salt)?;
    validate_schema(&connection)?;
    drop(connection);
    enforce_private_file(path)?;
    enforce_private_journal(path)?;
    Ok(salt)
}

enum ExistingStore {
    Current([u8; 32]),
    Legacy([u8; 32]),
}

fn inspect_existing_store(path: &Path) -> Result<ExistingStore, StoreError> {
    validate_store_file(path)?;
    let connection = open_connection(path, false)?;
    let integrity = connection
        .pragma_query_value(None, "quick_check", |row| row.get::<_, String>(0))
        .map_err(|error| StoreError::new("verify", error.to_string()))?;
    if integrity != "ok" {
        return Err(StoreError::new(
            "verify",
            "the database integrity check failed; run with --rebuild-store",
        ));
    }
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| StoreError::new("read schema version", error.to_string()))?;
    if version == STORE_SCHEMA_VERSION {
        validate_schema(&connection)?;
        return read_salt(&connection).map(ExistingStore::Current);
    }
    let expected_format = legacy_store_format(version).ok_or_else(|| {
        StoreError::new(
            "migrate",
            format!(
                "schema version {version} is unsupported; run with --rebuild-store to replace derived state"
            ),
        )
    })?;
    let format = connection
        .query_row(
            "
            SELECT CASE WHEN typeof(value) = 'blob' AND length(value) <= 64
                        THEN value END
            FROM meta WHERE key = 'format'
            ",
            [],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .map_err(|error| StoreError::new("read migration format", error.to_string()))?
        .ok_or_else(|| {
            StoreError::new(
                "migrate",
                "the prior schema format metadata has an invalid type or size; run with --rebuild-store",
            )
        })?;
    if format != expected_format {
        return Err(StoreError::new(
            "migrate",
            "the prior schema has an unknown format; run with --rebuild-store",
        ));
    }
    read_salt(&connection).map(ExistingStore::Legacy)
}

fn legacy_store_format(version: i64) -> Option<&'static [u8]> {
    match version {
        1 => Some(b"ccwrapped.incremental-store/v1"),
        2 => Some(b"ccwrapped.incremental-store/v2"),
        3 => Some(b"ccwrapped.incremental-store/v3"),
        4 => Some(b"ccwrapped.incremental-store/v4"),
        5 => Some(b"ccwrapped.incremental-store/v5"),
        6 => Some(b"ccwrapped.incremental-store/v6"),
        7 => Some(b"ccwrapped.incremental-store/v7"),
        8 => Some(b"ccwrapped.incremental-store/v8"),
        _ => None,
    }
}

fn acquire_store_lock(destination: &Path) -> Result<StoreLock, StoreError> {
    let lock_path = store_lock_path(destination);
    match create_private_file(&lock_path) {
        Ok(()) => {}
        Err(error) if error.action == "create" && lock_path.exists() => {
            validate_store_file(&lock_path)?;
        }
        Err(error) => return Err(error),
    }
    let connection = open_connection(&lock_path, true)?;
    connection
        .execute_batch(
            "
            PRAGMA busy_timeout = 30000;
            PRAGMA trusted_schema = OFF;
            PRAGMA synchronous = FULL;
            BEGIN IMMEDIATE;
            CREATE TABLE IF NOT EXISTS lease (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1)
            ) STRICT;
            ",
        )
        .map_err(|error| {
            StoreError::new(
                "lock store",
                format!(
                    "{error}; wait for the other ccwrapped invocation using this store to finish"
                ),
            )
        })?;
    enforce_private_file(&lock_path)?;
    enforce_private_journal(&lock_path)?;
    Ok(StoreLock { connection })
}

fn store_lock_path(destination: &Path) -> PathBuf {
    let mut value = destination.as_os_str().to_os_string();
    value.push(".lock.sqlite3");
    PathBuf::from(value)
}

pub(super) fn publish_report(
    path: &Path,
    options: &IngestionOptions,
    files: &[SourceFile],
    analysis_state: &super::AnalysisState,
    encoded_analysis_state: Option<&[u8]>,
    invalidate_analysis_state: bool,
    report_json: &[u8],
) -> Result<(), StoreError> {
    validate_store_file(path)?;
    let mut connection = open_connection(path, true)?;
    configure_write(&connection)?;
    validate_schema(&connection)?;
    let salt = read_salt(&connection)?;
    let options_key = options_key(options);
    let owned_analysis_payload;
    let analysis_payload = if invalidate_analysis_state {
        None
    } else if let Some(encoded) = encoded_analysis_state {
        Some(encoded)
    } else {
        owned_analysis_payload = encode_analysis_state(analysis_state)?;
        Some(owned_analysis_payload.as_slice())
    };
    let compressed_report = compress_bytes(report_json, "cached report")?;
    let report_digest = blake3::hash(&compressed_report);
    let transaction = connection
        .transaction()
        .map_err(|error| StoreError::new("begin transaction", error.to_string()))?;
    replace_inventory(&transaction, &salt, &options_key, files)?;
    if let Some(analysis_payload) = analysis_payload {
        replace_analysis_state(&transaction, &options_key, analysis_payload)?;
    } else {
        invalidate_stored_analysis_state(&transaction, &options_key)?;
    }
    transaction
        .execute(
            "
            INSERT INTO cached_report (
                singleton, options_key, report_json, report_digest
            ) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(singleton) DO UPDATE SET
                options_key = excluded.options_key,
                report_json = excluded.report_json,
                report_digest = excluded.report_digest
            ",
            params![
                REPORT_SINGLETON,
                options_key.as_bytes().as_slice(),
                compressed_report,
                report_digest.as_bytes().as_slice(),
            ],
        )
        .map_err(|error| StoreError::new("stage report", error.to_string()))?;
    transaction
        .commit()
        .map_err(|error| StoreError::new("commit transaction", error.to_string()))?;
    drop(connection);
    enforce_private_file(path)?;
    enforce_private_journal(path)?;
    Ok(())
}

fn prepare_store_path(path: &Path) -> Result<(), StoreError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| StoreError::new("prepare", "the store path has no parent directory"))?;
    prepare_private_directory(parent)?;
    Ok(())
}

fn rebuild_staging_path(destination: &Path) -> Result<PathBuf, StoreError> {
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| StoreError::new("prepare rebuild", "the store path has no parent"))?;
    let mut nonce = [0u8; 16];
    getrandom::fill(&mut nonce)
        .map_err(|error| StoreError::new("prepare rebuild", error.to_string()))?;
    let first = u64::from_le_bytes(nonce[..8].try_into().expect("eight-byte nonce half"));
    let second = u64::from_le_bytes(nonce[8..].try_into().expect("eight-byte nonce half"));
    Ok(parent.join(format!(
        ".ccwrapped-store-rebuild-{first:016x}{second:016x}.sqlite3"
    )))
}

fn initialize_schema(connection: &Connection) -> Result<(), StoreError> {
    let mut salt = [0u8; 32];
    getrandom::fill(&mut salt)
        .map_err(|error| StoreError::new("generate salt", error.to_string()))?;
    initialize_schema_with_salt(connection, salt)
}

fn initialize_schema_with_salt(connection: &Connection, salt: [u8; 32]) -> Result<(), StoreError> {
    connection
        .execute_batch(
            "
            BEGIN IMMEDIATE;
            CREATE TABLE meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            ) STRICT, WITHOUT ROWID;
            CREATE TABLE source_file (
                path_key BLOB PRIMARY KEY CHECK (length(path_key) = 32),
                normalization_key BLOB NOT NULL CHECK (length(normalization_key) = 32),
                source_key BLOB NOT NULL CHECK (length(source_key) = 32),
                source_alias TEXT NOT NULL CHECK (length(source_alias) <= 256),
                source_kind INTEGER NOT NULL CHECK (source_kind IN (1, 2)),
                file_alias TEXT NOT NULL CHECK (length(file_alias) <= 256),
                event_count INTEGER NOT NULL CHECK (event_count BETWEEN 0 AND 1000000),
                device BLOB NOT NULL CHECK (length(device) = 8),
                inode BLOB NOT NULL CHECK (length(inode) = 8),
                size INTEGER NOT NULL CHECK (size >= 0),
                modified_seconds INTEGER NOT NULL,
                modified_nanoseconds INTEGER NOT NULL,
                changed_seconds INTEGER NOT NULL,
                changed_nanoseconds INTEGER NOT NULL,
                content_digest BLOB NOT NULL CHECK (length(content_digest) = 32),
                normalized_events BLOB NOT NULL,
                diagnostics BLOB NOT NULL,
                metric_state BLOB
            ) STRICT, WITHOUT ROWID;
            CREATE TABLE cached_report (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                options_key BLOB NOT NULL CHECK (length(options_key) = 32),
                report_json BLOB NOT NULL,
                report_digest BLOB NOT NULL CHECK (length(report_digest) = 32)
            ) STRICT;
            CREATE TABLE analysis_state (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                options_key BLOB NOT NULL CHECK (length(options_key) = 32),
                payload BLOB NOT NULL,
                payload_digest BLOB NOT NULL CHECK (length(payload_digest) = 32)
            ) STRICT;
            ",
        )
        .map_err(|error| StoreError::new("initialize schema", error.to_string()))?;
    connection
        .execute(
            "INSERT INTO meta (key, value) VALUES ('format', ?1), ('salt', ?2)",
            params![STORE_FORMAT.as_bytes(), salt.as_slice()],
        )
        .map_err(|error| StoreError::new("initialize metadata", error.to_string()))?;
    connection
        .pragma_update(None, "user_version", STORE_SCHEMA_VERSION)
        .map_err(|error| StoreError::new("set schema version", error.to_string()))?;
    connection
        .execute_batch("COMMIT;")
        .map_err(|error| StoreError::new("commit schema", error.to_string()))
}

fn validate_schema(connection: &Connection) -> Result<(), StoreError> {
    let integrity = connection
        .pragma_query_value(None, "quick_check", |row| row.get::<_, String>(0))
        .map_err(|error| {
            StoreError::new(
                "verify",
                format!("{error}; run with --rebuild-store to replace derived state"),
            )
        })?;
    if integrity != "ok" {
        return Err(StoreError::new(
            "verify",
            "the database integrity check failed; run with --rebuild-store",
        ));
    }
    let version = connection
        .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
        .map_err(|error| StoreError::new("read schema version", error.to_string()))?;
    if version != STORE_SCHEMA_VERSION {
        return Err(StoreError::new(
            "migrate",
            format!(
                "schema version {version} is unsupported; run with --rebuild-store to replace derived state"
            ),
        ));
    }
    let format = connection
        .query_row(
            "
            SELECT CASE WHEN typeof(value) = 'blob' AND length(value) <= 64
                        THEN value END
            FROM meta WHERE key = 'format'
            ",
            [],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .map_err(|error| StoreError::new("read format", error.to_string()))?;
    let format = format.ok_or_else(|| {
        StoreError::new(
            "verify",
            "the store format metadata has an invalid type or size; run with --rebuild-store",
        )
    })?;
    if format != STORE_FORMAT.as_bytes() {
        return Err(StoreError::new(
            "verify",
            "the store format is unsupported; run with --rebuild-store",
        ));
    }
    Ok(())
}

fn configure_write(connection: &Connection) -> Result<(), StoreError> {
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = TRUNCATE;
            PRAGMA journal_size_limit = 0;
            PRAGMA synchronous = FULL;
            PRAGMA foreign_keys = ON;
            PRAGMA trusted_schema = OFF;
            PRAGMA temp_store = MEMORY;
            PRAGMA secure_delete = FAST;
            PRAGMA busy_timeout = 5000;
            ",
        )
        .map_err(|error| StoreError::new("configure", error.to_string()))
}

fn open_connection(path: &Path, writable: bool) -> Result<Connection, StoreError> {
    let flags = if writable {
        OpenFlags::SQLITE_OPEN_READ_WRITE
    } else {
        OpenFlags::SQLITE_OPEN_READ_ONLY
    } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    Connection::open_with_flags(path, flags).map_err(|error| {
        StoreError::new(
            "open",
            format!("{error}; run with --rebuild-store if this derived database is corrupt"),
        )
    })
}

fn read_salt(connection: &Connection) -> Result<[u8; 32], StoreError> {
    let salt = connection
        .query_row(
            "
            SELECT CASE WHEN typeof(value) = 'blob' AND length(value) = 32
                        THEN value END
            FROM meta WHERE key = 'salt'
            ",
            [],
            |row| row.get::<_, Option<Vec<u8>>>(0),
        )
        .map_err(|error| StoreError::new("read salt", error.to_string()))?;
    let salt = salt.ok_or_else(|| {
        StoreError::new(
            "verify",
            "the store salt has an invalid type or size; run with --rebuild-store",
        )
    })?;
    salt.try_into()
        .map_err(|_| StoreError::new("verify", "the store salt is not 32 bytes"))
}

fn inventory_matches(
    connection: &Connection,
    salt: &[u8; 32],
    normalization_key: &blake3::Hash,
    files: &[SourceFile],
) -> Result<bool, StoreError> {
    let stored_count = connection
        .query_row("SELECT count(*) FROM source_file", [], |row| {
            row.get::<_, usize>(0)
        })
        .map_err(|error| StoreError::new("count inventory", error.to_string()))?;
    if stored_count != files.len() {
        return Ok(false);
    }
    let mut select = connection
        .prepare(
            "
            SELECT normalization_key, source_key, source_alias, source_kind,
                   device, inode, size,
                   modified_seconds, modified_nanoseconds,
                   changed_seconds, changed_nanoseconds
            FROM source_file WHERE path_key = ?1
              AND typeof(normalization_key) = 'blob' AND length(normalization_key) = 32
              AND typeof(source_key) = 'blob' AND length(source_key) = 32
              AND typeof(source_alias) = 'text' AND length(source_alias) <= ?2
              AND source_kind IN (1, 2)
              AND typeof(device) = 'blob' AND length(device) = 8
              AND typeof(inode) = 'blob' AND length(inode) = 8
            ",
        )
        .map_err(|error| StoreError::new("prepare inventory check", error.to_string()))?;
    for file in files {
        let key = path_key(salt, &file.path);
        let stored = select
            .query_row(
                params![key.as_bytes().as_slice(), MAXIMUM_STORED_ALIAS_CHARACTERS],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| StoreError::new("check inventory", error.to_string()))?;
        let source_key = source_key(salt, file).as_bytes().to_vec();
        let snapshot = stored_snapshot(file)?;
        let expected = (
            normalization_key.as_bytes().to_vec(),
            source_key,
            snapshot.0,
            snapshot.1,
            snapshot.2,
            snapshot.3,
            snapshot.4,
            snapshot.5,
            snapshot.6,
            snapshot.7,
            snapshot.8,
        );
        if stored.as_ref() != Some(&expected) {
            return Ok(false);
        }
    }
    Ok(true)
}

fn retained_readable_inventory_matches(
    connection: &Connection,
    salt: &[u8; 32],
    normalization_key: &blake3::Hash,
    readable_files: &[SourceFile],
    stored_counts: &HashMap<(String, i64), (Vec<u8>, usize)>,
    partial_source_aliases: &HashSet<String>,
) -> Result<bool, StoreError> {
    let mut readable_counts = HashMap::new();
    let mut select = connection
        .prepare(
            "
            SELECT normalization_key, source_key, source_alias, source_kind,
                   device, inode, size,
                   modified_seconds, modified_nanoseconds,
                   changed_seconds, changed_nanoseconds
            FROM source_file WHERE path_key = ?1
              AND typeof(normalization_key) = 'blob' AND length(normalization_key) = 32
              AND typeof(source_key) = 'blob' AND length(source_key) = 32
              AND typeof(source_alias) = 'text' AND length(source_alias) <= ?2
              AND source_kind IN (1, 2)
              AND typeof(device) = 'blob' AND length(device) = 8
              AND typeof(inode) = 'blob' AND length(inode) = 8
            ",
        )
        .map_err(|error| {
            StoreError::new("prepare retained readable inventory", error.to_string())
        })?;
    for file in readable_files {
        let identity = (file.source_alias.clone(), source_kind(file.kind));
        let count = readable_counts.entry(identity).or_insert(0usize);
        *count = count.saturating_add(1);

        let key = path_key(salt, &file.path);
        let stored = select
            .query_row(
                params![key.as_bytes().as_slice(), MAXIMUM_STORED_ALIAS_CHARACTERS],
                |row| {
                    Ok((
                        row.get::<_, Vec<u8>>(0)?,
                        row.get::<_, Vec<u8>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, Vec<u8>>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, i64>(10)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| {
                StoreError::new("check retained readable inventory", error.to_string())
            })?;
        let snapshot = stored_snapshot(file)?;
        let expected = (
            normalization_key.as_bytes().to_vec(),
            source_key(salt, file).as_bytes().to_vec(),
            snapshot.0,
            snapshot.1,
            snapshot.2,
            snapshot.3,
            snapshot.4,
            snapshot.5,
            snapshot.6,
            snapshot.7,
            snapshot.8,
        );
        if stored.as_ref() != Some(&expected) {
            return Ok(false);
        }
    }

    if readable_counts
        .keys()
        .any(|identity| !stored_counts.contains_key(identity))
    {
        return Ok(false);
    }
    for (identity, (_, stored_count)) in stored_counts {
        let readable_count = readable_counts.get(identity).copied().unwrap_or(0);
        if partial_source_aliases.contains(&identity.0) {
            if readable_count > *stored_count {
                return Ok(false);
            }
        } else if readable_count != *stored_count {
            return Ok(false);
        }
    }
    Ok(true)
}

fn replace_analysis_state(
    transaction: &Transaction<'_>,
    options_key: &blake3::Hash,
    payload: &[u8],
) -> Result<(), StoreError> {
    let digest = blake3::hash(payload);
    transaction
        .execute(
            "
            INSERT INTO analysis_state (
                singleton, options_key, payload, payload_digest
            ) VALUES (?1, ?2, ?3, ?4)
            ON CONFLICT(singleton) DO UPDATE SET
                options_key = excluded.options_key,
                payload = excluded.payload,
                payload_digest = excluded.payload_digest
            ",
            params![
                REPORT_SINGLETON,
                options_key.as_bytes().as_slice(),
                payload,
                digest.as_bytes().as_slice(),
            ],
        )
        .map_err(|error| StoreError::new("write analysis state", error.to_string()))?;
    Ok(())
}

fn invalidate_stored_analysis_state(
    transaction: &Transaction<'_>,
    options_key: &blake3::Hash,
) -> Result<(), StoreError> {
    let mut invalid_options_key = *options_key.as_bytes();
    invalid_options_key[0] ^= u8::MAX;
    transaction
        .execute(
            "
            UPDATE analysis_state
            SET options_key = ?1
            WHERE singleton = ?2
            ",
            params![invalid_options_key.as_slice(), REPORT_SINGLETON],
        )
        .map_err(|error| StoreError::new("invalidate analysis state", error.to_string()))?;
    Ok(())
}

fn replace_inventory(
    transaction: &Transaction<'_>,
    salt: &[u8; 32],
    normalization_key: &blake3::Hash,
    files: &[SourceFile],
) -> Result<(), StoreError> {
    transaction
        .execute_batch(
            "
            CREATE TEMP TABLE current_source (
                path_key BLOB PRIMARY KEY
            ) WITHOUT ROWID;
            ",
        )
        .map_err(|error| StoreError::new("stage current inventory", error.to_string()))?;
    {
        let mut stage_key = transaction
            .prepare("INSERT INTO current_source (path_key) VALUES (?1)")
            .map_err(|error| StoreError::new("prepare current keys", error.to_string()))?;
        for file in files {
            let key = path_key(salt, &file.path);
            stage_key
                .execute(params![key.as_bytes().as_slice()])
                .map_err(|error| StoreError::new("stage current key", error.to_string()))?;
        }
    }
    let mut insert = transaction
        .prepare(
            "
            INSERT INTO source_file (
                path_key, normalization_key, source_key, source_alias, source_kind, file_alias,
                event_count, device, inode, size,
                modified_seconds, modified_nanoseconds,
                changed_seconds, changed_nanoseconds, content_digest,
                normalized_events, diagnostics, metric_state
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18
            )
            ON CONFLICT(path_key) DO UPDATE SET
                normalization_key = excluded.normalization_key,
                source_key = excluded.source_key,
                source_alias = excluded.source_alias,
                source_kind = excluded.source_kind,
                file_alias = excluded.file_alias,
                event_count = excluded.event_count,
                device = excluded.device,
                inode = excluded.inode,
                size = excluded.size,
                modified_seconds = excluded.modified_seconds,
                modified_nanoseconds = excluded.modified_nanoseconds,
                changed_seconds = excluded.changed_seconds,
                changed_nanoseconds = excluded.changed_nanoseconds,
                content_digest = excluded.content_digest,
                normalized_events = excluded.normalized_events,
                diagnostics = excluded.diagnostics,
                metric_state = excluded.metric_state
            ",
        )
        .map_err(|error| StoreError::new("prepare inventory write", error.to_string()))?;
    for file in files {
        if file.reused() {
            continue;
        }
        let key = path_key(salt, &file.path);
        let (alias, kind, device, inode, size, modified_s, modified_ns, changed_s, changed_ns) =
            stored_snapshot(file)?;
        let event_count = i64::try_from(file.event_count).map_err(|_| {
            StoreError::new("write inventory", "event count exceeds SQLite INTEGER")
        })?;
        insert
            .execute(params![
                key.as_bytes().as_slice(),
                normalization_key.as_bytes().as_slice(),
                source_key(salt, file).as_bytes().as_slice(),
                alias,
                kind,
                file.file_alias,
                event_count,
                device,
                inode,
                size,
                modified_s,
                modified_ns,
                changed_s,
                changed_ns,
                file.content_digest.as_slice(),
                file.events.as_deref().ok_or_else(|| {
                    StoreError::new("write inventory", "a source file has no normalized payload")
                })?,
                file.diagnostics.as_deref().ok_or_else(|| {
                    StoreError::new(
                        "write inventory",
                        "a source file has no diagnostics payload",
                    )
                })?,
                file.metric_state.as_deref(),
            ])
            .map_err(|error| StoreError::new("write inventory", error.to_string()))?;
    }
    drop(insert);
    let mut update_reused = transaction
        .prepare(
            "
            UPDATE source_file
            SET file_alias = ?1
            WHERE path_key = ?2 AND file_alias <> ?1
            ",
        )
        .map_err(|error| StoreError::new("prepare reused alias update", error.to_string()))?;
    for file in files.iter().filter(|file| file.reused()) {
        update_reused
            .execute(params![
                file.file_alias,
                path_key(salt, &file.path).as_bytes().as_slice(),
            ])
            .map_err(|error| StoreError::new("update reused alias", error.to_string()))?;
    }
    drop(update_reused);
    transaction
        .execute(
            "
            DELETE FROM source_file
            WHERE NOT EXISTS (
                SELECT 1 FROM current_source
                WHERE current_source.path_key = source_file.path_key
            )
            ",
            [],
        )
        .map_err(|error| StoreError::new("reconcile deleted files", error.to_string()))?;
    transaction
        .execute_batch("DROP TABLE current_source;")
        .map_err(|error| StoreError::new("finish current inventory", error.to_string()))?;
    Ok(())
}

type StoredSnapshot = (String, i64, Vec<u8>, Vec<u8>, i64, i64, i64, i64, i64);

fn stored_snapshot(file: &SourceFile) -> Result<StoredSnapshot, StoreError> {
    let size = i64::try_from(file.snapshot.len())
        .map_err(|_| StoreError::new("snapshot", "a source file exceeds SQLite INTEGER"))?;
    let (device, inode, modified_s, modified_ns, changed_s, changed_ns) =
        file.snapshot.store_identity();
    Ok((
        file.source_alias.clone(),
        source_kind(file.kind),
        device.to_le_bytes().to_vec(),
        inode.to_le_bytes().to_vec(),
        size,
        modified_s,
        modified_ns,
        changed_s,
        changed_ns,
    ))
}

fn source_kind(kind: SourceKind) -> i64 {
    match kind {
        SourceKind::Transcript => 1,
        SourceKind::Otel => 2,
    }
}

fn path_key(salt: &[u8; 32], path: &Path) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new_keyed(salt);
    hasher.update(b"ccwrapped-source-path/v1\0");
    update_path(&mut hasher, path);
    hasher.finalize()
}

fn source_key(salt: &[u8; 32], file: &SourceFile) -> blake3::Hash {
    source_root_key(salt, &file.source_alias, file.kind, &file.source_root)
}

fn source_root_key(
    salt: &[u8; 32],
    source_alias: &str,
    kind: SourceKind,
    root: &Path,
) -> blake3::Hash {
    let mut hasher = blake3::Hasher::new_keyed(salt);
    hasher.update(b"ccwrapped-source-root/v1\0");
    hasher.update(&source_kind(kind).to_le_bytes());
    hasher.update(source_alias.as_bytes());
    update_path(&mut hasher, root);
    hasher.finalize()
}

#[cfg(unix)]
fn update_path(hasher: &mut Hasher, path: &Path) {
    use std::os::unix::ffi::OsStrExt;
    hasher.update(path.as_os_str().as_bytes());
}

#[cfg(windows)]
fn update_path(hasher: &mut Hasher, path: &Path) {
    use std::os::windows::ffi::OsStrExt;
    for unit in path.as_os_str().encode_wide() {
        hasher.update(&unit.to_le_bytes());
    }
}

#[cfg(not(any(unix, windows)))]
fn update_path(hasher: &mut Hasher, path: &Path) {
    hasher.update(path.to_string_lossy().as_bytes());
}

fn options_key(options: &IngestionOptions) -> blake3::Hash {
    let mut hasher = Hasher::new();
    hasher.update(b"ccwrapped-store-options/v1\0");
    hasher.update(env!("CARGO_PKG_VERSION").as_bytes());
    hasher.update(NORMALIZED_SCHEMA.as_bytes());
    hasher.update(TRANSCRIPT_ADAPTER.as_bytes());
    hasher.update(OTEL_ADAPTER.as_bytes());
    hasher.update(options.time_context.name().as_bytes());
    hasher.update(options.time_context.database_version().as_bytes());
    hasher.update(
        &options
            .time_context
            .year()
            .unwrap_or_default()
            .to_le_bytes(),
    );
    hasher.update(&options.active_threshold_seconds.to_le_bytes());
    hasher.update(&[u8::from(options.timezone_fallback)]);
    hasher.finalize()
}

fn prepare_private_directory(path: &Path) -> Result<(), StoreError> {
    #[cfg(windows)]
    {
        prepare_windows_private_directory(path)
    }

    #[cfg(not(windows))]
    {
        if path.exists() {
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| StoreError::new("inspect directory", error.to_string()))?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(StoreError::new(
                    "prepare directory",
                    "the store parent must be a real directory",
                ));
            }
            validate_existing_store_parent(path, &metadata)?;
        } else {
            fs::create_dir_all(path)
                .map_err(|error| StoreError::new("create directory", error.to_string()))?;
            let metadata = fs::symlink_metadata(path)
                .map_err(|error| StoreError::new("inspect created directory", error.to_string()))?;
            if !metadata.file_type().is_dir() || metadata.file_type().is_symlink() {
                return Err(StoreError::new(
                    "prepare directory",
                    "the created store parent is not a real directory",
                ));
            }
            enforce_private_directory(path)?;
        }
        Ok(())
    }
}

#[cfg(windows)]
fn prepare_windows_private_directory(path: &Path) -> Result<(), StoreError> {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;

    let mut missing = Vec::new();
    let mut existing = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(metadata) => {
                if !metadata.file_type().is_dir()
                    || metadata.file_type().is_symlink()
                    || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
                {
                    return Err(StoreError::new(
                        "prepare directory",
                        "the store path and its ancestors must be real directories",
                    ));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(existing.clone());
                existing = existing
                    .parent()
                    .filter(|parent| !parent.as_os_str().is_empty())
                    .unwrap_or_else(|| Path::new("."))
                    .to_path_buf();
            }
            Err(error) => {
                return Err(StoreError::new("inspect directory", error.to_string()));
            }
        }
    }

    for ancestor in existing.ancestors() {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|error| StoreError::new("inspect directory", error.to_string()))?;
        if !metadata.file_type().is_dir()
            || metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        {
            return Err(StoreError::new(
                "verify directory",
                "the store path and its ancestors must not traverse reparse points",
            ));
        }
    }
    let canonical_existing = fs::canonicalize(&existing)
        .map_err(|error| StoreError::new("inspect directory", error.to_string()))?;
    match crate::windows_private_acl::ancestor_chain_is_safe(&canonical_existing) {
        Ok(true) => {}
        Ok(false) => {
            return Err(StoreError::new(
                "verify directory",
                "a store ancestor grants delete or ACL-takeover rights to another principal; choose a private directory",
            ));
        }
        Err(error) => return Err(StoreError::new("verify directory", error.to_string())),
    }

    for directory in missing.iter().rev() {
        crate::windows_private_acl::create_private_directory_new(directory)
            .map_err(|error| StoreError::new("create directory", error.to_string()))?;
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| StoreError::new("inspect created directory", error.to_string()))?;
    if !metadata.file_type().is_dir()
        || metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(StoreError::new(
            "prepare directory",
            "the created store parent is not a real directory",
        ));
    }
    validate_existing_store_parent(path, &metadata)
}

#[cfg(unix)]
fn validate_existing_store_parent(_path: &Path, metadata: &fs::Metadata) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;

    if metadata.permissions().mode() & 0o022 != 0 {
        return Err(StoreError::new(
            "verify directory",
            "an existing store parent must not be writable by group or other users; choose a private directory",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_existing_store_parent(path: &Path, _metadata: &fs::Metadata) -> Result<(), StoreError> {
    match crate::windows_private_acl::is_protected_for_current_user(path) {
        Ok(true) => Ok(()),
        Ok(false) => Err(StoreError::new(
            "verify directory",
            "an existing store parent must have a protected current-user-only ACL; choose a private directory",
        )),
        Err(error) => Err(StoreError::new("verify directory", error.to_string())),
    }
}

#[cfg(not(any(unix, windows)))]
fn validate_existing_store_parent(
    _path: &Path,
    _metadata: &fs::Metadata,
) -> Result<(), StoreError> {
    Err(StoreError::new(
        "verify directory",
        "the incremental store is unsupported on this platform",
    ))
}

fn create_private_file(path: &Path) -> Result<(), StoreError> {
    #[cfg(windows)]
    {
        crate::windows_private_acl::create_private_new(path)
            .map_err(|error| StoreError::new("create", error.to_string()))
    }
    #[cfg(not(windows))]
    {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(path)
            .map_err(|error| StoreError::new("create", error.to_string()))?;
        enforce_private_file(path)
    }
}

fn prepare_private_journal(path: &Path) -> Result<(), StoreError> {
    let journal = journal_path(path);
    if journal.exists() {
        validate_store_file(&journal)?;
    } else {
        create_private_file(&journal)?;
    }
    Ok(())
}

fn validate_store_file(path: &Path) -> Result<(), StoreError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| StoreError::new("inspect", error.to_string()))?;
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(StoreError::new(
            "verify",
            "the store artifact must be a regular file, not a link",
        ));
    }
    enforce_private_file(path)
}

fn remove_regular_store_artifact(path: &Path) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            fs::remove_file(path).map_err(|error| StoreError::new("rebuild", error.to_string()))
        }
        Ok(_) => Err(StoreError::new(
            "rebuild",
            "refusing to replace a non-regular store artifact",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StoreError::new("rebuild", error.to_string())),
    }
}

fn remove_completed_rebuild_journal(path: &Path) -> Result<(), StoreError> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if metadata.file_type().is_file()
                && !metadata.file_type().is_symlink()
                && metadata.len() == 0 =>
        {
            fs::remove_file(path)
                .map_err(|error| StoreError::new("publish rebuild", error.to_string()))
        }
        Ok(metadata) if metadata.file_type().is_file() && !metadata.file_type().is_symlink() => {
            Err(StoreError::new(
                "publish rebuild",
                "the staged rollback journal is not empty after report publication",
            ))
        }
        Ok(_) => Err(StoreError::new(
            "publish rebuild",
            "the staged rollback journal is not a regular file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(StoreError::new("publish rebuild", error.to_string())),
    }
}

#[cfg(not(windows))]
fn replace_store_artifact(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn replace_store_artifact(from: &Path, to: &Path) -> io::Result<()> {
    crate::windows_private_acl::replace_existing(from, to)
}

fn journal_path(path: &Path) -> PathBuf {
    let mut value = path.as_os_str().to_os_string();
    value.push("-journal");
    PathBuf::from(value)
}

fn enforce_private_journal(path: &Path) -> Result<(), StoreError> {
    let journal = journal_path(path);
    if journal.exists() {
        enforce_private_file(&journal)?;
    }
    Ok(())
}

#[cfg(unix)]
fn enforce_private_directory(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| StoreError::new("protect directory", error.to_string()))
}

#[cfg(windows)]
fn enforce_private_directory(path: &Path) -> Result<(), StoreError> {
    crate::windows_private_acl::protect(path)
        .map_err(|error| StoreError::new("protect directory", error.to_string()))
}

#[cfg(not(any(unix, windows)))]
fn enforce_private_directory(_path: &Path) -> Result<(), StoreError> {
    Err(StoreError::new(
        "protect directory",
        "the incremental store is unsupported on this platform",
    ))
}

#[cfg(unix)]
fn enforce_private_file(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| StoreError::new("protect", error.to_string()))
}

#[cfg(windows)]
fn enforce_private_file(path: &Path) -> Result<(), StoreError> {
    crate::windows_private_acl::protect(path)
        .map_err(|error| StoreError::new("protect", error.to_string()))
}

#[cfg(not(any(unix, windows)))]
fn enforce_private_file(_path: &Path) -> Result<(), StoreError> {
    Err(StoreError::new(
        "protect",
        "the incremental store is unsupported on this platform",
    ))
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::prepare_private_directory;
    use super::{
        acquire_store_lock, decode_with_budget, decompress_bytes_with_shared_budget, encode,
        encode_uncompressed, load_file_rows, prepare_store_path,
        validate_analysis_component_lengths, validate_stored_blob_length, DecodedByteBudget,
        StoredDiagnostics, StoredWarning, MAXIMUM_COMPRESSED_PAYLOAD_BYTES, MAXIMUM_PAYLOAD_BYTES,
        MAXIMUM_STORED_ALIAS_CHARACTERS, MAXIMUM_STORED_SOURCE_FILES,
    };
    use rusqlite::{params, Connection};

    #[test]
    fn analysis_payloads_share_one_decoded_size_budget() {
        let first_value = vec![1u8; 64];
        let second_value = vec![2u8; 64];
        let first = encode(&first_value, "first fixture").expect("encode first fixture");
        let second = encode(&second_value, "second fixture").expect("encode second fixture");
        let first_size = encode_uncompressed(&first_value, "first fixture")
            .expect("measure first fixture")
            .len() as u64;
        let second_size = encode_uncompressed(&second_value, "second fixture")
            .expect("measure second fixture")
            .len() as u64;
        let mut remaining = first_size + second_size - 1;

        let decoded: Vec<u8> =
            decode_with_budget(&first, "first fixture", &mut remaining).expect("decode first");
        assert_eq!(decoded, first_value);
        let error = decode_with_budget::<Vec<u8>>(&second, "second fixture", &mut remaining)
            .expect_err("aggregate overflow must fail");
        assert!(error.to_string().contains("aggregate decoded-size budget"));
    }

    #[test]
    fn concurrent_cached_rows_share_one_atomic_decoded_size_budget() {
        let decoded_row = vec![7u8; 512];
        let compressed_row =
            zstd::stream::encode_all(decoded_row.as_slice(), 1).expect("compress row fixture");
        let budget = std::sync::Arc::new(DecodedByteBudget::with_limit(768));

        let results = std::thread::scope(|scope| {
            let first_budget = std::sync::Arc::clone(&budget);
            let first_row = &compressed_row;
            let first = scope.spawn(move || {
                decompress_bytes_with_shared_budget(first_row, "first cached row", &first_budget)
            });
            let second_budget = std::sync::Arc::clone(&budget);
            let second_row = &compressed_row;
            let second = scope.spawn(move || {
                decompress_bytes_with_shared_budget(second_row, "second cached row", &second_budget)
            });
            [
                first.join().expect("first decoder did not panic"),
                second.join().expect("second decoder did not panic"),
            ]
        });

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert!(results
            .iter()
            .find_map(|result| result.as_ref().err())
            .expect("one row must exceed the aggregate budget")
            .to_string()
            .contains("aggregate decoded-size budget"));
    }

    #[test]
    fn analysis_encoder_rejects_aggregate_component_bytes_without_allocating_them() {
        assert!(validate_analysis_component_lengths([
            usize::try_from(MAXIMUM_PAYLOAD_BYTES).unwrap(),
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ])
        .is_ok());
        let error = validate_analysis_component_lengths([
            usize::try_from(MAXIMUM_PAYLOAD_BYTES).unwrap(),
            1,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ])
        .expect_err("aggregate encoded bytes above the budget must fail");
        assert!(error.to_string().contains("aggregate encoded-size budget"));
    }

    #[test]
    fn per_store_lock_serializes_preparation_and_releases_after_drop() {
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-store-lock-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&root);
        prepare_store_path(&root.join("state.sqlite3")).expect("prepare private store parent");
        let path = root.join("state.sqlite3");
        let first = acquire_store_lock(&path).expect("acquire first store lock");
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (result_tx, result_rx) = std::sync::mpsc::channel();
        let second_path = path.clone();
        let second = std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let acquired = acquire_store_lock(&second_path);
            result_tx.send(acquired.is_ok()).unwrap();
        });
        started_rx.recv().unwrap();
        assert!(
            result_rx
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "the second invocation acquired a live store lock"
        );
        drop(first);
        assert!(result_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("second invocation did not resume after lock release"));
        second.join().unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn windows_store_rejects_an_attacker_writable_ancestor() {
        use std::time::{SystemTime, UNIX_EPOCH};

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-store-ancestor-{}-{nonce}",
            std::process::id()
        ));
        crate::windows_private_acl::create_private_directory_new(&root)
            .expect("create protected test root");
        let attacker_writable = root.join("attacker-writable");
        crate::windows_private_acl::create_private_directory_new(&attacker_writable)
            .expect("create test ancestor");
        crate::windows_private_acl::grant_untrusted_delete_child_for_test(&attacker_writable)
            .expect("grant untrusted delete-child permission");

        let error = prepare_private_directory(&attacker_writable.join("nested").join("store"))
            .expect_err("an attacker-writable ancestor must fail closed");
        assert!(error.to_string().contains("store ancestor"));
        assert!(!attacker_writable.join("nested").exists());

        std::fs::remove_dir_all(root).expect("remove ACL regression tree");
    }

    #[test]
    fn cached_diagnostics_reject_terminal_control_sequences() {
        let diagnostics = StoredDiagnostics {
            source_root_count: 0,
            files_discovered: 0,
            accepted_records: 0,
            canonical_records: 0,
            malformed_records: 0,
            unsupported_records: 0,
            unknown_records: 0,
            unknown_fields: 0,
            filtered_records: 0,
            redacted_fields: 0,
            duplicate_records: 0,
            skipped_records: 0,
            resolved_overlap_records: 0,
            unresolved_overlap_records: 0,
            authority_excluded_records: 0,
            earliest: None,
            latest: None,
            sources: Default::default(),
            warnings: vec![StoredWarning {
                code: "W_CACHE_FIXTURE".to_string(),
                message: "unsafe\u{1b}]52;c;payload\u{7}".to_string(),
                source_alias: None,
            }],
            unknown_shapes: Vec::new(),
            capabilities: Default::default(),
            saw_source_cost: false,
            analytical_cost_coverage: None,
            excluded_analysis_token_categories: 0,
            excluded_analysis_cost: false,
            analytical_claims_uncertain: false,
        };

        let error = diagnostics
            .into_runtime()
            .expect_err("terminal control sequences must not cross the cache boundary");
        assert!(error.to_string().contains("cached warning"));
    }

    #[test]
    fn stored_blob_and_inventory_bounds_reject_oversized_values() {
        assert!(validate_stored_blob_length(
            "verify fixture",
            "fixture",
            i64::try_from(MAXIMUM_COMPRESSED_PAYLOAD_BYTES).expect("limit fits i64"),
            MAXIMUM_COMPRESSED_PAYLOAD_BYTES,
        )
        .is_ok());
        assert!(validate_stored_blob_length(
            "verify fixture",
            "fixture",
            i64::try_from(MAXIMUM_COMPRESSED_PAYLOAD_BYTES + 1).expect("limit fits i64"),
            MAXIMUM_COMPRESSED_PAYLOAD_BYTES,
        )
        .is_err());
        assert_eq!(MAXIMUM_STORED_SOURCE_FILES, 100_256);
    }

    #[test]
    fn file_row_scan_rejects_unbounded_scalar_metadata() {
        let connection = Connection::open_in_memory().expect("open fixture database");
        connection
            .execute_batch(
                "
                CREATE TABLE source_file (
                    path_key,
                    normalization_key,
                    source_key,
                    source_alias,
                    source_kind,
                    file_alias,
                    event_count,
                    device,
                    inode,
                    size,
                    modified_seconds,
                    modified_nanoseconds,
                    changed_seconds,
                    changed_nanoseconds,
                    content_digest,
                    normalized_events,
                    diagnostics,
                    metric_state
                );
                ",
            )
            .expect("create deliberately unconstrained fixture");
        connection
            .execute(
                "
                INSERT INTO source_file VALUES (
                    ?1, ?2, ?3, ?4, 1, 'transcript-1-file-1', 0,
                    ?5, ?5, 1, 0, 0, 0, 0, ?6, ?7, ?7, NULL
                )
                ",
                params![
                    vec![1u8; 32],
                    vec![2u8; 32],
                    vec![3u8; 32],
                    "x".repeat(
                        usize::try_from(MAXIMUM_STORED_ALIAS_CHARACTERS)
                            .expect("alias limit fits usize")
                            + 1
                    ),
                    vec![4u8; 8],
                    vec![5u8; 32],
                    vec![6u8; 1],
                ],
            )
            .expect("insert oversized alias fixture");

        let error = load_file_rows(&connection).expect_err("oversized alias must fail closed");
        assert!(error.to_string().contains("source alias"));

        connection
            .execute(
                "UPDATE source_file SET source_alias = 'transcript-1', path_key = ?1",
                params![vec![1u8; 33]],
            )
            .expect("replace alias with oversized key");
        let error = load_file_rows(&connection).expect_err("oversized path key must fail closed");
        assert!(error.to_string().contains("path key"));

        connection
            .execute(
                "UPDATE source_file SET path_key = ?1, device = ?2",
                params![vec![1u8; 32], vec![4u8; 9]],
            )
            .expect("replace key with oversized identity");
        let error =
            load_file_rows(&connection).expect_err("oversized device identity must fail closed");
        assert!(error.to_string().contains("device identity"));
    }
}
