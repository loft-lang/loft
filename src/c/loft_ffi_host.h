/* ── A library's native part, prebuilt behind the loft-ffi ABI (@PLN205) ───────────────────────
 * A package that ships its native part as a library for this host (`prebuilt/<triple>/`, built
 * against `loft-ffi/include/loft_ffi.h`, the header pasted above this file) is called through
 * that ABI: its symbol is resolved from the library on the first call, and a store is handed
 * over as the interpreter's `extensions::make_loft_store` hands it — the buffer, and callbacks
 * into this runtime's allocator.  Pasted into a unit that calls such a native (`LOFT_FFI`);
 * POSIX hosts (Windows is @PLN203 C8's). */

/* The handle's context is the store's NUMBER in its low 16 bits, as the interpreter's host sets
 * it (`extensions::make_loft_store`): a library names the store of a heap answer by it
 * (`loft_ffi_store_nr`, which reads those bits alone), and the store table may move while the
 * call runs.  Above them it carries the store context of the thread that made the call
 * (`@FR-C-Boundary`), which a
 * callback the library makes into this runtime works in: a `par` worker's own table, its own
 * claim log.  Room for both needs 64-bit pointers. */
typedef char loft_ffi_handle_fits[sizeof(uintptr_t) >= 8 ? 1 : -1];
static void *loft_ffi_handle(loft_cx *cx, uint16_t nr) {
    return (void *) (((uintptr_t) cx << 16) | nr);
}
static loft_cx *loft_ffi_handle_cx(loft_ffi_store_ctx c) {
    return (loft_cx *) ((uintptr_t) c.opaque >> 16);
}
static uint16_t loft_ffi_handle_nr(loft_ffi_store_ctx c) {
    return (uint16_t) (uintptr_t) c.opaque;
}
static loft_store_t *loft_ffi_ctx_store(loft_cx *cx, loft_ffi_store_ctx c) {
    return &cx->stores[loft_ffi_handle_nr(c)];
}

/* loft#2227 — the records ONE call pinned to an argument's store claims there (the answer and
 * its scratch, never what an argument held before the call), for `loft_ffi_own_*` to delete after
 * copying the answer out: `extensions::claim_log_*`, the interpreter's twin.  The log is the
 * calling thread's (`cx->ffi_log_*`): `par` workers call natives concurrently, and one shared
 * log let a worker's copy-out delete the records another worker's call had just claimed. */
static void loft_ffi_log_begin(loft_cx *cx, uint16_t nr) {
    cx->ffi_log_on = 1;
    cx->ffi_log_store = nr;
    cx->ffi_log_n = 0;
}

static void loft_ffi_log_note(loft_cx *cx, uint16_t nr, uint32_t old, uint32_t rec) {
    if (!cx->ffi_log_on || nr != cx->ffi_log_store) return;
    if (old) {
        for (uint32_t i = 0; i < cx->ffi_log_n; i++)
            if (cx->ffi_log_recs[i] == old) { cx->ffi_log_recs[i] = rec; return; }
        return;
    }
    if (cx->ffi_log_n == cx->ffi_log_cap) {
        cx->ffi_log_cap = cx->ffi_log_cap ? cx->ffi_log_cap * 2 : 16;
        cx->ffi_log_recs = (uint32_t *) realloc(cx->ffi_log_recs, cx->ffi_log_cap * sizeof(uint32_t));
    }
    cx->ffi_log_recs[cx->ffi_log_n++] = rec;
}

static uint32_t loft_ffi_claim(loft_ffi_store_ctx c, uint32_t words) {
    loft_cx *cx = loft_ffi_handle_cx(c);
    uint32_t rec = loft_claim(cx, loft_ffi_ctx_store(cx, c), words);
    loft_ffi_log_note(cx, loft_ffi_handle_nr(c), 0, rec);
    return rec;
}

static void loft_ffi_reload(loft_ffi_store_ctx c, uint8_t **ptr, uint32_t *size) {
    loft_cx *cx = loft_ffi_handle_cx(c);
    loft_store_t *s = loft_ffi_ctx_store(cx, c);
    *ptr = s->buf;
    *size = s->size;
}

static uint32_t loft_ffi_resize(loft_ffi_store_ctx c, uint32_t rec, uint32_t words) {
    loft_cx *cx = loft_ffi_handle_cx(c);
    uint32_t to = loft_st_resize(cx, loft_ffi_ctx_store(cx, c), rec, words);
    loft_ffi_log_note(cx, loft_ffi_handle_nr(c), rec, to);
    return to;
}

/* Store `nr` as the library sees it.  A library that hands bytes over to be adopted
 * (`foreign_fn`) is not carried yet: the field is null, as on a host that predates it. */
static loft_ffi_store loft_ffi_store_of(loft_cx *cx, uint16_t nr) {
    loft_store_t *s = &cx->stores[nr];
    loft_ffi_store f;
    f.ptr = s->buf;
    f.size = s->size;
    f.ctx.opaque = loft_ffi_handle(cx, nr);
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
static int loft_ffi_is_foreign(loft_cx *cx, loft_ref a) {
    return a.rec != 0 && a.store_nr < cx->nstores && loft_st_is_foreign(cx, &cx->stores[a.store_nr]);
}

static uint32_t loft_ffi_copy_in(loft_cx *cx, uint32_t pin, loft_ref a) {
    loft_fspan sp = cx->stores[a.store_nr].foreign.span;
    size_t bytes = (size_t) sp.len * sp.elem;
    loft_store_t *d = &cx->stores[pin];
    uint32_t rec = loft_st_claim(cx, d, (uint32_t) ((bytes + 15) / 8));
    loft_st_set_u32(cx, d, rec, 4, sp.len);
    if (bytes) memcpy(d->buf + (size_t) rec * 8 + 8, sp.base, bytes);
    return rec;
}

/* `Stores::bridge_args_done`: whether the library wrote through the copy — its length or a byte. */
static int loft_ffi_copy_changed(loft_cx *cx, loft_ref a, uint32_t pin, uint32_t rec) {
    loft_fspan sp = cx->stores[a.store_nr].foreign.span;
    loft_store_t *d = &cx->stores[pin];
    return loft_st_u32(cx, d, rec, 4) != sp.len
        || (sp.len && memcmp(d->buf + (size_t) rec * 8 + 8, sp.base, (size_t) sp.len * sp.elem) != 0);
}
#else
#define loft_ffi_is_foreign(cx, a) 0
#define loft_ffi_copy_in(cx, pin, a) 0u
#define loft_ffi_copy_changed(cx, a, pin, rec) 0
#endif

/* A heap answer as loft holds it (`codegen_runtime::from_loft_ref` / `from_loft_record`): a
 * vector behind a one-word header whose slot at +4 names its data record, a record as it is. */
static loft_ref loft_ffi_vector_answer(loft_cx *cx, loft_ffi_ref r) {
    if (r.rec == 0 && r.pos == 0) return loft_ref_null(cx);
    loft_store_t *s = &cx->stores[r.store_nr];
    uint32_t h = loft_claim(cx, s, 1);
    loft_set_u32(cx, s, h, 4, r.rec);
    loft_ref v = { r.store_nr, h, 4, 0 };
    return v;
}

static loft_ref loft_ffi_record_answer(loft_cx *cx, loft_ffi_ref r) {
    if (r.rec == 0 && r.pos == 0) return loft_ref_null(cx);
    loft_ref v = { r.store_nr, r.rec, r.pos, 0 };
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
static void *loft_ffi_lookup(loft_cx *cx, void *h, const char *sym) {
    void *p = 0;
    typedef void (*loft_ffi_reg_fn)(void (*)(const uint8_t *, size_t, const void *, void *), void *);
    loft_ffi_reg_fn reg;
    void *r = loft_dl_sym(cx, h, "loft_register_v1");
    memcpy(&reg, &r, sizeof reg);
    if (r) {
        loft_ffi_reg_q q = { sym, strlen(sym), 0 };
        reg(loft_ffi_reg_find, &q);
        memcpy(&p, &q.found, sizeof p);
    }
    return p ? p : loft_dl_sym(cx, h, sym);
}
#endif

/* `Store::snapshot_copy` (`Stores::clone_store_for_call`, loft#2239) — a writable copy of store
 * `from` in a store of its own, for ONE native call whose answer must not be claimed where `from`
 * lives: a store the workers of a running `par` share (`cx->par_sharing`), or a pinned one.
 * Every record keeps its number, so the arguments only change their store. */
static void loft_ffi_bits_copy(loft_cx *cx, loft_bits *d, const loft_bits *s) {
    if (d->cap < s->nwords) {
        d->bits = (uint64_t *) realloc(d->bits, (size_t) s->nwords * sizeof(uint64_t));
        if (!d->bits) { fputs("loft: out of memory\n", stderr); abort(); }
        d->cap = s->nwords;
    }
    if (s->nwords) memcpy(d->bits, s->bits, (size_t) s->nwords * sizeof(uint64_t));
    d->nwords = s->nwords;
    d->live = s->live;
}

static uint16_t loft_ffi_clone_store(loft_cx *cx, uint16_t from) {
    uint16_t ns = (uint16_t) loft_store_new(cx);
    loft_store_t *d = &cx->stores[ns];
    const loft_store_t *s = &cx->stores[from];
    if (d->size != s->size) {
        loft_heap_moved(cx, d, d->size, s->size);
        d->buf = (uint8_t *) realloc(d->buf, (size_t) s->size * 8);
        if (!d->buf) { fputs("loft: out of memory\n", stderr); abort(); }
        d->size = s->size;
    }
    memcpy(d->buf, s->buf, (size_t) s->size * 8);
    loft_ffi_bits_copy(cx, &d->claims, &s->claims);
    loft_ffi_bits_copy(cx, &d->referenced, &s->referenced);
    loft_ffi_bits_copy(cx, &d->removed, &s->removed);
    loft_ffi_bits_copy(cx, &d->lazy_free, &s->lazy_free);
    loft_ffi_bits_copy(cx, &d->parked_set, &s->parked_set);
    memcpy(d->parked_heads, s->parked_heads, sizeof d->parked_heads);
    d->parked_count = s->parked_count;
    d->claimed_end = s->claimed_end;
    d->free_root = s->free_root;
    d->wild = s->wild;
    d->dead_words = s->dead_words;
    d->needs_coalesce = s->needs_coalesce;
    d->lazy = s->lazy;
    d->defined = s->defined;
    d->no_park = s->no_park;
    d->known = s->known; /* LOFT_RAW_KNOWN: the store as it is in memory */
    return ns;
}

/* The store a pinned native call works in: `pn` itself, or a copy of it when it is shared. */
/* `@FR-C-Shared`: a shared store is never grown while workers run. */
static uint16_t loft_ffi_call_store(loft_cx *cx, uint16_t pn) {
    return (cx->par_sharing || cx->stores[pn].pinned) ? loft_ffi_clone_store(cx, pn) : pn;
}

/* The copy a call was given goes once its answer is copied out. */
static void loft_ffi_drop_call_store(loft_cx *cx, uint16_t cs, uint16_t pn) {
    if (cs != pn) {
        loft_ref m = { cs, 0, 0, 0 };
        loft_free_ref(cx, m);
    }
}

/* The logged records go, the log closes. */
static void loft_ffi_log_drop(loft_cx *cx, uint16_t nr) {
    for (uint32_t i = 0; i < cx->ffi_log_n; i++)
        if (cx->ffi_log_recs[i]) loft_st_delete(cx, &cx->stores[nr], cx->ffi_log_recs[i]);
    cx->ffi_log_on = 0;
    cx->ffi_log_n = 0;
}

/* loft#2227, `@FR-F-Ret` (`Stores::own_bridge_answer`) — a heap answer built in the argument's
 * store it was pinned to, copied out into a store of its own with its element type (a vector,
 * `loft_vec_add`) or its record type (`loft_copy_record`); then only what the call claimed is
 * deleted, and the vector's header. */
static loft_ref loft_ffi_own_vector(loft_cx *cx, const loft_tdesc *T, uint16_t elem_tp, loft_ref h) {
    if (h.rec == 0) { cx->ffi_log_on = 0; return h; }
    uint16_t ns = (uint16_t) loft_store_new(cx);
    uint32_t root = loft_claim(cx, &cx->stores[ns], 1);
    loft_set_u32(cx, &cx->stores[ns], root, 4, 0);
    loft_ref v = { ns, root, 4, 0 };
    loft_vec_add(cx, T, elem_tp, v, h);
    loft_ffi_log_drop(cx, h.store_nr);
    loft_st_delete(cx, &cx->stores[h.store_nr], h.rec);
    return v;
}

static loft_ref loft_ffi_own_record(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r) {
    if (r.rec == 0) { cx->ffi_log_on = 0; return r; }
    loft_ref d = { 0xFFFF, 0, 0, 0 };
    loft_database_typed(cx, &d, T[tp].size, tp);
    loft_copy_record(cx, T, tp, r, d, 4);
    loft_ffi_log_drop(cx, r.store_nr);
    return d;
}

static void *loft_ffi_sym(loft_cx *cx, const char *path, const char *file, const char *sym);

/* A component's context for this caller context (`@FR-C-Boundary`): made by the component's own
 * `loft_cx_make_v1` the first time this context calls into it, handed to each of its symbols
 * first, and ended by its `loft_cx_end_v1` with this context (`loft_cx_end`).  The component
 * keeps its connection tables and answers there, so a `par` worker's are its own. */
static void *loft_component_cx(loft_cx *cx, const char *path, const char *file) {
    for (int i = 0; i < cx->ncomps; i++)
        if (strcmp(cx->comps[i].path, path) == 0) return cx->comps[i].cx;
    if (cx->ncomps == (int) (sizeof cx->comps / sizeof cx->comps[0])) {
        fputs("loft: more components than a context holds\n", stderr);
        abort();
    }
    void *(*make)(void);
    void (*end)(void *);
    void *m = loft_ffi_sym(cx, path, file, "loft_cx_make_v1");
    void *e = loft_ffi_sym(cx, path, file, "loft_cx_end_v1");
    memcpy(&make, &m, sizeof make);
    memcpy(&end, &e, sizeof end);
    cx->comps[cx->ncomps].path = path;
    cx->comps[cx->ncomps].end = end;
    cx->comps[cx->ncomps].cx = make();
    return cx->comps[cx->ncomps++].cx;
}

/* Symbol `sym` of a library's native part, resolved once per call site: in a shipped program
 * (`LOFT_BIND_LIBS`) the library named `file` it opened at start by search order
 * (`loft_c_bind_libraries`), else the library at `path`, where this build found it.  A library or
 * symbol that is not there halts the program naming both, as the interpreter refuses a native
 * whose library did not load. */
static void *loft_ffi_sym(loft_cx *cx, const char *path, const char *file, const char *sym) {
#if defined(LOFT_DL)
#if defined(LOFT_BIND_LIBS)
    for (uint32_t i = 0; i < loft_ncbl && i < 64; i++) {
        if (!loft_cbl_row_handle[i] || strcmp(loft_cbl[i].file, file) != 0) continue;
        void *p = loft_ffi_lookup(cx, loft_cbl_row_handle[i], sym);
        if (p) return p;
        char m[1024];
        snprintf(m, sizeof m, "native function %s is not available: the library does not export it", sym);
        loft_panic(cx, loft_lit(cx, m, strlen(m)), loft_lit(cx, file, strlen(file)), 0);
        return 0;
    }
#endif
#if defined(LOFT_ANDROID)
    /* An app carries the library in its own `lib/<abi>/`, as `lib<stem>.so` whatever the build
     * machine calls it (`c_driver::android_components`), and the system's loader finds it there
     * by that name alone: `path` is the build machine's. */
    {
        char own[256];
        const char *dot = strchr(file, '.');
        size_t k = dot ? (size_t) (dot - file) : strlen(file);
        if (k + 4 <= sizeof own) {
            memcpy(own, file, k);
            memcpy(own + k, ".so", 4);
            void *ha = loft_dl_open(cx, own, LOFT_DL_LOCAL);
            void *pa = ha ? loft_ffi_lookup(cx, ha, sym) : 0;
            if (pa) return pa;
        }
    }
#endif
    (void) file;
    void *h = loft_dl_open(cx, path, LOFT_DL_LOCAL);
    void *p = h ? loft_ffi_lookup(cx, h, sym) : 0;
    if (p) return p;
    char m[1024];
    snprintf(m, sizeof m, "native function %s is not available: %s", sym,
             h ? "the library does not export it" : loft_dl_error(cx));
    loft_panic(cx, loft_lit(cx, m, strlen(m)), loft_lit(cx, path, strlen(path)), 0);
    return 0;
#else
    (void) file;
    char m[512];
    snprintf(m, sizeof m, "native function %s: this host cannot open a library's native part (%s)", sym, path);
    loft_panic(cx, loft_lit(cx, m, strlen(m)), loft_lit(cx, "", 0), 0);
    return 0;
#endif
}
