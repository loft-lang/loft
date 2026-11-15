/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
/* The neutral layer's C: it holds the table it was handed and passes calls on. */
#include "lctab_backend.h"
static const lyr_backend *lyr_table;
int64_t lyr_use(int64_t table) {
    const lyr_backend *t = (const lyr_backend *) (intptr_t) table;
    if (!t || t->version != 1) { return 0; }
    lyr_table = t;
    return 1;
}
int64_t lyr_apply(int64_t x) { return lyr_table ? lyr_table->apply(x) : -1; }
const char *lyr_name(void) { return lyr_table ? lyr_table->name : "none"; }
