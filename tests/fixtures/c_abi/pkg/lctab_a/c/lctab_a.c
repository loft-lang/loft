/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
#include "../../lctab/c/lctab_backend.h"
static int64_t ba_apply(int64_t x) { return x * 3 + 7; }
static const lyr_backend ba_tab = {1, ba_apply, "lctab_a"};
int64_t ba_table(void) { return (int64_t) (intptr_t) &ba_tab; }
