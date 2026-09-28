import Base
import ./bill.bend as B
import ./report.bend as R
import ./view.bend as V
import ./text.bend as X
import ./html.bend as H
import ./card.bend as C

# The formats for people. Each one renders the same View, built here once from the aggregate.

# With --share, a project is shown by its cost rank, never by its path.
def Ents.share(share: Bool, +xs: List<&2, V.Ent>, +i: Nat) -> List<&2, V.Ent>:
  match share xs:
    case True{} Con{V.Ent{n, c, k, m, x}, rest}:
      Con{V.Ent{"project-" ++ Nat.show(i), c, k, m, x}, Ents.share(True{}, rest, (i + 1n : Nat))}
    case _ xs:
      xs

def Mcp.is(+name: String) -> Bool:
  String.starts_with(name, "mcp__")

def Mcp.sum(xs: List<&2, V.Ent>) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{V.Ent{+n, c, k, m, x}, rest}:
      (V.Choose(Mcp.is(n), k, 0n) + Mcp.sum(rest) : Nat)

def Mcp.drop.pick(mcp: Bool, e: V.Ent, rest: List<&2, V.Ent>) -> List<&2, V.Ent>:
  match mcp:
    case True{}:
      rest
    case False{}:
      Con{e, rest}

def Mcp.drop(xs: List<&2, V.Ent>) -> List<&2, V.Ent>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{V.Ent{+n, c, k, m, x}, rest}:
      Mcp.drop.pick(Mcp.is(n), V.Ent{n, c, k, m, x}, Mcp.drop(rest))

def Tools.share.go(none: Bool, +total: Nat, xs: List<&2, V.Ent>) -> List<&2, V.Ent>:
  match none:
    case True{}:
      xs
    case False{}:
      List.sort(~V.Ent, ~V.Ent.by_count, Con{V.Ent{"MCP tools", 0n, total, B.Money.zero(), 0n}, xs})

# An MCP tool's name carries the name of its server, which can be private: with --share all
# MCP tools are counted as one.
def Tools.share(share: Bool, +xs: List<&2, V.Ent>) -> List<&2, V.Ent>:
  match share:
    case True{}:
      Tools.share.go(Nat.is_eq(Mcp.sum(xs), 0n), Mcp.sum(xs), Mcp.drop(xs))
    case False{}:
      xs

def Slots.get(got: Map<&2, Nat> & Nat) -> Nat:
  match got:
    case Tuple{m, v}:
      v

def Slots.nats(n: Nat, +i: Nat, +m: Map<&2, Nat>) -> List<&2, Nat>:
  match n:
    case 0n:
      Nil{}
    case 1n+p:
      Con{Slots.get(Map.get(Nat, 0n, m, Nat.show(i))), Slots.nats(p, (i + 1n : Nat), m)}

def View.of.tools(+share: Bool, year: Nat, zone: String, requests: Nat, toks: B.Toks, cost: B.Money, models: List<&2, V.Ent>, unpriced: List<&2, V.Ent>, projects: List<&2, V.Ent>, days: List<&2, V.Ent>, hours: Map<&2, Nat>, weekdays: Map<&2, Nat>, sessions: Nat, counts: R.Counts, +tools: List<&2, V.Ent>, human: Nat, streak: Nat, lines: Nat, bad: Nat, files: Nat) -> V.View:
  V.View{year, zone, share, cost, requests, human, V.Sum.counts(tools), sessions, streak, toks, models, unpriced, Ents.share(share, projects, 1n), Tools.share(share, tools), days, Slots.nats(24n, 0n, hours), Slots.nats(7n, 0n, weekdays), counts, lines, bad, files, V.Tool.calls(tools, "WebSearch"), V.Tool.calls(tools, "WebFetch")}

def View.of(share: Bool, year: Nat, zone: String, a: R.Agg, tool_calls: Map<&2, Nat>, human: Nat, streak: Nat, lines: Nat, bad: Nat, files: Nat) -> V.View:
  match a:
    case R.Agg{requests, toks, cost, models, unpriced, projects, days, hours, weekdays, sessions, counts}:
      View.of.tools(share, year, zone, requests, toks, cost, List.sort(~V.Ent, ~V.Ent.by_cost, V.Ents.models(Map.to_list(&2, R.ModelAgg, models))), List.sort(~V.Ent, ~V.Ent.name_le, V.Ents.nats(Map.to_list(&2, Nat, unpriced))), List.sort(~V.Ent, ~V.Ent.by_cost, V.Ents.projects(Map.to_list(&2, R.ProjAgg, projects))), List.sort(~V.Ent, ~V.Ent.name_le, V.Ents.days(Map.to_list(&2, R.DayAgg, days))), hours, weekdays, Map.size(&2, Unit, sessions), counts, List.sort(~V.Ent, ~V.Ent.by_count, V.Ents.nats(Map.to_list(&2, Nat, tool_calls))), human, streak, lines, bad, files)

# fmt: 0 terminal report, 2 Markdown report, 3 HTML slides, 4 share card.
@unsafe def Render.view(fmt: Nat, +v: V.View) -> String:
  match fmt:
    case 2n:
      X.Text.render(True{}, v)
    case 3n:
      H.Html.render(v)
    case 4n:
      C.Card.render(v)
    case _:
      X.Text.render(False{}, v)

@unsafe def Render.report(fmt: Nat, share: Bool, year: Nat, zone: String, a: R.Agg, tool_calls: Map<&2, Nat>, human: Nat, streak: Nat, lines: Nat, bad: Nat, files: Nat) -> String:
  Render.view(fmt, View.of(share, year, zone, a, tool_calls, human, streak, lines, bad, files))
