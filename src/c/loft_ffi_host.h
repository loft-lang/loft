/* ── A library's native part, prebuilt behind the loft-ffi ABI (@PLN205) ───────────────────────
 * A package that ships its native part as a library for this host (`prebuilt/<triple>/`, built
 * against `loft-ffi/include/loft_ffi.h`, the header pasted above this file) is called through
 * that ABI: its symbol is resolved from the library on the first call, and a store is handed
 * over as the interpreter's `extensions::make_loft_store` hands it — the buffer, and callbacks
 * into this runtime's allocator.  Pasted into a unit that calls such a native (`LOFT_FFI`);
 * POSIX hosts (Windows is @PLN203 C8's). */

/* The handle's context is the store's NUMBER, as the interpreter's host sets it
 * (`extensions::make_loft_store`): a library names the store of a heap answer by it
 * (`loft_ffi_store_nr`), and the store table may move while the call runs. */
static loft_store_t *loft_ffi_ctx_store(loft_ffi_store_ctx c) {
    return &loft_stores[(uint16_t) (uintptr_t) c.opaque];
}

/* loft#2227 — the records ONE call pinned to an argument's store claims there (the answer and
 * its scratch, never what an argument held before the call), for `loft_ffi_own_*` to delete after
 * copying the answer out: `extensions::claim_log_*`, the interpreter's twin. */
static int loft_ffi_log_on;
static uint16_t loft_ffi_log_store;
static uint32_t *loft_ffi_log_recs;
static uint32_t loft_ffi_log_n, loft_ffi_log_cap;

static void loft_ffi_log_begin(uint16_t nr) {
    loft_ffi_log_on = 1;
    loft_ffi_log_store = nr;
    loft_ffi_log_n = 0;
}

static void loft_ffi_log_note(uint16_t nr, uint32_t old, uint32_t rec) {
    if (!loft_ffi_log_on || nr != loft_ffi_log_store) return;
    if (old) {
        for (uint32_t i = 0; i < loft_ffi_log_n; i++)
            if (loft_ffi_log_recs[i] == old) { loft_ffi_log_recs[i] = rec; return; }
        return;
    }
    if (loft_ffi_log_n == loft_ffi_log_cap) {
        loft_ffi_log_cap = loft_ffi_log_cap ? loft_ffi_log_cap * 2 : 16;
        loft_ffi_log_recs = (uint32_t *) realloc(loft_ffi_log_recs, loft_ffi_log_cap * sizeof(uint32_t));
    }
    loft_ffi_log_recs[loft_ffi_log_n++] = rec;
}

static uint32_t loft_ffi_claim(loft_ffi_store_ctx c, uint32_t words) {
    uint32_t rec = loft_claim(loft_ffi_ctx_store(c), words);
    loft_ffi_log_note((uint16_t) (uintptr_t) c.opaque, 0, rec);
    return rec;
}

static void loft_ffi_reload(loft_ffi_store_ctx c, uint8_t **ptr, uint32_t *size) {
    loft_store_t *s = loft_ffi_ctx_store(c);
    *ptr = s->buf;
    *size = s->size;
}

static uint32_t loft_ffi_resize(loft_ffi_store_ctx c, uint32_t rec, uint32_t words) {
    uint32_t to = loft_st_resize(loft_ffi_ctx_store(c), rec, words);
    loft_ffi_log_note((uint16_t) (uintptr_t) c.opaque, rec, to);
    return to;
}

/* Store `nr` as the library sees it.  A library that hands bytes over to be adopted
 * (`foreign_fn`) is not carried yet: the field is null, as on a host that predates it. */
static loft_ffi_store loft_ffi_store_of(uint16_t nr) {
    loft_store_t *s = &loft_stores[nr];
    loft_ffi_store f;
    f.ptr = s->buf;
    f.size = s->size;
    f.ctx.opaque = (void *) (uintptr_t) nr;
    f.claim_fn = loft_ffi_claim;
    f.reload_fn = loft_ffi_reload;
    f.resize_fn = loft_ffi_resize;
    f.foreign_fn = 0;
    return f;
}

/* `Stores::bridge_args` for a foreign vector argument (a `file_map`): its bytes are copied into
 * a vector record of the store the library is handed, in one block, and that record is what
 * the library reads.  Only a unit that maps a file has a foreign store. */
#ifdef LOFT_FOREIGN
static int loft_ffi_is_foreign(loft_ref a) {
    return a.rec != 0 && a.store_nr < loft_nstores && loft_st_is_foreign(&loft_stores[a.store_nr]);
}

static uint32_t loft_ffi_copy_in(uint32_t pin, loft_ref a) {
    loft_fspan sp = loft_stores[a.store_nr].foreign.span;
    size_t bytes = (size_t) sp.len * sp.elem;
    loft_store_t *d = &loft_stores[pin];
    uint32_t rec = loft_st_claim(d, (uint32_t) ((bytes + 15) / 8));
    loft_st_set_u32(d, rec, 4, sp.len);
    if (bytes) memcpy(d->buf + (size_t) rec * 8 + 8, sp.base, bytes);
    return rec;
}

/* `Stores::bridge_args_done`: whether the library wrote through the copy — its length or a byte. */
static int loft_ffi_copy_changed(loft_ref a, uint32_t pin, uint32_t rec) {
    loft_fspan sp = loft_stores[a.store_nr].foreign.span;
    loft_store_t *d = &loft_stores[pin];
    return loft_st_u32(d, rec, 4) != sp.len
        || (sp.len && memcmp(d->buf + (size_t) rec * 8 + 8, sp.base, (size_t) sp.len * sp.elem) != 0);
}
#else
#define loft_ffi_is_foreign(a) 0
#define loft_ffi_copy_in(pin, a) 0u
#define loft_ffi_copy_changed(a, pin, rec) 0
#endif

/* A heap answer as loft holds it (`codegen_runtime::from_loft_ref` / `from_loft_record`): a
 * vector behind a one-word header whose slot at +4 names its data record, a record as it is. */
static loft_ref loft_ffi_vector_answer(loft_ffi_ref r) {
    if (r.rec == 0 && r.pos == 0) return loft_ref_null();
    loft_store_t *s = &loft_stores[r.store_nr];
    uint32_t h = loft_claim(s, 1);
    loft_set_u32(s, h, 4, r.rec);
    loft_ref v = { r.store_nr, h, 4 };
    return v;
}

static loft_ref loft_ffi_record_answer(loft_ffi_ref r) {
    loft_ref v = { r.store_nr, r.rec, r.pos };
    return v;
}

#if defined(LOFT_DL)
/* One `loft_register_v1` entry checked against the name `loft_ffi_sym` asks for. */
typedef struct { const char *want; size_t len; const void *found; } loft_ffi_reg_q;
static void loft_ffi_reg_find(const uint8_t *name, size_t len, const void *fn, void *ctx) {
    loft_ffi_reg_q *q = (loft_ffi_reg_q *) ctx;
    if (!q->found && len == q->len && memcmp(name, q->want, len) == 0) q->found = fn;
}

/* The function an open library implements native `sym` with: the entry its `loft_register_v1`
 * table registers under that name — which may be another symbol (`loft_save_png =>
 * n_save_png`, the store-aware variant the interpreter calls too) — else the symbol of that
 * name. */
static void *loft_ffi_lookup(void *h, const char *sym) {
    void *p = 0;
    typedef void (*loft_ffi_reg_fn)(void (*)(const uint8_t *, size_t, const void *, void *), void *);
    loft_ffi_reg_fn reg;
    void *r = loft_dl_sym(h, "loft_register_v1");
    memcpy(&reg, &r, sizeof reg);
    if (r) {
        loft_ffi_reg_q q = { sym, strlen(sym), 0 };
        reg(loft_ffi_reg_find, &q);
        memcpy(&p, &q.found, sizeof p);
    }
    return p ? p : loft_dl_sym(h, sym);
}
#endif

/* The logged records go, the log closes. */
static void loft_ffi_log_drop(uint16_t nr) {
    for (uint32_t i = 0; i < loft_ffi_log_n; i++)
        if (loft_ffi_log_recs[i]) loft_st_delete(&loft_stores[nr], loft_ffi_log_recs[i]);
    loft_ffi_log_on = 0;
    loft_ffi_log_n = 0;
}

/* loft#2227, `@FR-F-Ret` (`Stores::own_bridge_answer`) — a heap answer built in the argument's
 * store it was pinned to, copied out into a store of its own with its element type (a vector,
 * `loft_vec_add`) or its record type (`loft_copy_record`); then only what the call claimed is
 * deleted, and the vector's header. */
static loft_ref loft_ffi_own_vector(const loft_tdesc *T, uint16_t elem_tp, loft_ref h) {
    if (h.rec == 0) { loft_ffi_log_on = 0; return h; }
    uint16_t ns = (uint16_t) loft_store_new();
    uint32_t root = loft_claim(&loft_stores[ns], 1);
    loft_set_u32(&loft_stores[ns], root, 4, 0);
    loft_ref v = { ns, root, 4 };
    loft_vec_add(T, elem_tp, v, h);
    loft_ffi_log_drop(h.store_nr);
    loft_st_delete(&loft_stores[h.store_nr], h.rec);
    return v;
}

static loft_ref loft_ffi_own_record(const loft_tdesc *T, uint16_t tp, loft_ref r) {
    if (r.rec == 0) { loft_ffi_log_on = 0; return r; }
    loft_ref d = { 0xFFFF, 0, 0 };
    loft_database_typed(&d, T[tp].size, tp);
    loft_copy_record(T, tp, r, d, 4);
    loft_ffi_log_drop(r.store_nr);
    return d;
}

/* Symbol `sym` of a library's native part, resolved once per call site: in a shipped program
 * (`LOFT_BIND_LIBS`) the library named `file` it opened at start by search order
 * (`loft_c_bind_libraries`), else the library at `path`, where this build found it.  A library or
 * symbol that is not there halts the program naming both, as the interpreter refuses a native
 * whose library did not load. */
static void *loft_ffi_sym(const char *path, const char *file, const char *sym) {
#if defined(LOFT_DL)
#if defined(LOFT_BIND_LIBS)
    for (uint32_t i = 0; i < loft_ncbl && i < 64; i++) {
        if (!loft_cbl_row_handle[i] || strcmp(loft_cbl[i].file, file) != 0) continue;
        void *p = loft_ffi_lookup(loft_cbl_row_handle[i], sym);
        if (p) return p;
        char m[1024];
        snprintf(m, sizeof m, "native function %s is not available: the library does not export it", sym);
        loft_panic(loft_lit(m, strlen(m)), loft_lit(file, strlen(file)), 0);
        return 0;
    }
#endif
    (void) file;
    void *h = loft_dl_open(path, LOFT_DL_LOCAL);
    void *p = h ? loft_ffi_lookup(h, sym) : 0;
    if (p) return p;
    char m[1024];
    snprintf(m, sizeof m, "native function %s is not available: %s", sym,
             h ? "the library does not export it" : loft_dl_error());
    loft_panic(loft_lit(m, strlen(m)), loft_lit(path, strlen(path)), 0);
    return 0;
#else
    (void) file;
    char m[512];
    snprintf(m, sizeof m, "native function %s: this host cannot open a library's native part (%s)", sym, path);
    loft_panic(loft_lit(m, strlen(m)), loft_lit("", 0), 0);
    return 0;
#endif
}
