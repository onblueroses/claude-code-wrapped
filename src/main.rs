use ccwrapped::analyzers::cache::{analyze_cache_health, detect_inflection_points};
use ccwrapped::analyzers::cost::analyze_usage;
use ccwrapped::analyzers::models::{
    analyze_model_routing, analyze_session_intelligence, detect_anomalies,
};
use ccwrapped::analyzers::recommendations::generate_recommendations;
use ccwrapped::analyzers::story::build_wrapped_story;
use ccwrapped::readers::jsonl::{aggregate_by_project, aggregate_daily, read_all_jsonl};
use ccwrapped::readers::session::read_session_breakdown;
use ccwrapped::renderers::html::render_html;
use ccwrapped::renderers::markdown::render_markdown;
use ccwrapped::renderers::share_card::render_share_card;
use ccwrapped::renderers::terminal::render_terminal;
use ccwrapped::{home_dir, project_slug, Report};
use chrono::{Datelike, Utc};
use clap::Parser;
use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Parser)]
#[command(
    name = "ccwrapped",
    version,
    about = "Generate a Claude Code wrapped report from local JSONL history."
)]
struct Cli {
    #[arg(long, help = "writes claude-code-wrapped.md to current directory")]
    markdown: bool,
    #[arg(
        long,
        help = "writes claude-code-wrapped-card.html and opens it in browser"
    )]
    card: bool,
    #[arg(long, help = "writes per-project prompt files to ./wrapped-archive/")]
    archive: bool,
    #[arg(long = "no-open", help = "skip auto-opening browser")]
    no_open: bool,
    #[arg(long, help = "print JSON to stdout only, no files written")]
    json: bool,
    #[arg(value_name = "YEAR")]
    year: Option<i32>,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("\n  x Error: {error}");
        std::process::exit(1);
    }
}

struct BuiltReport {
    report: Report,
    entry_count: usize,
}

fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();
    let selected_year = cli.year.unwrap_or_else(|| Utc::now().year());

    let Some(home) = home_dir() else {
        return exit_with_message(
            &cli,
            selected_year,
            "home directory could not be resolved",
            "Claude Code home directory could not be resolved.".to_string(),
        );
    };

    let projects_dir = home.join(".claude").join("projects");
    if !projects_dir.exists() {
        return exit_with_message(
            &cli,
            selected_year,
            "~/.claude/projects not found",
            format!(
                "Claude Code data directory not found at {}.\nExpected JSONL files under ~/.claude/projects/. Nothing to analyze.",
                projects_dir.display()
            ),
        );
    }

    let Some(built_report) = build_report(selected_year, &projects_dir)? else {
        return exit_with_message(
            &cli,
            selected_year,
            "no records found",
            format!(
                "No Claude Code assistant usage records were found for {} in {}.",
                selected_year,
                projects_dir.display()
            ),
        );
    };

    if cli.json {
        println!("{}", serde_json::to_string_pretty(&built_report.report)?);
        return Ok(());
    }

    let cwd = std::env::current_dir()?;
    let outputs = write_outputs(&cwd, &built_report.report, &cli)?;
    print_summary(
        built_report.entry_count,
        selected_year,
        &built_report.report,
        &outputs,
    );
    Ok(())
}

fn exit_with_message(
    cli: &Cli,
    year: i32,
    json_error: &str,
    human_message: String,
) -> Result<(), Box<dyn Error>> {
    if cli.json {
        println!("{}", serde_json::json!({"error": json_error, "year": year}));
        std::process::exit(1);
    }
    println!("{human_message}");
    Ok(())
}

fn build_report(
    selected_year: i32,
    projects_dir: &Path,
) -> Result<Option<BuiltReport>, Box<dyn Error>> {
    let entries = read_all_jsonl(projects_dir, Some(selected_year));
    if entries.is_empty() {
        return Ok(None);
    }

    let session_breakdown = read_session_breakdown(projects_dir, Some(selected_year));
    let daily = aggregate_daily(&entries);
    let project_breakdown = aggregate_by_project(&entries);
    let cost_analysis = analyze_usage(selected_year, &daily, &session_breakdown);
    let cache_health = analyze_cache_health(&daily);
    let anomalies = detect_anomalies(&cost_analysis);
    let inflection = detect_inflection_points(&daily);
    let session_intel = analyze_session_intelligence(&session_breakdown, &entries);
    let model_routing = analyze_model_routing(&cost_analysis, &entries);
    let recommendations = generate_recommendations(
        &cost_analysis,
        &cache_health,
        &anomalies,
        &inflection,
        &session_intel,
        &model_routing,
        &project_breakdown,
    );

    let mut report = Report {
        generated_at: Utc::now().to_rfc3339(),
        year: selected_year,
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
    report.wrapped_story = build_wrapped_story(&report, &entries);

    Ok(Some(BuiltReport {
        report,
        entry_count: entries.len(),
    }))
}

fn write_outputs(cwd: &Path, report: &Report, cli: &Cli) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let html_path = cwd.join("claude-code-wrapped.html");
    fs::write(&html_path, render_html(report))?;

    let mut outputs = vec![html_path.clone()];
    if cli.markdown {
        let markdown_path = cwd.join("claude-code-wrapped.md");
        fs::write(&markdown_path, render_markdown(report))?;
        outputs.push(markdown_path);
    }

    if cli.card {
        let card_path = cwd.join("claude-code-wrapped-card.html");
        fs::write(&card_path, render_share_card(report))?;
        outputs.push(card_path.clone());
        if !cli.no_open {
            let _ = open_in_browser(&card_path);
        }
    }

    if cli.archive {
        let archive_dir = cwd.join("wrapped-archive");
        let written = write_archive(&archive_dir, report)?;
        println!(
            "  ✓ Prompt archive written to: {}/ ({} project{})",
            archive_dir.display(),
            written,
            if written == 1 { "" } else { "s" }
        );
    }

    if !cli.no_open && !cli.card {
        let _ = open_in_browser(&html_path);
    }

    Ok(outputs)
}

fn print_summary(entry_count: usize, selected_year: i32, report: &Report, outputs: &[PathBuf]) {
    println!(
        "  ✓ {} assistant usage entries parsed for {}",
        entry_count, selected_year
    );
    println!(
        "  ✓ {} active day buckets found",
        report.cost_analysis.daily_costs.len()
    );
    println!(
        "  ✓ {} sessions summarized from JSONL",
        report.session_breakdown.sessions.len()
    );
    render_terminal(report);
    for path in outputs {
        println!("  ✓ Wrote {}", path.display());
    }
}

fn write_archive(archive_dir: &Path, report: &Report) -> Result<usize, Box<dyn Error>> {
    fs::create_dir_all(archive_dir)?;
    // Key by project_hash (stable identity) to avoid merging unrelated repos that
    // share the same leaf directory name.
    let mut by_project: BTreeMap<String, (String, Vec<ccwrapped::SessionPrompt>)> = BTreeMap::new();
    let mut slug_counts: HashMap<String, usize> = HashMap::new();

    for session in &report.session_breakdown.sessions {
        let entry = by_project
            .entry(session.project_hash.clone())
            .or_insert_with(|| (session.project_name.clone(), Vec::new()));
        entry.1.extend(session.prompts.clone());
    }

    let mut written = 0usize;
    for (_hash, (project_name, prompts)) in by_project {
        if prompts.is_empty() {
            continue;
        }

        let base_slug = if project_slug(&project_name).is_empty() {
            "unknown".to_string()
        } else {
            project_slug(&project_name)
        };
        let count = slug_counts.entry(base_slug.clone()).or_insert(0);
        *count += 1;
        let filename = if *count == 1 {
            format!("{base_slug}.md")
        } else {
            format!("{base_slug}-{}.md", *count)
        };
        let top = prompts.into_iter().take(5).collect::<Vec<_>>();
        let mut lines = vec![
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
            if let Some(timestamp) = prompt.timestamp.as_deref() {
                lines.push(format!(
                    "**{}**",
                    timestamp.chars().take(10).collect::<String>()
                ));
                lines.push(String::new());
            }
            let cutoff = prompt
                .text
                .char_indices()
                .nth(500)
                .map(|(i, _)| i)
                .unwrap_or(prompt.text.len());
            let display = if prompt.text.len() > 500 {
                format!("{}... [truncated]", &prompt.text[..cutoff])
            } else {
                prompt.text.clone()
            };
            lines.push(display);
            lines.push(String::new());
        }
        fs::write(archive_dir.join(filename), lines.join("\n"))?;
        written += 1;
    }

    Ok(written)
}

fn open_in_browser(path: &Path) -> Result<(), Box<dyn Error>> {
    #[cfg(target_os = "windows")]
    {
        Command::new("cmd")
            .args(["/C", "start", "", &path.display().to_string()])
            .spawn()?;
    }
    #[cfg(target_os = "macos")]
    {
        Command::new("open").arg(path).spawn()?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open").arg(path).spawn()?;
    }
    Ok(())
}
