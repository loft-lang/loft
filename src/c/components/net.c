/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * libloft_net — the component behind the web, server and ssh libraries (@PLN205 L1.2, L1.3): their
 * `#native` functions as the loft-ffi ABI's typed symbols and bridges
 * (`loft-ffi/include/loft_ffi.h`), over the C runtime's own HTTP client, TLS and WebSocket code
 * (`loft_https.h`, which a C unit also pastes; `loft_web.h`, `loft_server.h`, `loft_ssh.h`,
 * `tls/loft_tls.c`, `tls/loft_ssh.c`).  Built ahead of time by `loft-components`
 * (`src/c/component.rs`), never where it is used. */
#include "loft_cc.h"
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

/* A text answer stays the library's until its next text answer on the thread (the ABI's
 * promise); loft copies it at once. */
static LOFT_TLS loft_text net_answer;

static loft_ffi_str net_text(loft_text t) {
    loft_text_free(&net_answer);
    net_answer = t;
    loft_ffi_str s = { (const uint8_t *) (t.p ? t.p : ""), t.len };
    return s;
}

static loft_str net_arg(const uint8_t *p, size_t n) { return loft_lit((const char *) (p ? p : (const uint8_t *) ""), n); }

/* ── Typed symbols: what compiled code calls ── */

NET_EXPORT int64_t n_http_do(const uint8_t *m, size_t mn, const uint8_t *u, size_t un, const uint8_t *b, size_t bn,
                             const uint8_t *h, size_t hn) {
    return loft_web_http_do(net_arg(m, mn), net_arg(u, un), net_arg(b, bn), net_arg(h, hn));
}
NET_EXPORT loft_ffi_str n_http_body(void) { return net_text(loft_web_http_body()); }
NET_EXPORT loft_ffi_str n_http_headers(void) { return net_text(loft_web_http_headers()); }
NET_EXPORT int64_t n_http_head_size(const uint8_t *u, size_t un) { return loft_web_http_head_size(net_arg(u, un)); }
NET_EXPORT int64_t n_ws_connect(const uint8_t *u, size_t un) { return loft_web_ws_connect(net_arg(u, un)); }
NET_EXPORT uint8_t n_ws_client_send(int64_t h, const uint8_t *m, size_t mn) { return loft_web_ws_client_send(h, net_arg(m, mn)); }
NET_EXPORT uint8_t n_ws_client_send_binary(int64_t h, const uint8_t *m, size_t mn) {
    return loft_web_ws_client_send_binary(h, net_arg(m, mn));
}
NET_EXPORT uint8_t n_ws_client_recv(int64_t h) { return loft_web_ws_client_recv(h); }
NET_EXPORT loft_ffi_str n_ws_client_message(void) { return net_text(loft_web_ws_client_message()); }
NET_EXPORT int64_t n_ws_client_opcode(void) { return loft_web_ws_client_opcode(); }
NET_EXPORT void n_ws_client_close(int64_t h) { loft_web_ws_client_close(h); }
NET_EXPORT void n_sleep_ms(int64_t ms) { loft_web_sleep_ms(ms); }
NET_EXPORT void n_pack_reset(void) { loft_web_pack_reset(); }
NET_EXPORT void n_pack_u8(int64_t b) { loft_web_pack_u8(b); }
NET_EXPORT void n_pack_u16_le(int64_t v) { loft_web_pack_u16_le(v); }
NET_EXPORT void n_pack_u32_le(int64_t v) { loft_web_pack_u32_le(v); }
NET_EXPORT loft_ffi_str n_pack_take(void) { return net_text(loft_web_pack_take()); }
NET_EXPORT int64_t n_byte_at(int64_t idx, const uint8_t *t, size_t tn) { return loft_web_byte_at(idx, net_arg(t, tn)); }
NET_EXPORT void n_ws_group_clear(void) { loft_web_ws_group_clear(); }
NET_EXPORT void n_ws_group_add(int64_t h) { loft_web_ws_group_add(h); }
NET_EXPORT int64_t n_ws_group_poll(void) { return loft_web_ws_group_poll(); }
NET_EXPORT void n_frame_yield(void) { loft_web_frame_yield(); }

/* The server library's (`loft_server.h`). */
NET_EXPORT int64_t n_tcp_listen(int64_t port) { return loft_srv_bind("", 0, port, 0); }
NET_EXPORT int64_t n_tcp_listen_on(const uint8_t *a, size_t an, int64_t port) {
    return loft_srv_bind((const char *) a, an, port, 0);
}
NET_EXPORT int64_t n_tcp_listen_tls(int64_t port, const uint8_t *c, size_t cn, const uint8_t *k, size_t kn) {
    return loft_srv_listen_tls("", 0, port, (const char *) c, cn, (const char *) k, kn);
}
NET_EXPORT int64_t n_tcp_listen_tls_on(const uint8_t *a, size_t an, int64_t port, const uint8_t *c, size_t cn,
                                       const uint8_t *k, size_t kn) {
    return loft_srv_listen_tls((const char *) a, an, port, (const char *) c, cn, (const char *) k, kn);
}
NET_EXPORT uint8_t n_tcp_accept(int64_t h) { return loft_srv_tcp_accept(h, 0); }
NET_EXPORT uint8_t n_tcp_accept_nonblocking(int64_t h) { return loft_srv_tcp_accept(h, 1); }
NET_EXPORT loft_ffi_str n_tcp_method(void) { return net_text(loft_srv_copy(&loft_srv_method)); }
NET_EXPORT loft_ffi_str n_tcp_path(void) { return net_text(loft_srv_copy(&loft_srv_path)); }
NET_EXPORT loft_ffi_str n_tcp_body(void) { return net_text(loft_srv_copy(&loft_srv_body)); }
NET_EXPORT loft_ffi_str n_tcp_headers(void) { return net_text(loft_srv_copy(&loft_srv_heads)); }
NET_EXPORT void n_tcp_respond(int64_t status, const uint8_t *b, size_t bn) {
    static const char ct[] = "text/plain; charset=utf-8";
    loft_srv_respond(status, ct, sizeof ct - 1, (const char *) b, bn);
}
NET_EXPORT void n_tcp_respond_typed(int64_t status, const uint8_t *b, size_t bn, const uint8_t *c, size_t cn) {
    static const char ct[] = "text/plain; charset=utf-8";
    loft_srv_respond(status, cn ? (const char *) c : ct, cn ? cn : sizeof ct - 1, (const char *) b, bn);
}
NET_EXPORT void n_tcp_respond_bytes(loft_ffi_store s, int64_t status, loft_ffi_ref body, const uint8_t *c, size_t cn) {
    static const char ct[] = "application/octet-stream";
    uint32_t n = body.rec ? loft_ffi_vector_len(s, body) : 0;
    const char *d = n ? (const char *) loft_ffi_vector_data(s, body) : "";
    loft_srv_respond(status, cn ? (const char *) c : ct, cn ? cn : sizeof ct - 1, d, n);
}
NET_EXPORT void n_tcp_respond_full(int64_t status, const uint8_t *b, size_t bn, const uint8_t *h, size_t hn) {
    loft_srv_respond_full(status, net_arg(b, bn), net_arg(h, hn));
}
NET_EXPORT loft_ffi_str n_tcp_slice(const uint8_t *b, size_t bn, int64_t off, int64_t len) {
    return net_text(loft_srv_slice(net_arg(b, bn), off, len));
}
NET_EXPORT void n_tcp_close(int64_t h) { loft_srv_tcp_close(h); }
NET_EXPORT int64_t n_ws_upgrade(void) { return loft_srv_ws_upgrade(); }
NET_EXPORT uint8_t n_ws_recv(int64_t id) { return loft_srv_ws_recv(id); }
NET_EXPORT loft_ffi_str n_ws_message(void) { return net_text(loft_srv_copy(&loft_srv_msg)); }
NET_EXPORT int64_t n_ws_opcode(void) { return loft_srv_msg_op; }
NET_EXPORT uint8_t n_ws_send(int64_t id, const uint8_t *m, size_t mn) { return loft_srv_ws_send(id, 0x1, net_arg(m, mn)); }
NET_EXPORT uint8_t n_ws_send_binary(int64_t id, const uint8_t *m, size_t mn) {
    return loft_srv_ws_send(id, 0x2, net_arg(m, mn));
}
NET_EXPORT void n_ws_close(int64_t id) { loft_srv_ws_close(id); }
NET_EXPORT int64_t n_ws_accept_nonblocking(int64_t h) { return loft_srv_ws_accept_nonblocking(h); }
NET_EXPORT int64_t n_ws_clients_len(void) { return loft_srv_clients_len(); }
NET_EXPORT uint8_t n_ws_client_active(int64_t id) { return loft_srv_client_active(id); }
NET_EXPORT uint8_t n_ws_next_event(int64_t h) { return loft_srv_ws_next_event(h); }
NET_EXPORT int64_t n_ws_event_kind(void) { return loft_srv_ev_kind; }
NET_EXPORT int64_t n_ws_event_client_id(void) { return loft_srv_ev_cid; }
NET_EXPORT loft_ffi_str n_ws_event_payload(void) { return net_text(loft_srv_copy(&loft_srv_ev_payload)); }
NET_EXPORT int64_t n_ws_broadcast(int64_t h, const uint8_t *m, size_t mn) {
    (void) h;
    return loft_srv_ws_broadcast(net_arg(m, mn));
}
NET_EXPORT void n_ws_idle_sleep_ms(int64_t ms) { loft_srv_sleep_ms(ms); }
NET_EXPORT int64_t n_ssh_connect(const uint8_t *h, size_t hn, int64_t port) { return loft_ssh_connect(net_arg(h, hn), port); }
NET_EXPORT uint8_t n_ssh_login(int64_t h, const uint8_t *u, size_t un, const uint8_t *p, size_t pn) {
    return loft_ssh_login_h(h, net_arg(u, un), net_arg(p, pn));
}
NET_EXPORT uint8_t n_ssh_open_shell(int64_t h, const uint8_t *t, size_t tn, int64_t cols, int64_t rows) {
    return loft_ssh_shell_h(h, net_arg(t, tn), cols, rows);
}
NET_EXPORT void n_ssh_send(int64_t h, const uint8_t *d, size_t dn) { loft_ssh_send_h(h, net_arg(d, dn)); }
NET_EXPORT loft_ffi_str n_ssh_recv(int64_t h) { return net_text(loft_ssh_recv_h(h)); }
NET_EXPORT void n_ssh_resize(int64_t h, int64_t cols, int64_t rows) { loft_ssh_resize_h(h, cols, rows); }
NET_EXPORT uint8_t n_ssh_is_open(int64_t h) { return loft_ssh_is_open_h(h); }
NET_EXPORT void n_ssh_close(int64_t h) { loft_ssh_close_h(h); }

/* ── Bridges: what the interpreter calls, one per symbol ── */

#define T(k) a[k].payload.text.ptr, a[k].payload.text.len
#define I(k) a[k].payload.i
#define BRIDGE(name) static void b_##name(loft_ffi_store s, const loft_ffi_value *a, size_t n, loft_ffi_value *r)
#define UNUSED (void) s; (void) a; (void) n
BRIDGE(http_do) { UNUSED; *r = loft_ffi_value_i64(n_http_do(T(0), T(1), T(2), T(3))); }
BRIDGE(http_body) { UNUSED; *r = loft_ffi_value_text(n_http_body()); }
BRIDGE(http_headers) { UNUSED; *r = loft_ffi_value_text(n_http_headers()); }
BRIDGE(http_head_size) { UNUSED; *r = loft_ffi_value_i64(n_http_head_size(T(0))); }
BRIDGE(ws_connect) { UNUSED; *r = loft_ffi_value_i64(n_ws_connect(T(0))); }
BRIDGE(ws_client_send) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_send(I(0), T(1))); }
BRIDGE(ws_client_send_binary) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_send_binary(I(0), T(1))); }
BRIDGE(ws_client_recv) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_recv(I(0))); }
BRIDGE(ws_client_message) { UNUSED; *r = loft_ffi_value_text(n_ws_client_message()); }
BRIDGE(ws_client_opcode) { UNUSED; *r = loft_ffi_value_i64(n_ws_client_opcode()); }
BRIDGE(ws_client_close) { UNUSED; n_ws_client_close(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(sleep_ms) { UNUSED; n_sleep_ms(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_reset) { UNUSED; n_pack_reset(); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_u8) { UNUSED; n_pack_u8(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_u16_le) { UNUSED; n_pack_u16_le(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_u32_le) { UNUSED; n_pack_u32_le(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(pack_take) { UNUSED; *r = loft_ffi_value_text(n_pack_take()); }
BRIDGE(byte_at) { UNUSED; *r = loft_ffi_value_i64(n_byte_at(I(0), T(1))); }
BRIDGE(ws_group_clear) { UNUSED; n_ws_group_clear(); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_group_add) { UNUSED; n_ws_group_add(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_group_poll) { UNUSED; *r = loft_ffi_value_i64(n_ws_group_poll()); }
BRIDGE(frame_yield) { UNUSED; n_frame_yield(); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_listen) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen(I(0))); }
BRIDGE(tcp_listen_on) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen_on(T(0), I(1))); }
BRIDGE(tcp_listen_tls) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen_tls(I(0), T(1), T(2))); }
BRIDGE(tcp_listen_tls_on) { UNUSED; *r = loft_ffi_value_i64(n_tcp_listen_tls_on(T(0), I(1), T(2), T(3))); }
BRIDGE(tcp_accept) { UNUSED; *r = loft_ffi_value_bool(n_tcp_accept(I(0))); }
BRIDGE(tcp_accept_nonblocking) { UNUSED; *r = loft_ffi_value_bool(n_tcp_accept_nonblocking(I(0))); }
BRIDGE(tcp_method) { UNUSED; *r = loft_ffi_value_text(n_tcp_method()); }
BRIDGE(tcp_path) { UNUSED; *r = loft_ffi_value_text(n_tcp_path()); }
BRIDGE(tcp_body) { UNUSED; *r = loft_ffi_value_text(n_tcp_body()); }
BRIDGE(tcp_headers) { UNUSED; *r = loft_ffi_value_text(n_tcp_headers()); }
BRIDGE(tcp_respond) { UNUSED; n_tcp_respond(I(0), T(1)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_respond_typed) { UNUSED; n_tcp_respond_typed(I(0), T(1), T(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_respond_bytes) { UNUSED; n_tcp_respond_bytes(s, I(0), a[1].payload.r, T(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_respond_full) { UNUSED; n_tcp_respond_full(I(0), T(1), T(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(tcp_slice) { UNUSED; *r = loft_ffi_value_text(n_tcp_slice(T(0), I(1), I(2))); }
BRIDGE(tcp_close) { UNUSED; n_tcp_close(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_upgrade) { UNUSED; *r = loft_ffi_value_i64(n_ws_upgrade()); }
BRIDGE(ws_recv) { UNUSED; *r = loft_ffi_value_bool(n_ws_recv(I(0))); }
BRIDGE(ws_message) { UNUSED; *r = loft_ffi_value_text(n_ws_message()); }
BRIDGE(ws_opcode) { UNUSED; *r = loft_ffi_value_i64(n_ws_opcode()); }
BRIDGE(ws_send) { UNUSED; *r = loft_ffi_value_bool(n_ws_send(I(0), T(1))); }
BRIDGE(ws_send_binary) { UNUSED; *r = loft_ffi_value_bool(n_ws_send_binary(I(0), T(1))); }
BRIDGE(ws_close) { UNUSED; n_ws_close(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ws_accept_nonblocking) { UNUSED; *r = loft_ffi_value_i64(n_ws_accept_nonblocking(I(0))); }
BRIDGE(ws_clients_len) { UNUSED; *r = loft_ffi_value_i64(n_ws_clients_len()); }
BRIDGE(ws_client_active) { UNUSED; *r = loft_ffi_value_bool(n_ws_client_active(I(0))); }
BRIDGE(ws_next_event) { UNUSED; *r = loft_ffi_value_bool(n_ws_next_event(I(0))); }
BRIDGE(ws_event_kind) { UNUSED; *r = loft_ffi_value_i64(n_ws_event_kind()); }
BRIDGE(ws_event_client_id) { UNUSED; *r = loft_ffi_value_i64(n_ws_event_client_id()); }
BRIDGE(ws_event_payload) { UNUSED; *r = loft_ffi_value_text(n_ws_event_payload()); }
BRIDGE(ws_broadcast) { UNUSED; *r = loft_ffi_value_i64(n_ws_broadcast(I(0), T(1))); }
BRIDGE(ws_idle_sleep_ms) { UNUSED; n_ws_idle_sleep_ms(I(0)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ssh_connect) { UNUSED; *r = loft_ffi_value_i64(n_ssh_connect(T(0), I(1))); }
BRIDGE(ssh_login) { UNUSED; *r = loft_ffi_value_bool(n_ssh_login(I(0), T(1), T(2))); }
BRIDGE(ssh_open_shell) { UNUSED; *r = loft_ffi_value_bool(n_ssh_open_shell(I(0), T(1), I(2), I(3))); }
BRIDGE(ssh_send) { UNUSED; n_ssh_send(I(0), T(1)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ssh_recv) { UNUSED; *r = loft_ffi_value_text(n_ssh_recv(I(0))); }
BRIDGE(ssh_resize) { UNUSED; n_ssh_resize(I(0), I(1), I(2)); r->tag = LOFT_FFI_TAG_VOID; }
BRIDGE(ssh_is_open) { UNUSED; *r = loft_ffi_value_bool(n_ssh_is_open(I(0))); }
BRIDGE(ssh_close) { UNUSED; n_ssh_close(I(0)); r->tag = LOFT_FFI_TAG_VOID; }

#define NATIVES(X) X(http_do) X(http_body) X(http_headers) X(http_head_size) X(ws_connect) X(ws_client_send) \
    X(ws_client_send_binary) X(ws_client_recv) X(ws_client_message) X(ws_client_opcode) X(ws_client_close) \
    X(sleep_ms) X(pack_reset) X(pack_u8) X(pack_u16_le) X(pack_u32_le) X(pack_take) X(byte_at) X(ws_group_clear) \
    X(ws_group_add) X(ws_group_poll) X(frame_yield) \
    X(tcp_listen) X(tcp_listen_on) X(tcp_listen_tls) X(tcp_listen_tls_on) X(tcp_accept) X(tcp_accept_nonblocking) \
    X(tcp_method) X(tcp_path) X(tcp_body) X(tcp_headers) X(tcp_respond) X(tcp_respond_typed) X(tcp_respond_bytes) \
    X(tcp_respond_full) X(tcp_slice) X(tcp_close) X(ws_upgrade) X(ws_recv) X(ws_message) X(ws_opcode) X(ws_send) \
    X(ws_send_binary) X(ws_close) X(ws_accept_nonblocking) X(ws_clients_len) X(ws_client_active) X(ws_next_event) \
    X(ws_event_kind) X(ws_event_client_id) X(ws_event_payload) X(ws_broadcast) X(ws_idle_sleep_ms) \
    X(ssh_connect) X(ssh_login) X(ssh_open_shell) X(ssh_send) X(ssh_recv) X(ssh_resize) X(ssh_is_open) X(ssh_close)

#define REG_TYPED(name) cb((const uint8_t *) "n_" #name, sizeof("n_" #name) - 1, (const void *) n_##name, ctx);
#define REG_BRIDGE(name) cb((const uint8_t *) "n_" #name, sizeof("n_" #name) - 1, (const void *) b_##name, ctx);

NET_EXPORT void loft_register_v1(loft_ffi_register_cb cb, void *ctx) { NATIVES(REG_TYPED) }
NET_EXPORT void loft_register_bridges_v1(loft_ffi_register_cb cb, void *ctx) { NATIVES(REG_BRIDGE) }
