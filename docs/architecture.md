# Architecture contract

Contract version: 1

Status: frozen for Phase 0. Later changes require a recorded decision, a compatibility
assessment, and a rubric expansion only when correctness, security, privacy, data loss,
or a stated public contract requires it.

## Product boundary

`ccwrapped` is a local, offline analytics tool. It observes retained local artifacts; it
does not claim billing authority, complete account usage, causal productivity impact, or
activity that the selected sources cannot represent. A report describes **observed
activity** unless coverage evidence supports a stronger period-completeness statement.

The implemented runtime data flow is:

```text
transcript snapshots       local OTLP/JSONL artifacts
        |                            |
        +----------- versioned source adapters -----------+
                                     |
                        normalized privacy-safe events
                                     |
                  validation + diagnostics + deterministic dedup
                                     |
                     optional incremental local event store
                                     |
                  canonical metric engine + method catalogue
                                     |
                    explainable insight/narrative engine
                                     |
                 versioned report shared by every renderer
```

No analyzer or renderer may parse a source artifact. Every renderer consumes one report
model. Source adapters preserve observations; the authority policy selects canonical
facts. Selection and correlation never silently sum overlapping transcript and telemetry
observations.

## Source discovery contract

Discovery is deterministic and records a privacy-safe, kind-specific alias for every
accepted source. Transcript-root precedence is:

1. Each repeatable `--data-dir PATH`, in command-line order.
2. `$CLAUDE_CONFIG_DIR/projects` when `CLAUDE_CONFIG_DIR` is set and no explicit
   transcript root was supplied.
3. `$HOME/.claude/projects` as the documented platform default.

An explicit path can name a projects directory or a Claude configuration directory; the
resolver records which interpretation succeeded. Canonicalization happens before
duplicate-root detection. A missing explicit root is an error. A missing implicit root is
a warning unless no usable root remains. Inaccessible and partially readable roots are
indeterminate, not empty.

An implicit root that passes the precedence probe but fails during canonicalization,
metadata inspection, or identity validation remains the selected transcript root: the
resolver records its safe alias as partial and does not fall through to a lower-precedence
home root. Independently selected explicit telemetry may still produce observed facts with
that warning present. If no usable declared source survives, the implicit failure remains
fatal.

There is no implicit generic XDG search in contract version 1. Current Claude Code
documentation names `CLAUDE_CONFIG_DIR`; arbitrary XDG roots remain accessible through
`--data-dir` without claiming an undocumented default.

Standard diagnostics expose `source-1`, `source-2`, source kinds, and counts. Exact roots
and relative file paths require an explicit private diagnostic profile.

Telemetry enrichment is never discovered implicitly. Each repeatable `--otel-file PATH`
selects one regular, uncompressed Collector JSON/JSONL file in command-line order. This
selector does not change transcript-root precedence. A path repeated within one source
kind is canonicalized, warned about, and imported once; the same path may legitimately
participate as a transcript root and an OTel file only when it satisfies both explicit
contracts. Aliases are assigned independently as `transcript-1` and `otel-1` from
deterministic selector/default order.

Every explicit telemetry path must exist, be of the promised kind, canonicalize, and be
readable or the invocation fails before analysis. A file that becomes unreadable or
changes during import makes that source indeterminate and produces an actionable error;
it is never treated as an authoritative empty source. Exact roots and files remain private
diagnostics. Version 1 has no telemetry environment variable, directory glob, or network
receiver.

Local Git enrichment is intentionally outside the implemented version-1 runtime contract.
There is no `--git-repo` option and no implicit current-repository scan. A future opt-in
Git adapter must add its CLI, privacy classification, source aliasing, coverage, and
behavioral fixtures together before this document may claim commit or pull-request facts.

Transcript discovery snapshots every traversed directory as well as every selected file.
Directory identities must still match after traversal and after all discovered files are
ingested; file identities must match the opened handle and final handle/path metadata. A
nested addition, removal, replacement, or metadata change fails the selected source instead of
publishing a scan whose apparent completeness was invalidated during the invocation.
One invocation-owned budget admits at most 100,000 `read_dir` entries across all transcript
roots and is consumed before allocating each entry path. Per-source unique-directory and
unique-file ceilings remain separate, so wide irrelevant forests cannot multiply a local limit
across directories or selected roots.

## Adapter contracts

### Transcript adapter `claude-transcript/v1`

The adapter accepts UTF-8 JSON Lines in the documented Claude project layout. The format
is internal and version-dependent, so active raw structs stay private to the adapter. The
pre-Phase-1 public `readers::wire` DTOs remain only as documented legacy, passive parse types
for source compatibility; no production reader or analyzer uses them. Files are streamed line
by line with a configurable maximum line size. The adapter:

- recognizes assistant, user, progress, summary, and supported system variants;
- preserves main-agent, sidechain, parent, subagent, session, message, and request
  relationships when present;
- retains token categories independently;
- records source-recorded cost only as an estimate with source provenance;
- emits bounded structural diagnostics for unknown variants without retaining values;
- counts accepted, malformed, unsupported, filtered, redacted, duplicate, and skipped
  records separately;
- continues after a malformed line and surfaces file-open/read failures as diagnostics.

Content never enters the standard normalized stream. Content-enabled parsing uses a
separate ephemeral private path and is never written to the standard store.

### Telemetry adapter `claude-otel-otlp-json/v1`

The accepted wire contract is pinned to OpenTelemetry Collector Contrib `v0.148.0`
(tag commit `d3c47b3`), its file exporter module `v0.148.0`, Collector pdata `v1.54.0`,
and `go.opentelemetry.io/proto/slim/otlp` `v1.10.0`. The producer configuration uses the
file exporter with `format: json`, no `encoding`, and no `compression`; append versus
truncate does not alter the per-line wire shape. Each physical line is exactly one pdata
OTLP/JSON metrics or logs object rooted at `resourceMetrics` or `resourceLogs`.

The file format does not embed its producer version. Diagnostics therefore record expected
producer contract `otelcol-contrib/file/v0.148.0+pdata/v1.54.0+slim-otlp/v1.10.0` and
producer verification `unverified` unless explicit private provenance supplies it. Runtime
acceptance is structural: required pinned-shape paths and value kinds must match the pinned
fixtures; unknown fields are counted, and an incompatible required path/type is a bounded
unsupported shape rather than a partially guessed record. A file from another release is
supported only when it conforms to this exact wire contract; no release compatibility is
inferred from a filename or user assertion.

The adapter does not claim support for console text, Prometheus exposition, binary protobuf,
compressed files, arbitrary Loki exports, traces/profiles, or network OTLP ingestion.

This contract is intentionally narrow and inspectable. The collector file exporter is
alpha for metrics/logs and warns that field names are not stable, so every accepted shape
is fixture-tested and unknown shapes degrade to bounded diagnostics.

Deserialization is bounded before an OTLP object can allocate without limit. Defaults are
a 16 MiB physical line, Serde's recursion limit (which must not be disabled), 256 resource
groups, 256 scopes per resource, 100,000 records/points per export object, 128 attributes
per resource/scope/record/point, 64 KiB total decoded attribute text per record/point, and
1,000,000 distinct metric streams per invocation. Options may lower these values; raising
them requires explicit unsafe-large-input acknowledgement and still remains under
compile-time safety ceilings. Oversized lines are drained without buffering their tail;
objects that exceed structural/cardinality limits are rejected as a unit with bounded
key/type/count diagnostics and explicit partial coverage. Counters saturate safely.
Resource, scope, and local attributes are parsed once per owning entity. Record/point
normalization resolves local → scope → resource precedence through a three-layer borrowed view;
it never clones inherited attribute maps or identity material per record. Inherited-attribute
work is therefore proportional to parsed attribute bytes plus a constant number of lookups per
record/point, even at the 100,000-record ceiling.

Supported signals are:

- timestamped `claude_code.api_request`, `api_error`, `tool_result`, `tool_decision`, and
  user-prompt occurrence events, with content disabled or redacted;
- Claude Code cost, token, session, active-time, code-edit decision, lines-of-code,
  commit, and pull-request metric streams;
- resource/scope identity required to distinguish writers without exposing it in normal
  reports.

Unsupported events or metric names remain counted, not guessed.

For a metric stream, identity is resource identity + instrumentation scope + metric name
+ attributes + point kind/unit/temporality. The importer applies the OTLP single-writer
rule, preserves `StartTimeUnixNano` and `TimeUnixNano`, and handles:

- delta points as half-open accumulation windows `(start, end]`;
- cumulative points by monotonic difference within one unbroken writer sequence;
- a lower cumulative value or changed start time as a reset, never a negative delta;
- gaps as partial coverage;
- overlaps as a diagnostic and unresolved coverage unless exact duplicate identity
  permits deterministic deduplication;
- missing timestamps as unusable for canonical period attribution;
- aggregate points crossing a UTC day/year boundary as filtered partial evidence rather
  than assigning by export time or inventing prorating; under `(start,end]`, a point ending
  exactly at midnight crosses days while one starting at midnight can remain within that day.

Every pending metric point receives a physical-record disposition after temporality reduction.
Points rejected for overlap, out-of-order cumulative state, ambiguous reset, or number-kind change
increment global and owning-source `filteredRecords`; accepted, duplicate, unsupported, and
boundary-filtered paths retain their separate dispositions. Temporality warnings therefore cannot
silently reduce `classifiedRecords` below the number of processed points. A filtered metric
degrades its named analytical category only when its interval can affect the selected year; a
valid interval wholly outside that year remains counted but cannot make the selected period
analytically partial.

Timestamped `api_request` events are preferred for request-level token and estimated-cost
period attribution. Aggregate points remain canonical when their interval or explicit session/
model/agent context is disjoint from every request observation. A compatible request inside a
metric's half-open interval `(start, end]` supersedes that metric; temporal overlap without enough
shared context remains unresolved rather than summed.

Repeat-import identity is a hash of adapter version, source-file identity, line byte
range, signal kind, stream identity, start/end timestamps, and source-native event or
request identifier when present. Reimport is idempotent.

## Normalized event contract

`NormalizedEvent` is the stable internal boundary. It records:

- adapter and schema versions;
- run-local source alias plus privacy-safe source/file fingerprints;
- source-local record position and observation fingerprint;
- UTC timestamp and selected display-timezone conversion status;
- salted local correlation keys for session, message, request, parent, agent/subagent,
  and sidechain when present;
- raw safe model identifier and mapping-input status; the versioned exact registry derives
  canonical identity independently for model shares and pricing;
- independent input, output, cache-read, cache-creation, and cache-TTL tokens;
- distinct optional source estimate, local API-equivalent estimate, and billing fact;
- allowed tool category/name, status, latency, retry/error, edit decision, compaction,
  and activity facts;
- capability flags that preserve the difference between zero and unavailable;
- a provenance reference and redaction count.

The legacy numeric analyzer accumulator is a compatibility projection, not an availability
model. Canonical events retain `Option` presence through an internal analytical entry, and
the projection emits `analysis_input_tokens`, `analysis_output_tokens`,
`analysis_cache_creation_tokens`, `analysis_cache_read_tokens`, `analysis_usage_totals`,
`analysis_cost`, and `analysis_cache_health`. A category is `available` only when every
applicable canonical event observed it, `partial` when only some did, and `unavailable` when
none did. Derived cost and cache paths run only when their complete required inputs are
available; otherwise JSON keeps observed category sums and the consistently local
API-equivalent compatibility total/day/model cost subtotal beside the limitation while terminal, HTML, Markdown, card, story,
model-routing, anomaly, and recommendation paths suppress the stronger claim. Partial
average/median/peak cost fields retain their compatibility defaults. Explicit numeric zero
remains an observed value and is not collapsed with
absence. Non-negative token/count rollups use saturating integer arithmetic. If a canonical
token-category sum exceeds `u64`, the report clamps only the affected numeric projections,
emits `W_ANALYTICAL_TOKEN_SATURATED`, marks that category and aggregate usage/cost analysis
partial, and makes cache analysis unavailable; renderers therefore cannot present the clamp
as an exact derived claim. Aggregate token metrics are evaluated collectively by category instead of being
misread as incomplete per-request tuples. When direct request events and disjoint aggregate
metrics coexist, every direct event and the collective metric family must support a category
before that category is globally available; one family cannot conceal a missing category in the
other. Compatibility `costAnalysis` total/day/model values always use the embedded local
API-equivalent pricing registry, including when source-cost metrics coexist. Session `costUsd`
retains only source-recorded estimates. Those two legacy surfaces therefore never form a mixed
total; canonical cost exposes both domains separately and marks incomplete or conflicting
coverage explicitly.

Every rejected physical record whose kind or period cannot be established marks the analytical
projection uncertain. Malformed, oversized, unsupported, missing-timestamp, and upstream
dropped-attribute paths therefore turn otherwise complete token/cost capabilities into `partial`,
retain only observed sums, and suppress spend/cache-derived claims. Exact known-category
exclusions degrade only their affected categories. Proven non-Claude/content-bearing signals and
valid records wholly outside the selected period remain counted without degrading unrelated
selected-period capabilities.

The compatibility projection also separates value contribution from occurrence cardinality.
Aggregate token/cost metrics may contribute to daily/project/model token totals, but only
`AssistantUsage` and `OtelApiRequest` observations increment message counts, message-derived model
counts, message session counts, project occurrences, first/last message times, or message story
facts. Metric-only input therefore retains measured usage with zero message/session cardinality;
it does not create a synthetic session or a “Messages sent” claim.

Capability discovery uses one normalized-event pass. Each event produces one compact bit/boolean
observation that is merged into the global accumulator and its source accumulator through a
source-alias lookup. The 256-source and one-million-event ceilings therefore compose as
`O(events + sources)` state transfer rather than `O(events × sources)` rescanning; no
per-source event vectors are allocated.

Raw user content, response content, commands, arguments, tool results, raw error bodies,
email addresses, organization/account identifiers, paths, and unsalted stable identifiers
are absent from the standard type.

Unknown records retain only adapter version, source alias, line location, record-kind
token, sorted allowlisted key names, JSON value-type summaries, and a bounded byte count.

## Privacy profiles

The data model enforces four explicitly labeled projections:

| Profile | Purpose | Identifiers/paths | Prompt/message content |
| --- | --- | --- | --- |
| `standard` | terminal, JSON, HTML, Markdown | run-local aliases only | never |
| `share` | aggregate-only card DTO | none | never |
| `private` | explicit local diagnostics | exact local identifiers and paths allowed | never |
| `private-content` | explicit archive/content analysis | exact local fields allowed | opt-in, ephemeral or protected output only |

`private-content` requires an explicit option in the same invocation and cannot be
selected by configuration discovery alone. `--archive` remains an explicit
private-content operation, labels every archive file, and emits a labeled privacy warning
to stderr. Private diagnostics likewise label their stderr records. Share cards project
the standard report into a separate aggregate-only serializable DTO before rendering and
reject all identifier/content fields regardless of the active profile. The card template
cannot receive `Report`.

Every adapter applies a deny-by-default field allowlist before normalized storage. The
standard store holds a random per-store salt with restrictive permissions and only salted
correlation hashes. The salt and salted values never enter standard output, public aliases,
canonical sort keys, or the canonical payload. Public aliases derive from deterministic accepted
source/event order; adapters validate or roll back a rejected record before it can consume an
alias, so rebuilding a store or adding rejected input does not alter retained identities. Logs
never contain source values.

## Source authority policy `authority/v1`

All source observations remain distinguishable. Canonical selection follows this table:

| Fact | Preferred observation | Fallback | Never do |
| --- | --- | --- | --- |
| request tokens | timestamped OTel `api_request` with stable correlation | deduplicated transcript assistant usage | sum uncorrelated source totals |
| source cost estimate | timestamped OTel `api_request` estimate | transcript `costUSD` | call either value billed cost |
| local API-equivalent cost | versioned pricing registry over canonical tokens | unavailable/unpriced | map unknown model to a broad tier |
| billing cost | future documented billing adapter only | unavailable | infer from subscription or tokens |
| tools/status/latency | OTel tool events | transcript tool-name occurrence | infer success/latency from occurrence |
| errors/retries | OTel API/tool events | unavailable | treat missing telemetry as zero errors |
| activity events | union of correlated timestamped canonical events | transcript timestamps | double-count main/subagent concurrency |
| edit/commit/PR facts | direct supported OTel event/metric | unavailable | infer productivity from tokens or claim unimplemented Git enrichment |

Correlation requires a matching salted source-native request/message/session identity and
compatible timestamps. A sole available source family is canonical for the observations
it directly supports even when its coverage is partial or indeterminate; the report keeps
that limitation and never upgrades it to period completeness. Request/transcript-to-aggregate
authority is evaluated per metric interval and context: disjoint intervals or explicit differing
sessions/models/agents are both retained. Within OTel, a compatible request inside `(start, end]`
supersedes that aggregate metric. A compatible transcript observation inside the interval cannot
prove replacement and is unresolved, as is temporal overlap without enough shared context; neither
suppresses unrelated metrics.
When uncorrelated transcript and OTel families overlap the requested period, `authority/v1`
selects transcript tokens and reports unresolved OTel overlap rather than summing. An
uncorrelated OTel family may replace that overlapping transcript family only when OTel
request-event coverage proves it spans the selected period. The decision and coverage appear in
the report.

When a request identity repeats, correlation performs a bounded exact assignment over the
compatible transcript and OTel observations: maximum resolved cardinality first, minimum total
timestamp distance second, and canonical public observation order plus rank distance last. Both
sides are sorted by timestamp, public project/parent aliases, safe model and usage facts, and
stable source position only after those facts; private salted identifiers never decide a tie.
Subagent status is established before hashing an OTel session so transcript and OTel session
domains agree. Groups or aggregate assignment work beyond the published ceiling remain
transcript-authoritative, count OTel observations as
unresolved, and emit a stable limit warning. Aggregate-budget admission is computed before any
group is matched, so source enumeration cannot decide which identities receive the remaining
budget.

Within a source, deterministic dedup prefers, in order: strongest native identity and then the
richest valid usage observation. Equally rich conflicts select the lexicographically smallest
privacy-safe normalized fact vector. That vector includes analytical facts such as timestamps,
model mapping, usage, cost, tool, reliability, metric-interval, and redaction facts; it excludes
salted identities, aliases, physical file position, and record position. Exact fact equality may
retain either physical position because its analytical result is identical. A reused
request/message identity at a distinct instant is a distinct observation; exact-instant repeats
remain duplicates. Main/sidechain/subagent context is part of identity.

Across selected roots, a strong request or message identity uses the same richness and canonical-
fact preference. A fallback observation without such an identity collapses only when every
privacy-safe normalized fact and every salted identity-context field match; source aliases,
physical file/record positions, and observation-position keys remain excluded. Distinct outcomes,
latencies, retries, edit/compaction decisions, or agent/skill/plugin/MCP contexts therefore remain
distinct canonical observations.

## Incremental store contract

The measured Phase 5 decision selected a local SQLite store. The default path is
`$XDG_CACHE_HOME/ccwrapped/store-v1.sqlite3` when `XDG_CACHE_HOME` is absolute, otherwise
`$HOME/.cache/ccwrapped/store-v1.sqlite3`; `--store-path` selects an explicit location,
`--no-store` performs a fresh source scan without reading or writing it, and
`--rebuild-store` constructs a private sibling database and atomically replaces the selected
database only after a complete scan and report publication succeed. A failed or partial scan
removes its staging database and leaves the prior store authoritative. The on-disk format is
`ccwrapped.incremental-store/v9` with SQLite `user_version=9`.

The selected store provides:

- explicit schema and adapter versions with transactional migrations;
- privacy-safe normalized facts, bounded diagnostics, metric state, compressed analysis
  checkpoints, and compressed report JSON—never prompts, responses, commands, or paths in
  plaintext;
- salted path/source keys, source fingerprints, file size/mtime/ctime identity plus content
  digests, and idempotent imports;
- import generations that delete rows for files authoritatively removed, truncated,
  replaced, or renamed after a fully successful root scan;
- preservation of last-known rows when a root is inaccessible or only partially scanned,
  but only after every readable file still matches its stored salted key and complete
  snapshot; a concurrent readable-file change fails with `E_INCREMENTAL_STORE` instead of
  returning stale facts;
- staged migrations from genuine formats 1–8 that construct the current database in a private
  sibling and replace the legacy artifact only after complete report publication. Any failure
  leaves the legacy bytes authoritative. A crash-releasing per-store SQLite lease serializes
  writers, rebuilds, and migration publication. Integrity/digest failures produce explicit
  `E_INCREMENTAL_STORE` errors, and rebuild is source-safe;
- `--no-store` and rebuild paths whose report payload equals the stored path;
- owner-only mode 0700 for store directories created by ccwrapped and mode 0600 for the
  database/journal on Unix; an existing parent is preserved and rejected when group- or
  world-writable. Windows creates every missing store directory component, database, journal,
  and lock with a protected current-user DACL, rejects reparse-point ancestors, rejects
  untrusted ancestor owners or ACLs that grant delete or ACL-takeover rights to untrusted
  principals, and rejects an existing store directory that is not already protected; unsupported ACL platforms fail
  closed;
- `TRUNCATE` journaling, `synchronous=FULL`, `trusted_schema=OFF`, bounded payload decoding,
  and `secure_delete=FAST`.

The store is derived, local state rather than source authority. Corruption never mutates
inputs, an indeterminate source scan never deletes last-known rows, and every stored path has
an exact no-store equality oracle.

## Report and determinism contract

Report schema `ccwrapped.report/v2` is the canonical renderer input. It contains:

- a safe coverage/ingestion diagnostic projection;
- canonical metric values;
- a method catalogue keyed by stable method IDs;
- per-fact provenance, window, timezone, sample size, coverage, quality/confidence, and
  limitations;
- explainable comparisons and insights that reference their input fact IDs;
- privacy profile and source-authority policy versions;
- a privacy-safe input summary containing source kinds and counts, without stable source
  or snapshot identifiers.

The canonical JSON payload excludes wall-clock build time. `generatedAt` is retained as a
compatibility field but is redefined as the deterministic data-through timestamp (or a
stable null/empty representation for an empty report). An optional build timestamp may be
printed to stderr or a non-canonical private diagnostic, never into default `--json`.

Given identical source bytes, options, timezone database, pricing registry, and tool
version, canonical JSON is byte-stable. Maps use sorted keys, events have explicit stable
sort keys, floats serialize through documented rounding, and parallel ingestion cannot
affect order. Random salts, store identities, file metadata, and canonical path values do
not participate in aliases, ordering, or serialized standard facts.

Phase 1 calendar compatibility helpers normalize valid RFC3339 instants to UTC, including hour
analysis, so the process's ambient `TZ` is not an undeclared report input. Equal-count rankings
use lexical ascending public identity and equal-count hour rankings use the earliest hour. Phase 2
changes calendar attribution only through its explicit selected-IANA-timezone contract.

## Explainable insight projection

Phase 3 runs one bounded insight pass after source authority, canonical tokens, active time,
cost, and cache facts have reconciled. `ccwrapped.insights/v1` adds ten family-status
records and up to 32 ordered cards to `ccwrapped.report/v2`. A card owns its stable ID,
class, metric, method, selected-zone window, sample gate, availability, coverage,
confidence, supporting facts, limitations, privacy class, and optional bounded action.
The production reconciliation gate verifies the exact report/family catalogue, required
capabilities, ordering, cardinality, finite bounded values, valid half-open windows, and
method-specific arithmetic/references for every factual, recommendation, and entertainment
family before the report is authoritative. Each stable card ID also owns its exact
program-authored title/finding template and, for recommendations, its exact reversible
experiment plus ordered alternatives. Deliberate value, fact, method, sample, window,
narrative, experiment, and alternative mutations across all ten families return
`E_INSIGHT_RECONCILIATION`.
The adjacent-window proof also binds the two active-date counts and equal source signatures.
Both windows retain the seven-active-date gate. Explicit all-zero direct request/message
observations count as activity and can form an exact-zero baseline. Aggregate zero intervals
cannot waive the gate under v1 because clean file ingestion is not an exhaustive producer
coverage declaration and `(start,end]` intervals accepted within one local date omit that
date's opening boundary. The reconciliation gate rejects a fabricated prior-zero waiver fact.

The shared renderer projection formats card title, finding, context, optional action and
alternatives, family status, and supporting facts once. Terminal, HTML, Markdown, and the
share card consume those strings rather than recomputing rates. Share rendering includes
every family status plus every title, finding, context, comparison, limitation, and fact line
from every `privacyClass: share` card without a presentation truncation. Any future
share-safe action uses the same projection; current recommendation cards remain
standard-only. The top-project-alias concentration card also remains standard-only because
it carries a run-local project alias.
Program-authored templates contain all narrative text. Normalized arbitrary content, raw
identifiers, paths, tool details, and telemetry bodies have no field in the insight model.
The F041 regression scans every Rust source-owned template, every Markdown contract under
`docs/`, the repository README, every rendered text surface, and OCR extracted from all five
manifest-pinned screenshots. A synthetic positive fixed-savings claim proves the
documentation predicate is active; per-image canaries make an empty or ineffective OCR pass
fail rather than silently treating the image as clean.

Analytical work is bounded by the existing one-million-event ingestion cap, 28 daily trend
points, 10 tool cards/ranked models, three anomaly cards, 32 total cards, and 16 facts per card.
Tool recommendations evaluate the full classified-tool population before the card cap; a
rank-below-ten winner uses one factual trigger slot and leaves nine ordinary ranked tool slots.
Routing denominators use the full mapped population and collapse omitted mapped rows into one
bounded `other-mapped` fact. Request share resolves exact registry identity from canonical
request/message events independently of token presence and priceability; output-token and
local-cost shares retain their separate token and fixed-point cost evidence.
Sorting gives `O(n log n)` worst-case work over the accepted local snapshot and does not
reread source files.

Production ingestion auto-selects 12 workers, clamped to the logical CPUs available to the
process. File/source parsing uses bounded queues, canonical and alias analysis use scoped
parallel projections, and private-content archive parsing stays single-threaded. The Phase 5
historical campaign identified JSON parsing CPU as the limiting pipeline for its exact pinned
binary and informed the 12-worker default. F057's source-current contract is byte-identical
reports at 1/2/4/8/12/15 workers and under randomized scheduling. The retained timing,
duration, utilization, and RSS measurements are informational rather than a source-current
release gate after later correctness, privacy, persistence, and pricing changes.

## Canonical metric projection

Phase 2 projects the selected normalized events once into `canonicalMetrics`:

- `period/local-calendar/v1` converts every instant through one selected IANA timezone and
  coalesces an entirely skipped local date with the next real boundary;
- `activity/capped-interval-union/v1` keeps elapsed span separate, clips and locally splits
  half-open intervals, and unions concurrency before global totals;
- `tokens/canonical-sum/v1` preserves category presence, samples, units, overflow, and
  explicit unavailable/partial/saturated limitations across day/model/project/session
  partitions; every shared renderer fact includes all six categories plus the total;
- `cost/source-estimate/v1`, `cost/api-equivalent/v1`, and `cost/billing/v1` remain separate,
  with exact provider/model/effective-date/cache-TTL registry lookup and unpriced coverage;
  every report pins the complete sorted record inventory with aliases, intervals, modifier,
  citation/date, and integer pico-USD-per-token input/output/cache rates;
- `cache/read-share/v1` and `cache/write-share/v1` expose their exact denominators and no
  inferred health/cause/monetary effect;
- `MetricReconciliation` verifies mutually exclusive token and active-time partitions plus
  exact fixed-point source, local, priced, and unpriced cost domains before a report can
  serialize.

The runtime returns `E_METRIC_RECONCILIATION` when a projection fails. Standard renderers
consume the same canonical proof objects and emit exact shared `FACT` lines, including
sample, overflow, and limitation semantics for unavailable/partial/saturated facts. Compatibility
cost/cache/session fields remain additive shims: elapsed `durationMinutes` remains available,
while unsupported cache grades/breaks/reasons/monetary effects are neutral. Direct callers of
the frozen public `analyze_cache_health` and `detect_inflection_points` signatures receive the
same neutral compatibility behavior: observed token totals are retained, and unsupported
grades, ratios, savings, causal breaks, and inflection claims remain `N/A`/zero/empty/null.

Every terminal, HTML, Markdown, archive, and card fact comes from this report. Renderers
may change presentation, not definitions. One trust projection supplies profile, schema,
selected period, timezone, completeness, local API-equivalent provenance/coverage, and
limitations to every human renderer; JSON exposes the same inputs structurally. Partial or
indeterminate histories use an `observed activity` opening, while complete snapshots retain
the concise Wrapped/year opening. Standard Markdown encodes every report-derived string
as inert text: ASCII punctuation becomes numeric entities and control/bidirectional formatting
characters are replaced, so raw HTML, links, images, autolinks, and structural prefixes cannot
originate from a hostile public `Report`. The terminal renderer replaces report-derived C0/C1
controls, DEL, Unicode line separators, and bidi formatting controls before writing; termcolor
remains the sole source of terminal control sequences. Its environment-derived `COLUMNS` width is
clamped to 40–512 columns before any rule, padding, chart, or bar allocation, so hostile ambient
configuration cannot bypass the report's resource bounds. Labels, section rules, and padding
measure Unicode display columns; combining marks and wide scalars do not cause byte-count
misalignment or split an over-width scalar. HTML sinks escape markup and replace control,
line-separator, and bidi-formatting characters. Public display dates parse RFC3339
instead of byte-slicing arbitrary UTF-8, and percentage products widen before multiplication. Private
archive prompts and entrypoints are emitted
inside dynamically sized code fences longer than every backtick run in the value. Entrypoints are
capped at 512 bytes and join prompt text under the 32 MiB/10,000-record private-content budget.

All requested file renderers share one publication transaction. They render into a fresh
owner-only/current-user staging directory, validate every final path, retain backups for existing
regular standard files, and publish the private archive last with a no-clobber move. A normal
failure restores every prior standard file whose destination retains the installed filesystem
identity and removes staging. Rollback never unlinks an identity it does not own: a concurrent
standard-path replacement remains in place, while the displaced prior file remains in the named
owner-only recovery staging directory and the command reports incomplete rollback. Moves into
backup and final paths are no-clobber operations with identity validation on both sides. New
standard paths use no-clobber publication; symlinks and non-regular standard destinations are
refused. A bounded manifest is durable before the first rename, and a completion marker is
durable only after the destination directory sync. The next invocation, serialized by a
crash-releasing directory lease, restores the prior set after an interrupted multi-file
publication. Browser opening and success messages run only after the full set commits. Standalone
`--open`, or `--open` beside only Markdown/archive, implies the standard HTML output; when
HTML/card/all is already selected, exactly those HTML artifacts open. The parent waits for
each launcher and requires a zero status. A launcher-start or non-zero-status failure returns
`E_BROWSER_OPEN` and a non-zero exit after explicitly confirming that the files remain
committed; it does not roll them back.

## Compatibility and migration policy

The existing positional `YEAR` and `--html`, `--markdown`, `--card`, `--archive`, `--all`,
`--open`, `--json`, and `--plain` flags remain accepted. New source, timezone, privacy,
store, and schema controls are additive. `--json` is exclusive with every file/browser
flag. A conflict produces one `E_CLI_ARGUMENT_INVALID` JSON object on stdout, empty ordinary
stderr, exit 2, and no side effect. Configuration, ingestion, and empty-input JSON failures
likewise keep stdout to one value; empty input uses `E_NO_RECORDS` plus remediation and safe
coverage. Explicit private diagnostics remain a separately labeled stderr stream.

Default JSON must stop exposing prompt text, session IDs, project paths/hashes, and stable
project names. This is an intentional privacy-breaking schema correction and therefore
ships as report schema v2. A time-bounded explicit private compatibility projection may
emit schema v1 for local migration, with a warning on stderr; it is never the default and
is not shareable. A migration document will list every renamed, removed, and retyped field.

The crate is pre-1.0, but public Rust compatibility is still audited. Existing module,
type, and function names remain available where their contract can stay truthful. New
normalized APIs are additive. Legacy readers/analyzers become deprecated adapters over the
canonical engine rather than independent implementations. Any unavoidable removal or
signature change is recorded as semver-impacting and delayed to a declared version bump.

The legacy `read_all_jsonl(Path, Option<i32>)` and
`read_session_breakdown(Path, Option<i32>)` signatures are retained as infallible
compatibility projections. Both invoke the bounded normalized transcript adapter with
content and private diagnostics disabled, return only run-local project/session aliases,
classify model/tool names, and clear paths/prompts. `None` continues to select all observed
years; `Some(year)` filters in UTC. Their new `try_read_*` counterparts return the same
projection plus `DataCoverage`, or a path-free `IngestionReadError` with a stable code,
safe source alias, and remediation. The infallible shims emit a value-free stderr warning
or error when their frozen return type cannot carry those diagnostics.

The frozen `discover_jsonl_files(Path)` and `discover_session_files(Path)` helpers are likewise
adapters over normalized transcript discovery rather than independent filesystem walkers. Their
additive `try_discover_jsonl_files` and `try_discover_session_files` forms surface path-free
`IngestionReadError`; the infallible shims emit a safe error and return an empty fallback. The
shared traversal canonicalizes every returned path, confines it to the selected root, excludes
symlink escapes, enforces the invocation-wide entry/file/depth budgets, and validates file and
directory identity snapshots before returning. Session scope is exactly one project component
plus one JSONL filename beneath the canonical projects root.

The Phase 0 public API baseline is the rustdoc item inventory generated at revision
`1eeec07ea37e861f489696dcb2d5b2625397413d` with the exact compiler, rustdoc, Cargo,
target, and extractor versions recorded in the artifact. The generator rejects another
toolchain instead of producing a falsely comparable file and excludes compiler-internal
`Freeze`/`UnsafeUnpin` implementation details. Final reconciliation compares the same
artifact class and manually reviews changed signatures.

## Failure and recovery rules

Source absence, zero, unknown, unsupported, malformed, filtered, duplicate, and redacted
are distinct states. A partial report is allowed only when its diagnostics make the loss
visible. Explicit-source failures produce a non-zero exit. Implicit-source partial failures
produce a report plus warnings when at least one trustworthy source remains.

No error path silently turns an unavailable capability into zero. No causal narrative is
emitted from correlation alone. Unknown model/pricing data remains unpriced, and unknown
telemetry fields remain unsupported until an adapter fixture defines them.
