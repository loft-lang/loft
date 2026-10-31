/* ── Text: the runtime's byte strings, and the clock (`loft_rt.h` builds on these) ─────────────
 * `loft_str` is a view, `loft_text` an owned, growable buffer; both hold bytes, not necessarily
 * UTF-8.  One home for what the C runtime and the components loft ships (@PLN205, `libloft_net`)
 * both use, so neither carries a copy: `loft_cc.h` comes first, then this. */
#include <string.h>
#include <time.h>

typedef struct { const char *p; size_t len; } loft_str;
typedef struct { char *p; size_t len; size_t cap; } loft_text;


static loft_str loft_lit(const char *p, size_t len) {
    loft_str s;
    s.p = p;
    s.len = len;
    return s;
}

static loft_str loft_view(const loft_text *t) {
    return loft_lit(t->p ? t->p : "", t->len);
}

/* The null text: the one byte "\0". */
static loft_str loft_null_text(void) {
    static const char zero[1] = { 0 };
    return loft_lit(zero, 1);
}

static int loft_str_is_null(loft_str s) {
    return s.len == 1 && s.p[0] == 0;
}

/* A thread's freed text buffers by power-of-two capacity (16 .. 4096 bytes), handed to the next
 * text of that capacity instead of a malloc and a free: every temporary text a statement builds
 * and drops paid both.  Each buffer is an ordinary malloc block of exactly its capacity —
 * `loft_reserve` is the one place a capacity is set — so code that frees or reallocs a text's
 * buffer directly stays right; a buffer past the cache's depth goes back to malloc.  Measured on
 * the library benches' text routines (gcc -O3, one E-core): markdown render_inline -27 %,
 * server header -16 %. */
/* A header built without `loft_cc.h` keeps one cache: no threads. */
#ifndef LOFT_TLS
#define LOFT_TLS
#endif
#define LOFT_TP_CLASSES 9
#define LOFT_TP_DEPTH 32
static LOFT_TLS char *loft_tp_blocks[LOFT_TP_CLASSES][LOFT_TP_DEPTH];
static LOFT_TLS unsigned char loft_tp_n[LOFT_TP_CLASSES];
static int loft_tp_class(size_t cap) {
    if (cap < 16 || cap > 4096 || (cap & (cap - 1)) != 0) return -1;
    int c = 0;
    while (((size_t) 16 << c) < cap) c++;
    return c;
}
/* A sanitizer or poison build, or `-DLOFT_NO_TEXT_POOL`, frees every buffer: a reader of a freed
 * text then reads freed memory, which those builds report, not a buffer the cache handed on. */
#if defined(__SANITIZE_ADDRESS__) || defined(LOFT_POISON) || defined(LOFT_NO_TEXT_POOL)
#define LOFT_TP_OFF 1
#elif defined(__has_feature)
#if __has_feature(address_sanitizer) || __has_feature(memory_sanitizer)
#define LOFT_TP_OFF 1
#endif
#endif
static char *loft_tp_take(size_t cap) {
#ifdef LOFT_TP_OFF
    return (char *) malloc(cap);
#endif
    int c = loft_tp_class(cap);
    if (c >= 0 && loft_tp_n[c]) return loft_tp_blocks[c][--loft_tp_n[c]];
    return (char *) malloc(cap);
}
static void loft_tp_give(char *p, size_t cap) {
#ifdef LOFT_TP_OFF
    free(p);
    return;
#endif
    int c = loft_tp_class(cap);
    if (c >= 0 && loft_tp_n[c] < LOFT_TP_DEPTH) {
        loft_tp_blocks[c][loft_tp_n[c]++] = p;
        return;
    }
    free(p);
}
static void loft_reserve(loft_text *t, size_t extra) {
    size_t need = t->len + extra;
    if (need <= t->cap) return;
    size_t cap = t->cap ? t->cap * 2 : 16;
    while (cap < need) cap *= 2;
    char *p = loft_tp_take(cap);
    if (!p) { fputs("loft: out of memory\n", stderr); abort(); }
    if (t->p) {
        if (t->len) memcpy(p, t->p, t->len);
        loft_tp_give(t->p, t->cap);
    }
    t->p = p;
    t->cap = cap;
}

static void loft_push_slow(loft_text *t, const char *p, size_t n) {
    if (n == 0) return;
    if (t->p && p >= t->p && p < t->p + t->cap) {
        /* Appending a part of itself: the buffer may move, so copy the part first. */
        char *copy = (char *) malloc(n);
        if (!copy) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(copy, p, n);
        loft_reserve(t, n);
        memcpy(t->p + t->len, copy, n);
        free(copy);
    } else {
        loft_reserve(t, n);
        memcpy(t->p + t->len, p, n);
    }
    t->len += n;
}

/* The append every text build runs per piece: in place while the buffer has room and the piece
 * is not a part of the buffer itself, the growth and the self-append out of line.  A piece of
 * one character (up to four bytes) is copied byte by byte: its length is not known to the C
 * compiler, so `memcpy` would be a library call per character.  `-DLOFT_NO_SHORT_APPEND`
 * copies every piece with `memcpy`. */
LOFT_HOT_INLINE void loft_push(loft_text *t, const char *p, size_t n) {
    if (n == 0) return;
    if (LOFT_LIKELY(t->len + n <= t->cap) && (p < t->p || p >= t->p + t->cap)) {
        char *d = t->p + t->len;
#ifndef LOFT_NO_SHORT_APPEND
        if (n <= 4) {
            d[0] = p[0];
            if (n > 1) { d[1] = p[1]; if (n > 2) { d[2] = p[2]; if (n > 3) d[3] = p[3]; } }
        } else
#endif
        {
#if defined(__GNUC__) && !defined(__clang__) && __GNUC__ >= 11
/* A caller with a four-byte buffer (a code point's encoding) reaches this copy only with
 * `n <= 4`, which the branch above takes; gcc's overread check does not see that and would
 * fail strict mode on a copy that cannot run. */
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wstringop-overread"
#endif
            memcpy(d, p, n);
#if defined(__GNUC__) && !defined(__clang__) && __GNUC__ >= 11
#pragma GCC diagnostic pop
#endif
        }
        t->len += n;
        return;
    }
    loft_push_slow(t, p, n);
}

static void loft_push_char(loft_text *t, char c) {
    loft_push(t, &c, 1);
}

static void loft_text_append(loft_text *t, loft_str s) {
    loft_push(t, s.p, s.len);
}

/* OpAppendText (ops::append_text): a null text stays null, a null source makes it null. */
/* The two text writes every text statement runs are inlined, so a reset to "" or a short append
 * is a few compares and a copy in place, not a call.  Measured over every library bench (C ship
 * lane, gcc, 119 routines): 13 faster (slugify -16 %, render_inline -15 %, atlas_cell -16 %), 100
 * the same, five 2-5 % slower (html_escape, patch, materialise, fill_triangle, sha256_small), and
 * hex_neighbor +22 % from code placement alone (its function byte-identical, 16 bytes later).
 * `-DLOFT_NO_TEXT_INLINE` keeps them out of line. */
#ifdef LOFT_NO_TEXT_INLINE
#define LOFT_TEXT_INLINE static
#else
#define LOFT_TEXT_INLINE LOFT_HOT_INLINE
#endif

LOFT_TEXT_INLINE void loft_text_concat(loft_text *t, loft_str s) {
#ifndef LOFT_NO_SHORT_APPEND
    /* One byte into a buffer with room: a store and a length bump.  Behind a one-byte slice's
     * own fast path (`loft_text_sub`) the C compiler sees the length 1 and keeps only this. */
    if (s.len == 1 && s.p[0] != 0 && t->len < t->cap && !(t->len == 1 && t->p[0] == 0)) {
        char c = s.p[0];
        t->p[t->len++] = c;
        return;
    }
#endif
    if (loft_str_is_null(loft_view(t))) return;
    if (loft_str_is_null(s)) {
        t->len = 0;
        loft_push_char(t, 0);
        return;
    }
    loft_text_append(t, s);
}

LOFT_TEXT_INLINE void loft_text_set(loft_text *t, loft_str s) {
    if (t->p && s.p == t->p && s.len == t->len) return;
    if (t->p && s.p >= t->p && s.p < t->p + t->cap) {
        memmove(t->p, s.p, s.len);
        t->len = s.len;
        return;
    }
    t->len = 0;
    loft_push(t, s.p, s.len);
}

static void loft_text_free(loft_text *t) {
    if (t->p) loft_tp_give(t->p, t->cap);
    t->p = 0;
    t->len = 0;
    t->cap = 0;
}

/* `*to = *from` for a `from` nothing else names: `to` releases what it held and takes `from`'s
 * buffer, which is left empty — the value `loft_text_set` would copy, without the copy. */
static void loft_text_move(loft_text *to, loft_text *from) {
    if (to == from) return;
    loft_text_free(to);
    *to = *from;
    from->p = 0;
    from->len = 0;
    from->cap = 0;
}

static loft_text loft_text_own(loft_str s) {
    loft_text t = { 0, 0, 0 };
    loft_push(&t, s.p, s.len);
    return t;
}

/* A text as a NUL-terminated C string the caller frees: what a `#c` binding's `const char *`
 * parameter receives (`c_call`'s `TextPointer`). */
static char *loft_cstr_own(loft_str s) {
    char *c = (char *) malloc(s.len + 1);
    if (!c) { fputs("loft: out of memory\n", stderr); abort(); }
    if (s.len) memcpy(c, s.p, s.len);
    c[s.len] = 0;
    return c;
}

static void loft_push_cstr(loft_text *s, const char *p) {
    loft_push(s, p, strlen(p));
}

/* Is `p[0..n)` well-formed UTF-8, as Rust's `String::from_utf8` asks? */
static int loft_utf8_valid(const unsigned char *p, size_t n) {
    size_t i = 0;
    while (i < n) {
        unsigned char c = p[i];
        size_t len;
        uint32_t cp, min;
        if (c < 0x80) { i++; continue; }
        if (c >= 0xC2 && c <= 0xDF) { len = 2; cp = c & 0x1F; min = 0x80; }
        else if (c >= 0xE0 && c <= 0xEF) { len = 3; cp = c & 0x0F; min = 0x800; }
        else if (c >= 0xF0 && c <= 0xF4) { len = 4; cp = c & 0x07; min = 0x10000; }
        else return 0;
        if (i + len > n) return 0;
        for (size_t k = 1; k < len; k++) {
            if ((p[i + k] & 0xC0) != 0x80) return 0;
            cp = (cp << 6) | (p[i + k] & 0x3F);
        }
        if (cp < min || cp > 0x10FFFF || (cp >= 0xD800 && cp <= 0xDFFF)) return 0;
        i += len;
    }
    return 1;
}

/* `String::from_utf8_lossy` over bytes that may hold a NUL: an invalid byte is U+FFFD. */
static void loft_push_lossy(loft_text *t, const char *c, size_t n) {
    const unsigned char *p = (const unsigned char *) c;
    if (loft_utf8_valid(p, n)) {
        loft_push(t, c, n);
        return;
    }
    for (size_t i = 0; i < n;) {
        size_t len = p[i] < 0x80 ? 1 : (p[i] >= 0xC2 && p[i] <= 0xDF) ? 2
                   : (p[i] >= 0xE0 && p[i] <= 0xEF) ? 3 : (p[i] >= 0xF0 && p[i] <= 0xF4) ? 4 : 0;
        if (len && i + len <= n && loft_utf8_valid(p + i, len)) {
            loft_push(t, c + i, len);
            i += len;
        } else {
            loft_push(t, "\xEF\xBF\xBD", 3);
            i++;
        }
    }
}

#if !defined(LOFT_POSIX) && defined(_WIN32)
/* Windows' precise clocks, declared as `windows.h` declares them (a unit that opens a socket
 * includes it later): the wall clock in 100 ns ticks since 1601, and the monotonic counter.
 * `time()` counts whole seconds there, so `now()` read 0, 1000, 2000 … where the interpreter
 * reads milliseconds; `clock()` is no monotonic clock on a host whose CRT follows the standard. */
struct _FILETIME;
union _LARGE_INTEGER;
__declspec(dllimport) void __stdcall GetSystemTimePreciseAsFileTime(struct _FILETIME *ft);
__declspec(dllimport) int __stdcall QueryPerformanceCounter(union _LARGE_INTEGER *count);
__declspec(dllimport) int __stdcall QueryPerformanceFrequency(union _LARGE_INTEGER *freq);

/* Milliseconds since the Unix epoch. */
static int64_t loft_win_wall_ms(void) {
    unsigned long long ticks = 0; /* a FILETIME: the low word, then the high one */
    GetSystemTimePreciseAsFileTime((struct _FILETIME *) &ticks);
    return (int64_t) ((ticks - 116444736000000000ull) / 10000ull);
}
#endif

static int64_t loft_now_us(void) {
#if defined(LOFT_POSIX)
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (int64_t) t.tv_sec * 1000000 + t.tv_nsec / 1000;
#elif defined(_WIN32)
    static long long freq = 0;
    long long count = 0;
    if (!freq) QueryPerformanceFrequency((union _LARGE_INTEGER *) &freq);
    QueryPerformanceCounter((union _LARGE_INTEGER *) &count);
    return (int64_t) (count / freq * 1000000 + count % freq * 1000000 / freq);
#else
    return (int64_t) ((double) clock() * 1000000.0 / CLOCKS_PER_SEC);
#endif
}
