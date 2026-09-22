// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! The on-disk PLACEMENT contract of the keyed collections (@PLN135 Q2).
//!
//! A persisted store is a raw image, and the @PLN97 layout identity commits to how its
//! bytes are *shaped* — record sizes, field positions, narrow-int encodings, element
//! strides, host endianness. It says nothing about **where a keyed collection puts an
//! entry**: which bucket a key hashes to, how a probe walks on, how a tree node is
//! reached. Two binaries can agree on every byte of the layout and disagree completely
//! on placement.
//!
//! That gap is not hypothetical — it is exactly what @PLN135's remaining arcs do. Arc D
//! widens a bucket slot to `(rec, hash)`, arc E changes the hash function and the index
//! derivation, arc H replaces the bucket array with an inline entry array. A store
//! written before any of those and read after it would pass the layout gate and then be
//! MISREAD: lookups find nothing, or find a neighbour, with no error anywhere. A silent
//! wrong answer is the one outcome the compatibility doctrine does not allow, so the
//! identity has to carry placement too.
//!
//! # How it refuses
//!
//! Each kind carries a token. The token is rendered into `Stores::layout_dump`, which
//! feeds `layout_algo_hash`, which is what `LayoutIdentity` records in the `.dschema`
//! sidecar beside a persisted store. `Stores::layout_verdict_ok` compares the sidecar
//! with the running program's identity on every whole-image load and every bind of an
//! existing file, and the paged/remote loaders gate on the same value before
//! range-reading foreign bytes. So bumping a token here
//! turns "misread silently" into the refusal that path already knows how to give:
//!
//! ```text
//! store_load: refusing <path> — it was written with a different layout than this
//! program reads it with, so its records would be read at the wrong stride …
//! ```
//!
//! Only stores that CONTAIN the changed kind are affected: the token is rendered per
//! collection, so bumping [`HASH`] leaves a store of plain structs and vectors loading
//! exactly as before. That is why this is a token per kind rather than a bump of
//! `Store::SIGNATURE`, which would refuse every store ever written.
//!
//! # Why absence means the baseline
//!
//! A token equal to [`BASELINE`] renders as **nothing at all**, so introducing this
//! mechanism does not change a single existing layout hash and does not invalidate a
//! single store already on disk. That is the correct reading rather than a trick: a
//! store written before the identity carried placement was written with the baseline
//! placement, by definition. The first real bump is then the only break, and it lands
//! exactly when placement actually changes.
//!
//! # When to bump
//!
//! Bump a kind's token when ANY of these changes for that kind — anything a reader must
//! reproduce to find an entry that a writer put somewhere:
//!
//! * the record layout of the table/tree itself (a reserved word, a slot width);
//! * the hash function, or how a hash value becomes a slot index;
//! * the probe or descent order;
//! * how the slot count is derived from the claimed size.
//!
//! `placement_contract_is_pinned` in `tests/layout_golden.rs` is what makes that
//! reliable: it pins the constants and the hash outputs a reader depends on, so a change
//! to any of them fails with the instruction to bump the token rather than trusting
//! whoever makes the change to remember this file.
//!
//! # What a token cannot cover, and why it no longer has to (loft#827)
//!
//! This mechanism refuses a store whose placement LOFT computes differently. It is blind
//! by construction to a store whose placement changed without loft changing — and that
//! WAS reachable: `keys::key_hash` ran on `std::hash::DefaultHasher`, whose algorithm std
//! does not guarantee across Rust releases, while the seed in the store makes every
//! reader re-derive buckets from it. A toolchain upgrade could therefore move placement
//! with no token to bump and nothing to refuse on.
//!
//! Closed by owning the hash: [`crate::siphash::SipHasher13`] is a byte-identical copy of
//! what `DefaultHasher` computes, proven against it in `tests/siphash_std_parity.rs`. It
//! was NOT a placement change — [`HASH`] did not bump and no store was invalidated — and
//! that is the point: the format now depends on loft alone, so a future std change is a
//! red test rather than a silent misread of somebody's data.

/// The placement every kind shipped with when the layout identity gained this field.
/// A token equal to this renders as nothing, so stores written before it are unaffected.
const BASELINE: &str = "1";

/// `hash<T[k]>` — the open-addressed bucket array in `crate::hash`: a size header word,
/// a seed word, the entry arena's four bookkeeping words, then `u32` ENTRY INDICES
/// probed linearly forward, indexed by `key_hash(key, seed) % elms` with
/// `elms = (room - 4) * 2`.
///
/// `2` is @PLN135 arc H.  A slot used to hold the record number of an entry that was a
/// store record of its own; it now holds a 1-based index into the chunked arena in
/// [`crate::arena`], where entries live packed at a fixed stride.  A pre-H store read
/// by a post-H loft would take a record number for an arena index and find an entry at
/// a byte offset nothing put one at, so it must REFUSE rather than answer — which is
/// what this token buys, and the reason it shipped before H rather than with it.
pub const HASH: &str = "2";

/// `index<T[k]>` — the red-black tree in `crate::tree`.
pub const INDEX: &str = "1";

/// `spatial<T[k]>` — the radix/Morton tree in `crate::radix_db`.
pub const RADIX: &str = "1";

/// `trie<T[k]>` — the paged text trie in `crate::trie_db`.
pub const TRIE: &str = "1";

/// The suffix a keyed collection contributes to its line in `Stores::layout_dump`.
///
/// Empty for the baseline, so the dump — and therefore every recorded layout hash —
/// is byte-identical until a placement actually changes.
///
/// `sorted` / `ordered` deliberately have no token. Their placement is not an algorithm
/// a reader has to reproduce: entries sit in key order and a reader re-derives that by
/// COMPARING keys it can already read. A change to the comparison order would be a
/// change to `keys::compare`, which reorders every kind at once and is a wider break
/// than this per-kind mechanism describes.
#[must_use]
pub fn tag(token: &str) -> String {
    if token == BASELINE {
        String::new()
    } else {
        format!(",placement={token}")
    }
}
