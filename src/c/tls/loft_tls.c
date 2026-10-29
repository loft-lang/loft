/* The TLS of loft's C runtime: BearSSL's engine (vendored under `bearssl/`, the upstream the
 * crypto library vendors its primitives from) behind the calls the unit declares — a client
 * session (`loft_tls_open`) and, for the server library's TLS listener, a server session over a
 * certificate chain and key (`loft_tls_server_new`, `loft_tls_accept`; at the end of the file).
 * It is not part of the unit's translation unit: `src/c/tls.rs` builds it with the BearSSL
 * sources into one cached archive, linked only into a unit that fetches over a URL or serves
 * TLS (`LOFT_HTTPS`).
 *
 * What the client promises, as `src/tls.rs` promises it for the Rust build:
 *   - the certificates the machine trusts are trusted: `SSL_CERT_FILE` / `SSL_CERT_DIR` when set,
 *     else the system bundle at the first of the paths curl and OpenSSL probe;
 *   - the server's chain is always validated and its name always checked — a DNS name by
 *     BearSSL, an IP address against the certificate's iPAddress names here (BearSSL matches
 *     DNS names only); nothing turns either off;
 *   - TLS 1.2 with forward-secret suites only (ECDHE, AES-GCM, ChaCha20-Poly1305, AES-CBC).
 *     BearSSL has no TLS 1.3, so a server that speaks only 1.3 refuses the handshake.
 * Unlike the Rust build there are no bundled roots: a machine with no trust store refuses every
 * server, saying where it looked. */
#define _POSIX_C_SOURCE 200809L
#include "bearssl.h"

#include <dirent.h>
#include <errno.h>
#include <poll.h>
#include <pthread.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/socket.h>
#include <sys/stat.h>
#include <arpa/inet.h>

/* ── The trust store ── */

typedef struct {
    br_x509_trust_anchor *v;
    size_t n, cap;
} loft_tls_anchors;

static loft_tls_anchors loft_tls_ta;
static pthread_once_t loft_tls_once = PTHREAD_ONCE_INIT;
static char loft_tls_where[512];

typedef struct {
    unsigned char *p;
    size_t n, cap;
} loft_tls_buf;

static int loft_tls_grow(loft_tls_buf *b, const void *p, size_t n) {
    if (b->n + n > b->cap) {
        size_t c = b->cap ? b->cap : 1024;
        while (c < b->n + n) c *= 2;
        unsigned char *q = (unsigned char *) realloc(b->p, c);
        if (!q) return 0;
        b->p = q;
        b->cap = c;
    }
    memcpy(b->p + b->n, p, n);
    b->n += n;
    return 1;
}

static void loft_tls_append(void *ctx, const void *p, size_t n) { loft_tls_grow((loft_tls_buf *) ctx, p, n); }

static void *loft_tls_dup(const void *p, size_t n) {
    void *q = malloc(n ? n : 1);
    if (q && n) memcpy(q, p, n);
    return q;
}

/* One DER certificate as a trust anchor: its subject DN and public key, CA-flagged when its
 * basic constraints say so (BearSSL's own `brssl` reads a store the same way). */
static void loft_tls_add_der(const unsigned char *der, size_t n) {
    br_x509_decoder_context dc;
    loft_tls_buf dn = { 0, 0, 0 };
    br_x509_decoder_init(&dc, loft_tls_append, &dn);
    br_x509_decoder_push(&dc, der, n);
    const br_x509_pkey *pk = br_x509_decoder_get_pkey(&dc);
    if (!pk || dn.n == 0) {
        free(dn.p);
        return;
    }
    br_x509_trust_anchor ta;
    memset(&ta, 0, sizeof ta);
    ta.dn.data = dn.p;
    ta.dn.len = dn.n;
    ta.flags = br_x509_decoder_isCA(&dc) ? BR_X509_TA_CA : 0;
    ta.pkey.key_type = pk->key_type;
    if (pk->key_type == BR_KEYTYPE_RSA) {
        ta.pkey.key.rsa.n = (unsigned char *) loft_tls_dup(pk->key.rsa.n, pk->key.rsa.nlen);
        ta.pkey.key.rsa.nlen = pk->key.rsa.nlen;
        ta.pkey.key.rsa.e = (unsigned char *) loft_tls_dup(pk->key.rsa.e, pk->key.rsa.elen);
        ta.pkey.key.rsa.elen = pk->key.rsa.elen;
    } else if (pk->key_type == BR_KEYTYPE_EC) {
        ta.pkey.key.ec.curve = pk->key.ec.curve;
        ta.pkey.key.ec.q = (unsigned char *) loft_tls_dup(pk->key.ec.q, pk->key.ec.qlen);
        ta.pkey.key.ec.qlen = pk->key.ec.qlen;
    } else {
        free(dn.p);
        return;
    }
    if (loft_tls_ta.n == loft_tls_ta.cap) {
        size_t c = loft_tls_ta.cap ? loft_tls_ta.cap * 2 : 256;
        br_x509_trust_anchor *v = (br_x509_trust_anchor *) realloc(loft_tls_ta.v, c * sizeof *v);
        if (!v) return;
        loft_tls_ta.v = v;
        loft_tls_ta.cap = c;
    }
    loft_tls_ta.v[loft_tls_ta.n++] = ta;
}

/* Each object of the PEM text `p` (`n` bytes), as its name and its DER bytes; text that is not
 * PEM, or past a malformed object, yields nothing more. */
static void loft_tls_pem(const unsigned char *p, size_t n, void (*each)(void *, const char *, const unsigned char *, size_t),
                         void *ctx) {
    br_pem_decoder_context pc;
    br_pem_decoder_init(&pc);
    loft_tls_buf der = { 0, 0, 0 };
    char name[128] = "";
    size_t off = 0;
    while (off < n) {
        off += br_pem_decoder_push(&pc, p + off, n - off);
        switch (br_pem_decoder_event(&pc)) {
        case BR_PEM_BEGIN_OBJ:
            snprintf(name, sizeof name, "%s", br_pem_decoder_name(&pc));
            der.n = 0;
            br_pem_decoder_setdest(&pc, loft_tls_append, &der);
            break;
        case BR_PEM_END_OBJ:
            if (der.n) each(ctx, name, der.p, der.n);
            break;
        case BR_PEM_ERROR:
            off = n;
            break;
        default:
            break;
        }
    }
    free(der.p);
}

static int loft_tls_is_cert(const char *name) {
    return strcmp(name, "CERTIFICATE") == 0 || strcmp(name, "X509 CERTIFICATE") == 0 ||
           strcmp(name, "TRUSTED CERTIFICATE") == 0;
}

static void loft_tls_anchor(void *ctx, const char *name, const unsigned char *der, size_t n) {
    (void) ctx;
    if (loft_tls_is_cert(name)) loft_tls_add_der(der, n);
}

/* Every CERTIFICATE object of a PEM file; a file that is not PEM, or an object of another kind,
 * adds nothing.  Answers whether the file could be read. */
static int loft_tls_add_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return 0;
    loft_tls_buf all = { 0, 0, 0 };
    unsigned char chunk[16384];
    size_t got;
    while ((got = fread(chunk, 1, sizeof chunk, f)) > 0) loft_tls_grow(&all, chunk, got);
    fclose(f);
    loft_tls_pem(all.p, all.n, loft_tls_anchor, 0);
    free(all.p);
    return 1;
}

static void loft_tls_add_dir(const char *dir) {
    DIR *d = opendir(dir);
    if (!d) return;
    struct dirent *e;
    while ((e = readdir(d)) != 0) {
        if (e->d_name[0] == '.') continue;
        char path[4096];
        snprintf(path, sizeof path, "%s/%s", dir, e->d_name);
        struct stat st;
        if (stat(path, &st) == 0 && S_ISREG(st.st_mode)) loft_tls_add_file(path);
    }
    closedir(d);
}

/* `rustls-native-certs` reads the store the same way: the environment's when it names one
 * (and then only that), else the system bundle. */
static void loft_tls_load(void) {
    const char *file = getenv("SSL_CERT_FILE"), *dir = getenv("SSL_CERT_DIR");
    if ((file && *file) || (dir && *dir)) {
        snprintf(loft_tls_where, sizeof loft_tls_where, "SSL_CERT_FILE=%s, SSL_CERT_DIR=%s", file ? file : "",
                 dir ? dir : "");
        if (file && *file) loft_tls_add_file(file);
        if (dir && *dir) {
            /* OpenSSL's separator: several directories, `:`-separated. */
            char *copy = strdup(dir), *save = 0;
            for (char *p = copy ? strtok_r(copy, ":", &save) : 0; p; p = strtok_r(0, ":", &save)) loft_tls_add_dir(p);
            free(copy);
        }
        return;
    }
    static const char *const bundles[] = {
        "/etc/ssl/certs/ca-certificates.crt",                /* Debian, Ubuntu, Arch, Gentoo */
        "/etc/pki/tls/certs/ca-bundle.crt",                  /* Fedora, RHEL */
        "/etc/pki/ca-trust/extracted/pem/tls-ca-bundle.pem", /* Fedora, RHEL (extracted) */
        "/etc/ssl/ca-bundle.pem",                            /* openSUSE */
        "/etc/pki/tls/cacert.pem",                           /* OpenELEC */
        "/etc/ssl/cert.pem",                                 /* macOS, Alpine, OpenBSD */
        "/usr/local/share/certs/ca-root-nss.crt",            /* FreeBSD */
        "/usr/local/etc/openssl/cert.pem",                   /* Homebrew */
    };
    snprintf(loft_tls_where, sizeof loft_tls_where, "the system bundle (/etc/ssl/certs/ca-certificates.crt and the "
             "other usual paths; SSL_CERT_FILE names another)");
    for (size_t i = 0; i < sizeof bundles / sizeof bundles[0]; i++) {
        if (loft_tls_add_file(bundles[i]) && loft_tls_ta.n) return;
    }
    loft_tls_add_dir("/etc/ssl/certs");
}

/* ── An IP address is checked against the certificate's iPAddress names ──
 * BearSSL's minimal X.509 engine matches a DNS name only.  For a host written as an address the
 * engine is told no name, the end-entity certificate is kept as it streams by, and its
 * subjectAltName must list the address, as rustls requires. */

/* One DER element at `p` (`n` bytes left): its tag and its contents, 0 when malformed. */
static int loft_tls_der(const unsigned char **p, size_t *n, unsigned *tag, const unsigned char **v, size_t *vn) {
    if (*n < 2) return 0;
    *tag = (*p)[0];
    size_t len = (*p)[1], hdr = 2;
    if (len & 0x80) {
        size_t k = len & 0x7f;
        if (k == 0 || k > 4 || *n < 2 + k) return 0;
        len = 0;
        for (size_t i = 0; i < k; i++) len = (len << 8) | (*p)[2 + i];
        hdr += k;
    }
    if (len > *n - hdr) return 0;
    *v = *p + hdr;
    *vn = len;
    *p += hdr + len;
    *n -= hdr + len;
    return 1;
}

/* Does the certificate `der` list the address `ip` (`iplen` 4 or 16) among its iPAddress names? */
static int loft_tls_cert_has_ip(const unsigned char *der, size_t n, const unsigned char *ip, size_t iplen) {
    static const unsigned char san[] = { 0x55, 0x1d, 0x11 }; /* 2.5.29.17 */
    const unsigned char *p = der, *v, *tbs, *x;
    size_t vn, tn, xn;
    unsigned tag;
    if (!loft_tls_der(&p, &n, &tag, &v, &vn) || tag != 0x30) return 0;
    p = v;
    n = vn;
    if (!loft_tls_der(&p, &n, &tag, &tbs, &tn) || tag != 0x30) return 0;
    /* The TBS fields; extensions are the [3] element, last. */
    while (tn) {
        if (!loft_tls_der(&tbs, &tn, &tag, &x, &xn)) return 0;
        if (tag != 0xa3) continue;
        const unsigned char *exts;
        size_t en;
        if (!loft_tls_der(&x, &xn, &tag, &exts, &en) || tag != 0x30) return 0;
        while (en) {
            const unsigned char *ext, *oid, *val;
            size_t exn, on, valn;
            if (!loft_tls_der(&exts, &en, &tag, &ext, &exn) || tag != 0x30) return 0;
            if (!loft_tls_der(&ext, &exn, &tag, &oid, &on) || tag != 0x06) return 0;
            if (on != sizeof san || memcmp(oid, san, sizeof san) != 0) continue;
            /* An optional `critical` BOOLEAN, then the OCTET STRING. */
            do {
                if (!loft_tls_der(&ext, &exn, &tag, &val, &valn)) return 0;
            } while (tag == 0x01 && exn);
            if (tag != 0x04) return 0;
            const unsigned char *names;
            size_t nn;
            if (!loft_tls_der(&val, &valn, &tag, &names, &nn) || tag != 0x30) return 0;
            while (nn) {
                const unsigned char *g;
                size_t gn;
                if (!loft_tls_der(&names, &nn, &tag, &g, &gn)) return 0;
                if (tag == 0x87 && gn == iplen && memcmp(g, ip, iplen) == 0) return 1;
            }
            return 0;
        }
        return 0;
    }
    return 0;
}

typedef struct {
    const br_x509_class *vtable;
    br_x509_minimal_context inner;
    unsigned char ip[16];
    size_t iplen; /* 0: the name was a DNS name, checked by the inner engine */
    int depth;
    loft_tls_buf ee;
} loft_tls_x509;

static void loft_tls_x_start_chain(const br_x509_class **ctx, const char *server_name) {
    loft_tls_x509 *x = (loft_tls_x509 *) (void *) ctx;
    x->depth = 0;
    x->ee.n = 0;
    x->inner.vtable->start_chain(&x->inner.vtable, server_name);
}

static void loft_tls_x_start_cert(const br_x509_class **ctx, uint32_t length) {
    loft_tls_x509 *x = (loft_tls_x509 *) (void *) ctx;
    x->inner.vtable->start_cert(&x->inner.vtable, length);
}

static void loft_tls_x_append(const br_x509_class **ctx, const unsigned char *buf, size_t len) {
    loft_tls_x509 *x = (loft_tls_x509 *) (void *) ctx;
    if (x->iplen && x->depth == 0) loft_tls_grow(&x->ee, buf, len);
    x->inner.vtable->append(&x->inner.vtable, buf, len);
}

static void loft_tls_x_end_cert(const br_x509_class **ctx) {
    loft_tls_x509 *x = (loft_tls_x509 *) (void *) ctx;
    x->depth++;
    x->inner.vtable->end_cert(&x->inner.vtable);
}

static unsigned loft_tls_x_end_chain(const br_x509_class **ctx) {
    loft_tls_x509 *x = (loft_tls_x509 *) (void *) ctx;
    unsigned err = x->inner.vtable->end_chain(&x->inner.vtable);
    if (err == 0 && x->iplen && !loft_tls_cert_has_ip(x->ee.p, x->ee.n, x->ip, x->iplen)) err = BR_ERR_X509_BAD_SERVER_NAME;
    return err;
}

static const br_x509_pkey *loft_tls_x_get_pkey(const br_x509_class *const *ctx, unsigned *usages) {
    const loft_tls_x509 *x = (const loft_tls_x509 *) (const void *) ctx;
    return x->inner.vtable->get_pkey(&x->inner.vtable, usages);
}

static const br_x509_class loft_tls_x509_vtable = {
    sizeof(loft_tls_x509), loft_tls_x_start_chain, loft_tls_x_start_cert, loft_tls_x_append,
    loft_tls_x_end_cert,   loft_tls_x_end_chain,   loft_tls_x_get_pkey,
};

/* ── The connection ── */

/* A session of either role: a client's validates the server, a server's presents its chain. */
struct loft_tls {
    union {
        br_ssl_client_context c;
        br_ssl_server_context s;
    } u;
    br_ssl_engine_context *eng;
    loft_tls_x509 xc;
    br_sslio_context io;
    int fd;
    unsigned char iobuf[BR_SSL_BUFSIZE_BIDI];
};

static int loft_tls_sock_read(void *ctx, unsigned char *buf, size_t len) {
    int fd = *(int *) ctx;
    for (;;) {
        ssize_t r = recv(fd, buf, len, 0);
        if (r > 0) return (int) r;
        if (r < 0 && errno == EINTR) continue;
        return -1;
    }
}

static int loft_tls_sock_write(void *ctx, const unsigned char *buf, size_t len) {
    int fd = *(int *) ctx;
#ifdef MSG_NOSIGNAL
    int flags = MSG_NOSIGNAL;
#else
    int flags = 0;
#endif
    for (;;) {
        ssize_t w = send(fd, buf, len, flags);
        if (w > 0) return (int) w;
        if (w < 0 && errno == EINTR) continue;
        return -1;
    }
}

/* The engine's profile: `br_ssl_client_init_full` cut to TLS 1.2 and forward-secret suites. */
static void loft_tls_profile(struct loft_tls *t) {
    static const uint16_t suites[] = {
        BR_TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256, BR_TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256,
        BR_TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,       BR_TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        BR_TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,       BR_TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,
        BR_TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA256,       BR_TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256,
        BR_TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA384,       BR_TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA384,
        BR_TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA,          BR_TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA,
        BR_TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA,          BR_TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA,
    };
    static const br_hash_class *const hashes[] = {
        &br_sha1_vtable, &br_sha224_vtable, &br_sha256_vtable, &br_sha384_vtable, &br_sha512_vtable,
    };
    br_ssl_client_context *cc = &t->u.c;
    t->eng = &cc->eng;
    br_x509_minimal_context *xc = &t->xc.inner;
    br_ssl_client_zero(cc);
    br_ssl_engine_set_versions(&cc->eng, BR_TLS12, BR_TLS12);
    br_x509_minimal_init(xc, &br_sha256_vtable, loft_tls_ta.v, loft_tls_ta.n);
    br_ssl_engine_set_suites(&cc->eng, suites, sizeof suites / sizeof suites[0]);
    br_ssl_engine_set_default_rsavrfy(&cc->eng);
    br_ssl_engine_set_default_ecdsa(&cc->eng);
    br_x509_minimal_set_rsa(xc, br_ssl_engine_get_rsavrfy(&cc->eng));
    br_x509_minimal_set_ecdsa(xc, br_ssl_engine_get_ec(&cc->eng), br_ssl_engine_get_ecdsa(&cc->eng));
    for (int id = br_sha1_ID; id <= br_sha512_ID; id++) {
        br_ssl_engine_set_hash(&cc->eng, id, hashes[id - br_sha1_ID]);
        br_x509_minimal_set_hash(xc, id, hashes[id - br_sha1_ID]);
    }
    t->xc.vtable = &loft_tls_x509_vtable;
    br_ssl_engine_set_x509(&cc->eng, &t->xc.vtable);
    br_ssl_engine_set_prf_sha256(&cc->eng, &br_tls12_sha256_prf);
    br_ssl_engine_set_prf_sha384(&cc->eng, &br_tls12_sha384_prf);
    br_ssl_engine_set_default_aes_cbc(&cc->eng);
    br_ssl_engine_set_default_aes_gcm(&cc->eng);
    br_ssl_engine_set_default_chapol(&cc->eng);
}

/* Why a handshake or a record failed, in the words the Rust build's client uses where it has
 * them. */
static void loft_tls_why(int code, char *err, size_t cap) {
    switch (code) {
    case BR_ERR_X509_NOT_TRUSTED:
        snprintf(err, cap, "invalid peer certificate: UnknownIssuer (trusted: %s)", loft_tls_where);
        return;
    case BR_ERR_X509_BAD_SERVER_NAME:
        snprintf(err, cap, "invalid peer certificate: NotValidForName");
        return;
    case BR_ERR_X509_EXPIRED:
        snprintf(err, cap, "invalid peer certificate: Expired");
        return;
    case BR_ERR_BAD_VERSION:
    case BR_ERR_BAD_CIPHER_SUITE:
        snprintf(err, cap, "the server shares no TLS version or cipher suite with this client (TLS 1.2, forward-secret suites)");
        return;
    default:
        break;
    }
    if (code >= BR_ERR_RECV_FATAL_ALERT && code < BR_ERR_RECV_FATAL_ALERT + 256) {
        int alert = code - BR_ERR_RECV_FATAL_ALERT;
        if (alert == 40 || alert == 70 || alert == 71)
            snprintf(err, cap, "the server refused the handshake (alert %d): it shares no TLS version or cipher suite with "
                     "this client (TLS 1.2, forward-secret suites)", alert);
        else
            snprintf(err, cap, "the server ended the handshake (TLS alert %d)", alert);
        return;
    }
    if (code >= BR_ERR_X509_OK && code < BR_ERR_X509_OK + 64)
        snprintf(err, cap, "invalid peer certificate (X.509 error %d)", code);
    else if (code == BR_ERR_IO || code == 0)
        snprintf(err, cap, "the connection closed or timed out during the TLS exchange");
    else
        snprintf(err, cap, "TLS error %d", code);
}

/* A TLS session over the connected socket `fd`, its handshake done, the server's certificate
 * validated for `host`; 0 with `err` saying why.  `fd` stays the caller's to close. */
struct loft_tls *loft_tls_open(int fd, const char *host, char *err, size_t cap) {
    pthread_once(&loft_tls_once, loft_tls_load);
    if (loft_tls_ta.n == 0) {
        snprintf(err, cap, "no trusted certificates were found in %s — install the system's CA certificates "
                 "(ca-certificates) or set SSL_CERT_FILE", loft_tls_where);
        return 0;
    }
    struct loft_tls *t = (struct loft_tls *) calloc(1, sizeof *t);
    if (!t) {
        snprintf(err, cap, "out of memory");
        return 0;
    }
    t->fd = fd;
    loft_tls_profile(t);
    br_ssl_engine_set_buffer(t->eng, t->iobuf, sizeof t->iobuf, 1);
    struct in6_addr a6;
    struct in_addr a4;
    const char *name = host;
    if (inet_pton(AF_INET, host, &a4) == 1) {
        memcpy(t->xc.ip, &a4, 4);
        t->xc.iplen = 4;
        name = 0;
    } else if (inet_pton(AF_INET6, host, &a6) == 1) {
        memcpy(t->xc.ip, &a6, 16);
        t->xc.iplen = 16;
        name = 0;
    }
    if (!br_ssl_client_reset(&t->u.c, name, 0)) {
        loft_tls_why(br_ssl_engine_last_error(t->eng), err, cap);
        free(t->xc.ee.p);
        free(t);
        return 0;
    }
    br_sslio_init(&t->io, t->eng, loft_tls_sock_read, &t->fd, loft_tls_sock_write, &t->fd);
    /* The handshake runs on the first flush: it answers once the engine takes application data. */
    if (br_sslio_flush(&t->io) != 0) {
        int code = br_ssl_engine_last_error(t->eng);
        loft_tls_why(code, err, cap);
        free(t->xc.ee.p);
        free(t);
        return 0;
    }
    return t;
}

/* Up to `n` bytes of the server's data: the count, 0 at a clean or abrupt end, -1 on a failure. */
long loft_tls_read(struct loft_tls *t, void *buf, size_t n) {
    int r = br_sslio_read(&t->io, buf, n);
    if (r > 0) return r;
    int code = br_ssl_engine_last_error(t->eng);
    /* A server that closes the socket without close_notify ends the body, as HTTP/1.1 with
     * `Connection: close` lets it; any other failure is one. */
    return code == BR_ERR_OK || code == BR_ERR_IO ? 0 : -1;
}

/* All `n` bytes sent and flushed: 1, else 0. */
/* Does the session hold decrypted application data a read returns without waiting?  A poller
 * asks this before waiting on the socket, because a record already read off it is not there. */
int loft_tls_pending(struct loft_tls *t) {
    size_t len = 0;
    return (br_ssl_engine_current_state(t->eng) & BR_SSL_RECVAPP) && br_ssl_engine_recvapp_buf(t->eng, &len) && len > 0;
}

int loft_tls_write_all(struct loft_tls *t, const void *buf, size_t n) {
    return br_sslio_write_all(&t->io, buf, n) == 0 && br_sslio_flush(&t->io) == 0;
}

/* Close the session and free it; the socket stays open.  A close_notify is sent when the session
 * is still open, without waiting for the server's (`br_sslio_close` would wait, for as long as
 * a server that keeps the socket open makes it). */
void loft_tls_close(struct loft_tls *t) {
    if (!t) return;
    br_ssl_engine_context *eng = t->eng;
    if (br_ssl_engine_current_state(eng) != BR_SSL_CLOSED) {
        br_ssl_engine_close(eng);
        for (int i = 0; i < 8 && (br_ssl_engine_current_state(eng) & BR_SSL_SENDREC); i++) {
            size_t len;
            unsigned char *rec = br_ssl_engine_sendrec_buf(eng, &len);
            int w = loft_tls_sock_write(&t->fd, rec, len);
            if (w <= 0) break;
            br_ssl_engine_sendrec_ack(eng, (size_t) w);
        }
    }
    free(t->xc.ee.p);
    free(t);
}

/* ── The server's side: a certificate chain and its key, one session per accepted socket ──
 * What `src/http_server.rs` promises with rustls: TLS 1.2 here (BearSSL has no 1.3) with the
 * client's forward-secret suites, the key the certificate's — a pair that does not match
 * refuses the listener, as rustls does, rather than failing every handshake later. */

struct loft_tls_server {
    br_x509_certificate *chain;
    size_t chain_len;
    int key_type; /* BR_KEYTYPE_RSA or BR_KEYTYPE_EC */
    br_rsa_private_key rsa;
    br_ec_private_key ec;
};

static void loft_tls_chain_add(void *ctx, const char *name, const unsigned char *der, size_t n) {
    struct loft_tls_server *c = (struct loft_tls_server *) ctx;
    if (!loft_tls_is_cert(name)) return;
    br_x509_certificate *v = (br_x509_certificate *) realloc(c->chain, (c->chain_len + 1) * sizeof *v);
    if (!v) return;
    c->chain = v;
    v[c->chain_len].data = (unsigned char *) loft_tls_dup(der, n);
    v[c->chain_len].data_len = n;
    c->chain_len++;
}

typedef struct {
    struct loft_tls_server *c;
    int found;
} loft_tls_keyfind;

/* The first private key object: PKCS#8 or the raw RSA / EC forms, copied out of the decoder. */
static void loft_tls_key_add(void *ctx, const char *name, const unsigned char *der, size_t n) {
    loft_tls_keyfind *k = (loft_tls_keyfind *) ctx;
    if (k->found || (strcmp(name, "PRIVATE KEY") != 0 && strcmp(name, "RSA PRIVATE KEY") != 0 &&
                     strcmp(name, "EC PRIVATE KEY") != 0))
        return;
    br_skey_decoder_context dc;
    br_skey_decoder_init(&dc);
    br_skey_decoder_push(&dc, der, n);
    if (br_skey_decoder_last_error(&dc) != 0) return;
    struct loft_tls_server *c = k->c;
    if (br_skey_decoder_key_type(&dc) == BR_KEYTYPE_RSA) {
        const br_rsa_private_key *r = br_skey_decoder_get_rsa(&dc);
        c->rsa.n_bitlen = r->n_bitlen;
        c->rsa.p = (unsigned char *) loft_tls_dup(r->p, r->plen);
        c->rsa.plen = r->plen;
        c->rsa.q = (unsigned char *) loft_tls_dup(r->q, r->qlen);
        c->rsa.qlen = r->qlen;
        c->rsa.dp = (unsigned char *) loft_tls_dup(r->dp, r->dplen);
        c->rsa.dplen = r->dplen;
        c->rsa.dq = (unsigned char *) loft_tls_dup(r->dq, r->dqlen);
        c->rsa.dqlen = r->dqlen;
        c->rsa.iq = (unsigned char *) loft_tls_dup(r->iq, r->iqlen);
        c->rsa.iqlen = r->iqlen;
        c->key_type = BR_KEYTYPE_RSA;
    } else if (br_skey_decoder_key_type(&dc) == BR_KEYTYPE_EC) {
        const br_ec_private_key *e = br_skey_decoder_get_ec(&dc);
        c->ec.curve = e->curve;
        c->ec.x = (unsigned char *) loft_tls_dup(e->x, e->xlen);
        c->ec.xlen = e->xlen;
        c->key_type = BR_KEYTYPE_EC;
    } else {
        return;
    }
    k->found = 1;
}

/* Does the key belong to the leaf certificate: the same type, and the public key it derives
 * the one the certificate carries? */
static int loft_tls_key_matches(const struct loft_tls_server *c) {
    br_x509_decoder_context dc;
    br_x509_decoder_init(&dc, 0, 0);
    br_x509_decoder_push(&dc, c->chain[0].data, c->chain[0].data_len);
    const br_x509_pkey *pk = br_x509_decoder_get_pkey(&dc);
    if (!pk || (int) pk->key_type != c->key_type) return 0;
    if (c->key_type == BR_KEYTYPE_EC) {
        unsigned char q[BR_EC_KBUF_PUB_MAX_SIZE];
        size_t qn = br_ec_compute_pub(br_ec_get_default(), 0, q, &c->ec);
        return pk->key.ec.curve == c->ec.curve && qn == pk->key.ec.qlen && memcmp(q, pk->key.ec.q, qn) == 0;
    }
    br_rsa_compute_modulus modulus = br_rsa_compute_modulus_get_default();
    size_t nn = modulus(0, &c->rsa);
    unsigned char *m = (unsigned char *) malloc(nn ? nn : 1);
    if (!m || modulus(m, &c->rsa) != nn) {
        free(m);
        return 0;
    }
    /* The certificate's modulus may carry a leading zero the computed one does not. */
    const unsigned char *cn = pk->key.rsa.n;
    size_t cl = pk->key.rsa.nlen;
    while (cl > 0 && *cn == 0) cn++, cl--;
    const unsigned char *mn = m;
    size_t ml = nn;
    while (ml > 0 && *mn == 0) mn++, ml--;
    int same = cl == ml && memcmp(cn, mn, ml) == 0;
    free(m);
    return same;
}

void loft_tls_server_free(struct loft_tls_server *c) {
    if (!c) return;
    for (size_t i = 0; i < c->chain_len; i++) free(c->chain[i].data);
    free(c->chain);
    free(c->rsa.p);
    free(c->rsa.q);
    free(c->rsa.dp);
    free(c->rsa.dq);
    free(c->rsa.iq);
    free(c->ec.x);
    free(c);
}

/* The server configuration of a PEM certificate chain (leaf first) and a PEM private key; 0 with
 * `err` saying why, in `src/http_server.rs`'s words. */
struct loft_tls_server *loft_tls_server_new(const char *cert, size_t cert_len, const char *key, size_t key_len,
                                            char *err, size_t cap) {
    struct loft_tls_server *c = (struct loft_tls_server *) calloc(1, sizeof *c);
    if (!c) {
        snprintf(err, cap, "out of memory");
        return 0;
    }
    loft_tls_pem((const unsigned char *) cert, cert_len, loft_tls_chain_add, c);
    if (c->chain_len == 0) {
        snprintf(err, cap, "no certificate found in the PEM cert chain");
        loft_tls_server_free(c);
        return 0;
    }
    loft_tls_keyfind k = { c, 0 };
    loft_tls_pem((const unsigned char *) key, key_len, loft_tls_key_add, &k);
    if (!k.found) {
        snprintf(err, cap, "no private key found in the PEM key");
        loft_tls_server_free(c);
        return 0;
    }
    if (!loft_tls_key_matches(c)) {
        snprintf(err, cap, "bad certificate/key pair: the private key does not match the certificate");
        loft_tls_server_free(c);
        return 0;
    }
    return c;
}

/* A server session over the accepted socket `fd`, its handshake done; 0 with `err` saying why.
 * `fd` stays the caller's to close. */
struct loft_tls *loft_tls_accept(int fd, const struct loft_tls_server *cfg, char *err, size_t cap) {
    static const uint16_t ec_suites[] = {
        BR_TLS_ECDHE_ECDSA_WITH_CHACHA20_POLY1305_SHA256, BR_TLS_ECDHE_ECDSA_WITH_AES_128_GCM_SHA256,
        BR_TLS_ECDHE_ECDSA_WITH_AES_256_GCM_SHA384,       BR_TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA256,
        BR_TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA384,       BR_TLS_ECDHE_ECDSA_WITH_AES_128_CBC_SHA,
        BR_TLS_ECDHE_ECDSA_WITH_AES_256_CBC_SHA,
    };
    static const uint16_t rsa_suites[] = {
        BR_TLS_ECDHE_RSA_WITH_CHACHA20_POLY1305_SHA256, BR_TLS_ECDHE_RSA_WITH_AES_128_GCM_SHA256,
        BR_TLS_ECDHE_RSA_WITH_AES_256_GCM_SHA384,       BR_TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA256,
        BR_TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA384,       BR_TLS_ECDHE_RSA_WITH_AES_128_CBC_SHA,
        BR_TLS_ECDHE_RSA_WITH_AES_256_CBC_SHA,
    };
    static const br_hash_class *const hashes[] = {
        &br_sha1_vtable, &br_sha224_vtable, &br_sha256_vtable, &br_sha384_vtable, &br_sha512_vtable,
    };
    struct loft_tls *t = (struct loft_tls *) calloc(1, sizeof *t);
    if (!t) {
        snprintf(err, cap, "out of memory");
        return 0;
    }
    t->fd = fd;
    br_ssl_server_context *sc = &t->u.s;
    t->eng = &sc->eng;
    br_ssl_server_zero(sc);
    br_ssl_engine_set_versions(t->eng, BR_TLS12, BR_TLS12);
    if (cfg->key_type == BR_KEYTYPE_EC) {
        br_ssl_engine_set_suites(t->eng, ec_suites, sizeof ec_suites / sizeof ec_suites[0]);
    } else {
        br_ssl_engine_set_suites(t->eng, rsa_suites, sizeof rsa_suites / sizeof rsa_suites[0]);
    }
    for (int id = br_sha1_ID; id <= br_sha512_ID; id++) br_ssl_engine_set_hash(t->eng, id, hashes[id - br_sha1_ID]);
    br_ssl_engine_set_prf_sha256(t->eng, &br_tls12_sha256_prf);
    br_ssl_engine_set_prf_sha384(t->eng, &br_tls12_sha384_prf);
    br_ssl_engine_set_default_aes_cbc(t->eng);
    br_ssl_engine_set_default_aes_gcm(t->eng);
    br_ssl_engine_set_default_chapol(t->eng);
    br_ssl_engine_set_default_ec(t->eng);
    if (cfg->key_type == BR_KEYTYPE_EC) {
        br_ssl_server_set_single_ec(sc, cfg->chain, cfg->chain_len, &cfg->ec, BR_KEYTYPE_SIGN, 0, br_ec_get_default(),
                                    br_ecdsa_sign_asn1_get_default());
    } else {
        br_ssl_server_set_single_rsa(sc, cfg->chain, cfg->chain_len, &cfg->rsa, BR_KEYTYPE_SIGN,
                                     br_rsa_private_get_default(), br_rsa_pkcs1_sign_get_default());
    }
    br_ssl_engine_set_buffer(t->eng, t->iobuf, sizeof t->iobuf, 1);
    if (!br_ssl_server_reset(sc)) {
        loft_tls_why(br_ssl_engine_last_error(t->eng), err, cap);
        free(t);
        return 0;
    }
    br_sslio_init(&t->io, t->eng, loft_tls_sock_read, &t->fd, loft_tls_sock_write, &t->fd);
    if (br_sslio_flush(&t->io) != 0) {
        loft_tls_why(br_ssl_engine_last_error(t->eng), err, cap);
        free(t);
        return 0;
    }
    return t;
}

/* Is application data ready to read, without consuming any: 1 ready, 0 none within `ms`, -1 the
 * session is over.  Bytes that arrive are fed to the engine as records, so the decrypted data
 * waits in it for the next read (BearSSL cannot peek; this is its engine driven by hand).  A
 * record begun is awaited for up to two seconds more; a non-blocking read would fail the
 * session instead. */
int loft_tls_poll(struct loft_tls *t, int ms) {
    br_ssl_engine_context *eng = t->eng;
    for (;;) {
        unsigned st = br_ssl_engine_current_state(eng);
        if (st == BR_SSL_CLOSED) return -1;
        if (st & BR_SSL_SENDREC) {
            size_t len;
            unsigned char *rec = br_ssl_engine_sendrec_buf(eng, &len);
            int w = loft_tls_sock_write(&t->fd, rec, len);
            if (w <= 0) return -1;
            br_ssl_engine_sendrec_ack(eng, (size_t) w);
            continue;
        }
        if (st & BR_SSL_RECVAPP) return 1;
        if (!(st & BR_SSL_RECVREC)) return 0;
        struct pollfd p = { t->fd, POLLIN, 0 };
        int r = poll(&p, 1, ms);
        if (r < 0 && errno == EINTR) continue;
        if (r == 0) return 0;
        if (r < 0) return -1;
        size_t len;
        unsigned char *buf = br_ssl_engine_recvrec_buf(eng, &len);
        ssize_t got = recv(t->fd, buf, len, 0);
        if (got < 0 && errno == EINTR) continue;
        if (got <= 0) return -1;
        br_ssl_engine_recvrec_ack(eng, (size_t) got);
        ms = 2000;
    }
}
