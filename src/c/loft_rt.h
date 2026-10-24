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
#include <stdint.h>
#include <stddef.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <setjmp.h>
#include <math.h>

#define LOFT_NULL INT64_MIN
#define LOFT_BOOL_NULL 255

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

static void loft_push(loft_text *t, const char *p, size_t n) {
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

static loft_text loft_text_own(loft_str s) {
    loft_text t = { 0, 0, 0 };
    loft_push(&t, s.p, s.len);
    return t;
}

static void loft_text_free(loft_text *t) {
    free(t->p);
    t->p = 0;
    t->len = 0;
    t->cap = 0;
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

/* ops::push_decimal */
static void loft_push_decimal(loft_text *s, int neg, uint64_t abs, int plus, int64_t width,
                              int dir, uint8_t token) {
    size_t start = s->len;
    if (neg) loft_push_char(s, '-');
    else if (plus) loft_push_char(s, '+');
    size_t sign = s->len - start;
    char buf[24];
    int n = snprintf(buf, sizeof buf, "%llu", (unsigned long long) abs);
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

static void loft_print(loft_str s) {
    fwrite(s.p, 1, s.len, stdout);
}

/* The integer operators: null in, null out; an overflow or a division by zero is null. */
static int64_t loft_add(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    if ((b > 0 && a > INT64_MAX - b) || (b < 0 && a < INT64_MIN - b)) return LOFT_NULL;
    return a + b;
}

static int64_t loft_sub(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    if ((b < 0 && a > INT64_MAX + b) || (b > 0 && a < INT64_MIN + b)) return LOFT_NULL;
    return a - b;
}

static int64_t loft_mul(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL) return LOFT_NULL;
    if (a > 0) {
        if (b > 0) { if (a > INT64_MAX / b) return LOFT_NULL; }
        else if (b < INT64_MIN / a) return LOFT_NULL;
    } else if (a < 0) {
        if (b > 0) { if (a < INT64_MIN / b) return LOFT_NULL; }
        else if (b != 0 && a < INT64_MAX / b) return LOFT_NULL;
    }
    return a * b;
}

static int64_t loft_div(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b == 0) return LOFT_NULL;
    return a / b;
}

static int64_t loft_rem(int64_t a, int64_t b) {
    if (a == LOFT_NULL || b == LOFT_NULL || b == 0) return LOFT_NULL;
    return a % b;
}

/* `assert`: a failed one ends the running test (the test main catches it) or, outside a
 * test, the program, with the interpreter's message on stderr and exit status 1. */
static jmp_buf loft_test_jmp;
static int loft_in_test = 0;

/* The message of the last failed assert, for a test expected to fail with it. */
static loft_text loft_fail_msg = { 0, 0, 0 };

static void loft_assert(uint8_t test, loft_str message, loft_str file, int64_t line) {
    (void) file;
    (void) line;
    if (test == 1) return;
    fputs("error: assertion failed: ", stderr);
    fwrite(message.p, 1, message.len, stderr);
    fputc('\n', stderr);
    loft_fail_msg.len = 0;
    loft_push(&loft_fail_msg, "assertion failed: ", 18);
    loft_push(&loft_fail_msg, message.p, message.len);
    if (loft_in_test) longjmp(loft_test_jmp, 1);
    fflush(stdout);
    exit(1);
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
static loft_str loft_text_sub(loft_str v, int64_t from64, int64_t till64) {
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

/* OpAppendCharacter: the null character appends nothing. */
static void loft_push_code(loft_text *t, uint32_t c) {
    char buf[4];
    size_t n = c == 0 ? 0 : loft_utf8(c, buf);
    loft_push(t, buf, n);
}

/* ── Stores ────────────────────────────────────────────────────────────────────────────────
 * A port of the store model the IR addresses (src/store.rs, src/vector.rs): a store is an
 * array of 8-byte words; a record is a run of words whose first word holds its claimed size
 * (and, at byte 4 of a vector record, the vector's length); a reference (`DbRef`) names a
 * store, a record and a byte position, and a field is `rec * 8 + pos + offset`.  `rec == 0`
 * is the null reference.  The allocator is a plain bump allocator: which words a claim gets
 * is not observable, and a store's words go back together when the store is freed. */
typedef struct { uint16_t store_nr; uint32_t rec; uint32_t pos; } loft_ref;

/* `known`: the type a wrapper mint gave the store's first record, 0xFFFF for any other. */
typedef struct { uint8_t *buf; uint32_t words; uint32_t top; int live; uint16_t known; int pinned; } loft_store_t;

static loft_store_t *loft_stores = 0;
static uint32_t loft_nstores = 0;

static loft_ref loft_ref_null(void) {
    loft_ref r;
    r.store_nr = 0xFFFF;
    r.rec = 0;
    r.pos = 0;
    return r;
}

static loft_store_t *loft_store_of(loft_ref r) {
    return &loft_stores[r.store_nr];
}

static uint32_t loft_store_new(void) {
    uint32_t nr = 0;
    while (nr < loft_nstores && loft_stores[nr].live) nr++;
    if (nr == loft_nstores) {
        uint32_t n = loft_nstores ? loft_nstores * 2 : 16;
        loft_store_t *s = (loft_store_t *) realloc(loft_stores, n * sizeof *s);
        if (!s) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(s + loft_nstores, 0, (n - loft_nstores) * sizeof *s);
        loft_stores = s;
        loft_nstores = n;
    }
    loft_store_t *s = &loft_stores[nr];
    if (!s->buf) {
        s->words = 64;
        s->buf = (uint8_t *) calloc(s->words, 8);
        if (!s->buf) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    s->top = 1; /* word 0 is never a record: rec 0 is the null reference */
    s->live = 1;
    s->known = 0xFFFF;
    s->pinned = 0;
    return nr;
}

static uint32_t loft_claim(loft_store_t *s, uint32_t words) {
    if (words < 1) words = 1;
    if (s->top + words > s->words) {
        uint32_t n = s->words;
        while (s->top + words > n) n *= 2;
        uint8_t *b = (uint8_t *) realloc(s->buf, (size_t) n * 8);
        if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(b + (size_t) s->words * 8, 0, (size_t) (n - s->words) * 8);
        s->buf = b;
        s->words = n;
    }
    uint32_t rec = s->top;
    s->top += words;
    memset(s->buf + (size_t) rec * 8, 0, (size_t) words * 8);
    int32_t size = (int32_t) words;
    memcpy(s->buf + (size_t) rec * 8, &size, 4);
    return rec;
}

static uint8_t *loft_at(loft_store_t *s, uint32_t rec, uint32_t byte) {
    return s->buf + (size_t) rec * 8 + byte;
}

static uint32_t loft_u32(loft_store_t *s, uint32_t rec, uint32_t byte) {
    uint32_t v;
    memcpy(&v, loft_at(s, rec, byte), 4);
    return v;
}

static void loft_set_u32(loft_store_t *s, uint32_t rec, uint32_t byte, uint32_t v) {
    memcpy(loft_at(s, rec, byte), &v, 4);
}

/* OpDatabase: a fresh record of `bytes` in the variable's store, cleared in place, or in a
 * new store when the variable holds none. */
static void loft_database(loft_ref *var, uint32_t bytes) {
    uint32_t nr;
    if (var->store_nr != 0xFFFF && var->store_nr < loft_nstores && loft_stores[var->store_nr].live) {
        nr = var->store_nr;
        loft_stores[nr].top = 1;
    } else {
        nr = loft_store_new();
    }
    uint32_t rec = loft_claim(&loft_stores[nr], (bytes + 15) / 8);
    loft_stores[nr].known = 0xFFFF;
    var->store_nr = (uint16_t) nr;
    var->rec = rec;
    var->pos = 8;
}

/* OpDatabase of a vector wrapper (`main_vector<T>`, Stores::mint_at_place, @FR-R-Place): a
 * buffer that is a PLACE — a collection field of another record, not the wrapper's own store
 * — stays where it is with its collection emptied, so the literal builds where it lives;
 * the ordinary mint would clear the store the place lives in. */
static void loft_database_wrapper(loft_ref *var, uint32_t bytes, uint16_t tp) {
    if (var->store_nr != 0xFFFF && var->rec != 0 && var->store_nr < loft_nstores
        && !(var->rec == 1 && loft_stores[var->store_nr].known == tp)) {
        loft_set_u32(&loft_stores[var->store_nr], var->rec, var->pos, 0);
        return;
    }
    loft_database(var, bytes);
    loft_stores[var->store_nr].known = tp;
}

/* OpFreeRef: the whole store goes, everything the root owned with it. */
static void loft_free_ref(loft_ref r) {
    if (r.store_nr == 0xFFFF || r.store_nr >= loft_nstores) return;
    loft_store_t *s = &loft_stores[r.store_nr];
    if (s->pinned) return;
    s->live = 0;
    s->top = 1;
}

/* A constant built at compile time (compile.rs build_const_vectors): its store's words as the
 * interpreter laid them out, loaded once and pinned for the whole run.  The vector is the
 * field at byte 8 of the store's first record. */
static loft_ref loft_const_store(const uint32_t *words, uint32_t n) {
    uint32_t nr = loft_store_new();
    loft_store_t *s = &loft_stores[nr];
    if (n > s->words) {
        uint8_t *b = (uint8_t *) realloc(s->buf, (size_t) n * 8);
        if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
        s->buf = b;
        s->words = n;
    }
    memcpy(s->buf, words, (size_t) n * 8);
    s->top = n;
    s->pinned = 1;
    loft_ref r = { (uint16_t) nr, 1, 8 };
    return r;
}

static loft_ref loft_field(loft_ref r, uint32_t fld) {
    r.pos += fld;
    return r;
}

static int64_t loft_get_int(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    int64_t v;
    memcpy(&v, loft_at(loft_store_of(r), r.rec, r.pos + fld), 8);
    return v;
}

static void loft_set_int(loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &v, 8);
}

static void loft_set_int4(loft_ref r, uint32_t fld, int64_t v) {
    if (r.rec == 0) return;
    int32_t x = v == LOFT_NULL ? INT32_MIN : (int32_t) v;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &x, 4);
}

/* The narrow integer fields: a byte biased by `min` (255 the nullable form's null), a short
 * in the `+1` encoding (0 is null), a raw short (0xFFFF its null), a 4-byte int (INT32_MIN).
 * A value a field cannot hold takes its default: the nullable forms' is null. */
static int32_t loft_i32_of(int64_t v) {
    return v == LOFT_NULL ? INT32_MIN : (int32_t) (uint32_t) (uint64_t) v;
}

static int64_t loft_get_byte(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    return *loft_at(loft_store_of(r), r.rec, r.pos + fld) + min;
}

static int64_t loft_get_byte_nullable(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    uint8_t raw = *loft_at(loft_store_of(r), r.rec, r.pos + fld);
    return raw == 255 ? LOFT_NULL : raw + min;
}

static uint8_t loft_byte_raw(int64_t min, int64_t val);

static void loft_set_byte_v(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    *loft_at(loft_store_of(r), r.rec, r.pos + fld) = loft_byte_raw(min, v);
}

static void loft_set_byte_nullable(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    int32_t x = loft_i32_of(v);
    if (!(x == INT32_MIN || ((int64_t) x >= min && (int64_t) x <= min + 255))) x = INT32_MIN;
    *loft_at(loft_store_of(r), r.rec, r.pos + fld) = loft_byte_raw(min, x);
}

static int64_t loft_get_short(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    uint16_t raw;
    memcpy(&raw, loft_at(loft_store_of(r), r.rec, r.pos + fld), 2);
    return raw == 0 ? LOFT_NULL : raw + min - 1;
}

static void loft_set_short(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    int32_t x = loft_i32_of(v);
    uint16_t raw = 0;
    if (x != INT32_MIN && (int64_t) x >= min && (int64_t) x <= min + 65534) raw = (uint16_t) ((int64_t) x - min + 1);
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &raw, 2);
}

static int64_t loft_get_short_raw(loft_ref r, uint32_t fld, int64_t min) {
    if (r.rec == 0) return LOFT_NULL;
    uint16_t raw;
    memcpy(&raw, loft_at(loft_store_of(r), r.rec, r.pos + fld), 2);
    return raw + min;
}

static void loft_set_short_raw(loft_ref r, uint32_t fld, int64_t min, int64_t v) {
    if (r.rec == 0) return;
    int32_t x = loft_i32_of(v);
    uint16_t raw;
    if (x == INT32_MIN) raw = 0xFFFF;
    else if ((int64_t) x >= min && (int64_t) x <= min + 65535) raw = (uint16_t) ((int64_t) x - min);
    else raw = 0;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &raw, 2);
}

static int64_t loft_get_int4(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return LOFT_NULL;
    int32_t x;
    memcpy(&x, loft_at(loft_store_of(r), r.rec, r.pos + fld), 4);
    return x == INT32_MIN ? LOFT_NULL : x;
}

static uint32_t loft_get_char(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return 0;
    uint32_t c;
    memcpy(&c, loft_at(loft_store_of(r), r.rec, r.pos + fld), 4);
    return c > 0x10FFFF || (c >= 0xD800 && c <= 0xDFFF) ? 0 : c;
}

static void loft_set_char(loft_ref r, uint32_t fld, uint32_t c) {
    if (r.rec == 0) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &c, 4);
}

/* ── Vectors (src/vector.rs) ───────────────────────────────────────────────────────────── */

static uint32_t loft_collection_rec(loft_store_t *s, uint32_t rec, uint32_t pos) {
    uint32_t v = loft_u32(s, rec, pos);
    return v == 0xFFFFFFFFu ? 0 : v;
}

static uint32_t loft_vec_cap_words(uint32_t count, uint32_t size) {
    return (uint32_t) (((uint64_t) count * size + 15) / 8);
}

static int64_t loft_vec_len(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return 0;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    return r == 0 ? 0 : (int64_t) loft_u32(s, r, 4);
}

static void loft_vec_prealloc(loft_ref v, uint32_t count, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(v);
    if (loft_collection_rec(s, v.rec, v.pos) != 0) return;
    uint32_t r = loft_claim(s, loft_vec_cap_words(count < 11 ? 11 : count, size));
    loft_set_u32(s, v.rec, v.pos, r);
    loft_set_u32(s, r, 4, 0);
}

/* vector_append: room for one more element of `size`; the reference to its slot.  The
 * length is not bumped: a push bumps it once its value is written (append_done), a new record
 * once it is finished (vector_finish). */
static loft_ref loft_vec_append(loft_ref v, uint32_t size) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return loft_ref_null();
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    uint32_t len;
    if (r == 0) {
        r = loft_claim(s, loft_vec_cap_words(11, size));
        loft_set_u32(s, v.rec, v.pos, r);
        loft_set_u32(s, r, 4, 0);
        len = 0;
    } else {
        len = loft_u32(s, r, 4);
        int32_t claimed;
        memcpy(&claimed, loft_at(s, r, 0), 4);
        uint32_t cap = (uint32_t) (((uint64_t) claimed * 8 - 8) / size);
        if (len + 1 > cap) {
            uint32_t nr = loft_claim(s, loft_vec_cap_words((len + 1) * 2, size));
            memcpy(loft_at(s, nr, 4), loft_at(s, r, 4), 4 + (size_t) len * size);
            loft_set_u32(s, v.rec, v.pos, nr);
            r = nr;
        }
    }
    loft_ref e;
    e.store_nr = v.store_nr;
    e.rec = r;
    e.pos = 8 + len * size;
    return e;
}

/* vector_finish: the element `loft_vec_append` made counts. */
static void loft_vec_finish(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0) return;
    loft_store_t *s = loft_store_of(v);
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    loft_set_u32(s, r, 4, loft_u32(s, r, 4) + 1);
}

static void loft_vec_push_int(loft_ref v, int64_t x) {
    loft_ref e = loft_vec_append(v, 8);
    if (e.rec == 0) return;
    loft_set_int(e, 0, x);
    loft_vec_finish(v);
}

/* A new record's defaults: the bytes the interpreter's default walk writes for its type
 * (Stores::set_default_value, captured by the emitter as `image`). */
static void loft_fill(loft_ref r, const uint8_t *image, uint32_t n) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos), image, n);
}

/* get_vector: the element at `from` (negative counts from the end), or the null reference. */
static loft_ref loft_vec_get(loft_ref v, uint32_t size, int64_t from) {
    if (v.store_nr == 0xFFFF) return loft_ref_null();
    int64_t len = loft_vec_len(v);
    int64_t f = from < 0 ? from + len : from;
    if (f < 0 || f >= len) return loft_ref_null();
    loft_ref e;
    e.store_nr = v.store_nr;
    e.rec = loft_collection_rec(loft_store_of(v), v.rec, v.pos);
    e.pos = (uint32_t) (8 + (uint64_t) f * size);
    return e;
}


/* ops::op_negate_long: null stays null. */
static int64_t loft_neg(int64_t a) {
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
static void loft_free_displaced(loft_ref displaced, loft_ref witness) {
    if (displaced.store_nr == witness.store_nr || displaced.store_nr == 0xFFFF) return;
    loft_free_ref(displaced);
}

/* vector::clear_vector: an absent slot becomes the empty vector; a present one length 0. */
static void loft_vec_clear(loft_ref v) {
    if (v.store_nr == 0xFFFF || v.rec == 0 || v.pos == 0) return;
    loft_store_t *s = loft_store_of(v);
    if (loft_u32(s, v.rec, v.pos) == 0xFFFFFFFFu) { loft_set_u32(s, v.rec, v.pos, 0); return; }
    uint32_t r = loft_collection_rec(s, v.rec, v.pos);
    if (r != 0) loft_set_u32(s, r, 4, 0);
}


/* ── Bytes, enums and text in records ──────────────────────────────────────────────────── */

static void loft_set_byte(loft_ref r, uint32_t fld, uint8_t v) {
    if (r.rec == 0) return;
    *loft_at(loft_store_of(r), r.rec, r.pos + fld) = v;
}

/* OpGetEnum: 0 for the null reference, the stored byte otherwise. */
static uint8_t loft_get_enum(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return 0;
    return *loft_at(loft_store_of(r), r.rec, r.pos + fld);
}

static void loft_vec_push_byte(loft_ref v, uint8_t x) {
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
static loft_str loft_get_text(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return loft_null_text();
    loft_store_t *s = loft_store_of(r);
    uint32_t srec = loft_u32(s, r.rec, r.pos + fld);
    if (srec == 0 || srec > 0x7FFFFFFFu) return loft_null_text();
    return loft_lit((const char *) loft_at(s, srec, 8), loft_u32(s, srec, 4));
}

/* OpSetText (Store::set_str): a new string record in the record's store.  A value that lives
 * in that store is copied out first, because the claim may move the buffer. */
static void loft_set_text(loft_ref r, uint32_t fld, loft_str v) {
    if (r.rec == 0) return;
    loft_store_t *s = loft_store_of(r);
    loft_text copy = { 0, 0, 0 };
    if (v.p >= (const char *) s->buf && v.p < (const char *) s->buf + (size_t) s->words * 8) {
        copy = loft_text_own(v);
        v = loft_view(&copy);
    }
    uint32_t srec = loft_claim(s, (uint32_t) ((v.len + 15) / 8));
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

typedef struct { uint16_t pos; uint16_t type; } loft_fld;
typedef struct {
    uint8_t kind;
    uint16_t size;             /* the type's bytes */
    uint16_t elem;             /* a vector's element type, a child record's content type */
    uint16_t n;                /* fields, or variants */
    const loft_fld *fields;    /* a struct's fields */
    const uint16_t *variants;  /* a struct-enum's variant types, 0xFFFF for a unit variant */
} loft_tdesc;

/* Copy `n` bytes between two places that may share a buffer a claim could move. */
static void loft_copy_bytes(loft_ref to, uint32_t to_off, loft_ref from, uint32_t from_off, size_t n) {
    if (n == 0) return;
    memmove(loft_at(loft_store_of(to), to.rec, to.pos + to_off),
            loft_at(loft_store_of(from), from.rec, from.pos + from_off), n);
}

static void loft_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    loft_store_t *fs = loft_store_of(from);
    switch (d->kind) {
    case LOFT_T_TEXT: {
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0) { loft_set_u32(loft_store_of(to), to.rec, to.pos, 0); return; }
        loft_text tmp = loft_text_own(loft_lit((const char *) loft_at(fs, cur, 8), loft_u32(fs, cur, 4)));
        loft_store_t *ts = loft_store_of(to);
        uint32_t srec = loft_claim(ts, (uint32_t) ((tmp.len + 15) / 8));
        loft_set_u32(ts, srec, 4, (uint32_t) tmp.len);
        if (tmp.len) memcpy(loft_at(ts, srec, 8), tmp.p, tmp.len);
        loft_set_u32(ts, to.rec, to.pos, srec);
        loft_text_free(&tmp);
        return;
    }
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) {
            loft_copy_claims(T, d->fields[i].type, loft_field(from, d->fields[i].pos), loft_field(to, d->fields[i].pos));
        }
        return;
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0 || cur == 0xFFFFFFFFu) { loft_set_u32(loft_store_of(to), to.rec, to.pos, cur); return; }
        uint32_t len = loft_u32(fs, cur, 4);
        uint32_t size = T[d->elem].size;
        uint32_t into = loft_claim(loft_store_of(to), 1 + (uint32_t) (((uint64_t) size * len + 7) / 8));
        loft_set_u32(loft_store_of(to), to.rec, to.pos, into);
        loft_ref src_v = { from.store_nr, cur, 0 };
        loft_ref dst_v = { to.store_nr, into, 0 };
        loft_copy_bytes(dst_v, 4, src_v, 4, (size_t) len * size + 4);
        if (T[d->elem].kind == LOFT_T_SCALAR) return;
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
    case LOFT_T_CHILD: {
        uint32_t cur = loft_u32(fs, from.rec, from.pos);
        if (cur == 0) { loft_set_u32(loft_store_of(to), to.rec, to.pos, 0); return; }
        uint32_t size = T[d->elem].size;
        uint32_t into = loft_claim(loft_store_of(to), size);
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
static void loft_copy_record(const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dst, int free_source) {
    if (src.store_nr == dst.store_nr && src.rec == dst.rec && src.pos == dst.pos) return;
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    loft_copy_bytes(dst, 0, src, 0, T[tp].size);
    loft_copy_claims(T, tp, src, dst);
    if (free_source && src.store_nr != dst.store_nr) loft_free_ref(src);
}

/* OpRemoveVector (vector::remove_vector): the element at `index` (negative from the end) goes,
 * its successors move down; false for a null vector or an index outside it. */
static int loft_vec_remove(loft_ref v, uint32_t size, int64_t index) {
    if (v.store_nr == 0xFFFF) return 0;
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
 * refilled with deep copies of the source's elements. */
static void loft_vec_add(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o);

static void loft_vec_replace(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
    int absent = o.store_nr == 0xFFFF
        || (o.rec != 0 && o.pos != 0 && loft_u32(loft_store_of(o), o.rec, o.pos) == 0xFFFFFFFFu);
    if (absent) {
        loft_vec_clear(v);
        if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0) loft_set_u32(loft_store_of(v), v.rec, v.pos, 0xFFFFFFFFu);
        return;
    }
    if (v.store_nr == o.store_nr) {
        uint32_t dr = loft_u32(loft_store_of(v), v.rec, v.pos);
        if (dr != 0 && dr == loft_u32(loft_store_of(o), o.rec, o.pos)) return;
    }
    loft_vec_clear(v);
    loft_vec_add(T, tp, v, o);
}

/* OpAppendCopy (State::append_copy): `[x; n]` — the template is the vector's last element and
 * `n` counts it, so n - 1 deep copies of it follow; n <= 0 removes the template too. */
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
    for (uint32_t i = 1; i < multiply; i++) {
        loft_ref e = loft_vec_append(v, size);
        loft_ref a = { v.store_nr, loft_collection_rec(loft_store_of(v), v.rec, v.pos), 8 + (length - 1) * size };
        loft_copy_bytes(e, 0, a, 0, size);
        if (T[ctp].kind != LOFT_T_SCALAR) loft_copy_claims(T, ctp, a, e);
        loft_vec_finish(v);
    }
}

/* OpClear (Stores::remove_claims): the record releases what it owns — every text, vector and
 * child record slot it holds, through its struct fields and enum variant, reads empty after. */
static void loft_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    const loft_tdesc *d = &T[tp];
    switch (d->kind) {
    case LOFT_T_TEXT:
    case LOFT_T_VECTOR:
    case LOFT_T_CHILD:
        loft_set_u32(loft_store_of(r), r.rec, r.pos, 0);
        return;
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) loft_remove_claims(T, d->fields[i].type, loft_field(r, d->fields[i].pos));
        return;
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(loft_store_of(r), r.rec, r.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) loft_remove_claims(T, d->variants[e], r);
        return;
    }
    default:
        return;
    }
}

/* OpAppendVector (Stores::vector_add): every element of `o` appended to `v`, deep-copied so the
 * destination owns what each element holds.  The source's length is read once, and each source
 * element is re-read after the growth, so a self-append (`v += v`) reads the moved record. */
static void loft_vec_add(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
    int64_t n = loft_vec_len(o);
    if (n == 0 || v.store_nr == 0xFFFF || v.rec == 0) return;
    uint32_t size = T[tp].size;
    for (int64_t i = 0; i < n; i++) {
        loft_ref e = loft_vec_append(v, size);
        loft_ref a = { o.store_nr, loft_collection_rec(loft_store_of(o), o.rec, o.pos), 8 + (uint32_t) i * size };
        loft_copy_bytes(e, 0, a, 0, size);
        if (T[tp].kind != LOFT_T_SCALAR) loft_copy_claims(T, tp, a, e);
        loft_vec_finish(v);
    }
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
    loft_database(dst, T[tp].size);
    loft_copy_bytes(*dst, 0, src, 0, T[tp].size);
    loft_copy_claims(T, tp, src, *dst);
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

static double loft_get_float(loft_ref r, uint32_t fld) {
    if (r.rec == 0) return (double) NAN;
    double v;
    memcpy(&v, loft_at(loft_store_of(r), r.rec, r.pos + fld), 8);
    return v;
}

static void loft_set_float(loft_ref r, uint32_t fld, double v) {
    if (r.rec == 0) return;
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
    memcpy(loft_at(loft_store_of(r), r.rec, r.pos + fld), &v, 4);
}

static void loft_vec_push_float(loft_ref v, double x) {
    loft_ref e = loft_vec_append(v, 8);
    if (e.rec == 0) return;
    loft_set_float(e, 0, x);
    loft_vec_finish(v);
}

static void loft_vec_push_char(loft_ref v, uint32_t c) {
    loft_ref e = loft_vec_append(v, 4);
    if (e.rec == 0) return;
    memcpy(loft_at(loft_store_of(e), e.rec, e.pos), &c, 4);
    loft_vec_finish(v);
}

static void loft_vec_push_single(loft_ref v, float x) {
    loft_ref e = loft_vec_append(v, 4);
    if (e.rec == 0) return;
    loft_set_single(e, 0, x);
    loft_vec_finish(v);
}

/* Rust's `{}` for a float: the shortest digits that read back as `v` (as a float when `single`),
 * written out in full with no exponent — 1e21 is "1000000000000000000000", 1.0 is "1", -0.0 is
 * "-0".  The digits are the first `%.*e` precision that round-trips, correctly rounded. */
static void loft_push_shortest(loft_text *s, double v, int single) {
    if (isinf(v)) {
        if (v < 0) loft_push(s, "-inf", 4); else loft_push(s, "inf", 3);
        return;
    }
    char buf[48];
    int maxp = single ? 9 : 17;
    for (int p = 1; p <= maxp; p++) {
        snprintf(buf, sizeof buf, "%.*e", p - 1, v);
        double back = strtod(buf, 0);
        if (single ? (float) back == (float) v : back == v) break;
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
static void loft_format_float(loft_text *s, double val, int64_t width, int64_t prec, uint8_t token, int plus,
                              int dir, int single) {
    if (dir == 2) dir = 1;
    size_t start = s->len;
    if (isnan(val)) {
        loft_push(s, "null", 4);
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

static void loft_push_cstr(loft_text *s, const char *p) {
    loft_push(s, p, strlen(p));
}

static void loft_push_i64(loft_text *s, int64_t v) {
    char buf[24];
    int n = snprintf(buf, sizeof buf, "%lld", (long long) v);
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
    if (h != 0 && h >= st->words) {
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
