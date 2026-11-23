/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * loft's C runtime (@PLN200 B5, @PLN91 strand 5): what C emitted by `src/c/` calls.  Every
 * function here is a port of the runtime the interpreter and the Rust backend share
 * (src/ops.rs), line for line where the semantics live, so the three answer alike:
 *
 *   integers   int64_t; INT64_MIN is null; an operand that is null, an overflow and a
 *              division by zero give null (ops::op_add_long and its peers).
 *   booleans   uint8_t where stored (0, 1, 255 = null); 0/1 in a condition.
 *   text       loft_text owns a byte buffer; loft_str borrows one.  The one-byte text "\0"
 *              is null (store::STRING_NULL), rendered as `null`.
 *
 * C99: <stdint.h>, declarations where they are used.  No allocation failure is survivable
 * here: one aborts the program, as the Rust runtime's does. */
/* loft_cc.h, the macro layer, precedes this file in every unit. */
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* `LOFT_FAULT_LOG=<file>` — each store fault as one JSON line, the Rust runtime's
 * `fault_log` in the same format (`"backend":"c"`), beside the message printed anyway. */
static inline FILE *loft_fault_file(loft_cx *cx) {
    static int ready = 0;
    static FILE *f = NULL;
    if (!ready) {
        const char *p = getenv("LOFT_FAULT_LOG");
        ready = 1;
        if (p && *p) f = fopen(p, "a");
    }
    return f;
}
static inline void loft_fault_str(loft_cx *cx, FILE *f, const char *s, size_t n) {
    size_t i;
    fputc('"', f);
    for (i = 0; s && i < n && s[i]; i++) {
        unsigned char c = (unsigned char) s[i];
        if (c == '"' || c == '\\') {
            fputc('\\', f);
            fputc(c, f);
        } else if (c < 0x20) {
            fprintf(f, "\\u%04x", (unsigned) c);
        } else {
            fputc(c, f);
        }
    }
    fputc('"', f);
}
static inline void loft_fault_begin(loft_cx *cx, FILE *f, const char *kind) {
    fputs("{\"kind\":", f);
    loft_fault_str(cx, f, kind, strlen(kind));
    fputs(",\"backend\":\"c\"", f);
}
static inline void loft_fault_field(loft_cx *cx, FILE *f, const char *k, const char *v) {
    fputc(',', f);
    loft_fault_str(cx, f, k, strlen(k));
    fputc(':', f);
    loft_fault_str(cx, f, v ? v : "", v ? strlen(v) : 0);
}
static inline void loft_fault_end(loft_cx *cx, FILE *f) {
    fputs("}\n", f);
    fflush(f);
}
#include <errno.h>
#include <math.h>
#include <float.h>
#include <time.h>

/* `isnan` of a `float` (a loft `single`, whose null is NaN): the C library's macro may widen its
 * argument to a `double` first — newlib's does, and on a core with a single-precision FPU that
 * is a soft-float conversion and a soft-float compare for every null test of a single (@PLN217).
 * The compiler's own test compares the value in its own width, with the same answer. */
#if defined(__GNUC__) || defined(__clang__)
#undef isnan
#define isnan(x) __builtin_isnan(x)
#endif

#define LOFT_NULL INT64_MIN

#define LOFT_BOOL_NULL 255

static double loft_usage_pct(loft_cx *cx, uint64_t a, uint64_t b) {
    return b == 0 ? 0.0 : 100.0 * (double) a / (double) b;
}

/* `Stores::memory_report` (`store_memory()`): every live store's space, then the eight largest
 * by capacity (ties in store order) with their type's name from `names`; a compiled program
 * has no bytecode positions, so the creation site reads `bc:0` as on `--native`. */
static loft_text loft_store_memory(loft_cx *cx, const char *const *names, uint32_t nnames) {
    uint32_t live = 0;
    uint64_t cap = 0, data = 0, fr = 0, recs = 0, free_blk = 0, mergeable = 0;
    uint32_t *rows = (uint32_t *) malloc(sizeof(uint32_t) * (cx->nstores ? cx->nstores : 1));
    loft_usage *us = (loft_usage *) malloc(sizeof(loft_usage) * (cx->nstores ? cx->nstores : 1));
    if (!rows || !us) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint32_t i = 0; i < cx->nstores; i++) {
        if (!cx->stores[i].live) continue;
        loft_usage u = loft_st_usage(cx, &cx->stores[i]);
        us[i] = u;
        live++;
        cap += u.capacity_words;
        data += u.claimed_words;
        fr += u.free_words;
        recs += u.claimed_count;
        free_blk += u.free_count;
        mergeable += u.mergeable_free_pairs;
        /* A stable insertion by descending capacity, as `sort_by_key(Reverse(..))` orders. */
        uint32_t at = live - 1;
        while (at > 0 && us[rows[at - 1]].capacity_words < u.capacity_words) {
            rows[at] = rows[at - 1];
            at--;
        }
        rows[at] = i;
    }
    char line[512];
    loft_text out = { 0, 0, 0 };
#define LOFT_MB(w) ((double) (w) * 8.0 / 1048576.0)
    int n = snprintf(line, sizeof line,
        "stores: %u live | cap %.2f MB | data %.2f MB (%.0f%%) | free %.2f MB | records %llu | free-blocks %llu | mergeable-pairs %llu",
        live, LOFT_MB(cap), LOFT_MB(data), loft_usage_pct(cx, data, cap), LOFT_MB(fr),
        (unsigned long long) recs, (unsigned long long) free_blk, (unsigned long long) mergeable);
    loft_push(cx, &out, line, (size_t) n);
    for (uint32_t k = 0; k < live && k < 8; k++) {
        uint32_t i = rows[k];
        loft_usage u = us[i];
        uint16_t kt = LOFT_KNOWN(cx->stores[i]);
        const char *tname = kt < nnames && names[kt] ? names[kt] : "?";
        char idx[16];
        snprintf(idx, sizeof idx, "%u", i);
        n = snprintf(line, sizeof line,
            "\n  #%-5s %8.3f MB  used %3.0f%%  recs %-5u free-blk %-4u mergeable %-4u largest-free %-7uw  tail %3.0f%% inner %3.0f%%  type %s bc:0",
            idx, LOFT_MB(u.capacity_words), loft_usage_pct(cx, u.claimed_words, u.capacity_words),
            u.claimed_count, u.free_count, u.mergeable_free_pairs, u.largest_free_words,
            loft_usage_pct(cx, u.capacity_words > u.live_end_words ? u.capacity_words - u.live_end_words : 0, u.capacity_words),
            loft_usage_pct(cx, u.live_end_words > u.claimed_words ? u.live_end_words - u.claimed_words : 0, u.capacity_words),
            tname);
        loft_push(cx, &out, line, (size_t) n < sizeof line ? (size_t) n : sizeof line - 1);
    }
#undef LOFT_MB
    free(rows);
    free(us);
    return out;
}

/* The order of two texts, byte by byte, a shorter prefix first (Rust's `str` order). */
static int loft_str_cmp(loft_cx *cx, loft_str a, loft_str b) {
    size_t n = a.len < b.len ? a.len : b.len;
    int c = n ? memcmp(a.p, b.p, n) : 0;
    if (c != 0) return c;
    return a.len < b.len ? -1 : a.len > b.len;
}

/* OpLtText / OpLeText (ops::op_lt_text): the null text orders before every present one, and
 * nothing present orders before it. */
static int loft_text_lt(loft_cx *cx, loft_str a, loft_str b) {
    int an = loft_str_is_null(cx, a), bn = loft_str_is_null(cx, b);
    if (an || bn) return an && !bn;
    return loft_str_cmp(cx, a, b) < 0;
}

static int loft_text_le(loft_cx *cx, loft_str a, loft_str b) {
    int an = loft_str_is_null(cx, a);
    if (an || loft_str_is_null(cx, b)) return an;
    return loft_str_cmp(cx, a, b) <= 0;
}

/* The compare reads `len` bytes of both only where both hold `len`; gcc's overread check, which
 * cannot see a text's length beside its pointer (a parked work text made it look at the empty
 * literal's one byte), would fail strict mode on a read that does not run. */
#if defined(__GNUC__) && !defined(__clang__) && __GNUC__ >= 11
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wstringop-overread"
#endif
static int loft_str_eq(loft_cx *cx, loft_str a, loft_str b) {
    return a.len == b.len && (a.len == 0 || memcmp(a.p, b.p, a.len) == 0);
}
#if defined(__GNUC__) && !defined(__clang__) && __GNUC__ >= 11
#pragma GCC diagnostic pop
#endif

/* Unicode scalar values in `s`: the bytes that do not continue a UTF-8 sequence. */
static size_t loft_chars(loft_cx *cx, loft_str s) {
    size_t n = 0;
    for (size_t i = 0; i < s.len; i++)
        if (((unsigned char) s.p[i] & 0xC0) != 0x80) n++;
    return n;
}

/* ops::format_text */
static void loft_format_text(loft_cx *cx, loft_text *s, loft_str val, int64_t width, int dir, uint8_t token) {
    if (loft_str_is_null(cx, val)) val = loft_lit(cx, "null", 4);
    if (dir == 2) dir = -1;
    size_t tokens = width > 0 ? (size_t) width : 0;
    size_t chars = loft_chars(cx, val);
    tokens = chars >= tokens ? 0 : tokens - chars;
    if (dir < 0) {
        loft_text_append(cx, s, val);
        while (tokens > 0) { loft_push_char(cx, s, (char) token); tokens--; }
    } else if (dir > 0) {
        while (tokens > 0) { loft_push_char(cx, s, (char) token); tokens--; }
        loft_text_append(cx, s, val);
    } else {
        size_t ct = 0;
        while (ct < tokens / 2) { loft_push_char(cx, s, (char) token); ct++; }
        loft_text_append(cx, s, val);
        while (ct < tokens) { loft_push_char(cx, s, (char) token); ct++; }
    }
}

static uint8_t loft_null_pad(loft_cx *cx, uint8_t token) {
    return token == '0' ? ' ' : token;
}

/* ops::format_prefixed */
static void loft_format_prefixed(loft_cx *cx, loft_text *s, loft_str prefix, loft_str digits, int64_t width,
                                 int dir, uint8_t token) {
    if (token == '0' && prefix.len > 0) {
        loft_text_append(cx, s, prefix);
        /* a null width is no width: subtracting from it would wrap */
        loft_format_text(cx, s, digits, width == LOFT_NULL ? width : width - (int64_t) loft_chars(cx, prefix), dir, token);
        return;
    }
    loft_text res = { 0, 0, 0 };
    loft_text_append(cx, &res, prefix);
    loft_text_append(cx, &res, digits);
    loft_format_text(cx, s, loft_view(cx, &res), width, dir, token);
    loft_text_free(cx, &res);
}

/* ops::pad_rendered */
static void loft_pad_rendered(loft_cx *cx, loft_text *s, size_t start, size_t sign, int64_t width, int dir,
                              uint8_t token) {
    if (width <= (int64_t) (s->len - start)) return;
    loft_text rendered = loft_text_own(cx, loft_lit(cx, s->p + start, s->len - start));
    s->len = start;
    loft_format_prefixed(cx, s, loft_lit(cx, rendered.p, sign),
                         loft_lit(cx, rendered.p + sign, rendered.len - sign), width, dir, token);
    loft_text_free(cx, &rendered);
}

/* The decimal digits of `v`, most significant first, into `buf` (twenty at most): what `%llu`
 * writes, without parsing a format per number. */
static int loft_u64_dec(loft_cx *cx, char *buf, uint64_t v) {
    char tmp[20];
    int n = 0;
    do {
        tmp[n++] = (char) ('0' + (int) (v % 10));
        v /= 10;
    } while (v);
    for (int i = 0; i < n; i++) buf[i] = tmp[n - 1 - i];
    return n;
}

/* ops::push_decimal */
static void loft_push_decimal(loft_cx *cx, loft_text *s, int neg, uint64_t abs, int plus, int64_t width,
                              int dir, uint8_t token) {
    size_t start = s->len;
    if (neg) loft_push_char(cx, s, '-');
    else if (plus) loft_push_char(cx, s, '+');
    size_t sign = s->len - start;
    char buf[24];
    int n = loft_u64_dec(cx, buf, abs);
    loft_push(cx, s, buf, (size_t) n);
    loft_pad_rendered(cx, s, start, sign, width, dir, token);
}

#define LOFT_HEX_UPPER 17

/* ops::format_long */
static void loft_format_long(loft_cx *cx, loft_text *s, int64_t val, uint8_t radix, int64_t width,
                             uint8_t token, int plus, int note, int dir) {
    if (dir == 2) dir = 1;
    if (val == LOFT_NULL) {
        loft_format_text(cx, s, loft_lit(cx, "null", 4), width, dir, loft_null_pad(cx, token));
        return;
    }
    if (radix == 10) {
        uint64_t abs = val < 0 ? (uint64_t) 0 - (uint64_t) val : (uint64_t) val;
        loft_push_decimal(cx, s, val < 0, abs, plus, width, dir, token);
        return;
    }
    /* Rust's {:b} {:o} {:x} {:X} on an i64 print its two's-complement bits. */
    uint64_t u = (uint64_t) val;
    char buf[72];
    const char *prefix = "";
    int n;
    if (radix == 2) {
        char rev[64];
        int k = 0;
        do { rev[k++] = (char) ('0' + (u & 1)); u >>= 1; } while (u);
        for (n = 0; n < k; n++) buf[n] = rev[k - 1 - n];
        buf[n] = 0;
        prefix = note ? "0b" : "";
    } else if (radix == 8 || radix == 16 || radix == LOFT_HEX_UPPER) {
        /* The digits written from the end of `buf`, as `{:o}` / `{:x}` / `{:X}` print them. */
        const char *digits = radix == LOFT_HEX_UPPER ? "0123456789ABCDEF" : "0123456789abcdef";
        unsigned shift = radix == 8 ? 3 : 4;
        uint64_t mask = radix == 8 ? 7 : 15;
        char *end = buf + sizeof buf, *at = end;
        do { *--at = digits[u & mask]; u >>= shift; } while (u);
        n = (int) (end - at);
        memmove(buf, at, (size_t) n);
        buf[n] = 0;
        prefix = radix == 8 ? (note ? "0o" : "") : (note ? "0x" : "");
    } else {
        fputs("loft: unknown radix\n", stderr);
        abort();
    }
    loft_format_prefixed(cx, s, loft_lit(cx, prefix, strlen(prefix)), loft_lit(cx, buf, (size_t) n), width,
                         dir, token);
}

/* @PLN217 — the same for a value held in 32 bits.  Its decimal digits are taken in 32 bits:
 * a 64-bit divide by ten is a library call on a 32-bit core.  Null, and the bit-pattern
 * radixes, which print the 64 bits a negative value widens to, go to `loft_format_long`. */
static void loft_format_int32(loft_cx *cx, loft_text *s, int32_t val, uint8_t radix, int64_t width,
                              uint8_t token, int plus, int note, int dir) {
    if (val == INT32_MIN || radix != 10) {
        loft_format_long(cx, s, val == INT32_MIN ? LOFT_NULL : (int64_t) val, radix, width, token, plus, note, dir);
        return;
    }
    if (dir == 2) dir = 1;
    uint32_t abs = val < 0 ? 0u - (uint32_t) val : (uint32_t) val;
    size_t start = s->len;
    if (val < 0) loft_push_char(cx, s, '-');
    else if (plus) loft_push_char(cx, s, '+');
    size_t sign = s->len - start;
    char tmp[10], buf[10];
    int n = 0;
    do {
        tmp[n++] = (char) ('0' + (int) (abs % 10u));
        abs /= 10u;
    } while (abs);
    for (int i = 0; i < n; i++) buf[i] = tmp[n - 1 - i];
    loft_push(cx, s, buf, (size_t) n);
    loft_pad_rendered(cx, s, start, sign, width, dir, token);
}

/* print (codegen_runtime::host_print): a reader that stopped reading ends the program quietly
 * with 0 — `prog | head` is the ordinary end of a pipeline, not a fault.  A POSIX host ignores
 * SIGPIPE (`loft_start`), so the closed pipe arrives here as EPIPE.  A text holding a newline is
 * flushed, as Rust's line-writing stdout flushes it: a reader on a pipe sees each line when it is
 * printed, not when a block fills or the program ends. */
static void loft_print(loft_cx *cx, loft_str s) {
#if defined(EPIPE)
    if (fwrite(s.p, 1, s.len, stdout) < s.len && errno == EPIPE) LOFT_EXIT(0);
    if (memchr(s.p, '\n', s.len) && fflush(stdout) != 0 && errno == EPIPE) LOFT_EXIT(0);
#else
    fwrite(s.p, 1, s.len, stdout);
    if (memchr(s.p, '\n', s.len)) fflush(stdout);
#endif
}

/* c_library_available (c_libs::library_available): a library this unit carries — the names
 * the unit lists after the runtime (`c_libraries` in mod.rs), bound by its `main`. */
static const char *const *loft_cl = 0;
static uint32_t loft_ncl = 0;
/* Each carried library's declared symbols, which a POSIX host asks the running program for
 * (`dlsym` over its own handle): a library missing one is not available. */
static const char *const *const *loft_cls = 0;
/* Opening a library and finding a symbol in it, or in the running program (@PLN203 C5, C8):
 * `dlopen` / `dlsym` on a POSIX host; `LoadLibraryA` / `GetProcAddress` on Windows, where "the
 * program" is every module it has loaded (`dlopen(0)`'s global scope).  `LOFT_DL` says the host
 * can.  The Windows declarations carry `windows.h`'s own types, so a unit that also includes it
 * (a socket unit) sees the same functions — the header itself is not included here: its `near`
 * and `far` macros would rename the runtime's own parameters. */
#if defined(LOFT_POSIX_HOST)
#include <dlfcn.h>
#define LOFT_DL 1
/* `scope`: LOFT_DL_DEFAULT (the host's own default), LOFT_DL_GLOBAL or LOFT_DL_LOCAL. */
#define LOFT_DL_DEFAULT 0
#define LOFT_DL_GLOBAL 1
#define LOFT_DL_LOCAL 2
static void *loft_dl_open(loft_cx *cx, const char *path, int scope) {
    return dlopen(path, scope == LOFT_DL_GLOBAL ? RTLD_NOW | RTLD_GLOBAL
                        : scope == LOFT_DL_LOCAL ? RTLD_NOW | RTLD_LOCAL : RTLD_NOW);
}
static void *loft_dl_sym(loft_cx *cx, void *h, const char *sym) { return dlsym(h, sym); }
static void *loft_dl_self_sym(loft_cx *cx, const char *sym) {
    void *self = dlopen(0, RTLD_LAZY);
    return self ? dlsym(self, sym) : 0;
}
static const char *loft_dl_error(loft_cx *cx) {
    const char *e = dlerror();
    return e ? e : "unknown error";
}
#elif defined(_WIN32)
#define LOFT_DL 1
struct HINSTANCE__;
typedef intptr_t (__stdcall *loft_win_proc)(void);
__declspec(dllimport) struct HINSTANCE__ *__stdcall LoadLibraryA(const char *name);
__declspec(dllimport) loft_win_proc __stdcall GetProcAddress(struct HINSTANCE__ *module, const char *name);
__declspec(dllimport) void *__stdcall GetCurrentProcess(void);
__declspec(dllimport) int __stdcall K32EnumProcessModules(void *process, struct HINSTANCE__ **modules, unsigned long cb,
                                                           unsigned long *needed);
__declspec(dllimport) unsigned long __stdcall GetLastError(void);
/* A procedure address as the data pointer the callers hold (ISO C converts neither way). */
static void *loft_dl_proc(loft_cx *cx, loft_win_proc p) {
    void *v = 0;
    memcpy(&v, &p, sizeof v < sizeof p ? sizeof v : sizeof p);
    return v;
}
#define LOFT_DL_DEFAULT 0
#define LOFT_DL_GLOBAL 1
#define LOFT_DL_LOCAL 2
/* Windows has no scope: a symbol is always asked of its own module. */
static void *loft_dl_open(loft_cx *cx, const char *path, int scope) {
    (void) scope;
    return (void *) LoadLibraryA(path);
}
static void *loft_dl_sym(loft_cx *cx, void *h, const char *sym) {
    return loft_dl_proc(cx, GetProcAddress((struct HINSTANCE__ *) h, sym));
}
static void *loft_dl_self_sym(loft_cx *cx, const char *sym) {
    struct HINSTANCE__ *mods[512];
    unsigned long need = 0;
    if (!K32EnumProcessModules(GetCurrentProcess(), mods, (unsigned long) sizeof mods, &need)) return 0;
    size_t n = need / sizeof mods[0];
    for (size_t i = 0; i < n && i < 512; i++) {
        void *p = loft_dl_proc(cx, GetProcAddress(mods[i], sym));
        if (p) return p;
    }
    return 0;
}
static const char *loft_dl_error(loft_cx *cx) {
    static char why[64];
    snprintf(why, sizeof why, "Windows error %lu", GetLastError());
    return why;
}
#endif
/* `[c] optional-libs` (@PLN24 arc G): a library a package works without is never linked; it is
 * opened when a symbol of its package is first called, or when `c_library_available` asks.  Each
 * entry: its declared name, its package, where to look (the file the package ships, else the
 * soname's spellings) and its package's declared symbols.  Bound by a unit's `main`. */
typedef struct { const char *name, *pkg; const char *const *cands; const char *const *syms; } loft_c_optlib;
static const loft_c_optlib *loft_col = 0;
static uint32_t loft_ncol = 0;

#if defined(LOFT_DL)
/* The handle of optional library `i`, opened once; null when none of its candidates opens. */
static void *loft_c_optlib_open(loft_cx *cx, uint32_t i) {
    static void *handles[64];
    static uint8_t tried[64];
    if (i >= 64) return 0;
    if (!tried[i]) {
        tried[i] = 1;
        for (const char *const *c = loft_col[i].cands; *c && !handles[i]; c++) handles[i] = loft_dl_open(cx, *c, LOFT_DL_DEFAULT);
    }
    return handles[i];
}
#endif

/* @PLN195 — a library a shipped program (`--out`) opens at start through loft's search order,
 * as `native_bind::bind` does: its package, version and file, never a directory of the machine
 * that built it.  `shim` marks a `[c] shim`'s own library, which only a `--dynamic` build opens
 * (LOFT_BIND_SHIMS): a static one carries the shim inside.  Bound by a unit's `main`. */
typedef struct { const char *pkg, *version, *file; uint8_t shim; } loft_c_boundlib;
static const loft_c_boundlib *loft_cbl = 0;
static uint32_t loft_ncbl = 0;
#if defined(LOFT_DL)
static void *loft_cbl_handles[64];
static uint32_t loft_ncbl_handles = 0;
#endif
#if defined(LOFT_HOSTED) && defined(LOFT_DL)
/* A hosted unit has no `main` to bind its libraries: the interpreter opened the unit, and with
 * it the libraries it links (`loft_cl`, its own `[c] shim` among them).  Their handles join the
 * ones `loft_c_bound_sym` searches, so a `#c` symbol of a package with optional libraries,
 * reached by name (`loft_c_lazy_sym`), finds the unit's own shim — the program's global scope,
 * which a whole program exports its shims to, does not hold a unit's.  Called by
 * `loft_shared_init`. */
static void loft_host_bind_libraries(loft_cx *cx) {
    for (uint32_t i = 0; i < loft_ncl && loft_ncbl_handles < 64; i++) {
        void *h = loft_dl_open(cx, loft_cl[i], LOFT_DL_LOCAL);
        if (h) loft_cbl_handles[loft_ncbl_handles++] = h;
    }
}
#endif

/* A `#c` symbol in this program (its linked libraries, its shims), else in a library
 * `loft_c_bind_libraries` opened.  Null when nothing exports it. */
static void *loft_c_bound_sym(loft_cx *cx, const char *sym) {
#if defined(LOFT_DL)
    void *p = loft_dl_self_sym(cx, sym);
    for (uint32_t i = 0; !p && i < loft_ncbl_handles; i++) p = loft_dl_sym(cx, loft_cbl_handles[i], sym);
    return p;
#else
    (void) sym;
    return 0;
#endif
}

/* `c_libs::resolve`: a `#c` symbol of a package with optional libraries — in this program
 * (its linked libraries, its shims), else in one of the package's optional libraries, opened
 * now.  Null when nothing exports it. */
static void *loft_c_lazy_sym(loft_cx *cx, const char *sym, const char *pkg) {
#if defined(LOFT_DL)
    void *p = loft_c_bound_sym(cx, sym);
    for (uint32_t i = 0; !p && i < loft_ncol; i++) {
        if (strcmp(loft_col[i].pkg, pkg) != 0) continue;
        void *h = loft_c_optlib_open(cx, i);
        if (h) p = loft_dl_sym(cx, h, sym);
    }
    return p;
#else
    (void) sym;
    (void) pkg;
    return 0;
#endif
}

static void loft_panic(loft_cx *cx, loft_str message, loft_str file, int64_t line);

/* `c_libs::missing_symbol_message`: a call of a symbol nothing exports halts, saying why. */
static void loft_c_missing_symbol(loft_cx *cx, const char *sym) {
    char m[512];
    snprintf(m, sizeof m,
             "`#c` symbol '%s' not found — it is not in this process and no loaded library exports it. If it "
             "comes from an `[c] optional-libs` library, that library is not installed: ask "
             "`c_library_available(\"<soname>\")` before calling. Otherwise declare the library it comes from, "
             "or check the spelling",
             sym);
    loft_panic(cx, loft_lit(cx, m, strlen(m)), loft_lit(cx, "", 0), 0);
}

static uint8_t loft_c_library_available(loft_cx *cx, loft_str name) {
    for (uint32_t i = 0; i < loft_ncl; i++) {
        if (strlen(loft_cl[i]) == name.len && memcmp(loft_cl[i], name.p, name.len) == 0) {
#if defined(LOFT_DL)
            if (loft_cls && loft_cls[i]) {
                for (const char *const *s = loft_cls[i]; *s; s++) {
                    if (!loft_c_bound_sym(cx, *s)) return 0;
                }
            }
#endif
            return 1;
        }
    }
#if defined(LOFT_DL)
    /* An optional library: available when it opens and exports every symbol its package
     * declares (from it or from what the program already has). */
    for (uint32_t i = 0; i < loft_ncol; i++) {
        if (strlen(loft_col[i].name) != name.len || memcmp(loft_col[i].name, name.p, name.len) != 0) continue;
        void *h = loft_c_optlib_open(cx, i);
        if (!h) return 0;
        for (const char *const *s = loft_col[i].syms; *s; s++)
            if (!loft_dl_sym(cx, h, *s) && !loft_c_bound_sym(cx, *s)) return 0;
        return 1;
    }
#endif
    return 0;
}

/* eprint (platform::host_eprint): the text to standard error, unbuffered as Rust's is. */
static void loft_eprint(loft_cx *cx, loft_str s) {
    fwrite(s.p, 1, s.len, stderr);
}

/* The run's exit status: 1 once a fault was fatal, or surfaced under `--dev-soft-halt`. */
static int loft_had_fatal = 0;

/* `--dev-soft-halt` (`LOFT_DEV_SOFT_HALT=1`, `ops::dev_soft_halt`): every recoverable fault is
 * surfaced on stderr as it happens and the run ends non-zero, as the interpreter does — a
 * triage flag; `(E-Report)` is otherwise unchanged. */
static int loft_soft_halt_flag = -1;
static LOFT_COLD int loft_soft_halt(loft_cx *cx) {
    if (loft_soft_halt_flag < 0) {
        const char *v = getenv("LOFT_DEV_SOFT_HALT");
        loft_soft_halt_flag = v && (strcmp(v, "1") == 0 || strcmp(v, "true") == 0);
    }
    return loft_soft_halt_flag;
}

/* An undefended integer overflow (`ops::note_integer_overflow`): the null it answers, surfaced
 * under `--dev-soft-halt`.  Reached only from the overflow arm the checked operators already
 * have, so an operation that fits pays nothing; the `??`-defended (`Nullable`) forms never call
 * it. */
static LOFT_COLD loft_word loft_ovf(loft_cx *cx, const char *op, loft_word a, loft_word b) {
    if (loft_soft_halt(cx)) {
        loft_had_fatal = 1;
        fprintf(stderr, "soft-halt: integer overflow: %lld %s %lld\n", (long long) a, op,
                (long long) b);
    }
    return LOFT_NULL;
}

/* The integer operators: null in, null out; an overflow or a division by zero is null. */
/* The checked operators: a null operand or an overflow answers null.  The multiply of two
 * computed operands asks LOFT_MUL_OVF (a flag test where the portable check divides); with a
 * constant operand the portable check folds to one compare, cheaper still.  Addition and
 * subtraction keep the portable comparison: an overflow builtin hides the operands' ranges
 * from the optimiser (measured on clang: a masked accumulator's loop ran 2.7x slower). */
/* `@FR-R-GuardedChain`'s guard arithmetic: the magnitude bound of a chain, each step clearing
 * `*ok` where it does not fit (`checked_add`, `checked_mul`, `checked_abs` — the last fails on the
 * null sentinel, which has no absolute value).  The value after a failure is never read. */
static int64_t loft_gc_abs(loft_cx *cx, int64_t a, int *ok) {
    if (a == LOFT_NULL) { *ok = 0; return 0; }
    return a < 0 ? -a : a;
}
static int64_t loft_gc_add(loft_cx *cx, int64_t a, int64_t b, int *ok) {
    int64_t r;
    if (LOFT_ADD_OVF(a, b, &r)) { *ok = 0; return 0; }
    return r;
}
static int64_t loft_gc_mul(loft_cx *cx, int64_t a, int64_t b, int *ok) {
    int64_t r;
    if (LOFT_MUL_OVF(a, b, &r)) { *ok = 0; return 0; }
    return r;
}
static int64_t loft_gc_max(loft_cx *cx, int64_t a, int64_t b) { return a > b ? a : b; }

/* The checked `+` and `-`: null in, null out, and null where the result does not fit — the
 * compiler's overflow builtin (`LOFT_ADD_OVF`), which both GCC and Clang lower to one flag test,
 * where Clang kept the spelled-out range comparisons as compares and branches. */
LOFT_HOT_INLINE int64_t loft_add(loft_cx *cx, int64_t a, int64_t b) {
    int64_t r;
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    return LOFT_ADD_OVF(a, b, &r) ? LOFT_NULL : r;
}

LOFT_HOT_INLINE int64_t loft_sub(loft_cx *cx, int64_t a, int64_t b) {
    int64_t r;
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    return LOFT_SUB_OVF(a, b, &r) ? LOFT_NULL : r;
}

LOFT_HOT_INLINE int64_t loft_mul(loft_cx *cx, int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    if (!LOFT_CONSTANT(a) && !LOFT_CONSTANT(b)) {
        int64_t r;
        return LOFT_MUL_OVF(a, b, &r) ? LOFT_NULL : r;
    }
    /* One operand is a constant: make it `b`, and every bound below divides by it, so it folds
     * to a compare.  With the constant on the left, `INT64_MAX / b` divided by the other operand
     * at run time — an `idiv` per multiply (`lerp_int`'s `(b - a) * i`). */
    if (LOFT_CONSTANT(a) && !LOFT_CONSTANT(b)) {
        int64_t t = a;
        a = b;
        b = t;
    }
    if (b > 0) {
        if (a > INT64_MAX / b || a < INT64_MIN / b) return LOFT_NULL;
    } else if (b < -1) {
        if (a < INT64_MAX / b || a > INT64_MIN / b) return LOFT_NULL;
    }
    /* b == -1 overflows only for INT64_MIN, which is null and answered above. */
    return a * b;
}

/* The UNDEFENDED `+ - *` (`OpAddInt`, not `OpAddIntNullable`): the same values, and an overflow
 * surfaced under `--dev-soft-halt` (`loft_ovf`). */
LOFT_HOT_INLINE loft_word loft_add_r(loft_cx *cx, loft_word a, loft_word b) {
    loft_word r;
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    return LOFT_ADD_OVF(a, b, &r) ? loft_ovf(cx, "+", a, b) : r;
}

LOFT_HOT_INLINE loft_word loft_sub_r(loft_cx *cx, loft_word a, loft_word b) {
    loft_word r;
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    return LOFT_SUB_OVF(a, b, &r) ? loft_ovf(cx, "-", a, b) : r;
}

LOFT_HOT_INLINE loft_word loft_mul_r(loft_cx *cx, loft_word a, loft_word b) {
    loft_word r = loft_mul(cx, a, b);
    if (LOFT_UNLIKELY(r == LOFT_NULL) && a != LOFT_NULL && b != LOFT_NULL) return loft_ovf(cx, "*", a, b);
    return r;
}

LOFT_HOT_INLINE int64_t loft_div(loft_cx *cx, int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b == 0) return LOFT_NULL;
    return a / b;
}

LOFT_HOT_INLINE int64_t loft_rem(loft_cx *cx, int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b == 0) return LOFT_NULL;
    return a % b;
}

/* @PLN204 — the 32-bit integers (`CType::Int32`): `int32_t`, null `INT32_MIN`.  `loft_w64`
 * widens one where a 64-bit value is wanted; `loft_n32` narrows a 64-bit value, a value that
 * does not fit reading null as `OpNarrow32` answers it.  The operators answer what the
 * interpreter's `OpNarrow32(OpAddInt(a, b))` answers: a null operand or a result outside
 * `INT32_MIN + 1 ..= INT32_MAX` is null.  Taken in 64 bits only where 32 cannot hold the
 * intermediate (a product); a sum is checked against the range before it is formed. */
#define LOFT_NULL32 INT32_MIN
LOFT_HOT_INLINE int64_t loft_w64(loft_cx *cx, int32_t a) { return a == LOFT_NULL32 ? LOFT_NULL : (int64_t) a; }
LOFT_HOT_INLINE int32_t loft_n32(loft_cx *cx, int64_t a) {
    return (a > (int64_t) INT32_MIN && a <= (int64_t) INT32_MAX) ? (int32_t) a : LOFT_NULL32;
}
/* @PLN217 — the build's integer (`loft_int`, loft_width.h) against the 64-bit one, null kept
 * as null: `loft_int_wide` reads a `loft_int` where 64 bits are wanted, `loft_int_of` holds a
 * 64-bit value in one (a value outside it reads null).  Both are the value itself in a 64-bit
 * build.  The runtime takes and answers `loft_int` wherever the value is a loft `integer` — a
 * length, an index, a count — so in a 32-bit build no 64-bit arithmetic is done for one; what
 * is 64 bits in every build (an `i64`, a file offset, a timestamp, a hash) is `loft_word`. */
#if defined(LOFT_WIDTH32)
#define loft_int_wide(a) loft_w64(cx, a)
#define loft_int_of(a) loft_n32(cx, a)
#else
#define loft_int_wide(a) (a)
#define loft_int_of(a) (a)
#endif
LOFT_HOT_INLINE int32_t loft_add32(loft_cx *cx, int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32) return LOFT_NULL32;
    if ((b > 0 && a > INT32_MAX - b) || (b < 0 && a <= INT32_MIN - b)) return LOFT_NULL32;
    return a + b;
}
LOFT_HOT_INLINE int32_t loft_sub32(loft_cx *cx, int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32) return LOFT_NULL32;
    if ((b < 0 && a > INT32_MAX + b) || (b > 0 && a <= INT32_MIN + b)) return LOFT_NULL32;
    return a - b;
}
LOFT_HOT_INLINE int32_t loft_mul32(loft_cx *cx, int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32) return LOFT_NULL32;
    return loft_n32(cx, (int64_t) a * (int64_t) b);
}
LOFT_HOT_INLINE int32_t loft_div32(loft_cx *cx, int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32 || b == 0) return LOFT_NULL32;
    return a / b; /* INT32_MIN is null, so INT32_MIN / -1 is never reached */
}
LOFT_HOT_INLINE int32_t loft_rem32(loft_cx *cx, int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32 || b == 0) return LOFT_NULL32;
    return a % b;
}

/* The release-pass probe's forms (`LOFT_RELEASE_PASS_PROBE=1`, @PLN201): the processor's
 * wrapping arithmetic, as `generation::ops::int_arith::probe_form` emits it for Rust — a
 * MEASUREMENT of the ship tier's ceiling, never a build anyone ships (@C129).  Each wrap is
 * DEFINED here: the sum, difference, product and negation are taken in unsigned arithmetic,
 * and `INT64_MIN / -1` answers what Rust's `wrapping_div` answers; a zero divisor still
 * answers null, as the probe keeps it. */
LOFT_HOT_INLINE int64_t loft_wrap_add(loft_cx *cx, int64_t a, int64_t b) { return (int64_t) ((uint64_t) a + (uint64_t) b); }
LOFT_HOT_INLINE int64_t loft_wrap_sub(loft_cx *cx, int64_t a, int64_t b) { return (int64_t) ((uint64_t) a - (uint64_t) b); }
LOFT_HOT_INLINE int64_t loft_wrap_mul(loft_cx *cx, int64_t a, int64_t b) { return (int64_t) ((uint64_t) a * (uint64_t) b); }
LOFT_HOT_INLINE int64_t loft_wrap_neg(loft_cx *cx, int64_t a) { return (int64_t) (0 - (uint64_t) a); }
LOFT_HOT_INLINE int64_t loft_wrap_div(loft_cx *cx, int64_t a, int64_t b) {
    if (b == 0) return LOFT_NULL;
    return b == -1 ? loft_wrap_neg(cx, a) : a / b;
}
LOFT_HOT_INLINE int64_t loft_wrap_rem(loft_cx *cx, int64_t a, int64_t b) {
    if (b == 0) return LOFT_NULL;
    return b == -1 ? 0 : a % b;
}

/* `assert`: a failed one ends the running test (the test main catches it) or, outside a
 * test, the program, with the interpreter's message on stderr and exit status 1. */

/* The message of the last failed assert, for a test expected to fail with it. */

/* The texts the running frames hold (loft#2198), in a unit that runs tests: a fault that ends a
 * test `longjmp`s past every scope-exit free, so each function names the slots its epilogue
 * frees (`LOFT_HOLD`), its wrapper drops them again on return (`LOFT_HOLD_MARK`/`_BACK`), and a
 * halt releases what the abandoned frames still hold before it jumps — while those frames are
 * still on the stack.  A unit that binds a lazy collection keeps them too, since a driver's fault
 * is contained and the program goes on (`cx->lazy_held_floor`).  Every other unit compiles this
 * to nothing: there a halt ends the process. */
#ifdef LOFT_HALT_RELEASE
typedef struct loft_held_frame { loft_text *const *slot; size_t n; } loft_held_frame;
static void loft_hold(loft_cx *cx, loft_text *const *slot, size_t n) {
    if (cx->nheld == cx->capheld) {
        cx->capheld = cx->capheld ? cx->capheld * 2 : 64;
        cx->held = (loft_held_frame *) realloc(cx->held, cx->capheld * sizeof *cx->held);
        if (!cx->held) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    cx->held[cx->nheld].slot = slot;
    cx->held[cx->nheld].n = n;
    cx->nheld++;
}
/* Free the texts of every frame above `floor`, innermost first; a slot already freed is empty. */
static void loft_release_held(loft_cx *cx, size_t floor) {
    while (cx->nheld > floor) {
        loft_held_frame *f = &cx->held[--cx->nheld];
        for (size_t i = 0; i < f->n; i++) loft_text_free(cx, f->slot[i]);
    }
}
#define LOFT_HOLD(n, ...) loft_text *const loft_hd[] = { __VA_ARGS__ }; loft_hold(cx, loft_hd, n)
#define LOFT_HOLD_MARK(m) size_t m = cx->nheld
#define LOFT_HOLD_BACK(m) (cx->nheld = (m))
#define LOFT_RELEASE_HELD(floor) loft_release_held(cx, floor)
/* The depth a contained fault releases down to: the frames under the lazy driver keep theirs. */
#else
#define LOFT_HOLD(n, ...) ((void) 0)
#define LOFT_HOLD_MARK(m) ((void) 0)
#define LOFT_HOLD_BACK(m) ((void) 0)
#define LOFT_RELEASE_HELD(floor) ((void) 0)
#endif

static void loft_fault_where(loft_cx *cx, loft_str file, int64_t line);
/* A fault inside a lazy driver is CONTAINED (`call_frames::in_lazy_driver`): it ends the driver,
 * not the program — `loft_lazy_miss` set the jump and records `<kind>: <prefix><message>` as the
 * collection's failure, the message as the fault reads (`assertion failed: …`, `panic: …`). */
static void loft_lazy_contain(loft_cx *cx, const char *label, const char *prefix, loft_str message) {
    cx->lazy_fault.len = 0;
    loft_push(cx, &cx->lazy_fault, label, strlen(label));
    loft_push(cx, &cx->lazy_fault, ": ", 2);
    loft_push(cx, &cx->lazy_fault, prefix, strlen(prefix));
    loft_push(cx, &cx->lazy_fault, message.p, message.len);
    LOFT_RELEASE_HELD(cx->lazy_held_floor);
    LOFT_LONGJMP(*cx->lazy_jmp);
}

/* loft#1056 — a halting fault is the PROGRAM's halt, reported once however many `par` workers
 * reach it together, as the interpreter's `REPORTING` lock makes it: the first to claim the halt
 * prints and ends the process; a later one waits for that end.  Not under the test runner, whose
 * faults jump back to the test rather than end anything. */
#if defined(LOFT_MSVC)
#include <intrin.h>
#endif
static volatile long loft_halt_claimed = 0;
static void loft_halt_claim(loft_cx *cx) {
    long was;
    if (cx->in_test) return;
#if defined(LOFT_GNU)
    was = __atomic_exchange_n(&loft_halt_claimed, 1L, __ATOMIC_SEQ_CST);
#elif defined(LOFT_MSVC)
    was = _InterlockedExchange(&loft_halt_claimed, 1L);
#else
    was = loft_halt_claimed;
    loft_halt_claimed = 1;
#endif
    if (was)
        for (;;) (void) loft_halt_claimed;
}

/* A production run's fault logger (`loft_log_production_fault`), bound by a unit's `main`; null
 * in a runtime built without one, where every fault halts. */
static int (*loft_fault_logger)(loft_cx *cx, int sev, const char *label, const char *prefix, loft_str message,
                                loft_str file, int64_t line) = 0;

static void loft_assert(loft_cx *cx, uint8_t test, loft_str message, loft_str file, int64_t line) {
    if (test == 1) return;
    if (cx->lazy_jmp) loft_lazy_contain(cx, "assertion_failed", "assertion failed: ", message);
    if (!cx->in_test && loft_fault_logger &&
        loft_fault_logger(cx, 2, "assertion_failed", "assertion failed: ", message, file, line))
        return;
    loft_halt_claim(cx);
    fflush(stdout);
    fputs("error: assertion failed: ", stderr);
    fwrite(message.p, 1, message.len, stderr);
    fputc('\n', stderr);
    if (!cx->in_test) loft_fault_where(cx, file, line);
    cx->fail_msg.len = 0;
    loft_push(cx, &cx->fail_msg, "assertion failed: ", 18);
    loft_push(cx, &cx->fail_msg, message.p, message.len);
    if (cx->in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(cx->test_jmp); }
    fflush(stdout);
    LOFT_EXIT(1);
}

/* A production run's discard of a locked write (`loft_log_locked_write`), bound by a unit's
 * `main`; null in a runtime built without one, where every locked write halts. */
static int (*loft_locked_logger)(loft_cx *cx, const char *advice) = 0;

/* What locked a store (`Store::lock_origin`), kept in its `read_only` byte: the author's
 * `#lock = true`, a constant (`Store::lock_constant`), or bytes the program does not own
 * (`Store::FOREIGN_ORIGIN`).  Any of them is non-zero, which is all a write test asks. */
#define LOFT_RO_LOCK 1
#define LOFT_RO_CONST 2
#define LOFT_RO_FOREIGN 3

/* `runtime_error::locked_write_advice`: what a refused write tells its author, by what locked
 * the store — a constant and foreign bytes are cured by a copy, never by `#lock = false`. */
static const char *loft_locked_advice(loft_cx *cx, uint8_t kind) {
    switch (kind) {
    case LOFT_RO_CONST:
        return "write to a constant \xe2\x80\x94 a constant is read-only; bind it to a local first "
               "(`v = NAMES`), which copies, and write through the local";
    case LOFT_RO_FOREIGN:
        return "write to bytes the program does not own (a mapped file, a library's buffer, or a "
               "slice of either) \xe2\x80\x94 they are read-only; copy them first (`w = v`, a bind "
               "copies) and write the copy";
    default:
        return "write to a locked store \xe2\x80\x94 unlock it with `#lock = false` before writing";
    }
}

/* `@FR-H-WriteLocked` (`RuntimeError::locked_store_write`): a development run stops at a write
 * into a read-only store, as the interpreter does; a test fails.  A production run discards the
 * write, logs it, and continues on the store's old bytes (`discard_locked_write_in_production`,
 * `@C80`): the answer is 1, and the caller skips it.  `kind` is the store's `read_only` byte. */
static int loft_locked_write_on(loft_cx *cx, uint8_t kind) {
    const char *msg = loft_locked_advice(cx, kind);
    /* `@FR-R-NoLocks`: a unit that makes no store read-only never gets here, and without the
     * discard every path below ends the run — so the compiler knows a write's lock test never
     * returns into the code after it, and keeps what that code holds in registers. */
#ifndef LOFT_NO_LOCKS
    if (!cx->in_test && loft_locked_logger && loft_locked_logger(cx, msg)) return 1;
#endif
    loft_halt_claim(cx);
    fflush(stdout);
    fputs("error: ", stderr);
    fputs(msg, stderr);
    fputc('\n', stderr);
    cx->fail_msg.len = 0;
    loft_push(cx, &cx->fail_msg, msg, strlen(msg));
    if (cx->in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(cx->test_jmp); }
    fflush(stdout);
    LOFT_EXIT(1);
}
#define loft_locked_write() loft_locked_write_on(cx, LOFT_RO_LOCK)

/* The call stack ran past `LOFT_MAX_CALL_DEPTH` (`RuntimeError::stack_overflow`): the program
 * stops as a failed assertion stops it, at `line` of `file`. */
#ifdef LOFT_MAX_CALL_DEPTH
static void loft_stack_overflow(loft_cx *cx, const char *file, uint32_t line) {
    char msg[96];
    int n = snprintf(msg, sizeof msg, "call stack overflow \xe2\x80\x94 exceeded %d stack frames",
                     (int) LOFT_MAX_CALL_DEPTH);
    /* Inside a lazy driver it ends the driver, not the program, as an assertion does. */
    if (cx->lazy_jmp) loft_lazy_contain(cx, "stack_overflow", "", loft_lit(cx, msg, (size_t) n));
    loft_halt_claim(cx);
    fflush(stdout);
    fputs("error: ", stderr);
    fwrite(msg, 1, (size_t) n, stderr);
    fputc('\n', stderr);
    if (!cx->in_test) loft_fault_where(cx, loft_lit(cx, file, strlen(file)), line);
    cx->fail_msg.len = 0;
    loft_push(cx, &cx->fail_msg, msg, (size_t) n);
    if (cx->in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(cx->test_jmp); }
    fflush(stdout);
    LOFT_EXIT(1);
}
#endif

#ifdef LOFT_CALL_CHAIN
/* The frames the program is running (call_frames.rs, `cr_call_push`): each function's name and
 * file, and the line it is calling from (`cr_call_site`), outermost first. */
typedef struct loft_frame { const char *fn, *file; uint32_t site, decl; } loft_frame;
/* The frames below this one are the caller's while the thread runs a `par` slice itself: a fault
 * there names the worker's own frames, as one on a thread of its own does (loft#1056). */
#ifdef LOFT_MAX_CALL_DEPTH
static void loft_stack_overflow(loft_cx *cx, const char *file, uint32_t line);
#endif

static void loft_frame_push(loft_cx *cx, const char *fn, const char *file, uint32_t decl) {
#ifdef LOFT_MAX_CALL_DEPTH
    /* The call-depth cap (`call_frames::cr_call_push`): checked BEFORE the push, so a refused
     * call never runs, and reported at the declaration of the function that is running — the
     * frame below — as the interpreter's `State::fn_call` reports it (loft#1058). */
    if (cx->nframes >= LOFT_MAX_CALL_DEPTH) {
        const loft_frame *below = cx->nframes ? &cx->frames[cx->nframes - 1] : 0;
        loft_stack_overflow(cx, below ? below->file : file, below ? below->decl : decl);
    }
#endif
    if (cx->nframes == cx->capframes) {
        cx->capframes = cx->capframes ? cx->capframes * 2 : 64;
        cx->frames = (loft_frame *) realloc(cx->frames, cx->capframes * sizeof *cx->frames);
        if (!cx->frames) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    cx->frames[cx->nframes].fn = fn;
    cx->frames[cx->nframes].file = file;
    cx->frames[cx->nframes].site = 0;
    cx->frames[cx->nframes].decl = decl;
    cx->nframes++;
}

static void loft_frame_pop(loft_cx *cx) {
    if (cx->nframes) cx->nframes--;
}

static void loft_frame_site(loft_cx *cx, uint32_t line) {
    if (cx->nframes) cx->frames[cx->nframes - 1].site = line;
}
#ifdef LOFT_ALLOC_TRACE
/* `LOFT_ALLOC_TRACE`'s sites (`store_census::alloc_event`): `@ <line>` before an event whose
 * source line — the running function's — is not the last one written.  The writer records a
 * line only before a statement that can fault, so a function that has none gives its
 * declaration instead, written `~<line>`. */
static void loft_trace_site(loft_cx *cx, FILE *f) {
    if (!cx->nframes) return;
    const loft_frame *fr = &cx->frames[cx->nframes - 1];
    uint32_t key = fr->site ? fr->site : fr->decl | 0x80000000u;
    if ((key & 0x7FFFFFFFu) == 0 || key == cx->trace_last_site) return;
    cx->trace_last_site = key;
    fprintf(f, fr->site ? "@ %u\n" : "@ ~%u\n", (unsigned) (key & 0x7FFFFFFFu));
}
static void (*loft_trace_site_hook)(loft_cx *cx, FILE *f) = loft_trace_site;
#endif
/* A test that failed left its frames pushed (`longjmp`); the next starts from none. */
#define LOFT_FRAMES_RESET() (cx->nframes = 0)
#define LOFT_HDEPTH ((uint32_t) cx->nframes)
#else
#define LOFT_FRAMES_RESET() ((void) 0)
#ifdef LOFT_HOSTED
/* `@PLN206 P7` — a hosted unit's call depth with no call chain to read it from: what tells a
 * routine it is the one the interpreter called (`loft_pause_asked`). */
#define loft_frame_push(cx, fn, file, decl) ((void) (cx)->hdepth++)
#define loft_frame_pop(cx) ((void) (cx)->hdepth--)
#define LOFT_HDEPTH cx->hdepth
#elif defined(LOFT_MAX_CALL_DEPTH)
/* The lean tier's frame entry (`call_frames::cr_call_push_lean`, the rustc writer's `--ship`):
 * the call-depth cap without the frame record — checked before the push, at the same cap, the
 * report naming this call's own entry site (there is no frame below to read).  The count is the
 * running thread's: a field of its store context. */
#define loft_frame_push(cx, fn, file, decl) \
    (LOFT_UNLIKELY((cx)->cdepth >= (uint32_t) LOFT_MAX_CALL_DEPTH) ? loft_stack_overflow(cx, (file), (decl)) \
                                                                    : (void) (cx)->cdepth++)
#define loft_frame_pop(cx) ((void) ((cx)->cdepth > 0 && (cx)->cdepth--))
#else
#define loft_frame_push(cx, fn, file, decl) ((void) 0)
#define loft_frame_pop(cx) ((void) 0)
#endif
#define loft_frame_site(cx, line) ((void) 0)
#endif

/* The frames a halting fault fired under (`RuntimeError::render_call_chain`): innermost first,
 * five at most with a count of the rest; nothing for a single frame, whose `-->` line already
 * says where. */
static void loft_fault_chain(loft_cx *cx) {
#ifdef LOFT_CALL_CHAIN
    size_t top = cx->nframes, n = top - cx->frames_floor, shown = n < 5 ? n : 5;
    if (n <= 1) return;
    fprintf(stderr, "  in fn %s() \xe2\x86\x90 called from\n", cx->frames[top - 1].fn);
    for (size_t i = 1; i < shown; i++) fprintf(stderr, "        fn %s()\n", cx->frames[top - 1 - i].fn);
    if (n > 5) fprintf(stderr, "        \xe2\x80\xa6 (%lu more frames)\n", (unsigned long) (n - 5));
#endif
}

/* Where a halting fault fired (`render_entry_pretty` for a fault at column 1): the `-->` line,
 * then the source line and its caret, read from the file now as `FileSourceLoader` reads it
 * (a tab shown as four spaces), then the call chain. */
static void loft_fault_where(loft_cx *cx, loft_str file, int64_t line) {
    if (file.len && line > 0) {
        char *path = (char *) malloc(file.len + 1);
        FILE *f;
        int digits = 1;
        int64_t at = 1;
        int c;
        if (!path) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(path, file.p, file.len);
        path[file.len] = 0;
        fprintf(stderr, "  --> %s:%lld:1\n", path, (long long) line);
        f = fopen(path, "rb");
        free(path);
        for (int64_t v = line; v >= 10; v /= 10) digits++;
        if (f) {
            loft_text text = { 0, 0, 0 };
            while (at < line && (c = fgetc(f)) != EOF) if (c == '\n') at++;
            if (at == line) {
                while ((c = fgetc(f)) != EOF && c != '\n') {
                    if (c == '\t') loft_push(cx, &text, "    ", 4);
                    else { char ch = (char) c; loft_push(cx, &text, &ch, 1); }
                }
                if (text.len && text.p[text.len - 1] == '\r') text.len--;
                if (c != EOF || text.len) {
                    fprintf(stderr, "%*s |\n%lld | ", digits, "", (long long) line);
                    fwrite(text.p, 1, text.len, stderr);
                    fprintf(stderr, "\n%*s | ^\n", digits, "");
                }
            }
            free(text.p);
            fclose(f);
        }
    }
    loft_fault_chain(cx);
}

/* panic(message): the program stops as a failed assertion stops it, saying `panic:`. */
static void loft_panic(loft_cx *cx, loft_str message, loft_str file, int64_t line) {
    if (cx->lazy_jmp) loft_lazy_contain(cx, "user_panic", "panic: ", message);
    if (!cx->in_test && loft_fault_logger && loft_fault_logger(cx, 3, "user_panic", "panic: ", message, file, line))
        return;
    loft_halt_claim(cx);
    fflush(stdout);
    fputs("error: panic: ", stderr);
    fwrite(message.p, 1, message.len, stderr);
    fputc('\n', stderr);
    if (!cx->in_test) loft_fault_where(cx, file, line);
    cx->fail_msg.len = 0;
    loft_push(cx, &cx->fail_msg, "panic: ", 7);
    loft_push(cx, &cx->fail_msg, message.p, message.len);
    if (cx->in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(cx->test_jmp); }
    fflush(stdout);
    LOFT_EXIT(1);
}

/* exit(code) (runtime_error::exit_program): the program ends in silence with the code's low
 * byte as its status; under the test runner it ends the test as a failure. */
static void loft_exit(loft_cx *cx, int64_t code) {
    if (cx->in_test) {
        char m[48];
        sprintf(m, "exit %lld", (long long) code);
        cx->fail_msg.len = 0;
        loft_push(cx, &cx->fail_msg, m, strlen(m));
        LOFT_RELEASE_HELD(0);
        LOFT_LONGJMP(cx->test_jmp);
    }
    fflush(stdout);
    fflush(stderr);
    LOFT_EXIT((int) ((uint64_t) code & 0xFF));
}

/* True when the last failure's message holds one of `want`. */
static int loft_failed_with(loft_cx *cx, const loft_str *want, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (want[i].len == 0) return 1;
        for (size_t at = 0; at + want[i].len <= cx->fail_msg.len; at++) {
            if (memcmp(cx->fail_msg.p + at, want[i].p, want[i].len) == 0) return 1;
        }
    }
    return 0;
}

/* The bit operators (ops::op_logical_and_long …): null when either side is; a shift by an
 * amount outside 0..63 is null. */
static int64_t loft_and(loft_cx *cx, int64_t a, int64_t b) { return a == LOFT_NULL || b == LOFT_NULL ? LOFT_NULL : (a & b); }
static int64_t loft_or(loft_cx *cx, int64_t a, int64_t b) { return a == LOFT_NULL || b == LOFT_NULL ? LOFT_NULL : (a | b); }
static int64_t loft_xor(loft_cx *cx, int64_t a, int64_t b) { return a == LOFT_NULL || b == LOFT_NULL ? LOFT_NULL : (a ^ b); }
static int64_t loft_shl(loft_cx *cx, int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b < 0 || b > 63) return LOFT_NULL;
    return (int64_t) ((uint64_t) a << b);
}
static int64_t loft_shr(loft_cx *cx, int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b < 0 || b > 63) return LOFT_NULL;
    return a >> b;
}

/* OpCastTextFromBool: the null boolean is the null text. */
static loft_str loft_bool_text(loft_cx *cx, uint8_t b) {
    return b == 1 ? loft_lit(cx, "true", 4) : b == 255 ? loft_null_text(cx) : loft_lit(cx, "false", 5);
}

static size_t loft_utf8(loft_cx *cx, uint32_t c, char *out);

/* OpGetTextSub (State::get_text_sub): the bytes [from, till) of `v`, a negative bound
 * counting from the end, `from` snapped back and `till` forward to a character boundary;
 * `till` INT32_MIN is the one character at `from`.  The bounds arrive as i64 and are taken
 * as i32, as the interpreter takes them. */
/* Inlined with the text writes (`LOFT_TEXT_INLINE`, loft_text.h).  A one-byte slice
 * `s[k..k + 1]` of an ASCII character answers that byte before any snapping: the general path
 * gives the same byte, since an ASCII byte starts a character and the next byte, tested here,
 * does not continue it.  Its length is then the constant 1, which the append after it folds
 * on (`loft_text_concat`).  `-DLOFT_NO_SHORT_APPEND` takes the general path. */
/* MSVC (cl 19.51, /O1 and /O2) answers a wrong slice once this function is inlined into a
 * program of any size — `"abcé"[3..4]` came back one byte, while a unit holding only the function
 * is right and gcc and clang are right everywhere — so on `cl` it stays out of line.  Measured
 * on the Windows runner, the 749 slice cell: inlined wrong, out of line right, reading the bytes
 * as `unsigned char` changes nothing. */
#if defined(_MSC_VER) && !defined(__clang__)
#define LOFT_TEXT_SUB_INLINE static __declspec(noinline)
#else
#define LOFT_TEXT_SUB_INLINE LOFT_TEXT_INLINE
#endif
LOFT_TEXT_SUB_INLINE loft_str loft_text_sub(loft_cx *cx, loft_str v, int64_t from64, int64_t till64) {
#ifndef LOFT_NO_SHORT_APPEND
    if ((uint64_t) till64 == (uint64_t) from64 + 1 && (uint64_t) from64 < v.len && from64 < INT32_MAX
        && (unsigned char) v.p[from64] < 0x80
        && ((uint64_t) till64 == v.len || ((unsigned char) v.p[till64] & 0xC0) != 0x80))
        return loft_lit(cx, v.p + from64, 1);
#endif
    int32_t from = (int32_t) (uint32_t) (uint64_t) from64;
    int32_t till = (int32_t) (uint32_t) (uint64_t) till64;
    int32_t n = (int32_t) v.len;
    const unsigned char *b = (const unsigned char *) v.p;
    if (from < 0) { from += n; if (from < 0) from = 0; }
    if (from >= n) return loft_lit(cx, v.p, 0);
    while ((b[from] & 0xC0) == 0x80 && from > 0) from--;
    if (till == INT32_MIN) {
        unsigned char c = b[from];
        int32_t w = (c & 0xE0) == 0xC0 ? 2 : (c & 0xF0) == 0xE0 ? 3 : (c & 0xF0) == 0xF0 ? 4 : 1;
        if (w > n - from) w = n - from;
        return loft_lit(cx, v.p + from, (size_t) w);
    }
    if (till < 0) till += n;
    int32_t len = till - from;
    if (len <= 0) return loft_lit(cx, v.p, 0);
    if (len + from > n) {
        len = n - from;
    } else if (till < n) {
        for (int32_t t = till; t < n && (b[t] & 0xC0) == 0x80; t++) len++;
    }
    return loft_lit(cx, v.p + from, (size_t) len);
}

/* The text natives (src/native.rs t_4text_*): Rust's `str` methods on the raw bytes, the
 * null text included as the one byte it is. */
static int loft_mem_at(loft_cx *cx, loft_str s, size_t at, loft_str v) {
    return at + v.len <= s.len && (v.len == 0 || memcmp(s.p + at, v.p, v.len) == 0);
}

static uint8_t loft_starts_with(loft_cx *cx, loft_str s, loft_str v) { return (uint8_t) loft_mem_at(cx, s, 0, v); }

static uint8_t loft_ends_with(loft_cx *cx, loft_str s, loft_str v) {
    return (uint8_t) (v.len <= s.len && loft_mem_at(cx, s, s.len - v.len, v));
}

/* How common byte `c` is in text, highest first: the order `memchr`'s searchers rank a needle's
 * bytes by, so a search stops on the needle's RAREST byte and not on its first — the first of
 * "alpha" is the commonest letter there is.  Bytes not listed rank rare. */
static int loft_byte_commonness(loft_cx *cx, unsigned char c) {
    static const char order[] = " etaoinsrhldcumfpgwybvkxjqzETAOINSRHLDCUMFPGWYBVKXJQZ,.\n0123456789";
    const char *at = memchr(order, c, sizeof order - 1);
    return at ? (int) (sizeof order - 1 - (size_t) (at - order)) : 0;
}

/* The offset of needle `v`'s rarest byte (`loft_byte_commonness`): the byte a search scans
 * for.  Asked once per search, not once per match. */
static size_t loft_rare_index(loft_cx *cx, loft_str v) {
    size_t k = 0;
    int best = v.len ? loft_byte_commonness(cx, (unsigned char) v.p[0]) : 0;
    for (size_t i = 1; i < v.len; i++) {
        int c = loft_byte_commonness(cx, (unsigned char) v.p[i]);
        if (c < best) best = c, k = i;
    }
    return k;
}

/* The first byte offset at or after `from` where `v` occurs in `s`, or -1: `memchr` for the
 * needle's byte at `k` (`loft_rare_index`), then the whole needle compared around each hit
 * (`str::find`'s answer; an empty needle is found at `from`). */
static int64_t loft_mem_find_at(loft_cx *cx, loft_str s, size_t from, loft_str v, size_t k) {
    if (v.len == 0) return from <= s.len ? (int64_t) from : -1;
    if (v.len > s.len || from > s.len - v.len) return -1;
    const char *end = s.p + (s.len - v.len) + k + 1;
    const char *at = s.p + from + k;
    while (at < end) {
        const char *hit = (const char *) memchr(at, (unsigned char) v.p[k], (size_t) (end - at));
        if (!hit) return -1;
        const char *start = hit - k;
        if (memcmp(start, v.p, v.len) == 0) return (int64_t) (start - s.p);
        at = hit + 1;
    }
    return -1;
}

static int64_t loft_find(loft_cx *cx, loft_str s, loft_str v) {
    int64_t at = loft_mem_find_at(cx, s, 0, v, loft_rare_index(cx, v));
    return at < 0 ? LOFT_NULL : at;
}

static int64_t loft_rfind(loft_cx *cx, loft_str s, loft_str v) {
    if (v.len > s.len) return LOFT_NULL;
    for (size_t at = s.len - v.len + 1; at-- > 0;) if (loft_mem_at(cx, s, at, v)) return (int64_t) at;
    return LOFT_NULL;
}

static uint8_t loft_contains(loft_cx *cx, loft_str s, loft_str v) { return (uint8_t) (loft_find(cx, s, v) != LOFT_NULL); }

/* char::is_whitespace, for the code point at the UTF-8 bytes `p` of `n` bytes. */
static int loft_is_space(loft_cx *cx, uint32_t c) {
    return (c >= 9 && c <= 13) || c == 32 || c == 0x85 || c == 0xA0 || c == 0x1680
        || (c >= 0x2000 && c <= 0x200A) || c == 0x2028 || c == 0x2029 || c == 0x202F
        || c == 0x205F || c == 0x3000;
}

/* The code point starting at byte `at` and its length in bytes. */
static uint32_t loft_decode(loft_cx *cx, loft_str s, size_t at, size_t *n) {
    const unsigned char *b = (const unsigned char *) s.p + at;
    size_t left = s.len - at;
    if (b[0] < 0x80 || left < 2) { *n = 1; return b[0]; }
    if (b[0] < 0xE0) { *n = 2; return ((uint32_t) (b[0] & 0x1F) << 6) | (b[1] & 0x3F); }
    if (b[0] < 0xF0 || left < 4) {
        *n = left < 3 ? left : 3;
        return ((uint32_t) (b[0] & 0x0F) << 12) | ((uint32_t) (b[1] & 0x3F) << 6) | (left > 2 ? (b[2] & 0x3F) : 0);
    }
    *n = 4;
    return ((uint32_t) (b[0] & 0x07) << 18) | ((uint32_t) (b[1] & 0x3F) << 12) | ((uint32_t) (b[2] & 0x3F) << 6) | (b[3] & 0x3F);
}

static loft_str loft_trim_start_view(loft_cx *cx, loft_str s) {
    size_t at = 0, n;
    while (at < s.len && loft_is_space(cx, loft_decode(cx, s, at, &n))) at += n;
    return loft_lit(cx, s.p + at, s.len - at);
}

static loft_str loft_trim_end_view(loft_cx *cx, loft_str s) {
    size_t end = s.len;
    while (end > 0) {
        size_t st = end - 1, n;
        while (st > 0 && (((const unsigned char *) s.p)[st] & 0xC0) == 0x80) st--;
        if (!loft_is_space(cx, loft_decode(cx, s, st, &n))) break;
        end = st;
    }
    return loft_lit(cx, s.p, end);
}

static loft_text loft_trim(loft_cx *cx, loft_str s) { return loft_text_own(cx, loft_trim_end_view(cx, loft_trim_start_view(cx, s))); }
static loft_text loft_trim_start(loft_cx *cx, loft_str s) { return loft_text_own(cx, loft_trim_start_view(cx, s)); }
static loft_text loft_trim_end(loft_cx *cx, loft_str s) { return loft_text_own(cx, loft_trim_end_view(cx, s)); }

/* str::replace: every non-overlapping `v` left to right; an empty `v` matches before every
 * character and at the end. */
static loft_text loft_replace(loft_cx *cx, loft_str s, loft_str v, loft_str w) {
    loft_text out = { 0, 0, 0 };
    size_t at = 0;
    if (v.len == 0) {
        while (at < s.len) {
            size_t n;
            loft_decode(cx, s, at, &n);
            loft_push(cx, &out, w.p, w.len);
            loft_push(cx, &out, s.p + at, n);
            at += n;
        }
        loft_push(cx, &out, w.p, w.len);
        return out;
    }
    /* As str::replace: find the next match (`loft_mem_find_at`), push the run before it whole. */
    size_t k = loft_rare_index(cx, v);
    while (at < s.len) {
        int64_t hit = loft_mem_find_at(cx, s, at, v, k);
        size_t to = hit < 0 ? s.len : (size_t) hit;
        loft_push(cx, &out, s.p + at, to - at);
        if (hit < 0) break;
        loft_push(cx, &out, w.p, w.len);
        at = to + v.len;
    }
    return out;
}

/* Stores::text_byte_at_native: the byte at `i` (negative from the end), 0 outside. */
static int64_t loft_byte_at(loft_cx *cx, loft_str s, int64_t i) {
    int64_t len = (int64_t) s.len;
    int64_t at = i < 0 ? i + len : i;
    if (i == LOFT_NULL || at < 0 || at >= len) return 0;
    return ((const unsigned char *) s.p)[at];
}

/* Characters: a code point, 0 the null character. */
static uint32_t loft_char_of(loft_cx *cx, int64_t v) {
    if (v == LOFT_NULL || v < 0 || v > 0x10FFFF || (v >= 0xD800 && v <= 0xDFFF)) return 0;
    return (uint32_t) v;
}

static int64_t loft_char_int(loft_cx *cx, uint32_t c) {
    return c == 0 ? LOFT_NULL : (int64_t) c;
}

static int64_t loft_char_len(loft_cx *cx, uint32_t c) {
    return c == 0 ? 0 : c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
}

/* The decode of a character that is not ASCII, the byte at `i` inside the text: snapped back to
 * its first byte.  Out of line: `loft_text_char` answers an ASCII byte itself. */
static uint32_t loft_text_char_wide(loft_cx *cx, loft_str t, int64_t i) {
    int64_t len = (int64_t) t.len;
    const unsigned char *b = (const unsigned char *) t.p;
    while (i > 0 && (b[i] & 0xC0) == 0x80) i--;
    uint32_t c = b[i];
    int n = c >= 0xF0 ? 3 : c >= 0xE0 ? 2 : 1;
    c &= n == 3 ? 0x07 : n == 2 ? 0x0F : 0x1F;
    for (int k = 1; k <= n && i + k < len; k++) c = (c << 6) | (b[i + k] & 0x3F);
    return c;
}

/* ops::text_character: the character holding byte `from` (negative counts from the end); an
 * index inside a character snaps back to its first byte; out of range is the null character.
 * Inline, so an ASCII byte — every character of an ASCII alphabet a loop indexes — is one load
 * and one test; gcc left the whole decode out of line and called it per character. */
LOFT_HOT_INLINE uint32_t loft_text_char(loft_cx *cx, loft_str t, int64_t from) {
    int64_t len = (int64_t) t.len;
    int64_t i = from < 0 ? from + len : from;
    if (from == LOFT_NULL || i < 0 || i >= len) return 0;
    const unsigned char *b = (const unsigned char *) t.p;
    if (LOFT_LIKELY(b[i] < 0x80)) return b[i];
    return loft_text_char_wide(cx, t, i);
}

/* OpAppendCharacter: the null character appends nothing.  An ASCII character into a buffer with
 * room is one store and a length bump (`-DLOFT_NO_SHORT_APPEND` encodes and pushes it), always
 * inlined: in a large unit clang left the whole function a call per character (zttext's
 * `materialise` 3.7x its twin, 2.3x inlined); the encoding and the growth stay out of line. */
LOFT_COLD static void loft_push_code_slow(loft_cx *cx, loft_text *t, uint32_t c) {
    char buf[8]; /* loft_utf8 writes at most 4; the room keeps gcc's bounds check quiet */
    size_t n = c == 0 ? 0 : loft_utf8(cx, c, buf);
    loft_push(cx, t, buf, n);
}
LOFT_HOT_INLINE void loft_push_code(loft_cx *cx, loft_text *t, uint32_t c) {
#ifndef LOFT_NO_SHORT_APPEND
    if (LOFT_LIKELY(c - 1u < 0x7Fu && t->len < t->cap)) {
        t->p[t->len++] = (char) c;
        return;
    }
#endif
    loft_push_code_slow(cx, t, c);
}

/* `@FR-R-PushFill`'s text clause — a text a loop pushes characters to and names nowhere else
 * holds its bytes, length and room in locals for the loop: a `char` store may alias the text's
 * header, so pushing through it reloaded the length and the bytes at every character.  The
 * length goes back around each growth and at the loop's end (`loft_tw_sync`).  The growth goes
 * through a COPY, as `loft_win`'s does, so the window's address never reaches a call and the
 * compiler keeps its fields in registers. */
typedef struct { char *p; size_t len, cap; } loft_tw;
LOFT_HOT_INLINE loft_tw loft_tw_of(loft_cx *cx, const loft_text *t) {
    loft_tw w = { t->p, t->len, t->cap };
    return w;
}
LOFT_COLD static loft_tw loft_tw_grow(loft_cx *cx, loft_tw w, loft_text *t, uint32_t c) {
    t->len = w.len;
    loft_push_code_slow(cx, t, c);
    w.p = t->p;
    w.len = t->len;
    w.cap = t->cap;
    return w;
}
LOFT_HOT_INLINE void loft_tw_push(loft_cx *cx, loft_tw *w, loft_text *t, uint32_t c) {
    if (LOFT_LIKELY(c - 1u < 0x7Fu && w->len < w->cap)) {
        w->p[w->len++] = (char) c;
        return;
    }
    loft_tw g = *w;
    g = loft_tw_grow(cx, g, t, c);
    *w = g;
}
LOFT_HOT_INLINE void loft_tw_sync(loft_cx *cx, const loft_tw *w, loft_text *t) { t->len = w->len; }

/* A function value: the definition it names (LOFT_NULL for the absent one) and the closure
 * record a capturing lambda reads.  A call goes through the emitted dispatcher of its
 * signature, `loft_call{n}`, which switches on `d` (State::fn_call_ref). */
/* `@FR-R-RefParam`: a reference parameter arrives as `<name>__in` and the body's first statement
 * makes the local `<name>` of it (`tidy_unit`).  The pad above keeps `rec|pos` out of one
 * register, but their bytes still lie side by side once gcc spills the two eightbytes, and a body
 * that reads both reloads them with one 8-byte load spanning the two stores — the stall again,
 * on every call.  Written field by field, the local's `rec` and `pos` are separate values gcc
 * never reloads as one.  clang does not stall and pays for the copy (cbor `encode_bytes` +31 %),
 * so everywhere but gcc it is the plain copy, which every compiler folds away. */
#if defined(__GNUC__) && !defined(__clang__)
#define LOFT_REF_IN(v) \
    loft_ref v; \
    v.store_nr = v##__in.store_nr; \
    v.rec = v##__in.rec; \
    v.pos = v##__in.pos; \
    v.pad_ = 0
#else
#define LOFT_REF_IN(v) loft_ref v = v##__in
#endif
typedef struct { int64_t d; loft_ref clos; } loft_fn;

/* ── Stores ────────────────────────────────────────────────────────────────────────────────
 * The store model the IR addresses (src/store.rs, src/vector.rs): a store is an array of 8-byte
 * words; a record is a run of words whose first word holds its claimed size (and, at byte 4 of a
 * vector record, the vector's length); a reference (`DbRef`) names a store, a record and a byte
 * position, and a field is `rec * 8 + pos + offset`.  `rec == 0` is the null reference.  The
 * allocator is loft's own (`loft_store.h`, a port of `Store`); the table of stores below is
 * `Stores`: the lowest free slot is made again, a new slot starts at 100 words and a reused one
 * keeps its capacity, and a store's serial is the order it was made. */
LOFT_HOT_INLINE loft_ref loft_ref_null(loft_cx *cx) {
    loft_ref r = {0};
    r.store_nr = 0xFFFF;
    r.rec = 0;
    r.pos = 0;
    return r;
}

/* `DbRef::NULL_REPORTED`: the null an `else` arm or a reported access owns (`@FR-E-Report`). */
static loft_ref loft_ref_null_reported(loft_cx *cx) {
    loft_ref r = loft_ref_null(cx);
    r.pos = 0x80000000u;
    return r;
}

/* `@FR-E-Report` (`Stores::raise_recoverable_runtime`): a recoverable fault at an unguarded
 * site writes one Warn record `[<label>] <message>` at the site's file and line.  The logger is
 * the unit's (`loft_log_warn`), bound by its `main`; a runtime built without one says nothing,
 * and so does a test, which runs with no logger on the interpreter. */
static void (*loft_warn_logger)(loft_cx *cx, loft_str, loft_str, int64_t) = 0;
static LOFT_COLD void loft_fault_report(loft_cx *cx, const char *message, const char *file, uint32_t line) {
    if (loft_warn_logger && !cx->in_test)
        loft_warn_logger(cx, loft_lit(cx, message, strlen(message)), loft_lit(cx, file, strlen(file)), line);
    /* `--dev-soft-halt`: the fault on stderr as well, in the interpreter's words — the message
     * without its `[kind]` label — and the run ends non-zero. */
    if (!cx->in_test && loft_soft_halt(cx)) {
        const char *m = message;
        if (*m == '[') {
            const char *close = strchr(m, ']');
            if (close) m = close[1] == ' ' ? close + 2 : close + 1;
        }
        loft_had_fatal = 1;
        fprintf(stderr, "soft-halt: %s at %s:%u\n", m, file, (unsigned) line);
    }
}

/* `loft_fault_report` with a message built from `fmt`, for the kinds that name their values. */
static LOFT_COLD void loft_fault_reportf(loft_cx *cx, const char *file, uint32_t line, const char *fmt, ...) {
    char m[256];
    va_list ap;
    if (!loft_warn_logger || cx->in_test) return;
    va_start(ap, fmt);
    vsnprintf(m, sizeof m, fmt, ap);
    va_end(ap);
    loft_fault_report(cx, m, file, line);
}

/* An index that names no element (`State::vec_get_or_raise_slow`): `negative_index` when it is
 * still negative counted from the end, `index_out_of_bounds` otherwise; the null index is the
 * most negative one. */
static LOFT_COLD void loft_index_miss(loft_cx *cx, int64_t from, int64_t len, const char *file, uint32_t line) {
    if ((from < 0 ? from + len : from) < 0)
        loft_fault_reportf(cx, file, line, "[negative_index] negative index %lld", (long long) from);
    else
        loft_fault_reportf(cx, file, line, "[index_out_of_bounds] index %lld out of bounds for length %lld",
                           (long long) from, (long long) len);
}

/* A write through `r` that names no record was dropped: reported unless its absence already
 * was (`DbRef::absence_unreported` — an `else` arm or the access that found it owns it). */
static LOFT_COLD void loft_write_dropped(loft_cx *cx, loft_ref r, const char *file, uint32_t line) {
    if (r.rec == 0 && !(r.pos & 0x80000000u))
        loft_fault_report(cx, "[write_dropped] a write to a place that names no record was dropped "
                          "\xe2\x80\x94 add `else { \xe2\x80\xa6 }` to the assignment to handle it",
                          file, line);
}

/* `LOFT_STRICT_STORES` (`keys::strict_stores`, @PLN206 P0b): a freed store stays dead — its
 * slot is not made again (`loft_store_make`), except the top one, whose free lowers the floor as
 * the interpreter's `trim_free_top` lowers `max` — so an access through a reference naming it is
 * a use after free, reported at the access in the
 * interpreter's words (`strict_store_violation`, a native run's: no bytecode positions); and a
 * store still live at exit is reported too (`loft_leak_report`).  Either makes the run exit 1.
 * The driver builds with it when `LOFT_STRICT_STORES` is set.  For probes: never reusing a slot
 * walks a long run off the end of the store table. */
/* `strict_note_free` (keys.rs): where each store slot was last freed — the function running and
 * the line it was at (`loft_frame_site`; a C unit has no bytecode position) — so a use after free
 * names the free that killed it, and the re-free census (`LOFT_DOUBLE_FREE`) tells a re-free from
 * the same site from one from another.  Kept by either instrument, as the interpreter keeps it. */
#if defined(LOFT_STRICT_STORES) || defined(LOFT_DOUBLE_FREE)
typedef struct loft_free_site { const char *fn, *file; uint32_t line; } loft_free_site;

/* Where the program is now: the innermost frame and the line it is at. */
static loft_free_site loft_site_here(loft_cx *cx) {
    loft_free_site w = { 0, 0, 0 };
    if (cx->nframes) {
        const loft_frame *f = &cx->frames[cx->nframes - 1];
        w.fn = f->fn;
        w.file = f->file;
        w.line = f->site ? f->site : f->decl;
    }
    return w;
}

static void loft_note_free(loft_cx *cx, uint32_t nr) {
    if (nr >= cx->nfreed_at) {
        uint32_t n = cx->nfreed_at ? cx->nfreed_at * 2 : 64;
        if (n <= nr) n = nr + 1;
        cx->freed_at = (loft_free_site *) realloc(cx->freed_at, (size_t) n * sizeof *cx->freed_at);
        if (!cx->freed_at) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(cx->freed_at + cx->nfreed_at, 0, (size_t) (n - cx->nfreed_at) * sizeof *cx->freed_at);
        cx->nfreed_at = n;
    }
    cx->freed_at[nr] = loft_site_here(cx);
}

/* The free that last released slot `nr`, or 0 when none was recorded. */
static const loft_free_site *loft_freed_site(loft_cx *cx, uint32_t nr) {
    return nr < cx->nfreed_at && cx->freed_at[nr].fn ? &cx->freed_at[nr] : 0;
}

static int loft_same_site(loft_cx *cx, const loft_free_site *a, const loft_free_site *b) {
    return a && b && a->fn == b->fn && a->line == b->line;
}
#endif

#ifdef LOFT_STRICT_STORES
static uint64_t loft_strict_violations = 0;

static LOFT_COLD void loft_strict_use_after_free(loft_cx *cx, loft_ref r) {
    uint64_t n = loft_strict_violations++;
    if (n < 20) {
        uint16_t kt = LOFT_KNOWN(cx->stores[r.store_nr]);
        const loft_free_site *w = loft_freed_site(cx, r.store_nr);
        loft_free_site here = loft_site_here(cx);
        fflush(stdout);
        fprintf(stderr, "[strict-store] USE AFTER FREE (access) store #%u type=%s rec=%u pos=%u\n",
                (unsigned) r.store_nr, kt < loft_ntn ? loft_tn[kt] : "?", (unsigned) r.rec, (unsigned) r.pos);
        {
            FILE *lf = loft_fault_file(cx);
            if (lf) {
                char buf[512];
                loft_fault_begin(cx, lf, "use-after-free");
                snprintf(buf, sizeof buf, "%u", (unsigned) r.store_nr);
                loft_fault_field(cx, lf, "store", buf);
                loft_fault_field(cx, lf, "access", "access");
                loft_fault_field(cx, lf, "type", kt < loft_ntn ? loft_tn[kt] : "?");
                if (w) {
                    snprintf(buf, sizeof buf, "`%s` %s:%u", w->fn, w->file, (unsigned) w->line);
                    loft_fault_field(cx, lf, "freed", buf);
                }
                snprintf(buf, sizeof buf, "`%s` %s:%u", here.fn ? here.fn : "?",
                         here.file ? here.file : "?", (unsigned) here.line);
                loft_fault_field(cx, lf, "at", buf);
                loft_fault_end(cx, lf);
            }
        }
        if (w)
            fprintf(stderr, "  killed by the free in fn %s() at %s:%u\n", w->fn, w->file, (unsigned) w->line);
        else
            fputs("  killed by the free of `<not recorded>`\n", stderr);
        if (here.fn)
            fprintf(stderr, "  accessed now in fn %s() at %s:%u\n", here.fn, here.file, (unsigned) here.line);
        fputs("  (no bytecode positions \xe2\x80\x94 native run; re-run with --interpret for pcs)\n", stderr);
    } else if (n == 20) {
        fputs("[strict-store] ... further use-after-free reports suppressed\n", stderr);
    }
}
#endif

LOFT_HOT_INLINE loft_store_t *loft_store_of(loft_cx *cx, loft_ref r) {
#ifdef LOFT_STRICT_STORES
    if (LOFT_UNLIKELY(r.store_nr < cx->nstores && !cx->stores[r.store_nr].live)) loft_strict_use_after_free(cx, r);
#endif
    return &cx->stores[r.store_nr];
}

/* `#lock = …` (`n_set_store_lock`) and `#lock` (`n_get_store_lock`): the store holding `r`. */
static void loft_set_store_lock(loft_cx *cx, loft_ref r, uint8_t locked) {
    if (r.rec == 0 || r.store_nr >= cx->nstores) return;
    /* A constant and foreign bytes stay read-only: `#lock = false` is the author's own lock's
     * cure (`Store::unlock` is not reachable from loft for them). */
    if (cx->stores[r.store_nr].read_only > LOFT_RO_LOCK) return;
    cx->stores[r.store_nr].read_only = locked == 1;
}

/* A text lent as a `loft_text` that owns nothing (capacity 0): a borrowed member of a by-value
 * tuple, which no one frees or writes. */
static loft_text loft_text_lend(loft_cx *cx, loft_str s) {
    loft_text t;
    t.p = (char *) s.p;
    t.len = s.len;
    t.cap = 0;
    return t;
}

/* `yield_frame()`: a compiled program has no host loop to return to, so a frame-driven program
 * runs straight through, as `--native` does. */
static void loft_yield_frame(loft_cx *cx) {}

/* `store_reclaim(r)` (`Stores::reclaim_store`): the bytes the store holding `r` gave back. */
static int64_t loft_store_reclaim(loft_cx *cx, loft_ref r) {
    if (r.store_nr >= cx->nstores) return 0;
    return (int64_t) loft_st_reclaim_tail(cx, &cx->stores[r.store_nr]) * 8;
}

static uint8_t loft_get_store_lock(loft_cx *cx, loft_ref r) {
    return (uint8_t) (r.rec != 0 && r.store_nr < cx->nstores && cx->stores[r.store_nr].read_only);
}


/* `Stores::database_named`: the lowest free slot, a fresh store in it; `counted` gives it the
 * next serial, as every store the program makes. */
/* `LOFT_LEAK_SITES` (the interpreter's leak-site report): each store remembers where it was made
 * — the function running, its file and the line it was at (`loft_frame_site`) — so a store left
 * at exit is named by its site, not only its type (`loft_leak_report`).  It keeps the frames
 * (`LOFT_CALL_CHAIN`); the driver builds with it when `LOFT_LEAK_SITES` is set. */
#ifdef LOFT_LEAK_SITES
/* The caller's function and line beside the maker's: a store a callee makes into a buffer its
 * caller handed it (a hidden return buffer) is the caller's to release, so the caller is where
 * a leak of it is decided. */
typedef struct loft_leak_site { const char *fn, *file; uint32_t line; const char *cfn; uint32_t cline; } loft_leak_site;
static void loft_note_site(loft_cx *cx, uint32_t nr) {
    if (nr >= cx->nsites) {
        uint32_t n = cx->nsites ? cx->nsites * 2 : 64;
        if (n <= nr) n = nr + 1;
        cx->sites = (loft_leak_site *) realloc(cx->sites, (size_t) n * sizeof *cx->sites);
        if (!cx->sites) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(cx->sites + cx->nsites, 0, (size_t) (n - cx->nsites) * sizeof *cx->sites);
        cx->nsites = n;
    }
    loft_leak_site *w = &cx->sites[nr];
    if (cx->nframes) {
        const loft_frame *f = &cx->frames[cx->nframes - 1];
        w->fn = f->fn;
        w->file = f->file;
        w->line = f->site ? f->site : f->decl;
        if (cx->nframes > 1) {
            const loft_frame *c = &cx->frames[cx->nframes - 2];
            w->cfn = c->fn;
            w->cline = c->site ? c->site : c->decl;
        } else {
            w->cfn = 0;
            w->cline = 0;
        }
    } else {
        w->fn = 0;
        w->file = 0;
        w->line = 0;
        w->cfn = 0;
        w->cline = 0;
    }
}
#endif

/* ── A hosted unit (`@PLN206 P4`, `-DLOFT_HOSTED`) ──────────────────────────────────────────
 * A unit the interpreter adopts works on the interpreter's stores: `cx->stores` is the
 * interpreter's table, whose entries are its `Store`s (`LOFT_HOST_STORE_TAIL`), and a store's
 * birth and death are the interpreter's acts (`host_abi::HostedTable`).  Everything inside a
 * store the unit does itself on the shared prefix.  `cx->host_stores` is the interpreter's
 * `Stores` of the bridge call running on this thread. */
#ifdef LOFT_HOSTED
typedef struct {
    uint8_t *(*table)(void *stores, uint32_t *n, uint64_t *serial);
    uint32_t (*store_new)(void *stores);
    void (*store_free)(void *stores, uint32_t nr);
    uint32_t store_tail;
    const unsigned char *(*source_dir)(void *stores, size_t *len, uint32_t *relative);
    void (*gen_adopt)(void *stores, uint32_t slot, uint32_t kind,
                      uint32_t (*next)(void *, uint32_t, uint32_t, unsigned char *),
                      void (*release)(void *, uint32_t), unsigned char *ret);
    /* `@PLN206 P7` — the debugger's pause request, and the hand-over of a routine to it. */
    const volatile uint8_t *pause;
    uint32_t (*handover)(void *stores, const char *key, size_t key_len, uint32_t scope,
                         const unsigned char *entries, uint32_t n);
    /* `@PLN226` — this unit's store context for `stores` (`loft_host_cx`). */
    void *(*cx_of)(void *stores, const void *unit, void *(*make)(void), void (*end)(void *));
} loft_hosted_table;
static const loft_hosted_table *loft_hosted = 0;
/* The depth of the routine a bridge call called, less one: set by the bridge, restored after. */
/* A hand-over the interpreter refused holds for the rest of the bridge call: the routine runs on
 * compiled, and the interpreter takes the pause once it returns.  Asking again each round would
 * cost the round a call into the interpreter. */

/* `@PLN206 P7` — a pause the debugger asked for, seen by the routine the interpreter called
 * (no deeper one: that routine's frame is live under the call it is in), at a loop's head. */
static int loft_pause_asked(loft_cx *cx) {
    return loft_hosted && loft_hosted->pause && *loft_hosted->pause && !cx->ho_off
        && LOFT_HDEPTH == cx->br_hdepth + 1u;
}
static void loft_ho_put(loft_cx *cx, const char *name, uint32_t kind, loft_word bits, loft_str t, loft_ref r) {
    if (cx->nhov >= 256) return;
    loft_ho_entry *e = &cx->hov[cx->nhov++];
    e->name = name;
    e->kind = kind;
    e->bits = bits;
    e->p = t.p;
    e->len = t.len;
    e->rs = r.store_nr;
    e->rr = r.rec;
    e->rp = r.pos;
}
static loft_str loft_ho_nostr(loft_cx *cx) { loft_str s = { 0, 0 }; return s; }
static int loft_ho_commit(loft_cx *cx, const char *key, uint32_t scope) {
    uint32_t n = cx->nhov;
    cx->nhov = 0;
    int taken = (int) loft_hosted->handover(cx->host_stores, key, strlen(key), scope,
                                            (const unsigned char *) cx->hov, n);
    if (!taken) cx->ho_off = 1;
    return taken;
}

/* The table as the interpreter holds it now: after a bridge call starts and after every act
 * the interpreter does for the unit, either of which may move it. */
static void loft_host_sync(loft_cx *cx) {
#ifdef LOFT_STORES_VIEW_AT
    /* `host_abi::TableView`, kept current inside the interpreter's `Stores` before each bridge
     * call and after each act the unit leaves to it: read in place, no call back. */
    typedef struct { uint8_t *table; uint32_t n; uint64_t serial; } loft_table_view;
    loft_table_view v;
    memcpy(&v, (const char *) cx->host_stores + LOFT_STORES_VIEW_AT, sizeof v);
    cx->stores = (loft_store_t *) v.table;
    cx->nstores = v.n;
    cx->stores_allocated = v.serial;
#else
    uint32_t n = 0;
    uint64_t serial = 0;
    cx->stores = (loft_store_t *) loft_hosted->table(cx->host_stores, &n, &serial);
    cx->nstores = n;
    cx->stores_allocated = serial;
#endif
    cx->hint_table = 0;
}
#endif

static uint32_t loft_store_make(loft_cx *cx, int counted) {
    uint32_t nr = 0;
#ifdef LOFT_HOSTED
    /* The interpreter makes the store: a live, counted one with no record. */
    if (cx->host_stores) {
        (void) counted;
        nr = loft_hosted->store_new(cx->host_stores);
        loft_host_sync(cx);
        return nr;
    }
#endif
#ifdef LOFT_STRICT_STORES
    /* `no_slot_reuse`: every slot below the floor was made once, and stays dead once freed. */
    nr = cx->strict_floor;
#endif
#ifndef LOFT_STRICT_STORES
    if (cx->hint_table == cx->stores && cx->hint <= cx->nstores) nr = cx->hint;
#endif
    while (nr < cx->nstores && cx->stores[nr].live) nr++;
#ifdef LOFT_STRICT_STORES
    cx->strict_floor = nr + 1;
#endif
#ifdef LOFT_STRICT_STORES
    if (nr >= 0xFFFF) {
        uint32_t live = 0, i;
        for (i = 0; i < cx->nstores; i++) live += cx->stores[i].live != 0;
        fprintf(stderr, "loft: store table exhausted by the checking mode, not by a leak: this run made 65535 "
                        "stores and %u are live.  LOFT_STRICT_STORES never reuses a freed store number, so that "
                        "a stale reference cannot reach a newer store, and store_nr is u16.  This program frees "
                        "and re-mints more stores than the mode can track: check it with a smaller loop count, "
                        "or run it without the switch.\n", (unsigned) live);
        abort();
    }
#endif
    if (nr >= 0xFFFF) { fputs("loft: store table exhausted: 65535 stores live at once\n", stderr); abort(); }
    if (nr == cx->nstores) {
        uint32_t n = cx->nstores ? cx->nstores * 2 : 16;
        if (n > 0xFFFF) n = 0xFFFF;
        loft_store_t *s = (loft_store_t *) realloc(cx->stores, n * sizeof *s);
        if (!s) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(s + cx->nstores, 0, (n - cx->nstores) * sizeof *s);
        cx->stores = s;
        cx->nstores = n;
    }
    cx->hint_table = cx->stores;
    cx->hint = nr + 1;
    loft_store_t *s = &cx->stores[nr];
#ifdef LOFT_LEAK_SITES
    loft_note_site(cx, nr);
#endif
    if (!s->buf) {
        loft_heap_moved(cx, s, 0, 100);
        s->size = 100;
        s->buf = (uint8_t *) malloc((size_t) s->size * 8);
        if (!s->buf) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_st_init(cx, s);
    s->live = 1;
    LOFT_SET_KNOWN(*s, 0xFFFF);
    s->pinned = 0;
    s->protect = 0;
    s->frame_holds = 0;
    s->orphaned = 0;
    s->closure_shares = 0;
    s->serial = 0;
#ifdef LOFT_ALLOC_TRACE
    s->trace_worker = cx->trace_worker;
#endif
    if (counted) {
        s->serial = ++cx->stores_allocated;
        LOFT_TRACE("N %s %u\n", loft_trace_name(cx, s), s->size);
    }
    return nr;
}

static uint32_t loft_store_new(loft_cx *cx) {
    return loft_store_make(cx, 1);
}

/* A record of `words` words.  Its payload holds no promised value (`@FR-H-Claim`, as
 * `Store::claim`): each caller writes the fields it reads.  `LOFT_POISON_CLAIM` fills it with
 * poison, and MemorySanitizer over the corpus flags a read of a byte nobody wrote. */
static uint32_t loft_claim(loft_cx *cx, loft_store_t *s, uint32_t words) {
    if (words < 1) words = 1;
    return loft_st_claim(cx, s, words);
}

LOFT_HOT_INLINE uint8_t *loft_at(loft_cx *cx, loft_store_t *s, uint32_t rec, uint32_t byte) {
#ifdef LOFT_FOREIGN
    /* `Store::read`'s intercept: the foreign record is served from the foreign bytes. */
    if (LOFT_UNLIKELY(rec == LOFT_FOREIGN_REC)) return loft_foreign_at(cx, s, byte);
#endif
    return s->buf + (size_t) rec * 8 + byte;
}

/* `Store::elem_base`: where vector record `rec`'s elements begin, past its header — the foreign
 * bytes themselves for the foreign record. */
LOFT_HOT_INLINE uint8_t *loft_elem_base(loft_cx *cx, loft_store_t *s, uint32_t rec) {
    return loft_at(cx, s, rec, 8);
}

LOFT_HOT_INLINE uint32_t loft_u32(loft_cx *cx, loft_store_t *s, uint32_t rec, uint32_t byte) {
    uint32_t v;
    memcpy(&v, loft_at(cx, s, rec, byte), 4);
    return v;
}

LOFT_HOT_INLINE void loft_set_u32(loft_cx *cx, loft_store_t *s, uint32_t rec, uint32_t byte, uint32_t v) {
    if (LOFT_WRITE_REFUSED(s)) return;
    memcpy(loft_at(cx, s, rec, byte), &v, 4);
}

#ifdef LOFT_PARSE
/* `Store::parse_errors` (loft_parse.h): a store made again answers none, an exchange moves
 * them with the content, a copy of a parsed root brings them. */
static void loft_perrs_reset(loft_cx *cx, uint16_t nr);
static void loft_perrs_swap(loft_cx *cx, uint16_t a, uint16_t b);
static void loft_perrs_deliver(loft_cx *cx, loft_ref data, loft_ref to, uint16_t tp, int fresh);
static void loft_perrs_deliver_vector(loft_cx *cx, loft_ref data, loft_ref to, int whole);
#endif

/* OpDatabase: a fresh record of `bytes` in the variable's store, cleared in place, or in a
 * new store when the variable holds none. */
/* `LOFT_DOUBLE_FREE` (`keys::double_free_audit`, `@FR-H-FreeTwice`): a CENSUS of tolerated
 * re-frees, split by whether the freeing site is the one that freed the slot first, and of FREED
 * slots made again through a reference that still names them (`loft_database`'s "a freed one
 * too").  A re-free is a free the compiler emitted twice on one path, which it removes where it
 * can (STORE_RECORDS.md § The checks): this branch only sees the ones whose slot was not reused.
 * A revival is the design (a loop's work-ref is freed at the end of a pass and re-made at the
 * next), but it is also the one place a
 * STALE reference re-initialises a slot another store may have taken since — which is why each
 * different-site line names both sites.  Printed at exit (`loft_double_free_report`). */
#ifdef LOFT_DOUBLE_FREE
static uint64_t loft_df_same = 0, loft_df_other = 0, loft_df_revived = 0;

static void loft_note_double_free(loft_cx *cx, uint32_t nr) {
    const loft_free_site *w = loft_freed_site(cx, nr);
    loft_free_site here = loft_site_here(cx);
    if (loft_same_site(cx, w, &here)) {
        /* Named too: a free run again on a slot it already released (a loop that frees a
         * reference it never reset) is as much a compiler defect as one from elsewhere, and
         * without the site it could only be counted. */
        if (loft_df_same++ < 20) {
            fflush(stdout);
            fprintf(stderr, "[double-free] store #%u freed again at the SAME site, in fn %s() at %s:%u\n",
                    (unsigned) nr, here.fn ? here.fn : "?", here.file ? here.file : "?", (unsigned) here.line);
        }
        return;
    }
    {
        FILE *lf = loft_fault_file(cx);
        if (lf && loft_df_other < 200) {
            char buf[512];
            loft_fault_begin(cx, lf, "re-free");
            snprintf(buf, sizeof buf, "%u", (unsigned) nr);
            loft_fault_field(cx, lf, "store", buf);
            snprintf(buf, sizeof buf, "`%s` %s:%u", w ? w->fn : "?", w ? w->file : "?",
                     w ? (unsigned) w->line : 0u);
            loft_fault_field(cx, lf, "freed", buf);
            snprintf(buf, sizeof buf, "`%s` %s:%u", here.fn ? here.fn : "?",
                     here.file ? here.file : "?", (unsigned) here.line);
            loft_fault_field(cx, lf, "again", buf);
            loft_fault_end(cx, lf);
        }
    }
    if (loft_df_other++ < 20) {
        fflush(stdout);
        fprintf(stderr, "[double-free] store #%u freed again in fn %s() at %s:%u, first freed in fn %s() at %s:%u\n",
                (unsigned) nr, here.fn ? here.fn : "?", here.file ? here.file : "?", (unsigned) here.line,
                w ? w->fn : "?", w ? w->file : "?", w ? (unsigned) w->line : 0u);
    }
}

static void loft_note_revival(loft_cx *cx, uint32_t nr) {
    const loft_free_site *w = loft_freed_site(cx, nr);
    loft_free_site here = loft_site_here(cx);
    {
        FILE *lf = loft_fault_file(cx);
        if (lf && loft_df_revived < 200) {
            char buf[512];
            loft_fault_begin(cx, lf, "use-after-free");
            snprintf(buf, sizeof buf, "%u", (unsigned) nr);
            loft_fault_field(cx, lf, "store", buf);
            snprintf(buf, sizeof buf, "`%s` %s:%u", w ? w->fn : "?", w ? w->file : "?",
                     w ? (unsigned) w->line : 0u);
            loft_fault_field(cx, lf, "freed", buf);
            snprintf(buf, sizeof buf, "`%s` %s:%u", here.fn ? here.fn : "?",
                     here.file ? here.file : "?", (unsigned) here.line);
            loft_fault_field(cx, lf, "at", buf);
            loft_fault_field(cx, lf, "detail", "a freed slot made again through a reference that named it");
            loft_fault_end(cx, lf);
        }
    }
    if (loft_df_revived++ < 20) {
        fflush(stdout);
        fprintf(stderr, "[double-free] store #%u made again in fn %s() at %s:%u, freed in fn %s() at %s:%u\n",
                (unsigned) nr, here.fn ? here.fn : "?", here.file ? here.file : "?", (unsigned) here.line,
                w ? w->fn : "?", w ? w->file : "?", w ? (unsigned) w->line : 0u);
    }
}

static void loft_double_free_report(loft_cx *cx) {
    fflush(stdout);
    fprintf(stderr,
            "[double-free] %llu re-free(s) of an already-free slot: %llu from a DIFFERENT site and %llu from the "
            "SAME site; %llu freed slot(s) made again through a reference that still named them.  Each re-free is a "
            "free the compiler emitted twice, harmless here only because nothing took the slot in between: run "
            "with LOFT_STRICT_STORES too, so a freed slot is never handed to another store and a stale reference "
            "meets the store it named.\n",
            (unsigned long long) (loft_df_same + loft_df_other), (unsigned long long) loft_df_other,
            (unsigned long long) loft_df_same, (unsigned long long) loft_df_revived);
}
#endif

static void loft_database(loft_cx *cx, loft_ref *var, uint32_t bytes) {
    uint32_t nr;
    uint32_t *kept = 0, nkept = 0;
    /* `@FR-G-Arg-Rebind`: a store a live generator frame names keeps the value the frame was
     * handed — the rebind orphans it and mints into a fresh store. */
    if (var->store_nr != 0xFFFF && var->store_nr < cx->nstores && cx->stores[var->store_nr].frame_holds) {
        cx->stores[var->store_nr].orphaned = 1;
        var->store_nr = 0xFFFF;
    }
    if (var->store_nr != 0xFFFF && var->store_nr < cx->nstores) {
        /* `Stores::clear`: the slot the variable names made again in place — a freed one too,
         * which keeps its serial — its tombstones kept. */
        nr = var->store_nr;
        loft_store_t *s = &cx->stores[nr];
#ifdef LOFT_DOUBLE_FREE
        if (!s->live && !s->pinned) loft_note_revival(cx, nr);
#endif
#ifdef LOFT_FOREIGN
        /* A store made again while it serves a view stops serving it. */
        if (!s->pinned) loft_st_release_foreign(cx, s);
#endif
        if (s->live) kept = loft_st_kept_positions(cx, s, &nkept);
        loft_st_init(cx, s);
        s->live = 1;
    } else {
        nr = loft_store_new(cx);
    }
    /* `alloc_record_into`: the root claimed, its payload zeroed — a fresh store's collection
     * handles must read "none" — and its type tag at byte 4 (none yet). */
    loft_store_t *s = &cx->stores[nr];
    uint32_t rec = loft_st_claim(cx, s, (bytes + 15) / 8);
    memset(s->buf + (size_t) rec * 8 + 4, 0, (size_t) loft_st_i32(cx, s, rec, 0) * 8 - 4);
    loft_set_u32(cx, s, rec, 4, 0xFFFFu);
    loft_st_entomb_kept(cx, s, kept, nkept);
    free(kept);
    LOFT_SET_KNOWN(*s, 0xFFFF);
#ifdef LOFT_PARSE
    loft_perrs_reset(cx, (uint16_t) nr);
#endif
    var->store_nr = (uint16_t) nr;
    var->rec = rec;
    var->pos = 8;
}

/* OpDatabase of a record type: the root carries the type it was minted as, which tells a move
 * (`loft_swap_in`) its own store from another. */
/* A fresh store whose root is an empty vector, minted as `Stores::text_vector` and
 * `Stores::fs_read_bytes` mint theirs — `Stores::database(4)`, a root of four words, which
 * `loft_database` counts in bytes past the header. */
static void loft_vector_store(loft_cx *cx, loft_ref *var) {
    loft_database(cx, var, 24);
}

#define LOFT_STORE_BUF_BIT 0x20000000u
static void loft_database_typed(loft_cx *cx, loft_ref *var, uint32_t bytes, uint16_t tp) {
    /* `@FR-R-HostStore`: a buffer naming its caller's store — the record is claimed there,
     * zeroed (the literal that follows writes every field). */
    if (var->store_nr != 0xFFFF && var->rec == 0 && (var->pos & LOFT_STORE_BUF_BIT)) {
        loft_store_t *s = &cx->stores[var->store_nr];
        uint32_t rec = loft_claim(cx, s, 1 + (bytes + 7) / 8);
        var->rec = rec;
        var->pos = 8;
        memset(loft_at(cx, s, rec, 8), 0, bytes);
        return;
    }
    loft_database(cx, var, bytes);
    LOFT_SET_KNOWN(cx->stores[var->store_nr], tp);
    loft_set_u32(cx, &cx->stores[var->store_nr], var->rec, 4, loft_tp_out(cx, tp));
}

/* OpDatabase of a vector wrapper (`main_vector<T>`, Stores::mint_at_place, @FR-R-Place): a
 * buffer that is a PLACE — a collection field of another record, not the wrapper's own store
 * — stays where it is with its collection released and emptied, so the literal builds where it lives;
 * the ordinary mint would clear the store the place lives in. */
/* The type descriptor is defined further down; a pointer to it is all this needs. */
struct loft_tdesc_s;
static void loft_remove_claims(loft_cx *cx, const struct loft_tdesc_s *T, uint16_t tp, loft_ref r);

static void loft_database_wrapper(loft_cx *cx, const struct loft_tdesc_s *T, loft_ref *var, uint32_t bytes, uint16_t tp) {
    if (var->store_nr != 0xFFFF && var->rec != 0 && var->store_nr < cx->nstores
        && !(var->rec == 1 && LOFT_KNOWN(cx->stores[var->store_nr]) == tp)) {
        /* The collection the place held is released, then its handle emptied. */
        loft_remove_claims(cx, T, tp, *var);
        loft_set_u32(cx, &cx->stores[var->store_nr], var->rec, var->pos, 0);
        return;
    }
    loft_database(cx, var, bytes);
    LOFT_SET_KNOWN(cx->stores[var->store_nr], tp);
    /* The root carries its type at byte 4, as `OpDatabase` writes it and the typed mint does
     * (`loft_database_typed`): `sizeof` of a reference reads it (`OpSizeofRef`), on the
     * interpreter's side of a hosted unit too (STORE_MODEL D47). */
    loft_set_u32(cx, &cx->stores[var->store_nr], var->rec, 4, loft_tp_out(cx, tp));
}

static loft_fn loft_fn_make(loft_cx *cx, int64_t d, loft_ref clos) {
    loft_fn f;
    f.d = d;
    f.clos = clos;
    return f;
}

static loft_fn loft_fn_of(loft_cx *cx, int64_t d) {
    return loft_fn_make(cx, d, loft_ref_null(cx));
}

#ifdef LOFT_HOSTED
/* `@PLN206` — a hosted unit's function values carry the interpreter's function numbers, as its
 * stores carry the interpreter's type numbers: the unit's function `d` as the interpreter
 * numbers it, and back (`-1`: none of the unit's, one the interpreter calls). */
static loft_word loft_fhost(loft_cx *cx, loft_word d);
static loft_word loft_funit(loft_cx *cx, loft_word h);
#endif

/* `Stores::evacuated` (`@FR-H-RefHome`): the copy each record a stored reference named was
 * evacuated to, per holder store, so a second holder in that store shares the one copy; an
 * entry goes with the store its record lived in. */
typedef struct loft_evac { loft_ref old; uint16_t holder; loft_ref copy; } loft_evac;

/* OpFreeRef: the whole store goes, everything the root owned with it. */
static void loft_gen_free(loft_cx *cx, loft_ref g);

/* `Stores::release_file_leases` (loft_io.h, which sets it): a store that is freed returns the
 * leases its records hold.  A tentative definition, so a unit without the file layer — the
 * allocator and keyed harnesses — leaves it null. */
static void (*loft_release_file_leases_hook)(loft_cx *cx, uint16_t nr);

/* `@FR-R-Transient`: the arena a transient value lives in — a value built, read and dropped
 * without being stored, held outside every store.  One block per thread, then a chain of
 * spill blocks.  A root site takes a mark before it builds and gives the mark back at each of
 * its exits: everything allocated since dies at once, so nothing is freed node by node and
 * nothing reaches the leak report.  A spill that cannot be allocated halts as a store that
 * cannot grow does. */
typedef struct loft_ta_blk { struct loft_ta_blk *prev; size_t cap, top; } loft_ta_blk;
typedef struct { size_t top; loft_ta_blk *spill; size_t spill_top; } loft_ta_mark_t;

static inline loft_ta_mark_t loft_ta_mark(loft_cx *cx) {
    loft_ta_mark_t m = { cx->ta_top, cx->ta_spill, cx->ta_spill ? cx->ta_spill->top : 0 };
    return m;
}

/* `n` bytes, 8-aligned, zeroed. */
static inline void *loft_ta_alloc(loft_cx *cx, size_t n) {
    n = (n + 7) & ~(size_t) 7;
    if (!cx->ta_spill && cx->ta_top + n <= LOFT_TA_BASE) {
        void *p = (unsigned char *) cx->ta_base + cx->ta_top;
        cx->ta_top += n;
        memset(p, 0, n);
        return p;
    }
    if (!cx->ta_spill || cx->ta_spill->top + n > cx->ta_spill->cap) {
        size_t cap = n > LOFT_TA_BASE ? n : LOFT_TA_BASE;
        loft_ta_blk *b = (loft_ta_blk *) malloc(sizeof(loft_ta_blk) + cap);
        if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
        b->prev = cx->ta_spill;
        b->cap = cap;
        b->top = 0;
        cx->ta_spill = b;
    }
    void *p = (unsigned char *) (cx->ta_spill + 1) + cx->ta_spill->top;
    cx->ta_spill->top += n;
    memset(p, 0, n);
    return p;
}

/* Everything allocated since `m` dies: the spill blocks made after it are freed. */
static inline void loft_ta_back(loft_cx *cx, loft_ta_mark_t m) {
    while (cx->ta_spill && cx->ta_spill != m.spill) {
        loft_ta_blk *b = cx->ta_spill;
        cx->ta_spill = b->prev;
        free(b);
    }
    if (cx->ta_spill) cx->ta_spill->top = m.spill_top;
    cx->ta_top = m.top;
}

/* A transient vector: its elements in the arena, `n` of them, room for `cap`. */
typedef struct { void *p; uint32_t n, cap; } loft_ta_vec;

/* A new zeroed element of `size` bytes at the end of `v`; a vector out of room moves to a
 * block twice as big (the old one is dead space until the mark goes back). */
static inline void *loft_ta_vec_push(loft_cx *cx, loft_ta_vec *v, size_t size) {
    if (v->n == v->cap) {
        uint32_t cap = v->cap ? v->cap * 2 : 4;
        void *p = loft_ta_alloc(cx, (size_t) cap * size);
        if (v->n) memcpy(p, v->p, (size_t) v->n * size);
        v->p = p;
        v->cap = cap;
    }
    void *e = (unsigned char *) v->p + (size_t) v->n * size;
    v->n++;
    return e;
}

/* Element `i` of `v` — a negative `i` counting from the end, as a store vector's read does —
 * or 0 past either end, what that read answers as null. */
static inline void *loft_ta_vec_at(loft_cx *cx, const loft_ta_vec *v, int64_t i, size_t size) {
    if (i < 0) i += (int64_t) v->n;
    if (i < 0 || i >= (int64_t) v->n) return 0;
    return (unsigned char *) v->p + (size_t) i * size;
}

/* A zeroed record nothing writes: what a `v[i]?` that missed answers in a transient tree (a
 * type whose default is all zeros).  One past the static block's size takes arena room. */
static inline void *loft_ta_zero(loft_cx *cx, size_t n) {
    static const uint64_t z[128];
    return n <= sizeof z ? (void *) z : loft_ta_alloc(cx, n);
}

/* A text's bytes copied into the arena. */
static inline loft_str loft_ta_str(loft_cx *cx, loft_str s) {
    loft_str out = { 0, 0 };
    if (s.len == 0) return out;
    char *p = (char *) loft_ta_alloc(cx, s.len);
    memcpy(p, s.p, s.len);
    out.p = p;
    out.len = s.len;
    return out;
}

/* `@FR-R-SliceArg`: the carrier stores a slice handed to a `const` parameter is copied into —
 * a few per thread, kept across uses instead of minted and freed per use.  An idle carrier is
 * PINNED, so the leak report passes it by and a stray free leaves it alone; one in use is an
 * ordinary store until the free that would have released it gives it back (`loft_free_ref`). */

/* Give carrier `nr` back if it is one in use: answers whether it was. */
static int loft_slice_give(loft_cx *cx, uint32_t nr) {
    for (int i = 0; i < cx->sp_n; i++) {
        if (cx->sp_used[i] && cx->sp_ref[i].store_nr == nr) {
            cx->sp_used[i] = 0;
            cx->stores[nr].pinned = 1;
            return 1;
        }
    }
    return 0;
}

/* `@FR-H-FreeTwice` (`Stores::refuse_refree`): a free of a store that is already free is a
 * compiler defect, and the run fails at it as the interpreter's does — the slot number is all a
 * free checks, so a store made in between would be the one released.  `LOFT_ALLOW_REFREE` lets
 * it pass; a `-DLOFT_DOUBLE_FREE` build counts instead. */
static int loft_refree_allowed(loft_cx *cx) {
    static int v = -1;
    if (v < 0) {
        const char *e = getenv("LOFT_ALLOW_REFREE");
        v = e && *e && strcmp(e, "0") != 0;
    }
    return v;
}

/* `@FR-H-DropOnce` (`Stores::drop_enter` / `drop_leave`): the outermost release of a droppable
 * `tp` releases its value once.  Leaving it marks the store's root released
 * (`released_root`, cleared when the root is claimed again); entering it on a marked root ends
 * the run as a store freed twice does.  Only the root record of a store of type `tp` is marked
 * (`Stores::is_own_root`): a value held inline can share its position. */
static int loft_drop_own_root(loft_cx *cx, loft_ref r, uint16_t tp) {
    return r.rec == LOFT_ST_PRIMARY && r.pos == 8 && r.store_nr < cx->nstores
        && cx->stores[r.store_nr].live && LOFT_KNOWN(cx->stores[r.store_nr]) == tp;
}

static void loft_drop_enter(loft_cx *cx, loft_ref r, uint16_t tp) {
    if (!loft_drop_own_root(cx, r, tp) || !cx->stores[r.store_nr].released_root)
        return;
    char m[240];
    snprintf(m, sizeof m,
             "a value was released twice: its OpDrop ran again on record %u of store #%u — by a "
             "call of `OpDrop` in the program, or by a defect in the loft compiler (please report it)",
             (unsigned) r.rec, (unsigned) r.store_nr);
    fflush(stdout);
    fprintf(stderr, "error: %s\n", m);
    cx->fail_msg.len = 0;
    loft_push(cx, &cx->fail_msg, m, strlen(m));
    if (cx->in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(cx->test_jmp); }
    LOFT_EXIT(1);
}

static void loft_drop_leave(loft_cx *cx, loft_ref r, uint16_t tp) {
    if (loft_drop_own_root(cx, r, tp)) cx->stores[r.store_nr].released_root = 1;
}

static void loft_refree_fail(loft_cx *cx, uint32_t nr) {
    char m[160];
    snprintf(m, sizeof m,
             "a store was freed twice: store #%u (a defect in the loft compiler; please report it)",
             (unsigned) nr);
    fflush(stdout);
    fprintf(stderr, "error: %s\n", m);
    cx->fail_msg.len = 0;
    loft_push(cx, &cx->fail_msg, m, strlen(m));
    if (cx->in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(cx->test_jmp); }
    LOFT_EXIT(1);
}

#if defined(LOFT_STORE_DUMP) && defined(LOFT_ALLOC_TRACE)
static void loft_store_dump_one(loft_cx *cx, loft_store_t *s); /* loft_log.h */
#endif
static void loft_free_ref(loft_cx *cx, loft_ref r) {
    if (r.store_nr == 0xFFFE) {
        loft_gen_free(cx, r);
        return;
    }
    if (r.store_nr == 0xFFFF || r.store_nr >= cx->nstores) return;
    if (cx->sp_n && loft_slice_give(cx, r.store_nr)) return;
    loft_store_t *s = &cx->stores[r.store_nr];
#ifdef LOFT_DOUBLE_FREE
    if (!s->pinned && !s->live) loft_note_double_free(cx, r.store_nr);
#else
    if (!s->pinned && !s->live && !loft_refree_allowed(cx)) loft_refree_fail(cx, r.store_nr);
#endif
    /* `Stores::free_named`: a store already free stays as it is. */
    if (s->pinned || !s->live) return;
    /* `@FR-G-Arg-Rebind`: a store a live generator frame names is released when the frame is
     * done with it too — this free only orphans it. */
    if (s->frame_holds) {
        s->orphaned = 1;
        return;
    }
    /* `Stores::release_closure_share`: a store a closure copy shares is released by the last
     * holder — this free takes one share back (loft_store.h `closure_shares`, its one use). */
    if (s->closure_shares) {
        s->closure_shares--;
        return;
    }
    for (uint32_t i = 0; i < cx->nevacs;) {
        if (cx->evacs[i].old.store_nr == r.store_nr) cx->evacs[i] = cx->evacs[--cx->nevacs];
        else i++;
    }
#ifdef LOFT_HOSTED
    /* The interpreter releases the store (`Stores::free_named`): its own fields and its
     * table's bookkeeping go with it. */
    if (cx->host_stores) {
        loft_hosted->store_free(cx->host_stores, r.store_nr);
        loft_host_sync(cx);
        return;
    }
#endif
    if (loft_release_file_leases_hook) loft_release_file_leases_hook(cx, r.store_nr);
#if defined(LOFT_STORE_DUMP) && defined(LOFT_ALLOC_TRACE)
    /* `@PLN206 P4c` — the store's final content, for `scripts/store_diff.py`. */
    loft_store_dump_one(cx, s);
#endif
    s->live = 0;
    loft_store_freed(cx, r.store_nr);
#if defined(LOFT_STRICT_STORES) || defined(LOFT_DOUBLE_FREE)
    loft_note_free(cx, r.store_nr);
#endif
#ifdef LOFT_STRICT_STORES
    /* `Stores::trim_free_top`, as the interpreter's strict mode keeps it: a freed TOP slot
     * lowers the floor, down past every dead slot under it, so the next store takes that
     * number again — a run that frees and re-makes in a loop stays within the table. */
    if (r.store_nr + 1u == cx->strict_floor)
        while (cx->strict_floor > 0 && !cx->stores[cx->strict_floor - 1].live) cx->strict_floor--;
#endif
    LOFT_TRACE("F %s\n", loft_trace_name(cx, s));
#ifdef LOFT_FOREIGN
    /* A foreign store's bytes go with the handle: the owner drops here. */
    loft_st_release_foreign(cx, s);
#endif
    /* A store bound to a file is cut to its content first (`free_named`: the file is
     * content-sized whether or not its author called `store_reclaim`), then unmapped, its bytes
     * kept in the file, before the poison below could reach them. */
    if (s->mapped) {
        if (!s->read_only) (void) loft_st_reclaim_tail(cx, s);
        loft_map_release_hook(cx, s);
    }
#ifdef LOFT_POISON
    /* `LOFT_POISON` (free_named): every byte past the first word reads 0xDEADBEEF, so a stale
     * reference into the freed store reads loud garbage. */
    {
        static const uint8_t poison[4] = { 0xEF, 0xBE, 0xAD, 0xDE };
        for (size_t off = 8; off < (size_t) s->size * 8; off++) s->buf[off] = poison[off & 3];
    }
#endif
}

/* A failed test `longjmp`s past every scope-exit free of its frames, the stores they made among
 * them.  The interpreter runs each test in a fork of its own program, so nothing a test made
 * outlives it; here every store made since `mark` (`cx->stores_allocated` as the test began)
 * that is still live and not pinned is released (@PLN206 D31). */
LOFT_NOINLINE static void loft_release_stores_since(loft_cx *cx, uint64_t mark) {
    for (uint32_t i = 0; i < cx->nstores; i++) {
        const loft_store_t *s = &cx->stores[i];
        if (s->live && !s->pinned && s->serial > mark) {
            loft_ref r = { (uint16_t) i, 1, 8, 0 };
            loft_free_ref(cx, r);
        }
    }
}

/* ── Generators (@F34, the interpreter's coroutines) ──
 * A generator is a frame — every local of its function, the label it resumes at, and whether
 * it ran to its end — and the function that resumes it: that function restores the locals
 * from the frame, jumps to the label after the last `yield`, and at the next `yield` saves
 * them and returns the value through `out`.  An iterator value names a frame's slot in
 * `cx->gens`: store number 0xFFFE, record slot + 1. */
typedef struct { int32_t state; int32_t done; } loft_gen_head;
/* `holds`: the holders that share the frame (`(G-Hold)`) — a release by one of several gives its
 * hold back, and the last one releases the frame. */
typedef struct loft_gen_slot { void *frame; void (*resume)(loft_cx *cx, void *frame, void *out); void (*release)(loft_cx *cx, void *frame); uint32_t holds; } loft_gen_slot;

static loft_ref loft_gen_new(loft_cx *cx, void *frame, void (*resume)(loft_cx *, void *, void *), void (*release)(loft_cx *, void *)) {
    uint32_t i = 0;
    while (i < cx->ngens && cx->gens[i].frame) i++;
    if (i == cx->ngens) {
        uint32_t n = cx->ngens ? cx->ngens * 2 : 16;
        loft_gen_slot *g = (loft_gen_slot *) realloc(cx->gens, n * sizeof *g);
        if (!g) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(g + cx->ngens, 0, (n - cx->ngens) * sizeof *g);
        cx->gens = g;
        cx->ngens = n;
    }
    cx->gens[i].frame = frame;
    cx->gens[i].resume = resume;
    cx->gens[i].release = release;
    cx->gens[i].holds = 1;
    loft_ref r = {0};
    r.store_nr = 0xFFFE;
    r.rec = i + 1;
    r.pos = 0;
    return r;
}

static loft_gen_slot *loft_gen_of(loft_cx *cx, loft_ref g) {
    if (g.store_nr != 0xFFFE || g.rec == 0 || g.rec > cx->ngens || !cx->gens[g.rec - 1].frame) return 0;
    return &cx->gens[g.rec - 1];
}

/* OpCoroutineNext: the generator runs to its next `yield`, whose value lands in `*out`; one
 * that has run to its end leaves `*out` as it was — the null the caller put there. */
static void loft_gen_next(loft_cx *cx, loft_ref g, void *out) {
    loft_gen_slot *s = loft_gen_of(cx, g);
    if (!s || ((loft_gen_head *) s->frame)->done) return;
    s->resume(cx, s->frame, out);
}

/* OpCoroutineRetain (State::coroutine_retain): one more holder of the frame. */
static loft_ref loft_gen_retain(loft_cx *cx, loft_ref g) {
    loft_gen_slot *s = loft_gen_of(cx, g);
    if (s) s->holds++;
    return g;
}

/* OpCoroutineExhausted: has the generator run to its end (or is it no generator)? */
static uint8_t loft_gen_exhausted(loft_cx *cx, loft_ref g) {
    loft_gen_slot *s = loft_gen_of(cx, g);
    return (uint8_t) (!s || ((loft_gen_head *) s->frame)->done);
}

/* `Stores::hold_for_frame` (`@FR-G-Arg-Rebind`): a generator frame takes a hold on the store
 * of a heap argument it was handed; answers whether it did — the null reference, a generator
 * and a freed store are not held. */
static int loft_hold_for_frame(loft_cx *cx, loft_ref r) {
    if (r.store_nr >= cx->nstores || r.rec == 0 || !cx->stores[r.store_nr].live) return 0;
    cx->stores[r.store_nr].frame_holds++;
    return 1;
}

/* `Stores::release_frame_hold`: the last hold on a store its owner already released frees it. */
static void loft_release_frame_hold(loft_cx *cx, loft_ref r) {
    if (r.store_nr >= cx->nstores) return;
    loft_store_t *s = &cx->stores[r.store_nr];
    if (!s->frame_holds) return;
    s->frame_holds--;
    if (!s->frame_holds && s->orphaned) {
        s->orphaned = 0;
        loft_free_ref(cx, r);
    }
}

static void loft_gen_free(loft_cx *cx, loft_ref g) {
    loft_gen_slot *s = loft_gen_of(cx, g);
    if (!s) return;
    if (s->holds > 1) {
        s->holds--;
        return;
    }
    s->release(cx, s->frame);
    s->frame = 0;
}

/* A constant built at compile time (compile.rs build_const_vectors): its store's words as the
 * interpreter laid them out, loaded once and pinned for the whole run.  The vector is the
 * field at byte 8 of the store's first record. */
static loft_ref loft_const_store(loft_cx *cx, const uint32_t *words, uint32_t n) {
    /* The interpreter's constant store is its own, made before the program's: it takes no
     * serial here either, so the program's stores are numbered alike. */
    uint32_t nr = loft_store_make(cx, 0);
    loft_store_t *s = &cx->stores[nr];
    if (n > s->size) {
        loft_heap_moved(cx, s, s->size, n);
        uint8_t *b = (uint8_t *) realloc(s->buf, (size_t) n * 8);
        if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
        s->buf = b;
        s->size = n;
    }
    memcpy(s->buf, words, (size_t) n * 8);
    /* The words past the constant's are zero, as in a fresh store: a hosted unit makes its
     * constants at its first call, in a store the interpreter may hand back from an earlier
     * use, whose old words would read as blocks.  (`n` drops the constant's trailing zero
     * words, so its last record may reach past `n`.) */
    if (n < s->size) memset(s->buf + (size_t) n * 8, 0, (size_t) (s->size - n) * 8);
    loft_st_resync(cx, s);
    s->pinned = 1;
    s->read_only = LOFT_RO_CONST; /* `Store::lock_constant`: constant data is read-only, as on the interpreter */
    loft_ref r = { (uint16_t) nr, 1, 8, 0 };
    return r;
}

/* OpEqRef */
static int loft_ref_eq(loft_cx *cx, loft_ref a, loft_ref b) {
    if (a.rec == 0 || b.rec == 0) return a.rec == 0 && b.rec == 0;
    return a.store_nr == b.store_nr && a.rec == b.rec && a.pos == b.pos;
}

/* OpStoreLive (Stores::store_live): the reference names a record of a store not freed. */
static int loft_store_live(loft_cx *cx, loft_ref r) {
    return r.rec != 0 && r.store_nr != 0xFFFF && r.store_nr < cx->nstores && cx->stores[r.store_nr].live;
}

LOFT_HOT_INLINE loft_ref loft_field(loft_cx *cx, loft_ref r, uint32_t fld) {
    r.pos += fld;
    return r;
}

LOFT_HOT_INLINE int64_t loft_get_int(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    int64_t v;
    memcpy(&v, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), 8);
    return v;
}

/* `@FR-R-RecPtr`: the address of record view `r`'s first byte, held for a block that grows no
 * store — 0 for the null record, where every read through it answers its getter's sentinel and
 * every write is skipped, as `loft_get_*` / `loft_set_*` do — and whether its store is locked,
 * which a write through it reports as `loft_set_*`'s write check does. */
LOFT_HOT_INLINE uint8_t *loft_rec_ptr(loft_cx *cx, loft_ref r) {
    return r.rec == 0 ? 0 : loft_at(cx, loft_store_of(cx, r), r.rec, r.pos);
}
LOFT_HOT_INLINE int loft_rec_locked(loft_cx *cx, loft_ref r) { return r.rec != 0 ? loft_store_of(cx, r)->read_only : 0; }
LOFT_HOT_INLINE int64_t loft_pa_i64(loft_cx *cx, const uint8_t *p, uint32_t off) {
    int64_t v;
    if (!p) return LOFT_NULL;
    memcpy(&v, p + off, 8);
    return v;
}
LOFT_HOT_INLINE double loft_pa_f64(loft_cx *cx, const uint8_t *p, uint32_t off) {
    double v;
    if (!p) return (double) NAN;
    memcpy(&v, p + off, 8);
    return v;
}
LOFT_HOT_INLINE float loft_pa_f32(loft_cx *cx, const uint8_t *p, uint32_t off) {
    float v;
    if (!p) return (float) NAN;
    memcpy(&v, p + off, 4);
    return v;
}
LOFT_HOT_INLINE uint8_t loft_pa_bool(loft_cx *cx, const uint8_t *p, uint32_t off) { return p ? p[off] : LOFT_BOOL_NULL; }
LOFT_HOT_INLINE void loft_pa_set(loft_cx *cx, uint8_t *p, int locked, uint32_t off, const void *v, size_t n) {
    if (!p) return;
    if (LOFT_UNLIKELY(locked) && loft_locked_write_on(cx, (uint8_t) locked)) return;
    memcpy(p + off, v, n);
}
/* A program's field write at a held address: through a null record — `root`, the view the
 * address was taken from — the write is reported where it was dropped (`loft_write_dropped`). */
static void loft_write_dropped(loft_cx *cx, loft_ref r, const char *file, uint32_t line);
LOFT_HOT_INLINE void loft_pa_set_at(loft_cx *cx, uint8_t *p, int locked, uint32_t off, const void *v, size_t n,
                                    loft_ref root, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(!p)) { loft_write_dropped(cx, root, file, line); return; }
    loft_pa_set(cx, p, locked, off, v, n);
}

/* The bytes of record `r` for a run of scalar field writes (`@FR-R-ValueRecord`'s copy into a
 * place): `loft_set_int`'s null test and write check once, and one address — 0 for the null
 * record, which every write of the run skips as each setter would. */
static uint8_t *loft_rec_w(loft_cx *cx, loft_ref r) {
    if (r.rec == 0) return 0;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return 0;
    return loft_at(cx, loft_store_of(cx, r), r.rec, r.pos);
}

LOFT_HOT_INLINE void loft_set_int(loft_cx *cx, loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &v, 8);
}

static void loft_set_int4(loft_cx *cx, loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    int32_t x = v == LOFT_NULL ? INT32_MIN : (int32_t) v;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &x, 4);
}

/* @PLN217 — the same for a value held in 32 bits: its null is the field's own (INT32_MIN), so
 * the four bytes are written as they stand. */
static void loft_set_int4_32(loft_cx *cx, loft_ref r, uint32_t fld, int32_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &v, 4);
}

/* The narrow integer fields: a byte biased by `min` (255 the nullable form's null), a short
 * in the `+1` encoding (0 is null), a raw short (0xFFFF its null), a 4-byte int (INT32_MIN).
 * A value a field cannot hold takes its default: the nullable forms' is null. */
static int32_t loft_i32_of(loft_cx *cx, int64_t v) {
    return v == LOFT_NULL ? INT32_MIN : (int32_t) (uint32_t) (uint64_t) v;
}

/* Each narrow read is decoded once, by `loft_dec_*` at a field's non-null address; the null
 * record is tested once on each path in: by `loft_get_*` on the reference, by `loft_pa_*` on a
 * held record's address (`@FR-R-RecPtr`, the null address answering null). */
LOFT_HOT_INLINE int64_t loft_dec_byte(loft_cx *cx, const uint8_t *q, int64_t min) { return q[0] + min; }
LOFT_HOT_INLINE int64_t loft_dec_byte_nullable(loft_cx *cx, const uint8_t *q, int64_t min) {
    return q[0] == 255 ? LOFT_NULL : q[0] + min;
}

LOFT_HOT_INLINE int64_t loft_pa_byte(loft_cx *cx, const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_byte(cx, p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_byte(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_byte(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), min);
}

LOFT_HOT_INLINE int64_t loft_pa_byte_nullable(loft_cx *cx, const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_byte_nullable(cx, p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_byte_nullable(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_byte_nullable(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), min);
}

static uint8_t loft_byte_raw(loft_cx *cx, int64_t min, int64_t val);

static void loft_set_byte_v(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld) = loft_byte_raw(cx, min, v);
}

static void loft_set_byte_nullable(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    int32_t x = loft_i32_of(cx, v);
    if (!(x == INT32_MIN || ((int64_t) x >= min && (int64_t) x <= min + 255))) x = INT32_MIN;
    *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld) = loft_byte_raw(cx, min, x);
}

LOFT_HOT_INLINE int64_t loft_dec_short(loft_cx *cx, const uint8_t *q, int64_t min) {
    uint16_t raw;
    memcpy(&raw, q, 2);
    return raw == 0 ? LOFT_NULL : raw + min - 1;
}

LOFT_HOT_INLINE int64_t loft_pa_short(loft_cx *cx, const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_short(cx, p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_short(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_short(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), min);
}

static void loft_set_short(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    int32_t x = loft_i32_of(cx, v);
    /* Null is raw 0; out of range is the type's lowest value, raw 1 (loft#984). */
    uint16_t raw = 0;
    if (x != INT32_MIN) raw = (int64_t) x >= min && (int64_t) x <= min + 65534 ? (uint16_t) ((int64_t) x - min + 1) : 1;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &raw, 2);
}

LOFT_HOT_INLINE int64_t loft_dec_short_raw(loft_cx *cx, const uint8_t *q, int64_t min) {
    uint16_t raw;
    memcpy(&raw, q, 2);
    return raw + min;
}

LOFT_HOT_INLINE int64_t loft_pa_short_raw(loft_cx *cx, const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_short_raw(cx, p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_short_raw(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_short_raw(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), min);
}

static void loft_set_short_raw(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    int32_t x = loft_i32_of(cx, v);
    uint16_t raw;
    if (x == INT32_MIN) raw = 0xFFFF;
    else if ((int64_t) x >= min && (int64_t) x <= min + 65535) raw = (uint16_t) ((int64_t) x - min);
    else raw = 0;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &raw, 2);
}

LOFT_HOT_INLINE int64_t loft_dec_int4(loft_cx *cx, const uint8_t *q) {
    int32_t x;
    memcpy(&x, q, 4);
    return x == INT32_MIN ? LOFT_NULL : x;
}

LOFT_HOT_INLINE int64_t loft_pa_int4(loft_cx *cx, const uint8_t *p, uint32_t fld) { return p ? loft_dec_int4(cx, p + fld) : LOFT_NULL; }

static int64_t loft_get_int4(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_int4(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld));
}

/* OpGetInt4 in a 32-bit build (@PLN217): the field as it is stored, its null (INT32_MIN) being
 * the 32-bit null, so no 64-bit value is formed to be narrowed again. */
LOFT_HOT_INLINE int32_t loft_pa_int4_32(loft_cx *cx, const uint8_t *p, uint32_t fld) {
    int32_t x;
    if (!p) return LOFT_NULL32;
    memcpy(&x, p + fld, 4);
    return x;
}

static int32_t loft_get_int4_32(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL32;
    return loft_pa_int4_32(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), fld);
}

/* OpGetInt4Raw / OpGetInt4Full / OpSetInt4Raw: an unsigned 4-byte field — the Raw form keeps
 * 0xFFFFFFFF for null, the Full form reserves nothing. */
LOFT_HOT_INLINE int64_t loft_dec_int4_raw(loft_cx *cx, const uint8_t *q) {
    uint32_t x;
    memcpy(&x, q, 4);
    return x == 0xFFFFFFFFu ? LOFT_NULL : (int64_t) x;
}

LOFT_HOT_INLINE int64_t loft_pa_int4_raw(loft_cx *cx, const uint8_t *p, uint32_t fld) {
    return p ? loft_dec_int4_raw(cx, p + fld) : LOFT_NULL;
}

static int64_t loft_get_int4_raw(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_int4_raw(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld));
}

LOFT_HOT_INLINE int64_t loft_dec_int4_full(loft_cx *cx, const uint8_t *q) {
    uint32_t x;
    memcpy(&x, q, 4);
    return (int64_t) x;
}

LOFT_HOT_INLINE int64_t loft_pa_int4_full(loft_cx *cx, const uint8_t *p, uint32_t fld) {
    return p ? loft_dec_int4_full(cx, p + fld) : LOFT_NULL;
}

static int64_t loft_get_int4_full(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_int4_full(cx, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld));
}

static void loft_set_int4_raw(loft_cx *cx, loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    loft_set_u32(cx, loft_store_of(cx, r), r.rec, r.pos + fld, v == LOFT_NULL ? 0xFFFFFFFFu : (uint32_t) (uint64_t) v);
}

static uint32_t loft_get_char(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return 0;
    uint32_t c;
    memcpy(&c, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), 4);
    return c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF) ? 0 : c;
}

static void loft_set_char(loft_cx *cx, loft_ref r, uint32_t fld, uint32_t c) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &c, 4);
}

/* A whole reference held in a field (Stores::get_db_ref / set_db_ref): store, record and
 * position as three 4-byte words — how a closure record holds what it captured by reference. */
/* A removed record is deleted; when a stored reference named it, the delete leaves a
 * tombstone and that reference reads null (@FR-H-Removed, Stores::reference_removed). */
static void loft_remove_record(loft_cx *cx, loft_store_t *s, uint32_t rec) {
    loft_st_delete(cx, s, rec);
}

LOFT_HOT_INLINE int loft_record_removed(loft_cx *cx, loft_ref x) {
    if (x.rec == 0 || x.store_nr == 0xFFFF || x.store_nr >= cx->nstores || !cx->stores[x.store_nr].live) return 0;
    return loft_st_is_removed(cx, &cx->stores[x.store_nr], x.rec);
}

LOFT_HOT_INLINE loft_ref loft_get_dbref(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return loft_ref_null(cx);
    loft_store_t *s = loft_store_of(cx, r);
    loft_ref x = {0};
    x.store_nr = (uint16_t) loft_u32(cx, s, r.rec, r.pos + fld);
    x.rec = loft_u32(cx, s, r.rec, r.pos + fld + 4);
    x.pos = loft_u32(cx, s, r.rec, r.pos + fld + 8);
    if (loft_record_removed(cx, x)) return loft_ref_null(cx);
    return x;
}

LOFT_HOT_INLINE void loft_set_dbref(loft_cx *cx, loft_ref r, uint32_t fld, loft_ref x) {
    if (r.rec == 0) return;
    loft_store_t *s = loft_store_of(cx, r);
    loft_set_u32(cx, s, r.rec, r.pos + fld, x.store_nr);
    loft_set_u32(cx, s, r.rec, r.pos + fld + 4, x.rec);
    loft_set_u32(cx, s, r.rec, r.pos + fld + 8, x.pos);
    /* `Stores::set_db_ref`: the record named is marked, so removing it leaves a tombstone. */
    if (x.rec != 0 && x.store_nr < cx->nstores && cx->stores[x.store_nr].live) loft_st_note_referenced(cx, &cx->stores[x.store_nr], x.rec);
}

/* ── Vectors (src/vector.rs) ───────────────────────────────────────────────────────────── */

LOFT_HOT_INLINE uint32_t loft_collection_rec(loft_cx *cx, loft_store_t *s, uint32_t rec, uint32_t pos) {
    uint32_t v = loft_u32(cx, s, rec, pos);
    return v == 0xFFFFFFFFu ? 0 : v;
}

/* `@FR-R-SliceArg`: OpDatabase of a slice carrier — `var` takes a carrier from the pool, its
 * vector kept at length 0, so the slice that follows is copied into room the carrier already
 * has.  A variable that still holds its carrier (the site ran again before the free) keeps it.
 * A variable holding any other store, or a full pool, takes the ordinary wrapper. */
static void loft_slice_take(loft_cx *cx, const struct loft_tdesc_s *T, loft_ref *var, uint32_t bytes, uint16_t tp) {
    int i = 0;
    if (var->store_nr == 0xFFFF) {
        while (i < cx->sp_n && cx->sp_used[i]) i++;
    } else {
        while (i < cx->sp_n && !(cx->sp_used[i] && cx->sp_ref[i].store_nr == var->store_nr)) i++;
        if (i == cx->sp_n) i = LOFT_SLICE_POOL;
    }
    if (i == cx->sp_n && i < LOFT_SLICE_POOL) {
        loft_ref r = loft_ref_null(cx);
        loft_database(cx, &r, bytes);
        cx->sp_ref[i] = r;
        cx->sp_used[i] = 0;
        cx->sp_n++;
    }
    if (i >= cx->sp_n) {
        loft_database_wrapper(cx, T, var, bytes, tp);
        return;
    }
    loft_ref r = cx->sp_ref[i];
    loft_store_t *s = &cx->stores[r.store_nr];
#ifdef LOFT_FOREIGN
    loft_st_release_foreign(cx, s);
#endif
    uint32_t v = loft_collection_rec(cx, s, r.rec, r.pos);
    if (v != 0) loft_set_u32(cx, s, v, 4, 0);
    cx->sp_used[i] = 1;
    s->pinned = 0;
    LOFT_SET_KNOWN(*s, tp);
    *var = r;
}

/* The elements of `size` bytes a record of `words` 8-byte words holds past its 8-byte header.
 * In a 32-bit address space the bytes fit 32 bits and the division is the core's own (@PLN217);
 * elsewhere a record may pass 4 GiB, so the bytes are counted in 64 bits. */
LOFT_HOT_INLINE uint32_t loft_rec_elems(loft_cx *cx, uint32_t words, uint32_t size) {
#if UINTPTR_MAX == 0xFFFFFFFFu
    return (words * 8u - 8u) / size;
#else
    return (uint32_t) (((uint64_t) words * 8 - 8) / size);
#endif
}

/* `checked_vec_cap`: the header word and `count` elements of `size` bytes, in words, in 32 bits
 * (the store does no 64-bit arithmetic).  A count past what 32 bits hold stops the program, as
 * `checked_vec_cap` panics: a clamped size would be a record smaller than its length says. */
static uint32_t loft_vec_cap_words(loft_cx *cx, uint32_t count, uint32_t size) {
    if (size != 0 && count > (UINT32_MAX - 15) / size) {
        fprintf(stderr, "loft: Vector capacity overflow: count=%u size=%u\n", count, size);
        abort();
    }
    return (count * size + 15) / 8;
}

/* `@FR-Col-Capacity` (`vector::grown_elems`): the room a vector's growth step makes, for a
 * length of `needed` — twice that, and never less than 88 bytes of elements (one at least). */
#define LOFT_VEC_START_BYTES 88u
static uint32_t loft_vec_grown(loft_cx *cx, uint32_t needed, uint32_t size) {
    uint32_t start = LOFT_VEC_START_BYTES / (size ? size : 1);
    uint32_t twice = needed > UINT32_MAX / 2 ? UINT32_MAX : needed * 2;
    if (start < 1) start = 1;
    return twice > start ? twice : start;
}

LOFT_HOT_INLINE loft_int loft_vec_len(loft_cx *cx, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return 0;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    return r == 0 ? 0 : (loft_int) loft_u32(cx, s, r, 4);
}

/* A vector handed to a `#c` binding: its element bytes, valid for the call only (the store may
 * move on the next claim), NULL for an absent vector — as `c_call` hands them. */
LOFT_HOT_INLINE uint8_t *loft_cvec_ptr(loft_cx *cx, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return NULL;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    return r == 0 ? NULL : loft_elem_base(cx, s, r);
}

static void loft_vec_prealloc(loft_cx *cx, loft_ref v, uint32_t count, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(cx, v);
    if (loft_collection_rec(cx, s, v.rec, v.pos) != 0) return;
    /* A known count is reserved exactly (`@FR-Col-Capacity`). */
    uint32_t r = loft_claim(cx, s, loft_vec_cap_words(cx, count, size));
    loft_set_u32(cx, s, v.rec, v.pos, r);
    loft_set_u32(cx, s, r, 4, 0);
}

/* vector_append: room for one more element of `size`; the reference to its slot.  The
 * length is not bumped: a push bumps it once its value is written (append_done), a new record
 * once it is finished (vector_finish). */
static loft_ref loft_vec_append_slow(loft_cx *cx, loft_ref v, uint32_t size);

/* `vector::append`: room for one more element of `size` bytes at the end, the length unchanged
 * (`loft_vec_finish` counts it).  The common case — a block with room — inline: one store
 * lookup the code after it can share; a first block or a growth out of line. */
LOFT_HOT_INLINE loft_ref loft_vec_append(loft_cx *cx, loft_ref v, uint32_t size) {
    if (v.store_nr != 0xFFFF && v.rec != 0) {
        loft_store_t *s = loft_store_of(cx, v);
        uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
        if (r != 0) {
            uint32_t len = loft_u32(cx, s, r, 4);
            int32_t claimed;
            memcpy(&claimed, loft_at(cx, s, r, 0), 4);
            if ((uint64_t) (len + 1) * size + 8 <= (uint64_t) claimed * 8) {
                /* `@FR-H-WriteLocked-Whole`: a refused push answers no slot, as the slow path. */
                if (LOFT_WRITE_REFUSED(s)) return loft_ref_null(cx);
                loft_ref e = {0};
                e.store_nr = v.store_nr;
                e.rec = r;
                e.pos = 8 + len * size;
                return e;
            }
        }
    }
    return loft_vec_append_slow(cx, v, size);
}

static loft_ref loft_vec_append_slow(loft_cx *cx, loft_ref v, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return loft_ref_null(cx);
    loft_store_t *s = loft_store_of(cx, v);
    if (LOFT_WRITE_REFUSED(s)) return loft_ref_null(cx);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    uint32_t len;
    if (r == 0) {
        r = loft_claim(cx, s, loft_vec_cap_words(cx, loft_vec_grown(cx, 1, size), size));
        loft_set_u32(cx, s, v.rec, v.pos, r);
        loft_set_u32(cx, s, r, 4, 0);
        len = 0;
    } else {
        len = loft_u32(cx, s, r, 4);
        int32_t claimed;
        memcpy(&claimed, loft_at(cx, s, r, 0), 4);
        /* `len + 1 > (claimed * 8 - 8) / size`, without the division every append paid. */
        if ((uint64_t) (len + 1) * size + 8 > (uint64_t) claimed * 8) {
            uint32_t nr = loft_st_resize(cx, s, r, loft_vec_cap_words(cx, loft_vec_grown(cx, len + 1, size), size));
            if (nr != r) {
                loft_set_u32(cx, s, v.rec, v.pos, nr);
                r = nr;
            }
        }
    }
    loft_ref e = {0};
    e.store_nr = v.store_nr;
    e.rec = r;
    e.pos = 8 + len * size;
    return e;
}

/* The push of one more element of `size` into a vector that has its record and room for it:
 * the element's slot, with the length already counted — or 0, and the caller takes the general
 * path (`loft_vec_append`, which claims the record or grows it).  One reading of the record
 * instead of append's and finish's two, and no capacity division: every push of a growing
 * vector asks this first. */
LOFT_HOT_INLINE uint8_t *loft_vec_push_slot(loft_cx *cx, loft_ref v, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return 0;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return 0;
    uint32_t len = loft_u32(cx, s, r, 4);
    int32_t claimed;
    memcpy(&claimed, loft_at(cx, s, r, 0), 4);
    if ((uint64_t) (len + 1) * size + 8 > (uint64_t) claimed * 8) return 0;
    if (LOFT_WRITE_REFUSED(s)) return 0;
    loft_set_u32(cx, s, r, 4, len + 1);
    return loft_at(cx, s, r, 8 + len * size);
}

/* vector_finish: the element `loft_vec_append` made counts. */
LOFT_HOT_INLINE void loft_vec_finish(loft_cx *cx, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    loft_set_u32(cx, s, r, 4, loft_u32(cx, s, r, 4) + 1);
}

/* OpPushInt4: a 4-byte element, INT32_MIN for null. */
static void loft_vec_push_int4(loft_cx *cx, loft_ref v, int64_t x) {
    uint8_t *slot = loft_vec_push_slot(cx, v, 4);
    if (slot) {
        uint32_t w = x == LOFT_NULL ? 0x80000000u : (uint32_t) (uint64_t) x;
        memcpy(slot, &w, 4);
        return;
    }
    loft_ref e = loft_vec_append(cx, v, 4);
    if (e.rec == 0) return;
    loft_set_u32(cx, loft_store_of(cx, e), e.rec, e.pos, x == LOFT_NULL ? 0x80000000u : (uint32_t) (uint64_t) x);
    loft_vec_finish(cx, v);
}

/* @PLN217 — the same for a value held in 32 bits: its null is the element's own bit pattern. */
static void loft_vec_push_int4_32(loft_cx *cx, loft_ref v, int32_t x) {
    uint32_t w = (uint32_t) x;
    uint8_t *slot = loft_vec_push_slot(cx, v, 4);
    if (slot) {
        memcpy(slot, &w, 4);
        return;
    }
    loft_ref e = loft_vec_append(cx, v, 4);
    if (e.rec == 0) return;
    loft_set_u32(cx, loft_store_of(cx, e), e.rec, e.pos, w);
    loft_vec_finish(cx, v);
}

static void loft_vec_push_int(loft_cx *cx, loft_ref v, int64_t x) {
    uint8_t *slot = loft_vec_push_slot(cx, v, 8);
    if (slot) {
        memcpy(slot, &x, 8);
        return;
    }
    loft_ref e = loft_vec_append(cx, v, 8);
    if (e.rec == 0) return;
    loft_set_int(cx, e, 0, x);
    loft_vec_finish(cx, v);
}

/* A new record's defaults: the bytes the interpreter's default walk writes for its type
 * (Stores::set_default_value, captured by the emitter as `image`). */
static void loft_fill(loft_cx *cx, loft_ref r, const uint8_t *image, uint32_t n) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), image, n);
}

/* get_vector: the element at `from` (negative counts from the end), or the null reference. */
/* `@FR-R-ElementPath`: the collection record and the length read once each, one unsigned
 * compare (a negative index past the front fails it too), the element's reference built here. */
LOFT_HOT_INLINE loft_ref loft_vec_get(loft_cx *cx, loft_ref v, uint32_t size, int64_t from) {
    loft_ref e = loft_ref_null(cx);
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return e;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return e;
    int64_t len = (int64_t) loft_u32(cx, s, r, 4);
    int64_t f = from < 0 ? from + len : from;
    if ((uint64_t) f >= (uint64_t) len) return e;
    e.store_nr = v.store_nr;
    e.rec = r;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}


/* A vector held for a loop (@FR-R-Header, opt::header): its store, record, length and element
 * base, read once before a loop that writes no store but in place — no record moves and no
 * length changes inside it — so an element read is one bounds test and one load. */
typedef struct { uint16_t store_nr; uint32_t rec; loft_int len; uint8_t *base; uint8_t locked; } loft_vh;

/* `@FR-H-WriteLocked` — a header over a locked store has no writable element: its lock state is
 * read with the header (no loop the hoist admits locks or unlocks a store), and a write through
 * it takes the runtime's refusing path (`loft_locked_write`: a development run stops, a
 * production run discards the write and goes on). */
#define LOFT_VH_WRITABLE(h) \
    if (LOFT_UNLIKELY((h).locked) && loft_locked_write_on(cx, (h).locked)) return
/* `base` is the element base (`Store::elem_base`), past the record's header. */

LOFT_HOT_INLINE loft_vh loft_vh_of(loft_cx *cx, loft_ref v) {
    loft_vh h;
    h.store_nr = v.store_nr;
    h.rec = 0;
    h.len = 0;
    h.base = 0;
    h.locked = 0;
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return h;
    loft_store_t *s = loft_store_of(cx, v);
    h.locked = s->read_only;
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return h;
    h.rec = r;
    h.len = loft_u32(cx, s, r, 4);
    h.base = loft_elem_base(cx, s, r);
    return h;
}

/* A header held by a loop that grows a store: the record and the length stand (no growth moves
 * another vector's record or changes its length, `@FR-R-Base`), the store's memory may have
 * moved, so the base is derived again from the held record — one load, no record walk. */
LOFT_HOT_INLINE loft_vh loft_vh_fresh(loft_cx *cx, loft_vh h) {
    if (h.rec != 0) h.base = loft_elem_base(cx, &cx->stores[h.store_nr], h.rec);
    return h;
}

/* (R-Header)'s proof, checked under LOFT_VERIFY at every held access: the vector still has the
 * record, length and base it was held with. */
static int loft_vh_same(loft_cx *cx, loft_vh h, loft_ref v) {
    loft_vh now = loft_vh_of(cx, v);
    return now.store_nr == h.store_nr && now.rec == h.rec && now.len == h.len && now.base == h.base;
}

/* The element index `from` names (negative from the end), or -1 outside, as loft_vec_get.
 * Branch-free: a negative `from` adds the length (`from >> 63` is all ones), and one unsigned
 * compare tests both ends — the null index, the most negative one, stays outside. */
#define LOFT_VH_WRAP(h, from) ((from) < 0 ? (from) + (h).len : (from))
#define LOFT_VH_OUT(h, f) LOFT_UNLIKELY((loft_uint) (f) >= (loft_uint) (h).len)
LOFT_HOT_INLINE loft_int loft_vh_index(loft_cx *cx, loft_vh h, loft_int from) {
    loft_int f = LOFT_VH_WRAP(h, from);
    return LOFT_VH_OUT(h, f) ? -1 : f;
}

LOFT_HOT_INLINE loft_ref loft_vh_get(loft_cx *cx, loft_vh h, uint32_t size, loft_int from) {
    loft_int f = loft_vh_index(cx, h, from);
    if (f < 0) return loft_ref_null(cx);
    loft_ref e = {0};
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}

LOFT_HOT_INLINE int64_t loft_vh_get_int(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld) {
    loft_int f = loft_vh_index(cx, h, from);
    if (f < 0) return LOFT_NULL;
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}

/* The address of element `from` (negative from the end) through the held base, or the null
 * address outside the vector — the `loft_pa_*` readers answer null there. */
LOFT_HOT_INLINE uint8_t *loft_vh_addr(loft_cx *cx, loft_vh h, uint32_t size, loft_int from) {
    loft_int f = loft_vh_index(cx, h, from);
    return f < 0 ? (uint8_t *) 0 : h.base + (size_t) f * size;
}

LOFT_HOT_INLINE void loft_vh_set_int(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v) {
    loft_int f = loft_vh_index(cx, h, from);
    if (f < 0) return;
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

LOFT_HOT_INLINE double loft_vh_get_float(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld) {
    loft_int f = loft_vh_index(cx, h, from);
    if (f < 0) return (double) NAN;
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}

LOFT_HOT_INLINE void loft_vh_set_float(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, double v) {
    loft_int f = loft_vh_index(cx, h, from);
    if (f < 0) return;
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

/* @FR-R-Version — a fast twin's element read and write at an index its precondition proved in
 * [0, len). */
LOFT_HOT_INLINE int64_t loft_vh_at_int(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld) {
    LOFT_PROVED(f >= 0 && f < h.len, "a proven index: int");
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}

LOFT_HOT_INLINE void loft_vh_put_int(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, int64_t v) {
    LOFT_PROVED(f >= 0 && f < h.len, "a proven index: put int");
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

/* @FR-R-Version — a fast twin's boolean element write at an index its precondition proved. */
LOFT_HOT_INLINE void loft_vh_put_bool(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, uint8_t v) {
    LOFT_PROVED(f >= 0 && f < h.len, "a proven index: put bool");
    LOFT_VH_WRITABLE(h);
    h.base[(size_t) f * size + fld] = v;
}

/* @FR-R-Base's guard clause — a float element's write at an index proven in [0, len). */
LOFT_HOT_INLINE void loft_vh_put_float(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, double v) {
    LOFT_PROVED(f >= 0 && f < h.len, "a proven index: put float");
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

/* @FR-R-Base's counter clause — an element of a held vector at a range counter proven in
 * [0, len): its float, its reference and its address, no index test. */
LOFT_HOT_INLINE double loft_vh_at_float(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld) {
    LOFT_PROVED(f >= 0 && f < h.len, "R-Base counter: float");
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE loft_ref loft_vh_at_ref(loft_cx *cx, loft_vh h, uint32_t size, loft_int f) {
    LOFT_PROVED(f >= 0 && f < h.len, "R-Base counter: element");
    loft_ref e = loft_ref_null(cx);
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}
/* @FR-R-Base's grid clause — does a held vector hold every index a * w + b with 0 <= a < rows and
 * 0 <= b < w?  The largest is rows * w - 1, so: rows and w not negative (a null is not) and
 * rows * w, not overflowing, no more than the length.  A multiply, not a division: asked at
 * every read, a division was what the compiler did not hoist (terrain 3.06x; 1.70x so). */
LOFT_HOT_INLINE int loft_grid_fits(loft_cx *cx, loft_vh h, int64_t rows, int64_t w) {
    int64_t p;
    return rows >= 0 && w >= 0 && !LOFT_MUL_OVF(rows, w, &p) && p <= h.len;
}
LOFT_HOT_INLINE uint8_t *loft_vh_at_addr(loft_cx *cx, loft_vh h, uint32_t size, loft_int f) {
    LOFT_PROVED(f >= 0 && f < h.len, "R-Base counter: address");
    return h.base + (size_t) f * size;
}

/* @FR-R-Version — a precondition's arithmetic: a null operand, an overflow or a null result
 * clears `*ok` (the checked operator would answer null where the plain one does not). */
static inline int64_t loft_vg_add(loft_cx *cx, int64_t a, int64_t b, int *ok) {
    int64_t r;
    if (a == LOFT_NULL || b == LOFT_NULL || LOFT_ADD_OVF(a, b, &r) || r == LOFT_NULL) { *ok = 0; return 0; }
    return r;
}
static inline int64_t loft_vg_sub(loft_cx *cx, int64_t a, int64_t b, int *ok) {
    int64_t r;
    if (a == LOFT_NULL || b == LOFT_NULL || LOFT_SUB_OVF(a, b, &r) || r == LOFT_NULL) { *ok = 0; return 0; }
    return r;
}
static inline int64_t loft_vg_mul(loft_cx *cx, int64_t a, int64_t b, int *ok) {
    int64_t r;
    if (a == LOFT_NULL || b == LOFT_NULL || LOFT_MUL_OVF(a, b, &r) || r == LOFT_NULL) { *ok = 0; return 0; }
    return r;
}
static inline int64_t loft_vg_neg(loft_cx *cx, int64_t a, int *ok) {
    if (a == LOFT_NULL) { *ok = 0; return 0; }
    return -a;
}
/* The counter's last value from its first, `first`: `end` itself (`inclusive`) or the value
 * below it; 0 when the range holds fewer than `min` values, where asking the precondition twice
 * costs more than the tests it saves. */
static inline int loft_vg_span(loft_cx *cx, int64_t first, int64_t end, int inclusive, int64_t min, int64_t *last) {
    int64_t d;
    if (inclusive) {
        *last = end;
    } else {
        if (end == LOFT_NULL) return 0;
        *last = end - 1;
    }
    if (first > *last) return 0;
    return LOFT_SUB_OVF(*last, first, &d) || d >= min - 1;
}
/* @FR-R-Version's nest clause — a box of two counter ranges, each non-empty, holds at least
 * `min` points. */
static inline int loft_vg_area(loft_cx *cx, int64_t f0, int64_t l0, int64_t f1, int64_t l1, int64_t min) {
    int64_t a, b;
    if (LOFT_SUB_OVF(l0, f0, &a) || LOFT_SUB_OVF(l1, f1, &b) || a >= min - 1 || b >= min - 1) {
        return 1;
    }
    return (a + 1) * (b + 1) >= min;
}
/* An argument `x + c * t` at counter value `t`: 0 where it does not fit or is null, or where
 * `x` is null (a checked argument that answered null does not move with the counter). */
static inline int loft_vg_lin(loft_cx *cx, int64_t x, int64_t c, int64_t t, int64_t *out) {
    int64_t m;
    if (x == LOFT_NULL || LOFT_MUL_OVF(c, t, &m) || LOFT_ADD_OVF(x, m, out)) return 0;
    return *out != LOFT_NULL;
}

/* @FR-R-Fill — the run `base + lo ..= base + last` of a held vector, as `Stores::fill_hoisted`
 * takes it: refused (0) when it is empty, when `base + i` overflows on the way, or when any
 * index falls outside [0, len) — the cases the loop answers itself (a negative index counts
 * from the end), so the caller runs the loop there.  `hi` is the range's end, `inclusive` its
 * form. */
static inline int loft_fill_run(loft_cx *cx, loft_vh h, int64_t base, int64_t lo, int64_t hi, int inclusive,
                                int64_t *first, int64_t *count) {
    int64_t last, f, e;
    if (inclusive) last = hi;
    else if (hi == INT64_MIN) return 0;
    else last = hi - 1;
    if (lo > last || LOFT_ADD_OVF(base, lo, &f) || LOFT_ADD_OVF(base, last, &e)) return 0;
    if (f < 0 || e >= h.len || h.locked) return 0;
    *first = f;
    *count = e - f + 1;
    return 1;
}

LOFT_HOT_INLINE int loft_vh_fill_i64(loft_cx *cx, loft_vh h, int64_t base, int64_t lo, int64_t hi, int inclusive,
                                     int64_t v) {
    int64_t f, n;
    if (!loft_fill_run(cx, h, base, lo, hi, inclusive, &f, &n)) return 0;
    uint64_t bits;
    memcpy(&bits, &v, 8);
    loft_fill8(h.base + (size_t) f * 8, bits, (size_t) n);
    return 1;
}

LOFT_HOT_INLINE int loft_vh_fill_f64(loft_cx *cx, loft_vh h, int64_t base, int64_t lo, int64_t hi, int inclusive,
                                     double v) {
    int64_t f, n;
    if (!loft_fill_run(cx, h, base, lo, hi, inclusive, &f, &n)) return 0;
    uint64_t bits;
    memcpy(&bits, &v, 8);
    loft_fill8(h.base + (size_t) f * 8, bits, (size_t) n);
    return 1;
}

LOFT_HOT_INLINE int loft_vh_fill_f32(loft_cx *cx, loft_vh h, int64_t base, int64_t lo, int64_t hi, int inclusive,
                                     float v) {
    int64_t f, n;
    if (!loft_fill_run(cx, h, base, lo, hi, inclusive, &f, &n)) return 0;
    uint32_t bits;
    memcpy(&bits, &v, 4);
    loft_fill4(h.base + (size_t) f * 4, bits, (size_t) n);
    return 1;
}

/* A `u32` element run: the code `loft_set_int4_raw` writes, null as 0xFFFFFFFF. */
LOFT_HOT_INLINE int loft_vh_fill_u32(loft_cx *cx, loft_vh h, int64_t base, int64_t lo, int64_t hi, int inclusive,
                                     int64_t v) {
    int64_t f, n;
    if (!loft_fill_run(cx, h, base, lo, hi, inclusive, &f, &n)) return 0;
    loft_fill4(h.base + (size_t) f * 4, v == LOFT_NULL ? 0xFFFFFFFFu : (uint32_t) (uint64_t) v, (size_t) n);
    return 1;
}

/* @FR-R-Fill's guarded clause: a body `t = base + i; if t < bound { v[t] = c }` writes exactly
 * the indexes below `bound`, so the range it fills ends at min(end, bound - base), saturating
 * where native's `saturating_sub` / `saturating_add` do. */
static inline int64_t loft_fill_guard_end(loft_cx *cx, int64_t hi, int inclusive, int64_t bound, int64_t base) {
    int64_t end, lim;
    if (!inclusive) end = hi;
    else if (LOFT_ADD_OVF(hi, (int64_t) 1, &end)) end = INT64_MAX;
    if (LOFT_SUB_OVF(bound, base, &lim)) lim = base > 0 ? INT64_MIN : INT64_MAX;
    return end < lim ? end : lim;
}

/* The guarded span's length, saturating: the fill is tried from FILL_MIN_SPAN elements. */
static inline int64_t loft_fill_span(loft_cx *cx, int64_t end, int64_t lo) {
    int64_t n;
    if (LOFT_SUB_OVF(end, lo, &n)) return lo > 0 ? INT64_MIN : INT64_MAX;
    return n;
}

/* ops::op_negate_long: null stays null. */
LOFT_HOT_INLINE int64_t loft_neg(loft_cx *cx, int64_t a) {
    return a == LOFT_NULL ? a : -a;
}

/* OpLengthText: characters, OpSizeText: bytes; both 0 for the null text. */
static loft_int loft_text_length(loft_cx *cx, loft_str s) {
    return loft_str_is_null(cx, s) ? 0 : (loft_int) loft_chars(cx, s);
}

static loft_int loft_text_size(loft_cx *cx, loft_str s) {
    return loft_str_is_null(cx, s) ? 0 : (loft_int) s.len;
}

/* Stores::free_displaced: free what a rebind displaced, unless it is the witness's store. */
/* A free a call's argument protection declines (Store::is_free_protected): a displaced store,
 * a copy's or a keyed write's given-up source.  A plain scope-exit free is not one. */
static int loft_free_protected(loft_cx *cx, loft_ref r) {
    return r.store_nr < cx->nstores && cx->stores[r.store_nr].protect > 0;
}

/* protect_store_frees / unprotect_store_frees: the call bracket around a reference argument. */
static void loft_protect(loft_cx *cx, loft_ref r) {
    if (r.store_nr < cx->nstores) cx->stores[r.store_nr].protect++;
}

static void loft_unprotect(loft_cx *cx, loft_ref r) {
    if (r.store_nr < cx->nstores && cx->stores[r.store_nr].protect > 0) cx->stores[r.store_nr].protect--;
}

static void loft_free_displaced(loft_cx *cx, loft_ref displaced, loft_ref witness) {
    if (displaced.store_nr == witness.store_nr || displaced.store_nr == 0xFFFF) return;
    if (loft_free_protected(cx, displaced)) return;
    loft_free_ref(cx, displaced);
}

/* OpFreeRef of a variable: its store released and the variable reset to the null sentinel, as
 * native's emitter resets it (and the bytecode writer's `OpInitRefSentinel` after an owned
 * rebind's free): a later free of the same variable — the next pass's pre-build free — is then
 * a no-op, where it would otherwise release whatever store reused the slot meanwhile. */
/* Inlined at every scope exit: the common case is a variable that was never minted, a test and
 * no call; the release itself (`loft_free_ref`) stays out of line. */
LOFT_HOT_INLINE void loft_free_var(loft_cx *cx, loft_ref *v) {
    if (v->store_nr == 0xFFFF) return;
    loft_free_ref(cx, *v);
    v->store_nr = 0xFFFF;
}

/* @FR-R-FnRefBuffer's frame clause: a kept record store parked between calls of the function
 * that keeps it, one slot per function, a static the main thread owns: a `par` worker and a
 * hosted unit's bridge call (`cx->no_park`) neither take nor park.  Taking empties the slot, so
 * a recursive or re-entered frame mints its own; parking into a full slot frees instead. */
static inline void loft_park_take(loft_cx *cx, loft_ref *slot, loft_ref *v) {
    if (cx->no_park || v->store_nr != 0xFFFF || slot->store_nr == 0xFFFF) return;
    *v = *slot;
    slot->store_nr = 0xFFFF;
    slot->rec = 0;
    slot->pos = 0;
}
/* The slots a store was ever parked in, so the program's end can give their stores back
 * (`loft_parks_drain`): a parked store is a cache of the next call's, and at exit there is no
 * next call — left in its slot it reads as a store never freed.  `par` workers never park
 * (`cx->no_park`), so the main thread's list holds every slot. */
static void loft_park_register(loft_cx *cx, loft_ref *slot) {
    for (uint32_t i = 0; i < cx->npark_slots; i++)
        if (cx->park_slots[i] == slot) return;
    if (cx->npark_slots == cx->park_slots_cap) {
        cx->park_slots_cap = cx->park_slots_cap ? cx->park_slots_cap * 2 : 8;
        cx->park_slots = (loft_ref **) realloc(cx->park_slots, cx->park_slots_cap * sizeof *cx->park_slots);
        if (!cx->park_slots) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    cx->park_slots[cx->npark_slots++] = slot;
}
static inline void loft_park_put(loft_cx *cx, loft_ref *slot, loft_ref *v) {
    if (!cx->no_park && slot->store_nr == 0xFFFF && v->store_nr != 0xFFFF) {
        loft_park_register(cx, slot);
        *slot = *v;
        v->store_nr = 0xFFFF;
        v->rec = 0;
        v->pos = 0;
    } else {
        loft_free_var(cx, v);
    }
}

/* The program's end: every parked store given back, so a store kept for a next call that never
 * comes is not reported as never freed (`loft_leak_report`). */
static void loft_parks_drain(loft_cx *cx) {
    for (uint32_t i = 0; i < cx->npark_slots; i++) loft_free_var(cx, cx->park_slots[i]);
    free(cx->park_slots);
    cx->park_slots = 0;
    cx->npark_slots = cx->park_slots_cap = 0;
}

/* OpFreeRefIfDistinct of a variable: released and reset when it names another store than the
 * witness (native's `OpFreeRefIfDistinctEmitter`). */
LOFT_HOT_INLINE void loft_free_var_distinct(loft_cx *cx, loft_ref *v, loft_ref witness) {
    if (v->store_nr == witness.store_nr || v->store_nr == 0xFFFF) return;
    if (!loft_free_protected(cx, *v)) loft_free_ref(cx, *v);
    v->store_nr = 0xFFFF;
}

/* `State::bind_fn_ref_result` / native's `cr_fnref_minted`: was the record a call through a
 * function value handed back MINTED by that call — a store made after `since` — rather than an
 * argument or a capture that predates it (or the closure's own record)?  A null is adopted as
 * the null it is. */
static int loft_fnref_minted(loft_cx *cx, loft_ref r, uint64_t since, loft_ref clos) {
    if (r.store_nr == 0xFFFF || r.rec == 0 || r.store_nr >= cx->nstores) return 1;
    if (clos.store_nr != 0xFFFF && r.store_nr == clos.store_nr) return 0;
    if (!cx->stores[r.store_nr].live) return 1;
    return cx->stores[r.store_nr].serial > since;
}

/* The stores a call through a function value handed back that no bind adopted — the buffer it
 * delivered into, or a store it minted — each with the serial of the allocation it named
 * (`codegen_runtime::FNREF_BUFS`, `State::release_fnref_bufs`).  The destination's static type
 * may say it owns nothing (a local that borrows on one assignment and receives this store on
 * another), so the FRAME holding it is its owner: a frame whose answer is no heap value frees
 * what it holds when it ends, one that answers a heap value hands it up (`framed`). */
typedef struct loft_fnref_held { loft_ref r; uint64_t serial; } loft_fnref_held;

/* One allocation is one entry: an entry left by a store its owner already freed names the same
 * slot as a store minted into it later, so it is replaced where it stands. */
static void loft_fnref_hold(loft_cx *cx, loft_ref r) {
    if (r.store_nr >= cx->nstores || r.rec == 0 || !cx->stores[r.store_nr].live) return;
    uint64_t serial = cx->stores[r.store_nr].serial;
    for (size_t i = 0; i < cx->nfnrefs; i++) {
        if (cx->fnrefs[i].r.store_nr == r.store_nr) {
            cx->fnrefs[i].r = r;
            cx->fnrefs[i].serial = serial;
            return;
        }
    }
    if (cx->nfnrefs == cx->capfnrefs) {
        cx->capfnrefs = cx->capfnrefs ? cx->capfnrefs * 2 : 16;
        cx->fnrefs = (loft_fnref_held *) realloc(cx->fnrefs, cx->capfnrefs * sizeof *cx->fnrefs);
        if (!cx->fnrefs) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    cx->fnrefs[cx->nfnrefs].r = r;
    cx->fnrefs[cx->nfnrefs].serial = serial;
    cx->nfnrefs++;
}

/* `cr_fnref_buf`: the buffer a dispatch arm minted is the call's answer — held by the frame —
 * or nobody's, and released. */
static void loft_fnref_buf(loft_cx *cx, loft_ref r, loft_ref hb) {
    if (r.store_nr != hb.store_nr) {
        loft_free_ref(cx, hb);
        return;
    }
    loft_fnref_hold(cx, hb);
}

/* `cr_fnref_minted`: a store the call minted is held by the frame; one that predates the call
 * (an argument, a capture) or is the closure's own record belongs further up. */
static void loft_fnref_mint(loft_cx *cx, loft_ref r, uint64_t since, loft_ref clos) {
    if (r.store_nr == 0xFFFF || r.rec == 0 || r.store_nr >= cx->nstores) return;
    if (clos.store_nr != 0xFFFF && r.store_nr == clos.store_nr) return;
    if (!cx->stores[r.store_nr].live || cx->stores[r.store_nr].serial <= since) return;
    loft_fnref_hold(cx, r);
}

/* `cr_fnref_adopt`: a bind that adopts the answer is its one owner; the frame lets it go. */
static void loft_fnref_adopt(loft_cx *cx, loft_ref src) {
    for (size_t i = cx->nfnrefs; i-- > 0;) {
        if (cx->fnrefs[i].r.store_nr == src.store_nr) {
            /* In order: every frame's mark keeps meaning the entries above it are its own. */
            memmove(&cx->fnrefs[i], &cx->fnrefs[i + 1], (cx->nfnrefs - i - 1) * sizeof *cx->fnrefs);
            cx->nfnrefs--;
            return;
        }
    }
}

/* `FnRefBufGuard::release`: free what the ending frame still holds above `mark` — only a store
 * still the allocation it named, since the frame may have freed it itself and the slot be
 * another's by now. */
static void loft_fnref_release(loft_cx *cx, size_t mark) {
    while (cx->nfnrefs > mark) {
        loft_fnref_held h = cx->fnrefs[--cx->nfnrefs];
        if (h.r.store_nr < cx->nstores && cx->stores[h.r.store_nr].live
            && cx->stores[h.r.store_nr].serial == h.serial)
            loft_free_ref(cx, h.r);
    }
}

/* vector::clear_vector: an absent slot becomes the empty vector; a present one length 0. */
static void loft_vec_clear(loft_cx *cx, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    loft_store_t *s = loft_store_of(cx, v);
#ifdef LOFT_FOREIGN
    /* A view of foreign bytes is dropped by its clear, never written. */
    if (loft_st_is_foreign(cx, s)) { loft_st_release_foreign(cx, s); return; }
#endif
    if (loft_u32(cx, s, v.rec, v.pos) == 0xFFFFFFFFu) { loft_set_u32(cx, s, v.rec, v.pos, 0); return; }
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r != 0) loft_set_u32(cx, s, r, 4, 0);
}


/* ── Bytes, enums and text in records ──────────────────────────────────────────────────── */

static void loft_set_byte(loft_cx *cx, loft_ref r, uint32_t fld, uint8_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld) = v;
}

/* OpCastEnumFromText (`Stores::to_enum`): the value of plain enum `names` (1-based, `n` with
 * the unused 0) whose name is `t`, 0 for none. */
static uint8_t loft_enum_of_text(loft_cx *cx, const char *const *names, size_t n, loft_str t) {
    for (size_t i = 1; i < n; i++)
        if (strlen(names[i]) == t.len && memcmp(names[i], t.p, t.len) == 0) return (uint8_t) i;
    return 0;
}

/* OpGetEnum: 0 for the null reference (the null address), the stored byte otherwise. */
LOFT_HOT_INLINE uint8_t loft_pa_enum(loft_cx *cx, const uint8_t *p, uint32_t fld) { return p ? p[fld] : 0; }

static uint8_t loft_get_enum(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return 0;
    return *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld);
}

static void loft_vec_push_byte(loft_cx *cx, loft_ref v, uint8_t x) {
    uint8_t *slot = loft_vec_push_slot(cx, v, 1);
    if (slot) {
        *slot = x;
        return;
    }
    loft_ref e = loft_vec_append(cx, v, 1);
    if (e.rec == 0) return;
    *loft_at(cx, loft_store_of(cx, e), e.rec, e.pos) = x;
    loft_vec_finish(cx, v);
}

/* OpRangeDefault: a value entering a narrow integer — null stays null (or takes the default
 * when one is given), a value outside [lo, hi] takes the default (a recoverable fault the
 * interpreter records silently). */
static int64_t loft_range_default(loft_cx *cx, int64_t v, int64_t lo, int64_t hi, int64_t dflt) {
    if (v == LOFT_NULL) return dflt == LOFT_NULL ? v : dflt;
    if (v >= lo && v <= hi) return v;
    return dflt;
}

/* OpGetBoolean: the null boolean for the null reference, the stored byte otherwise. */
static uint8_t loft_get_bool(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_BOOL_NULL;
    return *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld);
}

/* Store::byte_raw: the byte a slot biased by `min` holds for `val` (an i32, as `OpPushByte`
 * narrows it); 255 for null, 0 for a value out of the byte's range. */
static uint8_t loft_byte_raw(loft_cx *cx, int64_t min, int64_t val) {
    int32_t v = (int32_t) (uint32_t) (uint64_t) val;
    if (v == INT32_MIN) return 255;
    if ((int64_t) v >= min && (int64_t) v <= min + 255) return (uint8_t) ((int64_t) v - min);
    return 0;
}

/* vector::is_absent_collection: the null reference, no record, or the absent slot. */
static int loft_vec_absent(loft_cx *cx, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return 1;
    return loft_u32(cx, loft_store_of(cx, v), v.rec, v.pos) == 0xFFFFFFFFu;
}

/* OpGetText (Store::get_str): the field holds a string record — its length at byte 4, its
 * bytes from byte 8 — and 0 for the null text. */
/* Store::get_str: the string record `srec` (its length @4, its bytes from @8); null for no
 * record and for a length that runs past the store, so a corrupt record reads as null, never
 * past the buffer. */
static loft_str loft_store_str(loft_cx *cx, loft_store_t *s, uint32_t srec) {
    if (srec == 0 || srec > 0x7FFFFFFFu) return loft_null_text(cx);
    uint32_t len = loft_u32(cx, s, srec, 4);
    if (len / 8 + srec > s->size) return loft_null_text(cx);
    return loft_lit(cx, (const char *) loft_at(cx, s, srec, 8), len);
}

/* @FR-R-RecPtr's text clause — a text field through a held element's address: its string
 * record at `p + fld`, resolved in the element's store (its texts are records there); the null
 * address answers the null text, as loft_get_text of the null record. */
LOFT_HOT_INLINE loft_str loft_pa_text(loft_cx *cx, const uint8_t *p, uint32_t fld, uint16_t store_nr) {
    if (!p) return loft_null_text(cx);
    uint32_t srec;
    memcpy(&srec, p + fld, 4);
    return loft_store_str(cx, &cx->stores[store_nr], srec);
}

static loft_str loft_get_text(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return loft_null_text(cx);
    loft_store_t *s = loft_store_of(cx, r);
    return loft_store_str(cx, s, loft_u32(cx, s, r.rec, r.pos + fld));
}

/* OpSetText (Store::set_str): a new string record in the record's store.  A value that lives
 * in that store is copied out first, because the claim may move the buffer. */
static void loft_set_text(loft_cx *cx, loft_ref r, uint32_t fld, loft_str v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    loft_store_t *s = loft_store_of(cx, r);
    loft_text copy = { 0, 0, 0 };
    if (v.p >= (const char *) s->buf && v.p < (const char *) s->buf + (size_t) s->size * 8) {
        copy = loft_text_own(cx, v);
        v = loft_view(cx, &copy);
    }
    uint32_t srec = loft_st_claim(cx, s, (uint32_t) ((v.len + 15) / 8));
    loft_set_u32(cx, s, srec, 4, (uint32_t) v.len);
    if (v.len) memcpy(loft_at(cx, s, srec, 8), v.p, v.len);
    loft_set_u32(cx, s, r.rec, r.pos + fld, srec);
    loft_text_free(cx, &copy);
}

/* OpSetTextReplace (`Store::refill_str`): the text the field held is overwritten where it lies
 * when the new one fits its record, and released otherwise (`@FR-H-TextReplace`). */
static void loft_refill_text(loft_cx *cx, loft_ref r, uint32_t fld, loft_str v) {
    if (r.rec == 0) return;
    loft_store_t *s = loft_store_of(cx, r);
    if (LOFT_WRITE_REFUSED(s)) return;
    loft_text copy = { 0, 0, 0 };
    if (v.len && v.p >= (const char *) s->buf && v.p < (const char *) s->buf + (size_t) s->size * 8) {
        copy = loft_text_own(cx, v);
        v = loft_view(cx, &copy);
    }
    uint32_t old = loft_u32(cx, s, r.rec, r.pos + fld);
    if (old != 0) {
        int32_t words = loft_st_i32(cx, s, old, 0);
        if (words > 0 && v.len + 8 <= (size_t) words * 8) {
            loft_set_u32(cx, s, old, 4, (uint32_t) v.len);
            if (v.len) memcpy(loft_at(cx, s, old, 8), v.p, v.len);
            loft_text_free(cx, &copy);
            return;
        }
        loft_st_delete(cx, s, old);
    }
    uint32_t srec = loft_st_claim(cx, s, (uint32_t) ((v.len + 15) / 8));
    loft_set_u32(cx, s, srec, 4, (uint32_t) v.len);
    if (v.len) memcpy(loft_at(cx, s, srec, 8), v.p, v.len);
    loft_set_u32(cx, s, r.rec, r.pos + fld, srec);
    loft_text_free(cx, &copy);
}

/* ── Deep copies (Stores::copy_claims) ─────────────────────────────────────────────────────
 * A record's bytes are copied first; then every part it owns is copied into the destination's
 * store, walking a table of the program's types the emitter writes (`loft_types`). */
#define LOFT_T_SCALAR 0
#define LOFT_T_TEXT 1
#define LOFT_T_STRUCT 2
#define LOFT_T_VECTOR 3
#define LOFT_T_ENUM 4
#define LOFT_T_CHILD 5
#define LOFT_T_HASH 6
#define LOFT_T_INDEX 7
#define LOFT_T_ORDERED 8
#define LOFT_T_RADIX 9
#define LOFT_T_TRIE 10
/* A stored reference: copied and released as a scalar, its store number checked by a verify. */
#define LOFT_T_DBREF 11

/* A key part (keys::Key): its kind (negative: descending), its byte position in the element and
 * the start a narrow width counts from. */
typedef struct { int8_t type; uint16_t pos; int32_t start; } loft_key;

/* A struct field: its byte position, its type, whether it is a VIEW — a member of a linked
 * group that borrows its records from a sibling (`other_indexes`' marker) — and whether it is
 * hidden (a `#`-named field, which `type_at` passes over). */
/* A struct's field: its byte position, its type, whether it is a view member of a linked group
 * (it borrows its records; 2 when they are `vector<S?>` elements, whose null ones it does not
 * file), whether it is hidden (`#`-named), the group it is in — 1 + the field number of the
 * group's owner (the member that holds the records), 0 for none — and a stored reference's
 * target type (`Field::target`), 0xFFFF for any other field. */
typedef struct { uint16_t pos; uint16_t type; uint8_t view; uint8_t hidden; uint16_t group; uint16_t target; } loft_fld;
typedef struct loft_tdesc_s {
    uint8_t kind;
    uint16_t size;             /* the type's bytes */
    uint16_t elem;             /* a vector's element type, a child record's content type */
    uint16_t n;                /* fields, or variants */
    const loft_fld *fields;    /* a struct's fields */
    const uint16_t *variants;  /* a struct-enum's variant types, 0xFFFF for a unit variant */
    uint8_t owns_heap;         /* Stores::type_owns_heap: a value of it owns a claim of its own */
    const loft_key *keys;      /* a keyed collection's key parts */
    uint8_t nk;
    uint8_t linked;            /* Type::linked: a sibling or a stored reference names it by record */
    uint8_t file;              /* `@FR-H-Handle`: 2 a `File` record, 1 a type that can hold one */
} loft_tdesc;

/* The program's type table, bound at start (`loft_bind_tables`) for what a store's FREE walks:
 * the `File` leases its records hold.  0 in a runtime built without a unit. */
static const loft_tdesc *loft_T = 0;
static uint32_t loft_T_n = 0;
/* `@FR-H-Handle` (loft_io.h, which sets them): a `File` record's lease on its OS handle, taken
 * by a copy and returned by a record's death; the handle closes at the last return.  Null in a
 * unit without the file layer, which holds no `File`. */
static void (*loft_file_lease_hook)(loft_cx *cx, loft_ref f);
static void (*loft_file_release_hook)(loft_cx *cx, loft_ref f);

/* The hash's, the index's and the trees' halves of a copy and a release (loft_hash.h,
 * loft_btree.h, loft_radix.h). */
static void loft_hash_copy_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to);
static void loft_hash_remove_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed);
static void loft_ix_copy_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to);
static void loft_ix_remove_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed);
static void loft_rx_copy_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to);
static void loft_rx_remove_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed);

/* Copy `n` bytes between two places that may share a buffer a claim could move. */
static void loft_copy_bytes(loft_cx *cx, loft_ref to, uint32_t to_off, loft_ref from, uint32_t from_off, size_t n) {
    if (n == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, to))) return;
    memmove(loft_at(cx, loft_store_of(cx, to), to.rec, to.pos + to_off),
            loft_at(cx, loft_store_of(cx, from), from.rec, from.pos + from_off), n);
}

/* `relink_group_views` (loft_keyed.h, which sets it): a tentative definition, so a unit without
 * the keyed collections — and so without linked groups — leaves it null. */
static void (*loft_relink_hook)(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref to);

/* `@FR-L-OutOfLine`: the child records a copy or a release has met and not yet walked, and how
 * deep the walk is; the outermost call walks them as a loop, so a chain of any length takes no
 * stack per link (Stores::copy_claims, Stores::remove_claims). */
typedef struct loft_child_job { uint16_t tp; uint8_t release; loft_ref from, to; } loft_child_job;

static void loft_child_push(loft_cx *cx, uint16_t tp, int release, loft_ref from, loft_ref to) {
    if (cx->child_njobs == cx->child_cap) {
        cx->child_cap = cx->child_cap ? cx->child_cap * 2 : 64;
        cx->child_jobs = (loft_child_job *) realloc(cx->child_jobs, cx->child_cap * sizeof(loft_child_job));
        if (!cx->child_jobs) abort();
    }
    loft_child_job j = { tp, (uint8_t) release, from, to };
    cx->child_jobs[cx->child_njobs++] = j;
}

static void loft_copy_claims_one(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to);
static void loft_remove_claims_one(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed);

static void loft_child_drain(loft_cx *cx, const loft_tdesc *T) {
    while (cx->child_njobs) {
        loft_child_job j = cx->child_jobs[--cx->child_njobs];
        cx->child_depth++;
        if (j.release) {
            loft_remove_claims_one(cx, T, j.tp, j.from, 0);
            loft_st_delete(cx, loft_store_of(cx, j.from), j.from.rec);
        } else {
            loft_copy_claims_one(cx, T, j.tp, j.from, j.to);
        }
        cx->child_depth--;
    }
}

static void loft_copy_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to) {
    cx->child_depth++;
    loft_copy_claims_one(cx, T, tp, from, to);
    cx->child_depth--;
    if (cx->child_depth == 0) loft_child_drain(cx, T);
}

static void loft_copy_claims_one(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    loft_store_t *fs = loft_store_of(cx, from);
    switch (d->kind) {
    case LOFT_T_TEXT: {
        /* The text's block claimed, then its bytes copied straight from the source: the source
         * is re-resolved by its record number after the claim, which may have moved its store's
         * buffer when the two are one store. */
        uint32_t cur = loft_u32(cx, fs, from.rec, from.pos);
        if (cur == 0) { loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos, 0); return; }
        uint32_t len = loft_u32(cx, fs, cur, 4);
        loft_store_t *ts = loft_store_of(cx, to);
        uint32_t srec = loft_st_claim(cx, ts, (uint32_t) (((uint64_t) len + 15) / 8));
        loft_set_u32(cx, ts, srec, 4, len);
        if (len) memcpy(loft_at(cx, ts, srec, 8), loft_at(cx, loft_store_of(cx, from), cur, 8), len);
        loft_set_u32(cx, ts, to.rec, to.pos, srec);
        return;
    }
    case LOFT_T_STRUCT: {
        /* `@FR-R-FlatCopy` (Stores::copy_struct_fields): a record that owns no heap is its bytes,
         * which came with the block copy — no field has a claim to copy. */
        if (!d->owns_heap) return;
        /* `Stores::lease_if_file`: the copy names the same OS handle, one more lease on it. */
        if (d->file == 2 && loft_file_lease_hook) loft_file_lease_hook(cx, to);
        /* `copy_struct_fields` (@FR-Col-Group): a view member of a linked group holds the
         * records its owner holds — copied as a collection of its own it would hold copies a
         * write through the owner does not reach — so it starts empty and is indexed over the
         * copied owner's records. */
        int views = 0;
        for (uint16_t i = 0; i < d->n; i++) {
            const loft_fld *f = &d->fields[i];
            if (f->view) {
                uint32_t cur = loft_u32(cx, fs, from.rec, from.pos + f->pos);
                loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos + f->pos, cur == 0xFFFFFFFFu ? cur : 0);
                views = 1;
                continue;
            }
            loft_copy_claims(cx, T, f->type, loft_field(cx, from, f->pos), loft_field(cx, to, f->pos));
        }
        if (views && loft_relink_hook) loft_relink_hook(cx, T, tp, to);
        return;
    }
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(cx, fs, from.rec, from.pos);
        if (cur == 0 || cur == 0xFFFFFFFFu) { loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos, cur); return; }
        uint32_t len = loft_u32(cx, fs, cur, 4);
        uint32_t size = T[d->elem].size;
        uint32_t into = loft_st_claim(cx, loft_store_of(cx, to), 1 + (uint32_t) (((uint64_t) size * len + 7) / 8));
        loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos, into);
        loft_ref src_v = { from.store_nr, cur, 0, 0 };
        loft_ref dst_v = { to.store_nr, into, 0, 0 };
        loft_copy_bytes(cx, dst_v, 4, src_v, 4, (size_t) len * size + 4);
        /* Stores::zero_element_tails: into a DEFINED store (an image's), a struct element's
         * tail past its last field is zeroed — the bulk copy brought the source's leftover
         * bytes there, which differ between allocators and are not the program's. */
        if (loft_store_of(cx, to)->defined && T[d->elem].kind == LOFT_T_STRUCT) {
            const loft_tdesc *e = &T[d->elem];
            uint32_t used = 0;
            for (uint16_t k = 0; k < e->n; k++) {
                uint32_t end = (uint32_t) e->fields[k].pos + T[e->fields[k].type].size;
                if (end > used) used = end;
            }
            for (uint32_t i = 0; used < size && i < len; i++)
                memset(loft_at(cx, loft_store_of(cx, to), into, 8 + i * size + used), 0, size - used);
        }
        /* `@FR-R-FlatCopy`: an element that owns no heap is its bytes, which came with the
         * block copy — no element has a claim to copy. */
        if (!T[d->elem].owns_heap) return;
        for (uint32_t i = 0; i < len; i++) {
            loft_ref a = { from.store_nr, cur, 8 + i * size, 0 };
            loft_ref b = { to.store_nr, into, 8 + i * size, 0 };
            loft_copy_claims(cx, T, d->elem, a, b);
        }
        return;
    }
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(cx, fs, from.rec, from.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) loft_copy_claims(cx, T, d->variants[e], from, to);
        return;
    }
    case LOFT_T_HASH:
        loft_hash_copy_claims(cx, T, tp, from, to);
        return;
    case LOFT_T_INDEX:
        loft_ix_copy_claims(cx, T, tp, from, to);
        return;
    case LOFT_T_RADIX:
    case LOFT_T_TRIE:
        loft_rx_copy_claims(cx, T, tp, from, to);
        return;
    case LOFT_T_ORDERED: {
        /* Stores::copy_claims_array_body: a slot list of the source's length, each record it
         * names copied into a record of the destination's. */
        uint32_t cur = loft_u32(cx, fs, from.rec, from.pos);
        if (cur == 0 || cur == 0xFFFFFFFFu) { loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos, cur); return; }
        uint32_t len = loft_u32(cx, fs, cur, 4), size = T[d->elem].size;
        uint32_t into = loft_st_claim(cx, loft_store_of(cx, to), 1 + (len + 1) / 2);
        loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos, into);
        loft_set_u32(cx, loft_store_of(cx, to), into, 4, len);
        for (uint32_t i = 0; i < len; i++) {
            uint32_t elm = loft_u32(cx, loft_store_of(cx, from), cur, 8 + 4 * i);
            if (elm == 0) {
                loft_set_u32(cx, loft_store_of(cx, to), into, 8 + 4 * i, 0);
                continue;
            }
            loft_ref a = { from.store_nr, elm, 8, 0 };
            loft_ref b = { to.store_nr, loft_st_claim(cx, loft_store_of(cx, to), 1 + (size + 7) / 8), 8, 0 };
            loft_copy_bytes(cx, b, 0, a, 0, size);
            loft_set_u32(cx, loft_store_of(cx, to), into, 8 + 4 * i, b.rec);
            loft_copy_claims(cx, T, d->elem, a, b);
        }
        return;
    }
    case LOFT_T_CHILD: {
        uint32_t cur = loft_u32(cx, fs, from.rec, from.pos);
        if (cur == 0) { loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos, 0); return; }
        uint32_t size = T[d->elem].size;
        uint32_t into = loft_st_claim(cx, loft_store_of(cx, to), 1 + (size + 7) / 8);
        loft_ref a = { from.store_nr, cur, 8, 0 };
        loft_ref b = { to.store_nr, into, 8, 0 };
        loft_copy_bytes(cx, b, 0, a, 0, size);
        loft_set_u32(cx, loft_store_of(cx, to), to.rec, to.pos, into);
        loft_child_push(cx, d->elem, 0, a, b);
        return;
    }
    default:
        return;
    }
}

/* OpCopyRecord: the record's bytes and everything it owns, into the destination; the source's
 * store freed when the copy says so and it is another store (state/io.rs do_copy_record).  A
 * copy onto itself and a copy from or into a null reference do nothing. */
/* @FR-H-SwapIn (Stores::swap_stores_in): a copy that frees its source, from one store's root
 * to another's, both minted as `tp`, where the destination holds nothing but its root, MOVES
 * the source's content into the destination store instead of copying it.  The caller vouches
 * that `tp`'s tree holds no stored reference, which could name the source store. */
/* `Stores::swap_stores_in_as`'s live-state gate (loft_lazy.h, which sets it): null in a unit
 * without the walk scratches and lazy bindings it reads. */
static int (*loft_swap_blocked_hook)(loft_cx *cx);

/* `reset_dest`: the destination must hold nothing but its root (a copy's); a keyed rebind's
 * destination is its collection's own store, whose old content leaves with the released slot. */
static int loft_swap_in(loft_cx *cx, uint16_t tp, loft_ref src, loft_ref dst, int reset_dest) {
    if (src.store_nr == dst.store_nr || src.store_nr >= cx->nstores || dst.store_nr >= cx->nstores) return 0;
    if (src.rec != 1 || src.pos != 8 || dst.rec != 1 || dst.pos != 8) return 0;
    loft_store_t *a = &cx->stores[src.store_nr], *b = &cx->stores[dst.store_nr];
    if (!a->live || !b->live || LOFT_KNOWN(*a) != tp || LOFT_KNOWN(*b) != tp || a->pinned || b->pinned) return 0;
    /* `Store::content_swappable`: neither store locked (a lock is the slot's, and would move
     * with the content) nor serving foreign bytes nor bound to a file; only the RELEASED store's
     * protection refuses (`free_protect_depth`), a protected destination takes the content. */
    if (a->read_only || b->read_only || a->protect || a->nplaced || b->nplaced) return 0;
    if (a->mapped || b->mapped) return 0;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(cx, a) || loft_st_is_foreign(cx, b)) return 0;
#endif
    if (loft_swap_blocked_hook && loft_swap_blocked_hook(cx)) return 0;
    if (a->frame_holds || b->frame_holds) return 0;
    if (reset_dest && !loft_st_holds_only_root(cx, b)) return 0;
    /* `Store::swap_contents`: the content moves, each slot keeps its serial. */
    loft_store_t t = *a;
    *a = *b;
    *b = t;
    uint64_t serial = a->serial;
    a->serial = b->serial;
    b->serial = serial;
    /* Each slot holds the other's content: a handle on either resolves again. */
    loft_h_reshaped(cx, a);
    loft_h_reshaped(cx, b);
#ifdef LOFT_PARSE
    loft_perrs_swap(cx, src.store_nr, dst.store_nr);
#endif
    /* `LOFT_TRACE_STORE_SWAP=1` names each exchange as the interpreter does (`[swap-in]`). */
    {
        static int trace = -1;
        if (trace < 0) trace = getenv("LOFT_TRACE_STORE_SWAP") != NULL;
        if (trace)
            fprintf(stderr, "[swap-in] store #%u takes the content of #%u (tp=%u); #%u released\n",
                    (unsigned) dst.store_nr, (unsigned) src.store_nr, (unsigned) tp, (unsigned) src.store_nr);
    }
    loft_free_ref(cx, src);
    return 1;
}

/* `@FR-H-SwapRebind` (Stores::try_swap_rebind, `codegen_runtime::OpRebindRecord`): the rebind
 * `x = f(…)` whose result the bind gives up EXCHANGES the result's store into `*dst` without
 * the reset the copy would run first — `x`'s previous tree leaves with the released slot.
 * The emitter calls it only for a direct call whose result is given up and whose type keeps no
 * stored reference (`Cx::swap_safe`); a destination that holds no store takes a fresh one
 * first, and one that is not its store's typed root (a witnessed local's view) declines here,
 * and the caller resets and copies.  Answers 1 when the result was bound (exchanged, or copied
 * into the fresh store and given up).  `-DLOFT_NO_SWAP_REBIND` keeps the reset and the copy. */
static int loft_swap_rebind(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref *dst) {
#ifdef LOFT_NO_SWAP_REBIND
    (void) T; (void) tp; (void) src; (void) dst;
    return 0;
#else
    if (dst->store_nr == 0xFFFF) {
        /* `@FR-H-SwapIn`: a destination that holds no store takes a fresh typed one and the
         * exchange into it, reset — the bytecode's `OpDatabase` then its free-source copy.  An
         * exchange refused copies into that store and gives the result up, as the copy does. */
        if (src.store_nr == 0xFFFF || src.rec == 0) return 0;
        loft_database_typed(cx, dst, T[tp].size, tp);
        if (loft_swap_in(cx, tp, src, *dst, 1)) return 1;
#ifdef LOFT_PARSE
        loft_perrs_deliver(cx, src, *dst, tp, 1);
#endif
        loft_copy_bytes(cx, *dst, 0, src, 0, T[tp].size);
        loft_copy_claims(cx, T, tp, src, *dst);
        loft_free_ref(cx, src);
        return 1;
    }
    if (dst->rec != 1 || dst->pos != 8) return 0;
    return loft_swap_in(cx, tp, src, *dst, 0);
#endif
}

#ifdef LOFT_COPY_CHECK
/* `LOFT_COPY_CHECK` (`Stores::report_copy_mismatches`, @P317): after a deep copy the source and
 * the copy are walked in parallel, and every nested collection whose length differs is warned of
 * once per field path — a copy that inflates or truncates a nested collection, which no output
 * need show.  Warn and continue, so one run is a full census.  The driver builds with it when
 * `LOFT_COPY_CHECK` is set.  Defined further on: the keyed collections' lengths. */
static int64_t loft_hash_len(loft_cx *cx, loft_ref c);
static uint32_t loft_bt_len(loft_cx *cx, loft_ref data);
static uint64_t loft_copy_seen[256];
static uint32_t loft_ncopy_seen = 0;

static void loft_copy_warn(loft_cx *cx, const char *label, const char *path, const char *what, int64_t sl, int64_t dl, uint16_t kt) {
    uint64_t h = UINT64_C(0xcbf29ce484222325);
    for (const char *p = label; *p; p++) h = (h ^ (uint8_t) *p) * UINT64_C(0x100000001b3);
    for (const char *p = path; *p; p++) h = (h ^ (uint8_t) *p) * UINT64_C(0x100000001b3);
    for (uint32_t i = 0; i < loft_ncopy_seen; i++) if (loft_copy_seen[i] == h) return;
    if (loft_ncopy_seen < 256) loft_copy_seen[loft_ncopy_seen++] = h;
    fflush(stdout);
    fprintf(stderr, "[copy_check] MISMATCH %s%s: src_len=%lld dst_len=%lld (%s kt=%u)\n", label, path, (long long) sl,
            (long long) dl, what, (unsigned) kt);
}

static void loft_copy_cmp(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref s, loft_ref d, const char *label, char *path,
                          size_t at, int depth) {
    const loft_tdesc *t = &T[tp];
    if (depth > 32 || at > 200) return;
    path[at] = 0;
    switch (t->kind) {
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < t->n; i++) {
            loft_ref fs = s, fd = d;
            fs.pos += t->fields[i].pos;
            fd.pos += t->fields[i].pos;
            int w = snprintf(path + at, 256 - at, ".%u", (unsigned) i);
            loft_copy_cmp(cx, T, t->fields[i].type, fs, fd, label, path, at + (size_t) (w > 0 ? w : 0), depth + 1);
            path[at] = 0;
        }
        return;
    case LOFT_T_VECTOR:
    case LOFT_T_ORDERED: {
        int64_t sl = loft_vec_len(cx, s), dl = loft_vec_len(cx, d);
        if (sl != dl) loft_copy_warn(cx, label, path, t->kind == LOFT_T_VECTOR ? "vector elem" : "array/ordered", sl, dl, t->elem);
        const loft_tdesc *e = &T[t->elem];
        if (t->kind != LOFT_T_VECTOR || e->linked || !e->owns_heap || e->size == 0) return;
        uint32_t sv = loft_u32(cx, loft_store_of(cx, s), s.rec, s.pos), dv = loft_u32(cx, loft_store_of(cx, d), d.rec, d.pos);
        if (sv == 0 || dv == 0) return;
        int64_t n = sl < dl ? sl : dl;
        int w = snprintf(path + at, 256 - at, "[]");
        for (int64_t i = 0; i < n; i++) {
            loft_ref es = { s.store_nr, sv, 8 + (uint32_t) i * e->size, 0 }, ed = { d.store_nr, dv, 8 + (uint32_t) i * e->size, 0 };
            loft_copy_cmp(cx, T, t->elem, es, ed, label, path, at + (size_t) (w > 0 ? w : 0), depth + 1);
        }
        return;
    }
    case LOFT_T_HASH: {
        int64_t sl = loft_hash_len(cx, s), dl = loft_hash_len(cx, d);
        if (sl != dl) loft_copy_warn(cx, label, path, "hash", sl, dl, t->elem);
        return;
    }
    case LOFT_T_INDEX: {
        int64_t sl = loft_bt_len(cx, s), dl = loft_bt_len(cx, d);
        if (sl != dl) loft_copy_warn(cx, label, path, "index", sl, dl, t->elem);
        return;
    }
    case LOFT_T_CHILD: {
        uint32_t sc = loft_u32(cx, loft_store_of(cx, s), s.rec, s.pos), dc = loft_u32(cx, loft_store_of(cx, d), d.rec, d.pos);
        if (sc == 0 || dc == 0) return;
        loft_ref cs = { s.store_nr, sc, 8, 0 }, cd = { d.store_nr, dc, 8, 0 };
        loft_copy_cmp(cx, T, t->elem, cs, cd, label, path, at, depth + 1);
        return;
    }
    default:
        return;
    }
}

static void loft_report_copy_mismatches(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref s, loft_ref d, const char *label) {
    char path[256];
    if (s.store_nr == 0xFFFF || s.rec == 0 || d.store_nr == 0xFFFF || d.rec == 0) return;
    path[0] = 0;
    loft_copy_cmp(cx, T, tp, s, d, label, path, 0, 0);
}
#endif

static void loft_remove_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r);

/* OpCopyRecord's flags: bit 0 frees the source, bit 1 says the type may move (loft_swap_in). */
#ifdef LOFT_FOREIGN
static void loft_vec_add(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o);
#endif

static void loft_copy_record(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst, int flags) {
    int free_source = flags & 1, fresh_dest = flags & 4;
    if (src.store_nr == dst.store_nr && src.rec == dst.rec && src.pos == dst.pos) return;
    if (src.store_nr == 0xFFFF) {
        /* `Stores::copy_null_into`: a fresh destination is written absent. */
        if (fresh_dest && dst.store_nr != 0xFFFF && dst.rec != 0)
            memset(loft_at(cx, loft_store_of(cx, dst), dst.rec, dst.pos), 0, T[tp].size);
        return;
    }
    if (dst.store_nr == 0xFFFF || dst.rec == 0) {
        if (free_source && dst.store_nr != src.store_nr && !loft_free_protected(cx, src)) loft_free_ref(cx, src);
        return;
    }
    if (src.rec == 0) return;
#ifdef LOFT_FOREIGN
    /* `@FR-Const-Foreign` (State::do_copy_record): a vector read out of a foreign store is copied
     * element by element, as a bind of it copies — its bytes lie outside the store's blocks,
     * where the record copy would read a record that is not there. */
    if (T[tp].kind == LOFT_T_VECTOR && loft_st_is_foreign(cx, loft_store_of(cx, src))) {
        loft_vec_add(cx, T, T[tp].elem, dst, src);
        return;
    }
#endif
    if (free_source && (flags & 2) && loft_swap_in(cx, tp, src, dst, 1)) return;
#ifdef LOFT_PARSE
    loft_perrs_deliver(cx, src, dst, tp, 0);
#endif
    /* The destination's previous content goes first, unless it is fresh (`COPY_FRESH_DEST`). */
    if (!fresh_dest) loft_remove_claims(cx, T, tp, dst);
    loft_copy_bytes(cx, dst, 0, src, 0, T[tp].size);
    loft_copy_claims(cx, T, tp, src, dst);
#ifdef LOFT_COPY_CHECK
    loft_report_copy_mismatches(cx, T, tp, src, dst, "OpCopyRecord");
#endif
    if (free_source && src.store_nr != dst.store_nr && !loft_free_protected(cx, src)) loft_free_ref(cx, src);
}

/* A program's record copy (`OpCopyRecord`): into a null place it is dropped and reported, after
 * the two cases that copy nothing at all — the place itself and a null source — as
 * `State::do_copy_record` orders them. */
static void loft_copy_record_at(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst, int flags,
                                const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(dst.store_nr == 0xFFFF) && src.store_nr != 0xFFFF)
        loft_write_dropped(cx, dst, file, line);
    loft_copy_record(cx, T, tp, src, dst, flags);
}


/* @FR-R-FlatCopy (`hoist::flat_copy`) — a copy of a record type that owns no heap and keeps its
 * source is its bytes: the release of the destination's claims and the walk of the source's
 * claims both find nothing for such a type.  `size` is the type's width, a constant at the
 * site.  A null end and a parse result's root record (where `loft_perrs_deliver` acts) take the
 * general copy, which answers them. */
LOFT_HOT_INLINE void loft_copy_flat(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst,
                                    int flags, uint32_t size, const char *file, uint32_t line) {
    int general = src.store_nr == 0xFFFF || dst.store_nr == 0xFFFF || src.rec == 0 || dst.rec == 0;
#ifdef LOFT_PARSE
    general = general || (dst.rec == 1 && dst.pos == 8);
#endif
    if (LOFT_UNLIKELY(general)) {
        loft_copy_record_at(cx, T, tp, src, dst, flags, file, line);
        return;
    }
    loft_store_t *ds = loft_store_of(cx, dst);
    if (LOFT_WRITE_REFUSED(ds)) return;
    memmove(loft_at(cx, ds, dst.rec, dst.pos), loft_at(cx, loft_store_of(cx, src), src.rec, src.pos), size);
}

#ifdef LOFT_FRAMES
/* stack_trace (codegen_runtime::n_stack_trace): a StackFrame per live frame, outermost first; a
 * frame's `line` is the site its caller recorded, the entry frame's 0.  Like `--native`'s, a
 * frame carries no arguments or variables. */
static loft_ref loft_stack_trace(loft_cx *cx) {
    loft_ref vec = loft_ref_null(cx);
    loft_database(cx, &vec, LOFT_SF_SIZE * 8 - 8);
    loft_set_u32(cx, loft_store_of(cx, vec), vec.rec, vec.pos, 0);
    for (size_t i = 0; i < cx->nframes; i++) {
        loft_ref e = loft_vec_append(cx, vec, LOFT_SF_SIZE);
        loft_set_text(cx, e, LOFT_SF_FUNCTION, loft_lit(cx, cx->frames[i].fn, strlen(cx->frames[i].fn)));
        loft_set_text(cx, e, LOFT_SF_FILE, loft_lit(cx, cx->frames[i].file, strlen(cx->frames[i].file)));
#ifdef LOFT_STD_INT4
        loft_set_int4(cx, e, LOFT_SF_LINE, i == 0 ? 0 : (int64_t) cx->frames[i - 1].site);
#else
        loft_set_int(cx, e, LOFT_SF_LINE, i == 0 ? 0 : (int64_t) cx->frames[i - 1].site);
#endif
        loft_set_u32(cx, loft_store_of(cx, e), e.rec, e.pos + LOFT_SF_ARGS, 0);
        loft_set_u32(cx, loft_store_of(cx, e), e.rec, e.pos + LOFT_SF_VARS, 0);
        loft_vec_finish(cx, vec);
    }
    return vec;
}
#endif

/* OpInsertVector (vector::insert_vector): room for one element of `size` at `index` (negative
 * from the end), the elements from there on shifted up; the null reference for an index outside
 * 0..=len.  The caller writes the element's defaults (`insert_vector_element`). */
static loft_ref loft_vec_insert(loft_cx *cx, loft_ref v, uint32_t size, int64_t index) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return loft_ref_null(cx);
    int64_t len = loft_vec_len(cx, v);
    int64_t real = index < 0 ? index + len : index;
    if (real < 0 || real > len) return loft_ref_null(cx);
    loft_store_t *s = loft_store_of(cx, v);
    /* Refused whole at entry (`@FR-H-WriteLocked-Whole`): with room left no resize runs, so
     * nothing below would meet the lock before the shift. */
    if (LOFT_WRITE_REFUSED(s)) return loft_ref_null(cx);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    uint32_t nl;
    if (r == 0) {
        r = loft_claim(cx, s, loft_vec_cap_words(cx, loft_vec_grown(cx, 1, size), size));
        loft_set_u32(cx, s, v.rec, v.pos, r);
        nl = 1;
    } else {
        nl = (uint32_t) len + 1;
        /* The growth step only when the room runs out (`@FR-Col-Capacity`). */
        int32_t claimed;
        memcpy(&claimed, loft_at(cx, s, r, 0), 4);
        uint32_t nr = nl > ((uint32_t) claimed * 8 - 8) / size
            ? loft_st_resize(cx, s, r, loft_vec_cap_words(cx, loft_vec_grown(cx, nl, size), size)) : r;
        if (nr != r) {
            loft_set_u32(cx, s, v.rec, v.pos, nr);
            r = nr;
        }
        if (len > real)
            memmove(loft_at(cx, s, r, 8 + (uint32_t) (real + 1) * size), loft_at(cx, s, r, 8 + (uint32_t) real * size),
                    (size_t) (len - real) * size);
    }
    loft_set_u32(cx, s, r, 4, nl);
    loft_ref e = { v.store_nr, r, 8 + (uint32_t) real * size, 0 };
    return e;
}

/* OpReserveVector (vector::reserve_vector): room for exactly `count` elements, the record moved
 * if it has to grow; a negative count reserves nothing. */
static void loft_vec_reserve(loft_cx *cx, loft_ref v, int64_t count, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    if (count < 0 || count > (int64_t) UINT32_MAX) return;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t words = loft_vec_cap_words(cx, (uint32_t) count, size);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) {
        /* Not zeroed (`@FR-H-Claim`: a payload holds whatever the block held): an element past
         * the length is read by nothing, and every push writes its slot before the length
         * takes it in.  The length word is the one byte range a reader reads. */
        r = loft_st_claim(cx, s, words);
        loft_set_u32(cx, s, v.rec, v.pos, r);
        loft_set_u32(cx, s, r, 4, 0);
        return;
    }
    uint32_t nr = loft_st_resize(cx, s, r, words);
    if (nr != r) loft_set_u32(cx, s, v.rec, v.pos, nr);
}

/* `@FR-R-PushFill`'s window clause (vector::PushWindow, Stores::push_windowed) — the hot state
 * of a counted push loop: element 0's address, the length the loop has reached and the room in
 * elements.  A push that fits is one comparison, one store and a bump; the record's own length
 * is written when the window grows or closes, since nothing the loop runs reads it in between
 * (the emitter's proof, `hoist::push_window_ok`).  Passed and answered BY VALUE across the cold
 * growth arm, so a loop keeps the three in registers.  `rec` stands between `len` and `cap` on
 * purpose: adjacent, GCC's SLP pass carried the pair in one vector register and shuffled it out
 * at every push — 1.6 ns a push against 0.3 (bench 16 `f32_build`, -O2 and -O3 alike). */
typedef struct { uint8_t *base; uint32_t len; uint32_t rec; uint32_t cap; uint16_t store_nr; } loft_win;

/* Open a window over `v`'s vector of `size`-byte elements.  An absent vector, and a locked
 * store (`@FR-H-WriteLocked`), have no room: every push takes the growth arm, the ordinary
 * append, which refuses or drops it exactly as before. */
static loft_win loft_win_of(loft_cx *cx, loft_ref v, uint32_t size) {
    loft_win w = { 0, 0, 0, 0, 0xFFFF };
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return w;
    loft_store_t *s = loft_store_of(cx, v);
    w.store_nr = v.store_nr;
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return w;
    w.rec = r;
    w.len = loft_u32(cx, s, r, 4);
    int32_t claimed;
    memcpy(&claimed, loft_at(cx, s, r, 0), 4);
    if (!s->read_only && claimed > 0 && size != 0) w.cap = loft_rec_elems(cx, (uint32_t) claimed, size);
    w.base = loft_at(cx, s, r, 8);
    return w;
}

/* The record's length is the window's: written at the growth arm and at the close.  A window
 * with no room never moved its length, and a locked store is not written. */
static void loft_win_sync(loft_cx *cx, loft_win w) {
    if (w.rec == 0 || w.cap == 0) return;
    memcpy(loft_at(cx, &cx->stores[w.store_nr], w.rec, 4), &w.len, 4);
}

/* vector::reserve_more: room for `extra` MORE elements than the vector holds; nothing when the
 * block already has it, or for a non-positive or unrepresentable count. */
static void loft_vec_reserve_more(loft_cx *cx, loft_ref v, int64_t extra, uint32_t size) {
    if (extra <= 0 || extra == LOFT_NULL || v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    int64_t len = r == 0 ? 0 : (int64_t) loft_u32(cx, s, r, 4);
    if (extra > (int64_t) UINT32_MAX - len) return;
    uint32_t count = (uint32_t) (len + extra);
    if (r != 0 && !s->read_only) {
        int32_t claimed;
        memcpy(&claimed, loft_at(cx, s, r, 0), 4);
        if (claimed >= (int32_t) loft_vec_cap_words(cx, count, size)) return;
    }
    loft_vec_reserve(cx, v, (int64_t) count, size);
}

/* `@FR-R-PushFill`'s run clause: room for `extra` more elements before a straight-line run of
 * pushes — claimed for an empty vector, as its first push would.  A null reference and a locked
 * store are left alone — the pushes answer them as they would without it — so the reservation
 * is capacity and nothing else. */
static void loft_vec_reserve_run(loft_cx *cx, loft_ref v, int64_t extra, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0 || loft_store_of(cx, v)->read_only) return;
    loft_vec_reserve_more(cx, v, extra, size);
}

/* The growth arm, out of line: the length synced, the element appended the ordinary way (which
 * grows, refuses a locked store, drops a push into an absent vector), the window re-derived. */
LOFT_COLD static loft_win loft_win_grow(loft_cx *cx, loft_win w, loft_ref v, uint32_t size, uint64_t bits) {
    loft_win_sync(cx, w);
    loft_ref e = loft_vec_append(cx, v, size);
    if (e.rec != 0) {
        uint8_t *at = loft_at(cx, loft_store_of(cx, e), e.rec, e.pos);
        if (size == 8) memcpy(at, &bits, 8);
        else if (size == 4) { uint32_t b = (uint32_t) bits; memcpy(at, &b, 4); }
        else { uint8_t b = (uint8_t) bits; *at = b; }
        loft_vec_finish(cx, v);
    }
    return loft_win_of(cx, v, size);
}

/* The slow path of a push in place: the window grown through a COPY, so the caller's window never
 * has its address handed to a call — Clang then keeps its fields in registers across the loop,
 * where `*w = loft_win_grow(*w, …)` (a 24-byte struct passed and returned through memory) kept
 * the whole window on the stack and reloaded it at every push. */
LOFT_HOT_INLINE void loft_win_grow_into(loft_cx *cx, loft_win *w, loft_ref v, uint32_t size, uint64_t bits) {
    loft_win t = *w;
    t = loft_win_grow(cx, t, v, size, bits);
    *w = t;
}

/* `@FR-Col-Capacity` through a push window: `OpPreAllocVector`'s count, reserved exactly as
 * `loft_vec_prealloc` reserves it — which acts only while the vector has no record — and the
 * window derived again over the record it claimed.  A window over a record takes one test; the
 * reservation is out of line, through a copy, for the reason `loft_win_grow_into` gives. */
LOFT_COLD static loft_win loft_win_prealloced(loft_cx *cx, loft_ref v, uint32_t count, uint32_t size) {
    loft_vec_prealloc(cx, v, count, size);
    return loft_win_of(cx, v, size);
}

LOFT_HOT_INLINE void loft_win_prealloc(loft_cx *cx, loft_win *w, loft_ref v, uint32_t count, uint32_t size) {
    if (LOFT_LIKELY(w->rec != 0)) return;
    loft_win t = loft_win_prealloced(cx, v, count, size);
    *w = t;
}

/* A push through the window: one comparison, one store and a bump while it fits (the element's
 * bytes as the ordinary push writes them, encoded by the caller), the growth arm otherwise. */
LOFT_HOT_INLINE void loft_win_put8(loft_cx *cx, loft_win *w, loft_ref v, uint64_t bits) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        memcpy(w->base + (size_t) w->len * 8, &bits, 8);
        w->len++;
    } else {
        loft_win_grow_into(cx, w, v, 8, bits);
    }
}

LOFT_HOT_INLINE void loft_win_put4(loft_cx *cx, loft_win *w, loft_ref v, uint32_t bits) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        memcpy(w->base + (size_t) w->len * 4, &bits, 4);
        w->len++;
    } else {
        loft_win_grow_into(cx, w, v, 4, bits);
    }
}

LOFT_HOT_INLINE void loft_win_put1(loft_cx *cx, loft_win *w, loft_ref v, uint8_t bits) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        w->base[w->len] = bits;
        w->len++;
    } else {
        loft_win_grow_into(cx, w, v, 1, bits);
    }
}

/* `@FR-R-PushFill`'s record clause (Stores::push_record_windowed): the window's next slot for a
 * record mint, the window grown first when it is full — the length synced and one element's
 * room reserved the runtime's way.  The element stays invisible until the group's finish bumps
 * the window (`loft_win_finish`).  A vector the window cannot serve (absent, or in a locked store)
 * mints the ordinary way, so it drops or refuses exactly as before. */
LOFT_COLD static loft_win loft_win_room(loft_cx *cx, loft_win w, loft_ref v, uint32_t size) {
    loft_win_sync(cx, w);
    loft_vec_reserve_more(cx, v, 1, size);
    return loft_win_of(cx, v, size);
}

LOFT_HOT_INLINE loft_ref loft_win_slot(loft_cx *cx, loft_win *w, loft_ref v, uint32_t size) {
    if (LOFT_UNLIKELY(w->len >= w->cap)) {
        *w = loft_win_room(cx, *w, v, size);
        if (w->len >= w->cap) return loft_vec_append(cx, v, size);
    }
    loft_ref e = {0};
    e.store_nr = w->store_nr;
    e.rec = w->rec;
    e.pos = 8 + w->len * size;
    return e;
}

/* The group's finish: the minted element counts.  One the window could not serve was minted
 * and is finished the ordinary way, and the window re-derived from the record it changed. */
LOFT_HOT_INLINE void loft_win_finish(loft_cx *cx, loft_win *w, loft_ref v, uint32_t size) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        w->len++;
    } else {
        loft_vec_finish(cx, v);
        *w = loft_win_of(cx, v, size);
    }
}

/* `@FR-R-PushRec` (native's record push header): a minted path's record, length and room, with
 * no pointer — a growth of a sibling vector in the same store moves the store's memory, never
 * this record — so several paths of one store each keep their own across a loop.  A mint takes
 * the slot past the length while it fits; one that does not grows the record the runtime's way
 * (`loft_vec_append`: claimed, doubled, refused when locked, dropped when absent) and the header
 * is derived again.  The finish writes the length back, so the store answers every other read
 * and no exit has anything to sync.  A locked store has no room, so its mints refuse as before. */
/* `base` is the record's address when it was derived.  Only a loop whose one growth is this
 * header's own pushes writes through it (`@FR-R-Push`'s sole-growth clause, `loft_ph_putb*`):
 * nothing else in such a loop moves the store's memory, and the growth step derives the whole
 * header again. */
typedef struct { loft_ref v; uint32_t rec; uint32_t len; uint32_t cap; uint8_t *base; } loft_ph;

LOFT_HOT_INLINE loft_ph loft_ph_of(loft_cx *cx, loft_ref v, uint32_t size) {
    loft_ph h;
    h.v = v;
    h.rec = 0;
    h.len = 0;
    h.cap = 0;
    h.base = 0;
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return h;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return h;
    h.rec = r;
    h.base = loft_at(cx, s, r, 0);
    h.len = loft_u32(cx, s, r, 4);
    if (s->read_only) return h;
    int32_t claimed;
    memcpy(&claimed, loft_at(cx, s, r, 0), 4);
    h.cap = claimed < 1 ? 0 : loft_rec_elems(cx, (uint32_t) claimed, size);
    return h;
}

/* `@FR-R-Refresh`: the growth re-derives the whole header at its own site. */
LOFT_COLD static loft_ref loft_ph_grow(loft_cx *cx, loft_ph *h, uint32_t size) {
    loft_ref e = loft_vec_append(cx, h->v, size);
    *h = loft_ph_of(cx, h->v, size);
    return e;
}

LOFT_HOT_INLINE loft_ref loft_ph_new(loft_cx *cx, loft_ph *h, uint32_t size) {
    if (LOFT_UNLIKELY(h->len >= h->cap)) return loft_ph_grow(cx, h, size);
    loft_ref e = {0};
    e.store_nr = h->v.store_nr;
    e.rec = h->rec;
    e.pos = 8 + h->len * size;
    return e;
}

LOFT_HOT_INLINE void loft_ph_finish(loft_cx *cx, loft_ph *h) {
    if (LOFT_UNLIKELY(h->rec == 0)) {
        loft_vec_finish(cx, h->v);
        return;
    }
    h->len++;
    memcpy(loft_at(cx, &cx->stores[h->v.store_nr], h->rec, 4), &h->len, 4);
}

/* `@FR-R-Push`: a scalar push through a held push header — the slot past the length while it
 * fits (one bounds test), the element's encoded bytes, the length bump written to the header and
 * the record; a growth is the runtime's own append and a fresh header (`loft_ph_grow`).  An
 * absent or refused slot writes nothing, as the plain push. */
#define LOFT_PH_PUT(W, T)                                                                   \
    LOFT_HOT_INLINE void loft_ph_put##W(loft_cx *cx, loft_ph *h, T bits) {                               \
        loft_ref e = loft_ph_new(cx, h, W);                                                     \
        if (LOFT_UNLIKELY(e.rec == 0)) return;                                              \
        memcpy(loft_at(cx, &cx->stores[e.store_nr], e.rec, e.pos), &bits, W);                  \
        loft_ph_finish(cx, h);                                                                  \
    }
LOFT_PH_PUT(8, uint64_t)
LOFT_PH_PUT(4, uint32_t)
LOFT_PH_PUT(1, uint8_t)

/* `@FR-R-Push`'s sole-growth clause: the same push through the record address the header
 * holds — a push that fits stores the element and the length at that address, without
 * resolving the store; one that does not takes `loft_ph_put*`'s growth step, which derives
 * the header (and its address) again.  A header with no room (absent, refused, locked) never
 * reads the address. */
#define LOFT_PH_PUTB(W, T)                                                                  \
    LOFT_HOT_INLINE void loft_ph_putb##W(loft_cx *cx, loft_ph *h, T bits) {                              \
        if (LOFT_LIKELY(h->len < h->cap)) {                                                 \
            memcpy(h->base + 8 + (size_t) h->len * W, &bits, W);                            \
            h->len++;                                                                       \
            memcpy(h->base + 4, &h->len, 4);                                                \
            return;                                                                         \
        }                                                                                   \
        loft_ph_put##W(cx, h, bits);                                                            \
    }
LOFT_PH_PUTB(8, uint64_t)
LOFT_PH_PUTB(4, uint32_t)
LOFT_PH_PUTB(1, uint8_t)

/* `@FR-R-PushFill`'s run clause through its header: the writer tests the header's room for the
 * run's `k` elements inline, and a header without it takes this — the reservation
 * (`loft_vec_reserve_run`, capacity only) and the header derived again.  One with room — a
 * vector cleared and refilled, a reused record buffer's field — takes no call. */
LOFT_HOT_INLINE loft_ph loft_ph_reserved(loft_cx *cx, loft_ph h, int64_t k, uint32_t size) {
    loft_vec_reserve_run(cx, h.v, k, size);
    return loft_ph_of(cx, h.v, size);
}

/* `@FR-Col-Capacity` through a held push header: `OpPreAllocVector`'s known count, reserved
 * exactly as `loft_vec_prealloc` reserves it — which acts only while the vector has no record —
 * and the header derived again so its mints fill that record.  A header already holding the
 * record takes one test and nothing else. */
LOFT_HOT_INLINE void loft_ph_prealloc(loft_cx *cx, loft_ph *h, uint32_t count, uint32_t size) {
    if (LOFT_LIKELY(h->rec != 0)) return;
    loft_vec_prealloc(cx, h->v, count, size);
    *h = loft_ph_of(cx, h->v, size);
}

/* A loop's push header read as a held vector header: its record and length are current — every
 * growth and cut of the path inside the loop goes through it (`loft_ph_keep` below) — and the
 * element base is derived from the record, as `loft_vh_fresh` derives it, since a growth may
 * have moved the store's memory.  The `loft_vh` accessors then read an element exactly as they
 * read a held one: a negative index from the end, a missing element null. */
LOFT_HOT_INLINE loft_vh loft_ph_vh(loft_cx *cx, const loft_ph *h) {
    loft_vh r;
    r.store_nr = h->v.store_nr;
    r.rec = h->rec;
    r.len = h->len;
    r.base = h->rec != 0 ? loft_elem_base(cx, &cx->stores[h->v.store_nr], h->rec) : 0;
    r.locked = h->rec != 0 && cx->stores[h->v.store_nr].read_only;
    return r;
}

/* The plain element reads through a push header — `loft_vh_get_int` and `loft_vh_get_float` on
 * `loft_ph_vh`, without building the view: a negative index from the end, null (NaN for a float)
 * outside the vector. */
LOFT_HOT_INLINE int64_t loft_ph_get_int(loft_cx *cx, const loft_ph *h, uint32_t size, loft_int from, uint32_t fld) {
    int64_t f = from < 0 ? from + (int64_t) h->len : from;
    if (LOFT_UNLIKELY((uint64_t) f >= h->len)) return LOFT_NULL;
    int64_t v;
    memcpy(&v, loft_at(cx, &cx->stores[h->v.store_nr], h->rec, 8 + (uint32_t) f * size + fld), 8);
    return v;
}
LOFT_HOT_INLINE double loft_ph_get_float(loft_cx *cx, const loft_ph *h, uint32_t size, loft_int from, uint32_t fld) {
    int64_t f = from < 0 ? from + (int64_t) h->len : from;
    if (LOFT_UNLIKELY((uint64_t) f >= h->len)) return (double) NAN;
    double v;
    memcpy(&v, loft_at(cx, &cx->stores[h->v.store_nr], h->rec, 8 + (uint32_t) f * size + fld), 8);
    return v;
}

/* `@FR-R-Refresh`'s keep-range clause through a push header: `v = v[lo..hi]` with `lo` 0 — the
 * truncate — cuts the length at the header and the record, clamped as `loft_vec_keep_range`
 * clamps it; any other range moves its span there and derives the header again. */
static void loft_vec_keep_range(loft_cx *cx, loft_ref v, int64_t lo, int64_t hi, uint32_t size);
LOFT_HOT_INLINE void loft_ph_keep(loft_cx *cx, loft_ph *h, int64_t lo, int64_t hi, uint32_t size) {
#ifndef LOFT_FOREIGN
    if (lo == 0 && h->rec != 0) {
        if (hi < 0) hi = 0;
        if ((uint64_t) hi < h->len) {
            h->len = (uint32_t) hi;
            memcpy(loft_at(cx, &cx->stores[h->v.store_nr], h->rec, 4), &h->len, 4);
        }
        return;
    }
#endif
    loft_vec_keep_range(cx, h->v, lo, hi, size);
    *h = loft_ph_of(cx, h->v, size);
}

/* `@FR-R-PushFill`'s held-run clause: a run of `k` pushes through a header a loop already holds.
 * When all `k` elements fit (`len + k <= cap`, so the record is present and its store unlocked)
 * they are stored past the length at one record address and the length is bumped once, in the
 * header and the record; else the run pushes one call at a time, each taking its own growth
 * step, exactly as without the clause.  `_runb` is the sole-growth form: the address the header
 * holds. */
#define LOFT_PH_RUN(W, T, B, AT)                                                            \
    LOFT_HOT_INLINE void loft_ph_put##B##W##_run(loft_cx *cx, loft_ph *h, uint32_t k, const T *bits) { \
        if (LOFT_LIKELY(h->cap >= k && h->len <= h->cap - k)) {                             \
            uint8_t *r = (AT);                                                              \
            uint8_t *s = r + 8 + (size_t) h->len * W;                                       \
            for (uint32_t i = 0; i < k; i++) memcpy(s + (size_t) i * W, &bits[i], W);       \
            h->len += k;                                                                    \
            memcpy(r + 4, &h->len, 4);                                                      \
            return;                                                                         \
        }                                                                                   \
        for (uint32_t i = 0; i < k; i++) loft_ph_put##B##W(cx, h, bits[i]);                 \
    }
LOFT_PH_RUN(8, uint64_t, , loft_at(cx, &cx->stores[h->v.store_nr], h->rec, 0))
LOFT_PH_RUN(4, uint32_t, , loft_at(cx, &cx->stores[h->v.store_nr], h->rec, 0))
LOFT_PH_RUN(1, uint8_t, , loft_at(cx, &cx->stores[h->v.store_nr], h->rec, 0))
LOFT_PH_RUN(8, uint64_t, b, h->base)
LOFT_PH_RUN(4, uint32_t, b, h->base)
LOFT_PH_RUN(1, uint8_t, b, h->base)

/* `LOFT_VERIFY`'s proof for a push header: a fresh derivation answers its record, length and room. */
static int loft_ph_same(loft_cx *cx, loft_ph h, uint32_t size) {
    loft_ph f = loft_ph_of(cx, h.v, size);
    return f.rec == h.rec && f.len == h.len && f.cap == h.cap;
}

/* `LOFT_VERIFY`'s proof for a push header that writes through its address: the address too is
 * what a fresh derivation answers — nothing moved the store since the header was derived. */
static int loft_phb_same(loft_cx *cx, loft_ph h, uint32_t size) {
    loft_ph f = loft_ph_of(cx, h.v, size);
    return loft_ph_same(cx, h, size) && f.base == h.base;
}

/* `LOFT_VERIFY`'s proof for a windowed push: the window still describes the vector — its
 * record, element base and room are what a fresh derivation answers. */
static int loft_win_same(loft_cx *cx, loft_win w, loft_ref v, uint32_t size) {
    loft_win f = loft_win_of(cx, v, size);
    return f.rec == w.rec && f.base == w.base && f.cap == w.cap && f.store_nr == w.store_nr;
}

LOFT_HOT_INLINE uint64_t loft_f64_bits(loft_cx *cx, double x) { uint64_t b; memcpy(&b, &x, 8); return b; }
LOFT_HOT_INLINE uint32_t loft_f32_bits(loft_cx *cx, float x) { uint32_t b; memcpy(&b, &x, 4); return b; }
LOFT_HOT_INLINE uint32_t loft_i4_bits(loft_cx *cx, int64_t x) { return x == LOFT_NULL ? 0x80000000u : (uint32_t) (uint64_t) x; }

static void loft_vec_set_size(loft_cx *cx, loft_ref v, uint32_t adding, uint32_t size);

/* OpKeepRange (Stores::vector_keep_range): `v = v[lo..hi]` of scalar elements, kept in place —
 * the span moved to the front of its record and the length set.  A foreign view takes the copy
 * form: the span copied out, the view dropped, the span appended (`append_span`). */
static void loft_vec_keep_range(loft_cx *cx, loft_ref v, int64_t lo, int64_t hi, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    int64_t n = loft_vec_len(cx, v);
    lo = lo < 0 ? 0 : lo > n ? n : lo;
    hi = hi < lo ? lo : hi > n ? n : hi;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return;
    size_t bytes = (size_t) (hi - lo) * size;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(cx, s)) {
        uint8_t *span = (uint8_t *) malloc(bytes ? bytes : 1);
        if (!span) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(span, loft_at(cx, s, r, 8 + (uint32_t) lo * size), bytes);
        loft_vec_clear(cx, v);
        if (hi > lo) {
            loft_ref e = loft_vec_append(cx, v, size);
            loft_vec_set_size(cx, v, (uint32_t) (hi - lo), size);
            loft_store_t *d = loft_store_of(cx, v);
            memcpy(loft_at(cx, d, loft_collection_rec(cx, d, v.rec, v.pos), e.pos), span, bytes);
        }
        free(span);
        return;
    }
#endif
    if (bytes) memmove(loft_at(cx, s, r, 8), loft_at(cx, s, r, 8 + (uint32_t) lo * size), bytes);
    loft_set_u32(cx, s, r, 4, (uint32_t) (hi - lo));
}

/* OpMoveField (Stores::move_field_out): a field's value moved into `dst` — within one store its
 * bytes relocate; across two the destination takes a deep copy and the source releases what it
 * held.  Either way the source is left zeroed. */
static void loft_move_field(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0
        || src.store_nr >= cx->nstores || dst.store_nr >= cx->nstores) return;
    uint32_t size = T[tp].size;
    loft_copy_bytes(cx, dst, 0, src, 0, size);
    if (src.store_nr != dst.store_nr) {
        loft_copy_claims(cx, T, tp, src, dst);
        loft_remove_claims(cx, T, tp, src);
    }
    if (size) memset(loft_at(cx, loft_store_of(cx, src), src.rec, src.pos), 0, size);
}

/* OpRemoveVector (vector::remove_vector): the element at `index` (negative from the end) goes,
 * its successors move down; false for a null vector or an index outside it. */
static int loft_vec_remove(loft_cx *cx, loft_ref v, uint32_t size, int64_t index) {
    if (v.store_nr == 0xFFFF) return 0;
    if (v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(cx, v))) return 0;
    int64_t len = loft_vec_len(cx, v);
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (index == LOFT_NULL) return 0;
    int64_t i = index < 0 ? index + len : index;
    if (i >= len || i < 0 || r == 0) return 0;
    if (len - i > 1) {
        memmove(loft_at(cx, s, r, 8 + (uint32_t) i * size), loft_at(cx, s, r, 8 + (uint32_t) (i + 1) * size),
                (size_t) (len - i - 1) * size);
    }
    loft_set_u32(cx, s, r, 4, (uint32_t) (len - 1));
    return 1;
}

/* OpReplaceVector (Stores::vector_replace): `v = o` for a vector — an absent source leaves
 * the destination absent, the same vector is left alone, anything else is cleared and
 * refilled with deep copies of the source's elements.  The clear RELEASES what the old
 * elements own (`@FR-H-ClearRelease`): the destination may be a live vector. */
static void loft_vec_add(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o);
static void loft_vec_clear_release_body(loft_cx *cx, const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v);

static void loft_vec_replace(loft_cx *cx, const loft_tdesc *T, uint32_t nt, uint16_t tp, loft_ref v, loft_ref o) {
#ifdef LOFT_PARSE
    loft_perrs_deliver_vector(cx, o, v, 1);
#endif
    int absent = o.store_nr == 0xFFFF
        || (o.rec != 0 && o.pos != 0 && loft_u32(cx, loft_store_of(cx, o), o.rec, o.pos) == 0xFFFFFFFFu);
    if (absent) {
        loft_vec_clear_release_body(cx, T, nt, v);
        if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0) loft_set_u32(cx, loft_store_of(cx, v), v.rec, v.pos, 0xFFFFFFFFu);
        return;
    }
    if (v.store_nr == o.store_nr) {
        uint32_t dr = loft_u32(cx, loft_store_of(cx, v), v.rec, v.pos);
        if (dr != 0 && dr == loft_u32(cx, loft_store_of(cx, o), o.rec, o.pos)) return;
    }
    loft_vec_clear_release_body(cx, T, nt, v);
    loft_vec_add(cx, T, tp, v, o);
}

/* OpAppendCopy (State::append_copy): `[x; n]` — the template is the vector's last element and
 * `n` counts it, so n - 1 deep copies of it follow; n <= 0 removes the template too.  As
 * Stores::fill_from_template: one append and one resize to exactly the total (the record the
 * interpreter's layout has), the template copied in doubling blocks — each copy takes as many
 * elements as are already there — and the claims copied per element only for a type that
 * holds any. */
static void loft_vec_append_copy(loft_cx *cx, const loft_tdesc *T, uint16_t ctp, loft_ref v, int64_t count) {
    uint32_t multiply = count <= 0 ? 0 : count > (int64_t) UINT32_MAX ? UINT32_MAX : (uint32_t) count;
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (multiply == 0) {
        if (r != 0) {
            uint32_t len = loft_u32(cx, s, r, 4);
            loft_set_u32(cx, s, r, 4, len == 0 ? 0 : len - 1);
        }
        return;
    }
    uint32_t length = (uint32_t) loft_vec_len(cx, v);
    uint32_t size = T[ctp].size;
    uint32_t extra = multiply - 1;
    if (extra == 0 || length == 0) return;
    (void) loft_vec_append(cx, v, size);
    loft_vec_set_size(cx, v, extra, size);
    s = loft_store_of(cx, v);
    r = loft_collection_rec(cx, s, v.rec, v.pos);
    uint32_t from = 8 + (length - 1) * size;
    uint8_t *base = (uint8_t *) loft_at(cx, s, r, 0);
    /* A scalar template of a machine width is stored into every slot: the doubling reads back
     * each block it wrote to copy it, a store loop only writes (a 1920x1080 canvas of integers:
     * the doubling's memmove was 93 % of building it); `loft_fill8` and its siblings write it. */
    if (T[ctp].kind == LOFT_T_SCALAR && (size == 8 || size == 4 || size == 2 || size == 1)) {
        uint8_t *dst = base + from + size;
        if (size == 1) {
            memset(dst, base[from], (size_t) extra);
        } else if (size == 8) {
            uint64_t t;
            memcpy(&t, base + from, 8);
            loft_fill8(dst, t, extra);
        } else if (size == 4) {
            uint32_t t;
            memcpy(&t, base + from, 4);
            loft_fill4(dst, t, extra);
        } else {
            uint16_t t;
            memcpy(&t, base + from, 2);
            loft_fill2(dst, t, extra);
        }
        return;
    }
    for (uint32_t done = 1; done < multiply;) {
        uint32_t n = done < multiply - done ? done : multiply - done;
        memcpy(base + from + (size_t) done * size, base + from, (size_t) n * size);
        done += n;
    }
    if (T[ctp].kind != LOFT_T_SCALAR && T[ctp].kind != LOFT_T_DBREF) {
        loft_ref a = { v.store_nr, r, from, 0 };
        for (uint32_t i = 1; i <= extra; i++) {
            loft_ref e = { v.store_nr, r, from + i * size, 0 };
            loft_copy_claims(cx, T, ctp, a, e);
        }
    }
}

/* OpClear (Stores::remove_claims): the record releases what it owns — every text, vector and
 * child record slot it holds, through its struct fields and enum variant, reads empty after. */
/* Stores::remove_claims (`remove_claims_mode`, `remove_struct_claims` and the owned walk): every
 * claim the value of type `tp` at `r` owns is deleted, children before the record that holds
 * them, and each collection field is left empty — the order the interpreter deletes in, which
 * decides where later claims land.  A `borrowed` collection — a view member of a linked group —
 * releases its own storage and none of the records it names. */
static void loft_remove_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r);

static void loft_remove_claims_mode(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed) {
    cx->child_depth++;
    loft_remove_claims_one(cx, T, tp, r, borrowed);
    cx->child_depth--;
    if (cx->child_depth == 0) loft_child_drain(cx, T);
}

static void loft_remove_claims_one(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    const loft_tdesc *d = &T[tp];
    /* `Stores::release_file_handle`: a `File` record that dies returns its lease. */
    if (d->file == 2 && !borrowed && loft_file_release_hook) loft_file_release_hook(cx, r);
    /* `@FR-R-FlatCopy`: a record that owns no heap holds no claim to release. */
    if (d->kind == LOFT_T_STRUCT && !d->owns_heap) return;
    loft_store_t *s = loft_store_of(cx, r);
    if (d->kind == LOFT_T_TEXT) {
        uint32_t cur = loft_u32(cx, s, r.rec, r.pos);
        if (cur != 0) {
            loft_st_delete(cx, s, cur);
            loft_set_u32(cx, s, r.rec, r.pos, 0);
        }
        return;
    }
    if (d->kind == LOFT_T_VECTOR || d->kind == LOFT_T_CHILD || d->kind == LOFT_T_HASH || d->kind == LOFT_T_INDEX ||
        d->kind == LOFT_T_ORDERED || d->kind == LOFT_T_RADIX || d->kind == LOFT_T_TRIE) {
        /* An absent collection slot owns nothing and reads as empty afterwards. */
        if (loft_u32(cx, s, r.rec, r.pos) == 0xFFFFFFFFu) { loft_set_u32(cx, s, r.rec, r.pos, 0); return; }
    }
    if (!borrowed && !d->owns_heap) return;
    switch (d->kind) {
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) {
            if (!d->fields[i].view && !T[d->fields[i].type].owns_heap) continue;
            loft_remove_claims_mode(cx, T, d->fields[i].type, loft_field(cx, r, d->fields[i].pos), d->fields[i].view);
        }
        return;
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(cx, s, r.rec, r.pos);
        if (cur != 0) {
            uint32_t len = loft_u32(cx, s, cur, 4), size = T[d->elem].size;
            if (T[d->elem].owns_heap) {
                for (uint32_t i = 0; i < len; i++) {
                    loft_ref e = { r.store_nr, cur, 8 + i * size, 0 };
                    loft_remove_claims(cx, T, d->elem, e);
                }
            }
            loft_st_delete(cx, loft_store_of(cx, r), cur);
        }
        loft_set_u32(cx, loft_store_of(cx, r), r.rec, r.pos, 0);
        return;
    }
    case LOFT_T_CHILD: {
        uint32_t cur = loft_u32(cx, s, r.rec, r.pos);
        loft_set_u32(cx, loft_store_of(cx, r), r.rec, r.pos, 0);
        if (cur != 0) {
            loft_ref c = { r.store_nr, cur, 8, 0 };
            loft_child_push(cx, d->elem, 1, c, c);
        }
        return;
    }
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(cx, s, r.rec, r.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) loft_remove_claims(cx, T, d->variants[e], r);
        return;
    }
    case LOFT_T_HASH:
        loft_hash_remove_claims(cx, T, tp, r, borrowed);
        return;
    case LOFT_T_INDEX:
        loft_ix_remove_claims(cx, T, tp, r, borrowed);
        return;
    case LOFT_T_RADIX:
    case LOFT_T_TRIE:
        loft_rx_remove_claims(cx, T, tp, r, borrowed);
        return;
    case LOFT_T_ORDERED: {
        /* Every record the slot list names (not a view's), then the list. */
        uint32_t cur = loft_u32(cx, s, r.rec, r.pos);
        if (cur != 0) {
            if (!borrowed) {
                for (uint32_t i = 0; i < loft_u32(cx, s, cur, 4); i++) {
                    uint32_t elm = loft_u32(cx, s, cur, 8 + 4 * i);
                    if (elm == 0) continue;
                    loft_ref e = { r.store_nr, elm, 8, 0 };
                    loft_remove_claims(cx, T, d->elem, e);
                    loft_st_delete(cx, s, elm);
                }
            }
            loft_st_delete(cx, s, cur);
        }
        loft_set_u32(cx, s, r.rec, r.pos, 0);
        return;
    }
    default:
        return;
    }
}

static void loft_remove_claims(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r) { loft_remove_claims_mode(cx, T, tp, r, 0); }

/* vector::reached_capacity: the elements `v`'s claim holds room for, 0 when it has none. */
static uint32_t loft_vec_reached(loft_cx *cx, loft_ref v, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return 0;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return 0;
    int32_t words = loft_st_i32(cx, s, r, 0);
    return words <= 0 ? 0 : loft_rec_elems(cx, (uint32_t) words, size);
}

/* OpClearVector (Stores::clear_vector_release): `v` emptied, releasing what its elements own.
 * A vector that is the whole content of its store's root record (a one-field root type) whose
 * elements own heap resets the store instead — made again with the root alone, the vector
 * pre-sized to the room it had reached — rather than walking every element. */
static void loft_vec_clear_release_body(loft_cx *cx, const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v);

/* `v = []`: the common case — a vector already empty, which has nothing to release and whose
 * clear is nothing — answered inline; everything else out of line. */
LOFT_HOT_INLINE void loft_vec_clear_release(loft_cx *cx, const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
#ifndef LOFT_FOREIGN
    {
        loft_store_t *s = loft_store_of(cx, v);
        uint32_t cur = loft_u32(cx, s, v.rec, v.pos);
        if (cur == 0) return;
        if (cur != 0xFFFFFFFFu && loft_u32(cx, s, cur, 4) == 0) return;
    }
#endif
    loft_vec_clear_release_body(cx, T, loft_ntypes, v);
}

static void loft_vec_clear_release_body(loft_cx *cx, const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    loft_store_t *s = loft_store_of(cx, v);
#ifdef LOFT_FOREIGN
    /* A local bound to a view: the clear drops it, and the store is an ordinary empty one. */
    if (loft_st_is_foreign(cx, s)) { loft_st_release_foreign(cx, s); return; }
#endif
    /* A vector already empty — no block, or a block of length 0 — has nothing to release, and its
     * clear is nothing (`v = []` on a vector that holds nothing).  An absent slot still takes the
     * clear, which makes it the empty vector. */
    uint32_t cur = loft_u32(cx, s, v.rec, v.pos);
    if (cur == 0) return;
    if (cur != 0xFFFFFFFFu) {
        uint32_t r0 = loft_collection_rec(cx, s, v.rec, v.pos);
        if (r0 != 0 && loft_u32(cx, s, r0, 4) == 0) return;
    }
    if (v.rec == 1) {
        uint16_t kt = LOFT_KNOWN(*s);
        int one_field = kt < loft_ntypes && T[kt].kind == LOFT_T_STRUCT && T[kt].n == 1;
        uint16_t vec_tp = 0xFFFF;
        if (one_field) vec_tp = T[kt].fields[0].type;
        else if (kt < loft_ntypes && T[kt].kind == LOFT_T_STRUCT) {
            for (uint16_t i = 0; i < T[kt].n; i++)
                if (T[kt].fields[i].pos + 8u == v.pos) { vec_tp = T[kt].fields[i].type; break; }
        }
        uint16_t elem = vec_tp < loft_ntypes && T[vec_tp].kind == LOFT_T_VECTOR ? T[vec_tp].elem : 0xFFFF;
        if (elem != 0xFFFF && T[elem].owns_heap) {
            uint32_t size = T[elem].size;
            if (one_field && s->nplaced == 0) {
                uint32_t reached = loft_vec_reached(cx, v, size);
                loft_st_init(cx, s);
                uint32_t root = loft_st_claim(cx, s, 1 + (T[kt].size + 7u) / 8);
                memset(s->buf + (size_t) root * 8 + 4, 0, (size_t) loft_st_i32(cx, s, root, 0) * 8 - 4);
                loft_set_u32(cx, s, root, 4, kt);
                loft_vec_prealloc(cx, v, reached, size);
                return;
            }
            uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
            if (r != 0) {
                uint32_t len = loft_u32(cx, s, r, 4);
                for (uint32_t i = 0; i < len; i++) {
                    loft_ref e = { v.store_nr, r, 8 + i * size, 0 };
                    loft_remove_claims(cx, T, elem, e);
                }
            }
        }
    }
    loft_vec_clear(cx, v);
}

/* `@FR-H-KeepRoom`: OpDatabase of a vector wrapper where the variable already names its own
 * store's root wrapper — a buffer reused across calls (R-Reuse) — makes the store again exactly as
 * `loft_database_wrapper` does (everything it held goes) and re-establishes the wrapper's vector
 * at length zero with the room it had reached.  A capacity is no value.  Any other case is the
 * plain mint.  `-DLOFT_NO_KEEP_ROOM` keeps the plain mint everywhere. */
static void loft_database_room(loft_cx *cx, const loft_tdesc *T, uint32_t nt, loft_ref *var, uint32_t bytes, uint16_t tp) {
#ifndef LOFT_NO_KEEP_ROOM
    if (var->store_nr != 0xFFFF && var->rec == 1 && var->store_nr < cx->nstores
        && cx->stores[var->store_nr].live && !cx->stores[var->store_nr].frame_holds
        && LOFT_KNOWN(cx->stores[var->store_nr]) == tp && tp < nt && T[tp].kind == LOFT_T_STRUCT
        && T[tp].n == 1) {
        uint16_t vt = T[tp].fields[0].type;
        uint16_t elem = vt < nt && T[vt].kind == LOFT_T_VECTOR ? T[vt].elem : 0xFFFF;
        uint32_t size = elem < nt ? T[elem].size : 0;
        if (size != 0) {
            uint32_t fpos = T[tp].fields[0].pos;
            uint32_t reached = loft_vec_reached(cx, loft_field(cx, *var, fpos), size);
            loft_database_wrapper(cx, T, var, bytes, tp);
            if (reached != 0) loft_vec_prealloc(cx, loft_field(cx, *var, fpos), reached, size);
            return;
        }
    }
#else
    (void) nt;
#endif
    loft_database_wrapper(cx, T, var, bytes, tp);
}

/* OpAppendVector (Stores::vector_add): every element of `o` appended to `v`, deep-copied so the
 * destination owns what each element holds.  The source's length is read once, and each source
 * element is re-read after the growth, so a self-append (`v += v`) reads the moved record. */
/* Stores::vector_set_size: the vector grows by `adding` elements of `size`, its claim resized to
 * hold exactly them when more than one arrives. */
static void loft_vec_set_size(loft_cx *cx, loft_ref v, uint32_t adding, uint32_t size) {
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_u32(cx, s, v.rec, v.pos);
    uint32_t len = loft_u32(cx, s, r, 4);
    if ((uint64_t) len + adding > UINT32_MAX) {
        fprintf(stderr, "loft: Vector capacity overflow: count=%u+%u size=%u\n", len, adding, size);
        abort();
    }
    if (adding > 1) {
        uint32_t nr = loft_st_resize(cx, s, r, loft_vec_cap_words(cx, len + adding, size));
        if (nr != r) {
            loft_set_u32(cx, s, v.rec, v.pos, nr);
            r = nr;
        }
    }
    loft_set_u32(cx, s, r, 4, len + adding);
}

/* Stores::vector_add (OpAppendVector): the elements of `o` appended to `v` as deep copies.  When
 * they own no heap and fit the room `v` has, one block copy (`vector_add_fits`); into an empty
 * `v`, a claim of exactly their size; else `vector_append` and a resize to the total. */
static void loft_vec_add(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
#ifdef LOFT_PARSE
    loft_perrs_deliver_vector(cx, o, v, 0);
#endif
    int64_t n64 = loft_vec_len(cx, o);
    if (n64 == 0 || v.store_nr == 0xFFFF || v.rec == 0) return;
    uint32_t n = (uint32_t) n64;
    uint32_t size = T[tp].size;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t o_rec = loft_u32(cx, loft_store_of(cx, o), o.rec, o.pos);
    uint32_t d_rec = loft_collection_rec(cx, s, v.rec, v.pos);
    int same_vec = v.store_nr == o.store_nr && o_rec != 0 && loft_u32(cx, s, v.rec, v.pos) == o_rec;
    /* A foreign destination has no claimed header to read (`append_block_fits` refuses it); it is
     * locked, so the append below refuses the write. */
    if (size != 0 && !T[tp].owns_heap && d_rec != 0 && d_rec != LOFT_FOREIGN_REC
        && !(v.store_nr == o.store_nr && d_rec == o_rec)) {
        int32_t claimed = loft_st_i32(cx, s, d_rec, 0);
        uint32_t len = loft_u32(cx, s, d_rec, 4);
        uint32_t cap = claimed > 0 ? loft_rec_elems(cx, (uint32_t) claimed, size) : 0;
        if (claimed > 0 && (uint64_t) len + n <= cap) {
            loft_ref dst = { v.store_nr, d_rec, 0, 0 }, src = { o.store_nr, o_rec, 0, 0 };
            loft_copy_bytes(cx, dst, 8 + len * size, src, 8, (size_t) n * size);
            loft_set_u32(cx, s, d_rec, 4, len + n);
            return;
        }
    }
    if (!same_vec && loft_u32(cx, s, v.rec, v.pos) == 0) {
        uint32_t rec = loft_claim(cx, s, (uint32_t) (((uint64_t) n * size + 15) / 8));
        loft_set_u32(cx, s, v.rec, v.pos, rec);
        loft_set_u32(cx, s, rec, 4, 0);
    }
    loft_ref slot = loft_vec_append(cx, v, size);
    uint32_t append_pos = slot.pos;
    loft_vec_set_size(cx, v, n, size);
    uint32_t dest = loft_u32(cx, s, v.rec, v.pos);
    if (v.store_nr == o.store_nr) o_rec = loft_u32(cx, loft_store_of(cx, o), o.rec, o.pos);
    loft_ref dst = { v.store_nr, dest, 0, 0 }, src = { o.store_nr, o_rec, 0, 0 };
    loft_copy_bytes(cx, dst, append_pos, src, 8, (size_t) n * size);
    if (!T[tp].owns_heap) return;
    for (uint32_t i = 0; i < n; i++) {
        loft_ref a = { o.store_nr, o_rec, 8 + i * size, 0 };
        loft_ref b = { v.store_nr, dest, append_pos + i * size, 0 };
        loft_copy_claims(cx, T, tp, a, b);
    }
}

/* OpPrependVector (Stores::prepend_vector): `g`'s elements added as OpAppendVector adds them,
 * then rotated to the front.  0, having done nothing, for a null or absent vector or a linked
 * element type — the rewrite keeps the written-out statements as the fallback. */
static uint8_t loft_vec_prepend(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref g, int linked) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0 || linked) return 0;
    if (loft_u32(cx, loft_store_of(cx, v), v.rec, v.pos) == 0xFFFFFFFFu) return 0;
    int64_t before = loft_vec_len(cx, v);
    loft_vec_add(cx, T, tp, v, g);
    int64_t after = loft_vec_len(cx, v);
    if (before == 0 || after <= before) return 1;
    size_t size = T[tp].size, moved = (size_t) (after - before) * size, total = (size_t) after * size;
    uint8_t *e = loft_at(cx, loft_store_of(cx, v), loft_collection_rec(cx, loft_store_of(cx, v), v.rec, v.pos), 8);
    uint8_t *tmp = (uint8_t *) malloc(moved ? moved : 1);
    if (!tmp) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(tmp, e + total - moved, moved);
    memmove(e + moved, e, total - moved);
    memcpy(e, tmp, moved);
    free(tmp);
    return 1;
}

/* OpAdoptVector (Stores::vector_adopt): a local takes a native call's vector answer as its own —
 * a VIEW of the whole when the answer is a foreign store, the element copy otherwise. */
static void loft_vec_adopt(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
#ifdef LOFT_PARSE
    loft_perrs_deliver_vector(cx, o, v, 1);
#endif
#if defined(LOFT_FOREIGN) && !defined(LOFT_NO_FOREIGN_VIEW)
    if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0 && o.store_nr != 0xFFFF && o.rec != 0 && o.pos != 0
        && v.store_nr != o.store_nr) {
        int64_t n = loft_vec_len(cx, o);
        if (n > 0 && loft_st_is_foreign(cx, loft_store_of(cx, o))) {
            loft_fspan span = loft_st_foreign_span(cx, loft_store_of(cx, o), 0, (uint32_t) n);
            loft_store_t *s = loft_store_of(cx, v);
            loft_st_release_foreign(cx, s);
            /* As the slice view: a locked destination is copied into, and refuses (D19g). */
            if (!s->read_only) {
                loft_st_make_foreign(cx, s, v.rec, v.pos, span);
                return;
            }
        }
    }
#endif
    loft_vec_add(cx, T, tp, v, o);
}

/* OpMoveVector (Stores::move_vector, @FR-R-MoveLast): a local vector given up to a field.  In
 * one store, into an empty slot, the vector's record moves and the source slot empties; else
 * the elements are appended as deep copies. */
static void loft_move_vector(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref dst, loft_ref src) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    uint32_t h = loft_collection_rec(cx, loft_store_of(cx, src), src.rec, src.pos);
    if (h == 0) return;
    if (src.store_nr == dst.store_nr && loft_collection_rec(cx, loft_store_of(cx, dst), dst.rec, dst.pos) == 0) {
        loft_set_u32(cx, loft_store_of(cx, dst), dst.rec, dst.pos, h);
        loft_set_u32(cx, loft_store_of(cx, src), src.rec, src.pos, 0);
        return;
    }
    loft_vec_add(cx, T, tp, dst, src);
}

/* OpReverseVector (vector::reverse_vector): the elements in the opposite order. */
static void loft_vec_reverse(loft_cx *cx, loft_ref v, uint32_t size) {
    int64_t n = loft_vec_len(cx, v);
    if (n < 2) return;
    loft_store_t *s = loft_store_of(cx, v);
    if (LOFT_WRITE_REFUSED(s)) return;
    uint8_t *base = loft_at(cx, s, loft_collection_rec(cx, s, v.rec, v.pos), 8);
    /* An element of any width, swapped through a 64-byte window a piece at a time. */
    uint8_t tmp[64];
    for (int64_t lo = 0, hi = n - 1; lo < hi; lo++, hi--) {
        uint8_t *x = base + lo * size, *y = base + hi * size;
        for (uint32_t o = 0; o < size; o += (uint32_t) sizeof tmp) {
            uint32_t k = size - o < (uint32_t) sizeof tmp ? size - o : (uint32_t) sizeof tmp;
            memcpy(tmp, x + o, k);
            memcpy(x + o, y + o, k);
            memcpy(y + o, tmp, k);
        }
    }
}

static int loft_cmp_u8(const void *a, const void *b) { return (int) *(const uint8_t *) a - (int) *(const uint8_t *) b; }
static int loft_cmp_u16(const void *a, const void *b) {
    uint16_t x, y;
    memcpy(&x, a, 2);
    memcpy(&y, b, 2);
    return (x > y) - (x < y);
}
static int loft_cmp_i32(const void *a, const void *b) {
    int32_t x, y;
    memcpy(&x, a, 4);
    memcpy(&y, b, 4);
    return (x > y) - (x < y);
}
static int loft_cmp_u32(const void *a, const void *b) {
    uint32_t x, y;
    memcpy(&x, a, 4);
    memcpy(&y, b, 4);
    return (x > y) - (x < y);
}
static int loft_cmp_i64(const void *a, const void *b) {
    int64_t x, y;
    memcpy(&x, a, 8);
    memcpy(&y, b, 8);
    return (x > y) - (x < y);
}
/* `@FR-E-NullArg`: a float's null (a NaN) orders below every value, and the rest by value, so
 * `-0.0` and `0.0` compare equal.  The sort is STABLE (`sort`'s promise), so equal values — the
 * two zeros — keep their order.  It moves the elements' BIT PATTERNS, never a float, so a null
 * keeps its exact pattern; a float is decoded only to compare.  Below 16 elements an insertion
 * sort with no buffer (a small vector sorted in a loop allocates nothing); above, runs of 16 by
 * insertion and bottom-up merges through one scratch buffer.  The comparison is `<`'s own
 * (`loft_flt`), inlined where `qsort` called one through a pointer. */
static int loft_flt(loft_cx *cx, double a, double b);
#define LOFT_FLOAT_SORT(NAME, BITS, FLT)                                                         \
    static inline int NAME##_lt(loft_cx *cx, const void *ctx, BITS a, BITS b) {                  \
        FLT x, y;                                                                                \
        (void) ctx;                                                                              \
        memcpy(&x, &a, sizeof x);                                                                \
        memcpy(&y, &b, sizeof y);                                                                \
        return loft_flt(cx, (double) x, (double) y); /* `<` itself, one home */                       \
    }                                                                                            \
    LOFT_STABLE_SORT(NAME, BITS)
/* The stable sort itself, over `NAME##_lt(ctx, a, b)`: what a comparison needs (a text's
 * store) is handed in as `ctx`, never left where the comparison could find it. */
#define LOFT_STABLE_SORT(NAME, BITS)                                                             \
    static void NAME##_ins(loft_cx *cx, const void *ctx, BITS *a, size_t n) {                                                  \
        for (size_t i = 1; i < n; i++) {                                                         \
            BITS v = a[i];                                                                       \
            size_t j = i;                                                                        \
            while (j > 0 && NAME##_lt(cx, ctx, v, a[j - 1])) {                                            \
                a[j] = a[j - 1];                                                                 \
                j--;                                                                             \
            }                                                                                    \
            a[j] = v;                                                                            \
        }                                                                                        \
    }                                                                                            \
    static void NAME(loft_cx *cx, const void *ctx, uint8_t *base, size_t n) {                                                  \
        BITS small[16];                                                                          \
        BITS *a = n <= 16 ? small : (BITS *) malloc(2 * n * sizeof(BITS));                       \
        if (!a) { fputs("loft: out of memory\n", stderr); abort(); }                            \
        memcpy(a, base, n * sizeof(BITS));                                                       \
        if (n <= 16) {                                                                           \
            NAME##_ins(cx, ctx, a, n);                                                                    \
        } else {                                                                                 \
            for (size_t lo = 0; lo < n; lo += 16) NAME##_ins(cx, ctx, a + lo, n - lo < 16 ? n - lo : 16); \
            BITS *src = a, *dst = a + n;                                                         \
            for (size_t w = 16; w < n; w *= 2) {                                                 \
                for (size_t lo = 0; lo < n; lo += 2 * w) {                                       \
                    size_t mid = lo + w < n ? lo + w : n, hi = lo + 2 * w < n ? lo + 2 * w : n;  \
                    size_t i = lo, j = mid, k = lo;                                              \
                    while (i < mid && j < hi) dst[k++] = NAME##_lt(cx, ctx, src[j], src[i]) ? src[j++] : src[i++]; \
                    while (i < mid) dst[k++] = src[i++];                                         \
                    while (j < hi) dst[k++] = src[j++];                                          \
                }                                                                                \
                BITS *t = src;                                                                   \
                src = dst;                                                                       \
                dst = t;                                                                         \
            }                                                                                    \
            if (src != a) memcpy(a, src, n * sizeof(BITS));                                      \
        }                                                                                        \
        memcpy(base, a, n * sizeof(BITS));                                                       \
        if (a != small) free(a);                                                                 \
    }
LOFT_FLOAT_SORT(loft_sort_f32, uint32_t, float)
LOFT_FLOAT_SORT(loft_sort_f64, uint64_t, double)

/* A text-vector sort's order: `@FR-E-NullArg`'s `<` — a null text before every text, `""`
 * included (loft#2236) — over the texts' records in the store handed in as `ctx`. */
static inline int loft_sort_text_lt(loft_cx *cx, const void *ctx, uint32_t x, uint32_t y) {
    loft_store_t *s = (loft_store_t *) ctx;
    return loft_text_lt(cx, loft_store_str(cx, s, x), loft_store_str(cx, s, y));
}
LOFT_STABLE_SORT(loft_sort_text, uint32_t)

/* OpSortVector (vector::sort_vector, sort_text_vector): ascending, by the element's width and
 * kind — a text and a float in the order of `<` (null first, `@FR-E-NullArg`), a float stably
 * (`loft_sort_f32`/`_f64`), a byte or short by its stored code, a 4-byte integer signed or, in
 * the unsigned encoding (`Stores::sorts_unsigned`), unsigned. */
static void loft_vec_sort(loft_cx *cx, loft_ref v, uint32_t size, int is_float, int is_text, int is_unsigned) {
    int64_t n = loft_vec_len(cx, v);
    if (n < 2) return;
    loft_store_t *s = loft_store_of(cx, v);
    if (LOFT_WRITE_REFUSED(s)) return;
    void *base = loft_at(cx, s, loft_collection_rec(cx, s, v.rec, v.pos), 8);
    int (*cmp)(const void *, const void *) = 0;
    if (is_text) { loft_sort_text(cx, s, (uint8_t *) base, (size_t) n); return; }
    else if (size == 1) cmp = loft_cmp_u8;
    else if (size == 2) cmp = loft_cmp_u16;
    else if (size == 4 && is_float) { loft_sort_f32(cx, 0, (uint8_t *) base, (size_t) n); return; }
    else if (size == 8 && is_float) { loft_sort_f64(cx, 0, (uint8_t *) base, (size_t) n); return; }
    else if (size == 4) cmp = is_unsigned ? loft_cmp_u32 : loft_cmp_i32;
    else if (size == 8) cmp = loft_cmp_i64;
    if (cmp) qsort(base, (size_t) n, size, cmp);
}

/* OpAppendTextBytes (Stores::append_text_bytes): the bytes [lo, hi) of `t` appended to a
 * `vector<u8>` (the caller proved 0 <= lo <= hi <= size(t)).  Room for all of them reserved once,
 * then one copy and one length: the byte-copy rewrite (`byte_copy.rs`) turns a loft loop into this
 * call, so it must not be that loop again.  A text read out of the vector's own store moves with
 * the buffer when the reservation grows it, so it is found again by its offset.  Where the room
 * is not there after the reservation — a locked store, an absent vector — each byte takes the
 * ordinary append, which answers those cases as before. */
/* `@FR-Col-Capacity`'s growth step for a run of `needed` elements of `size` bytes: room claimed
 * for an absent vector, or the record grown, to `loft_vec_grown(needed)` — only when the room
 * runs out, as `vector::append_bytes` grows (a run of bytes is no known count to reserve
 * exactly).  A locked store is left alone. */
static void loft_vec_grow_for(loft_cx *cx, loft_ref v, uint32_t needed, uint32_t size) {
    loft_store_t *s = loft_store_of(cx, v);
    if (s->read_only) return;
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) {
        r = loft_claim(cx, s, loft_vec_cap_words(cx, loft_vec_grown(cx, needed, size), size));
        loft_set_u32(cx, s, v.rec, v.pos, r);
        loft_set_u32(cx, s, r, 4, 0);
        return;
    }
    int32_t claimed;
    memcpy(&claimed, loft_at(cx, s, r, 0), 4);
    if (claimed <= 0 || needed <= ((uint32_t) claimed * 8 - 8) / size) return;
    uint32_t nr = loft_st_resize(cx, s, r, loft_vec_cap_words(cx, loft_vec_grown(cx, needed, size), size));
    if (nr != r) loft_set_u32(cx, s, v.rec, v.pos, nr);
}

static void loft_append_text_bytes(loft_cx *cx, loft_ref v, loft_str t, int64_t lo, int64_t hi) {
    if (hi > (int64_t) t.len) hi = (int64_t) t.len;
    if (lo >= 0 && hi > lo && v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0) {
        uint32_t n = (uint32_t) (hi - lo);
        loft_store_t *s = loft_store_of(cx, v);
        const uint8_t *src = (const uint8_t *) t.p + lo;
        int inside = src >= s->buf && src < s->buf + (size_t) s->size * 8;
        size_t off = inside ? (size_t) (src - s->buf) : 0;
        uint32_t had = loft_collection_rec(cx, s, v.rec, v.pos);
        uint64_t needed = (uint64_t) (had == 0 ? 0 : loft_u32(cx, s, had, 4)) + n;
        if (needed > UINT32_MAX) return;
        loft_vec_grow_for(cx, v, (uint32_t) needed, 1);
        s = loft_store_of(cx, v);
        uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
        if (r != 0 && !s->read_only) {
            int32_t claimed;
            memcpy(&claimed, loft_at(cx, s, r, 0), 4);
            uint32_t len = loft_u32(cx, s, r, 4);
            if (claimed > 0 && (uint64_t) len + n + 8 <= (uint64_t) claimed * 8) {
                if (inside) src = s->buf + off;
                memmove(loft_at(cx, s, r, 8 + len), src, n);
                loft_set_u32(cx, s, r, 4, len + n);
                return;
            }
        }
    }
    for (int64_t i = lo; i < hi && i < (int64_t) t.len; i++) {
        loft_ref e = loft_vec_append(cx, v, 1);
        if (e.rec == 0) return;
        *loft_at(cx, loft_store_of(cx, e), e.rec, e.pos) = (uint8_t) t.p[i];
        loft_vec_finish(cx, v);
    }
}

/* `@FR-R-AppendTwin`'s window clause: `n` elements of `size` bytes at `src` appended through
 * window `w` over vector `v` — one copy while they fit.  Out of room, the length is synced, `v`
 * grown the ordinary way to at least twice its length and the window derived again.  A source
 * inside v's own store would move with that growth, and a vector the window cannot serve (absent,
 * locked) has no room at all: both take the ordinary append, the window synced around it. */
static void loft_win_grow_copy(loft_cx *cx, loft_win *w, loft_ref v, const uint8_t *src, uint32_t n, uint32_t size) {
    loft_win_sync(cx, *w);
    loft_store_t *s = v.store_nr == 0xFFFF ? 0 : loft_store_of(cx, v);
    int inside = s && src >= s->buf && src < s->buf + (size_t) s->size * 8;
    if (!inside && w->cap != 0) {
        uint32_t extra = n > w->len ? n : w->len;
        loft_vec_reserve_more(cx, v, (int64_t) extra, size);
        *w = loft_win_of(cx, v, size);
        if ((uint64_t) w->len + n <= w->cap) {
            memcpy(w->base + (size_t) w->len * size, src, (size_t) n * size);
            w->len += n;
            return;
        }
        loft_win_sync(cx, *w);
    }
    if (size == 1) {
        loft_str t = { (const char *) src, n };
        loft_append_text_bytes(cx, v, t, 0, n);
    } else {
        for (uint32_t i = 0; i < n; i++) {
            loft_ref e = loft_vec_append(cx, v, size);
            if (e.rec == 0) break;
            memcpy(loft_at(cx, loft_store_of(cx, e), e.rec, e.pos), src + (size_t) i * size, size);
            loft_vec_finish(cx, v);
        }
    }
    *w = loft_win_of(cx, v, size);
}

LOFT_HOT_INLINE void loft_win_copy(loft_cx *cx, loft_win *w, loft_ref v, const uint8_t *src, uint32_t n, uint32_t size) {
    if (LOFT_LIKELY((uint64_t) w->len + n <= w->cap)) {
        memcpy(w->base + (size_t) w->len * size, src, (size_t) n * size);
        w->len += n;
    } else {
        loft_win_grow_copy(cx, w, v, src, n, size);
    }
}

/* OpAppendTextBytes through a window: the bytes [lo, hi) of `t`, as `loft_append_text_bytes`. */
LOFT_HOT_INLINE void loft_win_text(loft_cx *cx, loft_win *w, loft_ref v, loft_str t, int64_t lo, int64_t hi) {
    if (hi > (int64_t) t.len) hi = (int64_t) t.len;
    if (lo < 0 || hi <= lo) return;
    loft_win_copy(cx, w, v, (const uint8_t *) t.p + lo, (uint32_t) (hi - lo), 1);
}

/* OpSliceVector through a window: the elements [lo, hi) of `o`, clamped to it as `loft_vec_slice`
 * clamps, `size` bytes each. */
static void loft_win_slice(loft_cx *cx, loft_win *w, loft_ref v, loft_ref o, int64_t lo, int64_t hi, uint32_t size) {
    if (o.store_nr == 0xFFFF || o.rec == 0 || o.pos == 0 || size == 0) return;
    int64_t n64 = loft_vec_len(cx, o);
    if (lo < 0) lo = 0;
    if (lo > n64) lo = n64;
    if (hi < lo) hi = lo;
    if (hi > n64) hi = n64;
    if (hi <= lo) return;
    loft_store_t *s = loft_store_of(cx, o);
    uint32_t r = loft_collection_rec(cx, s, o.rec, o.pos);
    if (r == 0) return;
    loft_win_copy(cx, w, v, loft_elem_base(cx, s, r) + (size_t) lo * size, (uint32_t) (hi - lo), size);
}

/* OpAppendVector of a scalar vector through a window: every element of `o`, `size` bytes each. */
static void loft_win_vec(loft_cx *cx, loft_win *w, loft_ref v, loft_ref o, uint32_t size) {
    int64_t n = loft_vec_len(cx, o);
    if (n <= 0 || o.store_nr == 0xFFFF || o.rec == 0 || o.pos == 0) return;
    loft_store_t *s = loft_store_of(cx, o);
    uint32_t r = loft_collection_rec(cx, s, o.rec, o.pos);
    if (r == 0) return;
    loft_win_copy(cx, w, v, loft_elem_base(cx, s, r), (uint32_t) n, size);
}

/* OpKeepVectorRange (Stores::keep_vector_range): only the elements [lo, hi) stay, moved to the
 * front; a range outside the vector leaves it as it was.  What each dropped element owns is
 * released first, front and back (a linked element's record removed with it), before anything
 * moves.  `size` 0 is a linked element: its slot is a record number. */
static void loft_keep_vector_range(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, uint32_t size, int64_t lo, int64_t hi) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || lo < 0 || hi < lo || lo == LOFT_NULL || hi == LOFT_NULL) return;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0 || hi > (int64_t) loft_u32(cx, s, r, 4)) return;
    uint32_t len = loft_u32(cx, s, r, 4);
    if (size == 0) {
        for (uint32_t i = 0; i < len; i++) {
            if ((int64_t) i >= lo && (int64_t) i < hi) continue;
            uint32_t rec = loft_u32(cx, s, r, 8 + i * 4);
            if (rec == 0) continue;
            loft_ref elem = { v.store_nr, rec, 8, 0 };
            loft_remove_claims(cx, T, tp, elem);
            loft_remove_record(cx, loft_store_of(cx, v), rec);
        }
        size = 4;
        s = loft_store_of(cx, v);
        r = loft_collection_rec(cx, s, v.rec, v.pos);
    } else if (T[tp].owns_heap) {
        for (uint32_t i = 0; i < len; i++) {
            if ((int64_t) i >= lo && (int64_t) i < hi) continue;
            loft_ref elem = { v.store_nr, r, 8 + i * size, 0 };
            loft_remove_claims(cx, T, tp, elem);
        }
        s = loft_store_of(cx, v);
        r = loft_collection_rec(cx, s, v.rec, v.pos);
    }
    uint32_t kept = (uint32_t) (hi - lo);
    if (lo > 0 && kept > 0) memmove(loft_at(cx, s, r, 8), loft_at(cx, s, r, 8 + (uint32_t) lo * size), (size_t) kept * size);
    loft_set_u32(cx, s, r, 4, kept);
}

/* A whole-value bind `dst = src` (@C86): an independent copy of a present source, the null of
 * an absent one, releasing what `dst` held.  The copy takes a fresh store when the source
 * lives in `dst`'s: refilling that store in place would destroy what the source owns. */
static void loft_bind_copy(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref *dst, loft_ref src) {
    if (src.store_nr == 0xFFFF || src.rec == 0) {
        loft_free_ref(cx, *dst);
        *dst = loft_ref_null(cx);
        return;
    }
    if (src.store_nr == dst->store_nr && src.rec == dst->rec && src.pos == dst->pos) return;
    if (src.store_nr == dst->store_nr) *dst = loft_ref_null(cx);
    loft_database_typed(cx, dst, T[tp].size, tp); /* typed, as `OpDatabase(tp)`: the next rebind may exchange */
#ifdef LOFT_PARSE
    loft_perrs_deliver(cx, src, *dst, tp, 1);
#endif
    loft_copy_bytes(cx, *dst, 0, src, 0, T[tp].size);
    loft_copy_claims(cx, T, tp, src, *dst);
#ifdef LOFT_COPY_CHECK
    loft_report_copy_mismatches(cx, T, tp, src, *dst, "bind_copy");
#endif
}

/* `@FR-R-DeferredCopy`'s remap: `y` — a view into the graph rooted at `from` — moved to the same
 * position in `to`, the copy `loft_bind_copy` made of it, walking both in step as
 * `loft_copy_claims` built `to`: the root's inline bytes, each struct field, each vector's record
 * and its elements, a struct-enum's variant.  1 with `*out` set when `y` lies in the graph. */
static int loft_dc_find(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to, loft_ref y, loft_ref *out) {
    const loft_tdesc *d = &T[tp];
    if (y.rec == from.rec && y.pos >= from.pos && y.pos < from.pos + d->size) {
        out->store_nr = to.store_nr;
        out->rec = to.rec;
        out->pos = to.pos + (y.pos - from.pos);
        return 1;
    }
    loft_store_t *fs = loft_store_of(cx, from);
    switch (d->kind) {
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) {
            const loft_fld *f = &d->fields[i];
            if (f->view) continue;
            if (loft_dc_find(cx, T, f->type, loft_field(cx, from, f->pos), loft_field(cx, to, f->pos), y, out)) return 1;
        }
        return 0;
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(cx, fs, from.rec, from.pos) - 1;
        return e >= 0 && e < d->n && d->variants[e] != 0xFFFF
            && loft_dc_find(cx, T, d->variants[e], from, to, y, out);
    }
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(cx, fs, from.rec, from.pos);
        if (cur == 0 || cur == 0xFFFFFFFFu) return 0;
        uint32_t into = loft_u32(cx, loft_store_of(cx, to), to.rec, to.pos);
        if (y.rec == cur) {
            out->store_nr = to.store_nr;
            out->rec = into;
            out->pos = y.pos;
            return 1;
        }
        uint16_t e = d->elem;
        if (T[e].kind == LOFT_T_SCALAR || T[e].kind == LOFT_T_DBREF || T[e].kind == LOFT_T_TEXT) return 0;
        uint32_t len = loft_u32(cx, fs, cur, 4), size = T[e].size;
        for (uint32_t i = 0; i < len; i++) {
            loft_ref a = { from.store_nr, cur, 8 + i * size, 0 };
            loft_ref b = { to.store_nr, into, 8 + i * size, 0 };
            if (loft_dc_find(cx, T, e, a, b, y, out)) return 1;
        }
        return 0;
    }
    default:
        return 0;
    }
}

static loft_ref loft_dc_remap(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to, loft_ref y) {
    loft_ref out = {0};
    if (y.store_nr != from.store_nr || y.rec == 0 || from.rec == 0) return y;
    return loft_dc_find(cx, T, tp, from, to, y, &out) ? out : y;
}

/* `@FR-R-DeferredCopy`'s index guard: is `x` the element `k` of the vector `v` names?  `k` is
 * resolved exactly as `loft_vec_get` resolves it (a negative index counts from the end), so the
 * test compares positions, never the raw integers; an index outside the vector names nothing. */
LOFT_HOT_INLINE int loft_dc_is_elem(loft_cx *cx, loft_ref x, loft_ref v, uint32_t size, int64_t k) {
    loft_ref e = loft_vec_get(cx, v, size, k);
    return e.rec != 0 && x.store_nr == e.store_nr && x.rec == e.rec && x.pos == e.pos;
}

/* ── Floats (src/ops.rs, the stdlib's #rust templates) ─────────────────────────────────────
 * float is a double, single a float; NaN is null.  A literal is written from its bits, so no
 * decimal round trip can change it. */
static double loft_f64(loft_cx *cx, uint64_t bits) { double d; memcpy(&d, &bits, 8); return d; }
static float loft_f32(loft_cx *cx, uint32_t bits) { float f; memcpy(&f, &bits, 4); return f; }

static int loft_feq(loft_cx *cx, double a, double b) { return (isnan(a) && isnan(b)) || (a <= b && b <= a); }
static int loft_flt(loft_cx *cx, double a, double b) { return !isnan(b) && !(a >= b); }
static int loft_fle(loft_cx *cx, double a, double b) { return isnan(a) || (!isnan(b) && a <= b); }
/* @PLN217 — the same of two singles, compared as singles: the answers are the double forms',
 * since a single widens to a double exactly, and no double is made on the way. */
static int loft_feq32(loft_cx *cx, float a, float b) { return (isnan(a) && isnan(b)) || (a <= b && b <= a); }
static int loft_flt32(loft_cx *cx, float a, float b) { return !isnan(b) && !(a >= b); }
static int loft_fle32(loft_cx *cx, float a, float b) { return isnan(a) || (!isnan(b) && a <= b); }

/* op_conv_float_from_long: null becomes NaN. */
static double loft_i2f(loft_cx *cx, int64_t v) { return v == LOFT_NULL ? (double) NAN : (double) v; }
/* `f.bits()` / `float_from_bits(v)` and the `single` pair: the IEEE-754 pattern moved as it
 * is, no conversion (`f64::to_bits` / `from_bits`); a single's 32 bits as a signed 32-bit
 * value. */
static int64_t loft_f2bits(loft_cx *cx, double f) { int64_t v; memcpy(&v, &f, 8); return v; }
static double loft_bits2f(loft_cx *cx, int64_t v) { double f; memcpy(&f, &v, 8); return f; }
static int64_t loft_s2bits(loft_cx *cx, float f) { int32_t v; memcpy(&v, &f, 4); return v; }
static float loft_bits2s(loft_cx *cx, int64_t v) { int32_t w = (int32_t) v; float f; memcpy(&f, &w, 4); return f; }
static float loft_i2s(loft_cx *cx, int64_t v) { return v == LOFT_NULL ? (float) NAN : (float) v; }
/* @PLN217 — the same for an integer held in 32 bits. */
static float loft_i2s32(loft_cx *cx, int32_t v) { return v == INT32_MIN ? (float) NAN : (float) v; }

/* OpCastIntFromFloat: NaN and a value outside i64 are null; else truncated toward zero. */
static int64_t loft_f2i(loft_cx *cx, double v) {
    if (isnan(v) || !(v >= -9223372036854775808.0 && v < 9223372036854775808.0)) return LOFT_NULL;
    return (int64_t) v;
}

static double loft_math1(loft_cx *cx, int id, double x) {
    switch (id) {
    case 0: return cos(x);
    case 1: return sin(x);
    case 2: return tan(x);
    case 3: return acos(x);
    case 4: return asin(x);
    case 5: return atan(x);
    case 6: return ceil(x);
    case 7: return floor(x);
    case 8: return round(x);
    case 9: return sqrt(x);
    case 10: return exp(x);
    default: return (double) NAN;
    }
}

static float loft_math1s(loft_cx *cx, int id, float x) {
    switch (id) {
    case 0: return cosf(x);
    case 1: return sinf(x);
    case 2: return tanf(x);
    case 3: return acosf(x);
    case 4: return asinf(x);
    case 5: return atanf(x);
    case 6: return ceilf(x);
    case 7: return floorf(x);
    case 8: return roundf(x);
    case 9: return sqrtf(x);
    case 10: return expf(x);
    default: return (float) NAN;
    }
}

/* id 0 is atan2; id 1 is the logarithm in base `b` (log10 and log2 exactly, else ln a / ln b). */
static double loft_math2(loft_cx *cx, int id, double a, double b) {
    if (id == 0) return atan2(a, b);
    if (id == 1) return b == 10.0 ? log10(a) : b == 2.0 ? log2(a) : log(a) / log(b);
    return (double) NAN;
}

static float loft_math2s(loft_cx *cx, int id, float a, float b) {
    if (id == 0) return atan2f(a, b);
    if (id == 1) return b == 10.0f ? log10f(a) : b == 2.0f ? log2f(a) : logf(a) / logf(b);
    return (float) NAN;
}

LOFT_HOT_INLINE double loft_get_float(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return (double) NAN;
    double v;
    memcpy(&v, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), 8);
    return v;
}

LOFT_HOT_INLINE void loft_set_float(loft_cx *cx, loft_ref r, uint32_t fld, double v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &v, 8);
}

static float loft_get_single(loft_cx *cx, loft_ref r, uint32_t fld) {
    if (r.rec == 0) return (float) NAN;
    float v;
    memcpy(&v, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), 4);
    return v;
}

static void loft_set_single(loft_cx *cx, loft_ref r, uint32_t fld, float v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, r))) return;
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos + fld), &v, 4);
}

/* The program's field writes (`OpSet*`): the setter, and a write through a null place reported
 * at the statement's file and line (`loft_write_dropped`), after the value is computed, as the
 * interpreter's setter reports.  The runtime's own writes call the setters, which say nothing.
 * Inlined, the site is two constants on the null branch alone. */
#define LOFT_SET_AT(name, T)                                                                   \
    LOFT_HOT_INLINE void name##_at(loft_cx *cx, loft_ref r, uint32_t fld, T v, const char *file, uint32_t line) { \
        if (LOFT_UNLIKELY(r.rec == 0)) { loft_write_dropped(cx, r, file, line); return; }          \
        name(cx, r, fld, v);                                                                       \
    }
#define LOFT_SET_MIN_AT(name)                                                                  \
    LOFT_HOT_INLINE void name##_at(loft_cx *cx, loft_ref r, uint32_t fld, int64_t min, int64_t v, \
                                   const char *file, uint32_t line) {                          \
        if (LOFT_UNLIKELY(r.rec == 0)) { loft_write_dropped(cx, r, file, line); return; }          \
        name(cx, r, fld, min, v);                                                                  \
    }
LOFT_SET_AT(loft_set_int, int64_t)
LOFT_SET_AT(loft_set_int4, int64_t)
LOFT_SET_AT(loft_set_int4_32, int32_t)
LOFT_SET_AT(loft_set_int4_raw, int64_t)
LOFT_SET_AT(loft_set_float, double)
LOFT_SET_AT(loft_set_single, float)
LOFT_SET_AT(loft_set_byte, uint8_t)
LOFT_SET_AT(loft_set_char, uint32_t)
LOFT_SET_AT(loft_set_dbref, loft_ref)
LOFT_SET_AT(loft_set_text, loft_str)
LOFT_SET_AT(loft_refill_text, loft_str)
LOFT_SET_MIN_AT(loft_set_byte_v)
LOFT_SET_MIN_AT(loft_set_byte_nullable)
LOFT_SET_MIN_AT(loft_set_short)
LOFT_SET_MIN_AT(loft_set_short_raw)
#undef LOFT_SET_AT
#undef LOFT_SET_MIN_AT

static void loft_vec_push_float(loft_cx *cx, loft_ref v, double x) {
    uint8_t *slot = loft_vec_push_slot(cx, v, 8);
    if (slot) {
        memcpy(slot, &x, 8);
        return;
    }
    loft_ref e = loft_vec_append(cx, v, 8);
    if (e.rec == 0) return;
    loft_set_float(cx, e, 0, x);
    loft_vec_finish(cx, v);
}

static void loft_vec_push_char(loft_cx *cx, loft_ref v, uint32_t c) {
    uint8_t *slot = loft_vec_push_slot(cx, v, 4);
    if (slot) {
        memcpy(slot, &c, 4);
        return;
    }
    loft_ref e = loft_vec_append(cx, v, 4);
    if (e.rec == 0) return;
    memcpy(loft_at(cx, loft_store_of(cx, e), e.rec, e.pos), &c, 4);
    loft_vec_finish(cx, v);
}

static void loft_vec_push_single(loft_cx *cx, loft_ref v, float x) {
    uint8_t *slot = loft_vec_push_slot(cx, v, 4);
    if (slot) {
        memcpy(slot, &x, 4);
        return;
    }
    loft_ref e = loft_vec_append(cx, v, 4);
    if (e.rec == 0) return;
    loft_set_single(cx, e, 0, x);
    loft_vec_finish(cx, v);
}

/* Rust's `{}` for a float: the shortest digits that read back as `v` (as a float when `single`),
 * written out in full with no exponent — 1e21 is "1000000000000000000000", 1.0 is "1", -0.0 is
 * "-0".  The digits are the first `%.*e` precision that round-trips, correctly rounded. */
/* `@FR-F-Render`'s shortest form (@C156): of the strings of the shortest length that read back as
 * `v`, the nearest, an exact tie to the even digit — `printf`'s own rounding, so its p-digit
 * candidate is the answer whenever it reads back.  When it does not, its neighbour on `v`'s
 * other side may (just below a power of two the half-gap towards zero is the narrower): `buf`
 * becomes that neighbour and 1 is answered when it reads back. */
static int loft_shortest_other(loft_cx *cx, char *buf, size_t cap, int p, double v, double near, int single) {
    const char *q = buf;
    int neg = *q == '-';
    if (neg) q++;
    char d[48];
    int nd = 0;
    for (; *q && *q != 'e'; q++) if (*q >= '0' && *q <= '9') d[nd++] = *q;
    if (nd != p || !*q) return 0;
    int e = atoi(q + 1);
    /* Away from `near` towards `v`: up in magnitude when `v` lies beyond it. */
    int up = (fabs(v) > fabs(near));
    int i = nd - 1;
    if (up) {
        while (i >= 0 && d[i] == '9') d[i--] = '0';
        if (i >= 0) d[i]++;
        else { d[0] = '1'; e++; }
    } else {
        while (i >= 0 && d[i] == '0') d[i--] = '9';
        if (i < 0) return 0;
        d[i]--;
        if (d[0] == '0') {
            memmove(d, d + 1, (size_t) nd - 1);
            d[nd - 1] = '0';
            e--;
        }
    }
    char txt[64];
    int t = 0;
    if (neg) txt[t++] = '-';
    txt[t++] = d[0];
    if (nd > 1) txt[t++] = '.';
    for (int k = 1; k < nd; k++) txt[t++] = d[k];
    snprintf(txt + t, sizeof txt - (size_t) t, "e%d", e);
    double back = strtod(txt, 0);
    if (single ? (float) back != (float) v : back != v) return 0;
    if (strlen(txt) + 1 > cap) return 0;
    memcpy(buf, txt, strlen(txt) + 1);
    return 1;
}

/* The shortest form of a `v` with at most 15 significant digits (6 for a single) and
 * 1e-6 <= |v| < 1e15, without printf: the integer m nearest |v| * 10^k for the least k at which
 * m / 10^k reads back as `v` — the division is correctly rounded, so it is what `strtod` answers
 * for those digits — m's neighbours tried too, since |v| * 10^k may itself be inexact.  At that
 * many digits the decimal grid is coarser than `v`'s spacing, so at most one grid point reads
 * back and the first k found is the shortest form `loft_push_shortest`'s search arrives at.
 * Answers 0, writing nothing, for every other `v`. */
static int loft_push_shortest_fast(loft_cx *cx, loft_text *s, double v, int single) {
    static const double p10[16] = {1e0, 1e1, 1e2, 1e3, 1e4, 1e5, 1e6, 1e7,
                                   1e8, 1e9, 1e10, 1e11, 1e12, 1e13, 1e14, 1e15};
    double a = fabs(v);
    if (!(a >= 1e-6 && a < 1e15)) return 0;
    uint64_t lim = single ? UINT64_C(1000000) : UINT64_C(1000000000000000);
    for (int k = 0; k <= 15; k++) {
        double sc = a * p10[k];
        if (sc >= (double) lim) return 0;
        double r = floor(sc + 0.5);
        for (int d = 0; d < 3; d++) {
            double c = d == 0 ? r : d == 1 ? r - 1 : r + 1;
            if (c < 1 || c >= (double) lim) continue;
            double back = c / p10[k];
            if (single ? (float) back != (float) a : back != a) continue;
            uint64_t m = (uint64_t) c;
            char dig[24];
            int nd = loft_u64_dec(cx, dig, m);
            if (v < 0) loft_push_char(cx, s, '-');
            if (nd <= k) {
                loft_push(cx, s, "0.", 2);
                for (int z = 0; z < k - nd; z++) loft_push_char(cx, s, '0');
                loft_push(cx, s, dig, (size_t) nd);
            } else {
                loft_push(cx, s, dig, (size_t) (nd - k));
                if (k > 0) {
                    loft_push_char(cx, s, '.');
                    loft_push(cx, s, dig + nd - k, (size_t) k);
                }
            }
            return 1;
        }
    }
    return 0;
}

static void loft_push_shortest(loft_cx *cx, loft_text *s, double v, int single) {
    if (loft_push_shortest_fast(cx, s, v, single)) return;
    if (isinf(v)) {
        if (v < 0) loft_push(cx, s, "-inf", 4); else loft_push(cx, s, "inf", 3);
        return;
    }
    /* The shortest digits that read back as `v`.  Rounding to 15 significant digits (6 for a
     * single) gives a value whose shortest form has k <= 15 digits exactly those k digits and
     * zeros — the 15-digit grid is far coarser than the value's own spacing, so that form is the
     * grid point nearest it — and the zeros are stripped below.  Past 15 the nearest rounding
     * reads back whenever any of that length does, so 16 and then 17 (9 for a single) finish:
     * at most three tries, where counting up from one digit took up to seventeen.  A subnormal
     * has fewer significant bits than the grid argument needs (its spacing can pass the 15-digit
     * grid's), so it counts up from one digit as before. */
    char buf[48];
    int maxp = single ? 9 : 17;
    int subnormal = v != 0 && fabs(v) < (single ? (double) FLT_MIN : DBL_MIN);
    int p = subnormal ? 1 : (single ? 6 : 15);
    for (; p <= maxp; p++) {
        snprintf(buf, sizeof buf, "%.*e", p - 1, v);
        double back = strtod(buf, 0);
        if (single ? (float) back == (float) v : back == v) break;
        if (loft_shortest_other(cx, buf, sizeof buf, p, v, back, single)) break;
    }
    const char *q = buf;
    if (*q == '-') { loft_push_char(cx, s, '-'); q++; }
    char digits[48];
    int nd = 0;
    for (; *q && *q != 'e'; q++) if (*q >= '0' && *q <= '9') digits[nd++] = *q;
    int exp = atoi(q + 1);
    while (nd > 1 && digits[nd - 1] == '0') nd--;
    if (nd == 1 && digits[0] == '0') { loft_push_char(cx, s, '0'); return; }
    if (exp >= 0) {
        int int_len = exp + 1;
        for (int i = 0; i < int_len; i++) loft_push_char(cx, s, i < nd ? digits[i] : '0');
        if (nd > int_len) { loft_push_char(cx, s, '.'); loft_push(cx, s, digits + int_len, (size_t) (nd - int_len)); }
    } else {
        loft_push(cx, s, "0.", 2);
        for (int i = 0; i < -exp - 1; i++) loft_push_char(cx, s, '0');
        loft_push(cx, s, digits, (size_t) nd);
    }
}

/* ops::format_signed */
static void loft_format_signed(loft_cx *cx, loft_text *s, loft_str res, int64_t width, int dir, uint8_t token) {
    int is_null = res.len == 4 && memcmp(res.p, "null", 4) == 0;
    if (token == '0' && !is_null && res.len > 0 && (res.p[0] == '-' || res.p[0] == '+')) {
        loft_push_char(cx, s, res.p[0]);
        loft_format_text(cx, s, loft_lit(cx, res.p + 1, res.len - 1), width - 1, dir, token);
        return;
    }
    loft_format_text(cx, s, res, width, dir, is_null ? loft_null_pad(cx, token) : token);
}

/* ops::format_float and ops::format_single */
/* `{v:.p}` for p <= 9 in exact integer arithmetic: v = m * 2^e with m < 2^53, so m * 10^p fits
 * 128 bits, and shifting it right by -e with round-half-even on the remainder gives the correctly
 * rounded p-digit value both Rust and printf print — without printf's arbitrary-precision path.
 * Answers 0 for what it does not cover (p > 9, an integer part past 2^63), which printf takes. */
typedef struct { uint64_t hi, lo; } loft_u128;

static int loft_push_fixed(loft_cx *cx, loft_text *s, double val, int prec) {
    if (prec < 0 || prec > 9 || !isfinite(val)) return 0;
    static const uint64_t p10[10] = { 1, 10, 100, 1000, 10000, 100000, 1000000, 10000000, 100000000,
                                      1000000000 };
    int ex;
    double f = frexp(fabs(val), &ex);
    uint64_t m = (uint64_t) ldexp(f, 53);
    int e = ex - 53;
    uint64_t P = p10[prec];
    /* m * P: m < 2^53, P < 2^30 — the halves of m times P, recombined with their carry. */
    uint64_t lo_part = (m & 0xFFFFFFFFu) * P, hi_part = (m >> 32) * P;
    loft_u128 x;
    x.lo = lo_part + (hi_part << 32);
    x.hi = (hi_part >> 32) + (x.lo < lo_part);
    uint64_t q;
    if (m == 0) {
        q = 0;
    } else if (e >= 0) {
        if (x.hi != 0 || e >= 63 || (x.lo >> (63 - e)) != 0) return 0;
        q = x.lo << e;
    } else {
        int sh = -e;
        if (sh >= 120) {
            q = 0; /* below 2^-36 after scaling: under one half, so it rounds to zero */
        } else {
            /* q = x >> sh, and the remainder against one half. */
            loft_u128 qq, rem, half;
            if (sh >= 64) {
                qq.hi = 0;
                qq.lo = sh == 64 ? x.hi : x.hi >> (sh - 64);
                rem.hi = sh == 64 ? 0 : x.hi & ((UINT64_C(1) << (sh - 64)) - 1);
                rem.lo = x.lo;
                half.hi = UINT64_C(1) << (sh - 65 < 0 ? 0 : sh - 65);
                half.lo = 0;
                if (sh == 64) { half.hi = 0; half.lo = UINT64_C(1) << 63; }
            } else {
                qq.hi = x.hi >> sh;
                qq.lo = (x.lo >> sh) | (sh == 0 ? 0 : x.hi << (64 - sh));
                rem.hi = 0;
                rem.lo = x.lo & ((UINT64_C(1) << sh) - 1);
                half.hi = 0;
                half.lo = UINT64_C(1) << (sh - 1);
            }
            if (qq.hi != 0 || (qq.lo >> 63) != 0) return 0;
            q = qq.lo;
            int cmp = rem.hi != half.hi ? (rem.hi > half.hi ? 1 : -1) : (rem.lo != half.lo ? (rem.lo > half.lo ? 1 : -1) : 0);
            if (cmp > 0 || (cmp == 0 && (q & 1))) q++;
        }
    }
    char buf[48];
    int n = 0;
    uint64_t ip = q / P, fp = q % P;
    char rev[24];
    int r = 0;
    do { rev[r++] = (char) ('0' + ip % 10); ip /= 10; } while (ip);
    if (signbit(val)) buf[n++] = '-';
    while (r) buf[n++] = rev[--r];
    if (prec > 0) {
        buf[n++] = '.';
        for (int i = prec - 1; i >= 0; i--) { buf[n + i] = (char) ('0' + fp % 10); fp /= 10; }
        n += prec;
    }
    loft_push(cx, s, buf, (size_t) n);
    return 1;
}

static void loft_format_float(loft_cx *cx, loft_text *s, double val, int64_t width, int64_t prec, uint8_t token, int plus,
                              int dir, int single) {
    if (dir == 2) dir = 1;
    size_t start = s->len;
    if (isnan(val)) {
        loft_push(cx, s, "null", 4);
    } else if (prec >= 0 && loft_push_fixed(cx, s, val, (int) prec)) {
    } else if (prec >= 0) {
        int n = snprintf(0, 0, "%.*f", (int) prec, val);
        char *buf = (char *) malloc((size_t) n + 1);
        if (!buf) { fputs("loft: out of memory\n", stderr); abort(); }
        snprintf(buf, (size_t) n + 1, "%.*f", (int) prec, val);
        loft_push(cx, s, buf, (size_t) n);
        free(buf);
    } else {
        loft_push_shortest(cx, s, val, single);
    }
    /* sign_a_number */
    if (plus && !isnan(val) && !(s->len > start && s->p[start] == '-')) {
        loft_push_char(cx, s, ' ');
        memmove(s->p + start + 1, s->p + start, s->len - start - 1);
        s->p[start] = '+';
    }
    if (width > (int64_t) (s->len - start)) {
        loft_text res = loft_text_own(cx, loft_lit(cx, s->p + start, s->len - start));
        s->len = start;
        loft_format_signed(cx, s, loft_view(cx, &res), width, dir, token);
        loft_text_free(cx, &res);
    }
}

/* ── Rendering a value of a store (database/format.rs ShowDb, the `{v}` of a record or a
 * vector) ─────────────────────────────────────────────────────────────────────────────────
 * `fmt` is the format operand: bit 1 pretty, bit 2 JSON.  The emitter writes one
 * `loft_show{tp}` per type a program renders; these are the parts every type shares. */
typedef void (*loft_show_fn)(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind);

static void loft_push_i64(loft_cx *cx, loft_text *s, int64_t v) {
    char buf[24];
    int n = 0;
    if (v < 0) buf[n++] = '-';
    n += loft_u64_dec(cx, buf + n, v < 0 ? (uint64_t) 0 - (uint64_t) v : (uint64_t) v);
    loft_push(cx, s, buf, (size_t) n);
}

static void loft_nl(loft_cx *cx, loft_text *s, int ind) {
    loft_push_char(cx, s, '\n');
    for (int i = 0; i < ind; i++) loft_push(cx, s, "  ", 2);
}

/* write_json_escaped */
static void loft_push_json(loft_cx *cx, loft_text *s, loft_str t) {
    loft_push_char(cx, s, '"');
    for (size_t i = 0; i < t.len; i++) {
        unsigned char c = (unsigned char) t.p[i];
        switch (c) {
        case '"': loft_push(cx, s, "\\\"", 2); break;
        case '\\': loft_push(cx, s, "\\\\", 2); break;
        case '\n': loft_push(cx, s, "\\n", 2); break;
        case '\r': loft_push(cx, s, "\\r", 2); break;
        case '\t': loft_push(cx, s, "\\t", 2); break;
        case 0x08: loft_push(cx, s, "\\b", 2); break;
        case 0x0c: loft_push(cx, s, "\\f", 2); break;
        default:
            if (c < 0x20) {
                char buf[8];
                snprintf(buf, sizeof buf, "\\u%04x", c);
                loft_push(cx, s, buf, 6);
            } else {
                loft_push_char(cx, s, (char) c);
            }
        }
    }
    loft_push_char(cx, s, '"');
}

/* A code point as UTF-8; 0 bytes for one that is not a character. */
static size_t loft_utf8(loft_cx *cx, uint32_t c, char *out) {
    if (c < 0x80) { out[0] = (char) c; return 1; }
    if (c < 0x800) { out[0] = (char) (0xC0 | (c >> 6)); out[1] = (char) (0x80 | (c & 0x3F)); return 2; }
    if (c >= 0xD800 && c <= 0xDFFF) return 0;
    if (c < 0x10000) {
        out[0] = (char) (0xE0 | (c >> 12)); out[1] = (char) (0x80 | ((c >> 6) & 0x3F));
        out[2] = (char) (0x80 | (c & 0x3F));
        return 3;
    }
    if (c < 0x110000) {
        out[0] = (char) (0xF0 | (c >> 18)); out[1] = (char) (0x80 | ((c >> 12) & 0x3F));
        out[2] = (char) (0x80 | ((c >> 6) & 0x3F)); out[3] = (char) (0x80 | (c & 0x3F));
        return 4;
    }
    return 0;
}

static void loft_show_long(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    int64_t v = loft_get_int(cx, r, 0);
    if (v == LOFT_NULL) loft_push(cx, s, "null", 4); else loft_push_i64(cx, s, v);
}

static void loft_show_single(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    float v = loft_get_single(cx, r, 0);
    if (isnan(v)) loft_push(cx, s, "null", 4); else loft_push_shortest(cx, s, v, 1);
}

static void loft_show_float(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    double v = loft_get_float(cx, r, 0);
    if (isnan(v)) loft_push(cx, s, "null", 4); else loft_push_shortest(cx, s, v, 0);
}

static void loft_show_bool(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    uint8_t b = *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos);
    loft_push_cstr(cx, s, b == 0 ? "false" : b == 255 ? "null" : "true");
}

static void loft_show_text(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind) {
    (void) ind;
    loft_store_t *st = loft_store_of(cx, r);
    uint32_t h = loft_u32(cx, st, r.rec, r.pos);
    if (h != 0 && h >= st->size) {
        /* A handle outside the store is corruption, not absence. */
        if (fmt & 2) { loft_push(cx, s, "null", 4); return; }
        loft_push_cstr(cx, s, "<bad-text:");
        loft_push_i64(cx, s, h);
        loft_push_char(cx, s, '>');
        return;
    }
    loft_str t = loft_get_text(cx, r, 0);
    if (loft_str_is_null(cx, t)) { loft_push(cx, s, "null", 4); return; }
    if (fmt & 2) { loft_push_json(cx, s, t); return; }
    loft_push_char(cx, s, '"');
    loft_text_append(cx, s, t);
    loft_push_char(cx, s, '"');
}

static void loft_show_char(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind) {
    (void) ind;
    uint32_t c = loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos);
    char buf[8]; /* loft_utf8 writes at most 4; the room keeps gcc's bounds check quiet */
    size_t n = c == 0 ? 0 : loft_utf8(cx, c, buf);
    if (n == 0) { loft_push(cx, s, "null", 4); return; }
    if (fmt & 2) { loft_push_json(cx, s, loft_lit(cx, buf, n)); return; }
    loft_push_char(cx, s, '\'');
    loft_push(cx, s, buf, n);
    loft_push_char(cx, s, '\'');
}

/* The narrow integer widths, each with its own null code (Stores::narrow_is_null). */
static void loft_show_byte(loft_cx *cx, loft_text *s, loft_ref r, int64_t min, int nullable) {
    uint8_t raw = *loft_at(cx, loft_store_of(cx, r), r.rec, r.pos);
    if (nullable && raw == 255) loft_push(cx, s, "null", 4); else loft_push_i64(cx, s, raw + min);
}

/* ── deliver (`Stores::deliver_loopback`): a value's bytes through its layout, printed as hex ── */
static void loft_dlv_hex(loft_cx *cx, loft_text *o, const uint8_t *p, size_t n) {
    static const char digits[] = "0123456789abcdef";
    for (size_t i = 0; i < n; i++) {
        char h[2] = { digits[p[i] >> 4], digits[p[i] & 15] };
        loft_push(cx, o, h, 2);
    }
}

/* A scalar's bytes as the store holds them, little-endian as `to_le_bytes` writes them. */
static void loft_dlv_raw(loft_cx *cx, loft_text *o, loft_ref r, uint32_t n) {
    loft_dlv_hex(cx, o, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), n);
}

/* A two-byte narrow integer: `get_short` (`null` reads 0, else the code less one) or, raw, the
 * code biased by `start` (`get_i16_raw`). */
static void loft_dlv_short(loft_cx *cx, loft_text *o, loft_ref r, int32_t start, int biased) {
    uint16_t raw;
    memcpy(&raw, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), 2);
    int16_t v = biased ? (raw ? (int16_t) (raw - 1) : 0) : (int16_t) (raw + start);
    uint8_t b[2] = { (uint8_t) v, (uint8_t) ((uint16_t) v >> 8) };
    loft_dlv_hex(cx, o, b, 2);
}

/* A text's bytes, its length nowhere. */
static void loft_dlv_text(loft_cx *cx, loft_text *o, loft_ref r) {
    loft_str s = loft_get_text(cx, r, 0);
    loft_dlv_hex(cx, o, (const uint8_t *) s.p, s.len);
}

typedef const char *(*loft_dlv_fn)(loft_cx *cx, loft_text *o, loft_ref r);

/* `deliver(tag, value)` on the loopback host: the bytes, or why the layout cannot carry them. */
static void loft_deliver(loft_cx *cx, int64_t tag, const char *name, loft_dlv_fn f, loft_ref r) {
    loft_text o = { 0, 0, 0 };
    const char *e = f(cx, &o, r);
    if (e) printf("deliver tag=%lld type=%s error=%s\n", (long long) tag, name, e);
    else printf("deliver tag=%lld type=%s bytes=%.*s\n", (long long) tag, name, (int) o.len, o.p ? o.p : "");
    loft_text_free(cx, &o);
}

/* The bytes of place `r` for a link that addresses them directly, or `absent` when `r` names
 * no record (`--native`'s `addr_mut` on a place, null for an absent one). */
static void *loft_place(loft_cx *cx, loft_ref r, void *absent) {
    return r.rec == 0 ? absent : (void *) loft_at(cx, loft_store_of(cx, r), r.rec, r.pos);
}

/* A stored reference (`Parts::DbRef`, ShowDb): `null` when it names no record or a removed
 * one (`@FR-H-Removed`), `iterator` for a generator handle, else its three parts — the debug
 * shape, which no format reads back. */
static void loft_show_dbref(loft_cx *cx, loft_text *s, loft_ref r) {
    uint32_t w[3];
    memcpy(w, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), 12);
    uint16_t nr = (uint16_t) w[0];
    if (w[1] == 0 || (nr < cx->nstores && loft_st_is_removed(cx, &cx->stores[nr], w[1]))) {
        loft_push(cx, s, "null", 4);
    } else if (nr == 0xFFFE || nr == 0xFFFF) {
        loft_push(cx, s, "iterator", 8);
    } else {
        char b[48];
        int n = snprintf(b, sizeof b, "DbRef(%u,%u,%u)", (unsigned) nr, (unsigned) w[1], (unsigned) w[2]);
        loft_push(cx, s, b, (size_t) n);
    }
}

static void loft_show_short(loft_cx *cx, loft_text *s, loft_ref r, int64_t min, int nullable) {
    uint16_t raw;
    memcpy(&raw, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), 2);
    if (nullable && raw == 0) { loft_push(cx, s, "null", 4); return; }
    loft_push_i64(cx, s, raw != 0 ? raw + min - 1 : (int64_t) INT32_MIN);
}

static void loft_show_short_raw(loft_cx *cx, loft_text *s, loft_ref r, int64_t min, int nullable) {
    uint16_t raw;
    memcpy(&raw, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), 2);
    if (nullable && raw == 0xFFFF) loft_push(cx, s, "null", 4); else loft_push_i64(cx, s, raw + min);
}

static void loft_show_int4(loft_cx *cx, loft_text *s, loft_ref r, int nullable) {
    int32_t v;
    memcpy(&v, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), 4);
    if (nullable && v == INT32_MIN) loft_push(cx, s, "null", 4); else loft_push_i64(cx, s, v);
}

static void loft_show_int_raw(loft_cx *cx, loft_text *s, loft_ref r, int nullable) {
    uint32_t v = loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos);
    if (nullable && v == 0xFFFFFFFFu) loft_push(cx, s, "null", 4); else loft_push_i64(cx, s, v);
}

static void loft_show_child(loft_cx *cx, loft_text *s, loft_ref r) {
    uint32_t v = loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos);
    if (v == 0) { loft_push(cx, s, "null", 4); return; }
    loft_push_cstr(cx, s, "ChildRec(");
    loft_push_i64(cx, s, v);
    loft_push_char(cx, s, ')');
}

/* write_struct's frame: the opening brace, and whether the body goes on lines of its own. */
static int loft_show_open(loft_cx *cx, loft_text *s, int fmt, int ind, int complex_type, int any) {
    int pretty = fmt & 1, json = (fmt & 2) != 0;
    int c = pretty && (json || complex_type);
    loft_push_char(cx, s, '{');
    if (c && json && any) loft_nl(cx, s, ind + 1);
    else if (pretty) loft_push_char(cx, s, ' ');
    return c;
}

static void loft_show_close_now(loft_cx *cx, loft_text *s, int fmt, int ind, int c, int any) {
    if (c && any) loft_nl(cx, s, ind);
    else if ((fmt & 1) && !c) loft_push_char(cx, s, ' ');
    loft_push_char(cx, s, '}');
}

/* `@FR-L-OutOfLine` (ShowDb::write): a chain of out-of-line children renders as a LOOP.  The
 * record whose last field holds a child leaves it in the tail; the closing text of every frame
 * the chain leaves waits on the closer stack until the chain ends. */
typedef struct loft_show_closer { uint8_t brace; int fmt, ind, c, any; } loft_show_closer;

static void loft_show_defer(loft_cx *cx, uint8_t brace, int fmt, int ind, int c, int any) {
    if (cx->nclosers == cx->closers_cap) {
        cx->closers_cap = cx->closers_cap ? cx->closers_cap * 2 : 64;
        cx->closers = (loft_show_closer *) realloc(cx->closers, cx->closers_cap * sizeof(loft_show_closer));
        if (!cx->closers) abort();
    }
    loft_show_closer k = { brace, fmt, ind, c, any };
    cx->closers[cx->nclosers++] = k;
}

static void loft_show_close(loft_cx *cx, loft_text *s, int fmt, int ind, int c, int any) {
    if (cx->tail_fn) loft_show_defer(cx, 0, fmt, ind, c, any);
    else loft_show_close_now(cx, s, fmt, ind, c, any);
}

/* A variant's JSON wrapper closing: now, or after a pending chain. */
static void loft_show_brace(loft_cx *cx, loft_text *s) {
    if (cx->tail_fn) loft_show_defer(cx, 1, 0, 0, 0, 0);
    else loft_push_char(cx, s, '}');
}

/* The reference-render path of a chain (`loft_show_top`, below). */
static void loft_rr_pop(loft_cx *cx);
static void loft_rr_tail_save(loft_cx *cx);
static void loft_rr_tail_take(loft_cx *cx);
typedef struct { char *p; size_t len; size_t *marks; size_t n; } loft_rr_saved;
static loft_rr_saved loft_rr_save(loft_cx *cx);
static void loft_rr_restore(loft_cx *cx, loft_rr_saved *k);

/* The last field's child: left for the loop of the nearest loft_show_run, or `null`.  The
 * field's name is on the reference-render path; a child left for the loop takes it along. */
static void loft_show_tail(loft_cx *cx, loft_text *s, loft_show1_fn f, loft_ref r, int ind) {
    if (r.rec == 0) { loft_push(cx, s, "null", 4); loft_rr_pop(cx); return; }
    cx->tail_fn = f;
    cx->tail_ref = r;
    cx->tail_ind = ind;
    loft_rr_tail_save(cx);
}

static void loft_show_run(loft_cx *cx, loft_text *s, loft_show1_fn f, loft_ref r, int fmt, int ind) {
    size_t mark = cx->nclosers;
    loft_rr_saved saved = { 0, 0, 0, 0 };
    int track = cx->rr_on;
    if (track) saved = loft_rr_save(cx);
    f(cx, s, r, fmt, ind);
    while (cx->tail_fn) {
        loft_show1_fn g = cx->tail_fn;
        cx->tail_fn = 0;
        loft_rr_tail_take(cx);
        g(cx, s, cx->tail_ref, fmt, cx->tail_ind);
    }
    if (track) loft_rr_restore(cx, &saved);
    while (cx->nclosers > mark) {
        loft_show_closer k = cx->closers[--cx->nclosers];
        if (k.brace) loft_push_char(cx, s, '}');
        else loft_show_close_now(cx, s, k.fmt, k.ind, k.c, k.any);
    }
}

/* Between two fields (write_indent with its zero test). */
static void loft_show_sep(loft_cx *cx, loft_text *s, int fmt, int ind, int c) {
    loft_push_char(cx, s, ',');
    if (c) loft_nl(cx, s, ind + 1);
    else if (fmt & 1) loft_push_char(cx, s, ' ');
}

static void loft_show_name(loft_cx *cx, loft_text *s, int fmt, const char *name) {
    if (fmt & 2) loft_push_char(cx, s, '"');
    loft_push_cstr(cx, s, name);
    if (fmt & 2) loft_push_char(cx, s, '"');
    loft_push_char(cx, s, ':');
    if (fmt & 1) loft_push_char(cx, s, ' ');
}

/* The reference-render path (`loft_show_top`, below): an element's position is its segment. */
static void loft_rr_push_index(loft_cx *cx, uint32_t i);
static void loft_rr_pop(loft_cx *cx);

/* write_list for a vector: its elements of `size` bytes, each rendered by `f`. */
static void loft_show_list(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind, uint32_t size,
                           int complex_elem, int struct_elem, loft_show_fn f) {
    int pretty = fmt & 1;
    int c = pretty && complex_elem;
    loft_push_char(cx, s, '[');
    uint32_t vr = loft_collection_rec(cx, loft_store_of(cx, r), r.rec, r.pos);
    uint32_t n = vr == 0 ? 0 : loft_u32(cx, loft_store_of(cx, r), vr, 4);
    for (uint32_t i = 0; i < n; i++) {
        if (i == 0) {
            if (pretty) { if (c) loft_nl(cx, s, ind + 1); else loft_push_char(cx, s, ' '); }
        } else {
            loft_push_char(cx, s, ',');
            if (pretty) { if (struct_elem) loft_nl(cx, s, ind + 1); else loft_push_char(cx, s, ' '); }
        }
        loft_ref e = { r.store_nr, vr, 8 + i * size, 0 };
        loft_rr_push_index(cx, i);
        f(cx, s, e, fmt, ind + 1);
        loft_rr_pop(cx);
    }
    if (pretty) loft_push_char(cx, s, ' ');
    loft_push_char(cx, s, ']');
}

/* `@FR-F-Render-Ref` / `@FR-F-Render-Refs` (Stores::render_top, database/format.rs) — a top-level
 * render of a type that can reach a stored reference writes every reference as an RFC 6901
 * pointer to its target, `{"$ref":"#/leaves/1"}`, and brings the targets the value does not reach
 * along under `$refs`.  The context is per render: the path the walk is at, the pointer of every
 * record it has written (first one wins), and the target of every reference it met — a
 * reference leaves a marker the end of the render patches once every pointer is known.
 * (`cx->rr_on`, the flag every helper asks first, is declared beside the chain loop.) */
typedef struct loft_rr_place { uint16_t nr; uint32_t rec, pos; size_t at, len; } loft_rr_place;
typedef struct loft_rr_target { uint16_t nr; uint32_t rec, pos; loft_show_fn f; } loft_rr_target;
/* The path an out-of-line child is written at (ShowDb::write's tail_path). */

#define LOFT_RR_MARK_OPEN "\001\002R"
#define LOFT_RR_MARK_CLOSE '\002'

static void loft_rr_push_raw(loft_cx *cx, const char *seg, size_t n) {
    if (!cx->rr_on) return;
    if (cx->rr_nmarks == cx->rr_cmarks) {
        cx->rr_cmarks = cx->rr_cmarks ? cx->rr_cmarks * 2 : 32;
        cx->rr_marks = (size_t *) realloc(cx->rr_marks, cx->rr_cmarks * sizeof(size_t));
        if (!cx->rr_marks) abort();
    }
    cx->rr_marks[cx->rr_nmarks++] = cx->rr_path.len;
    loft_push_char(cx, &cx->rr_path, '/');
    /* RFC 6901: `~` is `~0` and `/` is `~1` inside a segment. */
    for (size_t i = 0; i < n; i++) {
        if (seg[i] == '~') loft_push(cx, &cx->rr_path, "~0", 2);
        else if (seg[i] == '/') loft_push(cx, &cx->rr_path, "~1", 2);
        else loft_push_char(cx, &cx->rr_path, seg[i]);
    }
}

static void loft_rr_push(loft_cx *cx, const char *name) {
    if (cx->rr_on) loft_rr_push_raw(cx, name, strlen(name));
}

static void loft_rr_push_index(loft_cx *cx, uint32_t i) {
    if (!cx->rr_on) return;
    char b[16];
    int n = snprintf(b, sizeof b, "%u", (unsigned) i);
    loft_rr_push_raw(cx, b, (size_t) n);
}

static void loft_rr_pop(loft_cx *cx) {
    if (cx->rr_on && cx->rr_nmarks) cx->rr_path.len = cx->rr_marks[--cx->rr_nmarks];
}

static size_t loft_rr_hash(loft_cx *cx, uint16_t nr, uint32_t rec, uint32_t pos) {
    uint64_t h = ((uint64_t) nr << 48) ^ ((uint64_t) rec << 16) ^ pos;
    h ^= h >> 33;
    h *= 0xff51afd7ed558ccdULL;
    h ^= h >> 33;
    return (size_t) h;
}

/* The pointer recorded for a record, or NULL. */
static const loft_rr_place *loft_rr_find(loft_cx *cx, uint16_t nr, uint32_t rec, uint32_t pos) {
    if (!cx->rr_ctab) return 0;
    size_t m = cx->rr_ctab - 1;
    for (size_t i = loft_rr_hash(cx, nr, rec, pos) & m;; i = (i + 1) & m) {
        size_t k = cx->rr_tab[i];
        if (k == 0) return 0;
        const loft_rr_place *p = &cx->rr_at[k - 1];
        if (p->nr == nr && p->rec == rec && p->pos == pos) return p;
    }
}

/* Record `r`'s pointer as the path the walk is at, unless it has one: first one wins. */
static void loft_rr_place_at(loft_cx *cx, uint16_t nr, uint32_t rec, uint32_t pos, const char *p, size_t n) {
    if (loft_rr_find(cx, nr, rec, pos)) return;
    if (cx->rr_nat == cx->rr_cat) {
        cx->rr_cat = cx->rr_cat ? cx->rr_cat * 2 : 64;
        cx->rr_at = (loft_rr_place *) realloc(cx->rr_at, cx->rr_cat * sizeof(loft_rr_place));
        if (!cx->rr_at) abort();
    }
    if (2 * (cx->rr_nat + 1) > cx->rr_ctab) {
        size_t c = cx->rr_ctab ? cx->rr_ctab * 2 : 128;
        free(cx->rr_tab);
        cx->rr_tab = (size_t *) calloc(c, sizeof(size_t));
        if (!cx->rr_tab) abort();
        cx->rr_ctab = c;
        for (size_t k = 0; k < cx->rr_nat; k++) {
            size_t i = loft_rr_hash(cx, cx->rr_at[k].nr, cx->rr_at[k].rec, cx->rr_at[k].pos) & (c - 1);
            while (cx->rr_tab[i]) i = (i + 1) & (c - 1);
            cx->rr_tab[i] = k + 1;
        }
    }
    loft_rr_place e = { nr, rec, pos, cx->rr_pool.len, n };
    if (n) loft_push(cx, &cx->rr_pool, p, n);
    cx->rr_at[cx->rr_nat++] = e;
    size_t i = loft_rr_hash(cx, nr, rec, pos) & (cx->rr_ctab - 1);
    while (cx->rr_tab[i]) i = (i + 1) & (cx->rr_ctab - 1);
    cx->rr_tab[i] = cx->rr_nat;
}

/* A record of struct or enum type is being written: its pointer is where the walk is. */
static void loft_rr_record(loft_cx *cx, loft_ref r) {
    if (cx->rr_on) loft_rr_place_at(cx, r.store_nr, r.rec, r.pos, cx->rr_path.p, cx->rr_path.len);
}

/* An out-of-line child left for the chain loop is written at the path its field had. */
static void loft_rr_tail_save(loft_cx *cx) {
    if (!cx->rr_on) return;
    cx->rr_tail_path.len = 0;
    if (cx->rr_path.len) loft_push(cx, &cx->rr_tail_path, cx->rr_path.p, cx->rr_path.len);
    cx->rr_tail_marks = (size_t *) realloc(cx->rr_tail_marks, (cx->rr_nmarks + 1) * sizeof(size_t));
    if (!cx->rr_tail_marks) abort();
    if (cx->rr_nmarks) memcpy(cx->rr_tail_marks, cx->rr_marks, cx->rr_nmarks * sizeof(size_t));
    cx->rr_tail_nmarks = cx->rr_nmarks;
    cx->rr_tail_set = 1;
    loft_rr_pop(cx);
}

/* The whole path, for a chain loop to put back once its children are written. */
static loft_rr_saved loft_rr_save(loft_cx *cx) {
    loft_rr_saved k = { 0, cx->rr_path.len, 0, cx->rr_nmarks };
    if (cx->rr_path.len) {
        k.p = (char *) malloc(cx->rr_path.len);
        if (!k.p) abort();
        memcpy(k.p, cx->rr_path.p, cx->rr_path.len);
    }
    if (cx->rr_nmarks) {
        k.marks = (size_t *) malloc(cx->rr_nmarks * sizeof(size_t));
        if (!k.marks) abort();
        memcpy(k.marks, cx->rr_marks, cx->rr_nmarks * sizeof(size_t));
    }
    return k;
}

static void loft_rr_restore(loft_cx *cx, loft_rr_saved *k) {
    cx->rr_path.len = 0;
    if (k->len) loft_push(cx, &cx->rr_path, k->p, k->len);
    if (k->n > cx->rr_cmarks) {
        cx->rr_cmarks = k->n;
        cx->rr_marks = (size_t *) realloc(cx->rr_marks, cx->rr_cmarks * sizeof(size_t));
        if (!cx->rr_marks) abort();
    }
    if (k->n) memcpy(cx->rr_marks, k->marks, k->n * sizeof(size_t));
    cx->rr_nmarks = k->n;
    free(k->p);
    free(k->marks);
}

static void loft_rr_tail_take(loft_cx *cx) {
    if (!cx->rr_on || !cx->rr_tail_set) return;
    cx->rr_tail_set = 0;
    cx->rr_path.len = 0;
    if (cx->rr_tail_path.len) loft_push(cx, &cx->rr_path, cx->rr_tail_path.p, cx->rr_tail_path.len);
    if (cx->rr_tail_nmarks > cx->rr_cmarks) {
        cx->rr_cmarks = cx->rr_tail_nmarks;
        cx->rr_marks = (size_t *) realloc(cx->rr_marks, cx->rr_cmarks * sizeof(size_t));
        if (!cx->rr_marks) abort();
    }
    if (cx->rr_tail_nmarks) memcpy(cx->rr_marks, cx->rr_tail_marks, cx->rr_tail_nmarks * sizeof(size_t));
    cx->rr_nmarks = cx->rr_tail_nmarks;
}

/* A stored reference FIELD (ShowDb::write_reference_marker): under a tracking render a live
 * reference leaves a marker naming its target, written by `f` (the render of the type the
 * field names); a null or removed reference and a generator handle render as the plain slot. */
static void loft_show_ref_field(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind, loft_show_fn f) {
    uint32_t w[3];
    memcpy(w, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), 12);
    uint16_t nr = (uint16_t) w[0];
    if (!cx->rr_on || w[1] == 0 || nr == 0xFFFF || nr == 0xFFFE
        || (nr < cx->nstores && loft_st_is_removed(cx, &cx->stores[nr], w[1]))) {
        (void) fmt; (void) ind;
        loft_show_dbref(cx, s, r);
        return;
    }
    if (cx->rr_nt == cx->rr_ct) {
        cx->rr_ct = cx->rr_ct ? cx->rr_ct * 2 : 16;
        cx->rr_t = (loft_rr_target *) realloc(cx->rr_t, cx->rr_ct * sizeof(loft_rr_target));
        if (!cx->rr_t) abort();
    }
    loft_rr_target t = { nr, w[1], w[2], f };
    cx->rr_t[cx->rr_nt] = t;
    loft_push(cx, s, LOFT_RR_MARK_OPEN, 3);
    loft_push_i64(cx, s, (int64_t) cx->rr_nt);
    loft_push_char(cx, s, LOFT_RR_MARK_CLOSE);
    cx->rr_nt++;
}

static void loft_rr_reset(loft_cx *cx) {
    cx->rr_path.len = 0;
    cx->rr_nmarks = 0;
    cx->rr_nat = 0;
    cx->rr_nt = 0;
    cx->rr_pool.len = 0;
    cx->rr_tail_set = 0;
    if (cx->rr_ctab) memset(cx->rr_tab, 0, cx->rr_ctab * sizeof(size_t));
}

/* Stores::render_top: render `r` by `f` into `s`, references as pointers, the targets the value
 * does not reach under `$refs` (an object gains it as its last member, any other value is
 * wrapped as `{"$value": …, "$refs": […]}`).  A render inside a render writes as before. */
static void loft_show_top(loft_cx *cx, loft_text *s, loft_show_fn f, loft_ref r, int fmt) {
    if (cx->rr_on) { f(cx, s, r, fmt, 0); return; }
    loft_rr_reset(cx);
    cx->rr_on = 1;
    int pretty = fmt & 1;
    loft_text body = { 0, 0, 0 };
    f(cx, &body, r, fmt, 0);
    loft_text list = { 0, 0, 0 };
    size_t nrefs = 0;
    for (size_t k = 0; k < cx->rr_nt; k++) {
        loft_rr_target t = cx->rr_t[k];
        if (!t.f || loft_rr_find(cx, t.nr, t.rec, t.pos)) continue;
        cx->rr_path.len = 0;
        cx->rr_nmarks = 0;
        char seg[16];
        int n = snprintf(seg, sizeof seg, "%u", (unsigned) nrefs);
        loft_rr_push_raw(cx, "$refs", 5);
        loft_rr_push_raw(cx, seg, (size_t) n);
        loft_rr_place_at(cx, t.nr, t.rec, t.pos, cx->rr_path.p, cx->rr_path.len);
        if (nrefs) loft_push_char(cx, &list, ',');
        loft_ref tr = { t.nr, t.rec, t.pos, 0 };
        t.f(cx, &list, tr, fmt, 1);
        nrefs++;
    }
    cx->rr_on = 0;
    int wrapped = 0;
    if (nrefs) {
        loft_text b2 = { 0, 0, 0 };
        size_t end = body.len;
        while (end && (body.p[end - 1] == ' ' || body.p[end - 1] == '\n' || body.p[end - 1] == '\t' || body.p[end - 1] == '\r')) end--;
        if (end && body.p[end - 1] == '}') {
            size_t open = end - 1;
            while (open && (body.p[open - 1] == ' ' || body.p[open - 1] == '\n' || body.p[open - 1] == '\t' || body.p[open - 1] == '\r')) open--;
            if (open) loft_push(cx, &b2, body.p, open);
            if (!(open && body.p[open - 1] == '{')) loft_push_char(cx, &b2, ',');
            if (pretty) loft_push(cx, &b2, "\n  ", 3);
            loft_push_cstr(cx, &b2, pretty ? "\"$refs\": [" : "\"$refs\":[");
            if (list.len) loft_push(cx, &b2, list.p, list.len);
            loft_push_char(cx, &b2, ']');
            if (pretty) loft_push_char(cx, &b2, '\n');
            loft_push_char(cx, &b2, '}');
        } else {
            wrapped = 1;
            loft_push_cstr(cx, &b2, pretty ? "{\"$value\": " : "{\"$value\":");
            if (body.len) loft_push(cx, &b2, body.p, body.len);
            loft_push_cstr(cx, &b2, pretty ? ",\"$refs\": [" : ",\"$refs\":[");
            if (list.len) loft_push(cx, &b2, list.p, list.len);
            loft_push_cstr(cx, &b2, "]}");
        }
        loft_text_free(cx, &body);
        body = b2;
    }
    loft_text_free(cx, &list);
    /* Patch every marker with its target's pointer. */
    size_t i = 0;
    while (i < body.len) {
        const char *at = 0;
        for (size_t j = i; j + 2 < body.len; j++)
            if (body.p[j] == '\001' && body.p[j + 1] == '\002' && body.p[j + 2] == 'R') { at = body.p + j; break; }
        if (!at) break;
        size_t j = (size_t) (at - body.p);
        loft_push(cx, s, body.p + i, j - i);
        size_t k = 0, e = j + 3;
        while (e < body.len && body.p[e] != LOFT_RR_MARK_CLOSE) k = k * 10 + (size_t) (body.p[e++] - '0');
        const loft_rr_place *p = k < cx->rr_nt ? loft_rr_find(cx, cx->rr_t[k].nr, cx->rr_t[k].rec, cx->rr_t[k].pos) : 0;
        if (!p) {
            loft_push(cx, s, "null", 4);
        } else {
            const char *pp = p->len ? cx->rr_pool.p + p->at : "";
            int in_refs = p->len >= 7 && memcmp(pp, "/$refs/", 7) == 0;
            loft_push_cstr(cx, s, "{\"$ref\":\"#");
            if (wrapped && !in_refs) loft_push(cx, s, "/$value", 7);
            if (p->len) loft_push(cx, s, pp, p->len);
            loft_push_cstr(cx, s, "\"}");
        }
        i = e < body.len ? e + 1 : body.len;
    }
    if (body.len > i) loft_push(cx, s, body.p + i, body.len - i);
    loft_text_free(cx, &body);
}

/* ── The host: arguments and the clock ─────────────────────────────────────────────────── */

/* The program's own arguments, after its name (Stores::os_arguments). */
static int loft_argc = 0;
static char **loft_argv = 0;

/* arguments(): a fresh `vector<text>` of them. */
static loft_ref loft_arguments(loft_cx *cx) {
    loft_ref r = loft_ref_null(cx);
    loft_vector_store(cx, &r);
    for (int i = 1; i < loft_argc; i++) {
        loft_ref e = loft_vec_append(cx, r, 4);
        loft_set_text(cx, e, 0, loft_lit(cx, loft_argv[i], strlen(loft_argv[i])));
        loft_vec_finish(cx, r);
    }
    return r;
}

/* web's byte builder (`src/web_bytes.rs`, the `pack_*` natives) in the binary: a per-thread
 * buffer the appends grow and `pack_take` hands over as a text, bytes verbatim, emptying it. */
static void loft_pk_push(loft_cx *cx, const uint8_t *b, size_t n) {
    if (cx->pk_n + n > cx->pk_cap) {
        size_t cap = cx->pk_cap ? cx->pk_cap * 2 : 64;
        while (cap < cx->pk_n + n) cap *= 2;
        uint8_t *p = (uint8_t *) realloc(cx->pk_p, cap);
        if (!p) { fputs("loft: out of memory\n", stderr); abort(); }
        cx->pk_p = p;
        cx->pk_cap = cap;
    }
    memcpy(cx->pk_p + cx->pk_n, b, n);
    cx->pk_n += n;
}
static inline void loft_pack_reset(loft_cx *cx) { cx->pk_n = 0; }
static inline void loft_pack_u8(loft_cx *cx, int64_t b) {
    if (cx->pk_n < cx->pk_cap) {
        cx->pk_p[cx->pk_n++] = (uint8_t) b;
    } else {
        uint8_t x = (uint8_t) b;
        loft_pk_push(cx, &x, 1);
    }
}
static void loft_pack_u16_le(loft_cx *cx, int64_t v) {
    uint8_t x[2] = { (uint8_t) v, (uint8_t) (v >> 8) };
    loft_pk_push(cx, x, 2);
}
static void loft_pack_u32_le(loft_cx *cx, int64_t v) {
    uint8_t x[4] = { (uint8_t) v, (uint8_t) (v >> 8), (uint8_t) (v >> 16), (uint8_t) (v >> 24) };
    loft_pk_push(cx, x, 4);
}
static loft_text loft_pack_take(loft_cx *cx) {
    loft_text t = loft_text_own(cx, loft_lit(cx, cx->pk_n ? (const char *) cx->pk_p : "", cx->pk_n));
    cx->pk_n = 0;
    return t;
}
/* byte_at(idx, t): the byte at `idx`, or -1 outside the text. */
static inline int64_t loft_web_byte_at(loft_cx *cx, int64_t idx, loft_str t) {
    return idx < 0 || (uint64_t) idx >= (uint64_t) t.len ? -1 : (int64_t) (uint8_t) t.p[idx];
}

/* ticks(): microseconds since the program started, monotonic; LOFT_FAKE_TICKS_US pins it. */
static int64_t loft_start_us = -1;

/* The pinned clocks (LOFT_FAKE_TICKS_US, LOFT_FAKE_NOW_MS), read once — at `loft_start`, before
 * any thread runs, so a `par` worker or a `parallel` arm only reads them; a unit with no start
 * (the runner's shim) reads them at the first call. */
static int loft_clock_fakes_read = 0;
static int64_t loft_fake_ticks = LOFT_NULL, loft_fake_now = LOFT_NULL;

static int64_t loft_env_long(loft_cx *cx, const char *name) {
    const char *f = getenv(name);
    if (f && *f) {
        char *end;
        long long v = strtoll(f, &end, 10);
        if (*end == 0) return (int64_t) v;
    }
    return LOFT_NULL;
}

static void loft_clock_fakes(loft_cx *cx) {
    loft_fake_ticks = loft_env_long(cx, "LOFT_FAKE_TICKS_US");
    loft_fake_now = loft_env_long(cx, "LOFT_FAKE_NOW_MS");
    loft_clock_fakes_read = 1;
}

static int64_t loft_ticks(loft_cx *cx) {
    if (!loft_clock_fakes_read) loft_clock_fakes(cx);
    if (loft_fake_ticks != LOFT_NULL) return loft_fake_ticks;
    return loft_now_us(cx) - loft_start_us;
}

/* now(): milliseconds since the Unix epoch; LOFT_FAKE_NOW_MS pins it. */
static int64_t loft_now(loft_cx *cx) {
    if (!loft_clock_fakes_read) loft_clock_fakes(cx);
    if (loft_fake_now != LOFT_NULL) return loft_fake_now;
#if defined(LOFT_POSIX)
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    return (int64_t) t.tv_sec * 1000 + t.tv_nsec / 1000000;
#elif defined(_WIN32)
    return loft_win_wall_ms(cx);
#else
    return (int64_t) time(0) * 1000;
#endif
}

/* OpAbsInt (ops::op_abs_long): null stays null. */
static int64_t loft_abs(loft_cx *cx, int64_t v) {
    return v == LOFT_NULL ? v : v < 0 ? -v : v;
}

/* OpPowInt (ops::op_pow_int): `checked_pow` — null for a null or negative operand, an exponent
 * past 32 bits, or an overflow on the way. */
static int64_t loft_pow(loft_cx *cx, int64_t base, int64_t exp) {
    if (base == LOFT_NULL || exp == LOFT_NULL || exp < 0 || exp > (int64_t) UINT32_MAX) return LOFT_NULL;
    if (exp == 0) return 1;
    int64_t acc = 1;
    for (;;) {
        if (exp & 1) {
            if (loft_mul_ovf_c99(acc, base, &acc)) return LOFT_NULL;
            if (exp == 1) return acc;
        }
        exp /= 2;
        if (loft_mul_ovf_c99(base, base, &base)) return LOFT_NULL;
    }
}

/* OpCastIntFromText: Rust's `str::parse::<i64>` — an optional sign and decimal digits, nothing
 * else; anything that does not parse, or overflows, is null. */
static int64_t loft_int_of_text(loft_cx *cx, loft_str t) {
    size_t i = 0;
    int neg = 0;
    if (t.len == 0) return LOFT_NULL;
    if (t.p[0] == '+' || t.p[0] == '-') { neg = t.p[0] == '-'; i = 1; }
    if (i == t.len) return LOFT_NULL;
    uint64_t v = 0;
    for (; i < t.len; i++) {
        unsigned char c = (unsigned char) t.p[i];
        if (c < '0' || c > '9') return LOFT_NULL;
        if (v > (UINT64_MAX - (c - '0')) / 10) return LOFT_NULL;
        v = v * 10 + (c - '0');
    }
    if (neg) return v > (uint64_t) INT64_MAX + 1 ? LOFT_NULL : (int64_t) (0 - v);
    return v > (uint64_t) INT64_MAX ? LOFT_NULL : (int64_t) v;
}

/* Does `t` follow Rust's float grammar (`str::parse::<f64>`): an optional sign, then `inf`,
 * `infinity` or `nan` in any case, or digits with an optional point and exponent — no space,
 * no hexadecimal? */
static int loft_float_text_ok(loft_cx *cx, loft_str t) {
    size_t i = 0, n = t.len;
    if (n > 0 && (t.p[0] == '+' || t.p[0] == '-')) i = 1;
    if (i == n) return 0;
    const char *words[] = { "inf", "infinity", "nan" };
    for (int w = 0; w < 3; w++) {
        size_t wl = strlen(words[w]);
        if (n - i != wl) continue;
        size_t k = 0;
        while (k < wl && (t.p[i + k] | 32) == words[w][k]) k++;
        if (k == wl) return 1;
    }
    size_t digits = 0;
    while (i < n && t.p[i] >= '0' && t.p[i] <= '9') { i++; digits++; }
    if (i < n && t.p[i] == '.') {
        i++;
        while (i < n && t.p[i] >= '0' && t.p[i] <= '9') { i++; digits++; }
    }
    if (digits == 0) return 0;
    if (i < n && (t.p[i] == 'e' || t.p[i] == 'E')) {
        i++;
        if (i < n && (t.p[i] == '+' || t.p[i] == '-')) i++;
        size_t ed = 0;
        while (i < n && t.p[i] >= '0' && t.p[i] <= '9') { i++; ed++; }
        if (ed == 0) return 0;
    }
    return i == n;
}

/* Clinger's fast path for a validated decimal: a significand that fits 2^53 exactly and a power
 * of ten no larger than 10^22 (exact in a double) give the correctly rounded value as ONE IEEE
 * operation on two exact doubles — what strtod answers, without its arbitrary-precision path.
 * Answers 0 for every other text (an exponent outside the range, more digits than fit, inf,
 * nan), which strtod then reads; and for a platform that evaluates in wider precision, where
 * the one operation would round twice. */
static int loft_float_fast(loft_cx *cx, loft_str t, double *out) {
#if defined(FLT_EVAL_METHOD) && FLT_EVAL_METHOD != 0
    (void) t;
    (void) out;
    return 0;
#else
    static const double p10[23] = { 1e0,  1e1,  1e2,  1e3,  1e4,  1e5,  1e6,  1e7,  1e8,  1e9,  1e10, 1e11,
                                    1e12, 1e13, 1e14, 1e15, 1e16, 1e17, 1e18, 1e19, 1e20, 1e21, 1e22 };
    size_t i = 0, n = t.len;
    int neg = 0;
    if (i < n && (t.p[i] == '+' || t.p[i] == '-')) neg = t.p[i++] == '-';
    uint64_t m = 0;
    int64_t e10 = 0;
    int any = 0;
    for (int frac = 0; i < n; i++) {
        char c = t.p[i];
        if (c == '.' && !frac) { frac = 1; continue; }
        if (c < '0' || c > '9') break;
        if (m > UINT64_C(100000000000000000)) return 0;
        m = m * 10 + (uint64_t) (c - '0');
        if (frac) e10--;
        any = 1;
    }
    if (!any || m > (UINT64_C(1) << 53)) return 0;
    if (i < n && (t.p[i] == 'e' || t.p[i] == 'E')) {
        i++;
        int eneg = 0;
        if (i < n && (t.p[i] == '+' || t.p[i] == '-')) eneg = t.p[i++] == '-';
        int64_t e = 0;
        for (; i < n && t.p[i] >= '0' && t.p[i] <= '9'; i++) {
            if (e > 1000) return 0;
            e = e * 10 + (t.p[i] - '0');
        }
        e10 += eneg ? -e : e;
    }
    if (i != n || e10 < -22 || e10 > 22) return 0;
    double d = (double) m;
    d = e10 >= 0 ? d * p10[e10] : d / p10[-e10];
    *out = neg ? -d : d;
    return 1;
#endif
}

/* `@FR-R-LazySplit` (codegen_runtime::lazy_split): the pieces of `text.split(c)` taken one at a
 * time from the text, with no vector built — the same pieces in the same order: none for an
 * empty or null text, and after the last separator the rest, however short. */
typedef struct { loft_str rest; int live; unsigned char sep[4]; int nsep; } loft_ls;

static loft_ls loft_ls_of(loft_cx *cx, loft_str t, uint32_t sep) {
    loft_ls ls;
    ls.rest = t;
    ls.live = !(t.len == 0 || loft_str_is_null(cx, t));
    if (sep < 0x80) { ls.sep[0] = (unsigned char) sep; ls.nsep = 1; }
    else if (sep < 0x800) { ls.sep[0] = (unsigned char) (0xC0 | (sep >> 6)); ls.sep[1] = (unsigned char) (0x80 | (sep & 0x3F)); ls.nsep = 2; }
    else if (sep < 0x10000) {
        ls.sep[0] = (unsigned char) (0xE0 | (sep >> 12)); ls.sep[1] = (unsigned char) (0x80 | ((sep >> 6) & 0x3F));
        ls.sep[2] = (unsigned char) (0x80 | (sep & 0x3F)); ls.nsep = 3;
    } else {
        ls.sep[0] = (unsigned char) (0xF0 | (sep >> 18)); ls.sep[1] = (unsigned char) (0x80 | ((sep >> 12) & 0x3F));
        ls.sep[2] = (unsigned char) (0x80 | ((sep >> 6) & 0x3F)); ls.sep[3] = (unsigned char) (0x80 | (sep & 0x3F)); ls.nsep = 4;
    }
    return ls;
}

/* The next piece, or — out of pieces — the null text with `*done` raised, which the loop's
 * length test reads. */
LOFT_HOT_INLINE loft_str loft_ls_next(loft_cx *cx, loft_ls *ls, uint8_t *done) {
    if (!ls->live) {
        *done = 1;
        return loft_null_text(cx);
    }
    const char *p = ls->rest.p;
    size_t n = ls->rest.len, at = 0;
    for (;;) {
        const char *hit = n > at ? (const char *) memchr(p + at, ls->sep[0], n - at) : 0;
        if (!hit) break;
        size_t off = (size_t) (hit - p);
        if (off + (size_t) ls->nsep <= n && memcmp(hit, ls->sep, (size_t) ls->nsep) == 0) {
            loft_str piece = { p, off };
            ls->rest.p = p + off + ls->nsep;
            ls->rest.len = n - off - (size_t) ls->nsep;
            return piece;
        }
        at = off + 1;
    }
    ls->live = 0;
    return ls->rest;
}

/* `@FR-R-SplitTable` (codegen_runtime's split table): `parts = text.split(c)` read only by its
 * length and its element reads, as the pieces `loft_ls` takes — none for an empty or null text —
 * held as slices of a copy of the text the table owns, so no later write of the source reaches
 * them.  The vector is never built; a rebind refills the same buffers. */
typedef struct { loft_text src; loft_str *at; size_t n; size_t cap; } loft_stab;

static void loft_stab_fill(loft_cx *cx, loft_stab *t, loft_str s, uint32_t sep) {
    t->n = 0;
    if (s.len == 0 || loft_str_is_null(cx, s)) return;
    t->src.len = 0;
    loft_push(cx, &t->src, s.p, s.len);
    loft_ls ls = loft_ls_of(cx, loft_view(cx, &t->src), sep);
    uint8_t done = 0;
    for (;;) {
        loft_str piece = loft_ls_next(cx, &ls, &done);
        if (done) break;
        if (t->n == t->cap) {
            size_t cap = t->cap ? t->cap * 2 : 16;
            loft_str *at = (loft_str *) realloc(t->at, cap * sizeof *at);
            if (!at) { fputs("loft: out of memory\n", stderr); abort(); }
            t->at = at;
            t->cap = cap;
        }
        t->at[t->n++] = piece;
    }
}

/* Element `from` as the vector read answers it: negative counts from the end, outside is null. */
LOFT_HOT_INLINE loft_str loft_stab_get(loft_cx *cx, const loft_stab *t, int64_t from) {
    int64_t n = (int64_t) t->n;
    int64_t f = from < 0 ? from + n : from;
    return f < 0 || f >= n ? loft_null_text(cx) : t->at[f];
}

/* `parts[i]` of a split table (`split_table_get_or_raise`): an index naming no part reports. */
static loft_str loft_stab_get_at(loft_cx *cx, const loft_stab *t, int64_t from, const char *file, uint32_t line) {
    int64_t n = (int64_t) t->n, f = from < 0 ? from + n : from;
    if (LOFT_UNLIKELY(f < 0 || f >= n)) loft_index_miss(cx, from, n, file, line);
    return loft_stab_get(cx, t, from);
}

static void loft_stab_free(loft_cx *cx, loft_stab *t) {
    loft_text_free(cx, &t->src);
    free(t->at);
    t->at = 0;
    t->n = 0;
    t->cap = 0;
}

/* OpCastFloatFromText: `parse::<f64>().unwrap_or(NAN)`. */
static double loft_float_of_text(loft_cx *cx, loft_str t) {
    if (!loft_float_text_ok(cx, t)) return (double) NAN;
    double fast;
    if (loft_float_fast(cx, t, &fast)) return fast;
    char buf[64];
    char *heap = 0, *p = buf;
    if (t.len >= sizeof buf) {
        heap = (char *) malloc(t.len + 1);
        if (!heap) { fputs("loft: out of memory\n", stderr); abort(); }
        p = heap;
    }
    memcpy(p, t.p, t.len);
    p[t.len] = 0;
    double v = strtod(p, 0);
    free(heap);
    return v;
}

/* OpCastSingleFromText: `parse::<f32>().unwrap_or(NAN)`. */
static float loft_single_of_text(loft_cx *cx, loft_str t) {
    if (!loft_float_text_ok(cx, t)) return (float) NAN;
    char buf[64];
    char *heap = 0, *p = buf;
    if (t.len >= sizeof buf) {
        heap = (char *) malloc(t.len + 1);
        if (!heap) { fputs("loft: out of memory\n", stderr); abort(); }
        p = heap;
    }
    memcpy(p, t.p, t.len);
    p[t.len] = 0;
    float v = strtof(p, 0);
    free(heap);
    return v;
}

/* ── Unicode (src/c/unicode.rs writes the tables from Rust's own `char`) ── */
typedef struct { uint32_t c; const char *m; } loft_umap;

/* Is `c` in the sorted, disjoint ranges `r`? */
static uint8_t loft_in_ranges(loft_cx *cx, uint32_t c, const uint32_t (*r)[2], size_t n) {
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if (c < r[mid][0]) hi = mid;
        else if (c > r[mid][1]) lo = mid + 1;
        else return 1;
    }
    return 0;
}

static const char *loft_umap_find(loft_cx *cx, const loft_umap *m, size_t n, uint32_t c) {
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if (c < m[mid].c) hi = mid;
        else if (c > m[mid].c) lo = mid + 1;
        else return m[mid].m;
    }
    return 0;
}

/* A letter for the final-sigma rule: one with a case. */
static int loft_cased(loft_cx *cx, uint32_t c) {
    return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= 0xC0 && c != 0xD7 && c != 0xF7);
}

/* str::to_lowercase / to_uppercase: each character's mapping from `m`, or itself; lowering a
 * capital sigma at the end of a word gives the final sigma. */
static loft_text loft_text_case(loft_cx *cx, loft_str s, const loft_umap *m, size_t n, int lower) {
    loft_text out = { 0, 0, 0 };
    if (loft_str_is_null(cx, s)) return loft_text_own(cx, s);
    size_t i = 0;
    uint32_t prev = 0;
    /* The result is about as long as the input: one reservation, not one per character. */
    loft_reserve(cx, &out, s.len);
    while (i < s.len) {
        /* An ASCII run maps by its own rule, A-Z and a-z, with no table search: every ASCII
         * letter's mapping is its other case and no other ASCII character has one. */
        if ((uint8_t) s.p[i] < 0x80) {
            size_t j = i;
            while (j < s.len && (uint8_t) s.p[j] < 0x80) j++;
            loft_reserve(cx, &out, j - i);
            char *d = out.p + out.len;
            for (size_t k = i; k < j; k++) {
                char ch = s.p[k];
                if (lower ? (ch >= 'A' && ch <= 'Z') : (ch >= 'a' && ch <= 'z')) ch ^= 0x20;
                *d++ = ch;
            }
            out.len += j - i;
            prev = (uint8_t) s.p[j - 1];
            i = j;
            continue;
        }
        size_t w;
        uint32_t c = loft_decode(cx, s, i, &w);
        const char *to = loft_umap_find(cx, m, n, c);
        if (lower && c == 0x3A3) {
            uint32_t next = 0;
            if (i + w < s.len) {
                size_t w2;
                next = loft_decode(cx, s, i + w, &w2);
            }
            to = loft_cased(cx, prev) && !loft_cased(cx, next) ? "\xCF\x82" : "\xCF\x83";
        }
        if (to) loft_push(cx, &out, to, strlen(to));
        else loft_push(cx, &out, s.p + i, w);
        prev = c;
        i += w;
    }
    if (!out.p) loft_push(cx, &out, "", 0);
    return out;
}

/* Called first by `main`: the arguments, the clock's start, the constants.  A shim has no
 * `main` (`LOFT_SHIM_UNIT`, loft_crt.h). */
#if !defined(LOFT_SHIM_UNIT)
static void loft_init(loft_cx *cx);
#endif
#if defined(_WIN32)
#include <io.h>
#include <fcntl.h>
#endif

/* ── The run's time bound (timeout.rs) ─────────────────────────────────────────────────
 * `LOFT_TIMEOUT=<seconds>` (0 or unset: none): at the deadline the program stops with 124, the
 * GNU `timeout` status the interpreter's graceful exit gives, saying so on stderr.  A POSIX host
 * arms `alarm`, whose handler does only what a signal handler may (`write`, `_exit`); Windows
 * runs a watchdog thread that sleeps until the deadline.  Elsewhere (WASI has no signals and no
 * threads here) the bound is the driver's or the wasm host's. */
static char loft_timeout_msg[96];
static size_t loft_timeout_len = 0;
static unsigned long loft_timeout_secs = 0;
#if defined(LOFT_POSIX_HOST)
#include <signal.h>
#include <unistd.h>
static void loft_timeout_fire(int sig) {
    (void) sig;
    if (write(2, loft_timeout_msg, loft_timeout_len) < 0) { /* nothing left to tell */ }
    _exit(124);
}
#elif defined(_WIN32)
#include <process.h>
__declspec(dllimport) void __stdcall Sleep(unsigned long ms);
static unsigned __stdcall loft_timeout_watch(void *arg) {
    (void) arg;
    Sleep(loft_timeout_secs * 1000ul);
    fflush(stdout);
    fwrite(loft_timeout_msg, 1, loft_timeout_len, stderr);
    fflush(stderr);
    _exit(124);
    return 0;
}
#endif

/* A rename that replaces the file `to` names, as Rust's `fs::rename` — and so the interpreter —
 * does on every host.  POSIX `rename` does; the Windows CRT's refuses an existing target, so
 * there it is `MoveFileExA` with MOVEFILE_REPLACE_EXISTING, its failure told in `errno`. */
#if defined(_WIN32)
__declspec(dllimport) int __stdcall MoveFileExA(const char *from, const char *to, unsigned long flags);
__declspec(dllimport) unsigned long __stdcall GetLastError(void);
static int loft_rename(loft_cx *cx, const char *from, const char *to) {
    if (MoveFileExA(from, to, 1ul /* MOVEFILE_REPLACE_EXISTING */)) return 0;
    switch (GetLastError()) {
    case 2: case 3: errno = ENOENT; break; /* ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND */
    case 5: case 32: errno = EACCES; break; /* ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION */
    default: errno = EIO; break;
    }
    return -1;
}
#else
#define loft_rename(cx, from, to) rename(from, to)
#endif

static void loft_arm_timeout(loft_cx *cx) {
    const char *v = getenv("LOFT_TIMEOUT");
    unsigned long secs = 0;
    if (!v || !*v) return;
    for (const char *c = v; *c; c++) {
        if (*c < '0' || *c > '9' || secs > 100000000ul) return;
        secs = secs * 10 + (unsigned long) (*c - '0');
    }
    if (!secs) return;
    loft_timeout_secs = secs;
    sprintf(loft_timeout_msg, "[timeout] deadline reached after %lus (graceful)\n", secs);
    loft_timeout_len = strlen(loft_timeout_msg);
#if defined(LOFT_POSIX_HOST)
    signal(SIGALRM, loft_timeout_fire);
    alarm((unsigned) secs);
#elif defined(_WIN32)
    _beginthreadex(0, 0, loft_timeout_watch, 0, 0, 0);
#endif
}

#if defined(LOFT_BIND_LIBS) && defined(LOFT_DL)
#if defined(LOFT_POSIX_HOST)
#include <dirent.h>
#endif
#if defined(__APPLE__)
extern int _NSGetExecutablePath(char *buf, uint32_t *size);
#endif
/* LOFT_LIB_PATH's separator, as `std::env::split_paths` reads it. */
#if defined(_WIN32)
#define LOFT_LIB_PATH_SEP ';'
#else
#define LOFT_LIB_PATH_SEP ':'
#endif

static int loft_c_lib_exists(loft_cx *cx, const char *p) {
#if defined(_WIN32)
    return _access(p, 0) == 0;
#else
    return access(p, F_OK) == 0;
#endif
}
/* The handle each row of `loft_cbl` was opened as (null for a row not opened): where a library's
 * native part finds its symbols in a shipped program (`loft_ffi_sym`). */
static void *loft_cbl_row_handle[64];

/* `native_bind::search_dirs`: LOFT_LIB_PATH's directories, the executable's, the user's
 * `.loft/lib` (LOFT_HOME, else HOME) and the system root.  The first two may hold a library
 * flat; every one may hold it in a `<package>-<version>/` directory. */
typedef struct { char path[4096]; uint8_t flat; } loft_c_libdir;
static uint32_t loft_c_lib_dirs(loft_cx *cx, loft_c_libdir *d, uint32_t max) {
    uint32_t n = 0;
    const char *list = getenv("LOFT_LIB_PATH");
    while (list && *list && n < max) {
        const char *end = strchr(list, LOFT_LIB_PATH_SEP);
        size_t len = end ? (size_t) (end - list) : strlen(list);
        if (len > 0 && len < sizeof d[n].path) {
            memcpy(d[n].path, list, len);
            d[n].path[len] = 0;
            d[n].flat = 1;
            n++;
        }
        list = end ? end + 1 : 0;
    }
    char exe[4096], real[4096];
    exe[0] = 0;
#if defined(__linux__)
    ssize_t k = readlink("/proc/self/exe", exe, sizeof exe - 1);
    exe[k > 0 ? k : 0] = 0;
#elif defined(__APPLE__)
    uint32_t size = sizeof exe;
    if (_NSGetExecutablePath(exe, &size) != 0) exe[0] = 0;
#elif defined(_WIN32)
    char *pg = 0;
    if (_get_pgmptr(&pg) == 0 && pg) snprintf(exe, sizeof exe, "%s", pg);
#endif
    if (!exe[0] && loft_argv && loft_argv[0]) snprintf(exe, sizeof exe, "%s", loft_argv[0]);
#if defined(LOFT_POSIX_HOST)
    if (exe[0] && realpath(exe, real)) snprintf(exe, sizeof exe, "%s", real);
#else
    (void) real;
#endif
    char *slash = strrchr(exe, '/');
#if defined(_WIN32)
    char *back = strrchr(exe, '\\');
    if (back && (!slash || back > slash)) slash = back;
#endif
    if (slash && n < max) {
        *slash = 0;
        snprintf(d[n].path, sizeof d[n].path, "%s", exe[0] ? exe : "/");
        d[n].flat = 1;
        n++;
    }
    const char *home = getenv("LOFT_HOME");
#if defined(_WIN32)
    if (!home) home = getenv("USERPROFILE");
#else
    if (!home) home = getenv("HOME");
#endif
    if (home && n < max) {
        snprintf(d[n].path, sizeof d[n].path, "%s/.loft/lib", home);
        d[n].flat = 0;
        n++;
    }
    /* `platform::system_library_root`. */
    if (n < max) {
#if defined(_WIN32)
        const char *pd = getenv("ProgramData");
        snprintf(d[n].path, sizeof d[n].path, "%s/loft/lib", pd && *pd ? pd : "C:/ProgramData");
#else
        snprintf(d[n].path, sizeof d[n].path, "/usr/local/lib/loft");
#endif
        d[n].flat = 0;
        n++;
    }
    return n;
}

/* `native_bind::missing_library`: the package, the version wanted, every directory searched,
 * any other version of the package those directories hold, why a file there would not load,
 * and the cure.  Exit 1, before the program runs a line. */
static void loft_c_missing_library(loft_cx *cx, const loft_c_boundlib *l, const loft_c_libdir *d, uint32_t n,
                                   const char *refused) {
    fprintf(stderr, "loft: cannot find the native library of package `%s` %s (%s)\n  searched:", l->pkg,
            l->version, l->file);
    for (uint32_t i = 0; i < n; i++) fprintf(stderr, "\n    %s", d[i].path);
    size_t plen = strlen(l->pkg);
    int header = 0;
    for (uint32_t i = 0; i < n; i++) {
#if defined(_WIN32)
        char pattern[4200];
        snprintf(pattern, sizeof pattern, "%s/*", d[i].path);
        struct _finddata_t fd;
        intptr_t dir = _findfirst(pattern, &fd);
        if (dir == -1) continue;
        do {
            const char *nm = fd.name;
#else
        DIR *dir = opendir(d[i].path);
        if (!dir) continue;
        struct dirent *e;
        while ((e = readdir(dir)) != 0) {
            const char *nm = e->d_name;
#endif
            if (strncmp(nm, l->pkg, plen) != 0 || nm[plen] != '-' || nm[plen + 1] < '0' || nm[plen + 1] > '9' ||
                strcmp(nm + plen + 1, l->version) == 0)
                continue;
            if (!header) fprintf(stderr, "\n  other versions found (not used):");
            header = 1;
            fprintf(stderr, "\n    %s/%s", d[i].path, nm);
#if defined(_WIN32)
        } while (_findnext(dir, &fd) == 0);
        _findclose(dir);
#else
        }
        closedir(dir);
#endif
    }
    if (refused[0]) fprintf(stderr, "\n  %s", refused);
    fprintf(stderr,
            "\n  fix: put `%s` beside the program, in `%s-%s/` under one of these directories, or name its "
            "directory with LOFT_LIB_PATH\n",
            l->file, l->pkg, l->version);
    LOFT_EXIT(1);
}

/* `native_bind::bind`: open every library the program was shipped with, first found in search
 * order; a library found nowhere ends the program naming it. */
static void loft_c_bind_libraries(loft_cx *cx) {
    static loft_c_libdir dirs[32];
    uint32_t nd = loft_c_lib_dirs(cx, dirs, 32);
    for (uint32_t i = 0; i < loft_ncbl; i++) {
        const loft_c_boundlib *l = &loft_cbl[i];
#if !defined(LOFT_BIND_SHIMS)
        if (l->shim) continue;
#endif
        char refused[1024], p[8400];
        refused[0] = 0;
        void *h = 0;
        for (uint32_t k = 0; k < nd && !h; k++) {
            for (int flat = 0; flat < 2 && !h; flat++) {
                if (flat && !dirs[k].flat) break;
                if (flat) snprintf(p, sizeof p, "%s/%s", dirs[k].path, l->file);
                else snprintf(p, sizeof p, "%s/%s-%s/%s", dirs[k].path, l->pkg, l->version, l->file);
                if (!loft_c_lib_exists(cx, p)) continue;
                h = loft_dl_open(cx, p, LOFT_DL_GLOBAL);
                if (!h) {
                    const char *why = loft_dl_error(cx);
                    size_t used = strlen(refused);
                    snprintf(refused + used, sizeof refused - used, "%s`%s` would not load: %s", used ? "\n  " : "", p,
                             why ? why : "unknown error");
                }
            }
        }
        if (!h) loft_c_missing_library(cx, l, dirs, nd, refused);
        if (loft_ncbl_handles < 64) loft_cbl_handles[loft_ncbl_handles++] = h;
        if (i < 64) loft_cbl_row_handle[i] = h;
    }
}
#endif

/* The keys every hash table's seed is drawn under (`loft_fresh_seed`, loft_keys.h): the
 * operating system's entropy, read ONCE, at start-up — a hash made or grown never touches
 * `/dev/urandom`.  A unit that never runs `loft_start` (a library replayed by a test) reads them
 * at its first seed instead.  Without an entropy source: the clock and an address. */
static uint64_t loft_seed_k0 = 0, loft_seed_k1 = 0;
static void loft_seed_keys(loft_cx *cx) {
    uint64_t k[2] = { 0, 0 };
    FILE *f = fopen("/dev/urandom", "rb");
    if (!f || fread(k, 1, sizeof k, f) != sizeof k) {
        k[0] = (uint64_t) time(0) ^ ((uint64_t) clock() << 20) ^ (uint64_t) (size_t) &loft_seed_k0;
        k[1] = (uint64_t) (size_t) &loft_seed_k1 * UINT64_C(0x9E3779B97F4A7C15);
    }
    if (f) fclose(f);
    loft_seed_k1 = k[1];
    loft_seed_k0 = k[0] | 1;
}

#if !defined(LOFT_SHIM_UNIT)
static void loft_start(loft_cx *cx, int argc, char **argv) {
#if defined(_WIN32)
    /* The bytes a program writes are the bytes the other backends write: no `\r\n` for `\n`. */
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
#endif
    loft_argc = argc;
    loft_argv = argv;
    loft_start_us = loft_now_us(cx);
    loft_clock_fakes(cx);
    loft_heap_read_limit(cx);
    loft_seed_keys(cx);
    loft_arm_timeout(cx);
#if defined(LOFT_POSIX_HOST)
    signal(SIGPIPE, SIG_IGN);
#endif
#if defined(LOFT_BIND_LIBS) && defined(LOFT_DL)
    loft_c_bind_libraries(cx);
#endif
    LOFT_TRACE("M\n");
    loft_init(cx);
}
#endif

/* ── Content equality (Stores::eq_content, @FR-E-Eq) ──────────────────────────────────── */

typedef int (*loft_eq_fn)(loft_cx *cx, loft_ref a, loft_ref b);

static int loft_same_place(loft_cx *cx, loft_ref a, loft_ref b) {
    return a.store_nr == b.store_nr && a.rec == b.rec && a.pos == b.pos;
}

/* The start every comparison shares: the same place is equal, two absences are equal, an
 * absence differs from a present value.  -1 when both are present. */
static int loft_eq_start(loft_cx *cx, loft_ref a, loft_ref b) {
    if (loft_same_place(cx, a, b)) return 1;
    if (a.rec == 0 || b.rec == 0) return a.rec == 0 && b.rec == 0;
    return -1;
}

static int loft_eq_long(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    return s >= 0 ? s : loft_get_int(cx, a, 0) == loft_get_int(cx, b, 0);
}

static int loft_eq_float(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    if (s >= 0) return s;
    double x = loft_get_float(cx, a, 0), y = loft_get_float(cx, b, 0);
    if (isnan(x) || isnan(y)) return isnan(x) && isnan(y);
    return x == y;
}

static int loft_eq_single(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    if (s >= 0) return s;
    float x = loft_get_single(cx, a, 0), y = loft_get_single(cx, b, 0);
    if (isnan(x) || isnan(y)) return isnan(x) && isnan(y);
    return x == y;
}

static int loft_eq_byte(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    return s >= 0 ? s : *loft_at(cx, loft_store_of(cx, a), a.rec, a.pos) == *loft_at(cx, loft_store_of(cx, b), b.rec, b.pos);
}

static int loft_eq_u16(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    return s >= 0 ? s : memcmp(loft_at(cx, loft_store_of(cx, a), a.rec, a.pos), loft_at(cx, loft_store_of(cx, b), b.rec, b.pos), 2) == 0;
}

static int loft_eq_u32(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    return s >= 0 ? s : loft_u32(cx, loft_store_of(cx, a), a.rec, a.pos) == loft_u32(cx, loft_store_of(cx, b), b.rec, b.pos);
}

static int loft_eq_dbref(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    return s >= 0 ? s : memcmp(loft_at(cx, loft_store_of(cx, a), a.rec, a.pos), loft_at(cx, loft_store_of(cx, b), b.rec, b.pos), 12) == 0;
}

static int loft_eq_text(loft_cx *cx, loft_ref a, loft_ref b) {
    int s = loft_eq_start(cx, a, b);
    if (s >= 0) return s;
    loft_str x = loft_get_text(cx, a, 0), y = loft_get_text(cx, b, 0);
    if (loft_str_is_null(cx, x) || loft_str_is_null(cx, y)) return loft_str_is_null(cx, x) && loft_str_is_null(cx, y);
    return loft_str_eq(cx, x, y);
}

/* A vector: two absent ones are equal, an absent one differs; then element by element. */
static int loft_eq_vector(loft_cx *cx, loft_ref a, loft_ref b, uint32_t size, loft_eq_fn f) {
    int s = loft_eq_start(cx, a, b);
    if (s >= 0) return s;
    int na = loft_vec_absent(cx, a), nb = loft_vec_absent(cx, b);
    if (na || nb) return na && nb;
    int64_t n = loft_vec_len(cx, a);
    if (n != loft_vec_len(cx, b)) return 0;
    for (int64_t i = 0; i < n; i++) {
        if (!f(cx, loft_vec_get(cx, a, size, i), loft_vec_get(cx, b, size, i))) return 0;
    }
    return 1;
}

/* `@FR-L-OutOfLine` (eq_content's `EqState`): the child pairs a comparison has met and not
 * yet compared; `loft_eq_content` compares them as a loop, so a chain of any length takes no
 * stack per link. */
typedef struct loft_eq_job { loft_ref a, b; loft_eq_fn f; } loft_eq_job;

static int loft_eq_child(loft_cx *cx, loft_ref a, loft_ref b, loft_eq_fn f) {
    int s = loft_eq_start(cx, a, b);
    if (s >= 0) return s;
    loft_ref ca = { a.store_nr, loft_u32(cx, loft_store_of(cx, a), a.rec, a.pos), 8, 0 };
    loft_ref cb = { b.store_nr, loft_u32(cx, loft_store_of(cx, b), b.rec, b.pos), 8, 0 };
    if (ca.rec == 0 || cb.rec == 0) return ca.rec == 0 && cb.rec == 0;
    if (cx->eq_njobs == cx->eq_jobs_cap) {
        cx->eq_jobs_cap = cx->eq_jobs_cap ? cx->eq_jobs_cap * 2 : 64;
        cx->eq_jobs = (loft_eq_job *) realloc(cx->eq_jobs, cx->eq_jobs_cap * sizeof *cx->eq_jobs);
        if (!cx->eq_jobs) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_eq_job j = { ca, cb, f };
    cx->eq_jobs[cx->eq_njobs++] = j;
    return 1;
}

/* The record pairs under comparison: a pair met again is equal, which is where a cycle ends. */

static int loft_eq_reference(loft_cx *cx, loft_ref fa, loft_ref fb, uint16_t target, loft_eq_fn f) {
    loft_ref ra = loft_get_dbref(cx, fa, 0), rb = loft_get_dbref(cx, fb, 0);
    if (ra.rec == 0 || rb.rec == 0) return ra.rec == 0 && rb.rec == 0;
    for (size_t i = 0; i < cx->eq_seen_n; i += 3) {
        if (loft_same_place(cx, cx->eq_seen[i], ra) && loft_same_place(cx, cx->eq_seen[i + 1], rb)
            && cx->eq_seen[i + 2].rec == target) return 1;
    }
    if (cx->eq_seen_n + 3 > cx->eq_seen_cap) {
        cx->eq_seen_cap = cx->eq_seen_cap ? cx->eq_seen_cap * 2 : 48;
        cx->eq_seen = (loft_ref *) realloc(cx->eq_seen, cx->eq_seen_cap * sizeof *cx->eq_seen);
        if (!cx->eq_seen) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_ref t = { 0, target, 0, 0 };
    cx->eq_seen[cx->eq_seen_n++] = ra;
    cx->eq_seen[cx->eq_seen_n++] = rb;
    cx->eq_seen[cx->eq_seen_n++] = t;
    return f(cx, ra, rb);
}

/* OpEqContent: one comparison, with a list of pairs of its own, its child pairs compared as a
 * loop (`loft_eq_child`). */
static int loft_eq_content(loft_cx *cx, loft_eq_fn f, loft_ref a, loft_ref b) {
    cx->eq_seen_n = 0;
    cx->eq_njobs = 0;
    int r = f(cx, a, b);
    while (r && cx->eq_njobs > 0) {
        loft_eq_job j = cx->eq_jobs[--cx->eq_njobs];
        r = j.f(cx, j.a, j.b);
    }
    cx->eq_njobs = 0;
    return r;
}

/* ── Kernels and conversions the stdlib calls natively ─────────────────────────────────── */

/* vector_sum_int (loop_kernels.rs, `@FR-R-BoundedNest`'s reduction clause): `acc` plus every
 * element, with integer addition's rules.  The
 * elements are read off the vector's base, a block of LOFT_SUM_BLOCK at a time (vector::
 * sum_blocks_i64): a block whose every element lies in [-2^40, 2^40) and whose total starts more
 * than 2^50 from the int64 edge cannot overflow and holds no null, so its plain sum is what the
 * checked add answers.  The first block that fails, and every element after it, take the
 * checked add, which propagates a null and an overflow as before. */
#define LOFT_SUM_BLOCK 1024
#define LOFT_SUM_BOUND (INT64_C(1) << 40)
static int64_t loft_vector_sum_int(loft_cx *cx, loft_ref v, int64_t acc) {
    int64_t n = loft_vec_len(cx, v);
    if (n <= 0) return acc;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    const uint8_t *base = loft_at(cx, s, r, 8);
    int64_t at = 0;
    while (at < n) {
        int64_t stop = at + LOFT_SUM_BLOCK < n ? at + LOFT_SUM_BLOCK : n;
        uint64_t room = acc == LOFT_NULL ? 0 : (acc < 0 ? (uint64_t) 0 - (uint64_t) acc : (uint64_t) acc);
        if (acc == LOFT_NULL || room >= (uint64_t) INT64_MAX - (uint64_t) LOFT_SUM_BLOCK * (uint64_t) LOFT_SUM_BOUND) break;
        /* Eight lane accumulators, so the vectorised body runs four independent add chains as
         * LLVM's unrolled `sum_blocks_i64` does — one accumulator is one chain, which waits on
         * itself every element pair.  Sums are modulo 2^64 and the bound test an OR: the lanes
         * fold to the same answer. */
        uint64_t hs[8] = { 0 }, ss[8] = { 0 };
        int64_t i = at;
        for (; i + 8 <= stop; i += 8) {
            for (int j = 0; j < 8; j++) {
                int64_t x;
                memcpy(&x, base + (size_t) (i + j) * 8, 8);
                hs[j] |= ((uint64_t) x + (uint64_t) LOFT_SUM_BOUND) >> 41;
                ss[j] += (uint64_t) x;
            }
        }
        uint64_t high = 0, sum = 0;
        for (int j = 0; j < 8; j++) {
            high |= hs[j];
            sum += ss[j];
        }
        for (; i < stop; i++) {
            int64_t x;
            memcpy(&x, base + (size_t) i * 8, 8);
            high |= ((uint64_t) x + (uint64_t) LOFT_SUM_BOUND) >> 41;
            sum += (uint64_t) x;
        }
        if (high != 0) break;
        acc = (int64_t) ((uint64_t) acc + sum);
        at = stop;
    }
    for (int64_t i = at; i < n; i++) {
        int64_t x;
        memcpy(&x, base + (size_t) i * 8, 8);
        acc = loft_add(cx, acc, x);
    }
    return acc;
}

/* A `char *` a `#c` binding returns, as `c_call` reads it: NULL is the null text, the bytes end
 * at the first NUL, and an invalid UTF-8 byte is U+FFFD (`String::from_utf8_lossy`). */
static loft_text loft_text_from_cstr(loft_cx *cx, const char *c) {
    loft_text t = { 0, 0, 0 };
    if (!c) {
        loft_push_char(cx, &t, 0);
        return t;
    }
    size_t n = strlen(c);
    const unsigned char *p = (const unsigned char *) c;
    if (loft_utf8_valid(cx, p, n)) {
        loft_push(cx, &t, c, n);
        return t;
    }
    for (size_t i = 0; i < n;) {
        size_t len = p[i] < 0x80 ? 1 : (p[i] >= 0xC2 && p[i] <= 0xDF) ? 2
                   : (p[i] >= 0xE0 && p[i] <= 0xEF) ? 3 : (p[i] >= 0xF0 && p[i] <= 0xF4) ? 4 : 0;
        if (len && i + len <= n && loft_utf8_valid(cx, p + i, len)) {
            loft_push(cx, &t, c + i, len);
            i += len;
        } else {
            loft_push(cx, &t, "\xEF\xBF\xBD", 3);
            i++;
        }
    }
    return t;
}

/* text_from_bytes (Stores::text_from_bytes_native): the bytes of a vector<u8> as a text; bytes
 * that are not UTF-8 give the empty text. */
static loft_text loft_text_from_bytes(loft_cx *cx, loft_ref v) {
    loft_text out = { 0, 0, 0 };
    int64_t n = loft_vec_len(cx, v);
    if (n == 0) return out;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return out;
    const unsigned char *p = loft_at(cx, s, r, 8);
    if (loft_utf8_valid(cx, p, (size_t) n)) loft_push(cx, &out, (const char *) p, (size_t) n);
    return out;
}

/* text_from_byte_range (Stores::text_from_bytes_range): the bytes [lo, hi) of a vector<u8> as a
 * text, a negative bound counting from the end as `bytes[lo..hi]` does (@FR-Slice-Value); bytes
 * that are not UTF-8, or an empty range, give the empty text. */
static loft_text loft_text_from_byte_range(loft_cx *cx, loft_ref v, int64_t lo, int64_t hi) {
    loft_text out = { 0, 0, 0 };
    int64_t n = loft_vec_len(cx, v);
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0 || n == 0) return out;
    lo = lo < 0 ? (lo + n < 0 ? 0 : lo + n) : (lo > n ? n : lo);
    hi = hi < 0 ? (hi + n < 0 ? 0 : hi + n) : (hi > n ? n : hi);
    if (hi <= lo) return out;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return out;
    const unsigned char *p = loft_elem_base(cx, s, r) + lo;
    if (loft_utf8_valid(cx, p, (size_t) (hi - lo))) loft_push(cx, &out, (const char *) p, (size_t) (hi - lo));
    return out;
}

/* `@FR-R-Transient`: the scalar elements of store vector `src` copied into transient vector `d`,
 * each `size` bytes. */
static void loft_ta_vec_copy(loft_cx *cx, loft_ta_vec *d, loft_ref src, uint32_t size) {
    int64_t n = loft_vec_len(cx, src);
    d->p = 0;
    d->n = d->cap = 0;
    if (src.store_nr == 0xFFFF || src.rec == 0 || src.pos == 0 || n <= 0) return;
    loft_store_t *s = loft_store_of(cx, src);
    uint32_t r = loft_collection_rec(cx, s, src.rec, src.pos);
    if (r == 0) return;
    d->p = loft_ta_alloc(cx, (size_t) n * size);
    memcpy(d->p, loft_elem_base(cx, s, r), (size_t) n * size);
    d->n = d->cap = (uint32_t) n;
}

/* `@FR-R-SliceInto`'s transient clause: the elements [lo, hi) of `src`, clamped to it as
 * loft_vec_slice clamps them, copied into `d` — what the slice's temporary and the copy of it
 * made together. */
static void loft_ta_vec_copy_range(loft_cx *cx, loft_ta_vec *d, loft_ref src, int64_t lo, int64_t hi, uint32_t size) {
    int64_t n = loft_vec_len(cx, src);
    d->p = 0;
    d->n = d->cap = 0;
    if (src.store_nr == 0xFFFF || src.rec == 0 || src.pos == 0 || n <= 0) return;
    if (lo < 0) lo = 0;
    if (lo > n) lo = n;
    if (hi < lo) hi = lo;
    if (hi > n) hi = n;
    if (hi == lo) return;
    loft_store_t *s = loft_store_of(cx, src);
    uint32_t r = loft_collection_rec(cx, s, src.rec, src.pos);
    if (r == 0) return;
    d->p = loft_ta_alloc(cx, (size_t) (hi - lo) * size);
    memcpy(d->p, loft_elem_base(cx, s, r) + (size_t) lo * size, (size_t) (hi - lo) * size);
    d->n = d->cap = (uint32_t) (hi - lo);
}

/* OpSliceVector (Stores::vector_slice): the elements [lo, hi) of `o`, clamped to it, appended to
 * `v` as ONE block — into the room `v` has when it fits (`append_block_fits`), else after one
 * growth to the total, the source re-read from its slot when it shares `v`'s store (the growth
 * may move it, and does when it IS `v`: `v += v[a..b]`).  The span lies below the source's
 * length and the new slots above the destination's, so they never overlap.  Scalar elements
 * only. */
static void loft_vec_slice(loft_cx *cx, loft_ref v, loft_ref o, int64_t lo, int64_t hi, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || o.store_nr == 0xFFFF || o.rec == 0 || o.pos == 0) return;
    int64_t n64 = loft_vec_len(cx, o);
    if (lo < 0) lo = 0;
    if (lo > n64) lo = n64;
    if (hi < lo) hi = lo;
    if (hi > n64) hi = n64;
    uint32_t n = (uint32_t) (hi - lo);
    if (n == 0 || size == 0) return;
    uint32_t o_rec = loft_collection_rec(cx, loft_store_of(cx, o), o.rec, o.pos);
    if (o_rec == 0) return;
    uint32_t from = 8 + (uint32_t) lo * size;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t d_rec = loft_collection_rec(cx, s, v.rec, v.pos);
    if (d_rec != 0 && d_rec != LOFT_FOREIGN_REC && !s->read_only) {
        int32_t claimed = loft_st_i32(cx, s, d_rec, 0);
        uint32_t len = loft_u32(cx, s, d_rec, 4);
        uint64_t cap = claimed > 0 ? ((uint64_t) claimed * 8 - 8) / size : 0;
        if ((uint64_t) len + n <= cap) {
            loft_ref dst = { v.store_nr, d_rec, 0, 0 }, src = { o.store_nr, o_rec, 0, 0 };
            loft_copy_bytes(cx, dst, 8 + len * size, src, from, (size_t) n * size);
            loft_set_u32(cx, s, d_rec, 4, len + n);
            return;
        }
    }
    uint32_t at = loft_vec_append(cx, v, size).pos;
    loft_vec_set_size(cx, v, n, size);
    uint32_t dest = loft_u32(cx, loft_store_of(cx, v), v.rec, v.pos);
    if (v.store_nr == o.store_nr) o_rec = loft_collection_rec(cx, loft_store_of(cx, o), o.rec, o.pos);
    loft_ref dst = { v.store_nr, dest, 0, 0 }, src = { o.store_nr, o_rec, 0, 0 };
    loft_copy_bytes(cx, dst, at, src, from, (size_t) n * size);
}

/* OpSliceView (Stores::vector_slice_view): `v = o[lo..hi]` bound to a local that owns its store —
 * when `o` serves foreign bytes the local's store becomes a VIEW of the span, no copy, the span
 * sharing the owner; otherwise the copy loft_vec_slice makes.  `-DLOFT_NO_FOREIGN_VIEW` is the
 * interpreter's LOFT_NO_FOREIGN_VIEW: the copy everywhere. */
static void loft_vec_slice_view(loft_cx *cx, loft_ref v, loft_ref o, int64_t lo, int64_t hi, uint32_t size) {
#if defined(LOFT_FOREIGN) && !defined(LOFT_NO_FOREIGN_VIEW)
    if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0 && o.store_nr != 0xFFFF && o.rec != 0
        && o.pos != 0 && v.store_nr != o.store_nr) {
        int64_t n = loft_vec_len(cx, o);
        int64_t l = lo < 0 ? 0 : lo > n ? n : lo;
        int64_t h = hi < l ? l : hi > n ? n : hi;
        if (h > l && loft_st_is_foreign(cx, loft_store_of(cx, o))) {
            loft_fspan span = loft_st_foreign_span(cx, loft_store_of(cx, o), (uint32_t) l, (uint32_t) h);
            loft_store_t *s = loft_store_of(cx, v);
            loft_st_release_foreign(cx, s);
            /* A locked destination takes no view (`Stores::vector_slice_view`): the copy below
             * meets the write refusal.  STORE_MODEL.md D19g. */
            if (!s->read_only) {
                loft_st_make_foreign(cx, s, v.rec, v.pos, span);
                return;
            }
        }
    }
#endif
    loft_vec_slice(cx, v, o, lo, hi, size);
}

/* OpPlaceRecord (Stores::place_record_prefilled): a `tp` record claimed inside the host's store,
 * prefilled with its defaults, or in a fresh store of its own when there is no host.  `foreign`:
 * the host's root is a vector of another element type, so the record is noted
 * (`@FR-H-RootExtent`). */
static loft_ref loft_place_record(loft_cx *cx, loft_ref host, uint16_t tp, uint32_t size, const uint8_t *image, uint32_t n,
                                  int foreign) {
    loft_ref r = {0};
    if (host.store_nr == 0xFFFF || host.rec == 0 || host.store_nr >= cx->nstores) {
        r = loft_ref_null(cx);
        loft_database(cx, &r, size);
    } else {
        r.store_nr = host.store_nr;
        r.rec = loft_claim(cx, loft_store_of(cx, host), 1 + (size + 7) / 8);
        r.pos = 8;
        if (foreign) loft_st_note_placed(cx, loft_store_of(cx, r), r.rec);
    }
    loft_set_u32(cx, loft_store_of(cx, r), r.rec, 4, tp);
    loft_fill(cx, r, image, n);
    return r;
}

/* OpEvacuate (Stores::evacuate, `@FR-H-RefHome`): the store `home` names is about to go; when
 * the stored reference at byte `fld` of `holder` names a record of it, the record is copied as
 * a `tp` record into the holder's own store and the reference names the copy. */
static void loft_evacuate(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref holder, uint32_t fld, loft_ref home,
                          uint32_t size, const uint8_t *image, uint32_t n, int foreign) {
    if (holder.rec == 0 || holder.store_nr == 0xFFFF || home.store_nr == 0xFFFF
        || holder.store_nr == home.store_nr || holder.store_nr >= cx->nstores)
        return;
    loft_ref old = loft_get_dbref(cx, holder, fld);
    if (old.rec == 0 || old.store_nr != home.store_nr) return;
    loft_ref copy = loft_ref_null(cx);
    for (uint32_t i = 0; i < cx->nevacs; i++) {
        loft_evac *e = &cx->evacs[i];
        if (e->old.store_nr == old.store_nr && e->old.rec == old.rec && e->old.pos == old.pos
            && e->holder == holder.store_nr) {
            copy = e->copy;
            break;
        }
    }
    if (copy.rec == 0) {
        copy = loft_place_record(cx, holder, tp, size, image, n, foreign);
        loft_remove_claims(cx, T, tp, copy);
        loft_copy_bytes(cx, copy, 0, old, 0, T[tp].size);
        loft_copy_claims(cx, T, tp, old, copy);
        if (cx->nevacs == cx->evac_cap) {
            cx->evac_cap = cx->evac_cap ? cx->evac_cap * 2 : 8;
            cx->evacs = (loft_evac *) realloc(cx->evacs, cx->evac_cap * sizeof *cx->evacs);
            if (!cx->evacs) { fputs("loft: out of memory\n", stderr); abort(); }
        }
        cx->evacs[cx->nevacs].old = old;
        cx->evacs[cx->nevacs].holder = holder.store_nr;
        cx->evacs[cx->nevacs].copy = copy;
        cx->nevacs++;
    }
    loft_set_dbref(cx, holder, fld, copy);
}

/* OpFreeRecordIn (Stores::free_record_in): a store's root record frees the store; a placed
 * record releases what it owns and its block goes back to the host's store (`@FR-R-Place`). */
static void loft_free_record_in(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r) {
    if (r.store_nr == 0xFFFF || r.rec == 0 || r.store_nr >= cx->nstores) return;
    if (r.rec == 1 && r.pos == 8) {
        loft_free_ref(cx, r);
        return;
    }
    loft_remove_claims(cx, T, tp, r);
    loft_st_delete(cx, &cx->stores[r.store_nr], r.rec);
}

/* OpMoveRecord (Stores::move_record_out, `@FR-R-MoveLast`): the last use of an owned record
 * stored into a field.  Within one store the bytes relocate — the heap handles keep their
 * claims — and the source's block goes; across stores it is a deep copy and the source's
 * release. */
static void loft_move_record_out(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    if (src.store_nr >= cx->nstores || dst.store_nr >= cx->nstores) return;
    loft_remove_claims(cx, T, tp, dst);
    loft_copy_bytes(cx, dst, 0, src, 0, T[tp].size);
    if (src.store_nr != dst.store_nr) {
        loft_copy_claims(cx, T, tp, src, dst);
        loft_free_record_in(cx, T, tp, src);
        return;
    }
    loft_st_delete(cx, &cx->stores[src.store_nr], src.rec);
}

/* OpClear with `@FR-R-Reuse`'s reset flag (Stores::reset_pool_root): a pooled call buffer at its
 * store's root, the store minted for it, is made again as OpDatabase mints it and prefilled
 * with its type's defaults; answers 0, and the caller walks, for any other buffer.
 * `-DLOFT_NO_POOL_RESET` is the interpreter's LOFT_NO_POOL_RESET. */
static int loft_reset_pool_root(loft_cx *cx, loft_ref r, uint16_t tp, uint32_t size, const uint8_t *image, uint32_t n) {
#ifdef LOFT_NO_POOL_RESET
    (void) r; (void) tp; (void) size; (void) image; (void) n;
    return 0;
#else
    if (r.store_nr == 0xFFFF || r.rec != 1 || r.pos != 8 || r.store_nr >= cx->nstores) return 0;
    loft_store_t *s = &cx->stores[r.store_nr];
    if (!s->live || s->pinned || LOFT_KNOWN(*s) != tp || s->frame_holds) return 0;
    /* A locked store is not re-minted, nor one bound to a file or serving foreign bytes: the
     * caller walks. */
    if (s->read_only || s->mapped) return 0;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(cx, s)) return 0;
#endif
    loft_database_typed(cx, &r, size, tp);
    loft_fill(cx, r, image, n);
    return 1;
#endif
}

/* `@FR-E-Report` — the program's operations that can meet an uncomputable value, each the
 * plain helper and, where the interpreter reports, one Warn record at the statement (its
 * guarded `*Nullable` twin keeps the plain helper and says nothing).  Each tests the plain
 * helper's own answer, so the in-range path is the helper's. */
#define LOFT_DIV0 "[divide_by_zero] divide by zero"
LOFT_HOT_INLINE int64_t loft_div_at(loft_cx *cx, int64_t a, int64_t b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return loft_div(cx, a, b);
}
LOFT_HOT_INLINE int64_t loft_rem_at(loft_cx *cx, int64_t a, int64_t b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return loft_rem(cx, a, b);
}
LOFT_HOT_INLINE int32_t loft_div32_at(loft_cx *cx, int32_t a, int32_t b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return loft_div32(cx, a, b);
}
LOFT_HOT_INLINE int32_t loft_rem32_at(loft_cx *cx, int32_t a, int32_t b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return loft_rem32(cx, a, b);
}
/* A float division by zero answers the IEEE value and reports, the null operand too. */
LOFT_HOT_INLINE double loft_fdiv_at(loft_cx *cx, double a, double b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0.0)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return a / b;
}
LOFT_HOT_INLINE double loft_frem_at(loft_cx *cx, double a, double b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0.0)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return fmod(a, b);
}
LOFT_HOT_INLINE float loft_sdiv_at(loft_cx *cx, float a, float b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0.0f)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return a / b;
}
LOFT_HOT_INLINE float loft_srem_at(loft_cx *cx, float a, float b, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(b == 0.0f)) loft_fault_report(cx, LOFT_DIV0, file, line);
    return fmodf(a, b);
}
#undef LOFT_DIV0

/* A shift whose amount is outside 0..63, or whose result is the null: reported unless an
 * operand was null. */
#define LOFT_SHIFT_AT(name)                                                                     \
    LOFT_HOT_INLINE int64_t name##_at(loft_cx *cx, int64_t a, int64_t b, const char *file, uint32_t line) {  \
        int64_t r = name(cx, a, b);                                                             \
        if (LOFT_UNLIKELY(r == LOFT_NULL) && a != LOFT_NULL && b != LOFT_NULL)                  \
            loft_fault_report(cx, "[shift_out_of_range] shift amount out of range [0,64) or result " \
                              "is the reserved null value", file, line);                        \
        return r;                                                                               \
    }
LOFT_SHIFT_AT(loft_shl)
LOFT_SHIFT_AT(loft_shr)
#undef LOFT_SHIFT_AT

#define LOFT_CAST_RANGE "[cast_out_of_range] cast value cannot be represented in the target type"
/* A float cast to an integer that answers null from a number (NaN is the null float). */
static int64_t loft_f2i_at(loft_cx *cx, double v, const char *file, uint32_t line) {
    int64_t r = loft_f2i(cx, v);
    if (LOFT_UNLIKELY(r == LOFT_NULL) && !isnan(v)) loft_fault_report(cx, LOFT_CAST_RANGE, file, line);
    return r;
}

/* @PLN217 — a single cast to an integer held in 32 bits, with no double on the way: NaN is
 * null; a number outside an `i32` is null, and outside an `i64` it is reported first, as
 * `loft_f2i_at` reports it; else truncated toward zero.  The answer of `loft_f2i_at` narrowed. */
static int32_t loft_s2i32_at(loft_cx *cx, float v, const char *file, uint32_t line) {
    if (isnan(v)) return INT32_MIN;
    if (!(v > -2147483648.0f && v < 2147483648.0f)) {
        if (!(v >= -9223372036854775808.0f && v < 9223372036854775808.0f)) {
            loft_fault_report(cx, LOFT_CAST_RANGE, file, line);
        }
        return INT32_MIN;
    }
    return (int32_t) v;
}

/* An integer cast to a character that names no code point (`char::from_u32` of its low 32
 * bits); the null integer is the null character. */
static uint32_t loft_char_of_at(loft_cx *cx, int64_t v, const char *file, uint32_t line) {
    uint32_t u = (uint32_t) (uint64_t) v;
    if (v != LOFT_NULL && (u > 0x10FFFF || (u >= 0xD800 && u <= 0xDFFF)))
        loft_fault_report(cx, LOFT_CAST_RANGE, file, line);
    return loft_char_of(cx, v);
}

/* A text parsed as an integer that names the null itself (`-9223372036854775808`, leading
 * zeros allowed): it reads null, and is reported. */
static int64_t loft_int_of_text_at(loft_cx *cx, loft_str t, const char *file, uint32_t line) {
    static const char min[] = "9223372036854775808";
    int64_t r = loft_int_of_text(cx, t);
    if (r == LOFT_NULL && t.len > 1 && t.p[0] == '-') {
        size_t i = 1;
        while (i + 1 < t.len && t.p[i] == '0') i++;
        if (t.len - i == sizeof min - 1 && memcmp(t.p + i, min, sizeof min - 1) == 0)
            loft_fault_report(cx, LOFT_CAST_RANGE, file, line);
    }
    return r;
}
#undef LOFT_CAST_RANGE

/* A value outside a narrow slot's declared range takes the slot's default, reported. */
static int64_t loft_range_default_at(loft_cx *cx, int64_t v, int64_t lo, int64_t hi, int64_t dflt, const char *file,
                                     uint32_t line) {
    if (v != LOFT_NULL && (v < lo || v > hi))
        loft_fault_reportf(cx, file, line,
                           "[range_defaulted] value %lld is outside the declared range %lld..=%lld, so "
                           "the slot took its default instead",
                           (long long) v, (long long) lo, (long long) hi);
    return loft_range_default(cx, v, lo, hi, dflt);
}

/* `v[i]` (`State::vec_get_or_raise`): an index naming no element is reported, and its null is
 * the REPORTED one, so a write through it adds nothing (`loft_write_dropped`). */
LOFT_HOT_INLINE loft_ref loft_vec_get_at(loft_cx *cx, loft_ref v, uint32_t size, int64_t from, const char *file,
                                         uint32_t line) {
    loft_ref e = loft_vec_get(cx, v, size, from);
    if (LOFT_UNLIKELY(e.rec == 0)) {
        loft_index_miss(cx, from, loft_vec_len(cx, v), file, line);
        return loft_ref_null_reported(cx);
    }
    return e;
}

/* @PLN217 — `loft_vec_get` at an index held in 32 bits: the wrap from the end and the one
 * unsigned compare are 32-bit arithmetic.  A null index is the most negative one, and wraps to
 * past the end of any vector an `i32` can count; a longer one takes the 64-bit path. */
LOFT_HOT_INLINE loft_ref loft_vec_get32(loft_cx *cx, loft_ref v, uint32_t size, int32_t from) {
    loft_ref e = loft_ref_null(cx);
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return e;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t r = loft_collection_rec(cx, s, v.rec, v.pos);
    if (r == 0) return e;
    uint32_t len = loft_u32(cx, s, r, 4);
    if (LOFT_UNLIKELY(len > (uint32_t) INT32_MAX)) return loft_vec_get(cx, v, size, loft_w64(cx, from));
    uint32_t f = from < 0 ? (uint32_t) from + len : (uint32_t) from;
    if (f >= len) return e;
    e.store_nr = v.store_nr;
    e.rec = r;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}
LOFT_HOT_INLINE loft_ref loft_vec_get32_at(loft_cx *cx, loft_ref v, uint32_t size, int32_t from, const char *file,
                                           uint32_t line) {
    loft_ref e = loft_vec_get32(cx, v, size, from);
    if (LOFT_UNLIKELY(e.rec == 0)) {
        loft_index_miss(cx, loft_w64(cx, from), loft_vec_len(cx, v), file, line);
        return loft_ref_null_reported(cx);
    }
    return e;
}
LOFT_HOT_INLINE loft_ref loft_vh_get_at(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, const char *file, uint32_t line) {
    loft_int f = LOFT_VH_WRAP(h, from);
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, from, h.len, file, line);
        return loft_ref_null_reported(cx);
    }
    loft_ref e = {0};
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}
LOFT_HOT_INLINE uint8_t *loft_vh_addr_at(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, const char *file, uint32_t line) {
    loft_int f = LOFT_VH_WRAP(h, from);
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, from, h.len, file, line);
        return (uint8_t *) 0;
    }
    return h.base + (size_t) f * size;
}
LOFT_HOT_INLINE int64_t loft_vh_get_int_at(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, const char *file,
                                           uint32_t line) {
    loft_int f = LOFT_VH_WRAP(h, from);
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, from, h.len, file, line);
        return LOFT_NULL;
    }
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE double loft_vh_get_float_at(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, const char *file,
                                            uint32_t line) {
    loft_int f = LOFT_VH_WRAP(h, from);
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, from, h.len, file, line);
        return (double) NAN;
    }
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE void loft_vh_set_int_at(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v,
                                        const char *file, uint32_t line) {
    loft_int f = LOFT_VH_WRAP(h, from);
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, from, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}
LOFT_HOT_INLINE void loft_vh_set_float_at(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, double v,
                                          const char *file, uint32_t line) {
    loft_int f = LOFT_VH_WRAP(h, from);
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, from, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

/* `@FR-R-Range`'s index clause: the held accessors for an index the range proof shows
 * non-negative — one unsigned compare, no from-the-end wrap, the miss branched to directly (a
 * helper answering -1 and a second test kept both branches).  A miss is reported and answers as
 * the wrapping accessors do. */
LOFT_HOT_INLINE loft_ref loft_vh_get_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f) {
    if (LOFT_VH_OUT(h, f)) return loft_ref_null(cx);
    loft_ref e = {0};
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}
LOFT_HOT_INLINE loft_ref loft_vh_get_at_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, const char *file, uint32_t line) {
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, f, h.len, file, line);
        return loft_ref_null_reported(cx);
    }
    loft_ref e = {0};
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}
LOFT_HOT_INLINE uint8_t *loft_vh_addr_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f) {
    return LOFT_VH_OUT(h, f) ? (uint8_t *) 0 : h.base + (size_t) f * size;
}
LOFT_HOT_INLINE uint8_t *loft_vh_addr_at_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, const char *file, uint32_t line) {
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, f, h.len, file, line);
        return (uint8_t *) 0;
    }
    return h.base + (size_t) f * size;
}
LOFT_HOT_INLINE int64_t loft_vh_get_int_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld) {
    if (LOFT_VH_OUT(h, f)) return LOFT_NULL;
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE int64_t loft_vh_get_int_at_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, const char *file,
                                              uint32_t line) {
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, f, h.len, file, line);
        return LOFT_NULL;
    }
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE double loft_vh_get_float_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld) {
    if (LOFT_VH_OUT(h, f)) return (double) NAN;
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE double loft_vh_get_float_at_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, const char *file,
                                               uint32_t line) {
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, f, h.len, file, line);
        return (double) NAN;
    }
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE void loft_vh_set_int_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, int64_t v) {
    if (LOFT_VH_OUT(h, f)) return;
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}
LOFT_HOT_INLINE void loft_vh_set_int_at_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, int64_t v,
                                           const char *file, uint32_t line) {
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, f, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}
LOFT_HOT_INLINE void loft_vh_set_float_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, double v) {
    if (LOFT_VH_OUT(h, f)) return;
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}
LOFT_HOT_INLINE void loft_vh_set_float_at_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, double v,
                                             const char *file, uint32_t line) {
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, f, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

/* `@FR-R-Base`'s guard clause: the held accessors for an index a passed test keeps below the
 * length, its sign unproved — the from-the-end wrap, then one sign test (an index below
 * `-len` names no element); the end test is the guard's, proved again under `LOFT_VERIFY`.  A
 * miss answers and reports as the wrapping accessors do. */
#define LOFT_VH_LT(h, from, f) \
    loft_int f = LOFT_VH_WRAP(h, from); \
    LOFT_PROVED(f < (h).len, "a guarded index")
LOFT_HOT_INLINE loft_ref loft_vh_get_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) return loft_ref_null(cx);
    loft_ref e = {0};
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}
LOFT_HOT_INLINE loft_ref loft_vh_get_at_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, const char *file, uint32_t line) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) {
        loft_index_miss(cx, from, h.len, file, line);
        return loft_ref_null_reported(cx);
    }
    loft_ref e = {0};
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}
LOFT_HOT_INLINE uint8_t *loft_vh_addr_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from) {
    LOFT_VH_LT(h, from, f);
    return LOFT_UNLIKELY(f < 0) ? (uint8_t *) 0 : h.base + (size_t) f * size;
}
LOFT_HOT_INLINE uint8_t *loft_vh_addr_at_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, const char *file, uint32_t line) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) {
        loft_index_miss(cx, from, h.len, file, line);
        return (uint8_t *) 0;
    }
    return h.base + (size_t) f * size;
}
LOFT_HOT_INLINE int64_t loft_vh_get_int_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) return LOFT_NULL;
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE int64_t loft_vh_get_int_at_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, const char *file,
                                              uint32_t line) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) {
        loft_index_miss(cx, from, h.len, file, line);
        return LOFT_NULL;
    }
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE double loft_vh_get_float_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) return (double) NAN;
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE double loft_vh_get_float_at_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, const char *file,
                                               uint32_t line) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) {
        loft_index_miss(cx, from, h.len, file, line);
        return (double) NAN;
    }
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}
LOFT_HOT_INLINE void loft_vh_set_int_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) return;
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}
LOFT_HOT_INLINE void loft_vh_set_int_at_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v,
                                           const char *file, uint32_t line) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) {
        loft_index_miss(cx, from, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}
LOFT_HOT_INLINE void loft_vh_set_float_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, double v) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) return;
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}
LOFT_HOT_INLINE void loft_vh_set_float_at_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, double v,
                                             const char *file, uint32_t line) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) {
        loft_index_miss(cx, from, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

/* The same held-header element writes for a 4-byte unsigned slot (`OpSetInt4Raw`: `u32`, and
 * `u32?` with 0xFFFFFFFF for null), one family with the 8-byte ones above: each index form
 * answers a miss exactly as its `int` twin, and the value is encoded as `loft_set_int4_raw`
 * encodes it. */
LOFT_HOT_INLINE void loft_vh_store_u32(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, int64_t v) {
    uint32_t x = v == LOFT_NULL ? 0xFFFFFFFFu : (uint32_t) (uint64_t) v;
    memcpy(h.base + (size_t) f * size + fld, &x, 4);
}
LOFT_HOT_INLINE void loft_vh_put_u32(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, int64_t v) {
    LOFT_PROVED(f >= 0 && f < h.len, "a proven index: put u32");
    LOFT_VH_WRITABLE(h);
    loft_vh_store_u32(cx, h, size, f, fld, v);
}
LOFT_HOT_INLINE void loft_vh_set_u32(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v) {
    loft_int f = loft_vh_index(cx, h, from);
    if (f < 0) return;
    LOFT_VH_WRITABLE(h);
    loft_vh_store_u32(cx, h, size, f, fld, v);
}
LOFT_HOT_INLINE void loft_vh_set_u32_at(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v,
                                        const char *file, uint32_t line) {
    loft_int f = LOFT_VH_WRAP(h, from);
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, from, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    loft_vh_store_u32(cx, h, size, f, fld, v);
}
LOFT_HOT_INLINE void loft_vh_set_u32_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, int64_t v) {
    if (LOFT_VH_OUT(h, f)) return;
    LOFT_VH_WRITABLE(h);
    loft_vh_store_u32(cx, h, size, f, fld, v);
}
LOFT_HOT_INLINE void loft_vh_set_u32_at_nn(loft_cx *cx, loft_vh h, uint32_t size, loft_int f, uint32_t fld, int64_t v,
                                           const char *file, uint32_t line) {
    if (LOFT_VH_OUT(h, f)) {
        loft_index_miss(cx, f, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    loft_vh_store_u32(cx, h, size, f, fld, v);
}
LOFT_HOT_INLINE void loft_vh_set_u32_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) return;
    LOFT_VH_WRITABLE(h);
    loft_vh_store_u32(cx, h, size, f, fld, v);
}
LOFT_HOT_INLINE void loft_vh_set_u32_at_lt(loft_cx *cx, loft_vh h, uint32_t size, loft_int from, uint32_t fld, int64_t v,
                                           const char *file, uint32_t line) {
    LOFT_VH_LT(h, from, f);
    if (LOFT_UNLIKELY(f < 0)) {
        loft_index_miss(cx, from, h.len, file, line);
        return;
    }
    LOFT_VH_WRITABLE(h);
    loft_vh_store_u32(cx, h, size, f, fld, v);
}

/* `t[i]` (`State::text_char_or_raise`): a byte index naming no byte of the text. */
LOFT_HOT_INLINE uint32_t loft_text_char_at(loft_cx *cx, loft_str t, int64_t from, const char *file, uint32_t line) {
    int64_t len = (int64_t) t.len, i = from < 0 ? from + len : from;
    if (LOFT_UNLIKELY(i < 0 || i >= len)) loft_index_miss(cx, from, len, file, line);
    return loft_text_char(cx, t, from);
}
