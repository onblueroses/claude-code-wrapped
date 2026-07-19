use std::borrow::Cow;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

const SPARK_CHARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const BAR_FULL: char = '█';
const BAR_EMPTY: char = '░';
const MAX_TERMINAL_WIDTH: usize = 512;

fn bounded_width(width: usize) -> usize {
    width.min(MAX_TERMINAL_WIDTH)
}

/// Renders a sparkline from values using Unicode block characters.
/// Each value maps to one character. Width is ignored if values.len() < width;
/// if values.len() > width, values are bucketed.
pub fn sparkline(values: &[f64], width: usize) -> String {
    let width = bounded_width(width);
    if values.is_empty() || width == 0 {
        return String::new();
    }

    let buckets = if values.len() <= width {
        values.to_vec()
    } else {
        let bucket_size = values.len() as f64 / width as f64;
        (0..width)
            .map(|i| {
                let start = (i as f64 * bucket_size) as usize;
                let end = (((i + 1) as f64) * bucket_size) as usize;
                let slice = &values[start..end.min(values.len())];
                if slice.is_empty() {
                    0.0
                } else {
                    slice.iter().sum::<f64>() / slice.len() as f64
                }
            })
            .collect()
    };

    let max = buckets.iter().cloned().fold(0.0f64, f64::max);
    if max <= 0.0 {
        return SPARK_CHARS[0].to_string().repeat(buckets.len());
    }

    buckets
        .iter()
        .map(|v| {
            let normalized = (v / max * 7.0).round() as usize;
            SPARK_CHARS[normalized.min(7)]
        })
        .collect()
}

/// Renders a filled/empty percentage bar: ████░░░░░░
pub fn percentage_bar(pct: f64, width: usize) -> String {
    let width = bounded_width(width);
    if width == 0 {
        return String::new();
    }
    let clamped = pct.clamp(0.0, 100.0);
    let filled = ((clamped / 100.0) * width as f64).round() as usize;
    let empty = width.saturating_sub(filled);
    format!(
        "{}{}",
        BAR_FULL.to_string().repeat(filled),
        BAR_EMPTY.to_string().repeat(empty),
    )
}

/// Renders a two-tone ratio bar and returns (left_part, right_part) for coloring.
/// Uses distinct glyphs so the split is visible even without color.
pub fn ratio_bar(left_pct: f64, width: usize) -> (String, String) {
    let width = bounded_width(width);
    if width == 0 {
        return (String::new(), String::new());
    }
    let clamped = left_pct.clamp(0.0, 100.0);
    let left_width = ((clamped / 100.0) * width as f64).round() as usize;
    let right_width = width.saturating_sub(left_width);
    (
        BAR_FULL.to_string().repeat(left_width),
        BAR_EMPTY.to_string().repeat(right_width),
    )
}

/// Renders a line with label left-aligned and value right-aligned, padded to width.
pub fn label_value(label: &str, value: &str, width: usize) -> String {
    let width = bounded_width(width);
    let content_len = UnicodeWidthStr::width(label) + UnicodeWidthStr::width(value);
    if content_len >= width {
        return format!("{label}  {value}");
    }
    let gap = width - content_len;
    format!("{label}{}{value}", " ".repeat(gap))
}

/// Renders a section header with a rule line.
pub fn section_header(title: &str, width: usize) -> String {
    // This widget's established contract renders one column beyond `width`.
    // Reserve that column before applying the global allocation ceiling.
    let width = width.min(MAX_TERMINAL_WIDTH.saturating_sub(1));
    let rule_len = width.saturating_sub(UnicodeWidthStr::width(title) + 3);
    format!("-- {} {}", title, "-".repeat(rule_len))
}

/// Makes a report-derived value inert before it reaches a terminal sink.
pub fn terminal_text(value: &str) -> Cow<'_, str> {
    if value.chars().all(is_terminal_text_character) {
        return Cow::Borrowed(value);
    }

    Cow::Owned(
        value
            .chars()
            .map(|character| {
                if is_terminal_text_character(character) {
                    character
                } else {
                    '\u{fffd}'
                }
            })
            .collect(),
    )
}

fn is_terminal_text_character(character: char) -> bool {
    !character.is_control()
        && !matches!(
            character,
            '\u{061c}'
                | '\u{200e}'
                | '\u{200f}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2066}'..='\u{2069}'
        )
}

/// Pads or truncates a string to fit exactly `width` terminal columns.
pub fn pad(text: &str, width: usize) -> String {
    let width = bounded_width(width);
    let mut rendered = String::with_capacity(text.len().min(width));
    let mut columns = 0usize;
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if columns.saturating_add(character_width) > width {
            break;
        }
        rendered.push(character);
        columns = columns.saturating_add(character_width);
    }
    rendered.push_str(&" ".repeat(width.saturating_sub(columns)));
    rendered
}

const DEFAULT_TERMINAL_WIDTH: usize = 80;
const MIN_TERMINAL_WIDTH: usize = 40;

/// Detects terminal width from COLUMNS while bounding renderer allocations.
pub fn terminal_width() -> usize {
    let columns = std::env::var("COLUMNS").ok();
    terminal_width_from(columns.as_deref())
}

fn terminal_width_from(columns: Option<&str>) -> usize {
    columns
        .and_then(|value| value.parse::<usize>().ok())
        .unwrap_or(DEFAULT_TERMINAL_WIDTH)
        .clamp(MIN_TERMINAL_WIDTH, MAX_TERMINAL_WIDTH)
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- sparkline --

    #[test]
    fn sparkline_empty_input() {
        assert_eq!(sparkline(&[], 10), "");
    }

    #[test]
    fn sparkline_single_value() {
        let result = sparkline(&[5.0], 10);
        assert_eq!(result.chars().count(), 1);
        assert_eq!(result, "█");
    }

    #[test]
    fn terminal_text_replaces_controls_and_directional_formatting() {
        assert_eq!(
            terminal_text("safe\u{1b}]52\u{7}\r\n\u{202e}\u{2028}end"),
            "safe�]52�����end"
        );
    }

    #[test]
    fn sparkline_all_zeros() {
        let result = sparkline(&[0.0, 0.0, 0.0], 10);
        assert_eq!(result, "▁▁▁");
    }

    #[test]
    fn sparkline_ascending() {
        let result = sparkline(&[0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0], 8);
        assert_eq!(result.chars().count(), 8);
        // First char should be lowest, last should be highest
        let chars: Vec<char> = result.chars().collect();
        assert_eq!(chars[0], '▁');
        assert_eq!(chars[7], '█');
    }

    #[test]
    fn sparkline_buckets_when_values_exceed_width() {
        let values: Vec<f64> = (0..100).map(|i| i as f64).collect();
        let result = sparkline(&values, 10);
        assert_eq!(result.chars().count(), 10);
    }

    #[test]
    fn sparkline_zero_width() {
        assert_eq!(sparkline(&[1.0, 2.0], 0), "");
    }

    // -- percentage_bar --

    #[test]
    fn percentage_bar_zero() {
        let result = percentage_bar(0.0, 10);
        assert_eq!(result, "░░░░░░░░░░");
    }

    #[test]
    fn percentage_bar_full() {
        let result = percentage_bar(100.0, 10);
        assert_eq!(result, "██████████");
    }

    #[test]
    fn percentage_bar_half() {
        let result = percentage_bar(50.0, 10);
        assert_eq!(result, "█████░░░░░");
    }

    #[test]
    fn percentage_bar_clamps_above_100() {
        let result = percentage_bar(150.0, 10);
        assert_eq!(result, "██████████");
    }

    #[test]
    fn percentage_bar_clamps_negative() {
        let result = percentage_bar(-10.0, 10);
        assert_eq!(result, "░░░░░░░░░░");
    }

    #[test]
    fn percentage_bar_zero_width() {
        assert_eq!(percentage_bar(50.0, 0), "");
    }

    // -- ratio_bar --

    #[test]
    fn ratio_bar_splits_correctly() {
        let (left, right) = ratio_bar(70.0, 10);
        assert_eq!(left.chars().count(), 7);
        assert_eq!(right.chars().count(), 3);
    }

    #[test]
    fn ratio_bar_zero_width() {
        let (left, right) = ratio_bar(50.0, 0);
        assert!(left.is_empty());
        assert!(right.is_empty());
    }

    // -- label_value --

    #[test]
    fn label_value_pads_to_width() {
        let result = label_value("Cost", "$50", 20);
        assert_eq!(result.len(), 20);
        assert!(result.starts_with("Cost"));
        assert!(result.ends_with("$50"));
    }

    #[test]
    fn label_value_handles_overflow() {
        let result = label_value("Very long label", "Very long value", 10);
        assert!(result.contains("Very long label"));
        assert!(result.contains("Very long value"));
    }

    // -- section_header --

    #[test]
    fn section_header_has_title() {
        let result = section_header("Activity", 40);
        assert!(result.contains("Activity"));
        assert!(result.starts_with("--"));
    }

    // -- pad --

    #[test]
    fn pad_extends_short_string() {
        assert_eq!(pad("hi", 5), "hi   ");
    }

    #[test]
    fn pad_truncates_long_string() {
        assert_eq!(pad("hello world", 5), "hello");
    }

    // -- terminal_width --

    #[test]
    fn terminal_width_caps_hostile_environment_values() {
        assert_eq!(terminal_width_from(Some("1000000000")), 512);
        assert_eq!(terminal_width_from(Some("39")), 40);
        assert_eq!(terminal_width_from(Some("invalid")), 80);
    }

    #[test]
    fn public_widgets_bound_hostile_widths_without_changing_ordinary_widths() {
        let boundaries = [
            (0, 0),
            (39, 39),
            (40, 40),
            (512, 512),
            (513, 512),
            (usize::MAX, 512),
        ];
        let values = vec![1.0; 1_024];

        for (requested, expected) in boundaries {
            assert_eq!(sparkline(&values, requested).chars().count(), expected);
            assert_eq!(percentage_bar(50.0, requested).chars().count(), expected);
            let (left, right) = ratio_bar(50.0, requested);
            assert_eq!(left.chars().count() + right.chars().count(), expected);
            assert_eq!(pad("", requested).chars().count(), expected);

            if expected >= 4 {
                assert_eq!(
                    section_header("", requested).chars().count(),
                    requested.min(MAX_TERMINAL_WIDTH - 1) + 1
                );
                assert_eq!(label_value("a", "b", requested).chars().count(), expected);
            }
        }
    }

    #[test]
    fn terminal_width_fallback() {
        // When COLUMNS is not set or invalid, should return at least 40
        let width = terminal_width();
        assert!(width >= 40);
    }
}
