# Phase 5 benchmark decision

The frozen `phase5/v1` rubric selects the incremental SQLite branch. The user-approved
`rubricAmendment` `phase5/performance-throughput-amendment/v1` selects the stable
production-throughput plateau while retaining the original duration/utilization outcomes as
telemetry. The machine-readable authority is
[`phase5-record.json`](phase5-record.json).

> Historical-snapshot notice (2026-07-19): this record remains an immutable measurement of
> the exact release binary identified below. Subsequent correctness, privacy, persistence,
> and pricing work changed the product binary without re-running this timing campaign.
> Performance is informational and does not block the current release; current correctness,
> equality, recovery, and privacy claims are established by the source-current test suite.

## Exact source snapshot

- Release binary SHA-256:
  `621c4ec33f5afbcf795977a01a77437fb3250c8df81f1fc572d8b94029e2f955`.
- `measurementDriverSha256`:
  `257233e31967b272ab847e2d8ba8d1a7d81b222f114e81fc051f0a8b8fd0af61`.
- `verificationDriverSha256`:
  `257233e31967b272ab847e2d8ba8d1a7d81b222f114e81fc051f0a8b8fd0af61`.
- Generator source SHA-256:
  `f7ad31621b2c06ab5e7e3fea0dfd44d1b0ee07d6901d50b83c41f5cc35196cc1`.
- Support binary SHA-256:
  `19fb5dfc7a289e4c7e5fa4230a244c30622cd7897409385ebdd17f50bc42d01f`.
- Rust/Cargo: 1.95.0, target `x86_64-unknown-linux-gnu`.
- Host: Intel Core Ultra 7 255H, 16 logical CPUs, one socket/NUMA node.
- Synthetic seed: `0xc5c5_2026_0717_0001`.
- `productionAutoWorkers`: 12; one file per batch; result queue capacity: 24.

The exact corpus lineage is:

| Corpus | `generatorVersion` / seed | `manifestSha256` | `sourceManifestSha256` | Files / bytes |
| --- | --- | --- | --- | ---: |
| `oracle-small` | `phase5-corpus/2.0.0` / `0xc5c5_2026_0717_0001` | `05d48d72ecc2d10589c858876fad80f72021c3a84c6db5fa07eb6e786854b943` | `32402d3b5f27f83c8e767b97b8a788fa6477bbae2435388d5f08ae19f8f7e3aa` | 32 transcript + 4 OTel / 12,904,753 |
| `decision` | `phase5-corpus/2.0.0` / `0xc5c5_2026_0717_0001` | `f5c01306f86af79dc911bb579bfa121ebb959b82df8efd6709f503b92eb9e3d3` | `24b51b5c6bb681d3abae4224b0eea232f0c99a77e742ca00375e2a0f8533b4ea` | 4,096 transcript + 16 OTel / 536,461,603 |
| `saturation-large` | `phase5-corpus/2.0.0` / `0xc5c5_2026_0717_0001` | `b6a65fec9bcc4e4783651f5f6d96997bf5f311be875584c6ed501fa1f5e80361` | `05df0e594b797365ea891646191d556a12d48851fe2da8c85a3183403a9e5ead` | 16,384 transcript + 64 OTel / 3,755,841,597 |

The generator's exact shape and independent oracle are:

| Corpus | Physical / `classifiedRecords` | Normalized / accepted / `canonicalRecords` | Distribution: malformed / unsupported / unknown / filtered / duplicate / resolved overlap / unresolved overlap | `metricOracle`: points / accepted / filtered / delta / cumulative / reset / gap / overlap |
| --- | ---: | ---: | ---: | ---: |
| `oracle-small` | 14,800 / 14,800 | 6,862 / 6,222 / 4,234 | 128 / 128 / 128 / 7,682 / 640 / 4 / 1,984 | 12 / 10 / 2 / 6 / 6 / 2 / 4 / 2 |
| `decision` | 780,800 / 780,800 | 272,888 / 231,928 / 167,976 | 8,192 / 8,192 / 8,192 / 491,528 / 40,960 / 16 / 63,936 | 48 / 40 / 8 / 24 / 24 / 8 / 16 / 8 |
| `saturation-large` | 11,123,200 / 11,123,200 | 899,552 / 735,712 / 671,904 | 114,688 / 114,688 / 114,688 / 9,994,272 / 163,840 / 64 / 63,744 | 192 / 160 / 32 / 96 / 96 / 32 / 64 / 32 |

Every corpus contains deterministic synthetic data. Generation time is excluded. Each timed
configuration has three warmups and at least five measured repetitions; startup has 20.
The 617.681-second campaign duration sums the 26 accepted raw roots' post-warmup measured
invocations. Warmups remain in those roots. The record retains every wall sample, CPU
time/utilization, RSS, logical/physical I/O, source-byte/file counters, store allocation,
sample deviation, coefficient of variation, and exhaustive artifact-manifest digest.

## Results

| Workload | Primary median | Confirmation median | Primary/confirmation CoV | Result |
| --- | ---: | ---: | ---: | --- |
| `oracle-small`, no store | 65.92 ms | 59.26 ms | 14.08% / 4.53% | p95 89.39 / 62.05 ms |
| `oracle-small`, warm default | 6.50 ms | 4.34 ms | 17.24% / 17.56% | p95 8.89 / 6.43 ms |
| `decision`, no store (12 workers) | 2.032 s | 2.059 s | 1.81% / 1.59% | branch baseline |
| `decision`, first import | 2.433 s | 2.402 s | 7.52% / 1.76% | 1.197× / 1.167× baseline |
| `decision`, warm no change | 132.4 ms | 132.5 ms | 3.16% / 3.00% | 15.35× / 15.53× speedup |
| `decision`, incremental tail | 488.7 ms | 449.9 ms | 4.48% / 1.54% | 21.55% / 22.65% of matched full scan; exact equality |
| `saturation-large`, no store | 18.223 s | 22.689 s | 0.96% / 3.62% | throughput telemetry |
| `saturation-large`, first import | 19.927 s | 20.717 s | 2.78% / 3.08% | bounded import path |
| `saturation-large`, warm no change | 531.3 ms | 534.8 ms | 1.66% / 1.52% | cached-report path |

The decision warm path reduces median latency by 1.900–1.926 seconds and exceeds both branch
thresholds: at least 4× speedup and at least 750 ms absolute reduction. A separate
branch-selection run proves byte-identical no-store/SQLite JSON, a 15.17× warm speedup, and a
1.199× first-import ratio. Decision stores allocate at most 42,635,264 bytes. Large stores
allocate at most 149,004,288 bytes, below both source size and the 2 GiB ceiling.

Both incremental series read 449,668 source bytes, parse exactly four changed plus four new
files, and produce byte-identical JSON to clean no-store scans. They complete in 450–489 ms
while preserving the FULL-synchronous publication transaction and every F053–F057 recovery
test.

## Bottleneck and throughput selection

The measured bottleneck is JSON parsing CPU:

- transcript parsing consumes 13.791 / 16.500-second medians of the 17.484 / 21.890-second
  median ingestion pipelines on `saturation-large`;
- physical source reads are zero after warmups, while the independent same-filesystem reader
  reaches a 0.877 GB/s median;
- the 12-worker memory baseline reaches a 63.91 GB/s lower-bound traffic median, far above
  product source throughput;
- primary and confirmation large-corpus medians are 18.223 / 22.689 seconds with 74.06% /
  65.76% allocated-CPU utilization.

Production processes one file per batch, permits at most two queued results per worker, and
merges source results deterministically. The source-matched scaling evidence is:

| Workers | Median | Wall CoV | Relative to one worker |
| ---: | ---: | ---: | ---: |
| 1 | 4.373 s | 1.06% | 1.00× |
| 2 | 3.180 s | 6.48% | 1.38× |
| 4 | 2.720 s | 1.64% | 1.61× |
| 8 | 2.159 s | 5.80% | 2.03× |
| 12 | 2.032 s | 1.81% | 2.15× |
| 15 | 2.043 s | 1.38% | 2.14× |

The 8/12/15 points come from one paired temporal window. Twelve workers are the fastest point,
0.54% ahead of 15 workers; 8 workers are 6.24% slower than the fastest point. Twelve workers
are therefore the smallest point within the amended 2% throughput plateau. Production selects
12 workers and a 24-result queue.

The original 30-second/80% criterion remains mechanically reported:
`continuousDurationGate=false`, `utilizationGate=false`, and
`originalContractPassed=false` in both large-corpus confirmations. Under the user-approved
amendment, those observations are informative telemetry; the blocking performance result is
stable near-fastest throughput within 4 GiB RSS. The verifier prints the distinction directly.

Reproduce the accepted large-corpus measurement:

```bash
taskset -c 0-11 scripts/phase5-benchmark.sh saturate \
  --class saturation-large --workers 12 --runs 5 --warmups 3 --sample-ms 10
```

Equivalent live observation can use `pidstat -u -p PID 1`, `top -H -p PID`, or
`perf stat -p PID` when available. The authoritative record uses monotonic sampling plus
`/proc/<pid>/{stat,status,io}`. Five sampler-overhead runs place median mean poll cost at
14.790 microseconds and maximum per-run p95 at 17.343 microseconds.

## Variance and limitations

The host retained unrelated workloads, primarily on CPUs 12–15. Accepted incremental and
large-corpus runs used CPUs 0–11; the paired plateau used the full 0–15 allowance. Host noise
and filesystem-cache warm-up produced two excluded incremental-confirmation retries. A
separate invalid determinism attempt requested 15 workers under a 12-CPU affinity mask and
was rejected before acceptance. Each failed attempt remains in the external benchmark root;
no sample was removed from an accepted series. Every accepted series governed by the
repeated-run stability rule stays within 10% wall and utilization CoV.

Warm startup lasts 4–9 ms and is visibly quantized by the 2 ms sampler/scheduler interval, so
its 17–18% CoV is assessed through the rubric's 20-run p95 gate; both p95 values remain below
9 ms versus the 200 ms limit. Btrfs compression and a warm page cache make physical-read
counters unsuitable as a capacity denominator. The filesystem and memory baselines therefore
remain comparative evidence rather than a fabricated physical-cold claim.

## Historical verification

```bash
scripts/phase5-benchmark.sh verify-record
scripts/phase5-benchmark.sh verify-limits
scripts/phase5-benchmark.sh verify-utilization
```

These commands verify the retained campaign only when the pinned binary and external raw
roots are available. They are not source-current release gates. `verify-record` rehashes
exhaustive artifact manifests, checks run provenance, binds each raw summary to its record
path, derives the composite scale/memory/store series, probes the recorded worker policy, and
recomputes campaign wall time. `verify-limits` recomputes the historical startup, branch,
incremental, RSS, and store gates from raw aggregates. The compatibility command
`verify-utilization` recomputes the retained throughput plateau and prints:

```text
benchmark_throughput=PASS
benchmark_utilization=OBSERVED_NOT_GATED
```
