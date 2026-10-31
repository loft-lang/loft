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

#define LOFT_M_ARRAY 0
#define LOFT_M_HASH 1
#define LOFT_M_INDEX 2
#define LOFT_M_ORDERED 3
#define LOFT_M_SPATIAL 4
#define LOFT_M_TRIE 5

/* One member of a linked group: its field's byte position, its kind (an index and an ordered
 * share LOFT_M_INDEX) and its key parts. */
typedef struct { uint32_t pos; int kind; const loft_key *keys; int nk; } loft_member;

/* A linked group a keyed write files into (`@FR-Col-Group`): the struct holding it, its members,
 * the record type, whether a vector member holds the records and whether they are `vector<S?>`
 * elements (`loft_group_finish`). */
typedef struct { loft_ref parent; const loft_member *m; int n; int held; int nullable; } loft_group;
static void loft_group_finish(const loft_tdesc *T, uint16_t elem, int held, int nullable, loft_ref parent,
                              loft_ref rec, const loft_member *m, int n);

/* A new entry of an index (Stores::record_new): a record of its own in the collection's store,
 * naming at byte 4 the record that holds the collection, filed by `finish` once its fields are
 * written. */
LOFT_HOT_INLINE loft_ref loft_entry_new(loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    /* A locked collection gains no entry: the null record, whose writes are skipped. */
    if (LOFT_WRITE_REFUSED(loft_store_of(c))) return loft_ref_null();
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
 * is made with the growth step's room for two (the element and the scratch slot) and takes the
 * growth step again when its length plus two no longer fits (`@FR-Col-Capacity`). */
static loft_ref loft_sorted_new(loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    if (LOFT_WRITE_REFUSED(loft_store_of(c))) return loft_ref_null();
    loft_store_t *s = loft_store_of(c);
    uint32_t r = loft_collection_rec(s, c.rec, c.pos);
    loft_ref e = { c.store_nr, 0, 8 };
    if (r == 0) {
        r = loft_st_claim(s, loft_vec_cap_words(loft_vec_grown(2, size), size));
        loft_set_u32(s, c.rec, c.pos, r);
        loft_set_u32(s, r, 4, 0);
        e.rec = r;
        return e;
    }
    uint32_t len = loft_u32(s, r, 4);
    /* The element and the scratch slot: the growth step only when they do not fit. */
    int32_t claimed;
    memcpy(&claimed, loft_at(s, r, 0), 4);
    uint32_t nr = len + 2 > ((uint32_t) claimed * 8 - 8) / size
        ? loft_st_resize(s, r, loft_vec_cap_words(loft_vec_grown(len + 2, size), size)) : r;
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
    if (LOFT_WRITE_REFUSED(loft_store_of(c))) return;
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
    if (LOFT_WRITE_REFUSED(loft_store_of(c))) return;
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

/* ── copies, renders and equality over the entries ── */

/* The elements of a hash (kind 1, in key order), an index (2, in key order), a spatial or a trie
 * (3, in tree order), or an ordered or an array (0, in their own), as references; `*n` their
 * count. */
static loft_ref *loft_keyed_refs(loft_ref c, int kind, const loft_key *keys, int nk, uint32_t *n) {
    *n = 0;
    uint32_t *recs = 0;
    uint32_t count = 0;
    if (kind == 1) return loft_hash_refs(c, keys, nk, 0, n);
    if (kind == 2 || kind == 3) {
        recs = c.store_nr == 0xFFFF || c.rec == 0 ? (uint32_t *) calloc(1, 4)
               : kind == 2                         ? loft_bt_records(c, &count)
                                                   : loft_rx_records(c, &count);
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
    /* Stores::eq_absent_collections: an absent collection equals an absent one only. */
    int aa = loft_vec_absent(a), ab = loft_vec_absent(b);
    if (aa || ab) return aa && ab;
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
 * else is cleared and refilled with deep copies; a source the bind gives up is freed.  Bit 0 of
 * `flags`: the source is given up; bit 1: the element tree holds no stored reference, so such a
 * source's whole store may move into the destination's slot (`@FR-H-SwapIn`'s keyed clause,
 * `Stores::try_swap_keyed`) — the old collection leaves with the released slot, where the
 * copy rebuilt every element and bucket. */
static void loft_replace_keyed(const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dest, int flags) {
    int free_source = flags & 1;
    if (src.store_nr == 0xFFFF) {
        loft_remove_claims(T, tp, dest);
        if (dest.store_nr != 0xFFFF && dest.rec != 0 && dest.pos != 0)
            loft_set_u32(loft_store_of(dest), dest.rec, dest.pos, 0xFFFFFFFFu);
        return;
    }
    if (src.store_nr == dest.store_nr && src.rec == dest.rec && src.pos == dest.pos) return;
    if (dest.store_nr == 0xFFFF || dest.rec == 0) return;
    if (free_source && (flags & 2) && loft_swap_in(tp, src, dest, 0)) return;
    loft_remove_claims(T, tp, dest);
    loft_copy_claims(T, tp, src, dest);
#ifdef LOFT_COPY_CHECK
    loft_report_copy_mismatches(T, tp, src, dest, "OpReplaceKeyed");
#endif
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
#define LOFT_KEYED_SPATIAL 4
#define LOFT_KEYED_TRIE 5

/* The element of collection `c` (of kind `kind`) whose key parts equal the `n` given. */
static loft_ref loft_keyed_find(loft_ref c, int kind, uint32_t size, const loft_key *keys, int nk,
                                const loft_kv *kv, int n) {
    if (kind == LOFT_KEYED_HASH) return loft_h_find(c, keys, kv, n);
    if (kind == LOFT_KEYED_INDEX) return loft_ix_find(c, keys, nk, kv, n);
    if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE) return loft_rx_find(c, keys, nk, kind == LOFT_KEYED_TRIE, kv, n);
    return loft_seq_find(c, kind == LOFT_KEYED_SORTED ? size : 0, keys, nk, kv, n);
}

/* OpSetKeyed (Stores::set_keyed, @FR-Col-Assign): `c[k…] = v` — a deep copy of `v` filed under
 * the subscript's key (or, with no full subscript, under its own), replacing the element that
 * key named; a value the bind gives up is freed. */
static void loft_set_keyed(const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref value, int kind,
                           const loft_key *keys, int nk, const loft_kv *sub, int nsub, int free_source,
                           const loft_group *g) {
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
        else if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE)
            loft_rx_remove_owned(T, etp, c, existing, keys, nk, kind == LOFT_KEYED_TRIE, 0);
        else loft_ordered_remove(T, etp, c, existing, 0, 0);
    }
    loft_ref e = kind == LOFT_KEYED_SORTED ? loft_sorted_new(c, size)
                 : kind == LOFT_KEYED_HASH ? loft_hash_entry(c, size, T[etp].linked)
                                           : loft_entry_new(c, size);
    loft_copy_bytes(e, 0, source, 0, size);
    loft_copy_claims(T, etp, source, e);
    if (full) for (int i = 0; i < n; i++) loft_key_set(e, &keys[i], key[i]);
    /* A member of a linked group: the record enters every member (`record_finish`). */
    if (g) loft_group_finish(T, etp, g->held, g->nullable, g->parent, e, g->m, g->n);
    else if (kind == LOFT_KEYED_HASH) (void) loft_hash_insert(T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SORTED) loft_sorted_finish(c, size, keys, nk);
    else if (kind == LOFT_KEYED_INDEX) (void) loft_ix_insert(T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE)
        (void) loft_rx_insert_record(T, etp, c, e, keys, nk, kind == LOFT_KEYED_TRIE, 0);
    else (void) loft_ordered_insert(T, etp, c, e, keys, nk, 0);
    for (int i = 0; i < n; i++) loft_text_free(&own[i]);
    if (tmp.store_nr != 0xFFFF) loft_free_ref(tmp);
    if (free_source && value.store_nr != c.store_nr && !loft_free_protected(value)) loft_free_ref(value);
}

/* `@FR-R-InPlaceLiteral`'s keyed clause, the first half of `c[k…] = R { … }` written in place
 * (Stores::keyed_place_begin): the element the FULL subscript names removed, as `loft_set_keyed`
 * removes it, and a fresh one claimed in the collection's store, not yet filed — the literal's
 * fields are written into it before `loft_keyed_place_finish` files it. */
static loft_ref loft_keyed_place_begin(const loft_tdesc *T, uint16_t etp, loft_ref c, int kind, const loft_key *keys,
                                       int nk, const loft_kv *sub, int nsub) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    uint32_t size = T[etp].size;
    loft_ref existing = loft_keyed_find(c, kind, size, keys, nk, sub, nsub);
    if (existing.rec != 0) {
        if (kind == LOFT_KEYED_HASH) loft_hash_remove(T, etp, c, existing, keys, nk, 0);
        else if (kind == LOFT_KEYED_SORTED) loft_sorted_remove(c, existing, size);
        else if (kind == LOFT_KEYED_INDEX) loft_ix_remove(T, etp, c, existing, keys, nk, 0);
        else if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE)
            loft_rx_remove_owned(T, etp, c, existing, keys, nk, kind == LOFT_KEYED_TRIE, 0);
        else loft_ordered_remove(T, etp, c, existing, 0, 0);
    }
    return kind == LOFT_KEYED_SORTED ? loft_sorted_new(c, size)
           : kind == LOFT_KEYED_HASH ? loft_hash_entry(c, size, T[etp].linked)
                                     : loft_entry_new(c, size);
}

/* The second half (Stores::keyed_place_finish): the subscript's key written into the claimed
 * element and the element filed, as `loft_set_keyed` files its copy. */
static void loft_keyed_place_finish(const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref e, int kind,
                                    const loft_key *keys, int nk, const loft_kv *sub, int nsub) {
    if (e.rec == 0) return;
    uint32_t size = T[etp].size;
    for (int i = 0; i < nsub; i++) loft_key_set(e, &keys[i], sub[i]);
    if (kind == LOFT_KEYED_HASH) (void) loft_hash_insert(T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SORTED) loft_sorted_finish(c, size, keys, nk);
    else if (kind == LOFT_KEYED_INDEX) (void) loft_ix_insert(T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE)
        (void) loft_rx_insert_record(T, etp, c, e, keys, nk, kind == LOFT_KEYED_TRIE, 0);
    else (void) loft_ordered_insert(T, etp, c, e, keys, nk, 0);
}

/* `@FR-R-KeyedEntry`'s subscript clause (Stores::keyed_entry_ints): the entry `c[ks…] = T{…}`
 * writes its non-key fields into — found or filed in one probe (`loft_h_entry_ints`), its key
 * parts written.  Where that walk cannot serve (a walk over `c` is live, no table yet) the entry
 * is claimed, keyed and filed through `loft_keyed_place_begin` / `_finish` first, and the filed
 * place answered: an inline table moves the staged entry into itself. */
static loft_ref loft_keyed_entry_ints(const loft_tdesc *T, uint16_t etp, loft_ref c, const loft_key *keys, int nk,
                                      const int64_t *ks) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    /* A locked collection files nothing: the entry is the one already there, or the null record
     * whose writes are skipped (`@FR-H-WriteLocked`). */
    if (LOFT_WRITE_REFUSED(loft_store_of(c))) return loft_h_find_ints(c, keys, nk, ks);
    int live = 0;
    for (int i = 0; i < loft_nscratches && !live; i++)
        live = loft_scratches[i].source.store_nr == c.store_nr && loft_scratches[i].source.rec == c.rec &&
               loft_scratches[i].source.pos == c.pos;
    if (!live) {
        loft_ref e = loft_h_entry_ints(c, keys, nk, ks);
#ifdef LOFT_KEYED_VERIFY
        /* `keyed_entry_long`'s check: the entry the one probe answers is the one the key finds. */
        if (e.rec != 0) {
            loft_kv vk[4];
            for (int i = 0; i < nk && i < 4; i++) vk[i] = loft_kv_int(ks[i]);
            loft_ref found = loft_h_find(c, keys, vk, nk < 4 ? nk : 4);
            if (found.rec != e.rec || found.pos != e.pos) {
                fflush(stdout);
                fprintf(stderr, "LOFT_KEYED_VERIFY: the key answers %u+%u where the entry is %u+%u\n",
                        (unsigned) found.rec, (unsigned) found.pos, (unsigned) e.rec, (unsigned) e.pos);
                abort();
            }
        }
#endif
        if (e.rec != 0) return e;
    }
    loft_kv kv[4];
    for (int i = 0; i < nk && i < 4; i++) kv[i] = loft_kv_int(ks[i]);
    loft_ref e = loft_keyed_place_begin(T, etp, c, LOFT_KEYED_HASH, keys, nk, kv, nk);
    if (e.rec == 0) return e;
    loft_st_fresh(loft_store_of(e), e.rec, e.pos, T[etp].size);
    loft_keyed_place_finish(T, etp, c, e, LOFT_KEYED_HASH, keys, nk, kv, nk);
    return loft_h_find_ints(c, keys, nk, ks);
}

/* `@FR-R-KeyedEntry`'s text clause (Stores::keyed_entry_text): `loft_keyed_entry_ints` for a hash
 * keyed by one text — the entry under `key`, found or filed with the key written, in one probe
 * where the table allows (`loft_h_entry_text`), else claimed, keyed and filed first. */
static loft_ref loft_keyed_entry_text(const loft_tdesc *T, uint16_t etp, loft_ref c, const loft_key *keys, int nk,
                                      loft_str key) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    if (LOFT_WRITE_REFUSED(loft_store_of(c))) return loft_h_find_text(c, keys, key);
    int live = 0;
    for (int i = 0; i < loft_nscratches && !live; i++)
        live = loft_scratches[i].source.store_nr == c.store_nr && loft_scratches[i].source.rec == c.rec &&
               loft_scratches[i].source.pos == c.pos;
    if (!live) {
        loft_ref e = loft_h_entry_text(c, keys, key);
#ifdef LOFT_KEYED_VERIFY
        if (e.rec != 0) {
            loft_kv vk = loft_kv_text(key);
            loft_ref found = loft_h_find(c, keys, &vk, 1);
            if (found.rec != e.rec || found.pos != e.pos) {
                fflush(stdout);
                fprintf(stderr, "LOFT_KEYED_VERIFY: the key answers %u+%u where the entry is %u+%u\n",
                        (unsigned) found.rec, (unsigned) found.pos, (unsigned) e.rec, (unsigned) e.pos);
                abort();
            }
        }
#endif
        if (e.rec != 0) return e;
    }
    loft_text own = loft_text_own(key);
    loft_kv kv = loft_kv_text(loft_view(&own));
    loft_ref e = loft_keyed_place_begin(T, etp, c, LOFT_KEYED_HASH, keys, nk, &kv, 1);
    if (e.rec != 0) {
        loft_st_fresh(loft_store_of(e), e.rec, e.pos, T[etp].size);
        loft_keyed_place_finish(T, etp, c, e, LOFT_KEYED_HASH, keys, nk, &kv, 1);
        e = loft_h_find_text(c, keys, loft_view(&own));
    }
    loft_text_free(&own);
    return e;
}

/* `@FR-R-HandOver`'s rotate clause (Stores::rotate_into): the append of a copy of `src` to the
 * full vector `vec` (`k` elements of `size` bytes) followed by the drop of its oldest element,
 * as one step — `src` copied into element 0 (its bytes, and each of its `count` plain vectors,
 * `plan` holding (offset, element bytes) pairs, into the block element 0 owns, grown where too
 * small), then the slots rotated left by one.  The state the append and the keep of [1, k + 1)
 * leave, with no slot claimed and no record freed.  0, with nothing changed, unless the vector
 * holds exactly `k >= 1` elements, no block element 0 owns is named by a stored reference
 * (`@FR-H-Removed`), neither record is foreign, the store is writable, and `src` lies outside
 * the vector. */
static int loft_rotate_into(loft_ref vec, loft_ref src, uint32_t size, int64_t k, const uint32_t *plan, int count) {
    if (k < 1 || size > 64 || count > 8 || vec.store_nr == 0xFFFF || vec.rec == 0 || src.store_nr == 0xFFFF ||
        src.rec == 0 || src.rec >= loft_store_of(src)->size)
        return 0;
    loft_store_t *vs = loft_store_of(vec);
    if (vs->read_only) return 0;
    uint32_t vrec = loft_collection_rec(vs, vec.rec, vec.pos);
    if (vrec == 0 || vrec >= vs->size || (int64_t) loft_u32(vs, vrec, 4) != k) return 0;
    if (src.store_nr == vec.store_nr && src.rec == vrec) return 0;
    uint32_t held[8] = { 0 };
    for (int i = 0; i < count; i++) {
        uint32_t cur = loft_u32(vs, vrec, 8 + plan[2 * i]);
        if (cur == 0 || cur == 0xFFFFFFFFu) continue;
        if (cur >= vs->size || loft_bits_contains(&vs->referenced, cur)) return 0;
        held[i] = cur;
    }
    memmove(loft_at(vs, vrec, 8), loft_at(loft_store_of(src), src.rec, src.pos), size);
    for (int i = 0; i < count; i++) {
        uint32_t off = plan[2 * i], esize = plan[2 * i + 1];
        loft_store_t *ss = loft_store_of(src);
        uint32_t from = loft_u32(ss, src.rec, src.pos + off), b = held[i];
        if (from == 0 || from == 0xFFFFFFFFu) {
            if (b != 0) loft_st_delete(vs, b);
            loft_set_u32(vs, vrec, 8 + off, from);
            continue;
        }
        uint32_t length = loft_u32(ss, from, 4);
        uint32_t words = 1 + (uint32_t) (((uint64_t) esize * length + 7) / 8);
        if (b == 0) b = loft_st_claim(vs, words);
        else if (loft_rec_words(vs, b) < words) b = loft_st_resize(vs, b, words);
        loft_set_u32(vs, vrec, 8 + off, b);
        ss = loft_store_of(src);
        memmove(loft_at(vs, b, 4), loft_at(ss, from, 4), (size_t) length * esize + 4);
    }
    /* A claim or a resize above may have moved the store: the base is read after them. */
    uint8_t *base = loft_at(vs, vrec, 8), tmp[64];
    memcpy(tmp, base, size);
    memmove(base, base + size, (size_t) (k - 1) * size);
    memcpy(base + (size_t) (k - 1) * size, tmp, size);
    return 1;
}

/* `@FR-R-Apart` instance 2 (codegen_runtime::ap_get_f64 / ap_set_f64 / ap_store_f64): a record
 * of fixed-length `vector<float>` fields held as a C struct, one `double[N]` per field, while
 * it travels between the functions that build it, read and write its elements and hand it on.
 * An element answers what the C's store path answers: an index from the end, and an index out
 * of range read as null (NaN) and written as nothing. */
static inline double loft_ap_get(const double *a, int64_t n, int64_t i) {
    int64_t f = i < 0 ? i + n : i;
    return f < 0 || f >= n ? (double) NAN : a[f];
}

static inline void loft_ap_set(double *a, int64_t n, int64_t i, double x) {
    int64_t f = i < 0 ? i + n : i;
    if (f >= 0 && f < n) a[f] = x;
}

/* The BOUNDARY: the value written into the return buffer `buf` with the steps its constructor
 * runs — a buffer that holds no record minted (`bytes`, type `tp`), then each `vector<float>`
 * field at `offs[k]` emptied in place and its `lens[k]` elements appended.  Answers the buffer,
 * which is what the unrewritten function returns. */
static loft_ref loft_ap_store(loft_ref buf, uint32_t bytes, uint16_t tp, int nf, const uint32_t *offs,
                              const double *const *vals, const uint32_t *lens) {
    if (buf.store_nr == 0xFFFF || buf.rec == 0) {
        buf = loft_ref_null();
        loft_database_typed(&buf, bytes, tp);
        for (int k = 0; k < nf; k++) loft_set_u32(loft_store_of(buf), buf.rec, buf.pos + offs[k], 0);
    }
    for (int k = 0; k < nf; k++) {
        loft_ref fld = loft_field(buf, offs[k]);
        loft_vec_clear(fld);
        loft_vec_prealloc(fld, lens[k], 8);
        for (uint32_t i = 0; i < lens[k]; i++) loft_vec_push_float(fld, vals[k][i]);
    }
    return buf;
}

/* `@FR-R-Apart` instance 3 (codegen_runtime::ApVec): a `vector<float>` local that never leaves
 * its function — its first 64 elements inline, the rest in a spill (a text buffer, so the frame's
 * epilogue frees it with its texts).  Each operation answers what the C's store path answers for
 * the vector it stands for: an empty vector at its init, an index from the end, and an index out
 * of range read as null (NaN) and written as nothing. */
#define LOFT_APV_INLINE 64
typedef struct { size_t n; double d[LOFT_APV_INLINE]; loft_text spill; } loft_apvec;

static inline void loft_apv_clear(loft_apvec *v) {
    v->n = 0;
    v->spill.len = 0;
}

static inline void loft_apv_push(loft_apvec *v, double x) {
    if (v->n < LOFT_APV_INLINE) v->d[v->n] = x;
    else loft_push(&v->spill, (const char *) &x, sizeof x);
    v->n++;
}

/* The element `i` names (from the end when negative), or -1 out of range. */
static inline int64_t loft_apv_index(const loft_apvec *v, int64_t i) {
    int64_t f = i < 0 ? i + (int64_t) v->n : i;
    return f < 0 || (uint64_t) f >= (uint64_t) v->n ? -1 : f;
}

static inline double *loft_apv_at(loft_apvec *v, int64_t f) {
    return f < LOFT_APV_INLINE ? &v->d[f] : (double *) (void *) (v->spill.p + (size_t) (f - LOFT_APV_INLINE) * sizeof(double));
}

static inline double loft_apv_get(loft_apvec *v, int64_t i) {
    int64_t f = loft_apv_index(v, i);
    if (f < 0) return (double) NAN;
    double x;
    memcpy(&x, loft_apv_at(v, f), sizeof x);
    return x;
}

static inline void loft_apv_set(loft_apvec *v, int64_t i, double x) {
    int64_t f = loft_apv_index(v, i);
    if (f >= 0) memcpy(loft_apv_at(v, f), &x, sizeof x);
}

/* `v[i]` of a vector held apart (`@FR-E-Report`): an index naming no element is reported. */
static inline double loft_apv_get_at(loft_apvec *v, int64_t i, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(loft_apv_index(v, i) < 0)) loft_index_miss(i, (int64_t) v->n, file, line);
    return loft_apv_get(v, i);
}
static inline void loft_apv_set_at(loft_apvec *v, int64_t i, double x, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(loft_apv_index(v, i) < 0)) loft_index_miss(i, (int64_t) v->n, file, line);
    loft_apv_set(v, i, x);
}

/* `@FR-R-Apart` instance 4 (codegen_runtime::AprVec): one field of a record of record vectors
 * held apart is the bytes of its elements, `esize` each, in the element's store layout — a text
 * buffer, so the frame's epilogue frees it with its texts.  Clearing keeps the capacity: a value
 * rebuilt every iteration allocates nothing once it has grown.  A pushed element starts as zeros
 * only so that its padding is defined; the plan admits a push only when every field is written.
 * Answers the element's byte offset. */
static size_t loft_apr_push(loft_text *v, uint32_t esize) {
    size_t at = v->len, need = at + esize;
    if (need > v->cap) {
        size_t c = v->cap ? v->cap * 2 : 256;
        while (c < need) c *= 2;
        char *p = (char *) realloc(v->p, c);
        if (!p) { fputs("loft: out of memory\n", stderr); abort(); }
        v->p = p;
        v->cap = c;
    }
    memset(v->p + at, 0, esize);
    v->len = need;
    return at;
}

/* Stores::apart_block: a block of `s` holding the `n` elements whose `len` bytes are `bytes` —
 * `reuse` when it is large enough (0 claims one), grown when too small. */
static uint32_t loft_apart_block(loft_store_t *s, uint32_t reuse, const char *bytes, size_t len, uint32_t n) {
    uint32_t words = 1 + (uint32_t) ((len + 7) / 8);
    uint32_t b = reuse == 0 ? loft_st_claim(s, words)
                 : loft_rec_words(s, reuse) < words ? loft_st_resize(s, reuse, words) : reuse;
    loft_set_u32(s, b, 4, n);
    memcpy(loft_at(s, b, 8), bytes, len);
    return b;
}

/* `@FR-R-Apart` instance 4's boundary (Stores::apart_write_fresh): the apart record's `nf` fields
 * written into the FRESH element `dst` — per field a block of exactly its elements claimed in
 * `dst`'s store with the bytes copied in, its number written at the field's slot; an empty field
 * writes the empty vector (0).  A null destination is left as the store path's copy leaves it,
 * and a locked store refuses the write (`@FR-H-WriteLocked`). */
static void loft_apart_write_fresh(loft_ref dst, const loft_text *f, const uint32_t *slot, const uint32_t *esize, int nf) {
    if (dst.store_nr == 0xFFFF || dst.rec == 0 || dst.store_nr >= loft_nstores) return;
    loft_store_t *s = loft_store_of(dst);
    if (LOFT_WRITE_REFUSED(s)) return;
    for (int k = 0; k < nf; k++) {
        uint32_t n = (uint32_t) (f[k].len / esize[k]);
        uint32_t b = n == 0 ? 0 : loft_apart_block(s, 0, f[k].p, f[k].len, n);
        loft_set_u32(s, dst.rec, dst.pos + slot[k], b);
    }
}

/* `@FR-R-HandOver`'s rotate clause from an apart record (Stores::rotate_from_parts): as
 * `loft_rotate_into`, the new element's vectors given as the apart fields rather than read from
 * a record — each written into the block element 0 owns (grown where too small, released when
 * the field is empty), then the slots rotated left by one.  0, with nothing changed, on the
 * tests `loft_rotate_into` makes, or when the fields are not the element type's plain vectors
 * (`plan`, `count` pairs of offset and element bytes). */
static int loft_rotate_from_parts(loft_ref vec, const loft_text *f, const uint32_t *slot, int nf, uint32_t size,
                                  int64_t k, const uint32_t *plan, int count) {
    if (k < 1 || size > 64 || count > 8 || count != nf || vec.store_nr == 0xFFFF || vec.rec == 0) return 0;
    loft_store_t *vs = loft_store_of(vec);
    if (vs->read_only) return 0;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(vs)) return 0;
#endif
    uint32_t vrec = loft_collection_rec(vs, vec.rec, vec.pos);
    if (vrec == 0 || vrec >= vs->size || (int64_t) loft_u32(vs, vrec, 4) != k) return 0;
    for (int i = 0; i < count; i++)
        if (plan[2 * i] != slot[i] || f[i].len % plan[2 * i + 1] != 0) return 0;
    uint32_t held[8] = { 0 };
    for (int i = 0; i < count; i++) {
        uint32_t cur = loft_u32(vs, vrec, 8 + plan[2 * i]);
        if (cur == 0 || cur == 0xFFFFFFFFu) continue;
        if (cur >= vs->size || loft_bits_contains(&vs->referenced, cur)) return 0;
        held[i] = cur;
    }
    for (int i = 0; i < count; i++) {
        uint32_t n = (uint32_t) (f[i].len / plan[2 * i + 1]);
        if (n == 0) {
            if (held[i] != 0) loft_st_delete(vs, held[i]);
            loft_set_u32(vs, vrec, 8 + slot[i], 0);
            continue;
        }
        uint32_t b = loft_apart_block(vs, held[i], f[i].p, f[i].len, n);
        loft_set_u32(vs, vrec, 8 + slot[i], b);
    }
    /* A claim or a resize above may have moved the store: the base is read after them. */
    uint8_t *base = loft_at(vs, vrec, 8), tmp[64];
    memcpy(tmp, base, size);
    memmove(base, base + size, (size_t) (k - 1) * size);
    memcpy(base + (size_t) (k - 1) * size, tmp, size);
    return 1;
}

/* OpFillKeyed (Stores::fill_keyed_from_vector): each element of the vector `src` filed into the
 * keyed collection `c` as a deep copy under its own key (`insert_keyed_copy`, the latest of one
 * key winning); a null linked element has no key and stays out.  Every element is resolved
 * before the first insert, which may grow a store they share.  A source the bind gives up is
 * freed. */
static void loft_fill_keyed(const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref src, int kind,
                            const loft_key *keys, int nk, int linked, int free_source, const loft_group *g) {
    if (src.store_nr == 0xFFFF || src.store_nr >= loft_nstores) return;
    int64_t len = loft_vec_len(src);
    uint32_t width = linked ? 4u : T[etp].size;
    loft_ref *recs = (loft_ref *) malloc(sizeof(loft_ref) * (size_t) (len > 0 ? len : 1));
    if (!recs) { fputs("loft: out of memory\n", stderr); abort(); }
    int64_t n = 0;
    for (int64_t i = 0; i < len; i++) {
        loft_ref slot = loft_vec_get(src, width, i);
        if (slot.rec == 0) continue;
        if (linked) {
            uint32_t rec = loft_u32(loft_store_of(slot), slot.rec, slot.pos);
            if (rec == 0) continue;
            loft_ref r = { src.store_nr, rec, 8 };
            recs[n++] = r;
        } else {
            recs[n++] = slot;
        }
    }
    for (int64_t i = 0; i < n; i++) loft_set_keyed(T, etp, c, recs[i], kind, keys, nk, 0, 0, 0, g);
    free(recs);
    if (free_source && src.store_nr != c.store_nr && !loft_free_protected(src)) loft_free_ref(src);
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

/* OpRefFromChildRec (Stores::ref_from_child_rec): the child record field `f` names, or null. */
static loft_ref loft_ref_from_child_rec(loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return loft_ref_null();
    uint32_t rec = loft_u32(loft_store_of(f), f.rec, f.pos);
    if (rec == 0) return loft_ref_null();
    loft_ref r = { f.store_nr, rec, 8 };
    return r;
}

/* OpChildRec (Stores::new_child_rec): a fresh `tp` child record in field `f`'s store, filled with
 * its defaults and named by the field; a record the field held before is released as this
 * store's bytes.  The claim is `size` words, as the Rust's is. */
static loft_ref loft_new_child_rec(const loft_tdesc *T, uint16_t tp, loft_ref f, const uint8_t *image,
                                   uint32_t n) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return loft_ref_null();
    uint32_t old = loft_u32(loft_store_of(f), f.rec, f.pos);
    if (old != 0) {
        loft_ref o = { f.store_nr, old, 8 };
        loft_remove_claims(T, tp, o);
        loft_st_delete(loft_store_of(f), old);
    }
    uint32_t rec = loft_claim(loft_store_of(f), T[tp].size);
    loft_ref db = { f.store_nr, rec, 8 };
    loft_fill(db, image, n);
    loft_set_u32(loft_store_of(f), f.rec, f.pos, rec);
    return db;
}

/* Stores::vector_add_array: `v += o` of a linked element type, whose slots are record ids — each
 * element record copied into a fresh record of the destination's store, its heap deep-copied, a
 * null slot kept null. */
static void loft_vec_add_array(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
    int64_t n = loft_vec_len(o);
    if (n == 0 || v.store_nr == 0xFFFF || v.rec == 0) return;
    uint32_t size = T[tp].size, words = 1 + (size + 7) / 8;
    uint32_t o_rec = loft_u32(loft_store_of(o), o.rec, o.pos);
    if (o_rec == 0) return;
    for (int64_t i = 0; i < n; i++) {
        uint32_t src = loft_u32(loft_store_of(o), o_rec, 8 + 4 * (uint32_t) i);
        loft_ref slot = loft_vec_append(v, 4);
        if (src == 0) {
            loft_set_u32(loft_store_of(v), slot.rec, slot.pos, 0);
        } else {
            uint32_t nr = loft_claim(loft_store_of(v), words);
            loft_ref from = { o.store_nr, src, 8 }, to = { v.store_nr, nr, 8 };
            loft_copy_bytes(to, 0, from, 0, size);
            loft_copy_claims(T, tp, from, to);
            loft_set_u32(loft_store_of(v), slot.rec, slot.pos, nr);
        }
        loft_vec_finish(v);
    }
}

/* OpReplaceVector of a linked element type (Stores::vector_replace): an absent source leaves the
 * destination absent; else the destination emptied — an array's clear is the plain reset — and
 * the source's records copied in as `loft_vec_add_array` copies them. */
static void loft_vec_replace_array(const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
    int absent = o.store_nr == 0xFFFF
        || (o.rec != 0 && o.pos != 0 && loft_u32(loft_store_of(o), o.rec, o.pos) == 0xFFFFFFFFu);
    if (absent) {
        loft_vec_clear(v);
        if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0) loft_set_u32(loft_store_of(v), v.rec, v.pos, 0xFFFFFFFFu);
        return;
    }
    if (v.store_nr == o.store_nr) {
        uint32_t dr = loft_u32(loft_store_of(v), v.rec, v.pos);
        if (dr != 0 && dr == loft_u32(loft_store_of(o), o.rec, o.pos)) return;
    }
    loft_vec_clear(v);
    loft_vec_add_array(T, tp, v, o);
}

/* OpMoveVector of a linked element type (Stores::move_vector): the slot list handed over within
 * one store into an empty destination, else its records copied. */
static void loft_move_vector_array(const loft_tdesc *T, uint16_t tp, loft_ref dst, loft_ref src) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    uint32_t h = loft_collection_rec(loft_store_of(src), src.rec, src.pos);
    if (h == 0) return;
    if (src.store_nr == dst.store_nr && loft_collection_rec(loft_store_of(dst), dst.rec, dst.pos) == 0) {
        loft_set_u32(loft_store_of(dst), dst.rec, dst.pos, h);
        loft_set_u32(loft_store_of(src), src.rec, src.pos, 0);
        return;
    }
    loft_vec_add_array(T, tp, dst, src);
}

/* Stores::insert_vector_element of a linked element type: a record-id slot inserted at `index`,
 * and a fresh record of `size` bytes claimed for the element, the vector's record named at its
 * byte 4; the element (its defaults the caller writes), or null for an index outside 0..=len. */
static loft_ref loft_vec_insert_linked(loft_ref v, uint32_t size, int64_t index) {
    loft_ref slot = loft_vec_insert(v, 4, index);
    if (slot.rec == 0) return slot;
    loft_store_t *s = loft_store_of(v);
    uint32_t nr = loft_claim(s, 1 + (size + 7) / 8);
    loft_set_u32(s, nr, 4, v.rec);
    loft_set_u32(s, slot.rec, slot.pos, nr);
    loft_ref e = { v.store_nr, nr, 8 };
    return e;
}

/* Stores::remove_vector_at of a linked element type: the slot at `index` unlinked, then the
 * record it named released and deleted — the order `remove_owned` uses, since the release reads
 * the record's own fields. */
static int loft_vec_remove_linked(const loft_tdesc *T, uint16_t tp, loft_ref v, int64_t index) {
    loft_ref slot = loft_vec_get(v, 4, index);
    if (slot.rec == 0) return 0;
    uint32_t rec = loft_u32(loft_store_of(v), slot.rec, slot.pos);
    int shifted = loft_vec_remove(v, 4, index);
    if (rec != 0) {
        loft_ref elem = { v.store_nr, rec, 8 };
        loft_remove_claims(T, tp, elem);
        loft_st_delete(loft_store_of(v), rec);
    }
    return shifted;
}

/* OpRemove in a walk over a vector (State::remove, Stores::remove_vector_at): the element's
 * claims released, then its successors shift down; forward, the index steps back to visit the
 * one that moved into its place. */
/* OpRemoveVector (`Stores::remove_vector_at`, an inline element): the element's claims are
 * released before the shift, since the slot it is is overwritten by its successor. */
static int loft_vec_remove_at(const loft_tdesc *T, uint16_t elem, loft_ref v, int64_t index) {
    /* A refused removal releases nothing either: the element's claims stay with it. */
    if (v.store_nr != 0xFFFF && v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(v))) return 0;
    loft_ref e = loft_vec_get(v, T[elem].size, index);
    if (e.rec != 0) loft_remove_claims(T, elem, e);
    return loft_vec_remove(v, T[elem].size, index);
}

static void loft_vec_walk_remove(const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref v, int reverse) {
    /* A refused removal leaves the cursor, so the walk moves past the element still there. */
    if (v.store_nr != 0xFFFF && v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(v))) return;
    loft_ref e = loft_vec_get(v, T[elem].size, *state);
    if (e.rec != 0) loft_remove_claims(T, elem, e);
    (void) loft_vec_remove(v, T[elem].size, *state);
    if (!reverse) *state = *state - 1;
}

/* OpRemove in a walk over an array — a vector of a linked element type (Stores::remove_vector_at's
 * linked branch): the slot is removed, then the record it named releases its claims and goes. */
static void loft_array_walk_remove(const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref v, int reverse) {
    /* A refused removal leaves the cursor, so the walk moves past the element still there. */
    if (v.store_nr != 0xFFFF && v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(v))) return;
    loft_ref slot = loft_vec_get(v, 4, *state);
    uint32_t rec = slot.rec == 0 ? 0 : loft_u32(loft_store_of(slot), slot.rec, slot.pos);
    if (slot.rec == 0) return;
    (void) loft_vec_remove(v, 4, *state);
    if (rec != 0) {
        loft_ref e = { v.store_nr, rec, 8 };
        loft_remove_claims(T, elem, e);
        loft_st_delete(loft_store_of(v), rec);
    }
    if (!reverse) *state = *state - 1;
}

/* OpRemove in a walk over a sorted (Stores::remove_vector_at): the element's claims released,
 * then the element removed and the walk moved on. */
static void loft_sorted_walk_remove(const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref c, int reverse) {
    if (c.store_nr != 0xFFFF && c.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(c))) return;
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
    if (c.store_nr != 0xFFFF && c.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(c))) return;
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

/* `OpVectorRef` (`State::vec_ref_or_raise`): an index naming no slot is reported. */
static loft_ref loft_array_get_at(loft_ref v, int64_t from, const char *file, uint32_t line) {
    int64_t len = loft_vec_len(v), i = from < 0 ? from + len : from;
    if (LOFT_UNLIKELY(i < 0 || i >= len)) loft_index_miss(from, len, file, line);
    return loft_array_get(v, from);
}

/* insert_record on an array: the record's number joins the end. */
static void loft_array_append(loft_ref v, loft_ref rec) {
    if (rec.rec == 0) return;
    loft_ref slot = loft_vec_append(v, 4);
    if (slot.rec == 0) return;
    loft_set_u32(loft_store_of(slot), slot.rec, slot.pos, rec.rec);
    loft_vec_finish(v);
}


/* Does keyed member `m` of `parent` file record `dr` under its own key? */
static int loft_member_files(loft_ref parent, const loft_member *m, loft_ref dr) {
    if (m->kind >= LOFT_M_SPATIAL) return 0;
    loft_ref c = loft_field(parent, m->pos);
    loft_kv key[8];
    int nk = m->nk > 8 ? 8 : m->nk;
    for (int k = 0; k < nk; k++) key[k] = loft_key_get(loft_store_of(dr), dr.rec, 8, &m->keys[k]);
    if (m->kind == LOFT_M_HASH) return loft_h_find(c, m->keys, key, nk).rec == dr.rec;
    if (m->kind == LOFT_M_INDEX) return loft_ix_find(c, m->keys, m->nk, key, nk).rec == dr.rec;
    return loft_seq_find(c, 0, m->keys, m->nk, key, nk).rec == dr.rec;
}

/* The record a keyed member answers under record `dr`'s key, or 0. */
static uint32_t loft_member_answer(loft_ref parent, const loft_member *m, loft_ref dr) {
    if (m->kind >= LOFT_M_SPATIAL) return 0;
    loft_ref c = loft_field(parent, m->pos);
    loft_kv key[8];
    int nk = m->nk > 8 ? 8 : m->nk;
    for (int k = 0; k < nk; k++) key[k] = loft_key_get(loft_store_of(dr), dr.rec, 8, &m->keys[k]);
    if (m->kind == LOFT_M_HASH) return loft_h_find(c, m->keys, key, nk).rec;
    if (m->kind == LOFT_M_INDEX) return loft_ix_find(c, m->keys, m->nk, key, nk).rec;
    return loft_seq_find(c, 0, m->keys, m->nk, key, nk).rec;
}

/* `@FR-Col-Group-Dup`: a displaced record leaves every keyed member that still files it under
 * its key, and every tree member; an array keeps it. */
static void loft_group_unfile(loft_ref parent, loft_ref dr, const loft_member *m, int n) {
    for (int i = 0; i < n; i++) {
        if (m[i].kind >= LOFT_M_SPATIAL) {
            (void) loft_rx_unlink(loft_field(parent, m[i].pos), dr, m[i].keys, m[i].nk, m[i].kind == LOFT_M_TRIE);
            continue;
        }
        if (m[i].kind == LOFT_M_ARRAY || !loft_member_files(parent, &m[i], dr)) continue;
        loft_ref c = loft_field(parent, m[i].pos);
        if (m[i].kind == LOFT_M_HASH) (void) loft_hash_unlink(c, dr, m[i].keys, m[i].nk);
        else if (m[i].kind == LOFT_M_INDEX) (void) loft_bt_remove(c, dr.rec, m[i].keys, m[i].nk);
        else loft_index_remove(c, dr);
    }
}

/* record_finish on a member of a linked group (`link_siblings`, `settle_displaced`): the record
 * enters every member; a record a key collision displaced from one keyed member leaves every
 * keyed member that still files it under its key, and is released unless a vector member of
 * the group (`held`) keeps it.  OpLinkRecord
 * (`link_record_siblings`) is this over the members beside the one that holds the record. */
static void loft_group_finish(const loft_tdesc *T, uint16_t elem, int held, int nullable, loft_ref parent,
                              loft_ref rec, const loft_member *m, int n) {
    if (parent.store_nr == 0xFFFF || parent.rec == 0 || rec.rec == 0) return;
    /* Each member displaces at most one record, so `n` bounds the list (loft#2257: a fixed 16
     * left the 17th displaced record filed).  No variable-length array: MSVC has none. */
    uint32_t gone_local[16];
    uint32_t *gone = gone_local;
    if (n > 16) {
        gone = (uint32_t *) malloc((size_t) n * sizeof *gone);
        if (!gone) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    int ng = 0;
    /* `absent_nullable_record`: a null element of a `vector<S?>` member has no key — only a
     * `Some` record (discriminant 2) enters the keyed and tree members. */
    int absent = nullable && *loft_at(loft_store_of(rec), rec.rec, rec.pos) != 2;
    for (int i = 0; i < n; i++) {
        loft_ref c = loft_field(parent, m[i].pos);
        uint32_t d = 0;
        if (m[i].kind != LOFT_M_ARRAY && absent) continue;
        if (m[i].kind == LOFT_M_ARRAY) loft_array_append(c, rec);
        else if (m[i].kind == LOFT_M_HASH) d = loft_hash_insert(0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        else if (m[i].kind == LOFT_M_INDEX) d = loft_ix_insert(0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        else if (m[i].kind >= LOFT_M_SPATIAL)
            d = loft_rx_insert_record(0, 0, c, rec, m[i].keys, m[i].nk, m[i].kind == LOFT_M_TRIE, 1).rec;
        else d = loft_ordered_insert(0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        int seen = d == 0 || d == rec.rec;
        for (int j = 0; j < ng; j++) seen |= gone[j] == d;
        if (!seen) gone[ng++] = d;
    }
    for (int g = 0; g < ng; g++) {
        loft_ref dr = { rec.store_nr, gone[g], 8 };
        loft_group_unfile(parent, dr, m, n);
        /* No vector member holds the displaced record: nothing names it any more. */
        if (!held) {
            loft_remove_claims(T, elem, dr);
            loft_st_delete(loft_store_of(dr), dr.rec);
        }
    }
    if (gone != gone_local) free(gone);
}

/* OpIndexGroup (`index_group_records`, `settle_group`): every record the vector member at
 * `primary` holds enters view member `v` — by id, the group being one record set; then a record
 * some keyed member answers another record for under its key leaves every keyed member, so
 * each answers the last record of the vector under each key.  `prim` is the vector member's
 * byte position in the parent; `m` every keyed member. */
static void loft_index_group(loft_ref primary, uint32_t prim, int nullable, const loft_member *v, const loft_member *m,
                             int n) {
    if (primary.store_nr == 0xFFFF || primary.rec == 0 || primary.pos < prim) return;
    loft_ref parent = { primary.store_nr, primary.rec, primary.pos - prim };
    int64_t len = loft_vec_len(primary);
    loft_store_t *s = loft_store_of(primary);
    uint32_t arr = loft_collection_rec(s, primary.rec, primary.pos);
    if (len == 0 || arr == 0) return;
    loft_ref c = loft_field(parent, v->pos);
    for (int64_t i = 0; i < len; i++) {
        loft_ref r = { primary.store_nr, loft_u32(loft_store_of(primary), arr, 8 + 4 * (uint32_t) i), 8 };
        if (r.rec == 0 || (nullable && *loft_at(loft_store_of(r), r.rec, 8) != 2)) continue;
        if (v->kind == LOFT_M_HASH) (void) loft_hash_insert(0, 0, c, r, v->keys, v->nk, 1);
        else if (v->kind == LOFT_M_INDEX) (void) loft_ix_insert(0, 0, c, r, v->keys, v->nk, 1);
        else if (v->kind >= LOFT_M_SPATIAL)
            (void) loft_rx_insert_record(0, 0, c, r, v->keys, v->nk, v->kind == LOFT_M_TRIE, 1);
        else (void) loft_ordered_insert(0, 0, c, r, v->keys, v->nk, 1);
    }
    for (int64_t i = 0; i < len; i++) {
        loft_ref r = { primary.store_nr, loft_u32(loft_store_of(primary), arr, 8 + 4 * (uint32_t) i), 8 };
        if (r.rec == 0 || (nullable && *loft_at(loft_store_of(r), r.rec, 8) != 2)) continue;
        int displaced = 0;
        for (int j = 0; j < n && !displaced; j++) {
            uint32_t got = loft_member_answer(parent, &m[j], r);
            displaced = got != 0 && got != r.rec;
        }
        if (displaced) loft_group_unfile(parent, r, m, n);
    }
}

/* One record a group's owner holds, filed into every view of that owner. */
static void loft_relink_record(const loft_tdesc *T, const loft_tdesc *d, uint16_t o, loft_ref to, loft_ref r) {
    for (uint16_t v = 0; v < d->n; v++) {
        const loft_fld *vf = &d->fields[v];
        if (!vf->view || vf->group != o + 1u) continue;
        const loft_tdesc *vt = &T[vf->type];
        /* A null element of a `vector<S?>` owner has no key (view 2). */
        if (vf->view == 2 && *loft_at(loft_store_of(r), r.rec, 8) != 2) continue;
        loft_ref c = loft_field(to, vf->pos);
        if (vt->kind == LOFT_T_HASH) (void) loft_hash_insert(0, 0, c, r, vt->keys, vt->nk, 1);
        else if (vt->kind == LOFT_T_INDEX) (void) loft_ix_insert(0, 0, c, r, vt->keys, vt->nk, 1);
        else if (vt->kind == LOFT_T_RADIX || vt->kind == LOFT_T_TRIE)
            (void) loft_rx_insert_record(0, 0, c, r, vt->keys, vt->nk, vt->kind == LOFT_T_TRIE, 1);
        else if (vt->kind == LOFT_T_ORDERED && vt->nk) (void) loft_ordered_insert(0, 0, c, r, vt->keys, vt->nk, 1);
        else if (vt->kind == LOFT_T_ORDERED) loft_array_append(c, r);
    }
}

/* `relink_group_views`: every view member of record `to` indexed over the records its copied
 * owner holds, in the owner's order (`for_each_owned_child`), as inserts through the owner
 * would — a later record under a repeated key takes the key, the state the source settled to.
 * A view only unlinks what it displaces, so nothing the owner holds is freed. */
static void loft_relink_group_views(const loft_tdesc *T, uint16_t tp, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    for (uint16_t o = 0; o < d->n; o++) {
        const loft_fld *of = &d->fields[o];
        if (of->view || of->group != o + 1u) continue;
        loft_ref owner = loft_field(to, of->pos);
        uint8_t kind = T[of->type].kind;
        uint32_t n = 0, *ids = 0;
        if (kind == LOFT_T_HASH) {
            loft_hent *all = loft_h_entries_of(owner, &n);
            for (uint32_t i = 0; i < n; i++) loft_relink_record(T, d, o, to, all[i].e);
            free(all);
            continue;
        }
        if (kind == LOFT_T_INDEX) ids = loft_bt_records(owner, &n);
        else if (kind == LOFT_T_RADIX || kind == LOFT_T_TRIE) ids = loft_rx_records(owner, &n);
        else if (kind == LOFT_T_ORDERED) {
            int64_t len = loft_vec_len(owner);
            uint32_t arr = loft_collection_rec(loft_store_of(owner), owner.rec, owner.pos);
            if (len == 0 || arr == 0) continue;
            ids = (uint32_t *) malloc((size_t) len * 4);
            if (!ids) { fputs("loft: out of memory\n", stderr); abort(); }
            for (int64_t i = 0; i < len; i++) ids[n++] = loft_u32(loft_store_of(owner), arr, 8 + 4 * (uint32_t) i);
        }
        for (uint32_t i = 0; i < n; i++) {
            loft_ref r = { owner.store_nr, ids[i], 8 };
            if (r.rec != 0) loft_relink_record(T, d, o, to, r);
        }
        free(ids);
    }
}

static void (*loft_relink_hook)(const loft_tdesc *T, uint16_t tp, loft_ref to) = loft_relink_group_views;

/* ── par (parallel_natives.rs): the rows in order ──
 * A `for x in v par(r = f(x), n)` loop hands every row to the worker and reads the results back
 * by row.  The C runs the worker over the rows one after another — each row's result is the
 * one a thread would compute — into a buffer on a stack the loop reads (`parallel_buf_get*`)
 * and pops (`parallel_buf_drop*`). */
typedef struct { int64_t i; double f; float g; loft_text t; loft_ref r; loft_fn fn; uint8_t adopted; } loft_par_slot;
/* `res` is the store a threaded site's record results were copied into (@PLN203 C4 tier 2),
 * released with the buffer; null for every other site. */
typedef struct { loft_par_slot *rows; int64_t n; loft_ref res; int64_t *ints; double *flts; float *sgls; } loft_par_buf;
/* A header built without `loft_cc.h` (the replays of tests/c_emit.rs) keeps one copy of its
 * state: no threads. */
#ifndef LOFT_TLS
#define LOFT_TLS
#endif
static LOFT_TLS loft_par_buf *loft_par_bufs = 0;
static LOFT_TLS int loft_npar = 0, loft_par_cap = 0;

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
    loft_par_bufs[loft_npar].res = loft_ref_null();
    loft_par_bufs[loft_npar].ints = 0;
    loft_par_bufs[loft_npar].flts = 0;
    loft_par_bufs[loft_npar].sgls = 0;
    loft_npar++;
    return rows;
}

/* A buffer of scalar results, one dense row each — an integer kind's, a float's or a single's —
 * every row its null until a worker writes it (`loft_par_push` holds every kind in a slot). */
static loft_par_buf *loft_par_push_dense(int64_t n) {
    if (loft_npar == loft_par_cap) {
        loft_par_cap = loft_par_cap ? loft_par_cap * 2 : 8;
        loft_par_bufs = (loft_par_buf *) realloc(loft_par_bufs, (size_t) loft_par_cap * sizeof *loft_par_bufs);
        if (!loft_par_bufs) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_par_buf *b = &loft_par_bufs[loft_npar++];
    b->rows = 0;
    b->n = n;
    b->res = loft_ref_null();
    b->ints = 0;
    b->flts = 0;
    b->sgls = 0;
    return b;
}

static int64_t *loft_par_push_i64(int64_t n) {
    loft_par_buf *b = loft_par_push_dense(n);
    b->ints = (int64_t *) malloc((size_t) (n > 0 ? n : 1) * sizeof *b->ints);
    if (!b->ints) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) b->ints[i] = LOFT_NULL;
    return b->ints;
}

static double *loft_par_push_f64(int64_t n) {
    loft_par_buf *b = loft_par_push_dense(n);
    b->flts = (double *) malloc((size_t) (n > 0 ? n : 1) * sizeof *b->flts);
    if (!b->flts) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) b->flts[i] = (double) NAN;
    return b->flts;
}

static float *loft_par_push_f32(int64_t n) {
    loft_par_buf *b = loft_par_push_dense(n);
    b->sgls = (float *) malloc((size_t) (n > 0 ? n : 1) * sizeof *b->sgls);
    if (!b->sgls) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) b->sgls[i] = (float) NAN;
    return b->sgls;
}

/* The top buffer when row `i` is one of its rows, else 0. */
static loft_par_buf *loft_par_top(int64_t i) {
    if (loft_npar == 0 || i < 0 || i >= loft_par_bufs[loft_npar - 1].n) return 0;
    return &loft_par_bufs[loft_npar - 1];
}

/* The top buffer's result store: freed when the buffer is dropped. */
static void loft_par_hold(loft_ref res) {
    if (loft_npar > 0) loft_par_bufs[loft_npar - 1].res = res;
}

static loft_par_slot *loft_par_row(int64_t i) {
    loft_par_buf *b = loft_par_top(i);
    return b && b->rows ? &b->rows[i] : 0;
}

static int64_t loft_par_get(int64_t i) {
    loft_par_buf *b = loft_par_top(i);
    if (b && b->ints) return b->ints[i];
    loft_par_slot *s = loft_par_row(i);
    return s ? s->i : LOFT_NULL;
}
static double loft_par_get_float(int64_t i) {
    loft_par_buf *b = loft_par_top(i);
    if (b && b->flts) return b->flts[i];
    loft_par_slot *s = loft_par_row(i);
    return s ? s->f : (double) NAN;
}
static float loft_par_get_single(int64_t i) {
    loft_par_buf *b = loft_par_top(i);
    if (b && b->sgls) return b->sgls[i];
    loft_par_slot *s = loft_par_row(i);
    return s ? s->g : (float) NAN;
}
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
    free(b->ints);
    free(b->flts);
    free(b->sgls);
    for (int64_t i = 0; b->rows && i < b->n; i++) {
        loft_text_free(&b->rows[i].t);
        /* n_parallel_buf_drop_ref frees the ADOPTED stores — the ones a worker made — never a
         * result that names a store which was there before (a pass-through of its element). */
        if (refs && b->rows[i].adopted) loft_free_ref(b->rows[i].r);
    }
    if (refs) loft_free_ref(b->res);
    free(b->rows);
}

/* ── par on threads (@PLN203 C4, tier 1) ──
 * `loft_par_run(n, threads, body, ctx)` runs `body(ctx, lo, hi)` over `min(threads, n)`
 * contiguous slices of the rows, as `par_pool::parallel_workers` splits them: the calling thread
 * takes the first slice, a thread each the others.  A worker starts from a COPY of the caller's
 * store table — every entry live and pinned, so it reads the caller's stores through the same
 * buffers and never frees, grows or reuses one (@FR-C-Impure) — and makes its own stores past
 * them, destroyed when it ends.  Each slice writes only its own rows of the result buffer.
 * `LOFT_TRACE_PAR_WORKERS` reports how many workers ran rows (@PLN117's instrument).  Without
 * `LOFT_THREADS` every slice runs here, in order: the same answer (@FR-C-Det). */
typedef void (*loft_par_body)(void *ctx, int64_t lo, int64_t hi);

/* Set while a `par` runs its slices on threads: every store live before it is SHARED — read by
 * the workers through the same buffers, the caller's slice included — so nothing may grow one
 * until the workers have joined (`loft_ffi_own_*` hands a library native a copy instead).
 * Per thread: it covers the table of the thread running `loft_par_run`, whose workers share it.
 * A worker's own view of those stores is marked `pinned`, which asks for the same copy, so a
 * nested `par` in a worker counts on that worker's thread and races with no other. */
static LOFT_TLS int loft_par_sharing;

/* @PLN203 C4 tier 2 — a worker whose result is a RECORD makes it in a store of its own table.
 * `adopt(ctx, lo, hi, parent_n)` runs in the caller after that worker joined, with the worker's
 * table current, and copies each of the rows `lo..hi` whose record lies in a store the worker
 * made (number `parent_n` or past it) into the caller's result store — its bytes and the claims
 * it owns, as `Stores::copy_from_worker` does — so the worker's stores can go.  A reference held
 * in such a record is copied as it stands, as native copies it. */
typedef void (*loft_par_adopt)(void *ctx, int64_t lo, int64_t hi, uint32_t parent_n);

#if defined(LOFT_THREADS)
#if defined(_WIN32)
#include <process.h>
__declspec(dllimport) unsigned long __stdcall WaitForSingleObject(void *handle, unsigned long ms);
__declspec(dllimport) int __stdcall CloseHandle(void *handle);
#else
#include <pthread.h>
#endif

typedef struct {
    loft_par_body body;
    void *ctx;
    int64_t lo, hi;
    loft_store_t *table;
    uint32_t n, parent_n;
    /* Keep the stores it made when it ends: the caller copies the rows' records out first.
     * The table as the worker leaves it — a growth may have moved it — is handed back in
     * `kept`, written only by the worker and read by the caller after the join; `table`
     * stays the caller's mark that the thread was started. */
    int keep;
    loft_store_t *kept;
    uint32_t kept_n;
} loft_par_job;

/* The stores of `table` from `from` on, and the table: what a worker made. */
static void loft_par_free_table(loft_store_t *table, uint32_t from, uint32_t n) {
    uint32_t i;
    for (i = from; i < n; i++) {
        loft_store_t *s = &table[i];
        free(s->buf);
        free(s->claims.bits);
        free(s->referenced.bits);
        free(s->removed.bits);
        free(s->lazy_free.bits);
        free(s->parked_set.bits);
        free(s->placed);
    }
    free(table);
}

/* A worker's end: the stores it made are destroyed with its table and its frames — or, when it
 * keeps them, its table as it now stands handed back for the caller to adopt from. */
static void loft_par_job_run(loft_par_job *j) {
    loft_stores = j->table;
    loft_nstores = j->n;
    j->body(j->ctx, j->lo, j->hi);
    if (j->keep) {
        j->kept = loft_stores;
        j->kept_n = loft_nstores;
    } else {
        loft_par_free_table(loft_stores, j->parent_n, loft_nstores);
    }
    loft_stores = 0;
    loft_nstores = 0;
#if defined(LOFT_CALL_CHAIN)
    free(loft_frames);
    loft_frames = 0;
#endif
#if defined(LOFT_HALT_RELEASE)
    free(loft_held);
    loft_held = 0;
    loft_nheld = loft_capheld = 0;
#endif
}

#if defined(_WIN32)
static unsigned __stdcall loft_par_thread(void *arg) {
    loft_par_job_run((loft_par_job *) arg);
    return 0;
}
#else
static void *loft_par_thread(void *arg) {
    loft_par_job_run((loft_par_job *) arg);
    return 0;
}
#endif
#endif

/* `adopt` is 0 for a site whose results are values; for a record site it copies each worker's
 * rows into `res`, the caller's result store, made before the workers fork so every worker's
 * table holds it below `parent_n` (see loft_par_adopt). */
static void loft_par_run(int64_t n, int64_t threads, loft_par_body body, void *ctx, loft_par_adopt adopt, uint16_t res) {
    int64_t t = threads == LOFT_NULL || threads < 1 ? 1 : threads;
    if (t > n) t = n > 1 ? n : 1;
    if (getenv("LOFT_TRACE_PAR_WORKERS"))
        fprintf(stderr, "loft: LOFT_TRACE_PAR_WORKERS c n_rows=%lld threads=%lld distinct_workers=%lld\n",
                (long long) n, (long long) t, (long long) (n > 0 ? t : 0));
#if defined(LOFT_THREADS)
    if (t > 1) {
        int64_t k;
        loft_par_sharing++;
        loft_par_job *jobs = (loft_par_job *) calloc((size_t) t, sizeof *jobs);
#if defined(_WIN32)
        void **ids = (void **) calloc((size_t) t, sizeof *ids);
#else
        pthread_t *ids = (pthread_t *) calloc((size_t) t, sizeof *ids);
#endif
        if (!jobs || !ids) { fputs("loft: out of memory\n", stderr); abort(); }
        for (k = 1; k < t; k++) {
            uint32_t i, room = loft_nstores + 16;
            loft_par_job *j = &jobs[k];
            j->body = body;
            j->ctx = ctx;
            j->lo = k * n / t;
            j->hi = (k + 1) * n / t;
            /* Copied here, before any thread runs: the caller's own slice may grow its table. */
            j->table = (loft_store_t *) calloc(room, sizeof *j->table);
            if (!j->table) { fputs("loft: out of memory\n", stderr); abort(); }
            memcpy(j->table, loft_stores, (size_t) loft_nstores * sizeof *j->table);
            for (i = 0; i < loft_nstores; i++) {
                j->table[i].live = 1;
                j->table[i].pinned = 1;
            }
            j->n = room;
            j->parent_n = loft_nstores;
            j->keep = adopt != 0;
        }
        for (k = 1; k < t; k++) {
            /* A thread that cannot start: its slice runs here, on this thread's own table. */
#if defined(_WIN32)
            ids[k] = (void *) _beginthreadex(0, 0, loft_par_thread, &jobs[k], 0, 0);
            if (!ids[k]) {
#else
            if (pthread_create(&ids[k], 0, loft_par_thread, &jobs[k]) != 0) {
#endif
                free(jobs[k].table);
                jobs[k].table = 0;
                body(ctx, jobs[k].lo, jobs[k].hi);
            }
        }
        body(ctx, 0, n / t);
        for (k = 1; k < t; k++) {
#if defined(_WIN32)
            if (jobs[k].table) { WaitForSingleObject(ids[k], 0xFFFFFFFFul); CloseHandle(ids[k]); }
#else
            if (jobs[k].table) pthread_join(ids[k], 0);
#endif
        }
        loft_par_sharing--;
        /* Each worker's records copied out with its table current, one worker at a time; the
         * result store's entry travels in from the caller's table and back, so a growth of it
         * in one worker's adoption is seen by the next. */
        for (k = 1; adopt && k < t; k++) {
            loft_par_job *j = &jobs[k];
            if (!j->table || !j->kept) continue;
            loft_store_t *own = loft_stores;
            uint32_t own_n = loft_nstores;
            j->kept[res] = own[res];
            loft_stores = j->kept;
            loft_nstores = j->kept_n;
            adopt(ctx, j->lo, j->hi, j->parent_n);
            own[res] = loft_stores[res];
            loft_stores = own;
            loft_nstores = own_n;
            /* The result store's entry is the caller's again: not freed with the worker's. */
            loft_par_free_table(j->kept, j->parent_n, j->kept_n);
            j->kept = 0;
        }
        free(ids);
        free(jobs);
        return;
    }
#endif
    (void) adopt;
    (void) res;
    body(ctx, 0, n);
}

/* parallel_buf_get_narrow: a 1-, 2- or 4-byte result, its low bytes extended as `signed` says. */
static int64_t loft_par_get_narrow(int64_t i, int64_t size, int64_t is_signed) {
    loft_par_buf *b = loft_par_top(i);
    if (!b) return LOFT_NULL;
    int64_t v = b->ints ? b->ints[i] : b->rows ? b->rows[i].i : LOFT_NULL;
    uint64_t u = (uint64_t) v;
    int bits = (int) (size * 8);
    if (bits <= 0 || bits >= 64) return v;
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
