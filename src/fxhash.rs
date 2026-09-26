// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! A fast, non-cryptographic hasher for the compiler's own tables.
//!
//! `std`'s `HashMap` hashes with SipHash-1-3, a keyed hash built to resist collision
//! attacks on tables fed by an adversary.  The front end's tables are fed by the program
//! being compiled — its definition names, its variable numbers — and pay SipHash's per-key
//! setup on every lookup: over a compile of the 12 826-line front-end corpus the definition
//! index alone hashed 3 M keys for 13 % of all instructions (callgrind, @PLN166 B4).  This
//! is the multiply-rotate hash rustc itself uses for the same tables (`FxHasher`, the
//! `rustc-hash` crate's scheme, kept in-tree rather than as a dependency): a handful of
//! instructions per word, no setup, and iteration order that is a function of the keys
//! alone rather than of a per-process random seed.
//!
//! Not for anything an outside party can feed: a keyed collection a program builds from
//! network or file input keeps the store's own hashing (`hash.rs`), which is not this.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasherDefault, Hasher};

/// The `rustc-hash` 2.x scheme: add, multiply by an odd constant, and rotate on `finish` so
/// the well-mixed high bits reach the low bits a table indexes by.
#[derive(Default, Clone, Copy)]
pub struct FxHasher {
    hash: u64,
}

const K: u64 = 0xf135_7aea_2e62_a9c5;

impl FxHasher {
    #[inline]
    fn add_to_hash(&mut self, i: u64) {
        self.hash = self.hash.wrapping_add(i).wrapping_mul(K);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let mut b = bytes;
        while b.len() >= 8 {
            self.add_to_hash(u64::from_le_bytes(b[..8].try_into().expect("8 bytes")));
            b = &b[8..];
        }
        if b.len() >= 4 {
            self.add_to_hash(u64::from(u32::from_le_bytes(
                b[..4].try_into().expect("4 bytes"),
            )));
            b = &b[4..];
        }
        if b.len() >= 2 {
            self.add_to_hash(u64::from(u16::from_le_bytes(
                b[..2].try_into().expect("2 bytes"),
            )));
            b = &b[2..];
        }
        if let Some(&x) = b.first() {
            self.add_to_hash(u64::from(x));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add_to_hash(u64::from(i));
    }
    #[inline]
    fn write_u16(&mut self, i: u16) {
        self.add_to_hash(u64::from(i));
    }
    #[inline]
    fn write_u32(&mut self, i: u32) {
        self.add_to_hash(u64::from(i));
    }
    #[inline]
    fn write_u64(&mut self, i: u64) {
        self.add_to_hash(i);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add_to_hash(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        // `rustc-hash` answers the raw product rotated; measured on sequential keys that
        // leaves half the low-bit values a table indexes by unused (1 998 of 4 096 for
        // 0..4096, where a random hash reaches ~2 590), so one more multiply-xorshift round
        // finishes the mix.  A few instructions per lookup against SipHash's dozens.
        let h = self.hash;
        let h = (h ^ (h >> 32)).wrapping_mul(0x9e37_79b9_7f4a_7c15);
        h ^ (h >> 29)
    }
}

/// The `BuildHasher` for [`FxHashMap`] / [`FxHashSet`]; `Default`, so `HashMap::default()`
/// builds one.
pub type FxBuildHasher = BuildHasherDefault<FxHasher>;
/// A `HashMap` hashed by [`FxHasher`].
pub type FxHashMap<K, V> = HashMap<K, V, FxBuildHasher>;
/// A `HashSet` hashed by [`FxHasher`].
pub type FxHashSet<K> = HashSet<K, FxBuildHasher>;

#[cfg(test)]
mod tests {
    use super::*;
    use std::hash::{BuildHasher, Hash};

    /// The owned and the borrowed spelling of a definition key must hash alike here as they
    /// do under SipHash — the index's whole borrowed-lookup trick rests on it.
    #[test]
    fn a_str_and_its_string_hash_alike() {
        let b = FxBuildHasher::default();
        let h = |k: &dyn Fn(&mut FxHasher)| {
            let mut s = b.build_hasher();
            k(&mut s);
            s.finish()
        };
        let owned = ("n_main".to_string(), 7u16);
        let borrowed = ("n_main", 7u16);
        assert_eq!(h(&|s| owned.hash(s)), h(&|s| borrowed.hash(s)));
        assert_ne!(h(&|s| owned.hash(s)), h(&|s| ("n_main", 8u16).hash(s)));
    }

    /// Sequential small integers — definition and variable numbers — must not collide in
    /// the low bits a table indexes by.
    #[test]
    fn sequential_keys_spread() {
        let b = FxBuildHasher::default();
        let mut low: HashSet<u64> = HashSet::new();
        for i in 0u32..4096 {
            low.insert(b.hash_one(i) & 0xfff);
        }
        // A random hash reaches 4096 · (1 − 1/e) ≈ 2 590 distinct values; a poorly mixed one
        // reaches far fewer (the unfinished product: 1 998).
        assert!(
            low.len() > 2400,
            "only {} distinct low-12-bit values",
            low.len()
        );
    }
}
