# Proofs of LAWS.bend. The lemmas build up from bits: Bool.cmp and Word.cmp (the order of a
# U32) are total orders, hence so are Str.cmp and Lst.cmp (lexicographic); the minimum under
# any total order is commutative, associative and idempotent; so are Nat.max and or; and a
# response record merges field by field with those. Run `bend PROOF.bend --verdict`.

import Base
import ./fold.bend as F
import ./bill.bend as R
import ./LAWS.bend as Laws

# Contradictions between distinct Cmp values.
def Cmp.isEQ(c: Cmp) -> Type:
  match c:
    case LT{}:
      Empty
    case EQ{}:
      Unit
    case GT{}:
      Empty

def Cmp.isLT(c: Cmp) -> Type:
  match c:
    case LT{}:
      Unit
    case EQ{}:
      Empty
    case GT{}:
      Empty

law lt_not_eq:
  for e: {LT{} == EQ{} : Cmp}
  Empty

def lt_not_eq(e):
  %e : Cmp.isLT(_)
  Unit{}

def absurd(-A: Type, x: Empty) -> A:
  match x:

def Cmp.flip(c: Cmp) -> Cmp:
  match c:
    case LT{}:
      GT{}
    case EQ{}:
      EQ{}
    case GT{}:
      LT{}

law bool_cmp_anti:
  for a: Bool
  for b: Bool
  {Bool.cmp(a, b) == Cmp.flip(Bool.cmp(b, a)) : Cmp}

def bool_cmp_anti(a, b):
  match a b:
    case False{} False{}:
      {==}
    case False{} True{}:
      {==}
    case True{} False{}:
      {==}
    case True{} True{}:
      {==}

law fin_anti:
  for ab: Bool
  for bb: Bool
  for t: Cmp
  {Word.cmp.fin(ab, bb, Cmp.flip(t)) == Cmp.flip(Word.cmp.fin(bb, ab, t)) : Cmp}

def fin_anti(ab, bb, t):
  match t:
    case LT{}:
      {==}
    case EQ{}:
      bool_cmp_anti(ab, bb)
    case GT{}:
      {==}

law word_cmp_anti:
  for n: Nat
  for a: Word(n)
  for b: Word(n)
  {Word.cmp(n, a, b) == Cmp.flip(Word.cmp(n, b, a)) : Cmp}

def word_cmp_anti(n, a, b):
  match n:
    case 0n:
      {==}
    case 1n+(+p):
      match a b:
        case WCon{+ab, +at} WCon{+bb, +bt}:
          %Equal.sym(Cmp, Word.cmp(p, at, bt), Cmp.flip(Word.cmp(p, bt, at)), word_cmp_anti(p, at, bt)) : {Word.cmp.fin(ab, bb, _) == Cmp.flip(Word.cmp.fin(bb, ab, Word.cmp(p, bt, at))) : Cmp}
          fin_anti(ab, bb, Word.cmp(p, bt, at))

law bool_cmp_refl:
  for a: Bool
  {Bool.cmp(a, a) == EQ{} : Cmp}

def bool_cmp_refl(a):
  match a:
    case False{}:
      {==}
    case True{}:
      {==}

law word_cmp_refl:
  for n: Nat
  for a: Word(n)
  {Word.cmp(n, a, a) == EQ{} : Cmp}

def word_cmp_refl(n, a):
  match n:
    case 0n:
      {==}
    case 1n+(+p):
      match a:
        case WCon{+ab, +at}:
          %Equal.sym(Cmp, Word.cmp(p, at, at), EQ{}, word_cmp_refl(p, at)) : {Word.cmp.fin(ab, ab, _) == EQ{} : Cmp}
          bool_cmp_refl(ab)

def Cmp.isGT(c: Cmp) -> Type:
  match c:
    case LT{}:
      Empty
    case EQ{}:
      Empty
    case GT{}:
      Unit

law gt_not_eq:
  for e: {GT{} == EQ{} : Cmp}
  Empty

def gt_not_eq(e):
  %e : Cmp.isGT(_)
  Unit{}

law bool_cmp_eq:
  for a: Bool
  for b: Bool
  for e: {Bool.cmp(a, b) == EQ{} : Cmp}
  {a == b : Bool}

def bool_cmp_eq(a, b, e):
  match a b:
    case False{} False{}:
      {==}
    case False{} True{}:
      absurd({False{} == True{} : Bool}, lt_not_eq(e))
    case True{} False{}:
      absurd({True{} == False{} : Bool}, gt_not_eq(e))
    case True{} True{}:
      {==}

law fin_eq_tail:
  for ab: Bool
  for bb: Bool
  for t: Cmp
  for e: {Word.cmp.fin(ab, bb, t) == EQ{} : Cmp}
  {t == EQ{} : Cmp}

def fin_eq_tail(ab, bb, t, e):
  match t:
    case LT{}:
      absurd({LT{} == EQ{} : Cmp}, lt_not_eq(e))
    case EQ{}:
      {==}
    case GT{}:
      absurd({GT{} == EQ{} : Cmp}, gt_not_eq(e))

law fin_eq_head:
  for ab: Bool
  for bb: Bool
  for t: Cmp
  for e: {Word.cmp.fin(ab, bb, t) == EQ{} : Cmp}
  {Bool.cmp(ab, bb) == EQ{} : Cmp}

def fin_eq_head(ab, bb, t, e):
  match t:
    case LT{}:
      absurd({Bool.cmp(ab, bb) == EQ{} : Cmp}, lt_not_eq(e))
    case EQ{}:
      e
    case GT{}:
      absurd({Bool.cmp(ab, bb) == EQ{} : Cmp}, gt_not_eq(e))

law word_cmp_eq:
  for n: Nat
  for a: Word(n)
  for b: Word(n)
  for e: {Word.cmp(n, a, b) == EQ{} : Cmp}
  {a == b : Word(n)}

def word_cmp_eq(n, a, b, e):
  match n:
    case 0n:
      match a b:
        case WNil{} WNil{}:
          {==}
    case 1n+(+p):
      match a b:
        case WCon{+ab, +at} WCon{+bb, +bt}:
          +he = e
          %Equal.sym(Bool, ab, bb, bool_cmp_eq(ab, bb, fin_eq_head(ab, bb, Word.cmp(p, at, bt), he))) : {WCon{_, at} == WCon{bb, bt} : Word(1n+p)}
          %Equal.sym(Word(p), at, bt, word_cmp_eq(p, at, bt, fin_eq_tail(ab, bb, Word.cmp(p, at, bt), he))) : {WCon{bb, _} == WCon{bb, bt} : Word(1n+p)}
          {==}

law lt_not_gt:
  for e: {LT{} == GT{} : Cmp}
  Empty

def lt_not_gt(e):
  %e : Cmp.isLT(_)
  Unit{}

law gt_not_lt:
  for e: {GT{} == LT{} : Cmp}
  Empty

def gt_not_lt(e):
  %e : Cmp.isGT(_)
  Unit{}

law eq_not_lt:
  for e: {EQ{} == LT{} : Cmp}
  Empty

def eq_not_lt(e):
  %e : Cmp.isEQ(_)
  Unit{}

law bool_lt_chain.t:
  for b: Bool
  for e: {Bool.cmp(True{}, b) == LT{} : Cmp}
  Empty

def bool_lt_chain.t(b, e):
  match b:
    case False{}:
      gt_not_lt(e)
    case True{}:
      eq_not_lt(e)

# Bool.cmp(a, b) = LT and Bool.cmp(b, c) = LT cannot both hold.
law bool_lt_chain:
  for a: Bool
  for b: Bool
  for c: Bool
  for e1: {Bool.cmp(a, b) == LT{} : Cmp}
  for e2: {Bool.cmp(b, c) == LT{} : Cmp}
  Empty

def bool_lt_chain(a, b, c, e1, e2):
  match a b c:
    case False{} False{} _:
      eq_not_lt(e1)
    case True{} _ _:
      bool_lt_chain.t(b, e1)
    case False{} True{} False{}:
      gt_not_lt(e2)
    case False{} True{} True{}:
      eq_not_lt(e2)

law word_trans.step:
  for p: Nat
  for ab: Bool
  for bb: Bool
  for cb: Bool
  for +at: Word(p)
  for +bt: Word(p)
  for +ct: Word(p)
  for t1: Cmp
  for h1: {Word.cmp(p, at, bt) == t1 : Cmp}
  for t2: Cmp
  for h2: {Word.cmp(p, bt, ct) == t2 : Cmp}
  for e1: {Word.cmp.fin(ab, bb, t1) == LT{} : Cmp}
  for e2: {Word.cmp.fin(bb, cb, t2) == LT{} : Cmp}
  for ih: @x: Word(p) -> @y: Word(p) -> @z: Word(p) -> {Word.cmp(p, x, y) == LT{} : Cmp} -> {Word.cmp(p, y, z) == LT{} : Cmp} -> {Word.cmp(p, x, z) == LT{} : Cmp}
  {Word.cmp.fin(ab, cb, Word.cmp(p, at, ct)) == LT{} : Cmp}

def word_trans.step(p, ab, bb, cb, at, bt, ct, t1, h1, t2, h2, e1, e2, ih):
  match t1 t2:
    case LT{} LT{}:
      %Equal.sym(Cmp, Word.cmp(p, at, ct), LT{}, ih(at, bt, ct, h1, h2)) : {Word.cmp.fin(ab, cb, _) == LT{} : Cmp}
      {==}
    case LT{} EQ{}:
      %word_cmp_eq(p, bt, ct, h2) : {Word.cmp.fin(ab, cb, Word.cmp(p, at, _)) == LT{} : Cmp}
      %Equal.sym(Cmp, Word.cmp(p, at, bt), LT{}, h1) : {Word.cmp.fin(ab, cb, _) == LT{} : Cmp}
      {==}
    case EQ{} LT{}:
      %Equal.sym(Word(p), at, bt, word_cmp_eq(p, at, bt, h1)) : {Word.cmp.fin(ab, cb, Word.cmp(p, _, ct)) == LT{} : Cmp}
      %Equal.sym(Cmp, Word.cmp(p, bt, ct), LT{}, h2) : {Word.cmp.fin(ab, cb, _) == LT{} : Cmp}
      {==}
    case EQ{} EQ{}:
      absurd({Word.cmp.fin(ab, cb, Word.cmp(p, at, ct)) == LT{} : Cmp}, bool_lt_chain(ab, bb, cb, e1, e2))
    case GT{} _:
      absurd({Word.cmp.fin(ab, cb, Word.cmp(p, at, ct)) == LT{} : Cmp}, gt_not_lt(e1))
    case LT{} GT{}:
      absurd({Word.cmp.fin(ab, cb, Word.cmp(p, at, ct)) == LT{} : Cmp}, gt_not_lt(e2))
    case EQ{} GT{}:
      absurd({Word.cmp.fin(ab, cb, Word.cmp(p, at, ct)) == LT{} : Cmp}, gt_not_lt(e2))

law word_trans:
  for n: Nat
  for a: Word(n)
  for b: Word(n)
  for c: Word(n)
  for e1: {Word.cmp(n, a, b) == LT{} : Cmp}
  for e2: {Word.cmp(n, b, c) == LT{} : Cmp}
  {Word.cmp(n, a, c) == LT{} : Cmp}

def word_trans(n, a, b, c, e1, e2):
  match n:
    case 0n:
      absurd({Word.cmp(0n, a, c) == LT{} : Cmp}, eq_not_lt(e1))
    case 1n+(+p):
      match a b c:
        case WCon{ab, +at} WCon{bb, +bt} WCon{cb, +ct}:
          word_trans.step(p, ab, bb, cb, at, bt, ct, Word.cmp(p, at, bt), {==}, Word.cmp(p, bt, ct), {==}, e1, e2, x => y => z => f => g => word_trans(p, x, y, z, f, g))

# Cmp values
# ----------

law flip_flip:
  for c: Cmp
  {Cmp.flip(Cmp.flip(c)) == c : Cmp}

def flip_flip(c):
  match c:
    case LT{}:
      {==}
    case EQ{}:
      {==}
    case GT{}:
      {==}

law then_flip:
  for x: Cmp
  for y: Cmp
  {F.Cmp.then(Cmp.flip(x), Cmp.flip(y)) == Cmp.flip(F.Cmp.then(x, y)) : Cmp}

def then_flip(x, y):
  match x:
    case LT{}:
      {==}
    case EQ{}:
      {==}
    case GT{}:
      {==}

law then_eq_l:
  for x: Cmp
  for y: Cmp
  for e: {F.Cmp.then(x, y) == EQ{} : Cmp}
  {x == EQ{} : Cmp}

def then_eq_l(x, y, e):
  match x:
    case LT{}:
      absurd({LT{} == EQ{} : Cmp}, lt_not_eq(e))
    case EQ{}:
      {==}
    case GT{}:
      absurd({GT{} == EQ{} : Cmp}, gt_not_eq(e))

law then_eq_r:
  for x: Cmp
  for y: Cmp
  for e: {F.Cmp.then(x, y) == EQ{} : Cmp}
  {y == EQ{} : Cmp}

def then_eq_r(x, y, e):
  match x:
    case LT{}:
      absurd({y == EQ{} : Cmp}, lt_not_eq(e))
    case EQ{}:
      e
    case GT{}:
      absurd({y == EQ{} : Cmp}, gt_not_eq(e))

# U32
# ---

law u32_refl:
  for a: U32
  {U32.cmp(a, a) == EQ{} : Cmp}

def u32_refl(a):
  match a:
    case U32{+w}:
      word_cmp_refl(32n, w)

law u32_anti:
  for a: U32
  for b: U32
  {U32.cmp(a, b) == Cmp.flip(U32.cmp(b, a)) : Cmp}

def u32_anti(a, b):
  match a b:
    case U32{+x} U32{+y}:
      word_cmp_anti(32n, x, y)

law u32_eq:
  for a: U32
  for b: U32
  for e: {U32.cmp(a, b) == EQ{} : Cmp}
  {a == b : U32}

def u32_eq(a, b, e):
  match a b:
    case U32{+x} U32{+y}:
      %Equal.sym(Word(32n), x, y, word_cmp_eq(32n, x, y, e)) : {U32{_} == U32{y} : U32}
      {==}

law u32_trans:
  for a: U32
  for b: U32
  for c: U32
  for e1: {U32.cmp(a, b) == LT{} : Cmp}
  for e2: {U32.cmp(b, c) == LT{} : Cmp}
  {U32.cmp(a, c) == LT{} : Cmp}

def u32_trans(a, b, c, e1, e2):
  match a b c:
    case U32{+x} U32{+y} U32{+z}:
      word_trans(32n, x, y, z, e1, e2)

# Strings
# -------

law str_refl:
  for a: String
  {F.Str.cmp(a, a) == EQ{} : Cmp}

def str_refl(a):
  match a:
    case SNil{}:
      {==}
    case SCon{Chr{+x}, +t}:
      %Equal.sym(Cmp, U32.cmp(x, x), EQ{}, u32_refl(x)) : {F.Cmp.then(_, F.Str.cmp(t, t)) == EQ{} : Cmp}
      str_refl(t)

law str_anti:
  for a: String
  for b: String
  {F.Str.cmp(a, b) == Cmp.flip(F.Str.cmp(b, a)) : Cmp}

def str_anti(a, b):
  match a b:
    case SNil{} SNil{}:
      {==}
    case SNil{} SCon{Chr{x}, t}:
      {==}
    case SCon{Chr{x}, t} SNil{}:
      {==}
    case SCon{Chr{+x}, +at} SCon{Chr{+y}, +bt}:
      %Equal.sym(Cmp, U32.cmp(x, y), Cmp.flip(U32.cmp(y, x)), u32_anti(x, y)) : {F.Cmp.then(_, F.Str.cmp(at, bt)) == Cmp.flip(F.Cmp.then(U32.cmp(y, x), F.Str.cmp(bt, at))) : Cmp}
      %Equal.sym(Cmp, F.Str.cmp(at, bt), Cmp.flip(F.Str.cmp(bt, at)), str_anti(at, bt)) : {F.Cmp.then(Cmp.flip(U32.cmp(y, x)), _) == Cmp.flip(F.Cmp.then(U32.cmp(y, x), F.Str.cmp(bt, at))) : Cmp}
      then_flip(U32.cmp(y, x), F.Str.cmp(bt, at))

law str_eq:
  for a: String
  for b: String
  for e: {F.Str.cmp(a, b) == EQ{} : Cmp}
  {a == b : String}

def str_eq(a, b, e):
  match a b:
    case SNil{} SNil{}:
      {==}
    case SNil{} SCon{Chr{x}, t}:
      absurd({"" == SCon{Chr{x}, t} : String}, lt_not_eq(e))
    case SCon{Chr{x}, t} SNil{}:
      absurd({SCon{Chr{x}, t} == "" : String}, gt_not_eq(e))
    case SCon{Chr{+x}, +at} SCon{Chr{+y}, +bt}:
      +he = e
      %Equal.sym(U32, x, y, u32_eq(x, y, then_eq_l(U32.cmp(x, y), F.Str.cmp(at, bt), he))) : {SCon{Chr{_}, at} == SCon{Chr{y}, bt} : String}
      %Equal.sym(String, at, bt, str_eq(at, bt, then_eq_r(U32.cmp(x, y), F.Str.cmp(at, bt), he))) : {SCon{Chr{y}, _} == SCon{Chr{y}, bt} : String}
      {==}

law str_trans.step:
  for +x: U32
  for +y: U32
  for +z: U32
  for +at: String
  for +bt: String
  for +ct: String
  for t1: Cmp
  for h1: {U32.cmp(x, y) == t1 : Cmp}
  for t2: Cmp
  for h2: {U32.cmp(y, z) == t2 : Cmp}
  for e1: {F.Cmp.then(t1, F.Str.cmp(at, bt)) == LT{} : Cmp}
  for e2: {F.Cmp.then(t2, F.Str.cmp(bt, ct)) == LT{} : Cmp}
  for ih: {F.Str.cmp(at, bt) == LT{} : Cmp} -> {F.Str.cmp(bt, ct) == LT{} : Cmp} -> {F.Str.cmp(at, ct) == LT{} : Cmp}
  {F.Cmp.then(U32.cmp(x, z), F.Str.cmp(at, ct)) == LT{} : Cmp}

def str_trans.step(x, y, z, at, bt, ct, t1, h1, t2, h2, e1, e2, ih):
  match t1 t2:
    case LT{} LT{}:
      %Equal.sym(Cmp, U32.cmp(x, z), LT{}, u32_trans(x, y, z, h1, h2)) : {F.Cmp.then(_, F.Str.cmp(at, ct)) == LT{} : Cmp}
      {==}
    case LT{} EQ{}:
      %u32_eq(y, z, h2) : {F.Cmp.then(U32.cmp(x, _), F.Str.cmp(at, ct)) == LT{} : Cmp}
      %Equal.sym(Cmp, U32.cmp(x, y), LT{}, h1) : {F.Cmp.then(_, F.Str.cmp(at, ct)) == LT{} : Cmp}
      {==}
    case EQ{} LT{}:
      %Equal.sym(U32, x, y, u32_eq(x, y, h1)) : {F.Cmp.then(U32.cmp(_, z), F.Str.cmp(at, ct)) == LT{} : Cmp}
      %Equal.sym(Cmp, U32.cmp(y, z), LT{}, h2) : {F.Cmp.then(_, F.Str.cmp(at, ct)) == LT{} : Cmp}
      {==}
    case EQ{} EQ{}:
      %Equal.sym(U32, x, y, u32_eq(x, y, h1)) : {F.Cmp.then(U32.cmp(_, z), F.Str.cmp(at, ct)) == LT{} : Cmp}
      %Equal.sym(Cmp, U32.cmp(y, z), EQ{}, h2) : {F.Cmp.then(_, F.Str.cmp(at, ct)) == LT{} : Cmp}
      ih(e1, e2)
    case GT{} _:
      absurd({F.Cmp.then(U32.cmp(x, z), F.Str.cmp(at, ct)) == LT{} : Cmp}, gt_not_lt(e1))
    case LT{} GT{}:
      absurd({F.Cmp.then(U32.cmp(x, z), F.Str.cmp(at, ct)) == LT{} : Cmp}, gt_not_lt(e2))
    case EQ{} GT{}:
      absurd({F.Cmp.then(U32.cmp(x, z), F.Str.cmp(at, ct)) == LT{} : Cmp}, gt_not_lt(e2))

law str_trans:
  for a: String
  for b: String
  for c: String
  for e1: {F.Str.cmp(a, b) == LT{} : Cmp}
  for e2: {F.Str.cmp(b, c) == LT{} : Cmp}
  {F.Str.cmp(a, c) == LT{} : Cmp}

def str_trans(a, b, c, e1, e2):
  match a b c:
    case SNil{} SNil{} c:
      absurd({F.Str.cmp("", c) == LT{} : Cmp}, eq_not_lt(e1))
    case SNil{} SCon{Chr{y}, bt} SNil{}:
      absurd({F.Str.cmp("", "") == LT{} : Cmp}, gt_not_lt(e2))
    case SNil{} SCon{Chr{y}, bt} SCon{Chr{z}, ct}:
      {==}
    case SCon{Chr{x}, at} SNil{} c:
      absurd({F.Str.cmp(SCon{Chr{x}, at}, c) == LT{} : Cmp}, gt_not_lt(e1))
    case SCon{Chr{x}, at} SCon{Chr{y}, bt} SNil{}:
      absurd({F.Str.cmp(SCon{Chr{x}, at}, "") == LT{} : Cmp}, gt_not_lt(e2))
    case SCon{Chr{+x}, +at} SCon{Chr{+y}, +bt} SCon{Chr{+z}, +ct}:
      str_trans.step(x, y, z, at, bt, ct, U32.cmp(x, y), {==}, U32.cmp(y, z), {==}, e1, e2, f => g => str_trans(at, bt, ct, f, g))

# The minimum under any total order
# ---------------------------------

law min_idem:
  for -A: Data
  for C: A -> A -> Cmp
  for refl: @x: A -> {C(x, x) == EQ{} : Cmp}
  for +a: A
  {F.Min.pick(A, Cmp.is_gt(C(a, a)), a, a) == a : A}

def min_idem(A, C, refl, a):
  %Equal.sym(Cmp, C(a, a), EQ{}, refl(a)) : {F.Min.pick(A, Cmp.is_gt(_), a, a) == a : A}
  {==}

law min_comm.go:
  for -A: Data
  for -C: A -> A -> Cmp
  for eq: @x: A -> @y: A -> {C(x, y) == EQ{} : Cmp} -> {x == y : A}
  for +a: A
  for +b: A
  for c: Cmp
  for h: {C(b, a) == c : Cmp}
  {F.Min.pick(A, Cmp.is_gt(Cmp.flip(c)), a, b) == F.Min.pick(A, Cmp.is_gt(c), b, a) : A}

def min_comm.go(A, C, eq, a, b, c, h):
  match c:
    case LT{}:
      {==}
    case EQ{}:
      %eq(b, a, h) : {_ == b : A}
      {==}
    case GT{}:
      {==}

law min_comm:
  for -A: Data
  for C: A -> A -> Cmp
  for anti: @x: A -> @y: A -> {C(x, y) == Cmp.flip(C(y, x)) : Cmp}
  for eq: @x: A -> @y: A -> {C(x, y) == EQ{} : Cmp} -> {x == y : A}
  for +a: A
  for +b: A
  {F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) == F.Min.pick(A, Cmp.is_gt(C(b, a)), b, a) : A}

def min_comm(A, C, anti, eq, a, b):
  %Equal.sym(Cmp, C(a, b), Cmp.flip(C(b, a)), anti(a, b)) : {F.Min.pick(A, Cmp.is_gt(_), a, b) == F.Min.pick(A, Cmp.is_gt(C(b, a)), b, a) : A}
  min_comm.go(A, C, eq, a, b, C(b, a), {==})

# Lists of strings
# ----------------

law lst_refl:
  for a: List<&2, String>
  {F.Lst.cmp(a, a) == EQ{} : Cmp}

def lst_refl(a):
  match a:
    case Nil{}:
      {==}
    case Con{+x, +t}:
      %Equal.sym(Cmp, F.Str.cmp(x, x), EQ{}, str_refl(x)) : {F.Cmp.then(_, F.Lst.cmp(t, t)) == EQ{} : Cmp}
      lst_refl(t)

law lst_anti:
  for a: List<&2, String>
  for b: List<&2, String>
  {F.Lst.cmp(a, b) == Cmp.flip(F.Lst.cmp(b, a)) : Cmp}

def lst_anti(a, b):
  match a b:
    case Nil{} Nil{}:
      {==}
    case Nil{} Con{x, t}:
      {==}
    case Con{x, t} Nil{}:
      {==}
    case Con{+x, +at} Con{+y, +bt}:
      %Equal.sym(Cmp, F.Str.cmp(x, y), Cmp.flip(F.Str.cmp(y, x)), str_anti(x, y)) : {F.Cmp.then(_, F.Lst.cmp(at, bt)) == Cmp.flip(F.Cmp.then(F.Str.cmp(y, x), F.Lst.cmp(bt, at))) : Cmp}
      %Equal.sym(Cmp, F.Lst.cmp(at, bt), Cmp.flip(F.Lst.cmp(bt, at)), lst_anti(at, bt)) : {F.Cmp.then(Cmp.flip(F.Str.cmp(y, x)), _) == Cmp.flip(F.Cmp.then(F.Str.cmp(y, x), F.Lst.cmp(bt, at))) : Cmp}
      then_flip(F.Str.cmp(y, x), F.Lst.cmp(bt, at))

law lst_eq:
  for a: List<&2, String>
  for b: List<&2, String>
  for e: {F.Lst.cmp(a, b) == EQ{} : Cmp}
  {a == b : List<&2, String>}

def lst_eq(a, b, e):
  match a b:
    case Nil{} Nil{}:
      {==}
    case Nil{} Con{x, t}:
      absurd({Nil{} == Con{x, t} : List<&2, String>}, lt_not_eq(e))
    case Con{x, t} Nil{}:
      absurd({Con{x, t} == Nil{} : List<&2, String>}, gt_not_eq(e))
    case Con{+x, +at} Con{+y, +bt}:
      +he = e
      %Equal.sym(String, x, y, str_eq(x, y, then_eq_l(F.Str.cmp(x, y), F.Lst.cmp(at, bt), he))) : {Con{_, at} == Con{y, bt} : List<&2, String>}
      %Equal.sym(List<&2, String>, at, bt, lst_eq(at, bt, then_eq_r(F.Str.cmp(x, y), F.Lst.cmp(at, bt), he))) : {Con{y, _} == Con{y, bt} : List<&2, String>}
      {==}

law lst_trans.step:
  for +x: String
  for +y: String
  for +z: String
  for +at: List<&2, String>
  for +bt: List<&2, String>
  for +ct: List<&2, String>
  for t1: Cmp
  for h1: {F.Str.cmp(x, y) == t1 : Cmp}
  for t2: Cmp
  for h2: {F.Str.cmp(y, z) == t2 : Cmp}
  for e1: {F.Cmp.then(t1, F.Lst.cmp(at, bt)) == LT{} : Cmp}
  for e2: {F.Cmp.then(t2, F.Lst.cmp(bt, ct)) == LT{} : Cmp}
  for ih: {F.Lst.cmp(at, bt) == LT{} : Cmp} -> {F.Lst.cmp(bt, ct) == LT{} : Cmp} -> {F.Lst.cmp(at, ct) == LT{} : Cmp}
  {F.Cmp.then(F.Str.cmp(x, z), F.Lst.cmp(at, ct)) == LT{} : Cmp}

def lst_trans.step(x, y, z, at, bt, ct, t1, h1, t2, h2, e1, e2, ih):
  match t1 t2:
    case LT{} LT{}:
      %Equal.sym(Cmp, F.Str.cmp(x, z), LT{}, str_trans(x, y, z, h1, h2)) : {F.Cmp.then(_, F.Lst.cmp(at, ct)) == LT{} : Cmp}
      {==}
    case LT{} EQ{}:
      %str_eq(y, z, h2) : {F.Cmp.then(F.Str.cmp(x, _), F.Lst.cmp(at, ct)) == LT{} : Cmp}
      %Equal.sym(Cmp, F.Str.cmp(x, y), LT{}, h1) : {F.Cmp.then(_, F.Lst.cmp(at, ct)) == LT{} : Cmp}
      {==}
    case EQ{} LT{}:
      %Equal.sym(String, x, y, str_eq(x, y, h1)) : {F.Cmp.then(F.Str.cmp(_, z), F.Lst.cmp(at, ct)) == LT{} : Cmp}
      %Equal.sym(Cmp, F.Str.cmp(y, z), LT{}, h2) : {F.Cmp.then(_, F.Lst.cmp(at, ct)) == LT{} : Cmp}
      {==}
    case EQ{} EQ{}:
      %Equal.sym(String, x, y, str_eq(x, y, h1)) : {F.Cmp.then(F.Str.cmp(_, z), F.Lst.cmp(at, ct)) == LT{} : Cmp}
      %Equal.sym(Cmp, F.Str.cmp(y, z), EQ{}, h2) : {F.Cmp.then(_, F.Lst.cmp(at, ct)) == LT{} : Cmp}
      ih(e1, e2)
    case GT{} _:
      absurd({F.Cmp.then(F.Str.cmp(x, z), F.Lst.cmp(at, ct)) == LT{} : Cmp}, gt_not_lt(e1))
    case LT{} GT{}:
      absurd({F.Cmp.then(F.Str.cmp(x, z), F.Lst.cmp(at, ct)) == LT{} : Cmp}, gt_not_lt(e2))
    case EQ{} GT{}:
      absurd({F.Cmp.then(F.Str.cmp(x, z), F.Lst.cmp(at, ct)) == LT{} : Cmp}, gt_not_lt(e2))

law lst_trans:
  for a: List<&2, String>
  for b: List<&2, String>
  for c: List<&2, String>
  for e1: {F.Lst.cmp(a, b) == LT{} : Cmp}
  for e2: {F.Lst.cmp(b, c) == LT{} : Cmp}
  {F.Lst.cmp(a, c) == LT{} : Cmp}

def lst_trans(a, b, c, e1, e2):
  match a b c:
    case Nil{} Nil{} c:
      absurd({F.Lst.cmp(Nil{}, c) == LT{} : Cmp}, eq_not_lt(e1))
    case Nil{} Con{y, bt} Nil{}:
      absurd({F.Lst.cmp(Nil{}, Nil{}) == LT{} : Cmp}, gt_not_lt(e2))
    case Nil{} Con{y, bt} Con{z, ct}:
      {==}
    case Con{x, at} Nil{} c:
      absurd({F.Lst.cmp(Con{x, at}, c) == LT{} : Cmp}, gt_not_lt(e1))
    case Con{x, at} Con{y, bt} Nil{}:
      absurd({F.Lst.cmp(Con{x, at}, Nil{}) == LT{} : Cmp}, gt_not_lt(e2))
    case Con{+x, +at} Con{+y, +bt} Con{+z, +ct}:
      lst_trans.step(x, y, z, at, bt, ct, F.Str.cmp(x, y), {==}, F.Str.cmp(y, z), {==}, e1, e2, f => g => lst_trans(at, bt, ct, f, g))

# Associativity of the minimum, given the three comparisons among a, b and c and the facts a
# total order provides about them.
law min_assoc.core:
  for -A: Data
  for -C: A -> A -> Cmp
  for +a: A
  for +b: A
  for +c: A
  for x: Cmp
  for hx: {C(a, b) == x : Cmp}
  for y: Cmp
  for hy: {C(b, c) == y : Cmp}
  for z: Cmp
  for hz: {C(a, c) == z : Cmp}
  for gg: {x == GT{} : Cmp} -> {y == GT{} : Cmp} -> {z == GT{} : Cmp}
  for ll: {x == LT{} : Cmp} -> {y == LT{} : Cmp} -> {z == LT{} : Cmp}
  for le: {x == LT{} : Cmp} -> {y == EQ{} : Cmp} -> {z == LT{} : Cmp}
  for el: {x == EQ{} : Cmp} -> {y == LT{} : Cmp} -> {z == LT{} : Cmp}
  for ee: {x == EQ{} : Cmp} -> {y == EQ{} : Cmp} -> {z == EQ{} : Cmp}
  {F.Min.pick(A, Cmp.is_gt(C(F.Min.pick(A, Cmp.is_gt(x), a, b), c)), F.Min.pick(A, Cmp.is_gt(x), a, b), c) == F.Min.pick(A, Cmp.is_gt(C(a, F.Min.pick(A, Cmp.is_gt(y), b, c))), a, F.Min.pick(A, Cmp.is_gt(y), b, c)) : A}

def min_assoc.core(A, C, a, b, c, x, hx, y, hy, z, hz, gg, ll, le, el, ee):
  match x y:
    case GT{} GT{}:
      %Equal.sym(Cmp, C(b, c), GT{}, hy) : {F.Min.pick(A, Cmp.is_gt(_), b, c) == F.Min.pick(A, Cmp.is_gt(C(a, c)), a, c) : A}
      %Equal.sym(Cmp, C(a, c), z, hz) : {c == F.Min.pick(A, Cmp.is_gt(_), a, c) : A}
      %Equal.sym(Cmp, z, GT{}, gg({==}, {==})) : {c == F.Min.pick(A, Cmp.is_gt(_), a, c) : A}
      {==}
    case GT{} LT{}:
      %Equal.sym(Cmp, C(b, c), LT{}, hy) : {F.Min.pick(A, Cmp.is_gt(_), b, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, C(a, b), GT{}, hx) : {b == F.Min.pick(A, Cmp.is_gt(_), a, b) : A}
      {==}
    case GT{} EQ{}:
      %Equal.sym(Cmp, C(b, c), EQ{}, hy) : {F.Min.pick(A, Cmp.is_gt(_), b, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, C(a, b), GT{}, hx) : {b == F.Min.pick(A, Cmp.is_gt(_), a, b) : A}
      {==}
    case LT{} GT{}:
      {==}
    case EQ{} GT{}:
      {==}
    case LT{} LT{}:
      %Equal.sym(Cmp, C(a, c), z, hz) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, z, LT{}, ll({==}, {==})) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, C(a, b), LT{}, hx) : {a == F.Min.pick(A, Cmp.is_gt(_), a, b) : A}
      {==}
    case LT{} EQ{}:
      %Equal.sym(Cmp, C(a, c), z, hz) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, z, LT{}, le({==}, {==})) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, C(a, b), LT{}, hx) : {a == F.Min.pick(A, Cmp.is_gt(_), a, b) : A}
      {==}
    case EQ{} LT{}:
      %Equal.sym(Cmp, C(a, c), z, hz) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, z, LT{}, el({==}, {==})) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, C(a, b), EQ{}, hx) : {a == F.Min.pick(A, Cmp.is_gt(_), a, b) : A}
      {==}
    case EQ{} EQ{}:
      %Equal.sym(Cmp, C(a, c), z, hz) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, z, EQ{}, ee({==}, {==})) : {F.Min.pick(A, Cmp.is_gt(_), a, c) == F.Min.pick(A, Cmp.is_gt(C(a, b)), a, b) : A}
      %Equal.sym(Cmp, C(a, b), EQ{}, hx) : {a == F.Min.pick(A, Cmp.is_gt(_), a, b) : A}
      {==}

# Strings: the facts min_assoc.core needs
law str_gt_lt:
  for +a: String
  for +b: String
  for e: {F.Str.cmp(a, b) == GT{} : Cmp}
  {F.Str.cmp(b, a) == LT{} : Cmp}

def str_gt_lt(a, b, e):
  %Equal.sym(Cmp, F.Str.cmp(b, a), Cmp.flip(F.Str.cmp(a, b)), str_anti(b, a)) : {_ == LT{} : Cmp}
  %Equal.sym(Cmp, F.Str.cmp(a, b), GT{}, e) : {Cmp.flip(_) == LT{} : Cmp}
  {==}

law str_lt_gt:
  for +a: String
  for +b: String
  for e: {F.Str.cmp(a, b) == LT{} : Cmp}
  {F.Str.cmp(b, a) == GT{} : Cmp}

def str_lt_gt(a, b, e):
  %Equal.sym(Cmp, F.Str.cmp(b, a), Cmp.flip(F.Str.cmp(a, b)), str_anti(b, a)) : {_ == GT{} : Cmp}
  %Equal.sym(Cmp, F.Str.cmp(a, b), LT{}, e) : {Cmp.flip(_) == GT{} : Cmp}
  {==}

law str_gg:
  for +a: String
  for +b: String
  for +c: String
  for e1: {F.Str.cmp(a, b) == GT{} : Cmp}
  for e2: {F.Str.cmp(b, c) == GT{} : Cmp}
  {F.Str.cmp(a, c) == GT{} : Cmp}

def str_gg(a, b, c, e1, e2):
  str_lt_gt(c, a, str_trans(c, b, a, str_gt_lt(b, c, e2), str_gt_lt(a, b, e1)))

law str_le:
  for +a: String
  for +b: String
  for +c: String
  for e1: {F.Str.cmp(a, b) == LT{} : Cmp}
  for e2: {F.Str.cmp(b, c) == EQ{} : Cmp}
  {F.Str.cmp(a, c) == LT{} : Cmp}

def str_le(a, b, c, e1, e2):
  %str_eq(b, c, e2) : {F.Str.cmp(a, _) == LT{} : Cmp}
  e1

law str_el:
  for +a: String
  for +b: String
  for +c: String
  for e1: {F.Str.cmp(a, b) == EQ{} : Cmp}
  for e2: {F.Str.cmp(b, c) == LT{} : Cmp}
  {F.Str.cmp(a, c) == LT{} : Cmp}

def str_el(a, b, c, e1, e2):
  %Equal.sym(String, a, b, str_eq(a, b, e1)) : {F.Str.cmp(_, c) == LT{} : Cmp}
  e2

law str_ee:
  for +a: String
  for +b: String
  for +c: String
  for e1: {F.Str.cmp(a, b) == EQ{} : Cmp}
  for e2: {F.Str.cmp(b, c) == EQ{} : Cmp}
  {F.Str.cmp(a, c) == EQ{} : Cmp}

def str_ee(a, b, c, e1, e2):
  %Equal.sym(String, a, b, str_eq(a, b, e1)) : {F.Str.cmp(_, c) == EQ{} : Cmp}
  %str_eq(b, c, e2) : {F.Str.cmp(b, _) == EQ{} : Cmp}
  str_refl(b)

law str_min_idem:
  for +a: String
  {F.String.min(a, a) == a : String}

def str_min_idem(a):
  min_idem(String, F.Str.cmp, x => str_refl(x), a)

law str_min_comm:
  for +a: String
  for +b: String
  {F.String.min(a, b) == F.String.min(b, a) : String}

def str_min_comm(a, b):
  min_comm(String, F.Str.cmp, x => y => str_anti(x, y), x => y => e => str_eq(x, y, e), a, b)

law str_min_assoc:
  for +a: String
  for +b: String
  for +c: String
  {F.String.min(F.String.min(a, b), c) == F.String.min(a, F.String.min(b, c)) : String}

def str_min_assoc(a, b, c):
  min_assoc.core(String, F.Str.cmp, a, b, c, F.Str.cmp(a, b), {==}, F.Str.cmp(b, c), {==}, F.Str.cmp(a, c), {==}, p => q => str_gg(a, b, c, p, q), p => q => str_trans(a, b, c, p, q), p => q => str_le(a, b, c, p, q), p => q => str_el(a, b, c, p, q), p => q => str_ee(a, b, c, p, q))

# Lists of strings: the facts min_assoc.core needs
law lst_gt_lt:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for e: {F.Lst.cmp(a, b) == GT{} : Cmp}
  {F.Lst.cmp(b, a) == LT{} : Cmp}

def lst_gt_lt(a, b, e):
  %Equal.sym(Cmp, F.Lst.cmp(b, a), Cmp.flip(F.Lst.cmp(a, b)), lst_anti(b, a)) : {_ == LT{} : Cmp}
  %Equal.sym(Cmp, F.Lst.cmp(a, b), GT{}, e) : {Cmp.flip(_) == LT{} : Cmp}
  {==}

law lst_lt_gt:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for e: {F.Lst.cmp(a, b) == LT{} : Cmp}
  {F.Lst.cmp(b, a) == GT{} : Cmp}

def lst_lt_gt(a, b, e):
  %Equal.sym(Cmp, F.Lst.cmp(b, a), Cmp.flip(F.Lst.cmp(a, b)), lst_anti(b, a)) : {_ == GT{} : Cmp}
  %Equal.sym(Cmp, F.Lst.cmp(a, b), LT{}, e) : {Cmp.flip(_) == GT{} : Cmp}
  {==}

law lst_gg:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for +c: List<&2, String>
  for e1: {F.Lst.cmp(a, b) == GT{} : Cmp}
  for e2: {F.Lst.cmp(b, c) == GT{} : Cmp}
  {F.Lst.cmp(a, c) == GT{} : Cmp}

def lst_gg(a, b, c, e1, e2):
  lst_lt_gt(c, a, lst_trans(c, b, a, lst_gt_lt(b, c, e2), lst_gt_lt(a, b, e1)))

law lst_le:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for +c: List<&2, String>
  for e1: {F.Lst.cmp(a, b) == LT{} : Cmp}
  for e2: {F.Lst.cmp(b, c) == EQ{} : Cmp}
  {F.Lst.cmp(a, c) == LT{} : Cmp}

def lst_le(a, b, c, e1, e2):
  %lst_eq(b, c, e2) : {F.Lst.cmp(a, _) == LT{} : Cmp}
  e1

law lst_el:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for +c: List<&2, String>
  for e1: {F.Lst.cmp(a, b) == EQ{} : Cmp}
  for e2: {F.Lst.cmp(b, c) == LT{} : Cmp}
  {F.Lst.cmp(a, c) == LT{} : Cmp}

def lst_el(a, b, c, e1, e2):
  %Equal.sym(List<&2, String>, a, b, lst_eq(a, b, e1)) : {F.Lst.cmp(_, c) == LT{} : Cmp}
  e2

law lst_ee:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for +c: List<&2, String>
  for e1: {F.Lst.cmp(a, b) == EQ{} : Cmp}
  for e2: {F.Lst.cmp(b, c) == EQ{} : Cmp}
  {F.Lst.cmp(a, c) == EQ{} : Cmp}

def lst_ee(a, b, c, e1, e2):
  %Equal.sym(List<&2, String>, a, b, lst_eq(a, b, e1)) : {F.Lst.cmp(_, c) == EQ{} : Cmp}
  %lst_eq(b, c, e2) : {F.Lst.cmp(b, _) == EQ{} : Cmp}
  lst_refl(b)

law lst_min_idem:
  for +a: List<&2, String>
  {F.Lst.min(a, a) == a : List<&2, String>}

def lst_min_idem(a):
  min_idem(List<&2, String>, F.Lst.cmp, x => lst_refl(x), a)

law lst_min_comm:
  for +a: List<&2, String>
  for +b: List<&2, String>
  {F.Lst.min(a, b) == F.Lst.min(b, a) : List<&2, String>}

def lst_min_comm(a, b):
  min_comm(List<&2, String>, F.Lst.cmp, x => y => lst_anti(x, y), x => y => e => lst_eq(x, y, e), a, b)

law lst_min_assoc:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for +c: List<&2, String>
  {F.Lst.min(F.Lst.min(a, b), c) == F.Lst.min(a, F.Lst.min(b, c)) : List<&2, String>}

def lst_min_assoc(a, b, c):
  min_assoc.core(List<&2, String>, F.Lst.cmp, a, b, c, F.Lst.cmp(a, b), {==}, F.Lst.cmp(b, c), {==}, F.Lst.cmp(a, c), {==}, p => q => lst_gg(a, b, c, p, q), p => q => lst_trans(a, b, c, p, q), p => q => lst_le(a, b, c, p, q), p => q => lst_el(a, b, c, p, q), p => q => lst_ee(a, b, c, p, q))

# Flags
# -----

law or_idem:
  for a: Bool
  {F.Bool.or3(a, a) == a : Bool}

def or_idem(a):
  match a:
    case True{}:
      {==}
    case False{}:
      {==}

law or_comm:
  for a: Bool
  for b: Bool
  {F.Bool.or3(a, b) == F.Bool.or3(b, a) : Bool}

def or_comm(a, b):
  match a b:
    case True{} True{}:
      {==}
    case True{} False{}:
      {==}
    case False{} True{}:
      {==}
    case False{} False{}:
      {==}

law or_assoc:
  for a: Bool
  for b: Bool
  for c: Bool
  {F.Bool.or3(F.Bool.or3(a, b), c) == F.Bool.or3(a, F.Bool.or3(b, c)) : Bool}

def or_assoc(a, b, c):
  match a:
    case True{}:
      {==}
    case False{}:
      {==}

# Numbers
# -------

law max_idem:
  for +a: Nat
  {Nat.max(a, a) == a : Nat}

def max_idem(a):
  match a:
    case 0n:
      {==}
    case 1n+p:
      %max_idem(p) : {1n+Nat.max(p, p) == 1n+_ : Nat}
      {==}

law max_comm:
  for +a: Nat
  for +b: Nat
  {Nat.max(a, b) == Nat.max(b, a) : Nat}

def max_comm(a, b):
  match a b:
    case 0n 0n:
      {==}
    case 0n 1n+bp:
      {==}
    case 1n+ap 0n:
      {==}
    case 1n+ap 1n+bp:
      %max_comm(ap, bp) : {1n+Nat.max(ap, bp) == 1n+_ : Nat}
      {==}

law max_assoc:
  for +a: Nat
  for +b: Nat
  for +c: Nat
  {Nat.max(Nat.max(a, b), c) == Nat.max(a, Nat.max(b, c)) : Nat}

def max_assoc(a, b, c):
  match a b c:
    case 0n _ _:
      {==}
    case 1n+ap 0n _:
      {==}
    case 1n+ap 1n+bp 0n:
      {==}
    case 1n+ap 1n+bp 1n+cp:
      %max_assoc(ap, bp, cp) : {1n+Nat.max(Nat.max(ap, bp), cp) == 1n+_ : Nat}
      {==}


# Responses
# ---------

law group_merge_comm:
  for a: F.Group
  for b: F.Group
  {F.Group.merge(a, b) == F.Group.merge(b, a) : F.Group}

def group_merge_comm(a, b):
  match a b:
    case F.Group{+ai, +ao, +ar, +aw, +ah, +am, +as, +af, +afa, +ag, +ax} F.Group{+bi, +bo, +br, +bw, +bh, +bm, +bs, +bf, +bfa, +bg, +bx}:
      %Equal.sym(Nat, Nat.max(ai, bi), Nat.max(bi, ai), max_comm(ai, bi)) : {F.Group{_, Nat.max(ao, bo), Nat.max(ar, br), Nat.max(aw, bw), Nat.max(ah, bh), Nat.max(am, bm), Nat.max(as, bs), Nat.max(af, bf), F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Nat, Nat.max(ao, bo), Nat.max(bo, ao), max_comm(ao, bo)) : {F.Group{Nat.max(bi, ai), _, Nat.max(ar, br), Nat.max(aw, bw), Nat.max(ah, bh), Nat.max(am, bm), Nat.max(as, bs), Nat.max(af, bf), F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Nat, Nat.max(ar, br), Nat.max(br, ar), max_comm(ar, br)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), _, Nat.max(aw, bw), Nat.max(ah, bh), Nat.max(am, bm), Nat.max(as, bs), Nat.max(af, bf), F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Nat, Nat.max(aw, bw), Nat.max(bw, aw), max_comm(aw, bw)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), _, Nat.max(ah, bh), Nat.max(am, bm), Nat.max(as, bs), Nat.max(af, bf), F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Nat, Nat.max(ah, bh), Nat.max(bh, ah), max_comm(ah, bh)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), _, Nat.max(am, bm), Nat.max(as, bs), Nat.max(af, bf), F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Nat, Nat.max(am, bm), Nat.max(bm, am), max_comm(am, bm)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), _, Nat.max(as, bs), Nat.max(af, bf), F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Nat, Nat.max(as, bs), Nat.max(bs, as), max_comm(as, bs)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), _, Nat.max(af, bf), F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Nat, Nat.max(af, bf), Nat.max(bf, af), max_comm(af, bf)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), _, F.Bool.or3(afa, bfa), F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Bool, F.Bool.or3(afa, bfa), F.Bool.or3(bfa, afa), or_comm(afa, bfa)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), _, F.Bool.or3(ag, bg), F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(Bool, F.Bool.or3(ag, bg), F.Bool.or3(bg, ag), or_comm(ag, bg)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), _, F.Lst.min(ax, bx)} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      %Equal.sym(List<&2, String>, F.Lst.min(ax, bx), F.Lst.min(bx, ax), lst_min_comm(ax, bx)) : {F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), _} == F.Group{Nat.max(bi, ai), Nat.max(bo, ao), Nat.max(br, ar), Nat.max(bw, aw), Nat.max(bh, ah), Nat.max(bm, am), Nat.max(bs, as), Nat.max(bf, af), F.Bool.or3(bfa, afa), F.Bool.or3(bg, ag), F.Lst.min(bx, ax)} : F.Group}
      {==}

law group_merge_idem:
  for a: F.Group
  {F.Group.merge(a, a) == a : F.Group}

def group_merge_idem(a):
  match a:
    case F.Group{+ai, +ao, +ar, +aw, +ah, +am, +as, +af, +afa, +ag, +ax}:
      %Equal.sym(Nat, Nat.max(ai, ai), ai, max_idem(ai)) : {F.Group{_, Nat.max(ao, ao), Nat.max(ar, ar), Nat.max(aw, aw), Nat.max(ah, ah), Nat.max(am, am), Nat.max(as, as), Nat.max(af, af), F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Nat, Nat.max(ao, ao), ao, max_idem(ao)) : {F.Group{ai, _, Nat.max(ar, ar), Nat.max(aw, aw), Nat.max(ah, ah), Nat.max(am, am), Nat.max(as, as), Nat.max(af, af), F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Nat, Nat.max(ar, ar), ar, max_idem(ar)) : {F.Group{ai, ao, _, Nat.max(aw, aw), Nat.max(ah, ah), Nat.max(am, am), Nat.max(as, as), Nat.max(af, af), F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Nat, Nat.max(aw, aw), aw, max_idem(aw)) : {F.Group{ai, ao, ar, _, Nat.max(ah, ah), Nat.max(am, am), Nat.max(as, as), Nat.max(af, af), F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Nat, Nat.max(ah, ah), ah, max_idem(ah)) : {F.Group{ai, ao, ar, aw, _, Nat.max(am, am), Nat.max(as, as), Nat.max(af, af), F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Nat, Nat.max(am, am), am, max_idem(am)) : {F.Group{ai, ao, ar, aw, ah, _, Nat.max(as, as), Nat.max(af, af), F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Nat, Nat.max(as, as), as, max_idem(as)) : {F.Group{ai, ao, ar, aw, ah, am, _, Nat.max(af, af), F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Nat, Nat.max(af, af), af, max_idem(af)) : {F.Group{ai, ao, ar, aw, ah, am, as, _, F.Bool.or3(afa, afa), F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Bool, F.Bool.or3(afa, afa), afa, or_idem(afa)) : {F.Group{ai, ao, ar, aw, ah, am, as, af, _, F.Bool.or3(ag, ag), F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(Bool, F.Bool.or3(ag, ag), ag, or_idem(ag)) : {F.Group{ai, ao, ar, aw, ah, am, as, af, afa, _, F.Lst.min(ax, ax)} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      %Equal.sym(List<&2, String>, F.Lst.min(ax, ax), ax, lst_min_idem(ax)) : {F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, _} == F.Group{ai, ao, ar, aw, ah, am, as, af, afa, ag, ax} : F.Group}
      {==}

law group_merge_assoc:
  for a: F.Group
  for b: F.Group
  for c: F.Group
  {F.Group.merge(F.Group.merge(a, b), c) == F.Group.merge(a, F.Group.merge(b, c)) : F.Group}

def group_merge_assoc(a, b, c):
  match a b c:
    case F.Group{+ai, +ao, +ar, +aw, +ah, +am, +as, +af, +afa, +ag, +ax} F.Group{+bi, +bo, +br, +bw, +bh, +bm, +bs, +bf, +bfa, +bg, +bx} F.Group{+ci, +co, +cr, +cw, +ch, +cm, +cs, +cf, +cfa, +cg, +cx}:
      %Equal.sym(Nat, Nat.max(Nat.max(ai, bi), ci), Nat.max(ai, Nat.max(bi, ci)), max_assoc(ai, bi, ci)) : {F.Group{_, Nat.max(Nat.max(ao, bo), co), Nat.max(Nat.max(ar, br), cr), Nat.max(Nat.max(aw, bw), cw), Nat.max(Nat.max(ah, bh), ch), Nat.max(Nat.max(am, bm), cm), Nat.max(Nat.max(as, bs), cs), Nat.max(Nat.max(af, bf), cf), F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Nat, Nat.max(Nat.max(ao, bo), co), Nat.max(ao, Nat.max(bo, co)), max_assoc(ao, bo, co)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), _, Nat.max(Nat.max(ar, br), cr), Nat.max(Nat.max(aw, bw), cw), Nat.max(Nat.max(ah, bh), ch), Nat.max(Nat.max(am, bm), cm), Nat.max(Nat.max(as, bs), cs), Nat.max(Nat.max(af, bf), cf), F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Nat, Nat.max(Nat.max(ar, br), cr), Nat.max(ar, Nat.max(br, cr)), max_assoc(ar, br, cr)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), _, Nat.max(Nat.max(aw, bw), cw), Nat.max(Nat.max(ah, bh), ch), Nat.max(Nat.max(am, bm), cm), Nat.max(Nat.max(as, bs), cs), Nat.max(Nat.max(af, bf), cf), F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Nat, Nat.max(Nat.max(aw, bw), cw), Nat.max(aw, Nat.max(bw, cw)), max_assoc(aw, bw, cw)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), _, Nat.max(Nat.max(ah, bh), ch), Nat.max(Nat.max(am, bm), cm), Nat.max(Nat.max(as, bs), cs), Nat.max(Nat.max(af, bf), cf), F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Nat, Nat.max(Nat.max(ah, bh), ch), Nat.max(ah, Nat.max(bh, ch)), max_assoc(ah, bh, ch)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), _, Nat.max(Nat.max(am, bm), cm), Nat.max(Nat.max(as, bs), cs), Nat.max(Nat.max(af, bf), cf), F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Nat, Nat.max(Nat.max(am, bm), cm), Nat.max(am, Nat.max(bm, cm)), max_assoc(am, bm, cm)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), _, Nat.max(Nat.max(as, bs), cs), Nat.max(Nat.max(af, bf), cf), F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Nat, Nat.max(Nat.max(as, bs), cs), Nat.max(as, Nat.max(bs, cs)), max_assoc(as, bs, cs)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), _, Nat.max(Nat.max(af, bf), cf), F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Nat, Nat.max(Nat.max(af, bf), cf), Nat.max(af, Nat.max(bf, cf)), max_assoc(af, bf, cf)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), _, F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Bool, F.Bool.or3(F.Bool.or3(afa, bfa), cfa), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), or_assoc(afa, bfa, cfa)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), _, F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(Bool, F.Bool.or3(F.Bool.or3(ag, bg), cg), F.Bool.or3(ag, F.Bool.or3(bg, cg)), or_assoc(ag, bg, cg)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), _, F.Lst.min(F.Lst.min(ax, bx), cx)} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      %Equal.sym(List<&2, String>, F.Lst.min(F.Lst.min(ax, bx), cx), F.Lst.min(ax, F.Lst.min(bx, cx)), lst_min_assoc(ax, bx, cx)) : {F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), _} == F.Group{Nat.max(ai, Nat.max(bi, ci)), Nat.max(ao, Nat.max(bo, co)), Nat.max(ar, Nat.max(br, cr)), Nat.max(aw, Nat.max(bw, cw)), Nat.max(ah, Nat.max(bh, ch)), Nat.max(am, Nat.max(bm, cm)), Nat.max(as, Nat.max(bs, cs)), Nat.max(af, Nat.max(bf, cf)), F.Bool.or3(afa, F.Bool.or3(bfa, cfa)), F.Bool.or3(ag, F.Bool.or3(bg, cg)), F.Lst.min(ax, F.Lst.min(bx, cx))} : F.Group}
      {==}

# Billing
# -------

law add_zero:
  for +x: Nat
  {Nat.add(x, 0n) == x : Nat}

def add_zero(x):
  match x:
    case 0n:
      {==}
    case 1n+(+p):
      %Equal.sym(Nat, Nat.add(p, 0n), p, add_zero(p)) : {1n+_ == 1n+p : Nat}
      {==}

law add_succ:
  for +x: Nat
  for +y: Nat
  {Nat.add(x, 1n+y) == 1n+Nat.add(x, y) : Nat}

def add_succ(x, y):
  match x:
    case 0n:
      {==}
    case 1n+(+p):
      %Equal.sym(Nat, Nat.add(p, 1n+y), 1n+Nat.add(p, y), add_succ(p, y)) : {1n+_ == 1n+1n+Nat.add(p, y) : Nat}
      {==}

law sub_zero:
  for +x: Nat
  {Nat.sub(x, 0n) == x : Nat}

def sub_zero(x):
  match x:
    case 0n:
      {==}
    case 1n+p:
      {==}

# Taking the part of b covered by a, and the rest, gives b back.
law split_total:
  for +a: Nat
  for +b: Nat
  {Nat.add(Nat.sub(b, Nat.min(a, b)), Nat.min(a, b)) == b : Nat}

def split_total(a, b):
  match a b:
    case 0n +b:
      %Equal.sym(Nat, Nat.add(Nat.sub(b, 0n), 0n), Nat.sub(b, 0n), add_zero(Nat.sub(b, 0n))) : {_ == b : Nat}
      sub_zero(b)
    case 1n+ap 0n:
      {==}
    case 1n+(+ap) 1n+(+bp):
      %Equal.sym(Nat, Nat.add(Nat.sub(bp, Nat.min(ap, bp)), 1n+Nat.min(ap, bp)), 1n+Nat.add(Nat.sub(bp, Nat.min(ap, bp)), Nat.min(ap, bp)), add_succ(Nat.sub(bp, Nat.min(ap, bp)), Nat.min(ap, bp))) : {_ == 1n+bp : Nat}
      %Equal.sym(Nat, Nat.add(Nat.sub(bp, Nat.min(ap, bp)), Nat.min(ap, bp)), bp, split_total(ap, bp)) : {1n+_ == 1n+bp : Nat}
      {==}

# Every cache-write token is billed exactly once, at the 1-hour or the 5-minute rate.
law cache_write_billed_once:
  for inp: Nat
  for out: Nat
  for cread: Nat
  for +cwrite: Nat
  for +c1h: Nat
  {R.Toks.writes(R.Toks.of(inp, out, cread, cwrite, c1h)) == cwrite : Nat}

def cache_write_billed_once(inp, out, cread, cwrite, c1h):
  split_total(c1h, cwrite)


# The laws
# --------

def Laws.group_merge_comm(a, b):
  group_merge_comm(a, b)

def Laws.group_merge_assoc(a, b, c):
  group_merge_assoc(a, b, c)

def Laws.group_merge_idem(a):
  group_merge_idem(a)

def Laws.tool_merge_comm(a, b):
  lst_min_comm(a, b)

def Laws.tool_merge_assoc(a, b, c):
  lst_min_assoc(a, b, c)

def Laws.tool_merge_idem(a):
  lst_min_idem(a)

def Laws.prompt_merge_comm(a, b):
  str_min_comm(a, b)

def Laws.prompt_merge_assoc(a, b, c):
  str_min_assoc(a, b, c)

def Laws.prompt_merge_idem(a):
  str_min_idem(a)

def Laws.cache_write_billed_once(inp, out, cread, cwrite, c1h):
  split_total(c1h, cwrite)
