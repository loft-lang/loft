/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The `crt` library's shim (@PLN91 S2.2): the C runtime's own format routines, reached by
 * `#c` bindings on every backend.  Each function here only MARSHALS — loft's `#c` model
 * passes plain C (a text out as a caller's byte buffer, a float through a pointer, a null as
 * a flag) — and calls one routine the C runtime already defines, so a format the loft
 * interpreter renders is the C runtime's rendering.  Each answers the rendering's length in
 * bytes; when that exceeds `cap`, nothing is written and the caller asks again with room. */
#include "../../../src/c/loft_crt.h"

static int64_t loft_crt_out(loft_text *t, char *out, int64_t cap) {
    loft_str v = loft_view(t);
    int64_t n = (int64_t) v.len;
    if (n <= cap && n > 0) {
        memcpy(out, v.p, (size_t) n);
    }
    free(t->p);
    return n;
}

int64_t loft_crt_format_int(char *out, int64_t cap, int64_t val, int64_t is_null, int64_t radix,
                            int64_t width, int64_t token, int64_t plus, int64_t note, int64_t dir) {
    loft_text t = {0};
    loft_format_long(&t, is_null ? LOFT_NULL : val, (uint8_t) radix, width, (uint8_t) token,
                     (int) plus, (int) note, (int) dir);
    return loft_crt_out(&t, out, cap);
}

int64_t loft_crt_format_float(char *out, int64_t cap, const double *val, int64_t n_val,
                              int64_t is_null, int64_t width, int64_t prec, int64_t token,
                              int64_t plus, int64_t dir, int64_t single) {
    loft_text t = {0};
    double v = (is_null || n_val < 1) ? NAN : val[0];
    loft_format_float(&t, v, width, prec, (uint8_t) token, (int) plus, (int) dir, (int) single);
    return loft_crt_out(&t, out, cap);
}

int64_t loft_crt_format_text(char *out, int64_t cap, const char *val, int64_t is_null,
                             int64_t width, int64_t dir, int64_t token) {
    loft_text t = {0};
    loft_str v = is_null ? loft_null_text() : loft_lit(val, strlen(val));
    loft_format_text(&t, v, width, (int) dir, (uint8_t) token);
    return loft_crt_out(&t, out, cap);
}
