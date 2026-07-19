#!/usr/bin/env bash
set -euo pipefail

export LC_ALL=C

repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
cache_home=${XDG_CACHE_HOME:-${HOME:?HOME must be set}/.cache}
bench_root=${CCWRAPPED_PHASE5_ROOT:-$cache_home/ccwrapped-phase5-v2}
support_manifest=$repo_root/tests/support/phase5-bench/Cargo.toml
support_binary=$repo_root/tests/support/phase5-bench/target/release/phase5-bench
product_binary=$repo_root/target/release/ccwrapped
command_name=${1:-}

usage() {
  printf 'usage: %s preflight\n' "$0" >&2
  printf '       %s generate --class CLASS [--target-bytes BYTES]\n' "$0" >&2
  printf '       %s oracle --class oracle-small\n' "$0" >&2
  printf '       %s baseline --class CLASS [--runs N] [--workers N] [--sample-ms N]\n' "$0" >&2
  printf '       %s scale [--class CLASS] --workers CSV [--runs N] [--warmups N] [--sample-ms N]\n' "$0" >&2
  printf '       %s saturate [--class CLASS] [--workers N] [--runs N] [--warmups N] [--sample-ms N]\n' "$0" >&2
  printf '       %s determinism --class CLASS --workers CSV\n' "$0" >&2
  printf '       %s branch [--class decision] [--workers N] [--runs N] [--warmups N] [--sample-ms N]\n' "$0" >&2
  printf '       %s startup [--class oracle-small] [--workers N] [--runs N] [--warmups N] [--sample-ms N]\n' "$0" >&2
  printf '       %s warm-store --class CLASS --workers N [--runs N] [--warmups N] [--sample-ms N]\n' "$0" >&2
  printf '       %s first-import-point --class CLASS [--workers N] [--runs N] [--warmups N] [--sample-ms N]\n' "$0" >&2
  printf '       %s first-import --class CLASS [--workers N] [--runs N] [--warmups N] [--sample-ms N]\n' "$0" >&2
  printf '       %s incremental [--class decision] [--workers N] [--runs N] [--sample-ms N]\n' "$0" >&2
  printf '       %s reader --class CLASS [--runs N] [--passes N] [--buffer-bytes N]\n' "$0" >&2
  printf '       %s memory --workers CSV [--runs N] [--warmups N] [--bytes-per-worker N] [--passes N]\n' "$0" >&2
  printf '       %s instrument-overhead [--runs N] [--iterations N]\n' "$0" >&2
  printf '       %s verify-record|verify-limits|verify-utilization\n' "$0" >&2
  exit 2
}

require_positive_integer() {
  local name=$1
  local value=$2
  if [[ ! $value =~ ^[1-9][0-9]*$ ]]; then
    printf '%s must be a positive integer: %s\n' "$name" "$value" >&2
    exit 2
  fi
}

build_bench_binaries() {
  cargo build \
    --manifest-path "$support_manifest" \
    --release \
    --locked \
    --offline
  cargo build \
    --manifest-path "$repo_root/Cargo.toml" \
    --release \
    --locked \
    --offline
}

verify_corpus() {
  local corpus=$1
  if [[ ! -d $corpus/projects || ! -d $corpus/otel || ! -f $corpus/manifest.json ]]; then
    printf 'missing generated corpus: %s\n' "$corpus" >&2
    exit 1
  fi
  (
    cd "$corpus"
    sha256sum --check SOURCE-MANIFEST.sha256 >/dev/null
  )
}

new_run_root() {
  local mode=$1
  local class=$2
  local run_id=${CCWRAPPED_PHASE5_RUN_ID:-$(date -u +%Y%m%dT%H%M%SZ)-$$}
  local output=$bench_root/runs/$mode-$class-$run_id
  if [[ -e $output ]]; then
    printf 'benchmark run output already exists: %s\n' "$output" >&2
    exit 1
  fi
  mkdir -p "$output"
  jq -n \
    --arg command "$command_name" \
    --arg measurement_driver_sha256 "$(
      sha256sum "$repo_root/scripts/phase5-benchmark.sh" | awk '{print $1}'
    )" \
    --arg generator_source_sha256 "$(
      sha256sum "$repo_root/tests/support/phase5-bench/src/generator.rs" | awk '{print $1}'
    )" \
    --arg product_binary_sha256 "$(
      sha256sum "$product_binary" | awk '{print $1}'
    )" \
    --arg support_binary_sha256 "$(
      sha256sum "$support_binary" | awk '{print $1}'
    )" \
    '{
      schema: "ccwrapped.phase5-run-provenance/v1",
      command: $command,
      measurementDriverSha256: $measurement_driver_sha256,
      generatorSourceSha256: $generator_source_sha256,
      productBinarySha256: $product_binary_sha256,
      supportBinarySha256: $support_binary_sha256
    }' >"$output/RUN-PROVENANCE.json"
  printf '%s\n' "$output"
}

run_product_sample() {
  local corpus=$1
  local workers=$2
  local sample_ms=$3
  local sample=$4
  local stderr_log=$5
  local scratch=$6
  local mode=$7
  local store=${8:-}
  local args=(
    measure
    --binary "$product_binary" \
    --corpus "$corpus" \
    --stderr "$stderr_log" \
    --scratch "$scratch" \
    --sample-ms "$sample_ms" \
    --timeout-seconds 300 \
    --workers "$workers" \
    --mode "$mode"
  )
  if [[ -n $store ]]; then
    args+=(--store "$store")
  fi
  "$support_binary" "${args[@]}" >"$sample"
  jq -e \
    --argjson workers "$workers" \
    --arg mode "$mode" \
    '
      .success == true
      and .timedOut == false
      and .mode == $mode
      and .workerCount == $workers
      and .stageCounters.schema == "ccwrapped.ingestion-performance/v1"
      and
        ((.stageCounters.incrementalCheckpointStatus == "report-cache-hit")
         or (.stageCounters.selectedWorkers == $workers))
    ' \
    "$sample" >/dev/null
  test ! -s "$stderr_log"
}

summarize_measurements() {
  local class=$1
  local workers=$2
  local warmups=$3
  local source_bytes=$4
  local output=$5
  shift 5
  jq -s \
    --arg class "$class" \
    --argjson workers "$workers" \
    --argjson warmups "$warmups" \
    --argjson source_bytes "$source_bytes" \
    '
      def stats($values):
        ($values | length) as $count
        | ($values | sort) as $sorted
        | ($values | add / $count) as $mean
        | (if ($count % 2) == 1
           then $sorted[($count / 2 | floor)]
           else (($sorted[$count / 2 - 1] + $sorted[$count / 2]) / 2)
           end) as $median
        | $sorted[((0.95 * $count | ceil) - 1)] as $p95
        | (if $count <= 1
           then 0
           else (($values | map((. - $mean) * (. - $mean)) | add / ($count - 1)) | sqrt)
           end) as $stddev
        | {
            count: $count,
            mean: $mean,
            median: $median,
            p95: $p95,
            minimum: $sorted[0],
            maximum: $sorted[-1],
            sampleStddev: $stddev,
            coefficientOfVariation:
              (if $mean == 0 then null else ($stddev / $mean) end)
          };
      def stage_stats($samples):
        reduce [
          "discoveryNanos",
          "storeLoadNanos",
          "storePublishNanos",
          "transcriptParseNanos",
          "otelParseNanos",
          "metricFinalizeNanos",
          "sourceDedupNanos",
          "capabilityAggregationNanos",
          "authoritySelectionNanos",
          "analyticalCapabilityNanos",
          "canonicalProjectionNanos",
          "projectionActivityNanos",
          "projectionTokensNanos",
          "projectionCostNanos",
          "projectionCacheNanos",
          "projectionDailyNanos",
          "projectionProjectsNanos",
          "projectionSessionsNanos",
          "projectionMethodologyNanos",
          "projectionHourDistributionNanos",
          "projectionCompatibilityEntriesNanos",
          "insightBuildNanos",
          "ingestionTotalNanos",
          "reportBuildNanos",
          "reportSerializationNanos",
          "reportEntryProjectionNanos",
          "reportCostNanos",
          "reportCacheNanos",
          "reportSessionNanos",
          "reportModelRoutingNanos",
          "reportRecommendationNanos",
          "reportStoryNanos"
        ][] as $key
          ({};
           .[$key] = stats($samples | map(.stageCounters[$key])));
      . as $samples
      | {
          schema: "ccwrapped.phase5-scaling-point/v1",
          class: $class,
          mode: $samples[0].mode,
          workerCount: $workers,
          warmupCount: $warmups,
          measuredCount: ($samples | length),
          sourceBytes: $source_bytes,
          batchFiles: ($samples[0].stageCounters.batchFiles),
          resultQueueCapacity: ($samples[0].stageCounters.resultQueueCapacity),
          wallNanos: stats($samples | map(.wallNanos)),
          cpuSeconds: stats($samples | map(.cpuSeconds)),
          occupiedCpuCores:
            stats($samples | map(.cpuSeconds / (.wallNanos / 1000000000))),
          allocatedCpuUtilization:
            stats($samples | map(.allocatedCpuUtilization)),
          bytesPerSecond:
            stats($samples | map($source_bytes / (.wallNanos / 1000000000))),
          peakRssBytes: {
            maximum: ($samples | map(.peakRssBytes) | max),
            samples: ($samples | map(.peakRssBytes))
          },
          logicalReadBytes: stats($samples | map(.logicalReadBytes)),
          physicalReadBytes: stats($samples | map(.physicalReadBytes)),
          physicalWriteBytes: stats($samples | map(.physicalWriteBytes)),
          sourceContentBytesRead:
            stats($samples | map(.stageCounters.sourceContentBytesRead)),
          parsedSourceFiles:
            stats($samples | map(.stageCounters.parsedSourceFiles)),
          reusedSourceFiles:
            stats($samples | map(.stageCounters.reusedSourceFiles)),
          sampleCounts: ($samples | map(.sampleCount)),
          stageCounters: stage_stats($samples),
          rawAggregates:
            ($samples
             | map({
                 wallNanos,
                 cpuSeconds,
                 allocatedCpuUtilization,
                 peakRssBytes,
                 logicalReadBytes,
                 logicalWriteBytes,
                 physicalReadBytes,
                 physicalWriteBytes,
                 stageCounters
               })),
          stable:
            ((stats($samples | map(.wallNanos)).coefficientOfVariation <= 0.10)
             and
             (stats($samples | map(.allocatedCpuUtilization)).coefficientOfVariation <= 0.10))
        }
    ' \
    "$@" >"$output"
}

run_memory_sample() {
  local workers=$1
  local bytes_per_worker=$2
  local passes=$3
  local output=$4
  "$support_binary" memory-baseline \
    --workers "$workers" \
    --bytes-per-worker "$bytes_per_worker" \
    --passes "$passes" \
    >"$output"
  jq -e \
    --argjson workers "$workers" \
    --argjson bytes_per_worker "$bytes_per_worker" \
    --argjson passes "$passes" \
    '
      .schema == "ccwrapped.phase5-memory/v1"
      and .workerCount == $workers
      and .bytesPerWorker == $bytes_per_worker
      and .passes == $passes
      and .payloadBytesCopied > 0
      and .wallNanos > 0
    ' \
    "$output" >/dev/null
}

summarize_memory_measurements() {
  local workers=$1
  local warmups=$2
  local output=$3
  shift 3
  jq -s \
    --argjson workers "$workers" \
    --argjson warmups "$warmups" \
    '
      def stats($values):
        ($values | length) as $count
        | ($values | sort) as $sorted
        | ($values | add / $count) as $mean
        | (if ($count % 2) == 1
           then $sorted[($count / 2 | floor)]
           else (($sorted[$count / 2 - 1] + $sorted[$count / 2]) / 2)
           end) as $median
        | $sorted[((0.95 * $count | ceil) - 1)] as $p95
        | (if $count <= 1
           then 0
           else (($values | map((. - $mean) * (. - $mean)) | add / ($count - 1)) | sqrt)
           end) as $stddev
        | {
            count: $count,
            mean: $mean,
            median: $median,
            p95: $p95,
            minimum: $sorted[0],
            maximum: $sorted[-1],
            sampleStddev: $stddev,
            coefficientOfVariation:
              (if $mean == 0 then null else ($stddev / $mean) end)
          };
      . as $samples
      | {
          schema: "ccwrapped.phase5-memory-point/v1",
          workerCount: $workers,
          warmupCount: $warmups,
          measuredCount: ($samples | length),
          bytesPerWorker: $samples[0].bytesPerWorker,
          passes: $samples[0].passes,
          allocatedBytes: $samples[0].allocatedBytes,
          payloadBytesCopied: $samples[0].payloadBytesCopied,
          memoryTrafficBytesLowerBound: $samples[0].memoryTrafficBytesLowerBound,
          wallNanos: stats($samples | map(.wallNanos)),
          cpuSeconds: stats($samples | map(.cpuSeconds)),
          allocatedCpuUtilization:
            stats($samples | map(.allocatedCpuUtilization)),
          payloadBytesPerSecond:
            stats($samples | map(.payloadBytesPerSecond)),
          memoryTrafficBytesPerSecondLowerBound:
            stats($samples | map(.memoryTrafficBytesPerSecondLowerBound)),
          peakRssBytes: {
            maximum: ($samples | map(.peakRssBytes) | max),
            samples: ($samples | map(.peakRssBytes))
          },
          rawAggregates:
            ($samples
             | map({
                 wallNanos,
                 cpuSeconds,
                 allocatedCpuUtilization,
                 payloadBytesPerSecond,
                 memoryTrafficBytesPerSecondLowerBound,
                 peakRssBytes,
                 contentFingerprintFnv1a64
               })),
          stable:
            ((stats($samples | map(.wallNanos)).coefficientOfVariation <= 0.10)
             and
             (stats($samples | map(.allocatedCpuUtilization)).coefficientOfVariation <= 0.10))
        }
    ' \
    "$@" >"$output"
}

finalize_artifacts() {
  local run_root=$1
  local artifacts_tmp=$run_root.ARTIFACTS.sha256.tmp
  (
    cd "$run_root"
    find . -type f ! -name ARTIFACTS.sha256 -print0 |
      sort -z |
      xargs -0 sha256sum >"$artifacts_tmp"
    mv "$artifacts_tmp" ARTIFACTS.sha256
    sha256sum --check ARTIFACTS.sha256 >/dev/null
  )
}

run_canonical_report() {
  local corpus=$1
  local workers=$2
  local mode=$3
  local store=$4
  local output=$5
  local stderr_log=$6
  local scratch=$7
  local args=(
    --timezone UTC
    --data-dir "$corpus/projects"
    --ingestion-workers "$workers"
  )
  local path
  for path in "$corpus"/otel/*.jsonl; do
    args+=(--otel-file "$path")
  done
  if [[ $mode == store ]]; then
    args+=(--store-path "$store")
  elif [[ $mode == no-store ]]; then
    args+=(--no-store)
  else
    printf 'unsupported report mode: %s\n' "$mode" >&2
    exit 2
  fi
  mkdir -p "$scratch/home" "$scratch/config"
  HOME=$scratch/home \
    CLAUDE_CONFIG_DIR=$scratch/config \
    NO_COLOR=1 \
    "$product_binary" \
    "${args[@]}" \
    --json \
    2026 \
    >"$output" \
    2>"$stderr_log"
  test ! -s "$stderr_log"
}

verify_production_auto_workers() {
  local expected=$1
  local corpus=$bench_root/oracle-small
  local scratch
  verify_corpus "$corpus"
  scratch=$(mktemp -d "$bench_root/verify-auto-workers.XXXXXX")
  local args=(
    --timezone UTC
    --data-dir "$corpus/projects"
    --no-store
    --benchmark-counters "$scratch/counters.json"
  )
  local path
  for path in "$corpus"/otel/*.jsonl; do
    args+=(--otel-file "$path")
  done
  mkdir -p "$scratch/home" "$scratch/config"
  if ! HOME=$scratch/home \
    CLAUDE_CONFIG_DIR=$scratch/config \
    NO_COLOR=1 \
    "$product_binary" \
    "${args[@]}" \
    --json \
    2026 \
    >"$scratch/report.json" \
    2>"$scratch/stderr.log"; then
    rm -rf "$scratch"
    printf 'production auto-worker probe failed\n' >&2
    exit 1
  fi
  if [[ -s $scratch/stderr.log ]] ||
    ! jq -e \
      --argjson expected "$expected" \
      '
        .schema == "ccwrapped.ingestion-performance/v1"
        and .selectedWorkers == $expected
        and .transcriptWorkers == $expected
        and .batchFiles == 1
        and .resultQueueCapacity == ($expected * 2)
      ' \
      "$scratch/counters.json" >/dev/null; then
    rm -rf "$scratch"
    printf 'production auto-worker policy does not match benchmark selection\n' >&2
    exit 1
  fi
  rm -rf "$scratch"
}

store_allocation_bytes() {
  local store=$1
  local total=0
  local path blocks
  for path in "$store" "$store-wal" "$store-journal" "$store-shm"; do
    [[ -f $path ]] || continue
    blocks=$(stat -c %b "$path")
    total=$((total + blocks * 512))
  done
  printf '%s\n' "$total"
}

require_minimum_runs() {
  local name=$1
  local value=$2
  local minimum=$3
  require_positive_integer "$name" "$value"
  if ((value < minimum)); then
    printf '%s requires at least %s runs\n' "$name" "$minimum" >&2
    exit 2
  fi
}

if [[ $command_name == preflight ]]; then
  mkdir -p "$bench_root"
  printf 'timestamp=%s\n' "$(date --iso-8601=seconds)"
  printf 'repository=%s\n' "$repo_root"
  printf 'benchmark_root=%s\n' "$bench_root"
  printf 'rustc=%s\n' "$(rustc --version)"
  printf 'cargo=%s\n' "$(cargo --version)"
  printf 'affinity='
  taskset -pc $$ | sed 's/.*: //'
  printf 'cpuset='
  cat /sys/fs/cgroup/cpuset.cpus.effective 2>/dev/null || printf 'unavailable\n'
  printf 'cpu_max='
  cat /sys/fs/cgroup/cpu.max 2>/dev/null || printf 'unavailable\n'
  printf 'memory_available_bytes='
  awk '/MemAvailable:/ {printf "%.0f\n", $2 * 1024}' /proc/meminfo
  printf 'measurement_tools='
  for tool in hyperfine perf iostat pidstat strace fio nvtop nvidia-smi; do
    if command -v "$tool" >/dev/null 2>&1; then
      printf '%s:present ' "$tool"
    else
      printf '%s:absent ' "$tool"
    fi
  done
  printf '\n'
  findmnt -T "$bench_root" -o TARGET,SOURCE,FSTYPE,OPTIONS -n
  df -B1 "$bench_root"
  lscpu
  exit 0
fi

if [[ $command_name == generate ]]; then
  shift
  class=
  target_bytes=
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --target-bytes)
        (($# >= 2)) || usage
        target_bytes=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ -n $class ]] || usage

  output=$bench_root/$class
  mkdir -p "$bench_root"
  if [[ -e $output ]]; then
    printf 'benchmark output already exists: %s\n' "$output" >&2
    printf 'select a fresh CCWRAPPED_PHASE5_ROOT for immutable generation\n' >&2
    exit 1
  fi

  args=(generate --class "$class" --output "$output")
  if [[ -n $target_bytes ]]; then
    require_positive_integer target-bytes "$target_bytes"
    args+=(--target-bytes "$target_bytes")
  fi

  cargo run \
    --manifest-path "$support_manifest" \
    --release \
    --locked \
    --offline \
    -- "${args[@]}"

  (
    cd "$output"
    find projects otel -type f -print0 |
      sort -z |
      xargs -0 sha256sum >SOURCE-MANIFEST.sha256
    sha256sum --check SOURCE-MANIFEST.sha256 >/dev/null
  )
  printf 'generated=%s\n' "$output"
  exit 0
fi

if [[ $command_name == oracle ]]; then
  shift
  [[ ${1:-} == --class && ${2:-} == oracle-small && $# == 2 ]] || usage
  corpus=$bench_root/oracle-small
  verify_corpus "$corpus"
  build_bench_binaries
  run_root=$(new_run_root oracle oracle-small)
  otel_args=()
  for path in "$corpus"/otel/*.jsonl; do
    otel_args+=(--otel-file "$path")
  done
  mkdir -p "$run_root/home" "$run_root/config"
  HOME=$run_root/home \
    CLAUDE_CONFIG_DIR=$run_root/config \
    NO_COLOR=1 \
    "$product_binary" \
    --timezone UTC \
    --data-dir "$corpus/projects" \
    "${otel_args[@]}" \
    --json \
    2026 \
    >"$run_root/report.json" \
    2>"$run_root/stderr.log"
  test ! -s "$run_root/stderr.log"
  if grep -F \
    -e SYNTHETIC_PHASE5_PROMPT_CANARY \
    -e SYNTHETIC_PHASE5_PATH_CANARY \
    -e SYNTHETIC_PHASE5_EMAIL_CANARY \
    "$run_root/report.json"; then
    printf 'privacy canary leaked into oracle report\n' >&2
    exit 1
  fi
  jq -e --slurpfile manifest "$corpus/manifest.json" '
    .dataCoverage as $coverage
    | .canonicalMetrics.tokens.global as $tokens
    | $manifest[0].oracle as $oracle
    | $manifest[0].metricOracle as $metric
    | $manifest[0].activeTimeOracle as $active
    | $manifest[0].insightEligibility as $insights
    | ($coverage.acceptedRecords == $oracle.acceptedRecords)
      and ($coverage.canonicalRecords == $oracle.canonicalRecords)
      and ($coverage.classifiedRecords
        == $manifest[0].distribution.classifiedRecords)
      and ($coverage.malformedRecords == $oracle.malformedRecords)
      and ($coverage.unsupportedRecords == $oracle.unsupportedRecords)
      and ($coverage.unknownRecords == $oracle.unknownRecords)
      and ($coverage.filteredRecords == $oracle.filteredRecords)
      and ($coverage.duplicateRecords == $oracle.duplicateRecords)
      and ($coverage.resolvedOverlapRecords == $oracle.resolvedOverlapRecords)
      and ($coverage.unresolvedOverlapRecords == $oracle.unresolvedOverlapRecords)
      and ($tokens.input.observed == $oracle.inputTokens)
      and ($tokens.output.observed == $oracle.outputTokens)
      and ($tokens.cacheCreation.observed == $oracle.cacheCreationTokens)
      and ($tokens.cacheRead.observed == $oracle.cacheReadTokens)
      and ($coverage.capabilities.metric_token_usage == "available")
      and ($metric.points == 12)
      and ($metric.acceptedPoints == 10)
      and ($metric.filteredPoints == 2)
      and ($metric.deltaPoints == 6)
      and ($metric.cumulativePoints == 6)
      and ($metric.resetPoints == 2)
      and ($metric.gapPoints == 4)
      and ($metric.overlapPoints == 2)
      and ([ $coverage.warnings[].code ]
        | index("W_OTEL_METRIC_GAP") != null)
      and ([ $coverage.warnings[].code ]
        | index("W_OTEL_METRIC_OVERLAP") != null)
      and ([ $coverage.warnings[].code ]
        | index("W_OTEL_METRIC_RESET") != null)
      and ((.canonicalMetrics.activeTime
        | {
            intervalCount,
            totalElapsedSeconds,
            totalActiveSeconds,
            mainExclusiveSeconds,
            subagentExclusiveSeconds
          }) == $active)
      and (([ .insights.families[]
              | {
                  family,
                  availability,
                  sampleCount,
                  minimumSampleCount
                } ]) == $insights)
  ' "$run_root/report.json" >/dev/null
  (
    cd "$run_root"
    artifacts_tmp=$run_root.ARTIFACTS.sha256.tmp
    find . -type f ! -name ARTIFACTS.sha256 -print0 |
      sort -z |
      xargs -0 sha256sum >"$artifacts_tmp"
    mv "$artifacts_tmp" ARTIFACTS.sha256
    sha256sum --check ARTIFACTS.sha256 >/dev/null
  )
  printf 'oracle=PASS\n'
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == determinism ]]; then
  shift
  class=
  workers_csv=
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers_csv=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ -n $class && -n $workers_csv ]] || usage
  IFS=, read -r -a worker_counts <<<"$workers_csv"
  ((${#worker_counts[@]} > 0)) || usage
  declare -A seen_workers=()
  for workers in "${worker_counts[@]}"; do
    require_positive_integer workers "$workers"
    if [[ -n ${seen_workers[$workers]:-} ]]; then
      printf 'duplicate worker count: %s\n' "$workers" >&2
      exit 2
    fi
    seen_workers[$workers]=1
  done
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  run_root=$(new_run_root determinism "$class")
  reference=
  for workers in "${worker_counts[@]}"; do
    suffix=$(printf '%03d' "$workers")
    report=$run_root/report-workers-$suffix.json
    run_canonical_report \
      "$corpus" \
      "$workers" \
      no-store \
      "" \
      "$report" \
      "$run_root/stderr-workers-$suffix.log" \
      "$run_root/scratch-workers-$suffix"
    if [[ -z $reference ]]; then
      reference=$report
    else
      cmp "$reference" "$report"
    fi
  done
  sha256sum "$run_root"/report-workers-*.json >"$run_root/report-digests.sha256"
  finalize_artifacts "$run_root"
  printf 'determinism=PASS\n'
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == branch ]]; then
  shift
  class=decision
  workers=12
  runs=5
  warmups=3
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ $class == decision ]] || {
    printf 'branch selection is frozen to the decision corpus\n' >&2
    exit 2
  }
  require_positive_integer workers "$workers"
  require_minimum_runs runs "$runs" 5
  require_minimum_runs warmups "$warmups" 3
  require_positive_integer sample-ms "$sample_ms"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  run_root=$(new_run_root branch "$class")
  mkdir -p "$run_root/no-store" "$run_root/first-import" "$run_root/warm"

  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/no-store/warmup-$suffix.json" \
      "$run_root/no-store/warmup-stderr-$suffix.log" \
      "$run_root/no-store/warmup-scratch-$suffix" \
      no-store
  done
  no_store_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/no-store/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/no-store/stderr-$suffix.log" \
      "$run_root/no-store/scratch-$suffix" \
      no-store
    no_store_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$run_root/no-store/summary.json" \
    "${no_store_samples[@]}"

  first_samples=()
  store_allocation_maximum=0
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    store=$run_root/first-import/store-$suffix.sqlite3
    sample=$run_root/first-import/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/first-import/stderr-$suffix.log" \
      "$run_root/first-import/scratch-$suffix" \
      store \
      "$store"
    first_samples+=("$sample")
    allocation=$(store_allocation_bytes "$store")
    ((allocation > store_allocation_maximum)) &&
      store_allocation_maximum=$allocation
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    0 \
    "$source_bytes" \
    "$run_root/first-import/summary.json" \
    "${first_samples[@]}"

  warm_store=$run_root/first-import/store-01.sqlite3
  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/warm/warmup-$suffix.json" \
      "$run_root/warm/warmup-stderr-$suffix.log" \
      "$run_root/warm/warmup-scratch-$suffix" \
      store \
      "$warm_store"
  done
  warm_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/warm/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/warm/stderr-$suffix.log" \
      "$run_root/warm/scratch-$suffix" \
      store \
      "$warm_store"
    warm_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$run_root/warm/summary.json" \
    "${warm_samples[@]}"

  run_canonical_report \
    "$corpus" \
    "$workers" \
    no-store \
    "" \
    "$run_root/no-store-report.json" \
    "$run_root/no-store-report.stderr" \
    "$run_root/no-store-report-scratch"
  run_canonical_report \
    "$corpus" \
    "$workers" \
    store \
    "$warm_store" \
    "$run_root/store-report.json" \
    "$run_root/store-report.stderr" \
    "$run_root/store-report-scratch"
  cmp "$run_root/no-store-report.json" "$run_root/store-report.json"
  sha256sum \
    "$run_root/no-store-report.json" \
    "$run_root/store-report.json" \
    >"$run_root/report-digests.sha256"

  jq -n \
    --slurpfile no_store "$run_root/no-store/summary.json" \
    --slurpfile first "$run_root/first-import/summary.json" \
    --slurpfile warm "$run_root/warm/summary.json" \
    --argjson source_bytes "$source_bytes" \
    --argjson store_allocation "$store_allocation_maximum" \
    '
      ($no_store[0]) as $no
      | ($first[0]) as $first_import
      | ($warm[0]) as $warm_run
      | ($no.wallNanos.median / $warm_run.wallNanos.median) as $warm_speedup
      | ($no.wallNanos.median - $warm_run.wallNanos.median) as $warm_reduction
      | ($first_import.wallNanos.median / $no.wallNanos.median) as $first_ratio
      | ([2147483648, $source_bytes] | min) as $store_limit
      | {
          schema: "ccwrapped.phase5-branch/v1",
          selectedBranch: "sqlite",
          sourceBytes: $source_bytes,
          storeAllocationBytes: $store_allocation,
          storeAllocationLimitBytes: $store_limit,
          exactReportEquality: true,
          noStore: $no,
          firstImport: $first_import,
          warmNoChange: $warm_run,
          warmSpeedup: $warm_speedup,
          warmAbsoluteReductionNanos: $warm_reduction,
          firstImportRatio: $first_ratio,
          gates: {
            noStoreThreshold: ($no.wallNanos.median > 750000000),
            warmSpeedup: ($warm_speedup >= 4),
            warmAbsoluteReduction: ($warm_reduction >= 750000000),
            warmP95: ($warm_run.wallNanos.p95 <= 1000000000),
            firstImportOverhead: ($first_ratio <= 1.35),
            storeAllocation: ($store_allocation <= $store_limit),
            rss:
              ($no.peakRssBytes.maximum <= 4294967296
               and $first_import.peakRssBytes.maximum <= 4294967296
               and $warm_run.peakRssBytes.maximum <= 536870912),
            invocationDuration:
              ($no.wallNanos.maximum < 300000000000
               and $first_import.wallNanos.maximum < 300000000000),
            stable:
              ($no.stable
               and ($first_import.wallNanos.coefficientOfVariation <= 0.10)
               and ($warm_run.wallNanos.coefficientOfVariation <= 0.10)),
            exactReportEquality: true
          }
        }
      | .passed = (.gates | all(.[]; . == true))
    ' >"$run_root/summary.json"
  finalize_artifacts "$run_root"
  jq -c '{
    selectedBranch,
    noStoreMedianNanos: .noStore.wallNanos.median,
    firstImportMedianNanos: .firstImport.wallNanos.median,
    firstImportRatio,
    warmMedianNanos: .warmNoChange.wallNanos.median,
    warmSpeedup,
    storeAllocationBytes,
    passed
  }' "$run_root/summary.json"
  jq -e '.passed == true' "$run_root/summary.json" >/dev/null || {
    printf 'SQLite branch did not pass every frozen selection gate\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == baseline ]]; then
  shift
  class=
  runs=1
  workers=1
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ -n $class ]] || usage
  require_positive_integer runs "$runs"
  require_positive_integer workers "$workers"
  require_positive_integer sample-ms "$sample_ms"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  run_root=$(new_run_root baseline "$class")
  for ((run = 1; run <= runs; run++)); do
    sample=$run_root/sample-$(printf '%02d' "$run").json
    stderr_log=$run_root/stderr-$(printf '%02d' "$run").log
    scratch=$run_root/scratch-$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$stderr_log" \
      "$scratch" \
      no-store
    jq -c '{
      wallNanos,
      cpuSeconds,
      allocatedCpuUtilization,
      peakRssBytes,
      logicalReadBytes,
      physicalReadBytes,
      sampleCount
    }' "$sample"
  done
  rm -rf "$run_root"/scratch-*
  (
    cd "$run_root"
    sha256sum sample-*.json stderr-*.log >ARTIFACTS.sha256
  )
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == scale ]]; then
  shift
  class=decision
  workers_csv=
  runs=5
  warmups=3
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers_csv=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ -n $workers_csv ]] || usage
  require_positive_integer runs "$runs"
  require_positive_integer warmups "$warmups"
  require_positive_integer sample-ms "$sample_ms"
  if ((runs < 5)); then
    printf 'scale requires at least five measured runs per worker count\n' >&2
    exit 2
  fi
  IFS=, read -r -a worker_counts <<<"$workers_csv"
  if ((${#worker_counts[@]} == 0)); then
    usage
  fi
  declare -A seen_workers=()
  for workers in "${worker_counts[@]}"; do
    require_positive_integer workers "$workers"
    if [[ -n ${seen_workers[$workers]:-} ]]; then
      printf 'duplicate worker count: %s\n' "$workers" >&2
      exit 2
    fi
    seen_workers[$workers]=1
  done

  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  run_root=$(new_run_root scale "$class")
  point_summaries=()
  for workers in "${worker_counts[@]}"; do
    worker_root=$run_root/workers-$(printf '%03d' "$workers")
    mkdir -p "$worker_root"
    for ((run = 1; run <= warmups; run++)); do
      suffix=$(printf '%02d' "$run")
      run_product_sample \
        "$corpus" \
        "$workers" \
        "$sample_ms" \
        "$worker_root/warmup-$suffix.json" \
        "$worker_root/warmup-stderr-$suffix.log" \
        "$worker_root/warmup-scratch-$suffix" \
        no-store
    done
    measured_samples=()
    for ((run = 1; run <= runs; run++)); do
      suffix=$(printf '%02d' "$run")
      sample=$worker_root/sample-$suffix.json
      run_product_sample \
        "$corpus" \
        "$workers" \
        "$sample_ms" \
        "$sample" \
        "$worker_root/stderr-$suffix.log" \
        "$worker_root/scratch-$suffix" \
        no-store
      measured_samples+=("$sample")
    done
    rm -rf "$worker_root"/*-scratch-* "$worker_root"/scratch-*
    summary=$worker_root/summary.json
    summarize_measurements \
      "$class" \
      "$workers" \
      "$warmups" \
      "$source_bytes" \
      "$summary" \
      "${measured_samples[@]}"
    point_summaries+=("$summary")
    jq -c '{
      workerCount,
      wallMedianNanos: .wallNanos.median,
      wallCoV: .wallNanos.coefficientOfVariation,
      utilizationMedian: .allocatedCpuUtilization.median,
      utilizationCoV: .allocatedCpuUtilization.coefficientOfVariation,
      peakRssBytes: .peakRssBytes.maximum,
      stable
    }' "$summary"
  done
  jq -s \
    --arg class "$class" \
    --arg workers "$workers_csv" \
    --argjson warmups "$warmups" \
    --argjson runs "$runs" \
    '
      . as $points
      | ($points | map(select(.workerCount == 1))[0].wallNanos.median // null) as $one
      | {
          schema: "ccwrapped.phase5-scaling/v1",
          class: $class,
          requestedWorkers: $workers,
          warmupCount: $warmups,
          measuredCountPerWorker: $runs,
          points:
            ($points
             | map(. + {
                 speedupVsWorkerOne:
                   (if $one == null then null else ($one / .wallNanos.median) end)
               })),
          allStable: ($points | all(.stable))
        }
    ' \
    "${point_summaries[@]}" >"$run_root/summary.json"
  artifacts_tmp=$run_root.ARTIFACTS.sha256.tmp
  (
    cd "$run_root"
    find . -type f ! -name ARTIFACTS.sha256 -print0 |
      sort -z |
      xargs -0 sha256sum >"$artifacts_tmp"
    mv "$artifacts_tmp" ARTIFACTS.sha256
    sha256sum --check ARTIFACTS.sha256 >/dev/null
  )
  jq -e '.allStable == true' "$run_root/summary.json" >/dev/null || {
    printf 'scaling series exceeded the frozen 10%% coefficient-of-variation limit\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == saturate ]]; then
  shift
  class=saturation-large
  workers=12
  runs=5
  warmups=3
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  require_positive_integer workers "$workers"
  require_positive_integer runs "$runs"
  require_positive_integer warmups "$warmups"
  require_positive_integer sample-ms "$sample_ms"
  if ((runs < 5)); then
    printf 'saturate requires at least five measured runs\n' >&2
    exit 2
  fi
  if ((warmups < 3)); then
    printf 'saturate requires at least three warmup runs\n' >&2
    exit 2
  fi

  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  run_root=$(new_run_root saturate "$class")
  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/warmup-$suffix.json" \
      "$run_root/warmup-stderr-$suffix.log" \
      "$run_root/warmup-scratch-$suffix" \
      no-store
  done
  measured_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/stderr-$suffix.log" \
      "$run_root/scratch-$suffix" \
      no-store
    measured_samples+=("$sample")
  done
  rm -rf "$run_root"/*-scratch-* "$run_root"/scratch-*
  point_summary=$run_root/point-summary.json
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$point_summary" \
    "${measured_samples[@]}"
  jq \
    '
      . as $point
      | {
          schema: "ccwrapped.phase5-saturation/v1",
          performanceObjective: "production-throughput-plateau",
          class,
          workerCount,
          warmupCount,
          measuredCount,
          sourceBytes,
          continuousMinimumNanos: .wallNanos.minimum,
          continuousMaximumNanos: .wallNanos.maximum,
          continuousDurationGate: (.wallNanos.maximum >= 30000000000),
          utilizationGate: (.allocatedCpuUtilization.minimum >= 0.80),
          varianceGate: .stable,
          rssGate: (.peakRssBytes.maximum <= 4294967296),
          originalContractPassed:
            ((.wallNanos.maximum >= 30000000000)
             and (.allocatedCpuUtilization.minimum >= 0.80)
             and .stable
             and (.peakRssBytes.maximum <= 4294967296)),
          passed:
            (.stable
             and (.peakRssBytes.maximum <= 4294967296)),
          point: $point
        }
    ' \
    "$point_summary" >"$run_root/summary.json"
  artifacts_tmp=$run_root.ARTIFACTS.sha256.tmp
  (
    cd "$run_root"
    find . -type f ! -name ARTIFACTS.sha256 -print0 |
      sort -z |
      xargs -0 sha256sum >"$artifacts_tmp"
    mv "$artifacts_tmp" ARTIFACTS.sha256
    sha256sum --check ARTIFACTS.sha256 >/dev/null
  )
  jq -c '{
    continuousMinimumNanos,
    continuousMaximumNanos,
    utilizationMinimum: .point.allocatedCpuUtilization.minimum,
    utilizationMedian: .point.allocatedCpuUtilization.median,
    wallCoV: .point.wallNanos.coefficientOfVariation,
    utilizationCoV: .point.allocatedCpuUtilization.coefficientOfVariation,
    peakRssBytes: .point.peakRssBytes.maximum,
    originalContractPassed,
    passed
  }' "$run_root/summary.json"
  jq -e '.passed == true' "$run_root/summary.json" >/dev/null || {
    printf 'saturation series did not pass the throughput-objective variance/RSS measurement gate\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == startup ]]; then
  shift
  class=oracle-small
  workers=12
  runs=20
  warmups=3
  sample_ms=2
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ $class == oracle-small ]] || {
    printf 'startup measurement is frozen to the oracle-small corpus\n' >&2
    exit 2
  }
  require_positive_integer workers "$workers"
  require_minimum_runs runs "$runs" 20
  require_minimum_runs warmups "$warmups" 3
  require_positive_integer sample-ms "$sample_ms"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  run_root=$(new_run_root startup "$class")
  mkdir -p "$run_root/no-store" "$run_root/first-import" "$run_root/warm"

  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/no-store/warmup-$suffix.json" \
      "$run_root/no-store/warmup-stderr-$suffix.log" \
      "$run_root/no-store/warmup-scratch-$suffix" \
      no-store
  done
  no_store_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/no-store/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/no-store/stderr-$suffix.log" \
      "$run_root/no-store/scratch-$suffix" \
      no-store
    no_store_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$run_root/no-store/summary.json" \
    "${no_store_samples[@]}"

  first_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    store=$run_root/first-import/store-$suffix.sqlite3
    sample=$run_root/first-import/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/first-import/stderr-$suffix.log" \
      "$run_root/first-import/scratch-$suffix" \
      store \
      "$store"
    first_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    0 \
    "$source_bytes" \
    "$run_root/first-import/summary.json" \
    "${first_samples[@]}"

  warm_store=$run_root/first-import/store-01.sqlite3
  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/warm/warmup-$suffix.json" \
      "$run_root/warm/warmup-stderr-$suffix.log" \
      "$run_root/warm/warmup-scratch-$suffix" \
      store \
      "$warm_store"
  done
  warm_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/warm/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/warm/stderr-$suffix.log" \
      "$run_root/warm/scratch-$suffix" \
      store \
      "$warm_store"
    warm_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$run_root/warm/summary.json" \
    "${warm_samples[@]}"

  jq -n \
    --slurpfile no_store "$run_root/no-store/summary.json" \
    --slurpfile first "$run_root/first-import/summary.json" \
    --slurpfile warm "$run_root/warm/summary.json" \
    '
      ($no_store[0]) as $no
      | ($first[0]) as $first_import
      | ($warm[0]) as $warm_run
      | ($first_import.wallNanos.median - $no.wallNanos.median) as $first_delta
      | {
          schema: "ccwrapped.phase5-startup/v1",
          noStore: $no,
          firstImport: $first_import,
          warmDefault: $warm_run,
          firstImportMedianDeltaNanos: $first_delta,
          gates: {
            noStoreP95: ($no.wallNanos.p95 <= 150000000),
            defaultP95: ($warm_run.wallNanos.p95 <= 200000000),
            firstImportDelta: ($first_delta <= 50000000),
            rss:
              ($no.peakRssBytes.maximum <= 4294967296
               and $first_import.peakRssBytes.maximum <= 4294967296
               and $warm_run.peakRssBytes.maximum <= 536870912)
          }
        }
      | .passed = (.gates | all(.[]; . == true))
    ' >"$run_root/summary.json"
  finalize_artifacts "$run_root"
  jq -c '{
    noStoreP95Nanos: .noStore.wallNanos.p95,
    firstImportMedianNanos: .firstImport.wallNanos.median,
    firstImportMedianDeltaNanos,
    warmDefaultP95Nanos: .warmDefault.wallNanos.p95,
    passed
  }' "$run_root/summary.json"
  jq -e '.passed == true' "$run_root/summary.json" >/dev/null || {
    printf 'startup series did not pass every frozen latency/RSS gate\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == warm-store ]]; then
  shift
  class=
  workers=
  runs=5
  warmups=3
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ $class == decision || $class == saturation-large ]] || {
    printf 'warm-store class must be decision or saturation-large\n' >&2
    exit 2
  }
  [[ -n $workers ]] || usage
  require_positive_integer workers "$workers"
  require_minimum_runs runs "$runs" 5
  require_minimum_runs warmups "$warmups" 3
  require_positive_integer sample-ms "$sample_ms"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  run_root=$(new_run_root warm-store "$class")
  store=$run_root/store.sqlite3

  run_product_sample \
    "$corpus" \
    "$workers" \
    "$sample_ms" \
    "$run_root/first-import.json" \
    "$run_root/first-import.stderr" \
    "$run_root/first-import-scratch" \
    store \
    "$store"

  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/warmup-$suffix.json" \
      "$run_root/warmup-$suffix.stderr" \
      "$run_root/warmup-$suffix-scratch" \
      store \
      "$store"
  done
  measured_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/sample-$suffix.stderr" \
      "$run_root/sample-$suffix-scratch" \
      store \
      "$store"
    measured_samples+=("$sample")
  done
  rm -rf "$run_root"/*-scratch "$run_root"/*-scratch-*
  point_summary=$run_root/point-summary.json
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$point_summary" \
    "${measured_samples[@]}"
  store_allocation=$(store_allocation_bytes "$store")
  if [[ $class == decision ]]; then
    latency_limit=1000000000
  else
    latency_limit=3000000000
  fi
  jq \
    --argjson latency_limit "$latency_limit" \
    --argjson store_allocation "$store_allocation" \
    '
      . as $point
      | {
          schema: "ccwrapped.phase5-warm-store/v1",
          class,
          workerCount,
          warmupCount,
          measuredCount,
          sourceBytes,
          latencyLimitNanos: $latency_limit,
          storeAllocationBytes: $store_allocation,
          point: $point,
          gates: {
            latency: (.wallNanos.p95 <= $latency_limit),
            rss: (.peakRssBytes.maximum <= 536870912),
            stable: .stable,
            noSourceContentRead: (.sourceContentBytesRead.maximum == 0),
            noSourceFilesParsed: (.parsedSourceFiles.maximum == 0),
            reportCacheHit:
              (all(.rawAggregates[];
                .stageCounters.incrementalCheckpointStatus == "report-cache-hit"))
          }
        }
      | .passed = (.gates | all(.[]; . == true))
    ' \
    "$point_summary" >"$run_root/summary.json"
  finalize_artifacts "$run_root"
  jq -c '{
    class,
    wallMedianNanos: .point.wallNanos.median,
    wallP95Nanos: .point.wallNanos.p95,
    wallCoV: .point.wallNanos.coefficientOfVariation,
    peakRssBytes: .point.peakRssBytes.maximum,
    storeAllocationBytes,
    passed
  }' "$run_root/summary.json"
  jq -e '.passed == true' "$run_root/summary.json" >/dev/null || {
    printf 'warm-store series did not pass every frozen latency/RSS/variance/reuse gate\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == first-import ]]; then
  shift
  class=
  workers=12
  runs=5
  warmups=3
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ $class == decision || $class == saturation-large ]] || {
    printf 'first-import class must be decision or saturation-large\n' >&2
    exit 2
  }
  require_positive_integer workers "$workers"
  require_minimum_runs runs "$runs" 5
  require_minimum_runs warmups "$warmups" 3
  require_positive_integer sample-ms "$sample_ms"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  run_root=$(new_run_root first-import "$class")
  mkdir -p "$run_root/no-store" "$run_root/store" "$run_root/warm"

  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/no-store/warmup-$suffix.json" \
      "$run_root/no-store/warmup-stderr-$suffix.log" \
      "$run_root/no-store/warmup-scratch-$suffix" \
      no-store
  done
  no_store_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/no-store/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/no-store/stderr-$suffix.log" \
      "$run_root/no-store/scratch-$suffix" \
      no-store
    no_store_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$run_root/no-store/summary.json" \
    "${no_store_samples[@]}"

  first_samples=()
  store_allocation_maximum=0
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    store=$run_root/store/store-$suffix.sqlite3
    sample=$run_root/store/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/store/stderr-$suffix.log" \
      "$run_root/store/scratch-$suffix" \
      store \
      "$store"
    first_samples+=("$sample")
    allocation=$(store_allocation_bytes "$store")
    ((allocation > store_allocation_maximum)) &&
      store_allocation_maximum=$allocation
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    0 \
    "$source_bytes" \
    "$run_root/store/summary.json" \
    "${first_samples[@]}"

  warm_store=$run_root/store/store-01.sqlite3
  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/warm/warmup-$suffix.json" \
      "$run_root/warm/warmup-stderr-$suffix.log" \
      "$run_root/warm/warmup-scratch-$suffix" \
      store \
      "$warm_store"
  done
  warm_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/warm/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/warm/stderr-$suffix.log" \
      "$run_root/warm/scratch-$suffix" \
      store \
      "$warm_store"
    warm_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$run_root/warm/summary.json" \
    "${warm_samples[@]}"
  if [[ $class == saturation-large ]]; then
    warm_latency_limit=3000000000
  else
    warm_latency_limit=1000000000
  fi
  rm -rf \
    "$run_root"/no-store/*scratch* \
    "$run_root"/store/*scratch* \
    "$run_root"/warm/*scratch*

  jq -n \
    --slurpfile no_store "$run_root/no-store/summary.json" \
    --slurpfile first "$run_root/store/summary.json" \
    --slurpfile warm "$run_root/warm/summary.json" \
    --argjson source_bytes "$source_bytes" \
    --argjson store_allocation "$store_allocation_maximum" \
    --argjson warm_latency_limit "$warm_latency_limit" \
    '
      ($no_store[0]) as $no
      | ($first[0]) as $first_import
      | ($warm[0]) as $warm_store
      | ($first_import.wallNanos.median / $no.wallNanos.median) as $ratio
      | ([2147483648, $source_bytes] | min) as $store_limit
      | {
          schema: "ccwrapped.phase5-first-import/v1",
          class: $no.class,
          sourceBytes: $source_bytes,
          noStore: $no,
          firstImport: $first_import,
          warmNoChange: $warm_store,
          firstImportRatio: $ratio,
          storeAllocationBytes: $store_allocation,
          storeAllocationLimitBytes: $store_limit,
          warmLatencyLimitNanos: $warm_latency_limit,
          gates: {
            firstImportOverhead: ($ratio <= 1.35),
            rss:
              ($no.peakRssBytes.maximum <= 4294967296
               and $first_import.peakRssBytes.maximum <= 4294967296
               and $warm_store.peakRssBytes.maximum <= 536870912),
            storeAllocation: ($store_allocation <= $store_limit),
            invocationDuration:
              ($no.wallNanos.maximum < 300000000000
               and $first_import.wallNanos.maximum < 300000000000),
            stable:
              ($no.wallNanos.coefficientOfVariation <= 0.10
               and $first_import.wallNanos.coefficientOfVariation <= 0.10
               and $warm_store.wallNanos.coefficientOfVariation <= 0.10),
            warmLatency: ($warm_store.wallNanos.p95 <= $warm_latency_limit),
            warmNoSourceContentRead:
              ($warm_store.sourceContentBytesRead.maximum == 0),
            warmNoSourceFilesParsed:
              ($warm_store.parsedSourceFiles.maximum == 0),
            warmReportCacheHit:
              (all($warm_store.rawAggregates[];
                .stageCounters.incrementalCheckpointStatus == "report-cache-hit"))
          }
        }
      | .passed = (.gates | all(.[]; . == true))
    ' >"$run_root/summary.json"
  finalize_artifacts "$run_root"
  jq -c '{
    class,
    noStoreMedianNanos: .noStore.wallNanos.median,
    firstImportMedianNanos: .firstImport.wallNanos.median,
    warmMedianNanos: .warmNoChange.wallNanos.median,
    warmP95Nanos: .warmNoChange.wallNanos.p95,
    firstImportRatio,
    storeAllocationBytes,
    passed
  }' "$run_root/summary.json"
  jq -e '.passed == true' "$run_root/summary.json" >/dev/null || {
    printf 'first-import series did not pass every frozen overhead/RSS/store/warm-reuse/variance gate\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == first-import-point ]]; then
  shift
  class=
  workers=12
  runs=5
  warmups=3
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ $class == decision || $class == saturation-large ]] || {
    printf 'first-import-point class must be decision or saturation-large\n' >&2
    exit 2
  }
  require_positive_integer workers "$workers"
  require_minimum_runs runs "$runs" 5
  require_minimum_runs warmups "$warmups" 3
  require_positive_integer sample-ms "$sample_ms"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  store_limit=$source_bytes
  ((store_limit > 2147483648)) && store_limit=2147483648
  run_root=$(new_run_root first-import-point "$class")

  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    store=$run_root/warmup-store-$suffix.sqlite3
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/warmup-$suffix.json" \
      "$run_root/warmup-stderr-$suffix.log" \
      "$run_root/warmup-scratch-$suffix" \
      store \
      "$store"
    rm -f "$store" "$store-journal" "$store-wal" "$store-shm"
  done

  samples=()
  store_allocation_maximum=0
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    store=$run_root/store-$suffix.sqlite3
    sample=$run_root/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/stderr-$suffix.log" \
      "$run_root/scratch-$suffix" \
      store \
      "$store"
    samples+=("$sample")
    allocation=$(store_allocation_bytes "$store")
    ((allocation > store_allocation_maximum)) &&
      store_allocation_maximum=$allocation
    rm -f "$store" "$store-journal" "$store-wal" "$store-shm"
  done
  rm -rf "$run_root"/*scratch*
  point_summary=$run_root/point-summary.json
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$point_summary" \
    "${samples[@]}"
  jq \
    --argjson store_allocation "$store_allocation_maximum" \
    --argjson store_limit "$store_limit" \
    '
      . as $point
      | {
          schema: "ccwrapped.phase5-first-import-point/v1",
          class,
          sourceBytes,
          storeAllocationBytes: $store_allocation,
          storeAllocationLimitBytes: $store_limit,
          gates: {
            stable: .stable,
            rss: (.peakRssBytes.maximum <= 4294967296),
            storeAllocation: ($store_allocation <= $store_limit),
            invocationDuration: (.wallNanos.maximum < 300000000000)
          },
          point: $point
        }
      | .passed = (.gates | all(.[]; . == true))
    ' \
    "$point_summary" >"$run_root/summary.json"
  finalize_artifacts "$run_root"
  jq -c '{
    class,
    wallMedianNanos: .point.wallNanos.median,
    wallCoV: .point.wallNanos.coefficientOfVariation,
    utilizationCoV: .point.allocatedCpuUtilization.coefficientOfVariation,
    peakRssBytes: .point.peakRssBytes.maximum,
    storeAllocationBytes,
    passed
  }' "$run_root/summary.json"
  jq -e '.passed == true' "$run_root/summary.json" >/dev/null || {
    printf 'first-import point did not pass the variance/RSS/store/duration gate\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == incremental ]]; then
  shift
  class=decision
  workers=12
  runs=5
  warmups=3
  sample_ms=10
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --workers)
        (($# >= 2)) || usage
        workers=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --sample-ms)
        (($# >= 2)) || usage
        sample_ms=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ $class == decision ]] || {
    printf 'incremental-tail measurement is frozen to the decision corpus\n' >&2
    exit 2
  }
  require_positive_integer workers "$workers"
  require_minimum_runs runs "$runs" 5
  require_minimum_runs warmups "$warmups" 3
  require_positive_integer sample-ms "$sample_ms"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  build_bench_binaries
  source_bytes=$(jq -er '.sourceBytes' "$corpus/manifest.json")
  run_root=$(new_run_root incremental "$class")
  mkdir -p "$run_root/no-store" "$run_root/incremental"

  for ((run = 1; run <= warmups; run++)); do
    suffix=$(printf '%02d' "$run")
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$run_root/no-store/warmup-$suffix.json" \
      "$run_root/no-store/warmup-stderr-$suffix.log" \
      "$run_root/no-store/warmup-scratch-$suffix" \
      no-store
  done
  no_store_samples=()
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    sample=$run_root/no-store/sample-$suffix.json
    run_product_sample \
      "$corpus" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/no-store/stderr-$suffix.log" \
      "$run_root/no-store/scratch-$suffix" \
      no-store
    no_store_samples+=("$sample")
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    "$warmups" \
    "$source_bytes" \
    "$run_root/no-store/summary.json" \
    "${no_store_samples[@]}"

  incremental_samples=()
  : >"$run_root/incremental/report-digests.sha256"
  for ((run = 1; run <= runs; run++)); do
    suffix=$(printf '%02d' "$run")
    copy=$run_root/incremental/corpus-$suffix
    mkdir -p "$copy"
    cp -a --reflink=auto "$corpus"/. "$copy"/
    store=$run_root/incremental/store-$suffix.sqlite3
    run_product_sample \
      "$copy" \
      "$workers" \
      "$sample_ms" \
      "$run_root/incremental/prime-$suffix.json" \
      "$run_root/incremental/prime-stderr-$suffix.log" \
      "$run_root/incremental/prime-scratch-$suffix" \
      store \
      "$store"
    "$support_binary" incremental-tail \
      --corpus "$copy" \
      --output "$run_root/incremental/tail-$suffix.json" \
      >/dev/null
    sample=$run_root/incremental/sample-$suffix.json
    run_product_sample \
      "$copy" \
      "$workers" \
      "$sample_ms" \
      "$sample" \
      "$run_root/incremental/stderr-$suffix.log" \
      "$run_root/incremental/scratch-$suffix" \
      store \
      "$store"
    incremental_samples+=("$sample")

    run_canonical_report \
      "$copy" \
      "$workers" \
      store \
      "$store" \
      "$run_root/incremental/store-report-$suffix.json" \
      "$run_root/incremental/store-report-$suffix.stderr" \
      "$run_root/incremental/store-report-scratch-$suffix"
    run_canonical_report \
      "$copy" \
      "$workers" \
      no-store \
      "" \
      "$run_root/incremental/clean-report-$suffix.json" \
      "$run_root/incremental/clean-report-$suffix.stderr" \
      "$run_root/incremental/clean-report-scratch-$suffix"
    cmp \
      "$run_root/incremental/store-report-$suffix.json" \
      "$run_root/incremental/clean-report-$suffix.json"
    digest=$(
      sha256sum "$run_root/incremental/store-report-$suffix.json" |
        awk '{print $1}'
    )
    printf '%s  report-%s\n' "$digest" "$suffix" \
      >>"$run_root/incremental/report-digests.sha256"
    rm -rf \
      "$copy" \
      "$run_root/incremental/prime-scratch-$suffix" \
      "$run_root/incremental/scratch-$suffix" \
      "$run_root/incremental/store-report-scratch-$suffix" \
      "$run_root/incremental/clean-report-scratch-$suffix"
    rm -f \
      "$run_root/incremental/store-report-$suffix.json" \
      "$run_root/incremental/clean-report-$suffix.json"
  done
  summarize_measurements \
    "$class" \
    "$workers" \
    0 \
    "$source_bytes" \
    "$run_root/incremental/summary.json" \
    "${incremental_samples[@]}"
  jq -s '{
    schema: "ccwrapped.phase5-incremental-tail-series/v1",
    generatorVersions: (map(.generatorVersion) | unique),
    beforeSourceBytes: (map(.beforeSourceBytes) | unique),
    afterSourceBytes: (map(.afterSourceBytes) | unique),
    tailSourceBytes: (map(.tailSourceBytes) | unique),
    appendedRecords: (map(.appendedRecords) | unique),
    changedExistingFiles: (map(.changedExistingFiles) | unique),
    newFiles: (map(.newFiles) | unique)
  }' "$run_root"/incremental/tail-*.json \
    >"$run_root/incremental/tail-summary.json"
  unique_report_digests=$(
    awk '{print $1}' "$run_root/incremental/report-digests.sha256" |
      sort -u |
      wc -l
  )
  expected_changed_files=$(
    jq -er '.changedExistingFiles[0] + .newFiles[0]' \
      "$run_root/incremental/tail-summary.json"
  )
  jq -n \
    --slurpfile no_store "$run_root/no-store/summary.json" \
    --slurpfile incremental "$run_root/incremental/summary.json" \
    --slurpfile tail "$run_root/incremental/tail-summary.json" \
    --argjson source_bytes "$source_bytes" \
    --argjson expected_changed_files "$expected_changed_files" \
    --argjson unique_report_digests "$unique_report_digests" \
    '
      ($no_store[0]) as $no
      | ($incremental[0]) as $inc
      | ($tail[0]) as $tail_run
      | ($no.wallNanos.median * 0.25) as $latency_limit
      | ($source_bytes * 0.02) as $read_limit
      | {
          schema: "ccwrapped.phase5-incremental/v1",
          sourceBytes: $source_bytes,
          noStore: $no,
          incremental: $inc,
          tail: $tail_run,
          incrementalLatencyLimitNanos: $latency_limit,
          incrementalReadLimitBytes: $read_limit,
          exactReportEquality: ($unique_report_digests == 1),
          gates: {
            latency: ($inc.wallNanos.median <= $latency_limit),
            changedBytes:
              ($inc.sourceContentBytesRead.maximum <= $read_limit
               and $tail_run.tailSourceBytes[0] <= ($source_bytes * 0.01)),
            changedFiles:
              ($inc.parsedSourceFiles.minimum == $expected_changed_files
               and $inc.parsedSourceFiles.maximum == $expected_changed_files),
            recordLimit: ($tail_run.appendedRecords[0] <= 5000),
            rss: ($inc.peakRssBytes.maximum <= 4294967296),
            stable: ($inc.wallNanos.coefficientOfVariation <= 0.10),
            exactReportEquality: ($unique_report_digests == 1)
          }
        }
      | .passed = (.gates | all(.[]; . == true))
    ' >"$run_root/summary.json"
  finalize_artifacts "$run_root"
  jq -c '{
    noStoreMedianNanos: .noStore.wallNanos.median,
    incrementalMedianNanos: .incremental.wallNanos.median,
    incrementalReadBytes: .incremental.sourceContentBytesRead.maximum,
    parsedFiles: .incremental.parsedSourceFiles.maximum,
    wallCoV: .incremental.wallNanos.coefficientOfVariation,
    exactReportEquality,
    passed
  }' "$run_root/summary.json"
  jq -e '.passed == true' "$run_root/summary.json" >/dev/null || {
    printf 'incremental series did not pass every frozen work/latency/RSS/variance/equality gate\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == reader ]]; then
  shift
  class=
  runs=1
  passes=1
  buffer_bytes=$((1024 * 1024))
  while (($#)); do
    case $1 in
      --class)
        (($# >= 2)) || usage
        class=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --passes)
        (($# >= 2)) || usage
        passes=$2
        shift 2
        ;;
      --buffer-bytes)
        (($# >= 2)) || usage
        buffer_bytes=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ -n $class ]] || usage
  require_positive_integer runs "$runs"
  require_positive_integer passes "$passes"
  require_positive_integer buffer-bytes "$buffer_bytes"
  corpus=$bench_root/$class
  verify_corpus "$corpus"
  cargo build \
    --manifest-path "$support_manifest" \
    --release \
    --locked \
    --offline
  run_root=$(new_run_root reader "$class")
  for ((run = 1; run <= runs; run++)); do
    sample=$run_root/reader-$(printf '%02d' "$run").json
    "$support_binary" read-baseline \
      --corpus "$corpus" \
      --passes "$passes" \
      --buffer-bytes "$buffer_bytes" \
      >"$sample"
    jq -e '.bytesRead > 0 and .wallNanos > 0' "$sample" >/dev/null
    jq -c '{fileCount, bytesRead, wallNanos, bytesPerSecond}' "$sample"
  done
  (
    cd "$run_root"
    sha256sum reader-*.json >ARTIFACTS.sha256
  )
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == memory ]]; then
  shift
  workers_csv=
  runs=5
  warmups=3
  bytes_per_worker=$((64 * 1024 * 1024))
  passes=64
  while (($#)); do
    case $1 in
      --workers)
        (($# >= 2)) || usage
        workers_csv=$2
        shift 2
        ;;
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --warmups)
        (($# >= 2)) || usage
        warmups=$2
        shift 2
        ;;
      --bytes-per-worker)
        (($# >= 2)) || usage
        bytes_per_worker=$2
        shift 2
        ;;
      --passes)
        (($# >= 2)) || usage
        passes=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  [[ -n $workers_csv ]] || usage
  require_positive_integer runs "$runs"
  require_positive_integer warmups "$warmups"
  require_positive_integer bytes-per-worker "$bytes_per_worker"
  require_positive_integer passes "$passes"
  if ((runs < 5)); then
    printf 'memory baseline requires at least five measured runs per worker count\n' >&2
    exit 2
  fi
  IFS=, read -r -a worker_counts <<<"$workers_csv"
  if ((${#worker_counts[@]} == 0)); then
    usage
  fi
  declare -A seen_workers=()
  for workers in "${worker_counts[@]}"; do
    require_positive_integer workers "$workers"
    if [[ -n ${seen_workers[$workers]:-} ]]; then
      printf 'duplicate worker count: %s\n' "$workers" >&2
      exit 2
    fi
    seen_workers[$workers]=1
  done
  cargo build \
    --manifest-path "$support_manifest" \
    --release \
    --locked \
    --offline
  run_root=$(new_run_root memory host)
  point_summaries=()
  for workers in "${worker_counts[@]}"; do
    worker_root=$run_root/workers-$(printf '%03d' "$workers")
    mkdir -p "$worker_root"
    for ((run = 1; run <= warmups; run++)); do
      run_memory_sample \
        "$workers" \
        "$bytes_per_worker" \
        "$passes" \
        "$worker_root/warmup-$(printf '%02d' "$run").json"
    done
    measured_samples=()
    for ((run = 1; run <= runs; run++)); do
      sample=$worker_root/sample-$(printf '%02d' "$run").json
      run_memory_sample "$workers" "$bytes_per_worker" "$passes" "$sample"
      measured_samples+=("$sample")
    done
    summary=$worker_root/summary.json
    summarize_memory_measurements \
      "$workers" \
      "$warmups" \
      "$summary" \
      "${measured_samples[@]}"
    point_summaries+=("$summary")
    jq -c '{
      workerCount,
      wallMedianNanos: .wallNanos.median,
      trafficGiBPerSecond:
        (.memoryTrafficBytesPerSecondLowerBound.median / 1073741824),
      wallCoV: .wallNanos.coefficientOfVariation,
      utilizationMedian: .allocatedCpuUtilization.median,
      utilizationCoV: .allocatedCpuUtilization.coefficientOfVariation,
      peakRssBytes: .peakRssBytes.maximum,
      stable
    }' "$summary"
  done
  jq -s \
    --arg workers "$workers_csv" \
    --argjson warmups "$warmups" \
    --argjson runs "$runs" \
    '{
      schema: "ccwrapped.phase5-memory-scaling/v1",
      requestedWorkers: $workers,
      warmupCount: $warmups,
      measuredCountPerWorker: $runs,
      points: .,
      allStable: all(.[]; .stable)
    }' \
    "${point_summaries[@]}" >"$run_root/summary.json"
  artifacts_tmp=$run_root.ARTIFACTS.sha256.tmp
  (
    cd "$run_root"
    find . -type f ! -name ARTIFACTS.sha256 -print0 |
      sort -z |
      xargs -0 sha256sum >"$artifacts_tmp"
    mv "$artifacts_tmp" ARTIFACTS.sha256
    sha256sum --check ARTIFACTS.sha256 >/dev/null
  )
  jq -e '.allStable == true' "$run_root/summary.json" >/dev/null || {
    printf 'memory series exceeded the frozen 10%% coefficient-of-variation limit\n' >&2
    printf 'raw_output=%s\n' "$run_root" >&2
    exit 1
  }
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == instrument-overhead ]]; then
  shift
  runs=5
  iterations=1000
  while (($#)); do
    case $1 in
      --runs)
        (($# >= 2)) || usage
        runs=$2
        shift 2
        ;;
      --iterations)
        (($# >= 2)) || usage
        iterations=$2
        shift 2
        ;;
      *)
        usage
        ;;
    esac
  done
  require_minimum_runs runs "$runs" 5
  require_positive_integer iterations "$iterations"
  cargo build \
    --manifest-path "$support_manifest" \
    --release \
    --locked \
    --offline
  run_root=$(new_run_root instrument-overhead host)
  samples=()
  for ((run = 1; run <= runs; run++)); do
    sample=$run_root/sample-$(printf '%02d' "$run").json
    "$support_binary" sampler-overhead \
      --iterations "$iterations" \
      >"$sample"
    jq -e \
      --argjson iterations "$iterations" \
      '.schema == "ccwrapped.phase5-sampler-overhead/v1"
       and .iterations == $iterations
       and (.samplesNanos | length) == $iterations
       and .totalNanos > 0' \
      "$sample" >/dev/null
    samples+=("$sample")
  done
  jq -s \
    --argjson runs "$runs" \
    --argjson iterations "$iterations" \
    '
      def stats($values):
        ($values | length) as $count
        | ($values | sort) as $sorted
        | ($values | add / $count) as $mean
        | (if ($count % 2) == 1
           then $sorted[($count / 2 | floor)]
           else (($sorted[$count / 2 - 1] + $sorted[$count / 2]) / 2)
           end) as $median
        | {
            count: $count,
            mean: $mean,
            median: $median,
            minimum: $sorted[0],
            maximum: $sorted[-1]
          };
      {
        schema: "ccwrapped.phase5-sampler-overhead-series/v1",
        measuredCount: $runs,
        iterationsPerRun: $iterations,
        procFilesPerSample: .[0].procFilesPerSample,
        perPollMeanNanos: stats(map(.meanNanos)),
        perPollP95Nanos: stats(map(.p95Nanos)),
        rawAggregates:
          map({
            totalNanos,
            meanNanos,
            medianNanos,
            p95Nanos,
            maximumNanos
          })
      }
    ' \
    "${samples[@]}" >"$run_root/summary.json"
  finalize_artifacts "$run_root"
  jq -c '{
    measuredCount,
    iterationsPerRun,
    perPollMeanNanos: .perPollMeanNanos.median,
    perPollP95Nanos: .perPollP95Nanos.median
  }' "$run_root/summary.json"
  printf 'raw_output=%s\n' "$run_root"
  exit 0
fi

if [[ $command_name == verify-record ]]; then
  (($# == 1)) || usage
  record=$repo_root/docs/benchmarks/phase5-record.json
  jq -e '
    .environment.productionAutoWorkers as $production_workers
    | .schema == "ccwrapped.phase5-benchmark-record/v2"
    and .rubricVersion == "phase5/v1"
    and .rubricAmendment.id == "phase5/performance-throughput-amendment/v1"
    and .rubricAmendment.approvedAt == "2026-07-19"
    and .rubricAmendment.originalGatesRemainReported == true
    and .rubricAmendment.amendedObjective == "production-throughput-plateau"
    and .rubricAmendment.durableRecord
      == "docs/benchmarks/phase5-throughput-amendment.md"
    and (.rubricAmendment.durableRecordSha256 | test("^[0-9a-f]{64}$"))
    and .selectedBranch == "sqlite"
    and .environment.productionAutoWorkers == 12
    and .bottleneck.saturationWorkers
      == .environment.productionAutoWorkers
    and (.corpora | keys == ["decision", "oracleSmall", "saturationLarge"])
    and all(.corpora[];
      .manifest.schema == "ccwrapped.phase5-corpus/v2"
      and .manifest.generatorVersion == "phase5-corpus/2.0.0"
      and (.manifest.seed | length > 0)
      and .manifest.sourceBytes > 0
      and .manifest.physicalRecords > 0
      and .manifest.distribution.classifiedRecords > 0
      and .manifest.metricOracle.points > 0
      and (.manifestSha256 | test("^[0-9a-f]{64}$"))
      and (.sourceManifestSha256 | test("^[0-9a-f]{64}$"))
      and .sourceFileCount
        == (.manifest.transcriptFiles + .manifest.otelFiles))
    and .corpora.saturationLarge.manifest.sourceBytes >= 2147483648
    and (.measurements.startup.primary.noStore.rawAggregates | length >= 20)
    and (.measurements.startup.confirmation.noStore.rawAggregates | length >= 20)
    and (.measurements.decision.primary.noStore.rawAggregates | length >= 5)
    and (.measurements.decision.confirmation.noStore.rawAggregates | length >= 5)
    and (.measurements.decision.incrementalPrimary.incremental.rawAggregates | length >= 5)
    and (.measurements.decision.incrementalConfirmation.incremental.rawAggregates | length >= 5)
    and (.measurements.saturation.primary.noStore.rawAggregates | length >= 5)
    and (.measurements.saturation.confirmation.noStore.rawAggregates | length >= 5)
    and (.measurements.saturation.utilizationPrimary.point.rawAggregates | length >= 5)
    and (.measurements.saturation.utilizationConfirmation.point.rawAggregates | length >= 5)
    and all([
      .measurements.startup.primary.noStore,
      .measurements.startup.primary.firstImport,
      .measurements.startup.primary.warmDefault,
      .measurements.startup.confirmation.noStore,
      .measurements.startup.confirmation.firstImport,
      .measurements.startup.confirmation.warmDefault,
      .measurements.decision.primary.noStore,
      .measurements.decision.primary.firstImport,
      .measurements.decision.primary.warmNoChange,
      .measurements.decision.confirmation.noStore,
      .measurements.decision.confirmation.firstImport,
      .measurements.decision.confirmation.warmNoChange,
      .measurements.decision.incrementalPrimary.noStore,
      .measurements.decision.incrementalPrimary.incremental,
      .measurements.decision.incrementalConfirmation.noStore,
      .measurements.decision.incrementalConfirmation.incremental,
      .measurements.saturation.primary.noStore,
      .measurements.saturation.primary.firstImport,
      .measurements.saturation.primary.warmNoChange,
      .measurements.saturation.confirmation.noStore,
      .measurements.saturation.confirmation.firstImport,
      .measurements.saturation.confirmation.warmNoChange,
      .measurements.saturation.utilizationPrimary.point,
      .measurements.saturation.utilizationConfirmation.point
    ][];
      .workerCount == $production_workers)
    and ([1, 2, 4, 8, 12, 15]
      - (.measurements.scaling.points | map(.workerCount))) == []
    and .measurements.instrumentOverhead.measuredCount >= 5
    and (.rawEvidence | length >= 15)
    and ([.rawEvidence[].relativeRoot] | length == (unique | length))
    and ([.rawEvidence[].recordPath] | length == (unique | length))
    and all(.rawEvidence[];
      (.role | IN("primary", "confirmation", "support"))
      and (.relativeRoot | length > 0)
      and (.summaryFile | length > 0)
      and (.recordPath | length > 0)
      and
      (.artifactsManifestSha256 | test("^[0-9a-f]{64}$"))
      and (.summarySha256 | test("^[0-9a-f]{64}$"))
      and (.processWallNanos | type == "number")
      and .processWallNanos >= 0
      and (.kind | length > 0))
    and .campaignDurationNanos > 0
    and .campaignDurationNanos <= 1800000000000
    and .instrumentation.sampleIntervalMillis > 0
    and .instrumentation.pollingUncertaintyNanos > 0
    and (.sourceSnapshot.releaseBinarySha256 | test("^[0-9a-f]{64}$"))
    and (.sourceSnapshot.measurementDriverSha256 | test("^[0-9a-f]{64}$"))
    and (.sourceSnapshot.verificationDriverSha256 | test("^[0-9a-f]{64}$"))
    and (.sourceSnapshot.generatorSourceSha256 | test("^[0-9a-f]{64}$"))
    and (.sourceSnapshot.supportBinarySha256 | test("^[0-9a-f]{64}$"))
    and .support.determinism.allEqual == true
    and .measurements.decision.branch.exactReportEquality == true
    and .measurements.scaling.points
      == (([
            .measurements.scaling.sources.worker1.points[0],
            .measurements.scaling.sources.worker2.points[0],
            .measurements.scaling.sources.worker4.points[0],
            .measurements.scaling.sources.worker8.points[0]
          ] + .measurements.scaling.sources.plateau.points)
          | sort_by(.workerCount))
    and .measurements.decision.primary.noStore
      == (.measurements.scaling.sources.plateau.points
          | map(select(.workerCount == $production_workers))[0])
    and .measurements.decision.primary.firstImport
      == .support.decisionComponents.primary.firstImportSource.point
    and .measurements.decision.primary.warmNoChange
      == .support.decisionComponents.primary.warmSource.point
    and .measurements.decision.confirmation.noStore
      == .support.decisionComponents.confirmation.noStoreSource.points[0]
    and .measurements.decision.confirmation.firstImport
      == .support.decisionComponents.confirmation.firstImportSource.point
    and .measurements.decision.confirmation.warmNoChange
      == .support.decisionComponents.confirmation.warmSource.point
    and .measurements.saturation.primary.noStore
      == .measurements.saturation.utilizationPrimary.point
    and .measurements.saturation.primary.firstImport
      == .support.saturationComponents.primary.firstImportSource.point
    and .measurements.saturation.primary.warmNoChange
      == .support.saturationComponents.primary.warmSource.point
    and .measurements.saturation.confirmation.noStore
      == .measurements.saturation.utilizationConfirmation.point
    and .measurements.saturation.confirmation.firstImport
      == .support.saturationComponents.confirmation.firstImportSource.point
    and .measurements.saturation.confirmation.warmNoChange
      == .support.saturationComponents.confirmation.warmSource.point
    and .measurements.memory.points
      == (([.measurements.memory.multiSource.points[]
             | select(.stable and .workerCount != 11)]
           + .measurements.memory.worker11Source.points)
          | sort_by(.workerCount))
  ' "$record" >/dev/null

  amendment_relative=$(
    jq -er '.rubricAmendment.durableRecord' "$record"
  )
  [[ $amendment_relative != /* &&
     $amendment_relative != *..* &&
     -f $repo_root/$amendment_relative &&
     ! -L $repo_root/$amendment_relative ]] || {
    printf 'rubric amendment record is absent or unsafe\n' >&2
    exit 1
  }
  expected_amendment_sha=$(
    jq -er '.rubricAmendment.durableRecordSha256' "$record"
  )
  observed_amendment_sha=$(
    sha256sum "$repo_root/$amendment_relative" | awk '{print $1}'
  )
  [[ $observed_amendment_sha == "$expected_amendment_sha" ]] || {
    printf 'rubric amendment record drifted\n' >&2
    exit 1
  }

  benchmark_document=$repo_root/docs/benchmarks/phase5.md
  for required_field in \
    generatorVersion \
    manifestSha256 \
    sourceManifestSha256 \
    classifiedRecords \
    canonicalRecords \
    metricOracle \
    rubricAmendment \
    productionAutoWorkers \
    originalContractPassed; do
    grep -Fq "$required_field" "$benchmark_document" || {
      printf 'human benchmark record omits %s\n' "$required_field" >&2
      exit 1
    }
  done
  while IFS=$'\t' read -r corpus manifest sources; do
    grep -Fq "$manifest" "$benchmark_document" || {
      printf 'human benchmark record omits %s manifest hash\n' "$corpus" >&2
      exit 1
    }
    grep -Fq "$sources" "$benchmark_document" || {
      printf 'human benchmark record omits %s source-manifest hash\n' "$corpus" >&2
      exit 1
    }
  done < <(
    jq -r '
      .corpora
      | to_entries[]
      | [.key, .value.manifestSha256, .value.sourceManifestSha256]
      | @tsv
    ' "$record"
  )
  for driver_key in measurementDriverSha256 verificationDriverSha256; do
    driver_sha=$(jq -er --arg key "$driver_key" '.sourceSnapshot[$key]' "$record")
    grep -Fq "$driver_key" "$benchmark_document" || {
      printf 'human benchmark record omits %s\n' "$driver_key" >&2
      exit 1
    }
    grep -Fq "$driver_sha" "$benchmark_document" || {
      printf 'human benchmark record omits %s digest\n' "$driver_key" >&2
      exit 1
    }
  done

  [[ -x $product_binary && -x $support_binary ]] || {
    printf 'build the exact release benchmark binaries before verification\n' >&2
    exit 1
  }
  for binding in \
    "releaseBinarySha256:$product_binary" \
    "verificationDriverSha256:$repo_root/scripts/phase5-benchmark.sh" \
    "generatorSourceSha256:$repo_root/tests/support/phase5-bench/src/generator.rs" \
    "supportBinarySha256:$support_binary"; do
    key=${binding%%:*}
    path=${binding#*:}
    expected=$(jq -er --arg key "$key" '.sourceSnapshot[$key]' "$record")
    observed=$(sha256sum "$path" | awk '{print $1}')
    [[ $observed == "$expected" ]] || {
      printf '%s digest drifted\n' "$key" >&2
      exit 1
    }
  done
  production_auto_workers=$(jq -er '.environment.productionAutoWorkers' "$record")
  verify_production_auto_workers "$production_auto_workers"

  for mapping in \
    "decision:$bench_root/decision" \
    "oracleSmall:$bench_root/oracle-small" \
    "saturationLarge:$bench_root/saturation-large"; do
    key=${mapping%%:*}
    corpus=${mapping#*:}
    verify_corpus "$corpus"
    expected_manifest=$(jq -er --arg key "$key" '.corpora[$key].manifestSha256' "$record")
    expected_sources=$(
      jq -er --arg key "$key" '.corpora[$key].sourceManifestSha256' "$record"
    )
    observed_manifest=$(sha256sum "$corpus/manifest.json" | awk '{print $1}')
    observed_sources=$(sha256sum "$corpus/SOURCE-MANIFEST.sha256" | awk '{print $1}')
    [[ $observed_manifest == "$expected_manifest" ]] || {
      printf '%s manifest digest drifted\n' "$key" >&2
      exit 1
    }
    [[ $observed_sources == "$expected_sources" ]] || {
      printf '%s source-manifest digest drifted\n' "$key" >&2
      exit 1
    }
    observed_source_files=$(
      find "$corpus/projects" "$corpus/otel" -type f -printf . | wc -c
    )
    expected_source_files=$(jq -er --arg key "$key" '.corpora[$key].sourceFileCount' "$record")
    [[ $observed_source_files == "$expected_source_files" ]] || {
      printf '%s source-file count drifted\n' "$key" >&2
      exit 1
    }
    jq -e \
      --arg key "$key" \
      --slurpfile manifest "$corpus/manifest.json" \
      '.corpora[$key].manifest == $manifest[0]' \
      "$record" >/dev/null || {
      printf '%s manifest content drifted\n' "$key" >&2
      exit 1
    }
  done

  campaign_duration=0
  while IFS=$'\t' read -r kind _role relative_root summary_file \
    expected_artifacts expected_summary expected_process record_path equality_file; do
    [[ $relative_root != /* && $relative_root != *..* ]] || {
      printf 'unsafe raw-evidence root for %s\n' "$kind" >&2
      exit 1
    }
    root=$bench_root/$relative_root
    [[ -d $root && -f $root/ARTIFACTS.sha256 && -f $root/$summary_file ]] || {
      printf 'raw evidence is absent for %s\n' "$kind" >&2
      exit 1
    }
    observed_artifacts=$(sha256sum "$root/ARTIFACTS.sha256" | awk '{print $1}')
    observed_summary=$(sha256sum "$root/$summary_file" | awk '{print $1}')
    [[ $observed_artifacts == "$expected_artifacts" ]] || {
      printf 'artifact manifest drifted for %s\n' "$kind" >&2
      exit 1
    }
    [[ $observed_summary == "$expected_summary" ]] || {
      printf 'summary drifted for %s\n' "$kind" >&2
      exit 1
    }
    (
      cd "$root"
      sha256sum --check ARTIFACTS.sha256 >/dev/null
      diff -u \
        <(sed -E 's|^[0-9a-f]{64}  (\./)?||' ARTIFACTS.sha256 | sort) \
        <(find . -type f ! -name ARTIFACTS.sha256 -printf '%P\n' | sort) \
        >/dev/null
    )
    jq -e \
      --arg record_path "$record_path" \
      --slurpfile summary "$root/$summary_file" \
      'getpath($record_path | split(".")) == $summary[0]' \
      "$record" >/dev/null || {
      printf 'record summary binding drifted for %s\n' "$kind" >&2
      exit 1
    }
    jq -e \
      --arg driver "$(jq -er '.sourceSnapshot.measurementDriverSha256' "$record")" \
      --arg generator "$(jq -er '.sourceSnapshot.generatorSourceSha256' "$record")" \
      --arg product "$(jq -er '.sourceSnapshot.releaseBinarySha256' "$record")" \
      --arg support "$(jq -er '.sourceSnapshot.supportBinarySha256' "$record")" \
      '
        .schema == "ccwrapped.phase5-run-provenance/v1"
        and .measurementDriverSha256 == $driver
        and .generatorSourceSha256 == $generator
        and .productBinarySha256 == $product
        and .supportBinarySha256 == $support
      ' "$root/RUN-PROVENANCE.json" >/dev/null || {
      printf 'run provenance drifted for %s\n' "$kind" >&2
      exit 1
    }
    observed_process=$(
      find "$root" -type f \
        \( -name 'sample-*.json' -o -name 'reader-*.json' \) \
        -print0 |
        sort -z |
        xargs -0 -r jq -s '
          [
            .[]
            | if (.schema
                  | IN("ccwrapped.phase5-measurement/v1",
                       "ccwrapped.phase5-reader/v1",
                       "ccwrapped.phase5-memory/v1"))
              then .wallNanos
              elif .schema == "ccwrapped.phase5-sampler-overhead/v1"
              then .totalNanos
              else empty
              end
          ]
          | add // 0
        '
    )
    observed_process=${observed_process:-0}
    [[ $observed_process == "$expected_process" ]] || {
      printf 'process-wall accounting drifted for %s\n' "$kind" >&2
      exit 1
    }
    if [[ $equality_file != - ]]; then
      [[ -f $root/$equality_file ]] || {
        printf 'equality digest is absent for %s\n' "$kind" >&2
        exit 1
      }
      distinct_digests=$(awk '{print $1}' "$root/$equality_file" | sort -u | wc -l)
      [[ $distinct_digests == 1 ]] || {
        printf 'report equality failed for %s\n' "$kind" >&2
        exit 1
      }
    fi
    campaign_duration=$((campaign_duration + expected_process))
  done < <(
    jq -r '
      .rawEvidence[]
      | [
          .kind,
          .role,
          .relativeRoot,
          .summaryFile,
          .artifactsManifestSha256,
          .summarySha256,
          (.processWallNanos | tostring),
          .recordPath,
          (.equalityDigestFile // "-")
        ]
      | @tsv
    ' "$record"
  )
  expected_campaign=$(jq -er '.campaignDurationNanos' "$record")
  [[ $campaign_duration == "$expected_campaign" ]] || {
    printf 'campaign duration does not reconcile\n' >&2
    exit 1
  }
  printf 'benchmark_record=PASS\n'
  exit 0
fi

if [[ $command_name == verify-limits ]]; then
  (($# == 1)) || usage
  record=$repo_root/docs/benchmarks/phase5-record.json
  jq -e '
    def values($point; $key):
      [$point.rawAggregates[] | .[$key]];
    def walls($point): values($point; "wallNanos");
    def utils($point): values($point; "allocatedCpuUtilization");
    def source_reads($point):
      [$point.rawAggregates[].stageCounters.sourceContentBytesRead];
    def parsed_files($point):
      [$point.rawAggregates[].stageCounters.parsedSourceFiles];
    def mean($values): ($values | add / length);
    def median($values):
      ($values | sort) as $sorted
      | ($sorted | length) as $count
      | if ($count % 2) == 1
        then $sorted[($count / 2 | floor)]
        else (($sorted[$count / 2 - 1] + $sorted[$count / 2]) / 2)
        end;
    def p95($values):
      ($values | sort) as $sorted
      | $sorted[((0.95 * ($sorted | length) | ceil) - 1)];
    def maximum($values): ($values | max);
    def cv($values):
      mean($values) as $mean
      | if $mean == 0 then 0
        elif ($values | length) <= 1 then 0
        else
          (([$values[] | (. - $mean) * (. - $mean)] | add
            / (($values | length) - 1) | sqrt) / $mean)
        end;
    def stable($point):
      cv(walls($point)) <= 0.10 and cv(utils($point)) <= 0.10;
    def point_shape($point; $minimum):
      ($point.rawAggregates | length) >= $minimum
      and all($point.rawAggregates[];
        .wallNanos > 0
        and .wallNanos <= 300000000000
        and .peakRssBytes <= 4294967296);
    def startup_ok($series):
      point_shape($series.noStore; 20)
      and point_shape($series.firstImport; 20)
      and point_shape($series.warmDefault; 20)
      and p95(walls($series.noStore)) <= 150000000
      and p95(walls($series.warmDefault)) <= 200000000
      and (median(walls($series.firstImport))
           - median(walls($series.noStore))) <= 50000000
      and maximum(values($series.warmDefault; "peakRssBytes")) <= 536870912;
    def decision_ok($series):
      point_shape($series.noStore; 5)
      and point_shape($series.firstImport; 5)
      and point_shape($series.warmNoChange; 5)
      and stable($series.noStore)
      and stable($series.firstImport)
      and stable($series.warmNoChange)
      and median(walls($series.noStore)) > 750000000
      and (median(walls($series.noStore))
           / median(walls($series.warmNoChange))) >= 4
      and (median(walls($series.noStore))
           - median(walls($series.warmNoChange))) >= 750000000
      and p95(walls($series.warmNoChange)) <= 1000000000
      and (median(walls($series.firstImport))
           / median(walls($series.noStore))) <= 1.35
      and maximum(values($series.warmNoChange; "peakRssBytes")) <= 536870912
      and $series.storeAllocationBytes
        <= ([2147483648, $series.sourceBytes] | min);
    def incremental_ok($series; $decision):
      point_shape($series.noStore; 5)
      and point_shape($series.incremental; 5)
      and stable($series.noStore)
      and stable($series.incremental)
      and maximum(source_reads($series.incremental))
        <= ($series.sourceBytes * 0.02)
      and maximum(parsed_files($series.incremental)) == 8
      and median(walls($series.incremental))
        <= (median(walls($decision.noStore)) * 0.25);
    def saturation_ok($series):
      point_shape($series.noStore; 5)
      and point_shape($series.firstImport; 5)
      and point_shape($series.warmNoChange; 5)
      and stable($series.noStore)
      and stable($series.firstImport)
      and stable($series.warmNoChange)
      and ((($series.firstImportRatio
              - (median(walls($series.firstImport))
                 / median(walls($series.noStore))))
             | abs) < 0.000000000001)
      and (median(walls($series.firstImport))
           / median(walls($series.noStore))) <= 1.35
      and p95(walls($series.warmNoChange)) <= 3000000000
      and maximum(values($series.warmNoChange; "peakRssBytes")) <= 536870912
      and $series.storeAllocationBytes
        <= ([2147483648, $series.sourceBytes] | min)
      and $series.passed == true;
    .measurements as $measurements
    | startup_ok($measurements.startup.primary)
    and startup_ok($measurements.startup.confirmation)
    and decision_ok($measurements.decision.primary)
    and decision_ok($measurements.decision.confirmation)
    and incremental_ok(
      $measurements.decision.incrementalPrimary;
      $measurements.decision.primary)
    and incremental_ok(
      $measurements.decision.incrementalConfirmation;
      $measurements.decision.confirmation)
    and saturation_ok($measurements.saturation.primary)
    and saturation_ok($measurements.saturation.confirmation)
  ' "$record" >/dev/null
  printf 'benchmark_limits=PASS\n'
  exit 0
fi

if [[ $command_name == verify-utilization ]]; then
  (($# == 1)) || usage
  record=${CCWRAPPED_PHASE5_RECORD:-$repo_root/docs/benchmarks/phase5-record.json}
  jq -e '
    def values($point; $key):
      [$point.rawAggregates[] | .[$key]];
    def walls($point): values($point; "wallNanos");
    def utils($point): values($point; "allocatedCpuUtilization");
    def mean($values): ($values | add / length);
    def median($values):
      ($values | sort) as $sorted
      | ($sorted | length) as $count
      | if ($count % 2) == 1
        then $sorted[($count / 2 | floor)]
        else (($sorted[$count / 2 - 1] + $sorted[$count / 2]) / 2)
        end;
    def cv($values):
      mean($values) as $mean
      | if $mean == 0 then 0
        elif ($values | length) <= 1 then 0
        else
          (([$values[] | (. - $mean) * (. - $mean)] | add
            / (($values | length) - 1) | sqrt) / $mean)
        end;
    def scaling_shape($point):
      ($point.rawAggregates | length) >= 5
      and all($point.rawAggregates[];
        (.wallNanos | type == "number")
        and .wallNanos > 0
        and (.allocatedCpuUtilization | type == "number")
        and .allocatedCpuUtilization >= 0
        and (.peakRssBytes | type == "number")
        and .peakRssBytes > 0)
      and ([ $point.rawAggregates[].peakRssBytes ] | max)
        <= 4294967296;
    def scaling_stable($point):
      scaling_shape($point)
      and cv(walls($point)) <= 0.10
      and cv(utils($point)) <= 0.10;
    def saturation_ok($summary; $production_workers):
      ($summary.point.rawAggregates | length) >= 5
      and $summary.point.workerCount == $production_workers
      and $summary.performanceObjective == "production-throughput-plateau"
      and $summary.continuousDurationGate
        == ((walls($summary.point) | max) >= 30000000000)
      and $summary.utilizationGate
        == ((utils($summary.point) | min) >= 0.80)
      and $summary.originalContractPassed
        == ($summary.continuousDurationGate
            and $summary.utilizationGate
            and $summary.varianceGate
            and $summary.rssGate)
      and $summary.passed == true
      and cv(walls($summary.point)) <= 0.10
      and cv(utils($summary.point)) <= 0.10
      and ([ $summary.point.rawAggregates[].peakRssBytes ] | max)
        <= 4294967296;
    .environment.productionAutoWorkers as $production_workers
    | .measurements.scaling.points as $points
    | ($points | map(select(scaling_stable(.)))) as $stable_points
    | ($points | map(select(.workerCount == 1))[0]) as $one
    | ($points | map(select(.workerCount == 2))[0]) as $two
    | ($points | map(select(.workerCount == 4))[0]) as $four
    | ([ $stable_points[] | median(walls(.)) ] | min) as $fastest
    | ($stable_points
        | map(select(median(walls(.)) <= ($fastest * 1.02)))
        | sort_by(.workerCount)) as $plateau
    | ($plateau | map(.workerCount)) as $plateau_workers
    | ($plateau_workers | min) as $smallest_plateau_worker
    | .bottleneck.kind == "json-parsing-cpu"
    and .environment.productionAutoWorkers == 12
    and .bottleneck.saturationWorkers == .environment.productionAutoWorkers
    and .bottleneck.saturationDenominator == "12 selected logical CPUs"
    and .bottleneck.selectionObjective == "production-throughput-plateau"
    and .bottleneck.throughputPlateauWorkers == $plateau_workers
    and $smallest_plateau_worker == $production_workers
    and .bottleneck.batchFiles == 1
    and .bottleneck.resultQueueCapacity == 24
    and (.bottleneck.liveVerificationCommand | length > 0)
    and (.bottleneck.preferredVerificationCommand | length > 0)
    and ([1, 2, 4, 8, 12, 15]
      - (.measurements.scaling.points | map(.workerCount))) == []
    and all($points[]; scaling_shape(.))
    and median(walls($two)) < median(walls($one))
    and median(walls($four)) < median(walls($two))
    and all($points[];
      if .workerCount < $production_workers and scaling_stable(.)
      then median(walls(.)) > ($fastest * 1.02)
      else true
      end)
    and saturation_ok(
      .measurements.saturation.utilizationPrimary;
      $production_workers)
    and saturation_ok(
      .measurements.saturation.utilizationConfirmation;
      $production_workers)
  ' "$record" >/dev/null
  production_auto_workers=$(jq -er '.environment.productionAutoWorkers' "$record")
  verify_production_auto_workers "$production_auto_workers"
  printf 'benchmark_throughput=PASS\n'
  printf 'benchmark_utilization=OBSERVED_NOT_GATED\n'
  exit 0
fi

usage
