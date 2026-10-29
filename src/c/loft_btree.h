/* ── The index: a B+tree of key parts and record refs (src/btree.rs) ─────────────────────────
 * loft's own layout, ported line for line.  The collection slot names one ARENA record of the
 * collection's store; node `n` starts at byte 8 + n·NODE and node 0 is the header.  A node
 * holds up to FANOUT slots, each an 8-byte KEY PART (the first key field as an i64 in the
 * field's order) beside a record number; an internal slot i ≥ 1 also names its child, whose
 * minimum the record is.  Leaves are chained both ways.  A position names a leaf slot,
 * `leaf·FANOUT + slot`, 0 for none.  An entry is a record of its own, its payload at byte 8
 * (`@FR-S-Index`). */

#define LOFT_BT_FANOUT 16u
#define LOFT_BT_MIN (LOFT_BT_FANOUT / 4)
#define LOFT_BT_MAX_HEIGHT 24
#define LOFT_BT_N_COUNT 0u
#define LOFT_BT_N_LEAF 4u
#define LOFT_BT_N_SKIP 6u
#define LOFT_BT_N_NEXT 8u
#define LOFT_BT_N_PREV 12u
#define LOFT_BT_N_PART 16u
#define LOFT_BT_N_REC (LOFT_BT_N_PART + 8 * LOFT_BT_FANOUT)
#define LOFT_BT_N_CHILD (LOFT_BT_N_REC + 4 * LOFT_BT_FANOUT)
#define LOFT_BT_N_PREFIX (LOFT_BT_N_CHILD + 4 * LOFT_BT_FANOUT)
#define LOFT_BT_MAX_SKIP 24u
#define LOFT_BT_NODE ((LOFT_BT_N_PREFIX + LOFT_BT_MAX_SKIP + 7) / 8 * 8)
#define LOFT_BT_H_ROOT 0u
#define LOFT_BT_H_HEIGHT 4u
#define LOFT_BT_H_LEN 8u
#define LOFT_BT_H_USED 12u
#define LOFT_BT_H_FREE 16u
#define LOFT_BT_H_CAP 20u
#define LOFT_BT_H_FIRST 24u
#define LOFT_BT_H_LAST 28u
#define LOFT_BT_FIRST_CAP 2u

static uint32_t loft_bt_node_at(uint32_t n) { return 8 + n * LOFT_BT_NODE; }
static uint32_t loft_bt_arena_words(uint32_t cap) { return (8 + cap * LOFT_BT_NODE) / 8; }

static uint32_t loft_bt_hdr(loft_store_t *s, uint32_t a, uint32_t f) { return loft_u32(s, a, 8 + f); }
static void loft_bt_set_hdr(loft_store_t *s, uint32_t a, uint32_t f, uint32_t v) { loft_set_u32(s, a, 8 + f, v); }
static uint32_t loft_bt_get(loft_store_t *s, uint32_t a, uint32_t n, uint32_t f) { return loft_u32(s, a, loft_bt_node_at(n) + f); }
static void loft_bt_set(loft_store_t *s, uint32_t a, uint32_t n, uint32_t f, uint32_t v) { loft_set_u32(s, a, loft_bt_node_at(n) + f, v); }

static uint16_t loft_bt_get16(loft_store_t *s, uint32_t a, uint32_t n, uint32_t f) {
    uint16_t v;
    memcpy(&v, loft_at(s, a, loft_bt_node_at(n) + f), 2);
    return v;
}

static void loft_bt_set16(loft_store_t *s, uint32_t a, uint32_t n, uint32_t f, uint16_t v) {
    memcpy(loft_at(s, a, loft_bt_node_at(n) + f), &v, 2);
}

static uint32_t loft_bt_count(loft_store_t *s, uint32_t a, uint32_t n) { return loft_bt_get(s, a, n, LOFT_BT_N_COUNT); }
static int loft_bt_is_leaf(loft_store_t *s, uint32_t a, uint32_t n) { return loft_bt_get16(s, a, n, LOFT_BT_N_LEAF) == 1; }

static int64_t loft_bt_part(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i) {
    int64_t v;
    memcpy(&v, loft_at(s, a, loft_bt_node_at(n) + LOFT_BT_N_PART + 8 * i), 8);
    return v;
}

static void loft_bt_set_part(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i, int64_t v) {
    memcpy(loft_at(s, a, loft_bt_node_at(n) + LOFT_BT_N_PART + 8 * i), &v, 8);
}

static uint32_t loft_bt_rec(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i) { return loft_u32(s, a, loft_bt_node_at(n) + LOFT_BT_N_REC + 4 * i); }
static uint32_t loft_bt_child(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i) { return loft_u32(s, a, loft_bt_node_at(n) + LOFT_BT_N_CHILD + 4 * i); }
static void loft_bt_set_child(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i, uint32_t c) { loft_set_u32(s, a, loft_bt_node_at(n) + LOFT_BT_N_CHILD + 4 * i, c); }

static void loft_bt_set_slot(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i, int64_t part, uint32_t rec) {
    loft_bt_set_part(s, a, n, i, part);
    loft_set_u32(s, a, loft_bt_node_at(n) + LOFT_BT_N_REC + 4 * i, rec);
}

typedef struct { int64_t part; uint32_t rec, child; } loft_bt_slot;

static loft_bt_slot loft_bt_slot_of(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i) {
    loft_bt_slot v;
    v.part = loft_bt_part(s, a, n, i);
    v.rec = loft_bt_rec(s, a, n, i);
    /* A leaf's child words are never written (a node grown into a resized arena holds what the
     * allocator left there), so only an internal node's is read. */
    v.child = loft_bt_is_leaf(s, a, n) ? 0 : loft_bt_child(s, a, n, i);
    return v;
}

static void loft_bt_put(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i, loft_bt_slot v) {
    loft_bt_set_slot(s, a, n, i, v.part, v.rec);
    loft_bt_set_child(s, a, n, i, v.child);
}

/* Slots [from, from + len) move to `to`, their children with them for an internal node. */
static void loft_bt_shift(loft_store_t *s, uint32_t a, uint32_t n, uint32_t from, uint32_t to, uint32_t len, int children) {
    if (len == 0 || from == to) return;
    uint8_t *b = loft_at(s, a, loft_bt_node_at(n));
    memmove(b + LOFT_BT_N_PART + 8 * to, b + LOFT_BT_N_PART + 8 * from, (size_t) 8 * len);
    memmove(b + LOFT_BT_N_REC + 4 * to, b + LOFT_BT_N_REC + 4 * from, (size_t) 4 * len);
    if (children) memmove(b + LOFT_BT_N_CHILD + 4 * to, b + LOFT_BT_N_CHILD + 4 * from, (size_t) 4 * len);
}

static uint32_t loft_bt_step(loft_store_t *s, uint32_t a, uint32_t pos, int back) {
    uint32_t n = pos / LOFT_BT_FANOUT, i = pos % LOFT_BT_FANOUT;
    if (back) {
        if (i > 0) return pos - 1;
        uint32_t p = loft_bt_get(s, a, n, LOFT_BT_N_PREV);
        return p == 0 ? 0 : p * LOFT_BT_FANOUT + loft_bt_count(s, a, p) - 1;
    }
    if (i + 1 < loft_bt_count(s, a, n)) return pos + 1;
    return loft_bt_get(s, a, n, LOFT_BT_N_NEXT) * LOFT_BT_FANOUT;
}

static void loft_bt_init_node(loft_store_t *s, uint32_t a, uint32_t n, int leaf) {
    loft_bt_set(s, a, n, LOFT_BT_N_COUNT, 0);
    loft_bt_set16(s, a, n, LOFT_BT_N_LEAF, (uint16_t) (leaf ? 1 : 0));
    loft_bt_set(s, a, n, LOFT_BT_N_NEXT, 0);
    loft_bt_set(s, a, n, LOFT_BT_N_PREV, 0);
    loft_bt_set16(s, a, n, LOFT_BT_N_SKIP, 0);
}

/* A node's shared text prefix: its length, its bytes into `out` (LOFT_BT_MAX_SKIP of them). */
static uint32_t loft_bt_prefix(loft_store_t *s, uint32_t a, uint32_t n, uint8_t *out) {
    uint32_t skip = loft_bt_get16(s, a, n, LOFT_BT_N_SKIP);
    if (skip == 0) {
        memset(out, 0, LOFT_BT_MAX_SKIP);
        return 0;
    }
    memcpy(out, loft_at(s, a, loft_bt_node_at(n) + LOFT_BT_N_PREFIX), LOFT_BT_MAX_SKIP);
    return skip;
}

static void loft_bt_set_prefix(loft_store_t *s, uint32_t a, uint32_t n, uint32_t skip, const uint8_t *bytes) {
    uint8_t buf[LOFT_BT_MAX_SKIP];
    memset(buf, 0, sizeof buf);
    if (skip) memcpy(buf, bytes, skip);
    loft_bt_set16(s, a, n, LOFT_BT_N_SKIP, (uint16_t) skip);
    memcpy(loft_at(s, a, loft_bt_node_at(n) + LOFT_BT_N_PREFIX), buf, LOFT_BT_MAX_SKIP);
}

/* ── key parts ── */

/* `float_part`: the null (a NaN) below every value (`@FR-E-NullArg`, loft#2237). */
static int64_t loft_bt_float_part(double v) {
    int64_t b;
    if (v != v) return INT64_MIN;
    if (v == 0.0) v = 0.0;
    memcpy(&b, &v, 8);
    return b ^ (int64_t) ((uint64_t) (b >> 63) >> 1);
}

static int64_t loft_bt_single_part(float v) {
    int32_t b;
    if (v != v) return INT32_MIN;
    if (v == 0.0f) v = 0.0f;
    memcpy(&b, &v, 4);
    return (int64_t) (b ^ (int32_t) ((uint32_t) (b >> 31) >> 1));
}

/* The eight bytes after `skip`, big-endian, zero-padded, the sign bit flipped. */
static int64_t loft_bt_text_part_at(const uint8_t *b, size_t len, size_t skip) {
    uint64_t w = 0;
    for (size_t i = 0; i < 8; i++) w = w << 8 | (skip + i < len ? b[skip + i] : 0);
    return (int64_t) (w ^ (UINT64_C(1) << 63));
}

/* btree::key_text: the first key field's text of the record `rec`. */
static loft_str loft_bt_key_text(loft_store_t *s, uint32_t rec, const loft_key *k) {
    return loft_store_str(s, loft_u32(s, rec, 8 + k->pos));
}

static int loft_bt_is_text(const loft_key *keys) { return keys[0].type == 6 || keys[0].type == -6; }

/* A key value as a part, or 0 in `*ok` when its kind is not the field's. */
static int64_t loft_bt_kv_part(loft_kv v, const loft_key *k, int *ok) {
    int t = k->type < 0 ? -k->type : k->type;
    int64_t p;
    *ok = 1;
    if (v.kind == LOFT_KV_SINGLE && t == 3) p = loft_bt_single_part(v.g);
    else if (v.kind == LOFT_KV_FLOAT && t == 4) p = loft_bt_float_part(v.f);
    else if (v.kind == LOFT_KV_TEXT && t == 6) p = loft_bt_text_part_at((const uint8_t *) v.s.p, v.s.len, 0);
    else if (v.kind == LOFT_KV_INT && t != 3 && t != 4 && t != 6) p = v.i;
    else {
        *ok = 0;
        return 0;
    }
    return k->type < 0 ? ~p : p;
}

/* btree::record_part: the first key field of the record `rec` as a part. */
static int64_t loft_bt_record_part(loft_store_t *s, uint32_t rec, const loft_key *k) {
    int ok;
    return loft_bt_kv_part(loft_key_get(s, rec, 8, k), k, &ok);
}

static int64_t loft_bt_part_in(loft_store_t *s, uint32_t rec, const loft_key *keys, uint32_t skip) {
    if (skip == 0) return loft_bt_record_part(s, rec, &keys[0]);
    loft_str t = loft_bt_key_text(s, rec, &keys[0]);
    int64_t v = loft_bt_text_part_at((const uint8_t *) t.p, t.len, skip);
    return keys[0].type < 0 ? ~v : v;
}

/* btree::Probe: what a descent compares — a record's key, or `n` key values. */
typedef struct {
    int has_part, has_text, exact;
    int64_t part;
    uint8_t head[LOFT_BT_MAX_SKIP + 8];
    size_t len;
    uint32_t rec;            /* by record, or 0 */
    const loft_kv *kv;       /* by key values */
    int n;
    uint16_t store_nr;
} loft_bt_probe;

static void loft_bt_head_of(loft_bt_probe *p, loft_str t) {
    size_t n = t.len < sizeof p->head ? t.len : sizeof p->head;
    memset(p->head, 0, sizeof p->head);
    if (n) memcpy(p->head, t.p, n);
    p->len = t.len;
}

static loft_bt_probe loft_bt_of_record(loft_store_t *s, uint16_t store_nr, uint32_t rec, const loft_key *keys, int nk) {
    loft_bt_probe p;
    memset(&p, 0, sizeof p);
    p.has_text = loft_bt_is_text(keys);
    loft_bt_head_of(&p, p.has_text ? loft_bt_key_text(s, rec, &keys[0]) : loft_lit("", 0));
    p.has_part = 1;
    p.part = loft_bt_record_part(s, rec, &keys[0]);
    p.exact = nk == 1 && !loft_bt_is_text(keys);
    p.rec = rec;
    p.store_nr = store_nr;
    return p;
}

static loft_bt_probe loft_bt_of_key(uint16_t store_nr, const loft_kv *kv, int n, const loft_key *keys) {
    loft_bt_probe p;
    memset(&p, 0, sizeof p);
    if (n > 0) p.part = loft_bt_kv_part(kv[0], &keys[0], &p.has_part);
    p.has_text = n > 0 && kv[0].kind == LOFT_KV_TEXT && loft_bt_is_text(keys);
    loft_bt_head_of(&p, p.has_text ? kv[0].s : loft_lit("", 0));
    p.exact = p.has_part && n == 1 && !loft_bt_is_text(keys);
    p.kv = kv;
    p.n = n;
    p.store_nr = store_nr;
    return p;
}

/* Probe::full: the probe against the record `rec`, on every part it carries. */
static int loft_bt_full(const loft_bt_probe *p, uint32_t rec, const loft_key *keys, int nk) {
    loft_ref other = { p->store_nr, rec, 8 };
    if (p->rec != 0) {
        loft_ref mine = { p->store_nr, p->rec, 8 };
        return loft_key_cmp_refs(mine, other, keys, nk);
    }
    return loft_key_cmp_val(p->kv, p->n, loft_stores + p->store_nr, rec, 8, keys);
}

/* The leading `skip` bytes of word `k` of `bytes`, the rest zero. */
static uint64_t loft_bt_word_below(const uint8_t *bytes, uint32_t k, uint32_t skip) {
    uint32_t keep = skip > 8 * k ? skip - 8 * k : 0;
    uint64_t w = 0;
    if (keep > 8) keep = 8;
    for (uint32_t i = 0; i < 8; i++) w = w << 8 | bytes[8 * k + i];
    if (keep == 8) return w;
    if (keep == 0) return 0;
    return w & ~(UINT64_MAX >> (8 * keep));
}

#define LOFT_BT_WHOLE 0
#define LOFT_BT_PARTV 1
#define LOFT_BT_BELOW 2
#define LOFT_BT_ABOVE 3

/* btree::text_order: where a text probe falls against a node's shared prefix. */
static int loft_bt_text_order(loft_store_t *s, uint32_t a, uint32_t n, const loft_bt_probe *p, const loft_key *keys,
                              int64_t *part) {
    uint8_t prefix[LOFT_BT_MAX_SKIP];
    uint32_t skip = loft_bt_prefix(s, a, n, prefix);
    if (skip == 0 || !p->has_text) return LOFT_BT_WHOLE;
    int desc = keys[0].type < 0, order = 0;
    if (p->len >= skip) {
        for (uint32_t k = 0; k < (skip + 7) / 8 && order == 0; k++) {
            uint64_t x = loft_bt_word_below(p->head, k, skip), y = loft_bt_word_below(prefix, k, skip);
            order = x < y ? -1 : x > y;
        }
    } else {
        int c = p->len ? memcmp(p->head, prefix, p->len) : 0;
        order = c < 0 ? -1 : c > 0 ? 1 : -1;
    }
    if (order == 0) {
        uint64_t v = 0;
        for (uint32_t i = 0; i < 8; i++) v = v << 8 | p->head[skip + i];
        int64_t w = (int64_t) (v ^ (UINT64_C(1) << 63));
        *part = desc ? ~w : w;
        return LOFT_BT_PARTV;
    }
    return (order < 0) != desc ? LOFT_BT_BELOW : LOFT_BT_ABOVE;
}

/* btree::stop_at: the first slot of node `n` from `from` the probe stops at, and whether it
 * stopped on an equal key. */
static uint32_t loft_bt_stop_at(loft_store_t *s, uint32_t a, const loft_key *keys, int nk, uint32_t n, uint32_t from,
                                const loft_bt_probe *p, int ties_right, int *equal) {
    uint32_t c = loft_bt_count(s, a, n), lo = from, hi = c;
    int has_part = p->has_part;
    int64_t part = p->part;
    *equal = 0;
    if (p->has_text && has_part && loft_bt_get16(s, a, n, LOFT_BT_N_SKIP) != 0) {
        switch (loft_bt_text_order(s, a, n, p, keys, &part)) {
        case LOFT_BT_BELOW:
            return from;
        case LOFT_BT_ABOVE:
            return c;
        default:
            break;
        }
    }
    if (has_part && p->exact) {
        while (lo < c) {
            int64_t q = loft_bt_part(s, a, n, lo);
            if (part < q || (part == q && !ties_right)) {
                *equal = part == q;
                return lo;
            }
            lo++;
        }
        return c;
    }
    if (has_part) {
        while (lo < c && loft_bt_part(s, a, n, lo) < part) lo++;
        hi = lo;
        while (hi < c && loft_bt_part(s, a, n, hi) == part) hi++;
    }
    while (lo < hi) {
        uint32_t mid = lo + (hi - lo) / 2;
        int o = loft_bt_full(p, loft_bt_rec(s, a, n, mid), keys, nk);
        if (o < 0 || (o == 0 && !ties_right)) {
            hi = mid;
            *equal = o == 0;
        } else {
            lo = mid + 1;
        }
    }
    return lo;
}

typedef struct { uint32_t steps[LOFT_BT_MAX_HEIGHT][2]; uint32_t leaf; } loft_bt_path;

static loft_bt_path loft_bt_descend(loft_store_t *s, uint32_t a, const loft_key *keys, int nk, const loft_bt_probe *p,
                                    int ties_right) {
    loft_bt_path path;
    int eq;
    uint32_t height = loft_bt_hdr(s, a, LOFT_BT_H_HEIGHT);
    if (height >= LOFT_BT_MAX_HEIGHT) {
        fprintf(stderr, "index tree: corrupt height %u\n", height);
        abort();
    }
    memset(&path, 0, sizeof path);
    uint32_t n = loft_bt_hdr(s, a, LOFT_BT_H_ROOT);
    for (uint32_t l = 0; l < height; l++) {
        uint32_t i = loft_bt_stop_at(s, a, keys, nk, n, 1, p, ties_right, &eq) - 1;
        path.steps[l][0] = n;
        path.steps[l][1] = i;
        n = loft_bt_child(s, a, n, i);
    }
    path.leaf = n;
    return path;
}

static uint32_t loft_bt_leaf_of(loft_store_t *s, uint32_t a, const loft_key *keys, int nk, const loft_bt_probe *p,
                                int ties_right) {
    int eq;
    uint32_t n = loft_bt_hdr(s, a, LOFT_BT_H_ROOT), height = loft_bt_hdr(s, a, LOFT_BT_H_HEIGHT);
    if (height > LOFT_BT_MAX_HEIGHT) height = LOFT_BT_MAX_HEIGHT;
    for (uint32_t l = 0; l < height; l++) n = loft_bt_child(s, a, n, loft_bt_stop_at(s, a, keys, nk, n, 1, p, ties_right, &eq) - 1);
    return n;
}

/* The collection's arena record, 0 for none. */
static uint32_t loft_bt_arena_of(loft_ref data) {
    if (data.store_nr == 0xFFFF || data.rec == 0) return 0;
    return loft_collection_rec(loft_store_of(data), data.rec, data.pos);
}

/* btree::table_bytes: the bytes of the index's node table (`OpSizeIndex`). */
static int64_t loft_bt_table_bytes(loft_ref data) {
    uint32_t a = loft_bt_arena_of(data);
    return a == 0 ? 0 : (int64_t) (loft_bt_hdr(loft_store_of(data), a, LOFT_BT_H_CAP) - 1) * LOFT_BT_NODE;
}

static uint32_t loft_bt_create(loft_store_t *s, loft_ref data) {
    uint32_t a = loft_st_claim(s, loft_bt_arena_words(LOFT_BT_FIRST_CAP));
    loft_bt_set_hdr(s, a, LOFT_BT_H_HEIGHT, 0);
    loft_bt_set_hdr(s, a, LOFT_BT_H_LEN, 0);
    loft_bt_set_hdr(s, a, LOFT_BT_H_FREE, 0);
    loft_bt_set_hdr(s, a, LOFT_BT_H_CAP, LOFT_BT_FIRST_CAP);
    loft_bt_set_hdr(s, a, LOFT_BT_H_USED, 2);
    loft_bt_init_node(s, a, 1, 1);
    loft_bt_set_hdr(s, a, LOFT_BT_H_ROOT, 1);
    loft_bt_set_hdr(s, a, LOFT_BT_H_FIRST, 1);
    loft_bt_set_hdr(s, a, LOFT_BT_H_LAST, 1);
    loft_set_u32(s, data.rec, data.pos, a);
    return a;
}

/* A node: a released one first, else the next unused, the arena doubled when it is full. */
static uint32_t loft_bt_alloc(loft_store_t *s, loft_ref data, uint32_t *a, int leaf) {
    uint32_t n, free_n = loft_bt_hdr(s, *a, LOFT_BT_H_FREE);
    if (free_n != 0) {
        loft_bt_set_hdr(s, *a, LOFT_BT_H_FREE, loft_bt_get(s, *a, free_n, LOFT_BT_N_NEXT));
        n = free_n;
    } else {
        uint32_t used = loft_bt_hdr(s, *a, LOFT_BT_H_USED), cap = loft_bt_hdr(s, *a, LOFT_BT_H_CAP);
        if (used == cap) {
            uint32_t grown = cap * 2;
            *a = loft_st_resize(s, *a, loft_bt_arena_words(grown));
            loft_set_u32(s, data.rec, data.pos, *a);
            loft_bt_set_hdr(s, *a, LOFT_BT_H_CAP, grown);
        }
        loft_bt_set_hdr(s, *a, LOFT_BT_H_USED, used + 1);
        n = used;
    }
    loft_bt_init_node(s, *a, n, leaf);
    return n;
}

static void loft_bt_release(loft_store_t *s, uint32_t a, uint32_t n) {
    loft_bt_set16(s, a, n, LOFT_BT_N_LEAF, 2);
    loft_bt_set(s, a, n, LOFT_BT_N_NEXT, loft_bt_hdr(s, a, LOFT_BT_H_FREE));
    loft_bt_set_hdr(s, a, LOFT_BT_H_FREE, n);
}

/* btree::fit_text: a text node's prefix, the longest its slots share, kept or dropped. */
static void loft_bt_fit(loft_store_t *s, uint32_t a, uint32_t n, const loft_key *keys, int force) {
    if (!loft_bt_is_text(keys)) return;
    uint32_t c = loft_bt_count(s, a, n);
    uint32_t start = loft_bt_is_leaf(s, a, n) ? 0 : (c < 1 ? c : 1), end = c;
    uint8_t prefix[LOFT_BT_MAX_SKIP], bytes[LOFT_BT_MAX_SKIP];
    uint32_t old = loft_bt_prefix(s, a, n, prefix), cp = 0;
    if (end - start >= 2) {
        loft_str x = loft_bt_key_text(s, loft_bt_rec(s, a, n, start), &keys[0]);
        loft_str y = loft_bt_key_text(s, loft_bt_rec(s, a, n, end - 1), &keys[0]);
        while (cp < x.len && cp < y.len && x.p[cp] == y.p[cp]) cp++;
        if (cp > LOFT_BT_MAX_SKIP) cp = LOFT_BT_MAX_SKIP;
        if (cp) memcpy(bytes, x.p, cp);
    }
    uint32_t skip = old;
    if (force || (old > 0 && cp < old)) {
        skip = cp;
    } else {
        uint32_t tied = 0;
        for (uint32_t i = start + 1; i < end; i++) tied += loft_bt_part(s, a, n, i) == loft_bt_part(s, a, n, i - 1);
        if (tied >= 2 && tied * 4 >= end - start) skip = cp;
    }
    if (!force && old == skip && (skip == 0 || memcmp(prefix, bytes, skip) == 0)) return;
    loft_bt_set_prefix(s, a, n, skip, bytes);
    for (uint32_t i = start; i < end; i++) loft_bt_set_part(s, a, n, i, loft_bt_part_in(s, loft_bt_rec(s, a, n, i), keys, skip));
}

static void loft_bt_place(loft_store_t *s, uint32_t a, uint32_t n, uint32_t i, uint32_t rec, int64_t part,
                          const loft_key *keys) {
    if (!loft_bt_is_text(keys)) {
        loft_bt_set_slot(s, a, n, i, part, rec);
        return;
    }
    uint8_t prefix[LOFT_BT_MAX_SKIP];
    uint32_t skip = loft_bt_prefix(s, a, n, prefix);
    loft_str t = loft_bt_key_text(s, rec, &keys[0]);
    if (t.len >= skip && (skip == 0 || memcmp(t.p, prefix, skip) == 0)) {
        int64_t v = loft_bt_text_part_at((const uint8_t *) t.p, t.len, skip);
        loft_bt_set_slot(s, a, n, i, keys[0].type < 0 ? ~v : v, rec);
    } else {
        loft_bt_set_slot(s, a, n, i, 0, rec);
        loft_bt_fit(s, a, n, keys, 1);
    }
}

/* btree::insert_leaf: the slot goes into leaf `n`; a full leaf splits, its right half's
 * separator answered in `*sep` (1) — 0 when nothing splits. */
static int loft_bt_insert_leaf(loft_store_t *s, loft_ref data, uint32_t *a, uint32_t n, uint32_t slot, int64_t part,
                               uint32_t rec, const loft_key *keys, loft_bt_slot *sep) {
    uint32_t c = loft_bt_count(s, *a, n);
    if (c < LOFT_BT_FANOUT) {
        loft_bt_shift(s, *a, n, slot, slot + 1, c - slot, 0);
        loft_bt_set(s, *a, n, LOFT_BT_N_COUNT, c + 1);
        loft_bt_place(s, *a, n, slot, rec, part, keys);
        return 0;
    }
    uint32_t r = loft_bt_alloc(s, data, a, 1);
    if (loft_bt_is_text(keys)) {
        uint8_t prefix[LOFT_BT_MAX_SKIP];
        uint32_t skip = loft_bt_prefix(s, *a, n, prefix);
        loft_bt_set_prefix(s, *a, r, skip, prefix);
    }
    uint32_t half = LOFT_BT_FANOUT / 2;
    for (uint32_t i = half; i < LOFT_BT_FANOUT; i++) loft_bt_put(s, *a, r, i - half, loft_bt_slot_of(s, *a, n, i));
    loft_bt_set(s, *a, n, LOFT_BT_N_COUNT, half);
    loft_bt_set(s, *a, r, LOFT_BT_N_COUNT, LOFT_BT_FANOUT - half);
    uint32_t next = loft_bt_get(s, *a, n, LOFT_BT_N_NEXT);
    loft_bt_set(s, *a, r, LOFT_BT_N_NEXT, next);
    loft_bt_set(s, *a, r, LOFT_BT_N_PREV, n);
    loft_bt_set(s, *a, n, LOFT_BT_N_NEXT, r);
    if (next == 0) loft_bt_set_hdr(s, *a, LOFT_BT_H_LAST, r);
    else loft_bt_set(s, *a, next, LOFT_BT_N_PREV, r);
    uint32_t into = slot <= half ? n : r, at = slot <= half ? slot : slot - half;
    c = loft_bt_count(s, *a, into);
    loft_bt_shift(s, *a, into, at, at + 1, c - at, 0);
    loft_bt_set(s, *a, into, LOFT_BT_N_COUNT, c + 1);
    loft_bt_place(s, *a, into, at, rec, part, keys);
    loft_bt_fit(s, *a, n, keys, 0);
    loft_bt_fit(s, *a, r, keys, 0);
    sep->part = loft_bt_is_text(keys) ? 0 : loft_bt_part(s, *a, r, 0);
    sep->rec = loft_bt_rec(s, *a, r, 0);
    sep->child = r;
    return 1;
}

static int loft_bt_insert_internal(loft_store_t *s, loft_ref data, uint32_t *a, uint32_t n, uint32_t at, loft_bt_slot sep,
                                   const loft_key *keys, loft_bt_slot *up) {
    uint32_t c = loft_bt_count(s, *a, n);
    if (c < LOFT_BT_FANOUT) {
        loft_bt_shift(s, *a, n, at, at + 1, c - at, 1);
        loft_bt_set(s, *a, n, LOFT_BT_N_COUNT, c + 1);
        loft_bt_set_child(s, *a, n, at, sep.child);
        loft_bt_place(s, *a, n, at, sep.rec, sep.part, keys);
        return 0;
    }
    loft_bt_slot all[LOFT_BT_FANOUT + 1];
    for (uint32_t i = 0; i < c; i++) all[i < at ? i : i + 1] = loft_bt_slot_of(s, *a, n, i);
    all[at] = sep;
    uint32_t r = loft_bt_alloc(s, data, a, 0);
    uint32_t keep = (LOFT_BT_FANOUT + 1) / 2;
    for (uint32_t i = 0; i <= LOFT_BT_FANOUT; i++) {
        if (i < keep) loft_bt_put(s, *a, n, i, all[i]);
        else loft_bt_put(s, *a, r, i - keep, all[i]);
    }
    loft_bt_set(s, *a, n, LOFT_BT_N_COUNT, keep);
    loft_bt_set(s, *a, r, LOFT_BT_N_COUNT, LOFT_BT_FANOUT + 1 - keep);
    loft_bt_fit(s, *a, n, keys, 1);
    loft_bt_fit(s, *a, r, keys, 1);
    up->part = loft_bt_is_text(keys) ? 0 : all[keep].part;
    up->rec = all[keep].rec;
    up->child = r;
    return 1;
}

/* btree::add: the record `rec` filed by its key; a record already under the key refuses it and
 * is answered (0 when it is `rec` itself or the key is new). */
static uint32_t loft_bt_add(loft_ref data, uint32_t rec, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(data);
    uint32_t a = loft_bt_arena_of(data);
    int equal;
    if (a == 0) a = loft_bt_create(s, data);
    loft_bt_probe p = loft_bt_of_record(s, data.store_nr, rec, keys, nk);
    loft_bt_path path = loft_bt_descend(s, a, keys, nk, &p, 1);
    uint32_t slot = loft_bt_stop_at(s, a, keys, nk, path.leaf, 0, &p, 0, &equal);
    if (equal) {
        uint32_t held = loft_bt_rec(s, a, path.leaf, slot);
        return held == rec ? 0 : held;
    }
    loft_bt_slot sep;
    int up = loft_bt_insert_leaf(s, data, &a, path.leaf, slot, p.has_part ? p.part : 0, rec, keys, &sep);
    uint32_t height = loft_bt_hdr(s, a, LOFT_BT_H_HEIGHT), level = height;
    while (up) {
        if (level == 0) {
            uint32_t root = loft_bt_alloc(s, data, &a, 0);
            loft_bt_slot first = { 0, 0, loft_bt_hdr(s, a, LOFT_BT_H_ROOT) };
            loft_bt_put(s, a, root, 0, first);
            loft_bt_set(s, a, root, LOFT_BT_N_COUNT, 2);
            loft_bt_set_child(s, a, root, 1, sep.child);
            loft_bt_place(s, a, root, 1, sep.rec, sep.part, keys);
            loft_bt_set_hdr(s, a, LOFT_BT_H_ROOT, root);
            loft_bt_set_hdr(s, a, LOFT_BT_H_HEIGHT, height + 1);
            break;
        }
        level--;
        loft_bt_slot next;
        up = loft_bt_insert_internal(s, data, &a, path.steps[level][0], path.steps[level][1] + 1, sep, keys, &next);
        sep = next;
    }
    loft_bt_set_hdr(s, a, LOFT_BT_H_LEN, loft_bt_hdr(s, a, LOFT_BT_H_LEN) + 1);
    return 0;
}

/* btree::rebalance: an underfull node borrows from or merges with a sibling, up the path. */
static void loft_bt_rebalance(loft_store_t *s, uint32_t a, const loft_bt_path *path, uint32_t height, const loft_key *keys) {
    uint32_t n = path->leaf, level = height;
    for (;;) {
        if (level == 0) {
            if (!loft_bt_is_leaf(s, a, n) && loft_bt_count(s, a, n) == 1) {
                loft_bt_set_hdr(s, a, LOFT_BT_H_ROOT, loft_bt_child(s, a, n, 0));
                loft_bt_set_hdr(s, a, LOFT_BT_H_HEIGHT, loft_bt_hdr(s, a, LOFT_BT_H_HEIGHT) - 1);
                loft_bt_release(s, a, n);
            }
            return;
        }
        if (loft_bt_count(s, a, n) >= LOFT_BT_MIN) return;
        uint32_t parent = path->steps[level - 1][0], i = path->steps[level - 1][1];
        uint32_t ri = i > 0 ? i : 1;
        uint32_t left = loft_bt_child(s, a, parent, ri - 1), right = loft_bt_child(s, a, parent, ri);
        int leaf = loft_bt_is_leaf(s, a, n);
        uint32_t cl = loft_bt_count(s, a, left), cr = loft_bt_count(s, a, right);
        if (cl + cr <= LOFT_BT_FANOUT) {
            for (uint32_t k = 0; k < cr; k++) {
                loft_bt_slot v = loft_bt_slot_of(s, a, right, k);
                if (k == 0 && !leaf) {
                    v.part = loft_bt_part(s, a, parent, ri);
                    v.rec = loft_bt_rec(s, a, parent, ri);
                }
                loft_bt_put(s, a, left, cl + k, v);
            }
            loft_bt_set(s, a, left, LOFT_BT_N_COUNT, cl + cr);
            if (leaf) {
                uint32_t next = loft_bt_get(s, a, right, LOFT_BT_N_NEXT);
                loft_bt_set(s, a, left, LOFT_BT_N_NEXT, next);
                if (next == 0) loft_bt_set_hdr(s, a, LOFT_BT_H_LAST, left);
                else loft_bt_set(s, a, next, LOFT_BT_N_PREV, left);
            }
            loft_bt_release(s, a, right);
            uint32_t cp = loft_bt_count(s, a, parent);
            loft_bt_shift(s, a, parent, ri + 1, ri, cp - ri - 1, 1);
            loft_bt_set(s, a, parent, LOFT_BT_N_COUNT, cp - 1);
            loft_bt_fit(s, a, left, keys, 1);
        } else {
            loft_bt_slot all[2 * LOFT_BT_FANOUT];
            for (uint32_t k = 0; k < cl; k++) all[k] = loft_bt_slot_of(s, a, left, k);
            for (uint32_t k = 0; k < cr; k++) {
                loft_bt_slot v = loft_bt_slot_of(s, a, right, k);
                if (k == 0 && !leaf) {
                    v.part = loft_bt_part(s, a, parent, ri);
                    v.rec = loft_bt_rec(s, a, parent, ri);
                }
                all[cl + k] = v;
            }
            uint32_t total = cl + cr, keep = total / 2;
            for (uint32_t k = 0; k < total; k++) {
                if (k < keep) loft_bt_put(s, a, left, k, all[k]);
                else loft_bt_put(s, a, right, k - keep, all[k]);
            }
            loft_bt_set(s, a, left, LOFT_BT_N_COUNT, keep);
            loft_bt_set(s, a, right, LOFT_BT_N_COUNT, total - keep);
            loft_bt_fit(s, a, left, keys, 1);
            loft_bt_fit(s, a, right, keys, 1);
            loft_bt_place(s, a, parent, ri, all[keep].rec, loft_bt_is_text(keys) ? 0 : all[keep].part, keys);
        }
        n = parent;
        level--;
    }
}

/* btree::remove: the record `rec` leaves the tree; 0 when the tree does not hold it. */
static int loft_bt_remove(loft_ref data, uint32_t rec, const loft_key *keys, int nk) {
    uint32_t a = loft_bt_arena_of(data);
    int equal;
    if (a == 0) return 0;
    loft_store_t *s = loft_store_of(data);
    loft_bt_probe p = loft_bt_of_record(s, data.store_nr, rec, keys, nk);
    loft_bt_path path = loft_bt_descend(s, a, keys, nk, &p, 1);
    uint32_t slot = loft_bt_stop_at(s, a, keys, nk, path.leaf, 0, &p, 0, &equal);
    if (!equal || loft_bt_rec(s, a, path.leaf, slot) != rec) return 0;
    uint32_t height = loft_bt_hdr(s, a, LOFT_BT_H_HEIGHT);
    int named = -1;
    for (uint32_t l = 0; l < height && named < 0; l++) {
        if (path.steps[l][1] > 0 && loft_bt_rec(s, a, path.steps[l][0], path.steps[l][1]) == rec) named = (int) l;
    }
    uint32_t n = path.leaf, c = loft_bt_count(s, a, n);
    int has_successor = 1;
    int64_t spart = 0;
    uint32_t srec = 0;
    if (slot + 1 < c) {
        spart = loft_bt_part(s, a, n, slot + 1);
        srec = loft_bt_rec(s, a, n, slot + 1);
    } else {
        uint32_t nx = loft_bt_get(s, a, n, LOFT_BT_N_NEXT);
        if (nx == 0) {
            has_successor = 0;
        } else {
            spart = loft_bt_part(s, a, nx, 0);
            srec = loft_bt_rec(s, a, nx, 0);
        }
    }
    loft_bt_shift(s, a, n, slot + 1, slot, c - slot - 1, 0);
    loft_bt_set(s, a, n, LOFT_BT_N_COUNT, c - 1);
    if (named >= 0 && has_successor) loft_bt_place(s, a, path.steps[named][0], path.steps[named][1], srec, spart, keys);
    loft_bt_set_hdr(s, a, LOFT_BT_H_LEN, loft_bt_hdr(s, a, LOFT_BT_H_LEN) - 1);
    loft_bt_rebalance(s, a, &path, height, keys);
    return 1;
}

/* btree::find_exact: the record whose whole key is `kv`, 0 for none. */
static uint32_t loft_bt_find_exact(loft_ref data, const loft_key *keys, int nk, const loft_kv *kv, int n) {
    uint32_t a = loft_bt_arena_of(data);
    int equal;
    if (a == 0) return 0;
    loft_store_t *s = loft_store_of(data);
    loft_bt_probe p = loft_bt_of_key(data.store_nr, kv, n, keys);
    uint32_t leaf = loft_bt_leaf_of(s, a, keys, nk, &p, 1);
    uint32_t slot = loft_bt_stop_at(s, a, keys, nk, leaf, 0, &p, 0, &equal);
    return equal ? loft_bt_rec(s, a, leaf, slot) : 0;
}

static uint32_t loft_bt_first(loft_ref data) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0) return 0;
    loft_store_t *s = loft_store_of(data);
    uint32_t n = loft_bt_hdr(s, a, LOFT_BT_H_FIRST);
    return loft_bt_count(s, a, n) == 0 ? 0 : n * LOFT_BT_FANOUT;
}

static uint32_t loft_bt_last(loft_ref data) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0) return 0;
    loft_store_t *s = loft_store_of(data);
    uint32_t n = loft_bt_hdr(s, a, LOFT_BT_H_LAST), c = loft_bt_count(s, a, n);
    return c == 0 ? 0 : n * LOFT_BT_FANOUT + c - 1;
}

static uint32_t loft_bt_next(loft_ref data, uint32_t pos) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0 || pos == 0) return 0;
    return loft_bt_step(loft_store_of(data), a, pos, 0);
}

static uint32_t loft_bt_previous(loft_ref data, uint32_t pos) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0 || pos == 0) return 0;
    return loft_bt_step(loft_store_of(data), a, pos, 1);
}

static uint32_t loft_bt_record(loft_ref data, uint32_t pos) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0 || pos == 0) return 0;
    return loft_bt_rec(loft_store_of(data), a, pos / LOFT_BT_FANOUT, pos % LOFT_BT_FANOUT);
}

/* btree::bound: the first position at (or, `above`, past) the key. */
static uint32_t loft_bt_bound(loft_ref data, uint32_t a, const loft_key *keys, int nk, const loft_kv *kv, int n, int above) {
    loft_store_t *s = loft_store_of(data);
    int equal;
    loft_bt_probe p = loft_bt_of_key(data.store_nr, kv, n, keys);
    loft_bt_path path = loft_bt_descend(s, a, keys, nk, &p, above);
    uint32_t slot = loft_bt_stop_at(s, a, keys, nk, path.leaf, 0, &p, above, &equal);
    if (slot < loft_bt_count(s, a, path.leaf)) return path.leaf * LOFT_BT_FANOUT + slot;
    return loft_bt_get(s, a, path.leaf, LOFT_BT_N_NEXT) * LOFT_BT_FANOUT;
}

/* btree::find: the first position at the key, or with `before` the last below it. */
static uint32_t loft_bt_find(loft_ref data, int before, const loft_key *keys, int nk, const loft_kv *kv, int n) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0) return 0;
    if (!before) return loft_bt_bound(data, a, keys, nk, kv, n, 1);
    uint32_t p = loft_bt_bound(data, a, keys, nk, kv, n, 0);
    return p == 0 ? loft_bt_last(data) : loft_bt_previous(data, p);
}

/* btree::position: the leaf position holding `rec`, 0 for none. */
static uint32_t loft_bt_position(loft_ref data, const loft_key *keys, int nk, uint32_t rec) {
    uint32_t a = loft_bt_arena_of(data);
    int equal;
    if (a == 0) return 0;
    loft_store_t *s = loft_store_of(data);
    loft_bt_probe p = loft_bt_of_record(s, data.store_nr, rec, keys, nk);
    loft_bt_path path = loft_bt_descend(s, a, keys, nk, &p, 1);
    uint32_t slot = loft_bt_stop_at(s, a, keys, nk, path.leaf, 0, &p, 0, &equal);
    return equal && loft_bt_rec(s, a, path.leaf, slot) == rec ? path.leaf * LOFT_BT_FANOUT + slot : 0;
}

/* btree::next_record / previous_record: the record after (before) `rec` in key order. */
static uint32_t loft_bt_step_record(loft_ref data, const loft_key *keys, int nk, uint32_t rec, int back) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0) return 0;
    uint32_t at = loft_bt_position(data, keys, nk, rec);
    if (at == 0) return 0;
    uint32_t to = loft_bt_step(loft_store_of(data), a, at, back);
    return to == 0 ? 0 : loft_bt_rec(loft_store_of(data), a, to / LOFT_BT_FANOUT, to % LOFT_BT_FANOUT);
}

/* btree::range: the first and last positions of a key range; 0 when it is empty. */
static int loft_bt_range(loft_ref data, const loft_key *keys, int nk, const loft_kv *from, int nfrom, const loft_kv *till,
                         int ntill, int ex, uint32_t *low, uint32_t *high) {
    uint32_t a = loft_bt_arena_of(data);
    if (a == 0) return 0;
    *low = nfrom == 0 ? loft_bt_first(data) : loft_bt_bound(data, a, keys, nk, from, nfrom, 0);
    if (ntill == 0) {
        *high = loft_bt_last(data);
    } else {
        uint32_t p = loft_bt_bound(data, a, keys, nk, till, ntill, !ex);
        *high = p == 0 ? loft_bt_last(data) : loft_bt_previous(data, p);
    }
    if (*low == 0 || *high == 0) return 0;
    if (ntill != 0) {
        int c = loft_key_cmp_val(till, ntill, loft_store_of(data), loft_bt_record(data, *low), 8, keys);
        if (c < 0 || (ex && c == 0)) return 0;
    }
    return 1;
}

static uint32_t loft_bt_len(loft_ref data) {
    uint32_t a = loft_bt_arena_of(data);
    return a == 0 ? 0 : loft_bt_hdr(loft_store_of(data), a, LOFT_BT_H_LEN);
}

/* btree::records: every record in key order, malloc'd; `*n` the count. */
static uint32_t *loft_bt_records(loft_ref data, uint32_t *n) {
    uint32_t *out = (uint32_t *) malloc(((size_t) loft_bt_len(data) + 1) * 4);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    *n = 0;
    for (uint32_t p = loft_bt_first(data); p != 0; p = loft_bt_next(data, p)) out[(*n)++] = loft_bt_record(data, p);
    return out;
}

/* ── the Stores routines over an index (src/database) ── */

/* Stores::find_index: the record whose key parts equal the `n` given — a whole key by its exact
 * descent, a partial one by the boundary walk (the lowest it matches). */
static loft_ref loft_ix_find(loft_ref data, const loft_key *keys, int nk, const loft_kv *kv, int n) {
    loft_ref r = { data.store_nr, 0, 8 };
    if (data.store_nr == 0xFFFF || data.rec == 0) return loft_ref_null();
    if (n == nk) {
        r.rec = loft_bt_find_exact(data, keys, nk, kv, n);
        return r.rec == 0 ? loft_ref_null() : r;
    }
    uint32_t below = loft_bt_record(data, loft_bt_find(data, 1, keys, nk, kv, n));
    r.rec = below == 0 ? loft_bt_record(data, loft_bt_first(data)) : loft_bt_step_record(data, keys, nk, below, 0);
    if (r.rec != 0 && loft_key_cmp_val(kv, n, loft_store_of(data), r.rec, 8, keys) == 0) return r;
    return loft_ref_null();
}

/* Stores::insert_record on an index (`@FR-Col-Insert`): the record filed by its key; the record
 * it displaced is answered — released already unless `secondary` — or the null reference. */
static loft_ref loft_ix_insert(const loft_tdesc *T, uint16_t elem, loft_ref data, loft_ref rec, const loft_key *keys,
                               int nk, int secondary) {
    if (data.store_nr == 0xFFFF || data.rec == 0 || rec.rec == 0) return loft_ref_null();
    if (LOFT_WRITE_REFUSED(loft_store_of(data))) return loft_ref_null();
    uint32_t dup = loft_bt_add(data, rec.rec, keys, nk);
    if (dup == 0) return loft_ref_null();
    loft_ref existing = { data.store_nr, dup, 8 };
    (void) loft_bt_remove(data, dup, keys, nk);
    if (!secondary) {
        loft_remove_claims(T, elem, existing);
        loft_st_delete(loft_store_of(data), dup);
    }
    (void) loft_bt_add(data, rec.rec, keys, nk);
    return existing;
}

/* Stores::remove_owned on an index: unlinked, then its claims and its record released; a `view`
 * member only unlinks. */
static void loft_ix_remove(const loft_tdesc *T, uint16_t elem, loft_ref data, loft_ref e, const loft_key *keys, int nk,
                           int view) {
    if (e.rec == 0 || data.store_nr == 0xFFFF || data.rec == 0) return;
    (void) loft_bt_remove(data, e.rec, keys, nk);
    if (view) return;
    loft_ref rec = { e.store_nr, e.rec, 8 };
    loft_remove_claims(T, elem, rec);
    loft_st_delete(loft_store_of(data), e.rec);
}

/* OpIterate over an index (State::iterate, btree::range_cursors): the walk's state — the record
 * last yielded (0: none yet) in the low half, the record it ends on in the high half
 * (0: the end; UINT32_MAX: done). */
static int64_t loft_ix_iterate(loft_ref data, int ex, int reverse, const loft_key *keys, int nk, const loft_kv *from,
                               int nfrom, const loft_kv *till, int ntill) {
    uint32_t low, high, start, finish;
    if (data.store_nr == 0xFFFF || data.rec == 0 || !loft_bt_range(data, keys, nk, from, nfrom, till, ntill, ex, &low, &high))
        return (int64_t) ((uint64_t) UINT32_MAX << 32);
    if (reverse) {
        start = ntill == 0 ? 0 : loft_bt_record(data, loft_bt_next(data, high));
        finish = nfrom == 0 ? 0 : loft_bt_record(data, low);
    } else {
        start = nfrom == 0 ? 0 : loft_bt_record(data, loft_bt_previous(data, low));
        finish = ntill == 0 ? 0 : loft_bt_record(data, high);
    }
    return (int64_t) ((uint64_t) finish << 32 | start);
}

/* OpStep over an index: the next record, or the null reference. */
static loft_ref loft_ix_step(int64_t *state, loft_ref data, const loft_key *keys, int nk, int reverse) {
    uint64_t st = (uint64_t) *state;
    uint32_t cur = (uint32_t) st, finish = (uint32_t) (st >> 32);
    if (data.store_nr == 0xFFFF || data.rec == 0 || finish == UINT32_MAX) return loft_ref_null();
    uint32_t n = cur == 0 ? loft_bt_record(data, reverse ? loft_bt_last(data) : loft_bt_first(data))
                          : loft_bt_step_record(data, keys, nk, cur, reverse);
    if (n == finish) finish = UINT32_MAX;
    *state = (int64_t) ((uint64_t) finish << 32 | n);
    if (n == 0) return loft_ref_null();
    loft_ref e = { data.store_nr, n, 8 };
    return e;
}

/* OpRemove in a walk over an index (Stores::remove_during_tree_iteration): the record the walk
 * yielded last goes, and the walk goes on with the one after it. */
static void loft_ix_walk_remove(const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref data, const loft_key *keys,
                                int nk, int reverse) {
    uint64_t st = (uint64_t) *state;
    uint32_t cur = (uint32_t) st, finish = (uint32_t) (st >> 32);
    if (cur == 0 || data.store_nr == 0xFFFF || data.rec == 0) return;
    uint32_t after = loft_bt_step_record(data, keys, nk, cur, reverse);
    int removed_the_last = cur == finish;
    loft_ref e = { data.store_nr, cur, 8 };
    loft_ix_remove(T, elem, data, e, keys, nk, 0);
    if (after == 0 || removed_the_last) {
        finish = UINT32_MAX;
    } else {
        cur = loft_bt_step_record(data, keys, nk, after, !reverse);
    }
    *state = (int64_t) ((uint64_t) finish << 32 | cur);
}

/* Stores::copy_claims_index_body: each record of the source, in key order, copied into a record
 * of the destination's and filed there. */
static void loft_ix_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    uint32_t cur = loft_u32(loft_store_of(from), from.rec, from.pos);
    /* An absent collection is copied absent (`copy_absent_collection`). */
    loft_set_u32(loft_store_of(to), to.rec, to.pos, cur == 0xFFFFFFFFu ? cur : 0);
    if (cur == 0 || cur == 0xFFFFFFFFu) return;
    uint32_t n, size = T[d->elem].size;
    uint32_t *recs = loft_bt_records(from, &n);
    for (uint32_t i = 0; i < n; i++) {
        loft_store_t *ts = loft_store_of(to);
        loft_ref a = { from.store_nr, recs[i], 8 };
        loft_ref b = { to.store_nr, loft_st_claim(ts, 1 + (size + 7) / 8), 8 };
        loft_set_u32(ts, b.rec, 4, to.rec);
        loft_copy_bytes(b, 0, a, 0, size);
        loft_copy_claims(T, d->elem, a, b);
        (void) loft_bt_add(to, b.rec, d->keys, d->nk);
    }
    free(recs);
}

/* Stores::remove_claims on an index field: every record it holds released, in key order, then
 * its arena, and the field emptied; a `borrowed` view releases its arena only. */
static void loft_ix_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed) {
    loft_store_t *s = loft_store_of(r);
    uint32_t cur = loft_u32(s, r.rec, r.pos);
    if (cur != 0 && cur != 0xFFFFFFFFu) {
        if (!borrowed) {
            uint32_t n;
            uint32_t *recs = loft_bt_records(r, &n);
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
