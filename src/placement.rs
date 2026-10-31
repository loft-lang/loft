// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! The on-disk PLACEMENT contract of the keyed collections (@PLN135 Q2, `@FR-S-Format`).
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
//! A bump need not refuse the stores a build CAN read: [`HASH_READS`] lists the older hash
//! tokens this build reads as written, and `schema_sidecar::classify` hands such a store over
//! raw (@C152).
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
///
/// `3` is the tagged and inline tables (`(Col-Tag)`, `(Col-Inline)`): a slot may carry a tag,
/// a table may keep its entries in its own record, and each table records which in its
/// header (`(Col-Layouts)`, @C152).  A build before them reads such a table as untagged and
/// would misread it, so it must REFUSE a store carrying `3` — which a token it does not know
/// makes it do.  This build reads a `2` store as it was written ([`HASH_READS`]).
pub const HASH: &str = "3";

/// The older [`HASH`] tokens this build reads as they were written (@C152): every table of a
/// `2` store is an arena table of untagged slots, which a `3` table's header can also say, so
/// such a store is handed over unchanged — no conversion, no refusal.  `1` is not here: a slot
/// of a `1` store holds a record number, not an arena index.
pub const HASH_READS: &[&str] = &["2"];

/// Does this build read a `hash` written under placement `token`?
#[must_use]
pub fn hash_reads(token: &str) -> bool {
    token == HASH || HASH_READS.contains(&token)
}

/// `dump` (a `Stores::layout_dump`) with every `hash` part written under a token this build
/// reads ([`HASH_READS`]) given this build's own: the dump the same types have when written
/// now.  A store whose dump equals the running program's after this is read raw, as an
/// unchanged layout is (`schema_sidecar::classify`).  Any other part is left as it is, so a
/// token this build does not read still differs and is refused.
#[must_use]
pub fn read_as_own(dump: &str) -> std::borrow::Cow<'_, str> {
    let own = tag(HASH);
    let mut out = String::new();
    let mut from = 0;
    let mut at = 0;
    while let Some(i) = dump[at..].find("hash<") {
        let start = at + i;
        at = start + 5;
        // A row names its hash type and then lays it out (`hash<E[k]>\tsize=4\thash<E>(…)`):
        // both reach the same part, which is rewritten once.
        if start < from || (start > 0 && dump.as_bytes()[start - 1].is_ascii_alphanumeric()) {
            continue;
        }
        let Some(e) = dump[start..].find(",elem_size=") else {
            break;
        };
        let digits = start + e + ",elem_size=".len();
        let end = digits
            + dump[digits..]
                .bytes()
                .take_while(u8::is_ascii_digit)
                .count();
        let rest = &dump[end..];
        let Some(token) = rest
            .strip_prefix(",placement=")
            .and_then(|r| r.split_once(')'))
            .map(|(t, _)| t)
        else {
            continue;
        };
        if HASH_READS.contains(&token) {
            out.push_str(&dump[from..end]);
            out.push_str(&own);
            from = end + ",placement=".len() + token.len();
            at = from;
        }
    }
    if from == 0 {
        return std::borrow::Cow::Borrowed(dump);
    }
    out.push_str(&dump[from..]);
    std::borrow::Cow::Owned(out)
}

/// `index<T[k]>` — the B+tree in `crate::btree` (@C147): the collection's slot names
/// an ARENA record whose node 0 is the header and whose nodes hold 8-byte key parts beside
/// record numbers, descended by part and broken by the record's key.
///
/// `2` is that tree.  An older layout put, in this slot, the ROOT RECORD of a red-black tree threaded
/// through the element records' own `#left/#right/#color` fields; a store written then and
/// read now would take that record for an arena header and descend through bytes nothing
/// laid out as nodes, so it must REFUSE rather than answer.
pub const INDEX: &str = "2";

/// The KEY ORDER a `sorted`, `ordered` or `index` keyed by a `float`, `single` or `text`
/// field stores its entries in, as the suffix its line in `Stores::layout_dump` carries.
///
/// `@FR-E-NullArg` places a null at the LOW extreme of every ordered type; the key
/// comparison (`keys::compare`, the B-tree's parts) put a null `float` / `single` above
/// every value and a null `text` after `""` until loft#2237.  Entries stand in that order
/// in a saved image, and a lookup binary-searches it, so an image written under the old
/// order and searched under the new one answers a present key as absent.  The suffix
/// makes such a collection's layout identity differ from an old image's, so a whole-image
/// load CONVERTS the old image — `UpgradeMap` rebuilds a keyed collection element by
/// element, through this build's comparison — and a paged load refuses it (the
/// owner's ruling).  An integer key always ordered its null first, so its line, and every
/// identity recorded for it, is unchanged.
pub const KEY_ORDER: &str = ",order=1";

/// Does a key of type number `type_nr` (`keys::Key::type_nr`) order by [`KEY_ORDER`]?
#[must_use]
pub fn key_orders(type_nr: i8) -> bool {
    matches!(type_nr.unsigned_abs(), 3 | 4 | 6)
}

/// `spatial<T[k]>` — the radix/Morton tree in `crate::radix_db`.
pub const RADIX: &str = "1";

/// `trie<T[k]>` — the paged text trie in `crate::trie_db`.
///
/// `2` is loft#2241 (`@FR-Col-Trie`): a key's data bytes `0x00`/`0x01` are filed escaped
/// (`01 01`/`01 02`), so no key reads as another that ends in NULs, and a null key lives in a
/// slot ahead of the tree — the header word `1` used for the node capacity.  A `1` trie files
/// every other key exactly as `2` does ([`TRIE_READS`]).
pub const TRIE: &str = "2";
/// The older [`TRIE`] tokens this build reads.  A whole-image load CONVERTS such a trie — walks
/// it key-free and files every record again, an old null key into the slot — because a key it
/// holds with a `0x00`/`0x01` byte sits where the escape would not look.  A paged read reads it
/// as it was written, through the old key reading (`paged_reader::PagedReader::trie_v1`).
pub const TRIE_READS: &[&str] = &["1"];
/// Does this build read a `trie` written under placement `token`?
#[must_use]
pub fn trie_reads(token: &str) -> bool {
    token == TRIE || TRIE_READS.contains(&token)
}

/// `dump` with every `trie` part written under a token this build reads ([`TRIE_READS`])
/// given this build's own token — what the same types render as now.  The upgrade compares
/// an old layout re-rendered through this build's rows against this, since a row renders
/// this build's token; the trie itself is then walked and filed again, never read raw.
/// A row's layout part is its last tab-separated column, where a trie is one whole part.
#[must_use]
pub fn trie_read_as_own(dump: &str) -> String {
    let own = tag(TRIE);
    let mut out = String::with_capacity(dump.len());
    for (i, line) in dump.split('\n').enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let (head, part) = line.rsplit_once('\t').map_or(("", line), |(h, p)| (h, p));
        let rewritten = part.strip_prefix("trie<").and_then(|_| {
            let (body, token) = match part.split_once(",placement=") {
                Some((b, t)) => (b, t),
                None => (part, BASELINE),
            };
            (body.ends_with('>') && TRIE_READS.contains(&token)).then(|| format!("{body}{own}"))
        });
        match rewritten {
            Some(r) if !head.is_empty() => {
                out.push_str(head);
                out.push('\t');
                out.push_str(&r);
            }
            _ => out.push_str(line),
        }
    }
    out
}

/// The suffix a keyed collection contributes to its line in `Stores::layout_dump`.
///
/// Empty for the baseline, so the dump — and therefore every recorded layout hash —
/// is byte-identical until a placement actually changes.
///
/// `sorted` / `ordered` deliberately have no token. Their placement is not an algorithm
/// a reader has to reproduce: entries sit in key order and a reader re-derives that by
/// COMPARING keys it can already read. A change to the comparison order is a change to
/// `keys::compare`, which no per-kind token describes: it is [`KEY_ORDER`]'s.
#[must_use]
pub fn tag(token: &str) -> String {
    if token == BASELINE {
        String::new()
    } else {
        format!(",placement={token}")
    }
}
