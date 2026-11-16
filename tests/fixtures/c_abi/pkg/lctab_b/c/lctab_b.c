/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
#include "lctab/lctab_backend.h"
static int64_t bb_apply(int64_t x) { return x * 5 + 11; }
static const lyr_backend bb_tab = {1, bb_apply, "lctab_b"};
int64_t bb_table(void) { return (int64_t) (intptr_t) &bb_tab; }
