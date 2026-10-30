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
#ifndef LOFT_NO_JUMPS
#include <setjmp.h>
#define LOFT_JMP_BUF jmp_buf
#define LOFT_LONGJMP(b) longjmp(b, 1)
#else
/* A unit that runs no test and binds no lazy collection never jumps (`LOFT_NO_JUMPS`, a browser
 * page's module): wasm has setjmp only through its exception-handling proposal, which asyncify
 * cannot carry, so the unit holds no jump buffer at all and these sites are unreachable. */
typedef struct { int unused; } loft_no_jmp_buf;
#define LOFT_JMP_BUF loft_no_jmp_buf
#define LOFT_LONGJMP(b) abort()
#endif
#include <errno.h>
#include <math.h>
#include <float.h>
#include <time.h>

#define LOFT_NULL INT64_MIN

#define LOFT_BOOL_NULL 255

static double loft_usage_pct(uint64_t a, uint64_t b) {
    return b == 0 ? 0.0 : 100.0 * (double) a / (double) b;
}

/* `Stores::memory_report` (`store_memory()`): every live store's space, then the eight largest
 * by capacity (ties in store order) with their type's name from `names`; a compiled program
 * has no bytecode positions, so the creation site reads `bc:0` as on `--native`. */
static loft_text loft_store_memory(const char *const *names, uint32_t nnames) {
    uint32_t live = 0;
    uint64_t cap = 0, data = 0, fr = 0, recs = 0, free_blk = 0, mergeable = 0;
    uint32_t *rows = (uint32_t *) malloc(sizeof(uint32_t) * (loft_nstores ? loft_nstores : 1));
    loft_usage *us = (loft_usage *) malloc(sizeof(loft_usage) * (loft_nstores ? loft_nstores : 1));
    if (!rows || !us) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint32_t i = 0; i < loft_nstores; i++) {
        if (!loft_stores[i].live) continue;
        loft_usage u = loft_st_usage(&loft_stores[i]);
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
        live, LOFT_MB(cap), LOFT_MB(data), loft_usage_pct(data, cap), LOFT_MB(fr),
        (unsigned long long) recs, (unsigned long long) free_blk, (unsigned long long) mergeable);
    loft_push(&out, line, (size_t) n);
    for (uint32_t k = 0; k < live && k < 8; k++) {
        uint32_t i = rows[k];
        loft_usage u = us[i];
        uint16_t kt = loft_stores[i].known;
        const char *tname = kt < nnames && names[kt] ? names[kt] : "?";
        char idx[16];
        snprintf(idx, sizeof idx, "%u", i);
        n = snprintf(line, sizeof line,
            "\n  #%-5s %8.3f MB  used %3.0f%%  recs %-5u free-blk %-4u mergeable %-4u largest-free %-7uw  tail %3.0f%% inner %3.0f%%  type %s bc:0",
            idx, LOFT_MB(u.capacity_words), loft_usage_pct(u.claimed_words, u.capacity_words),
            u.claimed_count, u.free_count, u.mergeable_free_pairs, u.largest_free_words,
            loft_usage_pct(u.capacity_words > u.live_end_words ? u.capacity_words - u.live_end_words : 0, u.capacity_words),
            loft_usage_pct(u.live_end_words > u.claimed_words ? u.live_end_words - u.claimed_words : 0, u.capacity_words),
            tname);
        loft_push(&out, line, (size_t) n < sizeof line ? (size_t) n : sizeof line - 1);
    }
#undef LOFT_MB
    free(rows);
    free(us);
    return out;
}

/* The order of two texts, byte by byte, a shorter prefix first (Rust's `str` order). */
static int loft_str_cmp(loft_str a, loft_str b) {
    size_t n = a.len < b.len ? a.len : b.len;
    int c = n ? memcmp(a.p, b.p, n) : 0;
    if (c != 0) return c;
    return a.len < b.len ? -1 : a.len > b.len;
}

/* OpLtText / OpLeText (ops::op_lt_text): the null text orders before every present one, and
 * nothing present orders before it. */
static int loft_text_lt(loft_str a, loft_str b) {
    int an = loft_str_is_null(a), bn = loft_str_is_null(b);
    if (an || bn) return an && !bn;
    return loft_str_cmp(a, b) < 0;
}

static int loft_text_le(loft_str a, loft_str b) {
    int an = loft_str_is_null(a);
    if (an || loft_str_is_null(b)) return an;
    return loft_str_cmp(a, b) <= 0;
}

static int loft_str_eq(loft_str a, loft_str b) {
    return a.len == b.len && (a.len == 0 || memcmp(a.p, b.p, a.len) == 0);
}

/* Unicode scalar values in `s`: the bytes that do not continue a UTF-8 sequence. */
static size_t loft_chars(loft_str s) {
    size_t n = 0;
    for (size_t i = 0; i < s.len; i++)
        if (((unsigned char) s.p[i] & 0xC0) != 0x80) n++;
    return n;
}

/* ops::format_text */
static void loft_format_text(loft_text *s, loft_str val, int64_t width, int dir, uint8_t token) {
    if (loft_str_is_null(val)) val = loft_lit("null", 4);
    if (dir == 2) dir = -1;
    size_t tokens = width > 0 ? (size_t) width : 0;
    size_t chars = loft_chars(val);
    tokens = chars >= tokens ? 0 : tokens - chars;
    if (dir < 0) {
        loft_text_append(s, val);
        while (tokens > 0) { loft_push_char(s, (char) token); tokens--; }
    } else if (dir > 0) {
        while (tokens > 0) { loft_push_char(s, (char) token); tokens--; }
        loft_text_append(s, val);
    } else {
        size_t ct = 0;
        while (ct < tokens / 2) { loft_push_char(s, (char) token); ct++; }
        loft_text_append(s, val);
        while (ct < tokens) { loft_push_char(s, (char) token); ct++; }
    }
}

static uint8_t loft_null_pad(uint8_t token) {
    return token == '0' ? ' ' : token;
}

/* ops::format_prefixed */
static void loft_format_prefixed(loft_text *s, loft_str prefix, loft_str digits, int64_t width,
                                 int dir, uint8_t token) {
    if (token == '0' && prefix.len > 0) {
        loft_text_append(s, prefix);
        loft_format_text(s, digits, width - (int64_t) loft_chars(prefix), dir, token);
        return;
    }
    loft_text res = { 0, 0, 0 };
    loft_text_append(&res, prefix);
    loft_text_append(&res, digits);
    loft_format_text(s, loft_view(&res), width, dir, token);
    loft_text_free(&res);
}

/* ops::pad_rendered */
static void loft_pad_rendered(loft_text *s, size_t start, size_t sign, int64_t width, int dir,
                              uint8_t token) {
    if (width <= (int64_t) (s->len - start)) return;
    loft_text rendered = loft_text_own(loft_lit(s->p + start, s->len - start));
    s->len = start;
    loft_format_prefixed(s, loft_lit(rendered.p, sign),
                         loft_lit(rendered.p + sign, rendered.len - sign), width, dir, token);
    loft_text_free(&rendered);
}

/* The decimal digits of `v`, most significant first, into `buf` (twenty at most): what `%llu`
 * writes, without parsing a format per number. */
static int loft_u64_dec(char *buf, uint64_t v) {
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
static void loft_push_decimal(loft_text *s, int neg, uint64_t abs, int plus, int64_t width,
                              int dir, uint8_t token) {
    size_t start = s->len;
    if (neg) loft_push_char(s, '-');
    else if (plus) loft_push_char(s, '+');
    size_t sign = s->len - start;
    char buf[24];
    int n = loft_u64_dec(buf, abs);
    loft_push(s, buf, (size_t) n);
    loft_pad_rendered(s, start, sign, width, dir, token);
}

#define LOFT_HEX_UPPER 17

/* ops::format_long */
static void loft_format_long(loft_text *s, int64_t val, uint8_t radix, int64_t width,
                             uint8_t token, int plus, int note, int dir) {
    if (dir == 2) dir = 1;
    if (val == LOFT_NULL) {
        loft_format_text(s, loft_lit("null", 4), width, dir, loft_null_pad(token));
        return;
    }
    if (radix == 10) {
        uint64_t abs = val < 0 ? (uint64_t) 0 - (uint64_t) val : (uint64_t) val;
        loft_push_decimal(s, val < 0, abs, plus, width, dir, token);
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
    } else if (radix == 8) {
        n = snprintf(buf, sizeof buf, "%llo", (unsigned long long) u);
        prefix = note ? "0o" : "";
    } else if (radix == 16) {
        n = snprintf(buf, sizeof buf, "%llx", (unsigned long long) u);
        prefix = note ? "0x" : "";
    } else if (radix == LOFT_HEX_UPPER) {
        n = snprintf(buf, sizeof buf, "%llX", (unsigned long long) u);
        prefix = note ? "0x" : "";
    } else {
        fputs("loft: unknown radix\n", stderr);
        abort();
    }
    loft_format_prefixed(s, loft_lit(prefix, strlen(prefix)), loft_lit(buf, (size_t) n), width,
                         dir, token);
}

/* print (codegen_runtime::host_print): a reader that stopped reading ends the program quietly
 * with 0 — `prog | head` is the ordinary end of a pipeline, not a fault.  A POSIX host ignores
 * SIGPIPE (`loft_start`), so the closed pipe arrives here as EPIPE.  A text holding a newline is
 * flushed, as Rust's line-writing stdout flushes it: a reader on a pipe sees each line when it is
 * printed, not when a block fills or the program ends. */
static void loft_print(loft_str s) {
#if defined(EPIPE)
    if (fwrite(s.p, 1, s.len, stdout) < s.len && errno == EPIPE) exit(0);
    if (memchr(s.p, '\n', s.len) && fflush(stdout) != 0 && errno == EPIPE) exit(0);
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
static void *loft_dl_open(const char *path, int scope) {
    return dlopen(path, scope == LOFT_DL_GLOBAL ? RTLD_NOW | RTLD_GLOBAL
                        : scope == LOFT_DL_LOCAL ? RTLD_NOW | RTLD_LOCAL : RTLD_NOW);
}
static void *loft_dl_sym(void *h, const char *sym) { return dlsym(h, sym); }
static void *loft_dl_self_sym(const char *sym) {
    void *self = dlopen(0, RTLD_LAZY);
    return self ? dlsym(self, sym) : 0;
}
static const char *loft_dl_error(void) {
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
static void *loft_dl_proc(loft_win_proc p) {
    void *v = 0;
    memcpy(&v, &p, sizeof v < sizeof p ? sizeof v : sizeof p);
    return v;
}
#define LOFT_DL_DEFAULT 0
#define LOFT_DL_GLOBAL 1
#define LOFT_DL_LOCAL 2
/* Windows has no scope: a symbol is always asked of its own module. */
static void *loft_dl_open(const char *path, int scope) {
    (void) scope;
    return (void *) LoadLibraryA(path);
}
static void *loft_dl_sym(void *h, const char *sym) {
    return loft_dl_proc(GetProcAddress((struct HINSTANCE__ *) h, sym));
}
static void *loft_dl_self_sym(const char *sym) {
    struct HINSTANCE__ *mods[512];
    unsigned long need = 0;
    if (!K32EnumProcessModules(GetCurrentProcess(), mods, (unsigned long) sizeof mods, &need)) return 0;
    size_t n = need / sizeof mods[0];
    for (size_t i = 0; i < n && i < 512; i++) {
        void *p = loft_dl_proc(GetProcAddress(mods[i], sym));
        if (p) return p;
    }
    return 0;
}
static const char *loft_dl_error(void) {
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
static void *loft_c_optlib_open(uint32_t i) {
    static void *handles[64];
    static uint8_t tried[64];
    if (i >= 64) return 0;
    if (!tried[i]) {
        tried[i] = 1;
        for (const char *const *c = loft_col[i].cands; *c && !handles[i]; c++) handles[i] = loft_dl_open(*c, LOFT_DL_DEFAULT);
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

/* A `#c` symbol in this program (its linked libraries, its shims), else in a library
 * `loft_c_bind_libraries` opened.  Null when nothing exports it. */
static void *loft_c_bound_sym(const char *sym) {
#if defined(LOFT_DL)
    void *p = loft_dl_self_sym(sym);
    for (uint32_t i = 0; !p && i < loft_ncbl_handles; i++) p = loft_dl_sym(loft_cbl_handles[i], sym);
    return p;
#else
    (void) sym;
    return 0;
#endif
}

/* `c_libs::resolve`: a `#c` symbol of a package with optional libraries — in this program
 * (its linked libraries, its shims), else in one of the package's optional libraries, opened
 * now.  Null when nothing exports it. */
static void *loft_c_lazy_sym(const char *sym, const char *pkg) {
#if defined(LOFT_DL)
    void *p = loft_c_bound_sym(sym);
    for (uint32_t i = 0; !p && i < loft_ncol; i++) {
        if (strcmp(loft_col[i].pkg, pkg) != 0) continue;
        void *h = loft_c_optlib_open(i);
        if (h) p = loft_dl_sym(h, sym);
    }
    return p;
#else
    (void) sym;
    (void) pkg;
    return 0;
#endif
}

static void loft_panic(loft_str message, loft_str file, int64_t line);

/* `c_libs::missing_symbol_message`: a call of a symbol nothing exports halts, saying why. */
static void loft_c_missing_symbol(const char *sym) {
    char m[512];
    snprintf(m, sizeof m,
             "`#c` symbol '%s' not found — it is not in this process and no loaded library exports it. If it "
             "comes from an `[c] optional-libs` library, that library is not installed: ask "
             "`c_library_available(\"<soname>\")` before calling. Otherwise declare the library it comes from, "
             "or check the spelling",
             sym);
    loft_panic(loft_lit(m, strlen(m)), loft_lit("", 0), 0);
}

static uint8_t loft_c_library_available(loft_str name) {
    for (uint32_t i = 0; i < loft_ncl; i++) {
        if (strlen(loft_cl[i]) == name.len && memcmp(loft_cl[i], name.p, name.len) == 0) {
#if defined(LOFT_DL)
            if (loft_cls && loft_cls[i]) {
                for (const char *const *s = loft_cls[i]; *s; s++) {
                    if (!loft_c_bound_sym(*s)) return 0;
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
        void *h = loft_c_optlib_open(i);
        if (!h) return 0;
        for (const char *const *s = loft_col[i].syms; *s; s++)
            if (!loft_dl_sym(h, *s) && !loft_c_bound_sym(*s)) return 0;
        return 1;
    }
#endif
    return 0;
}

/* eprint (platform::host_eprint): the text to standard error, unbuffered as Rust's is. */
static void loft_eprint(loft_str s) {
    fwrite(s.p, 1, s.len, stderr);
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
static int64_t loft_gc_abs(int64_t a, int *ok) {
    if (a == LOFT_NULL) { *ok = 0; return 0; }
    return a < 0 ? -a : a;
}
static int64_t loft_gc_add(int64_t a, int64_t b, int *ok) {
    int64_t r;
    if (LOFT_ADD_OVF(a, b, &r)) { *ok = 0; return 0; }
    return r;
}
static int64_t loft_gc_mul(int64_t a, int64_t b, int *ok) {
    int64_t r;
    if (LOFT_MUL_OVF(a, b, &r)) { *ok = 0; return 0; }
    return r;
}
static int64_t loft_gc_max(int64_t a, int64_t b) { return a > b ? a : b; }

/* The checked `+` and `-`: null in, null out, and null where the result does not fit — the
 * compiler's overflow builtin (`LOFT_ADD_OVF`), which both GCC and Clang lower to one flag test,
 * where Clang kept the spelled-out range comparisons as compares and branches. */
LOFT_HOT_INLINE int64_t loft_add(int64_t a, int64_t b) {
    int64_t r;
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    return LOFT_ADD_OVF(a, b, &r) ? LOFT_NULL : r;
}

LOFT_HOT_INLINE int64_t loft_sub(int64_t a, int64_t b) {
    int64_t r;
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    return LOFT_SUB_OVF(a, b, &r) ? LOFT_NULL : r;
}

LOFT_HOT_INLINE int64_t loft_mul(int64_t a, int64_t b) {
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

LOFT_HOT_INLINE int64_t loft_div(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b == 0) return LOFT_NULL;
    return a / b;
}

LOFT_HOT_INLINE int64_t loft_rem(int64_t a, int64_t b) {
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
LOFT_HOT_INLINE int64_t loft_w64(int32_t a) { return a == LOFT_NULL32 ? LOFT_NULL : (int64_t) a; }
LOFT_HOT_INLINE int32_t loft_n32(int64_t a) {
    return (a > (int64_t) INT32_MIN && a <= (int64_t) INT32_MAX) ? (int32_t) a : LOFT_NULL32;
}
LOFT_HOT_INLINE int32_t loft_add32(int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32) return LOFT_NULL32;
    if ((b > 0 && a > INT32_MAX - b) || (b < 0 && a <= INT32_MIN - b)) return LOFT_NULL32;
    return a + b;
}
LOFT_HOT_INLINE int32_t loft_sub32(int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32) return LOFT_NULL32;
    if ((b < 0 && a > INT32_MAX + b) || (b > 0 && a <= INT32_MIN + b)) return LOFT_NULL32;
    return a - b;
}
LOFT_HOT_INLINE int32_t loft_mul32(int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32) return LOFT_NULL32;
    return loft_n32((int64_t) a * (int64_t) b);
}
LOFT_HOT_INLINE int32_t loft_div32(int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32 || b == 0) return LOFT_NULL32;
    return a / b; /* INT32_MIN is null, so INT32_MIN / -1 is never reached */
}
LOFT_HOT_INLINE int32_t loft_rem32(int32_t a, int32_t b) {
    if (a == LOFT_NULL32 || b == LOFT_NULL32 || b == 0) return LOFT_NULL32;
    return a % b;
}

/* The release-pass probe's forms (`LOFT_RELEASE_PASS_PROBE=1`, @PLN201): the processor's
 * wrapping arithmetic, as `generation::ops::int_arith::probe_form` emits it for Rust — a
 * MEASUREMENT of the ship tier's ceiling, never a build anyone ships (@C129).  Each wrap is
 * DEFINED here: the sum, difference, product and negation are taken in unsigned arithmetic,
 * and `INT64_MIN / -1` answers what Rust's `wrapping_div` answers; a zero divisor still
 * answers null, as the probe keeps it. */
LOFT_HOT_INLINE int64_t loft_wrap_add(int64_t a, int64_t b) { return (int64_t) ((uint64_t) a + (uint64_t) b); }
LOFT_HOT_INLINE int64_t loft_wrap_sub(int64_t a, int64_t b) { return (int64_t) ((uint64_t) a - (uint64_t) b); }
LOFT_HOT_INLINE int64_t loft_wrap_mul(int64_t a, int64_t b) { return (int64_t) ((uint64_t) a * (uint64_t) b); }
LOFT_HOT_INLINE int64_t loft_wrap_neg(int64_t a) { return (int64_t) (0 - (uint64_t) a); }
LOFT_HOT_INLINE int64_t loft_wrap_div(int64_t a, int64_t b) {
    if (b == 0) return LOFT_NULL;
    return b == -1 ? loft_wrap_neg(a) : a / b;
}
LOFT_HOT_INLINE int64_t loft_wrap_rem(int64_t a, int64_t b) {
    if (b == 0) return LOFT_NULL;
    return b == -1 ? 0 : a % b;
}

/* A header built without `loft_cc.h` (the replays of tests/c_emit.rs) keeps one copy of its
 * state: no threads. */
#ifndef LOFT_TLS
#define LOFT_TLS
#endif
/* `assert`: a failed one ends the running test (the test main catches it) or, outside a
 * test, the program, with the interpreter's message on stderr and exit status 1. */
static LOFT_TLS LOFT_JMP_BUF loft_test_jmp;
static LOFT_TLS int loft_in_test = 0;

/* The message of the last failed assert, for a test expected to fail with it. */
static LOFT_TLS loft_text loft_fail_msg = { 0, 0, 0 };

/* The texts the running frames hold (loft#2198), in a unit that runs tests: a fault that ends a
 * test `longjmp`s past every scope-exit free, so each function names the slots its epilogue
 * frees (`LOFT_HOLD`), its wrapper drops them again on return (`LOFT_HOLD_MARK`/`_BACK`), and a
 * halt releases what the abandoned frames still hold before it jumps — while those frames are
 * still on the stack.  A unit that binds a lazy collection keeps them too, since a driver's fault
 * is contained and the program goes on (`loft_lazy_held_floor`).  Every other unit compiles this
 * to nothing: there a halt ends the process. */
#ifdef LOFT_HALT_RELEASE
typedef struct { loft_text *const *slot; size_t n; } loft_held_frame;
static LOFT_TLS loft_held_frame *loft_held = 0;
static LOFT_TLS size_t loft_nheld = 0, loft_capheld = 0;
static void loft_hold(loft_text *const *slot, size_t n) {
    if (loft_nheld == loft_capheld) {
        loft_capheld = loft_capheld ? loft_capheld * 2 : 64;
        loft_held = (loft_held_frame *) realloc(loft_held, loft_capheld * sizeof *loft_held);
        if (!loft_held) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_held[loft_nheld].slot = slot;
    loft_held[loft_nheld].n = n;
    loft_nheld++;
}
/* Free the texts of every frame above `floor`, innermost first; a slot already freed is empty. */
static void loft_release_held(size_t floor) {
    while (loft_nheld > floor) {
        loft_held_frame *f = &loft_held[--loft_nheld];
        for (size_t i = 0; i < f->n; i++) loft_text_free(f->slot[i]);
    }
}
#define LOFT_HOLD(n, ...) loft_text *const loft_hd[] = { __VA_ARGS__ }; loft_hold(loft_hd, n)
#define LOFT_HOLD_MARK(m) size_t m = loft_nheld
#define LOFT_HOLD_BACK(m) (loft_nheld = (m))
#define LOFT_RELEASE_HELD(floor) loft_release_held(floor)
/* The depth a contained fault releases down to: the frames under the lazy driver keep theirs. */
static LOFT_TLS size_t loft_lazy_held_floor = 0;
#else
#define LOFT_HOLD(n, ...) ((void) 0)
#define LOFT_HOLD_MARK(m) ((void) 0)
#define LOFT_HOLD_BACK(m) ((void) 0)
#define LOFT_RELEASE_HELD(floor) ((void) 0)
#endif

static void loft_fault_where(loft_str file, int64_t line);
/* A fault inside a lazy driver is CONTAINED (`call_frames::in_lazy_driver`): it ends the driver,
 * not the program — `loft_lazy_miss` set the jump and records `<kind>: <message>` as the
 * collection's failure, as the interpreter's `panic_any(label: message)` does. */
static LOFT_TLS LOFT_JMP_BUF *loft_lazy_jmp = 0;
static LOFT_TLS loft_text loft_lazy_fault = { 0, 0, 0 };
static void loft_lazy_contain(const char *label, loft_str message) {
    loft_lazy_fault.len = 0;
    loft_push(&loft_lazy_fault, label, strlen(label));
    loft_push(&loft_lazy_fault, ": ", 2);
    loft_push(&loft_lazy_fault, message.p, message.len);
    LOFT_RELEASE_HELD(loft_lazy_held_floor);
    LOFT_LONGJMP(*loft_lazy_jmp);
}

/* A production run's fault logger (`loft_log_production_fault`), bound by a unit's `main`; null
 * in a runtime built without one, where every fault halts. */
static int (*loft_fault_logger)(int sev, const char *label, const char *prefix, loft_str message,
                                loft_str file, int64_t line) = 0;

static void loft_assert(uint8_t test, loft_str message, loft_str file, int64_t line) {
    if (test == 1) return;
    if (loft_lazy_jmp) loft_lazy_contain("assertion_failed", message);
    if (!loft_in_test && loft_fault_logger &&
        loft_fault_logger(2, "assertion_failed", "assertion failed: ", message, file, line))
        return;
    fflush(stdout);
    fputs("error: assertion failed: ", stderr);
    fwrite(message.p, 1, message.len, stderr);
    fputc('\n', stderr);
    if (!loft_in_test) loft_fault_where(file, line);
    loft_fail_msg.len = 0;
    loft_push(&loft_fail_msg, "assertion failed: ", 18);
    loft_push(&loft_fail_msg, message.p, message.len);
    if (loft_in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(loft_test_jmp); }
    fflush(stdout);
    exit(1);
}

/* A production run's discard of a locked write (`loft_log_locked_write`), bound by a unit's
 * `main`; null in a runtime built without one, where every locked write halts. */
static int (*loft_locked_logger)(void) = 0;

/* `@FR-H-WriteLocked` (`RuntimeError::locked_store_write`): a development run stops at a write
 * into a store the author locked, as the interpreter does; a test fails.  A production run
 * discards the write, logs it, and continues on the store's old bytes
 * (`discard_locked_write_in_production`, `@C80`): the answer is 1, and the caller skips it. */
static int loft_locked_write(void) {
    static const char msg[] = "write to a locked store — unlock it with `#lock = false` before writing";
    if (!loft_in_test && loft_locked_logger && loft_locked_logger()) return 1;
    fflush(stdout);
    fputs("error: ", stderr);
    fputs(msg, stderr);
    fputc('\n', stderr);
    loft_fail_msg.len = 0;
    loft_push(&loft_fail_msg, msg, sizeof msg - 1);
    if (loft_in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(loft_test_jmp); }
    fflush(stdout);
    exit(1);
}

/* The call stack ran past `LOFT_MAX_CALL_DEPTH` (`RuntimeError::stack_overflow`): the program
 * stops as a failed assertion stops it, at `line` of `file`. */
#ifdef LOFT_MAX_CALL_DEPTH
static void loft_stack_overflow(const char *file, uint32_t line) {
    char msg[96];
    int n = snprintf(msg, sizeof msg, "call stack overflow \xe2\x80\x94 exceeded %d stack frames",
                     (int) LOFT_MAX_CALL_DEPTH);
    fflush(stdout);
    fputs("error: ", stderr);
    fwrite(msg, 1, (size_t) n, stderr);
    fputc('\n', stderr);
    if (!loft_in_test) loft_fault_where(loft_lit(file, strlen(file)), line);
    loft_fail_msg.len = 0;
    loft_push(&loft_fail_msg, msg, (size_t) n);
    if (loft_in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(loft_test_jmp); }
    fflush(stdout);
    exit(1);
}
#endif

/* `stack_trace()` reads the frames, so a unit that reaches it keeps them in every build; any
 * other unit keeps them where the build asks for the call chain a fault prints
 * (`-DLOFT_CALL_CHAIN`, the semantics tier) and compiles the pushes away where it does not. */
#if (defined(LOFT_FRAMES) || defined(LOFT_LEAK_SITES)) && !defined(LOFT_CALL_CHAIN)
#define LOFT_CALL_CHAIN 1
#endif
#ifdef LOFT_CALL_CHAIN
/* The frames the program is running (call_frames.rs, `cr_call_push`): each function's name and
 * file, and the line it is calling from (`cr_call_site`), outermost first. */
typedef struct { const char *fn, *file; uint32_t site, decl; } loft_frame;
static LOFT_TLS loft_frame *loft_frames = 0;
static LOFT_TLS size_t loft_nframes = 0, loft_capframes = 0;
#ifdef LOFT_MAX_CALL_DEPTH
static void loft_stack_overflow(const char *file, uint32_t line);
#endif

static void loft_frame_push(const char *fn, const char *file, uint32_t decl) {
#ifdef LOFT_MAX_CALL_DEPTH
    /* The call-depth cap (`call_frames::cr_call_push`): checked BEFORE the push, so a refused
     * call never runs, and reported at the declaration of the function that is running — the
     * frame below — as the interpreter's `State::fn_call` reports it (loft#1058). */
    if (loft_nframes >= LOFT_MAX_CALL_DEPTH) {
        const loft_frame *below = loft_nframes ? &loft_frames[loft_nframes - 1] : 0;
        loft_stack_overflow(below ? below->file : file, below ? below->decl : decl);
    }
#endif
    if (loft_nframes == loft_capframes) {
        loft_capframes = loft_capframes ? loft_capframes * 2 : 64;
        loft_frames = (loft_frame *) realloc(loft_frames, loft_capframes * sizeof *loft_frames);
        if (!loft_frames) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_frames[loft_nframes].fn = fn;
    loft_frames[loft_nframes].file = file;
    loft_frames[loft_nframes].site = 0;
    loft_frames[loft_nframes].decl = decl;
    loft_nframes++;
}

static void loft_frame_pop(void) {
    if (loft_nframes) loft_nframes--;
}

static void loft_frame_site(uint32_t line) {
    if (loft_nframes) loft_frames[loft_nframes - 1].site = line;
}
/* A test that failed left its frames pushed (`longjmp`); the next starts from none. */
#define LOFT_FRAMES_RESET() (loft_nframes = 0)
#else
#define LOFT_FRAMES_RESET() ((void) 0)
#define loft_frame_push(fn, file, decl) ((void) 0)
#define loft_frame_pop() ((void) 0)
#define loft_frame_site(line) ((void) 0)
#endif

/* The frames a halting fault fired under (`RuntimeError::render_call_chain`): innermost first,
 * five at most with a count of the rest; nothing for a single frame, whose `-->` line already
 * says where. */
static void loft_fault_chain(void) {
#ifdef LOFT_CALL_CHAIN
    size_t n = loft_nframes, shown = n < 5 ? n : 5;
    if (n <= 1) return;
    fprintf(stderr, "  in fn %s() \xe2\x86\x90 called from\n", loft_frames[n - 1].fn);
    for (size_t i = 1; i < shown; i++) fprintf(stderr, "        fn %s()\n", loft_frames[n - 1 - i].fn);
    if (n > 5) fprintf(stderr, "        \xe2\x80\xa6 (%lu more frames)\n", (unsigned long) (n - 5));
#endif
}

/* Where a halting fault fired (`render_entry_pretty` for a fault at column 1): the `-->` line,
 * then the source line and its caret, read from the file now as `FileSourceLoader` reads it
 * (a tab shown as four spaces), then the call chain. */
static void loft_fault_where(loft_str file, int64_t line) {
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
                    if (c == '\t') loft_push(&text, "    ", 4);
                    else { char ch = (char) c; loft_push(&text, &ch, 1); }
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
    loft_fault_chain();
}

/* panic(message): the program stops as a failed assertion stops it, saying `panic:`. */
static void loft_panic(loft_str message, loft_str file, int64_t line) {
    if (loft_lazy_jmp) loft_lazy_contain("user_panic", message);
    if (!loft_in_test && loft_fault_logger && loft_fault_logger(3, "user_panic", "panic: ", message, file, line))
        return;
    fflush(stdout);
    fputs("error: panic: ", stderr);
    fwrite(message.p, 1, message.len, stderr);
    fputc('\n', stderr);
    if (!loft_in_test) loft_fault_where(file, line);
    loft_fail_msg.len = 0;
    loft_push(&loft_fail_msg, "panic: ", 7);
    loft_push(&loft_fail_msg, message.p, message.len);
    if (loft_in_test) { LOFT_RELEASE_HELD(0); LOFT_LONGJMP(loft_test_jmp); }
    fflush(stdout);
    exit(1);
}

/* exit(code) (runtime_error::exit_program): the program ends in silence with the code's low
 * byte as its status; under the test runner it ends the test as a failure. */
static void loft_exit(int64_t code) {
    if (loft_in_test) {
        char m[48];
        sprintf(m, "exit %lld", (long long) code);
        loft_fail_msg.len = 0;
        loft_push(&loft_fail_msg, m, strlen(m));
        LOFT_RELEASE_HELD(0);
        LOFT_LONGJMP(loft_test_jmp);
    }
    fflush(stdout);
    fflush(stderr);
    exit((int) ((uint64_t) code & 0xFF));
}

/* True when the last failure's message holds one of `want`. */
static int loft_failed_with(const loft_str *want, size_t n) {
    for (size_t i = 0; i < n; i++) {
        if (want[i].len == 0) return 1;
        for (size_t at = 0; at + want[i].len <= loft_fail_msg.len; at++) {
            if (memcmp(loft_fail_msg.p + at, want[i].p, want[i].len) == 0) return 1;
        }
    }
    return 0;
}

/* The bit operators (ops::op_logical_and_long …): null when either side is; a shift by an
 * amount outside 0..63 is null. */
static int64_t loft_and(int64_t a, int64_t b) { return a == LOFT_NULL || b == LOFT_NULL ? LOFT_NULL : (a & b); }
static int64_t loft_or(int64_t a, int64_t b) { return a == LOFT_NULL || b == LOFT_NULL ? LOFT_NULL : (a | b); }
static int64_t loft_xor(int64_t a, int64_t b) { return a == LOFT_NULL || b == LOFT_NULL ? LOFT_NULL : (a ^ b); }
static int64_t loft_shl(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b < 0 || b > 63) return LOFT_NULL;
    return (int64_t) ((uint64_t) a << b);
}
static int64_t loft_shr(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b < 0 || b > 63) return LOFT_NULL;
    return a >> b;
}

/* OpCastTextFromBool: the null boolean is the null text. */
static loft_str loft_bool_text(uint8_t b) {
    return b == 1 ? loft_lit("true", 4) : b == 255 ? loft_null_text() : loft_lit("false", 5);
}

static size_t loft_utf8(uint32_t c, char *out);

/* OpGetTextSub (State::get_text_sub): the bytes [from, till) of `v`, a negative bound
 * counting from the end, `from` snapped back and `till` forward to a character boundary;
 * `till` INT32_MIN is the one character at `from`.  The bounds arrive as i64 and are taken
 * as i32, as the interpreter takes them. */
/* Inlined with the text writes (`LOFT_TEXT_INLINE`, loft_text.h).  A one-byte slice
 * `s[k..k + 1]` of an ASCII character answers that byte before any snapping: the general path
 * gives the same byte, since an ASCII byte starts a character and the next byte, tested here,
 * does not continue it.  Its length is then the constant 1, which the append after it folds
 * on (`loft_text_concat`).  `-DLOFT_NO_SHORT_APPEND` takes the general path. */
LOFT_TEXT_INLINE loft_str loft_text_sub(loft_str v, int64_t from64, int64_t till64) {
#ifndef LOFT_NO_SHORT_APPEND
    if ((uint64_t) till64 == (uint64_t) from64 + 1 && (uint64_t) from64 < v.len && from64 < INT32_MAX
        && (unsigned char) v.p[from64] < 0x80
        && ((uint64_t) till64 == v.len || ((unsigned char) v.p[till64] & 0xC0) != 0x80))
        return loft_lit(v.p + from64, 1);
#endif
    int32_t from = (int32_t) (uint32_t) (uint64_t) from64;
    int32_t till = (int32_t) (uint32_t) (uint64_t) till64;
    int32_t n = (int32_t) v.len;
    const unsigned char *b = (const unsigned char *) v.p;
    if (from < 0) { from += n; if (from < 0) from = 0; }
    if (from >= n) return loft_lit(v.p, 0);
    while ((b[from] & 0xC0) == 0x80 && from > 0) from--;
    if (till == INT32_MIN) {
        unsigned char c = b[from];
        int32_t w = (c & 0xE0) == 0xC0 ? 2 : (c & 0xF0) == 0xE0 ? 3 : (c & 0xF0) == 0xF0 ? 4 : 1;
        if (w > n - from) w = n - from;
        return loft_lit(v.p + from, (size_t) w);
    }
    if (till < 0) till += n;
    int32_t len = till - from;
    if (len <= 0) return loft_lit(v.p, 0);
    if (len + from > n) {
        len = n - from;
    } else if (till < n) {
        for (int32_t t = till; t < n && (b[t] & 0xC0) == 0x80; t++) len++;
    }
    return loft_lit(v.p + from, (size_t) len);
}

/* The text natives (src/native.rs t_4text_*): Rust's `str` methods on the raw bytes, the
 * null text included as the one byte it is. */
static int loft_mem_at(loft_str s, size_t at, loft_str v) {
    return at + v.len <= s.len && (v.len == 0 || memcmp(s.p + at, v.p, v.len) == 0);
}

static uint8_t loft_starts_with(loft_str s, loft_str v) { return (uint8_t) loft_mem_at(s, 0, v); }

static uint8_t loft_ends_with(loft_str s, loft_str v) {
    return (uint8_t) (v.len <= s.len && loft_mem_at(s, s.len - v.len, v));
}

static int64_t loft_find(loft_str s, loft_str v) {
    for (size_t at = 0; at + v.len <= s.len; at++) if (loft_mem_at(s, at, v)) return (int64_t) at;
    return LOFT_NULL;
}

static int64_t loft_rfind(loft_str s, loft_str v) {
    if (v.len > s.len) return LOFT_NULL;
    for (size_t at = s.len - v.len + 1; at-- > 0;) if (loft_mem_at(s, at, v)) return (int64_t) at;
    return LOFT_NULL;
}

static uint8_t loft_contains(loft_str s, loft_str v) { return (uint8_t) (loft_find(s, v) != LOFT_NULL); }

/* char::is_whitespace, for the code point at the UTF-8 bytes `p` of `n` bytes. */
static int loft_is_space(uint32_t c) {
    return (c >= 9 && c <= 13) || c == 32 || c == 0x85 || c == 0xA0 || c == 0x1680
        || (c >= 0x2000 && c <= 0x200A) || c == 0x2028 || c == 0x2029 || c == 0x202F
        || c == 0x205F || c == 0x3000;
}

/* The code point starting at byte `at` and its length in bytes. */
static uint32_t loft_decode(loft_str s, size_t at, size_t *n) {
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

static loft_str loft_trim_start_view(loft_str s) {
    size_t at = 0, n;
    while (at < s.len && loft_is_space(loft_decode(s, at, &n))) at += n;
    return loft_lit(s.p + at, s.len - at);
}

static loft_str loft_trim_end_view(loft_str s) {
    size_t end = s.len;
    while (end > 0) {
        size_t st = end - 1, n;
        while (st > 0 && (((const unsigned char *) s.p)[st] & 0xC0) == 0x80) st--;
        if (!loft_is_space(loft_decode(s, st, &n))) break;
        end = st;
    }
    return loft_lit(s.p, end);
}

static loft_text loft_trim(loft_str s) { return loft_text_own(loft_trim_end_view(loft_trim_start_view(s))); }
static loft_text loft_trim_start(loft_str s) { return loft_text_own(loft_trim_start_view(s)); }
static loft_text loft_trim_end(loft_str s) { return loft_text_own(loft_trim_end_view(s)); }

/* str::replace: every non-overlapping `v` left to right; an empty `v` matches before every
 * character and at the end. */
static loft_text loft_replace(loft_str s, loft_str v, loft_str w) {
    loft_text out = { 0, 0, 0 };
    size_t at = 0;
    if (v.len == 0) {
        while (at < s.len) {
            size_t n;
            loft_decode(s, at, &n);
            loft_push(&out, w.p, w.len);
            loft_push(&out, s.p + at, n);
            at += n;
        }
        loft_push(&out, w.p, w.len);
        return out;
    }
    /* As str::replace: find the next match by its first byte, push the run before it whole. */
    while (at < s.len) {
        const char *hit = (const char *) memchr(s.p + at, (unsigned char) v.p[0], s.len - at);
        while (hit && ((size_t) (hit - s.p) + v.len > s.len || memcmp(hit, v.p, v.len) != 0)) {
            size_t next = (size_t) (hit - s.p) + 1;
            hit = next < s.len ? (const char *) memchr(s.p + next, (unsigned char) v.p[0], s.len - next) : 0;
        }
        size_t to = hit ? (size_t) (hit - s.p) : s.len;
        loft_push(&out, s.p + at, to - at);
        if (!hit) break;
        loft_push(&out, w.p, w.len);
        at = to + v.len;
    }
    return out;
}

/* Stores::text_byte_at_native: the byte at `i` (negative from the end), 0 outside. */
static int64_t loft_byte_at(loft_str s, int64_t i) {
    int64_t len = (int64_t) s.len;
    int64_t at = i < 0 ? i + len : i;
    if (i == LOFT_NULL || at < 0 || at >= len) return 0;
    return ((const unsigned char *) s.p)[at];
}

/* Characters: a code point, 0 the null character. */
static uint32_t loft_char_of(int64_t v) {
    if (v == LOFT_NULL || v < 0 || v > 0x10FFFF || (v >= 0xD800 && v <= 0xDFFF)) return 0;
    return (uint32_t) v;
}

static int64_t loft_char_int(uint32_t c) {
    return c == 0 ? LOFT_NULL : (int64_t) c;
}

static int64_t loft_char_len(uint32_t c) {
    return c == 0 ? 0 : c < 0x80 ? 1 : c < 0x800 ? 2 : c < 0x10000 ? 3 : 4;
}

/* ops::text_character: the character holding byte `from` (negative counts from the end); an
 * index inside a character snaps back to its first byte; out of range is the null character. */
static uint32_t loft_text_char(loft_str t, int64_t from) {
    int64_t len = (int64_t) t.len;
    int64_t i = from < 0 ? from + len : from;
    if (from == LOFT_NULL || i < 0 || i >= len) return 0;
    const unsigned char *b = (const unsigned char *) t.p;
    if (b[i] < 0x80) return b[i];
    while (i > 0 && (b[i] & 0xC0) == 0x80) i--;
    uint32_t c = b[i];
    int n = c >= 0xF0 ? 3 : c >= 0xE0 ? 2 : 1;
    c &= n == 3 ? 0x07 : n == 2 ? 0x0F : 0x1F;
    for (int k = 1; k <= n && i + k < len; k++) c = (c << 6) | (b[i + k] & 0x3F);
    return c;
}

/* OpAppendCharacter: the null character appends nothing.  An ASCII character into a buffer with
 * room is one store and a length bump (`-DLOFT_NO_SHORT_APPEND` encodes and pushes it). */
static void loft_push_code(loft_text *t, uint32_t c) {
#ifndef LOFT_NO_SHORT_APPEND
    if (c - 1u < 0x7Fu && t->len < t->cap) {
        t->p[t->len++] = (char) c;
        return;
    }
#endif
    char buf[4];
    size_t n = c == 0 ? 0 : loft_utf8(c, buf);
    loft_push(t, buf, n);
}

/* A function value: the definition it names (LOFT_NULL for the absent one) and the closure
 * record a capturing lambda reads.  A call goes through the emitted dispatcher of its
 * signature, `loft_call{n}`, which switches on `d` (State::fn_call_ref). */
typedef struct { uint16_t store_nr; uint32_t rec; uint32_t pos; } loft_ref;
typedef struct { int64_t d; loft_ref clos; } loft_fn;

/* ── Stores ────────────────────────────────────────────────────────────────────────────────
 * The store model the IR addresses (src/store.rs, src/vector.rs): a store is an array of 8-byte
 * words; a record is a run of words whose first word holds its claimed size (and, at byte 4 of a
 * vector record, the vector's length); a reference (`DbRef`) names a store, a record and a byte
 * position, and a field is `rec * 8 + pos + offset`.  `rec == 0` is the null reference.  The
 * allocator is loft's own (`loft_store.h`, a port of `Store`); the table of stores below is
 * `Stores`: the lowest free slot is made again, a new slot starts at 100 words and a reused one
 * keeps its capacity, and a store's serial is the order it was made. */
LOFT_HOT_INLINE loft_ref loft_ref_null(void) {
    loft_ref r;
    r.store_nr = 0xFFFF;
    r.rec = 0;
    r.pos = 0;
    return r;
}

/* `DbRef::NULL_REPORTED`: the null an `else` arm or a reported access owns (`@FR-E-Report`). */
static loft_ref loft_ref_null_reported(void) {
    loft_ref r = loft_ref_null();
    r.pos = 0x80000000u;
    return r;
}

/* `LOFT_STRICT_STORES` (`keys::strict_stores`, @PLN206 P0b): a freed store stays dead — its
 * slot is not made again (`loft_store_make`), except the top one, whose free lowers the floor as
 * the interpreter's `trim_free_top` lowers `max` — so an access through a reference naming it is
 * a use after free, reported at the access in the
 * interpreter's words (`strict_store_violation`, a native run's: no bytecode positions); and a
 * store still live at exit is reported too (`loft_leak_report`).  Either makes the run exit 1.
 * The driver builds with it when `LOFT_STRICT_STORES` is set.  For probes: never reusing a slot
 * walks a long run off the end of the store table. */
#ifdef LOFT_STRICT_STORES
static uint64_t loft_strict_violations = 0;
static LOFT_TLS uint32_t loft_strict_floor = 0;

static LOFT_COLD void loft_strict_use_after_free(loft_ref r) {
    uint64_t n = loft_strict_violations++;
    if (n < 20) {
        uint16_t kt = loft_stores[r.store_nr].known;
        fflush(stdout);
        fprintf(stderr,
                "[strict-store] USE AFTER FREE (access) store #%u type=%s rec=%u pos=%u\n"
                "  killed by the free of `<not recorded>`\n"
                "  (no bytecode positions \xe2\x80\x94 native run; re-run with --interpret for pcs)\n",
                (unsigned) r.store_nr, kt < loft_ntn ? loft_tn[kt] : "?", (unsigned) r.rec, (unsigned) r.pos);
    } else if (n == 20) {
        fputs("[strict-store] ... further use-after-free reports suppressed\n", stderr);
    }
}
#endif

LOFT_HOT_INLINE loft_store_t *loft_store_of(loft_ref r) {
#ifdef LOFT_STRICT_STORES
    if (LOFT_UNLIKELY(r.store_nr < loft_nstores && !loft_stores[r.store_nr].live)) loft_strict_use_after_free(r);
#endif
    return &loft_stores[r.store_nr];
}

/* `#lock = …` (`n_set_store_lock`) and `#lock` (`n_get_store_lock`): the store holding `r`. */
static void loft_set_store_lock(loft_ref r, uint8_t locked) {
    if (r.rec == 0 || r.store_nr >= loft_nstores) return;
    loft_stores[r.store_nr].read_only = locked == 1;
}

/* A text lent as a `loft_text` that owns nothing (capacity 0): a borrowed member of a by-value
 * tuple, which no one frees or writes. */
static loft_text loft_text_lend(loft_str s) {
    loft_text t;
    t.p = (char *) s.p;
    t.len = s.len;
    t.cap = 0;
    return t;
}

/* `yield_frame()`: a compiled program has no host loop to return to, so a frame-driven program
 * runs straight through, as `--native` does. */
static void loft_yield_frame(void) {}

/* `store_reclaim(r)` (`Stores::reclaim_store`): the bytes the store holding `r` gave back. */
static int64_t loft_store_reclaim(loft_ref r) {
    if (r.store_nr >= loft_nstores) return 0;
    return (int64_t) loft_st_reclaim_tail(&loft_stores[r.store_nr]) * 8;
}

static uint8_t loft_get_store_lock(loft_ref r) {
    return (uint8_t) (r.rec != 0 && r.store_nr < loft_nstores && loft_stores[r.store_nr].read_only);
}


/* `Stores::database_named`: the lowest free slot, a fresh store in it; `counted` gives it the
 * next serial, as every store the program makes. */
/* `LOFT_LEAK_SITES` (the interpreter's leak-site report): each store remembers where it was made
 * — the function running, its file and the line it was at (`loft_frame_site`) — so a store left
 * at exit is named by its site, not only its type (`loft_leak_report`).  It keeps the frames
 * (`LOFT_CALL_CHAIN`); the driver builds with it when `LOFT_LEAK_SITES` is set. */
#ifdef LOFT_LEAK_SITES
typedef struct { const char *fn, *file; uint32_t line; } loft_leak_site;
static LOFT_TLS loft_leak_site *loft_sites = 0;
static LOFT_TLS uint32_t loft_nsites = 0;
static void loft_note_site(uint32_t nr) {
    if (nr >= loft_nsites) {
        uint32_t n = loft_nsites ? loft_nsites * 2 : 64;
        if (n <= nr) n = nr + 1;
        loft_sites = (loft_leak_site *) realloc(loft_sites, (size_t) n * sizeof *loft_sites);
        if (!loft_sites) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(loft_sites + loft_nsites, 0, (size_t) (n - loft_nsites) * sizeof *loft_sites);
        loft_nsites = n;
    }
    loft_leak_site *w = &loft_sites[nr];
    if (loft_nframes) {
        const loft_frame *f = &loft_frames[loft_nframes - 1];
        w->fn = f->fn;
        w->file = f->file;
        w->line = f->site ? f->site : f->decl;
    } else {
        w->fn = 0;
        w->file = 0;
        w->line = 0;
    }
}
#endif

static uint32_t loft_store_make(int counted) {
    uint32_t nr = 0;
#ifdef LOFT_STRICT_STORES
    /* `no_slot_reuse`: every slot below the floor was made once, and stays dead once freed. */
    nr = loft_strict_floor;
#endif
    while (nr < loft_nstores && loft_stores[nr].live) nr++;
#ifdef LOFT_STRICT_STORES
    loft_strict_floor = nr + 1;
#endif
#ifdef LOFT_STRICT_STORES
    if (nr >= 0xFFFF) {
        uint32_t live = 0, i;
        for (i = 0; i < loft_nstores; i++) live += loft_stores[i].live != 0;
        fprintf(stderr, "loft: store table exhausted by the checking mode, not by a leak: this run made 65535 "
                        "stores and %u are live.  LOFT_STRICT_STORES never reuses a freed store number, so that "
                        "a stale reference cannot reach a newer store, and store_nr is u16.  This program frees "
                        "and re-mints more stores than the mode can track: check it with a smaller loop count, "
                        "or run it without the switch.\n", (unsigned) live);
        abort();
    }
#endif
    if (nr >= 0xFFFF) { fputs("loft: store table exhausted: 65535 stores live at once\n", stderr); abort(); }
    if (nr == loft_nstores) {
        uint32_t n = loft_nstores ? loft_nstores * 2 : 16;
        if (n > 0xFFFF) n = 0xFFFF;
        loft_store_t *s = (loft_store_t *) realloc(loft_stores, n * sizeof *s);
        if (!s) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(s + loft_nstores, 0, (n - loft_nstores) * sizeof *s);
        loft_stores = s;
        loft_nstores = n;
    }
    loft_store_t *s = &loft_stores[nr];
#ifdef LOFT_LEAK_SITES
    loft_note_site(nr);
#endif
    if (!s->buf) {
        loft_heap_moved(s, 0, 100);
        s->size = 100;
        s->buf = (uint8_t *) malloc((size_t) s->size * 8);
        if (!s->buf) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_st_init(s);
    s->live = 1;
    s->known = 0xFFFF;
    s->pinned = 0;
    s->protect = 0;
    s->frame_holds = 0;
    s->orphaned = 0;
    s->serial = 0;
    if (counted) {
        s->serial = ++loft_stores_allocated;
        LOFT_TRACE("N %llu %u\n", (unsigned long long) s->serial, s->size);
    }
    return nr;
}

static uint32_t loft_store_new(void) {
    return loft_store_make(1);
}

/* A record of `words` words.  Its payload holds no promised value (`@FR-H-Claim`, as
 * `Store::claim`): each caller writes the fields it reads.  `LOFT_POISON_CLAIM` fills it with
 * poison, and MemorySanitizer over the corpus flags a read of a byte nobody wrote. */
static uint32_t loft_claim(loft_store_t *s, uint32_t words) {
    if (words < 1) words = 1;
    return loft_st_claim(s, words);
}

LOFT_HOT_INLINE uint8_t *loft_at(loft_store_t *s, uint32_t rec, uint32_t byte) {
#ifdef LOFT_FOREIGN
    /* `Store::read`'s intercept: the foreign record is served from the foreign bytes. */
    if (LOFT_UNLIKELY(rec == LOFT_FOREIGN_REC)) return loft_foreign_at(s, byte);
#endif
    return s->buf + (size_t) rec * 8 + byte;
}

/* `Store::elem_base`: where vector record `rec`'s elements begin, past its header — the foreign
 * bytes themselves for the foreign record. */
LOFT_HOT_INLINE uint8_t *loft_elem_base(loft_store_t *s, uint32_t rec) {
    return loft_at(s, rec, 8);
}

LOFT_HOT_INLINE uint32_t loft_u32(loft_store_t *s, uint32_t rec, uint32_t byte) {
    uint32_t v;
    memcpy(&v, loft_at(s, rec, byte), 4);
    return v;
}

LOFT_HOT_INLINE void loft_set_u32(loft_store_t *s, uint32_t rec, uint32_t byte, uint32_t v) {
    if (LOFT_WRITE_REFUSED(s)) return;
    memcpy(loft_at(s, rec, byte), &v, 4);
}

#ifdef LOFT_PARSE
/* `Store::parse_errors` (loft_parse.h): a store made again answers none, an exchange moves
 * them with the content, a copy of a parsed root brings them. */
static void loft_perrs_reset(uint16_t nr);
static void loft_perrs_swap(uint16_t a, uint16_t b);
static void loft_perrs_deliver(loft_ref data, loft_ref to, uint16_t tp, int fresh);
static void loft_perrs_deliver_vector(loft_ref data, loft_ref to, int whole);
#endif

/* OpDatabase: a fresh record of `bytes` in the variable's store, cleared in place, or in a
 * new store when the variable holds none. */
static void loft_database(loft_ref *var, uint32_t bytes) {
    uint32_t nr;
    uint32_t *kept = 0, nkept = 0;
    /* `@FR-G-Arg-Rebind`: a store a live generator frame names keeps the value the frame was
     * handed — the rebind orphans it and mints into a fresh store. */
    if (var->store_nr != 0xFFFF && var->store_nr < loft_nstores && loft_stores[var->store_nr].frame_holds) {
        loft_stores[var->store_nr].orphaned = 1;
        var->store_nr = 0xFFFF;
    }
    if (var->store_nr != 0xFFFF && var->store_nr < loft_nstores) {
        /* `Stores::clear`: the slot the variable names made again in place — a freed one too,
         * which keeps its serial — its tombstones kept. */
        nr = var->store_nr;
        loft_store_t *s = &loft_stores[nr];
#ifdef LOFT_FOREIGN
        /* A store made again while it serves a view stops serving it. */
        if (!s->pinned) loft_st_release_foreign(s);
#endif
        if (s->live) kept = loft_st_kept_positions(s, &nkept);
        loft_st_init(s);
        s->live = 1;
    } else {
        nr = loft_store_new();
    }
    /* `alloc_record_into`: the root claimed, its payload zeroed — a fresh store's collection
     * handles must read "none" — and its type tag at byte 4 (none yet). */
    loft_store_t *s = &loft_stores[nr];
    uint32_t rec = loft_st_claim(s, (bytes + 15) / 8);
    memset(s->buf + (size_t) rec * 8 + 4, 0, (size_t) loft_st_i32(s, rec, 0) * 8 - 4);
    loft_set_u32(s, rec, 4, 0xFFFFu);
    loft_st_entomb_kept(s, kept, nkept);
    free(kept);
    s->known = 0xFFFF;
#ifdef LOFT_PARSE
    loft_perrs_reset((uint16_t) nr);
#endif
    var->store_nr = (uint16_t) nr;
    var->rec = rec;
    var->pos = 8;
}

/* OpDatabase of a record type: the root carries the type it was minted as, which tells a move
 * (`loft_swap_in`) its own store from another. */
static void loft_database_typed(loft_ref *var, uint32_t bytes, uint16_t tp) {
    loft_database(var, bytes);
    loft_stores[var->store_nr].known = tp;
    loft_set_u32(&loft_stores[var->store_nr], var->rec, 4, tp);
}

/* OpDatabase of a vector wrapper (`main_vector<T>`, Stores::mint_at_place, @FR-R-Place): a
 * buffer that is a PLACE — a collection field of another record, not the wrapper's own store
 * — stays where it is with its collection released and emptied, so the literal builds where it lives;
 * the ordinary mint would clear the store the place lives in. */
/* The type descriptor is defined further down; a pointer to it is all this needs. */
struct loft_tdesc_s;
static void loft_remove_claims(const struct loft_tdesc_s *T, uint16_t tp, loft_ref r);

static void loft_database_wrapper(const struct loft_tdesc_s *T, loft_ref *var, uint32_t bytes, uint16_t tp) {
    if (var->store_nr != 0xFFFF && var->rec != 0 && var->store_nr < loft_nstores
        && !(var->rec == 1 && loft_stores[var->store_nr].known == tp)) {
        /* The collection the place held is released, then its handle emptied. */
        loft_remove_claims(T, tp, *var);
        loft_set_u32(&loft_stores[var->store_nr], var->rec, var->pos, 0);
        return;
    }
    loft_database(var, bytes);
    loft_stores[var->store_nr].known = tp;
}

static loft_fn loft_fn_make(int64_t d, loft_ref clos) {
    loft_fn f;
    f.d = d;
    f.clos = clos;
    return f;
}

static loft_fn loft_fn_of(int64_t d) {
    return loft_fn_make(d, loft_ref_null());
}

/* `Stores::evacuated` (`@FR-H-RefHome`): the copy each record a stored reference named was
 * evacuated to, per holder store, so a second holder in that store shares the one copy; an
 * entry goes with the store its record lived in. */
typedef struct { loft_ref old; uint16_t holder; loft_ref copy; } loft_evac;
static LOFT_TLS loft_evac *loft_evacs = 0;
static LOFT_TLS uint32_t loft_nevacs = 0, loft_evac_cap = 0;

/* OpFreeRef: the whole store goes, everything the root owned with it. */
static void loft_gen_free(loft_ref g);

/* `Stores::release_file_leases` (loft_io.h, which sets it): a store that is freed returns the
 * leases its records hold.  A tentative definition, so a unit without the file layer — the
 * allocator and keyed harnesses — leaves it null. */
static void (*loft_release_file_leases_hook)(uint16_t nr);

/* `@FR-R-SliceArg`: the carrier stores a slice handed to a `const` parameter is copied into —
 * a few per thread, kept across uses instead of minted and freed per use.  An idle carrier is
 * PINNED, so the leak report passes it by and a stray free leaves it alone; one in use is an
 * ordinary store until the free that would have released it gives it back (`loft_free_ref`). */
#define LOFT_SLICE_POOL 16
static LOFT_TLS loft_ref loft_sp_ref[LOFT_SLICE_POOL];
static LOFT_TLS uint8_t loft_sp_used[LOFT_SLICE_POOL];
static LOFT_TLS int loft_sp_n = 0;

/* Give carrier `nr` back if it is one in use: answers whether it was. */
static int loft_slice_give(uint32_t nr) {
    for (int i = 0; i < loft_sp_n; i++) {
        if (loft_sp_used[i] && loft_sp_ref[i].store_nr == nr) {
            loft_sp_used[i] = 0;
            loft_stores[nr].pinned = 1;
            return 1;
        }
    }
    return 0;
}

static void loft_free_ref(loft_ref r) {
    if (r.store_nr == 0xFFFE) {
        loft_gen_free(r);
        return;
    }
    if (r.store_nr == 0xFFFF || r.store_nr >= loft_nstores) return;
    if (loft_sp_n && loft_slice_give(r.store_nr)) return;
    loft_store_t *s = &loft_stores[r.store_nr];
    /* `Stores::free_named`: a store already free stays as it is. */
    if (s->pinned || !s->live) return;
    /* `@FR-G-Arg-Rebind`: a store a live generator frame names is released when the frame is
     * done with it too — this free only orphans it. */
    if (s->frame_holds) {
        s->orphaned = 1;
        return;
    }
    for (uint32_t i = 0; i < loft_nevacs;) {
        if (loft_evacs[i].old.store_nr == r.store_nr) loft_evacs[i] = loft_evacs[--loft_nevacs];
        else i++;
    }
    if (loft_release_file_leases_hook) loft_release_file_leases_hook(r.store_nr);
    s->live = 0;
#ifdef LOFT_STRICT_STORES
    /* `Stores::trim_free_top`, as the interpreter's strict mode keeps it: a freed TOP slot
     * lowers the floor, down past every dead slot under it, so the next store takes that
     * number again — a run that frees and re-makes in a loop stays within the table. */
    if (r.store_nr + 1u == loft_strict_floor)
        while (loft_strict_floor > 0 && !loft_stores[loft_strict_floor - 1].live) loft_strict_floor--;
#endif
    LOFT_TRACE("F %llu\n", (unsigned long long) s->serial);
#ifdef LOFT_FOREIGN
    /* A foreign store's bytes go with the handle: the owner drops here. */
    loft_st_release_foreign(s);
#endif
    /* A store bound to a file is cut to its content first (`free_named`: the file is
     * content-sized whether or not its author called `store_reclaim`), then unmapped, its bytes
     * kept in the file, before the poison below could reach them. */
    if (s->mapped) {
        if (!s->read_only) (void) loft_st_reclaim_tail(s);
        loft_map_release_hook(s);
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
 * outlives it; here every store made since `mark` (`loft_stores_allocated` as the test began)
 * that is still live and not pinned is released (@PLN206 D31). */
static void loft_release_stores_since(uint64_t mark) {
    for (uint32_t i = 0; i < loft_nstores; i++) {
        const loft_store_t *s = &loft_stores[i];
        if (s->live && !s->pinned && s->serial > mark) {
            loft_ref r = { (uint16_t) i, 1, 8 };
            loft_free_ref(r);
        }
    }
}

/* ── Generators (@F34, the interpreter's coroutines) ──
 * A generator is a frame — every local of its function, the label it resumes at, and whether
 * it ran to its end — and the function that resumes it: that function restores the locals
 * from the frame, jumps to the label after the last `yield`, and at the next `yield` saves
 * them and returns the value through `out`.  An iterator value names a frame's slot in
 * `loft_gens`: store number 0xFFFE, record slot + 1. */
typedef struct { int32_t state; int32_t done; } loft_gen_head;
/* `holds`: the holders that share the frame (`(G-Hold)`) — a release by one of several gives its
 * hold back, and the last one releases the frame. */
typedef struct { void *frame; void (*resume)(void *frame, void *out); void (*release)(void *frame); uint32_t holds; } loft_gen_slot;
static LOFT_TLS loft_gen_slot *loft_gens = 0;
static LOFT_TLS uint32_t loft_ngens = 0;

static loft_ref loft_gen_new(void *frame, void (*resume)(void *, void *), void (*release)(void *)) {
    uint32_t i = 0;
    while (i < loft_ngens && loft_gens[i].frame) i++;
    if (i == loft_ngens) {
        uint32_t n = loft_ngens ? loft_ngens * 2 : 16;
        loft_gen_slot *g = (loft_gen_slot *) realloc(loft_gens, n * sizeof *g);
        if (!g) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(g + loft_ngens, 0, (n - loft_ngens) * sizeof *g);
        loft_gens = g;
        loft_ngens = n;
    }
    loft_gens[i].frame = frame;
    loft_gens[i].resume = resume;
    loft_gens[i].release = release;
    loft_gens[i].holds = 1;
    loft_ref r;
    r.store_nr = 0xFFFE;
    r.rec = i + 1;
    r.pos = 0;
    return r;
}

static loft_gen_slot *loft_gen_of(loft_ref g) {
    if (g.store_nr != 0xFFFE || g.rec == 0 || g.rec > loft_ngens || !loft_gens[g.rec - 1].frame) return 0;
    return &loft_gens[g.rec - 1];
}

/* OpCoroutineNext: the generator runs to its next `yield`, whose value lands in `*out`; one
 * that has run to its end leaves `*out` as it was — the null the caller put there. */
static void loft_gen_next(loft_ref g, void *out) {
    loft_gen_slot *s = loft_gen_of(g);
    if (!s || ((loft_gen_head *) s->frame)->done) return;
    s->resume(s->frame, out);
}

/* OpCoroutineRetain (State::coroutine_retain): one more holder of the frame. */
static loft_ref loft_gen_retain(loft_ref g) {
    loft_gen_slot *s = loft_gen_of(g);
    if (s) s->holds++;
    return g;
}

/* OpCoroutineExhausted: has the generator run to its end (or is it no generator)? */
static uint8_t loft_gen_exhausted(loft_ref g) {
    loft_gen_slot *s = loft_gen_of(g);
    return (uint8_t) (!s || ((loft_gen_head *) s->frame)->done);
}

/* `Stores::hold_for_frame` (`@FR-G-Arg-Rebind`): a generator frame takes a hold on the store
 * of a heap argument it was handed; answers whether it did — the null reference, a generator
 * and a freed store are not held. */
static int loft_hold_for_frame(loft_ref r) {
    if (r.store_nr >= loft_nstores || r.rec == 0 || !loft_stores[r.store_nr].live) return 0;
    loft_stores[r.store_nr].frame_holds++;
    return 1;
}

/* `Stores::release_frame_hold`: the last hold on a store its owner already released frees it. */
static void loft_release_frame_hold(loft_ref r) {
    if (r.store_nr >= loft_nstores) return;
    loft_store_t *s = &loft_stores[r.store_nr];
    if (!s->frame_holds) return;
    s->frame_holds--;
    if (!s->frame_holds && s->orphaned) {
        s->orphaned = 0;
        loft_free_ref(r);
    }
}

static void loft_gen_free(loft_ref g) {
    loft_gen_slot *s = loft_gen_of(g);
    if (!s) return;
    if (s->holds > 1) {
        s->holds--;
        return;
    }
    s->release(s->frame);
    s->frame = 0;
}

/* A constant built at compile time (compile.rs build_const_vectors): its store's words as the
 * interpreter laid them out, loaded once and pinned for the whole run.  The vector is the
 * field at byte 8 of the store's first record. */
static loft_ref loft_const_store(const uint32_t *words, uint32_t n) {
    /* The interpreter's constant store is its own, made before the program's: it takes no
     * serial here either, so the program's stores are numbered alike. */
    uint32_t nr = loft_store_make(0);
    loft_store_t *s = &loft_stores[nr];
    if (n > s->size) {
        loft_heap_moved(s, s->size, n);
        uint8_t *b = (uint8_t *) realloc(s->buf, (size_t) n * 8);
        if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
        s->buf = b;
        s->size = n;
    }
    memcpy(s->buf, words, (size_t) n * 8);
    loft_st_resync(s);
    s->pinned = 1;
    s->read_only = 1; /* `Store::lock_constant`: constant data is read-only, as on the interpreter */
    loft_ref r = { (uint16_t) nr, 1, 8 };
    return r;
}

/* OpEqRef */
static int loft_ref_eq(loft_ref a, loft_ref b) {
    if (a.rec == 0 || b.rec == 0) return a.rec == 0 && b.rec == 0;
    return a.store_nr == b.store_nr && a.rec == b.rec && a.pos == b.pos;
}

/* OpStoreLive (Stores::store_live): the reference names a record of a store not freed. */
static int loft_store_live(loft_ref r) {
    return r.rec != 0 && r.store_nr != 0xFFFF && r.store_nr < loft_nstores && loft_stores[r.store_nr].live;
}

LOFT_HOT_INLINE loft_ref loft_field(loft_ref r, uint32_t fld) {
    r.pos += fld;
    return r;
}

LOFT_HOT_INLINE int64_t loft_get_int(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    int64_t v;
    memcpy(&v, loft_at(loft_store_of(r), r.rec, r.pos + fld), 8);
    return v;
}

/* `@FR-R-RecPtr`: the address of record view `r`'s first byte, held for a block that grows no
 * store — 0 for the null record, where every read through it answers its getter's sentinel and
 * every write is skipped, as `loft_get_*` / `loft_set_*` do — and whether its store is locked,
 * which a write through it reports as `loft_set_*`'s write check does. */
LOFT_HOT_INLINE uint8_t *loft_rec_ptr(loft_ref r) {
    return r.rec == 0 ? 0 : loft_at(loft_store_of(r), r.rec, r.pos);
}
LOFT_HOT_INLINE int loft_rec_locked(loft_ref r) { return r.rec != 0 && loft_store_of(r)->read_only; }
LOFT_HOT_INLINE int64_t loft_pa_i64(const uint8_t *p, uint32_t off) {
    int64_t v;
    if (!p) return LOFT_NULL;
    memcpy(&v, p + off, 8);
    return v;
}
LOFT_HOT_INLINE double loft_pa_f64(const uint8_t *p, uint32_t off) {
    double v;
    if (!p) return (double) NAN;
    memcpy(&v, p + off, 8);
    return v;
}
LOFT_HOT_INLINE float loft_pa_f32(const uint8_t *p, uint32_t off) {
    float v;
    if (!p) return (float) NAN;
    memcpy(&v, p + off, 4);
    return v;
}
LOFT_HOT_INLINE uint8_t loft_pa_bool(const uint8_t *p, uint32_t off) { return p ? p[off] : LOFT_BOOL_NULL; }
LOFT_HOT_INLINE void loft_pa_set(uint8_t *p, int locked, uint32_t off, const void *v, size_t n) {
    if (!p) return;
    if (LOFT_UNLIKELY(locked) && loft_locked_write()) return;
    memcpy(p + off, v, n);
}

/* The bytes of record `r` for a run of scalar field writes (`@FR-R-ValueRecord`'s copy into a
 * place): `loft_set_int`'s null test and write check once, and one address — 0 for the null
 * record, which every write of the run skips as each setter would. */
static uint8_t *loft_rec_w(loft_ref r) {
    if (r.rec == 0) return 0;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return 0;
    return loft_at(loft_store_of(r), r.rec, r.pos);
}

LOFT_HOT_INLINE void loft_set_int(loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &v, 8);
}

static void loft_set_int4(loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    int32_t x = v == LOFT_NULL ? INT32_MIN : (int32_t) v;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &x, 4);
}

/* The narrow integer fields: a byte biased by `min` (255 the nullable form's null), a short
 * in the `+1` encoding (0 is null), a raw short (0xFFFF its null), a 4-byte int (INT32_MIN).
 * A value a field cannot hold takes its default: the nullable forms' is null. */
static int32_t loft_i32_of(int64_t v) {
    return v == LOFT_NULL ? INT32_MIN : (int32_t) (uint32_t) (uint64_t) v;
}

/* Each narrow read is decoded once, by `loft_dec_*` at a field's non-null address; the null
 * record is tested once on each path in: by `loft_get_*` on the reference, by `loft_pa_*` on a
 * held record's address (`@FR-R-RecPtr`, the null address answering null). */
LOFT_HOT_INLINE int64_t loft_dec_byte(const uint8_t *q, int64_t min) { return q[0] + min; }
LOFT_HOT_INLINE int64_t loft_dec_byte_nullable(const uint8_t *q, int64_t min) {
    return q[0] == 255 ? LOFT_NULL : q[0] + min;
}

LOFT_HOT_INLINE int64_t loft_pa_byte(const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_byte(p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_byte(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_byte(loft_at(loft_store_of(r), r.rec, r.pos + fld), min);
}

LOFT_HOT_INLINE int64_t loft_pa_byte_nullable(const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_byte_nullable(p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_byte_nullable(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_byte_nullable(loft_at(loft_store_of(r), r.rec, r.pos + fld), min);
}

static uint8_t loft_byte_raw(int64_t min, int64_t val);

static void loft_set_byte_v(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    *loft_at(loft_store_of(r), r.rec, r.pos + fld) = loft_byte_raw(min, v);
}

static void loft_set_byte_nullable(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    int32_t x = loft_i32_of(v);
    if (!(x == INT32_MIN || ((int64_t) x >= min && (int64_t) x <= min + 255))) x = INT32_MIN;
    *loft_at(loft_store_of(r), r.rec, r.pos + fld) = loft_byte_raw(min, x);
}

LOFT_HOT_INLINE int64_t loft_dec_short(const uint8_t *q, int64_t min) {
    uint16_t raw;
    memcpy(&raw, q, 2);
    return raw == 0 ? LOFT_NULL : raw + min - 1;
}

LOFT_HOT_INLINE int64_t loft_pa_short(const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_short(p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_short(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_short(loft_at(loft_store_of(r), r.rec, r.pos + fld), min);
}

static void loft_set_short(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    int32_t x = loft_i32_of(v);
    /* Null is raw 0; out of range is the type's lowest value, raw 1 (loft#984). */
    uint16_t raw = 0;
    if (x != INT32_MIN) raw = (int64_t) x >= min && (int64_t) x <= min + 65534 ? (uint16_t) ((int64_t) x - min + 1) : 1;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &raw, 2);
}

LOFT_HOT_INLINE int64_t loft_dec_short_raw(const uint8_t *q, int64_t min) {
    uint16_t raw;
    memcpy(&raw, q, 2);
    return raw + min;
}

LOFT_HOT_INLINE int64_t loft_pa_short_raw(const uint8_t *p, uint32_t fld, int64_t min) {
    return p ? loft_dec_short_raw(p + fld, min) : LOFT_NULL;
}

static int64_t loft_get_short_raw(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_short_raw(loft_at(loft_store_of(r), r.rec, r.pos + fld), min);
}

static void loft_set_short_raw(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    int32_t x = loft_i32_of(v);
    uint16_t raw;
    if (x == INT32_MIN) raw = 0xFFFF;
    else if ((int64_t) x >= min && (int64_t) x <= min + 65535) raw = (uint16_t) ((int64_t) x - min);
    else raw = 0;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &raw, 2);
}

LOFT_HOT_INLINE int64_t loft_dec_int4(const uint8_t *q) {
    int32_t x;
    memcpy(&x, q, 4);
    return x == INT32_MIN ? LOFT_NULL : x;
}

LOFT_HOT_INLINE int64_t loft_pa_int4(const uint8_t *p, uint32_t fld) { return p ? loft_dec_int4(p + fld) : LOFT_NULL; }

static int64_t loft_get_int4(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_int4(loft_at(loft_store_of(r), r.rec, r.pos + fld));
}

/* OpGetInt4Raw / OpGetInt4Full / OpSetInt4Raw: an unsigned 4-byte field — the Raw form keeps
 * 0xFFFFFFFF for null, the Full form reserves nothing. */
LOFT_HOT_INLINE int64_t loft_dec_int4_raw(const uint8_t *q) {
    uint32_t x;
    memcpy(&x, q, 4);
    return x == 0xFFFFFFFFu ? LOFT_NULL : (int64_t) x;
}

LOFT_HOT_INLINE int64_t loft_pa_int4_raw(const uint8_t *p, uint32_t fld) {
    return p ? loft_dec_int4_raw(p + fld) : LOFT_NULL;
}

static int64_t loft_get_int4_raw(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_int4_raw(loft_at(loft_store_of(r), r.rec, r.pos + fld));
}

LOFT_HOT_INLINE int64_t loft_dec_int4_full(const uint8_t *q) {
    uint32_t x;
    memcpy(&x, q, 4);
    return (int64_t) x;
}

LOFT_HOT_INLINE int64_t loft_pa_int4_full(const uint8_t *p, uint32_t fld) {
    return p ? loft_dec_int4_full(p + fld) : LOFT_NULL;
}

static int64_t loft_get_int4_full(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    return loft_dec_int4_full(loft_at(loft_store_of(r), r.rec, r.pos + fld));
}

static void loft_set_int4_raw(loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    loft_set_u32(loft_store_of(r), r.rec, r.pos + fld, v == LOFT_NULL ? 0xFFFFFFFFu : (uint32_t) (uint64_t) v);
}

static uint32_t loft_get_char(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return 0;
    uint32_t c;
    memcpy(&c, loft_at(loft_store_of(r), r.rec, r.pos + fld), 4);
    return c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF) ? 0 : c;
}

static void loft_set_char(loft_ref r, uint32_t fld, uint32_t c) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &c, 4);
}

/* A whole reference held in a field (Stores::get_db_ref / set_db_ref): store, record and
 * position as three 4-byte words — how a closure record holds what it captured by reference. */
/* A removed record is deleted; when a stored reference named it, the delete leaves a
 * tombstone and that reference reads null (@FR-H-Removed, Stores::reference_removed). */
static void loft_remove_record(loft_store_t *s, uint32_t rec) {
    loft_st_delete(s, rec);
}

static int loft_record_removed(loft_ref x) {
    if (x.rec == 0 || x.store_nr == 0xFFFF || x.store_nr >= loft_nstores || !loft_stores[x.store_nr].live) return 0;
    return loft_st_is_removed(&loft_stores[x.store_nr], x.rec);
}

static loft_ref loft_get_dbref(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return loft_ref_null();
    loft_store_t *s = loft_store_of(r);
    loft_ref x;
    x.store_nr = (uint16_t) loft_u32(s, r.rec, r.pos + fld);
    x.rec = loft_u32(s, r.rec, r.pos + fld + 4);
    x.pos = loft_u32(s, r.rec, r.pos + fld + 8);
    if (loft_record_removed(x)) return loft_ref_null();
    return x;
}

static void loft_set_dbref(loft_ref r, uint32_t fld, loft_ref x) {
    if (r.rec == 0) return;
    loft_store_t *s = loft_store_of(r);
    loft_set_u32(s, r.rec, r.pos + fld, x.store_nr);
    loft_set_u32(s, r.rec, r.pos + fld + 4, x.rec);
    loft_set_u32(s, r.rec, r.pos + fld + 8, x.pos);
    /* `Stores::set_db_ref`: the record named is marked, so removing it leaves a tombstone. */
    if (x.rec != 0 && x.store_nr < loft_nstores && loft_stores[x.store_nr].live) loft_st_note_referenced(&loft_stores[x.store_nr], x.rec);
}

/* ── Vectors (src/vector.rs) ───────────────────────────────────────────────────────────── */

LOFT_HOT_INLINE uint32_t loft_collection_rec(loft_store_t *s, uint32_t rec, uint32_t pos) {
    uint32_t v = loft_u32(s, rec, pos);
    return v == 0xFFFFFFFFu ? 0 : v;
}

/* `@FR-R-SliceArg`: OpDatabase of a slice carrier — `var` takes a carrier from the pool, its
 * vector kept at length 0, so the slice that follows is copied into room the carrier already
 * has.  A variable that still holds its carrier (the site ran again before the free) keeps it.
 * A variable holding any other store, or a full pool, takes the ordinary wrapper. */
static void loft_slice_take(const struct loft_tdesc_s *T, loft_ref *var, uint32_t bytes, uint16_t tp) {
    int i = 0;
    if (var->store_nr == 0xFFFF) {
        while (i < loft_sp_n && loft_sp_used[i]) i++;
    } else {
        while (i < loft_sp_n && !(loft_sp_used[i] && loft_sp_ref[i].store_nr == var->store_nr)) i++;
        if (i == loft_sp_n) i = LOFT_SLICE_POOL;
    }
    if (i == loft_sp_n && i < LOFT_SLICE_POOL) {
        loft_ref r = loft_ref_null();
        loft_database(&r, bytes);
        loft_sp_ref[i] = r;
        loft_sp_used[i] = 0;
        loft_sp_n++;
    }
    if (i >= loft_sp_n) {
        loft_database_wrapper(T, var, bytes, tp);
        return;
    }
    loft_ref r = loft_sp_ref[i];
    loft_store_t *s = &loft_stores[r.store_nr];
#ifdef LOFT_FOREIGN
    loft_st_release_foreign(s);
#endif
    uint32_t v = loft_collection_rec(s, r.rec, r.pos);
    if (v != 0) loft_set_u32(s, v, 4, 0);
    loft_sp_used[i] = 1;
    s->pinned = 0;
    s->known = tp;
    *var = r;
}

/* `checked_vec_cap`: the header word and `count` elements of `size` bytes, in words, in 32 bits
 * (the store does no 64-bit arithmetic).  A count past what 32 bits hold answers the largest
 * size, which the store's growth refuses ("store offset overflow"). */
static uint32_t loft_vec_cap_words(uint32_t count, uint32_t size) {
    if (size != 0 && count > (UINT32_MAX - 15) / size) return UINT32_MAX / 8;
    return (count * size + 15) / 8;
}

/* `@FR-Col-Capacity` (`vector::grown_elems`): the room a vector's growth step makes, for a
 * length of `needed` — twice that, and never less than 88 bytes of elements (one at least). */
#define LOFT_VEC_START_BYTES 88u
static uint32_t loft_vec_grown(uint32_t needed, uint32_t size) {
    uint32_t start = LOFT_VEC_START_BYTES / (size ? size : 1);
    uint32_t twice = needed > UINT32_MAX / 2 ? UINT32_MAX : needed * 2;
    if (start < 1) start = 1;
    return twice > start ? twice : start;
}

LOFT_HOT_INLINE int64_t loft_vec_len(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return 0;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    return r == 0 ? 0 : (int64_t) loft_u32(s, r, 4);
}

/* A vector handed to a `#c` binding: its element bytes, valid for the call only (the store may
 * move on the next claim), NULL for an absent vector — as `c_call` hands them. */
LOFT_HOT_INLINE uint8_t *loft_cvec_ptr(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return NULL;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    return r == 0 ? NULL : loft_elem_base(s, r);
}

static void loft_vec_prealloc(loft_ref v, uint32_t count, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(v);
    if (loft_collection_rec(s, v.rec, v.pos) != 0) return;
    /* A known count is reserved exactly (`@FR-Col-Capacity`). */
    uint32_t r = loft_claim(s, loft_vec_cap_words(count, size));
    loft_set_u32(s, v.rec, v.pos, r);
    loft_set_u32(s, r, 4, 0);
}

/* vector_append: room for one more element of `size`; the reference to its slot.  The
 * length is not bumped: a push bumps it once its value is written (append_done), a new record
 * once it is finished (vector_finish). */
static loft_ref loft_vec_append_slow(loft_ref v, uint32_t size);

/* `vector::append`: room for one more element of `size` bytes at the end, the length unchanged
 * (`loft_vec_finish` counts it).  The common case — a block with room — inline: one store
 * lookup the code after it can share; a first block or a growth out of line. */
LOFT_HOT_INLINE loft_ref loft_vec_append(loft_ref v, uint32_t size) {
    if (v.store_nr != 0xFFFF && v.rec != 0) {
        loft_store_t *s = loft_store_of(v);
        uint32_t r = loft_collection_rec(s, v.rec, v.pos);
        if (r != 0) {
            uint32_t len = loft_u32(s, r, 4);
            int32_t claimed;
            memcpy(&claimed, loft_at(s, r, 0), 4);
            if ((uint64_t) (len + 1) * size + 8 <= (uint64_t) claimed * 8) {
                /* `@FR-H-WriteLocked-Whole`: a refused push answers no slot, as the slow path. */
                if (LOFT_WRITE_REFUSED(s)) return loft_ref_null();
                loft_ref e;
                e.store_nr = v.store_nr;
                e.rec = r;
                e.pos = 8 + len * size;
                return e;
            }
        }
    }
    return loft_vec_append_slow(v, size);
}

static loft_ref loft_vec_append_slow(loft_ref v, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return loft_ref_null();
    loft_store_t *s = loft_store_of(v);
    if (LOFT_WRITE_REFUSED(s)) return loft_ref_null();
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    uint32_t len;
    if (r == 0) {
        r = loft_claim(s, loft_vec_cap_words(loft_vec_grown(1, size), size));
        loft_set_u32(s, v.rec, v.pos, r);
        loft_set_u32(s, r, 4, 0);
        len = 0;
    } else {
        len = loft_u32(s, r, 4);
        int32_t claimed;
        memcpy(&claimed, loft_at(s, r, 0), 4);
        /* `len + 1 > (claimed * 8 - 8) / size`, without the division every append paid. */
        if ((uint64_t) (len + 1) * size + 8 > (uint64_t) claimed * 8) {
            uint32_t nr = loft_st_resize(s, r, loft_vec_cap_words(loft_vec_grown(len + 1, size), size));
            if (nr != r) {
                loft_set_u32(s, v.rec, v.pos, nr);
                r = nr;
            }
        }
    }
    loft_ref e;
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
LOFT_HOT_INLINE uint8_t *loft_vec_push_slot(loft_ref v, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return 0;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return 0;
    uint32_t len = loft_u32(s, r, 4);
    int32_t claimed;
    memcpy(&claimed, loft_at(s, r, 0), 4);
    if ((uint64_t) (len + 1) * size + 8 > (uint64_t) claimed * 8) return 0;
    if (LOFT_WRITE_REFUSED(s)) return 0;
    loft_set_u32(s, r, 4, len + 1);
    return loft_at(s, r, 8 + len * size);
}

/* vector_finish: the element `loft_vec_append` made counts. */
LOFT_HOT_INLINE void loft_vec_finish(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    loft_set_u32(s, r, 4, loft_u32(s, r, 4) + 1);
}

/* OpPushInt4: a 4-byte element, INT32_MIN for null. */
static void loft_vec_push_int4(loft_ref v, int64_t x) {
    uint8_t *slot = loft_vec_push_slot(v, 4);
    if (slot) {
        uint32_t w = x == LOFT_NULL ? 0x80000000u : (uint32_t) (uint64_t) x;
        memcpy(slot, &w, 4);
        return;
    }
    loft_ref e = loft_vec_append(v, 4);
    if (e.rec == 0) return;
    loft_set_u32(loft_store_of(e), e.rec, e.pos, x == LOFT_NULL ? 0x80000000u : (uint32_t) (uint64_t) x);
    loft_vec_finish(v);
}

static void loft_vec_push_int(loft_ref v, int64_t x) {
    uint8_t *slot = loft_vec_push_slot(v, 8);
    if (slot) {
        memcpy(slot, &x, 8);
        return;
    }
    loft_ref e = loft_vec_append(v, 8);
    if (e.rec == 0) return;
    loft_set_int(e, 0, x);
    loft_vec_finish(v);
}

/* A new record's defaults: the bytes the interpreter's default walk writes for its type
 * (Stores::set_default_value, captured by the emitter as `image`). */
static void loft_fill(loft_ref r, const uint8_t *image, uint32_t n) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos), image, n);
}

/* get_vector: the element at `from` (negative counts from the end), or the null reference. */
/* `@FR-R-ElementPath`: the collection record and the length read once each, one unsigned
 * compare (a negative index past the front fails it too), the element's reference built here. */
LOFT_HOT_INLINE loft_ref loft_vec_get(loft_ref v, uint32_t size, int64_t from) {
    loft_ref e = loft_ref_null();
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return e;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return e;
    int64_t len = (int64_t) loft_u32(s, r, 4);
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
typedef struct { uint16_t store_nr; uint32_t rec; int64_t len; uint8_t *base; } loft_vh;
/* `base` is the element base (`Store::elem_base`), past the record's header. */

LOFT_HOT_INLINE loft_vh loft_vh_of(loft_ref v) {
    loft_vh h;
    h.store_nr = v.store_nr;
    h.rec = 0;
    h.len = 0;
    h.base = 0;
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return h;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return h;
    h.rec = r;
    h.len = loft_u32(s, r, 4);
    h.base = loft_elem_base(s, r);
    return h;
}

/* A header held by a loop that grows a store: the record and the length stand (no growth moves
 * another vector's record or changes its length, `@FR-R-Base`), the store's memory may have
 * moved, so the base is derived again from the held record — one load, no record walk. */
LOFT_HOT_INLINE loft_vh loft_vh_fresh(loft_vh h) {
    if (h.rec != 0) h.base = loft_elem_base(&loft_stores[h.store_nr], h.rec);
    return h;
}

/* (R-Header)'s proof, checked under LOFT_VERIFY at every held access: the vector still has the
 * record, length and base it was held with. */
static int loft_vh_same(loft_vh h, loft_ref v) {
    loft_vh now = loft_vh_of(v);
    return now.store_nr == h.store_nr && now.rec == h.rec && now.len == h.len && now.base == h.base;
}

/* The element index `from` names (negative from the end), or -1 outside, as loft_vec_get. */
LOFT_HOT_INLINE int64_t loft_vh_index(loft_vh h, int64_t from) {
    int64_t f = from < 0 ? from + h.len : from;
    return f < 0 || f >= h.len ? -1 : f;
}

LOFT_HOT_INLINE loft_ref loft_vh_get(loft_vh h, uint32_t size, int64_t from) {
    int64_t f = loft_vh_index(h, from);
    if (f < 0) return loft_ref_null();
    loft_ref e;
    e.store_nr = h.store_nr;
    e.rec = h.rec;
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}

LOFT_HOT_INLINE int64_t loft_vh_get_int(loft_vh h, uint32_t size, int64_t from, uint32_t fld) {
    int64_t f = loft_vh_index(h, from);
    if (f < 0) return LOFT_NULL;
    int64_t v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}

LOFT_HOT_INLINE void loft_vh_set_int(loft_vh h, uint32_t size, int64_t from, uint32_t fld, int64_t v) {
    int64_t f = loft_vh_index(h, from);
    if (f < 0) return;
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

LOFT_HOT_INLINE double loft_vh_get_float(loft_vh h, uint32_t size, int64_t from, uint32_t fld) {
    int64_t f = loft_vh_index(h, from);
    if (f < 0) return (double) NAN;
    double v;
    memcpy(&v, h.base + (size_t) f * size + fld, 8);
    return v;
}

LOFT_HOT_INLINE void loft_vh_set_float(loft_vh h, uint32_t size, int64_t from, uint32_t fld, double v) {
    int64_t f = loft_vh_index(h, from);
    if (f < 0) return;
    memcpy(h.base + (size_t) f * size + fld, &v, 8);
}

/* ops::op_negate_long: null stays null. */
LOFT_HOT_INLINE int64_t loft_neg(int64_t a) {
    return a == LOFT_NULL ? a : -a;
}

/* OpLengthText: characters, OpSizeText: bytes; both 0 for the null text. */
static int64_t loft_text_length(loft_str s) {
    return loft_str_is_null(s) ? 0 : (int64_t) loft_chars(s);
}

static int64_t loft_text_size(loft_str s) {
    return loft_str_is_null(s) ? 0 : (int64_t) s.len;
}

/* Stores::free_displaced: free what a rebind displaced, unless it is the witness's store. */
/* A free a call's argument protection declines (Store::is_free_protected): a displaced store,
 * a copy's or a keyed write's given-up source.  A plain scope-exit free is not one. */
static int loft_free_protected(loft_ref r) {
    return r.store_nr < loft_nstores && loft_stores[r.store_nr].protect > 0;
}

/* protect_store_frees / unprotect_store_frees: the call bracket around a reference argument. */
static void loft_protect(loft_ref r) {
    if (r.store_nr < loft_nstores) loft_stores[r.store_nr].protect++;
}

static void loft_unprotect(loft_ref r) {
    if (r.store_nr < loft_nstores && loft_stores[r.store_nr].protect > 0) loft_stores[r.store_nr].protect--;
}

static void loft_free_displaced(loft_ref displaced, loft_ref witness) {
    if (displaced.store_nr == witness.store_nr || displaced.store_nr == 0xFFFF) return;
    if (loft_free_protected(displaced)) return;
    loft_free_ref(displaced);
}

/* OpFreeRef of a variable: its store released and the variable reset to the null sentinel, as
 * native's emitter resets it (and the bytecode writer's `OpInitRefSentinel` after an owned
 * rebind's free): a later free of the same variable — the next pass's pre-build free — is then
 * a no-op, where it would otherwise release whatever store reused the slot meanwhile. */
/* Inlined at every scope exit: the common case is a variable that was never minted, a test and
 * no call; the release itself (`loft_free_ref`) stays out of line. */
LOFT_HOT_INLINE void loft_free_var(loft_ref *v) {
    if (v->store_nr == 0xFFFF) return;
    loft_free_ref(*v);
    v->store_nr = 0xFFFF;
}

/* OpFreeRefIfDistinct of a variable: released and reset when it names another store than the
 * witness (native's `OpFreeRefIfDistinctEmitter`). */
LOFT_HOT_INLINE void loft_free_var_distinct(loft_ref *v, loft_ref witness) {
    if (v->store_nr == witness.store_nr || v->store_nr == 0xFFFF) return;
    if (!loft_free_protected(*v)) loft_free_ref(*v);
    v->store_nr = 0xFFFF;
}

/* `State::bind_fn_ref_result` / native's `cr_fnref_minted`: was the record a call through a
 * function value handed back MINTED by that call — a store made after `since` — rather than an
 * argument or a capture that predates it (or the closure's own record)?  A null is adopted as
 * the null it is. */
static int loft_fnref_minted(loft_ref r, uint64_t since, loft_ref clos) {
    if (r.store_nr == 0xFFFF || r.rec == 0 || r.store_nr >= loft_nstores) return 1;
    if (clos.store_nr != 0xFFFF && r.store_nr == clos.store_nr) return 0;
    if (!loft_stores[r.store_nr].live) return 1;
    return loft_stores[r.store_nr].serial > since;
}

/* The stores a call through a function value handed back that no bind adopted — the buffer it
 * delivered into, or a store it minted — each with the serial of the allocation it named
 * (`codegen_runtime::FNREF_BUFS`, `State::release_fnref_bufs`).  The destination's static type
 * may say it owns nothing (a local that borrows on one assignment and receives this store on
 * another), so the FRAME holding it is its owner: a frame whose answer is no heap value frees
 * what it holds when it ends, one that answers a heap value hands it up (`framed`). */
typedef struct { loft_ref r; uint64_t serial; } loft_fnref_held;
static LOFT_TLS loft_fnref_held *loft_fnrefs = 0;
static LOFT_TLS size_t loft_nfnrefs = 0, loft_capfnrefs = 0;

/* One allocation is one entry: an entry left by a store its owner already freed names the same
 * slot as a store minted into it later, so it is replaced where it stands. */
static void loft_fnref_hold(loft_ref r) {
    if (r.store_nr >= loft_nstores || r.rec == 0 || !loft_stores[r.store_nr].live) return;
    uint64_t serial = loft_stores[r.store_nr].serial;
    for (size_t i = 0; i < loft_nfnrefs; i++) {
        if (loft_fnrefs[i].r.store_nr == r.store_nr) {
            loft_fnrefs[i].r = r;
            loft_fnrefs[i].serial = serial;
            return;
        }
    }
    if (loft_nfnrefs == loft_capfnrefs) {
        loft_capfnrefs = loft_capfnrefs ? loft_capfnrefs * 2 : 16;
        loft_fnrefs = (loft_fnref_held *) realloc(loft_fnrefs, loft_capfnrefs * sizeof *loft_fnrefs);
        if (!loft_fnrefs) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_fnrefs[loft_nfnrefs].r = r;
    loft_fnrefs[loft_nfnrefs].serial = serial;
    loft_nfnrefs++;
}

/* `cr_fnref_buf`: the buffer a dispatch arm minted is the call's answer — held by the frame —
 * or nobody's, and released. */
static void loft_fnref_buf(loft_ref r, loft_ref hb) {
    if (r.store_nr != hb.store_nr) {
        loft_free_ref(hb);
        return;
    }
    loft_fnref_hold(hb);
}

/* `cr_fnref_minted`: a store the call minted is held by the frame; one that predates the call
 * (an argument, a capture) or is the closure's own record belongs further up. */
static void loft_fnref_mint(loft_ref r, uint64_t since, loft_ref clos) {
    if (r.store_nr == 0xFFFF || r.rec == 0 || r.store_nr >= loft_nstores) return;
    if (clos.store_nr != 0xFFFF && r.store_nr == clos.store_nr) return;
    if (!loft_stores[r.store_nr].live || loft_stores[r.store_nr].serial <= since) return;
    loft_fnref_hold(r);
}

/* `cr_fnref_adopt`: a bind that adopts the answer is its one owner; the frame lets it go. */
static void loft_fnref_adopt(loft_ref src) {
    for (size_t i = loft_nfnrefs; i-- > 0;) {
        if (loft_fnrefs[i].r.store_nr == src.store_nr) {
            /* In order: every frame's mark keeps meaning the entries above it are its own. */
            memmove(&loft_fnrefs[i], &loft_fnrefs[i + 1], (loft_nfnrefs - i - 1) * sizeof *loft_fnrefs);
            loft_nfnrefs--;
            return;
        }
    }
}

/* `FnRefBufGuard::release`: free what the ending frame still holds above `mark` — only a store
 * still the allocation it named, since the frame may have freed it itself and the slot be
 * another's by now. */
static void loft_fnref_release(size_t mark) {
    while (loft_nfnrefs > mark) {
        loft_fnref_held h = loft_fnrefs[--loft_nfnrefs];
        if (h.r.store_nr < loft_nstores && loft_stores[h.r.store_nr].live
            && loft_stores[h.r.store_nr].serial == h.serial)
            loft_free_ref(h.r);
    }
}

/* vector::clear_vector: an absent slot becomes the empty vector; a present one length 0. */
static void loft_vec_clear(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    loft_store_t *s = loft_store_of(v);
#ifdef LOFT_FOREIGN
    /* A view of foreign bytes is dropped by its clear, never written. */
    if (loft_st_is_foreign(s)) { loft_st_release_foreign(s); return; }
#endif
    if (loft_u32(s, v.rec, v.pos) == 0xFFFFFFFFu) { loft_set_u32(s, v.rec, v.pos, 0); return; }
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r != 0) loft_set_u32(s, r, 4, 0);
}


/* ── Bytes, enums and text in records ──────────────────────────────────────────────────── */

static void loft_set_byte(loft_ref r, uint32_t fld, uint8_t v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    *loft_at(loft_store_of(r), r.rec, r.pos + fld) = v;
}

/* OpCastEnumFromText (`Stores::to_enum`): the value of plain enum `names` (1-based, `n` with
 * the unused 0) whose name is `t`, 0 for none. */
static uint8_t loft_enum_of_text(const char *const *names, size_t n, loft_str t) {
    for (size_t i = 1; i < n; i++)
        if (strlen(names[i]) == t.len && memcmp(names[i], t.p, t.len) == 0) return (uint8_t) i;
    return 0;
}

/* OpGetEnum: 0 for the null reference (the null address), the stored byte otherwise. */
LOFT_HOT_INLINE uint8_t loft_pa_enum(const uint8_t *p, uint32_t fld) { return p ? p[fld] : 0; }

static uint8_t loft_get_enum(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return 0;
    return *loft_at(loft_store_of(r), r.rec, r.pos + fld);
}

static void loft_vec_push_byte(loft_ref v, uint8_t x) {
    uint8_t *slot = loft_vec_push_slot(v, 1);
    if (slot) {
        *slot = x;
        return;
    }
    loft_ref e = loft_vec_append(v, 1);
    if (e.rec == 0) return;
    *loft_at(loft_store_of(e), e.rec, e.pos) = x;
    loft_vec_finish(v);
}

/* OpRangeDefault: a value entering a narrow integer — null stays null (or takes the default
 * when one is given), a value outside [lo, hi] takes the default (a recoverable fault the
 * interpreter records silently). */
static int64_t loft_range_default(int64_t v, int64_t lo, int64_t hi, int64_t dflt) {
    if (v == LOFT_NULL) return dflt == LOFT_NULL ? v : dflt;
    if (v >= lo && v <= hi) return v;
    return dflt;
}

/* OpGetBoolean: the null boolean for the null reference, the stored byte otherwise. */
static uint8_t loft_get_bool(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_BOOL_NULL;
    return *loft_at(loft_store_of(r), r.rec, r.pos + fld);
}

/* Store::byte_raw: the byte a slot biased by `min` holds for `val` (an i32, as `OpPushByte`
 * narrows it); 255 for null, 0 for a value out of the byte's range. */
static uint8_t loft_byte_raw(int64_t min, int64_t val) {
    int32_t v = (int32_t) (uint32_t) (uint64_t) val;
    if (v == INT32_MIN) return 255;
    if ((int64_t) v >= min && (int64_t) v <= min + 255) return (uint8_t) ((int64_t) v - min);
    return 0;
}

/* vector::is_absent_collection: the null reference, no record, or the absent slot. */
static int loft_vec_absent(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return 1;
    return loft_u32(loft_store_of(v), v.rec, v.pos) == 0xFFFFFFFFu;
}

/* OpGetText (Store::get_str): the field holds a string record — its length at byte 4, its
 * bytes from byte 8 — and 0 for the null text. */
/* Store::get_str: the string record `srec` (its length @4, its bytes from @8); null for no
 * record and for a length that runs past the store, so a corrupt record reads as null, never
 * past the buffer. */
static loft_str loft_store_str(loft_store_t *s, uint32_t srec) {
    if (srec == 0 || srec > 0x7FFFFFFFu) return loft_null_text();
    uint32_t len = loft_u32(s, srec, 4);
    if (len / 8 + srec > s->size) return loft_null_text();
    return loft_lit((const char *) loft_at(s, srec, 8), len);
}

static loft_str loft_get_text(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return loft_null_text();
    loft_store_t *s = loft_store_of(r);
    return loft_store_str(s, loft_u32(s, r.rec, r.pos + fld));
}

/* OpSetText (Store::set_str): a new string record in the record's store.  A value that lives
 * in that store is copied out first, because the claim may move the buffer. */
static void loft_set_text(loft_ref r, uint32_t fld, loft_str v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    loft_store_t *s = loft_store_of(r);
    loft_text copy = { 0, 0, 0 };
    if (v.p >= (const char *) s->buf && v.p < (const char *) s->buf + (size_t) s->size * 8) {
        copy = loft_text_own(v);
        v = loft_view(&copy);
    }
    uint32_t srec = loft_st_claim(s, (uint32_t) ((v.len + 15) / 8));
    loft_set_u32(s, srec, 4, (uint32_t) v.len);
    if (v.len) memcpy(loft_at(s, srec, 8), v.p, v.len);
    loft_set_u32(s, r.rec, r.pos + fld, srec);
    loft_text_free(&copy);
}

/* OpSetTextReplace (`Store::refill_str`): the text the field held is overwritten where it lies
 * when the new one fits its record, and released otherwise (`@FR-H-TextReplace`). */
static void loft_refill_text(loft_ref r, uint32_t fld, loft_str v) {
    if (r.rec == 0) return;
    loft_store_t *s = loft_store_of(r);
    if (LOFT_WRITE_REFUSED(s)) return;
    loft_text copy = { 0, 0, 0 };
    if (v.len && v.p >= (const char *) s->buf && v.p < (const char *) s->buf + (size_t) s->size * 8) {
        copy = loft_text_own(v);
        v = loft_view(&copy);
    }
    uint32_t old = loft_u32(s, r.rec, r.pos + fld);
    if (old != 0) {
        int32_t words = loft_st_i32(s, old, 0);
        if (words > 0 && v.len + 8 <= (size_t) words * 8) {
            loft_set_u32(s, old, 4, (uint32_t) v.len);
            if (v.len) memcpy(loft_at(s, old, 8), v.p, v.len);
            loft_text_free(&copy);
            return;
        }
        loft_st_delete(s, old);
    }
    uint32_t srec = loft_st_claim(s, (uint32_t) ((v.len + 15) / 8));
    loft_set_u32(s, srec, 4, (uint32_t) v.len);
    if (v.len) memcpy(loft_at(s, srec, 8), v.p, v.len);
    loft_set_u32(s, r.rec, r.pos + fld, srec);
    loft_text_free(&copy);
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
 * file), whether it is hidden (`#`-named), and the group it is in — 1 + the field number of the
 * group's owner (the member that holds the records), 0 for none. */
typedef struct { uint16_t pos; uint16_t type; uint8_t view; uint8_t hidden; uint16_t group; } loft_fld;
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
static void (*loft_file_lease_hook)(loft_ref f);
static void (*loft_file_release_hook)(loft_ref f);

/* The hash's, the index's and the trees' halves of a copy and a release (loft_hash.h,
 * loft_btree.h, loft_radix.h). */
static void loft_hash_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to);
static void loft_hash_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed);
static void loft_ix_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to);
static void loft_ix_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed);
static void loft_rx_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to);
static void loft_rx_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed);

/* Copy `n` bytes between two places that may share a buffer a claim could move. */
static void loft_copy_bytes(loft_ref to, uint32_t to_off, loft_ref from, uint32_t from_off, size_t n) {
    if (n == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(to))) return;
    memmove(loft_at(loft_store_of(to), to.rec, to.pos + to_off),
            loft_at(loft_store_of(from), from.rec, from.pos + from_off), n);
}

/* `relink_group_views` (loft_keyed.h, which sets it): a tentative definition, so a unit without
 * the keyed collections — and so without linked groups — leaves it null. */
static void (*loft_relink_hook)(const loft_tdesc *T, uint16_t tp, loft_ref to);

static void loft_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    loft_store_t *fs = loft_store_of(from);
    switch (d->kind) {
    case LOFT_T_TEXT: {
        /* The text's block claimed, then its bytes copied straight from the source: the source
         * is re-resolved by its record number after the claim, which may have moved its store's
         * buffer when the two are one store. */
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0) { loft_set_u32(loft_store_of(to), to.rec, to.pos, 0); return; }
        uint32_t len = loft_u32(fs, cur, 4);
        loft_store_t *ts = loft_store_of(to);
        uint32_t srec = loft_st_claim(ts, (uint32_t) (((uint64_t) len + 15) / 8));
        loft_set_u32(ts, srec, 4, len);
        if (len) memcpy(loft_at(ts, srec, 8), loft_at(loft_store_of(from), cur, 8), len);
        loft_set_u32(ts, to.rec, to.pos, srec);
        return;
    }
    case LOFT_T_STRUCT: {
        /* `@FR-R-FlatCopy` (Stores::copy_struct_fields): a record that owns no heap is its bytes,
         * which came with the block copy — no field has a claim to copy. */
        if (!d->owns_heap) return;
        /* `Stores::lease_if_file`: the copy names the same OS handle, one more lease on it. */
        if (d->file == 2 && loft_file_lease_hook) loft_file_lease_hook(to);
        /* `copy_struct_fields` (@FR-Col-Group): a view member of a linked group holds the
         * records its owner holds — copied as a collection of its own it would hold copies a
         * write through the owner does not reach — so it starts empty and is indexed over the
         * copied owner's records. */
        int views = 0;
        for (uint16_t i = 0; i < d->n; i++) {
            const loft_fld *f = &d->fields[i];
            if (f->view) {
                uint32_t cur = loft_u32(fs, from.rec, from.pos + f->pos);
                loft_set_u32(loft_store_of(to), to.rec, to.pos + f->pos, cur == 0xFFFFFFFFu ? cur : 0);
                views = 1;
                continue;
            }
            loft_copy_claims(T, f->type, loft_field(from, f->pos), loft_field(to, f->pos));
        }
        if (views && loft_relink_hook) loft_relink_hook(T, tp, to);
        return;
    }
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0 || cur == 0xFFFFFFFFu) { loft_set_u32(loft_store_of(to), to.rec, to.pos, cur); return; }
        uint32_t len = loft_u32(fs, cur, 4);
        uint32_t size = T[d->elem].size;
        uint32_t into = loft_st_claim(loft_store_of(to), 1 + (uint32_t) (((uint64_t) size * len + 7) / 8));
        loft_set_u32(loft_store_of(to), to.rec, to.pos, into);
        loft_ref src_v = { from.store_nr, cur, 0 };
        loft_ref dst_v = { to.store_nr, into, 0 };
        loft_copy_bytes(dst_v, 4, src_v, 4, (size_t) len * size + 4);
        /* Stores::zero_element_tails: into a DEFINED store (an image's), a struct element's
         * tail past its last field is zeroed — the bulk copy brought the source's leftover
         * bytes there, which differ between allocators and are not the program's. */
        if (loft_store_of(to)->defined && T[d->elem].kind == LOFT_T_STRUCT) {
            const loft_tdesc *e = &T[d->elem];
            uint32_t used = 0;
            for (uint16_t k = 0; k < e->n; k++) {
                uint32_t end = (uint32_t) e->fields[k].pos + T[e->fields[k].type].size;
                if (end > used) used = end;
            }
            for (uint32_t i = 0; used < size && i < len; i++)
                memset(loft_at(loft_store_of(to), into, 8 + i * size + used), 0, size - used);
        }
        /* `@FR-R-FlatCopy`: an element that owns no heap is its bytes, which came with the
         * block copy — no element has a claim to copy. */
        if (!T[d->elem].owns_heap) return;
        for (uint32_t i = 0; i < len; i++) {
            loft_ref a = { from.store_nr, cur, 8 + i * size };
            loft_ref b = { to.store_nr, into, 8 + i * size };
            loft_copy_claims(T, d->elem, a, b);
        }
        return;
    }
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(fs, from.rec, from.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) loft_copy_claims(T, d->variants[e], from, to);
        return;
    }
    case LOFT_T_HASH:
        loft_hash_copy_claims(T, tp, from, to);
        return;
    case LOFT_T_INDEX:
        loft_ix_copy_claims(T, tp, from, to);
        return;
    case LOFT_T_RADIX:
    case LOFT_T_TRIE:
        loft_rx_copy_claims(T, tp, from, to);
        return;
    case LOFT_T_ORDERED: {
        /* Stores::copy_claims_array_body: a slot list of the source's length, each record it
         * names copied into a record of the destination's. */
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0 || cur == 0xFFFFFFFFu) { loft_set_u32(loft_store_of(to), to.rec, to.pos, cur); return; }
        uint32_t len = loft_u32(fs, cur, 4), size = T[d->elem].size;
        uint32_t into = loft_st_claim(loft_store_of(to), 1 + (len + 1) / 2);
        loft_set_u32(loft_store_of(to), to.rec, to.pos, into);
        loft_set_u32(loft_store_of(to), into, 4, len);
        for (uint32_t i = 0; i < len; i++) {
            uint32_t elm = loft_u32(loft_store_of(from), cur, 8 + 4 * i);
            if (elm == 0) {
                loft_set_u32(loft_store_of(to), into, 8 + 4 * i, 0);
                continue;
            }
            loft_ref a = { from.store_nr, elm, 8 };
            loft_ref b = { to.store_nr, loft_st_claim(loft_store_of(to), 1 + (size + 7) / 8), 8 };
            loft_copy_bytes(b, 0, a, 0, size);
            loft_set_u32(loft_store_of(to), into, 8 + 4 * i, b.rec);
            loft_copy_claims(T, d->elem, a, b);
        }
        return;
    }
    case LOFT_T_CHILD: {
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0) { loft_set_u32(loft_store_of(to), to.rec, to.pos, 0); return; }
        uint32_t size = T[d->elem].size;
        uint32_t into = loft_st_claim(loft_store_of(to), size);
        loft_ref a = { from.store_nr, cur, 8 };
        loft_ref b = { to.store_nr, into, 8 };
        loft_copy_bytes(b, 0, a, 0, size);
        loft_copy_claims(T, d->elem, a, b);
        loft_set_u32(loft_store_of(to), to.rec, to.pos, into);
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
static int (*loft_swap_blocked_hook)(void);

/* `reset_dest`: the destination must hold nothing but its root (a copy's); a keyed rebind's
 * destination is its collection's own store, whose old content leaves with the released slot. */
static int loft_swap_in(uint16_t tp, loft_ref src, loft_ref dst, int reset_dest) {
    if (src.store_nr == dst.store_nr || src.store_nr >= loft_nstores || dst.store_nr >= loft_nstores) return 0;
    if (src.rec != 1 || src.pos != 8 || dst.rec != 1 || dst.pos != 8) return 0;
    loft_store_t *a = &loft_stores[src.store_nr], *b = &loft_stores[dst.store_nr];
    if (!a->live || !b->live || a->known != tp || b->known != tp || a->pinned || b->pinned) return 0;
    /* `Store::content_swappable`: neither store locked (a lock is the slot's, and would move
     * with the content) nor serving foreign bytes nor bound to a file; only the RELEASED store's
     * protection refuses (`free_protect_depth`), a protected destination takes the content. */
    if (a->read_only || b->read_only || a->protect || a->nplaced || b->nplaced) return 0;
    if (a->mapped || b->mapped) return 0;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(a) || loft_st_is_foreign(b)) return 0;
#endif
    if (loft_swap_blocked_hook && loft_swap_blocked_hook()) return 0;
    if (a->frame_holds || b->frame_holds) return 0;
    if (reset_dest && !loft_st_holds_only_root(b)) return 0;
    /* `Store::swap_contents`: the content moves, each slot keeps its serial. */
    loft_store_t t = *a;
    *a = *b;
    *b = t;
    uint64_t serial = a->serial;
    a->serial = b->serial;
    b->serial = serial;
#ifdef LOFT_PARSE
    loft_perrs_swap(src.store_nr, dst.store_nr);
#endif
    loft_free_ref(src);
    return 1;
}

/* `@FR-H-SwapRebind` (Stores::try_swap_rebind, `codegen_runtime::OpRebindRecord`): the rebind
 * `x = f(…)` whose result the bind gives up EXCHANGES the result's store into `*dst` without
 * the reset the copy would run first — `x`'s previous tree leaves with the released slot.
 * The emitter calls it only for a direct call whose result is given up and whose type keeps no
 * stored reference (`Cx::swap_safe`); a destination that is not its store's typed root (a first
 * bind, a witnessed local) declines here, and the caller resets and copies.  Answers 1 when it
 * exchanged.  `-DLOFT_NO_SWAP_REBIND` keeps the reset and the copy. */
static int loft_swap_rebind(uint16_t tp, loft_ref src, loft_ref *dst) {
#ifdef LOFT_NO_SWAP_REBIND
    (void) tp; (void) src; (void) dst;
    return 0;
#else
    if (dst->store_nr == 0xFFFF || dst->rec != 1 || dst->pos != 8) return 0;
    return loft_swap_in(tp, src, *dst, 0);
#endif
}

#ifdef LOFT_COPY_CHECK
/* `LOFT_COPY_CHECK` (`Stores::report_copy_mismatches`, @P317): after a deep copy the source and
 * the copy are walked in parallel, and every nested collection whose length differs is warned of
 * once per field path — a copy that inflates or truncates a nested collection, which no output
 * need show.  Warn and continue, so one run is a full census.  The driver builds with it when
 * `LOFT_COPY_CHECK` is set.  Defined further on: the keyed collections' lengths. */
static int64_t loft_hash_len(loft_ref c);
static uint32_t loft_bt_len(loft_ref data);
static uint64_t loft_copy_seen[256];
static uint32_t loft_ncopy_seen = 0;

static void loft_copy_warn(const char *label, const char *path, const char *what, int64_t sl, int64_t dl, uint16_t kt) {
    uint64_t h = UINT64_C(0xcbf29ce484222325);
    for (const char *p = label; *p; p++) h = (h ^ (uint8_t) *p) * UINT64_C(0x100000001b3);
    for (const char *p = path; *p; p++) h = (h ^ (uint8_t) *p) * UINT64_C(0x100000001b3);
    for (uint32_t i = 0; i < loft_ncopy_seen; i++) if (loft_copy_seen[i] == h) return;
    if (loft_ncopy_seen < 256) loft_copy_seen[loft_ncopy_seen++] = h;
    fflush(stdout);
    fprintf(stderr, "[copy_check] MISMATCH %s%s: src_len=%lld dst_len=%lld (%s kt=%u)\n", label, path, (long long) sl,
            (long long) dl, what, (unsigned) kt);
}

static void loft_copy_cmp(const loft_tdesc *T, uint16_t tp, loft_ref s, loft_ref d, const char *label, char *path,
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
            loft_copy_cmp(T, t->fields[i].type, fs, fd, label, path, at + (size_t) (w > 0 ? w : 0), depth + 1);
            path[at] = 0;
        }
        return;
    case LOFT_T_VECTOR:
    case LOFT_T_ORDERED: {
        int64_t sl = loft_vec_len(s), dl = loft_vec_len(d);
        if (sl != dl) loft_copy_warn(label, path, t->kind == LOFT_T_VECTOR ? "vector elem" : "array/ordered", sl, dl, t->elem);
        const loft_tdesc *e = &T[t->elem];
        if (t->kind != LOFT_T_VECTOR || e->linked || !e->owns_heap || e->size == 0) return;
        uint32_t sv = loft_u32(loft_store_of(s), s.rec, s.pos), dv = loft_u32(loft_store_of(d), d.rec, d.pos);
        if (sv == 0 || dv == 0) return;
        int64_t n = sl < dl ? sl : dl;
        int w = snprintf(path + at, 256 - at, "[]");
        for (int64_t i = 0; i < n; i++) {
            loft_ref es = { s.store_nr, sv, 8 + (uint32_t) i * e->size }, ed = { d.store_nr, dv, 8 + (uint32_t) i * e->size };
            loft_copy_cmp(T, t->elem, es, ed, label, path, at + (size_t) (w > 0 ? w : 0), depth + 1);
        }
        return;
    }
    case LOFT_T_HASH: {
        int64_t sl = loft_hash_len(s), dl = loft_hash_len(d);
        if (sl != dl) loft_copy_warn(label, path, "hash", sl, dl, t->elem);
        return;
    }
    case LOFT_T_INDEX: {
        int64_t sl = loft_bt_len(s), dl = loft_bt_len(d);
        if (sl != dl) loft_copy_warn(label, path, "index", sl, dl, t->elem);
        return;
    }
    case LOFT_T_CHILD: {
        uint32_t sc = loft_u32(loft_store_of(s), s.rec, s.pos), dc = loft_u32(loft_store_of(d), d.rec, d.pos);
        if (sc == 0 || dc == 0) return;
        loft_ref cs = { s.store_nr, sc, 8 }, cd = { d.store_nr, dc, 8 };
        loft_copy_cmp(T, t->elem, cs, cd, label, path, at, depth + 1);
        return;
    }
    default:
        return;
    }
}

static void loft_report_copy_mismatches(const loft_tdesc *T, uint16_t tp, loft_ref s, loft_ref d, const char *label) {
    char path[256];
    if (s.store_nr == 0xFFFF || s.rec == 0 || d.store_nr == 0xFFFF || d.rec == 0) return;
    path[0] = 0;
    loft_copy_cmp(T, tp, s, d, label, path, 0, 0);
}
#endif

static void loft_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r);

/* OpCopyRecord's flags: bit 0 frees the source, bit 1 says the type may move (loft_swap_in). */
#ifdef LOFT_FOREIGN
static void loft_vec_add(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o);
#endif

static void loft_copy_record(const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst, int flags) {
    int free_source = flags & 1, fresh_dest = flags & 4;
    if (src.store_nr == dst.store_nr && src.rec == dst.rec && src.pos == dst.pos) return;
    if (src.store_nr == 0xFFFF) {
        /* `Stores::copy_null_into`: a fresh destination is written absent. */
        if (fresh_dest && dst.store_nr != 0xFFFF && dst.rec != 0)
            memset(loft_at(loft_store_of(dst), dst.rec, dst.pos), 0, T[tp].size);
        return;
    }
    if (dst.store_nr == 0xFFFF || dst.rec == 0) {
        if (free_source && dst.store_nr != src.store_nr && !loft_free_protected(src)) loft_free_ref(src);
        return;
    }
    if (src.rec == 0) return;
#ifdef LOFT_FOREIGN
    /* `@FR-Const-Foreign` (State::do_copy_record): a vector read out of a foreign store is copied
     * element by element, as a bind of it copies — its bytes lie outside the store's blocks,
     * where the record copy would read a record that is not there. */
    if (T[tp].kind == LOFT_T_VECTOR && loft_st_is_foreign(loft_store_of(src))) {
        loft_vec_add(T, T[tp].elem, dst, src);
        return;
    }
#endif
    if (free_source && (flags & 2) && loft_swap_in(tp, src, dst, 1)) return;
#ifdef LOFT_PARSE
    loft_perrs_deliver(src, dst, tp, 0);
#endif
    /* The destination's previous content goes first, unless it is fresh (`COPY_FRESH_DEST`). */
    if (!fresh_dest) loft_remove_claims(T, tp, dst);
    loft_copy_bytes(dst, 0, src, 0, T[tp].size);
    loft_copy_claims(T, tp, src, dst);
#ifdef LOFT_COPY_CHECK
    loft_report_copy_mismatches(T, tp, src, dst, "OpCopyRecord");
#endif
    if (free_source && src.store_nr != dst.store_nr && !loft_free_protected(src)) loft_free_ref(src);
}


#ifdef LOFT_FRAMES
/* stack_trace (codegen_runtime::n_stack_trace): a StackFrame per live frame, outermost first; a
 * frame's `line` is the site its caller recorded, the entry frame's 0.  Like `--native`'s, a
 * frame carries no arguments or variables. */
static loft_ref loft_stack_trace(void) {
    loft_ref vec = loft_ref_null();
    loft_database(&vec, LOFT_SF_SIZE * 8 - 8);
    loft_set_u32(loft_store_of(vec), vec.rec, vec.pos, 0);
    for (size_t i = 0; i < loft_nframes; i++) {
        loft_ref e = loft_vec_append(vec, LOFT_SF_SIZE);
        loft_set_text(e, LOFT_SF_FUNCTION, loft_lit(loft_frames[i].fn, strlen(loft_frames[i].fn)));
        loft_set_text(e, LOFT_SF_FILE, loft_lit(loft_frames[i].file, strlen(loft_frames[i].file)));
        loft_set_int(e, LOFT_SF_LINE, i == 0 ? 0 : (int64_t) loft_frames[i - 1].site);
        loft_set_u32(loft_store_of(e), e.rec, e.pos + LOFT_SF_ARGS, 0);
        loft_set_u32(loft_store_of(e), e.rec, e.pos + LOFT_SF_VARS, 0);
        loft_vec_finish(vec);
    }
    return vec;
}
#endif

/* OpInsertVector (vector::insert_vector): room for one element of `size` at `index` (negative
 * from the end), the elements from there on shifted up; the null reference for an index outside
 * 0..=len.  The caller writes the element's defaults (`insert_vector_element`). */
static loft_ref loft_vec_insert(loft_ref v, uint32_t size, int64_t index) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return loft_ref_null();
    int64_t len = loft_vec_len(v);
    int64_t real = index < 0 ? index + len : index;
    if (real < 0 || real > len) return loft_ref_null();
    loft_store_t *s = loft_store_of(v);
    /* Refused whole at entry (`@FR-H-WriteLocked-Whole`): with room left no resize runs, so
     * nothing below would meet the lock before the shift. */
    if (LOFT_WRITE_REFUSED(s)) return loft_ref_null();
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    uint32_t nl;
    if (r == 0) {
        r = loft_claim(s, loft_vec_cap_words(loft_vec_grown(1, size), size));
        loft_set_u32(s, v.rec, v.pos, r);
        nl = 1;
    } else {
        nl = (uint32_t) len + 1;
        /* The growth step only when the room runs out (`@FR-Col-Capacity`). */
        int32_t claimed;
        memcpy(&claimed, loft_at(s, r, 0), 4);
        uint32_t nr = nl > ((uint32_t) claimed * 8 - 8) / size
            ? loft_st_resize(s, r, loft_vec_cap_words(loft_vec_grown(nl, size), size)) : r;
        if (nr != r) {
            loft_set_u32(s, v.rec, v.pos, nr);
            r = nr;
        }
        if (len > real)
            memmove(loft_at(s, r, 8 + (uint32_t) (real + 1) * size), loft_at(s, r, 8 + (uint32_t) real * size),
                    (size_t) (len - real) * size);
    }
    loft_set_u32(s, r, 4, nl);
    loft_ref e = { v.store_nr, r, 8 + (uint32_t) real * size };
    return e;
}

/* OpReserveVector (vector::reserve_vector): room for exactly `count` elements, the record moved
 * if it has to grow; a negative count reserves nothing. */
static void loft_vec_reserve(loft_ref v, int64_t count, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    if (count < 0 || count > (int64_t) UINT32_MAX) return;
    loft_store_t *s = loft_store_of(v);
    uint32_t words = loft_vec_cap_words((uint32_t) count, size);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) {
        /* Not zeroed (`@FR-H-Claim`: a payload holds whatever the block held): an element past
         * the length is read by nothing, and every push writes its slot before the length
         * takes it in.  The length word is the one byte range a reader reads. */
        r = loft_st_claim(s, words);
        loft_set_u32(s, v.rec, v.pos, r);
        loft_set_u32(s, r, 4, 0);
        return;
    }
    uint32_t nr = loft_st_resize(s, r, words);
    if (nr != r) loft_set_u32(s, v.rec, v.pos, nr);
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
static loft_win loft_win_of(loft_ref v, uint32_t size) {
    loft_win w = { 0, 0, 0, 0, 0xFFFF };
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return w;
    loft_store_t *s = loft_store_of(v);
    w.store_nr = v.store_nr;
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return w;
    w.rec = r;
    w.len = loft_u32(s, r, 4);
    int32_t claimed;
    memcpy(&claimed, loft_at(s, r, 0), 4);
    if (!s->read_only && claimed > 0 && size != 0) w.cap = (uint32_t) (((uint64_t) claimed * 8 - 8) / size);
    w.base = loft_at(s, r, 8);
    return w;
}

/* The record's length is the window's: written at the growth arm and at the close.  A window
 * with no room never moved its length, and a locked store is not written. */
static void loft_win_sync(loft_win w) {
    if (w.rec == 0 || w.cap == 0) return;
    memcpy(loft_at(&loft_stores[w.store_nr], w.rec, 4), &w.len, 4);
}

/* vector::reserve_more: room for `extra` MORE elements than the vector holds; nothing when the
 * block already has it, or for a non-positive or unrepresentable count. */
static void loft_vec_reserve_more(loft_ref v, int64_t extra, uint32_t size) {
    if (extra <= 0 || extra == LOFT_NULL || v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    int64_t len = r == 0 ? 0 : (int64_t) loft_u32(s, r, 4);
    if (extra > (int64_t) UINT32_MAX - len) return;
    uint32_t count = (uint32_t) (len + extra);
    if (r != 0 && !s->read_only) {
        int32_t claimed;
        memcpy(&claimed, loft_at(s, r, 0), 4);
        if (claimed >= (int32_t) loft_vec_cap_words(count, size)) return;
    }
    loft_vec_reserve(v, (int64_t) count, size);
}

/* `@FR-R-PushFill`'s run clause: room for `extra` more elements before a straight-line run of
 * pushes — claimed for an empty vector, as its first push would.  A null reference and a locked
 * store are left alone — the pushes answer them as they would without it — so the reservation
 * is capacity and nothing else. */
static void loft_vec_reserve_run(loft_ref v, int64_t extra, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0 || loft_store_of(v)->read_only) return;
    loft_vec_reserve_more(v, extra, size);
}

/* The growth arm, out of line: the length synced, the element appended the ordinary way (which
 * grows, refuses a locked store, drops a push into an absent vector), the window re-derived. */
LOFT_COLD static loft_win loft_win_grow(loft_win w, loft_ref v, uint32_t size, uint64_t bits) {
    loft_win_sync(w);
    loft_ref e = loft_vec_append(v, size);
    if (e.rec != 0) {
        uint8_t *at = loft_at(loft_store_of(e), e.rec, e.pos);
        if (size == 8) memcpy(at, &bits, 8);
        else if (size == 4) { uint32_t b = (uint32_t) bits; memcpy(at, &b, 4); }
        else { uint8_t b = (uint8_t) bits; *at = b; }
        loft_vec_finish(v);
    }
    return loft_win_of(v, size);
}

/* The slow path of a push in place: the window grown through a COPY, so the caller's window never
 * has its address handed to a call — Clang then keeps its fields in registers across the loop,
 * where `*w = loft_win_grow(*w, …)` (a 24-byte struct passed and returned through memory) kept
 * the whole window on the stack and reloaded it at every push. */
LOFT_HOT_INLINE void loft_win_grow_into(loft_win *w, loft_ref v, uint32_t size, uint64_t bits) {
    loft_win t = *w;
    t = loft_win_grow(t, v, size, bits);
    *w = t;
}

/* A push through the window: one comparison, one store and a bump while it fits (the element's
 * bytes as the ordinary push writes them, encoded by the caller), the growth arm otherwise. */
LOFT_HOT_INLINE void loft_win_put8(loft_win *w, loft_ref v, uint64_t bits) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        memcpy(w->base + (size_t) w->len * 8, &bits, 8);
        w->len++;
    } else {
        loft_win_grow_into(w, v, 8, bits);
    }
}

LOFT_HOT_INLINE void loft_win_put4(loft_win *w, loft_ref v, uint32_t bits) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        memcpy(w->base + (size_t) w->len * 4, &bits, 4);
        w->len++;
    } else {
        loft_win_grow_into(w, v, 4, bits);
    }
}

LOFT_HOT_INLINE void loft_win_put1(loft_win *w, loft_ref v, uint8_t bits) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        w->base[w->len] = bits;
        w->len++;
    } else {
        loft_win_grow_into(w, v, 1, bits);
    }
}

/* `@FR-R-PushFill`'s record clause (Stores::push_record_windowed): the window's next slot for a
 * record mint, the window grown first when it is full — the length synced and one element's
 * room reserved the runtime's way.  The element stays invisible until the group's finish bumps
 * the window (`loft_win_finish`).  A vector the window cannot serve (absent, or in a locked store)
 * mints the ordinary way, so it drops or refuses exactly as before. */
LOFT_COLD static loft_win loft_win_room(loft_win w, loft_ref v, uint32_t size) {
    loft_win_sync(w);
    loft_vec_reserve_more(v, 1, size);
    return loft_win_of(v, size);
}

LOFT_HOT_INLINE loft_ref loft_win_slot(loft_win *w, loft_ref v, uint32_t size) {
    if (LOFT_UNLIKELY(w->len >= w->cap)) {
        *w = loft_win_room(*w, v, size);
        if (w->len >= w->cap) return loft_vec_append(v, size);
    }
    loft_ref e;
    e.store_nr = w->store_nr;
    e.rec = w->rec;
    e.pos = 8 + w->len * size;
    return e;
}

/* The group's finish: the minted element counts.  One the window could not serve was minted
 * and is finished the ordinary way, and the window re-derived from the record it changed. */
LOFT_HOT_INLINE void loft_win_finish(loft_win *w, loft_ref v, uint32_t size) {
    if (LOFT_LIKELY(w->len < w->cap)) {
        w->len++;
    } else {
        loft_vec_finish(v);
        *w = loft_win_of(v, size);
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

LOFT_HOT_INLINE loft_ph loft_ph_of(loft_ref v, uint32_t size) {
    loft_ph h;
    h.v = v;
    h.rec = 0;
    h.len = 0;
    h.cap = 0;
    h.base = 0;
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return h;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return h;
    h.rec = r;
    h.base = loft_at(s, r, 0);
    h.len = loft_u32(s, r, 4);
    if (s->read_only) return h;
    int32_t claimed;
    memcpy(&claimed, loft_at(s, r, 0), 4);
    h.cap = claimed < 1 ? 0 : (uint32_t) (((uint64_t) claimed * 8 - 8) / size);
    return h;
}

/* `@FR-R-Refresh`: the growth re-derives the whole header at its own site. */
LOFT_COLD static loft_ref loft_ph_grow(loft_ph *h, uint32_t size) {
    loft_ref e = loft_vec_append(h->v, size);
    *h = loft_ph_of(h->v, size);
    return e;
}

LOFT_HOT_INLINE loft_ref loft_ph_new(loft_ph *h, uint32_t size) {
    if (LOFT_UNLIKELY(h->len >= h->cap)) return loft_ph_grow(h, size);
    loft_ref e;
    e.store_nr = h->v.store_nr;
    e.rec = h->rec;
    e.pos = 8 + h->len * size;
    return e;
}

LOFT_HOT_INLINE void loft_ph_finish(loft_ph *h) {
    if (LOFT_UNLIKELY(h->rec == 0)) {
        loft_vec_finish(h->v);
        return;
    }
    h->len++;
    memcpy(loft_at(&loft_stores[h->v.store_nr], h->rec, 4), &h->len, 4);
}

/* `@FR-R-Push`: a scalar push through a held push header — the slot past the length while it
 * fits (one bounds test), the element's encoded bytes, the length bump written to the header and
 * the record; a growth is the runtime's own append and a fresh header (`loft_ph_grow`).  An
 * absent or refused slot writes nothing, as the plain push. */
#define LOFT_PH_PUT(W, T)                                                                   \
    LOFT_HOT_INLINE void loft_ph_put##W(loft_ph *h, T bits) {                               \
        loft_ref e = loft_ph_new(h, W);                                                     \
        if (LOFT_UNLIKELY(e.rec == 0)) return;                                              \
        memcpy(loft_at(&loft_stores[e.store_nr], e.rec, e.pos), &bits, W);                  \
        loft_ph_finish(h);                                                                  \
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
    LOFT_HOT_INLINE void loft_ph_putb##W(loft_ph *h, T bits) {                              \
        if (LOFT_LIKELY(h->len < h->cap)) {                                                 \
            memcpy(h->base + 8 + (size_t) h->len * W, &bits, W);                            \
            h->len++;                                                                       \
            memcpy(h->base + 4, &h->len, 4);                                                \
            return;                                                                         \
        }                                                                                   \
        loft_ph_put##W(h, bits);                                                            \
    }
LOFT_PH_PUTB(8, uint64_t)
LOFT_PH_PUTB(4, uint32_t)
LOFT_PH_PUTB(1, uint8_t)

/* `LOFT_VERIFY`'s proof for a push header: a fresh derivation answers its record, length and room. */
static int loft_ph_same(loft_ph h, uint32_t size) {
    loft_ph f = loft_ph_of(h.v, size);
    return f.rec == h.rec && f.len == h.len && f.cap == h.cap;
}

/* `LOFT_VERIFY`'s proof for a push header that writes through its address: the address too is
 * what a fresh derivation answers — nothing moved the store since the header was derived. */
static int loft_phb_same(loft_ph h, uint32_t size) {
    loft_ph f = loft_ph_of(h.v, size);
    return loft_ph_same(h, size) && f.base == h.base;
}

/* `LOFT_VERIFY`'s proof for a windowed push: the window still describes the vector — its
 * record, element base and room are what a fresh derivation answers. */
static int loft_win_same(loft_win w, loft_ref v, uint32_t size) {
    loft_win f = loft_win_of(v, size);
    return f.rec == w.rec && f.base == w.base && f.cap == w.cap && f.store_nr == w.store_nr;
}

LOFT_HOT_INLINE uint64_t loft_f64_bits(double x) { uint64_t b; memcpy(&b, &x, 8); return b; }
LOFT_HOT_INLINE uint32_t loft_f32_bits(float x) { uint32_t b; memcpy(&b, &x, 4); return b; }
LOFT_HOT_INLINE uint32_t loft_i4_bits(int64_t x) { return x == LOFT_NULL ? 0x80000000u : (uint32_t) (uint64_t) x; }

static void loft_vec_set_size(loft_ref v, uint32_t adding, uint32_t size);

/* OpKeepRange (Stores::vector_keep_range): `v = v[lo..hi]` of scalar elements, kept in place —
 * the span moved to the front of its record and the length set.  A foreign view takes the copy
 * form: the span copied out, the view dropped, the span appended (`append_span`). */
static void loft_vec_keep_range(loft_ref v, int64_t lo, int64_t hi, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    int64_t n = loft_vec_len(v);
    lo = lo < 0 ? 0 : lo > n ? n : lo;
    hi = hi < lo ? lo : hi > n ? n : hi;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return;
    size_t bytes = (size_t) (hi - lo) * size;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(s)) {
        uint8_t *span = (uint8_t *) malloc(bytes ? bytes : 1);
        if (!span) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(span, loft_at(s, r, 8 + (uint32_t) lo * size), bytes);
        loft_vec_clear(v);
        if (hi > lo) {
            loft_ref e = loft_vec_append(v, size);
            loft_vec_set_size(v, (uint32_t) (hi - lo), size);
            loft_store_t *d = loft_store_of(v);
            memcpy(loft_at(d, loft_collection_rec(d, v.rec, v.pos), e.pos), span, bytes);
        }
        free(span);
        return;
    }
#endif
    if (bytes) memmove(loft_at(s, r, 8), loft_at(s, r, 8 + (uint32_t) lo * size), bytes);
    loft_set_u32(s, r, 4, (uint32_t) (hi - lo));
}

/* OpMoveField (Stores::move_field_out): a field's value moved into `dst` — within one store its
 * bytes relocate; across two the destination takes a deep copy and the source releases what it
 * held.  Either way the source is left zeroed. */
static void loft_move_field(const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0
        || src.store_nr >= loft_nstores || dst.store_nr >= loft_nstores) return;
    uint32_t size = T[tp].size;
    loft_copy_bytes(dst, 0, src, 0, size);
    if (src.store_nr != dst.store_nr) {
        loft_copy_claims(T, tp, src, dst);
        loft_remove_claims(T, tp, src);
    }
    if (size) memset(loft_at(loft_store_of(src), src.rec, src.pos), 0, size);
}

/* OpRemoveVector (vector::remove_vector): the element at `index` (negative from the end) goes,
 * its successors move down; false for a null vector or an index outside it. */
static int loft_vec_remove(loft_ref v, uint32_t size, int64_t index) {
    if (v.store_nr == 0xFFFF) return 0;
    if (v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(v))) return 0;
    int64_t len = loft_vec_len(v);
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (index == LOFT_NULL) return 0;
    int64_t i = index < 0 ? index + len : index;
    if (i >= len || i < 0 || r == 0) return 0;
    if (len - i > 1) {
        memmove(loft_at(s, r, 8 + (uint32_t) i * size), loft_at(s, r, 8 + (uint32_t) (i + 1) * size),
                (size_t) (len - i - 1) * size);
    }
    loft_set_u32(s, r, 4, (uint32_t) (len - 1));
    return 1;
}

/* OpReplaceVector (Stores::vector_replace): `v = o` for a vector — an absent source leaves
 * the destination absent, the same vector is left alone, anything else is cleared and
 * refilled with deep copies of the source's elements.  The clear RELEASES what the old
 * elements own (`@FR-H-ClearRelease`): the destination may be a live vector. */
static void loft_vec_add(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o);
static void loft_vec_clear_release_body(const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v);

static void loft_vec_replace(const loft_tdesc *T, uint32_t nt, uint16_t tp, loft_ref v, loft_ref o) {
#ifdef LOFT_PARSE
    loft_perrs_deliver_vector(o, v, 1);
#endif
    int absent = o.store_nr == 0xFFFF
        || (o.rec != 0 && o.pos != 0 && loft_u32(loft_store_of(o), o.rec, o.pos) == 0xFFFFFFFFu);
    if (absent) {
        loft_vec_clear_release_body(T, nt, v);
        if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0) loft_set_u32(loft_store_of(v), v.rec, v.pos, 0xFFFFFFFFu);
        return;
    }
    if (v.store_nr == o.store_nr) {
        uint32_t dr = loft_u32(loft_store_of(v), v.rec, v.pos);
        if (dr != 0 && dr == loft_u32(loft_store_of(o), o.rec, o.pos)) return;
    }
    loft_vec_clear_release_body(T, nt, v);
    loft_vec_add(T, tp, v, o);
}

/* OpAppendCopy (State::append_copy): `[x; n]` — the template is the vector's last element and
 * `n` counts it, so n - 1 deep copies of it follow; n <= 0 removes the template too.  As
 * Stores::fill_from_template: one append and one resize to exactly the total (the record the
 * interpreter's layout has), the template copied in doubling blocks — each copy takes as many
 * elements as are already there — and the claims copied per element only for a type that
 * holds any. */
static void loft_vec_append_copy(const loft_tdesc *T, uint16_t ctp, loft_ref v, int64_t count) {
    uint32_t multiply = count <= 0 ? 0 : count > (int64_t) UINT32_MAX ? UINT32_MAX : (uint32_t) count;
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (multiply == 0) {
        if (r != 0) {
            uint32_t len = loft_u32(s, r, 4);
            loft_set_u32(s, r, 4, len == 0 ? 0 : len - 1);
        }
        return;
    }
    uint32_t length = (uint32_t) loft_vec_len(v);
    uint32_t size = T[ctp].size;
    uint32_t extra = multiply - 1;
    if (extra == 0 || length == 0) return;
    (void) loft_vec_append(v, size);
    loft_vec_set_size(v, extra, size);
    s = loft_store_of(v);
    r = loft_collection_rec(s, v.rec, v.pos);
    uint32_t from = 8 + (length - 1) * size;
    uint8_t *base = (uint8_t *) loft_at(s, r, 0);
    for (uint32_t done = 1; done < multiply;) {
        uint32_t n = done < multiply - done ? done : multiply - done;
        memcpy(base + from + (size_t) done * size, base + from, (size_t) n * size);
        done += n;
    }
    if (T[ctp].kind != LOFT_T_SCALAR && T[ctp].kind != LOFT_T_DBREF) {
        loft_ref a = { v.store_nr, r, from };
        for (uint32_t i = 1; i <= extra; i++) {
            loft_ref e = { v.store_nr, r, from + i * size };
            loft_copy_claims(T, ctp, a, e);
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
static void loft_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r);

static void loft_remove_claims_mode(const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    const loft_tdesc *d = &T[tp];
    /* `Stores::release_file_handle`: a `File` record that dies returns its lease. */
    if (d->file == 2 && !borrowed && loft_file_release_hook) loft_file_release_hook(r);
    /* `@FR-R-FlatCopy`: a record that owns no heap holds no claim to release. */
    if (d->kind == LOFT_T_STRUCT && !d->owns_heap) return;
    loft_store_t *s = loft_store_of(r);
    if (d->kind == LOFT_T_TEXT) {
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur != 0) {
            loft_st_delete(s, cur);
            loft_set_u32(s, r.rec, r.pos, 0);
        }
        return;
    }
    if (d->kind == LOFT_T_VECTOR || d->kind == LOFT_T_CHILD || d->kind == LOFT_T_HASH || d->kind == LOFT_T_INDEX ||
        d->kind == LOFT_T_ORDERED || d->kind == LOFT_T_RADIX || d->kind == LOFT_T_TRIE) {
        /* An absent collection slot owns nothing and reads as empty afterwards. */
        if (loft_u32(s, r.rec, r.pos) == 0xFFFFFFFFu) { loft_set_u32(s, r.rec, r.pos, 0); return; }
    }
    if (!borrowed && !d->owns_heap) return;
    switch (d->kind) {
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) {
            if (!d->fields[i].view && !T[d->fields[i].type].owns_heap) continue;
            loft_remove_claims_mode(T, d->fields[i].type, loft_field(r, d->fields[i].pos), d->fields[i].view);
        }
        return;
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur != 0) {
            uint32_t len = loft_u32(s, cur, 4), size = T[d->elem].size;
            if (T[d->elem].owns_heap) {
                for (uint32_t i = 0; i < len; i++) {
                    loft_ref e = { r.store_nr, cur, 8 + i * size };
                    loft_remove_claims(T, d->elem, e);
                }
            }
            loft_st_delete(loft_store_of(r), cur);
        }
        loft_set_u32(loft_store_of(r), r.rec, r.pos, 0);
        return;
    }
    case LOFT_T_CHILD: {
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur != 0) {
            loft_ref c = { r.store_nr, cur, 8 };
            loft_remove_claims(T, d->elem, c);
            loft_st_delete(loft_store_of(r), cur);
        }
        loft_set_u32(loft_store_of(r), r.rec, r.pos, 0);
        return;
    }
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(s, r.rec, r.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) loft_remove_claims(T, d->variants[e], r);
        return;
    }
    case LOFT_T_HASH:
        loft_hash_remove_claims(T, tp, r, borrowed);
        return;
    case LOFT_T_INDEX:
        loft_ix_remove_claims(T, tp, r, borrowed);
        return;
    case LOFT_T_RADIX:
    case LOFT_T_TRIE:
        loft_rx_remove_claims(T, tp, r, borrowed);
        return;
    case LOFT_T_ORDERED: {
        /* Every record the slot list names (not a view's), then the list. */
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur != 0) {
            if (!borrowed) {
                for (uint32_t i = 0; i < loft_u32(s, cur, 4); i++) {
                    uint32_t elm = loft_u32(s, cur, 8 + 4 * i);
                    if (elm == 0) continue;
                    loft_ref e = { r.store_nr, elm, 8 };
                    loft_remove_claims(T, d->elem, e);
                    loft_st_delete(s, elm);
                }
            }
            loft_st_delete(s, cur);
        }
        loft_set_u32(s, r.rec, r.pos, 0);
        return;
    }
    default:
        return;
    }
}

static void loft_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r) { loft_remove_claims_mode(T, tp, r, 0); }

/* vector::reached_capacity: the elements `v`'s claim holds room for, 0 when it has none. */
static uint32_t loft_vec_reached(loft_ref v, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return 0;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return 0;
    int32_t words = loft_st_i32(s, r, 0);
    return words <= 0 ? 0 : (uint32_t) (((uint64_t) words * 8 - 8) / size);
}

/* OpClearVector (Stores::clear_vector_release): `v` emptied, releasing what its elements own.
 * A vector that is the whole content of its store's root record (a one-field root type) whose
 * elements own heap resets the store instead — made again with the root alone, the vector
 * pre-sized to the room it had reached — rather than walking every element. */
static void loft_vec_clear_release_body(const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v);

/* `v = []`: the common case — a vector already empty, which has nothing to release and whose
 * clear is nothing — answered inline; everything else out of line. */
LOFT_HOT_INLINE void loft_vec_clear_release(const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
#ifndef LOFT_FOREIGN
    {
        loft_store_t *s = loft_store_of(v);
        uint32_t cur = loft_u32(s, v.rec, v.pos);
        if (cur == 0) return;
        if (cur != 0xFFFFFFFFu && loft_u32(s, cur, 4) == 0) return;
    }
#endif
    loft_vec_clear_release_body(T, loft_ntypes, v);
}

static void loft_vec_clear_release_body(const loft_tdesc *T, uint32_t loft_ntypes, loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    loft_store_t *s = loft_store_of(v);
#ifdef LOFT_FOREIGN
    /* A local bound to a view: the clear drops it, and the store is an ordinary empty one. */
    if (loft_st_is_foreign(s)) { loft_st_release_foreign(s); return; }
#endif
    /* A vector already empty — no block, or a block of length 0 — has nothing to release, and its
     * clear is nothing (`v = []` on a vector that holds nothing).  An absent slot still takes the
     * clear, which makes it the empty vector. */
    uint32_t cur = loft_u32(s, v.rec, v.pos);
    if (cur == 0) return;
    if (cur != 0xFFFFFFFFu) {
        uint32_t r0 = loft_collection_rec(s, v.rec, v.pos);
        if (r0 != 0 && loft_u32(s, r0, 4) == 0) return;
    }
    if (v.rec == 1) {
        uint16_t kt = s->known;
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
                uint32_t reached = loft_vec_reached(v, size);
                loft_st_init(s);
                uint32_t root = loft_st_claim(s, 1 + (T[kt].size + 7u) / 8);
                memset(s->buf + (size_t) root * 8 + 4, 0, (size_t) loft_st_i32(s, root, 0) * 8 - 4);
                loft_set_u32(s, root, 4, kt);
                loft_vec_prealloc(v, reached, size);
                return;
            }
            uint32_t r = loft_collection_rec(s, v.rec, v.pos);
            if (r != 0) {
                uint32_t len = loft_u32(s, r, 4);
                for (uint32_t i = 0; i < len; i++) {
                    loft_ref e = { v.store_nr, r, 8 + i * size };
                    loft_remove_claims(T, elem, e);
                }
            }
        }
    }
    loft_vec_clear(v);
}

/* OpAppendVector (Stores::vector_add): every element of `o` appended to `v`, deep-copied so the
 * destination owns what each element holds.  The source's length is read once, and each source
 * element is re-read after the growth, so a self-append (`v += v`) reads the moved record. */
/* Stores::vector_set_size: the vector grows by `adding` elements of `size`, its claim resized to
 * hold exactly them when more than one arrives. */
static void loft_vec_set_size(loft_ref v, uint32_t adding, uint32_t size) {
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_u32(s, v.rec, v.pos);
    uint32_t len = loft_u32(s, r, 4);
    if (adding > 1) {
        uint32_t nr = loft_st_resize(s, r, (uint32_t) (((uint64_t) (len + adding) * size + 15) / 8));
        if (nr != r) {
            loft_set_u32(s, v.rec, v.pos, nr);
            r = nr;
        }
    }
    loft_set_u32(s, r, 4, len + adding);
}

/* Stores::vector_add (OpAppendVector): the elements of `o` appended to `v` as deep copies.  When
 * they own no heap and fit the room `v` has, one block copy (`vector_add_fits`); into an empty
 * `v`, a claim of exactly their size; else `vector_append` and a resize to the total. */
static void loft_vec_add(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
#ifdef LOFT_PARSE
    loft_perrs_deliver_vector(o, v, 0);
#endif
    int64_t n64 = loft_vec_len(o);
    if (n64 == 0 || v.store_nr == 0xFFFF || v.rec == 0) return;
    uint32_t n = (uint32_t) n64;
    uint32_t size = T[tp].size;
    loft_store_t *s = loft_store_of(v);
    uint32_t o_rec = loft_u32(loft_store_of(o), o.rec, o.pos);
    uint32_t d_rec = loft_collection_rec(s, v.rec, v.pos);
    int same_vec = v.store_nr == o.store_nr && o_rec != 0 && loft_u32(s, v.rec, v.pos) == o_rec;
    /* A foreign destination has no claimed header to read (`append_block_fits` refuses it); it is
     * locked, so the append below refuses the write. */
    if (size != 0 && !T[tp].owns_heap && d_rec != 0 && d_rec != LOFT_FOREIGN_REC
        && !(v.store_nr == o.store_nr && d_rec == o_rec)) {
        int32_t claimed = loft_st_i32(s, d_rec, 0);
        uint32_t len = loft_u32(s, d_rec, 4);
        uint32_t cap = (uint32_t) (((uint64_t) (claimed > 0 ? claimed : 0) * 8 - 8) / size);
        if (claimed > 0 && (uint64_t) len + n <= cap) {
            loft_ref dst = { v.store_nr, d_rec, 0 }, src = { o.store_nr, o_rec, 0 };
            loft_copy_bytes(dst, 8 + len * size, src, 8, (size_t) n * size);
            loft_set_u32(s, d_rec, 4, len + n);
            return;
        }
    }
    if (!same_vec && loft_u32(s, v.rec, v.pos) == 0) {
        uint32_t rec = loft_claim(s, (uint32_t) (((uint64_t) n * size + 15) / 8));
        loft_set_u32(s, v.rec, v.pos, rec);
        loft_set_u32(s, rec, 4, 0);
    }
    loft_ref slot = loft_vec_append(v, size);
    uint32_t append_pos = slot.pos;
    loft_vec_set_size(v, n, size);
    uint32_t dest = loft_u32(s, v.rec, v.pos);
    if (v.store_nr == o.store_nr) o_rec = loft_u32(loft_store_of(o), o.rec, o.pos);
    loft_ref dst = { v.store_nr, dest, 0 }, src = { o.store_nr, o_rec, 0 };
    loft_copy_bytes(dst, append_pos, src, 8, (size_t) n * size);
    if (!T[tp].owns_heap) return;
    for (uint32_t i = 0; i < n; i++) {
        loft_ref a = { o.store_nr, o_rec, 8 + i * size };
        loft_ref b = { v.store_nr, dest, append_pos + i * size };
        loft_copy_claims(T, tp, a, b);
    }
}

/* OpPrependVector (Stores::prepend_vector): `g`'s elements added as OpAppendVector adds them,
 * then rotated to the front.  0, having done nothing, for a null or absent vector or a linked
 * element type — the rewrite keeps the written-out statements as the fallback. */
static uint8_t loft_vec_prepend(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref g, int linked) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0 || linked) return 0;
    if (loft_u32(loft_store_of(v), v.rec, v.pos) == 0xFFFFFFFFu) return 0;
    int64_t before = loft_vec_len(v);
    loft_vec_add(T, tp, v, g);
    int64_t after = loft_vec_len(v);
    if (before == 0 || after <= before) return 1;
    size_t size = T[tp].size, moved = (size_t) (after - before) * size, total = (size_t) after * size;
    uint8_t *e = loft_at(loft_store_of(v), loft_collection_rec(loft_store_of(v), v.rec, v.pos), 8);
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
static void loft_vec_adopt(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
#ifdef LOFT_PARSE
    loft_perrs_deliver_vector(o, v, 1);
#endif
#if defined(LOFT_FOREIGN) && !defined(LOFT_NO_FOREIGN_VIEW)
    if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0 && o.store_nr != 0xFFFF && o.rec != 0 && o.pos != 0
        && v.store_nr != o.store_nr) {
        int64_t n = loft_vec_len(o);
        if (n > 0 && loft_st_is_foreign(loft_store_of(o))) {
            loft_fspan span = loft_st_foreign_span(loft_store_of(o), 0, (uint32_t) n);
            loft_store_t *s = loft_store_of(v);
            loft_st_release_foreign(s);
            loft_st_make_foreign(s, v.rec, v.pos, span);
            return;
        }
    }
#endif
    loft_vec_add(T, tp, v, o);
}

/* OpMoveVector (Stores::move_vector, @FR-R-MoveLast): a local vector given up to a field.  In
 * one store, into an empty slot, the vector's record moves and the source slot empties; else
 * the elements are appended as deep copies. */
static void loft_move_vector(const loft_tdesc *T, uint16_t tp, loft_ref dst, loft_ref src) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    uint32_t h = loft_collection_rec(loft_store_of(src), src.rec, src.pos);
    if (h == 0) return;
    if (src.store_nr == dst.store_nr && loft_collection_rec(loft_store_of(dst), dst.rec, dst.pos) == 0) {
        loft_set_u32(loft_store_of(dst), dst.rec, dst.pos, h);
        loft_set_u32(loft_store_of(src), src.rec, src.pos, 0);
        return;
    }
    loft_vec_add(T, tp, dst, src);
}

/* OpReverseVector (vector::reverse_vector): the elements in the opposite order. */
static void loft_vec_reverse(loft_ref v, uint32_t size) {
    int64_t n = loft_vec_len(v);
    if (n < 2) return;
    loft_store_t *s = loft_store_of(v);
    if (LOFT_WRITE_REFUSED(s)) return;
    uint8_t *base = loft_at(s, loft_collection_rec(s, v.rec, v.pos), 8);
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
static int loft_flt(double a, double b);
#define LOFT_FLOAT_SORT(NAME, BITS, FLT)                                                         \
    static inline int NAME##_lt(BITS a, BITS b) {                                                \
        FLT x, y;                                                                                \
        memcpy(&x, &a, sizeof x);                                                                \
        memcpy(&y, &b, sizeof y);                                                                \
        return loft_flt((double) x, (double) y); /* `<` itself, one home */                       \
    }                                                                                            \
    static void NAME##_ins(BITS *a, size_t n) {                                                  \
        for (size_t i = 1; i < n; i++) {                                                         \
            BITS v = a[i];                                                                       \
            size_t j = i;                                                                        \
            while (j > 0 && NAME##_lt(v, a[j - 1])) {                                            \
                a[j] = a[j - 1];                                                                 \
                j--;                                                                             \
            }                                                                                    \
            a[j] = v;                                                                            \
        }                                                                                        \
    }                                                                                            \
    static void NAME(uint8_t *base, size_t n) {                                                  \
        BITS small[16];                                                                          \
        BITS *a = n <= 16 ? small : (BITS *) malloc(2 * n * sizeof(BITS));                       \
        if (!a) { fputs("loft: out of memory\n", stderr); abort(); }                            \
        memcpy(a, base, n * sizeof(BITS));                                                       \
        if (n <= 16) {                                                                           \
            NAME##_ins(a, n);                                                                    \
        } else {                                                                                 \
            for (size_t lo = 0; lo < n; lo += 16) NAME##_ins(a + lo, n - lo < 16 ? n - lo : 16); \
            BITS *src = a, *dst = a + n;                                                         \
            for (size_t w = 16; w < n; w *= 2) {                                                 \
                for (size_t lo = 0; lo < n; lo += 2 * w) {                                       \
                    size_t mid = lo + w < n ? lo + w : n, hi = lo + 2 * w < n ? lo + 2 * w : n;  \
                    size_t i = lo, j = mid, k = lo;                                              \
                    while (i < mid && j < hi) dst[k++] = NAME##_lt(src[j], src[i]) ? src[j++] : src[i++]; \
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

/* The store a text-vector sort compares in (qsort has no context). */
static LOFT_TLS loft_store_t *loft_sort_store = 0;
static int loft_cmp_text_rec(const void *a, const void *b) {
    uint32_t x, y;
    memcpy(&x, a, 4);
    memcpy(&y, b, 4);
    loft_store_t *s = loft_sort_store;
    /* `@FR-E-NullArg`: the order of `<` — a null text before every text, `""` included
     * (loft#2236); equal texts are one value, so `qsort`'s instability is not observable. */
    loft_str a_ = loft_store_str(s, x), b_ = loft_store_str(s, y);
    return loft_text_lt(a_, b_) ? -1 : loft_text_lt(b_, a_) ? 1 : 0;
}

/* OpSortVector (vector::sort_vector, sort_text_vector): ascending, by the element's width and
 * kind — a text and a float in the order of `<` (null first, `@FR-E-NullArg`), a float stably
 * (`loft_sort_f32`/`_f64`), a byte or short by its stored code. */
static void loft_vec_sort(loft_ref v, uint32_t size, int is_float, int is_text) {
    int64_t n = loft_vec_len(v);
    if (n < 2) return;
    loft_store_t *s = loft_store_of(v);
    if (LOFT_WRITE_REFUSED(s)) return;
    void *base = loft_at(s, loft_collection_rec(s, v.rec, v.pos), 8);
    int (*cmp)(const void *, const void *) = 0;
    if (is_text) {
        loft_sort_store = s;
        cmp = loft_cmp_text_rec;
        size = 4;
    } else if (size == 1) cmp = loft_cmp_u8;
    else if (size == 2) cmp = loft_cmp_u16;
    else if (size == 4 && is_float) { loft_sort_f32((uint8_t *) base, (size_t) n); return; }
    else if (size == 8 && is_float) { loft_sort_f64((uint8_t *) base, (size_t) n); return; }
    else if (size == 4) cmp = loft_cmp_i32;
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
static void loft_append_text_bytes(loft_ref v, loft_str t, int64_t lo, int64_t hi) {
    if (hi > (int64_t) t.len) hi = (int64_t) t.len;
    if (lo >= 0 && hi > lo && v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0) {
        uint32_t n = (uint32_t) (hi - lo);
        loft_store_t *s = loft_store_of(v);
        const uint8_t *src = (const uint8_t *) t.p + lo;
        int inside = src >= s->buf && src < s->buf + (size_t) s->size * 8;
        size_t off = inside ? (size_t) (src - s->buf) : 0;
        loft_vec_reserve_more(v, (int64_t) n, 1);
        s = loft_store_of(v);
        uint32_t r = loft_collection_rec(s, v.rec, v.pos);
        if (r != 0 && !s->read_only) {
            int32_t claimed;
            memcpy(&claimed, loft_at(s, r, 0), 4);
            uint32_t len = loft_u32(s, r, 4);
            if (claimed > 0 && (uint64_t) len + n + 8 <= (uint64_t) claimed * 8) {
                if (inside) src = s->buf + off;
                memmove(loft_at(s, r, 8 + len), src, n);
                loft_set_u32(s, r, 4, len + n);
                return;
            }
        }
    }
    for (int64_t i = lo; i < hi && i < (int64_t) t.len; i++) {
        loft_ref e = loft_vec_append(v, 1);
        if (e.rec == 0) return;
        *loft_at(loft_store_of(e), e.rec, e.pos) = (uint8_t) t.p[i];
        loft_vec_finish(v);
    }
}

/* OpKeepVectorRange (Stores::keep_vector_range): only the elements [lo, hi) stay, moved to the
 * front; a range outside the vector leaves it as it was.  What each dropped element owns is
 * released first, front and back (a linked element's record removed with it), before anything
 * moves.  `size` 0 is a linked element: its slot is a record number. */
static void loft_keep_vector_range(const loft_tdesc *T, uint16_t tp, loft_ref v, uint32_t size, int64_t lo, int64_t hi) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || lo < 0 || hi < lo || lo == LOFT_NULL || hi == LOFT_NULL) return;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0 || hi > (int64_t) loft_u32(s, r, 4)) return;
    uint32_t len = loft_u32(s, r, 4);
    if (size == 0) {
        for (uint32_t i = 0; i < len; i++) {
            if ((int64_t) i >= lo && (int64_t) i < hi) continue;
            uint32_t rec = loft_u32(s, r, 8 + i * 4);
            if (rec == 0) continue;
            loft_ref elem = { v.store_nr, rec, 8 };
            loft_remove_claims(T, tp, elem);
            loft_remove_record(loft_store_of(v), rec);
        }
        size = 4;
        s = loft_store_of(v);
        r = loft_collection_rec(s, v.rec, v.pos);
    } else if (T[tp].owns_heap) {
        for (uint32_t i = 0; i < len; i++) {
            if ((int64_t) i >= lo && (int64_t) i < hi) continue;
            loft_ref elem = { v.store_nr, r, 8 + i * size };
            loft_remove_claims(T, tp, elem);
        }
        s = loft_store_of(v);
        r = loft_collection_rec(s, v.rec, v.pos);
    }
    uint32_t kept = (uint32_t) (hi - lo);
    if (lo > 0 && kept > 0) memmove(loft_at(s, r, 8), loft_at(s, r, 8 + (uint32_t) lo * size), (size_t) kept * size);
    loft_set_u32(s, r, 4, kept);
}

/* A whole-value bind `dst = src` (@C86): an independent copy of a present source, the null of
 * an absent one, releasing what `dst` held.  The copy takes a fresh store when the source
 * lives in `dst`'s: refilling that store in place would destroy what the source owns. */
static void loft_bind_copy(const loft_tdesc *T, uint16_t tp, loft_ref *dst, loft_ref src) {
    if (src.store_nr == 0xFFFF || src.rec == 0) {
        loft_free_ref(*dst);
        *dst = loft_ref_null();
        return;
    }
    if (src.store_nr == dst->store_nr && src.rec == dst->rec && src.pos == dst->pos) return;
    if (src.store_nr == dst->store_nr) *dst = loft_ref_null();
    loft_database_typed(dst, T[tp].size, tp); /* typed, as `OpDatabase(tp)`: the next rebind may exchange */
#ifdef LOFT_PARSE
    loft_perrs_deliver(src, *dst, tp, 1);
#endif
    loft_copy_bytes(*dst, 0, src, 0, T[tp].size);
    loft_copy_claims(T, tp, src, *dst);
#ifdef LOFT_COPY_CHECK
    loft_report_copy_mismatches(T, tp, src, *dst, "bind_copy");
#endif
}

/* `@FR-R-DeferredCopy`'s remap: `y` — a view into the graph rooted at `from` — moved to the same
 * position in `to`, the copy `loft_bind_copy` made of it, walking both in step as
 * `loft_copy_claims` built `to`: the root's inline bytes, each struct field, each vector's record
 * and its elements, a struct-enum's variant.  1 with `*out` set when `y` lies in the graph. */
static int loft_dc_find(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to, loft_ref y, loft_ref *out) {
    const loft_tdesc *d = &T[tp];
    if (y.rec == from.rec && y.pos >= from.pos && y.pos < from.pos + d->size) {
        out->store_nr = to.store_nr;
        out->rec = to.rec;
        out->pos = to.pos + (y.pos - from.pos);
        return 1;
    }
    loft_store_t *fs = loft_store_of(from);
    switch (d->kind) {
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) {
            const loft_fld *f = &d->fields[i];
            if (f->view) continue;
            if (loft_dc_find(T, f->type, loft_field(from, f->pos), loft_field(to, f->pos), y, out)) return 1;
        }
        return 0;
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(fs, from.rec, from.pos) - 1;
        return e >= 0 && e < d->n && d->variants[e] != 0xFFFF
            && loft_dc_find(T, d->variants[e], from, to, y, out);
    }
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0 || cur == 0xFFFFFFFFu) return 0;
        uint32_t into = loft_u32(loft_store_of(to), to.rec, to.pos);
        if (y.rec == cur) {
            out->store_nr = to.store_nr;
            out->rec = into;
            out->pos = y.pos;
            return 1;
        }
        uint16_t e = d->elem;
        if (T[e].kind == LOFT_T_SCALAR || T[e].kind == LOFT_T_DBREF || T[e].kind == LOFT_T_TEXT) return 0;
        uint32_t len = loft_u32(fs, cur, 4), size = T[e].size;
        for (uint32_t i = 0; i < len; i++) {
            loft_ref a = { from.store_nr, cur, 8 + i * size };
            loft_ref b = { to.store_nr, into, 8 + i * size };
            if (loft_dc_find(T, e, a, b, y, out)) return 1;
        }
        return 0;
    }
    default:
        return 0;
    }
}

static loft_ref loft_dc_remap(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to, loft_ref y) {
    loft_ref out;
    if (y.store_nr != from.store_nr || y.rec == 0 || from.rec == 0) return y;
    return loft_dc_find(T, tp, from, to, y, &out) ? out : y;
}

/* `@FR-R-DeferredCopy`'s index guard: is `x` the element `k` of the vector `v` names?  `k` is
 * resolved exactly as `loft_vec_get` resolves it (a negative index counts from the end), so the
 * test compares positions, never the raw integers; an index outside the vector names nothing. */
LOFT_HOT_INLINE int loft_dc_is_elem(loft_ref x, loft_ref v, uint32_t size, int64_t k) {
    loft_ref e = loft_vec_get(v, size, k);
    return e.rec != 0 && x.store_nr == e.store_nr && x.rec == e.rec && x.pos == e.pos;
}

/* ── Floats (src/ops.rs, the stdlib's #rust templates) ─────────────────────────────────────
 * float is a double, single a float; NaN is null.  A literal is written from its bits, so no
 * decimal round trip can change it. */
static double loft_f64(uint64_t bits) { double d; memcpy(&d, &bits, 8); return d; }
static float loft_f32(uint32_t bits) { float f; memcpy(&f, &bits, 4); return f; }

static int loft_feq(double a, double b) { return (isnan(a) && isnan(b)) || (a <= b && b <= a); }
static int loft_flt(double a, double b) { return !isnan(b) && !(a >= b); }
static int loft_fle(double a, double b) { return isnan(a) || (!isnan(b) && a <= b); }

/* op_conv_float_from_long: null becomes NaN. */
static double loft_i2f(int64_t v) { return v == LOFT_NULL ? (double) NAN : (double) v; }
static float loft_i2s(int64_t v) { return v == LOFT_NULL ? (float) NAN : (float) v; }

/* OpCastIntFromFloat: NaN and a value outside i64 are null; else truncated toward zero. */
static int64_t loft_f2i(double v) {
    if (isnan(v) || !(v >= -9223372036854775808.0 && v < 9223372036854775808.0)) return LOFT_NULL;
    return (int64_t) v;
}

static double loft_math1(int id, double x) {
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
    default: return (double) NAN;
    }
}

static float loft_math1s(int id, float x) {
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
    default: return (float) NAN;
    }
}

/* id 0 is atan2; id 1 is the logarithm in base `b` (log10 and log2 exactly, else ln a / ln b). */
static double loft_math2(int id, double a, double b) {
    if (id == 0) return atan2(a, b);
    if (id == 1) return b == 10.0 ? log10(a) : b == 2.0 ? log2(a) : log(a) / log(b);
    return (double) NAN;
}

static float loft_math2s(int id, float a, float b) {
    if (id == 0) return atan2f(a, b);
    if (id == 1) return b == 10.0f ? log10f(a) : b == 2.0f ? log2f(a) : logf(a) / logf(b);
    return (float) NAN;
}

LOFT_HOT_INLINE double loft_get_float(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return (double) NAN;
    double v;
    memcpy(&v, loft_at(loft_store_of(r), r.rec, r.pos + fld), 8);
    return v;
}

LOFT_HOT_INLINE void loft_set_float(loft_ref r, uint32_t fld, double v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &v, 8);
}

static float loft_get_single(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return (float) NAN;
    float v;
    memcpy(&v, loft_at(loft_store_of(r), r.rec, r.pos + fld), 4);
    return v;
}

static void loft_set_single(loft_ref r, uint32_t fld, float v) {
    if (r.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(r))) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &v, 4);
}

static void loft_vec_push_float(loft_ref v, double x) {
    uint8_t *slot = loft_vec_push_slot(v, 8);
    if (slot) {
        memcpy(slot, &x, 8);
        return;
    }
    loft_ref e = loft_vec_append(v, 8);
    if (e.rec == 0) return;
    loft_set_float(e, 0, x);
    loft_vec_finish(v);
}

static void loft_vec_push_char(loft_ref v, uint32_t c) {
    uint8_t *slot = loft_vec_push_slot(v, 4);
    if (slot) {
        memcpy(slot, &c, 4);
        return;
    }
    loft_ref e = loft_vec_append(v, 4);
    if (e.rec == 0) return;
    memcpy(loft_at(loft_store_of(e), e.rec, e.pos), &c, 4);
    loft_vec_finish(v);
}

static void loft_vec_push_single(loft_ref v, float x) {
    uint8_t *slot = loft_vec_push_slot(v, 4);
    if (slot) {
        memcpy(slot, &x, 4);
        return;
    }
    loft_ref e = loft_vec_append(v, 4);
    if (e.rec == 0) return;
    loft_set_single(e, 0, x);
    loft_vec_finish(v);
}

/* Rust's `{}` for a float: the shortest digits that read back as `v` (as a float when `single`),
 * written out in full with no exponent — 1e21 is "1000000000000000000000", 1.0 is "1", -0.0 is
 * "-0".  The digits are the first `%.*e` precision that round-trips, correctly rounded. */
/* `@FR-F-Render`'s shortest form (@C156): of the strings of the shortest length that read back as
 * `v`, the nearest, an exact tie to the even digit — `printf`'s own rounding, so its p-digit
 * candidate is the answer whenever it reads back.  When it does not, its neighbour on `v`'s
 * other side may (just below a power of two the half-gap towards zero is the narrower): `buf`
 * becomes that neighbour and 1 is answered when it reads back. */
static int loft_shortest_other(char *buf, size_t cap, int p, double v, double near, int single) {
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
static int loft_push_shortest_fast(loft_text *s, double v, int single) {
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
            int nd = loft_u64_dec(dig, m);
            if (v < 0) loft_push_char(s, '-');
            if (nd <= k) {
                loft_push(s, "0.", 2);
                for (int z = 0; z < k - nd; z++) loft_push_char(s, '0');
                loft_push(s, dig, (size_t) nd);
            } else {
                loft_push(s, dig, (size_t) (nd - k));
                if (k > 0) {
                    loft_push_char(s, '.');
                    loft_push(s, dig + nd - k, (size_t) k);
                }
            }
            return 1;
        }
    }
    return 0;
}

static void loft_push_shortest(loft_text *s, double v, int single) {
    if (loft_push_shortest_fast(s, v, single)) return;
    if (isinf(v)) {
        if (v < 0) loft_push(s, "-inf", 4); else loft_push(s, "inf", 3);
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
        if (loft_shortest_other(buf, sizeof buf, p, v, back, single)) break;
    }
    const char *q = buf;
    if (*q == '-') { loft_push_char(s, '-'); q++; }
    char digits[48];
    int nd = 0;
    for (; *q && *q != 'e'; q++) if (*q >= '0' && *q <= '9') digits[nd++] = *q;
    int exp = atoi(q + 1);
    while (nd > 1 && digits[nd - 1] == '0') nd--;
    if (nd == 1 && digits[0] == '0') { loft_push_char(s, '0'); return; }
    if (exp >= 0) {
        int int_len = exp + 1;
        for (int i = 0; i < int_len; i++) loft_push_char(s, i < nd ? digits[i] : '0');
        if (nd > int_len) { loft_push_char(s, '.'); loft_push(s, digits + int_len, (size_t) (nd - int_len)); }
    } else {
        loft_push(s, "0.", 2);
        for (int i = 0; i < -exp - 1; i++) loft_push_char(s, '0');
        loft_push(s, digits, (size_t) nd);
    }
}

/* ops::format_signed */
static void loft_format_signed(loft_text *s, loft_str res, int64_t width, int dir, uint8_t token) {
    int is_null = res.len == 4 && memcmp(res.p, "null", 4) == 0;
    if (token == '0' && !is_null && res.len > 0 && (res.p[0] == '-' || res.p[0] == '+')) {
        loft_push_char(s, res.p[0]);
        loft_format_text(s, loft_lit(res.p + 1, res.len - 1), width - 1, dir, token);
        return;
    }
    loft_format_text(s, res, width, dir, is_null ? loft_null_pad(token) : token);
}

/* ops::format_float and ops::format_single */
/* `{v:.p}` for p <= 9 in exact integer arithmetic: v = m * 2^e with m < 2^53, so m * 10^p fits
 * 128 bits, and shifting it right by -e with round-half-even on the remainder gives the correctly
 * rounded p-digit value both Rust and printf print — without printf's arbitrary-precision path.
 * Answers 0 for what it does not cover (p > 9, an integer part past 2^63), which printf takes. */
typedef struct { uint64_t hi, lo; } loft_u128;

static int loft_push_fixed(loft_text *s, double val, int prec) {
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
    loft_push(s, buf, (size_t) n);
    return 1;
}

static void loft_format_float(loft_text *s, double val, int64_t width, int64_t prec, uint8_t token, int plus,
                              int dir, int single) {
    if (dir == 2) dir = 1;
    size_t start = s->len;
    if (isnan(val)) {
        loft_push(s, "null", 4);
    } else if (prec >= 0 && loft_push_fixed(s, val, (int) prec)) {
    } else if (prec >= 0) {
        int n = snprintf(0, 0, "%.*f", (int) prec, val);
        char *buf = (char *) malloc((size_t) n + 1);
        if (!buf) { fputs("loft: out of memory\n", stderr); abort(); }
        snprintf(buf, (size_t) n + 1, "%.*f", (int) prec, val);
        loft_push(s, buf, (size_t) n);
        free(buf);
    } else {
        loft_push_shortest(s, val, single);
    }
    /* sign_a_number */
    if (plus && !isnan(val) && !(s->len > start && s->p[start] == '-')) {
        loft_push_char(s, ' ');
        memmove(s->p + start + 1, s->p + start, s->len - start - 1);
        s->p[start] = '+';
    }
    if (width > (int64_t) (s->len - start)) {
        loft_text res = loft_text_own(loft_lit(s->p + start, s->len - start));
        s->len = start;
        loft_format_signed(s, loft_view(&res), width, dir, token);
        loft_text_free(&res);
    }
}

/* ── Rendering a value of a store (database/format.rs ShowDb, the `{v}` of a record or a
 * vector) ─────────────────────────────────────────────────────────────────────────────────
 * `fmt` is the format operand: bit 1 pretty, bit 2 JSON.  The emitter writes one
 * `loft_show{tp}` per type a program renders; these are the parts every type shares. */
typedef void (*loft_show_fn)(loft_text *s, loft_ref r, int fmt, int ind);

static void loft_push_i64(loft_text *s, int64_t v) {
    char buf[24];
    int n = 0;
    if (v < 0) buf[n++] = '-';
    n += loft_u64_dec(buf + n, v < 0 ? (uint64_t) 0 - (uint64_t) v : (uint64_t) v);
    loft_push(s, buf, (size_t) n);
}

static void loft_nl(loft_text *s, int ind) {
    loft_push_char(s, '\n');
    for (int i = 0; i < ind; i++) loft_push(s, "  ", 2);
}

/* write_json_escaped */
static void loft_push_json(loft_text *s, loft_str t) {
    loft_push_char(s, '"');
    for (size_t i = 0; i < t.len; i++) {
        unsigned char c = (unsigned char) t.p[i];
        switch (c) {
        case '"': loft_push(s, "\\\"", 2); break;
        case '\\': loft_push(s, "\\\\", 2); break;
        case '\n': loft_push(s, "\\n", 2); break;
        case '\r': loft_push(s, "\\r", 2); break;
        case '\t': loft_push(s, "\\t", 2); break;
        case 0x08: loft_push(s, "\\b", 2); break;
        case 0x0c: loft_push(s, "\\f", 2); break;
        default:
            if (c < 0x20) {
                char buf[8];
                snprintf(buf, sizeof buf, "\\u%04x", c);
                loft_push(s, buf, 6);
            } else {
                loft_push_char(s, (char) c);
            }
        }
    }
    loft_push_char(s, '"');
}

/* A code point as UTF-8; 0 bytes for one that is not a character. */
static size_t loft_utf8(uint32_t c, char *out) {
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

static void loft_show_long(loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    int64_t v = loft_get_int(r, 0);
    if (v == LOFT_NULL) loft_push(s, "null", 4); else loft_push_i64(s, v);
}

static void loft_show_single(loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    float v = loft_get_single(r, 0);
    if (isnan(v)) loft_push(s, "null", 4); else loft_push_shortest(s, v, 1);
}

static void loft_show_float(loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    double v = loft_get_float(r, 0);
    if (isnan(v)) loft_push(s, "null", 4); else loft_push_shortest(s, v, 0);
}

static void loft_show_bool(loft_text *s, loft_ref r, int fmt, int ind) {
    (void) fmt; (void) ind;
    uint8_t b = *loft_at(loft_store_of(r), r.rec, r.pos);
    loft_push_cstr(s, b == 0 ? "false" : b == 255 ? "null" : "true");
}

static void loft_show_text(loft_text *s, loft_ref r, int fmt, int ind) {
    (void) ind;
    loft_store_t *st = loft_store_of(r);
    uint32_t h = loft_u32(st, r.rec, r.pos);
    if (h != 0 && h >= st->size) {
        /* A handle outside the store is corruption, not absence. */
        if (fmt & 2) { loft_push(s, "null", 4); return; }
        loft_push_cstr(s, "<bad-text:");
        loft_push_i64(s, h);
        loft_push_char(s, '>');
        return;
    }
    loft_str t = loft_get_text(r, 0);
    if (loft_str_is_null(t)) { loft_push(s, "null", 4); return; }
    if (fmt & 2) { loft_push_json(s, t); return; }
    loft_push_char(s, '"');
    loft_text_append(s, t);
    loft_push_char(s, '"');
}

static void loft_show_char(loft_text *s, loft_ref r, int fmt, int ind) {
    (void) ind;
    uint32_t c = loft_u32(loft_store_of(r), r.rec, r.pos);
    char buf[4];
    size_t n = c == 0 ? 0 : loft_utf8(c, buf);
    if (n == 0) { loft_push(s, "null", 4); return; }
    if (fmt & 2) { loft_push_json(s, loft_lit(buf, n)); return; }
    loft_push_char(s, '\'');
    loft_push(s, buf, n);
    loft_push_char(s, '\'');
}

/* The narrow integer widths, each with its own null code (Stores::narrow_is_null). */
static void loft_show_byte(loft_text *s, loft_ref r, int64_t min, int nullable) {
    uint8_t raw = *loft_at(loft_store_of(r), r.rec, r.pos);
    if (nullable && raw == 255) loft_push(s, "null", 4); else loft_push_i64(s, raw + min);
}

/* ── deliver (`Stores::deliver_loopback`): a value's bytes through its layout, printed as hex ── */
static void loft_dlv_hex(loft_text *o, const uint8_t *p, size_t n) {
    static const char digits[] = "0123456789abcdef";
    for (size_t i = 0; i < n; i++) {
        char h[2] = { digits[p[i] >> 4], digits[p[i] & 15] };
        loft_push(o, h, 2);
    }
}

/* A scalar's bytes as the store holds them, little-endian as `to_le_bytes` writes them. */
static void loft_dlv_raw(loft_text *o, loft_ref r, uint32_t n) {
    loft_dlv_hex(o, loft_at(loft_store_of(r), r.rec, r.pos), n);
}

/* A two-byte narrow integer: `get_short` (`null` reads 0, else the code less one) or, raw, the
 * code biased by `start` (`get_i16_raw`). */
static void loft_dlv_short(loft_text *o, loft_ref r, int32_t start, int biased) {
    uint16_t raw;
    memcpy(&raw, loft_at(loft_store_of(r), r.rec, r.pos), 2);
    int16_t v = biased ? (raw ? (int16_t) (raw - 1) : 0) : (int16_t) (raw + start);
    uint8_t b[2] = { (uint8_t) v, (uint8_t) ((uint16_t) v >> 8) };
    loft_dlv_hex(o, b, 2);
}

/* A text's bytes, its length nowhere. */
static void loft_dlv_text(loft_text *o, loft_ref r) {
    loft_str s = loft_get_text(r, 0);
    loft_dlv_hex(o, (const uint8_t *) s.p, s.len);
}

typedef const char *(*loft_dlv_fn)(loft_text *o, loft_ref r);

/* `deliver(tag, value)` on the loopback host: the bytes, or why the layout cannot carry them. */
static void loft_deliver(int64_t tag, const char *name, loft_dlv_fn f, loft_ref r) {
    loft_text o = { 0, 0, 0 };
    const char *e = f(&o, r);
    if (e) printf("deliver tag=%lld type=%s error=%s\n", (long long) tag, name, e);
    else printf("deliver tag=%lld type=%s bytes=%.*s\n", (long long) tag, name, (int) o.len, o.p ? o.p : "");
    loft_text_free(&o);
}

/* The bytes of place `r` for a link that addresses them directly, or `absent` when `r` names
 * no record (`--native`'s `addr_mut` on a place, null for an absent one). */
static void *loft_place(loft_ref r, void *absent) {
    return r.rec == 0 ? absent : (void *) loft_at(loft_store_of(r), r.rec, r.pos);
}

/* A stored reference (`Parts::DbRef`, ShowDb): `null` when it names no record or a removed
 * one (`@FR-H-Removed`), `iterator` for a generator handle, else its three parts — the debug
 * shape, which no format reads back. */
static void loft_show_dbref(loft_text *s, loft_ref r) {
    uint32_t w[3];
    memcpy(w, loft_at(loft_store_of(r), r.rec, r.pos), 12);
    uint16_t nr = (uint16_t) w[0];
    if (w[1] == 0 || (nr < loft_nstores && loft_st_is_removed(&loft_stores[nr], w[1]))) {
        loft_push(s, "null", 4);
    } else if (nr == 0xFFFE || nr == 0xFFFF) {
        loft_push(s, "iterator", 8);
    } else {
        char b[48];
        int n = snprintf(b, sizeof b, "DbRef(%u,%u,%u)", (unsigned) nr, (unsigned) w[1], (unsigned) w[2]);
        loft_push(s, b, (size_t) n);
    }
}

static void loft_show_short(loft_text *s, loft_ref r, int64_t min, int nullable) {
    uint16_t raw;
    memcpy(&raw, loft_at(loft_store_of(r), r.rec, r.pos), 2);
    if (nullable && raw == 0) { loft_push(s, "null", 4); return; }
    loft_push_i64(s, raw != 0 ? raw + min - 1 : (int64_t) INT32_MIN);
}

static void loft_show_short_raw(loft_text *s, loft_ref r, int64_t min, int nullable) {
    uint16_t raw;
    memcpy(&raw, loft_at(loft_store_of(r), r.rec, r.pos), 2);
    if (nullable && raw == 0xFFFF) loft_push(s, "null", 4); else loft_push_i64(s, raw + min);
}

static void loft_show_int4(loft_text *s, loft_ref r, int nullable) {
    int32_t v;
    memcpy(&v, loft_at(loft_store_of(r), r.rec, r.pos), 4);
    if (nullable && v == INT32_MIN) loft_push(s, "null", 4); else loft_push_i64(s, v);
}

static void loft_show_int_raw(loft_text *s, loft_ref r, int nullable) {
    uint32_t v = loft_u32(loft_store_of(r), r.rec, r.pos);
    if (nullable && v == 0xFFFFFFFFu) loft_push(s, "null", 4); else loft_push_i64(s, v);
}

static void loft_show_child(loft_text *s, loft_ref r) {
    uint32_t v = loft_u32(loft_store_of(r), r.rec, r.pos);
    if (v == 0) { loft_push(s, "null", 4); return; }
    loft_push_cstr(s, "ChildRec(");
    loft_push_i64(s, v);
    loft_push_char(s, ')');
}

/* write_struct's frame: the opening brace, and whether the body goes on lines of its own. */
static int loft_show_open(loft_text *s, int fmt, int ind, int complex_type, int any) {
    int pretty = fmt & 1, json = (fmt & 2) != 0;
    int c = pretty && (json || complex_type);
    loft_push_char(s, '{');
    if (c && json && any) loft_nl(s, ind + 1);
    else if (pretty) loft_push_char(s, ' ');
    return c;
}

static void loft_show_close(loft_text *s, int fmt, int ind, int c, int any) {
    if (c && any) loft_nl(s, ind);
    else if ((fmt & 1) && !c) loft_push_char(s, ' ');
    loft_push_char(s, '}');
}

/* Between two fields (write_indent with its zero test). */
static void loft_show_sep(loft_text *s, int fmt, int ind, int c) {
    loft_push_char(s, ',');
    if (c) loft_nl(s, ind + 1);
    else if (fmt & 1) loft_push_char(s, ' ');
}

static void loft_show_name(loft_text *s, int fmt, const char *name) {
    if (fmt & 2) loft_push_char(s, '"');
    loft_push_cstr(s, name);
    if (fmt & 2) loft_push_char(s, '"');
    loft_push_char(s, ':');
    if (fmt & 1) loft_push_char(s, ' ');
}

/* write_list for a vector: its elements of `size` bytes, each rendered by `f`. */
static void loft_show_list(loft_text *s, loft_ref r, int fmt, int ind, uint32_t size,
                           int complex_elem, int struct_elem, loft_show_fn f) {
    int pretty = fmt & 1;
    int c = pretty && complex_elem;
    loft_push_char(s, '[');
    uint32_t vr = loft_collection_rec(loft_store_of(r), r.rec, r.pos);
    uint32_t n = vr == 0 ? 0 : loft_u32(loft_store_of(r), vr, 4);
    for (uint32_t i = 0; i < n; i++) {
        if (i == 0) {
            if (pretty) { if (c) loft_nl(s, ind + 1); else loft_push_char(s, ' '); }
        } else {
            loft_push_char(s, ',');
            if (pretty) { if (struct_elem) loft_nl(s, ind + 1); else loft_push_char(s, ' '); }
        }
        loft_ref e = { r.store_nr, vr, 8 + i * size };
        f(s, e, fmt, ind + 1);
    }
    if (pretty) loft_push_char(s, ' ');
    loft_push_char(s, ']');
}

/* ── The host: arguments and the clock ─────────────────────────────────────────────────── */

/* The program's own arguments, after its name (Stores::os_arguments). */
static int loft_argc = 0;
static char **loft_argv = 0;

/* arguments(): a fresh `vector<text>` of them. */
static loft_ref loft_arguments(void) {
    loft_ref r = loft_ref_null();
    loft_database(&r, 8);
    for (int i = 1; i < loft_argc; i++) {
        loft_ref e = loft_vec_append(r, 4);
        loft_set_text(e, 0, loft_lit(loft_argv[i], strlen(loft_argv[i])));
        loft_vec_finish(r);
    }
    return r;
}

/* ticks(): microseconds since the program started, monotonic; LOFT_FAKE_TICKS_US pins it. */
static int64_t loft_start_us = -1;

static int64_t loft_ticks(void) {
    static int fake_read = 0;
    static int64_t fake = LOFT_NULL;
    if (!fake_read) {
        fake_read = 1;
        const char *f = getenv("LOFT_FAKE_TICKS_US");
        if (f && *f) {
            char *end;
            long long v = strtoll(f, &end, 10);
            if (*end == 0) fake = (int64_t) v;
        }
    }
    if (fake != LOFT_NULL) return fake;
    return loft_now_us() - loft_start_us;
}

/* now(): milliseconds since the Unix epoch; LOFT_FAKE_NOW_MS pins it. */
static int64_t loft_now(void) {
    static int fake_read = 0;
    static int64_t fake = LOFT_NULL;
    if (!fake_read) {
        fake_read = 1;
        const char *f = getenv("LOFT_FAKE_NOW_MS");
        if (f && *f) {
            char *end;
            long long v = strtoll(f, &end, 10);
            if (*end == 0) fake = (int64_t) v;
        }
    }
    if (fake != LOFT_NULL) return fake;
#if defined(LOFT_POSIX)
    struct timespec t;
    clock_gettime(CLOCK_REALTIME, &t);
    return (int64_t) t.tv_sec * 1000 + t.tv_nsec / 1000000;
#elif defined(_WIN32)
    return loft_win_wall_ms();
#else
    return (int64_t) time(0) * 1000;
#endif
}

/* OpAbsInt (ops::op_abs_long): null stays null. */
static int64_t loft_abs(int64_t v) {
    return v == LOFT_NULL ? v : v < 0 ? -v : v;
}

/* OpPowInt (ops::op_pow_int): `checked_pow` — null for a null or negative operand, an exponent
 * past 32 bits, or an overflow on the way. */
static int64_t loft_pow(int64_t base, int64_t exp) {
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
static int64_t loft_int_of_text(loft_str t) {
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
static int loft_float_text_ok(loft_str t) {
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
static int loft_float_fast(loft_str t, double *out) {
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

static loft_ls loft_ls_of(loft_str t, uint32_t sep) {
    loft_ls ls;
    ls.rest = t;
    ls.live = !(t.len == 0 || loft_str_is_null(t));
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
LOFT_HOT_INLINE loft_str loft_ls_next(loft_ls *ls, uint8_t *done) {
    if (!ls->live) {
        *done = 1;
        return loft_null_text();
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

static void loft_stab_fill(loft_stab *t, loft_str s, uint32_t sep) {
    t->n = 0;
    if (s.len == 0 || loft_str_is_null(s)) return;
    t->src.len = 0;
    loft_push(&t->src, s.p, s.len);
    loft_ls ls = loft_ls_of(loft_view(&t->src), sep);
    uint8_t done = 0;
    for (;;) {
        loft_str piece = loft_ls_next(&ls, &done);
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
LOFT_HOT_INLINE loft_str loft_stab_get(const loft_stab *t, int64_t from) {
    int64_t n = (int64_t) t->n;
    int64_t f = from < 0 ? from + n : from;
    return f < 0 || f >= n ? loft_null_text() : t->at[f];
}

static void loft_stab_free(loft_stab *t) {
    loft_text_free(&t->src);
    free(t->at);
    t->at = 0;
    t->n = 0;
    t->cap = 0;
}

/* OpCastFloatFromText: `parse::<f64>().unwrap_or(NAN)`. */
static double loft_float_of_text(loft_str t) {
    if (!loft_float_text_ok(t)) return (double) NAN;
    double fast;
    if (loft_float_fast(t, &fast)) return fast;
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
static float loft_single_of_text(loft_str t) {
    if (!loft_float_text_ok(t)) return (float) NAN;
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
static uint8_t loft_in_ranges(uint32_t c, const uint32_t (*r)[2], size_t n) {
    size_t lo = 0, hi = n;
    while (lo < hi) {
        size_t mid = lo + (hi - lo) / 2;
        if (c < r[mid][0]) hi = mid;
        else if (c > r[mid][1]) lo = mid + 1;
        else return 1;
    }
    return 0;
}

static const char *loft_umap_find(const loft_umap *m, size_t n, uint32_t c) {
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
static int loft_cased(uint32_t c) {
    return (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || (c >= 0xC0 && c != 0xD7 && c != 0xF7);
}

/* str::to_lowercase / to_uppercase: each character's mapping from `m`, or itself; lowering a
 * capital sigma at the end of a word gives the final sigma. */
static loft_text loft_text_case(loft_str s, const loft_umap *m, size_t n, int lower) {
    loft_text out = { 0, 0, 0 };
    if (loft_str_is_null(s)) return loft_text_own(s);
    size_t i = 0;
    uint32_t prev = 0;
    /* The result is about as long as the input: one reservation, not one per character. */
    loft_reserve(&out, s.len);
    while (i < s.len) {
        /* An ASCII run maps by its own rule, A-Z and a-z, with no table search: every ASCII
         * letter's mapping is its other case and no other ASCII character has one. */
        if ((uint8_t) s.p[i] < 0x80) {
            size_t j = i;
            while (j < s.len && (uint8_t) s.p[j] < 0x80) j++;
            loft_reserve(&out, j - i);
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
        uint32_t c = loft_decode(s, i, &w);
        const char *to = loft_umap_find(m, n, c);
        if (lower && c == 0x3A3) {
            uint32_t next = 0;
            if (i + w < s.len) {
                size_t w2;
                next = loft_decode(s, i + w, &w2);
            }
            to = loft_cased(prev) && !loft_cased(next) ? "\xCF\x82" : "\xCF\x83";
        }
        if (to) loft_push(&out, to, strlen(to));
        else loft_push(&out, s.p + i, w);
        prev = c;
        i += w;
    }
    if (!out.p) loft_push(&out, "", 0);
    return out;
}

/* Called first by `main`: the arguments, the clock's start, the constants. */
static void loft_init(void);
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
static int loft_rename(const char *from, const char *to) {
    if (MoveFileExA(from, to, 1ul /* MOVEFILE_REPLACE_EXISTING */)) return 0;
    switch (GetLastError()) {
    case 2: case 3: errno = ENOENT; break; /* ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND */
    case 5: case 32: errno = EACCES; break; /* ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION */
    default: errno = EIO; break;
    }
    return -1;
}
#else
#define loft_rename(from, to) rename(from, to)
#endif

static void loft_arm_timeout(void) {
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

static int loft_c_lib_exists(const char *p) {
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
static uint32_t loft_c_lib_dirs(loft_c_libdir *d, uint32_t max) {
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
static void loft_c_missing_library(const loft_c_boundlib *l, const loft_c_libdir *d, uint32_t n,
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
    exit(1);
}

/* `native_bind::bind`: open every library the program was shipped with, first found in search
 * order; a library found nowhere ends the program naming it. */
static void loft_c_bind_libraries(void) {
    static loft_c_libdir dirs[32];
    uint32_t nd = loft_c_lib_dirs(dirs, 32);
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
                if (!loft_c_lib_exists(p)) continue;
                h = loft_dl_open(p, LOFT_DL_GLOBAL);
                if (!h) {
                    const char *why = loft_dl_error();
                    size_t used = strlen(refused);
                    snprintf(refused + used, sizeof refused - used, "%s`%s` would not load: %s", used ? "\n  " : "", p,
                             why ? why : "unknown error");
                }
            }
        }
        if (!h) loft_c_missing_library(l, dirs, nd, refused);
        if (loft_ncbl_handles < 64) loft_cbl_handles[loft_ncbl_handles++] = h;
        if (i < 64) loft_cbl_row_handle[i] = h;
    }
}
#endif

static void loft_start(int argc, char **argv) {
#if defined(_WIN32)
    /* The bytes a program writes are the bytes the other backends write: no `\r\n` for `\n`. */
    _setmode(_fileno(stdout), _O_BINARY);
    _setmode(_fileno(stderr), _O_BINARY);
#endif
    loft_argc = argc;
    loft_argv = argv;
    loft_start_us = loft_now_us();
    loft_heap_read_limit();
    loft_arm_timeout();
#if defined(LOFT_POSIX_HOST)
    signal(SIGPIPE, SIG_IGN);
#endif
#if defined(LOFT_BIND_LIBS) && defined(LOFT_DL)
    loft_c_bind_libraries();
#endif
    LOFT_TRACE("M\n");
    loft_init();
}

/* ── Content equality (Stores::eq_content, @FR-E-Eq) ──────────────────────────────────── */

typedef int (*loft_eq_fn)(loft_ref a, loft_ref b);

static int loft_same_place(loft_ref a, loft_ref b) {
    return a.store_nr == b.store_nr && a.rec == b.rec && a.pos == b.pos;
}

/* The start every comparison shares: the same place is equal, two absences are equal, an
 * absence differs from a present value.  -1 when both are present. */
static int loft_eq_start(loft_ref a, loft_ref b) {
    if (loft_same_place(a, b)) return 1;
    if (a.rec == 0 || b.rec == 0) return a.rec == 0 && b.rec == 0;
    return -1;
}

static int loft_eq_long(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    return s >= 0 ? s : loft_get_int(a, 0) == loft_get_int(b, 0);
}

static int loft_eq_float(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    if (s >= 0) return s;
    double x = loft_get_float(a, 0), y = loft_get_float(b, 0);
    if (isnan(x) || isnan(y)) return isnan(x) && isnan(y);
    return x == y;
}

static int loft_eq_single(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    if (s >= 0) return s;
    float x = loft_get_single(a, 0), y = loft_get_single(b, 0);
    if (isnan(x) || isnan(y)) return isnan(x) && isnan(y);
    return x == y;
}

static int loft_eq_byte(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    return s >= 0 ? s : *loft_at(loft_store_of(a), a.rec, a.pos) == *loft_at(loft_store_of(b), b.rec, b.pos);
}

static int loft_eq_u16(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    return s >= 0 ? s : memcmp(loft_at(loft_store_of(a), a.rec, a.pos), loft_at(loft_store_of(b), b.rec, b.pos), 2) == 0;
}

static int loft_eq_u32(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    return s >= 0 ? s : loft_u32(loft_store_of(a), a.rec, a.pos) == loft_u32(loft_store_of(b), b.rec, b.pos);
}

static int loft_eq_dbref(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    return s >= 0 ? s : memcmp(loft_at(loft_store_of(a), a.rec, a.pos), loft_at(loft_store_of(b), b.rec, b.pos), 12) == 0;
}

static int loft_eq_text(loft_ref a, loft_ref b) {
    int s = loft_eq_start(a, b);
    if (s >= 0) return s;
    loft_str x = loft_get_text(a, 0), y = loft_get_text(b, 0);
    if (loft_str_is_null(x) || loft_str_is_null(y)) return loft_str_is_null(x) && loft_str_is_null(y);
    return loft_str_eq(x, y);
}

/* A vector: two absent ones are equal, an absent one differs; then element by element. */
static int loft_eq_vector(loft_ref a, loft_ref b, uint32_t size, loft_eq_fn f) {
    int s = loft_eq_start(a, b);
    if (s >= 0) return s;
    int na = loft_vec_absent(a), nb = loft_vec_absent(b);
    if (na || nb) return na && nb;
    int64_t n = loft_vec_len(a);
    if (n != loft_vec_len(b)) return 0;
    for (int64_t i = 0; i < n; i++) {
        if (!f(loft_vec_get(a, size, i), loft_vec_get(b, size, i))) return 0;
    }
    return 1;
}

static int loft_eq_child(loft_ref a, loft_ref b, loft_eq_fn f) {
    int s = loft_eq_start(a, b);
    if (s >= 0) return s;
    loft_ref ca = { a.store_nr, loft_u32(loft_store_of(a), a.rec, a.pos), 8 };
    loft_ref cb = { b.store_nr, loft_u32(loft_store_of(b), b.rec, b.pos), 8 };
    return f(ca, cb);
}

/* The record pairs under comparison: a pair met again is equal, which is where a cycle ends. */
static LOFT_TLS loft_ref *loft_eq_seen = 0;
static LOFT_TLS size_t loft_eq_seen_n = 0, loft_eq_seen_cap = 0;

static int loft_eq_reference(loft_ref fa, loft_ref fb, uint16_t target, loft_eq_fn f) {
    loft_ref ra = loft_get_dbref(fa, 0), rb = loft_get_dbref(fb, 0);
    if (ra.rec == 0 || rb.rec == 0) return ra.rec == 0 && rb.rec == 0;
    for (size_t i = 0; i < loft_eq_seen_n; i += 3) {
        if (loft_same_place(loft_eq_seen[i], ra) && loft_same_place(loft_eq_seen[i + 1], rb)
            && loft_eq_seen[i + 2].rec == target) return 1;
    }
    if (loft_eq_seen_n + 3 > loft_eq_seen_cap) {
        loft_eq_seen_cap = loft_eq_seen_cap ? loft_eq_seen_cap * 2 : 48;
        loft_eq_seen = (loft_ref *) realloc(loft_eq_seen, loft_eq_seen_cap * sizeof *loft_eq_seen);
        if (!loft_eq_seen) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_ref t = { 0, target, 0 };
    loft_eq_seen[loft_eq_seen_n++] = ra;
    loft_eq_seen[loft_eq_seen_n++] = rb;
    loft_eq_seen[loft_eq_seen_n++] = t;
    return f(ra, rb);
}

/* OpEqContent: one comparison, with a list of pairs of its own. */
static int loft_eq_content(loft_eq_fn f, loft_ref a, loft_ref b) {
    loft_eq_seen_n = 0;
    return f(a, b);
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
static int64_t loft_vector_sum_int(loft_ref v, int64_t acc) {
    int64_t n = loft_vec_len(v);
    if (n <= 0) return acc;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    const uint8_t *base = loft_at(s, r, 8);
    int64_t at = 0;
    while (at < n) {
        int64_t stop = at + LOFT_SUM_BLOCK < n ? at + LOFT_SUM_BLOCK : n;
        uint64_t room = acc == LOFT_NULL ? 0 : (acc < 0 ? (uint64_t) 0 - (uint64_t) acc : (uint64_t) acc);
        if (acc == LOFT_NULL || room >= (uint64_t) INT64_MAX - (uint64_t) LOFT_SUM_BLOCK * (uint64_t) LOFT_SUM_BOUND) break;
        uint64_t high = 0, sum = 0;
        for (int64_t i = at; i < stop; i++) {
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
        acc = loft_add(acc, x);
    }
    return acc;
}

/* A `char *` a `#c` binding returns, as `c_call` reads it: NULL is the null text, the bytes end
 * at the first NUL, and an invalid UTF-8 byte is U+FFFD (`String::from_utf8_lossy`). */
static loft_text loft_text_from_cstr(const char *c) {
    loft_text t = { 0, 0, 0 };
    if (!c) {
        loft_push_char(&t, 0);
        return t;
    }
    size_t n = strlen(c);
    const unsigned char *p = (const unsigned char *) c;
    if (loft_utf8_valid(p, n)) {
        loft_push(&t, c, n);
        return t;
    }
    for (size_t i = 0; i < n;) {
        size_t len = p[i] < 0x80 ? 1 : (p[i] >= 0xC2 && p[i] <= 0xDF) ? 2
                   : (p[i] >= 0xE0 && p[i] <= 0xEF) ? 3 : (p[i] >= 0xF0 && p[i] <= 0xF4) ? 4 : 0;
        if (len && i + len <= n && loft_utf8_valid(p + i, len)) {
            loft_push(&t, c + i, len);
            i += len;
        } else {
            loft_push(&t, "\xEF\xBF\xBD", 3);
            i++;
        }
    }
    return t;
}

/* text_from_bytes (Stores::text_from_bytes_native): the bytes of a vector<u8> as a text; bytes
 * that are not UTF-8 give the empty text. */
static loft_text loft_text_from_bytes(loft_ref v) {
    loft_text out = { 0, 0, 0 };
    int64_t n = loft_vec_len(v);
    if (n == 0) return out;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return out;
    const unsigned char *p = loft_at(s, r, 8);
    if (loft_utf8_valid(p, (size_t) n)) loft_push(&out, (const char *) p, (size_t) n);
    return out;
}

/* text_from_byte_range (Stores::text_from_bytes_range): the bytes [lo, hi) of a vector<u8> as a
 * text, a negative bound counting from the end as `bytes[lo..hi]` does (@FR-Slice-Value); bytes
 * that are not UTF-8, or an empty range, give the empty text. */
static loft_text loft_text_from_byte_range(loft_ref v, int64_t lo, int64_t hi) {
    loft_text out = { 0, 0, 0 };
    int64_t n = loft_vec_len(v);
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0 || n == 0) return out;
    lo = lo < 0 ? (lo + n < 0 ? 0 : lo + n) : (lo > n ? n : lo);
    hi = hi < 0 ? (hi + n < 0 ? 0 : hi + n) : (hi > n ? n : hi);
    if (hi <= lo) return out;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r == 0) return out;
    const unsigned char *p = loft_elem_base(s, r) + lo;
    if (loft_utf8_valid(p, (size_t) (hi - lo))) loft_push(&out, (const char *) p, (size_t) (hi - lo));
    return out;
}

/* OpSliceVector (Stores::vector_slice): the elements [lo, hi) of `o`, clamped to it, appended to
 * `v` as ONE block — into the room `v` has when it fits (`append_block_fits`), else after one
 * growth to the total, the source re-read from its slot when it shares `v`'s store (the growth
 * may move it, and does when it IS `v`: `v += v[a..b]`).  The span lies below the source's
 * length and the new slots above the destination's, so they never overlap.  Scalar elements
 * only. */
static void loft_vec_slice(loft_ref v, loft_ref o, int64_t lo, int64_t hi, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || o.store_nr == 0xFFFF || o.rec == 0 || o.pos == 0) return;
    int64_t n64 = loft_vec_len(o);
    if (lo < 0) lo = 0;
    if (lo > n64) lo = n64;
    if (hi < lo) hi = lo;
    if (hi > n64) hi = n64;
    uint32_t n = (uint32_t) (hi - lo);
    if (n == 0 || size == 0) return;
    uint32_t o_rec = loft_collection_rec(loft_store_of(o), o.rec, o.pos);
    if (o_rec == 0) return;
    uint32_t from = 8 + (uint32_t) lo * size;
    loft_store_t *s = loft_store_of(v);
    uint32_t d_rec = loft_collection_rec(s, v.rec, v.pos);
    if (d_rec != 0 && d_rec != LOFT_FOREIGN_REC && !s->read_only) {
        int32_t claimed = loft_st_i32(s, d_rec, 0);
        uint32_t len = loft_u32(s, d_rec, 4);
        uint64_t cap = claimed > 0 ? ((uint64_t) claimed * 8 - 8) / size : 0;
        if ((uint64_t) len + n <= cap) {
            loft_ref dst = { v.store_nr, d_rec, 0 }, src = { o.store_nr, o_rec, 0 };
            loft_copy_bytes(dst, 8 + len * size, src, from, (size_t) n * size);
            loft_set_u32(s, d_rec, 4, len + n);
            return;
        }
    }
    uint32_t at = loft_vec_append(v, size).pos;
    loft_vec_set_size(v, n, size);
    uint32_t dest = loft_u32(loft_store_of(v), v.rec, v.pos);
    if (v.store_nr == o.store_nr) o_rec = loft_collection_rec(loft_store_of(o), o.rec, o.pos);
    loft_ref dst = { v.store_nr, dest, 0 }, src = { o.store_nr, o_rec, 0 };
    loft_copy_bytes(dst, at, src, from, (size_t) n * size);
}

/* OpSliceView (Stores::vector_slice_view): `v = o[lo..hi]` bound to a local that owns its store —
 * when `o` serves foreign bytes the local's store becomes a VIEW of the span, no copy, the span
 * sharing the owner; otherwise the copy loft_vec_slice makes.  `-DLOFT_NO_FOREIGN_VIEW` is the
 * interpreter's LOFT_NO_FOREIGN_VIEW: the copy everywhere. */
static void loft_vec_slice_view(loft_ref v, loft_ref o, int64_t lo, int64_t hi, uint32_t size) {
#if defined(LOFT_FOREIGN) && !defined(LOFT_NO_FOREIGN_VIEW)
    if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0 && o.store_nr != 0xFFFF && o.rec != 0
        && o.pos != 0 && v.store_nr != o.store_nr) {
        int64_t n = loft_vec_len(o);
        int64_t l = lo < 0 ? 0 : lo > n ? n : lo;
        int64_t h = hi < l ? l : hi > n ? n : hi;
        if (h > l && loft_st_is_foreign(loft_store_of(o))) {
            loft_fspan span = loft_st_foreign_span(loft_store_of(o), (uint32_t) l, (uint32_t) h);
            loft_store_t *s = loft_store_of(v);
            loft_st_release_foreign(s);
            loft_st_make_foreign(s, v.rec, v.pos, span);
            return;
        }
    }
#endif
    loft_vec_slice(v, o, lo, hi, size);
}

/* OpPlaceRecord (Stores::place_record_prefilled): a `tp` record claimed inside the host's store,
 * prefilled with its defaults, or in a fresh store of its own when there is no host.  `foreign`:
 * the host's root is a vector of another element type, so the record is noted
 * (`@FR-H-RootExtent`). */
static loft_ref loft_place_record(loft_ref host, uint16_t tp, uint32_t size, const uint8_t *image, uint32_t n,
                                  int foreign) {
    loft_ref r;
    if (host.store_nr == 0xFFFF || host.rec == 0 || host.store_nr >= loft_nstores) {
        r = loft_ref_null();
        loft_database(&r, size);
    } else {
        r.store_nr = host.store_nr;
        r.rec = loft_claim(loft_store_of(host), 1 + (size + 7) / 8);
        r.pos = 8;
        if (foreign) loft_st_note_placed(loft_store_of(r), r.rec);
    }
    loft_set_u32(loft_store_of(r), r.rec, 4, tp);
    loft_fill(r, image, n);
    return r;
}

/* OpEvacuate (Stores::evacuate, `@FR-H-RefHome`): the store `home` names is about to go; when
 * the stored reference at byte `fld` of `holder` names a record of it, the record is copied as
 * a `tp` record into the holder's own store and the reference names the copy. */
static void loft_evacuate(const loft_tdesc *T, uint16_t tp, loft_ref holder, uint32_t fld, loft_ref home,
                          uint32_t size, const uint8_t *image, uint32_t n, int foreign) {
    if (holder.rec == 0 || holder.store_nr == 0xFFFF || home.store_nr == 0xFFFF
        || holder.store_nr == home.store_nr || holder.store_nr >= loft_nstores)
        return;
    loft_ref old = loft_get_dbref(holder, fld);
    if (old.rec == 0 || old.store_nr != home.store_nr) return;
    loft_ref copy = loft_ref_null();
    for (uint32_t i = 0; i < loft_nevacs; i++) {
        loft_evac *e = &loft_evacs[i];
        if (e->old.store_nr == old.store_nr && e->old.rec == old.rec && e->old.pos == old.pos
            && e->holder == holder.store_nr) {
            copy = e->copy;
            break;
        }
    }
    if (copy.rec == 0) {
        copy = loft_place_record(holder, tp, size, image, n, foreign);
        loft_remove_claims(T, tp, copy);
        loft_copy_bytes(copy, 0, old, 0, T[tp].size);
        loft_copy_claims(T, tp, old, copy);
        if (loft_nevacs == loft_evac_cap) {
            loft_evac_cap = loft_evac_cap ? loft_evac_cap * 2 : 8;
            loft_evacs = (loft_evac *) realloc(loft_evacs, loft_evac_cap * sizeof *loft_evacs);
            if (!loft_evacs) { fputs("loft: out of memory\n", stderr); abort(); }
        }
        loft_evacs[loft_nevacs].old = old;
        loft_evacs[loft_nevacs].holder = holder.store_nr;
        loft_evacs[loft_nevacs].copy = copy;
        loft_nevacs++;
    }
    loft_set_dbref(holder, fld, copy);
}

/* OpFreeRecordIn (Stores::free_record_in): a store's root record frees the store; a placed
 * record releases what it owns and its block goes back to the host's store (`@FR-R-Place`). */
static void loft_free_record_in(const loft_tdesc *T, uint16_t tp, loft_ref r) {
    if (r.store_nr == 0xFFFF || r.rec == 0 || r.store_nr >= loft_nstores) return;
    if (r.rec == 1 && r.pos == 8) {
        loft_free_ref(r);
        return;
    }
    loft_remove_claims(T, tp, r);
    loft_st_delete(&loft_stores[r.store_nr], r.rec);
}

/* OpMoveRecord (Stores::move_record_out, `@FR-R-MoveLast`): the last use of an owned record
 * stored into a field.  Within one store the bytes relocate — the heap handles keep their
 * claims — and the source's block goes; across stores it is a deep copy and the source's
 * release. */
static void loft_move_record_out(const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    if (src.store_nr >= loft_nstores || dst.store_nr >= loft_nstores) return;
    loft_remove_claims(T, tp, dst);
    loft_copy_bytes(dst, 0, src, 0, T[tp].size);
    if (src.store_nr != dst.store_nr) {
        loft_copy_claims(T, tp, src, dst);
        loft_free_record_in(T, tp, src);
        return;
    }
    loft_st_delete(&loft_stores[src.store_nr], src.rec);
}

/* OpClear with `@FR-R-Reuse`'s reset flag (Stores::reset_pool_root): a pooled call buffer at its
 * store's root, the store minted for it, is made again as OpDatabase mints it and prefilled
 * with its type's defaults; answers 0, and the caller walks, for any other buffer.
 * `-DLOFT_NO_POOL_RESET` is the interpreter's LOFT_NO_POOL_RESET. */
static int loft_reset_pool_root(loft_ref r, uint16_t tp, uint32_t size, const uint8_t *image, uint32_t n) {
#ifdef LOFT_NO_POOL_RESET
    (void) r; (void) tp; (void) size; (void) image; (void) n;
    return 0;
#else
    if (r.store_nr == 0xFFFF || r.rec != 1 || r.pos != 8 || r.store_nr >= loft_nstores) return 0;
    loft_store_t *s = &loft_stores[r.store_nr];
    if (!s->live || s->pinned || s->known != tp || s->frame_holds) return 0;
    /* A locked store is not re-minted, nor one bound to a file or serving foreign bytes: the
     * caller walks. */
    if (s->read_only || s->mapped) return 0;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(s)) return 0;
#endif
    loft_database_typed(&r, size, tp);
    loft_fill(r, image, n);
    return 1;
#endif
}
