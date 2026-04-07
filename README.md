# claude-code-wrapped

Your Claude Code year in review. Same idea as Spotify Wrapped, except for your dev sessions with Claude.

I built this because I couldn't answer basic questions about my own usage. How much have I spent? Is my caching actually doing anything? Am I reaching for Opus when Haiku would do just fine? The Claude dashboard has token counts and a cost total — that's it. I wanted something that reads the data and tells me what it means.

It reads `~/.claude/projects/**/*.jsonl` directly — nothing leaves your machine — and writes a terminal recap, an HTML report, a shareable card, and an optional per-project prompt archive.

```
$ ccwrapped 2026

Claude Code Wrapped 2026
Archetype: Precision Maximalist
"You lean on Opus for most work, which means you're either doing deeply nuanced
tasks or leaving efficiency gains on the table."

Hero Stats
  Total spend      $128.44
  Active days      71
  Longest streak   9 days
  Top project      payments-api
  Human prompts    73%

Quick Read
  Cache grade      B  (412:1 ratio)
  Model mix        62% Sonnet / 31% Opus / 7% Haiku
  Human vs tool    73% human (1,204 human / 441 tool)
  Next move        Compact earlier before long idle gaps rebuild the cache
```

| I want to... | Go to |
|---|---|
| Install and run it | [Quick start](#quick-start) |
| See all flags | [Flags](#flags) |
| Understand what it computes | [What it measures](#what-it-measures) |
| Contribute or hack on it | [Development](#development) |

## Quick start

```bash
cargo install --git https://github.com/onblueroses/claude-code-wrapped
ccwrapped
```

That's it. It finds your Claude Code history automatically at `~/.claude/projects/`.

## Flags

```bash
ccwrapped [YEAR]               # default: current year
ccwrapped --markdown           # also write claude-code-wrapped.md
ccwrapped --card               # write + open a shareable animated HTML card
ccwrapped --archive            # write per-project prompt files to ./wrapped-archive/ (contains prompt excerpts — don't share)
ccwrapped --no-open            # skip auto-opening browser
ccwrapped --json               # print raw JSON to stdout, no files written
```

The `--card` flag writes a 1080x1920 HTML file: CSS animations, no JavaScript, no project names or paths. It screenshots cleanly and shares without leaking anything about what you're working on.

## What it measures

**Cost** — total spend and per-model breakdown, using `costUSD` from your JSONL records directly (not estimated from tokens).

**Cache health** — hit rate, efficiency ratio, estimated savings, and an A–F grade. The grade factors in both your cache hit rate and how often your cache gets invalidated.

**Model routing** — distribution across Opus/Sonnet/Haiku and what it implies about your workflow. If you're running Opus on everything, it'll say so.

**Session shape** — busiest hour, favorite weekday, longest streak, burst vs. steady-tempo pattern. These feed into your archetype.

**Prompt ratio** — how many messages came from you vs. tool callbacks. A low human percentage usually means long agentic runs.

**Archetypes** — one of four patterns (Precision Maximalist, Delegation Director, Flow-State Builder, Balanced Operator) derived from your model mix and message cadence. A separate momentum card (Burst-mode Operator or Measured Tempo) captures your day-to-day pacing.

**Recommendations** — data-driven suggestions based on what the data actually shows: cache patterns, model routing, session structure.

## How it works

```
~/.claude/projects/**/*.jsonl
         │
         ▼
    readers/          parse JSONL, group by session, count prompts
         │
         ▼
    analyzers/        cost, cache grade, model routing, wrapped story
         │
         ▼
    renderers/        terminal, HTML, markdown, share card
         │
         ▼
    output files
```

The `vendor/glob` directory is a small vendored glob implementation. The original build had no crates.io access, so rather than pulling in the full crate and pretending otherwise, I left it in place. It handles recursive JSONL discovery. Swap it for the real `glob` crate if you want — one-line change in `Cargo.toml`.

## Development

```bash
git clone https://github.com/onblueroses/claude-code-wrapped
cd claude-code-wrapped
cargo build --release
cargo test
./target/release/ccwrapped --no-open
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for PR conventions.

## License

MIT — see [LICENSE](LICENSE).
