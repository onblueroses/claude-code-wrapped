use crate::readers::jsonl::{decode_project_hash, derive_project_name};
use crate::{
    parse_timestamp, timestamp_year, SessionBreakdown, SessionPrompt, SessionSummary,
    SubagentSummary, TokenUsage,
};
use glob::glob;
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct JsonlRecord {
    #[serde(rename = "type")]
    record_type: Option<String>,
    user_type: Option<String>,
    #[serde(default)]
    is_sidechain: bool,
    message: Option<JsonlMessage>,
    timestamp: Option<String>,
    session_id: Option<String>,
    cwd: Option<String>,
    entrypoint: Option<String>,
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

pub fn read_session_breakdown(projects_dir: &Path, year: Option<i32>) -> SessionBreakdown {
    let pattern = format!("{}/{}/*.jsonl", projects_dir.display(), "*");
    let mut sessions = Vec::new();

    if let Ok(paths) = glob(&pattern) {
        for path in paths.flatten().filter(|path| path.is_file()) {
            let Some(project_hash) = path
                .parent()
                .and_then(Path::file_name)
                .map(|value| value.to_string_lossy().to_string())
            else {
                continue;
            };

            if let Some(session) = parse_session_file(&path, &project_hash, false, year) {
                if session.total_tokens > 0 {
                    sessions.push(session);
                }
            }
        }
    }

    sessions.sort_by(|left, right| right.usage.output_tokens.cmp(&left.usage.output_tokens));

    let mut costly_subagents = sessions
        .iter()
        .flat_map(|session| session.subagents.clone())
        .collect::<Vec<_>>();
    costly_subagents.sort_by(|left, right| right.total_tokens.cmp(&left.total_tokens));

    SessionBreakdown {
        costly_sessions: sessions.iter().take(20).cloned().collect(),
        costly_subagents: costly_subagents.iter().take(20).cloned().collect(),
        total_subagent_sessions: costly_subagents.len(),
        total_subagent_tokens: costly_subagents.iter().map(|item| item.total_tokens).sum(),
        sessions,
    }
}

fn parse_session_file(
    file_path: &Path,
    project_hash: &str,
    is_subagent: bool,
    year: Option<i32>,
) -> Option<SessionSummary> {
    let raw = fs::read_to_string(file_path).ok()?;
    let mut totals = TokenUsage::default();
    let mut model_totals: BTreeMap<String, TokenUsage> = BTreeMap::new();
    let mut prompts = Vec::new();
    let mut seen_message_ids = HashSet::new();
    let mut tool_message_count = 0usize;
    let mut session_id = file_path.file_stem()?.to_string_lossy().to_string();
    let mut timestamp_start: Option<String> = None;
    let mut timestamp_end: Option<String> = None;
    let mut cwd: Option<String> = None;

    for line in raw.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(record) = serde_json::from_str::<JsonlRecord>(line) else {
            continue;
        };

        let Some(timestamp) = record.timestamp.clone() else {
            continue;
        };

        if let Some(selected_year) = year {
            if timestamp_year(&timestamp) != Some(selected_year) {
                continue;
            }
        }

        if timestamp_start
            .as_ref()
            .map(|value| timestamp < *value)
            .unwrap_or(true)
        {
            timestamp_start = Some(timestamp.clone());
        }
        if timestamp_end
            .as_ref()
            .map(|value| timestamp > *value)
            .unwrap_or(true)
        {
            timestamp_end = Some(timestamp.clone());
        }
        if cwd.is_none() {
            cwd = record.cwd.clone();
        }
        if let Some(record_session_id) = record.session_id {
            session_id = record_session_id;
        }

        match record.record_type.as_deref() {
            Some("assistant") => {
                let Some(message) = record.message else {
                    continue;
                };

                if let Some(message_id) = message.id {
                    if !seen_message_ids.insert(message_id) {
                        continue;
                    }
                }

                let Some(usage) = message.usage else {
                    continue;
                };
                let usage = TokenUsage {
                    input_tokens: usage.input_tokens.unwrap_or(0),
                    output_tokens: usage.output_tokens.unwrap_or(0),
                    cache_creation_tokens: usage.cache_creation_input_tokens.unwrap_or(0),
                    cache_read_tokens: usage.cache_read_input_tokens.unwrap_or(0),
                };

                totals += &usage;

                let model_name = message.model.unwrap_or_else(|| "unknown".to_string());
                let model_usage = model_totals.entry(model_name).or_default();
                *model_usage += &usage;
            }
            Some("user") if !record.is_sidechain && record.user_type.as_deref() != Some("tool") => {
                let Some(message) = record.message else {
                    continue;
                };
                let text = extract_user_text(message.content.as_ref());
                if !text.is_empty() {
                    prompts.push(SessionPrompt {
                        text,
                        timestamp: Some(timestamp),
                        entrypoint: record.entrypoint,
                    });
                }
            }
            Some("user") if record.user_type.as_deref() == Some("tool") => {
                tool_message_count += 1;
            }
            _ => {}
        }
    }

    let project_path = cwd.clone().or_else(|| decode_project_hash(project_hash).0);
    let project_name = project_path
        .as_deref()
        .map(derive_project_name)
        .unwrap_or_else(|| "Unknown".to_string());
    let duration_minutes = duration_minutes(timestamp_start.as_deref(), timestamp_end.as_deref());
    let total_tokens = totals.total_tokens();

    let mut subagents = Vec::new();
    if !is_subagent {
        let subagent_dir = file_path
            .file_stem()
            .map(|stem| file_path.with_file_name(stem))
            .map(|dir| dir.join("subagents"));

        if let Some(subagent_dir) = subagent_dir {
            let pattern = format!("{}/**/*.jsonl", subagent_dir.display());
            if let Ok(paths) = glob(&pattern) {
                for path in paths.flatten().filter(|path| path.is_file()) {
                    if let Some(subagent) = parse_session_file(&path, project_hash, true, year) {
                        if subagent.total_tokens > 0 {
                            subagents.push(SubagentSummary {
                                session_id: subagent.session_id.clone(),
                                timestamp_start: subagent.timestamp_start.clone(),
                                duration_minutes: subagent.duration_minutes,
                                total_tokens: subagent.total_tokens,
                                usage: subagent.usage.clone(),
                                first_prompt: subagent.first_prompt.clone(),
                                project_path: subagent.project_path.clone(),
                                project_name: Some(subagent.project_name.clone()),
                                parent_session_id: Some(session_id.clone()),
                            });
                        }
                    }
                }
            }
        }
    }

    Some(SessionSummary {
        session_id,
        project_hash: project_hash.to_string(),
        project_path,
        project_name,
        timestamp_start,
        timestamp_end,
        duration_minutes,
        usage: totals,
        model_totals,
        total_tokens,
        prompt_count: prompts.len(),
        tool_message_count,
        first_prompt: prompts.first().map(|prompt| prompt.text.clone()),
        prompts: prompts.into_iter().take(5).collect(),
        subagents,
    })
}

fn extract_user_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Array(items)) => {
            let mut parts = Vec::new();
            for item in items {
                if let Some(text) = item.as_str() {
                    if !text.trim().is_empty() {
                        parts.push(text.to_string());
                    }
                    continue;
                }

                let Some(object) = item.as_object() else {
                    continue;
                };

                if object.get("type").and_then(Value::as_str) == Some("tool_result") {
                    continue;
                }

                if let Some(text) = object.get("text").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        parts.push(text.to_string());
                    }
                } else if let Some(text) = object.get("content").and_then(Value::as_str) {
                    if !text.trim().is_empty() {
                        parts.push(text.to_string());
                    }
                }
            }
            parts.join("\n").trim().to_string()
        }
        _ => String::new(),
    }
}

fn duration_minutes(start: Option<&str>, end: Option<&str>) -> u64 {
    match (
        start.and_then(parse_timestamp),
        end.and_then(parse_timestamp),
    ) {
        (Some(start), Some(end)) if end >= start => {
            let minutes = (end - start).num_minutes();
            if minutes <= 0 {
                0
            } else {
                minutes as u64
            }
        }
        _ => 0,
    }
}
