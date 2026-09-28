import Base
import ./bill.bend as B
import ./report.bend as R
import ./view.bend as V
import ./gen/assets.bend as A

# The report for a terminal, and the same report as Markdown.

def Row(md: Bool, cells: List<&2, String>) -> String:
  match md:
    case True{}:
      "| " ++ String.join(cells, " | ") ++ " |"
    case False{}:
      "  " ++ String.join(cells, "  ")

def Heading(md: Bool, +s: String) -> String:
  match md:
    case True{}:
      "\n## " ++ s ++ "\n"
    case False{}:
      "\n" ++ s ++ "\n" ++ String.repeat("-", String.length(s))

def Rule(md: Bool, n: Nat) -> List<&2, String>:
  match md:
    case True{}:
      ["|" ++ String.repeat(" --- |", n)]
    case False{}:
      Nil{}

def Title(md: Bool, +s: String) -> String:
  match md:
    case True{}:
      "# " ++ s
    case False{}:
      s ++ "\n" ++ String.repeat("=", String.length(s))

# A table: a header row, the Markdown rule, the rows.
def Table(+md: Bool, header: List<&2, String>, n: Nat, rows: List<&2, String>) -> String:
  String.join(Con{Row(md, header), List.append(&2, String, Rule(md, n), rows)}, "\n")

def Table.or_none.go(empty: Bool, +md: Bool, header: List<&2, String>, n: Nat, rows: List<&2, String>, none: String) -> String:
  match empty:
    case True{}:
      none
    case False{}:
      Table(md, header, n, rows)

def Table.or_none(+md: Bool, header: List<&2, String>, n: Nat, +rows: List<&2, String>, none: String) -> String:
  Table.or_none.go(List.is_empty(&2, String, rows), md, header, n, rows, none)

def Lines.models(+md: Bool, xs: List<&2, V.Ent>, +total: Nat, +unpriced: List<&2, String>) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{V.Ent{+name, cost, count, money, toks}, rest}:
      Con{Row(md, [V.Pad.r(V.Md.name(md, V.Unpriced.name(name)), 28n), V.Pad.l(V.Fmt.int(count), 10n), V.Pad.l(V.Fmt.big(toks), 8n), V.Pad.l(V.Cost.cell(V.Names.has(unpriced, name), money), 12n), V.Pad.l(V.Fmt.pct(cost, total), 6n)]), Lines.models(md, rest, total, unpriced)}

def Lines.projects(+md: Bool, xs: List<&2, V.Ent>, n: Nat, +total: Nat) -> List<&2, String>:
  match xs n:
    case _ 0n:
      Nil{}
    case Nil{} 1n+p:
      Nil{}
    case Con{V.Ent{name, cost, count, money, sessions}, rest} 1n+p:
      Con{Row(md, [V.Pad.r(V.Md.name(md, V.Clip(V.Clean(name), 36n)), 36n), V.Pad.l(V.Fmt.int(count), 9n), V.Pad.l(V.Fmt.int(sessions), 8n), V.Pad.l(V.Fmt.usd(money), 12n), V.Pad.l(V.Fmt.pct(cost, total), 6n)]), Lines.projects(md, rest, p, total)}

def Lines.tools(+md: Bool, xs: List<&2, V.Ent>, n: Nat, +total: Nat) -> List<&2, String>:
  match xs n:
    case _ 0n:
      Nil{}
    case Nil{} 1n+p:
      Nil{}
    case Con{V.Ent{name, cost, +count, money, x}, rest} 1n+p:
      Con{Row(md, [V.Pad.r(V.Md.name(md, V.Clip(V.Clean(V.Tool.label(name)), 36n)), 36n), V.Pad.l(V.Fmt.int(count), 9n), V.Pad.l(V.Fmt.pct(count, total), 6n)]), Lines.tools(md, rest, p, total)}

def Toks.lines(+md: Bool, t: B.Toks) -> List<&2, String>:
  match t:
    case B.Toks{i, a, b, r, o}:
      [Row(md, [V.Pad.r("Input", 22n), V.Pad.l(V.Fmt.int(i), 18n)]), Row(md, [V.Pad.r("Cache writes, 5 min", 22n), V.Pad.l(V.Fmt.int(a), 18n)]), Row(md, [V.Pad.r("Cache writes, 1 hour", 22n), V.Pad.l(V.Fmt.int(b), 18n)]), Row(md, [V.Pad.r("Cache reads", 22n), V.Pad.l(V.Fmt.int(r), 18n)]), Row(md, [V.Pad.r("Output", 22n), V.Pad.l(V.Fmt.int(o), 18n)])]

def Total.row(+md: Bool, label: String, value: String) -> String:
  Row(md, [V.Pad.r(label, 22n), V.Pad.l(value, 18n)])

def Totals.lines(+md: Bool, +v: V.View) -> List<&2, String>:
  [Total.row(md, "API-equivalent cost", V.Fmt.usd(V.View.cost(v))), Total.row(md, "Responses", V.Fmt.int(V.View.requests(v))), Total.row(md, "Human prompts", V.Fmt.int(V.View.human(v))), Total.row(md, "Tool calls", V.Fmt.int(V.View.calls(v))), Total.row(md, "Sessions", V.Fmt.int(V.View.sessions(v))), Total.row(md, "Active days", V.Fmt.int(V.View.active(v))), Total.row(md, "Longest streak", V.Fmt.int(V.View.streak(v)) ++ V.Plural(V.View.streak(v), " day", " days"))]

@unsafe def Activity.some(+md: Bool, +v: V.View, peak: V.Ent) -> String:
  match peak:
    case V.Ent{pn, pc, pk, pm, px}:
      Para(md, ["Daily cost, one mark per active day:", V.Spark.go(V.View.days(v), V.Max.cost(V.View.days(v))), "Busiest day: " ++ pn ++ " (" ++ V.Fmt.usd(pm) ++ ", " ++ V.Fmt.int(pk) ++ " responses).", "Most responses at " ++ R.Show2(V.Peak.index(V.View.hours(v))) ++ ":00 and on " ++ V.Weekday.name(V.Peak.index(V.View.weekdays(v))) ++ "s."])

@unsafe def Activity.go(none: Bool, +md: Bool, +v: V.View) -> String:
  match none:
    case True{}:
      "No responses in " ++ Nat.show(V.View.year(v)) ++ "."
    case False{}:
      Activity.some(md, v, V.View.peak_day(v))

@unsafe def Activity.lines(+md: Bool, +v: V.View) -> String:
  Activity.go(Nat.is_eq(V.View.active(v), 0n), md, v)

# Markdown would run consecutive lines into one paragraph.
def Para(md: Bool, lines: List<&2, String>) -> String:
  match md:
    case True{}:
      String.join(lines, "\n\n")
    case False{}:
      String.join(lines, "\n")

def Bullets.go(xs: List<&2, String>) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{x, rest}:
      Con{"- " ++ V.Md.esc(x), Bullets.go(rest)}

def Bullets(md: Bool, xs: List<&2, String>) -> String:
  match md:
    case True{}:
      String.join(Bullets.go(xs), "\n")
    case False{}:
      String.join(xs, "\n")

def Web.lines.go(empty: Bool, s: String) -> List<&2, String>:
  match empty:
    case True{}:
      Nil{}
    case False{}:
      [s]

def Web.lines(+s: String) -> List<&2, String>:
  Web.lines.go(String.is_empty(s), s)

@unsafe def Text.render(+md: Bool, +v: V.View) -> String:
  +cost = V.Money.micro(V.View.cost(v))
  String.join([
    Title(md, "Claude Code usage, " ++ Nat.show(V.View.year(v))),
    "",
    "Local time: " ++ V.Md.name(md, V.View.zone(v)) ++ ". Cost is what the same traffic would cost at Anthropic API list prices (as of " ++ A.Prices.date() ++ "); a subscription is billed differently.",
    Heading(md, "Totals"),
    Table(md, [V.Pad.r("Measure", 22n), V.Pad.l("Value", 18n)], 2n, Totals.lines(md, v)),
    Heading(md, "Tokens"),
    Table(md, [V.Pad.r("Category", 22n), V.Pad.l("Tokens", 18n)], 2n, Toks.lines(md, V.View.toks(v))),
    Heading(md, "Models"),
    Table.or_none(md, [V.Pad.r("Model", 28n), V.Pad.l("Responses", 10n), V.Pad.l("Tokens", 8n), V.Pad.l("Cost", 12n), V.Pad.l("Share", 6n)], 5n, Lines.models(md, V.View.models(v), cost, V.Ents.names(V.View.unpriced(v))), "None."),
    Heading(md, "Projects"),
    Table.or_none(md, [V.Pad.r("Project", 36n), V.Pad.l("Responses", 9n), V.Pad.l("Sessions", 8n), V.Pad.l("Cost", 12n), V.Pad.l("Share", 6n)], 5n, Lines.projects(md, V.View.projects(v), 12n, cost), "None."),
    Heading(md, "Tools"),
    Table.or_none(md, [V.Pad.r("Tool", 36n), V.Pad.l("Calls", 9n), V.Pad.l("Share", 6n)], 3n, Lines.tools(md, V.View.tools(v), 12n, V.View.calls(v)), "None."),
    Heading(md, "Activity"),
    Activity.lines(md, v),
    Heading(md, "Notes"),
    Bullets(md, List.append(&2, String, V.Notes.lines(v), Web.lines(V.Web.note(v))))
  ], "\n")
