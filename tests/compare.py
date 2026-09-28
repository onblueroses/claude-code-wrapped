#!/usr/bin/env python3
"""Differential test: the Bend engine's metrics must equal the oracle's, field for field.

usage: tests/compare.py ENGINE ROOT [--year Y] [--tz ZONE]
"""

import argparse
import json
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "oracle"))
import oracle  # noqa: E402


def diff(a, b, path="$", out=None):
    out = [] if out is None else out
    if isinstance(a, dict) and isinstance(b, dict):
        for key in sorted(set(a) | set(b)):
            if key not in a:
                out.append(f"{path}.{key}: missing in engine (oracle {b[key]!r:.80})")
            elif key not in b:
                out.append(f"{path}.{key}: extra in engine ({a[key]!r:.80})")
            else:
                diff(a[key], b[key], f"{path}.{key}", out)
    elif isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
        for i, (x, y) in enumerate(zip(a, b)):
            diff(x, y, f"{path}[{i}]", out)
    elif a != b:
        out.append(f"{path}: engine {a!r:.80} != oracle {b!r:.80}")
    return out


def main():
    sys.stdout.reconfigure(errors="backslashreplace")
    ap = argparse.ArgumentParser()
    ap.add_argument("engine")
    ap.add_argument("root")
    ap.add_argument("--year", type=int, default=2026)
    ap.add_argument("--tz", default="America/Los_Angeles")
    ap.add_argument("--home", default=str(Path.home()))
    args = ap.parse_args()
    t0 = time.time()
    run = subprocess.run([args.engine, "--json", "--year", str(args.year), "--tz", args.tz, "--root", args.root], capture_output=True, text=True, env={"HOME": args.home, "PATH": "/usr/bin"})
    t1 = time.time()
    if run.returncode != 0:
        print(run.stderr)
        sys.exit(f"engine failed with {run.returncode}")
    engine = json.loads(run.stdout)
    expected = oracle.report([args.root], args.year, args.tz, args.home)
    t2 = time.time()
    problems = diff(engine, json.loads(json.dumps(expected)))
    print(f"engine {t1 - t0:.1f}s, oracle {t2 - t1:.1f}s")
    for line in problems[:60]:
        print(line)
    print(f"{len(problems)} differences")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
