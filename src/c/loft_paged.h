/* ── An image read a page at a time (src/paged_reader.rs) ──────────────────────────────────
 * The working-set loaders read a store image by byte range (`@FR-S-Paged`): a page is read
 * from the file the first time a walk touches it, every address comes from the caller's key and
 * the bytes already read, and a byte past the end of the image reads as 0 — so a malformed
 * image answers "absent" and never faults.  Pages are 64 KiB and the cache is uncapped, the
 * Rust reader's default.  The source is a local file, or an `http(s)://` image read by `Range` GETs
 * (`HttpRangeProvider`, through `loft_net.h`) in a unit that has the transport (`LOFT_NET`). */

#define LOFT_PG_PAGE 65536u

#ifdef LOFT_NET
static long loft_net_range(const char *url, uint64_t off, uint8_t *buf, size_t len, char *err, size_t cap);
static int loft_net_size(const char *url, uint64_t *size, char *err, size_t cap);
#endif

/* Is `path` an image a paged load reads over HTTP (`PageSource::open`)? */
static int loft_pg_is_url(const char *path) {
    return strncmp(path, "http://", 7) == 0 || strncmp(path, "https://", 8) == 0;
}

typedef struct {
    FILE *f;
    char *url;                 /* the image's URL when it is read over HTTP, else 0 */
    uint64_t size;
    uint8_t **pages;           /* by page number, 0 until read */
    uint64_t npages;
    /* The tree a walk reads: the nodes it minted, and the fuel a walk spends — a child pointer
     * that cycles makes a corrupt image's walk end instead of spin (PagedTrie, PagedSpatial). */
    uint32_t nodes, fuel;
    /* The text key read last, and its record (PagedTrie::key_bytes); a walk holds one at a
     * time. */
    uint8_t *kbuf;
    size_t kcap, klen;
    uint32_t krec;
    /* PagedReader::trie_v1: the image's tries were written under placement token 1 — keys read
     * raw, no null slot — and are read as written. */
    int trie_v1;
} loft_pg;

/* PageSource::open: the image, refused when it does not begin with the store signature — 1, else
 * 0 with `err` saying why.  A URL is sized by its probe and its first four bytes range-read
 * (`HttpRangeProvider::open`). */
static int loft_pg_open(loft_pg *r, const char *path, char *err, size_t cap) {
    memset(r, 0, sizeof *r);
    r->krec = UINT32_MAX;
    uint8_t head[4];
    uint32_t sig = 0;
    if (loft_pg_is_url(path)) {
#ifdef LOFT_NET
        if (!loft_net_size(path, &r->size, err, cap)) return 0;
        long got = loft_net_range(path, 0, head, 4, err, cap);
        if (got < 0) return 0;
        if (got == 4) memcpy(&sig, head, 4);
        if (got != 4 || sig != LOFT_ST_SIGNATURE) {
            snprintf(err, cap, "%s does not begin with the store signature — it is reachable but is not a loft store image", path);
            return 0;
        }
        size_t n = strlen(path) + 1;
        r->url = (char *) malloc(n);
        if (!r->url) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(r->url, path, n);
#else
        snprintf(err, cap, "this build has no HTTP transport, so it cannot range-read %s", path);
        return 0;
#endif
    } else {
        r->f = fopen(path, "rb");
        if (!r->f) {
            int e = errno;
            snprintf(err, cap, "%s (os error %d)", strerror(e), e);
            return 0;
        }
        size_t got = fread(head, 1, 4, r->f);
        if (got == 4) memcpy(&sig, head, 4);
        if (got != 4 || sig != LOFT_ST_SIGNATURE) {
            snprintf(err, cap, "not a loft store image (the file does not begin with the store signature)");
            fclose(r->f);
            r->f = 0;
            return 0;
        }
        long end = fseek(r->f, 0, SEEK_END) == 0 ? ftell(r->f) : -1;
        r->size = end < 0 ? 0 : (uint64_t) end;
    }
    r->npages = (r->size + LOFT_PG_PAGE - 1) / LOFT_PG_PAGE;
    r->pages = (uint8_t **) calloc(r->npages ? (size_t) r->npages : 1, sizeof(uint8_t *));
    if (!r->pages) { fputs("loft: out of memory\n", stderr); abort(); }
    return 1;
}

static void loft_pg_close(loft_pg *r) {
    for (uint64_t p = 0; p < r->npages; p++) free(r->pages[p]);
    free(r->pages);
    free(r->kbuf);
    free(r->url);
    if (r->f) fclose(r->f);
    memset(r, 0, sizeof *r);
}

/* PagedReader::page: page `p` resident, read on a miss; 0 for a page past the end. */
static const uint8_t *loft_pg_page(loft_pg *r, uint64_t p) {
    if (p >= r->npages) return 0;
    if (!r->pages[p]) {
        uint8_t *b = (uint8_t *) calloc(LOFT_PG_PAGE, 1);
        if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
        uint64_t off = p * LOFT_PG_PAGE, want = r->size - off < LOFT_PG_PAGE ? r->size - off : LOFT_PG_PAGE;
        /* A short read leaves the page's tail as `calloc` made it: zero, which is what a byte
         * past the end reads as.  The count is READ (a `(void)` cast does not satisfy glibc's
         * `warn_unused_result` on `fread`).  A range read that fails reads as zeros too, as the
         * Rust provider's does. */
        if (r->url) {
#ifdef LOFT_NET
            char err[1100];
            if (loft_net_range(r->url, off, b, (size_t) want, err, sizeof err) < 0) memset(b, 0, (size_t) want);
#endif
        } else if (fseek(r->f, (long) off, SEEK_SET) == 0) {
            size_t got = fread(b, 1, (size_t) want, r->f);
            if (got < (size_t) want) memset(b + got, 0, (size_t) want - got);
        }
        r->pages[p] = b;
    }
    return r->pages[p];
}

/* PagedReader::read_into: `len` bytes from byte `off`, zero past the end. */
static void loft_pg_read(loft_pg *r, uint64_t off, void *buf, size_t len) {
    uint8_t *out = (uint8_t *) buf;
    size_t done = 0;
    while (done < len) {
        uint64_t at = off + done;
        size_t in = (size_t) (at % LOFT_PG_PAGE), take = LOFT_PG_PAGE - in;
        if (take > len - done) take = len - done;
        const uint8_t *page = loft_pg_page(r, at / LOFT_PG_PAGE);
        if (page) memcpy(out + done, page + in, take);
        else memset(out + done, 0, take);
        done += take;
    }
}

/* The typed reads at (record, byte), as the `Store` accessors read them. */
static uint32_t loft_pg_u32(loft_pg *r, uint32_t rec, uint32_t fld) {
    uint32_t v;
    loft_pg_read(r, (uint64_t) rec * 8 + fld, &v, 4);
    return v;
}

static int64_t loft_pg_i64(loft_pg *r, uint32_t rec, uint32_t fld) {
    int64_t v;
    loft_pg_read(r, (uint64_t) rec * 8 + fld, &v, 8);
    return v;
}

static uint16_t loft_pg_u16(loft_pg *r, uint32_t rec, uint32_t fld) {
    uint16_t v;
    loft_pg_read(r, (uint64_t) rec * 8 + fld, &v, 2);
    return v;
}

static uint8_t loft_pg_u8(loft_pg *r, uint32_t rec, uint32_t fld) {
    uint8_t v;
    loft_pg_read(r, (uint64_t) rec * 8 + fld, &v, 1);
    return v;
}

/* PagedReader::record_extent: the words of the record at `rec` when the image holds one there —
 * a positive size word, ending inside the image — else 0.  A size or a count read from the image
 * is trusted only as far as the record that holds it (loft#2177). */
static uint32_t loft_pg_record_extent(loft_pg *r, uint32_t rec) {
    int32_t words = (int32_t) loft_pg_u32(r, rec, 0);
    return rec != 0 && words > 0 && ((uint64_t) rec + (uint64_t) words) * 8 <= r->size ? (uint32_t) words : 0;
}

/* paged_reader::field_value: an integer key or axis field, each width decoded by its own reader
 * with its `start` (`@FR-L-Narrow-Decode`, `@FR-S-Paged-Same`). */
static int64_t loft_pg_field_value(loft_pg *r, uint32_t rec, uint32_t p, const loft_key *k) {
    int t = k->type < 0 ? -k->type : k->type;
    switch (t) {
    case 8:
        return (int32_t) loft_pg_u32(r, rec, p);
    case 12:
        return loft_pg_u32(r, rec, p);
    case 9: {
        uint16_t raw = loft_pg_u16(r, rec, p);
        return raw == 0 ? (int64_t) INT32_MIN : (int64_t) raw + k->start - 1;
    }
    case 10:
        return (int64_t) loft_pg_u8(r, rec, p) + k->start;
    case 11:
        return (int64_t) loft_pg_u16(r, rec, p) + k->start;
    default:
        return loft_pg_i64(r, rec, p);
    }
}

/* PagedReader::str_bytes: the bytes of the string record `srec` (its length @4, its bytes from
 * @8) into the key buffer; empty for no record, and for one whose length runs past it. */
static loft_str loft_pg_text_at(loft_pg *r, uint32_t srec) {
    uint32_t words = loft_pg_record_extent(r, srec);
    size_t len = words == 0 ? 0 : loft_pg_u32(r, srec, 4);
    if ((uint64_t) len + 8 > (uint64_t) words * 8) len = 0;
    if (len > r->kcap) {
        r->kbuf = (uint8_t *) realloc(r->kbuf, len);
        if (!r->kbuf) { fputs("loft: out of memory\n", stderr); abort(); }
        r->kcap = len;
    }
    if (len) loft_pg_read(r, (uint64_t) srec * 8 + 8, r->kbuf, len);
    r->klen = len;
    return loft_lit((const char *) r->kbuf, len);
}

/* PagedTrie::key_bytes: record `rec`'s text key, read once while the walk stays on it. */
static loft_str loft_pg_key_text(loft_pg *r, uint32_t rec, const loft_key *k) {
    if (r->krec != rec) {
        (void) loft_pg_text_at(r, loft_pg_u32(r, rec, 8 + k->pos));
        r->krec = rec;
    }
    return loft_lit((const char *) r->kbuf, r->klen);
}

/* paged_reader::tree_at: the tree a collection slot names, and the nodes it minted — no more than
 * its record holds, so a walk's fuel is bounded by the bytes it reads; 0 for none. */
static uint32_t loft_pg_tree(loft_pg *r, uint32_t root_rec, uint32_t root_pos) {
    uint32_t t = loft_pg_u32(r, root_rec, root_pos), words = loft_pg_record_extent(r, t), cap;
    r->krec = UINT32_MAX;
    r->nodes = 0;
    if (words == 0) return 0;
    r->nodes = loft_pg_u32(r, t, LOFT_RX_NODES);
    cap = loft_rx_cap_for_words(words);
    if (r->nodes > cap) r->nodes = cap;
    return t;
}

/* `walk_begin`, and the guard on every child read (PagedTrie::child): no fuel left, or a node
 * the tree never minted, reads as the empty child. */
static void loft_pg_walk_begin(loft_pg *r) { r->fuel = r->nodes > UINT32_MAX - 4 ? UINT32_MAX : r->nodes + 4; }

static int loft_pg_child_ok(loft_pg *r, uint32_t n) {
    if (r->fuel == 0 || n == 0 || n > r->nodes) return 0;
    r->fuel--;
    return 1;
}

/* The walks over a paged image (loft_radix_walk.h, pasted next). */
#define LOFT_RXS loft_pg
#define LOFT_RXF(name) loft_prx_##name
#define LOFT_RX_RD(s, r, o) loft_pg_u32(s, r, o)
#define LOFT_RX_BEGIN(s) loft_pg_walk_begin(s)
#define LOFT_RX_GUARD(s, n) loft_pg_child_ok(s, n)
#define LOFT_RX_KEYI(s, r, k) loft_pg_field_value(s, r, 8 + (k)->pos, k)
#define LOFT_RX_KEYT(s, r, k) loft_pg_key_text(s, r, k)
#define LOFT_RX_ESC(s) (!(s)->trie_v1)
