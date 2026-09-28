import Base

# Billing arithmetic: exact money, the billed token categories of a response, and its cost.
# Everything here is total and free of foreign code, so its laws are checked (PROOF.bend).

# Money
# -----

# An exact amount of picodollars, kept as whole microdollars plus a remainder below 10^6 so
# that totals stay far inside a Nat (which aborts past 2^48).

type Money is Data:
  Money{micro: Nat, pico: Nat}

def Money.zero() -> Money:
  Money{0n, 0n}

def Mega() -> Nat:
  1000000n

def Money.norm(micro: Nat, +pico: Nat) -> Money:
  Money{(micro + Nat.div(pico, Mega()) : Nat), Nat.mod(pico, Mega())}

def Money.add(a: Money, b: Money) -> Money:
  match a b:
    case Money{am, ap} Money{bm, bp}:
      Money.norm((am + bm : Nat), (ap + bp : Nat))

# t tokens at p microdollars per million tokens, exactly: t * p picodollars, computed as
# (t div 10^6) * p microdollars plus (t mod 10^6) * p picodollars, so no product nears 2^48
# for any count up to 10^9 and any listed price.
def Money.mul.go(q: Nat, r: Nat, +p: Nat) -> Money:
  Money.norm((q * p : Nat), (r * p : Nat))

def Money.mul(+t: Nat, p: Nat) -> Money:
  Money.mul.go(Nat.div(t, Mega()), Nat.mod(t, Mega()), p)

# Picodollars below 2^48.
def Money.of(+p: Nat) -> Money:
  Money{Nat.div(p, Mega()), Nat.mod(p, Mega())}

def Pad6.go(n: Nat, s: String) -> String:
  match n:
    case 0n:
      s
    case 1n+p:
      Pad6.go(p, "0" ++ s)

def Pad6(+s: String) -> String:
  Pad6.go((6n - String.length(s) : Nat), s)

def Money.show.if(zero: Bool, micro: Nat, pico: Nat) -> String:
  match zero:
    case True{}:
      Nat.show(pico)
    case False{}:
      Nat.show(micro) ++ Pad6(Nat.show(pico))

def Money.show(m: Money) -> String:
  match m:
    case Money{+micro, pico}:
      Money.show.if(Nat.is_eq(micro, 0n), micro, pico)

type Row is Data:
  Row{inp: Nat, w5: Nat, w1: Nat, read: Nat, out: Nat}

# US-only inference multiplies every category by 1.1; every listed price is divisible by 10.
def Row.geo(geo: Bool, +r: Row) -> Row:
  match geo r:
    case True{} Row{inp, w5, w1, read, out}:
      Row{Nat.div((inp * 11n : Nat), 10n), Nat.div((w5 * 11n : Nat), 10n), Nat.div((w1 * 11n : Nat), 10n), Nat.div((read * 11n : Nat), 10n), Nat.div((out * 11n : Nat), 10n)}
    case False{} r:
      r

# Per request
# -----------

# The five billed token categories of one response.
type Toks is Data:
  Toks{inp: Nat, w5: Nat, w1: Nat, read: Nat, out: Nat}

def Toks.zero() -> Toks:
  Toks{0n, 0n, 0n, 0n, 0n}

def Toks.add(a: Toks, b: Toks) -> Toks:
  match a b:
    case Toks{ai, a5, a1, ar, ao} Toks{bi, b5, b1, br, bo}:
      Toks{(ai + bi : Nat), (a5 + b5 : Nat), (a1 + b1 : Nat), (ar + br : Nat), (ao + bo : Nat)}

def Toks.of.go(inp: Nat, out: Nat, cread: Nat, cwrite: Nat, +w1: Nat) -> Toks:
  Toks{inp, (cwrite - w1 : Nat), w1, cread, out}

# The cache-write total is authoritative; the 1-hour share comes from the split (capped at the
# total) and the rest is billed at the 5-minute rate.
def Toks.of(inp: Nat, out: Nat, cread: Nat, +cwrite: Nat, c1h: Nat) -> Toks:
  Toks.of.go(inp, out, cread, cwrite, Nat.min(c1h, cwrite))

def Toks.sum(t: Toks) -> Nat:
  match t:
    case Toks{i, a, b, r, o}:
      (i + a + b + r + o : Nat)

# The cache-write tokens a response is billed for, over both durations.
def Toks.writes(t: Toks) -> Nat:
  match t:
    case Toks{i, a, b, r, o}:
      (a + b : Nat)

# Tokens times microdollars per million tokens is picodollars; web search is $10 per 1,000.
def Cost.of(t: Toks, r: Row, ws: Nat) -> Money:
  match t r:
    case Toks{i, a, b, rd, o} Row{pi, pa, pb, pr, po}:
      Money.add(Money.add(Money.add(Money.mul(i, pi), Money.mul(a, pa)), Money.add(Money.mul(b, pb), Money.mul(rd, pr))), Money.add(Money.mul(o, po), Money{(ws * 10000n : Nat), 0n}))
