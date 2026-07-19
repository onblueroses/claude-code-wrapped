mod sections;
pub mod widgets;

use crate::Report;
use std::io;
use termcolor::{ColorChoice, StandardStream, WriteColor};

pub fn color_choice(plain: bool) -> ColorChoice {
    if plain || std::env::var_os("NO_COLOR").is_some() {
        ColorChoice::Never
    } else {
        ColorChoice::Auto
    }
}

/// Renders to stdout and panics if the terminal rejects the output.
///
/// Prefer [`try_render_terminal`] when the caller can handle an I/O error.
pub fn render_terminal(report: &Report) {
    try_render_terminal(report).expect("failed to render terminal report");
}

/// Renders to stdout with an explicit color policy and panics on an I/O error.
///
/// Prefer [`try_render_terminal_with`] when the caller can handle an I/O error.
pub fn render_terminal_with(report: &Report, choice: ColorChoice) {
    try_render_terminal_with(report, choice).expect("failed to render terminal report");
}

/// Renders to a caller-provided terminal writer and panics on an I/O error.
///
/// This compatibility helper preserves its original return type. Prefer
/// [`try_render_terminal_to`] when the caller can handle an I/O error.
pub fn render_terminal_to(report: &Report, writer: &mut impl WriteColor) {
    try_render_terminal_to(report, writer).expect("failed to render terminal report");
}

/// Renders to stdout and reports broken pipes and other terminal I/O failures.
pub fn try_render_terminal(report: &Report) -> io::Result<()> {
    try_render_terminal_with(report, ColorChoice::Auto)
}

/// Renders to stdout with an explicit color policy and reports I/O failures.
pub fn try_render_terminal_with(report: &Report, choice: ColorChoice) -> io::Result<()> {
    let mut stdout = StandardStream::stdout(choice);
    try_render_terminal_to(report, &mut stdout)
}

/// Renders to a caller-provided terminal writer and reports every I/O failure.
pub fn try_render_terminal_to(report: &Report, writer: &mut impl WriteColor) -> io::Result<()> {
    let width = widgets::terminal_width();
    writeln!(writer)?;
    sections::header(report, writer, width)?;
    sections::activity(report, writer, width)?;
    sections::cache(report, writer, width)?;
    sections::model_mix_and_projects(report, writer, width)?;
    sections::sessions_and_subagents(report, writer, width)?;
    sections::ratio_and_savings(report, writer, width)?;
    sections::highlights(report, writer, width)?;
    sections::insights(report, writer, width)?;
    sections::recommendations(report, writer, width)?;
    sections::trend(report, writer)?;
    sections::method_facts(report, writer, width)?;
    writer.reset()?;
    writeln!(writer)
}
