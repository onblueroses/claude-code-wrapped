use crate::analyzers::cost::approximate_cost;
use crate::{
    timestamp_date_key, timestamp_year, AssistantEntry, DailyAggregate, ModelAggregate,
    ProjectSummary,
};
use glob::glob;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct JsonlRecord {
    #[serde(rename = "type")]
    record_type: Option<String>,
    message: Option<JsonlMessage>,
    #[serde(rename = "costUSD")]
    cost_usd: Option<f64>,
    timestamp: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct JsonlMessage {
    id: Option<String>,
    model: Option<String>,
    usage: Option<JsonlUsage>,
    content: Option<Value>,
}

#[derive(Debug, Deserialize, Default)]
struct JsonlUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
    cache_creation_input_tokens: Option<u64>,
    cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
struct FileContext {
    session_id: String,
    project_hash: String,
    is_subagent: bool,
}

pub fn discover_jsonl_files(projects_dir: &Path) -> Vec<PathBuf> {
    let pattern = format!("{}/**/*.jsonl", projects_dir.display());
    let mut files = glob(&pattern)
        .ok()
        .into_iter()
        .flat_map(|entries| entries.flatten())
        .filter(|path| path.is_file())
        .collect::<Vec<_>>();
    files.sort();
    files
}

pub fn read_all_jsonl(projects_dir: &Path, year: Option<i32>) -> Vec<AssistantEntry> {
    let mut entries = Vec::new();
    let mut seen_message_ids = HashSet::new();
    let mut skipped_files = 0usize;
    let mut skipped_lines = 0usize;

    for path in discover_jsonl_files(projects_dir) {
        let Some(context) = file_context(projects_dir, &path) else {
            continue;
        };

        let Ok(raw) = fs::read_to_string(&path) else {
            skipped_files += 1;
            continue;
        };

        for line in raw.lines().filter(|line| !line.trim().is_empty()) {
            let Ok(record) = serde_json::from_str::<JsonlRecord>(line) else {
                // Only count as malformed if the line looks like intended JSON, not
                // a truncated continuation fragment or stray whitespace.
                // Use trim_start so leading-space records (e.g. indented lines or
                // partial records whose opening brace was not the first byte) are
                // still counted rather than silently discarded.
                let trimmed = line.trim_start();
                if trimmed.starts_with('{') || trimmed.starts_with('[') {
                    skipped_lines += 1;
                }
                continue;
            };

            if record.record_type.as_deref() != Some("assistant") {
                continue;
            }

            let Some(timestamp) = record.timestamp.clone() else {
                continue;
            };

            if let Some(selected_year) = year {
                if timestamp_year(&timestamp) != Some(selected_year) {
                    continue;
                }
            }

            let Some(message) = record.message else {
                continue;
            };
            let Some(usage) = message.usage else {
                continue;
            };

            if let Some(message_id) = message.id {
                if !seen_message_ids.insert(message_id) {
                    continue;
                }
            }

            entries.push(AssistantEntry {
                session_id: record
                    .session_id
                    .unwrap_or_else(|| context.session_id.clone()),
                project_hash: context.project_hash.clone(),
                is_subagent: context.is_subagent,
                cwd: record.cwd,
                timestamp,
                model: message.model.unwrap_or_else(|| "unknown".to_string()),
                input_tokens: usage.input_tokens.unwrap_or(0),
                output_tokens: usage.output_tokens.unwrap_or(0),
                cache_creation_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
                cache_read_tokens: usage.cache_read_input_tokens.unwrap_or(0),
                cost_usd: record.cost_usd.unwrap_or(0.0),
                tool_names: extract_tool_names(message.content.as_ref()),
            });
        }
    }

    // Sort by parsed epoch so mixed UTC/offset timestamps order correctly.
    entries.sort_by_key(|e| {
        crate::parse_timestamp(&e.timestamp)
            .map(|dt| dt.timestamp())
            .unwrap_or(0)
    });
    if skipped_files > 0 || skipped_lines > 0 {
        eprintln!(
            "warning: skipped {} unreadable file(s), {} malformed line(s) — report may be incomplete",
            skipped_files, skipped_lines
        );
    }
    entries
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
        day.total_cost += entry_cost;
        day.input_tokens += entry.input_tokens;
        day.output_tokens += entry.output_tokens;
        day.cache_creation_tokens += entry.cache_creation_tokens;
        day.cache_read_tokens += entry.cache_read_tokens;
        day.message_count += 1;
        day.session_ids.insert(entry.session_id.clone());

        let model = day.models.entry(entry.model.clone()).or_default();
        model.input_tokens += entry.input_tokens;
        model.output_tokens += entry.output_tokens;
        model.cache_creation_tokens += entry.cache_creation_tokens;
        model.cache_read_tokens += entry.cache_read_tokens;
        model.cost += entry_cost;
        model.message_count += 1;
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

        project.input_tokens += entry.input_tokens;
        project.output_tokens += entry.output_tokens;
        project.cache_creation_tokens += entry.cache_creation_tokens;
        project.cache_read_tokens += entry.cache_read_tokens;
        project.message_count += 1;
        project.sessions.insert(entry.session_id.clone());
        if entry.is_subagent {
            project.subagent_sessions.insert(entry.session_id.clone());
        } else {
            project.top_level_sessions.insert(entry.session_id.clone());
        }

        let entry_epoch = crate::parse_timestamp(&entry.timestamp).map(|dt| dt.timestamp());
        match (&project.first_seen, &project.last_seen) {
            (None, None) => {
                project.first_seen = Some(entry.timestamp.clone());
                project.last_seen = Some(entry.timestamp.clone());
            }
            (Some(first), Some(last)) => {
                let first_epoch = crate::parse_timestamp(first).map(|dt| dt.timestamp());
                let last_epoch = crate::parse_timestamp(last).map(|dt| dt.timestamp());
                if entry_epoch < first_epoch {
                    project.first_seen = Some(entry.timestamp.clone());
                }
                if entry_epoch > last_epoch {
                    project.last_seen = Some(entry.timestamp.clone());
                }
            }
            _ => {}
        }

        if let Some(cwd) = &entry.cwd {
            *project.cwd_counts.entry(cwd.clone()).or_insert(0) += 1;
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
                first_seen: project.first_seen,
                last_seen: project.last_seen,
            }
        })
        .collect::<Vec<_>>();

    projects.sort_by(|left, right| right.output_tokens.cmp(&left.output_tokens));
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
    if let Some((path, _)) = cwd_counts.iter().max_by(|left, right| left.1.cmp(right.1)) {
        return (Some(path.clone()), derive_project_name(path));
    }
    decode_project_hash(fallback_hash)
}

fn file_context(projects_dir: &Path, file_path: &Path) -> Option<FileContext> {
    let relative = file_path.strip_prefix(projects_dir).ok()?;
    let parts = relative
        .iter()
        .map(|part| part.to_string_lossy().to_string())
        .collect::<Vec<_>>();
    if parts.len() < 2 {
        return None;
    }

    Some(FileContext {
        session_id: file_path.file_stem()?.to_string_lossy().to_string(),
        project_hash: parts.first()?.to_string(),
        is_subagent: parts.iter().any(|part| part == "subagents"),
    })
}

fn extract_tool_names(content: Option<&Value>) -> Vec<String> {
    let Some(Value::Array(items)) = content else {
        return Vec::new();
    };

    items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            let item_type = object.get("type")?.as_str()?;
            if item_type != "tool_use" {
                return None;
            }
            object.get("name")?.as_str().map(str::to_string)
        })
        .collect()
}

fn round_ratio(numerator: u64, denominator: u64) -> u64 {
    if denominator == 0 {
        0
    } else {
        (numerator as f64 / denominator as f64).round() as u64
    }
}

fn resolved_entry_cost(entry: &AssistantEntry) -> f64 {
    if entry.cost_usd > 0.0 {
        entry.cost_usd
    } else {
        approximate_cost(&entry.model, &entry.usage())
    }
}
