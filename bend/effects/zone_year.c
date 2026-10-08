// Zone
// ====
// The current calendar year in an IANA zone ("" is the process zone), or 0 when the name is
// not a zone in the system's zone database. glibc reads an unknown TZ as UTC without saying
// so; checking for the zone file first turns a misspelled --tz into an error.

#include <limits.h>
#include <sys/stat.h>
#include <time.h>

static int zone_known(const char* name) {
  struct stat st;
  if (name[0] == '/') return stat(name, &st) == 0 && S_ISREG(st.st_mode);
  if (strstr(name, "..") != NULL) return 0;
  const char* dir = getenv("TZDIR");
  if (dir == NULL || dir[0] == 0) dir = "/usr/share/zoneinfo";
  char path[PATH_MAX];
  if (snprintf(path, sizeof path, "%s/%s", dir, name) >= (int)sizeof path) return 0;
  return stat(path, &st) == 0 && S_ISREG(st.st_mode);
}

Term zone_year_run(Env e, Term* f, IoWork* w) {
  (void)w;
  uint64_t n = 0;
  char* name = io_cstr(e, f[0], &n);
  uint32_t year = 0;
  if (!io_nul(name, n) && (n == 0 || zone_known(name))) {
    if (n > 0) setenv("TZ", name, 1);
    tzset();
    time_t now = time(NULL);
    struct tm tm;
    localtime_r(&now, &tm);
    year = (uint32_t)(tm.tm_year + 1900);
  }
  free(name);
  return (Term)year;
}

static void __attribute__((constructor)) zone_year_use(void) {
  io_eff(CID(Zone.year), zone_year_run);
}
