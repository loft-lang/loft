/* ── Spatial and trie: a PATRICIA tree over a bit-key oracle (src/radix_tree.rs) ────────────
 * loft's own layout, ported line for line.  The collection slot names the TREE record: @4 the
 * top child, @8 the records it holds, @12 the nodes ever minted, @16 the node capacity, @20 the
 * free-node list, and from @24 the nodes, 16 bytes each — @0 the absolute bit it splits on, @4
 * its parent, @8 / @12 its false / true child.  A child is 0 (empty), a record number (> 0) or
 * a node (its negated id).  The tree keys on `user bits ‖ 0x00 ‖ 32-bit record id ‖ zeros`, so
 * no key ends and distinct records always differ.  A spatial collection's user bits are its
 * axes' Morton interleave (src/radix_db.rs); a trie's are its text key's bytes (src/trie_db.rs)
 * (`@FR-S-Tree`, `@FR-S-TreeKey`).
 * An entry is a record of its own, its payload at byte 8. */

#define LOFT_RX_TERM_BITS 8u
#define LOFT_RX_ID_BITS 32u
#define LOFT_RX_SUFFIX_BITS (LOFT_RX_TERM_BITS + LOFT_RX_ID_BITS)
#define LOFT_RX_TOP 4u
#define LOFT_RX_LEN 8u
#define LOFT_RX_NODES 12u
#define LOFT_RX_CAP 16u
#define LOFT_RX_FREE 20u
#define LOFT_RX_HDR 24u
#define LOFT_RX_NODE_SIZE 16u
#define LOFT_RX_AXIS_BITS 64u
#define LOFT_RX_MAX_AXES 3

static uint32_t loft_rx_node_off(uint32_t node) { return LOFT_RX_HDR + LOFT_RX_NODE_SIZE * (node - 1); }
static uint32_t loft_rx_child_off(uint32_t node, int dir) { return loft_rx_node_off(node) + (dir ? 12 : 8); }
static uint32_t loft_rx_node_bit(loft_store_t *s, uint32_t t, uint32_t n) { return loft_u32(s, t, loft_rx_node_off(n)); }
static uint32_t loft_rx_node_parent(loft_store_t *s, uint32_t t, uint32_t n) { return loft_u32(s, t, loft_rx_node_off(n) + 4); }
static int32_t loft_rx_child(loft_store_t *s, uint32_t t, uint32_t n, int dir) { return (int32_t) loft_u32(s, t, loft_rx_child_off(n, dir)); }
static void loft_rx_set_child(loft_store_t *s, uint32_t t, uint32_t n, int dir, int32_t c) { loft_set_u32(s, t, loft_rx_child_off(n, dir), (uint32_t) c); }
static int32_t loft_rx_top(loft_store_t *s, uint32_t t) { return (int32_t) loft_u32(s, t, LOFT_RX_TOP); }
static uint32_t loft_rx_len(loft_store_t *s, uint32_t t) { return loft_u32(s, t, LOFT_RX_LEN); }
#define LOFT_RX_REC(c) ((c) > 0)
#define LOFT_RX_NODE(c) ((c) < 0)
#define LOFT_RX_NODE_ID(c) ((uint32_t) -(c))

/* How the tree reads a key: a record's through its collection's oracle — the Morton code of its
 * axes (`text` 0) or the bytes of its text key (`text` 1) — or a probe's own words. */
typedef struct {
    loft_store_t *s;
    const loft_key *keys;
    int nk, text;
    uint32_t rec;              /* a record's key, or 0 for a probe */
    uint64_t codes[LOFT_RX_MAX_AXES];
    const uint8_t *bytes;      /* a text probe */
    size_t len;
    uint32_t bits, id;
} loft_rx_key;

/* radix_db::coord_code: an axis value with its sign bit flipped, so unsigned order is its order. */
static uint64_t loft_rx_coord(int64_t v) { return (uint64_t) v ^ (UINT64_C(1) << 63); }

/* radix_db::interleave: 64 bits of the Morton code of `n` axis codes from bit `word * 64`. */
static uint64_t loft_rx_interleave(uint32_t word, uint32_t n, const uint64_t *codes) {
    uint64_t out = 0;
    uint32_t total = LOFT_RX_AXIS_BITS * n;
    for (uint32_t i = 0; i < 64; i++) {
        uint32_t b = word * 64 + i;
        if (b >= total) break;
        uint64_t bit = (codes[b % n] >> (LOFT_RX_AXIS_BITS - 1 - b / n)) & 1;
        out |= bit << (63 - i);
    }
    return out;
}

/* trie_db::bytes_word: the `word`-th big-endian 8-byte chunk, zero-padded past the end. */
static uint64_t loft_rx_bytes_word(const uint8_t *b, size_t len, uint32_t word) {
    uint64_t out = 0;
    for (uint32_t i = 0; i < 8; i++) {
        size_t idx = (size_t) word * 8 + i;
        out = out << 8 | (idx < len ? b[idx] : 0);
    }
    return out;
}

/* A spatial record's axis codes (radix_db::axis_code). */
static void loft_rx_axes(loft_store_t *s, uint32_t rec, const loft_key *keys, int nk, uint64_t *out) {
    for (int a = 0; a < nk && a < LOFT_RX_MAX_AXES; a++) out[a] = loft_rx_coord(loft_key_get(s, rec, 8, &keys[a]).i);
}

static loft_rx_key loft_rx_rec_key(loft_store_t *s, const loft_key *keys, int nk, int text, uint32_t rec) {
    loft_rx_key k;
    memset(&k, 0, sizeof k);
    k.s = s;
    k.keys = keys;
    k.nk = nk;
    k.text = text;
    k.rec = rec;
    k.id = rec;
    if (text) {
        loft_str t = loft_bt_key_text(s, rec, &keys[0]);
        k.bytes = (const uint8_t *) t.p;
        k.len = t.len;
        k.bits = (uint32_t) t.len * 8;
    } else {
        loft_rx_axes(s, rec, keys, nk, k.codes);
        k.bits = LOFT_RX_AXIS_BITS * (uint32_t) nk;
    }
    return k;
}

static uint64_t loft_rx_word(const loft_rx_key *k, uint32_t w) {
    if (k->text) return loft_rx_bytes_word(k->bytes, k->len, w);
    return loft_rx_interleave(w, (uint32_t) k->nk, k->codes);
}

/* radix_tree::composed_bit: bit `b` of `user bits ‖ 0x00 ‖ id ‖ zeros`. */
static int loft_rx_bit(const loft_rx_key *k, uint32_t b) {
    if (b < k->bits) return (int) ((loft_rx_word(k, b / 64) >> (63 - b % 64)) & 1);
    if (b < k->bits + LOFT_RX_TERM_BITS) return 0;
    if (b < k->bits + LOFT_RX_SUFFIX_BITS) return (int) ((k->id >> (LOFT_RX_ID_BITS - 1 - (b - k->bits - LOFT_RX_TERM_BITS))) & 1);
    return 0;
}

static uint32_t loft_rx_clz64(uint64_t x) {
    uint32_t n = 0;
    while (!(x & (UINT64_C(1) << 63))) {
        x <<= 1;
        n++;
    }
    return n;
}

/* radix_tree::first_diff_words: the first bit two composed keys differ on, and `a`'s value
 * there; 0 when they never differ. */
static int loft_rx_first_diff(const loft_rx_key *a, const loft_rx_key *b, uint32_t *bit, int *a_bit) {
    uint32_t common = a->bits < b->bits ? a->bits : b->bits;
    for (uint32_t w = 0; w * 64 < common; w++) {
        uint64_t x = loft_rx_word(a, w) ^ loft_rx_word(b, w);
        if (x != 0) {
            uint32_t at = w * 64 + loft_rx_clz64(x);
            if (at < common) {
                *bit = at;
                *a_bit = loft_rx_bit(a, at);
                return 1;
            }
            break;
        }
    }
    uint32_t la = a->bits + LOFT_RX_SUFFIX_BITS, lb = b->bits + LOFT_RX_SUFFIX_BITS;
    uint32_t limit = la > lb ? la : lb;
    for (uint32_t at = common; at < limit; at++) {
        int x = loft_rx_bit(a, at);
        if (x != loft_rx_bit(b, at)) {
            *bit = at;
            *a_bit = x;
            return 1;
        }
    }
    return 0;
}

/* radix_tree::descend_gen: the leaf a key's bits lead to, and its parent node. */
static uint32_t loft_rx_descend(loft_store_t *s, uint32_t t, const loft_rx_key *k, int32_t cur, uint32_t parent,
                                uint32_t *leaf_parent) {
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
        cur = loft_rx_child(s, t, n, loft_rx_bit(k, loft_rx_node_bit(s, t, n)));
        parent = n;
    }
}

/* radix_tree::descend_extreme_gen: the first (`dir` 0) or last record below `cur`. */
static uint32_t loft_rx_extreme(loft_store_t *s, uint32_t t, int32_t cur, int dir, uint32_t parent, uint32_t *leaf_parent) {
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
        cur = loft_rx_child(s, t, parent, dir);
    }
}

/* radix_tree::split_point_gen: the node under which a key diverging at bit `d` hangs. */
static uint32_t loft_rx_split_point(loft_store_t *s, uint32_t t, const loft_rx_key *k, uint32_t d, int *dir) {
    int32_t cur = loft_rx_top(s, t);
    uint32_t parent = 0;
    *dir = 0;
    while (LOFT_RX_NODE(cur)) {
        uint32_t n = LOFT_RX_NODE_ID(cur);
        uint32_t bit = loft_rx_node_bit(s, t, n);
        if (bit > d) break;
        *dir = loft_rx_bit(k, bit);
        cur = loft_rx_child(s, t, n, *dir);
        parent = n;
    }
    return parent;
}

static int32_t loft_rx_subtree_below(loft_store_t *s, uint32_t t, uint32_t parent, int dir) {
    return parent == 0 ? loft_rx_top(s, t) : loft_rx_child(s, t, parent, dir);
}

/* radix_tree::RadixIter: a record and its parent node, and a step budget against a corrupt
 * tree. */
typedef struct { uint32_t rec, node, budget; } loft_rx_iter;

static loft_rx_iter loft_rx_at(loft_store_t *s, uint32_t t, uint32_t rec, uint32_t node) {
    loft_rx_iter it;
    it.rec = rec;
    it.node = node;
    it.budget = loft_rx_len(s, t) + 1;
    return it;
}

static uint32_t loft_rx_step(loft_store_t *s, uint32_t t, loft_rx_iter *it, int forward) {
    if (it->rec == 0 || it->budget == 0) {
        it->rec = 0;
        return 0;
    }
    it->budget--;
    int32_t from = (int32_t) it->rec;
    uint32_t n = it->node;
    while (n != 0) {
        int came_true = loft_rx_child(s, t, n, 1) == from;
        if (came_true != forward) {
            it->rec = loft_rx_extreme(s, t, loft_rx_child(s, t, n, forward), !forward, n, &it->node);
            return it->rec;
        }
        from = -(int32_t) n;
        n = loft_rx_node_parent(s, t, n);
    }
    it->rec = 0;
    it->node = 0;
    return 0;
}

static loft_rx_iter loft_rx_straight(loft_store_t *s, uint32_t t, int dir) {
    uint32_t node, rec = loft_rx_extreme(s, t, loft_rx_top(s, t), dir, 0, &node);
    return loft_rx_at(s, t, rec, node);
}

/* radix_tree::seek_gen: an iterator on the first record at or past the probe; `*d` the bit the
 * probe first differs from its candidate on, or UINT32_MAX when it never does. */
static loft_rx_iter loft_rx_seek(loft_store_t *s, uint32_t t, const loft_rx_key *probe, uint32_t *d) {
    uint32_t cand_parent, bit;
    int probe_bit, dir;
    *d = UINT32_MAX;
    uint32_t cand = loft_rx_descend(s, t, probe, loft_rx_top(s, t), 0, &cand_parent);
    if (cand == 0) return loft_rx_at(s, t, 0, 0);
    loft_rx_key ck = loft_rx_rec_key(s, probe->keys, probe->nk, probe->text, cand);
    if (!loft_rx_first_diff(probe, &ck, &bit, &probe_bit)) return loft_rx_at(s, t, cand, cand_parent);
    *d = bit;
    uint32_t parent = loft_rx_split_point(s, t, probe, bit, &dir);
    int32_t sub = loft_rx_subtree_below(s, t, parent, dir);
    uint32_t node, rec = loft_rx_extreme(s, t, sub, probe_bit, parent, &node);
    loft_rx_iter it = loft_rx_at(s, t, rec, node);
    if (probe_bit) (void) loft_rx_step(s, t, &it, 1);
    return it;
}

/* radix_tree::rtree_get: the record whose whole user key is the probe's, 0 for none. */
static uint32_t loft_rx_get(loft_store_t *s, uint32_t t, const loft_rx_key *probe) {
    uint32_t d;
    loft_rx_iter it = loft_rx_seek(s, t, probe, &d);
    if (it.rec == 0) return 0;
    if (d != UINT32_MAX && d < probe->bits) return 0;
    loft_rx_key k = loft_rx_rec_key(s, probe->keys, probe->nk, probe->text, it.rec);
    return k.bits == probe->bits ? it.rec : 0;
}

static uint32_t loft_rx_cap_for_words(uint32_t words) {
    return words * 8 > LOFT_RX_HDR ? (words * 8 - LOFT_RX_HDR) / LOFT_RX_NODE_SIZE : 0;
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
static loft_rx_key loft_rx_probe(loft_store_t *s, const loft_key *keys, int nk, int text, const loft_kv *kv, int n) {
    loft_rx_key k;
    memset(&k, 0, sizeof k);
    k.s = s;
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
    loft_rx_key p = loft_rx_probe(loft_store_of(c), keys, nk, text, kv, n);
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

/* Morton words of `n` axis codes, compared and subtracted as one number (radix_db). */
static void loft_rx_morton(uint32_t n, const uint64_t *codes, uint64_t *words) {
    for (uint32_t w = 0; w < n; w++) words[w] = loft_rx_interleave(w, n, codes);
}

static int loft_rx_code_gt(const uint64_t *a, const uint64_t *b, uint32_t n) {
    for (uint32_t i = 0; i < n; i++) {
        if (a[i] != b[i]) return a[i] > b[i];
    }
    return 0;
}

static void loft_rx_abs_diff(const uint64_t *a, const uint64_t *b, uint32_t n, uint64_t *out) {
    const uint64_t *hi = loft_rx_code_gt(a, b, n) ? a : b, *lo = hi == a ? b : a;
    uint64_t borrow = 0;
    for (uint32_t i = n; i-- > 0;) {
        uint64_t d = hi[i] - lo[i];
        int b1 = hi[i] < lo[i];
        int b2 = d < borrow;
        out[i] = d - borrow;
        borrow = (uint64_t) (b1 || b2);
    }
}

static size_t loft_rx_cap(int64_t limit) {
    if (limit == INT64_MAX) return SIZE_MAX;
    return limit < 0 ? 0 : (size_t) limit;
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
    probe.s = s;
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

static uint64_t loft_rx_one_below_zero(uint64_t v, uint32_t p) {
    uint64_t mask = p == 63 ? UINT64_MAX : (UINT64_C(1) << (p + 1)) - 1;
    return (v & ~mask) | (UINT64_C(1) << p);
}

static uint64_t loft_rx_zero_below_one(uint64_t v, uint32_t p) {
    uint64_t mask = p == 63 ? UINT64_MAX : (UINT64_C(1) << (p + 1)) - 1;
    return (v & ~mask) | (mask >> 1);
}

/* radix_db::bigmin: the least Morton point past `cur` inside the box, 0 when none is. */
static int loft_rx_bigmin(const uint64_t *cur, uint32_t n, const uint64_t *qlo, const uint64_t *qhi, uint64_t *best) {
    uint64_t lo[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, hi[LOFT_RX_MAX_AXES] = { 0, 0, 0 };
    int found = 0;
    memcpy(lo, qlo, sizeof lo);
    memcpy(hi, qhi, sizeof hi);
    for (uint32_t b = 0; b < LOFT_RX_AXIS_BITS * n; b++) {
        uint32_t a = b % n, p = LOFT_RX_AXIS_BITS - 1 - b / n;
        int c = (int) ((cur[a] >> p) & 1), l = (int) ((lo[a] >> p) & 1), h = (int) ((hi[a] >> p) & 1);
        if (c == 0 && l == 0 && h == 1) {
            memcpy(best, lo, sizeof lo);
            best[a] = loft_rx_one_below_zero(lo[a], p);
            found = 1;
            hi[a] = loft_rx_zero_below_one(hi[a], p);
        } else if (c == 0 && l == 1 && h == 1) {
            memcpy(best, lo, sizeof lo);
            return 1;
        } else if (c == 1 && l == 0 && h == 0) {
            return found;
        } else if (c == 1 && l == 0 && h == 1) {
            lo[a] = loft_rx_one_below_zero(lo[a], p);
        }
    }
    return found;
}

/* radix_db::box_range: the records inside the closed box, in Morton order, up to `cap` — runs
 * of the tree from each in-box seek point, jumping out-of-box gaps by BIGMIN.  malloc'd. */
static uint32_t *loft_rx_box(loft_ref c, const loft_key *keys, int nk, const int64_t *from, const int64_t *till, size_t cap,
                             uint32_t *count) {
    uint32_t t = loft_rx_tree(c), d, n = (uint32_t) nk;
    uint32_t *out = (uint32_t *) malloc(((size_t) loft_rx_count(c) + 1) * 4);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    *count = 0;
    if (t == 0 || cap == 0 || nk == 0 || nk > LOFT_RX_MAX_AXES) return out;
    loft_store_t *s = loft_store_of(c);
    uint64_t qlo[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, qhi[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, hi_code[LOFT_RX_MAX_AXES];
    for (uint32_t a = 0; a < n; a++) {
        int64_t lo = from[a] <= till[a] ? from[a] : till[a], hi = from[a] <= till[a] ? till[a] : from[a];
        qlo[a] = loft_rx_coord(lo);
        qhi[a] = loft_rx_coord(hi);
    }
    loft_rx_morton(n, qhi, hi_code);
    uint64_t at[LOFT_RX_MAX_AXES];
    memcpy(at, qlo, sizeof at);
    uint32_t steps = loft_rx_len(s, t);
    steps = steps > UINT32_MAX - 4 ? UINT32_MAX : steps + 4;
    for (;;) {
        loft_rx_key probe;
        memset(&probe, 0, sizeof probe);
        probe.s = s;
        probe.keys = keys;
        probe.nk = nk;
        memcpy(probe.codes, at, sizeof at);
        probe.bits = LOFT_RX_AXIS_BITS * n;
        loft_rx_iter it = loft_rx_seek(s, t, &probe, &d);
        uint32_t rec = it.rec;
        int jumped = 0;
        while (rec != 0) {
            if (*count >= cap || steps == 0) return out;
            steps--;
            uint64_t axes[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, code[LOFT_RX_MAX_AXES];
            loft_rx_axes(s, rec, keys, nk, axes);
            int inside = 1;
            for (uint32_t a = 0; a < n; a++) inside &= axes[a] >= qlo[a] && axes[a] <= qhi[a];
            if (inside) {
                out[(*count)++] = rec;
                rec = loft_rx_step(s, t, &it, 1);
                continue;
            }
            loft_rx_morton(n, axes, code);
            if (loft_rx_code_gt(code, hi_code, n)) return out;
            if (!loft_rx_bigmin(axes, n, qlo, qhi, at)) return out;
            jumped = 1;
            break;
        }
        if (!jumped) return out;
    }
}

/* trie_db::prefix: the records whose key starts with `pre`, in key order, up to `cap`.
 * malloc'd. */
static uint32_t *loft_rx_prefix(loft_ref c, const loft_key *keys, loft_str pre, size_t cap, uint32_t *count) {
    uint32_t t = loft_rx_tree(c), d;
    uint32_t *out = (uint32_t *) malloc(((size_t) loft_rx_count(c) + 1) * 4);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    *count = 0;
    if (t == 0) return out;
    loft_store_t *s = loft_store_of(c);
    loft_kv kv = loft_kv_text(pre);
    loft_rx_key probe = loft_rx_probe(s, keys, 1, 1, &kv, 1);
    loft_rx_iter it = loft_rx_seek(s, t, &probe, &d);
    for (uint32_t rec = it.rec; rec != 0 && *count < cap; rec = loft_rx_step(s, t, &it, 1)) {
        loft_str k = loft_bt_key_text(s, rec, &keys[0]);
        if (k.len < pre.len || (pre.len && memcmp(k.p, pre.p, pre.len) != 0)) break;
        out[(*count)++] = rec;
    }
    return out;
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
