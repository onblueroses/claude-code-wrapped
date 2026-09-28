#!/usr/bin/env python3
"""Command-line test: options, defaults and errors.

usage: tests/cli.py ENGINE ROOT
"""

import datetime as dt
import json
import subprocess
import sys
import tempfile
from zoneinfo import ZoneInfo

ENGINE, ROOT = sys.argv[1], sys.argv[2]
ENV = {"HOME": "/home/u", "PATH": "/usr/bin", "TZ": "UTC"}
problems = []


def run(*args, env=None):
    return subprocess.run([ENGINE, *args], capture_output=True, text=True, env=env or ENV)


def expect(args, code, out=None, err=None, env=None):
    r = run(*args, env=env)
    what = " ".join(args) or "(no arguments)"
    if r.returncode != code:
        problems.append(f"{what}: exit {r.returncode}, want {code} ({r.stderr.strip()[:120]})")
    if out is not None and out not in r.stdout:
        problems.append(f"{what}: stdout lacks {out!r}")
    if err is not None and err not in r.stderr:
        problems.append(f"{what}: stderr lacks {err!r}: {r.stderr.strip()[:120]}")
    return r


def year_of(args):
    return json.loads(expect([*args, "--json"], 0).stdout)["period"]["year"]


expect(["--version"], 0, out="ccwrapped ")
expect(["--help"], 0, out="usage: ccwrapped")
expect(["-h"], 0, out="usage: ccwrapped")
expect(["--bogus"], 2, err="unknown option '--bogus'")
expect(["--year"], 2, err="--year wants a value")
expect(["--year", "26"], 2, err="four-digit year")
expect(["--year", "2026x"], 2, err="four-digit year")
expect(["--jobs", "0"], 2, err="from 1 to 256")
expect(["--jobs", "999"], 2, err="from 1 to 256")
expect(["--json", "--html"], 2, err="choose one of")
expect(["--json=yes"], 2, err="takes no value")
expect(["--root", ROOT, "--tz", "Mars/Olympus"], 2, err="unknown time zone 'Mars/Olympus'")
expect(["--root", ROOT, "--tz", "../../etc/passwd"], 2, err="unknown time zone")
expect(["--root", "/nonexistent/ccwrapped"], 1, err="cannot read transcripts at /nonexistent/ccwrapped")
with tempfile.TemporaryDirectory() as empty:
    expect(["--root", empty], 1, err="no transcripts")
    expect([], 1, err=f"cannot read transcripts at {empty}/projects", env={**ENV, "CLAUDE_CONFIG_DIR": empty})

# The default year is the current year in the report's time zone; forms of --year agree.
for zone in ("Pacific/Kiritimati", "Pacific/Pago_Pago"):
    now = dt.datetime.now(ZoneInfo(zone)).year
    got = year_of(["--root", ROOT, "--tz", zone])
    if got != now:
        problems.append(f"default year in {zone}: {got}, want {now}")
for args in (["2025"], ["--year", "2025"], ["--year=2025"]):
    if year_of(["--root", ROOT, *args]) != 2025:
        problems.append(f"{' '.join(args)}: not the 2025 report")
if year_of(["--root", ROOT, "--tz", "UTC"]) != year_of(["--root", ROOT]):
    problems.append("$TZ is not the default zone")
same = {expect(["--root", ROOT, "--year", "2026", *f], 0).stdout for f in ([], ["--markdown"], ["--html"], ["--card"], ["--json"])}
if len(same) != 5:
    problems.append("the five formats are not five different outputs")

# A year without responses still renders every format.
for f in ([], ["--markdown"], ["--html"], ["--card"]):
    r = expect(["--root", ROOT, "--year", "1999", *f], 0)
    if "1999" not in r.stdout:
        problems.append(f"empty year {' '.join(f)}: no year in the output")

for p in problems:
    print(p)
print(f"cli: {len(problems)} problems")
sys.exit(1 if problems else 0)
