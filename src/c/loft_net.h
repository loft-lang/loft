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

/* The layout gate of an image at `u` (`check_sidecar` over HTTP): `<u>.dschema` fetched and
 * classified.  A host's own "not there" (`loft_fetch_absent`) refuses it as a bare image, a request
 * that failed refuses naming why; `recorded` gets the sidecar's slot.  1 when it may be read. */
/* The sidecar text a URL gate kept for a converting load (gate 3). */
static LOFT_TLS char *loft_net_gate_kept;

static int loft_net_gate_pinned(const char *u, const loft_sidecar *sc, const char *who, long *recorded, int convert,
                                const char *schema_sha);
static int loft_net_gate(const char *u, const loft_sidecar *sc, const char *who, long *recorded, int convert) {
    return loft_net_gate_pinned(u, sc, who, recorded, convert, 0);
}

/* `loft_net_gate` with the sidecar held to `schema_sha` (lowercase or uppercase hex) when one is
 * given: the layout file decides how the pinned image is read, so a missing or different one
 * refuses the load (`url_layout_gate`'s pin). */
static int loft_net_gate_pinned(const char *u, const loft_sidecar *sc, const char *who, long *recorded, int convert,
                                const char *schema_sha) {
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
        loft_gate_refuse(who, u, schema_sha && *schema_sha
                                     ? "its layout sidecar (`.dschema`) is pinned and could not be fetched"
                                     : "it has no layout sidecar (`.dschema`) beside it, so nothing records the layout "
                                       "its records were written with; write it with store_persist_copy, which writes both");
        return 0;
    }
    if (schema_sha && *schema_sha && !loft_sha256_matches(b.p, b.n, schema_sha)) {
        char hex[65], why[256];
        loft_sha256_hex(b.p, b.n, hex);
        snprintf(why, sizeof why, "its layout sidecar (`.dschema`) does not match the pin: expected `%.64s`, got `%s`",
                 schema_sha, hex);
        loft_gate_refuse(who, u, why);
        free(b.p);
        return 0;
    }
    char *text = (char *) realloc(b.p, b.n + 1);
    if (!text) { fputs("loft: out of memory\n", stderr); abort(); }
    text[b.n] = 0;
    if (recorded) *recorded = loft_sidecar_slot(text);
    /* A layout the load converts (3) needs the text the gate consumes: a copy is kept for it
     * (`url_layout_gate`'s `Ok(Some(sidecar))`), so the sidecar is fetched once. */
    char *copy = convert ? loft_up_dup_text(text) : 0;
    int verdict = loft_image_gate_text(u, text, sc, who, convert);
    free(loft_net_gate_kept);
    loft_net_gate_kept = verdict == 3 ? copy : 0;
    if (verdict != 3) free(copy);
    return verdict;
}

/* `load_url_verified` (with `sha`) and `load_url`: the image's layout gated by `<url>.dschema`
 * — a typed store refuses an image whose sidecar cannot be fetched, as a bare file — then the
 * bytes fetched, pinned by their SHA-256 when one is given, and adopted structurally validated
 * (`Store::from_bytes`). */
static int loft_image_load_url(loft_ref r, loft_str url, const char *sha, const char *schema_sha,
                               const loft_sidecar *sc) {
    const char *who = sha ? "store_load_url" : "store_load_url_trusted";
    if (r.store_nr == 0xFFFF || r.store_nr >= loft_nstores) return 0;
    char *u = loft_cstr_own(url);
    char err[1024];
    loft_bytes b;
    long recorded = -1;
    loft_gate_trie_v1 = 0;
    int gate = sc && LOFT_KNOWN(loft_stores[r.store_nr]) == sc->tp ? loft_net_gate_pinned(u, sc, who, &recorded, 1, schema_sha) : 1;
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
        /* The pin is the fetched image's own, checked before anything is converted. */
        loft_sha256_hex(b.p, b.n, hex);
        if (!loft_sha256_matches(b.p, b.n, sha)) {
            fprintf(stderr, "store loader: refusing %s — sha256 mismatch: expected `%s`, got `%s`\n", u, sha, hex);
            free(b.p);
            free(u);
            return 0;
        }
    }
#ifdef LOFT_UPGRADE
    if (gate == 3) {
        /* An image of another layout converted in memory, as `store_load` converts a file. */
        char *text = loft_net_gate_kept;
        loft_net_gate_kept = 0;
        loft_ub why = { 0, 0, 0 };
        int ok = loft_up_load(r, sc, text, b.p, b.n, u, &why);
        if (!ok)
            fprintf(stderr, "%s: refusing %s \xe2\x80\x94 it was written with a different layout, and it cannot be converted to this program's: %s.\n",
                    who, u, loft_ub_str(&why));
        loft_ub_free(&why);
        free(text);
        free(u);
        return ok;
    }
#else
    /* No converter in this unit: an image of another layout is never read raw. */
    if (gate == 3) {
        free(b.p);
        free(u);
        return 0;
    }
#endif
    int ok = loft_image_adopt(r, b.p, b.n, 1, sc, recorded, u, who);
    free(u);
    if (ok && trie_v1) loft_trie_refile(sc->types, sc->tp, r.store_nr);
    if (ok && gate == 2) loft_rekey_image(sc, r);
    return ok;
}

static uint8_t loft_store_load_url(loft_ref r, loft_str url, loft_str sha, loft_str schema_sha, const loft_sidecar *sc) {
    char *s = loft_cstr_own(sha);
    char *ss = loft_cstr_own(schema_sha);
    int ok = loft_image_load_url(r, url, s, ss, sc);
    free(s);
    free(ss);
    return (uint8_t) ok;
}

static uint8_t loft_store_load_url_trusted(loft_ref r, loft_str url, const loft_sidecar *sc) {
    return (uint8_t) loft_image_load_url(r, url, 0, 0, sc);
}

#endif
