import Base
import ./bill.bend as B
import ./report.bend as R
import ./view.bend as V
import ./gen/assets.bend as A

# The report as HTML slides: one self-contained page, no scripts, and a content security
# policy that forbids every request, so the page cannot load anything or send anything.
# Every shown metric is marked data-m="<name>" for tests/views.py, which checks it against
# the oracle.

def Page.head(+title: String, css: String) -> String:
  String.join([
    "<!doctype html>",
    "<html lang='en'>",
    "<head>",
    "<meta charset='utf-8'>",
    "<meta name='viewport' content='width=device-width, initial-scale=1'>",
    "<meta name='color-scheme' content='dark'>",
    "<meta http-equiv='Content-Security-Policy' content=\"default-src 'none'; style-src 'unsafe-inline'\">",
    "<meta name='referrer' content='no-referrer'>",
    "<meta name='generator' content='" ++ V.Version() ++ "'>",
    "<title>" ++ title ++ "</title>",
    "<style>",
    css,
    "</style>",
    "</head>"
  ], "\n")

# A metric: its value in an element marked with its name.
def M(+tag: String, cls: String, +key: String, value: String) -> String:
  "<" ++ tag ++ " class='" ++ cls ++ "' data-m='" ++ V.Html.esc(key) ++ "'>" ++ value ++ "</" ++ tag ++ ">"

def Stat(+key: String, value: String, label: String) -> String:
  "<div class='stat'>" ++ M("p", "big", key, value) ++ "<p class='label'>" ++ label ++ "</p></div>"

def Slide(+cls: String, body: List<&2, String>) -> String:
  "<section class='slide s-" ++ cls ++ "' id='" ++ cls ++ "'><div class='in'>\n" ++ String.join(body, "\n") ++ "\n</div></section>"

def Eyebrow(s: String) -> String:
  "<p class='eyebrow'>" ++ s ++ "</p>"

def Lede(s: String) -> String:
  "<p class='lede'>" ++ s ++ "</p>"

def Pct.css.go(none: Bool, s: String) -> String:
  match none:
    case True{}:
      "0%"
    case False{}:
      s

# part/whole as a CSS percentage.
def Pct.css(part: Nat, +whole: Nat) -> String:
  Pct.css.go(Nat.is_eq(whole, 0n), V.Fmt.pct(part, whole))

# The calendar
# ------------

def Level.go(+c: Nat, +top: Nat) -> Nat:
  Nat.max(1n, Nat.min(4n, Nat.div((c * 4n + top - 1n : Nat), Nat.max(top, 1n))))

# 1 to 4 by the day's share of the busiest day's cost: quarters, rounded up.
def Level(+c: Nat, +top: Nat) -> Nat:
  V.Choose(Nat.is_eq(top, 0n), 1n, Level.go(c, top))

def Date.of(d: Nat & Nat & Nat) -> String:
  match d:
    case Tuple{y, Tuple{m, dd}}:
      R.Show4(y) ++ "-" ++ R.Show2(m) ++ "-" ++ R.Show2(dd)

def Cell.day(e: V.Ent, +top: Nat) -> String:
  match e:
    case V.Ent{n, +c, +k, m, x}:
      "<i class='l" ++ Nat.show(Level(c, top)) ++ "' title='" ++ n ++ " · " ++ V.Fmt.usd(m) ++ " · " ++ V.Fmt.int(k) ++ V.Plural(k, " response", " responses") ++ "'></i>"

def Cell.empty(date: String) -> String:
  "<i title='" ++ date ++ "'></i>"

def Ent.name(+e: V.Ent) -> String:
  match e:
    case V.Ent{n, c, k, m, x}:
      n

@unsafe def Heat.step(hit: Bool, +date: String, days: List<&2, V.Ent>, +day: Nat, end: Nat, +top: Nat) -> List<&2, String>:
  match hit days:
    case True{} Con{e, rest}:
      Con{Cell.day(e, top), Heat.go(rest, (day + 1n : Nat), end, top)}
    case _ days:
      Con{Cell.empty(date), Heat.go(days, (day + 1n : Nat), end, top)}

@unsafe def Heat.hit(+days: List<&2, V.Ent>, +date: String) -> Bool:
  match days:
    case Nil{}:
      False{}
    case Con{e, rest}:
      String.eq(Ent.name(e), date)

# One cell per day of the year, walking the date-sorted active days alongside.
@unsafe def Heat.go(+days: List<&2, V.Ent>, +day: Nat, +end: Nat, +top: Nat) -> List<&2, String>:
  Heat.more(Nat.is_lt(day, end), days, day, end, top)

@unsafe def Heat.more(more: Bool, +days: List<&2, V.Ent>, +day: Nat, end: Nat, top: Nat) -> List<&2, String>:
  match more:
    case True{}:
      Heat.at(Date.of(R.Civil.of(day)), days, day, end, top)
    case False{}:
      Nil{}

@unsafe def Heat.at(+date: String, +days: List<&2, V.Ent>, day: Nat, end: Nat, top: Nat) -> List<&2, String>:
  Heat.step(Heat.hit(days, date), date, days, day, end, top)

def Heat.pad(n: Nat) -> List<&2, String>:
  match n:
    case 0n:
      Nil{}
    case 1n+p:
      Con{"<i class='out'></i>", Heat.pad(p)}

# The year as a grid of weeks, Monday on top, like a contribution graph.
@unsafe def Heat(+year: Nat, +days: List<&2, V.Ent>) -> String:
  +d0 = R.Days.of(year, 1n, 1n)
  "<div class='heat' aria-label='Cost per day'>" ++ String.concat(List.append(&2, String, Heat.pad(Nat.mod((d0 + 2n : Nat), 7n)), Heat.go(days, d0, R.Days.of((year + 1n : Nat), 1n, 1n), V.Max.cost(days)))) ++ "</div>"

# Ranked bars
# -----------

def Bars.more.go(none: Bool, n: Nat, what: String) -> String:
  match none:
    case True{}:
      ""
    case False{}:
      "<p class='more'>and " ++ V.Fmt.int(n) ++ " more " ++ what ++ "</p>"

def Bars.more(+total: Nat, +shown: Nat, what: String) -> String:
  Bars.more.go(Nat.is_le(total, shown), (total - shown : Nat), what)

def Bars.list(rows: List<&2, String>) -> String:
  "<ol class='bars'>\n" ++ String.join(rows, "\n") ++ "\n</ol>"

def Bar(name: String, title: String, +key: String, value: String, width: String, sub: String) -> String:
  "<li><span class='name' title='" ++ title ++ "'>" ++ name ++ "</span>" ++ M("span", "val", key, value) ++ "<span class='track'><span class='fill' style='width:" ++ width ++ "'></span></span><span class='sub'>" ++ sub ++ "</span></li>"

def Rows.models(xs: List<&2, V.Ent>, n: Nat, +top: Nat, +total: Nat, +unpriced: List<&2, String>) -> List<&2, String>:
  match xs n:
    case _ 0n:
      Nil{}
    case Nil{} 1n+p:
      Nil{}
    case Con{V.Ent{+name, +cost, +count, money, toks}, rest} 1n+p:
      Con{Bar(V.Html.esc(V.Model.pretty(name)), V.Html.esc(V.Unpriced.name(name)), "model:" ++ name, V.Cost.cell(V.Names.has(unpriced, name), money), Pct.css(cost, top), V.Fmt.int(count) ++ V.Plural(count, " response", " responses") ++ " · " ++ V.Fmt.big(toks) ++ " tokens · " ++ V.Fmt.pct(cost, total) ++ " of the cost"), Rows.models(rest, p, top, total, unpriced)}

def Rows.projects(xs: List<&2, V.Ent>, n: Nat, +top: Nat, +total: Nat) -> List<&2, String>:
  match xs n:
    case _ 0n:
      Nil{}
    case Nil{} 1n+p:
      Nil{}
    case Con{V.Ent{+name, +cost, +count, money, +sessions}, rest} 1n+p:
      Con{Bar(V.Html.esc(name), V.Html.esc(name), "project:" ++ name, V.Fmt.usd(money), Pct.css(cost, top), V.Fmt.int(count) ++ V.Plural(count, " response", " responses") ++ " · " ++ V.Fmt.int(sessions) ++ V.Plural(sessions, " session", " sessions") ++ " · " ++ V.Fmt.pct(cost, total) ++ " of the cost"), Rows.projects(rest, p, top, total)}

def Rows.tools(xs: List<&2, V.Ent>, n: Nat, +top: Nat, +total: Nat) -> List<&2, String>:
  match xs n:
    case _ 0n:
      Nil{}
    case Nil{} 1n+p:
      Nil{}
    case Con{V.Ent{+name, cost, +count, money, x}, rest} 1n+p:
      Con{Bar(V.Html.esc(V.Tool.label(name)), V.Html.esc(V.Tool.label(name)), "tool:" ++ name, V.Fmt.int(count), Pct.css(count, top), V.Fmt.pct(count, total) ++ " of tool calls"), Rows.tools(rest, p, top, total)}

def Top.cost(xs: List<&2, V.Ent>) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{V.Ent{n, c, k, m, x}, rest}:
      c

def Top.count(xs: List<&2, V.Ent>) -> Nat:
  match xs:
    case Nil{}:
      0n
    case Con{V.Ent{n, c, k, m, x}, rest}:
      k

# The slides
# ----------

def Intro(+v: V.View) -> String:
  Slide("intro", [
    Eyebrow("Claude Code Wrapped"),
    M("h1", "year", "year", Nat.show(V.View.year(v))),
    Lede("A year of Claude Code, read from " ++ M("b", "num", "files", V.Fmt.int(V.View.files(v))) ++ V.Plural(V.View.files(v), " transcript", " transcripts") ++ " on this machine. Days and hours are in " ++ M("b", "", "zone", V.Html.esc(V.View.zone(v))) ++ "."),
    "<p class='hint'>Scroll ↓</p>"
  ])

def Unpriced.count(xs: List<&2, V.Ent>) -> Nat:
  V.Sum.counts(xs)

def Cost.unpriced.go(none: Bool, +n: Nat) -> String:
  match none:
    case True{}:
      ""
    case False{}:
      " " ++ V.Fmt.int(n) ++ V.Plural(n, " of them went to a model", " of them went to models") ++ " without a listed price, so their cost is not in this figure."

def Cost.unpriced(+v: V.View) -> String:
  Cost.unpriced.go(Nat.is_eq(Unpriced.count(V.View.unpriced(v)), 0n), Unpriced.count(V.View.unpriced(v)))

def Cost.slide(+v: V.View) -> String:
  Slide("cost", [
    Eyebrow("At API list prices"),
    M("p", "hero", "cost", V.Fmt.usd(V.View.cost(v))),
    Lede("What " ++ M("b", "num", "requests", V.Fmt.int(V.View.requests(v))) ++ V.Plural(V.View.requests(v), " response", " responses") ++ " would have cost through the Anthropic API, at prices as of " ++ A.Prices.date() ++ ". A subscription is billed differently: this is the value of the traffic, not a bill." ++ Cost.unpriced(v))
  ])

def Ratio.go(none: Bool, +a: Nat, b: Nat) -> String:
  match none:
    case True{}:
      ""
    case False{}:
      Lede("About " ++ V.Fmt.scaled(a, b, "") ++ " responses for every prompt you typed: the rest of the work ran on tool results.")

def Ratio(+a: Nat, +b: Nat) -> String:
  Ratio.go(Nat.is_eq(b, 0n), a, b)

def Talk.slide(+v: V.View) -> String:
  Slide("talk", [
    Eyebrow("The conversation"),
    "<div class='stats'>",
    Stat("human", V.Fmt.int(V.View.human(v)), "prompts you typed"),
    Stat("requests", V.Fmt.int(V.View.requests(v)), "responses from Claude"),
    Stat("tool_calls", V.Fmt.int(V.View.calls(v)), "tool calls"),
    Stat("sessions", V.Fmt.int(V.View.sessions(v)), "sessions"),
    "</div>",
    Ratio(V.View.requests(v), V.View.human(v))
  ])

def Legend(cls: String, label: String, +key: String, n: Nat, +total: Nat) -> String:
  "<li><i class='" ++ cls ++ "'></i><span>" ++ label ++ "</span>" ++ M("b", "", key, V.Fmt.int(n)) ++ "</li>"

def Seg(cls: String, n: Nat, +total: Nat) -> String:
  "<span class='" ++ cls ++ "' style='width:" ++ Pct.css(n, total) ++ "'></span>"

def Toks.slide.go(t: B.Toks, +total: Nat) -> String:
  match t:
    case B.Toks{+i, +a, +b, +r, +o}:
      Slide("tokens", [
        Eyebrow("Tokens"),
        "<p class='hero'>" ++ M("span", "", "tokens_total", V.Fmt.big(total)) ++ "</p>",
        Lede(M("b", "num", "cache_read_share", V.Fmt.pct(r, total)) ++ " of them were cache reads: context sent again and served from the prompt cache, the cheapest kind of token. " ++ M("b", "num", "tok:output", V.Fmt.int(o)) ++ " were written by Claude."),
        "<div class='stack'>" ++ Seg("t-in", i, total) ++ Seg("t-w5", a, total) ++ Seg("t-w1", b, total) ++ Seg("t-rd", r, total) ++ Seg("t-out", o, total) ++ "</div>",
        "<ul class='legend'>",
        Legend("t-in", "Input", "tok:input", i, total),
        Legend("t-w5", "Cache writes, 5 min", "tok:cache_write_5m", a, total),
        Legend("t-w1", "Cache writes, 1 hour", "tok:cache_write_1h", b, total),
        Legend("t-rd", "Cache reads", "tok:cache_read", r, total),
        Legend("t-out", "Output", "tok:output", o, total),
        "</ul>"
      ])

def Toks.slide(+v: V.View) -> String:
  Toks.slide.go(V.View.toks(v), B.Toks.sum(V.View.toks(v)))

def Lead.name(pretty: Bool, +n: String) -> String:
  match pretty:
    case True{}:
      V.Model.pretty(n)
    case False{}:
      n

def Lead.go(none: Bool, +xs: List<&2, V.Ent>, key: String, pretty: Bool, +total: Nat, what: String) -> String:
  match none xs:
    case True{} xs:
      "<h2 class='headline'>None this year</h2>"
    case False{} Con{V.Ent{+n, c, k, m, x}, rest}:
      "<h2 class='headline'>" ++ M("span", "", key, V.Html.esc(Lead.name(pretty, n))) ++ "</h2>" ++ Lede(what ++ " " ++ V.Fmt.pct(c, total) ++ " of the cost.")
    case False{} Nil{}:
      ""

def Models.slide(+v: V.View) -> String:
  Slide("models", [
    Eyebrow("Models"),
    Lead.go(List.is_empty(&2, V.Ent, V.View.models(v)), V.View.models(v), "top_model", True{}, V.Money.micro(V.View.cost(v)), "Your most expensive model, with"),
    Bars.list(Rows.models(V.View.models(v), 16n, Top.cost(V.View.models(v)), V.Money.micro(V.View.cost(v)), V.Ents.names(V.View.unpriced(v)))),
    Bars.more(List.length(&2, V.Ent, V.View.models(v)), 16n, "models")
  ])

def Projects.slide(+v: V.View) -> String:
  Slide("projects", [
    Eyebrow("Projects"),
    Lead.go(List.is_empty(&2, V.Ent, V.View.projects(v)), V.View.projects(v), "top_project", False{}, V.Money.micro(V.View.cost(v)), "The working directory that cost the most, with"),
    Bars.list(Rows.projects(V.View.projects(v), 8n, Top.cost(V.View.projects(v)), V.Money.micro(V.View.cost(v)))),
    Bars.more(List.length(&2, V.Ent, V.View.projects(v)), 8n, "projects")
  ])

def Note.go(empty: Bool, s: String) -> String:
  match empty:
    case True{}:
      ""
    case False{}:
      "<p class='note'>" ++ s ++ "</p>"

def Note(+s: String) -> String:
  Note.go(String.is_empty(s), s)

def Tools.slide(+v: V.View) -> String:
  Slide("tools", [
    Eyebrow("Tools"),
    "<h2 class='headline'>" ++ M("span", "num", "tool_calls", V.Fmt.int(V.View.calls(v))) ++ " tool calls</h2>",
    Bars.list(Rows.tools(V.View.tools(v), 10n, Top.count(V.View.tools(v)), V.View.calls(v))),
    Bars.more(List.length(&2, V.Ent, V.View.tools(v)), 10n, "tools"),
    Note(V.Web.note(v))
  ])

def Month.labels() -> String:
  "<div class='months'><span>Jan</span><span>Feb</span><span>Mar</span><span>Apr</span><span>May</span><span>Jun</span><span>Jul</span><span>Aug</span><span>Sep</span><span>Oct</span><span>Nov</span><span>Dec</span></div>"

def Scale() -> String:
  "<div class='scale'><span>less</span><i style='background:var(--h0)'></i><i style='background:var(--h1)'></i><i style='background:var(--h2)'></i><i style='background:var(--h3)'></i><i style='background:var(--h4)'></i><span>more cost</span></div>"

@unsafe def Busiest.go(none: Bool, e: V.Ent) -> String:
  match none e:
    case True{} e:
      ""
    case False{} V.Ent{n, c, +k, m, x}:
      Lede("Busiest day: " ++ M("b", "", "busiest_day", n) ++ ", with " ++ M("b", "num", "busiest_day_cost", V.Fmt.usd(m)) ++ " across " ++ V.Fmt.int(k) ++ V.Plural(k, " response", " responses") ++ ".")

@unsafe def Year.slide(+v: V.View) -> String:
  Slide("rhythm", [
    Eyebrow("The year"),
    "<div class='stats'>",
    Stat("active_days", V.Fmt.int(V.View.active(v)), "active days"),
    Stat("streak", V.Fmt.int(V.View.streak(v)), "days in a row, the longest streak"),
    "</div>",
    Heat(V.View.year(v), V.View.days(v)),
    Month.labels(),
    Scale(),
    Busiest.go(Nat.is_eq(V.View.active(v), 0n), V.View.peak_day(v))
  ])

def Hour.bar(+i: Nat, +n: Nat, +top: Nat, peak: Bool) -> String:
  "<span" ++ V.Choose.s(peak, " class='peak'", "") ++ " style='height:" ++ Pct.css(n, top) ++ "' title='" ++ R.Show2(i) ++ ":00 · " ++ V.Fmt.int(n) ++ V.Plural(n, " response", " responses") ++ "'></span>"

def Hour.bars(xs: List<&2, Nat>, +i: Nat, +top: Nat, +peak: Nat) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{n, rest}:
      Con{Hour.bar(i, n, top, Nat.is_eq(i, peak) && Nat.is_gt(top, 0n)), Hour.bars(rest, (i + 1n : Nat), top, peak)}

def Week.bar(+i: Nat, +n: Nat, +top: Nat, peak: Bool) -> String:
  "<span" ++ V.Choose.s(peak, " class='peak'", "") ++ " style='height:" ++ Pct.css(n, top) ++ "' title='" ++ V.Weekday.name(i) ++ " · " ++ V.Fmt.int(n) ++ V.Plural(n, " response", " responses") ++ "'></span>"

def Week.bars(xs: List<&2, Nat>, +i: Nat, +top: Nat, +peak: Nat) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{n, rest}:
      Con{Week.bar(i, n, top, Nat.is_eq(i, peak) && Nat.is_gt(top, 0n)), Week.bars(rest, (i + 1n : Nat), top, peak)}

def Clock.lede.go(none: Bool, +v: V.View) -> String:
  match none:
    case True{}:
      ""
    case False{}:
      Lede("Most responses came at " ++ M("b", "num", "peak_hour", R.Show2(V.Peak.index(V.View.hours(v))) ++ ":00") ++ ", and on " ++ M("b", "", "peak_weekday", V.Weekday.name(V.Peak.index(V.View.weekdays(v))) ++ "s") ++ ".")

def Clock.slide(+v: V.View) -> String:
  Slide("clock", [
    Eyebrow("The clock"),
    "<h2 class='headline'>When you worked</h2>",
    Clock.lede.go(Nat.is_eq(V.View.requests(v), 0n), v),
    "<div class='clock' aria-label='Responses per hour'>" ++ String.concat(Hour.bars(V.View.hours(v), 0n, V.Max.nat(V.View.hours(v)), V.Peak.index(V.View.hours(v)))) ++ "</div>",
    "<div class='ticks'><span>00:00</span><span>06:00</span><span>12:00</span><span>18:00</span></div>",
    "<div class='week' aria-label='Responses per weekday'>" ++ String.concat(Week.bars(V.View.weekdays(v), 0n, V.Max.nat(V.View.weekdays(v)), V.Peak.index(V.View.weekdays(v)))) ++ "</div>",
    "<div class='days'><span>Mon</span><span>Tue</span><span>Wed</span><span>Thu</span><span>Fri</span><span>Sat</span><span>Sun</span></div>"
  ])

def Items(xs: List<&2, String>) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{x, rest}:
      Con{"<li>" ++ V.Html.esc(x) ++ "</li>", Items(rest)}

def Share.note(share: Bool) -> List<&2, String>:
  match share:
    case True{}:
      ["Projects are shown by rank and MCP tools as one entry (--share)."]
    case False{}:
      Nil{}

def Fine.slide(+v: V.View) -> String:
  Slide("fine", [
    "<div class='fine'>",
    "<h2>How this was counted</h2>",
    "<ul>",
    String.join(Items(List.concat(&2, String, [[
      "A response is one API call. Claude Code writes each one several times (once per streamed block, again on resume and in subagent transcripts); the copies are merged by message id.",
      "Cost is the API-equivalent value at Anthropic's list prices as of " ++ A.Prices.date() ++ ", from exact integer arithmetic. It is not what a subscription costs.",
      "Human prompts are messages you typed, not tool results or text Claude Code writes itself."
    ], V.Notes.lines(v), Share.note(V.View.share(v))])), "\n"),
    "</ul>",
    "<p class='meta'>" ++ V.Version() ++ " · every rule is in docs/semantics.md · nothing on this page was sent anywhere</p>",
    "</div>"
  ])

@unsafe def Html.render(+v: V.View) -> String:
  String.join([
    Page.head("Claude Code Wrapped " ++ Nat.show(V.View.year(v)), A.Css.slides()),
    "<body>",
    "<main>",
    Intro(v),
    Cost.slide(v),
    Talk.slide(v),
    Toks.slide(v),
    Models.slide(v),
    Projects.slide(v),
    Tools.slide(v),
    Year.slide(v),
    Clock.slide(v),
    Fine.slide(v),
    "</main>",
    "</body>",
    "</html>",
    ""
  ], "\n")
