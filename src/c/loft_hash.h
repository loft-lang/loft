/* ── The hash table (src/hash.rs) and its entry arena (src/arena.rs) ───────────────────────
 * loft's own layout, ported line for line: a persisted store IS this layout, so a table the C
 * writes is one the interpreter reads and the other way round.
 *
 * The collection slot names the TABLE record (byte offsets):
 *   0  size header       4  live count (arena) / owner (inline)     8  seed (u64)
 *   16 arena directory (arena) / live count (inline)                20 arena cursor / insert count
 *   24 arena free list (arena) / tombstones (inline)                28 stride and flags
 *   32 the bucket slots: an entry index and, TAGGED, its digest's low 32 bits.
 * An ARENA table keeps its entries in chunked records (a slot names a 1-based arena index, or a
 * record with SLOT_RECORD set); an INLINE table keeps them after its slots, one per bucket.  A
 * table of stride 0 borrows its entries: records a linked group's sibling also names. */

#define LOFT_AR_BASE 64u
#define LOFT_AR_CAP_CHUNK 7u
#define LOFT_AR_CAP_SLOTS (LOFT_AR_BASE << LOFT_AR_CAP_CHUNK)
#define LOFT_AR_CAP_START (LOFT_AR_BASE * ((1u << (LOFT_AR_CAP_CHUNK + 1)) - 1))
#define LOFT_AR_DIR0 8u
#define LOFT_AR_SLOT0 8u
#define LOFT_AR_OWNER_FLD 4u
#define LOFT_AR_DIR_FLD 16u
#define LOFT_AR_NEXT_FLD 20u
#define LOFT_AR_FREE_FLD 24u

#define LOFT_H_LEN_FLD 4u
#define LOFT_H_SEED_FLD 8u
#define LOFT_H_STRIDE_FLD 28u
#define LOFT_H_BUCKET0 32u
#define LOFT_H_RESERVED 4u
#define LOFT_H_SLOT_BYTES 4u
#define LOFT_H_TAGGED 0x80000000u
#define LOFT_H_INLINE 0x40000000u
#define LOFT_H_TOMB 0xFFFFFFFFu
#define LOFT_H_STRIDE_MASK 0xFFFFu
#define LOFT_H_COUNT_SHIFT 16u
#define LOFT_H_INLINE_LEN_FLD LOFT_AR_DIR_FLD
#define LOFT_H_SLOT_RECORD 0x80000000u
#define LOFT_H_STAGE_MARK 0xFFFFFFF1u
#define LOFT_H_GROWTH 2u

static uint32_t loft_ilog2(uint32_t v) {
    uint32_t r = 0;
    while (v >>= 1) r++;
    return r;
}

static uint64_t loft_next_pow2(uint64_t v) {
    uint64_t p = 1;
    while (p < v) p <<= 1;
    return p;
}

static int loft_is_pow2(uint32_t v) { return v != 0 && (v & (v - 1)) == 0; }

/* Store::record_words: a claimed record's size in words. */
static uint32_t loft_rec_words(const loft_store_t *s, uint32_t rec) {
    int32_t w = loft_st_i32(s, rec, 0);
    return w > 0 ? (uint32_t) w : 0;
}

/* Store::zero_fill: a record's payload, every byte past its header word. */
static void loft_zero_fill(loft_store_t *s, uint32_t rec) {
    memset(s->buf + (size_t) rec * 8 + 4, 0, (size_t) loft_rec_words(s, rec) * 8 - 4);
}

/* ── arena.rs ── */

static uint32_t loft_ar_chunk_of(uint32_t i) {
    if (i < LOFT_AR_CAP_START) return loft_ilog2(i / LOFT_AR_BASE + 1);
    return LOFT_AR_CAP_CHUNK + 1 + (i - LOFT_AR_CAP_START) / LOFT_AR_CAP_SLOTS;
}

static uint32_t loft_ar_first_index_of(uint32_t k) {
    if (k <= LOFT_AR_CAP_CHUNK) return LOFT_AR_BASE * ((1u << k) - 1);
    return LOFT_AR_CAP_START + (k - LOFT_AR_CAP_CHUNK - 1) * LOFT_AR_CAP_SLOTS;
}

static void loft_ar_locate(uint32_t index, uint32_t stride, uint32_t *k, uint32_t *off) {
    uint32_t i = index - 1;
    *k = loft_ar_chunk_of(i);
    *off = LOFT_AR_SLOT0 + (i - loft_ar_first_index_of(*k)) * stride;
}

static uint32_t loft_ar_chunk_slots(uint32_t k) {
    return k <= LOFT_AR_CAP_CHUNK ? LOFT_AR_BASE << k : LOFT_AR_CAP_SLOTS;
}

static uint32_t loft_ar_chunk_words(uint32_t k, uint32_t stride) {
    return 1 + (loft_ar_chunk_slots(k) * stride + 7) / 8;
}

static uint32_t loft_ar_dir_capacity(const loft_store_t *s, uint32_t dir) {
    return dir == 0 ? 0 : (loft_rec_words(s, dir) - 1) * 2;
}

static uint32_t loft_ar_chunk_rec(loft_store_t *s, uint32_t dir, uint32_t k) {
    if (dir == 0 || k >= loft_ar_dir_capacity(s, dir)) return 0;
    return loft_u32(s, dir, LOFT_AR_DIR0 + 4 * k);
}

static void loft_ar_set_chunk_rec(loft_store_t *s, uint32_t table, uint32_t k, uint32_t rec) {
    uint32_t dir = loft_u32(s, table, LOFT_AR_DIR_FLD);
    if (k >= loft_ar_dir_capacity(s, dir)) {
        uint32_t m = k + 1 > 4 ? k + 1 : 4;
        uint32_t fresh = loft_st_claim(s, 1 + (m + 1) / 2);
        loft_zero_fill(s, fresh);
        if (dir != 0) {
            uint32_t old = loft_ar_dir_capacity(s, dir);
            for (uint32_t i = 0; i < old; i++) loft_set_u32(s, fresh, LOFT_AR_DIR0 + 4 * i, loft_u32(s, dir, LOFT_AR_DIR0 + 4 * i));
            loft_st_delete(s, dir);
        }
        loft_set_u32(s, table, LOFT_AR_DIR_FLD, fresh);
    }
    loft_set_u32(s, loft_u32(s, table, LOFT_AR_DIR_FLD), LOFT_AR_DIR0 + 4 * k, rec);
}

static uint32_t loft_ar_ensure_chunk(loft_store_t *s, uint32_t table, uint32_t k, uint32_t stride, uint32_t owner) {
    uint32_t existing = loft_ar_chunk_rec(s, loft_u32(s, table, LOFT_AR_DIR_FLD), k);
    if (existing != 0) return existing;
    uint32_t rec = loft_st_claim(s, loft_ar_chunk_words(k, stride));
    loft_zero_fill(s, rec);
    loft_set_u32(s, rec, LOFT_AR_OWNER_FLD, owner);
    loft_ar_set_chunk_rec(s, table, k, rec);
    return rec;
}

/* arena::alloc_at: a slot of `stride` bytes — a freed one first, its bytes cleared — as its
 * index, its chunk record and its offset there. */
static uint32_t loft_ar_alloc_at(loft_store_t *s, uint32_t table, uint32_t stride, uint32_t owner, uint32_t *rec_out,
                                 uint32_t *off_out) {
    uint32_t free_i = loft_u32(s, table, LOFT_AR_FREE_FLD), index, k, off;
    if (free_i != 0) {
        loft_ar_locate(free_i, stride, &k, &off);
        uint32_t rec = loft_ar_chunk_rec(s, loft_u32(s, table, LOFT_AR_DIR_FLD), k);
        loft_set_u32(s, table, LOFT_AR_FREE_FLD, loft_u32(s, rec, off));
        index = free_i;
    } else {
        uint32_t next = loft_u32(s, table, LOFT_AR_NEXT_FLD);
        if (next < 1) next = 1;
        loft_set_u32(s, table, LOFT_AR_NEXT_FLD, next + 1);
        index = next;
    }
    loft_ar_locate(index, stride, &k, &off);
    uint32_t rec = loft_ar_ensure_chunk(s, table, k, stride, owner);
    if (free_i != 0) {
        for (uint32_t w = 0; w < stride / 4; w++) loft_set_u32(s, rec, off + 4 * w, 0);
    }
    *rec_out = rec;
    *off_out = off;
    return index;
}

static void loft_ar_free(loft_store_t *s, uint32_t table, uint32_t index, uint32_t stride) {
    uint32_t k, off;
    if (index == 0) return;
    loft_ar_locate(index, stride, &k, &off);
    uint32_t rec = loft_ar_chunk_rec(s, loft_u32(s, table, LOFT_AR_DIR_FLD), k);
    if (rec == 0) return;
    loft_set_u32(s, rec, off, loft_u32(s, table, LOFT_AR_FREE_FLD));
    loft_set_u32(s, table, LOFT_AR_FREE_FLD, index);
}

/* arena::index_of: the index of the slot at (rec, off), 0 when no chunk is that record. */
static uint32_t loft_ar_index_of(loft_store_t *s, uint32_t table, uint32_t rec, uint32_t off, uint32_t stride) {
    uint32_t dir = loft_u32(s, table, LOFT_AR_DIR_FLD);
    uint32_t cap = loft_ar_dir_capacity(s, dir);
    uint32_t next = loft_u32(s, table, LOFT_AR_NEXT_FLD);
    uint32_t top = 0;
    if (next >= 2) {
        top = loft_ar_chunk_of(next - 2) + 1;
        if (top > cap) top = cap;
    }
    for (uint32_t k = top; k-- > 0;) {
        if (loft_u32(s, dir, LOFT_AR_DIR0 + 4 * k) == rec) return loft_ar_first_index_of(k) + (off - LOFT_AR_SLOT0) / stride + 1;
    }
    for (uint32_t k = top; k < cap; k++) {
        if (loft_u32(s, dir, LOFT_AR_DIR0 + 4 * k) == rec) return loft_ar_first_index_of(k) + (off - LOFT_AR_SLOT0) / stride + 1;
    }
    return 0;
}

/* arena::all_records, deleted: every chunk in directory order, then the directory. */
static void loft_ar_release(loft_store_t *s, uint32_t table) {
    uint32_t dir = loft_u32(s, table, LOFT_AR_DIR_FLD);
    if (dir == 0) return;
    uint32_t cap = loft_ar_dir_capacity(s, dir);
    for (uint32_t k = 0; k < cap; k++) {
        uint32_t r = loft_u32(s, dir, LOFT_AR_DIR0 + 4 * k);
        if (r != 0) loft_st_delete(s, r);
    }
    loft_st_delete(s, dir);
}

/* ── hash.rs ── */

typedef struct { uint32_t count, sb, stride; int inl; } loft_hhdr;

static uint32_t loft_h_slots_in(uint32_t words, uint32_t sb) {
    if (sb == 8) return words == 0 ? 0 : 1u << loft_ilog2(words);
    return words * 2;
}

static loft_hhdr loft_h_hdr(loft_store_t *s, uint32_t claim) {
    loft_hhdr h;
    uint32_t words = loft_rec_words(s, claim);
    uint32_t flags = loft_u32(s, claim, LOFT_H_STRIDE_FLD);
    words = words > LOFT_H_RESERVED ? words - LOFT_H_RESERVED : 0;
    h.stride = flags & LOFT_H_STRIDE_MASK;
    if (flags & LOFT_H_INLINE) {
        h.count = 1u << ((flags >> LOFT_H_COUNT_SHIFT) & 31);
        h.sb = 8;
        h.inl = 1;
    } else {
        h.sb = (flags & LOFT_H_TAGGED) ? 8 : LOFT_H_SLOT_BYTES;
        h.count = loft_h_slots_in(words, h.sb);
        h.inl = 0;
    }
    return h;
}

static int loft_h_is_inline(loft_store_t *s, uint32_t claim) {
    return (loft_u32(s, claim, LOFT_H_STRIDE_FLD) & LOFT_H_INLINE) != 0;
}

static uint32_t loft_h_slot_bytes(loft_store_t *s, uint32_t claim) {
    return (loft_u32(s, claim, LOFT_H_STRIDE_FLD) & LOFT_H_TAGGED) ? 8 : LOFT_H_SLOT_BYTES;
}

static uint32_t loft_h_stride(loft_store_t *s, uint32_t claim) {
    return loft_u32(s, claim, LOFT_H_STRIDE_FLD) & LOFT_H_STRIDE_MASK;
}

static uint32_t loft_h_inline_count(uint32_t words, uint32_t stride) {
    uint32_t n = (uint32_t) ((uint64_t) words * 8 / (8 + stride));
    return n == 0 ? 0 : 1u << loft_ilog2(n);
}

static uint32_t loft_h_inline_words(uint32_t count, uint32_t stride) {
    return LOFT_H_RESERVED + count * (8 + stride) / 8;
}

static uint32_t loft_h_ent0(uint32_t count) { return LOFT_H_BUCKET0 + count * 8; }

static void loft_h_put_slot(loft_store_t *s, uint32_t claim, uint32_t sb, uint32_t at, uint32_t index, uint32_t tag) {
    loft_set_u32(s, claim, LOFT_H_BUCKET0 + at * sb, index);
    if (sb == 8) loft_set_u32(s, claim, LOFT_H_BUCKET0 + at * 8 + 4, tag);
}

static int loft_h_tag_differs(loft_store_t *s, uint32_t claim, uint32_t sb, uint32_t at, uint32_t want) {
    return sb == 8 && loft_u32(s, claim, LOFT_H_BUCKET0 + at * 8 + 4) != want;
}

/* hash::stride_for: an entry's bytes, a whole number of words. */
static uint32_t loft_h_stride_for(uint32_t size) {
    if (size < 8) size = 8;
    return (size + 7) / 8 * 8;
}

static uint32_t loft_h_home(uint64_t digest, uint32_t count) {
    if (count == 0) return 0;
    if (loft_is_pow2(count)) return (uint32_t) (digest & (uint64_t) (count - 1));
    return (uint32_t) (digest % count);
}

/* `@FR-Col-Bucket`: a table is full at half load. */
static int loft_h_is_full(uint32_t length, uint32_t elms) { return (uint64_t) length * 2 >= elms; }

static uint64_t loft_h_words_for(uint64_t count) { return count + LOFT_H_RESERVED + 1; }

static uint64_t loft_h_room_for_buckets(uint64_t elms, uint32_t sb) {
    return LOFT_H_RESERVED + loft_next_pow2(elms < 2 ? 2 : elms) * sb / 8;
}

static uint64_t loft_h_read_seed(loft_store_t *s, uint32_t claim) {
    return (uint64_t) loft_u32(s, claim, LOFT_H_SEED_FLD) | (uint64_t) loft_u32(s, claim, LOFT_H_SEED_FLD + 4) << 32;
}

static void loft_h_write_seed(loft_store_t *s, uint32_t claim, uint64_t seed) {
    loft_set_u32(s, claim, LOFT_H_SEED_FLD, (uint32_t) seed);
    loft_set_u32(s, claim, LOFT_H_SEED_FLD + 4, (uint32_t) (seed >> 32));
}

/* hash::Entries: where a slot's entry lives. */
typedef struct { uint16_t store_nr; uint32_t stride, dir, cap; int inl; uint32_t claim, ent0; } loft_hents;

static loft_hents loft_h_entries(loft_store_t *s, uint32_t claim, uint16_t store_nr) {
    loft_hents e;
    loft_hhdr h = loft_h_hdr(s, claim);
    e.store_nr = store_nr;
    e.stride = h.stride;
    e.inl = h.inl;
    if (h.inl) {
        e.dir = e.cap = 0;
        e.claim = claim;
        e.ent0 = loft_h_ent0(h.count);
    } else {
        e.dir = h.stride == 0 ? 0 : loft_u32(s, claim, LOFT_AR_DIR_FLD);
        e.cap = loft_ar_dir_capacity(s, e.dir);
        e.claim = e.ent0 = 0;
    }
    return e;
}

static void loft_h_at(const loft_hents *e, loft_store_t *s, uint32_t slot, uint32_t *rec, uint32_t *pos) {
    uint32_t k, off;
    if (e->stride == 0 || (slot & LOFT_H_SLOT_RECORD)) {
        *rec = slot & ~LOFT_H_SLOT_RECORD;
        *pos = 8;
        return;
    }
    loft_ar_locate(slot, e->stride, &k, &off);
    *rec = *pos = 0;
    if (k >= e->cap) return;
    uint32_t r = loft_u32(s, e->dir, LOFT_AR_DIR0 + 4 * k);
    if (r == 0) return;
    *rec = r;
    *pos = off;
}

static void loft_h_at_bucket(const loft_hents *e, loft_store_t *s, uint32_t slot, uint32_t at, uint32_t *rec, uint32_t *pos) {
    if (e->inl) {
        *rec = e->claim;
        *pos = e->ent0 + at * e->stride;
    } else {
        loft_h_at(e, s, slot, rec, pos);
    }
}

static loft_ref loft_h_entry(const loft_hents *e, loft_store_t *s, uint32_t slot) {
    loft_ref r;
    r.store_nr = e->store_nr;
    loft_h_at(e, s, slot, &r.rec, &r.pos);
    return r;
}

static int loft_h_names(const loft_hents *e, loft_store_t *s, uint32_t slot, loft_ref rec) {
    uint32_t chunk, off;
    if (e->stride == 0 || (slot & LOFT_H_SLOT_RECORD)) return (slot & ~LOFT_H_SLOT_RECORD) == rec.rec;
    loft_h_at(e, s, slot, &chunk, &off);
    return chunk == rec.rec && rec.pos >= LOFT_AR_SLOT0 &&
           (rec.pos - LOFT_AR_SLOT0) / e->stride == (off - LOFT_AR_SLOT0) / e->stride;
}

/* hash::slot_value: what a bucket holds for the entry `rec` — its arena index, or its record. */
static uint32_t loft_h_slot_value(loft_store_t *s, uint32_t claim, loft_ref rec, uint32_t stride) {
    if (stride != 0) {
        uint32_t index = loft_ar_index_of(s, claim, rec.rec, rec.pos, stride);
        if (index != 0) return index;
    }
    return rec.rec | LOFT_H_SLOT_RECORD;
}

static uint32_t loft_h_arena_table(loft_store_t *s, uint32_t sb, uint32_t stride) {
    uint32_t claim = loft_st_claim(s, LOFT_H_RESERVED + 16 * sb / 8);
    loft_zero_fill(s, claim);
    loft_set_u32(s, claim, LOFT_H_STRIDE_FLD, stride | (sb == 8 ? LOFT_H_TAGGED : 0));
    return claim;
}

static uint32_t loft_h_inline_flags(uint32_t stride, uint32_t count) {
    return stride | LOFT_H_TAGGED | LOFT_H_INLINE | (loft_ilog2(count) << LOFT_H_COUNT_SHIFT);
}

static void loft_h_zero_slots(loft_store_t *s, uint32_t claim, uint32_t count) {
    memset(s->buf + (size_t) claim * 8 + LOFT_H_BUCKET0, 0, (size_t) count * 8);
}

static uint32_t loft_h_inline_create(loft_store_t *s, uint32_t count, uint32_t stride, uint32_t owner) {
    uint32_t claim = loft_st_claim(s, loft_h_inline_words(count, stride));
    loft_set_u32(s, claim, LOFT_H_SEED_FLD, 0);
    loft_set_u32(s, claim, LOFT_H_SEED_FLD + 4, 0);
    loft_set_u32(s, claim, LOFT_H_INLINE_LEN_FLD, 0);
    loft_set_u32(s, claim, LOFT_AR_FREE_FLD, 0);
    loft_set_u32(s, claim, LOFT_AR_OWNER_FLD, owner);
    loft_set_u32(s, claim, LOFT_AR_NEXT_FLD, 1);
    count = loft_h_inline_count(loft_rec_words(s, claim) - LOFT_H_RESERVED, stride);
    loft_set_u32(s, claim, LOFT_H_STRIDE_FLD, loft_h_inline_flags(stride, count));
    loft_h_zero_slots(s, claim, count);
    return claim;
}

/* hash::ensure_table: the collection's table, made on first use — inline when it owns its
 * entries (`@FR-Col-Inline`), with a fresh seed. */
static uint32_t loft_h_ensure_table(loft_ref c, uint32_t stride) {
    loft_store_t *s = loft_store_of(c);
    uint32_t existing = loft_collection_rec(s, c.rec, c.pos);
    if (existing != 0) return existing;
    uint32_t claim = stride != 0 ? loft_h_inline_create(s, 16, stride, c.rec) : loft_h_arena_table(s, 8, stride);
    loft_h_write_seed(s, claim, loft_fresh_seed());
    loft_set_u32(s, c.rec, c.pos, claim);
    return claim;
}

/* hash::alloc_entry: a fresh entry's bytes — a staged record for an inline table (its bytes
 * move into the table when it is filed), else a slot of the arena. */
static loft_ref loft_h_alloc_entry(loft_ref c, uint32_t stride, uint32_t owner) {
    uint32_t claim = loft_h_ensure_table(c, stride);
    loft_store_t *s = loft_store_of(c);
    loft_ref e;
    e.store_nr = c.store_nr;
    if (loft_h_is_inline(s, claim)) {
        e.rec = loft_st_claim(s, 1 + (stride + 7) / 8);
        loft_zero_fill(s, e.rec);
        loft_set_u32(s, e.rec, 4, LOFT_H_STAGE_MARK);
        e.pos = 8;
        return e;
    }
    (void) loft_ar_alloc_at(s, claim, stride, owner, &e.rec, &e.pos);
    return e;
}

static void loft_h_install_table(loft_ref c, uint32_t old_claim, uint32_t new_claim) {
    loft_store_t *s = loft_store_of(c);
    loft_set_u32(s, c.rec, c.pos, new_claim);
    if (old_claim != 0) loft_st_delete(s, old_claim);
}

/* hash::hash_free_pos: the first empty bucket from the entry's home, and its tag. */
static uint32_t loft_h_free_pos(loft_store_t *s, uint32_t claim, loft_ref rec, const loft_key *keys, int nk, uint32_t *tag) {
    uint32_t count = loft_h_hdr(s, claim).count, sb = loft_h_slot_bytes(s, claim);
    uint64_t hv = loft_ref_digest(rec, keys, nk, loft_h_read_seed(s, claim));
    uint32_t index = loft_h_home(hv, count);
    for (uint32_t i = 0; i < count; i++) {
        if (loft_u32(s, claim, LOFT_H_BUCKET0 + index * sb) == 0) break;
        if (++index >= count) index = 0;
    }
    *tag = (uint32_t) hv;
    return index;
}

static void loft_h_set(loft_store_t *s, uint32_t claim, uint32_t index, loft_ref rec, const loft_key *keys, int nk) {
    uint32_t tag, at = loft_h_free_pos(s, claim, rec, keys, nk, &tag);
    loft_h_put_slot(s, claim, loft_h_slot_bytes(s, claim), at, index, tag);
}

/* hash::rehash_into: every entry of `from` filed again in the empty `into`. */
static void loft_h_rehash_into(loft_ref c, uint32_t from, uint32_t into, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(c);
    loft_h_write_seed(s, into, loft_h_read_seed(s, from));
    loft_hents ents = loft_h_entries(s, from, c.store_nr);
    uint32_t width = loft_u32(s, into, LOFT_H_STRIDE_FLD) & LOFT_H_TAGGED;
    loft_set_u32(s, into, LOFT_AR_DIR_FLD, loft_u32(s, from, LOFT_AR_DIR_FLD));
    loft_set_u32(s, into, LOFT_AR_NEXT_FLD, loft_u32(s, from, LOFT_AR_NEXT_FLD));
    loft_set_u32(s, into, LOFT_AR_FREE_FLD, loft_u32(s, from, LOFT_AR_FREE_FLD));
    loft_set_u32(s, into, LOFT_H_STRIDE_FLD, loft_h_stride(s, from) | width);
    uint32_t length = loft_u32(s, from, LOFT_H_LEN_FLD);
    uint32_t count = loft_h_hdr(s, from).count, from_sb = loft_h_slot_bytes(s, from);
    uint32_t into_count = loft_h_hdr(s, into).count, into_sb = loft_h_slot_bytes(s, into);
    if (from_sb == 8 && loft_is_pow2(into_count)) {
        /* A tag is the digest's low half, so it places the entry without reading its key. */
        uint32_t mask = into_count - 1;
        for (uint32_t i = 0; i < count; i++) {
            uint32_t index = loft_u32(s, from, LOFT_H_BUCKET0 + 8 * i);
            if (index == 0) continue;
            uint32_t tag = loft_u32(s, from, LOFT_H_BUCKET0 + 8 * i + 4);
            uint32_t at = tag & mask;
            while (loft_u32(s, into, LOFT_H_BUCKET0 + at * into_sb) != 0) at = (at + 1) & mask;
            loft_h_put_slot(s, into, into_sb, at, index, tag);
        }
    } else {
        for (uint32_t i = 0; i < count; i++) {
            uint32_t index = loft_u32(s, from, LOFT_H_BUCKET0 + from_sb * i);
            if (index == 0) continue;
            loft_h_set(s, into, index, loft_h_entry(&ents, s, index), keys, nk);
        }
    }
    loft_set_u32(s, into, LOFT_H_LEN_FLD, length);
}

/* hash::grow_if_full: a table twice the buckets when this one is at half load, else 0. */
static uint32_t loft_h_grow_if_full(loft_ref c, uint32_t claim, uint32_t length, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(c);
    uint32_t count = loft_h_hdr(s, claim).count;
    if (!loft_h_is_full(length, count)) return 0;
    uint64_t want = loft_h_room_for_buckets((uint64_t) count * 2, 8);
    if (want > UINT32_MAX) return 0;
    uint32_t grown = loft_st_claim(s, (uint32_t) want);
    loft_zero_fill(s, grown);
    loft_set_u32(s, grown, LOFT_H_STRIDE_FLD, LOFT_H_TAGGED);
    loft_h_rehash_into(c, claim, grown, keys, nk);
    loft_h_install_table(c, claim, grown);
    return grown;
}

/* hash::inline_rebuild: an inline table of `want` buckets, the live entries moved in by tag. */
static uint32_t loft_h_inline_rebuild(loft_ref c, uint32_t claim, uint32_t want) {
    loft_store_t *s = loft_store_of(c);
    loft_hhdr h = loft_h_hdr(s, claim);
    uint32_t grown = loft_st_claim(s, loft_h_inline_words(want, h.stride));
    static const uint32_t kept[] = { LOFT_AR_OWNER_FLD, LOFT_H_SEED_FLD, LOFT_H_SEED_FLD + 4, LOFT_H_INLINE_LEN_FLD,
                                     LOFT_AR_NEXT_FLD };
    for (int i = 0; i < 5; i++) loft_set_u32(s, grown, kept[i], loft_u32(s, claim, kept[i]));
    uint32_t new_count = loft_h_inline_count(loft_rec_words(s, grown) - LOFT_H_RESERVED, h.stride);
    loft_set_u32(s, grown, LOFT_H_STRIDE_FLD, loft_h_inline_flags(h.stride, new_count));
    loft_set_u32(s, grown, LOFT_AR_FREE_FLD, 0);
    loft_h_zero_slots(s, grown, new_count);
    uint32_t mask = new_count - 1, old0 = loft_h_ent0(h.count), new0 = loft_h_ent0(new_count);
    for (uint32_t i = 0; i < h.count; i++) {
        uint32_t seq = loft_u32(s, claim, LOFT_H_BUCKET0 + i * 8);
        if (seq == 0 || seq == LOFT_H_TOMB) continue;
        uint32_t tag = loft_u32(s, claim, LOFT_H_BUCKET0 + i * 8 + 4);
        uint32_t at = tag & mask;
        while (loft_u32(s, grown, LOFT_H_BUCKET0 + at * 8) != 0) at = (at + 1) & mask;
        loft_set_u32(s, grown, LOFT_H_BUCKET0 + at * 8, seq);
        loft_set_u32(s, grown, LOFT_H_BUCKET0 + at * 8 + 4, tag);
        memcpy(s->buf + (size_t) grown * 8 + new0 + at * h.stride, s->buf + (size_t) claim * 8 + old0 + i * h.stride,
               h.stride);
    }
    loft_h_install_table(c, claim, grown);
    return grown;
}

/* hash::inline_slot_for: the bucket an inline entry of `tag` goes to, the table grown first
 * when it is full (live entries and tombstones). */
static uint32_t loft_h_inline_slot_for(loft_ref c, uint32_t claim, uint32_t tag, uint32_t *at_out) {
    loft_store_t *s = loft_store_of(c);
    loft_hhdr h = loft_h_hdr(s, claim);
    uint32_t used = loft_u32(s, claim, LOFT_H_INLINE_LEN_FLD) + loft_u32(s, claim, LOFT_AR_FREE_FLD);
    if (loft_h_is_full(used, h.count)) claim = loft_h_inline_rebuild(c, claim, h.count * LOFT_H_GROWTH);
    uint32_t mask = loft_h_hdr(s, claim).count - 1;
    uint32_t at = tag & mask;
    while (loft_u32(s, claim, LOFT_H_BUCKET0 + at * 8) != 0) at = (at + 1) & mask;
    *at_out = at;
    return claim;
}

/* hash::inline_file: bucket `at` takes the next insertion number; its entry's place answered. */
static loft_ref loft_h_inline_file(loft_store_t *s, uint16_t store_nr, uint32_t claim, uint32_t at, uint32_t tag,
                                   int zero) {
    loft_hhdr h = loft_h_hdr(s, claim);
    uint32_t seq = loft_u32(s, claim, LOFT_AR_NEXT_FLD);
    if (seq < 1) seq = 1;
    loft_set_u32(s, claim, LOFT_AR_NEXT_FLD, seq + 1);
    loft_h_put_slot(s, claim, 8, at, seq, tag);
    loft_set_u32(s, claim, LOFT_H_INLINE_LEN_FLD, loft_u32(s, claim, LOFT_H_INLINE_LEN_FLD) + 1);
    loft_ref e;
    e.store_nr = store_nr;
    e.rec = claim;
    e.pos = loft_h_ent0(h.count) + at * h.stride;
    if (zero) memset(s->buf + (size_t) claim * 8 + e.pos, 0, h.stride);
    return e;
}

static loft_ref loft_h_add(loft_ref c, loft_ref rec, const loft_key *keys, int nk);

/* hash::inline_demote: an inline table handed an entry it did not stage becomes an arena table,
 * every entry moved into an arena slot. */
static void loft_h_inline_demote(loft_ref c, uint32_t claim, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(c);
    loft_hhdr h = loft_h_hdr(s, claim);
    uint32_t owner = loft_u32(s, claim, LOFT_AR_OWNER_FLD);
    uint64_t seed = loft_h_read_seed(s, claim);
    uint32_t table = loft_h_arena_table(s, 8, h.stride);
    loft_h_write_seed(s, table, seed);
    loft_set_u32(s, c.rec, c.pos, table);
    uint32_t e0 = loft_h_ent0(h.count);
    uint8_t *bytes = (uint8_t *) malloc(h.stride ? h.stride : 1);
    if (!bytes) { fputs("loft: out of memory\n", stderr); abort(); }
    for (uint32_t at = 0; at < h.count; at++) {
        uint32_t seq = loft_u32(s, claim, LOFT_H_BUCKET0 + at * 8);
        if (seq == 0 || seq == LOFT_H_TOMB) continue;
        memcpy(bytes, s->buf + (size_t) claim * 8 + e0 + at * h.stride, h.stride);
        loft_ref e;
        e.store_nr = c.store_nr;
        (void) loft_ar_alloc_at(s, loft_u32(s, c.rec, c.pos), h.stride, owner, &e.rec, &e.pos);
        memcpy(s->buf + (size_t) e.rec * 8 + e.pos, bytes, h.stride);
        (void) loft_h_add(c, e, keys, nk);
    }
    free(bytes);
    loft_st_delete(s, claim);
}

/* hash::inline_add: a staged entry's bytes copied into its bucket and the staged record freed;
 * the entry's place in the table answered. */
static loft_ref loft_h_inline_add(loft_ref c, uint32_t claim, loft_ref rec, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(c);
    int own_entry = rec.store_nr == c.store_nr && rec.rec == claim;
    int staged = rec.store_nr == c.store_nr && rec.pos == 8 && loft_u32(loft_store_of(rec), rec.rec, 4) == LOFT_H_STAGE_MARK;
    if (!own_entry && !staged) {
        loft_h_inline_demote(c, claim, keys, nk);
        return loft_h_add(c, rec, keys, nk);
    }
    uint32_t tag = (uint32_t) loft_ref_digest(rec, keys, nk, loft_h_read_seed(s, claim)), at;
    claim = loft_h_inline_slot_for(c, claim, tag, &at);
    loft_ref placed = loft_h_inline_file(s, c.store_nr, claim, at, tag, 0);
    uint32_t stride = loft_h_hdr(s, claim).stride;
    memmove(s->buf + (size_t) placed.rec * 8 + placed.pos, loft_at(loft_store_of(rec), rec.rec, rec.pos), stride);
    if (!own_entry) loft_st_delete(loft_store_of(rec), rec.rec);
    return placed;
}

/* hash::add: file `rec` under its key; where it lives afterwards answered (an inline table moves
 * a staged entry into itself). */
static loft_ref loft_h_add(loft_ref c, loft_ref rec, const loft_key *keys, int nk) {
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (claim != 0 && loft_h_is_inline(s, claim)) return loft_h_inline_add(c, claim, rec, keys, nk);
    if (claim == 0) claim = loft_h_ensure_table(c, 0);
    uint32_t length = loft_u32(s, claim, LOFT_H_LEN_FLD);
    uint32_t index = loft_h_slot_value(s, claim, rec, loft_h_stride(s, claim));
    uint32_t grown = loft_h_grow_if_full(c, claim, length, keys, nk);
    if (grown != 0) claim = grown;
    loft_h_set(s, claim, index, rec, keys, nk);
    loft_set_u32(s, claim, LOFT_H_LEN_FLD, length + 1);
    return rec;
}

/* hash::probe_for_insert: one walk from `rec`'s home — the free bucket it takes, or the entry
 * already under its key; LOFT_H_UNKNOWN where the walk cannot answer (no table, an inline one). */
#define LOFT_H_UNKNOWN 0
#define LOFT_H_FREE 1
#define LOFT_H_PRESENT 2
typedef struct { int kind; uint32_t bucket, index, tag; loft_ref entry; } loft_hprobe;

static loft_hprobe loft_h_probe_for_insert(loft_ref c, loft_ref rec, const loft_key *keys, int nk) {
    loft_hprobe p;
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    p.kind = LOFT_H_UNKNOWN;
    if (claim == 0 || loft_rec_words(s, claim) <= LOFT_H_RESERVED || loft_h_is_inline(s, claim)) return p;
    uint32_t count = loft_h_hdr(s, claim).count, sb = loft_h_slot_bytes(s, claim);
    loft_hents ents = loft_h_entries(s, claim, c.store_nr);
    uint32_t own = loft_h_slot_value(s, claim, rec, ents.stride);
    uint64_t hv = loft_ref_digest(rec, keys, nk, loft_h_read_seed(s, claim));
    uint32_t tag = (uint32_t) hv, at = loft_h_home(hv, count);
    for (uint32_t i = 0; i < count; i++) {
        uint32_t slot = loft_u32(s, claim, LOFT_H_BUCKET0 + at * sb);
        if (slot == 0) {
            p.kind = LOFT_H_FREE;
            p.bucket = LOFT_H_BUCKET0 + at * sb;
            p.index = own;
            p.tag = tag;
            return p;
        }
        if (slot == own) return p;
        if (!loft_h_tag_differs(s, claim, sb, at, tag)) {
            loft_ref entry = loft_h_entry(&ents, s, slot);
            if (loft_key_cmp_refs(rec, entry, keys, nk) == 0) {
                p.kind = LOFT_H_PRESENT;
                p.entry = entry;
                p.bucket = LOFT_H_BUCKET0 + at * sb;
                p.index = own;
                return p;
            }
        }
        if (++at >= count) at = 0;
    }
    return p;
}

/* hash::add_at: file `rec` in the bucket a probe found free. */
static void loft_h_add_at(loft_ref c, loft_ref rec, uint32_t bucket, uint32_t index, uint32_t tag, const loft_key *keys,
                          int nk) {
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (loft_h_is_inline(s, claim)) {
        (void) loft_h_inline_add(c, claim, rec, keys, nk);
        return;
    }
    uint32_t length = loft_u32(s, claim, LOFT_H_LEN_FLD);
    uint32_t grown = loft_h_grow_if_full(c, claim, length, keys, nk);
    if (grown != 0) {
        loft_h_set(s, grown, index, rec, keys, nk);
        loft_set_u32(s, grown, LOFT_H_LEN_FLD, length + 1);
        return;
    }
    loft_set_u32(s, claim, bucket, index);
    if (loft_h_slot_bytes(s, claim) == 8) loft_set_u32(s, claim, bucket + 4, tag);
    loft_set_u32(s, claim, LOFT_H_LEN_FLD, length + 1);
}

/* hash::replace_at: bucket `bucket` names `index` instead; what it named answered. */
static uint32_t loft_h_replace_at(loft_ref c, uint32_t bucket, uint32_t index) {
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    uint32_t gone = loft_u32(s, claim, bucket);
    loft_set_u32(s, claim, bucket, index);
    return gone;
}

/* hash::reserve: buckets for `count` entries before they are filed. */
static void loft_h_reserve(loft_ref c, int64_t count, uint32_t stride, const loft_key *keys, int nk) {
    if (count < 0) return;
    uint64_t need = (loft_h_words_for((uint64_t) count) - LOFT_H_RESERVED) * 2;
    uint64_t words = loft_h_room_for_buckets(need, 8);
    if (words > UINT32_MAX) return;
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_h_ensure_table(c, stride);
    uint64_t target = loft_next_pow2(need < 2 ? 2 : need);
    if (loft_h_is_inline(s, claim)) {
        if (loft_h_hdr(s, claim).count < target && target <= UINT32_MAX) (void) loft_h_inline_rebuild(c, claim, (uint32_t) target);
        return;
    }
    if (loft_h_hdr(s, claim).count >= target) return;
    uint32_t grown = loft_st_claim(s, (uint32_t) words);
    loft_zero_fill(s, grown);
    loft_set_u32(s, grown, LOFT_H_STRIDE_FLD, LOFT_H_TAGGED);
    loft_h_rehash_into(c, claim, grown, keys, nk);
    loft_h_install_table(c, claim, grown);
}

/* hash::find: the entry whose first `n` key parts equal `kv`, or the null reference. */
static loft_ref loft_h_find(loft_ref c, const loft_key *keys, const loft_kv *kv, int n) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (claim == 0 || loft_rec_words(s, claim) == 0) return loft_ref_null();
    uint32_t count = loft_h_hdr(s, claim).count, sb = loft_h_slot_bytes(s, claim);
    uint64_t hv = loft_kv_digest(kv, n, loft_h_read_seed(s, claim));
    uint32_t tag = (uint32_t) hv, index = loft_h_home(hv, count);
    loft_hents ents = loft_h_entries(s, claim, c.store_nr);
    for (uint32_t i = 0; i < count; i++) {
        uint32_t slot = loft_u32(s, claim, LOFT_H_BUCKET0 + index * sb);
        if (slot == 0) break;
        if (!(ents.inl && slot == LOFT_H_TOMB) && !loft_h_tag_differs(s, claim, sb, index, tag)) {
            loft_ref e;
            e.store_nr = c.store_nr;
            loft_h_at_bucket(&ents, s, slot, index, &e.rec, &e.pos);
            if (loft_key_cmp_val(kv, n, s, e.rec, e.pos, keys) == 0) return e;
        }
        if (++index >= count) index = 0;
    }
    return loft_ref_null();
}

/* hash::hash_rec_pos: the bucket naming the entry `rec`, and what it holds; 0 when none does. */
static int loft_h_rec_pos(loft_store_t *s, uint32_t claim, const loft_hents *ents, loft_ref rec, const loft_key *keys,
                          int nk, uint32_t *at, uint32_t *val) {
    uint32_t count = loft_h_hdr(s, claim).count, sb = loft_h_slot_bytes(s, claim);
    uint64_t hv = loft_ref_digest(rec, keys, nk, loft_h_read_seed(s, claim));
    uint32_t tag = (uint32_t) hv, index = loft_h_home(hv, count);
    for (uint32_t i = 0; i < count; i++) {
        uint32_t v = loft_u32(s, claim, LOFT_H_BUCKET0 + index * sb);
        if (v == 0) return 0;
        if (!loft_h_tag_differs(s, claim, sb, index, tag) && loft_h_names(ents, s, v, rec)) {
            *at = index;
            *val = v;
            return 1;
        }
        if (++index >= count) index = 0;
    }
    return 0;
}

/* hash::inline_remove: the entry's bucket becomes a tombstone; its insertion number answered. */
static uint32_t loft_h_inline_remove(loft_store_t *s, uint32_t claim, loft_ref rec) {
    loft_hhdr h = loft_h_hdr(s, claim);
    uint32_t base = loft_h_ent0(h.count);
    if (rec.rec != claim || rec.pos < base || h.stride == 0) return 0;
    uint32_t at = (rec.pos - base) / h.stride;
    if (at >= h.count) return 0;
    uint32_t seq = loft_u32(s, claim, LOFT_H_BUCKET0 + at * 8);
    if (seq == 0 || seq == LOFT_H_TOMB) return 0;
    loft_set_u32(s, claim, LOFT_H_BUCKET0 + at * 8, LOFT_H_TOMB);
    uint32_t len = loft_u32(s, claim, LOFT_H_INLINE_LEN_FLD);
    loft_set_u32(s, claim, LOFT_H_INLINE_LEN_FLD, len ? len - 1 : 0);
    loft_set_u32(s, claim, LOFT_AR_FREE_FLD, loft_u32(s, claim, LOFT_AR_FREE_FLD) + 1);
    return seq;
}

/* hash::remove: the entry `rec` leaves the table, the probe run behind it shifted back into the
 * hole (`@FR-Col-Remove`); the bucket value that named it answered, 0 when none did. */
static uint32_t loft_h_remove(loft_ref c, loft_ref rec, const loft_key *keys, int nk) {
    if (rec.rec == 0 || c.store_nr == 0xFFFF || c.rec == 0) return 0;
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (claim == 0) return 0;
    if (loft_h_is_inline(s, claim)) return loft_h_inline_remove(s, claim, rec);
    uint32_t length = loft_u32(s, claim, LOFT_H_LEN_FLD);
    if (length == 0) return 0;
    uint32_t count = loft_h_hdr(s, claim).count;
    loft_hents ents = loft_h_entries(s, claim, c.store_nr);
    uint64_t seed = loft_h_read_seed(s, claim);
    uint32_t hole, gone;
    if (!loft_h_rec_pos(s, claim, &ents, rec, keys, nk, &hole, &gone)) return 0;
    uint32_t sb = loft_h_slot_bytes(s, claim);
    int by_tag = sb == 8 && loft_is_pow2(count);
    loft_set_u32(s, claim, LOFT_H_BUCKET0 + hole * sb, 0);
    uint32_t idx = hole + 1 >= count ? 0 : hole + 1;
    for (uint32_t i = 0; i < count; i++) {
        uint32_t val = loft_u32(s, claim, LOFT_H_BUCKET0 + idx * sb);
        if (val == 0) break;
        uint32_t tag = sb == 8 ? loft_u32(s, claim, LOFT_H_BUCKET0 + idx * 8 + 4) : 0;
        uint32_t ideal = by_tag ? tag & (count - 1) : loft_h_home(loft_ref_digest(loft_h_entry(&ents, s, val), keys, nk, seed), count);
        uint32_t to_hole = hole >= ideal ? hole - ideal : hole + count - ideal;
        uint32_t to_idx = idx >= ideal ? idx - ideal : idx + count - ideal;
        if (to_hole < to_idx) {
            loft_h_put_slot(s, claim, sb, hole, val, tag);
            loft_set_u32(s, claim, LOFT_H_BUCKET0 + idx * sb, 0);
            hole = idx;
        }
        idx = idx + 1 >= count ? 0 : idx + 1;
    }
    loft_set_u32(s, claim, LOFT_H_LEN_FLD, length - 1);
    return gone;
}

/* hash::free_slot: the arena slot a removal's bucket value named goes back to the arena. */
static void loft_h_free_slot(loft_ref c, uint32_t slot) {
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (claim == 0 || slot == 0 || (slot & LOFT_H_SLOT_RECORD) || loft_h_is_inline(s, claim)) return;
    uint32_t width = loft_h_stride(s, claim);
    if (width != 0) loft_ar_free(s, claim, slot, width);
}

/* hash::free_entry: the entry `rec`'s storage goes back — a staged record, or its arena slot. */
static void loft_h_free_entry(loft_ref c, loft_ref rec) {
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (claim == 0) return;
    if (loft_h_is_inline(s, claim)) {
        if (rec.rec != claim) loft_st_delete(s, rec.rec);
        return;
    }
    uint32_t width = loft_h_stride(s, claim);
    if (width == 0) return;
    loft_ar_free(s, claim, loft_ar_index_of(s, claim, rec.rec, rec.pos, width), width);
}

static int64_t loft_hash_len(loft_ref c) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (claim == 0) return 0;
    uint32_t flags = loft_u32(s, claim, LOFT_H_STRIDE_FLD);
    return loft_u32(s, claim, (flags & LOFT_H_INLINE) ? LOFT_H_INLINE_LEN_FLD : LOFT_H_LEN_FLD);
}

/* hash::entries: every live entry, with whether the table's arena or inline slots hold it
 * (`ours`) rather than a record of its own — an inline table's in insertion order, an arena
 * table's in bucket order.  malloc'd; `*n` the count. */
typedef struct { loft_ref e; uint32_t seq; int ours; } loft_hent;

static int loft_hent_by_seq(const void *a, const void *b) {
    uint32_t x = ((const loft_hent *) a)->seq, y = ((const loft_hent *) b)->seq;
    return x < y ? -1 : x > y;
}

static loft_hent *loft_h_entries_of(loft_ref c, uint32_t *n) {
    *n = 0;
    if (c.store_nr == 0xFFFF || c.rec == 0) return 0;
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_collection_rec(s, c.rec, c.pos);
    if (claim == 0) return 0;
    loft_hhdr h = loft_h_hdr(s, claim);
    loft_hent *out = (loft_hent *) malloc(((size_t) h.count + 1) * sizeof *out);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    if (h.inl) {
        uint32_t e0 = loft_h_ent0(h.count);
        for (uint32_t at = 0; at < h.count; at++) {
            uint32_t seq = loft_u32(s, claim, LOFT_H_BUCKET0 + at * 8);
            if (seq == 0 || seq == LOFT_H_TOMB) continue;
            out[*n].e.store_nr = c.store_nr;
            out[*n].e.rec = claim;
            out[*n].e.pos = e0 + at * h.stride;
            out[*n].seq = seq;
            out[*n].ours = 1;
            (*n)++;
        }
        qsort(out, *n, sizeof *out, loft_hent_by_seq);
        return out;
    }
    loft_hents ents = loft_h_entries(s, claim, c.store_nr);
    for (uint32_t i = 0; i < h.count; i++) {
        uint32_t index = loft_u32(s, claim, LOFT_H_BUCKET0 + i * h.sb);
        if (index == 0) continue;
        out[*n].e = loft_h_entry(&ents, s, index);
        out[*n].seq = index;
        out[*n].ours = h.stride != 0 && (index & LOFT_H_SLOT_RECORD) == 0;
        (*n)++;
    }
    return out;
}

/* Entries put in key order (a merge sort: the keys are unique). */
static void loft_sort_refs(loft_ref *a, loft_ref *tmp, uint32_t n, const loft_key *keys, int nk) {
    if (n < 2) return;
    uint32_t h = n / 2;
    loft_sort_refs(a, tmp, h, keys, nk);
    loft_sort_refs(a + h, tmp, n - h, keys, nk);
    uint32_t i = 0, j = h, k = 0;
    while (i < h && j < n) {
        if (loft_key_cmp_refs(a[j], a[i], keys, nk) < 0) tmp[k++] = a[j++];
        else tmp[k++] = a[i++];
    }
    while (i < h) tmp[k++] = a[i++];
    while (j < n) tmp[k++] = a[j++];
    memcpy(a, tmp, (size_t) n * sizeof *a);
}

/* hash::records_sorted (`@FR-Col-Order`), or with `by_slot` hash::records_by_slot
 * (`@FR-Col-Unordered`: inline insertion order, else bucket values ascending): the entries,
 * malloc'd; `*n` the count. */
static int loft_hent_by_slot(const void *a, const void *b) { return loft_hent_by_seq(a, b); }

static loft_ref *loft_hash_refs(loft_ref c, const loft_key *keys, int nk, int by_slot, uint32_t *n) {
    loft_hent *all = loft_h_entries_of(c, n);
    loft_ref *out = (loft_ref *) malloc(((size_t) *n + 1) * 2 * sizeof *out);
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    if (by_slot && *n) qsort(all, *n, sizeof *all, loft_hent_by_slot);
    for (uint32_t i = 0; i < *n; i++) out[i] = all[i].e;
    free(all);
    if (!by_slot) loft_sort_refs(out, out + *n + 1, *n, keys, nk);
    return out;
}

/* ── the Stores routines over a hash (src/database) ── */

/* Stores::hash_entry: a fresh entry of a hash whose elements are `size` bytes — a record of its
 * own when the element type is `linked` (a sibling or a stored reference names it by record),
 * else the table's (`@FR-Col-Inline`).  Its bytes are unwritten (`@FR-H-Claim`). */
static loft_ref loft_hash_entry(loft_ref c, uint32_t size, int linked) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    if (linked) {
        loft_store_t *s = loft_store_of(c);
        loft_ref e = { c.store_nr, loft_st_claim(s, 1 + (size + 7) / 8), 8 };
        loft_set_u32(s, e.rec, 4, c.rec);
        return e;
    }
    return loft_h_alloc_entry(c, loft_h_stride_for(size), c.rec);
}

static int loft_hash_owns_entries(loft_ref c) {
    loft_store_t *s = loft_store_of(c);
    uint32_t claim = loft_u32(s, c.rec, c.pos);
    return claim != 0 && claim != 0xFFFFFFFFu && loft_h_stride(s, claim) != 0;
}

static void loft_forget_in_scratches(loft_ref c, loft_ref e);

/* Stores::unlink on a hash: out of every walk over it, then out of the table. */
static uint32_t loft_hash_unlink(loft_ref c, loft_ref e, const loft_key *keys, int nk) {
    loft_forget_in_scratches(c, e);
    return loft_h_remove(c, e, keys, nk);
}

/* Stores::free_hash_entry: the entry's storage, through the bucket value the unlink answered
 * where there is one. */
static void loft_free_hash_entry(loft_ref c, loft_ref e, uint32_t slot) {
    if (slot == 0) loft_h_free_entry(c, e);
    else loft_h_free_slot(c, slot);
}

/* Stores::remove_owned on a hash: unlinked, then released — its claims and its storage.  With
 * `view` (a member that borrows its records, CLEAR_KEYED_VIEW) only unlinked. */
static void loft_hash_remove(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref e, const loft_key *keys, int nk,
                             int view) {
    if (e.rec == 0 || c.store_nr == 0xFFFF || c.rec == 0) return;
    uint32_t slot = loft_hash_unlink(c, e, keys, nk);
    if (view) return;
    if (loft_hash_owns_entries(c)) {
        loft_remove_claims(T, elem, e);
        loft_free_hash_entry(c, e, slot);
        return;
    }
    loft_ref rec = { e.store_nr, e.rec, 8 };
    loft_remove_claims(T, elem, rec);
    loft_st_delete(loft_store_of(c), e.rec);
}

/* Stores::displace_keyed: the entry `existing` makes room for an insert under its key — unlinked,
 * and released unless a `secondary` member only borrows it. */
static void loft_hash_displace(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref existing, const loft_key *keys,
                               int nk, int secondary) {
    uint32_t slot = loft_hash_unlink(c, existing, keys, nk);
    if (secondary) return;
    loft_remove_claims(T, elem, existing);
    loft_free_hash_entry(c, existing, slot);
}

/* Stores::insert_record on a hash (`@FR-Col-Insert`: latest insert wins): `rec` filed under its
 * key; the entry it displaced answered (released already unless `secondary`), or the null
 * reference. */
static loft_ref loft_hash_insert(const loft_tdesc *T, uint16_t elem, loft_ref c, loft_ref rec, const loft_key *keys,
                                 int nk, int secondary) {
    if (c.store_nr == 0xFFFF || c.rec == 0 || rec.rec == 0) return loft_ref_null();
    loft_hprobe p = loft_h_probe_for_insert(c, rec, keys, nk);
    if (p.kind == LOFT_H_FREE) {
        loft_h_add_at(c, rec, p.bucket, p.index, p.tag, keys, nk);
        return loft_ref_null();
    }
    if (p.kind == LOFT_H_PRESENT) {
        /* The new entry takes the displaced one's bucket. */
        loft_forget_in_scratches(c, p.entry);
        uint32_t gone = loft_h_replace_at(c, p.bucket, p.index);
        if (!secondary) {
            loft_remove_claims(T, elem, p.entry);
            loft_free_hash_entry(c, p.entry, gone);
        }
        return p.entry;
    }
    /* Stores::dedup_keyed: the entry under the key, displaced, then the add. */
    loft_kv key[8];
    int n = nk > 8 ? 8 : nk;
    for (int i = 0; i < n; i++) key[i] = loft_key_get(loft_store_of(rec), rec.rec, rec.pos, &keys[i]);
    loft_ref existing = loft_h_find(c, keys, key, n);
    loft_ref displaced = loft_ref_null();
    if (existing.rec != 0 && (existing.rec != rec.rec || existing.pos != rec.pos)) {
        loft_hash_displace(T, elem, c, existing, keys, nk, secondary);
        displaced = existing;
    }
    (void) loft_h_add(c, rec, keys, nk);
    return displaced;
}

/* Stores::reserve_hash: buckets for `count` entries (`@PLN135` arc C). */
static void loft_hash_reserve(const loft_tdesc *T, uint16_t tp, loft_ref c, int64_t count) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return;
    const loft_tdesc *d = &T[tp];
    uint32_t stride = T[d->elem].linked ? 0 : loft_h_stride_for(T[d->elem].size);
    loft_h_reserve(c, count, stride, d->keys, d->nk);
}

/* Stores::copy_claims_hash_body: the destination emptied, sized for every entry, and each entry
 * of the source re-filed into it — in the source's storage order (an arena's hand-out order, an
 * inline table's insertion order), so a copy's layout follows its source's. */
static int loft_ref_by_place(const void *a, const void *b) {
    const loft_hent *x = (const loft_hent *) a, *y = (const loft_hent *) b;
    if (x->e.rec != y->e.rec) return x->e.rec < y->e.rec ? -1 : 1;
    return x->e.pos < y->e.pos ? -1 : x->e.pos > y->e.pos;
}

static void loft_hash_copy_claims(const loft_tdesc *T, uint16_t tp, loft_ref from, loft_ref to) {
    const loft_tdesc *d = &T[tp];
    loft_store_t *fs = loft_store_of(from);
    uint32_t cur = loft_u32(fs, from.rec, from.pos);
    /* An absent collection is copied absent (`copy_absent_collection`). */
    loft_set_u32(loft_store_of(to), to.rec, to.pos, cur == 0xFFFFFFFFu ? cur : 0);
    if (cur == 0 || cur == 0xFFFFFFFFu) return;
    int owned = loft_h_stride(fs, cur) != 0, inl = loft_h_is_inline(fs, cur);
    uint32_t n, kept = 0;
    loft_hent *all = loft_h_entries_of(from, &n);
    for (uint32_t i = 0; i < n; i++) {
        if (owned && !all[i].ours) continue;
        all[kept++] = all[i];
    }
    if (!inl && kept) qsort(all, kept, sizeof *all, loft_ref_by_place);
    loft_hash_reserve(T, tp, to, kept);
    uint32_t size = T[d->elem].size;
    for (uint32_t i = 0; i < kept; i++) {
        loft_ref e = loft_hash_entry(to, size, T[d->elem].linked);
        loft_copy_bytes(e, 0, all[i].e, 0, size);
        loft_copy_claims(T, d->elem, all[i].e, e);
        (void) loft_h_add(to, e, d->keys, d->nk);
    }
    free(all);
}

/* Stores::remove_claims on a hash field: every entry it owns released, then its storage — the
 * arena's chunks, the table — and the field emptied.  A `borrowed` view releases its own
 * storage only (`borrowed_spine`). */
static void loft_hash_remove_claims(const loft_tdesc *T, uint16_t tp, loft_ref r, int borrowed) {
    loft_store_t *s = loft_store_of(r);
    uint32_t cur = loft_u32(s, r.rec, r.pos);
    const loft_tdesc *d = &T[tp];
    if (cur == 0xFFFFFFFFu || cur == 0) {
        loft_set_u32(s, r.rec, r.pos, 0);
        return;
    }
    int owned = loft_h_stride(s, cur) != 0;
    /* release_plain_hash: entries that own nothing go back with the table's own storage. */
    if (!borrowed && !(owned && !T[d->elem].owns_heap)) {
        uint32_t n;
        loft_hent *all = loft_h_entries_of(r, &n);
        for (uint32_t i = 0; i < n; i++) {
            if (owned && !all[i].ours) continue;
            loft_remove_claims(T, d->elem, all[i].e);
            if (!all[i].ours) loft_st_delete(s, all[i].e.rec);
        }
        free(all);
    }
    if (owned && !loft_h_is_inline(s, cur)) loft_ar_release(s, cur);
    loft_st_delete(s, cur);
    loft_set_u32(s, r.rec, r.pos, 0);
}

/* ── iteration scratches (Stores::build_ref_scratch / build_rec_scratch, vector::step_ordered)
 * A walk over a hash, a spatial or a trie reads a snapshot taken when it starts, kept in the
 * collection's store: a header record (@4 the element vector, @8 the elements' store, @12 the
 * tag 0x5C4A0000 | the element width) and a vector of elements — a hash's (record, offset)
 * pairs, 8 bytes each, or a tree's record numbers, 4 bytes each.  An element that leaves the
 * collection mid-walk reads 0 there (`forget_in_scratches`, loft#1710), which a step passes. */
#define LOFT_SCRATCH_MARK 0x5C4A0000u

typedef struct { loft_ref source, scratch; } loft_live_scratch;
static loft_live_scratch *loft_scratches = 0;
static int loft_nscratches = 0, loft_scratch_cap = 0;

static void loft_forget_in_scratches(loft_ref c, loft_ref e) {
    for (int i = 0; i < loft_nscratches; i++) {
        loft_live_scratch *w = &loft_scratches[i];
        if (w->source.store_nr != c.store_nr || w->source.rec != c.rec || w->source.pos != c.pos) continue;
        loft_store_t *s = loft_store_of(w->scratch);
        uint32_t vec = loft_u32(s, w->scratch.rec, 4), len = loft_u32(s, vec, 4);
        uint32_t width = loft_u32(s, w->scratch.rec, 12) & 0xFFFFu;
        for (uint32_t j = 0; j < len; j++) {
            if (loft_u32(s, vec, 8 + j * width) == e.rec && (width == 4 || loft_u32(s, vec, 12 + j * 8) == e.pos))
                loft_set_u32(s, vec, 8 + j * width, 0);
        }
    }
}

/* The snapshot of `n` elements `width` bytes each — (record, offset) pairs, or record numbers. */
static loft_ref loft_scratch_of(loft_ref c, const loft_ref *e, const uint32_t *recs, uint32_t n, uint32_t width) {
    loft_store_t *s = loft_store_of(c);
    uint32_t words = (n * width + 8 + 7) / 8;
    uint32_t vec = loft_st_claim(s, words < 1 ? 1 : words);
    uint32_t header = loft_st_claim(s, 2);
    loft_set_u32(s, vec, 4, n);
    for (uint32_t i = 0; i < n; i++) {
        if (width == 4) {
            loft_set_u32(s, vec, 8 + i * 4, recs[i]);
        } else {
            loft_set_u32(s, vec, 8 + i * 8, e[i].rec);
            loft_set_u32(s, vec, 12 + i * 8, e[i].pos);
        }
    }
    loft_set_u32(s, header, 4, vec);
    loft_set_u32(s, header, 8, c.store_nr);
    loft_set_u32(s, header, 12, LOFT_SCRATCH_MARK | width);
    loft_ref x = { c.store_nr, header, 4 };
    if (loft_nscratches == loft_scratch_cap) {
        loft_scratch_cap = loft_scratch_cap ? loft_scratch_cap * 2 : 8;
        loft_scratches = (loft_live_scratch *) realloc(loft_scratches, (size_t) loft_scratch_cap * sizeof *loft_scratches);
        if (!loft_scratches) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_scratches[loft_nscratches].source = c;
    loft_scratches[loft_nscratches].scratch = x;
    loft_nscratches++;
    return x;
}

static loft_ref loft_ref_scratch(loft_ref c, const loft_ref *e, uint32_t n) { return loft_scratch_of(c, e, 0, n, 8); }

static loft_ref loft_rec_scratch(loft_ref c, const uint32_t *recs, uint32_t n) { return loft_scratch_of(c, 0, recs, n, 4); }

/* n_hash_sorted / n_hash_unsorted (Stores::build_hash_sorted_vec / build_hash_unsorted_vec):
 * the walk's snapshot, in key order or in slot order. */
static loft_ref loft_hash_walk(loft_ref c, const loft_key *keys, int nk, int by_slot) {
    if (c.store_nr == 0xFFFF || c.rec == 0) return loft_ref_null();
    uint32_t n;
    loft_ref *e = loft_hash_refs(c, keys, nk, by_slot, &n);
    loft_ref x = loft_ref_scratch(c, e, n);
    free(e);
    return x;
}

static loft_ref loft_hash_sorted(loft_ref c, const loft_key *keys, int nk) { return loft_hash_walk(c, keys, nk, 0); }

static loft_ref loft_hash_unsorted(loft_ref c) { return loft_hash_walk(c, 0, 0, 1); }

/* OpFreeScratch (Stores::free_iteration_scratch): the snapshot's two records go back. */
static void loft_free_scratch(loft_ref x) {
    if (x.store_nr == 0xFFFF || x.rec <= 1) return;
    for (int i = 0; i < loft_nscratches; i++) {
        if (loft_scratches[i].scratch.store_nr != x.store_nr || loft_scratches[i].scratch.rec != x.rec) continue;
        loft_scratches[i] = loft_scratches[--loft_nscratches];
        break;
    }
    loft_store_t *s = loft_store_of(x);
    if (!s->live || loft_st_i32(s, x.rec, 0) < 2 || (loft_u32(s, x.rec, 12) & 0xFFFF0000u) != 0x5C4A0000u) return;
    uint32_t vec = loft_u32(s, x.rec, 4);
    loft_st_delete(s, x.rec);
    if (vec != x.rec && vec != 0) loft_st_delete(s, vec);
}
