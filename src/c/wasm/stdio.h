/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * <stdio.h> for a `[c] shim` compiled to wasm: `snprintf` of the common conversions, and
 * `stderr` writes that go nowhere.  No files — a shim that opens one does not build for a
 * wasm target (loft_wasm_libc.c has the story). */
#ifndef LOFT_WASM_STDIO_H
#define LOFT_WASM_STDIO_H
#include <stddef.h>
#include <stdarg.h>

typedef struct loft_w_file FILE;
#define stderr ((FILE *) 0)
#define stdout ((FILE *) 0)
#define EOF (-1)

int loft_w_snprintf(char *out, size_t cap, const char *fmt, ...);
int loft_w_vsnprintf(char *out, size_t cap, const char *fmt, va_list ap);
int loft_w_fputs(const char *s, void *stream);
int loft_w_fprintf(void *stream, const char *fmt, ...);

#define snprintf loft_w_snprintf
#define vsnprintf loft_w_vsnprintf
#define fputs loft_w_fputs
#define fprintf loft_w_fprintf

#endif
