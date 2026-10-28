/* ── The web library's natives, compiled into the `net` component (`src/c/components/net.c`) ───
 * The HTTP client is the runtime's (`loft_https.h`: a socket, TLS over it for https, redirects,
 * the 30-second bounds); a WebSocket session is RFC 6455 over the same client connection,
 * reconnecting with backoff.  A response body, a binary frame and the
 * pack buffer are bytes carried in a `text` (the library reads them with `byte_at`), so they are
 * copied as bytes.  Per-thread.  Never pasted into a unit: the component defines `LOFT_WEB`
 * (`c::component`); POSIX hosts. */
#ifdef LOFT_WEB
#if defined(LOFT_POSIX_HOST)
#include <time.h>

static LOFT_TLS loft_text loft_web_body;
static LOFT_TLS loft_text loft_web_heads;
static LOFT_TLS loft_text loft_web_pack;
static LOFT_TLS loft_text loft_web_msg;
static LOFT_TLS int64_t loft_web_op = 0;

static void loft_web_set(loft_text *t, const char *p, size_t n) {
    t->len = 0;
    if (n) loft_push(t, p, n);
}

static loft_text loft_web_copy(const loft_text *t) { return loft_text_own(loft_lit(t->p ? t->p : "", t->len)); }

/* ── HTTP ── */

/* `http_do(method, url, body, headers) -> status`: the body and the headers (`Key: Value` lines)
 * wait for `http_body` / `http_headers`.  An HTTP error status carries its body; a transport
 * failure is status 0. */
static int64_t loft_web_http_do(loft_str method, loft_str url, loft_str body, loft_str headers) {
    loft_web_body.len = 0;
    loft_web_heads.len = 0;
    char *m = loft_cstr_own(method), *u = loft_cstr_own(url), *b = loft_cstr_own(body);
    int known = !strcmp(m, "GET") || !strcmp(m, "POST") || !strcmp(m, "PUT") || !strcmp(m, "DELETE") || !strcmp(m, "HEAD");
    loft_text lines = { 0, 0, 0 };
    for (size_t i = 0; i < headers.len;) {
        size_t e = i;
        while (e < headers.len && headers.p[e] != '\n') e++;
        const char *c = (const char *) memchr(headers.p + i, ':', e - i);
        if (c) {
            const char *k = headers.p + i, *v = c + 1, *ve = headers.p + e;
            const char *ke = c;
            while (k < ke && isspace((unsigned char) *k)) k++;
            while (ke > k && isspace((unsigned char) ke[-1])) ke--;
            while (v < ve && isspace((unsigned char) *v)) v++;
            while (ve > v && isspace((unsigned char) ve[-1])) ve--;
            loft_push(&lines, k, (size_t) (ke - k));
            loft_push_cstr(&lines, ": ");
            loft_push(&lines, v, (size_t) (ve - v));
            loft_push_cstr(&lines, "\r\n");
        }
        i = e + 1;
    }
    loft_push(&lines, "", 1);
    int64_t status = 0;
    loft_http h;
    char err[1024];
    if (known && loft_http_open(&h, m, u, lines.p, b, 5, err, sizeof err)) {
        status = h.status;
        /* The head's lines after the status line, as `Key: Value` joined by "\n". */
        const char *p = h.head.p, *end = h.head.p + h.head.len;
        const char *nl = (const char *) memchr(p, '\n', h.head.len);
        for (p = nl ? nl + 1 : end; p < end;) {
            const char *e = (const char *) memchr(p, '\n', (size_t) (end - p));
            const char *le = e ? e : end;
            const char *c = (const char *) memchr(p, ':', (size_t) (le - p));
            if (c) {
                const char *v = c + 1, *ve = le;
                while (v < ve && isspace((unsigned char) *v)) v++;
                while (ve > v && isspace((unsigned char) ve[-1])) ve--;
                if (loft_web_heads.len) loft_push(&loft_web_heads, "\n", 1);
                loft_push(&loft_web_heads, p, (size_t) (c - p));
                loft_push_cstr(&loft_web_heads, ": ");
                loft_push(&loft_web_heads, v, (size_t) (ve - v));
            }
            p = e ? e + 1 : end;
        }
        char buf[65536];
        long r;
        while ((r = loft_http_read(&h, buf, sizeof buf)) > 0) loft_push(&loft_web_body, buf, (size_t) r);
        loft_http_close(&h);
    }
    free(lines.p);
    free(m);
    free(u);
    free(b);
    return status;
}

static loft_text loft_web_http_body(void) { return loft_web_copy(&loft_web_body); }
static loft_text loft_web_http_headers(void) { return loft_web_copy(&loft_web_heads); }

/* A URL's `Content-Length` by a `HEAD`, or -1. */
static int64_t loft_web_http_head_size(loft_str url) {
    int64_t status = loft_web_http_do(loft_lit("HEAD", 4), url, loft_lit("", 0), loft_lit("", 0));
    if (status < 200 || status >= 400) return -1;
    loft_text h = loft_web_heads;
    for (size_t i = 0; i < h.len;) {
        size_t e = i;
        while (e < h.len && h.p[e] != '\n') e++;
        if (e - i > 16 && strncasecmp(h.p + i, "content-length: ", 16) == 0) {
            int64_t n = -1;
            return loft_http_number(h.p + i + 16, e - i - 16, &n) ? n : -1;
        }
        i = e + 1;
    }
    return -1;
}

/* ── The pack builder and byte reads ── */

static void loft_web_pack_reset(void) { loft_web_pack.len = 0; }
static void loft_web_pack_u8(int64_t b) {
    char c = (char) (b & 0xff);
    loft_push(&loft_web_pack, &c, 1);
}
static void loft_web_pack_u16_le(int64_t v) {
    char c[2] = { (char) (v & 0xff), (char) ((v >> 8) & 0xff) };
    loft_push(&loft_web_pack, c, 2);
}
static void loft_web_pack_u32_le(int64_t v) {
    char c[4] = { (char) (v & 0xff), (char) ((v >> 8) & 0xff), (char) ((v >> 16) & 0xff), (char) ((v >> 24) & 0xff) };
    loft_push(&loft_web_pack, c, 4);
}
static loft_text loft_web_pack_take(void) {
    loft_text t = loft_web_copy(&loft_web_pack);
    loft_web_pack.len = 0;
    return t;
}
static int64_t loft_web_byte_at(int64_t idx, loft_str t) {
    return idx < 0 || (uint64_t) idx >= t.len ? -1 : (int64_t) (unsigned char) t.p[idx];
}
static void loft_web_sleep_ms(int64_t ms) {
    if (ms <= 0) return;
    struct timespec ts = { (time_t) (ms / 1000), (long) ((ms % 1000) * 1000000) };
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {
    }
}
static void loft_web_frame_yield(void) {}

/* ── The WebSocket client ── */

typedef struct {
    char *url;    /* 0: a closed slot */
    loft_stream st; /* `fd < 0`: down, reconnected after the backoff */
    int64_t last_attempt_ms;
    int backoff;
} loft_web_conn;

static LOFT_TLS loft_web_conn *loft_web_conns = 0;
static LOFT_TLS size_t loft_web_nconns = 0;
static LOFT_TLS int64_t *loft_web_group = 0;
static LOFT_TLS size_t loft_web_ngroup = 0, loft_web_group_cap = 0, loft_web_group_offset = 0;

static const int64_t loft_web_backoff_ms[] = { 100, 250, 500, 1000, 2500, 5000, 10000 };
#define LOFT_WEB_BACKOFFS ((int) (sizeof loft_web_backoff_ms / sizeof loft_web_backoff_ms[0]))

static int64_t loft_web_now_ms(void) { return loft_now_us() / 1000; }

/* `(tls, host, port, path)` of a `ws://` / `wss://` URL, the port defaulting by scheme. */
static int loft_web_parse(const char *url, int *tls, char *host, size_t hcap, char *port, char *path, size_t pcap) {
    const char *rest;
    if (strncmp(url, "wss://", 6) == 0) { *tls = 1; rest = url + 6; snprintf(port, 16, "443"); }
    else if (strncmp(url, "ws://", 5) == 0) { *tls = 0; rest = url + 5; snprintf(port, 16, "80"); }
    else return 0;
    const char *slash = strchr(rest, '/');
    size_t alen = slash ? (size_t) (slash - rest) : strlen(rest);
    snprintf(path, pcap, "%s", slash ? slash : "/");
    const char *colon = 0;
    for (size_t i = 0; i < alen; i++)
        if (rest[i] == ':') colon = rest + i;
    size_t hl = colon ? (size_t) (colon - rest) : alen;
    if (hl == 0 || hl >= hcap) return 0;
    memcpy(host, rest, hl);
    host[hl] = 0;
    if (colon) {
        size_t pl = alen - hl - 1;
        if (pl == 0 || pl > 5) return 0;
        for (size_t i = 0; i < pl; i++)
            if (colon[1 + i] < '0' || colon[1 + i] > '9') return 0;
        memcpy(port, colon + 1, pl);
        port[pl] = 0;
    }
    return 1;
}

/* A nonce or a frame mask: entropy-mixed, not cryptographic, as RFC 6455 asks of neither. */
static void loft_web_random(uint8_t *out, size_t n) {
    static LOFT_TLS uint64_t seed = 0;
    if (!seed) seed = (uint64_t) loft_now_us() ^ 0xDEADBEEFull;
    for (size_t i = 0; i < n; i++) {
        seed = seed * 6364136223846793005ull + 1442695040888963407ull;
        out[i] = (uint8_t) (seed >> 33);
    }
}

static int loft_web_write_frame(loft_stream *s, int opcode, const char *p, size_t n) {
    uint8_t mask[4];
    loft_web_random(mask, 4);
    return loft_ws_write_frame(s, opcode, p, n, mask);
}

/* One frame: 1 with its opcode and payload, 0 when the stream failed or ended. */
static int loft_web_read_frame(loft_stream *s, int *opcode, loft_text *payload) {
    return loft_ws_read_frame(s, opcode, payload, 256u * 1024 * 1024);
}

/* The upgrade; the head is read a byte at a time, so a frame sent right after the 101 stays
 * in the stream for the first receive. */
static int loft_web_handshake(loft_stream *s, const char *host, const char *port, const char *path) {
    static const char b64[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    uint8_t raw[16];
    char nonce[25];
    loft_web_random(raw, 16);
    size_t o = 0;
    for (int i = 0; i < 16; i += 3) {
        uint32_t v = (uint32_t) raw[i] << 16 | (i + 1 < 16 ? (uint32_t) raw[i + 1] << 8 : 0) | (i + 2 < 16 ? raw[i + 2] : 0);
        nonce[o++] = b64[(v >> 18) & 63];
        nonce[o++] = b64[(v >> 12) & 63];
        nonce[o++] = i + 1 < 16 ? b64[(v >> 6) & 63] : '=';
        nonce[o++] = i + 2 < 16 ? b64[v & 63] : '=';
    }
    nonce[o] = 0;
    char req[1024];
    int rn = snprintf(req, sizeof req,
                      "GET %s HTTP/1.1\r\nHost: %s:%s\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                      "Sec-WebSocket-Key: %s\r\nSec-WebSocket-Version: 13\r\n\r\n",
                      path, host, port, nonce);
    if (rn <= 0 || (size_t) rn >= sizeof req || !loft_stream_write_all(s, req, (size_t) rn)) return 0;
    loft_text head = { 0, 0, 0 };
    int ok = loft_http_head(s, &head);
    if (ok) {
        ok = head.len > 12 && (strncmp(head.p, "HTTP/1.1 101", 12) == 0 || strncmp(head.p, "HTTP/1.0 101", 12) == 0);
        const char *v;
        size_t vn;
        char want[29];
        loft_ws_accept_key(nonce, strlen(nonce), want);
        ok = ok && loft_http_header(&head, "sec-websocket-accept", &v, &vn) && vn == strlen(want) && memcmp(v, want, vn) == 0;
    }
    free(head.p);
    return ok;
}

/* Connect and upgrade, trying every address the host resolves to; a failed TLS handshake refuses
 * that address, never falling back to plaintext.  1 with the stream live. */
static int loft_web_open(const char *url, loft_stream *out) {
    int tls;
    char host[512], port[16], path[2048];
    out->fd = -1;
    out->tls = 0;
    if (!loft_web_parse(url, &tls, host, sizeof host, port, path, sizeof path)) return 0;
    struct addrinfo hints, *res = 0;
    memset(&hints, 0, sizeof hints);
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port, &hints, &res) != 0) return 0;
    int live = 0;
    for (struct addrinfo *ai = res; ai && !live; ai = ai->ai_next) {
        int fd = socket(ai->ai_family, SOCK_STREAM, 0);
        if (fd < 0) continue;
        loft_sock_cloexec(fd);
        if (!loft_sock_connect_within(fd, ai->ai_addr, (socklen_t) ai->ai_addrlen, 2000)) {
            close(fd);
            continue;
        }
        /* A peer that accepts but never answers the upgrade fails the attempt within 2 s. */
        loft_sock_timeout(fd, 2000);
        loft_stream st = { fd, 0 };
        char why[768];
        if ((tls && !loft_stream_tls(&st, host, why, sizeof why)) || !loft_web_handshake(&st, host, port, path)) {
            loft_stream_close(&st);
            continue;
        }
        *out = st;
        live = 1;
    }
    freeaddrinfo(res);
    return live;
}

static loft_web_conn *loft_web_get(int64_t h) {
    if (h < 0 || (uint64_t) h >= loft_web_nconns || !loft_web_conns[h].url) return 0;
    return &loft_web_conns[h];
}

static void loft_web_down(loft_web_conn *c) {
    if (c->st.fd >= 0) loft_stream_close(&c->st);
    c->last_attempt_ms = loft_web_now_ms();
    if (c->backoff < LOFT_WEB_BACKOFFS - 1) c->backoff++;
}

/* Live, or reconnected once the backoff since the last attempt has passed. */
static int loft_web_ensure(loft_web_conn *c) {
    if (c->st.fd >= 0) return 1;
    if (loft_web_now_ms() - c->last_attempt_ms < loft_web_backoff_ms[c->backoff]) return 0;
    c->last_attempt_ms = loft_web_now_ms();
    if (loft_web_open(c->url, &c->st)) {
        c->backoff = 0;
        return 1;
    }
    if (c->backoff < LOFT_WEB_BACKOFFS - 1) c->backoff++;
    return 0;
}

/* A session's handle, or -1 for a URL that is not `ws://` / `wss://`.  A first attempt that
 * fails still makes the session; the next send or receive retries. */
static int64_t loft_web_ws_connect(loft_str url) {
    char *u = loft_cstr_own(url);
    int tls;
    char host[512], port[16], path[2048];
    if (!loft_web_parse(u, &tls, host, sizeof host, port, path, sizeof path)) {
        free(u);
        return -1;
    }
    loft_web_conns = (loft_web_conn *) realloc(loft_web_conns, (loft_web_nconns + 1) * sizeof *loft_web_conns);
    if (!loft_web_conns) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_web_conn *c = &loft_web_conns[loft_web_nconns];
    c->url = u;
    c->backoff = 0;
    loft_web_open(u, &c->st);
    c->last_attempt_ms = loft_web_now_ms();
    return (int64_t) loft_web_nconns++;
}

static uint8_t loft_web_send(int64_t h, int opcode, loft_str msg) {
    loft_web_conn *c = loft_web_get(h);
    if (!c || !loft_web_ensure(c)) return 0;
    if (loft_web_write_frame(&c->st, opcode, msg.p, msg.len)) return 1;
    loft_web_down(c);
    return 0;
}
static uint8_t loft_web_ws_client_send(int64_t h, loft_str msg) { return loft_web_send(h, 0x1, msg); }
static uint8_t loft_web_ws_client_send_binary(int64_t h, loft_str msg) { return loft_web_send(h, 0x2, msg); }

/* Is a frame readable within `ms`: decrypted data the TLS session holds, else the socket. */
static int loft_web_readable(loft_web_conn *c, int ms) {
#ifdef LOFT_HTTPS
    if (c->st.tls && loft_tls_pending((struct loft_tls *) c->st.tls)) return 1;
#endif
    struct pollfd p = { c->st.fd, POLLIN, 0 };
    return poll(&p, 1, ms) > 0;
}

/* One frame off a live session: a text or binary frame is the answer (kept for
 * `ws_client_message`), a ping is answered, a close or a failure drops the connection. */
static int loft_web_take(loft_web_conn *c, int ms) {
    if (!loft_web_readable(c, ms)) return 0;
    int op = 0;
    loft_text payload = { 0, 0, 0 };
    int got = loft_web_read_frame(&c->st, &op, &payload);
    int answer = 0;
    if (!got || op == 0x8) {
        loft_web_down(c);
    } else if (op == 0x1 || op == 0x2) {
        loft_web_msg.len = 0;
        if (op == 0x1) loft_push_lossy(&loft_web_msg, payload.p, payload.len);
        else loft_web_set(&loft_web_msg, payload.p, payload.len);
        loft_web_op = op;
        answer = 1;
    } else if (op == 0x9) {
        loft_web_write_frame(&c->st, 0xA, payload.p, payload.len);
    }
    free(payload.p);
    return answer;
}

/* Is a message ready (it is then `ws_client_message`)?  Waits a few ms for one. */
static uint8_t loft_web_ws_client_recv(int64_t h) {
    loft_web_conn *c = loft_web_get(h);
    return c && loft_web_ensure(c) && loft_web_take(c, 7);
}

static loft_text loft_web_ws_client_message(void) { return loft_web_copy(&loft_web_msg); }
static int64_t loft_web_ws_client_opcode(void) { return loft_web_op; }

/* Closed for good: a close frame is sent and the handle never reconnects. */
static void loft_web_ws_client_close(int64_t h) {
    loft_web_conn *c = loft_web_get(h);
    if (!c) return;
    if (c->st.fd >= 0) {
        loft_web_write_frame(&c->st, 0x8, "", 0);
        loft_stream_close(&c->st);
    }
    free(c->url);
    c->url = 0;
}

static void loft_web_ws_group_clear(void) {
    loft_web_ngroup = 0;
    loft_web_group_offset = 0;
}
static void loft_web_ws_group_add(int64_t h) {
    if (loft_web_ngroup == loft_web_group_cap) {
        loft_web_group_cap = loft_web_group_cap ? 2 * loft_web_group_cap : 8;
        loft_web_group = (int64_t *) realloc(loft_web_group, loft_web_group_cap * sizeof *loft_web_group);
        if (!loft_web_group) { fputs("loft: out of memory\n", stderr); abort(); }
    }
    loft_web_group[loft_web_ngroup++] = h;
}

/* The first grouped handle, round from where the last poll left off, with a message ready, or
 * -1; each socket is probed without waiting. */
static int64_t loft_web_ws_group_poll(void) {
    size_t n = loft_web_ngroup;
    for (size_t i = 0; i < n; i++) {
        size_t at = (loft_web_group_offset + i) % n;
        loft_web_conn *c = loft_web_get(loft_web_group[at]);
        if (c && loft_web_ensure(c) && loft_web_take(c, 0)) {
            loft_web_group_offset = (at + 1) % n;
            return loft_web_group[at];
        }
    }
    return -1;
}
#endif
#endif
