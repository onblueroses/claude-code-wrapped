use crate::{escape_html, Report};

pub fn render_share_card(report: &Report) -> String {
    let wrapped = &report.wrapped_story;
    let stats = [
        (
            "Season spend".to_string(),
            crate::format_currency(report.cost_analysis.total_cost),
        ),
        (
            "Messages sent".to_string(),
            if wrapped.total_messages > 0 {
                crate::with_grouping(wrapped.total_messages as u64)
            } else {
                "-".to_string()
            },
        ),
        (
            "Human prompts".to_string(),
            format!("{}%", wrapped.prompt_ratio.human_pct),
        ),
        (
            "Cache grade".to_string(),
            format!("Grade {}", report.cache_health.grade.letter),
        ),
        (
            "Power hour".to_string(),
            wrapped
                .power_hour
                .as_ref()
                .map(|bucket| bucket.label.clone())
                .unwrap_or_else(|| "Unknown".to_string()),
        ),
    ];

    let stat_rows = stats
        .iter()
        .enumerate()
        .map(|(index, (label, value))| {
            format!(
                r#"<div class="stat-row" style="animation-delay:{:.2}s"><span class="stat-label">{}</span><span class="stat-value">{}</span></div>"#,
                0.30 + index as f64 * 0.14,
                escape_html(label),
                escape_html(value)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=1080, initial-scale=1.0">
  <title>Claude Code Wrapped Card</title>
  <style>
    * {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      width: 1080px;
      height: 1920px;
      overflow: hidden;
      display: flex;
      align-items: center;
      justify-content: center;
      color: #f8fafc;
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background:
        radial-gradient(circle at 18% 16%, rgba(88, 203, 255, 0.22), transparent 28%),
        radial-gradient(circle at 82% 84%, rgba(48, 214, 171, 0.16), transparent 26%),
        linear-gradient(160deg, #031018 0%, #0b1930 42%, #12142b 100%);
      position: relative;
    }}
    body::before {{
      content: "";
      position: absolute;
      inset: 0;
      background:
        linear-gradient(135deg, rgba(255,255,255,0.04), transparent 56%),
        repeating-linear-gradient(180deg, rgba(255,255,255,0.03), rgba(255,255,255,0.03) 2px, transparent 2px, transparent 8px);
      mix-blend-mode: screen;
      opacity: 0.14;
      pointer-events: none;
    }}
    .card {{
      width: 820px;
      position: relative;
      z-index: 1;
      text-align: center;
    }}
    .eyebrow {{
      font-size: 22px;
      letter-spacing: 0.22em;
      text-transform: uppercase;
      color: rgba(248,250,252,0.45);
      margin-bottom: 28px;
      animation: fadeUp 0.65s ease both;
    }}
    .title {{
      font-size: 76px;
      line-height: 1.04;
      letter-spacing: -0.05em;
      font-weight: 800;
      background: linear-gradient(135deg, #8be7ff 0%, #7cf2c8 52%, #f8fafc 100%);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
      background-clip: text;
      margin-bottom: 56px;
      animation: fadeUp 0.7s ease both;
      animation-delay: 0.12s;
    }}
    .stats {{
      display: grid;
      gap: 18px;
      margin-bottom: 54px;
    }}
    .stat-row {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      gap: 18px;
      padding: 28px 34px;
      border-radius: 22px;
      background: rgba(255,255,255,0.055);
      border: 1px solid rgba(255,255,255,0.09);
      backdrop-filter: blur(10px);
      animation: fadeUp 0.55s ease both;
    }}
    .stat-label {{
      font-size: 28px;
      color: rgba(248,250,252,0.62);
      letter-spacing: 0.02em;
    }}
    .stat-value {{
      font-size: 36px;
      font-weight: 700;
      letter-spacing: -0.02em;
      color: #ffffff;
    }}
    .footer {{
      font-size: 22px;
      color: rgba(248,250,252,0.28);
      letter-spacing: 0.14em;
      text-transform: uppercase;
      animation: fadeUp 0.6s ease both;
      animation-delay: 1.1s;
    }}
    @keyframes fadeUp {{
      from {{ opacity: 0; transform: translateY(24px); }}
      to {{ opacity: 1; transform: translateY(0); }}
    }}
  </style>
</head>
<body>
  <div class="card">
    <div class="eyebrow">Claude Code Wrapped</div>
    <div class="title">{}</div>
    <div class="stats">{}</div>
    <div class="footer">ccwrapped · {} active day{}</div>
  </div>
</body>
</html>"#,
        escape_html(&wrapped.archetype.title),
        stat_rows,
        report.cost_analysis.active_days,
        if report.cost_analysis.active_days == 1 {
            ""
        } else {
            "s"
        }
    )
}
