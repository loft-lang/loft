/* The SSH client of loft's C runtime: the ssh library's natives (`src/c/loft_ssh.h` in the `net`
 * component) over a socket the caller connected.  It lives beside `loft_tls.c` because it is
 * built from the same vendored BearSSL (`bearssl/`): SHA-2, HMAC, AES, GHASH, Curve25519, P-256
 * and RSA come from there, and only Ed25519 — which BearSSL lacks — is written here (in the
 * public-domain TweetNaCl formulation).  It is not part of a unit's translation unit:
 * `src/c/tls.rs` builds it into the TLS archive with the BearSSL sources.
 *
 * What it speaks — the slice of SSH-2 a remote interactive shell needs, and nothing more:
 *   - key exchange curve25519-sha256 (RFC 8731), with the strict-kex extension that closes the
 *     prefix-truncation attack (`kex-strict-c-v00@openssh.com`), rekeying when the server asks;
 *   - host keys ssh-ed25519, ecdsa-sha2-nistp256, rsa-sha2-512 and rsa-sha2-256 — the
 *     signature over the exchange is always verified.  WHICH key the server holds is not
 *     checked against a known-hosts store: the session is authenticated by password over the
 *     encrypted transport, as the library's Rust client did (a recorded first-use check is the
 *     hardening step left);
 *   - ciphers aes256-gcm@openssh.com and aes128-gcm@openssh.com, else AES-CTR with
 *     HMAC-SHA2-256/512 (encrypt-then-MAC preferred) — what OpenSSH, Dropbear and the common
 *     libraries all offer;
 *   - password authentication, one session channel with a pty and a shell, window-change, and
 *     the channel's flow control in both directions.
 * Every blocking step has a deadline; reading the shell's output never blocks. */
#if defined(_WIN32)
#define WIN32_LEAN_AND_MEAN 1
#define NOMINMAX 1
#include <winsock2.h>
#include <ws2tcpip.h>
#if defined(_MSC_VER)
#pragma comment(lib, "ws2_32.lib")
#endif
#else
#define _POSIX_C_SOURCE 200809L
#include <poll.h>
#include <sys/socket.h>
#include <time.h>
#endif
#include "bearssl.h"

#include <errno.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#if defined(_WIN32)
static long ssh_sock_recv(int fd, void *b, size_t n) { return recv((SOCKET) fd, (char *) b, (int) n, 0); }
static long ssh_sock_send(int fd, const void *b, size_t n) { return send((SOCKET) fd, (const char *) b, (int) n, 0); }
static int ssh_sock_interrupted(void) { return WSAGetLastError() == WSAEINTR; }
static int ssh_sock_wait(int fd, int ms) {
    WSAPOLLFD p = { (SOCKET) fd, POLLIN, 0 };
    return WSAPoll(&p, 1, ms);
}
static void ssh_sock_close(int fd) { closesocket((SOCKET) fd); }
static int64_t ssh_now_ms(void) { return (int64_t) GetTickCount64(); }
#else
#ifndef MSG_NOSIGNAL
#define MSG_NOSIGNAL 0
#endif
#include <unistd.h>
static long ssh_sock_recv(int fd, void *b, size_t n) { return (long) recv(fd, b, n, 0); }
static long ssh_sock_send(int fd, const void *b, size_t n) { return (long) send(fd, b, n, MSG_NOSIGNAL); }
static int ssh_sock_interrupted(void) { return errno == EINTR; }
static int ssh_sock_wait(int fd, int ms) {
    struct pollfd p = { fd, POLLIN, 0 };
    return poll(&p, 1, ms);
}
static void ssh_sock_close(int fd) { close(fd); }
static int64_t ssh_now_ms(void) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t) ts.tv_sec * 1000 + ts.tv_nsec / 1000000;
}
#endif

/* ── Messages ── */

enum {
    MSG_DISCONNECT = 1, MSG_IGNORE = 2, MSG_UNIMPLEMENTED = 3, MSG_DEBUG = 4, MSG_SERVICE_REQUEST = 5,
    MSG_SERVICE_ACCEPT = 6, MSG_EXT_INFO = 7, MSG_KEXINIT = 20, MSG_NEWKEYS = 21, MSG_KEX_ECDH_INIT = 30,
    MSG_KEX_ECDH_REPLY = 31, MSG_USERAUTH_REQUEST = 50, MSG_USERAUTH_FAILURE = 51, MSG_USERAUTH_SUCCESS = 52,
    MSG_USERAUTH_BANNER = 53, MSG_USERAUTH_PASSWD_CHANGEREQ = 60, MSG_GLOBAL_REQUEST = 80,
    MSG_REQUEST_FAILURE = 82, MSG_CHANNEL_OPEN = 90, MSG_CHANNEL_OPEN_CONFIRMATION = 91,
    MSG_CHANNEL_OPEN_FAILURE = 92, MSG_CHANNEL_WINDOW_ADJUST = 93, MSG_CHANNEL_DATA = 94,
    MSG_CHANNEL_EXTENDED_DATA = 95, MSG_CHANNEL_EOF = 96, MSG_CHANNEL_CLOSE = 97, MSG_CHANNEL_REQUEST = 98,
    MSG_CHANNEL_SUCCESS = 99, MSG_CHANNEL_FAILURE = 100
};

/* What this client offers, most preferred first. */
#define SSH_KEX "curve25519-sha256,curve25519-sha256@libssh.org,ext-info-c,kex-strict-c-v00@openssh.com"
#define SSH_HOSTKEYS "ssh-ed25519,ecdsa-sha2-nistp256,rsa-sha2-512,rsa-sha2-256"
#define SSH_CIPHERS "aes256-gcm@openssh.com,aes128-gcm@openssh.com,aes256-ctr,aes192-ctr,aes128-ctr"
#define SSH_MACS "hmac-sha2-256-etm@openssh.com,hmac-sha2-512-etm@openssh.com,hmac-sha2-256,hmac-sha2-512"
#define SSH_VERSION "SSH-2.0-loft_1.0"

#define SSH_STEP_MS 30000               /* the deadline of one blocking step */
#define SSH_WINDOW (2u * 1024u * 1024u) /* the receive window this client grants */
#define SSH_MAX_PACKET 32768u           /* the largest data packet it accepts */
#define SSH_MAX_WIRE (256u * 1024u)     /* the largest packet it reads at all */

/* ── Byte buffers ── */

typedef struct {
    unsigned char *p;
    size_t len, cap;
} sbuf;

static int sb_reserve(sbuf *b, size_t more) {
    if (b->len + more <= b->cap) return 1;
    size_t cap = b->cap ? b->cap : 256;
    while (cap < b->len + more) cap *= 2;
    unsigned char *p = (unsigned char *) realloc(b->p, cap);
    if (!p) return 0;
    b->p = p;
    b->cap = cap;
    return 1;
}
static int sb_put(sbuf *b, const void *p, size_t n) {
    if (!sb_reserve(b, n)) return 0;
    if (n) memcpy(b->p + b->len, p, n);
    b->len += n;
    return 1;
}
static void put32(unsigned char *p, uint32_t v) {
    p[0] = (unsigned char) (v >> 24);
    p[1] = (unsigned char) (v >> 16);
    p[2] = (unsigned char) (v >> 8);
    p[3] = (unsigned char) v;
}
static uint32_t get32(const unsigned char *p) {
    return ((uint32_t) p[0] << 24) | ((uint32_t) p[1] << 16) | ((uint32_t) p[2] << 8) | (uint32_t) p[3];
}
static int sb_u8(sbuf *b, unsigned v) {
    unsigned char c = (unsigned char) v;
    return sb_put(b, &c, 1);
}
static int sb_u32(sbuf *b, uint32_t v) {
    unsigned char c[4];
    put32(c, v);
    return sb_put(b, c, 4);
}
static int sb_str(sbuf *b, const void *p, size_t n) { return sb_u32(b, (uint32_t) n) && sb_put(b, p, n); }
static int sb_cstr(sbuf *b, const char *s) { return sb_str(b, s, strlen(s)); }
/* An unsigned big-endian number as an mpint: no leading zero bytes, one added when the top bit
 * is set. */
static int sb_mpint(sbuf *b, const unsigned char *p, size_t n) {
    while (n && *p == 0) {
        p++;
        n--;
    }
    if (n && (p[0] & 0x80)) return sb_u32(b, (uint32_t) n + 1) && sb_u8(b, 0) && sb_put(b, p, n);
    return sb_str(b, p, n);
}

typedef struct {
    const unsigned char *p;
    size_t n, i;
    int bad;
} rd;

static rd rd_of(const unsigned char *p, size_t n) {
    rd r = { p, n, 0, 0 };
    return r;
}
static unsigned rd_u8(rd *r) {
    if (r->bad || r->i + 1 > r->n) { r->bad = 1; return 0; }
    return r->p[r->i++];
}
static uint32_t rd_u32(rd *r) {
    if (r->bad || r->i + 4 > r->n) { r->bad = 1; return 0; }
    uint32_t v = get32(r->p + r->i);
    r->i += 4;
    return v;
}
static int rd_str(rd *r, const unsigned char **p, size_t *n) {
    uint32_t l = rd_u32(r);
    if (r->bad || l > r->n - r->i) { r->bad = 1; *p = 0; *n = 0; return 0; }
    *p = r->p + r->i;
    *n = l;
    r->i += l;
    return 1;
}
static int rd_skip(rd *r, size_t n) {
    if (r->bad || n > r->n - r->i) { r->bad = 1; return 0; }
    r->i += n;
    return 1;
}
static int str_is(const unsigned char *p, size_t n, const char *s) { return n == strlen(s) && memcmp(p, s, n) == 0; }

/* ── Ed25519 verification (RFC 8032), the TweetNaCl formulation ── */

typedef int64_t gf[16];
static const gf gf0 = { 0 }, gf1 = { 1 },
                D = { 0x78a3, 0x1359, 0x4dca, 0x75eb, 0xd8ab, 0x4141, 0x0a4d, 0x0070,
                      0xe898, 0x7779, 0x4079, 0x8cc7, 0xfe73, 0x2b6f, 0x6cee, 0x5203 },
                D2 = { 0xf159, 0x26b2, 0x9b94, 0xebd6, 0xb156, 0x8283, 0x149a, 0x00e0,
                       0xd130, 0xeef3, 0x80f2, 0x198e, 0xfce7, 0x56df, 0xd9dc, 0x2406 },
                GX = { 0xd51a, 0x8f25, 0x2d60, 0xc956, 0xa7b2, 0x9525, 0xc760, 0x692c,
                       0xdc5c, 0xfdd6, 0xe231, 0xc0a4, 0x53fe, 0xcd6e, 0x36d3, 0x2169 },
                GY = { 0x6658, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666,
                       0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666, 0x6666 },
                SQRTM1 = { 0xa0b0, 0x4a0e, 0x1b27, 0xc4ee, 0xe478, 0xad2f, 0x1806, 0x2f43,
                           0xd7a7, 0x3dfb, 0x0099, 0x2b4d, 0xdf0b, 0x4fc1, 0x2480, 0x2b83 };

static void set25519(gf r, const gf a) { for (int i = 0; i < 16; i++) r[i] = a[i]; }
static void car25519(gf o) {
    for (int i = 0; i < 16; i++) {
        o[i] += (int64_t) 1 << 16;
        int64_t c = o[i] >> 16;
        o[(i + 1) * (i < 15)] += c - 1 + 37 * (c - 1) * (i == 15);
        o[i] -= (int64_t) ((uint64_t) c << 16);
    }
}
static void sel25519(gf p, gf q, int b) {
    int64_t c = ~(int64_t) (b - 1);
    for (int i = 0; i < 16; i++) {
        int64_t t = c & (p[i] ^ q[i]);
        p[i] ^= t;
        q[i] ^= t;
    }
}
static void pack25519(unsigned char *o, const gf n) {
    gf m, t;
    set25519(t, n);
    car25519(t);
    car25519(t);
    car25519(t);
    for (int j = 0; j < 2; j++) {
        m[0] = t[0] - 0xffed;
        for (int i = 1; i < 15; i++) {
            m[i] = t[i] - 0xffff - ((m[i - 1] >> 16) & 1);
            m[i - 1] &= 0xffff;
        }
        m[15] = t[15] - 0x7fff - ((m[14] >> 16) & 1);
        int b = (int) ((m[15] >> 16) & 1);
        m[14] &= 0xffff;
        sel25519(t, m, 1 - b);
    }
    for (int i = 0; i < 16; i++) {
        o[2 * i] = (unsigned char) (t[i] & 0xff);
        o[2 * i + 1] = (unsigned char) (t[i] >> 8);
    }
}
static int same32(const unsigned char *a, const unsigned char *b) {
    unsigned d = 0;
    for (int i = 0; i < 32; i++) d |= (unsigned) (a[i] ^ b[i]);
    return d == 0;
}
static int neq25519(const gf a, const gf b) {
    unsigned char c[32], d[32];
    pack25519(c, a);
    pack25519(d, b);
    return !same32(c, d);
}
static unsigned par25519(const gf a) {
    unsigned char d[32];
    pack25519(d, a);
    return d[0] & 1;
}
static void unpack25519(gf o, const unsigned char *n) {
    for (int i = 0; i < 16; i++) o[i] = n[2 * i] + ((int64_t) n[2 * i + 1] << 8);
    o[15] &= 0x7fff;
}
static void fadd(gf o, const gf a, const gf b) { for (int i = 0; i < 16; i++) o[i] = a[i] + b[i]; }
static void fsub(gf o, const gf a, const gf b) { for (int i = 0; i < 16; i++) o[i] = a[i] - b[i]; }
static void fmul(gf o, const gf a, const gf b) {
    int64_t t[31];
    for (int i = 0; i < 31; i++) t[i] = 0;
    for (int i = 0; i < 16; i++)
        for (int j = 0; j < 16; j++) t[i + j] += a[i] * b[j];
    for (int i = 0; i < 15; i++) t[i] += 38 * t[i + 16];
    for (int i = 0; i < 16; i++) o[i] = t[i];
    car25519(o);
    car25519(o);
}
static void fsq(gf o, const gf a) { fmul(o, a, a); }
static void inv25519(gf o, const gf i) {
    gf c;
    set25519(c, i);
    for (int a = 253; a >= 0; a--) {
        fsq(c, c);
        if (a != 2 && a != 4) fmul(c, c, i);
    }
    set25519(o, c);
}
static void pow2523(gf o, const gf i) {
    gf c;
    set25519(c, i);
    for (int a = 250; a >= 0; a--) {
        fsq(c, c);
        if (a != 1) fmul(c, c, i);
    }
    set25519(o, c);
}
static void ed_add(gf p[4], gf q[4]) {
    gf a, b, c, d, t, e, f, g, h;
    fsub(a, p[1], p[0]);
    fsub(t, q[1], q[0]);
    fmul(a, a, t);
    fadd(b, p[0], p[1]);
    fadd(t, q[0], q[1]);
    fmul(b, b, t);
    fmul(c, p[3], q[3]);
    fmul(c, c, D2);
    fmul(d, p[2], q[2]);
    fadd(d, d, d);
    fsub(e, b, a);
    fsub(f, d, c);
    fadd(g, d, c);
    fadd(h, b, a);
    fmul(p[0], e, f);
    fmul(p[1], h, g);
    fmul(p[2], g, f);
    fmul(p[3], e, h);
}
static void ed_cswap(gf p[4], gf q[4], int b) { for (int i = 0; i < 4; i++) sel25519(p[i], q[i], b); }
static void ed_pack(unsigned char *r, gf p[4]) {
    gf tx, ty, zi;
    inv25519(zi, p[2]);
    fmul(tx, p[0], zi);
    fmul(ty, p[1], zi);
    pack25519(r, ty);
    r[31] ^= (unsigned char) (par25519(tx) << 7);
}
static void ed_scalarmult(gf p[4], gf q[4], const unsigned char *s) {
    set25519(p[0], gf0);
    set25519(p[1], gf1);
    set25519(p[2], gf1);
    set25519(p[3], gf0);
    for (int i = 255; i >= 0; --i) {
        int b = (s[i / 8] >> (i & 7)) & 1;
        ed_cswap(p, q, b);
        ed_add(q, p);
        ed_add(p, p);
        ed_cswap(p, q, b);
    }
}
static void ed_scalarbase(gf p[4], const unsigned char *s) {
    gf q[4];
    set25519(q[0], GX);
    set25519(q[1], GY);
    set25519(q[2], gf1);
    fmul(q[3], GX, GY);
    ed_scalarmult(p, q, s);
}
static const int64_t ED_L[32] = { 0xed, 0xd3, 0xf5, 0x5c, 0x1a, 0x63, 0x12, 0x58, 0xd6, 0x9c, 0xf7,
                                  0xa2, 0xde, 0xf9, 0xde, 0x14, 0,    0,    0,    0,    0,    0,
                                  0,    0,    0,    0,    0,    0,    0,    0,    0,    0x10 };
static void ed_modL(unsigned char *r, int64_t x[64]) {
    int64_t carry;
    int i, j;
    for (i = 63; i >= 32; --i) {
        carry = 0;
        for (j = i - 32; j < i - 12; ++j) {
            x[j] += carry - 16 * x[i] * ED_L[j - (i - 32)];
            carry = (x[j] + 128) >> 8;
            x[j] -= (int64_t) ((uint64_t) carry << 8);
        }
        x[j] += carry;
        x[i] = 0;
    }
    carry = 0;
    for (j = 0; j < 32; j++) {
        x[j] += carry - (x[31] >> 4) * ED_L[j];
        carry = x[j] >> 8;
        x[j] &= 255;
    }
    for (j = 0; j < 32; j++) x[j] -= carry * ED_L[j];
    for (i = 0; i < 32; i++) {
        x[i + 1] += x[i] >> 8;
        r[i] = (unsigned char) (x[i] & 255);
    }
}
static void ed_reduce(unsigned char *r) {
    int64_t x[64];
    for (int i = 0; i < 64; i++) x[i] = (int64_t) r[i];
    for (int i = 0; i < 64; i++) r[i] = 0;
    ed_modL(r, x);
}
static int ed_unpackneg(gf r[4], const unsigned char p[32]) {
    gf t, chk, num, den, den2, den4, den6;
    set25519(r[2], gf1);
    unpack25519(r[1], p);
    fsq(num, r[1]);
    fmul(den, num, D);
    fsub(num, num, r[2]);
    fadd(den, r[2], den);
    fsq(den2, den);
    fsq(den4, den2);
    fmul(den6, den4, den2);
    fmul(t, den6, num);
    fmul(t, t, den);
    pow2523(t, t);
    fmul(t, t, num);
    fmul(t, t, den);
    fmul(t, t, den);
    fmul(r[0], t, den);
    fsq(chk, r[0]);
    fmul(chk, chk, den);
    if (neq25519(chk, num)) fmul(r[0], r[0], SQRTM1);
    fsq(chk, r[0]);
    fmul(chk, chk, den);
    if (neq25519(chk, num)) return -1;
    if (par25519(r[0]) == (unsigned) (p[31] >> 7)) fsub(r[0], gf0, r[0]);
    fmul(r[3], r[0], r[1]);
    return 0;
}

/* 1 when `sig` is `pk`'s Ed25519 signature of `m`.  Exported for the C runtime's own tests. */
int loft_ssh_ed25519_verify(const unsigned char pk[32], const unsigned char *m, size_t n,
                            const unsigned char sig[64]) {
    unsigned char t[32], h[64];
    gf p[4], q[4];
    /* S must be below the group order (RFC 8032 § 5.1.7): its top three bits clear rules out
     * every S >= 2^253, and the rest is the order's own comparison. */
    if (sig[63] & 0xe0) return 0;
    {
        int lt = 0, gt = 0;
        for (int i = 31; i >= 0; i--) {
            int a = sig[32 + i], b = (int) ED_L[i];
            if (!lt && !gt) {
                if (a < b) lt = 1;
                else if (a > b) gt = 1;
            }
        }
        if (!lt) return 0;
    }
    if (ed_unpackneg(q, pk)) return 0;
    br_sha512_context hc;
    br_sha512_init(&hc);
    br_sha512_update(&hc, sig, 32);
    br_sha512_update(&hc, pk, 32);
    br_sha512_update(&hc, m, n);
    br_sha512_out(&hc, h);
    ed_reduce(h);
    ed_scalarmult(p, q, h);
    ed_scalarbase(q, sig + 32);
    ed_add(p, q);
    ed_pack(t, p);
    return same32(sig, t);
}

/* ── One direction's cipher ── */

enum { CIPHER_NONE = 0, CIPHER_GCM = 1, CIPHER_CTR = 2 };

typedef struct {
    int kind;
    int etm;        /* CTR: the MAC covers the ciphertext and the length travels in clear */
    const br_hash_class *mac; /* CTR: the HMAC's hash */
    size_t maclen;
    br_aes_ct64_ctr_keys aes;
    unsigned char iv[16]; /* GCM: 4 fixed bytes and an 8-byte invocation counter; CTR: the counter block */
    unsigned char h[16];  /* GCM: the hash key */
    br_hmac_key_context mk;
} sdir;

static void incr_be(unsigned char *p, size_t n) {
    for (size_t i = n; i-- > 0;)
        if (++p[i] != 0) break;
}

/* AES-CTR over a 128-bit big-endian counter; BearSSL's runs a 32-bit one, so a carry out of it
 * is taken by hand. */
static void ctr_run(sdir *d, unsigned char *p, size_t n) {
    while (n >= 16) {
        uint32_t cc = get32(d->iv + 12);
        uint64_t room = (uint64_t) 0x100000000u - cc, blocks = n / 16;
        size_t take = (size_t) (blocks < room ? blocks : room);
        br_aes_ct64_ctr_run(&d->aes, d->iv, cc, p, take * 16);
        uint32_t next = cc + (uint32_t) take;
        put32(d->iv + 12, next);
        if (next == 0) incr_be(d->iv, 12);
        p += take * 16;
        n -= take * 16;
    }
}

/* GCM's tag over the 4-byte length (the associated data) and the ciphertext. */
static void gcm_tag(sdir *d, const unsigned char *aad, const unsigned char *c, size_t cl, unsigned char tag[16]) {
    unsigned char y[16], lens[16], ek[16];
    memset(y, 0, sizeof y);
    br_ghash_ctmul64(y, d->h, aad, 4);
    br_ghash_ctmul64(y, d->h, c, cl);
    memset(lens, 0, sizeof lens);
    lens[7] = 32; /* 4 bytes of associated data, in bits */
    uint64_t bits = (uint64_t) cl * 8;
    for (int i = 0; i < 8; i++) lens[15 - i] = (unsigned char) (bits >> (8 * i));
    br_ghash_ctmul64(y, d->h, lens, 16);
    memset(ek, 0, sizeof ek);
    br_aes_ct64_ctr_run(&d->aes, d->iv, 1, ek, 16);
    for (int i = 0; i < 16; i++) tag[i] = (unsigned char) (y[i] ^ ek[i]);
}

static void hmac_of(sdir *d, uint32_t seq, const unsigned char *p, size_t n, unsigned char *out) {
    br_hmac_context hc;
    unsigned char s[4];
    put32(s, seq);
    br_hmac_init(&hc, &d->mk, 0);
    br_hmac_update(&hc, s, 4);
    br_hmac_update(&hc, p, n);
    br_hmac_out(&hc, out);
}

static int same_n(const unsigned char *a, const unsigned char *b, size_t n) {
    unsigned d = 0;
    for (size_t i = 0; i < n; i++) d |= (unsigned) (a[i] ^ b[i]);
    return d == 0;
}

/* ── The session ── */

struct loft_ssh {
    int fd;
    int dead;   /* the transport failed or the server disconnected */
    char err[256];
    br_hmac_drbg_context rng;
    sbuf in;    /* bytes read from the socket, from `in_at` on not yet a whole packet */
    size_t in_at;
    int in_first; /* CTR without ETM: the first block of the packet at `in_at` is decrypted */
    sdir rx, tx;
    uint32_t rx_seq, tx_seq;
    int strict;  /* strict kex: the sequence numbers restart at every NEWKEYS */
    int kexed;   /* the first key exchange completed */
    sbuf v_c, v_s; /* the two version strings, without their line ends */
    unsigned char session_id[32];
    int service; /* ssh-userauth accepted */
    int authed;
    /* The shell channel. */
    int chan_open, chan_eof, close_sent;
    uint32_t remote_id, remote_window, remote_max;
    uint32_t local_window;
    sbuf inbound; /* shell output not yet handed out */
    sbuf handed;  /* the last output handed out: lives until the next read */
};

static void ssh_fail(struct loft_ssh *s, const char *why) {
    if (!s->err[0]) snprintf(s->err, sizeof s->err, "%s", why);
    s->dead = 1;
}

static int ssh_random(struct loft_ssh *s, void *p, size_t n) {
    br_hmac_drbg_generate(&s->rng, p, n);
    return 1;
}

static int ssh_write_all(struct loft_ssh *s, const unsigned char *p, size_t n) {
    while (n) {
        long w = ssh_sock_send(s->fd, p, n);
        if (w < 0 && ssh_sock_interrupted()) continue;
        if (w <= 0) { ssh_fail(s, "the connection closed while sending"); return 0; }
        p += w;
        n -= (size_t) w;
    }
    return 1;
}

/* Read what the socket has within `ms` (0: only what is already there): 1 on bytes, 0 on
 * none, -1 when the connection closed. */
static int ssh_fill(struct loft_ssh *s, int ms) {
    int w = ssh_sock_wait(s->fd, ms < 0 ? 0 : ms);
    if (w < 0) {
        if (ssh_sock_interrupted()) return 0;
        ssh_fail(s, "waiting on the connection failed");
        return -1;
    }
    if (w == 0) return 0;
    if (s->in_at > 0 && s->in_at == s->in.len) {
        s->in.len = 0;
        s->in_at = 0;
    } else if (s->in_at > 65536) {
        memmove(s->in.p, s->in.p + s->in_at, s->in.len - s->in_at);
        s->in.len -= s->in_at;
        s->in_at = 0;
    }
    if (!sb_reserve(&s->in, 16384)) { ssh_fail(s, "out of memory"); return -1; }
    long r = ssh_sock_recv(s->fd, s->in.p + s->in.len, 16384);
    if (r < 0 && ssh_sock_interrupted()) return 0;
    if (r <= 0) { ssh_fail(s, "the server closed the connection"); return -1; }
    s->in.len += (size_t) r;
    return 1;
}

/* ── Packets ── */

static int ssh_send(struct loft_ssh *s, const sbuf *payload) {
    sdir *d = &s->tx;
    size_t bs = d->kind ? 16 : 8, n = payload->len;
    int clear_len = d->kind == CIPHER_GCM || (d->kind == CIPHER_CTR && d->etm);
    size_t base = 1 + n + (clear_len ? 0 : 4);
    size_t pad = bs - base % bs;
    if (pad < 4) pad += bs;
    size_t plen = 1 + n + pad, tail = d->kind == CIPHER_GCM ? 16 : d->maclen;
    unsigned char *pk = (unsigned char *) malloc(4 + plen + tail);
    if (!pk) { ssh_fail(s, "out of memory"); return 0; }
    put32(pk, (uint32_t) plen);
    pk[4] = (unsigned char) pad;
    if (n) memcpy(pk + 5, payload->p, n);
    ssh_random(s, pk + 5 + n, pad);
    if (d->kind == CIPHER_GCM) {
        br_aes_ct64_ctr_run(&d->aes, d->iv, 2, pk + 4, plen);
        gcm_tag(d, pk, pk + 4, plen, pk + 4 + plen);
        incr_be(d->iv + 4, 8);
    } else if (d->kind == CIPHER_CTR && d->etm) {
        ctr_run(d, pk + 4, plen);
        hmac_of(d, s->tx_seq, pk, 4 + plen, pk + 4 + plen);
    } else if (d->kind == CIPHER_CTR) {
        hmac_of(d, s->tx_seq, pk, 4 + plen, pk + 4 + plen);
        ctr_run(d, pk, 4 + plen);
    }
    int ok = ssh_write_all(s, pk, 4 + plen + tail);
    free(pk);
    s->tx_seq++;
    return ok;
}

/* The next whole packet already read: 1 with its payload (valid until the next read), 0 when
 * more bytes are needed, -1 on a corrupt or forged packet. */
static int ssh_take(struct loft_ssh *s, const unsigned char **pl, size_t *pn) {
    sdir *d = &s->rx;
    unsigned char *b = s->in.p + s->in_at;
    size_t have = s->in.len - s->in_at, tail = d->kind == CIPHER_GCM ? 16 : d->maclen;
    uint32_t plen;
    if (d->kind == CIPHER_CTR && !d->etm) {
        if (!s->in_first) {
            if (have < 16) return 0;
            ctr_run(d, b, 16);
            s->in_first = 1;
        }
        plen = get32(b);
        if (plen < 12 || plen > SSH_MAX_WIRE || (plen + 4) % 16) { ssh_fail(s, "a corrupt packet length"); return -1; }
        if (have < 4 + plen + tail) return 0;
        ctr_run(d, b + 16, 4 + plen - 16);
        unsigned char mac[64];
        hmac_of(d, s->rx_seq, b, 4 + plen, mac);
        if (!same_n(mac, b + 4 + plen, tail)) { ssh_fail(s, "a packet failed its MAC"); return -1; }
        s->in_first = 0;
    } else {
        if (have < 4) return 0;
        plen = get32(b);
        size_t align = d->kind ? 16 : 8;
        int clear_len = d->kind != CIPHER_NONE;
        if (plen < 5 || plen > SSH_MAX_WIRE || (clear_len ? plen : plen + 4) % align) {
            ssh_fail(s, "a corrupt packet length");
            return -1;
        }
        if (have < 4 + plen + tail) return 0;
        if (d->kind == CIPHER_GCM) {
            unsigned char tag[16];
            gcm_tag(d, b, b + 4, plen, tag);
            if (!same_n(tag, b + 4 + plen, 16)) { ssh_fail(s, "a packet failed its tag"); return -1; }
            br_aes_ct64_ctr_run(&d->aes, d->iv, 2, b + 4, plen);
            incr_be(d->iv + 4, 8);
        } else if (d->kind == CIPHER_CTR) {
            unsigned char mac[64];
            hmac_of(d, s->rx_seq, b, 4 + plen, mac);
            if (!same_n(mac, b + 4 + plen, tail)) { ssh_fail(s, "a packet failed its MAC"); return -1; }
            ctr_run(d, b + 4, plen);
        }
    }
    unsigned padn = b[4];
    if (padn < 4 || (size_t) padn + 1 > plen) { ssh_fail(s, "a corrupt packet padding"); return -1; }
    *pl = b + 5;
    *pn = plen - 1 - padn;
    s->in_at += 4 + plen + tail;
    s->rx_seq++;
    if (*pn == 0) { ssh_fail(s, "an empty packet"); return -1; }
    return 1;
}

/* The next packet within `deadline` (an `ssh_now_ms` time): 1, or 0 on a timeout or failure. */
static int ssh_next(struct loft_ssh *s, int64_t deadline, const unsigned char **pl, size_t *pn) {
    for (;;) {
        if (s->dead) return 0;
        int t = ssh_take(s, pl, pn);
        if (t < 0) return 0;
        if (t > 0) return 1;
        int64_t left = deadline - ssh_now_ms();
        if (left <= 0) { ssh_fail(s, "the server did not answer in time"); return 0; }
        if (ssh_fill(s, (int) (left > 1000 ? 1000 : left)) < 0) return 0;
    }
}

/* ── Key exchange ── */

static const char *ssh_pick(const char *ours, const unsigned char *theirs, size_t tn, char *out, size_t cap) {
    const char *p = ours;
    while (*p) {
        const char *e = strchr(p, ',');
        size_t l = e ? (size_t) (e - p) : strlen(p);
        size_t i = 0;
        while (i < tn) {
            size_t j = i;
            while (j < tn && theirs[j] != ',') j++;
            if (j - i == l && memcmp(theirs + i, p, l) == 0 && l < cap) {
                memcpy(out, p, l);
                out[l] = 0;
                return out;
            }
            i = j + 1;
        }
        if (!e) break;
        p = e + 1;
    }
    return 0;
}
static int ssh_lists(const unsigned char *list, size_t n, const char *name) {
    char tmp[64];
    return ssh_pick(name, list, n, tmp, sizeof tmp) != 0;
}

typedef struct {
    char kex[64], hostkey[64], enc_cs[64], enc_sc[64], mac_cs[64], mac_sc[64];
} ssh_algs;

/* The RSA / ECDSA / Ed25519 signature over `h` by the host key `ks`, as `alg` names it. */
static int ssh_host_signed(const char *alg, const unsigned char *ks, size_t kn, const unsigned char *sg, size_t sn,
                           const unsigned char *h, size_t hn) {
    rd k = rd_of(ks, kn), g = rd_of(sg, sn);
    const unsigned char *kt, *st, *sv;
    size_t ktn, stn, svn;
    rd_str(&k, &kt, &ktn);
    rd_str(&g, &st, &stn);
    rd_str(&g, &sv, &svn);
    if (k.bad || g.bad) return 0;
    if (strcmp(alg, "ssh-ed25519") == 0) {
        const unsigned char *pk;
        size_t pkn;
        rd_str(&k, &pk, &pkn);
        if (k.bad || !str_is(kt, ktn, alg) || !str_is(st, stn, alg) || pkn != 32 || svn != 64) return 0;
        return loft_ssh_ed25519_verify(pk, h, hn, sv);
    }
    if (strcmp(alg, "ecdsa-sha2-nistp256") == 0) {
        const unsigned char *curve, *q, *r, *sv2;
        size_t cn, qn, rn, sn2;
        rd_str(&k, &curve, &cn);
        rd_str(&k, &q, &qn);
        if (k.bad || !str_is(kt, ktn, alg) || !str_is(curve, cn, "nistp256") || !str_is(st, stn, alg) || qn != 65)
            return 0;
        rd sig = rd_of(sv, svn);
        rd_str(&sig, &r, &rn);
        rd_str(&sig, &sv2, &sn2);
        if (sig.bad) return 0;
        unsigned char raw[64];
        memset(raw, 0, sizeof raw);
        while (rn && *r == 0) { r++; rn--; }
        while (sn2 && *sv2 == 0) { sv2++; sn2--; }
        if (rn > 32 || sn2 > 32) return 0;
        memcpy(raw + 32 - rn, r, rn);
        memcpy(raw + 64 - sn2, sv2, sn2);
        unsigned char hh[32];
        br_sha256_context hc;
        br_sha256_init(&hc);
        br_sha256_update(&hc, h, hn);
        br_sha256_out(&hc, hh);
        br_ec_public_key pub;
        pub.curve = BR_EC_secp256r1;
        pub.q = (unsigned char *) q;
        pub.qlen = qn;
        return br_ecdsa_i31_vrfy_raw(&br_ec_p256_m31, hh, 32, &pub, raw, 64) == 1;
    }
    if (strcmp(alg, "rsa-sha2-256") == 0 || strcmp(alg, "rsa-sha2-512") == 0) {
        int big = strcmp(alg, "rsa-sha2-512") == 0;
        const unsigned char *e, *nn;
        size_t en, nnn;
        rd_str(&k, &e, &en);
        rd_str(&k, &nn, &nnn);
        if (k.bad || !str_is(kt, ktn, "ssh-rsa") || !str_is(st, stn, alg)) return 0;
        while (en && *e == 0) { e++; en--; }
        while (nnn && *nn == 0) { nn++; nnn--; }
        if (nnn < 128 || nnn > 1024 || svn > nnn || en == 0) return 0;
        unsigned char *sig = (unsigned char *) calloc(1, nnn);
        if (!sig) return 0;
        memcpy(sig + nnn - svn, sv, svn);
        unsigned char hh[64], want[64];
        size_t hl = big ? 64 : 32;
        if (big) {
            br_sha512_context hc;
            br_sha512_init(&hc);
            br_sha512_update(&hc, h, hn);
            br_sha512_out(&hc, want);
        } else {
            br_sha256_context hc;
            br_sha256_init(&hc);
            br_sha256_update(&hc, h, hn);
            br_sha256_out(&hc, want);
        }
        br_rsa_public_key pub;
        pub.n = (unsigned char *) nn;
        pub.nlen = nnn;
        pub.e = (unsigned char *) e;
        pub.elen = en;
        uint32_t ok = br_rsa_i31_pkcs1_vrfy(sig, nnn, big ? BR_HASH_OID_SHA512 : BR_HASH_OID_SHA256, hl, &pub, hh);
        free(sig);
        return ok == 1 && same_n(hh, want, hl);
    }
    return 0;
}

/* Key material `letter` (RFC 4253 § 7.2), `need` bytes of it. */
static void ssh_derive(struct loft_ssh *s, const sbuf *kenc, const unsigned char h[32], char letter, unsigned char *out,
                       size_t need) {
    unsigned char acc[128];
    size_t have = 0;
    br_sha256_context hc;
    br_sha256_init(&hc);
    br_sha256_update(&hc, kenc->p, kenc->len);
    br_sha256_update(&hc, h, 32);
    br_sha256_update(&hc, &letter, 1);
    br_sha256_update(&hc, s->session_id, 32);
    br_sha256_out(&hc, acc);
    have = 32;
    while (have < need && have + 32 <= sizeof acc) {
        br_sha256_init(&hc);
        br_sha256_update(&hc, kenc->p, kenc->len);
        br_sha256_update(&hc, h, 32);
        br_sha256_update(&hc, acc, have);
        br_sha256_out(&hc, acc + have);
        have += 32;
    }
    memcpy(out, acc, need);
}

/* Set up one direction from its negotiated names and key material. */
static int ssh_keys(struct loft_ssh *s, sdir *d, const char *enc, const char *mac, const sbuf *kenc,
                    const unsigned char h[32], char iv_l, char key_l, char mac_l) {
    memset(d, 0, sizeof *d);
    size_t klen = strstr(enc, "aes256") ? 32 : strstr(enc, "aes192") ? 24 : 16;
    unsigned char key[32], iv[16];
    if (strstr(enc, "gcm")) {
        d->kind = CIPHER_GCM;
        ssh_derive(s, kenc, h, iv_l, iv, 12);
        ssh_derive(s, kenc, h, key_l, key, klen);
        br_aes_ct64_ctr_init(&d->aes, key, klen);
        memcpy(d->iv, iv, 12);
        unsigned char zero_iv[12];
        memset(zero_iv, 0, sizeof zero_iv);
        memset(d->h, 0, 16);
        br_aes_ct64_ctr_run(&d->aes, zero_iv, 0, d->h, 16);
        return 1;
    }
    d->kind = CIPHER_CTR;
    ssh_derive(s, kenc, h, iv_l, iv, 16);
    ssh_derive(s, kenc, h, key_l, key, klen);
    br_aes_ct64_ctr_init(&d->aes, key, klen);
    memcpy(d->iv, iv, 16);
    d->etm = strstr(mac, "-etm@") != 0;
    int big = strstr(mac, "sha2-512") != 0;
    d->mac = big ? &br_sha512_vtable : &br_sha256_vtable;
    d->maclen = big ? 64 : 32;
    unsigned char mk[64];
    ssh_derive(s, kenc, h, mac_l, mk, d->maclen);
    br_hmac_key_init(&d->mk, d->mac, mk, d->maclen);
    return 1;
}

static int ssh_kexinit_payload(struct loft_ssh *s, sbuf *b) {
    unsigned char cookie[16];
    ssh_random(s, cookie, 16);
    return sb_u8(b, MSG_KEXINIT) && sb_put(b, cookie, 16) && sb_cstr(b, SSH_KEX) && sb_cstr(b, SSH_HOSTKEYS) &&
           sb_cstr(b, SSH_CIPHERS) && sb_cstr(b, SSH_CIPHERS) && sb_cstr(b, SSH_MACS) && sb_cstr(b, SSH_MACS) &&
           sb_cstr(b, "none") && sb_cstr(b, "none") && sb_cstr(b, "") && sb_cstr(b, "") && sb_u8(b, 0) &&
           sb_u32(b, 0);
}

/* During the first exchange under strict kex, a packet other than the exchange's own is an
 * attack, and outside it the transport's chatter is skipped. */
static int ssh_next_kex(struct loft_ssh *s, int64_t deadline, const unsigned char **pl, size_t *pn) {
    for (;;) {
        if (!ssh_next(s, deadline, pl, pn)) return 0;
        unsigned t = (*pl)[0];
        if (t == MSG_DISCONNECT) { ssh_fail(s, "the server disconnected during the key exchange"); return 0; }
        if (t == MSG_IGNORE || t == MSG_DEBUG || t == MSG_UNIMPLEMENTED) {
            if (s->strict && !s->kexed) { ssh_fail(s, "an unexpected message during a strict key exchange"); return 0; }
            continue;
        }
        return 1;
    }
}

/* One key exchange: `theirs` is the server's KEXINIT when the server started it (a rekey),
 * else this sends its own first and reads the server's. */
static int ssh_kex(struct loft_ssh *s, const unsigned char *theirs, size_t tn) {
    int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
    sbuf ic = { 0, 0, 0 }, is = { 0, 0, 0 }, pkt = { 0, 0, 0 }, hb = { 0, 0, 0 }, kenc = { 0, 0, 0 };
    int ok = 0;
    if (!ssh_kexinit_payload(s, &ic) || !ssh_send(s, &ic)) goto done;
    if (theirs) {
        sb_put(&is, theirs, tn);
    } else {
        const unsigned char *pl;
        size_t pn;
        if (!ssh_next_kex(s, deadline, &pl, &pn)) goto done;
        if (pl[0] != MSG_KEXINIT) { ssh_fail(s, "the server did not open with its algorithms"); goto done; }
        sb_put(&is, pl, pn);
    }
    rd r = rd_of(is.p, is.len);
    const unsigned char *l[10];
    size_t ln[10];
    rd_skip(&r, 17);
    for (int i = 0; i < 10; i++) rd_str(&r, &l[i], &ln[i]);
    unsigned follows = rd_u8(&r);
    if (r.bad) { ssh_fail(s, "a corrupt algorithm list"); goto done; }
    ssh_algs a;
    if (!ssh_pick("curve25519-sha256,curve25519-sha256@libssh.org", l[0], ln[0], a.kex, sizeof a.kex) ||
        !ssh_pick(SSH_HOSTKEYS, l[1], ln[1], a.hostkey, sizeof a.hostkey) ||
        !ssh_pick(SSH_CIPHERS, l[2], ln[2], a.enc_cs, sizeof a.enc_cs) ||
        !ssh_pick(SSH_CIPHERS, l[3], ln[3], a.enc_sc, sizeof a.enc_sc)) {
        ssh_fail(s, "no key exchange, host key or cipher in common with the server");
        goto done;
    }
    a.mac_cs[0] = a.mac_sc[0] = 0;
    if (!strstr(a.enc_cs, "gcm") && !ssh_pick(SSH_MACS, l[4], ln[4], a.mac_cs, sizeof a.mac_cs)) {
        ssh_fail(s, "no MAC in common with the server");
        goto done;
    }
    if (!strstr(a.enc_sc, "gcm") && !ssh_pick(SSH_MACS, l[5], ln[5], a.mac_sc, sizeof a.mac_sc)) {
        ssh_fail(s, "no MAC in common with the server");
        goto done;
    }
    if (!ssh_lists(l[6], ln[6], "none") || !ssh_lists(l[7], ln[7], "none")) {
        ssh_fail(s, "the server insists on compression");
        goto done;
    }
    if (!s->kexed) s->strict = ssh_lists(l[0], ln[0], "kex-strict-s-v00@openssh.com");
    if (!s->kexed && s->strict && theirs) { ssh_fail(s, "a strict key exchange must start the session"); goto done; }
    /* The server guessed a first exchange packet; when its guess is not what was agreed, that
     * packet is dropped (RFC 4253 § 7). */
    int drop_guess = 0;
    if (follows) {
        char first[64];
        size_t k = 0;
        while (k < ln[0] && l[0][k] != ',' && k + 1 < sizeof first) { first[k] = (char) l[0][k]; k++; }
        first[k] = 0;
        size_t h2 = 0;
        char firsth[64];
        while (h2 < ln[1] && l[1][h2] != ',' && h2 + 1 < sizeof firsth) { firsth[h2] = (char) l[1][h2]; h2++; }
        firsth[h2] = 0;
        drop_guess = strcmp(first, a.kex) != 0 || strcmp(firsth, a.hostkey) != 0;
    }
    /* Curve25519: an ephemeral scalar, its public point, the ECDH_INIT. */
    unsigned char priv[32], qc[32], qs[32];
    ssh_random(s, priv, 32);
    if (br_ec_c25519_m31.mulgen(qc, priv, 32, BR_EC_curve25519) != 32) { ssh_fail(s, "the key pair failed"); goto done; }
    pkt.len = 0;
    if (!sb_u8(&pkt, MSG_KEX_ECDH_INIT) || !sb_str(&pkt, qc, 32) || !ssh_send(s, &pkt)) goto done;
    const unsigned char *pl;
    size_t pn;
    for (;;) {
        if (!ssh_next_kex(s, deadline, &pl, &pn)) goto done;
        if (drop_guess) { drop_guess = 0; continue; }
        break;
    }
    if (pl[0] != MSG_KEX_ECDH_REPLY) { ssh_fail(s, "the server did not answer the key exchange"); goto done; }
    rd rr = rd_of(pl + 1, pn - 1);
    const unsigned char *ks, *qsp, *sg;
    size_t ksn, qsn, sgn;
    rd_str(&rr, &ks, &ksn);
    rd_str(&rr, &qsp, &qsn);
    rd_str(&rr, &sg, &sgn);
    if (rr.bad || qsn != 32) { ssh_fail(s, "a corrupt key exchange reply"); goto done; }
    memcpy(qs, qsp, 32);
    unsigned char shared[32];
    memcpy(shared, qs, 32);
    if (!br_ec_c25519_m31.mul(shared, 32, priv, 32, BR_EC_curve25519)) { ssh_fail(s, "a bad server point"); goto done; }
    {
        unsigned z = 0;
        for (int i = 0; i < 32; i++) z |= shared[i];
        if (!z) { ssh_fail(s, "a degenerate shared secret"); goto done; }
    }
    /* H = SHA-256(V_C, V_S, I_C, I_S, K_S, Q_C, Q_S, K). */
    if (!sb_mpint(&kenc, shared, 32)) goto done;
    sb_str(&hb, s->v_c.p, s->v_c.len);
    sb_str(&hb, s->v_s.p, s->v_s.len);
    sb_str(&hb, ic.p, ic.len);
    sb_str(&hb, is.p, is.len);
    sb_str(&hb, ks, ksn);
    sb_str(&hb, qc, 32);
    sb_str(&hb, qs, 32);
    sb_put(&hb, kenc.p, kenc.len);
    unsigned char h[32];
    br_sha256_context hc;
    br_sha256_init(&hc);
    br_sha256_update(&hc, hb.p, hb.len);
    br_sha256_out(&hc, h);
    if (!ssh_host_signed(a.hostkey, ks, ksn, sg, sgn, h, 32)) {
        ssh_fail(s, "the server's host key signature did not verify");
        goto done;
    }
    if (!s->kexed) memcpy(s->session_id, h, 32);
    sdir ntx, nrx;
    ssh_keys(s, &ntx, a.enc_cs, a.mac_cs, &kenc, h, 'A', 'C', 'E');
    ssh_keys(s, &nrx, a.enc_sc, a.mac_sc, &kenc, h, 'B', 'D', 'F');
    pkt.len = 0;
    if (!sb_u8(&pkt, MSG_NEWKEYS) || !ssh_send(s, &pkt)) goto done;
    s->tx = ntx;
    if (s->strict) s->tx_seq = 0;
    if (!ssh_next_kex(s, deadline, &pl, &pn)) goto done;
    if (pl[0] != MSG_NEWKEYS) { ssh_fail(s, "the server did not switch keys"); goto done; }
    s->rx = nrx;
    s->in_first = 0;
    if (s->strict) s->rx_seq = 0;
    s->kexed = 1;
    ok = 1;
done:
    memset(priv, 0, sizeof priv);
    free(ic.p);
    free(is.p);
    free(pkt.p);
    free(hb.p);
    free(kenc.p);
    return ok;
}

/* ── The messages the session answers by itself ── */

static void ssh_window_adjust(struct loft_ssh *s) {
    if (!s->chan_open || s->close_sent || s->local_window >= SSH_WINDOW / 2 || s->inbound.len > SSH_WINDOW / 2) return;
    uint32_t add = SSH_WINDOW - s->local_window;
    sbuf p = { 0, 0, 0 };
    if (sb_u8(&p, MSG_CHANNEL_WINDOW_ADJUST) && sb_u32(&p, s->remote_id) && sb_u32(&p, add) && ssh_send(s, &p))
        s->local_window += add;
    free(p.p);
}

/* 1 when the message was the session's own business (consumed), 0 when the caller waits for
 * it.  -1 on a fatal one. */
static int ssh_dispatch(struct loft_ssh *s, const unsigned char *pl, size_t pn) {
    rd r = rd_of(pl + 1, pn - 1);
    switch (pl[0]) {
    case MSG_IGNORE:
    case MSG_DEBUG:
    case MSG_UNIMPLEMENTED:
    case MSG_EXT_INFO:
        return 1;
    case MSG_DISCONNECT:
        ssh_fail(s, "the server disconnected");
        s->chan_open = 0;
        return -1;
    case MSG_KEXINIT: {
        sbuf copy = { 0, 0, 0 };
        int ok = sb_put(&copy, pl, pn) && ssh_kex(s, copy.p, copy.len);
        free(copy.p);
        return ok ? 1 : -1;
    }
    case MSG_GLOBAL_REQUEST: {
        const unsigned char *nm;
        size_t nn;
        rd_str(&r, &nm, &nn);
        unsigned want = rd_u8(&r);
        if (!r.bad && want) {
            sbuf p = { 0, 0, 0 };
            if (sb_u8(&p, MSG_REQUEST_FAILURE)) ssh_send(s, &p);
            free(p.p);
        }
        return 1;
    }
    case MSG_CHANNEL_WINDOW_ADJUST: {
        rd_u32(&r);
        uint32_t add = rd_u32(&r);
        if (!r.bad) s->remote_window = (uint32_t) (s->remote_window + add < s->remote_window ? 0xffffffffu : s->remote_window + add);
        return 1;
    }
    case MSG_CHANNEL_DATA:
    case MSG_CHANNEL_EXTENDED_DATA: {
        const unsigned char *d;
        size_t dn;
        rd_u32(&r);
        if (pl[0] == MSG_CHANNEL_EXTENDED_DATA) rd_u32(&r);
        rd_str(&r, &d, &dn);
        if (r.bad) return 1;
        if (dn > s->local_window) { ssh_fail(s, "the server overran the window"); return -1; }
        s->local_window -= (uint32_t) dn;
        if (!sb_put(&s->inbound, d, dn)) { ssh_fail(s, "out of memory"); return -1; }
        return 1;
    }
    case MSG_CHANNEL_EOF:
        s->chan_eof = 1;
        return 1;
    case MSG_CHANNEL_CLOSE:
        if (s->chan_open && !s->close_sent) {
            sbuf p = { 0, 0, 0 };
            if (sb_u8(&p, MSG_CHANNEL_CLOSE) && sb_u32(&p, s->remote_id)) ssh_send(s, &p);
            free(p.p);
            s->close_sent = 1;
        }
        s->chan_open = 0;
        return 1;
    case MSG_CHANNEL_REQUEST: {
        /* exit-status, exit-signal, keepalive@openssh.com: answered "no" when a reply is
         * wanted, otherwise noted and dropped. */
        const unsigned char *nm;
        size_t nn;
        rd_u32(&r);
        rd_str(&r, &nm, &nn);
        unsigned want = rd_u8(&r);
        if (!r.bad && want && s->chan_open) {
            sbuf p = { 0, 0, 0 };
            if (sb_u8(&p, MSG_CHANNEL_FAILURE) && sb_u32(&p, s->remote_id)) ssh_send(s, &p);
            free(p.p);
        }
        return 1;
    }
    default:
        return 0;
    }
}

/* The next message the caller waits for, the session's own answered on the way. */
static int ssh_wait(struct loft_ssh *s, int64_t deadline, const unsigned char **pl, size_t *pn) {
    for (;;) {
        if (!ssh_next(s, deadline, pl, pn)) return 0;
        int d = ssh_dispatch(s, *pl, *pn);
        if (d < 0) return 0;
        if (d == 0) return 1;
    }
}

/* Everything already readable, answered, without waiting. */
static void ssh_pump(struct loft_ssh *s) {
    for (;;) {
        const unsigned char *pl;
        size_t pn;
        int t = ssh_take(s, &pl, &pn);
        if (t < 0) return;
        if (t > 0) {
            if (ssh_dispatch(s, pl, pn) < 0) return;
            continue;
        }
        if (s->dead || ssh_fill(s, 0) <= 0) return;
    }
}

/* ── The API ── */

const char *loft_ssh_error(const struct loft_ssh *s) { return s && s->err[0] ? s->err : ""; }

struct loft_ssh *loft_ssh_open(int fd, char *err, size_t cap) {
    struct loft_ssh *s = (struct loft_ssh *) calloc(1, sizeof *s);
    if (!s) {
        snprintf(err, cap, "out of memory");
        return 0;
    }
    s->fd = fd;
    br_prng_seeder seed = br_prng_seeder_system(0);
    br_hmac_drbg_init(&s->rng, &br_sha256_vtable, 0, 0);
    if (!seed || !seed(&s->rng.vtable)) {
        snprintf(err, cap, "no system entropy to seed the session's keys");
        free(s);
        return 0;
    }
    sb_put(&s->v_c, SSH_VERSION, strlen(SSH_VERSION));
    if (!ssh_write_all(s, (const unsigned char *) SSH_VERSION "\r\n", strlen(SSH_VERSION) + 2)) goto fail;
    /* The server's version line, after any lines it sends before it (RFC 4253 § 4.2). */
    int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
    for (int lines = 0;;) {
        unsigned char *nl = s->in.len > s->in_at ? (unsigned char *) memchr(s->in.p + s->in_at, '\n', s->in.len - s->in_at) : 0;
        if (!nl) {
            if (s->in.len - s->in_at > 8192) { ssh_fail(s, "the server sent no SSH version line"); goto fail; }
            int64_t left = deadline - ssh_now_ms();
            if (left <= 0) { ssh_fail(s, "the server sent no SSH version line in time"); goto fail; }
            if (ssh_fill(s, (int) (left > 1000 ? 1000 : left)) < 0) goto fail;
            continue;
        }
        unsigned char *line = s->in.p + s->in_at;
        size_t ll = (size_t) (nl - line);
        s->in_at += ll + 1;
        if (ll && line[ll - 1] == '\r') ll--;
        if (ll >= 4 && memcmp(line, "SSH-", 4) == 0) {
            if (!(ll >= 8 && (memcmp(line, "SSH-2.0-", 8) == 0 || memcmp(line, "SSH-1.99-", 9) == 0))) {
                ssh_fail(s, "the server does not speak SSH-2");
                goto fail;
            }
            sb_put(&s->v_s, line, ll);
            break;
        }
        if (++lines > 50) { ssh_fail(s, "the server sent no SSH version line"); goto fail; }
    }
    if (!ssh_kex(s, 0, 0)) goto fail;
    return s;
fail:
    snprintf(err, cap, "%s", s->err[0] ? s->err : "the SSH handshake failed");
    free(s->in.p);
    free(s->v_c.p);
    free(s->v_s.p);
    free(s);
    return 0;
}

int loft_ssh_login(struct loft_ssh *s, const char *user, size_t ul, const char *pw, size_t pl) {
    if (!s || s->dead || s->authed) return s && s->authed && !s->dead;
    int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
    sbuf p = { 0, 0, 0 };
    const unsigned char *m;
    size_t mn;
    int ok = 0;
    if (!s->service) {
        if (!sb_u8(&p, MSG_SERVICE_REQUEST) || !sb_cstr(&p, "ssh-userauth") || !ssh_send(s, &p)) goto done;
        if (!ssh_wait(s, deadline, &m, &mn)) goto done;
        if (m[0] != MSG_SERVICE_ACCEPT) { ssh_fail(s, "the server refused authentication"); goto done; }
        s->service = 1;
    }
    p.len = 0;
    if (!sb_u8(&p, MSG_USERAUTH_REQUEST) || !sb_str(&p, user, ul) || !sb_cstr(&p, "ssh-connection") ||
        !sb_cstr(&p, "password") || !sb_u8(&p, 0) || !sb_str(&p, pw, pl) || !ssh_send(s, &p))
        goto done;
    for (;;) {
        if (!ssh_wait(s, deadline, &m, &mn)) goto done;
        if (m[0] == MSG_USERAUTH_BANNER) continue;
        if (m[0] == MSG_USERAUTH_SUCCESS) {
            s->authed = 1;
            ok = 1;
        }
        break; /* a failure, or a request to change the password: not logged in */
    }
done:
    if (p.p) memset(p.p, 0, p.cap);
    free(p.p);
    return ok;
}

/* The reply to a channel request with want-reply set, the channel's traffic answered on the
 * way. */
static int ssh_channel_reply(struct loft_ssh *s, int64_t deadline) {
    const unsigned char *m;
    size_t mn;
    for (;;) {
        if (!ssh_wait(s, deadline, &m, &mn)) return 0;
        if (m[0] == MSG_CHANNEL_SUCCESS) return 1;
        if (m[0] == MSG_CHANNEL_FAILURE) return 0;
    }
}

int loft_ssh_shell(struct loft_ssh *s, const char *term, size_t tl, uint32_t cols, uint32_t rows) {
    if (!s || s->dead || !s->authed || s->chan_open) return 0;
    int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
    sbuf p = { 0, 0, 0 };
    const unsigned char *m;
    size_t mn;
    int ok = 0;
    s->local_window = SSH_WINDOW;
    if (!sb_u8(&p, MSG_CHANNEL_OPEN) || !sb_cstr(&p, "session") || !sb_u32(&p, 0) || !sb_u32(&p, SSH_WINDOW) ||
        !sb_u32(&p, SSH_MAX_PACKET) || !ssh_send(s, &p))
        goto done;
    for (;;) {
        if (!ssh_wait(s, deadline, &m, &mn)) goto done;
        if (m[0] == MSG_CHANNEL_OPEN_FAILURE) goto done;
        if (m[0] == MSG_CHANNEL_OPEN_CONFIRMATION) break;
    }
    rd r = rd_of(m + 1, mn - 1);
    rd_u32(&r);
    s->remote_id = rd_u32(&r);
    s->remote_window = rd_u32(&r);
    s->remote_max = rd_u32(&r);
    if (r.bad) { ssh_fail(s, "a corrupt channel confirmation"); goto done; }
    if (s->remote_max == 0 || s->remote_max > SSH_MAX_PACKET) s->remote_max = s->remote_max ? SSH_MAX_PACKET : 1024;
    s->chan_open = 1;
    /* A pty with no terminal modes beyond the end marker, then the login shell. */
    p.len = 0;
    if (!sb_u8(&p, MSG_CHANNEL_REQUEST) || !sb_u32(&p, s->remote_id) || !sb_cstr(&p, "pty-req") || !sb_u8(&p, 1) ||
        !sb_str(&p, term, tl) || !sb_u32(&p, cols) || !sb_u32(&p, rows) || !sb_u32(&p, 0) || !sb_u32(&p, 0) ||
        !sb_str(&p, "\0", 1) || !ssh_send(s, &p))
        goto done;
    if (!ssh_channel_reply(s, deadline)) goto done;
    p.len = 0;
    if (!sb_u8(&p, MSG_CHANNEL_REQUEST) || !sb_u32(&p, s->remote_id) || !sb_cstr(&p, "shell") || !sb_u8(&p, 1) ||
        !ssh_send(s, &p))
        goto done;
    if (!ssh_channel_reply(s, deadline)) goto done;
    ok = 1;
done:
    free(p.p);
    if (!ok) s->chan_open = 0;
    return ok;
}

int loft_ssh_write(struct loft_ssh *s, const void *data, size_t n) {
    const unsigned char *d = (const unsigned char *) data;
    if (!s || s->dead || !s->chan_open || s->chan_eof) return 0;
    sbuf p = { 0, 0, 0 };
    int ok = 1;
    while (n && ok) {
        ssh_pump(s);
        if (s->dead || !s->chan_open) { ok = 0; break; }
        if (s->remote_window == 0) {
            /* The server's window is full: wait for it to open, answering the rest. */
            int64_t deadline = ssh_now_ms() + SSH_STEP_MS;
            while (!s->dead && s->chan_open && s->remote_window == 0) {
                int64_t left = deadline - ssh_now_ms();
                if (left <= 0) { ok = 0; break; }
                if (ssh_fill(s, (int) (left > 1000 ? 1000 : left)) < 0) { ok = 0; break; }
                ssh_pump(s);
            }
            continue;
        }
        size_t chunk = n;
        if (chunk > s->remote_window) chunk = s->remote_window;
        if (chunk > s->remote_max) chunk = s->remote_max;
        p.len = 0;
        ok = sb_u8(&p, MSG_CHANNEL_DATA) && sb_u32(&p, s->remote_id) && sb_str(&p, d, chunk) && ssh_send(s, &p);
        s->remote_window -= (uint32_t) chunk;
        d += chunk;
        n -= chunk;
    }
    free(p.p);
    return ok;
}

size_t loft_ssh_read(struct loft_ssh *s, const unsigned char **out) {
    static const unsigned char none = 0;
    *out = &none;
    if (!s) return 0;
    if (!s->dead) ssh_pump(s);
    sbuf t = s->handed;
    s->handed = s->inbound;
    s->inbound = t;
    s->inbound.len = 0;
    if (!s->dead) ssh_window_adjust(s);
    if (s->handed.len) *out = s->handed.p;
    return s->handed.len;
}

void loft_ssh_resize(struct loft_ssh *s, uint32_t cols, uint32_t rows) {
    if (!s || s->dead || !s->chan_open) return;
    sbuf p = { 0, 0, 0 };
    if (sb_u8(&p, MSG_CHANNEL_REQUEST) && sb_u32(&p, s->remote_id) && sb_cstr(&p, "window-change") && sb_u8(&p, 0) &&
        sb_u32(&p, cols) && sb_u32(&p, rows) && sb_u32(&p, 0) && sb_u32(&p, 0))
        ssh_send(s, &p);
    free(p.p);
}

int loft_ssh_is_open(struct loft_ssh *s) {
    if (!s) return 0;
    if (!s->dead) ssh_pump(s);
    return !s->dead && s->chan_open && !s->chan_eof;
}

void loft_ssh_close(struct loft_ssh *s) {
    if (!s) return;
    if (!s->dead) {
        sbuf p = { 0, 0, 0 };
        if (s->chan_open && !s->close_sent) {
            if (!s->chan_eof && sb_u8(&p, MSG_CHANNEL_EOF) && sb_u32(&p, s->remote_id)) ssh_send(s, &p);
            p.len = 0;
            if (sb_u8(&p, MSG_CHANNEL_CLOSE) && sb_u32(&p, s->remote_id)) ssh_send(s, &p);
        }
        p.len = 0;
        if (sb_u8(&p, MSG_DISCONNECT) && sb_u32(&p, 11) && sb_cstr(&p, "") && sb_cstr(&p, "")) ssh_send(s, &p);
        free(p.p);
    }
    ssh_sock_close(s->fd);
    free(s->in.p);
    free(s->v_c.p);
    free(s->v_s.p);
    free(s->inbound.p);
    free(s->handed.p);
    memset(s, 0, sizeof *s);
    free(s);
}
