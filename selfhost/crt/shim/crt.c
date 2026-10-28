/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The `crt` library's shim (@PLN91 S2.2): the C runtime's own format routines, reached by
 * `#c` bindings on every backend.  Each function here only MARSHALS — loft's `#c` model
 * passes plain C (a text out as a caller's byte buffer, a float through a pointer, a null as
 * a flag) — and calls one routine the C runtime already defines, so a format the loft
 * interpreter renders is the C runtime's rendering.  Each answers the rendering's length in
 * bytes; when that exceeds `cap`, nothing is written and the caller asks again with room. */
#include "../../../src/c/loft_crt.h"

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

int64_t loft_crt_format_text(char *out, int64_t cap, const char *val, int64_t is_null,
                             int64_t width, int64_t dir, int64_t token) {
    loft_text t = {0};
    loft_str v = is_null ? loft_null_text() : loft_lit(val, strlen(val));
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
                       int64_t is_null) {
    loft_str v = is_null ? loft_null_text() : loft_lit(val, strlen(val));
    loft_set_text(loft_crt_ref(store, rec, pos), (uint32_t) fld, v);
}

/* OpSetTextReplace: the text field refilled in place where its claim has room. */
void loft_crt_refill_text(int64_t store, int64_t rec, int64_t pos, int64_t fld, const char *val,
                          int64_t is_null) {
    loft_str v = is_null ? loft_null_text() : loft_lit(val, strlen(val));
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
 *   row:  tp, kind, size, elem, n_fields, owns_heap, linked,
 *         then per field: pos, type, view, hidden, group
 * Answers how many types were loaded, or -1 for rows that do not parse. */
static loft_tdesc *loft_crt_types = 0;
static uint32_t loft_crt_ntypes = 0;

static void loft_crt_drop_types(void) {
    for (uint32_t i = 0; i < loft_crt_ntypes; i++) free((void *) loft_crt_types[i].fields);
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
    while (at + 7 <= n) {
        int64_t tp = rows[at], nf = rows[at + 4];
        if (tp < 0 || tp >= ntypes || nf < 0 || at + 7 + nf * 5 > n) return -1;
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
                const int64_t *r = &rows[at + 7 + i * 5];
                f[i].pos = (uint16_t) r[0];
                f[i].type = (uint16_t) r[1];
                f[i].view = (uint8_t) r[2];
                f[i].hidden = (uint8_t) r[3];
                f[i].group = (uint16_t) r[4];
            }
            d->fields = f;
        }
        at += 7 + nf * 5;
        loaded++;
    }
    return at == n ? loaded : -1;
}

/* OpClear without its pool reset (as `LOFT_NO_POOL_RESET` runs it, values unchanged by
 * (R-Switch)): the claims the record owns released, by the table loaded above, so the store
 * can take a new value in place. */
void loft_crt_clear(int64_t store, int64_t rec, int64_t pos, int64_t tp) {
    if (!loft_crt_types || tp < 0 || (uint32_t) tp >= loft_crt_ntypes) return;
    loft_remove_claims(loft_crt_types, (uint16_t) tp, loft_crt_ref(store, rec, pos));
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
