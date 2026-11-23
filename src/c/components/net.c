/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * libloft_net — the component behind the web, server and ssh libraries (@PLN205 L1.2, L1.3): their
 * `#native` functions as the loft-ffi ABI's typed symbols and bridges
 * (`loft-ffi/include/loft_ffi.h`), over the C runtime's own HTTP client, TLS and WebSocket code
 * (`loft_https.h`, which a C unit also pastes; `loft_web.h`, `loft_server.h`, `loft_ssh.h`,
 * `tls/loft_tls.c`, `tls/loft_ssh.c`).  Built ahead of time by `loft-components`
 * (`src/c/component.rs`), never where it is used. */
#include "loft_os.h"
#include "loft_width.h"
#include "loft_cc.h"
#include "loft_cx.h"
#include "loft_text.h"
#define LOFT_HTTPS 1
#define LOFT_WEB 1
#define LOFT_SERVER 1
#define LOFT_SSH 1
#include "loft_sock.h"
#include "loft_https.h"
#include "loft_web.h"
#include "loft_server.h"
#include "loft_ssh.h"
#include "loft_ffi.h"

#if defined(_WIN32)
#define NET_EXPORT __declspec(dllexport)
#else
#define NET_EXPORT __attribute__((visibility("default")))
#endif

/* ── The component's context (@PLN226) ──
 * Each caller context — a C unit's main thread or `par` worker, an interpreter `Stores` —
 * keeps one of the component's own, made by `loft_cx_make_v1` the first time it calls in and
 * ended by `loft_cx_end_v1` with it, and hands it to every symbol first: the connection tables,
 * the last answer, the text pool are that caller's.  The caller's own context is not passed: the
 * component is compiled with its own switches, so its struct is laid out its own way. */
NET_EXPORT void *loft_cx_make_v1(void) {
    loft_cx *cx = (loft_cx *) malloc(sizeof *cx);
    if (!cx) { fputs("loft: out of memory\n", stderr); abort(); }
    loft_cx_fresh(cx);
    return cx;
}

NET_EXPORT void loft_cx_end_v1(void *p) {
    loft_cx *cx = (loft_cx *) p;
    loft_text *texts[] = { &cx->net_answer, &cx->web_body, &cx->web_heads, &cx->web_pack, &cx->web_msg,
                           &cx->srv_method, &cx->srv_path, &cx->srv_heads, &cx->srv_body, &cx->srv_msg,
                           &cx->srv_ev_payload };
    for (size_t i = 0; i < sizeof texts / sizeof texts[0]; i++) loft_text_free(cx, texts[i]);
    for (size_t i = 0; i < cx->web_nconns; i++) free(cx->web_conns[i].url);
    free(cx->web_conns);
    free(cx->web_group);
    for (size_t i = 0; i < cx->srv_nls; i++) free(cx->srv_ls[i].origins);
    free(cx->srv_ls);
    free(cx->srv_clients);
    free(cx->ssh_tab);
    for (int c = 0; c < LOFT_TP_CLASSES; c++)
        while (cx->tp_n[c]) free(cx->tp_blocks[c][--cx->tp_n[c]]);
    free(cx);
}

/* A text answer stays the library's until the caller's next text answer (the ABI's promise);
 * loft copies it at once. */
static loft_ffi_str net_text(loft_cx *cx, loft_text t) {
    loft_text_free(cx, &cx->net_answer);
    cx->net_answer = t;
    loft_ffi_str s = { (const uint8_t *) (t.p ? t.p : ""), t.len };
    return s;
}

static loft_str net_arg(loft_cx *cx, const uint8_t *p, size_t n) { return loft_lit(cx, (const char *) (p ? p : (const uint8_t *) ""), n); }

/* ── Typed symbols: what compiled code calls ── */

NET_EXPORT int64_t n_http_do(loft_cx *cx, const uint8_t *m, size_t mn, const uint8_t *u, size_t un, const uint8_t *b, size_t bn,
                             const uint8_t *h, size_t hn) {
    return loft_web_http_do(cx, net_arg(cx, m, mn), net_arg(cx, u, un), net_arg(cx, b, bn), net_arg(cx, h, hn));
}
NET_EXPORT loft_ffi_str n_http_body(loft_cx *cx) { return net_text(cx, loft_web_http_body(cx)); }
NET_EXPORT loft_ffi_str n_http_headers(loft_cx *cx) { return net_text(cx, loft_web_http_headers(cx)); }
NET_EXPORT int64_t n_http_head_size(loft_cx *cx, const uint8_t *u, size_t un) { return loft_web_http_head_size(cx, net_arg(cx, u, un)); }
NET_EXPORT int64_t n_ws_connect(loft_cx *cx, const uint8_t *u, size_t un) { return loft_web_ws_connect(cx, net_arg(cx, u, un)); }
NET_EXPORT uint8_t n_ws_client_send(loft_cx *cx, int64_t h, const uint8_t *m, size_t mn) { return loft_web_ws_client_send(cx, h, net_arg(cx, m, mn)); }
NET_EXPORT uint8_t n_ws_client_send_binary(loft_cx *cx, int64_t h, const uint8_t *m, size_t mn) {
    return loft_web_ws_client_send_binary(cx, h, net_arg(cx, m, mn));
}
NET_EXPORT uint8_t n_ws_client_recv(loft_cx *cx, int64_t h) { return loft_web_ws_client_recv(cx, h); }
NET_EXPORT loft_ffi_str n_ws_client_message(loft_cx *cx) { return net_text(cx, loft_web_ws_client_message(cx)); }
NET_EXPORT int64_t n_ws_client_opcode(loft_cx *cx) { return loft_web_ws_client_opcode(cx); }
NET_EXPORT void n_ws_client_close(loft_cx *cx, int64_t h) { loft_web_ws_client_close(cx, h); }
NET_EXPORT void n_sleep_ms(loft_cx *cx, int64_t ms) { loft_web_sleep_ms(cx, ms); }
NET_EXPORT void n_pack_reset(loft_cx *cx) { loft_web_pack_reset(cx); }
NET_EXPORT void n_pack_u8(loft_cx *cx, int64_t b) { loft_web_pack_u8(cx, b); }
NET_EXPORT void n_pack_u16_le(loft_cx *cx, int64_t v) { loft_web_pack_u16_le(cx, v); }
NET_EXPORT void n_pack_u32_le(loft_cx *cx, int64_t v) { loft_web_pack_u32_le(cx, v); }
NET_EXPORT loft_ffi_str n_pack_take(loft_cx *cx) { return net_text(cx, loft_web_pack_take(cx)); }
NET_EXPORT int64_t n_byte_at(loft_cx *cx, int64_t idx, const uint8_t *t, size_t tn) { return loft_web_byte_at(cx, idx, net_arg(cx, t, tn)); }
NET_EXPORT void n_ws_group_clear(loft_cx *cx) { loft_web_ws_group_clear(cx); }
NET_EXPORT void n_ws_group_add(loft_cx *cx, int64_t h) { loft_web_ws_group_add(cx, h); }
NET_EXPORT int64_t n_ws_group_poll(loft_cx *cx) { return loft_web_ws_group_poll(cx); }
NET_EXPORT void n_frame_yield(loft_cx *cx) { loft_web_frame_yield(cx); }

/* The server library's (`loft_server.h`). */
NET_EXPORT int64_t n_tcp_listen(loft_cx *cx, int64_t port) { return loft_srv_bind(cx, "", 0, port, 0); }
NET_EXPORT int64_t n_tcp_listen_on(loft_cx *cx, const uint8_t *a, size_t an, int64_t port) {
    return loft_srv_bind(cx, (const char *) a, an, port, 0);
}
NET_EXPORT int64_t n_tcp_listen_tls(loft_cx *cx, int64_t port, const uint8_t *c, size_t cn, const uint8_t *k, size_t kn) {
    return loft_srv_listen_tls(cx, "", 0, port, (const char *) c, cn, (const char *) k, kn);
}
NET_EXPORT int64_t n_tcp_listen_tls_on(loft_cx *cx, const uint8_t *a, size_t an, int64_t port, const uint8_t *c, size_t cn,
                                       const uint8_t *k, size_t kn) {
    return loft_srv_listen_tls(cx, (const char *) a, an, port, (const char *) c, cn, (const char *) k, kn);
}
NET_EXPORT uint8_t n_tcp_accept(loft_cx *cx, int64_t h) { return loft_srv_tcp_accept(cx, h, 0); }
NET_EXPORT uint8_t n_tcp_accept_nonblocking(loft_cx *cx, int64_t h) { return loft_srv_tcp_accept(cx, h, 1); }
NET_EXPORT loft_ffi_str n_tcp_method(loft_cx *cx) { return net_text(cx, loft_srv_copy(cx, &cx->srv_method)); }
NET_EXPORT loft_ffi_str n_tcp_path(loft_cx *cx) { return net_text(cx, loft_srv_copy(cx, &cx->srv_path)); }
NET_EXPORT loft_ffi_str n_tcp_body(loft_cx *cx) { return net_text(cx, loft_srv_copy(cx, &cx->srv_body)); }
NET_EXPORT loft_ffi_str n_tcp_headers(loft_cx *cx) { return net_text(cx, loft_srv_copy(cx, &cx->srv_heads)); }
NET_EXPORT void n_tcp_respond(loft_cx *cx, int64_t status, const uint8_t *b, size_t bn) {
    static const char ct[] = "text/plain; charset=utf-8";
    loft_srv_respond(cx, status, ct, sizeof ct - 1, (const char *) b, bn);
}
NET_EXPORT void n_tcp_respond_typed(loft_cx *cx, int64_t status, const uint8_t *b, size_t bn, const uint8_t *c, size_t cn) {
    static const char ct[] = "text/plain; charset=utf-8";
    loft_srv_respond(cx, status, cn ? (const char *) c : ct, cn ? cn : sizeof ct - 1, (const char *) b, bn);
}
NET_EXPORT void n_tcp_respond_bytes(loft_cx *cx, loft_ffi_store s, int64_t status, loft_ffi_ref body, const uint8_t *c, size_t cn) {
    static const char ct[] = "application/octet-stream";
    uint32_t n = body.rec ? loft_ffi_vector_len(s, body) : 0;
    const char *d = n ? (const char *) loft_ffi_vector_data(s, body) : "";
    loft_srv_respond(cx, status, cn ? (const char *) c : ct, cn ? cn : sizeof ct - 1, d, n);
}
NET_EXPORT void n_tcp_respond_full(loft_cx *cx, int64_t status, const uint8_t *b, size_t bn, const uint8_t *h, size_t hn) {
    loft_srv_respond_full(cx, status, net_arg(cx, b, bn), net_arg(cx, h, hn));
}
NET_EXPORT loft_ffi_str n_tcp_slice(loft_cx *cx, const uint8_t *b, size_t bn, int64_t off, int64_t len) {
    return net_text(cx, loft_srv_slice(cx, net_arg(cx, b, bn), off, len));
}
NET_EXPORT void n_tcp_close(loft_cx *cx, int64_t h) { loft_srv_tcp_close(cx, h); }
NET_EXPORT int64_t n_ws_upgrade(loft_cx *cx) { return loft_srv_ws_upgrade(cx); }
NET_EXPORT uint8_t n_ws_recv(loft_cx *cx, int64_t id) { return loft_srv_ws_recv(cx, id); }
NET_EXPORT loft_ffi_str n_ws_message(loft_cx *cx) { return net_text(cx, loft_srv_copy(cx, &cx->srv_msg)); }
NET_EXPORT int64_t n_ws_opcode(loft_cx *cx) { return cx->srv_msg_op; }
NET_EXPORT uint8_t n_ws_send(loft_cx *cx, int64_t id, const uint8_t *m, size_t mn) { return loft_srv_ws_send(cx, id, 0x1, net_arg(cx, m, mn)); }
NET_EXPORT uint8_t n_ws_send_binary(loft_cx *cx, int64_t id, const uint8_t *m, size_t mn) {
    return loft_srv_ws_send(cx, id, 0x2, net_arg(cx, m, mn));
}
NET_EXPORT void n_ws_close(loft_cx *cx, int64_t id) { loft_srv_ws_close(cx, id); }
NET_EXPORT int64_t n_ws_accept_nonblocking(loft_cx *cx, int64_t h) { return loft_srv_ws_accept_nonblocking(cx, h); }
NET_EXPORT int64_t n_ws_clients_len(loft_cx *cx) { return loft_srv_clients_len(cx); }
NET_EXPORT uint8_t n_ws_client_active(loft_cx *cx, int64_t id) { return loft_srv_client_active(cx, id); }
NET_EXPORT void n_ws_allow_origins(loft_cx *cx, int64_t h, const uint8_t *o, size_t on) { loft_srv_allow_origins(cx, h, net_arg(cx, o, on)); }
NET_EXPORT uint8_t n_ws_next_event(loft_cx *cx, int64_t h) { return loft_srv_ws_next_event(cx, h); }
NET_EXPORT int64_t n_ws_event_kind(loft_cx *cx) { return cx->srv_ev_kind; }
NET_EXPORT int64_t n_ws_event_client_id(loft_cx *cx) { return cx->srv_ev_cid; }
NET_EXPORT loft_ffi_str n_ws_event_payload(loft_cx *cx) { return net_text(cx, loft_srv_copy(cx, &cx->srv_ev_payload)); }
NET_EXPORT int64_t n_ws_broadcast(loft_cx *cx, int64_t h, const uint8_t *m, size_t mn) {
    (void) h;
    return loft_srv_ws_broadcast(cx, net_arg(cx, m, mn));
}
NET_EXPORT void n_ws_idle_sleep_ms(loft_cx *cx, int64_t ms) { loft_srv_sleep_ms(cx, ms); }
NET_EXPORT int64_t n_ssh_connect(loft_cx *cx, const uint8_t *h, size_t hn, int64_t port) { return loft_ssh_connect(cx, net_arg(cx, h, hn), port); }
NET_EXPORT loft_ffi_str n_ssh_host_key(loft_cx *cx, int64_t h) { return net_text(cx, loft_ssh_host_key_h(cx, h)); }
NET_EXPORT uint8_t n_ssh_login(loft_cx *cx, int64_t h, const uint8_t *u, size_t un, const uint8_t *p, size_t pn) {
    return loft_ssh_login_h(cx, h, net_arg(cx, u, un), net_arg(cx, p, pn));
}
NET_EXPORT uint8_t n_ssh_open_shell(loft_cx *cx, int64_t h, const uint8_t *t, size_t tn, int64_t cols, int64_t rows) {
    return loft_ssh_shell_h(cx, h, net_arg(cx, t, tn), cols, rows);
}
NET_EXPORT void n_ssh_send(loft_cx *cx, int64_t h, const uint8_t *d, size_t dn) { loft_ssh_send_h(cx, h, net_arg(cx, d, dn)); }
NET_EXPORT loft_ffi_str n_ssh_recv(loft_cx *cx, int64_t h) { return net_text(cx, loft_ssh_recv_h(cx, h)); }
NET_EXPORT void n_ssh_resize(loft_cx *cx, int64_t h, int64_t cols, int64_t rows) { loft_ssh_resize_h(cx, h, cols, rows); }
NET_EXPORT uint8_t n_ssh_is_open(loft_cx *cx, int64_t h) { return loft_ssh_is_open_h(cx, h); }
NET_EXPORT void n_ssh_close(loft_cx *cx, int64_t h) { loft_ssh_close_h(cx, h); }

/* ── Bridges: what the interpreter calls, one per symbol ── */

#define T(k) a[k].payload.text.ptr, a[k].payload.text.len
#define I(k) a[k].payload.i
#define BRIDGE(name) static void b_##name(void *ccx, loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r)
#define UNUSED loft_cx *cx = (loft_cx *) ccx; (void) s; (void) a; (void) n
BRIDGE(http_do) { UNUSED; *r = loft_ffi_value_i64(n_http_do(cx, T(0), T(1), T(2), T(3))); }
BRIDGE(http_body) { UNUSED; *r = loft_ffi_value_text(n_http_body(cx)); }
BRIDGE(http_headers) { UNUSED; *r = loft_ffi_value_text(n_http_headers(cx)); }
BRIDGE(http_head_size) { UNUSED; *r = loft_ffi_value_i64(n_http_head_size(cx, T(0))); }
BRIDGE(ws_connect) { UNUSED; *r = loft_ffi_value_i64(n_ws_connect(cx, T(0))); }
BRIDGE(ws_client_send) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_send(cx, I(0), T(1))); }
BRIDGE(ws_client_send_binary) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_send_binary(cx, I(0), T(1))); }
BRIDGE(ws_client_recv) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_recv(cx, I(0))); }
BRIDGE(ws_client_message) { UNUSED; *r = loft_ffi_value_text(n_ws_client_message(cx)); }
BRIDGE(ws_client_opcode) { UNUSED; *r = loft_ffi_value_i64(n_ws_client_opcode(cx)); }
BRIDGE(ws_client_close) { UNUSED; n_ws_client_close(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(sleep_ms) { UNUSED; n_sleep_ms(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_reset) { UNUSED; n_pack_reset(cx); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_u8) { UNUSED; n_pack_u8(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_u16_le) { UNUSED; n_pack_u16_le(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_u32_le) { UNUSED; n_pack_u32_le(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_take) { UNUSED; *r = loft_ffi_value_text(n_pack_take(cx)); }
BRIDGE(byte_at) { UNUSED; *r = loft_ffi_value_i64(n_byte_at(cx, I(0), T(1))); }
BRIDGE(ws_group_clear) { UNUSED; n_ws_group_clear(cx); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_group_add) { UNUSED; n_ws_group_add(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_group_poll) { UNUSED; *r = loft_ffi_value_i64(n_ws_group_poll(cx)); }
BRIDGE(frame_yield) { UNUSED; n_frame_yield(cx); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_listen) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen(cx, I(0))); }
BRIDGE(tcp_listen_on) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen_on(cx, T(0), I(1))); }
BRIDGE(tcp_listen_tls) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen_tls(cx, I(0), T(1), T(2))); }
BRIDGE(tcp_listen_tls_on) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen_tls_on(cx, T(0), I(1), T(2), T(3))); }
BRIDGE(tcp_accept) { UNUSED; *r = loft_ffi_value_bool(n_tcp_accept(cx, I(0))); }
BRIDGE(tcp_accept_nonblocking) { UNUSED; *r = loft_ffi_value_bool(n_tcp_accept_nonblocking(cx, I(0))); }
BRIDGE(tcp_method) { UNUSED; *r = loft_ffi_value_text(n_tcp_method(cx)); }
BRIDGE(tcp_path) { UNUSED; *r = loft_ffi_value_text(n_tcp_path(cx)); }
BRIDGE(tcp_body) { UNUSED; *r = loft_ffi_value_text(n_tcp_body(cx)); }
BRIDGE(tcp_headers) { UNUSED; *r = loft_ffi_value_text(n_tcp_headers(cx)); }
BRIDGE(tcp_respond) { UNUSED; n_tcp_respond(cx, I(0), T(1)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_respond_typed) { UNUSED; n_tcp_respond_typed(cx, I(0), T(1), T(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_respond_bytes) { UNUSED; n_tcp_respond_bytes(cx, s, I(0), a[1].payload.r, T(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_respond_full) { UNUSED; n_tcp_respond_full(cx, I(0), T(1), T(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_slice) { UNUSED; *r = loft_ffi_value_text(n_tcp_slice(cx, T(0), I(1), I(2))); }
BRIDGE(tcp_close) { UNUSED; n_tcp_close(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_upgrade) { UNUSED; *r = loft_ffi_value_i64(n_ws_upgrade(cx)); }
BRIDGE(ws_recv) { UNUSED; *r = loft_ffi_value_bool(n_ws_recv(cx, I(0))); }
BRIDGE(ws_message) { UNUSED; *r = loft_ffi_value_text(n_ws_message(cx)); }
BRIDGE(ws_opcode) { UNUSED; *r = loft_ffi_value_i64(n_ws_opcode(cx)); }
BRIDGE(ws_send) { UNUSED; *r = loft_ffi_value_bool(n_ws_send(cx, I(0), T(1))); }
BRIDGE(ws_send_binary) { UNUSED; *r = loft_ffi_value_bool(n_ws_send_binary(cx, I(0), T(1))); }
BRIDGE(ws_close) { UNUSED; n_ws_close(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_accept_nonblocking) { UNUSED; *r = loft_ffi_value_i64(n_ws_accept_nonblocking(cx, I(0))); }
BRIDGE(ws_clients_len) { UNUSED; *r = loft_ffi_value_i64(n_ws_clients_len(cx)); }
BRIDGE(ws_client_active) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_active(cx, I(0))); }
BRIDGE(ws_allow_origins) { UNUSED; n_ws_allow_origins(cx, I(0), T(1)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_next_event) { UNUSED; *r = loft_ffi_value_bool(n_ws_next_event(cx, I(0))); }
BRIDGE(ws_event_kind) { UNUSED; *r = loft_ffi_value_i64(n_ws_event_kind(cx)); }
BRIDGE(ws_event_client_id) { UNUSED; *r = loft_ffi_value_i64(n_ws_event_client_id(cx)); }
BRIDGE(ws_event_payload) { UNUSED; *r = loft_ffi_value_text(n_ws_event_payload(cx)); }
BRIDGE(ws_broadcast) { UNUSED; *r = loft_ffi_value_i64(n_ws_broadcast(cx, I(0), T(1))); }
BRIDGE(ws_idle_sleep_ms) { UNUSED; n_ws_idle_sleep_ms(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ssh_connect) { UNUSED; *r = loft_ffi_value_i64(n_ssh_connect(cx, T(0), I(1))); }
BRIDGE(ssh_host_key) { UNUSED; *r = loft_ffi_value_text(n_ssh_host_key(cx, I(0))); }
BRIDGE(ssh_login) { UNUSED; *r = loft_ffi_value_bool(n_ssh_login(cx, I(0), T(1), T(2))); }
BRIDGE(ssh_open_shell) { UNUSED; *r = loft_ffi_value_bool(n_ssh_open_shell(cx, I(0), T(1), I(2), I(3))); }
BRIDGE(ssh_send) { UNUSED; n_ssh_send(cx, I(0), T(1)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ssh_recv) { UNUSED; *r = loft_ffi_value_text(n_ssh_recv(cx, I(0))); }
BRIDGE(ssh_resize) { UNUSED; n_ssh_resize(cx, I(0), I(1), I(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ssh_is_open) { UNUSED; *r = loft_ffi_value_bool(n_ssh_is_open(cx, I(0))); }
BRIDGE(ssh_close) { UNUSED; n_ssh_close(cx, I(0)); r->tag = LOFT_FFI_TAG_VOID; }

#define NATIVES(X) X(http_do) X(http_body) X(http_headers) X(http_head_size) X(ws_connect) X(ws_client_send) \
    X(ws_client_send_binary) X(ws_client_recv) X(ws_client_message) X(ws_client_opcode) X(ws_client_close) \
    X(sleep_ms) X(pack_reset) X(pack_u8) X(pack_u16_le) X(pack_u32_le) X(pack_take) X(byte_at) X(ws_group_clear) \
    X(ws_group_add) X(ws_group_poll) X(frame_yield) \
    X(tcp_listen) X(tcp_listen_on) X(tcp_listen_tls) X(tcp_listen_tls_on) X(tcp_accept) X(tcp_accept_nonblocking) \
    X(tcp_method) X(tcp_path) X(tcp_body) X(tcp_headers) X(tcp_respond) X(tcp_respond_typed) X(tcp_respond_bytes) \
    X(tcp_respond_full) X(tcp_slice) X(tcp_close) X(ws_upgrade) X(ws_recv) X(ws_message) X(ws_opcode) X(ws_send) \
    X(ws_send_binary) X(ws_close) X(ws_accept_nonblocking) X(ws_clients_len) X(ws_client_active) X(ws_allow_origins) X(ws_next_event) \
    X(ws_event_kind) X(ws_event_client_id) X(ws_event_payload) X(ws_broadcast) X(ws_idle_sleep_ms) \
    X(ssh_connect) X(ssh_host_key) X(ssh_login) X(ssh_open_shell) X(ssh_send) X(ssh_recv) X(ssh_resize) X(ssh_is_open) X(ssh_close)

#define REG_TYPED(name) cb((const uint8_t *) "n_" #name, sizeof("n_" #name) - 1, (const void *) n_##name, ctx);
#define REG_BRIDGE(name) cb((const uint8_t *) "n_" #name, sizeof("n_" #name) - 1, (const void *) b_##name, ctx);

NET_EXPORT void loft_register_v1(loft_ffi_register_cb cb, void *ctx) { NATIVES(REG_TYPED) }
/* The bridges, each taking the caller's component context first (`loft_cx_make_v1`). */
NET_EXPORT void loft_register_bridges_cx_v1(loft_ffi_register_cb cb, void *ctx) { NATIVES(REG_BRIDGE) }
