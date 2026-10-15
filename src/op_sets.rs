// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — the op-number sets the use analysis consults

//! The op-number sets the use, dead-store and ownership walks consult — each a pure function
//! of the definition NAME table, so [`crate::data::Data::op_sets`] builds them once and caches
//! them; which is why they sit with `Data` in the core layer (@PLN191).
use crate::data::Data;
use crate::fxhash::FxHashSet as HashSet;

pub(crate) fn def_nrs(data: &Data, names: &[&str]) -> HashSet<u32> {
    let mut s = HashSet::default();
    for name in names {
        let nr = data.def_nr(name);
        if nr != u32::MAX {
            s.insert(nr);
        }
    }
    s
}

/// PROJECTION ops return a *reference into* their base container (arg 0) — so the
/// base is accessed in whatever context the projection's RESULT is: a projection
/// inside a write (`OpSetInt(OpGetVector(t,…), …)`) writes `t`; inside a read
/// (`OpGetInt(OpGetVectorNullable(t,…), …)`) reads `t`. They therefore PROPAGATE the
/// incoming context to arg 0 (the remaining args — an index — are pure reads).
///
/// **The membership test is the op's own declaration: `-> reference[arg0]`.** Every
/// element read carries it in both spellings (`OpGetVector` / `OpVectorRef` and their
/// nullable twins), as does the keyed lookup `OpGetRecord` and the field read
/// `OpGetField`. `OpGetDbRef` is the one member without it and is deliberate: it reads a
/// STORED `DbRef` out of a record, which may name any store, and it is here because a
/// closure's `__closure` read must still resolve to the record the caller holds.
///
/// [`Ownership::borrow_base_guarded`] roots a borrow through this set, so a
/// store-preserving read MISSING from it makes the oracle call a view of somebody
/// else's container `Owned` — the over-free direction (loft#1318, where `h[k].v` handed
/// to a fn-ref emptied the caller's hash, and the `sorted` and `index` kinds faulted).
///
/// [`is_projection_op`] asks the narrower ROOT-NAMING question over an overlapping list,
/// and the two are not the same set: this one carries `OpGetDbRef` and both nullable
/// element reads, that one carries neither. Where they differ, say which fact the site
/// wants — `@FR-O-Oracle` for own-vs-borrow, root-naming for which container a view came
/// out of.
pub(crate) fn projection_ops(data: &Data) -> HashSet<u32> {
    def_nrs(
        data,
        &[
            "OpGetVector",
            "OpGetVectorNullable",
            "OpGetField",
            "OpVectorRef",
            "OpVectorRefNullable",
            "OpGetRecord",
            "OpGetDbRef",
        ],
    )
}

/// @PLN90 item 3 — ops that WRITE THROUGH THEIR FIRST ARGUMENT (mirrors the
/// `first_arg_write` set in `parser::find_written_vars`). Used to build `mut_max_pos`,
/// the position-aware "the source is mutated after the copy" fact. `OpCopyRecord` writes
/// its SECOND arg (the dest), handled separately.
pub(crate) fn is_first_arg_write_name(n: &str) -> bool {
    n.starts_with("OpSet")
        || n.starts_with("OpPush")
        || n.starts_with("OpAppendStack")
        || n.starts_with("OpClearStack")
        || matches!(
            n,
            "OpNewRecord"
                | "OpAppendCopy"
                | "OpAppendVector"
                | "OpClearVector"
                | "OpClearKeyed"
                | "OpSetKeyed"
                | "OpHashRemove"
                | "OpInsertVector"
                | "OpRemoveVector"
                | "OpKeepVectorRange"
        )
}

/// LENGTH ops — the collection `len` methods (`t_6vector_len`, `t_6sorted_len`, …).
///
/// They observe how MANY elements a collection has, never what any of them is, so a
/// `len` read cannot witness an element write.  The dead-store lint therefore does not
/// let one discharge its signal: `d = self.data; if i < len(d) { d[i] = x }` is the
/// copy-mutate footgun in full, and counting `len(d)` as "the copy was read" made the
/// lint silent on exactly the shape it exists to catch.  Not hypothetical — the shipped
/// `graphics` canvas is written this way, so every `set_pixel` was a no-op and a
/// `--html` page rendered a blank texture with no diagnostic anywhere.
///
/// The bound guard is not the author's mistake, either: it is the idiom the `v[i]`
/// may-be-null warning ASKS for (skip-pattern 5), so the two lints were in tension —
/// satisfying one silenced the other.
pub(crate) fn is_length_op_name(n: &str) -> bool {
    // Method naming is `t_<LEN><Type>_<method>` (CODE.md), so match the suffix.
    n.starts_with("t_") && n.ends_with("_len")
}

/// VALUE-READER ops return a fresh value, not a reference into a container — their
/// arguments are pure reads regardless of the surrounding context.
pub(crate) fn value_reader_ops(data: &Data) -> HashSet<u32> {
    def_nrs(
        data,
        &[
            "OpGetInt",
            "OpGetByte",
            "OpGetEnum",
            "OpGetSingle",
            "OpGetCharacter",
            "OpLengthVector",
            "t_6vector_len",
        ],
    )
}

/// The op-number sets the use / dead-store / ownership walks consult.
///
/// Every one of them is a pure function of the definition **name** table, so for a
/// given [`Data`] they are constant — yet each walk rebuilt them, and the walks ask
/// once per FUNCTION. Two of the four are a full scan of every definition doing
/// string prefix matches, so the cost is O(functions × definitions): a
/// `println`-sized program rebuilt them **9 000 times over 708 definitions**, which
/// measured ~40 % of a warm-cache startup run (the rebuild itself plus the
/// `HashSet<u32>` inserts and rehashes it drives).
///
/// Cached on [`Data::op_sets`]. That is sound here for the reason it is NOT sound for
/// `function_defs` (loft#854): these derive from def names, which never change once a
/// definition exists, whereas `function_defs` derives from `Definition::code`, which
/// `scopes.rs` rewrites — so a `Data`-lived cache of that serves a stale body after
/// any rewrite. It is the same reason `caller_index` may live on `Data`.
///
/// The sets are `Arc`-shared so a consumer that needs to OWN them (`Uses`) pays a
/// refcount bump instead of a rebuild.
#[derive(Clone)]
pub(crate) struct OpSets {
    /// Projection ops — see [`projection_ops`].
    pub(crate) projections: std::sync::Arc<HashSet<u32>>,
    /// Value-reader ops — see [`value_reader_ops`].
    pub(crate) value_readers: std::sync::Arc<HashSet<u32>>,
    /// Ops writing through arg 0 — see [`is_first_arg_write_name`].
    pub(crate) write_first_arg: std::sync::Arc<HashSet<u32>>,
    /// Collection `len` methods — see [`is_length_op_name`].
    pub(crate) lengths: std::sync::Arc<HashSet<u32>>,
    /// The ops that RELEASE their first argument — one notion, five spellings: the plain
    /// reference free, its tag-checked twin, the text free, and the two witness-guarded
    /// conditional frees.  Every matcher that asks *"is this a free?"* reads THIS set, because
    /// a hand-spelled name list goes blind to a new spelling at once — `check_ref_leaks`
    /// reported a leak the compiler did not have when `OpFreeRefOrHandUp` arrived (loft#1186),
    /// and of the nine lists this replaced no two agreed.  @FR-O-Override's contract is stated
    /// over this notion, not over the one name the rule spells.
    pub(crate) frees: std::sync::Arc<HashSet<u32>>,
    /// The UNCONDITIONAL reference frees — `OpFreeRef` and `OpFreeRefTag`.  A check that asks
    /// *"is this store released here, whatever the run"* reads this one.
    pub(crate) unconditional_ref_frees: std::sync::Arc<HashSet<u32>>,
    /// The witness-GUARDED reference frees — `OpFreeRefIfDistinct` and `OpFreeRefOrHandUp`: a
    /// no-op where the placeholder aliases its witness, so a path walk reads one as CLEARING a
    /// pending free rather than as a free.
    pub(crate) conditional_ref_frees: std::sync::Arc<HashSet<u32>>,
    /// `OpFreeText` — the text release, or `u32::MAX` where the table has none.
    pub(crate) text_free: u32,
    /// The functions a drop site calls — a type's `OpDrop` hook and its cascades
    /// ([`Data::is_drop_function`]).  A release, like the frees above, and never a use of what
    /// it releases.
    pub(crate) drop_functions: std::sync::Arc<HashSet<u32>>,
}

impl OpSets {
    /// Build every set. The two prefix-matched sets share ONE pass over the
    /// definition table — they scanned it separately before.
    pub(crate) fn build(data: &Data) -> Self {
        let mut write_first_arg = HashSet::default();
        let mut lengths = HashSet::default();
        let mut drop_functions = HashSet::default();
        for d in 0..data.definitions() {
            let n = data.def(d).name();
            if is_first_arg_write_name(n) {
                write_first_arg.insert(d);
            }
            if is_length_op_name(n) {
                lengths.insert(d);
            }
            if data.is_drop_function(d) {
                drop_functions.insert(d);
            }
        }
        let nrs = |names: &[&str]| -> HashSet<u32> {
            names
                .iter()
                .map(|n| data.def_nr(n))
                .filter(|&d| d != u32::MAX)
                .collect()
        };
        // `OpFreeRecordIn` is @PLN164 B2's exit free of a PLACED result (`place_result`): the
        // local's record is released, unconditionally, exactly as `OpFreeRef` releases a
        // store — read as one here so the copy lint sees the local die at the exit rather
        // than survive its move.
        let unconditional_ref_frees = nrs(&["OpFreeRef", "OpFreeRefTag", "OpFreeRecordIn"]);
        let conditional_ref_frees = nrs(&["OpFreeRefIfDistinct", "OpFreeRefOrHandUp"]);
        let text_free = data.def_nr("OpFreeText");
        let mut frees: HashSet<u32> = unconditional_ref_frees
            .iter()
            .chain(conditional_ref_frees.iter())
            .copied()
            .collect();
        if text_free != u32::MAX {
            frees.insert(text_free);
        }
        Self {
            projections: std::sync::Arc::new(projection_ops(data)),
            value_readers: std::sync::Arc::new(value_reader_ops(data)),
            write_first_arg: std::sync::Arc::new(write_first_arg),
            lengths: std::sync::Arc::new(lengths),
            frees: std::sync::Arc::new(frees),
            unconditional_ref_frees: std::sync::Arc::new(unconditional_ref_frees),
            conditional_ref_frees: std::sync::Arc::new(conditional_ref_frees),
            text_free,
            drop_functions: std::sync::Arc::new(drop_functions),
        }
    }
}
