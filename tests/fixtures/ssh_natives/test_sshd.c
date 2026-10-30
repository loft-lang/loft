/* A small SSH server for tests/ssh_natives.rs: the INTERACTIONS of the ssh natives — a refused
 * password, the shell's start, a resize, bytes both ways through the channel's flow control,
 * the shell's exit — on every host, with no sshd.  It is not a check of the protocol: it is
 * built on the client's own packet layer (`src/c/tls/loft_ssh.c`, compiled in) and the same
 * BearSSL, so the two agree by construction; the protocol is checked against OpenSSH (the
 * probe in tests/ssh_natives.rs's doc).
 *
 * One host key (ecdsa-sha2-nistp256, a fixed test key), one cipher (aes256-gcm@openssh.com),
 * user `u` with password `right`.  It binds 127.0.0.1 on a free port, prints `PORT <n>`, and
 * serves the connections it is told to, one after another.  The "shell" is a script read
 * from the channel line by line:
 *   echo:<text>   answers <text>
 *   bytes         answers the 256 byte values, then a newline
 *   big:<n>       answers n bytes 'x' through the client's window, then `BIGDONE`
 *   count:<n>     reads the next n bytes raw and answers `COUNT <n> <sum of the bytes>`
 *   exit          sends exit-status 0, EOF and CLOSE
 * A pty request's size and every window-change are answered as `READY <term> <c>x<r>` (at the
 * shell's start) and `SIZE <c>x<r>`.  The server grants an 8 KiB window in 1 KiB packets, so a
 * client's large send must split and wait. */
#include "loft_ssh.c"

#if defined(_WIN32)
typedef int socklen_t;
#else
#include <arpa/inet.h>
#include <netinet/in.h>
#endif

#define SRV_WINDOW 8192u
#define SRV_MAX 1024u

static unsigned char srv_key[32] = { 0x1f, 0x2e, 0x3d, 0x4c, 0x5b, 0x6a, 0x79, 0x88, 0x97, 0xa6, 0xb5,
                                     0xc4, 0xd3, 0xe2, 0xf1, 0x01, 0x12, 0x23, 0x34, 0x45, 0x56, 0x67,
                                     0x78, 0x89, 0x9a, 0xab, 0xbc, 0xcd, 0xde, 0xef, 0x10, 0x21 };

typedef struct {
    struct loft_ssh *s;
    uint32_t peer_id, peer_window, peer_max, window;
    int closed;
    sbuf line;
    long counting; /* raw bytes still to count, -1 when reading lines */
    unsigned long sum, counted;
    char ready[128]; /* the pty request's answer, said once the shell starts */
} srv;

static int srv_msg(struct loft_ssh *s, int64_t deadline, const unsigned char **pl, size_t *pn) {
    for (;;) {
        if (!ssh_next(s, deadline, pl, pn)) return 0;
        unsigned t = (*pl)[0];
        if (t == MSG_IGNORE || t == MSG_DEBUG || t == MSG_UNIMPLEMENTED) continue;
        return 1;
    }
}

static int srv_kex(struct loft_ssh *s) {
    int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
    sbuf is = { 0, 0, 0 }, ic = { 0, 0, 0 }, p = { 0, 0, 0 }, hb = { 0, 0, 0 }, kenc = { 0, 0, 0 }, ks = { 0, 0, 0 },
         sg = { 0, 0, 0 }, rs = { 0, 0, 0 };
    unsigned char cookie[16];
    ssh_random(s, cookie, 16);
    sb_u8(&is, MSG_KEXINIT);
    sb_put(&is, cookie, 16);
    sb_cstr(&is, "curve25519-sha256,kex-strict-s-v00@openssh.com");
    sb_cstr(&is, "ecdsa-sha2-nistp256");
    sb_cstr(&is, "aes256-gcm@openssh.com");
    sb_cstr(&is, "aes256-gcm@openssh.com");
    sb_cstr(&is, "hmac-sha2-256");
    sb_cstr(&is, "hmac-sha2-256");
    sb_cstr(&is, "none");
    sb_cstr(&is, "none");
    sb_cstr(&is, "");
    sb_cstr(&is, "");
    sb_u8(&is, 0);
    sb_u32(&is, 0);
    int ok = 0;
    if (!ssh_send(s, &is)) goto done;
    const unsigned char *pl;
    size_t pn;
    if (!srv_msg(s, deadline, &pl, &pn) || pl[0] != MSG_KEXINIT) goto done;
    sb_put(&ic, pl, pn);
    {
        rd r = rd_of(ic.p, ic.len);
        const unsigned char *kx;
        size_t kn;
        rd_skip(&r, 17);
        rd_str(&r, &kx, &kn);
        s->strict = !r.bad && ssh_lists(kx, kn, "kex-strict-c-v00@openssh.com");
    }
    if (!srv_msg(s, deadline, &pl, &pn) || pl[0] != MSG_KEX_ECDH_INIT) goto done;
    rd r = rd_of(pl + 1, pn - 1);
    const unsigned char *qc;
    size_t qcn;
    rd_str(&r, &qc, &qcn);
    if (r.bad || qcn != 32) goto done;
    unsigned char qcv[32], priv[32], qs[32], shared[32], pub[65];
    memcpy(qcv, qc, 32);
    ssh_random(s, priv, 32);
    br_ec_c25519_m31.mulgen(qs, priv, 32, BR_EC_curve25519);
    memcpy(shared, qcv, 32);
    if (!br_ec_c25519_m31.mul(shared, 32, priv, 32, BR_EC_curve25519)) goto done;
    br_ec_p256_m31.mulgen(pub, srv_key, 32, BR_EC_secp256r1);
    sb_cstr(&ks, "ecdsa-sha2-nistp256");
    sb_cstr(&ks, "nistp256");
    sb_str(&ks, pub, 65);
    sb_mpint(&kenc, shared, 32);
    sb_str(&hb, s->v_c.p, s->v_c.len);
    sb_str(&hb, s->v_s.p, s->v_s.len);
    sb_str(&hb, ic.p, ic.len);
    sb_str(&hb, is.p, is.len);
    sb_str(&hb, ks.p, ks.len);
    sb_str(&hb, qcv, 32);
    sb_str(&hb, qs, 32);
    sb_put(&hb, kenc.p, kenc.len);
    unsigned char h[32], hh[32], raw[132];
    br_sha256_context hc;
    br_sha256_init(&hc);
    br_sha256_update(&hc, hb.p, hb.len);
    br_sha256_out(&hc, h);
    br_sha256_init(&hc);
    br_sha256_update(&hc, h, 32);
    br_sha256_out(&hc, hh);
    br_ec_private_key sk;
    sk.curve = BR_EC_secp256r1;
    sk.x = srv_key;
    sk.xlen = 32;
    size_t rl = br_ecdsa_i31_sign_raw(&br_ec_p256_m31, &br_sha256_vtable, hh, &sk, raw);
    if (rl != 64) goto done;
    sb_mpint(&rs, raw, 32);
    sb_mpint(&rs, raw + 32, 32);
    sb_cstr(&sg, "ecdsa-sha2-nistp256");
    sb_str(&sg, rs.p, rs.len);
    if (!s->kexed) memcpy(s->session_id, h, 32);
    sb_u8(&p, MSG_KEX_ECDH_REPLY);
    sb_str(&p, ks.p, ks.len);
    sb_str(&p, qs, 32);
    sb_str(&p, sg.p, sg.len);
    if (!ssh_send(s, &p)) goto done;
    sdir ntx, nrx;
    ssh_keys(s, &ntx, "aes256-gcm@openssh.com", "", &kenc, h, 'B', 'D', 'F');
    ssh_keys(s, &nrx, "aes256-gcm@openssh.com", "", &kenc, h, 'A', 'C', 'E');
    p.len = 0;
    sb_u8(&p, MSG_NEWKEYS);
    if (!ssh_send(s, &p)) goto done;
    s->tx = ntx;
    if (s->strict) s->tx_seq = 0;
    if (!srv_msg(s, deadline, &pl, &pn) || pl[0] != MSG_NEWKEYS) goto done;
    s->rx = nrx;
    if (s->strict) s->rx_seq = 0;
    s->kexed = 1;
    ok = 1;
done:
    free(is.p);
    free(ic.p);
    free(p.p);
    free(hb.p);
    free(kenc.p);
    free(ks.p);
    free(sg.p);
    free(rs.p);
    return ok;
}

static int srv_reply(srv *v, unsigned type) {
    sbuf p = { 0, 0, 0 };
    int ok = sb_u8(&p, type) && sb_u32(&p, v->peer_id) && ssh_send(v->s, &p);
    free(p.p);
    return ok;
}

static void srv_handle(srv *v, const unsigned char *pl, size_t pn);

/* Data to the client through its window, in its packet size, reading its adjustments. */
static int srv_out(srv *v, const void *data, size_t n) {
    const unsigned char *d = (const unsigned char *) data;
    sbuf p = { 0, 0, 0 };
    while (n) {
        while (v->peer_window == 0) {
            const unsigned char *pl;
            size_t pn;
            if (!srv_msg(v->s, ssh_now_ms() + SSH_STEP_MS, &pl, &pn)) { free(p.p); return 0; }
            srv_handle(v, pl, pn);
        }
        size_t c = n;
        if (c > v->peer_window) c = v->peer_window;
        if (c > v->peer_max) c = v->peer_max;
        p.len = 0;
        if (!sb_u8(&p, MSG_CHANNEL_DATA) || !sb_u32(&p, v->peer_id) || !sb_str(&p, d, c) || !ssh_send(v->s, &p)) {
            free(p.p);
            return 0;
        }
        v->peer_window -= (uint32_t) c;
        d += c;
        n -= c;
    }
    free(p.p);
    return 1;
}

static int srv_say(srv *v, const char *text) { return srv_out(v, text, strlen(text)); }

static void srv_exit(srv *v) {
    sbuf p = { 0, 0, 0 };
    if (sb_u8(&p, MSG_CHANNEL_REQUEST) && sb_u32(&p, v->peer_id) && sb_cstr(&p, "exit-status") && sb_u8(&p, 0) &&
        sb_u32(&p, 0))
        ssh_send(v->s, &p);
    free(p.p);
    srv_reply(v, MSG_CHANNEL_EOF);
    srv_reply(v, MSG_CHANNEL_CLOSE);
    v->closed = 1;
}

static void srv_line(srv *v, const char *l) {
    char buf[64];
    if (strncmp(l, "echo:", 5) == 0) {
        srv_say(v, l + 5);
        srv_say(v, "\n");
    } else if (strcmp(l, "bytes") == 0) {
        unsigned char all[256];
        for (int i = 0; i < 256; i++) all[i] = (unsigned char) i;
        srv_out(v, all, 256);
        srv_say(v, "\n");
    } else if (strncmp(l, "big:", 4) == 0) {
        long n = atol(l + 4);
        static unsigned char xs[4096];
        memset(xs, 'x', sizeof xs);
        while (n > 0) {
            size_t c = n > (long) sizeof xs ? sizeof xs : (size_t) n;
            if (!srv_out(v, xs, c)) return;
            n -= (long) c;
        }
        srv_say(v, "BIGDONE\n");
    } else if (strncmp(l, "count:", 6) == 0) {
        v->counting = atol(l + 6);
        v->sum = 0;
        v->counted = 0;
    } else if (strcmp(l, "exit") == 0) {
        srv_exit(v);
    } else {
        snprintf(buf, sizeof buf, "UNKNOWN %.40s\n", l);
        srv_say(v, buf);
    }
}

static void srv_data(srv *v, const unsigned char *d, size_t n) {
    for (size_t i = 0; i < n && !v->closed; i++) {
        if (v->counting > 0) {
            v->sum += d[i];
            v->counted++;
            if (--v->counting == 0) {
                char buf[64];
                snprintf(buf, sizeof buf, "COUNT %lu %lu\n", v->counted, v->sum);
                srv_say(v, buf);
                v->counting = -1;
            }
            continue;
        }
        if (d[i] == '\n') {
            sb_u8(&v->line, 0);
            srv_line(v, (const char *) v->line.p);
            v->line.len = 0;
        } else {
            sb_u8(&v->line, d[i]);
        }
    }
    /* Grant the window back once half of it is used. */
    v->window -= (uint32_t) n;
    if (v->window < SRV_WINDOW / 2 && !v->closed) {
        sbuf p = { 0, 0, 0 };
        if (sb_u8(&p, MSG_CHANNEL_WINDOW_ADJUST) && sb_u32(&p, v->peer_id) && sb_u32(&p, SRV_WINDOW - v->window))
            ssh_send(v->s, &p);
        free(p.p);
        v->window = SRV_WINDOW;
    }
}

static void srv_handle(srv *v, const unsigned char *pl, size_t pn) {
    rd r = rd_of(pl + 1, pn - 1);
    switch (pl[0]) {
    case MSG_CHANNEL_WINDOW_ADJUST:
        rd_u32(&r);
        v->peer_window += rd_u32(&r);
        break;
    case MSG_CHANNEL_DATA: {
        const unsigned char *d;
        size_t dn;
        rd_u32(&r);
        rd_str(&r, &d, &dn);
        if (!r.bad) srv_data(v, d, dn);
        break;
    }
    case MSG_CHANNEL_REQUEST: {
        const unsigned char *t;
        size_t tn;
        rd_u32(&r);
        rd_str(&r, &t, &tn);
        unsigned want = rd_u8(&r);
        char buf[128];
        if (str_is(t, tn, "pty-req")) {
            const unsigned char *term;
            size_t termn;
            rd_str(&r, &term, &termn);
            uint32_t c = rd_u32(&r), rows = rd_u32(&r);
            snprintf(v->ready, sizeof v->ready, "READY %.*s %ux%u\n", (int) (termn > 40 ? 40 : termn),
                     (const char *) term, c, rows);
            if (want) srv_reply(v, MSG_CHANNEL_SUCCESS);
        } else if (str_is(t, tn, "shell")) {
            if (want) srv_reply(v, MSG_CHANNEL_SUCCESS);
            srv_say(v, v->ready[0] ? v->ready : "READY\n");
        } else if (str_is(t, tn, "window-change")) {
            uint32_t c = rd_u32(&r), rows = rd_u32(&r);
            snprintf(buf, sizeof buf, "SIZE %ux%u\n", c, rows);
            srv_say(v, buf);
        } else if (want) {
            srv_reply(v, MSG_CHANNEL_FAILURE);
        }
        break;
    }
    case MSG_CHANNEL_EOF:
        break;
    case MSG_CHANNEL_CLOSE:
        if (!v->closed) srv_reply(v, MSG_CHANNEL_CLOSE);
        v->closed = 2;
        break;
    case MSG_DISCONNECT:
        v->closed = 2;
        v->s->dead = 1;
        break;
    default:
        break;
    }
}

/* One connection, start to end. */
static void srv_serve(int fd) {
    srv v;
    memset(&v, 0, sizeof v);
    v.counting = -1;
    struct loft_ssh *s = (struct loft_ssh *) calloc(1, sizeof *s);
    v.s = s;
    s->fd = fd;
    br_prng_seeder seed = br_prng_seeder_system(0);
    br_hmac_drbg_init(&s->rng, &br_sha256_vtable, 0, 0);
    if (!seed || !seed(&s->rng.vtable)) goto end;
    sb_put(&s->v_s, "SSH-2.0-loft_test_sshd", 22);
    if (!ssh_write_all(s, (const unsigned char *) "SSH-2.0-loft_test_sshd\r\n", 24)) goto end;
    int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
    for (;;) {
        unsigned char *nl = s->in.len > s->in_at ? (unsigned char *) memchr(s->in.p + s->in_at, '\n', s->in.len - s->in_at) : 0;
        if (nl) {
            size_t ll = (size_t) (nl - (s->in.p + s->in_at));
            sb_put(&s->v_c, s->in.p + s->in_at, ll && nl[-1] == '\r' ? ll - 1 : ll);
            s->in_at += ll + 1;
            break;
        }
        if (ssh_now_ms() > deadline || ssh_fill(s, 1000) < 0) goto end;
    }
    if (!srv_kex(s)) goto end;
    const unsigned char *pl;
    size_t pn;
    int authed = 0;
    while (!s->dead && v.closed < 2) {
        if (!srv_msg(s, ssh_now_ms() + 60000, &pl, &pn)) break;
        rd r = rd_of(pl + 1, pn - 1);
        sbuf p = { 0, 0, 0 };
        switch (pl[0]) {
        case MSG_SERVICE_REQUEST:
            sb_u8(&p, MSG_SERVICE_ACCEPT);
            sb_cstr(&p, "ssh-userauth");
            ssh_send(s, &p);
            break;
        case MSG_USERAUTH_REQUEST: {
            const unsigned char *u, *svc, *m, *pw;
            size_t un, sn, mn, pwn;
            rd_str(&r, &u, &un);
            rd_str(&r, &svc, &sn);
            rd_str(&r, &m, &mn);
            int good = 0;
            if (str_is(m, mn, "password")) {
                rd_u8(&r);
                rd_str(&r, &pw, &pwn);
                good = !r.bad && str_is(u, un, "u") && str_is(pw, pwn, "right");
            }
            if (good) {
                authed = 1;
                sb_u8(&p, MSG_USERAUTH_SUCCESS);
            } else {
                sb_u8(&p, MSG_USERAUTH_FAILURE);
                sb_cstr(&p, "password");
                sb_u8(&p, 0);
            }
            ssh_send(s, &p);
            break;
        }
        case MSG_CHANNEL_OPEN:
            if (!authed) break;
            {
                const unsigned char *t;
                size_t tn;
                rd_str(&r, &t, &tn);
                v.peer_id = rd_u32(&r);
                v.peer_window = rd_u32(&r);
                v.peer_max = rd_u32(&r);
                v.window = SRV_WINDOW;
                sb_u8(&p, MSG_CHANNEL_OPEN_CONFIRMATION);
                sb_u32(&p, v.peer_id);
                sb_u32(&p, 0);
                sb_u32(&p, SRV_WINDOW);
                sb_u32(&p, SRV_MAX);
                ssh_send(s, &p);
            }
            break;
        default:
            srv_handle(&v, pl, pn);
            break;
        }
        free(p.p);
    }
end:
    ssh_sock_close(fd);
    free(s->in.p);
    free(s->v_c.p);
    free(s->v_s.p);
    free(s);
    free(v.line.p);
}

int main(int argc, char **argv) {
    int conns = argc > 1 ? atoi(argv[1]) : 1;
#if defined(_WIN32)
    WSADATA wsa;
    WSAStartup(MAKEWORD(2, 2), &wsa);
#endif
    int lfd = (int) socket(AF_INET, SOCK_STREAM, 0);
    struct sockaddr_in a;
    memset(&a, 0, sizeof a);
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(0x7f000001);
    a.sin_port = 0;
    socklen_t al = (socklen_t) sizeof a;
    if (bind(lfd, (struct sockaddr *) &a, al) != 0 || listen(lfd, 4) != 0 ||
        getsockname(lfd, (struct sockaddr *) &a, &al) != 0) {
        fprintf(stderr, "test_sshd: cannot listen\n");
        return 1;
    }
    printf("PORT %d\n", (int) ntohs(a.sin_port));
    fflush(stdout);
    for (int i = 0; i < conns; i++) {
        int fd = (int) accept(lfd, 0, 0);
        if (fd < 0) break;
        srv_serve(fd);
    }
    ssh_sock_close(lfd);
    return 0;
}
