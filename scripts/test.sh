#!/usr/bin/env bash
# The full check: generated code is current, the laws hold (kernel verdict), the engine agrees
# with the Python oracle on the edge-case corpus in zones with unusual offsets, every format
# for people shows the oracle's figures, and the command line behaves.
# With a transcript root as $1, also runs both comparisons on that history (for the current
# year, in $TZ or UTC).
set -euo pipefail
cd "$(dirname "$0")/.."
export BEND_NO_TELEMETRY=${BEND_NO_TELEMETRY:-1}

python3 bend/gen.py --check

laws=$(cd bend && bend PROOF.bend --verdict 2>&1) || true
[ "$(printf '%s\n' "$laws" | tail -n 1)" = "ALL PROOFS CHECK" ] || { printf 'laws failed:\n%s\n' "$laws" | tail -n 40 >&2; exit 1; }
echo "laws: ALL PROOFS CHECK"

mkdir -p .local
scripts/build.sh .local/ccwrapped >/dev/null
python3 tests/make_fixtures.py
for zone in America/Los_Angeles UTC Asia/Kolkata Pacific/Chatham; do
  printf 'metrics, edge corpus, %s: ' "$zone"
  python3 tests/compare.py .local/ccwrapped tests/fixtures/edge --tz "$zone" --home /home/u | tail -n 1
done
for zone in UTC Pacific/Chatham; do
  printf 'formats, edge corpus, %s: ' "$zone"
  python3 tests/views.py .local/ccwrapped tests/fixtures/edge --tz "$zone" --home /home/u | tail -n 1
done
python3 tests/cli.py .local/ccwrapped tests/fixtures/edge

if [ $# -gt 0 ]; then
  year=$(date +%Y)
  printf 'metrics, history %s: ' "$1"
  python3 tests/compare.py .local/ccwrapped "$1" --year "$year" --tz "${TZ:-UTC}" | tail -n 2 | tr '\n' ' '
  echo
  printf 'formats, history %s: ' "$1"
  python3 tests/views.py .local/ccwrapped "$1" --year "$year" --tz "${TZ:-UTC}" | tail -n 1
fi
