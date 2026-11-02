/* ── lib/engine_host: the kernel's mechanics (src/engine_host.rs) ──────────────────────────────
 * The natives behind `lib/engine_host`, which owns no game meaning: the socket pump (non-blocking,
 * a peek before every frame read), the event queue, drift-free ticks, class-routed send and
 * broadcast, and the state-sync channel over UDP — one datagram socket on the listener's port, a
 * per-connection cookie handed over in the 101 response (`X-Loft-UDP`), `H:<cookie>` binding a
 * source address to its connection, `S:<seq>:<payload>` conflating to the newest per sender, kind
 * and (for a keyed kind) entity.  Both roles share one queue and one conflation machinery: the
 * listener (`run`) and the connector (`run_client`, or a transportless local kernel).  The
 * `D!:` debug control channel answers as a build with no live tier does, but for the rebuild
 * and the swap, which a C build carries (below).  `http_fetch` returns at
 * once and completes as a kind-3 event, its request on a worker thread.  Every host with sockets
 * (`loft_sock.h`: POSIX and Windows); elsewhere the kernel cannot listen or connect.  Compiled in
 * for a unit that reaches a kernel native (LOFT_ENGINE). */
#ifdef LOFT_ENGINE
#include <ctype.h>
#if defined(LOFT_SOCKETS) && !defined(LOFT_NO_THREADS) && !defined(LOFT_STRICT)
#if !defined(_WIN32)
#include <pthread.h>
#endif
#define LOFT_EH_THREADS 1
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
/* The lock between a worker parking its completion and the pump draining them. */
#if defined(LOFT_EH_THREADS) && defined(_WIN32)
static SRWLOCK loft_eh_done_lock = SRWLOCK_INIT;
#define LOFT_EH_LOCK() AcquireSRWLockExclusive(&loft_eh_done_lock)
#define LOFT_EH_UNLOCK() ReleaseSRWLockExclusive(&loft_eh_done_lock)
#elif defined(LOFT_EH_THREADS)
static pthread_mutex_t loft_eh_done_lock = PTHREAD_MUTEX_INITIALIZER;
#define LOFT_EH_LOCK() pthread_mutex_lock(&loft_eh_done_lock)
#define LOFT_EH_UNLOCK() pthread_mutex_unlock(&loft_eh_done_lock)
#else
#define LOFT_EH_LOCK() ((void) 0)
#define LOFT_EH_UNLOCK() ((void) 0)
#endif

static void loft_eh_park(int64_t id, int64_t status, const char *body, size_t n) {
    LOFT_EH_LOCK();
    loft_eh_dones = (loft_eh_done *) realloc(loft_eh_dones, (loft_eh_ndone + 1) * sizeof *loft_eh_dones);
    if (!loft_eh_dones) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_eh_done *d = &loft_eh_dones[loft_eh_ndone++];
    d->id = id;
    d->status = status;
    d->body.p = 0;
    d->body.len = d->body.cap = 0;
    loft_push(&d->body, body, n);
    LOFT_EH_UNLOCK();
}

static void loft_eh_drain_http(loft_eh_queue *q) {
    LOFT_EH_LOCK();
    for (size_t i = 0; i < loft_eh_ndone; i++) {
        loft_eh_push(q, loft_eh_dones[i].id, 3, loft_eh_dones[i].status, loft_eh_dones[i].body.p,
                     loft_eh_dones[i].body.len);
        free(loft_eh_dones[i].body.p);
    }
    loft_eh_ndone = 0;
    LOFT_EH_UNLOCK();
}

#if defined(LOFT_SOCKETS)
/* ── Socket mechanics ── */

/* Every byte of `p`, or 0. */
static int loft_eh_write_all(int fd, const char *p, size_t n) {
    while (n > 0) {
#if defined(MSG_NOSIGNAL)
        long w = loft_sock_send(fd, p, n, MSG_NOSIGNAL);
#else
        long w = loft_sock_send(fd, p, n, 0);
#endif
        if (w < 0 && loft_sock_interrupted()) continue;
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
        long r = loft_sock_recv(fd, p, n, 0);
        if (r < 0 && loft_sock_interrupted()) continue;
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
        long r = loft_sock_recv(fd, &b, 1, 0);
        if (r < 0 && loft_sock_interrupted()) continue;
        if (r != 1) return 0;
        loft_push(head, &b, 1);
        if (head->len > 16 * 1024) return 0;
    }
    return 1;
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
        loft_ws_accept_key(v, vn, accept);
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
    long peeked = loft_sock_peek(fd, hdr, 2);
    if (peeked == 0 || peeked == -2) return 2;
    if (peeked < 0) return 0;
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

/* glibc declares SO_REUSEPORT only under `_DEFAULT_SOURCE`, which the unit's ISO-and-POSIX feature
 * macros do not ask for; the kernel's own header defines it per architecture. */
#if defined(__linux__) && !defined(SO_REUSEPORT)
#include <asm/socket.h>
#endif

/* A socket bound on 0.0.0.0:`port` for a hand-over: close-on-exec, SO_REUSEADDR (TCP) and
 * SO_REUSEPORT, so a swap's new build binds while the old one serves.  Windows has no shared
 * port (`platform::bind_tcp_handover` binds plainly there): its listener is exclusive, as the
 * runtime's other listeners are.  -1 on failure. */
static int loft_eh_bind(int64_t port, int type) {
    int fd = loft_sock_open(AF_INET, type, 0);
    if (fd < 0) return -1;
    loft_sock_cloexec(fd);
#if defined(_WIN32)
    if (type == SOCK_STREAM) loft_sock_listener_addr(fd);
#else
    if (type == SOCK_STREAM) loft_sock_flag(fd, SOL_SOCKET, SO_REUSEADDR, 1);
#if defined(SO_REUSEPORT)
    loft_sock_flag(fd, SOL_SOCKET, SO_REUSEPORT, 1);
#endif
#endif
    if (type == SOCK_DGRAM) loft_sock_udp_quiet(fd);
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_port = htons((uint16_t) port);
    int bound = type == SOCK_STREAM ? loft_sock_bind_listen(fd, (struct sockaddr *) &a, sizeof a, 128)
                                    : loft_sock_bind(fd, (struct sockaddr *) &a, sizeof a);
    if (bound != 0) {
        loft_sock_close(fd);
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
#if defined(_WIN32)
/* `RtlGenRandom`, as `ntsecapi.h` declares it (that header wants types `windows.h` lacks). */
BOOLEAN WINAPI SystemFunction036(PVOID buffer, ULONG length);
#if defined(_MSC_VER)
#pragma comment(lib, "advapi32.lib")
#endif
#endif

static void loft_eh_cookie(size_t cid, char out[17]) {
    uint64_t x = 0;
#if defined(_WIN32)
    if (!SystemFunction036(&x, (ULONG) sizeof x)) x = 0;
    uint64_t pid = (uint64_t) GetCurrentProcessId();
#else
    int fd = open("/dev/urandom", O_RDONLY);
    if (fd >= 0) {
        long r = (long) read(fd, &x, sizeof x);
        (void) r;
        close(fd);
    }
    uint64_t pid = (uint64_t) getpid();
#endif
    x ^= (uint64_t) loft_now_us() * 0x9E3779B97F4A7C15ull ^ (uint64_t) cid * 0xBF58476D1CE4E5B9ull ^ pid;
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

static uint8_t loft_rebuild_start(void);
static int64_t loft_rebuild_status(void);
static loft_text loft_kernel_rebuild_artifact(void);
static const char *loft_rebuild_failure(void);
static uint8_t loft_swap_start(loft_str artifact);

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
        loft_push_cstr(r, loft_rebuild_start() ? "D:rebuild started" : "D:rebuild refused");
    } else if (LOFT_VERB("rebuild?")) {
        /* A failed build says why, so the editor shows it beside the build it refused. */
        char b[640];
        int64_t st = loft_rebuild_status();
        if (st == 3) snprintf(b, sizeof b, "D:rebuild 3 %s", loft_rebuild_failure());
        else snprintf(b, sizeof b, "D:rebuild %lld", (long long) st);
        loft_push_cstr(r, b);
    } else if (LOFT_VERB("swap")) {
        loft_text target = { 0, 0, 0 };
        if (an == 0 || (an == 4 && memcmp(arg, "auto", 4) == 0)) target = loft_kernel_rebuild_artifact();
        else loft_push(&target, arg, an);
        uint8_t ok = loft_swap_start(loft_view(&target));
        loft_text_free(&target);
        loft_push_cstr(r, ok ? "D:swap true" : "D:swap false");
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

#if defined(LOFT_SOCKETS)
static void loft_ehk_disconnect(size_t cid);

/* Every socket either role still holds, closed when the program exits — as the Rust kernel's
 * sockets close when its state drops.  A socket the process leaves open is closed by the OS, and
 * Windows closes it with a reset, which discards whatever the peer had not read yet: a seat's
 * last message before `client_stop` never reached its listener.  A plain close sends it, then
 * the FIN. */
static void loft_eh_close_fd(int *fd) {
    if (*fd >= 0) loft_sock_close(*fd);
    *fd = -1;
}

/* Only a role that is up: the other's descriptors are still the static's zeros, and 0 is stdin. */
static void loft_eh_close_all(void) {
    if (loft_ehk.up) {
        loft_eh_close_fd(&loft_ehk.listener);
        loft_eh_close_fd(&loft_ehk.udp);
        for (size_t i = 0; i < loft_ehk.nconns; i++) loft_eh_close_fd(&loft_ehk.conns[i].fd);
    }
    if (loft_ehc.up) {
        loft_eh_close_fd(&loft_ehc.conn);
        loft_eh_close_fd(&loft_ehc.udp);
        loft_eh_close_fd(&loft_ehc.ctl);
        for (size_t i = 0; i < loft_ehc.nctl; i++) loft_eh_close_fd(&loft_ehc.ctl_conns[i]);
    }
}

static void loft_eh_close_at_exit(void) {
    static int armed = 0;
    if (armed) return;
    armed = 1;
    atexit(loft_eh_close_all);
}

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
            ok = loft_sock_sendto(loft_ehk.udp, d.p, d.len, (struct sockaddr *) &c->addr, c->alen) >= 0;
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
    loft_sock_close(loft_ehk.conns[cid].fd);
    loft_eh_conn_reset(&loft_ehk.conns[cid]);
    loft_eh_push(&loft_ehk.events, (int64_t) cid, 2, 0, "", 0);
}

static int loft_eh_loopback(int fd) {
    struct sockaddr_storage a;
    socklen_t al = sizeof a;
    if (loft_sock_name(fd, 1, (struct sockaddr *) &a, &al) != 0) return 0;
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
        long len = loft_sock_recvfrom(loft_ehk.udp, buf, sizeof buf, (struct sockaddr *) &from, &fl);
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
                        loft_sock_sendto(loft_ehk.udp, ack, (size_t) an, (struct sockaddr *) &from, fl);
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
#if defined(LOFT_SOCKETS)
    int fd = loft_eh_bind(port, SOCK_STREAM);
    if (fd < 0) return 0;
    int udp = loft_eh_bind(port, SOCK_DGRAM);
    if (udp < 0) {
        int why = loft_sock_error();
        fprintf(stderr, "engine_host: UDP bind on %lld failed (%s (os error %d)); state-sync rides WS\n", (long long) port,
                strerror(why), why);
    }
    if (loft_ehk.up) {
        if (loft_ehk.listener >= 0) loft_sock_close(loft_ehk.listener);
        if (loft_ehk.udp >= 0) loft_sock_close(loft_ehk.udp);
        for (size_t i = 0; i < loft_ehk.nconns; i++) {
            if (loft_ehk.conns[i].fd >= 0) loft_sock_close(loft_ehk.conns[i].fd);
            loft_eh_conn_reset(&loft_ehk.conns[i]);
            free(loft_ehk.conns[i].debug_name);
        }
        loft_eh_queue_free(&loft_ehk.events);
    }
    free(loft_ehk.conns);
    loft_ehk.conns = 0;
    loft_ehk.nconns = 0;
    loft_eh_close_at_exit();
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
#if defined(LOFT_SOCKETS)
    for (;;) {
        int fd = loft_sock_accept(loft_ehk.listener);
        if (fd < 0) break;
        loft_sock_cloexec(fd);
        size_t cid = 0;
        while (cid < loft_ehk.nconns && loft_ehk.conns[cid].fd >= 0) cid++;
        char cookie[17] = { 0 };
        if (loft_ehk.udp >= 0) loft_eh_cookie(cid, cookie);
        if (!loft_eh_ws_upgrade(fd, cookie)) {
            loft_sock_close(fd);
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
#if defined(LOFT_SOCKETS)
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
#if defined(LOFT_SOCKETS)
    if (!loft_ehk.up) return 0;
    int sync = loft_eh_is_sync(msg.p, msg.len);
    for (size_t cid = 0; cid < loft_ehk.nconns; cid++)
        if (loft_ehk.conns[cid].fd >= 0 && loft_ehk_deliver(cid, msg.p, msg.len, sync)) sent++;
#else
    (void) msg;
#endif
    return sent;
}

/* Sleep `us`.  Windows' `Sleep` counts whole milliseconds on a ~15.6 ms clock, which would round
 * a tick's sub-frame wait to a frame or to a spin; a high-resolution waitable timer (Windows 10
 * 1803+, what `std::thread::sleep` uses) keeps it, and `Sleep` rounded UP is the fallback. */
static void loft_eh_sleep_us(int64_t us) {
    if (us <= 0) return;
#if defined(_WIN32)
#ifndef CREATE_WAITABLE_TIMER_HIGH_RESOLUTION
#define CREATE_WAITABLE_TIMER_HIGH_RESOLUTION 0x00000002
#endif
    static HANDLE timer = 0;
    static int tried = 0;
    if (!tried) {
        tried = 1;
        timer = CreateWaitableTimerExW(0, 0, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS);
    }
    LARGE_INTEGER due;
    due.QuadPart = -(LONGLONG) us * 10;
    if (timer && SetWaitableTimer(timer, &due, 0, 0, 0, FALSE)) {
        WaitForSingleObject(timer, INFINITE);
    } else {
        Sleep((DWORD) ((us + 999) / 1000));
    }
#else
    struct timespec ts = { (time_t) (us / 1000000), (long) (us % 1000000) * 1000 };
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {}
#endif
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
#if defined(LOFT_SOCKETS)
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

#if defined(LOFT_SOCKETS)
/* The client's loopback debug-control listener, announced on stdout; -1 unless
 * LOFT_DEBUG_CONTROL=1. */
static int loft_ehc_ctl_listener(void) {
    if (!loft_eh_debug_on()) return -1;
    int fd = loft_sock_open(AF_INET, SOCK_STREAM, 0);
    if (fd < 0) return -1;
    loft_sock_cloexec(fd);
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    socklen_t al = sizeof a;
    if (loft_sock_bind_listen(fd, (struct sockaddr *) &a, sizeof a, 128) != 0 ||
        loft_sock_name(fd, 0, (struct sockaddr *) &a, &al) != 0) {
        loft_sock_close(fd);
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
        int fd = loft_sock_accept(loft_ehc.ctl);
        if (fd < 0) break;
        loft_sock_cloexec(fd);
        if (!loft_eh_ws_upgrade(fd, "")) {
            loft_sock_close(fd);
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
                loft_sock_close(loft_ehc.ctl_conns[i]);
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
#if defined(LOFT_SOCKETS)
    if (loft_ehc.up) {
        if (loft_ehc.conn >= 0) loft_sock_close(loft_ehc.conn);
        if (loft_ehc.udp >= 0) loft_sock_close(loft_ehc.udp);
        if (loft_ehc.ctl >= 0) loft_sock_close(loft_ehc.ctl);
        for (size_t i = 0; i < loft_ehc.nctl; i++)
            if (loft_ehc.ctl_conns[i] >= 0) loft_sock_close(loft_ehc.ctl_conns[i]);
    }
#endif
    free(loft_ehc.ctl_conns);
    loft_ehc.ctl_conns = 0;
    loft_ehc.nctl = 0;
    loft_eh_queue_free(&loft_ehc.events);
    loft_eh_slots_free(&loft_ehc.slots);
#if defined(LOFT_SOCKETS)
    loft_eh_close_at_exit();
#endif
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
#if defined(LOFT_SOCKETS)
    loft_ehc.ctl = loft_ehc_ctl_listener();
#endif
    loft_eh_touch_ready("connected", 0);
    return 1;
}

/* `kernel_connect`: dial (retried across a swap's gap, five seconds), upgrade with the RFC's
 * sample key (anti-cache, not auth), take the `X-Loft-UDP` cookie, prepare the UDP socket.  A
 * kind-0 event marks the connect, as the listener's does. */
static uint8_t loft_kernel_connect(loft_str host, int64_t port, int64_t tick_us) {
#if defined(LOFT_SOCKETS)
    char *h = loft_cstr_own(host), ps[16];
    snprintf(ps, sizeof ps, "%u", (unsigned) (uint16_t) port);
    struct addrinfo hints, *res = 0;
    loft_sock_ready();
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
        fd = loft_sock_open(family, SOCK_STREAM, 0);
        if (fd >= 0) {
            loft_sock_cloexec(fd);
            if (loft_sock_connect_within(fd, (struct sockaddr *) &addr, al, 3000)) break;
            loft_sock_close(fd);
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
        loft_sock_close(fd);
        return 0;
    }
    loft_push(&head, "", 1);
    if (!strstr(head.p, " 101 ")) {
        free(head.p);
        loft_sock_close(fd);
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
        int u = loft_sock_open(family, SOCK_DGRAM, 0);
        if (u >= 0) {
            loft_sock_cloexec(u);
            loft_sock_udp_quiet(u);
            if (loft_sock_connect_to(u, (struct sockaddr *) &addr, al) == 0) {
                loft_sock_blocking(u, 0);
                loft_ehc.udp = u;
            } else {
                loft_sock_close(u);
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
#if defined(LOFT_SOCKETS)
    loft_ehc_pump_ctl();
#endif
    loft_eh_drain_http(&loft_ehc.events);
#if defined(LOFT_SOCKETS)
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
            long len = loft_sock_recv(loft_ehc.udp, buf, sizeof buf, 0);
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
            loft_sock_send(loft_ehc.udp, hello, (size_t) hn, 0);
            loft_ehc.last_hello = now;
        }
        if (loft_ehc.udp_bound && now - loft_ehc.last_keepalive > LOFT_EH_KEEPALIVE_US) {
            loft_sock_send(loft_ehc.udp, "K:", 2, 0);
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
#if defined(LOFT_SOCKETS)
    if (sync && loft_ehc.udp_bound && loft_ehc.udp >= 0) {
        loft_text d = { 0, 0, 0 };
        char seq[32];
        snprintf(seq, sizeof seq, "S:%lld:", (long long) loft_ehc.out_seq);
        loft_push_cstr(&d, seq);
        loft_push(&d, msg.p, msg.len);
        uint8_t ok = d.len <= LOFT_EH_UDP_MAX && loft_sock_send(loft_ehc.udp, d.p, d.len, 0) >= 0;
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

/* ── The live build swap (@PLN18 08-S4/S5, src/engine_host.rs `swap_*`, src/live_rebuild.rs) ──
 * `swap_world` names the world record: it becomes the snapshot root, and in a process booted AS a
 * swap target (`LOFT_RESUME`) the previous build's world is restored into it, through the store
 * converter when its layout moved.  `swap_start` asks for a swap; the run loop's swap step writes
 * the snapshot at the next frame boundary, boots the new build detached with the snapshot and a
 * ready file, keeps mechanics alive while meaning waits, and retires once the new build serves
 * (it touches the ready file when it listens or connects) — or rolls back, the old build never
 * having stopped.  The background rebuild runs the loft driver's own `--check --native-c` as a
 * child while this build serves.  Processes are POSIX here; elsewhere every request refuses. */

/* A world type's JSON render, for a world that shares its store (the snapshot then goes as JSON). */
typedef void (*loft_swap_show_fn)(loft_text *s, loft_ref r);

#define LOFT_SW_IDLE 0
#define LOFT_SW_REQUESTED 1
#define LOFT_SW_WAITING 2
#define LOFT_SW_DONE 3

static struct {
    int registered;
    loft_ref w;
    uint16_t tp;
    const loft_sidecar *sc;
    loft_swap_show_fn show;
    char *layout; /* the world's layout as sidecar text, for a rebuild to check the next build */
    int phase;
    char *artifact, *ready, *snap;
    long pid;
    int64_t deadline;
} loft_sw = { 0, { 0xFFFF, 0, 0 }, 0, 0, 0, 0, LOFT_SW_IDLE, 0, 0, 0, -1, 0 };

static char *loft_sw_join(const char *a, const char *b) {
    size_t n = strlen(a), m = strlen(b);
    char *c = (char *) malloc(n + m + 1);
    if (!c) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(c, a, n);
    memcpy(c + n, b, m + 1);
    return c;
}

static char *loft_sw_dup(const char *p, size_t n) {
    char *c = (char *) malloc(n + 1);
    if (!c) { fputs("loft: out of memory\n", stderr); abort(); }
    memcpy(c, p, n);
    c[n] = 0;
    return c;
}

static int loft_sw_write(const char *path, const char *p, size_t n) {
    FILE *f = fopen(path, "wb");
    if (!f) return 0;
    int ok = fwrite(p, 1, n, f) == n;
    return (fclose(f) == 0) && ok;
}

/* A whole file, malloc'd and NUL-terminated, or 0. */
static char *loft_sw_read(const char *path, size_t *len) {
    FILE *f = fopen(path, "rb");
    if (!f) return 0;
    size_t cap = 4096, n = 0, got;
    char *b = (char *) malloc(cap + 1);
    if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
    while ((got = fread(b + n, 1, cap - n, f)) > 0) {
        n += got;
        if (n == cap) {
            cap *= 2;
            b = (char *) realloc(b, cap + 1);
            if (!b) { fputs("loft: out of memory\n", stderr); abort(); }
        }
    }
    fclose(f);
    b[n] = 0;
    if (len) *len = n;
    return b;
}

static int loft_sw_exists(const char *path) {
    FILE *f = path && *path ? fopen(path, "rb") : 0;
    if (f) fclose(f);
    return f != 0;
}

/* `std::env::temp_dir`: TMPDIR, else /tmp. */
static const char *loft_sw_tmp(void) {
    const char *t = getenv("TMPDIR");
    return t && *t ? t : "/tmp";
}

/* A snapshot and the layout sidecar a store image has beside it. */
static void loft_sw_remove_snapshot(const char *snap) {
    if (!snap) return;
    char *side = loft_sw_join(snap, ".dschema");
    remove(snap);
    remove(side);
    free(side);
}

/* `swap_world(w)`: register `w` as the snapshot root and, in a swap target, restore the previous
 * build's world into it in place; 1 when a restore happened. */
static uint8_t loft_swap_world(loft_ref w, const loft_sidecar *sc, loft_swap_show_fn show) {
    uint16_t kt = w.store_nr < loft_nstores ? loft_stores[w.store_nr].known : 0xFFFF;
    if (w.store_nr == 0xFFFF || kt == 0xFFFF) {
        fputs("loft-swap: swap_world got a record with no known type; swaps disabled\n", stderr);
        return 0;
    }
    loft_sw.registered = 1;
    loft_sw.w = w;
    loft_sw.tp = kt;
    loft_sw.sc = sc && sc->tp == kt ? sc : 0;
    loft_sw.show = show;
    /* The layout a live rebuild checks the next instance against; the slot line names this store. */
    free(loft_sw.layout);
    loft_sw.layout = 0;
    if (loft_sw.sc) {
        loft_text t = { 0, 0, 0 };
        char slot[32];
        snprintf(slot, sizeof slot, "slot=%u\n", (unsigned) w.store_nr);
        loft_push_cstr(&t, loft_sw.sc->before_slot);
        loft_push_cstr(&t, slot);
        loft_push_cstr(&t, loft_sw.sc->after_slot);
        loft_sw.layout = loft_cstr_own(loft_view(&t));
        loft_text_free(&t);
        /* An editor serving this game reads it to warn about a change the next build could not
         * carry the world across; the first line names this process. */
        const char *out = getenv("LOFT_WORLD_LAYOUT_OUT");
        if (out) {
            loft_text o = { 0, 0, 0 };
            char head[64];
#if defined(LOFT_POSIX) && !defined(__wasi__)
            snprintf(head, sizeof head, "loft-world pid=%ld\n", (long) getpid());
#else
            snprintf(head, sizeof head, "loft-world pid=0\n");
#endif
            loft_push_cstr(&o, head);
            loft_push_cstr(&o, loft_sw.layout);
            (void) loft_sw_write(out, o.p, o.len);
            loft_text_free(&o);
        }
    }
    const char *snap = getenv("LOFT_RESUME");
    if (!snap) return 0;
    size_t sl = strlen(snap);
    if (sl > 6 && strcmp(snap + sl - 6, ".store") == 0) {
        /* A world handed over as a store image loads through the converter; a change it refuses
         * ends THIS build before it serves, which the old build reads as a failed swap. */
        if (loft_sw.sc && loft_image_load(w, loft_lit(snap, sl), loft_sw.sc, 0)) {
            fprintf(stderr, "loft-swap: world restored from %s\n", snap);
            return 1;
        }
        if (!loft_sw.sc) fprintf(stderr, "store_load: refusing %s \xe2\x80\x94 this build has no layout for the world's type.\n", snap);
        fputs("loft-swap: this build cannot take over the running world (the reason is above); the running build "
              "keeps serving\n",
              stderr);
        exit(1);
    }
    size_t n = 0;
    char *json = loft_sw_read(snap, &n);
    if (!json) {
        fprintf(stderr, "loft-swap: LOFT_RESUME set but %s unreadable; starting fresh\n", snap);
        return 0;
    }
#ifdef LOFT_UPGRADE
    loft_ref parsed = loft_record_from_text(loft_up_ptypes(), loft_lit(json, n), kt);
    free(json);
    if (parsed.store_nr != 0xFFFF) loft_copy_record(loft_T, kt, parsed, w, 1);
    fprintf(stderr, "loft-swap: world restored from %s\n", snap);
    return 1;
#else
    free(json);
    fprintf(stderr, "loft-swap: LOFT_RESUME set but %s unreadable; starting fresh\n", snap);
    return 0;
#endif
}

/* The forwarded `swap_world` a call site with no known world type reaches: no layout, so an image
 * handed over is refused rather than read raw. */
static uint8_t loft_swap_world_untyped(loft_ref w) { return loft_swap_world(w, 0, 0); }

/* `write_world_snapshot`: the world as a store image with its sidecar when it is the root of a
 * store of its own (`LOFT_NO_SWAP_IMAGE` sends it as JSON), else as JSON; the path, malloc'd. */
static char *loft_sw_snapshot(const char *base) {
    loft_ref w = loft_sw.w;
    int own = w.store_nr < loft_nstores && loft_stores[w.store_nr].known == loft_sw.tp;
    if (own && !getenv("LOFT_NO_SWAP_IMAGE")) {
        if (loft_sw.sc) {
            char *snap = loft_sw_join(base, ".snap.store");
            if (loft_image_write(loft_T, loft_T_n, w, loft_lit(snap, strlen(snap)), loft_sw.sc)) return snap;
            free(snap);
        }
        fputs("loft-swap: the world goes over as JSON (the store did not form an image)\n", stderr);
    }
    if (!loft_sw.show) return 0;
    loft_text t = { 0, 0, 0 };
    loft_sw.show(&t, w);
    char *snap = loft_sw_join(base, ".snap.json");
    int ok = loft_sw_write(snap, t.p ? t.p : "", t.len);
    loft_text_free(&t);
    if (!ok) { free(snap); return 0; }
    return snap;
}

#if defined(LOFT_POSIX) && !defined(__wasi__)
#include <signal.h>
#include <sys/stat.h>
#include <sys/wait.h>
#include <fcntl.h>
#include <unistd.h>

/* A child process: `argv[0]` run with `set` (name, value pairs, 0-terminated) added to and `unset`
 * (0-terminated) taken from the environment, stdin from /dev/null, stdout and stderr into the
 * files named (or inherited), and in a session of its own when `detached` — a swap's new build is
 * a handover target, not part of this process tree.  Its pid, or -1 with `errno` set. */
static long loft_sw_spawn(char *const argv[], const char *const *set, const char *const *unset, const char *out,
                          const char *err, int detached) {
    fflush(stdout);
    fflush(stderr);
    pid_t pid = fork();
    if (pid < 0) return -1;
    if (pid == 0) {
        if (detached) (void) setsid();
        int in = open("/dev/null", O_RDONLY);
        if (in >= 0) { dup2(in, 0); close(in); }
        if (out) { int fd = open(out, O_WRONLY | O_CREAT | O_TRUNC, 0644); if (fd >= 0) { dup2(fd, 1); close(fd); } }
        if (err) { int fd = open(err, O_WRONLY | O_CREAT | O_TRUNC, 0644); if (fd >= 0) { dup2(fd, 2); close(fd); } }
        for (size_t i = 0; set && set[i]; i += 2) setenv(set[i], set[i + 1], 1);
        for (size_t i = 0; unset && unset[i]; i++) unsetenv(unset[i]);
        execv(argv[0], argv);
        _exit(127);
    }
    return (long) pid;
}

/* Is child `pid` still running?  When it has ended, `*status` says how (`ExitStatus`'s text). */
static int loft_sw_alive(long pid, char *status, size_t cap) {
    int st = 0;
    pid_t r = waitpid((pid_t) pid, &st, WNOHANG);
    if (r == 0) return 1;
    if (r < 0) snprintf(status, cap, "unknown");
    else if (WIFEXITED(st)) snprintf(status, cap, "exit status: %d", WEXITSTATUS(st));
    else if (WIFSIGNALED(st)) snprintf(status, cap, "signal: %d", WTERMSIG(st));
    else snprintf(status, cap, "unknown");
    return 0;
}

static void loft_sw_stop_tree(long pid) {
    (void) kill(-(pid_t) pid, SIGKILL);
    (void) kill((pid_t) pid, SIGKILL);
    (void) waitpid((pid_t) pid, 0, 0);
}
#define LOFT_SW_CAN 1
#endif

/* `swap_start(artifact)`: request a swap to that binary; the run loop acts at the next frame
 * boundary. */
static uint8_t loft_swap_start(loft_str artifact) {
    char *a = loft_cstr_own(artifact);
    if (!*a || !loft_sw_exists(a)) {
        fprintf(stderr, "loft-swap: no such artifact `%s` \xe2\x80\x94 swap refused\n", a);
        free(a);
        return 0;
    }
#ifndef LOFT_SW_CAN
    fputs("loft-swap: this build cannot hand its world to another \xe2\x80\x94 swap refused\n", stderr);
    free(a);
    return 0;
#else
    if (loft_sw.phase != LOFT_SW_IDLE) {
        fputs("loft-swap: a swap is already in progress\n", stderr);
        free(a);
        return 0;
    }
    free(loft_sw.artifact);
    loft_sw.artifact = a;
    loft_sw.phase = LOFT_SW_REQUESTED;
    return 1;
#endif
}

static void loft_sw_reset(void) {
    free(loft_sw.ready);
    free(loft_sw.snap);
    loft_sw.ready = loft_sw.snap = 0;
    loft_sw.pid = -1;
    loft_sw.phase = LOFT_SW_IDLE;
}

/* The listener kernel dropped at a handover: its listener, UDP socket and every connection
 * closed, so seats reconnect into the new process. */
static void loft_sw_drop_kernel(void) {
#if defined(LOFT_SOCKETS)
    if (loft_ehk.up) {
        loft_eh_close_fd(&loft_ehk.listener);
        loft_eh_close_fd(&loft_ehk.udp);
        for (size_t i = 0; i < loft_ehk.nconns; i++) loft_eh_close_fd(&loft_ehk.conns[i].fd);
    }
#endif
    loft_ehk.alive = 0;
    loft_ehk.up = 0;
}

/* The swap step the run loop drives each turn: 0 serve normally, 1 FROZEN (mechanics only, meaning
 * waits), 2 handed over (the loop returns; this process retires).  Every failure rolls back to 0. */
static int64_t loft_kernel_swap_step(void) {
    switch (loft_sw.phase) {
    case LOFT_SW_IDLE: return 0;
    case LOFT_SW_DONE: return 2;
#ifdef LOFT_SW_CAN
    case LOFT_SW_REQUESTED: {
        if (!loft_sw.registered) {
            fputs("loft-swap: rolled back (no swap_world registered)\n", stderr);
            loft_sw_reset();
            return 0;
        }
        /* The frame boundary: meaning is frozen from here on, so this snapshot is THE world
         * state the new build resumes. */
        char base[1024];
        snprintf(base, sizeof base, "%s/loft_swap_%ld", loft_sw_tmp(), (long) getpid());
        loft_sw.ready = loft_sw_join(base, ".ready");
        remove(loft_sw.ready);
        loft_sw.snap = loft_sw_snapshot(base);
        if (!loft_sw.snap) {
            fputs("loft-swap: rolled back (cannot write snapshot)\n", stderr);
            loft_sw_reset();
            return 0;
        }
        char *argv[] = { loft_sw.artifact, 0 };
        const char *set[] = { "LOFT_RESUME", loft_sw.snap, "LOFT_SWAP_READY", loft_sw.ready, 0 };
        /* Dispatch reset: the new build has the edits compiled. */
        const char *unset[] = { "LOFT_FLIP_FNS", 0 };
        long pid = loft_sw_spawn(argv, set, unset, 0, 0, 1);
        if (pid < 0) {
            fprintf(stderr, "loft-swap: rolled back (cannot spawn %s: %s)\n", loft_sw.artifact, strerror(errno));
            loft_sw_remove_snapshot(loft_sw.snap);
            loft_sw_reset();
            return 0;
        }
        fprintf(stderr, "loft-swap: booting %s (meaning frozen)\n", loft_sw.artifact);
        loft_sw.pid = pid;
        loft_sw.deadline = loft_now_us() + 15000000;
        loft_sw.phase = LOFT_SW_WAITING;
        return 1;
    }
    case LOFT_SW_WAITING: {
        char status[64];
        if (loft_sw_exists(loft_sw.ready)) {
            fputs("loft-swap: handing over \xe2\x80\x94 this build retires\n", stderr);
            loft_sw_remove_snapshot(loft_sw.snap);
            remove(loft_sw.ready);
            free(loft_sw.ready);
            free(loft_sw.snap);
            loft_sw.ready = loft_sw.snap = 0;
            loft_sw.phase = LOFT_SW_DONE;
            loft_sw_drop_kernel();
            return 2;
        }
        if (!loft_sw_alive(loft_sw.pid, status, sizeof status)) {
            fprintf(stderr, "loft-swap: rolled back (new build exited %s before serving)\n", status);
            loft_sw_remove_snapshot(loft_sw.snap);
            remove(loft_sw.ready);
            loft_sw_reset();
            return 0;
        }
        if (loft_now_us() > loft_sw.deadline) {
            fputs("loft-swap: rolled back (new build never became ready)\n", stderr);
            loft_sw_stop_tree(loft_sw.pid);
            loft_sw_remove_snapshot(loft_sw.snap);
            remove(loft_sw.ready);
            loft_sw_reset();
            return 0;
        }
        return 1;
    }
#endif
    default: return 0;
    }
}

/* Did THIS process hand its world to a new build? */
static uint8_t loft_swap_retired(void) { return loft_sw.phase == LOFT_SW_DONE; }

/* ── The background rebuild (src/live_rebuild.rs): 0 idle, 1 building, 2 ready, 3 failed ── */
static struct {
    int state;
    long pid;
    char *snapshot;
    size_t snap_len;
    char *out, *err, *artifact;
    char failure[512];
    unsigned seq;
} loft_rb = { 0, -1, 0, 0, 0, 0, 0, { 0 }, 0 };

#ifdef LOFT_SW_CAN
/* `spawn_build`: one background `--check --native-c` build of `src` through the driver, its output
 * in files (a failed compile prints more than a pipe holds); 0 with `why` set. */
static int loft_rb_spawn(const char *driver, const char *src, char *why, size_t cap) {
    size_t n = 0;
    char *snapshot = loft_sw_read(src, &n);
    if (!snapshot) {
        snprintf(why, cap, "cannot read %s: %s", src, strerror(errno));
        return 0;
    }
    loft_rb.seq++;
    char base[1024];
    snprintf(base, sizeof base, "%s/loft_rebuild_%ld_%u", loft_sw_tmp(), (long) getpid(), loft_rb.seq);
    free(loft_rb.out);
    free(loft_rb.err);
    loft_rb.out = loft_sw_join(base, ".out");
    loft_rb.err = loft_sw_join(base, ".err");
    /* The argument list: the driver, its flags, a `--lib` per LOFT_LIVE_LIBS entry, the source. */
    const char *libs = getenv("LOFT_LIVE_LIBS");
    char *libcopy = libs ? loft_sw_join(libs, "") : 0;
    size_t nlib = 0;
    for (const char *q = libcopy; q && *q; q++) nlib += *q == ':';
    char **argv = (char **) calloc(8 + 2 * (nlib + 1), sizeof *argv);
    if (!argv) { fputs("loft: out of memory\n", stderr); abort(); }
    size_t a = 0;
    argv[a++] = (char *) driver;
    argv[a++] = (char *) "--no-warnings";
    argv[a++] = (char *) "--check";
    /* The running build is a C build: its successor is built the same way. */
    argv[a++] = (char *) "--native-c";
    for (char *tok = libcopy ? strtok(libcopy, ":") : 0; tok; tok = strtok(0, ":")) {
        if (!*tok) continue;
        argv[a++] = (char *) "--lib";
        argv[a++] = tok;
    }
    argv[a++] = (char *) src;
    argv[a] = 0;
    /* The build of the next instance is checked against the running world: a world it cannot
     * take over fails the build here, never the swap. */
    char *world = 0;
    if (loft_sw.layout) {
        world = loft_sw_join(base, ".world.dschema");
        if (!loft_sw_write(world, loft_sw.layout, strlen(loft_sw.layout))) {
            snprintf(why, cap, "%s", strerror(errno));
            free(world);
            free(argv);
            free(libcopy);
            free(snapshot);
            return 0;
        }
    }
    const char *set[] = { "LOFT_CHECK_ARTIFACT", "1", world ? "LOFT_CHECK_WORLD" : 0, world, 0 };
    /* A build legitimately takes minutes: never under the run watchdog. */
    const char *unset[] = { "LOFT_TIMEOUT", 0 };
    long pid = loft_sw_spawn(argv, set, unset, loft_rb.out, loft_rb.err, 0);
    free(world);
    free(argv);
    free(libcopy);
    if (pid < 0) {
        snprintf(why, cap, "cannot spawn %s: %s", driver, strerror(errno));
        free(snapshot);
        return 0;
    }
    free(loft_rb.snapshot);
    loft_rb.snapshot = snapshot;
    loft_rb.snap_len = n;
    loft_rb.pid = pid;
    loft_rb.state = 1;
    return 1;
}
#endif

/* `rebuild_start`: start a background rebuild of the program this binary serves; 1 building
 * (idempotent while one runs), 0 when the host lacks the live env or the spawn failed. */
static uint8_t loft_rebuild_start(void) {
    const char *driver = getenv("LOFT_LIVE_DRIVER"), *src = getenv("LOFT_LIVE_SRC");
    if (!driver || !src) {
        fputs("loft-live: rebuild needs LOFT_LIVE_DRIVER/LOFT_LIVE_SRC (run through the loft driver)\n", stderr);
        return 0;
    }
#ifdef LOFT_SW_CAN
    if (loft_rb.state == 1) return 1;
    char why[1200];
    if (loft_rb_spawn(driver, src, why, sizeof why)) {
        fprintf(stderr, "loft-live: rebuild started (%s)\n", src);
        return 1;
    }
    fprintf(stderr, "loft-live: rebuild failed to start \xe2\x80\x94 %s\n", why);
    loft_rb.state = 3;
    return 0;
#else
    fputs("loft-live: rebuild failed to start \xe2\x80\x94 this host cannot run the build\n", stderr);
    return 0;
#endif
}

/* `rebuild_status`: completion with a source that changed mid-build requeues (status stays 1), so
 * a ready artifact always corresponds to the source as of the call. */
static int64_t loft_rebuild_status(void) {
#ifdef LOFT_SW_CAN
    if (loft_rb.state != 1) return loft_rb.state;
    char status[64];
    if (loft_sw_alive(loft_rb.pid, status, sizeof status)) return 1;
    const char *src = getenv("LOFT_LIVE_SRC");
    if (!src) src = "";
    size_t n = 0;
    char *now = loft_sw_read(src, &n);
    int stale = !now || n != loft_rb.snap_len || memcmp(now, loft_rb.snapshot, n) != 0;
    free(now);
    if (stale) {
        /* The source changed while the build ran: requeue with the current content. */
        fputs("loft-live: rebuild stale (source changed during build) \xe2\x80\x94 requeued\n", stderr);
        const char *driver = getenv("LOFT_LIVE_DRIVER");
        char why[1200];
        if (!driver || !loft_rb_spawn(driver, src, why, sizeof why)) {
            fprintf(stderr, "loft-live: requeue failed \xe2\x80\x94 %s\n", driver ? why : "no driver");
            loft_rb.state = 3;
            return 3;
        }
        return 1;
    }
    if (strcmp(status, "exit status: 0") != 0) {
        /* The last five lines of the build's errors, and the one that says why. */
        char *err = loft_sw_read(loft_rb.err, 0);
        const char *tail = err ? err : "";
        size_t len = strlen(tail);
        while (len && tail[len - 1] == '\n') len--;
        size_t start = len;
        for (int lines = 0; start > 0; start--)
            if (tail[start - 1] == '\n' && ++lines == 5) break;
        fprintf(stderr, "loft-live: rebuild FAILED (old build keeps serving):\n%.*s\n", (int) (len - start), tail + start);
        const char *pick = 0, *last = 0;
        size_t pick_n = 0, last_n = 0;
        for (const char *l = tail + start; l < tail + len;) {
            const char *e = memchr(l, '\n', (size_t) (tail + len - l));
            size_t ln = e ? (size_t) (e - l) : (size_t) (tail + len - l);
            char *line = loft_sw_dup(l, ln);
            if (!pick && (strstr(line, "cannot take over the running world") || strstr(line, "error"))) { pick = l; pick_n = ln; }
            last = l;
            last_n = ln;
            free(line);
            l = e ? e + 1 : tail + len;
        }
        if (!pick) { pick = last; pick_n = last_n; }
        if (!pick) { pick = "the build failed"; pick_n = strlen(pick); }
        while (pick_n && isspace((unsigned char) *pick)) { pick++; pick_n--; }
        while (pick_n && isspace((unsigned char) pick[pick_n - 1])) pick_n--;
        snprintf(loft_rb.failure, sizeof loft_rb.failure, "%.*s", (int) pick_n, pick);
        free(err);
        loft_rb.state = 3;
        return 3;
    }
    /* The artifact rides the driver's ok line: "ok <src> <artifact>". */
    char *out = loft_sw_read(loft_rb.out, 0);
    char *prefix = loft_sw_join("ok ", src);
    char *artifact = 0;
    for (const char *l = out ? out : ""; *l;) {
        const char *e = strchr(l, '\n');
        size_t ln = e ? (size_t) (e - l) : strlen(l);
        size_t pl = strlen(prefix);
        if (ln > pl && memcmp(l, prefix, pl) == 0 && l[pl] == ' ') {
            const char *a = l + pl + 1;
            size_t an = ln - pl - 1;
            while (an && isspace((unsigned char) a[an - 1])) an--;
            artifact = loft_sw_dup(a, an);
            break;
        }
        l = e ? e + 1 : l + ln;
    }
    free(prefix);
    if (!artifact || !loft_sw_exists(artifact)) {
        fprintf(stderr, "loft-live: rebuild succeeded but no artifact on the ok line (\"%s\")\n", out ? out : "");
        free(artifact);
        free(out);
        loft_rb.state = 3;
        return 3;
    }
    free(out);
    fprintf(stderr, "loft-live: rebuild ready \xe2\x80\x94 %s\n", artifact);
    free(loft_rb.artifact);
    loft_rb.artifact = artifact;
    loft_rb.state = 2;
    return 2;
#else
    return loft_rb.state;
#endif
}

/* Why the last rebuild failed, in one line; "" when none has. */
static const char *loft_rebuild_failure(void) { return loft_rb.failure; }

/* The READY build's artifact; "" unless `rebuild_status() == 2`. */
static loft_text loft_kernel_rebuild_artifact(void) {
    loft_text t = { 0, 0, 0 };
    if (loft_rb.state == 2 && loft_rb.artifact) loft_push_cstr(&t, loft_rb.artifact);
    return t;
}
/* `live_flip`: no live tier to flip a function into. */
static uint8_t loft_live_flip(loft_str name, uint8_t on) {
    (void) name;
    (void) on;
    return 0;
}

/* ── `http_fetch` ── */

#if defined(LOFT_SOCKETS)
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

static void loft_eh_http_run(loft_eh_req *q) {
    loft_text body = { 0, 0, 0 };
    int64_t status = loft_eh_http(q->method, q->url, q->body, q->headers, &body, 5);
    loft_eh_park(q->id, status, body.p, body.len);
    free(body.p);
    free(q->method);
    free(q->url);
    free(q->body);
    free(q->headers);
    free(q);
}

#if defined(_WIN32)
static DWORD WINAPI loft_eh_http_worker(LPVOID arg) {
    loft_eh_http_run((loft_eh_req *) arg);
    return 0;
}
#else
static void *loft_eh_http_worker(void *arg) {
    loft_eh_http_run((loft_eh_req *) arg);
    return 0;
}
#endif
#endif

/* `http_fetch(method, url, body, headers) -> integer`: the request id at once; the completion a
 * kind-3 event (cid = the id, status = the HTTP status or negative, payload = the body). */
static int64_t loft_kernel_http_fetch(loft_str method, loft_str url, loft_str body, loft_str headers) {
    int64_t id = loft_eh_http_next++;
#if defined(LOFT_SOCKETS) && defined(LOFT_EH_THREADS)
    loft_eh_req *q = (loft_eh_req *) malloc(sizeof *q);
    if (!q) { fputs("loft: out of memory\n", stderr); abort(); }
    q->id = id;
    q->method = loft_cstr_own(method);
    q->url = loft_cstr_own(url);
    q->body = loft_cstr_own(body);
    q->headers = loft_cstr_own(headers);
    /* Detached: the worker parks its completion and ends; nobody joins it. */
#if defined(_WIN32)
    HANDLE t = CreateThread(0, 0, loft_eh_http_worker, q, 0, 0);
    int started = t != 0;
    if (t) CloseHandle(t);
#else
    pthread_t t;
    pthread_attr_t at;
    pthread_attr_init(&at);
    pthread_attr_setdetachstate(&at, PTHREAD_CREATE_DETACHED);
    int started = pthread_create(&t, &at, loft_eh_http_worker, q) == 0;
    pthread_attr_destroy(&at);
#endif
    if (!started) {
        static const char why[] = "cannot start the request's worker thread";
        loft_eh_park(id, -1, why, sizeof why - 1);
        free(q->method);
        free(q->url);
        free(q->body);
        free(q->headers);
        free(q);
    }
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
