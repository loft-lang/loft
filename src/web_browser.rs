// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I85 — Engine-host kernel natives (the web library's natives in a browser page)

//! The web library's natives in a page `loft --html` builds: the typed twins `codegen_runtime`
//! re-exports for the browser target, over the `loft_web` host imports `doc/loft-web.js`
//! provides (a `WebSocket` per handle with its inbound queue, `fetch()`).  A host's are the `net`
//! component (`src/c/components/net.c`); the byte helpers are `web_bytes`.
//!
//! `fetch()` and `WebSocket.onmessage` are asynchronous, so a synchronous loft call waits through
//! `ws_yield`, the asyncify suspend the page's `wasm-opt --asyncify` pass instruments: it returns
//! to the event loop for a frame and resumes.  Each import re-reads wasm memory on the JS side,
//! since memory can grow between two calls.

use crate::database::Stores;
use std::cell::UnsafeCell;

#[link(wasm_import_module = "loft_web")]
unsafe extern "C" {
    fn ws_connect(url_ptr: *const u8, url_len: usize) -> i32;
    fn ws_send(h: i32, ptr: *const u8, len: usize) -> i32;
    fn ws_send_binary(h: i32, ptr: *const u8, len: usize) -> i32;
    /// Latch the next inbound frame: 1 when one is now current.
    fn ws_poll(h: i32) -> i32;
    fn ws_msg_len(h: i32) -> i32;
    fn ws_msg_copy(h: i32, ptr: *mut u8);
    fn ws_opcode(h: i32) -> i32;
    fn ws_close(h: i32);
    /// The asyncify suspend: one frame of the JS event loop.
    fn ws_yield();
    #[allow(clippy::too_many_arguments)]
    fn http_start(
        method_ptr: *const u8,
        method_len: usize,
        url_ptr: *const u8,
        url_len: usize,
        body_ptr: *const u8,
        body_len: usize,
        headers_ptr: *const u8,
        headers_len: usize,
    ) -> i32;
    /// 1 done, 0 pending, -1 a transport failure.
    fn http_poll(h: i32) -> i32;
    fn http_status(h: i32) -> i32;
    fn http_body_len(h: i32) -> i32;
    fn http_body_copy(h: i32, ptr: *mut u8);
    fn http_headers_len(h: i32) -> i32;
    fn http_headers_copy(h: i32, ptr: *mut u8);
    fn http_free(h: i32);
}

/// The calling program's web buffers (`Stores::web`).
#[expect(
    clippy::mut_from_ref,
    reason = "the stores sit in an `UnsafeCell`, the writer's calling convention"
)]
fn web(cell: &UnsafeCell<Stores>) -> &mut crate::web_bytes::WebCx {
    // SAFETY: the cell is the calling program's stores, which no other reference reaches
    // while one of its natives runs.
    unsafe { &mut (*cell.get()).web }
}

fn handle(h: i64) -> i32 {
    i32::try_from(h).unwrap_or(-1)
}

/// `n` bytes the host copies into a fresh buffer.
fn copied(n: i32, copy: impl FnOnce(*mut u8)) -> Vec<u8> {
    let mut v = vec![0u8; usize::try_from(n).unwrap_or(0)];
    if !v.is_empty() {
        copy(v.as_mut_ptr());
    }
    v
}

/// A fetch run to completion — the page yields to its event loop until the answer is in — and
/// its body and headers kept; the status, or 0 on a transport failure.
fn http_run(
    web: &mut crate::web_bytes::WebCx,
    method: &str,
    url: &str,
    body: &str,
    headers: &str,
) -> i64 {
    let h = unsafe {
        http_start(
            method.as_ptr(),
            method.len(),
            url.as_ptr(),
            url.len(),
            body.as_ptr(),
            body.len(),
            headers.as_ptr(),
            headers.len(),
        )
    };
    if h < 0 {
        return 0;
    }
    loop {
        match unsafe { http_poll(h) } {
            1 => break,
            p if p < 0 => {
                unsafe { http_free(h) };
                return 0;
            }
            _ => unsafe { ws_yield() },
        }
    }
    let status = i64::from(unsafe { http_status(h) });
    let body = copied(unsafe { http_body_len(h) }, |p| unsafe {
        http_body_copy(h, p)
    });
    let heads = copied(unsafe { http_headers_len(h) }, |p| unsafe {
        http_headers_copy(h, p)
    });
    unsafe { http_free(h) };
    web.last_body = body;
    web.last_headers = heads;
    status
}

/// The browser's twins, named as the library declares its natives.
pub mod typed {
    use super::{Stores, UnsafeCell, copied, handle, http_run, web};
    use crate::web_bytes;

    pub fn n_http_do(
        cell: &UnsafeCell<Stores>,
        method: &str,
        url: &str,
        body: &str,
        headers: &str,
    ) -> i64 {
        http_run(web(cell), method, url, body, headers)
    }
    pub fn n_http_body(cell: &UnsafeCell<Stores>) -> String {
        web_bytes::bytes_text(web(cell).last_body.clone())
    }
    pub fn n_http_headers(cell: &UnsafeCell<Stores>) -> String {
        String::from_utf8_lossy(&web(cell).last_headers).into_owned()
    }
    /// A `HEAD`'s `Content-Length`, or -1 — also when the page's CORS hides the header; the
    /// library's `http_size` then falls back to a range read.
    pub fn n_http_head_size(cell: &UnsafeCell<Stores>, url: &str) -> i64 {
        let web = web(cell);
        let status = http_run(web, "HEAD", url, "", "");
        if !(200..400).contains(&status) {
            return -1;
        }
        {
            String::from_utf8_lossy(&web.last_headers)
                .split('\n')
                .find_map(|l| {
                    let (k, v) = l.split_once(':')?;
                    k.trim()
                        .eq_ignore_ascii_case("content-length")
                        .then(|| v.trim().parse().ok())?
                })
                .unwrap_or(-1)
        }
    }
    pub fn n_ws_connect(_cell: &UnsafeCell<Stores>, url: &str) -> i64 {
        i64::from(unsafe { super::ws_connect(url.as_ptr(), url.len()) })
    }
    pub fn n_ws_client_send(_cell: &UnsafeCell<Stores>, h: i64, msg: &str) -> u8 {
        u8::from(unsafe { super::ws_send(handle(h), msg.as_ptr(), msg.len()) } == 1)
    }
    pub fn n_ws_client_send_binary(_cell: &UnsafeCell<Stores>, h: i64, msg: &str) -> u8 {
        u8::from(unsafe { super::ws_send_binary(handle(h), msg.as_ptr(), msg.len()) } == 1)
    }
    pub fn n_ws_client_recv(cell: &UnsafeCell<Stores>, h: i64) -> u8 {
        let h = handle(h);
        if unsafe { super::ws_poll(h) } != 1 {
            return 0;
        }
        let bytes = copied(unsafe { super::ws_msg_len(h) }, |p| unsafe {
            super::ws_msg_copy(h, p)
        });
        let op = i64::from(unsafe { super::ws_opcode(h) });
        let web = web(cell);
        web.last_msg = bytes;
        web.last_op = op;
        1
    }
    pub fn n_ws_client_message(cell: &UnsafeCell<Stores>) -> String {
        web_bytes::bytes_text(web(cell).last_msg.clone())
    }
    pub fn n_ws_client_opcode(cell: &UnsafeCell<Stores>) -> i64 {
        web(cell).last_op
    }
    pub fn n_ws_client_close(_cell: &UnsafeCell<Stores>, h: i64) {
        unsafe { super::ws_close(handle(h)) };
    }
    /// A no-op: the page's event loop paces it; a program paces with `frame_yield` there.
    pub fn n_sleep_ms(_cell: &UnsafeCell<Stores>, _ms: i64) {}
    pub fn n_frame_yield(_cell: &UnsafeCell<Stores>) {
        unsafe { super::ws_yield() };
    }
    pub fn n_pack_reset(cell: &UnsafeCell<Stores>) {
        web_bytes::pack_reset(web(cell));
    }
    pub fn n_pack_u8(cell: &UnsafeCell<Stores>, b: i64) {
        web_bytes::pack_u8(web(cell), b);
    }
    pub fn n_pack_u16_le(cell: &UnsafeCell<Stores>, v: i64) {
        web_bytes::pack_u16_le(web(cell), v);
    }
    pub fn n_pack_u32_le(cell: &UnsafeCell<Stores>, v: i64) {
        web_bytes::pack_u32_le(web(cell), v);
    }
    pub fn n_pack_take(cell: &UnsafeCell<Stores>) -> String {
        web_bytes::pack_take(web(cell))
    }
    pub fn n_byte_at(_cell: &UnsafeCell<Stores>, idx: i64, t: &str) -> i64 {
        web_bytes::byte_at(idx, t)
    }
    /// A page rarely multiplexes sockets (the host's sync agent does), so the group is the
    /// bridge's stub: nothing is ever ready.
    pub fn n_ws_group_clear(_cell: &UnsafeCell<Stores>) {}
    pub fn n_ws_group_add(_cell: &UnsafeCell<Stores>, _h: i64) {}
    pub fn n_ws_group_poll(_cell: &UnsafeCell<Stores>) -> i64 {
        -1
    }
}
