use std::fs::{self, File};
use std::io::{BufReader, Read};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

const MEASUREMENT_SCHEMA: &str = "ccwrapped.phase5-measurement/v1";
const READER_SCHEMA: &str = "ccwrapped.phase5-reader/v1";
const MEMORY_SCHEMA: &str = "ccwrapped.phase5-memory/v1";
const SAMPLER_OVERHEAD_SCHEMA: &str = "ccwrapped.phase5-sampler-overhead/v1";
const MAXIMUM_TIMEOUT_SECONDS: u64 = 300;
const MAXIMUM_MEMORY_BASELINE_ALLOCATION: u64 = 4 * 1024 * 1024 * 1024;

#[derive(Debug)]
pub struct MeasureOptions {
    pub binary: PathBuf,
    pub corpus: PathBuf,
    pub stderr: PathBuf,
    pub scratch: PathBuf,
    pub sample_interval: Duration,
    pub timeout: Duration,
    pub worker_count: Option<usize>,
    pub store: Option<PathBuf>,
}

#[derive(Debug)]
struct ProcSample {
    elapsed_nanos: u128,
    state: char,
    user_ticks: u64,
    system_ticks: u64,
    threads: u64,
    rss_bytes: u64,
    peak_rss_bytes: u64,
    logical_read_bytes: u64,
    logical_write_bytes: u64,
    physical_read_bytes: u64,
    physical_write_bytes: u64,
}

impl ProcSample {
    fn read(pid: u32, elapsed_nanos: u128, page_size: u64) -> Result<Self, String> {
        let root = PathBuf::from(format!("/proc/{pid}"));
        let stat = fs::read_to_string(root.join("stat"))
            .map_err(|error| format!("read process stat: {error}"))?;
        let parsed_stat = parse_proc_stat(&stat, page_size)?;
        let status = fs::read_to_string(root.join("status"))
            .map_err(|error| format!("read process status: {error}"))?;
        let io = fs::read_to_string(root.join("io"))
            .map_err(|error| format!("read process I/O counters: {error}"))?;

        Ok(Self {
            elapsed_nanos,
            state: parsed_stat.state,
            user_ticks: parsed_stat.user_ticks,
            system_ticks: parsed_stat.system_ticks,
            threads: parsed_stat.threads,
            rss_bytes: named_kib(&status, "VmRSS").unwrap_or(parsed_stat.rss_bytes),
            peak_rss_bytes: named_kib(&status, "VmHWM").unwrap_or(parsed_stat.rss_bytes),
            logical_read_bytes: named_u64(&io, "rchar").unwrap_or(0),
            logical_write_bytes: named_u64(&io, "wchar").unwrap_or(0),
            physical_read_bytes: named_u64(&io, "read_bytes").unwrap_or(0),
            physical_write_bytes: named_u64(&io, "write_bytes").unwrap_or(0),
        })
    }

    fn total_ticks(&self) -> u64 {
        self.user_ticks.saturating_add(self.system_ticks)
    }

    fn json(&self) -> String {
        format!(
            concat!(
                "{{\"elapsedNanos\":{},\"state\":\"{}\",",
                "\"userTicks\":{},\"systemTicks\":{},\"threads\":{},",
                "\"rssBytes\":{},\"peakRssBytes\":{},",
                "\"logicalReadBytes\":{},\"logicalWriteBytes\":{},",
                "\"physicalReadBytes\":{},\"physicalWriteBytes\":{}}}"
            ),
            self.elapsed_nanos,
            self.state,
            self.user_ticks,
            self.system_ticks,
            self.threads,
            self.rss_bytes,
            self.peak_rss_bytes,
            self.logical_read_bytes,
            self.logical_write_bytes,
            self.physical_read_bytes,
            self.physical_write_bytes,
        )
    }
}

#[derive(Debug)]
struct ParsedStat {
    state: char,
    user_ticks: u64,
    system_ticks: u64,
    threads: u64,
    rss_bytes: u64,
}

pub fn measure(options: MeasureOptions) -> Result<String, String> {
    validate_measure_options(&options)?;
    let binary = fs::canonicalize(&options.binary)
        .map_err(|error| format!("resolve binary {}: {error}", options.binary.display()))?;
    let corpus = fs::canonicalize(&options.corpus)
        .map_err(|error| format!("resolve corpus {}: {error}", options.corpus.display()))?;
    let projects = corpus.join("projects");
    if !projects.is_dir() {
        return Err(format!(
            "corpus {} has no projects directory",
            corpus.display()
        ));
    }
    let otel_files = source_files_under(&corpus.join("otel"))?;

    fs::create_dir_all(&options.scratch)
        .map_err(|error| format!("create scratch {}: {error}", options.scratch.display()))?;
    let home = options.scratch.join("home");
    let config = options.scratch.join("config");
    let stage_counters = options.scratch.join("stage-counters.json");
    if stage_counters
        .try_exists()
        .map_err(|error| format!("inspect benchmark counter path: {error}"))?
    {
        return Err(format!(
            "benchmark counter path already exists: {}",
            stage_counters.display()
        ));
    }
    fs::create_dir_all(&home)
        .and_then(|()| fs::create_dir_all(&config))
        .map_err(|error| format!("create isolated process directories: {error}"))?;
    let stderr = File::create(&options.stderr)
        .map_err(|error| format!("create stderr log {}: {error}", options.stderr.display()))?;

    let clock_ticks = getconf_u64("CLK_TCK")?;
    let page_size = getconf_u64("PAGESIZE")?;
    let mut command = Command::new(binary);
    command
        .arg("--timezone")
        .arg("UTC")
        .arg("--data-dir")
        .arg(&projects);
    if let Some(worker_count) = options.worker_count {
        command
            .arg("--ingestion-workers")
            .arg(worker_count.to_string());
    }
    for path in &otel_files {
        command.arg("--otel-file").arg(path);
    }
    let mode = if let Some(store) = &options.store {
        command.arg("--store-path").arg(store);
        "store"
    } else {
        command.arg("--no-store");
        "no-store"
    };
    command
        .arg("--json")
        .arg("--benchmark-counters")
        .arg(&stage_counters)
        .arg("2026")
        .current_dir(&options.scratch)
        .env("HOME", &home)
        .env("CLAUDE_CONFIG_DIR", &config)
        .env("NO_COLOR", "1")
        .env("RUST_BACKTRACE", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr));
    let started = Instant::now();
    let mut child = command
        .spawn()
        .map_err(|error| format!("spawn measured ccwrapped process: {error}"))?;
    let pid = child.id();
    let mut samples = Vec::new();
    let mut timed_out = false;
    let mut reaped_status = None;

    loop {
        let elapsed = started.elapsed();
        match ProcSample::read(pid, elapsed.as_nanos(), page_size) {
            Ok(sample) => {
                let exited = matches!(sample.state, 'Z' | 'X' | 'x');
                samples.push(sample);
                if exited {
                    break;
                }
            }
            Err(error) => match child.try_wait().map_err(|wait_error| {
                format!("poll measured process after {error}: {wait_error}")
            })? {
                Some(status) => {
                    reaped_status = Some(status);
                    break;
                }
                None if samples.is_empty() => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(error);
                }
                None => {}
            },
        }
        if elapsed >= options.timeout {
            timed_out = true;
            child
                .kill()
                .map_err(|error| format!("terminate timed-out measured process: {error}"))?;
            if let Ok(sample) = ProcSample::read(pid, started.elapsed().as_nanos(), page_size) {
                samples.push(sample);
            }
            break;
        }
        thread::sleep(options.sample_interval);
    }

    let status = match reaped_status {
        Some(status) => status,
        None => child
            .wait()
            .map_err(|error| format!("wait for measured process: {error}"))?,
    };
    let wall_nanos = samples.last().map_or_else(
        || started.elapsed().as_nanos(),
        |sample| sample.elapsed_nanos,
    );
    let last = samples
        .last()
        .ok_or_else(|| "measured process produced no readable /proc sample".to_string())?;
    let peak_rss_bytes = samples
        .iter()
        .map(|sample| sample.peak_rss_bytes)
        .max()
        .unwrap_or(0);
    let cpu_seconds = last.total_ticks() as f64 / clock_ticks as f64;
    let wall_seconds = wall_nanos as f64 / 1_000_000_000.0;
    let allocation_utilization = options.worker_count.map(|workers| {
        if wall_seconds == 0.0 {
            0.0
        } else {
            cpu_seconds / wall_seconds / workers as f64
        }
    });
    let success = status.success() && !timed_out;
    let stage_counters_json = if success {
        let counters = fs::read_to_string(&stage_counters)
            .map_err(|error| format!("read aggregate stage counters: {error}"))?;
        let counters = counters.trim();
        if !counters.starts_with('{')
            || !counters.ends_with('}')
            || !counters.contains("\"schema\": \"ccwrapped.ingestion-performance/v1\"")
        {
            return Err("aggregate stage counter sidecar is malformed".to_string());
        }
        counters.to_string()
    } else {
        "null".to_string()
    };
    let samples_json = samples
        .iter()
        .map(ProcSample::json)
        .collect::<Vec<_>>()
        .join(",\n    ");

    Ok(format!(
        concat!(
            "{{\n",
            "  \"schema\": \"{}\",\n",
            "  \"success\": {},\n",
            "  \"timedOut\": {},\n",
            "  \"exitCode\": {},\n",
            "  \"termSignal\": {},\n",
            "  \"mode\": \"{}\",\n",
            "  \"workerCount\": {},\n",
            "  \"sampleIntervalMillis\": {},\n",
            "  \"pollingUncertaintyNanos\": {},\n",
            "  \"clockTicksPerSecond\": {},\n",
            "  \"pageSizeBytes\": {},\n",
            "  \"wallNanos\": {},\n",
            "  \"cpuTicks\": {},\n",
            "  \"cpuSeconds\": {:.9},\n",
            "  \"allocatedCpuUtilization\": {},\n",
            "  \"peakRssBytes\": {},\n",
            "  \"logicalReadBytes\": {},\n",
            "  \"logicalWriteBytes\": {},\n",
            "  \"physicalReadBytes\": {},\n",
            "  \"physicalWriteBytes\": {},\n",
            "  \"stageCounters\": {},\n",
            "  \"sampleCount\": {},\n",
            "  \"samples\": [\n    {}\n  ]\n",
            "}}\n"
        ),
        MEASUREMENT_SCHEMA,
        success,
        timed_out,
        option_i32_json(status.code()),
        option_i32_json(status.signal()),
        mode,
        option_usize_json(options.worker_count),
        options.sample_interval.as_millis(),
        options.sample_interval.as_nanos(),
        clock_ticks,
        page_size,
        wall_nanos,
        last.total_ticks(),
        cpu_seconds,
        option_f64_json(allocation_utilization),
        peak_rss_bytes,
        last.logical_read_bytes,
        last.logical_write_bytes,
        last.physical_read_bytes,
        last.physical_write_bytes,
        stage_counters_json,
        samples.len(),
        samples_json,
    ))
}

pub fn memory_baseline(
    worker_count: usize,
    bytes_per_worker: usize,
    passes: usize,
) -> Result<String, String> {
    if !(1..=256).contains(&worker_count) {
        return Err("memory worker count must be between 1 and 256".to_string());
    }
    if !(4_096..=256 * 1024 * 1024).contains(&bytes_per_worker) {
        return Err("memory bytes per worker must be between 4096 and 268435456".to_string());
    }
    if !(1..=1_000).contains(&passes) {
        return Err("memory passes must be between 1 and 1000".to_string());
    }
    let allocated_bytes = u64::try_from(worker_count)
        .ok()
        .and_then(|workers| workers.checked_mul(bytes_per_worker as u64))
        .and_then(|bytes| bytes.checked_mul(2))
        .ok_or_else(|| "memory baseline allocation size overflowed".to_string())?;
    if allocated_bytes > MAXIMUM_MEMORY_BASELINE_ALLOCATION {
        return Err(format!(
            "memory baseline allocation {allocated_bytes} exceeds the 4 GiB safety limit"
        ));
    }
    let payload_bytes = (worker_count as u64)
        .checked_mul(bytes_per_worker as u64)
        .and_then(|bytes| bytes.checked_mul(passes as u64))
        .ok_or_else(|| "memory baseline payload size overflowed".to_string())?;
    let traffic_lower_bound = payload_bytes
        .checked_mul(2)
        .ok_or_else(|| "memory baseline traffic size overflowed".to_string())?;

    let ready = Arc::new(Barrier::new(worker_count.saturating_add(1)));
    let start = Arc::new(Barrier::new(worker_count.saturating_add(1)));
    let mut handles = Vec::with_capacity(worker_count);
    for worker in 0..worker_count {
        let ready = Arc::clone(&ready);
        let start = Arc::clone(&start);
        handles.push(thread::spawn(move || {
            let pattern = (worker as u8).wrapping_mul(37).wrapping_add(11);
            let mut source = vec![pattern; bytes_per_worker];
            let mut destination = vec![!pattern; bytes_per_worker];
            ready.wait();
            start.wait();
            for pass in 0..passes {
                let position = pass % bytes_per_worker;
                source[position] ^= (pass as u8).wrapping_add(1);
                destination.copy_from_slice(&source);
                std::hint::black_box(&destination);
                std::mem::swap(&mut source, &mut destination);
            }
            let middle = bytes_per_worker / 2;
            u64::from(source[0])
                | (u64::from(source[middle]) << 8)
                | (u64::from(source[bytes_per_worker - 1]) << 16)
                | ((worker as u64) << 24)
        }));
    }
    ready.wait();
    let clock_ticks = getconf_u64("CLK_TCK")?;
    let cpu_ticks_before = self_cpu_ticks()?;
    let started = Instant::now();
    start.wait();
    let mut fingerprint = 0xcbf2_9ce4_8422_2325u64;
    for handle in handles {
        let worker_fingerprint = handle
            .join()
            .map_err(|_| "memory baseline worker panicked".to_string())?;
        fingerprint ^= worker_fingerprint;
        fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
    }
    let wall_nanos = started.elapsed().as_nanos();
    let cpu_ticks = self_cpu_ticks()?.saturating_sub(cpu_ticks_before);
    let cpu_seconds = cpu_ticks as f64 / clock_ticks as f64;
    let wall_seconds = wall_nanos as f64 / 1_000_000_000.0;
    let payload_bytes_per_second = if wall_seconds == 0.0 {
        0.0
    } else {
        payload_bytes as f64 / wall_seconds
    };
    let traffic_bytes_per_second_lower_bound = if wall_seconds == 0.0 {
        0.0
    } else {
        traffic_lower_bound as f64 / wall_seconds
    };
    let allocated_cpu_utilization = if wall_seconds == 0.0 {
        0.0
    } else {
        cpu_seconds / wall_seconds / worker_count as f64
    };
    let status = fs::read_to_string("/proc/self/status")
        .map_err(|error| format!("read memory baseline process status: {error}"))?;
    let peak_rss_bytes = named_kib(&status, "VmHWM").unwrap_or(0);

    Ok(format!(
        concat!(
            "{{\n",
            "  \"schema\": \"{}\",\n",
            "  \"workerCount\": {},\n",
            "  \"bytesPerWorker\": {},\n",
            "  \"passes\": {},\n",
            "  \"allocatedBytes\": {},\n",
            "  \"payloadBytesCopied\": {},\n",
            "  \"memoryTrafficBytesLowerBound\": {},\n",
            "  \"wallNanos\": {},\n",
            "  \"cpuTicks\": {},\n",
            "  \"cpuSeconds\": {:.9},\n",
            "  \"allocatedCpuUtilization\": {:.9},\n",
            "  \"payloadBytesPerSecond\": {:.3},\n",
            "  \"memoryTrafficBytesPerSecondLowerBound\": {:.3},\n",
            "  \"peakRssBytes\": {},\n",
            "  \"contentFingerprintFnv1a64\": \"{:016x}\"\n",
            "}}\n"
        ),
        MEMORY_SCHEMA,
        worker_count,
        bytes_per_worker,
        passes,
        allocated_bytes,
        payload_bytes,
        traffic_lower_bound,
        wall_nanos,
        cpu_ticks,
        cpu_seconds,
        allocated_cpu_utilization,
        payload_bytes_per_second,
        traffic_bytes_per_second_lower_bound,
        peak_rss_bytes,
        fingerprint,
    ))
}

pub fn read_baseline(corpus: &Path, buffer_bytes: usize, passes: usize) -> Result<String, String> {
    if !(4_096..=16 * 1024 * 1024).contains(&buffer_bytes) {
        return Err("reader buffer must be between 4096 and 16777216 bytes".to_string());
    }
    if !(1..=20).contains(&passes) {
        return Err("reader passes must be between 1 and 20".to_string());
    }
    let corpus = fs::canonicalize(corpus)
        .map_err(|error| format!("resolve reader corpus {}: {error}", corpus.display()))?;
    let started = Instant::now();
    let mut files = source_files_under(&corpus.join("projects"))?;
    files.extend(source_files_under(&corpus.join("otel"))?);
    files.sort();

    let mut buffer = vec![0u8; buffer_bytes];
    let mut bytes = 0u64;
    let mut fingerprint = 0xcbf2_9ce4_8422_2325u64;
    for _ in 0..passes {
        for path in &files {
            let file =
                File::open(path).map_err(|error| format!("open {}: {error}", path.display()))?;
            let mut reader = BufReader::with_capacity(buffer_bytes, file);
            loop {
                let read = reader
                    .read(&mut buffer)
                    .map_err(|error| format!("read {}: {error}", path.display()))?;
                if read == 0 {
                    break;
                }
                bytes = bytes.saturating_add(read as u64);
                for byte in &buffer[..read] {
                    fingerprint ^= u64::from(*byte);
                    fingerprint = fingerprint.wrapping_mul(0x0000_0100_0000_01b3);
                }
            }
        }
    }
    let wall_nanos = started.elapsed().as_nanos();
    let throughput = if wall_nanos == 0 {
        0.0
    } else {
        bytes as f64 / (wall_nanos as f64 / 1_000_000_000.0)
    };
    Ok(format!(
        concat!(
            "{{\n",
            "  \"schema\": \"{}\",\n",
            "  \"fileCount\": {},\n",
            "  \"passes\": {},\n",
            "  \"bufferBytes\": {},\n",
            "  \"bytesRead\": {},\n",
            "  \"wallNanos\": {},\n",
            "  \"bytesPerSecond\": {:.3},\n",
            "  \"contentFingerprintFnv1a64\": \"{:016x}\"\n",
            "}}\n"
        ),
        READER_SCHEMA,
        files.len(),
        passes,
        buffer_bytes,
        bytes,
        wall_nanos,
        throughput,
        fingerprint,
    ))
}

#[allow(clippy::manual_is_multiple_of)] // usize::is_multiple_of postdates the enforced MSRV.
pub fn sampling_overhead(iterations: usize) -> Result<String, String> {
    if !(10..=100_000).contains(&iterations) {
        return Err("sampler overhead iterations must be between 10 and 100000".to_string());
    }
    let page_size = getconf_u64("PAGESIZE")?;
    let pid = std::process::id();
    let mut samples = Vec::with_capacity(iterations);
    for _ in 0..iterations {
        let started = Instant::now();
        let sample = ProcSample::read(pid, 0, page_size)?;
        std::hint::black_box(sample);
        samples.push(started.elapsed().as_nanos());
    }
    let total_nanos = samples.iter().copied().sum::<u128>();
    let mut sorted = samples.clone();
    sorted.sort_unstable();
    let median_nanos = if iterations % 2 == 0 {
        sorted[iterations / 2 - 1].saturating_add(sorted[iterations / 2]) / 2
    } else {
        sorted[iterations / 2]
    };
    let p95_index = iterations
        .saturating_mul(95)
        .div_ceil(100)
        .saturating_sub(1);
    let samples_json = samples
        .iter()
        .map(u128::to_string)
        .collect::<Vec<_>>()
        .join(",");
    Ok(format!(
        concat!(
            "{{\n",
            "  \"schema\": \"{}\",\n",
            "  \"iterations\": {},\n",
            "  \"totalNanos\": {},\n",
            "  \"meanNanos\": {},\n",
            "  \"medianNanos\": {},\n",
            "  \"p95Nanos\": {},\n",
            "  \"maximumNanos\": {},\n",
            "  \"procFilesPerSample\": 3,\n",
            "  \"samplesNanos\": [{}]\n",
            "}}\n"
        ),
        SAMPLER_OVERHEAD_SCHEMA,
        iterations,
        total_nanos,
        total_nanos / iterations as u128,
        median_nanos,
        sorted[p95_index],
        sorted[iterations - 1],
        samples_json,
    ))
}

fn validate_measure_options(options: &MeasureOptions) -> Result<(), String> {
    let sample_millis = options.sample_interval.as_millis();
    if !(1..=1_000).contains(&sample_millis) {
        return Err("sample interval must be between 1 and 1000 milliseconds".to_string());
    }
    let timeout_seconds = options.timeout.as_secs();
    if !(1..=MAXIMUM_TIMEOUT_SECONDS).contains(&timeout_seconds) {
        return Err(format!(
            "timeout must be between 1 and {MAXIMUM_TIMEOUT_SECONDS} seconds"
        ));
    }
    if options.worker_count == Some(0) {
        return Err("worker count must be positive".to_string());
    }
    Ok(())
}

fn source_files_under(root: &Path) -> Result<Vec<PathBuf>, String> {
    if !root.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    collect_files(root, &mut files)?;
    files.sort();
    Ok(files)
}

fn collect_files(directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), String> {
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
            collect_files(&path, files)?;
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}

fn parse_proc_stat(input: &str, page_size: u64) -> Result<ParsedStat, String> {
    let close = input
        .rfind(')')
        .ok_or_else(|| "process stat has no command terminator".to_string())?;
    let fields = input
        .get(close.saturating_add(2)..)
        .ok_or_else(|| "process stat ended before state".to_string())?
        .split_ascii_whitespace()
        .collect::<Vec<_>>();
    if fields.len() <= 21 {
        return Err("process stat has too few fields".to_string());
    }
    let state = fields[0]
        .chars()
        .next()
        .ok_or_else(|| "process stat state is empty".to_string())?;
    let rss_pages = fields[21]
        .parse::<i64>()
        .map_err(|_| "process stat RSS is invalid".to_string())?
        .max(0) as u64;
    Ok(ParsedStat {
        state,
        user_ticks: parse_stat_u64(&fields, 11, "user ticks")?,
        system_ticks: parse_stat_u64(&fields, 12, "system ticks")?,
        threads: parse_stat_u64(&fields, 17, "thread count")?,
        rss_bytes: rss_pages.saturating_mul(page_size),
    })
}

fn parse_stat_u64(fields: &[&str], index: usize, name: &str) -> Result<u64, String> {
    fields[index]
        .parse::<u64>()
        .map_err(|_| format!("process stat {name} is invalid"))
}

fn named_u64(input: &str, name: &str) -> Option<u64> {
    input.lines().find_map(|line| {
        let (key, value) = line.split_once(':')?;
        (key == name)
            .then(|| value.split_ascii_whitespace().next()?.parse().ok())
            .flatten()
    })
}

fn named_kib(input: &str, name: &str) -> Option<u64> {
    named_u64(input, name).map(|value| value.saturating_mul(1024))
}

fn getconf_u64(name: &str) -> Result<u64, String> {
    let output = Command::new("getconf")
        .arg(name)
        .output()
        .map_err(|error| format!("run getconf {name}: {error}"))?;
    if !output.status.success() {
        return Err(format!("getconf {name} exited with {}", output.status));
    }
    std::str::from_utf8(&output.stdout)
        .map_err(|_| format!("getconf {name} returned non-UTF-8 output"))?
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("getconf {name} returned an invalid integer"))
}

fn self_cpu_ticks() -> Result<u64, String> {
    let stat = fs::read_to_string("/proc/self/stat")
        .map_err(|error| format!("read memory baseline process stat: {error}"))?;
    let parsed = parse_proc_stat(&stat, 1)?;
    Ok(parsed.user_ticks.saturating_add(parsed.system_ticks))
}

fn option_i32_json(value: Option<i32>) -> String {
    value.map_or_else(|| "null".to_string(), |value| value.to_string())
}

fn option_usize_json(value: Option<usize>) -> String {
    value.map_or_else(|| "null".to_string(), |value| value.to_string())
}

fn option_f64_json(value: Option<f64>) -> String {
    value.map_or_else(
        || "null".to_string(),
        |value| {
            if value.is_finite() {
                format!("{value:.9}")
            } else {
                "null".to_string()
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::{memory_baseline, named_kib, named_u64, parse_proc_stat, sampling_overhead};

    #[test]
    fn parses_proc_stat_with_spaces_and_parentheses_in_command() {
        let mut fields = vec!["0"; 22];
        fields[0] = "R";
        fields[11] = "123";
        fields[12] = "45";
        fields[17] = "7";
        fields[21] = "11";
        let input = format!("42 (a tricky ) name) {}", fields.join(" "));
        let parsed = parse_proc_stat(&input, 4096).expect("parse stat");
        assert_eq!(parsed.state, 'R');
        assert_eq!(parsed.user_ticks, 123);
        assert_eq!(parsed.system_ticks, 45);
        assert_eq!(parsed.threads, 7);
        assert_eq!(parsed.rss_bytes, 45_056);
    }

    #[test]
    fn bounded_parallel_memory_baseline_reports_complete_accounting() {
        let output = memory_baseline(2, 4_096, 3).expect("memory baseline");
        assert!(output.contains("\"schema\": \"ccwrapped.phase5-memory/v1\""));
        assert!(output.contains("\"workerCount\": 2"));
        assert!(output.contains("\"allocatedBytes\": 16384"));
        assert!(output.contains("\"payloadBytesCopied\": 24576"));
        assert!(output.contains("\"memoryTrafficBytesLowerBound\": 49152"));
    }

    #[test]
    fn parses_proc_status_and_io_counters_by_exact_key() {
        let text = "VmRSS:\t123 kB\nrchar: 456\nread_bytes: 789\n";
        assert_eq!(named_kib(text, "VmRSS"), Some(125_952));
        assert_eq!(named_u64(text, "rchar"), Some(456));
        assert_eq!(named_u64(text, "char"), None);
    }

    #[test]
    fn sampler_overhead_reports_every_proc_poll() {
        let output = sampling_overhead(10).expect("measure sampler overhead");
        let parsed: serde_json::Value =
            serde_json::from_str(&output).expect("parse sampler overhead");
        assert_eq!(parsed["schema"], "ccwrapped.phase5-sampler-overhead/v1");
        assert_eq!(parsed["iterations"], 10);
        assert_eq!(parsed["samplesNanos"].as_array().map(Vec::len), Some(10));
        assert!(parsed["totalNanos"].as_u64().is_some_and(|value| value > 0));
    }
}
