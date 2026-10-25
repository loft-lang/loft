/* ── The tree's walks, over any source (radix_tree.rs `TreeNodes`, radix_db.rs `box_walk`) ────
 * Pasted twice: over a resident store (`loft_rx_*`) and over a paged image (`loft_prx_*`,
 * loft_paged.h), as the Rust walks are generic over the storage they read.  What a source
 * supplies, defined before each paste:
 *   LOFT_RXS             the source's type
 *   LOFT_RXF(name)       a walk's name over it
 *   LOFT_RX_RD(s, r, o)  the 4-byte word at byte `o` of record `r`
 *   LOFT_RX_BEGIN(s)     `walk_begin`: a walk starts (a paged source refills its fuel)
 *   LOFT_RX_GUARD(s, n)  whether node `n`'s child may be read (a paged source: fuel left, `n` a
 *                        node the tree minted); a refused read is the empty child
 *   LOFT_RX_KEYI(s, r, k)  integer key part `k` of record `r`'s payload
 *   LOFT_RX_KEYT(s, r, k)  text key part `k` of record `r`'s payload */

static uint32_t LOFT_RXF(node_bit)(LOFT_RXS *s, uint32_t t, uint32_t n) { return LOFT_RX_RD(s, t, loft_rx_node_off(n)); }
static uint32_t LOFT_RXF(node_parent)(LOFT_RXS *s, uint32_t t, uint32_t n) { return LOFT_RX_RD(s, t, loft_rx_node_off(n) + 4); }
static int32_t LOFT_RXF(child)(LOFT_RXS *s, uint32_t t, uint32_t n, int dir) {
    return LOFT_RX_GUARD(s, n) ? (int32_t) LOFT_RX_RD(s, t, loft_rx_child_off(n, dir)) : 0;
}
static int32_t LOFT_RXF(top)(LOFT_RXS *s, uint32_t t) { return (int32_t) LOFT_RX_RD(s, t, LOFT_RX_TOP); }
static uint32_t LOFT_RXF(len)(LOFT_RXS *s, uint32_t t) { return LOFT_RX_RD(s, t, LOFT_RX_LEN); }

/* A spatial record's axis codes (radix_db::axis_code). */
static void LOFT_RXF(axes)(LOFT_RXS *s, uint32_t rec, const loft_key *keys, int nk, uint64_t *out) {
    for (int a = 0; a < nk && a < LOFT_RX_MAX_AXES; a++) out[a] = loft_rx_coord(LOFT_RX_KEYI(s, rec, &keys[a]));
}

static loft_rx_key LOFT_RXF(rec_key)(LOFT_RXS *s, const loft_key *keys, int nk, int text, uint32_t rec) {
    loft_rx_key k;
    memset(&k, 0, sizeof k);
    k.keys = keys;
    k.nk = nk;
    k.text = text;
    k.rec = rec;
    k.id = rec;
    if (text) {
        loft_str t = LOFT_RX_KEYT(s, rec, &keys[0]);
        k.bytes = (const uint8_t *) t.p;
        k.len = t.len;
        k.bits = (uint32_t) t.len * 8;
    } else {
        LOFT_RXF(axes)(s, rec, keys, nk, k.codes);
        k.bits = LOFT_RX_AXIS_BITS * (uint32_t) nk;
    }
    return k;
}

/* radix_tree::descend_gen: the leaf a key's bits lead to, and its parent node. */
static uint32_t LOFT_RXF(descend)(LOFT_RXS *s, uint32_t t, const loft_rx_key *k, int32_t cur, uint32_t parent,
                                  uint32_t *leaf_parent) {
    LOFT_RX_BEGIN(s);
    for (;;) {
        if (cur == 0) {
            *leaf_parent = 0;
            return 0;
        }
        if (LOFT_RX_REC(cur)) {
            *leaf_parent = parent;
            return (uint32_t) cur;
        }
        uint32_t n = LOFT_RX_NODE_ID(cur);
        cur = LOFT_RXF(child)(s, t, n, loft_rx_bit(k, LOFT_RXF(node_bit)(s, t, n)));
        parent = n;
    }
}

/* radix_tree::descend_extreme_gen: the first (`dir` 0) or last record below `cur`. */
static uint32_t LOFT_RXF(extreme)(LOFT_RXS *s, uint32_t t, int32_t cur, int dir, uint32_t parent, uint32_t *leaf_parent) {
    LOFT_RX_BEGIN(s);
    for (;;) {
        if (cur == 0) {
            *leaf_parent = 0;
            return 0;
        }
        if (LOFT_RX_REC(cur)) {
            *leaf_parent = parent;
            return (uint32_t) cur;
        }
        parent = LOFT_RX_NODE_ID(cur);
        cur = LOFT_RXF(child)(s, t, parent, dir);
    }
}

/* radix_tree::split_point_gen: the node under which a key diverging at bit `d` hangs. */
static uint32_t LOFT_RXF(split_point)(LOFT_RXS *s, uint32_t t, const loft_rx_key *k, uint32_t d, int *dir) {
    int32_t cur = LOFT_RXF(top)(s, t);
    uint32_t parent = 0;
    *dir = 0;
    LOFT_RX_BEGIN(s);
    while (LOFT_RX_NODE(cur)) {
        uint32_t n = LOFT_RX_NODE_ID(cur);
        uint32_t bit = LOFT_RXF(node_bit)(s, t, n);
        if (bit > d) break;
        *dir = loft_rx_bit(k, bit);
        cur = LOFT_RXF(child)(s, t, n, *dir);
        parent = n;
    }
    return parent;
}

static int32_t LOFT_RXF(subtree_below)(LOFT_RXS *s, uint32_t t, uint32_t parent, int dir) {
    return parent == 0 ? LOFT_RXF(top)(s, t) : LOFT_RXF(child)(s, t, parent, dir);
}

static loft_rx_iter LOFT_RXF(at)(LOFT_RXS *s, uint32_t t, uint32_t rec, uint32_t node) {
    loft_rx_iter it;
    it.rec = rec;
    it.node = node;
    it.budget = LOFT_RXF(len)(s, t) + 1;
    return it;
}

static uint32_t LOFT_RXF(step)(LOFT_RXS *s, uint32_t t, loft_rx_iter *it, int forward) {
    LOFT_RX_BEGIN(s);
    if (it->rec == 0 || it->budget == 0) {
        it->rec = 0;
        return 0;
    }
    it->budget--;
    int32_t from = (int32_t) it->rec;
    uint32_t n = it->node;
    while (n != 0) {
        int came_true = LOFT_RXF(child)(s, t, n, 1) == from;
        if (came_true != forward) {
            it->rec = LOFT_RXF(extreme)(s, t, LOFT_RXF(child)(s, t, n, forward), !forward, n, &it->node);
            return it->rec;
        }
        from = -(int32_t) n;
        n = LOFT_RXF(node_parent)(s, t, n);
    }
    it->rec = 0;
    it->node = 0;
    return 0;
}

static loft_rx_iter LOFT_RXF(straight)(LOFT_RXS *s, uint32_t t, int dir) {
    uint32_t node, rec = LOFT_RXF(extreme)(s, t, LOFT_RXF(top)(s, t), dir, 0, &node);
    return LOFT_RXF(at)(s, t, rec, node);
}

/* radix_tree::seek_gen: an iterator on the first record at or past the probe; `*d` the bit the
 * probe first differs from its candidate on, or UINT32_MAX when it never does. */
static loft_rx_iter LOFT_RXF(seek)(LOFT_RXS *s, uint32_t t, const loft_rx_key *probe, uint32_t *d) {
    uint32_t cand_parent, bit;
    int probe_bit, dir;
    *d = UINT32_MAX;
    uint32_t cand = LOFT_RXF(descend)(s, t, probe, LOFT_RXF(top)(s, t), 0, &cand_parent);
    if (cand == 0) return LOFT_RXF(at)(s, t, 0, 0);
    loft_rx_key ck = LOFT_RXF(rec_key)(s, probe->keys, probe->nk, probe->text, cand);
    if (!loft_rx_first_diff(probe, &ck, &bit, &probe_bit)) return LOFT_RXF(at)(s, t, cand, cand_parent);
    *d = bit;
    uint32_t parent = LOFT_RXF(split_point)(s, t, probe, bit, &dir);
    int32_t sub = LOFT_RXF(subtree_below)(s, t, parent, dir);
    uint32_t node, rec = LOFT_RXF(extreme)(s, t, sub, probe_bit, parent, &node);
    loft_rx_iter it = LOFT_RXF(at)(s, t, rec, node);
    if (probe_bit) (void) LOFT_RXF(step)(s, t, &it, 1);
    return it;
}

/* radix_tree::rtree_get: the record whose whole user key is the probe's, 0 for none. */
static uint32_t LOFT_RXF(get)(LOFT_RXS *s, uint32_t t, const loft_rx_key *probe) {
    uint32_t d;
    loft_rx_iter it = LOFT_RXF(seek)(s, t, probe, &d);
    if (it.rec == 0) return 0;
    if (d != UINT32_MAX && d < probe->bits) return 0;
    loft_rx_key k = LOFT_RXF(rec_key)(s, probe->keys, probe->nk, probe->text, it.rec);
    return k.bits == probe->bits ? it.rec : 0;
}

/* radix_db::box_walk: the records inside the closed box `qlo`..`qhi` (axis codes), in Morton
 * order, up to `cap` — runs of the tree from each in-box seek point, jumping out-of-box gaps by
 * BIGMIN. */
static void LOFT_RXF(box_walk)(LOFT_RXS *s, uint32_t t, const loft_key *keys, int nk, const uint64_t *qlo,
                               const uint64_t *qhi, size_t cap, loft_u32s *out) {
    uint32_t d, n = (uint32_t) nk;
    if (cap == 0 || nk == 0 || nk > LOFT_RX_MAX_AXES) return;
    uint64_t hi_code[LOFT_RX_MAX_AXES], at[LOFT_RX_MAX_AXES];
    loft_rx_morton(n, qhi, hi_code);
    memcpy(at, qlo, sizeof at);
    uint32_t steps = LOFT_RXF(len)(s, t);
    steps = steps > UINT32_MAX - 4 ? UINT32_MAX : steps + 4;
    for (;;) {
        loft_rx_key probe;
        memset(&probe, 0, sizeof probe);
        probe.keys = keys;
        probe.nk = nk;
        memcpy(probe.codes, at, sizeof at);
        probe.bits = LOFT_RX_AXIS_BITS * n;
        loft_rx_iter it = LOFT_RXF(seek)(s, t, &probe, &d);
        uint32_t rec = it.rec;
        int jumped = 0;
        while (rec != 0) {
            if (out->n >= cap || steps == 0) return;
            steps--;
            uint64_t axes[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, code[LOFT_RX_MAX_AXES];
            LOFT_RXF(axes)(s, rec, keys, nk, axes);
            int inside = 1;
            for (uint32_t a = 0; a < n; a++) inside &= axes[a] >= qlo[a] && axes[a] <= qhi[a];
            if (inside) {
                loft_u32s_push(out, rec);
                rec = LOFT_RXF(step)(s, t, &it, 1);
                continue;
            }
            loft_rx_morton(n, axes, code);
            if (loft_rx_code_gt(code, hi_code, n)) return;
            if (!loft_rx_bigmin(axes, n, qlo, qhi, at)) return;
            jumped = 1;
            break;
        }
        if (!jumped) return;
    }
}

/* trie_db::prefix: the records whose text key starts with `pre`, in key order, up to `cap`. */
static void LOFT_RXF(prefix_walk)(LOFT_RXS *s, uint32_t t, const loft_key *keys, loft_str pre, size_t cap, loft_u32s *out) {
    uint32_t d;
    loft_rx_key probe;
    memset(&probe, 0, sizeof probe);
    probe.keys = keys;
    probe.nk = 1;
    probe.text = 1;
    probe.bytes = (const uint8_t *) pre.p;
    probe.len = pre.len;
    probe.bits = (uint32_t) pre.len * 8;
    loft_rx_iter it = LOFT_RXF(seek)(s, t, &probe, &d);
    for (uint32_t rec = it.rec; rec != 0 && out->n < cap; rec = LOFT_RXF(step)(s, t, &it, 1)) {
        loft_str k = LOFT_RX_KEYT(s, rec, &keys[0]);
        if (k.len < pre.len || (pre.len && memcmp(k.p, pre.p, pre.len) != 0)) break;
        loft_u32s_push(out, rec);
    }
}

#undef LOFT_RXS
#undef LOFT_RXF
#undef LOFT_RX_RD
#undef LOFT_RX_BEGIN
#undef LOFT_RX_GUARD
#undef LOFT_RX_KEYI
#undef LOFT_RX_KEYT
