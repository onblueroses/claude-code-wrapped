import Base
import ./fold.bend as F
import ./bill.bend as B

# The laws the metrics rest on. Each is an open claim here; PROOF.bend proves it, and
# `bend PROOF.bend --verdict` re-checks every proof with Bend's small Lean-verified kernel.

# Deduplication and sharding
# --------------------------
# Claude Code writes one API response several times (once per streamed content block, again
# when a session is resumed or forked), and the engine reads files in whatever order and in
# whatever shards it likes. Every copy of a response is folded into one record with
# Group.merge. These three laws make the result independent of how many copies there are,
# which file each was in, and the order and grouping in which they were read.

law group_merge_comm:
  for a: F.Group
  for b: F.Group
  {F.Group.merge(a, b) == F.Group.merge(b, a) : F.Group}

law group_merge_assoc:
  for a: F.Group
  for b: F.Group
  for c: F.Group
  {F.Group.merge(F.Group.merge(a, b), c) == F.Group.merge(a, F.Group.merge(b, c)) : F.Group}

law group_merge_idem:
  for a: F.Group
  {F.Group.merge(a, a) == a : F.Group}

# Tool calls are attributed to their response by the least [name, message id] among copies.

law tool_merge_comm:
  for +a: List<&2, String>
  for +b: List<&2, String>
  {F.Lst.min(a, b) == F.Lst.min(b, a) : List<&2, String>}

law tool_merge_assoc:
  for +a: List<&2, String>
  for +b: List<&2, String>
  for +c: List<&2, String>
  {F.Lst.min(F.Lst.min(a, b), c) == F.Lst.min(a, F.Lst.min(b, c)) : List<&2, String>}

law tool_merge_idem:
  for +a: List<&2, String>
  {F.Lst.min(a, a) == a : List<&2, String>}

# A human prompt copied into several files counts once, at its earliest timestamp.

law prompt_merge_comm:
  for +a: String
  for +b: String
  {F.String.min(a, b) == F.String.min(b, a) : String}

law prompt_merge_assoc:
  for +a: String
  for +b: String
  for +c: String
  {F.String.min(F.String.min(a, b), c) == F.String.min(a, F.String.min(b, c)) : String}

law prompt_merge_idem:
  for +a: String
  {F.String.min(a, a) == a : String}

# Billing
# -------
# A response's cache writes are split into the 1-hour and 5-minute durations from its
# usage breakdown; the total is authoritative. Every cache-write token is billed exactly once.

law cache_write_billed_once:
  for inp: Nat
  for out: Nat
  for cread: Nat
  for +cwrite: Nat
  for +c1h: Nat
  {B.Toks.writes(B.Toks.of(inp, out, cread, cwrite, c1h)) == cwrite : Nat}
