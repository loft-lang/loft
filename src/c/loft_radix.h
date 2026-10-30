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
/* The kind's own header word: a trie's null-key record, tagged (trie_db::SLOT_TAG); placement
 * token 1 kept the node capacity here, which the claim's size says. */
#define LOFT_RX_SLOT 16u
#define LOFT_RX_SLOT_TAG 0x80000000u
#define LOFT_RX_FREE 20u
#define LOFT_RX_HDR 24u
#define LOFT_RX_NODE_SIZE 16u
#define LOFT_RX_AXIS_BITS 64u
#define LOFT_RX_MAX_AXES 3

static uint32_t loft_rx_node_off(uint32_t node) { return LOFT_RX_HDR + LOFT_RX_NODE_SIZE * (node - 1); }
static uint32_t loft_rx_child_off(uint32_t node, int dir) { return loft_rx_node_off(node) + (dir ? 12 : 8); }
#define LOFT_RX_REC(c) ((c) > 0)
#define LOFT_RX_NODE(c) ((c) < 0)
#define LOFT_RX_NODE_ID(c) ((uint32_t) -(c))

/* How the tree reads a key: a record's through its collection's oracle — the Morton code of its
 * axes (`text` 0) or the bytes of its text key (`text` 1) — or a probe's own words. */
typedef struct {
    const loft_key *keys;
    int nk, text;
    uint32_t rec;              /* a record's key, or 0 for a probe */
    uint64_t codes[LOFT_RX_MAX_AXES];
    const uint8_t *bytes;      /* a text probe */
    size_t len;
    int esc;                   /* the bytes are read escaped (trie_db::tree_bytes) */
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

/* trie_db::tree_bytes: how many bytes `b` files as — data 0x00 and 0x01 take two (`01 01`,
 * `01 02`), so no key reads as another that ends in NULs. */
static size_t loft_rx_esc_len(const uint8_t *b, size_t len) {
    size_t n = len;
    for (size_t i = 0; i < len; i++) n += b[i] < 2;
    return n;
}

/* bytes_word over tree_bytes(b), zero-padded past the end. */
static uint64_t loft_rx_esc_word(const uint8_t *b, size_t len, uint32_t word) {
    uint64_t out = 0;
    size_t want = (size_t) word * 8, at = 0;
    int got = 0;
    for (size_t i = 0; i < len && got < 8; i++) {
        uint8_t e[2] = { b[i], 0 };
        int n = 1;
        if (b[i] < 2) {
            e[0] = 1;
            e[1] = (uint8_t) (b[i] + 1);
            n = 2;
        }
        for (int j = 0; j < n; j++, at++) {
            if (at >= want && got < 8) {
                out = out << 8 | e[j];
                got++;
            }
        }
    }
    while (got++ < 8) out <<= 8;
    return out;
}

/* The bits a text key files as, and whether it must be read escaped: a key holding no byte
 * below 2 reads as it is — nearly every one. */
static void loft_rx_text_bits(loft_rx_key *k, int esc) {
    size_t n = esc ? loft_rx_esc_len(k->bytes, k->len) : k->len;
    k->esc = n != k->len;
    k->bits = (uint32_t) n * 8;
}

static uint64_t loft_rx_word(const loft_rx_key *k, uint32_t w) {
    if (k->text) return k->esc ? loft_rx_esc_word(k->bytes, k->len, w) : loft_rx_bytes_word(k->bytes, k->len, w);
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

/* radix_tree::RadixIter: a record and its parent node, and a step budget against a corrupt
 * tree. */
typedef struct { uint32_t rec, node, budget; } loft_rx_iter;

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

/* A growing list of record numbers, malloc'd: what a walk answers. */
typedef struct { uint32_t *p; size_t n, cap; } loft_u32s;

static loft_u32s loft_u32s_new(size_t cap) {
    loft_u32s v;
    v.cap = cap < 1 ? 1 : cap;
    v.n = 0;
    v.p = (uint32_t *) malloc(v.cap * 4);
    if (!v.p) { fputs("loft: out of memory\n", stderr); abort(); }
    return v;
}

static void loft_u32s_push(loft_u32s *v, uint32_t x) {
    if (v->n == v->cap) {
        v->cap *= 2;
        v->p = (uint32_t *) realloc(v->p, v->cap * 4);
        if (!v->p) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    v->p[v->n++] = x;
}

/* A box's corners as axis codes, low and high per axis: a corner given the other way round
 * names the same box. */
static void loft_rx_corners(int nk, const int64_t *from, const int64_t *till, uint64_t *qlo, uint64_t *qhi) {
    for (int a = 0; a < nk; a++) {
        int64_t lo = from[a] <= till[a] ? from[a] : till[a], hi = from[a] <= till[a] ? till[a] : from[a];
        qlo[a] = loft_rx_coord(lo);
        qhi[a] = loft_rx_coord(hi);
    }
}

/* The walks over a store (loft_radix_walk.h, pasted next): a node's child read as it is. */
#define LOFT_RXS loft_store_t
#define LOFT_RXF(name) loft_rx_##name
#define LOFT_RX_ESC(s) 1
#define LOFT_RX_RD(s, r, o) loft_u32(s, r, o)
#define LOFT_RX_BEGIN(s) ((void) (s))
#define LOFT_RX_GUARD(s, n) 1
#define LOFT_RX_KEYI(s, r, k) loft_key_get(s, r, 8, k).i
#define LOFT_RX_KEYT(s, r, k) loft_bt_key_text(s, r, k)
