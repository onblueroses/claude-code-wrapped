use crate::{
    escape_html, format_currency, format_tokens, ranked_projects, trim_text, with_grouping, Report,
    SessionSummary,
};

pub fn slide_opening(report: &Report) -> String {
    let trust = crate::trust_projection(report, "standard")
        .lines()
        .into_iter()
        .map(|line| format!("<div>{}</div>", escape_html(&line)))
        .collect::<Vec<_>>()
        .join("");
    format!(
        r#"  <!-- ── 1. OPENING / ARCHETYPE ── -->
  <section id="opening" class="slide s-black opening-slide">
    <div class="slide-inner">
      <span class="wordmark">{experience} · {year}</span>
      <div class="archetype-title">{title}</div>
      <p class="hero-desc">{summary}</p>
      <div class="trust-summary">{trust}</div>
      <div class="hero-stats">{hero_stats}</div>
    </div>
  </section>"#,
        experience = escape_html(crate::experience_label(report)),
        year = report.year,
        title = escape_html(&report.wrapped_story.archetype.title),
        summary = escape_html(&report.wrapped_story.summary),
        trust = trust,
        hero_stats = hero_stats_html(report),
    )
}

pub fn slide_spend(report: &Report) -> String {
    let spend_note = format!(
        "{} · coverage {} · registry {}",
        report.canonical_metrics.cost.local_api_equivalent.method_id,
        report
            .canonical_metrics
            .cost
            .local_api_equivalent
            .availability,
        report.methodology.pricing_registry.version
    );
    format!(
        r#"  <!-- ── 2. API-EQUIVALENT ESTIMATE ── -->
  <section id="api-equivalent" class="slide s-green stat-slide">
    <div class="slide-inner">
      <div class="slide-label" style="color:rgba(0,0,0,0.45)">API-equivalent estimate</div>
      <div class="slide-hero" style="color:#000">{total_cost}</div>
      <p class="slide-sub" style="color:#000;opacity:0.55">{spend_note}</p>
    </div>
  </section>"#,
        total_cost = escape_html(&total_cost_display(report)),
        spend_note = escape_html(&spend_note),
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
    let cache = &report.canonical_metrics.cache;
    format!(
        r#"  <!-- ── 5. CACHE EVIDENCE ── -->
  <section id="cache-evidence" class="slide s-dark stat-slide">
    <div class="slide-inner">
      <div class="slide-label">Cache evidence · descriptive token shares</div>
      <div class="cache-evidence-hero" style="color:#7cf2c8">{read_share}</div>
      <div class="cache-meta">
        <div>
          <div class="cache-stat-val">{read_numerator}/{read_denominator}</div>
          <div class="cache-stat-lbl">Read numerator / denominator</div>
        </div>
        <div>
          <div class="cache-stat-val">{write_share}</div>
          <div class="cache-stat-lbl">Cache-write share</div>
        </div>
        <div>
          <div class="cache-stat-val">{compactions}</div>
          <div class="cache-stat-lbl">Direct compactions</div>
        </div>
      </div>
      <p class="slide-sub">{method}; no cause, grade, or monetary effect is inferred.</p>
    </div>
  </section>"#,
        read_share = escape_html(&crate::canonical_ratio_display(&cache.read_share)),
        read_numerator = cache.read_share.numerator,
        read_denominator = cache.read_share.denominator,
        write_share = escape_html(&crate::canonical_ratio_display(&cache.write_share)),
        compactions = cache.direct_compactions,
        method = escape_html(&cache.read_share.method_id),
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
      <div class="section-title">Daily active-time union</div>
      <div class="activity-chart">{bars}</div>
    </div>
  </section>"#,
        bars = activity_bars(report),
    )
}

pub fn slide_model_and_projects(report: &Report) -> String {
    format!(
        r#"  <!-- ── 9. MODEL REQUEST MIX + PROJECTS ── -->
  <section id="model-projects" class="slide s-black data-slide">
    <div class="slide-inner">
      <div class="data-grid-2">
        <div>
          <div class="section-label">Model request mix</div>
          <div class="section-title">Full request population</div>
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
          <div class="section-label">Largest sessions</div>
          <div class="section-title">Observed runs</div>
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
    let read_share = crate::canonical_ratio_display(&report.canonical_metrics.cache.read_share);
    let write_share = crate::canonical_ratio_display(&report.canonical_metrics.cache.write_share);
    format!(
        r#"  <!-- ── 11. PROMPT RATIO + CACHE EVIDENCE ── -->
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
          <div class="section-label">Cache evidence</div>
          <div class="section-title">Observed token shares</div>
          <div class="savings-row"><span class="s-muted">Cache-read share</span><span class="s-pos">{read_share}</span></div>
          <div class="savings-row"><span class="s-muted">Cache-write share</span><span class="s-neg">{write_share}</span></div>
        </div>
      </div>
    </div>
  </section>"#,
        human_pct = human_pct,
        human_count = report.wrapped_story.prompt_ratio.human,
        tool_count = report.wrapped_story.prompt_ratio.tool,
        tool_pct = tool_pct,
        read_share = escape_html(&read_share),
        write_share = escape_html(&write_share),
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

pub fn slide_insights(report: &Report) -> String {
    let families = report
        .insights
        .families
        .iter()
        .map(|family| {
            format!(
                r#"<div class="eyebrow">{}</div>"#,
                escape_html(&super::insights::family_line(family))
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let cards = super::insights::cards(report, false)
        .map(|card| {
            let facts = card
                .supporting_facts
                .iter()
                .map(|fact| {
                    format!(
                        "<div>{}</div>",
                        escape_html(&super::insights::fact_line(fact))
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            let action = card.action.as_ref().map_or(String::new(), |action| {
                let alternatives = action
                    .alternative_explanations
                    .iter()
                    .map(|alternative| {
                        format!(
                            "<div>Alternative · {}</div>",
                            escape_html(alternative)
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!(
                    "<p>Experiment · {}</p><div class=\"eyebrow\">{}</div>",
                    escape_html(&action.experiment),
                    alternatives,
                )
            });
            let comparison = super::insights::comparison_line(card).map_or(String::new(), |line| {
                format!("<div>{}</div>", escape_html(&line))
            });
            let narratives = super::insights::narrative_lines(card)
                .into_iter()
                .map(|line| format!("<div>{}</div>", escape_html(&line)))
                .collect::<Vec<_>>()
                .join("");
            format!(
                r#"<article class="card"><div class="eyebrow">{class}</div><h3>{title}</h3><p>{finding}</p><div class="eyebrow">{context}{narratives}</div><div class="eyebrow">{comparison}<div>{limitations}</div>{facts}</div>{action}</article>"#,
                class = escape_html(&card.class),
                title = escape_html(&card.title),
                finding = escape_html(&card.finding),
                context = escape_html(&super::insights::context_line(card)),
                narratives = narratives,
                comparison = comparison,
                limitations = escape_html(&super::insights::limitations_line(card)),
                facts = facts,
                action = action,
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        r#"  <!-- ── EXPLAINABLE INSIGHTS ── -->
  <section class="slide s-dark data-slide">
    <div class="slide-inner">
      <div class="section-label">Explainable insights</div>
      <div class="section-title">Evidence, limits, and next experiments</div>
      <div class="eyebrow">{families}</div>
      <div class="card-grid">{cards}</div>
    </div>
  </section>"#,
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

/// Keeps cents on 4-digit API-equivalent estimates for the slide hero.
fn total_cost_display(report: &Report) -> String {
    let Some(cost) = crate::canonical_local_cost(report) else {
        return "Unavailable".to_string();
    };
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
    let activity_days = &report.canonical_metrics.active_time.days;
    let max_active = activity_days
        .iter()
        .map(|day| day.active_seconds)
        .max()
        .unwrap_or(0);
    if activity_days.is_empty() {
        return r#"<div style="opacity:0.35;font-size:13px">No daily data available.</div>"#
            .to_string();
    }

    activity_days
        .iter()
        .map(|day| {
            let pct = if max_active > 0 {
                ((u128::from(day.active_seconds) * 100) / u128::from(max_active)) as u64
            } else {
                0
            };
            let label = day.date.get(5..).unwrap_or(&day.date);
            format!(
                r#"<div class="spark-col" title="{} · {}"><div class="spark-bar" style="height:{}%"></div><span class="spark-label">{}</span></div>"#,
                escape_html(&day.date),
                escape_html(&format!("{} active seconds", day.active_seconds)),
                pct,
                escape_html(label),
            )
        })
        .collect::<Vec<_>>()
        .join("")
}

fn model_rows(report: &Report) -> String {
    if !report.model_routing.available {
        return r#"<p style="opacity:0.55;font-size:13px">No direct request observations.</p>"#
            .to_string();
    }
    let rows = crate::model_request_mix_rows(&report.model_routing)
        .into_iter()
        .map(|row| {
            format!(
                r#"<div class="model-row"><div class="model-row-top"><strong>{}</strong><span>{}%</span></div><div class="bar-track"><div class="bar-fill" style="width:{}%"></div></div></div>"#,
                escape_html(row.label),
                row.share_pct,
                row.share_pct,
            )
        })
        .collect::<Vec<_>>()
        .join("");
    format!(
        r#"{rows}<p style="opacity:0.62;font-size:12px">{method} · {observations} requests<br>Local cost coverage: {coverage} · {unpriced} unpriced requests</p>"#,
        method = escape_html(&report.model_routing.method_id),
        observations = report.model_routing.observations,
        coverage = escape_html(&report.canonical_metrics.cost.coverage),
        unpriced = report.canonical_metrics.cost.unpriced_requests,
    )
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
                ((u128::from(project.output_tokens) * 100) / u128::from(max_project_tokens))
                    .min(100) as u64
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
            let timing = format!(
                "{}s active / {}s elapsed",
                session.active_seconds, session.elapsed_seconds
            );
            format!(
                r#"<div class="session-row"><div><div class="session-project">{}</div><div class="session-meta">{}</div></div><span class="token-badge">{}</span></div>"#,
                escape_html(&session.project_name),
                escape_html(&timing),
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
            let timing = format!(
                "{}s active / {}s elapsed",
                subagent.active_seconds, subagent.elapsed_seconds
            );
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
                escape_html(&timing),
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

fn biggest_session_content(report: &Report) -> String {
    let by_cost = report.wrapped_story.biggest_session_by_cost.as_ref();
    let by_tokens = report.wrapped_story.biggest_session_by_tokens.as_ref();

    match (by_cost, by_tokens) {
        (Some(cost), Some(tokens)) if cost.session_id == tokens.session_id => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{card}</div>"#,
            card = biggest_session_card(cost, "by source estimate + tokens", true, true),
        ),
        (Some(cost), Some(tokens)) => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{cost_card}{token_card}</div>"#,
            cost_card = biggest_session_card(cost, "by source estimate", true, false),
            token_card = biggest_session_card(tokens, "by tokens", false, true),
        ),
        (Some(cost), None) => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{card}</div>"#,
            card = biggest_session_card(cost, "by source estimate", true, true),
        ),
        (None, Some(tokens)) => format!(
            r#"<div class="slide-label" style="color:rgba(0,0,0,0.45)">Biggest session</div>
      <div class="card-grid">{card}</div>"#,
            card = biggest_session_card(tokens, "by tokens", false, true),
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
    let timing = format!(
        "{}s active / {}s elapsed",
        session.active_seconds, session.elapsed_seconds
    );
    let preview = trim_text(session.first_prompt.as_deref().unwrap_or(""), 120);

    let mut metrics = Vec::new();
    if show_cost {
        metrics.push(format!(
            "Source estimate {cost}",
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
        <p>{timing} · {metrics}</p>
        <p>{preview}</p>
      </article>"#,
        label = escape_html(label),
        project = escape_html(&session.project_name),
        timing = escape_html(&timing),
        metrics = escape_html(&metrics.join(" · ")),
        preview = escape_html(&preview),
    )
}

#[cfg(test)]
mod tests {
    use super::total_cost_display;
    use crate::Report;

    fn report_with_cost(cost: f64) -> Report {
        let mut report = Report {
            ..Default::default()
        };
        report
            .canonical_metrics
            .cost
            .local_api_equivalent
            .amount_usd = Some(cost);
        report
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
