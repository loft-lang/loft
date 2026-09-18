// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
// loft#885 — loop-invariant vector headers

//! Which vectors a loop may read through a header derived once, instead of re-deriving
//! it per element (loft#885).
//!
//! Reading `v[i]` resolves the store, loads the container slot and loads the length, and
//! every one of those loads is guarded — so LLVM will not lift them out of the loop even
//! with the whole chain inlined (PERFORMANCE.md § Native vs Rust root cause 3c). The
//! emitter can lift them itself, because it is the one that knows where the loop is; what
//! it needs from here is the promise that makes lifting sound: **nothing the loop body
//! runs can write a store.**
//!
//! A write is what moves a vector: `Store::resize` relocates the record it grows, and
//! `remove` rewrites the length in place. A write to some *other* record cannot reach ours
//! — `claim` takes free space, `delete` touches only free blocks, and reallocating a
//! store's backing buffer leaves record numbers (word offsets into it) unchanged. So the
//! question is not "did anything allocate" but "could anything have written *this* vector",
//! and a body that writes no store at all answers it for every vector at once.
//!
//! [`writes_store`] answers that from an ALLOW-list of ops that provably do not write, so
//! an op missing from it costs the optimisation and never correctness — the inverse of the
//! deny-lists in PERFORMANCE.md § Design: P8, where an omission is a silent wrong read.
//! `LOFT_HOIST_VERIFY=1` is the second half: it emits the checking form of every hoisted
//! read, which re-derives the header and panics on a mismatch, so a hole in the allow-list
//! shows up as a failure under one suite run.

use crate::data::{Block, Data, DefType, Type, Value};
use crate::database::Stores;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// The two ops that turn `(vector, index)` into the address of an element. A hoisted
/// header replaces the store resolution + slot load + length load in both.
pub const ELEMENT_ADDRESS_OPS: [&str; 2] = ["OpGetVector", "OpGetVectorNullable"];

/// Zero-parameter ops that produce a constant.
///
/// They have to be named, because the structural rule below requires a parameter to read:
/// an op with no parameters at all is either a constant like these or something that
/// reaches state through the frame (`OpParallelJoin`), and the signature cannot tell them
/// apart. `OpConvIntFromNull` is the one that matters in practice — it initialises the
/// index of a `for` loop, so a nested loop carries it inside its parent's body.
const PURE_NULLARY_OPS: [&str; 15] = [
    "OpConvIntFromNull",
    "OpConvBoolFromNull",
    "OpConvCharacterFromNull",
    "OpConvSingleFromNull",
    "OpConvFloatFromNull",
    "OpConvTextFromNull",
    "OpConvEnumFromNull",
    "OpConvRefFromNull",
    "OpNullRefSentinel",
    "OpConstTrue",
    "OpConstFalse",
    "OpMathPiFloat",
    "OpMathEFloat",
    // The libm dispatchers (@PLN157 P4a): pure scalar math whose `const` first
    // parameter is a FUNCTION SELECTOR (`9` = sqrt), not a slot or type id — the
    // one const-param shape the signature rule's rationale does not cover.  Left
    // off this list they read as writers, and a loop calling `sin`/`sqrt`
    // through `seed_wave`-style helpers never hoisted at all.
    "OpMathFuncFloat",
    "OpMathFuncSingle",
];

/// Ops that take a collection or a reference and only READ it.
///
/// Every other op that can name a store is assumed to write one. That is the safe
/// direction: a reader left out of this list only means a loop that keeps re-deriving its
/// headers. Add to it when a loop that should hoist does not — never to make a loop hoist
/// that a measurement said was slow.
const READ_ONLY_COLLECTION_OPS: [&str; 24] = [
    // the reference's own identity — `store_nr`/`rec` tests that touch no store at all
    // (@PLN157 § V-c: the R1 guard put `OpRefIsNull` in every buffer-building body), and
    // the copy of one (@PLN164 B1b: the entry witness snapshots every promoted buffer)
    "OpRefIsNull",
    "OpConvBoolFromRef",
    "OpDistinctStore",
    "OpRefAlias",
    // element address + length: the vector reads themselves
    "OpGetVector",
    "OpGetVectorNullable",
    "OpLengthVector",
    // typed field reads through a reference
    "OpGetInt",
    "OpGetInt4",
    "OpGetInt4Raw",
    "OpGetInt4Full",
    "OpGetShort",
    "OpGetShortRaw",
    "OpGetShortFull",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetSingle",
    "OpGetFloat",
    "OpGetBoolean",
    "OpGetCharacter",
    "OpGetEnum",
    "OpGetRef",
    "OpGetField",
    "OpGetDbRef",
];

/// The scalar in-place setters (@PLN157 P4a): each writes one fixed-width value through
/// an address it is GIVEN — the `set_*` family in `Store` — and cannot resize, insert,
/// remove or re-key anything.  No record moves and no collection's length changes, so
/// every hoisted [`crate::vector::VecHeader`] stays valid across one, aliased or not: an
/// in-place write moves nothing, so there is nothing an alias could observe stale.
/// (Hoisted scalar VALUES would be a different question — offset-keyed invalidation —
/// but this pass hoists headers only.)  Deliberately scalar-only: `OpSetRef`/`OpSetDbRef`
/// hand records to owners, `OpSetText` re-allocates, `OpSetKeyed` re-keys — excluded,
/// and the allow-list doctrine holds: an op missing here costs the hoist, never
/// correctness.
/// Enforces `@FR-R-InPlace` (formal/rewrites.md): the allow-list of fixed-width scalar sets.
pub const IN_PLACE_SET_OPS: [&str; 12] = [
    "OpSetBoolean",
    "OpSetInt",
    "OpSetInt4",
    "OpSetInt4Raw",
    "OpSetCharacter",
    "OpSetSingle",
    "OpSetFloat",
    "OpSetByte",
    "OpSetByteNullable",
    "OpSetShort",
    "OpSetShortRaw",
    "OpSetEnum",
];

/// A hoist key (@PLN157 P4d): a vector reached from a local through zero or more CONST
/// field offsets — `v` is `(var, [])`, `lay.best` is `(var, [8])`.  Pure by
/// construction (`OpGetField` is a reader), so the prelude may evaluate the path once
/// and a fused fallback may re-emit it; the bare-`Var` restriction this lifts only ever
/// guarded IMPURE operands, which cannot form a path.
pub type PathKey = (u16, Vec<i64>);

/// The path for a vector operand, or `None` when it is not a `Var` or a
/// `OpGetField(path, const fld, const tp)` chain over one.
#[must_use]
/// Enforces `@FR-R-Header`: the pure path a header is keyed on.
pub fn vector_path(data: &Data, v: &Value) -> Option<PathKey> {
    match v.unspan() {
        Value::Var(var) => Some((*var, Vec::new())),
        Value::Call(d, args)
            if args.len() == 3
                && (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpGetField" =>
        {
            let Value::Int(off) = args[1].unspan() else {
                return None;
            };
            let (root, mut offs) = vector_path(data, &args[0])?;
            offs.push(i64::from(*off));
            Some((root, offs))
        }
        _ => None,
    }
}

/// The vector paths whose header `body` may derive once up front, each with a clone of
/// the operand expression the prelude evaluates.
///
/// Empty when anything in the loop could write a store — except, when `allow_in_place`
/// (the @PLN157 P4a tier, off under `LOFT_NO_WRITE_HOIST=1`), the [`IN_PLACE_SET_OPS`],
/// which cannot invalidate a header — when the body rebinds the path's ROOT variable, or
/// when nothing indexes a vector at all. Order is the order the accesses appear in, so
/// the generated prelude is stable across runs.
pub fn hoistable_vectors(
    body: &Block,
    data: &Data,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
) -> Vec<(PathKey, Value)> {
    let tiers = HoistTiers {
        in_place: allow_in_place,
        ..HoistTiers::default()
    };
    if body_blocks_hoist(body, data, def_nr, cache, tiers) {
        return Vec::new();
    }
    vector_candidates(body, data, def_nr)
}

/// The generation-time tiers of the hoist family — one flag per admitted rewrite, each an
/// `LOFT_NO_*` environment switch read once by `Output::new`.  Grouped so the gate, the
/// collector and the emitter read ONE value and cannot disagree about which tiers are on.
// Four bools by design: each IS one independently switchable rewrite tier (R-Switch), and a
// substructure would only put a name between a switch and its rule.
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone, Copy, Default)]
pub struct HoistTiers {
    /// `(R-InPlace)` — in-place scalar sets admitted (`LOFT_NO_WRITE_HOIST` off).
    pub in_place: bool,
    /// `(R-Scalar)` — record scalars hoisted (`LOFT_NO_SCALAR_HOIST` off).
    pub scalars: bool,
    /// `(R-Push)` — fusable pushes admitted under a push header (`LOFT_NO_PUSH_HOIST` off).
    pub push: bool,
    /// `(R-Mint)` — the record mint group admitted as a mover (`LOFT_NO_MINT_HOIST` off).
    pub mint: bool,
    /// `(R-PushRec)` — an admitted mint whose element is a plain no-heap struct EMITS
    /// through a push header of its own (`LOFT_NO_RECORD_PUSH` off).  A refinement of
    /// `mint`: it changes what the group's ops emit, never whether the loop hoists.
    pub record_push: bool,
}

/// Does anything in `body` invalidate a hoisted header?  The ONE gate both the vector
/// headers and the scalar hoist (@PLN157 P4c) stand behind: a scalar hoist is admitted only
/// in a loop whose store writes are all in place, so the two cannot disagree about which
/// loops qualify.
/// Enforces `@FR-R-InPlace` as the ONE gate `@FR-R-Header` and `@FR-R-Scalar` stand behind.
fn body_blocks_hoist(
    body: &Block,
    data: &Data,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    tiers: HoistTiers,
) -> bool {
    let mut fresh: HashSet<u16> = HashSet::new();
    body.operators.iter().any(|op| {
        blocks_header_hoist(
            op,
            data,
            cache,
            &mut HashSet::new(),
            Some(data.def(def_nr).variables()),
            tiers,
            &mut fresh,
        )
    })
}

/// The variables `body` rebinds — a `Set` or a `TuplePut` anywhere in it.  A rebind leaves
/// the store untouched and still invalidates anything hoisted off that variable, because the
/// hoist describes what the variable named on the way in.
fn rebound_vars(body: &Block) -> HashSet<u16> {
    let mut rebound: HashSet<u16> = HashSet::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Set(v, _) | Value::TuplePut(v, _, _) = n {
                rebound.insert(*v);
            }
            false
        });
    }
    rebound
}

/// The vector paths `body` indexes, in the order they appear, minus those whose root the
/// body rebinds (a field path's header describes the record the ROOT named; repointing the
/// field itself is an `OpSetRef`, which is not in [`IN_PLACE_SET_OPS`] and blocks outright).
/// Enforces `@FR-R-Header`: which paths a loop derives a header for.
fn vector_candidates(body: &Block, data: &Data, def_nr: u32) -> Vec<(PathKey, Value)> {
    let rebound = rebound_vars(body);
    let mut found: Vec<(PathKey, Value)> = Vec::new();
    let vars = data.def(def_nr).variables();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && args.len() == 3
                && is_element_address(data, *d)
                && let Some(path) = vector_path(data, &args[0])
            {
                // A bare var must TYPE as a vector (an odd non-vector shape stays out); a
                // field path is shape-trusted — it is the first argument of an
                // element-address op, which takes a vector.
                let vector_typed =
                    !path.1.is_empty() || matches!(vars.tp(path.0).base(), Type::Vector(_, _));
                if vector_typed && !found.iter().any(|(p, _)| *p == path) {
                    found.push((path, args[0].clone()));
                }
            }
            false
        });
    }
    found.retain(|(p, _)| !rebound.contains(&p.0));
    found
}

/// The typed field getters whose value is a plain `Copy` scalar in the emitted Rust — the
/// reads @PLN157 P4c may hoist.  Each `#rust` template answers its kind's null sentinel at
/// `rec == 0` and a typed load otherwise, and the prelude evaluates that SAME template
/// once, so the hoisted local holds exactly what a per-iteration read would; nothing here
/// needs a per-kind table.  Reference, text and field-address getters stay out — a `DbRef`
/// or a `Str` is a view, not a value.
pub const SCALAR_GETTERS: [&str; 14] = [
    "OpGetInt",
    "OpGetInt4",
    "OpGetInt4Raw",
    "OpGetInt4Full",
    "OpGetShort",
    "OpGetShortRaw",
    "OpGetShortFull",
    "OpGetByte",
    "OpGetByteNullable",
    "OpGetSingle",
    "OpGetFloat",
    "OpGetBoolean",
    "OpGetCharacter",
    "OpGetEnum",
];

/// A hoisted scalar's key (@PLN157 P4c): the record VARIABLE and the field's byte offset.
pub type ScalarKey = (u16, i64);

/// Recognise `OpGet<scalar>(Var(v), const fld, …)` — the ONE definition of the hoistable
/// scalar read, asked by the collector and by the emitter, so the local the prelude bound
/// and the read the loop body replaces cannot name different fields.  Reports shape only;
/// the caller confirms the variable's type and that the loop hoisted it.
#[must_use]
pub fn scalar_read(getter: &str, args: &[Value]) -> Option<ScalarKey> {
    if !SCALAR_GETTERS.contains(&getter) || args.len() < 2 {
        return None;
    }
    let Value::Var(v) = args[0].unspan() else {
        return None;
    };
    let Value::Int(fld) = args[1].unspan() else {
        return None;
    };
    Some((*v, i64::from(*fld)))
}

/// What a loop body writes in place, by RECORD TYPE (a schema type number) and offset
/// (@PLN157 P4c).  A hoisted scalar `(v, fld)` of a record typed `tp` is stale after a
/// write at `(tp, fld)` through ANY route — the variable itself, a `&`-bound alias, an
/// element view into a `vector<tp>`, a callee's parameter — so the key is the type and the
/// offset, never the variable: aliasing is decided by what a write can reach, and a
/// `vector<integer>` element written at offset 0 reaches no record's field.
#[derive(Default, Debug, Clone, PartialEq, Eq)]
/// Enforces `@FR-R-Scalar`: the write set keyed by (record type, offset).
pub struct WriteSet {
    /// `(record type, offset)` pairs a scalar setter wrote through a classified target.
    pub offsets: HashSet<(u16, i64)>,
    /// Record types written at offsets this walk cannot see — a return buffer a callee
    /// fills (§ V-c), a record a body frees — so every scalar of the type is stale.
    pub whole: HashSet<u16>,
}

impl WriteSet {
    #[must_use]
    pub fn evicts(&self, tp: u16, fld: i64) -> bool {
        self.whole.contains(&tp) || self.offsets.contains(&(tp, fld))
    }

    fn extend(&mut self, other: &WriteSet) {
        self.offsets.extend(other.offsets.iter().copied());
        self.whole.extend(other.whole.iter().copied());
    }
}

/// The per-callee memo of [`callee_writes`]: `None` is a callee this analysis cannot
/// classify, which keeps every scalar of its callers un-hoisted.
pub type WriteCache = HashMap<u32, Option<Rc<WriteSet>>>;

/// What one loop may derive up front (@PLN157 P4c): the vector headers and the record
/// scalars, decided by ONE gate.
#[derive(Default)]
pub struct LoopHoist {
    /// The vector paths, each with the operand expression the prelude evaluates.
    pub vectors: Vec<(PathKey, Value)>,
    /// The scalar reads, each with the getter call the prelude evaluates once.
    pub scalars: Vec<(ScalarKey, Value)>,
    /// @PLN157 § V-q — the paths the body PUSHES to, each with the vector operand the
    /// prelude evaluates: these take a [`crate::vector::PushHeader`] (never listed in
    /// `vectors` as well), which the push keeps current and every read of the path serves
    /// from.
    pub pushes: Vec<(PathKey, Value)>,
    /// @PLN157 § V-t (`@FR-R-PushRec`) — the admitted MINT paths whose element is a plain
    /// no-heap struct: each takes a [`crate::vector::PushHeader`] the record append emits
    /// through (the slot from the header, the length bump at the finish).  A mint that does
    /// not qualify stays a plain mover (§ V-s: admitted, no holder, templates per element).
    pub mint_pushes: Vec<(PathKey, Value)>,
    /// @PLN157 § V-ak (`@FR-R-Base`) — the loop GROWS no store: no push, no mint push.
    /// Everything else the admission lets through — in-place sets, store-free ops,
    /// store-free or in-place-only callees, a free, and a null-discharge buffer's mint
    /// (§ V-ad: a FRESH store, or a clear of the buffer's own, which no header names —
    /// see [`lazy_buffer_mint`] for why a new slot moves no live store's memory) —
    /// leaves every store's buffer where it is, so a hoisted header may carry the
    /// address of its vector's element 0 for the loop's whole extent.
    pub growth_free: bool,
}

/// The vector headers and the record scalars `body` may derive once up front.
///
/// The gate is [`hoistable_vectors`]' gate; a body it declines hoists nothing.  Under it
/// a scalar read `v.fld` (see [`scalar_read`]) of a plain-struct record variable the body
/// does not rebind is hoisted unless the body's [`WriteSet`] — its own in-place setters,
/// what an admitted callee writes, what a free or a return buffer may touch — reaches the
/// record's type at that offset, or the body writes through a target this analysis cannot
/// type (then no scalar hoists).  `scalars` off (`LOFT_NO_SCALAR_HOIST=1`) answers the
/// vector headers alone.
// The eight parameters are the loop, the two things that type it (the IR and the schema),
// the two memos shared across every loop of the program, and the two switches; a struct
// would put a name between each and the one call site without removing anything.
// The eight parameters are the loop, the two things that type it (the IR and the schema),
// the two memos shared across every loop of the program, the tier switches and the § V-p
// memo; a struct would put a name between each and the one call site without removing
// anything.
#[allow(clippy::too_many_arguments)]
/// Enforces `@FR-R-Scalar` (the candidates and the write set) beside `@FR-R-Header`.
pub fn hoistable(
    body: &Block,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    tiers: HoistTiers,
    inputs: Option<&mut InputCache>,
) -> LoopHoist {
    if body_blocks_hoist(body, data, def_nr, cache, tiers) {
        return LoopHoist::default();
    }
    let mut out = LoopHoist {
        vectors: vector_candidates(body, data, def_nr),
        scalars: Vec::new(),
        pushes: Vec::new(),
        mint_pushes: Vec::new(),
        growth_free: false,
    };
    let vars = data.def(def_nr).variables();
    let rebound = rebound_vars(body);
    // (key, record type, the call) in first-appearance order.
    let mut found: Vec<(ScalarKey, u16, Value)> = Vec::new();
    if tiers.scalars {
        for op in &body.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && let Some(key) = scalar_read(data.def(*d).name(), args)
                    && !rebound.contains(&key.0)
                    && let Some(tp) = plain_record_type(data, vars.tp(key.0))
                    && !found.iter().any(|(k, _, _)| *k == key)
                {
                    found.push((key, tp, n.clone()));
                }
                false
            });
        }
    }
    // @PLN157 § V-p (`@FR-R-Inputs`) — what an admitted callee reads of a record the body
    // passes it as a plain variable joins the candidates, re-spelled over that variable; the
    // eviction below then judges each, since the write set carries what the callee writes.
    if let Some(inputs) = inputs {
        for op in &body.operators {
            op.any_node(&mut |n| {
                if let Value::Call(f, args) = n
                    && (*f as usize) < data.definitions.len()
                    && matches!(data.def(*f).code(), Value::Block(_))
                    && let Some(fi) = callee_inputs(*f, data, stores, cache, writes, inputs)
                {
                    for (pf, offs, path) in &fi.headers {
                        if let Some((key, expr)) = args
                            .get(*pf as usize)
                            .and_then(|a| input_header_at(data, a, *pf, offs, path))
                            && !rebound.contains(&key.0)
                            && !out.vectors.iter().any(|(p, _)| *p == key)
                        {
                            out.vectors.push((key, expr));
                        }
                    }
                    if tiers.scalars {
                        for (pf, fld, getter) in &fi.scalars {
                            if let Some(Value::Var(c)) = args.get(*pf as usize).map(Value::unspan)
                                && !rebound.contains(c)
                                && let Some(tp) = plain_record_type(data, vars.tp(*c))
                                && !found.iter().any(|(k, _, _)| *k == (*c, *fld))
                            {
                                found.push(((*c, *fld), tp, substitute_root(getter, *pf, *c)));
                            }
                        }
                    }
                }
                false
            });
        }
    }
    // @PLN157 § V-q (`@FR-R-Push`) — the paths the body pushes to.  The gate admitted every
    // push in the body as a fusable one over a pure path, so what is left to decide is
    // ALIASING (`@FR-R-Alias`), and then `@FR-R-State`: a pushed path leaves the read list,
    // so it has ONE holder — which is why this block runs after every collector that can
    // add a read candidate (the body's own reads, § V-p's callee inputs): a growth moves the pushed vector's record, so any other header naming that
    // same vector would go stale.  A single push beside no other candidate has nothing to
    // alias with and is admitted whatever its root.  Otherwise every push root must be
    // EXCLUSIVE — a local that owns its store, or the function's own return buffer — and a
    // read candidate is kept only when its root cannot name a pushed vector: an owned local
    // (two owners, two stores), or a parameter when no push targets the return buffer (a
    // parameter cannot alias a local's store, but it can alias a buffer the caller offered).
    // A push that fails the rule declines the WHOLE loop, as a growth always did — a push
    // left to its template would move a record a kept header still describes.
    let mut pushes: Vec<(PathKey, Value)> = Vec::new();
    if tiers.push {
        for op in &body.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && let Some(fp) = fused_push(data, data.def(*d).name(), args)
                    && !pushes.iter().any(|(p, _)| *p == fp.path)
                {
                    pushes.push((fp.path, fp.vector.clone()));
                }
                false
            });
        }
    }
    // @PLN157 § V-s (`@FR-R-Mint`) — the paths the body MINTS a record element into.  A
    // mint is a mover like a push and joins the same (`@FR-R-Alias`) decision, but it
    // earns no push header: its ops keep their templates, which resolve per element, so
    // the path only has to LEAVE the read list.  The gate admitted every mint in the body
    // as one over a plain bare-variable vector, so this collection cannot disagree with it.
    let mut mints: Vec<PathKey> = Vec::new();
    // § V-t (`@FR-R-PushRec`) — per mint path, the operand expression and whether EVERY
    // mint group on it qualifies for the record push (the element a plain no-heap struct).
    // One unqualified group keeps the whole path on § V-s's no-holder behaviour, because a
    // header only some of the path's appends keep current would go stale at the others.
    let mut mint_fused: Vec<(PathKey, Value, bool)> = Vec::new();
    if tiers.mint {
        for op in &body.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpNewRecord"
                    && let Some(path) = mint_path(data, "OpNewRecord", args, vars)
                {
                    let q = tiers.record_push && mint_push_qualifies(stores, args);
                    if let Some(row) = mint_fused.iter_mut().find(|(p, _, _)| *p == path) {
                        row.2 &= q;
                    } else {
                        mints.push(path.clone());
                        mint_fused.push((path, args[0].clone(), q));
                    }
                }
                false
            });
        }
    }
    if !pushes.is_empty() || !mints.is_empty() {
        let mover = |q: &PathKey| pushes.iter().any(|(p, _)| p == q) || mints.contains(q);
        if pushes
            .iter()
            .map(|(p, _)| p)
            .chain(mints.iter())
            .any(|p| rebound.contains(&p.0))
        {
            return LoopHoist::default();
        }
        let retbuf = retbuf_var(data, def_nr);
        let others = out.vectors.iter().any(|(q, _)| !mover(q));
        if pushes.len() + mints.len() > 1 || others {
            if !pushes
                .iter()
                .map(|(p, _)| p)
                .chain(mints.iter())
                .all(|p| owned_local(vars, p.0) || Some(p.0) == retbuf)
            {
                return LoopHoist::default();
            }
            let any_retbuf = pushes
                .iter()
                .map(|(p, _)| p)
                .chain(mints.iter())
                .any(|p| Some(p.0) == retbuf);
            out.vectors.retain(|(q, _)| {
                mover(q) || owned_local(vars, q.0) || (vars.is_argument(q.0) && !any_retbuf)
            });
        }
        out.vectors.retain(|(q, _)| !mover(q));
        out.pushes = pushes;
        out.mint_pushes = mint_fused
            .into_iter()
            .filter_map(|(p, expr, q)| q.then_some((p, expr)))
            .collect();
    }
    // `@FR-R-Base` — decided here, before either early return below: a loop with no
    // record scalars to hoist is exactly the shape a pixel loop has.
    let null_buf = mints_null_buffer(body, data, vars);
    out.growth_free = out.pushes.is_empty() && out.mint_pushes.is_empty();
    if std::env::var("LOFT_TRACE_BASE").is_ok() {
        eprintln!(
            "base: {} loop {} growth_free={} (pushes {}, mint pushes {}, null buffer {}, headers {})",
            data.def(def_nr).name(),
            body.scope,
            out.growth_free,
            out.pushes.len(),
            out.mint_pushes.len(),
            null_buf,
            out.vectors.len()
        );
    }
    if found.is_empty() {
        return out;
    }
    let mut written = WriteSet::default();
    // The fresh-element set (`@FR-R-Mint`) persists across the body's statements: the mint
    // binds its element variable in one statement and the field writes follow in the next.
    let mut fresh: HashSet<u16> = HashSet::new();
    for op in &body.operators {
        let Some(w) = body_writes(
            op,
            data,
            stores,
            vars,
            cache,
            writes,
            &mut HashSet::new(),
            &mut fresh,
        ) else {
            return out;
        };
        written.extend(&w);
    }
    out.scalars = found
        .into_iter()
        .filter(|(key, tp, _)| !written.evicts(*tp, key.1))
        .map(|(key, _, call)| (key, call))
        .collect();
    out
}

/// Does the body mint a § V-ad null-discharge buffer — the one store allocation the
/// header admission lets through?  It moves no header's record and no base's element
/// (a fresh store is its own allocation; a clear touches the buffer's own store, which no
/// header or base names), so since 2026-09-18 it is not a growth for `@FR-R-Base` either —
/// the drawing library's crossing loop (`pg_cur = pg_table[i]?`) held headers and no bases
/// for it, a store resolution per element read and write.  Reported by `LOFT_TRACE_BASE`.
fn mints_null_buffer(body: &Block, data: &Data, vars: &crate::variables::Function) -> bool {
    body.operators.iter().any(|op| {
        op.any_node(&mut |n| {
            matches!(n, Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && null_buffer_alloc(data.def(*d).name(), args, Some(vars), data).is_some())
        })
    })
}

/// `@FR-O-LazyBuffer` — the record-buffer pool's mint, which `scopes::reuse_record_buffers`
/// places behind `OpRefIsNull` on the buffer itself: it only ever takes a FRESH store from
/// the null sentinel, never clears one, so no header a loop holds can name the store it
/// fills.  Nor is it a growth for `@FR-R-Base`: a new store is a new slot whose memory is
/// its own allocation (`Store::ptr`), so no live store's memory moves, even when the slot
/// table itself grows.  `LOFT_HOIST_VERIFY=1` re-checks every base at every use.
fn lazy_buffer_mint(name: &str, args: &[Value], vars: Option<&crate::variables::Function>) -> bool {
    let Some(vars) = vars else { return false };
    (name == "OpDatabase" || name == "OpDatabaseNP")
        && matches!(args.first().map(Value::unspan), Some(Value::Var(b))
            if *b < vars.count() && vars.is_lazy_buffer(*b))
}

/// The schema type of a variable that names a PLAIN struct record — a `Reference` to a
/// `DefType::Struct`, reached through any `&` links but not through an `Optional`.  A
/// struct-enum, a variant, a nullable and a synthetic `__nullable<S>` answer `None`: their
/// payload offsets are a layout question this key does not model, so they are neither
/// hoisted nor classified as a write target (which keeps every scalar out of that loop).
fn plain_record_type(data: &Data, tp: &Type) -> Option<u16> {
    if matches!(tp, Type::Optional(_)) {
        return None;
    }
    let Type::Reference(d, _) = tp.peel_link() else {
        return None;
    };
    if data.def_type(*d) != DefType::Struct {
        return None;
    }
    let known = data.def(*d).known_type();
    (known != u16::MAX).then_some(known)
}

/// Does local `r` OWN the store it names — so no other variable of this frame can name
/// that store (@PLN157 § V-q)?  Enforces `@FR-R-Alias`'s exclusivity test.  Not a parameter (the caller's store), not a `&` link, an
/// empty dep list (`@FR-O-Proxy`), and not captured by a closure.
fn owned_local(vars: &crate::variables::Function, r: u16) -> bool {
    // `.base()`: the shape question sees through a `τ?` slot (`@FR-N-Shape`).
    !vars.is_argument(r)
        && !matches!(vars.tp(r).base(), Type::RefVar(_))
        && !vars.is_captured(r)
        // A local vector's dep list names its OWN hidden store witness (`__vdb_N`), which
        // is the store it owns, not a borrow of someone else's.
        && vars
            .tp(r)
            .depend()
            .iter()
            .all(|d| vars.name(*d).starts_with("__vdb"))
}

/// The variable of `def_nr`'s hidden return buffer, when it has one.
fn retbuf_var(data: &Data, def_nr: u32) -> Option<u16> {
    let def = data.def(def_nr);
    let attr = def.hidden_return_buffer_attr()?;
    let v = def.variables().var(&def.attributes()[attr].name);
    (v != u16::MAX).then_some(v)
}

/// What an in-place setter's target reaches.
enum Target {
    /// A field of a record of this schema type.
    Record(u16),
    /// An element of a scalar vector — no record field can alias it.
    NoRecord,
    /// A shape this analysis does not type.
    Unknown,
}

/// The record type an in-place setter's first operand addresses (@PLN157 P4c): a record
/// variable, a const field path (`OpGetField` carries the field's schema type), or an
/// element of a vector named by either.  Every other shape is [`Target::Unknown`].
fn setter_target(
    target: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
) -> Target {
    match target.unspan() {
        Value::Var(u) => {
            plain_record_type(data, vars.tp(*u)).map_or(Target::Unknown, Target::Record)
        }
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            if name == "OpGetField" && args.len() == 3 {
                let Value::Int(content) = args[2].unspan() else {
                    return Target::Unknown;
                };
                return match u16::try_from(*content) {
                    Ok(tp) if stores.is_struct(tp) => Target::Record(tp),
                    _ => Target::Unknown,
                };
            }
            if is_element_address(data, *d) && args.len() == 3 {
                return element_target(&args[0], data, stores, vars);
            }
            Target::Unknown
        }
        _ => Target::Unknown,
    }
}

/// The element type of the vector operand of an element-address op, as a [`Target`].
fn element_target(
    vector: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
) -> Target {
    match vector.unspan() {
        Value::Var(w) => match vars.tp(*w).peel_link() {
            Type::Vector(elem, _) => {
                if is_scalar(elem) {
                    Target::NoRecord
                } else {
                    plain_record_type(data, elem).map_or(Target::Unknown, Target::Record)
                }
            }
            _ => Target::Unknown,
        },
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpGetField"
                && args.len() == 3 =>
        {
            let Value::Int(content) = args[2].unspan() else {
                return Target::Unknown;
            };
            let Ok(vec_tp) = u16::try_from(*content) else {
                return Target::Unknown;
            };
            let elem = stores.content(vec_tp);
            if elem == u16::MAX {
                Target::Unknown
            } else if stores.is_struct(elem) {
                Target::Record(elem)
            } else if stores.is_base(elem) {
                Target::NoRecord
            } else {
                Target::Unknown
            }
        }
        _ => Target::Unknown,
    }
}

/// The [`WriteSet`] of a body that passed the hoist gate, or `None` when it writes through
/// something this analysis cannot type.  Walks every call: an [`IN_PLACE_SET_OPS`] setter
/// contributes its target's `(type, offset)`; a record free contributes the record's whole
/// type; a user callee that writes contributes what [`callee_writes`] answers for it; a
/// store-free op contributes nothing.  Anything else — a native writer the gate would have
/// blocked, a `CallRef`, a `Parallel`, a `Yield` — answers `None`, as does a setter whose
/// offset is not a constant.
///
/// `fresh` carries the element variables the body has MINTED so far (@PLN157 § V-s,
/// `@FR-R-Mint`): a `Set(v, OpNewRecord(P, …))` over an admissible mint path inserts `v`,
/// any other assignment to `v` and the group's own `OpFinishRecord` remove it, and a
/// setter whose target is a fresh variable contributes NOTHING — a record minted inside
/// the body cannot be named by a variable whose getter the prelude already ran, so no
/// hoisted scalar can go stale through it.  The walk is preorder, which is execution
/// order for the straight-line `_elm_N = OpNewRecord; sets; OpFinishRecord` group the
/// parser emits (the only producer of `OpNewRecord`); a group any FUTURE lowering splits
/// across branches merely fails to register, which costs the exemption, never soundness —
/// except a mint and a write of the SAME variable split across two arms, which the
/// parser's per-group `_elm_N` temps (fresh variable per group, assigned nowhere else)
/// keep unconstructible.
/// Enforces `@FR-R-Scalar`: the write set of a body that passed the gate.
#[allow(clippy::too_many_arguments)]
fn body_writes(
    node: &Value,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    active: &mut HashSet<u32>,
    fresh: &mut HashSet<u16>,
) -> Option<WriteSet> {
    let mut set = WriteSet::default();
    let mut ok = true;
    node.any_node(&mut |n| match n {
        Value::Set(v, inner) => {
            if matches!(inner.unspan(), Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpNewRecord"
                    && mint_path(data, "OpNewRecord", args, vars).is_some())
            {
                fresh.insert(*v);
            } else {
                fresh.remove(v);
            }
            false
        }
        Value::Call(d, args) => {
            if (*d as usize) >= data.definitions.len() {
                ok = false;
                return true;
            }
            let def = data.def(*d);
            let name = def.name();
            if IN_PLACE_SET_OPS.contains(&name) {
                let fld = args.get(1).map(Value::unspan);
                let Some(Value::Int(fld)) = fld else {
                    ok = false;
                    return true;
                };
                // A write into a freshly minted element reaches no record a hoisted
                // scalar can name (`@FR-R-Mint`).
                if let Some(Value::Var(u)) = args.first().map(Value::unspan)
                    && fresh.contains(u)
                {
                    return false;
                }
                match args
                    .first()
                    .map_or(Target::Unknown, |t| setter_target(t, data, stores, vars))
                {
                    Target::Record(tp) => {
                        set.offsets.insert((tp, i64::from(*fld)));
                    }
                    Target::NoRecord => {}
                    Target::Unknown => {
                        ok = false;
                        return true;
                    }
                }
            } else if FUSABLE_PUSHES.iter().any(|(p, _, _)| *p == name)
                || name == "OpPreAllocVector"
            {
                // @PLN157 § V-q — a push (or the reservation before one) grows a vector and
                // rewrites its handle slot, which no scalar getter reads; the container
                // record itself does not move.
            } else if mint_path(data, name, args, vars).is_some() {
                // @PLN157 § V-s (`@FR-R-Mint`) — the mint's growth moves the pushed
                // vector's record without changing any value, and its element defaults land
                // in the fresh record only.  The close of the group ends the variable's
                // fresh window.
                if name == "OpFinishRecord"
                    && let Some(Value::Var(e)) = args.get(1).map(Value::unspan)
                {
                    fresh.remove(e);
                }
            } else if name == "OpCopyRecord"
                && args.len() == 3
                && matches!(args[1].unspan(), Value::Var(e) if fresh.contains(e))
            {
                // § V-d's delivery tail into a fresh mint variable (`@FR-R-Mint`): the copy
                // writes the fresh element, and its source-free releases the builder's own
                // this-iteration buffer — neither is a record a hoisted scalar can name.
            } else if let Some(tp) = null_buffer_alloc(name, args, Some(vars), data) {
                // § V-ad — the discharge buffer is re-initialised whole; only a scalar hoisted
                // off ITS type could observe that, and the buffer's view is rebound per use.
                set.whole.insert(tp);
            } else if lazy_buffer_mint(name, args, Some(vars)) {
                // `@FR-O-LazyBuffer` — a fresh store for the buffer from its sentinel; only a
                // scalar hoisted off the buffer's own record type could observe it.
                let minted = match args.first().map(Value::unspan) {
                    Some(Value::Var(b)) => plain_record_type(data, vars.tp(*b)),
                    _ => None,
                };
                let Some(tp) = minted else {
                    ok = false;
                    return true;
                };
                set.whole.insert(tp);
            } else if RECORD_FREE_OPS.contains(&name) {
                let freed = match args.first().map(Value::unspan) {
                    Some(Value::Var(r)) => plain_record_type(data, vars.tp(*r)),
                    _ => None,
                };
                let Some(tp) = freed else {
                    ok = false;
                    return true;
                };
                set.whole.insert(tp);
            } else if matches!(def.code(), Value::Null) {
                if !native_op_is_store_free(def) {
                    ok = false;
                    return true;
                }
            } else if call_writes_store(*d, data, cache, active) {
                let Some(w) = callee_writes(*d, data, stores, cache, writes, active) else {
                    ok = false;
                    return true;
                };
                set.extend(&w);
            }
            false
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => {
            ok = false;
            true
        }
        _ => false,
    });
    ok.then_some(set)
}

/// What a WRITING callee the hoist gate admits can reach (@PLN157 P4c): an in-place-only
/// writer (§ V-l) answers its own body's [`WriteSet`], with its parameters typed by its own
/// variable table — a write through `l: Lay` is `(Lay, fld)` whichever caller record was
/// passed; a return-buffer writer (§ V-c) answers its buffer's record type WHOLE, because the
/// buffer may be a record the caller offered (§ V-d).  A callee that is neither, or one met
/// while its own verdict is still open (recursion), answers `None`.  Memoised per callee.
/// Enforces `@FR-R-Scalar` through `@FR-R-Callee`: what an admitted callee reaches.
fn callee_writes(
    d_nr: u32,
    data: &Data,
    stores: &Stores,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    active: &mut HashSet<u32>,
) -> Option<Rc<WriteSet>> {
    if let Some(known) = writes.get(&d_nr) {
        return known.clone();
    }
    let def = data.def(d_nr);
    let answer = if matches!(def.code(), Value::Null) || !active.insert(d_nr) {
        None
    } else {
        let inner = if in_place_only_writer(d_nr, data, cache, active) {
            body_writes(
                def.code(),
                data,
                stores,
                def.variables(),
                cache,
                writes,
                active,
                &mut HashSet::new(),
            )
        } else if retbuf_only_writer(d_nr, data, cache, active) {
            def.hidden_return_buffer_attr()
                .and_then(|attr| plain_record_type(data, &def.attributes()[attr].typedef))
                .map(|tp| WriteSet {
                    offsets: HashSet::new(),
                    whole: HashSet::from([tp]),
                })
        } else {
            None
        };
        active.remove(&d_nr);
        inner.map(Rc::new)
    };
    // A `None` reached on a recursive edge is memoised too: it only ever withholds a hoist.
    writes.insert(d_nr, answer.clone());
    answer
}

/// @PLN157 § V-p — a callee's INVARIANT INPUTS (`@FR-R-Inputs`): what a caller that already
/// holds the hoisted values may hand it in place of the record.
///
/// Per plain-struct parameter the callee never rebinds: the scalar fields its own
/// [`WriteSet`] does not reach, and the vector paths it indexes or binds to a view the rest of
/// that block indexes (`@FR-R-View`).  Each carries the read as the callee's body spells it,
/// over `Var(parameter)`, so a caller re-spells it over its argument variable.  The two lists
/// are the twin's extra parameters, in this order.
#[derive(Debug, Default)]
pub struct CalleeInputs {
    /// `(parameter, field offset, the getter call)`, in first-appearance order.
    pub scalars: Vec<(u16, i64, Value)>,
    /// `(parameter, path offsets, the path expression)`, in first-appearance order.
    pub headers: Vec<(u16, Vec<i64>, Value)>,
}

/// The per-callee memo of [`callee_inputs`]: `None` is a callee that has no twin.
pub type InputCache = HashMap<u32, Option<Rc<CalleeInputs>>>;

/// @PLN157 § V-p — the invariant inputs of `d_nr`, or `None` when it has none or is not
/// admitted (`@FR-R-Inputs`).
///
/// Admitted: a loft body (no template, no return buffer, not a generator) that is store-free
/// or an in-place-only writer (`@FR-R-Callee`), whose write set this analysis can type.  A
/// scalar read `p.f` of a plain-struct parameter `p` the body never rebinds is an input unless
/// the body's own write set reaches `(type(p), f)`; a path `p.g` the body indexes, or binds to
/// a view the rest of its block indexes, is a header input.  A callee this body passes `p` on
/// to contributes ITS inputs re-spelled over `p` — transitively, and a cycle contributes
/// nothing — filtered by this body's write set, which already carries what that callee
/// writes.  Memoised per definition; the memo is seeded `None` while the walk is open, so a
/// recursive edge sees no inputs and only ever withholds a twin.
pub fn callee_inputs(
    d_nr: u32,
    data: &Data,
    stores: &Stores,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    inputs: &mut InputCache,
) -> Option<Rc<CalleeInputs>> {
    if let Some(known) = inputs.get(&d_nr) {
        return known.clone();
    }
    inputs.insert(d_nr, None);
    let answer = callee_inputs_inner(d_nr, data, stores, cache, writes, inputs).map(Rc::new);
    inputs.insert(d_nr, answer.clone());
    answer
}

fn callee_inputs_inner(
    d_nr: u32,
    data: &Data,
    stores: &Stores,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    inputs: &mut InputCache,
) -> Option<CalleeInputs> {
    if (d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let def = data.def(d_nr);
    let Value::Block(body) = def.code().unspan() else {
        return None;
    };
    // `.base()`: the shape question sees through a `τ?` result (`@FR-N-Shape`).
    if !def.rust().is_empty() || matches!(def.returned().base(), Type::Iterator(_, _)) {
        return None;
    }
    let mut active = HashSet::new();
    if call_writes_store(d_nr, data, cache, &mut active)
        && !in_place_only_writer(d_nr, data, cache, &mut active)
    {
        return None;
    }
    let vars = def.variables();
    let params = u16::try_from(def.attributes().len()).ok()?;
    let rebound = rebound_vars(body);
    // What the body writes, as the caller's gate accounts it (`@FR-R-Callee`): a
    // return-buffer writer (§ V-ac — `brush_sample` answering a `Smp` through its buffer)
    // reaches its buffer's record type WHOLE, which no parameter's field shares; any other
    // admitted body has a typed set of its own, or no twin.
    let written = if def.hidden_return_buffer_attr().is_some()
        && retbuf_only_writer(d_nr, data, cache, &mut active)
    {
        let attr = def.hidden_return_buffer_attr()?;
        WriteSet {
            offsets: HashSet::new(),
            whole: HashSet::from([plain_record_type(data, &def.attributes()[attr].typedef)?]),
        }
    } else {
        let mut written = WriteSet::default();
        let mut fresh_elems: HashSet<u16> = HashSet::new();
        for op in &body.operators {
            written.extend(&body_writes(
                op,
                data,
                stores,
                vars,
                cache,
                writes,
                &mut HashSet::new(),
                &mut fresh_elems,
            )?);
        }
        written
    };
    // A parameter (never rebound) as a candidate root: its plain record type, or — for a
    // header — whether it names a vector at all.
    let record_param = |p: u16| -> Option<u16> {
        if p < params && !rebound.contains(&p) {
            plain_record_type(data, vars.tp(p))
        } else {
            None
        }
    };
    let path_root = |root: u16, offs: &[i64]| -> bool {
        root < params
            && !rebound.contains(&root)
            && (!offs.is_empty() || matches!(vars.tp(root).peel_link(), Type::Vector(_, _)))
    };
    let mut out = CalleeInputs::default();
    def.code().any_node(&mut |n| {
        match n {
            Value::Call(g, args) if (*g as usize) < data.definitions.len() => {
                if let Some((p, fld)) = scalar_read(data.def(*g).name(), args) {
                    if let Some(tp) = record_param(p)
                        && !written.evicts(tp, fld)
                        && !out.scalars.iter().any(|(q, f, _)| (*q, *f) == (p, fld))
                    {
                        out.scalars.push((p, fld, n.clone()));
                    }
                } else if args.len() == 3
                    && is_element_address(data, *g)
                    && let Some((root, offs)) = vector_path(data, &args[0])
                    && path_root(root, &offs)
                    && !out.headers.iter().any(|(r, o, _)| *r == root && *o == offs)
                {
                    out.headers.push((root, offs, args[0].clone()));
                }
            }
            Value::Block(b) => {
                for at in 0..b.operators.len() {
                    if view_def_header(&b.operators, at, data, d_nr, cache, true).is_some()
                        && let Value::Set(_, rhs) = b.operators[at].unspan()
                        && let Some((root, offs)) = vector_path(data, rhs)
                        && path_root(root, &offs)
                        && !out.headers.iter().any(|(r, o, _)| *r == root && *o == offs)
                    {
                        out.headers.push((root, offs, (**rhs).clone()));
                    }
                }
            }
            _ => {}
        }
        false
    });
    // What a callee this body passes a parameter on to reads of it.
    def.code().any_node(&mut |n| {
        if let Value::Call(f, args) = n
            && (*f as usize) < data.definitions.len()
            && matches!(data.def(*f).code(), Value::Block(_))
            && let Some(fi) = callee_inputs(*f, data, stores, cache, writes, inputs)
        {
            for (pf, fld, getter) in &fi.scalars {
                if let Some(Value::Var(p)) = args.get(*pf as usize).map(Value::unspan)
                    && let Some(tp) = record_param(*p)
                    && !written.evicts(tp, *fld)
                    && !out.scalars.iter().any(|(q, f, _)| (*q, *f) == (*p, *fld))
                {
                    out.scalars
                        .push((*p, *fld, substitute_root(getter, *pf, *p)));
                }
            }
            for (pf, offs, path) in &fi.headers {
                if let Some(((p, key), expr)) = args
                    .get(*pf as usize)
                    .and_then(|a| input_header_at(data, a, *pf, offs, path))
                    && path_root(p, &key)
                    && !out.headers.iter().any(|(r, o, _)| *r == p && *o == key)
                {
                    out.headers.push((p, key, expr));
                }
            }
        }
        false
    });
    (!out.scalars.is_empty() || !out.headers.is_empty()).then_some(out)
}

/// `v` with every `Var(from)` renamed to `Var(to)` — a callee's read re-spelled over the
/// caller's argument variable (@PLN157 § V-p).
#[must_use]
pub fn substitute_root(v: &Value, from: u16, to: u16) -> Value {
    substitute_path(v, from, &Value::Var(to))
}

/// `path` — a callee's pure path over `Var(from)` — re-spelled over the caller's argument
/// `arg`, itself a pure path (@PLN157 § V-ac): the callee's `img` at an argument `br.img`
/// is the caller's `br.img`, its `c.data` at `h.cv` is `h.cv.data`.  A walk of its own
/// rather than `map_nodes`, which descends into the replacement: the argument is spelled
/// over the CALLER's variables, whose numbers can coincide with `from`.
#[must_use]
pub fn substitute_path(path: &Value, from: u16, arg: &Value) -> Value {
    match path {
        Value::Var(x) if *x == from => arg.clone(),
        Value::Span(b) => Value::Span(Box::new((b.0.clone(), substitute_path(&b.1, from, arg)))),
        Value::Call(d, args) => Value::Call(
            *d,
            args.iter().map(|a| substitute_path(a, from, arg)).collect(),
        ),
        other => other.clone(),
    }
}

/// A callee's header input `(pf, offs, path)` at the caller's argument `arg`, when that
/// argument is a pure path (`@FR-R-Header`): the caller's key — the argument's path
/// extended by the callee's offsets — and the expression that derives it.  `None` for any
/// other argument (an element view, a conditional, a call), which keeps the plain call.
/// Enforces `@FR-R-Inputs` (the path-argument half, @PLN157 § V-ac).
#[must_use]
pub fn input_header_at(
    data: &Data,
    arg: &Value,
    pf: u16,
    offs: &[i64],
    path: &Value,
) -> Option<(PathKey, Value)> {
    let (root, mut key) = vector_path(data, arg)?;
    key.extend_from_slice(offs);
    Some(((root, key), substitute_path(path, pf, arg)))
}

/// @PLN157 § V-n — does statement `at` of `stmts` bind a VIEW of a vector whose header the
/// rest of the block may derive once, right after the binding?
///
/// The shape is `d = &cv.data` (a `Set` of a vector-typed local from a pure path); the
/// promise is the loop hoist's, applied to the statements AFTER the binding: none of them
/// can write a store except in place (`allow_in_place`), none rebinds `d`, and at least
/// one indexes `d` — a length read alone is cheaper through the runtime than through a
/// header, so it earns none.  The binding's own value is what the header describes, so a
/// rebind of the path's ROOT later in the block does not matter: `d` keeps the `DbRef` it
/// was given (and a reassignment that replaces the record is a store write the gate sees).
/// Answers the view variable, whose `Var` the prelude evaluates.
/// Enforces `@FR-R-View`.
pub fn view_def_header(
    stmts: &[Value],
    at: usize,
    data: &Data,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
) -> Option<u16> {
    let Value::Set(d, rhs) = stmts.get(at)?.unspan() else {
        return None;
    };
    let vars = data.def(def_nr).variables();
    if !matches!(vars.tp(*d).peel_link(), Type::Vector(_, _)) {
        return None;
    }
    let (root, _) = vector_path(data, rhs)?;
    if root == *d {
        return None;
    }
    let rest = &stmts[at + 1..];
    if rest.iter().any(|op| {
        blocks_header_hoist(
            op,
            data,
            cache,
            &mut HashSet::new(),
            Some(vars),
            HoistTiers {
                in_place: allow_in_place,
                ..HoistTiers::default()
            },
            &mut HashSet::new(),
        )
    }) {
        return None;
    }
    let mut rebound = false;
    let mut indexed = false;
    for op in rest {
        op.any_node(&mut |n| {
            match n {
                Value::Set(v, _) | Value::TuplePut(v, _, _) if *v == *d => rebound = true,
                Value::Call(op_nr, args)
                    if args.len() == 3
                        && is_element_address(data, *op_nr)
                        && matches!(args[0].unspan(), Value::Var(v) if *v == *d) =>
                {
                    indexed = true;
                }
                _ => {}
            }
            false
        });
    }
    (indexed && !rebound).then_some(*d)
}

/// The Rust type a fusable scalar getter loads and the sentinel it answers at the null
/// record — [`FUSABLE_GETTERS`]' row, for the record-address read (`@FR-R-RecPtr`).
#[must_use]
pub fn scalar_kind(getter: &str) -> Option<(&'static str, &'static str)> {
    FUSABLE_GETTERS
        .iter()
        .find(|(name, _, _)| *name == getter)
        .map(|(_, ty, absent)| (*ty, *absent))
}

/// The Rust type a fusable scalar setter stores — [`FUSABLE_SETTERS`]' row.
#[must_use]
pub fn setter_kind(setter: &str) -> Option<&'static str> {
    FUSABLE_SETTERS
        .iter()
        .find(|(name, _)| *name == setter)
        .map(|(_, ty)| *ty)
}

/// `@FR-R-RecPtr` — does statement `at` of `stmts` bind a plain-record local whose ADDRESS
/// the rest of the block may derive once, right after the binding?
///
/// The record twin of [`view_def_header`].  `(B-View)` fixes the binding's `DbRef` at the
/// bind — `e = tbl[i]?`, `s = o.inner`, a copy of another view — so the record's first
/// byte is one address for as long as the place lives, and every scalar field read and
/// in-place field write of `r` in the remainder can be one load or store through it instead
/// of a store resolution each.  The promise is `(R-Base)`'s applied to the statements AFTER
/// the binding: none grows a store (pushes and mints block; a null-discharge buffer's mint
/// does not, see [`mints_null_buffer`]), none frees a record BEFORE a later use of `r` (a
/// freed record read through the store answers the sentinel, through a pointer it would
/// answer stale bytes; the releases a block ends with follow the last use and are fine),
/// none rebinds `r`, and at least one reads or writes a fusable scalar field of `r` — or hands
/// `r` to a callee whose TWIN takes that parameter's fields as inputs (`twin_params`, the
/// `(callee, parameter)` pairs the caller resolved: `(R-Inputs)` then reads them through the
/// address at the call).  Only a PLAIN struct qualifies ([`plain_record_type`]): a nullable, an enum payload and a
/// synthetic `__nullable<S>` carry a layout question this does not model.  The null record
/// keeps its sentinel: the address is null and the read tests it.
///
/// Answers the view variable.
///
/// # Errors
///
/// The reason the statement declines, in the words `LOFT_TRACE_RECPTR=1` prints: not a
/// binding, not a plain record, a remainder that may grow a store, one that frees a
/// record before a use of the view, one that rebinds it, or no fusable use at all.
pub fn record_view_ptr(
    stmts: &[Value],
    at: usize,
    data: &Data,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
    twin_params: &HashSet<(u32, u16)>,
) -> Result<u16, &'static str> {
    let Some(Value::Set(r, rhs)) = stmts.get(at).map(Value::unspan) else {
        return Err("not a binding");
    };
    let vars = data.def(def_nr).variables();
    // A NULLABLE view (`e = v[i]` without `?`, a `for e in v` loop variable whose null is
    // the loop's end signal) is the same record or the null DbRef: the address is null
    // there and every read through it answers the getter's own sentinel, exactly as the
    // store read does.  The enum payload and `__nullable<S>` exclusions are the inner
    // type's, as for a plain view.
    let view_tp = match vars.tp(*r) {
        Type::Optional(inner) => inner.as_ref(),
        tp => tp,
    };
    if plain_record_type(data, view_tp).is_none() {
        return Err("not a plain record");
    }
    // A buffer's pre-init (`__ref_p2_N = null`, then a mint into it): no place yet, so no
    // address — measured: the address was taken null and the literal's field writes through
    // it were dropped, a library's `mk()` answering a record of zeros (loft's own 47 golden
    // and the #672 parity test).
    if matches!(rhs.unspan(), Value::Null) {
        return Err("bound null");
    }
    let rest = &stmts[at + 1..];
    if rest.iter().any(|op| {
        blocks_header_hoist(
            op,
            data,
            cache,
            &mut HashSet::new(),
            Some(vars),
            HoistTiers {
                in_place: allow_in_place,
                ..HoistTiers::default()
            },
            &mut HashSet::new(),
        )
    }) {
        return Err("the remainder may grow a store");
    }
    // In statement order: a free (a scope exit's release of a local, a buffer's) BEFORE a
    // use of `r` declines — through the store a freed record answers the sentinel, through
    // an address it would answer stale bytes — while the frees that follow the last use,
    // which is where a block's own releases stand, cost nothing.  A view that outlives its
    // container is `(B-Disturb)`'s case and is materialised by the parser before this runs.
    let mut rebound = false;
    let mut touched = false;
    let mut released_before = false;
    for op in rest {
        let mut uses = false;
        let mut releases = false;
        op.any_node(&mut |n| {
            match n {
                Value::Set(v, _) | Value::TuplePut(v, _, _) if *v == *r => rebound = true,
                Value::Var(v) if *v == *r => uses = true,
                Value::Call(g, args) if (*g as usize) < data.definitions.len() => {
                    let name = data.def(*g).name();
                    if frees_a_record(name, args, Some(vars)) {
                        releases = true;
                    }
                    let on_r =
                        matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == *r);
                    // A NATIVE op with the view as its first operand that is not a read
                    // (`OpGet…`) or a fusable scalar set re-seats or releases the place —
                    // `OpDatabaseNP(buf, tp)` mints into it, `OpNewRecord`, a copy into it, a
                    // free — so it is a rebind; a loft-bodied callee can only write in place
                    // (`(R-Callee)`).
                    let native = matches!(data.def(*g).code(), Value::Null)
                        || !data.def(*g).rust().is_empty();
                    if on_r && native && !name.starts_with("OpGet") && setter_kind(name).is_none() {
                        rebound = true;
                    }
                    if on_r
                        && ((scalar_kind(name).is_some() && scalar_read(name, args).is_some())
                            || (setter_kind(name).is_some()
                                && matches!(args.get(1).map(Value::unspan), Some(Value::Int(_)))))
                    {
                        touched = true;
                    }
                    if args.iter().enumerate().any(|(i, a)| {
                        matches!(a.unspan(), Value::Var(v) if *v == *r)
                            && u16::try_from(i).is_ok_and(|p| twin_params.contains(&(*g, p)))
                    }) {
                        touched = true;
                    }
                }
                _ => {}
            }
            false
        });
        if uses && (released_before || releases) {
            return Err("the remainder frees a record before a use of the view");
        }
        released_before |= releases;
    }
    if rebound {
        return Err("the remainder rebinds the view");
    }
    if !touched {
        return Err("no fusable field read or write of the view");
    }
    Ok(*r)
}

/// @PLN157 § V-o — is `d_nr` a stdlib ONE-OP wrapper: a loft function whose whole body is
/// one native op over its own parameters (`pub fn len(both: vector) -> integer {
/// OpLengthVector(both) }`, `sqrt` = `OpMathFuncFloat(9, both)`)?
///
/// Answers the op and the operand list to emit in the wrapper's place, each operand a
/// PARAMETER INDEX or a constant the body spells.  A body that does anything else — a
/// conversion (`exp` passes `E as single`), a second statement, a call to another loft
/// function, a return buffer — answers `None` and the call is emitted as a call.
/// Emitting the op instead of the call changes nothing the program can observe (the
/// wrapper's Rust body IS the op's template) and lets the registry's header-aware emitters
/// see the op: `len(d)` after a view binding or inside a hoisted loop reads the header's
/// length instead of resolving the store.
#[must_use]
/// Enforces `@FR-R-Wrapper`, the STDLIB qualifier included.
pub fn one_op_wrapper(data: &Data, d_nr: u32) -> Option<(u32, Vec<WrapperOperand>)> {
    if (d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let def = data.def(d_nr);
    if !def.rust().is_empty() || matches!(def.code(), Value::Null) {
        return None;
    }
    if def.hidden_return_buffer_attr().is_some() {
        return None;
    }
    // Only the STDLIB's wrappers.  A user function's CALL is itself observable: the live
    // tier may flip it to the interpreter (`live_flipped`), and its frame is on the shadow
    // call stack — emitting its op in place of the call skips both.  A stdlib wrapper has
    // neither (a `t_` method carries no frame and no live check), so for it and only for
    // it the op IS the call.  Measured: a one-op user function (`fn reader(w: W) -> integer
    // { w.a }`) flipped to the interpreter ran compiled, and the wasm live-dispatch probe
    // counted 0 dispatches where it expected 2.
    if def.source() != crate::data::STD_SOURCE {
        return None;
    }
    // A text parameter or result takes the CALL's conversions (a `String` result handed to
    // a `&str` parameter, a work buffer for the answer) that a template's operands do not
    // get, so `len(t)` and `print(t)` stay calls.
    if def
        .attributes()
        .iter()
        .any(|a| matches!(a.typedef.base(), Type::Text(_)))
        || matches!(def.returned().base(), Type::Text(_))
    {
        return None;
    }
    let params = def.attributes().len();
    // The body: a block of exactly one statement (line markers aside), which is the op
    // call, possibly under a `Return`.
    let mut stmt: Option<&Value> = None;
    match def.code().unspan() {
        Value::Block(b) => {
            for op in &b.operators {
                if matches!(op.unspan(), Value::Line(_)) {
                    continue;
                }
                if stmt.is_some() {
                    return None;
                }
                stmt = Some(op);
            }
        }
        other => stmt = Some(other),
    }
    let mut stmt = stmt?.unspan();
    if let Value::Return(inner) = stmt {
        stmt = inner.unspan();
    }
    let Value::Call(op, args) = stmt else {
        return None;
    };
    let callee = data.def(*op);
    if !matches!(callee.code(), Value::Null) || callee.rust().is_empty() {
        return None;
    }
    let mut operands = Vec::with_capacity(args.len());
    for a in args {
        match a.unspan() {
            Value::Var(v) if (*v as usize) < params => operands.push(WrapperOperand::Param(*v)),
            Value::Int(n) => operands.push(WrapperOperand::Const(Value::Int(*n))),
            _ => return None,
        }
    }
    Some((*op, operands))
}

/// One operand of a [`one_op_wrapper`]'s op: the caller's argument at that parameter
/// position, or a constant the wrapper body spells (a libm selector).
#[derive(Clone, Debug, PartialEq)]
pub enum WrapperOperand {
    Param(u16),
    Const(Value),
}

/// The typed getters an element read can be fused INTO, with the Rust type each reads and
/// the null sentinel its `#rust` template answers at the absent element.
///
/// All three are `if rec != 0 && valid(..) { *addr } else { <sentinel> }` in `Store`, which
/// is what makes one fused load able to stand for the pair. A getter with a different shape
/// — `OpGetBoolean` masks, `OpGetByte` re-bases, `OpGetCharacter` decodes — is left out
/// rather than approximated; it keeps the unfused emission.
const FUSABLE_GETTERS: [(&str, &str, &str); 3] = [
    ("OpGetInt", "i64", "i64::MIN"),
    ("OpGetSingle", "f32", "f32::NAN"),
    ("OpGetFloat", "f64", "f64::NAN"),
];

/// An element read the emitter can collapse into ONE load: a scalar getter reading field
/// `fld` out of `vector[index]`, where the vector is a plain variable.
pub struct FusedRead<'a> {
    /// The vector operand — a `Var` or a pure `OpGetField` chain over one, so it
    /// re-emits without side effects.
    pub vector: &'a Value,
    pub path: PathKey,
    pub size: &'a Value,
    pub index: &'a Value,
    pub fld: &'a Value,
    /// Rust type of the load, e.g. `"f32"`.
    pub rust_type: &'static str,
    /// What the getter answers at an absent element, e.g. `"f32::NAN"`.
    pub absent: &'static str,
}

/// Recognise `OpGet<scalar>(OpGetVector*(Var(v), size, index), fld)`, given the outer
/// getter's op NAME.
///
/// The ONE definition of the fused shape: the emitter and the pre-eval collector both ask
/// here, so the pre-eval cannot hoist an inner read that the emitter then folds away (which
/// would leave the read happening twice). Answers `None` for every other shape — including
/// an indexed read whose vector operand is an expression rather than a variable.
///
/// The caller still has to confirm the vector HAS a hoisted header; this only reports shape.
#[must_use]
pub fn fused_element_read<'a>(
    data: &Data,
    getter: &str,
    args: &'a [Value],
) -> Option<FusedRead<'a>> {
    let [inner, fld] = args else { return None };
    let (_, rust_type, absent) = FUSABLE_GETTERS
        .iter()
        .find(|(name, _, _)| *name == getter)?;
    let Value::Call(elem_op, elem_args) = inner.unspan() else {
        return None;
    };
    if !is_element_address(data, *elem_op) {
        return None;
    }
    let [vector, size, index] = &elem_args[..] else {
        return None;
    };
    let path = vector_path(data, vector)?;
    Some(FusedRead {
        vector,
        path,
        size,
        index,
        fld,
        rust_type,
        absent,
    })
}

/// The typed setters an element write can be fused INTO (@PLN157 P4b), with the Rust
/// type each stores.  The write twins of [`FUSABLE_GETTERS`], excluded for the same
/// reasons: a setter that re-bases (`OpSetByte`/`OpSetShort`), masks or translates
/// keeps the unfused emission.
/// The scalar pushes a loop may hoist a header for (@PLN157 § V-q, `@FR-R-Push`): the op,
/// the Rust type of the value, and the element width.  The three [`HoistScalar`] kinds;
/// a push of another kind keeps its template AND blocks the loop, as every growth does,
/// because only a push emitted through the refreshing helper leaves the header current.
///
/// [`HoistScalar`]: crate::vector::HoistScalar
pub const FUSABLE_PUSHES: [(&str, &str, u32); 3] = [
    ("OpPushInt", "i64", 8),
    ("OpPushSingle", "f32", 4),
    ("OpPushFloat", "f64", 8),
];

/// A push the emitter can route through a hoisted [`crate::vector::PushHeader`].
pub struct FusedPush<'a> {
    /// The pushed path's key.
    pub path: PathKey,
    /// The vector operand, emitted for the growth step's runtime append.
    pub vector: &'a Value,
    /// The value pushed.
    pub val: &'a Value,
    /// The Rust type of the value.
    pub rust_type: &'static str,
    /// The element width in bytes.
    pub size: u32,
}

/// @PLN157 § V-ae (`@FR-R-Fill`) — a loop that is ONE fill: `for i in lo..hi { v[base + i]
/// = val }` with `base` and `val` invariant, over a pure path a header may serve.
pub struct FillLoop<'a> {
    /// The written path's key and operand (the header's, `@FR-R-Header`).
    pub path: PathKey,
    pub vector: &'a Value,
    /// The Rust type of the element, and its width as the setter spells it.
    pub rust_type: &'static str,
    pub size: u32,
    /// The invariant added to the loop variable, when the index is not the variable alone.
    pub base: Option<&'a Value>,
    /// The range's end operand, and whether the range includes it.
    pub hi: &'a Value,
    pub inclusive: bool,
    /// The counter the range runs on: the `#index` variable, and the `next` variable of
    /// the two-counter form (a computed start, § V-ab) — `None` in the single-counter form
    /// (a literal start, P3b), whose `#index` is seeded one below the start.
    pub index_var: u16,
    pub next_var: Option<u16>,
    pub val: &'a Value,
}

/// Is `v` an INVARIANT scalar expression the fill may evaluate once — a variable other
/// than the loop's own, a literal, plain integer/float arithmetic over those, or a record
/// scalar read off such a variable?  Anything else (a call, a `??` block, an element read,
/// a conversion) keeps the per-element loop: the fill evaluates the expression once and
/// the fallback once more, so it must be pure and free of the loop's counters.
fn simple_invariant(v: &Value, data: &Data, banned: &[u16]) -> bool {
    match v.unspan() {
        Value::Var(x) => !banned.contains(x),
        Value::Int(_) | Value::Float(_) => true,
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            if matches!(
                name,
                "OpAddInt" | "OpMinInt" | "OpMulInt" | "OpAddFloat" | "OpMinFloat" | "OpMulFloat"
            ) {
                args.len() == 2 && args.iter().all(|a| simple_invariant(a, data, banned))
            } else if SCALAR_GETTERS.contains(&name) && args.len() == 3 {
                matches!(args[0].unspan(), Value::Var(r) if !banned.contains(r))
                    && matches!(args[1].unspan(), Value::Int(_))
                    && matches!(args[2].unspan(), Value::Int(_))
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Is `v` a `break` — bare, or a block whose only statement is one?
fn is_break_block(v: &Value) -> bool {
    match v.unspan() {
        Value::Break(_) => true,
        Value::Block(b) => {
            b.operators.len() == 1 && matches!(b.operators[0].unspan(), Value::Break(_))
        }
        _ => false,
    }
}

/// The `if index == hi { break }` an INCLUSIVE range emits before its step (loft#1525) — the
/// stop decided on the value just yielded, so `hi + 1` never has to exist.  Recognised by SHAPE
/// (an equality whose arms are a break and a null), never by position alone, so an ordinary
/// leading statement is not mistaken for it.
fn is_inclusive_stop_guard(v: &Value, data: &Data) -> bool {
    let Value::If(cond, on_true, on_false) = v.unspan() else {
        return false;
    };
    if !is_break_block(on_true) || !matches!(on_false.unspan(), Value::Null) {
        return false;
    }
    // The EQUALITY is what tells this guard from the range's own break test, and the
    // distinction is load-bearing: the two-counter form is `[test, index, step, yield]`, whose
    // leading statement is ALSO an if-break over a two-argument call.  Matching on the shape
    // alone peeled that test and read the remainder as the single-counter form — a different
    // loop, silently.  The range tests with `OpLtInt`/`OpLeInt`; only the stop guard is `==`.
    matches!(
        cond.unspan(),
        Value::Call(d, args) if args.len() == 2 && data.def(*d).name() == "OpEqInt"
    )
}

/// Recognise the fill idiom in a `For loop` body (@PLN157 § V-ae): the iterator block of a
/// counted range in either lowering — the two-counter form `[if hi <cmp> next { break };
/// index = next; next = next + 1; index]` (§ V-ab) or the single-counter form `[index =
/// index + 1; if hi <cmp> index { break }; index]` (P3b) — followed by ONE statement, a
/// fusable scalar set (`OpSetInt`/`OpSetSingle`/`OpSetFloat`) at field 0 of `v[idx]` with
/// `v` a pure path, `idx` the loop variable or `invariant + variable`, and the value
/// invariant.  `<cmp>` is `OpLtInt` for an inclusive range and `OpLeInt` for an exclusive
/// one.  Reports shape only; the emitter confirms the path has a header.
/// Enforces `@FR-R-Fill`.
/// The counters of a counted `for` loop, as the parser lowers `for v in a..b`: the loop
/// variable, the `#index` it yields, the `next` counter of the two-counter form (a
/// computed start, § V-ab; `None` in the single-counter form, P3b), whether the range is
/// inclusive, and its end operand.  ONE home — the fill rewrite reads the shape here and
/// so does the non-sentinel pass, which seeds these counters as never-null off the bound.
pub struct RangeCounters<'a> {
    pub loop_var: u16,
    pub index: u16,
    pub next: Option<u16>,
    pub inclusive: bool,
    pub hi: &'a Value,
}

/// Parse a `For loop` block's iterator into its counters, or say which part of the shape
/// it is not.
///
/// # Errors
///
/// The reason the block is not a counted range — the trace the fill rewrite prints.
pub fn range_counters<'a>(lp: &'a Block, data: &Data) -> Result<RangeCounters<'a>, String> {
    let decline = |why: &str| -> Result<RangeCounters<'a>, String> { Err(why.to_string()) };
    if lp.operators.len() != 2 {
        return decline(&format!("loop has {} statements", lp.operators.len()));
    }
    let Value::Set(loop_var, iter) = lp.operators[0].unspan() else {
        return decline("first statement is not the loop variable's Set");
    };
    let Value::Block(it) = iter.unspan() else {
        return decline("iterator is not a block");
    };
    if it.name != "Iter range" {
        return decline(&format!("iterator block is `{}`, not a range", it.name));
    }
    // loft#1525 — an INCLUSIVE range decides its stop on the value just YIELDED, so its
    // iterator carries one extra leading statement: `if index == hi { break }`, emitted before
    // the step so the loop never has to represent `hi + 1`.  Peel it, because the loop under it
    // is the same fill: the guard changes when the loop ENDS, not which elements it writes.
    //
    // Safe to drop rather than carry, and `fill_hoisted` is why — it declines (returns false,
    // falling back to this very loop) whenever the span does not fit the vector, including the
    // `end >= h.len` and `u32::try_from` edges an unbounded end reaches.  So the fill applies
    // only where the guard would never have fired, and where it would fire the fallback is the
    // corrected loop.
    let ops: &[Value] = match &it.operators[..] {
        [first, rest @ ..] if rest.len() >= 3 && is_inclusive_stop_guard(first, data) => rest,
        all => all,
    };
    // The three statements and the yield, in one of the two orders.
    let (test, seed_index, step, yielded, next_var) = match ops {
        [a, b, c, d] => {
            // § V-ab: [if …; index = next; next = next + 1; index]
            let Value::Set(xi, from) = b.unspan() else {
                return decline("two-counter form: second statement is not the index Set");
            };
            let Value::Var(nx) = from.unspan() else {
                return decline("two-counter form: the index is not set from a variable");
            };
            let Value::Set(nx2, step) = c.unspan() else {
                return decline("two-counter form: third statement is not the step Set");
            };
            if nx2 != nx {
                return decline("two-counter form: the step is on another variable");
            }
            (a, *xi, step.unspan(), d, Some(*nx))
        }
        [a, b, c] => {
            // P3b: [index = index + 1; if …; index]
            let Value::Set(xi, step) = a.unspan() else {
                return decline("single-counter form: first statement is not the step Set");
            };
            (b, *xi, step.unspan(), c, None)
        }
        _ => return decline(&format!("iterator has {} statements", ops.len())),
    };
    let counter = next_var.unwrap_or(seed_index);
    let loop_var = *loop_var;
    // The step: counter = counter + 1.
    let Value::Call(sd, sargs) = step else {
        return decline("the step is not a call");
    };
    if data.def(*sd).name() != "OpAddInt"
        || sargs.len() != 2
        || !matches!(sargs[0].unspan(), Value::Var(c) if *c == counter)
        || !matches!(sargs[1].unspan(), Value::Int(1))
    {
        return decline("the step is not `counter + 1`");
    }
    // The test: if hi <cmp> counter { break } else null.
    let Value::If(cond, on_true, on_false) = test.unspan() else {
        return decline("the test is not an if");
    };
    if !is_break_block(on_true) || !matches!(on_false.unspan(), Value::Null) {
        return decline("the test does not break");
    }
    let Value::Call(cd, cargs) = cond.unspan() else {
        return decline("the test's condition is not a call");
    };
    let inclusive = match data.def(*cd).name() {
        "OpLtInt" => true,
        "OpLeInt" => false,
        _ => return decline("the test is not OpLtInt/OpLeInt"),
    };
    if cargs.len() != 2 || !matches!(cargs[1].unspan(), Value::Var(c) if *c == counter) {
        return decline("the test is not against the counter");
    }
    // The yield: the index variable.
    if !matches!(yielded.unspan(), Value::Var(y) if *y == seed_index) {
        return decline("the iterator does not yield the index");
    }
    Ok(RangeCounters {
        loop_var,
        index: seed_index,
        next: next_var,
        inclusive,
        hi: &cargs[0],
    })
}

/// `@FR-R-BoundedNest` — an innermost counted loop whose body is ONE accumulate over
/// `?`-discharged element reads: `acc = acc + <term>`, the term a chain of `+`, `-`, `*` and
/// negation over literals, the loop's counters, variables the loop does not write, and reads
/// `v[<index chain>]?` of `integer` vectors (the `ncc` block the parser lowers the discharge
/// to).  The emitter runs the nest with PLAIN operators when a guard, evaluated once at the
/// loop's entry, proves that no operation in it can fault: every invariant is not the
/// sentinel, every element bound is known, and the magnitude bound of every chain and of the
/// accumulate over the trip count fits `i64`.
pub struct BoundedNest<'a> {
    pub counters: RangeCounters<'a>,
    /// The accumulator: set once per trip to `acc + term`.
    pub acc: u16,
    pub term: &'a Value,
    /// Each read's vector path and its index chain, in body order.
    pub reads: Vec<(PathKey, &'a Value)>,
    /// The variables the chains read that are not the loop's counters — invariant for the
    /// loop's extent, since the body writes only `acc`.
    pub invariants: Vec<u16>,
    /// Every read's index chain names a counter at most ONCE, so each is affine in the
    /// counter and its extremes over the range are at the range's two ends — the fact the
    /// guard's in-range clause (raw reads, step 2) rests on.
    pub affine: bool,
}

/// Parse `lp` as a bounded nest, or say which part of the shape it is not.
///
/// The fallback of every `_ =>` below is a DECLINE: a node this matcher does not name is one
/// whose plain form it cannot vouch for (a division faults on zero, a shift on its width, a
/// call may do anything), and declining keeps the checked loop, which is always right.
///
/// # Errors
///
/// The reason the loop is not a bounded nest — what `LOFT_TRACE_NEST=1` prints.
pub fn bounded_nest<'a>(lp: &'a Block, data: &Data) -> Result<BoundedNest<'a>, String> {
    if lp.name != "For loop" {
        return Err("not a for loop".to_string());
    }
    let counters = range_counters(lp, data)?;
    if counters.inclusive {
        return Err("an inclusive range".to_string());
    }
    if !matches!(counters.hi.unspan(), Value::Var(_) | Value::Int(_)) {
        return Err("the range's end is not a variable or a literal".to_string());
    }
    let Value::Block(body) = lp.operators[1].unspan() else {
        return Err("the body is not a block".to_string());
    };
    // A statement carries its `Value::Line` marker beside it; only the statement counts.
    let stmts: Vec<&Value> = body
        .operators
        .iter()
        .map(Value::unspan)
        .filter(|v| !matches!(v, Value::Line(_)))
        .collect();
    let [stmt] = stmts[..] else {
        return Err(format!("the body has {} statements, not one", stmts.len()));
    };
    let Value::Set(acc, rhs) = stmt else {
        return Err("the body is not an assignment".to_string());
    };
    let Value::Call(d, args) = rhs.unspan() else {
        return Err("the assignment is not an add".to_string());
    };
    if data.def(*d).name() != "OpAddInt"
        || args.len() != 2
        || !matches!(args[0].unspan(), Value::Var(a) if a == acc)
    {
        return Err("the assignment is not `acc = acc + term`".to_string());
    }
    let mut counter_vars = vec![counters.loop_var, counters.index];
    if let Some(nx) = counters.next {
        counter_vars.push(nx);
    }
    let mut nest = BoundedNest {
        counters,
        acc: *acc,
        term: &args[1],
        reads: Vec::new(),
        invariants: Vec::new(),
        affine: false,
    };
    nest_chain(&args[1], data, &counter_vars, *acc, &mut nest, false)?;
    if nest.reads.is_empty() {
        return Err("the term reads no vector".to_string());
    }
    nest.affine = nest
        .reads
        .iter()
        .all(|(_, chain)| counter_mentions(chain, &counter_vars) <= 1);
    Ok(nest)
}

/// How many times `v` names one of the loop's counters.
fn counter_mentions(v: &Value, counters: &[u16]) -> usize {
    let mut n = 0;
    v.any_node(&mut |node| {
        if let Value::Var(x) = node
            && counters.contains(x)
        {
            n += 1;
        }
        false
    });
    n
}

/// One chain of a bounded nest: the term, or a read's index (`in_index`, where a nested read
/// is not admitted).  Records the reads and the invariants it meets.
fn nest_chain<'a>(
    v: &'a Value,
    data: &Data,
    counters: &[u16],
    acc: u16,
    nest: &mut BoundedNest<'a>,
    in_index: bool,
) -> Result<(), String> {
    match v.unspan() {
        Value::Int(_) => Ok(()),
        Value::Var(x) if *x == acc => Err("the accumulator appears inside the term".to_string()),
        Value::Var(x) => {
            if !counters.contains(x) && !nest.invariants.contains(x) {
                nest.invariants.push(*x);
            }
            Ok(())
        }
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            match (data.def(*d).name(), args.len()) {
                ("OpAddInt" | "OpMinInt" | "OpMulInt", 2) => {
                    nest_chain(&args[0], data, counters, acc, nest, in_index)?;
                    nest_chain(&args[1], data, counters, acc, nest, in_index)
                }
                ("OpMinSingleInt", 1) => nest_chain(&args[0], data, counters, acc, nest, in_index),
                (name, _) => Err(format!("`{name}` is not a nest operator")),
            }
        }
        Value::Block(bl) if bl.name == "ncc" && !in_index => {
            // `__ncc = OpGetInt(OpGetVectorNullable(v, 8, idx), 0); if __ncc != null { __ncc } else { 0 }`
            let ops: Vec<&Value> = bl.operators.iter().map(Value::unspan).collect();
            let [Value::Set(t, read), Value::If(cond, on_some, on_none)] = ops[..] else {
                return Err("a discharge block of another shape".to_string());
            };
            let Value::Call(gd, gargs) = read.unspan() else {
                return Err("the discharge does not read a call".to_string());
            };
            if data.def(*gd).name() != "OpGetInt"
                || gargs.len() != 2
                || !matches!(gargs[1].unspan(), Value::Int(0))
            {
                return Err("the discharge is not an `integer` field read".to_string());
            }
            let Value::Call(vd, vargs) = gargs[0].unspan() else {
                return Err("the discharge does not read an element".to_string());
            };
            if data.def(*vd).name() != "OpGetVectorNullable"
                || vargs.len() != 3
                || !matches!(vargs[1].unspan(), Value::Int(8))
            {
                return Err(
                    "the element read is not an eight-byte `OpGetVectorNullable`".to_string(),
                );
            }
            let Some(path) = vector_path(data, &vargs[0]) else {
                return Err("the read's vector is not a pure path".to_string());
            };
            if path.0 == acc || counters.contains(&path.0) {
                return Err("the read's vector is the accumulator or a counter".to_string());
            }
            let some_ok = matches!(cond.unspan(), Value::Call(cd, cargs)
                if data.def(*cd).name() == "OpConvBoolFromInt" && cargs.len() == 1
                    && matches!(cargs[0].unspan(), Value::Var(c) if c == t));
            if !some_ok
                || !matches!(on_some.unspan(), Value::Var(s) if s == t)
                || !matches!(on_none.unspan(), Value::Int(0))
            {
                return Err("the discharge does not select the element or 0".to_string());
            }
            nest_chain(&vargs[2], data, counters, acc, nest, true)?;
            nest.reads.push((path, &vargs[2]));
            Ok(())
        }
        other => Err(format!("`{}` is not a nest node", kind_of(other))),
    }
}

/// The vector paths every bounded nest under `body` reads and `body` leaves ALONE — what a
/// loop's prelude derives an element bound for beside the header (`@FR-R-BoundedNest`).
///
/// A header survives an in-place element write (the record does not move); a magnitude
/// bound does not (the element did).  So a path is dropped when anything in `body` can
/// change or hand out its elements: a rebind of its root, or its root reaching any op that
/// is not a plain read — an element or field SET (whose target is a projection over the
/// root), an append, a user call that could write through the parameter, a fn-ref call.
/// The reads are an ALLOW-list (`OpGet*`, `OpLength*`, `OpConv*`): an op missing from it
/// costs the bound, never correctness.
#[must_use]
pub fn nest_read_paths(body: &Block, data: &Data) -> HashSet<PathKey> {
    let mut out = HashSet::new();
    let mut touched: HashSet<u16> = HashSet::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            match n {
                Value::Loop(lp) => {
                    if let Ok(nest) = bounded_nest(lp, data) {
                        out.extend(nest.reads.iter().map(|(p, _)| p.clone()));
                    }
                }
                Value::Set(v, _) | Value::TuplePut(v, ..) => {
                    touched.insert(*v);
                }
                Value::Call(d, args) => {
                    let read_only = (*d as usize) < data.definitions.len() && {
                        let name = data.def(*d).name();
                        !matches!(data.def(*d).code(), Value::Block(_))
                            && (name.starts_with("OpGet")
                                || name.starts_with("OpLength")
                                || name.starts_with("OpConv"))
                    };
                    if !read_only {
                        for a in args {
                            if let Some(root) = projection_root(data, a) {
                                touched.insert(root);
                            }
                        }
                    }
                }
                Value::CallRef(_, args) => {
                    for a in args {
                        if let Some(root) = projection_root(data, a) {
                            touched.insert(root);
                        }
                    }
                }
                _ => {}
            }
            false
        });
    }
    out.retain(|p| !touched.contains(&p.0));
    out
}

/// The variable at the root of a projection chain — a bare variable, or `OpGet*` calls
/// (field, element, scalar) over one — or `None` for anything else.  The fallback is a
/// non-answer, not a clearance: the callers above mark roots to EXCLUDE, so a shape this
/// does not see through leaves its root unmarked only when no variable stands at it.
fn projection_root(data: &Data, v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(x) => Some(*x),
        Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name().starts_with("OpGet")
                && !args.is_empty() =>
        {
            projection_root(data, &args[0])
        }
        _ => None,
    }
}

pub fn fill_loop<'a>(lp: &'a Block, data: &Data) -> Option<FillLoop<'a>> {
    let trace = std::env::var("LOFT_TRACE_FILL").is_ok();
    let decline = |why: &str| -> Option<FillLoop<'a>> {
        if trace {
            eprintln!("fill: loop {} declined — {why}", lp.scope);
        }
        None
    };
    let kinds = |ops: &[Value]| -> String {
        ops.iter()
            .map(|o| kind_of(o.unspan()))
            .collect::<Vec<_>>()
            .join(", ")
    };
    if lp.name != "For loop" {
        return None;
    }
    let rc = match range_counters(lp, data) {
        Ok(rc) => rc,
        Err(why) => return decline(&why),
    };
    let (loop_var, seed_index, next_var, inclusive, hi) =
        (rc.loop_var, rc.index, rc.next, rc.inclusive, rc.hi);
    // The body: one fusable scalar set over the loop variable's index.
    let Value::Block(body) = lp.operators[1].unspan() else {
        return decline("the body is not a block");
    };
    // A source-line marker is not a statement (a library module's body carries one).
    let stmts: Vec<&Value> = body
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    if stmts.len() != 1 {
        return decline(&format!(
            "the body has {} statements: {}",
            stmts.len(),
            kinds(&body.operators)
        ));
    }
    let Value::Call(setter, wargs) = stmts[0].unspan() else {
        return decline("the body is not a call");
    };
    let Some((_, rust_type)) = FUSABLE_SETTERS
        .iter()
        .find(|(name, _)| *name == data.def(*setter).name())
    else {
        return decline(&format!(
            "the body is `{}`, not a fusable setter",
            data.def(*setter).name()
        ));
    };
    let [inner, fld, val] = &wargs[..] else {
        return decline("the setter does not take three operands");
    };
    if !matches!(fld.unspan(), Value::Int(0)) {
        return decline("the setter's field is not 0");
    }
    let Value::Call(addr, aargs) = inner.unspan() else {
        return decline("the address is not a call");
    };
    if !is_element_address(data, *addr) {
        return decline("the address is not an element address");
    }
    let [vector, size, index] = &aargs[..] else {
        return decline("the address does not take three operands");
    };
    let Value::Int(size) = size.unspan() else {
        return decline("the element size is not a literal");
    };
    let width: u32 = match *rust_type {
        "i64" | "f64" => 8,
        "f32" => 4,
        _ => return None,
    };
    if u32::try_from(*size).ok() != Some(width) {
        return decline("the element size is not the scalar's width");
    }
    let Some(path) = vector_path(data, vector) else {
        return decline("the vector is not a pure path");
    };
    let mut banned = vec![loop_var, seed_index];
    if let Some(nx) = next_var {
        banned.push(nx);
    }
    banned.push(path.0);
    // The index: the loop variable, or `invariant + variable` either way round.
    let base = match index.unspan() {
        Value::Var(x) if *x == loop_var => None,
        Value::Call(d, args) if data.def(*d).name() == "OpAddInt" && args.len() == 2 => {
            let is_var = |a: &Value| matches!(a.unspan(), Value::Var(x) if *x == loop_var);
            if is_var(&args[0]) && simple_invariant(&args[1], data, &banned) {
                Some(&args[1])
            } else if is_var(&args[1]) && simple_invariant(&args[0], data, &banned) {
                Some(&args[0])
            } else {
                return decline("the index is not `invariant + loop variable`");
            }
        }
        _ => return decline("the index is neither the loop variable nor a sum"),
    };
    if !simple_invariant(val, data, &banned) {
        return decline("the value is not a simple invariant");
    }
    if !simple_invariant(hi, data, &banned) {
        return decline("the range's end is not a simple invariant");
    }
    Some(FillLoop {
        path,
        vector,
        rust_type,
        size: width,
        base,
        hi,
        inclusive,
        index_var: seed_index,
        next_var,
        val,
    })
}

/// @PLN157 § V-am (`@FR-R-PushFill`) — a counted loop that PUSHES: `k` scalar pushes to
/// one pure path at the top level of its body, every iteration, with nothing that can
/// leave the loop early and no other write reaching the path.  The reserve form holds for
/// any such loop (`pushes` × the trip count, once, before the loop); the FILL form is the
/// body that is that one push of a simple invariant and nothing else.
pub struct PushLoop<'a> {
    pub path: PathKey,
    pub vector: &'a Value,
    pub rust_type: &'static str,
    pub size: u32,
    pub hi: &'a Value,
    pub inclusive: bool,
    pub index_var: u16,
    pub next_var: Option<u16>,
    /// The pushes per iteration.
    pub pushes: u32,
    /// The invariant value when the body is ONE push of it — the fill form.
    pub fill: Option<&'a Value>,
}

/// Recognise the counted push loop (`@FR-R-PushFill`).  Declines, and says why under
/// `LOFT_TRACE_PUSH_FILL=1`, whenever the trip count cannot be known before the loop runs
/// or the pushes per iteration cannot be counted: a `break`, a `return`, a `continue`, an
/// inner loop, a push under a branch, a push to a second path, an append or any other
/// write reaching the path, a range end that is not a simple invariant.  The fallback is
/// `None` — the loop runs as it did, which costs the reserve and never a value.
#[must_use]
pub fn push_loop<'a>(lp: &'a Block, data: &Data) -> Option<PushLoop<'a>> {
    let trace = std::env::var("LOFT_TRACE_PUSH_FILL").is_ok();
    let decline = |why: &str| -> Option<PushLoop<'a>> {
        if trace {
            eprintln!("push-fill: loop {} declined — {why}", lp.scope);
        }
        None
    };
    if lp.name != "For loop" {
        return None;
    }
    let rc = match range_counters(lp, data) {
        Ok(rc) => rc,
        Err(why) => return decline(&why),
    };
    let Value::Block(body) = lp.operators[1].unspan() else {
        return decline("the body is not a block");
    };
    let stmts: Vec<&Value> = body
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let push_kind = |d: &u32| -> Option<(&'static str, u32)> {
        if (*d as usize) >= data.definitions.len() {
            return None;
        }
        let name = data.def(*d).name();
        FUSABLE_PUSHES
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, rt, w)| (*rt, *w))
    };
    // The pushes at the top level of the body.
    let mut path: Option<PathKey> = None;
    let mut vector: Option<&'a Value> = None;
    let mut rust_type = "";
    let mut size = 0u32;
    let mut vals: Vec<&'a Value> = Vec::new();
    for s in &stmts {
        if let Value::Call(d, args) = s.unspan()
            && let Some((rt, w)) = push_kind(d)
            && args.len() == 2
            && let Some(p) = vector_path(data, &args[0])
        {
            match &path {
                Some(pp) if *pp != p => return decline("the pushes reach two paths"),
                Some(_) => {}
                None => {
                    path = Some(p);
                    vector = Some(&args[0]);
                    rust_type = rt;
                    size = w;
                }
            }
            vals.push(&args[1]);
        }
    }
    let (Some(path), Some(vector)) = (path, vector) else {
        return decline("no push at the top level of the body");
    };
    // The statements that are neither a counted push nor the path's own reservation (which
    // the parser emits BEFORE the push, so it is counted once the path is known).
    let plain = stmts
        .iter()
        .filter(|s| {
            !matches!(s.unspan(), Value::Call(d, a)
                if (push_kind(d).is_some()
                    || ((*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpPreAllocVector"))
                    && a.first().and_then(|f| vector_path(data, f)).as_ref() == Some(&path))
        })
        .count();
    let mut early = false;
    lp.operators[1].any_node(&mut |n| {
        if matches!(
            n,
            Value::Break(_)
                | Value::Return(_)
                | Value::Continue(_)
                | Value::Loop(_)
                | Value::Yield(_)
                | Value::Parallel(_)
        ) {
            early = true;
            return true;
        }
        false
    });
    if early {
        return decline("the body can leave early, or loops");
    }
    // Every push to the path is one of the counted ones, and nothing else writes it.
    let mut all = 0usize;
    let mut other = false;
    lp.operators[1].any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && let Some(first) = args.first()
            && vector_path(data, first).as_ref() == Some(&path)
        {
            let name = data.def(*d).name();
            if push_kind(d).is_some() {
                all += 1;
            } else if !(name == "OpPreAllocVector"
                || name.starts_with("OpGet")
                || name.starts_with("OpLength"))
            {
                other = true;
            }
        }
        false
    });
    if other {
        return decline("another write reaches the path");
    }
    if all != vals.len() {
        return decline("a push stands under a branch");
    }
    let mut banned = vec![rc.loop_var, rc.index, path.0];
    if let Some(nx) = rc.next {
        banned.push(nx);
    }
    if !simple_invariant(rc.hi, data, &banned) {
        return decline("the range's end is not a simple invariant");
    }
    let fill = if vals.len() == 1 && plain == 0 && simple_invariant(vals[0], data, &banned) {
        Some(vals[0])
    } else {
        None
    };
    let pushes = u32::try_from(vals.len()).ok()?;
    Some(PushLoop {
        path,
        vector,
        rust_type,
        size,
        hi: rc.hi,
        inclusive: rc.inclusive,
        index_var: rc.index,
        next_var: rc.next,
        pushes,
        fill,
    })
}

/// Recognise `OpPreAllocVector(path, count, size)` over a pure path (@PLN157 § V-q): the
/// reservation the parser emits before a push to a LOCAL vector.  It claims a record only
/// for an ABSENT vector and never moves an existing one, so it cannot stale a header; with a
/// push header active for the path it is redundant (the push grows on demand) and is
/// emitted as nothing.
#[must_use]
pub fn pre_alloc_path(data: &Data, op: &str, args: &[Value]) -> Option<PathKey> {
    if op != "OpPreAllocVector" || args.len() != 3 {
        return None;
    }
    vector_path(data, &args[0])
}

/// Recognise `OpPush<Kind>(path, val)` for a fusable kind over a pure path (@PLN157 § V-q)
/// — the ONE definition of the hoistable push, asked by the gate, the collector and the
/// emitter.  Shape only; the caller confirms the loop hoisted a push header for the path.
#[must_use]
pub fn fused_push<'a>(data: &Data, op: &str, args: &'a [Value]) -> Option<FusedPush<'a>> {
    let (_, rust_type, size) = FUSABLE_PUSHES.iter().find(|(n, _, _)| *n == op)?;
    if args.len() != 2 {
        return None;
    }
    let path = vector_path(data, &args[0])?;
    Some(FusedPush {
        path,
        vector: &args[0],
        val: &args[1],
        rust_type,
        size: *size,
    })
}

/// Recognise one op of the record MINT group — `OpNewRecord(P, tp, fld)` /
/// `OpFinishRecord(P, elm, tp, fld)` appending one element to a PLAIN vector named by a
/// bare variable (@PLN157 § V-s, `@FR-R-Mint`) — the ONE definition of the admissible
/// mint, asked by the gate, the collector and the write-set walk.
///
/// The container must TYPE as `Type::Vector` (the V-m lesson: ask the type, not the op
/// shape) — on a keyed collection (`sorted`, `index`, `hash`, …) the same op names are a
/// KEYED INSERT, a mover of OTHER records, and stay blocking.  `peel_link` and not
/// `base()`: a nullable vector's append is not this unit's shape, so `τ?` stays out.  A
/// FIELD path (`h.pts += […]`) also answers `None` — this unit admits the bare-variable
/// root the raster loops use; widening to field paths is a measured decision for later.
#[must_use]
pub fn mint_path(
    data: &Data,
    op: &str,
    args: &[Value],
    vars: &crate::variables::Function,
) -> Option<PathKey> {
    let arity = match op {
        "OpNewRecord" => 3,
        "OpFinishRecord" => 4,
        _ => return None,
    };
    if args.len() != arity {
        return None;
    }
    let path = vector_path(data, &args[0])?;
    if !path.1.is_empty() || path.0 >= vars.count() {
        return None;
    }
    matches!(vars.tp(path.0).peel_link(), Type::Vector(_, _)).then_some(path)
}

/// @PLN157 § V-t (`@FR-R-PushRec`) — may this mint group EMIT through a push header?  The
/// schema is asked, not the op shape: the parent must be a plain inline-element vector
/// (`Parts::Vector` — an `array`/`ordered` conversion holds 4-byte handles, and a keyed
/// container's same-named ops place records), the form the tail-slot append (`fld ==
/// u16::MAX`), and the element a plain struct that owns no heap — a raw slot carries stale
/// bytes, and a heap handle is the one field kind whose stale bytes something could walk
/// before the group's writes land.  The group's writes cover every scalar field explicitly
/// (the IR's literal lowering emits omitted fields' defaults and sentinels itself; a
/// declined delivery is a whole-record copy), so no prefill is owed.
#[must_use]
pub fn mint_push_qualifies(stores: &Stores, args: &[Value]) -> bool {
    let (Some(Value::Int(tp)), Some(Value::Int(fld))) = (
        args.get(1).map(Value::unspan),
        args.get(2).map(Value::unspan),
    ) else {
        return false;
    };
    if *fld != i32::from(u16::MAX) {
        return false;
    }
    let Ok(tp) = u16::try_from(*tp) else {
        return false;
    };
    if !stores.is_plain_vector(tp) {
        return false;
    }
    let elem = stores.content(tp);
    elem != u16::MAX && stores.is_struct(elem) && !stores.owns_heap(elem)
}

const FUSABLE_SETTERS: [(&str, &str); 3] = [
    ("OpSetInt", "i64"),
    ("OpSetSingle", "f32"),
    ("OpSetFloat", "f64"),
];

/// An element write the emitter can collapse into ONE store: a scalar setter writing
/// field `fld` of `vector[index]`, where the vector is a plain variable.
pub struct FusedWrite<'a> {
    /// The vector operand — a `Var` or a pure `OpGetField` chain over one, so it
    /// re-emits without side effects.
    pub vector: &'a Value,
    pub path: PathKey,
    pub size: &'a Value,
    pub index: &'a Value,
    pub fld: &'a Value,
    pub val: &'a Value,
    /// Rust type of the store, e.g. `"f64"`.
    pub rust_type: &'static str,
}

/// Recognise `OpSet<scalar>(OpGetVector*(Var(v), size, index), fld, val)`, given the
/// setter's op NAME — the write twin of [`fused_element_read`], and like it the ONE
/// definition of the fused shape: the emitter and the pre-eval collector both ask here,
/// so the pre-eval cannot lift an inner element address the emitter then folds away
/// (which would resolve the element twice).
///
/// The caller still has to confirm the vector HAS a hoisted header; this only reports
/// shape.
#[must_use]
pub fn fused_element_write<'a>(
    data: &Data,
    setter: &str,
    args: &'a [Value],
) -> Option<FusedWrite<'a>> {
    let [inner, fld, val] = args else { return None };
    let (_, rust_type) = FUSABLE_SETTERS.iter().find(|(name, _)| *name == setter)?;
    let Value::Call(elem_op, elem_args) = inner.unspan() else {
        return None;
    };
    if !is_element_address(data, *elem_op) {
        return None;
    }
    let [vector, size, index] = &elem_args[..] else {
        return None;
    };
    let path = vector_path(data, vector)?;
    Some(FusedWrite {
        vector,
        path,
        size,
        index,
        fld,
        val,
        rust_type,
    })
}

/// How many parameters `d_nr` declares, as [`call_writes_store`] counts them.
///
/// Exposed for the test that pins where a native op's parameters live. The verdict "this
/// op cannot name a store" is read off that list, so an empty list is indistinguishable
/// from "takes only scalars" — and every mutator would read as a reader.
#[must_use]
pub fn parameters_declared(data: &Data, d_nr: u32) -> usize {
    if (d_nr as usize) >= data.definitions.len() {
        return 0;
    }
    data.def(d_nr).attributes().len()
}

/// True when `d_nr` is one of the element-address ops a header serves.
#[must_use]
pub fn is_element_address(data: &Data, d_nr: u32) -> bool {
    (d_nr as usize) < data.definitions.len() && ELEMENT_ADDRESS_OPS.contains(&data.def(d_nr).name())
}

/// Could running `node` write any store?
///
/// Descends through called functions, so a loop calling a stdlib reader (`len(v)` is
/// `t_6vector_len`, whose whole body is `OpLengthVector`) still qualifies. A recursion
/// cycle, a call through a runtime fn-ref, a parallel arm and a `yield` all answer yes —
/// the first because the fixed point is not worth computing here, the rest because what
/// runs is not this body.
///
/// `cache` memoises per definition. A `true` may have come from a broken recursion cycle
/// and is still sound to reuse (it only declines a hoist); a `false` cannot have, because
/// a cycle contributes `true` and any caller of it answers `true` too.
pub fn may_write_store(node: &Value, data: &Data, cache: &mut HashMap<u32, bool>) -> bool {
    writes_store(node, data, cache, &mut HashSet::new(), None)
}

fn writes_store(
    node: &Value,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    vars: Option<&crate::variables::Function>,
) -> bool {
    blocks_header_hoist(
        node,
        data,
        cache,
        active,
        vars,
        HoistTiers::default(),
        &mut HashSet::new(),
    )
}

/// @PLN157 § V-c — the frees whose operand is a RECORD variable of the enclosing body.
///
/// A free releases exactly one store and moves no other, and the store a hoisted header
/// describes belongs to a loop-invariant vector that is live across the loop — so a
/// record's release cannot be it.  The operand's TYPE is what carries the argument: a
/// vector-typed operand (a per-iteration vector local, or a vector work-ref paired by
/// loft#1201) keeps the writer verdict, and so does a free whose body this cannot see
/// (`vars == None`).  `OpFreeRefIfDistinct(v, w)` compares `v` against a witness and
/// releases `v` alone, so only its first operand is the question.
const RECORD_FREE_OPS: [&str; 2] = ["OpFreeRef", "OpFreeRefIfDistinct"];

/// Is `def_nr` a record type with no heap in it — every field a fixed-width scalar (a
/// constant and a routine field hold no store either)?  The record's store then hosts
/// nothing a hoisted header could name: the fact `@FR-R-Callee`'s return-buffer half and
/// § V-ad's discharge buffer both stand on.
fn all_scalar_record(data: &Data, def_nr: u32) -> bool {
    data.def(def_nr).attributes().iter().all(|a| {
        a.constant || matches!(a.typedef.base(), Type::Routine(_)) || is_scalar(&a.typedef)
    })
}

/// @PLN157 § V-ad — `OpDatabase`/`OpDatabaseNP` into a hidden null-discharge buffer
/// (`__ref_p2_N`, the record `e = tbl[i]?` mints an ABSENT element into) whose record is
/// all-scalar: the allocation takes a store of its own from a null slot, or clears the
/// buffer's OWN store, and that store hosts no vector, text or reference — so no header
/// can go stale and no scalar hoist can be reached except through the buffer's own type.
/// Answers that type.  Only the pass-2 discharge buffers qualify: a `__ref_N` work-ref may
/// be a return buffer, and a return buffer may be a record the caller offered.
/// Enforces `@FR-R-InPlace` (the hidden-buffer allowance).
fn null_buffer_alloc(
    name: &str,
    args: &[Value],
    vars: Option<&crate::variables::Function>,
    data: &Data,
) -> Option<u16> {
    if !crate::keys::null_buffer_hoist_enabled()
        || !(name == "OpDatabase" || name == "OpDatabaseNP")
    {
        return None;
    }
    let vars = vars?;
    let Some(Value::Var(b)) = args.first().map(Value::unspan) else {
        return None;
    };
    if *b >= vars.count() || !vars.name(*b).starts_with("__ref_p2_") {
        return None;
    }
    let tp = plain_record_type(data, vars.tp(*b))?;
    let def_nr = vars.tp(*b).heap_def_nr()?;
    all_scalar_record(data, def_nr).then_some(tp)
}

fn frees_a_record(name: &str, args: &[Value], vars: Option<&crate::variables::Function>) -> bool {
    let Some(vars) = vars else { return false };
    RECORD_FREE_OPS.contains(&name)
        && matches!(args.first().map(Value::unspan), Some(Value::Var(v))
            if *v < vars.count()
                && matches!(vars.tp(*v).base(), Type::Reference(_, _) | Type::Enum(_, true, _)))
}

/// Does running `node` invalidate a hoisted header?  [`writes_store`] with one
/// extra allowance: under `allow_in_place`, a direct [`IN_PLACE_SET_OPS`] call is
/// not blocking (its target and value subtrees still walk, so a growing op INSIDE
/// either of them blocks on its own).  A user CALL that writes stays blocking even
/// when its writes happen to be in-place — interprocedural in-place classification
/// is not worth its soundness surface here.
/// Enforces `@FR-R-InPlace` and admits a callee under `@FR-R-Callee`.
fn blocks_header_hoist(
    node: &Value,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
    vars: Option<&crate::variables::Function>,
    tiers: HoistTiers,
    fresh: &mut HashSet<u16>,
) -> bool {
    node.any_node(&mut |n| match n {
        // @PLN157 § V-s (`@FR-R-Mint`) — track the element variables the body has minted
        // so far, in the same preorder the write-set walk uses (see `body_writes` for why
        // that order is the execution order of the group).
        Value::Set(v, inner) => {
            if tiers.mint
                && matches!(inner.unspan(), Value::Call(d, args)
                    if (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpNewRecord"
                        && vars.is_some_and(|vs| mint_path(data, "OpNewRecord", args, vs).is_some()))
            {
                fresh.insert(*v);
            } else {
                fresh.remove(v);
            }
            false
        }
        Value::Call(d, args) => {
            let known = (*d as usize) < data.definitions.len();
            let in_place_setter =
                known && tiers.in_place && IN_PLACE_SET_OPS.contains(&data.def(*d).name());
            let record_free = known
                && crate::keys::retbuf_hoist_enabled()
                && frees_a_record(data.def(*d).name(), args, vars);
            // @PLN157 § V-ad — the null-discharge buffer's allocation moves nothing a header
            // describes; its field sets below are in-place and walk on their own.
            let buffer_alloc = known
                && (null_buffer_alloc(data.def(*d).name(), args, vars, data).is_some()
                    || lazy_buffer_mint(data.def(*d).name(), args, vars));
            // @PLN157 § V-q (`@FR-R-Push`) — a fusable push over a pure path is admitted
            // under its own tier: it grows one vector whose header the loop keeps current
            // through the push itself; `hoistable` decides the aliasing.  The value operand
            // still walks below this node.
            let fusable_push = known
                && tiers.push
                && (fused_push(data, data.def(*d).name(), args).is_some()
                    || pre_alloc_path(data, data.def(*d).name(), args).is_some());
            // @PLN157 § V-s (`@FR-R-Mint`) — a record MINT into a plain vector is a mover
            // like a push: it grows that one vector and initialises a record no variable
            // bound before the loop can name; `hoistable` decides the aliasing.  The
            // element's field writes and the builder call still walk below this node.
            // § V-d's delivery tail — `OpCopyRecord(result, elm, tp)` when the builder
            // could not build in place — writes the fresh element and frees the builder's
            // own this-iteration buffer, neither of which a prelude holder can name; it is
            // admitted only while its DESTINATION is a fresh mint variable.
            let record_mint = known
                && tiers.mint
                && vars.is_some_and(|v| mint_path(data, data.def(*d).name(), args, v).is_some());
            let fresh_delivery = known
                && tiers.mint
                && data.def(*d).name() == "OpCopyRecord"
                && args.len() == 3
                && matches!(args[1].unspan(), Value::Var(e) if fresh.contains(e));
            if record_mint
                && data.def(*d).name() == "OpFinishRecord"
                && let Some(Value::Var(e)) = args.get(1).map(Value::unspan)
            {
                fresh.remove(e);
            }
            if in_place_setter
                || record_free
                || buffer_alloc
                || fusable_push
                || record_mint
                || fresh_delivery
            {
                false
            } else if call_writes_store(*d, data, cache, active) {
                // @PLN157 § V-l — a USER callee that writes, but only in place: admitted
                // under the same tier as a direct in-place setter, for the same reason
                // (its writes move nothing).  The arguments still walk below this node,
                // so a growing op inside one blocks on its own.
                !(known
                    && tiers.in_place
                    && crate::keys::inplace_callee_hoist_enabled()
                    && in_place_only_writer(*d, data, cache, active))
            } else {
                false
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    })
}

fn call_writes_store(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
) -> bool {
    if (d_nr as usize) >= data.definitions.len() {
        return true;
    }
    if let Some(known) = cache.get(&d_nr) {
        return *known;
    }
    let def = data.def(d_nr);
    let writes = if matches!(def.code(), Value::Null) {
        !native_op_is_store_free(def)
    } else if active.insert(d_nr) {
        let inner = writes_store(def.code(), data, cache, active, Some(def.variables()));
        // @PLN157 § V-c — a body whose only writes land in its own scalar return
        // buffer moves no header a caller could have hoisted.
        let inner = inner
            && !(crate::keys::retbuf_hoist_enabled()
                && retbuf_only_writer(d_nr, data, cache, active));
        active.remove(&d_nr);
        inner
    } else {
        true // recursion — the conservative answer rather than a fixed point
    };
    // Safe to memoise either way: a `true` only ever declines a hoist, and a `false` cannot
    // have come from the branch above, since a cycle contributes `true` to every caller.
    cache.insert(d_nr, writes);
    writes
}

/// @PLN157 § V-l — does this def write stores ONLY through [`IN_PLACE_SET_OPS`]?
///
/// The P4a argument, one call deep: a scalar set through an address it is given moves no
/// record and changes no length, so every header a CALLER hoisted stays valid across the
/// call, whatever the address — an element of a vector reached through a parameter, a
/// record's field, the callee's own local.  Everything else the callee runs must be
/// store-free: a native op that is neither store-free nor one of those setters (a growth,
/// a free, an `OpDatabase`, a text or reference set), a user callee that is neither
/// store-free nor in-place-only itself, and a `CallRef` / `Parallel` / `Yield` (what runs
/// is not this body) each keep the writer verdict; so does recursion, conservatively.
/// The `composite` row's `set_pixel` — three scalar field reads, one element address, one
/// `set_int` through it — is the shape.  Memoised beside [`call_writes_store`]'s answers
/// under [`IN_PLACE_KEY`]; `LOFT_HOIST_VERIFY=1` is the falsifier.
const IN_PLACE_KEY: u32 = 1 << 31;

/// Enforces `@FR-R-Callee` (the in-place-only half).
fn in_place_only_writer(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
) -> bool {
    let key = d_nr | IN_PLACE_KEY;
    if let Some(known) = cache.get(&key) {
        return *known;
    }
    let def = data.def(d_nr);
    if matches!(def.code(), Value::Null) || !active.insert(d_nr) {
        return false;
    }
    let only_in_place = !def.code().any_node(&mut |n| match n {
        Value::Call(op, _) => {
            if (*op as usize) >= data.definitions.len() {
                return true;
            }
            let callee = data.def(*op);
            if matches!(callee.code(), Value::Null) {
                !(native_op_is_store_free(callee) || IN_PLACE_SET_OPS.contains(&callee.name()))
            } else {
                call_writes_store(*op, data, cache, active)
                    && !in_place_only_writer(*op, data, cache, active)
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    });
    active.remove(&d_nr);
    // A verdict reached while a cycle was open is `false` on the recursive edge only, which
    // never admits a hoist; memoising it is safe either way.
    cache.insert(key, only_in_place);
    only_in_place
}

/// @PLN157 § V-c — does this def write nothing but fixed-width scalars into its own
/// hidden return buffer?
///
/// Such a call cannot invalidate a header a caller hoisted: a scalar set through an
/// address moves nothing and changes no length (the `IN_PLACE_SET_OPS` argument, one call
/// deep); `OpDatabase` on the buffer allocates from a null slot or clears the buffer's
/// OWN store, and a store is its own allocation; and the buffer's store hosts no hoisted
/// header, because the record is ALL-SCALAR — no collection, text or reference field —
/// and a loop never names its buffer variable.  Every miss keeps the writer verdict: a
/// record with a vector field (it grows), a write to any other place (a parameter, a
/// local), a native op that is neither store-free nor one of those setters, a user call
/// that writes, and a `CallRef` / `Parallel` / `Yield` (what runs is not this body).
/// Enforces `@FR-R-Callee` (the return-buffer half).
fn retbuf_only_writer(
    d_nr: u32,
    data: &Data,
    cache: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
) -> bool {
    let def = data.def(d_nr);
    let Some(attr) = def.hidden_return_buffer_attr() else {
        return false;
    };
    let Some(record) = def.attributes()[attr].typedef.heap_def_nr() else {
        return false;
    };
    if !all_scalar_record(data, record) {
        return false;
    }
    let buf = def.variables().var(&def.attributes()[attr].name);
    if buf == u16::MAX {
        return false;
    }
    !def.code().any_node(&mut |n| match n {
        Value::Call(op, args) => {
            if (*op as usize) >= data.definitions.len() {
                return true;
            }
            let callee = data.def(*op);
            if matches!(callee.code(), Value::Null) {
                if native_op_is_store_free(callee) {
                    return false;
                }
                let name = callee.name();
                let into_buffer =
                    matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == buf);
                !(into_buffer && (name == "OpDatabase" || IN_PLACE_SET_OPS.contains(&name)))
            } else {
                call_writes_store(*op, data, cache, active)
            }
        }
        Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => true,
        _ => false,
    })
}

/// Can this native op be ruled out as a writer?
///
/// Two ways to qualify, and everything else is assumed to write:
///
/// * it is named above as a constant or a reader; or
/// * it takes at least one parameter and every parameter is a plain runtime scalar.
///
/// The second is the arithmetic, comparison and conversion bulk. What it turns on is that
/// a **`const` parameter is not a value** — it is a compile-time slot number, type id or
/// field offset, and that is precisely the channel through which the scalar-signature ops
/// that DO touch state reach it: `OpDatabase(pos, db_tp)` allocates a store,
/// `OpCoroutineNext(value_size)` resumes a generator that can append to anything,
/// `OpFreeText(pos)` releases one. Read by signature alone those three are
/// indistinguishable from `OpAddInt`; read this way none of them qualifies.
///
/// Parameters come from `attributes()`. A native op has no body and therefore no variable
/// table, so `variables().arguments()` answers the empty list — which "are they all
/// scalar?" accepts, turning every mutator into a reader. That is not a hypothetical: it
/// is how the first cut of this gate let `v.remove(0)` run inside a hoisted loop.
/// `parameters_declared` is the guard that keeps it from coming back.
fn native_op_is_store_free(def: &crate::data::Definition) -> bool {
    if PURE_NULLARY_OPS.contains(&def.name()) || READ_ONLY_COLLECTION_OPS.contains(&def.name()) {
        return true;
    }
    !def.attributes().is_empty()
        && def
            .attributes()
            .iter()
            .all(|a| !a.constant && is_scalar(&a.typedef))
}

/// True for the types that cannot name a store. Anything else — a reference, a collection,
/// text, a tuple, an iterator, an unresolved type — counts as one that can.
fn is_scalar(tp: &Type) -> bool {
    matches!(
        tp.base(),
        Type::Integer(_)
            | Type::Boolean
            | Type::Float
            | Type::Single
            | Type::Character
            | Type::Enum(_, false, _)
    )
}

/// @PLN157 § V-j (`@FR-R-MoveAppend`) — one paired move-append: a `for f in call(…)` whose
/// loop variable's SINGLE use after binding is one append into an owned local vector.  The
/// call's hidden `__ref` buffer is then PLACED as a record inside the destination's own
/// `__vdb` store, the append relocates the element's bytes instead of re-claiming and
/// deep-copying its heap, and the buffer's free is a record-level release inside the store
/// that lives on.
#[derive(Clone, Debug)]
pub struct MoveAppend {
    /// The hidden `__ref` buffer variable the paired call fills.
    pub buf: u16,
    /// The `__vdb` witness variable of the destination — the store the buffer is placed in.
    pub host_vdb: u16,
    /// The buffer's holder type (`main_vector<E>`), for the placement and the record free.
    pub buf_tp: u16,
    /// The loop variable whose single append becomes the move.
    pub loop_var: u16,
    /// The destination vector variable.
    pub dest: u16,
    /// The element stride the move relocates, from the group's own `OpPreAllocVector`.
    pub elem_size: u32,
}

/// The paired move-appends of `def_nr`'s body, keyed by BUFFER variable
/// (@PLN157 § V-j, `@FR-R-MoveAppend`).  Every gate here is an under-approximation on
/// purpose — a declined pairing keeps today's deep copy, which is always correct:
///
/// - the iterated expression is a CALL of a loft-defined function that MINTS its result
///   (`returns_borrowed_view` declines — moving out of a borrowed view would zero a named
///   vector's elements, the V-j c6 cell), delivered through a trailing `__ref` buffer;
/// - the loop variable is bound once by the iterator and appears EXACTLY once after it, as
///   the source of the append group `Set(elm, OpNewRecord(dest)) · OpCopyRecord(f, elm) ·
///   OpFinishRecord(dest, elm)` — a read after the append would see the zeroed source
///   (c2/c3/c7 decline), and a group under a FURTHER loop appends once per inner iteration
///   while the move can only give the element away once (c8's class);
/// - the destination is an owned local plain vector of a PLAIN STRUCT element (an enum /
///   nullable element's discriminant is not bytes this move reasons about), never rebound
///   in the function — a rebind frees the store the buffer was placed in while the buffer
///   is still written through (the placed record must die with the store that hosts it);
/// - the buffer serves ONLY this call: its other appearances are its null declaration and
///   its scope-exit frees.
#[must_use]
pub fn move_appends(data: &Data, def_nr: u32) -> BTreeMap<u16, MoveAppend> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    // The rebind set of the WHOLE function: a destination or buffer reassigned anywhere
    // declines, wherever the For sits.
    let mut set_counts: HashMap<u16, u32> = HashMap::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, to) = n {
            // A declaration's two initialisation Sets are not rebinds: the `= null` decl,
            // and the vector local's binding to its own `__vdb` witness's path
            // (`v = OpGetField(__vdb_N, 0, tp)`).
            let init = match to.unspan() {
                Value::Null => true,
                Value::Call(d, cargs) => {
                    (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpGetField"
                        && matches!(cargs.first().map(Value::unspan),
                            Some(Value::Var(w)) if *w < vars.count() && vars.name(*w).starts_with("__vdb"))
                }
                _ => false,
            };
            if !init {
                *set_counts.entry(*v).or_default() += 1;
            }
        }
        false
    });
    // Ordered by buffer variable: the emitter walks these pairs to release and re-arm a
    // host's placed buffers, and a hash order made that text differ between two runs of
    // one compiler on one program (loft#1535).
    let mut out: BTreeMap<u16, MoveAppend> = BTreeMap::new();
    let mut dead: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && bl.name == "For block"
            && let Some(pair) = pair_for_block(bl, data, vars, &set_counts)
        {
            // A buffer serving TWO paired loops is a buffer this analysis does not own.
            if out.remove(&pair.buf).is_some() || dead.contains(&pair.buf) {
                dead.insert(pair.buf);
            } else {
                out.insert(pair.buf, pair);
            }
        }
        false
    });
    // The buffer's whole-function uses: one call argument (counted inside its own For by
    // construction), plus frees.  Any OTHER appearance — a second call, a read, a copy —
    // declines the pair.
    out.retain(|buf, _| {
        // Reconciled counts, because `any_node` descends into a call's arguments too:
        // every `Var(buf)` in the body must be accounted for as the ONE paired call's
        // argument or as a free's operand — anything else is a use this analysis does
        // not understand, and the pair declines.
        let mut total: u32 = 0;
        let mut call_args: u32 = 0;
        let mut free_args: u32 = 0;
        let mut rebound = false;
        body.any_node(&mut |n| {
            match n {
                Value::Var(v) if v == buf => total += 1,
                Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                    let name = data.def(*d).name();
                    let hits = args
                        .iter()
                        .filter(|a| matches!(a.unspan(), Value::Var(v) if v == buf))
                        .count() as u32;
                    if name == "OpFreeRef" || name == "OpFreeRefTag" {
                        free_args += hits;
                    } else {
                        call_args += hits;
                    }
                }
                // `Set(buf, Null)` is the declaration; any other Set is a rebind.
                Value::Set(v, to) if v == buf && !matches!(to.unspan(), Value::Null) => {
                    rebound = true;
                }
                _ => {}
            }
            false
        });
        let ok = call_args == 1 && total == call_args + free_args && !rebound;
        if !ok && std::env::var("LOFT_TRACE_MOVE").is_ok() {
            eprintln!("[move] buf {} uses: total={total} call_args={call_args} free_args={free_args} rebound={rebound}", vars.name(*buf));
        }
        ok
    });
    out
}

/// The § V-j pairing of ONE `For` block, or `None` — see [`move_appends`] for the gates.
fn pair_for_block(
    bl: &Block,
    data: &Data,
    vars: &crate::variables::Function,
    set_counts: &HashMap<u16, u32>,
) -> Option<MoveAppend> {
    let trace = std::env::var("LOFT_TRACE_MOVE").is_ok();
    // ops = [Set(_vector, Call(d, [.., Var(buf)])), Set(idx, -1), Loop(..)]
    let Some(Value::Set(_, call)) = bl.operators.first().map(Value::unspan) else {
        if trace {
            eprintln!(
                "[move] no leading Set: first={:?}",
                bl.operators.first().map(|v| kind_of(v))
            );
        }
        return None;
    };
    let Value::Call(d, cargs) = call.unspan() else {
        if trace {
            eprintln!("[move] Set rhs not a Call: {}", kind_of(call));
        }
        return None;
    };
    if (*d as usize) >= data.definitions.len() {
        return None;
    }
    let callee = data.def(*d);
    if !callee.is_loft_defined() || callee.returns_borrowed_view() {
        if trace {
            eprintln!(
                "[move] callee {} loft={} borrowed={}",
                callee.name(),
                callee.is_loft_defined(),
                callee.returns_borrowed_view()
            );
        }
        return None;
    }
    // The callee must treat the buffer VECTOR-level.  A callee whose result witness was
    // promoted INTO the buffer parameter re-inits it with `OpDatabase`, and that reuse arm
    // clears the buffer's WHOLE STORE — which under placement is the caller's result store,
    // half-built (the sqldb `collect_leaf` corruption; the mv2 cells pin it).
    if callee_reinits_buffer(data, callee) {
        if trace {
            eprintln!("[move] callee {} re-inits its buffer store", callee.name());
        }
        return None;
    }
    let Some(Value::Var(buf)) = cargs.last().map(Value::unspan) else {
        if trace {
            eprintln!("[move] last arg not Var: {:?}", cargs.last().map(kind_of));
        }
        return None;
    };
    let buf = *buf;
    if buf >= vars.count()
        || !vars.name(buf).starts_with("__ref")
        || !matches!(vars.tp(buf).peel_link(), Type::Vector(_, _))
    {
        if trace {
            eprintln!(
                "[move] buf gate: name={} tp-vector={}",
                vars.name(buf),
                matches!(vars.tp(buf).peel_link(), Type::Vector(_, _))
            );
        }
        return None;
    }
    let lp = bl.operators.iter().find_map(|op| match op.unspan() {
        Value::Loop(lp) => Some(lp),
        _ => None,
    })?;
    let Some(Value::Set(loop_var, _)) = lp.operators.first().map(Value::unspan) else {
        return None;
    };
    let loop_var = *loop_var;
    // One rebind only — the iterator's own Set.  A second write to f anywhere declines.
    if set_counts.get(&loop_var).copied().unwrap_or(0) != 1 {
        return None;
    }
    // Scan the loop past the iterator binding: exactly one append group with f as its
    // source, no use of f after it, no group under a nested loop.
    let mut scan = MoveScan {
        loop_var,
        copies: 0,
        group: None,
        use_after: false,
        bad: false,
    };
    for op in lp.operators.iter().skip(1) {
        scan_move(op, data, &mut scan, 0);
    }
    if std::env::var("LOFT_TRACE_MOVE").is_ok() {
        eprintln!(
            "[move] scan: copies={} bad={} use_after={} group={:?}",
            scan.copies, scan.bad, scan.use_after, scan.group
        );
    }
    let (dest, elem_size) = scan.group?;
    if scan.bad || scan.use_after || scan.copies != 1 {
        return None;
    }
    if dest >= vars.count() || !owned_local(vars, dest) {
        if std::env::var("LOFT_TRACE_MOVE").is_ok() {
            eprintln!("[move] dest {} not owned local", vars.name(dest));
        }
        return None;
    }
    // Never rebound: the placed buffer record must die with the store that hosts it, and a
    // rebind frees that store mid-flight.
    if set_counts.get(&dest).copied().unwrap_or(0) != 0 {
        if std::env::var("LOFT_TRACE_MOVE").is_ok() {
            eprintln!(
                "[move] dest {} rebound {}x",
                vars.name(dest),
                set_counts[&dest]
            );
        }
        return None;
    }
    let trace2 = std::env::var("LOFT_TRACE_MOVE").is_ok();
    let Type::Vector(elem, _) = vars.tp(dest).peel_link() else {
        if trace2 {
            eprintln!(
                "[move] dest {} not a vector: {:?}",
                vars.name(dest),
                vars.tp(dest)
            );
        }
        return None;
    };
    // A plain struct element only: the moved bytes are fields, and zeroing the source is
    // "owns nothing now" in every field kind; an enum's discriminant is not such a field.
    if plain_record_type(data, elem).is_none() {
        if trace2 {
            eprintln!("[move] elem not plain record: {elem:?}");
        }
        return None;
    }
    let Type::Reference(ed, _) = elem.peel_link() else {
        if trace2 {
            eprintln!("[move] elem not Reference");
        }
        return None;
    };
    let elem_name = elem.name(data);
    let buf_tp = data.name_type(&format!("main_vector<{elem_name}>"), 0);
    if buf_tp == u16::MAX {
        if trace2 {
            eprintln!("[move] no main_vector<{elem_name}> type");
        }
        return None;
    }
    // The lookup above is BY NAME, and a user struct named like a stdlib type
    // variable shares its wrapper's name with the GENERIC template (a struct `T`
    // finds `main_vector<T>` whose `vector` field still carries `__typevar_T`) —
    // a record-level walk through that def misreads every element as the
    // typevar's layout (the c11 refusal, 2026-09-11).  The def is a valid buffer
    // type only when its `vector` attribute names OUR element; anything else
    // declines the pairing and the deep copy stays, which is always correct.
    let wrapper = data.def_nr(&format!("main_vector<{elem_name}>"));
    let wrapper_elem_is_ours = wrapper != u32::MAX && {
        let a = data.attr(wrapper, "vector");
        a != usize::MAX
            && matches!(
                data.attr_type(wrapper, a).peel_link(),
                Type::Vector(e, _)
                    if matches!(e.peel_link(), Type::Reference(d2, _) if d2 == ed)
            )
    };
    if !wrapper_elem_is_ours {
        if trace2 {
            eprintln!(
                "[move] main_vector<{elem_name}> is not OUR element's wrapper (a name collision with a generic template) — declined"
            );
        }
        return None;
    }
    // The host store: the destination's own `__vdb` witness, named by its type's deps.
    let deps = vars.tp(dest).depend();
    let mut vdbs = deps
        .iter()
        .filter(|dv| vars.name(**dv).starts_with("__vdb"));
    let Some(host_vdb) = vdbs.next().copied() else {
        if trace2 {
            eprintln!("[move] dest {} has no __vdb dep: {deps:?}", vars.name(dest));
        }
        return None;
    };
    if vdbs.next().is_some() {
        if trace2 {
            eprintln!("[move] dest {} has 2+ __vdb deps", vars.name(dest));
        }
        return None;
    }
    Some(MoveAppend {
        buf,
        host_vdb,
        buf_tp,
        loop_var,
        dest,
        elem_size,
    })
}

struct MoveScan {
    loop_var: u16,
    copies: u32,
    /// `(dest, elem_size)` of the one valid append group.
    group: Option<(u16, u32)>,
    use_after: bool,
    bad: bool,
}

/// Preorder walk for [`pair_for_block`]: finds the append groups, counts the loop
/// variable's other appearances, and tracks whether anything reads it after the move.
fn scan_move(v: &Value, data: &Data, scan: &mut MoveScan, loop_depth: u32) {
    match v.unspan() {
        Value::Block(bl) => {
            let mut i = 0;
            while i < bl.operators.len() {
                // The group: [OpPreAllocVector(dest, 1, SIZE)] · Set(elm, OpNewRecord(dest,
                // tp, MAX)) · OpCopyRecord(f, elm, _) · OpFinishRecord(dest, elm, ..).
                if let Some((dest, elm, size)) = group_head(&bl.operators, i, data)
                    && let Some(Value::Call(cd, cargs)) = bl.operators.get(i + 2).map(Value::unspan)
                    && data.def(*cd).name() == "OpCopyRecord"
                    && matches!(cargs.first().map(Value::unspan), Some(Value::Var(f)) if *f == scan.loop_var)
                    && matches!(cargs.get(1).map(Value::unspan), Some(Value::Var(e)) if *e == elm)
                    && let Some(Value::Call(fd, fargs)) = bl.operators.get(i + 3).map(Value::unspan)
                    && data.def(*fd).name() == "OpFinishRecord"
                    && matches!(fargs.first().map(Value::unspan), Some(Value::Var(dv)) if *dv == dest)
                {
                    scan.copies += 1;
                    if scan.copies > 1 || loop_depth > 0 {
                        scan.bad = true;
                    } else {
                        scan.group = Some((dest, size));
                    }
                    // The group's own nodes are accounted; a SECOND group or any later
                    // use still flips the flags above.
                    i += 4;
                    continue;
                }
                scan_move(&bl.operators[i], data, scan, loop_depth);
                i += 1;
            }
        }
        Value::Loop(lp) => {
            for op in &lp.operators {
                scan_move(op, data, scan, loop_depth + 1);
            }
        }
        Value::Var(f) if *f == scan.loop_var => {
            if scan.group.is_some() {
                scan.use_after = true;
            }
        }
        // An OpCopyRecord with f as source OUTSIDE the exact group shape: not movable.
        Value::Call(d, args) => {
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpCopyRecord"
                && matches!(args.first().map(Value::unspan), Some(Value::Var(f)) if *f == scan.loop_var)
            {
                scan.bad = true;
            }
            for a in args {
                scan_move(a, data, scan, loop_depth);
            }
        }
        Value::Set(_, to) => scan_move(to, data, scan, loop_depth),
        Value::If(c, t, e) => {
            scan_move(c, data, scan, loop_depth);
            scan_move(t, data, scan, loop_depth);
            scan_move(e, data, scan, loop_depth);
        }
        Value::Return(r) => scan_move(r, data, scan, loop_depth),
        _ => {}
    }
}

/// The `Set(elm, OpNewRecord(dest, _, u16::MAX))` head of an append group at `i`
/// (`i` may point at the optional `OpPreAllocVector` before it) — answers
/// `(dest, elm, elem_size)`.
fn group_head(ops: &[Value], i: usize, data: &Data) -> Option<(u16, u16, u32)> {
    // The stride comes from the group's own OpPreAllocVector — the parser's exact number.
    let (set_at, size) = if let Some(Value::Call(pd, pargs)) = ops.get(i).map(Value::unspan)
        && data.def(*pd).name() == "OpPreAllocVector"
        && let Some(Value::Int(sz)) = pargs.get(2).map(Value::unspan)
    {
        (i + 1, u32::try_from(*sz).ok()?)
    } else {
        return None;
    };
    // Only the shifted shape below is matched, so callers pass the PreAlloc index.
    let Some(Value::Set(elm, rhs)) = ops.get(set_at).map(Value::unspan) else {
        return None;
    };
    let Value::Call(nd, nargs) = rhs.unspan() else {
        return None;
    };
    if data.def(*nd).name() != "OpNewRecord" {
        return None;
    }
    let Some(Value::Var(dest)) = nargs.first().map(Value::unspan) else {
        return None;
    };
    Some((*dest, *elm, size))
}

/// Debug label for the move trace.
fn kind_of(v: &Value) -> &'static str {
    match v.unspan() {
        Value::Set(_, _) => "Set",
        Value::Call(_, _) => "Call",
        Value::Block(_) => "Block",
        Value::Loop(_) => "Loop",
        Value::Var(_) => "Var",
        Value::Null => "Null",
        _ => "other",
    }
}

/// @PLN157 § V-u (`@FR-R-RetAdopt`) — a vector-returning function whose result local ADOPTS
/// the hidden return buffer: the local builds in the caller's buffer from its declaration,
/// so every delivery copy at the exits (`OpReplaceVector`, the `OpClearVector` +
/// `OpAppendVector` pair) has nothing left to move and is emitted as nothing, and the
/// local's own witness store is never allocated.
#[derive(Clone, Copy, Debug)]
pub struct RetAdopt {
    /// The result local every delivery site copies into the buffer.
    pub v: u16,
    /// Its `__vdb` witness variable — the store the adoption leaves unallocated.
    pub vdb: u16,
    /// The hidden return-buffer variable the local aliases.
    pub buf: u16,
    /// The witness's `OpDatabase` type, for the buffer's null arm (a caller that offered
    /// no buffer gets one allocated exactly as the witness would have been).
    pub db_tp: i32,
}

/// The § V-u adoption of `def_nr`, or `None`.  Every gate is an under-approximation on
/// purpose — a declined function keeps the delivery copies, which are always correct:
///
/// - the function VALUE-returns a plain vector through a hidden buffer (a borrow return
///   publishes a dep and delivers nothing);
/// - every delivery into the buffer sources the SAME local `v`, and the buffer serves
///   nothing else — a site delivering another value, or a call handed the buffer (the
///   `one_buffer_chain` shape), declines;
/// - `v` is bound exactly once, from its own witness (`Set(v, OpGetField(__vdb, 0, _))`),
///   never reassigned (a rebind's `OpDatabase` reuse would clear the CALLER's store) and
///   never captured;
/// - the witness serves only its init and its frees;
/// - no `Parallel` or `Yield` in the body (a resumable frame's buffer discipline is its
///   own question).
#[must_use]
pub fn ret_adopt(data: &Data, def_nr: u32) -> Option<RetAdopt> {
    let def = data.def(def_nr);
    if !def.is_loft_defined() {
        return None;
    }
    // The RETURN-TYPE gate comes FIRST, and the order is load-bearing.
    // `returns_borrowed_view` is a heap-return ownership read: it walks the return's dep list
    // as ATTRIBUTE indices, and a `Type::Function` return does not carry those — a closure's
    // deps are callee-frame notes tagged `0x8000`, which are not attr indices at all.
    // `data.rs` states that invariant as an assert ("closure-internal note reached a heap-return
    // ownership read") and it fired here on every closure factory under `-C debug-assertions=on`,
    // because this gate asked the ownership question before knowing the return was a vector.
    if !matches!(def.returned().peel_link(), Type::Vector(_, _)) {
        return None;
    }
    // A dep naming only HIDDEN attrs is the one-buffer return marker, not a borrow —
    // `returns_borrowed_view` reads exactly that distinction (a visible attr borrows).
    if def.returns_borrowed_view() {
        return None;
    }
    let attr = def.hidden_return_buffer_attr()?;
    let vars = def.variables();
    let buf = vars.var(&def.attributes()[attr].name);
    if buf == u16::MAX {
        return None;
    }
    let body = def.code();
    // One pass collects every fact the gates need.
    let mut delivery_src: Option<u16> = None;
    let mut bad = false;
    let mut init: Option<(u16, u16)> = None; // (v, vdb)
    let mut v_sets: u32 = 0;
    // The Clear+Append delivery pair is only skippable as its `one_buffer_vec_copy`
    // BLOCK; an append reaching the buffer outside one has no emission that elides it,
    // so it declines the adoption.
    let mut appends_total: u32 = 0;
    let mut appends_in_copy_block: u32 = 0;
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && bl.name == "one_buffer_vec_copy"
        {
            for op in &bl.operators {
                if let Value::Call(d, args) = op.unspan()
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpAppendVector"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if *b == buf)
                {
                    appends_in_copy_block += 1;
                }
            }
        }
        false
    });
    body.any_node(&mut |n| {
        match n {
            Value::Parallel(_) | Value::Yield(_) => bad = true,
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                match name {
                    "OpReplaceVector" | "OpAppendVector"
                        if matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if *b == buf) =>
                    {
                        if name == "OpAppendVector" {
                            appends_total += 1;
                        }
                        match args.get(1).map(Value::unspan) {
                            Some(Value::Var(s)) => match delivery_src {
                                None => delivery_src = Some(*s),
                                Some(prev) if prev == *s => {}
                                Some(_) => bad = true,
                            },
                            _ => bad = true,
                        }
                    }
                    "OpClearVector" => {}
                    _ => {
                        // Any other call handed the buffer is a use this analysis
                        // does not own (the chain shape delivers THROUGH a callee).
                        if args
                            .iter()
                            .any(|a| matches!(a.unspan(), Value::Var(b) if *b == buf))
                        {
                            bad = true;
                        }
                    }
                }
            }
            Value::Set(sv, to) => {
                if let Value::Call(d, gargs) = to.unspan()
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpGetField"
                    && let Some(Value::Var(w)) = gargs.first().map(Value::unspan)
                    && (*w as usize) < vars.count() as usize
                    && vars.name(*w).starts_with("__vdb")
                    && init.is_none()
                {
                    init = Some((*sv, *w));
                }
            }
            _ => {}
        }
        false
    });
    if std::env::var("LOFT_TRACE_ADOPT").is_ok() {
        eprintln!(
            "[adopt] {}: bad={bad} src={delivery_src:?} init={init:?} appends={appends_total}/{appends_in_copy_block}",
            def.name()
        );
    }
    if bad || appends_total != appends_in_copy_block {
        return None;
    }
    let v = delivery_src?;
    let (iv, vdb) = init?;
    if iv != v || v >= vars.count() || vars.is_argument(v) || vars.is_captured(v) {
        return None;
    }
    // Exactly one Set of v (its init), and the witness's OpDatabase for the null arm.
    let mut db_tp: Option<i32> = None;
    body.any_node(&mut |n| {
        match n {
            Value::Set(sv, to) if *sv == v && !matches!(to.unspan(), Value::Null) => {
                v_sets += 1;
            }
            Value::Set(sv, to) if *sv == vdb && !matches!(to.unspan(), Value::Null) => {
                bad = true;
            }
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpDatabase"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == vdb) =>
            {
                if let Some(Value::Int(tp)) = args.get(1).map(Value::unspan) {
                    if db_tp.is_some() {
                        bad = true; // two OpDatabase on the witness: a rebind
                    }
                    db_tp = Some(*tp);
                }
            }
            // The witness reached by any call BUT its own init family declines.
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                if !matches!(
                    name,
                    "OpDatabase" | "OpGetField" | "OpSetInt4" | "OpFreeRef" | "OpFreeRefTag"
                ) && args
                    .iter()
                    .any(|a| matches!(a.unspan(), Value::Var(w) if *w == vdb))
                {
                    if std::env::var("LOFT_TRACE_ADOPT").is_ok() {
                        eprintln!("[adopt] {}: witness touched by {name}", def.name());
                    }
                    bad = true;
                }
            }
            _ => {}
        }
        false
    });
    if std::env::var("LOFT_TRACE_ADOPT").is_ok() {
        eprintln!(
            "[adopt] {}: second pass bad={bad} v_sets={v_sets} db_tp={db_tp:?}",
            def.name()
        );
    }
    if bad || v_sets != 1 {
        return None;
    }
    Some(RetAdopt {
        v,
        vdb,
        buf,
        db_tp: db_tp?,
    })
}

/// Does `callee` run `OpDatabase` on its own hidden return buffer (@PLN157 § V-j)?  That
/// is the witness-promoted delivery shape: the reuse arm CLEARS the buffer's whole store,
/// so a buffer such a callee receives must own its store — it cannot be placed.
fn callee_reinits_buffer(data: &Data, callee: &crate::data::Definition) -> bool {
    let Some(attr) = callee.hidden_return_buffer_attr() else {
        return true; // no buffer attr: not a delivery this analysis understands
    };
    let bv = callee.variables().var(&callee.attributes()[attr].name);
    if bv == u16::MAX {
        return true;
    }
    let mut reinits = false;
    callee.code().any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpDatabase"
            && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == bv)
        {
            reinits = true;
        }
        false
    });
    reinits
}

/// The scalar getters a § V-x invariant part may read a value-const parameter through —
/// an ALLOW-list like the rest of the family: a getter missing here declines the
/// candidate, never miscompiles it.
const LIT_HOIST_GETTERS: [&str; 7] = [
    "OpGetInt",
    "OpGetFloat",
    "OpGetSingle",
    "OpGetBoolean",
    "OpGetByte",
    "OpGetShort",
    "OpGetCharacter",
];

/// Does `tp` mention record/enum definition `pd` anywhere in its shape?
fn mentions_def(tp: &Type, pd: u32) -> bool {
    // `.base()` at each node so a `τ?`-wrapped reference is the same mention
    // (`@FR-N-Shape` — the walk descends wrappers, and the test must too).
    tp.any_node(
        &mut |t| matches!(t.base(), Type::Reference(d, _) | Type::Enum(d, _, _) if *d == pd),
    )
}

/// A FRESH-STORE local (@PLN157 § V-x): every `Set` of it is the `= null` declaration and
/// an `OpDatabase(v, tp)` creates its own store — so the record it names is minted by
/// THIS activation and can never be a record a parameter links to.
fn fresh_store_local(v: u16, data: &Data, vars: &crate::variables::Function, body: &Value) -> bool {
    if vars.is_argument(v) {
        return false;
    }
    let mut all_null = true;
    let mut created = false;
    body.any_node(&mut |n| {
        match n {
            Value::Set(w, x) if *w == v => {
                if !matches!(x.unspan(), Value::Null) {
                    all_null = false;
                }
            }
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpDatabase"
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == v) =>
            {
                created = true;
            }
            _ => {}
        }
        false
    });
    all_null && created
}

/// A § V-x invariant SCALAR part: a literal, a pure/primitive op over invariant parts, a
/// by-value scalar parameter the function never reassigns, or a scalar-getter read of a
/// value-const record parameter (collected into `params` for the caller's alias gate).
fn lit_part_invariant(
    val: &Value,
    data: &Data,
    vars: &crate::variables::Function,
    set_counts: &HashMap<u16, u32>,
    params: &mut HashSet<u16>,
) -> bool {
    match val.unspan() {
        Value::Int(_)
        | Value::Long(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Boolean(_)
        | Value::Enum(_, _)
        | Value::Null => true,
        Value::Var(p) => {
            // A by-value scalar parameter never reassigned: copies cannot alias, so
            // only a rebind could move it, and there is none.
            vars.is_argument(*p)
                && matches!(
                    vars.tp(*p).base(),
                    Type::Integer(_) | Type::Float | Type::Single | Type::Boolean | Type::Character
                )
                && !set_counts.contains_key(p)
        }
        Value::If(c, a, b) => {
            lit_part_invariant(c, data, vars, set_counts, params)
                && lit_part_invariant(a, data, vars, set_counts, params)
                && lit_part_invariant(b, data, vars, set_counts, params)
        }
        Value::Block(bl) => bl
            .operators
            .iter()
            .all(|op| lit_part_invariant(op, data, vars, set_counts, params)),
        Value::Call(d, args) => {
            if (*d as usize) >= data.definitions.len() {
                return false;
            }
            let def = data.def(*d);
            let name = def.name();
            // A scalar-getter read of a VALUE-CONST record parameter.
            if LIT_HOIST_GETTERS.contains(&name)
                && let Some(Value::Var(p)) = args.first().map(Value::unspan)
                && vars.is_argument(*p)
                && vars.is_value_const(*p)
                && matches!(vars.tp(*p).peel_link(), Type::Reference(_, _))
                && args[1..]
                    .iter()
                    .all(|a| matches!(a.unspan(), Value::Int(_)))
            {
                params.insert(*p);
                return true;
            }
            // A pure or primitive scalar op over invariant parts.
            let primitive = matches!(def.code(), Value::Null) && !def.rust().is_empty();
            (primitive || def.purity == crate::data::Purity::Pure)
                && args
                    .iter()
                    .all(|a| lit_part_invariant(a, data, vars, set_counts, params))
        }
        _ => false,
    }
}

/// A § V-x invariant vector-literal INITIALIZER: the parser's build group — `OpDatabase`
/// on the declaration's own `__vdb` witness, the `_vec` temp bound from it, the length
/// reset, the reservation, and pushes of invariant scalar parts — possibly under an `if`
/// whose condition is itself invariant.  Anything else declines.
fn lit_init_invariant(
    val: &Value,
    data: &Data,
    vars: &crate::variables::Function,
    set_counts: &HashMap<u16, u32>,
    params: &mut HashSet<u16>,
) -> bool {
    match val.unspan() {
        Value::If(c, a, b) => {
            lit_part_invariant(c, data, vars, set_counts, params)
                && lit_init_invariant(a, data, vars, set_counts, params)
                && lit_init_invariant(b, data, vars, set_counts, params)
        }
        Value::Block(bl) => bl
            .operators
            .iter()
            .all(|op| lit_init_invariant(op, data, vars, set_counts, params)),
        Value::Set(t, x) => {
            // `_vec_N = OpGetField(__vdb_M, 0, tp)` — the temp bound to the fresh store.
            vars.name(*t).starts_with("_vec")
                && matches!(x.unspan(), Value::Call(d, cargs)
                    if (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpGetField"
                        && matches!(cargs.first().map(Value::unspan),
                            Some(Value::Var(w)) if vars.name(*w).starts_with("__vdb")))
        }
        Value::Var(t) => vars.name(*t).starts_with("_vec"),
        Value::Call(d, args) => {
            if (*d as usize) >= data.definitions.len() {
                return false;
            }
            let name = data.def(*d).name();
            match name {
                "OpDatabase" | "OpSetInt4" => matches!(
                    args.first().map(Value::unspan),
                    Some(Value::Var(w)) if vars.name(*w).starts_with("__vdb")
                ),
                "OpPreAllocVector" => matches!(
                    args.first().map(Value::unspan),
                    Some(Value::Var(w)) if vars.name(*w).starts_with("_vec")
                ),
                _ if name.starts_with("OpPush") => {
                    matches!(
                        args.first().map(Value::unspan),
                        Some(Value::Var(w)) if vars.name(*w).starts_with("_vec")
                    ) && args[1..]
                        .iter()
                        .all(|a| lit_part_invariant(a, data, vars, set_counts, params))
                }
                _ => false,
            }
        }
        _ => false,
    }
}

/// @PLN157 § V-x — what [`invariant_literals`] admitted, by SHAPE: a `wrapped` local's
/// whole build is the one `Set(v, if …)` statement the emitter guards directly; a `flat`
/// local's build is the parser's statement RUN (`OpDatabase(__vdb) · Set(v, OpGetField) ·
/// OpSetInt4 · OpPreAlloc · pushes`), which the emitter guards from the `OpDatabase` to
/// the first non-member statement ([`flat_lit_member`] is the shared predicate, so the
/// two cannot drift).
#[derive(Default)]
pub struct LitHoist {
    /// Admitted locals whose declaration is ONE `Set` statement.
    pub wrapped: HashSet<u16>,
    /// Admitted FLAT groups, keyed by the declaration's `__vdb` witness → the local.
    pub flat: HashMap<u16, u16>,
}

impl LitHoist {
    #[must_use]
    pub fn contains(&self, v: u16) -> bool {
        self.wrapped.contains(&v) || self.flat.values().any(|w| *w == v)
    }
}

/// Is `stmt` a member of the FLAT build group of local `v` with witness `vdb`
/// (@PLN157 § V-x)?  Shared by the analysis (group collection) and the emitter (guard
/// close), so an op admitted by one is admitted by the other.  A `Value::Line` marker is
/// a member — the parser may interleave source positions with the group.
#[must_use]
pub fn flat_lit_member(
    stmt: &Value,
    v: u16,
    vdb: u16,
    data: &Data,
    _vars: &crate::variables::Function,
) -> bool {
    match stmt.unspan() {
        Value::Line(_) => true,
        Value::Set(w, x) if *w == v => matches!(x.unspan(), Value::Call(d, cargs)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpGetField"
                && matches!(cargs.first().map(Value::unspan), Some(Value::Var(u)) if *u == vdb)),
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            let first_is =
                |t: u16| matches!(args.first().map(Value::unspan), Some(Value::Var(u)) if *u == t);
            match name {
                "OpSetInt4" => first_is(vdb),
                "OpPreAllocVector" => first_is(v),
                _ if name.starts_with("OpPush") => first_is(v),
                _ => false,
            }
        }
        _ => false,
    }
    // (The part-invariance of each push was proven at admission; membership here is
    // positional, so a group the analysis declined never reaches the emitter.)
}

/// @PLN157 § V-x (`@FR-R-LitHoist`) — the loop-body vector LITERALS that build ONCE per
/// activation: a `Set(v, init)` under a `For` where `v` is a plain no-heap-scalar vector
/// with one `Set` in the whole function, the initializer's parts are invariant
/// ([`lit_part_invariant`]), the local's other uses are reads only, and — the alias gate
/// `(Const-Value)` makes necessary, since `const` is per-NAME — every variable in the
/// function whose type can reach a record type the initializer reads is either a
/// fresh-store local (its record is this activation's, never the caller's) or a
/// value-const parameter itself used ONLY as a scalar-getter base.  The emitter
/// pre-declares each admitted local at function top and wraps its declaration statement
/// in an unbound-guard, so the build runs once and every later iteration (and re-entry)
/// reuses the store.  Two admitted locals sharing a sanitized name would share the one
/// function-top binding, so a collision declines both (the c11 cell).
pub fn invariant_literals(data: &Data, def_nr: u32) -> LitHoist {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    // A coroutine's locals persist as state-machine fields, and a `par` body's captures
    // run beside the loop: both break the plain-frame assumptions of the fn-top guard.
    if body.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_))) {
        return LitHoist::default();
    }
    let mut set_counts: HashMap<u16, u32> = HashMap::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, _) = n {
            *set_counts.entry(*v).or_default() += 1;
        }
        false
    });
    // Candidates, with the value-const params each one reads.  `flat_of` remembers which
    // admitted local is a statement-run build and through which witness.
    let mut cands: HashMap<u16, HashSet<u16>> = HashMap::new();
    let mut flat_of: HashMap<u16, u16> = HashMap::new();
    // A FLAT group's own statements mention the local (`OpPreAllocVector(v, …)`, the
    // pushes), and the use-reconciliation below must account for exactly those.
    let mut group_mentions: HashMap<u16, u32> = HashMap::new();
    let elem_ok = |v: u16| {
        matches!(vars.tp(v).peel_link(), Type::Vector(elem, _)
            if matches!(elem.base(), Type::Integer(_) | Type::Float
                | Type::Single | Type::Boolean | Type::Character))
    };
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && bl.name == "For block"
        {
            bl.operators.iter().for_each(|op| {
                op.any_node(&mut |m| {
                    // The WRAPPED shape: one Set whose value packages the whole build.
                    if let Value::Set(v, init) = m
                        && !cands.contains_key(v)
                        && set_counts.get(v) == Some(&1)
                        && elem_ok(*v)
                    {
                        let mut params = HashSet::new();
                        if lit_init_invariant(init, data, vars, &set_counts, &mut params) {
                            cands.insert(*v, params);
                        }
                    }
                    // The FLAT shape: `OpDatabase(__vdb) · Set(v, OpGetField(__vdb)) ·
                    // members…` as a statement run of some inner block.
                    if let Value::Block(inner) = m {
                        let ops = &inner.operators;
                        let trace = std::env::var("LOFT_TRACE_LITHOIST").is_ok();
                        // The parser interleaves `Line` markers with the group.
                        let next_code =
                            |from: usize| (from..ops.len())
                                .find(|j| !matches!(ops[*j].unspan(), Value::Line(_)));
                        for i in 0..ops.len() {
                            let Value::Call(d, dargs) = ops[i].unspan() else {
                                continue;
                            };
                            if (*d as usize) >= data.definitions.len()
                                || data.def(*d).name() != "OpDatabase"
                            {
                                continue;
                            }
                            let Some(Value::Var(vdb)) = dargs.first().map(Value::unspan) else {
                                continue;
                            };
                            if !vars.name(*vdb).starts_with("__vdb") {
                                continue;
                            }
                            let Some(si) = next_code(i + 1) else { continue };
                            let Value::Set(v, _) = ops[si].unspan() else {
                                if trace {
                                    eprintln!(
                                        "[lithoist] {}: after OpDatabase({}) not a Set",
                                        def.name(),
                                        vars.name(*vdb)
                                    );
                                }
                                continue;
                            };
                            if cands.contains_key(v)
                                || set_counts.get(v) != Some(&1)
                                || !elem_ok(*v)
                                || !flat_lit_member(&ops[si], *v, *vdb, data, vars)
                            {
                                if trace {
                                    eprintln!(
                                        "[lithoist] {}: {} declined (sets={:?}, elem_ok={}, member={})",
                                        def.name(),
                                        vars.name(*v),
                                        set_counts.get(v),
                                        elem_ok(*v),
                                        flat_lit_member(&ops[si], *v, *vdb, data, vars)
                                    );
                                }
                                continue;
                            }
                            // Collect the run and prove each push part invariant,
                            // tallying the group's own mentions of the local.
                            let mut params = HashSet::new();
                            let mut sound = true;
                            let mut mentions = 0u32;
                            for stmt in ops.iter().skip(si + 1) {
                                if !flat_lit_member(stmt, *v, *vdb, data, vars) {
                                    break;
                                }
                                stmt.any_node(&mut |n| {
                                    if matches!(n, Value::Var(u) if u == v) {
                                        mentions += 1;
                                    }
                                    false
                                });
                                if let Value::Call(pd, pargs) = stmt.unspan()
                                    && data.def(*pd).name().starts_with("OpPush")
                                    && !pargs[1..].iter().all(|a| {
                                        lit_part_invariant(a, data, vars, &set_counts, &mut params)
                                    })
                                {
                                    sound = false;
                                    if trace {
                                        eprintln!(
                                            "[lithoist] {}: {} push part not invariant",
                                            def.name(),
                                            vars.name(*v)
                                        );
                                    }
                                    break;
                                }
                            }
                            if sound {
                                cands.insert(*v, params);
                                flat_of.insert(*v, *vdb);
                                group_mentions.insert(*v, mentions);
                            }
                        }
                    }
                    false
                });
            });
        }
        false
    });
    if cands.is_empty() {
        return LitHoist::default();
    }
    // The local's OTHER uses must all be reads: For-head binds, indexed/length reads,
    // whole-value binds to another local, and its scope-exit free.  Reconciled counts,
    // because `any_node` visits the `Var` inside each allowed context too.
    let mut var_mentions: HashMap<u16, u32> = HashMap::new();
    let mut allowed_uses: HashMap<u16, u32> = HashMap::new();
    let mut getter_bases: HashMap<u16, u32> = HashMap::new();
    body.any_node(&mut |n| {
        match n {
            Value::Var(w) => {
                *var_mentions.entry(*w).or_default() += 1;
            }
            Value::Set(_, x) => {
                if let Value::Var(w) = x.unspan()
                    && cands.contains_key(w)
                {
                    *allowed_uses.entry(*w).or_default() += 1;
                }
            }
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                if let Some(Value::Var(w)) = args.first().map(Value::unspan) {
                    // OpGetVector* is NOT here on purpose: it is context-blind — the
                    // same node is the base of `v[0] = …`'s WRITE — so an indexed use
                    // declines the candidate (the c5 cell), costing the optimisation
                    // and never correctness.  The For-head bind covers iteration.
                    if matches!(name, "OpLengthVector" | "OpFreeRef") && cands.contains_key(w) {
                        *allowed_uses.entry(*w).or_default() += 1;
                    }
                    if LIT_HOIST_GETTERS.contains(&name) {
                        *getter_bases.entry(*w).or_default() += 1;
                    }
                }
            }
            _ => {}
        }
        false
    });
    cands.retain(|v, _| {
        var_mentions.get(v).copied().unwrap_or(0)
            == allowed_uses.get(v).copied().unwrap_or(0)
                + group_mentions.get(v).copied().unwrap_or(0)
    });
    // The alias gate: for each record definition an admitted init reads, every variable
    // whose type can reach it must be a fresh-store local or a value-const parameter
    // used only as a scalar-getter base.
    let pds: HashSet<u32> = cands
        .values()
        .flatten()
        .filter_map(|p| match vars.tp(*p).peel_link() {
            Type::Reference(pd, _) => Some(*pd),
            _ => None,
        })
        .collect();
    let mut fresh_memo: HashMap<u16, bool> = HashMap::new();
    let alias_clean = pds.iter().all(|pd| {
        (0..vars.count()).all(|w| {
            if !mentions_def(vars.tp(w), *pd) || var_mentions.get(&w).copied().unwrap_or(0) == 0 {
                return true;
            }
            if vars.is_argument(w) && vars.is_value_const(w) {
                // Its only permitted role is a scalar-getter base — a whole-value
                // escape (a call argument, a bind, an `&`) could reach a writer.
                return getter_bases.get(&w).copied().unwrap_or(0)
                    == var_mentions.get(&w).copied().unwrap_or(0);
            }
            *fresh_memo
                .entry(w)
                .or_insert_with(|| fresh_store_local(w, data, vars, body))
        })
    });
    if !alias_clean {
        // The gate is per-FUNCTION on purpose: one unprovable alias route makes every
        // param-reading candidate unsound, and a literal-only candidate reads no pd.
        cands.retain(|_, params| params.is_empty());
    }
    // Two admitted locals sharing a sanitized name would share the one fn-top binding.
    let mut by_name: HashMap<String, u32> = HashMap::new();
    for v in cands.keys() {
        *by_name.entry(super::sanitize(vars.name(*v))).or_default() += 1;
    }
    let mut out = LitHoist::default();
    for v in cands.into_keys() {
        if by_name[&super::sanitize(vars.name(v))] != 1 {
            continue;
        }
        if let Some(vdb) = flat_of.get(&v) {
            out.flat.insert(*vdb, v);
        } else {
            out.wrapped.insert(v);
        }
    }
    out
}

/// @PLN157 § V-y — what [`complete_writes`] admitted: `db_vars` are locals whose EVERY
/// `OpDatabase` site heads a covering literal group (the emitter calls `OpDatabaseNP`);
/// `mint_tps` are element types whose every `OpNewRecord` site in this function does
/// (the emitter calls `OpNewRecordNP`).
#[derive(Default)]
pub struct CompleteWrites {
    pub db_vars: HashSet<u16>,
    pub mint_tps: HashSet<u16>,
}

/// Do the statements FOLLOWING index `at` in `ops` cover every field position of `tp`
/// with contiguous `OpSet*`s on `target` (@PLN157 § V-y)?  Coverage is by BYTE OFFSET
/// against the schema's field list, the variant tag included (its field sits at
/// position 0 and the literal's `OpSetEnum` writes it).  A field the run never reaches
/// — a nested struct arriving by `OpCopyRecord`, a vector field bound by an append —
/// leaves the group incomplete, and the prefill stays: the check can only DECLINE the
/// elision, never miscompile it.
fn group_covers_type(
    ops: &[Value],
    at: usize,
    target: u16,
    tp: u16,
    data: &Data,
    stores: &Stores,
) -> bool {
    if (tp as usize) >= stores.types.len() {
        return false;
    }
    let (crate::database::Parts::Struct(fields) | crate::database::Parts::EnumValue(_, fields)) =
        &stores.types[tp as usize].parts
    else {
        return false;
    };
    if fields.is_empty() {
        return false;
    }
    let mut written: HashSet<u16> = HashSet::new();
    for stmt in ops.iter().skip(at) {
        match stmt.unspan() {
            Value::Line(_) => {}
            // The declaration's own bind (`v = OpGetField(target, 0, tp)`) interleaves
            // a VECTOR store's group between the OpDatabase and the length reset — a
            // member that covers nothing.  (The collection-field prefill is ONE u32
            // zero at the field position, exactly the group's `OpSetInt4`'s width, so
            // admitting the shape is width-equal, not width-blind.)
            Value::Set(_, x)
                if matches!(x.unspan(), Value::Call(d, cargs)
                    if (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpGetField"
                        && matches!(cargs.first().map(Value::unspan),
                            Some(Value::Var(u)) if *u == target)) => {}
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                if name.starts_with("OpSet")
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == target)
                    && let Some(Value::Int(off)) = args.get(1).map(Value::unspan)
                    && let Ok(off) = u16::try_from(*off)
                {
                    written.insert(off);
                } else {
                    break;
                }
            }
            _ => break,
        }
    }
    fields.iter().all(|f| written.contains(&f.position))
}

/// @PLN157 § V-y (`@FR-R-CompleteWrite`) — the literal groups whose write set is
/// COMPLETE, so the default prefill writes nothing that survives: the parser's lowering
/// writes every field explicitly (a named value, the declared default, the interned
/// empty text, the null sentinel, `false`, the variant tag), and the emitter's proof is
/// coverage of every schema field position by the group's contiguous `OpSet*`s.
/// `db_vars` is keyed by the LOCAL (every `OpDatabase(v, …)` site must head a covering
/// group — a var built completely in one branch and partially in another declines);
/// `mint_tps` by the ELEMENT TYPE (every `Set(_, OpNewRecord(…, tp, …))` group in the
/// function must cover it).  The interpreter keeps the prefill and is the oracle.
pub fn complete_writes(data: &Data, stores: &Stores, def_nr: u32) -> CompleteWrites {
    let def = data.def(def_nr);
    let body = def.code();
    let mut out = CompleteWrites::default();
    let mut db_declined: HashSet<u16> = HashSet::new();
    let mut mint_declined: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n {
            let ops = &bl.operators;
            for (i, stmt) in ops.iter().enumerate() {
                match stmt.unspan() {
                    // `OpDatabase(v, tp)` heading a struct-literal group.
                    Value::Call(d, args)
                        if (*d as usize) < data.definitions.len()
                            && data.def(*d).name() == "OpDatabase" =>
                    {
                        let (Some(Value::Var(v)), Some(Value::Int(tp))) = (
                            args.first().map(Value::unspan),
                            args.get(1).map(Value::unspan),
                        ) else {
                            continue;
                        };
                        let Ok(tp) = u16::try_from(*tp) else {
                            db_declined.insert(*v);
                            continue;
                        };
                        if group_covers_type(ops, i + 1, *v, tp, data, stores) {
                            out.db_vars.insert(*v);
                        } else {
                            db_declined.insert(*v);
                        }
                    }
                    // `Set(e, OpNewRecord(P, tp, fld))` heading a mint group: the sets
                    // that follow write through `e`.
                    Value::Set(e, inner) => {
                        let Value::Call(d, cargs) = inner.unspan() else {
                            continue;
                        };
                        if (*d as usize) >= data.definitions.len()
                            || data.def(*d).name() != "OpNewRecord"
                        {
                            continue;
                        }
                        let Some(Value::Int(ptp)) = cargs.get(1).map(Value::unspan) else {
                            continue;
                        };
                        let Ok(ptp) = u16::try_from(*ptp) else {
                            continue;
                        };
                        // The element's CONTENT type is what the prefill would walk.
                        let fld = match cargs.get(2).map(Value::unspan) {
                            Some(Value::Int(f)) => u16::try_from(*f).unwrap_or(u16::MAX),
                            _ => u16::MAX,
                        };
                        let etp = if fld == u16::MAX {
                            stores.content(ptp)
                        } else {
                            stores.content(stores.field_type(ptp, fld))
                        };
                        if etp != u16::MAX && group_covers_type(ops, i + 1, *e, etp, data, stores) {
                            out.mint_tps.insert(ptp);
                        } else {
                            mint_declined.insert(ptp);
                        }
                    }
                    _ => {}
                }
            }
        }
        false
    });
    out.db_vars.retain(|v| !db_declined.contains(v));
    out.mint_tps.retain(|t| !mint_declined.contains(t));
    out
}

/// @PLN157 § V-z — one paired temp of an ELEMENT-FIRST append: the local `tmp` (declared
/// through witness `vdb`) is consumed exactly once as the record-literal field at byte
/// `field_off` of the element `elm` appended to `out`.
pub struct ElemBind {
    pub tmp: u16,
    /// The declaration's key: the `__vdb` witness of a literal-built temp, or — `from_call` —
    /// the hidden return buffer of a temp a call fills (@PLN164 E-2).
    pub vdb: u16,
    pub field_off: i32,
    /// This temp's declaration carries the element MINT (the first temp in decl order).
    pub first: bool,
    /// The temp is `tmp = g(…, vdb)`: the call is handed the element's field as its buffer,
    /// and `vdb` is never minted.
    pub from_call: bool,
}

/// @PLN157 § V-z — an admitted element-first APPEND: the element minted at the first
/// temp's declaration site, every paired temp bound to its field slot, the append site
/// keeping its scalar sets and finish while the reservation, the mint, the paired
/// handle-zeros and the paired `OpAppendVector` copies are suppressed.
pub struct ElemFirst {
    pub out: u16,
    pub out_tp: i32,
    /// The collection's field NUMBER in `out`'s record (@PLN164 E-2), or `65535` when `out`
    /// is the vector itself — the mint's own third argument.
    pub out_fld: i32,
    pub elm: u16,
    /// @PLN164 E-2b — the element temps of the other arms of an `if` whose arms each append:
    /// their mint becomes an alias of `elm`, the element minted at the declaration.
    pub aliases: Vec<u16>,
    pub prealloc_size: i32,
    pub binds: Vec<ElemBind>,
}

/// @PLN157 § V-z (`@FR-R-ElemFirst`) — the element-first pairings of one function.
/// Keyed for the emitter: `by_vdb` finds a temp's declaration (the `OpDatabase` site is
/// where the prelude or the slot bind emits), `elms` marks the append-site element vars
/// whose reservation/mint/zeros/copies the statement loop suppresses.
#[derive(Default)]
pub struct ElemFirstMap {
    pub pairs: Vec<ElemFirst>,
    pub by_vdb: HashMap<u16, usize>,
    pub by_elm: HashMap<u16, usize>,
    pub elms: HashSet<u16>,
    /// @PLN164 E-2 — a call-filled temp's hidden buffer → the element and the field offset the
    /// call is handed in its place.
    pub buf_place: HashMap<u16, (u16, i32)>,
}

fn call_named<'v>(stmt: &'v Value, data: &Data, name: &str) -> Option<&'v [Value]> {
    if let Value::Call(d, args) = stmt.unspan()
        && (*d as usize) < data.definitions.len()
        && data.def(*d).name() == name
    {
        Some(args)
    } else {
        None
    }
}

fn as_var(v: Option<&Value>) -> Option<u16> {
    match v.map(Value::unspan) {
        Some(Value::Var(w)) => Some(*w),
        _ => None,
    }
}

fn as_int(v: Option<&Value>) -> Option<i32> {
    match v.map(Value::unspan) {
        Some(Value::Int(n)) => Some(*n),
        _ => None,
    }
}

/// Default-ON; `LOFT_NO_ELEMENT_PLACE=1` keeps § V-z to what it admitted before @PLN164 E-2 —
/// a literal-built temp appended into a LOCAL vector — and is the first bisect step for a
/// wrong, empty or leaked vector field in an element appended to a parameter's collection, or
/// built by a call (read once, generation time, `--native` only).
fn element_place_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_ELEMENT_PLACE").is_ok_and(|v| v != "0"))
}

/// `if OpRefIsNull(buf) { buf = null }` — a lazy hidden buffer's mint (`@FR-O-LazyBuffer`),
/// the statement that stands in front of the call it serves.  Answers `buf`.
fn lazy_buffer_guard(stmt: &Value, data: &Data) -> Option<u16> {
    let Value::If(cond, then_v, else_v) = stmt.unspan() else {
        return None;
    };
    if !matches!(else_v.unspan(), Value::Null) {
        return None;
    }
    let buf = as_var(call_named(cond, data, "OpRefIsNull")?.first())?;
    let only_null_set = |ops: &[Value]| {
        matches!(ops, [one] if matches!(one.unspan(), Value::Set(w, x)
            if *w == buf && matches!(x.unspan(), Value::Null)))
    };
    let ok = match then_v.unspan() {
        Value::Insert(ops) => only_null_set(ops),
        Value::Block(bl) => only_null_set(&bl.operators),
        Value::Set(w, x) => *w == buf && matches!(x.unspan(), Value::Null),
        _ => false,
    };
    ok.then_some(buf)
}

/// Does user function `g` FILL the buffer it is handed rather than mint into it?  The same
/// positive test `(R-Place)`'s buffer-is-the-place clause reads (`Parser::buffer_is_the_place`):
/// a loft-defined body with no `OpDatabase` into any argument slot.  A callee returning a vector
/// literal mints into its buffer and would mint over the element's field.
fn fills_its_buffer(data: &Data, g: u32) -> bool {
    if (g as usize) >= data.definitions.len() {
        return false;
    }
    let def = data.def(g);
    if !def.is_loft_defined() || matches!(def.code(), Value::Null) {
        return false;
    }
    let vars = def.variables();
    !def.code().any_node(&mut |n| {
        call_named(n, data, "OpDatabase")
            .or_else(|| call_named(n, data, "OpDatabaseNP"))
            .is_some_and(|a| as_var(a.first()).is_some_and(|w| vars.is_argument(w)))
    })
}

/// Does `v` carry a jump that leaves it — a `return`, or a `break`/`continue` aimed past the
/// loops `v` itself holds?  Between a temp's declaration and its append such a jump would leave
/// the early element minted and unfinished, holding what the temp built.
fn jumps_out(v: &Value) -> bool {
    fn walk(v: &Value, depth: u16) -> bool {
        match v.unspan() {
            Value::Return(_) => true,
            Value::Break(n) | Value::Continue(n) => *n >= depth,
            Value::Loop(bl) => bl.operators.iter().any(|o| walk(o, depth + 1)),
            other => {
                let mut found = false;
                other.for_each_child(&mut |c| found |= walk(c, depth));
                found
            }
        }
    }
    walk(v, 0)
}

/// The variables that may VIEW an element of `out`'s collection, which a statement between an
/// early mint and its append must not read (`@FR-B-Disturb`, loft#1553).
///
/// The early mint GROWS the destination at the declaration, and a growth moves every element
/// once the collection outgrows its allocation.  The scope pass placed its view copies against
/// the growth where the IR has it — at the append — so a view read in between would read an
/// element that already moved: `e = sc.ops[0]; p = mk(n); x = e.ow; sc.ops += [Op { opts: p }]`
/// answered `0` for `100` on the eleventh element.  Two shapes can hold such a view: a local
/// whose deps close over a store `out` lives in, and, where one of those stores is a CALLER's,
/// any heap-typed parameter (and the locals that view one) — a caller may hand an element in
/// beside its container, and nothing in this frame can tell.
///
/// An upper bound on purpose, because a miss reads freed bytes.  One refinement, for a
/// destination FIELD at byte `dest`: a local that depends on `out` alone and whose every
/// binding views another field of it (`sc.brushes[i]?` beside `sc.ops`, `parse_lock`'s shape)
/// names a collection the growth does not move, and is spared — read with the scope pass's own
/// place model ([`crate::scopes::value_view_places`]), which follows a `?` discharge's block
/// and both of its arms.  A record the frame placed in
/// `out`'s store owns its block (its deps are empty) and is not named either, which is what
/// keeps `parse_circle`'s paint, read in the window, admitted.  A parameter that views an
/// unrelated store still declines.
fn destination_views(
    data: &Data,
    body: &Value,
    vars: &crate::variables::Function,
    out: u16,
    dest: Option<u32>,
) -> HashSet<u16> {
    let closure = |start: u16| -> HashSet<u16> {
        let mut seen: HashSet<u16> = HashSet::new();
        let mut stack = vec![start];
        while let Some(v) = stack.pop() {
            for d in vars.tp(v).depend() {
                if d < vars.count() && seen.insert(d) {
                    stack.push(d);
                }
            }
        }
        seen
    };
    let mut roots = closure(out);
    roots.insert(out);
    let from_caller = roots.iter().any(|r| vars.is_argument(*r));
    let foreign: HashSet<u16> = if from_caller {
        (0..vars.count())
            .filter(|w| *w != out && vars.is_argument(*w) && vars.tp(*w).heap_dep().is_some())
            .collect()
    } else {
        HashSet::new()
    };
    let sibling = |w: u16| -> bool {
        let Some(dest) = dest else { return false };
        if vars.is_argument(w) || vars.tp(w).depend() != [out] {
            return false;
        }
        let mut bound = false;
        let mut other = true;
        body.any_node(&mut |n| {
            if let Value::Set(x, rhs) = n
                && *x == w
            {
                match rhs.unspan() {
                    Value::Null => {}
                    rhs => {
                        bound = true;
                        let places = crate::scopes::value_view_places(rhs, data, vars);
                        other &= !places.is_empty()
                            && places.iter().all(|&(r, off)| {
                                r == out && off != crate::use_analysis::ANY_FIELD && off != dest
                            });
                    }
                }
            }
            false
        });
        bound && other
    };
    (0..vars.count())
        .filter(|&w| w != out)
        .filter(|&w| {
            foreign.contains(&w)
                || closure(w)
                    .iter()
                    .any(|d| roots.contains(d) || foreign.contains(d))
        })
        .filter(|&w| !sibling(w))
        .collect()
}

/// @PLN157 § V-z (`@FR-R-ElemFirst`) — pair the temps with their one consuming append.
///
/// Gates, each carried by a cell: the temp's declaration and the append group are top-level
/// statements of the SAME block (an append under an `if` arm declines — the early mint would
/// strand an unfinished element per skipped iteration, c7), and no statement between them
/// names `out` (an early mint changes what `len(out)` answers, c5) or jumps out of the block
/// (the same stranding, by a `return`, `break` or `continue`); the temp's whole-function uses
/// reconcile to its build mentions plus the ONE `OpAppendVector` (a read after the append or a
/// second append declines, c3/c4); `out` is never rebound and its element a plain struct.
///
/// Two declarations are recognised.  A LITERAL-built temp: `OpDatabase(vdb) · Set(tmp,
/// OpGetField(vdb)) · OpSetInt4(vdb, 0, 0)`.  And (@PLN164 E-2, `LOFT_NO_ELEMENT_PLACE`) a
/// temp a CALL fills: `if OpRefIsNull(buf) { buf = null } · Set(tmp, g(…, buf))`, where `g`
/// fills the buffer it is handed ([`fills_its_buffer`]), no other argument names `out`, and
/// `buf` serves that call alone (its other mentions are frees and identity tests) — the call
/// is handed the element's field instead, and `buf` is never minted.
///
/// Two destinations: a local vector (`OpPreAllocVector(out, 1, size) · Set(elm,
/// OpNewRecord(out, tp, 65535))`), and (E-2) a collection FIELD of a record variable
/// (`Set(elm, OpNewRecord(out, tp, fld))`) whose elements are stored inline.
///
/// An admitted function's EXIT copies of a temp are not uses (`views`): the value form drops
/// them — the view leaf names the element's field instead (`@FR-O-ViewField`).
pub fn element_first(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    views: Option<&ViewPlan>,
) -> ElemFirstMap {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    if body.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_))) {
        return ElemFirstMap::default();
    }
    let place_on = element_place_on();
    let trace = std::env::var("LOFT_TRACE_ELEMFIRST").is_ok();
    // The admitted exits: a temp's copy inside one is dropped by the value form.
    let exits: HashSet<usize> = views
        .map(|v| v.leaves.keys().map(|(addr, _)| *addr).collect())
        .unwrap_or_default();
    // Rebind counts, decl-style Sets excluded (the move-append convention).
    let mut set_counts: HashMap<u16, u32> = HashMap::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, to) = n {
            let init = match to.unspan() {
                Value::Null => true,
                Value::Call(d, cargs) => {
                    (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpGetField"
                        && matches!(cargs.first().map(Value::unspan),
                            Some(Value::Var(w)) if *w < vars.count()
                                && vars.name(*w).starts_with("__vdb"))
                }
                _ => false,
            };
            if !init {
                *set_counts.entry(*v).or_default() += 1;
            }
        }
        false
    });
    // Mentions of `w` outside the admitted exits.
    let uses = |w: u16| -> u32 {
        fn walk(n: &Value, w: u16, exits: &HashSet<usize>, total: &mut u32) {
            if let Value::Block(bl) = n
                && exits.contains(&(std::ptr::from_ref(&**bl) as usize))
            {
                return;
            }
            if matches!(n, Value::Var(x) if *x == w) {
                *total += 1;
            }
            n.for_each_child(&mut |c| walk(c, w, exits, total));
        }
        let mut total = 0;
        walk(body, w, &exits, &mut total);
        total
    };
    // A call-filled temp's buffer serves the call alone: every other mention of it is a free
    // or an identity test, which a never-minted buffer answers as nothing.
    let buffer_serves_one_call = |buf: u16| -> bool {
        let mut total = 0u32;
        let mut released = 0u32;
        let mut calls = 0u32;
        body.any_node(&mut |n| {
            match n {
                Value::Var(w) if *w == buf => total += 1,
                Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                    let callee = data.def(*d);
                    let is_release = matches!(
                        callee.name(),
                        "OpFreeRef" | "OpFreeRefIfDistinct" | "OpDistinctStore" | "OpRefIsNull"
                    );
                    for a in args {
                        if matches!(a.unspan(), Value::Var(w) if *w == buf) {
                            if is_release {
                                released += 1;
                            } else if callee.is_loft_defined() {
                                calls += 1;
                            }
                        }
                    }
                }
                _ => {}
            }
            false
        });
        calls == 1 && total == released + calls
    };
    let mut out_map = ElemFirstMap::default();
    body.any_node(&mut |n| {
        let Value::Block(bl) = n else { return false };
        let ops = &bl.operators;
        let code_idx: Vec<usize> = (0..ops.len())
            .filter(|j| !matches!(ops[*j].unspan(), Value::Line(_)))
            .collect();
        // Temp declarations in this list: tmp -> (key var, first position, last position,
        // the call's arguments when a call fills it).
        let mut decls: HashMap<u16, (u16, usize, usize, Option<&[Value]>)> = HashMap::new();
        for (k, &j) in code_idx.iter().enumerate() {
            if let Some(args) = call_named(&ops[j], data, "OpDatabase")
                && let Some(vdb) = as_var(args.first())
                && vars.name(vdb).starts_with("__vdb")
                && k + 2 < code_idx.len()
                && let Value::Set(tmp, x) = ops[code_idx[k + 1]].unspan()
                && matches!(x.unspan(), Value::Call(d, cargs)
                    if data.def(*d).name() == "OpGetField"
                        && as_var(cargs.first()) == Some(vdb))
                && call_named(&ops[code_idx[k + 2]], data, "OpSetInt4")
                    .is_some_and(|a| as_var(a.first()) == Some(vdb))
            {
                decls.insert(*tmp, (vdb, k, k + 2, None));
            } else if place_on
                && let Some(buf) = lazy_buffer_guard(&ops[j], data)
                && k + 1 < code_idx.len()
                && let Value::Set(tmp, rhs) = ops[code_idx[k + 1]].unspan()
                && let Value::Call(g, gargs) = rhs.unspan()
                && fills_its_buffer(data, *g)
                && data
                    .def(*g)
                    .hidden_return_buffer_attr()
                    .is_some_and(|i| as_var(gargs.get(i)) == Some(buf))
                && vars.tp(*tmp).depend() == [buf]
                && buffer_serves_one_call(buf)
            {
                decls.insert(*tmp, (buf, k, k + 1, Some(gargs.as_slice())));
            }
        }
        if decls.is_empty() {
            return false;
        }
        // Does `v` name the destination in a way that could reach its collection?  For a local
        // vector, any naming does.  For a record's collection field, a naming that reaches
        // only ANOTHER field — `sc.sw` handed to the call that fills the points, a sibling
        // collection grown — moves nothing in this one (`namings_avoid_place`, the view
        // leaf's own test), and the early element stays where it was minted.
        let names_out = |v: &Value, out: u16, out_fld: i32, out_tp: i32| -> bool {
            if !v.reads_var(out) {
                return false;
            }
            if out_fld == 65535 {
                return true;
            }
            let (Ok(ptp), Ok(fld)) = (u16::try_from(out_tp), u16::try_from(out_fld)) else {
                return true;
            };
            let pos = stores.field_position(ptp, fld);
            pos == u16::MAX
                || !namings_avoid_place(data, stores, def_nr, v, (out, u32::from(pos)), None)
        };
        // Does `v` read a view of the destination's elements — which the early mint may already
        // have moved (`destination_views`)?
        let dest_pos = |out_fld: i32, out_tp: i32| -> Option<u32> {
            let (Ok(ptp), Ok(fld)) = (u16::try_from(out_tp), u16::try_from(out_fld)) else {
                return None;
            };
            if fld == u16::MAX {
                return None;
            }
            let pos = stores.field_position(ptp, fld);
            (pos != u16::MAX).then(|| u32::from(pos))
        };
        let moved_by = |g: &AppendGroup| -> HashSet<u16> {
            destination_views(data, body, vars, g.out, dest_pos(g.out_fld, g.out_tp))
        };
        let reads_view = |v: &Value, moved: &HashSet<u16>, elms: &[u16]| -> bool {
            moved.iter().any(|w| !elms.contains(w) && v.reads_var(*w))
        };
        // One append group — (reservation +) mint + field writes + finish — starting at `k`
        // of `list`: the destination, its mint arguments, the element, and the paired copies.
        let group_at = |list: &[Value], idx: &[usize], k: usize| -> Option<AppendGroup> {
            let j = idx[k];
            // The mint, with the reservation that precedes it for a local vector.
            let (mint_k, out, out_fld, size) =
                if let Some(pa) = call_named(&list[j], data, "OpPreAllocVector") {
                    let (Some(out), Some(size)) = (as_var(pa.first()), as_int(pa.get(2))) else {
                        return None;
                    };
                    if as_int(pa.get(1)) != Some(1) || k + 1 >= idx.len() {
                        return None;
                    }
                    (k + 1, out, 65535, size)
                } else if place_on
                    && let Value::Set(_, m) = list[j].unspan()
                    && let Some(margs) = call_named(m, data, "OpNewRecord")
                    && let (Some(out), Some(fld)) = (as_var(margs.first()), as_int(margs.get(2)))
                    && fld != 65535
                {
                    (k, out, fld, 0)
                } else {
                    return None;
                };
            let Value::Set(elm, mint) = list[idx[mint_k]].unspan() else {
                return None;
            };
            let margs = call_named(mint, data, "OpNewRecord")?;
            if as_var(margs.first()) != Some(out) || as_int(margs.get(2)) != Some(out_fld) {
                return None;
            }
            let out_tp = as_int(margs.get(1))?;
            if set_counts.contains_key(&out) {
                return None;
            }
            if out_fld == 65535 {
                // `out`: a plain vector whose element is a plain struct.
                if !matches!(vars.tp(out).peel_link(), Type::Vector(e, _)
                    if plain_record_type(data, e).is_some())
                {
                    return None;
                }
            } else {
                // `out`: a record variable whose collection field stores plain structs INLINE.
                let (Ok(ptp), Ok(fld)) = (u16::try_from(out_tp), u16::try_from(out_fld)) else {
                    return None;
                };
                if plain_record_type(data, vars.tp(out)).is_none()
                    || (ptp as usize) >= stores.types.len()
                {
                    return None;
                }
                let coll = stores.field_type(ptp, fld);
                if coll == u16::MAX
                    || !matches!(
                        stores.types.get(coll as usize).map(|t| &t.parts),
                        Some(crate::database::Parts::Vector(_))
                    )
                {
                    return None;
                }
                let content = stores.content(coll);
                if content == u16::MAX
                    || stores.is_linked(content)
                    || !matches!(
                        stores.types.get(content as usize).map(|t| &t.parts),
                        Some(crate::database::Parts::Struct(_))
                    )
                {
                    return None;
                }
            }
            // Scan the group: paired appends, and the finish that closes it.  Under E-2 a
            // statement that names neither `out` nor the element's paired fields, and jumps
            // nowhere, is part of the group too — the literal's nested records and scalar
            // writes run where they ran, after the mint.
            let mut appended: Vec<(u16, i32)> = Vec::new();
            let mut others: Vec<usize> = Vec::new();
            let mut fin = false;
            for &j2 in idx.iter().skip(mint_k + 1) {
                let stmt = &list[j2];
                if let Some(a) = call_named(stmt, data, "OpSetInt4")
                    && as_var(a.first()) == Some(*elm)
                {
                    continue;
                }
                if let Some(a) = call_named(stmt, data, "OpAppendVector")
                    && let Value::Call(gd, gargs) = a[0].unspan()
                    && data.def(*gd).name() == "OpGetField"
                    && as_var(gargs.first()) == Some(*elm)
                    && let Some(off) = as_int(gargs.get(1))
                    && let Some(tmp) = as_var(a.get(1))
                {
                    appended.push((tmp, off));
                    continue;
                }
                // A scalar field set on the element stays untouched.
                if let Value::Call(sd, sargs) = stmt.unspan()
                    && data.def(*sd).name().starts_with("OpSet")
                    && as_var(sargs.first()) == Some(*elm)
                {
                    continue;
                }
                if let Some(a) = call_named(stmt, data, "OpFinishRecord")
                    && as_var(a.first()) == Some(out)
                    && as_var(a.get(1)) == Some(*elm)
                {
                    fin = true;
                    break;
                }
                if place_on && !names_out(stmt, out, out_fld, out_tp) && !jumps_out(stmt) {
                    others.push(j2);
                    continue;
                }
                break;
            }
            if !fin || appended.is_empty() {
                return None;
            }
            // Nothing else in the group reaches a paired field: the only writes to it are the
            // suppressed zero and the paired copy.
            let paired: HashSet<i32> = appended.iter().map(|(_, o)| *o).collect();
            if others.iter().any(|&j2| {
                list[j2].any_node(&mut |m| {
                    matches!(m, Value::Call(d, a)
                        if (*d as usize) < data.definitions.len()
                            && data.def(*d).name() == "OpGetField"
                            && as_var(a.first()) == Some(*elm)
                            && as_int(a.get(1)).is_some_and(|o| paired.contains(&o)))
                })
            }) {
                if trace {
                    eprintln!(
                        "[elemfirst] {}: a paired field is written twice",
                        def.name()
                    );
                }
                return None;
            }
            Some(AppendGroup {
                out,
                out_fld,
                out_tp,
                size,
                elm: *elm,
                appended,
            })
        };
        // The group of one arm of an `if`: its list holds a group, and nothing before it
        // names the destination or jumps out.
        let arm_group = |arm: &Value, out_of: Option<u16>| -> Option<AppendGroup> {
            let list: &[Value] = match arm.unspan() {
                Value::Block(bl) => &bl.operators,
                Value::Insert(ops) => ops,
                _ => return None,
            };
            let idx: Vec<usize> = (0..list.len())
                .filter(|x| !matches!(list[*x].unspan(), Value::Line(_)))
                .collect();
            for ak in 0..idx.len() {
                if let Some(g) = group_at(list, &idx, ak) {
                    if out_of.is_some_and(|o| o != g.out) {
                        return None;
                    }
                    let moved = moved_by(&g);
                    let clean = idx[..ak]
                        .iter()
                        .all(|&x| {
                            !names_out(&list[x], g.out, g.out_fld, g.out_tp)
                                && !jumps_out(&list[x])
                                && !reads_view(&list[x], &moved, &[g.elm])
                        });
                    return clean.then_some(g);
                }
            }
            None
        };
        for (k, &j) in code_idx.iter().enumerate() {
            // A group in this list, or (@PLN164 E-2b) one in each arm of an `if` this list
            // holds — the parser shape `if c { out += [A { v: p }] } else { out += [B { v: p }] }`.
            // One early element serves both arms: the second arm's mint becomes an alias of
            // the first's, and each arm keeps its own writes and its finish.
            let mut groups: Vec<AppendGroup> = Vec::new();
            if let Some(g) = group_at(ops, &code_idx, k) {
                groups.push(g);
            } else if place_on
                && let Value::If(cond, then_v, else_v) = ops[j].unspan()
                && let Some(gt) = arm_group(then_v, None)
                && let Some(ge) = arm_group(else_v, Some(gt.out))
                && !names_out(cond, gt.out, gt.out_fld, gt.out_tp)
                && !jumps_out(cond)
                && !reads_view(cond, &moved_by(&gt), &[gt.elm, ge.elm])
                && gt.elm != ge.elm
                && (gt.out_fld, gt.out_tp, gt.size) == (ge.out_fld, ge.out_tp, ge.size)
                && {
                    let mut a = gt.appended.clone();
                    let mut b = ge.appended.clone();
                    a.sort_unstable();
                    b.sort_unstable();
                    a == b
                }
            {
                groups.push(gt);
                groups.push(ge);
            } else {
                continue;
            }
            let AppendGroup {
                out,
                out_fld,
                out_tp,
                size,
                elm,
                ref appended,
            } = groups[0];
            let elm = &elm;
            let arms = u32::try_from(groups.len()).unwrap_or(u32::MAX);
            // Pair each appended temp with a declaration EARLIER in this list.
            let mut binds: Vec<ElemBind> = Vec::new();
            let mut first_decl = usize::MAX;
            let mut sound = true;
            for (tmp, off) in appended {
                let Some((key, dk, dend, call)) = decls.get(tmp).copied() else {
                    // Under E-2 a value this list does not declare keeps its copy at the
                    // append, into the early element; before it, it declined the group.
                    if place_on && appended.iter().filter(|(t, _)| t == tmp).count() == 1 {
                        continue;
                    }
                    sound = false;
                    break;
                };
                if dend >= k {
                    // the declaration must fully precede the group
                    sound = false;
                    break;
                }
                // A call's other arguments may not name the destination.
                if let Some(cargs) = call
                    && cargs.iter().any(|a| names_out(a, out, out_fld, out_tp))
                {
                    sound = false;
                    break;
                }
                first_decl = first_decl.min(dk);
                binds.push(ElemBind {
                    tmp: *tmp,
                    vdb: key,
                    field_off: *off,
                    first: false,
                    from_call: call.is_some(),
                });
            }
            if !sound || binds.is_empty() {
                if trace {
                    eprintln!("[elemfirst] {}: append pairing incomplete", def.name());
                }
                continue;
            }
            // `out`'s own declaration must PRECEDE the first temp's: a temp declared
            // first would put the prelude before `out`'s binding exists (the sqldb
            // schema fixture's E0425, and a mint into a store not yet allocated).
            // `out` declared in an ENCLOSING block is not in this list and passes.
            if decls
                .get(&out)
                .is_some_and(|(_, ok, _, _)| *ok >= first_decl)
            {
                if trace {
                    eprintln!(
                        "[elemfirst] {}: out declared after the first temp",
                        def.name()
                    );
                }
                continue;
            }
            // Between the first declaration and the group: no naming of `out` outside the
            // declarations themselves, no jump out of the list, and no read of a view the
            // early mint may have moved — the declarations' own arguments included.
            // The group's own elements are named there only by the parser's null pre-inits,
            // which the emitter drops for an early element.
            let own: Vec<u16> = groups.iter().map(|g| g.elm).collect();
            let moved = moved_by(&groups[0]);
            if let Some(w) = code_idx[first_decl..k].iter().find_map(|&j2| {
                moved
                    .iter()
                    .find(|w| !own.contains(w) && ops[j2].reads_var(**w))
            })
            {
                if trace {
                    eprintln!(
                        "[elemfirst] {}: `{}` may view the destination and is read before the append",
                        def.name(),
                        vars.name(*w)
                    );
                }
                continue;
            }
            for &j2 in &code_idx[first_decl..k] {
                let is_call_decl = binds.iter().any(|b| {
                    b.from_call
                        && decls.get(&b.tmp).is_some_and(|(_, dk, dend, _)| {
                            code_idx[*dk] == j2 || code_idx[*dend] == j2
                        })
                });
                if (!is_call_decl && names_out(&ops[j2], out, out_fld, out_tp))
                    || jumps_out(&ops[j2])
                {
                    sound = false;
                }
            }
            if !sound {
                if trace {
                    eprintln!(
                        "[elemfirst] {}: out read or a jump between decl and append",
                        def.name()
                    );
                }
                continue;
            }
            // Whole-function reconciliation per temp: its mentions are its builds
            // (between its own declaration and the group), the one append — and
            // nothing else (a later read, a second append, an escape all decline).
            for b in &binds {
                // The declaration is the temp's ONLY binding: a rebind points the temp at
                // another store, and the element — whose copy is suppressed — keeps what
                // the declaration built (`p: vector = []; if c { p = mk() }` appended an
                // empty vector wherever `c` held).  A call-filled temp's own `Set` is its one
                // counted binding; a literal-built temp's is a declaration and uncounted.
                let rebinds = set_counts.get(&b.tmp).copied().unwrap_or(0);
                if rebinds != u32::from(b.from_call) {
                    if trace {
                        eprintln!(
                            "[elemfirst] {}: {} is bound more than once",
                            def.name(),
                            vars.name(b.tmp)
                        );
                    }
                    sound = false;
                    continue;
                }
                let total = uses(b.tmp);
                let mut allowed = arms; // one OpAppendVector per arm
                let (_, dk, _, _) = decls[&b.tmp];
                for &j2 in &code_idx[dk + 1..k] {
                    ops[j2].any_node(&mut |m| {
                        if matches!(m, Value::Var(w) if *w == b.tmp) {
                            allowed += 1;
                        }
                        false
                    });
                }
                if total != allowed {
                    if trace {
                        eprintln!(
                            "[elemfirst] {}: {} has uses beyond its build ({} vs {}{})",
                            def.name(),
                            vars.name(b.tmp),
                            total,
                            allowed,
                            if views.is_some() {
                                ""
                            } else {
                                ", no view plan"
                            }
                        );
                    }
                    sound = false;
                }
            }
            if !sound {
                continue;
            }
            // The FIRST temp in declaration order carries the mint.
            binds.sort_by_key(|b| decls[&b.tmp].1);
            if let Some(b0) = binds.first_mut() {
                b0.first = true;
            }
            if trace {
                eprintln!(
                    "[elemfirst] {}: {} built in its element ({})",
                    def.name(),
                    binds
                        .iter()
                        .map(|b| vars.name(b.tmp))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if out_fld == 65535 {
                        "local vector"
                    } else {
                        "a record's collection"
                    }
                );
            }
            let idx = out_map.pairs.len();
            for b in &binds {
                out_map.by_vdb.insert(b.vdb, idx);
                if b.from_call {
                    out_map.buf_place.insert(b.vdb, (*elm, b.field_off));
                }
            }
            out_map.elms.insert(*elm);
            out_map.by_elm.insert(*elm, idx);
            let aliases: Vec<u16> = groups[1..].iter().map(|g| g.elm).collect();
            for a in &aliases {
                out_map.elms.insert(*a);
                out_map.by_elm.insert(*a, idx);
            }
            out_map.pairs.push(ElemFirst {
                out,
                out_tp,
                out_fld,
                elm: *elm,
                aliases,
                prealloc_size: size,
                binds,
            });
        }
        false
    });
    // A temp or an element serving TWO admitted pairs is beyond this keying.
    let mut vdb_seen: HashMap<u16, u32> = HashMap::new();
    for p in &out_map.pairs {
        for b in &p.binds {
            *vdb_seen.entry(b.vdb).or_default() += 1;
        }
    }
    if vdb_seen.values().any(|c| *c > 1) {
        return ElemFirstMap::default();
    }
    out_map
}

/// @PLN157 § V-aa (`@FR-R-ValueRecord`) — the functions whose NO-HEAP RECORD result is
/// returned BY VALUE (a Rust tuple of its fields, in registers) instead of through a
/// return buffer, with the field offsets that map a `OpGetField` to a tuple index.
#[derive(Default)]
pub struct ValueRecords {
    /// Admitted functions → their record type.
    pub fns: HashMap<u32, u16>,
    /// `(record type, byte offset)` → tuple index, for the call site's field reads.
    pub index: HashMap<(u16, i64), usize>,
    /// The Rust tuple type of an admitted function's result.
    pub tuple: HashMap<u32, String>,
    /// Per admitted function, its fields in ORDER: `(byte offset, Rust type)` — what the
    /// tuple carries, and what the live-reload arm must read back out of the record the
    /// interpreter answers with.
    pub fields: HashMap<u32, Vec<(i64, &'static str)>>,
    /// @PLN164 C5 — per admitted function with a VIEW LEAF, the places its exits deliver a
    /// view of, which is what a call site must keep undisturbed ([`ViewPlan`]).  A function
    /// whose record owns no heap has no entry.
    pub views: HashMap<u32, ViewPlan>,
    /// @PLN164 C5 — per admitted function, the byte offsets of its view-leaf fields
    /// ([`ViewOffsets`]): what a call site may read off the tuple's reference.
    pub view_offs: ViewOffsets,
}

/// Default-ON since 2026-09-14 (@PLN157 § V-ah stage 1); `LOFT_NO_VALUE_RECORD=1` restores
/// the return buffer for every record return — the bisect step for a wrong field out of a
/// record-returning call on native.
///
/// It was opt-in from 2026-09-12 because the call-site gate did not hold over the script
/// corpus: 376 compile errors in three classes.  Two were shapes the gate could not see —
/// a result bound into a compiler `__lift_` temp whose set lowering reads `.store_nr` off
/// it, and a value branch joining an admitted call with a record expression — and the
/// third, 218 of the errors, was the fn-ref DISPATCH: the `match` a `CallRef` emits takes
/// its arms from a signature scan, so every arm shares one return type, and a gate that
/// asked "which functions can a fn-ref reach?" a second way (by returned record, beside a
/// `FnRef` node) missed a lambda that reached a dispatch through a typed variable alone.
/// The gate now reads the arm set from `fnref::dispatch_arms`, the emitter's own home for
/// that question, and declines every arm.
///
/// The library integration the opt-in phase existed for is closed: a cdylib bridge now
/// MATERIALISES the tuple into the destination record it already owns
/// (`native_lib::shared_bridge_wrapper`), so the C boundary keeps its record contract
/// while loft-to-loft calls inside the library take the value path.
fn value_record_disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_VALUE_RECORD").is_ok_and(|v| v != "0"))
}

/// The scalar field kinds a register tuple can carry: no heap, no collection, no nested
/// record — the fields `rust_type` maps to a plain Rust scalar.
fn value_field_type(tp: &Type) -> Option<&'static str> {
    match tp.base() {
        Type::Float => Some("f64"),
        Type::Single => Some("f32"),
        Type::Integer(_) => Some("i64"),
        Type::Boolean => Some("bool"),
        _ => None,
    }
}

/// @PLN157 § V-aa (`@FR-R-ValueRecord`) — which record-returning functions may return
/// their fields in registers.
///
/// Two gates, and the second is what makes the first safe to apply per FUNCTION: the
/// result type is a plain no-heap struct of at most [`VALUE_RECORD_MAX_FIELDS`] scalar
/// fields; and EVERY call site in the program consumes the result by reading fields off a
/// local it binds — never storing it, passing it on, returning it or binding it into a
/// collection.  A single non-reading site declines the whole function, so no site has to
/// materialise a record out of a tuple and no site can be made slower.
pub fn value_records(data: &Data, stores: &Stores) -> ValueRecords {
    let mut out = ValueRecords::default();
    if value_record_disabled() {
        return out;
    }
    let trace = std::env::var("LOFT_TRACE_VALUEREC").is_ok();
    // Candidates by RESULT TYPE.
    let mut cand: HashMap<u32, u16> = HashMap::new();
    for d_nr in 0..data.definitions.len() as u32 {
        let def = data.def(d_nr);
        if !matches!(def.def_type, crate::data::DefType::Function)
            || matches!(def.code(), Value::Null)
        {
            continue;
        }
        let Type::Reference(rd, _) = def.returned().peel_link() else {
            continue;
        };
        if trace {
            eprintln!("[valuerec] candidate {} returns ref({rd})", def.name());
        }
        let Some(tp) = plain_record_type(data, def.returned()) else {
            if trace {
                eprintln!("[valuerec] {}: not a plain record", def.name());
            }
            continue;
        };
        // @PLN164 C5 (`@FR-O-ViewField`) — a record that OWNS heap is a candidate only
        // through the view leaf: each heap field must be one a reference can deliver
        // ([`view_leaf_type`]) and the body gate must find the place it views.  With the
        // unit off, owning heap declines the record as it did before.
        let views_ok = view_field_on();
        if stores.owns_heap(tp) && !views_ok {
            continue;
        }
        let (crate::database::Parts::Struct(fields) | crate::database::Parts::EnumValue(_, fields)) =
            &stores.types[tp as usize].parts
        else {
            continue;
        };
        if fields.is_empty() || fields.len() > VALUE_RECORD_MAX_FIELDS {
            continue;
        }
        let mut parts: Vec<&'static str> = Vec::new();
        let mut order: Vec<(i64, &'static str)> = Vec::new();
        let mut ok = true;
        for (i, f) in fields.iter().enumerate() {
            // Pair the schema field with the DECLARATION that named it, by name.  The two
            // lists are not the same list and need not be the same length: a runtime
            // schema can carry a field the definition declares no attribute for, and
            // indexing `attributes` by the schema's position then reads another field's
            // type -- or panics, which is what it did (`882-keyed-element-read-borrows-
            // its-container.loft` crashed the compiler: "len is 2 but the index is 2").
            // No match means the record has a part this analysis cannot account for, so
            // the function declines and keeps its return buffer; declining only ever costs
            // the optimisation.
            let Some(a_nr) = data
                .def(*rd)
                .attributes
                .iter()
                .position(|a| a.name == f.name)
            else {
                ok = false;
                break;
            };
            let ftp = data.attr_type(*rd, a_nr);
            // `integer` at its 8-byte width only: the tuple carries an `i64`, and the
            // getter/setter the value path pairs it with (`OpGetInt`/`OpSetInt`, the live
            // arm's `get_int`) read and write eight bytes.  A narrow field (a ranged or
            // `size(1)` alias) declines the function rather than reading its neighbour.
            if matches!(ftp.base(), Type::Integer(_)) && stores.size(f.content) != 8 {
                ok = false;
                break;
            }
            if let Some(rt) = value_field_type(&ftp) {
                parts.push(rt);
                order.push((i64::from(f.position), rt));
            } else if views_ok && view_leaf_type(&ftp) {
                parts.push(VIEW_LEAF_PART);
                order.push((i64::from(f.position), VIEW_LEAF_PART));
            } else {
                ok = false;
                break;
            }
            out.index.insert((tp, i64::from(f.position)), i);
        }
        if !ok {
            continue;
        }
        // The BODY gate — every result position a value leaf — runs in the fixpoint
        // below, because what counts as a leaf depends on what else is admitted.
        // A ONE-FIELD record needs the trailing comma: `(bool)` is Rust for a
        // PARENTHESISED bool, not a 1-tuple, so the signature promised a scalar while
        // every call site read `.0` off it and the generated crate would not compile
        // (measured on `pub fn tx_new() -> Tx { Tx { open: false } }` in the sqldb
        // fixture: "`bool` is a primitive type and therefore doesn't have fields").
        // The same comma is required on the VALUE side in `emit.rs`, or the two disagree.
        let tuple = if parts.len() == 1 {
            format!("({},)", parts[0])
        } else {
            format!("({})", parts.join(", "))
        };
        out.tuple.insert(d_nr, tuple);
        out.fields.insert(d_nr, order);
        cand.insert(d_nr, tp);
    }
    if cand.is_empty() {
        return out;
    }
    // A function reachable through a FN-REF cannot change its ABI.  The dispatch a `CallRef`
    // emits is a `match` whose arms are every definition `fnref::dispatch_arms` admits for
    // the fn variable's type — all of them sharing one argument list and one return type —
    // so converting one arm to a tuple breaks the join and drops the buffer argument the
    // others still take.  The question is asked ONCE, of the same function the emitter
    // builds the match from: every arm of every `CallRef` in the program is declined, and
    // so is every `FnRef` target, whose dispatch can sit where no `CallRef` in loft code
    // shows it (a `#rust` template calling through the fn-ref value).  Over-broad on
    // purpose — a declined function costs the rewrite and never correctness
    // (`@FR-R-ValueRecord`).  Asked a second way it drifted: the by-record test that stood
    // here missed a lambda reaching a dispatch through a typed variable with no `FnRef`
    // node beside it.
    let mut arms: HashSet<u32> = HashSet::new();
    let every: HashSet<u32> = HashSet::new();
    for d_nr in 0..data.definitions.len() as u32 {
        let def = data.def(d_nr);
        if matches!(def.code(), Value::Null) {
            continue;
        }
        let vars = def.variables();
        def.code().any_node(&mut |n| {
            match n {
                Value::FnRef(target, _, _) => {
                    if let Ok(t) = u32::try_from(*target)
                        && (t as usize) < data.definitions.len()
                    {
                        arms.insert(t);
                    }
                }
                Value::CallRef(v, args) => {
                    if *v < vars.count()
                        && let Some(found) =
                            super::fnref::dispatch_arms(data, &every, vars.tp(*v), args.len())
                    {
                        arms.extend(found.into_iter().map(|a| a.d_nr));
                    }
                }
                // The third spelling: a `par` worker, named by its number as an integer
                // argument of the queue op, which the parallel emitter calls through its
                // own buffered spelling.
                Value::Call(d, args) => {
                    if (*d as usize) < data.definitions.len()
                        && let Some((i, min)) =
                            super::fnref::parallel_worker_arg(data.def(*d).name())
                        && args.len() >= min
                        && let Value::Int(n) = args[i].unspan()
                        && *n >= 0
                    {
                        arms.insert(*n as u32);
                    }
                }
                _ => {}
            }
            false
        });
    }
    cand.retain(|d_nr, _| {
        let keep = !arms.contains(d_nr);
        if !keep && trace {
            crate::loft_eprintln!(
                "[valuerec] {}: an arm of a fn-ref dispatch",
                data.def(*d_nr).name()
            );
        }
        keep
    });
    // The view-leaf plans, rebuilt each round beside the body gate that decides them (the
    // admitted set they read changes with it) and kept for the round that ends the fixpoint.
    let mut views: HashMap<u32, ViewPlan> = HashMap::new();
    // @PLN164 C5 — the callee half of `(B-Disturb)`, built ONCE over the whole program as
    // the scope pass builds it: a site asks whether a call between its bind and its last
    // read can grow the container its leaf views, and that is a fact about the callee.
    let disturbed =
        view_field_on().then(|| crate::scopes::disturbed_params_map(data, Some(stores)));

    // The view-leaf OFFSETS are fixed by the record's type, so they are read once: every
    // reader of a value local needs them to account a field read the tuple serves.
    let view_offs: ViewOffsets = out
        .fields
        .iter()
        .map(|(d, fs)| {
            (
                *d,
                fs.iter()
                    .filter(|(_, rt)| is_view_part(rt))
                    .map(|(off, _)| *off)
                    .collect(),
            )
        })
        .collect();
    // Admission is a FIXPOINT.  A body's tail may FORWARD another candidate's result and a
    // site may bind a BRANCH of candidate calls, so declining one function can decline
    // another; every round only removes, so it ends.  The body gate and the site gate read
    // the same three helpers the emitter reads (`value_shape`, `value_locals_in`,
    // `value_view_leaves`), so what is admitted here is exactly what is emitted there.
    loop {
        let before = cand.len();
        let admitted: HashSet<u32> = cand.keys().copied().collect();
        cand.retain(|d_nr, record| {
            let mut why = value_body(data, *d_nr, &admitted, &view_offs);
            // @PLN164 C5 — and, for a record with a heap field, the body must deliver a
            // view of a place that outlives the frame for every one of its exits.
            if why.is_none()
                && let Some(fields) = out.fields.get(d_nr)
                && fields.iter().any(|(_, rt)| is_view_part(rt))
            {
                // @PLN164 C5 / `(R-Escape)` — a `pub` function's result can leave the
                // unit the compiler sees whole: a library's cdylib bridge materialises a
                // tuple field by field into the record its ABI promises, and a reference
                // is not a field it can write.  So the record form stands for every `pub`
                // function, which costs the rewrite there and never a value; the internal
                // functions a `pub` one calls are where the row is.
                if data.def(*d_nr).pub_visible {
                    why = Some("a view leaf would cross a library API (R-Escape)");
                } else {
                    let locals = value_locals_in(data, *d_nr, &admitted, &view_offs);
                    let leaves = collect_leaves(data.def(*d_nr).code(), &locals, true);
                    match view_leaf_plan(
                        data,
                        stores,
                        *d_nr,
                        fields,
                        &leaves,
                        *record,
                        disturbed.as_ref(),
                    ) {
                        Some(plan) => {
                            views.insert(*d_nr, plan);
                        }
                        None => why = Some("a heap field is not a view leaf"),
                    }
                }
            }
            if let Some(why) = why
                && trace
            {
                crate::loft_eprintln!("[valuerec] {}: {why}", data.def(*d_nr).name());
            }
            why.is_none()
        });
        let admitted: HashSet<u32> = cand.keys().copied().collect();
        let mut declined: HashSet<u32> = HashSet::new();
        for caller in 0..data.definitions.len() as u32 {
            let cdef = data.def(caller);
            if matches!(cdef.code(), Value::Null) {
                continue;
            }
            let locals = value_locals_in(data, caller, &admitted, &view_offs);
            let own = admitted.contains(&caller).then_some(caller);
            let c = ShapeCtx {
                data,
                def_nr: caller,
                admitted: &admitted,
                locals: &locals,
                own,
            };
            let top = if own.is_some() {
                Pos::Tail
            } else {
                Pos::Operand
            };
            site_walk(cdef.code(), top, &c, &mut declined);
            // @PLN164 C5 — and the view leaf's own site condition: the bind dominates the
            // reads and nothing between them disturbs the place the leaf views.
            if !views.is_empty() {
                declined.extend(view_sites_declined(
                    data,
                    stores,
                    caller,
                    &views,
                    &locals,
                    &view_offs,
                    disturbed.as_ref(),
                ));
            }
        }
        if trace {
            for d in &declined {
                crate::loft_eprintln!(
                    "[valuerec] {}: a site consumes its record",
                    data.def(*d).name()
                );
            }
        }
        cand.retain(|d, _| !declined.contains(d));
        if cand.len() == before {
            break;
        }
    }
    out.fns = cand;
    out.tuple.retain(|d, _| out.fns.contains_key(d));
    out.fields.retain(|d, _| out.fns.contains_key(d));
    views.retain(|d, _| out.fns.contains_key(d));
    out.views = views;
    out.view_offs = view_offs;
    out.view_offs
        .retain(|d, offs| out.fns.contains_key(d) && !offs.is_empty());
    out
}

/// The widest record the value path carries.  A register tuple past this is spilled by
/// the ABI anyway, and the win is in the small ones (`Pt`, `Smp`).
pub const VALUE_RECORD_MAX_FIELDS: usize = 6;

/// Is `tp` the record `OpDatabase` mints to back a vector local — exactly one field, a
/// vector whose elements own no heap?  The fallback is `false`, which costs a loop buffer's
/// reuse and never a value: an element that owns heap must be released by a clear
/// (`@FR-H-ClearRelease`), and a record with any other field is not this shape.
fn one_no_heap_vector(stores: &Stores, tp: i32) -> bool {
    let Ok(kt) = u16::try_from(tp) else {
        return false;
    };
    if (kt as usize) >= stores.types.len() {
        return false;
    }
    let one = matches!(&stores.types[kt as usize].parts,
        crate::database::Parts::Struct(f) if f.len() == 1);
    if !one {
        return false;
    }
    let vec_tp = stores.field_type(kt, 0);
    if vec_tp == u16::MAX
        || (vec_tp as usize) >= stores.types.len()
        || !matches!(
            stores.types[vec_tp as usize].parts,
            crate::database::Parts::Vector(_)
        )
    {
        return false;
    }
    let elem = stores.content(vec_tp);
    elem != u16::MAX && !stores.owns_heap(elem)
}

/// Every node of `v` with whether it stands under a loop, outermost first.
fn walk_loops(v: &Value, in_loop: bool, f: &mut impl FnMut(&Value, bool)) {
    let n = v.unspan();
    f(n, in_loop);
    let inner = in_loop || matches!(n, Value::Loop(_));
    n.for_each_child(&mut |c| walk_loops(c, inner, f));
}

/// @PLN157 § V-al (`@FR-R-LoopBuffer`) — the LOOP BUFFERS of `def_nr`: a per-site vector
/// buffer (`__vdb_N`, the store a vector local declared `[]` is backed by) whose mint
/// stands INSIDE a loop, whose record is one vector field of elements that own no heap,
/// and whose every mention is its own init family — the mint, the `OpGetField` bind, the
/// literal's `OpSetInt4` zero of the vector field — or a free.  Such a buffer's store
/// already survives the iteration (the IR frees it at scope exit, and `OpDatabase` on a
/// var that still holds a store clears that store and claims the record again), so what
/// the re-mint per iteration buys is the clear and nothing else: the emitter keeps the
/// store AND the vector, and resets the vector's length instead — its capacity retained
/// across iterations, as a Rust `Vec` cleared in a loop retains its own.
///
/// Declined: a buffer any other call reaches (a callee could keep a handle into the
/// vector's record), a buffer whose declaration (`Set(v, Null)`) sits inside a loop (the
/// null-bind would orphan the kept store), an element type that owns heap (a length reset
/// would strand what the elements own), and a record that is not exactly one vector
/// field.  The fallback is "not a loop buffer", which costs the reuse and never a value.
/// A generator binds none (its locals persist as coroutine fields), and a body with a
/// `par` block binds none (an arm runs in a worker's frame, not this one).
#[must_use]
pub fn loop_buffers(data: &Data, stores: &Stores, def_nr: u32) -> HashSet<u16> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let mut out = HashSet::new();
    if matches!(body, Value::Null)
        || body.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_)))
    {
        return out;
    }
    let trace = std::env::var("LOFT_TRACE_LOOP_BUFFER").is_ok();
    let mut cand: HashSet<u16> = HashSet::new();
    let mut null_in_loop: HashSet<u16> = HashSet::new();
    walk_loops(body, false, &mut |n, in_loop| match n {
        Value::Call(d, args) if in_loop && (*d as usize) < data.definitions.len() => {
            if matches!(data.def(*d).name(), "OpDatabase" | "OpDatabaseNP")
                && let [a0, a1] = &args[..]
                && let Value::Var(v) = a0.unspan()
                && let Value::Int(tp) = a1.unspan()
                && vars.name(*v).starts_with("__vdb")
                && one_no_heap_vector(stores, *tp)
            {
                cand.insert(*v);
            }
        }
        Value::Set(v, rhs) if in_loop && matches!(rhs.unspan(), Value::Null) => {
            null_in_loop.insert(*v);
        }
        _ => {}
    });
    for v in cand {
        if null_in_loop.contains(&v) {
            if trace {
                eprintln!(
                    "[loop-buffer] {}: {} is declared inside the loop",
                    def.name(),
                    vars.name(v)
                );
            }
            continue;
        }
        let mut mentions = 0u32;
        let mut accounted = 0u32;
        body.any_node(&mut |n| {
            match n {
                Value::Var(w) if *w == v => mentions += 1,
                Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                    let first =
                        matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == v);
                    let int_at = |i: usize, k: i32| {
                        matches!(args.get(i).map(Value::unspan), Some(Value::Int(n)) if *n == k)
                    };
                    if first {
                        match data.def(*d).name() {
                            "OpDatabase" | "OpDatabaseNP" => accounted += 1,
                            "OpGetField" if int_at(1, 0) => accounted += 1,
                            "OpSetInt4" if int_at(1, 0) && int_at(2, 0) => accounted += 1,
                            "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefTag" => accounted += 1,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            false
        });
        if mentions == accounted {
            if trace {
                eprintln!(
                    "[loop-buffer] {}: {} keeps its vector across iterations",
                    def.name(),
                    vars.name(v)
                );
            }
            out.insert(v);
        } else if trace {
            eprintln!(
                "[loop-buffer] {}: {} is reached by {} mention(s) outside its init family",
                def.name(),
                vars.name(v),
                mentions - accounted
            );
        }
    }
    out
}

/// The hidden RETURN-BUFFER attribute of a record-returning function — the parameter
/// `ref_return` appends, a `Reference` or struct-enum marked hidden — by position, or
/// `None` for a function that has none.  ONE predicate for the four sites that drop it:
/// its NAME is `__retbuf` when the parser minted the buffer and the promoted LOCAL's own
/// name (`__ref_3`, `p`) when the buffer IS that local, so a name test sees half of them
/// (measured: `half_chord`'s buffer is `__ref_3`, and the call site kept passing it to a
/// signature that had dropped it).
#[must_use]
pub fn ret_buffer_attr(def: &crate::data::Definition) -> Option<usize> {
    def.attributes().iter().rposition(|a| {
        a.hidden
            && matches!(
                a.typedef.base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
    })
}

/// The getter op that reads a value-record field of Rust type `rt` out of a record — what
/// a VIEW leaf's tuple is built from.  Mirrors the parser's `get_val`: the four scalar
/// kinds the value path admits, `integer` at its 8-byte width only (`value_field_type`).
#[must_use]
pub fn value_getter(rt: &str) -> &'static str {
    match rt {
        "f64" => "OpGetFloat",
        "f32" => "OpGetSingle",
        "bool" => "OpGetBoolean",
        _ => "OpGetInt",
    }
}

/// The setter twin of [`value_getter`] — what a tuple is MATERIALISED into a destination
/// record with (`OpCopyRecord` from a value local).
#[must_use]
pub fn value_setter(rt: &str) -> &'static str {
    match rt {
        "f64" => "OpSetFloat",
        "f32" => "OpSetSingle",
        "bool" => "OpSetBoolean",
        _ => "OpSetInt",
    }
}

// ── @PLN164 E-1 (`@FR-R-ValueRecord`) — a FORWARD writes the tuple into its own buffer ──

/// Default-ON since 2026-09-17, with the view leaf; `LOFT_NO_FORWARD_TUPLE=1` restores the
/// decline — the first bisect step for a wrong field, a leak or a null-store panic at a
/// `return g(…)` of a record-returning function (read once, generation time, `--native` only).
fn forward_tuple_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_FORWARD_TUPLE").is_ok_and(|v| v != "0"))
}

/// @PLN164 E-1 — is `target = rhs` a FORWARD: a call to an admitted function `g` that is
/// handed `target` as its return buffer and whose answer is bound back into `target`?  That is
/// `g`'s record form writing the caller's buffer, and it is the one record-consuming position
/// the value form can serve without declining `g`: the site writes the tuple into `target`
/// exactly as `g`'s own exit would have — minting the buffer where it is null, setting every
/// field, copying a view part's vector — and the call's value is that buffer again.
///
/// It answers `g`.  `None` for everything else, which keeps the ordinary site rules: a
/// `target` that is a value local (that bind takes the tuple itself), a buffer argument that
/// is not `target`, a record type that differs, a nullable buffer.  The parser writes this
/// shape for every `return g(…)` of a function that keeps its record (`one_buffer_chain`).
#[must_use]
pub fn forward_site(
    data: &Data,
    def_nr: u32,
    target: u16,
    rhs: &Value,
    admitted: &HashSet<u32>,
    locals: &HashMap<u16, u32>,
) -> Option<u32> {
    if !forward_tuple_on() || locals.contains_key(&target) {
        return None;
    }
    let Value::Call(g, args) = rhs.unspan() else {
        return None;
    };
    if !admitted.contains(g) {
        return None;
    }
    let callee = data.def(*g);
    let idx = ret_buffer_attr(callee)?;
    if !matches!(args.get(idx).map(Value::unspan), Some(Value::Var(b)) if *b == target) {
        return None;
    }
    let want = plain_record_type(data, callee.returned())?;
    let vars = data.def(def_nr).variables();
    (plain_record_type(data, vars.tp(target)) == Some(want)).then_some(*g)
}

/// Every [`forward_site`] of `def_nr`, keyed by the address of the call's ARGUMENT LIST — the
/// slice the emitter is handed for that call — mapped to the buffer it writes.
#[must_use]
pub fn forward_sites(
    data: &Data,
    def_nr: u32,
    admitted: &HashSet<u32>,
    locals: &HashMap<u16, u32>,
) -> HashMap<usize, u16> {
    let mut out = HashMap::new();
    if !forward_tuple_on() {
        return out;
    }
    data.def(def_nr).code().any_node(&mut |n| {
        if let Value::Set(target, rhs) = n
            && forward_site(data, def_nr, *target, rhs, admitted, locals).is_some()
            && let Value::Call(_, args) = rhs.unspan()
        {
            out.insert(args.as_ptr() as usize, *target);
        }
        false
    });
    out
}

/// The vector type and the element type of the view-leaf field at byte `off` of record type
/// `tp` — what the record form's `OpGetField(buf, off, <vector>)` and `OpAppendVector(…,
/// <element>)` name, derived from the schema as the runtime derives an element type.
#[must_use]
pub fn view_field_types(stores: &Stores, tp: u16, off: i64) -> Option<(u16, u16)> {
    let crate::database::Parts::Struct(fields) = &stores.types.get(tp as usize)?.parts else {
        return None;
    };
    let f = fields.iter().find(|f| i64::from(f.position) == off)?;
    let elem = stores.content(f.content);
    (elem != u16::MAX).then_some((f.content, elem))
}

// ── @PLN164 C5 (`@FR-O-ViewField`) — a returned record's heap field as a VIEW LEAF ──────

/// Default-ON since 2026-09-17, once the native corpus held with it armed (1372 scripts, 69
/// docs, 36 feature examples, no failure); `LOFT_NO_VIEW_FIELD=1` restores the record form —
/// the first bisect step for a wrong or stale vector read out of a record-returning call (read
/// once, generation time, `--native` only — the interpreter keeps the record form and is the
/// values oracle, as § V-aa's value record already does).
fn view_field_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| !std::env::var("LOFT_NO_VIEW_FIELD").is_ok_and(|v| v != "0"))
}

/// The tuple part a VIEW LEAF carries: the `DbRef` of the place the field views, where a
/// scalar field carries its own value.  It is the same reference the record form's
/// `OpGetField` hands a reader, which is why every READ at a call site is unchanged and
/// only a write or a stale place could tell the difference (`@FR-O-ViewField`).
pub const VIEW_LEAF_PART: &str = "DbRef";

/// Is this field one a view leaf may deliver?  A plain `vector<T>` only: its value IS the
/// field slot's reference, so a reader that takes the slot's address reads the container's
/// own vector.  `text` and the keyed kinds are left out — a `text` field is read through
/// its own getters into a `String` and a keyed collection's reads go through the key
/// machinery, so neither is served by handing over the slot — and a nested record field is
/// not a leaf at all.  Leaving a kind out costs the rewrite, never a value.
fn view_leaf_type(ftp: &Type) -> bool {
    matches!(ftp.base(), Type::Vector(_, _))
}

/// Does `rt` name the view-leaf part?
#[must_use]
pub fn is_view_part(rt: &str) -> bool {
    rt == VIEW_LEAF_PART
}

/// The ops a view leaf may stand under as an OPERAND: each answers a VALUE — a length, a
/// null test — rather than a place inside the vector, so nothing the site does with the
/// result can reach the container.  An op that answers a PLACE (an element read) is not one,
/// however read-only it is itself.
const VIEW_LEAF_READ_OPS: [&str; 3] = ["OpLengthVector", "OpRefIsNull", "OpConvBoolFromRef"];

/// What one exit of an admitted body delivers for a view-leaf field — decided once by the
/// gate ([`leaf_source`]), kept in [`ViewPlan::leaves`], and written by the emitter as it
/// stands, so the two cannot drift.
#[derive(Clone, Debug, PartialEq)]
pub enum LeafSource {
    /// The exit does not NAME the field at all — an empty literal, whose value in the
    /// record form is the prefill's own zero — so the leaf is a null reference, which every
    /// read answers exactly as the empty vector it replaces (measured: `len` 0, an iteration
    /// of nothing, an element read null, a `const` argument the same).  A null view can never
    /// go stale, so it needs no place and no disturbance test.
    Null,
    /// Every path to the exit last appended the SAME element temp: the leaf is that element's
    /// field, `OpGetField(Var(elem), off, tp)` — the destination the append copied into.
    Elem { elem: u16, off: i64, tp: i32 },
    /// The paths to the exit appended DIFFERENT element temps (one per arm): no temp names the
    /// place on every path, and the container's LAST element does — the proof is that on
    /// every path the last change to the container was one of those appends.  `parent_tp` and
    /// `fld` are the mint's own arguments, from which the element's stride derives exactly as
    /// the runtime derives it.
    Last {
        root: crate::scopes::ParamPlace,
        parent_tp: i32,
        fld: i32,
        off: i64,
    },
}

impl LeafSource {
    /// The parameter place a call site must keep undisturbed, or `None` for a null view.
    fn root(&self, data: &Data, stores: &Stores, def_nr: u32) -> Option<crate::scopes::ParamPlace> {
        match self {
            LeafSource::Null => None,
            LeafSource::Last { root, .. } => Some(*root),
            LeafSource::Elem { elem, .. } => elem_mint(data, stores, def_nr, *elem).map(|m| m.0),
        }
    }
}

/// @PLN164 C5 — the source a view-leaf field is delivered from at ONE `Object` exit.  ONE
/// home: the gate calls it once per exit and field and keeps the answer in the plan the
/// emitter reads.
///
/// The record form writes such a field with `OpAppendVector(OpGetField(buffer, off, _),
/// src, _)` — the deep copy the leaf removes.  The one resolvable source is the natural
/// form: a LOCAL whose own copy landed in an element appended to a parameter's container,
/// `p = mk_pts(n); sc.ops += [Op { opts: p }]; Mark { …, mpts: p }` — the leaf hands over
/// that element's field, because the same value already lives there in a store the frame
/// does not own.  Whether it does on EVERY path to this exit is [`fresh_leaf`]'s question.
///
/// A source that is itself a parameter-rooted place (`Mark { mpts: sc.first }`) is not
/// resolved: no shape of it reached admission under the per-exit test that stood here, and
/// the record form costs it nothing but the rewrite.
///
/// `None` declines the function.  It is the answer for every shape not named here: a source
/// that is not a local, a local with two destinations outside the frame, a field written by
/// something other than one append, a path on which the element is not the local's copy.
pub fn leaf_source(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    bl: &Block,
    off: i64,
    exit_bufs: &HashSet<u16>,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> Option<LeafSource> {
    let buf = *bl.result.depend().first()?;
    // The append and its source: the append NODE is what the value form drops whole (its
    // destination names the buffer's field, which is the place the leaf takes), and the
    // source is what the leaf is resolved from.
    let mut found: Option<(&Value, &Value)> = None;
    for op in &bl.operators {
        for pair in appends_into(data, op, buf, off) {
            if found.is_some() {
                return None;
            }
            found = Some(pair);
        }
    }
    // Every mention of the BUFFER in this exit must be one the value form accounts for, and
    // that is what makes both answers positive rather than a fallback: the value form drops
    // the whole block, so a mention it does not account for is work the tuple would LOSE.
    // Accounted: the allocate-or-reuse guard, a scalar `OpSet*`, the one recognised append
    // into the leaf's field, and the block's own yield of the buffer.
    //
    // The two spellings this test exists for both FILL the field without an append: a vector
    // LITERAL returned through the buffer pushes element by element
    // (`OpPushInt(OpGetField(buf, off, _), …)`), and a record literal's vector field appends
    // ELEMENTS through the record (`OpNewRecord(buf, <record>, <field nr>)`).  Read as
    // "nothing wrote the field", each delivered a NULL view for a vector the program had
    // filled — the first measured on the corpus (`723-ncc-loop-element-bind`, `len` 0 for 8),
    // the second on this unit's own `d6` cell.
    if !buffer_uses_accounted(data, bl, buf, found.map(|(node, _)| node)) {
        return None;
    }
    let Some((copy, src)) = found else {
        return Some(LeafSource::Null);
    };
    let Value::Var(local) = src.unspan() else {
        return None;
    };
    fresh_leaf(
        &FreshQuery {
            data,
            stores,
            def_nr,
            local: *local,
            exit: std::ptr::from_ref(bl) as usize,
            exit_copy: std::ptr::from_ref(copy.unspan()) as usize,
            exit_bufs,
            disturbed,
        },
        bl,
    )
}

/// The byte stride of the elements a mint `OpNewRecord(_, parent_tp, fld)` appends to —
/// the element type is derived exactly as the runtime's mint derives it (the vector's own
/// content for a whole-vector parameter, the field's content otherwise), so a read of the
/// last element strides as the append wrote.
#[must_use]
pub fn element_stride(stores: &Stores, parent_tp: i32, fld: i32) -> u32 {
    let parent_tp = parent_tp as u16;
    let fld = fld as u16;
    let elem = if fld == u16::MAX {
        stores.content(parent_tp)
    } else {
        stores.content(stores.field_type(parent_tp, fld))
    };
    u32::from(stores.size(elem))
}

/// The element temp `e` an append is building, as its single mint names it:
/// `e = OpNewRecord(<parameter>, parent_tp, fld)`.  Answers the parameter place the element
/// lives in and the mint's `(parent_tp, fld)`.  A temp with a second non-null assignment, or
/// minted anywhere but into a parameter, answers `None` — the element is then not one a view
/// may name.
fn elem_mint(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    e: u16,
) -> Option<(crate::scopes::ParamPlace, i32, i32)> {
    let mut mint: Option<&Value> = None;
    let mut many = false;
    data.def(def_nr).code().any_node(&mut |n| {
        if let Value::Set(w, rhs) = n
            && *w == e
            && !matches!(rhs.unspan(), Value::Null)
        {
            many |= mint.is_some();
            mint = Some(rhs);
        }
        false
    });
    if many {
        return None;
    }
    let rhs = mint?;
    let Value::Call(d, args) = rhs.unspan() else {
        return None;
    };
    if (*d as usize) >= data.definitions.len()
        || !matches!(data.def(*d).name(), "OpNewRecord" | "OpNewRecordNP")
    {
        return None;
    }
    let (Some(Value::Int(ptp)), Some(Value::Int(fld))) = (
        args.get(1).map(Value::unspan),
        args.get(2).map(Value::unspan),
    ) else {
        return None;
    };
    let place = leaf_root(data, stores, def_nr, rhs, 0)?;
    Some((place, *ptp, *fld))
}

/// One append group [`element_first`] can pair: the destination (a local vector, or a record
/// variable's collection field `out_fld`), the mint's type argument, the reservation size of a
/// local vector, the element temp, and the `(temp, field offset)` copies into it.
#[derive(Clone)]
struct AppendGroup {
    out: u16,
    out_fld: i32,
    out_tp: i32,
    size: i32,
    elm: u16,
    appended: Vec<(u16, i32)>,
}

/// The inputs of one [`fresh_leaf`] question.
struct FreshQuery<'a> {
    data: &'a Data,
    stores: &'a Stores,
    def_nr: u32,
    /// The local the exit copies into its view-leaf field.
    local: u16,
    /// The exit `Object` block, by address.
    exit: usize,
    /// The exit's own copy of the local, by address — the one the view replaces.
    exit_copy: usize,
    /// The return buffers of every exit of this record: a copy into one of them is an exit's
    /// own, never a destination the view could name.
    exit_bufs: &'a HashSet<u16>,
    disturbed: Option<&'a crate::scopes::DisturbedParams>,
}

/// @PLN164 E-1 (`@FR-O-ViewField`) — does the local's copy already live in a parameter's
/// container on EVERY path to this exit, and which expression names it there?
///
/// The record form answers the local's value AT THE EXIT.  A view answers the value of an
/// element appended earlier.  The two agree exactly when, on every path from the entry to
/// the exit, the last change to that container was the append of an element whose field
/// took a copy of the local, and neither the local, nor that element's field, nor the
/// container's order changed after the copy.  That is a per-PATH fact, and the walk
/// ([`FreshWalk`]) proves it over the structured IR: an `if` joins its arms, a loop runs to
/// a fixpoint, `break`/`continue`/`return` carry their state to where they go.
///
/// The candidates are fixed first, over the whole body: every copy of the local except the
/// exits' own must go into a field (the same offset each time) of an element minted into
/// ONE parameter place, or into a place the frame owns (a read of the local, and nothing
/// the view could name).  A second place outside the frame declines (`(O-ViewField)`: which
/// place the view names is not decided).
///
/// The local may view only stores the FRAME owns — a call's hidden buffer, the backing record
/// of a `[]` local — because a write to what it views changes the record form's answer and not
/// the element's: naming any of those stores counts as naming the local, and a parameter's
/// store is declined outright, since a caller may hand it in under a second name the walk
/// cannot see.  A variable whose type depends on the local is a view of it, and naming one
/// counts too.
///
/// The answer: one element temp on every path → [`LeafSource::Elem`]; several (one per arm)
/// → [`LeafSource::Last`]; a path where the fact does not hold → `None`, which declines the
/// function and costs the rewrite, never a value.
fn fresh_leaf(q: &FreshQuery, exit_bl: &Block) -> Option<LeafSource> {
    let data = q.data;
    let def = data.def(q.def_nr);
    let vars = def.variables();
    let body = def.code();
    let why = |w: &str| {
        if std::env::var("LOFT_TRACE_VALUEREC").is_ok() {
            eprintln!("[viewleaf] {}: `{}` {w}", def.name(), vars.name(q.local));
        }
    };
    // The stores the local VIEWS — its deps, closed over theirs.  The local's value changes
    // whenever one of them is written, so a naming of any of them counts as a naming of the
    // local; and none may be a parameter, whose store the frame cannot watch (a caller may
    // hand the same store in twice, and a write through the other name would change the
    // local where this walk sees nothing).
    let own = crate::use_analysis::ownership_of(data, q.def_nr, &Value::Var(q.local));
    let mut aliases: HashSet<u16> = HashSet::new();
    let mut stack = vec![q.local];
    while let Some(v) = stack.pop() {
        for d in vars.tp(v).depend() {
            if d != q.local && d < vars.count() && aliases.insert(d) {
                stack.push(d);
            }
        }
    }
    let base_ok = match own {
        crate::use_analysis::Own::Owned => true,
        crate::use_analysis::Own::Borrowed { base } | crate::use_analysis::Own::Join { base } => {
            base == u16::MAX || !vars.is_argument(base)
        }
        crate::use_analysis::Own::Unknown => false,
    };
    if vars.is_argument(q.local) || !base_ok || aliases.iter().any(|a| vars.is_argument(*a)) {
        why(&format!("views a store the frame does not own ({own:?})"));
        return None;
    }
    // And the views OF any of those, closed: naming one names the local too.
    loop {
        let before = aliases.len();
        for w in 0..vars.count() {
            if w != q.local
                && !aliases.contains(&w)
                && vars
                    .tp(w)
                    .depend()
                    .iter()
                    .any(|d| *d == q.local || aliases.contains(d))
            {
                aliases.insert(w);
            }
        }
        if aliases.len() == before {
            break;
        }
    }
    // The candidates: every copy of the local that is not an exit's own.
    let mut cands: HashSet<u16> = HashSet::new();
    let mut copies: HashSet<usize> = HashSet::new();
    let mut shape: Option<(crate::scopes::ParamPlace, i32, i32, i64, i32)> = None;
    let mut ok = true;
    body.any_node(&mut |n| {
        let Value::Call(d, args) = n else {
            return false;
        };
        if (*d as usize) >= data.definitions.len()
            || data.def(*d).name() != "OpAppendVector"
            || std::ptr::from_ref(n) as usize == q.exit_copy
        {
            return false;
        }
        let [dst, src, ..] = &args[..] else {
            return false;
        };
        if !matches!(src.unspan(), Value::Var(w) if *w == q.local) {
            return false;
        }
        let Value::Call(g, gargs) = dst.unspan() else {
            ok &= leaf_root(data, q.stores, q.def_nr, dst, 0).is_none();
            return !ok;
        };
        if (*g as usize) < data.definitions.len()
            && data.def(*g).name() == "OpGetField"
            && let (Some(Value::Var(e)), Some(Value::Int(o)), Some(Value::Int(t))) = (
                gargs.first().map(Value::unspan),
                gargs.get(1).map(Value::unspan),
                gargs.get(2).map(Value::unspan),
            )
        {
            if q.exit_bufs.contains(e) {
                return false;
            }
            if let Some((place, ptp, fld)) = elem_mint(data, q.stores, q.def_nr, *e) {
                let this = (place, ptp, fld, i64::from(*o), *t);
                if shape.is_some_and(|s| s != this) {
                    ok = false;
                    return true;
                }
                shape = Some(this);
                cands.insert(*e);
                copies.insert(std::ptr::from_ref(n) as usize);
                return false;
            }
        }
        // A copy into anything else: a place the frame owns is a READ of the local; a second
        // place outside the frame leaves the view's place undecided.
        ok &= leaf_root(data, q.stores, q.def_nr, dst, 0).is_none();
        !ok
    });
    let Some((place, parent_tp, fld, off, tp)) = shape else {
        why("is copied into no element appended to a parameter");
        return None;
    };
    if !ok {
        why("is copied into a second place outside the frame");
        return None;
    }
    let mut walk = FreshWalk {
        q,
        aliases,
        cands,
        copies,
        place,
        fld,
        off,
        at_exit: None,
        loops: Vec::new(),
        ok: true,
    };
    // The exit's scalar writes into its buffer are the tuple's other parts, evaluated beside
    // the leaf: they may READ the local and nothing more.  Every other statement of the exit
    // is the build the tuple replaces, or a release that runs after the tuple is formed.
    for op in &exit_bl.operators {
        if let Value::Call(d, args) = op.unspan()
            && walk.op_name(*d).starts_with("OpSet")
            && matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if q.exit_bufs.contains(b))
            && !args.iter().skip(1).all(|a| walk.only_reads(a))
        {
            why("is changed by the exit's own tuple");
            return None;
        }
    }
    walk.walk(
        body,
        Fresh {
            live: true,
            last: None,
            copied: BTreeSet::new(),
        },
    );
    let Some(at) = walk.at_exit.take() else {
        why("reaches an exit the walk never visits");
        return None;
    };
    if !walk.ok || !at.live {
        why("stands in control flow the walk does not model");
        return None;
    }
    let Some(last) = at.last else {
        why("is not the container's last element on every path to the exit");
        return None;
    };
    match last.len() {
        0 => None,
        1 => Some(LeafSource::Elem {
            elem: *last.iter().next()?,
            off,
            tp,
        }),
        _ => Some(LeafSource::Last {
            root: place,
            parent_tp,
            fld,
            off,
        }),
    }
}

/// One path state of [`FreshWalk`].
#[derive(Clone, Debug, PartialEq)]
struct Fresh {
    /// Some path reaches this point.  An unreachable point is the identity of the join.
    live: bool,
    /// On every path reaching here, the container's last element is one of these candidate
    /// elements, finished, with its viewed field still the local's copy; `None` where some
    /// path cannot say so.
    last: Option<BTreeSet<u16>>,
    /// The candidate elements whose viewed field holds the local's CURRENT value on every
    /// path: the copy ran, and neither the local nor that field changed since.
    copied: BTreeSet<u16>,
}

impl Fresh {
    fn dead() -> Self {
        Fresh {
            live: false,
            last: None,
            copied: BTreeSet::new(),
        }
    }

    /// Both paths: the last element is one of EITHER path's set, and a field is the local's
    /// copy only where both paths say so.
    fn join(self, o: Fresh) -> Fresh {
        if !self.live {
            return o;
        }
        if !o.live {
            return self;
        }
        Fresh {
            live: true,
            last: match (self.last, o.last) {
                (Some(a), Some(b)) => Some(a.union(&b).copied().collect()),
                _ => None,
            },
            copied: self.copied.intersection(&o.copied).copied().collect(),
        }
    }
}

/// The walk behind [`fresh_leaf`]: a forward must-analysis over loft's STRUCTURED IR.
///
/// An atomic statement is one event ([`FreshWalk::event`]).  Its answer is an UPPER bound
/// on change, in the direction a miss would cost meaning: a statement that names the local
/// anywhere but a read position kills the fact, and so does one that names the container
/// in a way [`namings_avoid_place`] cannot show moves nothing.  The only statements that
/// ESTABLISH the fact are the exact spellings of the candidate's copy and of its finish.
///
/// A straight-line node that carries control flow inside it (a jump, a loop, the exit) is
/// walked child by child, and it may not name anything the fact watches: its own effect
/// would stand after its operands', and this walk does not model that order — so such a node
/// declines rather than being guessed.
struct FreshWalk<'a> {
    q: &'a FreshQuery<'a>,
    aliases: HashSet<u16>,
    cands: HashSet<u16>,
    /// The candidates' copies of the local, by address.
    copies: HashSet<usize>,
    place: crate::scopes::ParamPlace,
    /// The container's field NUMBER, as the mint and the finish both name it.
    fld: i32,
    /// The viewed field's byte offset inside the element.
    off: i64,
    at_exit: Option<Fresh>,
    /// Per enclosing loop, innermost last: the states its `break`s and `continue`s carry.
    loops: Vec<(Fresh, Fresh)>,
    ok: bool,
}

impl FreshWalk<'_> {
    fn op_name(&self, d: u32) -> &str {
        if (d as usize) < self.q.data.definitions.len() {
            self.q.data.def(d).name()
        } else {
            ""
        }
    }

    fn is_local(&self, w: u16) -> bool {
        w == self.q.local || self.aliases.contains(&w)
    }

    fn names_local(&self, v: &Value) -> bool {
        v.any_node(&mut |n| {
            n.names_var_here(self.q.local) || self.aliases.iter().any(|a| n.names_var_here(*a))
        })
    }

    /// Does `n` name the local only where the value is READ?  Three positions are reads: the
    /// operand of an op that answers a value ([`VIEW_LEAF_READ_OPS`]), the SOURCE of an append
    /// whose destination is not the local, and a `const` argument of a call that answers no
    /// reference — a call that answers one could hand back a place inside the local, which
    /// the caller may then write.  Anything else is treated as a change.
    fn only_reads(&self, n: &Value) -> bool {
        match n.unspan() {
            Value::Call(d, args) => args.iter().enumerate().all(|(i, a)| {
                if matches!(a.unspan(), Value::Var(w) if self.is_local(*w)) {
                    self.read_position(*d, i, args)
                } else {
                    self.only_reads(a)
                }
            }),
            x if self.names_local_here(x) => false,
            x => {
                let mut ok = true;
                x.for_each_child(&mut |c| ok &= self.only_reads(c));
                ok
            }
        }
    }

    fn names_local_here(&self, n: &Value) -> bool {
        n.names_var_here(self.q.local) || self.aliases.iter().any(|a| n.names_var_here(*a))
    }

    fn read_position(&self, d: u32, i: usize, args: &[Value]) -> bool {
        let data = self.q.data;
        let name = self.op_name(d);
        if VIEW_LEAF_READ_OPS.contains(&name) {
            return i == 0;
        }
        if name == "OpAppendVector" {
            return i == 1 && !args.first().is_some_and(|a| self.names_local(a));
        }
        if name.is_empty() {
            return false;
        }
        let def = data.def(d);
        def.attributes().get(i).is_some_and(|a| a.value_const)
            && (crate::data::is_scalar(def.returned())
                || matches!(def.returned().base(), Type::Void | Type::Text(_)))
    }

    /// Does `n` touch element `e` only through fields other than the viewed one?  A
    /// projection (`OpGetField`) and a fixed-width scalar read or write
    /// ([`IN_PLACE_SET_OPS`], [`SCALAR_GETTERS`]) carry the field's byte offset as their
    /// second operand and reach nothing outside it.  Every other use of the temp — an element
    /// append through it, a hand-off — may reach the viewed field.
    fn elem_use_elsewhere(&self, n: &Value, e: u16) -> bool {
        match n.unspan() {
            Value::Call(d, args) if matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == e) =>
            {
                let name = self.op_name(*d);
                let field_op = name == "OpGetField"
                    || IN_PLACE_SET_OPS.contains(&name)
                    || SCALAR_GETTERS.contains(&name);
                field_op
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Int(o)) if i64::from(*o) != self.off)
                    && args.iter().skip(1).all(|a| self.elem_use_elsewhere(a, e))
            }
            x if x.names_var_here(e) => false,
            x => {
                let mut ok = true;
                x.for_each_child(&mut |c| ok &= self.elem_use_elsewhere(c, e));
                ok
            }
        }
    }

    /// `OpFinishRecord(<root>, Var(e), _, fld)` into the viewed container: the element `e`
    /// becomes the container's last.
    fn finish_of(&self, n: &Value) -> Option<u16> {
        let Value::Call(d, args) = n else {
            return None;
        };
        if self.op_name(*d) != "OpFinishRecord" {
            return None;
        }
        match (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
            args.get(3).map(Value::unspan),
        ) {
            (Some(Value::Var(root)), Some(Value::Var(elem)), Some(Value::Int(fld)))
                if *root == self.place.0 && *fld == self.fld && self.cands.contains(elem) =>
            {
                Some(*elem)
            }
            _ => None,
        }
    }

    /// One atomic statement.
    fn event(&mut self, v: &Value, mut st: Fresh) -> Fresh {
        let node = v.unspan();
        let root = self.place.0;
        if self.copies.contains(&(std::ptr::from_ref(node) as usize)) {
            if let Value::Call(_, args) = node
                && let Some(Value::Call(_, gargs)) = args.first().map(Value::unspan)
                && let Some(Value::Var(e)) = gargs.first().map(Value::unspan)
            {
                st.copied.insert(*e);
            }
            return st;
        }
        if self.names_local(node) && !self.only_reads(node) {
            st.last = None;
            st.copied.clear();
        }
        let finished = self.finish_of(node);
        let named: Vec<u16> = self
            .cands
            .iter()
            .copied()
            .filter(|e| node.reads_var(*e) && Some(*e) != finished)
            .collect();
        for e in named {
            if !self.elem_use_elsewhere(node, e) {
                st.copied.remove(&e);
                if st.last.as_ref().is_some_and(|l| l.contains(&e)) {
                    st.last = None;
                }
            }
        }
        if node.reads_var(root) {
            if let Some(e) = finished {
                st.last = st.copied.contains(&e).then(|| BTreeSet::from([e]));
            } else if !namings_avoid_place(
                self.q.data,
                self.q.stores,
                self.q.def_nr,
                node,
                self.place,
                self.q.disturbed,
            ) {
                st.last = None;
            }
        }
        st
    }

    /// A node that must be walked child by child: it carries a jump, a loop or the exit.
    fn structured(&self, v: &Value) -> bool {
        v.any_node(&mut |n| match n {
            Value::Return(_) | Value::Break(_) | Value::Continue(_) | Value::Loop(_) => true,
            Value::Block(bl) => std::ptr::from_ref(&**bl) as usize == self.q.exit,
            _ => false,
        })
    }

    /// Does `v` name anything the fact is about?
    fn watches(&self, v: &Value) -> bool {
        self.names_local(v)
            || v.reads_var(self.place.0)
            || self.cands.iter().any(|e| v.reads_var(*e))
    }

    fn seq(&mut self, ops: &[Value], mut st: Fresh) -> Fresh {
        for op in ops {
            if !st.live {
                break;
            }
            st = self.walk(op, st);
        }
        st
    }

    fn walk(&mut self, v: &Value, st: Fresh) -> Fresh {
        if !st.live || !self.ok {
            return st;
        }
        match v.unspan() {
            Value::Block(bl) => {
                if std::ptr::from_ref(&**bl) as usize == self.q.exit {
                    let prev = self.at_exit.take().unwrap_or_else(Fresh::dead);
                    self.at_exit = Some(prev.join(st));
                    return Fresh::dead();
                }
                self.seq(&bl.operators, st)
            }
            Value::Insert(ops) => self.seq(ops, st),
            Value::If(cond, then_v, else_v) => {
                let after_cond = self.walk(cond, st);
                let then_end = self.walk(then_v, after_cond.clone());
                let else_end = self.walk(else_v, after_cond);
                then_end.join(else_end)
            }
            Value::Loop(bl) => {
                let mut head = st.clone();
                // The lattice is finite and every round only widens the head, so this ends;
                // the bound is a guard, and running into it declines.
                for _ in 0..64 {
                    self.loops.push((Fresh::dead(), Fresh::dead()));
                    let end = self.seq(&bl.operators, head.clone());
                    let (brk, cont) = self.loops.pop().unwrap_or((Fresh::dead(), Fresh::dead()));
                    let next = st.clone().join(end).join(cont);
                    if next == head {
                        return brk;
                    }
                    head = next;
                }
                self.ok = false;
                Fresh::dead()
            }
            Value::Return(x) => {
                self.walk(x, st);
                Fresh::dead()
            }
            Value::Break(n) | Value::Continue(n) => {
                let brk = matches!(v.unspan(), Value::Break(_));
                let Some(i) = self.loops.len().checked_sub(1 + usize::from(*n)) else {
                    self.ok = false;
                    return Fresh::dead();
                };
                let slot = if brk {
                    &mut self.loops[i].0
                } else {
                    &mut self.loops[i].1
                };
                *slot = std::mem::replace(slot, Fresh::dead()).join(st);
                Fresh::dead()
            }
            Value::Drop(x) => self.walk(x, st),
            Value::Set(w, rhs)
                if self.structured(rhs)
                    || matches!(
                        rhs.unspan(),
                        Value::If(..) | Value::Block(_) | Value::Insert(_)
                    ) =>
            {
                let mut s = self.walk(rhs, st);
                if self.is_local(*w) {
                    s.last = None;
                    s.copied.clear();
                }
                if self.cands.contains(w) {
                    s.copied.remove(w);
                    if s.last.as_ref().is_some_and(|l| l.contains(w)) {
                        s.last = None;
                    }
                }
                s
            }
            other if self.structured(other) => {
                if self.watches(other) {
                    self.ok = false;
                    return Fresh::dead();
                }
                let mut s = st;
                other.for_each_child(&mut |c| {
                    let cur = std::mem::replace(&mut s, Fresh::dead());
                    s = self.walk(c, cur);
                });
                s
            }
            _ => self.event(v, st),
        }
    }
}

/// Is every mention of `buffer` in this exit one the VALUE form accounts for?
///
/// The value form emits the tuple and drops the block, so the question is not what the block
/// does but what the tuple would lose: the allocate-or-reuse guard and the scalar `OpSet*`
/// writes become tuple elements, the one recognised append (`leaf`) becomes the leaf's place,
/// the trailing yield becomes the tuple itself, and the frees the exit owes keep running — a
/// free OF the buffer is nothing once there is no store, and a store-identity test against it
/// is always distinct, which is what makes the free it guards unconditional
/// (`@FR-R-ValueRecord`).  A mention outside those — an element pushed into a field, a record
/// appended through the buffer, a call handed it — is a fill or an effect the tuple cannot
/// carry, so the function keeps its record.
///
/// This is the INSIDE of an `Object` exit; [`retbuf_uses_ok`] asks the same question over the
/// rest of the body and exempts these blocks wholesale.  The two carry the same list of
/// accounted positions on purpose: they are one question over two scopes, and a position
/// added to one belongs in the other.
///
/// The fallback is `false`, and it is a claim worth stating: an unrecognised mention of the
/// buffer is assumed to WRITE something the tuple would not, which costs the rewrite where the
/// mention was harmless and never a value.
fn buffer_uses_accounted(data: &Data, bl: &Block, buffer: u16, leaf: Option<&Value>) -> bool {
    /// Is this node one the value form accounts for, whose OPERANDS still have to be read?
    fn accounted_head(data: &Data, n: &Value, buffer: u16) -> bool {
        let Value::Call(d, args) = n else {
            return false;
        };
        if (*d as usize) >= data.definitions.len() {
            return false;
        }
        if !matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == buffer) {
            return false;
        }
        let name = data.def(*d).name();
        name.starts_with("OpSet")
            || matches!(
                name,
                "OpDatabase"
                    | "OpDatabaseNP"
                    | "OpRefIsNull"
                    | "OpConvBoolFromRef"
                    | "OpFreeRef"
                    | "OpFreeRefIfDistinct"
                    | "OpFreeRefTag"
            )
    }

    /// The buffer as the WITNESS of a store-identity test (argument 1): the tuple is in no
    /// store, so the test answers `true` and the free it guards runs unconditionally.
    fn accounted_witness(data: &Data, n: &Value, buffer: u16) -> bool {
        let Value::Call(d, args) = n else {
            return false;
        };
        (*d as usize) < data.definitions.len()
            && matches!(
                data.def(*d).name(),
                "OpFreeRefIfDistinct" | "OpDistinctStore"
            )
            && matches!(args.get(1).map(Value::unspan), Some(Value::Var(w)) if *w == buffer)
    }

    fn walk(data: &Data, n: &Value, buffer: u16, leaf_addr: Option<usize>, ok: &mut bool) {
        let node = n.unspan();
        if Some(std::ptr::from_ref(node) as usize) == leaf_addr {
            return;
        }
        if matches!(node, Value::Var(w) if *w == buffer) {
            *ok = false;
            return;
        }
        if accounted_head(data, node, buffer) {
            if let Value::Call(_, args) = node {
                for a in args.iter().skip(1) {
                    walk(data, a, buffer, leaf_addr, ok);
                }
            }
            return;
        }
        if accounted_witness(data, node, buffer) {
            if let Value::Call(_, args) = node {
                for (i, a) in args.iter().enumerate() {
                    if i != 1 {
                        walk(data, a, buffer, leaf_addr, ok);
                    }
                }
            }
            return;
        }
        node.for_each_child(&mut |c| walk(data, c, buffer, leaf_addr, ok));
    }

    let leaf_addr = leaf.map(|v| std::ptr::from_ref(v.unspan()) as usize);
    let mut ok = true;
    for (i, op) in bl.operators.iter().enumerate() {
        // The block's last statement yields the buffer, and an explicit `return buf` is the
        // same yield: the tuple replaces both.
        let yields = matches!(op.unspan(), Value::Var(w) if *w == buffer)
            || matches!(op.unspan(), Value::Return(x) if matches!(x.unspan(), Value::Var(w) if *w == buffer));
        if yields && i + 1 == bl.operators.len() {
            continue;
        }
        walk(data, op, buffer, leaf_addr, &mut ok);
        if !ok {
            return false;
        }
    }
    ok
}

/// Every `src` an `OpAppendVector` in `op` copies into `buffer`'s field at `off` — the
/// record form's deep copy of a heap field, which is what a view leaf replaces.
fn appends_into<'a>(
    data: &Data,
    op: &'a Value,
    buffer: u16,
    off: i64,
) -> Vec<(&'a Value, &'a Value)> {
    let mut out = Vec::new();
    op.any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpAppendVector"
            && let [dst, src, ..] = &args[..]
            && let Value::Call(g, gargs) = dst.unspan()
            && (*g as usize) < data.definitions.len()
            && data.def(*g).name() == "OpGetField"
            && matches!(gargs.first().map(Value::unspan), Some(Value::Var(w)) if *w == buffer)
            && matches!(gargs.get(1).map(Value::unspan), Some(Value::Int(o)) if i64::from(*o) == off)
        {
            out.push((n, src));
        }
        false
    });
    out
}

/// The PARAMETER PLACE a leaf expression roots in — the container whose growth or removal
/// would move the record the leaf's field slot sits in, named as
/// [`crate::scopes::ParamPlace`] so the site gate can ask the disturbance producers about
/// it directly.
///
/// The chain it resolves is one step of indirection: a parameter's vector field, an element
/// of it, and a field of that element.  A local on the way is followed through its single
/// assignment — an element read (`sc.ops[i]?`) or a mint into the container
/// (`OpNewRecord(sc, tp, off)`, the element an append is building).
///
/// `None` is the decline, and it is the answer for everything else: a deeper chain, a local
/// with more than one assignment, a place rooted in a store the frame owns.  A frame-owned
/// root is the case that MUST decline — the view would name a store freed at the return —
/// and it is also the common one, which is why the resolution is positive: a place is a
/// leaf root only when the walk reaches a parameter.
fn leaf_root(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    e: &Value,
    depth: u8,
) -> Option<crate::scopes::ParamPlace> {
    // A bound, not a shape rule: the chain a real discharge spells is already five steps
    // (`o.opts` → `o` → its `ncc` block → the block's `if` → the element read), and the
    // bound is what keeps a local whose assignment mentions itself from recursing forever.
    if depth > 12 {
        return None;
    }
    let def = data.def(def_nr);
    let vars = def.variables();
    match e.unspan() {
        Value::Var(v) => {
            if vars.is_argument(*v) {
                return Some((*v, crate::use_analysis::ANY_FIELD));
            }
            // A local that OWNS what it holds is not a route to a parameter's place, even
            // where its assignment names one: a materialised view (`(B-View)`'s copy, which
            // is what a container grown under the binding produces) holds a COPY of the
            // element, and a leaf naming the container would answer the container's own
            // element instead of the copy the record form answers.  The ownership oracle
            // (`@FR-O-Oracle`) is the one home for that question.
            // A local whose ownership is a JOIN owns what it holds on at least one path,
            // and that path is the frame's: a `?`-DISCHARGE of an element mints the absent
            // arm's record in a store the frame frees at the return, so a leaf naming the
            // container would be a view of that store wherever the element was absent.
            // The oracle (`@FR-O-Oracle`) is the one home for the question; the rest of the
            // chain is decided by SHAPE below, which is what keeps a mint of the frame's
            // own store out: no arm of this walk resolves an `OpDatabase`.
            if matches!(
                crate::use_analysis::ownership_of(data, def_nr, e),
                crate::use_analysis::Own::Join { .. }
            ) {
                return None;
            }
            let mut assigned: Option<&Value> = None;
            let mut many = false;
            def.code().any_node(&mut |n| {
                if let Value::Set(w, rhs) = n
                    && w == v
                    && !matches!(rhs.unspan(), Value::Null)
                {
                    if assigned.is_some() {
                        many = true;
                        return true;
                    }
                    assigned = Some(rhs);
                }
                false
            });
            if many {
                return None;
            }
            leaf_root(data, stores, def_nr, assigned?, depth + 1)
        }
        Value::Block(bl) => leaf_root(data, stores, def_nr, bl.operators.last()?, depth + 1),
        Value::If(_, then_v, _) => leaf_root(data, stores, def_nr, then_v, depth + 1),
        Value::Insert(ops) => leaf_root(data, stores, def_nr, ops.last()?, depth + 1),
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            match name {
                // A field of a parameter, or a field of something rooted at one: the FIELD
                // is the place when its base is the parameter itself, and the base's place
                // otherwise (a field of an element does not name a container of its own).
                "OpGetField" | "OpGetDbRef" => {
                    let base = args.first()?;
                    if let Value::Var(v) = base.unspan()
                        && vars.is_argument(*v)
                        && let Some(Value::Int(off)) = args.get(1).map(Value::unspan)
                        && let Ok(o) = u32::try_from(*off)
                    {
                        return Some((*v, o));
                    }
                    leaf_root(data, stores, def_nr, base, depth + 1)
                }
                // An element read: the place is the CONTAINER it reads from.
                "OpGetVector" | "OpGetVectorNullable" | "OpVectorRef" => {
                    leaf_root(data, stores, def_nr, args.first()?, depth + 1)
                }
                // The element an append is building, minted in the container the last
                // argument names by field NUMBER — which a place carries as a byte OFFSET, so
                // the schema makes the conversion (`Stores::field_position`, the same one
                // `scopes::grown_containers` makes for the same reason).  Read as an offset,
                // the place named field 0's number where the field sits at byte 8, and a
                // removal from the very container the leaf views compared as a different
                // place: the `b8` cell measured it, reading the NEXT element's points.
                "OpNewRecord" | "OpNewRecordNP" => {
                    let Value::Var(v) = args.first()?.unspan() else {
                        return None;
                    };
                    if !vars.is_argument(*v) {
                        return None;
                    }
                    let Some(Value::Int(fld)) = args.get(2).map(Value::unspan) else {
                        return None;
                    };
                    if *fld == i32::from(u16::MAX) {
                        return Some((*v, crate::use_analysis::ANY_FIELD));
                    }
                    let parent = data.type_def_nr(vars.tp(*v).base());
                    if parent == u32::MAX {
                        return None;
                    }
                    let off = stores
                        .field_position(data.def(parent).known_type(), u16::try_from(*fld).ok()?);
                    (off != u16::MAX).then(|| (*v, u32::from(off)))
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// @PLN164 C5 — the view-leaf PLAN of one admitted body: per heap field, the parameter
/// places its exits deliver a view of, and per exit the leaf itself.  A field an exit writes
/// not at all contributes no place (a null view can never be stale); a field two exits view
/// through different places contributes both, and a site must keep every one of them
/// undisturbed.
///
/// `None` declines the function, and it is the answer whenever an exit's leaf cannot be
/// PROVEN ([`leaf_source`]) — which includes every body that could disturb the place between
/// the append the view names and the exit, since that is a path on which the fact fails.
pub struct ViewPlan {
    /// Field byte offset → the places its exits view.  Empty set: every exit is a null view.
    pub roots: HashMap<i64, HashSet<crate::scopes::ParamPlace>>,
    /// `(exit Object block address, field byte offset)` → what that exit delivers.  The
    /// emitter reads this and nothing else, so it writes what the gate proved.
    pub leaves: HashMap<(usize, i64), LeafSource>,
}

fn view_leaf_plan(
    data: &Data,
    stores: &Stores,
    d_nr: u32,
    fields: &[(i64, &'static str)],
    leaves: &ValueLeaves,
    record: u16,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> Option<ViewPlan> {
    let mut plan = ViewPlan {
        roots: HashMap::new(),
        leaves: HashMap::new(),
    };
    let views: Vec<i64> = fields
        .iter()
        .filter(|(_, rt)| is_view_part(rt))
        .map(|(off, _)| *off)
        .collect();
    if views.is_empty() {
        return Some(plan);
    }
    let def = data.def(d_nr);
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_VALUEREC").is_ok();
    // The exits this plan is about: the `Object` builds of this record the value form turns
    // into tuples.
    let mut exits: Vec<&Block> = Vec::new();
    body.any_node(&mut |n| {
        let Value::Block(bl) = n else { return false };
        if bl.name != "Object" {
            return false;
        }
        let leaf = leaves
            .objects
            .contains(&(std::ptr::from_ref(&**bl) as usize));
        if !leaf || plain_record_type(data, &bl.result) != Some(record) {
            if trace {
                eprintln!(
                    "[viewleaf] {}: an Object skipped (leaf={leaf}, tp={:?} want {record})",
                    def.name(),
                    plain_record_type(data, &bl.result)
                );
            }
            return false;
        }
        exits.push(bl);
        false
    });
    let exit_bufs: HashSet<u16> = exits
        .iter()
        .filter_map(|bl| bl.result.depend().first().copied())
        .collect();
    for bl in &exits {
        for off in &views {
            let Some(src) = leaf_source(data, stores, d_nr, bl, *off, &exit_bufs, disturbed) else {
                if trace {
                    eprintln!(
                        "[viewleaf] {}: +{off} has no leaf proven on every path",
                        def.name()
                    );
                }
                return None;
            };
            let entry = plan.roots.entry(*off).or_default();
            if let Some(place) = src.root(data, stores, d_nr) {
                entry.insert(place);
            } else if src != LeafSource::Null {
                return None;
            }
            if trace {
                eprintln!("[viewleaf] {}: +{off} {src:?}", def.name());
            }
            plan.leaves
                .insert((std::ptr::from_ref(*bl) as usize, *off), src);
        }
    }
    if plan.roots.len() != views.len() {
        if trace {
            eprintln!(
                "[viewleaf] {}: {} exit(s), {} of {} view field(s) resolved",
                def.name(),
                exits.len(),
                plan.roots.len(),
                views.len()
            );
        }
        return None;
    }
    Some(plan)
}

/// Does every naming of `root` in this statement leave the watched place alone?  The question
/// a mention rule cannot answer on its own, asked per naming SITE.
///
/// Four shapes name a container, and each says what it reaches:
///
/// * `OpPlaceRecord(root, tp)` — a record claimed in the store, which moves nothing: a
///   `DbRef` is logical, so growing the store's arena leaves every record where it is.  This
///   is the naming @PLN164 B2 puts in every function that places a call's result.
/// * `OpNewRecord(root, tp, fld)` and its `OpFinishRecord(root, elem, tp, fld)` half — an
///   element append, naming its field by NUMBER, which the schema turns into the byte offset a
///   place carries (the same conversion `scopes::grown_containers` makes, and for the same
///   reason).  A sibling field is harmless: claiming a new array for `sc.unparsed` moves
///   nothing in `sc.ops`.
/// * `OpGetField(root, off, _)` — a projection, whose offset IS the place.
/// * an ARGUMENT of a user call, licensed when the call's own disturbance summary
///   (`disturbed`, closed over the call graph) does not reach the watched place.
///
/// Anything else that names the root — a bare `Var` somewhere else, a `Set`, a hand-off this
/// walk was not told about — answers `false`.  That is the rule's direction: a naming whose
/// reach cannot be resolved is treated as reaching the watched place, because a missed
/// disturbance here is a read through an element that moved.
fn namings_avoid_place(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    stmt: &Value,
    watch: crate::scopes::ParamPlace,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> bool {
    let (root, watch_off) = watch;
    let vars = data.def(def_nr).variables();
    let parent = data.type_def_nr(vars.tp(root).base());
    let field_off = |fld: i32| -> Option<u32> {
        if fld == i32::from(u16::MAX) {
            return None;
        }
        let fld = u16::try_from(fld).ok()?;
        if parent == u32::MAX {
            return None;
        }
        let off = stores.field_position(data.def(parent).known_type(), fld);
        (off != u16::MAX).then(|| u32::from(off))
    };
    struct Cx<'a> {
        data: &'a Data,
        root: u16,
        watch: crate::scopes::ParamPlace,
        disturbed: Option<&'a crate::scopes::DisturbedParams>,
        ok: bool,
    }
    fn walk(cx: &mut Cx, n: &Value, field_off: &dyn Fn(i32) -> Option<u32>) {
        let node = n.unspan();
        if let Value::Call(d, args) = node
            && (*d as usize) < cx.data.definitions.len()
        {
            let def = cx.data.def(*d);
            let name = def.name();
            let first_is_root =
                matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == cx.root);
            // A user call: every argument that names the root is licensed when the callee's
            // own summary does not reach the watched place.
            if name.starts_with("n_") && !matches!(def.code(), Value::Null) {
                let places = cx.disturbed.and_then(|m| m.get(d));
                for (i, a) in args.iter().enumerate() {
                    if matches!(a.unspan(), Value::Var(w) if *w == cx.root) {
                        let reaches = places.is_some_and(|p| {
                            p.keys().any(|&(slot, inner)| {
                                usize::from(slot) == i
                                    && crate::scopes::same_place(cx.watch, (cx.root, inner))
                            })
                        });
                        if reaches || cx.disturbed.is_none() {
                            cx.ok = false;
                            return;
                        }
                    } else {
                        walk(cx, a, field_off);
                    }
                }
                return;
            }
            // An argument that PROJECTS the watched field is harmless where the call can only
            // read it: `len(sc.ops)` between a bind and its read moves nothing, and declining
            // it declined every cell that checks the container beside the result.
            let mut read_args: HashSet<usize> = HashSet::new();
            for (i, a) in args.iter().enumerate() {
                if read_context(cx.data, *d, i)
                    && let Value::Call(g, gargs) = a.unspan()
                    && (*g as usize) < cx.data.definitions.len()
                    && matches!(cx.data.def(*g).name(), "OpGetField" | "OpGetDbRef")
                    && matches!(gargs.first().map(Value::unspan), Some(Value::Var(w)) if *w == cx.root)
                {
                    read_args.insert(i);
                }
            }
            if !read_args.is_empty() {
                for (i, a) in args.iter().enumerate() {
                    if !read_args.contains(&i) {
                        walk(cx, a, field_off);
                    }
                }
                return;
            }
            if first_is_root {
                let reach = match name {
                    "OpPlaceRecord" => Some(None),
                    // The append's two halves name the container the same way and differ
                    // only in WHERE the field number sits: the mint takes it third, the
                    // finish fourth (after the element it publishes).
                    "OpNewRecord" | "OpNewRecordNP" => match args.get(2).map(Value::unspan) {
                        Some(Value::Int(fld)) => Some(Some(field_off(*fld).unwrap_or(u32::MAX))),
                        _ => None,
                    },
                    "OpFinishRecord" => match args.get(3).map(Value::unspan) {
                        Some(Value::Int(fld)) => Some(Some(field_off(*fld).unwrap_or(u32::MAX))),
                        _ => None,
                    },
                    "OpGetField" | "OpGetDbRef" => match args.get(1).map(Value::unspan) {
                        Some(Value::Int(off)) => {
                            Some(Some(u32::try_from(*off).unwrap_or(u32::MAX)))
                        }
                        _ => None,
                    },
                    // A fixed-width scalar read or write through an address it is GIVEN:
                    // `IN_PLACE_SET_OPS` and `SCALAR_GETTERS` are the two lists that already
                    // carry that property (`@FR-R-InPlace`), each documented as moving
                    // nothing, so a naming through one reaches its own field and no other.
                    // `s.seen = s.seen + 1` beside a view of `s.ops` is the shape.
                    n if IN_PLACE_SET_OPS.contains(&n) || SCALAR_GETTERS.contains(&n) => {
                        match args.get(1).map(Value::unspan) {
                            Some(Value::Int(off)) => {
                                Some(Some(u32::try_from(*off).unwrap_or(u32::MAX)))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };
                match reach {
                    Some(None) => {}
                    Some(Some(off))
                        if off != cx.watch.1
                            && off != u32::MAX
                            && cx.watch.1 != crate::use_analysis::ANY_FIELD => {}
                    _ => {
                        cx.ok = false;
                        return;
                    }
                }
                for a in args.iter().skip(1) {
                    walk(cx, a, field_off);
                }
                return;
            }
        }
        if node.names_var_here(cx.root) {
            cx.ok = false;
            return;
        }
        node.for_each_child(&mut |c| walk(cx, c, field_off));
    }
    let mut cx = Cx {
        data,
        root,
        watch: (root, watch_off),
        disturbed,
        ok: true,
    };
    walk(&mut cx, stmt, &field_off);
    cx.ok
}

/// What a walk over one function knows while it classifies value shapes.
struct ShapeCtx<'a> {
    data: &'a Data,
    def_nr: u32,
    /// The functions admitted so far.
    admitted: &'a HashSet<u32>,
    /// This function's value locals so far ([`value_locals_in`]).
    locals: &'a HashMap<u16, u32>,
    /// `Some(def_nr)` when THIS function is admitted: then an `Object` build of its own
    /// record and a borrowed VIEW of one are value leaves as well.
    own: Option<u32>,
}

/// The return-buffer variable of `own`, when it is admitted and has one — the PHANTOM
/// parameter the value form drops from the signature.  It may still be ASSIGNED in the
/// body: the parser's `return f(…)` lowering hands the callee this buffer and returns it
/// (`rb = f(…, rb); …; return rb`, the `one_buffer_chain` block), and a local the parser
/// PROMOTED into the buffer (`n = f(…); if !n.ok { … }; n`) is this variable under the
/// local's own name.  Both are the phantom bound from a value shape, so both are it as a
/// VALUE LOCAL ([`value_locals_in`]): the tuple the callee answered, read where the record
/// was.
fn own_retbuf(data: &Data, own: Option<u32>) -> Option<u16> {
    let def = data.def(own?);
    let a = ret_buffer_attr(def)?;
    let v = def.variables().var(&def.attributes()[a].name);
    (v != u16::MAX).then_some(v)
}

/// An `Object` block that RETURNS the record it builds — `{ OpDatabase(p); OpSet*(p, …);
/// frees…; return p }`, `p` the buffer the block's result names.  The scope pass lowers an
/// explicit `return S{…}` this way when the frees the return owes (a live local of another
/// record type) go inside the block.  Answers `p`.  In the value form the block is `return
/// (tuple)`, with those frees between the tuple's evaluation and the return, so a statement
/// that releases a real store still runs, in its order.  ONE home: the gate reads the shape
/// here, the leaf walk records it, the emitter converts by it.
#[must_use]
pub fn object_own_return(bl: &Block) -> Option<u16> {
    if bl.name != "Object" {
        return None;
    }
    let [p] = bl.result.depend()[..] else {
        return None;
    };
    match bl.operators.last()?.unspan() {
        Value::Return(x) if matches!(x.unspan(), Value::Var(w) if *w == p) => Some(p),
        _ => None,
    }
}

/// The record type an admitted body's own leaves must carry.
fn own_record(c: &ShapeCtx) -> Option<u16> {
    plain_record_type(c.data, c.data.def(c.own?).returned())
}

/// Is `v` a VALUE SHAPE — every result position a value LEAF — and if so, which admitted
/// function's tuple does it carry?  The leaves: a call to an admitted function (forwards
/// its tuple), a value local (already a tuple), and — inside an admitted body only — an
/// `Object` build of the body's own record (the tuple of its writes) or a borrowed VIEW of
/// that record (a tuple of its field reads; a view is never freed, so reading it is all the
/// value form owes).  `Block`, `If`, `Insert` and `Return` carry the question to their
/// result positions.
///
/// The fallback is `None` because every shape not named here delivers a RECORD — a bare
/// call to a buffer-returning function, an OWNED local (whose store the record form hands
/// up and the value form would have to mint per call, which is slower than the buffer it
/// replaces: `own`'s promoted parameter), a null, a copy — and a caller that reads a tuple
/// off a record cannot compile.
fn value_shape(node: &Value, ctx: &ShapeCtx) -> Option<u32> {
    match node.unspan() {
        Value::Call(callee, _) if ctx.admitted.contains(callee) => Some(*callee),
        Value::Block(bl) if bl.name == "Object" => {
            let own = ctx.own?;
            let rec = own_record(ctx)?;
            (plain_record_type(ctx.data, &bl.result) == Some(rec)).then_some(own)
        }
        Value::Block(bl) => {
            if matches!(bl.result.base(), Type::Void) {
                return None;
            }
            value_shape(bl.operators.last()?, ctx)
        }
        Value::If(_, then_v, else_v) => {
            let callee = value_shape(then_v, ctx)?;
            value_shape(else_v, ctx).map(|_| callee)
        }
        Value::Insert(ops) => value_shape(ops.last()?, ctx),
        Value::Return(x) => value_shape(x, ctx),
        Value::Var(var) => {
            if let Some(callee) = ctx.locals.get(var) {
                return Some(*callee);
            }
            let own = ctx.own?;
            let rec = own_record(ctx)?;
            let vars = ctx.data.def(ctx.def_nr).variables();
            // A compiler `__lift_` temp OWNS what it is bound to (`scopes::new_lift_var`):
            // its whole-record bind from a view MINTS a store and copies into it, and the
            // record form hands that store UP as the result (the join local it feeds is a
            // view of it, so nothing in the frame frees it).  The oracle reads the
            // un-minted bind as Borrowed of its source — the one answer a leaf must not
            // rest on: read as a view, the store the lowering mints is nobody's, one
            // leaked record per call (t15).  A lift is a leaf only as a VALUE LOCAL, above.
            if vars.name(*var).starts_with("__lift_") {
                return None;
            }
            let tp = vars.tp(*var);
            // A VIEW by the ownership oracle (`@FR-O-Oracle`), not by the dep list: then_v
            // `__ret_N` typed `ref(P)["then_v"]` holds the parameter's store on one arm and then_v
            // minted default on the other (`Own::Join`), and reading it as then_v tuple would
            // leave that mint nobody's.  Only then_v value the frame never owns is read as one.
            (plain_record_type(ctx.data, tp) == Some(rec)
                && matches!(
                    crate::use_analysis::ownership_of(ctx.data, ctx.def_nr, node),
                    crate::use_analysis::Own::Borrowed { .. }
                ))
            .then_some(own)
        }
        _ => None,
    }
}

/// The leaf NODES of a function's value positions, by address — what the emitter
/// converts: a `Var` that is a view (the tuple of its getters) and an `Object` block (the
/// tuple of its writes).  Any other `Object` in the body — one bound to a local that is
/// not a value local, one whose result is dropped — stays a record build.
#[derive(Default)]
pub struct ValueLeaves {
    /// `Var` leaves that are views (never value locals), by the node's address and its
    /// unspanned address, which is the identity `output_code_inner` keys on.
    pub views: HashSet<usize>,
    /// `Object` leaves, by the `Block`'s address (the `Box`'s content, which is what the
    /// block emitter is handed).
    pub objects: HashSet<usize>,
    /// EVERY `Var` node at a value position, view or value local, by both addresses — the
    /// whole-value reads a tuple serves, which is what [`local_uses_ok`] accounts a value
    /// local's read at the tail of a branch arm against.
    pub reads: HashSet<usize>,
}

fn collect_leaves(body: &Value, locals: &HashMap<u16, u32>, own: bool) -> ValueLeaves {
    let mut out = ValueLeaves::default();
    fn leaves(v: &Value, locals: &HashMap<u16, u32>, out: &mut ValueLeaves) {
        match v.unspan() {
            Value::Var(w) => {
                out.reads.insert(std::ptr::from_ref(v) as usize);
                out.reads.insert(std::ptr::from_ref(v.unspan()) as usize);
                if !locals.contains_key(w) {
                    out.views.insert(std::ptr::from_ref(v) as usize);
                    out.views.insert(std::ptr::from_ref(v.unspan()) as usize);
                }
            }
            Value::Block(bl) if bl.name == "Object" => {
                out.objects.insert(std::ptr::from_ref(&**bl) as usize);
            }
            Value::Block(bl) => {
                if let Some(l) = bl.operators.last() {
                    leaves(l, locals, out);
                }
            }
            Value::If(_, a, b) => {
                leaves(a, locals, out);
                leaves(b, locals, out);
            }
            Value::Insert(ops) => {
                if let Some(l) = ops.last() {
                    leaves(l, locals, out);
                }
            }
            Value::Return(x) => leaves(x, locals, out),
            _ => {}
        }
    }
    // The walk meets a block before its statements, so an `Object` that returns the
    // record it builds ([`object_own_return`]) is recorded before its own `return` is
    // reached — that return reads the block's buffer, which is no leaf.
    let mut own_returns: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        match n {
            Value::Block(bl) if own && object_own_return(bl).is_some() => {
                out.objects.insert(std::ptr::from_ref(&**bl) as usize);
                if let Some(last) = bl.operators.last() {
                    own_returns.insert(std::ptr::from_ref(last.unspan()) as usize);
                }
            }
            Value::Return(x) if own && !own_returns.contains(&(std::ptr::from_ref(n) as usize)) => {
                leaves(x, locals, &mut out);
            }
            Value::Set(w, rhs) if locals.contains_key(w) => leaves(rhs, locals, &mut out),
            _ => {}
        }
        false
    });
    if own {
        leaves(body, locals, &mut out);
    }
    out
}

/// Does every RESULT position of `d_nr`'s body carry a value leaf ([`value_shape`]), and
/// is its return buffer mentioned only where the value form can drop the mention?  The
/// buffer becomes a PHANTOM — the parameter is gone from the signature — so a body may
/// name it only inside a converted `Object` block, as the buffer argument of a call that
/// drops it, or as the subject of a free.  `None` admits; `Some` names the refusing test
/// for the `LOFT_TRACE_VALUEREC` line.
fn value_body(
    data: &Data,
    d_nr: u32,
    admitted: &HashSet<u32>,
    view_offs: &ViewOffsets,
) -> Option<&'static str> {
    let def = data.def(d_nr);
    let locals = value_locals_in(data, d_nr, admitted, view_offs);
    let c = ShapeCtx {
        data,
        def_nr: d_nr,
        admitted,
        locals: &locals,
        own: Some(d_nr),
    };
    let body = def.code();
    if value_shape(body, &c).is_none() {
        return Some("the tail is not a value leaf");
    }
    // An `Object` that returns the record it builds is the leaf; its own `return`, the
    // block's last statement, is not asked again below.
    let mut ok = true;
    let mut own_returns: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && object_own_return(bl).is_some()
        {
            if value_shape(n, &c).is_none() {
                ok = false;
                return true;
            }
            if let Some(last) = bl.operators.last() {
                own_returns.insert(std::ptr::from_ref(last.unspan()) as usize);
            }
        }
        false
    });
    if !ok {
        return Some("an object's return is not a value leaf");
    }
    body.any_node(&mut |n| {
        if let Value::Return(x) = n
            && !own_returns.contains(&(std::ptr::from_ref(n) as usize))
            && value_shape(x, &c).is_none()
        {
            ok = false;
            return true;
        }
        false
    });
    if !ok {
        return Some("a return is not a value leaf");
    }
    // No return buffer: nothing left to account for, admitted.
    let rb = own_retbuf(data, Some(d_nr))?;
    // A phantom that is itself a value local ([`own_retbuf`]) has every mention
    // accounted by [`local_uses_ok`] already.
    if locals.contains_key(&rb) {
        return None;
    }
    let leaves = collect_leaves(body, &locals, true);
    (!retbuf_uses_ok(body, rb, &c, &leaves.objects)).then_some("the return buffer is used")
}

/// Every mention of the return buffer `rb` in `v` is one the value form drops: inside a
/// converted `Object` block, as the buffer argument of an admitted callee, or as the
/// subject of a free.  Anything else — a write through it, a read of it, a rebind — needs
/// the parameter the value form no longer has.
fn retbuf_uses_ok(v: &Value, rb: u16, c: &ShapeCtx, objects: &HashSet<usize>) -> bool {
    match v.unspan() {
        Value::Var(w) => *w != rb,
        Value::Set(w, _) if *w == rb => false,
        Value::Block(bl) if objects.contains(&(std::ptr::from_ref(&**bl) as usize)) => true,
        Value::Call(d, args) => {
            let callee = c.data.def(*d);
            let is_free = matches!(callee.name(), "OpFreeRef" | "OpFreeRefIfDistinct");
            // As the WITNESS of a store-identity test the phantom is in no store, so the
            // emitter answers the test `true` and makes the guarded free unconditional
            // (`OpFreeRefIfDistinctEmitter`, `OpDistinctStoreEmitter`).
            let witnessed = matches!(callee.name(), "OpFreeRefIfDistinct" | "OpDistinctStore");
            // `@FR-O-Buffer` — the promoted buffer's ENTRY WITNESS snapshots it
            // (`OpRefAlias`) and every free of it is guarded by `OpDistinctStore(buffer,
            // witness)`.  A phantom has no store to snapshot: the alias emits the null
            // reference and the test answers `true` (`OpRefAliasEmitter`,
            // `OpDistinctStoreEmitter`), so both are accounted for.
            let snapshot = matches!(callee.name(), "OpRefAlias" | "OpDistinctStore");
            let dropped = if c.admitted.contains(d) {
                ret_buffer_attr(callee)
            } else {
                None
            };
            args.iter().enumerate().all(|(i, a)| {
                if matches!(a.unspan(), Value::Var(w) if *w == rb) {
                    (is_free && i == 0)
                        || (witnessed && i == 1)
                        || (snapshot && i == 0)
                        || dropped == Some(i)
                } else {
                    retbuf_uses_ok(a, rb, c, objects)
                }
            })
        }
        other => {
            let mut ok = true;
            other.for_each_child(&mut |ch| {
                if ok && !retbuf_uses_ok(ch, rb, c, objects) {
                    ok = false;
                }
            });
            ok
        }
    }
}

/// Which locals of `def_nr` hold a value-returned record — every non-null assignment a
/// value shape, every use one the tuple serves ([`local_uses_ok`]), never a parameter
/// (except the PHANTOM return buffer of an admitted body, [`own_retbuf`]) and never a
/// compiler `__lift_` temp bound from a CALL — mapped to the function whose tuple they
/// carry.  ONE home: the gate decides admission over it and the emitter types the locals
/// from it.
///
/// A `__lift_` temp bound from a call is excluded because that set lowering emits its own
/// displacement guard, reading `.store_nr` off the value — a use no IR walk can see,
/// because it is not an IR node.  A lift bound from a bare `Var` — the copy of a
/// parameter's view a selecting arm returns — has no such lowering: its bind is the
/// whole-record copy arm, which a value local turns into the tuple of the view's reads,
/// and a lift read as a VIEW leaf instead leaks the store that copy mints (t15).  A
/// fixpoint, because a leaf may name another value local.
#[must_use]
pub fn value_locals_in(
    data: &Data,
    def_nr: u32,
    admitted: &HashSet<u32>,
    view_offs: &ViewOffsets,
) -> HashMap<u16, u32> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let own = admitted.contains(&def_nr).then_some(def_nr);
    let mut locals: HashMap<u16, u32> = HashMap::new();
    // A GENERATOR's locals persist as fields of its coroutine struct, typed `DbRef` by the
    // factory; a tuple has no such field, so a generator binds no value local.
    if body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return locals;
    }
    // A `__lift_` temp with an assignment that is not a bare `Var` keeps its buffer (see
    // above); the set is fixed for the body, so it is read once.
    let mut call_bound: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, rhs) = n
            && !matches!(rhs.unspan(), Value::Null | Value::Var(_))
        {
            call_bound.insert(*v);
        }
        false
    });
    let rb = own_retbuf(data, own);
    // Fixed for the body: the reads whose value is dropped.
    let dropped = dropped_reads(body);
    let eligible = |v: u16| {
        (!vars.is_argument(v) || Some(v) == rb)
            && (!vars.name(v).starts_with("__lift_") || !call_bound.contains(&v))
    };
    // Per local, GIVEN a locals set: the tuple its assignments carry, or `None` once ANY
    // assignment is not a value shape (the declaration's `null` aside).
    let shapes_given = |locals: &HashMap<u16, u32>| -> HashMap<u16, Option<u32>> {
        let c = ShapeCtx {
            data,
            def_nr,
            admitted,
            locals,
            own,
        };
        let mut shapes: HashMap<u16, Option<u32>> = HashMap::new();
        body.any_node(&mut |n| {
            if let Value::Set(v, rhs) = n
                && !matches!(rhs.unspan(), Value::Null)
            {
                let s = value_shape(rhs, &c);
                shapes
                    .entry(*v)
                    .and_modify(|e| {
                        if s.is_none() {
                            *e = None;
                        }
                    })
                    .or_insert(s);
            }
            false
        });
        shapes
    };
    let joined = |locals: &HashMap<u16, u32>, cands: &HashMap<u16, u32>| -> HashMap<u16, u32> {
        let mut with = locals.clone();
        with.extend(cands.iter().map(|(v, d)| (*v, *d)));
        with
    };
    loop {
        // GROW: this round's candidates, admitted optimistically.  A candidate's shape may
        // rest on ANOTHER candidate — the join local of a selecting branch reads the lift
        // each arm binds, and the lift's read stands at that join's right — so a step over
        // `locals` alone reaches neither.  Every candidate still traces to a source outside
        // the set: the first pass admits from `locals`, views and admitted calls only, and
        // each later pass only from what the pass before found.
        let mut cands: HashMap<u16, u32> = HashMap::new();
        loop {
            let mut grown = cands.clone();
            for (v, s) in shapes_given(&joined(&locals, &cands)) {
                if let Some(d) = s
                    && !locals.contains_key(&v)
                    && eligible(v)
                {
                    grown.insert(v, d);
                }
            }
            if grown.len() == cands.len() {
                break;
            }
            cands = grown;
        }
        // PRUNE to a consistent set: a candidate whose uses the tuple cannot serve, or
        // whose shape rested on a candidate just pruned, leaves — until nothing else does.
        loop {
            let with = joined(&locals, &cands);
            let shapes = shapes_given(&with);
            let leaves = collect_leaves(body, &with, own.is_some());
            let mut served = leaves.reads;
            served.extend(dropped.iter().copied());
            let kept: HashMap<u16, u32> = cands
                .keys()
                .filter_map(|v| {
                    let d = shapes.get(v).copied().flatten()?;
                    let empty = HashSet::new();
                    let offs = view_offs.get(&d).unwrap_or(&empty);
                    let phantom = (Some(*v) == rb).then_some(&leaves.objects);
                    local_uses_ok(body, *v, data, &served, admitted, offs, phantom)
                        .then_some((*v, d))
                })
                .collect();
            let stable = kept.len() == cands.len();
            cands = kept;
            if stable {
                break;
            }
        }
        if cands.is_empty() {
            return locals;
        }
        locals.extend(cands);
    }
}

/// The `Var` reads in `body` whose value is DROPPED — the tail of a statement block, of a
/// loop body, of a `Drop` — by both addresses.  Such a read of a tuple is served by doing
/// nothing, which is what the emitter's plain read of the local does.  The parser's
/// `return f(…)` chain ends in one when the scope pass leaves its `return` outside the
/// block.  Positional, so a read under a node this walk does not know counts as USED —
/// the conservative side, which only ever costs the optimisation.
fn dropped_reads(body: &Value) -> HashSet<usize> {
    fn walk(v: &Value, used: bool, out: &mut HashSet<usize>) {
        match v.unspan() {
            Value::Var(_) => {
                if !used {
                    out.insert(std::ptr::from_ref(v) as usize);
                    out.insert(std::ptr::from_ref(v.unspan()) as usize);
                }
            }
            Value::Block(bl) => {
                let n = bl.operators.len();
                let tail_used = used && !matches!(bl.result.base(), Type::Void);
                for (i, op) in bl.operators.iter().enumerate() {
                    walk(op, tail_used && i + 1 == n, out);
                }
            }
            Value::Loop(bl) => {
                for op in &bl.operators {
                    walk(op, false, out);
                }
            }
            Value::If(c, a, b) => {
                walk(c, true, out);
                walk(a, used, out);
                walk(b, used, out);
            }
            Value::Insert(ops) => {
                let n = ops.len();
                for (i, op) in ops.iter().enumerate() {
                    walk(op, used && i + 1 == n, out);
                }
            }
            Value::Drop(x) => walk(x, false, out),
            other => other.for_each_child(&mut |ch| walk(ch, true, out)),
        }
    }
    let mut out = HashSet::new();
    walk(body, true, &mut out);
    out
}

/// Is every use of local `v` in `body` one a TUPLE can serve?  A scalar field read at a
/// constant offset (the tuple index), a free of it (nothing to release), a store-identity
/// test or a free guarded by one against it (the tuple is in no store, so always
/// distinct), a copy FROM it (the tuple is materialised into the destination), the buffer
/// ARGUMENT of an admitted callee (the argument the call site drops — how the phantom
/// reaches the callee of a `return f(…)` chain), and a whole-value read the tuple SERVES
/// (`served`: one at a VALUE POSITION — a return tail of an admitted body, the right of a
/// value local, the tail of a branch arm either stands at — consumed as the tuple; or one
/// whose value is dropped, [`dropped_reads`]).  For the PHANTOM return buffer (`phantom`
/// carries the body's converted `Object` leaves), every mention inside such a leaf is
/// dropped with the block: a literal exit built into a buffer a chain also binds
/// (@PLN164 B2) writes it there, exactly as [`retbuf_uses_ok`] accounts for a phantom that
/// is no local.  Any other use — an argument, an append, a copy INTO it, a whole-value
/// read anywhere else — needs the record, so the local keeps its buffer.
fn local_uses_ok(
    body: &Value,
    v: u16,
    data: &Data,
    served: &HashSet<usize>,
    admitted: &HashSet<u32>,
    view_offs: &HashSet<i64>,
    phantom: Option<&HashSet<usize>>,
) -> bool {
    let no_objects = HashSet::new();
    let objects = phantom.unwrap_or(&no_objects);
    // @PLN164 C5 — the view-field reads this local's uses may be accounted against, read
    // once: which of them stand where the field can only be READ.
    let view_reads = view_field_reads(body, v, data, view_offs);
    // Every node inside a converted `Object`, by address — the walk below is transparent
    // to `Span`, and so is this one.
    let mut in_object: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && objects.contains(&(std::ptr::from_ref(&**bl) as usize))
        {
            n.any_node(&mut |m| {
                in_object.insert(std::ptr::from_ref(m) as usize);
                false
            });
        }
        false
    });
    let mut mentions = 0u32;
    let mut accounted = 0u32;
    body.any_node(&mut |n| {
        let inside = in_object.contains(&(std::ptr::from_ref(n) as usize));
        match n {
            Value::Var(w) if *w == v => {
                mentions += 1;
                if inside || served.contains(&(std::ptr::from_ref(n) as usize)) {
                    accounted += 1;
                }
            }
            // Its `Var` operands are accounted above; no rule below may count them twice.
            Value::Call(..) if inside => {}
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let name = data.def(*d).name();
                let arg_is_v = |i: usize| {
                    matches!(args.get(i).map(Value::unspan), Some(Value::Var(w)) if *w == v)
                };
                if admitted.contains(d)
                    && let Some(a) = ret_buffer_attr(data.def(*d))
                    && arg_is_v(a)
                {
                    accounted += 1;
                }
                // A SCALAR FIELD READ at a constant offset — what the value path turns
                // into a tuple index.  (`OpGetField` is the COLLECTION-field spelling; a
                // record's scalar field reads through its typed getter.)
                if arg_is_v(0)
                    && VALUE_RECORD_GETTERS.contains(&name)
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Int(_)))
                {
                    accounted += 1;
                }
                // @PLN164 C5 — a VIEW-LEAF field read that stands in a read-only context
                // ([`view_field_reads`]): the tuple hands over the same reference the
                // record's field slot would, so the read is served.
                if name == "OpGetField" && arg_is_v(0) && view_reads.contains(&(std::ptr::from_ref(n) as usize)) {
                    accounted += 1;
                }
                // A copy FROM the local MATERIALISES the tuple into the destination, one
                // typed write per field — which a VIEW LEAF has no write for: its element
                // is a reference, and materialising it would be a vector copy the value
                // path does not emit.  So a view-leaf record declines a copy site
                // (@PLN164 C5); the record form stands there.
                let copies = name == "OpCopyRecord" && !view_offs.is_empty();
                if arg_is_v(0)
                    && !copies
                    && matches!(
                        name,
                        "OpFreeRef" | "OpFreeRefIfDistinct" | "OpCopyRecord" | "OpDistinctStore"
                    )
                {
                    accounted += 1;
                }
                if arg_is_v(1) && matches!(name, "OpFreeRefIfDistinct" | "OpDistinctStore") {
                    accounted += 1;
                }
                // `@FR-O-Buffer` — the phantom buffer's entry witness snapshots it; a phantom
                // is in no store, so the snapshot is the null reference (`OpRefAliasEmitter`).
                if phantom.is_some() && arg_is_v(0) && name == "OpRefAlias" {
                    accounted += 1;
                }
            }
            _ => {}
        }
        false
    });
    mentions == accounted
}

/// @PLN164 C5 — the SITE condition of a view leaf, per `(O-ViewField)`: the functions
/// `caller` declines because the place their leaf views cannot be shown to survive to the
/// last read of it.
///
/// Three things must hold at a site, and each has a cell:
///
/// * every read of the leaf field is a READ ([`view_field_reads`] has already accounted
///   that, or the local would not be a value local at all);
/// * every read stands in the statement list that carries the BIND, at or after it — so
///   the bind dominates the reads and a stale value from a previous loop iteration is
///   never read (a read before the bind in a loop body would be exactly that);
/// * no statement from the bind to the last read DISTURBS the place, in this frame or
///   through anything it calls ([`crate::scopes::places_disturbed_by`]).
///
/// A place the call's argument does not resolve to declines the function: the site cannot
/// then say what it must keep undisturbed.  A null-view leaf (an exit that writes the field
/// not at all) contributes no place and asks nothing of the site.
fn view_sites_declined(
    data: &Data,
    stores: &Stores,
    caller: u32,
    views: &HashMap<u32, ViewPlan>,
    locals: &HashMap<u16, u32>,
    view_offs: &ViewOffsets,
    disturbed: Option<&crate::scopes::DisturbedParams>,
) -> HashSet<u32> {
    let mut out = HashSet::new();
    let def = data.def(caller);
    let body = def.code();
    let mut lists: Vec<&Vec<Value>> = Vec::new();
    collect_lists(body, &mut lists);
    for list in lists {
        for (i, stmt) in list.iter().enumerate() {
            let Value::Set(v, rhs) = stmt.unspan() else {
                continue;
            };
            let Value::Call(d, args) = rhs.unspan() else {
                continue;
            };
            let Some(plan) = views.get(d) else { continue };
            if locals.get(v) != Some(d) {
                continue;
            }
            let Some(offs) = view_offs.get(d) else {
                continue;
            };
            let reads = view_field_reads(body, *v, data, offs);
            let trace = std::env::var("LOFT_TRACE_VALUEREC").is_ok();
            // Every read inside a statement of THIS list, at or after the bind.  A read the
            // walk cannot place declines the function rather than being assumed later.
            let mut last = i;
            let mut placed: HashSet<usize> = HashSet::new();
            for (j, other) in list.iter().enumerate() {
                let mut hits = 0usize;
                other.any_node(&mut |n| {
                    let a = std::ptr::from_ref(n) as usize;
                    if reads.contains(&a) {
                        placed.insert(a);
                        hits += 1;
                    }
                    false
                });
                if hits == 0 {
                    continue;
                }
                if j < i {
                    if trace {
                        eprintln!(
                            "[viewleaf] {}: a read of `{}` stands before its bind",
                            data.def(caller).name(),
                            def.variables().name(*v)
                        );
                    }
                    out.insert(*d);
                }
                last = last.max(j);
            }
            if placed.len() < reads.len() {
                if trace {
                    eprintln!(
                        "[viewleaf] {}: {} of {} read(s) of `{}` are outside the bind's list",
                        data.def(caller).name(),
                        reads.len() - placed.len(),
                        reads.len(),
                        def.variables().name(*v)
                    );
                }
                out.insert(*d);
            }
            // The places the leaf views, mapped onto the arguments this site passed.
            let mut watch: Vec<crate::scopes::ParamPlace> = Vec::new();
            for place in plan.roots.values().flatten() {
                let Some(arg) = args.get(usize::from(place.0)) else {
                    out.insert(*d);
                    continue;
                };
                if let Some(mapped) = crate::scopes::call_arg_place(arg, data)
                    .and_then(|base| crate::scopes::compose_param_place(base, place.1))
                {
                    watch.push(mapped);
                } else {
                    if trace {
                        eprintln!(
                            "[viewleaf] {}: the argument at slot {} names no place",
                            data.def(caller).name(),
                            place.0
                        );
                    }
                    out.insert(*d);
                }
            }
            if trace {
                eprintln!(
                    "[viewleaf] {}: site of `{}` at {i}, last read {last}, {} read(s), watch {watch:?}",
                    data.def(caller).name(),
                    def.variables().name(*v),
                    reads.len()
                );
            }
            if watch.is_empty() {
                continue;
            }
            // The BIND statement is excluded, and that is the whole reason the span is a
            // span: the call itself grows the container — the append that makes the element
            // the leaf names — and the callee's own half of that question is
            // [`fresh_leaf`].  Anything else in the bind statement runs BEFORE the
            // call, since a call's arguments are evaluated first, so no disturbance in it
            // can reach the view.
            for stmt in list.iter().take(last + 1).skip(i + 1) {
                if !watch
                    .iter()
                    .all(|w| namings_avoid_place(data, stores, caller, stmt, *w, disturbed))
                {
                    if trace {
                        eprintln!(
                            "[viewleaf] {}: the place `{}` views can be disturbed before its last read",
                            data.def(caller).name(),
                            def.variables().name(*v)
                        );
                    }
                    out.insert(*d);
                }
            }
        }
    }
    out
}

/// Every STATEMENT LIST of a body — what an ordering question is asked over, because a
/// statement's position only means something inside the list that carries it.
fn collect_lists<'a>(v: &'a Value, out: &mut Vec<&'a Vec<Value>>) {
    match v.unspan() {
        Value::Block(bl) | Value::Loop(bl) => {
            out.push(&bl.operators);
            for op in &bl.operators {
                collect_lists(op, out);
            }
        }
        Value::Insert(ops) => {
            out.push(ops);
            for op in ops {
                collect_lists(op, out);
            }
        }
        other => other.for_each_child(&mut |c| collect_lists(c, out)),
    }
}

/// Where a node stands, for the site gate: an admitted call is consumed as a TUPLE at a
/// result position of an admitted body (`Tail`), on the right of a value local (`Bound`),
/// or as a statement whose result is dropped (`Discard`); anywhere else (`Operand`) the
/// consumer wants a record.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Pos {
    Tail,
    Bound,
    Discard,
    Operand,
}

/// The SITE gate: every admitted call in `v` that stands at an `Operand` position — an
/// argument, a field value, a return of a non-admitted function, the right of a local
/// that is not a value local — is declined into `declined`.  Result positions carry the
/// caller's position down; everything else is an operand.
fn site_walk(node: &Value, pos: Pos, ctx: &ShapeCtx, declined: &mut HashSet<u32>) {
    match node.unspan() {
        Value::Call(callee, args) => {
            // The insert runs whenever the site declines; only the trace is conditional.
            if pos == Pos::Operand
                && ctx.admitted.contains(callee)
                && declined.insert(*callee)
                && std::env::var("LOFT_TRACE_VALUEREC").is_ok()
            {
                eprintln!(
                    "[valuerec] {}: its record is consumed in {}",
                    ctx.data.def(*callee).name(),
                    ctx.data.def(ctx.def_nr).name()
                );
            }
            for arg in args {
                site_walk(arg, Pos::Operand, ctx, declined);
            }
        }
        Value::CallRef(_, args) | Value::Tuple(args) => {
            for arg in args {
                site_walk(arg, Pos::Operand, ctx, declined);
            }
        }
        Value::Set(var, rhs) => {
            // @PLN164 E-1 — a forward writes the tuple into the buffer it was handed, which
            // is a position the value form serves: only the call's operands are sites.
            if forward_site(ctx.data, ctx.def_nr, *var, rhs, ctx.admitted, ctx.locals).is_some()
                && let Value::Call(_, args) = rhs.unspan()
            {
                for arg in args {
                    site_walk(arg, Pos::Operand, ctx, declined);
                }
                return;
            }
            let pos_here = if ctx.locals.contains_key(var) {
                Pos::Bound
            } else {
                Pos::Operand
            };
            site_walk(rhs, pos_here, ctx, declined);
        }
        Value::Block(bl) => {
            let count = bl.operators.len();
            for (idx, op) in bl.operators.iter().enumerate() {
                let pos_here = if idx + 1 == count && !matches!(bl.result.base(), Type::Void) {
                    pos
                } else {
                    Pos::Discard
                };
                site_walk(op, pos_here, ctx, declined);
            }
        }
        Value::Insert(ops) => {
            let count = ops.len();
            for (idx, op) in ops.iter().enumerate() {
                let pos_here = if idx + 1 == count { pos } else { Pos::Discard };
                site_walk(op, pos_here, ctx, declined);
            }
        }
        Value::If(test, then_v, else_v) => {
            site_walk(test, Pos::Operand, ctx, declined);
            site_walk(then_v, pos, ctx, declined);
            site_walk(else_v, pos, ctx, declined);
        }
        Value::Return(inner) => {
            let pos_here = if ctx.own.is_some() {
                Pos::Tail
            } else {
                Pos::Operand
            };
            site_walk(inner, pos_here, ctx, declined);
        }
        Value::Loop(bl) => {
            for op in &bl.operators {
                site_walk(op, Pos::Discard, ctx, declined);
            }
        }
        // A `par` arm'step result crosses the parallel machinery, which spells its own
        // buffered call (`n_make_pair(cell, elm, _pd1)`): then_v record, never then_v tuple.
        Value::Parallel(arms) => {
            for arm in arms {
                site_walk(arm, Pos::Operand, ctx, declined);
            }
        }
        Value::Drop(inner) => site_walk(inner, Pos::Discard, ctx, declined),
        Value::Iter(_, then_v, else_v, step) => {
            site_walk(then_v, Pos::Operand, ctx, declined);
            site_walk(else_v, Pos::Operand, ctx, declined);
            site_walk(step, Pos::Operand, ctx, declined);
        }
        Value::TuplePut(_, _, inner) | Value::Yield(inner) => {
            site_walk(inner, Pos::Operand, ctx, declined)
        }
        _ => {}
    }
}

/// The DEAD BUFFERS of `def_nr` (@PLN157 § V-ah, `@FR-R-ValueRecord`): a local minted by
/// `OpDatabase` whose every mention the value form drops — the buffer argument of an
/// admitted callee (dropped from the call), the subject of a free or of the pool's
/// release-on-reuse (`OpClear`), or an operand of a store-identity test whose other operand
/// is a value local (answered `true` without a read).  Such a local was a § V-af join buffer for a branch that now binds a tuple: the
/// store it minted per activation served nothing, so the mint and the frees are emitted
/// as nothing.  A buffer with any other mention — a witness read against a RECORD local,
/// an argument to a callee that keeps its buffer — is minted as before.
#[must_use]
pub fn dead_buffers(data: &Data, def_nr: u32, vr: &ValueRecords) -> HashSet<u16> {
    let mut out = HashSet::new();
    if vr.fns.is_empty() {
        return out;
    }
    let admitted: HashSet<u32> = vr.fns.keys().copied().collect();
    let locals = value_locals_in(data, def_nr, &admitted, &vr.view_offs);
    // A forward's buffer is WRITTEN by the site (`forward_site`), so its argument is a use.
    let forwards = forward_sites(data, def_nr, &admitted, &locals);
    let def = data.def(def_nr);
    let vars = def.variables();
    let mut minted: HashSet<u16> = HashSet::new();
    let mut mentions: HashMap<u16, u32> = HashMap::new();
    let mut dropped: HashMap<u16, u32> = HashMap::new();
    def.code().any_node(&mut |n| {
        match n {
            Value::Var(w) => *mentions.entry(*w).or_insert(0) += 1,
            Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
                let arg_var = |i: usize| match args.get(i).map(Value::unspan) {
                    Some(Value::Var(w)) => Some(*w),
                    _ => None,
                };
                let callee = data.def(*d);
                match callee.name() {
                    "OpDatabase" | "OpDatabaseNP" => {
                        if let Some(w) = arg_var(0) {
                            minted.insert(w);
                            *dropped.entry(w).or_insert(0) += 1;
                        }
                    }
                    // The pool's release of a reused buffer (loft#1549): a buffer never
                    // minted holds nothing to release (`OpClearEmitter`).
                    "OpClear" => {
                        if let Some(w) = arg_var(0) {
                            *dropped.entry(w).or_insert(0) += 1;
                        }
                    }
                    "OpFreeRef" | "OpFreeRefIfDistinct" => {
                        if let Some(w) = arg_var(0) {
                            *dropped.entry(w).or_insert(0) += 1;
                        }
                        // A free guarded FOR a value local is emitted as nothing
                        // (`OpFreeRefIfDistinctEmitter`), and its witness goes with it.
                        if callee.name() == "OpFreeRefIfDistinct"
                            && arg_var(0).is_some_and(|o| locals.contains_key(&o))
                            && let Some(w) = arg_var(1)
                        {
                            *dropped.entry(w).or_insert(0) += 1;
                        }
                    }
                    // `@FR-O-LazyBuffer` — the null test in front of a lazy buffer's mint
                    // is the mint's own guard, and goes with it.
                    "OpRefIsNull" => {
                        if let Some(w) = arg_var(0)
                            && vars.is_lazy_buffer(w)
                        {
                            *dropped.entry(w).or_insert(0) += 1;
                        }
                    }
                    "OpDistinctStore" => {
                        for (i, other) in [(0, 1), (1, 0)] {
                            if let Some(w) = arg_var(i)
                                && arg_var(other).is_some_and(|o| locals.contains_key(&o))
                            {
                                *dropped.entry(w).or_insert(0) += 1;
                            }
                        }
                    }
                    _ if admitted.contains(d)
                        && !forwards.contains_key(&(args.as_ptr() as usize)) =>
                    {
                        if let Some(idx) = ret_buffer_attr(callee)
                            && let Some(w) = arg_var(idx)
                        {
                            *dropped.entry(w).or_insert(0) += 1;
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
        false
    });
    for w in minted {
        if !vars.is_argument(w)
            && !locals.contains_key(&w)
            && mentions.get(&w).copied().unwrap_or(0) == dropped.get(&w).copied().unwrap_or(0)
        {
            out.insert(w);
        }
    }
    out
}

/// The value leaves of `def_nr` for the emitter ([`ValueLeaves`]): empty for a function
/// that is not admitted, since only an admitted body converts a view or an `Object`.
#[must_use]
pub fn value_leaves(data: &Data, def_nr: u32, vr: &ValueRecords) -> ValueLeaves {
    if !vr.fns.contains_key(&def_nr) {
        return ValueLeaves::default();
    }
    let admitted: HashSet<u32> = vr.fns.keys().copied().collect();
    let locals = value_locals_in(data, def_nr, &admitted, &vr.view_offs);
    collect_leaves(data.def(def_nr).code(), &locals, true)
}

/// @PLN164 C5 — per admitted function, the byte offsets of its VIEW-LEAF fields.  A call
/// site reads such a field off the tuple's own reference, so the read is accounted where a
/// record's collection-field read would decline the local.
pub type ViewOffsets = HashMap<u32, HashSet<i64>>;

/// Is argument `i` of a call to `def` a pure READ of the collection it is given — a value the
/// callee cannot write through?
///
/// Two contexts qualify, and they are the ones a reader actually writes: an argument at a
/// `const` parameter (`len(v)`, `totp(v)` — `value_const` is the signature fact C124 carries,
/// and the `const` of a stdlib `len(self: const vector)` is the same fact), and the operand of
/// an op in [`VIEW_LEAF_READ_OPS`], which answers a VALUE rather than a place.  ONE home: the
/// site gate asks it of a leaf read and the disturbance walk asks it of a naming that reaches
/// the watched field, and the two must agree about what a read is.
fn read_context(data: &Data, d: u32, i: usize) -> bool {
    if (d as usize) >= data.definitions.len() {
        return false;
    }
    let def = data.def(d);
    if VIEW_LEAF_READ_OPS.contains(&def.name()) {
        return i == 0;
    }
    def.attributes().get(i).is_some_and(|a| a.value_const)
}

/// The view-field READS of local `v` in `body`, by node address — every
/// `OpGetField(v, off, _)` at one of `offs` that stands where the value can only be READ.
///
/// It is an ALLOW-list, and that direction is the point (`@FR-O-ViewField`'s site
/// condition): a read hands over the container's own vector, so anything that could WRITE
/// through it — an append, a clear, a keyed write, a by-value parameter the callee writes
/// into — must decline the function, and a use this walk does not recognise is not
/// admitted.  A missed read costs the rewrite; a missed WRITE would cost the program its
/// meaning, because the write would land in a container the caller never named.
///
/// The two recognised contexts are the ones a reader actually writes: an argument at a
/// `const` parameter (`len(m.pts)`, `totp(m.pts)` — `value_const` is the signature fact C124
/// carries, and the `const` of a stdlib `len(self: const vector)` is the same fact), and the
/// operand of an op in [`VIEW_LEAF_READ_OPS`], which answers a VALUE and not a place.
///
/// An ELEMENT read is deliberately not one of them, and that is the measurement this list is
/// cut from: `m.pts[0]?.px = 100` reads the leaf with `OpGetVectorNullable` — an op that
/// only reads its container — and then WRITES through the element it answered.  An op being
/// read-only says nothing about what its RESULT is used for, so admitting one that answers a
/// place admits the write with it (measured: the site wrote 100 into the caller's own copy in
/// the record form and into `s.ops[0].opts` through the leaf).
fn view_field_reads(body: &Value, v: u16, data: &Data, offs: &HashSet<i64>) -> HashSet<usize> {
    let mut out = HashSet::new();
    if offs.is_empty() {
        return out;
    }
    let field_of = |arg: &Value| -> Option<i64> {
        let Value::Call(g, gargs) = arg.unspan() else {
            return None;
        };
        if (*g as usize) >= data.definitions.len() || data.def(*g).name() != "OpGetField" {
            return None;
        }
        if !matches!(gargs.first().map(Value::unspan), Some(Value::Var(w)) if *w == v) {
            return None;
        }
        match gargs.get(1).map(Value::unspan) {
            Some(Value::Int(off)) if offs.contains(&i64::from(*off)) => Some(i64::from(*off)),
            _ => None,
        }
    };
    body.any_node(&mut |n| {
        let Value::Call(d, args) = n else {
            return false;
        };
        if (*d as usize) >= data.definitions.len() {
            return false;
        }
        for (i, arg) in args.iter().enumerate() {
            if field_of(arg).is_none() {
                continue;
            }
            if read_context(data, *d, i) {
                // The UNSPANNED address only: every IR walk here visits a spanned node's
                // content, so that is the identity a use is recognised by, and one address
                // per node is what makes the site gate able to COUNT them.
                out.insert(std::ptr::from_ref(arg.unspan()) as usize);
            }
        }
        false
    });
    out
}

pub const VALUE_RECORD_GETTERS: [&str; 6] = [
    "OpGetFloat",
    "OpGetInt",
    "OpGetSingle",
    "OpGetBoolean",
    "OpGetByte",
    "OpGetShort",
];

// ── @PLN157 § V-ao (`@FR-R-Invariant`) — an invariant integer chain is evaluated once ──

/// The integer ops an invariant chain may be built from: each one's `#rust` template is a
/// pure, store-free `ops::` call over its operands, so evaluating the chain at its first
/// use and answering the memo after is exactly the per-iteration evaluation — the same
/// value on every path, the overflow note (`ops::note_integer_overflow`) fired where the
/// first evaluation stood.  A shift, a division and a remainder are NOT admitted: their
/// templates raise through `stores` on a bad amount or a zero divisor, and a memo's
/// evaluation stands inside expressions that already borrow `stores` (E0502).  The
/// `Nullable` twins of the three arithmetic ops are the same pure calls.
const INVARIANT_OPS: [(&str, usize); 10] = [
    ("OpAddInt", 2),
    ("OpMinInt", 2),
    ("OpMulInt", 2),
    ("OpMinSingleInt", 1),
    ("OpLandInt", 2),
    ("OpLorInt", 2),
    ("OpEorInt", 2),
    ("OpAddIntNullable", 2),
    ("OpMinIntNullable", 2),
    ("OpMulIntNullable", 2),
];

/// One chain a loop memoises: every node address in the loop that spells it (a body may
/// spell one chain more than once, and each spelling is registered under its `Span`
/// wrapper's address and its own), the chain itself — the first spelling, cloned, which the
/// memo's first-use evaluation is emitted from — its op count and its spelling count.
pub struct InvariantChain {
    pub nodes: Vec<usize>,
    pub chain: Value,
    pub ops: usize,
    /// How many times the loop spells the chain.
    pub spellings: usize,
}

/// Is `v` a chain of [`INVARIANT_OPS`] over literals and variables the loop neither
/// rebinds nor lets escape?  Answers `(ops, vars)` — the op count and the variable-leaf
/// count; a bare leaf is `(0, _)`, and a chain over literals alone is left to the constant
/// folder, which answers it for free where a memo would cost a test.
fn arith_chain(v: &Value, data: &Data, banned: &HashSet<u16>) -> Option<(usize, usize)> {
    match v.unspan() {
        Value::Int(_) | Value::Long(_) => Some((0, 0)),
        Value::Var(x) => (!banned.contains(x)).then_some((0, 1)),
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            let arity = INVARIANT_OPS.iter().find(|(n, _)| *n == name)?.1;
            if args.len() != arity {
                return None;
            }
            let (mut ops, mut vars) = (1, 0);
            for a in args {
                let (o, r) = arith_chain(a, data, banned)?;
                ops += o;
                vars += r;
            }
            Some((ops, vars))
        }
        _ => None,
    }
}

/// Structural equality with every `Span` peeled: two spellings of one chain at two source
/// positions are one memo.
fn same_chain(a: &Value, b: &Value) -> bool {
    match (a.unspan(), b.unspan()) {
        (Value::Call(x, xa), Value::Call(y, ya)) => {
            x == y && xa.len() == ya.len() && xa.iter().zip(ya).all(|(p, q)| same_chain(p, q))
        }
        (p, q) => p == q,
    }
}

/// The maximal invariant integer chains of loop `lp`, in first-appearance order, each with
/// every spelling's node addresses.  Enforces `@FR-R-Invariant`: a leaf is a literal or a
/// variable the loop never rebinds (`rebound_vars`, the loop's own counters included) and
/// never lets escape (a bare argument to a by-reference parameter or a fn-ref call, a tuple
/// destination, an iterator variable — `non_sentinel::collect_escapes`, the one home for
/// that question); a body that yields or runs arms in parallel memoises nothing.  A chain
/// belongs to the INNERMOST loop that spells it: a nested loop is not walked, its own pass
/// memoises what it spells, and its memo is declared where its flag is known clear on
/// every entry — the form LLVM peels the first-use test out of (measured: declared one
/// loop out, the flag's state at entry is unknown and the test stays in every tap).
#[must_use]
pub fn invariant_chains(lp: &Block, data: &Data) -> Vec<InvariantChain> {
    if lp
        .operators
        .iter()
        .any(|op| op.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_))))
    {
        return Vec::new();
    }
    let mut banned = rebound_vars(lp);
    for op in &lp.operators {
        super::non_sentinel::collect_escapes(data, op, &mut banned);
    }
    let mut out: Vec<InvariantChain> = Vec::new();
    for op in &lp.operators {
        collect_chains(op, data, &banned, &mut out);
    }
    out
}

/// Pre-order: a node that is a chain is recorded whole (under its wrapper's address and
/// its own) and not descended into; a nested loop is left to its own pass; any other
/// node's children are walked.
fn collect_chains(v: &Value, data: &Data, banned: &HashSet<u16>, out: &mut Vec<InvariantChain>) {
    if matches!(v.unspan(), Value::Loop(_)) {
        return;
    }
    if let Some((ops, vars)) = arith_chain(v, data, banned) {
        if ops == 0 || vars == 0 {
            return;
        }
        let mut addrs = vec![std::ptr::from_ref(v) as usize];
        let inner = v.unspan();
        if !std::ptr::eq(inner, v) {
            addrs.push(std::ptr::from_ref(inner) as usize);
        }
        if let Some(row) = out.iter_mut().find(|c| same_chain(&c.chain, v)) {
            row.nodes.extend(addrs);
            row.spellings += 1;
        } else {
            out.push(InvariantChain {
                nodes: addrs,
                chain: inner.clone(),
                ops,
                spellings: 1,
            });
        }
        return;
    }
    v.for_each_child(&mut |c| collect_chains(c, data, banned, out));
}
