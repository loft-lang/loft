/* ── lib/engine_host: the kernel's mechanics (src/engine_host.rs) ──────────────────────────────
 * The natives behind `lib/engine_host`, which owns no game meaning: the socket pump (non-blocking,
 * a peek before every frame read), the event queue, drift-free ticks, class-routed send and
 * broadcast, and the state-sync channel over UDP — one datagram socket on the listener's port, a
 * per-connection cookie handed over in the 101 response (`X-Loft-UDP`), `H:<cookie>` binding a
 * source address to its connection, `S:<seq>:<payload>` conflating to the newest per sender, kind
 * and (for a keyed kind) entity.  Both roles share one queue and one conflation machinery: the
 * listener (`run`) and the connector (`run_client`, or a transportless local kernel).  The
 * `D!:` debug control channel answers as a build with no live tier does.  `http_fetch` returns at
 * once and completes as a kind-3 event, its request on a worker thread.  POSIX hosts; elsewhere
 * the kernel cannot listen or connect.  Compiled in for a unit that reaches a kernel native
 * (LOFT_ENGINE). */
#ifdef LOFT_ENGINE
#include <ctype.h>
#if defined(LOFT_POSIX_HOST)
#include <arpa/inet.h>
#include <errno.h>
#include <fcntl.h>
#include <netdb.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <strings.h>
#include <sys/socket.h>
#include <sys/time.h>
#include <unistd.h>
#if !defined(LOFT_NO_THREADS) && !defined(LOFT_STRICT)
#include <pthread.h>
#define LOFT_EH_THREADS 1
#endif
#endif

#define LOFT_EH_UDP_TIMEOUT_US 3000000
#define LOFT_EH_UDP_MAX 1200
#define LOFT_EH_HELLO_US 200000
#define LOFT_EH_KEEPALIVE_US 500000
#define LOFT_EH_SLOTS_MAX 256

/* One pumped event: connect (0), message (1), disconnect (2), an HTTP completion (3). */
typedef struct { int64_t cid, kind, status; loft_text payload; } loft_eh_event;
typedef struct { loft_eh_event *v; size_t head, n, cap; } loft_eh_queue;

static void loft_eh_push(loft_eh_queue *q, int64_t cid, int64_t kind, int64_t status, const char *p, size_t n) {
    if (q->head > 0 && q->head == q->n) q->head = q->n = 0;
    if (q->n == q->cap) {
        if (q->head > 0) {
            memmove(q->v, q->v + q->head, (q->n - q->head) * sizeof *q->v);
            q->n -= q->head;
            q->head = 0;
        }
        if (q->n == q->cap) {
            q->cap = q->cap ? q->cap * 2 : 16;
            q->v = (loft_eh_event *) realloc(q->v, q->cap * sizeof *q->v);
            if (!q->v) { fputs("loft: out of memory\n", stderr); abort(); }
        }
    }
    loft_eh_event *e = &q->v[q->n++];
    e->cid = cid;
    e->kind = kind;
    e->status = status;
    e->payload.p = 0;
    e->payload.len = e->payload.cap = 0;
    loft_push(&e->payload, p, n);
}

/* Pop the head into `last` (its old payload freed): 0 when the queue is empty. */
static int loft_eh_pop(loft_eh_queue *q, loft_eh_event *last) {
    if (q->head >= q->n) return 0;
    free(last->payload.p);
    *last = q->v[q->head++];
    return 1;
}

static void loft_eh_queue_free(loft_eh_queue *q) {
    for (size_t i = q->head; i < q->n; i++) free(q->v[i].payload.p);
    free(q->v);
    q->v = 0;
    q->head = q->n = q->cap = 0;
}

/* ── The wire-schema table and conflation (shared by both roles) ── */

/* `msg_id -> keyed?` for the kinds whose messages are latest-value state. */
static int64_t *loft_eh_sync_id = 0;
static uint8_t *loft_eh_sync_keyed = 0;
static size_t loft_eh_nsync = 0;

static void loft_eh_sync_declare(int64_t id, uint8_t keyed) {
    for (size_t i = 0; i < loft_eh_nsync; i++) {
        if (loft_eh_sync_id[i] == id) {
            loft_eh_sync_keyed[i] = keyed;
            return;
        }
    }
    loft_eh_sync_id = (int64_t *) realloc(loft_eh_sync_id, (loft_eh_nsync + 1) * sizeof *loft_eh_sync_id);
    loft_eh_sync_keyed = (uint8_t *) realloc(loft_eh_sync_keyed, loft_eh_nsync + 1);
    if (!loft_eh_sync_id || !loft_eh_sync_keyed) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_eh_sync_id[loft_eh_nsync] = id;
    loft_eh_sync_keyed[loft_eh_nsync++] = keyed;
}

/* `str::parse::<i64>` over `p[0..n]`: 0 when it is not a whole decimal integer. */
static int loft_eh_int(const char *p, size_t n, int64_t *v) {
    if (n == 0 || n > 20) return 0;
    size_t i = 0;
    int neg = 0;
    if (p[0] == '+' || p[0] == '-') {
        neg = p[0] == '-';
        i = 1;
        if (n == 1) return 0;
    }
    uint64_t x = 0;
    for (; i < n; i++) {
        if (p[i] < '0' || p[i] > '9') return 0;
        uint64_t d = (uint64_t) (p[i] - '0');
        if (x > (UINT64_MAX - d) / 10) return 0;
        x = x * 10 + d;
    }
    if (x > (uint64_t) INT64_MAX + (uint64_t) neg) return 0;
    *v = neg ? (int64_t) (0 - x) : (int64_t) x;
    return 1;
}

/* The leading `<digits>:` of a wire message, or -1 when unframed. */
static int64_t loft_eh_msg_id(const char *p, size_t n) {
    const char *c = (const char *) memchr(p, ':', n);
    int64_t id;
    return c && loft_eh_int(p, (size_t) (c - p), &id) ? id : -1;
}

/* -1 when `id` is undeclared, else whether it is keyed. */
static int loft_eh_sync_kind(int64_t id) {
    for (size_t i = 0; i < loft_eh_nsync; i++)
        if (loft_eh_sync_id[i] == id) return loft_eh_sync_keyed[i];
    return -1;
}

static int loft_eh_is_sync(const char *p, size_t n) {
    int64_t id = loft_eh_msg_id(p, n);
    return id >= 0 && loft_eh_sync_kind(id) >= 0;
}

/* One conflation slot: the newest sync message from one peer for one (msg_id, key). */
typedef struct { int64_t msg_id, seq; loft_text key, payload; int dirty; } loft_eh_slot;
typedef struct { loft_eh_slot *v; size_t n; } loft_eh_slots;

static void loft_eh_slots_free(loft_eh_slots *s) {
    for (size_t i = 0; i < s->n; i++) {
        free(s->v[i].key.p);
        free(s->v[i].payload.p);
    }
    free(s->v);
    s->v = 0;
    s->n = 0;
}

/* The slot for this payload's (msg_id, key) — the key is a keyed kind's first comma field —
 * made when new; 0 past the slot cap (sync is loss-tolerant: over-cap drops). */
static loft_eh_slot *loft_eh_slot_for(loft_eh_slots *s, const char *p, size_t n) {
    int64_t id = loft_eh_msg_id(p, n);
    const char *key = "";
    size_t klen = 0;
    if (id >= 0 && loft_eh_sync_kind(id) == 1) {
        const char *c = (const char *) memchr(p, ':', n);
        key = c + 1;
        const char *comma = (const char *) memchr(key, ',', n - (size_t) (key - p));
        klen = comma ? (size_t) (comma - key) : n - (size_t) (key - p);
    }
    for (size_t i = 0; i < s->n; i++)
        if (s->v[i].msg_id == id && s->v[i].key.len == klen && (klen == 0 || memcmp(s->v[i].key.p, key, klen) == 0))
            return &s->v[i];
    if (s->n >= LOFT_EH_SLOTS_MAX) return 0;
    s->v = (loft_eh_slot *) realloc(s->v, (s->n + 1) * sizeof *s->v);
    if (!s->v) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_eh_slot *slot = &s->v[s->n++];
    memset(slot, 0, sizeof *slot);
    slot->msg_id = id;
    slot->seq = -1;
    loft_push(&slot->key, key, klen);
    return slot;
}

/* A sync message over an ordered carrier: arrival order is the seq. */
static void loft_eh_conflate_ws(loft_eh_slots *s, const char *p, size_t n) {
    loft_eh_slot *slot = loft_eh_slot_for(s, p, n);
    if (!slot) return;
    slot->seq += 1;
    slot->payload.len = 0;
    loft_push(&slot->payload, p, n);
    slot->dirty = 1;
}

/* A seq-stamped sync message: a higher seq overwrites, a stale one is dropped. */
static void loft_eh_conflate(loft_eh_slots *s, int64_t seq, const char *p, size_t n) {
    loft_eh_slot *slot = loft_eh_slot_for(s, p, n);
    if (!slot || seq <= slot->seq) return;
    slot->seq = seq;
    slot->payload.len = 0;
    loft_push(&slot->payload, p, n);
    slot->dirty = 1;
}

/* `S:<seq>:<payload>`: 1 and its parts when `p` is one. */
static int loft_eh_seq_frame(const char *p, size_t n, int64_t *seq, const char **body, size_t *blen) {
    if (n < 2 || p[0] != 'S' || p[1] != ':') return 0;
    const char *c = (const char *) memchr(p + 2, ':', n - 2);
    if (!c || !loft_eh_int(p + 2, (size_t) (c - p - 2), seq)) return 0;
    *body = c + 1;
    *blen = n - (size_t) (c + 1 - p);
    return 1;
}

static loft_text loft_eh_copy(const loft_text *t) {
    loft_text out = { 0, 0, 0 };
    loft_push(&out, t->p, t->len);
    return out;
}

/* ── Non-blocking outbound HTTP: the request runs on a worker; its completion is parked here and
 * drained into the pumping host's queue as an ordinary kind-3 event (cid = the request id). ── */

typedef struct { int64_t id, status; loft_text body; } loft_eh_done;
static loft_eh_done *loft_eh_dones = 0;
static size_t loft_eh_ndone = 0;
static int64_t loft_eh_http_next = 1;
#if defined(LOFT_EH_THREADS)
static pthread_mutex_t loft_eh_done_lock = PTHREAD_MUTEX_INITIALIZER;
#endif

static void loft_eh_park(int64_t id, int64_t status, const char *body, size_t n) {
#if defined(LOFT_EH_THREADS)
    pthread_mutex_lock(&loft_eh_done_lock);
#endif
    loft_eh_dones = (loft_eh_done *) realloc(loft_eh_dones, (loft_eh_ndone + 1) * sizeof *loft_eh_dones);
    if (!loft_eh_dones) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_eh_done *d = &loft_eh_dones[loft_eh_ndone++];
    d->id = id;
    d->status = status;
    d->body.p = 0;
    d->body.len = d->body.cap = 0;
    loft_push(&d->body, body, n);
#if defined(LOFT_EH_THREADS)
    pthread_mutex_unlock(&loft_eh_done_lock);
#endif
}

static void loft_eh_drain_http(loft_eh_queue *q) {
#if defined(LOFT_EH_THREADS)
    pthread_mutex_lock(&loft_eh_done_lock);
#endif
    for (size_t i = 0; i < loft_eh_ndone; i++) {
        loft_eh_push(q, loft_eh_dones[i].id, 3, loft_eh_dones[i].status, loft_eh_dones[i].body.p,
                     loft_eh_dones[i].body.len);
        free(loft_eh_dones[i].body.p);
    }
    loft_eh_ndone = 0;
#if defined(LOFT_EH_THREADS)
    pthread_mutex_unlock(&loft_eh_done_lock);
#endif
}

#if defined(LOFT_POSIX_HOST)
/* ── Socket mechanics ── */

/* Every byte of `p`, or 0. */
static int loft_eh_write_all(int fd, const char *p, size_t n) {
    while (n > 0) {
#if defined(MSG_NOSIGNAL)
        ssize_t w = send(fd, p, n, MSG_NOSIGNAL);
#else
        ssize_t w = send(fd, p, n, 0);
#endif
        if (w < 0 && errno == EINTR) continue;
        if (w <= 0) return 0;
        p += w;
        n -= (size_t) w;
    }
    return 1;
}

/* Exactly `n` bytes, within the socket's read timeout, or 0. */
static int loft_eh_read_exact(int fd, void *buf, size_t n) {
    uint8_t *p = (uint8_t *) buf;
    while (n > 0) {
        ssize_t r = recv(fd, p, n, 0);
        if (r < 0 && errno == EINTR) continue;
        if (r <= 0) return 0;
        p += r;
        n -= (size_t) r;
    }
    return 1;
}

/* Read an HTTP head to its blank line, a byte at a time (no over-read past it); 0 on a closed or
 * silent socket, or a head past 16 KiB. */
static int loft_eh_read_head(int fd, loft_text *head) {
    while (head->len < 4 || memcmp(head->p + head->len - 4, "\r\n\r\n", 4) != 0) {
        char b;
        ssize_t r = recv(fd, &b, 1, 0);
        if (r < 0 && errno == EINTR) continue;
        if (r != 1) return 0;
        loft_push(head, &b, 1);
        if (head->len > 16 * 1024) return 0;
    }
    return 1;
}

/* SHA-1 (RFC 3174), for the WebSocket accept key alone. */
static void loft_eh_sha1(const uint8_t *data, size_t n, uint8_t out[20]) {
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
static void loft_eh_accept_key(const char *key, size_t kn, char out[29]) {
    static const char guid[] = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";
    static const char b64[] = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    uint8_t in[256], d[20];
    size_t n = kn < 200 ? kn : 200;
    memcpy(in, key, n);
    memcpy(in + n, guid, sizeof guid - 1);
    loft_eh_sha1(in, n + sizeof guid - 1, d);
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

/* Upgrade a freshly accepted stream: read the request head (bounded), answer the handshake, the
 * cookie riding the 101 as `X-Loft-UDP` when there is one.  0 drops the connection. */
static int loft_eh_ws_upgrade(int fd, const char *cookie) {
    loft_sock_blocking(fd, 1);
    loft_sock_timeout(fd, 500);
    loft_text head = { 0, 0, 0 };
    const char *v;
    size_t vn;
    int ok = loft_eh_read_head(fd, &head);
    int upgrade = ok && loft_http_header(&head, "upgrade", &v, &vn) && vn == 9 && strncasecmp(v, "websocket", 9) == 0;
    char accept[29];
    if (upgrade && loft_http_header(&head, "sec-websocket-key", &v, &vn)) {
        loft_eh_accept_key(v, vn, accept);
    } else {
        upgrade = 0;
    }
    free(head.p);
    if (!upgrade) return 0;
    char resp[512];
    int n = snprintf(resp, sizeof resp,
                     "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                     "Sec-WebSocket-Accept: %s\r\n%s%s%s\r\n",
                     accept, *cookie ? "X-Loft-UDP: " : "", cookie, *cookie ? "\r\n" : "");
    if (!loft_eh_write_all(fd, resp, (size_t) n)) return 0;
    loft_sock_timeout(fd, 20);
    return 1;
}

/* Write one frame: `mask` for a client's (RFC 6455 requires it; the key only defeats broken
 * proxies, so it is derived from the length). */
static int loft_eh_write_frame(int fd, uint8_t opcode, const char *p, size_t n, int mask) {
    uint8_t hdr[14];
    size_t h = 0;
    hdr[h++] = (uint8_t) (0x80 | opcode);
    uint8_t m = mask ? 0x80 : 0;
    if (n <= 125) {
        hdr[h++] = (uint8_t) (m | n);
    } else if (n <= 0xFFFF) {
        hdr[h++] = (uint8_t) (m | 126);
        hdr[h++] = (uint8_t) (n >> 8);
        hdr[h++] = (uint8_t) n;
    } else {
        hdr[h++] = (uint8_t) (m | 127);
        for (int i = 7; i >= 0; i--) hdr[h++] = (uint8_t) ((uint64_t) n >> (8 * i));
    }
    loft_text frame = { 0, 0, 0 };
    if (mask) {
        uint32_t k = (uint32_t) n * 0x9E3779B9u;
        uint8_t key[4] = { (uint8_t) (k >> 24), (uint8_t) (k >> 16), (uint8_t) (k >> 8), (uint8_t) k };
        memcpy(hdr + h, key, 4);
        h += 4;
        loft_push(&frame, (const char *) hdr, h);
        for (size_t i = 0; i < n; i++) {
            char c = (char) (p[i] ^ key[i % 4]);
            loft_push(&frame, &c, 1);
        }
    } else {
        loft_push(&frame, (const char *) hdr, h);
        loft_push(&frame, p, n);
    }
    int ok = loft_eh_write_all(fd, frame.p, frame.len);
    free(frame.p);
    return ok;
}

/* One frame if pending: peek first (idle costs a syscall), then the in-flight frame within the
 * short read timeout.  0 none, 1 a text frame (into `out`), 2 closed. */
static int loft_eh_read_frame(int fd, loft_text *out) {
    uint8_t hdr[2];
    ssize_t peeked = recv(fd, hdr, 2, MSG_PEEK | MSG_DONTWAIT);
    if (peeked == 0) return 2;
    if (peeked < 0) return errno == EAGAIN || errno == EWOULDBLOCK || errno == EINTR ? 0 : 2;
    if (peeked < 2) return 0;
    if (!loft_eh_read_exact(fd, hdr, 2)) return 2;
    uint8_t opcode = hdr[0] & 0x0F;
    int masked = (hdr[1] & 0x80) != 0;
    uint64_t len = hdr[1] & 0x7F;
    if (len == 126) {
        uint8_t b[2];
        if (!loft_eh_read_exact(fd, b, 2)) return 2;
        len = (uint64_t) b[0] << 8 | b[1];
    } else if (len == 127) {
        uint8_t b[8];
        if (!loft_eh_read_exact(fd, b, 8)) return 2;
        len = 0;
        for (int i = 0; i < 8; i++) len = len << 8 | b[i];
    }
    if (len > 16 * 1024 * 1024) return 2;
    uint8_t mask[4] = { 0, 0, 0, 0 };
    if (masked && !loft_eh_read_exact(fd, mask, 4)) return 2;
    char *payload = (char *) malloc(len ? (size_t) len : 1);
    if (!payload) { fputs("loft: out of memory\n", stderr); abort(); }
    if (len && !loft_eh_read_exact(fd, payload, (size_t) len)) {
        free(payload);
        return 2;
    }
    if (masked)
        for (uint64_t i = 0; i < len; i++) payload[i] ^= (char) mask[i % 4];
    int answer = 0;
    if (opcode == 0x1) {
        loft_push_lossy(out, payload, (size_t) len);
        answer = 1;
    } else if (opcode == 0x8) {
        answer = 2;
    } else if (opcode == 0x9) {
        loft_eh_write_frame(fd, 0xA, payload, (size_t) len, 0);
    }
    free(payload);
    return answer;
}

/* A socket bound on 0.0.0.0:`port` for a hand-over: close-on-exec, SO_REUSEADDR (TCP) and
 * SO_REUSEPORT, so a swap's new build binds while the old one serves.  -1 on failure. */
static int loft_eh_bind(int64_t port, int type) {
    int fd = socket(AF_INET, type, 0);
    if (fd < 0) return -1;
    loft_sock_cloexec(fd);
    int one = 1;
    if (type == SOCK_STREAM) setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
#if defined(SO_REUSEPORT)
    setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &one, sizeof one);
#endif
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons((uint16_t) port);
    if (bind(fd, (struct sockaddr *) &a, sizeof a) != 0 || (type == SOCK_STREAM && listen(fd, 128) != 0)) {
        close(fd);
        return -1;
    }
    loft_sock_blocking(fd, 0);
    return fd;
}

static int loft_eh_same_addr(const struct sockaddr_storage *a, socklen_t al, const struct sockaddr_storage *b,
                             socklen_t bl) {
    return al == bl && memcmp(a, b, al) == 0;
}

/* 16 hex characters from the OS's random source, the clock and `cid`: spoof-resistant on a LAN,
 * not cryptographic (the cookie the Rust kernel mints from a seeded hasher). */
static void loft_eh_cookie(size_t cid, char out[17]) {
    uint64_t x = 0;
    int fd = open("/dev/urandom", O_RDONLY);
    if (fd >= 0) {
        ssize_t r = read(fd, &x, sizeof x);
        (void) r;
        close(fd);
    }
    x ^= (uint64_t) loft_now_us() * 0x9E3779B97F4A7C15ull ^ (uint64_t) cid * 0xBF58476D1CE4E5B9ull ^ (uint64_t) getpid();
    x ^= x >> 31;
    snprintf(out, 17, "%016llx", (unsigned long long) x);
}
#endif

/* ── The listener kernel ── */

typedef struct {
    int fd; /* -1: a free slot (cid = index, reused) */
    char cookie[17];
    int bound;
    struct sockaddr_storage addr;
    socklen_t alen;
    int64_t last_seen, out_seq;
    loft_eh_slots slots;
    char *debug_name;  /* a debug client's registered name */
    int64_t relay_to;  /* the agent a forwarded command's reply goes to; -1 none */
} loft_eh_conn;

static struct {
    int up, alive, listener, udp, warned_oversize;
    loft_eh_conn *conns;
    size_t nconns;
    loft_eh_queue events;
    loft_eh_event last;
    int64_t sync_cid, sync_seq;
    loft_text sync_payload;
    int64_t start, tick, last_tick;
} loft_ehk;

static int64_t loft_ehk_now(void) { return loft_now_us() - loft_ehk.start; }

static void loft_eh_conn_reset(loft_eh_conn *c) {
    c->fd = -1;
    c->cookie[0] = 0;
    c->bound = 0;
    c->alen = 0;
    c->last_seen = 0;
    c->out_seq = 0;
    loft_eh_slots_free(&c->slots);
    /* A debug client's name and a relay route belong to the slot, not the connection: the
     * Rust kernel's maps keep them across a reconnect. */
}

/* ── The connector (or local) kernel ── */

static struct {
    int up, alive, conn, udp, udp_bound, ctl;
    char cookie[64];
    int64_t last_hello, last_keepalive, out_seq;
    loft_eh_slots slots;
    int64_t sync_seq;
    loft_text sync_payload;
    loft_eh_queue events;
    loft_eh_event last;
    int64_t start, tick, last_tick;
    int *ctl_conns;
    size_t nctl;
} loft_ehc;

static int64_t loft_ehc_now(void) { return loft_now_us() - loft_ehc.start; }

/* ── The debug control channel: `D!:` frames from a loopback peer, gated on LOFT_DEBUG_CONTROL=1.
 * With no live tier (a compiled C build has none) a flip refuses, a breakpoint is queued and
 * acknowledged, and eval / resume / reload are queued without a reply — what an unbooted live
 * tier answers. ── */

static int loft_eh_debug_on(void) {
    static int on = -1;
    if (on < 0) {
        const char *v = getenv("LOFT_DEBUG_CONTROL");
        on = v && strcmp(v, "1") == 0;
    }
    return on;
}

static int loft_eh_quit = 0;

/* `{:?}` of a text, as the Rust kernel quotes an unknown command. */
static void loft_eh_debug_quote(loft_text *o, const char *p, size_t n) {
    loft_push(o, "\"", 1);
    for (size_t i = 0; i < n; i++) {
        unsigned char c = (unsigned char) p[i];
        char esc[16];
        if (c == '"' || c == '\\') { esc[0] = '\\'; esc[1] = (char) c; loft_push(o, esc, 2); }
        else if (c == '\n') loft_push(o, "\\n", 2);
        else if (c == '\r') loft_push(o, "\\r", 2);
        else if (c == '\t') loft_push(o, "\\t", 2);
        else if (c == 0) loft_push(o, "\\0", 2);
        else if (c < 0x20 || c == 0x7F) { snprintf(esc, sizeof esc, "\\u{%x}", c); loft_push_cstr(o, esc); }
        else loft_push(o, (const char *) &p[i], 1);
    }
    loft_push(o, "\"", 1);
}

/* `debug_cmd_dispatch`, the command core both roles share: 1 and the immediate reply in `r`, or 0
 * for a command whose answer would come from the pause loop. */
static int loft_eh_debug_dispatch(const char *cmd, size_t n, loft_text *r) {
    const char *sp = (const char *) memchr(cmd, ' ', n);
    size_t vl = sp ? (size_t) (sp - cmd) : n;
    const char *arg = sp ? sp + 1 : cmd + n;
    size_t an = n - (size_t) (arg - cmd);
#define LOFT_VERB(s) (vl == sizeof s - 1 && memcmp(cmd, s, vl) == 0)
    if (LOFT_VERB("bp") && an > 0) {
        loft_push_cstr(r, "D:ok bp ");
        loft_push(r, arg, an);
    } else if (LOFT_VERB("flip")) {
        loft_push_cstr(r, "D:flip false");
    } else if ((LOFT_VERB("eval") && an > 0) || LOFT_VERB("resume") || LOFT_VERB("reload")) {
        return 0;
    } else if (LOFT_VERB("apply")) {
        loft_push_cstr(r, "D:apply queued");
    } else if (LOFT_VERB("rebuild")) {
        fputs("loft-live: rebuild needs LOFT_LIVE_DRIVER/LOFT_LIVE_SRC (run through the loft driver)\n", stderr);
        loft_push_cstr(r, "D:rebuild refused");
    } else if (LOFT_VERB("rebuild?")) {
        loft_push_cstr(r, "D:rebuild 0");
    } else if (LOFT_VERB("swap")) {
        fputs("loft-swap: this build cannot hand its world to another — swap refused\n", stderr);
        loft_push_cstr(r, "D:swap false");
    } else if (LOFT_VERB("quit")) {
        fputs("loft-debug: quit via the control channel\n", stderr);
        loft_eh_quit = 1;
        loft_push_cstr(r, "D:quitting");
    } else {
        loft_push_cstr(r, "D:err unknown command ");
        loft_eh_debug_quote(r, cmd, n);
    }
#undef LOFT_VERB
    return 1;
}

static void loft_eh_trim(const char **p, size_t *n) {
    while (*n && isspace((unsigned char) **p)) { (*p)++; (*n)--; }
    while (*n && isspace((unsigned char) (*p)[*n - 1])) (*n)--;
}

#if defined(LOFT_POSIX_HOST)
static void loft_ehk_disconnect(size_t cid);

/* `deliver`: a sync-class message rides a seq-stamped datagram when the client's UDP path is
 * bound; everything else, and every client without one, gets a WS frame.  A failed WS write
 * disconnects the client. */
static int loft_ehk_deliver(size_t cid, const char *msg, size_t n, int sync) {
    if (cid >= loft_ehk.nconns) return 0;
    loft_eh_conn *c = &loft_ehk.conns[cid];
    if (sync && c->bound && loft_ehk.udp >= 0) {
        c->out_seq += 1;
        loft_text d = { 0, 0, 0 };
        char seq[32];
        snprintf(seq, sizeof seq, "S:%lld:", (long long) c->out_seq);
        loft_push_cstr(&d, seq);
        loft_push(&d, msg, n);
        int ok = 0;
        if (d.len > LOFT_EH_UDP_MAX) {
            if (!loft_ehk.warned_oversize) {
                loft_ehk.warned_oversize = 1;
                fprintf(stderr,
                        "engine_host: dropped a %zu B sync datagram (cap %d B; state frames must stay small — bulk "
                        "belongs on the WS channel)\n",
                        d.len, LOFT_EH_UDP_MAX);
            }
        } else {
            ok = sendto(loft_ehk.udp, d.p, d.len, 0, (struct sockaddr *) &c->addr, c->alen) >= 0;
        }
        free(d.p);
        return ok;
    }
    if (c->fd < 0) return 0;
    if (!loft_eh_write_frame(c->fd, 0x1, msg, n, 0)) {
        loft_ehk_disconnect(cid);
        return 0;
    }
    return 1;
}

static void loft_ehk_disconnect(size_t cid) {
    close(loft_ehk.conns[cid].fd);
    loft_eh_conn_reset(&loft_ehk.conns[cid]);
    loft_eh_push(&loft_ehk.events, (int64_t) cid, 2, 0, "", 0);
}

static int loft_eh_loopback(int fd) {
    struct sockaddr_storage a;
    socklen_t al = sizeof a;
    if (getpeername(fd, (struct sockaddr *) &a, &al) != 0) return 0;
    if (a.ss_family == AF_INET) return (ntohl(((struct sockaddr_in *) &a)->sin_addr.s_addr) >> 24) == 127;
    if (a.ss_family == AF_INET6) {
        const struct in6_addr *x = &((struct sockaddr_in6 *) &a)->sin6_addr;
        return IN6_IS_ADDR_LOOPBACK(x) || (IN6_IS_ADDR_V4MAPPED(x) && x->s6_addr[12] == 127);
    }
    return 0;
}

/* `handle_debug_control`: a debug client registering its name (`iam`), relaying a reply
 * (`reply`), an agent forwarding to a named client (`@name:cmd`), or a command for this
 * process. */
static void loft_ehk_debug(size_t cid, const char *cmd, size_t n) {
    if (!loft_eh_debug_on()) return;
    loft_eh_trim(&cmd, &n);
    loft_eh_conn *c = &loft_ehk.conns[cid];
    loft_text r = { 0, 0, 0 };
    if (n >= 4 && memcmp(cmd, "iam ", 4) == 0) {
        const char *name = cmd + 4;
        size_t nn = n - 4;
        loft_eh_trim(&name, &nn);
        free(c->debug_name);
        c->debug_name = (char *) malloc(nn + 1);
        if (!c->debug_name) { fputs("loft: out of memory\n", stderr); abort(); }
        memcpy(c->debug_name, name, nn);
        c->debug_name[nn] = 0;
        loft_push_cstr(&r, "D:registered ");
        loft_push(&r, name, nn);
        loft_ehk_deliver(cid, r.p, r.len, 0);
        free(r.p);
        return;
    }
    if (n >= 6 && memcmp(cmd, "reply ", 6) == 0) {
        const char *msg = cmd + 6;
        size_t mn = n - 6;
        loft_eh_trim(&msg, &mn);
        if (c->relay_to >= 0) loft_ehk_deliver((size_t) c->relay_to, msg, mn, 0);
        return;
    }
    if (c->fd < 0 || !loft_eh_loopback(c->fd)) return;
    if (n > 1 && cmd[0] == '@' && memchr(cmd, ':', n)) {
        const char *colon = (const char *) memchr(cmd, ':', n);
        const char *name = cmd + 1;
        size_t nn = (size_t) (colon - name);
        const char *inner = colon + 1;
        size_t in = n - (size_t) (inner - cmd);
        loft_eh_trim(&inner, &in);
        size_t target = loft_ehk.nconns;
        for (size_t i = 0; i < loft_ehk.nconns; i++)
            if (loft_ehk.conns[i].debug_name && strlen(loft_ehk.conns[i].debug_name) == nn &&
                memcmp(loft_ehk.conns[i].debug_name, name, nn) == 0)
                target = i;
        if (target < loft_ehk.nconns) {
            loft_ehk.conns[target].relay_to = (int64_t) cid;
            loft_text framed = { 0, 0, 0 };
            loft_push_cstr(&framed, "D!:");
            loft_push(&framed, inner, in);
            int sent = loft_ehk.conns[target].fd >= 0 &&
                       loft_eh_write_frame(loft_ehk.conns[target].fd, 1, framed.p, framed.len, 0);
            free(framed.p);
            loft_push_cstr(&r, sent ? "D:relayed " : "D:unreachable ");
        } else {
            loft_push_cstr(&r, "D:err no debug client ");
        }
        loft_push(&r, name, nn);
        loft_ehk_deliver(cid, r.p, r.len, 0);
        free(r.p);
        return;
    }
    if (loft_eh_debug_dispatch(cmd, n, &r)) loft_ehk_deliver(cid, r.p, r.len, 0);
    free(r.p);
    if (loft_eh_quit) {
        fflush(0);
        exit(0);
    }
}

/* Every pending datagram into the conflation slots; bind and refresh UDP paths; expire silent
 * ones.  Datagram arrivals are not loop work: they conflate, and the tick reads the newest. */
static void loft_ehk_pump_udp(void) {
    if (loft_ehk.udp < 0) return;
    int64_t now = loft_ehk_now();
    char buf[2048];
    for (;;) {
        struct sockaddr_storage from;
        socklen_t fl = sizeof from;
        ssize_t len = recvfrom(loft_ehk.udp, buf, sizeof buf, MSG_DONTWAIT, (struct sockaddr *) &from, &fl);
        if (len < 0) break;
        if (!loft_utf8_valid((const unsigned char *) buf, (size_t) len)) continue;
        if (len >= 2 && buf[0] == 'H' && buf[1] == ':') {
            const char *cookie = buf + 2;
            size_t cn = (size_t) len - 2;
            for (size_t i = 0; i < loft_ehk.nconns; i++) {
                loft_eh_conn *c = &loft_ehk.conns[i];
                if (c->cookie[0] && strlen(c->cookie) == cn && memcmp(c->cookie, cookie, cn) == 0) {
                    if (c->fd >= 0) {
                        c->bound = 1;
                        c->addr = from;
                        c->alen = fl;
                        c->last_seen = now;
                        char ack[32];
                        int an = snprintf(ack, sizeof ack, "A:%zu", i);
                        sendto(loft_ehk.udp, ack, (size_t) an, 0, (struct sockaddr *) &from, fl);
                    }
                    break;
                }
            }
            continue;
        }
        size_t cid = loft_ehk.nconns;
        for (size_t i = 0; i < loft_ehk.nconns && cid == loft_ehk.nconns; i++)
            if (loft_ehk.conns[i].bound && loft_eh_same_addr(&loft_ehk.conns[i].addr, loft_ehk.conns[i].alen, &from, fl))
                cid = i;
        if (cid == loft_ehk.nconns) continue;
        loft_ehk.conns[cid].last_seen = now;
        int64_t seq;
        const char *body;
        size_t bl;
        if (loft_eh_seq_frame(buf, (size_t) len, &seq, &body, &bl)) loft_eh_conflate(&loft_ehk.conns[cid].slots, seq, body, bl);
    }
    for (size_t i = 0; i < loft_ehk.nconns; i++)
        if (loft_ehk.conns[i].bound && now - loft_ehk.conns[i].last_seen > LOFT_EH_UDP_TIMEOUT_US) loft_ehk.conns[i].bound = 0;
}
#endif

/* ── The listener natives ── */

static uint8_t loft_kernel_listen(int64_t port, int64_t tick_us) {
#if defined(LOFT_POSIX_HOST)
    int fd = loft_eh_bind(port, SOCK_STREAM);
    if (fd < 0) return 0;
    int udp = loft_eh_bind(port, SOCK_DGRAM);
    if (udp < 0) fprintf(stderr, "engine_host: UDP bind on %lld failed (%s); state-sync rides WS\n", (long long) port, strerror(errno));
    if (loft_ehk.up) {
        if (loft_ehk.listener >= 0) close(loft_ehk.listener);
        if (loft_ehk.udp >= 0) close(loft_ehk.udp);
        for (size_t i = 0; i < loft_ehk.nconns; i++) {
            if (loft_ehk.conns[i].fd >= 0) close(loft_ehk.conns[i].fd);
            loft_eh_conn_reset(&loft_ehk.conns[i]);
            free(loft_ehk.conns[i].debug_name);
        }
        loft_eh_queue_free(&loft_ehk.events);
    }
    free(loft_ehk.conns);
    loft_ehk.conns = 0;
    loft_ehk.nconns = 0;
    loft_ehk.up = 1;
    loft_ehk.alive = 1;
    loft_ehk.listener = fd;
    loft_ehk.udp = udp;
    loft_ehk.warned_oversize = 0;
    free(loft_ehk.last.payload.p);
    loft_ehk.last.payload.p = 0;
    loft_ehk.last.payload.len = loft_ehk.last.payload.cap = 0;
    loft_ehk.last.cid = loft_ehk.last.kind = -1;
    loft_ehk.last.status = 0;
    loft_ehk.sync_cid = loft_ehk.sync_seq = -1;
    loft_ehk.sync_payload.len = 0;
    loft_ehk.start = loft_now_us();
    loft_ehk.tick = tick_us > 1 ? tick_us : 1;
    loft_ehk.last_tick = 0;
    const char *ready = getenv("LOFT_SWAP_READY");
    if (ready) {
        FILE *f = fopen(ready, "wb");
        if (f) { fputs("serving", f); fclose(f); }
        fprintf(stderr, "loft-swap: new build serving on port %lld (ready file touched)\n", (long long) port);
    }
    return 1;
#else
    (void) port;
    (void) tick_us;
    return 0;
#endif
}

static int64_t loft_kernel_pump(void) {
    if (!loft_ehk.up) return 0;
    loft_eh_drain_http(&loft_ehk.events);
    int64_t added = 0;
#if defined(LOFT_POSIX_HOST)
    for (;;) {
        int fd = accept(loft_ehk.listener, 0, 0);
        if (fd < 0) break;
        loft_sock_cloexec(fd);
        size_t cid = 0;
        while (cid < loft_ehk.nconns && loft_ehk.conns[cid].fd >= 0) cid++;
        char cookie[17] = { 0 };
        if (loft_ehk.udp >= 0) loft_eh_cookie(cid, cookie);
        if (!loft_eh_ws_upgrade(fd, cookie)) {
            close(fd);
            continue;
        }
        if (cid == loft_ehk.nconns) {
            loft_ehk.conns = (loft_eh_conn *) realloc(loft_ehk.conns, (loft_ehk.nconns + 1) * sizeof *loft_ehk.conns);
            if (!loft_ehk.conns) { fputs("loft: out of memory\n", stderr); abort(); }
            memset(&loft_ehk.conns[cid], 0, sizeof *loft_ehk.conns);
            loft_ehk.conns[cid].relay_to = -1;
            loft_ehk.nconns++;
        }
        loft_eh_conn_reset(&loft_ehk.conns[cid]);
        loft_ehk.conns[cid].fd = fd;
        memcpy(loft_ehk.conns[cid].cookie, cookie, sizeof cookie);
        loft_eh_push(&loft_ehk.events, (int64_t) cid, 0, 0, "", 0);
        added++;
    }
    for (size_t cid = 0; cid < loft_ehk.nconns; cid++) {
        while (loft_ehk.conns[cid].fd >= 0) {
            loft_text msg = { 0, 0, 0 };
            int got = loft_eh_read_frame(loft_ehk.conns[cid].fd, &msg);
            if (got == 0) {
                free(msg.p);
                break;
            }
            if (got == 2) {
                free(msg.p);
                loft_ehk_disconnect(cid);
                added++;
                break;
            }
            if (msg.len >= 3 && memcmp(msg.p, "D!:", 3) == 0) {
                loft_ehk_debug(cid, msg.p + 3, msg.len - 3);
            } else if (loft_eh_is_sync(msg.p, msg.len)) {
                loft_eh_conflate_ws(&loft_ehk.conns[cid].slots, msg.p, msg.len);
            } else {
                loft_eh_push(&loft_ehk.events, (int64_t) cid, 1, 0, msg.p, msg.len);
                added++;
            }
            free(msg.p);
        }
    }
    loft_ehk_pump_udp();
#endif
    return added;
}

static uint8_t loft_kernel_next_event(void) {
    return loft_ehk.up && loft_eh_pop(&loft_ehk.events, &loft_ehk.last);
}
static int64_t loft_kernel_event_cid(void) { return loft_ehk.up ? loft_ehk.last.cid : -1; }
static int64_t loft_kernel_event_kind(void) { return loft_ehk.up ? loft_ehk.last.kind : -1; }
static int64_t loft_kernel_event_status(void) { return loft_ehk.up ? loft_ehk.last.status : 0; }
static loft_text loft_kernel_event_payload(void) {
    loft_text t = { 0, 0, 0 };
    if (loft_ehk.up) t = loft_eh_copy(&loft_ehk.last.payload);
    return t;
}

/* Drift-free: a due tick advances the grid by one interval (never to `now`), so missed time is
 * caught up tick by tick; the first tick anchors the grid. */
static uint8_t loft_eh_tick_due(int64_t now, int64_t tick, int64_t *last) {
    if (now - *last < tick) return 0;
    *last = *last == 0 ? now : *last + tick;
    return 1;
}

static uint8_t loft_kernel_tick_due(void) {
    return loft_ehk.up && loft_eh_tick_due(loft_ehk_now(), loft_ehk.tick, &loft_ehk.last_tick);
}

static uint8_t loft_kernel_send(int64_t cid, loft_str msg) {
#if defined(LOFT_POSIX_HOST)
    if (!loft_ehk.up || cid < 0) return 0;
    return (uint8_t) loft_ehk_deliver((size_t) cid, msg.p, msg.len, loft_eh_is_sync(msg.p, msg.len));
#else
    (void) cid;
    (void) msg;
    return 0;
#endif
}

static int64_t loft_kernel_broadcast(loft_str msg) {
    int64_t sent = 0;
#if defined(LOFT_POSIX_HOST)
    if (!loft_ehk.up) return 0;
    int sync = loft_eh_is_sync(msg.p, msg.len);
    for (size_t cid = 0; cid < loft_ehk.nconns; cid++)
        if (loft_ehk.conns[cid].fd >= 0 && loft_ehk_deliver(cid, msg.p, msg.len, sync)) sent++;
#else
    (void) msg;
#endif
    return sent;
}

static void loft_eh_sleep_us(int64_t us) {
    if (us <= 0) return;
    struct timespec ts = { (time_t) (us / 1000000), (long) (us % 1000000) * 1000 };
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {}
}

/* Sleep up to `max_us`, never past the next tick boundary. */
static int64_t loft_eh_idle_for(int64_t max_us, int64_t now, int64_t tick, int64_t last) {
    int64_t until = last == 0 ? tick : (last + tick - now > 0 ? last + tick - now : 0);
    if (until < 1) until = 1;
    return max_us < 0 ? 0 : max_us > until ? until : max_us;
}

static void loft_kernel_idle(int64_t max_us) {
    loft_eh_sleep_us(loft_ehk.up ? loft_eh_idle_for(max_us, loft_ehk_now(), loft_ehk.tick, loft_ehk.last_tick)
                                 : (max_us > 0 ? max_us : 0));
}

static int64_t loft_kernel_clients(void) {
    int64_t n = 0;
    for (size_t i = 0; i < loft_ehk.nconns; i++) n += loft_ehk.conns[i].fd >= 0;
    return n;
}
static uint8_t loft_kernel_alive(void) { return loft_ehk.up && loft_ehk.alive; }
static void loft_kernel_stop(void) {
    if (loft_ehk.up) loft_ehk.alive = 0;
}
static void loft_kernel_frame(void) {}

/* `post`: a local event (`cid -1`) on the active role's queue, the connector's first. */
static uint8_t loft_kernel_post(loft_str msg) {
    if (loft_ehc.up) {
        loft_eh_push(&loft_ehc.events, -1, 1, 0, msg.p, msg.len);
        return 1;
    }
    if (loft_ehk.up) {
        loft_eh_push(&loft_ehk.events, -1, 1, 0, msg.p, msg.len);
        return 1;
    }
    return 0;
}

static uint8_t loft_kernel_udp_bound(int64_t cid) {
    return loft_ehk.up && cid >= 0 && (size_t) cid < loft_ehk.nconns && loft_ehk.conns[cid].bound;
}
static void loft_kernel_sync_class(int64_t id) { loft_eh_sync_declare(id, 0); }
static void loft_kernel_sync_class_keyed(int64_t id) { loft_eh_sync_declare(id, 1); }

/* `deliver_keyframe`: a sync sample promoted to must-deliver rides the reliable channel, stamped
 * in the datagram seq space for a bound peer (`S:<seq>:` — it conflates there), plain for one
 * that is not. */
static uint8_t loft_kernel_keyframe(int64_t cid, loft_str msg) {
#if defined(LOFT_POSIX_HOST)
    if (!loft_ehk.up || cid < 0 || (size_t) cid >= loft_ehk.nconns) return 0;
    loft_eh_conn *c = &loft_ehk.conns[cid];
    loft_text wire = { 0, 0, 0 };
    if (c->bound) {
        c->out_seq += 1;
        char seq[32];
        snprintf(seq, sizeof seq, "S:%lld:", (long long) c->out_seq);
        loft_push_cstr(&wire, seq);
    }
    loft_push(&wire, msg.p, msg.len);
    uint8_t ok = 0;
    if (c->fd >= 0) {
        ok = (uint8_t) loft_eh_write_frame(c->fd, 0x1, wire.p, wire.len, 0);
        if (!ok) loft_ehk_disconnect((size_t) cid);
    }
    free(wire.p);
    return ok;
#else
    (void) cid;
    (void) msg;
    return 0;
#endif
}

/* The next dirty slot, newest state from one client, into the sync getters. */
static uint8_t loft_kernel_sync_next(void) {
    if (!loft_ehk.up) return 0;
    for (size_t cid = 0; cid < loft_ehk.nconns; cid++) {
        loft_eh_slots *s = &loft_ehk.conns[cid].slots;
        for (size_t i = 0; i < s->n; i++) {
            if (!s->v[i].dirty) continue;
            s->v[i].dirty = 0;
            loft_ehk.sync_cid = (int64_t) cid;
            loft_ehk.sync_seq = s->v[i].seq;
            loft_ehk.sync_payload.len = 0;
            loft_push(&loft_ehk.sync_payload, s->v[i].payload.p, s->v[i].payload.len);
            return 1;
        }
    }
    return 0;
}
static int64_t loft_kernel_sync_cid(void) { return loft_ehk.up ? loft_ehk.sync_cid : -1; }
static int64_t loft_kernel_sync_seq(void) { return loft_ehk.up ? loft_ehk.sync_seq : -1; }
static loft_text loft_kernel_sync_payload(void) {
    loft_text t = { 0, 0, 0 };
    if (loft_ehk.up) t = loft_eh_copy(&loft_ehk.sync_payload);
    return t;
}

/* ── The connector natives ── */

#if defined(LOFT_POSIX_HOST)
/* The client's loopback debug-control listener, announced on stdout; -1 unless
 * LOFT_DEBUG_CONTROL=1. */
static int loft_ehc_ctl_listener(void) {
    if (!loft_eh_debug_on()) return -1;
    int fd = socket(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return -1;
    loft_sock_cloexec(fd);
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    socklen_t al = sizeof a;
    if (bind(fd, (struct sockaddr *) &a, sizeof a) != 0 || listen(fd, 128) != 0 ||
        getsockname(fd, (struct sockaddr *) &a, &al) != 0) {
        close(fd);
        return -1;
    }
    loft_sock_blocking(fd, 0);
    printf("engine_host: debug control on 127.0.0.1:%u\n", (unsigned) ntohs(a.sin_port));
    fflush(stdout);
    return fd;
}

/* Accept and serve the client's control connections through the shared command core. */
static void loft_ehc_pump_ctl(void) {
    if (loft_ehc.ctl < 0) return;
    for (;;) {
        int fd = accept(loft_ehc.ctl, 0, 0);
        if (fd < 0) break;
        loft_sock_cloexec(fd);
        if (!loft_eh_ws_upgrade(fd, "")) {
            close(fd);
            continue;
        }
        size_t i = 0;
        while (i < loft_ehc.nctl && loft_ehc.ctl_conns[i] >= 0) i++;
        if (i == loft_ehc.nctl) {
            loft_ehc.ctl_conns = (int *) realloc(loft_ehc.ctl_conns, (loft_ehc.nctl + 1) * sizeof *loft_ehc.ctl_conns);
            if (!loft_ehc.ctl_conns) { fputs("loft: out of memory\n", stderr); abort(); }
            loft_ehc.nctl++;
        }
        loft_ehc.ctl_conns[i] = fd;
    }
    for (size_t i = 0; i < loft_ehc.nctl; i++) {
        while (loft_ehc.ctl_conns[i] >= 0) {
            loft_text msg = { 0, 0, 0 };
            int got = loft_eh_read_frame(loft_ehc.ctl_conns[i], &msg);
            if (got == 0) {
                free(msg.p);
                break;
            }
            if (got == 2) {
                free(msg.p);
                close(loft_ehc.ctl_conns[i]);
                loft_ehc.ctl_conns[i] = -1;
                break;
            }
            if (msg.len >= 3 && memcmp(msg.p, "D!:", 3) == 0) {
                const char *cmd = msg.p + 3;
                size_t n = msg.len - 3;
                loft_eh_trim(&cmd, &n);
                loft_text r = { 0, 0, 0 };
                if (loft_eh_debug_dispatch(cmd, n, &r)) loft_eh_write_frame(loft_ehc.ctl_conns[i], 1, r.p, r.len, 0);
                free(r.p);
                if (loft_eh_quit) {
                    fflush(0);
                    exit(0);
                }
            }
            free(msg.p);
        }
    }
}
#endif

static void loft_ehc_reset(int64_t tick_us) {
#if defined(LOFT_POSIX_HOST)
    if (loft_ehc.up) {
        if (loft_ehc.conn >= 0) close(loft_ehc.conn);
        if (loft_ehc.udp >= 0) close(loft_ehc.udp);
        if (loft_ehc.ctl >= 0) close(loft_ehc.ctl);
        for (size_t i = 0; i < loft_ehc.nctl; i++)
            if (loft_ehc.ctl_conns[i] >= 0) close(loft_ehc.ctl_conns[i]);
    }
#endif
    free(loft_ehc.ctl_conns);
    loft_ehc.ctl_conns = 0;
    loft_ehc.nctl = 0;
    loft_eh_queue_free(&loft_ehc.events);
    loft_eh_slots_free(&loft_ehc.slots);
    loft_ehc.up = 1;
    loft_ehc.alive = 1;
    loft_ehc.conn = loft_ehc.udp = loft_ehc.ctl = -1;
    loft_ehc.udp_bound = 0;
    loft_ehc.cookie[0] = 0;
    loft_ehc.last_hello = INT64_MIN / 2;
    loft_ehc.last_keepalive = 0;
    loft_ehc.out_seq = 0;
    loft_ehc.sync_seq = -1;
    loft_ehc.sync_payload.len = 0;
    free(loft_ehc.last.payload.p);
    loft_ehc.last.payload.p = 0;
    loft_ehc.last.payload.len = loft_ehc.last.payload.cap = 0;
    loft_ehc.last.cid = loft_ehc.last.kind = -1;
    loft_ehc.last.status = 0;
    loft_ehc.start = loft_now_us();
    loft_ehc.tick = tick_us > 1 ? tick_us : 1;
    loft_ehc.last_tick = 0;
}

static void loft_eh_touch_ready(const char *what, int say) {
    const char *ready = getenv("LOFT_SWAP_READY");
    if (!ready) return;
    FILE *f = fopen(ready, "wb");
    if (f) { fputs(what, f); fclose(f); }
    if (say) fputs("loft-swap: new build connected (ready file touched)\n", stderr);
}

/* `kernel_local`: the client kernel with no transport — ticks, the swap step and the debug
 * endpoint without a server. */
static uint8_t loft_kernel_local(int64_t tick_us) {
    loft_ehc_reset(tick_us);
#if defined(LOFT_POSIX_HOST)
    loft_ehc.ctl = loft_ehc_ctl_listener();
#endif
    loft_eh_touch_ready("connected", 0);
    return 1;
}

/* `kernel_connect`: dial (retried across a swap's gap, five seconds), upgrade with the RFC's
 * sample key (anti-cache, not auth), take the `X-Loft-UDP` cookie, prepare the UDP socket.  A
 * kind-0 event marks the connect, as the listener's does. */
static uint8_t loft_kernel_connect(loft_str host, int64_t port, int64_t tick_us) {
#if defined(LOFT_POSIX_HOST)
    char *h = loft_cstr_own(host), ps[16];
    snprintf(ps, sizeof ps, "%u", (unsigned) (uint16_t) port);
    struct addrinfo hints, *res = 0;
    memset(&hints, 0, sizeof hints);
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(h, ps, &hints, &res) != 0 || !res) {
        free(h);
        return 0;
    }
    struct sockaddr_storage addr;
    socklen_t al = (socklen_t) res->ai_addrlen;
    memcpy(&addr, res->ai_addr, res->ai_addrlen);
    int family = res->ai_family;
    freeaddrinfo(res);
    int64_t deadline = loft_now_us() + 5000000;
    int fd = -1;
    for (;;) {
        fd = socket(family, SOCK_STREAM, 0);
        if (fd >= 0) {
            loft_sock_cloexec(fd);
            if (loft_sock_connect_within(fd, (struct sockaddr *) &addr, al, 3000)) break;
            close(fd);
            fd = -1;
        }
        if (loft_now_us() >= deadline) {
            free(h);
            return 0;
        }
        loft_eh_sleep_us(100000);
    }
    loft_sock_timeout(fd, 5000);
    char req[1024];
    int n = snprintf(req, sizeof req,
                     "GET /ws HTTP/1.1\r\nHost: %s:%u\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n"
                     "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n",
                     h, (unsigned) (uint16_t) port);
    free(h);
    loft_text head = { 0, 0, 0 };
    if (n <= 0 || (size_t) n >= sizeof req || !loft_eh_write_all(fd, req, (size_t) n) || !loft_eh_read_head(fd, &head)) {
        free(head.p);
        close(fd);
        return 0;
    }
    loft_push(&head, "", 1);
    if (!strstr(head.p, " 101 ")) {
        free(head.p);
        close(fd);
        return 0;
    }
    head.len--;
    const char *v;
    size_t vn;
    char cookie[64] = { 0 };
    if (loft_http_header(&head, "x-loft-udp", &v, &vn) && vn < sizeof cookie) memcpy(cookie, v, vn);
    free(head.p);
    loft_sock_timeout(fd, 20);
    loft_ehc_reset(tick_us);
    loft_ehc.conn = fd;
    memcpy(loft_ehc.cookie, cookie, sizeof cookie);
    if (cookie[0]) {
        int u = socket(family, SOCK_DGRAM, 0);
        if (u >= 0) {
            loft_sock_cloexec(u);
            if (connect(u, (struct sockaddr *) &addr, al) == 0) {
                loft_sock_blocking(u, 0);
                loft_ehc.udp = u;
            } else {
                close(u);
            }
        }
    }
    loft_eh_push(&loft_ehc.events, 0, 0, 0, "", 0);
    loft_ehc.ctl = loft_ehc_ctl_listener();
    loft_eh_touch_ready("connected", 1);
    return 1;
#else
    (void) host;
    (void) port;
    (void) tick_us;
    return 0;
#endif
}

static void loft_ehc_lost(void) {
    loft_ehc.alive = 0;
    loft_ehc.udp_bound = 0;
    loft_eh_push(&loft_ehc.events, 0, 2, 0, "", 0);
}

/* `pump_client`: the control endpoint, HTTP completions, the server's WS frames (an `S:`-framed
 * keyframe or a plain sync-class frame conflates, the rest queues), its datagrams (the ack, the
 * sync traffic — `LOFT_UDP_DROP_NTH=N` drops every Nth, for measurement), then the hello and
 * keepalive cadences. */
static int64_t loft_kernel_client_pump(void) {
    if (!loft_ehc.up) return 0;
    int64_t added = 0;
#if defined(LOFT_POSIX_HOST)
    loft_ehc_pump_ctl();
#endif
    loft_eh_drain_http(&loft_ehc.events);
#if defined(LOFT_POSIX_HOST)
    while (loft_ehc.alive && loft_ehc.conn >= 0) {
        loft_text msg = { 0, 0, 0 };
        int got = loft_eh_read_frame(loft_ehc.conn, &msg);
        if (got == 0) {
            free(msg.p);
            break;
        }
        if (got == 2) {
            free(msg.p);
            loft_ehc_lost();
            added++;
            break;
        }
        int64_t seq;
        const char *body;
        size_t bl;
        if (loft_eh_seq_frame(msg.p, msg.len, &seq, &body, &bl)) {
            loft_eh_conflate(&loft_ehc.slots, seq, body, bl);
        } else if (loft_eh_is_sync(msg.p, msg.len)) {
            loft_eh_conflate_ws(&loft_ehc.slots, msg.p, msg.len);
        } else {
            loft_eh_push(&loft_ehc.events, 0, 1, 0, msg.p, msg.len);
            added++;
        }
        free(msg.p);
    }
    if (loft_ehc.udp >= 0) {
        static int64_t drop_nth = -1;
        static uint64_t received = 0;
        if (drop_nth < 0) {
            const char *v = getenv("LOFT_UDP_DROP_NTH");
            int64_t d = 0;
            drop_nth = v && loft_eh_int(v, strlen(v), &d) && d > 0 ? d : 0;
        }
        char buf[2048];
        for (;;) {
            ssize_t len = recv(loft_ehc.udp, buf, sizeof buf, MSG_DONTWAIT);
            if (len < 0) break;
            if (!loft_utf8_valid((const unsigned char *) buf, (size_t) len)) continue;
            int64_t seq;
            const char *body;
            size_t bl;
            if (len >= 2 && buf[0] == 'A' && buf[1] == ':') {
                loft_ehc.udp_bound = 1;
            } else if (loft_eh_seq_frame(buf, (size_t) len, &seq, &body, &bl)) {
                if (drop_nth > 0 && ++received % (uint64_t) drop_nth == 0) continue;
                loft_eh_conflate(&loft_ehc.slots, seq, body, bl);
            }
        }
        int64_t now = loft_ehc_now();
        if (!loft_ehc.udp_bound && loft_ehc.cookie[0] && now - loft_ehc.last_hello > LOFT_EH_HELLO_US) {
            char hello[80];
            int hn = snprintf(hello, sizeof hello, "H:%s", loft_ehc.cookie);
            send(loft_ehc.udp, hello, (size_t) hn, 0);
            loft_ehc.last_hello = now;
        }
        if (loft_ehc.udp_bound && now - loft_ehc.last_keepalive > LOFT_EH_KEEPALIVE_US) {
            send(loft_ehc.udp, "K:", 2, 0);
            loft_ehc.last_keepalive = now;
        }
    }
#endif
    return added;
}

static uint8_t loft_kernel_client_alive(void) { return loft_ehc.up && loft_ehc.alive; }
static void loft_kernel_client_stop(void) {
    if (loft_ehc.up) loft_ehc.alive = 0;
}
static uint8_t loft_kernel_client_next_event(void) {
    return loft_ehc.up && loft_eh_pop(&loft_ehc.events, &loft_ehc.last);
}
static int64_t loft_kernel_client_event_kind(void) { return loft_ehc.up ? loft_ehc.last.kind : -1; }
static int64_t loft_kernel_client_event_cid(void) { return loft_ehc.up ? loft_ehc.last.cid : -1; }
static int64_t loft_kernel_client_event_status(void) { return loft_ehc.up ? loft_ehc.last.status : 0; }
static loft_text loft_kernel_client_event_payload(void) {
    loft_text t = { 0, 0, 0 };
    if (loft_ehc.up) t = loft_eh_copy(&loft_ehc.last.payload);
    return t;
}
static uint8_t loft_kernel_client_tick_due(void) {
    return loft_ehc.up && loft_eh_tick_due(loft_ehc_now(), loft_ehc.tick, &loft_ehc.last_tick);
}
static void loft_kernel_client_idle(int64_t max_us) {
    loft_eh_sleep_us(loft_ehc.up ? loft_eh_idle_for(max_us, loft_ehc_now(), loft_ehc.tick, loft_ehc.last_tick)
                                 : (max_us > 0 ? max_us : 0));
}

/* `client_send`: one seq across both carriers for sync sends; a datagram once the path is acked,
 * else a masked WS frame.  A failed write loses the server. */
static uint8_t loft_kernel_client_send(loft_str msg) {
    if (!loft_ehc.up || !loft_ehc.alive) return 0;
    int sync = loft_eh_is_sync(msg.p, msg.len);
    if (sync) loft_ehc.out_seq += 1;
#if defined(LOFT_POSIX_HOST)
    if (sync && loft_ehc.udp_bound && loft_ehc.udp >= 0) {
        loft_text d = { 0, 0, 0 };
        char seq[32];
        snprintf(seq, sizeof seq, "S:%lld:", (long long) loft_ehc.out_seq);
        loft_push_cstr(&d, seq);
        loft_push(&d, msg.p, msg.len);
        uint8_t ok = d.len <= LOFT_EH_UDP_MAX && send(loft_ehc.udp, d.p, d.len, 0) >= 0;
        free(d.p);
        return ok;
    }
    if (loft_ehc.conn < 0) return 0;
    if (!loft_eh_write_frame(loft_ehc.conn, 0x1, msg.p, msg.len, 1)) {
        loft_ehc_lost();
        return 0;
    }
    return 1;
#else
    return 0;
#endif
}

static uint8_t loft_kernel_client_sync_next(void) {
    if (!loft_ehc.up) return 0;
    for (size_t i = 0; i < loft_ehc.slots.n; i++) {
        loft_eh_slot *s = &loft_ehc.slots.v[i];
        if (!s->dirty) continue;
        s->dirty = 0;
        loft_ehc.sync_seq = s->seq;
        loft_ehc.sync_payload.len = 0;
        loft_push(&loft_ehc.sync_payload, s->payload.p, s->payload.len);
        return 1;
    }
    return 0;
}
static int64_t loft_kernel_client_sync_seq(void) { return loft_ehc.up ? loft_ehc.sync_seq : -1; }
static loft_text loft_kernel_client_sync_payload(void) {
    loft_text t = { 0, 0, 0 };
    if (loft_ehc.up) t = loft_eh_copy(&loft_ehc.sync_payload);
    return t;
}
static uint8_t loft_kernel_client_udp_bound(void) { return loft_ehc.up && loft_ehc.udp_bound; }
static void loft_kernel_client_frame(void) {}

/* `default_host`: LOFT_HOST, else loopback. */
static loft_text loft_kernel_default_host(void) {
    loft_text t = { 0, 0, 0 };
    const char *h = getenv("LOFT_HOST");
    loft_push_cstr(&t, h ? h : "127.0.0.1");
    return t;
}

/* ── The swap step, with no swap to drive: a compiled C build does not hand its world over, so
 * the step serves (0) and this process never retires. ── */
static int64_t loft_kernel_swap_step(void) { return 0; }
static uint8_t loft_swap_retired(void) { return 0; }
/* `live_flip`: no live tier to flip a function into. */
static uint8_t loft_live_flip(loft_str name, uint8_t on) {
    (void) name;
    (void) on;
    return 0;
}

/* ── `http_fetch` ── */

#if defined(LOFT_POSIX_HOST)
typedef struct { int64_t id; char *method, *url, *body, *headers; } loft_eh_req;

/* One HTTP/1.1 exchange on `url` over the runtime's client (`loft_https.h`): the status and the
 * body, or -1 and why.  `hops` redirects are followed, as the Rust kernel's client follows them. */
static int64_t loft_eh_http(const char *method, const char *url, const char *body, const char *headers,
                            loft_text *out, int hops) {
    loft_text lines = { 0, 0, 0 };
    for (const char *l = headers; *l;) {
        const char *e = strchr(l, '\n');
        size_t n = e ? (size_t) (e - l) : strlen(l);
        const char *c = (const char *) memchr(l, ':', n);
        if (c) {
            const char *k = l, *v = c + 1;
            size_t kn = (size_t) (c - l), vn = n - kn - 1;
            loft_eh_trim(&k, &kn);
            loft_eh_trim(&v, &vn);
            loft_push(&lines, k, kn);
            loft_push_cstr(&lines, ": ");
            loft_push(&lines, v, vn);
            loft_push_cstr(&lines, "\r\n");
        }
        l += n + (e ? 1 : 0);
    }
    loft_push(&lines, "", 1);
    loft_http h;
    char err[1024];
    int ok = loft_http_open(&h, method, url, lines.p, body, hops, err, sizeof err);
    free(lines.p);
    if (!ok) {
        loft_push_cstr(out, err);
        return -1;
    }
    loft_text raw = { 0, 0, 0 };
    char buf[65536];
    long r;
    while ((r = loft_http_read(&h, buf, sizeof buf)) > 0) loft_push(&raw, buf, (size_t) r);
    int64_t status = h.status;
    loft_http_close(&h);
    /* A body that is not UTF-8 reads as empty, as the Rust client's `into_string` gives up. */
    if (loft_utf8_valid((const unsigned char *) raw.p, raw.len)) loft_push(out, raw.p, raw.len);
    free(raw.p);
    return status;
}

static void *loft_eh_http_worker(void *arg) {
    loft_eh_req *q = (loft_eh_req *) arg;
    loft_text body = { 0, 0, 0 };
    int64_t status = loft_eh_http(q->method, q->url, q->body, q->headers, &body, 5);
    loft_eh_park(q->id, status, body.p, body.len);
    free(body.p);
    free(q->method);
    free(q->url);
    free(q->body);
    free(q->headers);
    free(q);
    return 0;
}
#endif

/* `http_fetch(method, url, body, headers) -> integer`: the request id at once; the completion a
 * kind-3 event (cid = the id, status = the HTTP status or negative, payload = the body). */
static int64_t loft_kernel_http_fetch(loft_str method, loft_str url, loft_str body, loft_str headers) {
    int64_t id = loft_eh_http_next++;
#if defined(LOFT_POSIX_HOST) && defined(LOFT_EH_THREADS)
    loft_eh_req *q = (loft_eh_req *) malloc(sizeof *q);
    if (!q) { fputs("loft: out of memory\n", stderr); abort(); }
    q->id = id;
    q->method = loft_cstr_own(method);
    q->url = loft_cstr_own(url);
    q->body = loft_cstr_own(body);
    q->headers = loft_cstr_own(headers);
    pthread_t t;
    pthread_attr_t at;
    pthread_attr_init(&at);
    pthread_attr_setdetachstate(&at, PTHREAD_CREATE_DETACHED);
    if (pthread_create(&t, &at, loft_eh_http_worker, q) != 0) {
        static const char why[] = "cannot start the request's worker thread";
        loft_eh_park(id, -1, why, sizeof why - 1);
        free(q->method);
        free(q->url);
        free(q->body);
        free(q->headers);
        free(q);
    }
    pthread_attr_destroy(&at);
#else
    (void) method;
    (void) url;
    (void) body;
    (void) headers;
    static const char why[] = "http support not compiled in (this build has no threads)";
    loft_eh_park(id, -2, why, sizeof why - 1);
#endif
    return id;
}
#endif
