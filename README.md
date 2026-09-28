# claude-code-wrapped

A year of Claude Code in numbers, read from the transcripts on your machine: what the traffic
would have cost at API list prices, which models and projects it went to, how much was cache,
which tools ran, and when you worked. As a terminal report, Markdown, JSON, HTML slides or a
share card. Nothing leaves your machine.

The engine is written in [Bend](https://bend-lang.com/), and the numbers are built to be
exactly right rather than roughly right:

- **One spec, two implementations.** [`docs/semantics.md`](docs/semantics.md) defines every
  metric. [`oracle/oracle.py`](oracle/oracle.py) implements it in plain Python; the Bend engine
  must produce the same metrics field for field, on real histories and on an edge-case corpus
  of malformed lines, duplicate keys, escapes, odd time zones and pricing corner cases.
- **Proved where it matters.** Claude Code writes each API response several times (once per
  streamed block, again when a session is resumed), and the engine reads files in parallel in
  any order. [`bend/LAWS.bend`](bend/LAWS.bend) states that merging those copies is
  commutative, associative and idempotent, so totals cannot depend on duplication, file order
  or sharding, and that every cache-write token is billed exactly once.
  [`bend/PROOF.bend`](bend/PROOF.bend) proves it, checked by Bend's Lean-verified kernel.
- **Exact money.** Costs are integer picodollars from the current price list
  ([`pricing/prices.tsv`](pricing/prices.tsv)), including 5-minute and 1-hour cache writes,
  per-model cache-read rates, fast mode, US-only inference and web search.
- **Checked on screen too.** [`tests/views.py`](tests/views.py) formats every figure the
  terminal report, Markdown, slides and card show, independently, from the oracle's metrics,
  and checks each output against it, along with escaping and what `--share` hides.

```
Claude Code usage, 2026
=======================

Totals
------
  Measure                              Value
  API-equivalent cost                 $18.86
  Responses                               55
  Human prompts                           13
  Tool calls                              13
  ...

Models
------
  Model                          Responses    Tokens          Cost   Share
  claude-sonnet-5                        5      4.0M        $16.20   85.9%
  claude-opus-5                         38     95.6K         $2.28   12.1%
  claude-fable-5-1                       1      1.2M         $0.34    1.8%
  ...
  claude-future-9                        1       163      unpriced    0.0%
```

(From the synthetic test corpus. Your own report also has projects, tools, daily activity and
notes on anything that could not be counted.)

## Quick start

Install Bend and a C compiler (clang 14 or newer); the tests also need Python 3.9 or newer:

```bash
curl -fsSL https://bend-lang.com/install.sh | sh
git clone https://github.com/onblueroses/claude-code-wrapped
cd claude-code-wrapped
scripts/build.sh            # writes ./ccwrapped
./ccwrapped                 # this year, in your local time zone
./ccwrapped --html > wrapped.html   # the same year as slides
./ccwrapped --card > card.html      # a one-page card to share
```

## Formats

Every format goes to standard output.

- The **terminal report** (default) and `--markdown`: totals, tokens, models, projects, tools,
  daily activity, and notes on anything that could not be counted.
- `--html`: ten full-screen slides in one self-contained page: cost, the conversation,
  tokens, models, projects, tools, a calendar of the year, the hours and weekdays you worked,
  and how it was counted. The page has no scripts and a content security policy that forbids
  every request, so it cannot load or send anything.
- `--card`: a 4:5 card with the headline numbers and the calendar. It never shows project
  names, tool names or your time zone.
- `--json`: every metric (`ccwrapped.metrics/v1`), with exact picodollar costs.

## Flags

| Flag | Effect |
|---|---|
| `YEAR`, `--year YYYY` | Report year (default: the current year in the report time zone) |
| `--tz ZONE` | IANA time zone for days and hours (default: `$TZ`, else the system zone) |
| `--root DIR` | Transcript directory; repeatable (default: `$CLAUDE_CONFIG_DIR/projects` or `~/.claude/projects`) |
| `--markdown` | The report as Markdown |
| `--html` | The report as HTML slides |
| `--card` | A share card as HTML |
| `--json` | Every metric as JSON (`ccwrapped.metrics/v1`) |
| `--share` | Show projects as `project-1`, `project-2`, ... and all MCP tools as one entry |
| `--jobs N` | Parallel reader processes (default: one per CPU thread, at most 16) |
| `-h`, `--help`, `--version` | Usage and version |

## What it counts

In short (the full rules are in [`docs/semantics.md`](docs/semantics.md)):

- A **response** is one API call. Its copies across lines and files are merged by message id,
  taking the highest count of each kind, since output grows as a response streams.
- **Cost** is the API-equivalent value at first-party list prices, not what a subscription is
  billed. Models without a listed price are shown as unpriced and left out of the total.
- **Human prompts** are messages you typed, not tool results, subagent instructions or text
  Claude Code writes itself.
- A file still being written is read up to its length when opened; malformed lines, unreadable
  timestamps and cache writes without a duration breakdown are counted and reported, not
  silently dropped.

## How it works

`bend/main.bend` lists the transcripts, skips files last modified before the year, splits the
rest into size-balanced shards and runs one child process per shard. Each child streams its
files through a strict JSON lexer (`bend/lex.bend`) that captures only the fields the metrics
read, folds every line into order-independent records (`bend/fold.bend`), and hands the fold
back as text. The parent merges the folds, applies the time zone and prices (`bend/report.bend`,
`bend/bill.bend`), builds one view of the report (`bend/view.bend`) and renders it
(`bend/text.bend`, `bend/html.bend`, `bend/card.bend`). File listing, whole-line reads and
time zones are small C effects in `bend/effects/`; everything else is Bend.

## Development

```bash
scripts/test.sh                       # laws, build, metrics and formats against the oracle, CLI
scripts/test.sh ~/.claude/projects    # also both comparisons on a real history
python3 bend/gen.py                   # after editing pricing/prices.tsv, bend/assets/ or the line record
```

To change what a metric means, change `docs/semantics.md`, then the oracle, then the engine,
and extend `tests/make_fixtures.py` with a case that tells the old rule from the new one.

The previous Rust implementation (v0.3) is in the git history before this rewrite. It counted
every streamed copy of a response, so its totals ran about twice too high.

## License

MIT, see [LICENSE](LICENSE).
