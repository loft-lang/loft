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
use std::collections::{HashMap, HashSet};
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
const READ_ONLY_COLLECTION_OPS: [&str; 23] = [
    // the reference's own identity — `store_nr`/`rec` tests that touch no store at all
    // (@PLN157 § V-c: the R1 guard put `OpRefIsNull` in every buffer-building body)
    "OpRefIsNull",
    "OpConvBoolFromRef",
    "OpDistinctStore",
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
                        if let Some(Value::Var(c)) = args.get(*pf as usize).map(Value::unspan)
                            && !rebound.contains(c)
                            && !out.vectors.iter().any(|(p, _)| p.0 == *c && p.1 == *offs)
                        {
                            out.vectors
                                .push(((*c, offs.clone()), substitute_root(path, *pf, *c)));
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
    if !def.rust().is_empty()
        || def.hidden_return_buffer_attr().is_some()
        || matches!(def.returned().base(), Type::Iterator(_, _))
    {
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
                if let Some(Value::Var(p)) = args.get(*pf as usize).map(Value::unspan)
                    && path_root(*p, offs)
                    && !out.headers.iter().any(|(r, o, _)| *r == *p && o == offs)
                {
                    out.headers
                        .push((*p, offs.clone(), substitute_root(path, *pf, *p)));
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
    let mut out = v.clone();
    out.map_nodes(&mut |n| {
        if matches!(n, Value::Var(x) if *x == from) {
            *n = Value::Var(to);
        }
    });
    out
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
            if in_place_setter || record_free || fusable_push || record_mint || fresh_delivery {
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
    if !data
        .def(record)
        .attributes()
        .iter()
        .all(|a| a.constant || matches!(a.typedef, Type::Routine(_)) || is_scalar(&a.typedef))
    {
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
