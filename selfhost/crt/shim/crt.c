/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The `crt` library's shim (@PLN91 S2.2): the C runtime's own format routines, reached by
 * `#c` bindings on every backend.  Each function here only MARSHALS — loft's `#c` model
 * passes plain C (a text out as a caller's byte buffer, a float through a pointer, a null as
 * a flag) — and calls one routine the C runtime already defines, so a format the loft
 * interpreter renders is the C runtime's rendering.  Each answers the rendering's length in
 * bytes; when that exceeds `cap`, nothing is written and the caller asks again with room. */
/* The allocator trace of the interpreted program's stores (@PLN91 1b-compare.md, the alloc
 * layer): this copy of the runtime writes the event lines `LOFT_ALLOC_TRACE` writes, to the file
 * `LOFT_CRT_ALLOC_TRACE` names, so the interpreter's own stores never mix into them. */
#define LOFT_ALLOC_TRACE 1
#define LOFT_ALLOC_TRACE_ENV "LOFT_CRT_ALLOC_TRACE"
#include "../../../src/c/loft_crt.h"

/* A text that crossed the bridge: `bytes` long (an embedded NUL kept), or null when -1. */
static loft_str loft_crt_str(const char *val, int64_t bytes) {
    return bytes < 0 ? loft_null_text() : loft_lit(val, (size_t) bytes);
}

static int64_t loft_crt_out(loft_text *t, char *out, int64_t cap) {
    loft_str v = loft_view(t);
    int64_t n = (int64_t) v.len;
    if (n <= cap && n > 0) {
        memcpy(out, v.p, (size_t) n);
    }
    free(t->p);
    return n;
}

int64_t loft_crt_format_int(char *out, int64_t cap, int64_t val, int64_t is_null, int64_t radix,
                            int64_t width, int64_t token, int64_t plus, int64_t note, int64_t dir) {
    loft_text t = {0};
    loft_format_long(&t, is_null ? LOFT_NULL : val, (uint8_t) radix, width, (uint8_t) token,
                     (int) plus, (int) note, (int) dir);
    return loft_crt_out(&t, out, cap);
}

int64_t loft_crt_format_float(char *out, int64_t cap, const double *val, int64_t n_val,
                              int64_t is_null, int64_t width, int64_t prec, int64_t token,
                              int64_t plus, int64_t dir, int64_t single) {
    loft_text t = {0};
    double v = (is_null || n_val < 1) ? NAN : val[0];
    loft_format_float(&t, v, width, prec, (uint8_t) token, (int) plus, (int) dir, (int) single);
    return loft_crt_out(&t, out, cap);
}

int64_t loft_crt_format_text(char *out, int64_t cap, const char *val, int64_t bytes,
                             int64_t width, int64_t dir, int64_t token) {
    loft_text t = {0};
    loft_str v = loft_crt_str(val, bytes);
    loft_format_text(&t, v, width, (int) dir, (uint8_t) token);
    return loft_crt_out(&t, out, cap);
}

/* ── The store (@PLN91 S3): the interpreted program's records live in this shim's own copy of
 * the C runtime's stores.  A reference crosses `#c` as its three parts (store, rec, pos), and
 * a routine that answers one writes them into the caller's three-integer vector.  Each
 * function marshals and calls one runtime routine; none derives a layout. */

static loft_ref loft_crt_ref(int64_t store, int64_t rec, int64_t pos) {
    loft_ref r;
    r.store_nr = (uint16_t) store;
    r.rec = (uint32_t) rec;
    r.pos = (uint32_t) pos;
    return r;
}

static void loft_crt_put_ref(int64_t *out, int64_t n, loft_ref r) {
    if (n < 3) return;
    out[0] = r.store_nr;
    out[1] = r.rec;
    out[2] = r.pos;
}

/* The variable a routine writes: its three parts in `io`, read before and written after. */
static loft_ref loft_crt_get_ref(const int64_t *io, int64_t n) {
    return n < 3 ? loft_ref_null() : loft_crt_ref(io[0], io[1], io[2]);
}

/* OpDatabase of a record type, on the variable in `io`: a store it already names is cleared and
 * made again in place, any other variable gets a fresh store; the root is a zeroed record of
 * `bytes`. */
int64_t loft_crt_database(int64_t *io, int64_t n, int64_t bytes, int64_t tp) {
    loft_ref r = loft_crt_get_ref(io, n);
    loft_database_typed(&r, (uint32_t) bytes, (uint16_t) tp);
    loft_crt_put_ref(io, n, r);
    return 0;
}

/* ── @PLN91 F-1: the interpreter's stack store (`State::stack_cur`) ────────────────────────
 * One store whose primary record holds every frame, each variable at the byte the Rust
 * interpreter keeps it at: frame byte `at` is field `at` of `(store, 1, 8)`.  The store holds
 * nothing but that record, which spans the whole store. */

/* `Store::extend_primary_to_store_end`: record 1 takes the rest of the store, so no free tail
 * is left for the allocator to hand out. */
static void loft_crt_stack_span(loft_store_t *s) {
    loft_set_u32(s, LOFT_ST_PRIMARY, 0, s->size - LOFT_ST_PRIMARY);
    s->wild = 0;
    s->free_root = 0;
    s->claimed_end = s->size;
}

/* `database_defined`: a fresh stack store of at least `bytes` of frame, pinned to its slot. */
int64_t loft_crt_stack_new(int64_t *io, int64_t n, int64_t bytes) {
    loft_ref r = loft_ref_null();
    loft_database(&r, (uint32_t) bytes);
    loft_store_t *s = &loft_stores[r.store_nr];
    s->pinned = 1;
    loft_crt_stack_span(s);
    loft_crt_put_ref(io, n, r);
    return 0;
}

/* `State::grow_stack`: the store grown until record 1 holds `bytes` of frame. */
void loft_crt_stack_grow(int64_t store, int64_t bytes) {
    loft_store_t *s = &loft_stores[store];
    uint32_t words = (uint32_t) ((8 + bytes + 7) / 8) + 1;
    if (s->size >= LOFT_ST_PRIMARY + words) return;
    loft_resize_store(s, LOFT_ST_PRIMARY + words);
    loft_crt_stack_span(s);
}

/* ── F-1b: a text variable in its slot, as the Rust frame holds it: a local a `loft_text` (24
 * bytes, a `String`), an argument a `loft_str` view (16 bytes, a `&str`) of a copy the call made
 * and its return releases. */
static uint8_t *loft_crt_slot_at(int64_t store, int64_t at) {
    return loft_stores[store].buf + (size_t) LOFT_ST_PRIMARY * 8 + 8 + (size_t) at;
}

/* The text a slot holds — a local's (`arg` 0) or an argument's (1) — copied into `out` when it
 * fits: its length, or -1 for the null text. */
int64_t loft_crt_slot_text(char *out, int64_t cap, int64_t store, int64_t at, int64_t arg) {
    loft_str v;
    if (arg) {
        loft_str *a = (loft_str *) loft_crt_slot_at(store, at);
        v = a->p ? *a : loft_lit("", 0);
    } else {
        v = loft_view((loft_text *) loft_crt_slot_at(store, at));
    }
    if (loft_str_is_null(v)) return -1;
    int64_t len = (int64_t) v.len;
    if (len <= cap && len > 0) memcpy(out, v.p, (size_t) len);
    return len;
}

/* A local's slot as `OpInitText` leaves it: the empty text, no buffer. */
void loft_crt_slot_text_init(int64_t store, int64_t at) {
    memset(loft_crt_slot_at(store, at), 0, sizeof(loft_text));
}

/* A local set to `val` (or the null text). */
void loft_crt_set_slot_text(int64_t store, int64_t at, const char *val, int64_t bytes) {
    loft_text *t = (loft_text *) loft_crt_slot_at(store, at);
    loft_text_set(t, loft_crt_str(val, bytes));
}

/* `val` appended to a local; a null local becomes `val`, as the empty text would. */
void loft_crt_slot_text_append(int64_t store, int64_t at, const char *val, int64_t bytes) {
    loft_text *t = (loft_text *) loft_crt_slot_at(store, at);
    loft_str s = loft_crt_str(val, bytes);
    if (loft_str_is_null(loft_view(t))) loft_text_set(t, s);
    else loft_text_append(t, s);
}

/* `OpClearText`: the local emptied, its buffer kept. */
void loft_crt_slot_text_clear(int64_t store, int64_t at) {
    loft_text_set((loft_text *) loft_crt_slot_at(store, at), loft_lit("", 0));
}

/* `OpFreeText`: the local's buffer released; the slot reads as the empty text. */
void loft_crt_slot_text_free(int64_t store, int64_t at) {
    loft_text_free((loft_text *) loft_crt_slot_at(store, at));
}

/* An argument's slot: a view of a copy of `val` (the null text's one byte too). */
void loft_crt_set_slot_arg(int64_t store, int64_t at, const char *val, int64_t bytes) {
    loft_str v = loft_crt_str(val, bytes);
    char *copy = (char *) malloc(v.len + 1);
    if (!copy) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(copy, v.p, v.len);
    copy[v.len] = 0;
    loft_str *a = (loft_str *) loft_crt_slot_at(store, at);
    a->p = copy;
    a->len = v.len;
}

/* The call's return: the argument's copy released. */
void loft_crt_slot_arg_free(int64_t store, int64_t at) {
    loft_str *a = (loft_str *) loft_crt_slot_at(store, at);
    free((char *) a->p);
    a->p = 0;
    a->len = 0;
}

/* The run's end: the stack store released (a pinned store keeps itself from `loft_free_ref`). */
void loft_crt_stack_free(int64_t store) {
    loft_store_t *s = &loft_stores[store];
    s->pinned = 0;
    loft_free_ref(loft_crt_ref(store, LOFT_ST_PRIMARY, 8));
}

/* A constant's store built (`consts.loft`): pinned and read-only, as `Store::lock_constant` and
 * `loft_const_store` leave it. */
void loft_crt_lock_constant(int64_t store) {
    loft_store_t *s = &loft_stores[store];
    s->pinned = 1;
    s->read_only = 1;
}

/* A constant's store released by the run that built it: unpinned and writable first, as the
 * stack store is. */
void loft_crt_free_constant(int64_t store) {
    loft_store_t *s = &loft_stores[store];
    s->pinned = 0;
    s->read_only = 0;
    loft_free_ref(loft_crt_ref(store, LOFT_ST_PRIMARY, 8));
}

/* The reference a PLACE holds — a frame slot (`(stack, 1, 8 + at)`), a field, an element —
 * read and written as `OpGetStackRef` / `OpSetStackRef` do: twelve raw bytes at `(rec, pos)`,
 * no record marked and no tombstone read through (`loft_get_dbref` does both, for a reference
 * FIELD read as a value). */
int64_t loft_crt_place_ref(int64_t *out, int64_t n, int64_t store, int64_t rec, int64_t pos) {
    loft_store_t *s = &loft_stores[store];
    loft_ref x;
    x.store_nr = (uint16_t) loft_u32(s, (uint32_t) rec, (uint32_t) pos);
    x.rec = loft_u32(s, (uint32_t) rec, (uint32_t) pos + 4);
    x.pos = loft_u32(s, (uint32_t) rec, (uint32_t) pos + 8);
    loft_crt_put_ref(out, n, x);
    return 0;
}

void loft_crt_set_place_ref(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                            int64_t r_pos) {
    loft_store_t *s = &loft_stores[store];
    loft_set_u32(s, (uint32_t) rec, (uint32_t) pos, (uint32_t) r_store);
    loft_set_u32(s, (uint32_t) rec, (uint32_t) pos + 4, (uint32_t) r_rec);
    loft_set_u32(s, (uint32_t) rec, (uint32_t) pos + 8, (uint32_t) r_pos);
}

/* OpFreeRef of a variable: its store released, the variable reset to the sentinel. */
void loft_crt_free_var(int64_t *io, int64_t n) {
    loft_ref r = loft_crt_get_ref(io, n);
    loft_free_var(&r);
    loft_crt_put_ref(io, n, r);
}

/* OpFreeRefIfDistinct of a variable: released and reset unless it names the witness's store. */
void loft_crt_free_var_distinct(int64_t *io, int64_t n, int64_t w_store, int64_t w_rec,
                                int64_t w_pos) {
    loft_ref r = loft_crt_get_ref(io, n);
    loft_free_var_distinct(&r, loft_crt_ref(w_store, w_rec, w_pos));
    loft_crt_put_ref(io, n, r);
}

void loft_crt_free_ref(int64_t store, int64_t rec, int64_t pos) {
    loft_free_ref(loft_crt_ref(store, rec, pos));
}

/* OpFreeRefIfDistinct of two values (`Stores::free_displaced`): the displaced store released
 * unless it is the witness's, absent, or protected by a call in flight. */
void loft_crt_free_displaced(int64_t d_store, int64_t d_rec, int64_t d_pos, int64_t w_store,
                             int64_t w_rec, int64_t w_pos) {
    loft_free_displaced(loft_crt_ref(d_store, d_rec, d_pos), loft_crt_ref(w_store, w_rec, w_pos));
}

/* n_protect_store_frees / n_unprotect_store_frees: the call bracket around a reference
 * argument. */
void loft_crt_protect(int64_t store, int64_t rec, int64_t pos, int64_t on) {
    if (on) loft_protect(loft_crt_ref(store, rec, pos));
    else loft_unprotect(loft_crt_ref(store, rec, pos));
}

int64_t loft_crt_get_int(int64_t store, int64_t rec, int64_t pos, int64_t fld) {
    return loft_get_int(loft_crt_ref(store, rec, pos), (uint32_t) fld);
}

void loft_crt_set_int(int64_t store, int64_t rec, int64_t pos, int64_t fld, int64_t v,
                      int64_t is_null) {
    loft_set_int(loft_crt_ref(store, rec, pos), (uint32_t) fld, is_null ? LOFT_NULL : v);
}

/* OpGetText: the text copied into the caller's buffer; answers its length in bytes, -1 for
 * null, and writes nothing when the length exceeds `cap`. */
int64_t loft_crt_get_text(char *out, int64_t cap, int64_t store, int64_t rec, int64_t pos,
                          int64_t fld) {
    loft_str v = loft_get_text(loft_crt_ref(store, rec, pos), (uint32_t) fld);
    if (loft_str_is_null(v)) return -1;
    int64_t len = (int64_t) v.len;
    if (len <= cap && len > 0) memcpy(out, v.p, (size_t) len);
    return len;
}

void loft_crt_set_text(int64_t store, int64_t rec, int64_t pos, int64_t fld, const char *val,
                       int64_t bytes) {
    loft_str v = loft_crt_str(val, bytes);
    loft_set_text(loft_crt_ref(store, rec, pos), (uint32_t) fld, v);
}

/* OpSetTextReplace: the text field refilled in place where its claim has room. */
void loft_crt_refill_text(int64_t store, int64_t rec, int64_t pos, int64_t fld, const char *val,
                          int64_t bytes) {
    loft_str v = loft_crt_str(val, bytes);
    loft_refill_text(loft_crt_ref(store, rec, pos), (uint32_t) fld, v);
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
    loft_ref r = loft_crt_ref(store, rec, pos);
    uint32_t f = (uint32_t) fld;
    switch (which) {
    case 0: return loft_get_int4(r, f);
    case 1: return loft_get_byte(r, f, min);
    case 2: return loft_get_byte_nullable(r, f, min);
    case 3: return loft_get_short(r, f, min);
    case 4: return loft_get_short_raw(r, f, min);
    case 5: return (int64_t) loft_get_char(r, f);
    case 6: {
        uint8_t b = loft_get_bool(r, f);
        return b == LOFT_BOOL_NULL ? LOFT_NULL : (int64_t) b;
    }
    case 7: {
        uint8_t e = loft_get_enum(r, f);
        return e == 255 ? LOFT_NULL : (int64_t) e;
    }
    case 8: return loft_get_int4_raw(r, f);
    default: return LOFT_NULL;
    }
}

void loft_crt_set_w(int64_t which, int64_t store, int64_t rec, int64_t pos, int64_t fld,
                    int64_t min, int64_t v, int64_t is_null) {
    loft_ref r = loft_crt_ref(store, rec, pos);
    uint32_t f = (uint32_t) fld;
    int64_t x = is_null ? LOFT_NULL : v;
    switch (which) {
    case 0: loft_set_int4(r, f, x); break;
    case 1: loft_set_byte_v(r, f, min, x); break;
    case 2: loft_set_byte_nullable(r, f, min, x); break;
    case 3: loft_set_short(r, f, min, x); break;
    case 4: loft_set_short_raw(r, f, min, x); break;
    case 5: loft_set_char(r, f, is_null ? 0u : (uint32_t) v); break;
    case 6: loft_set_byte(r, f, is_null ? LOFT_BOOL_NULL : (uint8_t) v); break;
    case 7: loft_set_byte(r, f, is_null ? (uint8_t) 255 : (uint8_t) v); break;
    case 8: loft_set_int4_raw(r, f, x); break;
    default: break;
    }
}

/* OpGetFloat and OpGetSingle (`single` 1): NaN is the null either way. */
double loft_crt_get_float(int64_t store, int64_t rec, int64_t pos, int64_t fld, int64_t single) {
    loft_ref r = loft_crt_ref(store, rec, pos);
    return single ? (double) loft_get_single(r, (uint32_t) fld) : loft_get_float(r, (uint32_t) fld);
}

/* OpSetFloat and OpSetSingle: the value is `val[0]`, a float argument being a vector of one. */
void loft_crt_set_float(int64_t store, int64_t rec, int64_t pos, int64_t fld, const double *val,
                        int64_t n, int64_t is_null, int64_t single) {
    loft_ref r = loft_crt_ref(store, rec, pos);
    double v = is_null || n < 1 ? (double) NAN : val[0];
    if (single) loft_set_single(r, (uint32_t) fld, (float) v);
    else loft_set_float(r, (uint32_t) fld, v);
}

/* OpGetField: the reference to an embedded record, `loft_field` — the position moved past the
 * field's offset.  Answers the new position; the store and record are the host's. */
int64_t loft_crt_field_pos(int64_t store, int64_t rec, int64_t pos, int64_t fld) {
    return (int64_t) loft_field(loft_crt_ref(store, rec, pos), (uint32_t) fld).pos;
}

/* ── Vectors (@PLN91 S3, the second slice): a vector is the reference to the field that holds
 * it; an element is a reference into the same store.  Each routine calls the operator's own
 * runtime routine. */

/* OpLengthVector. */
int64_t loft_crt_vec_len(int64_t store, int64_t rec, int64_t pos) {
    return loft_vec_len(loft_crt_ref(store, rec, pos));
}

/* OpGetVector and OpGetVectorNullable: the element at `idx` (negative from the end), or the null
 * reference past either end, written into `out`. */
int64_t loft_crt_vec_get(int64_t *out, int64_t n, int64_t store, int64_t rec, int64_t pos,
                         int64_t size, int64_t idx) {
    loft_crt_put_ref(out, n, loft_vec_get(loft_crt_ref(store, rec, pos), (uint32_t) size, idx));
    return 0;
}

/* OpPreAllocVector: room for `count` more elements of `size` bytes. */
void loft_crt_vec_prealloc(int64_t store, int64_t rec, int64_t pos, int64_t count, int64_t size) {
    loft_vec_prealloc(loft_crt_ref(store, rec, pos), (uint32_t) count, (uint32_t) size);
}

/* The pushes, by `which` (the loft side's `crt::P_*`): 0 OpPushInt, 1 OpPushInt4, 2 OpPushCharacter,
 * 3 OpPushByte (biased by `min`), 4 OpPushEnum, 5 OpPushBoolean.  A null crosses as `is_null`:
 * the int64 null, code 0, or 255 for an enum or a boolean, as a C program's operand carries it. */
void loft_crt_vec_push(int64_t which, int64_t store, int64_t rec, int64_t pos, int64_t min,
                       int64_t v, int64_t is_null) {
    loft_ref r = loft_crt_ref(store, rec, pos);
    int64_t x = is_null ? LOFT_NULL : v;
    switch (which) {
    case 0: loft_vec_push_int(r, x); break;
    case 1: loft_vec_push_int4(r, x); break;
    case 2: loft_vec_push_char(r, is_null ? 0u : (uint32_t) v); break;
    case 3: loft_vec_push_byte(r, loft_byte_raw(min, x)); break;
    case 4:
    case 5: loft_vec_push_byte(r, is_null ? (uint8_t) 255 : (uint8_t) v); break;
    default: break;
    }
}

/* OpPushFloat and OpPushSingle (`single` 1): the value is `val[0]`. */
void loft_crt_vec_push_float(int64_t store, int64_t rec, int64_t pos, const double *val,
                             int64_t n, int64_t is_null, int64_t single) {
    loft_ref r = loft_crt_ref(store, rec, pos);
    double v = is_null || n < 1 ? (double) NAN : val[0];
    if (single) loft_vec_push_single(r, (float) v);
    else loft_vec_push_float(r, v);
}

/* OpNewRecord on a vector: a new element of `size` bytes at its end, written into `out`. */
int64_t loft_crt_vec_append(int64_t *out, int64_t n, int64_t store, int64_t rec, int64_t pos,
                            int64_t size) {
    loft_crt_put_ref(out, n, loft_vec_append(loft_crt_ref(store, rec, pos), (uint32_t) size));
    return 0;
}

/* The bytes a fresh element holds before its literal runs: the type's default image. */
void loft_crt_fill(int64_t store, int64_t rec, int64_t pos, const uint8_t *image, int64_t n) {
    loft_fill(loft_crt_ref(store, rec, pos), image, (uint32_t) n);
}

/* OpFinishRecord on a vector: the appended element counts. */
void loft_crt_vec_finish(int64_t store, int64_t rec, int64_t pos) {
    loft_vec_finish(loft_crt_ref(store, rec, pos));
}

/* How many stores are live: what a probe reads to see a free took effect. */
int64_t loft_crt_live_stores(void) {
    int64_t n = 0;
    for (uint32_t i = 0; i < loft_nstores; i++) n += loft_stores[i].live ? 1 : 0;
    return n;
}

/* P2: one part of a reference after the trip through `loft_ref`, to show none is cut. */
int64_t loft_crt_ref_part(int64_t store, int64_t rec, int64_t pos, int64_t which) {
    loft_ref r = loft_crt_ref(store, rec, pos);
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
static uint32_t loft_crt_ntypes = 0;

static void loft_crt_drop_types(void) {
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
    loft_crt_drop_types();
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
    loft_ref r = loft_crt_get_ref(io, n);
    loft_database_wrapper((const struct loft_tdesc_s *) loft_crt_types, &r, (uint32_t) bytes, (uint16_t) tp);
    loft_crt_put_ref(io, n, r);
    return 0;
}

/* OpClear with `@FR-R-Reuse`'s reset flag (`Stores::reset_pool_root`): a pooled buffer at its
 * store's root made again and prefilled with its type's defaults; answers 0 for any other. */
int64_t loft_crt_reset_pool_root(int64_t store, int64_t rec, int64_t pos, int64_t tp, int64_t size,
                                 const uint8_t *image, int64_t n) {
    return loft_reset_pool_root(loft_crt_ref(store, rec, pos), (uint16_t) tp, (uint32_t) size, image,
                                (uint32_t) n);
}

void loft_crt_clear(int64_t store, int64_t rec, int64_t pos, int64_t tp) {
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_remove_claims(loft_crt_types, (uint16_t) tp, loft_crt_ref(store, rec, pos));
}

/* OpPlaceRecord (Stores::place_record_prefilled): a `tp` record of `size` bytes claimed inside
 * the host's store and prefilled with `image`, or in a fresh store when there is no host; written
 * into `out`.  `foreign`: the host's root is a vector of another element type (@FR-H-RootExtent). */
int64_t loft_crt_place_record(int64_t *out, int64_t n, int64_t h_store, int64_t h_rec, int64_t h_pos, int64_t tp,
                              int64_t size, const uint8_t *image, int64_t in, int64_t foreign) {
    loft_ref r = loft_place_record(loft_crt_ref(h_store, h_rec, h_pos), (uint16_t) tp, (uint32_t) size, image,
                                   (uint32_t) in, (int) foreign);
    loft_crt_put_ref(out, n, r);
    return 0;
}

/* The placement family (`@FR-R-Place`), each through the runtime's own routine and the loaded
 * type table; a call before the table is loaded does nothing. */
#define LOFT_CRT_TP_OK(tp) (loft_crt_types && (tp) >= 0 && (uint32_t) (tp) < loft_crt_ntypes)

/* OpMoveVector (Stores::move_vector): `src`'s elements moved onto `dst`, `linked` an element
 * type of a linked group. */
void loft_crt_move_vector(int64_t d_store, int64_t d_rec, int64_t d_pos, int64_t s_store, int64_t s_rec,
                          int64_t s_pos, int64_t tp, int64_t linked) {
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref d = loft_crt_ref(d_store, d_rec, d_pos), s = loft_crt_ref(s_store, s_rec, s_pos);
    if (linked) loft_move_vector_array(loft_crt_types, (uint16_t) tp, d, s);
    else loft_move_vector(loft_crt_types, (uint16_t) tp, d, s);
}

/* OpMoveField / OpMoveRecord: a `tp` record moved from `src` to `dst` — into a field, or out to a
 * returned record (`record_out`). */
void loft_crt_move_field(int64_t s_store, int64_t s_rec, int64_t s_pos, int64_t d_store, int64_t d_rec,
                         int64_t d_pos, int64_t tp, int64_t record_out) {
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref s = loft_crt_ref(s_store, s_rec, s_pos), d = loft_crt_ref(d_store, d_rec, d_pos);
    if (record_out) loft_move_record_out(loft_crt_types, (uint16_t) tp, s, d);
    else loft_move_field(loft_crt_types, (uint16_t) tp, s, d);
}

/* OpFreeRecordIn: a placed `tp` record released inside its host's store. */
void loft_crt_free_record_in(int64_t store, int64_t rec, int64_t pos, int64_t tp) {
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_free_record_in(loft_crt_types, (uint16_t) tp, loft_crt_ref(store, rec, pos));
}

/* OpReplaceVector (Stores::vector_replace): `v`'s elements replaced by `o`'s. */
void loft_crt_vec_replace(int64_t v_store, int64_t v_rec, int64_t v_pos, int64_t o_store, int64_t o_rec,
                          int64_t o_pos, int64_t tp, int64_t linked) {
    if (!LOFT_CRT_TP_OK(tp)) return;
    loft_ref v = loft_crt_ref(v_store, v_rec, v_pos), o = loft_crt_ref(o_store, o_rec, o_pos);
    if (linked) loft_vec_replace_array(loft_crt_types, (uint16_t) tp, v, o);
    else loft_vec_replace(loft_crt_types, loft_crt_ntypes, (uint16_t) tp, v, o);
}

/* `vector_sum_int` (KERNELS.md, `sum`'s integer loop): `acc` plus every element of the vector,
 * with integer addition's rules — a null element or an overflow makes it null. */
int64_t loft_crt_vector_sum_int(int64_t store, int64_t rec, int64_t pos, int64_t acc) {
    return loft_vector_sum_int(loft_crt_ref(store, rec, pos), acc);
}

/* OpClearKeyed of a SECONDARY VIEW of a linked group (`CLEAR_KEYED_VIEW`): the view drops its own
 * route and leaves the records to the member that holds them (`remove_claims_keyed`). */
void loft_crt_clear_view(int64_t store, int64_t rec, int64_t pos, int64_t tp) {
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_remove_claims_mode(loft_crt_types, (uint16_t) tp, loft_crt_ref(store, rec, pos), 1);
}

/* OpReplaceKeyed (`Stores::replace_keyed`): `dest`'s collection filled again from `src`'s, of
 * type `tp`; `flags` bit 0 the caller's own, bit 1 the element tree may move whole. */
void loft_crt_replace_keyed(int64_t s_store, int64_t s_rec, int64_t s_pos, int64_t d_store, int64_t d_rec,
                            int64_t d_pos, int64_t tp, int64_t flags) {
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_replace_keyed(loft_crt_types, (uint16_t) tp, loft_crt_ref(s_store, s_rec, s_pos),
                       loft_crt_ref(d_store, d_rec, d_pos), (int) flags);
}

/* A record variable bound to a copy of another (`loft_bind_copy`, the C generator's
 * `whole_value_bind`): the variable in `io` gets a store of its own holding `src`'s record and
 * the claims it owns, by the table loaded above. */
void loft_crt_bind_copy(int64_t *io, int64_t n, int64_t tp, int64_t s_store, int64_t s_rec,
                        int64_t s_pos) {
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_ref r = loft_crt_get_ref(io, n);
    loft_bind_copy(loft_crt_types, (uint16_t) tp, &r, loft_crt_ref(s_store, s_rec, s_pos));
    loft_crt_put_ref(io, n, r);
}

/* OpCopyRecord: the record at `src` copied into `dst`, the claims it owns copied with it, by the
 * table loaded above. */
void loft_crt_copy_record(int64_t s_store, int64_t s_rec, int64_t s_pos, int64_t d_store,
                          int64_t d_rec, int64_t d_pos, int64_t tp, int64_t flags) {
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_copy_record(loft_crt_types, (uint16_t) tp, loft_crt_ref(s_store, s_rec, s_pos),
                     loft_crt_ref(d_store, d_rec, d_pos), (int) flags);
}

/* OpClearVector: emptied, the claims its elements own released, by the loaded table. */
void loft_crt_vec_clear(int64_t store, int64_t rec, int64_t pos) {
    if (!loft_crt_types) return;
    loft_vec_clear_release(loft_crt_types, loft_crt_ntypes, loft_crt_ref(store, rec, pos));
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

static void loft_crt_key_push(loft_kv k, char *owned) {
    if (loft_crt_nkv >= LOFT_CRT_KEYS) {
        free(owned);
        return;
    }
    loft_crt_kv[loft_crt_nkv] = k;
    loft_crt_kv_owned[loft_crt_nkv] = owned;
    loft_crt_nkv++;
}

void loft_crt_key_int(int64_t v, int64_t is_null) {
    loft_crt_key_push(loft_kv_int(is_null ? LOFT_NULL : v), 0);
}

void loft_crt_key_float(const double *v, int64_t n, int64_t is_null, int64_t single) {
    double f = is_null || n < 1 ? (double) NAN : v[0];
    loft_crt_key_push(single ? loft_kv_single((float) f) : loft_kv_float(f), 0);
}

void loft_crt_key_text(const char *val, int64_t bytes) {
    if (bytes < 0) {
        loft_crt_key_push(loft_kv_text(loft_null_text()), 0);
        return;
    }
    size_t len = (size_t) bytes;
    char *copy = (char *) malloc(len + 1);
    if (!copy) return;
    memcpy(copy, val, len + 1);
    loft_crt_key_push(loft_kv_text(loft_lit(copy, len)), copy);
}

/* The top `n` parts popped, their text copies released. */
static void loft_crt_key_pop(int64_t n) {
    while (n-- > 0 && loft_crt_nkv > 0) {
        loft_crt_nkv--;
        free(loft_crt_kv_owned[loft_crt_nkv]);
        loft_crt_kv_owned[loft_crt_nkv] = 0;
    }
}

static int loft_crt_keyed(int64_t tp) {
    return loft_crt_types && tp >= 0 && (uint32_t) tp < loft_crt_ntypes;
}

/* OpGetRecord on a hash of type `tp`: the record whose key is the top `n` parts, into `out`; the
 * null reference on a miss.  The parts are popped either way. */
int64_t loft_crt_hash_find(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t tp, int64_t n) {
    loft_ref r = loft_ref_null();
    if (loft_crt_keyed(tp) && n >= 0 && n <= loft_crt_nkv)
        r = loft_h_find(loft_crt_ref(store, rec, pos), loft_crt_types[tp].keys,
                        &loft_crt_kv[loft_crt_nkv - n], (int) n);
    loft_crt_key_pop(n);
    loft_crt_put_ref(out, no, r);
    return 0;
}

/* OpNewRecord in a hash: an entry of `size` bytes of its table, into `out`. */
int64_t loft_crt_hash_entry(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                            int64_t size, int64_t linked) {
    loft_crt_put_ref(out, no,
                     loft_hash_entry(loft_crt_ref(store, rec, pos), (uint32_t) size, (int) linked));
    return 0;
}

/* OpFinishRecord in a hash of type `tp`: the record goes to its key's place, an equal key's
 * element replaced and released by the element type `elem`'s row. */
void loft_crt_hash_insert(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                          int64_t r_pos, int64_t elem, int64_t tp) {
    if (!loft_crt_keyed(tp) || !loft_crt_keyed(elem)) return;
    (void) loft_hash_insert(loft_crt_types, (uint16_t) elem, loft_crt_ref(store, rec, pos),
                            loft_crt_ref(r_store, r_rec, r_pos), loft_crt_types[tp].keys,
                            loft_crt_types[tp].nk, 0);
}

/* OpHashRemove on a hash of type `tp` (element type `elem`): the entry unlinked from its key, and
 * released unless `view` (`CLEAR_KEYED_VIEW`: a linked group's view member only unlinks). */
void loft_crt_hash_remove(int64_t store, int64_t rec, int64_t pos, int64_t r_store, int64_t r_rec,
                          int64_t r_pos, int64_t elem, int64_t tp, int64_t view) {
    if (!loft_crt_keyed(tp) || !loft_crt_keyed(elem)) return;
    loft_hash_remove(loft_crt_types, (uint16_t) elem, loft_crt_ref(store, rec, pos),
                     loft_crt_ref(r_store, r_rec, r_pos), loft_crt_types[tp].keys,
                     loft_crt_types[tp].nk, (int) view);
}

/* OpSetKeyed (`c[k…] = v`, `Stores::set_keyed`) on a hash of type `tp`: a copy of `v` filed under
 * the top `n` key parts, which are popped; `free_source` releases `v`'s store after. */
void loft_crt_set_keyed(int64_t store, int64_t rec, int64_t pos, int64_t v_store, int64_t v_rec,
                        int64_t v_pos, int64_t elem, int64_t tp, int64_t n, int64_t free_source) {
    if (loft_crt_keyed(tp) && loft_crt_keyed(elem) && n >= 0 && n <= loft_crt_nkv)
        loft_set_keyed(loft_crt_types, (uint16_t) elem, loft_crt_ref(store, rec, pos),
                       loft_crt_ref(v_store, v_rec, v_pos), LOFT_KEYED_HASH, loft_crt_types[tp].keys,
                       loft_crt_types[tp].nk, &loft_crt_kv[loft_crt_nkv - n], (int) n,
                       (int) free_source, 0);
    loft_crt_key_pop(n);
}

/* OpSizeHash (`hash::table_bytes`): the bytes of a hash's bucket table, holes included. */
int64_t loft_crt_hash_bytes(int64_t store, int64_t rec, int64_t pos) {
    return loft_hash_table_bytes(loft_crt_ref(store, rec, pos));
}

/* OpLengthHash. */
int64_t loft_crt_hash_len(int64_t store, int64_t rec, int64_t pos) {
    return loft_hash_len(loft_crt_ref(store, rec, pos));
}

/* ── Walking a hash (@PLN91 S3, the fourth slice continued): the snapshot `for c in h` steps
 * over, two records claimed in the hash's own store (loft_hash.h). ── */

/* n_hash_sorted: the snapshot of a hash of type `tp`, its elements in key order, into `out`;
 * n_hash_unsorted (`unordered`): in slot order. */
int64_t loft_crt_hash_sorted(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                             int64_t tp, int64_t unordered) {
    loft_ref r = loft_ref_null();
    if (unordered)
        r = loft_hash_unsorted(loft_crt_ref(store, rec, pos));
    else if (loft_crt_keyed(tp))
        r = loft_hash_sorted(loft_crt_ref(store, rec, pos), loft_crt_types[tp].keys,
                             loft_crt_types[tp].nk);
    loft_crt_put_ref(out, no, r);
    return 0;
}

/* `hash::records_sorted`: the hash's entries in key order, as `OpFormatDatabase` walks them —
 * held in the runtime's own memory, so the walk claims nothing in the store.  Writes up to
 * `no / 3` references into `out` and answers how many entries there are. */
int64_t loft_crt_hash_refs(int64_t *out, int64_t no, int64_t store, int64_t rec, int64_t pos,
                           int64_t tp) {
    if (!loft_crt_keyed(tp)) return 0;
    uint32_t n = 0;
    loft_ref *refs = loft_hash_refs(loft_crt_ref(store, rec, pos), loft_crt_types[tp].keys,
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
    return loft_scratch_iterate(loft_crt_ref(store, rec, pos));
}

/* OpStep over a snapshot: the next element into `out[0..3]`, the state it leaves into `out[3]`. */
int64_t loft_crt_scratch_step(int64_t *out, int64_t no, int64_t state, int64_t store, int64_t rec,
                              int64_t pos, int64_t reverse) {
    loft_ref r = loft_scratch_step(&state, loft_crt_ref(store, rec, pos), (int) reverse);
    loft_crt_put_ref(out, no, r);
    if (no >= 4) out[3] = state;
    return 0;
}

/* OpFreeScratch. */
void loft_crt_free_scratch(int64_t store, int64_t rec, int64_t pos) {
    loft_free_scratch(loft_crt_ref(store, rec, pos));
}

/* OpAppendVector (@PLN91 S3): the elements of `o` appended to `v` as copies of type `tp`, their
 * claims copied by the loaded table. */
void loft_crt_vec_add(int64_t tp, int64_t store, int64_t rec, int64_t pos, int64_t o_store,
                      int64_t o_rec, int64_t o_pos) {
    if (!loft_crt_keyed(tp)) return;
    loft_vec_add(loft_crt_types, (uint16_t) tp, loft_crt_ref(store, rec, pos),
                 loft_crt_ref(o_store, o_rec, o_pos));
}

/* ── Rendering a value (@PLN91 S3, `OpFormatDatabase`): the interpreter walks the schema in loft
 * as the C generator's `loft_show{tp}` does, and every value read from the store is the
 * runtime's own `loft_show_*` helper, chosen here by `kind`: the base types by their type
 * number (0–6), then 7 a byte, 8 a short, 9 a raw short, 10 a 4-byte and 11 a raw 4-byte
 * integer, 12 a stored reference, 13 a child record.  Rendered into a scratch text, copied out. */
static loft_text loft_crt_show_buf = { 0, 0, 0 };

int64_t loft_crt_show_leaf(char *out, int64_t cap, int64_t kind, int64_t store, int64_t rec,
                           int64_t pos, int64_t fmt, int64_t min, int64_t nullable) {
    loft_text *s = &loft_crt_show_buf;
    s->len = 0;
    loft_ref r = loft_crt_ref(store, rec, pos);
    int f = (int) fmt, n = (int) nullable;
    switch (kind) {
    case 0: case 1: loft_show_long(s, r, f, 0); break;
    case 2: loft_show_single(s, r, f, 0); break;
    case 3: loft_show_float(s, r, f, 0); break;
    case 4: loft_show_bool(s, r, f, 0); break;
    case 5: loft_show_text(s, r, f, 0); break;
    case 6: loft_show_char(s, r, f, 0); break;
    case 7: loft_show_byte(s, r, min, n); break;
    case 8: loft_show_short(s, r, min, n); break;
    case 9: loft_show_short_raw(s, r, min, n); break;
    case 10: loft_show_int4(s, r, n); break;
    case 11: loft_show_int_raw(s, r, n); break;
    case 12: loft_show_dbref(s, r); break;
    case 13: loft_show_child(s, r); break;
    default: return 0;
    }
    int64_t len = (int64_t) s->len;
    if (len <= cap && len > 0) memcpy(out, s->p, (size_t) len);
    return len;
}

/* loft_vec_absent: a collection field that holds none. */
int64_t loft_crt_vec_absent(int64_t store, int64_t rec, int64_t pos) {
    return loft_vec_absent(loft_crt_ref(store, rec, pos));
}

/* OpSliceVector / OpSliceView (Stores::vector_slice / vector_slice_view): `v = src[lo..hi]`
 * of scalar elements `size` bytes wide, the bounds clamped; `view` serves a foreign source's
 * bytes in place where the runtime can. */
void loft_crt_vec_slice(int64_t v_store, int64_t v_rec, int64_t v_pos, int64_t o_store, int64_t o_rec,
                        int64_t o_pos, int64_t lo, int64_t hi, int64_t size, int64_t view) {
    loft_ref v = loft_crt_ref(v_store, v_rec, v_pos), o = loft_crt_ref(o_store, o_rec, o_pos);
    if (view) loft_vec_slice_view(v, o, lo, hi, (uint32_t) size);
    else loft_vec_slice(v, o, lo, hi, (uint32_t) size);
}

/* OpAppendCopy (State::append_copy): `[x; n]` — the vector's last element repeated until it
 * holds `count`, of element type `ctp`; `count` <= 0 leaves it empty. */
void loft_crt_vec_append_copy(int64_t store, int64_t rec, int64_t pos, int64_t ctp, int64_t count) {
    loft_vec_append_copy(loft_crt_types, (uint16_t) ctp, loft_crt_ref(store, rec, pos), count);
}

/* OpKeepVectorRange (`Stores::keep_vector_range`): only the elements `lo..hi` kept, what the
 * dropped ones own released through the type table; `size` 0 for a linked element. */
void loft_crt_vec_keep(int64_t store, int64_t rec, int64_t pos, int64_t tp, int64_t size, int64_t lo,
                       int64_t hi) {
    loft_keep_vector_range((const loft_tdesc *) loft_crt_types, (uint16_t) tp,
                           loft_crt_ref(store, rec, pos), (uint32_t) size, lo, hi);
}

/* OpAppendTextBytes: the bytes `lo..hi` of a text appended to a byte vector. */
void loft_crt_append_text_bytes(int64_t store, int64_t rec, int64_t pos, const char *val,
                                int64_t bytes, int64_t lo, int64_t hi) {
    loft_append_text_bytes(loft_crt_ref(store, rec, pos), loft_crt_str(val, bytes), lo, hi);
}

/* text_from_byte_range (`Stores::text_from_bytes_range`): the bytes [lo, hi) of a vector<u8> as a
 * text, copied into `out` when it fits; answers the length in bytes. */
int64_t loft_crt_text_from_byte_range(char *out, int64_t cap, int64_t store, int64_t rec,
                                      int64_t pos, int64_t lo, int64_t hi) {
    loft_text t = loft_text_from_byte_range(loft_crt_ref(store, rec, pos), lo, hi);
    int64_t len = (int64_t) t.len;
    if (len <= cap && len > 0) memcpy(out, t.p, (size_t) len);
    loft_text_free(&t);
    return len;
}

/* OpReverseVector: the elements of `size` bytes reversed in place. */
void loft_crt_vec_reverse(int64_t store, int64_t rec, int64_t pos, int64_t size) {
    loft_vec_reverse(loft_crt_ref(store, rec, pos), (uint32_t) size);
}

/* OpRemoveVector (`Stores::remove_vector_at`): the element at `index` of element type `tp` taken
 * out, as the C generator picks the routine — `mode` 2 a linked element (its slot unlinked, its
 * record released), 1 an element owning heap (released first), 0 a plain one of `size` bytes.
 * Answers whether an element was removed. */
int64_t loft_crt_vec_remove(int64_t store, int64_t rec, int64_t pos, int64_t tp, int64_t size,
                            int64_t mode, int64_t index) {
    loft_ref v = loft_crt_ref(store, rec, pos);
    const loft_tdesc *T = (const loft_tdesc *) loft_crt_types;
    if (mode == 2) return loft_vec_remove_linked(T, (uint16_t) tp, v, index);
    if (mode == 1) return loft_vec_remove_at(T, (uint16_t) tp, v, index);
    return loft_vec_remove(v, (uint32_t) size, index);
}

/* The interpreted program starts: the trace's `M` line, as the Rust interpreter writes it when
 * it enters the program (`State::execute_argv_shared`). */
void loft_crt_trace_mark(void) {
    LOFT_TRACE("M\n");
}
