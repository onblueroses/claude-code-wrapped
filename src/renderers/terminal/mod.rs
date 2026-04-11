mod sections;
pub mod widgets;

use crate::Report;
use termcolor::{ColorChoice, StandardStream, WriteColor};

pub fn color_choice(plain: bool) -> ColorChoice {
    if plain || std::env::var_os("NO_COLOR").is_some() {
        ColorChoice::Never
    } else {
        ColorChoice::Auto
    }
}

pub fn render_terminal(report: &Report) {
    render_terminal_with(report, ColorChoice::Auto);
}

pub fn render_terminal_with(report: &Report, choice: ColorChoice) {
    let mut stdout = StandardStream::stdout(choice);
    render_terminal_to(report, &mut stdout);
}

pub fn render_terminal_to(report: &Report, writer: &mut impl WriteColor) {
    let width = widgets::terminal_width();
    let _ = writeln!(writer);
    sections::header(report, writer, width);
    sections::activity(report, writer, width);
    sections::cache(report, writer, width);
    sections::model_mix_and_projects(report, writer, width);
    sections::sessions_and_subagents(report, writer, width);
    sections::ratio_and_savings(report, writer, width);
    sections::highlights(report, writer, width);
    sections::recommendations(report, writer, width);
    sections::trend(report, writer);
    let _ = writer.reset();
    let _ = writeln!(writer);
}
