// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! SipHash-1-3, owned by loft (loft#827).
//!
//! A persisted `hash<T[k]>` stores its seed in the file so that every reader re-derives
//! the same buckets. That makes the hash function part of the **store format**: a reader
//! that computes it differently looks in the wrong bucket and answers a miss, or a
//! neighbour, against bytes that are still perfectly valid. `store_verify` passes. Nothing
//! reports anything.
//!
//! Until now that function was `std::hash::DefaultHasher`, whose documentation says the
//! internal algorithm "is not specified, and so it and its hashes should not be relied
//! upon over releases" — and `Hasher`'s adds that the bytes std's types feed a hasher are
//! not stable between compiler versions either. So loft's on-disk format depended on an
//! implementation detail of whichever rustc built the binary. A toolchain upgrade could
//! move bucket placement with no loft change at all, which also means no layout identity
//! and no `crate::placement` token could refuse the older store: nothing loft knows about
//! would have changed.
//!
//! # This is a copy, not a replacement
//!
//! Every value below is what `DefaultHasher` computes today, so **the format does not
//! move**: no existing store is invalidated, and `placement::HASH` does not bump. What
//! changes is only who owns the definition. Byte-identity is not an aspiration here, it is
//! the whole point, and it is checked rather than argued —
//! `tests/siphash_std_parity.rs` runs this against `DefaultHasher` over a wide corpus, and
//! `tests/layout_golden.rs::placement_contract_is_pinned` pins the resulting digests.
//!
//! If a future rustc changes std's hasher, that parity test fails and **loft is
//! unaffected** — the correct response is to retire the test, not to follow std. That is
//! the inversion this module buys: what used to be silent data corruption is now a red
//! test on a machine, before anything ships.
//!
//! # What is reproduced
//!
//! SipHash-1-3 with `k0 = k1 = 0` (what `DefaultHasher::new()` fixes), plus the byte
//! stream std's `Hash` impls feed it for the two key kinds loft hashes:
//!
//! * an integer — `<i64 as Hash>::hash` calls `write_i64`, which SipHasher13 does NOT
//!   specialise (std's own comment: "no integer hashing methods … are defined for this
//!   type"), so it takes `Hasher::write_i64`'s default → `write_u64(i as u64)` →
//!   `write(&i.to_ne_bytes())`;
//! * a string — `<str as Hash>::hash` calls `write_str`, which SipHasher13 DOES
//!   specialise: the bytes, then a `0xFF` terminator, which cannot occur in UTF-8 and so
//!   makes the encoding prefix-free.
//!
//! Native-endian is deliberate: it is what std does, so copying it keeps the format
//! unchanged. The resulting host-endianness dependence is not new and is already covered —
//! the @PLN97 layout identity carries an `@endian` line, so a store from the other
//! endianness is refused rather than misread.
//!
//! Only the operations loft actually performs are exposed. This is not a general-purpose
//! `Hasher`, and it should not become one: every method here is a promise about bytes on
//! disk, and the fewer of those there are, the fewer can drift.

/// SipHash-1-3 keyed with `k0 = k1 = 0`.
///
/// The keys are folded into the initial state by the constructor and not retained: std
/// keeps them so `reset()` can re-key a reused hasher, and loft builds a fresh hasher per
/// hash instead.
#[derive(Debug, Clone)]
pub struct SipHasher13 {
    /// Bytes processed so far — only its low 8 bits reach the digest.
    length: usize,
    state: State,
    /// Bytes not yet part of a full 8-byte word, packed little-endian.
    tail: u64,
    /// How many bytes of `tail` are valid.
    ntail: usize,
}

#[derive(Debug, Clone, Copy)]
struct State {
    v0: u64,
    v1: u64,
    v2: u64,
    v3: u64,
}

macro_rules! compress {
    ($state:expr) => {{
        let s = &mut $state;
        s.v0 = s.v0.wrapping_add(s.v1);
        s.v2 = s.v2.wrapping_add(s.v3);
        s.v1 = s.v1.rotate_left(13);
        s.v1 ^= s.v0;
        s.v3 = s.v3.rotate_left(16);
        s.v3 ^= s.v2;
        s.v0 = s.v0.rotate_left(32);

        s.v2 = s.v2.wrapping_add(s.v1);
        s.v0 = s.v0.wrapping_add(s.v3);
        s.v1 = s.v1.rotate_left(17);
        s.v1 ^= s.v2;
        s.v3 = s.v3.rotate_left(21);
        s.v3 ^= s.v0;
        s.v2 = s.v2.rotate_left(32);
    }};
}

/// Read up to 7 bytes of `buf[start..start + len]` as a little-endian `u64`.
///
/// Safe indexing throughout — std reaches for `copy_nonoverlapping` here for speed, and
/// the byte-for-byte result is the same. This is a parse of at most 7 bytes; the hot path
/// is the 8-byte loop in [`SipHasher13::write`].
fn u8to64_le(buf: &[u8], start: usize, len: usize) -> u64 {
    debug_assert!(len < 8);
    let mut out: u64 = 0;
    for i in 0..len {
        out |= u64::from(buf[start + i]) << (i * 8);
    }
    out
}

impl SipHasher13 {
    /// A hasher with both keys zero — what `DefaultHasher::new()` fixes.
    #[must_use]
    pub const fn new() -> Self {
        Self::new_with_keys(0, 0)
    }

    /// A hasher keyed off `key0` / `key1`.
    ///
    /// loft always uses zero keys and carries its per-hash randomness in the SEED it
    /// writes into the message (see `keys::seeded_hasher`), because that seed has to
    /// travel with the store for a reader to re-derive the buckets. Keys would not.
    #[must_use]
    pub const fn new_with_keys(key0: u64, key1: u64) -> Self {
        Self {
            length: 0,
            state: State {
                v0: key0 ^ 0x736f_6d65_7073_6575,
                v1: key1 ^ 0x646f_7261_6e64_6f6d,
                v2: key0 ^ 0x6c79_6765_6e65_7261,
                v3: key1 ^ 0x7465_6462_7974_6573,
            },
            tail: 0,
            ntail: 0,
        }
    }

    /// Feed raw bytes.
    pub fn write(&mut self, msg: &[u8]) {
        let length = msg.len();
        self.length += length;

        let mut needed = 0;

        if self.ntail != 0 {
            needed = 8 - self.ntail;
            self.tail |= u8to64_le(msg, 0, std::cmp::min(length, needed)) << (8 * self.ntail);
            if length < needed {
                self.ntail += length;
                return;
            }
            self.state.v3 ^= self.tail;
            compress!(self.state);
            self.state.v0 ^= self.tail;
            self.ntail = 0;
        }

        let len = length - needed;
        let left = len & 0x7;

        let mut i = needed;
        while i < len - left {
            // `len - left` is the largest multiple of 8 at or below `len`, so eight bytes
            // are always in bounds here. Taking the chunk as an `Option` rather than
            // slicing keeps that fact from being a panic path if the arithmetic above is
            // ever edited; a `None` would mean this loop and `left` had gone out of step,
            // and `tests/siphash_std_parity.rs` is what would say so.
            let Some(chunk) = msg[i..].first_chunk::<8>() else {
                break;
            };
            let mi = u64::from_le_bytes(*chunk);
            self.state.v3 ^= mi;
            compress!(self.state);
            self.state.v0 ^= mi;
            i += 8;
        }

        self.tail = u8to64_le(msg, i, left);
        self.ntail = left;
    }

    /// Feed a `u64` exactly as `<u64 as Hash>::hash` does — the default
    /// `Hasher::write_u64`, since SipHasher13 specialises no integer method.
    ///
    /// A whole word arriving on a word boundary IS one compression round, which is all
    /// [`Self::write`] does with it after its tail bookkeeping — and that is the shape of
    /// every integer key: the seed, then the key, each eight bytes with nothing buffered.
    /// Taking it here keeps the round inline at the call (the general `write` is 62
    /// instructions for a word whose round is 14, and was a fifth of a cache-resident
    /// lookup).  The digest is the same by construction; `tests/siphash_std_parity.rs`
    /// crosses both arms (a word after a text leaves a tail and takes `write`).
    #[inline]
    pub fn write_u64(&mut self, i: u64) {
        if self.ntail == 0 {
            // `write` reads the word little-endian out of the native bytes.
            let mi = u64::from_le_bytes(i.to_ne_bytes());
            self.length += 8;
            self.state.v3 ^= mi;
            compress!(self.state);
            self.state.v0 ^= mi;
            return;
        }
        self.write(&i.to_ne_bytes());
    }

    /// Feed an `i64` exactly as `<i64 as Hash>::hash` does: `write_i64`'s default is
    /// `write_u64(i as u64)`.
    #[inline]
    pub fn write_i64(&mut self, i: i64) {
        self.write_u64(i as u64);
    }

    /// Feed a `&str` exactly as `<str as Hash>::hash` does: the bytes, then a `0xFF`
    /// terminator. `0xFF` cannot occur in UTF-8, so the encoding is prefix-free and
    /// `"ab" + "c"` cannot collide with `"a" + "bc"`.
    pub fn write_str(&mut self, s: &str) {
        self.write(s.as_bytes());
        self.write(&[0xFF]);
    }

    /// The 64-bit digest. Does not consume the hasher, matching `Hasher::finish`.
    #[inline]
    #[must_use]
    pub fn finish(&self) -> u64 {
        let mut state = self.state;
        let b: u64 = (((self.length as u64) & 0xff) << 56) | self.tail;

        state.v3 ^= b;
        compress!(state);
        state.v0 ^= b;

        state.v2 ^= 0xff;
        compress!(state);
        compress!(state);
        compress!(state);

        state.v0 ^ state.v1 ^ state.v2 ^ state.v3
    }
}

impl Default for SipHasher13 {
    fn default() -> Self {
        Self::new()
    }
}
