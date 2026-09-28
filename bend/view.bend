import Base
import ./bill.bend as B
import ./report.bend as R

# What every human format shows: number formatting, safe text, and the View, the
# report's contents computed once from the aggregate.

# Numbers
# -------

@unsafe def Fmt.group.go(s: String, +n: Nat, acc: String) -> String:
  match s:
    case SNil{}:
      acc
    case SCon{c, t}:
      Fmt.group.put(Nat.is_eq(Nat.mod(n, 3n), 0n) && Nat.is_gt(n, 0n), c, t, n, acc)

@unsafe def Fmt.group.put(comma: Bool, c: Char, t: String, n: Nat, acc: String) -> String:
  match comma:
    case True{}:
      Fmt.group.go(t, (n + 1n : Nat), SCon{c, SCon{Chr{44}, acc}})
    case False{}:
      Fmt.group.go(t, (n + 1n : Nat), SCon{c, acc})

# 1234567 as "1,234,567".
def Fmt.int(n: Nat) -> String:
  Fmt.group.go(String.reverse(Nat.show(n)), 0n, "")

def Fmt.two(n: Nat) -> String:
  R.Show2(n)

def Fmt.usd.cents(+c: Nat) -> String:
  "$" ++ Fmt.int(Nat.div(c, 100n)) ++ "." ++ Fmt.two(Nat.mod(c, 100n))

# Dollars rounded half up to the cent.
def Fmt.usd(m: B.Money) -> String:
  match m:
    case B.Money{micro, pico}:
      Fmt.usd.cents(Nat.div((micro + 5000n : Nat), 10000n))

def Money.micro(m: B.Money) -> Nat:
  match m:
    case B.Money{micro, pico}:
      micro

def Fmt.pct.go(none: Bool, +tenths: Nat) -> String:
  match none:
    case True{}:
      "-"
    case False{}:
      Nat.show(Nat.div(tenths, 10n)) ++ "." ++ Nat.show(Nat.mod(tenths, 10n)) ++ "%"

# part/whole as a percentage with one decimal.
def Fmt.pct(part: Nat, +whole: Nat) -> String:
  Fmt.pct.go(Nat.is_eq(whole, 0n), Nat.div((part * 1000n + Nat.div(whole, 2n) : Nat), Nat.max(whole, 1n)))

def Fmt.scaled.go(+tenths: Nat, suffix: String) -> String:
  Nat.show(Nat.div(tenths, 10n)) ++ "." ++ Nat.show(Nat.mod(tenths, 10n)) ++ suffix

def Fmt.scaled(n: Nat, +unit: Nat, suffix: String) -> String:
  Fmt.scaled.go(Nat.div((n * 10n + Nat.div(unit, 2n) : Nat), unit), suffix)

def Fmt.big.go(b: Bool, m: Bool, k: Bool, n: Nat) -> String:
  match b m k:
    case True{} _ _:
      Fmt.scaled(n, 1000000000n, "B")
    case False{} True{} _:
      Fmt.scaled(n, 1000000n, "M")
    case False{} False{} True{}:
      Fmt.scaled(n, 1000n, "K")
    case False{} False{} False{}:
      Nat.show(n)

# Compact token counts: 1.2B, 34.5M, 6.7K.
def Fmt.big(+n: Nat) -> String:
  Fmt.big.go(Nat.is_ge(n, 1000000000n), Nat.is_ge(n, 1000000n), Nat.is_ge(n, 1000n), n)

def Plural.go(single: Bool, one: String, many: String) -> String:
  match single:
    case True{}:
      one
    case False{}:
      many

# Layout
# ------

def Plural(n: Nat, one: String, many: String) -> String:
  Plural.go(Nat.is_eq(n, 1n), one, many)

def Pad.go(n: Nat, s: String) -> String:
  match n:
    case 0n:
      s
    case 1n+p:
      Pad.go(p, s ++ " ")

def Pad.r(+s: String, w: Nat) -> String:
  Pad.go((w - String.length(s) : Nat), s)

def Pad.l.go(n: Nat, s: String) -> String:
  match n:
    case 0n:
      s
    case 1n+p:
      Pad.l.go(p, " " ++ s)

def Pad.l(+s: String, w: Nat) -> String:
  Pad.l.go((w - String.length(s) : Nat), s)

def Clip.go(long: Bool, +s: String, w: Nat) -> String:
  match long:
    case True{}:
      "..." ++ String.drop(s, (String.length(s) - (w - 3n : Nat) : Nat))
    case False{}:
      s

def Clip(+s: String, +w: Nat) -> String:
  Clip.go(Nat.is_gt(String.length(s), w), s, w)

# Ranked entries
# --------------

type Ent is Data:
  Ent{name: String, cost: Nat, count: Nat, money: B.Money, extra: Nat}

def Money.gt(a: B.Money, b: B.Money) -> Bool:
  match a b:
    case B.Money{+am, ap} B.Money{+bm, bp}:
      Nat.is_gt(am, bm) || (Nat.is_eq(am, bm) && Nat.is_gt(ap, bp))

def Money.eq(a: B.Money, b: B.Money) -> Bool:
  match a b:
    case B.Money{am, ap} B.Money{bm, bp}:
      Nat.is_eq(am, bm) && Nat.is_eq(ap, bp)

# Rankings are total orders, so equal values always list the same way: by cost or count,
# then by name.
def Ent.by_cost(a: Ent, b: Ent) -> Bool:
  match a b:
    case Ent{an, ac, ak, +am, ax} Ent{bn, bc, bk, +bm, bx}:
      Money.gt(am, bm) || (Money.eq(am, bm) && String.is_le(an, bn))

def Ent.by_count(a: Ent, b: Ent) -> Bool:
  match a b:
    case Ent{an, ac, +ak, am, ax} Ent{bn, bc, +bk, bm, bx}:
      Nat.is_gt(ak, bk) || (Nat.is_eq(ak, bk) && String.is_le(an, bn))

def Ent.name_le(a: Ent, b: Ent) -> Bool:
  match a b:
    case Ent{an, ac, ak, am, ax} Ent{bn, bc, bk, bm, bx}:
      String.is_le(an, bn)

def Ents.models(xs: List<&2, Sigma<&2, &2, String, _ => R.ModelAgg>>) -> List<&2, Ent>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, R.ModelAgg{r, +c, t}}, rest}:
      Con{Ent{k, Money.micro(c), r, c, B.Toks.sum(t)}, Ents.models(rest)}

def Ents.projects(xs: List<&2, Sigma<&2, &2, String, _ => R.ProjAgg>>) -> List<&2, Ent>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, R.ProjAgg{r, +c, t, s}}, rest}:
      Con{Ent{k, Money.micro(c), r, c, Map.size(&2, Unit, s)}, Ents.projects(rest)}

def Ents.nats(xs: List<&2, Sigma<&2, &2, String, _ => Nat>>) -> List<&2, Ent>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, v}, rest}:
      Con{Ent{k, 0n, v, B.Money.zero(), 0n}, Ents.nats(rest)}

def Ents.days(xs: List<&2, Sigma<&2, &2, String, _ => R.DayAgg>>) -> List<&2, Ent>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, R.DayAgg{r, +c, o}}, rest}:
      Con{Ent{k, Money.micro(c), r, c, o}, Ents.days(rest)}

def Cost.cell(unpriced: Bool, m: B.Money) -> String:
  match unpriced:
    case True{}:
      "unpriced"
    case False{}:
      Fmt.usd(m)

def Names.has(xs: List<&2, String>, +name: String) -> Bool:
  match xs:
    case Nil{}:
      False{}
    case Con{x, rest}:
      String.eq(x, name) || Names.has(rest, name)

def Ents.names(xs: List<&2, Ent>) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Ent{n, c, k, m, x}, rest}:
      Con{n, Ents.names(rest)}

def Sum.counts(xs: List<&2, Ent>) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{Ent{n, c, k, m, x}, rest}:
      (k + Sum.counts(rest) : Nat)

def Max.cost(xs: List<&2, Ent>) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{Ent{n, c, k, m, x}, rest}:
      Nat.max(c, Max.cost(rest))

# One block character per active day, scaled to the day with the highest cost.
def Spark.char(level: Nat) -> String:
  String.take(String.drop("▁▂▃▄▅▆▇█", level), 1n)

def Spark.go(xs: List<&2, Ent>, +top: Nat) -> String:
  match xs:
    case Nil{}:
      ""
    case Con{Ent{n, c, k, m, x}, rest}:
      Spark.char(Nat.min(7n, Nat.div((c * 7n + Nat.div(top, 2n) : Nat), Nat.max(top, 1n)))) ++ Spark.go(rest, top)

@unsafe def Peak.go(xs: List<&2, Ent>, +best: Ent) -> Ent:
  match xs best:
    case Nil{} best:
      best
    case Con{Ent{n, c, k, +m, x}, rest} Ent{bn, bc, bk, +bm, bx}:
      Peak.pick(Money.gt(m, bm), Ent{n, c, k, m, x}, Ent{bn, bc, bk, bm, bx}, rest)

@unsafe def Peak.pick(later: Bool, a: Ent, b: Ent, rest: List<&2, Ent>) -> Ent:
  match later:
    case True{}:
      Peak.go(rest, a)
    case False{}:
      Peak.go(rest, b)

def Choose(c: Bool, a: Nat, b: Nat) -> Nat:
  match c:
    case True{}:
      a
    case False{}:
      b

def Peak.index.go(xs: List<&2, Nat>, +i: Nat, best_i: Nat, +best: Nat) -> Nat:
  match xs:
    case Nil{}:
      best_i
    case Con{+n, rest}:
      Peak.index.go(rest, (i + 1n : Nat), Choose(Nat.is_gt(n, best), i, best_i), Nat.max(n, best))

# The index of the first largest count.
def Peak.index(xs: List<&2, Nat>) -> Nat:
  Peak.index.go(xs, 0n, 0n, 0n)

def Max.nat(xs: List<&2, Nat>) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{x, rest}:
      Nat.max(x, Max.nat(rest))

def Sum.nat(xs: List<&2, Nat>) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{x, rest}:
      (x + Sum.nat(rest) : Nat)

def Weekday.name(n: Nat) -> String:
  match n:
    case 0n:
      "Monday"
    case 1n:
      "Tuesday"
    case 2n:
      "Wednesday"
    case 3n:
      "Thursday"
    case 4n:
      "Friday"
    case 5n:
      "Saturday"
    case _:
      "Sunday"

def Unpriced.name.go(empty: Bool, n: String) -> String:
  match empty:
    case True{}:
      "(no model)"
    case False{}:
      n

def Unpriced.name(+n: String) -> String:
  Unpriced.name.go(String.is_empty(n), n)

def Tool.label.go(empty: Bool, n: String) -> String:
  match empty:
    case True{}:
      "(no name)"
    case False{}:
      n

# A tool_use block without a name still counts, under this label.
def Tool.label(+n: String) -> String:
  Tool.label.go(String.is_empty(n), n)

# Text from transcripts
# ---------------------

# Names come from transcripts, so they may hold anything JSON can: control characters (a
# terminal would act on an escape sequence) and lone UTF-16 surrogates (no UTF-8 encoding).
# Every human format shows those as U+FFFD.
def Clean.bad(+c: U32) -> Bool:
  U32.is_lt(c, 32) || U32.is_eq(c, 127) || (U32.is_ge(c, 128) && U32.is_lt(c, 160)) || (U32.is_ge(c, 55296) && U32.is_le(c, 57343))

def Clean.char(bad: Bool, c: U32) -> U32:
  match bad:
    case True{}:
      65533
    case False{}:
      c

def Clean.go(s: String, acc: String) -> String:
  match s:
    case SNil{}:
      String.reverse(acc)
    case SCon{Chr{+c}, t}:
      Clean.go(t, SCon{Chr{Clean.char(Clean.bad(c), c)}, acc})

def Clean(s: String) -> String:
  Clean.go(s, "")

# A name in Markdown: '|' would end a table cell, '<' and '&' could start HTML or an entity,
# and the other characters here are emphasis, code, links or strikethrough.
def Md.char(+c: U32) -> String:
  match c:
    case 38:
      "&amp;"
    case 60:
      "&lt;"
    case 62:
      "&gt;"
    case 92:
      "\\\\"
    case 96:
      "\\`"
    case 42:
      "\\*"
    case 95:
      "\\_"
    case 91:
      "\\["
    case 93:
      "\\]"
    case 126:
      "\\~"
    case 124:
      "\\|"
    case _:
      SCon{Chr{c}, SNil{}}

def Md.esc.go(s: String, acc: List<&2, String>) -> List<&2, String>:
  match s:
    case SNil{}:
      acc
    case SCon{Chr{c}, t}:
      Md.esc.go(t, Con{Md.char(c), acc})

def Md.esc(s: String) -> String:
  String.concat(List.reverse(&2, String, Md.esc.go(s, Nil{})))

def Md.name.go(md: Bool, s: String) -> String:
  match md:
    case True{}:
      Md.esc(s)
    case False{}:
      s

def Md.name(md: Bool, s: String) -> String:
  Md.name.go(md, Clean(s))

def Html.char(+c: U32) -> String:
  match c:
    case 38:
      "&amp;"
    case 60:
      "&lt;"
    case 62:
      "&gt;"
    case 34:
      "&quot;"
    case 39:
      "&#39;"
    case _:
      SCon{Chr{c}, SNil{}}

def Html.esc.go(s: String, acc: List<&2, String>) -> List<&2, String>:
  match s:
    case SNil{}:
      acc
    case SCon{Chr{c}, t}:
      Html.esc.go(t, Con{Html.char(c), acc})

# Text for an HTML element or a quoted attribute.
def Html.esc(s: String) -> String:
  String.concat(List.reverse(&2, String, Html.esc.go(Clean(s), Nil{})))

# Model names
# -----------

def Word.cap(s: String) -> String:
  match s:
    case SNil{}:
      SNil{}
    case SCon{c, t}:
      SCon{Char.to_upper(c), t}

def Word.num.go(s: String) -> Bool:
  match s:
    case SNil{}:
      True{}
    case SCon{c, t}:
      Char.is_digit(c) && Word.num.go(t)

def Word.num(+s: String) -> Bool:
  Bool.not(String.is_empty(s)) && Word.num.go(s)

def Pretty.join.pick(both: Bool, a: String, b: String) -> String:
  match both:
    case True{}:
      a ++ "." ++ b
    case False{}:
      a ++ " " ++ b

# Adjacent version numbers join with a dot: ["opus", "4", "8"] is "Opus 4.8".
def Pretty.join.first(first: Bool, both: Bool, acc: String, w: String) -> String:
  match first:
    case True{}:
      w
    case False{}:
      Pretty.join.pick(both, acc, w)

def Pretty.words(ws: List<&2, String>, prev_num: Bool, +acc: String) -> String:
  match ws:
    case Nil{}:
      acc
    case Con{+w, rest}:
      Pretty.words(rest, Word.num(w), Pretty.join.first(String.is_empty(acc), prev_num && Word.num(w), acc, Word.cap(w)))

def Snapshot(+w: String) -> Bool:
  Word.num(w) && Nat.is_eq(String.length(w), 8n)

def Pretty.keep(drop: Bool, w: String, rest: List<&2, String>) -> List<&2, String>:
  match drop:
    case True{}:
      rest
    case False{}:
      Con{w, rest}

def Pretty.drop_snapshot(ws: List<&2, String>) -> List<&2, String>:
  match ws:
    case Nil{}:
      Nil{}
    case Con{+w, +rest}:
      Pretty.keep(Snapshot(w) && List.is_empty(&2, String, rest), w, Pretty.drop_snapshot(rest))

def Pretty.claude(ws: List<&2, String>) -> String:
  match ws:
    case Con{c, rest}:
      Pretty.words(Pretty.drop_snapshot(rest), False{}, "")
    case Nil{}:
      ""

def Pretty.go(claude: Bool, +id: String) -> String:
  match claude:
    case True{}:
      Pretty.claude(String.split(id, '-'))
    case False{}:
      Unpriced.name(id)

# A Claude model id for people: claude-opus-4-8 is "Opus 4.8", claude-haiku-4-5-20251001 is
# "Haiku 4.5", claude-3-5-sonnet-20241022 is "3.5 Sonnet". Other ids stay as they are.
def Model.pretty(+id: String) -> String:
  Pretty.go(String.starts_with(id, "claude-") && Nat.is_gt(String.length(id), 7n), id)

# The view
# --------

type View is Data:
  View{year: Nat, zone: String, share: Bool, cost: B.Money, requests: Nat, human: Nat, calls: Nat, sessions: Nat, streak: Nat, toks: B.Toks, models: List<&2, Ent>, unpriced: List<&2, Ent>, projects: List<&2, Ent>, tools: List<&2, Ent>, days: List<&2, Ent>, hours: List<&2, Nat>, weekdays: List<&2, Nat>, counts: R.Counts, lines: Nat, bad: Nat, files: Nat, search: Nat, fetch: Nat}

def View.year(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      year

def View.zone(v: View) -> String:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      zone

def View.share(v: View) -> Bool:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      share

def View.cost(v: View) -> B.Money:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      cost

def View.requests(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      requests

def View.human(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      human

def View.calls(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      calls

def View.sessions(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      sessions

def View.streak(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      streak

def View.toks(v: View) -> B.Toks:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      toks

def View.models(v: View) -> List<&2, Ent>:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      models

def View.unpriced(v: View) -> List<&2, Ent>:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      unpriced

def View.projects(v: View) -> List<&2, Ent>:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      projects

def View.tools(v: View) -> List<&2, Ent>:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      tools

def View.days(v: View) -> List<&2, Ent>:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      days

def View.hours(v: View) -> List<&2, Nat>:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      hours

def View.weekdays(v: View) -> List<&2, Nat>:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      weekdays

def View.counts(v: View) -> R.Counts:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      counts

def View.lines(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      lines

def View.bad(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      bad

def View.files(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      files

def View.search(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      search

def View.fetch(v: View) -> Nat:
  match v:
    case View{year, zone, share, cost, requests, human, calls, sessions, streak, toks, models, unpriced, projects, tools, days, hours, weekdays, counts, lines, bad, files, search, fetch}:
      fetch

# What the view says
# ------------------

# The day with the highest cost; the earliest of equals.
@unsafe def Peak.first(xs: List<&2, Ent>) -> Ent:
  match xs:
    case Con{e, rest}:
      Peak.go(rest, e)
    case Nil{}:
      Ent{"", 0n, 0n, B.Money.zero(), 0n}

@unsafe def View.peak_day(+v: View) -> Ent:
  Peak.first(View.days(v))

def View.active(+v: View) -> Nat:
  List.length(&2, Ent, View.days(v))

def Toks.total(t: B.Toks) -> Nat:
  B.Toks.sum(t)

def Tool.calls.pick(hit: Bool, k: Nat, rest: Nat) -> Nat:
  match hit:
    case True{}:
      k
    case False{}:
      rest

def Tool.calls(xs: List<&2, Ent>, +name: String) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{Ent{n, c, k, m, x}, rest}:
      Tool.calls.pick(String.eq(n, name), k, Tool.calls(rest, name))

def Web.note.go(none: Bool, search: Nat, fetch: Nat) -> String:
  match none:
    case True{}:
      ""
    case False{}:
      "Claude Code's WebSearch (" ++ Fmt.int(search) ++ ") and WebFetch (" ++ Fmt.int(fetch) ++ ") tool calls leave no usage in the transcripts, so their own cost is not in the estimate."

# Claude Code's own web tools leave no usage in the transcripts, so their cost cannot be known.
def Web.note(+v: View) -> String:
  Web.note.go(Nat.is_eq((View.search(v) + View.fetch(v) : Nat), 0n), View.search(v), View.fetch(v))

def Unpriced.names(xs: List<&2, Ent>) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Ent{n, c, k, m, x}, rest}:
      Con{Clean(Unpriced.name(n)) ++ " (" ++ Fmt.int(k) ++ ")", Unpriced.names(rest)}

def Unpriced.line.go(none: Bool, xs: List<&2, Ent>) -> String:
  match none:
    case True{}:
      "Every response had a listed price."
    case False{}:
      "No listed price, left out of the cost: " ++ String.join(Unpriced.names(xs), ", ") ++ " responses."

def Unpriced.line(+xs: List<&2, Ent>) -> String:
  Unpriced.line.go(List.is_empty(&2, Ent, xs), xs)

# What was read and what could not be counted.
def Notes.lines.go(c: R.Counts, +v: View) -> List<&2, String>:
  match c:
    case R.Counts{bad_ts, unsplit, fast, +side, wf, ws}:
      ["Read " ++ Fmt.int(View.lines(v)) ++ " lines in " ++ Fmt.int(View.files(v)) ++ Plural(View.files(v), " transcript", " transcripts") ++ "; " ++ Fmt.int(View.bad(v)) ++ " were not JSON objects and were skipped.", Fmt.int(bad_ts) ++ " responses had no readable timestamp and are not counted.", Fmt.int(unsplit) ++ " cache-write tokens had no 5-minute/1-hour breakdown and were priced at the 5-minute rate.", Fmt.int(side) ++ Plural(side, " response", " responses") ++ " came from subagents; " ++ Fmt.int(fast) ++ " ran in fast mode.", "Usage records report " ++ Fmt.int(ws) ++ " server-side web searches (priced) and " ++ Fmt.int(wf) ++ " web fetches (free).", Unpriced.line(View.unpriced(v))]

def Notes.lines(+v: View) -> List<&2, String>:
  Notes.lines.go(View.counts(v), v)

def Choose.s(c: Bool, a: String, b: String) -> String:
  match c:
    case True{}:
      a
    case False{}:
      b

def Version() -> String:
  "ccwrapped 0.4.0"
