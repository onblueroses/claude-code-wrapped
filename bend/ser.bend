import Base
import ./fold.bend as F

# A fold as text, for shard processes to hand to the parent. One record per line, fields
# separated by tabs; backslash, tab and newline inside a field are escaped. The parent merges
# every shard's records into one fold (Fold.merge semantics), so a response read by two shards
# is still counted once.
#
#   S lines malformed usage_lines files
#   G mid inp out cread cwrite c1h c5m ws wf fast geo ts session cwd model side
#   T id ts name mid
#   P uuid ts

def Unesc.pick(hit: Bool, a: U32, b: U32) -> U32:
  match hit:
    case True{}:
      a
    case False{}:
      b

def Esc.hex(+n: U32) -> Char:
  Chr{Unesc.pick(U32.is_lt(n, 10), (n + 48 : U32), (n + 87 : U32))}

# A lone UTF-16 surrogate has no UTF-8 encoding, so it crosses as \uXXXX.
def Esc.surrogate(+c: U32, acc: String) -> String:
  SCon{Esc.hex(U32.mod(c, 16)), SCon{Esc.hex(U32.mod(U32.div(c, 16), 16)), SCon{Esc.hex(U32.mod(U32.div(c, 256), 16)), SCon{Esc.hex(U32.div(c, 4096)), SCon{Chr{117}, SCon{Chr{92}, acc}}}}}}

def Esc.char(bs: Bool, tab: Bool, nl: Bool, sur: Bool, c: U32, acc: String) -> String:
  match bs tab nl sur:
    case True{} _ _ _:
      SCon{Chr{92}, SCon{Chr{92}, acc}}
    case False{} True{} _ _:
      SCon{Chr{116}, SCon{Chr{92}, acc}}
    case False{} False{} True{} _:
      SCon{Chr{110}, SCon{Chr{92}, acc}}
    case False{} False{} False{} True{}:
      Esc.surrogate(c, acc)
    case False{} False{} False{} False{}:
      SCon{Chr{c}, acc}

def Esc.go(s: String, acc: String) -> String:
  match s:
    case SNil{}:
      String.reverse(acc)
    case SCon{Chr{+c}, t}:
      Esc.go(t, Esc.char(U32.is_eq(c, 92), U32.is_eq(c, 9), U32.is_eq(c, 10), U32.is_ge(c, 55296) && U32.is_le(c, 57343), c, acc))

def Esc.str(s: String) -> String:
  Esc.go(s, "")

def Unesc.code(+e: U32) -> U32:
  Unesc.pick(U32.is_eq(e, 116), 9, Unesc.pick(U32.is_eq(e, 110), 10, e))

def Unesc.hexval(+c: U32) -> U32:
  Unesc.pick(U32.is_le(c, 57), (c - 48 : U32), (c - 87 : U32))

@unsafe def Unesc.go(s: String, acc: String) -> String:
  match s:
    case SNil{}:
      String.reverse(acc)
    case SCon{Chr{+c}, t}:
      Unesc.step(U32.is_eq(c, 92), c, t, acc)

@unsafe def Unesc.step(bs: Bool, c: U32, +t: String, acc: String) -> String:
  match bs t:
    case True{} SCon{Chr{+e}, r}:
      Unesc.esc(U32.is_eq(e, 117), e, r, acc)
    case True{} SNil{}:
      String.reverse(acc)
    case False{} t:
      Unesc.go(t, SCon{Chr{c}, acc})

@unsafe def Unesc.esc(u: Bool, e: U32, +r: String, acc: String) -> String:
  match u r:
    case True{} SCon{Chr{a}, SCon{Chr{b}, SCon{Chr{c}, SCon{Chr{d}, rest}}}}:
      Unesc.go(rest, SCon{Chr{(Unesc.hexval(a) * 4096 + Unesc.hexval(b) * 256 + Unesc.hexval(c) * 16 + Unesc.hexval(d) : U32)}, acc})
    case True{} r:
      String.reverse(acc)
    case False{} r:
      Unesc.go(r, SCon{Chr{Unesc.code(e)}, acc})

@unsafe def Unesc.str(s: String) -> String:
  Unesc.go(s, "")

def Ser.bool(b: Bool) -> String:
  match b:
    case True{}:
      "1"
    case False{}:
      "0"

def Ser.tab(xs: List<&2, String>) -> String:
  String.join(xs, "\t")

def Ser.strs(xs: List<&2, String>) -> List<&2, String>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{x, rest}:
      Con{Esc.str(x), Ser.strs(rest)}

def Ser.group(k: String, g: F.Group) -> String:
  match g:
    case F.Group{inp, out, cread, cwrite, c1h, c5m, ws, wf, fast, geo, origin}:
      Ser.tab(Con{"G", Con{Esc.str(k), Con{Nat.show(inp), Con{Nat.show(out), Con{Nat.show(cread), Con{Nat.show(cwrite), Con{Nat.show(c1h), Con{Nat.show(c5m), Con{Nat.show(ws), Con{Nat.show(wf), Con{Ser.bool(fast), Con{Ser.bool(geo), Ser.strs(origin)}}}}}}}}}}}})

def Ser.tool(k: String, t: List<&2, String>) -> String:
  Ser.tab(Con{"T", Con{Esc.str(k), Ser.strs(t)}})

def Ser.prompt(k: String, ts: String) -> String:
  Ser.tab(["P", Esc.str(k), Esc.str(ts)])

def Ser.groups(xs: List<&2, Sigma<&2, &2, String, _ => F.Group>>, acc: List<&2, String>) -> List<&2, String>:
  match xs:
    case Nil{}:
      acc
    case Con{Tuple{k, g}, rest}:
      Ser.groups(rest, Con{Ser.group(k, g), acc})

def Ser.tools(xs: List<&2, Sigma<&2, &2, String, _ => List<&2, String>>>, acc: List<&2, String>) -> List<&2, String>:
  match xs:
    case Nil{}:
      acc
    case Con{Tuple{k, t}, rest}:
      Ser.tools(rest, Con{Ser.tool(k, t), acc})

def Ser.prompts(xs: List<&2, Sigma<&2, &2, String, _ => String>>, acc: List<&2, String>) -> List<&2, String>:
  match xs:
    case Nil{}:
      acc
    case Con{Tuple{k, ts}, rest}:
      Ser.prompts(rest, Con{Ser.prompt(k, ts), acc})

def Ser.fold(f: F.Fold) -> String:
  match f:
    case F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      String.join(Con{Ser.tab(["S", Nat.show(lines), Nat.show(bad), Nat.show(usage), Nat.show(files)]), Ser.groups(Map.to_list(&2, F.Group, groups), Ser.tools(Map.to_list(&2, List<&2, String>, tools), Ser.prompts(Map.to_list(&2, String, prompts), Nil{})))}, "\n")

# Parsing
# -------

def Par.nat.go(m: Maybe<&2, Nat>) -> Nat:
  match m:
    case Some{v}:
      v
    case None{}:
      0n

def Par.nat(s: String) -> Nat:
  Par.nat.go(Nat.read(s))

def Par.bool(s: String) -> Bool:
  String.eq(s, "1")

@unsafe def Des.group(xs: List<&2, String>, +f: F.Fold) -> F.Fold:
  match xs f:
    case [k, inp, out, cread, cwrite, c1h, c5m, ws, wf, fast, geo, ts, session, cwd, model, side] F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      F.Fold{F.Groups.put(groups, Unesc.str(k), F.Group{Par.nat(inp), Par.nat(out), Par.nat(cread), Par.nat(cwrite), Par.nat(c1h), Par.nat(c5m), Par.nat(ws), Par.nat(wf), Par.bool(fast), Par.bool(geo), [Unesc.str(ts), Unesc.str(session), Unesc.str(cwd), Unesc.str(model), Unesc.str(side)]}), tools, prompts, lines, bad, usage, files}
    case _ f:
      f

@unsafe def Des.tool(xs: List<&2, String>, +f: F.Fold) -> F.Fold:
  match xs f:
    case [k, ts, name, mid] F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      F.Fold{groups, F.Tools.put(tools, Unesc.str(k), [Unesc.str(ts), Unesc.str(name), Unesc.str(mid)]), prompts, lines, bad, usage, files}
    case _ f:
      f

@unsafe def Des.prompt(xs: List<&2, String>, +f: F.Fold) -> F.Fold:
  match xs f:
    case [k, ts] F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      F.Fold{groups, tools, F.Prompts.put(prompts, Unesc.str(k), Unesc.str(ts)), lines, bad, usage, files}
    case _ f:
      f

def Des.stats(xs: List<&2, String>, +f: F.Fold) -> F.Fold:
  match xs f:
    case [l, b, u, fi] F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      F.Fold{groups, tools, prompts, (lines + Par.nat(l) : Nat), (bad + Par.nat(b) : Nat), (usage + Par.nat(u) : Nat), (files + Par.nat(fi) : Nat)}
    case _ f:
      f

@unsafe def Des.tagged(g: Bool, t: Bool, p: Bool, s: Bool, rest: List<&2, String>, f: F.Fold) -> F.Fold:
  match g t p s:
    case True{} _ _ _:
      Des.group(rest, f)
    case False{} True{} _ _:
      Des.tool(rest, f)
    case False{} False{} True{} _:
      Des.prompt(rest, f)
    case False{} False{} False{} True{}:
      Des.stats(rest, f)
    case False{} False{} False{} False{}:
      f

@unsafe def Des.record(fields: List<&2, String>, f: F.Fold) -> F.Fold:
  match fields:
    case Con{+tag, rest}:
      Des.tagged(String.eq(tag, "G"), String.eq(tag, "T"), String.eq(tag, "P"), String.eq(tag, "S"), rest, f)
    case Nil{}:
      f

@unsafe def Des.lines(xs: List<&2, String>, f: F.Fold) -> F.Fold:
  match xs:
    case Nil{}:
      f
    case Con{line, rest}:
      Des.lines(rest, Des.record(String.split(line, '\t'), f))

# Merges a serialized fold into f.
@unsafe def Des.fold(text: String, f: F.Fold) -> F.Fold:
  Des.lines(String.split(text, '\n'), f)
