// Dir
// ===
// Lists every regular *.jsonl file under a root as "size\tmtime\tpath\n" lines, sorted by
// path. Symlinked directories are not followed; a symlink to a regular file is listed.

#include <ftw.h>
#include <sys/stat.h>

static __thread char* dir_walk_buf;
static __thread uint64_t dir_walk_len;
static __thread uint64_t dir_walk_cap;
static __thread char** dir_walk_rows;
static __thread uint64_t dir_walk_count;
static __thread uint64_t dir_walk_room;

static int dir_walk_visit(const char* path, const struct stat* st, int type, struct FTW* ftw) {
  (void)ftw;
  size_t n = strlen(path);
  if (n < 6 || strcmp(path + n - 6, ".jsonl") != 0) return 0;
  struct stat real;
  if (type == FTW_SL) {
    if (stat(path, &real) != 0 || !S_ISREG(real.st_mode)) return 0;
    st = &real;
  } else if (type != FTW_F || !S_ISREG(st->st_mode)) {
    return 0;
  }
  char head[64];
  int k = snprintf(head, sizeof head, "%llu\t%lld\t", (unsigned long long)st->st_size, (long long)st->st_mtime);
  char* row = io_mem(malloc((size_t)k + n + 2));
  memcpy(row, head, (size_t)k);
  memcpy(row + k, path, n);
  row[k + n] = '\n';
  row[k + n + 1] = 0;
  if (dir_walk_count == dir_walk_room) {
    dir_walk_room = dir_walk_room ? dir_walk_room * 2 : 1024;
    dir_walk_rows = io_mem(realloc(dir_walk_rows, dir_walk_room * sizeof(char*)));
  }
  dir_walk_rows[dir_walk_count++] = row;
  return 0;
}

static int dir_walk_order(const void* a, const void* b) {
  const char* x = strchr(strchr(*(char* const*)a, '\t') + 1, '\t') + 1;
  const char* y = strchr(strchr(*(char* const*)b, '\t') + 1, '\t') + 1;
  return strcmp(x, y);
}

static void dir_walk_call(IoWork* w) {
  dir_walk_rows = NULL;
  dir_walk_count = dir_walk_room = 0;
  int rc = nftw(w->data, dir_walk_visit, 64, FTW_PHYS);
  free(w->data);
  if (rc != 0) {
    for (uint64_t i = 0; i < dir_walk_count; i += 1) free(dir_walk_rows[i]);
    free(dir_walk_rows);
    w->data = NULL;
    io_sys_end(w, -1);
    return;
  }
  qsort(dir_walk_rows, dir_walk_count, sizeof(char*), dir_walk_order);
  dir_walk_len = 0;
  dir_walk_cap = 1;
  dir_walk_buf = io_mem(malloc(1));
  for (uint64_t i = 0; i < dir_walk_count; i += 1) {
    size_t n = strlen(dir_walk_rows[i]);
    while (dir_walk_len + n + 1 > dir_walk_cap) dir_walk_cap *= 2;
    dir_walk_buf = io_mem(realloc(dir_walk_buf, dir_walk_cap));
    memcpy(dir_walk_buf + dir_walk_len, dir_walk_rows[i], n);
    dir_walk_len += n;
    free(dir_walk_rows[i]);
  }
  free(dir_walk_rows);
  w->data = dir_walk_buf;
  w->size = dir_walk_len;
  io_sys_end(w, 0);
}

static Term dir_walk_pack(Env e, IoWork* w) {
  Term r = w->code ? io_fail(e, w->code, NULL) : io_done(e, io_str(e, w->data, w->size));
  free(w->data);
  return r;
}

Term dir_walk_run(Env e, Term* f, IoWork* w) {
  w->data = io_cstr(e, f[0], &w->size);
  if (io_nul(w->data, w->size)) {
    free(w->data);
    w->data = NULL;
    w->code = EILSEQ;
    return dir_walk_pack(e, w);
  }
  return io_work(w, dir_walk_call, dir_walk_pack);
}

static void __attribute__((constructor)) dir_walk_use(void) {
  io_eff(CID(Dir.walk), dir_walk_run);
}
