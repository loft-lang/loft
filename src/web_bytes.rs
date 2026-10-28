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

use std::cell::RefCell;

thread_local! {
    static PACK_BUF: RefCell<Vec<u8>> = const { RefCell::new(Vec::new()) };
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

fn push(bytes: &[u8]) {
    PACK_BUF.with(|b| b.borrow_mut().extend_from_slice(bytes));
}

pub fn pack_reset() {
    PACK_BUF.with(|b| b.borrow_mut().clear());
}

/// One byte, the value masked to 8 bits.
pub fn pack_u8(b: i64) {
    push(&[(b & 0xff) as u8]);
}

/// Two bytes, little-endian.
pub fn pack_u16_le(v: i64) {
    push(&((v & 0xffff) as u16).to_le_bytes());
}

/// Four bytes, little-endian; a negative value packs its two's complement.
pub fn pack_u32_le(v: i64) {
    push(&((v & 0xffff_ffff) as u32).to_le_bytes());
}

/// The buffer as a `text`, leaving it empty.
#[must_use]
pub fn pack_take() -> String {
    bytes_text(PACK_BUF.with(|b| std::mem::take(&mut *b.borrow_mut())))
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
        pack_reset();
        pack_u8(0);
        pack_u8(0x1ff);
        pack_u16_le(0x0102);
        pack_u32_le(-1);
        let t = pack_take();
        assert_eq!(t.as_bytes(), &[0, 0xff, 2, 1, 0xff, 0xff, 0xff, 0xff]);
        assert_eq!(byte_at(1, &t), 255);
        assert!(pack_take().is_empty(), "taking empties the buffer");
    }
}
