// Lines
// =====
// Reads at least `want` bytes of complete lines (more when one line is longer), ending on a
// newline and never past the limit recorded at open. An empty string means no complete line
// remains; a trailing partial line is never returned.

// The constructor id carries the module path of effects.bend, which differs between Bend
// releases; accept each spelling seen so far.
#if defined(CID_EFFECTS_LINES_NEXT)
#define LINES_NEXT_CID CID_EFFECTS_LINES_NEXT
#elif defined(CID___EFFECTS_LINES_NEXT)
#define LINES_NEXT_CID CID___EFFECTS_LINES_NEXT
#elif defined(CID____EFFECTS_LINES_NEXT)
#define LINES_NEXT_CID CID____EFFECTS_LINES_NEXT
#else
#define LINES_NEXT_CID CID_LINES_NEXT
#endif

#ifndef LINES_STATE
#define LINES_STATE
#include <sys/stat.h>
// Per-descriptor read limit (length at open) and next offset, shared by Lines.open and
// Lines.next; the compiler may emit either effect first.
#define LINES_MAX_FD 65536
static uint64_t lines_limit[LINES_MAX_FD];
static uint64_t lines_offset[LINES_MAX_FD];
#endif

// Strict UTF-8 (no overlongs, surrogates or code points past U+10FFFF), as a JSON text requires.
static int lines_utf8_ok(const unsigned char* p, uint64_t n) {
  uint64_t i = 0;
  while (i < n) {
    unsigned c = p[i];
    if (c < 0x80) { i += 1; continue; }
    uint64_t k;
    unsigned lo = 0x80, hi = 0xBF;
    if (c >= 0xC2 && c <= 0xDF) k = 1;
    else if (c == 0xE0) { k = 2; lo = 0xA0; }
    else if (c == 0xED) { k = 2; hi = 0x9F; }
    else if (c >= 0xE1 && c <= 0xEF) k = 2;
    else if (c == 0xF0) { k = 3; lo = 0x90; }
    else if (c == 0xF4) { k = 3; hi = 0x8F; }
    else if (c >= 0xF1 && c <= 0xF3) k = 3;
    else return 0;
    if (i + k >= n) return 0;
    if (p[i + 1] < lo || p[i + 1] > hi) return 0;
    for (uint64_t j = 2; j <= k; j += 1) {
      if (p[i + j] < 0x80 || p[i + j] > 0xBF) return 0;
    }
    i += k + 1;
  }
  return 1;
}

// Long strings
// ------------
// A JSON string whose content is valid and longer than LINES_LONG bytes is shortened to its
// first LINES_KEEP characters (a character is one UTF-8 sequence or one escape). If those are
// all blank and a later one is not, one 'x' is appended. This preserves everything the metrics
// read: the line's validity (only valid contents are shortened, and a shortened content is
// still valid), every value captured in full (the longest, cwd, is at most PATH_MAX = 4096
// bytes), and the human-prompt test, which reads the first 32 characters and whether any
// character is not blank. A string with an invalid escape or a raw control character is left
// as it is, so the lexer still rejects its line.

#define LINES_LONG 4096
#define LINES_KEEP 64

static int lines_hex(unsigned char c) {
  return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') || (c >= 'A' && c <= 'F');
}

static unsigned lines_hexval(const unsigned char* p) {
  unsigned v = 0;
  for (int i = 0; i < 4; i += 1) {
    unsigned c = p[i];
    v = v * 16 + (c <= '9' ? c - '0' : (c | 32) - 'a' + 10);
  }
  return v;
}

// ASCII whitespace as the human-prompt test defines it: space, \t, \n, \r, \v, \f.
static int lines_blank_code(unsigned c) {
  return c == 32 || c == 9 || c == 10 || c == 13 || c == 11 || c == 12;
}

// Length of the string content starting at p (just after the opening quote) up to the closing
// quote, or 0 when the content is invalid or unterminated. Counts characters and blankness.
static uint64_t lines_string(const unsigned char* p, uint64_t n, uint64_t* cut, int* blank_head, int* ink_tail) {
  uint64_t i = 0, chars = 0;
  *cut = 0;
  *blank_head = 1;
  *ink_tail = 0;
  while (i < n) {
    unsigned c = p[i];
    uint64_t len;
    unsigned code;
    if (c == '"') {
      if (chars <= LINES_KEEP) *cut = i;
      return i + 1;
    }
    if (c < 0x20) return 0;
    if (c == '\\') {
      if (i + 1 >= n) return 0;
      unsigned e = p[i + 1];
      if (e == 'u') {
        if (i + 5 >= n || !lines_hex(p[i + 2]) || !lines_hex(p[i + 3]) || !lines_hex(p[i + 4]) || !lines_hex(p[i + 5])) return 0;
        code = lines_hexval(p + i + 2);
        len = 6;
      } else {
        switch (e) {
          case '"': case '\\': case '/': code = e; break;
          case 'b': code = 8; break;
          case 'f': code = 12; break;
          case 'n': code = 10; break;
          case 'r': code = 13; break;
          case 't': code = 9; break;
          default: return 0;
        }
        len = 2;
      }
    } else {
      len = c < 0x80 ? 1 : c < 0xE0 ? 2 : c < 0xF0 ? 3 : 4;
      code = c < 0x80 ? c : 0x80;
    }
    if (chars < LINES_KEEP) {
      if (!lines_blank_code(code)) *blank_head = 0;
    } else if (!lines_blank_code(code)) {
      *ink_tail = 1;
    }
    chars += 1;
    i += len;
    if (chars == LINES_KEEP) *cut = i;
  }
  return 0;
}

// Shortens the long strings of one line in place; returns the new length.
static uint64_t lines_compact(unsigned char* b, uint64_t n) {
  uint64_t i = 0, out = 0;
  while (i < n) {
    if (b[i] != '"') {
      b[out++] = b[i++];
      continue;
    }
    uint64_t cut;
    int blank_head, ink_tail;
    uint64_t span = lines_string(b + i + 1, n - i - 1, &cut, &blank_head, &ink_tail);
    if (span == 0) {
      memmove(b + out, b + i, n - i);
      return out + (n - i);
    }
    if (span - 1 > LINES_LONG) {
      b[out++] = '"';
      memmove(b + out, b + i + 1, cut);
      out += cut;
      if (blank_head && ink_tail) b[out++] = 'x';
      b[out++] = '"';
    } else {
      memmove(b + out, b + i, span + 1);
      out += span + 1;
    }
    i += span + 1;
  }
  return out;
}

// Replaces each line that is not valid UTF-8 with one U+0001, which no JSON parser accepts, so
// the line is counted as malformed instead of being decoded leniently, and shortens the long
// strings of the others. Returns the new length.
static uint64_t lines_sanitize(char* buf, uint64_t n) {
  uint64_t out = 0;
  uint64_t start = 0;
  for (uint64_t i = 0; i < n; i += 1) {
    if (buf[i] != '\n') continue;
    uint64_t len = i - start;
    if (lines_utf8_ok((const unsigned char*)buf + start, len)) {
      memmove(buf + out, buf + start, len);
      out += lines_compact((unsigned char*)buf + out, len);
      buf[out++] = '\n';
    } else {
      buf[out] = 1;
      buf[out + 1] = '\n';
      out += 2;
    }
    start = i + 1;
  }
  return out;
}

static void lines_next_call(IoWork* w) {
  int fd = (int)w->hand;
  uint64_t off = lines_offset[fd];
  uint64_t limit = lines_limit[fd];
  uint64_t cap = w->word > 0 ? w->word : 1;
  char* buf = NULL;
  uint64_t have = 0;
  uint64_t cut = 0;
  w->size = 0;
  while (off + have < limit) {
    uint64_t room = cap - have;
    if (room == 0) {
      cap *= 2;
      room = cap - have;
    }
    buf = io_mem(realloc(buf, cap + 1));
    if (room > limit - off - have) room = limit - off - have;
    ssize_t n = pread(fd, buf + have, room, (off_t)(off + have));
    if (n < 0) {
      io_sys_end(w, -1);
      free(buf);
      w->data = NULL;
      return;
    }
    if (n == 0) break;
    for (uint64_t i = have + (uint64_t)n; i > have; i -= 1) {
      if (buf[i - 1] == '\n') {
        cut = i;
        break;
      }
    }
    have += (uint64_t)n;
    if (cut > 0 && have >= w->word) break;
  }
  lines_offset[fd] = off + cut;
  w->data = buf;
  w->size = buf ? lines_sanitize(buf, cut) : 0;
  io_sys_end(w, 0);
}

// The Txt constructors of types.bend; their id carries the module path, as above.
#if defined(CID_TYPES_TCON)
#define LINES_TCON CID_TYPES_TCON
#define LINES_TNIL CID_TYPES_TNIL
#elif defined(CID____TYPES_TCON)
#define LINES_TCON CID____TYPES_TCON
#define LINES_TNIL CID____TYPES_TNIL
#else
#define LINES_TCON CID_TCON
#define LINES_TNIL CID_TNIL
#endif

// Decodes UTF-8 (already validated) into a Txt, one code point per cell.
static Term lines_txt(Env e, const char* p, u64 n) {
  Term s = term_pak(LINES_TNIL, 0);
  u64 hole = 0;
  for (u64 i = 0; i < n;) {
    unsigned b = (uint8_t)p[i];
    u64 c, k;
    if (b < 0x80) { c = b; k = 1; }
    else if (b < 0xE0) { c = b & 0x1F; k = 2; }
    else if (b < 0xF0) { c = b & 0x0F; k = 3; }
    else { c = b & 0x07; k = 4; }
    for (u64 j = 1; j < k && i + j < n; j += 1) c = (c << 6) | ((uint8_t)p[i + j] & 0x3F);
    i += k;
    u64 l = heap_alloc(e, 1);
    Term t = term_ctr(LINES_TCON, l);
    e.mem[l] = c;
    if (hole == 0) {
      s = t;
    } else {
      e.mem[hole] = io_seal(e, t, LINES_TCON);
    }
    hole = l + 1;
  }
  if (hole != 0) {
    e.mem[hole] = io_seal(e, term_pak(LINES_TNIL, 0), LINES_TCON);
  }
  return s;
}

static Term lines_next_pack(Env e, IoWork* w) {
  Term r = w->code ? io_fail(e, w->code, NULL) : io_done(e, lines_txt(e, w->data ? w->data : "", w->size));
  free(w->data);
  return io_tup(e, io_hand(w->hand), r);
}

Term lines_next_run(Env e, Term* f, IoWork* w) {
  w->hand = (intptr_t)io_hand_v(f[0]);
  w->word = f[1] < INT32_MAX ? f[1] : INT32_MAX;
  if (w->hand < 0 || w->hand >= LINES_MAX_FD) {
    w->code = EBADF;
    w->data = NULL;
    return lines_next_pack(e, w);
  }
  return io_work(w, lines_next_call, lines_next_pack);
}

static void __attribute__((constructor)) lines_next_use(void) {
  io_eff(LINES_NEXT_CID, lines_next_run, 0);
}
