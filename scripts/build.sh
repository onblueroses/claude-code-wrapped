#!/usr/bin/env bash
# Builds the ccwrapped binary from bend/main.bend, by default to ./ccwrapped.
#
# BEND_NO_TELEMETRY=1 keeps the installed Bend release fixed for the build (the launcher
# otherwise updates itself); set it to 0 to allow updates.
set -euo pipefail
cd "$(dirname "$0")/.."
out=${1:-ccwrapped}
rm -f "$out"
python3 bend/gen.py --check || { echo "bend/gen/ is stale: run python3 bend/gen.py" >&2; exit 1; }
BEND_NO_TELEMETRY=${BEND_NO_TELEMETRY:-1} bend bend/main.bend -o "$out" 2>&1 \
  | grep -v -e '^- ' -e 'rely on unsafe' -e 'PROOFS' || true
test -x "$out" || { echo "build failed" >&2; exit 1; }
echo "built $out"
