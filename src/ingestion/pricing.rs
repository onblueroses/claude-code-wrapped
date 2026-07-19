use super::time::epoch_datetime;
use super::types::TokenFacts;
use ccwrapped::PricingRegistryRecordMetadata;
use chrono::NaiveDate;

pub(super) const REGISTRY_VERSION: &str = "anthropic-api-2026-07-19";
pub(super) const REGISTRY_CITATION: &str =
    "https://platform.claude.com/docs/en/about-claude/pricing";
pub(super) const REGISTRY_ACCESS_DATE: &str = "2026-07-19";
pub(super) const SELECTION_POLICY: &str = "pricing/exact-provider-model-interval-modifier/v1";

#[derive(Debug, Clone, Copy)]
struct PriceRecord {
    canonical_model: &'static str,
    aliases: &'static [&'static str],
    effective_start: Option<(i32, u32, u32)>,
    effective_end: Option<(i32, u32, u32)>,
    input: u64,
    output: u64,
    cache_write_5m: u64,
    cache_write_1h: u64,
    cache_read: u64,
}

#[derive(Debug, Clone, Default)]
pub(super) struct PriceResult {
    pub provider: String,
    pub canonical_model: Option<String>,
    pub pricing_key: Option<String>,
    pub cost_pico_usd: u128,
    pub priceable_tokens: u128,
    pub priced_tokens: u128,
    pub unpriced_tokens: u128,
    pub request_priced: bool,
    pub usage_complete: bool,
    pub cache_ttl_composition_invalid: bool,
}

const FABLE_5: &[&str] = &["claude-fable-5"];
const MYTHOS_5: &[&str] = &["claude-mythos-5"];
const OPUS_4_8: &[&str] = &["claude-opus-4-8"];
const OPUS_4_7: &[&str] = &["claude-opus-4-7"];
const OPUS_4_6: &[&str] = &["claude-opus-4-6"];
const OPUS_4_5: &[&str] = &["claude-opus-4-5", "claude-opus-4-5-20251101"];
const OPUS_4_1: &[&str] = &["claude-opus-4-1", "claude-opus-4-1-20250805"];
const OPUS_4: &[&str] = &["claude-opus-4-20250514"];
const SONNET_5: &[&str] = &["claude-sonnet-5"];
const SONNET_4_6: &[&str] = &["claude-sonnet-4-6"];
const SONNET_4_5: &[&str] = &["claude-sonnet-4-5", "claude-sonnet-4-5-20250929"];
const SONNET_4: &[&str] = &["claude-sonnet-4-20250514"];
const HAIKU_4_5: &[&str] = &["claude-haiku-4-5", "claude-haiku-4-5-20251001"];
const HAIKU_3_5: &[&str] = &["claude-3-5-haiku-latest", "claude-3-5-haiku-20241022"];

const PRICE_RECORDS: &[PriceRecord] = &[
    price(
        "claude-fable-5",
        FABLE_5,
        Some((2026, 6, 9)),
        Some((2026, 6, 11)),
        10_000_000,
        50_000_000,
    ),
    price(
        "claude-fable-5",
        FABLE_5,
        Some((2026, 7, 1)),
        None,
        10_000_000,
        50_000_000,
    ),
    price(
        "claude-mythos-5",
        MYTHOS_5,
        Some((2026, 6, 9)),
        Some((2026, 6, 11)),
        10_000_000,
        50_000_000,
    ),
    price(
        "claude-mythos-5",
        MYTHOS_5,
        Some((2026, 7, 1)),
        None,
        10_000_000,
        50_000_000,
    ),
    price(
        "claude-opus-4-8",
        OPUS_4_8,
        Some((2026, 5, 28)),
        None,
        5_000_000,
        25_000_000,
    ),
    price(
        "claude-opus-4-7",
        OPUS_4_7,
        Some((2026, 4, 16)),
        None,
        5_000_000,
        25_000_000,
    ),
    price(
        "claude-opus-4-6",
        OPUS_4_6,
        Some((2026, 2, 5)),
        None,
        5_000_000,
        25_000_000,
    ),
    price(
        "claude-opus-4-5",
        OPUS_4_5,
        Some((2025, 11, 24)),
        None,
        5_000_000,
        25_000_000,
    ),
    price(
        "claude-opus-4-1",
        OPUS_4_1,
        Some((2025, 8, 5)),
        Some((2026, 8, 4)),
        15_000_000,
        75_000_000,
    ),
    price(
        "claude-opus-4",
        OPUS_4,
        Some((2025, 5, 22)),
        Some((2026, 6, 14)),
        15_000_000,
        75_000_000,
    ),
    price(
        "claude-sonnet-5",
        SONNET_5,
        Some((2026, 6, 30)),
        Some((2026, 8, 31)),
        2_000_000,
        10_000_000,
    ),
    price(
        "claude-sonnet-5",
        SONNET_5,
        Some((2026, 9, 1)),
        None,
        3_000_000,
        15_000_000,
    ),
    price(
        "claude-sonnet-4-6",
        SONNET_4_6,
        Some((2026, 2, 17)),
        None,
        3_000_000,
        15_000_000,
    ),
    price(
        "claude-sonnet-4-5",
        SONNET_4_5,
        Some((2025, 9, 29)),
        None,
        3_000_000,
        15_000_000,
    ),
    price(
        "claude-sonnet-4",
        SONNET_4,
        Some((2025, 5, 22)),
        Some((2026, 6, 14)),
        3_000_000,
        15_000_000,
    ),
    price(
        "claude-haiku-4-5",
        HAIKU_4_5,
        Some((2025, 10, 15)),
        None,
        1_000_000,
        5_000_000,
    ),
    price(
        "claude-haiku-3-5",
        HAIKU_3_5,
        Some((2024, 11, 4)),
        Some((2026, 2, 18)),
        800_000,
        4_000_000,
    ),
];

const fn price(
    canonical_model: &'static str,
    aliases: &'static [&'static str],
    effective_start: Option<(i32, u32, u32)>,
    effective_end: Option<(i32, u32, u32)>,
    input: u64,
    output: u64,
) -> PriceRecord {
    PriceRecord {
        canonical_model,
        aliases,
        effective_start,
        effective_end,
        input,
        output,
        cache_write_5m: input.saturating_mul(5) / 4,
        cache_write_1h: input.saturating_mul(2),
        cache_read: input / 10,
    }
}

pub(super) fn registry_records() -> Vec<PricingRegistryRecordMetadata> {
    let mut records = PRICE_RECORDS
        .iter()
        .map(|record| PricingRegistryRecordMetadata {
            provider: "anthropic-api".to_string(),
            canonical_model: record.canonical_model.to_string(),
            aliases: record
                .aliases
                .iter()
                .map(|alias| (*alias).to_string())
                .collect(),
            effective_start: format_date(record.effective_start),
            effective_end: format_date(record.effective_end),
            modifier: "standard".to_string(),
            input_pico_usd_per_token: record.input,
            output_pico_usd_per_token: record.output,
            cache_read_pico_usd_per_token: record.cache_read,
            cache_write_5m_pico_usd_per_token: record.cache_write_5m,
            cache_write_1h_pico_usd_per_token: record.cache_write_1h,
            citation: REGISTRY_CITATION.to_string(),
            access_date: REGISTRY_ACCESS_DATE.to_string(),
        })
        .collect::<Vec<_>>();
    records.sort_by(|left, right| {
        left.provider
            .cmp(&right.provider)
            .then_with(|| left.canonical_model.cmp(&right.canonical_model))
            .then_with(|| left.effective_start.cmp(&right.effective_start))
            .then_with(|| left.effective_end.cmp(&right.effective_end))
            .then_with(|| left.modifier.cmp(&right.modifier))
    });
    records
}

fn format_date(date: Option<(i32, u32, u32)>) -> Option<String> {
    date.map(|(year, month, day)| format!("{year:04}-{month:02}-{day:02}"))
}

pub(super) fn price_usage(
    raw_model: &str,
    epoch_nanos: i128,
    modifier: &str,
    tokens: &TokenFacts,
) -> PriceResult {
    let (provider, model) = provider_and_model(raw_model);
    let observed_tokens = token_sum(tokens);
    if provider != "anthropic-api" {
        return PriceResult {
            provider: provider.to_string(),
            priceable_tokens: observed_tokens,
            unpriced_tokens: observed_tokens,
            usage_complete: usage_complete(tokens),
            ..PriceResult::default()
        };
    }
    let canonical_model = exact_identity(model).map(str::to_string);
    let Some(record) = exact_record(model, epoch_nanos) else {
        return PriceResult {
            provider: provider.to_string(),
            canonical_model,
            priceable_tokens: observed_tokens,
            unpriced_tokens: observed_tokens,
            usage_complete: usage_complete(tokens),
            ..PriceResult::default()
        };
    };
    if modifier != "standard" {
        return PriceResult {
            provider: provider.to_string(),
            canonical_model: Some(record.canonical_model.to_string()),
            priceable_tokens: observed_tokens,
            unpriced_tokens: observed_tokens,
            usage_complete: usage_complete(tokens),
            ..PriceResult::default()
        };
    }

    let mut result = PriceResult {
        provider: provider.to_string(),
        canonical_model: Some(record.canonical_model.to_string()),
        pricing_key: Some(pricing_key(record)),
        priceable_tokens: observed_tokens,
        usage_complete: usage_complete(tokens),
        ..PriceResult::default()
    };
    add_priced(tokens.input, record.input, &mut result);
    add_priced(tokens.output, record.output, &mut result);
    add_priced(tokens.cache_read, record.cache_read, &mut result);

    if let Some(cache_total) = tokens.cache_creation {
        let cache_5m = tokens.cache_creation_5m.unwrap_or(0);
        let cache_1h = tokens.cache_creation_1h.unwrap_or(0);
        let classified = cache_5m
            .checked_add(cache_1h)
            .filter(|classified| *classified <= cache_total);
        if let Some(classified) = classified {
            add_priced(tokens.cache_creation_5m, record.cache_write_5m, &mut result);
            add_priced(tokens.cache_creation_1h, record.cache_write_1h, &mut result);
            result.unpriced_tokens = result
                .unpriced_tokens
                .saturating_add(u128::from(cache_total - classified));
        } else {
            result.unpriced_tokens = result
                .unpriced_tokens
                .saturating_add(u128::from(cache_total));
            result.usage_complete = false;
            result.cache_ttl_composition_invalid = true;
        }
    }
    result.request_priced = result.unpriced_tokens == 0 && result.usage_complete;
    result
}

pub(crate) fn canonical_model(raw_model: &str) -> Option<&'static str> {
    let (provider, model) = provider_and_model(raw_model);
    (provider == "anthropic-api")
        .then(|| exact_identity(model))
        .flatten()
}

#[allow(dead_code)] // The library compatibility reader consumes this; the binary does not.
pub(crate) fn legacy_api_equivalent_cost(
    raw_model: &str,
    timestamp: &str,
    usage: &ccwrapped::TokenUsage,
) -> f64 {
    let Some(epoch_nanos) = timestamp
        .parse::<chrono::DateTime<chrono::FixedOffset>>()
        .ok()
        .and_then(|timestamp| timestamp.timestamp_nanos_opt())
        .map(i128::from)
    else {
        return 0.0;
    };
    let tokens = TokenFacts {
        input: Some(usage.input_tokens),
        output: Some(usage.output_tokens),
        cache_creation: Some(usage.cache_creation_tokens),
        cache_read: Some(usage.cache_read_tokens),
        cache_creation_5m: None,
        cache_creation_1h: None,
    };
    pico_usd_to_dollars(price_usage(raw_model, epoch_nanos, "standard", &tokens).cost_pico_usd)
}

fn exact_identity(model: &str) -> Option<&'static str> {
    PRICE_RECORDS
        .iter()
        .find(|record| record.aliases.contains(&model))
        .map(|record| record.canonical_model)
}

fn exact_record(model: &str, epoch_nanos: i128) -> Option<&'static PriceRecord> {
    let date = epoch_datetime(epoch_nanos)?.date_naive();
    PRICE_RECORDS
        .iter()
        .find(|record| record.aliases.contains(&model) && price_applies(record, date))
}

fn provider_and_model(raw: &str) -> (&'static str, &str) {
    if let Some(model) = raw
        .strip_prefix("anthropic/")
        .or_else(|| raw.strip_prefix("claude/"))
    {
        ("anthropic-api", model)
    } else if raw.starts_with("us.anthropic.") || raw.starts_with("anthropic.claude-") {
        ("aws-bedrock", raw)
    } else if raw.contains('@') {
        ("google-cloud", raw)
    } else {
        ("anthropic-api", raw)
    }
}

fn price_applies(record: &PriceRecord, date: NaiveDate) -> bool {
    let starts = record
        .effective_start
        .and_then(|(year, month, day)| NaiveDate::from_ymd_opt(year, month, day))
        .is_none_or(|start| date >= start);
    let ends = record
        .effective_end
        .and_then(|(year, month, day)| NaiveDate::from_ymd_opt(year, month, day))
        .is_none_or(|end| date <= end);
    starts && ends
}

fn pricing_key(record: &PriceRecord) -> String {
    let start = record.effective_start.map_or_else(
        || "open".to_string(),
        |(year, month, day)| format!("{year:04}-{month:02}-{day:02}"),
    );
    let end = record.effective_end.map_or_else(
        || "open".to_string(),
        |(year, month, day)| format!("{year:04}-{month:02}-{day:02}"),
    );
    format!(
        "anthropic-api/{}/{start}/{end}/standard",
        record.canonical_model
    )
}

fn add_priced(value: Option<u64>, rate: u64, result: &mut PriceResult) {
    if let Some(tokens) = value {
        result.priced_tokens = result.priced_tokens.saturating_add(u128::from(tokens));
        result.cost_pico_usd = result
            .cost_pico_usd
            .saturating_add(u128::from(tokens).saturating_mul(u128::from(rate)));
    }
}

fn token_sum(tokens: &TokenFacts) -> u128 {
    [
        tokens.input,
        tokens.output,
        tokens.cache_creation,
        tokens.cache_read,
    ]
    .into_iter()
    .flatten()
    .fold(0u128, |total, value| {
        total.saturating_add(u128::from(value))
    })
}

fn usage_complete(tokens: &TokenFacts) -> bool {
    tokens.input.is_some()
        && tokens.output.is_some()
        && tokens.cache_creation.is_some()
        && tokens.cache_read.is_some()
}

pub(super) fn pico_usd_to_dollars(value: u128) -> f64 {
    value as f64 / 1_000_000_000_000.0
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use serde_json::Value;

    use super::*;

    #[test]
    fn embedded_registry_matches_the_dated_row_level_evidence_inventory() {
        let evidence: Value =
            serde_json::from_str(include_str!("../../docs/pricing-registry-2026-07-19.json"))
                .expect("pricing evidence must remain valid JSON");
        assert_eq!(evidence["registryVersion"], REGISTRY_VERSION);
        assert_eq!(evidence["accessDate"], REGISTRY_ACCESS_DATE);
        assert_eq!(evidence["selectionPolicy"], SELECTION_POLICY);

        let source_ids = evidence["sources"]
            .as_object()
            .expect("pricing evidence sources must be an object")
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>();
        let evidence_records = evidence["records"]
            .as_array()
            .expect("pricing evidence records must be an array");
        let compiled_records = registry_records();
        assert_eq!(
            evidence_records.len(),
            compiled_records.len(),
            "every compiled interval needs exactly one evidence row"
        );

        for compiled in compiled_records {
            let matching = evidence_records
                .iter()
                .filter(|candidate| {
                    candidate["provider"].as_str() == Some(compiled.provider.as_str())
                        && candidate["canonicalModel"].as_str()
                            == Some(compiled.canonical_model.as_str())
                        && candidate["effectiveStart"].as_str()
                            == compiled.effective_start.as_deref()
                        && candidate["effectiveEnd"].as_str() == compiled.effective_end.as_deref()
                        && candidate["modifier"].as_str() == Some(compiled.modifier.as_str())
                })
                .collect::<Vec<_>>();
            assert_eq!(
                matching.len(),
                1,
                "compiled interval must map to one evidence row: {} {:?} {:?}",
                compiled.canonical_model,
                compiled.effective_start,
                compiled.effective_end
            );
            let row = matching[0];
            assert_eq!(row["aliases"], serde_json::json!(compiled.aliases));
            assert_eq!(
                row["inputPicoUsdPerToken"].as_u64(),
                Some(compiled.input_pico_usd_per_token)
            );
            assert_eq!(
                row["outputPicoUsdPerToken"].as_u64(),
                Some(compiled.output_pico_usd_per_token)
            );
            assert_eq!(
                row["cacheReadPicoUsdPerToken"].as_u64(),
                Some(compiled.cache_read_pico_usd_per_token)
            );
            assert_eq!(
                row["cacheWrite5mPicoUsdPerToken"].as_u64(),
                Some(compiled.cache_write_5m_pico_usd_per_token)
            );
            assert_eq!(
                row["cacheWrite1hPicoUsdPerToken"].as_u64(),
                Some(compiled.cache_write_1h_pico_usd_per_token)
            );
            let refs = row["sourceRefs"]
                .as_array()
                .expect("every pricing evidence row needs sourceRefs");
            assert!(
                !refs.is_empty(),
                "every pricing evidence row needs at least one source reference"
            );
            for source_ref in refs {
                let source_id = source_ref
                    .as_str()
                    .expect("sourceRef must be a string")
                    .split_once('#')
                    .expect("sourceRef must include a source id and locator")
                    .0;
                assert!(
                    source_ids.contains(source_id),
                    "sourceRef points to an undeclared source: {source_id}"
                );
            }
        }
    }
}
