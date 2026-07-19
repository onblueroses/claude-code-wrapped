# Phase 0 research record

Sources were accessed 2026-07-16 unless a row records a later verification.

Official sources are semantic authority within their stated stability. Comparison tools
are product evidence only. Runtime adapters still reject or diagnose inputs that do not
match fixtures.

## Official source decisions

| ID | Source | Product decision | Stability | Unknown behavior |
| --- | --- | --- | --- | --- |
| R001 | [Claude Code sessions](https://code.claude.com/docs/en/sessions) | Default transcripts live under `~/.claude/projects`; `CLAUDE_CONFIG_DIR` relocates storage; transcript JSONL is internal and can change. Isolate it behind a versioned adapter and accept explicit roots. | default/path controls documented; transcript shape version-dependent | diagnose unknown record shapes; do not guess fields |
| R002 | [Claude directory/application data](https://code.claude.com/docs/en/claude-directory) | Transcripts include messages/tool data, are plaintext, and are cleaned after `cleanupPeriodDays` (30 days by default). Default reports omit content and never assume a retained year is complete. | retention default and paths version-dependent | mark historical coverage indeterminate when deletion cannot be proven |
| R003 | [Claude Code monitoring](https://code.claude.com/docs/en/monitoring-usage) | OTel events can directly provide request tokens/cost estimates, durations, request IDs, model, tool outcomes, errors, edits, and attribution. Metric wire units are `count`, `USD`, `tokens`, or `s` according to the official metric table and must match the metric name before canonical conversion. Prompt/response/tool/raw-body fields are privacy-sensitive and disabled/redacted by default. | signal names/attributes/units version-dependent | capability unavailable or unsupported; never proxy from unrelated fields or reinterpret an unknown unit |
| R004 | [Prompt caching](https://code.claude.com/docs/en/prompt-caching) | Cache reads/writes are facts; invalidation causes are asserted only from direct evidence. General experiments may reference documented model/effort/tool-prefix/compaction/TTL behavior. | behavior depends on Claude Code/model/provider/config | describe observed ratio changes without a cause |
| R005 | [Costs](https://code.claude.com/docs/en/costs) | Local `/cost`/OTel dollar values are estimates, not billing authority; subscription usage is a different surface. Label estimates and keep billing unavailable. | plan/interfaces change | preserve estimate with interface provenance or mark unavailable |
| R006 | [Commands](https://code.claude.com/docs/en/commands) | Built-in `/usage`, `/cost`, `/stats`, and `/insights` cover adjacent needs. Differentiate through local inspectability, methodology, privacy, deterministic exports, and cross-source coverage. | command availability/version-dependent | do not claim replacement for an unavailable/uninspected server surface |
| R007 | [Anthropic pricing](https://platform.claude.com/docs/en/about-claude/pricing), rechecked 2026-07-19 | Pricing depends on exact model/version and may depend on cache TTL, batch/fast mode, provider, residency, and effective date. Use a dated exact registry; unknown IDs and unsupported modifiers remain unpriced. | highly time/version-dependent | count unpriced usage; never broad-tier fallback |
| R008 | [Organizational analytics](https://code.claude.com/docs/en/analytics) | Team/enterprise analytics expose adoption and code metrics but estimates and billing are separate. Local reports describe only selected local sources. | product/plan-dependent | mark organizational/account facts outside local coverage |
| R009 | [Anthropic monitoring guide](https://github.com/anthropics/claude-code-monitoring-guide) | Prometheus/Grafana can provide historical cost/token/rate views from Claude OTel. Adopt evidence-oriented views, not ROI or causal productivity conclusions without direct outcome data. | example implementation; configurations/prices age | treat queries/configuration as examples and validate against current signal docs |
| R010 | [OTel metrics data model](https://opentelemetry.io/docs/specs/otel/metrics/data-model/) | Honor stream identity, single-writer, delta/cumulative temporality, start/end timestamps, resets, gaps, and overlaps. Missing/ambiguous intervals cannot be safely assigned to a period. | identity/temporality stable; reset/gap section marked development | surface conflict/partial coverage and exclude unsafe canonical attribution |
| R011 | [OTel Collector Contrib v0.148.0 file exporter](https://github.com/open-telemetry/opentelemetry-collector-contrib/blob/v0.148.0/exporter/fileexporter/README.md) and its [pinned module manifest](https://github.com/open-telemetry/opentelemetry-collector-contrib/blob/v0.148.0/exporter/fileexporter/go.mod) | Pin adapter v1 to file exporter `v0.148.0` (tag `d3c47b3`), pdata `v1.54.0`, and slim OTLP proto `v1.10.0`, with `format: json`, no encoding, and no compression. Accept only structurally conforming `resourceMetrics`/`resourceLogs` lines and report producer identity as unverified when the file supplies none. | metrics/logs exporter alpha; exact field names explicitly not guaranteed; dependency versions release-specific | count unknown fields; reject incompatible required paths/types with bounded unsupported-shape diagnostics rather than infer cross-release compatibility |
| R012 | [Claude Platform model IDs](https://platform.claude.com/docs/en/about-claude/models/model-ids-and-versions), [release notes](https://platform.claude.com/docs/en/release-notes/overview), and [model deprecations](https://platform.claude.com/docs/en/about-claude/model-deprecations), accessed 2026-07-19 | Match pinned and documented convenience model IDs exactly. A pricing record is valid only while its exact first-party model can exist. Bound current records at official launch and historical records before first-party retirement; treat impossible dates as unpriced. | model IDs, releases, and retirement schedules are time-dependent | retain the observation and tokens as unpriced outside the bounded interval |
| R013 | [Anthropic Fable 5 redeployment notice](https://www.anthropic.com/news/redeploying-fable-5), accessed 2026-07-19 | Fable 5 and limited-access Mythos 5 launched June 9, 2026, were suspended beginning June 12, and were restored under the updated access policy by July 1. Represent the interruption as two exact availability intervals rather than pricing impossible observations during the gap. | availability and access policy are time-dependent | keep June 12–30 observations unpriced; absence remains visible in coverage |

Objective Section 5 requires R001 and R003-R010. R002 is additional retention/privacy
authority and R011 is additional release-pinned authority for the chosen local telemetry
artifact. R012 and R013 supply effective model-availability bounds for the pricing registry.
The complete row-to-source inventory is
[`pricing-registry-2026-07-19.json`](pricing-registry-2026-07-19.json); its executable parity
test prevents a compiled rate, alias, modifier, or interval from drifting away from that
evidence. The R011 release pin was rechecked after an adversarial dependency review.

## Bounded comparison set

Exactly three current tools were reviewed.

### 1. [ccusage](https://github.com/ccusage/ccusage)

Observed strengths: daily/weekly/monthly/session/block views; multiple local agent sources;
date and timezone filters; JSON; project filtering; separate cache token categories; custom
paths; offline pinned pricing with scheduled updates and explicit overrides.

Adopt/adapt: explicit timezone and roots, report granularity, offline exact pricing
registry, JSON discipline, narrow views. Add stronger provenance, privacy profiles,
retention coverage, source authority, and deterministic proof objects.

Reject: treating a comparison implementation as authority for Claude's evolving internal
format or billing semantics.

### 2. [phuryn/claude-usage](https://github.com/phuryn/claude-usage)

Observed strengths: local dependency-light dashboard; incremental SQLite scan; custom
project directory; read-only transcript mount in Docker; model/date filtering; explicit
statement that API pricing differs from Pro/Max subscription cost.

Adopt/adapt: benchmark an inspectable incremental SQLite store, use read-only source
access, keep subscription and API-equivalent values distinct, provide useful local trend
views. Strengthen file fingerprints beyond path/mtime and reconcile deletion/truncation,
privacy, partial scans, and schema migration.

Reject: binding a dashboard to `0.0.0.0` without an explicit security warning/default
local-only posture; relying on a CDN in the default offline product; substring-only model
pricing.

### 3. [ColeMurray/claude-code-otel](https://github.com/ColeMurray/claude-code-otel)

Observed strengths: OTel Collector → Prometheus/Loki → Grafana architecture; request,
token, cost, latency, tool success, error, session, and code-change views; explicit
cardinality and privacy configuration.

Adopt/adapt: use direct event capabilities for reliability/tool behavior, show live versus
historical coverage clearly, control cardinality, and document privacy toggles. Keep the
core product local/offline and accept a small inspectable file artifact rather than require
a multi-service stack.

Reject: interpreting tokens/commits/LOC as ROI or productivity without outcome evidence;
default account/session identifiers in standard analytics; assuming a running observability
stack is appropriate for a fast personal CLI.

## Research boundary

Further product comparison is out of Phase 0 scope. Additional browsing is justified only
to resolve a frozen criterion, update a time-sensitive source, or verify a dependency/API
during implementation.
