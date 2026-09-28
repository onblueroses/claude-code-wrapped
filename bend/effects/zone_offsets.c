// Zone
// ====
// UTC offsets of an IANA zone ("" keeps the process zone) around a calendar year, as
// "epoch\toffset_seconds\n" lines: the offset at the first epoch, then every transition.

// The constructor id carries the module path of effects.bend, which differs between Bend
// releases; accept each spelling seen so far.
#if defined(CID_EFFECTS_ZONE_OFFSETS)
#define ZONE_OFFSETS_CID CID_EFFECTS_ZONE_OFFSETS
#elif defined(CID___EFFECTS_ZONE_OFFSETS)
#define ZONE_OFFSETS_CID CID___EFFECTS_ZONE_OFFSETS
#elif defined(CID____EFFECTS_ZONE_OFFSETS)
#define ZONE_OFFSETS_CID CID____EFFECTS_ZONE_OFFSETS
#else
#define ZONE_OFFSETS_CID CID_ZONE_OFFSETS
#endif

#include <time.h>

static __thread char* zone_buf;
static __thread uint64_t zone_len;

static void zone_emit(long long t, long off) {
  char row[64];
  int k = snprintf(row, sizeof row, "%lld\t%ld\n", t, off);
  zone_buf = io_mem(realloc(zone_buf, zone_len + (uint64_t)k + 1));
  memcpy(zone_buf + zone_len, row, (size_t)k);
  zone_len += (uint64_t)k;
}

static long zone_offset_at(time_t t) {
  struct tm tm;
  localtime_r(&t, &tm);
  return tm.tm_gmtoff;
}

static void zone_offsets_call(IoWork* w) {
  if (w->size > 0) {
    setenv("TZ", w->data, 1);
  }
  tzset();
  free(w->data);
  struct tm start = {0};
  start.tm_year = (int)w->word - 1900 - 1;
  start.tm_mon = 11;
  start.tm_mday = 30;
  struct tm end = start;
  end.tm_year += 2;
  end.tm_mon = 0;
  end.tm_mday = 3;
  time_t t0 = timegm(&start);
  time_t t1 = timegm(&end);
  zone_buf = NULL;
  zone_len = 0;
  long off = zone_offset_at(t0);
  zone_emit((long long)t0, off);
  for (time_t t = t0; t < t1; t += 900) {
    time_t next = t + 900;
    long got = zone_offset_at(next);
    if (got == off) continue;
    time_t lo = t, hi = next;
    while (hi - lo > 1) {
      time_t mid = lo + (hi - lo) / 2;
      if (zone_offset_at(mid) == off) lo = mid; else hi = mid;
    }
    off = got;
    zone_emit((long long)hi, off);
  }
  w->data = zone_buf;
  w->size = zone_len;
  io_sys_end(w, 0);
}

static Term zone_offsets_pack(Env e, IoWork* w) {
  Term r = w->code ? io_fail(e, w->code, NULL) : io_done(e, io_str(e, w->data, w->size));
  free(w->data);
  return r;
}

Term zone_offsets_run(Env e, Term* f, IoWork* w) {
  w->data = io_cstr(e, f[0], &w->size);
  w->word = f[1];
  return io_work(w, zone_offsets_call, zone_offsets_pack);
}

static void __attribute__((constructor)) zone_offsets_use(void) {
  io_eff(ZONE_OFFSETS_CID, zone_offsets_run, 0);
}
