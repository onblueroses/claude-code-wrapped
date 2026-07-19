use super::discovery::Source;
use super::line_reader::{BoundedLines, DigestingFile};
use super::types::{
    classified_tool_name, safe_model_name, safe_source_cost, AliasRegistry, Diagnostics, EventKind,
    FileIdentity, FileSnapshot, NormalizedEvent, PrivacyHasher, PrivatePrompt, SourceStats,
    TokenFacts, MAX_UNKNOWN_SHAPE_DIAGNOSTICS, NORMALIZED_SCHEMA, TRANSCRIPT_ADAPTER,
};
use chrono::{SecondsFormat, Utc};
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::fs::{self, File};
use std::io::BufReader;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

const PRIVATE_PROMPT_LIMIT: usize = 65_536;
const PRIVATE_ENTRYPOINT_LIMIT: usize = 512;
const MAX_PRIVATE_PROMPTS: usize = 10_000;
const MAX_PRIVATE_CONTENT_BYTES: usize = 32 * 1024 * 1024;
const MAX_TRANSCRIPT_FILES: usize = 100_000;
const MAX_TRANSCRIPT_ENTRIES: usize = 100_000;
const MAX_DIRECTORY_DEPTH: usize = 128;
const MAX_TOOL_NAMES: usize = 128;

#[derive(Debug, Clone)]
pub(super) struct TranscriptOptions {
    pub time_context: super::TimeContext,
    pub maximum_line_bytes: usize,
    pub maximum_events: usize,
    pub include_private_content: bool,
    pub worker_count: usize,
    pub worker_delay_seed: Option<u64>,
    pub worker_panic_file: Option<usize>,
    pub read_accounting: Arc<super::SourceReadAccounting>,
}

#[derive(Debug, Clone)]
pub(super) struct TranscriptError {
    message: String,
}

impl TranscriptError {
    fn source(source: &Source, action: &str, error: impl fmt::Display) -> Self {
        Self {
            message: format!(
                "{action} failed for {}: {error}; the source is indeterminate",
                source.alias
            ),
        }
    }

    pub(super) fn is_source_work_limit(&self) -> bool {
        self.message.contains(super::SOURCE_WORK_LIMIT_CODE)
    }
}

impl fmt::Display for TranscriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for TranscriptError {}

#[derive(Debug)]
struct FileContext {
    project_raw: OsString,
    session_raw: OsString,
    parent_session_raw: Option<OsString>,
    is_subagent: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum FileDedupKey {
    FileSystem(FileIdentity),
    CanonicalPath(PathBuf),
}

#[derive(Debug, Default)]
struct FileDiscovery {
    seen_dirs: HashSet<PathBuf>,
    directories: Vec<DiscoveredDirectory>,
    seen_files: HashSet<FileDedupKey>,
    files: Vec<DiscoveredFile>,
}

#[derive(Debug)]
struct DiscoveredDirectory {
    path: PathBuf,
    snapshot: FileSnapshot,
}

#[derive(Debug)]
struct DiscoveredFile {
    path: PathBuf,
    snapshot: FileSnapshot,
}

#[derive(Debug)]
struct FileParseResult {
    events: Vec<NormalizedEvent>,
    diagnostics: Diagnostics,
    content_digest: [u8; 32],
    cached_event_payload: Option<Vec<u8>>,
    cached_diagnostics_payload: Option<Vec<u8>>,
}

pub(super) struct PreparedTranscript {
    pub events: Vec<NormalizedEvent>,
    pub append_safe: bool,
    pub file_alias_remap: Vec<(String, String)>,
    pub full_fallback: PreparedFullTranscript,
}

pub(super) struct PreparedFullTranscript {
    source: Source,
    files: Vec<DiscoveredFile>,
    prepared: Vec<PreparedFile>,
    diagnostics: Diagnostics,
    maximum_events: usize,
}

pub(super) struct FullTranscript {
    pub events: Vec<NormalizedEvent>,
    pub diagnostics: Diagnostics,
    pub aliases: AliasRegistry,
    pub store_files: Vec<super::store::SourceFile>,
}

#[derive(Debug)]
enum WorkerMessage {
    File {
        index: usize,
        result: Box<Result<FileParseResult, TranscriptError>>,
    },
    Panic,
    Finished,
}

#[derive(Debug)]
struct ParallelControl {
    cancelled: AtomicBool,
    event_count: AtomicUsize,
    maximum_events: usize,
}

impl ParallelControl {
    fn new(maximum_events: usize) -> Self {
        Self {
            cancelled: AtomicBool::new(false),
            event_count: AtomicUsize::new(0),
            maximum_events,
        }
    }

    fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    fn reserve_event(&self) -> bool {
        self.event_count
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                count
                    .checked_add(1)
                    .filter(|next| *next <= self.maximum_events)
            })
            .is_ok()
    }
}

#[derive(Debug)]
pub(super) struct TraversalBudget {
    traversed_entries: usize,
    maximum_entries: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // The binary compiles this private module without the library compatibility API.
pub(crate) enum CompatibilityPathScope {
    AllJsonl,
    DirectSessions,
}

impl Default for TraversalBudget {
    fn default() -> Self {
        Self {
            traversed_entries: 0,
            maximum_entries: MAX_TRANSCRIPT_ENTRIES,
        }
    }
}

impl TraversalBudget {
    fn consume_entry(&mut self, source: &Source) -> Result<(), TranscriptError> {
        let Some(next) = self.traversed_entries.checked_add(1) else {
            return Err(TranscriptError {
                message: format!(
                    "{} exceeded the invocation-wide directory-entry safety limit; narrow the selected sources",
                    source.alias
                ),
            });
        };
        if next > self.maximum_entries {
            return Err(TranscriptError {
                message: format!(
                    "{} exceeded the invocation-wide directory-entry safety limit; narrow the selected sources",
                    source.alias
                ),
            });
        }
        self.traversed_entries = next;
        Ok(())
    }
}

#[cfg(test)]
impl TraversalBudget {
    fn with_maximum(maximum_entries: usize) -> Self {
        Self {
            traversed_entries: 0,
            maximum_entries,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn ingest(
    source: &Source,
    options: &TranscriptOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    private_prompts: &mut Vec<PrivatePrompt>,
    private_content_bytes: &mut usize,
    traversal_budget: &mut TraversalBudget,
    store_files: &mut Vec<super::store::SourceFile>,
    file_cache: Option<&super::store::FileCache>,
) -> Result<Vec<NormalizedEvent>, TranscriptError> {
    let discovery = discover_files_with_budget(source, diagnostics, traversal_budget)?;
    diagnostics.files_discovered = diagnostics
        .files_discovered
        .saturating_add(discovery.files.len());
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.files_discovered = discovery.files.len();
    }

    let worker_count = options
        .worker_count
        .max(1)
        .min(discovery.files.len().max(1));
    if options.include_private_content {
        let mut content_digests = Vec::with_capacity(discovery.files.len());
        let events = ingest_files_serial(
            source,
            &discovery.files,
            options,
            diagnostics,
            hasher,
            aliases,
            private_prompts,
            private_content_bytes,
            &mut content_digests,
        )?;
        validate_discovered_directories(source, &discovery.directories, "transcript ingestion")?;
        store_files.extend(
            discovery
                .files
                .into_iter()
                .zip(content_digests)
                .enumerate()
                .map(|(index, (file, content_digest))| {
                    super::store::SourceFile::with_content_digest(
                        file.path,
                        source.path.clone(),
                        source.alias.clone(),
                        source.kind,
                        file.snapshot,
                        content_digest,
                    )
                    .with_file_alias(file_alias(source, index))
                }),
        );
        return Ok(events);
    }

    let mut cached_payloads = std::iter::repeat_with(|| None)
        .take(discovery.files.len())
        .collect::<Vec<Option<super::store::RawCachedFile>>>();
    if let Some(cache) = file_cache {
        for (index, file) in discovery.files.iter().enumerate() {
            let cached = cache
                .lookup_raw(
                    &file.path,
                    &source.path,
                    &source.alias,
                    source.kind,
                    &file.snapshot,
                )
                .map_err(|error| TranscriptError {
                    message: error.to_string(),
                })?;
            cached_payloads[index] = cached;
        }
    }
    let cached_results = decode_cached_results(source, cached_payloads, worker_count)?;
    let results = ingest_files_parallel(
        source,
        &discovery.files,
        options,
        hasher,
        worker_count,
        cached_results,
    )?;
    let total_events = results.iter().try_fold(0usize, |count, result| {
        count.checked_add(result.events.len())
    });
    if total_events.is_none_or(|count| count > options.maximum_events) {
        return Err(normalized_event_limit_error(source));
    }
    let mut events = Vec::with_capacity(total_events.unwrap_or(0));
    for (index, (file, mut result)) in discovery.files.iter().zip(results).enumerate() {
        let source_file = super::store::SourceFile::with_content_digest(
            file.path.clone(),
            source.path.clone(),
            source.alias.clone(),
            source.kind,
            file.snapshot.clone(),
            result.content_digest,
        )
        .with_file_alias(file_alias(source, index));
        let source_file = if file_cache.is_some() {
            match (
                result.cached_event_payload.take(),
                result.cached_diagnostics_payload.take(),
            ) {
                (Some(events), Some(diagnostics)) => {
                    source_file.with_encoded_payload(result.events.len(), events, diagnostics, None)
                }
                (None, None) => source_file
                    .with_payload(&result.events, &result.diagnostics, None)
                    .map_err(|error| TranscriptError {
                        message: error.to_string(),
                    })?,
                _ => {
                    return Err(TranscriptError {
                        message: "the cached transcript payload was incomplete".to_string(),
                    })
                }
            }
        } else {
            source_file
        };
        store_files.push(source_file);
        diagnostics.merge_file_parse(result.diagnostics);
        for event in &mut result.events {
            assign_event_aliases(event, aliases);
        }
        events.append(&mut result.events);
    }
    validate_discovered_directories(source, &discovery.directories, "transcript ingestion")?;
    Ok(events)
}

enum PreparedFile {
    Unchanged {
        raw: super::store::RawCachedFile,
    },
    Parsed {
        result: Box<FileParseResult>,
        previous: Option<Box<super::store::CachedFile>>,
        prefix_matches: bool,
    },
}

impl PreparedFullTranscript {
    pub fn materialize(self) -> Result<FullTranscript, TranscriptError> {
        let Self {
            source,
            files,
            prepared,
            mut diagnostics,
            maximum_events,
        } = self;
        let total_events = prepared.iter().try_fold(0usize, |total, file| {
            let count = match file {
                PreparedFile::Unchanged { raw } => raw.event_count(),
                PreparedFile::Parsed { result, .. } => result.events.len(),
            };
            total.checked_add(count)
        });
        if total_events.is_none_or(|count| count > maximum_events) {
            return Err(normalized_event_limit_error(&source));
        }

        let mut events = Vec::with_capacity(total_events.unwrap_or_default());
        let mut aliases = AliasRegistry::default();
        let mut store_files = Vec::with_capacity(files.len());
        for (index, (file, prepared_file)) in files.into_iter().zip(prepared).enumerate() {
            let file_alias = file_alias(&source, index);
            let mut result = match prepared_file {
                PreparedFile::Unchanged { raw } => {
                    if !raw.events_available() {
                        return Err(TranscriptError {
                            message: format!(
                                "{} cached normalized events are unavailable; rerun with --rebuild-store",
                                source.alias
                            ),
                        });
                    }
                    let mut cached =
                        super::store::decode_cached_file(raw).map_err(|error| TranscriptError {
                            message: error.to_string(),
                        })?;
                    rewrite_cached_aliases(
                        &mut cached.events,
                        &mut cached.diagnostics,
                        &source.alias,
                        &file_alias,
                    );
                    let source_file = super::store::SourceFile::with_content_digest(
                        file.path.clone(),
                        source.path.clone(),
                        source.alias.clone(),
                        source.kind,
                        file.snapshot.clone(),
                        cached.content_digest,
                    )
                    .with_file_alias(file_alias.clone())
                    .with_encoded_payload(
                        cached.event_count,
                        cached.event_payload.clone(),
                        cached.diagnostics_payload.clone(),
                        cached.metric_state.clone(),
                    );
                    store_files.push(source_file);
                    FileParseResult {
                        events: cached.events,
                        diagnostics: cached.diagnostics,
                        content_digest: cached.content_digest,
                        cached_event_payload: Some(cached.event_payload),
                        cached_diagnostics_payload: Some(cached.diagnostics_payload),
                    }
                }
                PreparedFile::Parsed { result, .. } => {
                    let result = *result;
                    let source_file = super::store::SourceFile::with_content_digest(
                        file.path.clone(),
                        source.path.clone(),
                        source.alias.clone(),
                        source.kind,
                        file.snapshot,
                        result.content_digest,
                    )
                    .with_file_alias(file_alias)
                    .with_payload(&result.events, &result.diagnostics, None)
                    .map_err(|error| TranscriptError {
                        message: error.to_string(),
                    })?;
                    store_files.push(source_file);
                    result
                }
            };
            diagnostics.merge_file_parse(result.diagnostics);
            for event in &mut result.events {
                assign_event_aliases(event, &mut aliases);
            }
            events.append(&mut result.events);
        }
        Ok(FullTranscript {
            events,
            diagnostics,
            aliases,
            store_files,
        })
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn ingest_prepared_append(
    source: &Source,
    options: &TranscriptOptions,
    current_diagnostics: &mut Diagnostics,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    traversal_budget: &mut TraversalBudget,
    store_files: &mut Vec<super::store::SourceFile>,
    file_cache: &super::store::FileCache,
) -> Result<PreparedTranscript, TranscriptError> {
    let discovery = discover_files_with_budget(source, current_diagnostics, traversal_budget)?;
    current_diagnostics.files_discovered = current_diagnostics
        .files_discovered
        .saturating_add(discovery.files.len());
    if let Some(stats) = current_diagnostics.sources.get_mut(&source.alias) {
        stats.files_discovered = discovery.files.len();
    }
    let full_diagnostics = current_diagnostics.clone();

    let mut prepared = Vec::with_capacity(discovery.files.len());
    let mut append_safe = true;
    for (index, file) in discovery.files.iter().enumerate() {
        if let Some(raw) = file_cache
            .lookup_raw_deferred(
                &file.path,
                &source.path,
                &source.alias,
                source.kind,
                &file.snapshot,
            )
            .map_err(|error| TranscriptError {
                message: error.to_string(),
            })?
        {
            prepared.push(PreparedFile::Unchanged { raw });
            continue;
        }

        let previous = file_cache
            .take_previous_raw(
                &file.path,
                &source.path,
                &source.alias,
                source.kind,
                &file.snapshot,
            )
            .map_err(|error| TranscriptError {
                message: error.to_string(),
            })?;
        let previous_size = previous.as_ref().map(|cached| cached.source_bytes);
        let prefix_bytes = previous_size.filter(|size| file.snapshot.len() > *size);
        if previous.is_some() && prefix_bytes.is_none() {
            append_safe = false;
        }
        let (result, prefix_digest) =
            ingest_file_isolated(source, file, index, options, hasher, None, prefix_bytes)?;
        let previous = previous
            .map(|cached| {
                let expected_digest = cached.raw.content_digest();
                let prefix_matches = prefix_digest == Some(expected_digest);
                super::store::decode_cached_file(cached.raw)
                    .map(|decoded| (decoded, prefix_matches))
            })
            .transpose()
            .map_err(|error| TranscriptError {
                message: error.to_string(),
            })?;
        let (previous, prefix_matches) =
            previous.map_or((None, true), |(cached, matches)| (Some(cached), matches));
        append_safe &= prefix_matches;
        prepared.push(PreparedFile::Parsed {
            result: Box::new(result),
            previous: previous.map(Box::new),
            prefix_matches,
        });
    }
    append_safe &= !file_cache.has_remaining_source(&source.alias, source.kind);
    append_safe &= current_diagnostics
        .sources
        .get(&source.alias)
        .is_none_or(|stats| !stats.partial);

    let total_events = prepared.iter().fold(0usize, |total, file| match file {
        PreparedFile::Unchanged { .. } => total,
        PreparedFile::Parsed { result, .. } => total.saturating_add(result.events.len()),
    });
    if total_events > options.maximum_events {
        return Err(normalized_event_limit_error(source));
    }

    let prior_file_count = diagnostics
        .sources
        .get(&source.alias)
        .map_or(0usize, |stats| stats.files_discovered);
    if discovery.files.len() < prior_file_count {
        append_safe = false;
    }

    if !append_safe {
        validate_discovered_directories(
            source,
            &discovery.directories,
            "incremental transcript ingestion",
        )?;
        return Ok(PreparedTranscript {
            events: Vec::new(),
            append_safe: false,
            file_alias_remap: Vec::new(),
            full_fallback: PreparedFullTranscript {
                source: source.clone(),
                files: discovery.files,
                prepared,
                diagnostics: full_diagnostics,
                maximum_events: options.maximum_events,
            },
        });
    }

    let mut events = Vec::new();
    let mut file_alias_remap = Vec::new();
    diagnostics.files_discovered = diagnostics
        .files_discovered
        .saturating_add(discovery.files.len().saturating_sub(prior_file_count));
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.files_discovered = discovery.files.len();
    }
    for (index, (file, prepared_file)) in
        discovery.files.iter().zip(prepared.iter_mut()).enumerate()
    {
        let current_file_alias = file_alias(source, index);
        match prepared_file {
            PreparedFile::Unchanged { raw } => {
                if raw.file_alias() != current_file_alias {
                    file_alias_remap
                        .push((raw.file_alias().to_string(), current_file_alias.clone()));
                }
                store_files.push(
                    super::store::SourceFile::reused_metadata(
                        file.path.clone(),
                        source.path.clone(),
                        source.alias.clone(),
                        source.kind,
                        file.snapshot.clone(),
                        raw.content_digest(),
                    )
                    .with_file_alias(current_file_alias),
                );
            }
            PreparedFile::Parsed {
                result,
                previous,
                prefix_matches,
            } => {
                debug_assert!(*prefix_matches);
                let previous_diagnostics = previous
                    .as_deref()
                    .map_or_else(Diagnostics::default, |cached| cached.diagnostics.clone());
                let old_records = previous.as_deref().map_or(0, |cached| {
                    classified_file_records(&cached.diagnostics, cached.event_count)
                });
                if let Some(previous) = &previous {
                    if previous.file_alias != current_file_alias {
                        file_alias_remap
                            .push((previous.file_alias.clone(), current_file_alias.clone()));
                    }
                }
                let delta_diagnostics = result
                    .diagnostics
                    .append_file_delta(&previous_diagnostics)
                    .ok_or_else(|| TranscriptError {
                        message: format!("{} append diagnostics did not reconcile", source.alias),
                    })?;
                let source_file = super::store::SourceFile::with_content_digest(
                    file.path.clone(),
                    source.path.clone(),
                    source.alias.clone(),
                    source.kind,
                    file.snapshot.clone(),
                    result.content_digest,
                )
                .with_file_alias(current_file_alias)
                .with_payload(&result.events, &result.diagnostics, None)
                .map_err(|error| TranscriptError {
                    message: error.to_string(),
                })?;
                store_files.push(source_file);
                diagnostics.merge_file_parse(delta_diagnostics);
                let mut delta_events = result
                    .events
                    .iter()
                    .filter(|event| event.record_index > old_records)
                    .cloned()
                    .collect::<Vec<_>>();
                for event in &mut delta_events {
                    assign_event_aliases(event, aliases);
                }
                events.append(&mut delta_events);
            }
        }
        if options.worker_panic_file == Some(index) {
            return Err(TranscriptError {
                message: format!(
                    "{} append worker panic injection is incompatible with reuse",
                    source.alias
                ),
            });
        }
    }

    validate_discovered_directories(
        source,
        &discovery.directories,
        "incremental transcript ingestion",
    )?;
    Ok(PreparedTranscript {
        events,
        append_safe,
        file_alias_remap,
        full_fallback: PreparedFullTranscript {
            source: source.clone(),
            files: discovery.files,
            prepared,
            diagnostics: full_diagnostics,
            maximum_events: options.maximum_events,
        },
    })
}

fn classified_file_records(diagnostics: &Diagnostics, accepted_events: usize) -> u64 {
    let count = [
        accepted_events,
        diagnostics.malformed_records,
        diagnostics.unsupported_records,
        diagnostics.filtered_records,
        diagnostics.skipped_records,
        diagnostics.duplicate_records,
    ]
    .into_iter()
    .fold(0usize, usize::saturating_add);
    u64::try_from(count).unwrap_or(u64::MAX)
}

fn decode_cached_results(
    source: &Source,
    payloads: Vec<Option<super::store::RawCachedFile>>,
    worker_count: usize,
) -> Result<Vec<Option<FileParseResult>>, TranscriptError> {
    let file_count = payloads.len();
    let mut buckets = std::iter::repeat_with(Vec::new)
        .take(worker_count.max(1))
        .collect::<Vec<Vec<(usize, super::store::RawCachedFile)>>>();
    for (index, payload) in payloads.into_iter().enumerate() {
        if let Some(payload) = payload.filter(|payload| payload.events_available()) {
            let bucket = index % buckets.len();
            buckets[bucket].push((index, payload));
        }
    }
    let decoded = thread::scope(|scope| {
        let mut handles = Vec::new();
        for bucket in buckets {
            handles.push(scope.spawn(move || {
                bucket
                    .into_iter()
                    .map(|(index, payload)| {
                        let mut cached =
                            super::store::decode_cached_file(payload).map_err(|error| {
                                TranscriptError {
                                    message: error.to_string(),
                                }
                            })?;
                        let file_alias = file_alias(source, index);
                        rewrite_cached_aliases(
                            &mut cached.events,
                            &mut cached.diagnostics,
                            &source.alias,
                            &file_alias,
                        );
                        Ok((
                            index,
                            FileParseResult {
                                events: cached.events,
                                diagnostics: cached.diagnostics,
                                content_digest: cached.content_digest,
                                cached_event_payload: Some(cached.event_payload),
                                cached_diagnostics_payload: Some(cached.diagnostics_payload),
                            },
                        ))
                    })
                    .collect::<Result<Vec<_>, TranscriptError>>()
            }));
        }
        let mut decoded = Vec::new();
        for handle in handles {
            decoded.extend(handle.join().map_err(|_| TranscriptError {
                message: format!(
                    "{} cached transcript decoder panicked; no partial result was published",
                    source.alias
                ),
            })??);
        }
        Ok::<_, TranscriptError>(decoded)
    })?;
    let mut results = std::iter::repeat_with(|| None)
        .take(file_count)
        .collect::<Vec<Option<FileParseResult>>>();
    for (index, result) in decoded {
        results[index] = Some(result);
    }
    Ok(results)
}

#[allow(clippy::too_many_arguments)]
fn ingest_files_serial(
    source: &Source,
    files: &[DiscoveredFile],
    options: &TranscriptOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    private_prompts: &mut Vec<PrivatePrompt>,
    private_content_bytes: &mut usize,
    content_digests: &mut Vec<[u8; 32]>,
) -> Result<Vec<NormalizedEvent>, TranscriptError> {
    let mut events = Vec::new();
    for (file_index, file) in files.iter().enumerate() {
        let file_alias = file_alias(source, file_index);
        let context = file_context(&source.path, &file.path).ok_or_else(|| TranscriptError {
            message: format!(
                "the layout of {file_alias} is outside the supported transcript contract"
            ),
        })?;
        let content_digest = ingest_file(
            source,
            file,
            &file_alias,
            &context,
            options,
            diagnostics,
            hasher,
            aliases,
            private_prompts,
            private_content_bytes,
            &mut events,
            None,
        )?;
        content_digests.push(content_digest);
    }
    Ok(events)
}

fn ingest_files_parallel(
    source: &Source,
    files: &[DiscoveredFile],
    options: &TranscriptOptions,
    hasher: &PrivacyHasher,
    worker_count: usize,
    cached_results: Vec<Option<FileParseResult>>,
) -> Result<Vec<FileParseResult>, TranscriptError> {
    let next_file = AtomicUsize::new(0);
    let control = ParallelControl::new(options.maximum_events);
    let queue_capacity = worker_count.saturating_mul(2).max(1);
    let (sender, receiver) = mpsc::sync_channel(queue_capacity);
    let cached = cached_results
        .iter()
        .map(Option::is_some)
        .collect::<Vec<_>>();

    thread::scope(|scope| {
        for _ in 0..worker_count {
            let sender = sender.clone();
            let next_file = &next_file;
            let control = &control;
            let cached = &cached;
            scope.spawn(move || {
                let outcome = catch_unwind(AssertUnwindSafe(|| {
                    worker_loop(
                        source, files, options, hasher, cached, next_file, control, &sender,
                    )
                }));
                if outcome.is_err() {
                    control.cancel();
                    let _ = sender.send(WorkerMessage::Panic);
                }
                let _ = sender.send(WorkerMessage::Finished);
            });
        }
        drop(sender);

        let mut results = cached_results;
        let mut finished = 0usize;
        let mut first_error = None;
        while finished < worker_count {
            let message = receiver.recv().map_err(|_| TranscriptError {
                message: format!(
                    "{} transcript worker channel closed before completion",
                    source.alias
                ),
            })?;
            match message {
                WorkerMessage::File { index, result } => match *result {
                    Ok(result) => {
                        if results.get(index).is_none_or(|current| current.is_some()) {
                            control.cancel();
                            first_error.get_or_insert_with(|| TranscriptError {
                                message: format!(
                                    "{} transcript worker returned an invalid file index",
                                    source.alias
                                ),
                            });
                        } else {
                            results[index] = Some(result);
                        }
                    }
                    Err(error) => {
                        control.cancel();
                        first_error.get_or_insert(error);
                    }
                },
                WorkerMessage::Panic => {
                    first_error.get_or_insert_with(|| TranscriptError {
                        message: format!(
                            "{} transcript worker panicked; no partial result was published",
                            source.alias
                        ),
                    });
                }
                WorkerMessage::Finished => finished = finished.saturating_add(1),
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        results
            .into_iter()
            .enumerate()
            .map(|(index, result)| {
                result.ok_or_else(|| TranscriptError {
                    message: format!(
                        "{} transcript worker omitted file {}",
                        source.alias,
                        index.saturating_add(1)
                    ),
                })
            })
            .collect()
    })
}

#[allow(clippy::too_many_arguments)]
fn worker_loop(
    source: &Source,
    files: &[DiscoveredFile],
    options: &TranscriptOptions,
    hasher: &PrivacyHasher,
    cached: &[bool],
    next_file: &AtomicUsize,
    control: &ParallelControl,
    sender: &SyncSender<WorkerMessage>,
) {
    while !control.is_cancelled() {
        let index = next_file.fetch_add(1, Ordering::AcqRel);
        let Some(file) = files.get(index) else {
            break;
        };
        if cached.get(index) == Some(&true) {
            continue;
        }
        if options.worker_panic_file == Some(index) {
            panic!("injected transcript worker panic");
        }
        apply_worker_delay(options.worker_delay_seed, index);
        let result = ingest_file_parallel(source, file, index, options, hasher, control);
        let failed = result.is_err();
        if sender
            .send(WorkerMessage::File {
                index,
                result: Box::new(result),
            })
            .is_err()
        {
            control.cancel();
            break;
        }
        if failed {
            control.cancel();
            break;
        }
    }
}

fn ingest_file_parallel(
    source: &Source,
    file: &DiscoveredFile,
    file_index: usize,
    options: &TranscriptOptions,
    hasher: &PrivacyHasher,
    control: &ParallelControl,
) -> Result<FileParseResult, TranscriptError> {
    ingest_file_isolated(
        source,
        file,
        file_index,
        options,
        hasher,
        Some(control),
        None,
    )
    .map(|(result, _)| result)
}

fn ingest_file_isolated(
    source: &Source,
    file: &DiscoveredFile,
    file_index: usize,
    options: &TranscriptOptions,
    hasher: &PrivacyHasher,
    control: Option<&ParallelControl>,
    prefix_bytes: Option<u64>,
) -> Result<(FileParseResult, Option<[u8; 32]>), TranscriptError> {
    let file_alias = file_alias(source, file_index);
    let context = file_context(&source.path, &file.path).ok_or_else(|| TranscriptError {
        message: format!("the layout of {file_alias} is outside the supported transcript contract"),
    })?;
    let mut diagnostics = Diagnostics::default();
    diagnostics.sources.insert(
        source.alias.clone(),
        SourceStats::transcript(source.alias.clone(), "parallel-file".to_string()),
    );
    let mut aliases = AliasRegistry::default();
    let mut private_prompts = Vec::new();
    let mut private_content_bytes = 0usize;
    let mut events = Vec::new();
    let (content_digest, prefix_digest) = ingest_file_with_prefix(
        source,
        file,
        &file_alias,
        &context,
        options,
        &mut diagnostics,
        hasher,
        &mut aliases,
        &mut private_prompts,
        &mut private_content_bytes,
        &mut events,
        control,
        prefix_bytes,
    )?;
    debug_assert!(private_prompts.is_empty());
    Ok((
        FileParseResult {
            events,
            diagnostics,
            content_digest,
            cached_event_payload: None,
            cached_diagnostics_payload: None,
        },
        prefix_digest,
    ))
}

fn rewrite_cached_aliases(
    events: &mut [NormalizedEvent],
    diagnostics: &mut Diagnostics,
    source_alias: &str,
    file_alias: &str,
) {
    for event in events {
        event.source_alias.clear();
        event.source_alias.push_str(source_alias);
        event.file_alias.clear();
        event.file_alias.push_str(file_alias);
    }
    for shape in &mut diagnostics.unknown_shapes {
        shape.source_alias.clear();
        shape.source_alias.push_str(source_alias);
        shape.file_alias.clear();
        shape.file_alias.push_str(file_alias);
    }
}

fn file_alias(source: &Source, file_index: usize) -> String {
    format!("{}-file-{}", source.alias, file_index.saturating_add(1))
}

fn assign_event_aliases(event: &mut NormalizedEvent, aliases: &mut AliasRegistry) {
    event.project_alias = aliases.project(event.project_key);
    event.session_alias = aliases.session(event.session_key);
    event.parent_session_alias = event.parent_key.map(|key| aliases.session(key));
}

fn normalized_event_limit_error(source: &Source) -> TranscriptError {
    TranscriptError {
        message: format!(
            "{} exceeded the normalized-event safety limit; narrow the selected period",
            source.alias
        ),
    }
}

fn apply_worker_delay(seed: Option<u64>, file_index: usize) {
    let Some(seed) = seed else {
        return;
    };
    let mut mixed = seed ^ (file_index as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15);
    mixed ^= mixed >> 30;
    mixed = mixed.wrapping_mul(0xbf58_476d_1ce4_e5b9);
    mixed ^= mixed >> 27;
    let microseconds = mixed % 2_000;
    thread::sleep(Duration::from_micros(microseconds));
}

#[allow(dead_code)] // The binary compiles this private module without the library compatibility API.
pub(super) fn discover_compatibility_paths(
    source: &Source,
    diagnostics: &mut Diagnostics,
    traversal_budget: &mut TraversalBudget,
    scope: CompatibilityPathScope,
) -> Result<Vec<PathBuf>, TranscriptError> {
    let discovery = discover_files_with_budget(source, diagnostics, traversal_budget)?;
    validate_discovered_files(source, &discovery.files, "compatibility discovery")?;
    validate_discovered_directories(source, &discovery.directories, "compatibility discovery")?;

    let paths = discovery
        .files
        .into_iter()
        .filter(|file| match scope {
            CompatibilityPathScope::AllJsonl => true,
            CompatibilityPathScope::DirectSessions => file
                .path
                .strip_prefix(&source.path)
                .is_ok_and(|relative| relative.components().count() == 2),
        })
        .map(|file| file.path)
        .collect();
    Ok(paths)
}

#[allow(dead_code)] // The library compatibility copy does not probe the binary store.
pub(super) fn discover_store_files(
    source: &Source,
    diagnostics: &mut Diagnostics,
    traversal_budget: &mut TraversalBudget,
) -> Result<Vec<super::store::SourceFile>, TranscriptError> {
    let discovery = discover_files_with_budget(source, diagnostics, traversal_budget)?;
    validate_discovered_files(source, &discovery.files, "store inventory")?;
    validate_discovered_directories(source, &discovery.directories, "store inventory")?;
    Ok(discovery
        .files
        .into_iter()
        .map(|file| {
            super::store::SourceFile::metadata_only(
                file.path,
                source.path.clone(),
                source.alias.clone(),
                source.kind,
                file.snapshot,
            )
        })
        .collect())
}

#[allow(dead_code)] // Used by library compatibility discovery, absent from the binary's module copy.
fn validate_discovered_files(
    source: &Source,
    files: &[DiscoveredFile],
    activity: &str,
) -> Result<(), TranscriptError> {
    for file in files {
        let resolved = fs::canonicalize(&file.path).map_err(|_| TranscriptError {
            message: format!(
                "{} changed during {activity}; rerun against a stable snapshot",
                source.alias
            ),
        })?;
        if resolved != file.path || !resolved.starts_with(&source.path) {
            return Err(TranscriptError {
                message: format!(
                    "{} changed during {activity}; rerun against a stable snapshot",
                    source.alias
                ),
            });
        }
        let metadata = fs::metadata(&resolved).map_err(|_| TranscriptError {
            message: format!(
                "{} changed during {activity}; rerun against a stable snapshot",
                source.alias
            ),
        })?;
        let identity_matches =
            file.snapshot
                .matches_path(&metadata, &resolved)
                .map_err(|error| {
                    TranscriptError::source(source, "validation file identity read", error)
                })?;
        if !metadata.is_file() || !identity_matches {
            return Err(TranscriptError {
                message: format!(
                    "{} changed during {activity}; rerun against a stable snapshot",
                    source.alias
                ),
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ingest_file(
    source: &Source,
    discovered: &DiscoveredFile,
    file_alias: &str,
    context: &FileContext,
    options: &TranscriptOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    private_prompts: &mut Vec<PrivatePrompt>,
    private_content_bytes: &mut usize,
    events: &mut Vec<NormalizedEvent>,
    parallel: Option<&ParallelControl>,
) -> Result<[u8; 32], TranscriptError> {
    ingest_file_with_prefix(
        source,
        discovered,
        file_alias,
        context,
        options,
        diagnostics,
        hasher,
        aliases,
        private_prompts,
        private_content_bytes,
        events,
        parallel,
        None,
    )
    .map(|(content_digest, _)| content_digest)
}

#[allow(clippy::too_many_arguments)]
fn ingest_file_with_prefix(
    source: &Source,
    discovered: &DiscoveredFile,
    file_alias: &str,
    context: &FileContext,
    options: &TranscriptOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    private_prompts: &mut Vec<PrivatePrompt>,
    private_content_bytes: &mut usize,
    events: &mut Vec<NormalizedEvent>,
    parallel: Option<&ParallelControl>,
    prefix_bytes: Option<u64>,
) -> Result<([u8; 32], Option<[u8; 32]>), TranscriptError> {
    let path = &discovered.path;
    let resolved = fs::canonicalize(path)
        .map_err(|error| TranscriptError::source(source, "file canonicalization", error))?;
    if resolved.as_path() != path.as_path() || !resolved.starts_with(&source.path) {
        return Err(TranscriptError {
            message: format!(
                "{file_alias} changed identity after discovery; rerun against a stable snapshot"
            ),
        });
    }
    let file = File::open(&resolved)
        .map_err(|error| TranscriptError::source(source, "file open", error))?;
    let before = file
        .metadata()
        .map_err(|error| TranscriptError::source(source, "metadata read", error))?;
    if !before.is_file() {
        return Err(TranscriptError {
            message: format!("{file_alias} stopped being a regular file during discovery"),
        });
    }
    let before_snapshot = FileSnapshot::capture_file(&before, &file)
        .map_err(|error| TranscriptError::source(source, "opened file identity read", error))?;
    if discovered.snapshot != before_snapshot {
        return Err(TranscriptError {
            message: format!(
                "{file_alias} changed identity between discovery and open; rerun against a stable snapshot"
            ),
        });
    }
    let mut lines = BoundedLines::with_accounting(
        BufReader::new(
            DigestingFile::with_prefix(
                file,
                hasher.store_salt(),
                prefix_bytes,
                Arc::clone(&options.read_accounting),
            )
            .map_err(|error| TranscriptError::source(source, "stream budget", error))?,
        ),
        options.maximum_line_bytes,
        Arc::clone(&options.read_accounting),
    );
    let mut record_index = 0u64;

    while let Some(line) = lines
        .next_line()
        .map_err(|error| TranscriptError::source(source, "stream read", error))?
    {
        if parallel.is_some_and(ParallelControl::is_cancelled) {
            return Err(TranscriptError {
                message: format!(
                    "{} transcript worker cancelled after a peer failure",
                    source.alias
                ),
            });
        }
        record_index = record_index.saturating_add(1);
        if line.oversized {
            record_malformed(
                diagnostics,
                source,
                "W_TRANSCRIPT_LINE_OVERSIZED",
                "An oversized transcript line was drained without buffering and excluded.",
            );
            continue;
        }
        if line.bytes.iter().all(u8::is_ascii_whitespace) {
            increment_skipped(diagnostics, source);
            continue;
        }
        let value = match serde_json::from_slice::<Value>(&line.bytes) {
            Ok(value) => value,
            Err(_) => {
                record_malformed(
                    diagnostics,
                    source,
                    "W_TRANSCRIPT_MALFORMED_JSON",
                    "Malformed transcript JSON was excluded; later lines were still scanned.",
                );
                continue;
            }
        };
        let observation_key = hasher.hash(&(
            source.alias.as_str(),
            file_alias,
            line.byte_offset,
            &line.bytes,
        ));
        if let Some(event) = normalize_record(
            source,
            file_alias,
            record_index,
            observation_key,
            line.bytes.len(),
            context,
            value,
            options,
            diagnostics,
            hasher,
            aliases,
            private_prompts,
            private_content_bytes,
        ) {
            let within_limit = parallel.map_or_else(
                || events.len() < options.maximum_events,
                ParallelControl::reserve_event,
            );
            if !within_limit {
                if let Some(parallel) = parallel {
                    parallel.cancel();
                }
                return Err(normalized_event_limit_error(source));
            }
            events.push(event);
        }
    }

    let digesting_file = lines.into_inner().into_inner();
    let (file, content_digest, prefix_digest) = digesting_file.finish();
    let after = file
        .metadata()
        .map_err(|error| TranscriptError::source(source, "final metadata read", error))?;
    let path_after = fs::metadata(&resolved)
        .map_err(|error| TranscriptError::source(source, "final path metadata read", error))?;
    let after_snapshot = FileSnapshot::capture_file(&after, &file)
        .map_err(|error| TranscriptError::source(source, "final file identity read", error))?;
    let path_matches = before_snapshot
        .matches_path(&path_after, &resolved)
        .map_err(|error| TranscriptError::source(source, "final path identity read", error))?;
    if before_snapshot != after_snapshot || !path_matches {
        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
            stats.partial = true;
        }
        return Err(TranscriptError {
            message: format!(
                "{file_alias} changed while it was being streamed; rerun against a stable snapshot"
            ),
        });
    }
    Ok((content_digest.unwrap_or([0; 32]), prefix_digest))
}

#[allow(clippy::too_many_arguments)]
fn normalize_record(
    source: &Source,
    file_alias: &str,
    record_index: u64,
    observation_key: u64,
    byte_count: usize,
    context: &FileContext,
    value: Value,
    options: &TranscriptOptions,
    diagnostics: &mut Diagnostics,
    hasher: &PrivacyHasher,
    aliases: &mut AliasRegistry,
    private_prompts: &mut Vec<PrivatePrompt>,
    private_content_bytes: &mut usize,
) -> Option<NormalizedEvent> {
    let object = match value {
        Value::Object(object) => object,
        _ => {
            record_malformed(
                diagnostics,
                source,
                "W_TRANSCRIPT_NON_OBJECT",
                "A transcript line was valid JSON but not an object and was excluded.",
            );
            return None;
        }
    };

    let record_type = match object.get("type").and_then(Value::as_str) {
        Some(value) => value,
        None => {
            record_redactions(
                diagnostics,
                source,
                object.values().fold(0usize, |count, value| {
                    count.saturating_add(dropped_field_count(value))
                }),
            );
            record_malformed(
                diagnostics,
                source,
                "W_TRANSCRIPT_TYPE_MISSING",
                "A transcript object had no string record type and was excluded.",
            );
            return None;
        }
    };
    let kind = match record_type {
        "assistant" => EventKind::AssistantUsage,
        "user" => {
            let content = object
                .get("message")
                .and_then(Value::as_object)
                .and_then(|message| message.get("content"));
            if is_tool_result_content(content) {
                EventKind::ToolResult
            } else {
                EventKind::UserPrompt
            }
        }
        "progress" => EventKind::Progress,
        "summary" => EventKind::Summary,
        "system" => EventKind::System,
        _ => {
            let redactions = object
                .iter()
                .filter(|(key, _)| key.as_str() != "type")
                .fold(0usize, |count, (_, value)| {
                    count.saturating_add(dropped_field_count(value))
                });
            record_redactions(diagnostics, source, redactions);
            record_unknown_shape(
                diagnostics,
                source,
                file_alias,
                record_index,
                record_type,
                &object,
                byte_count,
            );
            record_unsupported(
                diagnostics,
                source,
                "W_TRANSCRIPT_UNSUPPORTED_VARIANT",
                "An unknown transcript record variant was excluded; no source value was retained.",
            );
            return None;
        }
    };
    let message = object.get("message").and_then(Value::as_object);
    let mut redactions = privacy_redaction_count(&object, message).saturating_add(2);

    let timestamp = match object.get("timestamp").and_then(Value::as_str) {
        Some(timestamp) => timestamp,
        None => {
            record_redactions(diagnostics, source, redactions);
            record_skipped(
                diagnostics,
                source,
                "W_TRANSCRIPT_TIMESTAMP_MISSING",
                "A supported transcript record had no timestamp and was excluded.",
            );
            return None;
        }
    };
    let parsed = match ccwrapped::parse_timestamp(timestamp) {
        Some(parsed) => parsed,
        None => {
            record_redactions(diagnostics, source, redactions);
            record_malformed(
                diagnostics,
                source,
                "W_TRANSCRIPT_TIMESTAMP_INVALID",
                "A supported transcript record had an invalid timestamp and was excluded.",
            );
            return None;
        }
    };
    if !options.time_context.contains_fixed(parsed) {
        record_redactions(diagnostics, source, redactions);
        diagnostics.filtered_records = diagnostics.filtered_records.saturating_add(1);
        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
            stats.filtered_records = stats.filtered_records.saturating_add(1);
        }
        return None;
    }
    let timestamp = parsed
        .with_timezone(&Utc)
        .to_rfc3339_opts(SecondsFormat::AutoSi, true);

    let session_raw = object
        .get("sessionId")
        .and_then(Value::as_str)
        .map(OsStr::new)
        .unwrap_or(context.session_raw.as_os_str());
    let project_key = project_component_key(hasher, context.project_raw.as_os_str());
    let session_key = session_component_key(hasher, session_raw, context.is_subagent);
    let parent_key = context
        .parent_session_raw
        .as_ref()
        .map(|parent| session_component_key(hasher, parent.as_os_str(), false));
    let unknown_fields = transcript_unknown_field_count(&object, message);
    if unknown_fields > 0 {
        diagnostics.unknown_fields = diagnostics.unknown_fields.saturating_add(unknown_fields);
        if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
            stats.unknown_fields = stats.unknown_fields.saturating_add(unknown_fields);
        }
        warn_once(
            diagnostics,
            "W_TRANSCRIPT_UNKNOWN_FIELDS",
            "Unknown transcript fields were ignored and counted without retaining their names or values.",
            Some(source.alias.clone()),
        );
    }
    let message_key = message
        .and_then(|message| message.get("id"))
        .and_then(Value::as_str)
        .or_else(|| object.get("uuid").and_then(Value::as_str))
        .map(|id| hasher.hash(&("message", id)));
    let request_key = find_string(&object, message, &["requestId", "request_id"])
        .map(|id| hasher.hash(&("request", id)));
    let is_sidechain = object
        .get("isSidechain")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    let assistant_message_and_usage = if kind == EventKind::AssistantUsage {
        let message = match message {
            Some(message) => message,
            None => {
                record_redactions(diagnostics, source, redactions);
                record_skipped(
                    diagnostics,
                    source,
                    "W_TRANSCRIPT_ASSISTANT_MESSAGE_MISSING",
                    "An assistant record had no message object and was excluded.",
                );
                return None;
            }
        };
        let usage = match message.get("usage").and_then(Value::as_object) {
            Some(usage) => usage,
            None => {
                record_redactions(diagnostics, source, redactions);
                record_skipped(
                    diagnostics,
                    source,
                    "W_TRANSCRIPT_USAGE_MISSING",
                    "An assistant record had no usage object and was excluded.",
                );
                return None;
            }
        };
        Some((message, usage))
    } else {
        None
    };

    let project_alias = aliases.project(project_key);
    let session_alias = aliases.session(session_key);
    let parent_session_alias = parent_key.map(|key| aliases.session(key));

    let mut model = None;
    let mut tokens = TokenFacts::default();
    let mut source_cost_estimate = None;
    let mut tool_names = Vec::new();

    if kind == EventKind::AssistantUsage {
        let (message, usage) = assistant_message_and_usage
            .expect("assistant message and usage were validated before alias allocation");
        let invalid_usage_fields = [
            "input_tokens",
            "output_tokens",
            "cache_creation_input_tokens",
            "cache_read_input_tokens",
        ]
        .into_iter()
        .filter(|key| usage.get(*key).is_some() && optional_u64(usage, key).is_none())
        .count();
        if invalid_usage_fields > 0 {
            redactions = redactions.saturating_add(invalid_usage_fields);
            mark_partial_warning(
                diagnostics,
                source,
                "W_TRANSCRIPT_USAGE_FIELD_INVALID",
                "One or more transcript usage fields had invalid values and were excluded.",
            );
        }
        tokens = TokenFacts {
            input: optional_u64(usage, "input_tokens"),
            output: optional_u64(usage, "output_tokens"),
            cache_creation: optional_u64(usage, "cache_creation_input_tokens"),
            cache_read: optional_u64(usage, "cache_read_input_tokens"),
            cache_creation_5m: usage
                .get("cache_creation")
                .and_then(Value::as_object)
                .and_then(|cache| optional_u64(cache, "ephemeral_5m_input_tokens")),
            cache_creation_1h: usage
                .get("cache_creation")
                .and_then(Value::as_object)
                .and_then(|cache| optional_u64(cache, "ephemeral_1h_input_tokens")),
        };
        model = message
            .get("model")
            .and_then(Value::as_str)
            .and_then(safe_model_name);
        if message.get("model").is_some() && model.is_none() {
            redactions = redactions.saturating_add(1);
        }
        source_cost_estimate = object
            .get("costUSD")
            .and_then(Value::as_f64)
            .and_then(safe_source_cost);
        if object.get("costUSD").is_some() && source_cost_estimate.is_none() {
            redactions = redactions.saturating_add(1);
            mark_partial_warning(
                diagnostics,
                source,
                "W_TRANSCRIPT_COST_INVALID",
                "A source cost estimate was invalid or outside the bounded supported range and was excluded.",
            );
        }
        tool_names = extract_tool_names(message.get("content"), &mut redactions);
    } else if kind == EventKind::UserPrompt && options.include_private_content {
        if let Some(text) = message
            .and_then(|message| message.get("content"))
            .and_then(extract_user_text)
        {
            let entrypoint = object
                .get("entrypoint")
                .and_then(Value::as_str)
                .map(|value| bounded_private_text(value, PRIVATE_ENTRYPOINT_LIMIT));
            let record_bytes = text
                .len()
                .checked_add(entrypoint.as_ref().map_or(0, String::len));
            let next_bytes =
                record_bytes.and_then(|bytes| private_content_bytes.checked_add(bytes));
            if private_prompts.len() >= MAX_PRIVATE_PROMPTS
                || next_bytes.is_none_or(|bytes| bytes > MAX_PRIVATE_CONTENT_BYTES)
            {
                mark_partial_warning(
                    diagnostics,
                    source,
                    "W_PRIVATE_CONTENT_LIMIT",
                    "Private archive content reached its bounded memory limit; standard analytics remain available.",
                );
                redactions = redactions.saturating_add(1);
            } else {
                *private_content_bytes = next_bytes.unwrap_or(*private_content_bytes);
                private_prompts.push(PrivatePrompt {
                    project_alias: project_alias.clone(),
                    session_alias: session_alias.clone(),
                    timestamp: timestamp.clone(),
                    text,
                    entrypoint,
                });
            }
        }
    }

    record_redactions(diagnostics, source, redactions);
    let epoch_nanos = (parsed.timestamp() as i128)
        .saturating_mul(1_000_000_000)
        .saturating_add(parsed.timestamp_subsec_nanos() as i128);
    diagnostics.observe_time(epoch_nanos, &timestamp);
    if source_cost_estimate.is_some() {
        diagnostics.saw_source_cost = true;
    }
    Some(NormalizedEvent {
        schema_version: NORMALIZED_SCHEMA,
        adapter_version: TRANSCRIPT_ADAPTER,
        source_alias: source.alias.clone(),
        file_alias: file_alias.to_string(),
        record_index,
        timestamp,
        epoch_nanos,
        timestamp_conversion_status: "normalized-utc",
        project_key,
        project_identity_present: true,
        session_key,
        session_identity_present: true,
        message_key,
        request_key,
        parent_key,
        agent_key: find_string(&object, message, &["agentId", "agent_id"])
            .map(|id| hasher.hash(&("agent", id))),
        parent_agent_key: find_string(&object, message, &["parentAgentId", "parent_agent_id"])
            .map(|id| hasher.hash(&("agent", id))),
        skill_key: None,
        plugin_key: None,
        mcp_server_key: None,
        mcp_tool_key: None,
        observation_key,
        project_alias,
        session_alias,
        parent_session_alias,
        is_subagent: context.is_subagent,
        is_sidechain,
        kind,
        model_mapping_status: if model.is_some() {
            "unmapped"
        } else {
            "missing"
        },
        model,
        pricing_modifier: "standard".to_string(),
        tokens,
        source_cost_estimate,
        tool_names,
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
        redacted_fields: redactions,
    })
}

#[cfg(test)]
fn discover_files(
    source: &Source,
    diagnostics: &mut Diagnostics,
) -> Result<TranscriptDiscovery, TranscriptError> {
    discover_files_with_budget(source, diagnostics, &mut TraversalBudget::default())
}

fn discover_files_with_budget(
    source: &Source,
    diagnostics: &mut Diagnostics,
    traversal_budget: &mut TraversalBudget,
) -> Result<TranscriptDiscovery, TranscriptError> {
    discover_files_with_observer(source, diagnostics, traversal_budget, &mut |_| {})
}

fn discover_files_with_observer(
    source: &Source,
    diagnostics: &mut Diagnostics,
    traversal_budget: &mut TraversalBudget,
    observer: &mut impl FnMut(&Path),
) -> Result<TranscriptDiscovery, TranscriptError> {
    let root = fs::canonicalize(&source.path)
        .map_err(|error| TranscriptError::source(source, "root canonicalization", error))?;
    if root != source.path {
        return Err(TranscriptError {
            message: format!(
                "{} changed identity after discovery; rerun against a stable snapshot",
                source.alias
            ),
        });
    }
    let root_metadata = fs::metadata(&root)
        .map_err(|error| TranscriptError::source(source, "root metadata read", error))?;
    let root_matches = source
        .discovery_snapshot
        .matches_path(&root_metadata, &root)
        .map_err(|error| TranscriptError::source(source, "root identity read", error))?;
    if !root_matches {
        return Err(TranscriptError {
            message: format!(
                "{} changed identity between discovery and traversal; rerun against a stable snapshot",
                source.alias
            ),
        });
    }
    let mut discovery = FileDiscovery::default();
    visit_directory(
        source,
        &root,
        &root,
        diagnostics,
        &mut discovery,
        0,
        traversal_budget,
        observer,
    )?;
    discovery
        .files
        .sort_by(|left, right| left.path.cmp(&right.path));
    discovery
        .directories
        .sort_by(|left, right| left.path.cmp(&right.path));
    let root_after = fs::metadata(&root)
        .map_err(|error| TranscriptError::source(source, "final root metadata read", error))?;
    let root_matches = source
        .discovery_snapshot
        .matches_path(&root_after, &root)
        .map_err(|error| TranscriptError::source(source, "final root identity read", error))?;
    if !root_matches {
        return Err(TranscriptError {
            message: format!(
                "{} changed while transcript files were discovered; rerun against a stable snapshot",
                source.alias
            ),
        });
    }
    validate_discovered_directories(source, &discovery.directories, "traversal")?;
    Ok(TranscriptDiscovery {
        files: discovery.files,
        directories: discovery.directories,
    })
}

#[derive(Debug)]
struct TranscriptDiscovery {
    files: Vec<DiscoveredFile>,
    directories: Vec<DiscoveredDirectory>,
}

fn validate_discovered_directories(
    source: &Source,
    directories: &[DiscoveredDirectory],
    activity: &str,
) -> Result<(), TranscriptError> {
    for directory in directories {
        let metadata = fs::metadata(&directory.path).map_err(|_| TranscriptError {
            message: format!(
                "{} changed during {activity}; rerun against a stable snapshot",
                source.alias
            ),
        })?;
        let identity_matches = directory
            .snapshot
            .matches_path(&metadata, &directory.path)
            .map_err(|error| {
                TranscriptError::source(source, "validation directory identity read", error)
            })?;
        if !metadata.is_dir() || !identity_matches {
            return Err(TranscriptError {
                message: format!(
                    "{} changed during {activity}; rerun against a stable snapshot",
                    source.alias
                ),
            });
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn visit_directory(
    source: &Source,
    root: &Path,
    directory: &Path,
    diagnostics: &mut Diagnostics,
    discovery: &mut FileDiscovery,
    depth: usize,
    traversal_budget: &mut TraversalBudget,
    observer: &mut impl FnMut(&Path),
) -> Result<(), TranscriptError> {
    if depth > MAX_DIRECTORY_DEPTH {
        mark_partial_warning(
            diagnostics,
            source,
            "W_TRANSCRIPT_DIRECTORY_DEPTH_LIMIT",
            "A transcript directory branch exceeded the traversal-depth limit and was excluded.",
        );
        return Ok(());
    }
    let canonical = match fs::canonicalize(directory) {
        Ok(canonical) => canonical,
        Err(_) if depth > 0 => {
            mark_inaccessible_subtree(diagnostics, source);
            return Ok(());
        }
        Err(error) => {
            return Err(TranscriptError::source(
                source,
                "directory canonicalization",
                error,
            ))
        }
    };
    if !canonical.starts_with(root) {
        mark_partial_warning(
            diagnostics,
            source,
            "W_TRANSCRIPT_SYMLINK_ESCAPE",
            "A symlink resolving outside the selected transcript root was excluded.",
        );
        return Ok(());
    }
    if !discovery.seen_dirs.contains(&canonical)
        && discovery.seen_dirs.len() >= MAX_TRANSCRIPT_FILES
    {
        return Err(TranscriptError {
            message: format!(
                "{} exceeded the transcript-directory safety limit; narrow the selected source",
                source.alias
            ),
        });
    }
    if !discovery.seen_dirs.insert(canonical.clone()) {
        return Ok(());
    }
    let directory_metadata = match fs::metadata(&canonical) {
        Ok(metadata) => metadata,
        Err(_) if depth > 0 => {
            mark_inaccessible_subtree(diagnostics, source);
            return Ok(());
        }
        Err(error) => {
            return Err(TranscriptError::source(
                source,
                "directory metadata read",
                error,
            ))
        }
    };
    if !directory_metadata.is_dir() {
        return Err(TranscriptError {
            message: format!(
                "{} contained a directory that changed during discovery; rerun against a stable snapshot",
                source.alias
            ),
        });
    }
    discovery.directories.push(DiscoveredDirectory {
        path: canonical.clone(),
        snapshot: FileSnapshot::capture_path(&directory_metadata, &canonical)
            .map_err(|error| TranscriptError::source(source, "directory identity read", error))?,
    });

    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(_) if depth > 0 => {
            mark_inaccessible_subtree(diagnostics, source);
            return Ok(());
        }
        Err(error) => return Err(TranscriptError::source(source, "directory read", error)),
    };
    let mut paths = Vec::new();
    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) if depth > 0 => {
                mark_inaccessible_subtree(diagnostics, source);
                break;
            }
            Err(error) => {
                return Err(TranscriptError::source(
                    source,
                    "directory entry read",
                    error,
                ))
            }
        };
        traversal_budget.consume_entry(source)?;
        paths.push(entry.path());
        if paths.len() > MAX_TRANSCRIPT_FILES {
            return Err(TranscriptError {
                message: format!(
                    "{} exceeded the bounded directory-entry limit; narrow the selected source",
                    source.alias
                ),
            });
        }
    }
    paths.sort();

    for path in paths {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) if depth > 0 => {
                mark_inaccessible_subtree(diagnostics, source);
                continue;
            }
            Err(error) => {
                return Err(TranscriptError::source(
                    source,
                    "entry metadata read",
                    error,
                ))
            }
        };
        if metadata.is_dir() {
            visit_directory(
                source,
                root,
                &path,
                diagnostics,
                discovery,
                depth.saturating_add(1),
                traversal_budget,
                observer,
            )?;
        } else if metadata.is_file()
            && path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
        {
            let canonical_file = match fs::canonicalize(&path) {
                Ok(canonical_file) => canonical_file,
                Err(_) if depth > 0 => {
                    mark_inaccessible_subtree(diagnostics, source);
                    continue;
                }
                Err(error) => {
                    return Err(TranscriptError::source(
                        source,
                        "file canonicalization",
                        error,
                    ))
                }
            };
            if !canonical_file.starts_with(root) {
                mark_partial_warning(
                    diagnostics,
                    source,
                    "W_TRANSCRIPT_SYMLINK_ESCAPE",
                    "A transcript file symlink resolving outside the selected root was excluded.",
                );
            } else {
                let metadata = match fs::metadata(&canonical_file) {
                    Ok(metadata) => metadata,
                    Err(_) if depth > 0 => {
                        mark_inaccessible_subtree(diagnostics, source);
                        continue;
                    }
                    Err(error) => {
                        return Err(TranscriptError::source(
                            source,
                            "discovered file metadata read",
                            error,
                        ))
                    }
                };
                if !metadata.is_file() {
                    return Err(TranscriptError {
                        message: format!(
                            "{} contained an entry that changed during discovery; rerun against a stable snapshot",
                            source.alias
                        ),
                    });
                }
                let snapshot =
                    FileSnapshot::capture_path(&metadata, &canonical_file).map_err(|error| {
                        TranscriptError::source(source, "discovered file identity read", error)
                    })?;
                let dedup_key = snapshot.identity().map_or_else(
                    || FileDedupKey::CanonicalPath(canonical_file.clone()),
                    FileDedupKey::FileSystem,
                );
                if discovery.seen_files.insert(dedup_key) {
                    if discovery.files.len() >= MAX_TRANSCRIPT_FILES {
                        return Err(TranscriptError {
                            message: format!(
                                "{} exceeded the transcript-file safety limit; narrow the selected source",
                                source.alias
                            ),
                        });
                    }
                    discovery.files.push(DiscoveredFile {
                        path: canonical_file,
                        snapshot,
                    });
                    continue;
                }
                warn_once(
                    diagnostics,
                    "W_TRANSCRIPT_DUPLICATE_FILE",
                    "A duplicate filesystem transcript file was discovered and scanned once.",
                    Some(source.alias.clone()),
                );
            }
        }
    }
    observer(&canonical);
    Ok(())
}

fn mark_inaccessible_subtree(diagnostics: &mut Diagnostics, source: &Source) {
    mark_partial_warning(
        diagnostics,
        source,
        "W_TRANSCRIPT_SUBTREE_INACCESSIBLE",
        "A transcript directory branch could not be read. Stored reports retain last-known rows for that branch until a complete scan confirms changes.",
    );
}

fn file_context(root: &Path, file: &Path) -> Option<FileContext> {
    let relative = file.strip_prefix(root).ok()?;
    let parts = relative.iter().collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }
    let subagents_index = parts
        .iter()
        .position(|part| *part == OsStr::new("subagents"));
    let parent_session_raw = subagents_index
        .and_then(|index| index.checked_sub(1))
        .and_then(|index| parts.get(index))
        .map(|part| part.to_os_string());
    Some(FileContext {
        project_raw: parts.first()?.to_os_string(),
        session_raw: file.file_stem()?.to_os_string(),
        parent_session_raw,
        is_subagent: subagents_index.is_some(),
    })
}

#[cfg(unix)]
fn project_component_key(hasher: &PrivacyHasher, component: &OsStr) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    component.to_str().map_or_else(
        || hasher.hash(&("project-native-bytes", component.as_bytes())),
        |component| hasher.hash(&("project", component)),
    )
}

#[cfg(windows)]
fn project_component_key(hasher: &PrivacyHasher, component: &OsStr) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    component.to_str().map_or_else(
        || {
            let units = component.encode_wide().collect::<Vec<_>>();
            hasher.hash(&("project-native-wide", units))
        },
        |component| hasher.hash(&("project", component)),
    )
}

#[cfg(not(any(unix, windows)))]
fn project_component_key(hasher: &PrivacyHasher, component: &OsStr) -> u64 {
    hasher.hash(&("project", component))
}

#[cfg(unix)]
fn session_component_key(hasher: &PrivacyHasher, component: &OsStr, is_subagent: bool) -> u64 {
    use std::os::unix::ffi::OsStrExt;
    component.to_str().map_or_else(
        || hasher.hash(&("session-native-bytes", component.as_bytes(), is_subagent)),
        |component| hasher.hash(&("session", component, is_subagent)),
    )
}

#[cfg(windows)]
fn session_component_key(hasher: &PrivacyHasher, component: &OsStr, is_subagent: bool) -> u64 {
    use std::os::windows::ffi::OsStrExt;
    component.to_str().map_or_else(
        || {
            let units = component.encode_wide().collect::<Vec<_>>();
            hasher.hash(&("session-native-wide", units, is_subagent))
        },
        |component| hasher.hash(&("session", component, is_subagent)),
    )
}

#[cfg(not(any(unix, windows)))]
fn session_component_key(hasher: &PrivacyHasher, component: &OsStr, is_subagent: bool) -> u64 {
    hasher.hash(&("session", component, is_subagent))
}

fn privacy_redaction_count(
    object: &Map<String, Value>,
    message: Option<&Map<String, Value>>,
) -> usize {
    let mut count = 0usize;
    for key in [
        "sessionId",
        "cwd",
        "entrypoint",
        "uuid",
        "parentUuid",
        "requestId",
        "agentId",
    ] {
        count = count.saturating_add(usize::from(object.get(key).is_some()));
    }
    for (key, value) in object {
        if ![
            "type",
            "timestamp",
            "isSidechain",
            "message",
            "costUSD",
            "sessionId",
            "cwd",
            "entrypoint",
            "uuid",
            "parentUuid",
            "requestId",
            "agentId",
            "userType",
        ]
        .contains(&key.as_str())
        {
            count = count.saturating_add(dropped_field_count(value));
        }
    }
    if let Some(message) = message {
        count = count.saturating_add(usize::from(message.get("id").is_some()));
        if let Some(content) = message.get("content") {
            count = count.saturating_add(content_redaction_count(content));
        }
        for (key, value) in message {
            if !["id", "model", "usage", "content", "requestId"].contains(&key.as_str()) {
                count = count.saturating_add(dropped_field_count(value));
            }
        }
        if let Some(usage) = message.get("usage").and_then(Value::as_object) {
            for (key, value) in usage {
                if ![
                    "input_tokens",
                    "output_tokens",
                    "cache_creation_input_tokens",
                    "cache_read_input_tokens",
                    "cache_creation",
                ]
                .contains(&key.as_str())
                {
                    count = count.saturating_add(dropped_field_count(value));
                }
            }
            if let Some(cache) = usage.get("cache_creation").and_then(Value::as_object) {
                for (key, value) in cache {
                    if !["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"]
                        .contains(&key.as_str())
                    {
                        count = count.saturating_add(dropped_field_count(value));
                    }
                }
            }
        }
    }
    count
}

fn content_redaction_count(content: &Value) -> usize {
    let mut count = 1usize;
    let Value::Array(items) = content else {
        return count;
    };
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        for (key, value) in object {
            if key == "type" || key == "name" {
                continue;
            }
            count = count.saturating_add(dropped_field_count(value));
        }
    }
    count
}

fn dropped_field_count(value: &Value) -> usize {
    let nested = match value {
        Value::Object(object) => object.values().fold(0usize, |count, value| {
            count.saturating_add(dropped_field_count(value))
        }),
        Value::Array(items) => items.iter().fold(0usize, |count, value| match value {
            Value::Object(_) | Value::Array(_) => count.saturating_add(dropped_field_count(value)),
            _ => count,
        }),
        _ => 0,
    };
    1usize.saturating_add(nested)
}

fn record_redactions(diagnostics: &mut Diagnostics, source: &Source, count: usize) {
    diagnostics.redacted_fields = diagnostics.redacted_fields.saturating_add(count);
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.redacted_fields = stats.redacted_fields.saturating_add(count);
    }
}

fn transcript_unknown_field_count(
    object: &Map<String, Value>,
    message: Option<&Map<String, Value>>,
) -> usize {
    let mut count = object
        .keys()
        .filter(|key| {
            ![
                "type",
                "timestamp",
                "isSidechain",
                "message",
                "costUSD",
                "sessionId",
                "cwd",
                "entrypoint",
                "uuid",
                "parentUuid",
                "requestId",
                "request_id",
                "agentId",
                "agent_id",
                "parentAgentId",
                "parent_agent_id",
                "userType",
            ]
            .contains(&key.as_str())
        })
        .count();
    if let Some(message) = message {
        count = count.saturating_add(
            message
                .keys()
                .filter(|key| {
                    !["id", "model", "usage", "content", "requestId", "request_id"]
                        .contains(&key.as_str())
                })
                .count(),
        );
        if let Some(usage) = message.get("usage").and_then(Value::as_object) {
            count = count.saturating_add(
                usage
                    .keys()
                    .filter(|key| {
                        ![
                            "input_tokens",
                            "output_tokens",
                            "cache_creation_input_tokens",
                            "cache_read_input_tokens",
                            "cache_creation",
                        ]
                        .contains(&key.as_str())
                    })
                    .count(),
            );
            if let Some(cache) = usage.get("cache_creation").and_then(Value::as_object) {
                count = count.saturating_add(
                    cache
                        .keys()
                        .filter(|key| {
                            !["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"]
                                .contains(&key.as_str())
                        })
                        .count(),
                );
            }
        }
    }
    count
}

fn find_string<'a>(
    object: &'a Map<String, Value>,
    message: Option<&'a Map<String, Value>>,
    keys: &[&str],
) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        object
            .get(*key)
            .or_else(|| message.and_then(|message| message.get(*key)))
            .and_then(Value::as_str)
    })
}

fn optional_u64(object: &Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

fn record_unknown_shape(
    diagnostics: &mut Diagnostics,
    source: &Source,
    file_alias: &str,
    record_index: u64,
    _record_type: &str,
    object: &Map<String, Value>,
    byte_count: usize,
) {
    const ALLOWED_KEYS: &[&str] = &[
        "type",
        "timestamp",
        "message",
        "isSidechain",
        "sessionId",
        "uuid",
        "parentUuid",
        "requestId",
        "agentId",
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
            adapter_version: TRANSCRIPT_ADAPTER.to_string(),
            file_alias: file_alias.to_string(),
            record_index,
            record_kind: "unsupported-transcript-variant".to_string(),
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

fn extract_tool_names(content: Option<&Value>, redactions: &mut usize) -> Vec<String> {
    let Some(Value::Array(items)) = content else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for item in items {
        let Some(object) = item.as_object() else {
            continue;
        };
        if object.get("type").and_then(Value::as_str) != Some("tool_use") {
            continue;
        }
        if let Some(raw) = object.get("name").and_then(Value::as_str) {
            let (name, transformed) = classified_tool_name(raw);
            *redactions = redactions.saturating_add(transformed);
            if let Some(name) = name {
                if names.len() < MAX_TOOL_NAMES {
                    names.push(name);
                } else {
                    *redactions = redactions.saturating_add(1);
                }
            }
        }
    }
    names.sort();
    names.dedup();
    names
}

fn is_tool_result_content(content: Option<&Value>) -> bool {
    match content {
        Some(Value::Array(items)) if !items.is_empty() => items.iter().all(|item| {
            item.as_object()
                .and_then(|object| object.get("type"))
                .and_then(Value::as_str)
                == Some("tool_result")
        }),
        _ => false,
    }
}

fn extract_user_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(items) => items
            .iter()
            .filter_map(|item| {
                if let Some(text) = item.as_str() {
                    return Some(text);
                }
                let object = item.as_object()?;
                if object.get("type").and_then(Value::as_str) == Some("tool_result") {
                    return None;
                }
                object
                    .get("text")
                    .or_else(|| object.get("content"))
                    .and_then(Value::as_str)
            })
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    };
    if text.is_empty() {
        None
    } else {
        Some(bounded_private_text(&text, PRIVATE_PROMPT_LIMIT))
    }
}

fn bounded_private_text(value: &str, maximum: usize) -> String {
    if value.len() <= maximum {
        return value.to_string();
    }
    let mut boundary = maximum;
    while !value.is_char_boundary(boundary) {
        boundary = boundary.saturating_sub(1);
    }
    value[..boundary].to_string()
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

fn record_unsupported(diagnostics: &mut Diagnostics, source: &Source, code: &str, message: &str) {
    diagnostics.unsupported_records = diagnostics.unsupported_records.saturating_add(1);
    diagnostics.analytical_claims_uncertain = true;
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.unsupported_records = stats.unsupported_records.saturating_add(1);
        stats.partial = true;
    }
    warn_once(diagnostics, code, message, Some(source.alias.clone()));
}

fn record_skipped(diagnostics: &mut Diagnostics, source: &Source, code: &str, message: &str) {
    increment_skipped(diagnostics, source);
    diagnostics.analytical_claims_uncertain = true;
    warn_once(diagnostics, code, message, Some(source.alias.clone()));
}

fn increment_skipped(diagnostics: &mut Diagnostics, source: &Source) {
    diagnostics.skipped_records = diagnostics.skipped_records.saturating_add(1);
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.skipped_records = stats.skipped_records.saturating_add(1);
        stats.partial = true;
    }
}

fn mark_partial_warning(diagnostics: &mut Diagnostics, source: &Source, code: &str, message: &str) {
    if let Some(stats) = diagnostics.sources.get_mut(&source.alias) {
        stats.partial = true;
    }
    warn_once(diagnostics, code, message, Some(source.alias.clone()));
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
        discover_files, discover_files_with_budget, discover_files_with_observer, file_context,
        ingest_file, TranscriptOptions, TraversalBudget,
    };
    use crate::ingestion::discovery::{self, DiscoveryOptions};
    use crate::ingestion::types::{AliasRegistry, PrivacyHasher};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    #[test]
    fn transcript_discovery_open_replacement_is_rejected() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-transcript-discovery-open-{}-{nonce}",
            std::process::id()
        ));
        let projects = root.join("projects");
        let project = projects.join("project-a");
        fs::create_dir_all(&project).unwrap();
        let path = project.join("session-a.jsonl");
        let replacement = project.join("replacement.tmp");
        fs::write(&path, "\n").unwrap();
        fs::write(&replacement, "\n").unwrap();
        let discovery = discovery::discover(&DiscoveryOptions {
            data_dirs: vec![projects],
            otel_files: Vec::new(),
            claude_config_dir: None,
            home_dir: None,
            private_diagnostics: false,
        })
        .unwrap();
        let source = &discovery.sources[0];
        let mut diagnostics = discovery.diagnostics;
        let files = discover_files(source, &mut diagnostics).unwrap();
        assert_eq!(files.files.len(), 1);
        let discovered = &files.files[0];
        let context = file_context(&source.path, &discovered.path).unwrap();
        fs::rename(&replacement, &discovered.path).unwrap();
        let mut aliases = AliasRegistry::default();
        let mut private_prompts = Vec::new();
        let mut private_content_bytes = 0;
        let mut events = Vec::new();
        let result = ingest_file(
            source,
            discovered,
            "transcript-1-file-1",
            &context,
            &TranscriptOptions {
                time_context: super::super::TimeContext::new("UTC", Some(2026)).unwrap(),
                maximum_line_bytes: 1024,
                maximum_events: 10,
                include_private_content: false,
                worker_count: 1,
                worker_delay_seed: None,
                worker_panic_file: None,
                read_accounting: std::sync::Arc::new(super::super::SourceReadAccounting::default()),
            },
            &mut diagnostics,
            &PrivacyHasher::new(),
            &mut aliases,
            &mut private_prompts,
            &mut private_content_bytes,
            &mut events,
            None,
        );
        fs::remove_dir_all(&root).unwrap();

        let error = result.expect_err("same-path replacement was accepted");
        assert!(error.to_string().contains("between discovery and open"));
        assert!(events.is_empty());
        assert_eq!(diagnostics.accepted_records, 0);
        assert_eq!(diagnostics.earliest, None);
    }

    #[test]
    fn transcript_nested_directory_mutation_is_fatal() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-transcript-nested-mutation-{}-{nonce}",
            std::process::id()
        ));
        let projects = root.join("projects");
        let project = projects.join("project-a");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("session-a.jsonl"), "\n").unwrap();
        let discovery = discovery::discover(&DiscoveryOptions {
            data_dirs: vec![projects],
            otel_files: Vec::new(),
            claude_config_dir: None,
            home_dir: None,
            private_diagnostics: false,
        })
        .unwrap();
        let source = &discovery.sources[0];
        let mut diagnostics = discovery.diagnostics;
        let mut mutated = false;
        let result = discover_files_with_observer(
            source,
            &mut diagnostics,
            &mut TraversalBudget::default(),
            &mut |directory| {
                if directory == project {
                    fs::write(directory.join("session-late.jsonl"), "\n").unwrap();
                    mutated = true;
                }
            },
        );
        fs::remove_dir_all(&root).unwrap();

        assert!(mutated, "the deterministic mutation hook did not run");
        let error = result.expect_err("nested directory mutation was accepted");
        assert!(error.to_string().contains("changed during traversal"));
    }

    #[test]
    fn transcript_directory_mutation_after_discovery_is_fatal() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-transcript-post-discovery-mutation-{}-{nonce}",
            std::process::id()
        ));
        let projects = root.join("projects");
        let project = projects.join("project-a");
        fs::create_dir_all(&project).unwrap();
        fs::write(project.join("session-a.jsonl"), "\n").unwrap();
        let discovery = discovery::discover(&DiscoveryOptions {
            data_dirs: vec![projects],
            otel_files: Vec::new(),
            claude_config_dir: None,
            home_dir: None,
            private_diagnostics: false,
        })
        .unwrap();
        let source = &discovery.sources[0];
        let mut diagnostics = discovery.diagnostics;
        let files = discover_files(source, &mut diagnostics).unwrap();
        fs::write(project.join("session-late.jsonl"), "\n").unwrap();
        let result = super::validate_discovered_directories(
            source,
            &files.directories,
            "transcript ingestion",
        );
        fs::remove_dir_all(&root).unwrap();

        let error = result.expect_err("post-discovery directory mutation was accepted");
        assert!(error.to_string().contains("during transcript ingestion"));
    }

    #[test]
    fn transcript_directory_entries_share_one_invocation_budget() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-transcript-entry-budget-{}-{nonce}",
            std::process::id()
        ));
        let first = root.join("first/projects");
        let second = root.join("second/projects");
        for projects in [&first, &second] {
            let project = projects.join("project-a");
            fs::create_dir_all(&project).unwrap();
            fs::write(project.join("session-a.jsonl"), "\n").unwrap();
        }
        let discovery = discovery::discover(&DiscoveryOptions {
            data_dirs: vec![first, second],
            otel_files: Vec::new(),
            claude_config_dir: None,
            home_dir: None,
            private_diagnostics: false,
        })
        .unwrap();
        let mut diagnostics = discovery.diagnostics;
        let mut budget = TraversalBudget::with_maximum(3);
        discover_files_with_budget(&discovery.sources[0], &mut diagnostics, &mut budget).unwrap();
        let result =
            discover_files_with_budget(&discovery.sources[1], &mut diagnostics, &mut budget);
        fs::remove_dir_all(&root).unwrap();

        let error = result.expect_err("separate roots received separate entry budgets");
        assert!(error.to_string().contains("directory-entry safety limit"));
    }
}
