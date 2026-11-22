/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * @PLN206 P5 — a shim with state of its own (`tests/hosted_unit.rs`): one counter in the
 * process, which a hosted unit's calls and the interpreter's must both reach. */
#include <stdint.h>

static int64_t hs_count = 0;

int64_t hs_bump(int64_t by) {
    hs_count += by;
    return hs_count;
}

int64_t hs_peek(void) {
    return hs_count;
}
