# What the numbers mean

This is the specification of every metric ccwrapped reports. `oracle/oracle.py` implements it
in plain Python and is the reference; the Bend engine in `bend/` must produce exactly the same
metrics, which `tests/compare.py` checks, and its formats for people must show them as the last
section says, which `tests/views.py` checks. When a rule changes, it changes here first.

## Sources

- Transcripts are the `*.jsonl` files under `$CLAUDE_CONFIG_DIR/projects`, or
  `~/.claude/projects` when that variable is unset, or the roots given with `--root`.
  Directories are searched recursively; symlinked directories are not followed.
- A file is read up to its length when it is opened. A session that is still writing adds
  lines after that point; they are left for the next run. Only complete lines (ending in a
  newline) are read.
- A file last modified more than two days before 1 January (UTC) of the report year is not
  read. A line is written at the moment its timestamp records (copies keep the original
  timestamp), so such a file holds nothing from the year.

## Lines

- A line must be valid UTF-8 and one JSON object (RFC 8259: no `NaN` or `Infinity`, no raw
  control characters in strings, no trailing commas). Anything else is counted as malformed
  and otherwise ignored.
- When a key repeats, the last value wins, including everything nested under it.
- A field of the wrong type counts as absent. A count must be a non-negative integer of at
  most 10^9 (no response comes near a billion tokens); anything else reads as 0.
- A line without a string `timestamp` contributes nothing.

## Responses

- An assistant line with a `message.usage` object and a string `message.id` is one copy of
  an API response. Claude Code writes a response once per streamed content block, again when a
  session is resumed or forked, and again in subagent transcripts, so copies are merged by
  message id:
  - every count (input, output, cache read, cache write, the 1-hour and 5-minute cache-write
    breakdown, web searches, web fetches) is the maximum over the copies, because output
    tokens grow as a response streams;
  - fast mode and US-only inference hold when any copy says so (`usage.speed == "fast"`,
    `usage.inference_geo == "us"`);
  - the response is attributed to the least `(timestamp, session, cwd, model, sidechain)`
    among its copies, comparing strings by code point and `false` before `true`.
- This merge is commutative, associative and idempotent (proved in `bend/LAWS.bend`), so the
  result does not depend on file order, on sharding, or on how often a response was copied.
- A response belongs to the report year when its attributed timestamp falls in that calendar
  year in the report time zone. Timestamps must look like `YYYY-MM-DDTHH:MM:SS`, optionally
  with a fraction (ignored), and end in `Z` or `+HH:MM`/`-HH:MM`; a response whose timestamp
  does not parse, or names an impossible date or time, is counted separately and left out.

## Cost

- Cost is what the traffic would cost at Anthropic's first-party API list prices
  (`pricing/prices.tsv`, from the pricing page captured beside it). It is an estimate of
  API-equivalent value, not a bill: subscriptions are billed differently.
- A model id is priced by its row, or, with a trailing `-YYYYMMDD` snapshot suffix removed, by
  the row of its base id. A fast-mode response uses the model's fast row when there is one,
  otherwise its standard row. US-only inference multiplies every price by 1.1.
- Billed categories: input, 5-minute cache writes, 1-hour cache writes, cache reads, output,
  and web searches at $10 per 1,000. Web fetches are free.
- The cache-write total is authoritative. The 1-hour share is the reported 1-hour count,
  capped at the total; the rest is billed at the 5-minute rate. Every cache-write token is
  billed exactly once (proved). Tokens outside the reported breakdown are counted.
- Only what the transcripts record can be priced. Claude Code's WebSearch and WebFetch tools
  leave no usage of their own in the transcripts (the usage records report no searches), so
  their cost is not in the estimate; the report says how many such calls there were.
- `<synthetic>` responses (made up locally by Claude Code) cost nothing. A model with no
  price row is listed as unpriced and left out of the cost.
- Amounts are exact: tokens times microdollars per million tokens is picodollars, summed as
  integers. Reports round to cents only for display.

## Everything else

- Projects are the attributed `cwd`, with the home directory shown as `~`; a response with no
  cwd is `(unknown)`. With `--share`, projects are shown only as `project-N` by cost rank.
- Sessions are distinct non-empty attributed session ids.
- Days, hours and weekdays are local to the report time zone; the time zone's own rules
  (daylight saving included) come from the system's zone database.
- Human prompts are user lines with a string `uuid` that are neither meta nor sidechain, whose
  content is a non-blank string, or a list with a non-blank text block, that does not begin
  with a marker Claude Code writes itself (`<local-command-stdout>`, `<local-command-stderr>`,
  `<local-command-caveat>`, `[Request interrupted by user`, `<task-notification>`,
  `<bash-stdout>`, `<bash-stderr>`). Blank means only ASCII whitespace. A prompt copied into
  several files counts once, at its earliest timestamp.
- Tool calls are `tool_use` content blocks with a string id in a response line (see
  Responses), counted once per id. Each is dated and named by the least
  `(timestamp, name, message id)` among its copies, and counted when that timestamp falls in
  the report year.
- Active days are days with at least one response; the streak is the longest run of
  consecutive active days.

## The formats for people

The terminal report, the Markdown report, the HTML slides and the share card show the metrics
above; `--json` prints all of them.

- Money is rounded half up to the cent. Shares are rounded half up to a tenth of a percent,
  from costs in microdollars. Token counts shown as 1.2B, 34.5M or 6.7K are rounded half up to
  one decimal.
- Models and projects are ranked by exact cost, tools by calls; equal values are ordered by
  name. The terminal and Markdown reports list every model and the top 12 projects and tools;
  the slides list the top 16 models, 8 projects and 10 tools and say how many more there are.
- The busiest day has the highest cost (the earliest of equal days). The busiest hour and
  weekday have the most responses (the earliest of equals). In the slides' calendar a day's
  shade is its cost as a share of the busiest day's, in quarters rounded up; a day with
  responses but no cost gets the lightest shade.
- Model ids are shown as they are, except in the slides and the card, where a `claude-` id is
  shown by name: `claude-opus-4-8` as "Opus 4.8", `claude-haiku-4-5-20251001` as "Haiku 4.5".
  A tool call without a name is listed as `(no name)`.
- Names come from transcripts, so they are shown as text: control characters and lone
  surrogates appear as U+FFFD, HTML is escaped, and Markdown table cells escape `|` and `<`.
- `--share` shows projects as `project-1`, `project-2`, ... by cost rank, and counts every MCP
  tool (`mcp__<server>__<tool>`, whose name carries its server's) as one entry, `MCP tools`.
- The share card shows the year, cost, responses, typed prompts, tokens, active days, the
  longest streak, the top model and the calendar. It never shows a project, a tool or the
  time zone.
- The HTML pages load nothing and run no scripts: a content security policy forbids every
  request.
