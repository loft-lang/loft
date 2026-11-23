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
static uint32_t loft_keyed_rec(loft_cx *cx, loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    return loft_collection_rec(cx, loft_store_of(cx, c), c.rec, c.pos);
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
static void loft_group_finish(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int held, int nullable, loft_ref parent,
                              loft_ref rec, const loft_member *m, int n);

/* A new entry of an index (Stores::record_new): a record of its own in the collection's store,
 * naming at byte 4 the record that holds the collection, filed by `finish` once its fields are
 * written. */
LOFT_HOT_INLINE loft_ref loft_entry_new(loft_cx *cx, loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    /* A locked collection gains no entry: the null record, whose writes are skipped. */
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return loft_ref_null(cx);
    loft_ref e = {0};
    e.store_nr = c.store_nr;
    e.rec = loft_claim(cx, loft_store_of(cx, c), 1 + (size + 7) / 8);
    e.pos = 8;
    loft_set_u32(cx, loft_store_of(cx, c), e.rec, 4, c.rec);
    return e;
}

/* ── sorted and index: one key-ordered sequence each ── */

/* An ordered sequence: a sorted's inline elements of `size`, or (size 0) an index's record
 * numbers, each naming its entry at byte 8. */
typedef struct { loft_store_t *s; uint32_t vrec; uint32_t size; } loft_seq;

static void loft_seq_at(loft_cx *cx, const loft_seq *q, uint32_t i, uint32_t *rec, uint32_t *pos) {
    if (q->size) {
        *rec = q->vrec;
        *pos = 8 + i * q->size;
    } else {
        *rec = loft_u32(cx, q->s, q->vrec, 8 + i * 4);
        *pos = 8;
    }
}

/* vector::sorted_search over the first `len` elements: where the key goes, `before` every
 * element equal to it or after them, and whether one was.  The key is `n` values (`kv`), or
 * when `kv` is null the key of the element at (krec, kpos). */
static uint32_t loft_seq_search(loft_cx *cx, const loft_seq *q, uint32_t len, int before, const loft_kv *kv, int n,
                                uint32_t krec, uint32_t kpos, const loft_key *keys, int nk, int *found) {
    *found = 0;
    if (len == 0) return 0;
    uint32_t left = 0, right = len - 1;
    for (;;) {
        uint32_t mid = left + (right - left) / 2, rec, pos;
        loft_seq_at(cx, q, mid, &rec, &pos);
        int c = kv ? loft_key_cmp_val(cx, kv, n, q->s, rec, pos, keys)
                   : loft_key_cmp_recs(cx, q->s, krec, kpos, rec, pos, keys, nk);
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

static loft_seq loft_seq_of(loft_cx *cx, loft_ref c, uint32_t size) {
    loft_seq q;
    q.s = loft_store_of(cx, c);
    q.vrec = loft_keyed_rec(cx, c);
    q.size = size;
    return q;
}

static uint32_t loft_seq_len(loft_cx *cx, const loft_seq *q) {
    return q->vrec == 0 ? 0 : loft_u32(cx, q->s, q->vrec, 4);
}

/* Stores::find on a sorted (`size` its element) or an index (`size` 0): the first element
 * whose key parts equal the `n` given, or the null reference. */
static loft_ref loft_seq_find(loft_cx *cx, loft_ref c, uint32_t size, const loft_key *keys, int nk, const loft_kv *kv,
                              int n) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    loft_seq q = loft_seq_of(cx, c, size);
    int found;
    uint32_t p = loft_seq_search(cx, &q, loft_seq_len(cx, &q), 1, kv, n, 0, 0, keys, nk, &found);
    if (!found) return loft_ref_null(cx);
    loft_ref e = {0};
    e.store_nr = c.store_nr;
    loft_seq_at(cx, &q, p, &e.rec, &e.pos);
    return e;
}

/* vector::sorted_new: room for one more element, staged one slot past the end — the gap
 * `loft_sorted_finish` opens moves it into place — or in slot 0 of an empty sorted.  The vector
 * is made with the growth step's room for two (the element and the scratch slot) and takes the
 * growth step again when its length plus two no longer fits (`@FR-Col-Capacity`). */
static loft_ref loft_sorted_new(loft_cx *cx, loft_ref c, uint32_t size) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return loft_ref_null(cx);
    loft_store_t *s = loft_store_of(cx, c);
    uint32_t r = loft_collection_rec(cx, s, c.rec, c.pos);
    loft_ref e = { c.store_nr, 0, 8, 0 };
    if (r == 0) {
        r = loft_st_claim(cx, s, loft_vec_cap_words(cx, loft_vec_grown(cx, 2, size), size));
        loft_set_u32(cx, s, c.rec, c.pos, r);
        loft_set_u32(cx, s, r, 4, 0);
        e.rec = r;
        return e;
    }
    uint32_t len = loft_u32(cx, s, r, 4);
    /* The element and the scratch slot: the growth step only when they do not fit. */
    int32_t claimed;
    memcpy(&claimed, loft_at(cx, s, r, 0), 4);
    uint32_t nr = len + 2 > ((uint32_t) claimed * 8 - 8) / size
        ? loft_st_resize(cx, s, r, loft_vec_cap_words(cx, loft_vec_grown(cx, len + 2, size), size)) : r;
    if (nr != r) {
        loft_set_u32(cx, s, c.rec, c.pos, nr);
        r = nr;
    }
    e.rec = r;
    e.pos = 8 + (len == 0 ? 0 : len + 1) * size;
    return e;
}

/* vector::sorted_finish: the staged element moves to its key's place; one whose key is
 * already there replaces that element (latest insert wins). */
static void loft_sorted_finish(loft_cx *cx, loft_ref c, uint32_t size, const loft_key *keys, int nk) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return;
    loft_seq q = loft_seq_of(cx, c, size);
    if (q.vrec == 0) return;
    uint32_t len = loft_u32(cx, q.s, q.vrec, 4);
    if (len == 0) {
        loft_set_u32(cx, q.s, q.vrec, 4, 1);
        return;
    }
    uint32_t latest = 8 + (len + 1) * size;
    int found;
    uint32_t p = loft_seq_search(cx, &q, len, 1, 0, 0, q.vrec, latest, keys, nk, &found);
    uint8_t *base = loft_at(cx, q.s, q.vrec, 0);
    if (found) {
        memmove(base + 8 + p * size, base + latest, size);
        return;
    }
    if (p < len) memmove(base + 8 + (p + 1) * size, base + 8 + p * size, (size_t) (len - p) * size);
    memmove(base + 8 + p * size, base + latest, size);
    loft_set_u32(cx, q.s, q.vrec, 4, len + 1);
}

/* The element `e` leaves the sorted: its successors move down. */
static void loft_sorted_remove(loft_cx *cx, loft_ref c, loft_ref e, uint32_t size) {
    uint32_t r = loft_keyed_rec(cx, c);
    if (r == 0 || e.rec != r || e.pos < 8) return;
    loft_store_t *s = loft_store_of(cx, c);
    uint32_t len = loft_u32(cx, s, r, 4), i = (e.pos - 8) / size;
    if (i >= len) return;
    uint8_t *base = loft_at(cx, s, r, 0);
    memmove(base + 8 + i * size, base + 8 + (i + 1) * size, (size_t) (len - i - 1) * size);
    loft_set_u32(cx, s, r, 4, len - 1);
}

/* vector::ordered_finish (@FR-Col-Insert): the record's number goes to its key's place in the
 * ordered; a record the key already named is replaced and its number answered, 0 when the key
 * was new. */
static uint32_t loft_ordered_finish(loft_cx *cx, loft_ref c, loft_ref e, const loft_key *keys, int nk) {
    if (c.store_nr == 0xFFFF || c.rec == 0 || e.rec == 0) return 0;
    loft_ref slot = loft_sorted_new(cx, c, 4);
    loft_seq q = loft_seq_of(cx, c, 0);
    uint32_t len = loft_seq_len(cx, &q);
    if (len == 0) {
        loft_set_u32(cx, q.s, q.vrec, 4, 1);
        loft_set_u32(cx, q.s, q.vrec, slot.pos, e.rec);
        return 0;
    }
    int found;
    uint32_t p = loft_seq_search(cx, &q, len, 1, 0, 0, e.rec, 8, keys, nk, &found);
    if (found) {
        uint32_t old = loft_u32(cx, q.s, q.vrec, 8 + p * 4);
        loft_set_u32(cx, q.s, q.vrec, 8 + p * 4, e.rec);
        return old == e.rec ? 0 : old;
    }
    uint8_t *base = loft_at(cx, q.s, q.vrec, 0);
    if (p < len) memmove(base + 12 + p * 4, base + 8 + p * 4, (size_t) (len - p) * 4);
    loft_set_u32(cx, q.s, q.vrec, 8 + p * 4, e.rec);
    loft_set_u32(cx, q.s, q.vrec, 4, len + 1);
    return 0;
}

/* Stores::insert_record on an ordered: the record filed; the record its key displaced is
 * answered, released already unless `secondary` (a group member only unlinks). */
static loft_ref loft_ordered_insert(loft_cx *cx, const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, const loft_key *keys,
                                    int nk, int secondary) {
    uint32_t gone = loft_ordered_finish(cx, c, e, keys, nk);
    if (gone == 0) return loft_ref_null(cx);
    loft_ref old = { c.store_nr, gone, 8, 0 };
    if (!secondary) {
        loft_remove_claims(cx, T, elem, old);
        loft_st_delete(cx, loft_store_of(cx, c), gone);
    }
    return old;
}

/* The record `e` leaves an ordered or an array: its slot, found by its record number, is
 * removed (Stores::unlink, by reference). */
static void loft_index_remove(loft_cx *cx, loft_ref c, loft_ref e) {
    uint32_t r = loft_keyed_rec(cx, c);
    if (r == 0 || e.rec == 0) return;
    loft_store_t *s = loft_store_of(cx, c);
    uint32_t len = loft_u32(cx, s, r, 4);
    for (uint32_t i = 0; i < len; i++) {
        if (loft_u32(cx, s, r, 8 + i * 4) != e.rec) continue;
        uint8_t *base = loft_at(cx, s, r, 0);
        memmove(base + 8 + i * 4, base + 8 + (i + 1) * 4, (size_t) (len - i - 1) * 4);
        loft_set_u32(cx, s, r, 4, len - 1);
        return;
    }
}

/* Stores::remove_owned on an ordered: unlinked, then its claims and its record released; on an
 * array (`is_array`, whose removal reads `e` as given) the claims and then the unlink.  A `view`
 * member only unlinks. */
static void loft_ordered_remove(loft_cx *cx, const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, int is_array, int view) {
    if (e.rec == 0 || c.store_nr == 0xFFFF || c.rec == 0) return;
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return;
    if (view) {
        loft_index_remove(cx, c, e);
        return;
    }
    if (is_array) {
        loft_remove_claims(cx, T, elem, e);
        loft_index_remove(cx, c, e);
        return;
    }
    loft_index_remove(cx, c, e);
    loft_ref rec = { e.store_nr, e.rec, 8, 0 };
    loft_remove_claims(cx, T, elem, rec);
    loft_st_delete(cx, loft_store_of(cx, c), e.rec);
}

/* OpIterate over a sorted (on 2) or an index (on 1): the positions of the key range.  `from` is
 * inclusive; `ex` says `till` is exclusive; an empty bound is open (State::iterate,
 * btree::range). */
static int64_t loft_seq_iterate(loft_cx *cx, loft_ref c, uint32_t size, int ex, const loft_key *keys, int nk,
                                const loft_kv *from, int nfrom, const loft_kv *till, int ntill) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    loft_seq q = loft_seq_of(cx, c, size);
    uint32_t len = loft_seq_len(cx, &q);
    int found;
    uint32_t lo = nfrom == 0 ? 0 : loft_seq_search(cx, &q, len, 1, from, nfrom, 0, 0, keys, nk, &found);
    uint32_t hi = ntill == 0 ? len : loft_seq_search(cx, &q, len, ex, till, ntill, 0, 0, keys, nk, &found);
    return loft_iter_range(cx, lo, hi);
}

/* OpStep over a sorted or an index: the next element, or the null reference. */
static loft_ref loft_seq_step(loft_cx *cx, int64_t *state, loft_ref c, uint32_t size, int reverse) {
    int64_t at = loft_iter_next(cx, state, reverse);
    if (at < 0 || c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    loft_seq q = loft_seq_of(cx, c, size);
    if ((uint32_t) at >= loft_seq_len(cx, &q)) return loft_ref_null(cx);
    loft_ref e = {0};
    e.store_nr = c.store_nr;
    loft_seq_at(cx, &q, (uint32_t) at, &e.rec, &e.pos);
    return e;
}

/* ── copies, renders and equality over the entries ── */

/* The elements of a hash (kind 1, in key order), an index (2, in key order), a spatial or a trie
 * (3, in tree order), or an ordered or an array (0, in their own), as references; `*n` their
 * count. */
static loft_ref *loft_keyed_refs(loft_cx *cx, loft_ref c, int kind, const loft_key *keys, int nk, uint32_t *n) {
    *n = 0;
    uint32_t *recs = 0;
    uint32_t count = 0;
    if (kind == 1) return loft_hash_refs(cx, c, keys, nk, 0, n);
    if (kind == 2 || kind == 3) {
        recs = c.store_nr == 0xFFFF || c.rec == 0 ? (uint32_t *) calloc(1, 4)
               : kind == 2                         ? loft_bt_records(cx, c, &count)
                                                   : loft_rx_records(cx, c, &count);
        if (!recs) { fputs("loft: out of memory\n", stderr); abort(); }
    } else {
        uint32_t r = loft_keyed_rec(cx, c);
        if (r != 0) {
            count = loft_u32(cx, loft_store_of(cx, c), r, 4);
            recs = (uint32_t *) malloc((size_t) count * 4 + 4);
            if (!recs) { fputs("loft: out of memory\n", stderr); abort(); }
            if (count) memcpy(recs, loft_at(cx, loft_store_of(cx, c), r, 8), (size_t) count * 4);
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
static void loft_show_refs(loft_cx *cx, loft_text *s, const loft_ref *e, uint32_t n, int fmt, int ind, int complex_elem,
                           int struct_elem, loft_show_fn f) {
    int pretty = fmt & 1;
    int c = pretty && complex_elem;
    loft_push_char(cx, s, '[');
    for (uint32_t i = 0; i < n; i++) {
        if (i == 0) {
            if (pretty) { if (c) loft_nl(cx, s, ind + 1); else loft_push_char(cx, s, ' '); }
        } else {
            loft_push_char(cx, s, ',');
            if (pretty) { if (struct_elem) loft_nl(cx, s, ind + 1); else loft_push_char(cx, s, ' '); }
        }
        /* `@FR-F-Render-Ref` — a keyed element's segment is its POSITION in this list. */
        loft_rr_push_index(cx, i);
        f(cx, s, e[i], fmt, ind + 1);
        loft_rr_pop(cx);
    }
    if (pretty) loft_push_char(cx, s, ' ');
    loft_push_char(cx, s, ']');
}

static void loft_show_keyed(loft_cx *cx, loft_text *s, loft_ref r, int fmt, int ind, int kind, const loft_key *keys,
                            int nk, int complex_elem, int struct_elem, loft_show_fn f) {
    if (loft_vec_absent(cx, r)) { loft_push(cx, s, "null", 4); return; }
    uint32_t n;
    loft_ref *e = loft_keyed_refs(cx, r, kind, keys, nk, &n);
    loft_show_refs(cx, s, e, n, fmt, ind, complex_elem, struct_elem, f);
    free(e);
}

static int loft_eq_keyed(loft_cx *cx, loft_ref a, loft_ref b, int kind, const loft_key *keys, int nk, loft_eq_fn f) {
    uint32_t na, nb;
    /* Stores::eq_absent_collections: an absent collection equals an absent one only. */
    int aa = loft_vec_absent(cx, a), ab = loft_vec_absent(cx, b);
    if (aa || ab) return aa && ab;
    loft_ref *ea = loft_keyed_refs(cx, a, kind, keys, nk, &na);
    loft_ref *eb = loft_keyed_refs(cx, b, kind, keys, nk, &nb);
    int same = na == nb;
    for (uint32_t i = 0; same && i < na; i++) same = f(cx, ea[i], eb[i]);
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
static void loft_replace_keyed(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, loft_ref dest, int flags) {
    int free_source = flags & 1;
    if (src.store_nr == 0xFFFF) {
        loft_remove_claims(cx, T, tp, dest);
        if (dest.store_nr != 0xFFFF && dest.rec != 0 && dest.pos != 0)
            loft_set_u32(cx, loft_store_of(cx, dest), dest.rec, dest.pos, 0xFFFFFFFFu);
        return;
    }
    if (src.store_nr == dest.store_nr && src.rec == dest.rec && src.pos == dest.pos) return;
    if (dest.store_nr == 0xFFFF || dest.rec == 0) return;
    if (free_source && (flags & 2) && loft_swap_in(cx, tp, src, dest, 0)) return;
    loft_remove_claims(cx, T, tp, dest);
    loft_copy_claims(cx, T, tp, src, dest);
#ifdef LOFT_COPY_CHECK
    loft_report_copy_mismatches(cx, T, tp, src, dest, "OpReplaceKeyed");
#endif
    if (free_source && src.store_nr != dest.store_nr && !loft_free_protected(cx, src)) loft_free_ref(cx, src);
}

/* keys::set_key: key part `k` of the element `e` takes the value `v`. */
static void loft_key_set(loft_cx *cx, loft_ref e, const loft_key *k, loft_kv v) {
    loft_store_t *s = loft_store_of(cx, e);
    uint32_t at = e.pos + k->pos;
    int t = k->type < 0 ? -k->type : k->type;
    switch (t) {
    case 1:
    case 2:
        memcpy(loft_at(cx, s, e.rec, at), &v.i, 8);
        return;
    case 3:
        memcpy(loft_at(cx, s, e.rec, at), &v.g, 4);
        return;
    case 4:
        memcpy(loft_at(cx, s, e.rec, at), &v.f, 8);
        return;
    case 6: {
        /* The text the key field held — the copy's, or the literal's — is released first, as
         * `keys::set_key` deletes it; written over, it stayed claimed in the collection's store,
         * one record per text-keyed insert. */
        uint32_t old;
        memcpy(&old, loft_at(cx, s, e.rec, at), 4);
        if (old != 0 && !LOFT_WRITE_REFUSED(s)) loft_st_delete(cx, s, old);
        loft_set_text(cx, e, k->pos, v.s);
        return;
    }
    case 8: {
        int32_t x = loft_i32_of(cx, v.i);
        memcpy(loft_at(cx, s, e.rec, at), &x, 4);
        return;
    }
    case 12: {
        uint32_t x = (uint32_t) (uint64_t) v.i;
        memcpy(loft_at(cx, s, e.rec, at), &x, 4);
        return;
    }
    case 9:
        loft_set_short(cx, e, k->pos, k->start, v.i);
        return;
    case 11: {
        uint16_t x = (uint16_t) (uint64_t) (v.i - k->start);
        memcpy(loft_at(cx, s, e.rec, at), &x, 2);
        return;
    }
    default:
        loft_set_byte_v(cx, e, k->pos, k->start, v.i);
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
static loft_ref loft_keyed_find(loft_cx *cx, loft_ref c, int kind, uint32_t size, const loft_key *keys, int nk,
                                const loft_kv *kv, int n) {
    if (kind == LOFT_KEYED_HASH) return loft_h_find(cx, c, keys, kv, n);
    if (kind == LOFT_KEYED_INDEX) return loft_ix_find(cx, c, keys, nk, kv, n);
    if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE) return loft_rx_find(cx, c, keys, nk, kind == LOFT_KEYED_TRIE, kv, n);
    return loft_seq_find(cx, c, kind == LOFT_KEYED_SORTED ? size : 0, keys, nk, kv, n);
}

/* The element `existing` of `c` taken out and released, by the collection's kind — the removal
 * `c[k…] = v` makes before it files the copy, and the one an absent `v` makes alone. */
static void loft_sorted_remove_owned(loft_cx *cx, const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, uint32_t size);

static void loft_keyed_remove_at(loft_cx *cx, const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref existing, int kind,
                                 const loft_key *keys, int nk, uint32_t size) {
    if (kind == LOFT_KEYED_HASH) loft_hash_remove(cx, T, etp, c, existing, keys, nk, 0);
    /* A sorted element lives inline: what it owns (a text key, a nested collection) is released
     * before its successor shifts over it, as `Stores::remove_owned` does. */
    else if (kind == LOFT_KEYED_SORTED) loft_sorted_remove_owned(cx, T, etp, c, existing, size);
    else if (kind == LOFT_KEYED_INDEX) loft_ix_remove(cx, T, etp, c, existing, keys, nk, 0);
    else if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE)
        loft_rx_remove_owned(cx, T, etp, c, existing, keys, nk, kind == LOFT_KEYED_TRIE, 0);
    else loft_ordered_remove(cx, T, etp, c, existing, 0, 0);
}

/* OpSetKeyed (Stores::set_keyed, @FR-Col-Assign): `c[k…] = v` — a deep copy of `v` filed under
 * the subscript's key (or, with no full subscript, under its own), replacing the element that
 * key named; a value the bind gives up is freed. */
static void loft_set_keyed(loft_cx *cx, const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref value, int kind,
                           const loft_key *keys, int nk, const loft_kv *sub, int nsub, int free_source,
                           const loft_group *g) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return;
    uint32_t size = T[etp].size;
    int full = nsub > 0 && nsub == nk;
    /* @FR-Col-Assign, @FR-Col-Remove: an absent value leaves the place reading null, as
     * `c[k] = null` does — the element under the full subscript is removed (loft#2470).  A
     * linked group's member keeps the skip: its removal unlinks every member. */
    if (value.store_nr == 0xFFFF || value.rec == 0) {
        if (full && !g) {
            loft_ref existing = loft_keyed_find(cx, c, kind, size, keys, nk, sub, nsub);
            if (existing.rec != 0) {
                loft_keyed_remove_at(cx, T, etp, c, existing, kind, keys, nk, size);
            }
        }
        return;
    }
    loft_ref source = value, tmp = loft_ref_null(cx);
    if (full && value.store_nr == c.store_nr) {
        loft_database(cx, &tmp, size);
        loft_copy_bytes(cx, tmp, 0, value, 0, size);
        loft_copy_claims(cx, T, etp, value, tmp);
        source = tmp;
    }
    /* The key, its texts owned: the claims below may move the store a view points into. */
    loft_kv key[8];
    loft_text own[8];
    int n = full ? nsub : nk;
    if (n > 8) n = 8;
    for (int i = 0; i < n; i++) {
        key[i] = full ? sub[i] : loft_key_get(cx, loft_store_of(cx, source), source.rec, source.pos, &keys[i]);
        own[i].p = 0;
        own[i].len = own[i].cap = 0;
        if (key[i].kind == LOFT_KV_TEXT) {
            own[i] = loft_text_own(cx, key[i].s);
            key[i].s = loft_view(cx, &own[i]);
        }
    }
    loft_ref existing = loft_keyed_find(cx, c, kind, size, keys, nk, key, n);
    if (existing.rec != 0) {
        loft_keyed_remove_at(cx, T, etp, c, existing, kind, keys, nk, size);
    }
    loft_ref e = kind == LOFT_KEYED_SORTED ? loft_sorted_new(cx, c, size)
                 : kind == LOFT_KEYED_HASH ? loft_hash_entry(cx, c, size, T[etp].linked)
                                           : loft_entry_new(cx, c, size);
    loft_copy_bytes(cx, e, 0, source, 0, size);
    loft_copy_claims(cx, T, etp, source, e);
    if (full) for (int i = 0; i < n; i++) loft_key_set(cx, e, &keys[i], key[i]);
    /* A member of a linked group: the record enters every member (`record_finish`). */
    if (g) loft_group_finish(cx, T, etp, g->held, g->nullable, g->parent, e, g->m, g->n);
    else if (kind == LOFT_KEYED_HASH) (void) loft_hash_insert(cx, T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SORTED) loft_sorted_finish(cx, c, size, keys, nk);
    else if (kind == LOFT_KEYED_INDEX) (void) loft_ix_insert(cx, T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE)
        (void) loft_rx_insert_record(cx, T, etp, c, e, keys, nk, kind == LOFT_KEYED_TRIE, 0);
    else (void) loft_ordered_insert(cx, T, etp, c, e, keys, nk, 0);
    for (int i = 0; i < n; i++) loft_text_free(cx, &own[i]);
    if (tmp.store_nr != 0xFFFF) loft_free_ref(cx, tmp);
    if (free_source && value.store_nr != c.store_nr && !loft_free_protected(cx, value)) loft_free_ref(cx, value);
}

/* The text key fields of a freshly claimed element `e`, emptied.  A claim is not zeroed
 * (`@FR-H-Claim`: in a store without `defined` it holds whatever the slot held before), and
 * `loft_key_set` releases the text a key field holds, as `keys::set_key` does — so a field
 * nothing wrote must read as no text, or the release frees a record some other value owns. */
static void loft_key_texts_clear(loft_cx *cx, loft_ref e, const loft_key *keys, int nk) {
    if (e.rec == 0) return;
    for (int i = 0; i < nk; i++)
        if ((keys[i].type < 0 ? -keys[i].type : keys[i].type) == 6) {
            uint32_t none = 0;
            memcpy(loft_at(cx, loft_store_of(cx, e), e.rec, e.pos + keys[i].pos), &none, 4);
        }
}

/* `@FR-R-InPlaceLiteral`'s keyed clause, the first half of `c[k…] = R { … }` written in place
 * (Stores::keyed_place_begin): the element the FULL subscript names removed, as `loft_set_keyed`
 * removes it, and a fresh one claimed in the collection's store, not yet filed — the literal's
 * fields are written into it before `loft_keyed_place_finish` files it. */
static loft_ref loft_keyed_place_begin(loft_cx *cx, const loft_tdesc *T, uint16_t etp, loft_ref c, int kind, const loft_key *keys,
                                       int nk, const loft_kv *sub, int nsub) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    uint32_t size = T[etp].size;
    loft_ref existing = loft_keyed_find(cx, c, kind, size, keys, nk, sub, nsub);
    if (existing.rec != 0) {
        loft_keyed_remove_at(cx, T, etp, c, existing, kind, keys, nk, size);
    }
    loft_ref e = kind == LOFT_KEYED_SORTED ? loft_sorted_new(cx, c, size)
                 : kind == LOFT_KEYED_HASH ? loft_hash_entry(cx, c, size, T[etp].linked)
                                           : loft_entry_new(cx, c, size);
    /* A claim is not zeroed (`@FR-H-Claim`), and `loft_key_set` releases the text a key field
     * holds: a text key field starts empty here, so finishing an element whose key nothing
     * wrote (a subscript entry, `c["k"].n += 1`) releases nothing, while one a literal wrote is
     * released as `keys::set_key` releases it. */
    loft_key_texts_clear(cx, e, keys, nk);
    return e;
}

/* The second half (Stores::keyed_place_finish): the subscript's key written into the claimed
 * element and the element filed, as `loft_set_keyed` files its copy. */
static void loft_keyed_place_finish(loft_cx *cx, const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref e, int kind,
                                    const loft_key *keys, int nk, const loft_kv *sub, int nsub) {
    if (e.rec == 0) return;
    uint32_t size = T[etp].size;
    for (int i = 0; i < nsub; i++) loft_key_set(cx, e, &keys[i], sub[i]);
    if (kind == LOFT_KEYED_HASH) (void) loft_hash_insert(cx, T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SORTED) loft_sorted_finish(cx, c, size, keys, nk);
    else if (kind == LOFT_KEYED_INDEX) (void) loft_ix_insert(cx, T, etp, c, e, keys, nk, 0);
    else if (kind == LOFT_KEYED_SPATIAL || kind == LOFT_KEYED_TRIE)
        (void) loft_rx_insert_record(cx, T, etp, c, e, keys, nk, kind == LOFT_KEYED_TRIE, 0);
    else (void) loft_ordered_insert(cx, T, etp, c, e, keys, nk, 0);
}

/* `@FR-R-KeyedEntry`'s subscript clause (Stores::keyed_entry_ints): the entry `c[ks…] = T{…}`
 * writes its non-key fields into — found or filed in one probe (`loft_h_entry_ints`), its key
 * parts written.  Where that walk cannot serve (a walk over `c` is live, no table yet) the entry
 * is claimed, keyed and filed through `loft_keyed_place_begin` / `_finish` first, and the filed
 * place answered: an inline table moves the staged entry into itself. */
static loft_ref loft_keyed_entry_ints(loft_cx *cx, const loft_tdesc *T, uint16_t etp, loft_ref c, const loft_key *keys, int nk,
                                      const int64_t *ks) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    /* A locked collection files nothing: the entry is the one already there, or the null record
     * whose writes are skipped (`@FR-H-WriteLocked`). */
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return loft_h_find_ints(cx, c, keys, nk, ks);
    int live = 0;
    for (int i = 0; i < cx->nscratches && !live; i++)
        live = cx->scratches[i].source.store_nr == c.store_nr && cx->scratches[i].source.rec == c.rec &&
               cx->scratches[i].source.pos == c.pos;
    if (!live) {
        loft_ref e = loft_h_entry_ints(cx, c, keys, nk, ks);
#ifdef LOFT_KEYED_VERIFY
        /* `keyed_entry_long`'s check: the entry the one probe answers is the one the key finds. */
        if (e.rec != 0) {
            loft_kv vk[4];
            for (int i = 0; i < nk && i < 4; i++) vk[i] = loft_kv_int(cx, ks[i]);
            loft_ref found = loft_h_find(cx, c, keys, vk, nk < 4 ? nk : 4);
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
    for (int i = 0; i < nk && i < 4; i++) kv[i] = loft_kv_int(cx, ks[i]);
    loft_ref e = loft_keyed_place_begin(cx, T, etp, c, LOFT_KEYED_HASH, keys, nk, kv, nk);
    if (e.rec == 0) return e;
    loft_st_fresh(cx, loft_store_of(cx, e), e.rec, e.pos, T[etp].size);
        loft_key_texts_clear(cx, e, keys, nk);
    loft_keyed_place_finish(cx, T, etp, c, e, LOFT_KEYED_HASH, keys, nk, kv, nk);
    return loft_h_find_ints(cx, c, keys, nk, ks);
}

/* `@FR-R-KeyedEntry`'s text clause (Stores::keyed_entry_text): `loft_keyed_entry_ints` for a hash
 * keyed by one text — the entry under `key`, found or filed with the key written, in one probe
 * where the table allows (`loft_h_entry_text`), else claimed, keyed and filed first. */
static loft_ref loft_keyed_entry_text(loft_cx *cx, const loft_tdesc *T, uint16_t etp, loft_ref c, const loft_key *keys, int nk,
                                      loft_str key) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null(cx);
    if (LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return loft_h_find_text(cx, c, keys, key);
    int live = 0;
    for (int i = 0; i < cx->nscratches && !live; i++)
        live = cx->scratches[i].source.store_nr == c.store_nr && cx->scratches[i].source.rec == c.rec &&
               cx->scratches[i].source.pos == c.pos;
    if (!live) {
        loft_ref e = loft_h_entry_text(cx, c, keys, key);
#ifdef LOFT_KEYED_VERIFY
        if (e.rec != 0) {
            loft_kv vk = loft_kv_text(cx, key);
            loft_ref found = loft_h_find(cx, c, keys, &vk, 1);
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
    loft_text own = loft_text_own(cx, key);
    loft_kv kv = loft_kv_text(cx, loft_view(cx, &own));
    loft_ref e = loft_keyed_place_begin(cx, T, etp, c, LOFT_KEYED_HASH, keys, nk, &kv, 1);
    if (e.rec != 0) {
        loft_st_fresh(cx, loft_store_of(cx, e), e.rec, e.pos, T[etp].size);
        loft_key_texts_clear(cx, e, keys, nk);
        loft_keyed_place_finish(cx, T, etp, c, e, LOFT_KEYED_HASH, keys, nk, &kv, 1);
        e = loft_h_find_text(cx, c, keys, loft_view(cx, &own));
    }
    loft_text_free(cx, &own);
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
static int loft_rotate_into(loft_cx *cx, loft_ref vec, loft_ref src, uint32_t size, int64_t k, const uint32_t *plan, int count) {
    if (k < 1 || size > 64 || count > 8 || vec.store_nr == 0xFFFF || vec.rec == 0 || src.store_nr == 0xFFFF ||
        src.rec == 0 || src.rec >= loft_store_of(cx, src)->size)
        return 0;
    loft_store_t *vs = loft_store_of(cx, vec);
    if (vs->read_only) return 0;
    uint32_t vrec = loft_collection_rec(cx, vs, vec.rec, vec.pos);
    if (vrec == 0 || vrec >= vs->size || (int64_t) loft_u32(cx, vs, vrec, 4) != k) return 0;
    if (src.store_nr == vec.store_nr && src.rec == vrec) return 0;
    uint32_t held[8] = { 0 };
    for (int i = 0; i < count; i++) {
        uint32_t cur = loft_u32(cx, vs, vrec, 8 + plan[2 * i]);
        if (cur == 0 || cur == 0xFFFFFFFFu) continue;
        if (cur >= vs->size || loft_bits_contains(cx, &vs->referenced, cur)) return 0;
        held[i] = cur;
    }
    memmove(loft_at(cx, vs, vrec, 8), loft_at(cx, loft_store_of(cx, src), src.rec, src.pos), size);
    for (int i = 0; i < count; i++) {
        uint32_t off = plan[2 * i], esize = plan[2 * i + 1];
        loft_store_t *ss = loft_store_of(cx, src);
        uint32_t from = loft_u32(cx, ss, src.rec, src.pos + off), b = held[i];
        if (from == 0 || from == 0xFFFFFFFFu) {
            if (b != 0) loft_st_delete(cx, vs, b);
            loft_set_u32(cx, vs, vrec, 8 + off, from);
            continue;
        }
        uint32_t length = loft_u32(cx, ss, from, 4);
        uint32_t words = 1 + (uint32_t) (((uint64_t) esize * length + 7) / 8);
        if (b == 0) b = loft_st_claim(cx, vs, words);
        else if (loft_rec_words(cx, vs, b) < words) b = loft_st_resize(cx, vs, b, words);
        loft_set_u32(cx, vs, vrec, 8 + off, b);
        ss = loft_store_of(cx, src);
        memmove(loft_at(cx, vs, b, 4), loft_at(cx, ss, from, 4), (size_t) length * esize + 4);
    }
    /* A claim or a resize above may have moved the store: the base is read after them. */
    uint8_t *base = loft_at(cx, vs, vrec, 8), tmp[64];
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
static inline double loft_ap_get(loft_cx *cx, const double *a, int64_t n, int64_t i) {
    int64_t f = i < 0 ? i + n : i;
    return f < 0 || f >= n ? (double) NAN : a[f];
}

static inline void loft_ap_set(loft_cx *cx, double *a, int64_t n, int64_t i, double x) {
    int64_t f = i < 0 ? i + n : i;
    if (f >= 0 && f < n) a[f] = x;
}

/* The BOUNDARY: the value written into the return buffer `buf` with the steps its constructor
 * runs — a buffer that holds no record minted (`bytes`, type `tp`), then each `vector<float>`
 * field at `offs[k]` emptied in place and its `lens[k]` elements appended.  Answers the buffer,
 * which is what the unrewritten function returns. */
static loft_ref loft_ap_store(loft_cx *cx, loft_ref buf, uint32_t bytes, uint16_t tp, int nf, const uint32_t *offs,
                              const double *const *vals, const uint32_t *lens) {
    if (buf.store_nr == 0xFFFF || buf.rec == 0) {
        buf = loft_ref_null(cx);
        loft_database_typed(cx, &buf, bytes, tp);
        for (int k = 0; k < nf; k++) loft_set_u32(cx, loft_store_of(cx, buf), buf.rec, buf.pos + offs[k], 0);
    }
    for (int k = 0; k < nf; k++) {
        loft_ref fld = loft_field(cx, buf, offs[k]);
        loft_vec_clear(cx, fld);
        loft_vec_prealloc(cx, fld, lens[k], 8);
        for (uint32_t i = 0; i < lens[k]; i++) loft_vec_push_float(cx, fld, vals[k][i]);
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

static inline void loft_apv_clear(loft_cx *cx, loft_apvec *v) {
    v->n = 0;
    v->spill.len = 0;
}

static inline void loft_apv_push(loft_cx *cx, loft_apvec *v, double x) {
    if (v->n < LOFT_APV_INLINE) v->d[v->n] = x;
    else loft_push(cx, &v->spill, (const char *) &x, sizeof x);
    v->n++;
}

/* The element `i` names (from the end when negative), or -1 out of range. */
static inline int64_t loft_apv_index(loft_cx *cx, const loft_apvec *v, int64_t i) {
    int64_t f = i < 0 ? i + (int64_t) v->n : i;
    return f < 0 || (uint64_t) f >= (uint64_t) v->n ? -1 : f;
}

static inline double *loft_apv_at(loft_cx *cx, loft_apvec *v, int64_t f) {
    return f < LOFT_APV_INLINE ? &v->d[f] : (double *) (void *) (v->spill.p + (size_t) (f - LOFT_APV_INLINE) * sizeof(double));
}

static inline double loft_apv_get(loft_cx *cx, loft_apvec *v, int64_t i) {
    int64_t f = loft_apv_index(cx, v, i);
    if (f < 0) return (double) NAN;
    double x;
    memcpy(&x, loft_apv_at(cx, v, f), sizeof x);
    return x;
}

static inline void loft_apv_set(loft_cx *cx, loft_apvec *v, int64_t i, double x) {
    int64_t f = loft_apv_index(cx, v, i);
    if (f >= 0) memcpy(loft_apv_at(cx, v, f), &x, sizeof x);
}

/* `v[i]` of a vector held apart (`@FR-E-Report`): an index naming no element is reported. */
static inline double loft_apv_get_at(loft_cx *cx, loft_apvec *v, int64_t i, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(loft_apv_index(cx, v, i) < 0)) loft_index_miss(cx, i, (int64_t) v->n, file, line);
    return loft_apv_get(cx, v, i);
}
static inline void loft_apv_set_at(loft_cx *cx, loft_apvec *v, int64_t i, double x, const char *file, uint32_t line) {
    if (LOFT_UNLIKELY(loft_apv_index(cx, v, i) < 0)) loft_index_miss(cx, i, (int64_t) v->n, file, line);
    loft_apv_set(cx, v, i, x);
}

/* `@FR-R-Apart` instance 4 (codegen_runtime::AprVec): one field of a record of record vectors
 * held apart is the bytes of its elements, `esize` each, in the element's store layout — a text
 * buffer, so the frame's epilogue frees it with its texts.  Clearing keeps the capacity: a value
 * rebuilt every iteration allocates nothing once it has grown.  A pushed element starts as zeros
 * only so that its padding is defined; the plan admits a push only when every field is written.
 * Answers the element's byte offset. */
static size_t loft_apr_push(loft_cx *cx, loft_text *v, uint32_t esize) {
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
static uint32_t loft_apart_block(loft_cx *cx, loft_store_t *s, uint32_t reuse, const char *bytes, size_t len, uint32_t n) {
    uint32_t words = 1 + (uint32_t) ((len + 7) / 8);
    uint32_t b = reuse == 0 ? loft_st_claim(cx, s, words)
                 : loft_rec_words(cx, s, reuse) < words ? loft_st_resize(cx, s, reuse, words) : reuse;
    loft_set_u32(cx, s, b, 4, n);
    memcpy(loft_at(cx, s, b, 8), bytes, len);
    return b;
}

/* `@FR-R-Apart` instance 4's boundary (Stores::apart_write_fresh): the apart record's `nf` fields
 * written into the FRESH element `dst` — per field a block of exactly its elements claimed in
 * `dst`'s store with the bytes copied in, its number written at the field's slot; an empty field
 * writes the empty vector (0).  A null destination is left as the store path's copy leaves it,
 * and a locked store refuses the write (`@FR-H-WriteLocked`). */
static void loft_apart_write_fresh(loft_cx *cx, loft_ref dst, const loft_text *f, const uint32_t *slot, const uint32_t *esize, int nf) {
    if (dst.store_nr == 0xFFFF || dst.rec == 0 || dst.store_nr >= cx->nstores) return;
    loft_store_t *s = loft_store_of(cx, dst);
    if (LOFT_WRITE_REFUSED(s)) return;
    for (int k = 0; k < nf; k++) {
        uint32_t n = (uint32_t) (f[k].len / esize[k]);
        uint32_t b = n == 0 ? 0 : loft_apart_block(cx, s, 0, f[k].p, f[k].len, n);
        loft_set_u32(cx, s, dst.rec, dst.pos + slot[k], b);
    }
}

/* `@FR-R-HandOver`'s rotate clause from an apart record (Stores::rotate_from_parts): as
 * `loft_rotate_into`, the new element's vectors given as the apart fields rather than read from
 * a record — each written into the block element 0 owns (grown where too small, released when
 * the field is empty), then the slots rotated left by one.  0, with nothing changed, on the
 * tests `loft_rotate_into` makes, or when the fields are not the element type's plain vectors
 * (`plan`, `count` pairs of offset and element bytes). */
static int loft_rotate_from_parts(loft_cx *cx, loft_ref vec, const loft_text *f, const uint32_t *slot, int nf, uint32_t size,
                                  int64_t k, const uint32_t *plan, int count) {
    if (k < 1 || size > 64 || count > 8 || count != nf || vec.store_nr == 0xFFFF || vec.rec == 0) return 0;
    loft_store_t *vs = loft_store_of(cx, vec);
    if (vs->read_only) return 0;
#ifdef LOFT_FOREIGN
    if (loft_st_is_foreign(cx, vs)) return 0;
#endif
    uint32_t vrec = loft_collection_rec(cx, vs, vec.rec, vec.pos);
    if (vrec == 0 || vrec >= vs->size || (int64_t) loft_u32(cx, vs, vrec, 4) != k) return 0;
    for (int i = 0; i < count; i++)
        if (plan[2 * i] != slot[i] || f[i].len % plan[2 * i + 1] != 0) return 0;
    uint32_t held[8] = { 0 };
    for (int i = 0; i < count; i++) {
        uint32_t cur = loft_u32(cx, vs, vrec, 8 + plan[2 * i]);
        if (cur == 0 || cur == 0xFFFFFFFFu) continue;
        if (cur >= vs->size || loft_bits_contains(cx, &vs->referenced, cur)) return 0;
        held[i] = cur;
    }
    for (int i = 0; i < count; i++) {
        uint32_t n = (uint32_t) (f[i].len / plan[2 * i + 1]);
        if (n == 0) {
            if (held[i] != 0) loft_st_delete(cx, vs, held[i]);
            loft_set_u32(cx, vs, vrec, 8 + slot[i], 0);
            continue;
        }
        uint32_t b = loft_apart_block(cx, vs, held[i], f[i].p, f[i].len, n);
        loft_set_u32(cx, vs, vrec, 8 + slot[i], b);
    }
    /* A claim or a resize above may have moved the store: the base is read after them. */
    uint8_t *base = loft_at(cx, vs, vrec, 8), tmp[64];
    memcpy(tmp, base, size);
    memmove(base, base + size, (size_t) (k - 1) * size);
    memcpy(base + (size_t) (k - 1) * size, tmp, size);
    return 1;
}

/* OpFillKeyed (Stores::fill_keyed_from_vector): each element of the vector `src` filed into the
 * keyed collection `c` as a deep copy under its own key (`insert_keyed_copy`, the latest of one
 * key winning); a null linked element has no key and stays out.  Every element is resolved
 * before the first insert, which may grow a store they share.  A source the bind gives up is
 * freed.  `keyed_src`: the source is a keyed collection of `c`'s own type, and a kind whose
 * storage is no sequential list (a hash, an index, a spatial, a trie) is listed by its walk. */
static void loft_fill_keyed(loft_cx *cx, const loft_tdesc *T, uint16_t etp, loft_ref c, loft_ref src, int kind,
                            const loft_key *keys, int nk, int linked, int free_source, const loft_group *g,
                            int keyed_src) {
    if (src.store_nr == 0xFFFF || src.store_nr >= cx->nstores) return;
    if (keyed_src && kind != LOFT_KEYED_SORTED && kind != LOFT_KEYED_ORDERED) {
        uint32_t m = 0;
        loft_ref *e = loft_keyed_refs(cx, src, kind == LOFT_KEYED_HASH ? 1 : kind == LOFT_KEYED_INDEX ? 2 : 3, keys, nk, &m);
        for (uint32_t i = 0; i < m; i++) loft_set_keyed(cx, T, etp, c, e[i], kind, keys, nk, 0, 0, 0, g);
        free(e);
        if (free_source && src.store_nr != c.store_nr && !loft_free_protected(cx, src)) loft_free_ref(cx, src);
        return;
    }
    int64_t len = loft_vec_len(cx, src);
    uint32_t width = linked ? 4u : T[etp].size;
    loft_ref *recs = (loft_ref *) malloc(sizeof(loft_ref) * (size_t) (len > 0 ? len : 1));
    if (!recs) { fputs("loft: out of memory\n", stderr); abort(); }
    int64_t n = 0;
    for (int64_t i = 0; i < len; i++) {
        loft_ref slot = loft_vec_get(cx, src, width, i);
        if (slot.rec == 0) continue;
        if (linked) {
            uint32_t rec = loft_u32(cx, loft_store_of(cx, slot), slot.rec, slot.pos);
            if (rec == 0) continue;
            loft_ref r = { src.store_nr, rec, 8, 0 };
            recs[n++] = r;
        } else {
            recs[n++] = slot;
        }
    }
    for (int64_t i = 0; i < n; i++) loft_set_keyed(cx, T, etp, c, recs[i], kind, keys, nk, 0, 0, 0, g);
    free(recs);
    if (free_source && src.store_nr != c.store_nr && !loft_free_protected(cx, src)) loft_free_ref(cx, src);
}

/* OpRemove (`e#remove`) in a walk over a sorted (`size` its element) or an index (`size` 0):
 * the element the walk visited last goes, and the walk goes on with the one after it. */
static void loft_seq_remove(loft_cx *cx, int64_t *state, loft_ref c, uint32_t size, int reverse) {
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
    if (!loft_vec_remove(cx, c, size ? size : 4, at)) return;
    *state = loft_iter_range(cx, lo, hi);
}

/* The position a walk's removal names: the element it visited last (loft_seq_remove's). */
static int loft_walk_removed_at(loft_cx *cx, int64_t state, int reverse, uint32_t *at) {
    uint64_t st = (uint64_t) state;
    uint32_t lo = (uint32_t) st, hi = (uint32_t) (st >> 32);
    if (!reverse && lo == 0) return 0;
    *at = reverse ? hi : lo - 1;
    return 1;
}

/* OpRefFromChildRec (Stores::ref_from_child_rec): the child record field `f` names, or null. */
static loft_ref loft_ref_from_child_rec(loft_cx *cx, loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return loft_ref_null(cx);
    uint32_t rec = loft_u32(cx, loft_store_of(cx, f), f.rec, f.pos);
    if (rec == 0) return loft_ref_null(cx);
    loft_ref r = { f.store_nr, rec, 8, 0 };
    return r;
}

/* @PLN219 slot cells (keys::SLOT_FIELD_BIT): a `&S` parameter whose `S` is held out of line points
 * at a cell holding a view, or the address of an out-of-line field with this bit in its position. */
#define LOFT_SLOT_FIELD_BIT 0x40000000u
static inline int loft_slot_is_field(loft_cx *cx, loft_ref c) {
    return c.store_nr != 0xFFFF && (c.pos & LOFT_SLOT_FIELD_BIT) != 0;
}
/* OpSlotTag (keys::slot_tag): the cell content naming field `f`; null for an absent field. */
static inline loft_ref loft_slot_tag(loft_cx *cx, loft_ref f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return loft_ref_null(cx);
    f.pos |= LOFT_SLOT_FIELD_BIT;
    return f;
}
/* Stores::slot_read: the child a field-naming cell holds, else the cell's own view. */
static inline loft_ref loft_slot_read(loft_cx *cx, loft_ref c) {
    if (!loft_slot_is_field(cx, c)) return c;
    c.pos &= ~LOFT_SLOT_FIELD_BIT;
    return loft_ref_from_child_rec(cx, c);
}

/* OpChildRec (Stores::new_child_rec): a fresh `tp` child record in field `f`'s store, filled with
 * its defaults and named by the field; a record the field held before is released as this
 * store's bytes.  The claim is its header word and payload (`Stores::child_rec_words`). */
static loft_ref loft_new_child_rec(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref f, const uint8_t *image,
                                   uint32_t n) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return loft_ref_null(cx);
    uint32_t old = loft_u32(cx, loft_store_of(cx, f), f.rec, f.pos);
    if (old != 0) {
        loft_ref o = { f.store_nr, old, 8, 0 };
        loft_remove_claims(cx, T, tp, o);
        loft_st_delete(cx, loft_store_of(cx, f), old);
    }
    uint32_t rec = loft_claim(cx, loft_store_of(cx, f), 1 + (T[tp].size + 7) / 8);
    loft_ref db = { f.store_nr, rec, 8, 0 };
    loft_fill(cx, db, image, n);
    loft_set_u32(cx, loft_store_of(cx, f), f.rec, f.pos, rec);
    return db;
}

/* OpCopyClosure (Stores::copy_closure_record, `src/database/closure_share.rs`): a fn-ref read out of
 * a struct field and kept, as a copy of the `tp` closure record at `src` in a fresh store.  Every
 * capture is shared; each store an OWNED slot (`owned`, the byte positions the generator listed
 * from `Stores::owned_capture_slots`) names gains one share, so the copy and its source each
 * release it once.  The only writer of `closure_shares` that raises it. */
static loft_ref loft_copy_closure(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref src, const uint16_t *owned,
                                  uint32_t n) {
    if (src.store_nr == 0xFFFF || src.rec == 0) return loft_ref_null(cx);
    loft_ref to = loft_ref_null(cx);
    loft_database_typed(cx, &to, T[tp].size, tp);
    loft_copy_bytes(cx, to, 0, src, 0, T[tp].size);
    loft_copy_claims(cx, T, tp, src, to);
    for (uint32_t i = 0; i < n; i++) {
        loft_ref held = loft_get_dbref(cx, to, owned[i]);
        if (held.store_nr != 0xFFFF && held.rec != 0 && held.store_nr < cx->nstores
            && cx->stores[held.store_nr].live)
            cx->stores[held.store_nr].closure_shares++;
    }
    return to;
}

/* OpSetChildRec (Stores::set_child_rec, `@FR-B-OutOfLine`): write the `tp` value `src` into the
 * out-of-line field `f`.  A null releases the child; a value is copied into a fresh record of the
 * host's store first (it may lie inside the child it replaces), which an absent field takes and
 * a present child takes in place, keeping its record number.  `free_source` releases a given-up
 * source store, as loft_copy_record's does. */
static void loft_set_child_rec(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref f, loft_ref src, int free_source) {
    int present = src.store_nr != 0xFFFF && src.rec != 0;
    if (f.store_nr == 0xFFFF || f.rec == 0) {
        if (free_source && present && src.store_nr != f.store_nr && !loft_free_protected(cx, src)) loft_free_ref(cx, src);
        return;
    }
    uint32_t old = loft_u32(cx, loft_store_of(cx, f), f.rec, f.pos);
    if (!present) {
        if (old != 0) {
            loft_set_u32(cx, loft_store_of(cx, f), f.rec, f.pos, 0);
            loft_ref o = { f.store_nr, old, 8, 0 };
            loft_remove_claims(cx, T, tp, o);
            loft_st_delete(cx, loft_store_of(cx, f), old);
        }
        return;
    }
    /* The child the field already holds, written back (`t.left = insert(t.left, k)` handing its
     * argument back): the field already says it, so nothing is copied or released. */
    if (old != 0 && src.store_nr == f.store_nr && src.rec == old && src.pos == 8) return;
    uint32_t size = T[tp].size;
    uint32_t fresh = loft_claim(cx, loft_store_of(cx, f), 1 + (size + 7) / 8);
    loft_ref nw = { f.store_nr, fresh, 8, 0 };
    loft_copy_bytes(cx, nw, 0, src, 0, size);
    loft_copy_claims(cx, T, tp, src, nw);
    if (old == 0) {
        loft_set_u32(cx, loft_store_of(cx, f), f.rec, f.pos, fresh);
    } else {
        loft_ref o = { f.store_nr, old, 8, 0 };
        loft_remove_claims(cx, T, tp, o);
        loft_copy_bytes(cx, o, 0, nw, 0, size);
        loft_st_delete(cx, loft_store_of(cx, f), fresh);
    }
    if (free_source && src.store_nr != f.store_nr && !loft_free_protected(cx, src)) loft_free_ref(cx, src);
}

/* OpDemoteRec (Stores::demote_rec, `@FR-R-Demote`): `r`'s record bytes move into a fresh child of
 * r's store (its handles with them), r's record is zeroed and its field at `fld` names the child. */
static void loft_demote_rec(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, uint32_t fld) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    uint32_t size = T[tp].size;
    loft_store_t *s = loft_store_of(cx, r);
    uint32_t c = loft_claim(cx, s, 1 + (size + 7) / 8);
    loft_ref child = { r.store_nr, c, 8, 0 };
    loft_copy_bytes(cx, child, 0, r, 0, size);
    if (size) memset(loft_at(cx, s, r.rec, r.pos), 0, size);
    loft_set_u32(cx, s, r.rec, r.pos + fld, c);
}

/* OpStoreBuf (keys::store_buf): the buffer naming host's store and no record. */
static inline loft_ref loft_store_buf(loft_cx *cx, loft_ref host) {
    if (host.store_nr == 0xFFFF) return loft_ref_null(cx);
    loft_ref b = { host.store_nr, 0, LOFT_STORE_BUF_BIT, 0 };
    return b;
}
/* OpAdoptChildRec (Stores::adopt_child_rec): a record of f's store adopted by number. */
static void loft_adopt_child_rec(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref f, loft_ref v) {
    if (f.store_nr == 0xFFFF || f.rec == 0 || v.store_nr != f.store_nr || v.rec == 0 || v.pos != 8) {
        loft_set_child_rec(cx, T, tp, f, v, 0);
        return;
    }
    uint32_t old = loft_u32(cx, loft_store_of(cx, f), f.rec, f.pos);
    if (old == v.rec) return;
    loft_set_u32(cx, loft_store_of(cx, f), f.rec, f.pos, v.rec);
    if (old) {
        loft_ref o = { f.store_nr, old, 8, 0 };
        loft_remove_claims(cx, T, tp, o);
        loft_st_delete(cx, loft_store_of(cx, f), old);
    }
}
/* OpStoreBufOnly (Stores::store_buf_only): buf when it names a store and no record, else null. */
static inline loft_ref loft_store_buf_only(loft_cx *cx, loft_ref buf) {
    if (buf.store_nr != 0xFFFF && buf.rec == 0 && (buf.pos & LOFT_STORE_BUF_BIT)) return buf;
    return loft_ref_null(cx);
}
/* OpReleaseRecIfDistinct (Stores::release_rec_if_distinct): a dropped result — its own store
 * freed whole, a record of the answer's store released with what it owns. */
static void loft_release_rec_if_distinct(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref r, loft_ref ans) {
    if (r.store_nr == 0xFFFF || r.rec == 0) return;
    if (r.store_nr != ans.store_nr) { loft_free_displaced(cx, r, ans); return; }
    if (r.rec == ans.rec) return;
    loft_remove_claims(cx, T, tp, r);
    loft_st_delete(cx, loft_store_of(cx, r), r.rec);
}

/* OpMoveChildRec (Stores::move_child_rec): `f` takes the child `src_f` names (a field inside the
 * record f holds) by number; src_f is cleared and the replaced record released with what it owns. */
static void loft_move_child_rec(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref f, loft_ref src_f) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return;
    uint32_t old = loft_u32(cx, loft_store_of(cx, f), f.rec, f.pos);
    uint32_t moved = (src_f.store_nr == 0xFFFF || src_f.rec == 0) ? 0 : loft_u32(cx, loft_store_of(cx, src_f), src_f.rec, src_f.pos);
    if (moved) loft_set_u32(cx, loft_store_of(cx, src_f), src_f.rec, src_f.pos, 0);
    loft_set_u32(cx, loft_store_of(cx, f), f.rec, f.pos, moved);
    if (old && old != moved) {
        loft_ref o = { f.store_nr, old, 8, 0 };
        loft_remove_claims(cx, T, tp, o);
        loft_st_delete(cx, loft_store_of(cx, f), old);
    }
}

/* OpChildRecInPlace (Stores::child_rec_in_place): the present child, its claims released and its
 * bytes the type's defaults (number kept), else a fresh child where the field is absent. */
static loft_ref loft_child_rec_in_place(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref f, const uint8_t *image,
                                        uint32_t n) {
    if (f.store_nr == 0xFFFF || f.rec == 0) return loft_ref_null(cx);
    uint32_t old = loft_u32(cx, loft_store_of(cx, f), f.rec, f.pos);
    if (old == 0) return loft_new_child_rec(cx, T, tp, f, image, n);
    loft_ref o = { f.store_nr, old, 8, 0 };
    loft_remove_claims(cx, T, tp, o);
    loft_fill(cx, o, image, n);
    return o;
}

/* Stores::slot_move (@PLN219): `slot = slot.f` — a field-naming cell takes the child by number,
 * any other cell the child's view. */
static void loft_slot_move(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref *cell, loft_ref src_f) {
    if (loft_slot_is_field(cx, *cell)) {
        loft_ref f = *cell;
        f.pos &= ~LOFT_SLOT_FIELD_BIT;
        loft_move_child_rec(cx, T, tp, f, src_f);
    } else {
        loft_ref old = *cell;
        *cell = loft_ref_from_child_rec(cx, src_f);
        loft_free_displaced(cx, old, *cell);
    }
}

/* Stores::slot_write (@PLN219): a whole write through a slot cell — the field write when the cell
 * names a field (its free bit giving up an owned value's store), else the value written back. */
static void loft_slot_write(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref *cell, loft_ref v, int free_source) {
    if (loft_slot_is_field(cx, *cell)) {
        loft_ref f = *cell;
        f.pos &= ~LOFT_SLOT_FIELD_BIT;
        loft_set_child_rec(cx, T, tp, f, v, free_source);
    } else {
        *cell = v;
    }
}

/* Stores::vector_add_array: `v += o` of a linked element type, whose slots are record ids — each
 * element record copied into a fresh record of the destination's store, its heap deep-copied, a
 * null slot kept null. */
static void loft_vec_add_array(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
    int64_t n = loft_vec_len(cx, o);
    if (n == 0 || v.store_nr == 0xFFFF || v.rec == 0) return;
    uint32_t size = T[tp].size, words = 1 + (size + 7) / 8;
    uint32_t o_rec = loft_u32(cx, loft_store_of(cx, o), o.rec, o.pos);
    if (o_rec == 0) return;
    for (int64_t i = 0; i < n; i++) {
        uint32_t src = loft_u32(cx, loft_store_of(cx, o), o_rec, 8 + 4 * (uint32_t) i);
        loft_ref slot = loft_vec_append(cx, v, 4);
        if (src == 0) {
            loft_set_u32(cx, loft_store_of(cx, v), slot.rec, slot.pos, 0);
        } else {
            uint32_t nr = loft_claim(cx, loft_store_of(cx, v), words);
            loft_ref from = { o.store_nr, src, 8, 0 }, to = { v.store_nr, nr, 8, 0 };
            loft_copy_bytes(cx, to, 0, from, 0, size);
            loft_copy_claims(cx, T, tp, from, to);
            loft_set_u32(cx, loft_store_of(cx, v), slot.rec, slot.pos, nr);
        }
        loft_vec_finish(cx, v);
    }
}

/* OpReplaceVector of a linked element type (Stores::vector_replace): an absent source leaves the
 * destination absent; else the destination emptied — an array's clear is the plain reset — and
 * the source's records copied in as `loft_vec_add_array` copies them. */
static void loft_vec_replace_array(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, loft_ref o) {
    int absent = o.store_nr == 0xFFFF
        || (o.rec != 0 && o.pos != 0 && loft_u32(cx, loft_store_of(cx, o), o.rec, o.pos) == 0xFFFFFFFFu);
    if (absent) {
        loft_vec_clear(cx, v);
        if (v.store_nr != 0xFFFF && v.rec != 0 && v.pos != 0) loft_set_u32(cx, loft_store_of(cx, v), v.rec, v.pos, 0xFFFFFFFFu);
        return;
    }
    if (v.store_nr == o.store_nr) {
        uint32_t dr = loft_u32(cx, loft_store_of(cx, v), v.rec, v.pos);
        if (dr != 0 && dr == loft_u32(cx, loft_store_of(cx, o), o.rec, o.pos)) return;
    }
    loft_vec_clear(cx, v);
    loft_vec_add_array(cx, T, tp, v, o);
}

/* OpMoveVector of a linked element type (Stores::move_vector): the slot list handed over within
 * one store into an empty destination, else its records copied. */
static void loft_move_vector_array(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref dst, loft_ref src) {
    if (src.store_nr == 0xFFFF || src.rec == 0 || dst.store_nr == 0xFFFF || dst.rec == 0) return;
    uint32_t h = loft_collection_rec(cx, loft_store_of(cx, src), src.rec, src.pos);
    if (h == 0) return;
    if (src.store_nr == dst.store_nr && loft_collection_rec(cx, loft_store_of(cx, dst), dst.rec, dst.pos) == 0) {
        loft_set_u32(cx, loft_store_of(cx, dst), dst.rec, dst.pos, h);
        loft_set_u32(cx, loft_store_of(cx, src), src.rec, src.pos, 0);
        return;
    }
    loft_vec_add_array(cx, T, tp, dst, src);
}

/* Stores::insert_vector_element of a linked element type: a record-id slot inserted at `index`,
 * and a fresh record of `size` bytes claimed for the element, the vector's record named at its
 * byte 4; the element (its defaults the caller writes), or null for an index outside 0..=len. */
static loft_ref loft_vec_insert_linked(loft_cx *cx, loft_ref v, uint32_t size, int64_t index) {
    loft_ref slot = loft_vec_insert(cx, v, 4, index);
    if (slot.rec == 0) return slot;
    loft_store_t *s = loft_store_of(cx, v);
    uint32_t nr = loft_claim(cx, s, 1 + (size + 7) / 8);
    loft_set_u32(cx, s, nr, 4, v.rec);
    loft_set_u32(cx, s, slot.rec, slot.pos, nr);
    loft_ref e = { v.store_nr, nr, 8, 0 };
    return e;
}

/* Stores::remove_vector_at of a linked element type: the slot at `index` unlinked, then the
 * record it named released and deleted — the order `remove_owned` uses, since the release reads
 * the record's own fields. */
static int loft_vec_remove_linked(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref v, int64_t index) {
    loft_ref slot = loft_vec_get(cx, v, 4, index);
    if (slot.rec == 0) return 0;
    uint32_t rec = loft_u32(cx, loft_store_of(cx, v), slot.rec, slot.pos);
    int shifted = loft_vec_remove(cx, v, 4, index);
    if (rec != 0) {
        loft_ref elem = { v.store_nr, rec, 8, 0 };
        loft_remove_claims(cx, T, tp, elem);
        loft_st_delete(cx, loft_store_of(cx, v), rec);
    }
    return shifted;
}

/* OpRemove in a walk over a vector (State::remove, Stores::remove_vector_at): the element's
 * claims released, then its successors shift down; forward, the index steps back to visit the
 * one that moved into its place. */
/* OpRemoveVector (`Stores::remove_vector_at`, an inline element): the element's claims are
 * released before the shift, since the slot it is is overwritten by its successor. */
static int loft_vec_remove_at(loft_cx *cx, const loft_tdesc *T, uint16_t elem, loft_ref v, int64_t index) {
    /* A refused removal releases nothing either: the element's claims stay with it. */
    if (v.store_nr != 0xFFFF && v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(cx, v))) return 0;
    loft_ref e = loft_vec_get(cx, v, T[elem].size, index);
    if (e.rec != 0) loft_remove_claims(cx, T, elem, e);
    return loft_vec_remove(cx, v, T[elem].size, index);
}

static void loft_vec_walk_remove(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref v, int reverse) {
    /* A refused removal leaves the cursor, so the walk moves past the element still there. */
    if (v.store_nr != 0xFFFF && v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(cx, v))) return;
    loft_ref e = loft_vec_get(cx, v, T[elem].size, *state);
    if (e.rec != 0) loft_remove_claims(cx, T, elem, e);
    (void) loft_vec_remove(cx, v, T[elem].size, *state);
    if (!reverse) *state = *state - 1;
}

/* @PLN217 — the same for a walk whose index is held in 32 bits (a loop's counter under the
 * 32-bit regime, `(W-Count)`): the index is handed over at 64 bits and taken back. */
static void loft_vec_walk_remove32(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int32_t *state, loft_ref v, int reverse) {
    int64_t wide = loft_w64(cx, *state);
    loft_vec_walk_remove(cx, T, elem, &wide, v, reverse);
    *state = loft_n32(cx, wide);
}

/* OpRemove in a walk over an array — a vector of a linked element type (Stores::remove_vector_at's
 * linked branch): the slot is removed, then the record it named releases its claims and goes. */
static void loft_array_walk_remove(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref v, int reverse) {
    /* A refused removal leaves the cursor, so the walk moves past the element still there. */
    if (v.store_nr != 0xFFFF && v.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(cx, v))) return;
    loft_ref slot = loft_vec_get(cx, v, 4, *state);
    uint32_t rec = slot.rec == 0 ? 0 : loft_u32(cx, loft_store_of(cx, slot), slot.rec, slot.pos);
    if (slot.rec == 0) return;
    (void) loft_vec_remove(cx, v, 4, *state);
    if (rec != 0) {
        loft_ref e = { v.store_nr, rec, 8, 0 };
        loft_remove_claims(cx, T, elem, e);
        loft_st_delete(cx, loft_store_of(cx, v), rec);
    }
    if (!reverse) *state = *state - 1;
}

static void loft_array_walk_remove32(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int32_t *state, loft_ref v, int reverse) {
    int64_t wide = loft_w64(cx, *state);
    loft_array_walk_remove(cx, T, elem, &wide, v, reverse);
    *state = loft_n32(cx, wide);
}

/* OpRemove in a walk over a sorted (Stores::remove_vector_at): the element's claims released,
 * then the element removed and the walk moved on. */
static void loft_sorted_walk_remove(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref c, int reverse) {
    if (c.store_nr != 0xFFFF && c.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return;
    uint32_t at, r = loft_keyed_rec(cx, c), size = T[elem].size;
    if (r == 0 || !loft_walk_removed_at(cx, *state, reverse, &at) || at >= loft_u32(cx, loft_store_of(cx, c), r, 4)) return;
    loft_ref e = { c.store_nr, r, 8 + at * size, 0 };
    loft_remove_claims(cx, T, elem, e);
    loft_seq_remove(cx, state, c, size, reverse);
}

/* Stores::remove_owned on a sorted: the element's claims released, then the element removed. */
static void loft_sorted_remove_owned(loft_cx *cx, const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, uint32_t size) {
    if (e.rec == 0) return;
    loft_remove_claims(cx, T, elem, e);
    loft_sorted_remove(cx, c, e, size);
}

/* OpRemove in a walk over an ordered (Stores::remove_vector_at, a linked element): the slot the
 * walk visited last is unlinked, then its record's claims and the record released. */
static void loft_ordered_walk_remove(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int64_t *state, loft_ref c, int reverse) {
    if (c.store_nr != 0xFFFF && c.rec != 0 && LOFT_WRITE_REFUSED(loft_store_of(cx, c))) return;
    uint32_t at, r = loft_keyed_rec(cx, c);
    if (r == 0 || !loft_walk_removed_at(cx, *state, reverse, &at) || at >= loft_u32(cx, loft_store_of(cx, c), r, 4)) return;
    uint32_t rec = loft_u32(cx, loft_store_of(cx, c), r, 8 + at * 4);
    loft_seq_remove(cx, state, c, 0, reverse);
    if (rec != 0) {
        loft_ref e = { c.store_nr, rec, 8, 0 };
        loft_remove_claims(cx, T, elem, e);
        loft_st_delete(cx, loft_store_of(cx, c), rec);
    }
}

/* ── arrays and linked groups (@FR-Col-Group) ──
 * An array is a vector of record numbers: the form a vector takes when its element type is
 * shared — by a stored reference or by several collections of one struct — so every route to
 * a record names the one record.  Its layout is an index's, unsorted. */

/* OpVectorRef / OpVectorRefNullable: the record the slot at `from` names (negative from the
 * end), or the null reference. */
static loft_ref loft_array_get(loft_cx *cx, loft_ref v, int64_t from) {
    loft_ref slot = loft_vec_get(cx, v, 4, from);
    if (slot.store_nr == 0xFFFF || slot.rec == 0) return loft_ref_null(cx);
    uint32_t r = loft_u32(cx, loft_store_of(cx, slot), slot.rec, slot.pos);
    if (r == 0) return loft_ref_null(cx);
    loft_ref e = { slot.store_nr, r, 8, 0 };
    return e;
}

/* `OpVectorRef` (`State::vec_ref_or_raise`): an index naming no slot is reported. */
static loft_ref loft_array_get_at(loft_cx *cx, loft_ref v, int64_t from, const char *file, uint32_t line) {
    int64_t len = loft_vec_len(cx, v), i = from < 0 ? from + len : from;
    if (LOFT_UNLIKELY(i < 0 || i >= len)) loft_index_miss(cx, from, len, file, line);
    return loft_array_get(cx, v, from);
}

/* insert_record on an array: the record's number joins the end. */
static void loft_array_append(loft_cx *cx, loft_ref v, loft_ref rec) {
    if (rec.rec == 0) return;
    loft_ref slot = loft_vec_append(cx, v, 4);
    if (slot.rec == 0) return;
    loft_set_u32(cx, loft_store_of(cx, slot), slot.rec, slot.pos, rec.rec);
    loft_vec_finish(cx, v);
}


/* Does keyed member `m` of `parent` file record `dr` under its own key? */
static int loft_member_files(loft_cx *cx, loft_ref parent, const loft_member *m, loft_ref dr) {
    if (m->kind >= LOFT_M_SPATIAL) return 0;
    loft_ref c = loft_field(cx, parent, m->pos);
    loft_kv key[8];
    int nk = m->nk > 8 ? 8 : m->nk;
    for (int k = 0; k < nk; k++) key[k] = loft_key_get(cx, loft_store_of(cx, dr), dr.rec, 8, &m->keys[k]);
    if (m->kind == LOFT_M_HASH) return loft_h_find(cx, c, m->keys, key, nk).rec == dr.rec;
    if (m->kind == LOFT_M_INDEX) return loft_ix_find(cx, c, m->keys, m->nk, key, nk).rec == dr.rec;
    return loft_seq_find(cx, c, 0, m->keys, m->nk, key, nk).rec == dr.rec;
}

/* The record a keyed member answers under record `dr`'s key, or 0. */
static uint32_t loft_member_answer(loft_cx *cx, loft_ref parent, const loft_member *m, loft_ref dr) {
    if (m->kind >= LOFT_M_SPATIAL) return 0;
    loft_ref c = loft_field(cx, parent, m->pos);
    loft_kv key[8];
    int nk = m->nk > 8 ? 8 : m->nk;
    for (int k = 0; k < nk; k++) key[k] = loft_key_get(cx, loft_store_of(cx, dr), dr.rec, 8, &m->keys[k]);
    if (m->kind == LOFT_M_HASH) return loft_h_find(cx, c, m->keys, key, nk).rec;
    if (m->kind == LOFT_M_INDEX) return loft_ix_find(cx, c, m->keys, m->nk, key, nk).rec;
    return loft_seq_find(cx, c, 0, m->keys, m->nk, key, nk).rec;
}

/* `@FR-Col-Group-Dup`: a displaced record leaves every keyed member that still files it under
 * its key, and every tree member; an array keeps it. */
static void loft_group_unfile(loft_cx *cx, loft_ref parent, loft_ref dr, const loft_member *m, int n) {
    for (int i = 0; i < n; i++) {
        if (m[i].kind >= LOFT_M_SPATIAL) {
            (void) loft_rx_unlink(cx, loft_field(cx, parent, m[i].pos), dr, m[i].keys, m[i].nk, m[i].kind == LOFT_M_TRIE);
            continue;
        }
        if (m[i].kind == LOFT_M_ARRAY || !loft_member_files(cx, parent, &m[i], dr)) continue;
        loft_ref c = loft_field(cx, parent, m[i].pos);
        if (m[i].kind == LOFT_M_HASH) (void) loft_hash_unlink(cx, c, dr, m[i].keys, m[i].nk);
        else if (m[i].kind == LOFT_M_INDEX) (void) loft_bt_remove(cx, c, dr.rec, m[i].keys, m[i].nk);
        else loft_index_remove(cx, c, dr);
    }
}

/* record_finish on a member of a linked group (`link_siblings`, `settle_displaced`): the record
 * enters every member; a record a key collision displaced from one keyed member leaves every
 * keyed member that still files it under its key, and is released unless a vector member of
 * the group (`held`) keeps it.  OpLinkRecord
 * (`link_record_siblings`) is this over the members beside the one that holds the record. */
static void loft_group_finish(loft_cx *cx, const loft_tdesc *T, uint16_t elem, int held, int nullable, loft_ref parent,
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
    int absent = nullable && *loft_at(cx, loft_store_of(cx, rec), rec.rec, rec.pos) != 2;
    for (int i = 0; i < n; i++) {
        loft_ref c = loft_field(cx, parent, m[i].pos);
        uint32_t d = 0;
        if (m[i].kind != LOFT_M_ARRAY && absent) continue;
        if (m[i].kind == LOFT_M_ARRAY) loft_array_append(cx, c, rec);
        else if (m[i].kind == LOFT_M_HASH) d = loft_hash_insert(cx, 0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        else if (m[i].kind == LOFT_M_INDEX) d = loft_ix_insert(cx, 0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        else if (m[i].kind >= LOFT_M_SPATIAL)
            d = loft_rx_insert_record(cx, 0, 0, c, rec, m[i].keys, m[i].nk, m[i].kind == LOFT_M_TRIE, 1).rec;
        else d = loft_ordered_insert(cx, 0, 0, c, rec, m[i].keys, m[i].nk, 1).rec;
        int seen = d == 0 || d == rec.rec;
        for (int j = 0; j < ng; j++) seen |= gone[j] == d;
        if (!seen) gone[ng++] = d;
    }
    for (int g = 0; g < ng; g++) {
        loft_ref dr = { rec.store_nr, gone[g], 8, 0 };
        loft_group_unfile(cx, parent, dr, m, n);
        /* No vector member holds the displaced record: nothing names it any more. */
        if (!held) {
            loft_remove_claims(cx, T, elem, dr);
            loft_st_delete(cx, loft_store_of(cx, dr), dr.rec);
        }
    }
    if (gone != gone_local) free(gone);
}

/* OpIndexGroup (`index_group_records`, `settle_group`): every record the vector member at
 * `primary` holds enters view member `v` — by id, the group being one record set; then a record
 * some keyed member answers another record for under its key leaves every keyed member, so
 * each answers the last record of the vector under each key.  `prim` is the vector member's
 * byte position in the parent; `m` every keyed member. */
static void loft_index_group(loft_cx *cx, loft_ref primary, uint32_t prim, int nullable, const loft_member *v, const loft_member *m,
                             int n) {
    if (primary.store_nr == 0xFFFF || primary.rec == 0 || primary.pos < prim) return;
    loft_ref parent = { primary.store_nr, primary.rec, primary.pos - prim, 0 };
    int64_t len = loft_vec_len(cx, primary);
    loft_store_t *s = loft_store_of(cx, primary);
    uint32_t arr = loft_collection_rec(cx, s, primary.rec, primary.pos);
    if (len == 0 || arr == 0) return;
    loft_ref c = loft_field(cx, parent, v->pos);
    for (int64_t i = 0; i < len; i++) {
        loft_ref r = { primary.store_nr, loft_u32(cx, loft_store_of(cx, primary), arr, 8 + 4 * (uint32_t) i), 8, 0 };
        if (r.rec == 0 || (nullable && *loft_at(cx, loft_store_of(cx, r), r.rec, 8) != 2)) continue;
        if (v->kind == LOFT_M_HASH) (void) loft_hash_insert(cx, 0, 0, c, r, v->keys, v->nk, 1);
        else if (v->kind == LOFT_M_INDEX) (void) loft_ix_insert(cx, 0, 0, c, r, v->keys, v->nk, 1);
        else if (v->kind >= LOFT_M_SPATIAL)
            (void) loft_rx_insert_record(cx, 0, 0, c, r, v->keys, v->nk, v->kind == LOFT_M_TRIE, 1);
        else (void) loft_ordered_insert(cx, 0, 0, c, r, v->keys, v->nk, 1);
    }
    for (int64_t i = 0; i < len; i++) {
        loft_ref r = { primary.store_nr, loft_u32(cx, loft_store_of(cx, primary), arr, 8 + 4 * (uint32_t) i), 8, 0 };
        if (r.rec == 0 || (nullable && *loft_at(cx, loft_store_of(cx, r), r.rec, 8) != 2)) continue;
        int displaced = 0;
        for (int j = 0; j < n && !displaced; j++) {
            uint32_t got = loft_member_answer(cx, parent, &m[j], r);
            displaced = got != 0 && got != r.rec;
        }
        if (displaced) loft_group_unfile(cx, parent, r, m, n);
    }
}

/* One record a group's owner holds, filed into every view of that owner. */
static void loft_relink_record(loft_cx *cx, const loft_tdesc *T, const loft_tdesc *d, uint16_t o, loft_ref to, loft_ref r) {
    for (uint16_t v = 0; v < d->n; v++) {
        const loft_fld *vf = &d->fields[v];
        if (!vf->view || vf->group != o + 1u) continue;
        const loft_tdesc *vt = &T[vf->type];
        /* A null element of a `vector<S?>` owner has no key (view 2). */
        if (vf->view == 2 && *loft_at(cx, loft_store_of(cx, r), r.rec, 8) != 2) continue;
        loft_ref c = loft_field(cx, to, vf->pos);
        if (vt->kind == LOFT_T_HASH) (void) loft_hash_insert(cx, 0, 0, c, r, vt->keys, vt->nk, 1);
        else if (vt->kind == LOFT_T_INDEX) (void) loft_ix_insert(cx, 0, 0, c, r, vt->keys, vt->nk, 1);
        else if (vt->kind == LOFT_T_RADIX || vt->kind == LOFT_T_TRIE)
            (void) loft_rx_insert_record(cx, 0, 0, c, r, vt->keys, vt->nk, vt->kind == LOFT_T_TRIE, 1);
        else if (vt->kind == LOFT_T_ORDERED && vt->nk) (void) loft_ordered_insert(cx, 0, 0, c, r, vt->keys, vt->nk, 1);
        else if (vt->kind == LOFT_T_ORDERED) loft_array_append(cx, c, r);
    }
}

/* `relink_group_views`: every view member of record `to` indexed over the records its copied
 * owner holds, in the owner's order (`for_each_owned_child`), as inserts through the owner
 * would — a later record under a repeated key takes the key, the state the source settled to.
 * A view only unlinks what it displaces, so nothing the owner holds is freed. */
static void loft_relink_group_views(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    for (uint16_t o = 0; o < d->n; o++) {
        const loft_fld *of = &d->fields[o];
        if (of->view || of->group != o + 1u) continue;
        loft_ref owner = loft_field(cx, to, of->pos);
        uint8_t kind = T[of->type].kind;
        uint32_t n = 0, *ids = 0;
        if (kind == LOFT_T_HASH) {
            loft_hent *all = loft_h_entries_of(cx, owner, &n);
            for (uint32_t i = 0; i < n; i++) loft_relink_record(cx, T, d, o, to, all[i].e);
            free(all);
            continue;
        }
        if (kind == LOFT_T_INDEX) ids = loft_bt_records(cx, owner, &n);
        else if (kind == LOFT_T_RADIX || kind == LOFT_T_TRIE) ids = loft_rx_records(cx, owner, &n);
        else if (kind == LOFT_T_ORDERED) {
            int64_t len = loft_vec_len(cx, owner);
            uint32_t arr = loft_collection_rec(cx, loft_store_of(cx, owner), owner.rec, owner.pos);
            if (len == 0 || arr == 0) continue;
            ids = (uint32_t *) malloc((size_t) len * 4);
            if (!ids) { fputs("loft: out of memory\n", stderr); abort(); }
            for (int64_t i = 0; i < len; i++) ids[n++] = loft_u32(cx, loft_store_of(cx, owner), arr, 8 + 4 * (uint32_t) i);
        }
        for (uint32_t i = 0; i < n; i++) {
            loft_ref r = { owner.store_nr, ids[i], 8, 0 };
            if (r.rec != 0) loft_relink_record(cx, T, d, o, to, r);
        }
        free(ids);
    }
}

static void (*loft_relink_hook)(loft_cx *cx, const loft_tdesc *T, uint16_t tp, loft_ref to) = loft_relink_group_views;

/* ── par (parallel_natives.rs): the rows in order ──
 * A `for x in v par(r = f(x), n)` loop hands every row to the worker and reads the results back
 * by row.  The C runs the worker over the rows one after another — each row's result is the
 * one a thread would compute — into a buffer on a stack the loop reads (`parallel_buf_get*`)
 * and pops (`parallel_buf_drop*`). */
typedef struct { int64_t i; double f; float g; loft_text t; loft_ref r; loft_fn fn; uint8_t adopted; } loft_par_slot;
/* `more` are the stores a threaded site's rows reach through a stored reference that are no
 * row's own, taken in from a worker beside the rows' (`loft_par_move_rows`) and released with
 * the buffer; none for every other site. */
typedef struct loft_par_buf { loft_par_slot *rows; int64_t n; uint16_t *more; uint32_t nmore, more_cap; int64_t *ints; double *flts; float *sgls; } loft_par_buf;

static loft_par_slot *loft_par_push(loft_cx *cx, int64_t n) {
    if (cx->npar == cx->par_cap) {
        cx->par_cap = cx->par_cap ? cx->par_cap * 2 : 8;
        cx->par_bufs = (loft_par_buf *) realloc(cx->par_bufs, (size_t) cx->par_cap * sizeof *cx->par_bufs);
        if (!cx->par_bufs) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_par_slot *rows = (loft_par_slot *) calloc((size_t) (n > 0 ? n : 1), sizeof *rows);
    if (!rows) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) {
        rows[i].i = LOFT_NULL;
        rows[i].r = loft_ref_null(cx);
        rows[i].fn = loft_fn_of(cx, LOFT_NULL);
    }
    cx->par_bufs[cx->npar].rows = rows;
    cx->par_bufs[cx->npar].n = n;
    cx->par_bufs[cx->npar].more = 0;
    cx->par_bufs[cx->npar].nmore = 0;
    cx->par_bufs[cx->npar].more_cap = 0;
    cx->par_bufs[cx->npar].ints = 0;
    cx->par_bufs[cx->npar].flts = 0;
    cx->par_bufs[cx->npar].sgls = 0;
    cx->npar++;
    return rows;
}

/* A buffer of scalar results, one dense row each — an integer kind's, a float's or a single's —
 * every row its null until a worker writes it (`loft_par_push` holds every kind in a slot). */
static loft_par_buf *loft_par_push_dense(loft_cx *cx, int64_t n) {
    if (cx->npar == cx->par_cap) {
        cx->par_cap = cx->par_cap ? cx->par_cap * 2 : 8;
        cx->par_bufs = (loft_par_buf *) realloc(cx->par_bufs, (size_t) cx->par_cap * sizeof *cx->par_bufs);
        if (!cx->par_bufs) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_par_buf *b = &cx->par_bufs[cx->npar++];
    b->rows = 0;
    b->n = n;
    b->more = 0;
    b->nmore = 0;
    b->more_cap = 0;
    b->ints = 0;
    b->flts = 0;
    b->sgls = 0;
    return b;
}

static int64_t *loft_par_push_i64(loft_cx *cx, int64_t n) {
    loft_par_buf *b = loft_par_push_dense(cx, n);
    b->ints = (int64_t *) malloc((size_t) (n > 0 ? n : 1) * sizeof *b->ints);
    if (!b->ints) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) b->ints[i] = LOFT_NULL;
    return b->ints;
}

static double *loft_par_push_f64(loft_cx *cx, int64_t n) {
    loft_par_buf *b = loft_par_push_dense(cx, n);
    b->flts = (double *) malloc((size_t) (n > 0 ? n : 1) * sizeof *b->flts);
    if (!b->flts) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) b->flts[i] = (double) NAN;
    return b->flts;
}

static float *loft_par_push_f32(loft_cx *cx, int64_t n) {
    loft_par_buf *b = loft_par_push_dense(cx, n);
    b->sgls = (float *) malloc((size_t) (n > 0 ? n : 1) * sizeof *b->sgls);
    if (!b->sgls) { fputs("loft: out of memory\n", stderr); abort(); }
    for (int64_t i = 0; i < n; i++) b->sgls[i] = (float) NAN;
    return b->sgls;
}

/* The top buffer when row `i` is one of its rows, else 0. */
static loft_par_buf *loft_par_top(loft_cx *cx, int64_t i) {
    if (cx->npar == 0 || i < 0 || i >= cx->par_bufs[cx->npar - 1].n) return 0;
    return &cx->par_bufs[cx->npar - 1];
}

/* A store the top buffer's rows reach, taken in from a worker: freed when the buffer is
 * dropped. */
static void loft_par_hold(loft_cx *cx, uint16_t nr) {
    if (cx->npar == 0) return;
    loft_par_buf *b = &cx->par_bufs[cx->npar - 1];
    if (b->nmore == b->more_cap) {
        b->more_cap = b->more_cap ? b->more_cap * 2 : 8;
        b->more = (uint16_t *) realloc(b->more, b->more_cap * sizeof *b->more);
        if (!b->more) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    b->more[b->nmore++] = nr;
}

static loft_par_slot *loft_par_row(loft_cx *cx, int64_t i) {
    loft_par_buf *b = loft_par_top(cx, i);
    return b && b->rows ? &b->rows[i] : 0;
}

static int64_t loft_par_get(loft_cx *cx, int64_t i) {
    loft_par_buf *b = loft_par_top(cx, i);
    if (b && b->ints) return b->ints[i];
    loft_par_slot *s = loft_par_row(cx, i);
    return s ? s->i : LOFT_NULL;
}
static double loft_par_get_float(loft_cx *cx, int64_t i) {
    loft_par_buf *b = loft_par_top(cx, i);
    if (b && b->flts) return b->flts[i];
    loft_par_slot *s = loft_par_row(cx, i);
    return s ? s->f : (double) NAN;
}
static float loft_par_get_single(loft_cx *cx, int64_t i) {
    loft_par_buf *b = loft_par_top(cx, i);
    if (b && b->sgls) return b->sgls[i];
    loft_par_slot *s = loft_par_row(cx, i);
    return s ? s->g : (float) NAN;
}
static loft_ref loft_par_get_ref(loft_cx *cx, int64_t i) { loft_par_slot *s = loft_par_row(cx, i); return s ? s->r : loft_ref_null(cx); }
static loft_fn loft_par_get_fn(loft_cx *cx, int64_t i) { loft_par_slot *s = loft_par_row(cx, i); return s ? s->fn : loft_fn_of(cx, LOFT_NULL); }
static loft_text loft_par_get_text(loft_cx *cx, int64_t i) {
    loft_par_slot *s = loft_par_row(cx, i);
    return loft_text_own(cx, s ? loft_view(cx, &s->t) : loft_null_text(cx));
}

/* The buffer leaves the stack; `refs` releases the stores the workers' results were made in. */
static void loft_par_drop(loft_cx *cx, int refs) {
    if (cx->npar == 0) return;
    loft_par_buf *b = &cx->par_bufs[--cx->npar];
    free(b->ints);
    free(b->flts);
    free(b->sgls);
    for (int64_t i = 0; b->rows && i < b->n; i++) {
        loft_text_free(cx, &b->rows[i].t);
        /* n_parallel_buf_drop_ref frees the ADOPTED stores — the ones a worker made — never a
         * result that names a store which was there before (a pass-through of its element). */
        if (refs && b->rows[i].adopted) loft_free_ref(cx, b->rows[i].r);
    }
    for (uint32_t i = 0; refs && i < b->nmore; i++) {
        loft_ref m = { b->more[i], 0, 0, 0 };
        loft_free_ref(cx, m);
    }
    free(b->more);
    free(b->rows);
}

/* ── par on threads (@PLN203 C4, tier 1) ──
 * `loft_par_run(cx, n, threads, body, ctx)` runs `body(cx, ctx, lo, hi)` over `min(threads, n)`
 * contiguous slices of the rows, as `par_pool::parallel_workers` splits them: the calling thread
 * takes the first slice, a thread each the others.  A worker starts from a COPY of the caller's
 * store table — every entry live and pinned, so it reads the caller's stores through the same
 * buffers and never frees, grows or reuses one (@FR-C-Impure) — and makes its own stores past
 * them, destroyed when it ends.  Each slice writes only its own rows of the result buffer.
 * `LOFT_TRACE_PAR_WORKERS` reports how many workers ran rows (@PLN117's instrument).  Without
 * `LOFT_THREADS` every slice runs here, in order: the same answer (@FR-C-Det). */
typedef void (*loft_par_body)(loft_cx *cx, void *ctx, int64_t lo, int64_t hi);

/* Set while a `par` runs its slices on threads: every store live before it is SHARED — read by
 * the workers through the same buffers, the caller's slice included — so nothing may grow one
 * until the workers have joined (`loft_ffi_own_*` hands a library native a copy instead).
 * Per thread: it covers the table of the thread running `loft_par_run`, whose workers share it.
 * A worker's own view of those stores is marked `pinned`, which asks for the same copy, so a
 * nested `par` in a worker counts on that worker's thread and races with no other.  Declared
 * only where something reads it: the threaded `par` and the library host. */
#if defined(LOFT_THREADS) || defined(LOFT_FFI)
#endif

/* @PLN203 C4 tier 2 — a worker whose result is a RECORD makes it in a store of its own table.
 * `adopt(cx, ctx, lo, hi, parent_n)` runs in the caller after that worker joined, with the worker's
 * table current, and takes each of the rows `lo..hi` whose record lies in a store the worker made
 * (number `parent_n` or past it) into the caller's table with that store
 * (`loft_par_move_rows`, STORE_MODEL D46), so the rest of the worker's stores can go. */
typedef void (*loft_par_adopt)(loft_cx *cx, void *ctx, int64_t lo, int64_t hi, uint32_t parent_n);

/* The caller's store table while an adopt step runs with a worker's table current
 * (`loft_par_run`): where `loft_par_move_rows` puts the stores it takes, and grows. */


/* Everything a context holds besides its store table, released when its thread ends (a `par`
 * worker, a hosted unit's context for an interpreter thread): the frames, the held texts, the
 * queues and stacks its walks grew, the text pool's cached blocks, the transient arena's spill
 * blocks.  The struct itself is its owner's. */
static void loft_cx_end(loft_cx *cx) {
    free(cx->child_jobs);
    free(cx->par_bufs);
    free(cx->evacs);
    free(cx->gens);
    free(cx->park_slots);
    free(cx->fnrefs);
    free(cx->closers);
    loft_text_free(cx, &cx->rr_path);
    free(cx->rr_marks);
    free(cx->rr_at);
    free(cx->rr_tab);
    loft_text_free(cx, &cx->rr_pool);
    free(cx->rr_t);
    loft_text_free(cx, &cx->rr_tail_path);
    free(cx->rr_tail_marks);
    free(cx->pk_p);
    free(cx->eq_jobs);
    free(cx->eq_seen);
    loft_text_free(cx, &cx->fail_msg);
    loft_text_free(cx, &cx->lazy_fault);
    while (cx->ta_spill) {
        loft_ta_blk *b = cx->ta_spill;
        cx->ta_spill = b->prev;
        free(b);
    }
#if defined(LOFT_CALL_CHAIN)
    free(cx->frames);
#endif
#if defined(LOFT_HALT_RELEASE)
    free(cx->held);
#endif
#if defined(LOFT_STRICT_STORES) || defined(LOFT_DOUBLE_FREE)
    free(cx->freed_at);
#endif
#if defined(LOFT_LEAK_SITES)
    free(cx->sites);
#endif
#if defined(LOFT_FFI)
    free(cx->ffi_log_recs);
#endif
    free(cx->scratches);
    free(cx->pg_refusal);
    free(cx->vc_starts);
    for (size_t i = 0; i < cx->jerr_n; i++) loft_text_free(cx, &cx->jerr[i]);
    free(cx->jerr);
    loft_text_free(cx, &cx->rd_path);
    loft_text_free(cx, &cx->rd_pool);
    free(cx->rd_marks);
    free(cx->rd_marked);
    free(cx->rd_at);
    free(cx->rd_tab);
    free(cx->rd_pending);
    for (size_t i = 0; i < cx->store_nperrs; i++) {
        for (size_t j = 0; j < cx->store_perrs[i].n; j++) loft_text_free(cx, &cx->store_perrs[i].e[j]);
        free(cx->store_perrs[i].e);
    }
    free(cx->store_perrs);
    free(cx->upgrade_loose);
    free(cx->uo);
    free(cx->ur_tp);
    free(cx->ur_old);
    free(cx->ur_ver);
    /* The pool last: the texts above went back into it. */
    for (int c = 0; c < LOFT_TP_CLASSES; c++)
        while (cx->tp_n[c]) free(cx->tp_blocks[c][--cx->tp_n[c]]);
    loft_cx_fresh(cx);
}

#if defined(LOFT_THREADS)
#if defined(_WIN32)
#include <process.h>
__declspec(dllimport) unsigned long __stdcall WaitForSingleObject(void *handle, unsigned long ms);
__declspec(dllimport) int __stdcall CloseHandle(void *handle);
#else
#include <pthread.h>
#endif

typedef struct {
    /* The worker's own store context (@PLN226): made by the caller before the thread starts,
     * handed to everything the worker runs. */
    loft_cx cx;
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
#ifdef LOFT_ALLOC_TRACE
    /* The worker's name in `LOFT_ALLOC_TRACE` (`cx->trace_worker`), given in the caller's
     * thread before any worker starts. */
    uint32_t trace_id;
#endif
} loft_par_job;

/* The stores of `table` from `from` on, and the table: what a worker made. */
static void loft_par_free_table(loft_cx *cx, loft_store_t *table, uint32_t from, uint32_t n) {
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

/* The stack each `par` worker thread is started with (see `loft_par_run`). */
#ifndef LOFT_PAR_STACK
#define LOFT_PAR_STACK ((size_t) 8 << 20)
#endif

/* A `par` slice the calling thread runs itself: its faults name the slice's frames, not the
 * caller's (`cx->frames_floor`). */
#ifdef LOFT_CALL_CHAIN
#define LOFT_PAR_SLICE(call)                        \
    do {                                            \
        size_t loft_floor0 = cx->frames_floor;     \
        cx->frames_floor = cx->nframes;           \
        call;                                       \
        cx->frames_floor = loft_floor0;            \
    } while (0)
#else
#define LOFT_PAR_SLICE(call) call
#endif

/* A worker's end: the stores it made are destroyed with its table and its context — or, when it
 * keeps them, its table as it now stands handed back for the caller to adopt from. */
static void loft_par_job_run(loft_cx *cx, loft_par_job *j) {
    j->body(cx, j->ctx, j->lo, j->hi);
    if (j->keep) {
        j->kept = cx->stores;
        j->kept_n = cx->nstores;
    } else {
        loft_par_free_table(cx, cx->stores, j->parent_n, cx->nstores);
    }
    cx->stores = 0;
    cx->nstores = 0;
    loft_cx_end(cx);
}

#if defined(_WIN32)
typedef void *loft_thread_id;
static unsigned __stdcall loft_par_thread(void *arg) {
    loft_par_job *j = (loft_par_job *) arg;
    loft_par_job_run(&j->cx, j);
    return 0;
}
#else
typedef pthread_t loft_thread_id;
static void *loft_par_thread(void *arg) {
    loft_par_job *j = (loft_par_job *) arg;
    loft_par_job_run(&j->cx, j);
    return 0;
}
#endif

/* A worker's job over rows `lo..hi`: its table a copy of the caller's, made here, before any
 * thread runs — the caller's own slice may grow its table. */
static void loft_par_job_init(loft_cx *cx, loft_par_job *j, loft_par_body body, void *ctx, int64_t lo, int64_t hi, int keep) {
    uint32_t i, room = cx->nstores + 16;
    j->body = body;
    j->ctx = ctx;
    j->lo = lo;
    j->hi = hi;
#ifdef LOFT_ALLOC_TRACE
    {
        static uint32_t trace_ids = 0;
        j->trace_id = ++trace_ids;
    }
#endif
    j->table = (loft_store_t *) calloc(room, sizeof *j->table);
    if (!j->table) { fputs("loft: out of memory\n", stderr); abort(); }
    /* A `parallel` block may start before the program made any store: no table to copy. */
    if (cx->nstores) memcpy(j->table, cx->stores, (size_t) cx->nstores * sizeof *j->table);
    for (i = 0; i < cx->nstores; i++) {
        j->table[i].live = 1;
        j->table[i].pinned = 1;
    }
    j->n = room;
    j->parent_n = cx->nstores;
    j->keep = keep;
    /* The worker's own context, made here before its thread starts (formal/concurrency.md
     * `C-Par`): the table above, the caller's stores in it read-only, its frames and scratch
     * empty, and no parking of work buffers. */
    loft_cx_fresh(&j->cx);
    j->cx.stores = j->table;
    j->cx.nstores = j->n;
    j->cx.no_park = 1;
#ifdef LOFT_ALLOC_TRACE
    j->cx.trace_worker = j->trace_id;
#endif
}

/* Start `j` on a thread of its own.  A thread that cannot start: its rows run here, on this
 * thread's own table.  Each worker gets LOFT_PAR_STACK bytes of stack, the same on every host:
 * the platform default is 8 MiB on Linux but 512 KiB on macOS and 1 MiB on Windows, where a
 * worker a few thousand calls deep ran out (`@FR-R-StackBase`'s deep-call cell). */
static void loft_par_job_start(loft_cx *cx, loft_par_job *j, loft_thread_id *id) {
#if defined(_WIN32)
    *id = (void *) _beginthreadex(0, (unsigned) LOFT_PAR_STACK, loft_par_thread, j, 0, 0);
    if (!*id) {
#else
    pthread_attr_t at;
    int started;
    pthread_attr_init(&at);
    pthread_attr_setstacksize(&at, LOFT_PAR_STACK);
    started = pthread_create(id, &at, loft_par_thread, j) == 0;
    pthread_attr_destroy(&at);
    if (!started) {
#endif
        free(j->table);
        j->table = 0;
        LOFT_PAR_SLICE(j->body(cx, j->ctx, j->lo, j->hi));
    }
}

static void loft_par_job_join(loft_cx *cx, loft_par_job *j, loft_thread_id id) {
#if defined(_WIN32)
    if (j->table) { WaitForSingleObject(id, 0xFFFFFFFFul); CloseHandle(id); }
#else
    if (j->table) pthread_join(id, 0);
#endif
}
#endif

/* `adopt` is 0 for a site whose results are values; for a record site it copies each worker's
 * rows into `res`, the caller's result store, made before the workers fork so every worker's
 * table holds it below `parent_n` (see loft_par_adopt). */
static void loft_par_run(loft_cx *cx, int64_t n, int64_t threads, loft_par_body body, void *ctx, loft_par_adopt adopt) {
    int64_t t = threads == LOFT_NULL || threads < 1 ? 1 : threads;
    if (t > n) t = n > 1 ? n : 1;
    if (getenv("LOFT_TRACE_PAR_WORKERS"))
        fprintf(stderr, "loft: LOFT_TRACE_PAR_WORKERS c n_rows=%lld threads=%lld distinct_workers=%lld\n",
                (long long) n, (long long) t, (long long) (n > 0 ? t : 0));
/* A hosted unit's stores are the interpreter's, and a call it makes back runs in the
 * interpreter of the calling thread: its `par` rows run on that thread, with the same answers. */
#if defined(LOFT_THREADS) && !defined(LOFT_HOSTED)
    if (t > 1) {
        int64_t k;
        cx->par_sharing++;
        loft_par_job *jobs = (loft_par_job *) calloc((size_t) t, sizeof *jobs);
        loft_thread_id *ids = (loft_thread_id *) calloc((size_t) t, sizeof *ids);
        if (!jobs || !ids) { fputs("loft: out of memory\n", stderr); abort(); }
        for (k = 1; k < t; k++)
            loft_par_job_init(cx, &jobs[k], body, ctx, k * n / t, (k + 1) * n / t, adopt != 0);
        for (k = 1; k < t; k++)
            loft_par_job_start(cx, &jobs[k], &ids[k]);
        LOFT_PAR_SLICE(body(cx, ctx, 0, n / t));
        for (k = 1; k < t; k++)
            loft_par_job_join(cx, &jobs[k], ids[k]);
        cx->par_sharing--;
        /* Each worker's records taken into the caller's table with its table current, one worker
         * at a time (`loft_par_move_rows`); the caller's table travels in through
         * `cx->par_caller` and back, so a growth of it in one worker's adoption is seen by the
         * next. */
        for (k = 1; adopt && k < t; k++) {
            loft_par_job *j = &jobs[k];
            if (!j->table || !j->kept) continue;
            cx->par_caller = cx->stores;
            cx->par_caller_n = cx->nstores;
            cx->stores = j->kept;
            cx->nstores = j->kept_n;
            adopt(cx, ctx, j->lo, j->hi, j->parent_n);
            /* The caller's table as the adopt step left it: a move may have grown it. */
            loft_store_t *own = cx->par_caller;
            uint32_t own_n = cx->par_caller_n;
            cx->stores = own;
            cx->nstores = own_n;
            cx->hint_table = 0;
            /* The stores the adopt step took are zeroed in the worker's table: what is left
             * there goes with it. */
            loft_par_free_table(cx, j->kept, j->parent_n, j->kept_n);
            j->kept = 0;
        }
        free(ids);
        free(jobs);
        return;
    }
#endif
    (void) adopt;
    body(cx, ctx, 0, n);
}

/* `parallel { arm; arm }` (`@FR-C-Arms`, loft#2568): every arm on a thread of its own, the caller waiting
 * until all have ended.  The arms are run AT ONCE, not in turn, because arms may wait on each
 * other through the outside world — a socket, a file, a pipe — and an arm run after the one
 * it answers never starts.  Each arm is a function of the block's site (`parallel.rs`) that
 * takes `pv`, the addresses of the caller's locals the arms read, and copies them in: arms
 * read the enclosing frame and write none of it (LOFT_CONTROL.md § Parallel blocks).  Each
 * thread starts from a copy of the caller's store table, as a `par` worker does — the caller's
 * stores pinned, the arm's own stores past them, destroyed when it ends: an arm's result is
 * discarded, so nothing is adopted.  Without `LOFT_THREADS` the arms run here, in order. */
typedef void (*loft_arm_fn)(loft_cx *cx, void *const *loft_pv);
typedef struct { loft_arm_fn const *fns; void *const *pv; } loft_arms;

static void loft_arms_body(loft_cx *cx, void *vc, int64_t lo, int64_t hi) {
    loft_arms *a = (loft_arms *) vc;
    int64_t i;
    for (i = lo; i < hi; i++) a->fns[i](cx, a->pv);
}

static void loft_parallel_run(loft_cx *cx, int64_t n, loft_arm_fn const *fns, void *const *pv) {
    loft_arms a;
    a.fns = fns;
    a.pv = pv;
#if defined(LOFT_THREADS) && !defined(LOFT_HOSTED)
    if (n > 0) {
        int64_t k;
        loft_par_job *jobs = (loft_par_job *) calloc((size_t) n, sizeof *jobs);
        loft_thread_id *ids = (loft_thread_id *) calloc((size_t) n, sizeof *ids);
        if (!jobs || !ids) { fputs("loft: out of memory\n", stderr); abort(); }
        cx->par_sharing++;
        for (k = 0; k < n; k++) loft_par_job_init(cx, &jobs[k], loft_arms_body, &a, k, k + 1, 0);
        for (k = 0; k < n; k++) loft_par_job_start(cx, &jobs[k], &ids[k]);
        for (k = 0; k < n; k++) loft_par_job_join(cx, &jobs[k], ids[k]);
        cx->par_sharing--;
        free(ids);
        free(jobs);
        return;
    }
#endif
    loft_arms_body(cx, &a, 0, n);
}

/* parallel_buf_get_narrow: a 1-, 2- or 4-byte result, its low bytes extended as `signed` says. */
static int64_t loft_par_get_narrow(loft_cx *cx, int64_t i, int64_t size, int64_t is_signed) {
    loft_par_buf *b = loft_par_top(cx, i);
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
static uint64_t loft_par_arg(loft_cx *cx, loft_ref e, int64_t size) {
    if (e.rec == 0) return 0;
    uint8_t *p = loft_at(cx, loft_store_of(cx, e), e.rec, e.pos);
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

static int64_t loft_i64_bits(loft_cx *cx, uint64_t u) {
    int64_t v;
    memcpy(&v, &u, 8);
    return v;
}
