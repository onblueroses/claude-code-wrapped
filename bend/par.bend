import Base
import ./types.bend as T
import ./effects.bend as E
import ./fold.bend as F
import ./lex.bend as X

# Parallel reading. IO collects a batch of chunks, each a run of whole lines from some file;
# a balanced tree of parallel calls then lexes every chunk into its own fold and merges the
# folds pairwise. Chunks start at line boundaries, so they are independent, and fold merge is
# commutative, associative and idempotent (LAWS.bend), so neither the batching nor the shape
# of the tree can change the result.

# Chunk size requested per read, and chunks per batch.

def Par.want() -> U32:
  524288

def Par.batch() -> Nat:
  64n

@unsafe def Par.lex(s: T.Txt) -> F.Fold:
  Par.lex.fin(X.Lex.scan(s, X.MValue{}, X.Lx.fresh(), F.Fold.empty()))

def Par.lex.fin(r: X.Mode & X.Lx & F.Fold) -> F.Fold:
  (m, x, f) = r
  f

def Par.split.cons(h: T.Txt, r: List<&2, T.Txt> & List<&2, T.Txt>) -> List<&2, T.Txt> & List<&2, T.Txt>:
  (a, b) = r
  (Con{h, a}, b)

def Par.split(n: Nat, +xs: List<&2, T.Txt>) -> List<&2, T.Txt> & List<&2, T.Txt>:
  match n xs:
    case 0n xs:
      (Nil{}, xs)
    case 1n+p Nil{}:
      (Nil{}, Nil{})
    case 1n+p Con{h, t}:
      Par.split.cons(h, Par.split(p, t))

@unsafe def Par.tree(+n: Nat, +xs: List<&2, T.Txt>) -> F.Fold:
  match n xs:
    case 0n _:
      F.Fold.empty()
    case 1n Con{s, rest}:
      Par.lex(s)
    case 1n Nil{}:
      F.Fold.empty()
    case +n xs:
      Par.tree.halves(Nat.div(n, 2n), n, Par.split(Nat.div(n, 2n), xs))

@unsafe def Par.tree.halves(+h: Nat, n: Nat, parts: List<&2, T.Txt> & List<&2, T.Txt>) -> F.Fold:
  (left, right) = parts
  a b = Par.tree(h, left) Par.tree((n - h : Nat), right)
  F.Fold.merge(a, b)

# The IO side: files still to read, the open file, and the batch being collected.
type Rd is Type:
  Rd{files: List<&2, String>, open: Maybe<&1, File>, batch: List<&2, T.Txt>, count: Nat, fold: F.Fold}

@unsafe def Rd.flush(r: Rd) -> Rd:
  match r:
    case Rd{files, open, batch, count, fold}:
      Rd{files, open, Nil{}, 0n, F.Fold.merge(fold, Par.tree(count, batch))}

@unsafe def Rd.add(s: T.Txt, r: Rd) -> Rd:
  match r:
    case Rd{files, open, batch, count, fold}:
      Rd.add.check(Rd{files, open, Con{s, batch}, (count + 1n : Nat), fold})

@unsafe def Rd.add.check(r: Rd) -> Rd:
  match r:
    case Rd{files, open, batch, +count, fold}:
      Rd.add.full(Nat.is_ge(count, Par.batch()), Rd{files, open, batch, count, fold})

@unsafe def Rd.add.full(full: Bool, r: Rd) -> Rd:
  match full:
    case True{}:
      Rd.flush(r)
    case False{}:
      r

@unsafe def Rd.run(r: Rd) -> IO(F.Fold):
  match r:
    case Rd{files, Some{file}, batch, count, fold}:
      IO.bind(File & Result<&1, &1, U32 & String, T.Txt>, F.Fold, E.Lines.next(file, Par.want()), got => Rd.got(got, files, batch, count, fold))
    case Rd{Con{path, rest}, None{}, batch, count, fold}:
      IO.bind(Result<&1, &1, U32 & String, File>, F.Fold, E.Lines.open(path), o => Rd.opened(o, rest, batch, count, fold))
    case Rd{Nil{}, None{}, batch, count, fold}:
      IO.pure(F.Fold, Rd.done(Rd.flush(Rd{Nil{}, None{}, batch, count, fold})))

@unsafe def Rd.done(r: Rd) -> F.Fold:
  match r:
    case Rd{files, open, batch, count, fold}:
      fold

@unsafe def Rd.opened(o: Result<&1, &1, U32 & String, File>, files: List<&2, String>, batch: List<&2, T.Txt>, count: Nat, fold: F.Fold) -> IO(F.Fold):
  match o:
    case Done{file}:
      Rd.run(Rd{files, Some{file}, batch, count, F.Fold.file(fold)})
    case Fail{e}:
      IO.bind(Unit, F.Fold, IO.print_err("ccwrapped: a transcript could not be opened"), u => Rd.run(Rd{files, None{}, batch, count, fold}))

@unsafe def Rd.got(got: File & Result<&1, &1, U32 & String, T.Txt>, files: List<&2, String>, batch: List<&2, T.Txt>, count: Nat, fold: F.Fold) -> IO(F.Fold):
  (file, res) = got
  Rd.chunk(res, file, files, batch, count, fold)

@unsafe def Rd.chunk(res: Result<&1, &1, U32 & String, T.Txt>, file: File, files: List<&2, String>, batch: List<&2, T.Txt>, count: Nat, fold: F.Fold) -> IO(F.Fold):
  match res:
    case Done{T.TNil{}}:
      IO.bind(Unit, F.Fold, File.close(file), u => Rd.run(Rd{files, None{}, batch, count, fold}))
    case Done{T.TCon{h, t}}:
      Rd.run(Rd.add(T.TCon{h, t}, Rd{files, Some{file}, batch, count, fold}))
    case Fail{e}:
      IO.bind(Unit, F.Fold, File.close(file), u => IO.bind(Unit, F.Fold, IO.print_err("ccwrapped: a transcript could not be read"), v => Rd.run(Rd{files, None{}, batch, count, fold})))

@unsafe def Par.read(files: List<&2, String>, fold: F.Fold) -> IO(F.Fold):
  Rd.run(Rd{files, None{}, Nil{}, 0n, fold})
