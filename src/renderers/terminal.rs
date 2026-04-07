use crate::Report;
use std::io::Write;
use termcolor::{Color, ColorChoice, ColorSpec, StandardStream, WriteColor};

pub fn render_terminal(report: &Report) {
    let wrapped = &report.wrapped_story;
    let mut stdout = StandardStream::stdout(ColorChoice::Auto);

    let _ = writeln!(&mut stdout);
    set_color(&mut stdout, Some(Color::White), true, false);
    let _ = writeln!(&mut stdout, "CLAUDE CODE WRAPPED");

    let grade_color = match report.cache_health.grade.letter.as_str() {
        "A" => Color::Green,
        "B" => Color::Cyan,
        "C" => Color::Yellow,
        _ => Color::Red,
    };
    set_color(&mut stdout, Some(grade_color), true, false);
    let _ = write!(&mut stdout, "{} ", report.cache_health.grade.letter);
    set_color(&mut stdout, Some(Color::White), true, false);
    let _ = writeln!(&mut stdout, "{}", wrapped.archetype.title);
    set_color(&mut stdout, Some(Color::White), false, true);
    let _ = writeln!(&mut stdout, "{}", wrapped.summary);

    let _ = writeln!(&mut stdout);
    set_color(&mut stdout, Some(Color::White), false, true);
    let _ = writeln!(&mut stdout, "Season stats");
    for hero in &wrapped.hero {
        set_color(&mut stdout, Some(Color::Blue), true, false);
        let _ = write!(&mut stdout, "  • ");
        set_color(&mut stdout, Some(Color::White), true, false);
        let _ = writeln!(&mut stdout, "{} {}", hero.label, hero.value);
        set_color(&mut stdout, Some(Color::White), false, true);
        let _ = writeln!(&mut stdout, "    {}", hero.note);
    }

    let _ = writeln!(&mut stdout);
    set_color(&mut stdout, Some(Color::White), false, true);
    let _ = writeln!(&mut stdout, "Standout moments");
    for highlight in wrapped.highlights.iter().take(5) {
        set_color(&mut stdout, Some(Color::Magenta), true, false);
        let _ = write!(&mut stdout, "  {} ", highlight.eyebrow.to_uppercase());
        set_color(&mut stdout, Some(Color::White), true, false);
        let _ = writeln!(&mut stdout, "{}", highlight.title);
        set_color(&mut stdout, Some(Color::White), false, true);
        let _ = writeln!(&mut stdout, "    {}", highlight.note);
    }

    let _ = writeln!(&mut stdout);
    set_color(&mut stdout, Some(Color::White), false, true);
    let _ = writeln!(&mut stdout, "Quick read");
    if let Some(weekday) = &wrapped.favorite_weekday {
        let _ = writeln!(&mut stdout, "  • Busiest weekday: {}", weekday.label);
    }
    if let Some(tool) = &wrapped.top_tool {
        let _ = writeln!(
            &mut stdout,
            "  • Most-called tool: {} ({})",
            tool.name, tool.count
        );
    }
    if wrapped.longest_streak > 1 {
        let _ = writeln!(
            &mut stdout,
            "  • Longest streak: {} day{}",
            wrapped.longest_streak,
            if wrapped.longest_streak == 1 { "" } else { "s" }
        );
    }
    if report.session_breakdown.total_subagent_sessions > 0 {
        let _ = writeln!(
            &mut stdout,
            "  • Subagent sessions: {}",
            report.session_breakdown.total_subagent_sessions
        );
    }
    if wrapped.prompt_ratio.total > 0 {
        let _ = writeln!(
            &mut stdout,
            "  • Human vs tool: {}% human ({} human / {} tool)",
            wrapped.prompt_ratio.human_pct, wrapped.prompt_ratio.human, wrapped.prompt_ratio.tool
        );
    }

    if let Some(inflection) = &report.inflection {
        let _ = writeln!(&mut stdout);
        set_color(&mut stdout, Some(Color::White), false, true);
        let _ = writeln!(&mut stdout, "Trend");
        let _ = writeln!(&mut stdout, "  {}", inflection.summary);
    }

    let _ = stdout.reset();
    let _ = writeln!(&mut stdout);
}

fn set_color(stdout: &mut StandardStream, fg: Option<Color>, bold: bool, dimmed: bool) {
    let mut spec = ColorSpec::new();
    spec.set_fg(fg);
    spec.set_bold(bold);
    spec.set_dimmed(dimmed);
    let _ = stdout.set_color(&spec);
}
