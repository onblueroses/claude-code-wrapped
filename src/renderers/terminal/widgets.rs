const SPARK_CHARS: &[char] = &['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
const BAR_FULL: char = '█';
const BAR_EMPTY: char = '░';

/// Renders a sparkline from values using Unicode block characters.
/// Each value maps to one character. Width is ignored if values.len() < width;
/// if values.len() > width, values are bucketed.
pub fn sparkline(values: &[f64], width: usize) -> String {
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
    let content_len = label.len() + value.len();
    if content_len >= width {
        return format!("{label}  {value}");
    }
    let gap = width - content_len;
    format!("{label}{}{value}", " ".repeat(gap))
}

/// Renders a section header with a rule line.
pub fn section_header(title: &str, width: usize) -> String {
    let rule_len = width.saturating_sub(title.len() + 3);
    format!("-- {} {}", title, "-".repeat(rule_len))
}

/// Pads or truncates a string to fit exactly `width` characters.
pub fn pad(text: &str, width: usize) -> String {
    let char_count = text.chars().count();
    if char_count >= width {
        text.chars().take(width).collect()
    } else {
        format!("{}{}", text, " ".repeat(width - char_count))
    }
}

/// Detects terminal width from COLUMNS env var, falling back to 80.
pub fn terminal_width() -> usize {
    std::env::var("COLUMNS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(80)
        .max(40)
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
    fn terminal_width_fallback() {
        // When COLUMNS is not set or invalid, should return at least 40
        let width = terminal_width();
        assert!(width >= 40);
    }
}
