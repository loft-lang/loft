/* ── The ssh library's natives, compiled into the `net` component (`src/c/components/net.c`) ──
 * A remote interactive shell over SSH: `ssh_connect` resolves and connects and runs the
 * transport handshake, `ssh_login` authenticates by password, `ssh_open_shell` opens a session
 * channel with a pty and a shell, and `ssh_send` / `ssh_recv` / `ssh_resize` drive it; `ssh_recv`
 * hands out what has arrived since the last call and never waits.  The protocol is
 * `tls/loft_ssh.c`'s, built into the TLS archive beside the TLS client from the same vendored
 * BearSSL.  A session is a handle (an index, never reused); an unknown or closed handle makes
 * every call a no-op answering false or "".  `LOFT_SSH_TRACE=1` prints why a connect, a login or
 * a shell failed on stderr.  Never pasted into a unit (`LOFT_SSH` is the component's).  Guard:
 * `tests/ssh_natives.rs`. */
#if defined(LOFT_SSH) && defined(LOFT_SOCKETS)

struct loft_ssh;
struct loft_ssh *loft_ssh_open(int fd, char *err, size_t cap);
const char *loft_ssh_error(const struct loft_ssh *s);
int loft_ssh_login(struct loft_ssh *s, const char *user, size_t ul, const char *pw, size_t pl);
int loft_ssh_shell(struct loft_ssh *s, const char *term, size_t tl, uint32_t cols, uint32_t rows);
int loft_ssh_write(struct loft_ssh *s, const void *data, size_t n);
size_t loft_ssh_read(struct loft_ssh *s, const unsigned char **out);
void loft_ssh_resize(struct loft_ssh *s, uint32_t cols, uint32_t rows);
int loft_ssh_is_open(struct loft_ssh *s);
void loft_ssh_close(struct loft_ssh *s);

/* The sessions, by handle; a closed one's slot stays empty. */
static LOFT_TLS struct loft_ssh **loft_ssh_tab;
static LOFT_TLS size_t loft_ssh_n, loft_ssh_cap;

static struct loft_ssh *loft_ssh_at(int64_t h) {
    return h >= 0 && (size_t) h < loft_ssh_n ? loft_ssh_tab[h] : 0;
}

static void loft_ssh_trace(const char *what, const char *why) {
    const char *t = getenv("LOFT_SSH_TRACE");
    if (t && *t && strcmp(t, "0") != 0) fprintf(stderr, "ssh: %s: %s\n", what, why);
}

static uint32_t loft_ssh_dim(int64_t v) { return v < 0 ? 0u : v > 0xffff ? 0xffffu : (uint32_t) v; }

/* Resolve `host`, connect within 30 s, and run the handshake: a handle, or -1. */
static int64_t loft_ssh_connect(loft_str host, int64_t port) {
    char name[512], service[16], why[300];
    if (host.len == 0 || host.len >= sizeof name || port <= 0 || port > 65535) {
        loft_ssh_trace("connect", "a bad host or port");
        return -1;
    }
    memcpy(name, host.p, host.len);
    name[host.len] = 0;
    snprintf(service, sizeof service, "%d", (int) port);
    struct addrinfo hints, *res = 0, *ai;
    memset(&hints, 0, sizeof hints);
    hints.ai_socktype = SOCK_STREAM;
    loft_sock_ready();
    int gai = getaddrinfo(name, service, &hints, &res);
    if (gai != 0) {
        loft_ssh_trace("connect", gai_strerror(gai));
        return -1;
    }
    int fd = -1;
    for (ai = res; ai && fd < 0; ai = ai->ai_next) {
        fd = loft_sock_open(ai->ai_family, SOCK_STREAM, 0);
        if (fd < 0) continue;
        loft_sock_cloexec(fd);
        if (!loft_sock_connect_within(fd, ai->ai_addr, (socklen_t) ai->ai_addrlen, 30000)) {
            loft_sock_close(fd);
            fd = -1;
        }
    }
    freeaddrinfo(res);
    if (fd < 0) {
        loft_ssh_trace("connect", "no address of the host accepted the connection");
        return -1;
    }
    loft_sock_timeout_opt(fd, SO_SNDTIMEO, 30000);
    struct loft_ssh *s = loft_ssh_open(fd, why, sizeof why);
    if (!s) {
        loft_ssh_trace("connect", why);
        loft_sock_close(fd);
        return -1;
    }
    if (loft_ssh_n == loft_ssh_cap) {
        size_t cap = loft_ssh_cap ? loft_ssh_cap * 2 : 8;
        struct loft_ssh **t = (struct loft_ssh **) realloc(loft_ssh_tab, cap * sizeof *t);
        if (!t) {
            loft_ssh_close(s);
            return -1;
        }
        loft_ssh_tab = t;
        loft_ssh_cap = cap;
    }
    loft_ssh_tab[loft_ssh_n] = s;
    return (int64_t) loft_ssh_n++;
}

static uint8_t loft_ssh_login_h(int64_t h, loft_str user, loft_str pw) {
    struct loft_ssh *s = loft_ssh_at(h);
    if (!s) return 0;
    int ok = loft_ssh_login(s, user.p, user.len, pw.p, pw.len);
    if (!ok) loft_ssh_trace("login", loft_ssh_error(s)[0] ? loft_ssh_error(s) : "the server refused the password");
    return (uint8_t) ok;
}

static uint8_t loft_ssh_shell_h(int64_t h, loft_str term, int64_t cols, int64_t rows) {
    struct loft_ssh *s = loft_ssh_at(h);
    if (!s) return 0;
    int ok = loft_ssh_shell(s, term.p, term.len, loft_ssh_dim(cols), loft_ssh_dim(rows));
    if (!ok) loft_ssh_trace("open_shell", loft_ssh_error(s)[0] ? loft_ssh_error(s) : "not logged in, or the server refused the shell");
    return (uint8_t) ok;
}

static void loft_ssh_send_h(int64_t h, loft_str data) {
    struct loft_ssh *s = loft_ssh_at(h);
    if (s) loft_ssh_write(s, data.p, data.len);
}

/* What arrived since the last call, as a text the caller owns. */
static loft_text loft_ssh_recv_h(int64_t h) {
    loft_text t = { 0, 0, 0 };
    struct loft_ssh *s = loft_ssh_at(h);
    if (!s) return t;
    const unsigned char *p;
    size_t n = loft_ssh_read(s, &p);
    if (n) loft_push(&t, (const char *) p, n);
    return t;
}

static void loft_ssh_resize_h(int64_t h, int64_t cols, int64_t rows) {
    struct loft_ssh *s = loft_ssh_at(h);
    if (s) loft_ssh_resize(s, loft_ssh_dim(cols), loft_ssh_dim(rows));
}

static uint8_t loft_ssh_is_open_h(int64_t h) {
    struct loft_ssh *s = loft_ssh_at(h);
    return (uint8_t) (s && loft_ssh_is_open(s));
}

static void loft_ssh_close_h(int64_t h) {
    struct loft_ssh *s = loft_ssh_at(h);
    if (!s) return;
    loft_ssh_tab[h] = 0;
    loft_ssh_close(s);
}

#endif
