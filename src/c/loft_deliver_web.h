/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * ── deliver / expose / release (`Stores::deliver_reconstruct`, `expose_value`, `release_value`,
 * src/ffi_deliver.rs) ──
 * Off a page `deliver` prints the value's bytes (`loft_deliver_loopback`, loft_rt.h) and
 * `expose` / `release` lock and unlock the value's store.  On a page (`LOFT_BROWSER`) the value
 * is handed to the page where it lies: the `loft_host_deliver` / `loft_host_expose` import gets
 * the store's base address in linear memory, the value's `(rec, pos)`, its type and its layout
 * descriptor as JSON (`LayoutDesc::to_delivery_json`), which `doc/loft-deliver.js` reads the
 * value through.  The unit writes the descriptor (`loft_dlvd{tp}`, src/c/deliver.rs); the run
 * adds its `flat` map: each keyed collection the value holds materialised, by the walker
 * `loft_dlk{tp}`, to the element array its own walk yields, named under the `(rec, pos)` it
 * lives at.  A delivery's arrays are released when the import returns; an exposure's when it is
 * released.  An exposed store is locked after its arrays are made, so its addresses hold until
 * `release`. */

/* The `flat` map being written, and the arrays it names. */
typedef struct loft_dlv_flat { loft_text json; loft_ref *x; uint32_t n, cap; } loft_dlv_flat;
typedef void (*loft_dlk_fn)(loft_cx *cx, loft_dlv_flat *f, loft_ref at);

#if defined(LOFT_BROWSER)
__attribute__((import_module("loft_io"), import_name("loft_host_deliver"))) void
loft_io_deliver(loft_word tag, size_t base, uint32_t rec, uint32_t pos, uint32_t tp, const char *d, size_t n);
__attribute__((import_module("loft_io"), import_name("loft_host_expose"))) void
loft_io_expose(loft_word tag, size_t base, uint32_t rec, uint32_t pos, uint32_t tp, const char *d, size_t n);
__attribute__((import_module("loft_io"), import_name("loft_host_release"))) void
loft_io_release(loft_word tag);

/* `collect_keyed`'s one entry: the collection at `at` materialised as `x`, named by its data
 * record. */
static void loft_dlv_flat_add(loft_cx *cx, loft_dlv_flat *f, loft_ref at, loft_ref x) {
    if (x.store_nr == 0xFFFF || x.rec == 0) return;
    char b[48];
    int n = snprintf(b, sizeof b, "%s\"%u_%u\":%u", f->n ? "," : "", (unsigned) at.rec, (unsigned) at.pos,
                     (unsigned) loft_u32(cx, loft_store_of(cx, x), x.rec, x.pos));
    loft_push(cx, &f->json, b, (size_t) n);
    if (f->n == f->cap) {
        f->cap = f->cap ? f->cap * 2 : 4;
        f->x = (loft_ref *) realloc(f->x, (size_t) f->cap * sizeof *f->x);
        if (!f->x) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    f->x[f->n++] = x;
}

/* `build_index_sorted_vec`: an index's records in key order, as a walk's snapshot. */
static loft_ref loft_dlv_index(loft_cx *cx, loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    uint32_t n;
    uint32_t *recs = loft_bt_records(cx, c, &n);
    loft_ref x = loft_rec_scratch(cx, c, recs, n);
    free(recs);
    return x;
}

/* The arrays each exposed tag holds until its release. */
typedef struct { loft_word tag; loft_ref *x; uint32_t n; } loft_dlv_held;
LOFT_STATE loft_dlv_held *loft_dlv_holds;
static uint32_t loft_dlv_nholds, loft_dlv_holds_cap;

static void loft_dlv_free(loft_cx *cx, loft_ref *x, uint32_t n) {
    for (uint32_t i = 0; i < n; i++) loft_free_scratch(cx, x[i]);
    free(x);
}

static void loft_deliver_page(loft_cx *cx, loft_word tag, uint16_t tp, const char *desc, loft_dlk_fn k, loft_ref r, int expose) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    loft_dlv_flat f;
    memset(&f, 0, sizeof f);
    loft_push(cx, &f.json, desc, strlen(desc));
    loft_push(cx, &f.json, ",\"flat\":{", 9);
    if (k) k(cx, &f, r);
    loft_push(cx, &f.json, "}}", 2);
    if (expose) loft_set_store_lock(cx, r, 1);
    /* Read after the walk: a claim for an array can move the store's buffer. */
    size_t base = (size_t) loft_store_of(cx, r)->buf;
    if (expose) loft_io_expose(tag, base, r.rec, r.pos, tp, f.json.p, f.json.len);
    else loft_io_deliver(tag, base, r.rec, r.pos, tp, f.json.p, f.json.len);
    loft_text_free(cx, &f.json);
    if (!expose || f.n == 0) {
        loft_dlv_free(cx, f.x, f.n);
        return;
    }
    if (loft_dlv_nholds == loft_dlv_holds_cap) {
        loft_dlv_holds_cap = loft_dlv_holds_cap ? loft_dlv_holds_cap * 2 : 4;
        loft_dlv_holds = (loft_dlv_held *) realloc(loft_dlv_holds, (size_t) loft_dlv_holds_cap * sizeof *loft_dlv_holds);
        if (!loft_dlv_holds) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_dlv_holds[loft_dlv_nholds].tag = tag;
    loft_dlv_holds[loft_dlv_nholds].x = f.x;
    loft_dlv_holds[loft_dlv_nholds].n = f.n;
    loft_dlv_nholds++;
}
#endif

static void loft_deliver(loft_cx *cx, loft_word tag, const char *name, loft_dlv_fn f, loft_ref r, uint16_t tp,
                         const char *desc, loft_dlk_fn k) {
#if defined(LOFT_BROWSER)
    (void) name;
    (void) f;
    loft_deliver_page(cx, tag, tp, desc, k, r, 0);
#else
    (void) tp;
    (void) desc;
    (void) k;
    loft_deliver_loopback(cx, tag, name, f, r);
#endif
}

static void loft_expose(loft_cx *cx, loft_word tag, loft_ref r, uint16_t tp, const char *desc, loft_dlk_fn k) {
#if defined(LOFT_BROWSER)
    loft_deliver_page(cx, tag, tp, desc, k, r, 1);
#else
    (void) tag;
    (void) tp;
    (void) desc;
    (void) k;
    loft_set_store_lock(cx, r, 1);
#endif
}

static void loft_release(loft_cx *cx, loft_word tag, loft_ref r) {
#if defined(LOFT_BROWSER)
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    loft_set_store_lock(cx, r, 0);
    for (uint32_t i = 0; i < loft_dlv_nholds;) {
        if (loft_dlv_holds[i].tag != tag) {
            i++;
            continue;
        }
        loft_dlv_free(cx, loft_dlv_holds[i].x, loft_dlv_holds[i].n);
        loft_dlv_holds[i] = loft_dlv_holds[--loft_dlv_nholds];
    }
    loft_io_release(tag);
#else
    (void) tag;
    loft_set_store_lock(cx, r, 0);
#endif
}
