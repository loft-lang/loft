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

/* vector_append: room for one more element of `size`; the reference to its slot. */
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
    loft_set_u32(s, r, 4, len + 1);
    loft_ref e;
    e.store_nr = v.store_nr;
    e.rec = r;
    e.pos = 8 + len * size;
    return e;
}

static void loft_vec_push_int(loft_ref v, int64_t x) {
    loft_set_int(loft_vec_append(v, 8), 0, x);
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

/* OpCopyRecord, for a record that owns nothing past its own bytes: copy them, and free the
 * source's store when the copy says so and it is another store (state/io.rs do_copy_record).
 * A copy onto itself and a copy from or into a null reference do nothing. */
static void loft_copy_record(loft_ref src, loft_ref dst, uint32_t bytes, int free_source) {
    if (src.store_nr == dst.store_nr && src.rec == dst.rec && src.pos == dst.pos) return;
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    memmove(loft_at(loft_store_of(dst), dst.rec, dst.pos), loft_at(loft_store_of(src), src.rec, src.pos),
            bytes);
    if (free_source && src.store_nr != dst.store_nr) loft_free_ref(src);
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

/* A whole-value bind `dst = src` of a record that owns nothing past its bytes (@C86): an
 * independent copy of a present source, and the null of an absent one, releasing what `dst`
 * held.  The bytes go through a buffer: refilling `dst`'s store in place could clear the
 * source when both live in it. */
static void loft_bind_copy(loft_ref *dst, loft_ref src, uint32_t bytes) {
    if (src.store_nr == 0xFFFF || src.rec == 0) {
        loft_free_ref(*dst);
        *dst = loft_ref_null();
        return;
    }
    if (src.store_nr == dst->store_nr && src.rec == dst->rec && src.pos == dst->pos) return;
    uint8_t *tmp = (uint8_t *) malloc(bytes ? bytes : 1);
    if (!tmp) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(tmp, loft_at(loft_store_of(src), src.rec, src.pos), bytes);
    loft_database(dst, bytes);
    memcpy(loft_at(loft_store_of(*dst), dst->rec, dst->pos), tmp, bytes);
    free(tmp);
}
