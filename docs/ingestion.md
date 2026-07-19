# Unified ingestion contract

Implementation contract: ingestion/v1

The CLI reads each selected artifact once into one normalized event stream. Aggregate and
session views are derived from that same stream; analyzers and renderers do not parse
source files. All examples and tests use synthetic data.

The public compatibility readers follow the same rule. `read_all_jsonl` and
`read_session_breakdown` preserve their baseline signatures but delegate to this bounded
pipeline; they do not deserialize `readers::wire` records or read whole files. Additive
`try_read_all_jsonl` and `try_read_session_breakdown` return `(projection, DataCoverage)`
and a privacy-safe `IngestionReadError` on fatal discovery/import failure. Callers that
need to distinguish empty history from excluded or unreadable input use the `try_` forms.
The frozen infallible forms emit safe stderr diagnostics before returning an empty fallback.
For these APIs `Some(year)` filters in UTC and `None` means all observed years, recorded as
`dataCoverage.selectedPeriod = "all"`.

The public `discover_jsonl_files` and `discover_session_files` compatibility helpers also
delegate to the normalized transcript discovery boundary. Their additive `try_discover_*`
forms expose path-free `IngestionReadError` failures. Traversal shares the 100,000-entry/file
budgets and 128-level depth cap, canonicalizes returned files, rejects out-of-root symlink
escapes, validates directory/file snapshots, and never converts a read failure into an
authoritative empty list. The frozen infallible helpers emit a safe error and return an empty
fallback when that return type cannot represent failure. Session discovery retains only the
direct `projects/<project>/<session>.jsonl` layout; recursive JSONL discovery retains supported
nested paths.

## Discovery

Transcript-root precedence is deterministic:

1. Repeatable --data-dir PATH values, in command-line order.
2. CLAUDE_CONFIG_DIR/projects when no explicit transcript root was supplied.
3. HOME/.claude/projects when the prior implicit location is unavailable.

An explicit path may be a projects directory or a Claude configuration directory
containing projects/. Paths are canonicalized before duplicate detection. Duplicate
canonical roots import once. Transcript files are traversed in sorted order; aliases are
assigned as transcript-1, transcript-2, and transcript-N-file-M.
Coverage records whether each root was interpreted as explicit-projects, explicit-config,
claude-config-env, or home-default. A selected directory literally named projects is
treated as a projects root even if it contains a child with the same name.

Repeatable --otel-file PATH values select regular, uncompressed files in command-line
order and receive otel-1, otel-2, and subsequent aliases. Telemetry is never discovered
from an environment variable or network listener.

Standard diagnostics contain only aliases, kinds, and counts. Exact canonical paths are
printed only when `--private-diagnostics` is present, and then only in stderr records
labeled `privacy-profile: private`. This explicit stream may accompany JSON without
changing its stdout value. A missing or unreadable explicit source exits non-zero. Missing implicit roots are warnings when
another selected source is usable. A read or mutation failure is indeterminate, never an
authoritative empty history.

Once `CLAUDE_CONFIG_DIR/projects` is selected by the precedence probe, a later
canonicalization, metadata, or identity failure does not silently substitute the home
default. The failed root remains visible as a partial, path-free `transcript-N` source with
`W_DISCOVERY_IMPLICIT_UNREADABLE`, while any independently selected explicit telemetry
source continues through ingestion. The same implicit failure is fatal when no usable
declared source survives.

With `--json`, fatal discovery/import failures are one JSON object containing error, code,
sourceAlias, a path-free explanation, and remediation; stderr stays empty unless the user
also selected --private-diagnostics. Source files are opened through their discovered
canonical identity. A discovery-time file snapshot must match the opened handle before
parsing, and the same handle identity must still match final handle and path metadata.
Every traversed transcript directory retains an identity snapshot that must match after
traversal and again after all discovered files are ingested; the selected root is checked at
both boundaries as well. A nested addition, removal, replacement, or metadata change therefore
makes the selected source fatal/indeterminate instead of publishing an incomplete scan.
Transcript and explicit telemetry files deduplicate by Unix device/inode or Windows
volume/file-index identity, with canonical path only as a fallback when native identity is
unavailable, so hard links are scanned once. Fallback project/session context retains native path components: Unix
hashes exact bytes and Windows hashes exact UTF-16 units. Invalid-Unicode names never pass
through lossy text conversion or enter standard output.

An empty selected period is not a successful zero report. JSON returns one
`E_NO_RECORDS` object with a stable remediation and safe `dataCoverage`; human output
returns the same code and next action. Neither path writes an output artifact.

Every public day/year/hour compatibility helper validates RFC3339 before bucketing and converts
the instant to UTC. A source offset that crosses an hour, midnight, or New Year therefore uses the
normalized UTC bucket, while malformed timestamp text returns no bucket instead of contributing
a raw prefix. Ambient `TZ` never changes Phase 1 report facts; Phase 2 applies the user's explicit
IANA timezone selection instead.

## Supported adapters

claude-transcript/v1 accepts UTF-8 JSON Lines under the Claude projects layout. It
recognizes assistant, user, progress, summary, and system records plus main, sidechain, and
subagent context. Unknown variants and malformed lines are counted independently. Blank
physical lines are classified as skipped; the global and owning-source completeness states
both retain that partial/indeterminate evidence.

claude-otel-otlp-json/v1 accepts one JSON export object per physical line from the
OpenTelemetry Collector Contrib file exporter v0.148.0, Collector pdata v1.54.0, and slim
OTLP proto v1.10.0, configured with format json, no encoding, and no compression. The file
contains either resourceLogs or resourceMetrics per line. Metrics use the
com.anthropic.claude_code scope; log/events accept the known
com.anthropic.claude_code.events scope and the base scope used by compatible variants.
The producer version is not embedded, so reports state the expected contract and
unverified producer status.

The pinned protobuf JSON mapping represents every 64-bit integer (`intValue`, `asInt`,
and Unix-nanosecond fields) as a checked decimal JSON string. Integer metric points remain
exact signed 64-bit values through ordering and cumulative-delta calculation; converting
an exact non-negative token delta to `u64` happens only after normalization. A point with
both `asInt` and `asDouble`, a malformed decimal string, or an out-of-range integer rejects
its export object transactionally.

Supported event facts include API request/error, user-prompt occurrence, tool result and
decision, and compaction. Content-bearing request/response-body events are rejected by the
standard profile. Supported metrics are the documented Claude session, token, cost,
active-time, code-edit-decision, lines-of-code, commit, and pull-request monotonic sums.
Adapter v1 requires the documented wire unit for each name: `count` for counters/decisions,
`USD` for cost, `tokens` for token usage, and `s` for active time. Unit absence, wrong type,
or mismatch rejects the export object transactionally before any point is retained. Each
accepted metric retains a canonical metric name, numeric delta, canonical display unit,
temporality, and exact accumulation interval in the normalized event. Other signals,
scopes, metric kinds, names, and units are counted as unsupported rather than guessed.

## Resource limits

Before a record enters normalized storage, the reader enforces:

- 16 MiB per physical line; oversized tails are drained without buffering.
- 4 GiB of selected source bytes and 16,000,000 physical records across the entire
  invocation. Every line consumes the shared record budget before parsing, including
  malformed, unsupported, filtered, or oversized records.
- 256 selected local inputs per invocation; default transcript discovery reserves one
  slot when no explicit transcript root is selected.
- Serde JSON's enabled recursion limit.
- 256 resource groups and 256 scopes per resource/export object.
- 100,000 log records or metric points per export object.
- 128 attributes and 64 KiB decoded attribute text per entity.
- 1,000,000 distinct metric streams per invocation.
- 1,000,000 normalized events per invocation.
- 128 observations per repeated request-correlation group and 20,000,000 exact-assignment
  work units per invocation; an oversized group is excluded independently, while aggregate
  budget overflow excludes all otherwise bounded groups so source order cannot select winners.
  Excluded groups stay transcript-authoritative and emit `W_AUTHORITY_CORRELATION_LIMIT`
  rather than starting unbounded matching work.
- 100,000 directory entries total across every selected transcript root per invocation,
  consumed before path allocation; each source also retains separate 100,000 unique-file and
  unique-directory ceilings, 128 directory levels, and 128 tool categories per event.
- 10,000 private prompts and 32 MiB of private prompt plus bounded entrypoint text when --archive
  is explicit; each prompt is capped at 64 KiB and each entrypoint at 512 bytes.

The Phase 1 baseline used one worker. Production now defaults to 12 workers, clamped to the
affinity-available logical CPU count. The hidden benchmark override may select any positive
count within that same topology. Transcript files and OTel sources use bounded work queues
with at most two queued results per transcript worker; canonical and alias projections use
scoped threads.
Private-content archive ingestion remains single-threaded so content never crosses a worker
boundary. F057 proves source-current byte-identical output across hardware-valid worker counts
and randomized delays. The Phase 5 record retains the historical scaling curve for its exact
pinned binary; its speed, duration, and utilization values are informational.

Counters use saturating arithmetic. A structural-limit failure rejects the export object
and records a stable warning code; staged metric state, aliases, time coverage, and facts
from that object are rolled back together. It never returns partially guessed facts from
that object.

Transcript records validate their supported type, timestamp/period, and required assistant
message/usage shape before allocating public project or session aliases. A rejected record changes
only its bounded diagnostics; it cannot shift aliases assigned to later accepted observations.

Resource and scope attributes are parsed once and borrowed by each child record/point. A fixed
three-layer lookup applies local, then scope, then resource precedence without cloning inherited
maps or identity strings. The instrumented `otel_inherited_attribute_merge_work_is_linear`
regression bounds lookup probes independently of inherited text size, so legal record cardinality
cannot multiply 64 KiB inherited attribute payloads into gigabytes of copy work.

Global and per-source capability maps are reduced in one pass over normalized events into one
compact accumulator per selected source. The implementation performs one source-alias lookup
per event and allocates no per-source event vector, keeping capability work linear under the
combined 256-source/one-million-event bounds. The instrumented
`source_capabilities_are_linear_in_events_and_sources` regression enforces this work model.
Direct-event keys keep event-family occurrence separate from attribute evidence:
`api_request`, `api_error`, `direct_terminal_outcomes`, `retry_evidence`, `tool_result`,
`tool_status`, `tool_latency`, `tool_decision`, and `edit_decision`. A malformed or skipped
OTel record makes an otherwise observed direct denominator partial. A structurally parsed,
unsupported event name remains an unrelated excluded family and does not weaken complete
allowlisted API/tool evidence. Producer-declared dropped attributes weaken attribute-derived
capabilities while leaving the already parsed event-family occurrence intact. Resource- and
scope-level declarations propagate to their accepted children; record- and point-level
declarations remain local. The reducer then weakens only the child’s own family: declarations on
API request events affect retry/usage/cost evidence, tool-result declarations affect
status/latency, tool-decision declarations affect edit decisions, and unrelated prompt or tool
families cannot suppress one another. Token/cost metric-point declarations affect the
corresponding canonical usage/cost evidence without weakening direct API or tool families.
Direct duration is accepted only when finite and within zero through 86,400,000
milliseconds. An out-of-range duration is discarded before nanosecond conversion, preserves
the classified result occurrence, and weakens only latency evidence.

Comparison, trend, routing output-token share, concentration, and anomaly construction derive
a separate canonical-usage evidence state from the sources that actually contributed token
usage. Routing request share instead follows canonical request/message events, exact registry
mapping, and each contributing direct source's `api_request` capability; token presence and
priceability do not decide request identity. Unrelated malformed tool-only telemetry therefore
cannot suppress either exact request or token evidence, while malformed contributing evidence
remains partial in its own domain. Strong transcript/OTel request correlation selects one
authoritative observation and records the resolved overlap rather than summing both.

## Privacy boundary

Adapters use a deny-by-default allowlist before normalized events are retained. Standard
events never retain prompt/response content, commands, tool arguments/results, raw errors,
email/account/organization values, canonical paths, or unsalted stable identifiers.
Identifiers needed for local correlation are transformed through a fresh process-keyed
hasher and are never serialized. Deterministic public aliases derive from accepted source/event
order, so fresh salts and rejected records do not change the identities of retained facts.

Model identifiers must match a bounded Claude-family grammar. Built-in tool names are
allowlisted; MCP and arbitrary/custom names become the generic mcp or other category.
Rejected or transformed values increment redaction counts.
Unknown diagnostics retain counts, stable codes, a maximum of 32 structural samples,
allowlisted field names, JSON value kinds, and bounded byte counts; they never retain
arbitrary keys or values.
Redactions are counted at the physical attribute/field boundary. `--archive` is a separate
explicit private-content sidecar and does not put content back into the standard report.
Prompt excerpts and entrypoints are wrapped in dynamically sized Markdown code fences so their
HTML, links, images, autolinks, and fence-like text remain inert when opened. Standard Markdown
uses context-complete entity encoding for every report-derived string and replaces control/bidi
formatting characters. Terminal output replaces report-derived C0/C1 controls, DEL, line
separators, and bidi formatting controls before writing, so only renderer-owned color sequences can
alter terminal state.
Incremental-store directories and files use owner-only modes on Unix. On Windows, every missing
directory component and store file receives a protected current-user ACL in its creation syscall;
reparse-point ancestors, untrusted owners, and ancestor ACLs that grant delete or ACL-takeover
rights to untrusted principals fail closed before any missing component is created.
Every archive file starts with `privacy-profile: private-content`, and the stderr warning
uses the same label. The archive root must not already exist: the writer creates a fresh protected directory,
refuses files/directories/symlinks already present at wrapped-archive/, and creates each
staging filename once without following or replacing another entry. Unix uses 0700
directories and 0600 files. On Windows, the writer supplies a protected current-user ACL
while creating each private directory/file and fails safely on volumes that cannot enforce it;
file publication does not require hard-link support. The writer assembles and syncs the
complete archive in a protected sibling staging directory, cleans staging on ordinary
errors, and atomically moves it into the absent final name without clobbering a destination
created by a concurrent process. Supported Linux architectures call the kernel
`renameat2(RENAME_NOREPLACE)` operation through `syscall`, avoiding a dynamic dependency on
glibc's versioned `renameat2` wrapper; `ENOSYS` and filesystem capability errors fail safely.
Move or remove a prior archive before rerunning.

## Deduplication and authority

Within a source, the dedup key includes source, project/session context, main/subagent and
sidechain context, parent/agent context, event kind, and the strongest
request/message/tool identity together with its exact event instant. Reuse of an identifier
at another instant remains a distinct observation; an exact-instant repeat selects the richer
valid observation. Equally rich conflicts select the lexicographically smallest privacy-safe
normalized fact vector, excluding salted identities, aliases, and physical file/record positions;
exact fact equality is analytically interchangeable. Duplicate decisions are counted and output
ordering is stable. Strong request/message/session identities also collapse
repeated observations across selected roots; those decisions are counted separately as
resolved overlaps. When a record has no strong request/message identity, overlap collapse requires
equality across its complete privacy-safe normalized facts and salted identity context. Different
status, latency, error/retry, edit/compaction, or agent/skill/plugin/MCP facts remain separate.
Neither rule uses source aliases or physical file/record position to choose conflicting facts.

OTel delta windows use (start, end]. Cumulative points become differences within one
writer sequence, including sequential explicitly selected files. Supported points are
reduced to bounded privacy-safe metadata, sorted by interval and stable observation order,
and only then converted to deltas, so selector order cannot change analytical facts or
global accounting. Attribute order does not change stream identity. Exact repeats are
idempotent; non-overlapping changed starts create a reset sequence; gaps mark partial
coverage; ambiguous resets and overlaps are excluded and classified as filtered globally and for
the owning source. Windows crossing a selected-zone local day or reporting period are likewise filtered and
never assigned by export/end time or prorated. Because intervals are `(start,end]`, an endpoint at
midnight belongs to the new day. The per-source coverage table still
uses command-line-order aliases by the discovery contract.

authority/v1 prefers a correlated timestamped OTel API request over its transcript usage
observation. The selected OTel event inherits the transcript project/session alias for
stable attribution. Correlation requires the same salted request and present session
identity, compatible agent/sidechain context, and timestamps within five minutes. Repeated
identity groups use an exact bounded assignment: maximize resolved pairs, then minimize total
timestamp distance, then align canonical public observation ranks. Canonical ordering uses
timestamps, public project/parent aliases, safe model and usage facts, and only then stable source
position; it excludes private salted IDs. This makes equal-distance project attribution invariant
to record order. OTel derives subagent status before hashing its session identity so its main/
subagent domain matches transcript normalization. A group beyond the published resource
limit remains unresolved with a bounded warning. When both families overlap without that evidence,
transcript usage remains canonical and the OTel observation is counted as unresolved
instead of summed. With no transcript usage, OTel request events are canonical at their explicit
timestamps. Exact metric deltas remain canonical when every direct request or transcript usage
observation is disjoint by interval or explicit session/model/agent context. A compatible OTel
request inside a metric's `(start, end]` interval supersedes that aggregate observation. A
compatible transcript usage observation cannot prove aggregate replacement, so a same-context
interval containing it is excluded as unresolved; temporal overlap without enough shared context
is likewise unresolved. Neither case globally suppresses unrelated, provably disjoint metrics.
Aggregate metrics contribute usage values but never message, session, project-occurrence, or
message-story cardinality.

## Coverage and deterministic output

Report schema ccwrapped.report/v2 includes dataCoverage: selected period/timezone,
earliest/latest observations, observed span, sources/files/records, capabilities,
completeness, retention caveat, privacy and authority versions, every exclusion counter,
and stable warnings. acceptedRecords counts deduplicated source observations;
canonicalRecords counts post-authority facts; classifiedRecords reconciles accepted,
malformed, unsupported, filtered, skipped, and duplicate dispositions through the emitted
recordCountInvariant. Unknown/redacted fields and authority overlaps remain orthogonal
counters. generatedAt is the deterministic latest observed UTC timestamp rather than
wall-clock report construction time; subsecond ordering is preserved internally.

Canonical analytical capability keys preserve optional usage facts beyond normalization.
The four primary token categories use `available`, `partial`, or `unavailable` according to
presence across all applicable canonical events. `analysis_cost` additionally requires a
source estimate or a complete known-model token tuple for every cost event;
`analysis_cache_health` requires complete primary token tuples and a non-zero observed
denominator. Incomplete inputs set legacy `costCoverage` to
`unavailable-incomplete-usage` or `partial-observed-cost-evidence`, retain only the exact
locally priceable subtotal in compatibility total/day/model fields beside the partial
capability, and
neutralize legacy derived averages/peaks, routing monetary effects, cache grades, anomalies,
and recommendations. Canonical cost independently reports exact-registry priced/unpriced
coverage. Source-recorded estimates remain separate in canonical cost and the legacy session
`costUsd` field; they are never substituted into or added to the local compatibility subtotal.
A complete explicit zero tuple remains available evidence, while a zero-denominator
canonical cache share remains unavailable.
All non-negative token/count compatibility rollups saturate rather than panic or wrap. If
the canonical sum for any primary token category exceeds `u64`,
`W_ANALYTICAL_TOKEN_SATURATED` identifies the limitation without disclosing values; that
category and `analysis_usage_totals` become `partial`, legacy `analysis_cost` becomes `partial`,
legacy `analysis_cache_health` becomes `unavailable`, and observed numeric sums clamp to
`u64::MAX`. Canonical overflow fields remain true and downstream cache ratios become unavailable.
Separate aggregate token metric points satisfy their named categories collectively. A
simultaneous source-cost metric and locally priceable token-metric family sets
`costCoverage` to `unavailable-conflicting-cost-bases`; compatibility total/day/model costs
remain the local API-equivalent subtotal rather than summing unlike estimates.

Rejected records whose kind or period cannot be proven are analytical uncertainty, not merely a
diagnostic counter. Malformed, oversized, unsupported, missing-timestamp, and producer-declared
dropped-attribute records downgrade otherwise complete token and cost capabilities to `partial`;
observed sums remain visible, while stronger derived narratives and recommendations stay
suppressed. Exact category exclusions affect only the named categories. Valid records and
metric intervals proven wholly outside the selected year, plus known non-Claude or content-bearing
signals, remain counted as filtered/unsupported evidence without weakening unrelated period claims.

Completeness is empty, partial, indeterminate, or complete. Transcript-backed history is
indeterminate even after a clean scan because local retention may have removed older
activity. Malformed, unsupported, unknown, skipped, inaccessible, or unresolved overlap
evidence prevents a complete claim.
