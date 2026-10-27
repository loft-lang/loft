/* ── A library's native part, prebuilt behind the loft-ffi ABI (@PLN205) ───────────────────────
 * A package that ships its native part as a library for this host (`prebuilt/<triple>/`, built
 * against `loft-ffi/include/loft_ffi.h`, the header pasted above this file) is called through
 * that ABI: its symbol is resolved from the library on the first call, and a store is handed
 * over as the interpreter's `extensions::make_loft_store` hands it — the buffer, and callbacks
 * into this runtime's allocator.  Pasted into a unit that calls such a native (`LOFT_FFI`);
 * POSIX hosts (Windows is @PLN203 C8's). */

static uint32_t loft_ffi_claim(loft_ffi_store_ctx c, uint32_t words) {
    return loft_claim((loft_store_t *) c.opaque, words);
}

static void loft_ffi_reload(loft_ffi_store_ctx c, uint8_t **ptr, uint32_t *size) {
    loft_store_t *s = (loft_store_t *) c.opaque;
    *ptr = s->buf;
    *size = s->size;
}

static uint32_t loft_ffi_resize(loft_ffi_store_ctx c, uint32_t rec, uint32_t words) {
    return loft_st_resize((loft_store_t *) c.opaque, rec, words);
}

/* The store `s` as the library sees it.  A library that hands bytes over to be adopted
 * (`foreign_fn`) is not carried yet: the field is null, as on a host that predates it. */
static loft_ffi_store loft_ffi_store_of(loft_store_t *s) {
    loft_ffi_store f;
    f.ptr = s->buf;
    f.size = s->size;
    f.ctx.opaque = s;
    f.claim_fn = loft_ffi_claim;
    f.reload_fn = loft_ffi_reload;
    f.resize_fn = loft_ffi_resize;
    f.foreign_fn = 0;
    return f;
}

/* Symbol `sym` of the library at `path`, opened once; a library or symbol that is not there
 * halts the program naming both, as the interpreter refuses a native whose library did not
 * load. */
static void *loft_ffi_sym(const char *path, const char *sym) {
#if defined(LOFT_POSIX_HOST)
    void *h = dlopen(path, RTLD_NOW | RTLD_LOCAL);
    void *p = h ? dlsym(h, sym) : 0;
    if (p) return p;
    char m[1024];
    snprintf(m, sizeof m, "native function %s is not available: %s", sym,
             h ? "the library does not export it" : dlerror());
    loft_panic(loft_lit(m, strlen(m)), loft_lit(path, strlen(path)), 0);
    return 0;
#else
    char m[512];
    snprintf(m, sizeof m, "native function %s: this host cannot open a library's native part (%s)", sym, path);
    loft_panic(loft_lit(m, strlen(m)), loft_lit("", 0), 0);
    return 0;
#endif
}
