/* ── Lazy stores (@PLN129, `src/database/lazy.rs`): a collection bound to a source ─────────────
 * A keyed lookup that misses on a bound collection consults the source for that one key and
 * looks again, so the collection stays the only authority on what is resident.  A `.store`
 * image is read a page at a time by the paged loaders, through a fetch function the emitter
 * writes per binding site from the collection's type.  A `sqlite:` source is driven here as core
 * drives it (`sql_source.rs`, below); a loft driver's scheme calls the program's own driver.
 * The channel is sticky — every failure counted, the FIRST reason kept — and only
 * `store_lazy_clear` clears it.  Pasted only into a unit that binds (`LOFT_LAZY`). */
#ifdef LOFT_LAZY

/* The paged load of one key into collection `coll` from the image at `path`: the count loaded. */
typedef int64_t (*loft_lazy_fetch_fn)(loft_ref coll, loft_str path, const loft_kv *key, int n);

/* A `sqlite:` source's plan for one collection type (`sql_query::derive_select`, derived when
 * the unit was written): the equality query, its table, each selected column with the field it
 * fills (byte position, `write_cell`'s content id, the field type's name), how many keys it
 * binds, and what `loft_set_keyed` files the fetched record with. */
typedef struct { const char *column; uint32_t pos; int content; const char *type_name; } loft_sql_col;
typedef struct {
    const char *select, *table, *element;
    const loft_tdesc *types;
    const loft_sql_col *cols;
    int ncols, nparams;
    uint16_t elem;
    uint32_t size;
    int kind;
    const loft_key *keys;
    int nk;
    /* `lazy_query`'s select up to its WHERE, the caller's predicate appended; `lazy_range`'s
     * query and how many placeholders it has (0 when the collection has no range). */
    const char *filter, *range;
    int range_nparams;
} loft_sql_plan;

/* A binding: the collection's place, its source, the source's identity when it was bound
 * (`source_pin`: length, modification time, inode), the fetch for its kind, and for a database
 * its plan and the schema check's verdict (asked once: `schema_refusal`). */
typedef struct {
    loft_ref coll;
    loft_text source;
    int pinned;
    uint64_t len, mtime, ino;
    loft_lazy_fetch_fn fetch;
    const char *element;
    const loft_sql_plan *sql;
    int sql_checked;
    char *sql_refusal;
} loft_lazy_bind;

/* A collection's failures (`Stores::lazy_errors`): how many, and the first reason. */
typedef struct { loft_ref coll; int64_t count; loft_text reason; } loft_lazy_err;

/* A program's own lazy drivers (`Data::lazy_fetch_drivers`, @PLN133 S8/S9): one per element type,
 * each an adapter the unit writes with the driver's fixed shape, bound by the unit's `main`; or
 * the reason the program's drivers were refused, which every miss then reports. */
typedef int64_t (*loft_lazy_driver_fn)(loft_ref coll, loft_str source, int64_t key_int, loft_str key_text);
typedef struct { const char *element; loft_lazy_driver_fn fn; } loft_lazy_driver;
static const loft_lazy_driver *loft_lazy_drivers = 0;
static uint32_t loft_nlazy_drivers = 0;
static const char *loft_lazy_driver_refusal = 0;

static loft_lazy_driver_fn loft_lazy_driver_for(const char *element) {
    for (uint32_t i = 0; element && i < loft_nlazy_drivers; i++)
        if (!strcmp(loft_lazy_drivers[i].element, element)) return loft_lazy_drivers[i].fn;
    return 0;
}

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
                                   const char *unservable, const loft_sql_plan *sql) {
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
        b->sql_refusal = 0;
    }
    loft_text_set(&b->source, source);
    b->fetch = fetch;
    b->element = element;
    b->sql = sql;
    b->sql_checked = 0;
    free(b->sql_refusal);
    b->sql_refusal = 0;
    b->pinned = loft_lazy_pin(b->source.p, &b->len, &b->mtime, &b->ino);
    return 1;
}

/* ── The `sqlite:` source core drives (`sql_source.rs`) ──
 * SQLite's C API, opened read-only through the library the system has (`SQLITE_SONAMES`) and
 * resolved by name, so a program that never reads a database needs no sqlite at all.  One
 * connection per target, kept open; the DQS misfeature (a double-quoted name that resolves to
 * nothing read as a string literal) is turned off on it. */
typedef struct { int kind; int64_t i; double f; loft_text t; } loft_sql_cell;  /* kind 0 NULL, 1 int, 2 real, 3 text */

#if defined(LOFT_POSIX_HOST)
static struct {
    int tried, ok;
    int (*open_v2)(const char *, void **, int, const char *);
    int (*prepare)(void *, const char *, int, void **, const char **);
    int (*step)(void *);
    int (*col_count)(void *);
    int (*col_type)(void *, int);
    int64_t (*col_int)(void *, int);
    double (*col_double)(void *, int);
    const unsigned char *(*col_text)(void *, int);
    int (*bind_int)(void *, int, int64_t);
    int (*bind_text)(void *, int, const char *, int, void (*)(void *));
    int (*bind_double)(void *, int, double);
    int (*finalize)(void *);
    const char *(*errmsg)(void *);
    int (*close)(void *);
    int (*db_config)(void *, int, ...);
} loft_sq;

static int loft_sq_resolve(char *why, size_t cap) {
    static const char *const names[] = { "winsqlite3.dll", "libsqlite3.so.0", 0 };
    static const char *const syms[] = { "sqlite3_open_v2", "sqlite3_prepare_v2", "sqlite3_step", "sqlite3_column_count",
                                        "sqlite3_column_type", "sqlite3_column_int64", "sqlite3_column_double",
                                        "sqlite3_column_text", "sqlite3_bind_int64", "sqlite3_bind_text",
                                        "sqlite3_bind_double", "sqlite3_finalize", "sqlite3_errmsg", "sqlite3_close",
                                        "sqlite3_db_config", 0 };
    void **slots[] = { (void **) &loft_sq.open_v2, (void **) &loft_sq.prepare, (void **) &loft_sq.step,
                       (void **) &loft_sq.col_count, (void **) &loft_sq.col_type, (void **) &loft_sq.col_int,
                       (void **) &loft_sq.col_double, (void **) &loft_sq.col_text, (void **) &loft_sq.bind_int,
                       (void **) &loft_sq.bind_text, (void **) &loft_sq.bind_double, (void **) &loft_sq.finalize,
                       (void **) &loft_sq.errmsg, (void **) &loft_sq.close, (void **) &loft_sq.db_config };
    if (!loft_sq.tried) {
        void *self = dlopen(0, RTLD_LAZY), *h = 0;
        loft_sq.tried = 1;
        if (!(self && dlsym(self, "sqlite3_open_v2")))
            for (const char *const *n = names; *n && !h; n++) h = dlopen(*n, RTLD_NOW | RTLD_GLOBAL);
        loft_sq.ok = 1;
        for (int i = 0; syms[i]; i++) {
            void *p = h ? dlsym(h, syms[i]) : 0;
            if (!p && self) p = dlsym(self, syms[i]);
            if (!p) {
                snprintf(why, cap, "sqlite is not installed — `%s` did not resolve (looked for winsqlite3.dll, libsqlite3.so.0)",
                         syms[i]);
                loft_sq.ok = 0;
                break;
            }
            memcpy(slots[i], &p, sizeof p);
        }
    }
    if (!loft_sq.ok && !why[0])
        snprintf(why, cap, "sqlite is not installed — `sqlite3_open_v2` did not resolve (looked for winsqlite3.dll, libsqlite3.so.0)");
    return loft_sq.ok;
}

typedef struct { char *target; void *db; } loft_sq_conn;
static loft_sq_conn *loft_sq_conns = 0;
static size_t loft_sq_nconns = 0;

static void *loft_sq_open(const char *target, char *why, size_t cap) {
    void *db = 0;
    for (size_t i = 0; i < loft_sq_nconns; i++)
        if (!strcmp(loft_sq_conns[i].target, target)) return loft_sq_conns[i].db;
    if (!loft_sq_resolve(why, cap)) return 0;
    int rc = loft_sq.open_v2(target, &db, 1 /* SQLITE_OPEN_READONLY */, 0);
    if (rc != 0 || !db) {
        if (!db) snprintf(why, cap, "`%s` could not be opened (sqlite rc %d)", target, rc);
        else {
            snprintf(why, cap, "`%s` could not be opened: %s", target, loft_sq.errmsg(db));
            loft_sq.close(db);
        }
        return 0;
    }
    loft_sq.db_config(db, 1014 /* DQS_DML */, 0, (int *) 0);
    loft_sq.db_config(db, 1013 /* DQS_DDL */, 0, (int *) 0);
    loft_sq_conns = (loft_sq_conn *) realloc(loft_sq_conns, (loft_sq_nconns + 1) * sizeof *loft_sq_conns);
    if (!loft_sq_conns) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_sq_conns[loft_sq_nconns].target = loft_log_dup(target, strlen(target));
    loft_sq_conns[loft_sq_nconns].db = db;
    loft_sq_nconns++;
    return db;
}

/* `SqlConn::query`: every row of `sql` with `args` bound, as cells; 0 rows and `*n_rows` set on
 * success, -1 with the engine's own text in `why` when it does not prepare or run. */
static int loft_sq_query(void *db, const char *target, const char *sql, const loft_kv *args, int nargs,
                         loft_sql_cell **rows, int *n_rows, int *n_cols, char *why, size_t cap) {
    void *st = 0;
    *rows = 0;
    *n_rows = *n_cols = 0;
    if (loft_sq.prepare(db, sql, -1, &st, 0) != 0 || !st) {
        snprintf(why, cap, "`%s`: %s [%s]", target, loft_sq.errmsg(db), sql);
        return -1;
    }
    for (int i = 0; i < nargs; i++) {
        int rc;
        if (args[i].kind == LOFT_KV_TEXT) {
            char *t = loft_log_dup(args[i].s.p, args[i].s.len);
            rc = loft_sq.bind_text(st, i + 1, t, -1, (void (*)(void *)) -1 /* SQLITE_TRANSIENT */);
            free(t);
        } else if (args[i].kind == LOFT_KV_FLOAT) rc = loft_sq.bind_double(st, i + 1, args[i].f);
        else if (args[i].kind == LOFT_KV_SINGLE) rc = loft_sq.bind_double(st, i + 1, (double) args[i].g);
        else rc = loft_sq.bind_int(st, i + 1, args[i].i);
        if (rc != 0) {
            snprintf(why, cap, "`%s`: %s", target, loft_sq.errmsg(db));
            loft_sq.finalize(st);
            return -1;
        }
    }
    for (;;) {
        int rc = loft_sq.step(st);
        if (rc == 101 /* DONE */) break;
        if (rc != 100 /* ROW */) {
            snprintf(why, cap, "`%s`: %s [%s]", target, loft_sq.errmsg(db), sql);
            loft_sq.finalize(st);
            return -1;
        }
        int n = loft_sq.col_count(st);
        *n_cols = n;
        *rows = (loft_sql_cell *) realloc(*rows, (size_t) (*n_rows + 1) * (size_t) (n > 0 ? n : 1) * sizeof **rows);
        if (!*rows) { fputs("loft: out of memory\n", stderr); abort(); }
        loft_sql_cell *row = *rows + (size_t) *n_rows * (size_t) n;
        for (int i = 0; i < n; i++) {
            loft_sql_cell *c = &row[i];
            memset(c, 0, sizeof *c);
            switch (loft_sq.col_type(st, i)) {
            case 5: break; /* NULL — before `column_text`, which answers "" and NULL alike */
            case 1: c->kind = 1; c->i = loft_sq.col_int(st, i); break;
            case 2: c->kind = 2; c->f = loft_sq.col_double(st, i); break;
            default: {
                const unsigned char *t = loft_sq.col_text(st, i);
                if (t) {
                    c->kind = 3;
                    loft_push(&c->t, (const char *) t, strlen((const char *) t));
                    loft_push(&c->t, "", 0);
                }
            }
            }
        }
        (*n_rows)++;
    }
    loft_sq.finalize(st);
    return 0;
}

static void loft_sq_free_rows(loft_sql_cell *rows, int n_rows, int n_cols) {
    for (int i = 0; i < n_rows * n_cols; i++) free(rows[i].t.p);
    free(rows);
}

/* SQLite's affinity rule for a declared column type (`sql_source::affinity`). */
static const char *loft_sq_affinity(const char *declared) {
    char up[64];
    size_t n = strlen(declared);
    if (n >= sizeof up) n = sizeof up - 1;
    for (size_t i = 0; i < n; i++) up[i] = (char) (declared[i] >= 'a' && declared[i] <= 'z' ? declared[i] - 32 : declared[i]);
    up[n] = 0;
    if (strstr(up, "INT")) return "INTEGER";
    if (strstr(up, "CHAR") || strstr(up, "CLOB") || strstr(up, "TEXT")) return "TEXT";
    if (strstr(up, "BLOB") || !n) return "BLOB";
    if (strstr(up, "REAL") || strstr(up, "FLOA") || strstr(up, "DOUB")) return "REAL";
    return "NUMERIC";
}

/* `interrogate_schema`: the table exists, each column exists with an affinity its field can
 * hold, and an index serves the lookup (EXPLAIN QUERY PLAN says SEARCH, never SCAN). */
static int loft_sq_schema(void *db, const char *target, const loft_sql_plan *pl, char *why, size_t cap) {
    char q[512];
    loft_sql_cell *rows;
    int nr, nc;
    snprintf(q, sizeof q, "PRAGMA table_info(%s)", pl->table);
    if (loft_sq_query(db, target, q, 0, 0, &rows, &nr, &nc, why, cap) != 0) return 0;
    if (nr == 0) {
        snprintf(why, cap, "the source has no table %s", pl->table);
        loft_sq_free_rows(rows, nr, nc);
        return 0;
    }
    for (int c = 0; c < pl->ncols; c++) {
        char want[128];
        const char *w = pl->cols[c].column;
        size_t wl = strlen(w);
        while (wl && (*w == '"' || *w == '`')) { w++; wl--; }
        while (wl && (w[wl - 1] == '"' || w[wl - 1] == '`')) wl--;
        if (wl >= sizeof want) wl = sizeof want - 1;
        memcpy(want, w, wl);
        want[wl] = 0;
        const char *tp = 0;
        for (int r = 0; r < nr && nc > 2; r++) {
            loft_sql_cell *name = &rows[r * nc + 1], *decl = &rows[r * nc + 2];
            if (name->kind == 3 && !strcmp(name->t.p, want)) { tp = decl->kind == 3 ? decl->t.p : ""; break; }
        }
        if (!tp) {
            snprintf(why, cap, "%s has no column `%s`", pl->table, want);
            loft_sq_free_rows(rows, nr, nc);
            return 0;
        }
        const char *a = loft_sq_affinity(tp);
        int content = pl->cols[c].content;
        int wrong = content <= 4 ? (!strcmp(a, "TEXT") || !strcmp(a, "BLOB"))
                    : content == 5 ? (!strcmp(a, "INTEGER") || !strcmp(a, "REAL")) : 1;
        if (wrong) {
            snprintf(why, cap, "%s.%s is %s (%s affinity), which cannot hold `%s`", pl->table, want, tp, a,
                     pl->cols[c].type_name);
            loft_sq_free_rows(rows, nr, nc);
            return 0;
        }
    }
    loft_sq_free_rows(rows, nr, nc);
    snprintf(q, sizeof q, "EXPLAIN QUERY PLAN %s", pl->select);
    if (loft_sq_query(db, target, q, 0, 0, &rows, &nr, &nc, why, cap) != 0) return 0;
    for (int i = 0; i < nr * nc; i++) {
        if (rows[i].kind == 3 && !strncmp(rows[i].t.p, "SCAN", 4)) {
            snprintf(why, cap, "the source has no index for this lookup — `%s`. Every fault would read the whole "
                     "table, which is a working feature and a catastrophic one", rows[i].t.p);
            loft_sq_free_rows(rows, nr, nc);
            return 0;
        }
    }
    loft_sq_free_rows(rows, nr, nc);
    return 1;
}
#endif

#if defined(LOFT_POSIX_HOST)
static void loft_lazy_sql_materialise(const loft_sql_plan *pl, loft_ref coll, const loft_sql_cell *row);
#endif

/* `fetch_from_sql`: the equality query for the missed key, its first row written into a fresh
 * record (`materialise` / `write_cell`: a NULL is loft's null for the field) and filed by the
 * collection's own insert.  1 inserted, 0 absent; a source that cannot answer is refused. */
static int loft_lazy_sql_fetch(loft_lazy_bind *b, const char *target, const loft_kv *key, int n) {
    char why[1024];
    why[0] = 0;
    const loft_sql_plan *pl = b->sql;
    if (!pl) {
        snprintf(why, sizeof why, "no query can be derived for `%s` — its element has a field that is not a column",
                 b->element ? b->element : "?");
        loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
        return 0;
    }
    if (pl->nparams != n) {
        snprintf(why, sizeof why, "`%s` is keyed on %d column(s) and the lookup gave %d", pl->element, pl->nparams, n);
        loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
        return 0;
    }
#if defined(LOFT_POSIX_HOST)
    void *db = loft_sq_open(target, why, sizeof why);
    if (!db) {
        loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
        return 0;
    }
    if (!b->sql_checked) {
        b->sql_checked = 1;
        if (!loft_sq_schema(db, target, pl, why, sizeof why)) b->sql_refusal = loft_log_dup(why, strlen(why));
    }
    if (b->sql_refusal) {
        loft_lazy_refuse(b->coll, loft_lit(b->sql_refusal, strlen(b->sql_refusal)));
        return 0;
    }
    loft_sql_cell *rows;
    int nr, nc;
    if (loft_sq_query(db, target, pl->select, key, n, &rows, &nr, &nc, why, sizeof why) != 0) {
        loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
        return 0;
    }
    if (nr == 0) {
        loft_sq_free_rows(rows, nr, nc);
        return 0;
    }
    if (nc != pl->ncols) {
        snprintf(why, sizeof why, "`%s` answered %d column(s) where the query asked %d", target, nc, pl->ncols);
        loft_sq_free_rows(rows, nr, nc);
        loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
        return 0;
    }
    loft_lazy_sql_materialise(pl, b->coll, rows);
    loft_sq_free_rows(rows, nr, nc);
    return 1;
#else
    (void) target;
    (void) key;
    snprintf(why, sizeof why, "`%s` needs a database driver, which this build does not have", b->source.p);
    loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
    return 0;
#endif
}

#if defined(LOFT_POSIX_HOST)
/* `materialise` / `write_cell`: one row written into a fresh record and filed by the collection's
 * own insert; a NULL is the field's null. */
static void loft_lazy_sql_materialise(const loft_sql_plan *pl, loft_ref coll, const loft_sql_cell *row) {
    int nc = pl->ncols;
    loft_ref tmp = loft_ref_null();
    loft_database(&tmp, pl->size);
    for (int c = 0; c < nc; c++) {
        const loft_sql_cell *cell = &row[c];
        uint32_t at = pl->cols[c].pos;
        int64_t iv = cell->kind == 1 ? cell->i : cell->kind == 2 ? (int64_t) cell->f
                     : cell->kind == 3 ? strtoll(cell->t.p, 0, 10) : LOFT_NULL;
        int has_int = cell->kind == 1 || cell->kind == 2 || (cell->kind == 3 && cell->t.p[0]);
        double fv = cell->kind == 2 ? cell->f : cell->kind == 1 ? (double) cell->i
                    : cell->kind == 3 ? strtod(cell->t.p, 0) : (double) NAN;
        switch (pl->cols[c].content) {
        case 0:
        case 1: loft_set_int(tmp, at, has_int ? iv : LOFT_NULL); break;
        case 2: loft_set_single(tmp, at, (float) fv); break;
        case 3: loft_set_float(tmp, at, fv); break;
        case 4: loft_set_byte(tmp, at, (uint8_t) (has_int && iv != 0)); break;
        case 6: {
            uint32_t ch = 0;
            if (cell->kind == 3) {
                loft_str sv = loft_lit(cell->t.p, cell->t.len ? cell->t.len - 1 : 0);
                size_t used = 0;
                if (sv.len) ch = loft_decode(sv, 0, &used);
            } else if (has_int) ch = (uint32_t) iv;
            loft_set_char(tmp, at, ch);
            break;
        }
        case 5:
            if (cell->kind == 0) break;
            else if (cell->kind == 3) loft_set_text(tmp, at, loft_lit(cell->t.p, cell->t.len ? cell->t.len - 1 : 0));
            else {
                char num[64];
                if (cell->kind == 1) snprintf(num, sizeof num, "%lld", (long long) cell->i);
                else snprintf(num, sizeof num, "%.17g", cell->f);
                loft_set_text(tmp, at, loft_lit(num, strlen(num)));
            }
            break;
        default: break;
        }
    }
    loft_set_keyed(pl->types, pl->elem, coll, tmp, pl->kind, pl->keys, pl->nk, 0, 0, 1, 0);
}

/* `row_key` + the ask-first rule of `lazy_query` / `lazy_range`: every row whose key is not
 * resident is materialised (a person reached by a predicate and by navigation is one record);
 * the count added, or the refusal through the channel. */
static int64_t loft_lazy_sql_rows(loft_lazy_bind *b, const char *target, const char *sql, const loft_kv *args,
                                  int nargs) {
    const loft_sql_plan *pl = b->sql;
    char why[1024];
    why[0] = 0;
    void *db = loft_sq_open(target, why, sizeof why);
    if (!db) return loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
    loft_sql_cell *rows;
    int nr, nc;
    if (loft_sq_query(db, target, sql, args, nargs, &rows, &nr, &nc, why, sizeof why) != 0)
        return loft_lazy_refuse(b->coll, loft_lit(why, strlen(why)));
    int64_t added = 0;
    for (int r = 0; r < nr; r++) {
        const loft_sql_cell *row = rows + (size_t) r * (size_t) nc;
        if (nc != pl->ncols) {
            loft_sq_free_rows(rows, nr, nc);
            static const char m[] = "the source answered a different column count";
            return loft_lazy_refuse(b->coll, loft_lit(m, sizeof m - 1));
        }
        loft_kv key[8];
        int nk = pl->nk < 8 ? pl->nk : 8, found_all = 1;
        for (int k = 0; k < nk; k++) {
            int col = -1;
            for (int c = 0; c < nc; c++)
                if (pl->cols[c].pos == pl->keys[k].position) { col = c; break; }
            if (col < 0 || row[col].kind == 0) { found_all = 0; break; }
            memset(&key[k], 0, sizeof key[k]);
            const loft_sql_cell *cell = &row[col];
            if (cell->kind == 3) {
                key[k].kind = LOFT_KV_TEXT;
                key[k].s = loft_lit(cell->t.p, cell->t.len ? cell->t.len - 1 : 0);
            } else if (pl->cols[col].content == 2 || pl->cols[col].content == 3) {
                key[k].kind = LOFT_KV_FLOAT;
                key[k].f = cell->kind == 2 ? cell->f : (double) cell->i;
                key[k].g = (float) key[k].f;
            } else {
                key[k].kind = LOFT_KV_INT;
                key[k].i = cell->kind == 1 ? cell->i : (int64_t) cell->f;
            }
        }
        if (!found_all) {
            loft_sq_free_rows(rows, nr, nc);
            static const char m[] = "a row arrived without the collection's key";
            return loft_lazy_refuse(b->coll, loft_lit(m, sizeof m - 1));
        }
        if (loft_keyed_find(b->coll, pl->kind, pl->size, pl->keys, pl->nk, key, nk).rec != 0) continue;
        loft_lazy_sql_materialise(pl, b->coll, row);
        added++;
    }
    loft_sq_free_rows(rows, nr, nc);
    return added;
}
#endif

/* The binding of `coll` to a database, or the refusal (`lazy_query` / `lazy_range`'s preamble). */
static loft_lazy_bind *loft_lazy_db_binding(loft_ref coll, int64_t *refused) {
    loft_lazy_bind *b = loft_lazy_binding(coll);
    char why[512];
    if (!b) {
        static const char m[] = "this collection is not bound to a source";
        *refused = loft_lazy_refuse(coll, loft_lit(m, sizeof m - 1));
        return 0;
    }
    if (loft_lazy_database(loft_view(&b->source)) != 1) {
        snprintf(why, sizeof why, "`%s` is not a database source", b->source.p);
        *refused = loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
        return 0;
    }
    return b;
}

/* `store_lazy_query(coll, condition)` (`Stores::lazy_query`): the caller's predicate, verbatim, after
 * the derived select — not index-checked, it is the escape hatch — every new row materialised. */
static int64_t loft_lazy_query(loft_ref coll, loft_str condition) {
    int64_t refused = 0;
    loft_lazy_bind *b = loft_lazy_db_binding(coll, &refused);
    char why[512];
    if (!b) return refused;
    if (!b->sql || !b->sql->filter) {
        snprintf(why, sizeof why, "no query can be derived for `%s`", b->element ? b->element : "?");
        return loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
    }
#if defined(LOFT_POSIX_HOST)
    loft_text q = { 0, 0, 0 };
    loft_push(&q, b->sql->filter, strlen(b->sql->filter));
    loft_push(&q, condition.p, condition.len);
    loft_push(&q, "", 1);
    int64_t n = loft_lazy_sql_rows(b, b->source.p + 7, q.p, 0, 0);
    free(q.p);
    return n;
#else
    (void) condition;
    snprintf(why, sizeof why, "`%s` needs a database driver, which this build does not have", b->source.p);
    return loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
#endif
}

/* `store_lazy_range(coll, lo, hi)` (`Stores::lazy_range`): the ordered collection's one key column
 * between `lo` and `hi`, schema-checked like a lookup, every new row materialised. */
static int64_t loft_lazy_range(loft_ref coll, int64_t lo, int64_t hi) {
    int64_t refused = 0;
    loft_lazy_bind *b = loft_lazy_db_binding(coll, &refused);
    char why[1024];
    if (!b) return refused;
    if (!b->sql || !b->sql->range) {
        snprintf(why, sizeof why, "`%s` has no range to fetch — an ordered collection is what derives one",
                 b->element ? b->element : "?");
        return loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
    }
    if (b->sql->range_nparams != 2) {
        static const char m[] = "a range fetch takes ONE key column; a composite key needs an explicit query";
        return loft_lazy_refuse(coll, loft_lit(m, sizeof m - 1));
    }
#if defined(LOFT_POSIX_HOST)
    const char *target = b->source.p + 7;
    why[0] = 0;
    void *db = loft_sq_open(target, why, sizeof why);
    if (!db) return loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
    if (!b->sql_checked) {
        b->sql_checked = 1;
        if (!loft_sq_schema(db, target, b->sql, why, sizeof why)) b->sql_refusal = loft_log_dup(why, strlen(why));
    }
    if (b->sql_refusal) return loft_lazy_refuse(coll, loft_lit(b->sql_refusal, strlen(b->sql_refusal)));
    loft_kv args[2];
    memset(args, 0, sizeof args);
    args[0].kind = args[1].kind = LOFT_KV_INT;
    args[0].i = lo;
    args[1].i = hi;
    return loft_lazy_sql_rows(b, target, b->sql->range, args, 2);
#else
    (void) lo;
    (void) hi;
    snprintf(why, sizeof why, "`%s` needs a database driver, which this build does not have", b->source.p);
    return loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
#endif
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
    /* A driver WINS over the source core drives itself (@PLN133 S9), and a refused driver set
     * is reported at every miss rather than falling back. */
    loft_lazy_driver_fn driver = loft_lazy_driver_for(b->element);
    if (db && loft_lazy_driver_refusal) {
        loft_lazy_refuse(coll, loft_lit(loft_lazy_driver_refusal, strlen(loft_lazy_driver_refusal)));
        return 0;
    }
    if (db && driver) {
        /* One key, whichever spelling it arrived in: fetching the WRONG row is worse than
         * answering absent. */
        int64_t key_int = 0;
        loft_str key_text = loft_lit("", 0);
        if (n == 1 && key[0].kind == LOFT_KV_INT) key_int = key[0].i;
        else if (n == 1 && key[0].kind == LOFT_KV_TEXT) key_text = key[0].s;
        else {
            static const char composite[] = "a composite key cannot be fetched — a driver is given one key";
            loft_lazy_refuse(coll, loft_lit(composite, sizeof composite - 1));
            return 0;
        }
        jmp_buf jb;
        jmp_buf *volatile outer = loft_lazy_jmp;
        volatile uint64_t serial0 = loft_stores_allocated;
#if defined(LOFT_CALL_CHAIN)
        volatile size_t depth = loft_nframes;
#endif
        volatile int64_t answer = 0;
        if (setjmp(jb) == 0) {
            loft_lazy_jmp = &jb;
            answer = driver(coll, loft_view(&b->source), key_int, key_text);
            loft_lazy_jmp = outer;
        } else {
            /* The driver faulted: the stores its abandoned frames made are freed (the teardown
             * the fault skipped, `lazy_watch_end`), and the fault is the collection's failure. */
            loft_lazy_jmp = outer;
            for (uint32_t i = 0; i < loft_nstores; i++)
                if (loft_stores[i].live && loft_stores[i].serial > serial0) {
                    loft_ref r = { (uint16_t) i, 0, 0 };
                    loft_free_ref(r);
                }
#if defined(LOFT_CALL_CHAIN)
            loft_nframes = depth;
#endif
            loft_lazy_refuse(coll, loft_view(&loft_lazy_fault));
            return 0;
        }
        if (answer != 0 && answer != 1) {
            snprintf(why, sizeof why, "`lazy_fetch` answered %lld; it must answer 1 (inserted) or 0 (absent)",
                     (long long) answer);
            loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
            return 0;
        }
        return (int) answer;
    }
    if (db == 2) {
        /* `no_lazy_driver`: a loft driver's scheme, and this program declares none. */
        snprintf(why, sizeof why,
                 "`%s` needs a loft driver for %s — define `fn lazy_fetch(coll: <the %s collection>, source: text, "
                 "key_int: integer, key_text: text) -> integer` and bind the collection again",
                 source, b->element, b->element);
        loft_lazy_refuse(coll, loft_lit(why, strlen(why)));
        return 0;
    }
    if (db == 1) return loft_lazy_sql_fetch(b, source + 7 /* after `sqlite:` */, key, n);
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

#endif
