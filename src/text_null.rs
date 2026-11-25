// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! The null text: the one home for how Rust code spells an absent `text` and asks whether a
//! text is absent (`@FR-L-Null-Text`, `@C171`).
//!
//! Every place that MAKES a null text takes it from [`null_str`] (or [`Str::null`] for a view),
//! and every place that ASKS goes through [`is_null`] (or [`Str::is_null`]), so how absence is
//! spelled is decided here and nowhere else.  Absence is the one value no byte string can
//! produce, and a byte string may hold NUL bytes anywhere, so it is an IDENTITY, never a
//! content: the null text is whatever points at [`NULL_TEXT`] — the zero-length `&str` there
//! ([`null_str`]), and the owned `String` of capacity 0 there ([`null_string`]).  A view
//! (`Str`) spells it as a null pointer, the C runtime's spelling, and converts at
//! `Str::new` / `Str::str`.  A text of one NUL byte is an ordinary text.
//!
//! [`Str::null`]: crate::keys::Str::null
//! [`Str::is_null`]: crate::keys::Str::is_null

/// The storage the null text points into.  A `static`, not a `const`: a `const` has no fixed
/// address, and the null text is told apart by where it points.
pub static NULL_TEXT: [u8; 1] = [0];

/// The null text, as a borrowed `&str`: the zero-length slice at [`NULL_TEXT`].
#[inline]
#[must_use]
pub fn null_str() -> &'static str {
    // SAFETY: an empty slice of a live static is valid UTF-8.
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(NULL_TEXT.as_ptr(), 0)) }
}

/// The null text as an owned `String`: capacity 0 at [`NULL_TEXT`].  It is never freed — a
/// `String` of capacity 0 owns no allocation — and a push gives it a buffer of its own.
#[inline]
#[must_use]
#[expect(
    clippy::same_length_and_capacity,
    reason = "the null text is the empty String AT `NULL_TEXT`: its address is its identity, which a copy would lose"
)]
pub fn null_string() -> String {
    // SAFETY: with capacity 0 the pointer needs only be non-null and aligned
    // (`Vec::from_raw_parts`), and nothing is ever written through it.
    #[expect(
        clippy::same_length_and_capacity,
        reason = "the null text is its ADDRESS: an empty String built any other way points elsewhere"
    )]
    unsafe {
        String::from_raw_parts(NULL_TEXT.as_ptr().cast_mut(), 0, 0)
    }
}

/// Make `s` the null text.
#[inline]
pub fn set_null(s: &mut String) {
    *s = null_string();
}

/// Make `s` the empty text `""`, a null one too: the start of every assignment to a text.
#[inline]
pub fn clear(s: &mut String) {
    if is_null(s) {
        *s = String::new();
    } else {
        s.clear();
    }
}

/// Write `s` into a native's fresh destination `dst`: its bytes, or the null text when `s` is
/// null — a push of the null text's no bytes would leave `""`.
#[inline]
pub fn put(dst: &mut String, s: &str) {
    if is_null(s) {
        set_null(dst);
    } else {
        dst.push_str(s);
    }
}

/// An owned copy of `s` that keeps a null text null.
#[inline]
#[must_use]
pub fn own(s: &str) -> String {
    if is_null(s) {
        null_string()
    } else {
        s.to_owned()
    }
}

/// Is `s` the null text?
#[inline]
#[must_use]
pub fn is_null(s: &str) -> bool {
    std::ptr::eq(s.as_ptr(), NULL_TEXT.as_ptr())
}

/// The bytes a text key's digest is taken over (`@FR-S-Digest`): a null key digests as one
/// NUL byte, which is what every stored digest of a null key was taken over, so no hash
/// table changes when the spelling of absence does.  A present text digests as itself; a
/// present text of one NUL byte shares the null's bucket, and key equality tells them apart.
#[inline]
#[must_use]
pub fn digest_text(s: &str) -> &str {
    if is_null(s) { "\0" } else { s }
}

/// Is `b` the null text's bytes?  For a reader that holds a text as bytes (a trie or paged key).
#[inline]
#[must_use]
pub fn is_null_bytes(b: &[u8]) -> bool {
    std::ptr::eq(b.as_ptr(), NULL_TEXT.as_ptr())
}
