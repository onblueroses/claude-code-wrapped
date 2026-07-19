use blake3::Hasher;
use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

const SCHEMA: &str = "ccwrapped.phase5-sqlite-prototype/v1";
const STORE_SCHEMA_VERSION: i64 = 1;
const BUFFER_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    FirstImport,
    Warm,
}

#[derive(Debug)]
pub struct Options {
    pub mode: Mode,
    pub binary: PathBuf,
    pub corpus: PathBuf,
    pub store: PathBuf,
    pub scratch: PathBuf,
    pub worker_count: usize,
}

#[derive(Debug)]
struct SourceFile {
    path: PathBuf,
    kind: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Snapshot {
    device: i64,
    inode: i64,
    size: i64,
    modified_seconds: i64,
    modified_nanoseconds: i64,
    changed_seconds: i64,
    changed_nanoseconds: i64,
}

pub fn run(options: Options) -> Result<String, String> {
    validate(&options)?;
    match options.mode {
        Mode::FirstImport => first_import(&options),
        Mode::Warm => warm(&options),
    }
}

fn validate(options: &Options) -> Result<(), String> {
    if options.worker_count == 0 {
        return Err("prototype worker count must be positive".to_string());
    }
    if options.mode == Mode::FirstImport && options.store.exists() {
        return Err(format!(
            "prototype store already exists: {}",
            options.store.display()
        ));
    }
    if options.mode == Mode::Warm && !options.store.is_file() {
        return Err(format!(
            "prototype store is absent: {}",
            options.store.display()
        ));
    }
    Ok(())
}

fn first_import(options: &Options) -> Result<String, String> {
    let started = Instant::now();
    let corpus = fs::canonicalize(&options.corpus)
        .map_err(|error| format!("resolve corpus {}: {error}", options.corpus.display()))?;
    let files = source_files(&corpus)?;
    prepare_private_directory(
        options
            .store
            .parent()
            .ok_or_else(|| "prototype store has no parent directory".to_string())?,
    )?;
    prepare_private_directory(&options.scratch)?;
    create_private_file(&options.store)?;
    let report = run_product(options, &corpus)?;
    let salt = random_key()?;
    let report_digest = blake3::hash(&report);

    let mut connection = open_store(&options.store)?;
    configure_store(&connection)?;
    connection
        .execute_batch(
            "
            BEGIN IMMEDIATE;
            CREATE TABLE meta (
                key TEXT PRIMARY KEY,
                value BLOB NOT NULL
            ) STRICT;
            CREATE TABLE source_file (
                path_key BLOB PRIMARY KEY,
                source_kind INTEGER NOT NULL,
                device INTEGER NOT NULL,
                inode INTEGER NOT NULL,
                size INTEGER NOT NULL,
                modified_seconds INTEGER NOT NULL,
                modified_nanoseconds INTEGER NOT NULL,
                changed_seconds INTEGER NOT NULL,
                changed_nanoseconds INTEGER NOT NULL,
                content_digest BLOB NOT NULL
            ) STRICT, WITHOUT ROWID;
            CREATE TABLE cached_report (
                singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
                report_json BLOB NOT NULL,
                report_digest BLOB NOT NULL
            ) STRICT;
            COMMIT;
            ",
        )
        .map_err(|error| format!("create prototype schema: {error}"))?;
    connection
        .pragma_update(None, "user_version", STORE_SCHEMA_VERSION)
        .map_err(|error| format!("set prototype schema version: {error}"))?;

    let transaction = connection
        .transaction()
        .map_err(|error| format!("begin prototype import transaction: {error}"))?;
    transaction
        .execute(
            "INSERT INTO meta (key, value) VALUES ('path-key-salt', ?1)",
            params![salt.as_slice()],
        )
        .map_err(|error| format!("store prototype salt: {error}"))?;
    transaction
        .execute(
            "INSERT INTO meta (key, value) VALUES ('adapter-set', ?1)",
            params![b"normalized-event/v1|transcript/v1|otel/v1".as_slice()],
        )
        .map_err(|error| format!("store prototype adapter set: {error}"))?;
    {
        let mut insert = transaction
            .prepare(
                "
                INSERT INTO source_file (
                    path_key, source_kind, device, inode, size,
                    modified_seconds, modified_nanoseconds,
                    changed_seconds, changed_nanoseconds, content_digest
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                ",
            )
            .map_err(|error| format!("prepare prototype file insert: {error}"))?;
        for source in &files {
            let metadata = fs::metadata(&source.path)
                .map_err(|error| format!("inspect {}: {error}", source.path.display()))?;
            let snapshot = Snapshot::capture(&metadata)?;
            let path_key = path_key(&salt, &source.path);
            let digest = content_digest(&salt, &source.path)?;
            insert
                .execute(params![
                    path_key.as_bytes().as_slice(),
                    source.kind,
                    snapshot.device,
                    snapshot.inode,
                    snapshot.size,
                    snapshot.modified_seconds,
                    snapshot.modified_nanoseconds,
                    snapshot.changed_seconds,
                    snapshot.changed_nanoseconds,
                    digest.as_bytes().as_slice(),
                ])
                .map_err(|error| format!("store prototype source metadata: {error}"))?;
        }
    }
    transaction
        .execute(
            "
            INSERT INTO cached_report (singleton, report_json, report_digest)
            VALUES (1, ?1, ?2)
            ",
            params![report.as_slice(), report_digest.as_bytes().as_slice()],
        )
        .map_err(|error| format!("store prototype report: {error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("commit prototype import: {error}"))?;
    connection
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|error| format!("checkpoint prototype store: {error}"))?;
    drop(connection);
    enforce_private_file(&options.store)?;

    let wall_nanos = started.elapsed().as_nanos();
    let store_bytes = allocated_store_bytes(&options.store)?;
    Ok(summary(
        "first-import",
        wall_nanos,
        files.len(),
        files
            .iter()
            .map(|source| fs::metadata(&source.path).map(|metadata| metadata.len()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("remeasure prototype source bytes: {error}"))?
            .into_iter()
            .fold(0u64, u64::saturating_add),
        report.len(),
        report_digest.to_hex().as_str(),
        store_bytes,
        false,
    ))
}

fn warm(options: &Options) -> Result<String, String> {
    let started = Instant::now();
    let corpus = fs::canonicalize(&options.corpus)
        .map_err(|error| format!("resolve corpus {}: {error}", options.corpus.display()))?;
    let files = source_files(&corpus)?;
    let connection = open_store(&options.store)?;
    configure_read(&connection)?;
    let version: i64 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| format!("read prototype schema version: {error}"))?;
    if version != STORE_SCHEMA_VERSION {
        return Err(format!(
            "prototype schema version {version} is not {STORE_SCHEMA_VERSION}"
        ));
    }
    let salt_blob: Vec<u8> = connection
        .query_row(
            "SELECT value FROM meta WHERE key = 'path-key-salt'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("read prototype salt: {error}"))?;
    let salt: [u8; 32] = salt_blob
        .try_into()
        .map_err(|_| "prototype salt is not 32 bytes".to_string())?;
    let stored_count: usize = connection
        .query_row("SELECT count(*) FROM source_file", [], |row| row.get(0))
        .map_err(|error| format!("count prototype source files: {error}"))?;
    let mut reusable = stored_count == files.len();
    let mut source_bytes = 0u64;
    let mut select = connection
        .prepare(
            "
            SELECT device, inode, size, modified_seconds, modified_nanoseconds,
                   changed_seconds, changed_nanoseconds
            FROM source_file
            WHERE path_key = ?1 AND source_kind = ?2
            ",
        )
        .map_err(|error| format!("prepare prototype snapshot query: {error}"))?;
    for source in &files {
        let metadata = fs::metadata(&source.path)
            .map_err(|error| format!("inspect {}: {error}", source.path.display()))?;
        source_bytes = source_bytes.saturating_add(metadata.len());
        let current = Snapshot::capture(&metadata)?;
        let key = path_key(&salt, &source.path);
        let stored = select
            .query_row(params![key.as_bytes().as_slice(), source.kind], |row| {
                Ok(Snapshot {
                    device: row.get(0)?,
                    inode: row.get(1)?,
                    size: row.get(2)?,
                    modified_seconds: row.get(3)?,
                    modified_nanoseconds: row.get(4)?,
                    changed_seconds: row.get(5)?,
                    changed_nanoseconds: row.get(6)?,
                })
            })
            .optional()
            .map_err(|error| format!("read prototype snapshot: {error}"))?;
        reusable &= stored == Some(current);
    }
    if !reusable {
        return Err("prototype source snapshot changed; a rebuild is required".to_string());
    }
    let (report, stored_digest): (Vec<u8>, Vec<u8>) = connection
        .query_row(
            "SELECT report_json, report_digest FROM cached_report WHERE singleton = 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| format!("read prototype report: {error}"))?;
    let digest = blake3::hash(&report);
    if stored_digest.as_slice() != digest.as_bytes() {
        return Err("prototype cached report digest does not match".to_string());
    }
    let wall_nanos = started.elapsed().as_nanos();
    let store_bytes = allocated_store_bytes(&options.store)?;
    Ok(summary(
        "warm",
        wall_nanos,
        files.len(),
        source_bytes,
        report.len(),
        digest.to_hex().as_str(),
        store_bytes,
        true,
    ))
}

fn run_product(options: &Options, corpus: &Path) -> Result<Vec<u8>, String> {
    let binary = fs::canonicalize(&options.binary)
        .map_err(|error| format!("resolve binary {}: {error}", options.binary.display()))?;
    let projects = corpus.join("projects");
    let otel_files = collect_files(&corpus.join("otel"))?;
    let home = options.scratch.join("home");
    let config = options.scratch.join("config");
    fs::create_dir_all(&home)
        .and_then(|()| fs::create_dir_all(&config))
        .map_err(|error| format!("create prototype process isolation: {error}"))?;
    let mut command = Command::new(binary);
    command
        .args(["--timezone", "UTC", "--data-dir"])
        .arg(&projects)
        .args(["--ingestion-workers", &options.worker_count.to_string()]);
    for path in otel_files {
        command.arg("--otel-file").arg(path);
    }
    let output = command
        .args(["--json", "2026"])
        .current_dir(&options.scratch)
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &config)
        .env("NO_COLOR", "1")
        .env("RUST_BACKTRACE", "0")
        .stderr(Stdio::piped())
        .output()
        .map_err(|error| format!("run prototype first-import product: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "prototype first-import product failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    if !output.stderr.is_empty() {
        return Err("prototype first-import product wrote unexpected stderr".to_string());
    }
    Ok(output.stdout)
}

fn source_files(corpus: &Path) -> Result<Vec<SourceFile>, String> {
    let mut files = collect_files(&corpus.join("projects"))?
        .into_iter()
        .map(|path| SourceFile { path, kind: 1 })
        .collect::<Vec<_>>();
    files.extend(
        collect_files(&corpus.join("otel"))?
            .into_iter()
            .map(|path| SourceFile { path, kind: 2 }),
    );
    files.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(files)
}

fn collect_files(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files_recursive(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files_recursive(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
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
            collect_files_recursive(&path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn open_store(path: &Path) -> Result<Connection, String> {
    Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| format!("open prototype store {}: {error}", path.display()))
}

fn configure_store(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = FULL;
            PRAGMA foreign_keys = ON;
            PRAGMA temp_store = MEMORY;
            PRAGMA trusted_schema = OFF;
            ",
        )
        .map_err(|error| format!("configure prototype store: {error}"))
}

fn configure_read(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;
            PRAGMA query_only = ON;
            PRAGMA trusted_schema = OFF;
            ",
        )
        .map_err(|error| format!("configure prototype read: {error}"))
}

fn random_key() -> Result<[u8; 32], String> {
    let mut key = [0u8; 32];
    File::open("/dev/urandom")
        .and_then(|mut file| file.read_exact(&mut key))
        .map_err(|error| format!("read prototype salt: {error}"))?;
    Ok(key)
}

fn path_key(key: &[u8; 32], path: &Path) -> blake3::Hash {
    let mut hasher = Hasher::new_keyed(key);
    hasher.update(b"ccwrapped-path-key/v1\0");
    update_path(&mut hasher, path);
    hasher.finalize()
}

fn content_digest(key: &[u8; 32], path: &Path) -> Result<blake3::Hash, String> {
    let mut hasher = Hasher::new_keyed(key);
    hasher.update(b"ccwrapped-content-digest/v1\0");
    let mut file =
        File::open(path).map_err(|error| format!("open {} for digest: {error}", path.display()))?;
    let mut buffer = vec![0u8; BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("digest {}: {error}", path.display()))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize())
}

fn update_path(hasher: &mut Hasher, path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        hasher.update(path.as_os_str().as_bytes());
    }
    #[cfg(not(unix))]
    {
        hasher.update(path.as_os_str().to_string_lossy().as_bytes());
    }
}

impl Snapshot {
    fn capture(metadata: &fs::Metadata) -> Result<Self, String> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: i64::try_from(metadata.dev())
                    .map_err(|_| "prototype device id exceeds SQLite INTEGER".to_string())?,
                inode: i64::try_from(metadata.ino())
                    .map_err(|_| "prototype inode exceeds SQLite INTEGER".to_string())?,
                size: i64::try_from(metadata.len())
                    .map_err(|_| "prototype file size exceeds SQLite INTEGER".to_string())?,
                modified_seconds: metadata.mtime(),
                modified_nanoseconds: metadata.mtime_nsec(),
                changed_seconds: metadata.ctime(),
                changed_nanoseconds: metadata.ctime_nsec(),
            })
        }
        #[cfg(not(unix))]
        {
            let modified = metadata
                .modified()
                .ok()
                .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok());
            Ok(Self {
                device: 0,
                inode: 0,
                size: i64::try_from(metadata.len())
                    .map_err(|_| "prototype file size exceeds SQLite INTEGER".to_string())?,
                modified_seconds: modified
                    .and_then(|value| i64::try_from(value.as_secs()).ok())
                    .unwrap_or(0),
                modified_nanoseconds: modified.map_or(0, |value| i64::from(value.subsec_nanos())),
                changed_seconds: 0,
                changed_nanoseconds: 0,
            })
        }
    }
}

fn prepare_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("create private directory {}: {error}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("protect directory {}: {error}", path.display()))?;
    }
    Ok(())
}

fn create_private_file(path: &Path) -> Result<(), String> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("create private store {}: {error}", path.display()))?;
    file.flush()
        .map_err(|error| format!("initialize private store {}: {error}", path.display()))
}

fn enforce_private_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("protect store {}: {error}", path.display()))?;
    }
    Ok(())
}

fn allocated_store_bytes(path: &Path) -> Result<u64, String> {
    let mut bytes = 0u64;
    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
        PathBuf::from(format!("{}-journal", path.display())),
    ] {
        match fs::metadata(&candidate) {
            Ok(metadata) => bytes = bytes.saturating_add(metadata.len()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "inspect prototype allocation {}: {error}",
                    candidate.display()
                ))
            }
        }
    }
    Ok(bytes)
}

#[allow(clippy::too_many_arguments)]
fn summary(
    mode: &str,
    wall_nanos: u128,
    source_files: usize,
    source_bytes: u64,
    report_bytes: usize,
    report_digest: &str,
    store_bytes: u64,
    cache_hit: bool,
) -> String {
    format!(
        concat!(
            "{{\n",
            "  \"schema\": \"{}\",\n",
            "  \"mode\": \"{}\",\n",
            "  \"wallNanos\": {},\n",
            "  \"sourceFiles\": {},\n",
            "  \"sourceBytes\": {},\n",
            "  \"reportBytes\": {},\n",
            "  \"reportBlake3\": \"{}\",\n",
            "  \"storeBytes\": {},\n",
            "  \"cacheHit\": {}\n",
            "}}\n"
        ),
        SCHEMA,
        mode,
        wall_nanos,
        source_files,
        source_bytes,
        report_bytes,
        report_digest,
        store_bytes,
        cache_hit,
    )
}
