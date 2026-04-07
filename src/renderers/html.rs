use crate::{escape_html, format_currency_compact, format_ratio, format_tokens, trim_text, Report};

pub fn render_html(report: &Report) -> String {
    let wrapped = &report.wrapped_story;
    let grade = &report.cache_health.grade;

    // Total cost formatted for the big splash display
    let total_cost_display = {
        let cost = report.cost_analysis.total_cost;
        if cost < 1000.0 {
            format!("${:.2}", cost)
        } else {
            let whole = cost as u64;
            let frac = ((cost - whole as f64) * 100.0).round() as u64;
            let s = whole.to_string();
            let mut with_commas = String::new();
            for (i, c) in s.chars().rev().enumerate() {
                if i > 0 && i % 3 == 0 {
                    with_commas.push(',');
                }
                with_commas.push(c);
            }
            let reversed: String = with_commas.chars().rev().collect();
            format!("${}.{:02}", reversed, frac)
        }
    };

    // Hero stats strip (small, at bottom of opening slide)
    let hero_stats_html = wrapped
        .hero
        .iter()
        .map(|h| {
            format!(
                r#"<div class="hero-stat"><div class="hero-stat-val">{}</div><div class="hero-stat-lbl">{}</div></div>"#,
                escape_html(&h.value),
                escape_html(&h.label),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // Activity sparkline bars
    let activity_days = &report.cost_analysis.daily_costs;
    let max_cost = activity_days.iter().map(|d| d.cost).fold(0.0f64, f64::max);
    let activity_bars = if activity_days.is_empty() {
        r#"<div style="opacity:0.35;font-size:13px">No daily data available.</div>"#.to_string()
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

    // Model rows
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
                r#"<div class="model-row"><div class="model-row-top"><strong>{}</strong><span>{} · {:.0}%</span></div><div class="bar-track"><div class="bar-fill" style="width:{:.1}%"></div></div></div>"#,
                escape_html(model),
                escape_html(&format_currency_compact(*cost)),
                share,
                share,
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // Project rows
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
                r#"<div class="proj-row"><div><div class="proj-name">{}</div><div class="proj-bar-wrap"><div class="proj-bar" style="width:{}%"></div></div></div><span class="proj-sessions">{} sessions</span><span class="proj-tokens">{}</span></div>"#,
                escape_html(&p.name),
                bar_pct,
                p.session_count,
                escape_html(&format_tokens(p.output_tokens)),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // Costliest sessions
    let costliest_sessions = if report.session_breakdown.costly_sessions.is_empty() {
        r#"<p style="opacity:0.35;font-size:13px">No session data available.</p>"#.to_string()
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
                    r#"<div class="session-row"><div><div class="session-project">{}</div><div class="session-meta">{}</div></div><span class="token-badge">{}</span></div>"#,
                    escape_html(&s.project_name),
                    escape_html(date),
                    escape_html(&format_tokens(s.total_tokens)),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    // Subagent spikes
    let subagent_spikes = if report.session_breakdown.costly_subagents.is_empty() {
        r#"<p style="opacity:0.35;font-size:13px">No subagent spikes recorded.</p>"#.to_string()
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
                    r#"<div class="session-row"><div><div class="session-project">{}</div><div class="session-meta">{}</div><div class="session-prompt">{}</div></div><span class="token-badge">{}</span></div>"#,
                    escape_html(s.project_name.as_deref().unwrap_or("Subagent")),
                    escape_html(date),
                    escape_html(&prompt),
                    escape_html(&format_tokens(s.total_tokens)),
                )
            })
            .collect::<Vec<_>>()
            .join("")
    };

    // Highlights
    let highlights = wrapped
        .highlights
        .iter()
        .map(|h| {
            format!(
                r#"<article class="card"><div class="eyebrow">{}</div><h3>{}</h3><p>{}</p></article>"#,
                escape_html(&h.eyebrow),
                escape_html(&h.title),
                escape_html(&h.note),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // Recommendations
    let recommendations = report
        .recommendations
        .iter()
        .take(6)
        .map(|r| {
            format!(
                r#"<article class="card"><h3>{}</h3><p>{}</p></article>"#,
                escape_html(&r.title),
                escape_html(&r.action),
            )
        })
        .collect::<Vec<_>>()
        .join("");

    // Power hour
    let (power_hour_label, power_hour_note) = wrapped
        .power_hour
        .as_ref()
        .map(|ph| (ph.label.clone(), format!("{}% of turns", ph.share_pct)))
        .unwrap_or_else(|| ("—".to_string(), "No peak hour data".to_string()));

    // Top project
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

    // Inflection note
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

    // Cache values
    let cache_ratio = format_ratio(report.cache_health.efficiency_ratio);
    let hit_rate = report.cache_health.cache_hit_rate;
    let cache_saved = report.cache_health.savings.from_caching;
    let cache_overhead = report.cache_health.savings.wasted_from_breaks;

    // Prompt ratio
    let human_pct = wrapped.prompt_ratio.human_pct;
    let tool_pct = 100u64.saturating_sub(human_pct);
    let human_count = wrapped.prompt_ratio.human;
    let tool_count = wrapped.prompt_ratio.tool;

    // Top tool
    let (top_tool_name, top_tool_meta) = wrapped
        .top_tool
        .as_ref()
        .map(|t| (t.name.clone(), format!("{} calls this season", t.count)))
        .unwrap_or_else(|| ("—".to_string(), "No tool data".to_string()));

    // Biggest session slide content
    let biggest_session_html = wrapped
        .biggest_session
        .as_ref()
        .map(|s| {
            let date = s
                .timestamp_start
                .as_deref()
                .map(|v| &v[..v.len().min(10)])
                .unwrap_or("—");
            let preview = trim_text(s.first_prompt.as_deref().unwrap_or(""), 120);
            let preview_html = if preview.is_empty() {
                String::new()
            } else {
                format!(
                    r#"<p style="font-size:clamp(13px,1.8vw,16px);color:#000;opacity:0.5;margin-top:20px;max-width:540px;line-height:1.65">{}</p>"#,
                    escape_html(&preview)
                )
            };
            format!(
                r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="slide-hero" style="color:#000">{tokens}</div>
      <p class="slide-sub" style="color:#000;opacity:0.55">{project} · {date}</p>
      {preview_html}"#,
                tokens = escape_html(&format_tokens(s.total_tokens)),
                project = escape_html(&s.project_name),
                date = escape_html(date),
                preview_html = preview_html,
            )
        })
        .unwrap_or_else(|| {
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div><p class="slide-sub" style="color:#000">No session data</p>"#.to_string()
        });

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Claude Code Wrapped {year}</title>
  <style>
    *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: ui-sans-serif, -apple-system, BlinkMacSystemFont, "Segoe UI", system-ui, sans-serif;
      background: #000;
      color: #fff;
      -webkit-font-smoothing: antialiased;
      overflow-x: hidden;
    }}

    /* ── Slide layout ── */
    .slide {{
      width: 100%;
      padding: 80px 40px;
      display: flex;
      align-items: center;
      justify-content: center;
    }}
    .slide-inner {{
      width: 100%;
      max-width: 860px;
    }}
    .stat-slide {{ padding: 96px 40px; }}
    .data-slide {{ padding: 72px 40px; }}

    /* Slide backgrounds */
    .s-black  {{ background: #000; color: #fff; }}
    .s-dark   {{ background: #121212; color: #fff; }}
    .s-green  {{ background: #1a8a47; color: #fff; }}
    .s-coral  {{ background: #9e2f1a; color: #fff; }}
    .s-purple {{ background: #3d1480; color: #fff; }}
    .s-amber  {{ background: #a06c1a; color: #fff; }}

    /* ── Opening slide ── */
    .opening-slide {{
      min-height: 100vh;
      align-items: flex-end;
      padding-bottom: 72px;
    }}
    .wordmark {{
      display: block;
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.28em;
      text-transform: uppercase;
      color: #1a8a47;
      margin-bottom: 48px;
    }}
    .archetype-title {{
      font-size: clamp(56px, 10vw, 120px);
      font-weight: 900;
      line-height: 0.88;
      letter-spacing: -0.04em;
      margin-bottom: 24px;
    }}
    .hero-desc {{
      font-size: clamp(15px, 2vw, 19px);
      opacity: 0.5;
      line-height: 1.6;
      max-width: 520px;
      margin-bottom: 48px;
    }}
    .hero-stats {{
      display: flex;
      gap: 36px;
      flex-wrap: wrap;
    }}
    .hero-stat-val {{
      font-size: clamp(22px, 3vw, 32px);
      font-weight: 800;
      letter-spacing: -0.04em;
      line-height: 1;
    }}
    .hero-stat-lbl {{
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      opacity: 0.35;
      margin-top: 6px;
    }}

    /* ── Slide typography ── */
    .slide-label {{
      font-size: clamp(11px, 1.4vw, 13px);
      font-weight: 700;
      letter-spacing: 0.22em;
      text-transform: uppercase;
      opacity: 0.5;
      margin-bottom: 20px;
    }}
    .slide-hero {{
      font-size: clamp(72px, 12vw, 148px);
      font-weight: 900;
      line-height: 0.87;
      letter-spacing: -0.04em;
      margin-bottom: 20px;
    }}
    .slide-hero-med {{
      font-size: clamp(52px, 8vw, 104px);
      font-weight: 900;
      line-height: 0.88;
      letter-spacing: -0.04em;
      margin-bottom: 20px;
    }}
    .slide-sub {{
      font-size: clamp(15px, 2.2vw, 20px);
      font-weight: 500;
      opacity: 0.6;
      line-height: 1.5;
      max-width: 480px;
    }}

    /* ── Cache grade hero ── */
    .cache-grade-hero {{
      font-size: clamp(160px, 26vw, 300px);
      font-weight: 900;
      line-height: 0.82;
      letter-spacing: -0.06em;
      margin-bottom: 32px;
    }}
    .cache-meta {{
      display: flex;
      gap: 36px;
      flex-wrap: wrap;
    }}
    .cache-stat-val {{
      font-size: clamp(24px, 3vw, 36px);
      font-weight: 800;
      letter-spacing: -0.03em;
      line-height: 1;
    }}
    .cache-stat-lbl {{
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      opacity: 0.4;
      margin-top: 6px;
    }}

    /* ── Section headers (data slides) ── */
    .section-label {{
      font-size: 11px;
      font-weight: 700;
      letter-spacing: 0.22em;
      text-transform: uppercase;
      opacity: 0.35;
      margin-bottom: 10px;
    }}
    .section-title {{
      font-size: clamp(26px, 3.5vw, 44px);
      font-weight: 900;
      letter-spacing: -0.03em;
      margin-bottom: 32px;
    }}

    /* ── Activity chart ── */
    .activity-chart {{
      display: flex;
      align-items: flex-end;
      gap: 4px;
      height: 140px;
    }}
    .spark-col {{
      flex: 1;
      display: flex;
      flex-direction: column;
      align-items: center;
      justify-content: flex-end;
      height: 100%;
      gap: 6px;
    }}
    .spark-bar {{
      width: 100%;
      border-radius: 3px 3px 0 0;
      background: #1a8a47;
      min-height: 3px;
    }}
    .spark-label {{ font-size: 9px; opacity: 0.3; letter-spacing: 0.02em; }}

    /* ── Model rows ── */
    .model-list {{ display: flex; flex-direction: column; gap: 18px; }}
    .model-row {{ display: flex; flex-direction: column; gap: 7px; }}
    .model-row-top {{ display: flex; justify-content: space-between; align-items: baseline; }}
    .model-row-top strong {{ font-size: 13px; font-weight: 600; }}
    .model-row-top span {{ font-size: 12px; opacity: 0.4; }}
    .bar-track {{ height: 3px; background: rgba(255,255,255,0.08); border-radius: 2px; overflow: hidden; }}
    .bar-fill {{ height: 100%; background: #1a8a47; border-radius: inherit; }}

    /* ── Project rows ── */
    .proj-list {{ display: flex; flex-direction: column; }}
    .proj-row {{
      display: grid;
      grid-template-columns: 1fr 72px 72px;
      gap: 12px;
      align-items: center;
      padding: 11px 0;
      border-bottom: 1px solid rgba(255,255,255,0.06);
    }}
    .proj-row:last-child {{ border-bottom: none; }}
    .proj-name {{ font-size: 13px; font-weight: 600; margin-bottom: 6px; }}
    .proj-bar-wrap {{ height: 2px; background: rgba(255,255,255,0.08); border-radius: 1px; overflow: hidden; }}
    .proj-bar {{ height: 100%; background: #1a8a47; border-radius: inherit; }}
    .proj-sessions, .proj-tokens {{ font-size: 11px; opacity: 0.35; text-align: right; font-family: ui-monospace, monospace; }}

    /* ── Session rows ── */
    .session-list {{ display: flex; flex-direction: column; }}
    .session-row {{
      display: flex;
      align-items: flex-start;
      justify-content: space-between;
      gap: 16px;
      padding: 11px 0;
      border-bottom: 1px solid rgba(255,255,255,0.06);
    }}
    .session-row:last-child {{ border-bottom: none; }}
    .session-project {{ font-size: 13px; font-weight: 600; margin-bottom: 3px; }}
    .session-meta {{ font-size: 11px; opacity: 0.35; font-family: ui-monospace, monospace; }}
    .session-prompt {{ font-size: 11px; opacity: 0.35; margin-top: 3px; max-width: 300px; line-height: 1.5; }}
    .token-badge {{
      background: rgba(29,185,84,0.1);
      color: #1a8a47;
      border: 1px solid rgba(29,185,84,0.22);
      border-radius: 6px;
      padding: 4px 8px;
      font-size: 11px;
      font-weight: 700;
      white-space: nowrap;
      font-family: ui-monospace, monospace;
      flex-shrink: 0;
    }}

    /* ── Cards (highlights + recs) ── */
    .card-grid {{ display: grid; grid-template-columns: repeat(3, 1fr); gap: 12px; }}
    .card {{
      background: #1a1a1a;
      border: 1px solid rgba(255,255,255,0.07);
      border-radius: 12px;
      padding: 20px;
    }}
    .eyebrow {{
      font-size: 10px;
      font-weight: 700;
      letter-spacing: 0.18em;
      text-transform: uppercase;
      color: #1a8a47;
      margin-bottom: 8px;
    }}
    .card h3 {{ font-size: 14px; font-weight: 700; margin-bottom: 6px; line-height: 1.4; }}
    .card p {{ font-size: 12px; opacity: 0.45; line-height: 1.65; }}

    /* ── Ratio bar ── */
    .ratio-bar {{
      height: 7px;
      border-radius: 999px;
      overflow: hidden;
      background: rgba(255,255,255,0.1);
      margin: 14px 0 10px;
      display: flex;
    }}
    .ratio-human {{ height: 100%; background: #1a8a47; }}
    .ratio-meta {{ display: flex; justify-content: space-between; font-size: 12px; opacity: 0.45; }}

    /* ── Cache savings ── */
    .savings-row {{
      display: flex;
      justify-content: space-between;
      align-items: center;
      padding: 10px 0;
      border-bottom: 1px solid rgba(255,255,255,0.07);
      font-size: 13px;
    }}
    .savings-row:last-child {{ border-bottom: none; }}
    .s-pos {{ color: #1a8a47; font-weight: 700; font-family: ui-monospace, monospace; }}
    .s-neg {{ color: #c04030; font-weight: 700; font-family: ui-monospace, monospace; }}
    .s-muted {{ opacity: 0.4; }}

    /* ── Inflection note ── */
    .inflection-note {{
      font-size: 12px;
      padding: 10px 14px;
      border-radius: 8px;
      margin-top: 20px;
      line-height: 1.55;
    }}
    .inflection-note.warn {{
      background: rgba(232,71,42,0.1);
      color: #E8472A;
      border: 1px solid rgba(232,71,42,0.2);
    }}
    .inflection-note.good {{
      background: rgba(29,185,84,0.08);
      color: #1a8a47;
      border: 1px solid rgba(29,185,84,0.2);
    }}

    /* ── 2-column layout ── */
    .data-grid-2 {{ display: grid; grid-template-columns: repeat(2, 1fr); gap: 60px; }}

    /* ── Responsive ── */
    @media (max-width: 700px) {{
      .slide {{ padding: 60px 28px; }}
      .data-slide {{ padding: 56px 28px; }}
      .card-grid {{ grid-template-columns: 1fr; }}
      .data-grid-2 {{ grid-template-columns: 1fr; gap: 48px; }}
      .hero-stats {{ gap: 24px; }}
      .cache-grade-hero {{ font-size: 140px; }}
    }}
    @media (max-width: 420px) {{
      .slide {{ padding: 48px 20px; }}
      .opening-slide {{ padding-bottom: 56px; }}
    }}
  </style>
</head>
<body>

  <!-- ── 1. OPENING / ARCHETYPE ── -->
  <section class="slide s-black opening-slide">
    <div class="slide-inner">
      <span class="wordmark">Claude Code Wrapped · {year}</span>
      <div class="archetype-title">{archetype_title}</div>
      <p class="hero-desc">{summary}</p>
      <div class="hero-stats">{hero_stats_html}</div>
    </div>
  </section>

  <!-- ── 2. SEASON SPEND ── -->
  <section class="slide s-green stat-slide">
    <div class="slide-inner">
      <div class="slide-label" style="color:rgba(0,0,0,0.45)">Season spend</div>
      <div class="slide-hero" style="color:#000">{total_cost_display}</div>
      <p class="slide-sub" style="color:#000;opacity:0.55">{active_days} active days</p>
    </div>
  </section>

  <!-- ── 3. POWER HOUR ── -->
  <section class="slide s-purple stat-slide">
    <div class="slide-inner">
      <div class="slide-label" style="color:rgba(255,255,255,0.45)">Peak hour</div>
      <div class="slide-hero">{power_hour_label}</div>
      <p class="slide-sub">{power_hour_note}</p>
    </div>
  </section>

  <!-- ── 4. TOP PROJECT ── -->
  <section class="slide s-coral stat-slide">
    <div class="slide-inner">
      <div class="slide-label" style="color:rgba(255,255,255,0.45)">Main project</div>
      <div class="slide-hero-med">{top_project_name}</div>
      <p class="slide-sub">{top_project_meta}</p>
    </div>
  </section>

  <!-- ── 5. CACHE GRADE ── -->
  <section class="slide s-dark stat-slide">
    <div class="slide-inner">
      <div class="slide-label">Cache health · {grade_label}</div>
      <div class="cache-grade-hero" style="color:{grade_color}">{grade_letter}</div>
      <div class="cache-meta">
        <div>
          <div class="cache-stat-val">{hit_rate:.1}%</div>
          <div class="cache-stat-lbl">Hit rate</div>
        </div>
        <div>
          <div class="cache-stat-val">{cache_ratio}</div>
          <div class="cache-stat-lbl">Cache ratio</div>
        </div>
        <div>
          <div class="cache-stat-val" style="color:#1a8a47">+${cache_saved}</div>
          <div class="cache-stat-lbl">Saved</div>
        </div>
      </div>
      {inflection_html}
    </div>
  </section>

  <!-- ── 6. TOP TOOL ── -->
  <section class="slide s-black stat-slide">
    <div class="slide-inner">
      <div class="slide-label">Favorite tool</div>
      <div class="slide-hero" style="color:#1a8a47">{top_tool_name}</div>
      <p class="slide-sub" style="opacity:0.45">{top_tool_meta}</p>
    </div>
  </section>

  <!-- ── 7. BIGGEST SESSION ── -->
  <section class="slide s-amber stat-slide">
    <div class="slide-inner">
      {biggest_session_html}
    </div>
  </section>

  <!-- ── 8. ACTIVITY CHART ── -->
  <section class="slide s-dark data-slide">
    <div class="slide-inner">
      <div class="section-label">Activity</div>
      <div class="section-title">Daily spend</div>
      <div class="activity-chart">{activity_bars}</div>
    </div>
  </section>

  <!-- ── 9. MODEL MIX + PROJECTS ── -->
  <section class="slide s-black data-slide">
    <div class="slide-inner">
      <div class="data-grid-2">
        <div>
          <div class="section-label">Model mix</div>
          <div class="section-title">Routing</div>
          <div class="model-list">{model_rows}</div>
        </div>
        <div>
          <div class="section-label">Projects</div>
          <div class="section-title">Top projects</div>
          <div class="proj-list">{project_rows}</div>
        </div>
      </div>
    </div>
  </section>

  <!-- ── 10. SESSIONS + SUBAGENTS ── -->
  <section class="slide s-dark data-slide">
    <div class="slide-inner">
      <div class="data-grid-2">
        <div>
          <div class="section-label">Costliest sessions</div>
          <div class="section-title">Heaviest runs</div>
          <div class="session-list">{costliest_sessions}</div>
        </div>
        <div>
          <div class="section-label">Subagent spikes</div>
          <div class="section-title">Background bursts</div>
          <div class="session-list">{subagent_spikes}</div>
        </div>
      </div>
    </div>
  </section>

  <!-- ── 11. PROMPT RATIO + CACHE SAVINGS ── -->
  <section class="slide s-black data-slide">
    <div class="slide-inner">
      <div class="data-grid-2">
        <div>
          <div class="section-label">Turn breakdown</div>
          <div class="section-title">Human vs tool</div>
          <div class="ratio-bar">
            <div class="ratio-human" style="width:{human_pct}%"></div>
          </div>
          <div class="ratio-meta">
            <span>{human_count} human ({human_pct}%)</span>
            <span>{tool_count} tool ({tool_pct}%)</span>
          </div>
        </div>
        <div>
          <div class="section-label">Cache efficiency</div>
          <div class="section-title">Savings</div>
          <div class="savings-row"><span class="s-muted">Saved from caching</span><span class="s-pos">+${cache_saved}</span></div>
          <div class="savings-row"><span class="s-muted">Overhead from breaks</span><span class="s-neg">-${cache_overhead}</span></div>
        </div>
      </div>
    </div>
  </section>

  <!-- ── 12. HIGHLIGHTS ── -->
  <section class="slide s-dark data-slide">
    <div class="slide-inner">
      <div class="section-label">Season highlights</div>
      <div class="section-title">Standout moments</div>
      <div class="card-grid">{highlights}</div>
    </div>
  </section>

  <!-- ── 13. RECOMMENDATIONS ── -->
  <section class="slide s-black data-slide">
    <div class="slide-inner">
      <div class="section-label">Next season</div>
      <div class="section-title">Upgrades worth making</div>
      <div class="card-grid">{recommendations}</div>
    </div>
  </section>

</body>
</html>"#,
        year = report.year,
        active_days = report.cost_analysis.active_days,
        grade_color = escape_html(&grade.color),
        grade_letter = escape_html(&grade.letter),
        grade_label = escape_html(&grade.label),
        archetype_title = escape_html(&wrapped.archetype.title),
        summary = escape_html(&wrapped.summary),
        total_cost_display = escape_html(&total_cost_display),
        hero_stats_html = hero_stats_html,
        power_hour_label = escape_html(&power_hour_label),
        power_hour_note = escape_html(&power_hour_note),
        top_project_name = escape_html(&top_project_name),
        top_project_meta = escape_html(&top_project_meta),
        cache_ratio = escape_html(&cache_ratio),
        hit_rate = hit_rate,
        inflection_html = inflection_html,
        biggest_session_html = biggest_session_html,
        top_tool_name = escape_html(&top_tool_name),
        top_tool_meta = escape_html(&top_tool_meta),
        activity_bars = activity_bars,
        model_rows = model_rows,
        project_rows = project_rows,
        costliest_sessions = costliest_sessions,
        subagent_spikes = subagent_spikes,
        human_pct = human_pct,
        tool_pct = tool_pct,
        human_count = human_count,
        tool_count = tool_count,
        cache_saved = cache_saved,
        cache_overhead = cache_overhead,
        highlights = highlights,
        recommendations = recommendations,
    )
}
