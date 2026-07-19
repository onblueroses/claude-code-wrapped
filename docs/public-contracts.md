# Baseline public contracts

Baseline revision: `1eeec07ea37e861f489696dcb2d5b2625397413d`
Package version: `0.3.0`
Inventory format version: 1

This is the compatibility baseline before the normalized pipeline. It describes current
behavior; it does not endorse behavior that the architecture or methodology identifies as
unsafe or untruthful.

## CLI contract

The exact immutable baseline help is stored in
[`baseline/cli-help.txt`](baseline/cli-help.txt). The live Phase 1 help is stored separately
in [`current/cli-help.txt`](current/cli-help.txt); a regression byte-compares that current
contract with the built binary while retaining the Phase 0 bytes for the final migration
diff.
The executable accepts positional `YEAR` and these flags: `--html`, `--markdown`,
`--card`, `--archive`, `--all`, `--open`, `--json`, and `--plain`, plus
Clap's help/version flags.

With no output flag, terminal output is primary and no file is written. File formats are
opt-in. `--open` is a browser-readable-output request: it implies the standard HTML file
when HTML/card/all is otherwise absent, then opens every selected HTML artifact after
commit. `--json` is exclusive with HTML, Markdown, card, archive, all, and open. A conflict
returns one `E_CLI_ARGUMENT_INVALID` object on stdout, empty ordinary stderr, exit 2, and no
side effect. Default source discovery and selected-year behavior are documented in the
live ingestion contract.

Existing output filenames are `claude-code-wrapped.html`,
`claude-code-wrapped.md`, `claude-code-wrapped-card.html`, and
`wrapped-archive/`. The private archive is created only when that path is absent; existing
files, directories, and symlinks are refused so prior private output is never merged or
overwritten. Prompt and entrypoint values render as inert fenced Markdown literals; prompt text is
bounded at 64 KiB, entrypoints at 512 bytes, and both count toward the 32 MiB private-content
budget. Standard Markdown entity-encodes all report-derived punctuation and replaces control/bidi
formatting characters. Terminal rendering likewise replaces report-derived C0/C1 controls, DEL,
line separators, and bidi formatting controls while retaining only renderer-generated color
sequences. The public `terminal_width()` helper treats `COLUMNS` as ambient input and clamps its
result to 40–512 columns before renderer allocation. All requested file formats first enter one
protected sibling staging transaction.
Existing regular standard files retain rollback backups until the complete requested set commits;
standard symlinks/non-regular destinations are refused, absent standard paths use no-clobber
publication, and a later failure restores earlier files whose installed filesystem identities
still match. Rollback does not delete or overwrite a concurrent standard-path replacement. It
instead retains the prior file in the owner-only staging directory named by the returned error;
the user reconciles those two files before retrying. A bounded manifest is persisted before the
first destination rename and a completion marker only after the destination directory is synced.
The next invocation holds the same crash-releasing directory lease and restores the prior set
when it finds an incomplete manifest. A protected archive staging directory is
published through one no-clobber atomic move, so ordinary write failures do not expose a partial
final archive or block a retry. Archive files require no hard-link support. Windows archive output requires a volume
that accepts the protected current-user ACL and fails before content when that privacy
property is unavailable. Linux no-clobber publication uses the kernel syscall and does not
import glibc's versioned `renameat2` wrapper; unavailable kernel/filesystem support is a safe
runtime error. Browser-open is deferred until transaction commit; the parent waits for the
launcher and requires zero status. Spawn and non-zero-status failures are reported as
`E_BROWSER_OPEN` with a non-zero exit while retaining every committed file. Success messages
are likewise emitted only after commit. JSON configuration, ingestion, and empty-input failures
are one value with stable `code`, `message`, and `remediation`; empty input uses
`E_NO_RECORDS` and includes safe `dataCoverage`.

Compatibility policy: preserve every accepted baseline flag and positional argument.
Additive options may refine sources, timezone, privacy, store, and schema. Unsafe default
JSON changes through the declared v2 migration rather than silently retaining v1.

Phase 1 adds repeatable `--data-dir PATH`, repeatable `--otel-file PATH`, and
`--private-diagnostics`. The first two select explicit local artifacts. The last is the
only CLI surface that prints exact selected paths, labels that stderr stream `private`, and
never changes the standard JSON payload.

Phase 2 adds `--timezone IANA_ZONE` and
`--active-threshold-minutes MINUTES`. An explicit zone controls every canonical year/day/
weekday/hour label and makes ambient `TZ` irrelevant. With no explicit option, the host
IANA zone is selected when resolvable and UTC is selected with
`W_TIMEZONE_DEFAULTED_TO_UTC` otherwise. The active threshold accepts whole minutes from
1 through 1440 and defaults to 5; invalid values use path-free JSON error codes when
`--json` is active.

Report v2's `dataCoverage.capabilities` map also carries additive analytical
presence keys. Their values distinguish `available`, `partial`, and `unavailable`; consumers
must use `canonicalMetrics` for canonical time, token, cost, cache, and reconciliation
facts. Legacy numeric cost/cache accumulator fields remain compatibility projections and
cannot supply canonical billing or cache-health meaning. Standard renderers consume
canonical facts and emit an explicit limitation whenever source coverage is incomplete or
indeterminate. Token/count
rollups saturate at their existing unsigned integer bounds; canonical token-category
overflow additionally emits `W_ANALYTICAL_TOKEN_SATURATED` and marks affected analytical
capabilities partial so consumers cannot interpret the clamped value as exact.

Phase 3 additively names direct-event capability keys: `api_request`, `api_error`,
`direct_terminal_outcomes`, `retry_evidence`, `tool_result`, `tool_status`, `tool_latency`,
`tool_decision`, and `edit_decision`. These map entries describe observed, usable evidence;
they do not reinterpret an absent event as a measured zero. Unsupported named event families
remain separate from allowlisted denominators, while malformed/skipped records make affected
direct denominators partial.

## JSON contract

Baseline `--json` is the camelCase Serde serialization of `Report`. The
machine-comparable JSON-name/value-kind fixture inventory is
[`baseline/report-v1-fields.txt`](baseline/report-v1-fields.txt). It is generated from
actual `Serialize` implementations by
[`scripts/capture-report-schema.sh`](../scripts/capture-report-schema.sh), not inferred
from Rust field spelling. It records every current public `Default + Serialize` report
struct's default field inventory, recursively walks a canonical non-empty synthetic
`Report` plus the non-report public aggregate types, and includes generic
flatten/conditional/custom-serializer probes. A regression cross-checks the enumerated
types against the public Rust artifact.

This is a bounded fixture-shape inventory, not JSON Schema and not proof of every possible
value-dependent serialization. The baseline report domain contains structs only and uses
no Serde field/container behavior beyond `rename_all = "camelCase"`; regressions enforce
that boundary. Adding a serializable enum or a skip/flatten/custom field serializer must
first add an exhaustive variant/behavior fixture matrix or replace this extractor with a
schema generator. The public Rust artifact separately captures exact
field/container/option types.
The generator extractor-hashes and runs an independently locked `syn` source audit before
publication. It rejects unmodeled conditional/field-level rules and manual implementations,
and its public derived-type inventory must exactly equal the emitted fixture inventory.

Top-level fields are:

- `generatedAt`, `year`
- `costAnalysis`, `cacheHealth`, `anomalies`, `inflection`
- `sessionIntel`, `sessionBreakdown`, `modelRouting`
- `projectBreakdown`, `recommendations`, `wrappedStory`

All report structs serialize their public fields and do not omit defaults. The baseline
has no schema discriminator. It includes sensitive session IDs, project hashes/paths/names,
prompt text/previews, and parent identities through nested structures. It also includes
wall-clock `generatedAt`. Those are recorded compatibility facts and confirmed privacy
or determinism defects, not promises to retain in standard schema v2.

The compatibility plan is:

- standard `--json` becomes explicitly versioned safe report v2;
- an explicit local private v1 projection may exist only as a time-bounded migration shim;
- common factual values keep stable field meaning where truthful;
- removed, renamed, retyped, and privacy-reclassified fields enter a migration table and
  golden schema fixtures before release.

The current standard payload identifies itself as `ccwrapped.report/v2`, adds
`dataCoverage`, changes `generatedAt` to the latest observed source timestamp, and fills
legacy identity-shaped fields with run-local aliases or null rather than raw values. The
Phase 0 schema artifact stays immutable. The checked
[`current/report-v2-fields.txt`](current/report-v2-fields.txt) source-only capture is the
complete current v2 Serde authority; the capture test regenerates it with the pinned
extractor and requires byte-for-byte equality. It proves the Phase 1 ingestion/coverage
surface plus Phase 2
`MethodologyCatalog`, canonical token/activity/cost/cache/reconciliation types, active/
elapsed session fields, `PricingRegistryRecordMetadata`, Phase 3 `InsightReport` proof
types, and the expanded `Report` fields.

### Exhaustive report-v1 to report-v2 field migration

The two checked Serde inventories are the machine authority. Their field-declaration diff
contains no removed, renamed, or retyped v1 field. The complete structural delta is:

| Owner | Report-v1 fields | Report-v2 disposition |
| --- | --- | --- |
| `Report` | `anomalies`, `cacheHealth`, `costAnalysis`, `generatedAt`, `inflection`, `modelRouting`, `projectBreakdown`, `recommendations`, `sessionBreakdown`, `sessionIntel`, `wrappedStory`, `year` | All twelve names and JSON types remain. Add `schemaVersion`, `dataCoverage`, `methodology`, `canonicalMetrics`, and `insights`. |
| `ModelAggregate` | all seven v1 fields | Retain all; add `activeSeconds: number`. |
| `DailyAggregate` | all ten v1 fields | Retain all; add `activeSeconds: number`. |
| `ProjectSummary` | all twelve v1 fields | Retain all; add `activeSeconds: number`. |
| `SubagentSummary` | all nine v1 fields | Retain all; add `activeSeconds: number` and `elapsedSeconds: number`. |
| `SessionSummary` | all sixteen v1 fields | Retain all; add `activeSeconds: number`, `elapsedSeconds: number`, and `inclusiveActiveSeconds: number`. |
| `SessionBreakdown` | all four v1 fields | Retain all; add `totalActiveSeconds: number` and `totalElapsedSeconds: number`. |
| Every other v1 type | Every captured field of `TokenUsage`, `AssistantEntry`, `SessionPrompt`, the cost/cache/anomaly/session-intelligence/routing types, `InflectionPoint`, and the wrapped-story presentation types | Retain the exact JSON field name and captured JSON type. The immutable v1 inventory and regenerated v2 inventory enforce this catch-all mechanically. |
| New v2-only types | absent | Reachable only through the five new `Report` fields. The current inventory enumerates every coverage, methodology, registry, canonical-metric, reconciliation, and insight field. |

Retained shape does not imply retained unsafe meaning. This is the complete semantic
disposition for the v1 fields whose value contract changed:

| Field family | Report-v2 meaning |
| --- | --- |
| `generatedAt` | Latest accepted source observation, with deterministic selected-period fallback; no wall-clock generation timestamp. |
| `AssistantEntry.{sessionId,projectHash,cwd,model,toolNames}` and nested session/subagent/project identity or content carriers | Standard construction supplies run-local aliases, classified values, or null/empty values. Raw IDs, paths, prompts, and arbitrary labels are available only through the separately labeled private archive/diagnostic paths where documented. |
| `costAnalysis`, daily/model cost fields | Compatibility projection of the local API-equivalent domain only. Canonical source-recorded, local modeled, billing-unavailable, and unpriced coverage stay separate under `canonicalMetrics.cost`. |
| `SessionSummary.costUsd` and subagent/session source cost | Source-recorded estimate only; never added to the local modeled compatibility subtotal. |
| `cacheHealth` | Direct token totals remain; grade, inferred break, cause, ratio, inflection, and savings-shaped values are neutral compatibility defaults. Canonical supported shares and direct compaction counts live under `canonicalMetrics.cache`. |
| `anomalies` and `inflection` | Neutral compatibility outputs; evidence-backed descriptive insight cards replace unsupported legacy inference. |
| `modelRouting` | Observed mapped shares only; no quality, intent, or guaranteed-savings interpretation. |
| `recommendations` | Projection of evidence-gated typed recommendation cards; no unsupported fixed-savings claim. |
| `wrappedStory` | Entertainment-only labels are explicitly marked and sample-gated; factual totals still reconcile to canonical data. |

Thus the v2 migration is additive at the serialized field layer and intentionally corrective
at privacy and analytical-meaning boundaries. Consumers that need authoritative new meaning
should use the five versioned v2 roots, while compatibility fields remain bounded projections.
Rust callers that construct public structs with literals must add `..Default::default()` or
initialize additive fields explicitly. In particular, `ModelRouting` now exposes `methodId`,
`unit`, `observations`, `otherPct`, and `unknownPct`; canonical token and activity metrics expose
an explicit project-unattributed partition rather than manufacturing a project alias.

### Phase 3 insight migration

`Report.insights` is an additive `InsightReport` with schema identity
`ccwrapped.insights/v1`. Its public structs are `InsightWindow`, `InsightFact`,
`InsightComparison`, `InsightAction`, `InsightCard`, `InsightFamilyStatus`, and
`InsightReport`. Every type derives `Default` and `Serialize`, uses the existing camelCase
field convention, and is included in the bounded current-source report fixture inventory.

The compatibility `recommendations` vector remains present. Standard construction fills it
only by projecting typed evidence-rule cards; legacy public analyzer helpers remain callable
and return no recommendation because their aggregate arguments cannot carry the direct
proof contract. They do not feed the standard report. The compatibility `wrappedStory` entertainment fields
remain present and now carry a visible `Entertainment ·` marker or a neutral below-gate
message. These are intentional semantic corrections without a field removal or signature
change.

The JSON proof collection is authoritative. Terminal, HTML, Markdown, and share-card
outputs consume the same ordered cards and facts. Share projection includes every family
status and every title, finding, context, comparison, limitation, and supporting-fact line
from every `privacyClass: share` card without family/card/fact truncation. The same shared
projection carries experiments and ordered alternatives when an action is privacy-eligible.
It excludes current standard-only recommendation cards and the project-concentration card
that contains a run-local alias.

Adjacent-window comparison cards include prior/current active-date counts and coverage
signatures as supporting facts. Both windows require seven active dates under current v1
adapters. Explicit all-zero direct request/message observations count as activity; a clean
OTel file, available token capability, or accepted aggregate zero interval does not claim
exhaustive producer coverage and cannot waive the gate. Production reconciliation rejects a
fabricated `comparison.prior-zero-coverage-days` fact.

Routing share facts use the complete mapped/unknown denominator before named rows are capped
and expose an `other-mapped` tail when mapped models fall outside the ten-row presentation.
Tool recommendation candidates likewise use the complete classified-tool population. A
below-rank-ten winning tool receives a bounded factual trigger card that remains within the
ten-card tool-family budget, so `reference.card` never points at omitted proof.

### Phase 4 output-profile and trust contract

Terminal, JSON, HTML, and Markdown are labeled `standard`; exact-path diagnostics are
`private`; archive files are `private-content`; and the card is `share`. The compatibility
`render_share_card(&Report) -> String` signature remains available, but it immediately
projects into an internal serializable aggregate-only DTO. The card template accepts that
DTO rather than `Report`, so its field surface cannot carry project/session/request/account
identity, paths, prompts, content, commands, or diagnostic samples.

One shared trust projection supplies profile, `ccwrapped.report/v2`, selected period,
timezone, completeness, local API-equivalent provenance and cost coverage, registry/method,
and limitations. Terminal, HTML, Markdown, and card render its exact lines; JSON supplies
the same values through `schemaVersion`, `dataCoverage`, canonical cost, and methodology
fields. Partial and indeterminate histories open as `observed activity`; only complete
coverage uses the concise Wrapped/year opening.

HTML and Markdown neutralize markup, control characters, Unicode line separators, and bidi
formatting at their sinks. Terminal label/rule/padding widths use Unicode display columns.
The sole Phase 4 dependency addition is locked `unicode-width 0.2.2`; the rest of the
Phase 3 dependency graph remains pinned.

### Phase 2 metric migration

| Baseline/current compatibility surface | Canonical Phase 2 behavior | Migration disposition |
| --- | --- | --- |
| `costAnalysis.totalCost` and daily/model costs; session `costUsd` | `canonicalMetrics.cost` separates source-recorded estimate, local API-equivalent estimate, billing-authoritative unavailable, and unpriced coverage. | Use canonical cost fields for new consumers. Legacy total/day/model values are consistently local API-equivalent; session `costUsd` is consistently source-recorded. They are never mixed or labeled billing/actual spend. |
| `cacheHealth` and public `analyze_cache_health` / `detect_inflection_points` | `canonicalMetrics.cache` exposes documented read/write shares and direct compaction count. Legacy and direct public causal/grade/ratio/monetary/inflection claims are neutral `N/A`/zero/empty/null values while directly observed token totals remain available. | Migrate display and analysis to canonical cache facts; no source-compatible causal replacement exists. |
| Session `durationMinutes` | `elapsedSeconds`, `activeSeconds`, and `inclusiveActiveSeconds` distinguish raw elapsed span from capped unioned activity. | `durationMinutes` remains the elapsed-span compatibility projection. |
| Numeric token accumulators | `canonicalMetrics.tokens` preserves category presence, unit, samples, availability, method ID, overflow state, and limitations across dimensions. | Read canonical values when absent versus explicit zero, partial coverage, or saturation matters. |
| Registry label without offline record detail | `methodology.pricingRegistry.records` is a deterministic complete inventory of provider/model aliases, effective bounds, modifier, citation/date, and integer pico-USD-per-token input/output/cache rates. | Persist the report itself to reproduce which embedded records its registry version represented; unknown provider/model/interval/modifier combinations stay unpriced. |
| UTC compatibility helpers | Canonical report construction uses the selected `TimeContext`; frozen public timestamp helper signatures retain their documented UTC behavior. | Existing direct callers remain source-compatible; report consumers use `dataCoverage.timezone` and methodology. |
| Independently recomputed renderer values | Terminal, HTML, Markdown, and card expose identical canonical `FACT` lines; JSON supplies the structured source. | Treat JSON proof objects as authoritative and renderer lines as equivalent projections. |

The Phase 2 additions remain within `ccwrapped.report/v2`; they are additive fields plus
documented semantic neutralization of unsupported legacy claims. Both immutable Phase 0
capture hashes remain unchanged.

### Phase 1 reader migration

| Baseline surface | Phase 1 behavior | Migration disposition |
| --- | --- | --- |
| `read_all_jsonl(path, year) -> Vec<AssistantEntry>` | Same signature; delegates to bounded normalized ingestion and returns only privacy-safe aliases/classifications. Safe stderr diagnostics expose partial/fatal fallback. | Existing code compiles. Prefer `try_read_all_jsonl` to receive `DataCoverage` or `IngestionReadError`. |
| `read_session_breakdown(path, year) -> SessionBreakdown` | Same signature; delegates to the same normalized adapter and removes paths, prompt content, and raw identifiers. | Existing code compiles. Prefer `try_read_session_breakdown` for coverage/error handling. |
| `year: None` on either reader | Includes all observed UTC years and reports selected period `all` through the fallible API. | Baseline all-years meaning preserved. |
| `discover_jsonl_files(path) -> Vec<PathBuf>` | Same signature; delegates to bounded, canonical, root-confined normalized discovery. Safe stderr reports a fatal/partial fallback. | Existing code compiles. Prefer `try_discover_jsonl_files` to distinguish empty input from failure. |
| `discover_session_files(path) -> Vec<PathBuf>` | Same direct-session scope, now backed by the same bounded traversal and stable-snapshot checks. | Existing code compiles. Prefer `try_discover_session_files` for typed failure handling. |
| `timestamp_year(timestamp)` / `timestamp_date_key(timestamp)` / `timestamp_hour(timestamp)` | Same signatures; require valid RFC3339 and derive the calendar bucket from the UTC instant instead of trusting a raw prefix, source offset, or ambient process timezone. | Existing code compiles. Malformed values return `None`; offset-boundary values move to the truthful UTC bucket. Phase 2 replaces the fixed-hour UTC contract with the selected IANA timezone. |
| equal-ranked project paths, tools, and power hours | Count/token rankings use an explicit public tie policy: lexical ascending identity, and earliest hour for hour buckets. | Identical facts serialize identically across fresh hash seeds and process environments. `sessionIntel.topTools` and `wrappedStory.topTool` name the same winner. |
| public story/terminal/HTML functions with caller-constructed timestamps and maximum project tokens | Display dates require valid RFC3339 and fall back safely; terminal values are inert; HTML project percentages widen before multiplication. | Legal malformed UTF-8 strings and `u64::MAX` values no longer panic or emit terminal control sequences. |
| `readers::wire::{JsonlRecord, JsonlMessage, JsonlUsage}` | Documented legacy passive deserialization DTOs; no active production path consumes them. | Retained without a Rust deprecation attribute because the pinned baseline extractor treats that attribute as a removal. Migrate to a `try_read_*` projection; removal requires a declared version bump. |

The reader correction intentionally changes unsafe baseline values: public projections no
longer return raw session IDs, encoded project paths/hashes, `cwd`, prompt content, arbitrary
model labels, or arbitrary tool names. This is the same privacy correction already declared
for report v2, not a promise to preserve sensitive values. The fallible functions make malformed,
oversized, skipped, and unreadable input inspectable without adding fields to the frozen return
types.

## Public Rust contract

[`baseline/public-api-v0.2.0.txt`](baseline/public-api-v0.2.0.txt) is the
machine-comparable, fully qualified rustdoc signature inventory generated by
[`scripts/capture-public-api.sh`](../scripts/capture-public-api.sh). It records public
modules/reexports, function and struct declarations with parameter/return/field types,
owned inherent methods, and explicit/derived/auto public trait implementations. Its source
artifacts are generated with isolated `cargo doc --lib` so external type links retain
qualified identity. A re-exported dependency item is read only from the same fresh generated
`doc/` tree and emitted under each reachable `ccwrapped::...` alias; its declaration,
methods, constants, and trait implementations therefore participate in the comparison.
The immutable baseline remains the semver comparison origin.
[`current/public-api-v0.3.0.txt`](current/public-api-v0.3.0.txt) is the checked authority
for the complete current surface. The compatibility test regenerates that artifact from the
current source-only tree, requires byte-for-byte equality, and then applies exact
fully-qualified addition/removal classifications against the baseline.

Because rustdoc HTML, Serde execution, and compiler-generated implementations are
toolchain-sensitive, both machine-generated artifacts pin and record rustc `1.95.0`
commit `59807616e1fa2540724bfbac14d7976d7e4a3860` and Cargo `1.95.0` commit
`f2d3ce0bd7f24a49f8f72d9000448f8838c4e850`; the public Rust artifact also pins rustdoc
`1.95.0` commit `59807616e1fa2540724bfbac14d7976d7e4a3860`. Both record target
`x86_64-unknown-linux-gnu` and fail explicitly on a mismatch. Each generator resolves
the validated executables to absolute paths, forces Cargo to use them, and neutralizes
ambient compiler, rustdoc, wrapper, flag, and target overrides. Public extractor v11 omits
compiler-internal `Freeze` and `UnsafeUnpin` implementations while retaining stable
public/auto-trait compatibility facts, including inherent associated constants and trait
implementations whose `Self` is a reference or another compound type containing the
public nominal type.
The CI verification job installs this exact 1.95.0 toolchain and runs all targets; it does not
delegate compatibility capture to the moving `stable` channel.

The baseline package declares no Cargo features, so both artifacts record
`default-no-package-features`. Either generator fails closed if a package feature appears;
the feature must first be assigned to an explicit supported-surface capture matrix. This
prevents default-only rustdoc or Serde execution from silently omitting a supported
feature-gated contract.

Both generators accept `CCWRAPPED_CAPTURE_REVISION=<40-hex-revision>` as an explicit
source-revision claim. With Git metadata, the claim must resolve to that exact commit; it
may precede `HEAD` when later commits leave every captured product input unchanged.
`Cargo.toml`, `Cargo.lock`, `src/`, and every locked local workspace/path package must
match the claim; dirty or untracked product inputs fail. Local packages outside the
repository boundary are rejected. Local build scripts and Rust `include!`, `include_str!`,
`include_bytes!`, or `#[path]` directives are rejected because they can consume files
outside the mechanically enumerated closure. Symlinks are likewise rejected in every root
or local-package compiler input rather than following an unversioned external target.
Without the variable, capture uses the current Git revision. A source archive without
`.git` must set either a 40-hex externally asserted revision or the literal `unverified`
for a current-tree artifact that has no honest commit identity. Current generator output records
`source-revision-status` as `verified-commit`, `externally-asserted-gitless`, or
`unverified-gitless`; current checked captures use the last value instead of borrowing
the Phase 0 baseline revision. The deterministic `source-tree-sha256` remains the
mechanical identity in every mode, so an archive cannot silently substitute different
source beneath the same artifact. Before metadata inspection or compilation, each generator
copies the repository's regular files into a private snapshot while excluding VCS metadata
and build-output directories. Symlinks in that snapshot fail closed. Git comparison,
metadata validation, dep-info closure checks, hashing, compilation, and extraction all use
the frozen copy; a concurrent edit to the live checkout therefore cannot change the bytes
described by an artifact. Framed path/type/content digests of the live tree before copying,
the private tree, and the live tree after copying must all agree, so a Git-less archive that
changes during snapshot creation fails instead of producing a mixed-time tree. A second
deterministic digest binds the extractor: the capture
script for the public API, and both the script and executable Serde example for the report
fixture inventory. The running script is byte-checked before snapshotting and again before
publication. Byte-equality and concurrent-mutation tests pass the Phase 0 baseline revision
explicitly.

Every Cargo invocation uses `--locked`; a stale or absent lockfile fails without mutation
instead of resolving a different dependency graph beneath the same baseline metadata.
Capture also runs offline from cache-only dependency state, in a fresh working directory
and Cargo home, under an allowlisted environment. Phase 0 capture supports the current
crates.io sparse-index graph and stages only sparse-index entries and archives named by
`Cargo.lock`; unrelated user cache entries are never copied. Non-crates.io registries and
locked Git sources fail closed until an equally bounded staging contract is implemented.
The private Cargo home never receives ambient unpacked registry sources or Git checkouts.
Every active registry archive is SHA-256 checked
against its exact `Cargo.lock` checksum in the cache namespace from which metadata
materialized that package, so identical crate filenames in unrelated registries do not
collide. Repository `.cargo/config*` is rejected;
ancestor/user configuration, aliases, runners, source replacement, and ambient `CARGO_*`
channels therefore cannot change a captured build.

Dependency auditing traverses package IDs reachable from the pinned root in Cargo's
target-filtered resolve graph rather than treating the unfiltered metadata package list as
active. The public-library capture excludes dev-only edges; the executable report-example
capture retains them. Target-inactive registry packages therefore need no archive on the
pinned Linux host and cannot create false capture failures.

Compiler inputs are validated from pinned-rustc dep-info rather than source-text patterns.
Before rustdoc or Serde extraction, a fresh `cargo check` enumerates the actual root,
example, and local-package inputs. Every local-target dep-info path must resolve to the
already hashed product closure (plus the explicitly extractor-hashed Serde example), so
comment-interrupted macros and other legal Rust syntax cannot add an unbound file.
Pinned Cargo `compiler-artifact` JSON binds each hashed dep-info filename to its package
manifest directory; ownership therefore remains exact when distinct local packages legally
use the same target name.
Because stable rustdoc HTML builds do not emit dep-info, public capture also performs a
pinned check with rustdoc's built-in `cfg(doc)` before documentation generation and validates
that closure. Doc-only modules and include macros therefore cannot escape the product digest;
ambient rustdoc flags remain disabled.

Local build scripts remain unsupported. The seven build scripts in the pinned host-filtered
registry graph are accepted only by exact package name, version, registry source, and
source SHA-256 after source inspection confirmed that they depend solely on Cargo-provided
package/target paths, the pinned compiler/target/features, and their own locked sources.
The sorted accepted inventory is hashed into both artifacts. Any new, changed, Git-sourced,
or otherwise unreviewed dependency build script fails before compilation; expanding the
allowlist requires a new source audit and regression. Separately, every regular file in each
freshly materialized registry dependency source tree is path- and content-hashed into
`dependency-sources-sha256`, binding ordinary dependency code and build-script helpers as
well as top-level build scripts.

Git discovery and provenance checks likewise run through an absolute system Git executable
under an empty allowlisted environment. Discovery is bounded to the repository root; the
resolved Git directory and the actual repository root are then passed explicitly as
`--git-dir` and `--work-tree`. Replace objects, external diffs/text conversion, fsmonitor,
ambient configuration, and inherited `GIT_*` redirection cannot substitute another tree.

Only the designated checked baseline artifact may be replaced. Any other existing output
path is rejected before capture, preventing an output argument from overwriting source,
configuration, extractor, or unrelated user files.

The root publishes `analyzers`, `fmt`, `readers`, `renderers`, and `report`, and
glob-reexports `fmt::*` and `report::*`. The separate report field artifact captures the
camelCase serialization contract in a compact field-oriented form.

Phase 1 additionally publishes `readers::IngestionReadError`,
`readers::jsonl::try_read_all_jsonl`, and
`readers::session::try_read_session_breakdown`, plus
`readers::discovery::{try_discover_jsonl_files, try_discover_session_files}`. The compatibility
regression admits those exact additions while continuing to reject every unclassified removal
or signature change.

Phase 4 additionally publishes
`renderers::terminal::{try_render_terminal, try_render_terminal_with,
try_render_terminal_to}`. These return `io::Result<()>` so callers can handle broken pipes
and other writer failures. The original `render_terminal*` functions retain their exact
signatures as compatibility shims and panic on an I/O failure, matching their historical
infallible contract.

Phase 6 regenerates both normalized current text artifacts with the pinned capture
toolchain, compares them bytewise, and classifies every delta. Existing names remain as
truthful compatibility shims where feasible. A
[`examples/public_api_consumer.rs`](../examples/public_api_consumer.rs) is the downstream-style
compile fixture for supported construction, analysis, rendering, and serialization. An
unavoidable signature/removal requires an explicit version bump and migration disposition.

## Visual/output baseline

The repository contains five polished visual baselines:
`hero-slide.png`, `spend-slide.png`, `cache-slide.png`, `data-slide.png`, and
`share-card.png`. The hero/data slides establish large editorial typography, dense but
legible comparisons, and a dark Spotify-like Wrapped personality. The share card
demonstrates an aggregate-only surface. Phase 4 preserves that hierarchy while removing
unsupported or private claims.

The five files are executable product evidence, not hand-edited marketing art. They are
captured from `tests/fixtures/readme-assets` through the production HTML and card renderers
by `scripts/generate-readme-assets.sh`. `assets/README-ASSETS.sha256` pins the synthetic
fixture, all Rust renderer inputs, the generator, and the PNGs; the integration suite
rejects any unpaired drift. The Phase 3 narrative gate discovers the checked PNG set from
that manifest and scans all five with Tesseract 5.x OCR, requiring a per-image text canary
before applying the forbidden-claim vocabulary.

## Baseline command contract

The supported build floor is Rust 1.85, declared by `package.rust-version` and checked
with `cargo check --all-targets --locked`. Compatibility captures and release binaries
use the repository-pinned Rust/Cargo 1.95.0 toolchain. CI also requires the complete
locked test suite, formatting, strict all-target/all-feature Clippy, ShellCheck, and the
README asset manifest to pass without exceptions.
