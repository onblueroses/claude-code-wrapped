use crate::{escape_html, format_currency_compact, format_ratio, format_tokens, trim_text, Report};

pub fn render_html(report: &Report) -> String {
    let wrapped = &report.wrapped_story;
    let grade = &report.cache_health.grade;

    // --- Stat pills from hero stats ---
    let stat_pills = wrapped
        .hero
        .iter()
        .map(|h| {
            format!(
                r#"<div class="stat-pill"><div class="stat-pill-value">{}</div><div class="stat-pill-label">{}</div></div>"#,
                escape_html(&h.value),
                escape_html(&h.label),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // --- Activity sparkline bars ---
    let activity_days = &report.cost_analysis.daily_costs;
    let max_cost = activity_days.iter().map(|d| d.cost).fold(0.0f64, f64::max);
    let activity_bars = if activity_days.is_empty() {
        r#"<div class="muted">No daily data available.</div>"#.to_string()
    } else {
        activity_days
            .iter()
            .map(|day| {
                let pct = if max_cost > 0.0 {
                    ((day.cost / max_cost) * 100.0).round() as u64
                } else {
                    0
                };
                let label = day.date.get(5..).unwrap_or(&day.date);
                let cost_str = format_currency_compact(day.cost);
                format!(
                    r#"<div class="spark-col" title="{} · {}"><div class="spark-bar" style="height:{}%"></div><span class="spark-label">{}</span></div>"#,
                    escape_html(&day.date),
                    escape_html(&cost_str),
                    pct,
                    escape_html(label),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    // --- Model rows ---
    let model_rows = report
        .cost_analysis
        .model_costs
        .iter()
        .map(|(model, cost)| {
            let share = if report.cost_analysis.total_cost > 0.0 {
                (cost / report.cost_analysis.total_cost) * 100.0
            } else {
                0.0
            };
            format!(
                r#"<div class="model-row"><div class="model-name"><strong>{}</strong><span class="muted">{}</span></div><div class="bar-track"><div class="bar-fill" style="width:{:.1}%"></div></div><span class="model-pct">{:.0}%</span></div>"#,
                escape_html(model),
                escape_html(&format_currency_compact(*cost)),
                share,
                share,
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // --- Project rows ---
    let max_project_tokens = report
        .project_breakdown
        .first()
        .map(|p| p.output_tokens)
        .unwrap_or(1);
    let project_rows = report
        .project_breakdown
        .iter()
        .take(8)
        .map(|p| {
            let bar_pct = if max_project_tokens > 0 {
                (p.output_tokens * 100 / max_project_tokens).min(100)
            } else {
                0
            };
            format!(
                r#"<div class="proj-row"><div class="proj-info"><strong>{}</strong><div class="proj-bar-wrap"><div class="proj-bar" style="width:{}%"></div></div></div><span class="proj-sessions muted mono">{} sessions</span><span class="proj-tokens muted mono">{}</span></div>"#,
                escape_html(&p.name),
                bar_pct,
                p.session_count,
                escape_html(&format_tokens(p.output_tokens)),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // --- Costliest sessions ---
    let costliest_sessions = if report.session_breakdown.costly_sessions.is_empty() {
        r#"<p class="muted">No session data available.</p>"#.to_string()
    } else {
        report
            .session_breakdown
            .costly_sessions
            .iter()
            .take(6)
            .map(|s| {
                let date = s
                    .timestamp_start
                    .as_deref()
                    .map(|v| &v[..v.len().min(10)])
                    .unwrap_or("—");
                format!(
                    r#"<div class="session-row"><div><div class="session-project">{}</div><div class="session-date muted mono">{}</div></div><span class="token-badge">{}</span></div>"#,
                    escape_html(&s.project_name),
                    escape_html(date),
                    escape_html(&format_tokens(s.total_tokens)),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    // --- Subagent spikes ---
    let subagent_spikes = if report.session_breakdown.costly_subagents.is_empty() {
        r#"<p class="muted">No subagent spikes recorded.</p>"#.to_string()
    } else {
        report
            .session_breakdown
            .costly_subagents
            .iter()
            .take(6)
            .map(|s| {
                let date = s
                    .timestamp_start
                    .as_deref()
                    .map(|v| &v[..v.len().min(10)])
                    .unwrap_or("—");
                let prompt = trim_text(
                    s.first_prompt
                        .as_deref()
                        .unwrap_or("No preview available."),
                    80,
                );
                format!(
                    r#"<div class="session-row"><div><div class="session-project">{}</div><div class="session-prompt muted">{} · {}</div></div><span class="token-badge">{}</span></div>"#,
                    escape_html(s.project_name.as_deref().unwrap_or("Subagent")),
                    escape_html(date),
                    escape_html(&prompt),
                    escape_html(&format_tokens(s.total_tokens)),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    // --- Highlights ---
    let highlights = wrapped
        .highlights
        .iter()
        .map(|h| {
            format!(
                r#"<article class="highlight-card"><div class="eyebrow accent-terra">{}</div><h3>{}</h3><p class="muted">{}</p></article>"#,
                escape_html(&h.eyebrow),
                escape_html(&h.title),
                escape_html(&h.note),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // --- Recommendations ---
    let recommendations = report
        .recommendations
        .iter()
        .take(6)
        .map(|r| {
            format!(
                r#"<article class="rec-card"><h3>{}</h3><p class="muted">{}</p></article>"#,
                escape_html(&r.title),
                escape_html(&r.action),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // --- Bento: power hour ---
    let (power_hour_label, power_hour_note) = wrapped
        .power_hour
        .as_ref()
        .map(|ph| (ph.label.clone(), format!("{}% of turns", ph.share_pct)))
        .unwrap_or_else(|| ("—".to_string(), "No peak hour data".to_string()));

    // --- Bento: top project ---
    let (top_project_name, top_project_meta) = wrapped
        .top_project
        .as_ref()
        .map(|p| {
            (
                p.name.clone(),
                format!("{}% of output · {} sessions", p.share_pct, p.session_count),
            )
        })
        .unwrap_or_else(|| ("—".to_string(), "No project data".to_string()));

    // --- Inflection note ---
    let inflection_html = report
        .inflection
        .as_ref()
        .map(|ip| {
            let cls = if ip.direction == "worsened" {
                "warn"
            } else {
                "good"
            };
            format!(
                r#"<div class="inflection-note {cls}">{}</div>"#,
                escape_html(&ip.summary),
            )
        })
        .unwrap_or_default();

    // --- Cache values ---
    let cache_ratio = format_ratio(report.cache_health.efficiency_ratio);
    let hit_rate = report.cache_health.cache_hit_rate;
    let cache_saved = report.cache_health.savings.from_caching;
    let cache_overhead = report.cache_health.savings.wasted_from_breaks;

    // --- Prompt ratio ---
    let human_pct = wrapped.prompt_ratio.human_pct;
    let tool_pct = 100u64.saturating_sub(human_pct);
    let human_count = wrapped.prompt_ratio.human;
    let tool_count = wrapped.prompt_ratio.tool;

    // --- Top tool ---
    let top_tool_html = wrapped
        .top_tool
        .as_ref()
        .map(|t| {
            format!(
                r#"<div class="bento-big accent-amber">{}</div><div class="bento-label">top tool</div><div class="muted">{} calls this season</div>"#,
                escape_html(&t.name),
                t.count,
            )
        })
        .unwrap_or_else(|| r#"<div class="muted">No tool data</div>"#.to_string());

    // --- Biggest session snippet ---
    let biggest_session_html = wrapped
        .biggest_session
        .as_ref()
        .map(|s| {
            let date = s
                .timestamp_start
                .as_deref()
                .map(|v| &v[..v.len().min(10)])
                .unwrap_or("—");
            let preview = trim_text(s.first_prompt.as_deref().unwrap_or(""), 100);
            format!(
                r#"<div class="bento-big accent-rose">{}</div><div class="bento-label">biggest session</div><div class="muted">{} · {}</div>{}"#,
                escape_html(&format_tokens(s.total_tokens)),
                escape_html(&s.project_name),
                escape_html(date),
                if preview.is_empty() { String::new() } else {
                    format!(r#"<div class="session-preview muted">{}</div>"#, escape_html(&preview))
                },
            )
        })
        .unwrap_or_else(|| r#"<div class="muted">No session data</div>"#.to_string());

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Claude Code Wrapped {year}</title>
  <style>
    :root {{
      color-scheme: dark;
      --bg: #0f0e0d;
      --surface: #191817;
      --surface-2: #201f1d;
      --surface-3: #272523;
      --border: rgba(255,255,255,0.07);
      --border-bright: rgba(255,255,255,0.10);
      --text: #f5f2ee;
      --muted: #b2a89e;
      --heading: #d4c5b0;
      --stone: #d4c5b0;
      --sage: #7ec49a;
      --neg: #a07060;
      --radius: 14px;
      --radius-lg: 20px;
    }}
    *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
      background-color: var(--bg);
      background-image: none;
      color: var(--text);
      min-height: 100vh;
      padding: 28px 20px 80px;
      line-height: 1.5;
      -webkit-font-smoothing: antialiased;
    }}
    .page {{ max-width: 1140px; margin: 0 auto; display: flex; flex-direction: column; gap: 10px; }}

    /* ── Typography ── */
    h2 {{ font-size: 16px; font-weight: 600; letter-spacing: -0.02em; color: var(--heading); }}
    h3 {{ font-size: 14px; font-weight: 600; letter-spacing: -0.01em; color: var(--text); }}
    p {{ font-size: 13px; }}
    .eyebrow {{ font-size: 10px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--muted); font-weight: 600; }}
    .muted {{ color: var(--muted); font-size: 13px; line-height: 1.6; }}
    .mono {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}

    /* ── Accent colors ── */
    .accent-terra  {{ color: var(--stone); }}
    .accent-moss   {{ color: var(--sage); }}
    .accent-amber  {{ color: var(--stone); }}
    .accent-rose   {{ color: var(--neg); }}

    /* ── Base panel ── */
    .panel {{
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius-lg);
      padding: 20px 22px;
    }}
    .section-header {{ margin-bottom: 14px; display: flex; flex-direction: column; gap: 3px; }}

    /* ── Hero ── */
    .hero {{
      background: var(--surface);
      border: 1px solid var(--border-bright);
      border-radius: var(--radius-lg);
      padding: 36px 40px 32px;
      position: relative;
      overflow: hidden;
    }}
    .hero::after {{
      content: "";
      position: absolute;
      inset: 0;
      background: none;
      pointer-events: none;
    }}
    .hero-top {{
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      margin-bottom: 20px;
    }}
    .grade-badge {{
      width: 52px; height: 52px;
      border-radius: 50%;
      display: flex; align-items: center; justify-content: center;
      font-size: 24px; font-weight: 800;
      letter-spacing: -0.03em;
      color: #fff;
      flex-shrink: 0;
      box-shadow: 0 0 28px rgba(0,0,0,0.4);
    }}
    .archetype-title {{
      font-size: clamp(44px, 6.5vw, 80px);
      font-weight: 800;
      letter-spacing: -0.05em;
      line-height: 0.93;
      margin-bottom: 16px;
      background: linear-gradient(130deg, #eef2f7 0%, rgba(238,242,247,0.55) 100%);
      -webkit-background-clip: text;
      -webkit-text-fill-color: transparent;
      background-clip: text;
    }}
    .hero-sub {{
      font-size: 14px;
      color: var(--muted);
      max-width: 540px;
      line-height: 1.65;
    }}

    /* ── Stat strip ── */
    .stat-strip {{
      display: flex;
      gap: 8px;
      overflow-x: auto;
      scrollbar-width: none;
    }}
    .stat-strip::-webkit-scrollbar {{ display: none; }}
    .stat-pill {{
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: 12px;
      padding: 14px 18px;
      flex-shrink: 0;
      min-width: 110px;
    }}
    .stat-pill-value {{ font-size: 24px; font-weight: 700; letter-spacing: -0.04em; line-height: 1.05; }}
    .stat-pill-label {{ font-size: 10px; color: var(--muted); margin-top: 4px; text-transform: uppercase; letter-spacing: 0.1em; }}

    /* ── Bento grids ── */
    .grid-3 {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: 10px; }}
    .grid-2 {{ display: grid; grid-template-columns: repeat(2, 1fr); gap: 10px; }}
    .bento-card {{
      background: var(--surface);
      border: 1px solid var(--border);
      border-radius: var(--radius-lg);
      padding: 20px 22px;
      display: flex;
      flex-direction: column;
      gap: 8px;
    }}
    .bento-big {{
      font-size: clamp(32px, 3.8vw, 50px);
      font-weight: 800;
      letter-spacing: -0.05em;
      line-height: 1;
    }}
    .bento-label {{
      font-size: 10px;
      letter-spacing: 0.14em;
      text-transform: uppercase;
      color: var(--heading);
      font-weight: 600;
    }}

    /* ── Activity chart ── */
    .activity-chart {{
      display: flex;
      align-items: flex-end;
      gap: 5px;
      height: 72px;
      margin-top: 4px;
    }}
    .spark-col {{
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: flex-end;
      height: 100%;
      gap: 5px;
    }}
    .spark-bar {{
      width: 100%;
      border-radius: 3px 3px 0 0;
      background: linear-gradient(180deg, var(--stone) 0%, rgba(212,197,176,0.2) 100%);
      min-height: 3px;
    }}
    .spark-label {{ font-size: 9px; color: var(--muted); letter-spacing: 0.02em; }}

    /* ── Model rows ── */
    .model-row {{
      display: grid;
      grid-template-columns: minmax(0, 2fr) minmax(0, 3fr) 38px;
      gap: 10px;
      align-items: center;
      padding: 8px 0;
      border-bottom: 1px solid var(--border);
    }}
    .model-row:last-child {{ border-bottom: none; }}
    .model-name {{ display: flex; flex-direction: column; gap: 2px; }}
    .model-name strong {{ font-size: 12px; }}
    .model-name span {{ font-size: 11px; }}
    .bar-track {{
      height: 5px;
      border-radius: 999px;
      background: var(--surface-3);
      overflow: hidden;
    }}
    .bar-fill {{
      height: 100%;
      border-radius: inherit;
      background: linear-gradient(90deg, var(--stone), var(--sage));
    }}
    .model-pct {{ font-size: 11px; color: var(--muted); text-align: right; font-family: ui-monospace, monospace; }}

    /* ── Project rows ── */
    .proj-row {{
      display: grid;
      grid-template-columns: minmax(0, 2fr) 80px 80px;
      gap: 10px;
      align-items: center;
      padding: 9px 0;
      border-bottom: 1px solid var(--border);
    }}
    .proj-row:last-child {{ border-bottom: none; }}
    .proj-info strong {{ font-size: 12px; display: block; margin-bottom: 5px; }}
    .proj-bar-wrap {{ height: 3px; background: var(--surface-3); border-radius: 2px; overflow: hidden; }}
    .proj-bar {{ height: 100%; border-radius: inherit; background: var(--sage); }}
    .proj-sessions, .proj-tokens {{ font-size: 11px; text-align: right; }}

    /* ── Session rows ── */
    .session-row {{
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      gap: 12px;
      padding: 9px 0;
      border-bottom: 1px solid var(--border);
    }}
    .session-row:last-child {{ border-bottom: none; }}
    .session-project {{ font-size: 12px; font-weight: 600; margin-bottom: 3px; }}
    .session-date {{ font-size: 11px; }}
    .session-prompt {{ font-size: 11px; margin-top: 2px; max-width: 300px; }}
    .session-preview {{ font-size: 11px; margin-top: 6px; padding: 8px 10px; background: var(--surface-3); border-radius: 6px; line-height: 1.5; }}
    .token-badge {{
      background: rgba(212,197,176,0.07);
      color: var(--stone);
      border: 1px solid rgba(212,197,176,0.16);
      border-radius: 7px;
      padding: 4px 9px;
      font-size: 11px;
      font-weight: 700;
      white-space: nowrap;
      font-family: ui-monospace, monospace;
      flex-shrink: 0;
    }}

    /* ── Highlight cards ── */
    .highlight-card {{
      background: var(--surface-2);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      padding: 16px 18px;
      display: flex;
      flex-direction: column;
      gap: 7px;
    }}
    .highlight-card h3 {{ font-size: 13px; }}

    /* ── Rec cards ── */
    .rec-card {{
      background: var(--surface-2);
      border: 1px solid var(--border);
      border-radius: var(--radius);
      padding: 16px 18px;
    }}
    .rec-card h3 {{ font-size: 13px; margin-bottom: 5px; }}

    /* ── Ratio bar ── */
    .ratio-split {{
      height: 7px;
      border-radius: 999px;
      overflow: hidden;
      background: rgba(255,255,255,0.08);
      margin: 6px 0 8px;
      display: flex;
    }}
    .ratio-human {{ height: 100%; background: var(--stone); border-radius: inherit; }}
    .ratio-labels {{
      display: flex;
      justify-content: space-between;
      font-size: 11px;
      color: var(--muted);
    }}
    .ratio-labels strong {{ color: var(--text); }}

    /* ── Cache savings ── */
    .savings-row {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 7px 0;
      border-bottom: 1px solid var(--border);
      font-size: 12px;
    }}
    .savings-row:last-child {{ border-bottom: none; }}
    .savings-pos {{ color: var(--sage); font-weight: 700; font-family: ui-monospace, monospace; }}
    .savings-neg {{ color: var(--neg); font-weight: 700; font-family: ui-monospace, monospace; }}

    /* ── Inflection note ── */
    .inflection-note {{
      font-size: 11px;
      padding: 8px 12px;
      border-radius: 7px;
      margin-top: 4px;
      line-height: 1.5;
    }}
    .inflection-note.warn {{
      background: rgba(160,112,96,0.08);
      color: var(--neg);
      border: 1px solid rgba(160,112,96,0.18);
    }}
    .inflection-note.good {{
      background: rgba(107,158,122,0.08);
      color: var(--sage);
      border: 1px solid rgba(107,158,122,0.18);
    }}

    /* ── Divider ── */
    .row-divider {{ height: 1px; background: var(--border); margin: 14px 0; }}

    /* ── Responsive ── */
    @media (max-width: 860px) {{
      .grid-3 {{ grid-template-columns: 1fr; }}
      .grid-2 {{ grid-template-columns: 1fr; }}
      .stat-pill {{ min-width: calc(50% - 4px); flex-shrink: 0; }}
      .hero {{ padding: 24px; }}
    }}
    @media (max-width: 520px) {{
      body {{ padding: 14px 12px 60px; }}
      .archetype-title {{ font-size: 36px; }}
      .hero {{ padding: 20px; }}
      .bento-card, .panel {{ padding: 16px; }}
    }}
  </style>
</head>
<body>
  <div class="page">

    <!-- ── HERO ── -->
    <section class="hero">
      <div class="hero-top">
        <span class="eyebrow">Claude Code Wrapped · {year} · {active_days} active days</span>
        <div class="grade-badge" style="background:{grade_color}" title="Cache grade: {grade_label}">{grade_letter}</div>
      </div>
      <div class="archetype-title">{archetype_title}</div>
      <p class="hero-sub">{summary}</p>
    </section>

    <!-- ── STAT STRIP ── -->
    <div class="stat-strip">{stat_pills}</div>

    <!-- ── BENTO ROW 1: power hour / top project / cache ── -->
    <div class="grid-3">
      <div class="bento-card">
        <div class="bento-label">Power hour</div>
        <div class="bento-big accent-terra">{power_hour_label}</div>
        <div class="muted">{power_hour_note}</div>
      </div>
      <div class="bento-card">
        <div class="bento-label">Main project</div>
        <div class="bento-big accent-moss">{top_project_name}</div>
        <div class="muted">{top_project_meta}</div>
      </div>
      <div class="bento-card">
        <div class="bento-label">Cache health</div>
        <div class="bento-big" style="color:{grade_color}">{grade_letter} &nbsp;<span style="font-size:60%;opacity:0.7">{cache_ratio}</span></div>
        <div class="muted">{grade_label} · {hit_rate:.1}% hit rate</div>
        {inflection_html}
      </div>
    </div>

    <!-- ── BENTO ROW 2: top tool / biggest session / prompt+savings ── -->
    <div class="grid-3">
      <div class="bento-card">
        {top_tool_html}
      </div>
      <div class="bento-card">
        {biggest_session_html}
      </div>
      <div class="bento-card">
        <div class="bento-label">Human vs tool turns</div>
        <div class="ratio-split">
          <div class="ratio-human" style="width:{human_pct}%"></div>
        </div>
        <div class="ratio-labels"><span><strong>{human_count}</strong> human ({human_pct}%)</span><span><strong>{tool_count}</strong> tool ({tool_pct}%)</span></div>
        <div class="row-divider"></div>
        <div class="bento-label">Cache savings</div>
        <div class="savings-row"><span class="muted">Saved from caching</span><span class="savings-pos">+${cache_saved}</span></div>
        <div class="savings-row"><span class="muted">Overhead from breaks</span><span class="savings-neg">-${cache_overhead}</span></div>
      </div>
    </div>

    <!-- ── ACTIVITY CHART ── -->
    <div class="bento-card">
      <div class="section-header">
        <div class="eyebrow">Activity</div>
        <h2>Daily spend</h2>
      </div>
      <div class="activity-chart">{activity_bars}</div>
    </div>

    <!-- ── MODEL MIX + PROJECTS ── -->
    <div class="grid-2">
      <div class="panel">
        <div class="section-header">
          <div class="eyebrow">Model mix</div>
          <h2>Routing distribution</h2>
        </div>
        {model_rows}
      </div>
      <div class="panel">
        <div class="section-header">
          <div class="eyebrow">Projects</div>
          <h2>Top projects</h2>
        </div>
        {project_rows}
      </div>
    </div>

    <!-- ── SESSIONS + SUBAGENTS ── -->
    <div class="grid-2">
      <div class="panel">
        <div class="section-header">
          <div class="eyebrow">Costliest sessions</div>
          <h2>Heaviest runs</h2>
        </div>
        {costliest_sessions}
      </div>
      <div class="panel">
        <div class="section-header">
          <div class="eyebrow">Subagent spikes</div>
          <h2>Background bursts</h2>
        </div>
        {subagent_spikes}
      </div>
    </div>

    <!-- ── HIGHLIGHTS ── -->
    <div class="panel">
      <div class="section-header">
        <div class="eyebrow">Season highlights</div>
        <h2>Standout moments</h2>
      </div>
      <div class="grid-3">{highlights}</div>
    </div>

    <!-- ── RECOMMENDATIONS ── -->
    <div class="panel">
      <div class="section-header">
        <div class="eyebrow">Next season</div>
        <h2>Upgrades worth making</h2>
      </div>
      <div class="grid-3">{recommendations}</div>
    </div>

  </div>
</body>
</html>"#,
        year = report.year,
        active_days = report.cost_analysis.active_days,
        grade_color = escape_html(&grade.color),
        grade_letter = escape_html(&grade.letter),
        grade_label = escape_html(&grade.label),
        archetype_title = escape_html(&wrapped.archetype.title),
        summary = escape_html(&wrapped.summary),
        stat_pills = stat_pills,
        power_hour_label = escape_html(&power_hour_label),
        power_hour_note = escape_html(&power_hour_note),
        top_project_name = escape_html(&top_project_name),
        top_project_meta = escape_html(&top_project_meta),
        cache_ratio = escape_html(&cache_ratio),
        hit_rate = hit_rate,
        inflection_html = inflection_html,
        top_tool_html = top_tool_html,
        biggest_session_html = biggest_session_html,
        human_pct = human_pct,
        tool_pct = tool_pct,
        human_count = human_count,
        tool_count = tool_count,
        cache_saved = cache_saved,
        cache_overhead = cache_overhead,
        activity_bars = activity_bars,
        model_rows = model_rows,
        project_rows = project_rows,
        costliest_sessions = costliest_sessions,
        subagent_spikes = subagent_spikes,
        highlights = highlights,
        recommendations = recommendations,
    )
}
