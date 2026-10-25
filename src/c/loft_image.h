/* ── Store images: a store written to a file and read back (src/database, src/store.rs) ────────
 * loft's own image format: a store's words as they stand, from word 0 (the signature) to past
 * its last live record, then one free block to at least LOFT_IMG_MIN_WORDS — and beside it a
 * `.dschema` sidecar naming the layout and the slot the image's references point into.  An image
 * is written REBUILT (`Stores::persist_copy`): the collection's root copied into a fresh store
 * record by record, its trees laid out for paging, so the live store is left as it was
 * (`@FR-S-Frame`, `@FR-S-Image`, `@FR-S-Sidecar`, `@FR-S-Gate`). */

#define LOFT_IMG_MIN_WORDS 1024u

/* One persisted type's sidecar text (`LayoutIdentity::of_store(..).to_sidecar()`), split where
 * the slot line goes, and the layout the loader gates on (`classify`): the hash and the dump. */
typedef struct {
    uint16_t tp;
    const char *before_slot, *after_slot;
    uint64_t hash;
    const char *dump;
    const char *oldest;
} loft_sidecar;

/* ── radix_tree::rtree_relayout: the nodes renumbered in van Emde Boas order ── */

static uint32_t loft_rx_dfs(loft_store_t *s, uint32_t t, uint32_t *out) {
    uint32_t nodes = loft_u32(s, t, LOFT_RX_NODES), n = 0, depth = 0;
    uint32_t *stack = (uint32_t *) malloc(((size_t) nodes + 2) * 4);
    if (!stack) { fputs("loft: out of memory\n", stderr); abort(); }
    int32_t top = loft_rx_top(s, t);
    if (LOFT_RX_NODE(top)) stack[depth++] = LOFT_RX_NODE_ID(top);
    while (depth > 0) {
        uint32_t node = stack[--depth];
        if (n >= nodes) break;
        out[n++] = node;
        for (int dir = 1; dir >= 0; dir--) {
            int32_t c = loft_rx_child(s, t, node, dir);
            if (LOFT_RX_NODE(c) && depth < nodes + 1) stack[depth++] = LOFT_RX_NODE_ID(c);
        }
    }
    free(stack);
    return n;
}

typedef struct { uint32_t *out, n, cap; const uint32_t *heights; } loft_veb;

static void loft_veb_emit(loft_store_t *s, uint32_t t, loft_veb *v, uint32_t root, uint32_t h) {
    if (v->n >= v->cap) return;
    if (h <= 1) {
        v->out[v->n++] = root;
        return;
    }
    uint32_t top_h = (h + 1) / 2, bot_h = h - top_h;
    loft_veb_emit(s, t, v, root, top_h);
    uint32_t *level = (uint32_t *) malloc(((size_t) v->cap + 1) * 4), *next = (uint32_t *) malloc(((size_t) v->cap + 1) * 4);
    if (!level || !next) { fputs("loft: out of memory\n", stderr); abort(); }
    uint32_t nl = 1;
    level[0] = root;
    for (uint32_t step = 0; step < top_h; step++) {
        uint32_t nn = 0;
        for (uint32_t i = 0; i < nl; i++) {
            for (int dir = 0; dir < 2; dir++) {
                int32_t c = loft_rx_child(s, t, level[i], dir);
                if (LOFT_RX_NODE(c)) {
                    if (nn >= v->cap) {
                        free(level);
                        free(next);
                        return;
                    }
                    next[nn++] = LOFT_RX_NODE_ID(c);
                }
            }
        }
        uint32_t *swap = level;
        level = next;
        next = swap;
        nl = nn;
    }
    for (uint32_t i = 0; i < nl; i++) {
        uint32_t b = level[i], hb = v->heights[b];
        loft_veb_emit(s, t, v, b, bot_h < hb ? bot_h : hb);
    }
    free(level);
    free(next);
}

/* radix_tree::rtree_relayout: 0 when the walk does not account for every live node. */
static int loft_rx_relayout(loft_store_t *s, uint32_t t) {
    uint32_t nodes = loft_u32(s, t, LOFT_RX_NODES), len = loft_rx_len(s, t);
    uint32_t *dfs = (uint32_t *) malloc(((size_t) nodes + 2) * 4);
    uint32_t *heights = (uint32_t *) calloc((size_t) nodes + 2, 4);
    uint32_t *order = (uint32_t *) malloc(((size_t) nodes + 2) * 4);
    if (!dfs || !heights || !order) { fputs("loft: out of memory\n", stderr); abort(); }
    uint32_t nd = loft_rx_dfs(s, t, dfs);
    for (uint32_t i = nd; i-- > 0;) {
        uint32_t best = 1;
        for (int dir = 0; dir < 2; dir++) {
            int32_t c = loft_rx_child(s, t, dfs[i], dir);
            if (LOFT_RX_NODE(c) && heights[LOFT_RX_NODE_ID(c)] + 1 > best) best = heights[LOFT_RX_NODE_ID(c)] + 1;
        }
        heights[dfs[i]] = best;
    }
    loft_veb v = { order, 0, nodes, heights };
    int32_t top = loft_rx_top(s, t);
    if (LOFT_RX_NODE(top)) loft_veb_emit(s, t, &v, LOFT_RX_NODE_ID(top), heights[LOFT_RX_NODE_ID(top)]);
    int ok = v.n == (len > 0 ? len - 1 : 0);
    if (ok && v.n == 0) {
        loft_set_u32(s, t, LOFT_RX_NODES, 0);
        loft_set_u32(s, t, LOFT_RX_FREE, 0);
    } else if (ok) {
        uint32_t *new_id = (uint32_t *) calloc((size_t) nodes + 2, 4);
        uint32_t *body = (uint32_t *) malloc((size_t) v.n * 16);
        if (!new_id || !body) { fputs("loft: out of memory\n", stderr); abort(); }
        for (uint32_t i = 0; i < v.n; i++) new_id[order[i]] = i + 1;
#define LOFT_RX_MOVED(c) (LOFT_RX_NODE(c) ? -(int32_t) new_id[LOFT_RX_NODE_ID(c)] : (c))
        for (uint32_t i = 0; i < v.n; i++) {
            uint32_t n = order[i], parent = loft_rx_node_parent(s, t, n);
            body[i * 4] = loft_rx_node_bit(s, t, n);
            body[i * 4 + 1] = parent == 0 ? 0 : new_id[parent];
            body[i * 4 + 2] = (uint32_t) LOFT_RX_MOVED(loft_rx_child(s, t, n, 0));
            body[i * 4 + 3] = (uint32_t) LOFT_RX_MOVED(loft_rx_child(s, t, n, 1));
        }
        for (uint32_t i = 0; i < v.n; i++) {
            for (uint32_t w = 0; w < 4; w++) loft_set_u32(s, t, loft_rx_node_off(i + 1) + 4 * w, body[i * 4 + w]);
        }
        loft_set_u32(s, t, LOFT_RX_TOP, (uint32_t) LOFT_RX_MOVED(top));
#undef LOFT_RX_MOVED
        loft_set_u32(s, t, LOFT_RX_NODES, v.n);
        loft_set_u32(s, t, LOFT_RX_FREE, 0);
        free(new_id);
        free(body);
    }
    free(dfs);
    free(heights);
    free(order);
    return ok;
}

/* Stores::type_has_tree: can a value of `tp` hold a spatial or a trie at all? */
static int loft_type_has_tree(const loft_tdesc *T, uint32_t ntypes, uint16_t tp, int depth) {
    if (tp >= ntypes || depth > 64) return 0;
    const loft_tdesc *d = &T[tp];
    switch (d->kind) {
    case LOFT_T_RADIX:
    case LOFT_T_TRIE:
        return 1;
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++)
            if (loft_type_has_tree(T, ntypes, d->fields[i].type, depth + 1)) return 1;
        return 0;
    case LOFT_T_ENUM:
        for (uint16_t i = 0; i < d->n; i++)
            if (d->variants[i] != 0xFFFF && loft_type_has_tree(T, ntypes, d->variants[i], depth + 1)) return 1;
        return 0;
    case LOFT_T_VECTOR:
    case LOFT_T_CHILD:
    case LOFT_T_HASH:
    case LOFT_T_INDEX:
    case LOFT_T_ORDERED:
        return loft_type_has_tree(T, ntypes, d->elem, depth + 1);
    default:
        return 0;
    }
}

/* Stores::relayout_trees' walk (`tree_roots`): every spatial or trie field the value at `r`
 * holds, each laid out where it stands.  `*budget` bounds the walk as the Rust one bounds it. */
static uint32_t loft_relayout_walk(const loft_tdesc *T, uint32_t ntypes, uint16_t tp, loft_ref r, uint32_t *budget) {
    if (*budget == 0 || tp >= ntypes || r.rec == 0) return 0;
    (*budget)--;
    const loft_tdesc *d = &T[tp];
    loft_store_t *s = loft_store_of(r);
    uint32_t done = 0, cur;
    switch (d->kind) {
    case LOFT_T_RADIX:
    case LOFT_T_TRIE:
        cur = loft_u32(s, r.rec, r.pos);
        if (cur != 0 && cur != 0xFFFFFFFFu && loft_rx_relayout(s, cur)) done++;
        return done;
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) {
            if (!T[d->fields[i].type].owns_heap) continue;
            done += loft_relayout_walk(T, ntypes, d->fields[i].type, loft_field(r, d->fields[i].pos), budget);
        }
        return done;
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(s, r.rec, r.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) done += loft_relayout_walk(T, ntypes, d->variants[e], r, budget);
        return done;
    }
    case LOFT_T_VECTOR: {
        cur = loft_collection_rec(s, r.rec, r.pos);
        if (cur == 0) return 0;
        uint32_t len = loft_u32(s, cur, 4), size = T[d->elem].size;
        for (uint32_t i = 0; i < len; i++) {
            loft_ref e = { r.store_nr, cur, 8 + i * size };
            done += loft_relayout_walk(T, ntypes, d->elem, e, budget);
        }
        return done;
    }
    case LOFT_T_CHILD: {
        cur = loft_u32(s, r.rec, r.pos);
        loft_ref e = { r.store_nr, cur, 8 };
        return cur == 0 ? 0 : loft_relayout_walk(T, ntypes, d->elem, e, budget);
    }
    case LOFT_T_HASH: {
        uint32_t n;
        loft_hent *all = loft_h_entries_of(r, &n);
        for (uint32_t i = 0; i < n; i++) done += loft_relayout_walk(T, ntypes, d->elem, all[i].e, budget);
        free(all);
        return done;
    }
    case LOFT_T_INDEX: {
        uint32_t n;
        uint32_t *recs = loft_bt_records(r, &n);
        for (uint32_t i = 0; i < n; i++) {
            loft_ref e = { r.store_nr, recs[i], 8 };
            done += loft_relayout_walk(T, ntypes, d->elem, e, budget);
        }
        free(recs);
        return done;
    }
    case LOFT_T_ORDERED: {
        cur = loft_collection_rec(s, r.rec, r.pos);
        if (cur == 0) return 0;
        for (uint32_t i = 0; i < loft_u32(s, cur, 4); i++) {
            loft_ref e = { r.store_nr, loft_u32(s, cur, 8 + 4 * i), 8 };
            if (e.rec != 0) done += loft_relayout_walk(T, ntypes, d->elem, e, budget);
        }
        return done;
    }
    default:
        return 0;
    }
}

/* A fresh store of `words` zeroed words in the lowest free slot, uncounted (`adopt_store`). */
static uint16_t loft_store_scratch(uint32_t words) {
    uint32_t nr = loft_store_make(0);
    loft_store_t *s = &loft_stores[nr];
    free(s->buf);
    s->size = words < 4 ? 4 : words;
    s->buf = (uint8_t *) calloc(s->size, 8);
    if (!s->buf) { fputs("loft: out of memory\n", stderr); abort(); }
    s->defined = 1;
    loft_st_init(s);
    return (uint16_t) nr;
}

/* `take_store` + `release_slot`: the slot is free again, holding the freed sentinel
 * (`Store::new_freed_sentinel`, four words, parking on) its next store is made in. */
static void loft_store_drop(uint16_t nr) {
    loft_store_t *s = &loft_stores[nr];
    free(s->buf);
    s->size = 4;
    s->buf = (uint8_t *) malloc((size_t) s->size * 8);
    if (!s->buf) { fputs("loft: out of memory\n", stderr); abort(); }
    s->live = 0;
    s->defined = 0;
    s->no_park = 0;
}

/* store_image_live_end: the word past the last live record, 0 when the blocks do not chain to
 * the end. */
static uint32_t loft_image_live_end(const loft_store_t *s) {
    uint32_t rec = 1, live_end = 1;
    while (rec < s->size) {
        int32_t sz = loft_st_i32(s, rec, 0);
        if (sz == 0) return 0;
        rec += (uint32_t) (sz < 0 ? -sz : sz);
        if (sz > 0) live_end = rec;
    }
    return rec == s->size ? live_end : 0;
}

static char *loft_cpath(loft_str p, const char *suffix) {
    size_t extra = suffix ? strlen(suffix) : 0;
    char *c = (char *) malloc(p.len + extra + 1);
    if (!c) { fputs("loft: out of memory\n", stderr); abort(); }
    if (p.len) memcpy(c, p.p, p.len);
    if (extra) memcpy(c + p.len, suffix, extra);
    c[p.len + extra] = 0;
    return c;
}

/* Stores::persist_copy (`store_persist_copy`): the collection rooted in `r`'s store rebuilt into
 * a scratch store, its trees laid out, written to `path` sized to its content, and the sidecar
 * beside it.  0 on a store with no root, a type no sidecar was prepared for, or an I/O error. */
static int loft_image_write(const loft_tdesc *T, uint32_t ntypes, loft_ref r, loft_str path, const loft_sidecar *sc) {
    if (r.store_nr == 0xFFFF || r.store_nr >= loft_nstores || !loft_stores[r.store_nr].live) return 0;
    uint16_t tp = loft_stores[r.store_nr].known;
    int32_t header = loft_st_i32(&loft_stores[r.store_nr], 1, 0);
    if (tp == 0xFFFF || header <= 0 || sc == 0 || sc->tp != tp) return 0;
    uint16_t dst = loft_store_scratch((uint32_t) header + 2);
    loft_store_t *d = &loft_stores[dst];
    if (loft_st_claim(d, (uint32_t) header) != 1) {
        loft_store_drop(dst);
        return 0;
    }
    int32_t dst_header = loft_st_i32(d, 1, 0);
    memcpy(d->buf + 8, loft_stores[r.store_nr].buf + 8, (size_t) header * 8);
    loft_st_set_i32(d, 1, 0, dst_header);
    loft_ref src = { r.store_nr, 1, 8 }, to = { dst, 1, 8 };
    loft_copy_claims(T, tp, src, to);
    d = &loft_stores[dst];
    d->known = tp;
    if (loft_type_has_tree(T, ntypes, tp, 0)) {
        uint32_t budget = 200000;
        (void) loft_relayout_walk(T, ntypes, tp, to, &budget);
    }
    d = &loft_stores[dst];
    uint32_t live_end = loft_image_live_end(d);
    int ok = 0;
    if (live_end != 0) {
        uint32_t target = live_end > LOFT_IMG_MIN_WORDS ? live_end : LOFT_IMG_MIN_WORDS;
        uint8_t *img = (uint8_t *) calloc(target, 8);
        if (!img) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(img, d->buf, (size_t) live_end * 8);
        if (target > live_end) {
            int32_t free_words = -(int32_t) (target - live_end);
            memcpy(img + (size_t) live_end * 8, &free_words, 4);
        }
        /* `@FR-S-Pair`: the image and its sidecar are both staged beside the target, and
         * only when both are written are they renamed into place, as persist_copy does. */
        char *p = loft_cpath(path, 0), *ptmp = loft_cpath(path, ".tmp");
        char *side = loft_cpath(path, ".dschema"), *stmp = loft_cpath(path, ".dschema.tmp");
        FILE *f = fopen(ptmp, "wb");
        if (f) {
            ok = fwrite(img, 8, target, f) == target;
            ok = fclose(f) == 0 && ok;
        }
        free(img);
        if (ok) {
            FILE *g = fopen(stmp, "wb");
            ok = g != 0;
            if (g) {
                ok = fprintf(g, "%sslot=%u\n%s", sc->before_slot, (unsigned) dst, sc->after_slot) > 0;
                ok = fclose(g) == 0 && ok;
                if (!ok) (void) remove(stmp);
            }
        }
        if (!ok) (void) remove(ptmp);
        else ok = rename(ptmp, p) == 0 && rename(stmp, side) == 0;
        free(p);
        free(ptmp);
        free(side);
        free(stmp);
    }
    loft_store_drop(dst);
    return ok;
}

/* release::version_triple: `X`, `X.Y` or `X.Y.Z`, missing parts 0; 0 when it is not one. */
static int loft_version_triple(const char *v, size_t n, unsigned long t[3]) {
    size_t i = 0;
    int part = 0;
    t[0] = t[1] = t[2] = 0;
    if (n == 0) return 0;
    while (i < n) {
        if (part > 2 || v[i] < '0' || v[i] > '9') return 0;
        while (i < n && v[i] >= '0' && v[i] <= '9') t[part] = t[part] * 10 + (unsigned long) (v[i++] - '0');
        if (i < n) {
            if (v[i] != '.') return 0;
            i++;
            part++;
        }
    }
    return 1;
}

/* The schema gate (`check_beside`, `classify`, `Release::admits`): the sidecar beside `path`
 * against the layout the program reads its store as.  1 when the image may be read as it is: its
 * sidecar names this layout.  `who` names the caller in what a refusal says. */
static int loft_image_gate(const char *path, const loft_sidecar *sc, const char *who) {
    char *side = (char *) malloc(strlen(path) + 9);
    if (!side) { fputs("loft: out of memory\n", stderr); abort(); }
    sprintf(side, "%s.dschema", path);
    FILE *f = fopen(side, "rb");
    free(side);
    if (!f) {
        /* `@FR-S-Gate`: an image with no sidecar records no layout and is never read raw. */
        fprintf(stderr, "%s: refusing %s — it has no layout sidecar (`.dschema`) beside it, so nothing "
                "records the layout its records were written with; write it with store_persist_copy, which "
                "writes both.\n", who, path);
        return 0;
    }
    size_t cap = 4096, n = 0, got;
    char *text = (char *) malloc(cap + 1);
    if (!text) { fputs("loft: out of memory\n", stderr); abort(); }
    while ((got = fread(text + n, 1, cap - n, f)) > 0) {
        n += got;
        if (n == cap) {
            cap *= 2;
            text = (char *) realloc(text, cap + 1);
            if (!text) { fputs("loft: out of memory\n", stderr); abort(); }
        }
    }
    fclose(f);
    text[n] = 0;
    /* `LayoutIdentity::from_sidecar`: the magic, the header lines, the `--` fence, the dump. */
    int ok = 0, ver = 0;
    unsigned long long hash = 0;
    int has_hash = 0;
    const char *version = "";
    size_t version_len = 0;
    char *fence = strstr(text, "\n--\n");
    if (fence && strncmp(text, "LOFT-DSCHEMA v", 14) == 0) {
        *fence = 0;
        ver = atoi(text + 14);
        ok = ver >= 1 && ver <= 2;
        for (char *line = strchr(text, '\n'); ok && line && line < fence;) {
            char *start = line + 1, *end = strchr(start, '\n');
            if (!end) end = fence;
            if (strncmp(start, "hash=", 5) == 0) {
                hash = strtoull(start + 5, 0, 10);
                has_hash = 1;
            } else if (strncmp(start, "version=", 8) == 0 && ver >= 2) {
                version = start + 8;
                version_len = (size_t) (end - version);
            } else if (!(strncmp(start, "schema=", 7) == 0 || (ver >= 2 && (strncmp(start, "root=", 5) == 0 || strncmp(start, "slot=", 5) == 0)))) {
                ok = start >= fence;
            }
            line = end < fence ? end : 0;
        }
        ok = ok && has_hash;
    }
    if (!ok) {
        fprintf(stderr, "%s: refusing %s — its layout sidecar cannot be read.\n", who, path);
        free(text);
        return 0;
    }
    char *dump = fence + 4;
    if (ver >= 2) {
        char *facts = strstr(dump, "--facts--\n");
        if (facts) *facts = 0;
    }
    unsigned long w[3], o[3];
    if (sc->oldest[0] && loft_version_triple(version, version_len, w) &&
        loft_version_triple(sc->oldest, strlen(sc->oldest), o) &&
        (w[0] < o[0] || (w[0] == o[0] && (w[1] < o[1] || (w[1] == o[1] && w[2] < o[2]))))) {
        fprintf(stderr,
                "%s: refusing %s — this store was written by %.*s, and this program reads stores from %s on; "
                "open it once with a release that reads both, then with this one.\n",
                who, path, (int) version_len, version, sc->oldest);
        free(text);
        return 0;
    }
    ok = hash == sc->hash && strcmp(dump, sc->dump) == 0;
    if (!ok) fprintf(stderr, "%s: refusing %s — it was written with a different layout, and it cannot be converted to this program's.\n", who, path);
    free(text);
    return ok;
}

/* Stores::load_path (`store_load`): the image at `path` replaces the store `r` names, the slot
 * keeping its type and its liveness; its allocator state is re-derived from the image's own
 * blocks (`Store::load`: no parked lists, no lazy frees).  0 when the gate refuses the file, it
 * cannot be read, or it is not a store image. */
static int loft_image_load(loft_ref r, loft_str path, const loft_sidecar *sc) {
    if (r.store_nr == 0xFFFF || r.store_nr >= loft_nstores) return 0;
    char *p = loft_cpath(path, 0);
    if (p[0] == 0 || (sc && loft_stores[r.store_nr].known == sc->tp && !loft_image_gate(p, sc, "store_load"))) {
        free(p);
        return 0;
    }
    FILE *f = fopen(p, "rb");
    free(p);
    if (!f) return 0;
    size_t cap = 1 << 16, n = 0, got;
    uint8_t *bytes = (uint8_t *) malloc(cap);
    if (!bytes) { fputs("loft: out of memory\n", stderr); abort(); }
    while ((got = fread(bytes + n, 1, cap - n, f)) > 0) {
        n += got;
        if (n == cap) {
            cap *= 2;
            bytes = (uint8_t *) realloc(bytes, cap);
            if (!bytes) { fputs("loft: out of memory\n", stderr); abort(); }
        }
    }
    fclose(f);
    uint32_t sig;
    if (n < 16 || (memcpy(&sig, bytes, 4), sig != LOFT_ST_SIGNATURE)) {
        free(bytes);
        return 0;
    }
    uint32_t words = (uint32_t) ((n + 7) / 8);
    if (words < 2) words = 2;
    bytes = (uint8_t *) realloc(bytes, (size_t) words * 8);
    if (!bytes) { fputs("loft: out of memory\n", stderr); abort(); }
    memset(bytes + n, 0, (size_t) words * 8 - n);
    loft_store_t *s = &loft_stores[r.store_nr];
    free(s->buf);
    s->buf = bytes;
    s->size = words;
    s->defined = 1;
    s->no_park = 1;
    s->lazy = 0;
    s->needs_coalesce = 0;
    s->claimed_end = 0;
    s->serial = 0;
    loft_bits_clear(&s->referenced);
    loft_bits_clear(&s->removed);
    loft_st_resync(s);
    return 1;
}

/* The drain the parser puts after every `store_load` (@PLN196): the records a CONVERTING load
 * owes a computed default.  A C load reads only an image of this program's own layout and never
 * converts, so it owes nothing (`Stores::owed_next` over an empty list). */
static int64_t loft_store_upgrade_owed_next(void) { return -1; }
static loft_ref loft_store_upgrade_owed_record(void) { return loft_ref_null(); }
static loft_ref loft_store_upgrade_owed_old(void) { return loft_ref_null(); }
static loft_ref loft_store_upgrade_owed_from(void) { return loft_ref_null(); }
static uint8_t loft_store_upgrade_owed_finish(void) { return 1; }
static void loft_store_upgrade_routine(int64_t tp, int64_t old, int64_t version) {
    (void) tp;
    (void) old;
    (void) version;
}

/* ── store_verify (Stores::verify_graph_ok, validate_claims) ────────────────────────────────
 * A bounds-checked walk of a value's claim graph: every number is guarded before it is
 * followed, a problem is reported on stderr and counted, and the walk stops past eight.  The
 * report names the value as `store_verify` rather than by its field path: the C's type table
 * carries no field names. */
static void loft_validate_claims(const loft_tdesc *T, uint32_t nt, uint16_t tp, loft_ref r, uint32_t *problems) {
    if (*problems > 8) return;
    if (r.store_nr >= loft_nstores) {
        fprintf(stderr, "[cr-check] store_verify: ref #%u.%u,%u \xe2\x80\x94 store out of range\n",
                (unsigned) r.store_nr, r.rec, r.pos);
        (*problems)++;
        return;
    }
    loft_store_t *s = &loft_stores[r.store_nr];
    if (!s->live) {
        fprintf(stderr, "[cr-check] store_verify: ref #%u.%u,%u \xe2\x80\x94 store FREED\n",
                (unsigned) r.store_nr, r.rec, r.pos);
        (*problems)++;
        return;
    }
    uint32_t cap = s->size;
    if (r.rec >= cap || (uint64_t) r.rec * 8 + r.pos >= (uint64_t) cap * 8) {
        fprintf(stderr, "[cr-check] store_verify: ref #%u.%u,%u \xe2\x80\x94 record beyond store capacity (%u words)\n",
                (unsigned) r.store_nr, r.rec, r.pos, cap);
        (*problems)++;
        return;
    }
    if (tp >= nt) {
        fprintf(stderr, "[cr-check] store_verify: type %u out of range\n", (unsigned) tp);
        (*problems)++;
        return;
    }
    const loft_tdesc *d = &T[tp];
    switch (d->kind) {
    case LOFT_T_TEXT: {
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur != 0 && cur >= cap) {
            fprintf(stderr, "[cr-check] store_verify: text offset %u beyond store #%u capacity %u\n",
                    cur, (unsigned) r.store_nr, cap);
            (*problems)++;
        }
        return;
    }
    case LOFT_T_STRUCT:
        for (uint16_t i = 0; i < d->n; i++) {
            loft_ref f = { r.store_nr, r.rec, r.pos + d->fields[i].pos };
            loft_validate_claims(T, nt, d->fields[i].type, f, problems);
        }
        return;
    case LOFT_T_VECTOR: {
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur == 0) return;
        if (cur >= cap) {
            fprintf(stderr, "[cr-check] store_verify: vector rec %u beyond store #%u capacity %u\n",
                    cur, (unsigned) r.store_nr, cap);
            (*problems)++;
            return;
        }
        uint32_t len = loft_u32(s, cur, 4), size = T[d->elem].size;
        if ((uint64_t) len * size > (uint64_t) cap * 8) {
            fprintf(stderr, "[cr-check] store_verify: vector rec %u len %u (elem size %u) exceeds store #%u capacity %u words\n",
                    cur, len, size, (unsigned) r.store_nr, cap);
            (*problems)++;
            return;
        }
        for (uint32_t i = 0; i < len && i < 16; i++) {
            loft_ref e = { r.store_nr, cur, 8 + size * i };
            loft_validate_claims(T, nt, d->elem, e, problems);
        }
        return;
    }
    case LOFT_T_CHILD: {
        uint32_t c = loft_u32(s, r.rec, r.pos);
        if (c == 0) return;
        if (c >= cap) {
            fprintf(stderr, "[cr-check] store_verify: child rec %u beyond store #%u capacity %u\n",
                    c, (unsigned) r.store_nr, cap);
            (*problems)++;
            return;
        }
        loft_ref e = { r.store_nr, c, 8 };
        loft_validate_claims(T, nt, d->elem, e, problems);
        return;
    }
    case LOFT_T_ENUM: {
        int e = (int) *loft_at(s, r.rec, r.pos) - 1;
        if (e >= 0 && e < d->n && d->variants[e] != 0xFFFF) loft_validate_claims(T, nt, d->variants[e], r, problems);
        return;
    }
    case LOFT_T_HASH: {
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur == 0) return;
        if (cur >= cap) {
            fprintf(stderr, "[cr-check] store_verify: hash bucket rec %u beyond store #%u capacity %u\n",
                    cur, (unsigned) r.store_nr, cap);
            (*problems)++;
            return;
        }
        uint32_t room = loft_rec_words(s, cur);
        if (room < LOFT_H_RESERVED || room > cap) {
            fprintf(stderr, "[cr-check] store_verify: hash bucket rec %u insane room %u (cap %u)\n", cur, room, cap);
            (*problems)++;
            return;
        }
        if (loft_h_is_inline(s, cur)) {
            uint32_t n;
            loft_hent *ents = loft_h_entries_of(r, &n);
            for (uint32_t i = 0; i < n; i++) loft_validate_claims(T, nt, d->elem, ents[i].e, problems);
            free(ents);
            return;
        }
        uint32_t sb = loft_h_slot_bytes(s, cur);
        uint32_t elms = (room - LOFT_H_RESERVED) * 8 / sb;
        uint32_t stride = loft_h_stride(s, cur);
        uint32_t dir = loft_u32(s, cur, LOFT_AR_DIR_FLD);
        if (stride != 0 && dir >= cap) {
            fprintf(stderr, "[cr-check] store_verify: hash arena directory %u beyond store #%u capacity %u\n",
                    dir, (unsigned) r.store_nr, cap);
            (*problems)++;
            return;
        }
        for (uint32_t i = 0; i < elms; i++) {
            uint32_t slot = loft_u32(s, cur, LOFT_H_BUCKET0 + i * sb), entry, pos;
            if (slot == 0) continue;
            if (stride == 0 || (slot & LOFT_H_SLOT_RECORD) != 0) {
                entry = slot & ~LOFT_H_SLOT_RECORD;
                pos = 8;
            } else {
                uint32_t k, off;
                loft_ar_locate(slot, stride, &k, &off);
                entry = loft_ar_chunk_rec(s, dir, k);
                if (entry == 0) continue;
                pos = off;
            }
            if (entry >= cap) {
                fprintf(stderr, "[cr-check] store_verify: hash entry rec %u beyond store #%u capacity %u\n",
                        entry, (unsigned) r.store_nr, cap);
                (*problems)++;
                if (*problems > 8) return;
                continue;
            }
            loft_ref e = { r.store_nr, entry, pos };
            loft_validate_claims(T, nt, d->elem, e, problems);
        }
        return;
    }
    case LOFT_T_ORDERED: {
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur == 0) return;
        if (cur >= cap) {
            fprintf(stderr, "[cr-check] store_verify: array rec %u beyond store #%u capacity %u\n",
                    cur, (unsigned) r.store_nr, cap);
            (*problems)++;
            return;
        }
        uint32_t len = loft_u32(s, cur, 4);
        if ((uint64_t) len * 4 > (uint64_t) cap * 8) {
            fprintf(stderr, "[cr-check] store_verify: array rec %u len %u exceeds store #%u capacity %u words\n",
                    cur, len, (unsigned) r.store_nr, cap);
            (*problems)++;
            return;
        }
        for (uint32_t i = 0; i < len && i < 16; i++) {
            uint32_t elm = loft_u32(s, cur, 8 + 4 * i);
            if (elm == 0) continue;
            if (elm >= cap) {
                fprintf(stderr, "[cr-check] store_verify[%u]: array element rec %u beyond store #%u capacity %u\n",
                        i, elm, (unsigned) r.store_nr, cap);
                (*problems)++;
                return;
            }
            loft_ref e = { r.store_nr, elm, 8 };
            loft_validate_claims(T, nt, d->elem, e, problems);
        }
        return;
    }
    case LOFT_T_RADIX:
    case LOFT_T_TRIE: {
        /* The leaves belong to the primary collection; the spine alone is checked. */
        uint32_t cur = loft_u32(s, r.rec, r.pos);
        if (cur != 0 && cur >= cap) {
            fprintf(stderr, "[cr-check] store_verify: tree rec %u beyond store #%u capacity %u\n",
                    cur, (unsigned) r.store_nr, cap);
            (*problems)++;
        }
        return;
    }
    case LOFT_T_DBREF: {
        uint32_t sn = loft_u32(s, r.rec, r.pos);
        if (sn != 0xFFFFu && sn >= loft_nstores) {
            fprintf(stderr, "[cr-check] store_verify: stored DbRef names store #%u, out of range\n", sn);
            (*problems)++;
        }
        return;
    }
    default:
        /* An index borrows its rows from the primary collection; a scalar holds no claim. */
        return;
    }
}

/* store_verify (Stores::verify_graph_ok): the walk over what `r` points at — the type its
 * store was minted as, or the field of it at `r`'s offset (`type_at`) — false for a store
 * minted as no type. */
static uint8_t loft_store_verify(const loft_tdesc *T, uint32_t nt, loft_ref r) {
    if (r.store_nr >= loft_nstores) return 0;
    uint16_t tp = loft_stores[r.store_nr].known;
    if (tp == 0xFFFF || tp >= nt) return 0;
    if (r.pos > 8 && T[tp].kind == LOFT_T_STRUCT) {
        for (uint16_t i = 0; i < T[tp].n; i++)
            if (T[tp].fields[i].pos == r.pos - 8 && !T[tp].fields[i].hidden) {
                tp = T[tp].fields[i].type;
                break;
            }
    }
    uint32_t problems = 0;
    loft_validate_claims(T, nt, tp, r, &problems);
    return problems == 0;
}
