#define _POSIX_C_SOURCE 200809L
/* bench/23_lru `lru_mixed` written by hand in C: the price the generated C and its runtime can
 * reach (bench/portal/analysis/hosts-2026-11-09.md § The programs over 2.7x, written by hand).
 *   cc -std=c99 -O3 -o lru_hand lru_mixed.c [-DHAND_STORETAB] [-DHAND_HDR]; ./lru_hand --n 400
 * The two switches add back the runtime's representation costs one at a time.
 *
 * lru_mixed by hand, keeping loft's semantics and data layout, without the runtime's generic layers.
 *
 * Kept from loft:
 *  - nodes are RECORDS in a store: 8-byte words, a header word (size, owner), fields from byte 8;
 *    Node = key@8 val@16 prev@24 next@36 (12-byte references: store, record, position) = 6 words;
 *  - a stored reference marks its target REFERENCED; a removed referenced record is a tombstone of
 *    one word (removed bit set, read back as null), the rest of it freed (@FR-H-Removed);
 *  - freed records are parked by exact size and reused (the store's parks), claims tracked in a
 *    bit set;
 *  - the hash: power-of-two tagged slots (entry, digest low 32), linear probe, grown x2 when half
 *    full, rehash by tag, backward-shift removal by tag, the seeded SipHash-1-3 digest (sip.h is
 *    loft's own code, with its one-entry memo).
 * Left out: the generic key descriptors, the per-call decoding of the table header, the store
 * lookup per access, out-of-line bit inserts, the release walk of a node with no owned fields. */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
/* ---- loft's own SipHash-1-3 digest of one integer key (src/c/loft_rt.h), unchanged ---- */
#define LOFT_HOT_INLINE static inline
#define LOFT_TLS _Thread_local
typedef struct { uint64_t v0, v1, v2, v3, tail; uint64_t length; uint32_t ntail; } loft_sip;

#define LOFT_ROTL(x, b) (((x) << (b)) | ((x) >> (64 - (b))))

LOFT_HOT_INLINE void loft_sip_round(loft_sip *h) {
    h->v0 += h->v1;
    h->v2 += h->v3;
    h->v1 = LOFT_ROTL(h->v1, 13);
    h->v1 ^= h->v0;
    h->v3 = LOFT_ROTL(h->v3, 16);
    h->v3 ^= h->v2;
    h->v0 = LOFT_ROTL(h->v0, 32);
    h->v2 += h->v1;
    h->v0 += h->v3;
    h->v1 = LOFT_ROTL(h->v1, 17);
    h->v1 ^= h->v2;
    h->v3 = LOFT_ROTL(h->v3, 21);
    h->v3 ^= h->v0;
    h->v2 = LOFT_ROTL(h->v2, 32);
}

LOFT_HOT_INLINE void loft_sip_init(loft_sip *h) {
    h->v0 = UINT64_C(0x736f6d6570736575);
    h->v1 = UINT64_C(0x646f72616e646f6d);
    h->v2 = UINT64_C(0x6c7967656e657261);
    h->v3 = UINT64_C(0x7465646279746573);
    h->tail = 0;
    h->length = 0;
    h->ntail = 0;
}

/* Up to 7 bytes as a little-endian word. */
static uint64_t loft_sip_le(const uint8_t *buf, uint32_t len) {
    uint64_t out = 0;
    for (uint32_t i = 0; i < len; i++) out |= (uint64_t) buf[i] << (i * 8);
    return out;
}

LOFT_HOT_INLINE void loft_sip_word(loft_sip *h, uint64_t mi) {
    h->v3 ^= mi;
    loft_sip_round(h);
    h->v0 ^= mi;
}

LOFT_HOT_INLINE uint64_t loft_sip_finish(const loft_sip *in) {
    loft_sip h = *in;
    uint64_t b = ((h.length & 0xff) << 56) | h.tail;
    loft_sip_word(&h, b);
    h.v2 ^= 0xff;
    loft_sip_round(&h);
    loft_sip_round(&h);
    loft_sip_round(&h);
    return h.v0 ^ h.v1 ^ h.v2 ^ h.v3;
}

static LOFT_TLS uint64_t loft_dm_seed, loft_dm_key, loft_dm_hv;
static LOFT_TLS int loft_dm_ok;

/* keys::key_hash of the one integer key `[key]` under `seed`: the seed's word and the key's, then
 * the finish for sixteen bytes — what the general digest computes for `loft_kv_int(key)`,
 * unrolled (`@FR-R-TypedKeyed`'s digest). */
LOFT_HOT_INLINE uint64_t loft_digest_i64(uint64_t seed, int64_t key) {
    if (loft_dm_ok && loft_dm_key == (uint64_t) key && loft_dm_seed == seed) return loft_dm_hv;
    loft_dm_seed = seed;
    loft_dm_key = (uint64_t) key;
    loft_dm_ok = 1;
    loft_sip h;
    uint8_t b[8];
    loft_sip_init(&h);
    memcpy(b, &seed, 8);
    loft_sip_word(&h, loft_sip_le(b, 7) | (uint64_t) b[7] << 56);
    memcpy(b, &key, 8);
    loft_sip_word(&h, loft_sip_le(b, 7) | (uint64_t) b[7] << 56);
    h.length = 16;
    h.tail = 0;
    h.ntail = 0;
    loft_dm_hv = loft_sip_finish(&h);
    return loft_dm_hv;
}



/* ---- the store ---- */
typedef struct { uint64_t *w; uint32_t n, cap; } bits;
static inline int bits_has(const bits *b, uint32_t i) { uint32_t k = i >> 6; return k < b->n && (b->w[k] >> (i & 63)) & 1; }
static void bits_grow(bits *b, uint32_t k) {
    uint32_t cap = b->cap ? b->cap : 16;
    while (cap <= k) cap *= 2;
    if (cap != b->cap) { b->w = realloc(b->w, cap * 8); memset(b->w + b->cap, 0, (cap - b->cap) * 8); b->cap = cap; }
    b->n = k + 1;
}
static inline void bits_set(bits *b, uint32_t i) {
    uint32_t k = i >> 6;
    if (k >= b->n) bits_grow(b, k);
    b->w[k] |= 1ull << (i & 63);
}
static inline void bits_clear(bits *b, uint32_t i) { uint32_t k = i >> 6; if (k < b->n) b->w[k] &= ~(1ull << (i & 63)); }
static inline int bits_take(bits *b, uint32_t i) {
    uint32_t k = i >> 6; if (k >= b->n) return 0;
    uint64_t m = 1ull << (i & 63); int had = (b->w[k] & m) != 0; b->w[k] &= ~m; return had;
}

#define PARKS 17
typedef struct { uint8_t *buf; uint32_t size, cap; uint32_t park[PARKS]; bits claims, referenced, removed; } store;

static inline uint8_t *at(store *s, uint32_t rec, uint32_t byte) { return s->buf + (size_t) rec * 8 + byte; }
static inline uint32_t rd32(store *s, uint32_t rec, uint32_t byte) { uint32_t v; memcpy(&v, at(s, rec, byte), 4); return v; }
static inline void wr32(store *s, uint32_t rec, uint32_t byte, uint32_t v) { memcpy(at(s, rec, byte), &v, 4); }
static inline int64_t rd64(store *s, uint32_t rec, uint32_t byte) { int64_t v; memcpy(&v, at(s, rec, byte), 8); return v; }
static inline void wr64(store *s, uint32_t rec, uint32_t byte, int64_t v) { memcpy(at(s, rec, byte), &v, 8); }

static void store_reset(store *s) {
    s->size = 1;  /* record 0 is null */
    memset(s->park, 0, sizeof s->park);
    s->claims.n = s->referenced.n = s->removed.n = 0;
    if (s->claims.w) memset(s->claims.w, 0, s->claims.cap * 8);
    if (s->referenced.w) memset(s->referenced.w, 0, s->referenced.cap * 8);
    if (s->removed.w) memset(s->removed.w, 0, s->removed.cap * 8);
}
static uint32_t claim(store *s, uint32_t words) {
    uint32_t rec = words < PARKS ? s->park[words] : 0;
    if (rec) {
        s->park[words] = rd32(s, rec, 4);
    } else {
        if (s->size + words > s->cap) {
            uint32_t cap = s->cap ? s->cap * 2 : 1024;
            while (cap < s->size + words) cap *= 2;
            s->buf = realloc(s->buf, (size_t) cap * 8);
            s->cap = cap;
        }
        rec = s->size;
        s->size += words;
    }
    wr32(s, rec, 0, words);
    bits_set(&s->claims, rec);
    return rec;
}
static void park(store *s, uint32_t rec, uint32_t words) {
    bits_clear(&s->claims, rec);
    wr32(s, rec, 0, words);
    wr32(s, rec, 4, s->park[words]);
    s->park[words] = rec;
}
/* Store::delete: a referenced record leaves a one-word tombstone, its rest freed. */
static void delete_rec(store *s, uint32_t rec) {
    uint32_t words = rd32(s, rec, 0);
    if (bits_take(&s->referenced, rec)) {
        bits_clear(&s->claims, rec);
        bits_set(&s->removed, rec);
        wr32(s, rec, 0, 1);
        wr32(s, rec, 4, 0);
        if (words > 1) park(s, rec + 1, words - 1);
        return;
    }
    park(s, rec, words);
}

/* ---- references: (store, record, position), 12 bytes stored ---- */
typedef struct { uint32_t st, rec, pos; } ref;
static const ref NUL = { 0xFFFF, 0, 0 };
#ifdef HAND_STORETAB
typedef struct { store *st; int live; } slot_t;
static slot_t stores[4];
static uint32_t nstores = 1;
#define SO(r) (stores[(r).st].st)
static inline ref get_ref(store *s, uint32_t rec, uint32_t byte) {
    ref r = { rd32(s, rec, byte), rd32(s, rec, byte + 4), rd32(s, rec, byte + 8) };
    if (r.rec != 0 && r.st != 0xFFFF && r.st < nstores && stores[r.st].live && bits_has(&SO(r)->removed, r.rec)) return NUL;
    return r;
}
#else
#define SO(r) (&S)
static inline ref get_ref(store *s, uint32_t rec, uint32_t byte) {
    ref r = { rd32(s, rec, byte), rd32(s, rec, byte + 4), rd32(s, rec, byte + 8) };
    if (r.rec != 0 && bits_has(&s->removed, r.rec)) return NUL;
    return r;
}
#endif
static inline void set_ref(store *s, uint32_t rec, uint32_t byte, ref r) {
    wr32(s, rec, byte, r.st); wr32(s, rec, byte + 4, r.rec); wr32(s, rec, byte + 8, r.pos);
#ifdef HAND_STORETAB
    if (r.rec != 0 && r.st < nstores && stores[r.st].live && !bits_has(&SO(r)->referenced, r.rec)) bits_set(&SO(r)->referenced, r.rec);
#else
    if (r.rec != 0 && !bits_has(&s->referenced, r.rec)) bits_set(&s->referenced, r.rec);
#endif
}

/* ---- the hash table ---- */
#define SLOT_RECORD 0x80000000u
typedef struct { uint32_t *slots; uint32_t count, length; uint64_t seed; } table;
static void t_init(table *t) { t->count = 2; t->length = 0; t->slots = calloc(2 * t->count, 4); t->seed = 0x2545F4914F6CDD1Dull; }
static void t_grow(table *t) {
    uint32_t nc = t->count * 2, *ns = calloc(2 * (size_t) nc, 4);
    for (uint32_t i = 0; i < t->count; i++) {
        uint32_t v = t->slots[2 * i];
        if (!v) continue;
        uint32_t tag = t->slots[2 * i + 1], a = tag & (nc - 1);
        while (ns[2 * a]) a = (a + 1) & (nc - 1);
        ns[2 * a] = v; ns[2 * a + 1] = tag;
    }
    free(t->slots); t->slots = ns; t->count = nc;
}
/* The node record under `k`, 0 when absent. */
static inline uint32_t t_find(store *s, table *t, int64_t k) {
    uint64_t hv = loft_digest_i64(t->seed, k);
    uint32_t tag = (uint32_t) hv, m = t->count - 1, a = tag & m;
    for (;;) {
        uint32_t v = t->slots[2 * a];
        if (!v) return 0;
        if (t->slots[2 * a + 1] == tag) {
            uint32_t rec = v & ~SLOT_RECORD;
            if (rd64(s, rec, 8) == k) return rec;
        }
        a = (a + 1) & m;
    }
}
static void t_insert(table *t, int64_t k, uint32_t rec) {
    if ((uint64_t) t->length * 2 >= t->count) t_grow(t);
    uint64_t hv = loft_digest_i64(t->seed, k);
    uint32_t tag = (uint32_t) hv, m = t->count - 1, a = tag & m;
    while (t->slots[2 * a]) a = (a + 1) & m;
    t->slots[2 * a] = rec | SLOT_RECORD; t->slots[2 * a + 1] = tag;
    t->length++;
}
/* Removes `k`'s entry, the probe run behind it shifted back by tag; its record answered, 0 if none. */
static uint32_t t_remove(store *s, table *t, int64_t k) {
    uint64_t hv = loft_digest_i64(t->seed, k);
    uint32_t tag = (uint32_t) hv, m = t->count - 1, hole = tag & m, rec = 0;
    for (;;) {
        uint32_t v = t->slots[2 * hole];
        if (!v) return 0;
        if (t->slots[2 * hole + 1] == tag && rd64(s, v & ~SLOT_RECORD, 8) == k) { rec = v & ~SLOT_RECORD; break; }
        hole = (hole + 1) & m;
    }
    t->slots[2 * hole] = 0;
    for (uint32_t idx = (hole + 1) & m;; idx = (idx + 1) & m) {
        uint32_t v = t->slots[2 * idx];
        if (!v) break;
        uint32_t ideal = t->slots[2 * idx + 1] & m;
        if (((hole - ideal) & m) < ((idx - ideal) & m)) {
            t->slots[2 * hole] = v; t->slots[2 * hole + 1] = t->slots[2 * idx + 1];
            t->slots[2 * idx] = 0; hole = idx;
        }
    }
    t->length--;
    return rec;
}

/* ---- the program: Lru = cap@8, head@20, tail@32 (the table held aside) ---- */
enum { N_KEY = 8, N_VAL = 16, N_PREV = 24, N_NEXT = 36, NODE_WORDS = 6, C_HEAD = 20, C_TAIL = 32 };
static store S;
static table T;
static uint32_t C;  /* the Lru record */
#ifdef HAND_HDR
static table *tables[4];
typedef struct { uint32_t count, mask; uint64_t seed; uint32_t *slots; table *t; } hdr;
__attribute__((noinline)) static hdr decode(void) {
    hdr h; uint32_t claim = rd32(&S, C, 16);
    table *t = tables[claim == 0xFFFFFFFFu ? 0 : claim];
    uint32_t words = t->count + 4, flags = 0x80000000u | (uint32_t) (t->count == 0);
    h.count = (flags & 0x40000000u) ? 0 : 1u << (31 - __builtin_clz(words - 4));
    h.mask = h.count - 1;
    uint32_t lo = (uint32_t) t->seed, hi = (uint32_t) (t->seed >> 32);
    h.seed = (uint64_t) lo | (uint64_t) hi << 32;
    h.slots = t->slots; h.t = t;
    return h;
}
#endif

#ifdef HAND_HDR
#define TB() (decode().t)
#else
#define TB() (&T)
#endif
static inline ref R(uint32_t rec) { ref r = { 0, rec, 8 }; return r; }
static inline void unlink_(uint32_t n) {
    ref p = get_ref(&S, n, N_PREV), x = get_ref(&S, n, N_NEXT);
    if (p.rec) set_ref(SO(p), p.rec, N_NEXT, x); else set_ref(&S, C, C_HEAD, x);
    if (x.rec) set_ref(SO(x), x.rec, N_PREV, p); else set_ref(&S, C, C_TAIL, p);
    set_ref(&S, n, N_PREV, NUL);
    set_ref(&S, n, N_NEXT, NUL);
}
static inline void push_front(uint32_t n) {
    ref h = get_ref(&S, C, C_HEAD);
    set_ref(&S, n, N_NEXT, h);
    if (h.rec) set_ref(SO(h), h.rec, N_PREV, R(n)); else set_ref(&S, C, C_TAIL, R(n));
    set_ref(&S, C, C_HEAD, R(n));
}
static inline int64_t get(int64_t k) {
    uint32_t n = t_find(&S, TB(), k);
    if (!n) return -1;
    ref h = get_ref(&S, C, C_HEAD);
    if (rd64(&S, h.rec, N_KEY) != k) { unlink_(n); push_front(n); }
    return rd64(&S, n, N_VAL);
}
static inline void put(int64_t k, int64_t v) {
    uint32_t n = t_find(&S, TB(), k);
    if (n) { wr64(&S, n, N_VAL, v); unlink_(n); push_front(n); return; }
    ref t = get_ref(&S, C, C_TAIL);
    if ((int64_t) TB()->length >= rd64(&S, C, 8) && t.rec) {
        int64_t old = rd64(&S, t.rec, N_KEY);
        unlink_(t.rec);
        uint32_t gone = t_remove(&S, TB(), old);
        if (gone) delete_rec(&S, gone);
    }
    uint32_t m = claim(&S, NODE_WORDS);
    wr32(&S, m, 4, C);
    wr64(&S, m, N_KEY, k); wr64(&S, m, N_VAL, v);
    set_ref(&S, m, N_PREV, NUL); set_ref(&S, m, N_NEXT, NUL);
    t_insert(TB(), k, m);
    push_front(m);
}
static int64_t key(int64_t i, int64_t salt) { return ((i * 1103515245 + 12345 + salt * 7919) % 2147483648) % 100000; }

static int64_t lru_mixed(int64_t total, int64_t salt) {
    store_reset(&S);
    C = claim(&S, 6);
    wr64(&S, C, 8, 500);
    set_ref(&S, C, C_HEAD, NUL); set_ref(&S, C, C_TAIL, NUL);
    free(T.slots); t_init(&T);
#ifdef HAND_HDR
    tables[1] = &T; wr32(&S, C, 16, 1);
#endif
#ifdef HAND_STORETAB
    stores[0].st = &S; stores[0].live = 1;
#endif
    int64_t hits = 0, sum = 0;
    for (int64_t i = 0; i < total; i++) {
        int64_t k = key(i, salt) % 1000;
        int64_t v = get(k);
        if (v < 0) put(k, i);
        else { hits++; sum = (sum + v) % 1000000007; }
    }
    int64_t walk = 0;
    for (ref p = get_ref(&S, C, C_HEAD); p.rec; p = get_ref(&S, p.rec, N_NEXT))
        walk = (walk * 31 + rd64(&S, p.rec, N_KEY)) % 1000000007;
    return hits * 1000003 + (sum + walk) % 1000000007;
}

static int64_t us_now(void) { struct timespec t; clock_gettime(CLOCK_MONOTONIC, &t); return t.tv_sec * 1000000 + t.tv_nsec / 1000; }
int main(int argc, char **argv) {
    int64_t n = 10;
    for (int i = 1; i + 1 < argc; i++) if (!strcmp(argv[i], "--n")) n = atoll(argv[i + 1]);
    if (n < 2) n = 2;
    int64_t t0 = us_now(), sink = 0;
    for (int64_t r = 0; r < n; r++) sink += lru_mixed(4000, r & 1);
    int64_t us = us_now() - t0, res = lru_mixed(4000, 0);
    printf("routine\titers\tus\tns_op\tpx\tns_px\thash\n");
    printf("lru_mixed\t%lld\t%lld\t%lld\t4000\t%.3f\t%llx\n", (long long) n, (long long) us,
           (long long) (us * 1000 / n), (double) us * 1000 / (n * 4000), (unsigned long long) res);
    printf("time: %lldms sink=%lld\n", (long long) (us / 1000), (long long) sink);
    return 0;
}
