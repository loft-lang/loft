/* ── A client connection, and the HTTP/1.1 client over it ───────────────────────────────────────
 * The one transport and the one HTTP client under every fetch of the runtime — `store_load_url`'s
 * and the paged loaders' range reads (`loft_net.h`) and `http_fetch`'s (`loft_engine.h`).  An `https://` URL runs over the TLS client of
 * `src/c/tls/loft_tls.c` (BearSSL), which is not part of the unit: `src/c/tls.rs` builds it into a
 * cached archive and names it on a `link:` line, and `LOFT_HTTPS` says it is there.  A unit built
 * without it (a host whose compiler could not build it, `LOFT_TLS_UNAVAILABLE` saying why, or a
 * target with no sockets) answers an `https://` URL with that reason.  Pasted into a unit that
 * fetches over a URL (`LOFT_NET`, `LOFT_ENGINE`), and compiled into the `net` component
 * (`LOFT_WEB`, `src/c/components/net.c`). */
#if defined(LOFT_POSIX_HOST) && (defined(LOFT_NET) || defined(LOFT_ENGINE) || defined(LOFT_WEB))
#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <poll.h>
#include <strings.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>

#ifdef LOFT_HTTPS
struct loft_tls;
struct loft_tls *loft_tls_open(int fd, const char *host, char *err, size_t cap);
long loft_tls_read(struct loft_tls *t, void *buf, size_t n);
int loft_tls_pending(struct loft_tls *t);
int loft_tls_write_all(struct loft_tls *t, const void *buf, size_t n);
void loft_tls_close(struct loft_tls *t);
#endif

typedef struct {
    int fd;
    void *tls; /* a `struct loft_tls *` once `loft_stream_tls` succeeded */
} loft_stream;

/* TLS over the connected socket for `host` (the name its certificate must carry): 1, else 0 with
 * `err` saying why. */
static int loft_stream_tls(loft_stream *s, const char *host, char *err, size_t cap) {
#ifdef LOFT_HTTPS
    s->tls = loft_tls_open(s->fd, host, err, cap);
    return s->tls != 0;
#else
    (void) s;
    (void) host;
#ifdef LOFT_TLS_UNAVAILABLE
    snprintf(err, cap, "this build has no TLS transport (%s)", LOFT_TLS_UNAVAILABLE);
#else
    snprintf(err, cap, "this build has no TLS transport");
#endif
    return 0;
#endif
}

/* Up to `n` bytes: the count, 0 at the end of the stream, -1 on a failure or a timeout. */
static long loft_stream_read(loft_stream *s, void *buf, size_t n) {
#ifdef LOFT_HTTPS
    if (s->tls) return loft_tls_read((struct loft_tls *) s->tls, buf, n);
#endif
    for (;;) {
        ssize_t r = recv(s->fd, buf, n, 0);
        if (r < 0 && errno == EINTR) continue;
        return r < 0 ? -1 : (long) r;
    }
}

/* Every byte of `p`: 1, else 0. */
static int loft_stream_write_all(loft_stream *s, const void *p, size_t n) {
#ifdef LOFT_HTTPS
    if (s->tls) return loft_tls_write_all((struct loft_tls *) s->tls, p, n);
#endif
    const char *c = (const char *) p;
    while (n > 0) {
#if defined(MSG_NOSIGNAL)
        ssize_t w = send(s->fd, c, n, MSG_NOSIGNAL);
#else
        ssize_t w = send(s->fd, c, n, 0);
#endif
        if (w < 0 && errno == EINTR) continue;
        if (w <= 0) return 0;
        c += w;
        n -= (size_t) w;
    }
    return 1;
}

/* Exactly `n` bytes: 1, else 0. */
static int loft_stream_read_exact(loft_stream *s, void *buf, size_t n) {
    char *p = (char *) buf;
    while (n > 0) {
        long r = loft_stream_read(s, p, n);
        if (r <= 0) return 0;
        p += r;
        n -= (size_t) r;
    }
    return 1;
}

/* The session ended and the socket closed. */
static void loft_stream_close(loft_stream *s) {
#ifdef LOFT_HTTPS
    if (s->tls) loft_tls_close((struct loft_tls *) s->tls);
#endif
    s->tls = 0;
    if (s->fd >= 0) close(s->fd);
    s->fd = -1;
}

/* ── Socket mechanics ── */

static void loft_sock_cloexec(int fd) {
    fcntl(fd, F_SETFD, fcntl(fd, F_GETFD) | FD_CLOEXEC);
}

static void loft_sock_blocking(int fd, int on) {
    int fl = fcntl(fd, F_GETFL);
    fcntl(fd, F_SETFL, on ? (fl & ~O_NONBLOCK) : (fl | O_NONBLOCK));
}

static void loft_sock_timeout(int fd, int64_t ms) {
    struct timeval tv = { (time_t) (ms / 1000), (long) ((ms % 1000) * 1000) };
    setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof tv);
}

/* Connect within `ms`: 1 when the TCP handshake completed. */
static int loft_sock_connect_within(int fd, const struct sockaddr *a, socklen_t al, int64_t ms) {
    loft_sock_blocking(fd, 0);
    int r = connect(fd, a, al);
    if (r != 0 && errno != EINPROGRESS) return 0;
    if (r != 0) {
        struct pollfd p = { fd, POLLOUT, 0 };
        if (poll(&p, 1, (int) ms) <= 0) return 0;
        int err = 0;
        socklen_t el = sizeof err;
        if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &el) != 0 || err != 0) return 0;
    }
    loft_sock_blocking(fd, 1);
    return 1;
}

/* ── HTTP/1.1 ── */

/* Read an HTTP head from `s` to its blank line, a byte at a time (no over-read past it); 0 on a
 * closed or silent connection, or a head past 16 KiB. */
static int loft_http_head(loft_stream *s, loft_text *head) {
    while (head->len < 4 || memcmp(head->p + head->len - 4, "\r\n\r\n", 4) != 0) {
        char b;
        if (loft_stream_read(s, &b, 1) != 1) return 0;
        loft_push(head, &b, 1);
        if (head->len > 16 * 1024) return 0;
    }
    return 1;
}

/* The value of header `name` (case-blind) in an HTTP head, trimmed; 0 when absent. */
static int loft_http_header(const loft_text *head, const char *name, const char **v, size_t *vn) {
    size_t nl = strlen(name), i = 0;
    while (i < head->len) {
        size_t s = i;
        while (i < head->len && head->p[i] != '\n') i++;
        size_t e = i;
        i++;
        if (e > s && head->p[e - 1] == '\r') e--;
        const char *colon = (const char *) memchr(head->p + s, ':', e - s);
        if (!colon) continue;
        size_t ks = s, ke = (size_t) (colon - head->p);
        while (ks < ke && isspace((unsigned char) head->p[ks])) ks++;
        while (ke > ks && isspace((unsigned char) head->p[ke - 1])) ke--;
        if (ke - ks != nl) continue;
        size_t k = 0;
        while (k < nl && tolower((unsigned char) head->p[ks + k]) == tolower((unsigned char) name[k])) k++;
        if (k != nl) continue;
        size_t vs = (size_t) (colon - head->p) + 1, ve = e;
        while (vs < ve && isspace((unsigned char) head->p[vs])) vs++;
        while (ve > vs && isspace((unsigned char) head->p[ve - 1])) ve--;
        *v = head->p + vs;
        *vn = ve - vs;
        return 1;
    }
    return 0;
}

/* The decimal integer a header value holds: 1, else 0. */
static int loft_http_number(const char *p, size_t n, int64_t *out) {
    if (n == 0 || n > 18) return 0;
    int64_t x = 0;
    for (size_t i = 0; i < n; i++) {
        if (p[i] < '0' || p[i] > '9') return 0;
        x = x * 10 + (p[i] - '0');
    }
    *out = x;
    return 1;
}

/* A response being read: its status and head, then its body through `loft_http_read`. */
typedef struct {
    loft_stream st;
    loft_text head;
    int64_t status;
    int chunked, in_chunk, ended;
    int64_t left; /* body bytes left (of the chunk, when chunked); -1 reads to the close */
} loft_http;

/* One HTTP/1.1 request — `headers` complete `Name: value\r\n` lines, `body` sent with its length
 * — and the response's head read: 1, else 0 with `err` saying why as the Rust client (`ureq`)
 * words it.  `hops` redirects are followed, a 303 (and a POST's 301 or 302) as a GET, as the Rust
 * clients follow them.  Connecting and every read are bounded by 30 seconds, the Rust clients'
 * bound. */
static int loft_http_open(loft_http *h, const char *method, const char *url, const char *headers, const char *body,
                          int hops, char *err, size_t cap) {
    memset(h, 0, sizeof *h);
    h->st.fd = -1;
    int tls = strncmp(url, "https://", 8) == 0;
    if (!tls && strncmp(url, "http://", 7) != 0) {
        snprintf(err, cap, "%s: Bad URL: unknown scheme", url);
        return 0;
    }
    const char *auth = url + (tls ? 8 : 7), *slash = strchr(auth, '/');
    size_t alen = slash ? (size_t) (slash - auth) : strlen(auth);
    const char *path = slash ? slash : "/";
    char host[512], port[16];
    snprintf(port, sizeof port, "%s", tls ? "443" : "80");
    const char *hp = auth, *colon = 0;
    size_t hl = alen;
    if (alen && auth[0] == '[') {
        const char *close_ = (const char *) memchr(auth, ']', alen);
        if (!close_) { snprintf(err, cap, "%s: Bad URL: unterminated IPv6 host", url); return 0; }
        hp = auth + 1;
        hl = (size_t) (close_ - hp);
        if ((size_t) (close_ + 1 - auth) < alen && close_[1] == ':') colon = close_ + 1;
    } else {
        colon = (const char *) memchr(auth, ':', alen);
        if (colon) hl = (size_t) (colon - auth);
    }
    if (hl == 0 || hl >= sizeof host) { snprintf(err, cap, "%s: Bad URL: no host", url); return 0; }
    memcpy(host, hp, hl);
    host[hl] = 0;
    if (colon) {
        size_t pl = alen - (size_t) (colon + 1 - auth);
        if (pl == 0 || pl >= sizeof port) { snprintf(err, cap, "%s: Bad URL: bad port", url); return 0; }
        memcpy(port, colon + 1, pl);
        port[pl] = 0;
    }
    struct addrinfo hints, *res = 0, *ai;
    memset(&hints, 0, sizeof hints);
    hints.ai_socktype = SOCK_STREAM;
    int gai = getaddrinfo(host, port, &hints, &res);
    if (gai != 0) {
        snprintf(err, cap, "%s: Dns Failed: %s", url, gai_strerror(gai));
        return 0;
    }
    int fd = -1, e = 0;
    for (ai = res; ai && fd < 0; ai = ai->ai_next) {
        fd = socket(ai->ai_family, SOCK_STREAM, 0);
        if (fd < 0) continue;
        loft_sock_cloexec(fd);
        if (!loft_sock_connect_within(fd, ai->ai_addr, (socklen_t) ai->ai_addrlen, 30000)) {
            int se = 0;
            socklen_t el = sizeof se;
            getsockopt(fd, SOL_SOCKET, SO_ERROR, &se, &el);
            e = se ? se : errno;
            close(fd);
            fd = -1;
        }
    }
    freeaddrinfo(res);
    if (fd < 0) {
        if (!e) e = ECONNREFUSED;
        snprintf(err, cap, "%s: Connection Failed: Connect error: %s (os error %d)", url, strerror(e), e);
        return 0;
    }
    loft_sock_timeout(fd, 30000);
    struct timeval tv = { 30, 0 };
    setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof tv);
    h->st.fd = fd;
    if (tls) {
        char why[768];
        if (!loft_stream_tls(&h->st, host, why, sizeof why)) {
            loft_stream_close(&h->st);
            snprintf(err, cap, "%s: Connection Failed: %s", url, why);
            return 0;
        }
    }
    loft_text req = { 0, 0, 0 };
    loft_push_cstr(&req, method);
    loft_push_cstr(&req, " ");
    loft_push_cstr(&req, path);
    loft_push_cstr(&req, " HTTP/1.1\r\nHost: ");
    loft_push(&req, auth, alen);
    loft_push_cstr(&req, "\r\n");
    loft_push_cstr(&req, headers);
    if (*body) {
        char cl[48];
        snprintf(cl, sizeof cl, "Content-Length: %zu\r\n", strlen(body));
        loft_push_cstr(&req, cl);
    }
    loft_push_cstr(&req, "Connection: close\r\n\r\n");
    loft_push_cstr(&req, body);
    int ok = loft_stream_write_all(&h->st, req.p, req.len) && loft_http_head(&h->st, &h->head);
    free(req.p);
    if (!ok) {
        loft_stream_close(&h->st);
        free(h->head.p);
        snprintf(err, cap, "%s: Connection Failed: the server closed the connection or timed out", url);
        return 0;
    }
    h->status = -1;
    const char *sp = (const char *) memchr(h->head.p, ' ', h->head.len);
    if (sp && h->head.len - (size_t) (sp - h->head.p) > 3) loft_http_number(sp + 1, 3, &h->status);
    const char *v;
    size_t vn;
    if (h->status >= 300 && h->status < 400 && h->status != 304 && hops > 0 && loft_http_header(&h->head, "location", &v, &vn)) {
        loft_text next = { 0, 0, 0 };
        if (vn && v[0] == '/') {
            loft_push_cstr(&next, tls ? "https://" : "http://");
            loft_push(&next, auth, alen);
        }
        loft_push(&next, v, vn);
        loft_push(&next, "", 1);
        loft_stream_close(&h->st);
        free(h->head.p);
        int get = h->status == 303 || ((h->status == 301 || h->status == 302) && strcmp(method, "POST") == 0);
        int r = loft_http_open(h, get ? "GET" : method, next.p, headers, get ? "" : body, hops - 1, err, cap);
        free(next.p);
        return r;
    }
    h->chunked = loft_http_header(&h->head, "transfer-encoding", &v, &vn) && vn >= 7 && strncasecmp(v + vn - 7, "chunked", 7) == 0;
    h->left = -1;
    if (!h->chunked && loft_http_header(&h->head, "content-length", &v, &vn) && !loft_http_number(v, vn, &h->left)) h->left = -1;
    if (strcmp(method, "HEAD") == 0 || h->status == 204 || h->status == 304 || (h->status >= 100 && h->status < 200)) h->ended = 1;
    if (h->chunked) h->left = 0;
    if (!h->chunked && h->left == 0) h->ended = 1;
    return 1;
}

/* Up to `n` bytes of the body, a chunked one decoded: the count, 0 at its end, -1 on a failure or
 * a timeout. */
static long loft_http_read(loft_http *h, void *buf, size_t n) {
    if (h->ended || n == 0) return 0;
    if (h->chunked && h->left == 0) {
        char crlf[2];
        if (h->in_chunk && !loft_stream_read_exact(&h->st, crlf, 2)) return -1;
        loft_text line = { 0, 0, 0 };
        while (line.len < 2 || memcmp(line.p + line.len - 2, "\r\n", 2) != 0) {
            char b;
            if (loft_stream_read(&h->st, &b, 1) != 1 || line.len > 1024) { free(line.p); return -1; }
            loft_push(&line, &b, 1);
        }
        loft_push(&line, "", 1);
        unsigned long size = strtoul(line.p, 0, 16);
        free(line.p);
        if (size == 0) { h->ended = 1; return 0; }
        h->left = (int64_t) size;
        h->in_chunk = 1;
    }
    if (h->left >= 0 && (uint64_t) h->left < n) n = (size_t) h->left;
    long r = loft_stream_read(&h->st, buf, n);
    if (r <= 0) {
        h->ended = 1;
        return h->left < 0 && r == 0 ? 0 : -1;
    }
    if (h->left >= 0) {
        h->left -= r;
        if (h->left == 0 && !h->chunked) h->ended = 1;
    }
    return r;
}

/* The connection closed and the head freed; the rest of the body is not read. */
static void loft_http_close(loft_http *h) {
    loft_stream_close(&h->st);
    free(h->head.p);
    h->head.p = 0;
}

/* ── WebSocket handshake ── */

/* SHA-1 (RFC 3174), for the WebSocket accept key alone. */
static void loft_ws_sha1(const uint8_t *data, size_t n, uint8_t out[20]) {
    uint32_t h[5] = { 0x67452301u, 0xEFCDAB89u, 0x98BADCFEu, 0x10325476u, 0xC3D2E1F0u };
    size_t total = ((n + 8) / 64 + 1) * 64;
    uint8_t *m = (uint8_t *) calloc(total, 1);
    if (!m) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(m, data, n);
    m[n] = 0x80;
    uint64_t bits = (uint64_t) n * 8;
    for (int i = 0; i < 8; i++) m[total - 1 - i] = (uint8_t) (bits >> (8 * i));
    for (size_t c = 0; c < total; c += 64) {
        uint32_t w[80];
        for (int i = 0; i < 16; i++)
            w[i] = (uint32_t) m[c + 4 * i] << 24 | (uint32_t) m[c + 4 * i + 1] << 16 | (uint32_t) m[c + 4 * i + 2] << 8 |
                   m[c + 4 * i + 3];
        for (int i = 16; i < 80; i++) {
            uint32_t x = w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16];
            w[i] = x << 1 | x >> 31;
        }
        uint32_t a = h[0], b = h[1], cc = h[2], d = h[3], e = h[4];
        for (int i = 0; i < 80; i++) {
            uint32_t f, k;
            if (i < 20) { f = (b & cc) | (~b & d); k = 0x5A827999u; }
            else if (i < 40) { f = b ^ cc ^ d; k = 0x6ED9EBA1u; }
            else if (i < 60) { f = (b & cc) | (b & d) | (cc & d); k = 0x8F1BBCDCu; }
            else { f = b ^ cc ^ d; k = 0xCA62C1D6u; }
            uint32_t t = (a << 5 | a >> 27) + f + e + k + w[i];
            e = d;
            d = cc;
            cc = b << 30 | b >> 2;
            b = a;
            a = t;
        }
        h[0] += a; h[1] += b; h[2] += cc; h[3] += d; h[4] += e;
    }
    free(m);
    for (int i = 0; i < 5; i++)
        for (int j = 0; j < 4; j++) out[4 * i + j] = (uint8_t) (h[i] >> (24 - 8 * j));
}

/* `ws_handshake::ws_accept_key`: base64(SHA-1(key + the RFC 6455 GUID)). */
static void loft_ws_accept_key(const char *key, size_t kn, char out[29]) {
    static const char guid[] = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    static const char b64[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    uint8_t in[256], d[20];
    size_t n = kn < 200 ? kn : 200;
    memcpy(in, key, n);
    memcpy(in + n, guid, sizeof guid - 1);
    loft_ws_sha1(in, n + sizeof guid - 1, d);
    size_t o = 0;
    for (int i = 0; i < 20; i += 3) {
        uint32_t v = (uint32_t) d[i] << 16 | (i + 1 < 20 ? (uint32_t) d[i + 1] << 8 : 0) | (i + 2 < 20 ? d[i + 2] : 0);
        out[o++] = b64[(v >> 18) & 63];
        out[o++] = b64[(v >> 12) & 63];
        out[o++] = i + 1 < 20 ? b64[(v >> 6) & 63] : '=';
        out[o++] = i + 2 < 20 ? b64[v & 63] : '=';
    }
    out[o] = 0;
}
/* ── WebSocket frames (RFC 6455) ── */

/* One final frame on `s`, masked with `mask` when it is given (a client's frames must be, a
 * server's must not be), written in one call: 1 when every byte went out. */
static int loft_ws_write_frame(loft_stream *s, int opcode, const char *p, size_t n, const uint8_t *mask) {
    loft_text f = { 0, 0, 0 };
    uint8_t hdr[14];
    uint8_t bit = mask ? 0x80 : 0;
    size_t h = 0;
    hdr[h++] = (uint8_t) (0x80 | opcode);
    if (n < 126) {
        hdr[h++] = (uint8_t) (bit | n);
    } else if (n <= 0xFFFF) {
        hdr[h++] = (uint8_t) (bit | 126);
        hdr[h++] = (uint8_t) (n >> 8);
        hdr[h++] = (uint8_t) n;
    } else {
        hdr[h++] = (uint8_t) (bit | 127);
        for (int i = 7; i >= 0; i--) hdr[h++] = (uint8_t) ((uint64_t) n >> (8 * i));
    }
    if (mask) {
        memcpy(hdr + h, mask, 4);
        h += 4;
    }
    loft_push(&f, (const char *) hdr, h);
    if (mask) {
        for (size_t i = 0; i < n; i++) {
            char c = (char) (p[i] ^ mask[i % 4]);
            loft_push(&f, &c, 1);
        }
    } else if (n) {
        loft_push(&f, p, n);
    }
    int ok = loft_stream_write_all(s, f.p, f.len);
    free(f.p);
    return ok;
}

/* One frame of at most `cap` payload bytes, masked or not: 1 with its opcode and payload, 0 when
 * the stream failed or ended, or the frame claims more than `cap` (refused before it is
 * allocated, so a peer cannot claim a length that aborts the process). */
static int loft_ws_read_frame(loft_stream *s, int *opcode, loft_text *payload, uint64_t cap) {
    uint8_t h[2];
    if (!loft_stream_read_exact(s, h, 2)) return 0;
    uint64_t len = h[1] & 0x7F;
    if (len == 126) {
        uint8_t b[2];
        if (!loft_stream_read_exact(s, b, 2)) return 0;
        len = (uint64_t) b[0] << 8 | b[1];
    } else if (len == 127) {
        uint8_t b[8];
        if (!loft_stream_read_exact(s, b, 8)) return 0;
        len = 0;
        for (int i = 0; i < 8; i++) len = len << 8 | b[i];
    }
    if (len > cap) return 0;
    uint8_t mask[4] = { 0, 0, 0, 0 };
    if ((h[1] & 0x80) && !loft_stream_read_exact(s, mask, 4)) return 0;
    payload->len = 0;
    if (len) {
        char *p = (char *) malloc((size_t) len);
        if (!p) { fputs("loft: out of memory\n", stderr); abort(); }
        if (!loft_stream_read_exact(s, p, (size_t) len)) {
            free(p);
            return 0;
        }
        for (uint64_t i = 0; i < len; i++) p[i] ^= (char) mask[i % 4];
        loft_push(payload, p, (size_t) len);
        free(p);
    }
    *opcode = h[0] & 0x0F;
    return 1;
}
#endif
