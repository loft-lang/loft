// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The null text: the one home for how Rust code spells an absent `text` and asks whether a
//! text is absent (`@FR-L-Null-Text`).
//!
//! Every place that MAKES a null text takes it from [`null_str`] (or [`Str::null`] for a view),
//! and every place that ASKS goes through [`is_null`] (or [`Str::is_null`]), so how absence is
//! spelled is decided here and nowhere else.  Absence is the one value no byte string can
//! produce, and a byte string may hold NUL bytes anywhere, so a reader must never decide it
//! from a text's bytes of its own accord.
//!
//! [`Str::null`]: crate::keys::Str::null
//! [`Str::is_null`]: crate::keys::Str::is_null

/// The storage the null text points into.  A `static`, not a `const`: a `const` has no fixed
/// address, and the null text is told apart by where it points.
pub static NULL_TEXT: [u8; 1] = [0];

/// The null text, as a borrowed `&str`.
#[inline]
#[must_use]
pub fn null_str() -> &'static str {
    // SAFETY: one NUL byte is valid UTF-8.
    unsafe { std::str::from_utf8_unchecked(&NULL_TEXT) }
}

/// Is `s` the null text?
#[inline]
#[must_use]
pub fn is_null(s: &str) -> bool {
    s.as_bytes() == NULL_TEXT
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
    b == NULL_TEXT
}
