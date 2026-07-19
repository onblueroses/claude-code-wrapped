use crate::{InsightCard, InsightFact, Report};

pub(crate) fn cards(report: &Report, share_only: bool) -> impl Iterator<Item = &InsightCard> {
    report.insights.cards.iter().filter(move |card| {
        card.availability != "unavailable" && (!share_only || card.privacy_class == "share")
    })
}

pub(crate) fn context_line(card: &InsightCard) -> String {
    format!(
        "Insight · {} · {} · {} · samples {}/{} · availability {} · coverage {} · confidence {} · privacy {} · {} to {} ({})",
        card.id,
        card.class,
        card.method_id,
        card.sample_count,
        card.minimum_sample_count,
        card.availability,
        card.coverage,
        card.confidence,
        card.privacy_class,
        card.window.start,
        card.window.end,
        card.window.timezone,
    )
}

pub(crate) fn narrative_lines(card: &InsightCard) -> Vec<String> {
    let mut lines = vec![
        format!("Insight title · {} · {}", card.id, card.title),
        format!("Insight finding · {} · {}", card.id, card.finding),
    ];
    if let Some(action) = &card.action {
        lines.push(format!(
            "Insight experiment · {} · {}",
            card.id, action.experiment
        ));
        lines.extend(action.alternative_explanations.iter().enumerate().map(
            |(index, alternative)| {
                format!(
                    "Insight alternative · {} · {} · {}",
                    card.id,
                    index.saturating_add(1),
                    alternative
                )
            },
        ));
    }
    lines
}

pub(crate) fn fact_line(fact: &InsightFact) -> String {
    format!(
        "Insight fact · {} · {}={} {} · {} · samples {} · coverage {} · source {} · {} to {} ({})",
        fact.id,
        fact.metric_id,
        fact.value,
        fact.unit,
        fact.method_id,
        fact.sample_count,
        fact.coverage,
        fact.source,
        fact.window.start,
        fact.window.end,
        fact.window.timezone,
    )
}

pub(crate) fn family_line(family: &crate::InsightFamilyStatus) -> String {
    let limitations = if family.limitations.is_empty() {
        "none".to_string()
    } else {
        family.limitations.join(",")
    };
    let capabilities = if family.required_capabilities.is_empty() {
        "none".to_string()
    } else {
        family.required_capabilities.join(",")
    };
    format!(
        "Insight family · {}={} · capabilities {} · samples {}/{} · limitations {}",
        family.family,
        family.availability,
        capabilities,
        family.sample_count,
        family.minimum_sample_count,
        limitations,
    )
}

pub(crate) fn comparison_line(card: &InsightCard) -> Option<String> {
    card.comparison.as_ref().map(|comparison| {
        let relative = comparison
            .relative_delta_pct
            .map_or_else(|| "unavailable".to_string(), |value| format!("{value}%"));
        format!(
            "Insight comparison · {}={} · {}={} · absolute delta {} · relative delta {}",
            comparison.baseline_fact_id,
            comparison.baseline_value,
            comparison.current_fact_id,
            comparison.current_value,
            comparison.absolute_delta,
            relative,
        )
    })
}

pub(crate) fn limitations_line(card: &InsightCard) -> String {
    let limitations = if card.limitations.is_empty() {
        "none".to_string()
    } else {
        card.limitations.join(",")
    };
    format!("Insight limitations · {} · {limitations}", card.id)
}
