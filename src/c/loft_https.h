/* ── A client connection: a socket, or a TLS session over one ──────────────────────────────────
 * The one transport under both HTTP clients of the runtime — `store_load_url`'s (`loft_net.h`) and
 * `http_fetch`'s (`loft_engine.h`).  An `https://` URL runs over the TLS client of
 * `src/c/tls/loft_tls.c` (BearSSL), which is not part of the unit: `src/c/tls.rs` builds it into a
 * cached archive and names it on a `link:` line, and `LOFT_HTTPS` says it is there.  A unit built
 * without it (a host whose compiler could not build it, `LOFT_TLS_UNAVAILABLE` saying why, or a
 * target with no sockets) answers an `https://` URL with that reason.  Pasted into a unit that
 * fetches over a URL (`LOFT_NET`, `LOFT_ENGINE`). */
#if defined(LOFT_POSIX_HOST) && (defined(LOFT_NET) || defined(LOFT_ENGINE))
#include <errno.h>
#include <sys/socket.h>
#include <unistd.h>

#ifdef LOFT_HTTPS
struct loft_tls;
struct loft_tls *loft_tls_open(int fd, const char *host, char *err, size_t cap);
long loft_tls_read(struct loft_tls *t, void *buf, size_t n);
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
#endif
