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

static void loft_assert(uint8_t test, loft_str message, loft_str file, int64_t line) {
    (void) file;
    (void) line;
    if (test == 1) return;
    fputs("error: assertion failed: ", stderr);
    fwrite(message.p, 1, message.len, stderr);
    fputc('\n', stderr);
    if (loft_in_test) longjmp(loft_test_jmp, 1);
    fflush(stdout);
    exit(1);
}

/* ── Stores ────────────────────────────────────────────────────────────────────────────────
 * A port of the store model the IR addresses (src/store.rs, src/vector.rs): a store is an
 * array of 8-byte words; a record is a run of words whose first word holds its claimed size
 * (and, at byte 4 of a vector record, the vector's length); a reference (`DbRef`) names a
 * store, a record and a byte position, and a field is `rec * 8 + pos + offset`.  `rec == 0`
 * is the null reference.  The allocator is a plain bump allocator: which words a claim gets
 * is not observable, and a store's words go back together when the store is freed. */
typedef struct { uint16_t store_nr; uint32_t rec; uint32_t pos; } loft_ref;

typedef struct { uint8_t *buf; uint32_t words; uint32_t top; int live; } loft_store_t;

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
    var->store_nr = (uint16_t) nr;
    var->rec = rec;
    var->pos = 8;
}

/* OpFreeRef: the whole store goes, everything the root owned with it. */
static void loft_free_ref(loft_ref r) {
    if (r.store_nr == 0xFFFF || r.store_nr >= loft_nstores) return;
    loft_store_t *s = &loft_stores[r.store_nr];
    s->live = 0;
    s->top = 1;
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
