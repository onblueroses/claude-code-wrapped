#!/usr/bin/env python3
"""Reference implementation of the ccwrapped metrics.

This is the executable specification the Bend engine is tested against. It favours
obviousness over speed: every transcript line is parsed with the standard JSON parser and
every rule below is applied literally. See docs/semantics.md for the rules in prose.
"""

import argparse
import datetime as dt
import json
import re
import os
import sys
from collections import Counter, defaultdict
from pathlib import Path
from zoneinfo import ZoneInfo

REPO = Path(__file__).resolve().parent.parent
PICO_PER_MICRO = 1_000_000  # cost unit: picodollars; prices are microdollars per million tokens
WEB_SEARCH_MICRODOLLARS = 10_000  # $10 per 1,000 searches; web fetch is free
CATEGORIES = ("input", "cache_write_5m", "cache_write_1h", "cache_read", "output")
ASCII_SPACE = " \t\n\r\x0b\x0c"
GENERATED_PREFIXES = (
    "<local-command-stdout>",
    "<local-command-stderr>",
    "<local-command-caveat>",
    "[Request interrupted by user",
    "<task-notification>",
    "<bash-stdout>",
    "<bash-stderr>",
)


def reject_constant(name):
    raise ValueError(f"non-standard JSON constant {name}")


def load_prices():
    prices = {}
    for line in (REPO / "pricing" / "prices.tsv").read_text().splitlines():
        if not line or line.startswith("#"):
            continue
        model, speed, *values = line.split("\t")
        prices[(model, speed)] = dict(zip(CATEGORIES, map(int, values)))
    return prices


def price_row(prices, model, speed, geo_us):
    """The price row for a request, or None when the model is unpriced."""
    base = model
    tail = model.rsplit("-", 1)
    if len(tail) == 2 and len(tail[1]) == 8 and tail[1].isdigit():
        base = tail[0]
    row = prices.get((base, speed)) or prices.get((base, "standard"))
    if row is None:
        return None
    if geo_us:
        assert all(v * 11 % 10 == 0 for v in row.values())
        row = {k: v * 11 // 10 for k, v in row.items()}
    return row


# No response comes near a billion tokens, even with a 1M-token context; a larger count is not
# a usage figure and reads as 0, like any other value that is not a non-negative integer.
PLAUSIBLE = 1_000_000_000


def as_int(value):
    ok = isinstance(value, int) and not isinstance(value, bool) and 0 <= value <= PLAUSIBLE
    return value if ok else 0


def complete_lines(path):
    """Complete (newline-terminated) lines of a file; a live file's partial tail is ignored."""
    with open(path, "rb") as handle:
        data = handle.read()
    end = data.rfind(b"\n")
    return data[:end].split(b"\n") if end >= 0 else []


def transcript_files(roots):
    for root in roots:
        for directory, _, names in os.walk(root):
            for name in sorted(names):
                if name.endswith(".jsonl"):
                    yield os.path.join(directory, name)


def is_human_text(text):
    return isinstance(text, str) and text.strip(ASCII_SPACE) != "" and not text.startswith(GENERATED_PREFIXES)


def human_prompt(message):
    content = message.get("content")
    if isinstance(content, str):
        return is_human_text(content)
    if isinstance(content, list):
        return any(
            isinstance(block, dict) and block.get("type") == "text" and is_human_text(block.get("text"))
            for block in content
        )
    return False


def display_project(cwd, home):
    if not isinstance(cwd, str) or cwd == "":
        return "(unknown)"
    if home and (cwd == home or cwd.startswith(home + "/")):
        return "~" + cwd[len(home):]
    return cwd


def scan(roots, cutoff=None):
    """Fold every transcript line into order-independent group records.

    Files last modified before cutoff (epoch seconds) are skipped: a line is written after
    the moment its timestamp records, and a copied line keeps its original timestamp, so such
    a file cannot hold anything of a period that starts after the cutoff.
    """
    groups = {}  # message id -> merged request record
    tools = {}  # tool_use id -> least (timestamp, name, message id) among its copies
    prompts = {}  # user uuid -> earliest timestamp
    stats = dict.fromkeys(("files", "lines", "malformed_lines", "usage_lines"), 0)
    for path in transcript_files(roots):
        if cutoff is not None and int(os.stat(path).st_mtime) < cutoff:
            continue
        stats["files"] += 1
        for raw in complete_lines(path):
            stats["lines"] += 1
            try:
                record = json.loads(raw.decode("utf-8"), parse_constant=reject_constant)
            except ValueError:
                stats["malformed_lines"] += 1
                continue
            if not isinstance(record, dict):
                stats["malformed_lines"] += 1
                continue
            kind = record.get("type")
            message = record.get("message") if isinstance(record.get("message"), dict) else {}
            timestamp = record.get("timestamp")
            if not isinstance(timestamp, str):
                continue
            if kind == "assistant" and isinstance(message.get("usage"), dict) and isinstance(message.get("id"), str):
                stats["usage_lines"] += 1
                usage = message["usage"]
                split = usage.get("cache_creation") if isinstance(usage.get("cache_creation"), dict) else {}
                server = usage.get("server_tool_use") if isinstance(usage.get("server_tool_use"), dict) else {}
                line = {
                    "input": as_int(usage.get("input_tokens")),
                    "output": as_int(usage.get("output_tokens")),
                    "cache_read": as_int(usage.get("cache_read_input_tokens")),
                    "cache_write": as_int(usage.get("cache_creation_input_tokens")),
                    "cache_write_1h": as_int(split.get("ephemeral_1h_input_tokens")),
                    "cache_write_5m": as_int(split.get("ephemeral_5m_input_tokens")),
                    "web_search": as_int(server.get("web_search_requests")),
                    "web_fetch": as_int(server.get("web_fetch_requests")),
                    "fast": usage.get("speed") == "fast",
                    "geo_us": usage.get("inference_geo") == "us",
                    # Attribution comes from the least (timestamp, session, cwd, model, sidechain) line.
                    "origin": (
                        timestamp,
                        record.get("sessionId") if isinstance(record.get("sessionId"), str) else "",
                        record.get("cwd") if isinstance(record.get("cwd"), str) else "",
                        message.get("model") if isinstance(message.get("model"), str) else "",
                        record.get("isSidechain") is True,
                    ),
                }
                previous = groups.get(message["id"])
                if previous is None:
                    groups[message["id"]] = line
                else:
                    for key in ("input", "output", "cache_read", "cache_write", "cache_write_1h", "cache_write_5m", "web_search", "web_fetch"):
                        previous[key] = max(previous[key], line[key])
                    previous["fast"] = previous["fast"] or line["fast"]
                    previous["geo_us"] = previous["geo_us"] or line["geo_us"]
                    previous["origin"] = min(previous["origin"], line["origin"])
                content = message.get("content")
                if isinstance(content, list):
                    for block in content:
                        if isinstance(block, dict) and block.get("type") == "tool_use" and isinstance(block.get("id"), str):
                            name = block.get("name") if isinstance(block.get("name"), str) else ""
                            entry = (timestamp, name, message["id"])
                            tools[block["id"]] = min(tools.get(block["id"], entry), entry)
            elif kind == "user" and isinstance(record.get("uuid"), str):
                if record.get("isMeta") is True or record.get("isSidechain") is True:
                    continue
                if human_prompt(message):
                    uuid = record["uuid"]
                    prompts[uuid] = min(prompts.get(uuid, timestamp), timestamp)
    return groups, tools, prompts, stats


TIMESTAMP = re.compile(r"(\d{4})-(\d{2})-(\d{2})T(\d{2}):(\d{2}):(\d{2})(?:\.\d+)?(?:(Z)|([+-])(\d{2}):(\d{2}))")


def parse_timestamp(value):
    """An RFC 3339 instant with a seconds field and an explicit offset; fractions are dropped."""
    match = TIMESTAMP.fullmatch(value)
    if match is None:
        return None
    year, month, day, hour, minute, second = (int(match.group(i)) for i in range(1, 7))
    offset = dt.timedelta(0)
    if match.group(7) is None:
        offset = dt.timedelta(hours=int(match.group(9)), minutes=int(match.group(10)))
        if match.group(8) == "-":
            offset = -offset
    try:
        moment = dt.datetime(year, month, day, hour, minute, second, tzinfo=dt.timezone(offset))
    except ValueError:
        return None
    return moment


def report(roots, year, zone, home):
    prices = load_prices()
    # Two days before 1 January UTC covers every UTC offset.
    cutoff = int(dt.datetime(year, 1, 1, tzinfo=dt.timezone.utc).timestamp()) - 2 * 86400
    groups, tools, prompts, stats = scan(roots, cutoff)
    tz = ZoneInfo(zone)

    totals = dict.fromkeys(CATEGORIES, 0)
    cost = 0
    requests = 0
    by_model = defaultdict(lambda: {"requests": 0, "cost": 0, **dict.fromkeys(CATEGORIES, 0)})
    by_project = defaultdict(lambda: {"requests": 0, "cost": 0, "tokens": 0, "sessions": set()})
    by_day = defaultdict(lambda: {"requests": 0, "cost": 0, "output": 0})
    hours = [0] * 24
    weekdays = [0] * 7
    sessions = set()
    unpriced = Counter()
    counts = dict.fromkeys(
        (
            "bad_timestamp_requests",
            "cache_write_unsplit_tokens",
            "fast_requests",
            "sidechain_requests",
            "web_fetch_requests",
            "web_search_requests",
        ),
        0,
    )
    in_period = set()

    for message_id, group in groups.items():
        timestamp, session, cwd, model, sidechain = group["origin"]
        moment = parse_timestamp(timestamp)
        if moment is None:
            counts["bad_timestamp_requests"] += 1
            continue
        local = moment.astimezone(tz)
        if local.year != year:
            continue
        in_period.add(message_id)
        write_1h = min(group["cache_write_1h"], group["cache_write"])
        tokens = {
            "input": group["input"],
            "cache_write_5m": group["cache_write"] - write_1h,
            "cache_write_1h": write_1h,
            "cache_read": group["cache_read"],
            "output": group["output"],
        }
        if group["cache_write_5m"] + group["cache_write_1h"] != group["cache_write"]:
            # The total is authoritative; tokens outside the reported split bill at the 5m rate.
            counts["cache_write_unsplit_tokens"] += abs(group["cache_write"] - group["cache_write_5m"] - group["cache_write_1h"])
        requests += 1
        for key in CATEGORIES:
            totals[key] += tokens[key]
        row = None if model == "<synthetic>" else price_row(prices, model, "fast" if group["fast"] else "standard", group["geo_us"])
        request_cost = 0
        if row is not None:
            request_cost = sum(tokens[key] * row[key] for key in CATEGORIES)
            request_cost += group["web_search"] * WEB_SEARCH_MICRODOLLARS * PICO_PER_MICRO
        elif model != "<synthetic>":
            unpriced[model] += 1
        cost += request_cost
        entry = by_model[model]
        entry["requests"] += 1
        entry["cost"] += request_cost
        for key in CATEGORIES:
            entry[key] += tokens[key]
        project = display_project(cwd, home)
        by_project[project]["requests"] += 1
        by_project[project]["cost"] += request_cost
        by_project[project]["tokens"] += sum(tokens.values())
        if session:
            by_project[project]["sessions"].add(session)
            sessions.add(session)
        day = by_day[local.date().isoformat()]
        day["requests"] += 1
        day["cost"] += request_cost
        day["output"] += tokens["output"]
        hours[local.hour] += 1
        weekdays[local.weekday()] += 1
        counts["web_search_requests"] += group["web_search"]
        counts["web_fetch_requests"] += group["web_fetch"]
        counts["fast_requests"] += group["fast"]
        counts["sidechain_requests"] += sidechain

    tool_calls = Counter()
    for timestamp, name, _ in tools.values():
        moment = parse_timestamp(timestamp)
        if moment is not None and moment.astimezone(tz).year == year:
            tool_calls[name] += 1
    human = 0
    for timestamp in prompts.values():
        moment = parse_timestamp(timestamp)
        if moment is not None and moment.astimezone(tz).year == year:
            human += 1

    days = sorted(by_day)
    streak = best = 0
    previous = None
    for day in days:
        current = dt.date.fromisoformat(day)
        streak = streak + 1 if previous is not None and (current - previous).days == 1 else 1
        best = max(best, streak)
        previous = current

    return {
        "schema": "ccwrapped.metrics/v1",
        "period": {"year": year, "timezone": zone},
        "requests": requests,
        "tokens": totals,
        "cost_picodollars": cost,
        "models": {model: dict(value) for model, value in sorted(by_model.items())},
        "unpriced_models": dict(sorted(unpriced.items())),
        "projects": {
            project: {**{k: v for k, v in value.items() if k != "sessions"}, "sessions": len(value["sessions"])}
            for project, value in sorted(by_project.items())
        },
        "days": {day: by_day[day] for day in days},
        "active_days": len(days),
        "longest_streak_days": best,
        "hours": hours,
        "weekdays_monday_first": weekdays,
        "sessions": len(sessions),
        "human_prompts": human,
        "tool_calls": dict(sorted(tool_calls.items())),
        "counts": dict(sorted(counts.items())),
        "scan": dict(sorted(stats.items())),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", action="append", help="transcript root (repeatable)")
    parser.add_argument("--year", type=int, default=dt.date.today().year)
    parser.add_argument("--tz", default=None, help="IANA zone; defaults to the system zone")
    args = parser.parse_args()
    home = os.path.expanduser("~")
    roots = args.root or [os.path.join(os.environ.get("CLAUDE_CONFIG_DIR", os.path.join(home, ".claude")), "projects")]
    zone = args.tz or os.environ.get("TZ") or str(Path("/etc/localtime").resolve()).split("zoneinfo/")[-1]
    json.dump(report(roots, args.year, zone, home), sys.stdout, indent=1, sort_keys=True)
    sys.stdout.write("\n")


if __name__ == "__main__":
    main()
