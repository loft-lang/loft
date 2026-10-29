/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * A loft library's native part written in C against loft_ffi.h alone (@PLN205 L0a): each
 * `#native` as its typed function (what compiled code calls) and a bridge (what the
 * interpreter calls), both registered by name. */
#include "loft_ffi.h"
#include <stdlib.h>

static uint8_t out_buf[256];

loft_ffi_str n_make_bin(void) {
    static const uint8_t b[4] = { 'A', 0, 0xff, 'B' };
    loft_ffi_str s = { b, 4 };
    return s;
}
int64_t n_text_len(const uint8_t *p, size_t n) { (void) p; return (int64_t) n; }
loft_ffi_str n_wrap(const uint8_t *p, size_t n) {
    size_t k = n < sizeof out_buf - 2 ? n : sizeof out_buf - 2;
    out_buf[0] = '[';
    memcpy(out_buf + 1, p, k);
    out_buf[k + 1] = ']';
    loft_ffi_str s = { out_buf, k + 2 };
    return s;
}
int64_t n_byte(const uint8_t *p, size_t n, int64_t i) { return i >= 0 && (size_t) i < n ? p[i] : -1; }
int64_t n_add(int64_t a, int64_t b) { return a + b; }
int32_t n_narrow(int32_t x) { return x < 0 ? INT32_MIN : x * 2; }
uint8_t n_is_even(int64_t a) { return a % 2 == 0; }
int64_t n_sum_bytes(loft_ffi_store s, loft_ffi_ref v) {
    uint32_t n = loft_ffi_vector_len(s, v);
    const uint8_t *d = loft_ffi_vector_data(s, v);
    int64_t t = 0;
    for (uint32_t i = 0; i < n; i++) t += d[i];
    return t;
}
double n_half(double x) { return x / 2; }

/* Heap answers, built in the store loft hands over: a vector in a store of its own (no heap
 * argument), a vector in its argument's store, and a record. */
loft_ffi_ref n_iota(loft_ffi_store s, int64_t n) {
    int64_t v[64];
    if (n > 64) n = 64;
    for (int64_t i = 0; i < n; i++) v[i] = n * 10 + i + 1;
    return loft_ffi_vector_from_bytes(&s, 8, (uint32_t) n, v);
}
loft_ffi_ref n_rev(loft_ffi_store s, loft_ffi_ref a) {
    uint32_t n = loft_ffi_vector_len(s, a);
    uint8_t b[256];
    if (n > 255) n = 255;
    const uint8_t *d = loft_ffi_vector_data(s, a);
    for (uint32_t i = 0; i < n; i++) b[i] = d[n - 1 - i];
    b[n] = 7;
    return loft_ffi_vector_from_bytes(&s, 1, n + 1, b);
}
loft_ffi_ref n_pair(loft_ffi_store s, int64_t a, int64_t b) {
    loft_ffi_ref r = loft_ffi_alloc_record(&s, 3);
    memcpy(s.ptr + (size_t) r.rec * 8 + r.pos, &a, 8);
    memcpy(s.ptr + (size_t) r.rec * 8 + r.pos + 8, &b, 8);
    return r;
}

/* Answers that SHARE what their argument holds (loft#2227): the argument itself, a vector field
 * of a record argument, and a fresh record that takes over its argument's text. */
loft_ffi_ref n_same(loft_ffi_store s, loft_ffi_ref a) { (void) s; return a; }
loft_ffi_ref n_bag_items(loft_ffi_store s, loft_ffi_ref b) {
    loft_ffi_ref r;
    uint32_t vrec;
    memcpy(&vrec, s.ptr + (size_t) b.rec * 8 + b.pos, sizeof vrec);
    r.store_nr = b.store_nr;
    r.rec = vrec;
    r.pos = 8;
    return r;
}
loft_ffi_ref n_named_k(loft_ffi_store s, loft_ffi_ref p, int64_t k) {
    uint32_t text_rec;
    memcpy(&text_rec, s.ptr + (size_t) p.rec * 8 + p.pos + 8, sizeof text_rec);
    loft_ffi_ref r = loft_ffi_alloc_record(&s, 3);
    memcpy(s.ptr + (size_t) r.rec * 8 + r.pos, &k, 8);
    memcpy(s.ptr + (size_t) r.rec * 8 + r.pos + 8, &text_rec, sizeof text_rec);
    return r;
}

/* The interpreter's bridges. */
static loft_ffi_str arg_text(const loft_ffi_value *a) { return a->payload.text; }
static void b_make_bin(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) a; (void) n; *r = loft_ffi_value_text(n_make_bin()); }
static void b_text_len(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; loft_ffi_str t = arg_text(&a[0]); *r = loft_ffi_value_i64(n_text_len(t.ptr, t.len)); }
static void b_wrap(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; loft_ffi_str t = arg_text(&a[0]); *r = loft_ffi_value_text(n_wrap(t.ptr, t.len)); }
static void b_byte(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; loft_ffi_str t = arg_text(&a[0]); *r = loft_ffi_value_i64(n_byte(t.ptr, t.len, a[1].payload.i)); }
static void b_add(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; *r = loft_ffi_value_i64(n_add(a[0].payload.i, a[1].payload.i)); }
static void b_is_even(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; *r = loft_ffi_value_bool(n_is_even(a[0].payload.i)); }
static void b_sum_bytes(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_i64(n_sum_bytes(s, a[0].payload.r)); }
static void b_narrow(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) {
    (void) s; (void) n;
    int32_t v = n_narrow((int32_t) a[0].payload.i);
    *r = loft_ffi_value_i64(v == INT32_MIN ? INT64_MIN : (int64_t) v); /* loft_ffi::widen_i32 */
}
static void b_half(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; *r = loft_ffi_value_f64(n_half(a[0].payload.f)); }
static void b_iota(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_ref(n_iota(s, a[0].payload.i)); }
static void b_rev(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_ref(n_rev(s, a[0].payload.r)); }
static void b_pair(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_ref(n_pair(s, a[0].payload.i, a[1].payload.i)); }

static void b_same(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_ref(n_same(s, a[0].payload.r)); }
static void b_bag_items(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_ref(n_bag_items(s, a[0].payload.r)); }
static void b_named_k(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_ref(n_named_k(s, a[0].payload.r, a[1].payload.i)); }

#define REG(cb, ctx, name, fn) cb((const uint8_t *) name, sizeof name - 1, (const void *) fn, ctx)
void loft_register_v1(loft_ffi_register_cb cb, void *ctx) {
    REG(cb, ctx, "n_make_bin", n_make_bin); REG(cb, ctx, "n_text_len", n_text_len); REG(cb, ctx, "n_wrap", n_wrap);
    REG(cb, ctx, "n_byte", n_byte); REG(cb, ctx, "n_add", n_add); REG(cb, ctx, "n_is_even", n_is_even);
    REG(cb, ctx, "n_sum_bytes", n_sum_bytes); REG(cb, ctx, "n_half", n_half); REG(cb, ctx, "n_narrow", n_narrow);
    REG(cb, ctx, "n_iota", n_iota); REG(cb, ctx, "n_rev", n_rev); REG(cb, ctx, "n_pair", n_pair);
    REG(cb, ctx, "n_same", n_same); REG(cb, ctx, "n_bag_items", n_bag_items); REG(cb, ctx, "n_named_k", n_named_k);
}
void loft_register_bridges_v1(loft_ffi_register_cb cb, void *ctx) {
    REG(cb, ctx, "n_make_bin", b_make_bin); REG(cb, ctx, "n_text_len", b_text_len); REG(cb, ctx, "n_wrap", b_wrap);
    REG(cb, ctx, "n_byte", b_byte); REG(cb, ctx, "n_add", b_add); REG(cb, ctx, "n_is_even", b_is_even);
    REG(cb, ctx, "n_sum_bytes", b_sum_bytes); REG(cb, ctx, "n_half", b_half); REG(cb, ctx, "n_narrow", b_narrow);
    REG(cb, ctx, "n_iota", b_iota); REG(cb, ctx, "n_rev", b_rev); REG(cb, ctx, "n_pair", b_pair);
    REG(cb, ctx, "n_same", b_same); REG(cb, ctx, "n_bag_items", b_bag_items); REG(cb, ctx, "n_named_k", b_named_k);
}
