// Zone
// ====
// The IANA name of the system zone: $TZ when set, else the target of /etc/localtime after
// "zoneinfo/". Empty when neither names one.

#include <limits.h>

#if defined(CID_EFFECTS_ZONE_NAME)
#define ZONE_NAME_CID CID_EFFECTS_ZONE_NAME
#elif defined(CID___EFFECTS_ZONE_NAME)
#define ZONE_NAME_CID CID___EFFECTS_ZONE_NAME
#elif defined(CID____EFFECTS_ZONE_NAME)
#define ZONE_NAME_CID CID____EFFECTS_ZONE_NAME
#else
#define ZONE_NAME_CID CID_ZONE_NAME
#endif

Term zone_name_run(Env e, Term* f, IoWork* w) {
  (void)f;
  (void)w;
  const char* tz = getenv("TZ");
  if (tz != NULL && tz[0] != 0) {
    if (tz[0] == ':') tz += 1;
    return io_str(e, tz, strlen(tz));
  }
  char buf[PATH_MAX];
  if (realpath("/etc/localtime", buf) == NULL) {
    return io_str(e, "", 0);
  }
  const char* at = strstr(buf, "zoneinfo/");
  const char* name = at ? at + 9 : "";
  return io_str(e, name, strlen(name));
}

static void __attribute__((constructor)) zone_name_use(void) {
  io_eff(ZONE_NAME_CID, zone_name_run, 0);
}
