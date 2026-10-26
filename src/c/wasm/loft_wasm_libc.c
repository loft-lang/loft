/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The C library a package's `[c] shim` sees on a wasm target (@PLN24 arc E, the shim-to-wasm
 * route): the parts of the hosted library a shim that is pure computation needs — the
 * string functions, an allocator over the module's own memory, a `snprintf` of the common
 * conversions, `abort` — written freestanding, so one object serves `--native-wasm`
 * (wasip2, beside the WASI libc Rust links) and `--html` (wasm32-unknown-unknown, where
 * there is no libc at all).  Every symbol is prefixed `loft_w_` and reached through the
 * `#define`s of the headers beside this file, so nothing here collides with a libc the
 * module also links.  `memcpy`, `memmove`, `memset` and `memcmp` are NOT defined here: the
 * compiler emits calls to them for a struct copy, and the module always has them (the WASI
 * libc, or Rust's compiler builtins on wasm32-unknown-unknown).
 *
 * There is no file, no clock and no entropy here: a shim that opens a file does not build
 * for a wasm target, which is the right answer — the browser has no files. */
#include <stddef.h>
#include <stdint.h>
#include <stdarg.h>

void *memcpy(void *dst, const void *src, size_t n);
void *memmove(void *dst, const void *src, size_t n);
void *memset(void *dst, int c, size_t n);
int memcmp(const void *a, const void *b, size_t n);

/* ── strings ─────────────────────────────────────────────────────────────────────────────── */

size_t loft_w_strlen(const char *s) {
    size_t n = 0;
    while (s[n]) n++;
    return n;
}

int loft_w_strcmp(const char *a, const char *b) {
    while (*a && *a == *b) { a++; b++; }
    return (int) (unsigned char) *a - (int) (unsigned char) *b;
}

int loft_w_strncmp(const char *a, const char *b, size_t n) {
    while (n && *a && *a == *b) { a++; b++; n--; }
    return n ? (int) (unsigned char) *a - (int) (unsigned char) *b : 0;
}

char *loft_w_strcpy(char *dst, const char *src) {
    char *d = dst;
    while ((*d++ = *src++)) {}
    return dst;
}

char *loft_w_strncpy(char *dst, const char *src, size_t n) {
    size_t i = 0;
    for (; i < n && src[i]; i++) dst[i] = src[i];
    for (; i < n; i++) dst[i] = 0;
    return dst;
}

char *loft_w_strcat(char *dst, const char *src) {
    loft_w_strcpy(dst + loft_w_strlen(dst), src);
    return dst;
}

char *loft_w_strchr(const char *s, int c) {
    for (;; s++) {
        if (*s == (char) c) return (char *) s;
        if (!*s) return 0;
    }
}

char *loft_w_strrchr(const char *s, int c) {
    const char *last = 0;
    for (;; s++) {
        if (*s == (char) c) last = s;
        if (!*s) return (char *) last;
    }
}

char *loft_w_strstr(const char *h, const char *n) {
    size_t ln = loft_w_strlen(n);
    if (!ln) return (char *) h;
    for (; *h; h++)
        if (*h == *n && loft_w_strncmp(h, n, ln) == 0) return (char *) h;
    return 0;
}

void *loft_w_memchr(const void *s, int c, size_t n) {
    const unsigned char *p = (const unsigned char *) s;
    for (; n; n--, p++)
        if (*p == (unsigned char) c) return (void *) p;
    return 0;
}

/* ── the allocator: first-fit over a free list, memory grown a page at a time ───────────── */

typedef struct loft_w_block {
    size_t size;                 /* payload bytes, a multiple of 16 */
    struct loft_w_block *next;   /* on the free list */
} loft_w_block;

static loft_w_block *loft_w_free_list = 0;

static void *loft_w_grow(size_t bytes) {
    size_t pages = (bytes + 65535) / 65536;
    size_t before = __builtin_wasm_memory_grow(0, pages);
    if (before == (size_t) -1) return 0;
    return (void *) (before * 65536);
}

void *loft_w_malloc(size_t n) {
    loft_w_block **pp, *b;
    size_t need;
    if (n == 0) n = 1;
    need = (n + 15) & ~(size_t) 15;
    for (pp = &loft_w_free_list; (b = *pp); pp = &b->next) {
        if (b->size >= need) {
            if (b->size >= need + sizeof(loft_w_block) + 16) {
                loft_w_block *rest = (loft_w_block *) ((char *) (b + 1) + need);
                rest->size = b->size - need - sizeof(loft_w_block);
                rest->next = b->next;
                b->size = need;
                *pp = rest;
            } else {
                *pp = b->next;
            }
            b->next = 0;
            return b + 1;
        }
    }
    {
        size_t total = need + sizeof(loft_w_block);
        char *mem = (char *) loft_w_grow(total);
        size_t got;
        if (!mem) return 0;
        got = ((total + 65535) / 65536) * 65536;
        b = (loft_w_block *) mem;
        b->size = need;
        b->next = 0;
        if (got - total >= sizeof(loft_w_block) + 16) {
            loft_w_block *rest = (loft_w_block *) (mem + total);
            rest->size = got - total - sizeof(loft_w_block);
            rest->next = loft_w_free_list;
            loft_w_free_list = rest;
        }
        return b + 1;
    }
}

void loft_w_free(void *p) {
    loft_w_block *b;
    if (!p) return;
    b = (loft_w_block *) p - 1;
    b->next = loft_w_free_list;
    loft_w_free_list = b;
}

void *loft_w_calloc(size_t n, size_t size) {
    size_t total;
    void *p;
    if (size && n > (size_t) -1 / size) return 0;
    total = n * size;
    p = loft_w_malloc(total);
    if (p) memset(p, 0, total);
    return p;
}

void *loft_w_realloc(void *p, size_t n) {
    loft_w_block *b;
    void *q;
    if (!p) return loft_w_malloc(n);
    b = (loft_w_block *) p - 1;
    if (b->size >= n) return p;
    q = loft_w_malloc(n);
    if (!q) return 0;
    memcpy(q, p, b->size);
    loft_w_free(p);
    return q;
}

void loft_w_abort(void) {
    __builtin_trap();
}

int loft_w_abs(int v) { return v < 0 ? -v : v; }
long loft_w_labs(long v) { return v < 0 ? -v : v; }
long long loft_w_llabs(long long v) { return v < 0 ? -v : v; }

/* ── snprintf: %s %c %d %i %u %x %X %ld %lld %lu %llu %zu %% and a field width ──────────── */

typedef struct { char *out; size_t cap, len; } loft_w_sink;

static void loft_w_put(loft_w_sink *s, char c) {
    if (s->len + 1 < s->cap) s->out[s->len] = c;
    s->len++;
}

static void loft_w_puts(loft_w_sink *s, const char *p, int width, int left) {
    size_t n = loft_w_strlen(p), i;
    int pad = width > (int) n ? width - (int) n : 0;
    if (!left) while (pad-- > 0) loft_w_put(s, ' ');
    for (i = 0; i < n; i++) loft_w_put(s, p[i]);
    if (left) while (pad-- > 0) loft_w_put(s, ' ');
}

static void loft_w_putnum(loft_w_sink *s, unsigned long long v, int neg, int base, int upper, int width, int left, int zero) {
    char buf[24];
    int i = 23;
    buf[i] = 0;
    if (v == 0) buf[--i] = '0';
    while (v) {
        int d = (int) (v % (unsigned) base);
        buf[--i] = (char) (d < 10 ? '0' + d : (upper ? 'A' : 'a') + d - 10);
        v /= (unsigned) base;
    }
    if (neg) buf[--i] = '-';
    if (zero && !left) {
        int n = 23 - i;
        while (width-- > n) loft_w_put(s, '0');
        width = 0;
    }
    loft_w_puts(s, buf + i, width, left);
}

int loft_w_vsnprintf(char *out, size_t cap, const char *fmt, va_list ap) {
    loft_w_sink s;
    s.out = out;
    s.cap = cap;
    s.len = 0;
    for (; *fmt; fmt++) {
        int left = 0, zero = 0, width = 0, longs = 0, sizet = 0;
        if (*fmt != '%') { loft_w_put(&s, *fmt); continue; }
        fmt++;
        for (;; fmt++) {
            if (*fmt == '-') left = 1;
            else if (*fmt == '0') zero = 1;
            else break;
        }
        while (*fmt >= '0' && *fmt <= '9') width = width * 10 + (*fmt++ - '0');
        while (*fmt == 'l') { longs++; fmt++; }
        if (*fmt == 'z') { sizet = 1; fmt++; }
        switch (*fmt) {
        case 's': { const char *p = va_arg(ap, const char *); loft_w_puts(&s, p ? p : "(null)", width, left); break; }
        case 'c': { char c = (char) va_arg(ap, int); loft_w_put(&s, c); break; }
        case 'd': case 'i': {
            long long v = longs >= 2 ? va_arg(ap, long long) : longs == 1 ? va_arg(ap, long) : sizet ? (long long) va_arg(ap, size_t) : va_arg(ap, int);
            loft_w_putnum(&s, v < 0 ? (unsigned long long) (-(v + 1)) + 1u : (unsigned long long) v, v < 0, 10, 0, width, left, zero);
            break;
        }
        case 'u': case 'x': case 'X': {
            unsigned long long v = longs >= 2 ? va_arg(ap, unsigned long long) : longs == 1 ? va_arg(ap, unsigned long) : sizet ? va_arg(ap, size_t) : va_arg(ap, unsigned);
            loft_w_putnum(&s, v, 0, *fmt == 'u' ? 10 : 16, *fmt == 'X', width, left, zero);
            break;
        }
        case '%': loft_w_put(&s, '%'); break;
        case 0: fmt--; break;
        default: loft_w_put(&s, '%'); loft_w_put(&s, *fmt); break;
        }
    }
    if (cap) out[s.len < cap ? s.len : cap - 1] = 0;
    return (int) s.len;
}

int loft_w_snprintf(char *out, size_t cap, const char *fmt, ...) {
    va_list ap;
    int n;
    va_start(ap, fmt);
    n = loft_w_vsnprintf(out, cap, fmt, ap);
    va_end(ap);
    return n;
}

/* `stderr` writes go nowhere: a wasm module has no terminal of its own here. */
int loft_w_fputs(const char *s, void *stream) {
    (void) s;
    (void) stream;
    return 0;
}

int loft_w_fprintf(void *stream, const char *fmt, ...) {
    (void) stream;
    (void) fmt;
    return 0;
}
