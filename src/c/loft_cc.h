/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The macro layer of loft's C runtime (@PLN200 B5): the ONLY place the emitted C names a
 * compiler.  The runtime and the generator write these macros, never a raw builtin or
 * attribute.  Three compile-time modes; a mode changes speed, never a value:
 *
 *   -DLOFT_STRICT   pure C99, clean under `cc -std=c99 -pedantic -Werror`: every macro is
 *                   its exact C99 fallback.
 *   (default)       gcc / clang / MSVC extensions where detected, the fallback otherwise.
 *   -DLOFT_VERIFY   the default, and every PROOF the generator relied on is checked again:
 *                   a false one aborts naming its site (the C twin of LOFT_HOIST_VERIFY=1).
 *
 * Three kinds of site: SAFE HINTS (ignoring one changes speed only), PROOF SITES (emitted only
 * where a rewrite proved the fact), and SHIP SITES (@PLN201: the ship tier's dropped check,
 * wrapping through uint64_t, because a C signed overflow is undefined where Rust's
 * wrapping_add is not). */
#ifndef LOFT_CC_H
#define LOFT_CC_H

/* clock_gettime is POSIX, not C99; the request precedes every system header. */
#define _POSIX_C_SOURCE 200809L
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#if !defined(LOFT_STRICT) && (defined(__GNUC__) || defined(__clang__))
#define LOFT_GNU 1
#endif
#if !defined(LOFT_STRICT) && defined(_MSC_VER)
#define LOFT_MSVC 1
#endif

#if defined(LOFT_GNU) && defined(__has_builtin)
#define LOFT_HAS_BUILTIN(x) __has_builtin(x)
#elif defined(LOFT_GNU)
#define LOFT_HAS_BUILTIN(x) 1
#else
#define LOFT_HAS_BUILTIN(x) 0
#endif

/* ── Safe hints ───────────────────────────────────────────────────────────────────────── */

/* A helper every element and field access goes through: inlined, so a reference passed and
 * returned by value never travels through memory. */
#if defined(LOFT_GNU)
#define LOFT_HOT_INLINE static inline __attribute__((always_inline))
#define LOFT_COLD __attribute__((cold))
#define LOFT_LIKELY(x) __builtin_expect(!!(x), 1)
#define LOFT_UNLIKELY(x) __builtin_expect(!!(x), 0)
#elif defined(LOFT_MSVC)
#define LOFT_HOT_INLINE static __forceinline
#define LOFT_COLD
#define LOFT_LIKELY(x) (x)
#define LOFT_UNLIKELY(x) (x)
#else
#define LOFT_HOT_INLINE static inline
#define LOFT_COLD
#define LOFT_LIKELY(x) (x)
#define LOFT_UNLIKELY(x) (x)
#endif

/* Is `x` a compile-time constant?  0 where the compiler cannot say, which only costs speed. */
#if defined(LOFT_GNU)
#define LOFT_CONSTANT(x) __builtin_constant_p(x)
#else
#define LOFT_CONSTANT(x) 0
#endif

/* LOFT_ADD_OVF / LOFT_SUB_OVF / LOFT_MUL_OVF(a, b, r): *r = a op b, answering nonzero when the
 * exact result does not fit an int64_t.  The fallbacks compute through uint64_t, so no signed
 * overflow ever happens; the multiply divides only when an operand is past 2^31.5, below which
 * the product cannot overflow. */
static inline int loft_add_ovf_c99(int64_t a, int64_t b, int64_t *r) {
    uint64_t s = (uint64_t) a + (uint64_t) b;
    *r = (int64_t) s;
    return (a >= 0) == (b >= 0) && (*r >= 0) != (a >= 0);
}

static inline int loft_sub_ovf_c99(int64_t a, int64_t b, int64_t *r) {
    uint64_t s = (uint64_t) a - (uint64_t) b;
    *r = (int64_t) s;
    return (a >= 0) != (b >= 0) && (*r >= 0) != (a >= 0);
}

#define LOFT_SQRT_MAX INT64_C(3037000499) /* floor(sqrt(INT64_MAX)) */

static inline int loft_mul_ovf_c99(int64_t a, int64_t b, int64_t *r) {
    *r = (int64_t) ((uint64_t) a * (uint64_t) b);
    if (a > -LOFT_SQRT_MAX && a < LOFT_SQRT_MAX && b > -LOFT_SQRT_MAX && b < LOFT_SQRT_MAX) return 0;
    if (a == 0 || b == 0) return 0;
    if (a > 0) {
        if (b > 0) return a > INT64_MAX / b;
        return b < INT64_MIN / a;
    }
    if (b > 0) return a < INT64_MIN / b;
    return a < INT64_MAX / b;
}

#if LOFT_HAS_BUILTIN(__builtin_add_overflow)
#define LOFT_ADD_OVF(a, b, r) __builtin_add_overflow(a, b, r)
#define LOFT_SUB_OVF(a, b, r) __builtin_sub_overflow(a, b, r)
#define LOFT_MUL_OVF(a, b, r) __builtin_mul_overflow(a, b, r)
#else
#define LOFT_ADD_OVF(a, b, r) loft_add_ovf_c99(a, b, r)
#define LOFT_SUB_OVF(a, b, r) loft_sub_ovf_c99(a, b, r)
#define LOFT_MUL_OVF(a, b, r) loft_mul_ovf_c99(a, b, r)
#endif

/* ── Proof sites ──────────────────────────────────────────────────────────────────────── */

/* A fact a rewrite proved, at `site`: nothing in the default modes, checked under LOFT_VERIFY.
 * A plain operator computes through uint64_t, so a wrong proof is a wrong value and never
 * undefined behaviour. */
#if defined(LOFT_VERIFY)
LOFT_COLD static void loft_proof_failed(const char *site) {
    fprintf(stderr, "loft: a proof the C generator relied on is false at %s\n", site);
    abort();
}
#define LOFT_PROVED(cond, site) ((cond) ? (void) 0 : loft_proof_failed(site))
#else
#define LOFT_PROVED(cond, site) ((void) 0)
#endif

#define LOFT_PLAIN_ADD(a, b, site) ((int64_t) ((uint64_t) (a) + (uint64_t) (b)))
#define LOFT_PLAIN_SUB(a, b, site) ((int64_t) ((uint64_t) (a) - (uint64_t) (b)))
#define LOFT_PLAIN_MUL(a, b, site) ((int64_t) ((uint64_t) (a) * (uint64_t) (b)))

/* ── Ship sites (@PLN201) ─────────────────────────────────────────────────────────────── */

/* The ship tier's operator where the ledger licensed dropping its check: wrapping through
 * uint64_t, never a plain signed operator. */
#define LOFT_SHIP_ADD(a, b) ((int64_t) ((uint64_t) (a) + (uint64_t) (b)))
#define LOFT_SHIP_SUB(a, b) ((int64_t) ((uint64_t) (a) - (uint64_t) (b)))
#define LOFT_SHIP_MUL(a, b) ((int64_t) ((uint64_t) (a) * (uint64_t) (b)))
#define LOFT_SHIP_NEG(a) ((int64_t) (0 - (uint64_t) (a)))
#define LOFT_SHIP_SHL(a, b) ((int64_t) ((uint64_t) (a) << ((b) & 63)))

#endif
