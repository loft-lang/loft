/* A small SSH server for tests/ssh_natives.rs: the INTERACTIONS of the ssh natives — a refused
 * password, the shell's start, a resize, bytes both ways through the channel's flow control,
 * the shell's exit — on every host, with no sshd.  It is not a check of the protocol: it is
 * built on the client's own packet layer (`src/c/tls/loft_ssh.c`, compiled in) and the same
 * BearSSL, so the two agree by construction; the protocol is checked against OpenSSH (the
 * probe in tests/ssh_natives.rs's doc).
 *
 * Host keys: ecdsa-sha2-nistp256 (a fixed test key) unless `argv[2]`, a comma list, names
 * another for a connection (`rsa1024`, `rsa2048`, signing rsa-sha2-256); one cipher
 * (aes256-gcm@openssh.com); user `u` with password `right`.  It binds 127.0.0.1 on a free
 * port, prints `PORT <n>`, and serves the `argv[1]` connections it is told to, one after
 * another.  The "shell" is a script read from the channel line by line:
 *   echo:<text>   answers <text>
 *   bytes         answers the 256 byte values, then a newline
 *   big:<n>       answers n bytes 'x' through the client's window, then `BIGDONE`
 *   count:<n>     reads the next n bytes raw and answers `COUNT <n> <sum of the bytes>`
 *   rekey:same    starts a key exchange with the key the session holds, then answers `REKEYED`
 *   rekey:other   the same with another ECDSA key; a client that ends the session gets nothing
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

/* A second ECDSA key, which a rekey may bring in place of the first. */
static unsigned char srv_key2[32] = { 0x2a, 0x39, 0x48, 0x57, 0x66, 0x75, 0x84, 0x93, 0xa2, 0xb1, 0xc0,
                                      0xcf, 0xde, 0xed, 0xfc, 0x0b, 0x1a, 0x29, 0x38, 0x47, 0x56, 0x65,
                                      0x74, 0x83, 0x92, 0xa1, 0xb0, 0xbf, 0xce, 0xdd, 0xec, 0x3b };

/* Two RSA host keys, test keys made for this server alone: one of 1024 bits, which a client
 * should refuse, and one of 2048. */
/* RSA-1024 */
static const unsigned char rsa1024_p[64] = {
    0xee, 0x0f, 0x24, 0x05, 0x4f, 0xea, 0x5d, 0xad, 0xeb, 0xdb, 0xab, 0xed, 0xec, 0xfd, 0x94, 0xe6,
    0xd5, 0xfe, 0x19, 0x01, 0xf3, 0x98, 0x10, 0xb1, 0x04, 0x2b, 0x23, 0x5d, 0x71, 0x63, 0xfe, 0x46,
    0x96, 0x15, 0x24, 0xdd, 0xf0, 0xb9, 0x52, 0xa7, 0xa8, 0x75, 0xfb, 0xa1, 0x7c, 0x92, 0xbb, 0x63,
    0x39, 0x59, 0x5d, 0x85, 0xfe, 0x0a, 0x9a, 0xdf, 0x08, 0x12, 0x57, 0x54, 0xdd, 0x5a, 0x81, 0x9b,
};

static const unsigned char rsa1024_q[64] = {
    0xd9, 0x5a, 0x23, 0x02, 0x93, 0x25, 0x1b, 0x5c, 0x8e, 0x70, 0xe5, 0x25, 0x02, 0xea, 0x38, 0x40,
    0xc6, 0x45, 0xa0, 0x25, 0x36, 0x18, 0xf2, 0xfd, 0x6e, 0x55, 0x30, 0xae, 0xb8, 0x87, 0xdd, 0xd0,
    0x20, 0xcc, 0x3e, 0x95, 0xb0, 0xe6, 0xfa, 0xf9, 0xee, 0xd4, 0xf5, 0xbb, 0x4c, 0x5f, 0x1a, 0x0d,
    0x50, 0xc7, 0x11, 0x9d, 0xc0, 0xa6, 0x6f, 0xb0, 0xaa, 0xb8, 0xa6, 0x48, 0x67, 0x17, 0xfc, 0x95,
};

static const unsigned char rsa1024_dp[64] = {
    0x6e, 0x1b, 0x42, 0xa5, 0xd2, 0x58, 0xac, 0x0b, 0x8d, 0x53, 0x85, 0x29, 0xfe, 0xbb, 0x2a, 0xeb,
    0xb9, 0x7e, 0x55, 0x42, 0xb9, 0xd0, 0x53, 0xa8, 0x0c, 0x2d, 0x87, 0xa5, 0xe8, 0x94, 0x3f, 0x3f,
    0x96, 0x9b, 0x20, 0xfa, 0x26, 0x25, 0x11, 0x3d, 0xca, 0x85, 0x07, 0x65, 0xa1, 0xa0, 0x9c, 0x5b,
    0x40, 0x63, 0x3d, 0x06, 0x84, 0x81, 0x93, 0x30, 0x25, 0x23, 0x96, 0x91, 0x01, 0xf0, 0xa2, 0x91,
};

static const unsigned char rsa1024_dq[64] = {
    0x26, 0xc6, 0xb9, 0xf0, 0x3f, 0xa8, 0x23, 0x15, 0x84, 0x24, 0x07, 0xfd, 0x6e, 0x63, 0xb2, 0xdd,
    0x7b, 0x81, 0xf8, 0x6c, 0x2b, 0x16, 0x90, 0x63, 0x3e, 0x47, 0xdc, 0x83, 0x8f, 0x50, 0x4d, 0xf6,
    0x4f, 0xa3, 0xa0, 0x96, 0xbc, 0x14, 0xb9, 0x39, 0xd5, 0xae, 0x10, 0x98, 0xf5, 0xab, 0x11, 0xce,
    0xc6, 0x56, 0x7d, 0x82, 0x13, 0x58, 0x9e, 0x1c, 0x7a, 0x98, 0xae, 0xb1, 0xc4, 0x12, 0xb7, 0x71,
};

static const unsigned char rsa1024_iq[64] = {
    0x8c, 0xf6, 0x41, 0x2b, 0xa0, 0x63, 0x07, 0x82, 0xa5, 0x34, 0x8d, 0x8d, 0xaa, 0xaf, 0xf2, 0x83,
    0x40, 0xce, 0xf3, 0x67, 0x09, 0x93, 0xe7, 0x34, 0x5f, 0x8c, 0x48, 0xd3, 0xc7, 0xbf, 0x14, 0x1e,
    0x3f, 0x97, 0xc1, 0xf2, 0x83, 0x62, 0x95, 0x1f, 0x79, 0x7e, 0x8b, 0x00, 0x20, 0xbf, 0x3c, 0x3b,
    0xc9, 0x4a, 0x58, 0xea, 0x0d, 0x42, 0x74, 0x21, 0x45, 0xbc, 0xd1, 0x7c, 0x1b, 0x6b, 0xc9, 0x13,
};

/* RSA-2048 */
static const unsigned char rsa2048_p[128] = {
    0xdc, 0xb2, 0xa6, 0xd7, 0x8e, 0x39, 0xfd, 0xc7, 0x9e, 0x94, 0x0b, 0x9d, 0xb0, 0xc8, 0x1a, 0x1a,
    0x25, 0x51, 0xe3, 0x1f, 0xec, 0x38, 0xbb, 0xca, 0xfc, 0x22, 0xee, 0xcb, 0x50, 0x6a, 0xb1, 0x08,
    0xef, 0x6b, 0x3f, 0x8e, 0x82, 0xfc, 0x16, 0x8f, 0x19, 0xc7, 0x77, 0x08, 0x71, 0x02, 0x30, 0xd7,
    0xf7, 0x0f, 0x73, 0x69, 0xea, 0xeb, 0x68, 0xd5, 0x22, 0x9a, 0xa2, 0xd1, 0x9a, 0xe9, 0xf5, 0x9b,
    0x18, 0x94, 0xcc, 0x6b, 0x62, 0x35, 0x73, 0xd7, 0x88, 0xc5, 0xc4, 0xba, 0xab, 0x72, 0xc0, 0x24,
    0x16, 0x79, 0x4c, 0xec, 0x4f, 0x79, 0x11, 0xe9, 0x79, 0x6e, 0x8e, 0x41, 0x5f, 0xe1, 0x4f, 0x2f,
    0x3e, 0x20, 0x4e, 0x60, 0xdb, 0x04, 0xa6, 0x09, 0x86, 0xe6, 0x68, 0x4c, 0x8b, 0x1e, 0x98, 0x79,
    0x68, 0x02, 0xba, 0xf1, 0xf4, 0x95, 0xad, 0xec, 0x23, 0x74, 0xbc, 0xda, 0x75, 0x2f, 0x5b, 0x8d,
};

static const unsigned char rsa2048_q[128] = {
    0xd6, 0x4d, 0x7d, 0xa1, 0xbc, 0xb8, 0x04, 0x2a, 0xbd, 0x53, 0x30, 0x20, 0x2b, 0x72, 0x7d, 0x7a,
    0x27, 0xa3, 0x94, 0x2c, 0x2a, 0x3e, 0x19, 0x1b, 0xc0, 0xbd, 0x39, 0x36, 0xf5, 0x76, 0xbe, 0xca,
    0xdd, 0x16, 0xfa, 0x85, 0xb4, 0xa2, 0xb0, 0x33, 0xe8, 0xf8, 0xc0, 0xd3, 0x06, 0x6e, 0xd7, 0x13,
    0x62, 0x71, 0x27, 0xda, 0xb9, 0xcb, 0x20, 0x79, 0xf6, 0xad, 0x6e, 0x5d, 0x31, 0x7a, 0xb0, 0x66,
    0xde, 0x33, 0x91, 0x21, 0x5e, 0x86, 0x93, 0x11, 0x38, 0x04, 0xb3, 0xd0, 0x8b, 0x1f, 0xcb, 0x4b,
    0xce, 0x5a, 0x9f, 0x5d, 0x63, 0xf7, 0x28, 0xeb, 0x18, 0xf5, 0xc8, 0xe4, 0xae, 0xed, 0x92, 0xf7,
    0xab, 0x40, 0x8e, 0x02, 0x21, 0x67, 0x24, 0xc3, 0x1c, 0xf3, 0xa2, 0xd4, 0xee, 0x18, 0xa1, 0x24,
    0xfa, 0xc8, 0x91, 0x73, 0x3e, 0x9d, 0xce, 0x6b, 0xc1, 0x25, 0x3e, 0x6a, 0x4d, 0x80, 0xe6, 0x59,
};

static const unsigned char rsa2048_dp[128] = {
    0x54, 0x08, 0x8a, 0x52, 0xa5, 0x4b, 0x1b, 0x80, 0x78, 0xb7, 0x32, 0x3d, 0x16, 0xe4, 0xfa, 0x5b,
    0x5e, 0x3e, 0x71, 0xd6, 0xe3, 0xe1, 0x2e, 0x50, 0x39, 0xf5, 0x2b, 0x2b, 0xab, 0x31, 0x14, 0xbb,
    0xac, 0x3e, 0x0f, 0xf3, 0x39, 0x98, 0xb7, 0x16, 0x41, 0xe0, 0x9b, 0x48, 0x51, 0x8f, 0x5d, 0xfd,
    0xbd, 0x8a, 0xda, 0x8d, 0x47, 0xef, 0xde, 0xf8, 0xd8, 0xb6, 0x3c, 0xac, 0xc6, 0xe9, 0xb2, 0xa5,
    0x1d, 0x94, 0xfa, 0xc9, 0x20, 0x99, 0xed, 0xc3, 0xf1, 0xc6, 0x2c, 0x04, 0x4f, 0xe7, 0xdf, 0x95,
    0x5e, 0x21, 0x4d, 0xd2, 0xe4, 0xe7, 0xda, 0x94, 0x25, 0xb2, 0x9c, 0xe3, 0xf3, 0x98, 0x1c, 0xc8,
    0x86, 0x45, 0x52, 0xd2, 0xba, 0x52, 0x68, 0xce, 0x53, 0xd9, 0x43, 0x79, 0x71, 0xb7, 0x72, 0xa3,
    0x37, 0xa1, 0x62, 0x81, 0x62, 0x79, 0xe3, 0xc1, 0x78, 0x36, 0xa8, 0xfa, 0x1d, 0x88, 0xac, 0xb9,
};

static const unsigned char rsa2048_dq[128] = {
    0xb5, 0x94, 0x8f, 0x40, 0xbd, 0xd1, 0x05, 0xe6, 0xd8, 0xe8, 0x65, 0xbc, 0xbc, 0x53, 0x94, 0x32,
    0x2e, 0xe8, 0x11, 0xa2, 0xa2, 0x60, 0x5f, 0x91, 0xfe, 0x6b, 0xd6, 0xd0, 0xd4, 0x87, 0xba, 0xfd,
    0xd4, 0xfb, 0xa5, 0x66, 0x29, 0x3a, 0x38, 0x26, 0xa3, 0xde, 0x32, 0x34, 0x1c, 0x1d, 0x4a, 0xab,
    0xa8, 0x1b, 0xcc, 0xf3, 0x2d, 0xbe, 0x09, 0x54, 0x8e, 0x49, 0x41, 0x5c, 0x41, 0x92, 0x7d, 0x69,
    0x8c, 0x19, 0x11, 0x7d, 0xbe, 0xc4, 0x34, 0x54, 0xc0, 0xac, 0x6b, 0x57, 0xe6, 0xaa, 0x17, 0xed,
    0xb1, 0x79, 0x63, 0xd3, 0x13, 0xe2, 0xd6, 0x8e, 0xed, 0x65, 0x44, 0xc5, 0x4f, 0xdb, 0x09, 0x4e,
    0x45, 0x92, 0xbb, 0x40, 0xde, 0xe0, 0xb0, 0x6b, 0x8a, 0x5a, 0x7d, 0xbc, 0x01, 0xca, 0x68, 0xb0,
    0x16, 0xb6, 0x5f, 0x57, 0xd0, 0xd4, 0x91, 0x95, 0x07, 0xe0, 0xf0, 0xd3, 0xcd, 0xbb, 0xcc, 0x71,
};

static const unsigned char rsa2048_iq[128] = {
    0x05, 0x5c, 0xcb, 0xcf, 0x3c, 0xe8, 0x74, 0x27, 0xc0, 0x54, 0x72, 0x50, 0x0f, 0x1e, 0x86, 0x25,
    0x99, 0xd0, 0x92, 0xce, 0xe9, 0xe9, 0xc4, 0xf1, 0xcb, 0x6f, 0x4c, 0x1c, 0x09, 0x4c, 0xfd, 0x0f,
    0x15, 0xb1, 0x46, 0x93, 0xfc, 0x81, 0x40, 0x5e, 0x92, 0x59, 0x36, 0x28, 0x55, 0xa7, 0xac, 0x77,
    0x72, 0x63, 0x6b, 0x2d, 0x00, 0x9b, 0x35, 0xed, 0x74, 0x06, 0xfb, 0x03, 0x1d, 0x1f, 0xa5, 0xd3,
    0xe8, 0x01, 0x08, 0xbb, 0xda, 0xdd, 0x96, 0xfc, 0x8a, 0x3e, 0x16, 0xcc, 0xd1, 0x60, 0x9c, 0x37,
    0x87, 0x9f, 0x93, 0x1b, 0x54, 0x93, 0x53, 0xa0, 0x0b, 0x7e, 0x37, 0x08, 0xbf, 0x40, 0xa1, 0x85,
    0x10, 0xcd, 0x96, 0x52, 0xb1, 0xc1, 0xaf, 0x73, 0x53, 0x74, 0xf3, 0x89, 0xee, 0xb6, 0x8e, 0x62,
    0xfe, 0xe6, 0x6a, 0xbb, 0x6d, 0x2f, 0xe1, 0x70, 0x3c, 0x5a, 0x38, 0x03, 0x54, 0xfb, 0x3e, 0xb9,
};


/* The host keys this server can hold: which one a connection starts with is named on the
 * command line, and a rekey may bring another. */
enum { HK_ECDSA, HK_ECDSA2, HK_RSA1024, HK_RSA2048 };

static br_rsa_private_key srv_rsa(int kind) {
    br_rsa_private_key sk;
    int big = kind == HK_RSA2048;
    sk.n_bitlen = big ? 2048 : 1024;
    sk.p = (unsigned char *) (big ? rsa2048_p : rsa1024_p);
    sk.plen = big ? sizeof rsa2048_p : sizeof rsa1024_p;
    sk.q = (unsigned char *) (big ? rsa2048_q : rsa1024_q);
    sk.qlen = big ? sizeof rsa2048_q : sizeof rsa1024_q;
    sk.dp = (unsigned char *) (big ? rsa2048_dp : rsa1024_dp);
    sk.dplen = big ? sizeof rsa2048_dp : sizeof rsa1024_dp;
    sk.dq = (unsigned char *) (big ? rsa2048_dq : rsa1024_dq);
    sk.dqlen = big ? sizeof rsa2048_dq : sizeof rsa1024_dq;
    sk.iq = (unsigned char *) (big ? rsa2048_iq : rsa1024_iq);
    sk.iqlen = big ? sizeof rsa2048_iq : sizeof rsa1024_iq;
    return sk;
}

typedef struct {
    struct loft_ssh *s;
    int host; /* the host key this connection holds (HK_*) */
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

static int srv_kex(struct loft_ssh *s, int host) {
    int rsa = host == HK_RSA1024 || host == HK_RSA2048;
    unsigned char *key = host == HK_ECDSA2 ? srv_key2 : srv_key;
    int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
    sbuf is = { 0, 0, 0 }, ic = { 0, 0, 0 }, p = { 0, 0, 0 }, hb = { 0, 0, 0 }, kenc = { 0, 0, 0 }, ks = { 0, 0, 0 },
         sg = { 0, 0, 0 }, rs = { 0, 0, 0 };
    unsigned char cookie[16];
    ssh_random(s, cookie, 16);
    sb_u8(&is, MSG_KEXINIT);
    sb_put(&is, cookie, 16);
    sb_cstr(&is, "curve25519-sha256,kex-strict-s-v00@openssh.com");
    sb_cstr(&is, rsa ? "rsa-sha2-256" : "ecdsa-sha2-nistp256");
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
    br_rsa_private_key rk = srv_rsa(host);
    unsigned char modulus[256];
    size_t mn = rsa ? br_rsa_i31_compute_modulus(modulus, &rk) : 0;
    if (rsa) {
        static const unsigned char e[3] = { 0x01, 0x00, 0x01 };
        sb_cstr(&ks, "ssh-rsa");
        sb_mpint(&ks, e, 3);
        sb_mpint(&ks, modulus, mn);
    } else {
        br_ec_p256_m31.mulgen(pub, key, 32, BR_EC_secp256r1);
        sb_cstr(&ks, "ecdsa-sha2-nistp256");
        sb_cstr(&ks, "nistp256");
        sb_str(&ks, pub, 65);
    }
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
    if (rsa) {
        /* rsa-sha2-256: PKCS#1 v1.5 over the exchange hash's SHA-256. */
        unsigned char sig[256];
        if (!br_rsa_i31_pkcs1_sign(BR_HASH_OID_SHA256, hh, 32, &rk, sig)) goto done;
        sb_cstr(&sg, "rsa-sha2-256");
        sb_str(&sg, sig, mn);
    } else {
        br_ec_private_key sk;
        sk.curve = BR_EC_secp256r1;
        sk.x = key;
        sk.xlen = 32;
        size_t rl = br_ecdsa_i31_sign_raw(&br_ec_p256_m31, &br_sha256_vtable, hh, &sk, raw);
        if (rl != 64) goto done;
        sb_mpint(&rs, raw, 32);
        sb_mpint(&rs, raw + 32, 32);
        sb_cstr(&sg, "ecdsa-sha2-nistp256");
        sb_str(&sg, rs.p, rs.len);
    }
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
    } else if (strcmp(l, "rekey:same") == 0 || strcmp(l, "rekey:other") == 0) {
        /* A key exchange the server starts, with the key the session holds or another one. */
        if (strcmp(l, "rekey:other") == 0) v->host = v->host == HK_ECDSA ? HK_ECDSA2 : HK_ECDSA;
        if (srv_kex(v->s, v->host)) {
            srv_say(v, "REKEYED\n");
        } else {
            v->closed = 2;
            v->s->dead = 1;
        }
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
static void srv_serve(int fd, int host) {
    srv v;
    memset(&v, 0, sizeof v);
    v.counting = -1;
    v.host = host;
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
    if (!srv_kex(s, v.host)) goto end;
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
    {
        /* The public half of the host key, for the test to name the key by itself. */
        unsigned char pub[65];
        br_ec_p256_m31.mulgen(pub, srv_key, 32, BR_EC_secp256r1);
        printf("HOSTKEY ");
        for (int i = 0; i < 65; i++) printf("%02x", pub[i]);
        printf("\n");
    }
    printf("PORT %d\n", (int) ntohs(a.sin_port));
    fflush(stdout);
    for (int i = 0; i < conns; i++) {
        int fd = (int) accept(lfd, 0, 0);
        if (fd < 0) break;
        /* The host key of connection i: the i-th name of the comma list in argv[2]
         * (ecdsa, rsa1024, rsa2048), ecdsa past its end. */
        int host = HK_ECDSA;
        if (argc > 2) {
            const char *at = argv[2];
            for (int k = 0; k < i && at; k++) {
                at = strchr(at, ',');
                if (at) at++;
            }
            if (at && strncmp(at, "rsa1024", 7) == 0) host = HK_RSA1024;
            if (at && strncmp(at, "rsa2048", 7) == 0) host = HK_RSA2048;
        }
        srv_serve(fd, host);
    }
    ssh_sock_close(lfd);
    return 0;
}
