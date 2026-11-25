/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The native part of `c_scalar_pkg`: three C functions behind `#c` declarations.  The
 * values are the oracles the package's tests assert (tests/answer.loft). */
#include <stdint.h>

int64_t ns_answer(void) {
    return 42;
}

/* -1: the value that makes an integer-width mismatch visible, as 4294967295. */
int64_t ns_sentinel(void) {
    return -1;
}

/* Writes 100, 101, … into the `n` elements loft hands over. */
void ns_fill(int64_t *out, int64_t n) {
    for (int64_t i = 0; i < n; i++) out[i] = 100 + i;
}
