use super::discovery::Source;
use super::line_reader::{BoundedLines, DigestingFile};
use super::types::{
    classified_tool_name, safe_model_name, safe_source_cost, AliasRegistry, Diagnostics, EventKind,
    FileSnapshot, NormalizedEvent, PrivacyHasher, PrivatePrompt, TokenFacts,
    MAX_UNKNOWN_SHAPE_DIAGNOSTICS, NORMALIZED_SCHEMA, OTEL_ADAPTER, UNATTRIBUTED_PROJECT_ALIAS,
};
use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt;
use std::fs::{self, File};
use std::io::BufReader;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    mpsc, Arc,
};
use std::thread;
use std::time::Duration;

const MAX_RESOURCE_GROUPS: usize = 256;
const MAX_SCOPES_PER_RESOURCE: usize = 256;
const MAX_RECORDS_PER_OBJECT: usize = 100_000;
const MAX_ATTRIBUTES: usize = 128;
const MAX_ATTRIBUTE_TEXT_BYTES: usize = 64 * 1024;
const MAX_METRIC_STREAMS: usize = 1_000_000;
const CLAUDE_SCOPE: &str = "com.anthropic.claude_code";
const CLAUDE_EVENTS_SCOPE: &str = "com.anthropic.claude_code.events";

#[derive(Debug, Clone)]
pub(super) struct OtelOptions {
    pub time_context: super::TimeContext,
    pub maximum_line_bytes: usize,
    pub maximum_events: usize,
    pub read_accounting: Arc<super::SourceReadAccounting>,
}

#[derive(Debug, Clone)]
pub(super) struct OtelError {
    message: String,
}

impl OtelError {
    fn source(source: &Source, action: &str, error: impl fmt::Display) -> Self {
        Self {
            message: format!(
                "{action} failed for {}: {error}; the source is indeterminate",
                source.alias
            ),
        }
    }
}

impl fmt::Display for OtelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for OtelError {}

#[derive(Debug)]
pub(super) struct OtelBatchError {
    pub source_alias: String,
    message: String,
}

impl OtelBatchError {
    pub(super) fn is_source_work_limit(&self) -> bool {
        self.message.contains(super::SOURCE_WORK_LIMIT_CODE)
    }
}

impl fmt::Display for OtelBatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[derive(Debug)]
struct OtelSourceResult {
    events: Vec<NormalizedEvent>,
    diagnostics: Diagnostics,
    tracker: MetricTracker,
    content_digest: [u8; 32],
    cached_event_payload: Option<Vec<u8>>,
    cached_diagnostics_payload: Option<Vec<u8>>,
    cached_metric_payload: Option<Vec<u8>>,
}

#[derive(Debug)]
enum OtelWorkerMessage {
    Source {
        index: usize,
        result: Box<Result<OtelSourceResult, OtelError>>,
    },
    Panic {
        index: usize,
    },
    Finished,
}

#[derive(Debug, Clone)]
struct ShapeError {
    code: &'static str,
    message: &'static str,
}

impl ShapeError {
    fn new(code: &'static str, message: &'static str) -> Self {
        Self { code, message }
    }
}

#[derive(Debug, Clone)]
enum Scalar {
    String(String),
    Integer(i64),
    Float(f64),
    Boolean(bool),
    Other,
}

#[derive(Debug, Default)]
struct Attributes {
    values: BTreeMap<String, Scalar>,
    redactions: usize,
    unknown_fields: usize,
    identity_material: String,
    token_family_identity_material: String,
}

impl Attributes {
    fn string(&self, key: &str) -> Option<&str> {
        match self.values.get(key) {
            Some(Scalar::String(value)) => Some(value),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct AttributeLayers<'a> {
    layers: [&'a Attributes; 3],
}

impl<'a> AttributeLayers<'a> {
    fn new(resource: &'a Attributes, scope: &'a Attributes, local: &'a Attributes) -> Self {
        Self {
            layers: [local, scope, resource],
        }
    }

    fn scalar(&self, key: &str) -> Option<&Scalar> {
        for layer in self.layers {
            #[cfg(test)]
            ATTRIBUTE_LAYER_PROBES.with(|probes| probes.set(probes.get().saturating_add(1)));
            if let Some(value) = layer.values.get(key) {
                return Some(value);
            }
        }
        None
    }

    fn string(&self, key: &str) -> Option<&str> {
        match self.scalar(key) {
            Some(Scalar::String(value)) => Some(value),
            _ => None,
        }
    }

    fn u64(&self, key: &str) -> Option<u64> {
        match self.scalar(key) {
            Some(Scalar::Integer(value)) => u64::try_from(*value).ok(),
            Some(Scalar::Float(value))
                if value.is_finite()
                    && *value >= 0.0
                    && value.fract() == 0.0
                    && *value <= (1u64 << 53) as f64 =>
            {
                Some(*value as u64)
            }
            _ => None,
        }
    }

    fn f64(&self, key: &str) -> Option<f64> {
        match self.scalar(key) {
            Some(Scalar::Integer(value)) => Some(*value as f64),
            Some(Scalar::Float(value)) if value.is_finite() => Some(*value),
            _ => None,
        }
    }

    fn boolish(&self, key: &str) -> Option<bool> {
        match self.scalar(key) {
            Some(Scalar::Boolean(value)) => Some(*value),
            Some(Scalar::String(value)) if value == "true" => Some(true),
            Some(Scalar::String(value)) if value == "false" => Some(false),
            _ => None,
        }
    }

    fn contains(&self, key: &str) -> bool {
        self.scalar(key).is_some()
    }
}

#[cfg(test)]
thread_local! {
    static ATTRIBUTE_LAYER_PROBES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct MetricState {
    start_nanos: u64,
    end_nanos: u64,
    raw_value: MetricNumber,
    last_delta: MetricNumber,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct MetricTracker {
    streams: HashMap<u64, MetricState>,
    known_streams: HashSet<u64>,
    pending: Vec<PendingMetricPoint>,
    line_journal: Option<MetricLineJournal>,
}

#[derive(Debug, Clone, Copy)]
struct MetricDelta {
    interval_start: u64,
    interval_end: u64,
    value: MetricNumber,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
enum MetricNumber {
    Integer(i64),
    Double(f64),
}

impl MetricNumber {
    fn as_f64(self) -> f64 {
        match self {
            Self::Integer(value) => value as f64,
            Self::Double(value) => value,
        }
    }

    fn total_cmp(self, other: Self) -> std::cmp::Ordering {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => left.cmp(&right),
            (Self::Double(left), Self::Double(right)) => left.total_cmp(&right),
            (Self::Integer(_), Self::Double(_)) => std::cmp::Ordering::Less,
            (Self::Double(_), Self::Integer(_)) => std::cmp::Ordering::Greater,
        }
    }

    fn same_kind(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Integer(_), Self::Integer(_)) | (Self::Double(_), Self::Double(_))
        )
    }

    fn is_less_than(self, other: Self) -> bool {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => left < right,
            (Self::Double(left), Self::Double(right)) => left < right,
            _ => false,
        }
    }

    fn subtract(self, other: Self) -> Option<Self> {
        match (self, other) {
            (Self::Integer(left), Self::Integer(right)) => {
                left.checked_sub(right).map(Self::Integer)
            }
            (Self::Double(left), Self::Double(right)) => {
                let value = left - right;
                value.is_finite().then_some(Self::Double(value))
            }
            _ => None,
        }
    }

    fn zero_of_same_kind(self) -> Self {
        match self {
            Self::Integer(_) => Self::Integer(0),
            Self::Double(_) => Self::Double(0.0),
        }
    }
}

impl From<f64> for MetricNumber {
    fn from(value: f64) -> Self {
        Self::Double(value)
    }
}

impl PartialEq<f64> for MetricNumber {
    fn eq(&self, other: &f64) -> bool {
        self.as_f64() == *other
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct MetricLineJournal {
    pending_len: usize,
    new_streams: Vec<u64>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum TokenCategory {
    Input,
    Output,
    CacheRead,
    CacheCreation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
enum PendingMetricKind {
    Token(TokenCategory),
    Cost,
    EditDecision,
    Other,
}

#[derive(Debug, Clone, Copy)]
struct MetricContract {
    wire_unit: &'static str,
    canonical_name: &'static str,
    canonical_unit: &'static str,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingMetricPoint {
    stream_key: u64,
    token_family_key: u64,
    temporality: u64,
    start_nanos: u64,
    end_nanos: u64,
    raw_value: MetricNumber,
    source_alias: String,
    file_alias: String,
    record_index: u64,
    observation_key: u64,
    project_key: u64,
    session_key: u64,
    session_identity_present: bool,
    agent_key: Option<u64>,
    skill_key: Option<u64>,
    plugin_key: Option<u64>,
    mcp_server_key: Option<u64>,
    mcp_tool_key: Option<u64>,
    is_subagent: bool,
    model: Option<String>,
    pricing_modifier: String,
    metric_kind: PendingMetricKind,
    canonical_metric_name: String,
    canonical_metric_unit: String,
    tool_name: Option<String>,
    edit_decision: Option<String>,
    attribute_evidence_uncertain: bool,
    redacted_fields: usize,
}

#[allow(clippy::too_many_arguments)]
pub(super) fn ingest_sources(
    sources: &[&Source],
    options: &OtelOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    tracker: &mut MetricTracker,
    worker_count: usize,
    worker_delay_seed: Option<u64>,
    store_files: &mut Vec<super::store::SourceFile>,
    file_cache: Option<&super::store::FileCache>,
) -> Result<Vec<NormalizedEvent>, OtelBatchError> {
    if sources.is_empty() {
        return Ok(Vec::new());
    }
    if let Some(cache) = file_cache {
        let mut cached_results = Vec::with_capacity(sources.len());
        for source in sources {
            let cached = cache
                .lookup(
                    &source.path,
                    &source.path,
                    &source.alias,
                    source.kind,
                    &source.discovery_snapshot,
                )
                .map_err(|error| OtelBatchError {
                    source_alias: source.alias.clone(),
                    message: error.to_string(),
                })?;
            let cached = if let Some(mut cached) = cached {
                for event in &mut cached.events {
                    event.source_alias.clone_from(&source.alias);
                    event.file_alias = format!("{}-file-1", source.alias);
                }
                for shape in &mut cached.diagnostics.unknown_shapes {
                    shape.source_alias.clone_from(&source.alias);
                    shape.file_alias = format!("{}-file-1", source.alias);
                }
                let metric_payload = cached.metric_state.ok_or_else(|| OtelBatchError {
                    source_alias: source.alias.clone(),
                    message: "the cached telemetry payload has no metric state".to_string(),
                })?;
                let tracker =
                    super::store::decode_metric_state(&metric_payload, &cached.decode_budget)
                        .map_err(|error| OtelBatchError {
                            source_alias: source.alias.clone(),
                            message: error.to_string(),
                        })?;
                Some(OtelSourceResult {
                    events: cached.events,
                    diagnostics: cached.diagnostics,
                    tracker,
                    content_digest: cached.content_digest,
                    cached_event_payload: Some(cached.event_payload),
                    cached_diagnostics_payload: Some(cached.diagnostics_payload),
                    cached_metric_payload: Some(metric_payload),
                })
            } else {
                None
            };
            cached_results.push(cached);
        }
        if cached_results.iter().all(Option::is_none) {
            let worker_count = worker_count.max(1).min(sources.len());
            let results = if worker_count == 1 {
                sources
                    .iter()
                    .map(|source| {
                        ingest_source_isolated(source, options, hasher).map_err(|error| {
                            OtelBatchError {
                                source_alias: source.alias.clone(),
                                message: error.to_string(),
                            }
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                ingest_sources_parallel(sources, options, hasher, worker_count, worker_delay_seed)?
            };
            return merge_cached_sources(
                sources,
                results,
                options,
                diagnostics,
                aliases,
                tracker,
                store_files,
            );
        }
        let mut results = Vec::with_capacity(sources.len());
        for (source, cached) in sources.iter().zip(cached_results) {
            if let Some(cached) = cached {
                results.push(cached);
            } else {
                results.push(
                    ingest_source_isolated(source, options, hasher).map_err(|error| {
                        OtelBatchError {
                            source_alias: source.alias.clone(),
                            message: error.to_string(),
                        }
                    })?,
                );
            }
        }
        return merge_cached_sources(
            sources,
            results,
            options,
            diagnostics,
            aliases,
            tracker,
            store_files,
        );
    }
    let worker_count = worker_count.max(1).min(sources.len());
    if worker_count == 1 {
        let mut events = Vec::new();
        let mut private_prompts = Vec::new();
        for source in sources {
            let source_options = OtelOptions {
                time_context: options.time_context.clone(),
                maximum_line_bytes: options.maximum_line_bytes,
                maximum_events: options.maximum_events.saturating_sub(events.len()),
                read_accounting: Arc::clone(&options.read_accounting),
            };
            let (mut source_events, content_digest) = ingest(
                source,
                &source_options,
                diagnostics,
                hasher,
                aliases,
                &mut private_prompts,
                tracker,
            )
            .map_err(|error| OtelBatchError {
                source_alias: source.alias.clone(),
                message: error.to_string(),
            })?;
            store_files.push(
                super::store::SourceFile::with_content_digest(
                    source.path.clone(),
                    source.path.clone(),
                    source.alias.clone(),
                    source.kind,
                    source.discovery_snapshot.clone(),
                    content_digest,
                )
                .with_file_alias(format!("{}-file-1", source.alias)),
            );
            events.append(&mut source_events);
        }
        return Ok(events);
    }

    let results =
        ingest_sources_parallel(sources, options, hasher, worker_count, worker_delay_seed)?;
    let mut events = Vec::new();
    for (source, mut result) in sources.iter().zip(results) {
        store_files.push(
            super::store::SourceFile::with_content_digest(
                source.path.clone(),
                source.path.clone(),
                source.alias.clone(),
                source.kind,
                source.discovery_snapshot.clone(),
                result.content_digest,
            )
            .with_file_alias(format!("{}-file-1", source.alias)),
        );
        diagnostics.merge_file_parse(result.diagnostics);
        for event in &mut result.events {
            assign_event_aliases(event, aliases);
        }
        tracker
            .merge(result.tracker)
            .map_err(|message| OtelBatchError {
                source_alias: source.alias.clone(),
                message,
            })?;
        if events
            .len()
            .checked_add(result.events.len())
            .and_then(|count| count.checked_add(tracker.pending.len()))
            .is_none_or(|count| count > options.maximum_events)
        {
            return Err(OtelBatchError {
                source_alias: source.alias.clone(),
                message: format!(
                    "{} exceeded the normalized-event safety limit; narrow the selected period",
                    source.alias
                ),
            });
        }
        events.append(&mut result.events);
    }
    Ok(events)
}

#[allow(clippy::too_many_arguments)]
fn merge_cached_sources(
    sources: &[&Source],
    results: Vec<OtelSourceResult>,
    options: &OtelOptions,
    diagnostics: &mut Diagnostics,
    aliases: &mut AliasRegistry,
    tracker: &mut MetricTracker,
    store_files: &mut Vec<super::store::SourceFile>,
) -> Result<Vec<NormalizedEvent>, OtelBatchError> {
    let mut events = Vec::new();
    for (source, mut result) in sources.iter().zip(results) {
        let metric_payload = match result.cached_metric_payload.take() {
            Some(payload) => payload,
            None => super::store::encode_metric_state(&result.tracker).map_err(|error| {
                OtelBatchError {
                    source_alias: source.alias.clone(),
                    message: error.to_string(),
                }
            })?,
        };
        let source_file = super::store::SourceFile::with_content_digest(
            source.path.clone(),
            source.path.clone(),
            source.alias.clone(),
            source.kind,
            source.discovery_snapshot.clone(),
            result.content_digest,
        )
        .with_file_alias(format!("{}-file-1", source.alias));
        let source_file = match (
            result.cached_event_payload.take(),
            result.cached_diagnostics_payload.take(),
        ) {
            (Some(event_payload), Some(diagnostics_payload)) => source_file.with_encoded_payload(
                result.events.len(),
                event_payload,
                diagnostics_payload,
                Some(metric_payload),
            ),
            (None, None) => source_file
                .with_payload(&result.events, &result.diagnostics, Some(metric_payload))
                .map_err(|error| OtelBatchError {
                    source_alias: source.alias.clone(),
                    message: error.to_string(),
                })?,
            _ => {
                return Err(OtelBatchError {
                    source_alias: source.alias.clone(),
                    message: "the cached telemetry payload was incomplete".to_string(),
                })
            }
        };
        store_files.push(source_file);
        diagnostics.merge_file_parse(result.diagnostics);
        for event in &mut result.events {
            assign_event_aliases(event, aliases);
        }
        tracker
            .merge(result.tracker)
            .map_err(|message| OtelBatchError {
                source_alias: source.alias.clone(),
                message,
            })?;
        if events
            .len()
            .checked_add(result.events.len())
            .and_then(|count| count.checked_add(tracker.pending.len()))
            .is_none_or(|count| count > options.maximum_events)
        {
            return Err(OtelBatchError {
                source_alias: source.alias.clone(),
                message: format!(
                    "{} exceeded the normalized-event safety limit; narrow the selected period",
                    source.alias
                ),
            });
        }
        events.append(&mut result.events);
    }
    Ok(events)
}

fn ingest_sources_parallel(
    sources: &[&Source],
    options: &OtelOptions,
    hasher: &PrivacyHasher,
    worker_count: usize,
    worker_delay_seed: Option<u64>,
) -> Result<Vec<OtelSourceResult>, OtelBatchError> {
    let next_source = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let queue_capacity = worker_count.saturating_mul(2).max(1);
    let (sender, receiver) = mpsc::sync_channel(queue_capacity);

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let next_source = &next_source;
            let cancelled = &cancelled;
            scope.spawn(move || {
                while !cancelled.load(Ordering::Acquire) {
                    let index = next_source.fetch_add(1, Ordering::AcqRel);
                    let Some(source) = sources.get(index).copied() else {
                        break;
                    };
                    apply_worker_delay(worker_delay_seed, index);
                    let result = catch_unwind(AssertUnwindSafe(|| {
                        ingest_source_isolated(source, options, hasher)
                    }));
                    let message = match result {
                        Ok(result) => OtelWorkerMessage::Source {
                            index,
                            result: Box::new(result),
                        },
                        Err(_) => {
                            cancelled.store(true, Ordering::Release);
                            OtelWorkerMessage::Panic { index }
                        }
                    };
                    if sender.send(message).is_err() {
                        cancelled.store(true, Ordering::Release);
                        break;
                    }
                }
                let _ = sender.send(OtelWorkerMessage::Finished);
            });
        }
        drop(sender);

        let mut results = std::iter::repeat_with(|| None)
            .take(sources.len())
            .collect::<Vec<Option<OtelSourceResult>>>();
        let mut finished = 0usize;
        let mut first_error = None;
        while finished < worker_count {
            let message = receiver.recv().map_err(|_| OtelBatchError {
                source_alias: "otel".to_string(),
                message: "telemetry worker channel closed before completion".to_string(),
            })?;
            match message {
                OtelWorkerMessage::Source { index, result } => match *result {
                    Ok(result) => {
                        if results.get(index).is_none_or(Option::is_some) {
                            cancelled.store(true, Ordering::Release);
                            first_error.get_or_insert_with(|| OtelBatchError {
                                source_alias: "otel".to_string(),
                                message: "telemetry worker returned an invalid source index"
                                    .to_string(),
                            });
                        } else {
                            results[index] = Some(result);
                        }
                    }
                    Err(error) => {
                        cancelled.store(true, Ordering::Release);
                        let source_alias = sources
                            .get(index)
                            .map_or_else(|| "otel".to_string(), |source| source.alias.clone());
                        first_error.get_or_insert_with(|| OtelBatchError {
                            source_alias,
                            message: error.to_string(),
                        });
                    }
                },
                OtelWorkerMessage::Panic { index } => {
                    let source_alias = sources
                        .get(index)
                        .map_or_else(|| "otel".to_string(), |source| source.alias.clone());
                    first_error.get_or_insert_with(|| OtelBatchError {
                        source_alias,
                        message: "a telemetry worker panicked; no partial result was published"
                            .to_string(),
                    });
                }
                OtelWorkerMessage::Finished => finished = finished.saturating_add(1),
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        results
            .into_iter()
            .enumerate()
            .map(|(index, result)| {
                result.ok_or_else(|| OtelBatchError {
                    source_alias: sources[index].alias.clone(),
                    message: format!(
                        "{} telemetry worker omitted its source result",
                        sources[index].alias
                    ),
                })
            })
            .collect()
    })
}

fn ingest_source_isolated(
    source: &Source,
    options: &OtelOptions,
    hasher: &PrivacyHasher,
) -> Result<OtelSourceResult, OtelError> {
    let mut diagnostics = Diagnostics::default();
    let mut source_stats = super::types::SourceStats::otel(source.alias.clone());
    source_stats.files_discovered = 0;
    diagnostics
        .sources
        .insert(source.alias.clone(), source_stats);
    let mut aliases = AliasRegistry::default();
    let mut private_prompts = Vec::new();
    let mut tracker = MetricTracker::default();
    let (events, content_digest) = ingest(
        source,
        options,
        &mut diagnostics,
        hasher,
        &mut aliases,
        &mut private_prompts,
        &mut tracker,
    )?;
    debug_assert!(private_prompts.is_empty());
    Ok(OtelSourceResult {
        events,
        diagnostics,
        tracker,
        content_digest,
        cached_event_payload: None,
        cached_diagnostics_payload: None,
        cached_metric_payload: None,
    })
}

fn assign_event_aliases(event: &mut NormalizedEvent, aliases: &mut AliasRegistry) {
    event.project_alias = if event.project_identity_present {
        aliases.project(event.project_key)
    } else {
        UNATTRIBUTED_PROJECT_ALIAS.to_string()
    };
    event.session_alias = aliases.session(event.session_key);
    event.parent_session_alias = event.parent_key.map(|key| aliases.session(key));
}

fn apply_worker_delay(seed: Option<u64>, source_index: usize) {
    let Some(seed) = seed else {
        return;
    };
    let mut mixed = seed ^ (source_index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    mixed ^= mixed >> 30;
    mixed = mixed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed ^= mixed >> 27;
    thread::sleep(Duration::from_micros(mixed % 2_000));
}

pub(super) fn ingest(
    source: &Source,
    options: &OtelOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    _private_prompts: &mut [PrivatePrompt],
    tracker: &mut MetricTracker,
) -> Result<(Vec<NormalizedEvent>, [u8; 32]), OtelError> {
    diagnostics.files_discovered = diagnostics.files_discovered.saturating_add(1);
    let resolved = fs::canonicalize(&source.path)
        .map_err(|error| OtelError::source(source, "telemetry canonicalization", error))?;
    if resolved != source.path {
        return Err(OtelError {
            message: format!(
                "{} changed identity after discovery; rerun against a stable snapshot",
                source.alias
            ),
        });
    }
    let file = File::open(&resolved)
        .map_err(|error| OtelError::source(source, "telemetry file open", error))?;
    let before = file
        .metadata()
        .map_err(|error| OtelError::source(source, "telemetry metadata read", error))?;
    if !before.is_file() {
        return Err(OtelError {
            message: format!("{} is no longer a regular telemetry file", source.alias),
        });
    }
    let before_snapshot = FileSnapshot::capture_file(&before, &file)
        .map_err(|error| OtelError::source(source, "opened telemetry identity read", error))?;
    if source.discovery_snapshot != before_snapshot {
        return Err(OtelError {
            message: format!(
                "{} changed identity between discovery and open; rerun against a stable snapshot",
                source.alias
            ),
        });
    }
    let mut lines = BoundedLines::with_accounting(
        BufReader::new(
            DigestingFile::new(
                file,
                hasher.store_salt(),
                Arc::clone(&options.read_accounting),
            )
            .map_err(|error| OtelError::source(source, "telemetry stream budget", error))?,
        ),
        options.maximum_line_bytes,
        Arc::clone(&options.read_accounting),
    );
    let file_alias = format!("{}-file-1", source.alias);
    let mut line_index = 0u64;
    let mut events = Vec::new();

    while let Some(line) = lines
        .next_line()
        .map_err(|error| OtelError::source(source, "telemetry stream read", error))?
    {
        line_index = line_index.saturating_add(1);
        if line.oversized {
            record_malformed(
                diagnostics,
                source,
                "W_OTEL_LINE_OVERSIZED",
                "An oversized telemetry line was drained without buffering and excluded.",
            );
            continue;
        }
        if line.bytes.iter().all(u8::is_ascii_whitespace) {
            record_skipped(diagnostics, source);
            continue;
        }
        let object = match serde_json::from_slice::<Value>(&line.bytes) {
            Ok(Value::Object(object)) => object,
            Ok(_) => {
                record_malformed(
                    diagnostics,
                    source,
                    "W_OTEL_NON_OBJECT",
                    "A telemetry line was valid JSON but not a pinned export object.",
                );
                continue;
            }
            Err(_) => {
                record_malformed(
                    diagnostics,
                    source,
                    "W_OTEL_MALFORMED_JSON",
                    "Malformed telemetry JSON was excluded; later lines were still scanned.",
                );
                continue;
            }
        };
        let observation_base = hasher.hash(&(
            source.alias.as_str(),
            file_alias.as_str(),
            line.byte_offset,
            &line.bytes,
        ));
        let diagnostics_checkpoint = diagnostics.checkpoint_otel_line(&source.alias);
        let alias_checkpoint = aliases.checkpoint();
        let is_metric_object =
            object.contains_key("resourceMetrics") && !object.contains_key("resourceLogs");
        if is_metric_object {
            tracker.begin_line();
        }
        let parsed = if object.contains_key("resourceLogs")
            && !object.contains_key("resourceMetrics")
        {
            count_unknown_object_fields(diagnostics, source, &object, &["resourceLogs"]);
            parse_logs(
                source,
                &file_alias,
                line_index,
                observation_base,
                &object,
                options,
                diagnostics,
                hasher,
                aliases,
            )
        } else if object.contains_key("resourceMetrics") && !object.contains_key("resourceLogs") {
            count_unknown_object_fields(diagnostics, source, &object, &["resourceMetrics"]);
            parse_metrics(
                source,
                &file_alias,
                line_index,
                observation_base,
                &object,
                diagnostics,
                hasher,
                tracker,
            )
        } else {
            record_unknown_shape(
                diagnostics,
                source,
                &file_alias,
                line_index,
                "unsupported-export-root",
                &object,
                line.bytes.len(),
            );
            Err(ShapeError::new(
                "W_OTEL_ROOT_SHAPE_UNSUPPORTED",
                "A telemetry object must contain exactly one pinned resourceLogs or resourceMetrics root.",
            ))
        };
        match parsed {
            Ok(mut line_events) => {
                if events
                    .len()
                    .checked_add(tracker.pending.len())
                    .and_then(|count| count.checked_add(line_events.len()))
                    .is_none_or(|count| count > options.maximum_events)
                {
                    if is_metric_object {
                        tracker.rollback_line();
                    }
                    return Err(OtelError {
                        message: format!(
                            "{} exceeded the normalized-event safety limit; narrow the selected period",
                            source.alias
                        ),
                    });
                }
                if is_metric_object {
                    tracker.commit_line();
                }
                events.append(&mut line_events);
            }
            Err(error) => {
                if is_metric_object {
                    tracker.rollback_line();
                }
                diagnostics.rollback_otel_line(&source.alias, diagnostics_checkpoint);
                aliases.rollback(alias_checkpoint);
                record_unknown_shape(
                    diagnostics,
                    source,
                    &file_alias,
                    line_index,
                    "unsupported-otel-object",
                    &object,
                    line.bytes.len(),
                );
                record_unsupported(diagnostics, source, error.code, error.message);
            }
        }
    }

    let digesting_file = lines.into_inner().into_inner();
    let (file, content_digest, _) = digesting_file.finish();
    let after = file
        .metadata()
        .map_err(|error| OtelError::source(source, "final telemetry metadata read", error))?;
    let path_after = fs::metadata(&resolved)
        .map_err(|error| OtelError::source(source, "final telemetry path metadata read", error))?;
    let after_snapshot = FileSnapshot::capture_file(&after, &file)
        .map_err(|error| OtelError::source(source, "final telemetry identity read", error))?;
    let path_matches = before_snapshot
        .matches_path(&path_after, &resolved)
        .map_err(|error| OtelError::source(source, "final telemetry path identity read", error))?;
    if before_snapshot != after_snapshot || !path_matches {
        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
            stats.partial = true;
        }
        return Err(OtelError {
            message: format!(
                "{} changed while it was being streamed; rerun against a stable snapshot",
                source.alias
            ),
        });
    }
    Ok((events, content_digest.unwrap_or([0; 32])))
}

#[allow(clippy::too_many_arguments)]
fn parse_logs(
    source: &Source,
    file_alias: &str,
    line_index: u64,
    observation_base: u64,
    object: &Map<String, Value>,
    options: &OtelOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
) -> Result<Vec<NormalizedEvent>, ShapeError> {
    let resources = required_array(object.get("resourceLogs"), "resourceLogs")?;
    require_limit(
        resources.len(),
        MAX_RESOURCE_GROUPS,
        "W_OTEL_RESOURCE_LIMIT",
        "A telemetry export exceeded the resource-group safety limit.",
    )?;
    let mut events = Vec::new();
    let mut record_count = 0usize;
    for (resource_index, resource_value) in resources.iter().enumerate() {
        let resource = required_object(Some(resource_value), "resourceLogs item")?;
        count_unknown_object_fields(
            diagnostics,
            source,
            resource,
            &["resource", "scopeLogs", "schemaUrl"],
        );
        let resource_attributes = parse_entity_attributes(resource.get("resource"))?;
        record_attribute_diagnostics(diagnostics, source, &resource_attributes);
        let resource_attribute_evidence_uncertain = record_declared_attribute_drops(
            resource.get("resource").and_then(Value::as_object),
            diagnostics,
            source,
        )?;
        let scopes = required_array(resource.get("scopeLogs"), "scopeLogs")?;
        require_limit(
            scopes.len(),
            MAX_SCOPES_PER_RESOURCE,
            "W_OTEL_SCOPE_LIMIT",
            "A telemetry resource exceeded the scope safety limit.",
        )?;
        if resource_attributes
            .string("service.name")
            .is_some_and(|name| name != "claude-code")
        {
            let count = nested_record_count(resource.get("scopeLogs"), "logRecords")?;
            record_count = record_count.saturating_add(count);
            require_limit(
                record_count,
                MAX_RECORDS_PER_OBJECT,
                "W_OTEL_RECORD_LIMIT",
                "A telemetry export exceeded the log-record safety limit.",
            )?;
            record_known_irrelevant_many(
                diagnostics,
                source,
                count,
                "W_OTEL_NON_CLAUDE_RESOURCE",
                "A log resource not identified as claude-code was excluded.",
            );
            continue;
        }
        for (scope_index, scope_value) in scopes.iter().enumerate() {
            let scope_logs = required_object(Some(scope_value), "scopeLogs item")?;
            count_unknown_object_fields(
                diagnostics,
                source,
                scope_logs,
                &["scope", "logRecords", "schemaUrl"],
            );
            let scope = required_object(scope_logs.get("scope"), "scope")?;
            count_unknown_object_fields(
                diagnostics,
                source,
                scope,
                &["name", "version", "attributes", "droppedAttributesCount"],
            );
            let scope_name = scope.get("name").and_then(Value::as_str).ok_or_else(|| {
                ShapeError::new(
                    "W_OTEL_SCOPE_NAME_MISSING",
                    "A telemetry scope had no string name.",
                )
            })?;
            let scope_attributes = parse_attributes(scope.get("attributes"))?;
            record_attribute_diagnostics(diagnostics, source, &scope_attributes);
            let scope_attribute_evidence_uncertain =
                record_declared_attribute_drops(Some(scope), diagnostics, source)?;
            let records = required_array(scope_logs.get("logRecords"), "logRecords")?;
            let record_base = record_count;
            record_count = record_count.saturating_add(records.len());
            require_limit(
                record_count,
                MAX_RECORDS_PER_OBJECT,
                "W_OTEL_RECORD_LIMIT",
                "A telemetry export exceeded the log-record safety limit.",
            )?;
            if !matches!(scope_name, CLAUDE_SCOPE | CLAUDE_EVENTS_SCOPE) {
                record_known_irrelevant_many(
                    diagnostics,
                    source,
                    records.len(),
                    "W_OTEL_SCOPE_UNSUPPORTED",
                    "A non-Claude instrumentation scope was excluded.",
                );
                continue;
            }
            for (record_index, record_value) in records.iter().enumerate() {
                let record = required_object(Some(record_value), "log record")?;
                if let Some(event) = normalize_log_record(
                    source,
                    file_alias,
                    line_index,
                    resource_index,
                    scope_index,
                    record_index,
                    record_base.saturating_add(record_index),
                    observation_base,
                    record,
                    &resource_attributes,
                    &scope_attributes,
                    resource_attribute_evidence_uncertain || scope_attribute_evidence_uncertain,
                    options,
                    diagnostics,
                    hasher,
                    aliases,
                )? {
                    events.push(event);
                }
            }
        }
    }
    Ok(events)
}

#[allow(clippy::too_many_arguments)]
fn normalize_log_record(
    source: &Source,
    file_alias: &str,
    line_index: u64,
    resource_index: usize,
    scope_index: usize,
    record_index: usize,
    logical_record_index: usize,
    observation_base: u64,
    record: &Map<String, Value>,
    resource_attributes: &Attributes,
    scope_attributes: &Attributes,
    inherited_attribute_evidence_uncertain: bool,
    options: &OtelOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
) -> Result<Option<NormalizedEvent>, ShapeError> {
    count_unknown_object_fields(
        diagnostics,
        source,
        record,
        &[
            "timeUnixNano",
            "observedTimeUnixNano",
            "severityNumber",
            "severityText",
            "body",
            "attributes",
            "droppedAttributesCount",
            "flags",
            "traceId",
            "spanId",
            "eventName",
        ],
    );
    let record_attributes = parse_attributes(record.get("attributes"))?;
    record_attribute_diagnostics(diagnostics, source, &record_attributes);
    let attribute_evidence_uncertain = inherited_attribute_evidence_uncertain
        || record_declared_attribute_drops(Some(record), diagnostics, source)?;
    let attributes =
        AttributeLayers::new(resource_attributes, scope_attributes, &record_attributes);
    let body_redaction = usize::from(
        record
            .get("body")
            .is_some_and(|body| body.as_object().is_none_or(|object| !object.is_empty())),
    );
    let identity_redactions = ["traceId", "spanId"]
        .into_iter()
        .filter(|field| record.contains_key(*field))
        .count();
    record_transformed_redactions(
        diagnostics,
        source,
        body_redaction.saturating_add(identity_redactions),
    );
    let event_name = record
        .get("eventName")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_EVENT_NAME_MISSING",
                "A pinned log record had no eventName field.",
            )
        })?;
    let kind = match event_name {
        "claude_code.api_request" => EventKind::OtelApiRequest,
        "claude_code.api_error" => EventKind::OtelApiError,
        "claude_code.tool_result" => EventKind::OtelToolResult,
        "claude_code.tool_decision" => EventKind::OtelToolDecision,
        "claude_code.user_prompt" => EventKind::UserPrompt,
        "claude_code.compaction" => EventKind::Compaction,
        "claude_code.api_request_body"
        | "claude_code.api_response_body"
        | "claude_code.assistant_response" => {
            record_known_irrelevant(
                diagnostics,
                source,
                "W_OTEL_CONTENT_EVENT_EXCLUDED",
                "A content-bearing telemetry event was excluded by the standard privacy profile.",
            );
            return Ok(None);
        }
        _ => {
            record_unknown_shape(
                diagnostics,
                source,
                file_alias,
                line_index
                    .saturating_mul(MAX_RECORDS_PER_OBJECT as u64)
                    .saturating_add(logical_record_index as u64),
                event_name,
                record,
                serde_json::to_vec(record).map_or(0, |bytes| bytes.len()),
            );
            record_unsupported(
                diagnostics,
                source,
                "W_OTEL_EVENT_UNSUPPORTED",
                "An unsupported telemetry event was counted and excluded without retaining its name or values.",
            );
            return Ok(None);
        }
    };

    let (timestamp, epoch_nanos) = event_timestamp(record, &attributes)?;
    let parsed = ccwrapped::parse_timestamp(&timestamp).ok_or_else(|| {
        ShapeError::new(
            "W_OTEL_TIMESTAMP_INVALID",
            "A telemetry event had an invalid timestamp.",
        )
    })?;
    if !options.time_context.contains_fixed(parsed) {
        diagnostics.filtered_records = diagnostics.filtered_records.saturating_add(1);
        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
            stats.filtered_records = stats.filtered_records.saturating_add(1);
        }
        return Ok(None);
    }

    let agent_key = attributes
        .string("agent_id")
        .or_else(|| attributes.string("agent.name"))
        .map(|value| hasher.hash(&("agent", value)));
    let is_subagent = agent_key.is_some() || attributes.string("query_source") == Some("subagent");
    let session_raw = attributes.string("session.id");
    let session_key = session_raw.map_or_else(
        || {
            hasher.hash(&(
                "missing-session",
                source.alias.as_str(),
                file_alias,
                line_index,
                resource_index,
                scope_index,
                record_index,
            ))
        },
        |session| hasher.hash(&("session", session, is_subagent)),
    );
    let project_key = hasher.hash(&("otel-project", "unattributed"));
    let session_alias = aliases.session(session_key);
    let project_alias = UNATTRIBUTED_PROJECT_ALIAS.to_string();
    let request_key = attributes
        .string("request_id")
        .map(|value| hasher.hash(&("request", value)));
    let prompt_key = attributes
        .string("prompt.id")
        .map(|value| hasher.hash(&("prompt", value)));
    let tool_key = attributes
        .string("tool_use_id")
        .map(|value| hasher.hash(&("tool", value)));
    let parent_agent_key = attributes
        .string("parent_agent_id")
        .map(|value| hasher.hash(&("agent", value)));
    let skill_key = attributes
        .string("skill.name")
        .map(|value| hasher.hash(&("skill", value)));
    let plugin_key = attributes
        .string("plugin.name")
        .map(|value| hasher.hash(&("plugin", value)));
    let mcp_server_key = attributes
        .string("mcp_server.name")
        .map(|value| hasher.hash(&("mcp-server", value)));
    let mcp_tool_key = attributes
        .string("mcp_tool.name")
        .map(|value| hasher.hash(&("mcp-tool", value)));
    let model = attributes.string("model").and_then(safe_model_name);
    let (pricing_modifier, pricing_modifier_redactions) = pricing_modifier(&attributes);
    let (tool_name, tool_name_redactions) = attributes
        .string("tool_name")
        .map_or((None, 0), classified_tool_name);
    let model_redactions = usize::from(attributes.string("model").is_some() && model.is_none());
    let success = attributes.boolish("success");
    let latency_ms = attributes.f64("duration_ms").filter(|value| {
        value.is_finite() && *value >= 0.0 && *value <= super::types::MAX_DIRECT_DURATION_MS
    });
    let attempt = attributes.u64("attempt");
    let edit_decision = attributes
        .string("decision")
        .filter(|value| matches!(*value, "accept" | "reject"))
        .map(str::to_string);
    let compaction = if kind == EventKind::Compaction {
        success
    } else {
        None
    };
    let source_cost_estimate = attributes.f64("cost_usd").and_then(safe_source_cost);
    let token_value_redactions = if kind == EventKind::OtelApiRequest {
        [
            "input_tokens",
            "output_tokens",
            "cache_creation_tokens",
            "cache_read_tokens",
        ]
        .into_iter()
        .filter(|key| attributes.contains(key) && attributes.u64(key).is_none())
        .count()
    } else {
        0
    };
    let analytical_value_redactions = token_value_redactions
        .saturating_add(usize::from(
            attributes.contains("cost_usd") && source_cost_estimate.is_none(),
        ))
        .saturating_add(usize::from(
            attributes.contains("duration_ms") && latency_ms.is_none(),
        ))
        .saturating_add(usize::from(
            attributes.contains("success") && success.is_none(),
        ))
        .saturating_add(pricing_modifier_redactions);
    let transformed_redactions = tool_name_redactions
        .saturating_add(model_redactions)
        .saturating_add(analytical_value_redactions);
    record_transformed_redactions(diagnostics, source, transformed_redactions);
    if analytical_value_redactions > 0 {
        mark_partial_warning(
            diagnostics,
            source,
            "W_OTEL_ANALYTICAL_ATTRIBUTE_INVALID",
            "One or more supported analytical attributes had invalid values and were excluded.",
        );
    }
    if source_cost_estimate.is_some() {
        diagnostics.saw_source_cost = true;
    }
    diagnostics.observe_time(epoch_nanos, &timestamp);
    let logical_index = line_index
        .saturating_mul(MAX_RECORDS_PER_OBJECT as u64)
        .saturating_add(logical_record_index as u64);
    let observation_key = hasher.hash(&(
        observation_base,
        resource_index,
        scope_index,
        record_index,
        event_name,
    ));
    let message_key = request_key.or(tool_key).or(prompt_key);
    let tokens = if kind == EventKind::OtelApiRequest {
        TokenFacts {
            input: attributes.u64("input_tokens"),
            output: attributes.u64("output_tokens"),
            cache_creation: attributes.u64("cache_creation_tokens"),
            cache_read: attributes.u64("cache_read_tokens"),
            cache_creation_5m: None,
            cache_creation_1h: None,
        }
    } else {
        TokenFacts::default()
    };

    Ok(Some(NormalizedEvent {
        schema_version: NORMALIZED_SCHEMA,
        adapter_version: OTEL_ADAPTER,
        source_alias: source.alias.clone(),
        file_alias: file_alias.to_string(),
        record_index: logical_index,
        timestamp,
        epoch_nanos,
        timestamp_conversion_status: "normalized-utc",
        project_key,
        project_identity_present: false,
        session_key,
        session_identity_present: session_raw.is_some(),
        message_key,
        request_key,
        parent_key: None,
        agent_key,
        parent_agent_key,
        skill_key,
        plugin_key,
        mcp_server_key,
        mcp_tool_key,
        observation_key,
        project_alias,
        session_alias,
        parent_session_alias: None,
        is_subagent,
        is_sidechain: false,
        kind,
        model_mapping_status: if model.is_some() {
            "unmapped"
        } else {
            "missing"
        },
        model,
        pricing_modifier,
        tokens,
        source_cost_estimate,
        tool_names: tool_name.into_iter().collect(),
        tool_status: success.map(|value| {
            if value {
                "success".to_string()
            } else {
                "error".to_string()
            }
        }),
        latency_ms,
        error_count: match kind {
            EventKind::OtelApiError => Some(1),
            EventKind::OtelToolResult if success == Some(false) => Some(1),
            _ => None,
        },
        retry_count: attempt.map(|value| value.saturating_sub(1)),
        edit_decision,
        compaction,
        metric_name: None,
        metric_value: None,
        metric_unit: None,
        metric_interval_start_nanos: None,
        metric_interval_end_nanos: None,
        metric_temporality: None,
        metric_family_key: None,
        attribute_evidence_uncertain,
        redacted_fields: record_attributes
            .redactions
            .saturating_add(body_redaction)
            .saturating_add(identity_redactions)
            .saturating_add(transformed_redactions),
    }))
}

#[allow(clippy::too_many_arguments)]
fn parse_metrics(
    source: &Source,
    file_alias: &str,
    line_index: u64,
    observation_base: u64,
    object: &Map<String, Value>,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    tracker: &mut MetricTracker,
) -> Result<Vec<NormalizedEvent>, ShapeError> {
    let resources = required_array(object.get("resourceMetrics"), "resourceMetrics")?;
    require_limit(
        resources.len(),
        MAX_RESOURCE_GROUPS,
        "W_OTEL_RESOURCE_LIMIT",
        "A telemetry export exceeded the resource-group safety limit.",
    )?;
    let mut events = Vec::new();
    let mut point_count = 0usize;
    for (resource_index, resource_value) in resources.iter().enumerate() {
        let resource = required_object(Some(resource_value), "resourceMetrics item")?;
        count_unknown_object_fields(
            diagnostics,
            source,
            resource,
            &["resource", "scopeMetrics", "schemaUrl"],
        );
        let resource_attributes = parse_entity_attributes(resource.get("resource"))?;
        record_attribute_diagnostics(diagnostics, source, &resource_attributes);
        let resource_attribute_evidence_uncertain = record_declared_attribute_drops(
            resource.get("resource").and_then(Value::as_object),
            diagnostics,
            source,
        )?;
        let scopes = required_array(resource.get("scopeMetrics"), "scopeMetrics")?;
        require_limit(
            scopes.len(),
            MAX_SCOPES_PER_RESOURCE,
            "W_OTEL_SCOPE_LIMIT",
            "A telemetry resource exceeded the scope safety limit.",
        )?;
        if resource_attributes
            .string("service.name")
            .is_some_and(|name| name != "claude-code")
        {
            let count = nested_metric_point_count(resource.get("scopeMetrics"))?;
            point_count = point_count.saturating_add(count);
            require_limit(
                point_count,
                MAX_RECORDS_PER_OBJECT,
                "W_OTEL_POINT_LIMIT",
                "A telemetry export exceeded the metric-point safety limit.",
            )?;
            record_known_irrelevant_many(
                diagnostics,
                source,
                count,
                "W_OTEL_NON_CLAUDE_RESOURCE",
                "A metric resource not identified as claude-code was excluded.",
            );
            continue;
        }
        for (scope_index, scope_value) in scopes.iter().enumerate() {
            let scope_metrics = required_object(Some(scope_value), "scopeMetrics item")?;
            count_unknown_object_fields(
                diagnostics,
                source,
                scope_metrics,
                &["scope", "metrics", "schemaUrl"],
            );
            let scope = required_object(scope_metrics.get("scope"), "scope")?;
            count_unknown_object_fields(
                diagnostics,
                source,
                scope,
                &["name", "version", "attributes", "droppedAttributesCount"],
            );
            let scope_name = scope.get("name").and_then(Value::as_str).ok_or_else(|| {
                ShapeError::new(
                    "W_OTEL_SCOPE_NAME_MISSING",
                    "A telemetry scope had no string name.",
                )
            })?;
            let scope_attributes = parse_attributes(scope.get("attributes"))?;
            record_attribute_diagnostics(diagnostics, source, &scope_attributes);
            let scope_attribute_evidence_uncertain =
                record_declared_attribute_drops(Some(scope), diagnostics, source)?;
            let metrics = required_array(scope_metrics.get("metrics"), "metrics")?;
            if scope_name != CLAUDE_SCOPE {
                record_known_irrelevant_many(
                    diagnostics,
                    source,
                    metrics.len(),
                    "W_OTEL_SCOPE_UNSUPPORTED",
                    "A non-Claude instrumentation scope was excluded.",
                );
                continue;
            }
            for (metric_index, metric_value) in metrics.iter().enumerate() {
                let metric = required_object(Some(metric_value), "metric")?;
                count_unknown_object_fields(
                    diagnostics,
                    source,
                    metric,
                    &["name", "description", "unit", "sum", "metadata"],
                );
                let name = metric.get("name").and_then(Value::as_str).ok_or_else(|| {
                    ShapeError::new(
                        "W_OTEL_METRIC_NAME_MISSING",
                        "A telemetry metric had no string name.",
                    )
                })?;
                let sum = match metric.get("sum").and_then(Value::as_object) {
                    Some(sum) => sum,
                    None => {
                        let count = metric_point_count(metric);
                        record_unsupported_many(
                            diagnostics,
                            source,
                            count.max(1),
                            "W_OTEL_METRIC_KIND_UNSUPPORTED",
                            "Only pinned sum metrics are accepted in telemetry adapter v1.",
                        );
                        continue;
                    }
                };
                count_unknown_object_fields(
                    diagnostics,
                    source,
                    sum,
                    &["dataPoints", "aggregationTemporality", "isMonotonic"],
                );
                if sum.get("isMonotonic").and_then(Value::as_bool) != Some(true) {
                    record_unsupported_many(
                        diagnostics,
                        source,
                        points_or_one(sum),
                        "W_OTEL_NON_MONOTONIC_SUM",
                        "A supported Claude counter was not a monotonic sum and was excluded.",
                    );
                    continue;
                }
                let points = required_array(sum.get("dataPoints"), "sum.dataPoints")?;
                let point_base = point_count;
                point_count = point_count.saturating_add(points.len());
                require_limit(
                    point_count,
                    MAX_RECORDS_PER_OBJECT,
                    "W_OTEL_POINT_LIMIT",
                    "A telemetry export exceeded the metric-point safety limit.",
                )?;
                let contract = match metric_contract(name) {
                    Some(contract) => contract,
                    None => {
                        record_unknown_shape(
                            diagnostics,
                            source,
                            file_alias,
                            line_index
                                .saturating_mul(MAX_RECORDS_PER_OBJECT as u64)
                                .saturating_add(metric_index as u64),
                            "unsupported-metric",
                            metric,
                            serde_json::to_vec(metric).map_or(0, |bytes| bytes.len()),
                        );
                        record_unsupported_many(
                            diagnostics,
                            source,
                            points.len().max(1),
                            "W_OTEL_METRIC_UNSUPPORTED",
                            "An unsupported Claude metric was counted and excluded.",
                        );
                        continue;
                    }
                };
                let wire_unit = metric.get("unit").and_then(Value::as_str).ok_or_else(|| {
                    ShapeError::new(
                        "W_OTEL_METRIC_UNIT_UNSUPPORTED",
                        "A supported metric had no pinned string unit.",
                    )
                })?;
                if wire_unit != contract.wire_unit {
                    return Err(ShapeError::new(
                        "W_OTEL_METRIC_UNIT_UNSUPPORTED",
                        "A supported metric declared a unit outside its pinned contract.",
                    ));
                }
                let temporality = sum
                    .get("aggregationTemporality")
                    .and_then(json_u64)
                    .ok_or_else(|| {
                        ShapeError::new(
                            "W_OTEL_TEMPORALITY_MISSING",
                            "A supported sum metric had no pinned aggregation temporality.",
                        )
                    })?;
                if !matches!(temporality, 1 | 2) {
                    return Err(ShapeError::new(
                        "W_OTEL_TEMPORALITY_UNSUPPORTED",
                        "A supported sum metric used an unsupported aggregation temporality.",
                    ));
                }
                for (point_index, point_value) in points.iter().enumerate() {
                    let point = required_object(Some(point_value), "number data point")?;
                    if let Some(event) = normalize_metric_point(
                        source,
                        file_alias,
                        line_index,
                        resource_index,
                        scope_index,
                        metric_index,
                        point_index,
                        point_base.saturating_add(point_index),
                        observation_base,
                        name,
                        contract,
                        temporality,
                        point,
                        &resource_attributes,
                        &scope_attributes,
                        resource_attribute_evidence_uncertain || scope_attribute_evidence_uncertain,
                        diagnostics,
                        hasher,
                        tracker,
                    )? {
                        events.push(event);
                    }
                }
            }
        }
    }
    Ok(events)
}

#[allow(clippy::too_many_arguments)]
fn normalize_metric_point(
    source: &Source,
    file_alias: &str,
    line_index: u64,
    resource_index: usize,
    scope_index: usize,
    metric_index: usize,
    point_index: usize,
    logical_point_index: usize,
    observation_base: u64,
    metric_name: &str,
    metric_contract: MetricContract,
    temporality: u64,
    point: &Map<String, Value>,
    resource_attributes: &Attributes,
    scope_attributes: &Attributes,
    inherited_attribute_evidence_uncertain: bool,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    tracker: &mut MetricTracker,
) -> Result<Option<NormalizedEvent>, ShapeError> {
    count_unknown_object_fields(
        diagnostics,
        source,
        point,
        &[
            "attributes",
            "startTimeUnixNano",
            "timeUnixNano",
            "asInt",
            "asDouble",
            "exemplars",
            "flags",
            "droppedAttributesCount",
        ],
    );
    let point_attributes = parse_attributes(point.get("attributes"))?;
    record_attribute_diagnostics(diagnostics, source, &point_attributes);
    let attribute_evidence_uncertain = inherited_attribute_evidence_uncertain
        || record_declared_attribute_drops(Some(point), diagnostics, source)?;
    let attributes = AttributeLayers::new(resource_attributes, scope_attributes, &point_attributes);
    let start_nanos = point
        .get("startTimeUnixNano")
        .and_then(json_u64)
        .ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_METRIC_START_MISSING",
                "A supported metric point had no valid start timestamp.",
            )
        })?;
    let end_nanos = point
        .get("timeUnixNano")
        .and_then(json_u64)
        .ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_METRIC_END_MISSING",
                "A supported metric point had no valid end timestamp.",
            )
        })?;
    if start_nanos == 0 || end_nanos <= start_nanos {
        return Err(ShapeError::new(
            "W_OTEL_METRIC_INTERVAL_INVALID",
            "A supported metric point had an invalid accumulation interval.",
        ));
    }
    let raw_value = point_number(point)?;
    let stream_key = hasher.hash(&(
        resource_attributes.identity_material.as_str(),
        scope_attributes.identity_material.as_str(),
        metric_name,
        metric_contract.wire_unit,
        temporality,
        point_attributes.identity_material.as_str(),
    ));
    let token_family_key = hasher.hash(&(
        resource_attributes.token_family_identity_material.as_str(),
        scope_attributes.token_family_identity_material.as_str(),
        metric_name,
        metric_contract.wire_unit,
        temporality,
        point_attributes.token_family_identity_material.as_str(),
    ));
    if metric_stream_limit_reached(tracker, stream_key, MAX_METRIC_STREAMS) {
        return Err(ShapeError::new(
            "W_OTEL_STREAM_CARDINALITY_LIMIT",
            "The telemetry invocation exceeded the metric-stream cardinality limit.",
        ));
    }
    nanos_datetime(start_nanos)?;
    nanos_datetime(end_nanos)?;
    let agent_key = attributes
        .string("agent.name")
        .map(|value| hasher.hash(&("agent", value)));
    let is_subagent = agent_key.is_some() || attributes.string("query_source") == Some("subagent");
    let session_raw = attributes.string("session.id");
    let session_key = session_raw.map_or_else(
        || hasher.hash(&("missing-metric-session", stream_key)),
        |session| hasher.hash(&("session", session, is_subagent)),
    );
    let project_key = hasher.hash(&("otel-project", "unattributed"));
    let skill_key = attributes
        .string("skill.name")
        .map(|value| hasher.hash(&("skill", value)));
    let plugin_key = attributes
        .string("plugin.name")
        .map(|value| hasher.hash(&("plugin", value)));
    let mcp_server_key = attributes
        .string("mcp_server.name")
        .map(|value| hasher.hash(&("mcp-server", value)));
    let mcp_tool_key = attributes
        .string("mcp_tool.name")
        .map(|value| hasher.hash(&("mcp-tool", value)));
    let model = attributes.string("model").and_then(safe_model_name);
    let (pricing_modifier, pricing_modifier_redactions) = pricing_modifier(&attributes);
    let model_redactions = usize::from(attributes.string("model").is_some() && model.is_none());
    let metric_kind = if metric_name == "claude_code.token.usage" {
        exact_u64(raw_value).ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_TOKEN_VALUE_FRACTIONAL",
                "A token metric point was not an exact non-negative integer.",
            )
        })?;
        match attributes.string("type") {
            Some("input") => PendingMetricKind::Token(TokenCategory::Input),
            Some("output") => PendingMetricKind::Token(TokenCategory::Output),
            Some("cacheRead") => PendingMetricKind::Token(TokenCategory::CacheRead),
            Some("cacheCreation") => PendingMetricKind::Token(TokenCategory::CacheCreation),
            _ => {
                record_unsupported(
                    diagnostics,
                    source,
                    "W_OTEL_TOKEN_TYPE_UNSUPPORTED",
                    "A token metric point had an unsupported type and was excluded.",
                );
                return Ok(None);
            }
        }
    } else if metric_name == "claude_code.cost.usage" {
        safe_source_cost(raw_value.as_f64()).ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_COST_VALUE_RANGE",
                "A cost metric value was outside the bounded supported estimate range.",
            )
        })?;
        PendingMetricKind::Cost
    } else if metric_name == "claude_code.code_edit_tool.decision" {
        PendingMetricKind::EditDecision
    } else {
        PendingMetricKind::Other
    };
    let logical_index = line_index
        .saturating_mul(MAX_RECORDS_PER_OBJECT as u64)
        .saturating_add(logical_point_index as u64);
    let observation_key = hasher.hash(&(
        observation_base,
        resource_index,
        scope_index,
        metric_index,
        point_index,
    ));
    let edit_decision = if metric_name == "claude_code.code_edit_tool.decision" {
        attributes
            .string("decision")
            .filter(|value| matches!(*value, "accept" | "reject"))
            .map(str::to_string)
    } else {
        None
    };
    let (tool_name, tool_name_redactions) = attributes
        .string("tool_name")
        .map_or((None, 0), classified_tool_name);
    record_transformed_redactions(
        diagnostics,
        source,
        tool_name_redactions
            .saturating_add(model_redactions)
            .saturating_add(pricing_modifier_redactions),
    );
    if pricing_modifier_redactions > 0 {
        mark_partial_warning(
            diagnostics,
            source,
            "W_OTEL_ANALYTICAL_ATTRIBUTE_INVALID",
            "One or more supported analytical attributes had invalid values and were excluded.",
        );
    }
    tracker.queue(PendingMetricPoint {
        stream_key,
        token_family_key,
        temporality,
        start_nanos,
        end_nanos,
        raw_value,
        source_alias: source.alias.clone(),
        file_alias: file_alias.to_string(),
        record_index: logical_index,
        project_key,
        session_key,
        session_identity_present: session_raw.is_some(),
        agent_key,
        skill_key,
        plugin_key,
        mcp_server_key,
        mcp_tool_key,
        is_subagent,
        model,
        pricing_modifier,
        metric_kind,
        canonical_metric_name: metric_contract.canonical_name.to_string(),
        canonical_metric_unit: metric_contract.canonical_unit.to_string(),
        tool_name,
        edit_decision,
        attribute_evidence_uncertain,
        redacted_fields: point_attributes
            .redactions
            .saturating_add(tool_name_redactions)
            .saturating_add(model_redactions)
            .saturating_add(pricing_modifier_redactions),
        observation_key,
    });
    Ok(None)
}

fn metric_stream_limit_reached(tracker: &MetricTracker, stream_key: u64, maximum: usize) -> bool {
    !tracker.known_streams.contains(&stream_key) && tracker.known_streams.len() >= maximum
}

impl MetricTracker {
    fn merge(&mut self, mut other: Self) -> Result<(), String> {
        debug_assert!(self.line_journal.is_none());
        debug_assert!(other.line_journal.is_none());
        for stream_key in other.known_streams.drain() {
            self.known_streams.insert(stream_key);
            if self.known_streams.len() > MAX_METRIC_STREAMS {
                return Err(
                    "the telemetry invocation exceeded the metric-stream cardinality limit"
                        .to_string(),
                );
            }
        }
        self.pending.append(&mut other.pending);
        Ok(())
    }

    fn begin_line(&mut self) {
        debug_assert!(self.line_journal.is_none());
        self.line_journal = Some(MetricLineJournal {
            pending_len: self.pending.len(),
            new_streams: Vec::new(),
        });
    }

    fn commit_line(&mut self) {
        self.line_journal = None;
    }

    fn rollback_line(&mut self) {
        let Some(journal) = self.line_journal.take() else {
            return;
        };
        self.pending.truncate(journal.pending_len);
        for stream_key in journal.new_streams {
            self.known_streams.remove(&stream_key);
        }
    }

    fn queue(&mut self, point: PendingMetricPoint) {
        if self.known_streams.insert(point.stream_key) {
            if let Some(journal) = &mut self.line_journal {
                journal.new_streams.push(point.stream_key);
            }
        }
        self.pending.push(point);
    }

    #[allow(clippy::too_many_arguments)]
    fn apply<T: Into<MetricNumber>>(
        &mut self,
        stream_key: u64,
        temporality: u64,
        start_nanos: u64,
        end_nanos: u64,
        raw_value: T,
        diagnostics: &mut Diagnostics,
        source_alias: &str,
    ) -> Option<MetricDelta> {
        let raw_value = raw_value.into();
        self.known_streams.insert(stream_key);
        let previous = self.streams.get(&stream_key).cloned();
        if temporality == 2
            && previous
                .as_ref()
                .is_some_and(|previous| !raw_value.same_kind(previous.raw_value))
        {
            mark_partial_warning_alias(
                diagnostics,
                source_alias,
                "W_OTEL_METRIC_NUMBER_KIND_CHANGED",
                "A cumulative metric stream changed between integer and double point values and was excluded.",
            );
            return None;
        }
        let delta = match (temporality, previous.as_ref()) {
            (1, Some(previous))
                if start_nanos == previous.start_nanos
                    && end_nanos == previous.end_nanos
                    && raw_value == previous.raw_value =>
            {
                MetricDelta {
                    interval_start: start_nanos,
                    interval_end: end_nanos,
                    value: previous.last_delta,
                }
            }
            (1, Some(previous)) if start_nanos < previous.end_nanos => {
                mark_partial_warning_alias(
                    diagnostics,
                    source_alias,
                    "W_OTEL_METRIC_OVERLAP",
                    "Overlapping delta metric windows were excluded unless they were exact duplicates.",
                );
                return None;
            }
            (1, Some(previous)) => {
                if start_nanos > previous.end_nanos {
                    mark_partial_warning_alias(
                        diagnostics,
                        source_alias,
                        "W_OTEL_METRIC_GAP",
                        "A gap in a delta metric stream makes its coverage partial.",
                    );
                }
                MetricDelta {
                    interval_start: start_nanos,
                    interval_end: end_nanos,
                    value: raw_value,
                }
            }
            (1, None) => MetricDelta {
                interval_start: start_nanos,
                interval_end: end_nanos,
                value: raw_value,
            },
            (2, Some(previous))
                if start_nanos == previous.start_nanos
                    && end_nanos == previous.end_nanos
                    && raw_value == previous.raw_value =>
            {
                MetricDelta {
                    interval_start: start_nanos,
                    interval_end: end_nanos,
                    value: previous.last_delta,
                }
            }
            (2, Some(previous)) if end_nanos <= previous.end_nanos => {
                mark_partial_warning_alias(
                    diagnostics,
                    source_alias,
                    "W_OTEL_METRIC_OVERLAP",
                    "An overlapping or out-of-order cumulative metric point was excluded.",
                );
                return None;
            }
            (2, Some(previous))
                if start_nanos != previous.start_nanos && start_nanos < previous.end_nanos =>
            {
                mark_partial_warning_alias(
                    diagnostics,
                    source_alias,
                    "W_OTEL_METRIC_OVERLAP",
                    "A changed cumulative writer interval overlapped the prior sequence and was excluded.",
                );
                return None;
            }
            (2, Some(previous)) if start_nanos == previous.start_nanos => {
                if raw_value.is_less_than(previous.raw_value) {
                    mark_partial_warning_alias(
                        diagnostics,
                        source_alias,
                        "W_OTEL_METRIC_RESET_AMBIGUOUS",
                        "A decreased cumulative value with an unchanged start time was excluded as an ambiguous reset.",
                    );
                    self.streams.insert(
                        stream_key,
                        MetricState {
                            start_nanos,
                            end_nanos,
                            raw_value,
                            last_delta: raw_value.zero_of_same_kind(),
                        },
                    );
                    return None;
                }
                MetricDelta {
                    interval_start: previous.end_nanos,
                    interval_end: end_nanos,
                    value: raw_value
                        .subtract(previous.raw_value)
                        .expect("cumulative metric number kinds were checked"),
                }
            }
            (2, Some(previous)) => {
                if start_nanos > previous.end_nanos {
                    mark_partial_warning_alias(
                        diagnostics,
                        source_alias,
                        "W_OTEL_METRIC_GAP",
                        "A gap before a reset cumulative metric sequence makes its coverage partial.",
                    );
                }
                warn_once(
                    diagnostics,
                    "W_OTEL_METRIC_RESET",
                    "A changed cumulative start time was treated as a new writer sequence.",
                    Some(source_alias.to_string()),
                );
                MetricDelta {
                    interval_start: start_nanos,
                    interval_end: end_nanos,
                    value: raw_value,
                }
            }
            (2, None) => MetricDelta {
                interval_start: start_nanos,
                interval_end: end_nanos,
                value: raw_value,
            },
            _ => return None,
        };
        self.streams.insert(
            stream_key,
            MetricState {
                start_nanos,
                end_nanos,
                raw_value,
                last_delta: delta.value,
            },
        );
        Some(delta)
    }
}

pub(super) fn finalize_metrics(
    tracker: &mut MetricTracker,
    time_context: &super::TimeContext,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
) -> Vec<NormalizedEvent> {
    let mut pending = std::mem::take(&mut tracker.pending);
    pending.sort_by(|left, right| {
        left.start_nanos
            .cmp(&right.start_nanos)
            .then_with(|| left.end_nanos.cmp(&right.end_nanos))
            .then_with(|| left.raw_value.total_cmp(right.raw_value))
            .then_with(|| left.source_alias.cmp(&right.source_alias))
            .then_with(|| left.file_alias.cmp(&right.file_alias))
            .then_with(|| left.record_index.cmp(&right.record_index))
            .then_with(|| left.observation_key.cmp(&right.observation_key))
    });
    tracker.streams.clear();
    let mut events = Vec::with_capacity(pending.len());
    for point in pending {
        let metric_name = known_metric_name(&point.canonical_metric_name);
        let metric_unit = known_metric_unit(&point.canonical_metric_unit);
        let Some(delta) = tracker.apply(
            point.stream_key,
            point.temporality,
            point.start_nanos,
            point.end_nanos,
            point.raw_value,
            diagnostics,
            &point.source_alias,
        ) else {
            record_filtered_metric_point(
                diagnostics,
                &point.source_alias,
                point.metric_kind,
                metric_interval_may_affect_period(point.start_nanos, point.end_nanos, time_context),
            );
            continue;
        };
        let start_epoch = i128::from(delta.interval_start);
        let end_epoch = i128::from(delta.interval_end);
        let crosses_selected_period = !time_context.contains_epoch(start_epoch)
            || !time_context.contains_epoch(end_epoch.saturating_sub(1));
        if crosses_selected_period || !time_context.same_local_day(start_epoch, end_epoch) {
            record_filtered_metric_point(
                diagnostics,
                &point.source_alias,
                point.metric_kind,
                metric_interval_may_affect_period(point.start_nanos, point.end_nanos, time_context),
            );
            mark_partial_warning_alias(
                diagnostics,
                &point.source_alias,
                "W_OTEL_PERIOD_BOUNDARY_STRADDLE",
                "A metric accumulation window crossed a selected local day or reporting period and was not assigned or prorated.",
            );
            continue;
        }
        let end = nanos_datetime(delta.interval_end)
            .expect("queued metric ends were range-validated before normalization");
        let mut tokens = TokenFacts::default();
        let mut source_cost_estimate = None;
        match point.metric_kind {
            PendingMetricKind::Token(category) => {
                let Some(value) = exact_u64(delta.value) else {
                    note_excluded_analytical_metric(diagnostics, point.metric_kind);
                    record_unsupported_alias(
                        diagnostics,
                        &point.source_alias,
                        "W_OTEL_TOKEN_VALUE_FRACTIONAL",
                        "A normalized token metric delta was not an exact non-negative integer.",
                    );
                    continue;
                };
                match category {
                    TokenCategory::Input => tokens.input = Some(value),
                    TokenCategory::Output => tokens.output = Some(value),
                    TokenCategory::CacheRead => tokens.cache_read = Some(value),
                    TokenCategory::CacheCreation => tokens.cache_creation = Some(value),
                }
            }
            PendingMetricKind::Cost => {
                let Some(value) = safe_source_cost(delta.value.as_f64()) else {
                    note_excluded_analytical_metric(diagnostics, point.metric_kind);
                    record_unsupported_alias(
                        diagnostics,
                        &point.source_alias,
                        "W_OTEL_COST_VALUE_RANGE",
                        "A normalized cost metric delta was outside the supported estimate range.",
                    );
                    continue;
                };
                source_cost_estimate = Some(value);
                diagnostics.saw_source_cost = true;
            }
            PendingMetricKind::EditDecision | PendingMetricKind::Other => {}
        }
        let timestamp = end.to_rfc3339();
        let epoch_nanos = datetime_order_key(&end);
        diagnostics.observe_time(epoch_nanos, &timestamp);
        let session_alias = aliases.session(point.session_key);
        let project_alias = UNATTRIBUTED_PROJECT_ALIAS.to_string();
        let metric_identity = hasher.hash(&(
            "otel-metric",
            point.stream_key,
            delta.interval_start,
            delta.interval_end,
        ));
        events.push(NormalizedEvent {
            schema_version: NORMALIZED_SCHEMA,
            adapter_version: OTEL_ADAPTER,
            source_alias: point.source_alias,
            file_alias: point.file_alias,
            record_index: point.record_index,
            timestamp,
            epoch_nanos,
            timestamp_conversion_status: "normalized-utc",
            project_key: point.project_key,
            project_identity_present: false,
            session_key: point.session_key,
            session_identity_present: point.session_identity_present,
            message_key: Some(metric_identity),
            request_key: None,
            parent_key: None,
            agent_key: point.agent_key,
            parent_agent_key: None,
            skill_key: point.skill_key,
            plugin_key: point.plugin_key,
            mcp_server_key: point.mcp_server_key,
            mcp_tool_key: point.mcp_tool_key,
            observation_key: point.observation_key,
            project_alias,
            session_alias,
            parent_session_alias: None,
            is_subagent: point.is_subagent,
            is_sidechain: false,
            kind: EventKind::OtelMetric,
            model_mapping_status: if point.model.is_some() {
                "unmapped"
            } else {
                "missing"
            },
            model: point.model,
            pricing_modifier: point.pricing_modifier,
            tokens,
            source_cost_estimate,
            tool_names: point.tool_name.into_iter().collect(),
            tool_status: None,
            latency_ms: None,
            error_count: None,
            retry_count: None,
            edit_decision: point.edit_decision,
            compaction: None,
            metric_name: Some(metric_name),
            metric_value: Some(delta.value.as_f64()),
            metric_unit: Some(metric_unit),
            metric_interval_start_nanos: Some(delta.interval_start),
            metric_interval_end_nanos: Some(delta.interval_end),
            metric_temporality: Some(point.temporality),
            metric_family_key: Some(point.token_family_key),
            attribute_evidence_uncertain: point.attribute_evidence_uncertain,
            redacted_fields: point.redacted_fields,
        });
    }
    tracker.streams.clear();
    tracker.known_streams.clear();
    events
}

fn record_filtered_metric_point(
    diagnostics: &mut Diagnostics,
    source_alias: &str,
    metric_kind: PendingMetricKind,
    affects_selected_period: bool,
) {
    diagnostics.filtered_records = diagnostics.filtered_records.saturating_add(1);
    if let Some(stats) = diagnostics.sources.get_mut(source_alias) {
        stats.filtered_records = stats.filtered_records.saturating_add(1);
        stats.partial = true;
    }
    if affects_selected_period {
        note_excluded_analytical_metric(diagnostics, metric_kind);
    }
}

fn metric_interval_may_affect_period(
    start_nanos: u64,
    end_nanos: u64,
    time_context: &super::TimeContext,
) -> bool {
    match time_context.period_bounds() {
        Some((period_start, period_end)) => {
            i128::from(end_nanos) > period_start && i128::from(start_nanos) < period_end
        }
        None => true,
    }
}

fn note_excluded_analytical_metric(diagnostics: &mut Diagnostics, metric_kind: PendingMetricKind) {
    match metric_kind {
        PendingMetricKind::Token(TokenCategory::Input) => {
            diagnostics.excluded_analysis_token_categories |= 1 << 0;
        }
        PendingMetricKind::Token(TokenCategory::Output) => {
            diagnostics.excluded_analysis_token_categories |= 1 << 1;
        }
        PendingMetricKind::Token(TokenCategory::CacheCreation) => {
            diagnostics.excluded_analysis_token_categories |= 1 << 2;
        }
        PendingMetricKind::Token(TokenCategory::CacheRead) => {
            diagnostics.excluded_analysis_token_categories |= 1 << 3;
        }
        PendingMetricKind::Cost => diagnostics.excluded_analysis_cost = true,
        PendingMetricKind::EditDecision | PendingMetricKind::Other => {}
    }
}

fn parse_entity_attributes(entity: Option<&Value>) -> Result<Attributes, ShapeError> {
    match entity {
        None => Ok(Attributes::default()),
        Some(value) => {
            let object = required_object(Some(value), "resource")?;
            parse_attributes(object.get("attributes"))
        }
    }
}

fn parse_attributes(value: Option<&Value>) -> Result<Attributes, ShapeError> {
    let Some(value) = value else {
        return Ok(Attributes::default());
    };
    let array = required_array(Some(value), "attributes")?;
    require_limit(
        array.len(),
        MAX_ATTRIBUTES,
        "W_OTEL_ATTRIBUTE_LIMIT",
        "A telemetry entity exceeded the attribute-count safety limit.",
    )?;
    let mut parsed = Attributes::default();
    let mut identity_fields = BTreeMap::new();
    let mut text_bytes = 0usize;
    for item in array {
        let object = required_object(Some(item), "attribute")?;
        let key = object.get("key").and_then(Value::as_str).ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_ATTRIBUTE_KEY_INVALID",
                "A telemetry attribute had no string key.",
            )
        })?;
        text_bytes = text_bytes.saturating_add(key.len());
        let value = object.get("value").ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_ATTRIBUTE_VALUE_MISSING",
                "A telemetry attribute had no value object.",
            )
        })?;
        text_bytes = text_bytes.saturating_add(decoded_text_bytes(value));
        if text_bytes > MAX_ATTRIBUTE_TEXT_BYTES {
            return Err(ShapeError::new(
                "W_OTEL_ATTRIBUTE_TEXT_LIMIT",
                "A telemetry entity exceeded the decoded attribute-text safety limit.",
            ));
        }
        if parsed.values.contains_key(key) {
            return Err(ShapeError::new(
                "W_OTEL_ATTRIBUTE_DUPLICATE",
                "A telemetry entity contained duplicate attribute keys.",
            ));
        }
        let (scalar, unknown_fields) = parse_any_value(value)?;
        parsed.unknown_fields = parsed.unknown_fields.saturating_add(unknown_fields);
        if !analytical_attribute(key) {
            parsed.redactions = parsed.redactions.saturating_add(1);
        }
        let identity_value = serde_json::to_string(value).map_err(|_| {
            ShapeError::new(
                "W_OTEL_ATTRIBUTE_IDENTITY_INVALID",
                "A telemetry attribute could not be represented by the pinned identity contract.",
            )
        })?;
        identity_fields.insert(key.to_string(), identity_value);
        parsed.values.insert(key.to_string(), scalar);
    }
    parsed.identity_material = serde_json::to_string(&identity_fields).map_err(|_| {
        ShapeError::new(
            "W_OTEL_ATTRIBUTE_IDENTITY_INVALID",
            "Telemetry attributes could not be represented by the pinned identity contract.",
        )
    })?;
    identity_fields.remove("type");
    parsed.token_family_identity_material =
        serde_json::to_string(&identity_fields).map_err(|_| {
            ShapeError::new(
                "W_OTEL_ATTRIBUTE_IDENTITY_INVALID",
                "Telemetry attributes could not be represented by the pinned token-family identity contract.",
            )
        })?;
    Ok(parsed)
}

fn parse_any_value(value: &Value) -> Result<(Scalar, usize), ShapeError> {
    let object = required_object(Some(value), "AnyValue")?;
    if object.len() != 1 {
        return Err(ShapeError::new(
            "W_OTEL_ANY_VALUE_SHAPE",
            "A telemetry AnyValue did not contain exactly one pinned value field.",
        ));
    }
    let Some((key, value)) = object.iter().next() else {
        return Ok((Scalar::Other, 0));
    };
    let scalar = match key.as_str() {
        "stringValue" => value
            .as_str()
            .map(|value| Scalar::String(value.to_string()))
            .ok_or_else(|| {
                ShapeError::new(
                    "W_OTEL_ANY_VALUE_TYPE",
                    "A telemetry stringValue did not contain a string.",
                )
            })?,
        "boolValue" => value.as_bool().map(Scalar::Boolean).ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_ANY_VALUE_TYPE",
                "A telemetry boolValue did not contain a boolean.",
            )
        })?,
        "intValue" => Scalar::Integer(parse_pinned_i64(value)?),
        "doubleValue" => value
            .as_f64()
            .filter(|value| value.is_finite())
            .map(Scalar::Float)
            .ok_or_else(|| {
                ShapeError::new(
                    "W_OTEL_ANY_VALUE_TYPE",
                    "A telemetry doubleValue did not contain a finite number.",
                )
            })?,
        "arrayValue" | "kvlistValue" | "bytesValue" => Scalar::Other,
        _ => return Ok((Scalar::Other, 1)),
    };
    Ok((scalar, 0))
}

fn analytical_attribute(key: &str) -> bool {
    matches!(
        key,
        "service.name"
            | "event.name"
            | "event.timestamp"
            | "event.sequence"
            | "model"
            | "cost_usd"
            | "duration_ms"
            | "input_tokens"
            | "output_tokens"
            | "cache_read_tokens"
            | "cache_creation_tokens"
            | "speed"
            | "query_source"
            | "effort"
            | "type"
            | "tool_name"
            | "success"
            | "status_code"
            | "attempt"
            | "decision"
            | "source"
            | "trigger"
            | "pre_tokens"
            | "post_tokens"
    )
}

fn pricing_modifier(attributes: &AttributeLayers<'_>) -> (String, usize) {
    match attributes.string("speed") {
        Some("fast") => ("fast".to_string(), 0),
        Some("normal") => ("standard".to_string(), 0),
        Some(_) => ("unknown".to_string(), 1),
        None if attributes.contains("speed") => ("unknown".to_string(), 1),
        None => ("standard".to_string(), 0),
    }
}

fn event_timestamp(
    record: &Map<String, Value>,
    attributes: &AttributeLayers<'_>,
) -> Result<(String, i128), ShapeError> {
    if let Some(timestamp) = attributes.string("event.timestamp") {
        let parsed = ccwrapped::parse_timestamp(timestamp).ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_TIMESTAMP_INVALID",
                "A telemetry event timestamp attribute was invalid.",
            )
        })?;
        return Ok((
            parsed
                .with_timezone(&Utc)
                .to_rfc3339_opts(SecondsFormat::AutoSi, true),
            datetime_order_key(&parsed),
        ));
    }
    let nanos = record
        .get("timeUnixNano")
        .and_then(json_u64)
        .ok_or_else(|| {
            ShapeError::new(
                "W_OTEL_TIMESTAMP_MISSING",
                "A telemetry log record had no usable event or record timestamp.",
            )
        })?;
    let timestamp = nanos_datetime(nanos)?.to_rfc3339();
    let epoch = datetime_order_key(&nanos_datetime(nanos)?);
    Ok((timestamp, epoch))
}

fn datetime_order_key<Tz: chrono::TimeZone>(timestamp: &DateTime<Tz>) -> i128 {
    (timestamp.timestamp() as i128)
        .saturating_mul(1_000_000_000)
        .saturating_add(timestamp.timestamp_subsec_nanos() as i128)
}

fn nanos_datetime(nanos: u64) -> Result<DateTime<Utc>, ShapeError> {
    let seconds = nanos / 1_000_000_000;
    let subsecond = (nanos % 1_000_000_000) as u32;
    let seconds = i64::try_from(seconds).map_err(|_| {
        ShapeError::new(
            "W_OTEL_TIMESTAMP_RANGE",
            "A telemetry timestamp was outside the supported range.",
        )
    })?;
    DateTime::<Utc>::from_timestamp(seconds, subsecond).ok_or_else(|| {
        ShapeError::new(
            "W_OTEL_TIMESTAMP_RANGE",
            "A telemetry timestamp was outside the supported range.",
        )
    })
}

fn json_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn parse_pinned_i64(value: &Value) -> Result<i64, ShapeError> {
    let raw = value.as_str().ok_or_else(|| {
        ShapeError::new(
            "W_OTEL_INTEGER_STRING_INVALID",
            "A pinned OTLP int64 value was not encoded as a decimal JSON string.",
        )
    })?;
    let digits = raw.strip_prefix('-').unwrap_or(raw);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ShapeError::new(
            "W_OTEL_INTEGER_STRING_INVALID",
            "A pinned OTLP int64 value contained an invalid decimal string.",
        ));
    }
    raw.parse::<i64>().map_err(|error| {
        if matches!(
            error.kind(),
            std::num::IntErrorKind::PosOverflow | std::num::IntErrorKind::NegOverflow
        ) {
            ShapeError::new(
                "W_OTEL_INTEGER_RANGE",
                "A pinned OTLP int64 value was outside the signed 64-bit range.",
            )
        } else {
            ShapeError::new(
                "W_OTEL_INTEGER_STRING_INVALID",
                "A pinned OTLP int64 value contained an invalid decimal string.",
            )
        }
    })
}

fn point_number(point: &Map<String, Value>) -> Result<MetricNumber, ShapeError> {
    match (point.get("asInt"), point.get("asDouble")) {
        (Some(_), Some(_)) => Err(ShapeError::new(
            "W_OTEL_POINT_VALUE_CONFLICT",
            "A telemetry number point contained both asInt and asDouble values.",
        )),
        (Some(value), None) => {
            let value = parse_pinned_i64(value)?;
            if value < 0 {
                Err(ShapeError::new(
                    "W_OTEL_METRIC_VALUE_INVALID",
                    "A supported metric point contained a negative integer value.",
                ))
            } else {
                Ok(MetricNumber::Integer(value))
            }
        }
        (None, Some(value)) => value
            .as_f64()
            .filter(|value| value.is_finite() && *value >= 0.0)
            .map(MetricNumber::Double)
            .ok_or_else(|| {
                ShapeError::new(
                    "W_OTEL_METRIC_VALUE_INVALID",
                    "A supported metric point had no finite non-negative double value.",
                )
            }),
        (None, None) => Err(ShapeError::new(
            "W_OTEL_METRIC_VALUE_INVALID",
            "A supported metric point had neither an asInt nor asDouble value.",
        )),
    }
}

fn exact_u64(value: MetricNumber) -> Option<u64> {
    match value {
        MetricNumber::Integer(value) => u64::try_from(value).ok(),
        MetricNumber::Double(value)
            if value.is_finite()
                && value >= 0.0
                && value.fract() == 0.0
                && value <= (1u64 << 53) as f64 =>
        {
            Some(value as u64)
        }
        MetricNumber::Double(_) => None,
    }
}

fn record_unknown_shape(
    diagnostics: &mut Diagnostics,
    source: &Source,
    file_alias: &str,
    record_index: u64,
    record_kind: &str,
    object: &Map<String, Value>,
    byte_count: usize,
) {
    const ALLOWED_KEYS: &[&str] = &[
        "resourceLogs",
        "resourceMetrics",
        "eventName",
        "timeUnixNano",
        "observedTimeUnixNano",
        "body",
        "attributes",
        "name",
        "sum",
    ];
    diagnostics.unknown_records = diagnostics.unknown_records.saturating_add(1);
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.unknown_records = stats.unknown_records.saturating_add(1);
    }
    if diagnostics.unknown_shapes.len() >= MAX_UNKNOWN_SHAPE_DIAGNOSTICS {
        warn_once(
            diagnostics,
            "W_UNKNOWN_SHAPE_SAMPLES_TRUNCATED",
            "Unknown-shape samples reached the bounded diagnostic limit; aggregate counts remain complete.",
            None,
        );
        return;
    }
    let structural_fields = object
        .iter()
        .filter(|(key, _)| ALLOWED_KEYS.contains(&key.as_str()))
        .take(8)
        .map(|(key, value)| (key.clone(), json_value_kind(value).to_string()))
        .collect();
    diagnostics
        .unknown_shapes
        .push(ccwrapped::UnknownShapeDiagnostic {
            source_alias: source.alias.clone(),
            adapter_version: OTEL_ADAPTER.to_string(),
            file_alias: file_alias.to_string(),
            record_index,
            record_kind: match record_kind {
                "unsupported-export-root" => "unsupported-export-root",
                _ => "unsupported-otel-record",
            }
            .to_string(),
            structural_fields,
            byte_count,
        });
}

fn json_value_kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn metric_contract(name: &str) -> Option<MetricContract> {
    let (wire_unit, canonical_name, canonical_unit) = match name {
        "claude_code.session.count" => ("count", "session-count", "sessions"),
        "claude_code.lines_of_code.count" => ("count", "lines-of-code", "lines"),
        "claude_code.pull_request.count" => ("count", "pull-requests", "pull-requests"),
        "claude_code.commit.count" => ("count", "commits", "commits"),
        "claude_code.cost.usage" => ("USD", "source-cost-estimate", "usd"),
        "claude_code.token.usage" => ("tokens", "token-usage", "tokens"),
        "claude_code.code_edit_tool.decision" => ("count", "code-edit-decision", "decisions"),
        "claude_code.active_time.total" => ("s", "active-time", "seconds"),
        _ => return None,
    };
    Some(MetricContract {
        wire_unit,
        canonical_name,
        canonical_unit,
    })
}

fn known_metric_name(value: &str) -> &'static str {
    [
        "session-count",
        "lines-of-code",
        "pull-requests",
        "commits",
        "source-cost-estimate",
        "token-usage",
        "code-edit-decision",
        "active-time",
    ]
    .into_iter()
    .find(|known| *known == value)
    .expect("queued metric names come from the pinned metric contract")
}

fn known_metric_unit(value: &str) -> &'static str {
    [
        "sessions",
        "lines",
        "pull-requests",
        "commits",
        "usd",
        "tokens",
        "decisions",
        "seconds",
    ]
    .into_iter()
    .find(|known| *known == value)
    .expect("queued metric units come from the pinned metric contract")
}

fn metric_point_count(metric: &Map<String, Value>) -> usize {
    [
        "sum",
        "gauge",
        "histogram",
        "exponentialHistogram",
        "summary",
    ]
    .into_iter()
    .find_map(|kind| {
        metric
            .get(kind)
            .and_then(Value::as_object)
            .and_then(|object| object.get("dataPoints"))
            .and_then(Value::as_array)
            .map(Vec::len)
    })
    .unwrap_or(0)
}

fn points_or_one(sum: &Map<String, Value>) -> usize {
    sum.get("dataPoints")
        .and_then(Value::as_array)
        .map_or(1, |points| points.len().max(1))
}

fn nested_record_count(scopes: Option<&Value>, record_field: &str) -> Result<usize, ShapeError> {
    let scopes = required_array(scopes, "scopes")?;
    let mut count = 0usize;
    for scope in scopes {
        let scope = required_object(Some(scope), "scope")?;
        let records = required_array(scope.get(record_field), record_field)?;
        count = count.saturating_add(records.len());
    }
    Ok(count)
}

fn nested_metric_point_count(scopes: Option<&Value>) -> Result<usize, ShapeError> {
    let scopes = required_array(scopes, "scopeMetrics")?;
    let mut count = 0usize;
    for scope in scopes {
        let scope = required_object(Some(scope), "scopeMetrics item")?;
        let metrics = required_array(scope.get("metrics"), "metrics")?;
        for metric in metrics {
            let metric = required_object(Some(metric), "metric")?;
            count = count.saturating_add(metric_point_count(metric).max(1));
        }
    }
    Ok(count)
}

fn required_array<'a>(value: Option<&'a Value>, _name: &str) -> Result<&'a Vec<Value>, ShapeError> {
    value.and_then(Value::as_array).ok_or_else(|| {
        ShapeError::new(
            "W_OTEL_REQUIRED_ARRAY",
            "A required pinned telemetry array was absent or had the wrong type.",
        )
    })
}

fn required_object<'a>(
    value: Option<&'a Value>,
    _name: &str,
) -> Result<&'a Map<String, Value>, ShapeError> {
    value.and_then(Value::as_object).ok_or_else(|| {
        ShapeError::new(
            "W_OTEL_REQUIRED_OBJECT",
            "A required pinned telemetry object was absent or had the wrong type.",
        )
    })
}

fn require_limit(
    actual: usize,
    maximum: usize,
    code: &'static str,
    message: &'static str,
) -> Result<(), ShapeError> {
    if actual > maximum {
        Err(ShapeError::new(code, message))
    } else {
        Ok(())
    }
}

fn decoded_text_bytes(value: &Value) -> usize {
    match value {
        Value::String(value) => value.len(),
        Value::Array(values) => values
            .iter()
            .map(decoded_text_bytes)
            .fold(0usize, usize::saturating_add),
        Value::Object(values) => values
            .iter()
            .map(|(key, value)| key.len().saturating_add(decoded_text_bytes(value)))
            .fold(0usize, usize::saturating_add),
        _ => 0,
    }
}

fn count_unknown_object_fields(
    diagnostics: &mut Diagnostics,
    source: &Source,
    object: &Map<String, Value>,
    allowed: &[&str],
) {
    let count = object
        .keys()
        .filter(|key| !allowed.contains(&key.as_str()))
        .count();
    if count == 0 {
        return;
    }
    diagnostics.unknown_fields = diagnostics.unknown_fields.saturating_add(count);
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.unknown_fields = stats.unknown_fields.saturating_add(count);
        stats.partial = true;
    }
    warn_once(
        diagnostics,
        "W_OTEL_UNKNOWN_FIELDS",
        "Unknown telemetry fields were ignored and counted without retaining their names or values.",
        Some(source.alias.clone()),
    );
}

fn record_attribute_diagnostics(
    diagnostics: &mut Diagnostics,
    source: &Source,
    attributes: &Attributes,
) {
    if attributes.redactions > 0 {
        diagnostics.redacted_fields = diagnostics
            .redacted_fields
            .saturating_add(attributes.redactions);
        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
            stats.redacted_fields = stats.redacted_fields.saturating_add(attributes.redactions);
        }
    }
    if attributes.unknown_fields > 0 {
        diagnostics.unknown_fields = diagnostics
            .unknown_fields
            .saturating_add(attributes.unknown_fields);
        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
            stats.unknown_fields = stats
                .unknown_fields
                .saturating_add(attributes.unknown_fields);
            stats.partial = true;
        }
        warn_once(
            diagnostics,
            "W_OTEL_UNKNOWN_FIELDS",
            "Unknown telemetry fields were ignored and counted without retaining their names or values.",
            Some(source.alias.clone()),
        );
    }
}

fn record_transformed_redactions(diagnostics: &mut Diagnostics, source: &Source, count: usize) {
    if count == 0 {
        return;
    }
    diagnostics.redacted_fields = diagnostics.redacted_fields.saturating_add(count);
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.redacted_fields = stats.redacted_fields.saturating_add(count);
    }
}

fn record_declared_attribute_drops(
    entity: Option<&Map<String, Value>>,
    diagnostics: &mut Diagnostics,
    source: &Source,
) -> Result<bool, ShapeError> {
    let Some(raw) = entity.and_then(|entity| entity.get("droppedAttributesCount")) else {
        return Ok(false);
    };
    let count = json_u64(raw).ok_or_else(|| {
        ShapeError::new(
            "W_OTEL_DROPPED_ATTRIBUTE_COUNT_INVALID",
            "A droppedAttributesCount field had the wrong pinned value type.",
        )
    })?;
    if count > 0 {
        mark_partial_warning(
            diagnostics,
            source,
            "W_OTEL_UPSTREAM_DROPPED_ATTRIBUTES",
            "The telemetry producer reported dropped attributes; affected capabilities are partial.",
        );
    }
    Ok(count > 0)
}

fn record_malformed(diagnostics: &mut Diagnostics, source: &Source, code: &str, message: &str) {
    diagnostics.malformed_records = diagnostics.malformed_records.saturating_add(1);
    diagnostics.analytical_claims_uncertain = true;
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.malformed_records = stats.malformed_records.saturating_add(1);
        stats.partial = true;
    }
    warn_once(diagnostics, code, message, Some(source.alias.clone()));
}

fn record_skipped(diagnostics: &mut Diagnostics, source: &Source) {
    diagnostics.skipped_records = diagnostics.skipped_records.saturating_add(1);
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.skipped_records = stats.skipped_records.saturating_add(1);
        stats.partial = true;
    }
}

fn record_unsupported(diagnostics: &mut Diagnostics, source: &Source, code: &str, message: &str) {
    record_unsupported_many(diagnostics, source, 1, code, message);
}

fn record_unsupported_alias(
    diagnostics: &mut Diagnostics,
    source_alias: &str,
    code: &str,
    message: &str,
) {
    diagnostics.unsupported_records = diagnostics.unsupported_records.saturating_add(1);
    if let Some(stats) = diagnostics.sources.get_mut(source_alias) {
        stats.unsupported_records = stats.unsupported_records.saturating_add(1);
        stats.partial = true;
    }
    warn_once(diagnostics, code, message, Some(source_alias.to_string()));
}

fn record_unsupported_many(
    diagnostics: &mut Diagnostics,
    source: &Source,
    count: usize,
    code: &str,
    message: &str,
) {
    if count > 0 {
        diagnostics.analytical_claims_uncertain = true;
    }
    record_known_irrelevant_many(diagnostics, source, count, code, message);
}

fn record_known_irrelevant(
    diagnostics: &mut Diagnostics,
    source: &Source,
    code: &str,
    message: &str,
) {
    record_known_irrelevant_many(diagnostics, source, 1, code, message);
}

fn record_known_irrelevant_many(
    diagnostics: &mut Diagnostics,
    source: &Source,
    count: usize,
    code: &str,
    message: &str,
) {
    diagnostics.unsupported_records = diagnostics.unsupported_records.saturating_add(count);
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.unsupported_records = stats.unsupported_records.saturating_add(count);
        stats.partial = true;
    }
    warn_once(diagnostics, code, message, Some(source.alias.clone()));
}

fn mark_partial_warning(diagnostics: &mut Diagnostics, source: &Source, code: &str, message: &str) {
    mark_partial_warning_alias(diagnostics, &source.alias, code, message);
}

fn mark_partial_warning_alias(
    diagnostics: &mut Diagnostics,
    source_alias: &str,
    code: &str,
    message: &str,
) {
    if let Some(stats) = diagnostics.sources.get_mut(source_alias) {
        stats.partial = true;
    }
    warn_once(diagnostics, code, message, Some(source_alias.to_string()));
}

fn warn_once(
    diagnostics: &mut Diagnostics,
    code: &str,
    message: &str,
    source_alias: Option<String>,
) {
    if diagnostics
        .warnings
        .iter()
        .any(|warning| warning.code == code && warning.source_alias == source_alias)
    {
        return;
    }
    diagnostics.warning(code, message, source_alias);
}

#[cfg(test)]
#[cfg_attr(windows, allow(unused_imports))]
mod tests {
    use super::{
        ingest, metric_stream_limit_reached, AttributeLayers, Attributes, MetricTracker,
        OtelOptions, Scalar, Source, ATTRIBUTE_LAYER_PROBES,
    };
    use crate::ingestion::discovery::{self, DiscoveryOptions, SourceKind};
    use crate::ingestion::types::{
        AliasRegistry, Diagnostics, FileSnapshot, PrivacyHasher, SourceStats,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn source() -> Source {
        let metadata = fs::metadata(".").unwrap();
        Source {
            alias: "otel-1".to_string(),
            kind: SourceKind::Otel,
            path: PathBuf::from("synthetic"),
            discovery_snapshot: FileSnapshot::capture(&metadata),
        }
    }

    fn diagnostics() -> Diagnostics {
        let mut diagnostics = Diagnostics::default();
        diagnostics.sources.insert(
            "otel-1".to_string(),
            SourceStats::otel("otel-1".to_string()),
        );
        diagnostics
    }

    #[test]
    fn otel_inherited_attribute_merge_work_is_linear() {
        let inherited_bytes = 64usize * 1024;
        let records = 100usize;
        let mut resource = Attributes {
            identity_material: "r".repeat(inherited_bytes),
            ..Attributes::default()
        };
        resource.values.insert(
            "session.id".to_string(),
            Scalar::String("resource-session".to_string()),
        );
        resource.values.insert(
            "precedence".to_string(),
            Scalar::String("resource".to_string()),
        );
        let mut scope = Attributes {
            identity_material: "s".repeat(inherited_bytes),
            ..Attributes::default()
        };
        scope.values.insert(
            "model".to_string(),
            Scalar::String("scope-model".to_string()),
        );
        scope.values.insert(
            "precedence".to_string(),
            Scalar::String("scope".to_string()),
        );
        scope.values.insert(
            "typed-shadow".to_string(),
            Scalar::String("scope".to_string()),
        );
        let mut local = Attributes::default();
        local
            .values
            .insert("output_tokens".to_string(), Scalar::Integer(10));
        local.values.insert(
            "precedence".to_string(),
            Scalar::String("local".to_string()),
        );
        local
            .values
            .insert("typed-shadow".to_string(), Scalar::Integer(7));
        ATTRIBUTE_LAYER_PROBES.with(|probes| probes.set(0));

        for _ in 0..records {
            let attributes = AttributeLayers::new(&resource, &scope, &local);
            assert_eq!(attributes.u64("output_tokens"), Some(10));
            assert_eq!(attributes.string("model"), Some("scope-model"));
            assert_eq!(attributes.string("session.id"), Some("resource-session"));
            assert_eq!(attributes.string("precedence"), Some("local"));
            assert_eq!(attributes.string("typed-shadow"), None);
            assert_eq!(attributes.u64("typed-shadow"), Some(7));
            assert!(!attributes.contains("missing"));
        }

        let probes = ATTRIBUTE_LAYER_PROBES.with(std::cell::Cell::get);
        let linear_ceiling = records.saturating_mul(15);
        assert!(
            probes <= linear_ceiling,
            "layered attribute lookup performed {probes} probes for {records} records"
        );
        assert_eq!(resource.identity_material.len(), inherited_bytes);
        assert_eq!(scope.identity_material.len(), inherited_bytes);
    }

    #[cfg(unix)]
    #[test]
    fn otel_discovery_open_replacement_is_rejected() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-otel-discovery-open-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("telemetry.jsonl");
        let replacement = root.join("replacement.jsonl");
        fs::write(&path, "\n").unwrap();
        fs::write(&replacement, "\n").unwrap();
        let discovery = discovery::discover(&DiscoveryOptions {
            data_dirs: Vec::new(),
            otel_files: vec![path.clone()],
            claude_config_dir: None,
            home_dir: None,
            private_diagnostics: false,
        })
        .unwrap();
        let source = discovery
            .sources
            .iter()
            .find(|source| source.kind == SourceKind::Otel)
            .unwrap();
        fs::rename(&replacement, &path).unwrap();
        let mut diagnostics = discovery.diagnostics;
        let mut aliases = AliasRegistry::default();
        let mut private_prompts = Vec::new();
        let mut tracker = MetricTracker::default();
        let result = ingest(
            source,
            &OtelOptions {
                time_context: super::super::TimeContext::new("UTC", Some(2026)).unwrap(),
                maximum_line_bytes: 1024,
                maximum_events: 10,
                read_accounting: std::sync::Arc::new(super::super::SourceReadAccounting::default()),
            },
            &mut diagnostics,
            &PrivacyHasher::new(),
            &mut aliases,
            &mut private_prompts,
            &mut tracker,
        );
        fs::remove_dir_all(&root).unwrap();

        let error = result.expect_err("same-path replacement was accepted");
        assert!(error.to_string().contains("between discovery and open"));
        assert_eq!(diagnostics.accepted_records, 0);
        assert_eq!(diagnostics.earliest, None);
    }

    #[test]
    fn rejected_line_rolls_back_only_compact_transaction_state() {
        let mut diagnostics = diagnostics();
        diagnostics.sources.insert(
            "otel-2".to_string(),
            SourceStats::otel("otel-2".to_string()),
        );
        let checkpoint = diagnostics.checkpoint_otel_line("otel-1");

        diagnostics.unsupported_records = 9;
        diagnostics.unknown_records = 8;
        diagnostics.redacted_fields = 7;
        diagnostics.earliest = Some((1, "1970-01-01T00:00:00Z".to_string()));
        diagnostics.latest = Some((2, "1970-01-01T00:00:01Z".to_string()));
        diagnostics.saw_source_cost = true;
        diagnostics.sources.get_mut("otel-1").unwrap().partial = true;
        diagnostics.sources.get_mut("otel-2").unwrap().partial = true;
        diagnostics.warning("W_SYNTHETIC", "synthetic", Some("otel-1".to_string()));
        diagnostics.unknown_shapes.push(Default::default());

        diagnostics.rollback_otel_line("otel-1", checkpoint);

        assert_eq!(diagnostics.unsupported_records, 0);
        assert_eq!(diagnostics.unknown_records, 0);
        assert_eq!(diagnostics.redacted_fields, 0);
        assert_eq!(diagnostics.earliest, None);
        assert_eq!(diagnostics.latest, None);
        assert!(!diagnostics.saw_source_cost);
        assert!(!diagnostics.sources["otel-1"].partial);
        assert!(diagnostics.sources["otel-2"].partial);
        assert!(diagnostics.warnings.is_empty());
        assert!(diagnostics.unknown_shapes.is_empty());
    }

    #[test]
    fn cumulative_points_emit_differences_and_resets_without_negative_values() {
        let source = source();
        let mut diagnostics = diagnostics();
        let mut tracker = MetricTracker::default();
        let first = tracker
            .apply(1, 2, 100, 200, 10.0, &mut diagnostics, &source.alias)
            .unwrap();
        let second = tracker
            .apply(1, 2, 100, 300, 16.0, &mut diagnostics, &source.alias)
            .unwrap();
        let reset = tracker
            .apply(1, 2, 300, 400, 3.0, &mut diagnostics, &source.alias)
            .unwrap();
        assert_eq!(first.value, 10.0);
        assert_eq!(second.value, 6.0);
        assert_eq!(second.interval_start, 200);
        assert_eq!(reset.value, 3.0);
        assert!(diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "W_OTEL_METRIC_RESET"));
    }

    #[test]
    fn overlapping_delta_is_excluded_but_exact_repeat_is_idempotent() {
        let source = source();
        let mut diagnostics = diagnostics();
        let mut tracker = MetricTracker::default();
        let first = tracker
            .apply(1, 1, 100, 200, 4.0, &mut diagnostics, &source.alias)
            .unwrap();
        let repeated = tracker
            .apply(1, 1, 100, 200, 4.0, &mut diagnostics, &source.alias)
            .unwrap();
        let overlap = tracker.apply(1, 1, 150, 250, 5.0, &mut diagnostics, &source.alias);
        assert_eq!(first.value, repeated.value);
        assert!(overlap.is_none());
        assert!(diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "W_OTEL_METRIC_OVERLAP"));
    }

    #[test]
    fn changed_cumulative_start_cannot_hide_an_overlap() {
        let source = source();
        let mut diagnostics = diagnostics();
        let mut tracker = MetricTracker::default();
        assert!(tracker
            .apply(1, 2, 100, 300, 10.0, &mut diagnostics, &source.alias)
            .is_some());
        assert!(tracker
            .apply(1, 2, 250, 400, 3.0, &mut diagnostics, &source.alias)
            .is_none());
        assert!(diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "W_OTEL_METRIC_OVERLAP"));
        assert!(diagnostics.sources["otel-1"].partial);
    }

    #[test]
    fn metric_stream_cardinality_rejects_only_new_identities_at_the_limit() {
        let source = source();
        let mut diagnostics = diagnostics();
        let mut tracker = MetricTracker::default();
        assert!(tracker
            .apply(11, 1, 100, 200, 1.0, &mut diagnostics, &source.alias)
            .is_some());
        assert!(!metric_stream_limit_reached(&tracker, 11, 1));
        assert!(metric_stream_limit_reached(&tracker, 12, 1));
    }

    #[test]
    fn delta_gaps_are_partial_and_distinct_writer_keys_do_not_interfere() {
        let source = source();
        let mut diagnostics = diagnostics();
        let mut tracker = MetricTracker::default();
        let first = tracker
            .apply(11, 1, 100, 200, 4.0, &mut diagnostics, &source.alias)
            .unwrap();
        let second_writer = tracker
            .apply(12, 1, 100, 200, 9.0, &mut diagnostics, &source.alias)
            .unwrap();
        let after_gap = tracker
            .apply(11, 1, 300, 400, 5.0, &mut diagnostics, &source.alias)
            .unwrap();
        assert_eq!(first.value, 4.0);
        assert_eq!(second_writer.value, 9.0);
        assert_eq!(after_gap.value, 5.0);
        assert!(diagnostics
            .warnings
            .iter()
            .any(|warning| warning.code == "W_OTEL_METRIC_GAP"));
        assert!(diagnostics.sources["otel-1"].partial);
    }
}
