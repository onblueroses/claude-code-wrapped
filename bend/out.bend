import Base
import ./fold.bend as F
import ./report.bend as R
import ./bill.bend as B
import ./render.bend as V

# The metrics as JSON, in the oracle's schema (ccwrapped.metrics/v1).

# A JSON member: a key and its already-encoded value.

type KV is Data:
  KV{k: String, v: String}

def Json.hex.pick(digit: Bool, n: U32) -> String:
  match digit:
    case True{}:
      SCon{Chr{(n + 48 : U32)}, SNil{}}
    case False{}:
      SCon{Chr{(n + 87 : U32)}, SNil{}}

def Json.hex(+n: U32) -> String:
  Json.hex.pick(U32.is_lt(n, 10), n)

# A code point as four hex digits.
def Json.hex4(+c: U32) -> String:
  Json.hex(U32.mod(U32.div(c, 4096), 16)) ++ Json.hex(U32.mod(U32.div(c, 256), 16)) ++ Json.hex(U32.mod(U32.div(c, 16), 16)) ++ Json.hex(U32.mod(c, 16))

# Quotes, backslashes and control characters are escaped, and so is a lone UTF-16 surrogate
# (a JSON \u escape may decode to one), which has no UTF-8 encoding.
def Json.char.go(quote: Bool, bs: Bool, esc: Bool, c: U32) -> String:
  match quote bs esc:
    case True{} _ _:
      "\\\""
    case False{} True{} _:
      "\\\\"
    case False{} False{} True{}:
      "\\u" ++ Json.hex4(c)
    case False{} False{} False{}:
      SCon{Chr{c}, SNil{}}

def Json.char(+c: U32) -> String:
  Json.char.go(U32.is_eq(c, 34), U32.is_eq(c, 92), U32.is_lt(c, 32) || (U32.is_ge(c, 55296) && U32.is_le(c, 57343)), c)

def Json.str.go(s: String, acc: List<&2, String>) -> List<&2, String>:
  match s:
    case SNil{}:
      acc
    case SCon{Chr{c}, t}:
      Json.str.go(t, Con{Json.char(c), acc})

def Json.str(s: String) -> String:
  "\"" ++ String.concat(List.reverse(&2, String, Json.str.go(s, Nil{}))) ++ "\""

def Json.members(kvs: List<&2, KV>) -> List<&2, String>:
  match kvs:
    case Nil{}:
      Nil{}
    case Con{KV{k, v}, rest}:
      Con{Json.str(k) ++ ":" ++ v, Json.members(rest)}

def Json.obj(kvs: List<&2, KV>) -> String:
  "{" ++ String.join(Json.members(kvs), ",") ++ "}"

def Json.arr(xs: List<&2, String>) -> String:
  "[" ++ String.join(xs, ",") ++ "]"

def Json.nat(n: Nat) -> String:
  Nat.show(n)

# Map entries, sorted by key
# --------------------------

def KV.le(a: KV, b: KV) -> Bool:
  match a b:
    case KV{ka, va} KV{kb, vb}:
      String.is_le(ka, kb)

def Sorted(xs: List<&2, KV>) -> List<&2, KV>:
  List.sort(~KV, ~KV.le, xs)

def Ent.nats(xs: List<&2, Sigma<&2, &2, String, _ => Nat>>) -> List<&2, KV>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, v}, rest}:
      Con{KV{k, Json.nat(v)}, Ent.nats(rest)}

def Json.toks(t: B.Toks) -> List<&2, KV>:
  match t:
    case B.Toks{i, a, b, r, o}:
      [KV{"input", Json.nat(i)}, KV{"cache_write_5m", Json.nat(a)}, KV{"cache_write_1h", Json.nat(b)}, KV{"cache_read", Json.nat(r)}, KV{"output", Json.nat(o)}]

def Ent.models(xs: List<&2, Sigma<&2, &2, String, _ => R.ModelAgg>>) -> List<&2, KV>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, R.ModelAgg{r, c, t}}, rest}:
      Con{KV{k, Json.obj(Con{KV{"requests", Json.nat(r)}, Con{KV{"cost", B.Money.show(c)}, Json.toks(t)}})}, Ent.models(rest)}

def Ent.projects(xs: List<&2, Sigma<&2, &2, String, _ => R.ProjAgg>>) -> List<&2, KV>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, R.ProjAgg{r, c, t, s}}, rest}:
      Con{KV{k, Json.obj([KV{"requests", Json.nat(r)}, KV{"cost", B.Money.show(c)}, KV{"tokens", Json.nat(t)}, KV{"sessions", Json.nat(Map.size(&2, Unit, s))}])}, Ent.projects(rest)}

def Ent.days(xs: List<&2, Sigma<&2, &2, String, _ => R.DayAgg>>) -> List<&2, KV>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, R.DayAgg{r, c, o}}, rest}:
      Con{KV{k, Json.obj([KV{"requests", Json.nat(r)}, KV{"cost", B.Money.show(c)}, KV{"output", Json.nat(o)}])}, Ent.days(rest)}

# Hours and weekdays
# ------------------

def Slots.get(got: Map<&2, Nat> & Nat) -> Nat:
  match got:
    case Tuple{m, v}:
      v

# Counts for slots 0..n-1 of a map keyed by the slot number.
def Slots.go(n: Nat, +i: Nat, +m: Map<&2, Nat>) -> List<&2, String>:
  match n:
    case 0n:
      Nil{}
    case 1n+p:
      Con{Json.nat(Slots.get(Map.get(Nat, 0n, m, Nat.show(i)))), Slots.go(p, (i + 1n : Nat), m)}

# Streaks
# -------

def Day.num.d(y: Nat, m: Nat, r: Maybe<&1, Nat & String>) -> Nat:
  match r:
    case Some{Tuple{d, t}}:
      R.Days.of(y, m, d)
    case None{}:
      0n

@unsafe def Day.num.m(y: Nat, r: Maybe<&1, Nat & String>) -> Nat:
  match r:
    case Some{Tuple{m, t}}:
      Day.num.d(y, m, R.Digits.go(String.drop(t, 1n), 2n, 0n))
    case None{}:
      0n

@unsafe def Day.num.go(r: Maybe<&1, Nat & String>) -> Nat:
  match r:
    case Some{Tuple{y, t}}:
      Day.num.m(y, R.Digits.go(String.drop(t, 1n), 2n, 0n))
    case None{}:
      0n

# The day number of a YYYY-MM-DD date.
@unsafe def Day.num(date: String) -> Nat:
  Day.num.go(R.Digits.go(date, 4n, 0n))

@unsafe def Streak.step(next: Bool, d: Nat, rest: List<&2, Nat>, +run: Nat, best: Nat) -> Nat:
  match next:
    case True{}:
      Streak.go(rest, d, (run + 1n : Nat), Nat.max(best, (run + 1n : Nat)))
    case False{}:
      Streak.go(rest, d, 1n, Nat.max(best, 1n))

# The longest run of consecutive days, over day numbers in ascending order.
@unsafe def Streak.go(days: List<&2, Nat>, prev: Nat, run: Nat, best: Nat) -> Nat:
  match days:
    case Nil{}:
      best
    case Con{+d, rest}:
      Streak.step(Nat.is_eq(d, (prev + 1n : Nat)), d, rest, run, best)

@unsafe def Streak.nums(xs: List<&2, KV>) -> List<&2, Nat>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{KV{k, v}, rest}:
      Con{Day.num(k), Streak.nums(rest)}

@unsafe def Streak(days: List<&2, KV>) -> Nat:
  Streak.go(Streak.nums(days), 0n, 0n, 0n)

# The document
# ------------

def Json.counts(c: R.Counts) -> String:
  match c:
    case R.Counts{bad, unsplit, fast, side, wf, ws}:
      Json.obj([KV{"bad_timestamp_requests", Json.nat(bad)}, KV{"cache_write_unsplit_tokens", Json.nat(unsplit)}, KV{"fast_requests", Json.nat(fast)}, KV{"sidechain_requests", Json.nat(side)}, KV{"web_fetch_requests", Json.nat(wf)}, KV{"web_search_requests", Json.nat(ws)}])

def Json.scan(lines: Nat, bad: Nat, usage: Nat, files: Nat) -> String:
  Json.obj([KV{"files", Json.nat(files)}, KV{"lines", Json.nat(lines)}, KV{"malformed_lines", Json.nat(bad)}, KV{"usage_lines", Json.nat(usage)}])

@unsafe def Json.report.days(+days: List<&2, KV>, year: Nat, zone: String, requests: Nat, toks: B.Toks, cost: B.Money, models: Map<&2, R.ModelAgg>, unpriced: Map<&2, Nat>, projects: Map<&2, R.ProjAgg>, hours: Map<&2, Nat>, weekdays: Map<&2, Nat>, sessions: Map<&2, Unit>, counts: R.Counts, tool_calls: Map<&2, Nat>, human: Nat, lines: Nat, bad: Nat, usage: Nat, files: Nat) -> String:
  Json.obj([
    KV{"schema", Json.str("ccwrapped.metrics/v1")},
    KV{"period", Json.obj([KV{"year", Json.nat(year)}, KV{"timezone", Json.str(zone)}])},
    KV{"requests", Json.nat(requests)},
    KV{"tokens", Json.obj(Json.toks(toks))},
    KV{"cost_picodollars", B.Money.show(cost)},
    KV{"models", Json.obj(Sorted(Ent.models(Map.to_list(&2, R.ModelAgg, models))))},
    KV{"unpriced_models", Json.obj(Sorted(Ent.nats(Map.to_list(&2, Nat, unpriced))))},
    KV{"projects", Json.obj(Sorted(Ent.projects(Map.to_list(&2, R.ProjAgg, projects))))},
    KV{"days", Json.obj(days)},
    KV{"active_days", Json.nat(List.length(&2, KV, days))},
    KV{"longest_streak_days", Json.nat(Streak(days))},
    KV{"hours", Json.arr(Slots.go(24n, 0n, hours))},
    KV{"weekdays_monday_first", Json.arr(Slots.go(7n, 0n, weekdays))},
    KV{"sessions", Json.nat(Map.size(&2, Unit, sessions))},
    KV{"human_prompts", Json.nat(human)},
    KV{"tool_calls", Json.obj(Sorted(Ent.nats(Map.to_list(&2, Nat, tool_calls))))},
    KV{"counts", Json.counts(counts)},
    KV{"scan", Json.scan(lines, bad, usage, files)}
  ])

@unsafe def Json.report(year: Nat, zone: String, a: R.Agg, tool_calls: Map<&2, Nat>, human: Nat, lines: Nat, bad: Nat, usage: Nat, files: Nat) -> String:
  match a:
    case R.Agg{requests, toks, cost, models, unpriced, projects, days, hours, weekdays, sessions, counts}:
      Json.report.days(Sorted(Ent.days(Map.to_list(&2, R.DayAgg, days))), year, zone, requests, toks, cost, models, unpriced, projects, hours, weekdays, sessions, counts, tool_calls, human, lines, bad, usage, files)

@unsafe def Metrics.json.agg(a: R.Agg, +year: Nat, zone: String, +shifts: List<&2, R.Shift>, tools: Map<&2, List<&2, String>>, prompts: Map<&2, String>, lines: Nat, bad: Nat, usage: Nat, files: Nat) -> String:
  match a:
    case R.Agg{requests, toks, cost, models, unpriced, projects, days, hours, weekdays, sessions, counts}:
      Json.report(year, zone, R.Agg{requests, toks, cost, models, unpriced, projects, days, hours, weekdays, sessions, counts}, R.Tools.count(Map.to_list(&2, List<&2, String>, tools), year, shifts, MTip{}), R.Prompts.count(Map.to_list(&2, String, prompts), year, shifts, 0n), lines, bad, usage, files)

# Everything from a merged fold.
@unsafe def Metrics.json(+year: Nat, zone: String, home: String, +shifts: List<&2, R.Shift>, f: F.Fold) -> String:
  match f:
    case F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      Metrics.json.agg(R.Agg.all(Map.to_list(&2, F.Group, groups), year, home, shifts), year, zone, shifts, tools, prompts, lines, bad, usage, files)

# The report for people, from a merged fold.
@unsafe def Metrics.text(fmt: Nat, share: Bool, +year: Nat, zone: String, home: String, +shifts: List<&2, R.Shift>, f: F.Fold) -> String:
  match f:
    case F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      Metrics.text.agg(fmt, share, R.Agg.all(Map.to_list(&2, F.Group, groups), year, home, shifts), year, zone, shifts, tools, prompts, lines, bad, files)

@unsafe def Metrics.text.agg(fmt: Nat, share: Bool, a: R.Agg, +year: Nat, zone: String, +shifts: List<&2, R.Shift>, tools: Map<&2, List<&2, String>>, prompts: Map<&2, String>, lines: Nat, bad: Nat, files: Nat) -> String:
  match a:
    case R.Agg{requests, toks, cost, models, unpriced, projects, +days, hours, weekdays, sessions, counts}:
      V.Render.report(fmt, share, year, zone, R.Agg{requests, toks, cost, models, unpriced, projects, days, hours, weekdays, sessions, counts}, R.Tools.count(Map.to_list(&2, List<&2, String>, tools), year, shifts, MTip{}), R.Prompts.count(Map.to_list(&2, String, prompts), year, shifts, 0n), Streak(Sorted(Ent.days(Map.to_list(&2, R.DayAgg, days)))), lines, bad, files)
