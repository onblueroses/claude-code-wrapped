#!/usr/bin/env python3
"""Presentation test: what the formats for people show must agree with the oracle.

compare.py proves the engine's metrics equal the oracle's. This test proves the terminal
report, the Markdown report, the HTML slides and the share card show those metrics correctly:
every figure is formatted here, independently, from the oracle's metrics and looked up in each
output. It also checks that names from transcripts come out as text (no markup, no terminal
control) and that --share and the card leak no project path, MCP server or time zone.

usage: tests/views.py ENGINE ROOT [--year Y] [--tz ZONE] [--home DIR]
"""

import argparse
import datetime as dt
import html
import html.parser
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(REPO / "oracle"))
import oracle  # noqa: E402

WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"]
MODELS_SHOWN, PROJECTS_SHOWN, TOOLS_SHOWN, TEXT_ROWS = 16, 8, 10, 12


# Formatting, written from docs/semantics.md and the report's conventions
# -----------------------------------------------------------------------

def fmt_int(n):
    return f"{n:,}"


def fmt_usd(pico):
    cents = (pico // 10**6 + 5000) // 10000
    return f"${cents // 100:,}.{cents % 100:02d}"


def fmt_pct(part, whole):
    if whole == 0:
        return "-"
    tenths = (part * 1000 + whole // 2) // whole
    return f"{tenths // 10}.{tenths % 10}%"


def fmt_big(n):
    for unit, suffix in ((10**9, "B"), (10**6, "M"), (10**3, "K")):
        if n >= unit:
            tenths = (n * 10 + unit // 2) // unit
            return f"{tenths // 10}.{tenths % 10}{suffix}"
    return str(n)


def plural(n, one, many):
    return one if n == 1 else many


def clean(s):
    """Control characters and lone surrogates show as U+FFFD."""
    return "".join("�" if ord(c) < 32 or ord(c) == 127 or 128 <= ord(c) < 160 or 0xD800 <= ord(c) <= 0xDFFF else c for c in s)


def clip(s, w):
    return "..." + s[len(s) - (w - 3):] if len(s) > w else s


def pretty(model):
    if not (model.startswith("claude-") and len(model) > 7):
        return model or "(no model)"
    words = model.split("-")[1:]
    if words and len(words[-1]) == 8 and words[-1].isdigit():
        words = words[:-1]
    out, prev_num = "", False
    for w in words:
        num = w.isdigit()
        word = w[:1].upper() + w[1:]
        out = word if not out else out + ("." if prev_num and num else " ") + word
        prev_num = num
    return out


def micro(pico):
    return pico // 10**6


# What the report shows
# ---------------------

def expected(m, share):
    """The figures every format shows, from the oracle's metrics."""
    cost = m["cost_picodollars"]
    toks = m["tokens"]
    total_toks = sum(toks.values())
    models = sorted(m["models"].items(), key=lambda kv: (-kv[1]["cost"], kv[0]))
    projects = sorted(m["projects"].items(), key=lambda kv: (-kv[1]["cost"], kv[0]))
    if share:
        projects = [(f"project-{i}", v) for i, (_, v) in enumerate(projects, 1)]
    tools = sorted(m["tool_calls"].items(), key=lambda kv: (-kv[1], kv[0]))
    calls = sum(m["tool_calls"].values())
    if share:
        mcp = sum(n for name, n in tools if name.startswith("mcp__"))
        tools = [(name, n) for name, n in tools if not name.startswith("mcp__")]
        if mcp:
            tools = sorted(tools + [("MCP tools", mcp)], key=lambda kv: (-kv[1], kv[0]))
    days = sorted(m["days"].items())
    busiest = None
    for date, d in days:
        if busiest is None or d["cost"] > busiest[1]["cost"]:
            busiest = (date, d)
    hours, weekdays = m["hours"], m["weekdays_monday_first"]
    return {
        "cost": cost, "toks": toks, "total_toks": total_toks, "models": models,
        "unpriced": m["unpriced_models"], "projects": projects, "tools": tools, "calls": calls,
        "days": days, "busiest": busiest, "hours": hours, "weekdays": weekdays,
        "peak_hour": hours.index(max(hours)), "peak_weekday": weekdays.index(max(weekdays)),
    }


class Page(html.parser.HTMLParser):
    """Collects data-m values, heat cells, tags and attributes of an HTML page."""

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.metrics, self.stack, self.tags, self.attrs, self.heat, self.text = {}, [], set(), [], [], []
        self.in_heat = 0

    def handle_starttag(self, tag, attrs):
        a = dict(attrs)
        self.tags.add(tag)
        self.attrs.extend(attrs)
        if tag == "div" and a.get("class") == "heat":
            self.in_heat = 1
        elif tag == "i" and self.in_heat and a.get("class") != "out":
            self.heat.append((a.get("class") or "l0", a.get("title", "")))
        if tag != "meta":
            self.stack.append([tag, a.get("data-m"), ""])

    def handle_endtag(self, tag):
        if not self.stack or self.stack[-1][0] != tag:
            raise AssertionError(f"mismatched </{tag}> in {[t for t, _, _ in self.stack[-4:]]}")
        tag, key, text = self.stack.pop()
        if key is not None:
            self.metrics.setdefault(key, []).append(text)
        if self.stack:
            self.stack[-1][2] += text
        if tag == "div" and self.in_heat:
            self.in_heat = 0

    def handle_data(self, data):
        self.text.append(data)
        if self.stack:
            self.stack[-1][2] += data


def parse(page, c):
    """The parsed page; a page that does not nest is a problem and parses as empty."""
    p = Page()
    try:
        p.feed(page)
        p.close()
        assert not p.stack, f"unclosed tags: {[t for t, _, _ in p.stack]}"
    except AssertionError as err:
        c.ok(f"malformed HTML: {err}", False)
        return Page()
    return p


class Check:
    def __init__(self, name):
        self.name, self.problems, self.count = name, [], 0

    def eq(self, what, got, want):
        self.count += 1
        if got != want:
            self.problems.append(f"{self.name}: {what}: shows {got!r:.120}, oracle says {want!r:.120}")

    def ok(self, what, cond):
        self.count += 1
        if not cond:
            self.problems.append(f"{self.name}: {what}")


def check_metrics(c, page, want):
    """Every data-m on the page shows the value in want; every value in want is on the page."""
    for key, values in page.metrics.items():
        if key not in want:
            c.ok(f"unexpected metric {key!r}", False)
            continue
        for v in values:
            c.eq(key, v, want[key])
    for key in want:
        c.ok(f"metric {key!r} is missing", key in page.metrics)


def heat_cells(e, year):
    top = max((micro(d["cost"]) for _, d in e["days"]), default=0)
    days = dict(e["days"])
    cells = []
    day = dt.date(year, 1, 1)
    while day.year == year:
        iso = day.isoformat()
        if iso in days:
            d = days[iso]
            c = micro(d["cost"])
            level = 1 if top == 0 else max(1, min(4, (c * 4 + top - 1) // top))
            cells.append((f"l{level}", f"{iso} · {fmt_usd(d['cost'])} · {fmt_int(d['requests'])}{plural(d['requests'], ' response', ' responses')}"))
        else:
            cells.append(("l0", iso))
        day += dt.timedelta(days=1)
    return cells


def common(m, e):
    return {
        "year": str(m["period"]["year"]),
        "cost": fmt_usd(e["cost"]),
        "requests": fmt_int(m["requests"]),
        "human": fmt_int(m["human_prompts"]),
        "tokens_total": fmt_big(e["total_toks"]),
        "active_days": fmt_int(m["active_days"]),
    }


def check_slides(c, page, m, e, zone, share):
    p = parse(page, c)
    total_micro = micro(e["cost"])
    want = common(m, e)
    want.update({
        "files": fmt_int(m["scan"]["files"]),
        "zone": zone,
        "tool_calls": fmt_int(e["calls"]),
        "sessions": fmt_int(m["sessions"]),
        "cache_read_share": fmt_pct(e["toks"]["cache_read"], e["total_toks"]),
        "streak": fmt_int(m["longest_streak_days"]),
    })
    for k in ("input", "cache_write_5m", "cache_write_1h", "cache_read", "output"):
        want[f"tok:{k}"] = fmt_int(e["toks"][k])
    if e["models"]:
        want["top_model"] = pretty(e["models"][0][0])
    for name, v in e["models"][:MODELS_SHOWN]:
        want[f"model:{name}"] = "unpriced" if name in e["unpriced"] else fmt_usd(v["cost"])
    if e["projects"]:
        want["top_project"] = e["projects"][0][0]
    for name, v in e["projects"][:PROJECTS_SHOWN]:
        want[f"project:{name}"] = fmt_usd(v["cost"])
    for name, n in e["tools"][:TOOLS_SHOWN]:
        want[f"tool:{name}"] = fmt_int(n)
    if e["busiest"]:
        want["busiest_day"] = e["busiest"][0]
        want["busiest_day_cost"] = fmt_usd(e["busiest"][1]["cost"])
    if m["requests"]:
        want["peak_hour"] = f"{e['peak_hour']:02d}:00"
        want["peak_weekday"] = WEEKDAYS[e["peak_weekday"]] + "s"
    # Keys hold names from transcripts; the parser has already decoded their escapes.
    want = {clean(k): clean(v) for k, v in want.items()}
    check_metrics(c, p, want)
    c.eq("calendar", p.heat, heat_cells(e, m["period"]["year"]))
    # Share of cost in each model row's caption.
    captions = re.findall(r"of the cost</span>", page)
    c.ok("model and project captions", len(captions) == min(len(e["models"]), MODELS_SHOWN) + min(len(e["projects"]), PROJECTS_SHOWN))
    for name, v in e["models"][:MODELS_SHOWN]:
        c.ok(f"share of {name!r}", f"{fmt_pct(micro(v['cost']), total_micro)} of the cost" in page)
    return p


def check_card(c, page, m, e, zone):
    p = parse(page, c)
    want = common(m, e)
    want["streak"] = fmt_int(m["longest_streak_days"]) + plural(m["longest_streak_days"], " day", " days")
    want["top_model"] = clean(pretty(e["models"][0][0])) if e["models"] else "none"
    check_metrics(c, p, want)
    c.eq("calendar", p.heat, heat_cells(e, m["period"]["year"]))
    text = "".join(p.text) + " ".join(v for _, v in p.attrs if v)
    for name in m["projects"]:
        if len(name) > 3:
            c.ok(f"card shows project {name!r:.60}", clean(name) not in text)
    c.ok("card shows an MCP tool", "mcp__" not in text)
    if len(zone) > 3:
        c.ok("card shows the time zone", zone not in text)


def check_safe(c, p):
    """Names came out as text: only the page's own tags and attributes exist."""
    allowed_tags = {"html", "head", "meta", "title", "style", "body", "main", "section", "div", "p", "h1", "h2",
                    "b", "span", "ol", "ul", "li", "i", "dl", "dt", "dd"}
    allowed_attrs = {"id", "lang", "charset", "name", "content", "http-equiv", "class", "data-m", "title", "style", "aria-label"}
    c.ok(f"unexpected tags {p.tags - allowed_tags}", p.tags <= allowed_tags)
    names = {k for k, _ in p.attrs}
    c.ok(f"unexpected attributes {names - allowed_attrs}", names <= allowed_attrs)
    c.ok("a raw control character", not any(clean(t.replace("\n", "")) != t.replace("\n", "") for t in p.text))


def rows(text, header):
    """The rows of the table under a heading of the terminal report, split into cells."""
    lines = text.split("\n")
    i = lines.index(header) + 3
    out = []
    while i < len(lines) and lines[i].startswith("  "):
        out.append(re.split(r"\s{2,}", lines[i].strip()))
        i += 1
    return out


def md_cells(line):
    """A Markdown table row's cells, with backslash escapes and entities decoded."""
    cells, cell, chars = [], "", iter(line)
    for ch in chars:
        if ch == "\\":
            cell += "\\" + next(chars)
        elif ch == "|":
            cells.append(cell)
            cell = ""
        else:
            cell += ch
    cells.append(cell)
    return [html.unescape(re.sub(r"\\(.)", r"\1", c.strip())) for c in cells[1:-1]]


def md_rows(text, header):
    lines = text.split("\n")
    i = lines.index(f"## {header}") + 4
    out = []
    while i < len(lines) and lines[i].startswith("| "):
        out.append(md_cells(lines[i]))
        i += 1
    return out


def check_text(c, text, m, e, md):
    c.ok("a raw control character", clean(text.replace("\n", "")) == text.replace("\n", ""))
    get = (lambda h: md_rows(text, h)) if md else (lambda h: rows(text, h))
    totals = dict((r[0], r[1]) for r in get("Totals"))
    c.eq("cost", totals.get("API-equivalent cost"), fmt_usd(e["cost"]))
    c.eq("responses", totals.get("Responses"), fmt_int(m["requests"]))
    c.eq("human prompts", totals.get("Human prompts"), fmt_int(m["human_prompts"]))
    c.eq("tool calls", totals.get("Tool calls"), fmt_int(e["calls"]))
    c.eq("sessions", totals.get("Sessions"), fmt_int(m["sessions"]))
    c.eq("active days", totals.get("Active days"), fmt_int(m["active_days"]))
    streak = m["longest_streak_days"]
    c.eq("streak", totals.get("Longest streak"), fmt_int(streak) + plural(streak, " day", " days"))
    toks = dict((r[0], r[1]) for r in get("Tokens"))
    for label, key in (("Input", "input"), ("Cache writes, 5 min", "cache_write_5m"), ("Cache writes, 1 hour", "cache_write_1h"), ("Cache reads", "cache_read"), ("Output", "output")):
        c.eq(f"tokens {label}", toks.get(label), fmt_int(e["toks"][key]))
    total_micro = micro(e["cost"])
    want_models = [[clean(name or "(no model)"), fmt_int(v["requests"]), fmt_big(sum(v[k] for k in ("input", "cache_write_5m", "cache_write_1h", "cache_read", "output"))),
                    "unpriced" if name in e["unpriced"] else fmt_usd(v["cost"]), fmt_pct(micro(v["cost"]), total_micro)] for name, v in e["models"]]
    c.eq("models", get("Models") if e["models"] else [], want_models)
    want_projects = [[clip(clean(name), 36), fmt_int(v["requests"]), fmt_int(v["sessions"]), fmt_usd(v["cost"]), fmt_pct(micro(v["cost"]), total_micro)]
                     for name, v in e["projects"][:TEXT_ROWS]]
    c.eq("projects", get("Projects") if e["projects"] else [], want_projects)
    want_tools = [[clip(clean(name or "(no name)"), 36), fmt_int(n), fmt_pct(n, e["calls"])] for name, n in e["tools"][:TEXT_ROWS]]
    c.eq("tools", get("Tools") if e["tools"] else [], want_tools)
    if e["busiest"]:
        date, d = e["busiest"]
        c.ok("busiest day", f"Busiest day: {date} ({fmt_usd(d['cost'])}, {fmt_int(d['requests'])} responses)." in text)
        c.ok("peak hour", f"Most responses at {e['peak_hour']:02d}:00 and on {WEEKDAYS[e['peak_weekday']]}s." in text)


def run(engine, args, extra):
    cmd = [engine, "--year", str(args.year), "--tz", args.tz, "--root", args.root, *extra]
    r = subprocess.run(cmd, capture_output=True, env={"HOME": args.home, "PATH": "/usr/bin"})
    if r.returncode != 0:
        sys.exit(f"{' '.join(extra) or 'report'}: engine failed: {r.stderr.decode(errors='replace')}")
    return r.stdout.decode("utf-8")  # strict: every format must be valid UTF-8


def main():
    sys.stdout.reconfigure(errors="backslashreplace")
    ap = argparse.ArgumentParser()
    ap.add_argument("engine")
    ap.add_argument("root")
    ap.add_argument("--year", type=int, default=2026)
    ap.add_argument("--tz", default="America/Los_Angeles")
    ap.add_argument("--home", default=str(Path.home()))
    args = ap.parse_args()
    m = json.loads(json.dumps(oracle.report([args.root], args.year, args.tz, args.home)))
    e, es = expected(m, False), expected(m, True)
    checks = []
    for fmt, md in (([], False), (["--markdown"], True)):
        c = Check(fmt[0] if fmt else "terminal")
        check_text(c, run(args.engine, args, fmt), m, e, md)
        checks.append(c)
    c = Check("--share")
    shared = run(args.engine, args, ["--share"])
    check_text(c, shared, m, es, False)
    checks.append(c)
    c = Check("--html")
    check_safe(c, check_slides(c, run(args.engine, args, ["--html"]), m, e, args.tz, False))
    checks.append(c)
    c = Check("--html --share")
    page = run(args.engine, args, ["--html", "--share"])
    check_safe(c, check_slides(c, page, m, es, args.tz, True))
    for name in m["projects"]:
        if "/" in name and len(name) > 4:
            c.ok(f"shows project {name!r:.60}", clean(name) not in page and clean(name) not in shared)
    c.ok("shows an MCP tool", "mcp__" not in page and "mcp__" not in shared)
    checks.append(c)
    c = Check("--card")
    card = run(args.engine, args, ["--card"])
    check_card(c, card, m, e, args.tz)
    check_safe(c, parse(card, c))
    checks.append(c)
    problems = [p for c in checks for p in c.problems]
    for line in problems[:60]:
        print(line)
    print(f"{sum(c.count for c in checks)} checks, {len(problems)} problems")
    sys.exit(1 if problems else 0)


if __name__ == "__main__":
    main()
