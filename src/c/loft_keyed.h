/* ── Keyed collections: hash, sorted, index ──────────────────────────────────────────────────
 * The orders and answers of src/hash.rs, src/vector.rs (`sorted_*`) and src/btree.rs, in
 * layouts of the C runtime's own: none of them is observable, because every walk over a keyed
 * collection is in KEY order (@FR-Col-Order) and every lookup is by key.
 *
 *   hash    the collection slot names a TABLE record: @4 the live count, @8 the slot count (a
 *           power of two), @12 the slots taken (live and removed), the slots from @16 — each the
 *           record number of an entry, 0 empty, LOFT_TOMB removed.  An entry is a record of its
 *           own in the collection's store, at byte 8.
 *   index   the slot names an ARRAY record: @4 the count, from @8 the entries' record numbers in
 *           key order.  An entry is a record of its own, at byte 8.
 *   sorted  a vector whose elements are kept in key order (`sorted_finish`): its layout IS a
 *           vector's, so a vector's readers read it.
 *
 * A key part (keys::Key) names its kind (negative: descending), its byte position in the
 * element and the start a narrow width counts from. */

typedef struct { int8_t type; uint16_t pos; int32_t start; } loft_key;

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
        uint32_t srec = loft_u32(s, rec, at);
        if (srec == 0 || srec > 0x7FFFFFFFu) return loft_kv_text(loft_null_text());
        return loft_kv_text(loft_lit((const char *) loft_at(s, srec, 8), loft_u32(s, srec, 4)));
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

/* A hash of key values that agrees with loft_kv_cmp's equality. */
static uint64_t loft_kv_hash(uint64_t h, loft_kv k) {
    uint64_t v;
    switch (k.kind) {
    case LOFT_KV_SINGLE:
        v = loft_total32(k.g);
        break;
    case LOFT_KV_FLOAT:
        v = loft_total64(k.f);
        break;
    case LOFT_KV_TEXT: {
        v = UINT64_C(14695981039346656037);
        for (size_t i = 0; i < k.s.len; i++) v = (v ^ (unsigned char) k.s.p[i]) * UINT64_C(1099511628211);
        break;
    }
    default:
        v = (uint64_t) k.i;
    }
    h ^= v + UINT64_C(0x9E3779B97F4A7C15) + (h << 6) + (h >> 2);
    h ^= h >> 31;
    h *= UINT64_C(0xBF58476D1CE4E5B9);
    h ^= h >> 29;
    return h;
}

static uint64_t loft_rec_hash(loft_store_t *s, uint32_t rec, uint32_t pos, const loft_key *keys, int n) {
    uint64_t h = 0;
    for (int i = 0; i < n; i++) h = loft_kv_hash(h, loft_key_get(s, rec, pos, &keys[i]));
    return h;
}

static uint64_t loft_val_hash(const loft_kv *kv, int n) {
    uint64_t h = 0;
    for (int i = 0; i < n; i++) h = loft_kv_hash(h, kv[i]);
    return h;
}

/* The collection record a keyed slot names, 0 for an empty or absent one or a null holder. */
static uint32_t loft_keyed_rec(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    return loft_collection_rec(loft_store_of(c), c.rec, c.pos);
}

/* The snapshots of hash walks still running (Stores::live_scratches), by the collection place
 * they were taken of: an entry a write displaces or removes leaves each walk over its hash —
 * its slot reads 0, which a step passes (`forget_in_scratches`, loft#1710). */
typedef struct { uint16_t scratch; uint16_t store; uint32_t rec; uint32_t pos; } loft_live_scratch;
static loft_live_scratch loft_scratches[64];
static int loft_nscratches = 0;

static void loft_forget_in_scratches(loft_ref c, uint32_t rec) {
    for (int i = 0; i < loft_nscratches; i++) {
        loft_live_scratch *w = &loft_scratches[i];
        if (w->store != c.store_nr || w->rec != c.rec || w->pos != c.pos) continue;
        loft_store_t *s = &loft_stores[w->scratch];
        uint32_t n = loft_u32(s, 1, 4);
        for (uint32_t j = 0; j < n; j++) {
            if (loft_u32(s, 1, 12 + j * 4) == rec) loft_set_u32(s, 1, 12 + j * 4, 0);
        }
    }
}

/* A new entry of a hash or an index (Stores::record_new): a record of its own in the
 * collection's store, filed by `finish` once its fields are written. */
static loft_ref loft_entry_new(loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    loft_ref e;
    e.store_nr = c.store_nr;
    e.rec = loft_claim(loft_store_of(c), 1 + (size + 7) / 8);
    e.pos = 8;
    return e;
}

/* OpClearKeyed: the collection holds nothing. */
static void loft_keyed_clear(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return;
    loft_set_u32(loft_store_of(c), c.rec, c.pos, 0);
}

/* ── hash ── */

#define LOFT_TOMB 0xFFFFFFFFu

static uint32_t loft_hash_make(loft_store_t *s, uint32_t cap) {
    uint32_t t = loft_claim(s, 2 + (cap + 1) / 2);
    loft_set_u32(s, t, 8, cap);
    return t;
}

/* A slot for an entry that is not in the table: the first empty or removed one. */
static uint32_t loft_hash_free_slot(loft_store_t *s, uint32_t t, uint64_t hv) {
    uint32_t cap = loft_u32(s, t, 8);
    uint32_t i = (uint32_t) (hv & (cap - 1));
    for (;;) {
        uint32_t r = loft_u32(s, t, 16 + i * 4);
        if (r == 0 || r == LOFT_TOMB) return i;
        i = (i + 1) & (cap - 1);
    }
}

/* A table with room for one more entry: the same one, or a rebuilt one without removed slots
 * and with twice the slots when it is half full of live entries. */
static uint32_t loft_hash_room(loft_ref c, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(c);
    uint32_t t = loft_collection_rec(s, c.rec, c.pos);
    if (t == 0) {
        t = loft_hash_make(s, 16);
        loft_set_u32(s, c.rec, c.pos, t);
        return t;
    }
    uint32_t cap = loft_u32(s, t, 8);
    if ((loft_u32(s, t, 12) + 1) * 2 <= cap) return t;
    uint32_t live = loft_u32(s, t, 4);
    uint32_t ncap = (live + 1) * 2 > cap / 2 ? cap * 2 : cap;
    uint32_t nt = loft_hash_make(s, ncap);
    for (uint32_t i = 0; i < cap; i++) {
        uint32_t r = loft_u32(s, t, 16 + i * 4);
        if (r == 0 || r == LOFT_TOMB) continue;
        uint32_t j = loft_hash_free_slot(s, nt, loft_rec_hash(s, r, 8, keys, nk));
        loft_set_u32(s, nt, 16 + j * 4, r);
    }
    loft_set_u32(s, nt, 4, live);
    loft_set_u32(s, nt, 12, live);
    loft_set_u32(s, c.rec, c.pos, nt);
    return nt;
}

/* record_finish on a hash (@FR-Col-Insert): the entry is filed under its key; an entry the
 * key already named is replaced (latest insert wins), and its record number answered. */
static uint32_t loft_hash_finish(loft_ref c, loft_ref e, const loft_key *keys, int nk) {
    if (c.store_nr == 0xFFFF || c.rec == 0 || e.rec == 0) return 0;
    uint32_t t = loft_hash_room(c, keys, nk);
    loft_store_t *s = loft_store_of(c);
    uint32_t cap = loft_u32(s, t, 8);
    uint32_t i = (uint32_t) (loft_rec_hash(s, e.rec, e.pos, keys, nk) & (cap - 1));
    uint32_t tomb = LOFT_TOMB;
    for (;;) {
        uint32_t r = loft_u32(s, t, 16 + i * 4);
        if (r == 0) break;
        if (r == LOFT_TOMB) {
            if (tomb == LOFT_TOMB) tomb = i;
        } else if (loft_key_cmp_recs(s, r, 8, e.rec, e.pos, keys, nk) == 0) {
            loft_set_u32(s, t, 16 + i * 4, e.rec);
            loft_forget_in_scratches(c, r);
            return r;
        }
        i = (i + 1) & (cap - 1);
    }
    if (tomb != LOFT_TOMB) i = tomb;
    else loft_set_u32(s, t, 12, loft_u32(s, t, 12) + 1);
    loft_set_u32(s, t, 16 + i * 4, e.rec);
    loft_set_u32(s, t, 4, loft_u32(s, t, 4) + 1);
    return 0;
}

/* hash::find: the entry under the key, or the null reference. */
static loft_ref loft_hash_find(loft_ref c, const loft_key *keys, int nk, const loft_kv *kv, int n) {
    uint32_t t = loft_keyed_rec(c);
    if (t == 0) return loft_ref_null();
    loft_store_t *s = loft_store_of(c);
    uint32_t cap = loft_u32(s, t, 8);
    if (n == nk) {
        uint32_t i = (uint32_t) (loft_val_hash(kv, n) & (cap - 1));
        for (;;) {
            uint32_t r = loft_u32(s, t, 16 + i * 4);
            if (r == 0) return loft_ref_null();
            if (r != LOFT_TOMB && loft_key_cmp_val(kv, n, s, r, 8, keys) == 0) {
                loft_ref e = { c.store_nr, r, 8 };
                return e;
            }
            i = (i + 1) & (cap - 1);
        }
    }
    for (uint32_t i = 0; i < cap; i++) {
        uint32_t r = loft_u32(s, t, 16 + i * 4);
        if (r != 0 && r != LOFT_TOMB && loft_key_cmp_val(kv, n, s, r, 8, keys) == 0) {
            loft_ref e = { c.store_nr, r, 8 };
            return e;
        }
    }
    return loft_ref_null();
}

static int64_t loft_hash_len(loft_ref c) {
    uint32_t t = loft_keyed_rec(c);
    return t == 0 ? 0 : (int64_t) loft_u32(loft_store_of(c), t, 4);
}

/* The entry `e` leaves the hash (Stores::remove). */
static void loft_hash_remove(loft_ref c, loft_ref e, const loft_key *keys, int nk) {
    uint32_t t = loft_keyed_rec(c);
    if (t == 0 || e.rec == 0) return;
    loft_store_t *s = loft_store_of(c);
    uint32_t cap = loft_u32(s, t, 8);
    uint32_t i = (uint32_t) (loft_rec_hash(s, e.rec, e.pos, keys, nk) & (cap - 1));
    for (uint32_t n = 0; n < cap; n++) {
        uint32_t r = loft_u32(s, t, 16 + i * 4);
        if (r == 0) return;
        if (r == e.rec) {
            loft_set_u32(s, t, 16 + i * 4, LOFT_TOMB);
            loft_set_u32(s, t, 4, loft_u32(s, t, 4) - 1);
            loft_forget_in_scratches(c, r);
            return;
        }
        i = (i + 1) & (cap - 1);
    }
}

/* Record numbers of elements at byte 8, put in key order (a merge sort: the keys are unique). */
static void loft_sort_recs(loft_store_t *s, uint32_t *a, uint32_t *tmp, uint32_t n, const loft_key *keys,
                           int nk) {
    if (n < 2) return;
    uint32_t h = n / 2;
    loft_sort_recs(s, a, tmp, h, keys, nk);
    loft_sort_recs(s, a + h, tmp, n - h, keys, nk);
    uint32_t i = 0, j = h, k = 0;
    while (i < h && j < n) {
        if (loft_key_cmp_recs(s, a[j], 8, a[i], 8, keys, nk) < 0) tmp[k++] = a[j++];
        else tmp[k++] = a[i++];
    }
    while (i < h) tmp[k++] = a[i++];
    while (j < n) tmp[k++] = a[j++];
    memcpy(a, tmp, (size_t) n * 4);
}

/* hash::records_sorted: the entries' record numbers in key order, malloc'd; `*n` their count. */
static uint32_t *loft_hash_records(loft_ref c, const loft_key *keys, int nk, uint32_t *n) {
    *n = 0;
    uint32_t t = loft_keyed_rec(c);
    if (t == 0) return 0;
    loft_store_t *s = loft_store_of(c);
    uint32_t cap = loft_u32(s, t, 8), live = loft_u32(s, t, 4);
    uint32_t *a = (uint32_t *) malloc((size_t) (live + 1) * 4 * 2);
    if (!a) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint32_t i = 0; i < cap; i++) {
        uint32_t r = loft_u32(s, t, 16 + i * 4);
        if (r != 0 && r != LOFT_TOMB) a[(*n)++] = r;
    }
    loft_sort_recs(s, a, a + live + 1, *n, keys, nk);
    return a;
}

/* n_hash_sorted (Stores::build_hash_sorted_vec): a scratch in a store of its own — @4 the
 * count, @8 the hash's store, the entries' record numbers in key order from @12 — that the
 * loop walks and OpFreeScratch releases. */
static loft_ref loft_hash_sorted(loft_ref c, const loft_key *keys, int nk) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    uint32_t n;
    uint32_t *a = loft_hash_records(c, keys, nk, &n);
    uint32_t nr = loft_store_new();
    loft_store_t *s = &loft_stores[nr];
    uint32_t rec = loft_claim(s, 2 + (n + 1) / 2);
    loft_set_u32(s, rec, 4, n);
    loft_set_u32(s, rec, 8, c.store_nr);
    if (n) memcpy(loft_at(s, rec, 12), a, (size_t) n * 4);
    free(a);
    if (loft_nscratches < 64) {
        loft_live_scratch w = { (uint16_t) nr, c.store_nr, c.rec, c.pos };
        loft_scratches[loft_nscratches++] = w;
    }
    loft_ref r = { (uint16_t) nr, rec, 0 };
    return r;
}

static void loft_free_scratch(loft_ref r) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    for (int i = 0; i < loft_nscratches; i++) {
        if (loft_scratches[i].scratch != r.store_nr) continue;
        loft_scratches[i] = loft_scratches[--loft_nscratches];
        break;
    }
    loft_free_ref(r);
}

/* ── sorted and index: one key-ordered sequence each ── */

/* An ordered sequence: a sorted's inline elements of `size`, or (size 0) an index's record
 * numbers, each naming its entry at byte 8. */
typedef struct { loft_store_t *s; uint32_t vrec; uint32_t size; } loft_seq;

static void loft_seq_at(const loft_seq *q, uint32_t i, uint32_t *rec, uint32_t *pos) {
    if (q->size) {
        *rec = q->vrec;
        *pos = 8 + i * q->size;
    } else {
        *rec = loft_u32(q->s, q->vrec, 8 + i * 4);
        *pos = 8;
    }
}

/* vector::sorted_search over the first `len` elements: where the key goes, `before` every
 * element equal to it or after them, and whether one was.  The key is `n` values (`kv`), or
 * when `kv` is null the key of the element at (krec, kpos). */
static uint32_t loft_seq_search(const loft_seq *q, uint32_t len, int before, const loft_kv *kv, int n,
                                uint32_t krec, uint32_t kpos, const loft_key *keys, int nk, int *found) {
    *found = 0;
    if (len == 0) return 0;
    uint32_t left = 0, right = len - 1;
    for (;;) {
        uint32_t mid = left + (right - left) / 2, rec, pos;
        loft_seq_at(q, mid, &rec, &pos);
        int c = kv ? loft_key_cmp_val(kv, n, q->s, rec, pos, keys)
                   : loft_key_cmp_recs(q->s, krec, kpos, rec, pos, keys, nk);
        int action = c;
        if (c == 0) {
            *found = 1;
            action = before ? -1 : 1;
        }
        if (action < 0) {
            if (mid > 0) right = mid - 1;
            else { right = 0; left += 1; }
        } else {
            left = mid + 1;
        }
        if (left > right) return action > 0 ? mid + 1 : mid;
    }
}

static loft_seq loft_seq_of(loft_ref c, uint32_t size) {
    loft_seq q;
    q.s = loft_store_of(c);
    q.vrec = loft_keyed_rec(c);
    q.size = size;
    return q;
}

static uint32_t loft_seq_len(const loft_seq *q) {
    return q->vrec == 0 ? 0 : loft_u32(q->s, q->vrec, 4);
}

/* Stores::find on a sorted (`size` its element) or an index (`size` 0): the first element
 * whose key parts equal the `n` given, or the null reference. */
static loft_ref loft_seq_find(loft_ref c, uint32_t size, const loft_key *keys, int nk, const loft_kv *kv,
                              int n) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    loft_seq q = loft_seq_of(c, size);
    int found;
    uint32_t p = loft_seq_search(&q, loft_seq_len(&q), 1, kv, n, 0, 0, keys, nk, &found);
    if (!found) return loft_ref_null();
    loft_ref e;
    e.store_nr = c.store_nr;
    loft_seq_at(&q, p, &e.rec, &e.pos);
    return e;
}

/* vector::sorted_new: room for one more element, staged one slot past the end — the gap
 * `loft_sorted_finish` opens moves it into place — or in slot 0 of an empty sorted. */
static loft_ref loft_sorted_new(loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    loft_store_t *s = loft_store_of(c);
    uint32_t r = loft_collection_rec(s, c.rec, c.pos);
    uint32_t len = 0;
    if (r == 0) {
        r = loft_claim(s, loft_vec_cap_words(12, size));
        loft_set_u32(s, c.rec, c.pos, r);
        loft_set_u32(s, r, 4, 0);
    } else {
        len = loft_u32(s, r, 4);
        int32_t claimed;
        memcpy(&claimed, loft_at(s, r, 0), 4);
        uint32_t cap = (uint32_t) (((uint64_t) claimed * 8 - 8) / size);
        if (len + 2 > cap) {
            uint32_t nr = loft_claim(s, loft_vec_cap_words((len + 2) * 2, size));
            memcpy(loft_at(s, nr, 4), loft_at(s, r, 4), 4 + (size_t) len * size);
            loft_set_u32(s, c.rec, c.pos, nr);
            r = nr;
        }
    }
    loft_ref e;
    e.store_nr = c.store_nr;
    e.rec = r;
    e.pos = 8 + (len == 0 ? 0 : len + 1) * size;
    return e;
}

/* vector::sorted_finish: the staged element moves to its key's place; one whose key is
 * already there replaces that element (latest insert wins). */
static void loft_sorted_finish(loft_ref c, uint32_t size, const loft_key *keys, int nk) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return;
    loft_seq q = loft_seq_of(c, size);
    if (q.vrec == 0) return;
    uint32_t len = loft_u32(q.s, q.vrec, 4);
    if (len == 0) {
        loft_set_u32(q.s, q.vrec, 4, 1);
        return;
    }
    uint32_t latest = 8 + (len + 1) * size;
    int found;
    uint32_t p = loft_seq_search(&q, len, 1, 0, 0, q.vrec, latest, keys, nk, &found);
    uint8_t *base = loft_at(q.s, q.vrec, 0);
    if (found) {
        memmove(base + 8 + p * size, base + latest, size);
        return;
    }
    if (p < len) memmove(base + 8 + (p + 1) * size, base + 8 + p * size, (size_t) (len - p) * size);
    memmove(base + 8 + p * size, base + latest, size);
    loft_set_u32(q.s, q.vrec, 4, len + 1);
}

/* The element `e` leaves the sorted: its successors move down. */
static void loft_sorted_remove(loft_ref c, loft_ref e, uint32_t size) {
    uint32_t r = loft_keyed_rec(c);
    if (r == 0 || e.rec != r || e.pos < 8) return;
    loft_store_t *s = loft_store_of(c);
    uint32_t len = loft_u32(s, r, 4), i = (e.pos - 8) / size;
    if (i >= len) return;
    uint8_t *base = loft_at(s, r, 0);
    memmove(base + 8 + i * size, base + 8 + (i + 1) * size, (size_t) (len - i - 1) * size);
    loft_set_u32(s, r, 4, len - 1);
}

/* record_finish on an index or an ordered (@FR-Col-Insert): the entry's record number goes to
 * its key's place; an entry the key already named is replaced, and its number answered. */
static uint32_t loft_index_finish(loft_ref c, loft_ref e, const loft_key *keys, int nk) {
    if (c.store_nr == 0xFFFF || c.rec == 0 || e.rec == 0) return 0;
    loft_seq q = loft_seq_of(c, 0);
    uint32_t len = loft_seq_len(&q);
    int found;
    uint32_t p = loft_seq_search(&q, len, 1, 0, 0, e.rec, e.pos, keys, nk, &found);
    if (found) {
        uint32_t old = loft_u32(q.s, q.vrec, 8 + p * 4);
        loft_set_u32(q.s, q.vrec, 8 + p * 4, e.rec);
        return old;
    }
    loft_ref slot = loft_vec_append(c, 4);
    if (slot.rec == 0) return 0;
    loft_store_t *s = loft_store_of(c);
    uint8_t *base = loft_at(s, slot.rec, 0);
    if (p < len) memmove(base + 8 + (p + 1) * 4, base + 8 + p * 4, (size_t) (len - p) * 4);
    loft_set_u32(s, slot.rec, 8 + p * 4, e.rec);
    loft_set_u32(s, slot.rec, 4, len + 1);
    return 0;
}

/* The entry `e` leaves an index, an ordered or an array (an array only unlinks it). */
static void loft_index_remove(loft_ref c, loft_ref e) {
    uint32_t r = loft_keyed_rec(c);
    if (r == 0 || e.rec == 0) return;
    loft_store_t *s = loft_store_of(c);
    uint32_t len = loft_u32(s, r, 4);
    for (uint32_t i = 0; i < len; i++) {
        if (loft_u32(s, r, 8 + i * 4) != e.rec) continue;
        uint8_t *base = loft_at(s, r, 0);
        memmove(base + 8 + i * 4, base + 8 + (i + 1) * 4, (size_t) (len - i - 1) * 4);
        loft_set_u32(s, r, 4, len - 1);
        return;
    }
}

/* ── walks ──
 * A walk's state is the range of positions still to visit, [lo, hi): a forward step takes lo,
 * a reverse one hi - 1. */

static int64_t loft_iter_range(uint32_t lo, uint32_t hi) {
    if (hi < lo) hi = lo;
    return (int64_t) (((uint64_t) hi << 32) | lo);
}

/* The position the walk visits next, or -1 when it is done. */
static int64_t loft_iter_next(int64_t *state, int reverse) {
    uint64_t st = (uint64_t) *state;
    uint32_t lo = (uint32_t) st, hi = (uint32_t) (st >> 32);
    if (lo >= hi) return -1;
    int64_t at;
    if (reverse) { hi--; at = hi; }
    else { at = lo; lo++; }
    *state = loft_iter_range(lo, hi);
    return at;
}

/* OpIterate over a sorted (on 2) or an index (on 1): the positions of the key range.  `from` is
 * inclusive; `ex` says `till` is exclusive; an empty bound is open (State::iterate,
 * btree::range). */
static int64_t loft_seq_iterate(loft_ref c, uint32_t size, int ex, const loft_key *keys, int nk,
                                const loft_kv *from, int nfrom, const loft_kv *till, int ntill) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    loft_seq q = loft_seq_of(c, size);
    uint32_t len = loft_seq_len(&q);
    int found;
    uint32_t lo = nfrom == 0 ? 0 : loft_seq_search(&q, len, 1, from, nfrom, 0, 0, keys, nk, &found);
    uint32_t hi = ntill == 0 ? len : loft_seq_search(&q, len, ex, till, ntill, 0, 0, keys, nk, &found);
    return loft_iter_range(lo, hi);
}

/* OpStep over a sorted or an index: the next element, or the null reference. */
static loft_ref loft_seq_step(int64_t *state, loft_ref c, uint32_t size, int reverse) {
    int64_t at = loft_iter_next(state, reverse);
    if (at < 0 || c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    loft_seq q = loft_seq_of(c, size);
    if ((uint32_t) at >= loft_seq_len(&q)) return loft_ref_null();
    loft_ref e;
    e.store_nr = c.store_nr;
    loft_seq_at(&q, (uint32_t) at, &e.rec, &e.pos);
    return e;
}

/* OpIterate over a hash's scratch (on 4): every entry. */
static int64_t loft_scratch_iterate(loft_ref x) {
    if (x.store_nr == 0xFFFF || x.rec == 0) return 0;
    return loft_iter_range(0, loft_u32(loft_store_of(x), x.rec, 4));
}

/* OpStep over a hash's scratch: the next entry still in the hash, in the hash's store. */
static loft_ref loft_scratch_step(int64_t *state, loft_ref x, int reverse) {
    if (x.store_nr == 0xFFFF || x.rec == 0) return loft_ref_null();
    for (;;) {
        int64_t at = loft_iter_next(state, reverse);
        if (at < 0) return loft_ref_null();
        loft_store_t *s = loft_store_of(x);
        uint32_t r = loft_u32(s, x.rec, 12 + (uint32_t) at * 4);
        if (r == 0) continue;
        loft_ref e;
        e.store_nr = (uint16_t) loft_u32(s, x.rec, 8);
        e.rec = r;
        e.pos = 8;
        return e;
    }
}

/* ── copies, renders and equality over the entries ── */

/* The elements of a hash (in key order) or an index, as references; `*n` their count. */
static loft_ref *loft_keyed_refs(loft_ref c, int is_hash, const loft_key *keys, int nk, uint32_t *n) {
    *n = 0;
    uint32_t *recs = 0;
    uint32_t count = 0;
    if (is_hash) {
        recs = loft_hash_records(c, keys, nk, &count);
    } else {
        uint32_t r = loft_keyed_rec(c);
        if (r != 0) {
            count = loft_u32(loft_store_of(c), r, 4);
            recs = (uint32_t *) malloc((size_t) count * 4 + 4);
            if (!recs) { fputs("loft: out of memory\n", stderr); abort(); }
            if (count) memcpy(recs, loft_at(loft_store_of(c), r, 8), (size_t) count * 4);
        }
    }
    loft_ref *out = (loft_ref *) malloc(((size_t) count + 1) * sizeof *out);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint32_t i = 0; i < count; i++) {
        out[i].store_nr = c.store_nr;
        out[i].rec = recs[i];
        out[i].pos = 8;
    }
    free(recs);
    *n = count;
    return out;
}

/* write_records: the elements `e` rendered as a list. */
static void loft_show_refs(loft_text *s, const loft_ref *e, uint32_t n, int fmt, int ind, int complex_elem,
                           int struct_elem, loft_show_fn f) {
    int pretty = fmt & 1;
    int c = pretty && complex_elem;
    loft_push_char(s, '[');
    for (uint32_t i = 0; i < n; i++) {
        if (i == 0) {
            if (pretty) { if (c) loft_nl(s, ind + 1); else loft_push_char(s, ' '); }
        } else {
            loft_push_char(s, ',');
            if (pretty) { if (struct_elem) loft_nl(s, ind + 1); else loft_push_char(s, ' '); }
        }
        f(s, e[i], fmt, ind + 1);
    }
    if (pretty) loft_push_char(s, ' ');
    loft_push_char(s, ']');
}

static void loft_show_keyed(loft_text *s, loft_ref r, int fmt, int ind, int is_hash, const loft_key *keys,
                            int nk, int complex_elem, int struct_elem, loft_show_fn f) {
    if (loft_vec_absent(r)) { loft_push(s, "null", 4); return; }
    uint32_t n;
    loft_ref *e = loft_keyed_refs(r, is_hash, keys, nk, &n);
    loft_show_refs(s, e, n, fmt, ind, complex_elem, struct_elem, f);
    free(e);
}

static int loft_eq_keyed(loft_ref a, loft_ref b, int is_hash, const loft_key *keys, int nk, loft_eq_fn f) {
    uint32_t na, nb;
    loft_ref *ea = loft_keyed_refs(a, is_hash, keys, nk, &na);
    loft_ref *eb = loft_keyed_refs(b, is_hash, keys, nk, &nb);
    int same = na == nb;
    for (uint32_t i = 0; same && i < na; i++) same = f(ea[i], eb[i]);
    free(ea);
    free(eb);
    return same;
}

/* ── whole-collection writes ── */

/* OpReplaceKeyed (State::replace_keyed): `dest = src` for a keyed collection of type `tp` — an
 * absent source leaves the destination absent, the same collection is left alone, anything
 * else is cleared and refilled with deep copies; a source the bind gives up is freed. */
static void loft_replace_keyed(const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dest, int free_source) {
    if (src.store_nr == 0xFFFF) {
        loft_remove_claims(T, tp, dest);
        if (dest.store_nr != 0xFFFF && dest.rec != 0 && dest.pos != 0)
            loft_set_u32(loft_store_of(dest), dest.rec, dest.pos, 0xFFFFFFFFu);
        return;
    }
    if (src.store_nr == dest.store_nr && src.rec == dest.rec && src.pos == dest.pos) return;
    if (dest.store_nr == 0xFFFF || dest.rec == 0) return;
    loft_remove_claims(T, tp, dest);
    loft_copy_claims(T, tp, src, dest);
    if (free_source && src.store_nr != dest.store_nr) loft_free_ref(src);
}

/* keys::set_key: key part `k` of the element `e` takes the value `v`. */
static void loft_key_set(loft_ref e, const loft_key *k, loft_kv v) {
    loft_store_t *s = loft_store_of(e);
    uint32_t at = e.pos + k->pos;
    int t = k->type < 0 ? -k->type : k->type;
    switch (t) {
    case 1:
    case 2:
        memcpy(loft_at(s, e.rec, at), &v.i, 8);
        return;
    case 3:
        memcpy(loft_at(s, e.rec, at), &v.g, 4);
        return;
    case 4:
        memcpy(loft_at(s, e.rec, at), &v.f, 8);
        return;
    case 6:
        loft_set_text(e, k->pos, v.s);
        return;
    case 8: {
        int32_t x = loft_i32_of(v.i);
        memcpy(loft_at(s, e.rec, at), &x, 4);
        return;
    }
    case 12: {
        uint32_t x = (uint32_t) (uint64_t) v.i;
        memcpy(loft_at(s, e.rec, at), &x, 4);
        return;
    }
    case 9:
        loft_set_short(e, k->pos, k->start, v.i);
        return;
    case 11: {
        uint16_t x = (uint16_t) (uint64_t) (v.i - k->start);
        memcpy(loft_at(s, e.rec, at), &x, 2);
        return;
    }
    default:
        loft_set_byte_v(e, k->pos, k->start, v.i);
        return;
    }
}

#define LOFT_KEYED_HASH 0
#define LOFT_KEYED_SORTED 1
#define LOFT_KEYED_INDEX 2

/* The element of collection `c` (of kind `kind`) whose key parts equal the `n` given. */
static loft_ref loft_keyed_find(loft_ref c, int kind, uint32_t size, const loft_key *keys, int nk,
                                const loft_kv *kv, int n) {
    if (kind == LOFT_KEYED_HASH) return loft_hash_find(c, keys, nk, kv, n);
    return loft_seq_find(c, kind == LOFT_KEYED_SORTED ? size : 0, keys, nk, kv, n);
}

/* OpSetKeyed (Stores::set_keyed, @FR-Col-Assign): `c[k…] = v` — a deep copy of `v` filed under
 * the subscript's key (or, with no full subscript, under its own), replacing the element that
 * key named; a value the bind gives up is freed. */
static void loft_set_keyed(const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref value, int kind,
                           const loft_key *keys, int nk, const loft_kv *sub, int nsub, int free_source) {
    if (value.store_nr == 0xFFFF || value.rec == 0 || c.store_nr == 0xFFFF || c.rec == 0) return;
    uint32_t size = T[etp].size;
    int full = nsub > 0 && nsub == nk;
    loft_ref source = value, tmp = loft_ref_null();
    if (full && value.store_nr == c.store_nr) {
        loft_database(&tmp, size);
        loft_copy_bytes(tmp, 0, value, 0, size);
        loft_copy_claims(T, etp, value, tmp);
        source = tmp;
    }
    /* The key, its texts owned: the claims below may move the store a view points into. */
    loft_kv key[8];
    loft_text own[8];
    int n = full ? nsub : nk;
    if (n > 8) n = 8;
    for (int i = 0; i < n; i++) {
        key[i] = full ? sub[i] : loft_key_get(loft_store_of(source), source.rec, source.pos, &keys[i]);
        own[i].p = 0;
        own[i].len = own[i].cap = 0;
        if (key[i].kind == LOFT_KV_TEXT) {
            own[i] = loft_text_own(key[i].s);
            key[i].s = loft_view(&own[i]);
        }
    }
    loft_ref existing = loft_keyed_find(c, kind, size, keys, nk, key, n);
    if (existing.rec != 0) {
        if (kind == LOFT_KEYED_HASH) loft_hash_remove(c, existing, keys, nk);
        else if (kind == LOFT_KEYED_SORTED) loft_sorted_remove(c, existing, size);
        else loft_index_remove(c, existing);
    }
    loft_ref e = kind == LOFT_KEYED_SORTED ? loft_sorted_new(c, size) : loft_entry_new(c, size);
    loft_copy_bytes(e, 0, source, 0, size);
    loft_copy_claims(T, etp, source, e);
    if (full) for (int i = 0; i < n; i++) loft_key_set(e, &keys[i], key[i]);
    if (kind == LOFT_KEYED_HASH) loft_hash_finish(c, e, keys, nk);
    else if (kind == LOFT_KEYED_SORTED) loft_sorted_finish(c, size, keys, nk);
    else loft_index_finish(c, e, keys, nk);
    for (int i = 0; i < n; i++) loft_text_free(&own[i]);
    if (tmp.store_nr != 0xFFFF) loft_free_ref(tmp);
    if (free_source && value.store_nr != c.store_nr) loft_free_ref(value);
}

/* OpRemove (`e#remove`) in a walk over a sorted (`size` its element) or an index (`size` 0):
 * the element the walk visited last goes, and the walk goes on with the one after it. */
static void loft_seq_remove(int64_t *state, loft_ref c, uint32_t size, int reverse) {
    uint64_t st = (uint64_t) *state;
    uint32_t lo = (uint32_t) st, hi = (uint32_t) (st >> 32);
    uint32_t at;
    if (reverse) {
        at = hi;
    } else {
        if (lo == 0) return;
        at = lo - 1;
        lo--;
        if (hi > 0) hi--;
    }
    if (!loft_vec_remove(c, size ? size : 4, at)) return;
    *state = loft_iter_range(lo, hi);
}

/* ── arrays and linked groups (@FR-Col-Group) ──
 * An array is a vector of record numbers: the form a vector takes when its element type is
 * shared — by a stored reference or by several collections of one struct — so every route to
 * a record names the one record.  Its layout is an index's, unsorted. */

/* OpVectorRef / OpVectorRefNullable: the record the slot at `from` names (negative from the
 * end), or the null reference. */
static loft_ref loft_array_get(loft_ref v, int64_t from) {
    loft_ref slot = loft_vec_get(v, 4, from);
    if (slot.store_nr == 0xFFFF || slot.rec == 0) return loft_ref_null();
    uint32_t r = loft_u32(loft_store_of(slot), slot.rec, slot.pos);
    if (r == 0) return loft_ref_null();
    loft_ref e = { slot.store_nr, r, 8 };
    return e;
}

/* insert_record on an array: the record's number joins the end. */
static void loft_array_append(loft_ref v, loft_ref rec) {
    if (rec.rec == 0) return;
    loft_ref slot = loft_vec_append(v, 4);
    if (slot.rec == 0) return;
    loft_set_u32(loft_store_of(slot), slot.rec, slot.pos, rec.rec);
    loft_vec_finish(v);
}

#define LOFT_M_ARRAY 0
#define LOFT_M_HASH 1
#define LOFT_M_INDEX 2

/* One member of a linked group: its field's byte position, its kind (an index and an ordered
 * share LOFT_M_INDEX) and its key parts. */
typedef struct { uint32_t pos; int kind; const loft_key *keys; int nk; } loft_member;

/* record_finish on a member of a linked group (`link_siblings`, `settle_displaced`): the record
 * enters every member; a record a key collision displaced from one keyed member leaves every
 * keyed member that still files it under its key, and an array keeps it. */
static void loft_group_finish(loft_ref parent, loft_ref rec, const loft_member *m, int n) {
    uint32_t gone[16];
    int ng = 0;
    if (parent.store_nr == 0xFFFF || parent.rec == 0 || rec.rec == 0) return;
    for (int i = 0; i < n; i++) {
        loft_ref c = loft_field(parent, m[i].pos);
        uint32_t d = 0;
        if (m[i].kind == LOFT_M_ARRAY) loft_array_append(c, rec);
        else if (m[i].kind == LOFT_M_HASH) d = loft_hash_finish(c, rec, m[i].keys, m[i].nk);
        else d = loft_index_finish(c, rec, m[i].keys, m[i].nk);
        int seen = d == 0 || d == rec.rec;
        for (int j = 0; j < ng; j++) seen |= gone[j] == d;
        if (!seen && ng < 16) gone[ng++] = d;
    }
    for (int g = 0; g < ng; g++) {
        loft_ref dr = { rec.store_nr, gone[g], 8 };
        for (int i = 0; i < n; i++) {
            if (m[i].kind == LOFT_M_ARRAY) continue;
            loft_ref c = loft_field(parent, m[i].pos);
            loft_kv key[8];
            int nk = m[i].nk > 8 ? 8 : m[i].nk;
            for (int k = 0; k < nk; k++) key[k] = loft_key_get(loft_store_of(dr), dr.rec, 8, &m[i].keys[k]);
            if (m[i].kind == LOFT_M_HASH) {
                if (loft_hash_find(c, m[i].keys, m[i].nk, key, nk).rec == dr.rec) loft_hash_remove(c, dr, m[i].keys, m[i].nk);
            } else if (loft_seq_find(c, 0, m[i].keys, m[i].nk, key, nk).rec == dr.rec) {
                loft_index_remove(c, dr);
            }
        }
    }
}
