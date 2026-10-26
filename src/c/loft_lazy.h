/* ── Lazy stores (@PLN129, `src/database/lazy.rs`): a collection bound to a source ─────────────
 * A keyed lookup that misses on a bound collection consults the source for that one key and
 * looks again, so the collection stays the only authority on what is resident.  A `.store`
 * image is read a page at a time by the paged loaders, through a fetch function the emitter
 * writes per binding site from the collection's type.  A database (`sqlite:`) or a loft driver's
 * scheme has no driver in a C build: its fetch is unreachable, which the fault channel says.
 * The channel is sticky — every failure counted, the FIRST reason kept — and only
 * `store_lazy_clear` clears it.  Pasted only into a unit that binds (`LOFT_LAZY`). */
#ifdef LOFT_LAZY

/* The paged load of one key into collection `coll` from the image at `path`: the count loaded. */
typedef int64_t (*loft_lazy_fetch_fn)(loft_ref coll, loft_str path, const loft_kv *key, int n);

/* A binding: the collection's place, its source, the source's identity when it was bound
 * (`source_pin`: length, modification time, inode), and the fetch for its kind. */
typedef struct {
    loft_ref coll;
    loft_text source;
    int pinned;
    uint64_t len, mtime, ino;
    loft_lazy_fetch_fn fetch;
    const char *element;
} loft_lazy_bind;

/* A collection's failures (`Stores::lazy_errors`): how many, and the first reason. */
typedef struct { loft_ref coll; int64_t count; loft_text reason; } loft_lazy_err;

static loft_lazy_bind *loft_lazy_binds = 0;
static size_t loft_nlazy = 0, loft_lazy_cap = 0;
static loft_lazy_err *loft_lazy_errs = 0;
static size_t loft_nlazy_errs = 0, loft_lazy_errs_cap = 0;

static int loft_lazy_same(loft_ref a, loft_ref b) {
    return a.store_nr == b.store_nr && a.rec == b.rec && a.pos == b.pos;
}

static loft_lazy_bind *loft_lazy_binding(loft_ref coll) {
    for (size_t i = 0; i < loft_nlazy; i++)
        if (loft_lazy_same(loft_lazy_binds[i].coll, coll)) return &loft_lazy_binds[i];
    return 0;
}

/* `Stores::lazy_bound`: is a source bound to this collection? */
static int loft_lazy_bound(loft_ref coll) {
    return loft_nlazy != 0 && loft_lazy_binding(coll) != 0;
}

static loft_lazy_err *loft_lazy_err_of(loft_ref coll) {
    for (size_t i = 0; i < loft_nlazy_errs; i++)
        if (loft_lazy_same(loft_lazy_errs[i].coll, coll)) return &loft_lazy_errs[i];
    return 0;
}

/* `Stores::lazy_refuse`: one more failure for the collection, the first reason kept; 0. */
static int64_t loft_lazy_refuse(loft_ref coll, loft_str why) {
    loft_lazy_err *e = loft_lazy_err_of(coll);
    if (!e) {
        if (loft_nlazy_errs == loft_lazy_errs_cap) {
            loft_lazy_errs_cap = loft_lazy_errs_cap ? loft_lazy_errs_cap * 2 : 8;
            loft_lazy_errs = (loft_lazy_err *) realloc(loft_lazy_errs, loft_lazy_errs_cap * sizeof *loft_lazy_errs);
            if (!loft_lazy_errs) { fputs("loft: out of memory\n", stderr); abort(); }
        }
        e = &loft_lazy_errs[loft_nlazy_errs++];
        e->coll = coll;
        e->count = 0;
        e->reason = loft_text_own(why);
    }
    e->count++;
    return 0;
}

/* `Stores::source_pin`: a local source's identity, 0 for a URL or a path that is not there. */
static int loft_lazy_pin(const char *source, uint64_t *len, uint64_t *mtime, uint64_t *ino) {
    if (strncmp(source, "http://", 7) == 0 || strncmp(source, "https://", 8) == 0) return 0;
#ifdef LOFT_POSIX
    struct stat st;
    if (stat(source, &st) != 0) return 0;
    *len = (uint64_t) st.st_size;
    *mtime = LOFT_ST_MTIME_NS(st);
    *ino = (uint64_t) st.st_ino;
    return 1;
#else
    FILE *f = fopen(source, "rb");
    if (!f) return 0;
    fseek(f, 0, SEEK_END);
    *len = (uint64_t) ftell(f);
    fclose(f);
    *mtime = 0;
    *ino = 0;
    return 1;
#endif
}

/* Does `source` name a database (`sqlite:`) or a loft driver's scheme (`LazySource::of`)? */
static int loft_lazy_database(loft_str source) {
    static const char *const schemes[] = { "sqlite", "postgres", "postgresql", "pg", "mysql", "mariadb", "maria",
                                           "duckdb", "duck" };
    for (size_t i = 0; i < source.len; i++) {
        if (source.p[i] != ':') continue;
        for (size_t k = 0; k < sizeof schemes / sizeof schemes[0]; k++) {
            if (strlen(schemes[k]) != i) continue;
            size_t j = 0;
            while (j < i) {
                char c = source.p[j];
                if (c >= 'A' && c <= 'Z') c = (char) (c - 'A' + 'a');
                if (c != schemes[k][j]) break;
                j++;
            }
            if (j == i) return k == 0 ? 1 : 2;
        }
        return 0;
    }
    return 0;
}

/* `store_bind_lazy(local, source)` (`Stores::bind_lazy`): a source an image cannot serve this
 * collection's kind from is refused here (`unservable`, the emitter's reason), the binding
 * dropped; else the collection is bound, the source pinned, a rebind re-pinning it. */
static uint8_t loft_lazy_bind_coll(loft_ref coll, loft_str source, loft_lazy_fetch_fn fetch, const char *element,
                                   const char *unservable) {
    if (coll.rec == 0) return 0;
    loft_lazy_bind *b = loft_lazy_binding(coll);
    if (unservable && !loft_lazy_database(source)) {
        if (b) {
            loft_text_free(&b->source);
            *b = loft_lazy_binds[--loft_nlazy];
        }
        fputs("store_bind_lazy: refusing `", stderr);
        fwrite(source.p, 1, source.len, stderr);
        fprintf(stderr, "` — %s\n", unservable);
        loft_lazy_refuse(coll, loft_lit(unservable, strlen(unservable)));
        return 0;
    }
    if (!b) {
        if (loft_nlazy == loft_lazy_cap) {
            loft_lazy_cap = loft_lazy_cap ? loft_lazy_cap * 2 : 8;
            loft_lazy_binds = (loft_lazy_bind *) realloc(loft_lazy_binds, loft_lazy_cap * sizeof *loft_lazy_binds);
            if (!loft_lazy_binds) { fputs("loft: out of memory\n", stderr); abort(); }
        }
        b = &loft_lazy_binds[loft_nlazy++];
        b->coll = coll;
        b->source.p = 0;
        b->source.len = b->source.cap = 0;
    }
    loft_text_set(&b->source, source);
    b->fetch = fetch;
    b->element = element;
    b->pinned = loft_lazy_pin(b->source.p, &b->len, &b->mtime, &b->ino);
    return 1;
}

/* `Stores::fetch_missing`: the lookup on bound collection `coll` missed `key`; 1 when the source
 * inserted the entry and the lookup is to be run again. */
static int loft_lazy_miss(loft_ref coll, const loft_kv *key, int n) {
    loft_lazy_bind *b = loft_lazy_binding(coll);
    if (!b) return 0;
    const char *source = b->source.p;
    char why[512];
    if (b->pinned) {
        uint64_t len = 0, mtime = 0, ino = 0;
        if (!loft_lazy_pin(source, &len, &mtime, &ino) || len != b->len || mtime != b->mtime || ino != b->ino) {
            snprintf(why, sizeof why, "`%s` changed since it was bound — rebind to accept the new version", source);
            loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
            return 0;
        }
    }
    int db = loft_lazy_database(loft_view(&b->source));
    if (db == 2) {
        /* `no_lazy_driver`: a loft driver's scheme, and this program declares none. */
        snprintf(why, sizeof why,
                 "`%s` needs a loft driver for %s — define `fn lazy_fetch(coll: <the %s collection>, source: text, "
                 "key_int: integer, key_text: text) -> integer` and bind the collection again",
                 source, b->element, b->element);
        loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
        return 0;
    }
    if (db == 1) {
        snprintf(why, sizeof why, "`%s` needs a database driver, which this build does not have", source);
        loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
        return 0;
    }
    free(loft_pg_refusal);
    loft_pg_refusal = 0;
    if (b->fetch(coll, loft_view(&b->source), key, n) > 0) return 1;
    /* Unreachable is decided by the open, a refusal by what the loader said, else the key is
     * absent from a source that was reached (`fetch_from_file`). */
    char *p = loft_cpath(loft_view(&b->source), 0);
    FILE *f = fopen(p, "rb");
    free(p);
    if (!f) {
        int err = errno;
        snprintf(why, sizeof why, "%s (os error %d)", strerror(err), err);
        loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
        return 0;
    }
    uint8_t head[4];
    size_t got = fread(head, 1, 4, f);
    fclose(f);
    uint32_t sig = 0;
    if (got == 4) memcpy(&sig, head, 4);
    if (got != 4 || sig != LOFT_ST_SIGNATURE) {
        snprintf(why, sizeof why, "not a loft store image (the file does not begin with the store signature)");
        loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
        return 0;
    }
    if (loft_pg_refusal) loft_lazy_refuse(coll, loft_lit(loft_pg_refusal, strlen(loft_pg_refusal)));
    return 0;
}

/* `store_lazy_error`: the first reason a fetch for `coll` failed, "" when healthy. */
static loft_str loft_lazy_error(loft_ref coll) {
    loft_lazy_err *e = loft_lazy_err_of(coll);
    return e ? loft_view(&e->reason) : loft_lit("", 0);
}

/* `store_lazy_faults`: how many fetches for `coll` failed. */
static int64_t loft_lazy_faults(loft_ref coll) {
    loft_lazy_err *e = loft_lazy_err_of(coll);
    return e ? e->count : 0;
}

/* `store_lazy_clear`: the failures acknowledged; 1 when there was something to clear. */
static uint8_t loft_lazy_clear(loft_ref coll) {
    loft_lazy_err *e = loft_lazy_err_of(coll);
    if (!e) return 0;
    loft_text_free(&e->reason);
    *e = loft_lazy_errs[--loft_nlazy_errs];
    return 1;
}

/* `store_lazy_fail(local, why)`: a driver's refusal, through the same channel. */
static void loft_lazy_fail(loft_ref coll, loft_str why) {
    loft_lazy_refuse(coll, why);
}

/* `store_lazy_query` / `store_lazy_range` (`lazy_query`, `lazy_range`): a database's explicit
 * fetch, which a C build has no driver to run — refused through the channel, nothing added. */
static int64_t loft_lazy_db_fetch(loft_ref coll) {
    loft_lazy_bind *b = loft_lazy_binding(coll);
    char why[512];
    if (!b) return loft_lazy_refuse(coll, loft_lit("this collection is not bound to a source", 40));
    if (loft_lazy_database(loft_view(&b->source)) != 1)
        snprintf(why, sizeof why, "`%s` is not a database source", b->source.p);
    else
        snprintf(why, sizeof why, "`%s` needs a database driver, which this build does not have", b->source.p);
    return loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
}

#endif
