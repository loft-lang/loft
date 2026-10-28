/* Copyright (c) 2026 Jurjen Stellingwerff
 * SPDX-License-Identifier: LGPL-3.0-or-later
 *
 * libloft_net — the component behind the web library (@PLN205 L1.2): its `#native` functions as
 * the loft-ffi ABI's typed symbols and bridges (`loft-ffi/include/loft_ffi.h`), over the C
 * runtime's own HTTP client, TLS and WebSocket code (`loft_https.h`, `loft_web.h`,
 * `tls/loft_tls.c`), which a C unit also pastes.  Built ahead of time, never where it is used;
 * the build step and the switch of `web` onto it are the rest of @PLN205 L1.2 — until then
 * nothing compiles this file and the web natives stay in the binary. */
#include "loft_cc.h"
#include "loft_text.h"
#define LOFT_HTTPS 1
#define LOFT_WEB 1
#include "loft_https.h"
#include "loft_web.h"
#include "loft_ffi.h"

#define NET_EXPORT __attribute__((visibility("default")))

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

#define NATIVES(X) X(http_do) X(http_body) X(http_headers) X(http_head_size) X(ws_connect) X(ws_client_send) \
    X(ws_client_send_binary) X(ws_client_recv) X(ws_client_message) X(ws_client_opcode) X(ws_client_close) \
    X(sleep_ms) X(pack_reset) X(pack_u8) X(pack_u16_le) X(pack_u32_le) X(pack_take) X(byte_at) X(ws_group_clear) \
    X(ws_group_add) X(ws_group_poll) X(frame_yield)

#define REG_TYPED(name) cb((const uint8_t *) "n_" #name, sizeof("n_" #name) - 1, (const void *) n_##name, ctx);
#define REG_BRIDGE(name) cb((const uint8_t *) "n_" #name, sizeof("n_" #name) - 1, (const void *) b_##name, ctx);

NET_EXPORT void loft_register_v1(loft_ffi_register_cb cb, void *ctx) { NATIVES(REG_TYPED) }
NET_EXPORT void loft_register_bridges_v1(loft_ffi_register_cb cb, void *ctx) { NATIVES(REG_BRIDGE) }
