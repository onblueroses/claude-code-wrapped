use serde_json::{json, Map, Value};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const SEED: &str = "0xc5c5_2026_0717_0001";
pub const GENERATOR_VERSION: &str = "phase5-corpus/2.0.0";
pub const INCREMENTAL_GENERATOR_VERSION: &str = "phase5-incremental-tail/1.2.0";
pub const MAXIMUM_CORPUS_BYTES: u64 = 3_758_096_384;
const ESTIMATED_UNPADDED_LINE_BYTES: usize = 519;
const INCREMENTAL_EXISTING_FILES: usize = 4;
const INCREMENTAL_NEW_FILES: usize = 4;
const INCREMENTAL_RECORDS_PER_FILE: usize = 1;
const DECISION_TRANSCRIPT_FILES: usize = 4_096;
const DECISION_TRANSCRIPT_RECORDS_PER_FILE: usize = 175;
const METRIC_RECORDS_PER_OTEL_FILE: usize = 3;
const METRIC_BASE_NANOS: u64 = 1_772_323_200_000_000_000;
const NANOS_PER_SECOND: u64 = 1_000_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CorpusClass {
    OracleSmall,
    Decision,
    SaturationLarge,
}

impl CorpusClass {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "oracle-small" => Ok(Self::OracleSmall),
            "decision" => Ok(Self::Decision),
            "saturation-large" => Ok(Self::SaturationLarge),
            _ => Err(format!(
                "unknown corpus class `{value}`; use oracle-small, decision, or saturation-large"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::OracleSmall => "oracle-small",
            Self::Decision => "decision",
            Self::SaturationLarge => "saturation-large",
        }
    }

    fn configuration(self) -> Configuration {
        match self {
            Self::OracleSmall => Configuration {
                transcript_files: 32,
                transcript_records_per_file: 400,
                background_records_per_file: 0,
                otel_files: 4,
                otel_records_per_file: 500,
                target_bytes: 12 * 1024 * 1024,
            },
            Self::Decision => Configuration {
                transcript_files: 4_096,
                transcript_records_per_file: 175,
                background_records_per_file: 0,
                otel_files: 16,
                otel_records_per_file: 4_000,
                target_bytes: 512 * 1024 * 1024,
            },
            Self::SaturationLarge => Configuration {
                transcript_files: 16_384,
                transcript_records_per_file: 175,
                background_records_per_file: 500,
                otel_files: 64,
                otel_records_per_file: 1_000,
                target_bytes: 9 * 256 * 1024 * 1024,
            },
        }
    }
}

impl fmt::Display for CorpusClass {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.name())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusSummary {
    pub class: CorpusClass,
    pub seed: &'static str,
    pub transcript_files: usize,
    pub otel_files: usize,
    pub source_bytes: u64,
    pub physical_records: u64,
    pub normalized_candidates: u64,
    pub accepted_records: u64,
    pub canonical_records: u64,
    pub malformed_records: u64,
    pub unsupported_records: u64,
    pub unknown_records: u64,
    pub filtered_records: u64,
    pub duplicate_records: u64,
    pub resolved_overlap_records: u64,
    pub unresolved_overlap_records: u64,
    pub metric_points: u64,
    pub metric_accepted_points: u64,
    pub metric_filtered_points: u64,
    pub metric_delta_points: u64,
    pub metric_cumulative_points: u64,
    pub metric_reset_points: u64,
    pub metric_gap_points: u64,
    pub metric_overlap_points: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
    pub active_time_oracle: Option<ActiveTimeOracle>,
    pub insight_eligibility: Vec<InsightEligibility>,
    pub content_fingerprint: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTimeOracle {
    pub interval_count: u64,
    pub total_elapsed_seconds: u64,
    pub total_active_seconds: u64,
    pub main_exclusive_seconds: u64,
    pub subagent_exclusive_seconds: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsightEligibility {
    pub family: &'static str,
    pub availability: &'static str,
    pub sample_count: u64,
    pub minimum_sample_count: u64,
}

impl CorpusSummary {
    pub fn manifest_json(&self) -> String {
        let classified_records = self
            .accepted_records
            .saturating_add(self.malformed_records)
            .saturating_add(self.unsupported_records)
            .saturating_add(self.filtered_records)
            .saturating_add(self.duplicate_records);
        let active_time_oracle = self.active_time_oracle.as_ref().map(|active| {
            json!({
                "intervalCount": active.interval_count,
                "totalElapsedSeconds": active.total_elapsed_seconds,
                "totalActiveSeconds": active.total_active_seconds,
                "mainExclusiveSeconds": active.main_exclusive_seconds,
                "subagentExclusiveSeconds": active.subagent_exclusive_seconds
            })
        });
        let insight_eligibility = self
            .insight_eligibility
            .iter()
            .map(|insight| {
                json!({
                    "family": insight.family,
                    "availability": insight.availability,
                    "sampleCount": insight.sample_count,
                    "minimumSampleCount": insight.minimum_sample_count
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string_pretty(&json!({
            "schema": "ccwrapped.phase5-corpus/v2",
            "generatorVersion": GENERATOR_VERSION,
            "class": self.class.name(),
            "seed": self.seed,
            "transcriptFiles": self.transcript_files,
            "otelFiles": self.otel_files,
            "sourceBytes": self.source_bytes,
            "physicalRecords": self.physical_records,
            "normalizedCandidates": self.normalized_candidates,
            "distribution": {
                "subagentTranscriptFiles": self.transcript_files.saturating_add(19) / 20,
                "classifiedRecords": classified_records,
                "malformedRecords": self.malformed_records,
                "unsupportedRecords": self.unsupported_records,
                "unknownRecords": self.unknown_records,
                "filteredRecords": self.filtered_records,
                "duplicateRecords": self.duplicate_records,
                "resolvedOverlapRecords": self.resolved_overlap_records,
                "unresolvedOverlapRecords": self.unresolved_overlap_records
            },
            "metricOracle": {
                "points": self.metric_points,
                "acceptedPoints": self.metric_accepted_points,
                "filteredPoints": self.metric_filtered_points,
                "deltaPoints": self.metric_delta_points,
                "cumulativePoints": self.metric_cumulative_points,
                "resetPoints": self.metric_reset_points,
                "gapPoints": self.metric_gap_points,
                "overlapPoints": self.metric_overlap_points
            },
            "activeTimeOracle": active_time_oracle,
            "insightEligibility": insight_eligibility,
            "oracle": {
                "acceptedRecords": self.accepted_records,
                "canonicalRecords": self.canonical_records,
                "malformedRecords": self.malformed_records,
                "unsupportedRecords": self.unsupported_records,
                "unknownRecords": self.unknown_records,
                "filteredRecords": self.filtered_records,
                "duplicateRecords": self.duplicate_records,
                "resolvedOverlapRecords": self.resolved_overlap_records,
                "unresolvedOverlapRecords": self.unresolved_overlap_records,
                "inputTokens": self.input_tokens,
                "outputTokens": self.output_tokens,
                "cacheCreationTokens": self.cache_creation_tokens,
                "cacheReadTokens": self.cache_read_tokens
            },
            "contentFingerprintFnv1a64": format!("{:016x}", self.content_fingerprint)
        }))
        .expect("the bounded corpus manifest must serialize")
            + "\n"
    }
}

#[derive(Debug, Clone, Copy)]
struct Configuration {
    transcript_files: usize,
    transcript_records_per_file: usize,
    background_records_per_file: usize,
    otel_files: usize,
    otel_records_per_file: usize,
    target_bytes: u64,
}

#[derive(Debug, Default)]
struct Oracle {
    physical_records: u64,
    accepted_records: u64,
    canonical_records: u64,
    malformed_records: u64,
    unsupported_records: u64,
    unknown_records: u64,
    filtered_records: u64,
    duplicate_records: u64,
    resolved_overlap_records: u64,
    unresolved_overlap_records: u64,
    metric_points: u64,
    metric_accepted_points: u64,
    metric_filtered_points: u64,
    metric_delta_points: u64,
    metric_cumulative_points: u64,
    metric_reset_points: u64,
    metric_gap_points: u64,
    metric_overlap_points: u64,
    input_tokens: u64,
    output_tokens: u64,
    cache_creation_tokens: u64,
    cache_read_tokens: u64,
}

impl Oracle {
    fn observe_tokens(&mut self, token_index: u64) {
        self.input_tokens = self.input_tokens.saturating_add(1);
        self.output_tokens = self
            .output_tokens
            .saturating_add(output_tokens(token_index));
        self.cache_creation_tokens = self.cache_creation_tokens.saturating_add(2);
        self.cache_read_tokens = self.cache_read_tokens.saturating_add(3);
    }
}

struct FingerprintedWriter {
    writer: BufWriter<File>,
    bytes: u64,
    fingerprint: u64,
}

impl FingerprintedWriter {
    fn create(path: &Path) -> io::Result<Self> {
        Ok(Self {
            writer: BufWriter::with_capacity(256 * 1024, File::create(path)?),
            bytes: 0,
            fingerprint: 0xcbf2_9ce4_8422_2325,
        })
    }

    fn line(&mut self, value: &str) -> io::Result<()> {
        self.writer.write_all(value.as_bytes())?;
        self.writer.write_all(b"\n")?;
        for byte in value
            .as_bytes()
            .iter()
            .copied()
            .chain(std::iter::once(b'\n'))
        {
            self.fingerprint ^= u64::from(byte);
            self.fingerprint = self.fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
        }
        self.bytes = self
            .bytes
            .saturating_add(value.len() as u64)
            .saturating_add(1);
        Ok(())
    }

    fn finish(mut self) -> io::Result<(u64, u64)> {
        self.writer.flush()?;
        Ok((self.bytes, self.fingerprint))
    }
}

pub fn generate(
    class: CorpusClass,
    output: &Path,
    target_bytes: Option<u64>,
) -> Result<CorpusSummary, String> {
    let mut configuration = class.configuration();
    if let Some(target_bytes) = target_bytes {
        configuration.target_bytes = target_bytes;
    }
    validate_requested_bytes(class, configuration.target_bytes)?;
    if configuration.target_bytes > MAXIMUM_CORPUS_BYTES {
        return Err(format!(
            "requested corpus is {} bytes; maximum is {MAXIMUM_CORPUS_BYTES}",
            configuration.target_bytes
        ));
    }
    if output.exists() {
        return Err(format!(
            "output {} already exists; select a new dedicated benchmark path",
            output.display()
        ));
    }

    let projects = output.join("projects");
    let otel = output.join("otel");
    fs::create_dir_all(&projects)
        .and_then(|_| fs::create_dir(&otel))
        .map_err(|error| format!("create {}: {error}", output.display()))?;

    let physical_records = configuration
        .transcript_files
        .saturating_mul(
            configuration
                .transcript_records_per_file
                .saturating_add(configuration.background_records_per_file),
        )
        .saturating_add(
            configuration
                .otel_files
                .saturating_mul(configuration.otel_records_per_file),
        );
    let target_line_bytes = usize::try_from(
        configuration
            .target_bytes
            .checked_div(physical_records as u64)
            .unwrap_or(0),
    )
    .unwrap_or(usize::MAX);
    let padding = "x".repeat(target_line_bytes.saturating_sub(ESTIMATED_UNPADDED_LINE_BYTES));

    let mut oracle = Oracle::default();
    let mut source_bytes = 0u64;
    let mut corpus_fingerprint = 0xcbf2_9ce4_8422_2325;

    for file_index in 0..configuration.transcript_files {
        let project = projects.join(format!("project-{file_index:05}"));
        let (path, session) = if file_index % 20 == 0 {
            let parent = format!("parent-{file_index:05}");
            let directory = project.join(parent).join("subagents");
            fs::create_dir_all(&directory)
                .map_err(|error| format!("create {}: {error}", directory.display()))?;
            (
                directory.join(format!("agent-{file_index:05}.jsonl")),
                format!("agent-{file_index:05}"),
            )
        } else {
            fs::create_dir_all(&project)
                .map_err(|error| format!("create {}: {error}", project.display()))?;
            (
                project.join(format!("session-{file_index:05}.jsonl")),
                format!("session-{file_index:05}"),
            )
        };
        let mut writer = FingerprintedWriter::create(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        for record_index in 0..configuration.transcript_records_per_file {
            let global = file_index
                .saturating_mul(configuration.transcript_records_per_file)
                .saturating_add(record_index) as u64;
            let line = transcript_line(
                file_index,
                record_index,
                global,
                &session,
                &padding,
                &mut oracle,
            );
            writer
                .line(&line)
                .map_err(|error| format!("write {}: {error}", path.display()))?;
        }
        let background_start = configuration
            .transcript_files
            .saturating_mul(configuration.transcript_records_per_file)
            as u64;
        for record_index in 0..configuration.background_records_per_file {
            let global = background_start.saturating_add(
                file_index
                    .saturating_mul(configuration.background_records_per_file)
                    .saturating_add(record_index) as u64,
            );
            let line = background_line(global, record_index, &mut oracle);
            writer
                .line(&line)
                .map_err(|error| format!("write {}: {error}", path.display()))?;
        }
        let (bytes, fingerprint) = writer
            .finish()
            .map_err(|error| format!("finish {}: {error}", path.display()))?;
        source_bytes = source_bytes.saturating_add(bytes);
        corpus_fingerprint = fold_file_fingerprint(corpus_fingerprint, fingerprint, bytes);
    }

    for file_index in 0..configuration.otel_files {
        let path = otel.join(format!("collector-{file_index:03}.jsonl"));
        let mut writer = FingerprintedWriter::create(&path)
            .map_err(|error| format!("create {}: {error}", path.display()))?;
        for record_index in 0..configuration.otel_records_per_file {
            if record_index < METRIC_RECORDS_PER_OTEL_FILE {
                let line = otel_metric_line(file_index, record_index, &padding, &mut oracle);
                writer
                    .line(&line)
                    .map_err(|error| format!("write {}: {error}", path.display()))?;
                continue;
            }
            let overlap = record_index == METRIC_RECORDS_PER_OTEL_FILE;
            let transcript_file =
                (file_index.saturating_mul(20).saturating_add(1)) % configuration.transcript_files;
            let token_index = if overlap {
                transcript_file
                    .saturating_mul(configuration.transcript_records_per_file)
                    .saturating_add(72) as u64
            } else {
                configuration
                    .transcript_files
                    .saturating_mul(configuration.transcript_records_per_file)
                    .saturating_add(
                        file_index
                            .saturating_mul(configuration.otel_records_per_file)
                            .saturating_add(record_index),
                    ) as u64
            };
            let session = format!("session-{transcript_file:05}");
            let request = if overlap {
                format!("request-{transcript_file:05}-00072")
            } else {
                format!("otel-request-{file_index:03}-{record_index:05}")
            };
            let line = otel_request_line(token_index, &session, &request, &padding);
            writer
                .line(&line)
                .map_err(|error| format!("write {}: {error}", path.display()))?;
            oracle.physical_records = oracle.physical_records.saturating_add(1);
            oracle.accepted_records = oracle.accepted_records.saturating_add(1);
            if overlap {
                oracle.resolved_overlap_records = oracle.resolved_overlap_records.saturating_add(1);
            } else {
                oracle.unresolved_overlap_records =
                    oracle.unresolved_overlap_records.saturating_add(1);
            }
        }
        let (bytes, fingerprint) = writer
            .finish()
            .map_err(|error| format!("finish {}: {error}", path.display()))?;
        source_bytes = source_bytes.saturating_add(bytes);
        corpus_fingerprint = fold_file_fingerprint(corpus_fingerprint, fingerprint, bytes);
    }

    let summary = CorpusSummary {
        class,
        seed: SEED,
        transcript_files: configuration.transcript_files,
        otel_files: configuration.otel_files,
        source_bytes,
        physical_records: oracle.physical_records,
        normalized_candidates: oracle
            .accepted_records
            .saturating_add(oracle.duplicate_records),
        accepted_records: oracle.accepted_records,
        canonical_records: oracle.canonical_records,
        malformed_records: oracle.malformed_records,
        unsupported_records: oracle.unsupported_records,
        unknown_records: oracle.unknown_records,
        filtered_records: oracle.filtered_records,
        duplicate_records: oracle.duplicate_records,
        resolved_overlap_records: oracle.resolved_overlap_records,
        unresolved_overlap_records: oracle.unresolved_overlap_records,
        metric_points: oracle.metric_points,
        metric_accepted_points: oracle.metric_accepted_points,
        metric_filtered_points: oracle.metric_filtered_points,
        metric_delta_points: oracle.metric_delta_points,
        metric_cumulative_points: oracle.metric_cumulative_points,
        metric_reset_points: oracle.metric_reset_points,
        metric_gap_points: oracle.metric_gap_points,
        metric_overlap_points: oracle.metric_overlap_points,
        input_tokens: oracle.input_tokens,
        output_tokens: oracle.output_tokens,
        cache_creation_tokens: oracle.cache_creation_tokens,
        cache_read_tokens: oracle.cache_read_tokens,
        active_time_oracle: active_time_oracle(class),
        insight_eligibility: insight_eligibility(class),
        content_fingerprint: corpus_fingerprint,
    };
    validate_summary(&summary)?;
    fs::write(output.join("manifest.json"), summary.manifest_json())
        .map_err(|error| format!("write corpus manifest: {error}"))?;
    Ok(summary)
}

pub fn append_incremental_tail(corpus: &Path, output: &Path) -> Result<String, String> {
    if output.exists() {
        return Err(format!(
            "incremental manifest {} already exists",
            output.display()
        ));
    }
    let corpus = fs::canonicalize(corpus)
        .map_err(|error| format!("resolve incremental corpus {}: {error}", corpus.display()))?;
    let base_manifest_path = corpus.join("manifest.json");
    let base_manifest_bytes = fs::read(&base_manifest_path)
        .map_err(|error| format!("read {}: {error}", base_manifest_path.display()))?;
    let base_manifest: Value = serde_json::from_slice(&base_manifest_bytes)
        .map_err(|error| format!("parse {}: {error}", base_manifest_path.display()))?;
    if base_manifest.get("schema").and_then(Value::as_str) != Some("ccwrapped.phase5-corpus/v2")
        || base_manifest.get("class").and_then(Value::as_str) != Some("decision")
    {
        return Err("incremental-tail requires a decision corpus manifest".to_string());
    }
    let before_source_bytes = source_bytes(&corpus)?;
    if base_manifest.get("sourceBytes").and_then(Value::as_u64) != Some(before_source_bytes) {
        return Err("decision source bytes do not match the base manifest".to_string());
    }

    let mut oracle = Oracle::default();
    let mut planned = Vec::with_capacity(INCREMENTAL_EXISTING_FILES + INCREMENTAL_NEW_FILES);
    for offset in 0..INCREMENTAL_EXISTING_FILES {
        let file_index = offset.saturating_add(1);
        let path = corpus
            .join("projects")
            .join(format!("project-{file_index:05}"))
            .join(format!("session-{file_index:05}.jsonl"));
        if !path.is_file() {
            return Err(format!(
                "incremental existing source {} is absent",
                path.display()
            ));
        }
        let session = format!("session-{file_index:05}");
        let lines = (0..INCREMENTAL_RECORDS_PER_FILE)
            .map(|offset| {
                let record_index = DECISION_TRANSCRIPT_RECORDS_PER_FILE.saturating_add(offset);
                let timestamp_index = file_index
                    .saturating_mul(DECISION_TRANSCRIPT_RECORDS_PER_FILE)
                    .saturating_add(DECISION_TRANSCRIPT_RECORDS_PER_FILE.saturating_sub(1))
                    as u64;
                incremental_line(
                    file_index,
                    record_index,
                    timestamp_index,
                    &session,
                    &mut oracle,
                )
            })
            .collect::<Vec<_>>();
        planned.push((path, true, lines));
    }
    for offset in 0..INCREMENTAL_NEW_FILES {
        let file_index = DECISION_TRANSCRIPT_FILES.saturating_add(offset);
        let identity_index = offset.saturating_add(1);
        let directory = corpus
            .join("projects")
            .join(format!("project-{identity_index:05}"));
        let path = directory.join(format!("incremental-{file_index:05}.jsonl"));
        if path.exists() {
            return Err(format!(
                "incremental new source {} already exists",
                path.display()
            ));
        }
        let session = format!("session-{identity_index:05}");
        let lines = (0..INCREMENTAL_RECORDS_PER_FILE)
            .map(|offset| {
                let record_index = DECISION_TRANSCRIPT_RECORDS_PER_FILE
                    .saturating_add(INCREMENTAL_RECORDS_PER_FILE)
                    .saturating_add(offset);
                let timestamp_index = identity_index
                    .saturating_mul(DECISION_TRANSCRIPT_RECORDS_PER_FILE)
                    .saturating_add(DECISION_TRANSCRIPT_RECORDS_PER_FILE.saturating_sub(1))
                    as u64;
                incremental_line(
                    file_index,
                    record_index,
                    timestamp_index,
                    &session,
                    &mut oracle,
                )
            })
            .collect::<Vec<_>>();
        planned.push((path, false, lines));
    }

    let tail_source_bytes = planned.iter().try_fold(0u64, |total, (_, _, lines)| {
        lines.iter().try_fold(total, |total, line| {
            total
                .checked_add(line.len() as u64)
                .and_then(|value| value.checked_add(1))
                .ok_or_else(|| "incremental source byte count overflowed".to_string())
        })
    })?;
    let maximum_tail_bytes = before_source_bytes / 100;
    if tail_source_bytes > maximum_tail_bytes || oracle.physical_records > 5_000 {
        return Err(format!(
            "incremental tail exceeds its frozen bounds: bytes={tail_source_bytes}/{maximum_tail_bytes}, records={}",
            oracle.physical_records
        ));
    }

    for (path, append, lines) in &planned {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .map_err(|error| format!("create {}: {error}", parent.display()))?;
        }
        let file = if *append {
            OpenOptions::new().append(true).open(path)
        } else {
            OpenOptions::new().write(true).create_new(true).open(path)
        }
        .map_err(|error| format!("open incremental source {}: {error}", path.display()))?;
        let mut writer = BufWriter::with_capacity(256 * 1024, file);
        for line in lines {
            writer
                .write_all(line.as_bytes())
                .and_then(|()| writer.write_all(b"\n"))
                .map_err(|error| format!("write incremental source {}: {error}", path.display()))?;
        }
        writer
            .flush()
            .map_err(|error| format!("flush incremental source {}: {error}", path.display()))?;
    }

    let after_source_bytes = source_bytes(&corpus)?;
    if after_source_bytes != before_source_bytes.saturating_add(tail_source_bytes) {
        return Err("incremental source byte accounting did not reconcile".to_string());
    }
    let before_oracle = base_manifest
        .get("oracle")
        .and_then(Value::as_object)
        .ok_or_else(|| "base manifest has no oracle object".to_string())?;
    let delta_oracle = oracle_json(&oracle);
    let after_oracle = add_oracles(before_oracle, &delta_oracle)?;
    let summary = json!({
        "schema": "ccwrapped.phase5-incremental-tail/v1",
        "generatorVersion": INCREMENTAL_GENERATOR_VERSION,
        "seed": SEED,
        "baseManifestBlake3": blake3::hash(&base_manifest_bytes).to_hex().to_string(),
        "changedExistingFiles": INCREMENTAL_EXISTING_FILES,
        "newFiles": INCREMENTAL_NEW_FILES,
        "changedFileAliases": [
            "changed-existing-1",
            "changed-existing-2",
            "changed-existing-3",
            "changed-existing-4",
            "new-source-1",
            "new-source-2",
            "new-source-3",
            "new-source-4"
        ],
        "appendedRecords": oracle.physical_records,
        "tailSourceBytes": tail_source_bytes,
        "beforeSourceBytes": before_source_bytes,
        "afterSourceBytes": after_source_bytes,
        "maximumTailBytes": maximum_tail_bytes,
        "beforeOracle": Value::Object(before_oracle.clone()),
        "deltaOracle": Value::Object(delta_oracle),
        "afterOracle": Value::Object(after_oracle)
    });
    let rendered = serde_json::to_string_pretty(&summary)
        .map_err(|error| format!("render incremental manifest: {error}"))?
        + "\n";
    fs::write(output, &rendered)
        .map_err(|error| format!("write incremental manifest {}: {error}", output.display()))?;
    Ok(rendered)
}

fn active_time_oracle(class: CorpusClass) -> Option<ActiveTimeOracle> {
    (class == CorpusClass::OracleSmall).then_some(ActiveTimeOracle {
        interval_count: 4_196,
        total_elapsed_seconds: 10_110,
        total_active_seconds: 10_784,
        main_exclusive_seconds: 10_110,
        subagent_exclusive_seconds: 674,
    })
}

fn insight_eligibility(class: CorpusClass) -> Vec<InsightEligibility> {
    if class != CorpusClass::OracleSmall {
        return Vec::new();
    }
    [
        ("comparison", "unavailable", 0, 14),
        ("trend", "unavailable", 2, 8),
        ("active-efficiency", "partial", 3_584, 5),
        ("reliability", "unavailable", 4, 10),
        ("tool-behavior", "partial", 3_580, 5),
        ("model-routing", "partial", 3_584, 5),
        ("project-concentration", "partial", 32, 1),
        ("anomaly", "unavailable", 2, 7),
        ("recommendation", "unavailable", 3_584, 10),
        ("entertainment", "unavailable", 3_584, 20),
    ]
    .into_iter()
    .map(
        |(family, availability, sample_count, minimum_sample_count)| InsightEligibility {
            family,
            availability,
            sample_count,
            minimum_sample_count,
        },
    )
    .collect()
}

fn source_bytes(corpus: &Path) -> Result<u64, String> {
    ["projects", "otel"]
        .into_iter()
        .try_fold(0u64, |total, name| {
            relative_source_files(&corpus.join(name))?
                .into_iter()
                .try_fold(total, |total, relative| {
                    let bytes = fs::metadata(corpus.join(name).join(relative))
                        .map_err(|error| format!("measure incremental source: {error}"))?
                        .len();
                    total
                        .checked_add(bytes)
                        .ok_or_else(|| "incremental source byte count overflowed".to_string())
                })
        })
}

fn oracle_json(oracle: &Oracle) -> Map<String, Value> {
    [
        ("acceptedRecords", oracle.accepted_records),
        ("canonicalRecords", oracle.canonical_records),
        ("malformedRecords", oracle.malformed_records),
        ("unsupportedRecords", oracle.unsupported_records),
        ("unknownRecords", oracle.unknown_records),
        ("filteredRecords", oracle.filtered_records),
        ("duplicateRecords", oracle.duplicate_records),
        ("resolvedOverlapRecords", oracle.resolved_overlap_records),
        (
            "unresolvedOverlapRecords",
            oracle.unresolved_overlap_records,
        ),
        ("inputTokens", oracle.input_tokens),
        ("outputTokens", oracle.output_tokens),
        ("cacheCreationTokens", oracle.cache_creation_tokens),
        ("cacheReadTokens", oracle.cache_read_tokens),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_string(), Value::from(value)))
    .collect()
}

fn add_oracles(
    before: &Map<String, Value>,
    delta: &Map<String, Value>,
) -> Result<Map<String, Value>, String> {
    before
        .iter()
        .map(|(key, before_value)| {
            let before_value = before_value
                .as_u64()
                .ok_or_else(|| format!("base oracle `{key}` is not an unsigned integer"))?;
            let delta_value = delta.get(key).and_then(Value::as_u64).unwrap_or(0);
            let after = before_value
                .checked_add(delta_value)
                .ok_or_else(|| format!("incremental oracle `{key}` overflowed"))?;
            Ok((key.clone(), Value::from(after)))
        })
        .collect()
}

fn validate_requested_bytes(class: CorpusClass, target_bytes: u64) -> Result<(), String> {
    let valid = match class {
        CorpusClass::OracleSmall => target_bytes <= 16 * 1024 * 1024,
        CorpusClass::Decision => ((512 * 1024 * 1024 * 95 / 100)..=(512 * 1024 * 1024 * 105 / 100))
            .contains(&target_bytes),
        CorpusClass::SaturationLarge => {
            ((2 * 1024 * 1024 * 1024)..=MAXIMUM_CORPUS_BYTES).contains(&target_bytes)
        }
    };
    if valid {
        Ok(())
    } else {
        Err(format!(
            "requested byte count {target_bytes} is outside the frozen {} corpus bounds",
            class.name()
        ))
    }
}

fn validate_summary(summary: &CorpusSummary) -> Result<(), String> {
    let metric_pairs = summary.otel_files / 2;
    let metric_shape_is_exact = summary.metric_points
        == summary
            .otel_files
            .saturating_mul(METRIC_RECORDS_PER_OTEL_FILE) as u64
        && summary.metric_accepted_points == metric_pairs.saturating_mul(5) as u64
        && summary.metric_filtered_points == metric_pairs as u64
        && summary.metric_delta_points == metric_pairs.saturating_mul(3) as u64
        && summary.metric_cumulative_points == metric_pairs.saturating_mul(3) as u64
        && summary.metric_reset_points == metric_pairs as u64
        && summary.metric_gap_points == metric_pairs.saturating_mul(2) as u64
        && summary.metric_overlap_points == metric_pairs as u64;
    let valid = match summary.class {
        CorpusClass::OracleSmall => {
            (10_000..=20_000).contains(&summary.physical_records)
                && summary.source_bytes <= 16 * 1024 * 1024
        }
        CorpusClass::Decision => {
            summary.physical_records >= 500_000
                && (250_000..=500_000).contains(&summary.normalized_candidates)
                && ((512 * 1024 * 1024 * 95 / 100)..=(512 * 1024 * 1024 * 105 / 100))
                    .contains(&summary.source_bytes)
        }
        CorpusClass::SaturationLarge => {
            summary.physical_records >= 2_000_000
                && (750_000..=950_000).contains(&summary.normalized_candidates)
                && ((2 * 1024 * 1024 * 1024)..=MAXIMUM_CORPUS_BYTES).contains(&summary.source_bytes)
        }
    };
    if valid && metric_shape_is_exact {
        Ok(())
    } else {
        Err(format!(
            "generated {} corpus violates its frozen shape: physical_records={}, normalized_candidates={}, source_bytes={}",
            summary.class,
            summary.physical_records,
            summary.normalized_candidates,
            summary.source_bytes
        ))
    }
}

fn otel_metric_line(
    file_index: usize,
    record_index: usize,
    padding: &str,
    oracle: &mut Oracle,
) -> String {
    let pair = file_index / 2;
    let base = METRIC_BASE_NANOS.saturating_add(
        (pair as u64)
            .saturating_mul(60)
            .saturating_mul(NANOS_PER_SECOND),
    );
    let second = |offset: u64| base.saturating_add(offset.saturating_mul(NANOS_PER_SECOND));
    let (temporality, start_nanos, end_nanos, value, accepted, reset, gap, overlap) =
        match (file_index % 2, record_index) {
            (0, 0) => (1, second(0), second(10), 4, true, false, false, false),
            (0, 1) => (1, second(20), second(30), 5, true, false, true, false),
            (0, 2) => (2, second(0), second(10), 10, true, false, false, false),
            (1, 0) => (1, second(25), second(35), 6, false, false, false, true),
            (1, 1) => (2, second(0), second(20), 16, true, false, false, false),
            (1, 2) => (2, second(30), second(40), 3, true, true, true, false),
            _ => unreachable!("metric records are limited to the frozen three-point file shape"),
        };

    oracle.physical_records = oracle.physical_records.saturating_add(1);
    oracle.metric_points = oracle.metric_points.saturating_add(1);
    if temporality == 1 {
        oracle.metric_delta_points = oracle.metric_delta_points.saturating_add(1);
    } else {
        oracle.metric_cumulative_points = oracle.metric_cumulative_points.saturating_add(1);
    }
    if reset {
        oracle.metric_reset_points = oracle.metric_reset_points.saturating_add(1);
    }
    if gap {
        oracle.metric_gap_points = oracle.metric_gap_points.saturating_add(1);
    }
    if overlap {
        oracle.metric_overlap_points = oracle.metric_overlap_points.saturating_add(1);
    }
    if accepted {
        let delta = match (file_index % 2, record_index) {
            (1, 1) => 6,
            _ => value,
        };
        oracle.accepted_records = oracle.accepted_records.saturating_add(1);
        oracle.canonical_records = oracle.canonical_records.saturating_add(1);
        oracle.output_tokens = oracle.output_tokens.saturating_add(delta);
        oracle.metric_accepted_points = oracle.metric_accepted_points.saturating_add(1);
    } else {
        oracle.filtered_records = oracle.filtered_records.saturating_add(1);
        oracle.metric_filtered_points = oracle.metric_filtered_points.saturating_add(1);
    }

    json!({
        "resourceMetrics": [{
            "resource": {
                "attributes": [
                    {"key": "service.name", "value": {"stringValue": "claude-code"}},
                    {"key": "user.email", "value": {"stringValue": "SYNTHETIC_PHASE5_EMAIL_CANARY@example.invalid"}},
                    {"key": "benchmark.padding", "value": {"stringValue": padding}}
                ]
            },
            "scopeMetrics": [{
                "scope": {"name": "com.anthropic.claude_code"},
                "metrics": [{
                    "name": "claude_code.token.usage",
                    "unit": "tokens",
                    "sum": {
                        "aggregationTemporality": temporality,
                        "isMonotonic": true,
                        "dataPoints": [{
                            "attributes": [
                                {"key": "session.id", "value": {"stringValue": format!("metric-stream-{pair:05}")}},
                                {"key": "type", "value": {"stringValue": "output"}},
                                {"key": "model", "value": {"stringValue": "claude-sonnet-4-6"}}
                            ],
                            "startTimeUnixNano": start_nanos.to_string(),
                            "timeUnixNano": end_nanos.to_string(),
                            "asInt": value.to_string()
                        }]
                    }
                }]
            }]
        }]
    })
    .to_string()
}

fn transcript_line(
    file_index: usize,
    record_index: usize,
    global: u64,
    session: &str,
    padding: &str,
    oracle: &mut Oracle,
) -> String {
    oracle.physical_records = oracle.physical_records.saturating_add(1);
    match record_index % 100 {
        0 => {
            oracle.malformed_records = oracle.malformed_records.saturating_add(1);
            format!("{{\"malformed\":\"{padding}\"")
        }
        1 => {
            oracle.unsupported_records = oracle.unsupported_records.saturating_add(1);
            oracle.unknown_records = oracle.unknown_records.saturating_add(1);
            format!(
                "{{\"type\":\"future_benchmark_variant\",\"timestamp\":\"{}\",\"payload\":\"{padding}\"}}",
                timestamp(global, false)
            )
        }
        2..=61 => {
            oracle.filtered_records = oracle.filtered_records.saturating_add(1);
            assistant_line(
                file_index,
                record_index,
                global,
                session,
                &timestamp(global, true),
                padding,
            )
        }
        63 | 65 | 67 | 69 | 71 => {
            oracle.duplicate_records = oracle.duplicate_records.saturating_add(1);
            let original_index = record_index.saturating_sub(1);
            let original_global = global.saturating_sub(1);
            assistant_line(
                file_index,
                original_index,
                original_global,
                session,
                &timestamp(original_global, false),
                padding,
            )
        }
        95..=99 => {
            oracle.accepted_records = oracle.accepted_records.saturating_add(1);
            oracle.canonical_records = oracle.canonical_records.saturating_add(1);
            format!(
                concat!(
                    "{{\"type\":\"user\",\"sessionId\":\"{}\",\"timestamp\":\"{}\",",
                    "\"message\":{{\"id\":\"user-{file_index:05}-{record_index:05}\",",
                    "\"content\":\"SYNTHETIC_PHASE5_PROMPT_CANARY {padding}\"}}}}"
                ),
                session,
                timestamp(global, false),
                file_index = file_index,
                record_index = record_index,
                padding = padding,
            )
        }
        _ => {
            oracle.accepted_records = oracle.accepted_records.saturating_add(1);
            oracle.canonical_records = oracle.canonical_records.saturating_add(1);
            oracle.observe_tokens(global);
            assistant_line(
                file_index,
                record_index,
                global,
                session,
                &timestamp(global, false),
                padding,
            )
        }
    }
}

fn incremental_line(
    file_index: usize,
    record_index: usize,
    timestamp_index: u64,
    session: &str,
    oracle: &mut Oracle,
) -> String {
    oracle.physical_records = oracle.physical_records.saturating_add(1);
    oracle.accepted_records = oracle.accepted_records.saturating_add(1);
    oracle.canonical_records = oracle.canonical_records.saturating_add(1);
    format!(
        concat!(
            "{{\"type\":\"user\",\"sessionId\":\"{}\",\"timestamp\":\"{}\",",
            "\"message\":{{\"id\":\"incremental-user-{file_index:05}-{record_index:05}\",",
            "\"content\":\"SYNTHETIC_PHASE5_INCREMENTAL_PROMPT\"}}}}"
        ),
        session,
        timestamp(timestamp_index, false),
        file_index = file_index,
        record_index = record_index,
    )
}

fn background_line(global: u64, record_index: usize, oracle: &mut Oracle) -> String {
    oracle.physical_records = oracle.physical_records.saturating_add(1);
    match record_index % 100 {
        0 => {
            oracle.malformed_records = oracle.malformed_records.saturating_add(1);
            "{\"malformed\":".to_string()
        }
        1 => {
            oracle.unsupported_records = oracle.unsupported_records.saturating_add(1);
            oracle.unknown_records = oracle.unknown_records.saturating_add(1);
            format!(
                "{{\"type\":\"future_benchmark_variant\",\"timestamp\":\"{}\"}}",
                timestamp(global, true)
            )
        }
        _ => {
            oracle.filtered_records = oracle.filtered_records.saturating_add(1);
            format!(
                "{{\"type\":\"assistant\",\"timestamp\":\"{}\",\"message\":{{}},\"benchmark\":{}}}",
                timestamp(global, true),
                background_probe(),
            )
        }
    }
}

fn background_probe() -> &'static str {
    static PROBE: OnceLock<String> = OnceLock::new();
    PROBE.get_or_init(|| format!("{}0{}", "[".repeat(103), "]".repeat(103)))
}

fn assistant_line(
    file_index: usize,
    record_index: usize,
    global: u64,
    session: &str,
    timestamp: &str,
    padding: &str,
) -> String {
    format!(
        concat!(
            "{{\"type\":\"assistant\",\"sessionId\":\"{}\",",
            "\"requestId\":\"request-{file_index:05}-{record_index:05}\",",
            "\"timestamp\":\"{}\",\"message\":{{",
            "\"id\":\"message-{file_index:05}-{record_index:05}\",",
            "\"model\":\"claude-sonnet-4-6\",\"usage\":{{",
            "\"input_tokens\":1,\"output_tokens\":{},",
            "\"cache_creation_input_tokens\":2,\"cache_read_input_tokens\":3,",
            "\"cache_creation\":{{\"ephemeral_5m_input_tokens\":2,",
            "\"ephemeral_1h_input_tokens\":0}}}},",
            "\"content\":[{{\"type\":\"tool_use\",\"id\":\"tool-{global}\",",
            "\"name\":\"Read\",\"input\":{{\"path\":\"SYNTHETIC_PHASE5_PATH_CANARY\",",
            "\"padding\":\"{padding}\"}}}}]}}}}"
        ),
        session,
        timestamp,
        output_tokens(global),
        file_index = file_index,
        record_index = record_index,
        global = global,
        padding = padding,
    )
}

fn otel_request_line(token_index: u64, session: &str, request: &str, padding: &str) -> String {
    let timestamp = timestamp(token_index, false);
    let unix_nanos = 1_767_225_600_000_000_000u128
        .saturating_add(u128::from(token_index).saturating_mul(1_000_000_000));
    format!(
        concat!(
            "{{\"resourceLogs\":[{{\"resource\":{{\"attributes\":[",
            "{{\"key\":\"service.name\",\"value\":{{\"stringValue\":\"claude-code\"}}}},",
            "{{\"key\":\"user.email\",\"value\":{{\"stringValue\":\"SYNTHETIC_PHASE5_EMAIL_CANARY@example.invalid\"}}}}",
            "]}},\"scopeLogs\":[{{\"scope\":{{\"name\":\"com.anthropic.claude_code.events\"}},",
            "\"logRecords\":[{{\"timeUnixNano\":\"{unix_nanos}\",\"body\":{{}},",
            "\"attributes\":[",
            "{{\"key\":\"event.timestamp\",\"value\":{{\"stringValue\":\"{timestamp}\"}}}},",
            "{{\"key\":\"session.id\",\"value\":{{\"stringValue\":\"{session}\"}}}},",
            "{{\"key\":\"request_id\",\"value\":{{\"stringValue\":\"{request}\"}}}},",
            "{{\"key\":\"model\",\"value\":{{\"stringValue\":\"claude-sonnet-4-6\"}}}},",
            "{{\"key\":\"input_tokens\",\"value\":{{\"intValue\":\"1\"}}}},",
            "{{\"key\":\"output_tokens\",\"value\":{{\"intValue\":\"{}\"}}}},",
            "{{\"key\":\"cache_read_tokens\",\"value\":{{\"intValue\":\"3\"}}}},",
            "{{\"key\":\"cache_creation_tokens\",\"value\":{{\"intValue\":\"2\"}}}},",
            "{{\"key\":\"cost_usd\",\"value\":{{\"doubleValue\":0.0001}}}},",
            "{{\"key\":\"duration_ms\",\"value\":{{\"intValue\":\"125\"}}}},",
            "{{\"key\":\"benchmark.padding\",\"value\":{{\"stringValue\":\"{padding}\"}}}}",
            "],\"eventName\":\"claude_code.api_request\"}}]}}]}}]}}"
        ),
        output_tokens(token_index),
        unix_nanos = unix_nanos,
        timestamp = timestamp,
        session = session,
        request = request,
        padding = padding,
    )
}

fn timestamp(index: u64, filtered: bool) -> String {
    let year = if filtered { 2025 } else { 2026 };
    let day = 1 + (index / 86_400) % 28;
    let seconds = index % 86_400;
    let hour = seconds / 3_600;
    let minute = (seconds % 3_600) / 60;
    let second = seconds % 60;
    format!("{year:04}-01-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn output_tokens(index: u64) -> u64 {
    index % 7 + 1
}

fn fold_file_fingerprint(current: u64, file: u64, bytes: u64) -> u64 {
    let mut value = current;
    for byte in file.to_le_bytes().into_iter().chain(bytes.to_le_bytes()) {
        value ^= u64::from(byte);
        value = value.wrapping_mul(0x0000_0100_0000_01b3);
    }
    value
}

pub fn relative_source_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = fs::read_dir(directory)
        .map_err(|error| format!("read directory {}: {error}", directory.display()))?;
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("read directory {}: {error}", directory.display()))?;
        let path = entry.path();
        let metadata = entry
            .metadata()
            .map_err(|error| format!("inspect {}: {error}", path.display()))?;
        if metadata.is_dir() {
            collect_files(root, &path, files)?;
        } else if metadata.is_file() {
            files.push(
                path.strip_prefix(root)
                    .map_err(|error| format!("relativize {}: {error}", path.display()))?
                    .to_path_buf(),
            );
        }
    }
    Ok(())
}

pub fn byte_identity(left: &Path, right: &Path) -> Result<bool, String> {
    let left_files = relative_source_files(left)?;
    let right_files = relative_source_files(right)?;
    if left_files != right_files {
        return Ok(false);
    }
    for relative in left_files {
        let left_bytes = fs::read(left.join(&relative))
            .map_err(|error| format!("read {}: {error}", left.join(&relative).display()))?;
        let right_bytes = fs::read(right.join(&relative))
            .map_err(|error| format!("read {}: {error}", right.join(&relative).display()))?;
        if left_bytes != right_bytes {
            return Ok(false);
        }
    }
    Ok(true)
}
