import Base
import ./types.bend as T
import ./effects.bend as E
import ./fold.bend as F
import ./par.bend as P
import ./ser.bend as S
import ./report.bend as R
import ./out.bend as O
import ./view.bend as V

# ccwrapped: Claude Code usage metrics from local transcripts.
#
#   ccwrapped [--year YYYY] [--tz ZONE] [--root DIR]... [--jobs N]
#
# The parent lists the transcripts, drops those last modified before the year (they cannot
# hold a response of it: every copy of a response keeps its original timestamp), splits the
# rest into size-balanced shards and runs one child process per shard. Each child folds its
# files and prints the fold; the parent merges the folds, which the fold laws make independent
# of the sharding, and prints the metrics.

# Options
# -------

# fmt: 0 terminal report, 1 JSON metrics, 2 Markdown report, 3 HTML slides, 4 share card.
# nfmt counts the format flags given, so that two of them can be refused. info: 1 --help,
# 2 --version. err holds the first problem with the command line; parsing stops there.

type Opts is Data:
  Opts{year: Nat, zone: String, roots: List<&2, String>, jobs: Nat, shard: Bool, fmt: Nat, nfmt: Nat, share: Bool, info: Nat, err: String}

def Opts.empty() -> Opts:
  Opts{0n, "", Nil{}, 0n, False{}, 0n, 0n, False{}, 0n, ""}

def Opts.set_year(o: Opts, v: Nat) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{v, zone, roots, jobs, shard, fmt, nfmt, share, info, err}

def Opts.set_zone(o: Opts, v: String) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, v, roots, jobs, shard, fmt, nfmt, share, info, err}

def Opts.add_root(o: Opts, v: String) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, zone, List.append(&2, String, roots, [v]), jobs, shard, fmt, nfmt, share, info, err}

def Opts.set_jobs(o: Opts, v: Nat) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, zone, roots, v, shard, fmt, nfmt, share, info, err}

def Opts.set_shard(o: Opts) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, zone, roots, jobs, True{}, fmt, nfmt, share, info, err}

def Opts.set_fmt(o: Opts, v: Nat) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, zone, roots, jobs, shard, v, (nfmt + 1n : Nat), share, info, err}

def Opts.set_share(o: Opts) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, zone, roots, jobs, shard, fmt, nfmt, True{}, info, err}

def Opts.set_info(o: Opts, v: Nat) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, v, err}

def Opts.fail(o: Opts, v: String) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, v}

type Flag is Data:
  Flag{name: String, code: Nat}

# Codes 1-4 take a value; 10 and up are switches; 0 is not an option.
def Flags() -> List<&2, Flag>:
  [Flag{"--year", 1n}, Flag{"--tz", 2n}, Flag{"--root", 3n}, Flag{"--jobs", 4n}, Flag{"--shard", 10n}, Flag{"--json", 11n}, Flag{"--markdown", 12n}, Flag{"--html", 13n}, Flag{"--card", 14n}, Flag{"--share", 15n}, Flag{"--help", 16n}, Flag{"-h", 16n}, Flag{"--version", 17n}]

def Flag.pick(hit: Bool, c: Nat, later: Nat) -> Nat:
  match hit:
    case True{}:
      c
    case False{}:
      later

def Flag.find(fs: List<&2, Flag>, +f: String) -> Nat:
  match fs:
    case Nil{}:
      0n
    case Con{Flag{n, c}, rest}:
      Flag.pick(String.eq(n, f), c, Flag.find(rest, f))

def Digits.all.go(s: String) -> Bool:
  match s:
    case SNil{}:
      True{}
    case SCon{c, t}:
      Char.is_digit(c) && Digits.all.go(t)

# A non-empty run of ASCII digits.
def Digits.all(+s: String) -> Bool:
  Bool.not(String.is_empty(s)) && Digits.all.go(s)

def Year.ok(+s: String) -> Bool:
  Digits.all(s) && Nat.is_eq(String.length(s), 4n) && Nat.is_ge(S.Par.nat(s), 1971n) && Nat.is_le(S.Par.nat(s), 9998n)

def Jobs.ok(+s: String) -> Bool:
  Digits.all(s) && Nat.is_le(String.length(s), 3n) && Nat.is_ge(S.Par.nat(s), 1n) && Nat.is_le(S.Par.nat(s), 256n)

def Opts.year.go(ok: Bool, +v: String, o: Opts) -> Opts:
  match ok:
    case True{}:
      Opts.set_year(o, S.Par.nat(v))
    case False{}:
      Opts.fail(o, "--year wants a four-digit year, not '" ++ v ++ "'")

def Opts.jobs.go(ok: Bool, +v: String, o: Opts) -> Opts:
  match ok:
    case True{}:
      Opts.set_jobs(o, S.Par.nat(v))
    case False{}:
      Opts.fail(o, "--jobs wants a number from 1 to 256, not '" ++ v ++ "'")

def Opts.nonempty(empty: Bool, +key: String, o: Opts) -> Opts:
  match empty:
    case True{}:
      Opts.fail(o, key ++ " wants a value")
    case False{}:
      o

def Opts.value(code: Nat, +key: String, +v: String, o: Opts) -> Opts:
  match code:
    case 1n:
      Opts.year.go(Year.ok(v), v, o)
    case 2n:
      Opts.nonempty(String.is_empty(v), key, Opts.set_zone(o, v))
    case 3n:
      Opts.nonempty(String.is_empty(v), key, Opts.add_root(o, v))
    case _:
      Opts.jobs.go(Jobs.ok(v), v, o)

def Opts.switch(code: Nat, o: Opts) -> Opts:
  match code:
    case 10n:
      Opts.set_shard(o)
    case 11n:
      Opts.set_fmt(o, 1n)
    case 12n:
      Opts.set_fmt(o, 2n)
    case 13n:
      Opts.set_fmt(o, 3n)
    case 14n:
      Opts.set_fmt(o, 4n)
    case 15n:
      Opts.set_share(o)
    case 16n:
      Opts.set_info(o, 1n)
    case _:
      Opts.set_info(o, 2n)

def Opts.bare.go(year: Bool, +a: String, o: Opts) -> Opts:
  match year:
    case True{}:
      Opts.set_year(o, S.Par.nat(a))
    case False{}:
      Opts.fail(o, "unknown option '" ++ a ++ "'")

def Opts.errs(+o: Opts) -> Bool:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      Bool.not(String.is_empty(err))

# An argument: "--flag", "--flag=value", "--flag value", or a bare year.
@unsafe def Opts.parse(args: List<&2, String>, +o: Opts) -> Opts:
  match args:
    case Nil{}:
      o
    case Con{+a, rest}:
      Opts.parse.gate(Opts.errs(o), a, rest, o)

@unsafe def Opts.parse.gate(stop: Bool, +a: String, rest: List<&2, String>, o: Opts) -> Opts:
  match stop:
    case True{}:
      o
    case False{}:
      Opts.parse.arg(String.split(a, '='), a, rest, o)

@unsafe def Opts.parse.arg(parts: List<&2, String>, +a: String, rest: List<&2, String>, o: Opts) -> Opts:
  match parts:
    case Con{+key, tail}:
      Opts.parse.code(Flag.find(Flags(), key), key, tail, a, rest, o)
    case Nil{}:
      Opts.parse(rest, o)

@unsafe def Opts.parse.code(+code: Nat, +key: String, tail: List<&2, String>, a: String, rest: List<&2, String>, o: Opts) -> Opts:
  Opts.parse.kind(Nat.is_eq(code, 0n), Nat.is_lt(code, 10n), code, key, tail, a, rest, o)

@unsafe def Opts.parse.kind(unknown: Bool, takes: Bool, code: Nat, +key: String, tail: List<&2, String>, +a: String, rest: List<&2, String>, o: Opts) -> Opts:
  match unknown takes tail:
    case True{} _ _:
      Opts.parse(rest, Opts.bare.go(Year.ok(a), a, o))
    case False{} True{} Con{v, more}:
      Opts.parse(rest, Opts.value(code, key, String.join(Con{v, more}, "="), o))
    case False{} True{} Nil{}:
      Opts.parse.next(code, key, rest, o)
    case False{} False{} Nil{}:
      Opts.parse(rest, Opts.switch(code, o))
    case False{} False{} Con{v, more}:
      Opts.fail(o, key ++ " takes no value")

@unsafe def Opts.parse.next(code: Nat, +key: String, rest: List<&2, String>, o: Opts) -> Opts:
  match rest:
    case Con{v, more}:
      Opts.parse(more, Opts.value(code, key, v, o))
    case Nil{}:
      Opts.fail(o, key ++ " wants a value")

def Opts.check.fmt(clash: Bool, o: Opts) -> Opts:
  match clash:
    case True{}:
      Opts.fail(o, "choose one of --json, --markdown, --html and --card")
    case False{}:
      o

def Opts.check(+o: Opts) -> Opts:
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, +nfmt, share, info, +err}:
      Opts.check.fmt(Nat.is_gt(nfmt, 1n) && String.is_empty(err), Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err})

def Args.dup(xs: List<String>) -> List<&2, String>:
  match xs:
    case Con{h, t}:
      Con{h, Args.dup(t)}
    case Nil{}:
      Nil{}

# The arguments after the program name.
def Args.tail(args: List<String>) -> List<&2, String>:
  match args:
    case Con{prog, rest}:
      Args.dup(rest)
    case Nil{}:
      Nil{}

def Usage() -> String:
  String.join([
    "ccwrapped: a year of Claude Code in numbers, from the transcripts on this machine.",
    "",
    "usage: ccwrapped [YEAR] [options]",
    "",
    "  --year YYYY     report year (default: the current year in the report time zone)",
    "  --tz ZONE       IANA time zone for days and hours (default: $TZ, else the system zone)",
    "  --root DIR      transcript directory, repeatable (default: $CLAUDE_CONFIG_DIR/projects",
    "                  or ~/.claude/projects)",
    "  --json          every metric as JSON (schema ccwrapped.metrics/v1)",
    "  --markdown      the report as Markdown",
    "  --html          the report as HTML slides, one self-contained page",
    "  --card          a one-page share card as HTML (never shows project names)",
    "  --share         show projects as project-1, project-2, ... and MCP tools by server rank",
    "  --jobs N        parallel reader processes (default: CPU threads, at most 16)",
    "  -h, --help      this help",
    "  --version       the version",
    "",
    "Every format goes to standard output: ccwrapped --html > wrapped.html",
    "Cost is the API-equivalent value at list prices, not a bill. Rules: docs/semantics.md."
  ], "\n")

# Files
# -----

type Src is Data:
  Src{size: Nat, mtime: Nat, path: String}

# One "size\tmtime\tpath" row of the walk.
def Src.of(fields: List<&2, String>) -> Maybe<&2, Src>:
  match fields:
    case Con{size, Con{mtime, rest}}:
      Some{Src{S.Par.nat(size), S.Par.nat(mtime), String.join(rest, "\t")}}
    case _:
      None{}

def Src.keep.if(keep: Bool, s: Src, acc: List<&2, Src>) -> List<&2, Src>:
  match keep:
    case True{}:
      Con{s, acc}
    case False{}:
      acc

def Src.keep(m: Maybe<&2, Src>, cutoff: Nat, acc: List<&2, Src>) -> List<&2, Src>:
  match m:
    case Some{Src{size, +mtime, path}}:
      Src.keep.if(Nat.is_ge(mtime, cutoff), Src{size, mtime, path}, acc)
    case None{}:
      acc

def Src.rows(rows: List<&2, String>, +cutoff: Nat, acc: List<&2, Src>) -> List<&2, Src>:
  match rows:
    case Nil{}:
      acc
    case Con{row, rest}:
      Src.rows(rest, cutoff, Src.keep(Src.of(String.split(row, '\t')), cutoff, acc))

def Src.bigger(a: Src, b: Src) -> Bool:
  match a b:
    case Src{sa, ma, pa} Src{sb, mb, pb}:
      Nat.is_ge(sa, sb)

# Snake order over size-sorted files, 0..N-1 then N-1..0, keeps shard loads close.
def Shard.of.pick(low: Bool, r: Nat, n: Nat) -> Nat:
  match low:
    case True{}:
      r
    case False{}:
      (n * 2n - 1n - r : Nat)

def Shard.of.go(+r: Nat, +n: Nat) -> Nat:
  Shard.of.pick(Nat.is_lt(r, n), r, n)

def Shard.of(i: Nat, +n: Nat) -> Nat:
  Shard.of.go(Nat.mod(i, (n * 2n : Nat)), n)

def Shard.push.fin(k: String, path: String, got: Map<&2, List<&2, String>> & List<&2, String>) -> Map<&2, List<&2, String>>:
  match got:
    case Tuple{m, xs}:
      Map.set(&2, List<&2, String>, m, k, Con{path, xs})

def Shard.push(+k: String, path: String, m: Map<&2, List<&2, String>>) -> Map<&2, List<&2, String>>:
  Shard.push.fin(k, path, Map.get(List<&2, String>, Nil{}, m, k))

def Shard.assign(files: List<&2, Src>, +i: Nat, +n: Nat, m: Map<&2, List<&2, String>>) -> Map<&2, List<&2, String>>:
  match files:
    case Nil{}:
      m
    case Con{Src{size, mtime, path}, rest}:
      Shard.assign(rest, (i + 1n : Nat), n, Shard.push(Nat.show(Shard.of(i, n)), path, m))

def Shard.lists(xs: List<&2, Sigma<&2, &2, String, _ => List<&2, String>>>) -> List<&2, List<&2, String>>:
  match xs:
    case Nil{}:
      Nil{}
    case Con{Tuple{k, paths}, rest}:
      Con{paths, Shard.lists(rest)}

# IO
# --

@unsafe def Walk.all.got(r: Result<&1, &1, U32 & String, String>, +root: String, roots: List<&2, String>, acc: String) -> IO(String):
  match r:
    case Done{rows}:
      Walk.all(roots, acc ++ rows)
    case Fail{Tuple{code, msg}}:
      IO.die(String, 1, "ccwrapped: cannot read transcripts at " ++ root ++ ": " ++ msg)

@unsafe def Walk.all(roots: List<&2, String>, acc: String) -> IO(String):
  match roots:
    case Nil{}:
      IO.pure(String, acc)
    case Con{+root, rest}:
      IO.bind(Result<&1, &1, U32 & String, String>, String, E.Dir.walk(root), r => Walk.all.got(r, root, rest, acc))

@unsafe def Child.read.more(r: Result<&1, &1, U32 & String, String>, f: File, acc: String) -> IO(String):
  match r:
    case Done{SNil{}}:
      IO.bind(Unit, String, File.close(f), u => IO.pure(String, acc))
    case Done{SCon{h, t}}:
      Child.read(f, acc ++ SCon{h, t})
    case Fail{e}:
      IO.bind(Unit, String, File.close(f), u => IO.pure(String, acc))

@unsafe def Child.read.got(got: File & Result<&1, &1, U32 & String, String>, acc: String) -> IO(String):
  match got:
    case Tuple{f, r}:
      Child.read.more(r, f, acc)

@unsafe def Child.read(f: File, acc: String) -> IO(String):
  IO.bind(File & Result<&1, &1, U32 & String, String>, String, File.read(f, 1048576), got => Child.read.got(got, acc))

def Child.nonempty(s: String) -> Bool:
  Bool.not(String.is_empty(s))

def Child.paths(xs: List<&2, String>) -> List<&2, String>:
  List.filter(~String, ~Child.nonempty, xs)

# A child: the paths arrive on stdin, one per line; the fold goes to stdout.
@unsafe def Child.run() -> IO(Unit):
  do IO<Unit>:
    f : File <- IO.try(File, File.open("/dev/stdin", "r"))
    text : String <- Child.read(f, "")
    fold : F.Fold <- P.Par.read(Child.paths(String.lines(text)), F.Fold.empty())
    IO.write(S.Ser.fold(fold))

@unsafe def Parent.spawn(shards: List<&2, List<&2, String>>, acc: List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)>) -> IO(List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)>):
  match shards:
    case Nil{}:
      IO.pure(List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)>, acc)
    case Con{paths, rest}:
      IO.bind(Chan(Result<&1, &1, U32 & String, U32 & (String & String)>), List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)>, IO.fork(Result<&1, &1, U32 & String, U32 & (String & String)>, Process.run("/proc/self/exe", ["--threads", "2", "--", "--shard"], String.join(paths, "\n"), 2147483647, 3600000)), c => Parent.spawn(rest, Con{c, acc}))

@unsafe def Parent.collect.status(ok: Bool, out: String, err: String, rest: List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)>, f: F.Fold) -> IO(F.Fold):
  match ok:
    case True{}:
      Parent.collect(rest, S.Des.fold(out, f))
    case False{}:
      IO.die(F.Fold, 1, "ccwrapped: a shard process failed: " ++ err)

@unsafe def Parent.collect.got(r: Result<&1, &1, U32 & String, U32 & (String & String)>, rest: List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)>, f: F.Fold) -> IO(F.Fold):
  match r:
    case Done{Tuple{status, Tuple{out, err}}}:
      Parent.collect.status(U32.is_eq(status, 0), out, err, rest, f)
    case Fail{Tuple{code, msg}}:
      IO.die(F.Fold, 1, "ccwrapped: a shard process could not run: " ++ msg)

@unsafe def Parent.collect(chans: List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)>, f: F.Fold) -> IO(F.Fold):
  match chans:
    case Nil{}:
      IO.pure(F.Fold, f)
    case Con{c, rest}:
      IO.bind(Result<&1, &1, U32 & String, U32 & (String & String)>, F.Fold, IO.join(Result<&1, &1, U32 & String, U32 & (String & String)>, c), r => Parent.collect.got(r, rest, f))

def Zone.pick.if(empty: Bool, given: String, system: String) -> String:
  match empty:
    case True{}:
      system
    case False{}:
      given

def Zone.pick(+given: String, system: String) -> String:
  Zone.pick.if(String.is_empty(given), given, system)

# An unnamed system zone (no $TZ, /etc/localtime not a link into the zone database).
def Zone.label.go(empty: Bool, tz: String) -> String:
  match empty:
    case True{}:
      "the system time zone"
    case False{}:
      tz

def Zone.label(+tz: String) -> String:
  Zone.label.go(String.is_empty(tz), tz)

@unsafe def Zone.check.go(bad: Bool, now: Nat, tz: String) -> IO(Nat):
  match bad:
    case True{}:
      IO.die(Nat, 2, "ccwrapped: unknown time zone '" ++ tz ++ "'; use an IANA name such as Europe/Berlin")
    case False{}:
      IO.pure(Nat, now)

# The current year in the zone, which is 0 when the zone database does not know it.
@unsafe def Zone.check(+now: Nat, tz: String) -> IO(Nat):
  Zone.check.go(Nat.is_eq(now, 0n), now, tz)

def Year.pick.go(unset: Bool, asked: Nat, now: Nat) -> Nat:
  match unset:
    case True{}:
      now
    case False{}:
      asked

def Year.pick(+asked: Nat, now: Nat) -> Nat:
  Year.pick.go(Nat.is_eq(asked, 0n), asked, now)

@unsafe def Files.none.go(empty: Bool, roots: List<&2, String>) -> IO(Unit):
  match empty:
    case True{}:
      IO.die(Unit, 1, "ccwrapped: no transcripts (*.jsonl) under " ++ String.join(roots, ", "))
    case False{}:
      IO.pure(Unit, Unit{})

@unsafe def Files.none(+rows: String, roots: List<&2, String>) -> IO(Unit):
  Files.none.go(String.is_empty(rows), roots)

def Env.or(r: Result<&1, &1, U32 & String, String>, d: String) -> String:
  match r:
    case Done{v}:
      v
    case Fail{e}:
      d

def Roots.default.go(unset: Bool, config: String, home: String) -> List<&2, String>:
  match unset:
    case True{}:
      [home ++ "/.claude/projects"]
    case False{}:
      [config ++ "/projects"]

def Roots.default(roots: List<&2, String>, +config: String, home: String) -> List<&2, String>:
  match roots:
    case Nil{}:
      Roots.default.go(String.is_empty(config), config, home)
    case Con{r, rest}:
      Con{r, rest}

# The earliest modification time a transcript may have and still hold a response of the year:
# two days before 1 January UTC, which covers every UTC offset.
def Cutoff(year: Nat) -> Nat:
  (R.Days.of(year, 1n, 1n) * 86400n - R.Epoch0() - 172800n : Nat)

def Jobs.if(auto: Bool, asked: Nat, auto_n: Nat) -> Nat:
  match auto:
    case True{}:
      auto_n
    case False{}:
      asked

def Jobs(+asked: Nat, threads: Nat) -> Nat:
  Jobs.if(Nat.is_eq(asked, 0n), asked, Nat.min(threads, 16n))

@unsafe def Parent.run(o: Opts) -> IO(Unit):
  match o:
    case Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err}:
      do IO<Unit>:
        home : String <- IO.bind(Result<&1, &1, U32 & String, String>, String, IO.get_env("HOME"), r => IO.pure(String, Env.or(r, "")))
        config : String <- IO.bind(Result<&1, &1, U32 & String, String>, String, IO.get_env("CLAUDE_CONFIG_DIR"), r => IO.pure(String, Env.or(r, "")))
        system : String <- E.Zone.name()
        +tz : String = Zone.pick(zone, system)
        now : U32 <- E.Zone.year(tz)
        current : Nat <- Zone.check(U32.to_nat(now), tz)
        +y : Nat = Year.pick(year, current)
        offsets : String <- IO.try(String, E.Zone.offsets(tz, U32.from_nat(y)))
        +hm : String = home
        +rs : List<&2, String> = Roots.default(roots, config, hm)
        +rows : String <- Walk.all(rs, "")
        u : Unit <- Files.none(rows, rs)
        threads : U32 <- IO.thread_count()
        +n : Nat = Jobs(jobs, U32.to_nat(threads))
        files : List<&2, Src> = List.sort(~Src, ~Src.bigger, Src.rows(String.lines(rows), Cutoff(y), Nil{}))
        chans : List<&1, Chan(Result<&1, &1, U32 & String, U32 & (String & String)>)> <- Parent.spawn(Shard.lists(Map.to_list(&2, List<&2, String>, Shard.assign(files, 0n, n, MTip{}))), Nil{})
        fold : F.Fold <- Parent.collect(chans, F.Fold.empty())
        IO.print(Main.output(fmt, share, y, tz, hm, R.Zone.parse(offsets), fold))

@unsafe def Main.output(fmt: Nat, share: Bool, year: Nat, tz: String, home: String, shifts: List<&2, R.Shift>, fold: F.Fold) -> String:
  match fmt:
    case 1n:
      O.Metrics.json(year, tz, home, shifts, fold)
    case _:
      O.Metrics.text(fmt, share, year, Zone.label(tz), home, shifts, fold)

@unsafe def Main.run(shard: Bool, o: Opts) -> IO(Unit):
  match shard:
    case True{}:
      Child.run()
    case False{}:
      Parent.run(o)

@unsafe def Main.go(o: Opts) -> IO(Unit):
  match o:
    case Opts{year, zone, roots, jobs, +shard, fmt, nfmt, share, +info, +err}:
      Main.info(String.is_empty(err), info, err, shard, Opts{year, zone, roots, jobs, shard, fmt, nfmt, share, info, err})

@unsafe def Main.info(ok: Bool, info: Nat, err: String, shard: Bool, o: Opts) -> IO(Unit):
  match ok info:
    case False{} _:
      IO.die(Unit, 2, "ccwrapped: " ++ err ++ "\nTry 'ccwrapped --help'.")
    case True{} 1n:
      IO.print(Usage())
    case True{} 2n:
      IO.print(V.Version())
    case True{} _:
      Main.run(shard, o)

@unsafe def main() -> IO(Unit):
  do IO<Unit>:
    args : List<String> <- IO.args()
    Main.go(Opts.check(Opts.parse(Args.tail(args), Opts.empty())))
