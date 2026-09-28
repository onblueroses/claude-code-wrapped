import Base
import ./fold.bend as F
import ./scan.bend as S

def Stats.show(f: F.Fold) -> String:
  match f:
    case F.Fold{groups, tools, prompts, lines, bad, usage, files}:
      "files " ++ Nat.show(files) ++ " lines " ++ Nat.show(lines) ++ " malformed " ++ Nat.show(bad) ++ " usage_lines " ++ Nat.show(usage) ++ " groups " ++ Nat.show(Map.size(&2, F.Group, groups)) ++ " tools " ++ Nat.show(Map.size(&2, F.ToolRec, tools)) ++ " prompts " ++ Nat.show(Map.size(&2, String, prompts))

def Stats.root(args: List<String>) -> String:
  match args:
    case Con{prog, Con{root, rest}}:
      root
    case _:
      "."

def main() -> IO(Unit):
  do IO<Unit>:
    args : List<String> <- IO.args()
    f : F.Fold <- S.Scan.root(Stats.root(args), F.Fold.empty())
    IO.print(Stats.show(f))
