/* ── Key parts and their digest (src/keys.rs, src/siphash.rs) ──────────────────────────────
 * A key part (keys::Key, `loft_key`) names its kind (negative: descending), its byte position
 * in the element and the start a narrow width counts from.  Its value is a `loft_kv`
 * (keys::Content).  A hash table files an entry by the SipHash-1-3 digest of its key, seeded
 * per table: the digest is part of the store format, so it is loft's own, byte for byte
 * (`@FR-S-Digest`). */

#define LOFT_KV_INT 0
#define LOFT_KV_SINGLE 1
#define LOFT_KV_FLOAT 2
#define LOFT_KV_TEXT 3

/* One key part's value (keys::Content). */
typedef struct { int kind; int64_t i; float g; double f; loft_str s; } loft_kv;

static loft_kv loft_kv_int(int64_t i) {
    loft_kv k;
    memset(&k, 0, sizeof k);
    k.kind = LOFT_KV_INT;
    k.i = i;
    return k;
}

static loft_kv loft_kv_single(float g) {
    loft_kv k = loft_kv_int(0);
    k.kind = LOFT_KV_SINGLE;
    k.g = g;
    return k;
}

static loft_kv loft_kv_float(double f) {
    loft_kv k = loft_kv_int(0);
    k.kind = LOFT_KV_FLOAT;
    k.f = f;
    return k;
}

static loft_kv loft_kv_text(loft_str s) {
    loft_kv k = loft_kv_int(0);
    k.kind = LOFT_KV_TEXT;
    k.s = s;
    return k;
}

/* keys::compare_key's reads: the value of key part `k` of the element at (rec, pos). */
static loft_kv loft_key_get(loft_store_t *s, uint32_t rec, uint32_t pos, const loft_key *k) {
    uint32_t at = pos + k->pos;
    int t = k->type < 0 ? -k->type : k->type;
    switch (t) {
    case 1:
    case 2: {
        int64_t v;
        memcpy(&v, loft_at(s, rec, at), 8);
        return loft_kv_int(v);
    }
    case 3: {
        float v;
        memcpy(&v, loft_at(s, rec, at), 4);
        return loft_kv_single(v);
    }
    case 4: {
        double v;
        memcpy(&v, loft_at(s, rec, at), 8);
        return loft_kv_float(v);
    }
    case 6: {
        return loft_kv_text(loft_store_str(s, loft_u32(s, rec, at)));
    }
    case 8: {
        int32_t v;
        memcpy(&v, loft_at(s, rec, at), 4);
        return loft_kv_int(v);
    }
    case 12:
        return loft_kv_int(loft_u32(s, rec, at));
    case 9: {
        uint16_t raw;
        memcpy(&raw, loft_at(s, rec, at), 2);
        return loft_kv_int(raw != 0 ? (int64_t) raw + k->start - 1 : (int64_t) INT32_MIN);
    }
    case 11: {
        uint16_t raw;
        memcpy(&raw, loft_at(s, rec, at), 2);
        return loft_kv_int((int64_t) raw + k->start);
    }
    default:
        return loft_kv_int((int64_t) *loft_at(s, rec, at) + k->start);
    }
}

/* `total_cmp` as an unsigned order: a negative value's bits reversed, a positive one's above
 * every negative.  A zero of either sign is the one key 0 (@FR-E-Eq-Key). */
static uint64_t loft_total64(double d) {
    uint64_t u;
    if (d == 0.0) d = 0.0;
    memcpy(&u, &d, 8);
    return (u >> 63) ? ~u : u | (UINT64_C(1) << 63);
}

static uint32_t loft_total32(float g) {
    uint32_t u;
    if (g == 0.0f) g = 0.0f;
    memcpy(&u, &g, 4);
    return (u >> 31) ? ~u : u | (UINT32_C(1) << 31);
}

static int loft_kv_cmp(loft_kv a, loft_kv b) {
    switch (a.kind) {
    case LOFT_KV_SINGLE: {
        uint32_t x = loft_total32(a.g), y = loft_total32(b.g);
        return x < y ? -1 : x > y;
    }
    case LOFT_KV_FLOAT: {
        uint64_t x = loft_total64(a.f), y = loft_total64(b.f);
        return x < y ? -1 : x > y;
    }
    case LOFT_KV_TEXT: {
        size_t n = a.s.len < b.s.len ? a.s.len : b.s.len;
        int c = n ? memcmp(a.s.p, b.s.p, n) : 0;
        if (c != 0) return c < 0 ? -1 : 1;
        return a.s.len < b.s.len ? -1 : a.s.len > b.s.len;
    }
    default:
        return a.i < b.i ? -1 : a.i > b.i;
    }
}

/* keys::key_compare: the `n` given parts against the element's, a descending part reversed. */
static int loft_key_cmp_val(const loft_kv *kv, int n, loft_store_t *s, uint32_t rec, uint32_t pos,
                            const loft_key *keys) {
    for (int i = 0; i < n; i++) {
        int c = loft_kv_cmp(kv[i], loft_key_get(s, rec, pos, &keys[i]));
        if (c != 0) return keys[i].type < 0 ? -c : c;
    }
    return 0;
}

/* keys::compare: element a against element b on every part. */
static int loft_key_cmp_recs(loft_store_t *s, uint32_t ra, uint32_t pa, uint32_t rb, uint32_t pb,
                             const loft_key *keys, int n) {
    for (int i = 0; i < n; i++) {
        int c = loft_kv_cmp(loft_key_get(s, ra, pa, &keys[i]), loft_key_get(s, rb, pb, &keys[i]));
        if (c != 0) return keys[i].type < 0 ? -c : c;
    }
    return 0;
}

/* keys::compare between elements of two stores: a against b on every part. */
static int loft_key_cmp_refs(loft_ref a, loft_ref b, const loft_key *keys, int n) {
    for (int i = 0; i < n; i++) {
        int c = loft_kv_cmp(loft_key_get(loft_store_of(a), a.rec, a.pos, &keys[i]),
                            loft_key_get(loft_store_of(b), b.rec, b.pos, &keys[i]));
        if (c != 0) return keys[i].type < 0 ? -c : c;
    }
    return 0;
}

/* SipHash-1-3 keyed with k0 = k1 = 0 (siphash.rs: what std's DefaultHasher computes), fed the
 * bytes std's Hash impls feed it: an integer as its eight native-endian bytes, a text as its
 * bytes and a 0xFF terminator. */
typedef struct { uint64_t v0, v1, v2, v3, tail; uint64_t length; uint32_t ntail; } loft_sip;

#define LOFT_ROTL(x, b) (((x) << (b)) | ((x) >> (64 - (b))))

static void loft_sip_round(loft_sip *h) {
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

static void loft_sip_init(loft_sip *h) {
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

static void loft_sip_word(loft_sip *h, uint64_t mi) {
    h->v3 ^= mi;
    loft_sip_round(h);
    h->v0 ^= mi;
}

static void loft_sip_write(loft_sip *h, const uint8_t *msg, size_t length) {
    size_t needed = 0;
    h->length += length;
    if (h->ntail != 0) {
        needed = 8 - h->ntail;
        h->tail |= loft_sip_le(msg, (uint32_t) (length < needed ? length : needed)) << (8 * h->ntail);
        if (length < needed) {
            h->ntail += (uint32_t) length;
            return;
        }
        loft_sip_word(h, h->tail);
        h->ntail = 0;
    }
    size_t len = length - needed;
    size_t left = len & 7;
    size_t i = needed;
    while (i < len - left) {
        loft_sip_word(h, loft_sip_le(msg + i, 7) | (uint64_t) msg[i + 7] << 56);
        i += 8;
    }
    h->tail = loft_sip_le(msg + i, (uint32_t) left);
    h->ntail = (uint32_t) left;
}

static void loft_sip_i64(loft_sip *h, int64_t v) {
    uint8_t b[8];
    memcpy(b, &v, 8);
    loft_sip_write(h, b, 8);
}

static void loft_sip_str(loft_sip *h, loft_str s) {
    static const uint8_t end = 0xFF;
    if (s.len) loft_sip_write(h, (const uint8_t *) s.p, s.len);
    loft_sip_write(h, &end, 1);
}

static uint64_t loft_sip_finish(const loft_sip *in) {
    loft_sip h = *in;
    uint64_t b = ((h.length & 0xff) << 56) | h.tail;
    loft_sip_word(&h, b);
    h.v2 ^= 0xff;
    loft_sip_round(&h);
    loft_sip_round(&h);
    loft_sip_round(&h);
    return h.v0 ^ h.v1 ^ h.v2 ^ h.v3;
}

/* keys::seeded_hasher: the table's seed is the message's first word. */
static void loft_sip_seeded(loft_sip *h, uint64_t seed) {
    loft_sip_init(h);
    loft_sip_i64(h, (int64_t) seed);
}

/* One key part into the digest: an integer kind its value, a text its bytes; a float part adds
 * nothing (keys::key_hash, keys::hash_ref). */
static void loft_sip_kv(loft_sip *h, loft_kv k) {
    if (k.kind == LOFT_KV_INT) loft_sip_i64(h, k.i);
    else if (k.kind == LOFT_KV_TEXT) loft_sip_str(h, k.s);
}

/* keys::key_hash of the one integer key `[key]` under `seed`: the seed's word and the key's, then
 * the finish for sixteen bytes — what the general digest computes for `loft_kv_int(key)`,
 * unrolled (`@FR-R-TypedKeyed`'s digest). */
static uint64_t loft_digest_i64(uint64_t seed, int64_t key) {
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
    return loft_sip_finish(&h);
}

/* keys::ints_hash: the digest of `n` integer key parts — the seed's word and one per part, no
 * tail, the finish for their bytes (`@FR-R-TypedKeyed`'s compound clause). */
static uint64_t loft_digest_ints(uint64_t seed, const int64_t *ks, int n) {
    loft_sip h;
    uint8_t b[8];
    loft_sip_init(&h);
    memcpy(b, &seed, 8);
    loft_sip_word(&h, loft_sip_le(b, 7) | (uint64_t) b[7] << 56);
    for (int i = 0; i < n; i++) {
        memcpy(b, &ks[i], 8);
        loft_sip_word(&h, loft_sip_le(b, 7) | (uint64_t) b[7] << 56);
    }
    h.length = (uint64_t) (n + 1) * 8;
    h.tail = 0;
    h.ntail = 0;
    return loft_sip_finish(&h);
}

/* keys::key_hash: the digest of `n` key values. */
static uint64_t loft_kv_digest(const loft_kv *kv, int n, uint64_t seed) {
    if (n == 1 && kv[0].kind == LOFT_KV_INT) return loft_digest_i64(seed, kv[0].i);
    if (n > 1 && n <= 4) {
        int64_t ks[4];
        int ints = 1;
        for (int i = 0; i < n; i++) {
            if (kv[i].kind != LOFT_KV_INT) ints = 0;
            ks[i] = kv[i].i;
        }
        if (ints) return loft_digest_ints(seed, ks, n);
    }
    loft_sip h;
    loft_sip_seeded(&h, seed);
    for (int i = 0; i < n; i++) loft_sip_kv(&h, kv[i]);
    return loft_sip_finish(&h);
}

/* keys::hash: the digest of the key an element carries. */
static uint64_t loft_ref_digest(loft_ref e, const loft_key *keys, int n, uint64_t seed) {
    if (n == 1) {
        loft_kv k = loft_key_get(loft_store_of(e), e.rec, e.pos, &keys[0]);
        if (k.kind == LOFT_KV_INT) return loft_digest_i64(seed, k.i);
    } else if (n <= 4) {
        int64_t ks[4];
        int ints = 1;
        for (int i = 0; i < n; i++) {
            loft_kv k = loft_key_get(loft_store_of(e), e.rec, e.pos, &keys[i]);
            if (k.kind != LOFT_KV_INT) ints = 0;
            ks[i] = k.i;
        }
        if (ints) return loft_digest_ints(seed, ks, n);
    }
    loft_sip h;
    loft_sip_seeded(&h, seed);
    for (int i = 0; i < n; i++) loft_sip_kv(&h, loft_key_get(loft_store_of(e), e.rec, e.pos, &keys[i]));
    return loft_sip_finish(&h);
}

/* A header built without `loft_cc.h` (the replays of tests/c_emit.rs) keeps one copy of its
 * state: no threads. */
#ifndef LOFT_TLS
#define LOFT_TLS
#endif

/* keys::fresh_seed: a new table's seed — LOFT_HASH_SEED's value (decimal or 0x hex) when set, so
 * a persisted store is byte-reproducible, else an unpredictable one (the hash-DoS defence).
 * Never 0, which marks an unseeded table. */
static uint64_t loft_fresh_seed(void) {
    static LOFT_TLS int fixed_read = 0, fixed_set = 0;
    static LOFT_TLS uint64_t fixed = 0, counter = 0;
    if (!fixed_read) {
        const char *raw = getenv("LOFT_HASH_SEED");
        fixed_read = 1;
        if (raw) {
            const char *v = raw;
            while (*v == ' ' || *v == '\t') v++;
            if (*v) {
                int hex = v[0] == '0' && (v[1] == 'x' || v[1] == 'X');
                char *end = 0;
                unsigned long long n = strtoull(hex ? v + 2 : v, &end, hex ? 16 : 10);
                while (end && (*end == ' ' || *end == '\t' || *end == '\n')) end++;
                if (end && *end == 0 && (!hex || v[2] != 0) && v[0] != '-' && v[0] != '+') {
                    fixed = (uint64_t) n;
                    fixed_set = 1;
                } else {
                    fprintf(stderr, "warning: LOFT_HASH_SEED=\"%s\" is not a 64-bit number — hashing stays "
                                    "random, so a persisted store will NOT be byte-reproducible\n", raw);
                }
            }
        }
    }
    uint64_t s = fixed;
    if (!fixed_set) {
        FILE *f = fopen("/dev/urandom", "rb");
        if (!f || fread(&s, 1, 8, f) != 8) {
            /* No entropy source: the clock, an address and a count, mixed (splitmix64). */
            s = (uint64_t) time(0) ^ ((uint64_t) clock() << 20) ^ (uint64_t) (size_t) &counter ^ ++counter * UINT64_C(0x9E3779B97F4A7C15);
            s ^= s >> 30;
            s *= UINT64_C(0xBF58476D1CE4E5B9);
            s ^= s >> 27;
            s *= UINT64_C(0x94D049BB133111EB);
            s ^= s >> 31;
        }
        if (f) fclose(f);
    }
    return s == 0 ? UINT64_C(0x9E3779B97F4A7C15) : s;
}

