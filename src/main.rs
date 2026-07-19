use ccwrapped::analyzers::cache::analyze_cache_health;
use ccwrapped::analyzers::cost::analyze_usage;
use ccwrapped::analyzers::models::{analyze_model_routing, analyze_session_intelligence};
use ccwrapped::analyzers::story::build_wrapped_story;
use ccwrapped::renderers::html::render_html;
use ccwrapped::renderers::markdown::render_markdown;
use ccwrapped::renderers::share_card::render_share_card;
use ccwrapped::renderers::terminal::widgets::terminal_text;
use ccwrapped::renderers::terminal::{color_choice, try_render_terminal_with};
use ccwrapped::{format_hour, home_dir, project_slug, Report, TimeBucket};
use clap::{error::ErrorKind, Parser};
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::Instant;

mod ingestion;
#[cfg(windows)]
mod windows_private_acl;

#[derive(Debug, Parser)]
#[command(
    name = "ccwrapped",
    version,
    about = "Generate a Claude Code wrapped report from local Claude Code artifacts."
)]
struct Cli {
    #[arg(
        long = "data-dir",
        value_name = "PATH",
        help = "read a projects or Claude config directory (repeatable)"
    )]
    data_dirs: Vec<PathBuf>,
    #[arg(
        long = "otel-file",
        value_name = "PATH",
        help = "read a supported local Collector JSON/JSONL file (repeatable)"
    )]
    otel_files: Vec<PathBuf>,
    #[arg(
        long,
        help = "include exact paths in local diagnostic errors (never report JSON)"
    )]
    private_diagnostics: bool,
    #[arg(long, help = "write claude-code-wrapped.html")]
    html: bool,
    #[arg(long, help = "write claude-code-wrapped.md")]
    markdown: bool,
    #[arg(long, help = "write claude-code-wrapped-card.html")]
    card: bool,
    #[arg(long, help = "write per-project prompt files to ./wrapped-archive/")]
    archive: bool,
    #[arg(long, help = "write all output formats (html + card + markdown)")]
    all: bool,
    #[arg(
        long,
        help = "open selected HTML outputs after writing (implies --html when needed)"
    )]
    open: bool,
    #[arg(
        long,
        conflicts_with_all = ["html", "markdown", "card", "archive", "all", "open"],
        help = "print JSON to stdout only; conflicts with file and browser outputs"
    )]
    json: bool,
    #[arg(long, help = "disable colors (also respects NO_COLOR env var)")]
    plain: bool,
    #[arg(
        long,
        value_name = "IANA_ZONE",
        help = "attribute periods and calendar labels in this IANA timezone"
    )]
    timezone: Option<String>,
    #[arg(
        long,
        value_name = "MINUTES",
        default_value_t = 5,
        help = "cap inferred active intervals at this many minutes"
    )]
    active_threshold_minutes: u64,
    #[arg(long = "ingestion-workers", value_name = "COUNT", hide = true)]
    ingestion_workers: Option<usize>,
    #[arg(long = "ingestion-delay-seed", value_name = "SEED", hide = true)]
    ingestion_delay_seed: Option<u64>,
    #[arg(
        long = "ingestion-panic-file",
        value_name = "ZERO_BASED_INDEX",
        hide = true
    )]
    ingestion_panic_file: Option<usize>,
    #[arg(
        long = "benchmark-counters",
        value_name = "PATH",
        requires = "json",
        hide = true
    )]
    benchmark_counters: Option<PathBuf>,
    #[arg(
        long,
        conflicts_with = "rebuild_store",
        help = "disable the local incremental SQLite store for this invocation"
    )]
    no_store: bool,
    #[arg(
        long,
        conflicts_with_all = ["no_store", "archive"],
        help = "replace the local incremental store from a complete source scan"
    )]
    rebuild_store: bool,
    #[arg(
        long,
        value_name = "PATH",
        conflicts_with_all = ["no_store", "archive"],
        help = "use an explicit local incremental-store database"
    )]
    store_path: Option<PathBuf>,
    #[arg(value_name = "YEAR")]
    year: Option<i32>,
}

fn main() {
    if let Err(error) = run() {
        if error_is_broken_pipe(error.as_ref()) {
            return;
        }
        let _ = writeln!(io::stderr().lock(), "\n  x Error: {error}");
        std::process::exit(1);
    }
}

fn error_is_broken_pipe(mut error: &(dyn Error + 'static)) -> bool {
    loop {
        if error
            .downcast_ref::<io::Error>()
            .is_some_and(|error| error.kind() == io::ErrorKind::BrokenPipe)
        {
            return true;
        }
        let Some(source) = error.source() else {
            return false;
        };
        error = source;
    }
}

fn write_stdout_line(arguments: std::fmt::Arguments<'_>) -> io::Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    writer.write_fmt(arguments)?;
    writer.write_all(b"\n")
}

fn write_stdout_bytes_line(bytes: &[u8]) -> io::Result<()> {
    let stdout = io::stdout();
    let mut writer = stdout.lock();
    writer.write_all(bytes)?;
    writer.write_all(b"\n")
}

fn exit_after_json_write(result: io::Result<()>, status: i32) -> ! {
    match result {
        Ok(()) => std::process::exit(status),
        Err(error) if error.kind() == io::ErrorKind::BrokenPipe => std::process::exit(0),
        Err(error) => {
            let _ = writeln!(io::stderr().lock(), "failed to write JSON output: {error}");
            std::process::exit(1);
        }
    }
}

struct BuiltReport {
    report: Report,
    entry_count: usize,
    private_prompts: Vec<ingestion::PrivatePrompt>,
    store_files: Vec<ingestion::StoreSourceFile>,
    analysis_state: ingestion::AnalysisState,
    encoded_analysis_state: Option<Vec<u8>>,
    invalidate_analysis_state: bool,
    store_publish_allowed: bool,
    performance: ingestion::IngestionPerformance,
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = parse_cli();
    let (unbounded_time, timezone_fallback) = match cli.timezone.as_deref() {
        Some(name) => match ingestion::TimeContext::new(name, None) {
            Ok(context) => (context, false),
            Err(error) => return exit_with_time_error(&cli, cli.year, &error),
        },
        None => ingestion::TimeContext::resolve_default(None),
    };
    let selected_year = cli.year.unwrap_or_else(|| unbounded_time.current_year());
    let time_context = match ingestion::TimeContext::new(unbounded_time.name(), Some(selected_year))
    {
        Ok(context) => context,
        Err(error) => return exit_with_time_error(&cli, Some(selected_year), &error),
    };
    let active_threshold_seconds = match cli.active_threshold_minutes.checked_mul(60) {
        Some(seconds) if (60..=86_400).contains(&seconds) => seconds,
        _ => {
            return exit_with_config_error(
                &cli,
                Some(selected_year),
                "E_ACTIVE_THRESHOLD_INVALID",
                "the active-time threshold must be between 1 and 1440 minutes",
                "Choose a whole-minute value from 1 through 1440.",
            )
        }
    };

    let resolved_home = home_dir();
    let configured_store_path = if cli.no_store || cli.archive {
        None
    } else {
        resolve_store_path(cli.store_path.as_deref(), resolved_home.as_deref())
    };
    let mut prepared_store = if let Some(path) = configured_store_path.as_deref() {
        match ingestion::prepare_store(path, cli.rebuild_store) {
            Ok(prepared) => Some(prepared),
            Err(error) => return exit_with_ingestion_error(&cli, selected_year, &error),
        }
    } else {
        None
    };
    let store_path = prepared_store
        .as_ref()
        .map(|prepared| prepared.path().to_path_buf());
    let store_salt = prepared_store.as_ref().map(ingestion::PreparedStore::salt);
    let ingestion_options = ingestion::IngestionOptions {
        time_context,
        active_threshold_seconds,
        timezone_fallback,
        data_dirs: cli.data_dirs.clone(),
        otel_files: cli.otel_files.clone(),
        claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from),
        home_dir: resolved_home.clone(),
        include_private_content: cli.archive && !cli.json,
        private_diagnostics: cli.private_diagnostics,
        worker_count: cli.ingestion_workers,
        worker_delay_seed: cli.ingestion_delay_seed,
        worker_panic_file: cli.ingestion_panic_file,
        store_path: store_path.clone(),
        store_salt,
    };
    if cli.json && !cli.rebuild_store && !cli.private_diagnostics {
        if let Some(path) = store_path.as_deref() {
            match ingestion::lookup_cached_report(&ingestion_options, path) {
                Ok(Some(report)) => {
                    if let Some(path) = &cli.benchmark_counters {
                        write_benchmark_counters(path, &ingestion::IngestionPerformance::cached())?;
                    }
                    write_stdout_bytes_line(&report)?;
                    return Ok(());
                }
                Ok(None) => {}
                Err(error) => {
                    return exit_with_ingestion_error_after_store_abort(
                        &cli,
                        selected_year,
                        &error,
                        &mut prepared_store,
                    );
                }
            }
        }
    }

    let mut ingested = match ingestion::ingest(ingestion_options.clone()) {
        Ok(ingested) => ingested,
        Err(error) => {
            return exit_with_ingestion_error_after_store_abort(
                &cli,
                selected_year,
                &error,
                &mut prepared_store,
            );
        }
    };
    if cli.private_diagnostics {
        for (alias, path) in &ingested.private_source_paths {
            let _ = writeln!(
                io::stderr().lock(),
                "[privacy-profile: private] source {alias}: {}",
                path.display()
            );
        }
    }
    if cli.json {
        if let Some(report_json) = ingested.fast_report_json.take() {
            if cli.rebuild_store && !ingested.store_publish_allowed {
                let error = ingestion::incomplete_rebuild_error();
                return exit_with_ingestion_error_after_store_abort(
                    &cli,
                    selected_year,
                    &error,
                    &mut prepared_store,
                );
            }
            if ingested.store_publish_allowed {
                if let Some(path) = store_path.as_deref() {
                    let publish_started = Instant::now();
                    if let Err(error) = ingestion::publish_cached_report(
                        path,
                        &ingestion_options,
                        &ingested.store_files,
                        &ingested.analysis_state,
                        ingested.encoded_analysis_state.as_deref(),
                        ingested.invalidate_analysis_state,
                        &report_json,
                    ) {
                        return exit_with_ingestion_error_after_store_abort(
                            &cli,
                            selected_year,
                            &error,
                            &mut prepared_store,
                        );
                    }
                    ingested.performance.store_publish_nanos = publish_started.elapsed().as_nanos();
                }
            }
            if let Some(prepared) = prepared_store.as_mut() {
                if let Err(error) = prepared.commit() {
                    return exit_with_ingestion_error_after_store_abort(
                        &cli,
                        selected_year,
                        &error,
                        &mut prepared_store,
                    );
                }
            }
            if let Some(path) = &cli.benchmark_counters {
                write_benchmark_counters(path, &ingested.performance)?;
            }
            write_stdout_bytes_line(&report_json)?;
            return Ok(());
        }
    }
    let coverage_for_empty = ingested.coverage.clone();
    let report_started = Instant::now();
    let built_report = match build_report(selected_year, ingested) {
        Ok(report) => report,
        Err(error) => {
            abort_prepared_store(&mut prepared_store)?;
            return Err(error);
        }
    };
    let Some(mut built_report) = built_report else {
        if let Err(error) = abort_prepared_store(&mut prepared_store) {
            return exit_with_ingestion_error(&cli, selected_year, &error);
        }
        return exit_with_message(
            &cli,
            selected_year,
            "E_NO_RECORDS",
            "no records found",
            format!("No supported Claude Code usage records were observed for {selected_year}."),
            "Select a period containing supported records or add an explicit --data-dir/--otel-file source.",
            Some(&coverage_for_empty),
        );
    };
    built_report.performance.report_build_nanos = report_started.elapsed().as_nanos();

    let serialization_started = Instant::now();
    let report_json = match serde_json::to_string_pretty(&built_report.report) {
        Ok(report) => report,
        Err(error) => {
            abort_prepared_store(&mut prepared_store)?;
            return Err(error.into());
        }
    };
    built_report.performance.report_serialization_nanos =
        serialization_started.elapsed().as_nanos();
    if cli.rebuild_store && !built_report.store_publish_allowed {
        let error = ingestion::incomplete_rebuild_error();
        return exit_with_ingestion_error_after_store_abort(
            &cli,
            selected_year,
            &error,
            &mut prepared_store,
        );
    }
    if built_report.store_publish_allowed {
        if let Some(path) = store_path.as_deref() {
            let publish_started = Instant::now();
            if let Err(error) = ingestion::publish_cached_report(
                path,
                &ingestion_options,
                &built_report.store_files,
                &built_report.analysis_state,
                built_report.encoded_analysis_state.as_deref(),
                built_report.invalidate_analysis_state,
                report_json.as_bytes(),
            ) {
                return exit_with_ingestion_error_after_store_abort(
                    &cli,
                    selected_year,
                    &error,
                    &mut prepared_store,
                );
            }
            built_report.performance.store_publish_nanos = publish_started.elapsed().as_nanos();
        }
    }
    if let Some(prepared) = prepared_store.as_mut() {
        if let Err(error) = prepared.commit() {
            return exit_with_ingestion_error_after_store_abort(
                &cli,
                selected_year,
                &error,
                &mut prepared_store,
            );
        }
    }

    if cli.json {
        if let Some(path) = &cli.benchmark_counters {
            write_benchmark_counters(path, &built_report.performance)?;
        }
        write_stdout_line(format_args!("{report_json}"))?;
        return Ok(());
    }

    // Terminal output is always the primary experience
    let choice = color_choice(cli.plain);
    print_summary(
        built_report.entry_count,
        selected_year,
        &built_report.report,
        choice,
    )?;
    print_coverage_summary(&built_report.report.data_coverage)?;

    // File outputs are opt-in
    let wants_files = cli.html || cli.card || cli.markdown || cli.archive || cli.all || cli.open;
    if wants_files {
        let cwd = std::env::current_dir()?;
        let outputs = write_outputs(
            &cwd,
            &built_report.report,
            &built_report.private_prompts,
            &cli,
        )?;
        for path in &outputs {
            write_stdout_line(format_args!("  Wrote {}", path.display()))?;
        }
    }
    Ok(())
}

fn parse_cli() -> Cli {
    let args = std::env::args_os().collect::<Vec<_>>();
    match Cli::try_parse_from(&args) {
        Ok(cli) => cli,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            error.exit()
        }
        Err(_) if json_output_requested(&args[1..]) => {
            exit_after_json_write(
                write_stdout_line(format_args!(
                    "{}",
                    serde_json::json!({
                        "error": "invalid configuration",
                        "year": null,
                        "code": "E_CLI_ARGUMENT_INVALID",
                        "message": "one or more command-line arguments are invalid",
                        "remediation": "Check --help and provide values in the documented format.",
                    })
                )),
                2,
            );
        }
        Err(error) => error.exit(),
    }
}

fn resolve_store_path(explicit: Option<&Path>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(path) = explicit {
        return Some(path.to_path_buf());
    }
    if let Some(cache_home) = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
    {
        return Some(cache_home.join("ccwrapped").join("store-v1.sqlite3"));
    }
    home.map(|home| {
        home.join(".cache")
            .join("ccwrapped")
            .join("store-v1.sqlite3")
    })
}

fn json_output_requested(args: &[OsString]) -> bool {
    args.iter()
        .take_while(|argument| argument.as_os_str() != OsStr::new("--"))
        .any(|argument| argument.as_os_str() == OsStr::new("--json"))
}

fn abort_prepared_store(
    prepared_store: &mut Option<ingestion::PreparedStore>,
) -> Result<(), ingestion::IngestionError> {
    prepared_store
        .as_mut()
        .map(ingestion::PreparedStore::abort)
        .transpose()
        .map(|_| ())
}

fn exit_with_ingestion_error_after_store_abort(
    cli: &Cli,
    year: i32,
    original: &ingestion::IngestionError,
    prepared_store: &mut Option<ingestion::PreparedStore>,
) -> Result<(), Box<dyn Error>> {
    match abort_prepared_store(prepared_store) {
        Ok(()) => exit_with_ingestion_error(cli, year, original),
        Err(cleanup) => exit_with_ingestion_error(cli, year, &cleanup),
    }
}

fn exit_with_time_error(
    cli: &Cli,
    year: Option<i32>,
    error: &ingestion::TimeContextError,
) -> Result<(), Box<dyn Error>> {
    exit_with_config_error(
        cli,
        year,
        error.code(),
        &error.to_string(),
        error.remediation(),
    )
}

fn exit_with_config_error(
    cli: &Cli,
    year: Option<i32>,
    code: &str,
    message: &str,
    remediation: &str,
) -> Result<(), Box<dyn Error>> {
    if cli.json {
        exit_after_json_write(
            write_stdout_line(format_args!(
                "{}",
                serde_json::json!({
                    "error": "invalid configuration",
                    "year": year,
                    "code": code,
                    "message": message,
                    "remediation": remediation,
                })
            )),
            1,
        );
    }
    let _ = writeln!(
        io::stderr().lock(),
        "[{code}] {message}\nRemediation: {remediation}"
    );
    std::process::exit(1);
}

fn exit_with_message(
    cli: &Cli,
    year: i32,
    code: &str,
    json_error: &str,
    human_message: String,
    remediation: &str,
    coverage: Option<&ccwrapped::DataCoverage>,
) -> Result<(), Box<dyn Error>> {
    if cli.json {
        exit_after_json_write(
            write_stdout_line(format_args!(
                "{}",
                serde_json::json!({
                    "error": json_error,
                    "year": year,
                    "code": code,
                    "message": human_message,
                    "remediation": remediation,
                    "dataCoverage": coverage,
                })
            )),
            1,
        );
    }
    let _ = writeln!(
        io::stderr().lock(),
        "[{code}] {human_message}\nRemediation: {remediation}"
    );
    std::process::exit(1);
}

fn exit_with_ingestion_error(
    cli: &Cli,
    year: i32,
    error: &ingestion::IngestionError,
) -> Result<(), Box<dyn Error>> {
    if cli.json {
        exit_after_json_write(
            write_stdout_line(format_args!(
                "{}",
                serde_json::json!({
                    "error": "ingestion failed",
                    "year": year,
                    "code": error.code(),
                    "sourceAlias": error.source_alias(),
                    "message": "A selected local source could not be ingested safely.",
                    "remediation": error.remediation(),
                })
            )),
            1,
        );
    }
    let _ = writeln!(
        io::stderr().lock(),
        "{} ({}): {}\nRemediation: {}",
        error.source_alias().unwrap_or("ingestion"),
        error.code(),
        error.message(),
        error.remediation()
    );
    std::process::exit(1);
}

fn build_report(
    selected_year: i32,
    ingested: ingestion::IngestionResult,
) -> Result<Option<BuiltReport>, Box<dyn Error>> {
    let ingestion::IngestionResult {
        entries: analysis_entries,
        session_breakdown,
        daily,
        project_breakdown,
        methodology,
        canonical_metrics,
        insights,
        hour_distribution,
        coverage,
        private_prompts,
        private_source_paths: _,
        store_files,
        analysis_state,
        encoded_analysis_state,
        invalidate_analysis_state,
        store_publish_allowed,
        fast_report_json: _,
        mut performance,
    } = ingested;
    let entry_projection_started = Instant::now();
    let entries = analysis_entries
        .into_iter()
        .filter(|entry| entry.is_message_occurrence())
        .map(ingestion::AnalysisEntry::into_observed_accumulator)
        .collect::<Vec<_>>();
    performance.report_entry_projection_nanos = entry_projection_started.elapsed().as_nanos();
    if entries.is_empty() && coverage.canonical_records == 0 {
        return Ok(None);
    }

    let cost_claims_available = analytical_capability_available(&coverage, "analysis_cost");
    let partial_cost_evidence = analytical_capability_is(&coverage, "analysis_cost", "partial");
    let worker_count = performance.selected_workers;
    let (cost_analysis, cost_nanos, cache_health, cache_nanos, session_intel, session_nanos) =
        if worker_count >= 3 {
            thread::scope(|scope| {
                let cost = scope.spawn(|| {
                    timed(|| {
                        compatibility_cost_analysis(
                            selected_year,
                            &daily,
                            &session_breakdown,
                            cost_claims_available,
                            partial_cost_evidence,
                        )
                    })
                });
                let session = scope.spawn(|| {
                    timed(|| {
                        compatibility_session_intel(&session_breakdown, &entries, hour_distribution)
                    })
                });
                let (cache, cache_nanos) = timed(|| compatibility_cache_health(&daily));
                let (cost, cost_nanos) = cost
                    .join()
                    .map_err(|_| io::Error::other("compatibility cost worker panicked"))?;
                let (session, session_nanos) = session
                    .join()
                    .map_err(|_| io::Error::other("compatibility session worker panicked"))?;
                Ok::<_, io::Error>((cost, cost_nanos, cache, cache_nanos, session, session_nanos))
            })?
        } else if worker_count == 2 {
            thread::scope(|scope| {
                let session = scope.spawn(|| {
                    timed(|| {
                        compatibility_session_intel(&session_breakdown, &entries, hour_distribution)
                    })
                });
                let (cost, cost_nanos) = timed(|| {
                    compatibility_cost_analysis(
                        selected_year,
                        &daily,
                        &session_breakdown,
                        cost_claims_available,
                        partial_cost_evidence,
                    )
                });
                let (cache, cache_nanos) = timed(|| compatibility_cache_health(&daily));
                let (session, session_nanos) = session
                    .join()
                    .map_err(|_| io::Error::other("compatibility session worker panicked"))?;
                Ok::<_, io::Error>((cost, cost_nanos, cache, cache_nanos, session, session_nanos))
            })?
        } else {
            let (cost, cost_nanos) = timed(|| {
                compatibility_cost_analysis(
                    selected_year,
                    &daily,
                    &session_breakdown,
                    cost_claims_available,
                    partial_cost_evidence,
                )
            });
            let (cache, cache_nanos) = timed(|| compatibility_cache_health(&daily));
            let (session, session_nanos) = timed(|| {
                compatibility_session_intel(&session_breakdown, &entries, hour_distribution)
            });
            (cost, cost_nanos, cache, cache_nanos, session, session_nanos)
        };
    performance.report_cost_nanos = cost_nanos;
    performance.report_cache_nanos = cache_nanos;
    performance.report_session_nanos = session_nanos;
    let anomalies = Default::default();
    let inflection = None;
    let model_routing_started = Instant::now();
    let mut model_routing =
        compatibility_model_routing(&entries, &cost_analysis, cost_claims_available);
    model_routing.busiest_hour = session_intel.peak_hours.first().cloned();
    model_routing.estimated_savings = 0.0;
    performance.report_model_routing_nanos = model_routing_started.elapsed().as_nanos();
    let recommendation_started = Instant::now();
    let recommendations = compatibility_recommendations(&insights);
    performance.report_recommendation_nanos = recommendation_started.elapsed().as_nanos();

    let mut report = Report {
        schema_version: "ccwrapped.report/v2".to_string(),
        generated_at: coverage
            .latest_observed_at
            .clone()
            .unwrap_or_else(|| format!("{selected_year:04}-01-01T00:00:00Z")),
        year: selected_year,
        data_coverage: coverage,
        methodology,
        canonical_metrics,
        insights,
        cost_analysis,
        cache_health,
        anomalies,
        inflection,
        session_intel,
        session_breakdown,
        model_routing,
        project_breakdown,
        recommendations,
        wrapped_story: Default::default(),
    };
    let story_started = Instant::now();
    report.wrapped_story = build_wrapped_story(&report, &entries);
    performance.report_story_nanos = story_started.elapsed().as_nanos();

    Ok(Some(BuiltReport {
        report,
        entry_count: entries.len(),
        private_prompts,
        store_files,
        analysis_state,
        encoded_analysis_state,
        invalidate_analysis_state,
        store_publish_allowed,
        performance,
    }))
}

fn timed<T>(operation: impl FnOnce() -> T) -> (T, u128) {
    let started = Instant::now();
    let result = operation();
    (result, started.elapsed().as_nanos())
}

fn compatibility_cost_analysis(
    selected_year: i32,
    daily: &[ccwrapped::DailyAggregate],
    session_breakdown: &ccwrapped::SessionBreakdown,
    cost_claims_available: bool,
    partial_cost_evidence: bool,
) -> ccwrapped::CostAnalysis {
    let mut cost_analysis = analyze_usage(selected_year, daily, session_breakdown);
    if partial_cost_evidence {
        suppress_partial_cost_derivations(&mut cost_analysis);
    } else if !cost_claims_available {
        suppress_unavailable_cost_claims(&mut cost_analysis);
    }
    cost_analysis
}

fn compatibility_cache_health(daily: &[ccwrapped::DailyAggregate]) -> ccwrapped::CacheHealth {
    let mut cache_health = analyze_cache_health(daily);
    suppress_unavailable_cache_claims(&mut cache_health);
    cache_health
}

fn compatibility_session_intel(
    session_breakdown: &ccwrapped::SessionBreakdown,
    entries: &[ccwrapped::AssistantEntry],
    hour_distribution: Vec<usize>,
) -> ccwrapped::SessionIntel {
    let mut session_intel = analyze_session_intelligence(session_breakdown, entries);
    apply_hour_distribution(&mut session_intel, hour_distribution);
    session_intel
}

fn compatibility_model_routing(
    entries: &[ccwrapped::AssistantEntry],
    cost_analysis: &ccwrapped::CostAnalysis,
    cost_claims_available: bool,
) -> ccwrapped::ModelRouting {
    if entries.is_empty() {
        ccwrapped::ModelRouting::default()
    } else if cost_claims_available {
        analyze_model_routing(cost_analysis, entries)
    } else {
        let mut unavailable_cost_projection = cost_analysis.clone();
        suppress_unavailable_cost_claims(&mut unavailable_cost_projection);
        analyze_model_routing(&unavailable_cost_projection, entries)
    }
}

fn write_benchmark_counters(
    path: &Path,
    performance: &ingestion::IngestionPerformance,
) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(performance).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(&bytes)?;
    file.sync_all()
}

fn compatibility_recommendations(
    insights: &ccwrapped::InsightReport,
) -> Vec<ccwrapped::Recommendation> {
    insights
        .cards
        .iter()
        .filter(|card| card.class == "recommendation")
        .filter_map(|card| {
            Some(ccwrapped::Recommendation {
                severity: "evidence".to_string(),
                title: card.title.clone(),
                savings: "Bounded experiment".to_string(),
                action: card.action.as_ref()?.experiment.clone(),
            })
        })
        .collect()
}

fn apply_hour_distribution(session_intel: &mut ccwrapped::SessionIntel, distribution: Vec<usize>) {
    let total = distribution
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add);
    let mut buckets = distribution
        .iter()
        .enumerate()
        .map(|(hour, count)| TimeBucket {
            hour: u8::try_from(hour).unwrap_or_default(),
            label: format_hour(u8::try_from(hour).unwrap_or_default()),
            count: *count,
            share_pct: if total == 0 {
                0
            } else {
                ((*count as f64 / total as f64) * 100.0).round() as u64
            },
        })
        .collect::<Vec<_>>();
    buckets.sort_by(|left, right| {
        right
            .count
            .cmp(&left.count)
            .then_with(|| left.hour.cmp(&right.hour))
    });
    session_intel.peak_hours = if total == 0 {
        Vec::new()
    } else {
        buckets.into_iter().take(3).collect()
    };
    let overlap = distribution
        .get(12..=18)
        .unwrap_or_default()
        .iter()
        .copied()
        .fold(0usize, usize::saturating_add);
    session_intel.peak_overlap_pct = if total == 0 {
        0
    } else {
        ((overlap as f64 / total as f64) * 100.0).round() as u64
    };
    session_intel.hour_distribution = distribution;
}

fn analytical_capability_available(coverage: &ccwrapped::DataCoverage, capability: &str) -> bool {
    analytical_capability_is(coverage, capability, "available")
}

fn analytical_capability_is(
    coverage: &ccwrapped::DataCoverage,
    capability: &str,
    expected: &str,
) -> bool {
    coverage.capabilities.get(capability).map(String::as_str) == Some(expected)
}

fn suppress_unavailable_cost_claims(cost_analysis: &mut ccwrapped::CostAnalysis) {
    cost_analysis.total_cost = 0.0;
    cost_analysis.model_costs.clear();
    for day in &mut cost_analysis.daily_costs {
        day.cost = 0.0;
        for model in &mut day.models {
            model.cost = 0.0;
        }
    }
    suppress_partial_cost_derivations(cost_analysis);
}

fn suppress_partial_cost_derivations(cost_analysis: &mut ccwrapped::CostAnalysis) {
    cost_analysis.avg_daily_cost = 0.0;
    cost_analysis.median_daily_cost = 0.0;
    cost_analysis.peak_day = None;
}

fn suppress_unavailable_cache_claims(cache_health: &mut ccwrapped::CacheHealth) {
    cache_health.estimated_breaks = 0;
    cache_health.reasons_ranked.clear();
    cache_health.cache_hit_rate = 0.0;
    cache_health.efficiency_ratio = 0;
    cache_health.grade = ccwrapped::CacheGrade {
        letter: "N/A".to_string(),
        color: "#94a3b8".to_string(),
        label: "Unavailable — usage categories are incomplete".to_string(),
        ..ccwrapped::CacheGrade::default()
    };
    cache_health.savings = Default::default();
}

fn write_outputs(
    cwd: &Path,
    report: &Report,
    private_prompts: &[ingestion::PrivatePrompt],
    cli: &Cli,
) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut rendered = Vec::<(&str, Vec<u8>)>::new();
    let open_implies_html = cli.open && !(cli.html || cli.card || cli.all);
    if cli.html || cli.all || open_implies_html {
        rendered.push(("claude-code-wrapped.html", render_html(report).into_bytes()));
    }
    if cli.markdown || cli.all {
        rendered.push((
            "claude-code-wrapped.md",
            render_markdown(report).into_bytes(),
        ));
    }
    if cli.card || cli.all {
        rendered.push((
            "claude-code-wrapped-card.html",
            render_share_card(report).into_bytes(),
        ));
    }

    let archive_dir = cli.archive.then(|| cwd.join("wrapped-archive"));
    if let Some(destination) = &archive_dir {
        require_absent_output(destination)?;
        let _ = writeln!(
            io::stderr().lock(),
            "warning: [privacy-profile: private-content] --archive writes private prompt content to a local, explicitly selected output"
        );
    }

    let mut transaction = OutputTransaction::begin(cwd)?;
    for (filename, contents) in rendered {
        transaction.stage_file(cwd.join(filename), &contents)?;
    }
    let archive_projects = if let Some(destination) = archive_dir.as_ref() {
        Some(transaction.stage_archive(destination.clone(), private_prompts)?)
    } else {
        None
    };
    let outputs = transaction.commit()?;

    if cli.open {
        for path in &outputs {
            if path.extension().and_then(|extension| extension.to_str()) == Some("html") {
                open_in_browser(path).map_err(|error| {
                    io::Error::other(format!(
                        "E_BROWSER_OPEN: output files were committed, but the requested browser launch failed: {error}"
                    ))
                })?;
            }
        }
    }
    if let (Some(destination), Some(written)) = (archive_dir, archive_projects) {
        write_stdout_line(format_args!(
            "  Prompt archive: {}/ ({} project{})",
            destination.display(),
            written,
            if written == 1 { "" } else { "s" }
        ))?;
    }

    Ok(outputs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum OutputPathIdentity {
    #[cfg(unix)]
    Unix { device: u64, inode: u64 },
    #[cfg(windows)]
    Windows {
        volume_serial_number: u32,
        file_index: u64,
    },
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutputPublicationManifest {
    schema: String,
    files: Vec<OutputManifestFile>,
    archive: Option<OutputManifestArchive>,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutputManifestFile {
    destination: String,
    prior: OutputManifestPrior,
    installed_identity: OutputPathIdentity,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct OutputManifestArchive {
    destination: String,
    installed_identity: OutputPathIdentity,
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
enum OutputManifestPrior {
    Absent,
    File { identity: OutputPathIdentity },
}

fn output_path_identity(_path: &Path, metadata: &fs::Metadata) -> Option<OutputPathIdentity> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        Some(OutputPathIdentity::Unix {
            device: metadata.dev(),
            inode: metadata.ino(),
        })
    }
    #[cfg(windows)]
    {
        let _ = metadata;
        crate::windows_private_acl::file_identity(_path).ok().map(
            |(volume_serial_number, file_index)| OutputPathIdentity::Windows {
                volume_serial_number,
                file_index,
            },
        )
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (_path, metadata);
        None
    }
}

fn capture_output_path_identity(path: &Path) -> io::Result<OutputPathIdentity> {
    let metadata = fs::symlink_metadata(path)?;
    output_path_identity(path, &metadata).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "safe output replacement requires filesystem object identities",
        )
    })
}

#[derive(Debug, Clone)]
enum PriorOutput {
    Absent,
    File {
        identity: OutputPathIdentity,
        len: u64,
        modified: Option<std::time::SystemTime>,
        readonly: bool,
        permissions: fs::Permissions,
    },
}

impl PriorOutput {
    fn capture(path: &Path) -> io::Result<Self> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                Ok(Self::File {
                    identity: output_path_identity(path, &metadata).ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::Unsupported,
                            "safe output replacement requires filesystem object identities",
                        )
                    })?,
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                    readonly: metadata.permissions().readonly(),
                    permissions: metadata.permissions(),
                })
            }
            Ok(_) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "standard output destination must be absent or a regular file",
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::Absent),
            Err(error) => Err(error),
        }
    }

    fn validate(&self, path: &Path) -> io::Result<()> {
        match (self, fs::symlink_metadata(path)) {
            (Self::Absent, Err(error)) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            (
                Self::File {
                    identity,
                    len,
                    modified,
                    readonly,
                    ..
                },
                Ok(metadata),
            ) if metadata.is_file()
                && !metadata.file_type().is_symlink()
                && output_path_identity(path, &metadata) == Some(*identity)
                && metadata.len() == *len
                && metadata.modified().ok() == *modified
                && metadata.permissions().readonly() == *readonly =>
            {
                Ok(())
            }
            _ => Err(io::Error::other(
                "an output destination changed while the report was staged; retry",
            )),
        }
    }

    fn permissions(&self) -> Option<&fs::Permissions> {
        match self {
            Self::Absent => None,
            Self::File { permissions, .. } => Some(permissions),
        }
    }
}

#[derive(Debug)]
struct OutputDirectoryLock {
    connection: rusqlite::Connection,
}

impl Drop for OutputDirectoryLock {
    fn drop(&mut self) {
        let _ = self.connection.execute_batch("ROLLBACK;");
    }
}

fn acquire_output_directory_lock(output_dir: &Path) -> io::Result<OutputDirectoryLock> {
    let path = output_dir.join(".ccwrapped-output.lock.sqlite3");
    match create_private_output_control_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            validate_private_output_control_file(&path)?;
        }
        Err(error) => return Err(error),
    }
    let flags =
        rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let connection = rusqlite::Connection::open_with_flags(&path, flags)
        .map_err(|error| io::Error::other(format!("open output lock: {error}")))?;
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
            io::Error::other(format!(
                "lock output directory: {error}; wait for the other ccwrapped output transaction to finish"
            ))
        })?;
    protect_private_output_control_file(&path)?;
    let journal = path.with_file_name(format!(
        "{}-journal",
        path.file_name().unwrap_or_default().to_string_lossy()
    ));
    if journal.exists() {
        protect_private_output_control_file(&journal)?;
    }
    Ok(OutputDirectoryLock { connection })
}

fn create_private_output_control_file(path: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        crate::windows_private_acl::create_private_new(path)
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
        options.open(path)?;
        protect_private_output_control_file(path)
    }
}

fn validate_private_output_control_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an output transaction control artifact is not a regular file",
        ));
    }
    protect_private_output_control_file(path)
}

fn protect_private_output_control_file(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
    }
    #[cfg(windows)]
    {
        crate::windows_private_acl::protect(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private output transactions are unsupported on this platform",
        ))
    }
}

fn write_private_output_control_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    create_private_output_control_file(path)?;
    let result = (|| {
        let mut file = OpenOptions::new().write(true).open(path)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn output_destination_name(output_dir: &Path, destination: &Path) -> io::Result<String> {
    let relative = destination.strip_prefix(output_dir).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "output destinations must remain inside the selected output directory",
        )
    })?;
    let mut components = relative.components();
    let Some(std::path::Component::Normal(name)) = components.next() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output destination has no ordinary filename",
        ));
    };
    if components.next().is_some() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "output destinations must be direct children of the output directory",
        ));
    }
    name.to_str().map(str::to_string).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "output destination filenames must be valid UTF-8",
        )
    })
}

fn recover_incomplete_output_transactions(output_dir: &Path) -> io::Result<()> {
    let mut staging = fs::read_dir(output_dir)?
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".ccwrapped-output-stage-")
        })
        .map(|entry| entry.path())
        .collect::<Vec<_>>();
    if staging.len() > 128 {
        return Err(io::Error::other(
            "too many incomplete output transactions require recovery",
        ));
    }
    staging.sort();
    for root in staging {
        recover_incomplete_output_transaction(output_dir, &root)?;
    }
    Ok(())
}

fn recover_incomplete_output_transaction(output_dir: &Path, root: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an incomplete output transaction path is not a real directory",
        ));
    }
    let transaction_path = root.join("transaction.json");
    if !transaction_path.exists() {
        let staging_path = root.join("staging.json");
        let staging = read_bounded_output_control(&staging_path)?;
        if staging != b"{\"schema\":\"ccwrapped.output-staging/v1\"}\n" {
            return Err(io::Error::other(
                "an incomplete output staging marker is invalid",
            ));
        }
        fs::remove_dir_all(root)?;
        sync_archive_directory(output_dir)?;
        return Ok(());
    }
    let bytes = read_bounded_output_control(&transaction_path)?;
    let manifest: OutputPublicationManifest =
        serde_json::from_slice(&bytes).map_err(io::Error::other)?;
    validate_output_publication_manifest(&manifest)?;
    if root.join("committed").exists() {
        let committed = read_bounded_output_control(&root.join("committed"))?;
        if committed != b"ccwrapped.output-committed/v1\n" {
            return Err(io::Error::other(
                "an output transaction completion marker is invalid",
            ));
        }
        fs::remove_dir_all(root)?;
        sync_archive_directory(output_dir)?;
        return Ok(());
    }

    if let Some(archive) = &manifest.archive {
        recover_manifest_absent_destination(
            &output_dir.join(&archive.destination),
            &root.join("recovered-current-archive"),
            archive.installed_identity,
            RollbackPathKind::Directory,
        )?;
    }
    for (index, file) in manifest.files.iter().enumerate().rev() {
        recover_manifest_file(output_dir, root, index, file)?;
    }
    sync_archive_directory(output_dir)?;
    fs::remove_dir_all(root)?;
    sync_archive_directory(output_dir)
}

fn read_bounded_output_control(path: &Path) -> io::Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 1024 * 1024 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an output transaction control file is invalid or oversized",
        ));
    }
    fs::read(path)
}

fn validate_output_publication_manifest(manifest: &OutputPublicationManifest) -> io::Result<()> {
    if manifest.schema != "ccwrapped.output-publication/v1" || manifest.files.len() > 3 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "an output transaction manifest has an unsupported contract",
        ));
    }
    let mut names = std::collections::BTreeSet::new();
    for name in manifest
        .files
        .iter()
        .map(|file| file.destination.as_str())
        .chain(
            manifest
                .archive
                .iter()
                .map(|archive| archive.destination.as_str()),
        )
    {
        let path = Path::new(name);
        if path.components().count() != 1
            || !matches!(
                path.components().next(),
                Some(std::path::Component::Normal(_))
            )
            || !names.insert(name)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "an output transaction manifest contains an unsafe destination",
            ));
        }
    }
    Ok(())
}

fn recover_manifest_file(
    output_dir: &Path,
    root: &Path,
    index: usize,
    file: &OutputManifestFile,
) -> io::Result<()> {
    let destination = output_dir.join(&file.destination);
    let backup = root.join(format!("backup-{index}"));
    match &file.prior {
        OutputManifestPrior::Absent => recover_manifest_absent_destination(
            &destination,
            &root.join(format!("recovered-current-{index}")),
            file.installed_identity,
            RollbackPathKind::File,
        ),
        OutputManifestPrior::File { identity } => {
            match existing_regular_output_identity(&destination, RollbackPathKind::File)? {
                Some(observed) if observed == file.installed_identity => {
                    rename_archive_noreplace(
                        &destination,
                        &root.join(format!("recovered-current-{index}")),
                    )?;
                }
                Some(observed) if observed == *identity => {
                    if backup.exists() {
                        return Err(io::Error::other(
                            "both the prior output and its recovery backup exist",
                        ));
                    }
                    return Ok(());
                }
                Some(_) => {
                    return Err(io::Error::other(
                        "an output destination changed before crash recovery",
                    ))
                }
                None => {}
            }
            let backup_identity =
                existing_regular_output_identity(&backup, RollbackPathKind::File)?.ok_or_else(
                    || io::Error::other("the prior output recovery backup is missing"),
                )?;
            if backup_identity != *identity {
                return Err(io::Error::other(
                    "the prior output recovery backup changed before recovery",
                ));
            }
            rename_archive_noreplace(&backup, &destination)
        }
    }
}

fn recover_manifest_absent_destination(
    destination: &Path,
    recovery: &Path,
    installed_identity: OutputPathIdentity,
    kind: RollbackPathKind,
) -> io::Result<()> {
    match existing_regular_output_identity(destination, kind)? {
        None => Ok(()),
        Some(identity) if identity == installed_identity => {
            rename_archive_noreplace(destination, recovery)
        }
        Some(_) => Err(io::Error::other(
            "an output destination changed before crash recovery",
        )),
    }
}

fn existing_regular_output_identity(
    path: &Path,
    kind: RollbackPathKind,
) -> io::Result<Option<OutputPathIdentity>> {
    match fs::symlink_metadata(path) {
        Ok(metadata)
            if !metadata.file_type().is_symlink()
                && match kind {
                    RollbackPathKind::File => metadata.is_file(),
                    RollbackPathKind::Directory => metadata.is_dir(),
                } =>
        {
            output_path_identity(path, &metadata)
                .map(Some)
                .ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::Unsupported,
                        "safe output recovery requires filesystem object identities",
                    )
                })
        }
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "an output recovery path changed type",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[derive(Debug)]
struct StagedOutputFile {
    destination: PathBuf,
    staged: PathBuf,
    backup: PathBuf,
    prior: PriorOutput,
    installed_identity: OutputPathIdentity,
    backup_moved: bool,
    installed: bool,
}

#[derive(Debug)]
struct StagedOutputArchive {
    destination: PathBuf,
    staged: PathBuf,
    installed_identity: OutputPathIdentity,
    installed: bool,
}

#[derive(Debug)]
struct OutputTransaction {
    _lock: OutputDirectoryLock,
    root: PathBuf,
    output_dir: PathBuf,
    files: Vec<StagedOutputFile>,
    archive: Option<StagedOutputArchive>,
    committed: bool,
    rollback_attempted: bool,
    preserve_root: bool,
}

impl OutputTransaction {
    fn begin(output_dir: &Path) -> io::Result<Self> {
        static NEXT_OUTPUT_STAGE: AtomicU64 = AtomicU64::new(0);

        let output_lock = acquire_output_directory_lock(output_dir)?;
        recover_incomplete_output_transactions(output_dir)?;
        for _ in 0..128 {
            let sequence = NEXT_OUTPUT_STAGE.fetch_add(1, Ordering::Relaxed);
            let root = output_dir.join(format!(
                ".ccwrapped-output-stage-{}-{sequence}",
                std::process::id()
            ));
            match create_private_archive_dir(&root) {
                Ok(()) => {
                    write_private_output_control_file(
                        &root.join("staging.json"),
                        b"{\"schema\":\"ccwrapped.output-staging/v1\"}\n",
                    )?;
                    sync_archive_directory(&root)?;
                    return Ok(Self {
                        _lock: output_lock,
                        root,
                        output_dir: output_dir.to_path_buf(),
                        files: Vec::new(),
                        archive: None,
                        committed: false,
                        rollback_attempted: false,
                        preserve_root: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve an output staging directory; retry the invocation",
        ))
    }

    fn stage_file(&mut self, destination: PathBuf, contents: &[u8]) -> io::Result<()> {
        let prior = PriorOutput::capture(&destination)?;
        let index = self.files.len();
        let staged = self.root.join(format!("new-{index}"));
        let backup = self.root.join(format!("backup-{index}"));
        write_staged_output_file(&staged, contents, prior.permissions())?;
        let installed_identity = capture_output_path_identity(&staged)?;
        self.files.push(StagedOutputFile {
            destination,
            staged,
            backup,
            prior,
            installed_identity,
            backup_moved: false,
            installed: false,
        });
        Ok(())
    }

    fn stage_archive(
        &mut self,
        destination: PathBuf,
        private_prompts: &[ingestion::PrivatePrompt],
    ) -> Result<usize, Box<dyn Error>> {
        if self.archive.is_some() {
            return Err(io::Error::other("only one private archive may be staged").into());
        }
        require_absent_output(&destination)?;
        let staged = self.root.join("new-archive");
        create_private_archive_dir(&staged)?;
        let written = write_archive_contents(&staged, private_prompts)?;
        let installed_identity = capture_output_path_identity(&staged)?;
        self.archive = Some(StagedOutputArchive {
            destination,
            staged,
            installed_identity,
            installed: false,
        });
        Ok(written)
    }

    fn commit(self) -> io::Result<Vec<PathBuf>> {
        self.commit_with_hook(|| {})
    }

    #[cfg(test)]
    fn abandon_for_recovery(mut self) {
        self.committed = true;
        self.preserve_root = true;
    }

    fn commit_with_hook(
        mut self,
        before_archive_commit: impl FnOnce(),
    ) -> io::Result<Vec<PathBuf>> {
        match self.commit_inner(before_archive_commit) {
            Ok(outputs) => {
                self.committed = true;
                Ok(outputs)
            }
            Err(commit_error) => match self.rollback() {
                Ok(()) => Err(commit_error),
                Err(rollback_error) => Err(io::Error::other(format!(
                    "output commit failed ({commit_error}); {rollback_error}"
                ))),
            },
        }
    }

    fn commit_inner(&mut self, before_archive_commit: impl FnOnce()) -> io::Result<Vec<PathBuf>> {
        for file in &self.files {
            file.prior.validate(&file.destination)?;
        }
        if let Some(archive) = &self.archive {
            require_absent_output(&archive.destination)?;
        }
        self.persist_publication_manifest()?;
        sync_archive_directory(&self.root)?;

        for file in &mut self.files {
            match &file.prior {
                PriorOutput::Absent => {
                    rename_archive_noreplace(&file.staged, &file.destination)?;
                }
                PriorOutput::File { .. } => {
                    rename_archive_noreplace(&file.destination, &file.backup)?;
                    file.backup_moved = true;
                    file.prior.validate(&file.backup)?;
                    rename_archive_noreplace(&file.staged, &file.destination)?;
                }
            }
            file.installed = true;
            if capture_output_path_identity(&file.destination)? != file.installed_identity {
                return Err(io::Error::other(
                    "an output destination changed during publication; retry",
                ));
            }
        }
        before_archive_commit();
        if let Some(archive) = &mut self.archive {
            rename_archive_noreplace(&archive.staged, &archive.destination)?;
            archive.installed = true;
            if capture_output_path_identity(&archive.destination)? != archive.installed_identity {
                return Err(io::Error::other(
                    "the private archive destination changed during publication; retry",
                ));
            }
        }
        sync_archive_directory(&self.output_dir)?;
        write_private_output_control_file(
            &self.root.join("committed"),
            b"ccwrapped.output-committed/v1\n",
        )?;
        sync_archive_directory(&self.root)?;

        let outputs = self
            .files
            .iter()
            .map(|file| file.destination.clone())
            .collect();
        Ok(outputs)
    }

    fn persist_publication_manifest(&self) -> io::Result<()> {
        let files = self
            .files
            .iter()
            .map(|file| {
                Ok(OutputManifestFile {
                    destination: output_destination_name(&self.output_dir, &file.destination)?,
                    prior: match &file.prior {
                        PriorOutput::Absent => OutputManifestPrior::Absent,
                        PriorOutput::File { identity, .. } => OutputManifestPrior::File {
                            identity: *identity,
                        },
                    },
                    installed_identity: file.installed_identity,
                })
            })
            .collect::<io::Result<Vec<_>>>()?;
        let archive = self
            .archive
            .as_ref()
            .map(|archive| {
                Ok::<OutputManifestArchive, io::Error>(OutputManifestArchive {
                    destination: output_destination_name(&self.output_dir, &archive.destination)?,
                    installed_identity: archive.installed_identity,
                })
            })
            .transpose()?;
        let manifest = OutputPublicationManifest {
            schema: "ccwrapped.output-publication/v1".to_string(),
            files,
            archive,
        };
        let mut bytes = serde_json::to_vec(&manifest).map_err(io::Error::other)?;
        bytes.push(b'\n');
        write_private_output_control_file(&self.root.join("transaction.json"), &bytes)
    }

    fn rollback(&mut self) -> io::Result<()> {
        self.rollback_attempted = true;
        let mut failures = Vec::new();

        if let Some(archive) = &mut self.archive {
            if archive.installed {
                let recovery = self.root.join("rollback-current-archive");
                if let Err(error) = recover_installed_output(
                    &archive.destination,
                    &recovery,
                    archive.installed_identity,
                    RollbackPathKind::Directory,
                ) {
                    failures.push(format!("private archive: {error}"));
                }
                archive.installed = false;
            }
        }
        for (index, file) in self.files.iter_mut().enumerate().rev() {
            if file.installed {
                let recovery = self.root.join(format!("rollback-current-{index}"));
                if let Err(error) = recover_installed_output(
                    &file.destination,
                    &recovery,
                    file.installed_identity,
                    RollbackPathKind::File,
                ) {
                    failures.push(format!("standard output {}: {error}", index + 1));
                }
                file.installed = false;
            }
            if file.backup_moved {
                match rename_archive_noreplace(&file.backup, &file.destination) {
                    Ok(()) => file.backup_moved = false,
                    Err(error) => failures.push(format!(
                        "standard output {} prior file remains in recovery staging: {error}",
                        index + 1
                    )),
                }
            }
        }
        if let Err(error) = sync_archive_directory(&self.output_dir) {
            failures.push(format!("output directory sync failed: {error}"));
        }

        if failures.is_empty() {
            Ok(())
        } else {
            self.preserve_root = true;
            let staging_name = self
                .root
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "the hidden output staging directory".to_string());
            Err(io::Error::other(format!(
                "rollback incomplete; protected recovery staging `{staging_name}` was retained ({})",
                failures.join("; ")
            )))
        }
    }
}

impl Drop for OutputTransaction {
    fn drop(&mut self) {
        if !self.committed && !self.rollback_attempted {
            let _ = self.rollback();
        }
        if !self.preserve_root {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum RollbackPathKind {
    File,
    Directory,
}

fn recover_installed_output(
    destination: &Path,
    recovery: &Path,
    installed_identity: OutputPathIdentity,
    kind: RollbackPathKind,
) -> io::Result<()> {
    let observed_identity = match fs::symlink_metadata(destination) {
        Ok(metadata) => output_path_identity(destination, &metadata).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::Unsupported,
                "cannot identify the current filesystem object",
            )
        })?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if observed_identity != installed_identity {
        return Ok(());
    }

    match rename_archive_noreplace(destination, recovery) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    }
    let moved_identity = match capture_output_path_identity(recovery) {
        Ok(identity) => identity,
        Err(identity_error) => {
            return match rename_archive_noreplace(recovery, destination) {
                Ok(()) => Err(io::Error::other(format!(
                    "could not verify the moved object and restored it: {identity_error}"
                ))),
                Err(restore_error) => Err(io::Error::other(format!(
                    "could not verify the moved object ({identity_error}); it remains in recovery staging because restoration failed: {restore_error}"
                ))),
            };
        }
    };
    if moved_identity != installed_identity {
        return match rename_archive_noreplace(recovery, destination) {
            Ok(()) => Ok(()),
            Err(error) => Err(io::Error::other(format!(
                "a competing object remains in recovery staging because restoration failed: {error}"
            ))),
        };
    }

    match kind {
        RollbackPathKind::File => fs::remove_file(recovery),
        RollbackPathKind::Directory => fs::remove_dir_all(recovery),
    }
}

fn require_absent_output(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(existing_archive_error()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn write_staged_output_file(
    path: &Path,
    contents: &[u8],
    prior_permissions: Option<&fs::Permissions>,
) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o666);
    }
    let mut file = options.open(path)?;
    file.write_all(contents)?;
    if let Some(permissions) = prior_permissions {
        file.set_permissions(permissions.clone())?;
    }
    file.sync_all()
}

fn print_summary(
    entry_count: usize,
    selected_year: i32,
    report: &Report,
    choice: termcolor::ColorChoice,
) -> io::Result<()> {
    write_stdout_line(format_args!(
        "  {} entries, {} days, {} sessions ({})",
        entry_count,
        report.cost_analysis.daily_costs.len(),
        report.session_breakdown.sessions.len(),
        selected_year,
    ))?;
    try_render_terminal_with(report, choice)
}

fn print_coverage_summary(coverage: &ccwrapped::DataCoverage) -> io::Result<()> {
    let completeness = terminal_text(&coverage.completeness);
    write_stdout_line(format_args!(
        "  Coverage: {} source{}, {} file{}, {} accepted; {}",
        coverage.source_root_count,
        if coverage.source_root_count == 1 {
            ""
        } else {
            "s"
        },
        coverage.files_discovered,
        if coverage.files_discovered == 1 {
            ""
        } else {
            "s"
        },
        coverage.accepted_records,
        completeness,
    ))?;
    for source in &coverage.sources {
        let alias = terminal_text(&source.alias);
        let kind = terminal_text(&source.kind);
        write_stdout_line(format_args!(
            "    {} ({}) — {} file{}, {} accepted",
            alias,
            kind,
            source.files_discovered,
            if source.files_discovered == 1 {
                ""
            } else {
                "s"
            },
            source.accepted_records,
        ))?;
    }
    for warning in &coverage.warnings {
        let code = terminal_text(&warning.code);
        let message = terminal_text(&warning.message);
        write_stdout_line(format_args!("    warning {code}: {message}"))?;
    }
    Ok(())
}

#[cfg(test)]
fn write_archive(
    archive_dir: &Path,
    private_prompts: &[ingestion::PrivatePrompt],
) -> Result<usize, Box<dyn Error>> {
    let transaction = PrivateArchiveTransaction::begin(archive_dir)?;
    let written = write_archive_contents(transaction.staging_dir(), private_prompts)?;
    transaction.commit()?;
    Ok(written)
}

fn write_archive_contents(
    staging_dir: &Path,
    private_prompts: &[ingestion::PrivatePrompt],
) -> Result<usize, Box<dyn Error>> {
    let mut by_project: BTreeMap<String, Vec<&ingestion::PrivatePrompt>> = BTreeMap::new();
    let mut slug_counts: HashMap<String, usize> = HashMap::new();

    for prompt in private_prompts {
        by_project
            .entry(prompt.project_alias.clone())
            .or_default()
            .push(prompt);
    }

    let mut written = 0usize;
    for (project_name, mut prompts) in by_project {
        if prompts.is_empty() {
            continue;
        }
        prompts.sort_by(|left, right| {
            left.timestamp
                .cmp(&right.timestamp)
                .then_with(|| left.session_alias.cmp(&right.session_alias))
        });

        let base_slug = if project_slug(&project_name).is_empty() {
            "unknown".to_string()
        } else {
            project_slug(&project_name)
        };
        let count = slug_counts.entry(base_slug.clone()).or_insert(0);
        *count = count.saturating_add(1);
        let filename = if *count == 1 {
            format!("{base_slug}.md")
        } else {
            format!("{base_slug}-{}.md", *count)
        };
        let top = prompts.into_iter().take(5).collect::<Vec<_>>();
        let mut lines = vec![
            "<!-- privacy-profile: private-content -->".to_string(),
            String::new(),
            format!("# {}", project_name),
            String::new(),
            format!(
                "_Top {} prompt{}_",
                top.len(),
                if top.len() == 1 { "" } else { "s" }
            ),
            String::new(),
        ];
        for prompt in top {
            lines.push("---".to_string());
            lines.push(String::new());
            lines.push(format!(
                "**{}**",
                prompt.timestamp.chars().take(10).collect::<String>()
            ));
            lines.push(String::new());
            if let Some(entrypoint) = &prompt.entrypoint {
                lines.push("_Entrypoint:_".to_string());
                lines.push(markdown_literal_block(entrypoint));
                lines.push(String::new());
            }
            let display = if let Some((cutoff, _)) = prompt.text.char_indices().nth(500) {
                format!("{}... [truncated]", &prompt.text[..cutoff])
            } else {
                prompt.text.clone()
            };
            lines.push(markdown_literal_block(&display));
            lines.push(String::new());
        }
        write_private_archive_file(staging_dir, &filename, lines.join("\n").as_bytes())?;
        written = written.saturating_add(1);
    }

    sync_archive_directory(staging_dir)?;
    Ok(written)
}

fn markdown_literal_block(value: &str) -> String {
    let longest_run = value
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat(longest_run.saturating_add(1).max(3));
    format!("{fence}\n{value}\n{fence}")
}

#[cfg(test)]
struct PrivateArchiveTransaction {
    staging_dir: PathBuf,
    destination: PathBuf,
    committed: bool,
}

#[cfg(test)]
impl PrivateArchiveTransaction {
    fn begin(destination: &Path) -> io::Result<Self> {
        static NEXT_ARCHIVE_STAGE: AtomicU64 = AtomicU64::new(0);

        match fs::symlink_metadata(destination) {
            Ok(_) => return Err(existing_archive_error()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let parent = destination.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "private archive output requires a parent directory",
            )
        })?;

        for _ in 0..128 {
            let sequence = NEXT_ARCHIVE_STAGE.fetch_add(1, Ordering::Relaxed);
            let staging_dir = parent.join(format!(
                ".ccwrapped-archive-stage-{}-{sequence}",
                std::process::id()
            ));
            match create_private_archive_dir(&staging_dir) {
                Ok(()) => {
                    return Ok(Self {
                        staging_dir,
                        destination: destination.to_path_buf(),
                        committed: false,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }

        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not reserve a private archive staging directory; retry the invocation",
        ))
    }

    fn staging_dir(&self) -> &Path {
        &self.staging_dir
    }

    fn commit(mut self) -> io::Result<()> {
        rename_archive_noreplace(&self.staging_dir, &self.destination)?;
        self.committed = true;
        Ok(())
    }
}

#[cfg(test)]
impl Drop for PrivateArchiveTransaction {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_dir_all(&self.staging_dir);
        }
    }
}

fn existing_archive_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::AlreadyExists,
        "private archive output already exists; move or remove it before retrying",
    )
}

fn create_private_archive_dir(archive_dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let mut builder = fs::DirBuilder::new();
        builder.mode(0o700).create(archive_dir).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                existing_archive_error()
            } else {
                error
            }
        })?;
        if let Err(error) = fs::set_permissions(archive_dir, fs::Permissions::from_mode(0o700)) {
            let _ = fs::remove_dir(archive_dir);
            return Err(error);
        }
    }

    #[cfg(windows)]
    {
        windows_private_acl::create_private_directory_new(archive_dir).map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                existing_archive_error()
            } else {
                error
            }
        })?;
    }

    #[cfg(not(any(unix, windows)))]
    {
        let _ = archive_dir;
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "private archive output is unsupported on this platform",
        ));
    }

    Ok(())
}

fn write_private_archive_file(
    archive_dir: &Path,
    filename: &str,
    contents: &[u8],
) -> io::Result<()> {
    let destination = archive_dir.join(filename);
    #[cfg(windows)]
    windows_private_acl::create_private_new(&destination)?;
    let mut options = OpenOptions::new();
    options.write(true);
    #[cfg(not(windows))]
    options.create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = match options.open(&destination) {
        Ok(file) => file,
        Err(error) => {
            #[cfg(windows)]
            {
                let _ = fs::remove_file(&destination);
            }
            return Err(error);
        }
    };

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(error) = file.set_permissions(fs::Permissions::from_mode(0o600)) {
            drop(file);
            let _ = fs::remove_file(&destination);
            return Err(error);
        }
    }
    if let Err(error) = file.write_all(contents).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&destination);
        return Err(error);
    }
    Ok(())
}

fn sync_archive_directory(archive_dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    File::open(archive_dir)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = archive_dir;
    Ok(())
}

#[cfg(all(
    target_os = "linux",
    any(
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "loongarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "riscv64",
        target_arch = "s390x",
        target_arch = "x86",
        target_arch = "x86_64"
    )
))]
fn rename_archive_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::c_long;
    use std::os::unix::ffi::OsStrExt;

    const AT_FDCWD: c_long = -100;
    const RENAME_NOREPLACE: c_long = 1;
    #[cfg(target_arch = "x86_64")]
    const SYS_RENAMEAT2: c_long = 316;
    #[cfg(target_arch = "x86")]
    const SYS_RENAMEAT2: c_long = 353;
    #[cfg(target_arch = "arm")]
    const SYS_RENAMEAT2: c_long = 382;
    #[cfg(any(
        target_arch = "aarch64",
        target_arch = "loongarch64",
        target_arch = "riscv64"
    ))]
    const SYS_RENAMEAT2: c_long = 276;
    #[cfg(any(target_arch = "powerpc", target_arch = "powerpc64"))]
    const SYS_RENAMEAT2: c_long = 357;
    #[cfg(target_arch = "s390x")]
    const SYS_RENAMEAT2: c_long = 347;

    #[link(name = "c")]
    extern "C" {
        fn syscall(number: c_long, ...) -> c_long;
    }

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid archive path"))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid archive path"))?;
    let result = unsafe {
        syscall(
            SYS_RENAMEAT2,
            AT_FDCWD,
            from.as_ptr(),
            AT_FDCWD,
            to.as_ptr(),
            RENAME_NOREPLACE,
        )
    };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(all(
    target_os = "linux",
    not(any(
        target_arch = "aarch64",
        target_arch = "arm",
        target_arch = "loongarch64",
        target_arch = "powerpc",
        target_arch = "powerpc64",
        target_arch = "riscv64",
        target_arch = "s390x",
        target_arch = "x86",
        target_arch = "x86_64"
    ))
))]
fn rename_archive_noreplace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private archive atomic publication is unsupported on this Linux architecture",
    ))
}

#[cfg(target_os = "macos")]
fn rename_archive_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    use std::ffi::CString;
    use std::os::raw::c_char;
    use std::os::unix::ffi::OsStrExt;

    const RENAME_EXCL: u32 = 0x0000_0004;

    extern "C" {
        fn renamex_np(old_path: *const c_char, new_path: *const c_char, flags: u32) -> i32;
    }

    let from = CString::new(from.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid archive path"))?;
    let to = CString::new(to.as_os_str().as_bytes())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid archive path"))?;
    let result = unsafe { renamex_np(from.as_ptr(), to.as_ptr(), RENAME_EXCL) };
    if result == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn rename_archive_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    windows_private_acl::move_noreplace(from, to)
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn rename_archive_noreplace(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "private archive atomic publication is unsupported on this platform",
    ))
}

fn open_in_browser(path: &Path) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("rundll32.exe");
        command.arg("url.dll,FileProtocolHandler").arg(path);
        run_browser_command(&mut command)?;
    }
    #[cfg(target_os = "macos")]
    {
        run_browser_command(Command::new("open").arg(path))?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        run_browser_command(Command::new("xdg-open").arg(path))?;
    }
    Ok(())
}

fn run_browser_command(command: &mut Command) -> io::Result<()> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "browser launcher exited with status {status}"
        )))
    }
}

#[cfg(test)]
mod archive_tests {
    use super::{write_archive, write_outputs, Cli, OutputTransaction, PrivateArchiveTransaction};
    use crate::ingestion::PrivatePrompt;
    use ccwrapped::Report;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn output_cli() -> Cli {
        Cli {
            data_dirs: Vec::new(),
            otel_files: Vec::new(),
            private_diagnostics: false,
            html: true,
            markdown: false,
            card: false,
            archive: true,
            all: false,
            open: false,
            json: false,
            plain: true,
            timezone: Some("UTC".to_string()),
            active_threshold_minutes: 5,
            ingestion_workers: None,
            ingestion_delay_seed: None,
            ingestion_panic_file: None,
            benchmark_counters: None,
            no_store: false,
            rebuild_store: false,
            store_path: None,
            year: Some(2026),
        }
    }

    #[test]
    fn output_failure_rolls_back_all_requested_destinations() {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-output-transaction-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create output transaction test root");
        let html = root.join("claude-code-wrapped.html");
        let archive = root.join("wrapped-archive");
        fs::write(&html, "HTML_SENTINEL").expect("write prior HTML");
        fs::create_dir(&archive).expect("create conflicting archive");
        fs::write(archive.join("prior.txt"), "ARCHIVE_SENTINEL")
            .expect("write prior archive marker");

        assert!(write_outputs(&root, &Report::default(), &[], &output_cli()).is_err());
        assert_eq!(fs::read_to_string(&html).unwrap(), "HTML_SENTINEL");
        assert_eq!(
            fs::read_to_string(archive.join("prior.txt")).unwrap(),
            "ARCHIVE_SENTINEL"
        );
        assert!(fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .all(|name| !name
                .to_string_lossy()
                .starts_with(".ccwrapped-output-stage-")));

        fs::remove_dir_all(&archive).expect("remove archive conflict");
        let outputs = write_outputs(&root, &Report::default(), &[], &output_cli())
            .expect("retry complete output transaction");
        assert_eq!(outputs, vec![html.clone()]);
        assert_ne!(fs::read_to_string(&html).unwrap(), "HTML_SENTINEL");
        assert!(archive.is_dir());
        fs::remove_dir_all(root).expect("remove output transaction test root");
    }

    #[test]
    fn output_commit_race_restores_all_prior_destinations() {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-output-race-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create output race test root");
        let html = root.join("claude-code-wrapped.html");
        let markdown = root.join("claude-code-wrapped.md");
        let archive = root.join("wrapped-archive");
        fs::write(&html, "HTML_SENTINEL").expect("write prior HTML");
        fs::write(&markdown, "MARKDOWN_SENTINEL").expect("write prior Markdown");

        let mut transaction = OutputTransaction::begin(&root).expect("begin output transaction");
        transaction
            .stage_file(html.clone(), b"replacement")
            .expect("stage HTML replacement");
        transaction
            .stage_file(markdown.clone(), b"replacement")
            .expect("stage Markdown replacement");
        transaction
            .stage_archive(archive.clone(), &[])
            .expect("stage private archive");
        let result = transaction.commit_with_hook(|| {
            fs::create_dir(&archive).expect("create competing archive");
            fs::write(archive.join("competitor.txt"), "preserve me")
                .expect("write competing archive marker");
        });

        assert!(result.is_err());
        assert_eq!(fs::read_to_string(&html).unwrap(), "HTML_SENTINEL");
        assert_eq!(fs::read_to_string(&markdown).unwrap(), "MARKDOWN_SENTINEL");
        assert_eq!(
            fs::read_to_string(archive.join("competitor.txt")).unwrap(),
            "preserve me"
        );
        assert!(fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .all(|name| !name
                .to_string_lossy()
                .starts_with(".ccwrapped-output-stage-")));
        fs::remove_dir_all(root).expect("remove output race test root");
    }

    #[test]
    fn next_output_transaction_recovers_a_crash_between_file_renames() {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-output-crash-recovery-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create output crash-recovery root");
        let html = root.join("claude-code-wrapped.html");
        let markdown = root.join("claude-code-wrapped.md");
        fs::write(&html, "PRIOR_HTML_SENTINEL").expect("write prior HTML");

        let mut transaction = OutputTransaction::begin(&root).expect("begin output transaction");
        transaction
            .stage_file(html.clone(), b"NEW_HTML")
            .expect("stage HTML replacement");
        transaction
            .stage_file(markdown.clone(), b"NEW_MARKDOWN")
            .expect("stage new Markdown");
        transaction
            .persist_publication_manifest()
            .expect("persist recovery manifest");
        super::sync_archive_directory(&transaction.root).expect("sync recovery manifest");
        fs::rename(
            &transaction.files[0].destination,
            &transaction.files[0].backup,
        )
        .expect("simulate prior-output displacement");
        fs::rename(
            &transaction.files[0].staged,
            &transaction.files[0].destination,
        )
        .expect("simulate replacement installation");
        fs::rename(
            &transaction.files[1].staged,
            &transaction.files[1].destination,
        )
        .expect("simulate new-output installation");
        super::sync_archive_directory(&root).expect("sync simulated partial publication");
        transaction.abandon_for_recovery();

        let recovery =
            OutputTransaction::begin(&root).expect("next transaction recovers partial publication");
        drop(recovery);
        assert_eq!(
            fs::read_to_string(&html).expect("read recovered prior HTML"),
            "PRIOR_HTML_SENTINEL"
        );
        assert!(
            !markdown.exists(),
            "recovery retained a partially new output"
        );
        assert!(fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .all(|name| !name
                .to_string_lossy()
                .starts_with(".ccwrapped-output-stage-")));
        fs::remove_dir_all(root).expect("remove output crash-recovery root");
    }

    #[test]
    fn output_rollback_preserves_competing_standard_destination() {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-output-rollback-race-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create rollback race test root");
        let html = root.join("claude-code-wrapped.html");
        let displaced_report = root.join("transaction-report-moved-by-competitor.html");
        let archive = root.join("wrapped-archive");
        fs::write(&html, "PRIOR_HTML_SENTINEL").expect("write prior HTML");

        let mut transaction = OutputTransaction::begin(&root).expect("begin output transaction");
        transaction
            .stage_file(html.clone(), b"TRANSACTION_REPLACEMENT")
            .expect("stage HTML replacement");
        transaction
            .stage_archive(archive.clone(), &[])
            .expect("stage private archive");
        let result = transaction.commit_with_hook(|| {
            fs::rename(&html, &displaced_report).expect("move installed transaction report");
            fs::write(&html, "COMPETING_HTML_SENTINEL")
                .expect("write competing standard destination");
            fs::create_dir(&archive).expect("create competing archive");
            fs::write(archive.join("competitor.txt"), "COMPETING_ARCHIVE_SENTINEL")
                .expect("write competing archive marker");
        });

        let error = result.expect_err("archive collision must fail the output transaction");
        assert!(error.to_string().contains("rollback incomplete"));
        assert_eq!(
            fs::read_to_string(&html).expect("competing destination must remain"),
            "COMPETING_HTML_SENTINEL"
        );
        assert_eq!(
            fs::read_to_string(&displaced_report).expect("moved report must remain"),
            "TRANSACTION_REPLACEMENT"
        );
        assert_eq!(
            fs::read_to_string(archive.join("competitor.txt"))
                .expect("competing archive must remain"),
            "COMPETING_ARCHIVE_SENTINEL"
        );
        let staging = fs::read_dir(&root)
            .expect("read rollback race test root")
            .map(|entry| entry.expect("read rollback race entry").path())
            .find(|path| {
                path.file_name().is_some_and(|name| {
                    name.to_string_lossy()
                        .starts_with(".ccwrapped-output-stage-")
                })
            })
            .expect("ambiguous rollback must retain protected recovery staging");
        let staging_name = staging
            .file_name()
            .expect("staging directory has a name")
            .to_string_lossy();
        assert!(error.to_string().contains(staging_name.as_ref()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;

            assert_eq!(
                fs::metadata(&staging)
                    .expect("stat recovery staging")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert!(fs::read_dir(&staging)
            .expect("read recovery staging")
            .filter_map(Result::ok)
            .any(|entry| matches!(
                fs::read_to_string(entry.path()),
                Ok(contents) if contents == "PRIOR_HTML_SENTINEL"
            )));

        fs::remove_dir_all(root).expect("remove rollback race test root");
    }

    #[cfg(unix)]
    #[test]
    fn output_transaction_refuses_standard_symlinks() {
        use std::os::unix::fs::symlink;

        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-output-symlink-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create output symlink test root");
        let target = root.join("external-target.txt");
        let html = root.join("claude-code-wrapped.html");
        fs::write(&target, "TARGET_SENTINEL").expect("write external target");
        symlink(&target, &html).expect("plant standard output symlink");
        let mut cli = output_cli();
        cli.archive = false;

        assert!(write_outputs(&root, &Report::default(), &[], &cli).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "TARGET_SENTINEL");
        assert!(fs::symlink_metadata(&html)
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(fs::read_dir(&root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .all(|name| !name
                .to_string_lossy()
                .starts_with(".ccwrapped-output-stage-")));
        fs::remove_dir_all(root).expect("remove output symlink test root");
    }

    #[test]
    fn archive_failure_removes_staging_and_allows_retry() {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-archive-transaction-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create archive transaction test root");
        let archive = root.join("wrapped-archive");
        let prompt = |project_alias: String| PrivatePrompt {
            project_alias,
            session_alias: "session-1".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            text: "private prompt".to_string(),
            entrypoint: None,
        };

        let invalid = vec![prompt("a".to_string()), prompt("z".repeat(300))];
        assert!(write_archive(&archive, &invalid).is_err());
        assert!(
            !archive.exists(),
            "failed archive transaction published a partial final directory"
        );
        let residue = fs::read_dir(&root)
            .expect("read archive transaction test root")
            .collect::<Result<Vec<_>, _>>()
            .expect("read archive transaction entries");
        assert!(
            residue.is_empty(),
            "failed archive transaction left staging residue"
        );

        assert_eq!(
            write_archive(&archive, &[prompt("project-1".to_string())]).unwrap(),
            1
        );
        assert!(archive.join("project-1.md").is_file());
        fs::remove_dir_all(root).expect("remove archive transaction test root");
    }

    #[test]
    fn archive_renders_hostile_prompts_as_inert_markdown() {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-hostile-archive-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create hostile archive test root");
        let archive = root.join("wrapped-archive");
        let prompt = PrivatePrompt {
            project_alias: "project-1".to_string(),
            session_alias: "session-1".to_string(),
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            text:
                "<img src=\"https://attacker.invalid/?secret\">\n[click](javascript:alert(1))\n```"
                    .to_string(),
            entrypoint: Some("![entry](https://attacker.invalid/entry)".to_string()),
        };

        assert_eq!(write_archive(&archive, &[prompt]).unwrap(), 1);
        let markdown =
            fs::read_to_string(archive.join("project-1.md")).expect("read hostile private archive");
        let mut fence_length = None;
        for line in markdown.lines() {
            let trimmed = line.trim();
            let run_length = trimmed
                .chars()
                .all(|character| character == '`')
                .then_some(trimmed.len())
                .filter(|length| *length >= 3);
            if fence_length.is_none() && run_length.is_some() {
                fence_length = run_length;
                continue;
            }
            if fence_length
                .is_some_and(|opening| run_length.is_some_and(|closing| closing >= opening))
            {
                fence_length = None;
                continue;
            }
            if line.contains("<img") || line.contains("javascript:") || line.contains("![entry]") {
                assert!(
                    fence_length.is_some(),
                    "active private Markdown escaped its code fence"
                );
            }
        }
        assert!(
            fence_length.is_none(),
            "private Markdown fence was not closed"
        );
        fs::remove_dir_all(root).expect("remove hostile archive test root");
    }

    #[test]
    fn archive_commit_never_clobbers_a_competing_destination() {
        static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock must follow Unix epoch")
            .as_nanos();
        let sequence = NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ccwrapped-archive-race-{}-{nonce}-{sequence}",
            std::process::id()
        ));
        fs::create_dir(&root).expect("create archive race test root");
        let archive = root.join("wrapped-archive");
        let transaction =
            PrivateArchiveTransaction::begin(&archive).expect("begin archive transaction");
        let staging = transaction.staging_dir().to_path_buf();
        fs::create_dir(&archive).expect("create competing archive destination");
        fs::write(archive.join("competitor.txt"), "preserve me")
            .expect("write competing archive marker");

        assert!(transaction.commit().is_err());
        assert_eq!(
            fs::read_to_string(archive.join("competitor.txt")).unwrap(),
            "preserve me"
        );
        assert!(
            !staging.exists(),
            "failed commit left its staging directory"
        );
        fs::remove_dir_all(root).expect("remove archive race test root");
    }
}
