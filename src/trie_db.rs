// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I70 — Database subsystem (alloc / persistence / journal / snapshot / schema)

//! The bit-key oracle for a TEXT-keyed radix collection.
//!
//! Its own module, not a branch inside `radix_db`. A trie and a spatial index share
//! the radix TREE and nothing above it: `spatial`'s properties are geometric — Morton
//! interleaving, bounding boxes, near/within/nearest — and none of them mean anything
//! for a word. Sharing the storage structure is not sharing the kind, which is why
//! `spatial` is not called `radix` at the surface. See
//! `doc/claude/plans/text-keyed-trie.md`.
//!
//! The tree itself needs nothing new: `radix_tree.rs` is a PATRICIA tree over an
//! abstract bit-key oracle with per-record `bits()`, and its `TERM_BITS` suffix exists
//! so a shorter key sorts before a longer one that extends it — a string concern,
//! proven by `r8c`. This module supplies the bytes; the tree does the rest.
//!
//! The same key reading serves a PAGED trie: `paged_reader::trie_find_rec` and
//! `trie_prefix_recs` answer over an image instead of a `Store`, composing keys with
//! this module's `bytes_word` and descending with `radix_tree`'s own geometry, so
//! there is one answer and not two (@PLN134). `mod paged` below pins that they agree.

use crate::keys::{Content, DbRef, Key};
use crate::radix_tree::{self as rt, KeyOracle};
use crate::store::Store;

/// Payload offset of an element record's fields; mirrors `radix_db`.
pub(crate) const PAYLOAD: u32 = 8;

/// The text key's bytes, read through the field's string pointer.
///
/// `type_nr == 6` is a `text` key; the field holds a `u32` pointer to the string
/// record, which is the same indirection `compare_key` follows.
fn text_bytes<'a>(store: &'a Store, rec: u32, key: &Key) -> &'a str {
    let p = PAYLOAD + u32::from(key.position);
    store.get_str(store.get_u32_raw(rec, p))
}

/// `text`'s key type number, as `compare_key` spells it.
///
/// Read only by this module's tests until the trie KIND lands (step 2 of
/// `doc/claude/plans/text-keyed-trie.md`), which is what will select an oracle by key
/// type. Kept here rather than inlined into the tests because it is a fact about the
/// schema, not about the fixture.
#[allow(dead_code)]
const TEXT_TYPE_NR: i8 = 6;

/// How the tree reads a TEXT record's key: its bytes, big-endian, 8 at a time.
///
/// Big-endian is what makes bytewise order and bit order the same, so the tree's
/// in-order traversal is lexicographic and a prefix descends to its extensions.
/// `bits()` is per-record — the variable-length half of [`KeyOracle`] that the
/// Morton oracle never exercises, and that `radix_tree`'s `TERM_BITS` suffix exists
/// to order (`"ab"` sorts before `"abc"`; proven by `r8c`).
struct TextOracle<'a> {
    key: &'a Key,
}

impl KeyOracle for TextOracle<'_> {
    fn bits(&self, store: &Store, rec: u32) -> u32 {
        text_bytes(store, rec, self.key).len() as u32 * 8
    }
    fn word(&self, store: &Store, rec: u32, word: u32) -> u64 {
        bytes_word(text_bytes(store, rec, self.key).as_bytes(), word)
    }
}

/// The `word`-th big-endian 8-byte chunk of `b`, zero-padded past the end.
/// Shared by the record oracle and the probe so both read a key the same way — the
/// two disagreeing is the classic radix fault, and it presents as "inserted, not
/// found". `pub(crate)` for the same reason one step out: the PAGED walk over a
/// persisted trie reads keys out of an image and must compose them identically
/// (@PLN134).
pub(crate) fn bytes_word(b: &[u8], word: u32) -> u64 {
    let mut out = 0u64;
    for i in 0..8 {
        let idx = (word * 8 + i) as usize;
        out = (out << 8) | u64::from(b.get(idx).copied().unwrap_or(0));
    }
    out
}

/// The bytes a query key carries, when it is a text one.
///
/// A non-text probe cannot match a text key: answering an empty slice makes that a
/// miss rather than a panic, which is the same choice `probe_code` makes on the
/// coordinate side.
fn probe_bytes(key: &[Content]) -> &[u8] {
    match key.first() {
        Some(Content::Str(v)) => v.str().as_bytes(),
        _ => &[],
    }
}

/// Insert element record `rec` into the `Trie` collection at field `coll`.
///
/// The tree lives in `coll`'s store, its record id in `coll`'s 4-byte field (the
/// `hash`-bucket convention). A growing insert can relocate the tree, so the
/// (possibly new) id is written back. Two records may share a key — they differ in
/// the id suffix and land adjacent, which `r8b` pins.
pub fn add(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) {
    let Some(key) = keys.first() else {
        return;
    };
    let store = crate::keys::mut_store(coll, stores);
    let mut tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        tree = rt::rtree_init(store, 0);
        store.set_u32_raw(coll.rec, coll.pos, tree);
    }
    let tree = rt::rtree_insert(store, tree, rec.rec, &TextOracle { key });
    store.set_u32_raw(coll.rec, coll.pos, tree);
}

/// The record whose key equals `key`, or a null `DbRef` (`rec == 0`) when absent.
/// When several records share the key, the first in order.
#[must_use]
pub fn find(coll: &DbRef, stores: &[Store], keys: &[Key], key: &[Content]) -> DbRef {
    let store = crate::keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let rec = match keys.first() {
        Some(k) if tree != 0 => {
            // The probe reads bytes exactly as `TextOracle` does — via the same
            // `bytes_word`. The two reading a key differently is the classic radix
            // fault and presents as "inserted, then not found".
            let q = probe_bytes(key);
            let probe = |word: u32| bytes_word(q, word);
            rt::rtree_get(
                store,
                tree,
                &probe,
                q.len() as u32 * 8,
                &TextOracle { key: k },
            )
        }
        _ => 0,
    };
    DbRef {
        store_nr: coll.store_nr,
        rec,
        pos: PAYLOAD,
    }
}

/// Unlink element record `rec`; the caller frees `rec` itself. `false` when it was
/// not present — the tree removes only the record whose key AND id match, so a
/// same-key sibling is never removed by mistake.
pub fn remove(coll: &DbRef, rec: &DbRef, stores: &mut [Store], keys: &[Key]) -> bool {
    let Some(key) = keys.first() else {
        return false;
    };
    let store = crate::keys::mut_store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        return false;
    }
    rt::rtree_remove(store, tree, rec.rec, &TextOracle { key })
}

/// Number of element records. Reads the tree's cached length word (O(1)).
#[must_use]
pub fn count(coll: &DbRef, stores: &[Store]) -> u32 {
    // An absent collection has no records; tested before any store is indexed.
    if coll.is_null() {
        return 0;
    }
    let store = crate::keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    if tree == 0 {
        0
    } else {
        rt::rtree_len(store, tree)
    }
}

/// Every element record, in key order. Key-free (a plain tree walk), so teardown
/// and iteration reach it without building an oracle.
#[must_use]
pub fn records(coll: &DbRef, stores: &[Store]) -> Vec<u32> {
    let store = crate::keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let mut out = Vec::new();
    if tree != 0 {
        let mut it = rt::rtree_first(store, tree);
        let mut r = it.rec();
        while r != 0 {
            out.push(r);
            r = it.next(store, tree).unwrap_or(0);
        }
    }
    out
}

/// Every record whose key begins with `pre`, in key order, capped at `limit`.
///
/// The capability that earns the kind its place, and the one `sorted` cannot offer.
/// A `sorted` range needs a SUCCESSOR string — `c["kerk".."kerl"]` — which the caller
/// has to construct, gets wrong at a byte boundary, and which answers a key INTERVAL
/// rather than a prefix. Here the prefix is the query: seek to it, then walk while the
/// key still begins with it.
///
/// Both halves lean on facts the tree already guarantees. `rtree_seek` positions at
/// the lowest key `>= pre`, and because a probe carries id `0` that is the first
/// record BEARING the prefix (`radix_tree`, and `r8` pins it). In-order traversal is
/// increasing key order, so every extension of the prefix is contiguous from there —
/// which makes the first key that does not begin with `pre` a correct stop, not a
/// heuristic one.
///
/// An empty `pre` is every record, which is what "begins with nothing" means and what
/// `starts_with` already answers.
#[must_use]
pub fn prefix(
    coll: &DbRef,
    stores: &[Store],
    keys: &[Key],
    pre: &[u8],
    limit: Option<usize>,
) -> Vec<u32> {
    let store = crate::keys::store(coll, stores);
    let tree = store.get_u32_raw(coll.rec, coll.pos);
    let mut out = Vec::new();
    let Some(key) = keys.first() else {
        return out;
    };
    if tree == 0 {
        return out;
    }
    let probe = |word: u32| bytes_word(pre, word);
    let mut it = rt::rtree_seek(
        store,
        tree,
        &probe,
        pre.len() as u32 * 8,
        &TextOracle { key },
    );
    let cap = limit.unwrap_or(usize::MAX);
    let mut rec = it.rec();
    while rec != 0 && out.len() < cap {
        if !text_bytes(store, rec, key).as_bytes().starts_with(pre) {
            break;
        }
        out.push(rec);
        rec = it.next(store, tree).unwrap_or(0);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::radix_tree as rt;

    /// One `text` key at payload offset 0.
    fn w_key() -> Key {
        Key {
            type_nr: TEXT_TYPE_NR,
            position: 0,
            start: 0,
        }
    }

    /// An element record whose text field holds `w` — the shape `compare_key` reads:
    /// a `u32` pointer at `PAYLOAD + position` to the string record.
    fn add_word(store: &mut Store, w: &str) -> u32 {
        let ptr = store.set_str(w);
        let rec = store.claim(4);
        store.set_u32_raw(rec, PAYLOAD, ptr);
        rec
    }

    const WORDS: [&str; 6] = [
        "kerkstraat",
        "kerk",
        "lonneker",
        "kerf",
        "kerkweg",
        "kerklaan",
    ];

    /// Fill a tree with `WORDS` in an order that is neither sorted nor reverse-sorted,
    /// so a passing order cannot have come from the insertion sequence.
    fn build(store: &mut Store) -> (u32, Vec<(&'static str, u32)>) {
        let key = w_key();
        let mut tree = rt::rtree_init(store, 8);
        let mut recs = Vec::new();
        for w in WORDS {
            let rec = add_word(store, w);
            recs.push((w, rec));
            tree = rt::rtree_insert(store, tree, rec, &TextOracle { key: &key });
        }
        (tree, recs)
    }

    /// In-order traversal is lexicographic, and a prefix sorts before what extends it.
    ///
    /// The `TERM_BITS` property, exercised through the DB record layout rather than a
    /// tree-local fixture: `kerk` must precede the three keys that extend it.
    #[test]
    fn text_keys_order_lexicographically_through_the_db_layout() {
        let mut store = Store::new_in_use(1024);
        let key = w_key();
        let (tree, recs) = build(&mut store);
        rt::rtree_validate(&store, tree, &TextOracle { key: &key });

        let mut it = rt::rtree_first(&store, tree);
        let mut seen = Vec::new();
        let mut r = it.rec();
        while r != 0 {
            seen.push(
                recs.iter()
                    .find(|(_, rec)| *rec == r)
                    .expect("record came from this tree")
                    .0,
            );
            r = it.next(&store, tree).unwrap_or(0);
        }
        assert_eq!(
            seen,
            [
                "kerf",
                "kerk",
                "kerklaan",
                "kerkstraat",
                "kerkweg",
                "lonneker"
            ],
            "bytewise order, with `kerk` before the three keys that extend it"
        );
    }

    /// A key just inserted must be found, and one that was not must not be — the two
    /// halves loft#799 got wrong for a text-keyed `spatial`.
    #[test]
    fn text_keys_answer_exact_lookups_through_the_db_layout() {
        let mut store = Store::new_in_use(1024);
        let key = w_key();
        let (tree, recs) = build(&mut store);

        for (w, want) in &recs {
            let q = w.as_bytes();
            let probe = |word: u32| bytes_word(q, word);
            let got = rt::rtree_get(
                &store,
                tree,
                &probe,
                q.len() as u32 * 8,
                &TextOracle { key: &key },
            );
            assert_eq!(got, *want, "exact lookup of {w:?}");
        }
        // `kerks` extends `kerk` AND is a prefix of `kerkstraat` — the shape a wrong
        // terminator or a probe/oracle mismatch answers for.
        for absent in ["kerks", "ker", "kerkstraatx", "aaa", "zzz"] {
            let q = absent.as_bytes();
            let probe = |word: u32| bytes_word(q, word);
            let got = rt::rtree_get(
                &store,
                tree,
                &probe,
                q.len() as u32 * 8,
                &TextOracle { key: &key },
            );
            assert_eq!(got, 0, "{absent:?} is absent and must not be found");
        }
    }

    // ---- the OPERATIONS, through the collection surface ---------------------
    //
    // The tests above drive the tree directly. These drive `add` / `find` / `remove` /
    // `records` / `count` — the functions `search.rs` and `structures.rs` now call —
    // so a mistake in the collection plumbing (the tree id written back into the
    // field, an empty key list, a probe built the wrong way) cannot hide behind a
    // correct tree.

    /// A collection field holding the tree id, plus an element record per word.
    fn collection(store: &mut Store) -> (DbRef, Vec<(&'static str, u32)>) {
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        let keys = vec![w_key()];
        let mut recs = Vec::new();
        for w in WORDS {
            let ptr = store.set_str(w);
            let rec = store.claim(4);
            store.set_u32_raw(rec, PAYLOAD, ptr);
            add(
                &coll,
                &DbRef {
                    store_nr: 0,
                    rec,
                    pos: PAYLOAD,
                },
                std::slice::from_mut(store),
                &keys,
            );
            recs.push((w, rec));
        }
        (coll, recs)
    }

    /// The round trip: every word inserted is found by its own key, and an absent key
    /// answers `rec == 0` rather than a neighbour.
    #[test]
    fn the_collection_round_trips_every_key() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let stores = std::slice::from_ref(&store);

        assert_eq!(count(&coll, stores), WORDS.len() as u32, "count");

        for (w, want) in &recs {
            let got = find(
                &coll,
                stores,
                &keys,
                &[Content::Str(crate::keys::Str::new(w))],
            );
            assert_eq!(got.rec, *want, "find {w:?}");
        }
        // `kerks` extends `kerk` and is a prefix of `kerkstraat` — the shape a probe
        // built differently from the oracle answers for.
        for absent in ["kerks", "ker", "kerkstraatx", "aaa", "zzz"] {
            let got = find(
                &coll,
                stores,
                &keys,
                &[Content::Str(crate::keys::Str::new(absent))],
            );
            assert_eq!(got.rec, 0, "{absent:?} must not be found");
        }
    }

    /// `records` walks in key order, so iteration and teardown see a prefix before
    /// what extends it.
    #[test]
    fn the_collection_walks_in_key_order() {
        let mut store = Store::new_in_use(1 << 14);
        let (coll, recs) = collection(&mut store);
        let walk: Vec<&str> = records(&coll, std::slice::from_ref(&store))
            .into_iter()
            .map(|r| recs.iter().find(|(_, x)| *x == r).expect("known rec").0)
            .collect();
        assert_eq!(
            walk,
            [
                "kerf",
                "kerk",
                "kerklaan",
                "kerkstraat",
                "kerkweg",
                "lonneker"
            ]
        );
    }

    /// Removal unlinks exactly one record and leaves its neighbours findable — the
    /// `kerk` family is the interesting case, since they share a prefix.
    #[test]
    fn removal_takes_one_key_and_leaves_the_rest() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let (_, kerklaan) = *recs.iter().find(|(w, _)| *w == "kerklaan").unwrap();

        let gone = remove(
            &coll,
            &DbRef {
                store_nr: 0,
                rec: kerklaan,
                pos: PAYLOAD,
            },
            std::slice::from_mut(&mut store),
            &keys,
        );
        assert!(gone, "kerklaan was present");
        assert_eq!(count(&coll, std::slice::from_ref(&store)), 5, "one fewer");

        let stores = std::slice::from_ref(&store);
        let missing = find(
            &coll,
            stores,
            &keys,
            &[Content::Str(crate::keys::Str::new("kerklaan"))],
        );
        assert_eq!(missing.rec, 0, "kerklaan is gone");
        for w in ["kerk", "kerkstraat", "kerkweg", "kerf", "lonneker"] {
            let got = find(
                &coll,
                stores,
                &keys,
                &[Content::Str(crate::keys::Str::new(w))],
            );
            assert_ne!(
                got.rec, 0,
                "{w:?} survives the removal of its prefix-sibling"
            );
        }
    }

    /// The step-5 gate. Every expectation is hand-computed from the bytes.
    ///
    /// `WORDS` sorts `kerf kerk kerklaan kerkstraat kerkweg lonneker`, so:
    ///   "kerk"  -> kerk and its three extensions, NOT kerf ('f' < 'k')
    ///   "ker"   -> those four AND kerf, since `kerf` begins with `ker` too
    ///   "kerkl" -> kerklaan alone
    ///   "kerx"  -> nothing; it sorts between kerkweg and lonneker, so a seek lands on
    ///              lonneker and a missing stop-check would answer that instead
    ///   ""      -> everything
    #[test]
    fn a_prefix_yields_its_extensions_and_stops() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let stores = std::slice::from_ref(&store);
        let names = |v: Vec<u32>| -> Vec<&str> {
            v.into_iter()
                .map(|r| recs.iter().find(|(_, x)| *x == r).expect("known rec").0)
                .collect()
        };

        assert_eq!(
            names(prefix(&coll, stores, &keys, b"kerk", None)),
            ["kerk", "kerklaan", "kerkstraat", "kerkweg"],
            "the prefix itself is included, and kerf is not"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"ker", None)),
            ["kerf", "kerk", "kerklaan", "kerkstraat", "kerkweg"],
            "kerf begins with `ker`, so a shorter prefix widens the answer"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"kerkl", None)),
            ["kerklaan"],
            "a prefix that is not itself a key still finds its extension"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"lonneker", None)),
            ["lonneker"],
            "an exact key is a prefix of itself"
        );
        assert!(
            prefix(&coll, stores, &keys, b"kerx", None).is_empty(),
            "an absent prefix answers NOTHING, not the neighbour a seek lands on"
        );
        assert!(
            prefix(&coll, stores, &keys, b"zzz", None).is_empty(),
            "a prefix past the last key answers nothing"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"", None)).len(),
            WORDS.len(),
            "the empty prefix is every record"
        );
        assert_eq!(
            names(prefix(&coll, stores, &keys, b"kerk", Some(2))),
            ["kerk", "kerklaan"],
            "the cap takes the first N in key order"
        );
    }

    /// A prefix query must not be disturbed by a removal inside its run.
    #[test]
    fn a_prefix_reflects_a_removal_in_its_run() {
        let mut store = Store::new_in_use(1 << 14);
        let keys = vec![w_key()];
        let (coll, recs) = collection(&mut store);
        let (_, kerklaan) = *recs.iter().find(|(w, _)| *w == "kerklaan").unwrap();
        remove(
            &coll,
            &DbRef {
                store_nr: 0,
                rec: kerklaan,
                pos: PAYLOAD,
            },
            std::slice::from_mut(&mut store),
            &keys,
        );
        let got: Vec<&str> = prefix(&coll, std::slice::from_ref(&store), &keys, b"kerk", None)
            .into_iter()
            .map(|r| recs.iter().find(|(_, x)| *x == r).expect("known rec").0)
            .collect();
        assert_eq!(got, ["kerk", "kerkstraat", "kerkweg"], "the run closes up");
    }
}

/// @PLN134 step 4 — the PAGED walk answers exactly what the resident walk answers.
///
/// The one gate that matters for a second reader of the same tree. `paged_reader`
/// reads the node array out of an IMAGE, a page at a time, and every piece of
/// geometry it uses — descent, split point, in-order step, seek — comes from
/// `radix_tree` itself, so the two cannot silently disagree about where a key
/// lives. What they still could disagree about is everything around that: how a key
/// is read (a string record through a pointer, versus `get_str`), where the tree
/// record sits, whether a cap stops the walk or just the answer. That is what these
/// compare, record for record, against the resident functions the interpreter runs.
///
/// The corpus is generated rather than borrowed from the host, because it has to run
/// in the ordinary suite; it is built for prefix SHARING (a small syllable alphabet,
/// every length from 2 to 5 characters), which is the property a trie is about and
/// the one a random string set does not have.
#[cfg(all(test, paged_store))]
mod paged {
    use super::*;
    use crate::keys::Str;
    use crate::paged_reader::{PagedReader, trie_find_rec, trie_prefix_recs};

    /// One `text` key at payload offset 0 — the same shape `mod tests` builds.
    fn w_key() -> Key {
        Key {
            type_nr: TEXT_TYPE_NR,
            position: 0,
            start: 0,
        }
    }

    /// An in-memory image that counts the pages asked for — the bounded-walk
    /// assertion is a COUNT, so the provider has to keep one.
    struct Image {
        img: Vec<u8>,
        fetches: usize,
        bytes: usize,
    }

    impl crate::paged_reader::PageProvider for Image {
        fn size(&self) -> u64 {
            self.img.len() as u64
        }
        fn fetch(&mut self, off: u64, len: usize) -> Vec<u8> {
            self.fetches += 1;
            self.bytes += len;
            let mut buf = vec![0u8; len];
            let start = off as usize;
            if start < self.img.len() {
                let end = (start + len).min(self.img.len());
                buf[..end - start].copy_from_slice(&self.img[start..end]);
            }
            buf
        }
    }

    /// Words with heavy prefix sharing: every syllable pair, then every triple, and
    /// a fourth syllable on a slice of them. Deterministic, so a failure reproduces.
    fn corpus() -> Vec<String> {
        const SYL: [&str; 12] = [
            "ke", "ker", "lo", "am", "st", "ra", "at", "we", "la", "no", "ei", "de",
        ];
        let mut out = std::collections::BTreeSet::new();
        for a in SYL {
            for b in SYL {
                out.insert(format!("{a}{b}"));
                for c in SYL {
                    out.insert(format!("{a}{b}{c}"));
                }
            }
        }
        out.into_iter().collect()
    }

    /// Every fourth word appears TWICE.
    ///
    /// Two records may share a user key — they differ only in the 32-bit id suffix,
    /// and that suffix is the ONLY thing that orders them. Without a duplicate in
    /// the corpus the suffix region is never reached by a comparison whose outcome
    /// matters, so a paged reader that composed the id at the wrong bit offset would
    /// answer every query correctly and the differential test would confirm it.
    /// (It did: an oracle deliberately broken that way passed until this landed.)
    const DUP_EVERY: usize = 4;

    /// The corpus as a trie collection, in shuffled insertion order — the node array
    /// scatters exactly as a real build's does, so the paged walk is not being handed
    /// a layout that happens to be easy. Answers the words and how many RECORDS were
    /// inserted, which differ because of [`DUP_EVERY`].
    fn build(relayout: bool) -> (Store, DbRef, Vec<Key>, Vec<String>, usize) {
        let words = corpus();
        let keys = vec![w_key()];
        let mut store = Store::new_in_use(1 << 20);
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        let mut order: Vec<usize> = (0..words.len()).collect();
        for i in 0..words.len() {
            if i % DUP_EVERY == 0 {
                order.push(i);
            }
        }
        let records = order.len();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for i in (1..order.len()).rev() {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            order.swap(i, (seed >> 33) as usize % (i + 1));
        }
        for &i in &order {
            let ptr = store.set_str(&words[i]);
            let rec = store.claim(4);
            store.set_u32_raw(rec, PAYLOAD, ptr);
            add(
                &coll,
                &DbRef {
                    store_nr: 0,
                    rec,
                    pos: PAYLOAD,
                },
                std::slice::from_mut(&mut store),
                &keys,
            );
        }
        if relayout {
            // What `store_persist_bind` does before it writes the image, so this is
            // the layout a reader actually pages.
            let tree = store.get_u32_raw(coll.rec, coll.pos);
            assert!(crate::radix_tree::rtree_relayout(&mut store, tree));
        }
        (store, coll, keys, words, records)
    }

    fn reader_over(store: &Store, page: usize, cache: usize) -> PagedReader<Image> {
        PagedReader::with_config(
            Image {
                img: store.raw_bytes().to_vec(),
                fetches: 0,
                bytes: 0,
            },
            page,
            cache,
        )
    }

    /// The prefixes every comparison runs over: every length from 1 to 4 that the
    /// corpus can produce, plus the empty one, plus shapes with no match.
    fn probes(words: &[String]) -> Vec<String> {
        let mut set = std::collections::BTreeSet::new();
        set.insert(String::new());
        for (i, w) in words.iter().enumerate() {
            if i % 7 != 0 {
                continue;
            }
            for n in 1..=w.len().min(4) {
                set.insert(w[..n].to_string());
            }
        }
        for absent in ["zz", "kex", "amstx", "q", "keral"] {
            set.insert(absent.to_string());
        }
        set.into_iter().collect()
    }

    /// **r12 — every exact lookup and every prefix query agrees, on both layouts.**
    #[test]
    fn the_paged_walk_answers_what_the_resident_walk_answers() {
        for relayout in [false, true] {
            let (store, coll, keys, words, records) = build(relayout);
            let stores = std::slice::from_ref(&store);
            let mut reader = reader_over(&store, 4096, 64);

            let mut hits = 0;
            for w in &words {
                // `find` answers the FIRST record of a duplicated key's run, so this
                // pins which of the two the paged descent lands on, not merely that
                // it lands on one of them.
                let want = find(&coll, stores, &keys, &[Content::Str(Str::new(w))]).rec;
                let got = trie_find_rec(&mut reader, coll.rec, coll.pos, w, &keys);
                assert_eq!(got, want, "exact {w:?} (relayout={relayout})");
                hits += usize::from(want != 0);
            }
            assert_eq!(hits, words.len(), "the fixture itself must be findable");

            // Absent keys must answer 0 through BOTH readers — a walk that returned
            // the neighbour a seek landed on would pass every cell above.
            for absent in ["ke", "kerx", "keram", "", "zzzz", "amstraatx"] {
                let want = find(&coll, stores, &keys, &[Content::Str(Str::new(absent))]).rec;
                let got = trie_find_rec(&mut reader, coll.rec, coll.pos, absent, &keys);
                assert_eq!(got, want, "absent {absent:?} (relayout={relayout})");
            }

            let mut widest = 0;
            for pre in probes(&words) {
                for cap in [None, Some(0), Some(1), Some(3), Some(8)] {
                    let want = prefix(&coll, stores, &keys, pre.as_bytes(), cap);
                    let got = trie_prefix_recs(&mut reader, coll.rec, coll.pos, &pre, &keys, cap);
                    assert_eq!(
                        got, want,
                        "prefix {pre:?} cap={cap:?} (relayout={relayout})"
                    );
                    if cap.is_none() {
                        widest = widest.max(want.len());
                    }
                }
            }
            // Non-vacuity: a harness answering an empty vector everywhere would
            // satisfy every equality above. `records`, not `words`, so a walk that
            // skipped the second half of every duplicated key's run still fails.
            assert!(
                widest >= records,
                "the empty prefix must reach every record, got {widest} of {records}"
            );
        }
    }

    /// **A query reads a small part of the image — the claim the whole plan rests
    /// on.**
    ///
    /// Every other test here says the paged walk gives the RIGHT answer; a walk that
    /// fetched the whole file would satisfy all of them. This one says it gives that
    /// answer cheaply, and it is the property that regresses silently — one stray
    /// full read inside the descent costs nothing in correctness and everything in
    /// what the feature is for.
    ///
    /// Calibrated against a measured control rather than a constant: the same reader
    /// walking the WHOLE collection is what "reading everything" costs in these
    /// units, so the bound holds whatever the corpus, the page size or the machine.
    /// 512-byte pages because the node array is 37 kB — at 64 kB it is one page and
    /// the number could not distinguish a descent from a scan.
    #[test]
    fn a_query_fetches_a_small_fraction_of_what_a_full_scan_does() {
        let (store, coll, keys, _, records) = build(true);

        let mut full = reader_over(&store, 512, 4096);
        let scanned = trie_prefix_recs(&mut full, coll.rec, coll.pos, "", &keys, None);
        assert_eq!(scanned.len(), records, "the control must read everything");
        let whole = full.provider().fetches;

        let mut exact = reader_over(&store, 512, 4096);
        assert_ne!(
            trie_find_rec(&mut exact, coll.rec, coll.pos, "kerst", &keys),
            0,
            "the fixture holds `kerst`, so this is a real descent"
        );
        assert!(
            exact.provider().fetches * 20 < whole,
            "one exact lookup is a root→leaf descent plus its record: {} pages \
             against {whole} for the whole collection",
            exact.provider().fetches
        );

        for pre in ["ke", "amst", "lo", "kerst"] {
            let mut r = reader_over(&store, 512, 4096);
            let hits = trie_prefix_recs(&mut r, coll.rec, coll.pos, pre, &keys, Some(20));
            assert!(
                hits.len() >= 10,
                "{pre:?} answered {} records — too few for the page count to mean \
                 anything",
                hits.len()
            );
            assert!(
                r.provider().fetches * 5 < whole,
                "a 20-record prefix query on {pre:?} costs {} pages against {whole} \
                 for the whole collection",
                r.provider().fetches
            );
        }
    }

    /// **A SMALL tree answers too** — the case a large corpus cannot show.
    ///
    /// Every bound in the paged walk scales with the node count, so a tree with
    /// five nodes is where an under-provisioned one bites and a tree with 1 700 is
    /// where it hides. It bit exactly here: with one budget for a whole seek rather
    /// than one per bounded walk, this six-word trie answered every exact lookup as
    /// ABSENT while the prefix walk on the same image still worked.
    #[test]
    fn a_tree_of_six_records_answers_every_lookup() {
        const SIX: [&str; 6] = [
            "kerkstraat",
            "kerk",
            "lonneker",
            "kerf",
            "kerkweg",
            "kerklaan",
        ];
        let keys = vec![w_key()];
        let mut store = Store::new_in_use(1 << 14);
        let coll_rec = store.claim(1);
        let coll = DbRef {
            store_nr: 0,
            rec: coll_rec,
            pos: 4,
        };
        for w in SIX {
            let ptr = store.set_str(w);
            let rec = store.claim(4);
            store.set_u32_raw(rec, PAYLOAD, ptr);
            add(
                &coll,
                &DbRef {
                    store_nr: 0,
                    rec,
                    pos: PAYLOAD,
                },
                std::slice::from_mut(&mut store),
                &keys,
            );
        }
        let tree = store.get_u32_raw(coll.rec, coll.pos);
        assert!(crate::radix_tree::rtree_relayout(&mut store, tree));
        let stores = std::slice::from_ref(&store);
        let mut reader = reader_over(&store, 4096, 64);

        for w in SIX {
            let want = find(&coll, stores, &keys, &[Content::Str(Str::new(w))]).rec;
            assert_ne!(want, 0, "the fixture holds {w:?}");
            assert_eq!(
                trie_find_rec(&mut reader, coll.rec, coll.pos, w, &keys),
                want,
                "exact {w:?} on a six-record tree"
            );
        }
        for pre in ["kerk", "ker", "kerkl", "lon", "", "kerx", "zzz"] {
            assert_eq!(
                trie_prefix_recs(&mut reader, coll.rec, coll.pos, pre, &keys, None),
                prefix(&coll, stores, &keys, pre.as_bytes(), None),
                "prefix {pre:?} on a six-record tree"
            );
        }
    }

    /// **The cap bounds the WALK, not just the answer.**
    ///
    /// `t["ke"..:4]` must stop after the fourth record — the fifth is never stepped
    /// to, so its pages are never asked for. Measured as a page count, because that
    /// is the only thing that tells a bounded walk from a full one that truncates:
    /// both return four records.
    #[test]
    fn a_capped_prefix_walk_stops_fetching_at_the_cap() {
        let (store, coll, keys, _, _) = build(true);
        // One node per page, no cache: every record the walk visits shows up as a
        // fetch, so the two runs differ by exactly the records not visited.
        let mut capped = reader_over(&store, 64, 1);
        let mut whole = reader_over(&store, 64, 1);

        let few = trie_prefix_recs(&mut capped, coll.rec, coll.pos, "ke", &keys, Some(4));
        let all = trie_prefix_recs(&mut whole, coll.rec, coll.pos, "ke", &keys, None);
        assert_eq!(few.len(), 4, "the cap is what it says");
        assert!(all.len() > 40, "and the run it stops short of is long");
        assert_eq!(&all[..4], &few[..], "the same first four, in key order");
        assert!(
            capped.provider().fetches * 4 < whole.provider().fetches,
            "a capped walk must not read the untaken tail: {} fetches capped vs {} whole",
            capped.provider().fetches,
            whole.provider().fetches
        );
    }

    /// **A corrupted image answers; it does not hang.**
    ///
    /// The bytes are a FILE — truncated, foreign or hostile — and a child pointer
    /// that cycles would spin a descent forever where a resident tree cannot, because
    /// this process built that one. The fuel bound turns it into an absence.
    ///
    /// Proven non-vacuous by the control: the same query on the same image, with the
    /// cycle NOT written, still answers its records.
    #[test]
    fn a_cyclic_node_pointer_ends_the_walk_instead_of_spinning() {
        let (store, coll, keys, _, _) = build(true);
        let tree = store.get_u32_raw(coll.rec, coll.pos);

        let mut healthy = reader_over(&store, 4096, 64);
        assert!(
            !trie_prefix_recs(&mut healthy, coll.rec, coll.pos, "ke", &keys, None).is_empty(),
            "control: the intact image answers"
        );

        // Point the root node's FALSE child at the root node itself.
        let mut bytes = store.raw_bytes().to_vec();
        let off = usize::try_from(u64::from(tree) * 8).unwrap()
            + crate::radix_tree::child_off(1, false) as usize;
        bytes[off..off + 4].copy_from_slice(&(-1i32).to_ne_bytes());
        let mut broken = PagedReader::with_config(
            Image {
                img: bytes,
                fetches: 0,
                bytes: 0,
            },
            4096,
            64,
        );

        // The assertion is that these RETURN. A value is asserted too so the calls
        // cannot be optimised into nothing.
        let found = trie_find_rec(&mut broken, coll.rec, coll.pos, "kera", &keys);
        let run = trie_prefix_recs(&mut broken, coll.rec, coll.pos, "ke", &keys, None);
        assert!(
            found == 0 || run.len() <= 1,
            "a cycle degrades to an absence, it does not invent a run"
        );
    }
}

/// @PLN134 — how many 64 KB PAGES one prefix query touches, and what a LAYOUT
/// would do to that number.
///
/// The measurement the plan opens on, and the reason it opens on one at all: a
/// PATRICIA descent is cheap in NODES — one root→leaf path, branching on bits of a
/// probe the caller already holds — and that says nothing about what it costs over a
/// link. A reader fetches `paged_reader::PAGE_SIZE` (64 KB) at a time, and node ids
/// are handed out in INSERTION order, so a path through a large tree can land on a
/// fresh page at every hop.
///
/// Nodes are one contiguous array (`radix_tree::node_off` is `HDR + 16*(id-1)`), so
/// the span a descent can reach is bounded by `16 × node_count` — which is the fact
/// that makes the answer worth measuring rather than assuming in either direction.
/// Step 1 measured it: ~27 pages as built, ~15 renumbered breadth-first, to read
/// ~330 bytes of nodes. So the question became which layout reaches the floor, and
/// every candidate here is a PERMUTATION fed to the same counter — nothing is
/// rebuilt, so a layout costs one walk to evaluate rather than a persist pass.
///
/// Three questions, one fixture:
///
/// * [`prefix_query_page_span`] — the node array, under five numberings.
/// * [`record_placement`] — the records a query RETURNS, which live elsewhere.
/// * [`warm_cache_session`] — what the second keystroke costs, cache still warm.
///
/// `#[ignore]` because they read the host's dictionaries and print rather than
/// assert — they answer a design question, they do not defend an invariant.
///
/// ```text
/// cargo test --release --lib trie_db::pages -- --ignored --nocapture
/// ```
#[cfg(test)]
mod pages {
    #![allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss
    )]

    use super::*;
    use crate::page_metrics::{
        CACHE_PAGES, FINE, PAGE, SessionCache, add_span, layouts, mean, pct, row,
    };
    use crate::radix_tree as rt;
    use std::collections::{BTreeSet, HashMap};

    /// How many records a query returns; `t["kerk"..:20]` caps it, and the cap is
    /// what makes a search box cheap.
    const CAP: usize = 20;

    fn w_key() -> Key {
        Key {
            type_nr: TEXT_TYPE_NR,
            position: 0,
            start: 0,
        }
    }

    fn add_word(store: &mut Store, w: &str) -> u32 {
        let ptr = store.set_str(w);
        let rec = store.claim(4);
        store.set_u32_raw(rec, PAYLOAD, ptr);
        rec
    }

    /// Real words, not generated ones: prefix SHARING is the whole subject, and a
    /// synthetic vocabulary has whatever sharing its generator happened to produce.
    /// Several languages, because routing's index is place names — proper nouns
    /// across languages, which is closer to this than one language's lexicon.
    fn vocabulary() -> Vec<String> {
        let mut words = BTreeSet::new();
        for f in [
            "/usr/share/dict/american-english",
            "/usr/share/dict/british-english",
            "/usr/share/dict/french",
            "/usr/share/dict/spanish",
            "/usr/share/dict/italian",
            "/usr/share/dict/ngerman",
            "/usr/share/dict/portuguese",
        ] {
            let Ok(text) = std::fs::read_to_string(f) else {
                continue;
            };
            for w in text.lines() {
                let w = w.trim().to_lowercase();
                // ASCII only: the oracle is bytewise and a multi-byte word is not
                // wrong here, just harder to reason about when reading the output.
                if (2..=24).contains(&w.len()) && w.chars().all(|c| c.is_ascii_lowercase()) {
                    words.insert(w);
                }
            }
        }
        words.into_iter().collect()
    }

    /// The tree all three measurements read.
    struct Fixture {
        words: Vec<String>,
        store: Store,
        tree: u32,
        key: Key,
        nodes: u32,
    }

    /// Build the vocabulary trie, or `None` where the host has no dictionaries.
    fn build() -> Option<Fixture> {
        let words = vocabulary();
        if words.len() < 20_000 {
            println!("SKIP — only {} words available on this host", words.len());
            return None;
        }
        let key = w_key();
        let mut store = Store::new_in_use(1 << 20);
        let mut tree = rt::rtree_init(&mut store, 8);
        // INSERTION ORDER IS THE VARIABLE UNDER TEST. Sorted input would hand the
        // node array the one layout that cannot scatter, and answer a question
        // nobody asked — a real vocabulary arrives in whatever order its source
        // had. Shuffled deterministically so the number is reproducible.
        let mut order: Vec<usize> = (0..words.len()).collect();
        let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
        for i in (1..order.len()).rev() {
            seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
            order.swap(i, (seed >> 33) as usize % (i + 1));
        }
        for &i in &order {
            let rec = add_word(&mut store, &words[i]);
            tree = rt::rtree_insert(&mut store, tree, rec, &TextOracle { key: &key });
        }
        // The node high-water mark — the array's SIZE is what a page count is
        // measured against.
        let nodes = store.get_u32_raw(tree, crate::radix_tree::NODES);
        Some(Fixture {
            words,
            store,
            tree,
            key,
            nodes,
        })
    }

    impl Fixture {
        /// Bytes the node array occupies — `HDR + 16 × nodes`.
        fn node_bytes(&self) -> u64 {
            24 + 16 * u64::from(self.nodes)
        }

        /// The prefixes a search box actually issues: what a person has typed after
        /// `plen` keystrokes, sampled across the vocabulary so every probe HITS. A
        /// miss walks less, so hits are the honest side.
        fn probes(&self, plen: usize, want: usize) -> Vec<&str> {
            let step = (self.words.len() / want).max(1);
            self.words
                .iter()
                .step_by(step)
                .filter(|w| w.len() >= plen)
                .map(|w| &w[..plen])
                .collect()
        }

        /// The seek + bounded walk `prefix` performs, without the `DbRef`/`Stores`
        /// plumbing — this measures the TREE, and a collection wrapper would only
        /// add its own constant. Returns the records it would hand back.
        fn query(&self, pre: &str) -> Vec<u32> {
            let pre = pre.as_bytes();
            let probe = |word: u32| bytes_word(pre, word);
            let spec = TextOracle { key: &self.key };
            let mut it =
                rt::rtree_seek(&self.store, self.tree, &probe, pre.len() as u32 * 8, &spec);
            let mut hits = Vec::new();
            let mut rec = it.rec();
            while rec != 0 && hits.len() < CAP {
                if !text_bytes(&self.store, rec, &self.key)
                    .as_bytes()
                    .starts_with(pre)
                {
                    break;
                }
                hits.push(rec);
                rec = it.next(&self.store, self.tree).unwrap_or(0);
            }
            hits
        }

        /// One query's node reads and its results, in one recording window.
        fn query_touch(&self, pre: &str) -> (BTreeSet<u32>, Vec<u32>) {
            rt::touch_begin();
            let hits = self.query(pre);
            (rt::touch_end(), hits)
        }
    }

    /// The node array under five numberings.
    #[test]
    #[ignore = "measurement — run with --release --lib --ignored --nocapture"]
    fn prefix_query_page_span() {
        let Some(f) = build() else { return };
        println!(
            "\nvocabulary {} words | {} nodes | node array {:.2} MB = {} pages of 64 KB",
            f.words.len(),
            f.nodes,
            f.node_bytes() as f64 / (1024.0 * 1024.0),
            f.node_bytes().div_ceil(PAGE)
        );
        let ls = layouts(&f.store, f.tree, f.nodes);
        println!(
            "\n  distinct pages per query, mean (p95)      floor = 1 page\n  {:>6} {:>5} {}",
            "prefix",
            "page",
            row(&ls, |l| format!("{:>13}", l.name))
        );
        for plen in [2usize, 3, 4, 6, 8] {
            let probes = f.probes(plen, 400);
            let touched: Vec<BTreeSet<u32>> = probes
                .iter()
                .map(|p| f.query_touch(p).0)
                .collect::<Vec<_>>();
            let bytes = mean(&touched.iter().map(|t| t.len() * 4).collect::<Vec<_>>());
            for (page, label) in [(PAGE, "64K"), (FINE, "4K")] {
                let cells = row(&ls, |l| {
                    let v: Vec<usize> =
                        touched.iter().map(|t| l.pages(t, page)).collect::<Vec<_>>();
                    format!("{:>8.1} ({:>2})", mean(&v), pct(&v, 0.95))
                });
                println!("  {plen:>4}ch {label:>5}{cells}");
            }
            println!(
                "         {:>5}  {bytes:.0} bytes of node data actually read",
                ""
            );
        }
    }

    /// The records a query RETURNS — a separate allocation with its own layout, and
    /// the half the node measurement says nothing about.
    ///
    /// The counterfactual is the placement `routing` already uses for its postings:
    /// write the records in trie KEY order, so one prefix is one contiguous run.
    #[test]
    #[ignore = "measurement — run with --release --lib --ignored --nocapture"]
    fn record_placement() {
        let Some(f) = build() else { return };
        // Where a key-order pass would put each record: its element record and its
        // string, together, in the order the tree walks them.
        let mut place: HashMap<u32, (u64, u64)> = HashMap::with_capacity(f.words.len());
        let mut at = 0u64;
        let mut it = rt::rtree_first(&f.store, f.tree);
        let mut rec = it.rec();
        while rec != 0 {
            let len =
                u64::from(f.store.record_words(rec) + f.store.record_words(str_of(&f, rec))) * 8;
            place.insert(rec, (at, len));
            at += len;
            rec = it.next(&f.store, f.tree).unwrap_or(0);
        }
        println!(
            "\n  records {} | {:.2} MB packed in key order = {} pages of 64 KB",
            place.len(),
            at as f64 / (1024.0 * 1024.0),
            at.div_ceil(PAGE)
        );
        println!(
            "\n  distinct pages for the {CAP} records a query returns, mean (p95)\n  \
             {:>6} {:>5} {:>13} {:>13}",
            "prefix", "page", "as inserted", "key order"
        );
        for plen in [2usize, 3, 4, 6, 8] {
            let hits: Vec<Vec<u32>> = f
                .probes(plen, 400)
                .iter()
                .map(|p| f.query(p))
                .collect::<Vec<_>>();
            for (page, label) in [(PAGE, "64K"), (FINE, "4K")] {
                let built: Vec<usize> = hits
                    .iter()
                    .map(|h| {
                        let mut pages = BTreeSet::new();
                        for &r in h {
                            let s = str_of(&f, r);
                            add_span(
                                &mut pages,
                                u64::from(r) * 8,
                                u64::from(f.store.record_words(r)) * 8,
                                page,
                            );
                            add_span(
                                &mut pages,
                                u64::from(s) * 8,
                                u64::from(f.store.record_words(s)) * 8,
                                page,
                            );
                        }
                        pages.len()
                    })
                    .collect();
                let keyed: Vec<usize> = hits
                    .iter()
                    .map(|h| {
                        let mut pages = BTreeSet::new();
                        for &r in h {
                            let (off, len) = place[&r];
                            add_span(&mut pages, off, len, page);
                        }
                        pages.len()
                    })
                    .collect();
                println!(
                    "  {plen:>4}ch {label:>5} {:>8.1} ({:>2}) {:>8.1} ({:>2})",
                    mean(&built),
                    pct(&built, 0.95),
                    mean(&keyed),
                    pct(&keyed, 0.95)
                );
            }
        }
    }

    /// One search box session: the keystrokes of a single word, against a reader
    /// that keeps `CACHE_PAGES` pages.
    ///
    /// Every number in the other two measurements is the COLD cost of one query.
    /// A real search box issues a query per keystroke, each a prefix of the last,
    /// and the top of the tree is shared by all of them — so what a user waits for
    /// is the MARGINAL fetch, not the first one.
    #[test]
    #[ignore = "measurement — run with --release --lib --ignored --nocapture"]
    fn warm_cache_session() {
        let Some(f) = build() else { return };
        let ls = layouts(&f.store, f.tree, f.nodes);
        let keys = [1usize, 2, 3, 4, 5, 6, 7, 8];
        println!(
            "\n  pages FETCHED per keystroke, {CACHE_PAGES}-page cache, cold at keystroke 1\n  \
             {:>10}{}",
            "layout",
            row(&keys, |k| format!("{k:>6}"))
        );
        for l in &ls {
            let words: Vec<&String> = f
                .words
                .iter()
                .step_by((f.words.len() / 200).max(1))
                .filter(|w| w.len() >= 8)
                .collect();
            let mut per_key = vec![Vec::new(); keys.len()];
            for w in &words {
                let mut cache = SessionCache::new();
                for (i, &k) in keys.iter().enumerate() {
                    let touched = f.query_touch(&w[..k]).0;
                    let want: BTreeSet<u64> = touched.iter().map(|o| l.moved(*o) / PAGE).collect();
                    per_key[i].push(cache.fetch(&want));
                }
            }
            println!(
                "  {:>10}{}",
                l.name,
                row(&per_key, |v| format!("{:>6.1}", mean(v)))
            );
        }
    }

    /// The string record behind an element record's only field.
    fn str_of(f: &Fixture, rec: u32) -> u32 {
        f.store
            .get_u32_raw(rec, PAYLOAD + u32::from(f.key.position))
    }
}
