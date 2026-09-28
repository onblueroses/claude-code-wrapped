import Base

# The record type of a transcript line, read from its top-level "type".
type Rec is Data:
  KOther{}
  KAssistant{}
  KUser{}

# The "type" of a message content block.
type BKind is Data:
  BOther{}
  BText{}
  BTool{}

# A tool_use content block.
type Tool is Data:
  Tool{id: String, name: String}

# List prices in microdollars per million tokens, from pricing/prices.tsv.
type Price is Data:
  Price{model: String, fast: Bool, inp: Nat, w5: Nat, w1: Nat, read: Nat, out: Nat}

# Text read from a transcript, one code point per cell. The lexer consumes it without ever
# reusing a cell, so the runtime does not reference-count it (a String would be, since the
# program reuses Strings elsewhere).
type Txt is Data:
  TNil{}
  TCon{head: U32, tail: Txt}
