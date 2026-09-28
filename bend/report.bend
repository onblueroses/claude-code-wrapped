import Base
import ./types.bend as T
import ./fold.bend as F
import ./gen/prices.bend as PR
import ./bill.bend as B

# From the merged fold to the metrics of one calendar year in one time zone.
# docs/semantics.md states each rule; oracle/oracle.py is the executable reference.

# Calendar
# --------

# Days from a fixed era to a proleptic Gregorian date, so every value is a Nat: the Unix epoch
# (1970-01-01) is day 719468.

def Days.of.fin(era: Nat, +yoe: Nat, mp: Nat, d: Nat) -> Nat:
  (era * 146097n + yoe * 365n + Nat.div(yoe, 4n) - Nat.div(yoe, 100n) + Nat.div((153n * mp + 2n : Nat), 5n) + d - 1n : Nat)

def Days.of.era(+y: Nat, mp: Nat, d: Nat) -> Nat:
  Days.of.fin(Nat.div(y, 400n), (y - (Nat.div(y, 400n) * 400n : Nat) : Nat), mp, d)

def Days.of.go(early: Bool, y: Nat, m: Nat, d: Nat) -> Nat:
  match early:
    case True{}:
      Days.of.era((y - 1n : Nat), (m + 9n : Nat), d)
    case False{}:
      Days.of.era(y, (m - 3n : Nat), d)

def Days.of(y: Nat, +m: Nat, d: Nat) -> Nat:
  Days.of.go(Nat.is_le(m, 2n), y, m, d)

def Civil.of.fin(early: Bool, y: Nat, mp: Nat, d: Nat) -> Nat & Nat & Nat:
  match early:
    case True{}:
      (y, (mp + 3n : Nat), d)
    case False{}:
      ((y + 1n : Nat), (mp - 9n : Nat), d)

def Civil.of.mp(y: Nat, doy: Nat, +mp: Nat) -> Nat & Nat & Nat:
  Civil.of.fin(Nat.is_lt(mp, 10n), y, mp, (doy - Nat.div((153n * mp + 2n : Nat), 5n) + 1n : Nat))

def Civil.of.doy(y: Nat, +doy: Nat) -> Nat & Nat & Nat:
  Civil.of.mp(y, doy, Nat.div((5n * doy + 2n : Nat), 153n))

def Civil.of.yoe(era: Nat, doe: Nat, +yoe: Nat) -> Nat & Nat & Nat:
  Civil.of.doy((yoe + era * 400n : Nat), (doe - (365n * yoe + Nat.div(yoe, 4n) - Nat.div(yoe, 100n) : Nat) : Nat))

def Civil.of.era(era: Nat, +doe: Nat) -> Nat & Nat & Nat:
  Civil.of.yoe(era, doe, Nat.div((doe - Nat.div(doe, 1460n) + Nat.div(doe, 36524n) - Nat.div(doe, 146096n) : Nat), 365n))

# The civil date (year, month, day) of a day number from Days.of.
def Civil.of(+z: Nat) -> Nat & Nat & Nat:
  Civil.of.era(Nat.div(z, 146097n), (z - (Nat.div(z, 146097n) * 146097n : Nat) : Nat))

def Leap(+y: Nat) -> Bool:
  (Nat.is_eq(Nat.mod(y, 4n), 0n) && Nat.is_ne(Nat.mod(y, 100n), 0n)) || Nat.is_eq(Nat.mod(y, 400n), 0n)

def Month.days.go(feb: Bool, leap: Bool, short: Bool) -> Nat:
  match feb leap short:
    case True{} True{} _:
      29n
    case True{} False{} _:
      28n
    case False{} _ True{}:
      30n
    case False{} _ False{}:
      31n

def Month.days(y: Nat, +m: Nat) -> Nat:
  Month.days.go(Nat.is_eq(m, 2n), Leap(y), Nat.is_eq(m, 4n) || Nat.is_eq(m, 6n) || Nat.is_eq(m, 9n) || Nat.is_eq(m, 11n))

# Seconds of the Unix epoch in the shifted scale.
def Epoch0() -> Nat:
  (719468n * 86400n : Nat)

# Timestamps
# ----------

# An instant in shifted seconds (Epoch0 is 1970-01-01T00:00:00Z).
type Instant is Data:
  NoInstant{}
  At{secs: Nat}

@unsafe def Digits.step(ok: Bool, p: Nat, c: U32, t: String, acc: Nat) -> Maybe<&1, Nat & String>:
  match ok:
    case True{}:
      Digits.go(t, p, (acc * 10n + U32.to_nat((c - 48 : U32)) : Nat))
    case False{}:
      None{}

# Exactly n ASCII digits, and the rest of the string.
@unsafe def Digits.go(s: String, n: Nat, acc: Nat) -> Maybe<&1, Nat & String>:
  match s n:
    case rest 0n:
      Some{(acc, rest)}
    case SCon{Chr{+c}, t} 1n+p:
      Digits.step(U32.is_ge(c, 48) && U32.is_le(c, 57), p, c, t, acc)
    case SNil{} 1n+p:
      None{}

def Lit.go.if(ok: Bool, t: String) -> Maybe<&2, String>:
  match ok:
    case True{}:
      Some{t}
    case False{}:
      None{}

def Lit.go(s: String, c: U32) -> Maybe<&2, String>:
  match s:
    case SCon{Chr{h}, t}:
      Lit.go.if(U32.is_eq(h, c), t)
    case SNil{}:
      None{}

def Ts.build.if(ok: Bool, local: Nat, neg: Bool, off: Nat) -> Instant:
  match ok neg:
    case True{} False{}:
      At{(local - off : Nat)}
    case True{} True{}:
      At{(local + off : Nat)}
    case False{} _:
      NoInstant{}

# Python's datetime accepts the fields only in range, and an offset under 24 hours.
def Ts.build(+y: Nat, +mo: Nat, +d: Nat, +h: Nat, +mi: Nat, +sec: Nat, neg: Bool, +off: Nat) -> Instant:
  Ts.build.if(Nat.is_ge(y, 1n) && Nat.is_ge(mo, 1n) && Nat.is_le(mo, 12n) && Nat.is_ge(d, 1n) && Nat.is_le(d, Month.days(y, mo)) && Nat.is_lt(h, 24n) && Nat.is_lt(mi, 60n) && Nat.is_lt(sec, 60n) && Nat.is_lt(off, 86400n), (Days.of(y, mo, d) * 86400n + h * 3600n + mi * 60n + sec : Nat), neg, off)

def Ts.off3(neg: Bool, y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, oh: Nat, r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{om, SNil{}}}:
      Ts.build(y, mo, d, h, mi, sec, neg, (oh * 3600n + om * 60n : Nat))
    case _:
      NoInstant{}

@unsafe def Ts.off2(neg: Bool, y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, oh: Nat, r: Maybe<&2, String>) -> Instant:
  match r:
    case Some{t}:
      Ts.off3(neg, y, mo, d, h, mi, sec, oh, Digits.go(t, 2n, 0n))
    case None{}:
      NoInstant{}

@unsafe def Ts.off(neg: Bool, y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{oh, t}}:
      Ts.off2(neg, y, mo, d, h, mi, sec, oh, Lit.go(t, 58))
    case None{}:
      NoInstant{}

@unsafe def Ts.zone.kind(z: Bool, plus: Bool, minus: Bool, y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, +rest: String) -> Instant:
  match z plus minus rest:
    case True{} _ _ SNil{}:
      Ts.build(y, mo, d, h, mi, sec, False{}, 0n)
    case False{} True{} _ rest:
      Ts.off(False{}, y, mo, d, h, mi, sec, Digits.go(rest, 2n, 0n))
    case False{} False{} True{} rest:
      Ts.off(True{}, y, mo, d, h, mi, sec, Digits.go(rest, 2n, 0n))
    case _ _ _ _:
      NoInstant{}

@unsafe def Ts.zone(y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, t: String) -> Instant:
  match t:
    case SCon{Chr{+c}, rest}:
      Ts.zone.kind(U32.is_eq(c, 90), U32.is_eq(c, 43), U32.is_eq(c, 45), y, mo, d, h, mi, sec, rest)
    case SNil{}:
      NoInstant{}

def Ts.skip_digits.end(seen: Bool, t: String) -> Maybe<&2, String>:
  match seen:
    case True{}:
      Some{t}
    case False{}:
      None{}

@unsafe def Ts.skip_digits.if(digit: Bool, seen: Bool, c: U32, t: String) -> Maybe<&2, String>:
  match digit:
    case True{}:
      Ts.skip_digits(t, True{})
    case False{}:
      Ts.skip_digits.end(seen, SCon{Chr{c}, t})

# Skips a fraction's digits; at least one is required.
@unsafe def Ts.skip_digits(s: String, seen: Bool) -> Maybe<&2, String>:
  match s:
    case SCon{Chr{+c}, t}:
      Ts.skip_digits.if(U32.is_ge(c, 48) && U32.is_le(c, 57), seen, c, t)
    case SNil{}:
      Ts.skip_digits.end(seen, SNil{})

@unsafe def Ts.frac.digits(y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, r: Maybe<&2, String>) -> Instant:
  match r:
    case Some{t}:
      Ts.zone(y, mo, d, h, mi, sec, t)
    case None{}:
      NoInstant{}

@unsafe def Ts.frac.if(dot: Bool, y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, +t: String) -> Instant:
  match dot t:
    case True{} SCon{c, rest}:
      Ts.frac.digits(y, mo, d, h, mi, sec, Ts.skip_digits(rest, False{}))
    case False{} t:
      Ts.zone(y, mo, d, h, mi, sec, t)
    case True{} SNil{}:
      NoInstant{}

@unsafe def Ts.frac(y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, sec: Nat, t: String) -> Instant:
  match t:
    case SCon{Chr{+c}, rest}:
      Ts.frac.if(U32.is_eq(c, 46), y, mo, d, h, mi, sec, SCon{Chr{c}, rest})
    case SNil{}:
      NoInstant{}

@unsafe def Ts.s(y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{sec, t}}:
      Ts.frac(y, mo, d, h, mi, sec, t)
    case None{}:
      NoInstant{}

@unsafe def Ts.mi2(y: Nat, mo: Nat, d: Nat, h: Nat, mi: Nat, r: Maybe<&2, String>) -> Instant:
  match r:
    case Some{t}:
      Ts.s(y, mo, d, h, mi, Digits.go(t, 2n, 0n))
    case None{}:
      NoInstant{}

@unsafe def Ts.mi(y: Nat, mo: Nat, d: Nat, h: Nat, r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{mi, t}}:
      Ts.mi2(y, mo, d, h, mi, Lit.go(t, 58))
    case None{}:
      NoInstant{}

@unsafe def Ts.h2(y: Nat, mo: Nat, d: Nat, h: Nat, r: Maybe<&2, String>) -> Instant:
  match r:
    case Some{t}:
      Ts.mi(y, mo, d, h, Digits.go(t, 2n, 0n))
    case None{}:
      NoInstant{}

@unsafe def Ts.h(y: Nat, mo: Nat, d: Nat, r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{h, t}}:
      Ts.h2(y, mo, d, h, Lit.go(t, 58))
    case None{}:
      NoInstant{}

@unsafe def Ts.d2(y: Nat, mo: Nat, d: Nat, r: Maybe<&2, String>) -> Instant:
  match r:
    case Some{t}:
      Ts.h(y, mo, d, Digits.go(t, 2n, 0n))
    case None{}:
      NoInstant{}

@unsafe def Ts.d(y: Nat, mo: Nat, r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{d, t}}:
      Ts.d2(y, mo, d, Lit.go(t, 84))
    case None{}:
      NoInstant{}

@unsafe def Ts.mo2(y: Nat, mo: Nat, r: Maybe<&2, String>) -> Instant:
  match r:
    case Some{t}:
      Ts.d(y, mo, Digits.go(t, 2n, 0n))
    case None{}:
      NoInstant{}

@unsafe def Ts.mo(y: Nat, r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{mo, t}}:
      Ts.mo2(y, mo, Lit.go(t, 45))
    case None{}:
      NoInstant{}

@unsafe def Ts.y2(y: Nat, r: Maybe<&2, String>) -> Instant:
  match r:
    case Some{t}:
      Ts.mo(y, Digits.go(t, 2n, 0n))
    case None{}:
      NoInstant{}

@unsafe def Ts.y(r: Maybe<&1, Nat & String>) -> Instant:
  match r:
    case Some{Tuple{y, t}}:
      Ts.y2(y, Lit.go(t, 45))
    case None{}:
      NoInstant{}

# Parses YYYY-MM-DDTHH:MM:SS, an optional .fraction (ignored) and Z or +HH:MM/-HH:MM.
@unsafe def Ts.parse(s: String) -> Instant:
  Ts.y(Digits.go(s, 4n, 0n))

# Zones
# -----

# A UTC offset in force from an instant on: seconds east or west of UTC.
type Shift is Data:
  Shift{at: Nat, off: Nat, west: Bool}

def Zone.parse.off.go(at: Nat, west: Bool, n: Maybe<&2, Nat>) -> Maybe<&2, Shift>:
  match n:
    case Some{off}:
      Some{Shift{at, off, west}}
    case None{}:
      None{}

def Zone.parse.off(west: Bool, at: Nat, +o: String) -> Maybe<&2, Shift>:
  match west o:
    case True{} SCon{c, rest}:
      Zone.parse.off.go(at, True{}, Nat.read(rest))
    case False{} o:
      Zone.parse.off.go(at, False{}, Nat.read(o))
    case True{} SNil{}:
      None{}

def Zone.parse.row.go(t: Maybe<&2, Nat>, o: String) -> Maybe<&2, Shift>:
  match t o:
    case Some{at} SCon{Chr{+c}, rest}:
      Zone.parse.off(U32.is_eq(c, 45), (at + Epoch0() : Nat), SCon{Chr{c}, rest})
    case _ _:
      None{}

def Zone.parse.row(fields: List<&2, String>) -> Maybe<&2, Shift>:
  match fields:
    case [t, o]:
      Zone.parse.row.go(Nat.read(t), o)
    case _:
      None{}

def Zone.parse.keep(m: Maybe<&2, Shift>, rest: List<&2, Shift>) -> List<&2, Shift>:
  match m:
    case Some{s}:
      Con{s, rest}
    case None{}:
      rest

def Zone.parse.rows(rows: List<&2, String>) -> List<&2, Shift>:
  match rows:
    case Nil{}:
      Nil{}
    case Con{row, rest}:
      Zone.parse.keep(Zone.parse.row(String.split(row, '\t')), Zone.parse.rows(rest))

# The Zone.offsets effect's rows: "epoch\toffset" at the window start, then each transition.
def Zone.parse(text: String) -> List<&2, Shift>:
  Zone.parse.rows(String.lines(text))

def Zone.apply(t: Nat, off: Nat, west: Bool) -> Nat:
  match west:
    case True{}:
      (t - off : Nat)
    case False{}:
      (t + off : Nat)

@unsafe def Zone.local.go.if(hit: Bool, rest: List<&2, Shift>, t: Nat, local: Nat, best: Maybe<&2, Nat>) -> Maybe<&2, Nat>:
  match hit:
    case True{}:
      Zone.local.go(rest, t, Some{local})
    case False{}:
      best

@unsafe def Zone.local.go(shifts: List<&2, Shift>, +t: Nat, best: Maybe<&2, Nat>) -> Maybe<&2, Nat>:
  match shifts:
    case Nil{}:
      best
    case Con{Shift{at, off, west}, rest}:
      Zone.local.go.if(Nat.is_le(at, t), rest, t, Zone.apply(t, off, west), best)

# Local time at an instant: the last transition at or before it applies. Instants before the
# window (which spans the report year with days to spare) have no local time here.
@unsafe def Zone.local(shifts: List<&2, Shift>, t: Nat) -> Maybe<&2, Nat>:
  Zone.local.go(shifts, t, None{})

# A local moment, broken into what the report uses.
type Local is Data:
  Local{year: Nat, date: String, day: Nat, hour: Nat, weekday: Nat}

def Show2.if(pad: Bool, s: String) -> String:
  match pad:
    case True{}:
      "0" ++ s
    case False{}:
      s

def Show2(+n: Nat) -> String:
  Show2.if(Nat.is_lt(n, 10n), Nat.show(n))

def Show4(+n: Nat) -> String:
  B.Pad6.go((4n - String.length(Nat.show(n)) : Nat), Nat.show(n))

def Local.of.fin(+y: Nat, m: Nat, d: Nat, +day: Nat, hour: Nat) -> Local:
  Local{y, Show4(y) ++ "-" ++ Show2(m) ++ "-" ++ Show2(d), day, hour, Nat.mod((day + 2n : Nat), 7n)}

def Local.of.civil(day: Nat, hour: Nat, c: Nat & Nat & Nat) -> Local:
  match c:
    case Tuple{y, Tuple{m, d}}:
      Local.of.fin(y, m, d, day, hour)

def Local.of.go(+day: Nat, rem: Nat) -> Local:
  Local.of.civil(day, Nat.div(rem, 3600n), Civil.of(day))

# Weekday 0 is Monday: day 719468 (1970-01-01) was a Thursday, and 719468 + 2 = 3 (mod 7).
def Local.of(+secs: Nat) -> Local:
  Local.of.go(Nat.div(secs, 86400n), Nat.mod(secs, 86400n))

# Pricing
# -------

def Model.digit(c: Char) -> Bool:
  match c:
    case Chr{+u}:
      U32.is_ge(u, 48) && U32.is_le(u, 57)

# True when a reversed model id ends in -YYYYMMDD.
def Model.dated(r: String) -> Bool:
  match r:
    case SCon{a, SCon{b, SCon{c, SCon{d, SCon{e, SCon{f, SCon{g, SCon{h, SCon{Chr{dash}, rest}}}}}}}}}:
      U32.is_eq(dash, 45) && Model.digit(a) && Model.digit(b) && Model.digit(c) && Model.digit(d) && Model.digit(e) && Model.digit(f) && Model.digit(g) && Model.digit(h)
    case _:
      False{}

def Model.base.go(dated: Bool, +m: String) -> String:
  match dated:
    case True{}:
      String.take(m, (String.length(m) - 9n : Nat))
    case False{}:
      m

# A model id without a -YYYYMMDD snapshot suffix.
def Model.base(+m: String) -> String:
  Model.base.go(Model.dated(String.reverse(m)), m)

def Bool.same(a: Bool, b: Bool) -> Bool:
  match a b:
    case True{} True{}:
      True{}
    case False{} False{}:
      True{}
    case _ _:
      False{}

def Prices.flag(fast: Bool) -> String:
  match fast:
    case True{}:
      "1"
    case False{}:
      "0"

# The price table as a map from "model|fast" (fast is 0 or 1) to its row, built once.
def Prices.key(model: String, fast: Bool) -> String:
  model ++ "|" ++ Prices.flag(fast)

def Prices.map.go(rows: List<&2, T.Price>, m: Map<&2, B.Row>) -> Map<&2, B.Row>:
  match rows:
    case Nil{}:
      m
    case Con{T.Price{model, fast, inp, w5, w1, read, out}, rest}:
      Prices.map.go(rest, Map.set(&2, B.Row, m, Prices.key(model, fast), B.Row{inp, w5, w1, read, out}))

def Prices.map() -> Map<&2, B.Row>:
  Prices.map.go(PR.Prices.table(), MTip{})

def Price.lookup.get(got: Map<&2, B.Row> & B.Row) -> B.Row:
  match got:
    case Tuple{m, r}:
      r

def Price.lookup.got(got: Map<&2, B.Row> & Bool, key: String) -> Maybe<&2, B.Row>:
  match got:
    case Tuple{m, True{}}:
      Some{Price.lookup.get(Map.get(B.Row, B.Row{0n, 0n, 0n, 0n, 0n}, m, key))}
    case Tuple{m, False{}}:
      None{}

def Price.lookup(prices: Map<&2, B.Row>, +key: String) -> Maybe<&2, B.Row>:
  Price.lookup.got(Map.has(&2, B.Row, prices, key), key)

def Price.row.fallback(m: Maybe<&2, B.Row>, prices: Map<&2, B.Row>, base: String) -> Maybe<&2, B.Row>:
  match m:
    case Some{r}:
      Some{r}
    case None{}:
      Price.lookup(prices, Prices.key(base, False{}))

def Price.row.base(+base: String, fast: Bool, +prices: Map<&2, B.Row>) -> Maybe<&2, B.Row>:
  Price.row.fallback(Price.lookup(prices, Prices.key(base, fast)), prices, base)

# The row for a model at a speed, falling back to its standard row.
def Price.row(model: String, fast: Bool, prices: Map<&2, B.Row>) -> Maybe<&2, B.Row>:
  Price.row.base(Model.base(model), fast, prices)

type Priced is Data:
  Synthetic{}
  Unpriced{}
  Priced{row: B.Row}

def Price.pick.row(m: Maybe<&2, B.Row>, geo: Bool) -> Priced:
  match m:
    case Some{r}:
      Priced{B.Row.geo(geo, r)}
    case None{}:
      Unpriced{}

def Price.pick.go(synthetic: Bool, model: String, fast: Bool, geo: Bool, prices: Map<&2, B.Row>) -> Priced:
  match synthetic:
    case True{}:
      Synthetic{}
    case False{}:
      Price.pick.row(Price.row(model, fast, prices), geo)

# "<synthetic>" marks responses Claude Code makes up locally; they cost nothing.
def Price.pick(+model: String, fast: Bool, geo: Bool, prices: Map<&2, B.Row>) -> Priced:
  Price.pick.go(String.eq(model, "<synthetic>"), model, fast, geo, prices)

def Priced.cost(p: Priced, t: B.Toks, ws: Nat) -> B.Money:
  match p:
    case Priced{r}:
      B.Cost.of(t, r, ws)
    case _:
      B.Money.zero()

def Priced.unpriced(p: Priced) -> Bool:
  match p:
    case Unpriced{}:
      True{}
    case _:
      False{}

# Aggregates
# ----------

type ModelAgg is Data:
  ModelAgg{requests: Nat, cost: B.Money, toks: B.Toks}

type ProjAgg is Data:
  ProjAgg{requests: Nat, cost: B.Money, tokens: Nat, sessions: Map<&2, Unit>}

type DayAgg is Data:
  DayAgg{requests: Nat, cost: B.Money, output: Nat}

type Counts is Data:
  Counts{bad_ts: Nat, unsplit: Nat, fast: Nat, side: Nat, wf: Nat, ws: Nat}

type Agg is Data:
  Agg{requests: Nat, toks: B.Toks, cost: B.Money, models: Map<&2, ModelAgg>, unpriced: Map<&2, Nat>, projects: Map<&2, ProjAgg>, days: Map<&2, DayAgg>, hours: Map<&2, Nat>, weekdays: Map<&2, Nat>, sessions: Map<&2, Unit>, counts: Counts}

def Agg.empty() -> Agg:
  Agg{0n, B.Toks.zero(), B.Money.zero(), MTip{}, MTip{}, MTip{}, MTip{}, MTip{}, MTip{}, MTip{}, Counts{0n, 0n, 0n, 0n, 0n, 0n}}

def Bump.nat.fin(k: String, n: Nat, got: Map<&2, Nat> & Nat) -> Map<&2, Nat>:
  match got:
    case Tuple{m, old}:
      Map.set(&2, Nat, m, k, (old + n : Nat))

def Bump.nat(m: Map<&2, Nat>, +k: String, n: Nat) -> Map<&2, Nat>:
  Bump.nat.fin(k, n, Map.get(Nat, 0n, m, k))

def Bump.model.fin(k: String, cost: B.Money, t: B.Toks, got: Map<&2, ModelAgg> & ModelAgg) -> Map<&2, ModelAgg>:
  match got:
    case Tuple{m, ModelAgg{r, c, tk}}:
      Map.set(&2, ModelAgg, m, k, ModelAgg{(r + 1n : Nat), B.Money.add(c, cost), B.Toks.add(tk, t)})

def Bump.model(m: Map<&2, ModelAgg>, +k: String, cost: B.Money, t: B.Toks) -> Map<&2, ModelAgg>:
  Bump.model.fin(k, cost, t, Map.get(ModelAgg, ModelAgg{0n, B.Money.zero(), B.Toks.zero()}, m, k))

def Set.add.if(empty: Bool, k: String, s: Map<&2, Unit>) -> Map<&2, Unit>:
  match empty:
    case True{}:
      s
    case False{}:
      Map.set(&2, Unit, s, k, Unit{})

def Bump.project.fin(k: String, cost: B.Money, tokens: Nat, +session: String, got: Map<&2, ProjAgg> & ProjAgg) -> Map<&2, ProjAgg>:
  match got:
    case Tuple{m, ProjAgg{r, c, t, s}}:
      Map.set(&2, ProjAgg, m, k, ProjAgg{(r + 1n : Nat), B.Money.add(c, cost), (t + tokens : Nat), Set.add.if(String.is_empty(session), session, s)})

def Bump.project(m: Map<&2, ProjAgg>, +k: String, cost: B.Money, tokens: Nat, session: String) -> Map<&2, ProjAgg>:
  Bump.project.fin(k, cost, tokens, session, Map.get(ProjAgg, ProjAgg{0n, B.Money.zero(), 0n, MTip{}}, m, k))

def Bump.day.fin(k: String, cost: B.Money, out: Nat, got: Map<&2, DayAgg> & DayAgg) -> Map<&2, DayAgg>:
  match got:
    case Tuple{m, DayAgg{r, c, o}}:
      Map.set(&2, DayAgg, m, k, DayAgg{(r + 1n : Nat), B.Money.add(c, cost), (o + out : Nat)})

def Bump.day(m: Map<&2, DayAgg>, +k: String, cost: B.Money, out: Nat) -> Map<&2, DayAgg>:
  Bump.day.fin(k, cost, out, Map.get(DayAgg, DayAgg{0n, B.Money.zero(), 0n}, m, k))

def Project.name.under.if(ok: Bool, rest: String, cwd: String) -> String:
  match ok:
    case True{}:
      "~" ++ rest
    case False{}:
      cwd

def Project.name.under(+rest: String, cwd: String) -> String:
  Project.name.under.if(String.is_empty(rest) || String.starts_with(rest, "/"), rest, cwd)

def Project.name.go(empty: Bool, outside: Bool, home: String, +cwd: String) -> String:
  match empty outside:
    case True{} _:
      "(unknown)"
    case False{} True{}:
      cwd
    case False{} False{}:
      Project.name.under(String.drop(cwd, String.length(home)), cwd)

# The project a response belongs to: its cwd, with the home directory shown as "~".
def Project.name(+home: String, +cwd: String) -> String:
  Project.name.go(String.is_empty(cwd), String.is_empty(home) || Bool.not(String.starts_with(cwd, home)), home, cwd)

def Abs.diff.if(ge: Bool, a: Nat, b: Nat) -> Nat:
  match ge:
    case True{}:
      (a - b : Nat)
    case False{}:
      (b - a : Nat)

def Abs.diff(+a: Nat, +b: Nat) -> Nat:
  Abs.diff.if(Nat.is_ge(a, b), a, b)

def Nat.of_bool(b: Bool) -> Nat:
  match b:
    case True{}:
      1n
    case False{}:
      0n

def Counts.add(c: Counts, bad_ts: Nat, unsplit: Nat, fast: Nat, side: Nat, wf: Nat, ws: Nat) -> Counts:
  match c:
    case Counts{a, b, f, s, x, y}:
      Counts{(a + bad_ts : Nat), (b + unsplit : Nat), (f + fast : Nat), (s + side : Nat), (x + wf : Nat), (y + ws : Nat)}

def Unpriced.bump(hit: Bool, model: String, m: Map<&2, Nat>) -> Map<&2, Nat>:
  match hit:
    case True{}:
      Bump.nat(m, model, 1n)
    case False{}:
      m

def Agg.request.go(unpriced: Bool, +cost: B.Money, date: String, hour: Nat, weekday: Nat, home: String, mid: String, t: B.Toks, unsplit: Nat, ws: Nat, wf: Nat, fast: Bool, +session: String, cwd: String, +model: String, side: Bool, a: Agg) -> Agg:
  match t a:
    case B.Toks{+i, +w5, +w1, +rd, +o} Agg{requests, toks, acost, models, unp, projects, days, hours, weekdays, sessions, counts}:
      Agg{(requests + 1n : Nat), B.Toks.add(toks, B.Toks{i, w5, w1, rd, o}), B.Money.add(acost, cost), Bump.model(models, model, cost, B.Toks{i, w5, w1, rd, o}), Unpriced.bump(unpriced, model, unp), Bump.project(projects, Project.name(home, cwd), cost, (i + w5 + w1 + rd + o : Nat), session), Bump.day(days, date, cost, o), Bump.nat(hours, Nat.show(hour), 1n), Bump.nat(weekdays, Nat.show(weekday), 1n), Set.add.if(String.is_empty(session), session, sessions), Counts.add(counts, 0n, unsplit, Nat.of_bool(fast), Nat.of_bool(side), wf, ws)}

def Agg.request(+p: Priced, date: String, hour: Nat, weekday: Nat, home: String, mid: String, +t: B.Toks, unsplit: Nat, +ws: Nat, wf: Nat, fast: Bool, session: String, cwd: String, model: String, side: Bool, a: Agg) -> Agg:
  Agg.request.go(Priced.unpriced(p), Priced.cost(p, t, ws), date, hour, weekday, home, mid, t, unsplit, ws, wf, fast, session, cwd, model, side, a)

@unsafe def Agg.group.in(hit: Bool, date: String, hour: Nat, weekday: Nat, home: String, mid: String, t: B.Toks, unsplit: Nat, ws: Nat, wf: Nat, +fast: Bool, geo: Bool, session: String, cwd: String, +model: String, side: Bool, a: Agg, prices: Map<&2, B.Row>) -> Agg:
  match hit:
    case True{}:
      Agg.request(Price.pick(model, fast, geo, prices), date, hour, weekday, home, mid, t, unsplit, ws, wf, fast, session, cwd, model, side, a)
    case False{}:
      a

@unsafe def Agg.group.year(loc: Local, year: Nat, home: String, mid: String, t: B.Toks, unsplit: Nat, ws: Nat, wf: Nat, fast: Bool, geo: Bool, session: String, cwd: String, model: String, side: Bool, a: Agg, prices: Map<&2, B.Row>) -> Agg:
  match loc:
    case Local{y, date, day, hour, weekday}:
      Agg.group.in(Nat.is_eq(y, year), date, hour, weekday, home, mid, t, unsplit, ws, wf, fast, geo, session, cwd, model, side, a, prices)

@unsafe def Agg.group.local(l: Maybe<&2, Nat>, year: Nat, home: String, mid: String, t: B.Toks, unsplit: Nat, ws: Nat, wf: Nat, fast: Bool, geo: Bool, session: String, cwd: String, model: String, side: Bool, a: Agg, prices: Map<&2, B.Row>) -> Agg:
  match l:
    case Some{secs}:
      Agg.group.year(Local.of(secs), year, home, mid, t, unsplit, ws, wf, fast, geo, session, cwd, model, side, a, prices)
    case None{}:
      a

@unsafe def Agg.group.at(i: Instant, year: Nat, home: String, shifts: List<&2, Shift>, mid: String, t: B.Toks, unsplit: Nat, ws: Nat, wf: Nat, fast: Bool, geo: Bool, session: String, cwd: String, model: String, side: Bool, +a: Agg, prices: Map<&2, B.Row>) -> Agg:
  match i a:
    case NoInstant{} Agg{requests, toks, cost, models, unpriced, projects, days, hours, weekdays, sessions, counts}:
      Agg{requests, toks, cost, models, unpriced, projects, days, hours, weekdays, sessions, Counts.add(counts, 1n, 0n, 0n, 0n, 0n, 0n)}
    case At{secs} a:
      Agg.group.local(Zone.local(shifts, secs), year, home, mid, t, unsplit, ws, wf, fast, geo, session, cwd, model, side, a, prices)

type Ctx is Data:
  Ctx{year: Nat, home: String, shifts: List<&2, Shift>, prices: Map<&2, B.Row>}

# One response: counted as a bad timestamp when its timestamp does not parse, skipped when it
# falls outside the year, otherwise priced and added to every aggregate.
@unsafe def Agg.group(ctx: Ctx, mid: String, g: F.Group, a: Agg) -> Agg:
  match ctx g:
    case Ctx{year, home, shifts, prices} F.Group{inp, out, cread, +cwrite, +c1h, c5m, ws, wf, fast, geo, [ts, session, cwd, model, side]}:
      Agg.group.at(Ts.parse(ts), year, home, shifts, mid, B.Toks.of(inp, out, cread, cwrite, c1h), Abs.diff(cwrite, (c5m + c1h : Nat)), ws, wf, fast, geo, session, cwd, model, String.eq(side, "1"), a, prices)
    case ctx g:
      a

@unsafe def Agg.groups(xs: List<&2, Sigma<&2, &2, String, _ => F.Group>>, +ctx: Ctx, a: Agg) -> Agg:
  match xs:
    case Nil{}:
      a
    case Con{Tuple{k, g}, rest}:
      Agg.groups(rest, ctx, Agg.group(ctx, k, g, a))

# Tool calls and prompts
# ----------------------

def Tools.count.hit(hit: Bool, name: String, acc: Map<&2, Nat>) -> Map<&2, Nat>:
  match hit:
    case True{}:
      Bump.nat(acc, name, 1n)
    case False{}:
      acc

# Tool calls dated in the period, by tool name. A tool record is [timestamp, name, message id].
@unsafe def Tools.count(xs: List<&2, Sigma<&2, &2, String, _ => List<&2, String>>>, +year: Nat, +shifts: List<&2, Shift>, acc: Map<&2, Nat>) -> Map<&2, Nat>:
  match xs:
    case Nil{}:
      acc
    case Con{Tuple{k, [ts, name, mid]}, rest}:
      Tools.count(rest, year, shifts, Tools.count.hit(Nat.is_eq(Prompts.in(Ts.parse(ts), year, shifts), 1n), name, acc))
    case Con{Tuple{k, other}, rest}:
      Tools.count(rest, year, shifts, acc)

def Prompts.in.year(loc: Local, year: Nat) -> Nat:
  match loc:
    case Local{y, date, day, hour, weekday}:
      Nat.of_bool(Nat.is_eq(y, year))

def Prompts.in.local(l: Maybe<&2, Nat>, year: Nat) -> Nat:
  match l:
    case Some{secs}:
      Prompts.in.year(Local.of(secs), year)
    case None{}:
      0n

@unsafe def Prompts.in(i: Instant, year: Nat, shifts: List<&2, Shift>) -> Nat:
  match i:
    case At{secs}:
      Prompts.in.local(Zone.local(shifts, secs), year)
    case NoInstant{}:
      0n

@unsafe def Prompts.count(xs: List<&2, Sigma<&2, &2, String, _ => String>>, +year: Nat, +shifts: List<&2, Shift>, n: Nat) -> Nat:
  match xs:
    case Nil{}:
      n
    case Con{Tuple{k, ts}, rest}:
      Prompts.count(rest, year, shifts, (n + Prompts.in(Ts.parse(ts), year, shifts) : Nat))

# Parallel aggregation
# --------------------

# Aggregates of disjoint sets of responses combine by adding counts and money and uniting
# sets, so the responses can be split, aggregated in parallel and merged in any grouping.

def Merge.nats(xs: List<&2, Sigma<&2, &2, String, _ => Nat>>, m: Map<&2, Nat>) -> Map<&2, Nat>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{k, v}, rest}:
      Merge.nats(rest, Bump.nat(m, k, v))

def Merge.units(xs: List<&2, Sigma<&2, &2, String, _ => Unit>>, m: Map<&2, Unit>) -> Map<&2, Unit>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{k, v}, rest}:
      Merge.units(rest, Map.set(&2, Unit, m, k, Unit{}))

def Merge.model.fin(k: String, got: Map<&2, ModelAgg> & ModelAgg, x: ModelAgg) -> Map<&2, ModelAgg>:
  match got x:
    case Tuple{m, ModelAgg{r, c, t}} ModelAgg{xr, xc, xt}:
      Map.set(&2, ModelAgg, m, k, ModelAgg{(r + xr : Nat), B.Money.add(c, xc), B.Toks.add(t, xt)})

def Merge.models(xs: List<&2, Sigma<&2, &2, String, _ => ModelAgg>>, m: Map<&2, ModelAgg>) -> Map<&2, ModelAgg>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{+k, x}, rest}:
      Merge.models(rest, Merge.model.fin(k, Map.get(ModelAgg, ModelAgg{0n, B.Money.zero(), B.Toks.zero()}, m, k), x))

def Merge.project.fin(k: String, got: Map<&2, ProjAgg> & ProjAgg, x: ProjAgg) -> Map<&2, ProjAgg>:
  match got x:
    case Tuple{m, ProjAgg{r, c, t, s}} ProjAgg{xr, xc, xt, xs}:
      Map.set(&2, ProjAgg, m, k, ProjAgg{(r + xr : Nat), B.Money.add(c, xc), (t + xt : Nat), Merge.units(Map.to_list(&2, Unit, xs), s)})

def Merge.projects(xs: List<&2, Sigma<&2, &2, String, _ => ProjAgg>>, m: Map<&2, ProjAgg>) -> Map<&2, ProjAgg>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{+k, x}, rest}:
      Merge.projects(rest, Merge.project.fin(k, Map.get(ProjAgg, ProjAgg{0n, B.Money.zero(), 0n, MTip{}}, m, k), x))

def Merge.day.fin(k: String, got: Map<&2, DayAgg> & DayAgg, x: DayAgg) -> Map<&2, DayAgg>:
  match got x:
    case Tuple{m, DayAgg{r, c, o}} DayAgg{xr, xc, xo}:
      Map.set(&2, DayAgg, m, k, DayAgg{(r + xr : Nat), B.Money.add(c, xc), (o + xo : Nat)})

def Merge.days(xs: List<&2, Sigma<&2, &2, String, _ => DayAgg>>, m: Map<&2, DayAgg>) -> Map<&2, DayAgg>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{+k, x}, rest}:
      Merge.days(rest, Merge.day.fin(k, Map.get(DayAgg, DayAgg{0n, B.Money.zero(), 0n}, m, k), x))

def Counts.merge(a: Counts, b: Counts) -> Counts:
  match b:
    case Counts{x1, x2, x3, x4, x5, x6}:
      Counts.add(a, x1, x2, x3, x4, x5, x6)

def Agg.merge(a: Agg, b: Agg) -> Agg:
  match a b:
    case Agg{r1, t1, c1, m1, u1, p1, d1, h1, w1, s1, n1} Agg{r2, t2, c2, m2, u2, p2, d2, h2, w2, s2, n2}:
      Agg{(r1 + r2 : Nat), B.Toks.add(t1, t2), B.Money.add(c1, c2), Merge.models(Map.to_list(&2, ModelAgg, m2), m1), Merge.nats(Map.to_list(&2, Nat, u2), u1), Merge.projects(Map.to_list(&2, ProjAgg, p2), p1), Merge.days(Map.to_list(&2, DayAgg, d2), d1), Merge.nats(Map.to_list(&2, Nat, h2), h1), Merge.nats(Map.to_list(&2, Nat, w2), w1), Merge.units(Map.to_list(&2, Unit, s2), s1), Counts.merge(n1, n2)}

def Split.cons(h: Sigma<&2, &2, String, _ => F.Group>, r: List<&2, Sigma<&2, &2, String, _ => F.Group>> & List<&2, Sigma<&2, &2, String, _ => F.Group>>) -> List<&2, Sigma<&2, &2, String, _ => F.Group>> & List<&2, Sigma<&2, &2, String, _ => F.Group>>:
  match r:
    case Tuple{a, b}:
      (Con{h, a}, b)

def Split.go(n: Nat, +xs: List<&2, Sigma<&2, &2, String, _ => F.Group>>) -> List<&2, Sigma<&2, &2, String, _ => F.Group>> & List<&2, Sigma<&2, &2, String, _ => F.Group>>:
  match n xs:
    case 0n xs:
      (Nil{}, xs)
    case 1n+p Nil{}:
      (Nil{}, Nil{})
    case 1n+p Con{h, t}:
      Split.cons(h, Split.go(p, t))

# Aggregates responses in a balanced tree of parallel calls, depth levels deep.
@unsafe def Agg.tree(depth: Nat, +n: Nat, xs: List<&2, Sigma<&2, &2, String, _ => F.Group>>, ctx: Ctx) -> Agg:
  match depth:
    case 0n:
      Agg.groups(xs, ctx, Agg.empty())
    case 1n+d:
      Agg.tree.halves(d, Nat.div(n, 2n), (n - Nat.div(n, 2n) : Nat), Split.go(Nat.div(n, 2n), xs), ctx)

@unsafe def Agg.tree.halves(d: Nat, h: Nat, rest: Nat, parts: List<&2, Sigma<&2, &2, String, _ => F.Group>> & List<&2, Sigma<&2, &2, String, _ => F.Group>>, +ctx: Ctx) -> Agg:
  match parts:
    case Tuple{left, right}:
      Agg.tree.join(d, h, rest, left, right, Ctx.dup(ctx))

@unsafe def Agg.tree.join(d: Nat, h: Nat, rest: Nat, left: List<&2, Sigma<&2, &2, String, _ => F.Group>>, right: List<&2, Sigma<&2, &2, String, _ => F.Group>>, cs: Ctx & Ctx) -> Agg:
  match cs:
    case Tuple{c1, c2}:
      Agg.tree.fork(d, h, rest, left, right, c1, c2)

@unsafe def Agg.tree.fork(+d: Nat, h: Nat, rest: Nat, left: List<&2, Sigma<&2, &2, String, _ => F.Group>>, right: List<&2, Sigma<&2, &2, String, _ => F.Group>>, c1: Ctx, c2: Ctx) -> Agg:
  a b = Agg.tree(d, h, left, c1) Agg.tree(d, rest, right, c2)
  Agg.merge(a, b)

# All responses of a fold, aggregated in 64 parallel parts.
@unsafe def Agg.all(xs: List<&2, Sigma<&2, &2, String, _ => F.Group>>, +year: Nat, +home: String, +shifts: List<&2, Shift>) -> Agg:
  Agg.all.go(Len.go(xs, 0n, Nil{}), Ctx{year, home, shifts, Prices.map()})

# The length of a list and the list rebuilt, so the caller holds it unshared.
def Len.go(xs: List<&2, Sigma<&2, &2, String, _ => F.Group>>, n: Nat, acc: List<&2, Sigma<&2, &2, String, _ => F.Group>>) -> Nat & List<&2, Sigma<&2, &2, String, _ => F.Group>>:
  match xs:
    case Nil{}:
      (n, acc)
    case Con{h, t}:
      Len.go(t, (n + 1n : Nat), Con{h, acc})

@unsafe def Agg.all.go(got: Nat & List<&2, Sigma<&2, &2, String, _ => F.Group>>, ctx: Ctx) -> Agg:
  match got:
    case Tuple{n, xs}:
      Agg.tree(6n, n, xs, ctx)

# Independent copies of the context for parallel lanes: every lane reading one shared value
# pays an atomic reference count per read.
def Str.copy(s: String) -> String:
  String.reverse(String.reverse(s))

def Shifts.copy(xs: List<&2, Shift>) -> List<&2, Shift>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Shift{a, o, w}, rest}:
      Con{Shift{a, o, w}, Shifts.copy(rest)}

def Ctx.dup(c: Ctx) -> Ctx & Ctx:
  match c:
    case Ctx{+year, +home, +shifts, prices}:
      (Ctx{year, Str.copy(home), Shifts.copy(shifts), Prices.map()}, Ctx{year, Str.copy(home), Shifts.copy(shifts), Prices.map()})
