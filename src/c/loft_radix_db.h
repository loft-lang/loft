/* ── Spatial and trie collections over a resident store (src/radix_db.rs, src/trie_db.rs) ──── */

static void loft_rx_set_child(loft_store_t *s, uint32_t t, uint32_t n, int dir, int32_t c) { loft_set_u32(s, t, loft_rx_child_off(n, dir), (uint32_t) c); }

static uint32_t loft_rx_cap_for_words(uint32_t words) {
    return words * 8 > LOFT_RX_HDR ? (words * 8 - LOFT_RX_HDR) / LOFT_RX_NODE_SIZE : 0;
}

/* radix_db::table_bytes / trie_db::table_bytes (`OpSizeSpatial`, `OpSizeTrie`): the tree's node
 * array, room included. */
static int64_t loft_rx_table_bytes(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    loft_store_t *s = loft_store_of(c);
    uint32_t t = loft_collection_rec(s, c.rec, c.pos);
    return t == 0 ? 0 : (int64_t) loft_u32(s, t, LOFT_RX_CAP) * LOFT_RX_NODE_SIZE;
}

static uint32_t loft_rx_init(loft_store_t *s, uint32_t initial) {
    uint32_t words = (LOFT_RX_HDR + LOFT_RX_NODE_SIZE * initial + 7) / 8;
    if (words < LOFT_RX_HDR / 8) words = LOFT_RX_HDR / 8;
    uint32_t t = loft_st_claim(s, words);
    uint32_t cap = loft_rx_cap_for_words(loft_rec_words(s, t));
    loft_set_u32(s, t, LOFT_RX_TOP, 0);
    loft_set_u32(s, t, LOFT_RX_LEN, 0);
    loft_set_u32(s, t, LOFT_RX_NODES, 0);
    loft_set_u32(s, t, LOFT_RX_CAP, cap);
    loft_set_u32(s, t, LOFT_RX_FREE, 0);
    return t;
}

/* A node: a freed one first, else the next, the record doubled (at least +2 words) when full. */
static uint32_t loft_rx_alloc_node(loft_store_t *s, uint32_t *t) {
    uint32_t free_n = loft_u32(s, *t, LOFT_RX_FREE);
    if (free_n != 0) {
        loft_set_u32(s, *t, LOFT_RX_FREE, loft_u32(s, *t, loft_rx_child_off(free_n, 0)));
        return free_n;
    }
    uint32_t used = loft_u32(s, *t, LOFT_RX_NODES);
    if (used >= loft_u32(s, *t, LOFT_RX_CAP)) {
        uint32_t words = loft_rec_words(s, *t);
        *t = loft_st_resize(s, *t, words * 2 > words + 2 ? words * 2 : words + 2);
        loft_set_u32(s, *t, LOFT_RX_CAP, loft_rx_cap_for_words(loft_rec_words(s, *t)));
    }
    loft_set_u32(s, *t, LOFT_RX_NODES, used + 1);
    return used + 1;
}

static void loft_rx_free_node(loft_store_t *s, uint32_t t, uint32_t node) {
    loft_set_u32(s, t, loft_rx_child_off(node, 0), loft_u32(s, t, LOFT_RX_FREE));
    loft_set_u32(s, t, LOFT_RX_FREE, node);
}

/* radix_tree::rtree_insert: the record linked in at the bit it first differs from its
 * candidate on; the tree record answered (a growth moves it). */
static uint32_t loft_rx_insert(loft_store_t *s, uint32_t t, uint32_t rec, const loft_key *keys, int nk, int text) {
    uint32_t parent_unused, d;
    int rec_bit, dir;
    if (loft_rx_len(s, t) == 0) {
        loft_set_u32(s, t, LOFT_RX_TOP, rec);
        loft_set_u32(s, t, LOFT_RX_LEN, 1);
        return t;
    }
    loft_rx_key ka = loft_rx_rec_key(s, keys, nk, text, rec);
    uint32_t cand = loft_rx_descend(s, t, &ka, loft_rx_top(s, t), 0, &parent_unused);
    if (cand == rec) return t;
    loft_rx_key kb = loft_rx_rec_key(s, keys, nk, text, cand);
    if (!loft_rx_first_diff(&ka, &kb, &d, &rec_bit)) return t;
    uint32_t parent = loft_rx_split_point(s, t, &ka, d, &dir);
    int32_t displaced = loft_rx_subtree_below(s, t, parent, dir);
    /* link_split */
    uint32_t len = loft_rx_len(s, t);
    uint32_t node = loft_rx_alloc_node(s, &t);
    loft_set_u32(s, t, loft_rx_node_off(node), d);
    loft_set_u32(s, t, loft_rx_node_off(node) + 4, parent);
    loft_rx_set_child(s, t, node, rec_bit, (int32_t) rec);
    loft_rx_set_child(s, t, node, !rec_bit, displaced);
    if (LOFT_RX_NODE(displaced)) loft_set_u32(s, t, loft_rx_node_off(LOFT_RX_NODE_ID(displaced)) + 4, node);
    if (parent == 0) loft_set_u32(s, t, LOFT_RX_TOP, (uint32_t) -(int32_t) node);
    else loft_rx_set_child(s, t, parent, dir, -(int32_t) node);
    loft_set_u32(s, t, LOFT_RX_LEN, len + 1);
    return t;
}

/* radix_tree::rtree_remove: the record unlinked, its parent node freed and its sibling moved up;
 * 0 when the tree does not hold it. */
static int loft_rx_remove(loft_store_t *s, uint32_t t, uint32_t rec, const loft_key *keys, int nk, int text) {
    uint32_t parent;
    loft_rx_key k = loft_rx_rec_key(s, keys, nk, text, rec);
    if (loft_rx_descend(s, t, &k, loft_rx_top(s, t), 0, &parent) != rec) return 0;
    loft_set_u32(s, t, LOFT_RX_LEN, loft_rx_len(s, t) - 1);
    if (parent == 0) {
        loft_set_u32(s, t, LOFT_RX_TOP, 0);
        return 1;
    }
    uint32_t grand = loft_rx_node_parent(s, t, parent);
    int rec_is_true = loft_rx_child(s, t, parent, 1) == (int32_t) rec;
    int32_t sibling = loft_rx_child(s, t, parent, !rec_is_true);
    if (grand == 0) loft_set_u32(s, t, LOFT_RX_TOP, (uint32_t) sibling);
    else loft_rx_set_child(s, t, grand, loft_rx_child(s, t, grand, 1) == -(int32_t) parent, sibling);
    if (LOFT_RX_NODE(sibling)) loft_set_u32(s, t, loft_rx_node_off(LOFT_RX_NODE_ID(sibling)) + 4, grand);
    loft_rx_free_node(s, t, parent);
    return 1;
}

/* ── the collections over it (radix_db.rs, trie_db.rs) ── */

static uint32_t loft_rx_tree(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    return loft_collection_rec(loft_store_of(c), c.rec, c.pos);
}

/* radix_db::add / trie_db::add: the record into the tree, made on first use. */
static void loft_rx_add(loft_ref c, uint32_t rec, const loft_key *keys, int nk, int text) {
    loft_store_t *s = loft_store_of(c);
    uint32_t t = loft_collection_rec(s, c.rec, c.pos);
    if (t == 0) {
        t = loft_rx_init(s, 0);
        loft_set_u32(s, c.rec, c.pos, t);
    }
    t = loft_rx_insert(s, t, rec, keys, nk, text);
    loft_set_u32(s, c.rec, c.pos, t);
}

static int64_t loft_rx_count(loft_ref c) {
    uint32_t t = loft_rx_tree(c);
    return t == 0 ? 0 : (int64_t) loft_rx_len(loft_store_of(c), t);
}

/* A probe from key values: a spatial one's axis codes, a trie's text. */
static loft_rx_key loft_rx_probe(const loft_key *keys, int nk, int text, const loft_kv *kv, int n) {
    loft_rx_key k;
    memset(&k, 0, sizeof k);
    k.keys = keys;
    k.nk = nk;
    k.text = text;
    if (text) {
        if (n > 0 && kv[0].kind == LOFT_KV_TEXT) {
            k.bytes = (const uint8_t *) kv[0].s.p;
            k.len = kv[0].s.len;
        }
        k.bits = (uint32_t) k.len * 8;
    } else {
        for (int a = 0; a < n && a < LOFT_RX_MAX_AXES; a++) k.codes[a] = kv[a].kind == LOFT_KV_INT ? loft_rx_coord(kv[a].i) : 0;
        k.nk = n;
        k.bits = LOFT_RX_AXIS_BITS * (uint32_t) nk;
    }
    return k;
}

/* radix_db::find / trie_db::find: the record under the key, or the null reference. */
static loft_ref loft_rx_find(loft_ref c, const loft_key *keys, int nk, int text, const loft_kv *kv, int n) {
    uint32_t t = loft_rx_tree(c);
    if (t == 0) return loft_ref_null();
    loft_rx_key p = loft_rx_probe(keys, nk, text, kv, n);
    loft_ref e = { c.store_nr, loft_rx_get(loft_store_of(c), t, &p), 8 };
    return e.rec == 0 ? loft_ref_null() : e;
}

/* The records in key order, malloc'd; `*n` their count. */
static uint32_t *loft_rx_records(loft_ref c, uint32_t *n) {
    uint32_t t = loft_rx_tree(c);
    uint32_t *out = (uint32_t *) malloc(((size_t) loft_rx_count(c) + 1) * 4);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    *n = 0;
    if (t == 0) return out;
    loft_store_t *s = loft_store_of(c);
    loft_rx_iter it = loft_rx_straight(s, t, 0);
    for (uint32_t r = it.rec; r != 0; r = loft_rx_step(s, t, &it, 1)) out[(*n)++] = r;
    return out;
}

/* radix_db::near_range: the records outward from a point in Morton order — at each step the
 * nearer of the next one up and the next one down — up to `cap`.  malloc'd. */
static uint32_t *loft_rx_near(loft_ref c, const loft_key *keys, int nk, const int64_t *from, size_t cap, uint32_t *count) {
    uint32_t t = loft_rx_tree(c), d;
    uint32_t *out = (uint32_t *) malloc(((size_t) loft_rx_count(c) + 1) * 4);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    *count = 0;
    if (t == 0 || nk == 0 || nk > LOFT_RX_MAX_AXES) return out;
    loft_store_t *s = loft_store_of(c);
    uint64_t qc[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, query[LOFT_RX_MAX_AXES];
    for (int a = 0; a < nk; a++) qc[a] = loft_rx_coord(from[a]);
    loft_rx_morton((uint32_t) nk, qc, query);
    loft_rx_key probe;
    memset(&probe, 0, sizeof probe);
    probe.keys = keys;
    probe.nk = nk;
    memcpy(probe.codes, qc, sizeof qc);
    probe.bits = LOFT_RX_AXIS_BITS * (uint32_t) nk;
    loft_rx_iter fore = loft_rx_seek(s, t, &probe, &d), back;
    if (fore.rec == 0) {
        back = loft_rx_straight(s, t, 1);
    } else {
        back = fore;
        (void) loft_rx_step(s, t, &back, 0);
    }
    while (*count < cap) {
        uint32_t b = back.rec, f = fore.rec;
        if (b == 0 && f == 0) break;
        int take_fore;
        if (b == 0) take_fore = 1;
        else if (f == 0) take_fore = 0;
        else {
            uint64_t ca[LOFT_RX_MAX_AXES], cb[LOFT_RX_MAX_AXES], ga[LOFT_RX_MAX_AXES], gb[LOFT_RX_MAX_AXES];
            uint64_t ax[LOFT_RX_MAX_AXES] = { 0, 0, 0 };
            loft_rx_axes(s, b, keys, nk, ax);
            loft_rx_morton((uint32_t) nk, ax, cb);
            loft_rx_axes(s, f, keys, nk, ax);
            loft_rx_morton((uint32_t) nk, ax, ca);
            loft_rx_abs_diff(cb, query, (uint32_t) nk, gb);
            loft_rx_abs_diff(ca, query, (uint32_t) nk, ga);
            take_fore = loft_rx_code_gt(gb, ga, (uint32_t) nk);
        }
        if (take_fore) {
            (void) loft_rx_step(s, t, &fore, 1);
            out[(*count)++] = f;
        } else {
            (void) loft_rx_step(s, t, &back, 0);
            out[(*count)++] = b;
        }
    }
    return out;
}

/* radix_db::box_range: the records inside the closed box, in Morton order, up to `cap`.
 * malloc'd. */
static uint32_t *loft_rx_box(loft_ref c, const loft_key *keys, int nk, const int64_t *from, const int64_t *till, size_t cap,
                             uint32_t *count) {
    uint32_t t = loft_rx_tree(c);
    loft_u32s out = loft_u32s_new((size_t) loft_rx_count(c) + 1);
    if (t != 0 && nk > 0 && nk <= LOFT_RX_MAX_AXES) {
        uint64_t qlo[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, qhi[LOFT_RX_MAX_AXES] = { 0, 0, 0 };
        loft_rx_corners(nk, from, till, qlo, qhi);
        loft_rx_box_walk(loft_store_of(c), t, keys, nk, qlo, qhi, cap, &out);
    }
    *count = (uint32_t) out.n;
    return out.p;
}

/* trie_db::prefix: the records whose key starts with `pre`, in key order, up to `cap`.
 * malloc'd. */
static uint32_t *loft_rx_prefix(loft_ref c, const loft_key *keys, loft_str pre, size_t cap, uint32_t *count) {
    uint32_t t = loft_rx_tree(c);
    loft_u32s out = loft_u32s_new((size_t) loft_rx_count(c) + 1);
    if (t != 0) loft_rx_prefix_walk(loft_store_of(c), t, keys, pre, cap, &out);
    *count = (uint32_t) out.n;
    return out.p;
}

/* ── the Stores routines over a spatial or a trie (src/database) ── */

/* Stores::unlink on a spatial or a trie: out of every walk over it, then out of the tree. */
static int loft_rx_unlink(loft_ref c, loft_ref e, const loft_key *keys, int nk, int text) {
    uint32_t t = loft_rx_tree(c);
    loft_forget_in_scratches(c, e);
    return t != 0 && loft_rx_remove(loft_store_of(c), t, e.rec, keys, nk, text);
}

/* Stores::remove_owned: unlinked, then its claims and its record released; a `view` member
 * only unlinks. */
static void loft_rx_remove_owned(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, const loft_key *keys, int nk,
                                 int text, int view) {
    if (e.rec == 0 || c.store_nr == 0xFFFF || c.rec == 0) return;
    loft_ref rec = { e.store_nr, e.rec, 8 };
    (void) loft_rx_unlink(c, rec, keys, nk, text);
    if (view) return;
    loft_remove_claims(T, elem, rec);
    loft_st_delete(loft_store_of(c), e.rec);
}

/* Stores::insert_record on a spatial (two records may share a point: no displacement) or a trie
 * (`@FR-Col-Insert`: the record under the key is displaced first — released unless
 * `secondary`).  The displaced record answered, or the null reference. */
static loft_ref loft_rx_insert_record(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref rec, const loft_key *keys,
                                      int nk, int text, int secondary) {
    loft_ref displaced = loft_ref_null();
    if (c.store_nr == 0xFFFF || c.rec == 0 || rec.rec == 0) return displaced;
    if (text) {
        loft_kv key = loft_key_get(loft_store_of(rec), rec.rec, 8, &keys[0]);
        loft_ref existing = loft_rx_find(c, keys, nk, 1, &key, 1);
        if (existing.rec != 0 && existing.rec != rec.rec) {
            (void) loft_rx_unlink(c, existing, keys, nk, 1);
            if (!secondary) {
                loft_remove_claims(T, elem, existing);
                loft_st_delete(loft_store_of(c), existing.rec);
            }
            displaced = existing;
        }
    }
    loft_rx_add(c, rec.rec, keys, nk, text);
    return displaced;
}

/* Stores::copy_claims_radix_body / copy_claims_trie_body: each record of the source, in tree
 * order, copied into a record of the destination's and filed there. */
static void loft_rx_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    uint32_t cur = loft_u32(loft_store_of(from), from.rec, from.pos);
    loft_set_u32(loft_store_of(to), to.rec, to.pos, cur == 0xFFFFFFFFu ? cur : 0);
    if (cur == 0 || cur == 0xFFFFFFFFu) return;
    uint32_t n, size = T[d->elem].size;
    uint32_t *recs = loft_rx_records(from, &n);
    for (uint32_t i = 0; i < n; i++) {
        loft_store_t *ts = loft_store_of(to);
        loft_ref a = { from.store_nr, recs[i], 8 };
        loft_ref b = { to.store_nr, loft_st_claim(ts, 1 + (size + 7) / 8), 8 };
        loft_set_u32(ts, b.rec, 4, to.rec);
        loft_copy_bytes(b, 0, a, 0, size);
        loft_copy_claims(T, d->elem, a, b);
        loft_rx_add(to, b.rec, d->keys, d->nk, d->kind == LOFT_T_TRIE);
    }
    free(recs);
}

/* Stores::remove_claims on a spatial or a trie field: every record in tree order released, then
 * the tree; a `borrowed` view releases its tree only. */
static void loft_rx_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed) {
    loft_store_t *s = loft_store_of(r);
    uint32_t cur = loft_u32(s, r.rec, r.pos);
    if (cur != 0 && cur != 0xFFFFFFFFu) {
        if (!borrowed) {
            uint32_t n;
            uint32_t *recs = loft_rx_records(r, &n);
            for (uint32_t i = 0; i < n; i++) {
                loft_ref e = { r.store_nr, recs[i], 8 };
                loft_remove_claims(T, T[tp].elem, e);
                loft_st_delete(s, recs[i]);
            }
            free(recs);
        }
        loft_st_delete(s, cur);
    }
    loft_set_u32(s, r.rec, r.pos, 0);
}

/* radix_sorted, spatial_range and trie_prefix (Stores::build_radix_sorted_vec,
 * build_radix_range_vec, build_trie_prefix_vec): a walk's snapshot of record numbers. */
static loft_ref loft_radix_sorted(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    uint32_t n;
    uint32_t *recs = loft_rx_records(c, &n);
    loft_ref x = loft_rec_scratch(c, recs, n);
    free(recs);
    return x;
}

static loft_ref loft_spatial_range(loft_ref c, const loft_key *keys, int nk, int64_t fx, int64_t fy, int64_t fz,
                                   int64_t has_till, int64_t tx, int64_t ty, int64_t tz, int64_t limit) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    int64_t from[3] = { fx, fy, fz }, till[3] = { tx, ty, tz };
    int n = nk < LOFT_RX_MAX_AXES ? nk : LOFT_RX_MAX_AXES;
    uint32_t count;
    uint32_t *recs = has_till != 0 ? loft_rx_box(c, keys, n, from, till, loft_rx_cap(limit), &count)
                                   : loft_rx_near(c, keys, n, from, loft_rx_cap(limit), &count);
    loft_ref x = loft_rec_scratch(c, recs, count);
    free(recs);
    return x;
}

static loft_ref loft_trie_prefix(loft_ref c, const loft_key *keys, loft_str pre, int64_t limit) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    uint32_t count;
    uint32_t *recs = loft_rx_prefix(c, keys, pre, loft_rx_cap(limit), &count);
    loft_ref x = loft_rec_scratch(c, recs, count);
    free(recs);
    return x;
}
