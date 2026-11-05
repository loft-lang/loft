/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The build's width, for C that meets loft (@PLN217): a library's `[c] shim` spells a loft
 * `integer` as `loft_int` and a loft `float` as `loft_float`, never a fixed width, so one source
 * serves the 64-bit build and the 32-bit one (`--width 32`, which defines LOFT_WIDTH32).  A value
 * that needs 64 bits in every build is declared `i64` / `f64` on the loft side and `int64_t` /
 * `double` here.  loft puts this header beside every shim it compiles. */
#ifndef LOFT_WIDTH_H
#define LOFT_WIDTH_H
#include <stdint.h>
#if defined(LOFT_WIDTH32)
typedef int32_t loft_int;
typedef uint32_t loft_uint;
typedef float loft_float;
#else
typedef int64_t loft_int;
typedef uint64_t loft_uint;
typedef double loft_float;
#endif
#endif
