# Phase 5 performance amendment — production throughput

Goal: Select and verify the worker policy that gives users the fastest stable product on the
measured host while preserving honest resource telemetry.

Success means:

- Production auto-selection uses the stable scaling point within 2% of the fastest measured
  median on the topology-valid `1, 2, 4, 8, 12, 15` curve.
- The large-corpus record retains measured duration, CPU utilization, variance, RSS, worker
  count, batch size, queue depth, affinity, and the original 30-second/80% gate outcome.
- The benchmark gate accepts the selected throughput point when five post-warmup runs remain
  stable at at most 10% wall/utilization coefficient of variation and at most 4 GiB peak RSS.
- Documentation labels the original duration/utilization result as observed and ungated under
  this amendment.
- Every Phase 5 correctness, determinism, privacy, recovery, incremental-work, startup,
  first-import, store-size, and report-equality gate remains in force.

Stop when: One source-matched benchmark record proves the selected worker point is within 2% of
the fastest stable median, all retained product gates pass, and fresh reviewers receive this
amendment with the original rubric.

## Authority and scope

- Approved: 2026-07-19.
- Authority: the user explicitly directed the run to continue because the throughput-oriented
  implementation is purely better and designated the prior compute-utilization contract as
  observed telemetry rather than a blocking target.
- Amendment ID: `phase5/performance-throughput-amendment/v1`.
- Amended objective: `production-throughput-plateau`.
- Original objective retained as telemetry: 30-second continuous duration and 80% selected-CPU
  utilization.

## Direction for implementation and review

1. Measure every scaling point from the same source snapshot, driver, generator, support binary,
   corpus, repetition count, and host allowance.
2. Select the smallest worker count whose stable median lies within 2% of the fastest stable
   point.
3. Preserve `continuousDurationGate`, `utilizationGate`, and `originalContractPassed` as
   mechanically derived fields.
4. Gate accepted saturation evidence on stable repeated measurements and the 4 GiB RSS bound.
5. Report `benchmark_throughput=PASS` when the selected point satisfies the curve rule.
6. Report `benchmark_utilization=OBSERVED_NOT_GATED` so automated and human readers receive the
   amended meaning directly.
7. Apply the original Phase 5 rubric to every criterion outside this narrowly amended
   duration/utilization stopping condition.

## Accepted selection evidence

The accepted source-matched six-point decision curve measured stable medians of 4.373, 3.180,
2.720, 2.159, 2.032, and 2.043 seconds for 1, 2, 4, 8, 12, and 15 workers. The
1/2/4-worker points and paired 8/12/15 plateau point used the same source snapshot, binaries,
corpus, run count, and full `0-15` affinity allowance. Twelve workers are the fastest point,
0.54% ahead of 15 workers; 8 workers are 6.24% slower than the fastest point. Twelve workers
are therefore the smallest measured stable point within 2% of the fastest median.
