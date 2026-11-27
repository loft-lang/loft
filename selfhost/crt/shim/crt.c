/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The `crt` library's shim (@PLN91 S2.2): the C runtime's own format routines, reached by
 * `#c` bindings on every backend.  Each function here only MARSHALS — loft's `#c` model
 * passes plain C (a text out as a caller's byte buffer, a float through a pointer, a null as
 * a flag) — and calls one routine the C runtime already defines, so a format the loft
 * interpreter renders is the C runtime's rendering.  Each answers the rendering's length in
 * bytes; when that exceeds `cap`, nothing is written and the caller asks again with room. */
/* The store context (@PLN226): the shim's helpers take it first, as every runtime function does;
 * each exported function — a fixed `#c` ABI the interpreter calls — starts from the main
 * thread's, `loft_main_cx`. */
/* The allocator trace of the interpreted program's stores (@PLN91 1b-compare.md, the alloc
 * layer): this copy of the runtime writes the event lines `LOFT_ALLOC_TRACE` writes, to the file
 * `LOFT_CRT_ALLOC_TRACE` names, so the interpreter's own stores never mix into them. */
#define LOFT_ALLOC_TRACE 1
#define LOFT_ALLOC_TRACE_ENV "LOFT_CRT_ALLOC_TRACE"
/* The text parser (`t as vector<T>`, `json_parse`): the C runtime's own, with each store's parse
 * errors kept beside it (a side table; no store's layout changes, so the runner's own unit, built
 * without it, shares the stores unchanged). */
#define LOFT_JSON 1
#define LOFT_PARSE 1
/* A mapped file (`file_map`): a store whose bytes are the mapping's, read-only. */
#define LOFT_FOREIGN 1
/* The engine-host kernel (`lib/engine_host`): the runtime's own (`loft_engine.h`), its sockets
 * and its HTTPS client, as a C program that reaches a kernel native is built. */
#define LOFT_ENGINE 1
/* `@FR-Path-Case`: a name is compared with its directory's entries through the full lowercase
 * mapping (`É.txt` beside `é.txt` is a case clash), as a C program that reaches the file library
 * is built.  `loft_text_lower` and its table come with the compiled bodies (`loft_bodies.c`,
 * included below), which always carry the text predicates' case tables; a shim built without
 * them (`-DLOFT_CRT_NO_BODIES`) compares names by ASCII case alone. */
#ifndef LOFT_CRT_NO_BODIES
#define LOFT_PATH_FOLD 1
#endif
#include "../../../src/c/loft_crt.h"
/* The file routines (@PLN91 S6's files): a File record of the interpreted program is read and
 * written by the runtime's own `loft_get_file`, `loft_write_file`, `loft_read_file`, …, and a
 * relative path resolves against the program's directory as a C program's does. */
#include "../../../src/c/loft_io.h"
#include "../../../src/c/loft_image.h"
#include "../../../src/c/loft_paged.h"
/* A second pass over the radix walks defines their paged twins, as the C backend's unit order. */
#include "../../../src/c/loft_radix_walk.h"
#include "../../../src/c/loft_paged_load.h"
#include "../../../src/c/loft_sock.h"
#include "../../../src/c/loft_https.h"
#include "../../../src/c/loft_engine.h"

/* The text parser (`loft_parse.h`, `loft_json.h`) reads its unit's type table and asks which
 * types are a vector variable's own store (`main_vector<T>`); for the runner both come from the
 * loaded table and from a list the runner hands over (`loft_crt_main_vector`). */
static uint8_t *loft_crt_main_vectors = 0;
static uint32_t loft_crt_nmain_vectors = 0;
static int loft_is_main_vector(loft_cx *cx, uint16_t tp) {
    return tp < loft_crt_nmain_vectors && loft_crt_main_vectors[tp];
}
/* The JsonValue tree's layout (the emitter's `json_layout`): positions in the stdlib's own
 * JsonValue types, read from the loaded schema before the first `json_parse`
 * (`loft_crt_json_layout`). */
static uint32_t loft_crt_jl[11];
#define LOFT_JV_SIZE loft_crt_jl[0]
#define LOFT_JF_SIZE loft_crt_jl[1]
#define LOFT_TEXT_SIZE loft_crt_jl[2]
#define LOFT_JF_NAME loft_crt_jl[3]
#define LOFT_JF_VALUE loft_crt_jl[4]
#define LOFT_JV_BOOL_VALUE loft_crt_jl[5]
#define LOFT_JV_NUMBER_VALUE loft_crt_jl[6]
#define LOFT_JV_INT_VALUE loft_crt_jl[7]
#define LOFT_JV_STRING_VALUE loft_crt_jl[8]
#define LOFT_JV_ITEMS loft_crt_jl[9]
#define LOFT_JV_FIELDS loft_crt_jl[10]
#include "../../../src/c/loft_json.h"
#include "../../../src/c/loft_parse.h"
#include "../../../src/c/loft_log.h"

/* A text that crossed the bridge: `bytes` long (an embedded NUL kept), or null when -1. */
static loft_str loft_crt_str(loft_cx *cx, const char *val, int64_t bytes) {
    return bytes < 0 ? loft_null_text(cx) : loft_lit(cx, val, (size_t) bytes);
}

static int64_t loft_crt_out(loft_cx *cx, loft_text *t, char *out, int64_t cap) {
    loft_str v = loft_view(cx, t);
    int64_t n = (int64_t) v.len;
    if (n <= cap && n > 0) {
        memcpy(out, v.p, (size_t) n);
    }
    free(t->p);
    return n;
}

int64_t loft_crt_format_int(char *out, int64_t cap, int64_t val, int64_t is_null, int64_t radix,
                            int64_t width, int64_t token, int64_t plus, int64_t note, int64_t dir) {
    loft_cx *cx = &loft_main_cx;
    loft_text t = {0};
    loft_format_long(cx, &t, is_null ? LOFT_NULL : val, (uint8_t) radix, width, (uint8_t) token,
                     (int) plus, (int) note, (int) dir);
    return loft_crt_out(cx, &t, out, cap);
}

int64_t loft_crt_format_float(char *out, int64_t cap, const double *val, int64_t n_val,
                              int64_t is_null, int64_t width, int64_t prec, int64_t token,
                              int64_t plus, int64_t dir, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    loft_text t = {0};
    double v = (is_null || n_val < 1) ? NAN : val[0];
    loft_format_float(cx, &t, v, width, prec, (uint8_t) token, (int) plus, (int) dir, (int) single);
    return loft_crt_out(cx, &t, out, cap);
}

int64_t loft_crt_format_text(char *out, int64_t cap, const char *val, int64_t bytes,
                             int64_t width, int64_t dir, int64_t token) {
    loft_cx *cx = &loft_main_cx;
    loft_text t = {0};
    loft_str v = loft_crt_str(cx, val, bytes);
    loft_format_text(cx, &t, v, width, (int) dir, (uint8_t) token);
    return loft_crt_out(cx, &t, out, cap);
}

/* ── The store (@PLN91 S3): the interpreted program's records live in this shim's own copy of
 * the C runtime's stores.  A reference crosses `#c` as its three parts (store, rec, pos), and
 * a routine that answers one writes them into the caller's three-integer vector.  Each
 * function marshals and calls one runtime routine; none derives a layout. */

static loft_ref loft_crt_ref(loft_cx *cx, int64_t store, int64_t rec, int64_t pos) {
    loft_ref r;
    r.store_nr = (uint16_t) store;
    r.rec = (uint32_t) rec;
    r.pos = (uint32_t) pos;
    return r;
}

static void loft_crt_put_ref(loft_cx *cx, int64_t *out, int64_t n, loft_ref r) {
    if (n < 3) return;
    out[0] = r.store_nr;
    out[1] = r.rec;
    out[2] = r.pos;
}

/* The variable a routine writes: its three parts in `io`, read before and written after. */
static loft_ref loft_crt_get_ref(loft_cx *cx, const int64_t *io, int64_t n) {
    return n < 3 ? loft_ref_null(cx) : loft_crt_ref(cx, io[0], io[1], io[2]);
}

/* OpDatabase of a record type, on the variable in `io`: a store it already names is cleared and
 * made again in place, any other variable gets a fresh store; the root is a zeroed record of
 * `bytes`. */
int64_t loft_crt_database(int64_t *io, int64_t n, int64_t bytes, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_get_ref(cx, io, n);
    loft_database_typed(cx, &r, (uint32_t) bytes, (uint16_t) tp);
    loft_crt_put_ref(cx, io, n, r);
    return 0;
}

/* ── @PLN91 F-1: the interpreter's stack store (`State::stack_cur`) ────────────────────────
 * One store whose primary record holds every frame, each variable at the byte the Rust
 * interpreter keeps it at: frame byte `at` is field `at` of `(store, 1, 8)`.  The store holds
 * nothing but that record, which spans the whole store. */

/* `Store::extend_primary_to_store_end`: record 1 takes the rest of the store, so no free tail
 * is left for the allocator to hand out. */
static void loft_crt_stack_span(loft_cx *cx, loft_store_t *s) {
    loft_set_u32(cx, s, LOFT_ST_PRIMARY, 0, s->size - LOFT_ST_PRIMARY);
    s->wild = 0;
    s->free_root = 0;
    s->claimed_end = s->size;
}

/* `database_defined`: a fresh stack store of at least `bytes` of frame, pinned to its slot. */
int64_t loft_crt_stack_new(int64_t *io, int64_t n, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_ref_null(cx);
    loft_database(cx, &r, (uint32_t) bytes);
    loft_store_t *s = &cx->stores[r.store_nr];
    s->pinned = 1;
    loft_crt_stack_span(cx, s);
    loft_crt_put_ref(cx, io, n, r);
    return 0;
}

/* `State::grow_stack`: the store grown until record 1 holds `bytes` of frame. */
void loft_crt_stack_grow(int64_t store, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    uint32_t words = (uint32_t) ((8 + bytes + 7) / 8) + 1;
    if (s->size >= LOFT_ST_PRIMARY + words) return;
    loft_resize_store(cx, s, LOFT_ST_PRIMARY + words);
    loft_crt_stack_span(cx, s);
}

/* ── F-1b: a text variable in its slot, as the Rust frame holds it: a local a `loft_text` (24
 * bytes, a `String`), an argument a `loft_str` view (16 bytes, a `&str`) of a copy the call made
 * and its return releases. */
static uint8_t *loft_crt_slot_at(loft_cx *cx, int64_t store, int64_t at) {
    return cx->stores[store].buf + (size_t) LOFT_ST_PRIMARY * 8 + 8 + (size_t) at;
}

/* The interpreter's frame bytes as `State::get_stack` / `put_stack` / `get_var` / `put_var`
 * read and write them: the raw value of width `which` (0 u8, 1 i8, 2 u16, 3 i16, 4 u32, 5 i32,
 * 6 i64) at byte `at` of the stack store's frame, no field encoding; a float as its own bytes. */
int64_t loft_crt_stack_get(int64_t which, int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    const uint8_t *p = loft_crt_slot_at(cx, store, at);
    switch (which) {
    case 0: { uint8_t v; memcpy(&v, p, 1); return v; }
    case 1: { int8_t v; memcpy(&v, p, 1); return v; }
    case 2: { uint16_t v; memcpy(&v, p, 2); return v; }
    case 3: { int16_t v; memcpy(&v, p, 2); return v; }
    case 4: { uint32_t v; memcpy(&v, p, 4); return v; }
    case 5: { int32_t v; memcpy(&v, p, 4); return v; }
    default: { int64_t v; memcpy(&v, p, 8); return v; }
    }
}
void loft_crt_stack_put(int64_t which, int64_t store, int64_t at, int64_t v, int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    uint8_t *p = loft_crt_slot_at(cx, store, at);
    if (is_null) v = INT64_MIN; /* the integer null's pattern, cut to the width */
    switch (which) {
    case 0: case 1: { uint8_t w = (uint8_t) v; memcpy(p, &w, 1); break; }
    case 2: case 3: { uint16_t w = (uint16_t) v; memcpy(p, &w, 2); break; }
    case 4: case 5: { uint32_t w = (uint32_t) v; memcpy(p, &w, 4); break; }
    default: memcpy(p, &v, 8); break;
    }
}
double loft_crt_stack_get_float(int64_t store, int64_t at, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    const uint8_t *p = loft_crt_slot_at(cx, store, at);
    if (single) { float f; memcpy(&f, p, 4); return (double) f; }
    double d;
    memcpy(&d, p, 8);
    return d;
}
void loft_crt_stack_put_float(int64_t store, int64_t at, const double *v, int64_t n, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    uint8_t *p = loft_crt_slot_at(cx, store, at);
    double d = n > 0 ? v[0] : 0.0;
    if (single) { float f = (float) d; memcpy(p, &f, 4); }
    else memcpy(p, &d, 8);
}
/* `(R-StackBase)` — the frame area's address for the run's dispatch loop, as an integer a loft
 * local holds; taken again after anything that can grow the stack store. */
int64_t loft_crt_stack_base(int64_t store) {
    loft_cx *cx = &loft_main_cx;
    return (int64_t) (uintptr_t) loft_crt_slot_at(cx, store, 0);
}
/* A routine the dispatch loop calls at every step: a shipped program's link-time optimisation
 * takes it into the caller whatever the caller's size (`extern` keeps the definition for a
 * build that links the shim as a library). */
#if defined(__GNUC__)
#define LOFT_STEP_FN __attribute__((always_inline)) inline
#else
#define LOFT_STEP_FN
#endif
/* The value at byte `at` from the address `base` — held code bytes or the stack's frame area —
 * and one written there, with no test of the position: the dispatch loop tests its positions
 * once a step.  `loft_crt_peek` / `loft_crt_poke` are what the loft bodies call (the width as
 * `loft_crt_stack_get` numbers it); the twins are one load or store each. */
#define LOFT_AT(base, at) ((uint8_t *) (uintptr_t) (base) + (at))
#define LOFT_PEEK(name, ctype) \
    extern int64_t loft_twin_peek_##name(int64_t base, int64_t at); \
    LOFT_STEP_FN int64_t loft_twin_peek_##name(int64_t base, int64_t at) { \
        ctype v; \
        memcpy(&v, LOFT_AT(base, at), sizeof v); \
        return (int64_t) v; \
    }
LOFT_PEEK(u8, uint8_t)
LOFT_PEEK(i8, int8_t)
LOFT_PEEK(u16, uint16_t)
LOFT_PEEK(i16, int16_t)
LOFT_PEEK(u32, uint32_t)
LOFT_PEEK(i32, int32_t)
LOFT_PEEK(i64, int64_t)
#define LOFT_POKE(name, ctype) \
    extern void loft_twin_poke_##name(int64_t base, int64_t at, int64_t v); \
    LOFT_STEP_FN void loft_twin_poke_##name(int64_t base, int64_t at, int64_t v) { \
        ctype w = (ctype) v; \
        memcpy(LOFT_AT(base, at), &w, sizeof w); \
    }
LOFT_POKE(u8, uint8_t)
LOFT_POKE(u32, uint32_t)
LOFT_POKE(i64, int64_t)
extern double loft_twin_peek_f64(int64_t base, int64_t at);
extern void loft_twin_poke_f64(int64_t base, int64_t at, double v);
LOFT_STEP_FN double loft_twin_peek_f64(int64_t base, int64_t at) {
    double v;
    memcpy(&v, LOFT_AT(base, at), 8);
    return v;
}
LOFT_STEP_FN void loft_twin_poke_f64(int64_t base, int64_t at, double v) {
    loft_cx *cx = &loft_main_cx;
    memcpy(LOFT_AT(base, at), &v, 8);
}
extern float loft_twin_peek_f32(int64_t base, int64_t at);
extern void loft_twin_poke_f32(int64_t base, int64_t at, float v);
LOFT_STEP_FN float loft_twin_peek_f32(int64_t base, int64_t at) {
    float v;
    memcpy(&v, LOFT_AT(base, at), 4);
    return v;
}
LOFT_STEP_FN void loft_twin_poke_f32(int64_t base, int64_t at, float v) {
    loft_cx *cx = &loft_main_cx;
    memcpy(LOFT_AT(base, at), &v, 4);
}
void loft_crt_move(int64_t base, int64_t from, int64_t to, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    memmove(LOFT_AT(base, to), LOFT_AT(base, from), (size_t) n);
}
extern void loft_twin_move(int64_t base, int64_t from, int64_t to, int64_t n);
LOFT_STEP_FN void loft_twin_move(int64_t base, int64_t from, int64_t to, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    memmove(LOFT_AT(base, to), LOFT_AT(base, from), (size_t) n);
}
int64_t loft_crt_peek(int64_t which, int64_t base, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    const uint8_t *p = LOFT_AT(base, at);
    switch (which) {
    case 0: { uint8_t v; memcpy(&v, p, 1); return v; }
    case 1: { int8_t v; memcpy(&v, p, 1); return v; }
    case 2: { uint16_t v; memcpy(&v, p, 2); return v; }
    case 3: { int16_t v; memcpy(&v, p, 2); return v; }
    case 4: { uint32_t v; memcpy(&v, p, 4); return v; }
    case 5: { int32_t v; memcpy(&v, p, 4); return v; }
    default: { int64_t v; memcpy(&v, p, 8); return v; }
    }
}
double loft_crt_peek_f64(int64_t base, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    double v;
    memcpy(&v, LOFT_AT(base, at), 8);
    return v;
}
void loft_crt_poke(int64_t which, int64_t base, int64_t at, int64_t v, int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    uint8_t *p = LOFT_AT(base, at);
    if (is_null) v = INT64_MIN; /* the integer null's pattern, cut to the width */
    switch (which) {
    case 0: case 1: { uint8_t w = (uint8_t) v; memcpy(p, &w, 1); break; }
    case 2: case 3: { uint16_t w = (uint16_t) v; memcpy(p, &w, 2); break; }
    case 4: case 5: { uint32_t w = (uint32_t) v; memcpy(p, &w, 4); break; }
    default: memcpy(p, &v, 8); break;
    }
}
void loft_crt_poke_f64(int64_t base, int64_t at, const double *v, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    double d = n > 0 ? v[0] : 0.0;
    memcpy(LOFT_AT(base, at), &d, 8);
}

/* A reference in a frame slot, by address: written, copied, and the two reads that answer one
 * (`loft_crt_vec_get`, `loft_crt_get_dbref`) writing it in place; a null index reads null. */
typedef struct { uint32_t store, rec, pos; } loft_crt_slot_ref;
extern void loft_twin_poke_ref(int64_t to, int64_t store, int64_t rec, int64_t pos);
extern void loft_twin_copy_ref(int64_t from, int64_t to);
LOFT_STEP_FN void loft_twin_poke_ref(int64_t to, int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_slot_ref v = { (uint32_t) store, (uint32_t) rec, (uint32_t) pos };
    memcpy(LOFT_AT(to, 0), &v, 12);
}
LOFT_STEP_FN void loft_twin_copy_ref(int64_t from, int64_t to) {
    loft_cx *cx = &loft_main_cx;
    memmove(LOFT_AT(to, 0), LOFT_AT(from, 0), 12);
}
static int64_t loft_crt_slot_part(loft_cx *cx, loft_ref r, int64_t part) {
    return part == 0 ? (int64_t) r.store_nr : part == 1 ? (int64_t) r.rec : (int64_t) r.pos;
}
int64_t loft_crt_vec_get_part(int64_t part, int64_t store, int64_t rec, int64_t pos, int64_t size, int64_t idx) {
    loft_cx *cx = &loft_main_cx;
    return loft_crt_slot_part(cx, loft_vec_get(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size, idx), part);
}
int64_t loft_crt_get_dbref_part(int64_t part, int64_t store, int64_t rec, int64_t pos, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    return loft_crt_slot_part(cx, loft_get_dbref(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld), part);
}
void loft_twin_vec_get_to(int64_t to, int64_t store, int64_t rec, int64_t pos, int64_t size, int64_t idx) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = idx == INT64_MIN ? loft_ref_null(cx) : loft_vec_get(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size, idx);
    loft_twin_poke_ref(to, r.store_nr, r.rec, r.pos);
}
void loft_twin_get_dbref_to(int64_t to, int64_t store, int64_t rec, int64_t pos, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_get_dbref(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld);
    loft_twin_poke_ref(to, r.store_nr, r.rec, r.pos);
}

/* The text a slot holds — a local's (`arg` 0) or an argument's (1) — copied into `out` when it
 * fits: its length, or -1 for the null text. */
int64_t loft_crt_slot_text(char *out, int64_t cap, int64_t store, int64_t at, int64_t arg) {
    loft_cx *cx = &loft_main_cx;
    loft_str v;
    if (arg) {
        /* A view with no pointer is the null text (`@C171`, `Str::is_null`). */
        v = *(loft_str *) loft_crt_slot_at(cx, store, at);
    } else {
        v = loft_view(cx, (loft_text *) loft_crt_slot_at(cx, store, at));
    }
    if (loft_str_is_null(cx, v)) return -1;
    int64_t len = (int64_t) v.len;
    if (len <= cap && len > 0) memcpy(out, v.p, (size_t) len);
    return len;
}

/* A local's slot as `OpInitText` leaves it: the empty text, no buffer. */
void loft_crt_slot_text_init(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    memset(loft_crt_slot_at(cx, store, at), 0, sizeof(loft_text));
}

/* A local set to `val` (or the null text). */
void loft_crt_set_slot_text(int64_t store, int64_t at, const char *val, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_crt_slot_at(cx, store, at);
    loft_text_set(cx, t, loft_crt_str(cx, val, bytes));
}

/* `val` appended to a local (`OpAppendText`, `loft_text_concat`): a null local stays null, a null
 * `val` makes it null (`@FR-E-NullArg`). */
void loft_crt_slot_text_append(int64_t store, int64_t at, const char *val, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_text_concat(cx, (loft_text *) loft_crt_slot_at(cx, store, at), loft_crt_str(cx, val, bytes));
}

/* `OpClearText`: the local emptied, its buffer kept. */
void loft_crt_slot_text_clear(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_text_set(cx, (loft_text *) loft_crt_slot_at(cx, store, at), loft_lit(cx, "", 0));
}

/* `OpFreeText`: the local's buffer released; the slot reads as the empty text. */
void loft_crt_slot_text_free(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_text_free(cx, (loft_text *) loft_crt_slot_at(cx, store, at));
}

/* An argument's slot: a view of a copy of `val`, or the null text's view (no pointer). */
void loft_crt_set_slot_arg(int64_t store, int64_t at, const char *val, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = loft_crt_str(cx, val, bytes);
    if (loft_str_is_null(cx, v)) {
        *(loft_str *) loft_crt_slot_at(cx, store, at) = v;
        return;
    }
    char *copy = (char *) malloc(v.len + 1);
    if (!copy) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(copy, v.p, v.len);
    copy[v.len] = 0;
    loft_str *a = (loft_str *) loft_crt_slot_at(cx, store, at);
    a->p = copy;
    a->len = v.len;
}

/* The call's return: the argument's copy released. */
void loft_crt_slot_arg_free(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_str *a = (loft_str *) loft_crt_slot_at(cx, store, at);
    free((char *) a->p);
    a->p = 0;
    a->len = 0;
}

/* ── The run image's runner (@PLN91, handoff-2026-11-13-run-image.md step 3): the Rust
 * interpreter's stack image, read and written at the Rust positions.  A text temporary is a
 * 16-byte `loft_str` view, a text local a 24-byte `loft_text`, as the Rust frame holds them. */

/* `OpConstText`: a view of `len` bytes of the image's code at `from` — the bytes stay where the
 * image holds them. */
void loft_crt_stack_str_code(int64_t store, int64_t at, const char *code, int64_t from, int64_t len) {
    loft_cx *cx = &loft_main_cx;
    loft_str *v = (loft_str *) loft_crt_slot_at(cx, store, at);
    v->p = code + from;
    v->len = (size_t) len;
}

/* Bytes held for a run's text views — the code, the constant store's texts — in memory of their
 * own, which no store's growth moves; its address as an integer.  Held for the program's life. */
int64_t loft_crt_hold_bytes(const uint8_t *b, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    uint8_t *p = (uint8_t *) malloc(n > 0 ? (size_t) n : 1);
    if (p && n > 0) memcpy(p, b, (size_t) n);
    return (int64_t) (uintptr_t) p;
}

/* `bytes` of zeroed memory of its own, held for the program's life. */
int64_t loft_crt_hold_room(int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    return (int64_t) (uintptr_t) calloc(bytes > 0 ? (size_t) bytes : 1, 1);
}

/* A view of `len` held bytes at `from` (`loft_crt_hold_bytes`) in the frame slot at `at`. */
void loft_crt_stack_str_held(int64_t store, int64_t at, int64_t base, int64_t from, int64_t len) {
    loft_cx *cx = &loft_main_cx;
    loft_str *v = (loft_str *) loft_crt_slot_at(cx, store, at);
    v->p = (const char *) (uintptr_t) base + from;
    v->len = (size_t) len;
}

/* The unsigned 32-bit value at `at` of held bytes. */
int64_t loft_crt_held_u32(int64_t base, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    uint32_t v = 0;
    if (base) memcpy(&v, (const uint8_t *) (uintptr_t) base + at, 4);
    return (int64_t) v;
}

/* `OpVarText`: a view of the text local at `src`. */
void loft_crt_stack_str_of_text(int64_t store, int64_t at, int64_t src) {
    loft_cx *cx = &loft_main_cx;
    *(loft_str *) loft_crt_slot_at(cx, store, at) = loft_view(cx, (loft_text *) loft_crt_slot_at(cx, store, src));
}

/* `OpAppendText`: the view at `src` appended to the text local at `at`. */
void loft_crt_stack_text_append_str(int64_t store, int64_t at, int64_t src) {
    loft_cx *cx = &loft_main_cx;
    loft_text_concat(cx, (loft_text *) loft_crt_slot_at(cx, store, at), *(loft_str *) loft_crt_slot_at(cx, store, src));
}

/* `OpConstFloat` and its kin: `n` bytes of the image's code at `from` copied onto the stack at
 * `at` as they stand — a constant's bits, which the runner does not reinterpret. */
void loft_crt_stack_code_bytes(int64_t store, int64_t at, const char *code, int64_t from, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    memcpy(loft_crt_slot_at(cx, store, at), code + from, (size_t) n);
}

/* `OpConvTextFromNull`: the null text's view (no pointer, `@C171`) pushed at `at`. */
void loft_crt_stack_str_null(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    *(loft_str *) loft_crt_slot_at(cx, store, at) = loft_null_text(cx);
}

/* `OpGetText`: a view of the text the field at `fld` of `(s, r, p)` holds, pushed at `at` — the
 * bytes stay in the store. */
void loft_crt_stack_str_of_field(int64_t store, int64_t at, int64_t s, int64_t r, int64_t p, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    *(loft_str *) loft_crt_slot_at(cx, store, at) = loft_get_text(cx, loft_crt_ref(cx, s, r, p), (uint32_t) fld);
}

/* A `*_dest` native's write: `val` appended to the text the reference `(s, r, p)` names — a text
 * local of the stack store, or a text a store holds. */
void loft_crt_ref_text_append(int64_t s, int64_t r, int64_t p, const char *val, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    if (t) loft_text_concat(cx, t, loft_crt_str(cx, val, bytes));
}

/* A rendering appended in place to the text a format operator writes: the text local in the
 * stack store's frame slot `at` (`rec` -1, `store` the stack) or the text `(store, rec, pos)`
 * names — the runtime's own formatter writing into it, no buffer between (`OpFormatInt`,
 * `OpFormatFloat`, `OpFormatText` and their stack forms).  A float arrives as its bits. */
static loft_text *loft_crt_fmt_target(loft_cx *cx, int64_t store, int64_t rec, int64_t pos) {
    if (rec < 0) return (loft_text *) loft_crt_slot_at(cx, store, pos);
    return (loft_text *) loft_place(cx, loft_crt_ref(cx, store, rec, pos), 0);
}
void loft_crt_append_int(int64_t store, int64_t rec, int64_t pos, int64_t val, int64_t is_null,
                         int64_t radix, int64_t width, int64_t token, int64_t plus, int64_t note,
                         int64_t dir) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = loft_crt_fmt_target(cx, store, rec, pos);
    if (t) loft_format_long(cx, t, is_null ? LOFT_NULL : val, (uint8_t) radix, width, (uint8_t) token,
                            (int) plus, (int) note, (int) dir);
}
void loft_crt_append_float(int64_t store, int64_t rec, int64_t pos, int64_t bits, int64_t is_null,
                           int64_t width, int64_t prec, int64_t token, int64_t plus, int64_t dir,
                           int64_t single) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = loft_crt_fmt_target(cx, store, rec, pos);
    double v;
    memcpy(&v, &bits, 8);
    if (is_null) v = NAN;
    if (t) loft_format_float(cx, t, v, width, prec, (uint8_t) token, (int) plus, (int) dir, (int) single);
}
void loft_crt_append_text(int64_t store, int64_t rec, int64_t pos, const char *val, int64_t bytes,
                          int64_t width, int64_t dir, int64_t token) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = loft_crt_fmt_target(cx, store, rec, pos);
    if (t) loft_format_text(cx, t, loft_crt_str(cx, val, bytes), width, (int) dir, (uint8_t) token);
}

/* The null text appended to the text the reference names, which makes it null: what a Rust
 * native's destination form writes for an absent answer (`text_null::put`).  Absence is an
 * identity (`@C171`), never the one NUL byte it once was. */
void loft_crt_ref_text_append_null(int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    if (t) loft_text_concat(cx, t, loft_null_text(cx));
}

/* `OpClearStackText`: the text the reference `(s, r, p)` names emptied, its buffer kept. */
void loft_crt_ref_text_clear(int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    if (t) loft_text_set(cx, t, loft_lit(cx, "", 0));
}

/* `OpCastTextFromBool`: the view of "true", "false" or — for the null boolean (255) — the null
 * text, pushed at `at`. */
void loft_crt_stack_str_bool(int64_t store, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    *(loft_str *) loft_crt_slot_at(cx, store, at) = v == 1 ? loft_lit(cx, "true", 4)
        : v == 255 ? loft_null_text(cx) : loft_lit(cx, "false", 5);
}

/* `OpGetStackText`: a view of the text the reference `(s, r, p)` names, pushed at `at`. */
void loft_crt_stack_str_of_ref(int64_t store, int64_t at, int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    *(loft_str *) loft_crt_slot_at(cx, store, at) = t ? loft_view(cx, t) : loft_null_text(cx);
}

/* A run image's constant (`OpConstRef`): a fresh store adopting `bytes` — the store as Rust built
 * it — as its image (`loft_image_adopt`, the loader `store_load` uses); its reference written
 * into `io`.  Answers 0 when the image is refused. */
int64_t loft_crt_store_from_image(int64_t *io, int64_t n_io, const char *bytes, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_ref_null(cx);
    loft_database(cx, &r, 8);
    uint8_t *copy = (uint8_t *) malloc((size_t) n);
    if (!copy) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(copy, bytes, (size_t) n);
    if (!loft_image_adopt(cx, r, copy, (size_t) n, 0, 0, 0, "a constant", "run image")) return 0;
    loft_crt_put_ref(cx, io, n_io, r);
    return 1;
}

/* Is store `nr` live?  A freed store answers 0 (`Store::is_free`). */
int64_t loft_crt_store_live(int64_t nr) {
    loft_cx *cx = &loft_main_cx;
    return nr >= 0 && nr < cx->nstores && cx->stores[nr].live ? 1 : 0;
}

/* Does live store `nr` hold nothing but its root record (`Store::holds_only_root`)?  A store that
 * is not live answers 0. */
int64_t loft_crt_store_only_root(int64_t nr) {
    loft_cx *cx = &loft_main_cx;
    return nr >= 0 && nr < cx->nstores && cx->stores[nr].live && loft_st_holds_only_root(cx, &cx->stores[nr]) ? 1 : 0;
}

/* The type a typed mint gave store `nr`'s root (`Store::known_type`), 65535 for none. */
int64_t loft_crt_store_known(int64_t nr) {
    loft_cx *cx = &loft_main_cx;
    return nr >= 0 && nr < cx->nstores ? cx->stores[nr].known : 0xFFFF;
}

/* As `Stores::place_record_prefilled` when the host names no store: the fresh store keeps the
 * placed record's type. */
void loft_crt_set_store_known(int64_t nr, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (nr >= 0 && nr < cx->nstores) cx->stores[nr].known = (uint16_t) tp;
}

/* The run image's runner (step 3): text views on the stack as `State::get_text_sub`,
 * `text_character` and `text_walk_step` work them, through the runtime's own rules. */

/* OpGetTextSub: the view at `at` replaced by its `from..till` slice — the two raw words above
 * it (`till` i32::MIN: the one character at `from`) — snapped to character boundaries. */
void loft_crt_stack_text_sub(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_str *v = (loft_str *) loft_crt_slot_at(cx, store, at);
    int64_t from = *(int64_t *) loft_crt_slot_at(cx, store, at + 16);
    int64_t till = *(int64_t *) loft_crt_slot_at(cx, store, at + 24);
    *v = loft_text_sub(cx, *v, from, till);
}

/* OpTextCharacter: the character holding byte `idx` — the raw word above the view at `at` — of
 * that view, 0 out of range. */
int64_t loft_crt_stack_text_char(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    int64_t idx = *(int64_t *) loft_crt_slot_at(cx, store, at + 16);
    return (int64_t) loft_text_char(cx, *(loft_str *) loft_crt_slot_at(cx, store, at), idx);
}

/* OpTextWalkStep: a `for c in t` step, the frame slots addressed from the stack top `sp`: the
 * character at byte `next` of the text at `src` (a local's text, `arg` 0, or a view) read into
 * `c`, `index` set to where it starts and `next` moved past it — by one byte at least. */
void loft_crt_text_walk_step(int64_t store, int64_t sp, int64_t c, int64_t index, int64_t next, int64_t src,
                             int64_t arg) {
    loft_cx *cx = &loft_main_cx;
    int64_t *nx = (int64_t *) loft_crt_slot_at(cx, store, sp - next);
    int64_t at = *nx;
    loft_str t = arg ? *(loft_str *) loft_crt_slot_at(cx, store, sp - src)
                     : loft_view(cx, (loft_text *) loft_crt_slot_at(cx, store, sp - src));
    uint32_t ch = loft_str_is_null(cx, t) ? 0 : loft_text_char(cx, t, at);
    int64_t w = ch == 0 ? 0 : ch < 0x80 ? 1 : ch < 0x800 ? 2 : ch < 0x10000 ? 3 : 4;
    int64_t moved = at == LOFT_NULL ? LOFT_NULL : at + w;
    if (moved <= at) moved = at == LOFT_NULL ? LOFT_NULL : at + 1;
    *(int64_t *) loft_crt_slot_at(cx, store, sp - index) = at;
    *nx = moved;
    *(uint32_t *) loft_crt_slot_at(cx, store, sp - c) = ch;
}

/* The text a frame slot holds — a local's (`arg` 0) or a view (1) — as `State::local_text`. */
static loft_str loft_crt_local_text(loft_cx *cx, int64_t store, int64_t at, int64_t arg) {
    if (arg) {
        /* A view with no pointer is the null text (`@C171`, `Str::is_null`): kept as it is. */
        return *(loft_str *) loft_crt_slot_at(cx, store, at);
    }
    return loft_view(cx, (loft_text *) loft_crt_slot_at(cx, store, at));
}

/* OpTextEndJump's size: the byte length of the text at `at`, 0 for the null text. */
int64_t loft_crt_local_text_len(int64_t store, int64_t at, int64_t arg) {
    loft_cx *cx = &loft_main_cx;
    return (int64_t) loft_crt_local_text(cx, store, at, arg).len;
}

/* OpTextNullJump: is the text at `at` the null text? */
int64_t loft_crt_local_text_null(int64_t store, int64_t at, int64_t arg) {
    loft_cx *cx = &loft_main_cx;
    return loft_str_is_null(cx, loft_crt_local_text(cx, store, at, arg)) ? 1 : 0;
}

/* A text view in its frame slot at `at`, measured and compared in place — what an arm of the
 * dispatch loop answers `OpSizeText` (`which` 0: bytes), `OpLengthText` (1: characters) and
 * `OpConvBoolFromText` (2: is it a text at all) with, and `OpEqText`, `OpLtText`, `OpLeText`
 * (`which` 0, 1, 2) for the views at `a` and `b`; the character holding byte `idx` of one. */
int64_t loft_crt_view_measure(int64_t which, int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = *(loft_str *) loft_crt_slot_at(cx, store, at);
    return which == 0 ? loft_text_size(cx, v) : which == 1 ? loft_text_length(cx, v) : !loft_str_is_null(cx, v);
}
int64_t loft_crt_view_compare(int64_t which, int64_t store, int64_t a, int64_t b) {
    loft_cx *cx = &loft_main_cx;
    loft_str x = *(loft_str *) loft_crt_slot_at(cx, store, a), y = *(loft_str *) loft_crt_slot_at(cx, store, b);
    return which == 0 ? loft_str_eq(cx, x, y) : which == 1 ? loft_text_lt(cx, x, y) : loft_text_le(cx, x, y);
}
int64_t loft_crt_view_char(int64_t store, int64_t at, int64_t idx) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = *(loft_str *) loft_crt_slot_at(cx, store, at);
    return loft_str_is_null(cx, v) ? 0 : (int64_t) loft_text_char(cx, v, idx);
}
/* The text natives on the views in their slots, no text copied out: `find` (`which` 0) and
 * `rfind` (1) of the view at `val` in the one at `self`, whether that one starts (2) or ends
 * (3) with it; the null text reads as the empty one, as the natives' loft ports read it. */
int64_t loft_crt_view_search(int64_t which, int64_t store, int64_t self, int64_t val) {
    loft_cx *cx = &loft_main_cx;
    /* A null text is searched as its own bytes, the one NUL (`STRING_NULL`), as the Rust natives
     * search `Str::str()`: `t.contains("\0")` asks for a NUL byte, and is not the empty needle
     * every text holds. */
    loft_str s = *(loft_str *) loft_crt_slot_at(cx, store, self), v = *(loft_str *) loft_crt_slot_at(cx, store, val);
    return which == 0 ? loft_find(cx, s, v) : which == 1 ? loft_rfind(cx, s, v)
         : which == 2 ? (int64_t) loft_starts_with(cx, s, v) : (int64_t) loft_ends_with(cx, s, v);
}
/* `text.byte_at(i)` of the view at `at`: 0 out of range. */
int64_t loft_crt_view_byte(int64_t store, int64_t at, int64_t i) {
    loft_cx *cx = &loft_main_cx;
    loft_str s = *(loft_str *) loft_crt_slot_at(cx, store, at);
    return loft_str_is_null(cx, s) ? 0 : loft_byte_at(cx, s, i);
}
/* `OpSetText` / `OpSetTextReplace` (`refill`) of the view in the slot at `at`, and that view
 * appended to the text a reference names (`loft_crt_ref_text_append`): no copy of the text
 * is made on the way. */
void loft_crt_set_text_view(int64_t store, int64_t rec, int64_t pos, int64_t fld, int64_t stk, int64_t at,
                            int64_t refill) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = *(loft_str *) loft_crt_slot_at(cx, stk, at);
    if (refill) loft_refill_text(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld, v);
    else loft_set_text(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld, v);
}
void loft_crt_ref_text_append_view(int64_t s, int64_t r, int64_t p, int64_t stk, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    loft_str v = *(loft_str *) loft_crt_slot_at(cx, stk, at);
    if (t) loft_text_concat(cx, t, v);
}

/* A program's own `#c` binding, called as the interpreter's `c_call` calls it: the symbol looked
 * up once per native (`idx`) in what the process has loaded, each argument read at its stack
 * slot's width (`args`: `b` a byte, `c` four bytes, `i` eight; the first deepest under `top`)
 * and handed over as a 64-bit integer, the answer narrowed to the C return's width and sign
 * (`ret_bits` 0 for none).  -1 in `*ok` where the symbol is not found or the arity is past the
 * ladder; the caller stops the run then. */
#define LOFT_CRT_CCALLS 4096
static void *loft_crt_cfn[LOFT_CRT_CCALLS];
int64_t loft_crt_c_call(int64_t stk, int64_t top, const char *sym, int64_t symlen, const char *args,
                        int64_t nargs, int64_t ret_bits, int64_t ret_signed, int64_t idx, int64_t *ok,
                        int64_t okn) {
    loft_cx *cx = &loft_main_cx;
    if (okn < 1) return 0;
    ok[0] = -1;
    if (nargs < 0 || nargs > 8 || idx < 0 || idx >= LOFT_CRT_CCALLS || symlen <= 0) return 0;
    void *f = loft_crt_cfn[idx];
    if (!f) {
        /* What the process has loaded, by the runtime's own lookup (`dlsym` on POSIX, every
         * module's `GetProcAddress` on Windows); a host with no loader finds nothing. */
#if defined(LOFT_DL)
        f = loft_dl_self_sym(cx, sym);
#endif
        if (!f) return 0;
        loft_crt_cfn[idx] = f;
    }
    int64_t a[8] = { 0 };
    for (int64_t k = 0; k < nargs; k++) {
        const uint8_t *q = loft_crt_slot_at(cx, stk, top - 8 * (nargs - k));
        if (args[k] == 'b') { uint8_t v; memcpy(&v, q, 1); a[k] = v; }
        else if (args[k] == 'c') { uint32_t v; memcpy(&v, q, 4); a[k] = v; }
        else { memcpy(&a[k], q, 8); }
    }
    typedef int64_t (*f0)(void);
    typedef int64_t (*f8)(int64_t, int64_t, int64_t, int64_t, int64_t, int64_t, int64_t, int64_t);
    int64_t r;
    if (nargs == 0) r = ((f0) f)();
    else r = ((f8) f)(a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7]);
    ok[0] = 0;
    switch (ret_bits) {
    case 0: return 0;
    case 8: return ret_signed ? (int64_t) (int8_t) r : (int64_t) (uint8_t) r;
    case 16: return ret_signed ? (int64_t) (int16_t) r : (int64_t) (uint16_t) r;
    case 32: return ret_signed ? (int64_t) (int32_t) r : (int64_t) (uint32_t) r;
    default: return r;
    }
}

/* `OpCastFloatFromText` / `OpCastSingleFromText` on the view in its frame slot: the runtime's
 * own parse (`loft_float_of_text`, as the C backend casts), NaN — null — where it is not a
 * number, the null text included. */
double loft_crt_view_float(int64_t stk, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    return loft_float_of_text(cx, *(loft_str *) loft_crt_slot_at(cx, stk, at));
}
double loft_crt_view_single(int64_t stk, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    return (double) loft_single_of_text(cx, *(loft_str *) loft_crt_slot_at(cx, stk, at));
}

/* `self.replace(value, with)` over the three views in their frame slots, the answer appended
 * to the text `(s, r, p)` names — the runtime's own `loft_replace`, no text copied out first.  A
 * null view reads as the empty text, as the native's `?? ""` does. */
static loft_str loft_crt_view_or_empty(loft_cx *cx, int64_t stk, int64_t at) {
    loft_str v = *(loft_str *) loft_crt_slot_at(cx, stk, at);
    return loft_str_is_null(cx, v) ? loft_lit(cx, "", 0) : v;
}
void loft_crt_ref_text_replace_view(int64_t s, int64_t r, int64_t p, int64_t stk, int64_t self_at,
                                    int64_t val_at, int64_t with_at) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    loft_text out = loft_replace(cx, loft_crt_view_or_empty(cx, stk, self_at),
                                 loft_crt_view_or_empty(cx, stk, val_at),
                                 loft_crt_view_or_empty(cx, stk, with_at));
    if (t) loft_text_concat(cx, t, loft_view(cx, &out));
    loft_text_free(cx, &out);
}

/* `text.trim()` / `trim_start()` / `trim_end()` (`which` 0, 1, 2) on the view at `at`: the
 * view narrowed in place, as Rust's `str::trim*` answers a sub-slice of the same text. */
void loft_crt_stack_text_trim(int64_t store, int64_t at, int64_t which) {
    loft_cx *cx = &loft_main_cx;
    loft_str *v = (loft_str *) loft_crt_slot_at(cx, store, at);
    if (loft_str_is_null(cx, *v)) return;
    if (which != 2) *v = loft_trim_start_view(cx, *v);
    if (which != 1) *v = loft_trim_end_view(cx, *v);
}

/* The run image's runner (step 3): a generator frame's pieces (`State::coroutine_*`). */

/* `n` stack bytes at `at` copied into `out` (a byte vector's buffer). */
void loft_crt_stack_read_bytes(char *out, int64_t store, int64_t at, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    memcpy(out, loft_crt_slot_at(cx, store, at), (size_t) n);
}

/* `n` stack bytes at `at` zeroed: a generator's locals region at its first resume. */
void loft_crt_stack_zero(int64_t store, int64_t at, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    memset(loft_crt_slot_at(cx, store, at), 0, (size_t) n);
}

/* A created generator's text argument: the view at `at` repointed at a copy the frame owns (the
 * caller's text may be gone before the generator runs, `serialise_text_args`).  Answers the copy
 * to release with the frame, 0 for the null or the empty text, which keep their view. */
int64_t loft_crt_stack_str_own(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_str *v = (loft_str *) loft_crt_slot_at(cx, store, at);
    if (v->len == 0 || loft_str_is_null(cx, *v)) return 0;
    char *copy = (char *) malloc(v->len);
    if (!copy) return 0;
    memcpy(copy, v->p, v->len);
    v->p = copy;
    return (int64_t) (intptr_t) copy;
}

/* The copy `loft_crt_stack_str_own` made, released when its frame ends. */
void loft_crt_owned_free(int64_t copy) {
    loft_cx *cx = &loft_main_cx;
    free((void *) (intptr_t) copy);
}

/* `Stores::hold_for_frame` / `release_frame_hold` (`@FR-G-Arg-Rebind`). */
int64_t loft_crt_hold_for_frame(int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    return loft_hold_for_frame(cx, loft_crt_ref(cx, s, r, p)) ? 1 : 0;
}

void loft_crt_release_frame_hold(int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    loft_release_frame_hold(cx, loft_crt_ref(cx, s, r, p));
}

/* The allocation count as it stands (`Stores::stores_allocated`), and store `nr`'s stamp
 * (`Store::alloc_serial`), 0 for none: a call through a function value tells a store it minted
 * from one already there by them (loft#1185, `State::release_fnref_bufs`). */
int64_t loft_crt_serial_now(void) {
    loft_cx *cx = &loft_main_cx;
    return (int64_t) cx->stores_allocated;
}

int64_t loft_crt_store_serial(int64_t nr) {
    loft_cx *cx = &loft_main_cx;
    return nr >= 0 && nr < cx->nstores ? (int64_t) cx->stores[nr].serial : 0;
}

/* A text the runner keeps for the whole run (an enum variant's name, which Rust views in its type
 * table): copied once into memory of its own; the handle is that copy. */
int64_t loft_crt_intern_text(const char *val, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    char *copy = (char *) malloc(bytes > 0 ? (size_t) bytes : 1);
    if (!copy) return 0;
    if (bytes > 0) memcpy(copy, val, (size_t) bytes);
    return (int64_t) (intptr_t) copy;
}

/* A view of `len` bytes of an interned text pushed at `at`. */
void loft_crt_stack_str_interned(int64_t store, int64_t at, int64_t handle, int64_t len) {
    loft_cx *cx = &loft_main_cx;
    *(loft_str *) loft_crt_slot_at(cx, store, at) = loft_lit(cx, (const char *) (intptr_t) handle, (size_t) len);
}

/* A `par` row passed by value (`parallel::read_primitive_at` + `put_stack`): `esize` bytes of the
 * row at `(store, rec, pos)` — 1, 4, else 8 — read unsigned and written as `isize` bytes — 1, 4,
 * else 8 — at stack byte `at`, the rest of its 8-byte slot zero. */
void loft_crt_par_row(int64_t store, int64_t rec, int64_t pos, int64_t esize, int64_t stk, int64_t at,
                      int64_t isize) {
    loft_cx *cx = &loft_main_cx;
    const uint8_t *p = (const uint8_t *) loft_place(cx, loft_crt_ref(cx, store, rec, pos), 0);
    uint64_t v = 0;
    if (p) {
        if (esize == 1) v = p[0];
        else if (esize == 4) { uint32_t w; memcpy(&w, p, 4); v = w; }
        else memcpy(&v, p, 8);
    }
    uint8_t *d = loft_crt_slot_at(cx, stk, at);
    memset(d, 0, 8);
    memcpy(d, &v, isize == 1 ? 1 : isize == 4 ? 4 : 8);
}

/* A `par` row passed wide (`read_primitive_at_wide`): `n` bytes of the row copied to stack byte
 * `at`, the slot's stepped rest zero. */
void loft_crt_par_row_wide(int64_t store, int64_t rec, int64_t pos, int64_t stk, int64_t at, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    const uint8_t *p = (const uint8_t *) loft_place(cx, loft_crt_ref(cx, store, rec, pos), 0);
    uint8_t *d = loft_crt_slot_at(cx, stk, at);
    memset(d, 0, (size_t) ((n + 7) / 8 * 8));
    if (p) memcpy(d, p, (size_t) n);
}

/* A tuple row for a worker taking the tuple (`read_tuple_at_wide`): each element moved by the
 * image's plan (`parallel::tuple_row_plan`), four numbers per element — its offset in the row,
 * its stored width, its argument width and its kind: 0 copied, 1 a text record as a `Str`, 2 a
 * function number as a fn-ref whose closure is the null sentinel. */
void loft_crt_par_row_tuple(int64_t store, int64_t rec, int64_t pos, int64_t stk, int64_t at, const int64_t *plan,
                            int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ref row = loft_crt_ref(cx, store, rec, pos);
    const uint8_t *p = (const uint8_t *) loft_place(cx, row, 0);
    int64_t width = 0;
    for (int64_t k = 0; k + 3 < n; k += 4) width += plan[k + 2];
    uint8_t *d = loft_crt_slot_at(cx, stk, at);
    memset(d, 0, (size_t) ((width + 7) / 8 * 8));
    if (!p) return;
    int64_t off = 0;
    for (int64_t k = 0; k + 3 < n; k += 4) {
        int64_t in = plan[k], stored = plan[k + 1], arg = plan[k + 2], kind = plan[k + 3];
        if (kind == 1) {
            *(loft_str *) (d + off) = loft_get_text(cx, row, (uint32_t) in);
        } else if (kind == 2) {
            uint32_t fn;
            memcpy(&fn, p + in, 4);
            int64_t f64 = fn;
            memcpy(d + off, &f64, 8);
            loft_ref none = loft_ref_null(cx);
            memcpy(d + off + 8, &none, 12);
        } else {
            memcpy(d + off, p + in, (size_t) (stored < arg ? stored : arg));
        }
        off += arg;
    }
}

/* `log_info` … `log_fatal` (`sev` 0 … 3): the C runtime's logger, configured as a compiled
 * program's is (`log.conf`). */
void loft_crt_log(int64_t sev, const char *m, int64_t mlen, const char *f, int64_t flen, int64_t line) {
    loft_cx *cx = &loft_main_cx;
    loft_str ms = loft_crt_str(cx, m, mlen), fs = loft_crt_str(cx, f, flen);
    if (sev == 0) loft_log_info(cx, ms, fs, line);
    else if (sev == 1) loft_log_warn(cx, ms, fs, line);
    else if (sev == 2) loft_log_error(cx, ms, fs, line);
    else loft_log_fatal(cx, ms, fs, line);
}

/* ── The `#c_twin` routines of crt.loft's typed byte reads and writes: each loads or stores
 * its width at `at` in the `n` bytes of `b`, little-endian as the host is; outside the bytes a
 * read answers 0 and a write does nothing, as each loft body says.  A fixed-size `memcpy` is a
 * single load or store with no alignment or aliasing assumption (an operand of the code sits at
 * any byte). */
#define LOFT_TWIN_IN(n, at, w) ((at) >= 0 && (at) <= (n) - (w))
int64_t loft_twin_get_u8(const uint8_t *b, int64_t n, int64_t at) {
    uint8_t v = 0;
    if (LOFT_TWIN_IN(n, at, 1)) memcpy(&v, b + at, 1);
    return (int64_t) v;
}
void loft_twin_put_u8(uint8_t *b, int64_t n, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    uint8_t w = (uint8_t) v;
    if (LOFT_TWIN_IN(n, at, 1)) memcpy(b + at, &w, 1);
}
int64_t loft_twin_get_i8(const uint8_t *b, int64_t n, int64_t at) {
    int8_t v = 0;
    if (LOFT_TWIN_IN(n, at, 1)) memcpy(&v, b + at, 1);
    return (int64_t) v;
}
void loft_twin_put_i8(uint8_t *b, int64_t n, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    int8_t w = (int8_t) v;
    if (LOFT_TWIN_IN(n, at, 1)) memcpy(b + at, &w, 1);
}
int64_t loft_twin_get_u16(const uint8_t *b, int64_t n, int64_t at) {
    uint16_t v = 0;
    if (LOFT_TWIN_IN(n, at, 2)) memcpy(&v, b + at, 2);
    return (int64_t) v;
}
void loft_twin_put_u16(uint8_t *b, int64_t n, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    uint16_t w = (uint16_t) v;
    if (LOFT_TWIN_IN(n, at, 2)) memcpy(b + at, &w, 2);
}
int64_t loft_twin_get_i16(const uint8_t *b, int64_t n, int64_t at) {
    int16_t v = 0;
    if (LOFT_TWIN_IN(n, at, 2)) memcpy(&v, b + at, 2);
    return (int64_t) v;
}
void loft_twin_put_i16(uint8_t *b, int64_t n, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    int16_t w = (int16_t) v;
    if (LOFT_TWIN_IN(n, at, 2)) memcpy(b + at, &w, 2);
}
int64_t loft_twin_get_u32(const uint8_t *b, int64_t n, int64_t at) {
    uint32_t v = 0;
    if (LOFT_TWIN_IN(n, at, 4)) memcpy(&v, b + at, 4);
    return (int64_t) v;
}
void loft_twin_put_u32(uint8_t *b, int64_t n, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    uint32_t w = (uint32_t) v;
    if (LOFT_TWIN_IN(n, at, 4)) memcpy(b + at, &w, 4);
}
int64_t loft_twin_get_i32(const uint8_t *b, int64_t n, int64_t at) {
    int32_t v = 0;
    if (LOFT_TWIN_IN(n, at, 4)) memcpy(&v, b + at, 4);
    return (int64_t) v;
}
void loft_twin_put_i32(uint8_t *b, int64_t n, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    int32_t w = (int32_t) v;
    if (LOFT_TWIN_IN(n, at, 4)) memcpy(b + at, &w, 4);
}
int64_t loft_twin_get_i64(const uint8_t *b, int64_t n, int64_t at) {
    int64_t v = 0;
    if (LOFT_TWIN_IN(n, at, 8)) memcpy(&v, b + at, 8);
    return (int64_t) v;
}
void loft_twin_put_i64(uint8_t *b, int64_t n, int64_t at, int64_t v) {
    loft_cx *cx = &loft_main_cx;
    int64_t w = (int64_t) v;
    if (LOFT_TWIN_IN(n, at, 8)) memcpy(b + at, &w, 8);
}
void loft_twin_copy_within(uint8_t *b, int64_t n, int64_t from, int64_t to, int64_t len) {
    loft_cx *cx = &loft_main_cx;
    if (len <= 0 || from < 0 || to < 0 || from > n - len || to > n - len) return;
    memmove(b + to, b + from, (size_t) len);
}
float loft_twin_get_f32(const uint8_t *b, int64_t n, int64_t at) {
    float v = 0.0f;
    if (LOFT_TWIN_IN(n, at, 4)) memcpy(&v, b + at, 4);
    return v;
}
void loft_twin_put_f32(uint8_t *b, int64_t n, int64_t at, float v) {
    loft_cx *cx = &loft_main_cx;
    if (LOFT_TWIN_IN(n, at, 4)) memcpy(b + at, &v, 4);
}
double loft_twin_get_f64(const uint8_t *b, int64_t n, int64_t at) {
    double v = 0.0;
    if (LOFT_TWIN_IN(n, at, 8)) memcpy(&v, b + at, 8);
    return v;
}
void loft_twin_put_f64(uint8_t *b, int64_t n, int64_t at, double v) {
    loft_cx *cx = &loft_main_cx;
    if (LOFT_TWIN_IN(n, at, 8)) memcpy(b + at, &v, 8);
}

/* `expose` / `release` off the browser (`Stores::expose_value`): the value's store locked
 * read-only while the host holds it, and unlocked. */
void loft_crt_store_lock(int64_t s, int64_t r, int64_t p, int64_t lock) {
    loft_cx *cx = &loft_main_cx;
    loft_set_store_lock(cx, loft_crt_ref(cx, s, r, p), (uint8_t) lock);
}

/* `copy_result`: `n` bytes moved within the stack. */
void loft_crt_stack_move(int64_t store, int64_t from, int64_t to, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    memmove(loft_crt_slot_at(cx, store, to), loft_crt_slot_at(cx, store, from), (size_t) n);
}

/* The text a view at `at` shows, copied into `out` when it fits: its length, -1 for null. */
int64_t loft_crt_stack_str(char *out, int64_t cap, int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = *(loft_str *) loft_crt_slot_at(cx, store, at);
    if (loft_str_is_null(cx, v)) return -1;
    int64_t len = (int64_t) v.len;
    if (len <= cap && len > 0) memcpy(out, v.p, (size_t) len);
    return len;
}

/* The run's end: the stack store released (a pinned store keeps itself from `loft_free_ref`). */
void loft_crt_stack_free(int64_t store) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    s->pinned = 0;
    loft_free_ref(cx, loft_crt_ref(cx, store, LOFT_ST_PRIMARY, 8));
}

/* A store's length in 8-byte words (`Store::capacity_words`). */
int64_t loft_crt_store_words(int64_t store) {
    loft_cx *cx = &loft_main_cx;
    return store >= 0 && store < cx->nstores ? (int64_t) cx->stores[store].size : 0;
}

/* A store's first `n` words copied into `out` (a byte vector's buffer, `n * 8` bytes): the
 * words `const_code` writes for a constant's store (`loft_cwords<d>`). */
void loft_crt_store_bytes(char *out, int64_t store, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    memcpy(out, cx->stores[store].buf, (size_t) n * 8);
}

/* A constant's store built (`consts.loft`): pinned and read-only, as `Store::lock_constant` and
 * `loft_const_store` leave it. */
void loft_crt_lock_constant(int64_t store) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    s->pinned = 1;
    s->read_only = 1;
}

/* A constant's store released by the run that built it: unpinned and writable first, as the
 * stack store is. */
void loft_crt_free_constant(int64_t store) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    s->pinned = 0;
    s->read_only = 0;
    loft_free_ref(cx, loft_crt_ref(cx, store, LOFT_ST_PRIMARY, 8));
}

/* The reference a PLACE holds — a frame slot (`(stack, 1, 8 + at)`), a field, an element —
 * read and written as `OpGetStackRef` / `OpSetStackRef` do: twelve raw bytes at `(rec, pos)`,
 * no record marked and no tombstone read through (`loft_get_dbref` does both, for a reference
 * FIELD read as a value). */
int64_t loft_crt_place_ref(int64_t *out, int64_t n, int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    loft_ref x;
    x.store_nr = (uint16_t) loft_u32(cx, s, (uint32_t) rec, (uint32_t) pos);
    x.rec = loft_u32(cx, s, (uint32_t) rec, (uint32_t) pos + 4);
    x.pos = loft_u32(cx, s, (uint32_t) rec, (uint32_t) pos + 8);
    loft_crt_put_ref(cx, out, n, x);
    return 0;
}

void loft_crt_set_place_ref(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                            int64_t r_pos) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    loft_set_u32(cx, s, (uint32_t) rec, (uint32_t) pos, (uint32_t) r_store);
    loft_set_u32(cx, s, (uint32_t) rec, (uint32_t) pos + 4, (uint32_t) r_rec);
    loft_set_u32(cx, s, (uint32_t) rec, (uint32_t) pos + 8, (uint32_t) r_pos);
}

/* OpFreeRef of a variable: its store released, the variable reset to the sentinel. */
void loft_crt_free_var(int64_t *io, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_get_ref(cx, io, n);
    loft_free_var(cx, &r);
    loft_crt_put_ref(cx, io, n, r);
}

/* OpFreeRefIfDistinct of a variable: released and reset unless it names the witness's store. */
void loft_crt_free_var_distinct(int64_t *io, int64_t n, int64_t w_store, int64_t w_rec,
                                int64_t w_pos) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_get_ref(cx, io, n);
    loft_free_var_distinct(cx, &r, loft_crt_ref(cx, w_store, w_rec, w_pos));
    loft_crt_put_ref(cx, io, n, r);
}

void loft_crt_free_ref(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_free_ref(cx, loft_crt_ref(cx, store, rec, pos));
}

/* OpFreeRefIfDistinct of two values (`Stores::free_displaced`): the displaced store released
 * unless it is the witness's, absent, or protected by a call in flight. */
void loft_crt_free_displaced(int64_t d_store, int64_t d_rec, int64_t d_pos, int64_t w_store,
                             int64_t w_rec, int64_t w_pos) {
    loft_cx *cx = &loft_main_cx;
    loft_free_displaced(cx, loft_crt_ref(cx, d_store, d_rec, d_pos), loft_crt_ref(cx, w_store, w_rec, w_pos));
}

/* n_protect_store_frees / n_unprotect_store_frees: the call bracket around a reference
 * argument. */
void loft_crt_protect(int64_t store, int64_t rec, int64_t pos, int64_t on) {
    loft_cx *cx = &loft_main_cx;
    if (on) loft_protect(cx, loft_crt_ref(cx, store, rec, pos));
    else loft_unprotect(cx, loft_crt_ref(cx, store, rec, pos));
}

int64_t loft_crt_get_int(int64_t store, int64_t rec, int64_t pos, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    return loft_get_int(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld);
}

void loft_crt_set_int(int64_t store, int64_t rec, int64_t pos, int64_t fld, int64_t v,
                      int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    loft_set_int(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld, is_null ? LOFT_NULL : v);
}

/* OpGetText: the text copied into the caller's buffer; answers its length in bytes, -1 for
 * null, and writes nothing when the length exceeds `cap`. */
int64_t loft_crt_get_text(char *out, int64_t cap, int64_t store, int64_t rec, int64_t pos,
                          int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = loft_get_text(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld);
    if (loft_str_is_null(cx, v)) return -1;
    int64_t len = (int64_t) v.len;
    if (len <= cap && len > 0) memcpy(out, v.p, (size_t) len);
    return len;
}

void loft_crt_set_text(int64_t store, int64_t rec, int64_t pos, int64_t fld, const char *val,
                       int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = loft_crt_str(cx, val, bytes);
    loft_set_text(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld, v);
}

/* OpSetTextReplace: the text field refilled in place where its claim has room. */
void loft_crt_refill_text(int64_t store, int64_t rec, int64_t pos, int64_t fld, const char *val,
                          int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_str v = loft_crt_str(cx, val, bytes);
    loft_refill_text(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld, v);
}

/* The other field widths, by the operator's `which` (the loft side's `crt::W_*`), each calling
 * that operator's own runtime routine as the C generator emits it: 0 OpGetInt4/OpSetInt4,
 * 1 OpGetByte/OpSetByte, 2 the Nullable byte, 3 OpGetShort/OpSetShort, 4 the Raw short,
 * 5 OpGetCharacter/OpSetCharacter, 6 OpGetBoolean/OpSetBoolean, 7 OpGetEnum/OpSetEnum (a plain
 * enum's byte, 255 its null).  `min` is the narrow integer's
 * minimum (unused by the others).  A null crosses as `is_null`: the int64 null for the integer
 * widths, code 0 for a character, LOFT_BOOL_NULL for a boolean, 255 for an enum — the values a C
 * program's operands carry.  A read answers the int64 null for a null boolean or enum, as
 * OpConvIntFromEnum reads 255. */
int64_t loft_crt_get_w(int64_t which, int64_t store, int64_t rec, int64_t pos, int64_t fld,
                       int64_t min) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    uint32_t f = (uint32_t) fld;
    switch (which) {
    case 0: return loft_get_int4(cx, r, f);
    case 1: return loft_get_byte(cx, r, f, min);
    case 2: return loft_get_byte_nullable(cx, r, f, min);
    case 3: return loft_get_short(cx, r, f, min);
    case 4: return loft_get_short_raw(cx, r, f, min);
    case 5: return (int64_t) loft_get_char(cx, r, f);
    case 6: {
        uint8_t b = loft_get_bool(cx, r, f);
        return b == LOFT_BOOL_NULL ? LOFT_NULL : (int64_t) b;
    }
    case 7: {
        uint8_t e = loft_get_enum(cx, r, f);
        return e == 255 ? LOFT_NULL : (int64_t) e;
    }
    case 8: return loft_get_int4_raw(cx, r, f);
    case 9: return loft_get_int4_full(cx, r, f);
    default: return LOFT_NULL;
    }
}

void loft_crt_set_w(int64_t which, int64_t store, int64_t rec, int64_t pos, int64_t fld,
                    int64_t min, int64_t v, int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    uint32_t f = (uint32_t) fld;
    int64_t x = is_null ? LOFT_NULL : v;
    switch (which) {
    case 0: loft_set_int4(cx, r, f, x); break;
    case 1: loft_set_byte_v(cx, r, f, min, x); break;
    case 2: loft_set_byte_nullable(cx, r, f, min, x); break;
    case 3: loft_set_short(cx, r, f, min, x); break;
    case 4: loft_set_short_raw(cx, r, f, min, x); break;
    case 5: loft_set_char(cx, r, f, is_null ? 0u : (uint32_t) v); break;
    case 6: loft_set_byte(cx, r, f, is_null ? LOFT_BOOL_NULL : (uint8_t) v); break;
    case 7: loft_set_byte(cx, r, f, is_null ? (uint8_t) 255 : (uint8_t) v); break;
    case 8: loft_set_int4_raw(cx, r, f, x); break;
    /* A full four-byte word reads without a null code and writes raw (`NarrowIntKind::Int4Full`). */
    case 9: loft_set_int4_raw(cx, r, f, x); break;
    default: break;
    }
}

/* OpGetFloat and OpGetSingle (`single` 1): NaN is the null either way. */
double loft_crt_get_float(int64_t store, int64_t rec, int64_t pos, int64_t fld, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    return single ? (double) loft_get_single(cx, r, (uint32_t) fld) : loft_get_float(cx, r, (uint32_t) fld);
}

/* OpSetFloat and OpSetSingle: the value is `val[0]`, a float argument being a vector of one. */
void loft_crt_set_float(int64_t store, int64_t rec, int64_t pos, int64_t fld, const double *val,
                        int64_t n, int64_t is_null, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    double v = is_null || n < 1 ? (double) NAN : val[0];
    if (single) loft_set_single(cx, r, (uint32_t) fld, (float) v);
    else loft_set_float(cx, r, (uint32_t) fld, v);
}

/* `set_float` and `vec_push_float` with the value itself: NaN is the null float. */
void loft_twin_set_float(int64_t store, int64_t rec, int64_t pos, int64_t fld, double v, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_set_float(store, rec, pos, fld, &v, 1, 0, single);
}
/* OpGetField: the reference to an embedded record, `loft_field` — the position moved past the
 * field's offset.  Answers the new position; the store and record are the host's. */
int64_t loft_crt_field_pos(int64_t store, int64_t rec, int64_t pos, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    return (int64_t) loft_field(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld).pos;
}

/* ── Vectors (@PLN91 S3, the second slice): a vector is the reference to the field that holds
 * it; an element is a reference into the same store.  Each routine calls the operator's own
 * runtime routine. */

/* OpLengthVector. */
int64_t loft_crt_vec_len(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_vec_len(cx, loft_crt_ref(cx, store, rec, pos));
}

/* OpGetVector and OpGetVectorNullable: the element at `idx` (negative from the end), or the null
 * reference past either end, written into `out`. */
int64_t loft_crt_vec_get(int64_t *out, int64_t n, int64_t store, int64_t rec, int64_t pos,
                         int64_t size, int64_t idx) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, n, loft_vec_get(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size, idx));
    return 0;
}

/* The integer field `fld` of element `idx` (`size` bytes each) of a vector, read and written
 * in one step: null where there is no such element, and no write to one. */
int64_t loft_crt_vec_elem_int(int64_t store, int64_t rec, int64_t pos, int64_t size, int64_t idx, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    loft_ref e = loft_vec_get(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size, idx);
    return e.rec == 0 ? LOFT_NULL : loft_get_int(cx, e, (uint32_t) fld);
}
void loft_crt_vec_elem_set_int(int64_t store, int64_t rec, int64_t pos, int64_t size, int64_t idx, int64_t fld,
                               int64_t v, int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    loft_ref e = loft_vec_get(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size, idx);
    if (e.rec != 0) loft_set_int(cx, e, (uint32_t) fld, is_null ? LOFT_NULL : v);
}

/* OpPreAllocVector: room for `count` more elements of `size` bytes. */
void loft_crt_vec_prealloc(int64_t store, int64_t rec, int64_t pos, int64_t count, int64_t size) {
    loft_cx *cx = &loft_main_cx;
    loft_vec_prealloc(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) count, (uint32_t) size);
}

/* The pushes, by `which` (the loft side's `crt::P_*`): 0 OpPushInt, 1 OpPushInt4, 2 OpPushCharacter,
 * 3 OpPushByte (biased by `min`), 4 OpPushEnum, 5 OpPushBoolean.  A null crosses as `is_null`:
 * the int64 null, code 0, or 255 for an enum or a boolean, as a C program's operand carries it. */
void loft_crt_vec_push(int64_t which, int64_t store, int64_t rec, int64_t pos, int64_t min,
                       int64_t v, int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    int64_t x = is_null ? LOFT_NULL : v;
    switch (which) {
    case 0: loft_vec_push_int(cx, r, x); break;
    case 1: loft_vec_push_int4(cx, r, x); break;
    case 2: loft_vec_push_char(cx, r, is_null ? 0u : (uint32_t) v); break;
    case 3: loft_vec_push_byte(cx, r, loft_byte_raw(cx, min, x)); break;
    case 4:
    case 5: loft_vec_push_byte(cx, r, is_null ? (uint8_t) 255 : (uint8_t) v); break;
    default: break;
    }
}

/* OpPushFloat and OpPushSingle (`single` 1): the value is `val[0]`. */
void loft_crt_vec_push_float(int64_t store, int64_t rec, int64_t pos, const double *val,
                             int64_t n, int64_t is_null, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    double v = is_null || n < 1 ? (double) NAN : val[0];
    if (single) loft_vec_push_single(cx, r, (float) v);
    else loft_vec_push_float(cx, r, v);
}

void loft_twin_vec_push_float(int64_t store, int64_t rec, int64_t pos, double v, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_vec_push_float(store, rec, pos, &v, 1, 0, single);
}

/* OpNewRecord on a vector: a new element of `size` bytes at its end, written into `out`. */
int64_t loft_crt_vec_append(int64_t *out, int64_t n, int64_t store, int64_t rec, int64_t pos,
                            int64_t size) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, n, loft_vec_append(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size));
    return 0;
}

/* The bytes a fresh element holds before its literal runs: the type's default image. */
void loft_crt_fill(int64_t store, int64_t rec, int64_t pos, const uint8_t *image, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_fill(cx, loft_crt_ref(cx, store, rec, pos), image, (uint32_t) n);
}

/* OpFinishRecord on a vector: the appended element counts. */
void loft_crt_vec_finish(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_vec_finish(cx, loft_crt_ref(cx, store, rec, pos));
}

/* How many stores are live: what a probe reads to see a free took effect. */
int64_t loft_crt_live_stores(void) {
    loft_cx *cx = &loft_main_cx;
    int64_t n = 0;
    for (uint32_t i = 0; i < cx->nstores; i++) n += cx->stores[i].live ? 1 : 0;
    return n;
}

/* P2: one part of a reference after the trip through `loft_ref`, to show none is cut. */
int64_t loft_crt_ref_part(int64_t store, int64_t rec, int64_t pos, int64_t which) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    return which == 0 ? (int64_t) r.store_nr : which == 1 ? (int64_t) r.rec : (int64_t) r.pos;
}

/* ── The type table (@PLN91 S3, question 2): the rows the interpreter derives in loft from the
 * image's schema, copied into the runtime's own descriptor structs.  Marshalling only: each
 * number lands where `copy_table` would have written it, and no value is computed here.
 *   row:  tp, kind, size, elem, n_fields, owns_heap, linked, n_variants, n_keys,
 *         then per field: pos, type, view, hidden, group,
 *         then each variant's type (a struct enum's, 0xFFFF for a unit variant),
 *         then per key part: type, pos, start (a keyed collection's, `DbType.keys`)
 * A struct enum's `n` is its variant count, as `copy_table` writes it.
 * Answers how many types were loaded, or -1 for rows that do not parse. */
static loft_tdesc *loft_crt_types = 0;
/* Counts the loads: what a holder of a verdict on the table (the compiled bodies) keys it by. */
static uint32_t loft_crt_types_gen = 0;
static uint32_t loft_crt_ntypes = 0;

static void loft_crt_drop_types(loft_cx *cx) {
    for (uint32_t i = 0; i < loft_crt_ntypes; i++) {
        free((void *) loft_crt_types[i].fields);
        free((void *) loft_crt_types[i].variants);
        free((void *) loft_crt_types[i].keys);
    }
    free(loft_crt_types);
    loft_crt_types = 0;
    loft_crt_ntypes = 0;
}

int64_t loft_crt_load_types(const int64_t *rows, int64_t n, int64_t ntypes) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_drop_types(cx);
    loft_crt_types_gen++;
    if (ntypes < 1 || ntypes > 0xFFFF) return -1;
    loft_crt_types = (loft_tdesc *) calloc((size_t) ntypes, sizeof(loft_tdesc));
    if (!loft_crt_types) return -1;
    loft_crt_ntypes = (uint32_t) ntypes;
    int64_t at = 0, loaded = 0;
    while (at + 9 <= n) {
        int64_t tp = rows[at], nf = rows[at + 4], nv = rows[at + 7], nk = rows[at + 8];
        if (tp < 0 || tp >= ntypes || nf < 0 || nv < 0 || nk < 0 || nk > 255
            || at + 9 + nf * 5 + nv + nk * 3 > n)
            return -1;
        loft_tdesc *d = &loft_crt_types[tp];
        d->kind = (uint8_t) rows[at + 1];
        d->size = (uint16_t) rows[at + 2];
        d->elem = (uint16_t) rows[at + 3];
        d->n = (uint16_t) nf;
        d->owns_heap = (uint8_t) rows[at + 5];
        d->linked = (uint8_t) rows[at + 6];
        if (nf > 0) {
            loft_fld *f = (loft_fld *) calloc((size_t) nf, sizeof(loft_fld));
            if (!f) return -1;
            for (int64_t i = 0; i < nf; i++) {
                const int64_t *r = &rows[at + 9 + i * 5];
                f[i].pos = (uint16_t) r[0];
                f[i].type = (uint16_t) r[1];
                f[i].view = (uint8_t) r[2];
                f[i].hidden = (uint8_t) r[3];
                f[i].group = (uint16_t) r[4];
            }
            d->fields = f;
        }
        if (nv > 0) {
            uint16_t *v = (uint16_t *) calloc((size_t) nv, sizeof(uint16_t));
            if (!v) return -1;
            for (int64_t i = 0; i < nv; i++) v[i] = (uint16_t) rows[at + 9 + nf * 5 + i];
            d->variants = v;
            d->n = (uint16_t) nv;
        }
        if (nk > 0) {
            loft_key *k = (loft_key *) calloc((size_t) nk, sizeof(loft_key));
            if (!k) return -1;
            for (int64_t i = 0; i < nk; i++) {
                const int64_t *r = &rows[at + 9 + nf * 5 + nv + i * 3];
                k[i].type = (int8_t) r[0];
                k[i].pos = (uint16_t) r[1];
                k[i].start = (int32_t) r[2];
            }
            d->keys = k;
            d->nk = (uint8_t) nk;
        }
        at += 9 + nf * 5 + nv + nk * 3;
        loaded++;
    }
    return at == n ? loaded : -1;
}

/* OpClear without its pool reset (as `LOFT_NO_POOL_RESET` runs it, values unchanged by
 * (R-Switch)): the claims the record owns released, by the table loaded above, so the store
 * can take a new value in place. */
/* OpDatabase of a vector wrapper (`main_vector<T>`, `Stores::mint_at_place`): a live buffer
 * that is a place stays where it is with its collection emptied. */
int64_t loft_crt_database_wrapper(int64_t *io, int64_t n, int64_t bytes, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_get_ref(cx, io, n);
    loft_database_wrapper(cx, (const struct loft_tdesc_s *) loft_crt_types, &r, (uint32_t) bytes, (uint16_t) tp);
    loft_crt_put_ref(cx, io, n, r);
    return 0;
}

/* OpClear with `@FR-R-Reuse`'s reset flag (`Stores::reset_pool_root`): a pooled buffer at its
 * store's root made again and prefilled with its type's defaults; answers 0 for any other. */
int64_t loft_crt_reset_pool_root(int64_t store, int64_t rec, int64_t pos, int64_t tp, int64_t size,
                                 const uint8_t *image, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    return loft_reset_pool_root(cx, loft_crt_ref(cx, store, rec, pos), (uint16_t) tp, (uint32_t) size, image,
                                (uint32_t) n);
}

void loft_crt_clear(int64_t store, int64_t rec, int64_t pos, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_remove_claims(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos));
}

/* OpPlaceRecord (Stores::place_record_prefilled): a `tp` record of `size` bytes claimed inside
 * the host's store and prefilled with `image`, or in a fresh store when there is no host; written
 * into `out`.  `foreign`: the host's root is a vector of another element type (@FR-H-RootExtent). */
int64_t loft_crt_place_record(int64_t *out, int64_t n, int64_t h_store, int64_t h_rec, int64_t h_pos, int64_t tp,
                              int64_t size, const uint8_t *image, int64_t in, int64_t foreign) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_place_record(cx, loft_crt_ref(cx, h_store, h_rec, h_pos), (uint16_t) tp, (uint32_t) size, image,
                                   (uint32_t) in, (int) foreign);
    loft_crt_put_ref(cx, out, n, r);
    return 0;
}

/* The placement family (`@FR-R-Place`), each through the runtime's own routine and the loaded
 * type table; a call before the table is loaded does nothing. */
#define LOFT_CRT_TP_OK(tp) (loft_crt_types && (tp) >= 0 && (uint32_t) (tp) < loft_crt_ntypes)

/* `OpFinishRecord` in a member of a linked group (`@FR-Col-Group`, the C backend's
 * `group_finish`): the record enters every member — `m` holds `(position, kind, type)` per member,
 * its keys the loaded table's for that type — and a record a keyed member displaces leaves them
 * all, released when no vector member holds it (`held`). */
void loft_crt_group_finish(int64_t elem, int64_t held, int64_t nullable, int64_t ps, int64_t pr, int64_t pp,
                           int64_t rs, int64_t rr, int64_t rp, const int64_t *m, int64_t nm) {
    loft_cx *cx = &loft_main_cx;
    int n = (int) (nm / 3);
    if (!loft_crt_types || n <= 0 || n > 64) return;
    loft_member rows[64];
    for (int i = 0; i < n; i++) {
        int64_t tp = m[3 * i + 2];
        rows[i].pos = (uint32_t) m[3 * i];
        rows[i].kind = (int) m[3 * i + 1];
        rows[i].keys = LOFT_CRT_TP_OK(tp) ? loft_crt_types[tp].keys : 0;
        rows[i].nk = LOFT_CRT_TP_OK(tp) ? loft_crt_types[tp].nk : 0;
    }
    loft_group_finish(cx, loft_crt_types, (uint16_t) elem, (int) held, (int) nullable, loft_crt_ref(cx, ps, pr, pp),
                      loft_crt_ref(cx, rs, rr, rp), rows, n);
}

/* OpFillKeyed (`Stores::fill_keyed_from_vector`): every record of the vector `src` filed into
 * the keyed collection at `c`, of type `tp` with elements of type `elem`.  `m` holds the
 * collection's linked group as `loft_crt_group_finish` takes one — empty outside a group — and
 * `p` is the struct that holds it.  `kind` as the lowering numbers it (`loft_crt_set_keyed`).
 * `keyed_src`: the source is a keyed collection of the same type, not a vector. */
void loft_crt_fill_keyed(int64_t elem, int64_t tp, int64_t cs, int64_t cr, int64_t cp, int64_t ss, int64_t sr,
                         int64_t sp, int64_t kind, int64_t linked, int64_t free_source, int64_t held,
                         int64_t nullable, int64_t ps, int64_t pr, int64_t pp, int64_t keyed_src, const int64_t *m,
                         int64_t nm) {
    loft_cx *cx = &loft_main_cx;
    int k = kind == 1 ? LOFT_KEYED_SORTED : kind == 2 ? LOFT_KEYED_ORDERED : kind == 3 ? LOFT_KEYED_INDEX
          : kind == 4 ? LOFT_KEYED_SPATIAL : kind == 5 ? LOFT_KEYED_TRIE : LOFT_KEYED_HASH;
    int n = (int) (nm / 3);
    loft_member rows[64];
    loft_group g;
    if (!LOFT_CRT_TP_OK(tp) || !LOFT_CRT_TP_OK(elem) || n > 64) return;
    for (int i = 0; i < n; i++) {
        int64_t mt = m[3 * i + 2];
        rows[i].pos = (uint32_t) m[3 * i];
        rows[i].kind = (int) m[3 * i + 1];
        rows[i].keys = LOFT_CRT_TP_OK(mt) ? loft_crt_types[mt].keys : 0;
        rows[i].nk = LOFT_CRT_TP_OK(mt) ? loft_crt_types[mt].nk : 0;
    }
    g.parent = loft_crt_ref(cx, ps, pr, pp);
    g.m = rows;
    g.n = n;
    g.held = (int) held;
    g.nullable = (int) nullable;
    loft_fill_keyed(cx, loft_crt_types, (uint16_t) elem, loft_crt_ref(cx, cs, cr, cp), loft_crt_ref(cx, ss, sr, sp), k,
                    loft_crt_types[tp].keys, loft_crt_types[tp].nk, (int) linked, (int) free_source, n > 0 ? &g : 0,
                    (int) keyed_src);
}

/* OpSortVector (`Stores::sort_vector`): the primitive vector sorted in place, its elements of
 * `size` bytes compared as floats, texts, unsigned or signed integers. */
void loft_crt_vec_sort(int64_t store, int64_t rec, int64_t pos, int64_t size, int64_t is_float, int64_t is_text,
                       int64_t is_unsigned) {
    loft_cx *cx = &loft_main_cx;
    loft_vec_sort(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size, (int) is_float, (int) is_text, (int) is_unsigned);
}

/* OpInsertVector: a new element of `size` bytes at `index` — a linked element a record of its
 * own, its slot the record's id — its reference in `out`. */
int64_t loft_crt_vec_insert(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos, int64_t size,
                            int64_t index, int64_t linked) {
    loft_cx *cx = &loft_main_cx;
    loft_ref v = loft_crt_ref(cx, store, rec, pos);
    loft_crt_put_ref(cx, out, no, linked ? loft_vec_insert_linked(cx, v, (uint32_t) size, index)
                                     : loft_vec_insert(cx, v, (uint32_t) size, index));
    return 0;
}

/* OpChildRecInPlace (`Stores::child_rec_in_place`): the child record the field names, made in
 * place with the type's defaults when it names none; its reference in `out`. */
int64_t loft_crt_child_rec_in_place(int64_t *out, int64_t no, int64_t tp, int64_t store, int64_t rec, int64_t pos,
                                    const uint8_t *image, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return 0;
    loft_crt_put_ref(cx, out, no, loft_child_rec_in_place(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos),
                                                      image, (uint32_t) n));
    return 0;
}

/* OpChildRec (`Stores::new_child_rec`): a new child record of type `tp` for the field, with the
 * type's defaults; its reference in `out`. */
int64_t loft_crt_new_child_rec(int64_t *out, int64_t no, int64_t tp, int64_t store, int64_t rec, int64_t pos,
                               const uint8_t *image, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return 0;
    loft_crt_put_ref(cx, out, no, loft_new_child_rec(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos), image,
                                                 (uint32_t) n));
    return 0;
}

/* OpCopyClosure (`Stores::copy_closure_record`): a closure record of type `tp` copied, its
 * owned capture slots (`owned`, their positions) given records of their own; in `out`. */
int64_t loft_crt_copy_closure(int64_t *out, int64_t no, int64_t tp, int64_t store, int64_t rec, int64_t pos,
                              const int64_t *owned, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    uint16_t slots[64];
    int64_t i;
    if (!LOFT_CRT_TP_OK(tp) || n > 64) return 0;
    for (i = 0; i < n; i++) slots[i] = (uint16_t) owned[i];
    loft_crt_put_ref(cx, out, no, loft_copy_closure(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos),
                                                n ? slots : 0, (uint32_t) n));
    return 0;
}

/* OpDemoteRec (`Stores::demote_rec`): the record's field `fld` of type `tp` demoted to a child
 * record. */
void loft_crt_demote_rec(int64_t tp, int64_t store, int64_t rec, int64_t pos, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_demote_rec(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld);
}

/* OpStoreBuf / OpStoreBufOnly (`Stores::store_buf`, `store_buf_only`): a return buffer in the
 * host's store, or the buffer as its own; its reference in `out`. */
int64_t loft_crt_store_buf(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos, int64_t only) {
    loft_cx *cx = &loft_main_cx;
    loft_ref h = loft_crt_ref(cx, store, rec, pos);
    loft_crt_put_ref(cx, out, no, only ? loft_store_buf_only(cx, h) : loft_store_buf(cx, h));
    return 0;
}

/* `store_buf`'s answer written at the address `to` (`OpStoreBuf`'s arm). */
extern void loft_twin_store_buf_to(int64_t to, int64_t store, int64_t rec, int64_t pos, int64_t only);
LOFT_STEP_FN void loft_twin_store_buf_to(int64_t to, int64_t store, int64_t rec, int64_t pos, int64_t only) {
    loft_cx *cx = &loft_main_cx;
    loft_ref h = loft_crt_ref(cx, store, rec, pos);
    loft_ref b = only ? loft_store_buf_only(cx, h) : loft_store_buf(cx, h);
    loft_twin_poke_ref(to, b.store_nr, b.rec, b.pos);
}

/* OpAdoptChildRec (`Stores::adopt_child_rec`): the field's child record adopted from the value. */
void loft_crt_adopt_child_rec(int64_t tp, int64_t fs, int64_t fr, int64_t fp, int64_t vs, int64_t vr, int64_t vp) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_adopt_child_rec(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, fs, fr, fp), loft_crt_ref(cx, vs, vr, vp));
}

/* OpReleaseRecIfDistinct (`Stores::release_rec_if_distinct`): the record released unless the
 * answer is it. */
void loft_crt_release_rec_if_distinct(int64_t tp, int64_t rs, int64_t rr, int64_t rp, int64_t as, int64_t ar,
                                      int64_t ap) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_release_rec_if_distinct(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, rs, rr, rp), loft_crt_ref(cx, as, ar, ap));
}

/* OpAdoptVector (`Stores::vector_adopt`): the vector takes the other's elements as its own. */
void loft_crt_vec_adopt(int64_t tp, int64_t vs, int64_t vr, int64_t vp, int64_t os, int64_t orec, int64_t op) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_vec_adopt(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, vs, vr, vp), loft_crt_ref(cx, os, orec, op));
}

/* OpEvacuate (`Stores::evacuate`): the holder's field `fld` moved out to a record of its own at
 * `home`, placed as `OpPlaceRecord` places one. */
void loft_crt_evacuate(int64_t tp, int64_t hs, int64_t hr, int64_t hp, int64_t fld, int64_t ms, int64_t mr,
                       int64_t mp, int64_t size, const uint8_t *image, int64_t n, int64_t foreign) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_evacuate(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, hs, hr, hp), (uint32_t) fld, loft_crt_ref(cx, ms, mr, mp),
                  (uint32_t) size, image, (uint32_t) n, (int) foreign);
}

/* OpGetDir (`Stores::get_dir`): the directory's entries appended to the vector of `File`
 * records of `size` bytes; whether the path is a directory that was read. */
int64_t loft_crt_get_dir(const char *path, int64_t bytes, int64_t store, int64_t rec, int64_t pos, int64_t size) {
    loft_cx *cx = &loft_main_cx;
    return loft_get_dir(cx, loft_crt_str(cx, path, bytes), loft_crt_ref(cx, store, rec, pos), (uint32_t) size);
}

/* `Stores::reference_removed` (`@FR-H-Removed`): does a reference to record `rec` of store
 * `store` name a record that was removed?  Such a reference reads as null. */
/* `Stores::drop_leave` (`@FR-H-DropOnce`): the root record of `store`, a `tp` released by its
 * `OpDrop`, is marked released. */
int64_t loft_crt_drop_leave(int64_t store, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_ref_null_reported(cx);
    r.store_nr = (uint16_t) store;
    r.rec = LOFT_ST_PRIMARY;
    r.pos = 8;
    loft_drop_leave(cx, r, (uint16_t) tp);
    return 0;
}

/* `Store::released_root`: was the root record of `store` released by its `OpDrop`? */
int64_t loft_crt_drop_released(int64_t store) {
    loft_cx *cx = &loft_main_cx;
    return store >= 0 && (uint64_t) store < cx->nstores && cx->stores[store].live
        && cx->stores[store].released_root;
}

int64_t loft_crt_ref_removed(int64_t store, int64_t rec) {
    loft_cx *cx = &loft_main_cx;
    return rec != 0 && store >= 0 && (uint64_t) store < cx->nstores
        && loft_st_is_removed(cx, &cx->stores[store], (uint32_t) rec);
}

/* `DbRef::NULL_REPORTED`: a null whose absence is accounted for; in `out`. */
int64_t loft_crt_null_reported(int64_t *out, int64_t no) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_ref_null_reported(cx));
    return 0;
}

/* OpSetChildRec (`Stores::set_child_rec`): the child record the field names set from `src` as a
 * `tp` record, the source freed after when `free_source`. */
void loft_crt_set_child_rec(int64_t tp, int64_t fs, int64_t fr, int64_t fp, int64_t ss, int64_t sr, int64_t sp,
                            int64_t free_source) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_set_child_rec(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, fs, fr, fp), loft_crt_ref(cx, ss, sr, sp), (int) free_source);
}

/* The run image's persisted-type sidecars (`RunSidecar`), made once a run: each the C backend's
 * `loft_sidecar{tp}`, its texts copied into memory of its own. */
static loft_sidecar *loft_crt_sidecars = 0;
static int64_t loft_crt_nsidecars = 0;

static const char *loft_crt_cstr(loft_cx *cx, const char *p, int64_t n) {
    char *c = (char *) malloc((size_t) n + 1);
    if (!c) { fputs("loft: out of memory\n", stderr); abort(); }
    if (n > 0) memcpy(c, p, (size_t) n);
    c[n] = 0;
    return c;
}

/* One sidecar added; its index. */
int64_t loft_crt_sidecar_add(int64_t tp, int64_t hash, int64_t walks, const char *before, int64_t blen,
                             const char *after, int64_t alen, const char *dump, int64_t dlen, const char *oldest,
                             int64_t olen, const char *bridge, int64_t brlen) {
    loft_cx *cx = &loft_main_cx;
    loft_sidecar *t = (loft_sidecar *) realloc(loft_crt_sidecars, (size_t) (loft_crt_nsidecars + 1) * sizeof *t);
    if (!t) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_crt_sidecars = t;
    loft_sidecar *c = &t[loft_crt_nsidecars];
    memset(c, 0, sizeof *c);
    c->tp = (uint16_t) tp;
    c->before_slot = loft_crt_cstr(cx, before, blen);
    c->after_slot = loft_crt_cstr(cx, after, alen);
    c->hash = (uint64_t) hash;
    c->dump = loft_crt_cstr(cx, dump, dlen);
    c->oldest = loft_crt_cstr(cx, oldest, olen);
    c->bridge = loft_crt_cstr(cx, bridge, brlen);
    c->types = walks ? (const loft_tdesc *) loft_crt_types : 0;
    return loft_crt_nsidecars++;
}

/* `store_persist_copy(r, path)` (`loft_image_write`) with sidecar `sc`. */
int64_t loft_crt_image_write(int64_t s, int64_t r, int64_t p, const char *path, int64_t plen, int64_t sc) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types || sc < 0 || sc >= loft_crt_nsidecars) return 0;
    return loft_image_write(cx, (const loft_tdesc *) loft_crt_types, loft_crt_ntypes, loft_crt_ref(cx, s, r, p),
                            loft_crt_str(cx, path, plen), &loft_crt_sidecars[sc]) ? 1 : 0;
}

/* `store_load(r, path)` / `store_load_untrusted` (`loft_image_load`) with sidecar `sc`. */
int64_t loft_crt_image_load(int64_t s, int64_t r, int64_t p, const char *path, int64_t plen, int64_t sc,
                            int64_t untrusted) {
    loft_cx *cx = &loft_main_cx;
    if (sc < 0 || sc >= loft_crt_nsidecars) return 0;
    return loft_image_load(cx, loft_crt_ref(cx, s, r, p), loft_crt_str(cx, path, plen), &loft_crt_sidecars[sc], (int) untrusted)
               ? 1 : 0;
}

/* `store_load_key(r, path, key)` / `_text` (`loft_pg_load_keys`, the C backend's `paged_load`):
 * the one record under the key read from the image into the collection `r`'s store holds, with
 * sidecar `sc` and the plan the image carries — the collection `coll`, its element `content`,
 * its `kind`, the refusal (empty for none).  Whether a record was read. */
int64_t loft_crt_pg_load_key(int64_t s, int64_t r, int64_t p, const char *path, int64_t plen, int64_t sc,
                             const char *refusal, int64_t rlen, int64_t content, int64_t coll, int64_t kind,
                             int64_t key, const char *tkey, int64_t tlen, int64_t is_text) {
    loft_cx *cx = &loft_main_cx;
    if (sc < 0 || sc >= loft_crt_nsidecars || !LOFT_CRT_TP_OK(coll)) return 0;
    const char *why = rlen > 0 ? loft_crt_cstr(cx, refusal, rlen) : 0;
    loft_kv kv = is_text ? loft_kv_text(cx, loft_crt_str(cx, tkey, tlen)) : loft_kv_int(cx, key);
    int64_t n = loft_pg_load_keys(cx, (const loft_tdesc *) loft_crt_types, loft_crt_ref(cx, s, r, p), loft_crt_str(cx, path, plen),
                                  &loft_crt_sidecars[sc], why, (uint16_t) (content < 0 ? 0 : content),
                                  loft_crt_types[coll].keys, loft_crt_types[coll].nk, (int) kind, &kv, 1);
    return n > 0 ? 1 : 0;
}

/* `store_load_keys(r, path, keys)` (`loft_pg_load_key_list`): every key of the vector `(ls, lr, lp)`
 * read; how many records were. */
int64_t loft_crt_pg_load_keys(int64_t s, int64_t r, int64_t p, const char *path, int64_t plen, int64_t sc,
                              const char *refusal, int64_t rlen, int64_t content, int64_t coll, int64_t kind,
                              int64_t ls, int64_t lr, int64_t lp, int64_t is_text) {
    loft_cx *cx = &loft_main_cx;
    if (sc < 0 || sc >= loft_crt_nsidecars || !LOFT_CRT_TP_OK(coll)) return 0;
    const char *why = rlen > 0 ? loft_crt_cstr(cx, refusal, rlen) : 0;
    return loft_pg_load_key_list(cx, (const loft_tdesc *) loft_crt_types, loft_crt_ref(cx, s, r, p), loft_crt_str(cx, path, plen),
                                 &loft_crt_sidecars[sc], why, (uint16_t) (content < 0 ? 0 : content),
                                 loft_crt_types[coll].keys, loft_crt_types[coll].nk, (int) kind, loft_crt_ref(cx, ls, lr, lp),
                                 (int) is_text);
}

/* `store_load_prefix(r, path, pre, limit)` (`loft_pg_load_prefix`) on a trie: how many records were
 * read. */
int64_t loft_crt_pg_load_prefix(int64_t s, int64_t r, int64_t p, const char *path, int64_t plen, int64_t sc,
                                const char *refusal, int64_t rlen, int64_t content, int64_t coll, const char *pre,
                                int64_t prelen, int64_t limit) {
    loft_cx *cx = &loft_main_cx;
    if (sc < 0 || sc >= loft_crt_nsidecars || !LOFT_CRT_TP_OK(coll)) return 0;
    const char *why = rlen > 0 ? loft_crt_cstr(cx, refusal, rlen) : 0;
    return loft_pg_load_prefix(cx, (const loft_tdesc *) loft_crt_types, loft_crt_ref(cx, s, r, p), loft_crt_str(cx, path, plen),
                               &loft_crt_sidecars[sc], why, (uint16_t) (content < 0 ? 0 : content),
                               loft_crt_types[coll].keys, loft_crt_types[coll].nk, loft_crt_str(cx, pre, prelen), limit);
}

/* `store_load_box(r, path, from, till, limit)` (`loft_pg_load_box`) on a spatial index: the
 * records inside the box read; how many were.  `name` the collection's type name. */
int64_t loft_crt_pg_load_box(int64_t s, int64_t r, int64_t p, const char *path, int64_t plen, int64_t sc,
                             const char *refusal, int64_t rlen, int64_t content, int64_t coll, int64_t fs, int64_t fr,
                             int64_t fp, int64_t ts, int64_t tr, int64_t tp, int64_t limit, const char *name,
                             int64_t nlen) {
    loft_cx *cx = &loft_main_cx;
    if (sc < 0 || sc >= loft_crt_nsidecars || !LOFT_CRT_TP_OK(coll)) return 0;
    const char *why = rlen > 0 ? loft_crt_cstr(cx, refusal, rlen) : 0;
    return loft_pg_load_box(cx, (const loft_tdesc *) loft_crt_types, loft_crt_ref(cx, s, r, p), loft_crt_str(cx, path, plen),
                            &loft_crt_sidecars[sc], why, (uint16_t) (content < 0 ? 0 : content),
                            loft_crt_types[coll].keys, loft_crt_types[coll].nk, loft_crt_ref(cx, fs, fr, fp),
                            loft_crt_ref(cx, ts, tr, tp), limit, loft_crt_cstr(cx, name, nlen));
}

/* `store_load_range(r, path, lo, hi)` (`loft_pg_load_range`) on a sorted collection, the root type
 * `tp` itself: the records in the key range read; how many were. */
int64_t loft_crt_pg_load_range(int64_t s, int64_t r, int64_t p, const char *path, int64_t plen, int64_t sc,
                               const char *refusal, int64_t rlen, int64_t content, int64_t tp, int64_t lo,
                               int64_t hi) {
    loft_cx *cx = &loft_main_cx;
    if (sc < 0 || sc >= loft_crt_nsidecars || !LOFT_CRT_TP_OK(tp)) return 0;
    const char *why = rlen > 0 ? loft_crt_cstr(cx, refusal, rlen) : 0;
    return loft_pg_load_range(cx, (const loft_tdesc *) loft_crt_types, loft_crt_ref(cx, s, r, p), loft_crt_str(cx, path, plen),
                              &loft_crt_sidecars[sc], why, (uint16_t) (content < 0 ? 0 : content),
                              loft_crt_types[tp].keys, lo, hi);
}

/* `store_verify(r)` (`loft_store_verify`). */
int64_t loft_crt_store_verify(int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types) return 0;
    return loft_store_verify(cx, (const loft_tdesc *) loft_crt_types, loft_crt_ntypes, loft_crt_ref(cx, s, r, p));
}

/* The parser's type table (`loft_parse.h` declares it): the loaded one. */
static const loft_tdesc *loft_parse_types(loft_cx *cx) {
    return (const loft_tdesc *) loft_crt_types;
}

/* ── The parse table (`loft_ptypes`, the C backend's `parse_table`), built at run time from the
 * rows the runner derives from the schema (selfhost/src/ptypes.loft). ─────────────────────── */
static loft_ptype *loft_crt_ptypes = 0;
static uint32_t loft_crt_nptypes = 0;

/* A type is a vector variable's own store (`main_vector<T>`). */
void loft_crt_main_vector(int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (tp < 0) return;
    if ((uint32_t) tp >= loft_crt_nmain_vectors) {
        uint32_t n = (uint32_t) tp + 64;
        uint8_t *t = (uint8_t *) realloc(loft_crt_main_vectors, n);
        if (!t) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(t + loft_crt_nmain_vectors, 0, n - loft_crt_nmain_vectors);
        loft_crt_main_vectors = t;
        loft_crt_nmain_vectors = n;
    }
    loft_crt_main_vectors[tp] = 1;
}

/* Row `tp` of the parse table, the table grown to hold it; zeroed when new. */
static loft_ptype *loft_crt_prow(loft_cx *cx, int64_t tp) {
    if (tp < 0 || tp > 65535) return 0;
    if ((uint32_t) tp >= loft_crt_nptypes) {
        uint32_t n = (uint32_t) tp + 64;
        loft_ptype *t = (loft_ptype *) realloc(loft_crt_ptypes, n * sizeof *t);
        if (!t) { fputs("loft: out of memory\n", stderr); abort(); }
        memset(t + loft_crt_nptypes, 0, (n - loft_crt_nptypes) * sizeof *t);
        loft_crt_ptypes = t;
        loft_crt_nptypes = n;
    }
    return &loft_crt_ptypes[tp];
}

/* Is row `tp` written already? */
int64_t loft_crt_ptype_known(int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    return tp >= 0 && (uint32_t) tp < loft_crt_nptypes && loft_crt_ptypes[tp].name != 0 ? 1 : 0;
}

/* A row's scalar parts; a keyed collection's key parts are the loaded table's. */
void loft_crt_ptype_set(int64_t tp, int64_t kind, int64_t size, int64_t elem, int64_t min, int64_t nullable,
                        int64_t n, int64_t strict, int64_t zero, int64_t variant, int64_t keyed, int64_t linked,
                        int64_t some, int64_t payload_tp, int64_t payload_pos, int64_t refs, const char *name,
                        int64_t nlen) {
    loft_cx *cx = &loft_main_cx;
    loft_ptype *r = loft_crt_prow(cx, tp);
    if (!r) return;
    r->kind = (uint8_t) kind;
    r->size = (uint16_t) size;
    r->elem = (uint16_t) elem;
    r->min = (int32_t) min;
    r->nullable = (uint8_t) nullable;
    r->n = (uint16_t) n;
    r->strict_enum = (uint8_t) strict;
    r->zero_default = (uint8_t) zero;
    r->variant = (uint8_t) variant;
    r->name = loft_crt_cstr(cx, name, nlen);
    r->keyed = (uint8_t) keyed;
    if (kind == LOFT_P_KEYED && LOFT_CRT_TP_OK(tp)) {
        r->keys = loft_crt_types[tp].keys;
        r->nk = (uint8_t) loft_crt_types[tp].nk;
    }
    r->linked = (uint8_t) linked;
    r->some = (uint8_t) some;
    r->payload_tp = (uint16_t) payload_tp;
    r->payload_pos = (uint16_t) payload_pos;
    r->refs = (uint8_t) refs;
}

/* A record row's prefill image. */
void loft_crt_ptype_prefill(int64_t tp, const uint8_t *img, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ptype *r = loft_crt_prow(cx, tp);
    if (!r || n <= 0) return;
    uint8_t *c = (uint8_t *) malloc((size_t) n);
    if (!c) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(c, img, (size_t) n);
    r->prefill = c;
}

/* An enum row's variants: their record types; the names follow, one `loft_crt_ptype_vname` each. */
void loft_crt_ptype_enum(int64_t tp, const int64_t *vt, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ptype *r = loft_crt_prow(cx, tp);
    if (!r) return;
    uint16_t *types = (uint16_t *) calloc((size_t) (n > 0 ? n : 1), sizeof *types);
    /* The names by value from 1, as the emitter's `loft_enum{tp}`: slot 0 the empty name. */
    const char **vn = (const char **) calloc((size_t) n + 1, sizeof *vn);
    if (!types || !vn) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i <= n; i++) vn[i] = "";
    for (int64_t i = 0; i < n; i++) types[i] = (uint16_t) vt[i];
    r->vtypes = types;
    r->vnames = vn;
}

/* The name of variant `k` (from 1) of enum row `tp`. */
void loft_crt_ptype_vname(int64_t tp, int64_t k, const char *name, int64_t nlen) {
    loft_cx *cx = &loft_main_cx;
    if (tp < 0 || (uint32_t) tp >= loft_crt_nptypes || !loft_crt_ptypes[tp].vnames) return;
    if (k < 1 || k > loft_crt_ptypes[tp].n) return;
    ((const char **) loft_crt_ptypes[tp].vnames)[k] = loft_crt_cstr(cx, name, nlen);
}

/* A record row's field table of `n` fields. */
void loft_crt_ptype_fields(int64_t tp, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ptype *r = loft_crt_prow(cx, tp);
    if (!r) return;
    loft_pfield *f = (loft_pfield *) calloc((size_t) (n > 0 ? n : 1), sizeof *f);
    if (!f) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) f[i].name = "";
    r->fields = f;
}

/* Field `i` of record row `tp`, as the C backend's `parse_field` writes it. */
void loft_crt_pfield_set(int64_t tp, int64_t i, const char *name, int64_t nlen, int64_t content, int64_t pos,
                         int64_t nullable, int64_t skip, int64_t type_tag, int64_t dkind, int64_t di, const double *dnum,
                         int64_t ndnum, const char *ds, int64_t dslen, int64_t target) {
    loft_cx *cx = &loft_main_cx;
    if (tp < 0 || (uint32_t) tp >= loft_crt_nptypes || !loft_crt_ptypes[tp].fields) return;
    loft_pfield *f = (loft_pfield *) &loft_crt_ptypes[tp].fields[i];
    f->name = loft_crt_cstr(cx, name, nlen);
    f->content = (uint16_t) content;
    f->pos = (uint16_t) pos;
    f->nullable = (uint8_t) nullable;
    f->skip = (uint8_t) skip;
    f->type_tag = (uint8_t) type_tag;
    f->dkind = (uint8_t) dkind;
    f->di = di;
    double dv = ndnum > 0 ? dnum[0] : 0.0;
    memcpy(&f->dbits, &dv, sizeof f->dbits); /* a NUMBER default's bits */
    f->ds = loft_crt_cstr(cx, ds, dslen);
    f->dslen = (size_t) dslen;
    f->target = (uint16_t) target;
}

/* The linked group field `i` of record row `tp` is a member of: `(position, kind, type)` per
 * member, the field itself first. */
void loft_crt_pfield_group(int64_t tp, int64_t i, const int64_t *m, int64_t nm, int64_t gelem, int64_t gheld,
                           int64_t gnull) {
    loft_cx *cx = &loft_main_cx;
    if (tp < 0 || (uint32_t) tp >= loft_crt_nptypes || !loft_crt_ptypes[tp].fields) return;
    int n = (int) (nm / 3);
    loft_member *rows = (loft_member *) calloc((size_t) (n > 0 ? n : 1), sizeof *rows);
    if (!rows) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int k = 0; k < n; k++) {
        int64_t mtp = m[3 * k + 2];
        rows[k].pos = (uint32_t) m[3 * k];
        rows[k].kind = (int) m[3 * k + 1];
        rows[k].keys = LOFT_CRT_TP_OK(mtp) ? loft_crt_types[mtp].keys : 0;
        rows[k].nk = LOFT_CRT_TP_OK(mtp) ? loft_crt_types[mtp].nk : 0;
    }
    loft_pfield *f = (loft_pfield *) &loft_crt_ptypes[tp].fields[i];
    f->grp = rows;
    f->ngrp = (uint8_t) n;
    f->gelem = (uint16_t) gelem;
    f->gheld = (uint8_t) gheld;
    f->gnull = (uint8_t) gnull;
}

/* The JsonValue layout, in `loft_crt_jl`'s order. */
void loft_crt_json_layout(const int64_t *v, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    for (int64_t i = 0; i < n && i < 11; i++) loft_crt_jl[i] = (uint32_t) v[i];
}

/* `json_parse(raw)`: the tree in a store of its own. */
int64_t loft_crt_json_parse(int64_t *out, int64_t no, const char *raw, int64_t len) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_json_parse_value(cx, loft_crt_str(cx, raw, len)));
    return 0;
}

/* `json_errors()`: the last parse's errors, `|`-separated. */
int64_t loft_crt_json_errors(char *out, int64_t cap) {
    loft_cx *cx = &loft_main_cx;
    loft_text t = loft_json_errors(cx);
    return loft_crt_out(cx, &t, out, cap);
}

/* `parse_errors_of(r)`: the errors of the parse that built `r`'s store, as a fresh vector. */
int64_t loft_crt_parse_errors_of(int64_t *out, int64_t no, int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_parse_errors_of(cx, loft_crt_ref(cx, s, r, p)));
    return 0;
}

/* The JsonValue natives (`default/06_json.loft`), by `which`, as the C backend maps each to its
 * runtime routine (src/c/mod.rs): a reference answer goes to `ro`, a float one to `f[0]`, a text
 * one to `out` (its length answered), any other is the answer.  `v` is the receiver or the one
 * reference argument, `t` the text argument, `i` an integer or boolean one (`struct_from_jsonvalue`'s
 * type), `f[0]` a float one. */
int64_t loft_crt_jv(int64_t which, int64_t *ro, int64_t nro, int64_t s, int64_t r, int64_t p, const char *t, int64_t tl,
                    int64_t i, double *f, int64_t nf, char *out, int64_t cap) {
    loft_cx *cx = &loft_main_cx;
    loft_ref v = loft_crt_ref(cx, s, r, p);
    loft_str ts = loft_crt_str(cx, t, tl);
    loft_text o = { 0, 0, 0 };
    switch (which) {
    case 0: loft_crt_put_ref(cx, ro, nro, loft_json_null(cx)); return 0;
    case 1: loft_crt_put_ref(cx, ro, nro, loft_json_bool(cx, (uint8_t) i)); return 0;
    case 2: loft_crt_put_ref(cx, ro, nro, loft_json_number(cx, nf > 0 ? f[0] : 0.0)); return 0;
    case 3: loft_crt_put_ref(cx, ro, nro, loft_json_string(cx, ts)); return 0;
    case 4: loft_crt_put_ref(cx, ro, nro, loft_json_array(cx, v)); return 0;
    case 5: loft_crt_put_ref(cx, ro, nro, loft_json_object(cx, v)); return 0;
    case 6: loft_crt_put_ref(cx, ro, nro, loft_jv_fields(cx, v)); return 0;
    case 7: loft_crt_put_ref(cx, ro, nro, loft_jv_field(cx, v, ts)); return 0;
    case 8: loft_crt_put_ref(cx, ro, nro, loft_jv_item(cx, v, i)); return 0;
    case 9: loft_crt_put_ref(cx, ro, nro, loft_jv_keys(cx, v)); return 0;
    case 10: loft_crt_put_ref(cx, ro, nro, loft_struct_from_jsonvalue(cx, loft_crt_ptypes, v, (uint16_t) i)); return 0;
    case 20: return loft_jv_len(cx, v);
    case 21: return loft_jv_as_long(cx, v);
    case 22: return loft_jv_as_bool(cx, v);
    case 23: return loft_jv_has_field(cx, v, ts);
    case 30: if (nf > 0) f[0] = loft_jv_as_number(cx, v); return 0;
    case 40: o = loft_jv_as_text(cx, v); break;
    case 41: o = loft_jv_kind(cx, v); break;
    case 42: o = loft_jv_to_json(cx, v); break;
    case 43: o = loft_jv_to_json_pretty(cx, v); break;
    default: return 0;
    }
    return loft_crt_out(cx, &o, out, cap);
}

/* `deliver(tag, value)` off the browser: the value's bytes, walked over the parse table's rows
 * as the C backend's per-type walkers (`src/c/deliver.rs`, `deliver_body`) walk them — a base
 * value or a narrow integer as its bytes, a record's data fields in order, a vector's or an
 * array's elements — or why the layout cannot carry them. */
static const char *loft_crt_dlv_walk(loft_cx *cx, loft_text *o, uint16_t tp, loft_ref r) {
    static char why[256];
    if (tp <= 6) {
        if (tp == 2 || tp == 6) loft_dlv_raw(cx, o, r, 4);
        else if (tp == 4) loft_dlv_raw(cx, o, r, 1);
        else if (tp == 5) loft_dlv_text(cx, o, r);
        else loft_dlv_raw(cx, o, r, 8);
        return 0;
    }
    if ((uint32_t) tp >= loft_crt_nptypes || !loft_crt_ptypes[tp].name) {
        snprintf(why, sizeof why, "type %u not in descriptor", tp);
        return why;
    }
    const loft_ptype *P = &loft_crt_ptypes[tp];
    switch (P->kind) {
    case LOFT_P_BYTE:
    case LOFT_P_ENUM: loft_dlv_raw(cx, o, r, 1); return 0;
    case LOFT_P_SHORT: loft_dlv_short(cx, o, r, 0, 1); return 0;
    case LOFT_P_SHORTRAW: loft_dlv_short(cx, o, r, P->min, 0); return 0;
    case LOFT_P_NINT:
    case LOFT_P_NINTRAW: loft_dlv_raw(cx, o, r, 4); return 0;
    case LOFT_P_RECORD:
        for (uint16_t k = 0; k < P->n; k++) {
            const loft_pfield *f = &P->fields[k];
            /* `LayoutField::is_data`: the enum discriminant, an absent field and a `#` synthetic
             * are not data. */
            if (strcmp(f->name, "enum") == 0 || f->pos == 0xFFFF || f->name[0] == '#') continue;
            const char *e = loft_crt_dlv_walk(cx, o, f->content, loft_field(cx, r, f->pos));
            if (e) return e;
        }
        return 0;
    case LOFT_P_VECTOR: {
        uint32_t v = loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos), n = v ? loft_u32(cx, loft_store_of(cx, r), v, 4) : 0;
        uint32_t size = P->elem < loft_crt_nptypes ? loft_crt_ptypes[P->elem].size : 0;
        for (uint32_t i = 0; i < n; i++) {
            loft_ref x = { r.store_nr, v, 8 + size * i, 0 };
            const char *e = loft_crt_dlv_walk(cx, o, P->elem, x);
            if (e) return e;
        }
        return 0;
    }
    default:
        if (P->kind == LOFT_P_KEYED && P->keyed == LOFT_P_ARRAY) {
            uint32_t v = loft_u32(cx, loft_store_of(cx, r), r.rec, r.pos), n = v ? loft_u32(cx, loft_store_of(cx, r), v, 4) : 0;
            for (uint32_t i = 0; i < n; i++) {
                loft_ref x = { r.store_nr, loft_u32(cx, loft_store_of(cx, r), v, 8 + 4 * i), 8, 0 };
                const char *e = loft_crt_dlv_walk(cx, o, P->elem, x);
                if (e) return e;
            }
            return 0;
        }
        snprintf(why, sizeof why,
                 "type %u (%s) is a store-internal kind — not in the serializable subset (cursor-walked in a later phase)",
                 tp, P->name);
        return why;
    }
}

void loft_crt_deliver(int64_t tag, int64_t s, int64_t r, int64_t p, int64_t tp, const char *name, int64_t nlen) {
    loft_cx *cx = &loft_main_cx;
    loft_text o = { 0, 0, 0 };
    const char *e = loft_crt_dlv_walk(cx, &o, (uint16_t) tp, loft_crt_ref(cx, s, r, p));
    if (e) printf("deliver tag=%lld type=%.*s error=%s\n", (long long) tag, (int) nlen, name, e);
    else printf("deliver tag=%lld type=%.*s bytes=%.*s\n", (long long) tag, (int) nlen, name, (int) o.len, o.p ? o.p : "");
    loft_text_free(cx, &o);
}

/* `LOFT_CRT_PTYPES=1`: the parse table's rows on standard error, in the emitter's spelling of
 * `loft_ptypes`, to compare with a `--native-c --native-emit` of the same program. */
static void loft_crt_ptypes_dump(loft_cx *cx) {
    for (uint32_t tp = 0; tp < loft_crt_nptypes; tp++) {
        const loft_ptype *r = &loft_crt_ptypes[tp];
        if (!r->name) continue;
        fprintf(stderr, "[%u] kind=%u size=%u elem=%u min=%d nullable=%u n=%u strict=%u zero=%u variant=%u keyed=%u nk=%u linked=%u some=%u payload=%u@%u refs=%u name=%s\n",
                tp, r->kind, r->size, r->elem, r->min, r->nullable, r->n, r->strict_enum, r->zero_default, r->variant,
                r->keyed, r->nk, r->linked, r->some, r->payload_tp, r->payload_pos, r->refs, r->name);
        for (uint16_t k = 0; r->kind == LOFT_P_ENUM && k < r->n; k++)
            fprintf(stderr, "    variant %u %s -> %u\n", k + 1, r->vnames[k + 1], r->vtypes[k]);
        for (uint16_t k = 0; r->kind == LOFT_P_RECORD && k < r->n; k++) {
            const loft_pfield *f = &r->fields[k];
            fprintf(stderr, "    field %s content=%u pos=%u nullable=%u skip=%u tag=%u dkind=%u di=%lld ds=%s ngrp=%u target=%u\n",
                    f->name, f->content, f->pos, f->nullable, f->skip, f->type_tag, f->dkind, (long long) f->di,
                    f->ds, f->ngrp, f->target);
        }
        if (r->prefill) {
            fprintf(stderr, "    prefill");
            for (uint16_t k = 0; k < r->size; k++) fprintf(stderr, " %u", r->prefill[k]);
            fprintf(stderr, "\n");
        }
    }
}

/* `t as vector<T>` / a record from text (`loft_record_from_text`) as type `tp`: its reference. */
int64_t loft_crt_record_from_text(int64_t *out, int64_t no, const char *text, int64_t len, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    const char *d = getenv("LOFT_CRT_PTYPES");
    if (d && *d == '1') loft_crt_ptypes_dump(cx);
    loft_crt_put_ref(cx, out, no, loft_record_from_text(cx, loft_crt_ptypes, loft_crt_str(cx, text, len), (uint16_t) tp));
    return 0;
}

/* OpMoveVector (Stores::move_vector): `src`'s elements moved onto `dst`, `linked` an element
 * type of a linked group. */
void loft_crt_move_vector(int64_t d_store, int64_t d_rec, int64_t d_pos, int64_t s_store, int64_t s_rec,
                          int64_t s_pos, int64_t tp, int64_t linked) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref d = loft_crt_ref(cx, d_store, d_rec, d_pos), s = loft_crt_ref(cx, s_store, s_rec, s_pos);
    if (linked) loft_move_vector_array(cx, loft_crt_types, (uint16_t) tp, d, s);
    else loft_move_vector(cx, loft_crt_types, (uint16_t) tp, d, s);
}

/* OpMoveField / OpMoveRecord: a `tp` record moved from `src` to `dst` — into a field, or out to a
 * returned record (`record_out`). */
void loft_crt_move_field(int64_t s_store, int64_t s_rec, int64_t s_pos, int64_t d_store, int64_t d_rec,
                         int64_t d_pos, int64_t tp, int64_t record_out) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref s = loft_crt_ref(cx, s_store, s_rec, s_pos), d = loft_crt_ref(cx, d_store, d_rec, d_pos);
    if (record_out) loft_move_record_out(cx, loft_crt_types, (uint16_t) tp, s, d);
    else loft_move_field(cx, loft_crt_types, (uint16_t) tp, s, d);
}

/* OpFreeRecordIn: a placed `tp` record released inside its host's store. */
void loft_crt_free_record_in(int64_t store, int64_t rec, int64_t pos, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_free_record_in(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos));
}

/* OpReplaceVector (Stores::vector_replace): `v`'s elements replaced by `o`'s. */
void loft_crt_vec_replace(int64_t v_store, int64_t v_rec, int64_t v_pos, int64_t o_store, int64_t o_rec,
                          int64_t o_pos, int64_t tp, int64_t linked) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref v = loft_crt_ref(cx, v_store, v_rec, v_pos), o = loft_crt_ref(cx, o_store, o_rec, o_pos);
    if (linked) loft_vec_replace_array(cx, loft_crt_types, (uint16_t) tp, v, o);
    else loft_vec_replace(cx, loft_crt_types, loft_crt_ntypes, (uint16_t) tp, v, o);
}

/* `vector_sum_int` (KERNELS.md, `sum`'s integer loop): `acc` plus every element of the vector,
 * with integer addition's rules — a null element or an overflow makes it null. */
int64_t loft_crt_vector_sum_int(int64_t store, int64_t rec, int64_t pos, int64_t acc) {
    loft_cx *cx = &loft_main_cx;
    return loft_vector_sum_int(cx, loft_crt_ref(cx, store, rec, pos), acc);
}

/* OpClearKeyed of a SECONDARY VIEW of a linked group (`CLEAR_KEYED_VIEW`): the view drops its own
 * route and leaves the records to the member that holds them (`remove_claims_keyed`). */
void loft_crt_clear_view(int64_t store, int64_t rec, int64_t pos, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_remove_claims_mode(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos), 1);
}

/* OpReplaceKeyed (`Stores::replace_keyed`): `dest`'s collection filled again from `src`'s, of
 * type `tp`; `flags` bit 0 the caller's own, bit 1 the element tree may move whole. */
void loft_crt_replace_keyed(int64_t s_store, int64_t s_rec, int64_t s_pos, int64_t d_store, int64_t d_rec,
                            int64_t d_pos, int64_t tp, int64_t flags) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_replace_keyed(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, s_store, s_rec, s_pos),
                       loft_crt_ref(cx, d_store, d_rec, d_pos), (int) flags);
}

/* A record variable bound to a copy of another (`loft_bind_copy`, the C generator's
 * `whole_value_bind`): the variable in `io` gets a store of its own holding `src`'s record and
 * the claims it owns, by the table loaded above. */
void loft_crt_bind_copy(int64_t *io, int64_t n, int64_t tp, int64_t s_store, int64_t s_rec,
                        int64_t s_pos) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_ref r = loft_crt_get_ref(cx, io, n);
    loft_bind_copy(cx, loft_crt_types, (uint16_t) tp, &r, loft_crt_ref(cx, s_store, s_rec, s_pos));
    loft_crt_put_ref(cx, io, n, r);
}

/* OpCopyRecord: the record at `src` copied into `dst`, the claims it owns copied with it, by the
 * table loaded above. */
void loft_crt_copy_record(int64_t s_store, int64_t s_rec, int64_t s_pos, int64_t d_store,
                          int64_t d_rec, int64_t d_pos, int64_t tp, int64_t flags) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_copy_record(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, s_store, s_rec, s_pos),
                     loft_crt_ref(cx, d_store, d_rec, d_pos), (int) flags);
}

/* OpClearVector: emptied, the claims its elements own released, by the loaded table. */
void loft_crt_vec_clear(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_types) return;
    loft_vec_clear_release(cx, loft_crt_types, loft_crt_ntypes, loft_crt_ref(cx, store, rec, pos));
}

/* ── Keyed collections (@PLN91 S3, the fourth slice): the hash routines the C generator calls
 * (`keyed_new`, `keyed_finish`, `get_record`, `OpLengthHash`), each reading its collection's key
 * parts from the table loaded above, so the interpreter hands no pointer across.
 *
 * A lookup's values are pushed one part at a time onto a KEY STACK, and the find takes the top
 * `n` parts: a key may itself be a lookup (`h[g[k].name]`), whose parts are pushed and popped
 * before the outer one's are pushed.  A text part is copied, so it outlives the interpreter's
 * text it came from until the find pops it. */
#define LOFT_CRT_KEYS 256
static loft_kv loft_crt_kv[LOFT_CRT_KEYS];
static char *loft_crt_kv_owned[LOFT_CRT_KEYS];
static int loft_crt_nkv = 0;

static void loft_crt_key_push(loft_cx *cx, loft_kv k, char *owned) {
    if (loft_crt_nkv >= LOFT_CRT_KEYS) {
        free(owned);
        return;
    }
    loft_crt_kv[loft_crt_nkv] = k;
    loft_crt_kv_owned[loft_crt_nkv] = owned;
    loft_crt_nkv++;
}

void loft_crt_key_int(int64_t v, int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_key_push(cx, loft_kv_int(cx, is_null ? LOFT_NULL : v), 0);
}

void loft_crt_key_float(const double *v, int64_t n, int64_t is_null, int64_t single) {
    loft_cx *cx = &loft_main_cx;
    double f = is_null || n < 1 ? (double) NAN : v[0];
    loft_crt_key_push(cx, single ? loft_kv_single(cx, (float) f) : loft_kv_float(cx, f), 0);
}

void loft_crt_key_text(const char *val, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    if (bytes < 0) {
        loft_crt_key_push(cx, loft_kv_text(cx, loft_null_text(cx)), 0);
        return;
    }
    size_t len = (size_t) bytes;
    char *copy = (char *) malloc(len + 1);
    if (!copy) return;
    memcpy(copy, val, len + 1);
    loft_crt_key_push(cx, loft_kv_text(cx, loft_lit(cx, copy, len)), copy);
}

/* A text part as the view in the stack store's frame slot at `at`, not copied: the operator
 * that pushed it reads the key before it pops the slot. */
void loft_crt_key_view(int64_t store, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_key_push(cx, loft_kv_text(cx, *(const loft_str *) loft_crt_slot_at(cx, store, at)), 0);
}

/* The top `n` parts popped, their text copies released. */
static void loft_crt_key_pop(loft_cx *cx, int64_t n) {
    while (n-- > 0 && loft_crt_nkv > 0) {
        loft_crt_nkv--;
        free(loft_crt_kv_owned[loft_crt_nkv]);
        loft_crt_kv_owned[loft_crt_nkv] = 0;
    }
}

static int loft_crt_keyed(loft_cx *cx, int64_t tp) {
    return loft_crt_types && tp >= 0 && (uint32_t) tp < loft_crt_ntypes;
}

/* OpGetRecord on a hash of type `tp`: the record whose key is the top `n` parts, into `out`; the
 * null reference on a miss.  The parts are popped either way. */
int64_t loft_crt_hash_find(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t tp, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_ref_null(cx);
    if (loft_crt_keyed(cx, tp) && n >= 0 && n <= loft_crt_nkv)
        r = loft_h_find(cx, loft_crt_ref(cx, store, rec, pos), loft_crt_types[tp].keys,
                        &loft_crt_kv[loft_crt_nkv - n], (int) n);
    loft_crt_key_pop(cx, n);
    loft_crt_put_ref(cx, out, no, r);
    return 0;
}

/* `@FR-R-KeyedEntry` (`n_keyed_entry_ints` / `n_keyed_entry_text`): the entry under the top `n`
 * key parts in hash `tp` of element type `elem` — the one already there, or a fresh one with the
 * key written and filed — into `out`; the null reference where the hash cannot take it.  The
 * parts are popped either way. */
int64_t loft_crt_keyed_entry(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                             int64_t elem, int64_t tp, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_ref_null(cx);
    if (loft_crt_keyed(cx, tp) && loft_crt_keyed(cx, elem) && n >= 1 && n <= 4 && n <= loft_crt_nkv) {
        const loft_kv *kv = &loft_crt_kv[loft_crt_nkv - n];
        loft_ref c = loft_crt_ref(cx, store, rec, pos);
        const loft_tdesc *T = &loft_crt_types[tp];
        if (kv[0].kind == LOFT_KV_TEXT)
            r = loft_keyed_entry_text(cx, loft_crt_types, (uint16_t) elem, c, T->keys, T->nk, kv[0].s);
        else {
            int64_t ks[4];
            for (int i = 0; i < n; i++) ks[i] = kv[i].i;
            r = loft_keyed_entry_ints(cx, loft_crt_types, (uint16_t) elem, c, T->keys, T->nk, ks);
        }
    }
    loft_crt_key_pop(cx, n);
    loft_crt_put_ref(cx, out, no, r);
    return 0;
}

/* OpNewRecord in a hash: an entry of `size` bytes of its table, into `out`. */
int64_t loft_crt_hash_entry(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                            int64_t size, int64_t linked) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no,
                     loft_hash_entry(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size, (int) linked));
    return 0;
}

/* OpFinishRecord in a hash of type `tp`: the record goes to its key's place, an equal key's
 * element replaced and released by the element type `elem`'s row. */
void loft_crt_hash_insert(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                          int64_t r_pos, int64_t elem, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp) || !loft_crt_keyed(cx, elem)) return;
    (void) loft_hash_insert(cx, loft_crt_types, (uint16_t) elem, loft_crt_ref(cx, store, rec, pos),
                            loft_crt_ref(cx, r_store, r_rec, r_pos), loft_crt_types[tp].keys,
                            loft_crt_types[tp].nk, 0);
}

/* OpHashRemove on a hash of type `tp` (element type `elem`): the entry unlinked from its key, and
 * released unless `view` (`CLEAR_KEYED_VIEW`: a linked group's view member only unlinks). */
void loft_crt_hash_remove(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                          int64_t r_pos, int64_t elem, int64_t tp, int64_t view) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp) || !loft_crt_keyed(cx, elem)) return;
    loft_hash_remove(cx, loft_crt_types, (uint16_t) elem, loft_crt_ref(cx, store, rec, pos),
                     loft_crt_ref(cx, r_store, r_rec, r_pos), loft_crt_types[tp].keys,
                     loft_crt_types[tp].nk, (int) view);
}

/* OpSetKeyed (`c[k…] = v`, `Stores::set_keyed`) on a hash of type `tp`: a copy of `v` filed under
 * the top `n` key parts, which are popped; `free_source` releases `v`'s store after. */
void loft_crt_set_keyed(int64_t store, int64_t rec, int64_t pos, int64_t v_store, int64_t v_rec,
                        int64_t v_pos, int64_t elem, int64_t tp, int64_t n, int64_t free_source,
                        int64_t kind) {
    loft_cx *cx = &loft_main_cx;
    /* `kind` as the lowering numbers it: 0 a hash, 1 a sorted, 2 an ordered, 3 an index, 4 a
     * spatial index, 5 a trie. */
    int k = kind == 1 ? LOFT_KEYED_SORTED : kind == 2 ? LOFT_KEYED_ORDERED : kind == 3 ? LOFT_KEYED_INDEX
          : kind == 4 ? LOFT_KEYED_SPATIAL : kind == 5 ? LOFT_KEYED_TRIE : LOFT_KEYED_HASH;
    if (loft_crt_keyed(cx, tp) && loft_crt_keyed(cx, elem) && n >= 0 && n <= loft_crt_nkv)
        loft_set_keyed(cx, loft_crt_types, (uint16_t) elem, loft_crt_ref(cx, store, rec, pos),
                       loft_crt_ref(cx, v_store, v_rec, v_pos), k, loft_crt_types[tp].keys,
                       loft_crt_types[tp].nk, &loft_crt_kv[loft_crt_nkv - n], (int) n,
                       (int) free_source, 0);
    loft_crt_key_pop(cx, n);
}

/* OpSizeHash (`hash::table_bytes`): the bytes of a hash's bucket table, holes included. */
int64_t loft_crt_hash_bytes(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_hash_table_bytes(cx, loft_crt_ref(cx, store, rec, pos));
}

/* OpLengthHash. */
int64_t loft_crt_hash_len(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_hash_len(cx, loft_crt_ref(cx, store, rec, pos));
}

/* ── Walking a hash (@PLN91 S3, the fourth slice continued): the snapshot `for c in h` steps
 * over, two records claimed in the hash's own store (loft_hash.h). ── */

/* n_hash_sorted: the snapshot of a hash of type `tp`, its elements in key order, into `out`;
 * n_hash_unsorted (`unordered`): in slot order, or key order where the table holds store
 * records (`@FR-Col-Unordered`), so it is handed the keys too. */
int64_t loft_crt_hash_sorted(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                             int64_t tp, int64_t unordered) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_ref_null(cx);
    int keyed = loft_crt_keyed(cx, tp);
    if (unordered)
        r = loft_hash_unsorted(cx, loft_crt_ref(cx, store, rec, pos), keyed ? loft_crt_types[tp].keys : 0,
                               keyed ? loft_crt_types[tp].nk : 0);
    else if (keyed)
        r = loft_hash_sorted(cx, loft_crt_ref(cx, store, rec, pos), loft_crt_types[tp].keys,
                             loft_crt_types[tp].nk);
    loft_crt_put_ref(cx, out, no, r);
    return 0;
}

/* `hash::records_sorted`: the hash's entries in key order, as `OpFormatDatabase` walks them —
 * held in the runtime's own memory, so the walk claims nothing in the store.  Writes up to
 * `no / 3` references into `out` and answers how many entries there are. */
int64_t loft_crt_hash_refs(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp)) return 0;
    uint32_t n = 0;
    loft_ref *refs = loft_hash_refs(cx, loft_crt_ref(cx, store, rec, pos), loft_crt_types[tp].keys,
                                    loft_crt_types[tp].nk, 0, &n);
    for (uint32_t i = 0; i < n && 3 * (int64_t) i + 2 < no; i++) {
        out[3 * i] = refs[i].store_nr;
        out[3 * i + 1] = refs[i].rec;
        out[3 * i + 2] = refs[i].pos;
    }
    free(refs);
    return n;
}

/* OpIterate over a snapshot: the walk's state. */
int64_t loft_crt_scratch_iterate(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_scratch_iterate(cx, loft_crt_ref(cx, store, rec, pos));
}

/* OpStep over a snapshot: the next element into `out[0..3]`, the state it leaves into `out[3]`. */
int64_t loft_crt_scratch_step(int64_t *out, int64_t no, int64_t state, int64_t store, int64_t rec,
                              int64_t pos, int64_t reverse) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_scratch_step(cx, &state, loft_crt_ref(cx, store, rec, pos), (int) reverse);
    loft_crt_put_ref(cx, out, no, r);
    if (no >= 4) out[3] = state;
    return 0;
}

/* OpFreeScratch. */
void loft_crt_free_scratch(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_free_scratch(cx, loft_crt_ref(cx, store, rec, pos));
}

/* OpAppendVector (@PLN91 S3): the elements of `o` appended to `v` as copies of type `tp`, their
 * claims copied by the loaded table. */
void loft_crt_vec_add(int64_t tp, int64_t store, int64_t rec, int64_t pos, int64_t o_store,
                      int64_t o_rec, int64_t o_pos) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp)) return;
    /* A linked element type is an array of record ids: each record copied into a record of its
     * own (`loft_vec_add_array`). */
    if (loft_crt_types[tp].linked)
        loft_vec_add_array(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos),
                           loft_crt_ref(cx, o_store, o_rec, o_pos));
    else
        loft_vec_add(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos),
                     loft_crt_ref(cx, o_store, o_rec, o_pos));
}

/* ── Rendering a value (@PLN91 S3, `OpFormatDatabase`): the interpreter walks the schema in loft
 * as the C generator's `loft_show{tp}` does, and every value read from the store is the
 * runtime's own `loft_show_*` helper, chosen here by `kind`: the base types by their type
 * number (0–6), then 7 a byte, 8 a short, 9 a raw short, 10 a 4-byte and 11 a raw 4-byte
 * integer, 12 a stored reference, 13 a child record.  Rendered into a scratch text, copied out. */
static loft_text loft_crt_show_buf = { 0, 0, 0 };

int64_t loft_crt_show_leaf(char *out, int64_t cap, int64_t kind, int64_t store, int64_t rec,
                           int64_t pos, int64_t fmt, int64_t min, int64_t nullable) {
    loft_cx *cx = &loft_main_cx;
    loft_text *s = &loft_crt_show_buf;
    s->len = 0;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    int f = (int) fmt, n = (int) nullable;
    switch (kind) {
    case 0: case 1: loft_show_long(cx, s, r, f, 0); break;
    case 2: loft_show_single(cx, s, r, f, 0); break;
    case 3: loft_show_float(cx, s, r, f, 0); break;
    case 4: loft_show_bool(cx, s, r, f, 0); break;
    case 5: loft_show_text(cx, s, r, f, 0); break;
    case 6: loft_show_char(cx, s, r, f, 0); break;
    case 7: loft_show_byte(cx, s, r, min, n); break;
    case 8: loft_show_short(cx, s, r, min, n); break;
    case 9: loft_show_short_raw(cx, s, r, min, n); break;
    case 10: loft_show_int4(cx, s, r, n); break;
    case 11: loft_show_int_raw(cx, s, r, n); break;
    case 12: loft_show_dbref(cx, s, r); break;
    case 13: loft_show_child(cx, s, r); break;
    default: return 0;
    }
    int64_t len = (int64_t) s->len;
    if (len <= cap && len > 0) memcpy(out, s->p, (size_t) len);
    return len;
}

/* loft_vec_absent: a collection field that holds none. */
int64_t loft_crt_vec_absent(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_vec_absent(cx, loft_crt_ref(cx, store, rec, pos));
}

/* OpSliceVector / OpSliceView (Stores::vector_slice / vector_slice_view): `v = src[lo..hi]`
 * of scalar elements `size` bytes wide, the bounds clamped; `view` serves a foreign source's
 * bytes in place where the runtime can. */
void loft_crt_vec_slice(int64_t v_store, int64_t v_rec, int64_t v_pos, int64_t o_store, int64_t o_rec,
                        int64_t o_pos, int64_t lo, int64_t hi, int64_t size, int64_t view) {
    loft_cx *cx = &loft_main_cx;
    loft_ref v = loft_crt_ref(cx, v_store, v_rec, v_pos), o = loft_crt_ref(cx, o_store, o_rec, o_pos);
    if (view) loft_vec_slice_view(cx, v, o, lo, hi, (uint32_t) size);
    else loft_vec_slice(cx, v, o, lo, hi, (uint32_t) size);
}

/* OpAppendCopy (State::append_copy): `[x; n]` — the vector's last element repeated until it
 * holds `count`, of element type `ctp`; `count` <= 0 leaves it empty. */
void loft_crt_vec_append_copy(int64_t store, int64_t rec, int64_t pos, int64_t ctp, int64_t count) {
    loft_cx *cx = &loft_main_cx;
    loft_vec_append_copy(cx, loft_crt_types, (uint16_t) ctp, loft_crt_ref(cx, store, rec, pos), count);
}

/* OpKeepVectorRange (`Stores::keep_vector_range`): only the elements `lo..hi` kept, what the
 * dropped ones own released through the type table; `size` 0 for a linked element. */
void loft_crt_vec_keep(int64_t store, int64_t rec, int64_t pos, int64_t tp, int64_t size, int64_t lo,
                       int64_t hi) {
    loft_cx *cx = &loft_main_cx;
    loft_keep_vector_range(cx, (const loft_tdesc *) loft_crt_types, (uint16_t) tp,
                           loft_crt_ref(cx, store, rec, pos), (uint32_t) size, lo, hi);
}

/* OpAppendTextBytes: the bytes `lo..hi` of a text appended to a byte vector. */
void loft_crt_append_text_bytes(int64_t store, int64_t rec, int64_t pos, const char *val,
                                int64_t bytes, int64_t lo, int64_t hi) {
    loft_cx *cx = &loft_main_cx;
    loft_append_text_bytes(cx, loft_crt_ref(cx, store, rec, pos), loft_crt_str(cx, val, bytes), lo, hi);
}

/* text_from_byte_range (`Stores::text_from_bytes_range`): the bytes [lo, hi) of a vector<u8> as a
 * text, copied into `out` when it fits; answers the length in bytes. */
int64_t loft_crt_text_from_byte_range(char *out, int64_t cap, int64_t store, int64_t rec,
                                      int64_t pos, int64_t lo, int64_t hi) {
    loft_cx *cx = &loft_main_cx;
    loft_text t = loft_text_from_byte_range(cx, loft_crt_ref(cx, store, rec, pos), lo, hi);
    int64_t len = (int64_t) t.len;
    if (len <= cap && len > 0) memcpy(out, t.p, (size_t) len);
    loft_text_free(cx, &t);
    return len;
}

/* OpReverseVector: the elements of `size` bytes reversed in place. */
void loft_crt_vec_reverse(int64_t store, int64_t rec, int64_t pos, int64_t size) {
    loft_cx *cx = &loft_main_cx;
    loft_vec_reverse(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size);
}

/* OpReserveVector (`vector::reserve_vector`): room for `count` elements of `size` bytes. */
void loft_crt_vec_reserve(int64_t store, int64_t rec, int64_t pos, int64_t count, int64_t size) {
    loft_cx *cx = &loft_main_cx;
    loft_vec_reserve(cx, loft_crt_ref(cx, store, rec, pos), count, (uint32_t) size);
}

/* OpReserveHash (`Stores::reserve_hash`): a hash of type `tp` sized for `count` entries. */
void loft_crt_hash_reserve(int64_t store, int64_t rec, int64_t pos, int64_t tp, int64_t count) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp)) return;
    loft_hash_reserve(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos), count);
}

/* OpPrependVector (`Stores::prepend_vector`): `o`'s elements put in front of `v`'s, copied by the
 * element type `tp`'s table entry; answers whether anything was prepended. */
int64_t loft_crt_vec_prepend(int64_t store, int64_t rec, int64_t pos, int64_t o_store, int64_t o_rec,
                             int64_t o_pos, int64_t tp, int64_t linked) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp)) return 0;
    return loft_vec_prepend(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, store, rec, pos),
                            loft_crt_ref(cx, o_store, o_rec, o_pos), (int) linked);
}

/* OpSetDbRef (`Stores::set_db_ref`): a reference written into the twelve bytes at `fld`, the record
 * it names marked referenced, so removing it leaves a tombstone. */
void loft_crt_set_dbref(int64_t store, int64_t rec, int64_t pos, int64_t fld, int64_t r_store,
                        int64_t r_rec, int64_t r_pos) {
    loft_cx *cx = &loft_main_cx;
    loft_set_dbref(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld, loft_crt_ref(cx, r_store, r_rec, r_pos));
}

/* OpGetDbRef: the reference stored at `fld`, into `out`; a removed record reads null. */
int64_t loft_crt_get_dbref(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_get_dbref(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) fld));
    return 0;
}

/* OpNewRecord in an array (a vector of linked records): a record of `size` bytes of its own in
 * the collection's store (`loft_entry_new`), into `out`. */
int64_t loft_crt_entry_new(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t size) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_entry_new(cx, loft_crt_ref(cx, store, rec, pos), (uint32_t) size));
    return 0;
}

/* OpFinishRecord in an array: the record's number joins the end. */
void loft_crt_array_append(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                           int64_t r_pos) {
    loft_cx *cx = &loft_main_cx;
    loft_array_append(cx, loft_crt_ref(cx, store, rec, pos), loft_crt_ref(cx, r_store, r_rec, r_pos));
}

/* OpVectorRef / OpVectorRefNullable on an array: the record its slot `idx` names, into `out`; the
 * null reference past either end. */
int64_t loft_crt_array_get(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t idx, int64_t is_null) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, is_null ? loft_ref_null(cx) : loft_array_get(cx, loft_crt_ref(cx, store, rec, pos), idx));
    return 0;
}

/* OpRemoveVector (`Stores::remove_vector_at`): the element at `index` of element type `tp` taken
 * out, as the C generator picks the routine — `mode` 2 a linked element (its slot unlinked, its
 * record released), 1 an element owning heap (released first), 0 a plain one of `size` bytes.
 * Answers whether an element was removed. */
int64_t loft_crt_vec_remove(int64_t store, int64_t rec, int64_t pos, int64_t tp, int64_t size,
                            int64_t mode, int64_t index) {
    loft_cx *cx = &loft_main_cx;
    loft_ref v = loft_crt_ref(cx, store, rec, pos);
    const loft_tdesc *T = (const loft_tdesc *) loft_crt_types;
    if (mode == 2) return loft_vec_remove_linked(cx, T, (uint16_t) tp, v, index);
    if (mode == 1) return loft_vec_remove_at(cx, T, (uint16_t) tp, v, index);
    return loft_vec_remove(cx, v, (uint32_t) size, index);
}

/* The interpreted program starts: the trace's `M` line, as the Rust interpreter writes it when
 * it enters the program (`State::execute_argv_shared`). */
void loft_crt_trace_mark(void) {
    loft_cx *cx = &loft_main_cx;
    LOFT_TRACE("M\n");
}

/* ── Files (@PLN91 S6): the interpreted program's File records, through the runtime's own file
 * routines.  A relative path resolves against `LOFT_SOURCE_DIR`, which the driver hands the run,
 * as a C program anchors (`loft_set_anchor`) — or against the working directory, for a program
 * that declared `#cwd`. */
void loft_crt_fs_anchor(int64_t program_relative) {
    loft_cx *cx = &loft_main_cx;
    loft_set_anchor(cx, (int) program_relative);
}

/* OpGetFile: what the File's path names. */
int64_t loft_crt_get_file(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_get_file(cx, loft_crt_ref(cx, store, rec, pos));
}

/* OpGetFileText: the whole file into the text local at byte `at` of the stack store. */
void loft_crt_file_text(int64_t store, int64_t rec, int64_t pos, int64_t stk, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_get_file_text(cx, loft_crt_ref(cx, store, rec, pos), (loft_text *) loft_crt_slot_at(cx, stk, at));
}

/* `write_bytes(path, bytes)`: the file is the store vector's bytes. */
int64_t loft_crt_write_bytes(const char *path, int64_t bytes, int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_write_bytes(cx, loft_crt_str(cx, path, bytes), loft_crt_ref(cx, store, rec, pos));
}

/* Is a write or read through this File little-endian? */
int64_t loft_crt_file_le(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    return loft_file_le(cx, loft_crt_ref(cx, store, rec, pos));
}

/* A value's binary form (`Stores::read_data`) is gathered here, piece by piece as the loft side
 * walks its type, then written at the File's next position in one `loft_write_file`. */
static loft_text loft_crt_wbuf = { 0, 0, 0 };

/* `w` raw bytes at a place, in the File's byte order. */
void loft_crt_ser_at(int64_t store, int64_t rec, int64_t pos, int64_t w, int64_t le) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    uint64_t v = 0;
    memcpy(&v, loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), (size_t) w);
    loft_put(cx, &loft_crt_wbuf, v, (int) w, (int) le);
}

/* An integer's low `w` bytes, in the File's byte order. */
void loft_crt_ser_int(int64_t v, int64_t w, int64_t le) {
    loft_cx *cx = &loft_main_cx;
    loft_put(cx, &loft_crt_wbuf, (uint64_t) v, (int) w, (int) le);
}

/* A text field's bytes. */
void loft_crt_ser_text(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_text_append(cx, &loft_crt_wbuf, loft_get_text(cx, loft_crt_ref(cx, store, rec, pos), 0));
}

/* A text local's bytes (the local at byte `at` of the stack store). */
void loft_crt_ser_slot_text(int64_t stk, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_text_append(cx, &loft_crt_wbuf, loft_view(cx, (loft_text *) loft_crt_slot_at(cx, stk, at)));
}

/* The run image's runner (step 3): a text reached through a reference rather than a frame byte. */
void loft_crt_ser_ref_text(int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    if (t) loft_text_append(cx, &loft_crt_wbuf, loft_view(cx, t));
}

/* OpWriteFile: the gathered bytes at the File's next position; the buffer emptied. */
void loft_crt_ser_write(int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_write_file(cx, loft_crt_ref(cx, store, rec, pos), &loft_crt_wbuf);
    loft_crt_wbuf.len = 0;
}

/* OpReadFile: up to `n` bytes from the File's next position, kept here while the loft side
 * decodes them into the value (`Stores::write_data`); how many came, -1 when it cannot read. */
static uint8_t *loft_crt_rbuf = 0;
static int64_t loft_crt_rlen = 0;

int64_t loft_crt_read_file(int64_t store, int64_t rec, int64_t pos, int64_t n, int64_t text, int64_t width) {
    loft_cx *cx = &loft_main_cx;
    free(loft_crt_rbuf);
    loft_crt_rbuf = 0;
    loft_crt_rlen = loft_read_file(cx, loft_crt_ref(cx, store, rec, pos), n, (int) text, width, &loft_crt_rbuf);
    return loft_crt_rlen;
}

/* The read bytes as the text local at byte `at` of the stack store. */
void loft_crt_de_slot_text(int64_t stk, int64_t at) {
    loft_cx *cx = &loft_main_cx;
    loft_text_set(cx, (loft_text *) loft_crt_slot_at(cx, stk, at),
                  loft_lit(cx, (const char *) loft_crt_rbuf, (size_t) (loft_crt_rlen < 0 ? 0 : loft_crt_rlen)));
}

void loft_crt_de_ref_text(int64_t s, int64_t r, int64_t p) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, s, r, p), 0);
    if (t) loft_text_set(cx, t, loft_lit(cx, (const char *) loft_crt_rbuf, (size_t) (loft_crt_rlen < 0 ? 0 : loft_crt_rlen)));
}

/* `OpGetFileText`: the file's text into the text the reference `(ts, tr, tp)` names. */
void loft_crt_file_text_ref(int64_t store, int64_t rec, int64_t pos, int64_t ts, int64_t tr, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    loft_text *t = (loft_text *) loft_place(cx, loft_crt_ref(cx, ts, tr, tp), 0);
    if (t) loft_get_file_text(cx, loft_crt_ref(cx, store, rec, pos), t);
}

/* The read bytes `off..off+n` as a text field. */
void loft_crt_de_text(int64_t store, int64_t rec, int64_t pos, int64_t off, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_set_text(cx, loft_crt_ref(cx, store, rec, pos), 0, loft_lit(cx, (const char *) loft_crt_rbuf + off, (size_t) n));
}

/* The `w` read bytes at `off`, in the File's byte order, raw at a place. */
void loft_crt_de_at(int64_t store, int64_t rec, int64_t pos, int64_t off, int64_t w, int64_t le) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    uint64_t v = loft_take(cx, loft_crt_rbuf + off, (int) w, (int) le);
    memcpy(loft_at(cx, loft_store_of(cx, r), r.rec, r.pos), &v, (size_t) w);
}

/* The `w` read bytes at `off` as an integer, sign-extended from `w` bytes when `sgn`. */
int64_t loft_crt_de_int(int64_t off, int64_t w, int64_t sgn, int64_t le) {
    loft_cx *cx = &loft_main_cx;
    return loft_fcur_decode(cx, loft_crt_rbuf + off, (int) w, (int) sgn, (int) le);
}

/* OpDelete, OpMoveFile, OpMkdir, OpMkdirAll, OpRmdir, by `which` (0–4): the runtime's own, each
 * answering its FileResult code. */
int64_t loft_crt_fs_op(int64_t which, const char *a, int64_t an, const char *b, int64_t bn) {
    loft_cx *cx = &loft_main_cx;
    loft_str x = loft_crt_str(cx, a, an);
    switch (which) {
    case 0: return loft_fs_delete(cx, x);
    case 1: return loft_fs_move(cx, x, loft_crt_str(cx, b, bn));
    case 2: return loft_fs_mkdir(cx, x, 0);
    case 3: return loft_fs_mkdir(cx, x, 1);
    case 4: return loft_fs_rmdir(cx, x);
    default: return LOFT_FS_OTHER;
    }
}

/* OpSizeFile, OpSeekFile, OpTruncateFile, OpSyncFile, by `which` (0–3), on a File record. */
int64_t loft_crt_file_op(int64_t which, int64_t store, int64_t rec, int64_t pos, int64_t arg) {
    loft_cx *cx = &loft_main_cx;
    loft_ref f = loft_crt_ref(cx, store, rec, pos);
    switch (which) {
    case 0: return loft_size_file(cx, f);
    case 1: loft_seek_file(cx, f, arg); return 0;
    case 2: return loft_truncate_file(cx, f, arg);
    case 3: return loft_sync_file(cx, f);
    default: return 0;
    }
}

/* `read_bytes(path)`: the whole file as a byte vector in a store of its own, into `out`; the null
 * reference for a missing or unreadable file. */
int64_t loft_crt_read_bytes(int64_t *out, int64_t no, const char *path, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_read_bytes(cx, loft_crt_str(cx, path, bytes)));
    return 0;
}

/* `File.content()` as the compiled `t_4File_content` answers it, for a compiled body that calls it
 * (`t_4File_lines`): null for a missing path or a directory, else the file's text through
 * `loft_get_file_text` — null when a file with bytes gave none (they are not text).  The length,
 * or -1 for null; nothing is written when it exceeds `cap`. */
int64_t loft_crt_file_content(char *out, int64_t cap, int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_ref f = loft_crt_ref(cx, store, rec, pos);
    uint8_t format = loft_file_format(cx, f);
    if (format == 5 || format == 4) return -1;
    loft_text t = { 0, 0, 0 };
    loft_get_file_text(cx, f, &t);
    int64_t size = loft_file_long(cx, f, 0);
    if (t.len == 0 && size != LOFT_NULL && size > 0) {
        loft_text_free(cx, &t);
        return -1;
    }
    return loft_crt_out(cx, &t, out, cap);
}

/* `write_text_raw(file, v)`: `v` at the File's position, through the runtime's own. */
int64_t loft_crt_write_text_raw(int64_t store, int64_t rec, int64_t pos, const char *v, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    return loft_write_text_raw(cx, loft_crt_ref(cx, store, rec, pos), loft_crt_str(cx, v, bytes));
}

/* `list_dir(path)`: the entries' names in a store of their own, into `out`; the null reference for
 * a path that is not a readable directory. */
int64_t loft_crt_list_dir(int64_t *out, int64_t no, const char *path, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_list_dir(cx, loft_crt_str(cx, path, bytes)));
    return 0;
}

/* `file_map(path)` (`Stores::fs_file_map`): the file's bytes as a read-only vector, null where
 * it cannot be mapped; its reference in `out`. */
int64_t loft_crt_file_map(int64_t *out, int64_t no, const char *path, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_file_map(cx, loft_crt_str(cx, path, bytes)));
    return 0;
}

/* The store natives by `which`: 0 `protect_store_frees`, 1 `unprotect_store_frees`, 2
 * `get_store_lock`, 3 `set_store_lock` (`arg` the lock), 4 `store_reclaim` — the runtime's own. */
int64_t loft_crt_store_native(int64_t which, int64_t store, int64_t rec, int64_t pos, int64_t arg) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    switch (which) {
    case 0: loft_protect(cx, r); return 0;
    case 1: loft_unprotect(cx, r); return 0;
    case 2: return loft_get_store_lock(cx, r);
    case 3: loft_set_store_lock(cx, r, (uint8_t) arg); return 0;
    case 4: return loft_store_reclaim(cx, r);
    default: return 0;
    }
}

/* The interpreted program's command line, one argument at a time after its name (the image):
 * what `arguments()` answers. */
void loft_crt_add_arg(const char *v, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    if (loft_argc == 0) {
        loft_argv = (char **) malloc(sizeof(char *));
        if (!loft_argv) { fputs("loft: out of memory\n", stderr); abort(); }
        loft_argv[0] = (char *) "";
        loft_argc = 1;
    }
    char **grown = (char **) realloc(loft_argv, sizeof(char *) * (size_t) (loft_argc + 1));
    char *copy = (char *) malloc((size_t) bytes + 1);
    if (!grown || !copy) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(copy, v, (size_t) bytes);
    copy[bytes] = 0;
    loft_argv = grown;
    loft_argv[loft_argc++] = copy;
}

/* `arguments()`: the program's arguments in a store of their own, into `out`. */
/* `panic(message, file, line)` reported at the PROGRAM's place: the runtime's own `loft_panic`
 * (a loft call to `panic` gets the caller's place injected by the compiler). */
void loft_crt_panic(const char *msg, int64_t mlen, const char *file, int64_t flen, int64_t line) {
    loft_cx *cx = &loft_main_cx;
    loft_panic(cx, loft_crt_str(cx, msg, mlen), loft_crt_str(cx, file, flen), line);
}

/* `env_variables()`: the environment as `EnvVariable` records of `size` bytes, the C runtime's
 * own (`loft_env_variables`), into `out`. */
int64_t loft_crt_env_variables(int64_t *out, int64_t no, int64_t size) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_env_variables(cx, (uint32_t) size));
    return 0;
}

int64_t loft_crt_arguments(int64_t *out, int64_t no) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_arguments(cx));
    return 0;
}

/* The bundle's type names by number, for the reports that name a store's type. */
static const char **loft_crt_tnames = 0;
static uint32_t loft_crt_ntnames = 0;

void loft_crt_add_type_name(const char *v, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    const char **grown = (const char **) realloc((void *) loft_crt_tnames, sizeof(char *) * (loft_crt_ntnames + 1));
    char *copy = (char *) malloc((size_t) bytes + 1);
    if (!grown || !copy) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(copy, v, (size_t) bytes);
    copy[bytes] = 0;
    loft_crt_tnames = grown;
    loft_crt_tnames[loft_crt_ntnames++] = copy;
}

int64_t loft_crt_type_names(void) {
    loft_cx *cx = &loft_main_cx;
    return loft_crt_ntnames;
}

/* `store_memory()`: the runtime's own report over the interpreted program's stores. */
int64_t loft_crt_store_memory(char *out, int64_t cap) {
    loft_cx *cx = &loft_main_cx;
    loft_text t = loft_store_memory(cx, loft_crt_tnames, loft_crt_ntnames);
    return loft_crt_out(cx, &t, out, cap);
}

/* ── S4: the keyed collections beyond a hash, by `kind` (1 a sorted, 2 an ordered, 3 an index),
 * each through the routine the C generator calls for that kind (`src/c/body/keyed.rs`), its keys
 * the loaded table's. */
static uint32_t loft_crt_size_of(loft_cx *cx, int64_t tp) {
    return loft_crt_keyed(cx, tp) ? loft_crt_types[tp].size : 0;
}

/* OpNewRecord: a sorted stages an element of `size` bytes past its end; an ordered or an index
 * gives it a record of its own.  Into `out`; the caller fills it with the defaults. */
int64_t loft_crt_keyed_new(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t kind, int64_t size) {
    loft_cx *cx = &loft_main_cx;
    loft_ref c = loft_crt_ref(cx, store, rec, pos);
    loft_crt_put_ref(cx, out, no, kind == 1 ? loft_sorted_new(cx, c, (uint32_t) size) : loft_entry_new(cx, c, (uint32_t) size));
    return 0;
}

/* OpFinishRecord: the record filed under its key, an equal key's element replaced and released
 * by the element type `elem`'s row. */
void loft_crt_keyed_finish(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                           int64_t r_pos, int64_t kind, int64_t elem, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp) || !loft_crt_keyed(cx, elem)) return;
    loft_ref c = loft_crt_ref(cx, store, rec, pos), r = loft_crt_ref(cx, r_store, r_rec, r_pos);
    const loft_key *keys = loft_crt_types[tp].keys;
    int nk = loft_crt_types[tp].nk;
    if (kind == 1) loft_sorted_finish(cx, c, loft_crt_size_of(cx, elem), keys, nk);
    else if (kind == 2) (void) loft_ordered_insert(cx, loft_crt_types, (uint16_t) elem, c, r, keys, nk, 0);
    else if (kind == 3) (void) loft_ix_insert(cx, loft_crt_types, (uint16_t) elem, c, r, keys, nk, 0);
    else (void) loft_rx_insert_record(cx, loft_crt_types, (uint16_t) elem, c, r, keys, nk, kind == 5, 0);
}

/* OpGetRecord: the element whose key is the top `n` parts, into `out`; null on a miss.  The parts
 * are popped either way. */
int64_t loft_crt_keyed_find(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                            int64_t kind, int64_t tp, int64_t n) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_ref_null(cx);
    if (loft_crt_keyed(cx, tp) && n >= 0 && n <= loft_crt_nkv) {
        loft_ref c = loft_crt_ref(cx, store, rec, pos);
        const loft_key *keys = loft_crt_types[tp].keys;
        int nk = loft_crt_types[tp].nk;
        const loft_kv *kv = &loft_crt_kv[loft_crt_nkv - n];
        if (kind == 1) r = loft_seq_find(cx, c, loft_crt_size_of(cx, loft_crt_types[tp].elem), keys, nk, kv, (int) n);
        else if (kind == 2) r = loft_seq_find(cx, c, 0, keys, nk, kv, (int) n);
        else if (kind == 3) r = loft_ix_find(cx, c, keys, nk, kv, (int) n);
        else r = loft_rx_find(cx, c, keys, nk, kind == 5, kv, (int) n);
    }
    loft_crt_key_pop(cx, n);
    loft_crt_put_ref(cx, out, no, r);
    return 0;
}

/* OpHashRemove on a sorted, an ordered or an index: the element leaves, what it owns released; a
 * view member (`view`) only unlinks. */
void loft_crt_keyed_remove(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                           int64_t r_pos, int64_t kind, int64_t elem, int64_t tp, int64_t view) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp) || !loft_crt_keyed(cx, elem)) return;
    loft_ref c = loft_crt_ref(cx, store, rec, pos), r = loft_crt_ref(cx, r_store, r_rec, r_pos);
    if (kind == 1) loft_sorted_remove_owned(cx, loft_crt_types, (uint16_t) elem, c, r, loft_crt_size_of(cx, elem));
    else if (kind == 2) loft_ordered_remove(cx, loft_crt_types, (uint16_t) elem, c, r, 0, (int) view);
    else if (kind == 3) loft_ix_remove(cx, loft_crt_types, (uint16_t) elem, c, r, loft_crt_types[tp].keys, loft_crt_types[tp].nk, (int) view);
    else loft_rx_remove_owned(cx, loft_crt_types, (uint16_t) elem, c, r, loft_crt_types[tp].keys, loft_crt_types[tp].nk, kind == 5, (int) view);
}

/* OpHashRemove on a group's vector member (an array of linked records): the record's slot
 * removed, as the C backend's `Keyed::Array` arm (`loft_ordered_remove` over the array); a view
 * member only unlinks. */
void loft_crt_array_remove(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec, int64_t r_pos,
                           int64_t elem, int64_t view) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, elem)) return;
    loft_ordered_remove(cx, loft_crt_types, (uint16_t) elem, loft_crt_ref(cx, store, rec, pos), loft_crt_ref(cx, r_store, r_rec, r_pos),
                        1, (int) view);
}

/* OpIndexGroup (`index_group_records`): every record the vector member at `prim` holds entered
 * into the view member `(vpos, vkind, vtp)`, then a record a keyed member answers another for
 * leaves every keyed member — `m` the keyed members' `(position, kind, type)` rows. */
void loft_crt_index_group(int64_t ps, int64_t pr, int64_t pp, int64_t prim, int64_t nullable, int64_t vpos,
                          int64_t vkind, int64_t vtp, const int64_t *m, int64_t nm) {
    loft_cx *cx = &loft_main_cx;
    int n = (int) (nm / 3);
    if (!loft_crt_types || n < 0 || n > 64 || !LOFT_CRT_TP_OK(vtp)) return;
    loft_member rows[64];
    for (int i = 0; i < n; i++) {
        int64_t tp = m[3 * i + 2];
        rows[i].pos = (uint32_t) m[3 * i];
        rows[i].kind = (int) m[3 * i + 1];
        rows[i].keys = LOFT_CRT_TP_OK(tp) ? loft_crt_types[tp].keys : 0;
        rows[i].nk = LOFT_CRT_TP_OK(tp) ? loft_crt_types[tp].nk : 0;
    }
    loft_member view = { (uint32_t) vpos, (int) vkind, loft_crt_types[vtp].keys, loft_crt_types[vtp].nk };
    loft_index_group(cx, loft_crt_ref(cx, ps, pr, pp), (uint32_t) prim, (int) nullable, &view, rows, n);
}

/* OpMoveChildRec (`Stores::move_child_rec`): the source record moved into the field's child. */
void loft_crt_move_child_rec(int64_t tp, int64_t fs, int64_t fr, int64_t fp, int64_t ss, int64_t sr, int64_t sp) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_move_child_rec(cx, loft_crt_types, (uint16_t) tp, loft_crt_ref(cx, fs, fr, fp), loft_crt_ref(cx, ss, sr, sp));
}

/* The slot cells (`@FR-F-ParamRef`): a reference cell that names either a value or, tagged, a
 * field whose child it stands for. */

/* A cell's reference read and written raw, as Rust's slot operators do (`read::<DbRef>`). */
static loft_ref loft_crt_cell_get(loft_cx *cx, loft_ref at, uint32_t fld) {
    loft_ref x = loft_ref_null(cx);
    if (at.rec == 0 || at.store_nr >= cx->nstores) return x;
    loft_store_t *s = loft_store_of(cx, at);
    x.store_nr = (uint16_t) loft_u32(cx, s, at.rec, at.pos + fld);
    x.rec = loft_u32(cx, s, at.rec, at.pos + fld + 4);
    x.pos = loft_u32(cx, s, at.rec, at.pos + fld + 8);
    return x;
}

static void loft_crt_cell_set(loft_cx *cx, loft_ref at, loft_ref x) {
    if (at.rec == 0 || at.store_nr >= cx->nstores) return;
    loft_store_t *s = loft_store_of(cx, at);
    loft_set_u32(cx, s, at.rec, at.pos, x.store_nr);
    loft_set_u32(cx, s, at.rec, at.pos + 4, x.rec);
    loft_set_u32(cx, s, at.rec, at.pos + 8, x.pos);
}

/* OpGetSlotRef: what the cell at field `fld` of `(s, r, p)` reads as (`slot_read`). */
int64_t loft_crt_slot_get(int64_t *out, int64_t no, int64_t s, int64_t r, int64_t p, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    loft_crt_put_ref(cx, out, no, loft_slot_read(cx, loft_crt_cell_get(cx, loft_crt_ref(cx, s, r, p), (uint32_t) fld)));
    return 0;
}

/* `place_ref`'s answer — the reference stored at `(store, rec, pos)` — a part at a time, and
 * written at the address `to` (`OpGetStackRef`'s arm). */
int64_t loft_crt_place_ref_part(int64_t store, int64_t rec, int64_t pos, int64_t part) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    uint32_t v = loft_u32(cx, s, (uint32_t) rec, (uint32_t) (pos + part * 4));
    return part == 0 ? (int64_t) (uint16_t) v : (int64_t) v;
}
extern void loft_twin_place_ref_to(int64_t to, int64_t store, int64_t rec, int64_t pos);
LOFT_STEP_FN void loft_twin_place_ref_to(int64_t to, int64_t store, int64_t rec, int64_t pos) {
    loft_cx *cx = &loft_main_cx;
    loft_store_t *s = &cx->stores[store];
    loft_twin_poke_ref(to, (uint16_t) loft_u32(cx, s, (uint32_t) rec, (uint32_t) pos),
                       loft_u32(cx, s, (uint32_t) rec, (uint32_t) pos + 4),
                       loft_u32(cx, s, (uint32_t) rec, (uint32_t) pos + 8));
}

/* `slot_get`'s answer a part at a time (0 store, 1 record, 2 position), and written at the
 * address `to` (`OpGetSlotRef`'s arm). */
int64_t loft_crt_slot_get_part(int64_t part, int64_t s, int64_t r, int64_t p, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    return loft_crt_slot_part(cx, loft_slot_read(cx, loft_crt_cell_get(cx, loft_crt_ref(cx, s, r, p), (uint32_t) fld)), part);
}
extern void loft_twin_slot_get_to(int64_t to, int64_t s, int64_t r, int64_t p, int64_t fld);
LOFT_STEP_FN void loft_twin_slot_get_to(int64_t to, int64_t s, int64_t r, int64_t p, int64_t fld) {
    loft_cx *cx = &loft_main_cx;
    loft_ref v = loft_slot_read(cx, loft_crt_cell_get(cx, loft_crt_ref(cx, s, r, p), (uint32_t) fld));
    loft_twin_poke_ref(to, v.store_nr, v.rec, v.pos);
}

/* OpMoveSlotRef: `slot = slot.f` through the cell at `(s, r, p)` (`slot_move`). */
void loft_crt_slot_move(int64_t tp, int64_t s, int64_t r, int64_t p, int64_t ss, int64_t sr, int64_t sp) {
    loft_cx *cx = &loft_main_cx;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref at = loft_crt_ref(cx, s, r, p);
    loft_ref cell = loft_crt_cell_get(cx, at, 0);
    loft_slot_move(cx, loft_crt_types, (uint16_t) tp, &cell, loft_crt_ref(cx, ss, sr, sp));
    loft_crt_cell_set(cx, at, cell);
}

/* OpSetSlotRef: a whole write through the cell at `(s, r, p)` (`slot_write`); the type's top bit
 * frees the source after. */
void loft_crt_slot_set(int64_t raw, int64_t s, int64_t r, int64_t p, int64_t vs, int64_t vr, int64_t vp) {
    loft_cx *cx = &loft_main_cx;
    int64_t tp = raw & 0x3FFF;
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref at = loft_crt_ref(cx, s, r, p);
    loft_ref cell = loft_crt_cell_get(cx, at, 0);
    loft_slot_write(cx, loft_crt_types, (uint16_t) tp, &cell, loft_crt_ref(cx, vs, vr, vp), (raw & 0x8000) != 0);
    loft_crt_cell_set(cx, at, cell);
}

/* A walk's key list (`OpIterate`'s `Keys`), one part at a time, for the iterate that follows. */
static loft_key loft_crt_wkeys[LOFT_CRT_KEYS];
static int loft_crt_nwkeys = 0;

void loft_crt_walk_key(int64_t type, int64_t pos, int64_t start) {
    loft_cx *cx = &loft_main_cx;
    if (loft_crt_nwkeys >= LOFT_CRT_KEYS) return;
    loft_crt_wkeys[loft_crt_nwkeys].type = (int8_t) type;
    loft_crt_wkeys[loft_crt_nwkeys].pos = (uint16_t) pos;
    loft_crt_wkeys[loft_crt_nwkeys].start = (int32_t) start;
    loft_crt_nwkeys++;
}

/* OpIterate over an index (`on & 63` 1), a sorted (2, `arg` its element size) or an ordered (3):
 * the walk's first state, its bounds the top `nfrom` + `ntill` parts (popped), its keys the list
 * above (cleared). */
int64_t loft_crt_keyed_iterate(int64_t store, int64_t rec, int64_t pos, int64_t on, int64_t arg,
                               int64_t nfrom, int64_t ntill) {
    loft_cx *cx = &loft_main_cx;
    int64_t state = 0;
    int64_t n = nfrom + ntill;
    if (n >= 0 && n <= loft_crt_nkv) {
        loft_ref c = loft_crt_ref(cx, store, rec, pos);
        const loft_kv *from = &loft_crt_kv[loft_crt_nkv - n];
        const loft_kv *till = from + nfrom;
        int ex = (on & 128) == 0, reverse = (on & 64) != 0;
        switch (on & 63) {
        case 1: state = loft_ix_iterate(cx, c, ex, reverse, loft_crt_wkeys, loft_crt_nwkeys, from, (int) nfrom, till, (int) ntill); break;
        case 2: state = loft_seq_iterate(cx, c, (uint32_t) arg, ex, loft_crt_wkeys, loft_crt_nwkeys, from, (int) nfrom, till, (int) ntill); break;
        case 3: state = loft_seq_iterate(cx, c, 0, ex, loft_crt_wkeys, loft_crt_nwkeys, from, (int) nfrom, till, (int) ntill); break;
        default: break;
        }
    }
    loft_crt_key_pop(cx, n);
    loft_crt_nwkeys = 0;
    return state;
}

/* OpStep of such a walk: the next element into `out[0..3]`, the state it leaves into `out[3]`;
 * `arg` is the index type (its keys find the record again) or the sorted's element size. */
int64_t loft_crt_keyed_step(int64_t *out, int64_t no, int64_t state, int64_t store, int64_t rec,
                            int64_t pos, int64_t on, int64_t arg) {
    loft_cx *cx = &loft_main_cx;
    loft_ref c = loft_crt_ref(cx, store, rec, pos), r = loft_ref_null(cx);
    int reverse = (on & 64) != 0;
    switch (on & 63) {
    case 1:
        if (loft_crt_keyed(cx, arg)) r = loft_ix_step(cx, &state, c, loft_crt_types[arg].keys, loft_crt_types[arg].nk, reverse);
        break;
    case 2: r = loft_seq_step(cx, &state, c, (uint32_t) arg, reverse); break;
    case 3: r = loft_seq_step(cx, &state, c, 0, reverse); break;
    default: break;
    }
    loft_crt_put_ref(cx, out, no, r);
    if (no >= 4) out[3] = state;
    return 0;
}

/* By `which`: OpLengthIndex (0), OpSizeIndex (1), OpLengthSpatial / OpLengthTrie (2), OpSizeSpatial
 * (3) — the records, or the table's bytes. */
int64_t loft_crt_index_len(int64_t store, int64_t rec, int64_t pos, int64_t which) {
    loft_cx *cx = &loft_main_cx;
    loft_ref r = loft_crt_ref(cx, store, rec, pos);
    switch (which) {
    case 1: return (int64_t) loft_bt_table_bytes(cx, r);
    case 2: return (int64_t) loft_rx_count(cx, r);
    case 3: return (int64_t) loft_rx_table_bytes(cx, r);
    default: return (int64_t) loft_bt_len(cx, r);
    }
}

/* A spatial or trie walk's snapshot, into `out`: `which` 0 all of it in key order, 1 the range
 * `ints[0..8]` of spatial type `tp`, 2 the elements under prefix `pre` of trie type `tp`, at most
 * `ints[0]`. */
int64_t loft_crt_radix_snap(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                            int64_t which, int64_t tp, const int64_t *ints, int64_t nints,
                            const char *pre, int64_t pre_bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_ref c = loft_crt_ref(cx, store, rec, pos), r = loft_ref_null(cx);
    if (which == 0) r = loft_radix_sorted(cx, c);
    else if (which == 1 && nints >= 8 && loft_crt_keyed(cx, tp))
        r = loft_spatial_range(cx, c, loft_crt_types[tp].keys, loft_crt_types[tp].nk, ints[0], ints[1], ints[2],
                               ints[3], ints[4], ints[5], ints[6], ints[7]);
    else if (which == 2 && nints >= 1 && loft_crt_keyed(cx, tp))
        r = loft_trie_prefix(cx, c, loft_crt_types[tp].keys, loft_crt_str(cx, pre, pre_bytes), ints[0]);
    loft_crt_put_ref(cx, out, no, r);
    return 0;
}

/* OpRemove (`e#remove` in a walk): the element the walk visited last leaves its collection, what
 * it owns released, and the walk goes on with the next.  `on & 63` the walk's kind — 0 a vector
 * (an array when `tp`'s records are linked), 1 an index (`tp` the index type), 2 a sorted, 3 an
 * ordered (`tp` the element type) — and the state it leaves is answered. */
int64_t loft_crt_walk_remove(int64_t state, int64_t store, int64_t rec, int64_t pos, int64_t on, int64_t tp) {
    loft_cx *cx = &loft_main_cx;
    if (!loft_crt_keyed(cx, tp)) return state;
    loft_ref d = loft_crt_ref(cx, store, rec, pos);
    int reverse = (on & 64) != 0;
    uint16_t t = (uint16_t) tp;
    switch (on & 63) {
    case 0:
        if (loft_crt_types[tp].linked) loft_array_walk_remove(cx, loft_crt_types, t, &state, d, reverse);
        else loft_vec_walk_remove(cx, loft_crt_types, t, &state, d, reverse);
        break;
    case 1:
        loft_ix_walk_remove(cx, loft_crt_types, loft_crt_types[tp].elem, &state, d, loft_crt_types[tp].keys,
                            loft_crt_types[tp].nk, reverse);
        break;
    case 2: loft_sorted_walk_remove(cx, loft_crt_types, t, &state, d, reverse); break;
    case 3: loft_ordered_walk_remove(cx, loft_crt_types, t, &state, d, reverse); break;
    default: break;
    }
    return state;
}

/* ── The lexer (@PLN91 stage 1): loft source text as the tokens src/lexer.rs reads from it, for the
 * front end written in loft.  `loft_lex` is the port (src/c/loft_lex.h); this only encodes its
 * tokens for the bridge, one record each, little-endian:
 *   kind u8 · line u32 · col u32 · end line u32 · end col u32 (the scan cursor after the
 *   token) · then by kind:
 *   Ident, Str, StrHole  len u32 · its UTF-8 bytes
 *   Int                  value u32 · zero u8 (spelled with a leading `0`)
 *   Long, Wide           value u64
 *   Char                 code point u32
 *   Float                zero u8 · len u8 · the value as `%.17g` (exact: it reads back as itself)
 *   Double, Single       len u8 · the value as `%.17g` (a single's value is exact as a double)
 *   every other kind     nothing
 * The last record is `End` (kind 88), so a reader needs no count. */
#define LOFT_LEX 1
#include "../../../src/c/loft_lex.h"

static void loft_crt_lex_u32(loft_cx *cx, loft_lx_buf *e, uint64_t v, int bytes) {
    for (int i = 0; i < bytes; i++) loft_lx_buf_byte(cx, e, (uint8_t) (v >> (8 * i)));
}

static void loft_crt_lex_float(loft_cx *cx, loft_lx_buf *e, double v) {
    char s[40];
    int n = snprintf(s, sizeof s, "%.17g", v);
    loft_lx_buf_byte(cx, e, (uint8_t) n);
    loft_lx_buf_put(cx, e, s, (size_t) n);
}

int64_t loft_crt_lex(char *out, int64_t cap, const char *src, int64_t bytes) {
    loft_cx *cx = &loft_main_cx;
    loft_lx_toks toks = {0};
    loft_lex(cx, (const uint8_t *) src, bytes < 0 ? 0 : (size_t) bytes, &toks);
    loft_lx_buf e = {0};
    for (size_t i = 0; i < toks.n; i++) {
        const loft_lx_tok *t = &toks.t[i];
        loft_lx_buf_byte(cx, &e, t->kind);
        loft_crt_lex_u32(cx, &e, t->line, 4);
        loft_crt_lex_u32(cx, &e, t->col, 4);
        loft_crt_lex_u32(cx, &e, t->eline, 4);
        loft_crt_lex_u32(cx, &e, t->ecol, 4);
        switch (t->kind) {
        case LOFT_LX_IDENT: case LOFT_LX_STR: case LOFT_LX_STRHOLE:
            loft_crt_lex_u32(cx, &e, t->s_len, 4);
            loft_lx_buf_put(cx, &e, toks.txt.p + t->s_at, t->s_len);
            break;
        case LOFT_LX_INT:
            loft_crt_lex_u32(cx, &e, (uint64_t) t->i, 4);
            loft_lx_buf_byte(cx, &e, t->zero);
            break;
        case LOFT_LX_LONG: case LOFT_LX_WIDE: loft_crt_lex_u32(cx, &e, (uint64_t) t->i, 8); break;
        case LOFT_LX_CHAR: loft_crt_lex_u32(cx, &e, (uint64_t) t->i, 4); break;
        case LOFT_LX_FLOAT:
            loft_lx_buf_byte(cx, &e, t->zero);
            loft_crt_lex_float(cx, &e, t->f);
            break;
        case LOFT_LX_DOUBLE: case LOFT_LX_SINGLE: loft_crt_lex_float(cx, &e, t->f); break;
        default: break;
        }
    }
    loft_lx_toks_free(cx, &toks);
    int64_t n = (int64_t) e.len;
    if (n <= cap && n > 0) memcpy(out, e.p, (size_t) n);
    free(e.p);
    return n;
}

/* ── Compiled standard-library bodies (@PLN91; src/c/bodies.rs) ─────────────────────────────
 * `loft_bodies.c` is the C writer's code for the standard-library functions the Rust build also
 * compiles, with one entry each that reads its arguments from the runner's stack as the
 * bytecode's call leaves them.  It is compiled here because the program's stores and types are
 * this unit's: every runtime global is private to its C unit.  loft writes it into the
 * directory this shim compiles in, from the building binary (`c_shim::put_bodies`), so it is
 * never a file of this package: angle brackets, so a stray copy beside this file cannot shadow
 * it.  `-DLOFT_CRT_NO_BODIES` leaves it out (a compile outside loft): every lookup then answers
 * -1 and the runner runs each function's loft body. */
#ifndef LOFT_CRT_NO_BODIES
#include <loft_bodies.c>
#else
int64_t loft_body_find(const char *name, int64_t len) {
    (void) name;
    (void) len;
    return -1;
}
int64_t loft_body_call(int64_t entry, int64_t stk, int64_t sp) {
    (void) entry;
    (void) stk;
    (void) sp;
    return -1;
}
int64_t loft_body_call_dest(int64_t entry, int64_t stk, int64_t sp, int64_t s, int64_t r, int64_t p) {
    (void) entry;
    (void) stk;
    (void) sp;
    (void) s;
    (void) r;
    (void) p;
    return -1;
}
#endif

/* ── The engine-host kernel for the runner (`lib/engine_host`, `loft_engine.h`) ────────────────
 * One entry for every kernel routine that answers a scalar, selected by `which` (the runner's
 * `natives::KERNEL_*` order): its integer operands `a`, `b` and its text operand. */
int64_t loft_crt_kernel(int64_t which, int64_t a, int64_t b, const char *s, int64_t slen) {
    loft_cx *cx = &loft_main_cx;
    loft_str t = loft_crt_str(cx, s, slen);
    switch (which) {
    case 0: return loft_kernel_listen(cx, a, b);
    case 1: return loft_kernel_pump(cx);
    case 2: return loft_kernel_next_event(cx);
    case 3: return loft_kernel_event_cid(cx);
    case 4: return loft_kernel_event_kind(cx);
    case 5: return loft_kernel_event_status(cx);
    case 6: return loft_kernel_tick_due(cx);
    case 7: return loft_kernel_send(cx, a, t);
    case 8: return loft_kernel_broadcast(cx, t);
    case 9: loft_kernel_idle(cx, a); return 0;
    case 10: return loft_kernel_clients(cx);
    case 11: return loft_kernel_alive(cx);
    case 12: loft_kernel_stop(cx); return 0;
    case 13: loft_kernel_frame(cx); return 0;
    case 14: return loft_kernel_post(cx, t);
    case 15: return loft_kernel_udp_bound(cx, a);
    case 16: loft_kernel_sync_class(cx, a); return 0;
    case 17: loft_kernel_sync_class_keyed(cx, a); return 0;
    case 18: return loft_kernel_keyframe(cx, a, t);
    case 19: return loft_kernel_sync_next(cx);
    case 20: return loft_kernel_sync_cid(cx);
    case 21: return loft_kernel_sync_seq(cx);
    case 22: return loft_kernel_local(cx, a);
    case 23: return loft_kernel_connect(cx, t, a, b);
    case 24: return loft_kernel_client_pump(cx);
    case 25: return loft_kernel_client_alive(cx);
    case 26: loft_kernel_client_stop(cx); return 0;
    case 27: return loft_kernel_client_next_event(cx);
    case 28: return loft_kernel_client_event_kind(cx);
    case 29: return loft_kernel_client_event_cid(cx);
    case 30: return loft_kernel_client_event_status(cx);
    case 31: return loft_kernel_client_tick_due(cx);
    case 32: loft_kernel_client_idle(cx, a); return 0;
    case 33: return loft_kernel_client_send(cx, t);
    case 34: return loft_kernel_client_sync_next(cx);
    case 35: return loft_kernel_client_sync_seq(cx);
    case 36: return loft_kernel_client_udp_bound(cx);
    case 37: loft_kernel_client_frame(cx); return 0;
    case 38: return loft_kernel_swap_step(cx);
    default: return LOFT_NULL;
    }
}

/* A kernel routine that answers a text (`which`: the event payload, the sync payload, the
 * client's two, the default host), its bytes into `out` (`cap` bytes): its length, which the
 * caller reads again with a larger buffer when it exceeds `cap`; -1 for the null text. */
int64_t loft_crt_kernel_text(char *out, int64_t cap, int64_t which) {
    loft_cx *cx = &loft_main_cx;
    loft_text v;
    switch (which) {
    case 0: v = loft_kernel_event_payload(cx); break;
    case 1: v = loft_kernel_sync_payload(cx); break;
    case 2: v = loft_kernel_client_event_payload(cx); break;
    case 3: v = loft_kernel_client_sync_payload(cx); break;
    case 4: v = loft_kernel_default_host(cx); break;
    default: return -1;
    }
    loft_str w = loft_view(cx, &v);
    int64_t n = loft_str_is_null(cx, w) ? -1 : (int64_t) w.len;
    if (n > 0) memcpy(out, w.p, (size_t) (n < cap ? n : cap));
    loft_text_free(cx, &v);
    return n;
}

/* `http_fetch(method, url, body, headers)`: the request's id, its completion a kind-3 event. */
int64_t loft_crt_kernel_fetch(const char *m, int64_t ml, const char *u, int64_t ul, const char *b, int64_t bl,
                              const char *h, int64_t hl) {
    loft_cx *cx = &loft_main_cx;
    return loft_kernel_http_fetch(cx, loft_crt_str(cx, m, ml), loft_crt_str(cx, u, ul), loft_crt_str(cx, b, bl), loft_crt_str(cx, h, hl));
}
