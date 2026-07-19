use crate::readers::{
    compatibility_ingest, emit_compatibility_coverage, emit_compatibility_error, IngestionReadError,
};
use crate::{
    round_ratio, timestamp_date_key, AssistantEntry, DailyAggregate, DataCoverage, ModelAggregate,
    ProjectSummary,
};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;

/// Reads assistant occurrences through the bounded, privacy-safe normalized pipeline.
///
/// This preserves the original infallible signature. Prefer [`try_read_all_jsonl`] when
/// coverage or an actionable ingestion error is required.
pub fn read_all_jsonl(projects_dir: &Path, year: Option<i32>) -> Vec<AssistantEntry> {
    match try_read_all_jsonl(projects_dir, year) {
        Ok((entries, coverage)) => {
            emit_compatibility_coverage(&coverage, "read_all_jsonl");
            entries
        }
        Err(error) => {
            emit_compatibility_error(&error, "read_all_jsonl");
            Vec::new()
        }
    }
}

/// Returns privacy-safe assistant occurrences together with ingestion coverage.
pub fn try_read_all_jsonl(
    projects_dir: &Path,
    year: Option<i32>,
) -> Result<(Vec<AssistantEntry>, DataCoverage), IngestionReadError> {
    let ingested = compatibility_ingest(projects_dir, year)?;
    let entries = ingested
        .entries
        .iter()
        .filter(|entry| entry.is_message_occurrence())
        .map(|entry| entry.observed_accumulator())
        .collect();
    Ok((entries, ingested.coverage))
}

pub fn aggregate_daily(entries: &[AssistantEntry]) -> Vec<DailyAggregate> {
    #[derive(Default)]
    struct Accumulator {
        total_cost: f64,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
        message_count: usize,
        session_ids: BTreeSet<String>,
        models: BTreeMap<String, ModelAggregate>,
    }

    let mut by_date: BTreeMap<String, Accumulator> = BTreeMap::new();

    for entry in entries {
        let Some(date) = timestamp_date_key(&entry.timestamp) else {
            continue;
        };

        let entry_cost = resolved_entry_cost(entry);
        let day = by_date.entry(date.clone()).or_default();
        day.total_cost = (day.total_cost + entry_cost).min(f64::MAX);
        day.input_tokens = day.input_tokens.saturating_add(entry.input_tokens);
        day.output_tokens = day.output_tokens.saturating_add(entry.output_tokens);
        day.cache_creation_tokens = day
            .cache_creation_tokens
            .saturating_add(entry.cache_creation_tokens);
        day.cache_read_tokens = day
            .cache_read_tokens
            .saturating_add(entry.cache_read_tokens);
        day.message_count = day.message_count.saturating_add(1);
        day.session_ids.insert(entry.session_id.clone());

        let model = day.models.entry(entry.model.clone()).or_default();
        model.input_tokens = model.input_tokens.saturating_add(entry.input_tokens);
        model.output_tokens = model.output_tokens.saturating_add(entry.output_tokens);
        model.cache_creation_tokens = model
            .cache_creation_tokens
            .saturating_add(entry.cache_creation_tokens);
        model.cache_read_tokens = model
            .cache_read_tokens
            .saturating_add(entry.cache_read_tokens);
        model.cost = (model.cost + entry_cost).min(f64::MAX);
        model.message_count = model.message_count.saturating_add(1);
    }

    by_date
        .into_iter()
        .map(|(date, day)| DailyAggregate {
            date,
            total_cost: day.total_cost,
            input_tokens: day.input_tokens,
            output_tokens: day.output_tokens,
            cache_creation_tokens: day.cache_creation_tokens,
            cache_read_tokens: day.cache_read_tokens,
            message_count: day.message_count,
            session_count: day.session_ids.len(),
            active_seconds: 0,
            cache_output_ratio: round_ratio(day.cache_read_tokens, day.output_tokens),
            models: day.models,
        })
        .collect()
}

pub fn aggregate_by_project(entries: &[AssistantEntry]) -> Vec<ProjectSummary> {
    #[derive(Default)]
    struct Accumulator {
        hash: String,
        input_tokens: u64,
        output_tokens: u64,
        cache_creation_tokens: u64,
        cache_read_tokens: u64,
        message_count: usize,
        sessions: BTreeSet<String>,
        top_level_sessions: BTreeSet<String>,
        subagent_sessions: BTreeSet<String>,
        first_seen: Option<String>,
        last_seen: Option<String>,
        cwd_counts: HashMap<String, usize>,
    }

    let mut by_project: BTreeMap<String, Accumulator> = BTreeMap::new();

    for entry in entries {
        let hash = if entry.project_hash.is_empty() {
            "unknown".to_string()
        } else {
            entry.project_hash.clone()
        };

        let project = by_project
            .entry(hash.clone())
            .or_insert_with(|| Accumulator {
                hash,
                ..Accumulator::default()
            });

        project.input_tokens = project.input_tokens.saturating_add(entry.input_tokens);
        project.output_tokens = project.output_tokens.saturating_add(entry.output_tokens);
        project.cache_creation_tokens = project
            .cache_creation_tokens
            .saturating_add(entry.cache_creation_tokens);
        project.cache_read_tokens = project
            .cache_read_tokens
            .saturating_add(entry.cache_read_tokens);
        project.message_count = project.message_count.saturating_add(1);
        project.sessions.insert(entry.session_id.clone());
        if entry.is_subagent {
            project.subagent_sessions.insert(entry.session_id.clone());
        } else {
            project.top_level_sessions.insert(entry.session_id.clone());
        }

        let entry_epoch = crate::parse_timestamp(&entry.timestamp).map(|dt| dt.timestamp());
        if let Some(entry_epoch) = entry_epoch {
            match (&project.first_seen, &project.last_seen) {
                (None, None) => {
                    project.first_seen = Some(entry.timestamp.clone());
                    project.last_seen = Some(entry.timestamp.clone());
                }
                (Some(first), Some(last)) => {
                    let first_epoch = crate::parse_timestamp(first).map(|dt| dt.timestamp());
                    let last_epoch = crate::parse_timestamp(last).map(|dt| dt.timestamp());
                    if first_epoch.is_some_and(|first_epoch| entry_epoch < first_epoch) {
                        project.first_seen = Some(entry.timestamp.clone());
                    }
                    if last_epoch.is_some_and(|last_epoch| entry_epoch > last_epoch) {
                        project.last_seen = Some(entry.timestamp.clone());
                    }
                }
                _ => {}
            }
        }

        if let Some(cwd) = &entry.cwd {
            let count = project.cwd_counts.entry(cwd.clone()).or_insert(0);
            *count = count.saturating_add(1);
        }
    }

    let mut projects = by_project
        .into_values()
        .map(|project| {
            let (path, name) = resolve_project_path(&project.cwd_counts, &project.hash);
            ProjectSummary {
                hash: project.hash,
                path,
                name,
                input_tokens: project.input_tokens,
                output_tokens: project.output_tokens,
                cache_creation_tokens: project.cache_creation_tokens,
                cache_read_tokens: project.cache_read_tokens,
                message_count: project.message_count,
                session_count: if project.top_level_sessions.is_empty() {
                    project.sessions.len()
                } else {
                    project.top_level_sessions.len()
                },
                subagent_session_count: project.subagent_sessions.len(),
                active_seconds: 0,
                first_seen: project.first_seen,
                last_seen: project.last_seen,
            }
        })
        .collect::<Vec<_>>();

    projects.sort_by(|left, right| {
        right
            .output_tokens
            .cmp(&left.output_tokens)
            .then_with(|| left.hash.cmp(&right.hash))
    });
    projects
}

pub fn derive_project_name(path: &str) -> String {
    if path.is_empty() {
        return "Unknown".to_string();
    }

    let trimmed = path.trim_end_matches('/');
    let segments = trimmed
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>();
    if (trimmed.starts_with("/home/") || trimmed.starts_with("/Users/")) && segments.len() == 2 {
        return "workspace root".to_string();
    }
    segments
        .last()
        .map(|segment| (*segment).to_string())
        .unwrap_or_else(|| path.to_string())
}

pub fn decode_project_hash(hash: &str) -> (Option<String>, String) {
    if hash.is_empty() || hash == "unknown" {
        return (None, "Unknown".to_string());
    }

    // Claude encodes path separators as single hyphens; a literal hyphen in a
    // directory name becomes a double hyphen. A leading hyphen signals an absolute path.
    let path = if let Some(rest) = hash.strip_prefix('-') {
        format!("/{}", decode_hash_segments(rest))
    } else {
        let chars: Vec<char> = hash.chars().collect();
        if chars.len() >= 3 && chars[0].is_ascii_alphabetic() && chars[1] == '-' && chars[2] == '-'
        {
            // Windows-style drive letter prefix (e.g. "c--Users-...")
            format!("{}:/{}", chars[0], decode_hash_segments(&hash[3..]))
        } else {
            decode_hash_segments(hash)
        }
    };

    let name = derive_project_name(&path);
    (Some(path), name)
}

fn decode_hash_segments(s: &str) -> String {
    // Replace "--" with a placeholder so single "-" can be used as the path separator,
    // then restore the placeholder as a literal hyphen.
    const PLACEHOLDER: &str = "\x00";
    s.replace("--", PLACEHOLDER)
        .split('-')
        .map(|seg| seg.replace(PLACEHOLDER, "-"))
        .collect::<Vec<_>>()
        .join("/")
}

pub fn resolve_project_path(
    cwd_counts: &HashMap<String, usize>,
    fallback_hash: &str,
) -> (Option<String>, String) {
    if let Some((path, _)) = cwd_counts
        .iter()
        .max_by(|left, right| left.1.cmp(right.1).then_with(|| right.0.cmp(left.0)))
    {
        return (Some(path.clone()), derive_project_name(path));
    }
    if fallback_hash
        .strip_prefix("project-")
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_digit())
        })
    {
        return (None, fallback_hash.to_string());
    }
    decode_project_hash(fallback_hash)
}

fn resolved_entry_cost(entry: &AssistantEntry) -> f64 {
    crate::ingestion::pricing::legacy_api_equivalent_cost(
        &entry.model,
        &entry.timestamp,
        &entry.usage(),
    )
}
