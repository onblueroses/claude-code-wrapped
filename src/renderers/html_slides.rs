use crate::{
    escape_html, format_currency, format_ratio, format_tokens, ranked_projects, trim_text,
    with_grouping, Report, SessionSummary,
};

pub fn slide_opening(report: &Report) -> String {
    format!(
        r#"  <!-- ── 1. OPENING / ARCHETYPE ── -->
  <section class="slide s-black opening-slide">
    <div class="slide-inner">
      <span class="wordmark">Claude Code Wrapped · {year}</span>
      <div class="archetype-title">{title}</div>
      <p class="hero-desc">{summary}</p>
      <div class="hero-stats">{hero_stats}</div>
    </div>
  </section>"#,
        year = report.year,
        title = escape_html(&report.wrapped_story.archetype.title),
        summary = escape_html(&report.wrapped_story.summary),
        hero_stats = hero_stats_html(report),
    )
}

pub fn slide_spend(report: &Report) -> String {
    format!(
        r#"  <!-- ── 2. SEASON SPEND ── -->
  <section class="slide s-green stat-slide">
    <div class="slide-inner">
      <div class="slide-label" style="color:rgba(0,0,0,0.45)">Season spend</div>
      <div class="slide-hero" style="color:#000">{total_cost}</div>
      <p class="slide-sub" style="color:#000;opacity:0.55">{active_days} active days</p>
    </div>
  </section>"#,
        total_cost = escape_html(&total_cost_display(report)),
        active_days = report.cost_analysis.active_days,
    )
}

pub fn slide_power_hour(report: &Report) -> String {
    let (label, note) = power_hour_data(report);
    format!(
        r#"  <!-- ── 3. POWER HOUR ── -->
  <section class="slide s-purple stat-slide">
    <div class="slide-inner">
      <div class="slide-label" style="color:rgba(255,255,255,0.45)">Peak hour</div>
      <div class="slide-hero">{label}</div>
      <p class="slide-sub">{note}</p>
    </div>
  </section>"#,
        label = escape_html(&label),
        note = escape_html(&note),
    )
}

pub fn slide_top_project(report: &Report) -> String {
    let (name, meta) = top_project_data(report);
    format!(
        r#"  <!-- ── 4. TOP PROJECT ── -->
  <section class="slide s-coral stat-slide">
    <div class="slide-inner">
      <div class="slide-label" style="color:rgba(255,255,255,0.45)">Main project</div>
      <div class="slide-hero-med">{name}</div>
      <p class="slide-sub">{meta}</p>
    </div>
  </section>"#,
        name = escape_html(&name),
        meta = escape_html(&meta),
    )
}

pub fn slide_cache_grade(report: &Report) -> String {
    let grade = &report.cache_health.grade;
    format!(
        r#"  <!-- ── 5. CACHE GRADE ── -->
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
      {inflection}
    </div>
  </section>"#,
        grade_label = escape_html(&grade.label),
        grade_color = escape_html(&grade.color),
        grade_letter = escape_html(&grade.letter),
        hit_rate = report.cache_health.cache_hit_rate,
        cache_ratio = escape_html(&format_ratio(report.cache_health.efficiency_ratio)),
        cache_saved = report.cache_health.savings.from_caching,
        inflection = inflection_html(report),
    )
}

pub fn slide_top_tool(report: &Report) -> String {
    let (name, meta) = top_tool_data(report);
    format!(
        r#"  <!-- ── 6. TOP TOOL ── -->
  <section class="slide s-black stat-slide">
    <div class="slide-inner">
      <div class="slide-label">Favorite tool</div>
      <div class="slide-hero" style="color:#1a8a47">{name}</div>
      <p class="slide-sub" style="opacity:0.45">{meta}</p>
    </div>
  </section>"#,
        name = escape_html(&name),
        meta = escape_html(&meta),
    )
}

pub fn slide_biggest_session(report: &Report) -> String {
    format!(
        r#"  <!-- ── 7. BIGGEST SESSION ── -->
  <section class="slide s-amber stat-slide">
    <div class="slide-inner">
      {content}
    </div>
  </section>"#,
        content = biggest_session_content(report),
    )
}

pub fn slide_activity(report: &Report) -> String {
    format!(
        r#"  <!-- ── 8. ACTIVITY CHART ── -->
  <section class="slide s-dark data-slide">
    <div class="slide-inner">
      <div class="section-label">Activity</div>
      <div class="section-title">Daily spend</div>
      <div class="activity-chart">{bars}</div>
    </div>
  </section>"#,
        bars = activity_bars(report),
    )
}

pub fn slide_model_and_projects(report: &Report) -> String {
    format!(
        r#"  <!-- ── 9. MODEL MIX + PROJECTS ── -->
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
  </section>"#,
        model_rows = model_rows(report),
        project_rows = project_rows(report),
    )
}

pub fn slide_sessions_and_subagents(report: &Report) -> String {
    format!(
        r#"  <!-- ── 10. SESSIONS + SUBAGENTS ── -->
  <section class="slide s-dark data-slide">
    <div class="slide-inner">
      <div class="data-grid-2">
        <div>
          <div class="section-label">Costliest sessions</div>
          <div class="section-title">Heaviest runs</div>
          <div class="session-list">{sessions}</div>
        </div>
        <div>
          <div class="section-label">Subagent spikes</div>
          <div class="section-title">Background bursts</div>
          <div class="session-list">{subagents}</div>
        </div>
      </div>
    </div>
  </section>"#,
        sessions = costliest_sessions(report),
        subagents = subagent_spikes(report),
    )
}

pub fn slide_prompts_and_savings(report: &Report) -> String {
    let human_pct = report.wrapped_story.prompt_ratio.human_pct;
    let tool_pct = 100u64.saturating_sub(human_pct);
    format!(
        r#"  <!-- ── 11. PROMPT RATIO + CACHE SAVINGS ── -->
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
  </section>"#,
        human_pct = human_pct,
        human_count = report.wrapped_story.prompt_ratio.human,
        tool_count = report.wrapped_story.prompt_ratio.tool,
        tool_pct = tool_pct,
        cache_saved = report.cache_health.savings.from_caching,
        cache_overhead = report.cache_health.savings.wasted_from_breaks,
    )
}

pub fn slide_highlights(report: &Report) -> String {
    format!(
        r#"  <!-- ── 12. HIGHLIGHTS ── -->
  <section class="slide s-dark data-slide">
    <div class="slide-inner">
      <div class="section-label">Season highlights</div>
      <div class="section-title">Standout moments</div>
      <div class="card-grid">{highlights}</div>
    </div>
  </section>"#,
        highlights = highlights_html(report),
    )
}

pub fn slide_recommendations(report: &Report) -> String {
    format!(
        r#"  <!-- ── 13. RECOMMENDATIONS ── -->
  <section class="slide s-black data-slide">
    <div class="slide-inner">
      <div class="section-label">Next season</div>
      <div class="section-title">Upgrades worth making</div>
      <div class="card-grid">{recommendations}</div>
    </div>
  </section>"#,
        recommendations = recommendations_html(report),
    )
}

fn hero_stats_html(report: &Report) -> String {
    report
        .wrapped_story
        .hero
        .iter()
        .map(|hero| {
            format!(
                r#"<div class="hero-stat"><div class="hero-stat-val">{}</div><div class="hero-stat-lbl">{}</div></div>"#,
                escape_html(&hero.value),
                escape_html(&hero.label),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

/// Keeps cents on 4-digit totals for the slide hero so the headline cost matches
/// billed spend more precisely than the compact `format_currency` summary style.
fn total_cost_display(report: &Report) -> String {
    let cost = report.cost_analysis.total_cost;
    if cost >= 1000.0 {
        let cents = (cost * 100.0).round() as u64;
        let whole = cents / 100;
        let frac = cents % 100;
        format!("${}.{:02}", with_grouping(whole), frac)
    } else {
        format!("${cost:.2}")
    }
}

fn activity_bars(report: &Report) -> String {
    let activity_days = &report.cost_analysis.daily_costs;
    let max_cost = activity_days
        .iter()
        .map(|day| day.cost)
        .fold(0.0f64, f64::max);
    if activity_days.is_empty() {
        return r#"<div style="opacity:0.35;font-size:13px">No daily data available.</div>"#
            .to_string();
    }

    activity_days
        .iter()
        .map(|day| {
            let pct = if max_cost > 0.0 {
                ((day.cost / max_cost) * 100.0).round() as u64
            } else {
                0
            };
            let label = day.date.get(5..).unwrap_or(&day.date);
            format!(
                r#"<div class="spark-col" title="{} · {}"><div class="spark-bar" style="height:{}%"></div><span class="spark-label">{}</span></div>"#,
                escape_html(&day.date),
                escape_html(&format_currency(day.cost)),
                pct,
                escape_html(label),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn model_rows(report: &Report) -> String {
    report
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
                escape_html(&format_currency(*cost)),
                share,
                share,
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn project_rows(report: &Report) -> String {
    let projects = ranked_projects(&report.project_breakdown);
    let max_project_tokens = projects
        .first()
        .map(|project| project.output_tokens)
        .unwrap_or(1);

    projects
        .into_iter()
        .take(8)
        .map(|project| {
            let bar_pct = if max_project_tokens > 0 {
                (project.output_tokens * 100 / max_project_tokens).min(100)
            } else {
                0
            };
            format!(
                r#"<div class="proj-row"><div><div class="proj-name">{}</div><div class="proj-bar-wrap"><div class="proj-bar" style="width:{}%"></div></div></div><span class="proj-sessions">{} sessions</span><span class="proj-tokens">{}</span></div>"#,
                escape_html(&project.name),
                bar_pct,
                project.session_count,
                escape_html(&format_tokens(project.output_tokens)),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn costliest_sessions(report: &Report) -> String {
    if report.session_breakdown.sessions.is_empty() {
        return r#"<p style="opacity:0.35;font-size:13px">No session data available.</p>"#
            .to_string();
    }

    report
        .session_breakdown
        .sessions
        .iter()
        .take(6)
        .map(|session| {
            let date = session
                .timestamp_start
                .as_deref()
                .map(|value| &value[..value.len().min(10)])
                .unwrap_or("—");
            format!(
                r#"<div class="session-row"><div><div class="session-project">{}</div><div class="session-meta">{}</div></div><span class="token-badge">{}</span></div>"#,
                escape_html(&session.project_name),
                escape_html(date),
                escape_html(&format_tokens(session.total_tokens)),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn subagent_spikes(report: &Report) -> String {
    if report.session_breakdown.costly_subagents.is_empty() {
        return r#"<p style="opacity:0.35;font-size:13px">No subagent spikes recorded.</p>"#
            .to_string();
    }

    report
        .session_breakdown
        .costly_subagents
        .iter()
        .take(6)
        .map(|subagent| {
            let date = subagent
                .timestamp_start
                .as_deref()
                .map(|value| &value[..value.len().min(10)])
                .unwrap_or("—");
            let prompt = trim_text(
                subagent
                    .first_prompt
                    .as_deref()
                    .unwrap_or("No preview available."),
                80,
            );
            format!(
                r#"<div class="session-row"><div><div class="session-project">{}</div><div class="session-meta">{}</div><div class="session-prompt">{}</div></div><span class="token-badge">{}</span></div>"#,
                escape_html(subagent.project_name.as_deref().unwrap_or("Subagent")),
                escape_html(date),
                escape_html(&prompt),
                escape_html(&format_tokens(subagent.total_tokens)),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn highlights_html(report: &Report) -> String {
    report
        .wrapped_story
        .highlights
        .iter()
        .map(|highlight| {
            format!(
                r#"<article class="card"><div class="eyebrow">{}</div><h3>{}</h3><p>{}</p></article>"#,
                escape_html(&highlight.eyebrow),
                escape_html(&highlight.title),
                escape_html(&highlight.note),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn recommendations_html(report: &Report) -> String {
    report
        .recommendations
        .iter()
        .take(6)
        .map(|recommendation| {
            format!(
                r#"<article class="card"><h3>{}</h3><p>{}</p></article>"#,
                escape_html(&recommendation.title),
                escape_html(&recommendation.action),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn power_hour_data(report: &Report) -> (String, String) {
    report
        .wrapped_story
        .power_hour
        .as_ref()
        .map(|bucket| {
            (
                bucket.label.clone(),
                format!("{}% of turns", bucket.share_pct),
            )
        })
        .unwrap_or_else(|| ("—".to_string(), "No peak hour data".to_string()))
}

fn top_project_data(report: &Report) -> (String, String) {
    report
        .wrapped_story
        .top_project
        .as_ref()
        .map(|project| {
            (
                project.name.clone(),
                format!(
                    "{}% of output · {} sessions",
                    project.share_pct, project.session_count
                ),
            )
        })
        .unwrap_or_else(|| ("—".to_string(), "No project data".to_string()))
}

fn top_tool_data(report: &Report) -> (String, String) {
    report
        .wrapped_story
        .top_tool
        .as_ref()
        .map(|tool| {
            (
                tool.name.clone(),
                format!("{} calls this season", tool.count),
            )
        })
        .unwrap_or_else(|| ("—".to_string(), "No tool data".to_string()))
}

fn inflection_html(report: &Report) -> String {
    report
        .inflection
        .as_ref()
        .map(|inflection| {
            let class = if inflection.direction == "worsened" {
                "warn"
            } else {
                "good"
            };
            format!(
                r#"<div class="inflection-note {class}">{}</div>"#,
                escape_html(&inflection.summary),
            )
        })
        .unwrap_or_default()
}

fn biggest_session_content(report: &Report) -> String {
    let by_cost = report.wrapped_story.biggest_session_by_cost.as_ref();
    let by_tokens = report.wrapped_story.biggest_session_by_tokens.as_ref();

    match (by_cost, by_tokens) {
        (Some(cost), Some(tokens)) if cost.session_id == tokens.session_id => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{card}</div>"#,
            card = biggest_session_card(cost, "by cost + tokens", true, true),
        ),
        (Some(cost), Some(tokens)) => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{cost_card}{token_card}</div>"#,
            cost_card = biggest_session_card(cost, "by cost", true, false),
            token_card = biggest_session_card(tokens, "by tokens", false, true),
        ),
        (Some(cost), None) => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{card}</div>"#,
            card = biggest_session_card(cost, "by cost", true, true),
        ),
        (None, Some(tokens)) => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{card}</div>"#,
            card = biggest_session_card(tokens, "by tokens", true, true),
        ),
        (None, None) => {
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div><p class="slide-sub" style="color:#000">No session data</p>"#.to_string()
        }
    }
}

fn biggest_session_card(
    session: &SessionSummary,
    label: &str,
    show_cost: bool,
    show_tokens: bool,
) -> String {
    let date = session
        .timestamp_start
        .as_deref()
        .map(|value| &value[..value.len().min(10)])
        .unwrap_or("—");
    let preview = trim_text(session.first_prompt.as_deref().unwrap_or(""), 120);

    let mut metrics = Vec::new();
    if show_cost {
        metrics.push(format!(
            "Cost {cost}",
            cost = format_currency(session.cost_usd)
        ));
    }
    if show_tokens {
        metrics.push(format!(
            "Tokens {tokens}",
            tokens = format_tokens(session.total_tokens)
        ));
    }

    format!(
        r#"<article class="card">
        <div class="eyebrow">{label}</div>
        <h3>{project}</h3>
        <p>{date} · {metrics}</p>
        <p>{preview}</p>
      </article>"#,
        label = escape_html(label),
        project = escape_html(&session.project_name),
        date = escape_html(date),
        metrics = escape_html(&metrics.join(" · ")),
        preview = escape_html(&preview),
    )
}

#[cfg(test)]
mod tests {
    use super::total_cost_display;
    use crate::Report;

    fn report_with_cost(cost: f64) -> Report {
        Report {
            cost_analysis: crate::CostAnalysis {
                total_cost: cost,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn total_cost_display_never_shows_three_digit_cents() {
        // The old implementation split whole/frac separately, so rounding the
        // fractional part could produce "100" cents (e.g. "$1,000.100").
        // The fix rounds to total cents first, then splits.
        for cost in [1000.999, 1000.9999, 2500.998, 9999.9951] {
            let display = total_cost_display(&report_with_cost(cost));
            let dot = display.find('.').expect("should contain a dot");
            let after_dot = &display[dot + 1..];
            assert_eq!(
                after_dot.len(),
                2,
                "cost {cost} produced {display} — expected exactly 2 decimal places"
            );
        }
    }

    #[test]
    fn total_cost_display_normal_values() {
        assert_eq!(total_cost_display(&report_with_cost(1234.56)), "$1,234.56");
        assert_eq!(total_cost_display(&report_with_cost(999.99)), "$999.99");
        assert_eq!(total_cost_display(&report_with_cost(5.50)), "$5.50");
    }
}
