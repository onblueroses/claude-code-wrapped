use ccwrapped::{DataCoverage, IngestionWarning, SourceCoverage, UnknownShapeDiagnostic};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs::{File, Metadata};
use std::hash::{BuildHasher, Hash, Hasher};
use std::io;
use std::path::Path;
use std::time::SystemTime;

pub(super) const TRANSCRIPT_ADAPTER: &str = "claude-transcript/v1";
pub(super) const OTEL_ADAPTER: &str = "claude-otel-otlp-json/v1";
pub(super) const NORMALIZED_SCHEMA: &str = "ccwrapped.normalized-event/v2";
pub(super) const UNATTRIBUTED_PROJECT_ALIAS: &str = "unattributed";
pub(super) const OTEL_CONTRACT: &str =
    "otelcol-contrib/file/v0.148.0+pdata/v1.54.0+slim-otlp/v1.10.0";
pub(super) const MAX_UNKNOWN_SHAPE_DIAGNOSTICS: usize = 32;
pub(super) const MAX_DIRECT_DURATION_MS: f64 = 86_400_000.0;
const SAFE_TOKEN_LIMIT: usize = 128;
const MAX_SOURCE_COST_ESTIMATE: f64 = 1_000_000_000_000.0;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct FileSnapshot {
    len: u64,
    modified: Option<SystemTime>,
    is_file: bool,
    is_dir: bool,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(unix)]
    modified_seconds: i64,
    #[cfg(unix)]
    modified_nanoseconds: i64,
    #[cfg(unix)]
    changed_seconds: i64,
    #[cfg(unix)]
    changed_nanoseconds: i64,
    #[cfg(windows)]
    volume_serial_number: Option<u32>,
    #[cfg(windows)]
    file_index: Option<u64>,
    #[cfg(windows)]
    last_write_time: u64,
    #[cfg(windows)]
    change_time: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) enum FileIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows {
        volume_serial_number: u32,
        file_index: u64,
    },
}

impl FileSnapshot {
    pub fn capture(metadata: &Metadata) -> Self {
        #[cfg(unix)]
        use std::os::unix::fs::MetadataExt;
        #[cfg(windows)]
        use std::os::windows::fs::MetadataExt;

        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            is_file: metadata.is_file(),
            is_dir: metadata.is_dir(),
            #[cfg(unix)]
            device: metadata.dev(),
            #[cfg(unix)]
            inode: metadata.ino(),
            #[cfg(unix)]
            modified_seconds: metadata.mtime(),
            #[cfg(unix)]
            modified_nanoseconds: metadata.mtime_nsec(),
            #[cfg(unix)]
            changed_seconds: metadata.ctime(),
            #[cfg(unix)]
            changed_nanoseconds: metadata.ctime_nsec(),
            #[cfg(windows)]
            volume_serial_number: None,
            #[cfg(windows)]
            file_index: None,
            #[cfg(windows)]
            last_write_time: metadata.last_write_time(),
            #[cfg(windows)]
            change_time: 0,
        }
    }

    pub fn capture_path(metadata: &Metadata, path: &Path) -> io::Result<Self> {
        let snapshot = Self::capture(metadata);
        #[cfg(windows)]
        {
            let mut snapshot = snapshot;
            let identity = windows_snapshot_info_path(path)?;
            snapshot.change_time = identity.change_time;
            snapshot.volume_serial_number = Some(identity.volume_serial_number);
            snapshot.file_index = Some(identity.file_index);
            Ok(snapshot)
        }
        #[cfg(not(windows))]
        {
            let _ = path;
            Ok(snapshot)
        }
    }

    pub fn capture_file(metadata: &Metadata, file: &File) -> io::Result<Self> {
        let snapshot = Self::capture(metadata);
        #[cfg(windows)]
        {
            let mut snapshot = snapshot;
            let identity = windows_snapshot_info_file(file)?;
            snapshot.change_time = identity.change_time;
            snapshot.volume_serial_number = Some(identity.volume_serial_number);
            snapshot.file_index = Some(identity.file_index);
            Ok(snapshot)
        }
        #[cfg(not(windows))]
        {
            let _ = file;
            Ok(snapshot)
        }
    }

    pub fn matches_path(&self, metadata: &Metadata, path: &Path) -> io::Result<bool> {
        Ok(self == &Self::capture_path(metadata, path)?)
    }

    pub fn len(&self) -> u64 {
        self.len
    }

    #[cfg(unix)]
    pub fn store_identity(&self) -> (u64, u64, i64, i64, i64, i64) {
        (
            self.device,
            self.inode,
            self.modified_seconds,
            self.modified_nanoseconds,
            self.changed_seconds,
            self.changed_nanoseconds,
        )
    }

    #[cfg(windows)]
    pub fn store_identity(&self) -> (u64, u64, i64, i64, i64, i64) {
        let modified = i64::try_from(self.last_write_time).unwrap_or(i64::MAX);
        let (modified_seconds, modified_nanoseconds) = windows_time_parts(modified);
        let (changed_seconds, changed_nanoseconds) = windows_time_parts(self.change_time);
        (
            u64::from(self.volume_serial_number.unwrap_or_default()),
            self.file_index.unwrap_or_default(),
            modified_seconds,
            modified_nanoseconds,
            changed_seconds,
            changed_nanoseconds,
        )
    }

    #[cfg(not(any(unix, windows)))]
    pub fn store_identity(&self) -> (u64, u64, i64, i64, i64, i64) {
        use std::time::UNIX_EPOCH;

        let modified = self
            .modified
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok());
        (
            0,
            0,
            modified
                .and_then(|value| i64::try_from(value.as_secs()).ok())
                .unwrap_or_default(),
            modified.map_or(0, |value| i64::from(value.subsec_nanos())),
            0,
            0,
        )
    }

    #[cfg(unix)]
    pub fn identity(&self) -> Option<FileIdentity> {
        Some(FileIdentity::Unix {
            device: self.device,
            inode: self.inode,
        })
    }

    #[cfg(windows)]
    pub fn identity(&self) -> Option<FileIdentity> {
        Some(FileIdentity::Windows {
            volume_serial_number: self.volume_serial_number?,
            file_index: self.file_index?,
        })
    }

    #[cfg(not(any(unix, windows)))]
    pub fn identity(&self) -> Option<FileIdentity> {
        None
    }
}

#[cfg(windows)]
fn windows_time_parts(value: i64) -> (i64, i64) {
    const TICKS_PER_SECOND: i64 = 10_000_000;
    (
        value.div_euclid(TICKS_PER_SECOND),
        value.rem_euclid(TICKS_PER_SECOND).saturating_mul(100),
    )
}

#[cfg(windows)]
fn windows_snapshot_info_file(file: &File) -> io::Result<WindowsSnapshotInfo> {
    use std::os::windows::io::AsRawHandle;

    windows_snapshot_info_handle(file.as_raw_handle())
}

#[cfg(windows)]
fn windows_snapshot_info_path(path: &Path) -> io::Result<WindowsSnapshotInfo> {
    use std::os::windows::ffi::OsStrExt;
    use std::ptr;

    const FILE_READ_ATTRIBUTES: u32 = 0x0080;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    const FILE_SHARE_WRITE: u32 = 0x0000_0002;
    const FILE_SHARE_DELETE: u32 = 0x0000_0004;
    const OPEN_EXISTING: u32 = 3;
    const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null_mut(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let result = windows_snapshot_info_handle(handle);
    unsafe {
        CloseHandle(handle);
    }
    result
}

#[cfg(windows)]
fn windows_snapshot_info_handle(
    handle: std::os::windows::io::RawHandle,
) -> io::Result<WindowsSnapshotInfo> {
    use std::ffi::c_void;

    const FILE_BASIC_INFO_CLASS: i32 = 0;
    let mut basic = FileBasicInfo::default();
    let size = u32::try_from(std::mem::size_of::<FileBasicInfo>())
        .map_err(|_| io::Error::other("Windows file identity structure is too large"))?;
    let succeeded = unsafe {
        GetFileInformationByHandleEx(
            handle,
            FILE_BASIC_INFO_CLASS,
            (&mut basic as *mut FileBasicInfo).cast::<c_void>(),
            size,
        )
    };
    if succeeded == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut identity = ByHandleFileInformation::default();
    if unsafe { GetFileInformationByHandle(handle, &mut identity) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(WindowsSnapshotInfo {
        change_time: basic.change_time,
        volume_serial_number: identity.volume_serial_number,
        file_index: (u64::from(identity.file_index_high) << 32)
            | u64::from(identity.file_index_low),
    })
}

#[cfg(windows)]
struct WindowsSnapshotInfo {
    change_time: i64,
    volume_serial_number: u32,
    file_index: u64,
}

#[cfg(windows)]
#[derive(Default)]
#[repr(C)]
struct FileBasicInfo {
    creation_time: i64,
    last_access_time: i64,
    last_write_time: i64,
    change_time: i64,
    file_attributes: u32,
}

#[cfg(windows)]
#[derive(Default)]
#[repr(C)]
struct WindowsFileTime {
    low: u32,
    high: u32,
}

#[cfg(windows)]
#[derive(Default)]
#[repr(C)]
struct ByHandleFileInformation {
    file_attributes: u32,
    creation_time: WindowsFileTime,
    last_access_time: WindowsFileTime,
    last_write_time: WindowsFileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(windows)]
type WindowsHandle = *mut std::ffi::c_void;

#[cfg(windows)]
const INVALID_HANDLE_VALUE: WindowsHandle = -1isize as WindowsHandle;

#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateFileW(
        file_name: *const u16,
        desired_access: u32,
        share_mode: u32,
        security_attributes: *mut std::ffi::c_void,
        creation_disposition: u32,
        flags_and_attributes: u32,
        template_file: WindowsHandle,
    ) -> WindowsHandle;
    fn GetFileInformationByHandleEx(
        file: WindowsHandle,
        file_information_class: i32,
        file_information: *mut std::ffi::c_void,
        buffer_size: u32,
    ) -> i32;
    fn GetFileInformationByHandle(
        file: WindowsHandle,
        information: *mut ByHandleFileInformation,
    ) -> i32;
    fn CloseHandle(handle: WindowsHandle) -> i32;
}

pub(super) fn safe_source_cost(value: f64) -> Option<f64> {
    (value.is_finite() && (0.0..=MAX_SOURCE_COST_ESTIMATE).contains(&value)).then_some(value)
}

pub(super) fn safe_model_name(raw: &str) -> Option<String> {
    if raw.bytes().any(|byte| byte.is_ascii_uppercase()) {
        return None;
    }
    let token = safe_ascii_token(raw)?;
    let model = token
        .strip_prefix("us.anthropic.")
        .or_else(|| token.strip_prefix("anthropic."))
        .or_else(|| token.strip_prefix("anthropic/"))
        .or_else(|| token.strip_prefix("claude/"))
        .unwrap_or(&token);
    let segments = model.strip_prefix("claude-")?.split('-');
    segments
        .into_iter()
        .any(|segment| matches!(segment, "opus" | "sonnet" | "haiku" | "fable" | "mythos"))
        .then_some(token)
}

pub(super) fn classified_tool_name(raw: &str) -> (Option<String>, usize) {
    let Some(token) = safe_ascii_token(raw) else {
        return (None, 1);
    };
    if matches!(
        token.as_str(),
        "Bash"
            | "BashOutput"
            | "KillShell"
            | "Read"
            | "Write"
            | "Edit"
            | "MultiEdit"
            | "Glob"
            | "Grep"
            | "LS"
            | "Task"
            | "TaskOutput"
            | "WebFetch"
            | "WebSearch"
            | "NotebookEdit"
            | "TodoWrite"
            | "AskUserQuestion"
            | "Skill"
            | "EnterPlanMode"
            | "ExitPlanMode"
    ) {
        (Some(token), 0)
    } else if token.starts_with("mcp__") {
        (Some("mcp".to_string()), 1)
    } else {
        (Some("other".to_string()), 1)
    }
}

fn safe_ascii_token(raw: &str) -> Option<String> {
    if raw.is_empty()
        || raw.len() > SAFE_TOKEN_LIMIT
        || !raw.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'/')
        })
    {
        return None;
    }
    Some(raw.to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) enum EventKind {
    AssistantUsage,
    UserPrompt,
    ToolResult,
    Progress,
    Summary,
    System,
    Compaction,
    OtelApiRequest,
    OtelApiError,
    OtelToolResult,
    OtelToolDecision,
    OtelMetric,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct TokenFacts {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub cache_creation: Option<u64>,
    pub cache_read: Option<u64>,
    pub cache_creation_5m: Option<u64>,
    pub cache_creation_1h: Option<u64>,
}

impl TokenFacts {
    pub fn richness(&self) -> usize {
        [
            self.input,
            self.output,
            self.cache_creation,
            self.cache_read,
            self.cache_creation_5m,
            self.cache_creation_1h,
        ]
        .into_iter()
        .flatten()
        .count()
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct NormalizedEvent {
    pub schema_version: &'static str,
    pub adapter_version: &'static str,
    pub source_alias: String,
    pub file_alias: String,
    pub record_index: u64,
    pub timestamp: String,
    pub epoch_nanos: i128,
    pub timestamp_conversion_status: &'static str,
    pub project_key: u64,
    pub project_identity_present: bool,
    pub session_key: u64,
    pub session_identity_present: bool,
    pub message_key: Option<u64>,
    pub request_key: Option<u64>,
    pub parent_key: Option<u64>,
    pub agent_key: Option<u64>,
    pub parent_agent_key: Option<u64>,
    pub skill_key: Option<u64>,
    pub plugin_key: Option<u64>,
    pub mcp_server_key: Option<u64>,
    pub mcp_tool_key: Option<u64>,
    pub observation_key: u64,
    pub project_alias: String,
    pub session_alias: String,
    pub parent_session_alias: Option<String>,
    pub is_subagent: bool,
    pub is_sidechain: bool,
    pub kind: EventKind,
    pub model: Option<String>,
    pub model_mapping_status: &'static str,
    pub pricing_modifier: String,
    pub tokens: TokenFacts,
    pub source_cost_estimate: Option<f64>,
    pub tool_names: Vec<String>,
    pub tool_status: Option<String>,
    pub latency_ms: Option<f64>,
    pub error_count: Option<u64>,
    pub retry_count: Option<u64>,
    pub edit_decision: Option<String>,
    pub compaction: Option<bool>,
    pub metric_name: Option<&'static str>,
    pub metric_value: Option<f64>,
    pub metric_unit: Option<&'static str>,
    pub metric_interval_start_nanos: Option<u64>,
    pub metric_interval_end_nanos: Option<u64>,
    pub metric_temporality: Option<u64>,
    pub metric_family_key: Option<u64>,
    pub attribute_evidence_uncertain: bool,
    pub redacted_fields: usize,
}

impl NormalizedEvent {
    pub fn richness(&self) -> usize {
        self.tokens.richness()
            + usize::from(self.model.is_some())
            + usize::from(self.pricing_modifier != "standard")
            + usize::from(self.source_cost_estimate.is_some())
            + self.tool_names.len()
            + usize::from(self.tool_status.is_some())
            + usize::from(self.latency_ms.is_some())
            + usize::from(self.error_count.is_some())
            + usize::from(self.retry_count.is_some())
            + usize::from(self.edit_decision.is_some())
            + usize::from(self.compaction.is_some())
            + usize::from(self.metric_name.is_some())
            + usize::from(self.metric_value.is_some())
            + usize::from(self.metric_unit.is_some())
            + usize::from(self.metric_interval_start_nanos.is_some())
            + usize::from(self.metric_interval_end_nanos.is_some())
            + usize::from(self.metric_temporality.is_some())
            + usize::from(self.message_key.is_some())
            + usize::from(self.request_key.is_some())
    }

    pub fn dedup_key(&self, source_index: usize) -> DedupKey {
        let native_key = self
            .request_key
            .or(self.message_key)
            .unwrap_or(self.observation_key);
        DedupKey {
            source_index,
            project_key: self.project_key,
            project_identity_present: self.project_identity_present,
            session_key: self.session_key,
            native_key,
            epoch_nanos: self.epoch_nanos,
            kind: self.kind,
            is_sidechain: self.is_sidechain,
            is_subagent: self.is_subagent,
            parent_key: self.parent_key,
            agent_key: self.agent_key,
            parent_agent_key: self.parent_agent_key,
            skill_key: self.skill_key,
            plugin_key: self.plugin_key,
            mcp_server_key: self.mcp_server_key,
            mcp_tool_key: self.mcp_tool_key,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub(super) struct DedupKey {
    source_index: usize,
    project_key: u64,
    project_identity_present: bool,
    session_key: u64,
    native_key: u64,
    epoch_nanos: i128,
    kind: EventKind,
    is_sidechain: bool,
    is_subagent: bool,
    parent_key: Option<u64>,
    agent_key: Option<u64>,
    parent_agent_key: Option<u64>,
    skill_key: Option<u64>,
    plugin_key: Option<u64>,
    mcp_server_key: Option<u64>,
    mcp_tool_key: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct PrivatePrompt {
    pub project_alias: String,
    pub session_alias: String,
    pub timestamp: String,
    pub text: String,
    pub entrypoint: Option<String>,
}

#[derive(Debug)]
pub(super) struct PrivacyHasher {
    state: PrivacyHashState,
}

#[derive(Debug)]
enum PrivacyHashState {
    Ephemeral(std::collections::hash_map::RandomState),
    Persistent([u8; 32]),
}

impl PrivacyHasher {
    pub fn new() -> Self {
        Self {
            state: PrivacyHashState::Ephemeral(std::collections::hash_map::RandomState::new()),
        }
    }

    pub fn persistent(salt: [u8; 32]) -> Self {
        Self {
            state: PrivacyHashState::Persistent(salt),
        }
    }

    pub fn hash<T: Hash>(&self, value: &T) -> u64 {
        match &self.state {
            PrivacyHashState::Ephemeral(state) => state.hash_one(value),
            PrivacyHashState::Persistent(salt) => {
                let mut hasher = std::collections::hash_map::DefaultHasher::new();
                salt.hash(&mut hasher);
                value.hash(&mut hasher);
                hasher.finish()
            }
        }
    }

    pub fn store_salt(&self) -> Option<[u8; 32]> {
        match &self.state {
            PrivacyHashState::Ephemeral(_) => None,
            PrivacyHashState::Persistent(salt) => Some(*salt),
        }
    }
}

#[derive(Debug, Default)]
pub(super) struct AliasRegistry {
    projects: HashMap<u64, String>,
    sessions: HashMap<u64, String>,
    project_insertions: Vec<u64>,
    session_insertions: Vec<u64>,
    next_project: usize,
    next_session: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AliasState {
    projects: Vec<(u64, String)>,
    sessions: Vec<(u64, String)>,
    next_project: usize,
    next_session: usize,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct AliasCheckpoint {
    project_insertions: usize,
    session_insertions: usize,
    next_project: usize,
    next_session: usize,
}

impl AliasRegistry {
    pub fn snapshot(&self) -> AliasState {
        let mut projects = self
            .projects
            .iter()
            .map(|(key, alias)| (*key, alias.clone()))
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        let mut sessions = self
            .sessions
            .iter()
            .map(|(key, alias)| (*key, alias.clone()))
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| left.1.cmp(&right.1).then_with(|| left.0.cmp(&right.0)));
        AliasState {
            projects,
            sessions,
            next_project: self.next_project,
            next_session: self.next_session,
        }
    }

    pub fn restore(state: AliasState) -> Self {
        Self {
            projects: state.projects.into_iter().collect(),
            sessions: state.sessions.into_iter().collect(),
            project_insertions: Vec::new(),
            session_insertions: Vec::new(),
            next_project: state.next_project,
            next_session: state.next_session,
        }
    }

    pub fn existing_project(&self, key: u64) -> Option<&str> {
        self.projects.get(&key).map(String::as_str)
    }

    pub fn existing_session(&self, key: u64) -> Option<&str> {
        self.sessions.get(&key).map(String::as_str)
    }

    pub fn project(&mut self, key: u64) -> String {
        if let Some(alias) = self.projects.get(&key) {
            return alias.clone();
        }
        self.next_project = self.next_project.saturating_add(1);
        let alias = format!("project-{}", self.next_project);
        self.projects.insert(key, alias.clone());
        self.project_insertions.push(key);
        alias
    }

    pub fn session(&mut self, key: u64) -> String {
        if let Some(alias) = self.sessions.get(&key) {
            return alias.clone();
        }
        self.next_session = self.next_session.saturating_add(1);
        let alias = format!("session-{}", self.next_session);
        self.sessions.insert(key, alias.clone());
        self.session_insertions.push(key);
        alias
    }

    pub fn checkpoint(&self) -> AliasCheckpoint {
        AliasCheckpoint {
            project_insertions: self.project_insertions.len(),
            session_insertions: self.session_insertions.len(),
            next_project: self.next_project,
            next_session: self.next_session,
        }
    }

    pub fn rollback(&mut self, checkpoint: AliasCheckpoint) {
        while self.project_insertions.len() > checkpoint.project_insertions {
            if let Some(key) = self.project_insertions.pop() {
                self.projects.remove(&key);
            }
        }
        while self.session_insertions.len() > checkpoint.session_insertions {
            if let Some(key) = self.session_insertions.pop() {
                self.sessions.remove(&key);
            }
        }
        self.next_project = checkpoint.next_project;
        self.next_session = checkpoint.next_session;
    }
}

#[derive(Debug, Clone, Serialize)]
pub(super) struct SourceStats {
    pub alias: String,
    pub kind: String,
    pub selection: String,
    pub adapter_version: String,
    pub files_discovered: usize,
    pub accepted_records: usize,
    pub malformed_records: usize,
    pub unsupported_records: usize,
    pub unknown_records: usize,
    pub unknown_fields: usize,
    pub filtered_records: usize,
    pub redacted_fields: usize,
    pub duplicate_records: usize,
    pub skipped_records: usize,
    pub earliest: Option<(i128, String)>,
    pub latest: Option<(i128, String)>,
    pub capabilities: BTreeMap<String, String>,
    pub partial: bool,
    pub producer_contract: Option<String>,
    pub producer_verification: Option<String>,
}

impl SourceStats {
    pub fn transcript(alias: String, selection: String) -> Self {
        Self {
            alias,
            kind: "transcript".to_string(),
            selection,
            adapter_version: TRANSCRIPT_ADAPTER.to_string(),
            files_discovered: 0,
            accepted_records: 0,
            malformed_records: 0,
            unsupported_records: 0,
            unknown_records: 0,
            unknown_fields: 0,
            filtered_records: 0,
            redacted_fields: 0,
            duplicate_records: 0,
            skipped_records: 0,
            earliest: None,
            latest: None,
            capabilities: BTreeMap::new(),
            partial: false,
            producer_contract: None,
            producer_verification: None,
        }
    }

    pub fn otel(alias: String) -> Self {
        Self {
            alias,
            kind: "otel".to_string(),
            selection: "explicit-file".to_string(),
            adapter_version: OTEL_ADAPTER.to_string(),
            files_discovered: 1,
            accepted_records: 0,
            malformed_records: 0,
            unsupported_records: 0,
            unknown_records: 0,
            unknown_fields: 0,
            filtered_records: 0,
            redacted_fields: 0,
            duplicate_records: 0,
            skipped_records: 0,
            earliest: None,
            latest: None,
            capabilities: BTreeMap::new(),
            partial: false,
            producer_contract: Some(OTEL_CONTRACT.to_string()),
            producer_verification: Some("unverified".to_string()),
        }
    }

    pub fn observe_time(&mut self, epoch: i128, timestamp: &str) {
        if self
            .earliest
            .as_ref()
            .is_none_or(|(current, _)| epoch < *current)
        {
            self.earliest = Some((epoch, timestamp.to_string()));
        }
        if self
            .latest
            .as_ref()
            .is_none_or(|(current, _)| epoch > *current)
        {
            self.latest = Some((epoch, timestamp.to_string()));
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub(super) struct Diagnostics {
    pub source_root_count: usize,
    pub files_discovered: usize,
    pub accepted_records: usize,
    pub canonical_records: usize,
    pub malformed_records: usize,
    pub unsupported_records: usize,
    pub unknown_records: usize,
    pub unknown_fields: usize,
    pub filtered_records: usize,
    pub redacted_fields: usize,
    pub duplicate_records: usize,
    pub skipped_records: usize,
    pub resolved_overlap_records: usize,
    pub unresolved_overlap_records: usize,
    pub authority_excluded_records: usize,
    pub earliest: Option<(i128, String)>,
    pub latest: Option<(i128, String)>,
    pub sources: BTreeMap<String, SourceStats>,
    pub warnings: Vec<IngestionWarning>,
    pub unknown_shapes: Vec<UnknownShapeDiagnostic>,
    pub capabilities: BTreeMap<String, String>,
    pub saw_source_cost: bool,
    pub analytical_cost_coverage: Option<&'static str>,
    pub excluded_analysis_token_categories: u8,
    pub excluded_analysis_cost: bool,
    pub analytical_claims_uncertain: bool,
}

#[derive(Debug)]
pub(super) struct OtelDiagnosticsCheckpoint {
    source: Option<OtelSourceStatsCheckpoint>,
    malformed_records: usize,
    unsupported_records: usize,
    unknown_records: usize,
    unknown_fields: usize,
    filtered_records: usize,
    redacted_fields: usize,
    skipped_records: usize,
    earliest: Option<(i128, String)>,
    latest: Option<(i128, String)>,
    warnings_len: usize,
    unknown_shapes_len: usize,
    saw_source_cost: bool,
    analytical_claims_uncertain: bool,
}

#[derive(Debug, Clone, Copy)]
struct OtelSourceStatsCheckpoint {
    malformed_records: usize,
    unsupported_records: usize,
    unknown_records: usize,
    unknown_fields: usize,
    filtered_records: usize,
    redacted_fields: usize,
    duplicate_records: usize,
    skipped_records: usize,
    partial: bool,
}

impl From<&SourceStats> for OtelSourceStatsCheckpoint {
    fn from(source: &SourceStats) -> Self {
        Self {
            malformed_records: source.malformed_records,
            unsupported_records: source.unsupported_records,
            unknown_records: source.unknown_records,
            unknown_fields: source.unknown_fields,
            filtered_records: source.filtered_records,
            redacted_fields: source.redacted_fields,
            duplicate_records: source.duplicate_records,
            skipped_records: source.skipped_records,
            partial: source.partial,
        }
    }
}

impl Diagnostics {
    pub fn append_file_delta(&self, previous: &Self) -> Option<Self> {
        fn subtract(current: usize, previous: usize) -> Option<usize> {
            current.checked_sub(previous)
        }

        let mut delta = self.clone();
        delta.source_root_count = subtract(self.source_root_count, previous.source_root_count)?;
        delta.files_discovered = subtract(self.files_discovered, previous.files_discovered)?;
        delta.accepted_records = subtract(self.accepted_records, previous.accepted_records)?;
        delta.canonical_records = subtract(self.canonical_records, previous.canonical_records)?;
        delta.malformed_records = subtract(self.malformed_records, previous.malformed_records)?;
        delta.unsupported_records =
            subtract(self.unsupported_records, previous.unsupported_records)?;
        delta.unknown_records = subtract(self.unknown_records, previous.unknown_records)?;
        delta.unknown_fields = subtract(self.unknown_fields, previous.unknown_fields)?;
        delta.filtered_records = subtract(self.filtered_records, previous.filtered_records)?;
        delta.redacted_fields = subtract(self.redacted_fields, previous.redacted_fields)?;
        delta.duplicate_records = subtract(self.duplicate_records, previous.duplicate_records)?;
        delta.skipped_records = subtract(self.skipped_records, previous.skipped_records)?;
        delta.resolved_overlap_records = subtract(
            self.resolved_overlap_records,
            previous.resolved_overlap_records,
        )?;
        delta.unresolved_overlap_records = subtract(
            self.unresolved_overlap_records,
            previous.unresolved_overlap_records,
        )?;
        delta.authority_excluded_records = subtract(
            self.authority_excluded_records,
            previous.authority_excluded_records,
        )?;

        for (alias, source) in &mut delta.sources {
            let Some(previous_source) = previous.sources.get(alias) else {
                continue;
            };
            source.files_discovered =
                subtract(source.files_discovered, previous_source.files_discovered)?;
            source.accepted_records =
                subtract(source.accepted_records, previous_source.accepted_records)?;
            source.malformed_records =
                subtract(source.malformed_records, previous_source.malformed_records)?;
            source.unsupported_records = subtract(
                source.unsupported_records,
                previous_source.unsupported_records,
            )?;
            source.unknown_records =
                subtract(source.unknown_records, previous_source.unknown_records)?;
            source.unknown_fields =
                subtract(source.unknown_fields, previous_source.unknown_fields)?;
            source.filtered_records =
                subtract(source.filtered_records, previous_source.filtered_records)?;
            source.redacted_fields =
                subtract(source.redacted_fields, previous_source.redacted_fields)?;
            source.duplicate_records =
                subtract(source.duplicate_records, previous_source.duplicate_records)?;
            source.skipped_records =
                subtract(source.skipped_records, previous_source.skipped_records)?;
        }

        if previous.unknown_shapes.len() > delta.unknown_shapes.len() {
            return None;
        }
        delta.unknown_shapes.drain(..previous.unknown_shapes.len());
        Some(delta)
    }

    pub fn merge_file_parse(&mut self, mut other: Self) {
        self.files_discovered = self.files_discovered.saturating_add(other.files_discovered);
        self.accepted_records = self.accepted_records.saturating_add(other.accepted_records);
        self.canonical_records = self
            .canonical_records
            .saturating_add(other.canonical_records);
        self.malformed_records = self
            .malformed_records
            .saturating_add(other.malformed_records);
        self.unsupported_records = self
            .unsupported_records
            .saturating_add(other.unsupported_records);
        self.unknown_records = self.unknown_records.saturating_add(other.unknown_records);
        self.unknown_fields = self.unknown_fields.saturating_add(other.unknown_fields);
        self.filtered_records = self.filtered_records.saturating_add(other.filtered_records);
        self.redacted_fields = self.redacted_fields.saturating_add(other.redacted_fields);
        self.duplicate_records = self
            .duplicate_records
            .saturating_add(other.duplicate_records);
        self.skipped_records = self.skipped_records.saturating_add(other.skipped_records);
        self.resolved_overlap_records = self
            .resolved_overlap_records
            .saturating_add(other.resolved_overlap_records);
        self.unresolved_overlap_records = self
            .unresolved_overlap_records
            .saturating_add(other.unresolved_overlap_records);
        self.authority_excluded_records = self
            .authority_excluded_records
            .saturating_add(other.authority_excluded_records);

        if let Some((epoch, timestamp)) = other.earliest.take() {
            self.observe_time(epoch, &timestamp);
        }
        if let Some((epoch, timestamp)) = other.latest.take() {
            self.observe_time(epoch, &timestamp);
        }

        for (alias, source) in other.sources {
            let Some(current) = self.sources.get_mut(&alias) else {
                self.sources.insert(alias, source);
                continue;
            };
            current.files_discovered = current
                .files_discovered
                .saturating_add(source.files_discovered);
            current.accepted_records = current
                .accepted_records
                .saturating_add(source.accepted_records);
            current.malformed_records = current
                .malformed_records
                .saturating_add(source.malformed_records);
            current.unsupported_records = current
                .unsupported_records
                .saturating_add(source.unsupported_records);
            current.unknown_records = current
                .unknown_records
                .saturating_add(source.unknown_records);
            current.unknown_fields = current.unknown_fields.saturating_add(source.unknown_fields);
            current.filtered_records = current
                .filtered_records
                .saturating_add(source.filtered_records);
            current.redacted_fields = current
                .redacted_fields
                .saturating_add(source.redacted_fields);
            current.duplicate_records = current
                .duplicate_records
                .saturating_add(source.duplicate_records);
            current.skipped_records = current
                .skipped_records
                .saturating_add(source.skipped_records);
            if let Some((epoch, timestamp)) = source.earliest {
                current.observe_time(epoch, &timestamp);
            }
            if let Some((epoch, timestamp)) = source.latest {
                current.observe_time(epoch, &timestamp);
            }
            current.partial |= source.partial;
            for (capability, availability) in source.capabilities {
                current.capabilities.insert(capability, availability);
            }
        }

        for warning in other.warnings {
            if !self.warnings.iter().any(|existing| {
                existing.code == warning.code
                    && existing.message == warning.message
                    && existing.source_alias == warning.source_alias
            }) {
                self.warnings.push(warning);
            }
        }
        let remaining = MAX_UNKNOWN_SHAPE_DIAGNOSTICS.saturating_sub(self.unknown_shapes.len());
        let samples_truncated = other.unknown_shapes.len() > remaining;
        self.unknown_shapes
            .extend(other.unknown_shapes.into_iter().take(remaining));
        if samples_truncated
            && !self.warnings.iter().any(|warning| {
                warning.code == "W_UNKNOWN_SHAPE_SAMPLES_TRUNCATED"
                    && warning.source_alias.is_none()
            })
        {
            self.warning(
                "W_UNKNOWN_SHAPE_SAMPLES_TRUNCATED",
                "Unknown-shape samples reached the bounded diagnostic limit; aggregate counts remain complete.",
                None,
            );
        }
        for (capability, availability) in other.capabilities {
            self.capabilities.insert(capability, availability);
        }
        self.saw_source_cost |= other.saw_source_cost;
        self.analytical_cost_coverage = self
            .analytical_cost_coverage
            .or(other.analytical_cost_coverage);
        self.excluded_analysis_token_categories |= other.excluded_analysis_token_categories;
        self.excluded_analysis_cost |= other.excluded_analysis_cost;
        self.analytical_claims_uncertain |= other.analytical_claims_uncertain;
    }

    pub fn checkpoint_otel_line(&self, source_alias: &str) -> OtelDiagnosticsCheckpoint {
        OtelDiagnosticsCheckpoint {
            source: self.sources.get(source_alias).map(Into::into),
            malformed_records: self.malformed_records,
            unsupported_records: self.unsupported_records,
            unknown_records: self.unknown_records,
            unknown_fields: self.unknown_fields,
            filtered_records: self.filtered_records,
            redacted_fields: self.redacted_fields,
            skipped_records: self.skipped_records,
            earliest: self.earliest.clone(),
            latest: self.latest.clone(),
            warnings_len: self.warnings.len(),
            unknown_shapes_len: self.unknown_shapes.len(),
            saw_source_cost: self.saw_source_cost,
            analytical_claims_uncertain: self.analytical_claims_uncertain,
        }
    }

    pub fn rollback_otel_line(
        &mut self,
        source_alias: &str,
        checkpoint: OtelDiagnosticsCheckpoint,
    ) {
        self.malformed_records = checkpoint.malformed_records;
        self.unsupported_records = checkpoint.unsupported_records;
        self.unknown_records = checkpoint.unknown_records;
        self.unknown_fields = checkpoint.unknown_fields;
        self.filtered_records = checkpoint.filtered_records;
        self.redacted_fields = checkpoint.redacted_fields;
        self.skipped_records = checkpoint.skipped_records;
        self.earliest = checkpoint.earliest;
        self.latest = checkpoint.latest;
        self.warnings.truncate(checkpoint.warnings_len);
        self.unknown_shapes.truncate(checkpoint.unknown_shapes_len);
        self.saw_source_cost = checkpoint.saw_source_cost;
        self.analytical_claims_uncertain = checkpoint.analytical_claims_uncertain;
        if let (Some(source), Some(current)) =
            (checkpoint.source, self.sources.get_mut(source_alias))
        {
            current.malformed_records = source.malformed_records;
            current.unsupported_records = source.unsupported_records;
            current.unknown_records = source.unknown_records;
            current.unknown_fields = source.unknown_fields;
            current.filtered_records = source.filtered_records;
            current.redacted_fields = source.redacted_fields;
            current.duplicate_records = source.duplicate_records;
            current.skipped_records = source.skipped_records;
            current.partial = source.partial;
        }
    }

    pub fn warning(
        &mut self,
        code: impl Into<String>,
        message: impl Into<String>,
        source_alias: Option<String>,
    ) {
        self.warnings.push(IngestionWarning {
            code: code.into(),
            message: message.into(),
            source_alias,
        });
    }

    pub fn observe_time(&mut self, epoch: i128, timestamp: &str) {
        if self
            .earliest
            .as_ref()
            .is_none_or(|(current, _)| epoch < *current)
        {
            self.earliest = Some((epoch, timestamp.to_string()));
        }
        if self
            .latest
            .as_ref()
            .is_none_or(|(current, _)| epoch > *current)
        {
            self.latest = Some((epoch, timestamp.to_string()));
        }
    }

    pub fn finalize(
        mut self,
        time_context: &super::TimeContext,
        timezone_fallback: bool,
    ) -> DataCoverage {
        if timezone_fallback {
            self.warning(
                "W_TIMEZONE_DEFAULTED_TO_UTC",
                "The host IANA timezone could not be resolved; UTC was selected explicitly for this report.",
                None,
            );
        }
        self.warnings.sort_by(|left, right| {
            left.source_alias
                .cmp(&right.source_alias)
                .then_with(|| left.code.cmp(&right.code))
                .then_with(|| left.message.cmp(&right.message))
        });

        let has_partial = self.malformed_records > 0
            || self.unsupported_records > 0
            || self.unknown_records > 0
            || self.unknown_fields > 0
            || self.skipped_records > 0
            || self.unresolved_overlap_records > 0
            || self.sources.values().any(|source| source.partial)
            || self
                .warnings
                .iter()
                .any(|warning| warning.code.starts_with("W_DISCOVERY_"));
        let completeness = if self.accepted_records == 0 && has_partial {
            "indeterminate"
        } else if self.accepted_records == 0 {
            "empty"
        } else if has_partial {
            "partial"
        } else if self
            .sources
            .values()
            .any(|source| source.kind == "transcript")
        {
            "indeterminate"
        } else {
            "complete"
        };

        let observed_day_span = match (&self.earliest, &self.latest) {
            (Some((first, _)), Some((last, _))) => time_context.observed_day_span(*first, *last),
            _ => 0,
        };

        let mut sources = self
            .sources
            .into_values()
            .map(|source| {
                let is_transcript = source.kind == "transcript";
                let is_empty = source.accepted_records == 0;
                SourceCoverage {
                    alias: source.alias,
                    kind: source.kind,
                    selection: source.selection,
                    files_discovered: source.files_discovered,
                    accepted_records: source.accepted_records,
                    classified_records: classified_record_count(
                        source.accepted_records,
                        source.malformed_records,
                        source.unsupported_records,
                        source.filtered_records,
                        source.skipped_records,
                        source.duplicate_records,
                    ),
                    malformed_records: source.malformed_records,
                    unsupported_records: source.unsupported_records,
                    unknown_records: source.unknown_records,
                    unknown_fields: source.unknown_fields,
                    filtered_records: source.filtered_records,
                    redacted_fields: source.redacted_fields,
                    duplicate_records: source.duplicate_records,
                    skipped_records: source.skipped_records,
                    earliest_observed_at: source.earliest.map(|(_, value)| value),
                    latest_observed_at: source.latest.map(|(_, value)| value),
                    capabilities: source.capabilities,
                    completeness: if source.partial && is_empty {
                        "indeterminate".to_string()
                    } else if source.partial {
                        "partial".to_string()
                    } else if is_empty {
                        "empty".to_string()
                    } else if is_transcript {
                        "indeterminate".to_string()
                    } else {
                        "complete".to_string()
                    },
                    adapter_version: source.adapter_version,
                    producer_contract: source.producer_contract,
                    producer_verification: source.producer_verification,
                }
            })
            .collect::<Vec<_>>();
        sources.sort_by_key(|source| source_order_key(&source.alias));

        DataCoverage {
            selected_period: time_context
                .year()
                .map_or_else(|| "all".to_string(), |year| year.to_string()),
            timezone: time_context.name().to_string(),
            earliest_observed_at: self.earliest.map(|(_, value)| value),
            latest_observed_at: self.latest.map(|(_, value)| value),
            observed_day_span,
            source_root_count: self.source_root_count,
            files_discovered: self.files_discovered,
            accepted_records: self.accepted_records,
            canonical_records: self.canonical_records,
            classified_records: classified_record_count(
                self.accepted_records,
                self.malformed_records,
                self.unsupported_records,
                self.filtered_records,
                self.skipped_records,
                self.duplicate_records,
            ),
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
            record_count_invariant: "classifiedRecords = acceptedRecords + malformedRecords + unsupportedRecords + filteredRecords + skippedRecords + duplicateRecords; unknown/redacted/overlap counts are orthogonal".to_string(),
            completeness: completeness.to_string(),
            retention_caveat: "Local transcript retention may omit earlier activity; observed history is not billing- or account-complete.".to_string(),
            cost_coverage: self
                .analytical_cost_coverage
                .unwrap_or("unavailable-incomplete-usage")
                .to_string(),
            privacy_profile: "standard".to_string(),
            authority_policy_version: "authority/v1".to_string(),
            capabilities: self.capabilities,
            sources,
            warnings: self.warnings,
            unknown_shapes: self.unknown_shapes,
        }
    }
}

fn classified_record_count(
    accepted: usize,
    malformed: usize,
    unsupported: usize,
    filtered: usize,
    skipped: usize,
    duplicate: usize,
) -> usize {
    [
        accepted,
        malformed,
        unsupported,
        filtered,
        skipped,
        duplicate,
    ]
    .into_iter()
    .fold(0usize, usize::saturating_add)
}

fn source_order_key(alias: &str) -> (u8, usize, String) {
    let (kind, suffix) = alias.rsplit_once('-').unwrap_or((alias, "0"));
    let rank = match kind {
        "transcript" => 0,
        "otel" => 1,
        "git" => 2,
        _ => 3,
    };
    (
        rank,
        suffix.parse().unwrap_or(usize::MAX),
        alias.to_string(),
    )
}
