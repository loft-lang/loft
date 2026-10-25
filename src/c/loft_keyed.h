/* ── Keyed collections: sorted and index, walks, renders and writes ──────────────────────────
 * The hash is loft's own (loft_hash.h).  The others, in the orders and answers of
 * src/vector.rs (`sorted_*`) and src/btree.rs:
 *
 *   index   the slot names an ARRAY record: @4 the count, from @8 the entries' record numbers in
 *           key order.  An entry is a record of its own, at byte 8.  (A layout of the C's own
 *           until btree.rs is ported.)
 *   sorted  a vector whose elements are kept in key order (`sorted_finish`): its layout IS a
 *           vector's, so a vector's readers read it. */

/* The collection record a keyed slot names, 0 for an empty or absent one or a null holder. */
static uint32_t loft_keyed_rec(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    return loft_collection_rec(loft_store_of(c), c.rec, c.pos);
}

/* A new entry of an index (Stores::record_new): a record of its own in the collection's store,
 * naming at byte 4 the record that holds the collection, filed by `finish` once its fields are
 * written. */
static loft_ref loft_entry_new(loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    loft_ref e;
    e.store_nr = c.store_nr;
    e.rec = loft_claim(loft_store_of(c), 1 + (size + 7) / 8);
    e.pos = 8;
    loft_set_u32(loft_store_of(c), e.rec, 4, c.rec);
    return e;
}

/* OpClearKeyed on a view member: the collection holds nothing. */
static void loft_keyed_clear(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return;
    loft_set_u32(loft_store_of(c), c.rec, c.pos, 0);
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
 * `loft_sorted_finish` opens moves it into place — or in slot 0 of an empty sorted.  The vector
 * is made with room for twelve and grows by `resize` to its length plus two. */
static loft_ref loft_sorted_new(loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    loft_store_t *s = loft_store_of(c);
    uint32_t r = loft_collection_rec(s, c.rec, c.pos);
    loft_ref e = { c.store_nr, 0, 8 };
    if (r == 0) {
        r = loft_st_claim(s, loft_vec_cap_words(12, size));
        loft_set_u32(s, c.rec, c.pos, r);
        loft_set_u32(s, r, 4, 0);
        e.rec = r;
        return e;
    }
    uint32_t len = loft_u32(s, r, 4);
    uint32_t nr = loft_st_resize(s, r, loft_vec_cap_words(len + 2, size));
    if (nr != r) {
        loft_set_u32(s, c.rec, c.pos, nr);
        r = nr;
    }
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

/* vector::ordered_finish (@FR-Col-Insert): the record's number goes to its key's place in the
 * ordered; a record the key already named is replaced and its number answered, 0 when the key
 * was new. */
static uint32_t loft_ordered_finish(loft_ref c, loft_ref e, const loft_key *keys, int nk) {
    if (c.store_nr == 0xFFFF || c.rec == 0 || e.rec == 0) return 0;
    loft_ref slot = loft_sorted_new(c, 4);
    loft_seq q = loft_seq_of(c, 0);
    uint32_t len = loft_seq_len(&q);
    if (len == 0) {
        loft_set_u32(q.s, q.vrec, 4, 1);
        loft_set_u32(q.s, q.vrec, slot.pos, e.rec);
        return 0;
    }
    int found;
    uint32_t p = loft_seq_search(&q, len, 1, 0, 0, e.rec, 8, keys, nk, &found);
    if (found) {
        uint32_t old = loft_u32(q.s, q.vrec, 8 + p * 4);
        loft_set_u32(q.s, q.vrec, 8 + p * 4, e.rec);
        return old == e.rec ? 0 : old;
    }
    uint8_t *base = loft_at(q.s, q.vrec, 0);
    if (p < len) memmove(base + 12 + p * 4, base + 8 + p * 4, (size_t) (len - p) * 4);
    loft_set_u32(q.s, q.vrec, 8 + p * 4, e.rec);
    loft_set_u32(q.s, q.vrec, 4, len + 1);
    return 0;
}

/* Stores::insert_record on an ordered: the record filed; the record its key displaced is
 * answered, released already unless `secondary` (a group member only unlinks). */
static loft_ref loft_ordered_insert(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, const loft_key *keys,
                                    int nk, int secondary) {
    uint32_t gone = loft_ordered_finish(c, e, keys, nk);
    if (gone == 0) return loft_ref_null();
    loft_ref old = { c.store_nr, gone, 8 };
    if (!secondary) {
        loft_remove_claims(T, elem, old);
        loft_st_delete(loft_store_of(c), gone);
    }
    return old;
}

/* The record `e` leaves an ordered or an array: its slot, found by its record number, is
 * removed (Stores::unlink, by reference). */
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

/* Stores::remove_owned on an ordered: unlinked, then its claims and its record released; on an
 * array (`is_array`, whose removal reads `e` as given) the claims and then the unlink.  A `view`
 * member only unlinks. */
static void loft_ordered_remove(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, int is_array, int view) {
    if (e.rec == 0 || c.store_nr == 0xFFFF || c.rec == 0) return;
    if (view) {
        loft_index_remove(c, e);
        return;
    }
    if (is_array) {
        loft_remove_claims(T, elem, e);
        loft_index_remove(c, e);
        return;
    }
    loft_index_remove(c, e);
    loft_ref rec = { e.store_nr, e.rec, 8 };
    loft_remove_claims(T, elem, rec);
    loft_st_delete(loft_store_of(c), e.rec);
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

/* OpIterate over a walk's snapshot (on 4, loft_ref_scratch): every pair. */
static int64_t loft_scratch_iterate(loft_ref x) {
    if (x.store_nr == 0xFFFF || x.rec == 0) return 0;
    loft_store_t *s = loft_store_of(x);
    return loft_iter_range(0, loft_u32(s, loft_u32(s, x.rec, 4), 4));
}

/* OpStep over a snapshot (vector::step_ordered): the next entry still in the collection, in
 * the entries' store. */
static loft_ref loft_scratch_step(int64_t *state, loft_ref x, int reverse) {
    if (x.store_nr == 0xFFFF || x.rec == 0) return loft_ref_null();
    for (;;) {
        int64_t at = loft_iter_next(state, reverse);
        if (at < 0) return loft_ref_null();
        loft_store_t *s = loft_store_of(x);
        uint32_t vec = loft_u32(s, x.rec, 4);
        uint32_t r = loft_u32(s, vec, 8 + (uint32_t) at * 8);
        if (r == 0) continue;
        loft_ref e;
        e.store_nr = (uint16_t) loft_u32(s, x.rec, 8);
        e.rec = r;
        e.pos = loft_u32(s, vec, 12 + (uint32_t) at * 8);
        return e;
    }
}

/* ── copies, renders and equality over the entries ── */

/* The elements of a hash (kind 1, in key order), an index (2, in key order), or an ordered or an
 * array (0, in their own), as references; `*n` their count. */
static loft_ref *loft_keyed_refs(loft_ref c, int kind, const loft_key *keys, int nk, uint32_t *n) {
    *n = 0;
    uint32_t *recs = 0;
    uint32_t count = 0;
    if (kind == 1) return loft_hash_refs(c, keys, nk, 0, n);
    if (kind == 2) {
        recs = c.store_nr == 0xFFFF || c.rec == 0 ? (uint32_t *) calloc(1, 4) : loft_bt_records(c, &count);
        if (!recs) { fputs("loft: out of memory\n", stderr); abort(); }
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

static void loft_show_keyed(loft_text *s, loft_ref r, int fmt, int ind, int kind, const loft_key *keys,
                            int nk, int complex_elem, int struct_elem, loft_show_fn f) {
    if (loft_vec_absent(r)) { loft_push(s, "null", 4); return; }
    uint32_t n;
    loft_ref *e = loft_keyed_refs(r, kind, keys, nk, &n);
    loft_show_refs(s, e, n, fmt, ind, complex_elem, struct_elem, f);
    free(e);
}

static int loft_eq_keyed(loft_ref a, loft_ref b, int kind, const loft_key *keys, int nk, loft_eq_fn f) {
    uint32_t na, nb;
    loft_ref *ea = loft_keyed_refs(a, kind, keys, nk, &na);
    loft_ref *eb = loft_keyed_refs(b, kind, keys, nk, &nb);
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
    if (free_source && src.store_nr != dest.store_nr && !loft_free_protected(src)) loft_free_ref(src);
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
#define LOFT_KEYED_ORDERED 3

/* The element of collection `c` (of kind `kind`) whose key parts equal the `n` given. */
static loft_ref loft_keyed_find(loft_ref c, int kind, uint32_t size, const loft_key *keys, int nk,
                                const loft_kv *kv, int n) {
    if (kind == LOFT_KEYED_HASH) return loft_h_find(c, keys, kv, n);
    if (kind == LOFT_KEYED_INDEX) return loft_ix_find(c, keys, nk, kv, n);
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
        if (kind == LOFT_KEYED_HASH) loft_hash_remove(T, etp, c, existing, keys, nk, 0);
        else if (kind == LOFT_KEYED_SORTED) loft_sorted_remove(c, existing, size);
        else if (kind == LOFT_KEYED_INDEX) loft_ix_remove(T, etp, c, existing, keys, nk, 0);
        else loft_ordered_remove(T, etp, c, existing, 0, 0);
    }
    loft_ref e = kind == LOFT_KEYED_SORTED ? loft_sorted_new(c, size)
                 : kind == LOFT_KEYED_HASH ? loft_hash_entry(c, size, T[etp].linked)
                                           : loft_entry_new(c, size);
    loft_copy_bytes(e, 0, source, 0, size);
    loft_copy_claims(T, etp, source, e);
    if (full) for (int i = 0; i < n; i++) loft_key_set(e, &keys[i], key[i]);
    if (kind == LOFT_KEYED_HASH) (void) loft_hash_insert(T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SORTED) loft_sorted_finish(c, size, keys, nk);
    else if (kind == LOFT_KEYED_INDEX) (void) loft_ix_insert(T, etp, c, e, keys, nk, 0);
    else (void) loft_ordered_insert(T, etp, c, e, keys, nk, 0);
    for (int i = 0; i < n; i++) loft_text_free(&own[i]);
    if (tmp.store_nr != 0xFFFF) loft_free_ref(tmp);
    if (free_source && value.store_nr != c.store_nr && !loft_free_protected(value)) loft_free_ref(value);
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

/* The position a walk's removal names: the element it visited last (loft_seq_remove's). */
static int loft_walk_removed_at(int64_t state, int reverse, uint32_t *at) {
    uint64_t st = (uint64_t) state;
    uint32_t lo = (uint32_t) st, hi = (uint32_t) (st >> 32);
    if (!reverse && lo == 0) return 0;
    *at = reverse ? hi : lo - 1;
    return 1;
}

/* OpRemove in a walk over a vector (State::remove, Stores::remove_vector_at): the element's
 * claims released, then its successors shift down; forward, the index steps back to visit the
 * one that moved into its place. */
static void loft_vec_walk_remove(const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref v, int reverse) {
    loft_ref e = loft_vec_get(v, T[elem].size, *state);
    if (e.rec != 0) loft_remove_claims(T, elem, e);
    (void) loft_vec_remove(v, T[elem].size, *state);
    if (!reverse) *state = *state - 1;
}

/* OpRemove in a walk over a sorted (Stores::remove_vector_at): the element's claims released,
 * then the element removed and the walk moved on. */
static void loft_sorted_walk_remove(const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref c, int reverse) {
    uint32_t at, r = loft_keyed_rec(c), size = T[elem].size;
    if (r == 0 || !loft_walk_removed_at(*state, reverse, &at) || at >= loft_u32(loft_store_of(c), r, 4)) return;
    loft_ref e = { c.store_nr, r, 8 + at * size };
    loft_remove_claims(T, elem, e);
    loft_seq_remove(state, c, size, reverse);
}

/* Stores::remove_owned on a sorted: the element's claims released, then the element removed. */
static void loft_sorted_remove_owned(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, uint32_t size) {
    if (e.rec == 0) return;
    loft_remove_claims(T, elem, e);
    loft_sorted_remove(c, e, size);
}

/* OpRemove in a walk over an ordered (Stores::remove_vector_at, a linked element): the slot the
 * walk visited last is unlinked, then its record's claims and the record released. */
static void loft_ordered_walk_remove(const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref c, int reverse) {
    uint32_t at, r = loft_keyed_rec(c);
    if (r == 0 || !loft_walk_removed_at(*state, reverse, &at) || at >= loft_u32(loft_store_of(c), r, 4)) return;
    uint32_t rec = loft_u32(loft_store_of(c), r, 8 + at * 4);
    loft_seq_remove(state, c, 0, reverse);
    if (rec != 0) {
        loft_ref e = { c.store_nr, rec, 8 };
        loft_remove_claims(T, elem, e);
        loft_st_delete(loft_store_of(c), rec);
    }
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
#define LOFT_M_ORDERED 3

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
        else if (m[i].kind == LOFT_M_HASH) d = loft_hash_insert(0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        else if (m[i].kind == LOFT_M_INDEX) d = loft_ix_insert(0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        else d = loft_ordered_insert(0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
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
                if (loft_h_find(c, m[i].keys, key, nk).rec == dr.rec) (void) loft_hash_unlink(c, dr, m[i].keys, m[i].nk);
            } else if (m[i].kind == LOFT_M_INDEX) {
                if (loft_ix_find(c, m[i].keys, m[i].nk, key, nk).rec == dr.rec) (void) loft_bt_remove(c, dr.rec, m[i].keys, m[i].nk);
            } else if (loft_seq_find(c, 0, m[i].keys, m[i].nk, key, nk).rec == dr.rec) {
                loft_index_remove(c, dr);
            }
        }
    }
}

/* ── par (parallel_natives.rs): the rows in order ──
 * A `for x in v par(r = f(x), n)` loop hands every row to the worker and reads the results back
 * by row.  The C runs the worker over the rows one after another — each row's result is the
 * one a thread would compute — into a buffer on a stack the loop reads (`parallel_buf_get*`)
 * and pops (`parallel_buf_drop*`). */
typedef struct { int64_t i; double f; float g; loft_text t; loft_ref r; loft_fn fn; } loft_par_slot;
typedef struct { loft_par_slot *rows; int64_t n; } loft_par_buf;
static loft_par_buf *loft_par_bufs = 0;
static int loft_npar = 0, loft_par_cap = 0;

static loft_par_slot *loft_par_push(int64_t n) {
    if (loft_npar == loft_par_cap) {
        loft_par_cap = loft_par_cap ? loft_par_cap * 2 : 8;
        loft_par_bufs = (loft_par_buf *) realloc(loft_par_bufs, (size_t) loft_par_cap * sizeof *loft_par_bufs);
        if (!loft_par_bufs) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_par_slot *rows = (loft_par_slot *) calloc((size_t) (n > 0 ? n : 1), sizeof *rows);
    if (!rows) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) {
        rows[i].i = LOFT_NULL;
        rows[i].r = loft_ref_null();
        rows[i].fn = loft_fn_of(LOFT_NULL);
    }
    loft_par_bufs[loft_npar].rows = rows;
    loft_par_bufs[loft_npar].n = n;
    loft_npar++;
    return rows;
}

static loft_par_slot *loft_par_row(int64_t i) {
    if (loft_npar == 0 || i < 0 || i >= loft_par_bufs[loft_npar - 1].n) return 0;
    return &loft_par_bufs[loft_npar - 1].rows[i];
}

static int64_t loft_par_get(int64_t i) { loft_par_slot *s = loft_par_row(i); return s ? s->i : LOFT_NULL; }
static double loft_par_get_float(int64_t i) { loft_par_slot *s = loft_par_row(i); return s ? s->f : (double) NAN; }
static float loft_par_get_single(int64_t i) { loft_par_slot *s = loft_par_row(i); return s ? s->g : (float) NAN; }
static loft_ref loft_par_get_ref(int64_t i) { loft_par_slot *s = loft_par_row(i); return s ? s->r : loft_ref_null(); }
static loft_fn loft_par_get_fn(int64_t i) { loft_par_slot *s = loft_par_row(i); return s ? s->fn : loft_fn_of(LOFT_NULL); }
static loft_text loft_par_get_text(int64_t i) {
    loft_par_slot *s = loft_par_row(i);
    return loft_text_own(s ? loft_view(&s->t) : loft_null_text());
}

/* The buffer leaves the stack; `refs` releases the stores the workers' results were made in. */
static void loft_par_drop(int refs) {
    if (loft_npar == 0) return;
    loft_par_buf *b = &loft_par_bufs[--loft_npar];
    for (int64_t i = 0; i < b->n; i++) {
        loft_text_free(&b->rows[i].t);
        if (refs) loft_free_ref(b->rows[i].r);
    }
    free(b->rows);
}

/* parallel_buf_get_narrow: a 1-, 2- or 4-byte result, its low bytes extended as `signed` says. */
static int64_t loft_par_get_narrow(int64_t i, int64_t size, int64_t is_signed) {
    loft_par_slot *s = loft_par_row(i);
    if (!s) return LOFT_NULL;
    uint64_t u = (uint64_t) s->i;
    int bits = (int) (size * 8);
    if (bits <= 0 || bits >= 64) return s->i;
    u &= (UINT64_C(1) << bits) - 1;
    if (is_signed && (u >> (bits - 1))) return (int64_t) (u | ~((UINT64_C(1) << bits) - 1)) ;
    return (int64_t) u;
}

/* read_primitive_at: a row handed to a worker by value — its stored bits, 1, 4 or 8 bytes by the
 * element's width. */
static uint64_t loft_par_arg(loft_ref e, int64_t size) {
    if (e.rec == 0) return 0;
    uint8_t *p = loft_at(loft_store_of(e), e.rec, e.pos);
    if (size == 1) return p[0];
    if (size == 4) {
        uint32_t x;
        memcpy(&x, p, 4);
        return x;
    }
    uint64_t x;
    memcpy(&x, p, 8);
    return x;
}

static int64_t loft_i64_bits(uint64_t u) {
    int64_t v;
    memcpy(&v, &u, 8);
    return v;
}
