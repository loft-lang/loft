/* ── The working-set loaders (Stores::load_keys, load_key_text, load_prefix, load_box,
 * load_range in src/database/allocation.rs) ──────────────────────────────────────────────
 * Each finds its entries in an image a page at a time (loft_paged.h) and copies them into the
 * local collection: the entry's bytes verbatim, then every `text`, vector and nested record it
 * points at copied into the local store (`relocate_ptr_fields`), then the entry linked in by the
 * collection's own insert — never a bucket or a node built here.  Which collections a loader
 * serves and which element types it can copy is decided when the program is written to C, by the
 * Rust store's own checks; a loader it cannot serve still runs, and answers 0 after saying why,
 * as the interpreter does.  The Rust loaders' batched warms only order the page reads, and are
 * not ported. */

#define LOFT_PG_HASH 0
#define LOFT_PG_TRIE 1
#define LOFT_PG_RADIX 2

/* Stores::refuse_paged: a refusal, which reads as 0 like an absent key, said on stderr. */
static void loft_pg_refuse(const char *path, const char *reason) {
    fprintf(stderr, "store loader: refusing %s — %s; loaded NOTHING (a refusal, not an absent key)\n", path, reason);
    loft_pg_record(path, reason);
}

#ifdef LOFT_NET
static int loft_net_gate(const char *u, const loft_sidecar *sc, const char *who, long *recorded, int convert);
#endif

/* What every loader does before it reads (layout_gate_ok, PageSource::open, the store's
 * recorded type, then the refusal the emitter found): 1 with the reader open, else 0.  A URL is
 * gated by its fetched sidecar (`check_sidecar`) and read by range. */
static int loft_pg_begin(loft_pg *r, loft_ref local, loft_str path, const loft_sidecar *sc, const char *refusal) {
    memset(r, 0, sizeof *r);
    if (local.store_nr == 0xFFFF || local.store_nr >= loft_nstores) return 0;
    char *p = loft_cpath(path, 0);
    int url = loft_pg_is_url(p);
    if (!url && loft_path_refused(path)) {
        free(p);
        return 0;
    }
    uint16_t known = loft_stores[local.store_nr].known;
    char err[1100];
    loft_gate_paged = 1;
    loft_gate_trie_v1 = 0;
#ifdef LOFT_NET
    int ok = known == 0xFFFF || (url ? loft_net_gate(p, sc, 0, 0, 0) : loft_image_gate(p, sc, 0, 0));
#else
    int ok = known == 0xFFFF || (!url && loft_image_gate(p, sc, 0, 0));
    if (url && !ok) loft_pg_refuse(p, "this build has no HTTP transport, so it cannot range-read it");
#endif
    loft_gate_paged = 0;
    if (ok && !loft_pg_open(r, p, err, sizeof err)) {
        loft_pg_refuse(p, "it cannot be opened as a paged source");
        ok = 0;
    } else if (ok && known == 0xFFFF) {
        loft_pg_refuse(p, "the target collection's store has no recorded type");
        ok = 0;
    } else if (ok && refusal) {
        loft_pg_refuse(p, refusal);
        ok = 0;
    }
    if (!ok) loft_pg_close(r);
    else r->trie_v1 = loft_gate_trie_v1;
    free(p);
    return ok;
}

/* paged_reader::key_compare_reader: the probe against the entry at (rec, pos) — below 0, 0 or
 * above as the probe sorts before, with or after it.  An integer probe reads an integer key by its
 * width, a text probe a text key; any other pairing sorts after, so it never matches. */
static int loft_pg_key_cmp(loft_pg *r, const loft_kv *kv, int n, uint32_t rec, uint32_t pos, const loft_key *keys) {
    for (int i = 0; i < n; i++) {
        const loft_key *k = &keys[i];
        int t = k->type < 0 ? -k->type : k->type, c;
        uint32_t p = pos + k->pos;
        if (kv[i].kind == LOFT_KV_INT && (t == 1 || t == 2 || (t >= 8 && t <= 12))) {
            int64_t v = loft_pg_field_value(r, rec, p, k);
            c = kv[i].i < v ? -1 : kv[i].i > v;
        } else if (kv[i].kind == LOFT_KV_TEXT && t == 6) {
            loft_str e = loft_pg_text_at(r, loft_pg_u32(r, rec, p));
            r->krec = UINT32_MAX;
            c = loft_str_cmp(kv[i].s, e);
            c = c < 0 ? -1 : c > 0;
        } else {
            return 1;
        }
        if (c != 0) return k->type < 0 ? -c : c;
    }
    return 0;
}

/* paged_reader::entry_at: where an arena table's slot puts its entry; a slot naming a record
 * (a table that borrows them) is that record's payload. */
static void loft_pg_entry_at(loft_pg *r, uint32_t claim, uint32_t slot, uint32_t stride, uint32_t *rec, uint32_t *pos) {
    uint32_t k, off;
    *rec = *pos = 0;
    if (stride == 0 || (slot & LOFT_H_SLOT_RECORD)) {
        *rec = slot & ~LOFT_H_SLOT_RECORD;
        *pos = 8;
        return;
    }
    uint32_t dir = loft_pg_u32(r, claim, LOFT_AR_DIR_FLD), dir_words = loft_pg_record_extent(r, dir);
    if (dir_words == 0) return;
    loft_ar_locate(slot, stride, &k, &off);
    if (k >= (dir_words - 1) * 2) return;
    uint32_t chunk = loft_pg_u32(r, dir, LOFT_AR_DIR0 + 4 * k);
    if (chunk != 0) {
        *rec = chunk;
        *pos = off;
    }
}

/* paged_reader::find_hash_entry: hash::find over the image (`@FR-S-Digest`, `@FR-S-Probe`) — the
 * entry's record and byte position, 0 when the key is absent. */
static uint32_t loft_pg_hash_find(loft_pg *r, uint32_t root_rec, uint32_t root_pos, const loft_kv *kv, int n,
                                  const loft_key *keys, uint32_t *pos_out) {
    uint32_t claim = loft_pg_u32(r, root_rec, root_pos), rec, pos;
    uint32_t room = loft_pg_record_extent(r, claim);
    if (room < LOFT_H_RESERVED) return 0;
    uint32_t flags = loft_pg_u32(r, claim, LOFT_H_STRIDE_FLD);
    uint32_t sb = (flags & LOFT_H_TAGGED) ? 8 : LOFT_H_SLOT_BYTES, stride = flags & LOFT_H_STRIDE_MASK;
    int inl = (flags & LOFT_H_INLINE) != 0;
    uint32_t elms = inl ? 1u << ((flags >> LOFT_H_COUNT_SHIFT) & 31) : loft_h_slots_in(room - LOFT_H_RESERVED, sb);
    /* An inline table's count comes from its flags: its buckets and entries must fit the record. */
    if (inl && LOFT_H_BUCKET0 + (uint64_t) elms * (8 + (uint64_t) stride) > (uint64_t) room * 8) return 0;
    uint64_t seed = (uint64_t) loft_pg_i64(r, claim, LOFT_H_SEED_FLD);
    uint32_t index = loft_h_home(loft_kv_digest(kv, n, seed), elms);
    uint32_t slot = loft_pg_u32(r, claim, LOFT_H_BUCKET0 + index * sb);
    for (uint32_t i = 0; i < elms; i++) {
        if (slot == 0) return 0;
        if (!inl) {
            loft_pg_entry_at(r, claim, slot, stride, &rec, &pos);
        } else if (slot == LOFT_H_TOMB) {
            rec = pos = 0;
        } else {
            rec = claim;
            pos = loft_h_ent0(elms) + index * stride;
        }
        if (rec != 0 && loft_pg_key_cmp(r, kv, n, rec, pos, keys) == 0) {
            *pos_out = pos;
            return rec;
        }
        index = (index + 1) % elms;
        slot = loft_pg_u32(r, claim, LOFT_H_BUCKET0 + index * sb);
    }
    return 0;
}

/* paged_reader::trie_find_rec: the record whose text key is `key`, by one descent; 0 for none. */
static uint32_t loft_pg_trie_find(loft_pg *r, uint32_t root_rec, uint32_t root_pos, loft_str key, const loft_key *keys) {
    if (keys[0].type != 6) return 0;
    uint32_t t = loft_pg_tree(r, root_rec, root_pos);
    if (t == 0) return 0;
    loft_rx_key probe;
    memset(&probe, 0, sizeof probe);
    probe.keys = keys;
    probe.nk = 1;
    probe.text = 1;
    probe.bytes = (const uint8_t *) key.p;
    probe.len = key.len;
    /* A null key is filed in the slot ahead of the tree (`@FR-Col-Trie`); a token-1 image has
     * none, and reads its keys raw. */
    if (!r->trie_v1 && loft_rx_null_key(probe.bytes, probe.len)) {
        uint32_t w = loft_pg_u32(r, t, LOFT_RX_SLOT);
        return (w & LOFT_RX_SLOT_TAG) ? w & ~LOFT_RX_SLOT_TAG : 0;
    }
    loft_rx_text_bits(&probe, !r->trie_v1);
    return loft_prx_get(r, t, &probe);
}

/* The copy (Stores::copy_flat_subrecord): a string or a vector of scalars — no pointers inside —
 * claimed as long as its LENGTH needs, never its source's spare capacity, its bytes from @4.
 * 0, an absent field, for a source the image does not hold whole or whose length runs past it. */
static uint32_t loft_pg_copy_flat(loft_pg *r, uint16_t store_nr, uint32_t src, uint32_t esize) {
    uint32_t ssz = loft_pg_record_extent(r, src), len, want;
    if (ssz == 0) return 0;
    len = loft_pg_u32(r, src, 4);
    if (esize > 0 && 8 + (uint64_t) len * esize > (uint64_t) ssz * 8) return 0;
    want = ssz;
    if (esize > 0) {
        uint64_t words = (8 + (uint64_t) len * esize + 7) / 8;
        if (words < ssz) want = (uint32_t) words;
    }
    loft_store_t *s = &loft_stores[store_nr];
    uint32_t nrec = loft_st_claim(s, want < 2 ? 2 : want);
    loft_zero_fill(s, nrec);
    size_t body = want * 8 > 4 ? (size_t) want * 8 - 4 : 0;
    uint8_t *buf = (uint8_t *) malloc(body ? body : 1);
    if (!buf) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_pg_read(r, (uint64_t) src * 8 + 4, buf, body);
    for (uint32_t at = 4; at + 4 <= want * 8; at += 4) {
        uint32_t w;
        memcpy(&w, buf + at - 4, 4);
        loft_set_u32(s, nrec, at, w);
    }
    free(buf);
    return nrec;
}

static uint32_t loft_pg_copy_vector(loft_pg *r, const loft_tdesc *T, uint16_t store_nr, uint32_t src, uint16_t elem);

/* Stores::relocate_ptr_fields: the pointers of a struct of type `tp`, its bytes already copied to
 * byte `dst_fb` of `dst_rec`, each copied into the local store — a string or a vector of
 * scalars flat, a nested struct field by field, an enum by the variant it holds, a vector of
 * structs element by element.  The
 * source pointer is read from the local copy, which holds the image's own words. */
static void loft_pg_relocate(loft_pg *r, const loft_tdesc *T, uint16_t store_nr, uint32_t dst_rec, uint32_t dst_fb,
                             uint16_t tp) {
    const loft_tdesc *d = &T[tp];
    if (d->kind != LOFT_T_STRUCT) return;
    for (uint32_t i = 0; i < d->n; i++) {
        const loft_fld *f = &d->fields[i];
        const loft_tdesc *ft = &T[f->type];
        uint32_t at = dst_fb + f->pos, sub, nrec;
        if (ft->kind == LOFT_T_TEXT || (ft->kind == LOFT_T_VECTOR && T[ft->elem].kind == LOFT_T_SCALAR)) {
            sub = loft_u32(&loft_stores[store_nr], dst_rec, at);
            if (sub != 0) {
                nrec = loft_pg_copy_flat(r, store_nr, sub, ft->kind == LOFT_T_TEXT ? 1 : T[ft->elem].size);
                loft_set_u32(&loft_stores[store_nr], dst_rec, at, nrec);
            }
        } else if (ft->kind == LOFT_T_STRUCT) {
            loft_pg_relocate(r, T, store_nr, dst_rec, at, f->type);
        } else if (ft->kind == LOFT_T_ENUM) {
            /* An enum: its tag byte (1 is the first variant, 0 null), and a struct-enum
             * variant's payload at the same position, as `copy_claims` walks it. */
            int e = (int) *loft_at(&loft_stores[store_nr], dst_rec, at) - 1;
            if (e >= 0 && e < (int) ft->n && ft->variants[e] != 0xFFFF)
                loft_pg_relocate(r, T, store_nr, dst_rec, at, ft->variants[e]);
        } else if (ft->kind == LOFT_T_VECTOR) {
            sub = loft_u32(&loft_stores[store_nr], dst_rec, at);
            if (sub != 0) {
                nrec = loft_pg_copy_vector(r, T, store_nr, sub, ft->elem);
                loft_set_u32(&loft_stores[store_nr], dst_rec, at, nrec);
            }
        }
    }
}

/* Stores::copy_vector_of_struct: the elements' bytes flat, then each element's pointers. */
static uint32_t loft_pg_copy_vector(loft_pg *r, const loft_tdesc *T, uint16_t store_nr, uint32_t src, uint16_t elem) {
    uint32_t esize = T[elem].size;
    uint32_t nrec = loft_pg_copy_flat(r, store_nr, src, esize);
    uint32_t length = loft_pg_u32(r, src, 4);
    if (esize == 0 || nrec == 0) return nrec;
    for (uint32_t i = 0; i < length; i++) loft_pg_relocate(r, T, store_nr, nrec, 8 + i * esize, elem);
    return nrec;
}

/* Stores::copy_entry: the located entry `(src_rec, src_pos)` copied into `local` — a hash entry
 * into a slot the local table hands out, a tree's into a record of its own — its pointers
 * relocated, and filed by the collection's own insert. */
static int loft_pg_copy_entry(loft_pg *r, const loft_tdesc *T, loft_ref local, uint16_t content, const loft_key *keys,
                              int nk, int kind, uint32_t src_rec, uint32_t src_pos) {
    loft_store_t *s = &loft_stores[local.store_nr];
    uint32_t bytes;
    loft_ref e;
    if (kind == LOFT_PG_HASH) {
        bytes = loft_h_stride_for(T[content].size);
        e = loft_hash_entry(local, T[content].size, T[content].linked);
        if (e.rec == 0) return 0;
    } else {
        uint32_t size = loft_pg_record_extent(r, src_rec);
        if (size < 2) return 0;
        e.store_nr = local.store_nr;
        e.rec = loft_st_claim(s, size);
        e.pos = 8;
        loft_zero_fill(s, e.rec);
        bytes = size * 8 - 8;
    }
    uint8_t *buf = (uint8_t *) malloc(bytes ? bytes : 1);
    if (!buf) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_pg_read(r, (uint64_t) src_rec * 8 + src_pos, buf, bytes);
    for (uint32_t off = 0; off + 4 <= bytes; off += 4) {
        uint32_t w;
        memcpy(&w, buf + off, 4);
        loft_set_u32(s, e.rec, e.pos + off, w);
    }
    free(buf);
    loft_pg_relocate(r, T, local.store_nr, e.rec, e.pos, content);
    if (kind == LOFT_PG_HASH) (void) loft_h_add(local, e, keys, nk);
    else loft_rx_add(local, e.rec, keys, nk, kind == LOFT_PG_TRIE);
    return 1;
}

/* Stores::load_one: one key located — a hash by its digest, a trie by its text — and copied. */
static int loft_pg_load_one(loft_pg *r, const loft_tdesc *T, loft_ref local, uint16_t content, const loft_key *keys, int nk,
                            int kind, const loft_kv *kv) {
    uint32_t rec = 0, pos = 0;
    if (kind == LOFT_PG_HASH) {
        rec = loft_pg_hash_find(r, local.rec, local.pos, kv, 1, keys, &pos);
    } else if (kind == LOFT_PG_TRIE && kv->kind == LOFT_KV_TEXT) {
        rec = loft_pg_trie_find(r, local.rec, local.pos, kv->s, keys);
        pos = 8;
    }
    return rec != 0 && loft_pg_copy_entry(r, T, local, content, keys, nk, kind, rec, pos);
}

/* store_load_key / store_load_keys / store_load_key_text / store_load_keys_text: each key's
 * entry, through one reader; the count found. */
static int64_t loft_pg_load_keys(const loft_tdesc *T, loft_ref local, loft_str path, const loft_sidecar *sc,
                                 const char *refusal, uint16_t content, const loft_key *keys, int nk, int kind,
                                 const loft_kv *kv, int64_t n) {
    loft_pg r;
    int64_t loaded = 0;
    if (!loft_pg_begin(&r, local, path, sc, refusal)) return 0;
    for (int64_t i = 0; i < n; i++) loaded += loft_pg_load_one(&r, T, local, content, keys, nk, kind, &kv[i]);
    loft_pg_close(&r);
    return loaded;
}

/* The keys of a `vector<integer>` or a `vector<text>` argument (Stores::int_vector, str_vector),
 * malloc'd; a text key views the vector's own string. */
static loft_kv *loft_pg_key_list(loft_ref v, int text, int64_t *n) {
    loft_store_t *s = loft_store_of(v);
    uint32_t inner = v.rec == 0 ? 0 : loft_u32(s, v.rec, v.pos);
    uint32_t len = inner == 0 ? 0 : loft_u32(s, inner, 4);
    loft_kv *kv = (loft_kv *) malloc(((size_t) len + 1) * sizeof(loft_kv));
    if (!kv) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint32_t i = 0; i < len; i++) {
        if (text) {
            kv[i] = loft_kv_text(loft_store_str(s, loft_u32(s, inner, 8 + i * 4)));
        } else {
            int64_t x;
            memcpy(&x, loft_at(s, inner, 8 + i * 8), 8);
            kv[i] = loft_kv_int(x);
        }
    }
    *n = len;
    return kv;
}

static int64_t loft_pg_load_key_list(const loft_tdesc *T, loft_ref local, loft_str path, const loft_sidecar *sc,
                                     const char *refusal, uint16_t content, const loft_key *keys, int nk, int kind,
                                     loft_ref list, int text) {
    int64_t n;
    loft_kv *kv = loft_pg_key_list(list, text, &n);
    int64_t got = loft_pg_load_keys(T, local, path, sc, refusal, content, keys, nk, kind, kv, n);
    free(kv);
    return got;
}

/* store_load_prefix (Stores::load_prefix): every entry of a trie whose key starts with `pre`, the
 * walk capped at `limit` (a negative one: no cap); the count loaded. */
static int64_t loft_pg_load_prefix(const loft_tdesc *T, loft_ref local, loft_str path, const loft_sidecar *sc,
                                   const char *refusal, uint16_t content, const loft_key *keys, int nk, loft_str pre,
                                   int64_t limit) {
    loft_pg r;
    int64_t loaded = 0;
    if (!loft_pg_begin(&r, local, path, sc, refusal)) return 0;
    uint32_t t = keys[0].type == 6 ? loft_pg_tree(&r, local.rec, local.pos) : 0;
    if (t != 0) {
        loft_u32s found = loft_u32s_new(16);
        loft_prx_prefix_walk(&r, t, keys, pre, limit < 0 ? SIZE_MAX : (size_t) limit, &found);
        for (size_t i = 0; i < found.n; i++) loaded += loft_pg_copy_entry(&r, T, local, content, keys, nk, LOFT_PG_TRIE, found.p[i], 8);
        free(found.p);
    }
    loft_pg_close(&r);
    return loaded;
}

/* store_load_box (Stores::load_box): every entry of a spatial index inside the closed box between
 * two corners, the walk capped at `limit`; the count loaded. */
static int64_t loft_pg_load_box_kv(const loft_tdesc *T, loft_ref local, loft_str path, const loft_sidecar *sc,
                                   const char *refusal, uint16_t content, const loft_key *keys, int nk,
                                   const loft_kv *f, int64_t nf, const loft_kv *t, int64_t nt, int64_t limit,
                                   const char *type_name) {
    loft_pg r;
    int64_t loaded = 0;
    if (!loft_pg_begin(&r, local, path, sc, refusal)) return 0;
    if (nf < nk || nt < nk) {
        char why[256];
        snprintf(why, sizeof why,
                 "the box has %lld and %lld coordinates but `%s` has %d axes — a corner must name every one of them",
                 (long long) nf, (long long) nt, type_name, nk);
        char *p = loft_cpath(path, 0);
        loft_pg_refuse(p, why);
        free(p);
    } else {
        int text = 0;
        for (int a = 0; a < nk; a++) text |= keys[a].type == 6;
        uint32_t tree = nk > 0 && nk <= LOFT_RX_MAX_AXES && !text ? loft_pg_tree(&r, local.rec, local.pos) : 0;
        if (tree != 0) {
            int64_t fa[LOFT_RX_MAX_AXES], ta[LOFT_RX_MAX_AXES];
            uint64_t qlo[LOFT_RX_MAX_AXES] = { 0, 0, 0 }, qhi[LOFT_RX_MAX_AXES] = { 0, 0, 0 };
            for (int a = 0; a < nk; a++) {
                fa[a] = f[a].i;
                ta[a] = t[a].i;
            }
            loft_rx_corners(nk, fa, ta, qlo, qhi);
            loft_u32s found = loft_u32s_new(16);
            loft_prx_box_walk(&r, tree, keys, nk, qlo, qhi, limit < 0 ? SIZE_MAX : (size_t) limit, &found);
            for (size_t i = 0; i < found.n; i++)
                loaded += loft_pg_copy_entry(&r, T, local, content, keys, nk, LOFT_PG_RADIX, found.p[i], 8);
            free(found.p);
        }
    }
    loft_pg_close(&r);
    return loaded;
}

static int64_t loft_pg_load_box(const loft_tdesc *T, loft_ref local, loft_str path, const loft_sidecar *sc,
                                const char *refusal, uint16_t content, const loft_key *keys, int nk, loft_ref from,
                                loft_ref till, int64_t limit, const char *type_name) {
    int64_t nf, nt;
    loft_kv *f = loft_pg_key_list(from, 0, &nf), *t = loft_pg_key_list(till, 0, &nt);
    int64_t loaded = loft_pg_load_box_kv(T, local, path, sc, refusal, content, keys, nk, f, nf, t, nt, limit, type_name);
    free(f);
    free(t);
    return loaded;
}

/* store_load_range (Stores::load_range): the entries of a sorted collection with an integer key in
 * [lo, hi] — a binary search to the first, a walk to the last (paged_reader::
 * sorted_range_positions) — built as the local vector in key order, each element's pointers
 * relocated; the count loaded. */
static int64_t loft_pg_load_range(const loft_tdesc *T, loft_ref local, loft_str path, const loft_sidecar *sc,
                                  const char *refusal, uint16_t content, const loft_key *keys, int64_t lo, int64_t hi) {
    loft_pg r;
    uint32_t esize = T[content].size, count = 0;
    if (!loft_pg_begin(&r, local, path, sc, refusal)) return 0;
    uint32_t inner = loft_pg_u32(&r, local.rec, local.pos), words = loft_pg_record_extent(&r, inner);
    uint32_t length = words == 0 ? 0 : loft_pg_u32(&r, inner, 4);
    if (esize != 0 && words != 0 && 8 + (uint64_t) length * esize <= (uint64_t) words * 8) {
        loft_kv klo = loft_kv_int(lo), khi = loft_kv_int(hi);
        uint32_t left = 0, right = length;
        while (left < right) {
            uint32_t mid = left + (right - left) / 2;
            if (loft_pg_key_cmp(&r, &klo, 1, inner, 8 + mid * esize, keys) > 0) left = mid + 1;
            else right = mid;
        }
        loft_u32s at = loft_u32s_new(16);
        for (uint32_t i = left; i < length; i++) {
            if (loft_pg_key_cmp(&r, &khi, 1, inner, 8 + i * esize, keys) < 0) break;
            loft_u32s_push(&at, 8 + i * esize);
        }
        count = (uint32_t) at.n;
        if (count != 0) {
            loft_store_t *s = &loft_stores[local.store_nr];
            uint32_t words = (8 + count * esize + 7) / 8, vec;
            vec = loft_st_claim(s, words < 2 ? 2 : words);
            loft_zero_fill(s, vec);
            loft_set_u32(s, vec, 4, count);
            loft_set_u32(s, local.rec, local.pos, vec);
            for (uint32_t i = 0; i < count; i++) {
                uint32_t dst = 8 + i * esize;
                for (uint32_t b = 0; b + 4 <= esize; b += 4) loft_set_u32(s, vec, dst + b, loft_pg_u32(&r, inner, at.p[i] + b));
                loft_pg_relocate(&r, T, local.store_nr, vec, dst, content);
            }
        }
        free(at.p);
    }
    loft_pg_close(&r);
    return count;
}
