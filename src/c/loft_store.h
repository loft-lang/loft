/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

/* ── The store allocator ───────────────────────────────────────────────────────────────────
 * A PORT of `src/store.rs`'s allocator, line for line, with every switch at its default: the
 * memory model is loft's, so a store holds the same bytes at the same positions as on the
 * interpreter and `--native` (@PLN200 B5, owner).  Change `Store` first and this after it; the
 * names follow the Rust ones (`claim` → `loft_st_claim`, `fl_insert` → `loft_fl_insert`).
 *
 * A store is `size` 8-byte words.  Word 0 holds the signature and is never a record; a block
 * starts with an `int32_t` header — its size in words, NEGATIVE when free — and a free block
 * also carries a FOOTER (`-words`) in the high half of its last word (`@FR-H-FreeFooter`).  A
 * free block of two words or more is a node of the LLRB free tree, keyed on (size, position):
 * its left link at byte 4, its right link at byte 8 with the node's colour in bit 31.  The
 * free block that ends the store is the WILDERNESS and stays out of the tree
 * (`@FR-H-Wilderness`); a freed block of 2..16 words is PARKED on the list of its size
 * (`@FR-H-Park`); a freed small block of a store that has not swept yet is left untracked in
 * `lazy_free` (`@FR-H-LazyFree`).  `claims` names every live record's start, `referenced` every
 * record a stored reference names, `removed` every tombstone a removal of one left
 * (`@FR-H-Removed`). */

#define LOFT_ST_SIGNATURE 0x53746f31u
#define LOFT_ST_PRIMARY 1u
#define LOFT_MIN_FREE_TREE 2
#define LOFT_FL_LEFT 4u
#define LOFT_FL_RIGHT 8u
#define LOFT_FL_RED_BIT 0x80000000u
#define LOFT_FL_RIGHT_MASK 0x7FFFFFFFu
#define LOFT_LAZY_FLOOR_WORDS 256u
#define LOFT_LAZY_MAX_WORDS 64
#define LOFT_PARK_MAX_WORDS 16
#define LOFT_PARK_PREV 4u
#define LOFT_PARK_NEXT 8u

/* `Claims`: one bit per word, grown on demand to the highest position inserted.  `nwords` is
 * the length and `cap` the allocation, as a `Vec<u64>`'s: growth doubles the allocation and a
 * clear keeps it, so a store cleared and refilled in a loop allocates nothing. */
typedef struct { uint64_t *bits; uint32_t nwords; uint32_t live; uint32_t cap; } loft_bits;

static int loft_bits_contains(const loft_bits *b, uint32_t pos) {
    uint32_t w = pos >> 6;
    return w < b->nwords && ((b->bits[w] >> (pos & 63)) & 1u);
}

static int loft_bits_insert(loft_bits *b, uint32_t pos) {
    uint32_t w = pos >> 6;
    if (w >= b->nwords) {
        uint32_t n = w + 1;
        if (n > b->cap) {
            uint32_t cap = b->cap * 2 > n ? b->cap * 2 : n;
            uint64_t *nb = (uint64_t *) realloc(b->bits, (size_t) cap * 8);
            if (!nb) { fputs("loft: out of memory\n", stderr); abort(); }
            b->bits = nb;
            b->cap = cap;
        }
        memset(b->bits + b->nwords, 0, (size_t) (n - b->nwords) * 8);
        b->nwords = n;
    }
    uint64_t mask = (uint64_t) 1 << (pos & 63);
    int fresh = (b->bits[w] & mask) == 0;
    b->bits[w] |= mask;
    b->live += (uint32_t) fresh;
    return fresh;
}

static int loft_bits_remove(loft_bits *b, uint32_t pos) {
    uint32_t w = pos >> 6;
    uint64_t mask = (uint64_t) 1 << (pos & 63);
    if (w < b->nwords && (b->bits[w] & mask)) {
        b->bits[w] &= ~mask;
        b->live--;
        return 1;
    }
    return 0;
}

static void loft_bits_clear(loft_bits *b) {
    b->nwords = 0;
    b->live = 0;
}

static int loft_lz64(uint64_t x) {
    int n = 0;
    if (!x) return 64;
    while (!(x & ((uint64_t) 1 << 63))) { x <<= 1; n++; }
    return n;
}

static int loft_tz64(uint64_t x) {
    int n = 0;
    if (!x) return 64;
    while (!(x & 1u)) { x >>= 1; n++; }
    return n;
}

/* The highest set position below `end`, or 0 with *found = 0. */
static uint32_t loft_bits_prev_start(const loft_bits *b, uint32_t end, int *found) {
    *found = 0;
    if (end == 0 || b->nwords == 0) return 0;
    uint32_t last = end - 1;
    uint32_t w = last >> 6;
    if (w > b->nwords - 1) w = b->nwords - 1;
    uint64_t word = w == (last >> 6) ? b->bits[w] & (UINT64_MAX >> (63 - (last & 63))) : b->bits[w];
    for (;;) {
        if (word) { *found = 1; return w * 64 + 63 - (uint32_t) loft_lz64(word); }
        if (w == 0) return 0;
        w--;
        word = b->bits[w];
    }
}

/* The lowest set position at or after `from`, or 0 with *found = 0. */
static uint32_t loft_bits_next_at_or_after(const loft_bits *b, uint32_t from, int *found) {
    uint32_t w = from >> 6;
    *found = 0;
    if (w >= b->nwords) return 0;
    uint64_t bits = b->bits[w] & (~(uint64_t) 0 << (from & 63));
    for (;;) {
        if (bits) { *found = 1; return (w << 6) | (uint32_t) loft_tz64(bits); }
        w++;
        if (w >= b->nwords) return 0;
        bits = b->bits[w];
    }
}

/* `FOREIGN_REC` (@PLN174): the one synthetic record id a FOREIGN store serves its bytes under —
 * a vector handle names it in its collection slot exactly as it names a claimed record, and
 * `loft_at` answers it from the foreign bytes.  Past any record a store can hold.  The emitter
 * defines `LOFT_FOREIGN` only for a unit that reaches `file_map`, so no other unit pays the
 * test in `loft_at`; the id itself is defined for every unit, which names it in a gate. */
#define LOFT_FOREIGN_REC 0x7FFFFFF0u

#ifdef LOFT_FOREIGN

/* `ForeignOwner`: what keeps the foreign bytes alive, released with the last span over them. */
typedef struct {
    uint32_t refs;
    void *bytes;
    size_t n;
    void (*release)(void *bytes, size_t n);
} loft_fowner;

/* `ForeignSpan`: `len` elements of `elem` bytes at `base`, and their owner (shared). */
typedef struct { const uint8_t *base; uint32_t len, elem; loft_fowner *owner; } loft_fspan;

/* `Foreign`: the span a store serves, the header (size word, length) a vector read expects at
 * +0 and +4 of the record, and the collection slot that names `LOFT_FOREIGN_REC`. */
typedef struct { loft_fspan span; uint8_t header[8]; uint32_t slot_rec, slot_pos; } loft_foreign;
#endif

/* `loft_store_t`: the `Store` fields the allocator reads, and the C runtime's own beside them
 * (`known`: the type a wrapper mint gave the store's first record, 0xFFFF for any other;
 * `protect`: the calls in flight that hold this store as an argument). */
typedef struct {
    uint8_t *buf;
    uint32_t size;
    int live;
    uint16_t known;
    int pinned;
    int protect;
    /* `Store::read_only` set by the author's `#lock = true` (`@FR-H-WriteLocked`). */
    int read_only;
    uint64_t serial;
    loft_bits claims, referenced, removed, lazy_free, parked_set;
    uint32_t parked_heads[LOFT_PARK_MAX_WORDS + 1];
    uint32_t parked_count;
    uint32_t claimed_end, free_root, wild, dead_words;
    int needs_coalesce, lazy;
    /* `Store::defined`: the buffer was zeroed when made, and a growth zeroes its new tail. */
    int defined;
    /* `Store::park` off — a store loaded from an image parks nothing (`Store::load`). */
    int no_park;
    /* `Store::placed_foreign` (`@FR-H-RootExtent`): the live records placed here that are not
     * elements of the root collection; while any lives, a clear does not reset the store and
     * no exchange takes its content. */
    uint32_t *placed, nplaced, placed_cap;
    /* `Stores::frame_held` (`@FR-G-Arg-Rebind`): the live generator frames holding this store
     * as an argument, and whether its owner released it while they did. */
    uint32_t frame_holds;
    int orphaned;
#ifdef LOFT_FOREIGN
    /* `Store::foreign`: the bytes this store serves under `LOFT_FOREIGN_REC`; an owner of 0 for
     * every ordinary store. */
    loft_foreign foreign;
#endif
} loft_store_t;

/* A header built without `loft_cc.h` (the replays of tests/c_emit.rs) keeps one copy of its
 * state: no threads. */
#ifndef LOFT_TLS
#define LOFT_TLS
#endif
static LOFT_TLS loft_store_t *loft_stores = 0;
static LOFT_TLS uint32_t loft_nstores = 0;
static LOFT_TLS uint64_t loft_stores_allocated = 0;

/* ── The heap ceiling (store_budget.rs) ──────────────────────────────────────────────────
 * `loft_heap_total` is the bytes every store's buffer holds; `LOFT_MEMORY_LIMIT` sets
 * `loft_heap_limit` (0: none, the default).  Every site that sizes a store's buffer calls
 * `loft_heap_moved` with the buffer's old and new word counts BEFORE it allocates, so a growth
 * that would cross the ceiling stops the run with the store as it was, and the report names the
 * growth that was refused.  The names are the unit's type table, which a generated `main` binds
 * (`loft_bind_tables`); a runtime built without a unit names no type. */
static const char *const *loft_tn = 0;
static uint32_t loft_ntn = 0;
/* Each thread counts its own stores' bytes; the limit is the program's. */
static LOFT_TLS uint64_t loft_heap_total = 0;
static uint64_t loft_heap_limit = 0;

static void loft_heap_human(char *out, uint64_t b) {
    if (b >= ((uint64_t) 1 << 30)) sprintf(out, "%.1f GiB", (double) b / (double) ((uint64_t) 1 << 30));
    else if (b >= ((uint64_t) 1 << 20)) sprintf(out, "%.1f MiB", (double) b / (double) ((uint64_t) 1 << 20));
    else if (b >= ((uint64_t) 1 << 10)) sprintf(out, "%.1f KiB", (double) b / (double) ((uint64_t) 1 << 10));
    else sprintf(out, "%llu B", (unsigned long long) b);
}

/* `parse_limit`: `2G`, `512M`, `4k`, a bare number of bytes, an optional `B`; 0 when the text is
 * no size, with `*ok` cleared. */
static uint64_t loft_heap_parse_limit(const char *t, int *ok) {
    size_t n = strlen(t);
    uint64_t scale = 1, v = 0;
    *ok = 0;
    while (n && (t[n - 1] == ' ' || t[n - 1] == '\t')) n--;
    while (n && (*t == ' ' || *t == '\t')) { t++; n--; }
    if (n && (t[n - 1] == 'b' || t[n - 1] == 'B')) n--;
    if (n && (t[n - 1] == 'k' || t[n - 1] == 'K')) { scale = (uint64_t) 1 << 10; n--; }
    else if (n && (t[n - 1] == 'm' || t[n - 1] == 'M')) { scale = (uint64_t) 1 << 20; n--; }
    else if (n && (t[n - 1] == 'g' || t[n - 1] == 'G')) { scale = (uint64_t) 1 << 30; n--; }
    if (!n) return 0;
    for (size_t i = 0; i < n; i++) {
        if (t[i] < '0' || t[i] > '9') return 0;
        if (v > (UINT64_MAX - 9) / 10) return 0;
        v = v * 10 + (uint64_t) (t[i] - '0');
    }
    if (v && scale > UINT64_MAX / v) return 0;
    *ok = 1;
    return v * scale;
}

/* `apply_env_limit_if_set`: an ordinary run is capped only when the variable is set; a typo is
 * reported and leaves the run uncapped, as the interpreter's default for an ordinary run. */
static void loft_heap_read_limit(void) {
    const char *v = getenv("LOFT_MEMORY_LIMIT");
    int ok = 0;
    if (!v) return;
    loft_heap_limit = loft_heap_parse_limit(v, &ok);
    if (!ok) {
        fprintf(stderr, "loft: LOFT_MEMORY_LIMIT='%s' is not a size (try 2G, 512M or 0) — keeping the default 0 B\n", v);
        loft_heap_limit = 0;
    }
}

static void loft_heap_moved(const loft_store_t *s, uint32_t old_words, uint32_t new_words) {
    uint64_t old_b = (uint64_t) old_words * 8, new_b = (uint64_t) new_words * 8;
    if (new_b <= old_b) {
        uint64_t freed = old_b - new_b;
        loft_heap_total = loft_heap_total > freed ? loft_heap_total - freed : 0;
        return;
    }
    if (loft_heap_limit && loft_heap_total + (new_b - old_b) > loft_heap_limit) {
        char used[32], cap[32], from[32], to[32];
        uint16_t kt = s->known;
        loft_heap_human(used, loft_heap_total);
        loft_heap_human(cap, loft_heap_limit);
        loft_heap_human(from, old_b);
        loft_heap_human(to, new_b);
        fflush(stdout);
        fprintf(stderr, "loft: store memory limit reached — %s in use, limit %s\n\n  the growth that crossed it\n", used, cap);
        if (kt != 0xFFFF && kt < loft_ntn)
            fprintf(stderr, "    a store of type `%s` (kt=%u) growing %s → %s\n", loft_tn[kt], (unsigned) kt, from, to);
        else
            fprintf(stderr, "    a store growing %s → %s\n", from, to);
        fputs("\n  Raise or remove the limit with LOFT_MEMORY_LIMIT=<size|0>, e.g. 8G.\n", stderr);
        exit(101);
    }
    loft_heap_total += new_b - old_b;
}

#ifdef LOFT_ALLOC_TRACE
/* `LOFT_ALLOC_TRACE=<file>` — the same event lines `store_census::alloc_event` writes. */
static FILE *loft_trace_file(void) {
    static FILE *f = 0;
    static int opened = 0;
    if (!opened) {
        const char *p = getenv("LOFT_ALLOC_TRACE");
        opened = 1;
        if (p && *p) f = fopen(p, "w");
        if (f) setvbuf(f, 0, _IONBF, 0);
    }
    return f;
}
#define LOFT_TRACE(...) do { FILE *loft_tf_ = loft_trace_file(); if (loft_tf_) fprintf(loft_tf_, __VA_ARGS__); } while (0)
#else
#define LOFT_TRACE(...) do { } while (0)
#endif

static int32_t loft_st_i32(const loft_store_t *s, uint32_t rec, uint32_t fld) {
    int32_t v;
    memcpy(&v, s->buf + (size_t) rec * 8 + fld, 4);
    return v;
}

static uint32_t loft_st_u32(const loft_store_t *s, uint32_t rec, uint32_t fld) {
    uint32_t v;
    memcpy(&v, s->buf + (size_t) rec * 8 + fld, 4);
    return v;
}

static void loft_st_set_i32(loft_store_t *s, uint32_t rec, uint32_t fld, int32_t v) {
    memcpy(s->buf + (size_t) rec * 8 + fld, &v, 4);
}

static void loft_st_set_u32(loft_store_t *s, uint32_t rec, uint32_t fld, uint32_t v) {
    memcpy(s->buf + (size_t) rec * 8 + fld, &v, 4);
}

static void loft_set_free_header(loft_store_t *s, uint32_t pos, int32_t words) {
    loft_st_set_i32(s, pos, 0, -words);
    loft_st_set_i32(s, pos + (uint32_t) words - 1, 4, -words);
}

/* ---- the LLRB free-space tree ---- */

static int32_t loft_fl_size(const loft_store_t *s, uint32_t p) { return -loft_st_i32(s, p, 0); }
static uint32_t loft_fl_left(const loft_store_t *s, uint32_t p) { return loft_st_u32(s, p, LOFT_FL_LEFT); }
static uint32_t loft_fl_right(const loft_store_t *s, uint32_t p) { return loft_st_u32(s, p, LOFT_FL_RIGHT) & LOFT_FL_RIGHT_MASK; }
static int loft_fl_red(const loft_store_t *s, uint32_t p) { return (loft_st_u32(s, p, LOFT_FL_RIGHT) & LOFT_FL_RED_BIT) != 0; }
static void loft_fl_set_left(loft_store_t *s, uint32_t p, uint32_t v) { loft_st_set_u32(s, p, LOFT_FL_LEFT, v); }

static void loft_fl_set_right(loft_store_t *s, uint32_t p, uint32_t v) {
    uint32_t red = loft_st_u32(s, p, LOFT_FL_RIGHT) & LOFT_FL_RED_BIT;
    loft_st_set_u32(s, p, LOFT_FL_RIGHT, v | red);
}

static void loft_fl_set_red(loft_store_t *s, uint32_t p, int v) {
    uint32_t right = loft_st_u32(s, p, LOFT_FL_RIGHT) & LOFT_FL_RIGHT_MASK;
    loft_st_set_u32(s, p, LOFT_FL_RIGHT, right | (v ? LOFT_FL_RED_BIT : 0));
}

/* (size, position) order: -1, 0, 1. */
static int loft_fl_cmp(const loft_store_t *s, uint32_t a, uint32_t b) {
    int32_t sa = loft_fl_size(s, a), sb = loft_fl_size(s, b);
    if (sa != sb) return sa < sb ? -1 : 1;
    return a < b ? -1 : a > b ? 1 : 0;
}

static uint32_t loft_fl_rotate_left(loft_store_t *s, uint32_t h) {
    uint32_t x = loft_fl_right(s, h);
    uint32_t x_left = loft_fl_left(s, x);
    loft_fl_set_right(s, h, x_left);
    loft_fl_set_left(s, x, h);
    int h_red = loft_fl_red(s, h);
    loft_fl_set_red(s, x, h_red);
    loft_fl_set_red(s, h, 1);
    return x;
}

static uint32_t loft_fl_rotate_right(loft_store_t *s, uint32_t h) {
    uint32_t x = loft_fl_left(s, h);
    uint32_t x_right = loft_fl_right(s, x);
    loft_fl_set_left(s, h, x_right);
    loft_fl_set_right(s, x, h);
    int h_red = loft_fl_red(s, h);
    loft_fl_set_red(s, x, h_red);
    loft_fl_set_red(s, h, 1);
    return x;
}

static void loft_fl_flip_colors(loft_store_t *s, uint32_t h) {
    int h_red = loft_fl_red(s, h);
    loft_fl_set_red(s, h, !h_red);
    uint32_t l = loft_fl_left(s, h);
    if (l) loft_fl_set_red(s, l, !loft_fl_red(s, l));
    uint32_t r = loft_fl_right(s, h);
    if (r) loft_fl_set_red(s, r, !loft_fl_red(s, r));
}

static uint32_t loft_fl_balance(loft_store_t *s, uint32_t h) {
    uint32_t r = loft_fl_right(s, h);
    if (r && loft_fl_red(s, r)) h = loft_fl_rotate_left(s, h);
    uint32_t l = loft_fl_left(s, h);
    uint32_t ll = l ? loft_fl_left(s, l) : 0;
    if (l && loft_fl_red(s, l) && ll && loft_fl_red(s, ll)) h = loft_fl_rotate_right(s, h);
    uint32_t l2 = loft_fl_left(s, h);
    uint32_t r2 = loft_fl_right(s, h);
    if (l2 && loft_fl_red(s, l2) && r2 && loft_fl_red(s, r2)) loft_fl_flip_colors(s, h);
    return h;
}

static uint32_t loft_fl_insert_node(loft_store_t *s, uint32_t h, uint32_t rec) {
    if (h == 0) {
        loft_fl_set_left(s, rec, 0);
        loft_st_set_u32(s, rec, LOFT_FL_RIGHT, LOFT_FL_RED_BIT);
        return rec;
    }
    if (loft_fl_cmp(s, rec, h) < 0) {
        uint32_t l = loft_fl_left(s, h);
        uint32_t nl = loft_fl_insert_node(s, l, rec);
        loft_fl_set_left(s, h, nl);
    } else {
        uint32_t r = loft_fl_right(s, h);
        uint32_t nr = loft_fl_insert_node(s, r, rec);
        loft_fl_set_right(s, h, nr);
    }
    return loft_fl_balance(s, h);
}

static void loft_fl_insert(loft_store_t *s, uint32_t rec) {
    if (loft_fl_size(s, rec) < LOFT_MIN_FREE_TREE) return;
    if (rec + (uint32_t) loft_fl_size(s, rec) == s->size) {
        s->wild = rec;
        return;
    }
    s->free_root = loft_fl_insert_node(s, s->free_root, rec);
    loft_fl_set_red(s, s->free_root, 0);
}

static uint32_t loft_fl_min_node(const loft_store_t *s, uint32_t h) {
    if (h == 0) return 0;
    for (;;) {
        uint32_t l = loft_fl_left(s, h);
        if (l == 0) return h;
        h = l;
    }
}

static uint32_t loft_fl_move_red_left(loft_store_t *s, uint32_t h) {
    loft_fl_flip_colors(s, h);
    uint32_t r = loft_fl_right(s, h);
    uint32_t rl = r ? loft_fl_left(s, r) : 0;
    if (rl && loft_fl_red(s, rl)) {
        uint32_t nr = loft_fl_rotate_right(s, r);
        loft_fl_set_right(s, h, nr);
        h = loft_fl_rotate_left(s, h);
        loft_fl_flip_colors(s, h);
    }
    return h;
}

static uint32_t loft_fl_move_red_right(loft_store_t *s, uint32_t h) {
    loft_fl_flip_colors(s, h);
    uint32_t l = loft_fl_left(s, h);
    uint32_t ll = l ? loft_fl_left(s, l) : 0;
    if (ll && loft_fl_red(s, ll)) {
        h = loft_fl_rotate_right(s, h);
        loft_fl_flip_colors(s, h);
    }
    return h;
}

static uint32_t loft_fl_delete_min_node(loft_store_t *s, uint32_t h) {
    if (loft_fl_left(s, h) == 0) return 0;
    uint32_t l = loft_fl_left(s, h);
    uint32_t ll = loft_fl_left(s, l);
    uint32_t cur = h;
    if (!loft_fl_red(s, l) && (ll == 0 || !loft_fl_red(s, ll))) cur = loft_fl_move_red_left(s, cur);
    uint32_t left = loft_fl_left(s, cur);
    uint32_t nl = loft_fl_delete_min_node(s, left);
    loft_fl_set_left(s, cur, nl);
    return loft_fl_balance(s, cur);
}

static uint32_t loft_fl_delete_node(loft_store_t *s, uint32_t h, uint32_t target) {
    if (h == 0) return 0;
    int32_t tsz = loft_fl_size(s, target), hsz = loft_fl_size(s, h);
    if (tsz < hsz || (tsz == hsz && target < h)) {
        uint32_t l = loft_fl_left(s, h);
        uint32_t ll = l ? loft_fl_left(s, l) : 0;
        if (l && !loft_fl_red(s, l) && (ll == 0 || !loft_fl_red(s, ll))) h = loft_fl_move_red_left(s, h);
        uint32_t left = loft_fl_left(s, h);
        uint32_t nl = loft_fl_delete_node(s, left, target);
        loft_fl_set_left(s, h, nl);
    } else {
        if (loft_fl_left(s, h) && loft_fl_red(s, loft_fl_left(s, h))) h = loft_fl_rotate_right(s, h);
        if (h == target && loft_fl_right(s, h) == 0) return 0;
        uint32_t r = loft_fl_right(s, h);
        uint32_t rl = r ? loft_fl_left(s, r) : 0;
        if (r && !loft_fl_red(s, r) && (rl == 0 || !loft_fl_red(s, rl))) h = loft_fl_move_red_right(s, h);
        if (h == target) {
            uint32_t right = loft_fl_right(s, h);
            if (right == 0) return loft_fl_left(s, h);
            uint32_t succ = loft_fl_min_node(s, right);
            uint32_t h_left = loft_fl_left(s, h);
            int h_red = loft_fl_red(s, h);
            uint32_t nr = loft_fl_delete_min_node(s, right);
            loft_fl_set_left(s, succ, h_left);
            loft_fl_set_right(s, succ, nr);
            loft_fl_set_red(s, succ, h_red);
            h = succ;
        } else {
            uint32_t right = loft_fl_right(s, h);
            uint32_t nr = loft_fl_delete_node(s, right, target);
            loft_fl_set_right(s, h, nr);
        }
    }
    return loft_fl_balance(s, h);
}

static int loft_fl_tree_contains(const loft_store_t *s, uint32_t target) {
    if (target == 0) return 0;
    if (target == s->wild) return 1;
    uint32_t h = s->free_root;
    while (h) {
        int c = loft_fl_cmp(s, target, h);
        if (c == 0) return 1;
        h = c < 0 ? loft_fl_left(s, h) : loft_fl_right(s, h);
    }
    return 0;
}

/* ---- the parked lists (`@FR-H-Park`) ---- */

static void loft_park_block(loft_store_t *s, uint32_t rec, int32_t words) {
    loft_set_free_header(s, rec, words);
    uint32_t head = s->parked_heads[words];
    loft_st_set_u32(s, rec, LOFT_PARK_PREV, 0);
    loft_st_set_u32(s, rec, LOFT_PARK_NEXT, head);
    if (head) loft_st_set_u32(s, head, LOFT_PARK_PREV, rec);
    s->parked_heads[words] = rec;
    loft_bits_insert(&s->parked_set, rec);
    s->parked_count++;
}

static void loft_unpark(loft_store_t *s, uint32_t rec) {
    uint32_t prev = loft_st_u32(s, rec, LOFT_PARK_PREV);
    uint32_t next = loft_st_u32(s, rec, LOFT_PARK_NEXT);
    if (prev == 0) s->parked_heads[loft_fl_size(s, rec)] = next;
    else loft_st_set_u32(s, prev, LOFT_PARK_NEXT, next);
    if (next) loft_st_set_u32(s, next, LOFT_PARK_PREV, prev);
    loft_bits_remove(&s->parked_set, rec);
    s->parked_count--;
}

static uint32_t loft_pop_parked(loft_store_t *s, uint32_t size) {
    if (size > LOFT_PARK_MAX_WORDS) return 0;
    uint32_t rec = s->parked_heads[size];
    if (rec == 0) return 0;
    uint32_t next = loft_st_u32(s, rec, LOFT_PARK_NEXT);
    s->parked_heads[size] = next;
    if (next) loft_st_set_u32(s, next, LOFT_PARK_PREV, 0);
    loft_bits_remove(&s->parked_set, rec);
    s->parked_count--;
    loft_st_set_i32(s, rec, 0, (int32_t) size);
    loft_bits_insert(&s->claims, rec);
    return rec;
}

static void loft_parked_clear(loft_store_t *s) {
    memset(s->parked_heads, 0, sizeof s->parked_heads);
    loft_bits_clear(&s->parked_set);
    s->parked_count = 0;
}

/* ---- the free-tree removal and the frees ---- */

static void loft_fl_remove(loft_store_t *s, uint32_t rec) {
    if (s->parked_count && loft_bits_contains(&s->parked_set, rec)) {
        loft_unpark(s, rec);
        return;
    }
    if (s->lazy && loft_bits_remove(&s->lazy_free, rec)) {
        uint32_t w = (uint32_t) loft_fl_size(s, rec);
        s->dead_words = s->dead_words > w ? s->dead_words - w : 0;
        return;
    }
    if (rec == s->wild && rec != 0) {
        s->wild = 0;
        return;
    }
    if (s->free_root == 0 || loft_fl_size(s, rec) < LOFT_MIN_FREE_TREE) return;
    s->free_root = loft_fl_delete_node(s, s->free_root, rec);
    if (s->free_root) loft_fl_set_red(s, s->free_root, 0);
}

static void loft_forget_free(loft_store_t *s, uint32_t pos) {
    if (loft_bits_remove(&s->lazy_free, pos)) {
        int32_t w = -loft_st_i32(s, pos, 0);
        uint32_t uw = w > 0 ? (uint32_t) w : 0;
        s->dead_words = s->dead_words > uw ? s->dead_words - uw : 0;
    } else {
        loft_fl_remove(s, pos);
    }
}

static int loft_claimed_block_ends_at(const loft_store_t *s, uint32_t end) {
    int found;
    uint32_t p = loft_bits_prev_start(&s->claims, end, &found);
    if (!found) return 0;
    int32_t words = loft_st_i32(s, p, 0);
    return words > 0 && p + (uint32_t) words == end;
}

static int loft_parked_predecessor(const loft_store_t *s, uint32_t rec) {
    if (s->parked_count == 0 || loft_claimed_block_ends_at(s, rec)) return 0;
    int32_t f = loft_st_i32(s, rec - 1, 4);
    if (f == INT32_MIN) return 0;
    uint32_t words = (uint32_t) -f;
    if (words > rec) return 0;
    uint32_t prev = rec - words;
    return prev >= LOFT_ST_PRIMARY && loft_bits_contains(&s->parked_set, prev) && loft_st_i32(s, prev, 0) == f;
}

/* A confirmed free predecessor (`free_predecessor`): *prev and *words, or 0. */
static int loft_free_predecessor(const loft_store_t *s, uint32_t rec, uint32_t *prev, int32_t *words) {
    if (rec <= LOFT_ST_PRIMARY || loft_claimed_block_ends_at(s, rec)) return 0;
    int32_t f = loft_st_i32(s, rec - 1, 4);
    if (f >= 0 || f == INT32_MIN) return 0;
    int32_t w = -f;
    if ((uint32_t) w > rec) return 0;
    uint32_t p = rec - (uint32_t) w;
    if (p < LOFT_ST_PRIMARY || w < LOFT_MIN_FREE_TREE) return 0;
    if (loft_st_i32(s, p, 0) == f && loft_fl_tree_contains(s, p)) {
        *prev = p;
        *words = w;
        return 1;
    }
    return 0;
}

static void loft_free_exact(loft_store_t *s, uint32_t rec, int32_t claim) {
    while (rec + (uint32_t) claim < s->size) {
        uint32_t next_pos = rec + (uint32_t) claim;
        int32_t next_header = loft_st_i32(s, next_pos, 0);
        if (next_header >= 0 || loft_bits_contains(&s->parked_set, next_pos)) break;
        loft_fl_remove(s, next_pos);
        claim -= next_header;
    }
    uint32_t prev;
    int32_t words;
    if (loft_free_predecessor(s, rec, &prev, &words)) {
        loft_fl_remove(s, prev);
        claim += words;
        rec = prev;
    } else if (rec > LOFT_ST_PRIMARY && !loft_claimed_block_ends_at(s, rec)
               && loft_st_i32(s, rec - 1, 4) < 0 && !loft_parked_predecessor(s, rec)) {
        s->needs_coalesce = 1;
    }
    loft_set_free_header(s, rec, claim);
    loft_fl_insert(s, rec);
}

static void loft_flush_parked(loft_store_t *s) {
    for (int32_t words = LOFT_MIN_FREE_TREE; words <= LOFT_PARK_MAX_WORDS; words++) {
        for (;;) {
            uint32_t rec = s->parked_heads[words];
            if (rec == 0) break;
            loft_unpark(s, rec);
            loft_free_exact(s, rec, words);
        }
    }
}

static void loft_fl_rebuild(loft_store_t *s) {
    s->free_root = 0;
    s->wild = 0;
    loft_bits_clear(&s->lazy_free);
    s->dead_words = 0;
    loft_parked_clear(s);
    uint32_t pos = LOFT_ST_PRIMARY;
    while (pos < s->size) {
        int32_t header = loft_st_i32(s, pos, 0);
        int32_t bs = header < 0 ? -header : header;
        if (bs == 0) break;
        if (header < 0) {
            loft_st_set_i32(s, pos + (uint32_t) bs - 1, 4, header);
            if (-header >= LOFT_MIN_FREE_TREE) loft_fl_insert(s, pos);
        }
        pos += (uint32_t) bs;
    }
}

static void loft_coalesce_free(loft_store_t *s) {
    uint32_t pos = LOFT_ST_PRIMARY;
    while (pos < s->size) {
        int32_t header = loft_st_i32(s, pos, 0);
        int32_t bs = header < 0 ? -header : header;
        if (header < 0) {
            uint32_t next = pos + (uint32_t) bs;
            while (next < s->size) {
                int32_t nh = loft_st_i32(s, next, 0);
                if (nh >= 0) break;
                bs += -nh;
                next = pos + (uint32_t) bs;
            }
            loft_set_free_header(s, pos, bs);
        }
        pos += (uint32_t) bs;
    }
    loft_fl_rebuild(s);
    s->needs_coalesce = 0;
}

static void loft_lazy_delete(loft_store_t *s, uint32_t rec) {
    uint32_t start = rec;
    int32_t words = loft_st_i32(s, rec, 0);
    loft_bits_remove(&s->claims, rec);
    for (;;) {
        uint32_t next = start + (uint32_t) words;
        if (next >= s->size) break;
        int32_t nh = loft_st_i32(s, next, 0);
        if (nh >= 0 || loft_bits_contains(&s->parked_set, next)) break;
        loft_forget_free(s, next);
        words -= nh;
    }
    if (start > LOFT_ST_PRIMARY && !loft_claimed_block_ends_at(s, start)) {
        int32_t f = loft_st_i32(s, start - 1, 4);
        if (f < 0 && f != INT32_MIN) {
            int32_t pw = -f;
            if ((uint32_t) pw <= start) {
                uint32_t prev = start - (uint32_t) pw;
                if (prev >= LOFT_ST_PRIMARY && loft_st_i32(s, prev, 0) == f
                    && (loft_bits_contains(&s->lazy_free, prev) || loft_fl_tree_contains(s, prev))) {
                    loft_forget_free(s, prev);
                    start = prev;
                    words += pw;
                }
            }
        }
    }
    loft_set_free_header(s, start, words);
    if (start + (uint32_t) words == s->size) {
        loft_fl_insert(s, start);
    } else {
        loft_bits_insert(&s->lazy_free, start);
        uint32_t sum = s->dead_words + (uint32_t) words;
        s->dead_words = sum < s->dead_words ? UINT32_MAX : sum;
        s->needs_coalesce = 1;
    }
}

/* ---- growth ---- */

static void loft_resize_store(loft_store_t *s, uint32_t to_size) {
    if (to_size <= s->size) return;
    if (to_size > (uint32_t) INT32_MAX) { fputs("loft: store offset overflow\n", stderr); abort(); }
    uint64_t inc64 = (uint64_t) s->size * 7;
    uint32_t inc = inc64 > UINT32_MAX ? UINT32_MAX / 3 : (uint32_t) (inc64 / 3);
    uint32_t size = to_size > inc ? to_size : inc;
    loft_heap_moved(s, s->size, size);
#ifdef LOFT_POISON
    /* `LOFT_POISON`: the growth always MOVES the bytes and scribbles the block it leaves, which
     * is kept, never handed back — an address taken into the store before the growth then reads
     * the scribble instead of bytes a `realloc` would have left readable and equal. */
    uint8_t *b = (uint8_t *) malloc((size_t) size * 8);
    if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(b, s->buf, (size_t) s->size * 8);
    memset(s->buf, 0xA5, (size_t) s->size * 8);
#else
    uint8_t *b = (uint8_t *) realloc(s->buf, (size_t) size * 8);
    if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
#endif
    if (s->defined) memset(b + (size_t) s->size * 8, 0, (size_t) (size - s->size) * 8);
    s->buf = b;
    s->size = size;
}

/* ---- claims ---- */

/* `Store::poison_range` (`LOFT_POISON_CLAIM`): `len` bytes at byte `pos` of `rec` read
 * 0xDEADBEEF, whole words only — a record id, a handle or a length read out of a claim nothing
 * wrote is then loud.  Compiled only into a poison build. */
#ifdef LOFT_POISON_CLAIM
static void loft_st_poison_range(loft_store_t *s, uint32_t rec, uint32_t pos, uint32_t len) {
    const uint32_t v = 0xDEADBEEFu;
    for (uint32_t i = 0; i < len / 4; i++) memcpy(s->buf + (size_t) rec * 8 + pos + 4 * i, &v, 4);
}
#endif

/* `@C154`: bytes a caller is about to write, in a record of `s`, take no fill.  A DEFINED store
 * (its bytes leave the process) is written whole, so it zeroes them; a poison build fills them
 * with poison, so a reader of a byte nobody wrote is loud (c_sweep's `claim-unwritten`). */
static void loft_st_fresh(loft_store_t *s, uint32_t rec, uint32_t pos, uint32_t len) {
#ifdef LOFT_POISON_CLAIM
    if (!s->defined) { loft_st_poison_range(s, rec, pos, len); return; }
#endif
    if (s->defined) memset(s->buf + (size_t) rec * 8 + pos, 0, len);
}

static uint32_t loft_claim_block(loft_store_t *s, uint32_t pos, uint32_t size) {
    int32_t req = (int32_t) size;
    int32_t bs = -loft_st_i32(s, pos, 0);
    if ((int64_t) bs > (int64_t) req * 4 / 3) {
        loft_st_set_i32(s, pos, 0, req);
        uint32_t nf = pos + size;
        loft_set_free_header(s, nf, bs - req);
        loft_fl_insert(s, nf);
    } else {
        loft_st_set_i32(s, pos, 0, bs);
    }
    loft_bits_insert(&s->claims, pos);
    uint32_t end = pos + (uint32_t) loft_st_i32(s, pos, 0);
    if (end > s->claimed_end) s->claimed_end = end;
    return pos;
}

static uint32_t loft_carve_in_place(loft_store_t *s, uint32_t node, uint32_t parent, uint32_t size, int32_t rest) {
    uint32_t left = loft_fl_left(s, node);
    uint32_t right = loft_st_u32(s, node, LOFT_FL_RIGHT);
    loft_st_set_i32(s, node, 0, (int32_t) size);
    uint32_t rem = node + size;
    loft_set_free_header(s, rem, rest);
    loft_st_set_u32(s, rem, LOFT_FL_LEFT, left);
    loft_st_set_u32(s, rem, LOFT_FL_RIGHT, right);
    if (parent == 0) s->free_root = rem;
    else if (loft_fl_left(s, parent) == node) loft_fl_set_left(s, parent, rem);
    else loft_fl_set_right(s, parent, rem);
    loft_bits_insert(&s->claims, node);
    if (rem > s->claimed_end) s->claimed_end = rem;
    return node;
}

/* `claim_best_fit`: the position, or 0 when no tracked block fits. */
static uint32_t loft_claim_best_fit(loft_store_t *s, uint32_t size) {
    int32_t req = (int32_t) size;
    uint32_t h = s->free_root, parent = 0, found = 0, found_parent = 0;
    while (h) {
        uint32_t next;
        if (loft_fl_size(s, h) < req) {
            next = loft_fl_right(s, h);
        } else {
            found = h;
            found_parent = parent;
            next = loft_fl_left(s, h);
        }
        parent = h;
        h = next;
    }
    uint32_t wild = s->wild;
    if (wild && loft_fl_size(s, wild) >= req && (found == 0 || loft_fl_size(s, wild) < loft_fl_size(s, found))) {
        s->wild = 0;
        return loft_claim_block(s, wild, size);
    }
    if (found == 0) return 0;
    int32_t rest = loft_fl_size(s, found) - req;
    if (rest >= req && rest >= LOFT_MIN_FREE_TREE) return loft_carve_in_place(s, found, found_parent, size, rest);
    s->free_root = loft_fl_delete_node(s, s->free_root, found);
    if (s->free_root) loft_fl_set_red(s, s->free_root, 0);
    return loft_claim_block(s, found, size);
}

static uint32_t loft_bump_wild(loft_store_t *s, uint32_t size) {
    uint32_t pos = s->wild;
    if (pos == 0 || s->free_root != 0) return 0;
    int32_t req = (int32_t) size;
    int32_t block = loft_fl_size(s, pos);
    if ((int64_t) block <= (int64_t) req * 4 / 3 || block - req < LOFT_MIN_FREE_TREE) return 0;
    loft_st_set_i32(s, pos, 0, req);
    uint32_t rest = pos + size;
    loft_set_free_header(s, rest, block - req);
    s->wild = rest;
    loft_bits_insert(&s->claims, pos);
    if (rest > s->claimed_end) s->claimed_end = rest;
    return pos;
}

/* `indexed_scan`: 1 with (*pos, *last, *claim) when it answers, 0 when the walk must. */
static int loft_indexed_scan(const loft_store_t *s, int32_t req, uint32_t *pos, uint32_t *last, int32_t *claim) {
    if (req < LOFT_MIN_FREE_TREE || s->size <= LOFT_ST_PRIMARY) return 0;
    int found;
    uint32_t p = loft_bits_next_at_or_after(&s->lazy_free, LOFT_ST_PRIMARY, &found);
    while (found) {
        if (p >= s->size) break;
        int32_t h = loft_st_i32(s, p, 0);
        if (h >= 0) return 0;
        if (-h >= req) { *pos = p; *last = p; *claim = h; return 1; }
        p = loft_bits_next_at_or_after(&s->lazy_free, p + 1, &found);
    }
    if (loft_claimed_block_ends_at(s, s->size)) { *pos = s->size; *last = s->size; *claim = 1; return 1; }
    int32_t f = loft_st_i32(s, s->size - 1, 4);
    if (f >= 0) { *pos = s->size; *last = s->size; *claim = 1; return 1; }
    if (f == INT32_MIN || (uint32_t) -f > s->size) return 0;
    uint32_t prev = s->size - (uint32_t) -f;
    if (prev >= LOFT_ST_PRIMARY && loft_st_i32(s, prev, 0) == f
        && (loft_bits_contains(&s->lazy_free, prev) || loft_fl_tree_contains(s, prev))) {
        *pos = s->size; *last = prev; *claim = f;
        return 1;
    }
    return 0;
}

static void loft_walk_scan(const loft_store_t *s, int32_t req, uint32_t *pos_out, uint32_t *last_out, int32_t *claim_out) {
    uint32_t pos = LOFT_ST_PRIMARY, last = pos;
    int32_t claim = loft_st_i32(s, pos, 0);
    while (pos < s->size && (claim >= 0 || -claim < req)) {
        last = pos;
        pos += (uint32_t) (claim < 0 ? -claim : claim);
        if (pos >= s->size) break;
        claim = loft_st_i32(s, pos, 0);
        if (claim == 0) {
            fprintf(stderr, "loft: block chain is malformed at word %u of %u (a block owning no words); allocating past it and leaking the remainder\n", pos, s->size);
            claim = loft_st_i32(s, last, 0);
            pos = s->size;
            break;
        }
    }
    *pos_out = pos;
    *last_out = last;
    *claim_out = claim;
}

static uint32_t loft_claim_grow(loft_store_t *s, uint32_t size, uint32_t last, int32_t last_claim) {
    uint32_t cur = s->size;
    uint32_t new_size = last_claim < 0 ? (uint32_t) ((int32_t) s->size + (int32_t) size + last_claim) : s->size + size;
    loft_resize_store(s, new_size);
    int32_t increase = (int32_t) (s->size - cur);
    if (last_claim < 0) {
        loft_set_free_header(s, last, -(last_claim - increase));
        return last;
    }
    loft_set_free_header(s, cur, increase);
    return cur;
}

static uint32_t loft_claim_scan(loft_store_t *s, uint32_t size) {
    int32_t req = (int32_t) size;
    uint32_t pos, last;
    int32_t claim;
    if (!loft_indexed_scan(s, req, &pos, &last, &claim)) loft_walk_scan(s, req, &pos, &last, &claim);
    if (pos >= s->size) {
        if (claim < 0) loft_fl_remove(s, last);
        pos = loft_claim_grow(s, size, last, claim);
    }
    if (pos == s->wild) {
        s->wild = 0;
    } else if (pos < s->size && loft_bits_remove(&s->lazy_free, pos)) {
        int32_t w = -loft_st_i32(s, pos, 0);
        uint32_t uw = w > 0 ? (uint32_t) w : 0;
        s->dead_words = s->dead_words > uw ? s->dead_words - uw : 0;
    }
    return loft_claim_block(s, pos, size);
}

static int loft_sweep_due(const loft_store_t *s) {
    return s->dead_words >= LOFT_LAZY_FLOOR_WORDS && (uint64_t) s->dead_words * 5 >= (uint64_t) s->claimed_end;
}

static uint32_t loft_st_claim_inner(loft_store_t *s, uint32_t size) {
    if (size < 1) { fputs("loft: Incomplete record\n", stderr); abort(); }
    if (s->lazy && loft_sweep_due(s)) {
        loft_coalesce_free(s);
        s->lazy = 0;
        s->dead_words = 0;
        loft_bits_clear(&s->lazy_free);
    }
    if (s->parked_count) {
        uint32_t pos = loft_pop_parked(s, size);
        if (pos) return pos;
    }
    {
        uint32_t pos = loft_bump_wild(s, size);
        if (pos) return pos;
    }
    {
        uint32_t r = loft_claim_best_fit(s, size);
        if (r) return r;
    }
    if (s->parked_count) {
        loft_flush_parked(s);
        uint32_t r = loft_claim_best_fit(s, size);
        if (r) return r;
    }
    if (s->needs_coalesce && !s->lazy) {
        loft_coalesce_free(s);
        uint32_t r = loft_claim_best_fit(s, size);
        if (r) return r;
    }
    return loft_claim_scan(s, size);
}

/* `@FR-H-WriteLocked`: a write that reached a locked store (loft_rt.h).  The allocator compiled
 * on its own (without `loft_cc.h`, as its c_emit harness compiles it) has no locks. */
#ifdef LOFT_CC_H
static void loft_locked_write(void);
#define LOFT_WRITE_CHECK(s) do { if (LOFT_UNLIKELY((s)->read_only)) loft_locked_write(); } while (0)
#else
#define LOFT_WRITE_CHECK(s) ((void) (s))
#endif

/* `Store::claim`: a record of `size` words, its header written; the payload holds whatever the
 * block held (`@FR-H-Claim`). */
static uint32_t loft_st_claim(loft_store_t *s, uint32_t size) {
    LOFT_WRITE_CHECK(s);
    uint32_t pos = loft_st_claim_inner(s, size);
    LOFT_TRACE("C %llu %d %u\n", (unsigned long long) s->serial, loft_st_i32(s, pos, 0), pos);
#ifdef LOFT_POISON_CLAIM
    loft_st_poison_range(s, pos, 4, (uint32_t) loft_st_i32(s, pos, 0) * 8 - 4);
#endif
    return pos;
}

/* ---- deletes ---- */

static void loft_st_delete(loft_store_t *s, uint32_t rec);

static void loft_entomb(loft_store_t *s, uint32_t rec) {
    int32_t words = loft_st_i32(s, rec, 0);
    loft_bits_remove(&s->claims, rec);
    loft_bits_insert(&s->removed, rec);
    loft_st_set_i32(s, rec, 4, 0);
    if (words > 1) {
        loft_st_set_i32(s, rec, 0, 1);
        loft_st_set_i32(s, rec + 1, 0, words - 1);
        loft_bits_insert(&s->claims, rec + 1);
        loft_st_delete(s, rec + 1);
    }
}

/* `Store::delete`. */
static void loft_st_delete(loft_store_t *s, uint32_t rec) {
    /* A locked store changes in no way, deletes included (`Store::delete`).  The allocator
     * compiled on its own (no `loft_cc.h`) has no locks. */
#ifdef LOFT_CC_H
    if (LOFT_UNLIKELY(s->read_only)) { loft_locked_write(); return; }
#endif
    LOFT_TRACE("D %llu %u\n", (unsigned long long) s->serial, rec);
    for (uint32_t i = 0; i < s->nplaced;) {
        if (s->placed[i] == rec) s->placed[i] = s->placed[--s->nplaced];
        else i++;
    }
    if (loft_bits_remove(&s->referenced, rec)) {
        loft_entomb(s, rec);
        return;
    }
    int32_t words = loft_st_i32(s, rec, 0);
    if (!s->no_park && words >= LOFT_MIN_FREE_TREE && words <= LOFT_PARK_MAX_WORDS && rec + (uint32_t) words != s->size) {
        loft_bits_remove(&s->claims, rec);
        loft_park_block(s, rec, words);
        return;
    }
    if (s->lazy && words <= LOFT_LAZY_MAX_WORDS) {
        loft_lazy_delete(s, rec);
        return;
    }
    loft_bits_remove(&s->claims, rec);
    loft_free_exact(s, rec, words);
}

/* ---- resize ---- */

static uint32_t loft_st_resize_inner(loft_store_t *s, uint32_t rec, uint32_t size) {
    int32_t req = (int32_t) size;
    int32_t claim = loft_st_i32(s, rec, 0);
    if (claim >= req) return rec;
    uint32_t next = rec + (uint32_t) claim;
    if (next < s->size) {
        int32_t next_size = loft_st_i32(s, next, 0);
        if (next_size < 0 && claim - next_size > req) {
            loft_fl_remove(s, next);
            int32_t act = req * 7 / 4;
            if (claim - next_size > act) {
                uint32_t new_next = rec + (uint32_t) act;
                uint32_t new_free = (uint32_t) -next_size + next - new_next;
                loft_st_set_i32(s, rec, 0, act);
                loft_set_free_header(s, new_next, (int32_t) new_free);
                loft_fl_insert(s, new_next);
            } else {
                loft_st_set_i32(s, rec, 0, claim - next_size);
            }
#ifdef LOFT_POISON_CLAIM
            /* The absorbed region promises nothing, as a claim's payload does not. */
            loft_st_poison_range(s, rec, (uint32_t) claim * 8, (uint32_t) (loft_st_i32(s, rec, 0) - claim) * 8);
#endif
            LOFT_TRACE("R %llu %u %d %u\n", (unsigned long long) s->serial, rec, loft_st_i32(s, rec, 0), rec);
            return rec;
        }
    }
    uint32_t nw = loft_st_claim(s, size);
    memcpy(s->buf + (size_t) nw * 8 + 4, s->buf + (size_t) rec * 8 + 4, (size_t) claim * 8 - 4);
    loft_st_delete(s, rec);
    LOFT_TRACE("R %llu %u %d %u\n", (unsigned long long) s->serial, rec, loft_st_i32(s, nw, 0), nw);
    return nw;
}

/* `Store::resize`: `rec` grown to `size` words, in place when the next block allows, else
 * moved (the new position answered). */
static uint32_t loft_st_resize(loft_store_t *s, uint32_t rec, uint32_t size) {
    LOFT_WRITE_CHECK(s);
    uint32_t nw = loft_st_resize_inner(s, rec, size);
    return nw;
}

/* ---- the store ---- */

/* `Store::init`: one free block spanning the store, which is its wilderness. */
static void loft_st_init(loft_store_t *s) {
    uint32_t sig = LOFT_ST_SIGNATURE, one = 1;
    s->read_only = 0;
    memcpy(s->buf, &sig, 4);
    memcpy(s->buf + 4, &one, 4);
    loft_set_free_header(s, LOFT_ST_PRIMARY, (int32_t) s->size - 1);
    s->free_root = 0;
    s->wild = 0;
    loft_bits_clear(&s->claims);
    loft_bits_clear(&s->referenced);
    loft_bits_clear(&s->removed);
    s->lazy = 1;
    s->dead_words = 0;
    loft_bits_clear(&s->lazy_free);
    loft_parked_clear(s);
    s->nplaced = 0;
    s->needs_coalesce = 0;
    s->claimed_end = 0;
    loft_fl_insert(s, LOFT_ST_PRIMARY);
}

/* `Store::note_placed_foreign` (`@FR-H-RootExtent`). */
static void loft_st_note_placed(loft_store_t *s, uint32_t rec) {
    if (s->nplaced == s->placed_cap) {
        s->placed_cap = s->placed_cap ? s->placed_cap * 2 : 4;
        s->placed = (uint32_t *) realloc(s->placed, (size_t) s->placed_cap * 4);
        if (!s->placed) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    s->placed[s->nplaced++] = rec;
}

#ifdef LOFT_FOREIGN
static int loft_st_is_foreign(const loft_store_t *s) { return s->foreign.span.owner != 0; }

/* `Store::foreign_addr`: field `byte` of the foreign record — the header below 8, the foreign
 * bytes past it. */
static uint8_t *loft_foreign_at(loft_store_t *s, uint32_t byte) {
    if (byte < 8) return s->foreign.header + byte;
    return (uint8_t *) (uintptr_t) (s->foreign.span.base + (byte - 8));
}

/* `Store::foreign_span`: elements [lo, hi) of the foreign bytes, clamped, the owner shared; a
 * span with no owner when the store serves none. */
static loft_fspan loft_st_foreign_span(loft_store_t *s, uint32_t lo, uint32_t hi) {
    loft_fspan sp;
    memset(&sp, 0, sizeof sp);
    if (!loft_st_is_foreign(s)) return sp;
    const loft_fspan *f = &s->foreign.span;
    if (lo > f->len) lo = f->len;
    if (hi < lo) hi = lo;
    if (hi > f->len) hi = f->len;
    sp.base = f->base + (size_t) lo * f->elem;
    sp.len = hi - lo;
    sp.elem = f->elem;
    sp.owner = f->owner;
    sp.owner->refs++;
    return sp;
}

/* `Store::make_foreign`: the slot at (rec, pos) names `LOFT_FOREIGN_REC`, the header presents
 * the span's elements, and the span (its owner reference taken) is the store's.  A record the
 * slot named before goes back to the free tree. */
static void loft_st_make_foreign(loft_store_t *s, uint32_t rec, uint32_t pos, loft_fspan span) {
    if (loft_st_is_foreign(s) || s->read_only) { fputs("loft: make_foreign on a store that is already foreign or locked\n", stderr); abort(); }
    uint32_t old = loft_st_u32(s, rec, pos);
    if (old != 0) loft_st_delete(s, old);
    loft_st_set_u32(s, rec, pos, LOFT_FOREIGN_REC);
    uint64_t bytes = (uint64_t) span.len * span.elem;
    uint64_t w = (bytes + 7) / 8 + 1;
    uint32_t words = w > UINT32_MAX ? UINT32_MAX : (uint32_t) w;
    memcpy(s->foreign.header, &words, 4);
    memcpy(s->foreign.header + 4, &span.len, 4);
    s->foreign.span = span;
    s->foreign.slot_rec = rec;
    s->foreign.slot_pos = pos;
    s->read_only = 1; /* the bytes are the mapping's or the library's: locked, as `Store::make_foreign` */
}

/* `Store::release_foreign`: the store stops serving the bytes, its slot names the empty vector
 * again, and the owner goes with the last span over it. */
static void loft_st_release_foreign(loft_store_t *s) {
    if (!loft_st_is_foreign(s)) return;
    loft_fowner *o = s->foreign.span.owner;
    s->foreign.span.owner = 0;
    s->read_only = 0;
    loft_st_set_u32(s, s->foreign.slot_rec, s->foreign.slot_pos, 0);
    if (--o->refs == 0) {
        if (o->release) o->release(o->bytes, o->n);
        free(o);
    }
}
#endif

/* `Store::note_referenced` (`@FR-H-Removed`). */
static void loft_st_note_referenced(loft_store_t *s, uint32_t rec) {
    if (rec >= LOFT_ST_PRIMARY && rec < s->size) loft_bits_insert(&s->referenced, rec);
}

static int loft_st_is_removed(const loft_store_t *s, uint32_t rec) {
    return loft_bits_contains(&s->removed, rec);
}

/* `StoreUsage`: a store's space, from a walk of its block chain. */
typedef struct {
    uint32_t capacity_words, claimed_words, claimed_count, free_words, free_count;
    uint32_t largest_free_words, mergeable_free_pairs, live_end_words;
    int walk_complete;
} loft_usage;

/* `Store::usage`: a negative header is a free block of `-size` words, a positive one a claimed
 * record; a run of free blocks holding a parked one is the one block its flush leaves
 * (`@FR-H-Park`), and a removed record's header word is held space, not a record. */
static loft_usage loft_st_usage(const loft_store_t *s) {
    loft_usage u;
    memset(&u, 0, sizeof u);
    u.capacity_words = s->size;
    if (!s->live || s->size <= LOFT_ST_PRIMARY) return u;
    uint32_t pos = LOFT_ST_PRIMARY, run_words = 0;
    int prev_was_free = 0, run_parked = 0;
    while (pos < s->size) {
        int32_t claim = loft_st_i32(s, pos, 0);
        if (claim == 0) break;
        uint32_t sz = claim < 0 ? (uint32_t) -(int64_t) claim : (uint32_t) claim;
        if (claim < 0) {
            u.free_words += sz;
            int parked = s->parked_count != 0 && loft_bits_contains(&s->parked_set, pos);
            if (prev_was_free && (run_parked || parked)) {
                run_words += sz;
                run_parked = 1;
            } else {
                if (prev_was_free) u.mergeable_free_pairs++;
                u.free_count++;
                run_words = sz;
                run_parked = parked;
            }
            if (run_words > u.largest_free_words) u.largest_free_words = run_words;
            prev_was_free = 1;
        } else {
            u.claimed_words += sz;
            u.claimed_count += !loft_st_is_removed(s, pos);
            prev_was_free = 0;
        }
        pos += sz;
        if (claim > 0) u.live_end_words = pos;
    }
    u.walk_complete = pos == s->size;
    return u;
}

/* `Store::shrink_to` (a heap store): the buffer cut to `words`, never under the end of the last
 * claimed block nor the two-word floor, and the free space left above it re-tiled as one block
 * ending the store, the free tree rebuilt (`retile_tail`).  0 when it did not shrink. */
static int loft_st_shrink_to(loft_store_t *s, uint32_t words) {
    if (s->read_only) return 0;
    loft_flush_parked(s);
    loft_usage u = loft_st_usage(s);
    if (!u.walk_complete) return 0;
    uint32_t mark = u.live_end_words;
    if (words < mark) return 0;
    if (words < LOFT_ST_PRIMARY + 1) words = LOFT_ST_PRIMARY + 1;
    if (words >= s->size) return 0;
    loft_heap_moved(s, s->size, words);
    uint8_t *b = (uint8_t *) realloc(s->buf, (size_t) words * 8);
    if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
    s->buf = b;
    s->size = words;
    uint32_t tail = mark > LOFT_ST_PRIMARY ? mark : LOFT_ST_PRIMARY;
    if (tail < s->size) loft_set_free_header(s, tail, (int32_t) (s->size - tail));
    loft_fl_rebuild(s);
    return 1;
}

/* `Store::reclaim_tail`: parked blocks flushed and the free blocks merged, lazy frees off for
 * good, and the tail given back down to the live end plus an eighth of slack (`slack_target`);
 * the words it shrank by. */
static uint32_t loft_st_reclaim_tail(loft_store_t *s) {
    loft_flush_parked(s);
    if (s->needs_coalesce) loft_coalesce_free(s);
    s->lazy = 0;
    s->dead_words = 0;
    loft_bits_clear(&s->lazy_free);
    uint32_t before = s->size;
    uint32_t mark = loft_st_usage(s).live_end_words;
    uint32_t target = mark + mark / 8 < mark ? UINT32_MAX : mark + mark / 8;
    return loft_st_shrink_to(s, target) ? before - s->size : 0;
}

/* `Store::claim_at`: the record at exactly `pos`, `size` words, carved out of the free blocks
 * that cover it (an image's tombstones are placed this way). */
static uint32_t loft_st_claim_at(loft_store_t *s, uint32_t pos, uint32_t size) {
    uint32_t base = LOFT_ST_PRIMARY;
    for (;;) {
        int32_t h = loft_st_i32(s, base, 0);
        uint32_t bsz = (uint32_t) (h < 0 ? -h : h);
        if (base >= s->size || bsz == 0) { fputs("loft: claim_at past the store's end\n", stderr); abort(); }
        if (pos < base + bsz) break;
        base += bsz;
    }
    int32_t header = loft_st_i32(s, base, 0);
    if (header >= 0) { fputs("loft: claim_at on a claimed block\n", stderr); abort(); }
    loft_fl_remove(s, base);
    uint32_t bend = base + (uint32_t) -header;
    while (bend < pos + size) {
        int32_t nx = bend < s->size ? loft_st_i32(s, bend, 0) : 0;
        if (nx >= 0) { fputs("loft: claim_at runs into a claimed block\n", stderr); abort(); }
        loft_fl_remove(s, bend);
        bend += (uint32_t) -nx;
    }
    if (base < pos) {
        loft_set_free_header(s, base, (int32_t) (pos - base));
        loft_fl_insert(s, base);
    }
    loft_st_set_i32(s, pos, 0, (int32_t) size);
    loft_bits_insert(&s->claims, pos);
    uint32_t tail = pos + size;
    if (tail < bend) {
        loft_set_free_header(s, tail, (int32_t) (bend - tail));
        loft_fl_insert(s, tail);
    }
    return pos;
}

/* `Store::kept_positions`: the referenced and removed positions, ascending, in a fresh array
 * (*n of them; 0 and no array when there are none). */
static uint32_t *loft_st_kept_positions(const loft_store_t *s, uint32_t *n) {
    *n = s->referenced.live + s->removed.live;
    if (*n == 0) return 0;
    uint32_t *out = (uint32_t *) malloc(*n * sizeof *out), k = 0;
    if (!out) { fputs("loft: out of memory\n", stderr); abort(); }
    uint32_t words = s->referenced.nwords > s->removed.nwords ? s->referenced.nwords : s->removed.nwords;
    for (uint32_t w = 0; w < words; w++) {
        uint64_t bits = (w < s->referenced.nwords ? s->referenced.bits[w] : 0) | (w < s->removed.nwords ? s->removed.bits[w] : 0);
        for (uint32_t b = 0; b < 64; b++) if ((bits >> b) & 1u) out[k++] = w * 64 + b;
    }
    *n = k;
    return out;
}

/* `Store::entomb_kept`: after a reset, each kept position past the root becomes a tombstone
 * again, so a stored reference to it still reads null (`@FR-H-Removed`). */
static void loft_st_entomb_kept(loft_store_t *s, const uint32_t *kept, uint32_t n) {
    uint32_t root_end = LOFT_ST_PRIMARY;
    if (loft_bits_contains(&s->claims, LOFT_ST_PRIMARY)) {
        int32_t h = loft_st_i32(s, LOFT_ST_PRIMARY, 0);
        root_end = LOFT_ST_PRIMARY + (uint32_t) (h < 0 ? -h : h);
    }
    for (uint32_t i = 0; i < n; i++) {
        uint32_t pos = kept[i];
        if (pos < root_end || pos >= s->size) continue;
        loft_st_claim_at(s, pos, 1);
        loft_bits_remove(&s->claims, pos);
        loft_bits_insert(&s->removed, pos);
        loft_st_set_i32(s, pos, 4, 0);
    }
}

/* `Store::holds_only_root`. */
static int loft_st_holds_only_root(const loft_store_t *s) {
    return s->claims.live == 1 && loft_bits_contains(&s->claims, LOFT_ST_PRIMARY);
}

/* `Store::claims_rebuild` with `fl_rebuild` before it (`resync_allocator`): the allocator's
 * state read back off the store's own words, as for an image adopted whole. */
static void loft_st_resync(loft_store_t *s) {
    loft_fl_rebuild(s);
    loft_bits_clear(&s->claims);
    uint32_t pos = LOFT_ST_PRIMARY;
    while (pos < s->size) {
        int32_t h = loft_st_i32(s, pos, 0);
        uint32_t bs = (uint32_t) (h < 0 ? -h : h);
        if (bs == 0) break;
        if (h > 0 && !loft_bits_contains(&s->removed, pos)) loft_bits_insert(&s->claims, pos);
        pos += bs;
    }
}
