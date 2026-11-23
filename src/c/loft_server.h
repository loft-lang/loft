/* ── The server library's natives, compiled into the `net` component (`src/c/components/net.c`) ──
 * An HTTP listener and a WebSocket server over the runtime's connection (`loft_stream`, TLS by
 * `tls/loft_tls.c`'s server session for a TLS listener).  The model is the library's: loft
 * drives the loop, the native does the socket work.  A listener is a handle; `tcp_accept`
 * parses one request and parks its connection, which the next `tcp_respond*` answers and
 * closes, or `ws_upgrade` turns into a WebSocket client.  The event pump (`ws_next_event`) does
 * both on one port without blocking: a connection that asks to upgrade becomes a client, any
 * other request is parked for the handler to answer.  A TLS listener takes a PEM certificate
 * chain and key and completes each handshake as it accepts; a certificate it cannot use
 * refuses the listener, never a fallback to cleartext.  Per-thread state; the sockets are
 * `loft_sock.h`'s (POSIX or Winsock); never
 * pasted into a unit (`LOFT_SERVER` is the component's).  Guard: `tests/server_natives.rs`. */
#if defined(LOFT_SERVER) && defined(LOFT_SOCKETS)

#ifdef LOFT_HTTPS
struct loft_tls_server;
/* Defined in the TLS archive (`tls.rs`), compiled apart from the unit: no store context. */
struct loft_tls_server *loft_tls_server_new(const char *cert, size_t cert_len, const char *key, size_t key_len,
                                            char *err, size_t cap);
void loft_tls_server_free(struct loft_tls_server *c);
struct loft_tls *loft_tls_accept(int fd, const struct loft_tls_server *cfg, char *err, size_t cap);
int loft_tls_poll(struct loft_tls *t, int ms);
#endif

/* The longest frame a client may send; a longer one drops that client. */
#define LOFT_SRV_MAX_FRAME (16u * 1024 * 1024)
/* The largest request body taken: a `Content-Length` is the peer's word, and sizes an allocation
 * before a byte of the body has arrived. */
#define LOFT_SRV_MAX_BODY (16u * 1024 * 1024)
/* The longest request head read before the connection is dropped. */
#define LOFT_SRV_MAX_HEAD (16 * 1024)
/* A pump client's frame read, once its header is here: a plain socket's bound, and a TLS
 * session's (a socket timeout ends a TLS session, so it waits for the record instead). */
#define LOFT_SRV_CLIENT_READ_MS 20
#define LOFT_SRV_CLIENT_READ_TLS_MS 2000
/* A request parked by the non-blocking accept is read within this. */
#define LOFT_SRV_REQUEST_READ_MS 500
/* A whole request, head and body, where the server may not wait: the read timeout bounds one
 * read, and a peer that sends a byte per read would otherwise hold a single-threaded server for
 * as long as it likes. */
#define LOFT_SRV_REQUEST_TOTAL_MS 5000
/* One write to a peer: a client that stops reading fills its window, and a send with no bound
 * then never returns. */
#define LOFT_SRV_SEND_MS 5000

typedef struct loft_srv_listener {
    int fd; /* -1: a closed slot */
    void *tls; /* a `struct loft_tls_server *` for a TLS listener */
    /* The origins whose pages may open a WebSocket here, one per line; 0 or "": any.  A browser
     * names the page's origin on every upgrade, whatever site the page is of; a client that is
     * not a browser sends none and is not asked. */
    char *origins;
} loft_srv_listener;

static loft_text loft_srv_copy(loft_cx *cx, const loft_text *t) { return loft_text_own(cx, loft_lit(cx, t->p ? t->p : "", t->len)); }

static void loft_srv_set(loft_cx *cx, loft_text *t, const char *p, size_t n) {
    t->len = 0;
    if (n) loft_push(cx, t, p, n);
}

/* ── Requests ── */

/* The value of header `name` in `Key: Value` lines (`\n`-separated, a trailing `\r` ignored),
 * any case, trimmed: 1 with it, else 0. */
static int loft_srv_header(loft_cx *cx, const loft_text *heads, const char *name, const char **v, size_t *vn) {
    size_t nl = strlen(name);
    const char *p = heads->p ? heads->p : "";
    const char *end = p + heads->len;
    while (p < end) {
        const char *eol = memchr(p, '\n', (size_t) (end - p));
        const char *le = eol ? eol : end;
        const char *e = le;
        if (e > p && e[-1] == '\r') e--;
        const char *colon = memchr(p, ':', (size_t) (e - p));
        if (colon) {
            const char *k = p, *ke = colon;
            while (k < ke && isspace((unsigned char) *k)) k++;
            while (ke > k && isspace((unsigned char) ke[-1])) ke--;
            if ((size_t) (ke - k) == nl && strncasecmp(k, name, nl) == 0) {
                const char *s = colon + 1;
                while (s < e && isspace((unsigned char) *s)) s++;
                while (e > s && isspace((unsigned char) e[-1])) e--;
                *v = s;
                *vn = (size_t) (e - s);
                return 1;
            }
        }
        p = eol ? eol + 1 : end;
    }
    return 0;
}

/* The request on `s` into the request state: its line, its `Key: Value` header lines (`\n` each)
 * and a `Content-Length` body.  The head is read a byte at a time, so a WebSocket frame a client
 * sends right behind its upgrade request stays in the stream for the first frame read.  0 when
 * the stream ends, the head passes 16 KiB, the request line is not two words, or the request
 * takes longer than `budget_ms` to arrive (0: no bound, a caller that chose to wait). */
/* Why the last request was not taken, as the status the peer is told before the connection
 * closes: 413 a body above the limit, 408 a request that outran its budget, 0 anything else
 * (a peer that went away or sent no request gets no answer). */

/* Tell the peer why (`cx->srv_refused`), when there is a why. */
static void loft_srv_say_refusal(loft_cx *cx, loft_stream *s) {
    const char *line = cx->srv_refused == 413 ? "413 Payload Too Large" : cx->srv_refused == 408 ? "408 Request Timeout" : 0;
    if (!line) return;
    char resp[96];
    int n = snprintf(resp, sizeof resp, "HTTP/1.1 %s\r\nContent-Length: 0\r\nConnection: close\r\n\r\n", line);
    (void) loft_stream_write_all(cx, s, resp, (size_t) n);
}

static int loft_srv_parse_request(loft_cx *cx, loft_stream *s, loft_word budget_ms) {
    loft_text raw = { 0, 0, 0 };
    loft_word late = budget_ms ? loft_now_us(cx) + budget_ms * 1000 : 0;
    cx->srv_refused = 0;
    while (raw.len < 4 || memcmp(raw.p + raw.len - 4, "\r\n\r\n", 4) != 0) {
        char b;
        int slow = late && loft_now_us(cx) > late;
        if (slow || loft_stream_read(cx, s, &b, 1) != 1 || raw.len > LOFT_SRV_MAX_HEAD) {
            if (slow) cx->srv_refused = 408;
            free(raw.p);
            return 0;
        }
        loft_push(cx, &raw, &b, 1);
    }
    loft_text head = { 0, 0, 0 };
    loft_push_lossy(cx, &head, raw.p, raw.len);
    free(raw.p);
    const char *p = head.p, *end = head.p + head.len;
    const char *eol = p;
    while (eol + 1 < end && !(eol[0] == '\r' && eol[1] == '\n')) eol++;
    /* The request line's first two words. */
    const char *w[2];
    size_t wn[2];
    int words = 0;
    for (const char *c = p; c < eol && words < 2;) {
        while (c < eol && isspace((unsigned char) *c)) c++;
        if (c >= eol) break;
        const char *st = c;
        while (c < eol && !isspace((unsigned char) *c)) c++;
        w[words] = st;
        wn[words] = (size_t) (c - st);
        words++;
    }
    if (words < 2) {
        free(head.p);
        return 0;
    }
    loft_text heads = { 0, 0, 0 };
    uint64_t length = 0;
    for (const char *l = eol + 2; l < end;) {
        const char *le = l;
        while (le + 1 < end && !(le[0] == '\r' && le[1] == '\n')) le++;
        if (le + 1 >= end) le = end;
        if (le > l) {
            loft_push(cx, &heads, l, (size_t) (le - l));
            loft_push(cx, &heads, "\n", 1);
        }
        l = le + 2;
    }
    const char *v;
    size_t vn;
    if (loft_srv_header(cx, &heads, "content-length", &v, &vn)) {
        size_t i = (vn && v[0] == '+') ? 1 : 0;
        int ok = i < vn;
        uint64_t n = 0;
        for (; i < vn && ok; i++) {
            if (v[i] < '0' || v[i] > '9' || n > (UINT64_MAX - 9) / 10) ok = 0;
            else n = n * 10 + (uint64_t) (v[i] - '0');
        }
        length = ok ? n : 0;
    }
    char *body = length > LOFT_SRV_MAX_BODY ? NULL : (char *) malloc(length ? (size_t) length : 1);
    /* Too large, or no memory for it: this request is dropped, the server goes on. */
    if (!body) {
        cx->srv_refused = 413;
        free(head.p);
        free(heads.p);
        return 0;
    }
    for (size_t got = 0; got < (size_t) length;) {
        int slow = late && loft_now_us(cx) > late;
        long r = slow ? 0 : loft_stream_read(cx, s, body + got, (size_t) length - got);
        if (r <= 0) {
            if (slow) cx->srv_refused = 408;
            free(body);
            free(head.p);
            free(heads.p);
            return 0;
        }
        got += (size_t) r;
    }
    loft_srv_set(cx, &cx->srv_method, w[0], wn[0]);
    loft_srv_set(cx, &cx->srv_path, w[1], wn[1]);
    loft_srv_set(cx, &cx->srv_heads, heads.p, heads.len);
    cx->srv_body.len = 0;
    loft_push_lossy(cx, &cx->srv_body, body, (size_t) length);
    free(body);
    free(head.p);
    free(heads.p);
    return 1;
}

/* Answer an upgrade request (headers `heads`) with the 101. */
static int loft_srv_upgrade(loft_cx *cx, loft_stream *s, const loft_text *heads) {
    const char *k;
    size_t kn;
    if (!loft_srv_header(cx, heads, "sec-websocket-key", &k, &kn)) return 0;
    char accept[29];
    loft_ws_accept_key(cx, k, kn, accept);
    char resp[256];
    int n = snprintf(resp, sizeof resp,
                     "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                     "Sec-WebSocket-Accept: %s\r\n\r\n",
                     accept);
    return loft_stream_write_all(cx, s, resp, (size_t) n);
}

/* ── Listening and accepting ── */

static loft_srv_listener *loft_srv_listener_at(loft_cx *cx, int64_t h) {
    if (h < 0 || (uint64_t) h >= cx->srv_nls || cx->srv_ls[h].fd < 0) return 0;
    return &cx->srv_ls[h];
}

/* Bind `addr:port`: the handle, or -1 with the reason on stderr.  A bind that works says nothing
 * unless `LOFT_NET_LISTEN_BANNER` asks for it.  Takes `tls` (freed on a failure). */
static int64_t loft_srv_bind(loft_cx *cx, const char *addr, size_t an, int64_t port, void *tls) {
    char host[256];
    if (an == 0) {
        snprintf(host, sizeof host, "0.0.0.0");
    } else {
        snprintf(host, sizeof host, "%.*s", (int) (an < sizeof host - 1 ? an : sizeof host - 1), addr);
    }
    char at[300], service[16];
    snprintf(service, sizeof service, "%u", (unsigned) (uint32_t) port);
    snprintf(at, sizeof at, "%s:%s", host, service);
    struct addrinfo hints, *res = 0;
    memset(&hints, 0, sizeof hints);
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    hints.ai_flags = AI_PASSIVE | AI_NUMERICSERV;
    loft_sock_ready(cx);
    int gai = getaddrinfo(host, service, &hints, &res);
    int fd = -1, why = 0;
    if (gai != 0) {
        fprintf(stderr, "loft_tcp_listen: cannot bind %s: %s\n", at, gai_strerror(gai));
    } else {
        for (struct addrinfo *a = res; a && fd < 0; a = a->ai_next) {
            int s = loft_sock_open(cx, a->ai_family, a->ai_socktype, a->ai_protocol);
            if (s < 0) {
                why = loft_sock_error(cx);
                continue;
            }
            loft_sock_cloexec(cx, s);
            loft_sock_listener_addr(cx, s);
            if (loft_sock_bind_listen(cx, s, a->ai_addr, (size_t) a->ai_addrlen, 128) == 0) {
                fd = s;
            } else {
                why = loft_sock_error(cx);
                loft_sock_close(cx, s);
            }
        }
        freeaddrinfo(res);
        if (fd < 0) fprintf(stderr, "loft_tcp_listen: cannot bind %s: %s (os error %d)\n", at, strerror(why), why);
    }
    if (fd < 0) {
#ifdef LOFT_HTTPS
        loft_tls_server_free((struct loft_tls_server *) tls);
#endif
        return -1;
    }
    if (getenv("LOFT_NET_LISTEN_BANNER"))
        fprintf(stderr, "loft server listening on %s (%s)\n", at, tls ? "https/wss" : "http/ws");
    loft_srv_listener *grown = (loft_srv_listener *) realloc(cx->srv_ls, (cx->srv_nls + 1) * sizeof *grown);
    if (!grown) { fputs("loft: out of memory\n", stderr); abort(); }
    cx->srv_ls = grown;
    cx->srv_ls[cx->srv_nls].fd = fd;
    cx->srv_ls[cx->srv_nls].tls = tls;
    cx->srv_ls[cx->srv_nls].origins = 0;
    return (int64_t) cx->srv_nls++;
}

static int64_t loft_srv_listen_tls(loft_cx *cx, const char *addr, size_t an, int64_t port, const char *cert, size_t cn,
                                   const char *key, size_t kn) {
#ifdef LOFT_HTTPS
    char err[256] = "";
    struct loft_tls_server *cfg = loft_tls_server_new(cert, cn, key, kn, err, sizeof err);
    if (!cfg) {
        fprintf(stderr, "loft_tcp_listen_tls: %s\n", err);
        return -1;
    }
    return loft_srv_bind(cx, addr, an, port, cfg);
#else
    (void) addr, (void) an, (void) port, (void) cert, (void) cn, (void) key, (void) kn;
    fprintf(stderr, "loft_tcp_listen_tls: this build has no TLS stack\n");
    return -1;
#endif
}

/* The next connection on listener `h`, its TLS handshake done: 1 with it in `out`, 0 when
 * nothing is pending (or the handle is not a listener), -1 when the accept or the handshake
 * failed. */
static int loft_srv_accept(loft_cx *cx, int64_t h, int nonblocking, loft_stream *out) {
    loft_srv_listener *l = loft_srv_listener_at(cx, h);
    if (!l) return 0;
    loft_sock_blocking(cx, l->fd, !nonblocking);
    int fd;
    for (;;) {
        fd = loft_sock_accept(cx, l->fd);
        if (fd >= 0) break;
        if (loft_sock_interrupted(cx)) continue;
        return loft_sock_would_block(cx) ? 0 : -1;
    }
    loft_sock_cloexec(cx, fd);
    loft_sock_blocking(cx, fd, 1);
    loft_sock_timeout_opt(cx, fd, SO_SNDTIMEO, LOFT_SRV_SEND_MS);
    out->fd = fd;
    out->tls = 0;
#ifdef LOFT_HTTPS
    if (l->tls) {
        /* The handshake completes here, so a failure is this connection's, not a broken stream
         * later. */
        char err[256];
        loft_sock_timeout(cx, fd, 10000);
        out->tls = loft_tls_accept(fd, (const struct loft_tls_server *) l->tls, err, sizeof err);
        if (!out->tls) {
            loft_sock_close(cx, fd);
            out->fd = -1;
            return -1;
        }
        loft_sock_timeout(cx, fd, 0);
    }
#endif
    return 1;
}

static void loft_srv_park(loft_cx *cx, loft_stream conn) {
    if (cx->srv_parked.fd >= 0) loft_stream_close(cx, &cx->srv_parked);
    cx->srv_parked = conn;
}

/* Accept and parse one request, parking its connection for the response. */
static uint8_t loft_srv_tcp_accept(loft_cx *cx, int64_t h, int nonblocking) {
    loft_stream conn = { -1, 0 };
    if (loft_srv_accept(cx, h, nonblocking, &conn) != 1) return 0;
    if (nonblocking) loft_sock_timeout(cx, conn.fd, LOFT_SRV_REQUEST_READ_MS);
    if (!loft_srv_parse_request(cx, &conn, nonblocking ? LOFT_SRV_REQUEST_TOTAL_MS : 0)) {
        loft_srv_say_refusal(cx, &conn);
        loft_stream_close(cx, &conn);
        return 0;
    }
    loft_srv_park(cx, conn);
    return 1;
}

/* `ws_allow_origins`: the listener's allowed origins become `list` (one per line; empty: any). */
static void loft_srv_allow_origins(loft_cx *cx, loft_word h, loft_str list) {
    loft_srv_listener *l = loft_srv_listener_at(cx, h);
    if (!l) return;
    free(l->origins);
    l->origins = loft_cstr_own(cx, list);
}

/* Whether a request with these headers may upgrade on `l`: it names no origin, the listener
 * lists none, or the origin is a line of the list (compared whole, case-insensitively). */
static int loft_srv_origin_allowed(loft_cx *cx, const loft_srv_listener *l, const loft_text *heads) {
    const char *v;
    size_t vn;
    if (!l || !l->origins || !*l->origins || !loft_srv_header(cx, heads, "origin", &v, &vn)) return 1;
    for (const char *p = l->origins; *p;) {
        const char *e = strchr(p, '\n');
        size_t n = e ? (size_t) (e - p) : strlen(p);
        while (n && (p[n - 1] == '\r' || p[n - 1] == ' ')) n--;
        if (n == vn && strncasecmp(p, v, n) == 0) return 1;
        if (!e) break;
        p = e + 1;
    }
    return 0;
}

static void loft_srv_tcp_close(loft_cx *cx, int64_t h) {
    loft_srv_listener *l = loft_srv_listener_at(cx, h);
    if (!l) return;
    free(l->origins);
    l->origins = 0;
    loft_sock_close(cx, l->fd);
    l->fd = -1;
#ifdef LOFT_HTTPS
    loft_tls_server_free((struct loft_tls_server *) l->tls);
#endif
    l->tls = 0;
}

/* ── Responses ── */

static const char *loft_srv_status_text(loft_cx *cx, int64_t status) {
    switch (status) {
    case 200: return "OK";
    case 201: return "Created";
    case 204: return "No Content";
    case 206: return "Partial Content";
    case 304: return "Not Modified";
    case 400: return "Bad Request";
    case 404: return "Not Found";
    case 405: return "Method Not Allowed";
    case 416: return "Range Not Satisfiable";
    case 500: return "Internal Server Error";
    default: return "Unknown";
    }
}

/* Write the whole response on the parked connection and close it (a TLS session with its
 * close_notify, so the client reads a clean end and not a truncation). */
static void loft_srv_send_and_close(loft_cx *cx, const char *p, size_t n) {
    if (cx->srv_parked.fd < 0) return;
    loft_stream_write_all(cx, &cx->srv_parked, p, n);
    loft_stream_close(cx, &cx->srv_parked);
    cx->srv_parked.fd = -1;
    cx->srv_parked.tls = 0;
}

/* A response with the library's fixed headers: no caching, the connection closed. */
/* The length of `p[0..n)` up to its first control character (a tab excepted): what of a header
 * value may be written.  A CR or LF in it would end the header there and start another the
 * caller never wrote — a value echoed from a request could add headers, or a body. */
static size_t loft_srv_header_safe(loft_cx *cx, const char *p, size_t n) {
    size_t i = 0;
    while (i < n && ((unsigned char) p[i] >= 0x20 || p[i] == '\t') && (unsigned char) p[i] != 0x7f) i++;
    return i;
}

static void loft_srv_respond(loft_cx *cx, int64_t status, const char *ct, size_t ctn, const char *body, size_t bn) {
    status = (int64_t) (uint16_t) status;
    ctn = loft_srv_header_safe(cx, ct, ctn);
    char head[1024];
    int hn = snprintf(head, sizeof head,
                      "HTTP/1.1 %d %s\r\nContent-Length: %zu\r\nContent-Type: %.*s\r\n"
                      "Cache-Control: no-store, no-cache, must-revalidate, max-age=0\r\nPragma: no-cache\r\n"
                      "Expires: 0\r\nConnection: close\r\n\r\n",
                      (int) status, loft_srv_status_text(cx, status), bn, (int) (ctn < 256 ? ctn : 256), ct);
    loft_text out = { 0, 0, 0 };
    loft_push(cx, &out, head, (size_t) hn);
    if (bn) loft_push(cx, &out, body, bn);
    loft_srv_send_and_close(cx, out.p, out.len);
    free(out.p);
}

/* A response with the caller's headers (`Key: Value` lines): `Content-Length` is added from the
 * body unless the caller gave one (a HEAD reply), `Connection: close` always.  A line holding a
 * control character (a CR, a NUL; a tab is allowed) is left out whole: written, it would end
 * the header inside the line and begin one the caller did not write. */
static void loft_srv_respond_full(loft_cx *cx, int64_t status, loft_str body, loft_str heads) {
    status = (int64_t) (uint16_t) status;
    loft_text out = { 0, 0, 0 };
    char line[64];
    int n = snprintf(line, sizeof line, "HTTP/1.1 %d %s\r\n", (int) status, loft_srv_status_text(cx, status));
    loft_push(cx, &out, line, (size_t) n);
    int has_len = 0;
    const char *p = heads.p ? heads.p : "", *end = p + heads.len;
    while (p < end) {
        const char *eol = memchr(p, '\n', (size_t) (end - p));
        const char *s = p, *e = eol ? eol : end;
        while (s < e && isspace((unsigned char) *s)) s++;
        while (e > s && isspace((unsigned char) e[-1])) e--;
        if (e > s && loft_srv_header_safe(cx, s, (size_t) (e - s)) == (size_t) (e - s)) {
            if ((size_t) (e - s) >= 15 && strncasecmp(s, "content-length:", 15) == 0) has_len = 1;
            loft_push(cx, &out, s, (size_t) (e - s));
            loft_push(cx, &out, "\r\n", 2);
        }
        p = eol ? eol + 1 : end;
    }
    if (!has_len) {
        n = snprintf(line, sizeof line, "Content-Length: %zu\r\n", body.len);
        loft_push(cx, &out, line, (size_t) n);
    }
    loft_push(cx, &out, "Connection: close\r\n\r\n", 21);
    if (body.len) loft_push(cx, &out, body.p, body.len);
    loft_srv_send_and_close(cx, out.p, out.len);
    free(out.p);
}

/* Bytes `[off, off+len)` of `body`, clamped to it — by byte, so a binary body is cut where
 * asked and not re-encoded. */
static loft_text loft_srv_slice(loft_cx *cx, loft_str body, int64_t off, int64_t len) {
    size_t n = body.len;
    size_t start = off <= 0 ? 0 : ((uint64_t) off > n ? n : (size_t) off);
    size_t take = len <= 0 ? 0 : (uint64_t) len;
    size_t end = take > n - start ? n : start + take;
    loft_text t = { 0, 0, 0 };
    if (end > start) loft_push(cx, &t, body.p + start, end - start);
    return t;
}

/* ── WebSockets ── */

/* A client in the first free slot: its id. */
static int64_t loft_srv_add_client(loft_cx *cx, loft_stream conn) {
    for (size_t i = 0; i < cx->srv_nclients; i++) {
        if (cx->srv_clients[i].fd < 0) {
            cx->srv_clients[i] = conn;
            return (int64_t) i;
        }
    }
    loft_stream *grown = (loft_stream *) realloc(cx->srv_clients, (cx->srv_nclients + 1) * sizeof *grown);
    if (!grown) { fputs("loft: out of memory\n", stderr); abort(); }
    cx->srv_clients = grown;
    cx->srv_clients[cx->srv_nclients] = conn;
    return (int64_t) cx->srv_nclients++;
}

static loft_stream *loft_srv_client(loft_cx *cx, int64_t id) {
    if (id < 0 || (uint64_t) id >= cx->srv_nclients || cx->srv_clients[id].fd < 0) return 0;
    return &cx->srv_clients[id];
}

/* The parked connection upgraded into a client: its id, or -1. */
static int64_t loft_srv_ws_upgrade(loft_cx *cx) {
    if (cx->srv_parked.fd < 0) return -1;
    loft_stream conn = cx->srv_parked;
    cx->srv_parked.fd = -1;
    cx->srv_parked.tls = 0;
    if (!loft_srv_upgrade(cx, &conn, &cx->srv_heads)) {
        loft_stream_close(cx, &conn);
        return -1;
    }
    return loft_srv_add_client(cx, conn);
}

/* Read client `id`'s next message, blocking: 1 with it surfaced, 0 on a close or an error.  A
 * ping is answered and the read goes on: reporting it as a message would hand the library's
 * `next` the previous message a second time. */
static uint8_t loft_srv_ws_recv(loft_cx *cx, int64_t id) {
    loft_stream *c = loft_srv_client(cx, id);
    if (!c) return 0;
    loft_text p = { 0, 0, 0 };
    for (;;) {
        int op;
        if (!loft_ws_read_frame(cx, c, &op, &p, LOFT_SRV_MAX_FRAME, 1) || op == 0x8) {
            free(p.p);
            return 0;
        }
        if (op == 0x9) {
            loft_ws_write_frame(cx, c, 0xA, p.p, p.len, 0);
            continue;
        }
        if (op == 0xA) continue;
        cx->srv_msg.len = 0;
        loft_push_lossy(cx, &cx->srv_msg, p.p ? p.p : "", p.len);
        cx->srv_msg_op = op;
        free(p.p);
        return 1;
    }
}

static uint8_t loft_srv_ws_send(loft_cx *cx, int64_t id, int opcode, loft_str msg) {
    loft_stream *c = loft_srv_client(cx, id);
    return c && loft_ws_write_frame(cx, c, opcode, msg.p, msg.len, 0);
}

/* Send a close frame and drop the client. */
static void loft_srv_ws_close(loft_cx *cx, int64_t id) {
    loft_stream *c = loft_srv_client(cx, id);
    if (!c) return;
    loft_ws_write_frame(cx, c, 0x8, "", 0, 0);
    loft_stream_close(cx, c);
    c->fd = -1;
    c->tls = 0;
}

/* The pump's accept: a new client's id (>= 0), -1 nothing pending, -2 a connection that went
 * wrong, -3 a plain request parked for the handler. */
static int64_t loft_srv_accept_client(loft_cx *cx, int64_t h) {
    loft_stream conn = { -1, 0 };
    int got = loft_srv_accept(cx, h, 1, &conn);
    if (got == 0) return -1;
    if (got < 0) return -2;
    /* The pump serves every client from this one thread: a peer that connects and says nothing
     * must cost it one read timeout, not the server. */
    loft_sock_timeout(cx, conn.fd, LOFT_SRV_REQUEST_READ_MS);
    if (!loft_srv_parse_request(cx, &conn, LOFT_SRV_REQUEST_TOTAL_MS)) {
        loft_srv_say_refusal(cx, &conn);
        loft_stream_close(cx, &conn);
        return -2;
    }
    const char *v;
    size_t vn;
    if (!loft_srv_header(cx, &cx->srv_heads, "sec-websocket-key", &v, &vn)) {
        loft_srv_park(cx, conn);
        return -3;
    }
    if (!loft_srv_origin_allowed(cx, loft_srv_listener_at(cx, h), &cx->srv_heads)) {
        static const char no[] = "HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
        (void) loft_stream_write_all(cx, &conn, no, sizeof no - 1);
        loft_stream_close(cx, &conn);
        return -2;
    }
    if (!loft_srv_upgrade(cx, &conn, &cx->srv_heads)) {
        loft_stream_close(cx, &conn);
        return -2;
    }
    loft_sock_timeout(cx, conn.fd, conn.tls ? LOFT_SRV_CLIENT_READ_TLS_MS : LOFT_SRV_CLIENT_READ_MS);
    return loft_srv_add_client(cx, conn);
}

static int64_t loft_srv_ws_accept_nonblocking(loft_cx *cx, int64_t h) {
    int64_t r = loft_srv_accept_client(cx, h);
    /* The WebSocket-only entry has no use for a plain request: it is reported as a failure. */
    return r == -3 ? -2 : r;
}

/* Is a frame waiting on client `c`, without consuming it: 1 ready, 0 not yet, -1 the client is
 * gone.  A plain socket is peeked; a TLS session reads records into its engine, where the
 * decrypted data waits for the frame read (`loft_tls_poll`). */
static int loft_srv_probe(loft_cx *cx, loft_stream *c) {
#ifdef LOFT_HTTPS
    if (c->tls) return loft_tls_poll((struct loft_tls *) c->tls, 0);
#endif
    uint8_t hdr[2];
    long r = loft_sock_peek(cx, c->fd, hdr, 2);
    if (r == 0 || r == -2) return -1;
    if (r < 0) return 0;
    return r >= 2 ? 1 : 0;
}

/* What a poll of client `c` found: 0 nothing, 1 a message (into the event payload), 2 gone. */
static int loft_srv_poll_client(loft_cx *cx, loft_stream *c) {
    loft_text p = { 0, 0, 0 };
    for (;;) {
        int ready = loft_srv_probe(cx, c);
        if (ready < 0) {
            free(p.p);
            return 2;
        }
        if (ready == 0) {
            free(p.p);
            return 0;
        }
        int op;
        if (!loft_ws_read_frame(cx, c, &op, &p, LOFT_SRV_MAX_FRAME, 1) || op == 0x8) {
            free(p.p);
            return 2;
        }
        if (op == 0x9) {
            loft_ws_write_frame(cx, c, 0xA, p.p, p.len, 0);
            continue;
        }
        if (op == 0xA) continue;
        cx->srv_ev_payload.len = 0;
        loft_push_lossy(cx, &cx->srv_ev_payload, p.p ? p.p : "", p.len);
        free(p.p);
        return 1;
    }
}

static uint8_t loft_srv_event(loft_cx *cx, int64_t kind, int64_t cid) {
    cx->srv_ev_kind = kind;
    cx->srv_ev_cid = cid;
    return 1;
}

/* At most one event: a new connection (or a parked HTTP request) first, then the first client,
 * in slot order, with a message or gone.  0 connected, 1 message, 2 disconnected, 3 an HTTP
 * request (parked, its path the payload). */
static uint8_t loft_srv_ws_next_event(loft_cx *cx, int64_t h) {
    int64_t got = loft_srv_accept_client(cx, h);
    if (got >= 0) {
        /* A connect carries the upgrade request's headers (`Key: Value` lines), so a program
         * can decide about a client by what it sent — its origin, a cookie, a token. */
        loft_srv_set(cx, &cx->srv_ev_payload, cx->srv_heads.p, cx->srv_heads.len);
        return loft_srv_event(cx, 0, got);
    }
    if (got == -3) {
        loft_srv_set(cx, &cx->srv_ev_payload, cx->srv_path.p, cx->srv_path.len);
        return loft_srv_event(cx, 3, -1);
    }
    for (size_t i = 0; i < cx->srv_nclients; i++) {
        loft_stream *c = &cx->srv_clients[i];
        if (c->fd < 0) continue;
        int r = loft_srv_poll_client(cx, c);
        if (r == 1) return loft_srv_event(cx, 1, (int64_t) i);
        if (r == 2) {
            loft_stream_close(cx, c);
            c->fd = -1;
            c->tls = 0;
            cx->srv_ev_payload.len = 0;
            return loft_srv_event(cx, 2, (int64_t) i);
        }
    }
    return 0;
}

/* A text frame to every client: how many took it. */
static int64_t loft_srv_ws_broadcast(loft_cx *cx, loft_str msg) {
    int64_t sent = 0;
    for (size_t i = 0; i < cx->srv_nclients; i++)
        if (cx->srv_clients[i].fd >= 0 && loft_ws_write_frame(cx, &cx->srv_clients[i], 0x1, msg.p, msg.len, 0)) sent++;
    return sent;
}

static void loft_srv_sleep_ms(loft_cx *cx, int64_t ms) { loft_sock_nap(cx, ms); }

static int64_t loft_srv_clients_len(loft_cx *cx) { return (int64_t) cx->srv_nclients; }
static uint8_t loft_srv_client_active(loft_cx *cx, int64_t id) { return loft_srv_client(cx, id) != 0; }
#endif
