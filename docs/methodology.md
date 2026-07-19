# Analytics methodology

Methodology version: 1

This document defines what `ccwrapped` may calculate and say. The report exposes the
method ID, version, source capability, selected period, timezone, sample size, coverage,
quality, and limitations for each displayed fact or claim.

## Trust vocabulary

Coverage is one of `complete`, `partial`, `empty`, or `indeterminate` for the selected
source snapshot and period. It does not mean account-wide completeness.

Quality is one of:

- `direct`: the selected source directly emitted the fact;
- `derived`: deterministic arithmetic over direct facts;
- `modeled`: a versioned model or pricing registry was applied;
- `descriptive`: a non-causal comparison or classification;
- `unavailable`: source capability or sufficient coverage is absent.

Confidence is `high`, `medium`, `low`, or `unavailable`, derived from source capability,
coverage, sample size, conflicts, and model assumptions. It is never a decorative score.

Every human output opens with or carries one shared trust projection: privacy profile,
report schema, selected period, timezone, completeness, local API-equivalent cost
provenance/coverage and registry method, and limitations. JSON exposes those same inputs
structurally. `complete` describes the selected source snapshot and period only; partial or
indeterminate histories are presented as `observed activity`, never as an account-complete
year in review.

## Reporting period and timezone

Method `period/local-calendar/v1` accepts an IANA timezone. The documented default is the
host IANA timezone when it can be resolved; otherwise UTC with a warning. The chosen name
and timezone-database version enter the deterministic report inputs.

UTC timestamps are converted once, then the same local instant drives year/day, weekday,
hour, streak, comparison, and labels. A selected year is `[local Jan 1 00:00, next local
Jan 1 00:00)`, converted to UTC. Ambiguous DST folds preserve the source instant; nonexistent
local wall times are never synthesized. When a timezone transition skips an entire nominal
local date, its boundary is the first real instant after that date and is deduplicated with
the next date's boundary, so the report emits no synthetic zero-length day. Leap days and
UTC/local year crossovers are normal calendar cases.

An event belongs to a period by its instant. An interval is clipped to the reporting
period and split at local-day boundaries before attribution.

## Sessions, elapsed span, and active time

`session/elapsed-span/v1` is last valid event instant minus first valid event instant after
stable ordering. It is descriptive and may include inactive resumptions. Zero/one-event
sessions have a zero elapsed span.

`activity/capped-interval-union/v1` estimates active time from ordered timestamped activity
events. Each adjacent pair contributes `[earlier, min(later, earlier + threshold))`; a
single event contributes no duration unless a source-native duration exists. The default
threshold is 5 minutes and is recorded/configurable from 1 through 1440 whole minutes.
For supported direct tool/API duration, the source timestamp is the half-open interval end;
duration is bounded at 86,400 seconds and the start is derived from that real instant.
Intervals are clipped to the period, split by local day, and unioned
across main agents and subagents before totals, so concurrency is not double-counted.
Canonical activity reports `seconds`, availability, raw interval count, the configured
threshold, partitioned main/subagent seconds, and inclusive versus partitioned dimensions.

Active-time efficiency families may divide tokens, requests, tool duration, errors, or
modeled cost by unioned active hours. They must state the threshold and cannot be described
as human productivity.

`session/distinct-observed/v1` counts distinct canonical session keys with at least one
event inside the queried reporting window. A session crossing a boundary can therefore
appear once in each independently queried adjacent period, but never more than once inside
either query. Daily session counts are distinct-per-day facts and are not summed to produce
a month/year session count; the wider window performs its own distinct count. Reports and
comparisons disclose this non-additive convention. Missing stable session correlation
lowers coverage and uses a source-local synthetic record group rather than merging by time.

## Tokens and caching

`tokens/canonical-sum/v1` sums the separately selected input, output, cache-read, and
cache-creation values after source-local dedup and authority selection. `total tokens` is
the sum of those four categories only when all participating categories are supported;
otherwise the report names the included categories. The 5-minute and 1-hour
cache-creation categories describe the known composition of the generic cache-creation
total and are never added to that total again. Every category reports unit `tokens`,
availability, contributing samples, overflow state, method ID, and explicit limitations.
An unavailable category says that no supported observation supplied it; a partial category
states the contributing/eligible sample counts; a saturated category states that its
`u64::MAX` projection is a lower bound. Wider accumulation prevents wrapping, and saturated
categories cannot produce exact-looking cache ratios or prices.

`cache/read-share/v1` is `cache_read / (input + cache_read)` when the denominator is
positive. `cache/write-share/v1` is `cache_creation / (input + cache_creation)` under the
same rule. Zero denominators yield unavailable, not 0%. Token ratios are usage facts.

The tool never derives a cache invalidation, reset, TTL, or cause from token counts alone.
When a direct compaction/cache event is absent, changes are described as `cache-read share
rose/fell` with the comparison window and sample size. Cache-share changes remain
descriptive only and cannot trigger a recommendation under the frozen three-rule
recommendation contract. Any future cache action requires its own frozen evidence rule,
direct supporting facts, minimum sample and coverage gates, and a reversible experiment.

Legacy report-v2 cache-grade, estimated-break, reason, and monetary-effect-shaped fields
remain only as neutral compatibility fields (`N/A`, zero, and empty collections). Standard
renderers consume the canonical shares and direct compaction count.

## Model identity and pricing

`model/registry-map/v1` retains the raw identifier and maps only exact documented aliases
to a canonical provider, family, version/date, mode/modifier, and pricing key. An unknown
model stays unknown; substring matching never assigns a price. Exact request-share model
identity is resolved independently of token presence and priceability. A request may
therefore contribute to an exact mapped request denominator while its output-token and
local-cost shares remain unavailable.

Cost values remain separate:

- `cost/source-estimate/v1`: a source-emitted estimate, named by interface;
- `cost/api-equivalent/v1`: canonical tokens multiplied by an embedded, dated pricing
  registry whose source URL, effective interval, provider, cache TTL, and supported pricing
  modifier are explicit;
- `cost/billing/v1`: available only from a future documented billing source;
- `cost/subscription-comparison/v1`: an optional labeled scenario, never spend or savings.

Locally computed cost uses exact model/pricing coverage. Unknown tokens contribute to an
`unpriced` count and never fall back to Opus/Sonnet/Haiku tiers. Mixed cost totals show the
priced token/request share. Current pricing changes are version-dependent; a report embeds
the registry version it used and remains reproducible offline.

The 5-minute and 1-hour cache-write components are priceable only when their checked sum
does not exceed the enclosing generic cache-creation total. An overflow or over-composed
observation leaves that generic total unpriced, marks the request incomplete, and emits
`W_PRICING_CACHE_TTL_COMPOSITION`. Production reconciliation requires priced plus unpriced
tokens to equal the canonical four-category priceable token total in every dimension.

The Phase 2 embedded registry is `anthropic-api-2026-07-19`, cites the official Anthropic
pricing page accessed 2026-07-19, and applies
`pricing/exact-provider-model-interval-modifier/v1`. Official platform release notes and
model-deprecation records, rechecked 2026-07-19, bound each first-party interval to model
availability: records begin on documented API availability, not merely the date embedded
in a pinned model ID, and retired records end on the last calendar day before retirement.
That distinction starts Opus 4 and Sonnet 4 on 2025-05-22 and Haiku 3.5 on 2024-11-04.
Fable 5 and Mythos 5 use separate 2026-06-09–11 and 2026-07-01–open intervals so the
documented June 12–30 suspension remains unpriced. Claude Sonnet 5 begins 2026-06-30,
uses the evidenced introductory price through 2026-08-31, and uses the succeeding price
from 2026-09-01. Exact `anthropic/` and `claude/` first-party prefixes are normalized
before lookup. AWS Bedrock, Google Cloud, future identifiers, unknown cache TTLs, and
unsupported modifiers remain unpriced unless their own exact registry record exists.

[`pricing-registry-2026-07-19.json`](pricing-registry-2026-07-19.json) is the dated
row-level evidence inventory. Every row names its exact aliases, inclusive interval,
modifier, integer rates, and source locators. A unit parity gate compares every compiled
record to that file, so a runtime pricing change requires an explicit evidence update.

The direct OTel adapter retains the documented request/metric `speed` attribute for pricing
selection. `normal` or an absent metric speed selects `standard`; `fast` remains explicitly
unpriced because this embedded inventory has no fast-mode record. Transcript v1 exposes no
documented request-modifier contract, so its compatibility projection uses the standard
record and must not be interpreted as modifier-aware billing.

`methodology.pricingRegistry.records` pins the complete sorted embedded inventory in every
report. Each record includes provider, canonical model, exact raw aliases, inclusive
effective bounds, modifier, citation/access date, and integer pico-USD-per-token rates for
input, output, cache read, 5-minute cache write, and 1-hour cache write. Registry
construction uses those integer fixed-point constants directly; no floating-point
configuration value participates in price selection or multiplication.

Phase 2 supports only the embedded registry. It exposes no pricing-override flag, parser,
or dormant file-loading surface.

No value is called `actual cost`, `bill`, or `savings` unless its source supports that
meaning. Source and local estimates use `API-equivalent estimate` language.

Canonical cost values report unit `USD`, source/interface or registry, sample count,
availability/quality, limitations, priced and unpriced request/token coverage, and token
counter overflow state. Source-recorded, local API-equivalent, and billing-authoritative
domains never form a mixed grand total.

The frozen legacy surfaces keep the same separation. `costAnalysis.totalCost`, daily costs,
and model costs are one local API-equivalent projection from the embedded registry; session
`costUsd` is source-recorded only. Unknown or out-of-range model observations contribute zero
to the local subtotal and remain explicitly unpriced in canonical coverage. The two legacy
domains are never substituted or added together.

The frozen public cache analyzer functions are neutral compatibility adapters because their
inputs cannot prove eligibility denominators, invalidations, causal breaks, or monetary
savings. They retain directly observed token totals but return no grade, ratio, savings,
breakpoint, or inflection claim. Canonical cache shares and direct compaction counts are the
supported analytical surface.

## Executable reconciliation

`MetricReconciliation` checks every token category across mutually exclusive day, model,
project, and session-plus-unattributed partitions. It checks partitioned active nanoseconds
across day, model, project, and session dimensions before display rounding. Exact fixed-point
source-recorded cost, local API-equivalent cost, priced observations/requests, and unpriced
tokens/requests each reconcile across the same mutually exclusive dimensions; billing remains
unavailable when no authoritative source exists. Inclusive active-time projections may overlap
and are explicitly non-additive.

A failing reconciliation cannot serialize as an authoritative report. Production returns
`E_METRIC_RECONCILIATION` with path-free remediation, and a deliberate perturbed-projection
unit test exercises the same runtime gate.

## Comparisons and trends

The legacy `inflection` compatibility field remains null, so no UTC-derived comparison
label can bypass the selected-zone contract. The typed `ccwrapped.insights/v1` collection
contains the selected-zone comparison proof objects described below.

Comparison method `comparison/adjacent-equal-window/v1` compares the selected window with
the immediately preceding equal local-calendar window. It reports both windows, absolute
delta, relative delta only when the baseline is positive, sample sizes, coverage, and
source compatibility. An active date has either a canonical direct request/message usage
observation—including an explicit all-zero tuple—or a positive canonical aggregate
primary-token metric. A zero aggregate point proves coverage, not activity. Each window
requires at least seven active dates. Explicit all-zero direct request/message observations
therefore support an exact-zero baseline through the ordinary gate. Current v1 adapters do
not carry an exhaustive producer-coverage declaration that can waive the gate: a clean file
scan and an available token capability describe accepted input, not the absence of missing
time intervals. Under the pinned `(start,end]` OTel model, an accepted interval ending one
nanosecond before the next local boundary also omits the date's opening instant and cannot
prove a complete local day. Missing observations, aggregate zero intervals, transcript
retention, mixed source signatures, partial telemetry, or fewer than seven active dates keep
the comparison unavailable. The proof serializes both active-date counts and both equal
coverage signatures. Incomparable coverage produces unavailable.

`trend/median-halves/v1` uses the most recent 28 exact observed active-date output-token
points, keeps the largest even suffix, and requires at least eight points. It compares equal
chronological halves. `rose` or `fell` requires an absolute change of at least 100 tokens
and, for a positive earlier median, at least 10%; otherwise the result is `stable`.
Relative change remains absent for a zero earlier median. Its serialized sample count is
the number of daily points, never the number of underlying observations. The proof records
both median values, each half's point count, the total point count, half size, first and last
selected-zone local dates, exact direction threshold, and derived direction.

Anomaly method `anomaly/median-mad/v1` uses median and median absolute deviation over a
declared baseline and reports the raw value, robust score, baseline, and minimum sample.
When MAD is zero, only a documented absolute/practical threshold can mark an outlier.
Anomaly means unusual within observed data, not bad or causal.

## Reliability and tool behavior

Error rate, retry rate, latency, and success rate are available only from direct telemetry
capabilities. `reliability/event-rate/v1` names the event definition; for example Claude
Code `api_error` represents errors after retry exhaustion, so it is not an attempt-level
failure rate. `direct_terminal_outcomes` governs the request-plus-terminal-error denominator;
`retry_evidence` independently governs completed requests carrying attempt evidence. A parsed
unsupported event family does not weaken either denominator, while a malformed/skipped record
makes an otherwise observed denominator partial and suppresses its recommendation. Missing
telemetry yields unavailable rather than 0%.

Tool occurrence comes only from transcript assistant `tool_use` blocks. A direct OTel
`tool_result` contributes result/status/latency evidence but does not also increment the
transcript occurrence count. Edit decisions likewise remain their own direct capability.
Per-tool comparisons include each available capability and its coverage. Tool names are
allowlisted/categorized; arguments, results, and error bodies are content and are redacted
by default. Result occurrence, status, latency, and edit-decision capabilities remain
independent, so an unavailable sub-capability cannot become a zero or add its limitation to
an otherwise exact card from another insight family.
Direct tool duration accepts finite values from zero through 86,400,000 milliseconds
(24 hours), matching the active-time ceiling. A negative, non-finite, or larger value is
invalid evidence: the result occurrence survives, latency becomes partial or unavailable,
and no rounded non-finite value reaches a fact. Tool presentation ranks at most ten
classified tools. Recommendation candidacy is evaluated over the complete classified-tool
population before presentation is capped. When the winning candidate falls below displayed
rank ten, a dedicated factual trigger card occupies one of the ten tool-card slots and the
ordinary ranked presentation uses the remaining nine slots.

## Routing and concentration

`routing/model-share/v1` reports canonical request/token/cost shares by exact model
mapping with unknown coverage. It does not label a request as wrongly routed without a
direct task/intent label. Request share uses the contributing canonical request/message
events and, for direct telemetry, each source's `api_request` capability. Complete direct
request evidence is labeled `complete-canonical-usage`; transcript evidence is
`indeterminate-retained-history`; and uncertain request evidence is
`partial-canonical-usage`. Only the first request-evidence state can trigger the routing
experiment. Output-token share separately uses canonical token evidence, and local-cost
share uses the fixed-point cost proof; neither can weaken an otherwise exact request share.
Request, output-token, and local-cost denominators cover the complete mapped population
before the ten-model presentation cap. Omitted mapped models are preserved as deterministic
`other-mapped` tail facts, while unknown-model observations remain a separate bucket. The
visible named, tail, and unknown shares therefore reconcile to 100% without allowing rank
truncation to change a denominator.

`concentration/project-hhi/v1` computes Herfindahl-Hirschman concentration over privacy-safe
project aliases for a named weight (requests, tokens, active time, or estimate). It reports
project count, unknown share, HHI, and the top-share facts. Labels such as concentrated or
diverse are descriptive thresholds defined in the method catalogue, not personality facts.

## Insights and recommendations

Every insight is a small proof object:

```text
claim + descriptive/entertainment class
input fact IDs
comparison/baseline and method ID
window + timezone + sample size + coverage
confidence + limitations
optional reversible experiment
```

For an explicit calendar-year report, card windows use that selected-zone half-open year
boundary unless a method declares a narrower window. The public all-period compatibility
reader has no calendar-year boundary, so period-level cards use the first through the day
after the last observed selected-zone local date. Empty evidence emits family status only
and therefore never fabricates window bounds.

Required insight families are comparisons, trends, active-time efficiency, reliability,
tool behavior, model routing, project concentration, anomalies, and recommendations.
Entertainment archetypes are visibly labeled and cannot alter factual metrics.

The engine emits no more than 32 cards, 16 supporting facts per card, 10 ranked entries per
family, three anomalies, and one recommendation per rule. Renderer order is priority,
family, then stable card ID; supporting facts sort by stable fact ID. Every family serializes an
available/partial/unavailable status even when it has no card.

Before serialization, `E_INSIGHT_RECONCILIATION` fail-closes on an unknown report/family/card,
an invalid or shifted half-open window, unsorted or duplicate proof identity, non-finite or
over-precision number, family-capability drift, and method-specific arithmetic/reference
drift. The production predicate independently recomputes comparison/trend deltas,
trend point/half counts, local-date bounds, direction threshold, direction/finding,
active-hour rates, reliability and tool rates, tool latency ordering and edit acceptance,
routing share closure, project coverage/HHI bounds, anomaly score/guards, recommendation
thresholds/references, and entertainment gates. Mutation fixtures cover every family.

The three recommendation rules are exact:

- terminal API errors: at least 10 direct terminal outcomes and an error rate of at least
  10%;
- tool-result errors: the highest-rate classified tool with at least 10 direct results and
  a failure rate of at least 20%;
- model-routing experiment: at least 20 canonical observations, one exact mapped model at
  or above 80% request share, and unknown-model request share at or below 10%.

Each rule cites its triggering facts and proposes a bounded reversible comparison. Fixed
alternative explanations remain attached. No other recommendation enters standard report
construction. The recommendation family is unavailable when none of its three rules has
an exact denominator at its minimum sample; it is available with
`recommendation-no-rule-threshold-met` when exact rules were evaluated but no threshold
was crossed.

`entertainment/sample-gated/v1` requires at least 20 canonical request/message observations
and five observed active dates. It chooses The Orchestrator at a subagent share of at least
30%; otherwise The Toolsmith when every observation carries a classified tool occurrence;
otherwise The Specialist when project-output HHI is at least 2,500; otherwise The Explorer.
This tie order is total and deterministic. Cache cartographer additionally requires an
available canonical cache-read share, and Observed Momentum additionally requires an
available median-halves trend. Below any gate the corresponding title says the entertainment
label is unavailable or that there is not enough observed activity.

A recommendation is emitted only when its fixture-tested rule has direct supporting facts,
minimum sample/coverage, and a reversible action. It says what was observed and proposes
an experiment. It does not assert throttling, cache resets, wasted spend, causal savings,
or productivity gains without direct evidence.

## Retention and completeness

Claude Code transcripts are plaintext internal artifacts, cleaned up after the configured
retention period (30 days by default in current documentation), and may be disabled. A
requested year is therefore not assumed complete. Coverage uses earliest/latest observed
instants, observed-day span, source diagnostics, explicit gaps, retention hints when
available, and deletion/import history. It remains indeterminate when history may have
expired before observation.

The report cannot see server-side usage, other devices, or sources not selected. Built-in
usage/analytics and billing surfaces answer different questions; local observation is not
reconciled to them unless a future documented adapter supplies comparable facts.

## Output agreement

All output profiles consume `ccwrapped.report/v2`. Common metric IDs have identical values,
units, periods, and method versions. Terminal/HTML/Markdown use `standard`, the card uses
`share`, exact-path diagnostics use `private`, and archive files use `private-content`;
every surface labels its profile. Terminal/HTML/Markdown show the shared trust summary
before detailed analytics; JSON includes the complete safe proof objects. The archive adds
explicit private content but must not recompute metrics. The share card accepts an
aggregate-only typed projection and labels estimates. Its proof appendix includes every family status and every
title, finding, context, comparison, limitation, and supporting-fact line from every
`privacyClass: share` card; it applies no family/card/fact truncation. Narrative/action lines
come from one shared formatter, and any future share-safe action would retain its experiment
and ordered alternatives. Current recommendation cards and other standard-only facts,
including the top-project alias, remain excluded structurally.

`F030` constructs each common `FACT` line from JSON
method/value/unit/period/sample/overflow/limitation metadata and requires the exact line in
terminal, HTML, Markdown, and the card. The shared projection includes every token
category plus total, active time, local API-equivalent cost, and both cache shares.
Unavailable, partial, explicit-zero, and saturated fixtures use the same equality oracle.
Privacy tests scan decoded and escaped output for hostile synthetic identifiers/content,
not only literal source text.

## Scale and incremental operation

The selected default is a rebuildable SQLite v9 store. Exact inventory matches return the
compressed cached report without reading source content; small prompt-only appends patch a
bounded analysis checkpoint; other changed files reuse privacy-safe normalized payloads and
parse only changed/new content. A successful complete scan transactionally reconciles
deletions, replacements, truncations, and renames. An inaccessible root does not publish
deletions. When one nested transcript branch becomes inaccessible but the unchanged readable
subset proves the cached source identity, the report retains last-known rows, marks coverage
partial, and publishes no store mutation. Every readable path key, source key, and native
size/mtime/ctime snapshot must still match the store; a readable append, replacement,
truncation, or rename during the partial scan returns `E_INCREMENTAL_STORE` instead of stale
facts.

`--rebuild-store` uses a new private sibling database throughout discovery, ingestion, and
report publication. Only a complete publishable scan promotes that database over the selected
store; every error and partial-coverage path removes staging and preserves the prior database.
Legacy formats 1–8 use the same staged publication rule, and a crash-releasing per-store lease
serializes every writer through commit or abort.

Production auto-selects 12 workers, clamped to the logical CPUs available to the process.
Parsing uses one-file batches and at most two queued results per worker; deterministic ordered
merge plus scoped canonical/alias projections keep scheduling out of aliases, diagnostics,
facts, and JSON. Explicit benchmark overrides replay the full scaling curve, while
private-content archive parsing remains single-threaded and bypasses the store.

The pinned historical synthetic benchmark names JSON parsing CPU as the bottleneck for its
exact binary. Its 3,755,841,597-byte no-store workload completed in 18.223 / 22.689-second
medians at 74.06% / 65.76% of 12 selected logical CPUs, with 0.96% / 3.62% wall-time CoV.
Its paired 8/12/15-worker curve placed 12 workers 0.54% ahead of 15; 8 workers were 6.24%
slower. Later correctness, privacy, persistence, and pricing changes are outside that timing
snapshot. Performance is informational rather than a source-current release gate. See
[`benchmarks/phase5.md`](benchmarks/phase5.md) and the machine-readable
[`benchmarks/phase5-record.json`](benchmarks/phase5-record.json) for the branch rule,
complete samples, resource counters, rejected noisy series, and verification commands.
