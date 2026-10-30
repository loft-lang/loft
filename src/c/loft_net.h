/* ── Loads over a URL (`store_load_url`, `store_load_url_trusted`, a paged load of an `http(s)://`
 * image) ───────────────────────────────────────────────────────────────────────────────────────
 * `net::fetch_bytes`, `fetch_range` and `fetch_size` for a C build: a `file://` URL is read from
 * the filesystem, an `http://` or `https://` one goes through the runtime's HTTP client
 * (`loft_https.h`: a socket of `loft_sock.h`, TLS over it for https).  `integrity::verify_sha256` pins an
 * image's bytes.  Pasted only into a unit that loads over a URL or pages an image (`LOFT_NET`). */
#ifdef LOFT_NET

/* A fetched body, malloc'd; on failure `err` says why. */
typedef struct { uint8_t *p; size_t n; } loft_bytes;

/* Set by `loft_fetch` when the host answered that it has no such file (403, 404, 410, a
 * `file://` path that cannot be read) — `net::Miss::Absent` — rather than the request failing. */
static LOFT_TLS int loft_fetch_absent = 0;

static void loft_bytes_push(loft_bytes *b, size_t *cap, const uint8_t *p, size_t n) {
    if (b->n + n > *cap) {
        size_t c = *cap ? *cap : 4096;
        while (c < b->n + n) c *= 2;
        b->p = (uint8_t *) realloc(b->p, c);
        if (!b->p) { fputs("loft: out of memory\n", stderr); abort(); }
        *cap = c;
    }
    memcpy(b->p + b->n, p, n);
    b->n += n;
}

static int loft_fetch_file(const char *path, loft_bytes *out, char *err, size_t cap) {
    FILE *f = fopen(path, "rb");
    if (!f) {
        int e = errno;
        loft_fetch_absent = 1;
        snprintf(err, cap, "file:// read error for %s: %s (os error %d)", path, strerror(e), e);
        return 0;
    }
    size_t got, bc = 0;
    uint8_t buf[65536];
    while ((got = fread(buf, 1, sizeof buf, f)) > 0) loft_bytes_push(out, &bc, buf, got);
    fclose(f);
    return 1;
}

#ifdef LOFT_SOCKETS
/* One GET of `http[s]://host[:port]/path`: the 2xx body, a chunked one decoded. */
static int loft_fetch_http(const char *url, loft_bytes *out, char *err, size_t cap) {
    loft_http h;
    char why[1024];
    if (!loft_http_open(&h, "GET", url, "", "", 5, why, sizeof why)) {
        snprintf(err, cap, "HTTP error: %s", why);
        return 0;
    }
    if (h.status < 200 || h.status >= 300) {
        snprintf(err, cap, "HTTP error: %s: status code %d", url, (int) h.status);
        loft_fetch_absent = h.status == 403 || h.status == 404 || h.status == 410;
        loft_http_close(&h);
        return 0;
    }
    size_t oc = 0;
    uint8_t buf[65536];
    long got;
    while ((got = loft_http_read(&h, buf, sizeof buf)) > 0) loft_bytes_push(out, &oc, buf, (size_t) got);
    loft_http_close(&h);
    if (got < 0) {
        free(out->p);
        out->p = 0;
        out->n = 0;
        snprintf(err, cap, "HTTP error: %s: the connection ended inside the body", url);
        return 0;
    }
    return 1;
}
#endif

/* `net::fetch_range`: up to `len` bytes of `url` from byte `off`, by a `Range` GET; a server that
 * ignores the range answers the whole file, which is read through to `off` — right, only slow.
 * The count (short at the end of the file), else -1 with `err` saying why. */
static long loft_net_range(const char *url, uint64_t off, uint8_t *buf, size_t len, char *err, size_t cap) {
    if (len == 0) return 0;
#ifdef LOFT_SOCKETS
    char range[96], why[1024];
    snprintf(range, sizeof range, "Range: bytes=%llu-%llu\r\n", (unsigned long long) off,
             (unsigned long long) (off + len - 1));
    loft_http h;
    if (!loft_http_open(&h, "GET", url, range, "", 5, why, sizeof why)) {
        snprintf(err, cap, "HTTP error: %s", why);
        return -1;
    }
    if (h.status < 200 || h.status >= 400) {
        snprintf(err, cap, "HTTP error: %s: status code %d", url, (int) h.status);
        loft_http_close(&h);
        return -1;
    }
    long r = 1;
    for (uint64_t skip = h.status == 206 ? 0 : off; skip > 0 && r > 0; skip -= (uint64_t) r) {
        uint8_t sink[65536];
        r = loft_http_read(&h, sink, skip < sizeof sink ? (size_t) skip : sizeof sink);
    }
    size_t got = 0;
    while (r > 0 && got < len && (r = loft_http_read(&h, buf + got, len - got)) > 0) got += (size_t) r;
    loft_http_close(&h);
    if (r < 0) {
        snprintf(err, cap, "HTTP error: %s: the connection ended inside the body", url);
        return -1;
    }
    return (long) got;
#else
    (void) off;
    (void) buf;
    snprintf(err, cap, "this build has no HTTP transport, so it cannot range-read %s — a store it carries pages "
             "through the host filesystem instead", url);
    return -1;
#endif
}

/* `net::fetch_size`: the byte length of `url` by a `Range: bytes=0-0` probe — the total of its
 * `Content-Range`, else its `Content-Length`; 1, else 0 with `err` saying why.  The body is not
 * read, so a server that ignores the range does not send the file for nothing. */
static int loft_net_size(const char *url, uint64_t *size, char *err, size_t cap) {
#ifdef LOFT_SOCKETS
    char why[1024];
    loft_http h;
    if (!loft_http_open(&h, "GET", url, "Range: bytes=0-0\r\n", "", 5, why, sizeof why)) {
        snprintf(err, cap, "HTTP error: %s", why);
        return 0;
    }
    if (h.status < 200 || h.status >= 400) {
        snprintf(err, cap, "HTTP error: %s: status code %d", url, (int) h.status);
        loft_http_close(&h);
        return 0;
    }
    const char *v;
    size_t vn;
    int64_t n = -1;
    if (loft_http_header(&h.head, "content-range", &v, &vn)) {
        const char *slash = v + vn;
        while (slash > v && slash[-1] != '/') slash--;
        const char *t = slash, *e = v + vn;
        while (t < e && isspace((unsigned char) *t)) t++;
        if (slash == v || !loft_http_number(t, (size_t) (e - t), &n)) n = -1;
    }
    if (n < 0 && loft_http_header(&h.head, "content-length", &v, &vn) && !loft_http_number(v, vn, &n)) n = -1;
    loft_http_close(&h);
    if (n < 0) {
        snprintf(err, cap, "remote store: could not determine size of %s", url);
        return 0;
    }
    *size = (uint64_t) n;
    return 1;
#else
    (void) size;
    snprintf(err, cap, "this build has no HTTP transport, so it cannot size %s", url);
    return 0;
#endif
}

/* `net::fetch_bytes`: the bytes at `url`, 1; else 0 with `err` saying why. */
static int loft_fetch(const char *url, loft_bytes *out, char *err, size_t cap) {
    out->p = 0;
    out->n = 0;
    loft_fetch_absent = 0;
    if (strncmp(url, "file://", 7) == 0) return loft_fetch_file(url + 7, out, err, cap);
#ifdef LOFT_SOCKETS
    if (strncmp(url, "http://", 7) == 0 || strncmp(url, "https://", 8) == 0) return loft_fetch_http(url, out, err, cap);
#endif
    snprintf(err, cap, "this build has no HTTP transport, so it cannot fetch %s — a store it carries reads with "
             "`store_load`, which needs no network", url);
    return 0;
}

/* FIPS 180-4 SHA-256 (`integrity::sha256_hex`): the digest of `p` as 64 lowercase hex digits. */
static void loft_sha256_hex(const uint8_t *p, size_t n, char hex[65]) {
    static const uint32_t k[64] = {
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
        0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
        0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
        0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
        0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
        0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
        0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2 };
    uint32_t h[8] = { 0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19 };
    size_t total = ((n + 9 + 63) / 64) * 64;
    uint8_t *m = (uint8_t *) calloc(total, 1);
    if (!m) { fputs("loft: out of memory\n", stderr); abort(); }
    if (n) memcpy(m, p, n);
    m[n] = 0x80;
    uint64_t bits = (uint64_t) n * 8;
    for (int i = 0; i < 8; i++) m[total - 1 - i] = (uint8_t) (bits >> (8 * i));
#define LOFT_ROR(x, r) (((x) >> (r)) | ((x) << (32 - (r))))
    for (size_t off = 0; off < total; off += 64) {
        uint32_t w[64];
        for (int i = 0; i < 16; i++)
            w[i] = (uint32_t) m[off + 4 * i] << 24 | (uint32_t) m[off + 4 * i + 1] << 16 | (uint32_t) m[off + 4 * i + 2] << 8 |
                   (uint32_t) m[off + 4 * i + 3];
        for (int i = 16; i < 64; i++) {
            uint32_t s0 = LOFT_ROR(w[i - 15], 7) ^ LOFT_ROR(w[i - 15], 18) ^ (w[i - 15] >> 3);
            uint32_t s1 = LOFT_ROR(w[i - 2], 17) ^ LOFT_ROR(w[i - 2], 19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16] + s0 + w[i - 7] + s1;
        }
        uint32_t a = h[0], b = h[1], c = h[2], d = h[3], e = h[4], f = h[5], g = h[6], hh = h[7];
        for (int i = 0; i < 64; i++) {
            uint32_t t1 = hh + (LOFT_ROR(e, 6) ^ LOFT_ROR(e, 11) ^ LOFT_ROR(e, 25)) + ((e & f) ^ (~e & g)) + k[i] + w[i];
            uint32_t t2 = (LOFT_ROR(a, 2) ^ LOFT_ROR(a, 13) ^ LOFT_ROR(a, 22)) + ((a & b) ^ (a & c) ^ (b & c));
            hh = g;
            g = f;
            f = e;
            e = d + t1;
            d = c;
            c = b;
            b = a;
            a = t1 + t2;
        }
        h[0] += a; h[1] += b; h[2] += c; h[3] += d; h[4] += e; h[5] += f; h[6] += g; h[7] += hh;
    }
#undef LOFT_ROR
    free(m);
    for (int i = 0; i < 8; i++) snprintf(hex + 8 * i, 9, "%08x", (unsigned) h[i]);
}

/* The layout gate of an image at `u` (`check_sidecar` over HTTP): `<u>.dschema` fetched and
 * classified.  A host's own "not there" (`loft_fetch_absent`) refuses it as a bare image, a request
 * that failed refuses naming why; `recorded` gets the sidecar's slot.  1 when it may be read. */
static int loft_net_gate(const char *u, const loft_sidecar *sc, const char *who, long *recorded, int convert) {
    char err[1024];
    loft_bytes b;
    size_t ulen = strlen(u);
    char *side = (char *) malloc(ulen + 9);
    if (!side) { fputs("loft: out of memory\n", stderr); abort(); }
    snprintf(side, ulen + 9, "%s.dschema", u);
    int got = loft_fetch(side, &b, err, sizeof err);
    free(side);
    if (!got && !loft_fetch_absent) {
        /* The request failed: nothing is known about the sidecar. */
        char why[1200];
        snprintf(why, sizeof why, "its layout sidecar (`.dschema`) could not be fetched: %s", err);
        loft_gate_refuse(who, u, why);
        return 0;
    }
    if (!got) {
        loft_gate_refuse(who, u, "it has no layout sidecar (`.dschema`) beside it, so nothing records the layout "
                         "its records were written with; write it with store_persist_copy, which writes both");
        return 0;
    }
    char *text = (char *) realloc(b.p, b.n + 1);
    if (!text) { fputs("loft: out of memory\n", stderr); abort(); }
    text[b.n] = 0;
    if (recorded) *recorded = loft_sidecar_slot(text);
    return loft_image_gate_text(u, text, sc, who, convert);
}

/* `load_url_verified` (with `sha`) and `load_url`: the image's layout gated by `<url>.dschema`
 * — a typed store refuses an image whose sidecar cannot be fetched, as a bare file — then the
 * bytes fetched, pinned by their SHA-256 when one is given, and adopted structurally validated
 * (`Store::from_bytes`). */
static int loft_image_load_url(loft_ref r, loft_str url, const char *sha, const loft_sidecar *sc) {
    const char *who = sha ? "store_load_url" : "store_load_url_trusted";
    if (r.store_nr == 0xFFFF || r.store_nr >= loft_nstores) return 0;
    char *u = loft_cstr_own(url);
    char err[1024];
    loft_bytes b;
    long recorded = -1;
    loft_gate_trie_v1 = 0;
    int gate = sc && loft_stores[r.store_nr].known == sc->tp ? loft_net_gate(u, sc, who, &recorded, 1) : 1;
    if (gate == 0) {
        free(u);
        return 0;
    }
    int trie_v1 = loft_gate_trie_v1;
    loft_gate_trie_v1 = 0;
    if (!loft_fetch(u, &b, err, sizeof err)) {
        fprintf(stderr, sha ? "store loader: refusing %s — fetch failed: %s\n" : "store loader: %s — fetch failed: %s\n", u, err);
        free(u);
        return 0;
    }
    if (sha) {
        char hex[65];
        loft_sha256_hex(b.p, b.n, hex);
        size_t i = 0;
        while (sha[i] && i < 64) {
            char c = sha[i];
            if (c >= 'A' && c <= 'Z') c = (char) (c - 'A' + 'a');
            if (c != hex[i]) break;
            i++;
        }
        if (i != 64 || sha[64] != 0) {
            fprintf(stderr, "store loader: refusing %s — sha256 mismatch: expected `%s`, got `%s`\n", u, sha, hex);
            free(b.p);
            free(u);
            return 0;
        }
    }
    int ok = loft_image_adopt(r, b.p, b.n, 1, sc, recorded, u, who);
    free(u);
    if (ok && trie_v1) loft_trie_refile(sc->types, sc->tp, r.store_nr);
    if (ok && gate == 2) loft_rekey_image(sc, r);
    return ok;
}

static uint8_t loft_store_load_url(loft_ref r, loft_str url, loft_str sha, const loft_sidecar *sc) {
    char *s = loft_cstr_own(sha);
    int ok = loft_image_load_url(r, url, s, sc);
    free(s);
    return (uint8_t) ok;
}

static uint8_t loft_store_load_url_trusted(loft_ref r, loft_str url, const loft_sidecar *sc) {
    return (uint8_t) loft_image_load_url(r, url, 0, sc);
}

#endif
