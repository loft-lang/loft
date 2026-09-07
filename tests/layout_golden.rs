// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN97 Phase B — the golden layout-conformance test (the instrument).
//
// Pins the EXACT store layout of a corpus of loft structures via the reusable
// `Stores::layout_dump` / `Stores::layout_algo_hash` (src/database/types.rs — the
// same API phases D/F consume): for each type its record `size` and its `Parts`
// descriptor rendered structurally — struct field byte positions, narrow-int
// encodings (#399), and collection element strides (the nested-vector stride
// #477 changed). The store is ONE format (bit-identical in memory and on disk),
// and this layout is a deterministic function of the type table shared by BOTH
// backends — so pinning it here makes any layout change a red diff at commit time
// instead of silently-invalidated data (the gap @PLN97 exists to close).
//
// A layout change is not forbidden — it must be DELIBERATE: the golden diff shows
// exactly what moved, and `LAYOUT_ALGO_HASH` must be re-blessed in the same change
// (that constant is the Phase-D handoff hash).
//
// Regenerate after an intentional layout change:
//   LOFT_BLESS_LAYOUT=1 cargo test --release --test layout_golden layout_golden
// then hand-verify the diff and update LAYOUT_ALGO_HASH to the printed value.

mod common;
extern crate loft;

use common::cached_default;
use loft::data::Data;
use loft::database::{Parts, Stores};
use loft::parser::Parser;
use std::path::PathBuf;

/// The corpus — one structure per representative layout cell. Closure (fn-ref
/// field → DbRef/ChildRec), Radix (1.1+), and Sorted/Short (a local / nullable
/// narrow) stay uncovered — the coverage audit classifies them (Phase B4).
const CORPUS: &str = r#"
struct Scalars { b: boolean, c: character, s: single, f: float, i: integer, t: text }
struct Narrow { a: i32, b: u8, c: u16 }
struct Unsigned { a: u32, b: u32?, v: vector<u32> }
struct Wide { s: i16 }
struct NotNull { i: integer, t: text }
struct Nullable { n: integer?, f: float? }
struct NarrowRange { a: integer limit(10,255), b: integer limit(10,1000), c: integer limit(10,255)?, v: vector<integer limit(10,255)> }
struct Vec1 { v: vector<integer> }
struct VecNest { vv: vector<vector<integer>> }
struct VecNestNarrow { vv: vector<vector<u8>> }
struct VecText { v: vector<text> }
struct Item { ik: integer }
struct Bag { items: hash<Item[ik]> }
struct SortedBag { items: sorted<Item[ik]> }
struct IndexBag { items: index<Item[ik]> }
struct RefHost { child: Scalars }
struct Tup { pair: (integer, text) }
enum Color { Red, Green, Blue }
enum Shape { Circle { radius: integer }, Rect { width: integer, height: integer } }
"#;

/// The corpus roots — the closure over their referenced types is what the golden
/// pins and the audit classifies.
const TYPES: &[&str] = &[
    "Scalars",
    "Narrow",
    "Unsigned",
    "Wide",
    "NotNull",
    "Nullable",
    "NarrowRange",
    "Vec1",
    "VecNest",
    "VecNestNarrow",
    "VecText",
    "Item",
    "Bag",
    "SortedBag",
    "IndexBag",
    "RefHost",
    "Tup",
    "Color",
    "Shape",
];

/// A layout change flips this. Re-bless (with the golden) on an intentional change.
/// (@PLN97 F9 2026-07-17 — re-blessed after adding the `@endian` line to the layout dump.
/// @PLN102 arc-E F9 2026-07-19 — re-blessed after adding the `Nullable` corpus type +
/// folding the DEF-level nullability schema into the golden.
/// 2026-07-24 — re-blessed after adding `VecNestNarrow`: a nested vector with a NARROW
/// inner is the shape the #477/#483/#624 class keeps recurring in, and the corpus could
/// not see it.  `vector<vector<integer>>` alone is layout-UNCHANGED by that fix, so
/// only the added row moves the hash — see doc/claude/plans/nested-narrow-width/.
/// 2026-08-09 — re-blessed for @PLN135 arc H: `hash<Item[ik]>` gained `placement=2`,
/// which is Q2's mechanism doing exactly what it was built for.  ONE line of the
/// golden moved and no other type's row changed, because the token renders per KIND —
/// a store of plain structs and vectors still loads, and a pre-arena store of hashes
/// is now REFUSED instead of misread.
/// 2026-08-20 — re-blessed for loft#1036: added `NarrowRange`, the bare `limit(lo, hi)`
/// spelling of a narrow integer, as a FIELD (1- and 2-byte) and as a VECTOR ELEMENT.
/// The corpus could only see the `size(N)` ALIAS spelling (`u8`/`u16`/`i32`), so it could
/// not see that the two spellings of one range had drifted apart: `(L-Narrow)` narrows on
/// the RANGE, but the element stride + schema were keyed on `forced_size`, leaving
/// `vector<integer limit(10,255)>` at the wide 8-byte stride while its READ decoded 1
/// byte + `min`.  Only the added rows move the hash — every pre-existing row is
/// unchanged, which is the claim "the alias spelling did not move" made checkable.
/// 2026-09-07 — re-blessed for loft#1437: added `Unsigned`, a `u32` field, a nullable one and
/// a `u32` ELEMENT.  The corpus carried `i32`, `u8` and `u16` but no `u32` — the one narrow
/// width whose ENCODING its width does not decide — so `(L-Narrow-Enc)` had no row at all.
/// That is a fact a layout dump CAN carry, because the row names the Part, while a value
/// comparison cannot see it.  Only the added rows move the hash.)
const LAYOUT_ALGO_HASH: u64 = 2_090_453_648_790_136_790;

/// @PLN135 Q2 — `keys::key_hash` for a fixed seed over a fixed key set: the function a
/// reader must reproduce to find an entry a writer placed. Pinned by
/// `placement_contract_is_pinned`, which explains the two ways it can move. Re-bless only
/// after deciding WHICH of the two happened — they need opposite responses.
const PLACEMENT_KEY_HASHES: [u64; 5] = [
    17_666_971_441_118_593_204,
    1_462_129_204_271_929_792,
    16_094_394_784_318_215_136,
    3_950_772_723_845_657_195,
    1_729_413_894_695_066_270,
];

/// @PLN102 arc-E flip-gate (Gate 1 step 3) — the `contract` version at which the
/// CURRENT layout was frozen. The store layout IS the persistence contract, so
/// POST-FLIP a change to `LAYOUT_ALGO_HASH` / the golden may land only alongside a
/// `CONTRACT_VERSION` bump (a declared, epoch-style break) — enforced git-side by
/// `scripts/check_contract_goldens.sh`. When you re-bless the layout at
/// `CONTRACT_VERSION > 0`, set this to the new `CONTRACT_VERSION` too. INERT while
/// `CONTRACT_VERSION == 0` (pre-freeze: the language is still settling, layout
/// changes are free). Invariant: `LAYOUT_CONTRACT <= CONTRACT_VERSION` always — a
/// layout cannot be frozen at a contract the runtime has not reached.
const LAYOUT_CONTRACT: u32 = 0;

/// The corpus roots resolved to known_types (loudly — a parse / syntax drift fails).
fn corpus_roots(data: &Data) -> Vec<u16> {
    TYPES
        .iter()
        .map(|name| {
            let kt = data.def(data.def_nr(name)).known_type;
            assert!(
                kt != u16::MAX,
                "corpus type `{name}` did not resolve to a known_type — parse failed or syntax drifted"
            );
            kt
        })
        .collect()
}

fn golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/golden/layout/corpus.txt")
}

#[test]
fn layout_golden() {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);

    let roots = corpus_roots(&p.data);
    let dump = p.database.layout_dump(&roots);
    // @PLN102 arc-E F9 — fold the DEF-level nullability schema into the golden so a
    // full-width `τ` → `τ?` flip (byte-identical `dump`, so invisible to the hash) is
    // ALSO a red diff here + a contract-goldens gate trip. `Nullable` in the corpus
    // makes the pin non-trivial.
    let schema = loft::schema_sidecar::nullability_schema(&p.data, &roots);
    let golden = format!("{dump}=== F9 nullability ===\n{schema}\n");
    let path = golden_path();

    if std::env::var("LOFT_BLESS_LAYOUT").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, &golden).unwrap();
        eprintln!(
            "blessed {} ; LAYOUT_ALGO_HASH = {}",
            path.display(),
            p.database.layout_algo_hash(&roots)
        );
        eprintln!(
            "@PLN102 flip-gate: at CONTRACT_VERSION={}, a layout re-bless is a \
             persistence-contract change — post-freeze, bump CONTRACT_VERSION and set \
             LAYOUT_CONTRACT = CONTRACT_VERSION (inert at 0).",
            loft::manifest::CONTRACT_VERSION,
        );
        return;
    }

    let expected = std::fs::read_to_string(&path).unwrap_or_else(|_| {
        panic!(
            "missing golden {} — regenerate with LOFT_BLESS_LAYOUT=1",
            path.display()
        )
    });
    assert_eq!(
        golden, expected,
        "\nSTORE LAYOUT or F9 NULLABILITY CHANGED. If intentional: re-bless \
         (LOFT_BLESS_LAYOUT=1), hand-verify the diff, and update LAYOUT_ALGO_HASH. If \
         not: a layout regression (the #477 class) or a silent `τ`↔`τ?` reshape just \
         got caught.\n"
    );
    assert_eq!(
        p.database.layout_algo_hash(&roots),
        LAYOUT_ALGO_HASH,
        "layout-algo hash drifted from the pinned constant"
    );
}

/// @PLN102 arc-E flip-gate (Gate 1 step 3) — the layout is frozen at `LAYOUT_CONTRACT`;
/// the running `CONTRACT_VERSION` can never be BELOW it (a layout cannot be frozen at a
/// contract the runtime has not reached). This pins the in-tree half of the coupling;
/// the "a layout change requires a contract bump" half is git-diff-shaped and lives in
/// `scripts/check_contract_goldens.sh` (also inert while `CONTRACT_VERSION == 0`). When
/// the two are equal the layout is frozen at the current contract — the normal state.
#[test]
fn layout_contract_pin_is_consistent() {
    // `black_box` so the check reads as the runtime comparison it becomes once the two
    // diverge (post-flip) — not a const-folded tautology clippy would flag while both
    // are 0 (inert). The invariant is real: a re-bless at CONTRACT_VERSION>0 must raise
    // LAYOUT_CONTRACT to match, so LAYOUT_CONTRACT can never exceed the running contract.
    let frozen_at = std::hint::black_box(LAYOUT_CONTRACT);
    let running = std::hint::black_box(loft::manifest::CONTRACT_VERSION);
    assert!(
        frozen_at <= running,
        "LAYOUT_CONTRACT ({frozen_at}) exceeds CONTRACT_VERSION ({running}) — a layout \
         cannot be frozen at a contract the runtime has not reached; re-bless the layout \
         (LOFT_BLESS_LAYOUT=1) and set LAYOUT_CONTRACT = CONTRACT_VERSION",
    );
}

// ── B4 — coverage self-audit ────────────────────────────────────────────────
//
// Guarantees "every structure" stays true as loft grows. Three teeth:
//  1. `coverage()` is EXHAUSTIVE over `Parts` — a NEW storage kind fails to
//     COMPILE here, forcing whoever adds it to declare its layout-test coverage.
//  2. Every kind the corpus produces must be classified `Covered` (a ratchet:
//     add a corpus entry that produces a `Gap` kind → this fails → promote it).
//  3. The corpus produces EXACTLY the `Covered` kinds (delete a corpus entry
//     that drops a covered kind → this fails).

/// The types a type's layout references — the closure the audit classifies.
/// (`Stores::layout_dump`/`layout_algo_hash` carry the same walk internally.)
fn referenced(db: &Stores, kt: u16, out: &mut Vec<u16>) {
    match &db.types[kt as usize].parts {
        Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
            out.extend(fields.iter().map(|f| f.content));
        }
        Parts::Vector(e) | Parts::Array(e) => out.push(*e),
        Parts::Sorted(e, _)
        | Parts::Ordered(e, _)
        | Parts::Hash(e, _)
        | Parts::Index(e, _, _)
        | Parts::Radix(e, _) => out.push(*e),
        Parts::ChildRec(c) => out.push(*c),
        // A plain variant (no data) keeps `known_type == u16::MAX`; only
        // data-carrying variants have an `EnumValue` type to reach.
        Parts::Enum(vs) => out.extend(vs.iter().map(|(t, _)| *t).filter(|t| *t != u16::MAX)),
        _ => {}
    }
}

fn reached_types(db: &Stores, data: &Data) -> std::collections::BTreeSet<u16> {
    let mut seen: std::collections::BTreeSet<u16> = std::collections::BTreeSet::new();
    let mut stack: Vec<u16> = corpus_roots(data);
    while let Some(kt) = stack.pop() {
        if !seen.insert(kt) {
            continue;
        }
        let mut refs = Vec::new();
        referenced(db, kt, &mut refs);
        for r in refs {
            if !seen.contains(&r) {
                stack.push(r);
            }
        }
    }
    seen
}

#[derive(PartialEq, Eq, Debug)]
enum Cover {
    /// Exercised by the corpus — its layout is pinned by the golden.
    Covered,
    /// A real user-writable storage kind NOT yet in the corpus — add it.
    Gap(&'static str),
    /// Not a user-declared shape (codegen-only) — no corpus entry expected.
    Internal(&'static str),
}

/// EXHAUSTIVE over `Parts`. A new variant makes this a non-exhaustive match =
/// a compile error, so no storage kind can be added without a coverage verdict.
fn coverage(p: &Parts) -> (&'static str, Cover) {
    match p {
        Parts::Base => ("Base", Cover::Covered),
        Parts::Struct(_) => ("Struct", Cover::Covered),
        Parts::Byte(..) => ("Byte", Cover::Covered),
        Parts::ShortRaw(..) => ("ShortRaw", Cover::Covered),
        Parts::Int(..) => ("Int", Cover::Covered),
        Parts::IntRaw(..) => ("IntRaw", Cover::Covered),
        Parts::Vector(_) => ("Vector", Cover::Covered),
        Parts::Hash(..) => ("Hash", Cover::Covered),
        Parts::Enum(_) => ("Enum", Cover::Covered),
        Parts::EnumValue(..) => ("EnumValue", Cover::Covered),
        Parts::Index(..) => ("Index", Cover::Covered),
        // The trie kind exists at the schema level (step 2 of
        // doc/claude/plans/text-keyed-trie.md) but cannot be CONSTRUCTED until the
        // keyword lands (step 6), so there is nothing in the corpus to cover it yet.
        // This gate is what forced the verdict, which is the mechanism working.
        Parts::Trie(..) => (
            "Trie",
            Cover::Gap("no keyword yet — text-keyed-trie plan step 6"),
        ),
        // `sorted<T[k]>` as a struct field is array-backed → Ordered.
        Parts::Ordered(..) => ("Ordered", Cover::Covered),
        // The vector-backed `sorted` (Parts::Sorted) needs a local `sorted<T[k]>=[]`,
        // not a struct field — not in the corpus yet.
        Parts::Sorted(..) => (
            "Sorted",
            Cover::Gap("vector-backed sorted — a local, not a field"),
        ),
        // The 2-byte SHIFTED narrow int (distinct from ShortRaw); a nullable
        // narrow field, not produced by i16/u16 (those are ShortRaw).
        Parts::Short(..) => (
            "Short",
            Cover::Gap("2-byte shifted narrow int — nullable narrow field"),
        ),
        Parts::Radix(..) => (
            "Radix",
            Cover::Gap("spatial<T[key]> — planned 1.1+, errors today"),
        ),
        Parts::Array(_) => (
            "Array",
            Cover::Internal("codegen-only reference collection"),
        ),
        Parts::DbRef => (
            "DbRef",
            Cover::Internal("12B stored DbRef — fn-ref closure half"),
        ),
        Parts::ChildRec(_) => (
            "ChildRec",
            Cover::Internal("closure-in-struct-field codegen"),
        ),
    }
}

/// The storage kinds the corpus is expected to produce — kept in lockstep with
/// the `Cover::Covered` arms above by the audit's exact-set assertion.
const COVERED_LABELS: &[&str] = &[
    "Base",
    "Struct",
    "Byte",
    "ShortRaw",
    "Int",
    "IntRaw",
    "Vector",
    "Hash",
    "Ordered",
    "Index",
    "Enum",
    "EnumValue",
];

#[test]
fn layout_coverage_audit() {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);

    let mut produced: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for kt in reached_types(&p.database, &p.data) {
        let (label, cover) = coverage(&p.database.types[kt as usize].parts);
        assert_eq!(
            cover,
            Cover::Covered,
            "corpus now produces storage kind `{label}` (classified {cover:?}) — a gap just \
             closed: promote it to Cover::Covered and add it to COVERED_LABELS"
        );
        produced.insert(label);
    }

    let expected: std::collections::BTreeSet<&'static str> =
        COVERED_LABELS.iter().copied().collect();
    assert_eq!(
        produced, expected,
        "corpus storage-kind coverage drifted: the corpus must produce EXACTLY the \
         Cover::Covered kinds. Missing → a corpus structure was removed; extra → promote \
         the new kind in coverage() + COVERED_LABELS."
    );
}

// ── Phase D — the schema-description sidecar (self-describing store) ─────────

/// The in-memory layout identity built over the real corpus round-trips through
/// the `.dschema` sidecar text, carries the same pinned hash the golden asserts,
/// and an UNCHANGED store hands over raw (`Handoff::Identical`).
#[test]
fn schema_sidecar_identity_end_to_end() {
    use loft::schema_sidecar::{Handoff, LayoutIdentity, classify};

    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);
    let roots = corpus_roots(&p.data);

    let id = LayoutIdentity::of(&p.database, &roots);
    assert_eq!(
        id.layout_hash, LAYOUT_ALGO_HASH,
        "sidecar identity hash must match the golden's pinned layout hash"
    );
    assert_eq!(
        LayoutIdentity::from_sidecar(&id.to_sidecar()),
        Some(id.clone()),
        "sidecar text must round-trip"
    );
    assert_eq!(
        classify(&id, &id),
        Handoff::Identical,
        "an unchanged store hands over raw"
    );
}

/// `program_roots` (Phase F) returns exactly the user-defined struct/enum types
/// — the same set the corpus roots name — excluding stdlib and enum variants.
#[test]
fn program_roots_are_the_user_types() {
    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);

    let roots: std::collections::BTreeSet<u16> = loft::schema_sidecar::program_roots(&p.data)
        .into_iter()
        .collect();
    let expected: std::collections::BTreeSet<u16> = corpus_roots(&p.data).into_iter().collect();
    let name = |kt: u16| p.database.types[kt as usize].name.clone();
    let extra: Vec<String> = roots.difference(&expected).map(|k| name(*k)).collect();
    let missing: Vec<String> = expected.difference(&roots).map(|k| name(*k)).collect();
    assert_eq!(
        roots, expected,
        "program_roots must be exactly the corpus's user struct/enum types.\n  \
         extra: {extra:?}\n  missing: {missing:?}"
    );
}

/// @PLN135 Q2 — the on-disk PLACEMENT contract is pinned, so it cannot change quietly.
///
/// The @PLN97 layout identity commits to how a store's bytes are SHAPED. It says nothing
/// about where a keyed collection puts an entry, and `loft::placement` closes that gap by
/// carrying a per-kind token into the same identity. A token only helps if somebody
/// remembers to bump it, which is what this pins: every fact below is one a reader has to
/// reproduce to find an entry a writer placed, so changing one without bumping the token
/// would let an older store pass the gate and then be MISREAD — the silent wrong answer
/// the compatibility doctrine rules out.
///
/// What it covers, against the arcs actually in flight: arc D widens a bucket slot, arc H
/// replaces the bucket record outright — both move the constants. Arc E changes the hash
/// function and the index derivation — that moves the digest. What it does NOT cover is
/// the probe ORDER (linear, forward) and the `elms = (room - 2) * 2` slot-count rule;
/// those live in the module doc of `loft::placement` as things to bump for, and a change
/// to either would need this test extended rather than merely re-blessed.
///
/// It also catches something the placement token CANNOT (loft#827). `key_hash` runs on
/// `std::hash::DefaultHasher`, whose algorithm std explicitly does not guarantee across
/// releases, so a Rust upgrade can move bucket placement with no loft change at all — and
/// a token nobody bumped cannot refuse the resulting store. This pin is the only thing
/// that would notice, which is why its failure message asks WHICH cause it is before
/// anyone re-blesses it.
#[test]
fn placement_contract_is_pinned() {
    use loft::keys::{Content, Str};

    const BUMP: &str = "the on-disk bucket placement changed. If loft changed: bump \
                        `placement::HASH` in src/placement.rs so a store written by an \
                        older binary is REFUSED instead of misread, then re-bless this \
                        pin. If ONLY the Rust toolchain changed: this is loft#827 — \
                        `keys::key_hash` runs on `std::hash::DefaultHasher`, whose \
                        algorithm std does not guarantee across releases, so bucket \
                        placement moved with no loft change and nothing can refuse the \
                        old store. Do NOT simply re-bless that; see the issue";

    // The bucket record's shape: a size header word, a live-count field, a 64-bit seed,
    // the entry arena's four bookkeeping words, then `u32` ENTRY INDICES. A reader
    // derives every slot address from these, and decodes a slot through the arena.
    assert_eq!(loft::hash::LEN_FLD, 4, "{BUMP}");
    assert_eq!(loft::hash::SEED_FLD, 8, "{BUMP}");
    assert_eq!(loft::arena::DIR_FLD, 16, "{BUMP}");
    assert_eq!(loft::arena::NEXT_FLD, 20, "{BUMP}");
    assert_eq!(loft::arena::FREE_FLD, 24, "{BUMP}");
    assert_eq!(loft::hash::STRIDE_FLD, 28, "{BUMP}");
    assert_eq!(loft::hash::BUCKET0, 32, "{BUMP}");
    assert_eq!(loft::hash::RESERVED_WORDS, 4, "{BUMP}");
    assert_eq!(loft::hash::SLOT_BYTES, 4, "{BUMP}");
    // The high bit that says a slot names a RECORD rather than an arena index. THREE
    // readers decode a bucket slot — `hash::entry_ref`, the bounds-checked walk in
    // `Stores::validate_claims`, and `paged_reader::entry_at` — and a fourth is a
    // matter of time. Pinning it here is what makes a decoder that does not know
    // about it a red test rather than an entry silently read as a chunk that does
    // not exist.
    assert_eq!(loft::hash::SLOT_RECORD, 0x8000_0000, "{BUMP}");
    // The arena's own geometry: a slot's address is a pure function of its index, so
    // these three ARE where an entry lives.  A reader that tiles the index space
    // differently finds an entry at a byte offset nothing put one at.
    assert_eq!(loft::arena::BASE, 64, "{BUMP}");
    assert_eq!(loft::arena::SLOT0, 8, "{BUMP}");
    assert_eq!(loft::arena::DIR0, 8, "{BUMP}");
    assert_eq!(loft::arena::locate(1, 16), (0, 8), "{BUMP}");
    assert_eq!(loft::arena::locate(64, 16), (0, 8 + 63 * 16), "{BUMP}");
    assert_eq!(loft::arena::locate(65, 16), (1, 8), "{BUMP}");
    assert_eq!(loft::arena::locate(193, 16), (2, 8), "{BUMP}");

    // The hash function and the `Content` encoding that feeds it. A fixed seed makes this
    // a pure function of the key, so any change to either — a different construction, a
    // different byte order, a different widening — moves the digest.
    let seed: u64 = 0x0123_4567_89ab_cdef;
    let digest: Vec<u64> = [
        Content::Long(0),
        Content::Long(1),
        Content::Long(-1),
        Content::Long(i64::from(i32::MAX)),
        Content::Str(Str::new("loft")),
    ]
    .iter()
    .map(|c| loft::keys::key_hash(std::slice::from_ref(c), seed))
    .collect();
    assert_eq!(digest, PLACEMENT_KEY_HASHES.to_vec(), "{BUMP}");

    // A multi-field key hashes as the whole key, not as its first element — the property
    // a compound-keyed store depends on for placement.
    let compound = loft::keys::key_hash(&[Content::Long(7), Content::Str(Str::new("x"))], seed);
    assert_ne!(
        compound,
        loft::keys::key_hash(&[Content::Long(7)], seed),
        "{BUMP}"
    );
}

/// A placement bump must actually REFUSE an older store, not merely differ.
///
/// This is the other half of the mechanism: `placement::tag` feeds `layout_dump`, which
/// feeds `layout_algo_hash`, which is what the `.dschema` sidecar records and
/// `Stores::schema_gate_ok` compares on every `store_load`. Here the sidecar of a store
/// containing a hash is edited to carry a different placement token — exactly what a
/// pre-arc-H store would look like to a post-arc-H binary — and the verdict must be a
/// refusal that routes to `SchemaMismatch`, never a raw handoff.
#[test]
fn a_changed_placement_token_refuses_the_store() {
    use loft::schema_sidecar::{LayoutIdentity, SchemaVerdict, verdict_for_sidecar_text};

    let (data, db) = cached_default();
    let mut p = Parser::new();
    p.data = data;
    p.database = db;
    p.parse_str(CORPUS, "layout_corpus", false);
    let roots = corpus_roots(&p.data);
    let current = LayoutIdentity::of(&p.database, &roots);

    let text = current.to_sidecar();
    assert!(
        text.contains("hash<"),
        "the corpus must contain a hash for this test to mean anything"
    );
    assert!(
        matches!(
            verdict_for_sidecar_text(&text, &current),
            SchemaVerdict::Match
        ),
        "an untouched sidecar hands over raw"
    );

    // What a store written before a placement bump looks like to a binary after one.
    let older = text.replace("hash<", "hash|PLACEMENT|<");
    let verdict = verdict_for_sidecar_text(&older, &current);
    assert!(
        !verdict.is_raw_safe(),
        "a store whose hash placement this binary would compute differently must not be \
         read raw — it would be misread, not merely mismatched"
    );
    assert!(
        verdict.as_corrupt_reason().is_some(),
        "the refusal must route through the store's corruption path so the reader is told"
    );
}
