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
uint8_t n_is_even(int64_t a) { return a % 2 == 0; }
int64_t n_sum_bytes(loft_ffi_store s, loft_ffi_ref v) {
    uint32_t n = loft_ffi_vector_len(s, v);
    const uint8_t *d = loft_ffi_vector_data(s, v);
    int64_t t = 0;
    for (uint32_t i = 0; i < n; i++) t += d[i];
    return t;
}
double n_half(double x) { return x / 2; }

/* The interpreter's bridges. */
static loft_ffi_str arg_text(const loft_ffi_value *a) { return a->payload.text; }
static void b_make_bin(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) a; (void) n; *r = loft_ffi_value_text(n_make_bin()); }
static void b_text_len(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; loft_ffi_str t = arg_text(&a[0]); *r = loft_ffi_value_i64(n_text_len(t.ptr, t.len)); }
static void b_wrap(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; loft_ffi_str t = arg_text(&a[0]); *r = loft_ffi_value_text(n_wrap(t.ptr, t.len)); }
static void b_byte(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; loft_ffi_str t = arg_text(&a[0]); *r = loft_ffi_value_i64(n_byte(t.ptr, t.len, a[1].payload.i)); }
static void b_add(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; *r = loft_ffi_value_i64(n_add(a[0].payload.i, a[1].payload.i)); }
static void b_is_even(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; *r = loft_ffi_value_bool(n_is_even(a[0].payload.i)); }
static void b_sum_bytes(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) n; *r = loft_ffi_value_i64(n_sum_bytes(s, a[0].payload.r)); }
static void b_half(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r) { (void) s; (void) n; *r = loft_ffi_value_f64(n_half(a[0].payload.f)); }

#define REG(cb, ctx, name, fn) cb((const uint8_t *) name, sizeof name - 1, (const void *) fn, ctx)
void loft_register_v1(loft_ffi_register_cb cb, void *ctx) {
    REG(cb, ctx, "n_make_bin", n_make_bin); REG(cb, ctx, "n_text_len", n_text_len); REG(cb, ctx, "n_wrap", n_wrap);
    REG(cb, ctx, "n_byte", n_byte); REG(cb, ctx, "n_add", n_add); REG(cb, ctx, "n_is_even", n_is_even);
    REG(cb, ctx, "n_sum_bytes", n_sum_bytes); REG(cb, ctx, "n_half", n_half);
}
void loft_register_bridges_v1(loft_ffi_register_cb cb, void *ctx) {
    REG(cb, ctx, "n_make_bin", b_make_bin); REG(cb, ctx, "n_text_len", b_text_len); REG(cb, ctx, "n_wrap", b_wrap);
    REG(cb, ctx, "n_byte", b_byte); REG(cb, ctx, "n_add", b_add); REG(cb, ctx, "n_is_even", b_is_even);
    REG(cb, ctx, "n_sum_bytes", b_sum_bytes); REG(cb, ctx, "n_half", b_half);
}
