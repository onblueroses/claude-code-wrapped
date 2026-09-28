import Base
import ./types.bend as T

def Lines.open(path: String) -> IO(Result<&1, &1, U32 & String, File>):
  import "./effects/lines_open.c"
  import "./effects/lines_open.js"

def Lines.next(file: File, want: U32) -> IO(File & Result<&1, &1, U32 & String, T.Txt>):
  import "./effects/lines_next.c"
  import "./effects/lines_next.js"

def Dir.walk(root: String) -> IO(Result<&1, &1, U32 & String, String>):
  import "./effects/dir_walk.c"
  import "./effects/dir_walk.js"

def Zone.offsets(zone: String, year: U32) -> IO(Result<&1, &1, U32 & String, String>):
  import "./effects/zone_offsets.c"
  import "./effects/zone_offsets.js"

def Zone.name() -> IO(String):
  import "./effects/zone_name.c"
  import "./effects/zone_name.js"

def Zone.year(zone: String) -> IO(U32):
  import "./effects/zone_year.c"
  import "./effects/zone_year.js"
