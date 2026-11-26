/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The build's width, for C that meets loft (@PLN217).  A loft `integer` is `loft_int` and a loft
 * `float` is `loft_float`, never a fixed width, so one source serves the 64-bit build and the
 * 32-bit one (`--width 32`, which defines LOFT_WIDTH32).  A value that needs 64 bits in every
 * build is declared `i64` / `f64` on the loft side and `loft_word` / `loft_fword` here, the
 * names the standard library gives a 64-bit machine operand (`word`, `fword`).
 *
 * Two readers: a library's `[c] shim`, which loft puts this header beside, and loft's own C
 * runtime, whose first header it is.  So it holds types and constants only — no function, and
 * nothing but <stdint.h>.
 *
 *   loft_int, loft_uint   an `integer` and its unsigned twin; LOFT_INT_NULL is its null
 *                         (the most negative value), LOFT_INT_MIN .. LOFT_INT_MAX its range,
 *                         LOFT_INT_C(v) a literal of it, LOFT_INT_BITS its size.
 *   loft_float            a `float`; NaN is its null.
 *   loft_word, loft_fword `i64` and `f64`: 64 bits in every build; LOFT_WORD_NULL is the
 *                         integer's null. */
#ifndef LOFT_WIDTH_H
#define LOFT_WIDTH_H

/* @PLN229 B1a — the runtime's STATE, spelled once for every build.  Here, in the header every
 * assembly of the runtime reaches before any state (the unit, a `[c] shim`, a component).
 *
 * Every unit carries the whole runtime (`@C169`), so in a program written as one unit each piece
 * of runtime state is `static` there, as it always was.  A program written as several units
 * (the opt-in split build) carries the runtime's functions in each of them, but its state must
 * stay ONE: the main context, the thread pool, the locks, the open trace files.  So each piece
 * of state is declared `LOFT_STATE <type> <name> LOFT_INIT(<initializer>);`:
 *
 *   one unit (no LOFT_SPLIT_*)   static <type> <name> = <initializer>;
 *   the unit that owns it        <type> <name> = <initializer>;          (LOFT_SPLIT_OWN)
 *   every other unit             extern <type> <name>;                    (LOFT_SPLIT_USE)
 *
 * An initializer holding a directive cannot be a macro argument: it is written between
 * `#ifndef LOFT_SPLIT_USE` and `#endif` instead (`loft_main_cx`). */
#if defined(LOFT_SPLIT_USE)
#define LOFT_STATE extern
#define LOFT_INIT(...)
#elif defined(LOFT_SPLIT_OWN)
#define LOFT_STATE
#define LOFT_INIT(...) = __VA_ARGS__
#else
#define LOFT_STATE static
#define LOFT_INIT(...) = __VA_ARGS__
#endif
#include <stdint.h>
typedef int64_t loft_word;
typedef double loft_fword;
#define LOFT_WORD_NULL INT64_MIN
#if defined(LOFT_WIDTH32)
typedef int32_t loft_int;
typedef uint32_t loft_uint;
typedef float loft_float;
#define LOFT_INT_BITS 32
#define LOFT_INT_NULL INT32_MIN
#define LOFT_INT_MIN (INT32_MIN + 1)
#define LOFT_INT_MAX INT32_MAX
#define LOFT_INT_C(v) INT32_C(v)
#else
typedef int64_t loft_int;
typedef uint64_t loft_uint;
typedef double loft_float;
#define LOFT_INT_BITS 64
#define LOFT_INT_NULL INT64_MIN
#define LOFT_INT_MIN (INT64_MIN + 1)
#define LOFT_INT_MAX INT64_MAX
#define LOFT_INT_C(v) INT64_C(v)
#endif
#endif
