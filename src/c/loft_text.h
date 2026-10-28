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

static void loft_reserve(loft_text *t, size_t extra) {
    size_t need = t->len + extra;
    if (need <= t->cap) return;
    size_t cap = t->cap ? t->cap * 2 : 16;
    while (cap < need) cap *= 2;
    char *p = (char *) realloc(t->p, cap);
    if (!p) { fputs("loft: out of memory\n", stderr); abort(); }
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
 * is not a part of the buffer itself, the growth and the self-append out of line. */
LOFT_HOT_INLINE void loft_push(loft_text *t, const char *p, size_t n) {
    if (n == 0) return;
    if (LOFT_LIKELY(t->len + n <= t->cap) && (p < t->p || p >= t->p + t->cap)) {
        memcpy(t->p + t->len, p, n);
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
static void loft_text_concat(loft_text *t, loft_str s) {
    if (loft_str_is_null(loft_view(t))) return;
    if (loft_str_is_null(s)) {
        t->len = 0;
        loft_push_char(t, 0);
        return;
    }
    loft_text_append(t, s);
}

static void loft_text_set(loft_text *t, loft_str s) {
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
    free(t->p);
    t->p = 0;
    t->len = 0;
    t->cap = 0;
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

static int64_t loft_now_us(void) {
#if defined(LOFT_POSIX)
    struct timespec t;
    clock_gettime(CLOCK_MONOTONIC, &t);
    return (int64_t) t.tv_sec * 1000000 + t.tv_nsec / 1000;
#else
    return (int64_t) ((double) clock() * 1000000.0 / CLOCKS_PER_SEC);
#endif
}
