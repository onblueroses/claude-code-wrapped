use crate::{escape_html, Report};
use serde::Serialize;

pub fn render_share_card(report: &Report) -> String {
    let card = ShareCardReport::from_report(report);
    render_share_card_report(&card)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareCardReport {
    experience: String,
    title: String,
    stats: Vec<ShareCardStat>,
    limitation: Option<String>,
    trust: Vec<String>,
    method_facts: Vec<String>,
    insight_facts: Vec<String>,
    active_days: usize,
    timezone: String,
}

#[derive(Debug, Clone, Serialize)]
struct ShareCardStat {
    label: String,
    value: String,
}

impl ShareCardReport {
    fn from_report(report: &Report) -> Self {
        let wrapped = &report.wrapped_story;
        let local_cost = crate::canonical_local_cost(report);
        let cache_read = &report.canonical_metrics.cache.read_share;
        let stats = vec![
            ShareCardStat {
                label: "API-equivalent estimate".to_string(),
                value: local_cost.map_or_else(|| "Unavailable".to_string(), crate::format_currency),
            },
            ShareCardStat {
                label: "Active-time estimate".to_string(),
                value: format!(
                    "{} seconds",
                    report.canonical_metrics.active_time.total_active_seconds
                ),
            },
            ShareCardStat {
                label: "Observed messages".to_string(),
                value: if wrapped.total_messages > 0 {
                    crate::with_grouping(wrapped.total_messages as u64)
                } else {
                    "-".to_string()
                },
            },
            ShareCardStat {
                label: "Cache-read share".to_string(),
                value: crate::canonical_ratio_display(cache_read),
            },
            ShareCardStat {
                label: "Power hour".to_string(),
                value: wrapped
                    .power_hour
                    .as_ref()
                    .map(|bucket| crate::format_hour(bucket.hour))
                    .unwrap_or_else(|| "Unknown".to_string()),
            },
        ];
        let mut insight_facts = report
            .insights
            .families
            .iter()
            .map(super::insights::family_line)
            .collect::<Vec<_>>();
        insight_facts.extend(super::insights::cards(report, true).flat_map(|card| {
            let mut lines = vec![super::insights::context_line(card)];
            lines.extend(super::insights::narrative_lines(card));
            lines.extend(super::insights::comparison_line(card));
            lines.push(super::insights::limitations_line(card));
            lines.extend(card.supporting_facts.iter().map(super::insights::fact_line));
            lines
        }));

        Self {
            experience: crate::experience_label(report).to_string(),
            title: share_safe_title(&wrapped.archetype.title).to_string(),
            stats,
            limitation: crate::canonical_evidence_is_limited(report)
                .then(|| crate::PARTIAL_USAGE_LIMITATION.to_string()),
            trust: crate::trust_projection(report, "share").lines(),
            method_facts: crate::canonical_fact_lines(report),
            insight_facts,
            active_days: report
                .cost_analysis
                .daily_costs
                .iter()
                .filter(|day| day.message_count > 0)
                .count(),
            timezone: report.data_coverage.timezone.clone(),
        }
    }
}

fn share_safe_title(title: &str) -> &'static str {
    match title {
        "Entertainment · The Orchestrator" => "Entertainment · The Orchestrator",
        "Entertainment · The Toolsmith" => "Entertainment · The Toolsmith",
        "Entertainment · The Specialist" => "Entertainment · The Specialist",
        "Entertainment · The Explorer" => "Entertainment · The Explorer",
        "Entertainment · Not enough observed activity" => {
            "Entertainment · Not enough observed activity"
        }
        _ => "Entertainment · Not enough observed activity",
    }
}

fn render_share_card_report(card: &ShareCardReport) -> String {
    let stat_rows = card
        .stats
        .iter()
        .enumerate()
        .map(|(index, stat)| {
            format!(
                r#"<div class="stat-row" style="animation-delay:{:.2}s"><span class="stat-label">{}</span><span class="stat-value">{}</span></div>"#,
                0.30 + index as f64 * 0.14,
                escape_html(&stat.label),
                escape_html(&stat.value)
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let limitation = card.limitation.as_ref().map_or_else(String::new, |text| {
        format!(r#"<div class="limitation">{}</div>"#, escape_html(text))
    });
    let trust = card
        .trust
        .iter()
        .map(|line| format!("<div>{}</div>", escape_html(line)))
        .collect::<Vec<_>>()
        .join("");
    let method_facts = card
        .method_facts
        .iter()
        .map(|line| format!("<div>{}</div>", escape_html(line)))
        .collect::<Vec<_>>()
        .join("");
    let insight_facts = card
        .insight_facts
        .iter()
        .map(|line| format!("<div>{}</div>", escape_html(line)))
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
      min-height: 1920px;
      overflow-x: hidden;
      overflow-y: auto;
      display: flex;
      align-items: flex-start;
      justify-content: center;
      padding: 120px 0;
      color: #f8fafc;
      font-family: "Liberation Sans", Arial, sans-serif;
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
      color: rgba(248,250,252,0.78);
      letter-spacing: 0.14em;
      text-transform: uppercase;
      animation: fadeUp 0.6s ease both;
      animation-delay: 1.1s;
    }}
    .limitation {{
      margin: 0 24px 34px;
      color: rgba(248,250,252,0.84);
      font-size: 21px;
      line-height: 1.45;
      animation: fadeUp 0.55s ease both;
      animation-delay: 0.98s;
    }}
    .proof-ledger {{
      margin: 0 0 28px;
      color: rgba(248,250,252,0.82);
      font: 24px/1.35 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      overflow-wrap: anywhere;
      text-align: left;
    }}
    .proof-ledger > div {{
      break-inside: avoid;
      margin-bottom: 4px;
    }}
    .extended-proof {{
      margin: 0 0 24px;
      color: rgba(248,250,252,0.86);
      font: 24px/1.35 ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
      text-align: left;
    }}
    .extended-proof summary {{
      cursor: pointer;
      font-weight: 700;
      margin-bottom: 16px;
    }}
    .extended-proof .proof-ledger {{
      font-size: 22px;
      color: rgba(248,250,252,0.84);
    }}
    @keyframes fadeUp {{
      from {{ opacity: 0; transform: translateY(24px); }}
      to {{ opacity: 1; transform: translateY(0); }}
    }}
  </style>
</head>
<body>
  <div class="card">
    <div class="eyebrow">{}</div>
    <div class="title">{}</div>
    <div class="stats">{}</div>
    {}
    <div class="proof-ledger trust-ledger">{}</div>
    <details class="extended-proof"><summary>Metric proof details</summary><div class="proof-ledger">{}</div></details>
    <details class="extended-proof"><summary>Insight proof details</summary><div class="proof-ledger">{}</div></details>
    <div class="footer">ccwrapped · {} active day{} · {}</div>
  </div>
</body>
</html>"#,
        escape_html(&card.experience),
        escape_html(&card.title),
        stat_rows,
        limitation,
        trust,
        method_facts,
        insight_facts,
        card.active_days,
        if card.active_days == 1 { "" } else { "s" },
        escape_html(&card.timezone),
    )
}

#[cfg(test)]
mod tests {
    use super::{render_share_card_report, ShareCardReport};
    use crate::{ProjectSummary, Report, SessionSummary, TimeBucket};

    #[test]
    fn typed_share_projection_has_no_private_field_or_value_carrier() {
        const CANARY: &str = "SHARE_DTO_PRIVATE_CANARY_6A51";
        let mut report = Report {
            schema_version: "ccwrapped.report/v2".to_string(),
            ..Default::default()
        };
        report.data_coverage.selected_period = "2026".to_string();
        report.data_coverage.timezone = "UTC".to_string();
        report.data_coverage.completeness = "indeterminate".to_string();
        report.data_coverage.privacy_profile = "standard".to_string();
        report.wrapped_story.archetype.title = CANARY.to_string();
        report.wrapped_story.power_hour = Some(TimeBucket {
            hour: 9,
            label: CANARY.to_string(),
            ..Default::default()
        });
        report.project_breakdown.push(ProjectSummary {
            hash: CANARY.to_string(),
            path: Some(CANARY.to_string()),
            name: CANARY.to_string(),
            ..Default::default()
        });
        report.session_breakdown.sessions.push(SessionSummary {
            session_id: CANARY.to_string(),
            project_hash: CANARY.to_string(),
            project_path: Some(CANARY.to_string()),
            project_name: CANARY.to_string(),
            first_prompt: Some(CANARY.to_string()),
            ..Default::default()
        });

        let dto = ShareCardReport::from_report(&report);
        let serialized = serde_json::to_value(&dto).expect("share DTO must serialize");
        let object = serialized.as_object().expect("share DTO must be an object");
        for forbidden in [
            "path",
            "project",
            "session",
            "message",
            "request",
            "account",
            "prompt",
            "content",
            "diagnostic",
            "command",
        ] {
            assert!(
                object
                    .keys()
                    .all(|field| !field.to_ascii_lowercase().contains(forbidden)),
                "share DTO exposes forbidden field carrier {forbidden}"
            );
        }
        let serialized = serialized.to_string();
        assert!(!serialized.contains(CANARY));
        let html = render_share_card_report(&dto);
        assert!(!html.contains(CANARY));
        assert!(html.contains("Trust · profile=share"));
    }
}
