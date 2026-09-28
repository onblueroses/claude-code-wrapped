import Base
import ./types.bend as T
import ./gen/line.bend as L
import ./fold.bend as F

# Streaming JSON-lines reader. It validates every line as strict JSON (RFC 8259: no NaN or
# Infinity, no raw control characters in strings, no trailing commas) so that a line counts as
# malformed exactly when Python's json.loads rejects it, and it captures only the fields the
# metrics read, found by their path. Everything else is skipped without building values.
#
# A key that repeats replaces the earlier value, including everything nested under it, as in
# Python's json module: starting a value resets the fields that live under its slot.
#
# The lexer is one state machine over characters. Its defs are @unsafe: they call each other
# in both directions, which Bend's termination check does not cover. No law depends on them;
# the lexer is tested against the Python oracle instead (tests/).

# Where a value sits, as far as the metrics care.

type Slot is Data:
  SNone{}
  SRoot{}
  SType{}
  STs{}
  SSession{}
  SCwd{}
  SSide{}
  SMeta{}
  SUuid{}
  SMsg{}
  SMid{}
  SModel{}
  SUsage{}
  SContent{}
  SInp{}
  SOut{}
  SCread{}
  SCwrite{}
  SCC{}
  SSTU{}
  SSpeed{}
  SGeo{}
  SC1h{}
  SC5m{}
  SWs{}
  SWf{}
  SBlock{}
  SBType{}
  SBId{}
  SBName{}
  SBText{}

# The object whose keys are being read.
type Ctx is Data:
  CRoot{}
  CMsg{}
  CUsage{}
  CCC{}
  CSTU{}
  CBlock{}
  COther{}

type Frame is Data:
  FObj{ctx: Ctx, slot: Slot}
  FArr{content: Bool}

# What a string's characters are kept for: nothing, the whole value, or the human-prompt test
# (the first 32 characters and whether any is not ASCII whitespace). CapDone is a human test
# whose answer is already settled, so the rest of the string is skipped like CapNone.
type Cap is Data:
  CapNone{}
  CapFull{}
  CapHuman{}
  CapDone{}

# A string being captured: characters reversed, how many, whether one was not blank, and a
# pending UTF-16 high surrogate from a \u escape (0 when none). It lives in the mode, so
# reading a string never rebuilds the line state.
type SB is Data:
  SB{buf: String, n: Nat, nb: Bool, hi: U32}

type NMode is Data:
  NMinus{}
  NZero{}
  NInt{}
  NDot{}
  NFrac{}
  NE{}
  NESign{}
  NExp{}

type Mode is Data:
  MValue{}
  MValueOrEnd{}
  MKey{}
  MKeyOrEnd{}
  MColon{}
  MAfter{}
  MDone{}
  MStr{cap: Cap, sb: SB}
  MEsc{cap: Cap, sb: SB}
  MHex{cap: Cap, sb: SB, left: Nat, acc: U32}
  MNum{nm: NMode}
  MLit{rest: String, val: Bool}
  MErr{}

# key: the string being read is an object key. val/ok: a number's value and whether it is a
# non-negative integer that fits. top: the line's value is an object.
type Lx is Data:
  Lx{stack: List<&2, Frame>, slot: Slot, key: Bool, val: Nat, ok: Bool, top: Bool, line: L.Line}

def SB.empty() -> SB:
  SB{"", 0n, False{}, 0}

def Lx.fresh() -> Lx:
  Lx{Nil{}, SRoot{}, False{}, 0n, False{}, False{}, L.Line.empty()}

# Characters
# ----------

def Chr.is_ws(+c: U32) -> Bool:
  U32.is_eq(c, 32) || U32.is_eq(c, 9) || U32.is_eq(c, 13)

def Chr.is_blank(+c: U32) -> Bool:
  U32.is_eq(c, 32) || U32.is_eq(c, 9) || U32.is_eq(c, 10) || U32.is_eq(c, 13) || U32.is_eq(c, 11) || U32.is_eq(c, 12)

def Chr.is_digit(+c: U32) -> Bool:
  U32.is_ge(c, 48) && U32.is_le(c, 57)

def Chr.is_plain(+c: U32) -> Bool:
  U32.is_ge(c, 32) && U32.is_ne(c, 34) && U32.is_ne(c, 92)

# Skips characters that end nothing inside a string whose content is not kept.
@unsafe def Lex.skip.if(plain: Bool, c: U32, t: T.Txt) -> T.Txt:
  match plain:
    case True{}:
      Lex.skip(t)
    case False{}:
      T.TCon{c, t}

@unsafe def Lex.skip(s: T.Txt) -> T.Txt:
  match s:
    case T.TNil{}:
      T.TNil{}
    case T.TCon{+c, t}:
      Lex.skip.if(Chr.is_plain(c), c, t)

# Skips to the end of a malformed line.
@unsafe def Lex.skip_line.if(nl: Bool, c: U32, t: T.Txt) -> T.Txt:
  match nl:
    case True{}:
      T.TCon{c, t}
    case False{}:
      Lex.skip_line(t)

@unsafe def Lex.skip_line(s: T.Txt) -> T.Txt:
  match s:
    case T.TNil{}:
      T.TNil{}
    case T.TCon{+c, t}:
      Lex.skip_line.if(U32.is_eq(c, 10), c, t)

# Slots
# -----

def Key.pick(hit: Bool, slot: Slot, rest: Slot) -> Slot:
  match hit:
    case True{}:
      slot
    case False{}:
      rest

def Key.root(+k: String) -> Slot:
  Key.pick(String.eq(k, "type"), SType{}, Key.pick(String.eq(k, "timestamp"), STs{}, Key.pick(String.eq(k, "sessionId"), SSession{}, Key.pick(String.eq(k, "cwd"), SCwd{}, Key.pick(String.eq(k, "isSidechain"), SSide{}, Key.pick(String.eq(k, "isMeta"), SMeta{}, Key.pick(String.eq(k, "uuid"), SUuid{}, Key.pick(String.eq(k, "message"), SMsg{}, SNone{}))))))))

def Key.msg(+k: String) -> Slot:
  Key.pick(String.eq(k, "id"), SMid{}, Key.pick(String.eq(k, "model"), SModel{}, Key.pick(String.eq(k, "usage"), SUsage{}, Key.pick(String.eq(k, "content"), SContent{}, SNone{}))))

def Key.usage(+k: String) -> Slot:
  Key.pick(String.eq(k, "input_tokens"), SInp{}, Key.pick(String.eq(k, "output_tokens"), SOut{}, Key.pick(String.eq(k, "cache_read_input_tokens"), SCread{}, Key.pick(String.eq(k, "cache_creation_input_tokens"), SCwrite{}, Key.pick(String.eq(k, "cache_creation"), SCC{}, Key.pick(String.eq(k, "server_tool_use"), SSTU{}, Key.pick(String.eq(k, "speed"), SSpeed{}, Key.pick(String.eq(k, "inference_geo"), SGeo{}, SNone{}))))))))

def Key.cc(+k: String) -> Slot:
  Key.pick(String.eq(k, "ephemeral_1h_input_tokens"), SC1h{}, Key.pick(String.eq(k, "ephemeral_5m_input_tokens"), SC5m{}, SNone{}))

def Key.stu(+k: String) -> Slot:
  Key.pick(String.eq(k, "web_search_requests"), SWs{}, Key.pick(String.eq(k, "web_fetch_requests"), SWf{}, SNone{}))

def Key.block(+k: String) -> Slot:
  Key.pick(String.eq(k, "type"), SBType{}, Key.pick(String.eq(k, "id"), SBId{}, Key.pick(String.eq(k, "name"), SBName{}, Key.pick(String.eq(k, "text"), SBText{}, SNone{}))))

def Key.slot(ctx: Ctx, k: String) -> Slot:
  match ctx:
    case CRoot{}:
      Key.root(k)
    case CMsg{}:
      Key.msg(k)
    case CUsage{}:
      Key.usage(k)
    case CCC{}:
      Key.cc(k)
    case CSTU{}:
      Key.stu(k)
    case CBlock{}:
      Key.block(k)
    case COther{}:
      SNone{}

def Ctx.keeps_keys(ctx: Ctx) -> Bool:
  match ctx:
    case COther{}:
      False{}
    case CRoot{}:
      True{}
    case CMsg{}:
      True{}
    case CUsage{}:
      True{}
    case CCC{}:
      True{}
    case CSTU{}:
      True{}
    case CBlock{}:
      True{}

# Starting any value in a slot resets what the slot holds, so a repeated key starts clean and
# a value of the wrong type leaves the field at its default, as the oracle's type checks do.
def Slot.reset(slot: Slot, l: L.Line) -> L.Line:
  match slot:
    case SType{}:
      L.Line.set_kind(l, T.KOther{})
    case STs{}:
      L.Line.set_has_ts(l, False{})
    case SSession{}:
      L.Line.set_session(l, "")
    case SCwd{}:
      L.Line.set_cwd(l, "")
    case SSide{}:
      L.Line.set_side(l, False{})
    case SMeta{}:
      L.Line.set_meta(l, False{})
    case SUuid{}:
      L.Line.set_has_uuid(l, False{})
    case SMsg{}:
      L.Line.reset_message(l)
    case SMid{}:
      L.Line.set_has_mid(l, False{})
    case SModel{}:
      L.Line.set_model(l, "")
    case SUsage{}:
      L.Line.reset_usage(l)
    case SContent{}:
      L.Line.reset_content(l)
    case SInp{}:
      L.Line.set_inp(l, 0n)
    case SOut{}:
      L.Line.set_out(l, 0n)
    case SCread{}:
      L.Line.set_cread(l, 0n)
    case SCwrite{}:
      L.Line.set_cwrite(l, 0n)
    case SCC{}:
      L.Line.reset_cc(l)
    case SSTU{}:
      L.Line.reset_stu(l)
    case SSpeed{}:
      L.Line.set_fast(l, False{})
    case SGeo{}:
      L.Line.set_geo(l, False{})
    case SC1h{}:
      L.Line.set_c1h(l, 0n)
    case SC5m{}:
      L.Line.set_c5m(l, 0n)
    case SWs{}:
      L.Line.set_ws(l, 0n)
    case SWf{}:
      L.Line.set_wf(l, 0n)
    case SBlock{}:
      L.Line.reset_block(l)
    case SBType{}:
      L.Line.set_b_type(l, T.BOther{})
    case SBId{}:
      L.Line.set_b_has_id(l, False{})
    case SBName{}:
      L.Line.set_b_name(l, "")
    case SBText{}:
      L.Line.set_b_human(l, False{})
    case SNone{}:
      l
    case SRoot{}:
      l

def Slot.obj_ctx(slot: Slot) -> Ctx:
  match slot:
    case SRoot{}:
      CRoot{}
    case SMsg{}:
      CMsg{}
    case SUsage{}:
      CUsage{}
    case SCC{}:
      CCC{}
    case SSTU{}:
      CSTU{}
    case SBlock{}:
      CBlock{}
    case _:
      COther{}

def Slot.is_content(slot: Slot) -> Bool:
  match slot:
    case SContent{}:
      True{}
    case _:
      False{}

def Slot.is_usage(slot: Slot) -> Bool:
  match slot:
    case SUsage{}:
      True{}
    case _:
      False{}

def Slot.is_root(slot: Slot) -> Bool:
  match slot:
    case SRoot{}:
      True{}
    case _:
      False{}

def Slot.cap(slot: Slot) -> Cap:
  match slot:
    case SType{}:
      CapFull{}
    case STs{}:
      CapFull{}
    case SSession{}:
      CapFull{}
    case SCwd{}:
      CapFull{}
    case SUuid{}:
      CapFull{}
    case SMid{}:
      CapFull{}
    case SModel{}:
      CapFull{}
    case SSpeed{}:
      CapFull{}
    case SGeo{}:
      CapFull{}
    case SBType{}:
      CapFull{}
    case SBId{}:
      CapFull{}
    case SBName{}:
      CapFull{}
    case SContent{}:
      CapHuman{}
    case SBText{}:
      CapHuman{}
    case _:
      CapNone{}

def Slot.set_true(slot: Slot, l: L.Line) -> L.Line:
  match slot:
    case SSide{}:
      L.Line.set_side(l, True{})
    case SMeta{}:
      L.Line.set_meta(l, True{})
    case _:
      l

def Slot.set_nat(slot: Slot, v: Nat, l: L.Line) -> L.Line:
  match slot:
    case SInp{}:
      L.Line.set_inp(l, v)
    case SOut{}:
      L.Line.set_out(l, v)
    case SCread{}:
      L.Line.set_cread(l, v)
    case SCwrite{}:
      L.Line.set_cwrite(l, v)
    case SC1h{}:
      L.Line.set_c1h(l, v)
    case SC5m{}:
      L.Line.set_c5m(l, v)
    case SWs{}:
      L.Line.set_ws(l, v)
    case SWf{}:
      L.Line.set_wf(l, v)
    case _:
      l

def Rec.pick(assistant: Bool, user: Bool) -> T.Rec:
  match assistant user:
    case True{} _:
      T.KAssistant{}
    case False{} True{}:
      T.KUser{}
    case False{} False{}:
      T.KOther{}

def Rec.of(+v: String) -> T.Rec:
  Rec.pick(String.eq(v, "assistant"), String.eq(v, "user"))

def BKind.pick(text: Bool, tool: Bool) -> T.BKind:
  match text tool:
    case True{} _:
      T.BText{}
    case False{} True{}:
      T.BTool{}
    case False{} False{}:
      T.BOther{}

def BKind.of(+v: String) -> T.BKind:
  BKind.pick(String.eq(v, "text"), String.eq(v, "tool_use"))

def Slot.set_str(slot: Slot, v: String, l: L.Line) -> L.Line:
  match slot:
    case SType{}:
      L.Line.set_kind(l, Rec.of(v))
    case STs{}:
      L.Line.set_has_ts(L.Line.set_ts(l, v), True{})
    case SSession{}:
      L.Line.set_session(l, v)
    case SCwd{}:
      L.Line.set_cwd(l, v)
    case SUuid{}:
      L.Line.set_has_uuid(L.Line.set_uuid(l, v), True{})
    case SMid{}:
      L.Line.set_has_mid(L.Line.set_mid(l, v), True{})
    case SModel{}:
      L.Line.set_model(l, v)
    case SSpeed{}:
      L.Line.set_fast(l, String.eq(v, "fast"))
    case SGeo{}:
      L.Line.set_geo(l, String.eq(v, "us"))
    case SBType{}:
      L.Line.set_b_type(l, BKind.of(v))
    case SBId{}:
      L.Line.set_b_has_id(L.Line.set_b_id(l, v), True{})
    case SBName{}:
      L.Line.set_b_name(l, v)
    case _:
      l

def Slot.set_human(slot: Slot, h: Bool, l: L.Line) -> L.Line:
  match slot:
    case SContent{}:
      L.Line.set_human(l, h)
    case SBText{}:
      L.Line.set_b_human(l, h)
    case _:
      l

# The prefixes Claude Code writes into user records it generates itself.
def Human.generated(+p: String) -> Bool:
  String.starts_with(p, "<local-command-stdout>") || String.starts_with(p, "<local-command-stderr>") || String.starts_with(p, "<local-command-caveat>") || String.starts_with(p, "[Request interrupted by user") || String.starts_with(p, "<task-notification>") || String.starts_with(p, "<bash-stdout>") || String.starts_with(p, "<bash-stderr>")

def Human.text(nonblank: Bool, p: String) -> Bool:
  nonblank && Bool.not(Human.generated(p))

# A content block ends: a tool_use block with a string id becomes a tool call; a text block
# with human text marks the message as a human prompt.
def Block.end.human(t: T.BKind, h: Bool, l: L.Line) -> L.Line:
  match t h:
    case T.BText{} True{}:
      L.Line.set_human(l, True{})
    case _ _:
      l

def Block.end.tool(t: T.BKind, has_id: Bool, id: String, name: String, tools: List<&2, T.Tool>) -> List<&2, T.Tool>:
  match t has_id:
    case T.BTool{} True{}:
      Con{T.Tool{id, name}, tools}
    case _ _:
      tools

def Block.end(l: L.Line) -> L.Line:
  match l:
    case L.Line{kind, has_ts, ts, session, cwd, side, meta, has_uuid, uuid, has_mid, mid, model, has_usage, inp, out, cread, cwrite, c1h, c5m, ws, wf, fast, geo, human, tools, +b_type, +b_has_id, +b_id, +b_name, +b_human}:
      Block.end.human(b_type, b_human, L.Line{kind, has_ts, ts, session, cwd, side, meta, has_uuid, uuid, has_mid, mid, model, has_usage, inp, out, cread, cwrite, c1h, c5m, ws, wf, fast, geo, human, Block.end.tool(b_type, b_has_id, b_id, b_name, tools), b_type, b_has_id, b_id, b_name, b_human})

def Ctx.end(ctx: Ctx, l: L.Line) -> L.Line:
  match ctx:
    case CBlock{}:
      Block.end(l)
    case _:
      l

# The machine
# -----------

# One line's worth of state, the characters left in the chunk, and the running fold.
@unsafe def Lex.scan(s: T.Txt, m: Mode, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match s:
    case T.TNil{}:
      (m, x, f)
    case T.TCon{+c, t}:
      Lex.char(U32.is_eq(c, 10), c, t, m, x, f)

@unsafe def Lex.char(nl: Bool, c: U32, t: T.Txt, m: Mode, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match nl:
    case True{}:
      Lex.eol(m, x, t, f)
    case False{}:
      Lex.mode(m, c, t, x, f)

def Mode.done(m: Mode) -> Bool:
  match m:
    case MDone{}:
      True{}
    case _:
      False{}

@unsafe def Lex.eol(m: Mode, x: Lx, t: T.Txt, f: F.Fold) -> Mode & Lx & F.Fold:
  match x:
    case Lx{stack, slot, key, val, ok, top, line}:
      Lex.scan(t, MValue{}, Lx.fresh(), F.Fold.line(Mode.done(m) && top, line, f))

@unsafe def Lex.err(t: T.Txt, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.scan(Lex.skip_line(t), MErr{}, Lx.fresh(), f)

@unsafe def Lex.mode(m: Mode, +c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match m:
    case MStr{cap, sb}:
      Lex.str(Chr.is_plain(c), cap, sb, c, t, x, f)
    case MEsc{cap, sb}:
      Lex.esc(cap, sb, c, t, x, f)
    case MHex{cap, sb, left, acc}:
      Lex.hex(cap, sb, left, acc, c, t, x, f)
    case MNum{nm}:
      Lex.num(nm, c, t, x, f)
    case MLit{rest, val}:
      Lex.lit(rest, val, c, t, x, f)
    case MErr{}:
      Lex.scan(Lex.skip_line(t), MErr{}, x, f)
    case MValue{}:
      Lex.ws(Chr.is_ws(c), MValue{}, c, t, x, f)
    case MValueOrEnd{}:
      Lex.ws(Chr.is_ws(c), MValueOrEnd{}, c, t, x, f)
    case MKey{}:
      Lex.ws(Chr.is_ws(c), MKey{}, c, t, x, f)
    case MKeyOrEnd{}:
      Lex.ws(Chr.is_ws(c), MKeyOrEnd{}, c, t, x, f)
    case MColon{}:
      Lex.ws(Chr.is_ws(c), MColon{}, c, t, x, f)
    case MAfter{}:
      Lex.ws(Chr.is_ws(c), MAfter{}, c, t, x, f)
    case MDone{}:
      Lex.ws(Chr.is_ws(c), MDone{}, c, t, x, f)

@unsafe def Lex.ws(ws: Bool, m: Mode, c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match ws:
    case True{}:
      Lex.scan(t, m, x, f)
    case False{}:
      Lex.structural(m, c, t, x, f)

@unsafe def Lex.structural(m: Mode, +c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match m:
    case MValue{}:
      Lex.value(c, t, x, f)
    case MValueOrEnd{}:
      Lex.value_or_end(U32.is_eq(c, 93), c, t, x, f)
    case MKey{}:
      Lex.key_start(U32.is_eq(c, 34), t, x, f)
    case MKeyOrEnd{}:
      Lex.key_or_end(U32.is_eq(c, 125), c, t, x, f)
    case MColon{}:
      Lex.colon(U32.is_eq(c, 58), t, x, f)
    case MAfter{}:
      Lex.after(c, t, x, f)
    case _:
      Lex.err(t, f)

# Values
# ------

# The slot of the value about to start, from the enclosing container.
def Lx.value_slot(stack: List<&2, Frame>) -> Slot:
  match stack:
    case Nil{}:
      SRoot{}
    case Con{FObj{ctx, slot}, rest}:
      slot
    case Con{FArr{True{}}, rest}:
      SBlock{}
    case Con{FArr{False{}}, rest}:
      SNone{}

def Lx.begin.go(+slot: Slot, stack: List<&2, Frame>, top: Bool, line: L.Line) -> Lx:
  Lx{stack, slot, False{}, 0n, False{}, top, Slot.reset(slot, line)}

def Lx.begin(x: Lx) -> Lx:
  match x:
    case Lx{+stack, slot, key, val, ok, top, line}:
      Lx.begin.go(Lx.value_slot(stack), stack, top, line)

# The value that just ended leaves the machine after a value: at the end of the line when it
# was the top-level value, otherwise before a ',' or a closer.
def Lx.after_value(stack: List<&2, Frame>) -> Mode:
  match stack:
    case Nil{}:
      MDone{}
    case Con{h, r}:
      MAfter{}

@unsafe def Lex.value(c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.value.go(Lx.begin(x), c, t, f)

@unsafe def Lex.value.go(x: Lx, c: U32, t: T.Txt, f: F.Fold) -> Mode & Lx & F.Fold:
  match x:
    case Lx{stack, slot, key, val, ok, top, line}:
      Lex.value.kind(c, t, stack, slot, top, line, f)

@unsafe def Lex.value.kind(+c: U32, t: T.Txt, stack: List<&2, Frame>, slot: Slot, top: Bool, line: L.Line, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.value.obj(U32.is_eq(c, 123), c, t, stack, slot, top, line, f)

@unsafe def Lex.value.obj(hit: Bool, +c: U32, t: T.Txt, stack: List<&2, Frame>, +slot: Slot, top: Bool, line: L.Line, f: F.Fold) -> Mode & Lx & F.Fold:
  match hit:
    case True{}:
      Lex.scan(t, MKeyOrEnd{}, Lx{Con{FObj{Slot.obj_ctx(slot), SNone{}}, stack}, slot, False{}, 0n, False{}, top || Slot.is_root(slot), Lex.usage_on(Slot.is_usage(slot), line)}, f)
    case False{}:
      Lex.value.arr(U32.is_eq(c, 91), c, t, stack, slot, top, line, f)

def Lex.usage_on(hit: Bool, l: L.Line) -> L.Line:
  match hit:
    case True{}:
      L.Line.set_has_usage(l, True{})
    case False{}:
      l

@unsafe def Lex.value.arr(hit: Bool, +c: U32, t: T.Txt, stack: List<&2, Frame>, +slot: Slot, top: Bool, line: L.Line, f: F.Fold) -> Mode & Lx & F.Fold:
  match hit:
    case True{}:
      Lex.scan(t, MValueOrEnd{}, Lx{Con{FArr{Slot.is_content(slot)}, stack}, slot, False{}, 0n, False{}, top, line}, f)
    case False{}:
      Lex.value.str(U32.is_eq(c, 34), c, t, stack, slot, top, line, f)

@unsafe def Lex.value.str(hit: Bool, +c: U32, t: T.Txt, stack: List<&2, Frame>, +slot: Slot, top: Bool, line: L.Line, f: F.Fold) -> Mode & Lx & F.Fold:
  match hit:
    case True{}:
      Lex.scan(t, MStr{Slot.cap(slot), SB.empty()}, Lx{stack, slot, False{}, 0n, False{}, top, line}, f)
    case False{}:
      Lex.value.num(Chr.is_digit(c), U32.is_eq(c, 45), c, t, stack, slot, top, line, f)

@unsafe def Lex.value.num(digit: Bool, minus: Bool, +c: U32, t: T.Txt, stack: List<&2, Frame>, slot: Slot, top: Bool, line: L.Line, f: F.Fold) -> Mode & Lx & F.Fold:
  match digit minus:
    case True{} _:
      Lex.scan(t, MNum{Lex.num.first(U32.is_eq(c, 48))}, Lx{stack, slot, False{}, U32.to_nat((c - 48 : U32)), True{}, top, line}, f)
    case False{} True{}:
      Lex.scan(t, MNum{NMinus{}}, Lx{stack, slot, False{}, 0n, False{}, top, line}, f)
    case False{} False{}:
      Lex.value.lit(c, t, stack, slot, top, line, f)

def Lex.num.first(zero: Bool) -> NMode:
  match zero:
    case True{}:
      NZero{}
    case False{}:
      NInt{}

@unsafe def Lex.value.lit(+c: U32, t: T.Txt, stack: List<&2, Frame>, slot: Slot, top: Bool, line: L.Line, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.value.lit.go(U32.is_eq(c, 116), U32.is_eq(c, 102), U32.is_eq(c, 110), t, stack, slot, top, line, f)

@unsafe def Lex.value.lit.go(tr: Bool, fa: Bool, nu: Bool, t: T.Txt, stack: List<&2, Frame>, slot: Slot, top: Bool, line: L.Line, f: F.Fold) -> Mode & Lx & F.Fold:
  match tr fa nu:
    case True{} _ _:
      Lex.scan(t, MLit{"rue", True{}}, Lx{stack, slot, False{}, 0n, False{}, top, line}, f)
    case False{} True{} _:
      Lex.scan(t, MLit{"alse", False{}}, Lx{stack, slot, False{}, 0n, False{}, top, line}, f)
    case False{} False{} True{}:
      Lex.scan(t, MLit{"ull", False{}}, Lx{stack, slot, False{}, 0n, False{}, top, line}, f)
    case False{} False{} False{}:
      Lex.err(t, f)

@unsafe def Lex.value_or_end(close: Bool, c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match close:
    case True{}:
      Lex.close(False{}, t, x, f)
    case False{}:
      Lex.value(c, t, x, f)

# Literals: true sets a boolean slot; false and null leave the reset default.
@unsafe def Lex.lit(rest: String, v: Bool, c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match rest:
    case SNil{}:
      Lex.err(t, f)
    case SCon{Chr{e}, r}:
      Lex.lit.step(U32.is_eq(c, e), r, v, t, x, f)

@unsafe def Lex.lit.step(same: Bool, r: String, v: Bool, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match same:
    case False{}:
      Lex.err(t, f)
    case True{}:
      Lex.lit.more(r, v, t, x, f)

@unsafe def Lex.lit.more(r: String, v: Bool, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match r:
    case SCon{h, rr}:
      Lex.scan(t, MLit{SCon{h, rr}, v}, x, f)
    case SNil{}:
      Lex.lit.end(v, t, x, f)

@unsafe def Lex.lit.end(v: Bool, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match x:
    case Lx{+stack, slot, key, val, ok, top, line}:
      Lex.scan(t, Lx.after_value(stack), Lx{stack, SNone{}, False{}, 0n, False{}, top, Lex.lit.apply(v, slot, line)}, f)

def Lex.lit.apply(v: Bool, slot: Slot, l: L.Line) -> L.Line:
  match v:
    case True{}:
      Slot.set_true(slot, l)
    case False{}:
      l

def Num.push.check(+v: Nat) -> Nat & Bool:
  (v, Nat.is_le(v, 1000000000n))

def Num.push.go(fits: Bool, val: Nat, d: Nat) -> Nat & Bool:
  match fits:
    case True{}:
      Num.push.check((val * 10n + d : Nat))
    case False{}:
      (val, False{})

# Numbers
# -------

# A digit extends an integer. A count above 10^9 is not a usage figure (no response comes near
# a billion tokens) and reads as 0, as in the oracle; accumulation stops there, far below the
# 2^48 at which a Nat aborts.
def Num.push(ok: Bool, +val: Nat, d: Nat) -> Nat & Bool:
  match ok:
    case True{}:
      Num.push.go(Nat.is_le(val, 100000000n), val, d)
    case False{}:
      (val, False{})

@unsafe def Lex.num(nm: NMode, +c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.num.go(nm, Chr.is_digit(c), U32.is_eq(c, 46), U32.is_eq(c, 101) || U32.is_eq(c, 69), U32.is_eq(c, 43) || U32.is_eq(c, 45), c, t, x, f)

@unsafe def Lex.num.go(nm: NMode, digit: Bool, dot: Bool, exp: Bool, sign: Bool, c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match nm digit dot exp sign:
    case NMinus{} True{} _ _ _:
      Lex.scan(t, MNum{Lex.num.first(U32.is_eq(c, 48))}, x, f)
    case NMinus{} False{} _ _ _:
      Lex.err(t, f)
    case NZero{} _ True{} _ _:
      Lex.scan(t, MNum{NDot{}}, Lx.not_int(x), f)
    case NZero{} _ False{} True{} _:
      Lex.scan(t, MNum{NE{}}, Lx.not_int(x), f)
    case NZero{} _ False{} False{} _:
      Lex.num.end(c, t, x, f)
    case NInt{} True{} _ _ _:
      Lex.scan(t, MNum{NInt{}}, Lx.digit(U32.to_nat((c - 48 : U32)), x), f)
    case NInt{} False{} True{} _ _:
      Lex.scan(t, MNum{NDot{}}, Lx.not_int(x), f)
    case NInt{} False{} False{} True{} _:
      Lex.scan(t, MNum{NE{}}, Lx.not_int(x), f)
    case NInt{} False{} False{} False{} _:
      Lex.num.end(c, t, x, f)
    case NDot{} True{} _ _ _:
      Lex.scan(t, MNum{NFrac{}}, x, f)
    case NDot{} False{} _ _ _:
      Lex.err(t, f)
    case NFrac{} True{} _ _ _:
      Lex.scan(t, MNum{NFrac{}}, x, f)
    case NFrac{} False{} _ True{} _:
      Lex.scan(t, MNum{NE{}}, x, f)
    case NFrac{} False{} _ False{} _:
      Lex.num.end(c, t, x, f)
    case NE{} True{} _ _ _:
      Lex.scan(t, MNum{NExp{}}, x, f)
    case NE{} False{} _ _ True{}:
      Lex.scan(t, MNum{NESign{}}, x, f)
    case NE{} False{} _ _ False{}:
      Lex.err(t, f)
    case NESign{} True{} _ _ _:
      Lex.scan(t, MNum{NExp{}}, x, f)
    case NESign{} False{} _ _ _:
      Lex.err(t, f)
    case NExp{} True{} _ _ _:
      Lex.scan(t, MNum{NExp{}}, x, f)
    case NExp{} False{} _ _ _:
      Lex.num.end(c, t, x, f)

def Lx.not_int(x: Lx) -> Lx:
  match x:
    case Lx{stack, slot, key, val, ok, top, line}:
      Lx{stack, slot, key, val, False{}, top, line}

def Lx.digit.go(vo: Nat & Bool, stack: List<&2, Frame>, slot: Slot, top: Bool, line: L.Line) -> Lx:
  (val, ok) = vo
  Lx{stack, slot, False{}, val, ok, top, line}

def Lx.digit(d: Nat, x: Lx) -> Lx:
  match x:
    case Lx{stack, slot, key, val, ok, top, line}:
      Lx.digit.go(Num.push(ok, val, d), stack, slot, top, line)

def Num.apply(ok: Bool, slot: Slot, val: Nat, l: L.Line) -> L.Line:
  match ok:
    case True{}:
      Slot.set_nat(slot, val, l)
    case False{}:
      l

# The character after a number ends it and is then read as the character after a value.
@unsafe def Lex.num.end(c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match x:
    case Lx{+stack, slot, key, val, ok, top, line}:
      Lex.mode(Lx.after_value(stack), c, t, Lx{stack, SNone{}, False{}, 0n, False{}, top, Num.apply(ok, slot, val, line)}, f)

# Strings
# -------

@unsafe def Lex.str(plain: Bool, +cap: Cap, sb: SB, +c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match plain cap:
    case True{} CapNone{}:
      Lex.scan(Lex.skip(t), MStr{CapNone{}, sb}, x, f)
    case True{} CapDone{}:
      Lex.scan(Lex.skip(t), MStr{CapDone{}, sb}, x, f)
    case True{} +cap:
      Lex.str.pushed(cap, SB.push(cap, c, sb), t, x, f)
    case False{} cap:
      Lex.str.special(U32.is_eq(c, 34), U32.is_eq(c, 92), cap, sb, t, x, f)

@unsafe def Lex.str.pushed(cap: Cap, +sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.scan(t, MStr{Cap.next(cap, sb), sb}, x, f)

@unsafe def Lex.str.special(quote: Bool, slash: Bool, cap: Cap, sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match quote slash:
    case True{} _:
      Lex.str.end(cap, sb, t, x, f)
    case False{} True{}:
      Lex.scan(t, MEsc{cap, sb}, x, f)
    case False{} False{}:
      Lex.err(t, f)

def Cap.keeps(cap: Cap) -> Bool:
  match cap:
    case CapFull{}:
      True{}
    case CapHuman{}:
      True{}
    case CapNone{}:
      False{}
    case CapDone{}:
      False{}

def Cap.next.human(done: Bool) -> Cap:
  match done:
    case True{}:
      CapDone{}
    case False{}:
      CapHuman{}

# The human test is settled once 32 characters are kept and one of them is not blank.
def Cap.next(+cap: Cap, sb: SB) -> Cap:
  match cap sb:
    case CapHuman{} SB{buf, n, nb, hi}:
      Cap.next.human(Nat.is_ge(n, 32n) && nb)
    case cap _:
      cap

def SB.add.go(keep: Bool, c: U32, buf: String, n: Nat, nb: Bool, hi: U32) -> SB:
  match keep:
    case True{}:
      SB{SCon{Chr{c}, buf}, (n + 1n : Nat), nb, hi}
    case False{}:
      SB{buf, n, nb, hi}

def Cap.full(cap: Cap) -> Bool:
  match cap:
    case CapFull{}:
      True{}
    case _:
      False{}

# Keeps one character: always for a full capture, the first 32 for the human test.
def SB.add(cap: Cap, +c: U32, sb: SB) -> SB:
  match sb:
    case SB{buf, +n, nb, hi}:
      SB.add.go(Cap.full(cap) || Nat.is_lt(n, 32n), c, buf, n, nb || Bool.not(Chr.is_blank(c)), hi)

def SB.flush.go(none: Bool, cap: Cap, hi: U32, sb: SB) -> SB:
  match none:
    case True{}:
      sb
    case False{}:
      SB.add(cap, hi, sb)

# A pending high surrogate with no low half after it stands alone, as Python keeps it.
def SB.flush(cap: Cap, sb: SB) -> SB:
  match sb:
    case SB{buf, n, nb, +hi}:
      SB.flush.go(U32.is_eq(hi, 0), cap, hi, SB{buf, n, nb, 0})

def SB.push(+cap: Cap, c: U32, sb: SB) -> SB:
  SB.add(cap, c, SB.flush(cap, sb))

def SB.set_hi(u: U32, sb: SB) -> SB:
  match sb:
    case SB{buf, n, nb, hi}:
      SB{buf, n, nb, u}

def SB.join(cap: Cap, lo: U32, sb: SB) -> SB:
  match sb:
    case SB{buf, n, nb, hi}:
      SB.add(cap, (65536 + (hi - 55296) * 1024 + (lo - 56320) : U32), SB{buf, n, nb, 0})

def SB.unit.go(high: Bool, low: Bool, cap: Cap, u: U32, sb: SB) -> SB:
  match high low:
    case True{} _:
      SB.set_hi(u, SB.flush(cap, sb))
    case False{} True{}:
      SB.join(cap, u, sb)
    case False{} False{}:
      SB.push(cap, u, sb)

# A \u escape: a high surrogate waits for its low half; a low half after one joins it.
def SB.unit(cap: Cap, +u: U32, sb: SB) -> SB:
  match sb:
    case SB{buf, n, nb, +hi}:
      SB.unit.go(U32.is_ge(u, 55296) && U32.is_le(u, 56319), U32.is_ge(u, 56320) && U32.is_le(u, 57343) && U32.is_ne(hi, 0), cap, u, SB{buf, n, nb, hi})

# The character a one-letter escape stands for, or 0 when the escape is invalid.
def Lex.esc.pick(hit: Bool, a: U32, b: U32) -> U32:
  match hit:
    case True{}:
      a
    case False{}:
      b

def Lex.esc.char(+c: U32) -> U32:
  Lex.esc.pick(U32.is_eq(c, 34) || U32.is_eq(c, 92) || U32.is_eq(c, 47), c, Lex.esc.pick(U32.is_eq(c, 98), 8, Lex.esc.pick(U32.is_eq(c, 102), 12, Lex.esc.pick(U32.is_eq(c, 110), 10, Lex.esc.pick(U32.is_eq(c, 114), 13, Lex.esc.pick(U32.is_eq(c, 116), 9, 0))))))

@unsafe def Lex.esc(cap: Cap, sb: SB, +c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.esc.go(U32.is_eq(c, 117), Lex.esc.char(c), cap, sb, t, x, f)

@unsafe def Lex.esc.go(u: Bool, +e: U32, cap: Cap, sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match u:
    case True{}:
      Lex.scan(t, MHex{cap, sb, 4n, 0}, x, f)
    case False{}:
      Lex.esc.one(U32.is_eq(e, 0), e, cap, sb, t, x, f)

@unsafe def Lex.esc.one(bad: Bool, e: U32, +cap: Cap, sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match bad:
    case True{}:
      Lex.err(t, f)
    case False{}:
      Lex.str.kept(Cap.keeps(cap), cap, e, sb, t, x, f)

@unsafe def Lex.str.kept(keep: Bool, +cap: Cap, e: U32, sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match keep:
    case True{}:
      Lex.str.pushed(cap, SB.push(cap, e, sb), t, x, f)
    case False{}:
      Lex.scan(t, MStr{cap, sb}, x, f)

# The value of a hexadecimal digit, or 16 when it is not one.
def Hex.digit(+c: U32) -> U32:
  Lex.esc.pick(Chr.is_digit(c), (c - 48 : U32), Lex.esc.pick(U32.is_ge(c, 97) && U32.is_le(c, 102), (c - 87 : U32), Lex.esc.pick(U32.is_ge(c, 65) && U32.is_le(c, 70), (c - 55 : U32), 16)))

@unsafe def Lex.hex(cap: Cap, sb: SB, left: Nat, acc: U32, c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.hex.go(Hex.digit(c), cap, sb, left, acc, t, x, f)

@unsafe def Lex.hex.go(+d: U32, cap: Cap, sb: SB, left: Nat, acc: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.hex.step(U32.is_eq(d, 16), left, d, cap, sb, acc, t, x, f)

@unsafe def Lex.hex.step(bad: Bool, left: Nat, d: U32, +cap: Cap, sb: SB, acc: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match bad left:
    case True{} _:
      Lex.err(t, f)
    case False{} 1n:
      Lex.hex.unit(Cap.keeps(cap), cap, (acc * 16 + d : U32), sb, t, x, f)
    case False{} 1n+p:
      Lex.scan(t, MHex{cap, sb, p, (acc * 16 + d : U32)}, x, f)
    case False{} 0n:
      Lex.err(t, f)

@unsafe def Lex.hex.unit(keep: Bool, +cap: Cap, u: U32, sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match keep:
    case True{}:
      Lex.str.pushed(cap, SB.unit(cap, u, sb), t, x, f)
    case False{}:
      Lex.scan(t, MStr{cap, sb}, x, f)

# A string ends: a key selects the slot of the value after the colon; a value is stored.
@unsafe def Lex.str.end(+cap: Cap, sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.str.end.go(cap, SB.flush(cap, sb), t, x, f)

@unsafe def Lex.str.end.go(cap: Cap, sb: SB, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match sb x:
    case SB{buf, n, nb, hi} Lx{stack, slot, key, val, ok, top, line}:
      Lex.str.end.kind(key, cap, stack, slot, String.reverse(buf), nb, top, line, t, f)

@unsafe def Lex.str.end.kind(key: Bool, cap: Cap, stack: List<&2, Frame>, slot: Slot, text: String, nonblank: Bool, top: Bool, line: L.Line, t: T.Txt, f: F.Fold) -> Mode & Lx & F.Fold:
  match key:
    case True{}:
      Lex.scan(t, MColon{}, Lx{Lex.key.set(stack, text), SNone{}, False{}, 0n, False{}, top, line}, f)
    case False{}:
      Lex.str.value(cap, stack, slot, text, nonblank, top, line, t, f)

@unsafe def Lex.str.value(cap: Cap, +stack: List<&2, Frame>, slot: Slot, text: String, nonblank: Bool, top: Bool, line: L.Line, t: T.Txt, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.scan(t, Lx.after_value(stack), Lx{stack, SNone{}, False{}, 0n, False{}, top, Lex.str.apply(cap, slot, text, nonblank, line)}, f)

def Lex.str.apply(cap: Cap, slot: Slot, text: String, nonblank: Bool, l: L.Line) -> L.Line:
  match cap:
    case CapFull{}:
      Slot.set_str(slot, text, l)
    case CapHuman{}:
      Slot.set_human(slot, Human.text(nonblank, text), l)
    case CapDone{}:
      Slot.set_human(slot, Human.text(nonblank, text), l)
    case CapNone{}:
      l

def Lex.key.set(stack: List<&2, Frame>, text: String) -> List<&2, Frame>:
  match stack:
    case Con{FObj{+ctx, old}, rest}:
      Con{FObj{ctx, Key.slot(ctx, text)}, rest}
    case other:
      other

# Keys
# ----

@unsafe def Lex.key_start(quote: Bool, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match quote:
    case True{}:
      Lex.key_begin(t, x, f)
    case False{}:
      Lex.err(t, f)

@unsafe def Lex.key_begin(t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match x:
    case Lx{+stack, slot, key, val, ok, top, line}:
      Lex.scan(t, MStr{Lex.key.cap(stack), SB.empty()}, Lx{stack, SNone{}, True{}, 0n, False{}, top, line}, f)

def Lex.key.cap.go(keep: Bool) -> Cap:
  match keep:
    case True{}:
      CapFull{}
    case False{}:
      CapNone{}

def Lex.key.cap(stack: List<&2, Frame>) -> Cap:
  match stack:
    case Con{FObj{ctx, slot}, rest}:
      Lex.key.cap.go(Ctx.keeps_keys(ctx))
    case _:
      CapNone{}

@unsafe def Lex.key_or_end(close: Bool, c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match close:
    case True{}:
      Lex.close(True{}, t, x, f)
    case False{}:
      Lex.key_start(U32.is_eq(c, 34), t, x, f)

@unsafe def Lex.colon(hit: Bool, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match hit:
    case True{}:
      Lex.scan(t, MValue{}, x, f)
    case False{}:
      Lex.err(t, f)

# After a value
# -------------

@unsafe def Lex.after(+c: U32, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  Lex.after.go(U32.is_eq(c, 44), U32.is_eq(c, 125), U32.is_eq(c, 93), t, x, f)

@unsafe def Lex.after.go(comma: Bool, rbrace: Bool, rbrack: Bool, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match comma rbrace rbrack:
    case True{} _ _:
      Lex.comma(t, x, f)
    case False{} True{} _:
      Lex.close(True{}, t, x, f)
    case False{} False{} True{}:
      Lex.close(False{}, t, x, f)
    case False{} False{} False{}:
      Lex.err(t, f)

@unsafe def Lex.comma(t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match x:
    case Lx{+stack, slot, key, val, ok, top, line}:
      Lex.scan(t, Lex.comma.mode(stack), Lx{stack, slot, key, val, ok, top, line}, f)

def Lex.comma.mode(stack: List<&2, Frame>) -> Mode:
  match stack:
    case Con{FObj{ctx, slot}, rest}:
      MKey{}
    case Con{FArr{c}, rest}:
      MValue{}
    case Nil{}:
      MErr{}

# A closer must match the innermost container. Closing an object runs its end hook.
@unsafe def Lex.close(obj: Bool, t: T.Txt, x: Lx, f: F.Fold) -> Mode & Lx & F.Fold:
  match x:
    case Lx{stack, slot, key, val, ok, top, line}:
      Lex.close.go(obj, stack, top, line, t, f)

@unsafe def Lex.close.go(obj: Bool, stack: List<&2, Frame>, top: Bool, line: L.Line, t: T.Txt, f: F.Fold) -> Mode & Lx & F.Fold:
  match obj stack:
    case True{} Con{FObj{ctx, slot}, +rest}:
      Lex.scan(t, Lx.after_value(rest), Lx{rest, SNone{}, False{}, 0n, False{}, top, Ctx.end(ctx, line)}, f)
    case False{} Con{FArr{c}, +rest}:
      Lex.scan(t, Lx.after_value(rest), Lx{rest, SNone{}, False{}, 0n, False{}, top, line}, f)
    case _ _:
      Lex.err(t, f)
