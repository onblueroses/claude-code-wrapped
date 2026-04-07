use crate::{escape_html, format_currency_compact, format_ratio, format_tokens, trim_text, Report};

pub fn render_html(report: &Report) -> String {
    let wrapped = &report.wrapped_story;
    let model_rows = report
        .cost_analysis
        .model_costs
        .iter()
        .map(|(model, cost)| {
            let share = if report.cost_analysis.total_cost > 0.0 {
                (*cost / report.cost_analysis.total_cost) * 100.0
            } else {
                0.0
            };
            format!(
                r#"<div class="model-row"><div><strong>{}</strong><div class="muted mono">{}</div></div><div class="bar"><span style="width:{:.1}%"></span></div><div class="muted mono">{:.1}%</div></div>"#,
                escape_html(model),
                escape_html(&format_currency_compact(*cost)),
                share,
                share
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let project_rows = report
        .project_breakdown
        .iter()
        .take(8)
        .map(|project| {
            format!(
                r#"<div class="list-row"><div><strong>{}</strong></div><div class="muted mono">{} output</div><div class="muted mono">{} session{}</div></div>"#,
                escape_html(&project.name),
                escape_html(&format_tokens(project.output_tokens)),
                project.session_count,
                if project.session_count == 1 { "" } else { "s" }
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let costliest_sessions = if report.session_breakdown.costly_sessions.is_empty() {
        r#"<div class="muted">No heavy session data available yet.</div>"#.to_string()
    } else {
        report
            .session_breakdown
            .costly_sessions
            .iter()
            .take(6)
            .map(|session| {
                format!(
                    r#"<article class="card list-card"><div class="row-top"><div><div class="eyebrow">{}</div><strong>{}</strong></div><span class="pill mono">{}</span></div></article>"#,
                    escape_html(&session.project_name),
                    escape_html(
                        &session
                            .timestamp_start
                            .as_deref()
                            .map(|value| value.chars().take(10).collect::<String>())
                            .unwrap_or_else(|| "Unknown".to_string())
                    ),
                    escape_html(&format_tokens(session.total_tokens)),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let subagent_spikes = if report.session_breakdown.costly_subagents.is_empty() {
        r#"<div class="muted">No distinct subagent spikes showed up in this slice.</div>"#
            .to_string()
    } else {
        report
            .session_breakdown
            .costly_subagents
            .iter()
            .take(6)
            .map(|subagent| {
                format!(
                    r#"<article class="card list-card"><div class="row-top"><div><div class="eyebrow">{}</div><strong>{}</strong></div><span class="pill mono">{}</span></div><div class="muted">{}</div></article>"#,
                    escape_html(
                        subagent
                            .project_name
                            .as_deref()
                            .unwrap_or("Subagent")
                    ),
                    escape_html(
                        &subagent
                            .timestamp_start
                            .as_deref()
                            .map(|value| value.chars().take(10).collect::<String>())
                            .unwrap_or_else(|| "Unknown".to_string())
                    ),
                    escape_html(&format_tokens(subagent.total_tokens)),
                    escape_html(&trim_text(
                        subagent
                            .first_prompt
                            .as_deref()
                            .unwrap_or("No prompt preview available."),
                        140
                    ))
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    let recommendations = report
        .recommendations
        .iter()
        .take(6)
        .map(|rec| {
            format!(
                r#"<article class="card list-card"><h3>{}</h3><p class="muted">{}</p></article>"#,
                escape_html(&rec.title),
                escape_html(&rec.action)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let highlights = wrapped
        .highlights
        .iter()
        .map(|highlight| {
            format!(
                r#"<article class="card highlight-card"><div class="eyebrow">{}</div><h3>{}</h3><p class="muted">{}</p></article>"#,
                escape_html(&highlight.eyebrow),
                escape_html(&highlight.title),
                escape_html(&highlight.note)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let hero = wrapped
        .hero
        .iter()
        .map(|hero| {
            format!(
                r#"<article class="card stat-card"><div class="eyebrow">{}</div><div class="stat">{}</div><div class="muted">{}</div></article>"#,
                escape_html(&hero.label),
                escape_html(&hero.value),
                escape_html(&hero.note)
            )
        })
        .collect::<Vec<_>>()
        .join("");

    let daily_rows = report
        .cost_analysis
        .daily_costs
        [report.cost_analysis.daily_costs.len().saturating_sub(12)..]
        .iter()
        .map(|day| {
            format!(
                r#"<tr><td>{}</td><td class="mono">{}</td><td class="mono">{}</td><td class="mono">{}</td></tr>"#,
                escape_html(&day.date),
                escape_html(&format_currency_compact(day.cost)),
                escape_html(&format_tokens(day.output_tokens)),
                escape_html(&format_ratio(day.cache_output_ratio))
            )
        })
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Claude Code Wrapped</title>
  <style>
    :root {{
      color-scheme: dark;
      --bg: #081018;
      --panel: rgba(16, 24, 38, 0.9);
      --panel-strong: rgba(22, 31, 48, 0.98);
      --line: rgba(255, 255, 255, 0.08);
      --text: #f7fafc;
      --muted: #9fb0c6;
      --sky: #6dd3ff;
      --mint: #74e2b3;
      --gold: #f3c969;
      --rose: #ff8d8d;
      --shadow: 0 24px 80px rgba(0,0,0,0.35);
    }}
    * {{ box-sizing: border-box; }}
    body {{
      margin: 0;
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", sans-serif;
      background:
        radial-gradient(circle at top left, rgba(109, 211, 255, 0.16), transparent 26%),
        radial-gradient(circle at top right, rgba(116, 226, 179, 0.12), transparent 24%),
        linear-gradient(180deg, #081018 0%, #05090f 100%);
      color: var(--text);
      padding: 32px 18px 80px;
    }}
    .shell {{ max-width: 1220px; margin: 0 auto; display: grid; gap: 20px; }}
    .hero, .panel, .card {{
      background: var(--panel);
      border: 1px solid var(--line);
      border-radius: 24px;
      box-shadow: var(--shadow);
    }}
    .hero, .panel {{ padding: 26px; }}
    .hero {{
      display: grid;
      gap: 22px;
      background:
        linear-gradient(135deg, rgba(109, 211, 255, 0.08), rgba(255,255,255,0.02)),
        var(--panel);
    }}
    .eyebrow {{ font-size: 11px; letter-spacing: 0.18em; text-transform: uppercase; color: var(--muted); }}
    h1, h2, h3, p {{ margin: 0; }}
    h1 {{ font-size: clamp(34px, 5vw, 58px); line-height: 0.96; letter-spacing: -0.04em; }}
    h2 {{ font-size: 28px; letter-spacing: -0.03em; margin-bottom: 8px; }}
    h3 {{ font-size: 22px; letter-spacing: -0.03em; margin-bottom: 10px; }}
    .muted {{ color: var(--muted); line-height: 1.7; }}
    .mono {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
    .hero-grid, .highlights, .two-up {{ display: grid; gap: 16px; }}
    .hero-grid {{ grid-template-columns: repeat(5, minmax(0, 1fr)); }}
    .highlights {{ grid-template-columns: repeat(3, minmax(0, 1fr)); }}
    .two-up {{ grid-template-columns: repeat(2, minmax(0, 1fr)); }}
    .card {{ padding: 18px; background: var(--panel-strong); }}
    .stat {{ font-size: 28px; font-weight: 700; letter-spacing: -0.04em; margin: 6px 0; }}
    .highlight-card {{ display: grid; gap: 10px; }}
    .list-card {{ display: grid; gap: 12px; }}
    .row-top {{ display: flex; align-items: start; justify-content: space-between; gap: 12px; }}
    .pill {{
      display: inline-flex;
      align-items: center;
      justify-content: center;
      padding: 8px 12px;
      border-radius: 999px;
      background: rgba(109, 211, 255, 0.12);
      color: var(--sky);
      font-size: 12px;
      font-weight: 700;
      letter-spacing: 0.08em;
      text-transform: uppercase;
    }}
    .model-row {{
      display: grid;
      grid-template-columns: 1.3fr 3fr auto;
      gap: 14px;
      align-items: center;
      margin-bottom: 14px;
    }}
    .bar {{
      height: 10px;
      border-radius: 999px;
      background: rgba(255,255,255,0.07);
      overflow: hidden;
    }}
    .bar span {{
      display: block;
      height: 100%;
      border-radius: inherit;
      background: linear-gradient(90deg, var(--sky), var(--mint));
    }}
    .list-row {{
      display: grid;
      grid-template-columns: minmax(0, 1.8fr) auto auto;
      gap: 14px;
      padding: 14px 0;
      border-bottom: 1px solid rgba(255,255,255,0.06);
      align-items: center;
    }}
    .list-row:last-child {{ border-bottom: none; }}
    .project-path {{ font-size: 12px; margin-top: 4px; word-break: break-all; }}
    table {{ width: 100%; border-collapse: collapse; }}
    td, th {{ padding: 10px 0; border-bottom: 1px solid rgba(255,255,255,0.06); text-align: left; }}
    tr:last-child td {{ border-bottom: none; }}
    @media (max-width: 1100px) {{
      .hero-grid, .highlights, .two-up {{ grid-template-columns: 1fr; }}
      .list-row {{ grid-template-columns: 1fr; }}
    }}
  </style>
</head>
<body>
  <main class="shell">
    <section class="hero">
      <div class="eyebrow">Local data only · {}</div>
      <h1>Claude Code Wrapped</h1>
      <p class="muted">{}</p>
      <div class="hero-grid">{}</div>
    </section>

    <section class="highlights">{}</section>

    <section class="panel">
      <div class="eyebrow">Season arc</div>
      <h2>Daily cost snapshot</h2>
      <p class="muted">A quick view of equivalent spend, output tokens, and cache ratio for recent active days.</p>
      <table>
        <thead>
          <tr><th>Date</th><th>Spend</th><th>Output</th><th>Cache ratio</th></tr>
        </thead>
        <tbody>{}</tbody>
      </table>
    </section>

    <section class="two-up">
      <section class="panel">
        <div class="eyebrow">Model mix</div>
        <h2>Routing distribution</h2>
        <p class="muted">How this season split across the available model tiers.</p>
        {}
      </section>

      <section class="panel">
        <div class="eyebrow">Projects</div>
        <h2>Top projects</h2>
        <p class="muted">The projects that absorbed most of your output tokens.</p>
        {}
      </section>
    </section>

    <section class="two-up">
      <section class="panel">
        <div class="eyebrow">Costliest sessions</div>
        <h2>Costliest sessions</h2>
        <p class="muted">The heaviest top-level runs from your local history.</p>
        {}
      </section>

      <section class="panel">
        <div class="eyebrow">Subagent spikes</div>
        <h2>Subagent spikes</h2>
        <p class="muted">The biggest background-agent bursts in this dataset.</p>
        {}
      </section>
    </section>

    <section class="two-up">
      <section class="panel">
        <div class="eyebrow">Next season upgrades</div>
        <h2>Next season upgrades</h2>
        <p class="muted">The shortest path to a cleaner next run.</p>
        {}
      </section>

      <section class="panel">
        <div class="eyebrow">Context tax</div>
        <h2>Context tax</h2>
        <p class="muted">What the current workflow shape costs before the next prompt starts.</p>
        <div class="card list-card">
          <div class="row-top"><span class="muted">Cache ratio</span><strong class="mono">{}</strong></div>
          <div class="muted">{}</div>
        </div>
        <div class="card list-card">
          <div class="row-top"><span class="muted">Equivalent spend</span><strong class="mono">{}</strong></div>
          <div class="muted">{} assistant turns · {} total tokens touched</div>
        </div>
      </section>
    </section>
  </main>
</body>
</html>"#,
        escape_html(&format!("{} active days", report.cost_analysis.active_days)),
        escape_html(&wrapped.summary),
        hero,
        highlights,
        daily_rows,
        model_rows,
        project_rows,
        costliest_sessions,
        subagent_spikes,
        recommendations,
        escape_html(&format_ratio(report.cache_health.efficiency_ratio)),
        escape_html(&format!(
            "{} · {} break estimates",
            report.cache_health.grade.label, report.cache_health.estimated_breaks
        )),
        escape_html(&format_currency_compact(report.cost_analysis.total_cost)),
        report.wrapped_story.total_messages,
        escape_html(&format_tokens(report.wrapped_story.total_tokens))
    )
}
