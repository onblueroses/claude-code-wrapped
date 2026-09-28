import Base
import ./effects.bend as E
import ./fold.bend as F
import ./par.bend as P

# Every transcript under a root, read into one fold.

@unsafe def Walk.after_tab(s: String) -> String:
  match s:
    case SNil{}:
      SNil{}
    case SCon{Chr{+c}, t}:
      Walk.after_tab.if(U32.is_eq(c, 9), c, t)

# The walk lists "size\tmtime\tpath" lines; the path is everything after the second tab.
def Walk.path(line: String) -> String:
  Walk.after_tab(Walk.after_tab(line))

@unsafe def Walk.after_tab.if(tab: Bool, c: U32, t: String) -> String:
  match tab:
    case True{}:
      t
    case False{}:
      Walk.after_tab(t)

def Walk.paths.keep(empty: Bool, row: String, rest: List<&2, String>) -> List<&2, String>:
  match empty:
    case True{}:
      rest
    case False{}:
      Con{Walk.path(row), rest}

def Walk.paths(rows: List<&2, String>) -> List<&2, String>:
  match rows:
    case Nil{}:
      Nil{}
    case Con{+row, rest}:
      Walk.paths.keep(String.is_empty(row), row, Walk.paths(rest))

@unsafe def Scan.root(root: String, f: F.Fold) -> IO(F.Fold):
  IO.bind(String, F.Fold, IO.try(String, E.Dir.walk(root)), rows => P.Par.read(Walk.paths(String.lines(rows)), f))
