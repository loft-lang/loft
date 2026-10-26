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
