import Base
import ./bill.bend as B
import ./view.bend as V
import ./html.bend as H
import ./gen/assets.bend as A

# A share card: one 4:5 page with the year's headline numbers and its calendar. It never
# shows a project, a tool name or the time zone, with or without --share.

def Card.top_model(xs: List<&2, V.Ent>) -> String:
  match xs:
    case Nil{}:
      "none"
    case Con{V.Ent{+n, c, k, m, x}, rest}:
      V.Model.pretty(n)

def Card.item(+key: String, label: String, value: String) -> String:
  "<div><dt>" ++ label ++ "</dt>" ++ H.M("dd", "", key, value) ++ "</div>"

@unsafe def Card.render(+v: V.View) -> String:
  String.join([
    H.Page.head("Claude Code Wrapped " ++ Nat.show(V.View.year(v)), A.Css.card()),
    "<body>",
    "<main class='card'><div class='ci'>",
    "<div class='top'><p class='eyebrow'>Claude Code Wrapped</p>" ++ H.M("p", "yr", "year", Nat.show(V.View.year(v))) ++ "</div>",
    H.M("p", "hero", "cost", V.Fmt.usd(V.View.cost(v))),
    "<p class='cap'>API-equivalent value at list prices; a subscription is billed differently</p>",
    "<dl class='grid'>",
    Card.item("requests", "responses", V.Fmt.int(V.View.requests(v))),
    Card.item("human", "prompts typed", V.Fmt.int(V.View.human(v))),
    Card.item("tokens_total", "tokens", V.Fmt.big(B.Toks.sum(V.View.toks(v)))),
    Card.item("active_days", "active days", V.Fmt.int(V.View.active(v))),
    Card.item("streak", "longest streak", V.Fmt.int(V.View.streak(v)) ++ V.Plural(V.View.streak(v), " day", " days")),
    Card.item("top_model", "top model", V.Html.esc(Card.top_model(V.View.models(v)))),
    "</dl>",
    H.Heat(V.View.year(v), V.View.days(v)),
    "<div class='foot'><span>ccwrapped</span><span>prices as of " ++ A.Prices.date() ++ "</span></div>",
    "</div></main>",
    "</body>",
    "</html>",
    ""
  ], "\n")
