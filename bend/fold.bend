import Base
import ./types.bend as T
import ./gen/line.bend as L

# One API response, merged over every transcript line that repeats it. Streaming writes the
# same response several times with growing output counts, and resumed sessions copy earlier
# lines into new files, so every count merges by maximum and the attribution (timestamp,
# session, cwd, model, sidechain) by lexicographic minimum. That makes the merge commutative,
# associative and idempotent: totals do not depend on file order or on how often a response
# was repeated (LAWS.bend).
# origin is where the response is attributed: [timestamp, session, cwd, model, sidechain "0" or
# "1"], the least such list among the response's copies.

type Group is Data:
  Group{inp: Nat, out: Nat, cread: Nat, cwrite: Nat, c1h: Nat, c5m: Nat, ws: Nat, wf: Nat, fast: Bool, geo: Bool, origin: List<&2, String>}

type Ev is Data:
  ENone{}
  EAssist{mid: String, ts: String, g: Group, tools: List<&2, T.Tool>}
  EUser{uuid: String, ts: String}

type Fold is Data:
  Fold{groups: Map<&2, Group>, tools: Map<&2, List<&2, String>>, prompts: Map<&2, String>, lines: Nat, bad: Nat, usage: Nat, files: Nat}

def Fold.empty() -> Fold:
  Fold{MTip{}, MTip{}, MTip{}, 0n, 0n, 0n, 0n}

# Lexicographic comparison; the second comparison only matters on a tie.
def Cmp.then(first: Cmp, second: Cmp) -> Cmp:
  match first:
    case EQ{}:
      second
    case LT{}:
      LT{}
    case GT{}:
      GT{}

# Strings by code point, as Python orders them.
def Str.cmp(a: String, b: String) -> Cmp:
  match a b:
    case SNil{} SNil{}:
      EQ{}
    case SNil{} SCon{h, t}:
      LT{}
    case SCon{h, t} SNil{}:
      GT{}
    case SCon{Chr{x}, at} SCon{Chr{y}, bt}:
      Cmp.then(U32.cmp(x, y), Str.cmp(at, bt))

def Side.str(b: Bool) -> String:
  match b:
    case True{}:
      "1"
    case False{}:
      "0"

def Bool.or3(a: Bool, b: Bool) -> Bool:
  match a:
    case True{}:
      True{}
    case False{}:
      b

# Lists of strings, lexicographically, as Python orders tuples of strings.
def Lst.cmp(a: List<&2, String>, b: List<&2, String>) -> Cmp:
  match a b:
    case Nil{} Nil{}:
      EQ{}
    case Nil{} Con{h, t}:
      LT{}
    case Con{h, t} Nil{}:
      GT{}
    case Con{x, at} Con{y, bt}:
      Cmp.then(Str.cmp(x, y), Lst.cmp(at, bt))

# The lesser of two values under a comparison: b when a compares greater, else a. For a total
# order this is commutative, associative and idempotent (LAWS.bend).
def Min.pick(-A: Data, later: Bool, a: A, b: A) -> A:
  match later:
    case True{}:
      b
    case False{}:
      a

def Lst.min(+a: List<&2, String>, +b: List<&2, String>) -> List<&2, String>:
  Min.pick(List<&2, String>, Cmp.is_gt(Lst.cmp(a, b)), a, b)

def Group.merge(a: Group, b: Group) -> Group:
  match a b:
    case Group{ai, ao, ar, aw, a1, a5, as, af, afa, ag, ax} Group{bi, bo, br, bw, b1, b5, bs, bf, bfa, bg, bx}:
      Group{Nat.max(ai, bi), Nat.max(ao, bo), Nat.max(ar, br), Nat.max(aw, bw), Nat.max(a1, b1), Nat.max(a5, b5), Nat.max(as, bs), Nat.max(af, bf), Bool.or3(afa, bfa), Bool.or3(ag, bg), Lst.min(ax, bx)}

def String.min(+a: String, +b: String) -> String:
  Min.pick(String, Cmp.is_gt(Str.cmp(a, b)), a, b)

# Insert-or-merge. Using the new value as the lookup default is sound because merge is
# idempotent: merging a value with itself returns it unchanged.
def Groups.put.fin(key: String, g: Group, got: Map<&2, Group> & Group) -> Map<&2, Group>:
  (m2, old) = got
  Map.set(&2, Group, m2, key, Group.merge(old, g))

def Groups.put(m: Map<&2, Group>, +key: String, +g: Group) -> Map<&2, Group>:
  Groups.put.fin(key, g, Map.get(Group, g, m, key))

def Tools.put.fin(key: String, t: List<&2, String>, got: Map<&2, List<&2, String>> & List<&2, String>) -> Map<&2, List<&2, String>>:
  (m2, old) = got
  Map.set(&2, List<&2, String>, m2, key, Lst.min(old, t))

def Tools.put(m: Map<&2, List<&2, String>>, +key: String, +t: List<&2, String>) -> Map<&2, List<&2, String>>:
  Tools.put.fin(key, t, Map.get(List<&2, String>, t, m, key))

def Prompts.put.fin(key: String, ts: String, got: Map<&2, String> & String) -> Map<&2, String>:
  (m2, old) = got
  Map.set(&2, String, m2, key, String.min(old, ts))

def Prompts.put(m: Map<&2, String>, +key: String, +ts: String) -> Map<&2, String>:
  Prompts.put.fin(key, ts, Map.get(String, ts, m, key))

def Tools.put_all(tools: List<&2, T.Tool>, +ts: String, +mid: String, m: Map<&2, List<&2, String>>) -> Map<&2, List<&2, String>>:
  match tools:
    case Nil{}:
      m
    case Con{T.Tool{id, name}, rest}:
      Tools.put_all(rest, ts, mid, Tools.put(m, id, [ts, name, mid]))

def Line.event.assistant(has_usage: Bool, has_mid: Bool, mid: String, ts: String, g: Group, tools: List<&2, T.Tool>) -> Ev:
  match has_usage has_mid:
    case True{} True{}:
      EAssist{mid, ts, g, tools}
    case _ _:
      ENone{}

def Line.event.user(has_uuid: Bool, meta: Bool, side: Bool, human: Bool, uuid: String, ts: String) -> Ev:
  match has_uuid meta side human:
    case True{} False{} False{} True{}:
      EUser{uuid, ts}
    case _ _ _ _:
      ENone{}

def Line.event.kind(kind: T.Rec, has_usage: Bool, has_mid: Bool, mid: String, g: Group, tools: List<&2, T.Tool>, has_uuid: Bool, meta: Bool, side: Bool, human: Bool, uuid: String, ts: String) -> Ev:
  match kind:
    case T.KAssistant{}:
      Line.event.assistant(has_usage, has_mid, mid, ts, g, tools)
    case T.KUser{}:
      Line.event.user(has_uuid, meta, side, human, uuid, ts)
    case T.KOther{}:
      ENone{}

def Line.event.ts(has_ts: Bool, kind: T.Rec, has_usage: Bool, has_mid: Bool, mid: String, g: Group, tools: List<&2, T.Tool>, has_uuid: Bool, meta: Bool, side: Bool, human: Bool, uuid: String, ts: String) -> Ev:
  match has_ts:
    case True{}:
      Line.event.kind(kind, has_usage, has_mid, mid, g, tools, has_uuid, meta, side, human, uuid, ts)
    case False{}:
      ENone{}

# What one well-formed line contributes: a response (assistant line with a usage object and a
# message id), a human prompt (user line, not meta, not a sidechain, with human text), or
# nothing. Lines without a string timestamp contribute nothing.
def Line.event(l: L.Line) -> Ev:
  match l:
    case L.Line{kind, has_ts, +ts, session, cwd, +side, meta, has_uuid, uuid, has_mid, mid, model, has_usage, inp, out, cread, cwrite, c1h, c5m, ws, wf, fast, geo, human, tools, b_type, b_has_id, b_id, b_name, b_human}:
      Line.event.ts(has_ts, kind, has_usage, has_mid, mid, Group{inp, out, cread, cwrite, c1h, c5m, ws, wf, fast, geo, [ts, session, cwd, model, Side.str(side)]}, tools, has_uuid, meta, side, human, uuid, ts)

def Fold.event(e: Ev, +f: Fold) -> Fold:
  match e f:
    case ENone{} f:
      f
    case EAssist{+mid, ts, g, tools} Fold{groups, tmap, prompts, lines, bad, usage, files}:
      Fold{Groups.put(groups, mid, g), Tools.put_all(tools, ts, mid, tmap), prompts, lines, bad, (usage + 1n : Nat), files}
    case EUser{uuid, ts} Fold{groups, tmap, prompts, lines, bad, usage, files}:
      Fold{groups, tmap, Prompts.put(prompts, uuid, ts), lines, bad, usage, files}

def Fold.count(ok: Bool, f: Fold) -> Fold:
  match ok f:
    case True{} Fold{groups, tmap, prompts, lines, bad, usage, files}:
      Fold{groups, tmap, prompts, (lines + 1n : Nat), bad, usage, files}
    case False{} Fold{groups, tmap, prompts, lines, bad, usage, files}:
      Fold{groups, tmap, prompts, (lines + 1n : Nat), (bad + 1n : Nat), usage, files}

def Fold.line.if(ok: Bool, l: L.Line, f: Fold) -> Fold:
  match ok:
    case True{}:
      Fold.event(Line.event(l), Fold.count(True{}, f))
    case False{}:
      Fold.count(False{}, f)

# Every complete line is counted; a line that is not a JSON object is counted as malformed.
def Fold.line(ok: Bool, l: L.Line, f: Fold) -> Fold:
  Fold.line.if(ok, l, f)

def Fold.file(f: Fold) -> Fold:
  match f:
    case Fold{groups, tmap, prompts, lines, bad, usage, files}:
      Fold{groups, tmap, prompts, lines, bad, usage, (files + 1n : Nat)}

def Groups.put_list(xs: List<&2, Sigma<&2, &2, String, _ => Group>>, m: Map<&2, Group>) -> Map<&2, Group>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{k, g}, rest}:
      Groups.put_list(rest, Groups.put(m, k, g))

def Tools.put_list(xs: List<&2, Sigma<&2, &2, String, _ => List<&2, String>>>, m: Map<&2, List<&2, String>>) -> Map<&2, List<&2, String>>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{k, t}, rest}:
      Tools.put_list(rest, Tools.put(m, k, t))

def Prompts.put_list(xs: List<&2, Sigma<&2, &2, String, _ => String>>, m: Map<&2, String>) -> Map<&2, String>:
  match xs:
    case Nil{}:
      m
    case Con{Tuple{k, ts}, rest}:
      Prompts.put_list(rest, Prompts.put(m, k, ts))

# Two folds of disjoint inputs combine into the fold of both. Every entry of b is merged into
# a exactly as if its lines had been read after a's, so, with merge being commutative,
# associative and idempotent, how the inputs were split and in what order they are combined
# does not matter. The line counters simply add.
def Fold.merge(a: Fold, b: Fold) -> Fold:
  match a b:
    case Fold{g1, t1, p1, l1, b1, u1, f1} Fold{g2, t2, p2, l2, b2, u2, f2}:
      Fold{Groups.put_list(Map.to_list(&2, Group, g2), g1), Tools.put_list(Map.to_list(&2, List<&2, String>, t2), t1), Prompts.put_list(Map.to_list(&2, String, p2), p1), (l1 + l2 : Nat), (b1 + b2 : Nat), (u1 + u2 : Nat), (f1 + f2 : Nat)}
