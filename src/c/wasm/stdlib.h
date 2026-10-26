/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * <stdlib.h> for a `[c] shim` compiled to wasm (loft_wasm_libc.c has the story). */
#ifndef LOFT_WASM_STDLIB_H
#define LOFT_WASM_STDLIB_H
#include <stddef.h>

#define EXIT_SUCCESS 0
#define EXIT_FAILURE 1

void *loft_w_malloc(size_t n);
void *loft_w_calloc(size_t n, size_t size);
void *loft_w_realloc(void *p, size_t n);
void loft_w_free(void *p);
void loft_w_abort(void);
int loft_w_abs(int v);
long loft_w_labs(long v);
long long loft_w_llabs(long long v);

#define malloc loft_w_malloc
#define calloc loft_w_calloc
#define realloc loft_w_realloc
#define free loft_w_free
#define abort loft_w_abort
#define abs loft_w_abs
#define labs loft_w_labs
#define llabs loft_w_llabs

#endif
