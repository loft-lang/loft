/* ── Sockets on every host that has them (@PLN203 C8) ─────────────────────────────────────────
 * The one place the runtime's socket users — the HTTP client (`loft_https.h`), the server
 * (`loft_server.h`), the engine host (`loft_engine.h`), the `web` component — meet the OS.  A
 * POSIX host has BSD sockets; Windows has Winsock, the same calls with a different close, a
 * different non-blocking switch, `WSAPoll` for `poll`, a millisecond receive timeout, its own
 * error variable and a library to start.  Those differences are the routines below; a user writes
 * `loft_sock_*` and never an OS name.  `LOFT_SOCKETS` says the unit has them: a host with
 * sockets (not WASI) and a unit that asked for them. */
#ifndef LOFT_SOCK_H
#define LOFT_SOCK_H
/* Only a unit that opens a socket includes the OS's socket headers — on Windows they bring
 * `windows.h`. */
#if (defined(LOFT_POSIX_HOST) || defined(_WIN32)) && \
    (defined(LOFT_NET) || defined(LOFT_ENGINE) || defined(LOFT_WEB) || defined(LOFT_SERVER))
#define LOFT_SOCKETS 1
#endif

#if defined(LOFT_SOCKETS)
#if defined(_WIN32)
/* `windows.h` comes with Winsock: no `min` / `max` macros, none of the COM and RPC headers (which
 * define `small`, `interface` and more).  Generated names carry a `vN_` / `n_` / `t_` prefix, so no
 * macro it leaves can reach them. */
#ifndef WIN32_LEAN_AND_MEAN
#define WIN32_LEAN_AND_MEAN 1
#endif
#ifndef NOMINMAX
#define NOMINMAX 1
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
#if defined(_MSC_VER)
#pragma comment(lib, "ws2_32.lib")
#define strncasecmp _strnicmp
#define strcasecmp _stricmp
#else
#include <strings.h>
#endif
#include <errno.h>
#else
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
#endif

/* The socket library ready for use: Winsock started — before the first name lookup or socket;
 * a second `WSAStartup` only counts — and nothing to do under POSIX. */
static void loft_sock_ready(loft_cx *cx) {
#if defined(_WIN32)
    /* `@FR-C-Publish`: Winsock started once, under the lock. */
    static int started = 0;
    loft_pub_lock();
    if (!started) {
        WSADATA w;
        WSAStartup(MAKEWORD(2, 2), &w);
        started = 1;
    }
    loft_pub_unlock();
#endif
}

/* A socket as the runtime holds it: an `int`, -1 for none.  Winsock's SOCKET is a kernel handle,
 * which fits. */
static int loft_sock_open(loft_cx *cx, int family, int type, int proto) {
#if defined(_WIN32)
    loft_sock_ready(cx);
    SOCKET s = socket(family, type, proto);
    return s == INVALID_SOCKET ? -1 : (int) s;
#else
    return socket(family, type, proto);
#endif
}

static void loft_sock_close(loft_cx *cx, int fd) {
#if defined(_WIN32)
    closesocket((SOCKET) fd);
#else
    close(fd);
#endif
}

/* The last socket call's failure, as POSIX names it: interrupted, would block, in progress. */
static int loft_sock_interrupted(loft_cx *cx) {
#if defined(_WIN32)
    return WSAGetLastError() == WSAEINTR;
#else
    return errno == EINTR;
#endif
}

static int loft_sock_would_block(loft_cx *cx) {
#if defined(_WIN32)
    return WSAGetLastError() == WSAEWOULDBLOCK;
#else
    return errno == EAGAIN || errno == EWOULDBLOCK;
#endif
}

static int loft_sock_in_progress(loft_cx *cx) {
#if defined(_WIN32)
    return WSAGetLastError() == WSAEWOULDBLOCK;
#else
    return errno == EINPROGRESS;
#endif
}

/* The last failure as an errno value, for a message. */
static int loft_sock_error(loft_cx *cx) {
#if defined(_WIN32)
    return WSAGetLastError();
#else
    return errno;
#endif
}

/* Up to `n` bytes; the count, 0 at the end, -1 on a failure. */
static long loft_sock_recv(loft_cx *cx, int fd, void *buf, size_t n, int flags) {
#if defined(_WIN32)
    return (long) recv((SOCKET) fd, (char *) buf, (int) (n > 0x7fffffff ? 0x7fffffff : n), flags);
#else
    return (long) recv(fd, buf, n, flags);
#endif
}

static long loft_sock_send(loft_cx *cx, int fd, const void *buf, size_t n, int flags) {
#if defined(_WIN32)
    return (long) send((SOCKET) fd, (const char *) buf, (int) (n > 0x7fffffff ? 0x7fffffff : n), flags);
#else
    return (long) send(fd, buf, n, flags);
#endif
}

/* A socket a child process does not inherit. */
static void loft_sock_cloexec(loft_cx *cx, int fd) {
#if defined(_WIN32)
    SetHandleInformation((HANDLE) (intptr_t) fd, HANDLE_FLAG_INHERIT, 0);
#else
    fcntl(fd, F_SETFD, fcntl(fd, F_GETFD) | FD_CLOEXEC);
#endif
}

static void loft_sock_blocking(loft_cx *cx, int fd, int on) {
#if defined(_WIN32)
    u_long nb = on ? 0 : 1;
    ioctlsocket((SOCKET) fd, FIONBIO, &nb);
#else
    int fl = fcntl(fd, F_GETFL);
    fcntl(fd, F_SETFL, on ? (fl & ~O_NONBLOCK) : (fl | O_NONBLOCK));
#endif
}

/* `SO_RCVTIMEO` / `SO_SNDTIMEO` (`opt`) at `ms`: a `timeval` under POSIX, milliseconds on Windows. */
static void loft_sock_timeout_opt(loft_cx *cx, int fd, int opt, int64_t ms) {
#if defined(_WIN32)
    DWORD v = (DWORD) (ms < 0 ? 0 : ms);
    setsockopt((SOCKET) fd, SOL_SOCKET, opt, (const char *) &v, sizeof v);
#else
    struct timeval tv = { (time_t) (ms / 1000), (long) ((ms % 1000) * 1000) };
    setsockopt(fd, SOL_SOCKET, opt, &tv, sizeof tv);
#endif
}

/* An integer socket option (`SO_REUSEADDR`, `TCP_NODELAY`, …). */
static int loft_sock_flag(loft_cx *cx, int fd, int level, int opt, int on) {
#if defined(_WIN32)
    return setsockopt((SOCKET) fd, level, opt, (const char *) &on, sizeof on);
#else
    return setsockopt(fd, level, opt, &on, sizeof on);
#endif
}

/* A listener that takes its port back at once after a restart, and never shares a live one:
 * `SO_REUSEADDR` under POSIX; on Windows that option lets a second socket bind over a live
 * listener, so it is `SO_EXCLUSIVEADDRUSE` there (`std::net::TcpListener` sets neither). */
static void loft_sock_listener_addr(loft_cx *cx, int fd) {
#if defined(_WIN32)
    loft_sock_flag(cx, fd, SOL_SOCKET, SO_EXCLUSIVEADDRUSE, 1);
#else
    loft_sock_flag(cx, fd, SOL_SOCKET, SO_REUSEADDR, 1);
#endif
}

/* The pending error of a connect that finished (`SO_ERROR`): 0 when it connected. */
static int loft_sock_pending_error(loft_cx *cx, int fd) {
    int err = 0;
#if defined(_WIN32)
    int el = sizeof err;
    if (getsockopt((SOCKET) fd, SOL_SOCKET, SO_ERROR, (char *) &err, &el) != 0) return -1;
#else
    socklen_t el = sizeof err;
    if (getsockopt(fd, SOL_SOCKET, SO_ERROR, &err, &el) != 0) return -1;
#endif
    return err;
}

/* `poll` over sockets: `WSAPoll` on Windows, whose `pollfd` holds a SOCKET. */
typedef struct { int fd; short events, revents; } loft_pollfd;

static int loft_sock_poll(loft_cx *cx, loft_pollfd *p, size_t n, int ms) {
#if defined(_WIN32)
    WSAPOLLFD one[16], *w = n <= 16 ? one : (WSAPOLLFD *) malloc(n * sizeof *w);
    if (!w) return -1;
    for (size_t i = 0; i < n; i++) {
        w[i].fd = p[i].fd < 0 ? INVALID_SOCKET : (SOCKET) p[i].fd;
        w[i].events = p[i].events;
        w[i].revents = 0;
    }
    int r = WSAPoll(w, (ULONG) n, ms);
    for (size_t i = 0; i < n; i++) p[i].revents = w[i].revents;
    if (w != one) free(w);
    return r;
#else
    struct pollfd one[16], *w = n <= 16 ? one : (struct pollfd *) malloc(n * sizeof *w);
    if (!w) return -1;
    for (size_t i = 0; i < n; i++) {
        w[i].fd = p[i].fd;
        w[i].events = p[i].events;
        w[i].revents = 0;
    }
    int r = poll(w, (nfds_t) n, ms);
    for (size_t i = 0; i < n; i++) p[i].revents = w[i].revents;
    if (w != one) free(w);
    return r;
#endif
}

/* A listening socket bound to `a`: 0, else -1 with the failure in `loft_sock_error`. */
static int loft_sock_bind_listen(loft_cx *cx, int fd, const struct sockaddr *a, size_t al, int backlog) {
#if defined(_WIN32)
    return bind((SOCKET) fd, a, (int) al) == 0 && listen((SOCKET) fd, backlog) == 0 ? 0 : -1;
#else
    return bind(fd, a, (socklen_t) al) == 0 && listen(fd, backlog) == 0 ? 0 : -1;
#endif
}

/* The next connection on listener `fd`: its socket, else -1 (`loft_sock_would_block` says
 * whether one was only not pending yet). */
static int loft_sock_accept(loft_cx *cx, int fd) {
#if defined(_WIN32)
    SOCKET s = accept((SOCKET) fd, 0, 0);
    return s == INVALID_SOCKET ? -1 : (int) s;
#else
    return accept(fd, 0, 0);
#endif
}

/* `fd` bound to `a` without listening (a datagram socket, or before `listen`): 0, else -1. */
static int loft_sock_bind(loft_cx *cx, int fd, const struct sockaddr *a, socklen_t al) {
#if defined(_WIN32)
    return bind((SOCKET) fd, a, (int) al) == 0 ? 0 : -1;
#else
    return bind(fd, a, al) == 0 ? 0 : -1;
#endif
}

/* `fd`'s own address (`getsockname`) or its peer's (`getpeername`): 0, else -1. */
static int loft_sock_name(loft_cx *cx, int fd, int peer, struct sockaddr *a, socklen_t *al) {
#if defined(_WIN32)
    int n = (int) *al;
    int r = peer ? getpeername((SOCKET) fd, a, &n) : getsockname((SOCKET) fd, a, &n);
    *al = (socklen_t) n;
    return r == 0 ? 0 : -1;
#else
    return (peer ? getpeername(fd, a, al) : getsockname(fd, a, al)) == 0 ? 0 : -1;
#endif
}

/* A datagram socket's peer fixed (`connect` on UDP: no handshake): 0, else -1. */
static int loft_sock_connect_to(loft_cx *cx, int fd, const struct sockaddr *a, socklen_t al) {
#if defined(_WIN32)
    return connect((SOCKET) fd, a, (int) al) == 0 ? 0 : -1;
#else
    return connect(fd, a, al) == 0 ? 0 : -1;
#endif
}

/* A datagram socket whose receive does not fail on an earlier send's ICMP "port unreachable":
 * Winsock reports one as WSAECONNRESET on the NEXT `recvfrom`, which a pump reads as "nothing
 * waiting" and so delays the datagrams queued behind it by a whole pump; POSIX reports none on
 * an unconnected socket. */
static void loft_sock_udp_quiet(loft_cx *cx, int fd) {
#if defined(_WIN32)
#ifndef SIO_UDP_CONNRESET
#define SIO_UDP_CONNRESET _WSAIOW(IOC_VENDOR, 12)
#endif
    BOOL off = FALSE;
    DWORD got = 0;
    WSAIoctl((SOCKET) fd, SIO_UDP_CONNRESET, &off, sizeof off, 0, 0, &got, 0, 0);
#else
    (void) fd;
#endif
}

/* One datagram to `a`: the count sent, -1 on a failure. */
static long loft_sock_sendto(loft_cx *cx, int fd, const void *buf, size_t n, const struct sockaddr *a, socklen_t al) {
#if defined(_WIN32)
    return (long) sendto((SOCKET) fd, (const char *) buf, (int) (n > 0x7fffffff ? 0x7fffffff : n), 0, a, (int) al);
#else
    return (long) sendto(fd, buf, n, 0, a, al);
#endif
}

/* One datagram and its source: the count, -1 when none is waiting (the socket non-blocking) or
 * on a failure. */
static long loft_sock_recvfrom(loft_cx *cx, int fd, void *buf, size_t n, struct sockaddr *a, socklen_t *al) {
#if defined(_WIN32)
    int fl = (int) *al;
    long r = (long) recvfrom((SOCKET) fd, (char *) buf, (int) (n > 0x7fffffff ? 0x7fffffff : n), 0, a, &fl);
    *al = (socklen_t) fl;
    return r;
#else
    return (long) recvfrom(fd, buf, n, 0, a, al);
#endif
}

/* Up to `n` bytes waiting on `fd`, left there: the count, 0 when the peer closed, -1 when none
 * is waiting yet, -2 on a failure.  POSIX peeks without blocking (`MSG_DONTWAIT`); Winsock has
 * no such flag, so the socket is non-blocking for the peek. */
static long loft_sock_peek(loft_cx *cx, int fd, void *buf, size_t n) {
    for (;;) {
#if defined(_WIN32)
        loft_sock_blocking(cx, fd, 0);
        long r = loft_sock_recv(cx, fd, buf, n, MSG_PEEK);
        int interrupted = r < 0 && loft_sock_interrupted(cx), waiting = r < 0 && loft_sock_would_block(cx);
        loft_sock_blocking(cx, fd, 1);
#else
        long r = loft_sock_recv(cx, fd, buf, n, MSG_PEEK | MSG_DONTWAIT);
        int interrupted = r < 0 && loft_sock_interrupted(cx), waiting = r < 0 && loft_sock_would_block(cx);
#endif
        if (interrupted) continue;
        return r >= 0 ? r : waiting ? -1 : -2;
    }
}

/* Sleep `ms`. */
static void loft_sock_nap(loft_cx *cx, int64_t ms) {
    if (ms <= 0) return;
#if defined(_WIN32)
    Sleep((DWORD) ms);
#else
    struct timespec ts = { (time_t) (ms / 1000), (long) ((ms % 1000) * 1000000) };
    while (nanosleep(&ts, &ts) != 0 && errno == EINTR) {}
#endif
}

/* Connect within `ms`: 1 when the TCP handshake completed. */
static int loft_sock_connect_within(loft_cx *cx, int fd, const struct sockaddr *a, socklen_t al, int64_t ms) {
    loft_sock_blocking(cx, fd, 0);
#if defined(_WIN32)
    int r = connect((SOCKET) fd, a, (int) al);
#else
    int r = connect(fd, a, al);
#endif
    if (r != 0 && !loft_sock_in_progress(cx)) return 0;
    if (r != 0) {
        loft_pollfd p = { fd, POLLOUT, 0 };
        if (loft_sock_poll(cx, &p, 1, (int) ms) <= 0) return 0;
        if (loft_sock_pending_error(cx, fd) != 0) return 0;
    }
    loft_sock_blocking(cx, fd, 1);
    return 1;
}
#endif
#endif
