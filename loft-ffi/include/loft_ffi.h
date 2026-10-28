/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * loft_ffi.h — the loft native-library ABI, for a library written in C (@PLN205).
 *
 * The C mirror of the `loft-ffi` crate (loft-ffi/src/lib.rs), which is the authority: every
 * struct here is that crate's `#[repr(C)]` type field for field.  A library built against it
 * loads where a Rust cdylib loads — `prebuilt/<host-triple>/` in its package, stamped with the
 * loft-ffi fingerprint in `.loft-build-fp` — and is called two ways:
 *
 *   - by compiled code (`--native`, `--native-c`): each `#native` symbol directly, with the
 *     TYPED signature below;
 *   - by the interpreter: through a BRIDGE per symbol (`loft_ffi_bridge_fn`), which unpacks the
 *     `loft_ffi_value` arguments and calls the typed function, both handed over at load by the two
 *     registration exports at the end of this file.
 *
 * Typed signature of a `#native` function, from its loft declaration:
 *   - first a `loft_ffi_store` when an argument or the result is a heap value (a vector, a record,
 *     a keyed collection);
 *   - `text`      → `const uint8_t *x_ptr, size_t x_len` (bytes, not NUL-terminated);
 *   - `integer`   → `int64_t`; a narrow integer (`i32`, `u8` …) → `int32_t`;
 *   - `float`     → `double`; `single` → `float`; `boolean` → `bool`;
 *   - heap value  → `loft_ffi_ref`; a vector's ref names its data record (length at byte 4, the
 *                   elements from byte 8: `loft_ffi_vector_len`, `loft_ffi_vector_data`).
 * Results: `int64_t`, `double`, `float`, `uint8_t` for a boolean (0 or 1), `loft_ffi_str` for a
 * text (the bytes stay the library's until its next call on the thread; loft copies them),
 * `loft_ffi_ref` for a heap value. */
#ifndef LOFT_FFI_H
#define LOFT_FFI_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>
#include <string.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct {
    uint16_t store_nr;
    uint32_t rec;
    uint32_t pos;
} loft_ffi_ref;

typedef struct {
    const uint8_t *ptr;
    size_t len;
} loft_ffi_str;

typedef struct {
    void *opaque;
} loft_ffi_store_ctx;

typedef void (*loft_ffi_foreign_release)(uint8_t *ptr, size_t len, size_t cap);

typedef struct {
    uint8_t *ptr;  /* the store's memory; stale after a claim, resize or foreign call */
    uint32_t size; /* in 8-byte words */
    loft_ffi_store_ctx ctx;
    uint32_t (*claim_fn)(loft_ffi_store_ctx, uint32_t words);
    void (*reload_fn)(loft_ffi_store_ctx, uint8_t **ptr, uint32_t *size);
    uint32_t (*resize_fn)(loft_ffi_store_ctx, uint32_t rec, uint32_t words);
    loft_ffi_ref (*foreign_fn)(loft_ffi_store_ctx, uint8_t *ptr, size_t len, size_t cap, uint32_t elem_size,
                           loft_ffi_foreign_release release);
} loft_ffi_store;

enum {
    LOFT_FFI_TAG_VOID = 0,
    LOFT_FFI_TAG_I64 = 1,
    LOFT_FFI_TAG_F64 = 2,
    LOFT_FFI_TAG_BOOL = 3,
    LOFT_FFI_TAG_TEXT = 4,
    LOFT_FFI_TAG_REF = 5,
};

typedef union {
    int64_t i;
    double f;
    bool b;
    loft_ffi_str text;
    loft_ffi_ref r;
    uint64_t none;
} loft_ffi_payload;

typedef struct {
    uint8_t tag;
    loft_ffi_payload payload;
} loft_ffi_value;

/* One bridge per `#native` symbol: `args[0..n]` in declaration order, the result into `*ret`
 * (`LOFT_FFI_TAG_VOID` for none).  An `integer` arrives as `LOFT_FFI_TAG_I64` whatever its width. */
typedef void (*loft_ffi_bridge_fn)(loft_ffi_store store, const loft_ffi_value *args, size_t n, loft_ffi_value *ret);

/* The registration callback loft hands to the two exports below: one call per symbol. */
typedef void (*loft_ffi_register_cb)(const uint8_t *name, size_t name_len, const void *fn, void *ctx);

/* A library defines both:
 *   void loft_register_v1(loft_ffi_register_cb cb, void *ctx);          each typed function
 *   void loft_register_bridges_v1(loft_ffi_register_cb cb, void *ctx);  each bridge
 * under the `#native` symbol's name. */

static inline uint32_t loft_ffi_vector_len(loft_ffi_store s, loft_ffi_ref v) {
    if (v.rec == 0) return 0;
    uint32_t n;
    memcpy(&n, s.ptr + (size_t) v.rec * 8 + 4, sizeof n); /* native-endian, as the store is */
    return n;
}

static inline const uint8_t *loft_ffi_vector_data(loft_ffi_store s, loft_ffi_ref v) {
    return s.ptr + (size_t) v.rec * 8 + 8;
}

static inline loft_ffi_value loft_ffi_value_i64(int64_t i) {
    loft_ffi_value v;
    v.tag = LOFT_FFI_TAG_I64;
    v.payload.i = i;
    return v;
}

static inline loft_ffi_value loft_ffi_value_f64(double f) {
    loft_ffi_value v;
    v.tag = LOFT_FFI_TAG_F64;
    v.payload.f = f;
    return v;
}

static inline loft_ffi_value loft_ffi_value_bool(bool b) {
    loft_ffi_value v;
    v.payload.none = 0;
    v.tag = LOFT_FFI_TAG_BOOL;
    v.payload.b = b;
    return v;
}

static inline loft_ffi_value loft_ffi_value_text(loft_ffi_str t) {
    loft_ffi_value v;
    v.tag = LOFT_FFI_TAG_TEXT;
    v.payload.text = t;
    return v;
}

#ifdef __cplusplus
}
#endif

#endif /* LOFT_FFI_H */
