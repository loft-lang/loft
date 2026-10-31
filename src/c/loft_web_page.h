/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * The web library's natives in a page the C backend writes (`--native-c --html`): the C twin of
 * `src/web_browser.rs`, over the same `loft_web` imports `doc/loft-web.js` provides (a
 * `WebSocket` per handle with its inbound queue, `fetch()`), so the page script serves both
 * writers.  A host's are the `net` component (`loft_web.h`).
 *
 * `fetch()` and `WebSocket.onmessage` are asynchronous, so a call waits through `ws_yield`, the
 * asyncify suspend the page's `wasm-opt --asyncify` pass instruments.  Bodies, messages and the
 * pack buffer are bytes carried in a text, read with `byte_at` — never as characters. */

#if defined(LOFT_BROWSER)
#define LOFT_WB_IMPORT(name) __attribute__((import_module("loft_web"), import_name(#name)))
LOFT_WB_IMPORT(ws_connect) int32_t loft_wbi_ws_connect(const char *url, size_t len);
LOFT_WB_IMPORT(ws_send) int32_t loft_wbi_ws_send(int32_t h, const char *p, size_t len);
LOFT_WB_IMPORT(ws_send_binary) int32_t loft_wbi_ws_send_binary(int32_t h, const char *p, size_t len);
LOFT_WB_IMPORT(ws_poll) int32_t loft_wbi_ws_poll(int32_t h);
LOFT_WB_IMPORT(ws_msg_len) int32_t loft_wbi_ws_msg_len(int32_t h);
LOFT_WB_IMPORT(ws_msg_copy) void loft_wbi_ws_msg_copy(int32_t h, char *p);
LOFT_WB_IMPORT(ws_opcode) int32_t loft_wbi_ws_opcode(int32_t h);
LOFT_WB_IMPORT(ws_close) void loft_wbi_ws_close(int32_t h);
LOFT_WB_IMPORT(ws_yield) void loft_wbi_ws_yield(void);
LOFT_WB_IMPORT(http_start) int32_t loft_wbi_http_start(const char *m, size_t ml, const char *u,
                                                       size_t ul, const char *b, size_t bl,
                                                       const char *h, size_t hl);
LOFT_WB_IMPORT(http_poll) int32_t loft_wbi_http_poll(int32_t h);
LOFT_WB_IMPORT(http_status) int32_t loft_wbi_http_status(int32_t h);
LOFT_WB_IMPORT(http_body_len) int32_t loft_wbi_http_body_len(int32_t h);
LOFT_WB_IMPORT(http_body_copy) void loft_wbi_http_body_copy(int32_t h, char *p);
LOFT_WB_IMPORT(http_headers_len) int32_t loft_wbi_http_headers_len(int32_t h);
LOFT_WB_IMPORT(http_headers_copy) void loft_wbi_http_headers_copy(int32_t h, char *p);
LOFT_WB_IMPORT(http_free) void loft_wbi_http_free(int32_t h);

/* The last answer of each kind, as the Rust twin keeps them. */
static loft_text loft_wp_body, loft_wp_heads, loft_wp_msg, loft_wp_pack;
static int64_t loft_wp_op = 0;

static int32_t loft_wp_handle(int64_t h) { return h < INT32_MIN || h > INT32_MAX ? -1 : (int32_t) h; }

/* Replace `t` with the `n` bytes `copy` writes. */
static void loft_wp_fill(loft_text *t, int32_t n, int32_t h, void (*copy)(int32_t, char *)) {
    t->len = 0;
    if (n <= 0) return;
    char *b = (char *) malloc((size_t) n);
    if (!b) return;
    copy(h, b);
    loft_push(t, b, (size_t) n);
    free(b);
}

static loft_text loft_wp_copy(const loft_text *t) {
    return loft_text_own(loft_lit(t->p ? t->p : "", t->len));
}

/* A fetch run to completion — the page yields to its event loop until the answer is in — with
 * its body and headers kept: the status, or 0 on a transport failure. */
static int64_t loft_wp_http_run(const char *m, size_t ml, loft_str url, loft_str body,
                                loft_str headers) {
    int32_t h = loft_wbi_http_start(m, ml, url.p, url.len, body.p, body.len, headers.p,
                                    headers.len);
    if (h < 0) return 0;
    for (;;) {
        int32_t p = loft_wbi_http_poll(h);
        if (p == 1) break;
        if (p < 0) {
            loft_wbi_http_free(h);
            return 0;
        }
        loft_wbi_ws_yield();
    }
    int64_t status = loft_wbi_http_status(h);
    loft_wp_fill(&loft_wp_body, loft_wbi_http_body_len(h), h, loft_wbi_http_body_copy);
    loft_wp_fill(&loft_wp_heads, loft_wbi_http_headers_len(h), h, loft_wbi_http_headers_copy);
    loft_wbi_http_free(h);
    return status;
}

static int64_t loft_wp_http_do(loft_str method, loft_str url, loft_str body, loft_str headers) {
    return loft_wp_http_run(method.p, method.len, url, body, headers);
}
static loft_text loft_wp_http_body(void) { return loft_wp_copy(&loft_wp_body); }
static loft_text loft_wp_http_headers(void) {
    loft_text t = { 0, 0, 0 };
    loft_hi_push_lossy(&t, (const unsigned char *) (loft_wp_heads.p ? loft_wp_heads.p : ""),
                       loft_wp_heads.len);
    return t;
}

/* A `HEAD`'s `Content-Length`, or -1 — also when the page's CORS hides the header; the library
 * then falls back to a range read. */
static int64_t loft_wp_http_head_size(loft_str url) {
    loft_str none = { "", 0 };
    int64_t status = loft_wp_http_run("HEAD", 4, url, none, none);
    if (status < 200 || status >= 400) return -1;
    const char *p = loft_wp_heads.p ? loft_wp_heads.p : "";
    size_t n = loft_wp_heads.len;
    for (size_t i = 0; i < n;) {
        size_t e = i;
        while (e < n && p[e] != '\n') e++;
        size_t c = i;
        while (c < e && p[c] != ':') c++;
        if (c < e) {
            size_t ks = i, ke = c;
            while (ks < ke && (p[ks] == ' ' || p[ks] == '\t' || p[ks] == '\r')) ks++;
            while (ke > ks && (p[ke - 1] == ' ' || p[ke - 1] == '\t' || p[ke - 1] == '\r')) ke--;
            static const char want[] = "content-length";
            int same = ke - ks == sizeof want - 1;
            for (size_t k = 0; same && k < sizeof want - 1; k++) {
                char ch = p[ks + k];
                if (ch >= 'A' && ch <= 'Z') ch = (char) (ch - 'A' + 'a');
                same = ch == want[k];
            }
            if (same) {
                size_t vs = c + 1, ve = e;
                while (vs < ve && (p[vs] == ' ' || p[vs] == '\t')) vs++;
                while (ve > vs && (p[ve - 1] == ' ' || p[ve - 1] == '\t' || p[ve - 1] == '\r')) ve--;
                if (vs == ve) return -1;
                int64_t v = 0;
                for (size_t k = vs; k < ve; k++) {
                    if (p[k] < '0' || p[k] > '9') return -1;
                    v = v * 10 + (p[k] - '0');
                }
                return v;
            }
        }
        i = e + 1;
    }
    return -1;
}

static int64_t loft_wp_ws_connect(loft_str url) { return loft_wbi_ws_connect(url.p, url.len); }
static uint8_t loft_wp_ws_client_send(int64_t h, loft_str msg) {
    return loft_wbi_ws_send(loft_wp_handle(h), msg.p, msg.len) == 1;
}
static uint8_t loft_wp_ws_client_send_binary(int64_t h, loft_str msg) {
    return loft_wbi_ws_send_binary(loft_wp_handle(h), msg.p, msg.len) == 1;
}
static uint8_t loft_wp_ws_client_recv(int64_t h) {
    int32_t s = loft_wp_handle(h);
    if (loft_wbi_ws_poll(s) != 1) return 0;
    loft_wp_fill(&loft_wp_msg, loft_wbi_ws_msg_len(s), s, loft_wbi_ws_msg_copy);
    loft_wp_op = loft_wbi_ws_opcode(s);
    return 1;
}
static loft_text loft_wp_ws_client_message(void) { return loft_wp_copy(&loft_wp_msg); }
static int64_t loft_wp_ws_client_opcode(void) { return loft_wp_op; }
static void loft_wp_ws_client_close(int64_t h) { loft_wbi_ws_close(loft_wp_handle(h)); }
/* The page's event loop paces a program; it paces itself with `frame_yield`. */
static void loft_wp_sleep_ms(int64_t ms) { (void) ms; }
static void loft_wp_frame_yield(void) { loft_wbi_ws_yield(); }

static void loft_wp_pack_reset(void) { loft_wp_pack.len = 0; }
static void loft_wp_pack_u8(int64_t b) {
    char c = (char) (b & 0xFF);
    loft_push(&loft_wp_pack, &c, 1);
}
static void loft_wp_pack_u16_le(int64_t v) {
    char c[2] = { (char) (v & 0xFF), (char) ((v >> 8) & 0xFF) };
    loft_push(&loft_wp_pack, c, 2);
}
static void loft_wp_pack_u32_le(int64_t v) {
    char c[4] = { (char) (v & 0xFF), (char) ((v >> 8) & 0xFF), (char) ((v >> 16) & 0xFF),
                  (char) ((v >> 24) & 0xFF) };
    loft_push(&loft_wp_pack, c, 4);
}
static loft_text loft_wp_pack_take(void) {
    loft_text t = loft_wp_copy(&loft_wp_pack);
    loft_wp_pack.len = 0;
    return t;
}
static int64_t loft_wp_byte_at(int64_t idx, loft_str t) {
    return idx < 0 || (uint64_t) idx >= t.len ? -1 : (int64_t) (unsigned char) t.p[idx];
}
/* A page rarely multiplexes sockets: the group is a stub, nothing is ever ready. */
static void loft_wp_ws_group_clear(void) {}
static void loft_wp_ws_group_add(int64_t h) { (void) h; }
static int64_t loft_wp_ws_group_poll(void) { return -1; }
#endif
