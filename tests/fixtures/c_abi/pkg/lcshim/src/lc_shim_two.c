/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * A SECOND shim source: a shim of several files is built into a static archive and named in
 * the C backend's unit by a `link:` line, where one file is `#include`d (`src/c/mod.rs`
 * `c_libraries`).  Its `static` helper deliberately shares a name with one in lc_shim.c:
 * two sources can never share a translation unit. */
#include <stdint.h>

static int64_t lcs_divmod(int64_t a, int64_t b) { return b == 0 ? 0 : a / b; }

int64_t lcs_shim_half(int64_t a);
int64_t lcs_shim_half(int64_t a) { return lcs_divmod(a, 2); }

/* Fourteen arguments, the ones past the eighth of four widths: where a convention passes them on
 * the stack at their own size (Apple's AArch64), each must still be read from its own place.
 * The answer weights argument i by i, so two arguments swapped or one read from another's bytes
 * change it. */
int64_t lcs_shim_wide(int64_t a1, int64_t a2, int64_t a3, int64_t a4, int64_t a5, int64_t a6,
                      int64_t a7, int64_t a8, int32_t b9, int8_t b10, int64_t b11, int16_t b12,
                      int32_t b13, int32_t b14);
int64_t lcs_shim_wide(int64_t a1, int64_t a2, int64_t a3, int64_t a4, int64_t a5, int64_t a6,
                      int64_t a7, int64_t a8, int32_t b9, int8_t b10, int64_t b11, int16_t b12,
                      int32_t b13, int32_t b14) {
    return a1 + 2 * a2 + 3 * a3 + 4 * a4 + 5 * a5 + 6 * a6 + 7 * a7 + 8 * a8 + 9 * (int64_t) b9 +
           10 * (int64_t) b10 + 11 * b11 + 12 * (int64_t) b12 + 13 * (int64_t) b13 + 14 * (int64_t) b14;
}
