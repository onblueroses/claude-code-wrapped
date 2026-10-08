// Lines
// =====
// A transcript source opened for whole-line reads. The byte length observed at open is the
// read limit, so records appended while a session is still writing are left for the next run.

#ifndef LINES_STATE
#define LINES_STATE
#include <sys/stat.h>
// Per-descriptor read limit (length at open) and next offset, shared by Lines.open and
// Lines.next; the compiler may emit either effect first.
#define LINES_MAX_FD 65536
static uint64_t lines_limit[LINES_MAX_FD];
static uint64_t lines_offset[LINES_MAX_FD];
#endif

static void lines_open_call(IoWork* w) {
  int fd = open(w->data, O_RDONLY | O_CLOEXEC);
  if (fd < 0) {
    w->made = (intptr_t)io_sys_end(w, -1);
    return;
  }
  struct stat st;
  if (fstat(fd, &st) != 0 || fd >= LINES_MAX_FD) {
    int err = fd >= LINES_MAX_FD ? EMFILE : errno;
    close(fd);
    errno = err;
    w->made = (intptr_t)io_sys_end(w, -1);
    return;
  }
  lines_limit[fd] = (uint64_t)st.st_size;
  lines_offset[fd] = 0;
  w->made = (intptr_t)io_sys_end(w, fd);
}

static Term lines_open_pack(Env e, IoWork* w) {
  free(w->data);
  return w->code != 0 ? io_fail(e, w->code, NULL) : io_done(e, io_hand(w->made));
}

Term lines_open_run(Env e, Term* f, IoWork* w) {
  w->data = io_cstr(e, f[0], &w->size);
  if (io_nul(w->data, w->size)) {
    w->code = EILSEQ;
    return lines_open_pack(e, w);
  }
  return io_work(w, lines_open_call, lines_open_pack);
}

static void __attribute__((constructor)) lines_open_use(void) {
  io_eff(CID(Lines.open), lines_open_run);
}
