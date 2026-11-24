// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I85 — Engine-host kernel natives (the web library's byte helpers, every target)

//! The web library's binary helpers, the same on every target: the pack builder (`pack_reset`,
//! `pack_u8`, `pack_u16_le`, `pack_u32_le`, `pack_take`), `byte_at`, and bytes turned into the
//! `text` that carries them.  The browser page's natives (`web_browser`) and the in-browser
//! interpreter's (`native::WEB_FUNCTIONS_WASM`) call these; a host's are the `net` component's
//! C (`src/c/loft_web.h`).
//!
//! A packed buffer, a response body and a binary frame are BYTES carried in a loft `text` — the
//! library reads them back with `byte_at` and never as characters — so [`bytes_text`] keeps a
//! byte that is not UTF-8, as the library's protocol requires.

/// `@PLN226` — a program's web buffers (`Stores::web`, the C runtime's `cx->pk_*`): the bytes
/// the `pack_*` natives build, and in the browser the last WebSocket message, its opcode and
/// the last fetch's body and headers (`web_browser`).  The program's own (`@FR-C-Own`).
#[derive(Default)]
pub struct WebCx {
    pack: Vec<u8>,
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    pub(crate) last_msg: Vec<u8>,
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    pub(crate) last_op: i64,
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    pub(crate) last_body: Vec<u8>,
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    pub(crate) last_headers: Vec<u8>,
}

impl Clone for WebCx {
    /// A new context (a worker's `Stores`) starts empty.
    fn clone(&self) -> Self {
        WebCx::default()
    }
}

/// Bytes as the `text` the library reads them from: unchanged, a non-UTF-8 byte included.
#[must_use]
pub fn bytes_text(b: Vec<u8>) -> String {
    match String::from_utf8(b) {
        Ok(s) => s,
        // SAFETY: the library treats these texts as byte buffers (`byte_at`, `send_binary`)
        // and never as characters; the crate this replaced kept the same contract.
        Err(e) => unsafe { String::from_utf8_unchecked(e.into_bytes()) },
    }
}

/// Empty the pack buffer.
pub fn pack_reset(web: &mut WebCx) {
    web.pack.clear();
}

/// One byte, the value masked to 8 bits.
pub fn pack_u8(web: &mut WebCx, b: i64) {
    web.pack.push((b & 0xff) as u8);
}

/// Two bytes, little-endian.
pub fn pack_u16_le(web: &mut WebCx, v: i64) {
    web.pack
        .extend_from_slice(&((v & 0xffff) as u16).to_le_bytes());
}

/// Four bytes, little-endian; a negative value packs its two's complement.
pub fn pack_u32_le(web: &mut WebCx, v: i64) {
    web.pack
        .extend_from_slice(&((v & 0xffff_ffff) as u32).to_le_bytes());
}

/// The buffer as a `text`, leaving it empty.
#[must_use]
pub fn pack_take(web: &mut WebCx) -> String {
    bytes_text(std::mem::take(&mut web.pack))
}

/// The byte at `idx`, or -1 outside the text.
#[must_use]
pub fn byte_at(idx: i64, t: &str) -> i64 {
    usize::try_from(idx)
        .ok()
        .and_then(|i| t.as_bytes().get(i))
        .map_or(-1, |b| i64::from(*b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_byte_outside_the_text_is_minus_one() {
        assert_eq!(byte_at(0, "A"), 65);
        assert_eq!(byte_at(1, "A"), -1);
        assert_eq!(byte_at(-1, "A"), -1);
    }

    #[test]
    fn a_packed_buffer_keeps_its_bytes() {
        let mut web = WebCx::default();
        pack_reset(&mut web);
        pack_u8(&mut web, 0);
        pack_u8(&mut web, 0x1ff);
        pack_u16_le(&mut web, 0x0102);
        pack_u32_le(&mut web, -1);
        let t = pack_take(&mut web);
        assert_eq!(t.as_bytes(), &[0, 0xff, 2, 1, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(byte_at(1, &t), 255);
        assert!(pack_take(&mut web).is_empty(), "taking empties the buffer");
    }
}
