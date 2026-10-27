/* ── Whole-image loads over a URL (`store_load_url`, `store_load_url_trusted`) ──────────────────
 * `net::fetch_bytes` for a C build: a `file://` URL is read from the filesystem, an `http://` or
 * `https://` one is one HTTP/1.1 GET (`Connection: close`, a `Content-Length` or chunked body)
 * over the runtime's client connection (`loft_https.h`: a POSIX socket, TLS over it for https).
 * `integrity::verify_sha256` pins an image's bytes.  Pasted only into a unit that loads over a
 * URL (`LOFT_NET`). */
#ifdef LOFT_NET
#ifdef LOFT_POSIX_HOST
#include <netdb.h>
#include <sys/socket.h>
#include <unistd.h>
#endif

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

#ifdef LOFT_POSIX_HOST
/* One GET of `http[s]://host[:port]/path`: the 2xx body, a chunked one decoded. */
static int loft_fetch_http(const char *url, loft_bytes *out, char *err, size_t cap) {
    int tls = strncmp(url, "https://", 8) == 0;
    const char *rest = url + (tls ? 8 : 7), *slash = strchr(rest, '/');
    size_t hlen = slash ? (size_t) (slash - rest) : strlen(rest);
    char host[256], port[16];
    snprintf(port, sizeof port, "%s", tls ? "443" : "80");
    if (hlen == 0 || hlen >= sizeof host) {
        snprintf(err, cap, "HTTP error: %s is not a URL this client reads", url);
        return 0;
    }
    memcpy(host, rest, hlen);
    host[hlen] = 0;
    char *colon = strrchr(host, ':');
    if (colon) {
        snprintf(port, sizeof port, "%s", colon + 1);
        *colon = 0;
    }
    const char *path = slash ? slash : "/";
    struct addrinfo hints, *res = 0;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &res) != 0 || !res) {
        snprintf(err, cap, "HTTP error: %s: the host cannot be resolved", url);
        return 0;
    }
    int fd = -1;
    for (struct addrinfo *a = res; a; a = a->ai_next) {
        fd = socket(a->ai_family, a->ai_socktype, a->ai_protocol);
        if (fd < 0) continue;
        if (connect(fd, a->ai_addr, a->ai_addrlen) == 0) break;
        close(fd);
        fd = -1;
    }
    freeaddrinfo(res);
    if (fd < 0) {
        int e = errno;
        snprintf(err, cap, "HTTP error: %s: Connection Failed: %s (os error %d)", url, strerror(e), e);
        return 0;
    }
    loft_stream st = { fd, 0 };
    char why[768];
    if (tls && !loft_stream_tls(&st, host, why, sizeof why)) {
        snprintf(err, cap, "HTTP error: %s: Connection Failed: %s", url, why);
        loft_stream_close(&st);
        return 0;
    }
    char req[1024];
    int rn = snprintf(req, sizeof req, "GET %s HTTP/1.1\r\nHost: %.*s\r\nConnection: close\r\n\r\n", path, (int) hlen, rest);
    loft_stream_write_all(&st, req, rn > 0 && (size_t) rn < sizeof req ? (size_t) rn : 0);
    loft_bytes all = { 0, 0 };
    size_t ac = 0;
    uint8_t buf[65536];
    long got;
    while ((got = loft_stream_read(&st, buf, sizeof buf)) > 0) loft_bytes_push(&all, &ac, buf, (size_t) got);
    loft_stream_close(&st);
    size_t head = 0;
    while (head + 3 < all.n && memcmp(all.p + head, "\r\n\r\n", 4) != 0) head++;
    int status = 0;
    if (all.n >= 12 && memcmp(all.p, "HTTP/", 5) == 0) {
        const uint8_t *sp = memchr(all.p, ' ', all.n);
        if (sp) status = atoi((const char *) sp + 1);
    }
    if (head + 3 >= all.n || status == 0) {
        snprintf(err, cap, "HTTP error: %s: the answer is not HTTP", url);
        free(all.p);
        return 0;
    }
    if (status < 200 || status >= 300) {
        snprintf(err, cap, "HTTP error: %s: status code %d", url, status);
        loft_fetch_absent = status == 403 || status == 404 || status == 410;
        free(all.p);
        return 0;
    }
    int chunked = 0;
    for (size_t i = 0; i + 26 <= head; i++) {
        if ((all.p[i] == 'T' || all.p[i] == 't') && strncmp((const char *) all.p + i + 1, "ransfer-Encoding: chunked", 25) == 0) chunked = 1;
    }
    const uint8_t *body = all.p + head + 4, *end = all.p + all.n;
    size_t oc = 0;
    if (!chunked) {
        loft_bytes_push(out, &oc, body, (size_t) (end - body));
    } else {
        while (body < end) {
            unsigned long len = strtoul((const char *) body, 0, 16);
            const uint8_t *nl = memchr(body, '\n', (size_t) (end - body));
            if (!nl || len == 0) break;
            body = nl + 1;
            if ((size_t) (end - body) < len) len = (unsigned long) (end - body);
            loft_bytes_push(out, &oc, body, len);
            body += len;
            if (body < end && *body == '\r') body++;
            if (body < end && *body == '\n') body++;
        }
    }
    free(all.p);
    return 1;
}
#endif

/* `net::fetch_bytes`: the bytes at `url`, 1; else 0 with `err` saying why. */
static int loft_fetch(const char *url, loft_bytes *out, char *err, size_t cap) {
    out->p = 0;
    out->n = 0;
    loft_fetch_absent = 0;
    if (strncmp(url, "file://", 7) == 0) return loft_fetch_file(url + 7, out, err, cap);
#ifdef LOFT_POSIX_HOST
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
    if (sc && loft_stores[r.store_nr].known == sc->tp) {
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
            free(u);
            return 0;
        }
        if (!got) {
            loft_gate_refuse(who, u, "it has no layout sidecar (`.dschema`) beside it, so nothing records the layout "
                             "its records were written with; write it with store_persist_copy, which writes both");
            free(u);
            return 0;
        }
        char *text = (char *) realloc(b.p, b.n + 1);
        if (!text) { fputs("loft: out of memory\n", stderr); abort(); }
        text[b.n] = 0;
        recorded = loft_sidecar_slot(text);
        if (!loft_image_gate_text(u, text, sc, who)) {
            free(u);
            return 0;
        }
    }
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
