// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
// loft#885 — loop-invariant vector headers

//! Which vectors a loop may read through a header derived once, instead of re-deriving
//! it per element (loft#885).
//!
//! Reading `v[i]` resolves the store, loads the container slot and loads the length, and
//! every one of those loads is guarded — so LLVM will not lift them out of the loop even
//! with the whole chain inlined (PERFORMANCE-history.md § Native vs Rust root cause 3c). The
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
//! deny-lists in PERFORMANCE-history.md § Design: P8, where an omission is a silent wrong read.
//! `LOFT_HOIST_VERIFY=1` is the second half: it emits the checking form of every hoisted
//! read, which re-derives the header and panics on a mismatch, so a hole in the allow-list
//! shows up as a failure under one suite run.

use crate::data::{Block, Data, DefType, Type, Value};
use crate::database::Stores;
pub(crate) use crate::ir_shape::split_call;
pub use crate::ir_shape::{WrapperOperand, one_op_wrapper};
use crate::ir_shape::{as_int, as_var, call_named};
use crate::opt::lazy_split::buffer_serves_one_call;
pub use crate::opt::lazy_split::{LazySplit, lazy_splits};
pub use crate::opt::store_text_borrow::{StoreTextBorrows, borrowed_store_texts};
use crate::opt::store_text_borrow::{foreign_store_writer, frame_minted_locals};
// The return-buffer adoption and the literal-group coverage analyses live in `opt`, which
// both writers may name (the C backend reads them too); this module keeps its own names.
pub use crate::opt::ret_adopt::{CompleteWrites, RetAdopt, complete_writes, ret_adopt};
use crate::opt::ret_adopt::{callee_reinits_buffer, kind_of};
// The push and mint window analyses live in `opt` too (the C backend reads them).
pub(crate) use crate::opt::push_window::work_buffer_arg;
pub use crate::opt::push_window::{
    MintLoop, PushLoop, StoreFacts, enum_record_enabled, mint_loop, mint_push_qualifies,
    mint_window_ok, push_loop, push_window_ok,
};
use crate::opt::push_window::{push_loop_body, push_loop_on, simple_invariant, window_parts_ok};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::rc::Rc;

pub use crate::opt::body_writes::{
    ELEMENT_ADDRESS_OPS, WriteCache, WriteSet, is_element_address, sub_record, view_field,
};
pub(crate) use crate::opt::body_writes::{body_writes, link_targets, owned_local, rebound_vars};

pub use crate::opt::writes::{
    FUSABLE_PUSHES, FusedPush, HoistOwned, HoistTiers, MintTarget, PathKey, fused_push,
    keep_range_path, mint_path, mint_target, pre_alloc_path, push_operands, vector_path,
};
pub(crate) use crate::opt::writes::{READ_ONLY_COLLECTION_OPS, plain_record_type};

pub use crate::opt::loop_records::{
    IN_PLACE_SET_OPS, LoopRecord, LoopRecords, SCALAR_GETTERS, loop_records, mint_type_of,
};
pub(crate) use crate::opt::loop_records::{group_covers_type, is_scalar, is_var, refillable_plain};

pub use crate::opt::header::hoistable_vectors;
pub(crate) use crate::opt::header::{body_blocks_hoist, indexed_vectors, vector_candidates};

/// The operators that only COMPARE a reference — its record number, or the whole `DbRef` —
/// and read no store: a view tested against null (`v != null`, or the handle test
/// `OpRefIsNull`) is still the view it was, so a held address survives them (`@FR-R-RecPtr`).
/// An allow-list like the one above.
pub const REF_COMPARE_OPS: [&str; 4] = ["OpEqRef", "OpNeRef", "OpConvBoolFromRef", "OpRefIsNull"];

pub(crate) use crate::opt::writes::{
    blocks_header_hoist, call_writes_store, frees_a_record, in_place_only_writer, is_text_set,
    native_op_is_store_free, null_buffer_alloc, retbuf_only_writer,
};
pub use crate::opt::writes::{in_place_copy, may_write_store};

/// `@FR-R-FlatCopy`'s emitted clause — `OpCopyRecord(src, dst, tp)` that keeps its source
/// (no `COPY_FREE_SOURCE`), from a VARIABLE, of a record type that owns no heap: the copy is
/// the type's bytes, so the emitter writes them (`OpCopyRecordEmitter`), through the two
/// record addresses where the block holds both (`@FR-R-RecPtr`).  A fresh destination
/// (`COPY_FRESH_DEST`, a minted push slot) is no case apart: such a type has no claims to
/// release in it.  The source is never the call whose result the copy binds, because that
/// bind is what consumes an unresolved fn-ref's borrowed-return marker.  Answers the width.
#[must_use]
pub fn flat_copy(stores: &Stores, op: &str, args: &[Value]) -> Option<u16> {
    if op != "OpCopyRecord" || !crate::keys::flat_copy_enabled() {
        return None;
    }
    let [src, _, tp] = args else {
        return None;
    };
    let Value::Int(n) = tp.unspan() else {
        return None;
    };
    if n & i32::from(crate::keys::COPY_FREE_SOURCE) != 0 || !matches!(src.unspan(), Value::Var(_)) {
        return None;
    }
    let known = u16::try_from(n & i32::from(crate::keys::COPY_TP_MASK)).ok()?;
    stores.flat_record_type(known).then(|| stores.size(known))
}

/// The variables `body` rebinds — a `Set` or a `TuplePut` anywhere in it.  A rebind leaves
/// the store untouched and still invalidates anything hoisted off that variable, because the
/// hoist describes what the variable named on the way in.
/// `@FR-R-GuardedChain` — every counted loop nested anywhere in `body`, with the SEED of
/// its stepped counter (the one `Set` to it that is not its own step, found in `body`): the
/// leaves a guard needs to bound that counter over the enclosing loop's extent.  A loop
/// whose seed is set from two places, or not at all in `body`, is left out.
pub fn nested_counted_loops<'a>(
    body: &'a Block,
    data: &Data,
) -> Vec<(RangeCounters<'a>, &'a Value)> {
    let mut out: Vec<(RangeCounters<'a>, &'a Value)> = Vec::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Loop(lp) = n
                && let Ok(rc) = range_counters(lp, data)
            {
                let stepped = rc.next.unwrap_or(rc.index);
                let mut seed: Option<&'a Value> = None;
                let mut seeds = 0usize;
                for o in &body.operators {
                    o.any_node(&mut |m| {
                        if let Value::Set(v, e) = m
                            && *v == stepped
                        {
                            let is_step = matches!(e.unspan(), Value::Call(d, a)
                                if data.def(*d).name() == "OpAddInt"
                                    && a.len() == 2
                                    && matches!(a[0].unspan(), Value::Var(c) if *c == stepped));
                            if !is_step {
                                seeds += 1;
                                seed = Some(e);
                            }
                        }
                        false
                    });
                }
                if seeds == 1
                    && let Some(s) = seed
                {
                    // The end, likewise: a non-literal bound is taken once into a local in
                    // the loop's prelude (`@FR-I-Range`), so an end that is a local set ONCE
                    // in this body is read as the value it was set from.
                    let mut rc = rc;
                    if let Value::Var(h) = rc.hi.unspan() {
                        let mut ends: Vec<&'a Value> = Vec::new();
                        for o in &body.operators {
                            o.any_node(&mut |m| {
                                if let Value::Set(v, e) = m
                                    && v == h
                                {
                                    ends.push(e);
                                }
                                false
                            });
                        }
                        if let [e] = ends[..] {
                            rc.hi = e;
                        }
                    }
                    out.push((rc, s));
                }
            }
            false
        });
    }
    out
}

/// `@FR-I-Range` — the range ends of `def_nr` its loops' preludes bind ONCE from the length of
/// a pure vector path (`for i in 0..len(v)`: `_range_end = len(v)`), with that path.  An end
/// set anywhere else in the function, or from anything but a length, is left out.
#[must_use]
pub fn range_end_lengths(data: &Data, def_nr: u32) -> BTreeMap<u16, PathKey> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let mut sets: BTreeMap<u16, (usize, Option<PathKey>)> = BTreeMap::new();
    def.code().walk(&mut |n| {
        if let Value::Set(v, rhs) = n
            && vars.name(*v).starts_with("_range_end")
        {
            let path = match rhs.unspan() {
                Value::Call(d, a)
                    if a.len() == 1
                        && matches!(data.def(*d).name(), "OpLengthVector" | "t_6vector_len") =>
                {
                    vector_path(data, &a[0])
                }
                _ => None,
            };
            let e = sets.entry(*v).or_insert((0, None));
            e.0 += 1;
            e.1 = path;
        }
    });
    sets.into_iter()
        .filter_map(|(v, (count, path))| (count == 1).then_some((v, path?)))
        .collect()
}

/// The variables `body` rebinds anywhere — a `Set`, a `TuplePut`, an `Iter` binding.
#[must_use]
pub fn written_vars(body: &Block, data: &Data, vars: &crate::variables::Function) -> HashSet<u16> {
    let mut w = rebound_vars(body, data, vars);
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Iter(v, ..) = n {
                w.insert(*v);
            }
            false
        });
    }
    w
}

/// Does statement node `n` rebind `root` — or a variable `root` links to?  The
/// single-variable form of [`rebound_vars`]'s question.
fn rebinds_root(n: &Value, root: u16, vars: &crate::variables::Function) -> bool {
    // `unspan`, as every shape test over a statement must: a rebind the parser wrapped for its
    // source position is the same rebind, and a `Span` that hid it would leave a stale header
    // on the very shape this predicate exists to decline.
    match n.unspan() {
        Value::Set(v, _) | Value::TuplePut(v, _, _) => {
            *v == root || link_targets(vars, root).contains(v)
        }
        _ => false,
    }
}

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

/// `@FR-R-Inputs`' path clause — a callee's scalar input at offset `fld` of its record
/// parameter, when the caller's argument is a SUB-RECORD path (`atlas.a_cv` handed to
/// `get_pixel(self: Canvas, …)`): the hoist key — the root variable and the field's SUMMED
/// offset from the root's first byte, the bytes the read loads — and every `(record type,
/// offset)` a write could reach the field by.  The key alone is not enough to evict on: a
/// write through a view of the sub-record (`c = atlas.a_cv; c.width = …`) is typed by the
/// SUB-record, so the field is stale after a write at the root's type and summed offset or
/// at any sub-record's type and the offset within it.  A bare variable is not a path here
/// (its one pair is the plain key); a chain whose root is no plain struct answers `None`.
#[must_use]
pub fn path_scalar(
    data: &Data,
    vars: &crate::variables::Function,
    arg: &Value,
    fld: i64,
) -> Option<(ScalarKey, Vec<(u16, i64)>)> {
    let mut chain: Vec<(i64, u16)> = Vec::new();
    let mut cur = arg.unspan();
    loop {
        match cur {
            Value::Var(_) => break,
            Value::Call(d, args)
                if args.len() == 3
                    && (*d as usize) < data.definitions.len()
                    && data.def(*d).name() == "OpGetField" =>
            {
                let (Value::Int(off), Value::Int(tp)) = (args[1].unspan(), args[2].unspan()) else {
                    return None;
                };
                chain.push((i64::from(*off), u16::try_from(*tp).ok()?));
                cur = args[0].unspan();
            }
            _ => return None,
        }
    }
    let Value::Var(root) = cur else { return None };
    if chain.is_empty() {
        return None;
    }
    let root_tp = plain_record_type(data, vars.tp(*root))?;
    // `chain` runs from the argument inward to the root; each sub-record's offset of the
    // field is `fld` plus the offsets of the sub-records nested inside it.
    let mut evict: Vec<(u16, i64)> = Vec::with_capacity(chain.len() + 1);
    let mut within = fld;
    for (off, tp) in &chain {
        evict.push((*tp, within));
        within += off;
    }
    evict.push((root_tp, within));
    Some(((*root, within), evict))
}

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
    /// @PLN157 § V-ak (`@FR-R-Base`) — the loop GROWS no store: no push and no mint,
    /// whether or not the mint emits through a push header.
    /// Everything else the admission lets through — in-place sets, store-free ops,
    /// store-free or in-place-only callees, a free, and a null-discharge buffer's mint
    /// (§ V-ad: a FRESH store, or a clear of the buffer's own, which no header names —
    /// see [`lazy_buffer_mint`] for why a new slot moves no live store's memory) —
    /// leaves every store's buffer where it is, so a hoisted header may carry the
    /// address of its vector's element 0 for the loop's whole extent.
    pub growth_free: bool,
    /// `@FR-R-Push`'s sole-growth clause — the loop's ONE store growth is the pushes of its
    /// one pushed path: no other push, no mint, no reservation of another root, no text set
    /// and no rebound mover.  Only that path's own growth step can then move its store's
    /// memory, and the step derives the header again, so the header may hold its record's
    /// address for the loop's extent.
    pub sole_push: bool,
    /// `@FR-R-Base`'s growth clause — the ROOT variable of every push, pre-allocation and
    /// mint in the body, the rebound ones included: the stores the loop grows.  A header
    /// whose store is proven apart from every one of them ([`StoreFacts::distinct`]) keeps
    /// its base under that growth.  Empty exactly when `growth_free`.
    pub movers: Vec<u16>,
    /// `@FR-R-PushRec`'s apart-call clause admitted `mint_pushes` ([`apart_mint`]): the
    /// header grows by the runtime's doubling and the loop takes no up-front reservation,
    /// since its trip count may be a value read from input and its mints conditional.
    pub apart_mint: bool,
}

/// The locals whose null init `(R-Place)` replaced by `OpPlaceRecord`: their record lives
/// in the host's store, never in one of their own.
#[must_use]
pub fn placed_locals(code: &Value, data: &Data) -> HashSet<u16> {
    let mut out = HashSet::new();
    code.any_node(&mut |n| {
        if let Value::Set(v, rhs) = n
            && let Value::Call(d, _) = rhs.unspan()
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpPlaceRecord"
        {
            out.insert(*v);
        }
        false
    });
    out
}

/// The root variable of every push, pre-allocation and record mint in `stmts` — the stores
/// they grow — in first-appearance order.  Exactly the ops [`blocks_header_hoist`] lets
/// through under the push and mint tiers, so a caller that proves every root apart from a
/// store may open those tiers.
pub(crate) fn mover_roots(
    stmts: &[Value],
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    for op in stmts {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
            {
                let name = data.def(*d).name();
                let root = fused_push(data, name, args)
                    .map(|fp| fp.path.0)
                    .or_else(|| pre_alloc_path(data, name, args).map(|p| p.0))
                    .or_else(|| mint_path(data, stores, name, args, vars).map(|p| p.0));
                if let Some(r) = root
                    && !out.contains(&r)
                {
                    out.push(r);
                }
            }
            false
        });
    }
    out
}

/// Can a value of type `t` hold a record of definition `target`, at any depth — through a
/// reference, an enum's payload, a collection's elements, a tuple?  A type this walk does not
/// know answers YES: a miss here would let a loop hold a header something can still move.
pub(crate) fn type_reaches(data: &Data, t: &Type, target: u32, seen: &mut HashSet<u32>) -> bool {
    match t {
        Type::Integer(_)
        | Type::Boolean
        | Type::Float
        | Type::Single
        | Type::Character
        | Type::Text(_)
        | Type::Null
        | Type::Void
        | Type::Never
        | Type::Enum(_, false, _) => false,
        Type::Reference(d, _)
        | Type::Enum(d, true, _)
        | Type::Sorted(d, _, _)
        | Type::Index(d, _, _)
        | Type::Radix(d, _, _)
        | Type::Trie(d, _, _)
        | Type::Hash(d, _, _) => def_reaches(data, *d, target, seen),
        Type::Vector(elm, _) | Type::Optional(elm) | Type::RefVar(elm) | Type::Rewritten(elm) => {
            type_reaches(data, elm, target, seen)
        }
        Type::Tuple(elms) => elms.iter().any(|e| type_reaches(data, e, target, seen)),
        _ => true,
    }
}

fn def_reaches(data: &Data, d: u32, target: u32, seen: &mut HashSet<u32>) -> bool {
    if d == target {
        return true;
    }
    if d as usize >= data.definitions.len() {
        return true;
    }
    if !seen.insert(d) {
        return false;
    }
    data.def(d)
        .attributes()
        .iter()
        .any(|a| type_reaches(data, &a.typedef, target, seen))
        || data
            .children_of(d)
            .into_iter()
            .any(|c| data.def_type(c) == DefType::EnumValue && def_reaches(data, c, target, seen))
}

/// `@FR-R-PushRec`'s apart-call clause — the record-push header a loop the gate DECLINES
/// still holds for the ONE vector it mints into, when nothing else in the body can reach that
/// vector.  The gate declines a loop that calls a writer: a recursive decoder's
/// `sub = read_value(bytes, p, d, buf); if sub.ok { items += [sub.value] }` claims in the
/// very store `items` lives in.  A header is a record number, a length and a capacity — a
/// claim elsewhere in the store moves none of them; only an op that names the vector's owner
/// slot or record can.  So the body is read for MENTIONS: every variable whose type can hold
/// an element of the vector ([`type_reaches`]) must be FRESH this pass — the element a mint
/// answered, the pooled call buffer its pool statement placed or cleared, or the result of a
/// call handed that buffer as its return buffer, none of which holds a record that existed
/// before the pass — and the vector itself appears only as the first operand of its own mint
/// group (`OpPreAllocVector`, `OpNewRecord`, `OpFinishRecord`) or as the host a placement
/// claims beside (`OpPlaceRecord`).  Loft reaches a record only through a value that names
/// it, so no call in such a body can resize the vector or change its length.  The answer is
/// the path and its operand; the loop holds no other header, no base and no scalar under
/// this clause, and takes no reservation ([`LoopHoist::apart_mint`]): the header grows by
/// the runtime's doubling, so a trip count read from input reserves nothing.
fn apart_mint(
    body: &Block,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    tiers: HoistTiers,
) -> Option<(PathKey, Value)> {
    // The push tier too: `LOFT_NO_PUSH_HOIST=1` promises a loop that pushes hoists nothing,
    // and this clause runs exactly where the header gate declined — which that switch makes it do.
    if !(tiers.mint && tiers.record_push && tiers.push && crate::keys::apart_mint_enabled()) {
        return None;
    }
    let vars = data.def(def_nr).variables();
    let mut target: Option<MintTarget> = None;
    let mut ok = true;
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpNewRecord"
            {
                match mint_target(data, stores, "OpNewRecord", args, vars) {
                    Some(t) if mint_push_qualifies(stores, t.vector_tp, tiers.heap_push) => {
                        match &target {
                            Some(tt) if tt.path != t.path => ok = false,
                            Some(_) => {}
                            None => target = Some(t),
                        }
                    }
                    _ => ok = false,
                }
            }
            false
        });
    }
    let target = target.filter(|_| ok)?;
    let p = target.path.0;
    if !target.path.1.is_empty() {
        return None;
    }
    let Type::Vector(element_tp, _) = vars.tp(p).base() else {
        return None;
    };
    let elem = match element_tp.base() {
        Type::Reference(d, _) | Type::Enum(d, true, _) => *d,
        _ => return None,
    };
    let fresh = apart_fresh(body, data, stores, vars, &target.path);
    let reaches =
        |v: u16| v >= vars.count() || type_reaches(data, vars.tp(v), elem, &mut HashSet::new());
    let admitted = apart_mentions_ok(body, data, p, &|v| fresh.contains(&v) || !reaches(v));
    if !admitted {
        if std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
            eprintln!(
                "hoist: {} loop {} holds no apart mint header — a value that can reach {} is not fresh",
                data.def(def_nr).name(),
                body.scope,
                vars.name(p)
            );
        }
        return None;
    }
    crate::rewrite_census::fired("R-PushRec/apart", 1);
    Some((target.path, target.vector))
}

/// [`apart_mint`]'s FRESH variables, to a fixed point (a call's result is fresh only once its
/// buffer is): the elements minted on `path`, a placed record, a pooled call buffer (its pool
/// statement), the result of a call handed a fresh buffer as its return buffer, and a field
/// view into a fresh record (`kv = kd.value`).
fn apart_fresh(
    body: &Block,
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    path: &PathKey,
) -> HashSet<u16> {
    let mut fresh: HashSet<u16> = HashSet::new();
    loop {
        let before = fresh.len();
        for op in &body.operators {
            op.any_node(&mut |n| {
                match n {
                    Value::Set(v, inner) => {
                        if let Value::Call(d, args) = inner.unspan()
                            && (*d as usize) < data.definitions.len()
                        {
                            let name = data.def(*d).name();
                            let view_of_fresh = |a: Option<&Value>| {
                                matches!(a.map(Value::unspan), Some(Value::Var(b)) if fresh.contains(b))
                            };
                            if (name == "OpNewRecord"
                                && mint_path(data, stores, name, args, vars).as_ref() == Some(path))
                                || placed_pool_of(n, data).is_some()
                                || (name == "OpGetField" && view_of_fresh(args.first()))
                                || data
                                    .def(*d)
                                    .hidden_return_buffer_attr()
                                    .is_some_and(|a| view_of_fresh(args.get(a)))
                            {
                                fresh.insert(*v);
                            }
                        }
                    }
                    Value::If(..) => {
                        if let Some((b, _)) = pool_of(n, data).or_else(|| placed_pool_of(n, data)) {
                            fresh.insert(b);
                        }
                    }
                    _ => {}
                }
                false
            });
        }
        if fresh.len() == before {
            return fresh;
        }
    }
}

/// [`apart_mint`]'s mention walk: every variable the body names must be `allowed` (fresh, or of
/// a type that cannot reach the vector) — save the vector `p` itself as the first operand of
/// its own mint group or as a placement's host — and the body runs no `CallRef`, `Parallel` or
/// `Yield`, whose code this walk cannot see.
fn apart_mentions_ok(body: &Block, data: &Data, p: u16, allowed: &dyn Fn(u16) -> bool) -> bool {
    const HOST_OPS: [&str; 4] = [
        "OpPreAllocVector",
        "OpNewRecord",
        "OpFinishRecord",
        "OpPlaceRecord",
    ];
    fn walk(n: &Value, ok: &mut dyn FnMut(&Value) -> Option<bool>) -> bool {
        if let Some(verdict) = ok(n) {
            return verdict;
        }
        let mut all = true;
        n.for_each_child(&mut |c| all &= walk(c, ok));
        all
    }
    fn judge(n: &Value, data: &Data, p: u16, allowed: &dyn Fn(u16) -> bool) -> Option<bool> {
        match n.unspan() {
            Value::Var(v) => Some(allowed(*v)),
            Value::Set(v, _) if !allowed(*v) => Some(false),
            Value::CallRef(..) | Value::Parallel(_) | Value::Yield(_) => Some(false),
            // The vector as its own mint group's operand, or as a placement's host: the
            // remaining operands still walk.
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && HOST_OPS.contains(&data.def(*d).name())
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == p) =>
            {
                Some(
                    args[1..]
                        .iter()
                        .all(|a| walk(a, &mut |m| judge(m, data, p, allowed))),
                )
            }
            _ => None,
        }
    }
    body.operators
        .iter()
        .all(|op| walk(op, &mut |n| judge(n, data, p, allowed)))
}

/// `(R-Place)`'s pool statement — `if OpRefIsNull(b) { b = OpPlaceRecord(host, tp) } else
/// OpClear(b, tp)`, answered as `(b, tp)`: [`pool_of`] with the buffer placed beside a host
/// instead of minted in a store of its own.  Asked of the `If`, or of the placement itself.
fn placed_pool_of(stmt: &Value, data: &Data) -> Option<(u16, u16)> {
    let placed = |s: &Value| -> Option<(u16, u16)> {
        let Value::Set(b, inner) = s.unspan() else {
            return None;
        };
        match call_named(inner, data, "OpPlaceRecord")? {
            [_, Value::Int(tp)] => Some((*b, u16::try_from(*tp).ok()?)),
            _ => None,
        }
    };
    let Value::If(cond, mint_branch, release) = stmt.unspan() else {
        return placed(stmt);
    };
    let buf = match call_named(cond, data, "OpRefIsNull")? {
        [arg] => match arg.unspan() {
            Value::Var(var) => *var,
            _ => return None,
        },
        _ => return None,
    };
    let (b, tp) = placed(only_statement(mint_branch)?)?;
    if b != buf {
        return None;
    }
    match call_named(release, data, "OpClear")? {
        // `@FR-R-Reuse`'s reset clause sets `POOL_RESET` on the release's type.
        [arg, Value::Int(clear_tp)]
            if is_var(arg, buf)
                && i32::from(tp) == *clear_tp & i32::from(crate::keys::POOL_TP_MASK) =>
        {
            Some((buf, tp))
        }
        _ => None,
    }
}

/// `@FR-R-Header`'s apart clause — what a loop the gate DECLINES still holds: the header
/// and the element base of each vector it only reads in a store nothing in it is handed
/// (`apart::apart_headers`).  No push header, no scalar, no callee input: those rest on the
/// gate's verdict about the whole body, and this rests on the one store's mentions alone.
fn apart_hoist(
    body: &Block,
    data: &Data,
    def_nr: u32,
    facts: Option<&StoreFacts>,
    cache: &mut HashMap<u32, bool>,
    owned: Option<&HoistOwned>,
) -> LoopHoist {
    let Some(facts) = facts.filter(|_| crate::keys::apart_headers_enabled()) else {
        return LoopHoist::default();
    };
    let vectors = super::apart::apart_headers(
        body,
        data,
        facts,
        &|t| work_buffer_arg(data, def_nr, t),
        vector_candidates(body, data, def_nr),
        super::apart::Callees {
            cache,
            owned,
            code: data.def(def_nr).code(),
        },
    );
    crate::rewrite_census::fired("R-Header/apart", vectors.len());
    if !vectors.is_empty() && std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
        let vars = data.def(def_nr).variables();
        eprintln!(
            "hoist: {} loop {} keeps {} header(s) apart from what declined it: {:?}",
            data.def(def_nr).name(),
            body.scope,
            vectors.len(),
            vectors
                .iter()
                .map(|(p, _)| vars.name(p.0))
                .collect::<Vec<_>>()
        );
    }
    LoopHoist {
        // The loop grows no store these headers live in, so each carries its base.
        growth_free: !vectors.is_empty(),
        vectors,
        ..LoopHoist::default()
    }
}

/// `@FR-R-Header`'s rebind clause — asked by the C alone, which re-derives the header after
/// every rebind of its root (`FnWriter::refresh_rebound_header`): of the bare vector variables
/// `body` REBINDS, the ones the apart clause would keep were they not rebound.  The store
/// reasoning covers every bind already — a variable's owner is read off its type's
/// dependencies, which name what any of its binds may view — so after a rebind the header is
/// derived again from a vector in the same proven-apart store.  Only a rebind that NAVIGATES
/// (`v = x.f`, `v = w[i].f`: no mint, no call) is admitted; a variable the loop pushes to or
/// writes in place is no candidate (the apart clause keeps read-only vectors).
pub(crate) fn rebound_apart_headers(
    body: &Block,
    data: &Data,
    def_nr: u32,
    facts: Option<&StoreFacts>,
    cache: &mut HashMap<u32, bool>,
    owned: Option<&HoistOwned>,
) -> Vec<(PathKey, Value)> {
    let Some(facts) = facts.filter(|_| crate::keys::apart_headers_enabled()) else {
        return Vec::new();
    };
    if crate::env_once!(std::env::var("LOFT_NO_REBOUND_HEADER").is_ok_and(|v| v != "0")) {
        return Vec::new();
    }
    let vars = data.def(def_nr).variables();
    let rebound = super::body_writes::rebound_vars(body, data, vars);
    let navigates = |v: u16| {
        let mut ok = true;
        for op in &body.operators {
            op.any_node(&mut |n| {
                match n {
                    Value::Set(x, rhs) if *x == v => {
                        ok &= !rhs.any_node(&mut |m| match m {
                            Value::Call(d, _) => !matches!(
                                data.def(*d).name(),
                                "OpGetField" | "OpGetVector" | "OpGetVectorNullable"
                            ),
                            Value::Block(_) | Value::If(..) | Value::Loop(_) => true,
                            _ => false,
                        });
                    }
                    Value::TuplePut(x, _, _) if *x == v => ok = false,
                    _ => {}
                }
                !ok
            });
        }
        ok
    };
    let trace = std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok();
    let candidates: Vec<(PathKey, Value)> = indexed_vectors(body, data, def_nr)
        .into_iter()
        .filter(|(p, _)| {
            let keep = p.1.is_empty() && rebound.contains(&p.0) && navigates(p.0);
            if trace && p.1.is_empty() && rebound.contains(&p.0) {
                eprintln!(
                    "hoist: {} loop {} rebound {}: {}",
                    data.def(def_nr).name(),
                    body.scope,
                    vars.name(p.0),
                    if keep {
                        "a candidate"
                    } else {
                        "rebound by more than navigation"
                    }
                );
            }
            keep
        })
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let kept = super::apart::apart_headers(
        body,
        data,
        facts,
        &|t| work_buffer_arg(data, def_nr, t),
        candidates,
        super::apart::Callees {
            cache,
            owned,
            code: data.def(def_nr).code(),
        },
    );
    if trace {
        eprintln!(
            "hoist: {} loop {} rebind clause keeps {:?}",
            data.def(def_nr).name(),
            body.scope,
            kept.iter().map(|(p, _)| vars.name(p.0)).collect::<Vec<_>>()
        );
    }
    crate::rewrite_census::fired("R-Header/rebind", kept.len());
    kept
}

/// `@FR-R-Header`'s growth half — asked by the C alone, which derives the header again after
/// every statement that grows its vector (`FnWriter::refresh_grown_header`): of the bare
/// vector variables `body` reads through an element address and never rebinds, the ones whose
/// store this activation owns alone and that the loop GROWS only through statements of its own
/// level that `apart::growth_args` recognises (`apart::grown_headers`).  An interpreter's value
/// stack, pushed through a helper and read every op, is the shape (bench 20's `put`).
pub(crate) fn grown_apart_headers(
    body: &Block,
    data: &Data,
    def_nr: u32,
    facts: Option<&StoreFacts>,
    cache: &mut HashMap<u32, bool>,
    owned: Option<&HoistOwned>,
) -> Vec<(PathKey, Value)> {
    let Some(facts) = facts.filter(|_| crate::keys::apart_headers_enabled()) else {
        return Vec::new();
    };
    if crate::env_once!(std::env::var("LOFT_NO_GROWN_HEADER").is_ok_and(|v| v != "0")) {
        return Vec::new();
    }
    let vars = data.def(def_nr).variables();
    let rebound = super::body_writes::rebound_vars(body, data, vars);
    let candidates: Vec<(PathKey, Value)> = indexed_vectors(body, data, def_nr)
        .into_iter()
        .filter(|(p, _)| p.1.is_empty() && !rebound.contains(&p.0))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let kept = super::apart::grown_headers(
        body,
        data,
        facts,
        &|t| work_buffer_arg(data, def_nr, t),
        candidates,
        super::apart::Callees {
            cache,
            owned,
            code: data.def(def_nr).code(),
        },
    );
    if std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
        eprintln!(
            "hoist: {} loop {} growth clause keeps {:?}",
            data.def(def_nr).name(),
            body.scope,
            kept.iter().map(|(p, _)| vars.name(p.0)).collect::<Vec<_>>()
        );
    }
    kept
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
// The nine parameters are the loop, the two things that type it (the IR and the schema),
// the two memos shared across every loop of the program, the tier switches, the § V-p
// memo and the emitter-owned buffer vars; a struct would put a name between each and the
// one call site without removing anything.
#[allow(clippy::too_many_arguments)]
/// Enforces `@FR-R-Scalar` (the candidates and the write set) beside `@FR-R-Header`.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn hoistable(
    body: &Block,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    writes: &mut WriteCache,
    tiers: HoistTiers,
    inputs: Option<&mut InputCache>,
    owned: Option<&HoistOwned>,
    facts: Option<&StoreFacts>,
) -> LoopHoist {
    // `(R-Header)`'s text clause: a loop the gate declines is asked again with its text sets
    // admitted, so a loop that hoisted without the clause hoists exactly as it did.
    let clause = crate::keys::text_set_header_enabled();
    let text_grown = if !body_blocks_hoist(
        body,
        data,
        Some(stores),
        def_nr,
        cache,
        tiers,
        owned,
        !clause,
    ) {
        None
    } else if clause
        && !body_blocks_hoist(
            body,
            data,
            Some(stores),
            def_nr,
            cache,
            HoistTiers {
                text_set: true,
                ..tiers
            },
            owned,
            true,
        )
    {
        Some(text_set_roots(body, data))
    } else {
        let apart = apart_hoist(body, data, def_nr, facts, cache, owned);
        // `@FR-R-PushRec`'s apart-call clause — only where the apart headers hold nothing,
        // so a loop that keeps them keeps exactly them.
        if apart.vectors.is_empty()
            && let Some((path, expr)) = apart_mint(body, data, stores, def_nr, tiers)
        {
            return LoopHoist {
                movers: vec![path.0],
                mint_pushes: vec![(path, expr)],
                apart_mint: true,
                ..LoopHoist::default()
            };
        }
        return apart;
    };
    let mut out = LoopHoist {
        vectors: vector_candidates(body, data, def_nr),
        scalars: Vec::new(),
        pushes: Vec::new(),
        mint_pushes: Vec::new(),
        growth_free: false,
        sole_push: false,
        movers: Vec::new(),
        apart_mint: false,
    };
    let vars = data.def(def_nr).variables();
    let rebound = rebound_vars(body, data, vars);
    // (key, the (record type, offset) pairs a write evicts it by, the call) in
    // first-appearance order.  A bare variable's field has one pair; a field reached through
    // a sub-record path has one per record on the path ([`path_scalar`]).
    let mut found: Vec<(ScalarKey, Vec<(u16, i64)>, Value)> = Vec::new();
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
                    found.push((key, vec![(tp, key.1)], n.clone()));
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
                            let Some(arg) = args.get(*pf as usize) else {
                                continue;
                            };
                            if let Value::Var(c) = arg.unspan()
                                && !rebound.contains(c)
                                && let Some(tp) = plain_record_type(data, vars.tp(*c))
                                && !found.iter().any(|(k, _, _)| *k == (*c, *fld))
                            {
                                found.push((
                                    (*c, *fld),
                                    vec![(tp, *fld)],
                                    substitute_root(getter, *pf, *c),
                                ));
                            } else if let Some((key, evict)) = path_scalar(data, vars, arg, *fld)
                                && !rebound.contains(&key.0)
                                && !found.iter().any(|(k, _, _)| *k == key)
                            {
                                found.push((key, evict, substitute_path(getter, *pf, arg)));
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
        // `@FR-R-Refresh`'s keep-range clause — a popped path holds a push header too (the
        // mutable holder its length refresh writes) and is judged for aliasing as a pushed
        // one: an owned root, or the return buffer, else the loop declines.
        for op in &body.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && let Some(path) = keep_range_path(data, data.def(*d).name(), args)
                    && !pushes.iter().any(|(p, _)| *p == path)
                {
                    pushes.push((path, args[0].clone()));
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
                    && let Some(target) = mint_target(data, stores, "OpNewRecord", args, vars)
                {
                    let q = tiers.record_push
                        && mint_push_qualifies(stores, target.vector_tp, tiers.heap_push);
                    if let Some(row) = mint_fused.iter_mut().find(|(p, _, _)| *p == target.path) {
                        row.2 &= q;
                    } else {
                        mints.push(target.path.clone());
                        mint_fused.push((target.path, target.vector, q));
                    }
                }
                false
            });
        }
    }
    // `(R-Mint)`'s rebound clause — a mover whose root the body REBINDS: its holder would
    // describe what the root named on the way in, so it takes none.  It declines the loop
    // (as every rebound mover did) unless every rebind is alias-free — a null, or the
    // projection of an emitter-owned buffer var (§ V-al's loop buffer, § V-z's element
    // slot: a fresh store or a fresh element's field, which no kept header can name).
    // Its pushes and mints keep their templates — or take a GROUP header (`R-GroupPush`)
    // — and they still GROW a store, so the loop is not growth-free.
    let mut dropped_mover = false;
    {
        let mut movers: Vec<u16> = Vec::new();
        for r in pushes
            .iter()
            .map(|(p, _)| p.0)
            .chain(mints.iter().map(|p| p.0))
        {
            if !movers.contains(&r) {
                movers.push(r);
            }
        }
        for op in &body.operators {
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && let Some(p) = pre_alloc_path(data, data.def(*d).name(), args)
                    && !movers.contains(&p.0)
                {
                    movers.push(p.0);
                }
                false
            });
        }
        out.movers = movers;
    }
    // A loop admitted through its text sets grows the stores they claim in.  It may not also
    // push, mint or reserve: a push header carries what the store's buffer looked like, and
    // the clause is stated for a loop that grows through text alone.  Its bases are kept only
    // for vectors proven apart from every store a text set writes — none when a target's
    // root is unknown.
    if let Some(roots) = &text_grown {
        if !out.movers.is_empty() || !pushes.is_empty() || !mints.is_empty() {
            crate::rewrite_census::declined(
                "R-Header",
                &format!("{}:{}", data.def(def_nr).name(), body.scope),
                "the loop writes a text and also pushes, mints or reserves",
            );
            if std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
                eprintln!(
                    "hoist: {} loop {} declined: it writes a text and also pushes, mints or reserves",
                    data.def(def_nr).name(),
                    body.scope
                );
            }
            return apart_hoist(body, data, def_nr, facts, cache, owned);
        }
        crate::rewrite_census::fired("R-Header/text", 1);
        out.movers = roots.clone().unwrap_or_default();
    }
    if !pushes.is_empty() || !mints.is_empty() {
        let rebound_movers: Vec<u16> = pushes
            .iter()
            .map(|(p, _)| p.0)
            .chain(mints.iter().map(|p| p.0))
            .filter(|r| rebound.contains(r))
            .collect::<HashSet<u16>>()
            .into_iter()
            .collect();
        if !rebound_movers.is_empty() {
            if !tiers.rebound_movers
                || !rebound_movers
                    .iter()
                    .all(|r| rebinds_alias_free(body, *r, data, owned))
            {
                return apart_hoist(body, data, def_nr, facts, cache, owned);
            }
            dropped_mover = true;
            pushes.retain(|(p, _)| !rebound_movers.contains(&p.0));
            mints.retain(|p| !rebound_movers.contains(&p.0));
            mint_fused.retain(|(p, _, _)| !rebound_movers.contains(&p.0));
        }
    }
    if !pushes.is_empty() || !mints.is_empty() {
        let mover = |q: &PathKey| pushes.iter().any(|(p, _)| p == q) || mints.contains(q);
        let retbuf = retbuf_var(data, def_nr);
        let others = out.vectors.iter().any(|(q, _)| !mover(q));
        if pushes.len() + mints.len() > 1 || others {
            if !pushes.iter().map(|(p, _)| p).chain(mints.iter()).all(|p| {
                owned_local(vars, p.0) || Some(p.0) == retbuf || work_buffer_arg(data, def_nr, p.0)
            }) {
                return apart_hoist(body, data, def_nr, facts, cache, owned);
            }
            let any_retbuf = pushes
                .iter()
                .map(|(p, _)| p)
                .chain(mints.iter())
                .any(|p| Some(p.0) == retbuf);
            // `@FR-R-Alias` — and a candidate whose store is proven apart from every
            // store the loop grows ([`StoreFacts::distinct`]: a `for` walk's hidden vector
            // over an owned local, a `&` view of one) cannot name a pushed vector either.
            let movers = &out.movers;
            out.vectors.retain(|(q, _)| {
                mover(q)
                    || owned_local(vars, q.0)
                    || (vars.is_argument(q.0) && !any_retbuf)
                    || facts.is_some_and(|f| movers.iter().all(|m| f.distinct(q.0, *m)))
            });
        }
        out.vectors.retain(|(q, _)| !mover(q));
        crate::rewrite_census::fired("R-Mint", mints.len());
        out.pushes = pushes;
        out.mint_pushes = mint_fused
            .into_iter()
            .filter_map(|(p, expr, q)| q.then_some((p, expr)))
            .collect();
    }
    // `@FR-R-Base` — decided here, before either early return below: a loop with no
    // record scalars to hoist is exactly the shape a pixel loop has.
    let null_buf = mints_null_buffer(body, data, vars);
    // EVERY mint counts, not only the ones that earned a push header: a mint left on its
    // templates (an element that is no struct — a vector of vectors — or a tier switched
    // off) appends through the runtime and grows its store all the same, and a base bound
    // for a sibling field of the same record would address the buffer that growth freed.
    out.growth_free =
        out.pushes.is_empty() && mints.is_empty() && !dropped_mover && text_grown.is_none();
    out.sole_push = out.pushes.len() == 1
        && mints.is_empty()
        && !dropped_mover
        && text_grown.is_none()
        && out.movers.len() == 1
        && out.movers[0] == out.pushes[0].0.0;
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
            owned,
        ) else {
            // A statement the write-set walk cannot type: no scalar hoists in this loop.
            crate::rewrite_census::declined(
                "R-Header/scalars",
                &format!("{}:{}", data.def(def_nr).name(), body.scope),
                "a statement the write-set walk cannot type",
            );
            if std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
                let shown = format!("{op:?}");
                eprintln!(
                    "hoist: {} loop {} scalars declined by {}",
                    data.def(def_nr).name(),
                    body.scope,
                    &shown[..shown.len().min(200)]
                );
            }
            return out;
        };
        written.extend(&w);
    }
    out.scalars = found
        .into_iter()
        .filter(|(_, evict, _)| !evict.iter().any(|(tp, off)| written.evicts(*tp, *off)))
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

pub use crate::opt::attrs::retbuf_var;

/// `@FR-R-RecPtr` — is `n` an access of view `r` the held address SERVES: a fusable scalar
/// field read or in-place write (either spelling of the field), a copy the address serves,
/// or `r` handed as a twin's input?  The one definition the verdict's `touched` and the
/// ordered walk ([`address_use_after_growth`]) both read.  A node of any other shape is no
/// such access: a text read, a hand to a plain callee or a comparison goes through the
/// `DbRef`, so a growth after it moves nothing the address answered.
fn address_use(
    n: &Value,
    r: u16,
    data: &Data,
    stores: &Stores,
    owned: Option<&HoistOwned>,
    twin_params: &HashSet<(u32, u16)>,
) -> bool {
    let Value::Call(g, args) = n else {
        return false;
    };
    if (*g as usize) >= data.definitions.len() {
        return false;
    }
    let name = data.def(*g).name();
    let on_r = matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == r);
    if (scalar_kind(name).is_some() || setter_kind(name).is_some())
        && args.len() >= 2
        && view_field(data, &args[0], &args[1]).is_some_and(|(v, _)| v == r)
        && (on_r || nested_field_enabled())
    {
        return true;
    }
    if copy_serves_view(stores, name, args, r, owned) {
        return true;
    }
    args.iter().enumerate().any(|(i, a)| {
        matches!(a.unspan(), Value::Var(v) if *v == r)
            && u16::try_from(i).is_ok_and(|p| twin_params.contains(&(*g, p)))
    })
}

/// The ORDERED question behind `@FR-R-RecPtr`'s remainder clause: does any address-served
/// use of `r` (`address_use`) run AFTER a statement that may grow a store?  Statement order
/// within a block, each arm of an `if` on its own path from the state before it, a loop
/// body read as repeating (a growth anywhere in it precedes a use anywhere in it), and a
/// statement that both grows and uses answering yes — the order inside one statement is not
/// read.  `may_grow` is the verdict's own growth test, so the two agree on what grows.
fn address_use_after_growth(
    stmts: &[Value],
    grown: &mut bool,
    may_grow: &mut dyn FnMut(&Value) -> bool,
    is_use: &dyn Fn(&Value) -> bool,
) -> bool {
    for s in stmts {
        match s.unspan() {
            Value::Block(b) => {
                if address_use_after_growth(&b.operators, grown, may_grow, is_use) {
                    return true;
                }
            }
            Value::If(c, a, b) => {
                if address_use_after_growth(std::slice::from_ref(c), grown, may_grow, is_use) {
                    return true;
                }
                let before = *grown;
                let mut ga = before;
                if address_use_after_growth(std::slice::from_ref(a), &mut ga, may_grow, is_use) {
                    return true;
                }
                let mut gb = before;
                if address_use_after_growth(std::slice::from_ref(b), &mut gb, may_grow, is_use) {
                    return true;
                }
                *grown = ga || gb;
            }
            Value::Loop(b) => {
                let g = b.operators.iter().any(&mut *may_grow);
                let u = b.operators.iter().any(|o| o.any_node(&mut |n| is_use(n)));
                if u && (*grown || g) {
                    return true;
                }
                *grown |= g;
            }
            _ => {
                let g = may_grow(s);
                let u = s.any_node(&mut |n| is_use(n));
                if u && (*grown || g) {
                    return true;
                }
                *grown |= g;
            }
        }
    }
    false
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
    /// The plain-record parameters whose ADDRESS crosses the call (`@FR-R-RecPtr`'s twin
    /// clause): each one the body reads or writes through at least twice beyond its scalar
    /// inputs, as the parameter clause counts it.  The caller hands the address it holds,
    /// and the twin binds it where the plain body resolves the store at entry.
    pub records: Vec<u16>,
}

/// The per-callee memo of [`callee_inputs`]: `None` is a callee that has no twin — beside
/// the tuple parameters (`(R-ValueLocal)`), which a twin never takes as inputs: their
/// fields already cross the call as scalars.
#[derive(Default)]
pub struct InputCache {
    pub memo: HashMap<u32, Option<Rc<CalleeInputs>>>,
    pub params: TupleParams,
}

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
    if let Some(known) = inputs.memo.get(&d_nr) {
        return known.clone();
    }
    inputs.memo.insert(d_nr, None);
    let answer = callee_inputs_inner(d_nr, data, stores, cache, writes, inputs).map(Rc::new);
    inputs.memo.insert(d_nr, answer.clone());
    answer
}

#[expect(clippy::too_many_lines, reason = "inherited")]
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
    if call_writes_store(d_nr, data, cache, &mut active, None)
        && !in_place_only_writer(d_nr, data, cache, &mut active, None)
    {
        return None;
    }
    let vars = def.variables();
    let params = u16::try_from(def.attributes().len()).ok()?;
    let rebound = rebound_vars(body, data, vars);
    // What the body writes, as the caller's gate accounts it (`@FR-R-Callee`): a
    // return-buffer writer (§ V-ac — `brush_sample` answering a `Smp` through its buffer)
    // reaches its buffer's record type WHOLE, which no parameter's field shares; any other
    // admitted body has a typed set of its own, or no twin.
    let written = if def.hidden_return_buffer_attr().is_some()
        && retbuf_only_writer(d_nr, data, cache, &mut active, None)
    {
        let attr = def.hidden_return_buffer_attr()?;
        WriteSet {
            offsets: HashSet::new(),
            whole: HashSet::from([plain_record_type(data, &def.attributes()[attr].typedef)?]),
            retbufs: HashSet::new(),
            own: HashSet::new(),
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
                None,
            )?);
        }
        written
    };
    // A parameter (never rebound) as a candidate root: its plain record type, or — for a
    // header — whether it names a vector at all.  A TUPLE parameter (`(R-ValueLocal)`)
    // is no root: its fields are scalars the call already hands over.
    let tuple_params = tuple_param_vars(data, d_nr, &inputs.params);
    let record_param = |p: u16| -> Option<u16> {
        if p < params && !rebound.contains(&p) && !tuple_params.contains_key(&p) {
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
    if twin_record_enabled() {
        for p in 0..params {
            // A hidden return buffer is the callee's own to mint: no caller holds it.
            if record_param(p).is_none() || def.attributes()[usize::from(p)].hidden {
                continue;
            }
            let served: Vec<i64> = out
                .scalars
                .iter()
                .filter(|(q, _, _)| *q == p)
                .map(|(_, f, _)| *f)
                .collect();
            if scalar_field_uses(&body.operators, p, data, &served) >= 2 {
                out.records.push(p);
            }
        }
    }
    (!out.scalars.is_empty() || !out.headers.is_empty() || !out.records.is_empty()).then_some(out)
}

/// `@FR-R-RecPtr`'s twin clause, switched off by `LOFT_NO_TWIN_RECORD`.
fn twin_record_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_TWIN_RECORD").is_ok_and(|v| v != "0"))
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
        Value::Span(b) => Value::Span(Box::new((b.0, substitute_path(&b.1, from, arg)))),
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
            None,
            cache,
            &mut HashSet::new(),
            Some(vars),
            HoistTiers {
                in_place: allow_in_place,
                ..HoistTiers::default()
            },
            &mut HashSet::new(),
            None,
        )
    }) {
        return None;
    }
    let mut rebound = false;
    let mut indexed = false;
    for op in rest {
        op.any_node(&mut |n| {
            match n {
                _ if rebinds_root(n, *d, vars) => rebound = true,
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
        .chain(enum_record_enabled().then_some(&TAG_GETTER))
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
// The parameters are the block and the statement, the two things that type it, the
// function, the memo, the write tier, the twin parameters, the store facts and the
// emitter's owned buffers; a struct would put a name between each and the one call site
// without removing anything.
#[allow(clippy::too_many_arguments)]
pub fn record_view_ptr(
    stmts: &[Value],
    at: usize,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
    twin_params: &HashSet<(u32, u16)>,
    facts: Option<&StoreFacts>,
    owned: Option<&HoistOwned>,
    extra_use: bool,
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
    if plain_record_type(data, view_tp).is_none() && !struct_enum_view(data, view_tp) {
        return Err("not a plain record");
    }
    // A buffer's pre-init (`__ref_p2_N = null`, then a mint into it): no place yet, so no
    // address — measured: the address was taken null and the literal's field writes through
    // it were dropped, a library's `mk()` answering a record of zeros (loft's own 47 golden
    // and the #672 parity test).
    if matches!(rhs.unspan(), Value::Null) {
        return Err("bound null");
    }
    view_extent_verdict(
        &stmts[at + 1..],
        *r,
        data,
        stores,
        def_nr,
        cache,
        allow_in_place,
        twin_params,
        facts,
        owned,
        None,
        extra_use,
    )?;
    Ok(*r)
}

/// `@FR-R-RecPtr`'s PARAMETER clause — may record parameter `p` hold its address for the
/// whole function body `stmts`?
///
/// A parameter is a view the caller fixed before the first statement: its `DbRef` names
/// one place for the call, exactly as a binding's does from its bind, so the body is the
/// binding's remainder and is judged by the same verdict ([`view_extent_verdict`]) — no
/// growth of the parameter's store, no free before a use, no rebind, a fusable use.  What
/// the caller holds is the caller's business: a store grows only through a write in THIS
/// activation (a callee's growth is one the verdict sees through `blocks_header_hoist`),
/// so nothing outside the body can move the record while the body runs.
///
/// # Errors
///
/// The reason it declines, in the words `LOFT_TRACE_RECPTR=1` prints.
// The parameters are `record_view_ptr`'s, with the parameter in place of the statement.
#[allow(clippy::too_many_arguments)]
pub fn param_view_ptr(
    stmts: &[Value],
    p: u16,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
    twin_params: &HashSet<(u32, u16)>,
    facts: Option<&StoreFacts>,
    owned: Option<&HoistOwned>,
) -> Result<u16, &'static str> {
    let vars = data.def(def_nr).variables();
    if !vars.is_argument(p) {
        return Err("not a parameter");
    }
    let tp = vars.tp(p);
    if matches!(tp, Type::Optional(_)) || plain_record_type(data, tp).is_none() {
        return Err("not a plain record");
    }
    view_extent_verdict(
        stmts,
        p,
        data,
        stores,
        def_nr,
        cache,
        allow_in_place,
        twin_params,
        facts,
        owned,
        None,
        false,
    )?;
    Ok(p)
}

/// How many fusable scalar field reads and writes of record `r` — either spelling of the
/// field ([`view_field`]) — stand in `stmts`, beside those at an offset in `served`: a
/// twin's inputs (`@FR-R-Inputs`), which its body reads from a parameter and not the record.
/// A served field is one the body never writes, so every use at its offset is a read.
#[must_use]
pub fn scalar_field_uses(stmts: &[Value], r: u16, data: &Data, served: &[i64]) -> usize {
    let mut n = 0;
    for op in stmts {
        op.walk(&mut |c| {
            if let Value::Call(g, args) = c
                && (*g as usize) < data.definitions.len()
                && args.len() >= 2
            {
                let name = data.def(*g).name();
                if (scalar_kind(name).is_some() || setter_kind(name).is_some())
                    && view_field(data, &args[0], &args[1])
                        .is_some_and(|(v, off)| v == r && !served.contains(&off))
                {
                    n += 1;
                }
            }
        });
    }
    n
}

/// `@FR-R-RecPtr`'s MINT clause — does statement `at` of `stmts` mint a plain-record
/// element (`e = OpNewRecord(…)`) whose ADDRESS may serve the group's writes, from the mint
/// up to its own `OpFinishRecord(…, e, …)`?
///
/// [`record_view_ptr`] promises an address for the whole REMAINDER of a block, and a mint
/// group never gets one: the remainder holds the next append, which grows a store.  The
/// element needs the address only while it is being filled, and that WINDOW — the
/// statements between the mint and its finish — is the literal's field writes and the
/// expressions they store.  The window is judged exactly as a remainder is
/// ([`view_extent_verdict`]): nothing in it grows a store (a nested mint, a builder that
/// appends, a delivery copy all decline), frees a record before a use, or rebinds `e`, and
/// at least one write is fusable.  The address is taken right after the mint — after its
/// growth step, if it had one — so the window cannot start stale.  Any mint form
/// qualifies (a vector's tail slot, a keyed collection's claimed record): each answers one
/// `DbRef` whose bytes stay put until a store grows.
///
/// Answers the element variable and the index of its finish, where the address dies.
///
/// # Errors
///
/// The reason the statement declines, as `LOFT_TRACE_RECPTR=1` prints it.
// The parameters are the block and the statement, the two things that type it, the
// function, the memo, the write tier and the emitter's owned buffers, as `record_view_ptr`'s.
#[allow(clippy::too_many_arguments)]
pub fn mint_window(
    stmts: &[Value],
    at: usize,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
    owned: Option<&HoistOwned>,
) -> Result<(u16, usize), &'static str> {
    let Some(Value::Set(e, rhs)) = stmts.get(at).map(Value::unspan) else {
        return Err("not a binding");
    };
    let minted = matches!(rhs.unspan(), Value::Call(d, args)
        if (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpNewRecord"
            && args.len() == 3);
    if !minted {
        return Err("not a mint");
    }
    // The element is a fresh record whatever its type: the window's writes carry the
    // offsets the literal's lowering resolved, so the address asks no layout.  (A
    // struct-enum element's variable is typed by a placeholder record, not by the enum.)
    let e_tp = data.def(def_nr).variables().tp(*e);
    // `.base()`: the shape question sees through a `τ?` slot (`@FR-N-Shape`).
    let is_record = matches!(e_tp.base(), Type::Reference(_, _) | Type::Enum(_, true, _));
    if !is_record || (plain_record_type(data, e_tp).is_none() && !enum_record_enabled()) {
        return Err("not a record element");
    }
    let finish = stmts[at + 1..].iter().position(|op| {
        matches!(op.unspan(), Value::Call(d, args)
            if (*d as usize) < data.definitions.len()
                && data.def(*d).name() == "OpFinishRecord"
                && matches!(args.get(1).map(Value::unspan), Some(Value::Var(v)) if v == e))
    });
    let Some(finish) = finish.map(|i| at + 1 + i) else {
        return Err("the mint's finish is not in this block");
    };
    view_extent_verdict(
        &stmts[at + 1..finish],
        *e,
        data,
        stores,
        def_nr,
        cache,
        allow_in_place,
        &HashSet::new(),
        None,
        owned,
        Some(*e),
        false,
    )?;
    Ok((*e, finish))
}

/// The statements a record view's address must stay good for — a block's remainder
/// ([`record_view_ptr`]) or a mint's window ([`mint_window`]) — judged under ONE definition:
/// none may grow a store, free a record before a use of `r`, or rebind `r`, and at least one
/// must read or write a fusable scalar field of `r` or hand it to a twin.
/// `@FR-R-RecPtr` — may a use of view `r` run AFTER a record free, anywhere in `stmts`?
///
/// Through the store a freed record answers the sentinel; through an address it would
/// answer stale bytes — so an address is held only where every free follows the last use.
/// The walk is in EXECUTION order, down through the structure the statements have: a block
/// is its statements in sequence, an `if` its condition and then either arm, a loop its body
/// — twice when the body itself frees, since a free at the end of one pass precedes a use
/// at the start of the next.  Everything else is a leaf, and a leaf that both uses `r` and
/// frees is refused without asking its inner order.
///
/// A block whose last statement is a `return`, and which holds no `break` or `continue`,
/// leaves the FUNCTION on every path that reaches a free in it, so its frees precede no
/// later use and are not carried past it.  That is the shape of every find-and-answer
/// helper — `for c in m.chunks { if … { return c.hexes[i]?.h } }`, where the `return`
/// releases the `?` discharge buffer after the last read of `c`.  A `break` or `continue`
/// keeps running code of this function, so a block holding one carries its frees on.
fn free_before_use(
    stmts: &[Value],
    r: u16,
    data: &Data,
    vars: &crate::variables::Function,
) -> bool {
    free_before_use_by(stmts, &mut |leaf| {
        let (mut uses, mut releases) = (false, false);
        leaf.any_node(&mut |n| {
            match n {
                Value::Var(x) if *x == r => uses = true,
                Value::Call(g, args)
                    if (*g as usize) < data.definitions.len()
                        && frees_a_record(data.def(*g).name(), args, Some(vars)) =>
                {
                    releases = true;
                }
                _ => {}
            }
            false
        });
        (uses, releases)
    })
}

/// [`free_before_use`]'s ORDER walk, over any leaf classifier answering `(uses the view,
/// frees a record)` — the structure is the claim, so it is what the unit tests drive.
fn free_before_use_by(stmts: &[Value], leaf_of: &mut dyn FnMut(&Value) -> (bool, bool)) -> bool {
    /// `Err(())` — a use after a free; `Ok(released)` — whether a free may have run by
    /// the end of `v`, given whether one may have run before it.
    fn scan(
        v: &Value,
        released: bool,
        leaf_of: &mut dyn FnMut(&Value) -> (bool, bool),
    ) -> Result<bool, ()> {
        match v.unspan() {
            Value::Block(b) => {
                let mut now = released;
                for op in &b.operators {
                    now = scan(op, now, leaf_of)?;
                }
                let returns = b
                    .operators
                    .iter()
                    .rev()
                    .find(|o| !matches!(o, Value::Line(_)))
                    .is_some_and(|o| matches!(o.unspan(), Value::Return(_)));
                let jumps = v.any_node(&mut |n| matches!(n, Value::Break(_) | Value::Continue(_)));
                Ok(if returns && !jumps { released } else { now })
            }
            Value::If(cond, on_true, on_false) => {
                let after = scan(cond, released, leaf_of)?;
                let a = scan(on_true, after, leaf_of)?;
                let b = scan(on_false, after, leaf_of)?;
                Ok(a || b)
            }
            Value::Loop(lp) => {
                let mut now = released;
                for op in &lp.operators {
                    now = scan(op, now, leaf_of)?;
                }
                if now && !released {
                    // A free in one pass precedes every use in the next.
                    let mut again = true;
                    for op in &lp.operators {
                        again = scan(op, again, leaf_of)?;
                    }
                }
                Ok(now)
            }
            leaf => {
                let (uses, frees) = leaf_of(leaf);
                if uses && (released || frees) {
                    return Err(());
                }
                Ok(released || frees)
            }
        }
    }
    let mut released = false;
    for op in stmts {
        match scan(op, released, leaf_of) {
            Ok(now) => released = now,
            Err(()) => return true,
        }
    }
    false
}

#[cfg(test)]
mod free_order_tests {
    use super::*;

    // `Var(1)` stands for a use of the view, `Var(2)` for a record free; everything else is
    // neither.  The walk's claim is about ORDER and STRUCTURE, which is all these drive.
    fn leaf(v: &Value) -> (bool, bool) {
        (
            v.any_node(&mut |n| matches!(n, Value::Var(1))),
            v.any_node(&mut |n| matches!(n, Value::Var(2))),
        )
    }
    fn verdict(stmts: &[Value]) -> bool {
        free_before_use_by(stmts, &mut leaf)
    }
    fn block(ops: Vec<Value>) -> Value {
        Value::Block(Box::new(Block {
            name: "block",
            operators: ops,
            result: Type::Void,
            scope: 0,
            var_size: 0,
        }))
    }
    fn when(then: Value) -> Value {
        Value::If(
            Box::new(Value::Boolean(true)),
            Box::new(then),
            Box::new(Value::Null),
        )
    }
    fn lp(ops: Vec<Value>) -> Value {
        let Value::Block(b) = block(ops) else {
            unreachable!()
        };
        Value::Loop(b)
    }
    const USE: Value = Value::Var(1);
    const FREE: Value = Value::Var(2);
    fn ret() -> Value {
        Value::Return(Box::new(Value::Int(0)))
    }

    #[test]
    fn a_free_after_the_last_use_is_fine_and_one_before_a_use_is_not() {
        assert!(!verdict(&[USE, FREE]));
        assert!(verdict(&[FREE, USE]));
        assert!(
            verdict(&[USE, block(vec![FREE]), USE]),
            "a nested block runs in sequence"
        );
    }

    #[test]
    fn a_returning_block_carries_no_free_past_itself() {
        // `for c in v { if … { use; free; return } use }` — the lookup helper.
        let body = vec![when(block(vec![USE, FREE, ret()])), USE];
        assert!(!verdict(&[lp(body)]));
        // …but a use AFTER the free inside that block is still a use after a free.
        assert!(verdict(&[when(block(vec![FREE, USE, ret()]))]));
    }

    #[test]
    fn a_break_or_continue_keeps_running_this_function() {
        // The free stands before a `break`, and the view is used after the loop.
        let body = vec![when(block(vec![FREE, Value::Break(0)]))];
        assert!(verdict(&[lp(body), USE]));
        // A returning block that also holds a `break` carries its free on.
        let mixed = block(vec![when(Value::Break(0)), FREE, ret()]);
        assert!(verdict(&[lp(vec![USE, when(mixed)])]));
    }

    #[test]
    fn a_free_at_the_end_of_one_pass_precedes_a_use_in_the_next() {
        assert!(verdict(&[lp(vec![USE, FREE])]));
        assert!(!verdict(&[lp(vec![USE]), FREE]));
    }

    #[test]
    fn either_arm_of_an_if_may_have_freed() {
        let v = Value::If(
            Box::new(Value::Boolean(true)),
            Box::new(block(vec![FREE])),
            Box::new(Value::Null),
        );
        assert!(verdict(&[v, USE]));
    }
}

// The eight parameters are the extent, the view, the two things that type it, the
// function, the memo, the write tier and the twin parameters; a struct would put a name
// between each and the two call sites without removing anything.
// `@FR-H-ShortRef` — the held address is valid only while the store it points into grows
// nowhere in the view's extent; "the remainder may grow a store" is that refusal.
// A refusal is one `R-RecPtr` decline of the view in the census, under its reason.
#[allow(clippy::too_many_arguments)]
fn view_extent_verdict(
    rest: &[Value],
    r: u16,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
    twin_params: &HashSet<(u32, u16)>,
    facts: Option<&StoreFacts>,
    owned: Option<&HoistOwned>,
    fresh_e: Option<u16>,
    extra_use: bool,
) -> Result<(), &'static str> {
    let verdict = view_extent_verdict_inner(
        rest,
        r,
        data,
        stores,
        def_nr,
        cache,
        allow_in_place,
        twin_params,
        facts,
        owned,
        fresh_e,
        extra_use,
    );
    if let Err(why) = verdict {
        let def = data.def(def_nr);
        crate::rewrite_census::declined(
            "R-RecPtr",
            &format!("{}:{}", def.name(), def.variables().name(r)),
            why,
        );
    }
    verdict
}

#[allow(clippy::too_many_arguments)]
#[expect(
    clippy::too_many_lines,
    reason = "the trace's classification stands beside the verdict"
)]
fn view_extent_verdict_inner(
    rest: &[Value],
    r: u16,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    allow_in_place: bool,
    twin_params: &HashSet<(u32, u16)>,
    facts: Option<&StoreFacts>,
    owned: Option<&HoistOwned>,
    fresh_e: Option<u16>,
    extra_use: bool,
) -> Result<(), &'static str> {
    let vars = data.def(def_nr).variables();
    let r = &r;
    // A mint window's own element is FRESH for its extent: a copy into it is a write of
    // its bytes, not a growth (`fresh_copy` in `blocks_header_hoist`).
    let mut fresh: HashSet<u16> = fresh_e.into_iter().collect();
    let mut rebound = false;
    let mut touched = false;
    for op in rest {
        op.any_node(&mut |n| {
            match n {
                _ if rebinds_root(n, *r, vars) => rebound = true,
                Value::Call(g, args) if (*g as usize) < data.definitions.len() => {
                    let name = data.def(*g).name();
                    let on_r =
                        matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == *r);
                    // A NATIVE op with the view as its first operand that is not a read
                    // (`OpGet…`) or an in-place scalar set (`IN_PLACE_SET_OPS`: a fixed-width
                    // value stored at the address it is given — a kind the address does not
                    // serve keeps its store write, to the same bytes) re-seats or releases
                    // the place — `OpDatabaseNP(buf, tp)` mints into it, `OpNewRecord`, a
                    // copy into it, a free — so it is a rebind; a loft-bodied callee can only
                    // write in place (`(R-Callee)`).
                    let native = matches!(data.def(*g).code(), Value::Null)
                        || !data.def(*g).rust().is_empty();
                    // `OpCopyRecord(r, dst, tp)` has the view as its SOURCE: a read of r.
                    if on_r
                        && native
                        && !name.starts_with("OpGet")
                        && !IN_PLACE_SET_OPS.contains(&name)
                        && !REF_COMPARE_OPS.contains(&name)
                        && name != "OpCopyRecord"
                    {
                        rebound = true;
                    }
                    // A fusable scalar field read or in-place write of the view, by either
                    // spelling of the field (`view_field`): direct, or through inline
                    // sub-records.
                    if (scalar_kind(name).is_some() || setter_kind(name).is_some())
                        && args.len() >= 2
                        && view_field(data, &args[0], &args[1]).is_some_and(|(v, _)| v == *r)
                        && (on_r || nested_field_enabled())
                    {
                        touched = true;
                    }
                    if copy_serves_view(stores, name, args, *r, owned) {
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
    }
    // Asked before the growth walk: a view the remainder never reads through its address
    // gains nothing from one, so its verdict names that and not a growth that would have
    // cost nothing (the decline census counts the reasons that cost).  `extra_use` is the
    // emitter's own count of the uses this walk does not see — a text field read under a
    // held span, a vector field read (`@FR-R-RecPtr`'s text and nested clauses) — so such a
    // view still meets every condition below.
    if !touched && !extra_use {
        return Err(NO_FUSABLE_USE);
    }
    // `@FR-R-RecPtr`'s remainder clause — a push or mint into a store proven apart from the
    // view's cannot move it, so the remainder may grow THOSE stores: the push and mint tiers
    // open exactly when every mover's root is distinct from `r`'s store.
    let movers_apart = facts.is_some_and(|f| {
        let roots = mover_roots(rest, data, stores, vars);
        !roots.is_empty() && roots.iter().all(|m| f.distinct(*r, *m))
    });
    if let Some(op) = rest.iter().find(|op| {
        blocks_header_hoist(
            op,
            data,
            Some(stores),
            cache,
            &mut HashSet::new(),
            Some(vars),
            HoistTiers {
                in_place: allow_in_place,
                push: movers_apart,
                mint: movers_apart,
                ..HoistTiers::default()
            },
            &mut fresh,
            owned,
        )
    }) {
        if std::env::var("LOFT_TRACE_RECPTR").is_ok() {
            let shown = format!("{op:?}");
            eprintln!(
                "recptr:   `{}` grows a store by {}",
                vars.name(*r),
                &shown[..shown.len().min(220)]
            );
            // The classification line: the view's chain end and freshness, each mover's,
            // and the apart verdict — what a sharper fact would have to prove.
            let end_of = |x: u16| -> String {
                match facts.and_then(|f| f.terminal(x)) {
                    Some(e) => format!(
                        "{}{}",
                        vars.name(e),
                        if facts.is_some_and(|f| f.owns_alone(e)) {
                            "+"
                        } else {
                            "-"
                        }
                    ),
                    None => "?".to_owned(),
                }
            };
            let roots = mover_roots(std::slice::from_ref(op), data, stores, vars);
            let movers: Vec<String> = roots
                .iter()
                .map(|m| {
                    format!(
                        "{}={}{}",
                        vars.name(*m),
                        end_of(*m),
                        if facts.is_some_and(|f| f.distinct(*r, *m)) {
                            ":apart"
                        } else {
                            ":same?"
                        }
                    )
                })
                .collect();
            let mut buffer_op = String::new();
            op.any_node(&mut |n| {
                if let Value::Call(d, args) = n
                    && (*d as usize) < data.definitions.len()
                    && matches!(
                        data.def(*d).name(),
                        "OpDatabase" | "OpDatabaseNP" | "OpClear"
                    )
                    && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                    && *b < vars.count()
                    && buffer_op.is_empty()
                {
                    buffer_op = format!("{}({})", data.def(*d).name(), vars.name(*b));
                }
                false
            });
            let mut grown = false;
            let mut fresh_walk = fresh.clone();
            let after = address_use_after_growth(
                rest,
                &mut grown,
                &mut |o: &Value| {
                    blocks_header_hoist(
                        o,
                        data,
                        Some(stores),
                        cache,
                        &mut HashSet::new(),
                        Some(vars),
                        HoistTiers {
                            in_place: allow_in_place,
                            push: movers_apart,
                            mint: movers_apart,
                            ..HoistTiers::default()
                        },
                        &mut fresh_walk,
                        owned,
                    )
                },
                &|n: &Value| address_use(n, *r, data, stores, owned, twin_params),
            );
            eprintln!(
                "recptr:   class `{}` end={} facts={} movers=[{}] buffer-op={} use-after-growth={}",
                vars.name(*r),
                end_of(*r),
                if facts.is_some() { "some" } else { "none" },
                movers.join(" "),
                if buffer_op.is_empty() {
                    "-"
                } else {
                    &buffer_op
                },
                if after { "yes" } else { "no" }
            );
        }
        return Err("the remainder may grow a store");
    }
    if movers_apart && std::env::var("LOFT_TRACE_RECPTR").is_ok() {
        eprintln!(
            "recptr: {} keeps `{}` beside a growth of another store",
            data.def(def_nr).name(),
            vars.name(*r)
        );
    }
    // In statement order: a free (a scope exit's release of a local, a buffer's) BEFORE a
    // use of `r` declines — through the store a freed record answers the sentinel, through
    // an address it would answer stale bytes — while the frees that follow the last use,
    // which is where a block's own releases stand, cost nothing.  A view that outlives its
    // container is `(B-Disturb)`'s case and is materialised by the parser before this runs.
    if free_before_use(rest, *r, data, vars) {
        return Err("the remainder frees a record before a use of the view");
    }
    if rebound {
        return Err("the remainder rebinds the view");
    }
    Ok(())
}

/// Is `name(args)` a record copy the view `r`'s address serves?  A copy INTO the view FROM a
/// value local — a builder's tuple delivered into an appended element — lowers to one typed
/// setter per field (`OpCopyRecordEmitter`), each served like a direct write; a flat copy
/// INTO or OUT OF the view ([`flat_copy`]) stores its bytes at, or reads them from, the
/// address.  Any other copy from a store record asks the store, not the address.
fn copy_serves_view(
    stores: &Stores,
    name: &str,
    args: &[Value],
    r: u16,
    owned: Option<&HoistOwned>,
) -> bool {
    if name != "OpCopyRecord" || args.len() != 3 {
        return false;
    }
    let is_r = |v: &Value| matches!(v.unspan(), Value::Var(x) if *x == r);
    let from_value_local = is_r(&args[1])
        && matches!(args[0].unspan(), Value::Var(s)
            if owned.is_some_and(|o| o.value_locals.contains(s)));
    from_value_local
        || ((is_r(&args[1]) || is_r(&args[0])) && flat_copy(stores, name, args).is_some())
}

/// [`record_view_ptr`]'s FIRST decline after the walk, asked before the growth, free and
/// rebind conditions: no statement of the remainder reads or writes a field the address
/// serves, and the emitter counted no use of its own (`extra_use` — the text and nested
/// clauses, `Output::extra_view_use`).
pub const NO_FUSABLE_USE: &str = "no fusable field read or write of the view";

/// Does `rest` read a text field of view `r` directly — `OpGetText(r, fld)` with a literal
/// field offset, the spelling `vector::rec_text` serves?
#[must_use]
pub fn reads_text_field(rest: &[Value], r: u16, data: &Data) -> bool {
    rest.iter().any(|op| {
        op.any_node(&mut |n| {
            matches!(n, Value::Call(g, a)
                if (*g as usize) < data.definitions.len()
                    && data.def(*g).name() == "OpGetText"
                    && matches!(a.first().map(Value::unspan), Some(Value::Var(v)) if *v == r)
                    && matches!(a.get(1).map(Value::unspan), Some(Value::Int(_))))
        })
    })
}

/// How many reads in `rest` take a VECTOR field of view `r` as their vector — a length or an
/// element read (`OpLengthVector` or `len`'s `t_6vector_len`, whose body it is,
/// `OpGetVector`, `OpGetVectorNullable`) whose operand is an
/// `OpGetField` path rooted at `r`: the reads `@FR-R-RecPtr`'s nested clause serves
/// (`Output::field_header_expr`).  Counted by site, not by execution.
#[must_use]
pub fn vector_field_reads(rest: &[Value], r: u16, data: &Data) -> usize {
    let mut n = 0;
    for op in rest {
        op.any_node(&mut |node| {
            if let Value::Call(g, a) = node
                && (*g as usize) < data.definitions.len()
                && matches!(
                    data.def(*g).name(),
                    "OpLengthVector" | "t_6vector_len" | "OpGetVector" | "OpGetVectorNullable"
                )
                && a.first()
                    .and_then(|v| vector_path(data, v))
                    .is_some_and(|(root, offs)| root == r && !offs.is_empty())
            {
                n += 1;
            }
            false
        });
    }
    n
}

/// `@FR-R-RecPtr`'s path clause is on — `LOFT_NO_NESTED_FIELD=1` keeps a field reached
/// through an inline sub-record (`v.pos.x`) on its store read (`@FR-R-Switch`).  Read at
/// generation time.
#[must_use]
pub fn nested_field_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_NESTED_FIELD").is_ok_and(|v| v != "0"))
}

/// The typed getters an element read can be fused INTO, with the Rust type each reads and
/// the null sentinel its `#rust` template answers at the absent element.
///
/// All three are `if rec != 0 && valid(..) { *addr } else { <sentinel> }` in `Store`, which
/// is what makes one fused load able to stand for the pair. A getter with a different shape
/// — `OpGetBoolean` masks, `OpGetByte` re-bases, `OpGetCharacter` decodes — is left out
/// rather than approximated; it keeps the unfused emission.
/// `@FR-R-RecPtr`'s enum clause — `OpGetEnum(r, fld)`, a struct-enum's TAG, read through a
/// record address: one byte, and `0` (no variant) for the null record, which is the
/// getter's own answer there.  For the address only — an element read through a header
/// keeps [`FUSABLE_GETTERS`], whose cold path has no one-byte form.
const TAG_GETTER: (&str, &str, &str) = ("OpGetEnum", "u8", "0u8");

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
    /// The element address is the RAISING `OpGetVector`: an index outside the vector reports
    /// (`@FR-E-Report`) beside the absent value.  `OpGetVectorNullable` answers it silently.
    pub raises: bool,
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
        raises: data.def(*elem_op).name() == "OpGetVector",
    })
}

/// `@FR-R-Base`'s join clause — a scalar field of a `?`-DISCHARGED element, `v[i]?.f`: the
/// getter's operand is not the element address [`fused_element_read`] wants but the JOIN
/// the `?` lowers to, which answers the element when it is present and a default record
/// minted into a discharge buffer when it is not.
pub struct JoinRead<'a> {
    /// The join's temp — assigned the element address on every path, so the fused form
    /// assigns it too.
    pub temp: u16,
    /// The join block itself: emitted whole, as the fallback, for every index the fast
    /// path refuses.
    pub join: &'a Value,
    pub vector: &'a Value,
    pub path: PathKey,
    pub size: &'a Value,
    /// The index VARIABLE.  A computed index is declined: it would be evaluated for the
    /// range test and again inside the fallback's join, and the second evaluation of a
    /// CHECKED operator can note one overflow twice.
    pub index: u16,
    pub fld: &'a Value,
    pub rust_type: &'static str,
    pub absent: &'static str,
}

/// Recognise `OpGet<scalar>(Block "ncc" { Set(t, OpGetVector*(path, size, Var(i)));
/// If(OpConvBoolFromRef(Var(t)), Var(t), <absent arm>) }, fld)` — the ONE definition of the
/// fused join read, asked by the pre-eval collector (which then leaves the join block where
/// it stands instead of lifting it into a `let _pre_N`) and by the emitter (which folds it),
/// so the two cannot disagree; an arm in the emitter alone would never fire, because the
/// collector lifts every `Block` argument.  Shape only: the caller confirms the path holds
/// a header AND an element base.
///
/// The fallback is `None` and that is the safe side: every shape not named here — a second
/// statement in the join, a present arm that is not the temp, a test that is not the ref
/// conversion, a computed index — keeps the general emission, which costs the fused load and
/// never a value.  The ABSENT arm is deliberately not inspected: it is emitted verbatim on
/// the fallback path, so whatever it mints, this rewrite runs it exactly as it stood.
#[must_use]
pub fn fused_join_read<'a>(data: &Data, getter: &str, args: &'a [Value]) -> Option<JoinRead<'a>> {
    let [join, fld] = args else { return None };
    let (_, rust_type, absent) = FUSABLE_GETTERS
        .iter()
        .find(|(name, _, _)| *name == getter)?;
    let Value::Block(bl) = join.unspan() else {
        return None;
    };
    let stmts: Vec<&Value> = bl
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let [bind, select] = stmts[..] else {
        return None;
    };
    let Value::Set(temp, elem) = bind.unspan() else {
        return None;
    };
    let Value::Call(elem_op, elem_args) = elem.unspan() else {
        return None;
    };
    if !is_element_address(data, *elem_op) {
        return None;
    }
    let [vector, size, index] = &elem_args[..] else {
        return None;
    };
    let Value::Var(index) = index.unspan() else {
        return None;
    };
    // The select: `if <temp is present> { temp } else { … }`.
    let Value::If(test, present, _absent) = select.unspan() else {
        return None;
    };
    let Value::Call(conv, conv_args) = test.unspan() else {
        return None;
    };
    if (*conv as usize) >= data.definitions.len()
        || data.def(*conv).name() != "OpConvBoolFromRef"
        || !matches!(conv_args.as_slice(), [a] if matches!(a.unspan(), Value::Var(t) if t == temp))
        || !matches!(present.unspan(), Value::Var(t) if t == temp)
    {
        return None;
    }
    // The index must not be the temp, and the join must not rebind it: the range test reads
    // it before the join runs.
    if index == temp {
        return None;
    }
    let path = vector_path(data, vector)?;
    Some(JoinRead {
        temp: *temp,
        join,
        vector,
        path,
        size,
        index: *index,
        fld,
        rust_type,
        absent,
    })
}

/// The cast a fused push's VALUE takes to become its element type: an unbiased `u8` kind
/// (`OpPushBoolean`, `OpPushEnum`) is spelled `as u8`, because a boolean's native value is a
/// `bool`; every other kind's value already has its element type (the byte kind's is
/// encoded by `Store::byte_raw`, the `i32`/`u32` kinds convert in the push emitter).
#[must_use]
pub fn push_value_cast(rust_type: &str, bias: bool) -> &'static str {
    if rust_type == "u8" && !bias {
        " as u8"
    } else {
        ""
    }
}

/// The element an unbiased fused push stores, from its VALUE spelled `val` (an `i64` for every
/// integer kind): an `i32` keeps the integer null as `i32::MIN` (`OpPushInt4`), a `u32` is the
/// value `as u32` (`OpPushCharacter`), a `u8` kind takes [`push_value_cast`], and every other
/// kind's value already is its element.  The push fill stores `n` of these, so it spells its
/// value here; the per-element push (`HoistedPushEmitter`) writes the same arms in line.
#[must_use]
pub fn push_element(rust_type: &str, val: &str) -> String {
    match rust_type {
        "i32" => format!(
            "{{ let __v: i64 = ({val}); if __v == i64::MIN {{ i32::MIN }} else {{ __v as i32 }} }}"
        ),
        "u32" => format!("({val}) as u32"),
        _ => format!("({val}){}", push_value_cast(rust_type, false)),
    }
}

/// `@FR-R-PushFill`'s repeat-literal clause — the lowering of `[c, c, …, c]` / `[c; n]` into a
/// vector: `{ OpPreAllocVector(p, n, w); OpPush<K>(p, c) }` then `OpAppendCopy(p, n, tp)`, the
/// template pushed once and copied `n - 1` times.  When `c` is a literal of an unbiased
/// scalar kind, the two statements are ONE fill of `n` copies at the vector's tail (the same
/// elements, the same final length); the fill's own refusals (a count that is not positive,
/// a null vector) fall back to the two statements as they stand.  Answers the vector, the
/// count, the value, its Rust type and width.
pub struct RepeatLiteral<'a> {
    pub vector: &'a Value,
    pub count: i64,
    pub val: &'a Value,
    pub rust_type: &'static str,
    pub size: u32,
}

/// `@FR-R-PushFill`'s run clause — a straight-line RUN of byte appends to one vector,
/// `v += [a]; v += [b]; …`, each lowered to `Insert([OpPreAllocVector(v, 1, 1),
/// OpPushByte(v, min, x)])` with only line markers between them.  Answers the vector, the
/// shared `min`, each pushed value in order, and the index of the run's last statement.
///
/// A value may read no variable that could reach `v`'s store — `v`'s root itself, or any
/// local that is not a scalar (a view of `v` would read the length the open window has not
/// written back yet) — and may call nothing but an operator.  Two pushes at least: one is
/// already a single append.
pub struct PushRun<'a> {
    pub vector: &'a Value,
    pub min: &'a Value,
    pub vals: Vec<&'a Value>,
    pub last: usize,
}

/// Recognise a [`PushRun`] starting at statement `at`.
#[must_use]
/// `@FR-R-PushFill`'s run clause: whether `val`, pushed onto a vector rooted at `root`, can be
/// written while the vector's header is held — it reads neither `root` nor any non-scalar local
/// (a view could reach the store and read a length not yet written back) and calls nothing but
/// an operator, so nothing it does can move the store's memory.
pub fn run_value_plain(
    data: &Data,
    vars: &crate::variables::Function,
    root: u16,
    val: &Value,
) -> bool {
    let mut ok = true;
    val.walk(&mut |n| match n {
        Value::Var(x) if *x == root || !crate::data::is_scalar(vars.tp(*x).base()) => {
            ok = false;
        }
        Value::Call(d, _) if !data.def(*d).name().starts_with("Op") => ok = false,
        // A value with statements of its own — a `??` temp bound in a block, a branch —
        // is emitted with pre-evaluations a held header's single expression cannot carry.
        Value::CallRef(..)
        | Value::Block(_)
        | Value::Loop(_)
        | Value::Insert(_)
        | Value::Set(..)
        | Value::If(..) => ok = false,
        _ => {}
    });
    ok
}

pub fn push_run<'a>(
    ops: &'a [Value],
    at: usize,
    data: &Data,
    vars: &crate::variables::Function,
) -> Option<PushRun<'a>> {
    let named = |v: &'a Value, n: &str| -> Option<&'a [Value]> {
        match v.unspan() {
            Value::Call(d, a)
                if (*d as usize) < data.definitions.len() && data.def(*d).name() == n =>
            {
                Some(a)
            }
            _ => None,
        }
    };
    // One append: `Insert([pre, push])`, or — once a block flattens the insert — the two
    // statements `pre`, `push` in a row.  Answers the vector, `min`, the value and the index
    // of the statement after it.
    let one = |i: usize| -> Option<(&'a Value, &'a Value, &'a Value, usize)> {
        let (pre, push, next) = match ops.get(i)?.unspan() {
            Value::Insert(ins) => match &ins[..] {
                [pre, push] => (pre, push, i + 1),
                _ => return None,
            },
            _ => (ops.get(i)?, ops.get(i + 1)?, i + 2),
        };
        let [v, Value::Int(1), Value::Int(1)] = named(pre, "OpPreAllocVector")? else {
            return None;
        };
        let [v2, min, val] = named(push, "OpPushByte")? else {
            return None;
        };
        (v.unspan() == v2.unspan()).then_some((v, min, val, next))
    };
    let (vector, min, first, mut next) = one(at)?;
    let (root, _) = vector_path(data, vector)?;
    let plain = |val: &Value| run_value_plain(data, vars, root, val);
    if !plain(first) {
        return None;
    }
    let mut pushed = vec![first];
    let mut last = next - 1;
    loop {
        while matches!(ops.get(next), Some(Value::Line(_))) {
            next += 1;
        }
        let Some((v, m, val, after)) = one(next) else {
            break;
        };
        if v.unspan() != vector.unspan() || m.unspan() != min.unspan() || !plain(val) {
            break;
        }
        pushed.push(val);
        last = after - 1;
        next = after;
    }
    (pushed.len() >= 2).then_some(PushRun {
        vector,
        min,
        vals: pushed,
        last,
    })
}

/// Recognise [`RepeatLiteral`] in statement `first` followed by statement `second`.
#[must_use]
pub fn repeat_literal<'a>(
    first: &'a Value,
    second: &'a Value,
    data: &Data,
) -> Option<RepeatLiteral<'a>> {
    let Value::Insert(ops) = first.unspan() else {
        return None;
    };
    let [pre, push] = &ops[..] else { return None };
    let named = |v: &'a Value, n: &str| -> Option<&'a [Value]> {
        match v.unspan() {
            Value::Call(d, a)
                if (*d as usize) < data.definitions.len() && data.def(*d).name() == n =>
            {
                Some(a)
            }
            _ => None,
        }
    };
    let [p0, Value::Int(n0), Value::Int(w)] = named(pre, "OpPreAllocVector")? else {
        return None;
    };
    let Value::Call(pd, pargs) = push.unspan() else {
        return None;
    };
    let pname = data.def(*pd).name();
    let (rust_type, size) = match pname {
        "OpPushInt" => ("i64", 8),
        "OpPushFloat" => ("f64", 8),
        "OpPushSingle" => ("f32", 4),
        _ => return None,
    };
    let [p1, val] = &pargs[..] else { return None };
    if !matches!(
        val.unspan(),
        Value::Int(_) | Value::Long(_) | Value::Float(_) | Value::Single(_)
    ) {
        return None;
    }
    let [p2, Value::Int(n2), _] = named(second, "OpAppendCopy")? else {
        return None;
    };
    if *n0 != *n2 || i64::from(*w) != i64::from(size) || p0 != p1 || p0 != p2 {
        return None;
    }
    vector_path(data, p0)?;
    Some(RepeatLiteral {
        vector: p0,
        count: i64::from(*n0),
        val,
        rust_type,
        size,
    })
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
    /// The guarded clause's bound N: the body was `t = base + i; if t < N { v[t] = val }`,
    /// so the loop writes exactly the indexes below N.
    pub bound: Option<&'a Value>,
}

// The shape of a counted `for` loop is read in the core layer (`loop_shape`), because
// the slot allocator and the validator ask it too; these are its paths here.
pub use crate::loop_shape::{RangeCounters, plain_for_body, range_counters};

pub use crate::opt::iteration::{VectorIteration, element_binding, iteration_head};

pub use crate::opt::text_borrow::{SplitTable, TextBorrow, borrowed_text_walks, text_walk_head};
pub(crate) use crate::opt::text_borrow::{as_emitted_by_tables, text_escapes, text_escapes_with};

pub use crate::opt::char_walk::{CharWalk, char_walks, text_written};

/// `@FR-R-TextBorrow`'s versioned clause — the loop variables of text walks that borrow
/// their element in `@FR-R-Alias`'s versioned copy only: [`borrowed_text_walks`] declined
/// the walk for writing a store, every writer in its rest names a store this frame minted
/// or the hidden return buffer (or a local bound from it), and the walked vector's dep chain
/// ends at a PARAMETER.  The copy runs only when that parameter's store is not the return
/// buffer's, so no write of the loop can grow, free or rewrite the store the element's bytes
/// live in — the walk clause's own condition, proven by the copy's run-time test instead of
/// by the absence of writes.  Answers each loop variable with that parameter and the borrow.
#[must_use]
pub fn versioned_text_walks(
    data: &Data,
    def_nr: u32,
    sliced: &BTreeSet<u16>,
    tables: &BTreeMap<u16, SplitTable>,
) -> HashMap<u16, (u16, TextBorrow)> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_TEXT_BORROW").is_ok();
    let mut out = HashMap::new();
    let Some(rb) = retbuf_var(data, def_nr) else {
        return out;
    };
    if body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return out;
    }
    let minted = frame_minted_locals(data, vars, body, &[rb]);
    // The parameter a variable's dep chain ends at, through single-dep views.
    let root = |v: u16| -> Option<u16> {
        let mut cur = v;
        for _ in 0..16 {
            if vars.is_argument(cur) {
                return Some(cur);
            }
            let [d] = vars.tp(cur).depend()[..] else {
                return None;
            };
            cur = d;
        }
        None
    };
    let mut cache: HashMap<u32, bool> = HashMap::new();
    let mut active: HashSet<u32> = HashSet::new();
    body.any_node(&mut |n| {
        let Value::Loop(lp) = n else {
            return false;
        };
        if lp.name != "For loop" || lp.operators.is_empty() {
            return false;
        }
        let Some((p, _index, vec, read)) = text_walk_head(&lp.operators[0], data) else {
            return false;
        };
        if sliced.contains(&vec) || !matches!(vars.tp(p).base(), Type::Text(_)) {
            return false;
        }
        let rest: Vec<Value> = lp.operators[1..]
            .iter()
            .map(|s| as_emitted_by_tables(s, tables, data))
            .collect();
        if !rest.iter().any(|s| may_write_store(s, data, &mut cache)) {
            return false; // the walk clause's own: no copy needed
        }
        let verdict = match root(vec) {
            Some(r) if r != rb && crate::data::is_dbref(vars.tp(r).base()) => {
                if let Some(why) = lp.operators[1..]
                    .iter()
                    .find_map(|s| text_escapes(s, p, data))
                {
                    Err(why)
                } else if let Some(why) = rest.iter().find_map(|s| {
                    foreign_store_writer(s, data, &minted, &mut cache, &mut active, Some(vars))
                }) {
                    Err(why)
                } else {
                    Ok(r)
                }
            }
            _ => Err("the walked vector is no parameter's"),
        };
        match verdict {
            Ok(r) => {
                if trace {
                    eprintln!(
                        "[text-borrow] {}: `{}` borrows its element in the copy versioned on `{}`",
                        def.name(),
                        vars.name(p),
                        vars.name(r)
                    );
                }
                out.insert(
                    p,
                    (
                        r,
                        TextBorrow {
                            read: Some(read.clone()),
                        },
                    ),
                );
            }
            Err(why) if trace => {
                eprintln!(
                    "[text-borrow] {}: `{}` no versioned borrow — {why}",
                    def.name(),
                    vars.name(p)
                );
            }
            Err(_) => {}
        }
        false
    });
    out
}

/// `@FR-R-TextBorrow`'s discharge clause — the `__ncc_N` temps of this function that hold a
/// split TABLE's element read for a `?` / `??` discharge (`parts[i]?`, `parts[i] ?? d`),
/// where every mention of the temp is a text-value read: the temp BORROWS the slice the
/// read answers.  The slice points into the table's source — a borrowed parameter or the
/// block-long copy — which outlives the temp's block and every consumer of the discharge's
/// value (an expression of the same statement), so no store condition applies, as for a
/// walk over a sliced vector.  The temp's own bind is the one `Set` admitted; a second
/// bind, a link, a capture or a return declines it, as for a loop variable.
#[must_use]
pub fn borrowed_discharge_temps(
    data: &Data,
    def_nr: u32,
    tables: &BTreeMap<u16, SplitTable>,
) -> HashMap<u16, TextBorrow> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_TEXT_BORROW").is_ok();
    let mut out = HashMap::new();
    if tables.is_empty() || body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return out;
    }
    let names: BTreeSet<u16> = tables
        .iter()
        .flat_map(|(t, st)| std::iter::once(*t).chain(st.aliases.iter().copied()))
        .collect();
    let table_read = |to: &Value| {
        call_named(to, data, "OpGetText")
            .and_then(|a| a.first())
            .and_then(|elem| {
                call_named(elem, data, "OpGetVectorNullable")
                    .or_else(|| call_named(elem, data, "OpGetVector"))
            })
            .and_then(|inner| as_var(inner.first()))
            .is_some_and(|v| names.contains(&v))
    };
    body.any_node(&mut |n| {
        let Value::Set(v, to) = n else {
            return false;
        };
        if !vars.name(*v).starts_with("__ncc_")
            || !matches!(vars.tp(*v).base(), Type::Text(_))
            || !table_read(to)
        {
            return false;
        }
        let mut binds = 0u32;
        body.any_node(&mut |m| {
            if matches!(m, Value::Set(x, t) if x == v && !matches!(t.unspan(), Value::Null)) {
                binds += 1;
            }
            false
        });
        let verdict = if binds != 1 {
            Err("bound more than once")
        } else if let Some(why) = text_escapes_with(body, *v, data, &table_read) {
            Err(why)
        } else {
            Ok(())
        };
        match verdict {
            Ok(()) => {
                out.insert(*v, TextBorrow { read: None });
                if trace {
                    eprintln!(
                        "[text-borrow] {}: `{}` borrows the table's slice",
                        def.name(),
                        vars.name(*v)
                    );
                }
            }
            Err(why) if trace => {
                eprintln!(
                    "[text-borrow] {}: `{}` declined — {why}",
                    def.name(),
                    vars.name(*v)
                );
            }
            Err(_) => {}
        }
        false
    });
    out
}

pub use crate::opt::iteration::bounded_iteration_index;

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
    let Some(body) = plain_for_body(lp) else {
        return Err("the loop is not `[iterator, body block]`".to_string());
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

/// `(R-BoundedNest)`'s reduction clause — a counted loop that is ONE integer accumulate of a
/// scalar vector's elements, `acc = acc + v[i]` (the stdlib `sum`'s loop): the accumulator,
/// the vector's path and operand, the range's end, and the loop's `#index`.
pub struct BoundedSum<'a> {
    pub acc: u16,
    pub path: PathKey,
    pub vector: &'a Value,
    pub index: u16,
    pub hi: &'a Value,
}

/// Recognise the reduction: a counted, EXCLUSIVE range in the single-counter form (its
/// `#index` seeded one below a literal start, so the prelude can advance that one counter
/// and the checked loop resumes from it), whose body is exactly `Set(acc, OpAddInt(Var(acc),
/// read))` — or the operands the other way round — where `read` is an 8-byte `OpGetInt` at
/// field 0 of `v[i]` over a pure path with `i` the loop variable.  Shape only: the emitter
/// confirms the loop holds the path's header AND base, since the block sum reads through the
/// base.  A `?`-discharged read is not this shape (its operand is a join), a float accumulate
/// is not (a float `+` mints no null and would need reassociation to vectorise at all), and a
/// second statement, a computed index or a start the two-counter form carries all answer
/// `Err` — the loop keeps its checked form, which costs the block sum and never a value.
///
/// # Errors
///
/// The reason the loop is not the reduction — what `LOFT_TRACE_NEST=1` prints.
pub fn bounded_sum<'a>(lp: &'a Block, data: &Data) -> Result<BoundedSum<'a>, String> {
    if lp.name != "For loop" {
        return Err("not a for loop".to_string());
    }
    let rc = range_counters(lp, data)?;
    if rc.inclusive {
        return Err("an inclusive range".to_string());
    }
    if rc.next.is_some() {
        return Err("a computed start (the two-counter form)".to_string());
    }
    let Some(body) = plain_for_body(lp) else {
        return Err("the loop is not `[iterator, body block]`".to_string());
    };
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
    if *acc == rc.loop_var || *acc == rc.index {
        return Err("the accumulator is a counter".to_string());
    }
    let Value::Call(d, args) = rhs.unspan() else {
        return Err("the assignment is not an add".to_string());
    };
    if data.def(*d).name() != "OpAddInt" || args.len() != 2 {
        return Err("the assignment is not an integer add".to_string());
    }
    let is_acc = |v: &Value| matches!(v.unspan(), Value::Var(a) if a == acc);
    let read = if is_acc(&args[0]) {
        &args[1]
    } else if is_acc(&args[1]) {
        &args[0]
    } else {
        return Err("neither operand is the accumulator".to_string());
    };
    let Value::Call(g, gargs) = read.unspan() else {
        return Err("the term is not a read".to_string());
    };
    let Some(fused) = fused_element_read(data, data.def(*g).name(), gargs) else {
        return Err("the term is not a fused element read".to_string());
    };
    if fused.rust_type != "i64" {
        return Err("the element is not an integer".to_string());
    }
    if !matches!(fused.size.unspan(), Value::Int(8)) || !matches!(fused.fld.unspan(), Value::Int(0))
    {
        return Err("the element is not an 8-byte scalar at field 0".to_string());
    }
    if !matches!(fused.index.unspan(), Value::Var(i) if *i == rc.loop_var) {
        return Err("the index is not the loop variable".to_string());
    }
    if fused.path.0 == *acc {
        return Err("the vector is the accumulator".to_string());
    }
    Ok(BoundedSum {
        acc: *acc,
        path: fused.path,
        vector: fused.vector,
        index: rc.index,
        hi: rc.hi,
    })
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

/// Recognise the fill idiom in a `For loop` body (@PLN157 § V-ae): the iterator block of a
/// counted range in either lowering — the two-counter form `[if hi <cmp> next { break };
/// index = next; next = next + 1; index]` (§ V-ab) or the single-counter form `[index =
/// index + 1; if hi <cmp> index { break }; index]` (P3b) — followed by ONE statement, a
/// fusable scalar set (`OpSetInt`/`OpSetSingle`/`OpSetFloat`) at field 0 of `v[idx]` with
/// `v` a pure path, `idx` the loop variable or `invariant + variable`, and the value
/// invariant.  `<cmp>` is `OpLtInt` for an inclusive range and `OpLeInt` for an exclusive
/// one.  Reports shape only; the emitter confirms the path has a header.
/// Enforces `@FR-R-Fill`.
#[expect(clippy::too_many_lines, reason = "inherited")]
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
    let Some(body) = plain_for_body(lp) else {
        return decline("the loop is not `[iterator, body block]`");
    };
    // A source-line marker is not a statement (a library module's body carries one).
    let stmts: Vec<&Value> = body
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    // `@FR-R-Fill`'s guarded clause: `t = <index>; if t < N { <the setter at v[t]> }`, with
    // `t` the body's own local.  The setter's index is then `t`, and the index proper is
    // what `t` was bound to.
    let mut guarded: Option<(u16, &'a Value, &'a Value)> = None;
    let setter_stmt: &'a Value = match stmts.as_slice() {
        [one] => one,
        [bind, test] => {
            let (Value::Set(t, bound_idx), Value::If(cond, then, other)) =
                (bind.unspan(), test.unspan())
            else {
                return decline("the two statements are not `t = index; if t < N { … }`");
            };
            let Value::Call(lt, largs) = cond.unspan() else {
                return decline("the guard is not a comparison");
            };
            if data.def(*lt).name() != "OpLtInt"
                || largs.len() != 2
                || !matches!(largs[0].unspan(), Value::Var(x) if x == t)
                || !matches!(other.unspan(), Value::Null)
            {
                return decline("the guard is not `t < N` with no else");
            }
            let inner: Vec<&Value> = match then.unspan() {
                Value::Block(b) => b
                    .operators
                    .iter()
                    .filter(|o| !matches!(o.unspan(), Value::Line(_)))
                    .collect(),
                other => vec![other],
            };
            let [only] = inner.as_slice() else {
                return decline("the guarded arm is not one statement");
            };
            guarded = Some((*t, &**bound_idx, &largs[1]));
            only
        }
        _ => {
            return decline(&format!(
                "the body has {} statements: {}",
                stmts.len(),
                kinds(&body.operators)
            ));
        }
    };
    let Value::Call(setter, wargs) = setter_stmt.unspan() else {
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
    let index: &Value = match guarded {
        Some((t, bound_idx, _)) => {
            if !matches!(index.unspan(), Value::Var(x) if *x == t) {
                return decline("the guarded setter's index is not the bound local");
            }
            banned.push(t);
            bound_idx
        }
        None => index,
    };
    // The index: the loop variable, or `invariant + variable` either way round.  The
    // nullable add answers null on overflow, where the fill's own checked add refuses and
    // the loop runs: the same writes.
    let base = match index.unspan() {
        Value::Var(x) if *x == loop_var => None,
        Value::Call(d, args)
            if matches!(data.def(*d).name(), "OpAddInt" | "OpAddIntNullable")
                && args.len() == 2 =>
        {
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
    let bound = match guarded {
        Some((_, _, n)) if simple_invariant(n, data, &banned) => Some(n),
        Some(_) => return decline("the guard's bound is not a simple invariant"),
        None => None,
    };
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
        bound,
    })
}

/// `@FR-R-PushFill`'s several-paths clause — a counted loop whose top-level pushes reach
/// MORE than one path (`for _ in 0..n { m += [0 as u8]; sf += [0 as i32]; }`): the reserve
/// form holds per path, each judged by [`push_loop`]'s own rules with the other paths'
/// pushes read as plain statements.  `None` unless there are two or more paths and every
/// one qualifies.  No fill and no window: each is one path's form.
#[must_use]
pub fn push_loop_paths<'a>(lp: &'a Block, data: &Data) -> Option<Vec<PushLoop<'a>>> {
    let body = push_loop_body(lp)?;
    let mut paths: Vec<PathKey> = Vec::new();
    for s in &body {
        if let Value::Call(d, args) = s.unspan()
            && (*d as usize) < data.definitions.len()
            && FUSABLE_PUSHES
                .iter()
                .any(|(n, _, _)| *n == data.def(*d).name())
            && let Some((vec_arg, _, _)) = push_operands(data.def(*d).name(), args)
            && let Some(p) = vector_path(data, vec_arg)
            && !paths.contains(&p)
        {
            paths.push(p);
        }
    }
    if paths.len() < 2 {
        return None;
    }
    // Two pushes rooted at one variable may name one vector by two spellings; distinct
    // roots name distinct locals, and only a path's own pushes are counted against it.
    let roots: HashSet<u16> = paths.iter().map(|p| p.0).collect();
    if roots.len() != paths.len() {
        return None;
    }
    paths
        .iter()
        .map(|p| push_loop_on(lp, data, Some(p)))
        .collect::<Option<Vec<_>>>()
        .map(|mut v| {
            for p in &mut v {
                p.fill = None;
            }
            v
        })
}

/// `@FR-R-PushFill`'s several-paths WINDOW clause — [`push_window_ok`] for the paths of
/// [`push_loop_paths`] TOGETHER: every root exclusive, and the body's parts beside the pushes
/// (each push of ANY path contributes its value operand alone) name no root and no variable
/// that may view a root's store, and write no store.  Every path is reserved before any
/// header is re-derived, and each path's pushes are counted and unconditional, so no push
/// outgrows its window: no store reallocates while the windows are open, which is what each
/// window's address stands on — two windows into ONE store included.
///
/// # Errors
///
/// The condition that does not hold — what `LOFT_TRACE_PUSH_FILL=1` prints.  The caller
/// keeps the header pushes, which costs the windows and never a value.
pub fn push_windows_ok(
    lp: &Block,
    ps: &[PushLoop],
    data: &Data,
    def_nr: u32,
    stores: Option<&Stores>,
    cache: &mut HashMap<u32, bool>,
) -> Result<(), String> {
    let vars = data.def(def_nr).variables();
    let retbuf = retbuf_var(data, def_nr);
    let roots: Vec<u16> = ps.iter().map(|p| p.path.0).collect();
    for &root in &roots {
        if root >= vars.count()
            || !(owned_local(vars, root)
                || retbuf == Some(root)
                || work_buffer_arg(data, def_nr, root))
        {
            return Err(format!(
                "the pushed vector's root `{}` is not exclusive",
                vars.name(root)
            ));
        }
    }
    let Some(body) = push_loop_body(lp) else {
        return Err("not a counted push loop".to_string());
    };
    let mut parts: Vec<&Value> = Vec::new();
    for s in body {
        match s.unspan() {
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && args
                        .first()
                        .and_then(|f| vector_path(data, f))
                        .is_some_and(|fp| ps.iter().any(|p| p.path == fp))
                    && (FUSABLE_PUSHES
                        .iter()
                        .any(|(n, _, _)| *n == data.def(*d).name())
                        || data.def(*d).name() == "OpPreAllocVector") =>
            {
                parts.extend(args.iter().skip(1));
            }
            _ => parts.push(s),
        }
    }
    let any_retbuf = roots.iter().any(|r| retbuf == Some(*r));
    window_parts_ok(&parts, &roots, any_retbuf, vars, data, stores, cache, None)
}

/// `@FR-R-PushFill`'s DEEP window clause — a loop whose pushes to `paths` stand at any depth
/// (under an `if` arm, in an inner loop), with no reservation: a push that fits is the
/// window's store and bump, one that does not takes the window's growth arm, which appends
/// through the runtime and answers a fresh window.  So the window needs only what its
/// address and its deferred length stand on — every root exclusive, the body outside the
/// pushes of these paths naming no root and no variable that may view a root's store and
/// writing no store ([`window_parts_ok`]) — and that the close after the loop runs on
/// every way out: no `return` or `yield` in the body, and no `break` or `continue` that
/// leaves a loop enclosing it.  Without a reservation the growth arm can run, and a growth
/// moves every address into its store — a second window's too — so the loop pushes to ONE
/// path: a push to any other is a store write the parts test declines.  The growth arm is
/// the header push's own growth step, so what else the loop holds is judged as it is for
/// the header push (`@FR-R-Alias` counts the push a mover either way).
///
/// # Errors
///
/// The condition that does not hold — what `LOFT_TRACE_PUSH_FILL=1` prints.  The caller
/// keeps the header pushes, which costs the window and never a value.
pub fn deep_window_ok(
    lp: &Block,
    path: &PathKey,
    data: &Data,
    def_nr: u32,
    stores: Option<&Stores>,
    cache: &mut HashMap<u32, bool>,
    facts: Option<&StoreFacts>,
) -> Result<(), String> {
    fn collect<'v>(
        n: &'v Value,
        data: &Data,
        path: &PathKey,
        depth: u16,
        parts: &mut Vec<&'v Value>,
    ) -> Result<(), String> {
        match n.unspan() {
            Value::Return(_) | Value::Yield(_) => Err("the body can return".to_string()),
            Value::Break(k) | Value::Continue(k) if *k > depth => {
                Err("a jump leaves a loop enclosing the window".to_string())
            }
            Value::Break(_) | Value::Continue(_) | Value::Line(_) => Ok(()),
            Value::If(c, t, e) => {
                parts.push(c);
                collect(t, data, path, depth, parts)?;
                collect(e, data, path, depth, parts)
            }
            Value::Block(b) => b
                .operators
                .iter()
                .try_for_each(|o| collect(o, data, path, depth, parts)),
            Value::Loop(b) => b
                .operators
                .iter()
                .try_for_each(|o| collect(o, data, path, depth + 1, parts)),
            Value::Insert(ops) => ops
                .iter()
                .try_for_each(|o| collect(o, data, path, depth, parts)),
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && args.first().and_then(|f| vector_path(data, f)).as_ref() == Some(path)
                    && (FUSABLE_PUSHES
                        .iter()
                        .any(|(name, _, _)| *name == data.def(*d).name())
                        || data.def(*d).name() == "OpPreAllocVector") =>
            {
                // A push contributes its value operands alone; the window is its vector.
                parts.extend(args.iter().skip(1));
                Ok(())
            }
            _ => {
                parts.push(n);
                Ok(())
            }
        }
    }
    let vars = data.def(def_nr).variables();
    let root = path.0;
    let root_is_retbuf = retbuf_var(data, def_nr) == Some(root);
    if root >= vars.count()
        || !(owned_local(vars, root) || root_is_retbuf || work_buffer_arg(data, def_nr, root))
    {
        return Err("the pushed vector's root is not exclusive".to_string());
    }
    let mut parts: Vec<&Value> = Vec::new();
    for o in &lp.operators {
        collect(o, data, path, 0, &mut parts)?;
    }
    window_parts_ok(
        &parts,
        &[root],
        root_is_retbuf,
        vars,
        data,
        stores,
        cache,
        facts,
    )
}

/// `@FR-R-PushFill`'s hash clause — a counted loop that inserts records into ONE hash local:
/// the hash, its type, the most inserts one pass runs, and the loop's counters.  Reserving
/// the table for them up front changes capacity only — never `len` or contents — so no
/// program answers differently; a pass whose key is already present inserts nothing, which
/// leaves room unused, bounded by the trip count.
pub struct HashFill {
    pub hash: u16,
    pub tp: u16,
    pub per_pass: u32,
    pub index: u16,
    pub next: Option<u16>,
    pub inclusive: bool,
}

/// [`HashFill`] for `lp`, or why it is no hash-fill loop.
///
/// # Errors
///
/// The reason the loop is no counted fill of one hash, as a trace would name it.
pub fn hash_fill_loop<'a>(
    lp: &'a Block,
    data: &Data,
    stores: &Stores,
) -> Result<(HashFill, &'a Value), &'static str> {
    let body = plain_for_body(lp).ok_or("not a plain counted loop")?;
    let rc = range_counters(lp, data).map_err(|_| "not a range loop")?;
    if leaves_early(body) {
        return Err("the body can leave early, or loops");
    }
    let mint_of = |n: &Value| -> Option<(u16, u16)> {
        let Value::Call(d, args) = n.unspan() else {
            return None;
        };
        if (*d as usize) >= data.definitions.len() || data.def(*d).name() != "OpNewRecord" {
            return None;
        }
        let (Some(Value::Var(h)), Some(Value::Int(tp))) = (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
        ) else {
            return None;
        };
        let tp = u16::try_from(*tp).ok()?;
        matches!(
            stores.types.get(tp as usize).map(|t| &t.parts),
            Some(crate::database::Parts::Hash(..))
        )
        .then_some((*h, tp))
    };
    let mut target: Option<(u16, u16)> = None;
    let mut two = false;
    let mut rebound = false;
    for s in &body.operators {
        s.any_node(&mut |n| {
            if let Some(t) = mint_of(n) {
                match target {
                    Some(old) if old != t => two = true,
                    _ => target = Some(t),
                }
            }
            false
        });
    }
    let (hash, tp) = target.ok_or("no hash insert in the body")?;
    if two {
        return Err("the inserts reach two hashes");
    }
    for s in &body.operators {
        s.any_node(&mut |n| {
            if matches!(n, Value::Set(v, _) if *v == hash) {
                rebound = true;
            }
            false
        });
    }
    if rebound {
        return Err("the hash is rebound in the body");
    }
    let mut banned = vec![rc.loop_var, rc.index, hash];
    if let Some(nx) = rc.next {
        banned.push(nx);
    }
    if !simple_invariant(rc.hi, data, &banned) {
        return Err("the range's end is not a simple invariant");
    }
    fn most(n: &Value, mint_of: &dyn Fn(&Value) -> Option<(u16, u16)>) -> u32 {
        if mint_of(n).is_some() {
            return 1;
        }
        if let Value::If(c, t, e) = n.unspan() {
            return most(c, mint_of) + most(t, mint_of).max(most(e, mint_of));
        }
        let mut total = 0u32;
        n.for_each_child(&mut |child| total += most(child, mint_of));
        total
    }
    let per_pass: u32 = body.operators.iter().map(|s| most(s, &mint_of)).sum();
    if per_pass == 0 {
        return Err("no hash insert in the body");
    }
    Ok((
        HashFill {
            hash,
            tp,
            per_pass,
            index: rc.index,
            next: rc.next,
            inclusive: rc.inclusive,
        },
        rc.hi,
    ))
}

/// Can a statement of `body` leave the loop early, or run a loop of its own?
fn leaves_early(body: &Block) -> bool {
    body.operators.iter().any(|s| {
        s.any_node(&mut |n| {
            matches!(
                n,
                Value::Break(_)
                    | Value::Return(_)
                    | Value::Continue(_)
                    | Value::Loop(_)
                    | Value::Yield(_)
                    | Value::Parallel(_)
            )
        })
    })
}

/// `(R-Mint)`'s rebound clause — is every `Set(root, rhs)` in `body` alias-free: a null, or
/// the field projection of an emitter-owned buffer var (`buffers`: a § V-al loop buffer or a
/// § V-z element-first witness)?  Such a rebind names a fresh store's root or a fresh
/// element's field slot, which no header kept across the body can describe; a `Set` from
/// anything else (`v = w`, a call's result, an element of another vector) may alias a held
/// vector, and the fallback answers NO because the caller then declines the whole loop —
/// the conservative side.
fn rebinds_alias_free(body: &Block, root: u16, data: &Data, owned: Option<&HoistOwned>) -> bool {
    let Some(owned) = owned else { return false };
    let buffers = &owned.buffers;
    !body.operators.iter().any(|op| {
        op.any_node(&mut |n| match n {
            Value::Set(v, rhs) if *v == root => !match rhs.unspan() {
                Value::Null => true,
                Value::Call(d, args) => {
                    (*d as usize) < data.definitions.len()
                        && data.def(*d).name() == "OpGetField"
                        && matches!(args.first().map(Value::unspan), Some(Value::Var(b))
                            if buffers.contains(b))
                }
                _ => false,
            },
            Value::TuplePut(v, _, _) => *v == root,
            _ => false,
        })
    })
}

/// `(R-GroupPush)` — a mint GROUP that holds no push header of its own: the statements
/// `ops[at + 1 ..= end]` after an `OpPreAllocVector(P, n, size)` at `ops[at]`, where P is a
/// bare-variable plain vector (the `(R-Mint)` shape), the group's `n` mints and finishes
/// name P, every mint qualifies for the record push, and every OTHER statement in the range
/// is one the header admission lets through (an in-place set, a store-free builder, a fresh
/// delivery) — so nothing in the range moves P's record but the group's own pushes, which
/// the header follows.  Answers the index of the n-th finish; `None` declines.  The
/// reservation stays: it is what gives the header its capacity.  A rebind of P inside the
/// range, a statement that may grow or clear a store, a mint on P outside the count, or a
/// range that runs out before the n-th finish each decline — the fallback is the template,
/// which resolves per element and is always right.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn mint_group(
    ops: &[Value],
    at: usize,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    cache: &mut HashMap<u32, bool>,
    tiers: HoistTiers,
    owned: Option<&HoistOwned>,
) -> Option<(PathKey, usize)> {
    let vars = data.def(def_nr).variables();
    let Value::Call(d, pargs) = ops.get(at)?.unspan() else {
        return None;
    };
    if (*d as usize) >= data.definitions.len() {
        return None;
    }
    let path = pre_alloc_path(data, data.def(*d).name(), pargs)?;
    if !path.1.is_empty() || path.0 >= vars.count() {
        return None;
    }
    if !matches!(vars.tp(path.0).peel_link(), Type::Vector(_, _)) {
        return None;
    }
    let Some(Value::Int(n)) = pargs.get(1).map(Value::unspan) else {
        return None;
    };
    let n = usize::try_from(*n).ok().filter(|n| *n > 0)?;
    let mut fresh: HashSet<u16> = HashSet::new();
    let mut mints = 0usize;
    let mut finishes = 0usize;
    // The arguments of `inner` when it is one of the group's own mints on P.
    fn group_mint<'v>(
        inner: &'v Value,
        data: &Data,
        stores: &Stores,
        vars: &crate::variables::Function,
        path: &PathKey,
    ) -> Option<&'v [Value]> {
        if let Value::Call(md, margs) = inner.unspan()
            && (*md as usize) < data.definitions.len()
            && data.def(*md).name() == "OpNewRecord"
            && mint_path(data, stores, "OpNewRecord", margs, vars).as_ref() == Some(path)
        {
            Some(margs)
        } else {
            None
        }
    }
    for (i, op) in ops.iter().enumerate().skip(at + 1) {
        if matches!(op, Value::Line(_)) {
            continue;
        }
        if let Value::Set(e, inner) = op.unspan()
            && let Some(margs) = group_mint(inner, data, stores, vars, &path)
        {
            if !mint_target(data, stores, "OpNewRecord", margs, vars)
                .is_some_and(|t| mint_push_qualifies(stores, t.vector_tp, tiers.heap_push))
            {
                return None;
            }
            mints += 1;
            if mints > n {
                return None;
            }
            fresh.insert(*e);
            continue;
        }
        match op.unspan() {
            Value::Call(fd, fargs)
                if (*fd as usize) < data.definitions.len()
                    && data.def(*fd).name() == "OpFinishRecord"
                    && mint_path(data, stores, "OpFinishRecord", fargs, vars).as_ref()
                        == Some(&path) =>
            {
                finishes += 1;
                if let Some(Value::Var(e)) = fargs.get(1).map(Value::unspan) {
                    fresh.remove(e);
                }
                if finishes == n {
                    return (mints == n).then_some((path, i));
                }
                continue;
            }
            n if rebinds_root(n, path.0, vars) => return None,
            _ => {}
        }
        // A statement of the group that is not one of its mints or finishes: admitted on
        // the same terms as a loop body's — and a mint or finish on P NESTED in it (inside an
        // `if`) is one the count above cannot see, so it declines.
        if blocks_header_hoist(
            op,
            data,
            Some(stores),
            cache,
            &mut HashSet::new(),
            Some(vars),
            tiers,
            &mut fresh,
            owned,
        )
            || op.any_node(&mut |x| matches!(x, Value::Call(xd, xargs)
                if (*xd as usize) < data.definitions.len()
                    && matches!(data.def(*xd).name(), "OpNewRecord" | "OpFinishRecord" | "OpPreAllocVector")
                    && vector_path(data, xargs.first().unwrap_or(&Value::Null)).as_ref() == Some(&path)))
        {
            return None;
        }
    }
    None
}

/// A USER struct-enum view, for the record ADDRESS (`@FR-R-RecPtr`'s enum clause): the tag
/// byte at 0 and the variant's fields behind it, each read at the offset the parser
/// resolved for the arm that reads it — so the address needs no layout of its own.
/// `(R-Scalar)`'s hoisted scalars keep [`plain_record_type`]: a key of (type, offset)
/// cannot tell one variant's field from another's at the same offset.  The synthetic
/// `__nullable<S>` stays out — its absence is a discriminant, not a null record.
fn struct_enum_view(data: &Data, tp: &Type) -> bool {
    if !enum_record_enabled() {
        return false;
    }
    let d = match tp.peel_link() {
        Type::Enum(d, true, _) => *d,
        Type::Reference(d, _) if data.def_type(*d) == DefType::Enum => *d,
        _ => return false,
    };
    !data.def(d).name().starts_with("__nullable<")
}

const FUSABLE_SETTERS: [(&str, &str); 4] = [
    ("OpSetInt", "i64"),
    ("OpSetSingle", "f32"),
    ("OpSetFloat", "f64"),
    // A struct-enum's TAG (and any value-enum field): one byte, its null 255 — the byte
    // `set_byte(…, 0, v)` writes for a `u8` operand, so the store through the address is
    // the same write (`@FR-R-RecPtr`'s enum clause; `enum_match`'s build paid a second
    // store lookup per element for the tag alone).
    ("OpSetEnum", "u8"),
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
/// `(R-Header)`'s FUNCTION clause — the vector PARAMETERS whose header the body may hold
/// from entry: read twice or more through an element address, and left as they were found
/// — never written, freed or rebound by the body, nor by any call it makes, a callee asked
/// the same question of the position the parameter reaches and a recursive call answering
/// for itself ([`param_untouched`]).  Growth of OTHER stores is no concern of a header (its
/// `rec` and `len` are the vector's own, addressed through the store), so a body that
/// builds its result beside the reads keeps them; only a BASE would need the store's
/// memory still, and the clause binds none.  One read gains nothing: deriving the header
/// is that read's own work.
pub fn fn_header_params(
    body: &Block,
    data: &Data,
    def_nr: u32,
    memo: &mut HashMap<(u32, u16), bool>,
) -> Vec<(PathKey, Value)> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let Ok(params) = u16::try_from(def.attributes().len()) else {
        return Vec::new();
    };
    let mut counts: Vec<(u16, usize, Value)> = Vec::new();
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && args.len() == 3
                && is_element_address(data, *d)
                && let Some((root, offs)) = vector_path(data, &args[0])
                && offs.is_empty()
                && root < params
                && matches!(vars.tp(root).base(), Type::Vector(_, _))
            {
                match counts.iter_mut().find(|(r, _, _)| *r == root) {
                    Some(e) => e.1 += 1,
                    None => counts.push((root, 1, args[0].clone())),
                }
            }
            false
        });
    }
    let trace = std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok();
    let mut out = Vec::new();
    for (root, n, expr) in counts {
        if n < 2 {
            continue;
        }
        if param_untouched(data, def_nr, root, memo, &mut HashSet::new()) {
            out.push(((root, Vec::new()), expr));
        } else if trace {
            eprintln!(
                "hoist: {} function header for {} declined: the body or a call may write it (params {:?})",
                def.name(),
                vars.name(root),
                def.attributes()
                    .iter()
                    .map(|a| a.name.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }
    out
}

/// Does function `d_nr` leave the vector its parameter `p` names as it found it — the same
/// record, the same length — on every path?  A rebind of `p`, a native op that is not a
/// reader with `p` as an operand, a free, a fn-ref call, a parallel arm and a `yield` answer
/// no.  A loft-defined callee is asked the same of every position `p` reaches — and of
/// every position a heap value that is not this frame's own reaches, since that value may
/// name `p`'s store — with a recursion assuming the answer it is computing: a body whose
/// only writes to `p` would come through its own recursion has none.  Memoised per
/// `(function, parameter)`.
pub fn param_untouched(
    data: &Data,
    d_nr: u32,
    p: u16,
    memo: &mut HashMap<(u32, u16), bool>,
    active: &mut HashSet<(u32, u16)>,
) -> bool {
    if let Some(&known) = memo.get(&(d_nr, p)) {
        return known;
    }
    if !active.insert((d_nr, p)) {
        return true;
    }
    let def = data.def(d_nr);
    let vars = def.variables();
    let params = u16::try_from(def.attributes().len()).unwrap_or(u16::MAX);
    let answer = match def.code().unspan() {
        Value::Block(body) => {
            let mut ok = true;
            for op in &body.operators {
                op.any_node(&mut |n| {
                    if !ok {
                        return true;
                    }
                    match n {
                        Value::Set(v, _) | Value::TuplePut(v, _, _) if *v == p => ok = false,
                        Value::Call(d, args) => {
                            ok = call_leaves_param(
                                data, d_nr, *d, args, p, params, vars, memo, active,
                            );
                            if !ok && std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
                                eprintln!(
                                    "hoist: {} parameter {} touched by {}",
                                    def.name(),
                                    vars.name(p),
                                    if (*d as usize) < data.definitions.len() {
                                        data.def(*d).name()
                                    } else {
                                        "?"
                                    }
                                );
                            }
                        }
                        Value::CallRef(..) | Value::Parallel(_) | Value::Yield(_) => ok = false,
                        _ => {}
                    }
                    !ok
                });
                if !ok {
                    break;
                }
            }
            ok
        }
        _ => false,
    };
    active.remove(&(d_nr, p));
    memo.insert((d_nr, p), answer);
    answer
}

/// A LOCAL — not one of the first `params` variables, the parameters, unless a work buffer
/// (`@FR-R-WorkBuffer`: a local promoted to a scratch store the caller hands this call
/// alone) — that owns the store it names: its deps name its own `__vdb_` witness, it is no
/// link and no closure capture.  Its store is fresh to this frame, so it cannot be a
/// parameter's.
pub(crate) fn local_owns_store(
    data: &Data,
    d_nr: u32,
    vars: &crate::variables::Function,
    params: u16,
    x: u16,
) -> bool {
    (x >= params || work_buffer_arg(data, d_nr, x))
        && !matches!(vars.tp(x).base(), Type::RefVar(_))
        && !vars.is_captured(x)
        && vars
            .tp(x)
            .depend()
            .iter()
            .all(|d| vars.name(*d).starts_with("__vdb"))
}

/// Is `a` the vector `p` names, or a place inside its record (`OpGetField(p, …)`)?  A
/// VALUE computed from `p` — an element read, a length — is not: it reaches no one who
/// could write the vector.
fn is_path_of(data: &Data, a: &Value, p: u16) -> bool {
    vector_path(data, a).is_some_and(|(root, _)| root == p)
}

/// [`param_untouched`]'s verdict for one call: a loft-defined callee answers for the
/// positions `p` — or a heap value that may name `p`'s store — reaches; a native reads `p`
/// only when it is a listed reader, a copy FROM `p`, or a routine whose every collection
/// parameter is `const`.
#[allow(clippy::too_many_arguments)]
fn call_leaves_param(
    data: &Data,
    d_nr: u32,
    d: u32,
    args: &[Value],
    p: u16,
    params: u16,
    vars: &crate::variables::Function,
    memo: &mut HashMap<(u32, u16), bool>,
    active: &mut HashSet<(u32, u16)>,
) -> bool {
    if (d as usize) >= data.definitions.len() {
        return false;
    }
    let callee = data.def(d);
    if callee.is_loft_defined() {
        for (i, a) in args.iter().enumerate() {
            let Ok(pos) = u16::try_from(i) else {
                return false;
            };
            // A heap value that is not this frame's own may name `p`'s store: a parameter, a
            // view, a link.  A path over an OWNED local names that local's fresh store.
            let reaches = is_path_of(data, a, p)
                || match (a.unspan(), vector_path(data, a)) {
                    (_, Some((root, _))) => {
                        !is_scalar(vars.tp(root))
                            && !matches!(vars.tp(root).base(), Type::Text(_))
                            && !local_owns_store(data, d_nr, vars, params, root)
                    }
                    (Value::Var(x), None) => {
                        !is_scalar(vars.tp(*x))
                            && !matches!(vars.tp(*x).base(), Type::Text(_))
                            && !local_owns_store(data, d_nr, vars, params, *x)
                    }
                    _ => callee.attributes().get(i).is_none_or(|at| {
                        !is_scalar(&at.typedef) && !matches!(at.typedef.base(), Type::Text(_))
                    }),
                };
            if reaches && !param_untouched(data, d, pos, memo, active) {
                if std::env::var("LOFT_TRACE_HOIST_DECLINE").is_ok() {
                    let shown = format!("{:?}", a.unspan());
                    eprintln!(
                        "hoist:   arg {i} of {} reaches it: {} (argument={})",
                        callee.name(),
                        &shown[..shown.len().min(160)],
                        match a.unspan() {
                            Value::Var(x) => vars.is_argument(*x),
                            _ => false,
                        }
                    );
                }
                return false;
            }
        }
        return true;
    }
    let name = callee.name();
    let const_collections = !callee.attributes().is_empty()
        && callee.attributes().iter().all(|at| {
            is_scalar(&at.typedef) || matches!(at.typedef.base(), Type::Text(_)) || at.value_const
        });
    for (i, a) in args.iter().enumerate() {
        if !is_path_of(data, a, p) {
            continue;
        }
        // An append READS its source — the whole vector, or a range of it (`t += v[lo..hi]`,
        // `(R-ByteCopy)`'s slice) — and writes only its destination, argument 0.
        let reads = READ_ONLY_COLLECTION_OPS.contains(&name)
            || (i > 0 && matches!(name, "OpAppendVector" | "OpSliceVector"))
            || const_collections;
        if !reads {
            return false;
        }
    }
    true
}

/// The ROOT variable of every text set in `body` — the store each one grows — or `None` when
/// one's target has a root this cannot name.  A target is a variable or a projection of
/// one (an element, a field), whose store is its root's (`StoreFacts::distinct` reads the
/// root's dep chain).
fn text_set_roots(body: &Block, data: &Data) -> Option<Vec<u16>> {
    fn root(v: &Value, data: &Data) -> Option<u16> {
        match v.unspan() {
            Value::Var(r) => Some(*r),
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && matches!(
                        data.def(*d).name(),
                        "OpGetVector" | "OpGetVectorNullable" | "OpGetField"
                    ) =>
            {
                root(args.first()?, data)
            }
            _ => None,
        }
    }
    let mut roots: Vec<u16> = Vec::new();
    let mut known = true;
    for op in &body.operators {
        op.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
                && is_text_set(data.def(*d).name())
            {
                match args.first().and_then(|t| root(t, data)) {
                    Some(r) if !roots.contains(&r) => roots.push(r),
                    Some(_) => {}
                    None => known = false,
                }
            }
            false
        });
    }
    known.then_some(roots)
}

/// The split tables of `def_nr`'s body, keyed by the local the call's result is bound to
/// (`@FR-R-SplitTable`).
///
/// `parts = src.split(c)` followed by `len(parts)`, `parts[i]` and `for p in parts` is the
/// shape [`lazy_splits`] declines — the vector bound to a name — and the one a library
/// takes when it needs the count or the i-th piece.  Nothing but those readers can name
/// the local, so one pass over the text records its pieces as a table of slices and each
/// reader answers from the table: the same pieces, the same count, the same element at
/// each index.  Every gate is an under-approximation — a declined bind keeps the vector,
/// which is always correct:
///
/// - the call is the STANDARD LIBRARY's `split(self: text, separator: character)` with a
///   character CONSTANT other than the null character ([`lazy_splits`]' conditions), bound
///   to a plain local `vector<text>` that is not a parameter and not the hidden vector of
///   a `for … in T.split(c)` — that one is `(R-LazySplit)`'s;
/// - that bind is the local's ONLY binding, and every other mention is one of the three
///   reads the emitter answers ([`table_mentions`]): the length, the element read, or the
///   walk's alias bind — whose own mentions are the same two reads and nothing else;
/// - every mention stands inside the block that holds the bind, after it: the table is a
///   Rust `let` at the bind, and a mention outside that block would not compile;
/// - the function is not a generator: the table is a local of the activation, not of the
///   state machine;
/// - `len` reaches the emitter as its op through `(R-Wrapper)`, so with the wrapper kept a
///   call (`LOFT_NO_WRAPPER_INLINE`) no table is admitted at all.
///
/// The source text needs no gate: the emitter borrows it where nothing can write it and
/// copies it once at the bind everywhere else (`Output::lazy_split_borrows`).
#[must_use]
pub fn split_tables(data: &Data, def_nr: u32) -> BTreeMap<u16, SplitTable> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let body = def.code();
    let trace = std::env::var("LOFT_TRACE_SPLIT_TABLE").is_ok();
    let mut out: BTreeMap<u16, SplitTable> = BTreeMap::new();
    if body.any_node(&mut |n| matches!(n, Value::Yield(_))) {
        return out;
    }
    if std::env::var("LOFT_NO_WRAPPER_INLINE").is_ok_and(|v| v != "0") {
        if trace {
            eprintln!(
                "[split-table] {}: no table — `len` stays a call under LOFT_NO_WRAPPER_INLINE",
                def.name()
            );
        }
        return out;
    }
    // Every bind of a plain local from a constant split, with its buffer and separator.
    let mut binds: Vec<(u16, u16, char)> = Vec::new();
    body.any_node(&mut |n| {
        if let Value::Set(v, call) = n
            && let Ok((buf, separator)) = split_call(call, data, vars)
            && !vars.is_argument(*v)
            && !vars.name(*v).starts_with("_vector_")
            && matches!(vars.tp(*v), Type::Vector(el, _) if matches!(el.base(), Type::Text(_)))
        {
            binds.push((*v, buf, separator));
        }
        false
    });
    for (v, buf, separator) in binds {
        let mut m = TableMentions::default();
        let verdict = table_mentions(body, v, data, vars, &mut m)
            .and_then(|()| {
                if m.binds != 1 {
                    return Err(format!("`{}` is bound {} times", vars.name(v), m.binds));
                }
                Ok(())
            })
            .and_then(|()| {
                // Every alias reads the table the way the walk does, and nothing else.
                for a in &m.aliases {
                    let mut am = TableMentions::default();
                    table_mentions(body, *a, data, vars, &mut am)?;
                    if am.binds != 1 || !am.aliases.is_empty() {
                        return Err(format!(
                            "`{}` is not a plain walk of the table",
                            vars.name(*a)
                        ));
                    }
                }
                Ok(())
            })
            .and_then(|()| bind_block_holds_every_mention(body, v));
        match verdict {
            Ok(()) => {
                let dead_buf = buffer_serves_one_call(body, data, buf).then_some(buf);
                if trace {
                    eprintln!(
                        "[split-table] {}: `{}` is a table, separator {separator:?}, {} reads, {} walks, buffer {} {}",
                        def.name(),
                        vars.name(v),
                        m.reads,
                        m.aliases.len(),
                        vars.name(buf),
                        if dead_buf.is_some() {
                            "never minted"
                        } else {
                            "shared, keeps its mint"
                        }
                    );
                }
                out.insert(
                    v,
                    SplitTable {
                        dead_buf,
                        separator,
                        aliases: m.aliases,
                    },
                );
            }
            Err(why) => {
                if trace {
                    eprintln!(
                        "[split-table] {}: `{}` declined — {why}",
                        def.name(),
                        vars.name(v)
                    );
                }
            }
        }
    }
    out
}

/// What [`table_mentions`] counted for one local.
#[derive(Default)]
struct TableMentions {
    /// Bindings of the local from a value (a `= null` declaration is not one).
    binds: u32,
    /// Length and element reads.
    reads: u32,
    /// Hidden `_vector_N` locals bound from it — the walks `for p in V`.
    aliases: Vec<u16>,
}

/// Walk `node` classifying every mention of `v` as one the split table answers, or answer
/// why the first other mention is not.  Walks the tree by hand because the question is
/// about a mention's PARENT: `Var(v)` as the vector of `OpLengthVector` is a read the table
/// answers, and the same `Var(v)` as a call's argument is the vector handed away.
///
/// The reads: `OpLengthVector(v)` — or the stdlib wrapper `(R-Wrapper)` emits as that op —
/// and the element read `OpGetText(OpGetVectorNullable(v, _, i), _)` or its raising twin
/// `OpGetVector`, which is what every spelling of `v[i]` lowers to (bare, under `?`, under
/// `?? d`, bound to a `text?`; the parser picks the twin by the site).  The
/// alias: `Set(_vector_N, Var(v))`, the parser's first statement of `for p in v`.  The
/// bind: `Set(v, <split call>)`; a `Set(v, Null)` is the hoisted null declaration and
/// binds nothing.
fn table_mentions(
    node: &Value,
    v: u16,
    data: &Data,
    vars: &crate::variables::Function,
    m: &mut TableMentions,
) -> Result<(), String> {
    let is_v = |x: &Value| matches!(x.unspan(), Value::Var(u) if *u == v);
    match node.unspan() {
        Value::Var(u) if *u == v => Err(format!(
            "`{}` is read outside a table position",
            vars.name(v)
        )),
        Value::Set(x, to) => {
            if *x == v {
                return match to.unspan() {
                    Value::Null => Ok(()),
                    Value::Call(..) if split_call(to, data, vars).is_ok() => {
                        m.binds += 1;
                        table_mentions(to, v, data, vars, m)
                    }
                    // An alias's one bind is `Set(alias, Var(table))`.
                    Value::Var(_) if vars.name(v).starts_with("_vector_") => {
                        m.binds += 1;
                        Ok(())
                    }
                    _ => Err(format!("`{}` is rebound", vars.name(v))),
                };
            }
            if is_v(to) {
                if vars.name(*x).starts_with("_vector_") {
                    m.aliases.push(*x);
                    return Ok(());
                }
                return Err(format!(
                    "`{}` is copied into `{}`",
                    vars.name(v),
                    vars.name(*x)
                ));
            }
            table_mentions(to, v, data, vars, m)
        }
        Value::Call(d, args) if (*d as usize) < data.definitions.len() => {
            let name = data.def(*d).name();
            let length_read = (name == "OpLengthVector" && args.len() == 1)
                || one_op_wrapper(data, *d).is_some_and(|(op, operands)| {
                    data.def(op).name() == "OpLengthVector"
                        && matches!(operands.as_slice(), [WrapperOperand::Param(0)])
                });
            if length_read && args.first().is_some_and(is_v) {
                m.reads += 1;
                return Ok(());
            }
            if name == "OpGetText"
                && let Some(elem) = args.first()
                && let Some(inner) = call_named(elem, data, "OpGetVectorNullable")
                    .or_else(|| call_named(elem, data, "OpGetVector"))
                && inner.first().is_some_and(is_v)
            {
                m.reads += 1;
                // The index may hold anything but the table itself.
                for a in inner.iter().skip(1).chain(args.iter().skip(1)) {
                    table_mentions(a, v, data, vars, m)?;
                }
                return Ok(());
            }
            if args.iter().any(is_v) {
                return Err(format!("`{}` is handed to `{name}`", vars.name(v)));
            }
            for a in args {
                table_mentions(a, v, data, vars, m)?;
            }
            Ok(())
        }
        _ => {
            let mut verdict = Ok(());
            node.for_each_child(&mut |child| {
                if verdict.is_ok() {
                    verdict = table_mentions(child, v, data, vars, m);
                }
            });
            verdict
        }
    }
}

/// Does the block that binds `v` from its split hold every other mention of `v`, after
/// the bind?  The table is a Rust `let` where the bind stands, so this is its scope.
fn bind_block_holds_every_mention(body: &Value, v: u16) -> Result<(), String> {
    let is_bind = |op: &Value| matches!(op.unspan(), Value::Set(x, to) if *x == v && !matches!(to.unspan(), Value::Null));
    let count = |n: &Value| {
        let mut c = 0u32;
        n.any_node(&mut |x| {
            if matches!(x, Value::Var(u) if *u == v) {
                c += 1;
            }
            false
        });
        c
    };
    let total = count(body);
    let mut after: Option<u32> = None;
    body.any_node(&mut |n| {
        let ops = match n {
            Value::Block(bl) => &bl.operators,
            Value::Loop(lp) => &lp.operators,
            _ => return false,
        };
        if let Some(k) = ops.iter().position(&is_bind) {
            after = Some(ops[k + 1..].iter().map(count).sum());
            return true;
        }
        false
    });
    match after {
        Some(a) if a == total => Ok(()),
        Some(a) => Err(format!(
            "{} of {total} mentions lie outside the binding block",
            total - a
        )),
        None => Err("the bind is not a statement of a block".to_string()),
    }
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
#[expect(clippy::too_many_lines, reason = "inherited")]
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
#[expect(clippy::too_many_lines, reason = "inherited")]
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
        *by_name
            .entry(super::names::sanitize(vars.name(*v)))
            .or_default() += 1;
    }
    let mut out = LitHoist::default();
    for v in cands.into_keys() {
        if by_name[&super::names::sanitize(vars.name(v))] != 1 {
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

/// `@FR-R-RefillBuffer` — what [`refill_buffers`] admitted for one function: its return
/// buffer `var`, minted by `OpDatabaseRefill`, and the literal zeroes of that buffer's vector
/// fields (by node address) that empty the vector in place instead.
#[derive(Default)]
pub struct RefillBuffers {
    pub var: Option<u16>,
    pub field_zeros: HashSet<usize>,
    /// The buffer is a one-field wrapper of a vector whose elements own heap (a text field):
    /// every empty of it is the releasing clear, which on the store's root wrapper resets the
    /// store whole and keeps the vector's capacity (`@FR-H-ClearRelease`, § V-ag / V-ai) —
    /// never the bare length reset, which would strand the elements' texts.
    pub heap_elems: bool,
    /// `@FR-R-RefillText`'s collection clause — the heap-element buffer keeps its elements
    /// across calls and the build refills their texts ([`keep_elements`]).
    pub keep: Option<KeepElems>,
}

/// `@FR-R-RefillText`'s collection clause, admitted for a heap-element refill buffer: the
/// wrapper's type and its vector field's byte position, the element type with its width and
/// text slots, and the element locals an append mints into the buffer's vector — the only
/// targets of the build's text sets.
#[derive(Clone, Debug)]
pub struct KeepElems {
    pub wrapper_tp: u16,
    pub field: u32,
    pub elem_tp: u16,
    pub size: u32,
    pub texts: Vec<u32>,
    pub elems: HashSet<u16>,
    /// The type operands of the appends' `OpNewRecord` mints — what `(R-CompleteWrite)`'s
    /// `mint_tps` is keyed by.
    pub mint_tps: HashSet<u16>,
}

/// Can a refilled record of type `tp` be rewritten whole by a complete literal, leaving
/// nothing of its previous value?  Every field owns no heap, or is a vector whose elements
/// own none — the literal empties such a vector in place and fills it again.  A text, a
/// keyed collection, a nested record that owns heap or anything else answers no: its old
/// heap would outlive the write that replaced the handle.
/// Does the function's top level clear the buffer `b` (`OpClearVector(b)`) before the first
/// statement that mints it?
fn entry_cleared(body: &Value, b: u16, data: &Data) -> bool {
    let Value::Block(bl) = body.unspan() else {
        return false;
    };
    let is = |v: &Value, name: &str| {
        let mut hit = false;
        v.any_node(&mut |n| {
            if let Value::Call(d, args) = n
                && (*d as usize) < data.definitions.len()
                && data.def(*d).name() == name
                && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == b)
            {
                hit = true;
            }
            hit
        });
        hit
    };
    for stmt in &bl.operators {
        if is(stmt, "OpDatabase") {
            return false;
        }
        if is(stmt, "OpClearVector") {
            return true;
        }
    }
    false
}

fn refillable(stores: &Stores, tp: u16) -> bool {
    refillable_plain(stores, tp) || heap_element_wrapper(stores, tp)
}

/// A one-field wrapper of a `vector<E>` whose element `E` is a record of scalars and texts:
/// the shape a vector local or a return buffer mints.  Its releasing clear resets the whole
/// store — everything in it was claimed inside the vector (`@FR-H-RootExtent`) — so a refill
/// that empties it that way leaves nothing of the previous value behind.  A text needs no
/// pointer outside the store, so the content stays swappable (`content_swappable`).
fn heap_element_wrapper(stores: &Stores, tp: u16) -> bool {
    let Some(crate::database::Parts::Struct(fields)) =
        stores.types.get(tp as usize).map(|t| &t.parts)
    else {
        return false;
    };
    let [field] = &fields[..] else {
        return false;
    };
    let Some(crate::database::Parts::Vector(e)) =
        stores.types.get(field.content as usize).map(|c| &c.parts)
    else {
        return false;
    };
    if !stores.owns_heap(*e) {
        return false;
    }
    let Some(crate::database::Parts::Struct(efields)) =
        stores.types.get(*e as usize).map(|c| &c.parts)
    else {
        return false;
    };
    efields
        .iter()
        .all(|f| f.content == 5 || !stores.owns_heap(f.content))
}

/// The `OpDatabase(b, tp)` a statement mints `b` with: the statement itself, or the arm of
/// the return buffer's null-check prologue (`if <b present> {} else OpDatabase(b, tp)`).
fn mint_of(stmt: &Value, b: u16, data: &Data) -> Option<u16> {
    let call = |v: &Value| -> Option<u16> {
        let Value::Call(d, args) = v.unspan() else {
            return None;
        };
        if (*d as usize) >= data.definitions.len() || data.def(*d).name() != "OpDatabase" {
            return None;
        }
        match (
            args.first().map(Value::unspan),
            args.get(1).map(Value::unspan),
        ) {
            (Some(Value::Var(v)), Some(Value::Int(tp))) if *v == b => u16::try_from(*tp).ok(),
            _ => None,
        }
    };
    if let Some(tp) = call(stmt) {
        return Some(tp);
    }
    if let Value::If(_, t, f) = stmt.unspan() {
        return call(f).or_else(|| call(t));
    }
    None
}

/// `@FR-R-RefillBuffer` — the function's hidden return buffer, when EVERY mint of it heads a
/// literal group that writes every field ([`group_covers_type`]) of a [`refillable`] type.
/// Such a buffer may arrive holding the previous value of a rebound variable (the kept store
/// of `Stores::take_spare`) and still answer exactly the value a fresh mint would: the group
/// writes every scalar, and each vector field's zero becomes an in-place emptying that the
/// group's fill then refills.  A mint anywhere else — inside a call argument, a loop
/// expression, a second shape — is not one this walk can see the group of, so it declines
/// the whole function rather than refill a buffer some path leaves partly written.
pub fn refill_buffers(data: &Data, stores: &Stores, def_nr: u32) -> RefillBuffers {
    let mut out = RefillBuffers::default();
    let Some(b) = retbuf_var(data, def_nr) else {
        return out;
    };
    let body = data.def(def_nr).code();
    let mut mints = 0usize;
    body.any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpDatabase"
            && matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == b)
        {
            mints += 1;
        }
        false
    });
    if mints == 0 {
        return out;
    }
    let mut admitted = 0usize;
    let mut heap_tp: Option<u16> = None;
    let mut ok = true;
    let mut zeros: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Block(bl) = n {
            let ops = &bl.operators;
            for (i, stmt) in ops.iter().enumerate() {
                let Some(tp) = mint_of(stmt, b, data) else {
                    continue;
                };
                admitted += 1;
                heap_tp = Some(tp);
                if !refillable(stores, tp)
                    || !group_covers_type(ops, i + 1, b, tp, data, stores, &HashMap::new())
                {
                    ok = false;
                    continue;
                }
                let crate::database::Parts::Struct(fields) = &stores.types[tp as usize].parts
                else {
                    ok = false;
                    continue;
                };
                for g in ops.iter().skip(i + 1) {
                    let Value::Call(d, args) = g.unspan() else {
                        // A view of one of the buffer's fields (`buf = b.data`, the witness
                        // form of a local vector) writes nothing: the group goes on past it.
                        let view = matches!(g.unspan(), Value::Set(_, rhs)
                            if matches!(rhs.unspan(), Value::Call(gf, ga)
                                if (*gf as usize) < data.definitions.len()
                                    && data.def(*gf).name() == "OpGetField"
                                    && matches!(ga.first().map(Value::unspan), Some(Value::Var(w)) if *w == b)));
                        if matches!(g.unspan(), Value::Line(_)) || view {
                            continue;
                        }
                        break;
                    };
                    if (*d as usize) >= data.definitions.len()
                        || !data.def(*d).name().starts_with("OpSet")
                    {
                        break;
                    }
                    if data.def(*d).name() == "OpSetInt4"
                        && let [a0, off, zero] = &args[..]
                        && matches!(a0.unspan(), Value::Var(w) if *w == b)
                        && let Value::Int(off) = off.unspan()
                        && matches!(zero.unspan(), Value::Int(0))
                        && fields.iter().any(|f| {
                            i32::from(f.position) == *off
                                && matches!(
                                    stores.types.get(f.content as usize).map(|c| &c.parts),
                                    Some(crate::database::Parts::Vector(_))
                                )
                        })
                    {
                        zeros.insert(std::ptr::from_ref(g.unspan()) as usize);
                    }
                }
            }
        }
        false
    });
    if ok && admitted == mints {
        let heap_elems = heap_tp.is_some_and(|tp| heap_element_wrapper(stores, tp));
        // Elements that own heap are released ONCE, by the body's own entry clear of the
        // buffer — the releasing clear resets the store, and a second one in the group
        // re-claims and zero-fills the capacity for nothing.  So the group's empty is the
        // length reset, which is sound only when that entry clear runs before the first
        // mint; without it the buffer keeps its plain mint.
        if heap_elems && !entry_cleared(body, b, data) {
            return out;
        }
        out.var = Some(b);
        out.heap_elems = heap_elems;
        out.field_zeros = zeros;
        if heap_elems && let Some(tp) = heap_tp {
            out.keep = keep_elements(data, stores, def_nr, b, tp).ok();
        }
    }
    out
}

/// The variables a kept-element build names: the buffer, its field views and the elements
/// minted into it.
struct KeepVars {
    buf: u16,
    field: u32,
    views: HashSet<u16>,
    elems: HashSet<u16>,
}

impl KeepVars {
    /// The buffer's vector: a field view, or the field read in place.
    fn is_view(&self, v: &Value, data: &Data) -> bool {
        match v.unspan() {
            Value::Var(var) => self.views.contains(var),
            _ => matches!(call_named(v, data, "OpGetField"),
                Some([b, Value::Int(off), _]) if is_var(b, self.buf) && *off as u32 == self.field),
        }
    }

    fn is_elem(&self, v: &Value) -> bool {
        matches!(v.unspan(), Value::Var(var) if self.elems.contains(var))
    }

    /// Is every mention of the buffer, a view or an element under `node` one the kept
    /// elements survive: the entry clear and the mint, the field's zero, a self-replace, a
    /// release, a return; on the vector a reservation, a length, an element read, an append's
    /// mint and finish; on an element its own sets and reads.
    fn admits(&self, node: &Value, data: &Data) -> bool {
        let rest = |args: &[Value], from: usize| args[from..].iter().all(|a| self.admits(a, data));
        match node.unspan() {
            Value::Var(var) => {
                *var != self.buf && !self.views.contains(var) && !self.elems.contains(var)
            }
            Value::Set(var, rhs) if *var == self.buf => matches!(rhs.unspan(), Value::Null),
            Value::Set(var, rhs) if self.views.contains(var) => self.is_view(rhs, data),
            Value::Set(var, rhs) if self.elems.contains(var) => match rhs.unspan() {
                Value::Null => true,
                _ => matches!(call_named(rhs, data, "OpNewRecord"),
                    Some(args) if self.is_view(&args[0], data) && rest(args, 1)),
            },
            Value::Return(r) if is_var(r, self.buf) || self.is_view(r, data) => true,
            Value::Call(op, args)
                if (*op as usize) < data.definitions.len() && !args.is_empty() =>
            {
                let name = data.def(*op).name();
                let on_buf = is_var(&args[0], self.buf);
                match name {
                    "OpRefAlias" | "OpClearVector" | "OpDatabase" if on_buf => rest(args, 1),
                    "OpSetInt4" => {
                        (on_buf
                            && matches!(&args[1..], [Value::Int(off), Value::Int(0)] if *off as u32 == self.field))
                            || (!on_buf && args.iter().all(|a| self.admits(a, data)))
                    }
                    "OpReplaceVector"
                        if on_buf && args.len() > 1 && self.is_view(&args[1], data) =>
                    {
                        rest(args, 2)
                    }
                    "OpFreeRef" | "OpFreeRefIfDistinct" => {
                        args.iter().all(|a| matches!(a.unspan(), Value::Var(_)))
                    }
                    "OpPreAllocVector"
                    | "OpLengthVector"
                    | "t_6vector_len"
                    | "OpGetVector"
                    | "OpGetVectorNullable"
                        if self.is_view(&args[0], data) =>
                    {
                        rest(args, 1)
                    }
                    "OpFinishRecord"
                        if self.is_view(&args[0], data)
                            && args.len() > 1
                            && self.is_elem(&args[1]) =>
                    {
                        rest(args, 2)
                    }
                    "OpSetText" if self.is_elem(&args[0]) => {
                        matches!(args.get(1).map(Value::unspan), Some(Value::Int(_)))
                            && rest(args, 2)
                    }
                    n if (n.starts_with("OpSet") || n.starts_with("OpGet"))
                        && self.is_elem(&args[0]) =>
                    {
                        rest(args, 1)
                    }
                    _ => args.iter().all(|a| self.admits(a, data)),
                }
            }
            _ => {
                let mut ok = true;
                node.for_each_child(&mut |c| {
                    if ok && !self.admits(c, data) {
                        ok = false;
                    }
                });
                ok
            }
        }
    }
}

/// `@FR-R-RefillText`'s collection clause — may the heap-element buffer `buf` (a one-field
/// `wrapper_tp` around `vector<E>`) keep its elements across calls, the build refilling each
/// element's texts in its slot?  The element is a record of scalars and texts; its build
/// appends through `OpNewRecord` minted elements only; the buffer is cleared once, at entry
/// (a second clear would reset the store under the kept slots); and every other mention is
/// one [`KeepVars::admits`] names.  Answers the facts the emitter needs, or why it declines.
///
/// # Errors
/// The condition the build fails, in the words `LOFT_TRACE_REFILL_TEXT` prints.
pub fn keep_elements(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    buf: u16,
    wrapper_tp: u16,
) -> Result<KeepElems, &'static str> {
    let Some(crate::database::Parts::Struct(fields)) =
        stores.types.get(wrapper_tp as usize).map(|t| &t.parts)
    else {
        return Err("the buffer is not a wrapper record");
    };
    let [field] = &fields[..] else {
        return Err("the buffer is not a one-field wrapper");
    };
    let Some(crate::database::Parts::Vector(elem_tp)) =
        stores.types.get(field.content as usize).map(|c| &c.parts)
    else {
        return Err("the wrapper's field is not a vector");
    };
    let Some(texts) = stores.text_slots(*elem_tp) else {
        return Err("an element owns heap other than text");
    };
    let body = data.def(def_nr).code();
    let mut k = KeepVars {
        buf,
        field: u32::from(field.position),
        views: HashSet::new(),
        elems: HashSet::new(),
    };
    let mut clears = 0usize;
    body.any_node(&mut |n| {
        match n {
            Value::Set(var, rhs) if k.is_view(rhs, data) => {
                k.views.insert(*var);
            }
            Value::Call(..) if matches!(call_named(n, data, "OpClearVector"), Some([b]) if is_var(b, buf)) => {
                clears += 1;
            }
            _ => {}
        }
        false
    });
    let mut mint_tps = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(var, rhs) = n
            && let Some(args) = call_named(rhs, data, "OpNewRecord")
            && k.is_view(&args[0], data)
        {
            k.elems.insert(*var);
            if let Some(Value::Int(tp)) = args.get(1).map(Value::unspan) {
                mint_tps.insert(*tp as u16);
            }
        }
        false
    });
    if clears != 1 {
        return Err("the buffer is cleared again after its entry");
    }
    if k.elems.is_empty() {
        return Err("no element is appended");
    }
    if !k.admits(body, data) {
        return Err("the vector or an element is used another way");
    }
    Ok(KeepElems {
        wrapper_tp,
        field: k.field,
        elem_tp: *elem_tp,
        size: u32::from(stores.size(*elem_tp)),
        texts,
        elems: k.elems,
        mint_tps,
    })
}

/// `@FR-R-RefillText` — the pooled call sites of one function whose release is not emitted
/// and whose call goes to the callee's refill twin (`__rt`): each pool statement
/// (`if OpRefIsNull(b) { OpDatabase(b, tp) } else OpClear(b, tp)`) and each call's argument
/// slice, by address, with the buffer variable and its type.
#[derive(Default, Debug)]
pub struct RefillTextSites {
    pub pools: HashMap<usize, (u16, u16)>,
    pub calls: HashMap<usize, (u16, u16)>,
    /// The calls admitted under the VECTOR clause, by argument-slice address: the plain
    /// callee empties each vector field in place, so the call takes no twin and no release.
    pub vectors: HashSet<usize>,
}

/// Which clause of `@FR-R-RefillText` a pooled site is admitted under.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RefillClause {
    /// The callee's refill twin writes each text over its slot's block.
    Text,
    /// The plain callee empties each vector field in place (`(R-RefillBuffer)`).
    Vectors,
}

/// `LOFT_NO_REFILL_VECTORS=1` — the vector clause off: a pooled buffer whose type holds
/// vectors of plain elements is released before every call again.  The first bisect step
/// for a wrong or stale vector field of a result built in a reused call buffer.
fn refill_vectors_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_REFILL_VECTORS").is_ok_and(|v| v != "0"))
}

/// `@FR-R-RefillText`'s vector clause for a callee: `(R-RefillBuffer)` refills its return
/// buffer — every mint heads a literal writing every field, each vector field's zero an
/// in-place emptying — its vector elements own no heap, and the result type's heap is
/// those vectors and nothing else ([`Stores::vector_slots`]).  Answers the result type.
///
/// # Errors
/// The condition the callee fails, in the words `LOFT_TRACE_REFILL_TEXT` prints.
pub fn refill_vector_callee(
    data: &Data,
    stores: &Stores,
    callee: u32,
) -> Result<u16, &'static str> {
    if !refill_vectors_enabled() {
        return Err("the vector clause is switched off");
    }
    let Some(buf) = retbuf_var(data, callee) else {
        return Err("no return buffer");
    };
    let refill = refill_buffers(data, stores, callee);
    if refill.var != Some(buf) {
        return Err("the callee does not refill its buffer");
    }
    if refill.heap_elems {
        return Err("the callee's elements own heap");
    }
    let Some(tp) = mint_type_of(data, callee, buf) else {
        return Err("no mint of the buffer");
    };
    match stores.vector_slots(tp) {
        Some(found) if !found.is_empty() => Ok(tp),
        Some(_) => Err("the result holds no vector"),
        None => Err("the result owns heap other than vectors of plain elements"),
    }
}

/// The one statement of a mint branch: a bare call, or a block or insert holding only it.
fn only_statement(branch: &Value) -> Option<&Value> {
    let ops = match branch.unspan() {
        Value::Block(bl) => &bl.operators[..],
        Value::Insert(ops) => &ops[..],
        other => return Some(other),
    };
    match ops {
        [stmt] => Some(stmt),
        _ => None,
    }
}

/// The pool statement of a call site: `if OpRefIsNull(b) { OpDatabase(b, tp) } else
/// OpClear(b, tp)`, answered as `(b, tp)`.
fn pool_of(stmt: &Value, data: &Data) -> Option<(u16, u16)> {
    let Value::If(cond, mint_branch, release) = stmt.unspan() else {
        return None;
    };
    let buf = match call_named(cond, data, "OpRefIsNull")? {
        [arg] => match arg.unspan() {
            Value::Var(var) => *var,
            _ => return None,
        },
        _ => return None,
    };
    let mint = only_statement(mint_branch)?;
    let tp = match call_named(mint, data, "OpDatabase")? {
        [arg, Value::Int(tp)] if is_var(arg, buf) => u16::try_from(*tp).ok()?,
        _ => return None,
    };
    match call_named(release, data, "OpClear")? {
        // `@FR-R-Reuse`'s reset clause sets `POOL_RESET` on the release's type.
        [arg, Value::Int(clear_tp)]
            if is_var(arg, buf)
                && i32::from(tp) == *clear_tp & i32::from(crate::keys::POOL_TP_MASK) =>
        {
            Some((buf, tp))
        }
        _ => None,
    }
}

/// The byte offset of a record path rooted at `b` — `b` itself, or `OpGetField` steps into
/// its inline sub-records.
pub(crate) fn path_offset(v: &Value, b: u16, data: &Data) -> Option<u32> {
    if is_var(v, b) {
        return Some(0);
    }
    match call_named(v, data, "OpGetField")? {
        [p, Value::Int(off), _] => Some(path_offset(p, b, data)? + u32::try_from(*off).ok()?),
        _ => None,
    }
}

/// The literal a mint at `ops[at]` heads, read for `@FR-R-RefillText` (b): its contiguous
/// `OpSet*`s into `buf` (or its inline sub-records), then the frees of its own work texts,
/// then the exit — `return buf`, or `buf` as the block's value.  Answers how many mentions
/// of `buf` the literal accounts for and the byte offsets of the texts it writes, or why it
/// is not such a literal.
fn literal_text_writes(
    ops: &[Value],
    at: usize,
    buf: u16,
    data: &Data,
) -> Result<(u32, Vec<u32>), &'static str> {
    let mut mentions = var_mentions(&ops[at], buf);
    let mut written: Vec<u32> = Vec::new();
    let mut next = at + 1;
    while let Some(stmt) = ops.get(next) {
        if matches!(stmt.unspan(), Value::Line(_)) {
            next += 1;
            continue;
        }
        let Value::Call(setter, args) = stmt.unspan() else {
            break;
        };
        if (*setter as usize) >= data.definitions.len()
            || !data.def(*setter).name().starts_with("OpSet")
        {
            break;
        }
        let Some(base) = args.first().and_then(|t| path_offset(t, buf, data)) else {
            break;
        };
        if args.iter().skip(1).any(|a| var_mentions(a, buf) > 0) {
            return Err("a literal value reads the buffer");
        }
        if data.def(*setter).name() == "OpSetText" {
            let Some(Value::Int(off)) = args.get(1).map(Value::unspan) else {
                return Err("a text set at a computed offset");
            };
            written.push(base + u32::try_from(*off).unwrap_or(u32::MAX));
        }
        mentions += 1;
        next += 1;
    }
    for stmt in &ops[next..] {
        match stmt.unspan() {
            Value::Line(_) => {}
            Value::Return(r) if is_var(r, buf) => return Ok((mentions + 1, written)),
            Value::Var(var) if *var == buf => return Ok((mentions + 1, written)),
            _ if call_named(stmt, data, "OpFreeText").is_some() && var_mentions(stmt, buf) == 0 => {
            }
            _ => break,
        }
    }
    Err("a literal that is not the exit")
}

/// Does this exit answer the buffer: `buf`, or a block whose value is `buf` (the literal's
/// own block, `return { …; buf }`)?
fn answers_buffer(v: &Value, buf: u16) -> bool {
    match v.unspan() {
        Value::Var(var) => *var == buf,
        Value::Block(bl) => bl.operators.last().is_some_and(|l| answers_buffer(l, buf)),
        _ => false,
    }
}

/// `@FR-R-RefillText` (a) and (b) for a callee — its result type's heap is text only, and
/// every exit is a record literal into its own buffer writing every text slot exactly once,
/// with nothing else in the body naming the buffer.  Answers the result type.
///
/// # Errors
/// The condition the callee fails, in the words `LOFT_TRACE_REFILL_TEXT` prints.
pub fn refill_text_callee(data: &Data, stores: &Stores, callee: u32) -> Result<u16, &'static str> {
    let Some(buf) = retbuf_var(data, callee) else {
        return Err("no return buffer");
    };
    let body = data.def(callee).code();
    let mut result_tp: Option<u16> = None;
    let mut admitted = 0u32;
    let mut failed: Option<&'static str> = None;
    let mut slots: Vec<u32> = Vec::new();
    body.any_node(&mut |n| {
        let Value::Block(bl) = n else {
            return false;
        };
        for at in 0..bl.operators.len() {
            let Some(tp) = mint_of(&bl.operators[at], buf, data) else {
                continue;
            };
            if result_tp.is_some_and(|known| known != tp) {
                failed = Some("two result types");
                return true;
            }
            if result_tp.is_none() {
                match stores.text_slots(tp) {
                    Some(found) if !found.is_empty() => slots = found,
                    Some(_) => failed = Some("the result holds no text"),
                    None => failed = Some("the result owns heap other than text"),
                }
                if failed.is_some() {
                    return true;
                }
                slots.sort_unstable();
                result_tp = Some(tp);
            }
            match literal_text_writes(&bl.operators, at, buf, data) {
                Ok((mentions, mut written)) => {
                    written.sort_unstable();
                    if written != slots {
                        failed = Some("a literal does not write every text once");
                        return true;
                    }
                    admitted += mentions;
                }
                Err(why) => {
                    failed = Some(why);
                    return true;
                }
            }
        }
        false
    });
    if let Some(why) = failed {
        return Err(why);
    }
    let Some(tp) = result_tp else {
        return Err("no literal into the buffer");
    };
    if var_mentions(body, buf) != admitted {
        return Err("the buffer is named outside its literals");
    }
    let other_exit =
        body.any_node(&mut |n| matches!(n, Value::Return(r) if !answers_buffer(r, buf)));
    if other_exit {
        return Err("an exit that is not the literal");
    }
    Ok(tp)
}

/// Is every mention of the pooled result `res` in `node` a READ — the record a reading
/// `OpGet*` (through `OpGetField` steps) reads, the source of a copy that keeps it, or a
/// release that leaves the pool's buffer alone?  A write, a move, a pass to a call, a rebind
/// or a bare use declines.  Its binding (`Set(res, …)`) is counted by the caller, once.
fn result_only_read(node: &Value, res: u16, buf: u16, data: &Data) -> bool {
    match node.unspan() {
        Value::Var(var) => *var != res,
        Value::Set(_, val) => result_only_read(val, res, buf, data),
        Value::Call(op, args) if (*op as usize) < data.definitions.len() => {
            let name = data.def(*op).name();
            let reads_first = (name.starts_with("OpGet") && name != "OpGetField")
                || (name == "OpCopyRecord"
                    && matches!(args.get(2).map(Value::unspan), Some(Value::Int(flags))
                        if (*flags as u16) & crate::keys::COPY_FREE_SOURCE == 0));
            if name == "OpFreeRefIfDistinct"
                && matches!(&args[..], [first, second] if is_var(first, res) && is_var(second, buf))
            {
                return true;
            }
            args.iter().enumerate().all(|(pos, arg)| {
                (pos == 0 && reads_first && path_offset(arg, res, data).is_some())
                    || result_only_read(arg, res, buf, data)
            })
        }
        _ => {
            let mut ok = true;
            node.for_each_child(&mut |child| {
                if ok && !result_only_read(child, res, buf, data) {
                    ok = false;
                }
            });
            ok
        }
    }
}

/// One pooled call site as `refill_text_sites` finds it: the pool statement, the call's
/// arguments, the buffer and its type, the local bound from the call, and the callee.
struct PoolSite<'a> {
    pool: &'a Value,
    args: &'a [Value],
    buf: u16,
    tp: u16,
    res: u16,
    callee: u32,
}

/// The clause `site` is admitted under, or why it is not: the callee's conditions
/// ([`refill_text_callee`], [`refill_vector_callee`]) and (c) — the buffer named by nothing
/// but its pool statement, this call's buffer argument and its releases, the result bound
/// once.  The text clause also asks that the result be only READ; the vector clause does
/// not, because every write the language makes to a vector field leaves it owning its vector
/// (or empty), which is all the in-place emptying reads.
fn refill_text_site_declines(
    data: &Data,
    stores: &Stores,
    body: &Value,
    site: &PoolSite,
    callee_declines: &dyn Fn(u32) -> Option<&'static str>,
) -> Result<RefillClause, &'static str> {
    let PoolSite {
        pool,
        args,
        buf,
        tp,
        res,
        callee,
    } = *site;
    if (callee as usize) >= data.definitions.len()
        || !matches!(data.def(callee).def_type(), DefType::Function)
    {
        return Err("not a loft function");
    }
    // The buffer ARGUMENT's position is its attribute's, not its variable number: a renamed
    // buffer (`(R-Rebind)`'s `__ref_1`) sits behind locals of its own.
    let Some(buf_arg) = data.def(callee).hidden_return_buffer_attr() else {
        return Err("no return buffer");
    };
    if let Some(why) = callee_declines(callee) {
        return Err(why);
    }
    if args.iter().enumerate().any(|(pos, arg)| {
        if pos == buf_arg {
            !is_var(arg, buf)
        } else {
            var_mentions(arg, buf) > 0 || var_mentions(arg, res) > 0
        }
    }) {
        return Err("the call's arguments name the buffer or the result");
    }
    let clause = match refill_text_callee(data, stores, callee) {
        Ok(callee_tp) if callee_tp != tp => return Err("the pool's type is not the callee's"),
        Ok(_) => RefillClause::Text,
        Err(text_why) => match refill_vector_callee(data, stores, callee) {
            Ok(callee_tp) if callee_tp == tp => RefillClause::Vectors,
            Ok(_) => return Err("the pool's type is not the callee's"),
            // The reason of the clause the pool's type belongs to.
            Err(vector_why) if stores.vector_slots(tp).is_some_and(|v| !v.is_empty()) => {
                return Err(vector_why);
            }
            Err(_) => return Err(text_why),
        },
    };
    let mut allowed = var_mentions(pool, buf) + 1;
    let mut binds = 0usize;
    body.any_node(&mut |n| {
        match n {
            Value::Call(op, op_args)
                if (*op as usize) < data.definitions.len()
                    && matches!(data.def(*op).name(), "OpFreeRef" | "OpFreeRefIfDistinct") =>
            {
                allowed += op_args.iter().filter(|a| is_var(a, buf)).count() as u32;
            }
            Value::Set(var, val) if *var == buf && matches!(val.unspan(), Value::Null) => {
                allowed += 1;
            }
            Value::Set(var, val) if *var == res && !matches!(val.unspan(), Value::Null) => {
                binds += 1;
            }
            _ => {}
        }
        false
    });
    if var_mentions(body, buf) != allowed {
        return Err("the buffer is named outside its pool");
    }
    if binds != 1 {
        return Err("the result is bound more than once");
    }
    if clause == RefillClause::Text && !result_only_read(body, res, buf, data) {
        return Err("the result is not only read");
    }
    Ok(clause)
}

/// `LOFT_TRACE_REFILL_TEXT`'s line for one site, printed once: a function is emitted more
/// than once (its twins, a second pass) and its sites are named the first time.
/// Print a `LOFT_TRACE_REFILL_TEXT` line once: a function is emitted more than once (its
/// twins, a second pass) and each verdict is named the first time.
pub fn trace_refill_text_once(line: &str) {
    thread_local! {
        static SEEN: std::cell::RefCell<HashSet<String>> =
            std::cell::RefCell::new(HashSet::new());
    }
    if SEEN.with(|seen| seen.borrow_mut().insert(line.to_string())) {
        eprintln!("{line}");
    }
}

fn trace_refill_text_site(
    data: &Data,
    site_fn: u32,
    target: u32,
    verdict: Result<RefillClause, &str>,
) {
    let to = if (target as usize) < data.definitions.len() {
        data.def(target).name()
    } else {
        "?"
    };
    let from = data.def(site_fn).name();
    let line = match verdict {
        Ok(RefillClause::Text) => format!("refill-text: {from} → {to} admitted"),
        Ok(RefillClause::Vectors) => format!("refill-text: {from} → {to} admitted (vectors)"),
        Err(why) => format!("refill-text: {from} → {to} declined — {why}"),
    };
    trace_refill_text_once(&line);
}

/// `@FR-R-RefillText` — the pooled call sites of `def_nr` whose callee refills its texts in
/// place and whose buffer keeps the slot invariant ([`refill_text_site_declines`]).  `trace`
/// prints each site's verdict (`LOFT_TRACE_REFILL_TEXT`); `callee_declines` is the emitter's
/// own reason a callee cannot take a twin (a value-record callee has no buffer).
pub fn refill_text_sites(
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    callee_declines: &dyn Fn(u32) -> Option<&'static str>,
    trace: bool,
) -> RefillTextSites {
    let mut out = RefillTextSites::default();
    let body = data.def(def_nr).code();
    let mut sites: Vec<PoolSite> = Vec::new();
    body.any_node(&mut |n| {
        let Value::Block(bl) = n else {
            return false;
        };
        let ops = &bl.operators;
        for (at, pool) in ops.iter().enumerate() {
            let Some((buf, tp)) = pool_of(pool, data) else {
                continue;
            };
            let Some(next) = ops[at + 1..]
                .iter()
                .find(|o| !matches!(o.unspan(), Value::Line(_)))
            else {
                continue;
            };
            let Value::Set(res, call) = next.unspan() else {
                continue;
            };
            let Value::Call(callee, args) = call.unspan() else {
                continue;
            };
            sites.push(PoolSite {
                pool,
                args: args.as_slice(),
                buf,
                tp,
                res: *res,
                callee: *callee,
            });
        }
        false
    });
    for site in &sites {
        let verdict = refill_text_site_declines(data, stores, body, site, callee_declines);
        if trace {
            trace_refill_text_site(data, def_nr, site.callee, verdict);
        }
        if let Ok(clause) = verdict {
            out.pools.insert(
                std::ptr::from_ref(site.pool.unspan()) as usize,
                (site.buf, site.tp),
            );
            out.calls
                .insert(site.args.as_ptr() as usize, (site.buf, site.tp));
            if clause == RefillClause::Vectors {
                out.vectors.insert(site.args.as_ptr() as usize);
            }
        }
    }
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
    /// The comprehension clause — `(block value, element)` pairs whose `OpCopyRecord(c, elm, tp)`
    /// is emitted as nothing: the block's vector was built in the element ([`comprehension_rows`]).
    pub whole_copies: HashSet<(u16, u16)>,
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
/// answered `0` for `100` on the eleventh element.  Which variables can hold such a view is
/// [`crate::variables::Function::store_viewers`]'s answer.
///
/// An upper bound on purpose, because a miss reads freed bytes.  One refinement, for a
/// destination FIELD at byte `dest`: a local that depends on `out` alone and whose every
/// binding views another field of it (`sc.brushes[i]?` beside `sc.ops`, `parse_lock`'s shape)
/// names a collection the growth does not move, and is spared — read with the scope pass's own
/// place model ([`crate::scopes::value_view_places_depth`]), which follows a `?` discharge's block
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
                        let places = crate::scopes::value_view_places_depth(rhs, data, vars);
                        other &= !places.is_empty()
                            && places.iter().all(|&((r, off), _)| {
                                r == out && off != crate::use_analysis::ANY_FIELD && off != dest
                            });
                    }
                }
            }
            false
        });
        bound && other
    };
    vars.store_viewers(out)
        .into_iter()
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
#[expect(clippy::too_many_lines, reason = "inherited")]
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
    comprehension_rows(data, stores, vars, body, &mut out_map, trace);
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

/// `@FR-R-ElemFirst`'s comprehension clause — a vector built by a block (a comprehension,
/// `[for x in … { … }]`) and appended as a WHOLE element of a local vector of vectors is built
/// in that element:
///
/// ```text
/// c = { …  OpDatabase[NP](vdb, tp) · vec = OpGetField(vdb, 0, tp) · … pushes into vec … · vec }
/// [OpPreAllocVector(out, 1, size)] · elm = OpNewRecord(out, etp, 65535) · [OpSetInt4(elm, 0, 0)]
/// OpCopyRecord(c, elm, vtp) · OpFinishRecord(out, elm, etp, 65535)
/// ```
///
/// becomes the element minted where `vdb` was, `vec` bound to the element's slot, and the copy
/// emitted as nothing: the row is written once, in the store it lives in, instead of built in a
/// loop-buffer store and copied across.  Admitted where `c` is named by its bind and the copy
/// alone, `vec` only inside its block, `vdb` only there and by frees; the block names neither
/// `out` nor a view of its elements and jumps nowhere outside itself (an unfinished element
/// would be stranded); and the block LOOPS — a body that loops again holds no push window on
/// `out` (`@FR-R-PushFill`), the one raw address into `out`'s store the growth of the row's
/// vector could leave stale.  Everything else held on that store is already refreshed or
/// declined per pass, because the element's own mint grows it.
fn comprehension_rows(
    data: &Data,
    stores: &Stores,
    vars: &crate::variables::Function,
    body: &Value,
    out_map: &mut ElemFirstMap,
    trace: bool,
) {
    let mut found: Vec<(ElemFirst, u16)> = Vec::new();
    body.any_node(&mut |n| {
        // A comprehension's append group stands in the body of the LOOP that builds the outer
        // vector, so a loop body is searched as a block is.
        let (Value::Block(bl) | Value::Loop(bl)) = n else {
            return false;
        };
        let ops = &bl.operators;
        let idx: Vec<usize> = (0..ops.len())
            .filter(|p| !matches!(ops[*p].unspan(), Value::Line(_)))
            .collect();
        for ki in 0..idx.len() {
            let Value::Set(comp, cv) = ops[idx[ki]].unspan() else {
                continue;
            };
            let Some(row) = comprehension_block(data, vars, cv) else {
                continue;
            };
            let Some(group) = row_append_group(data, ops, &idx, ki, *comp, row.vtp) else {
                continue;
            };
            if let Err(why) =
                admit_row(data, vars, body, ops[idx[ki]].unspan(), &row, &group, *comp)
            {
                if trace {
                    eprintln!(
                        "elemfirst: comprehension {} -> {}: DECLINED — {why}",
                        vars.name(row.vec),
                        vars.name(group.out)
                    );
                }
                continue;
            }
            if trace {
                eprintln!(
                    "elemfirst: comprehension {} built in its element of {}",
                    vars.name(row.vec),
                    vars.name(group.out)
                );
            }
            let size = group.size.unwrap_or_else(|| {
                u16::try_from(group.etp).map_or(0, |t| i32::from(stores.size(t)))
            });
            found.push((
                ElemFirst {
                    out: group.out,
                    out_tp: group.etp,
                    out_fld: 65535,
                    elm: group.elm,
                    aliases: Vec::new(),
                    prealloc_size: size,
                    binds: vec![ElemBind {
                        tmp: row.vec,
                        vdb: row.vdb,
                        field_off: 0,
                        first: true,
                        from_call: false,
                    }],
                },
                *comp,
            ));
        }
        false
    });
    for (pair, comp) in found {
        if out_map.by_vdb.contains_key(&pair.binds[0].vdb) || out_map.by_elm.contains_key(&pair.elm)
        {
            continue;
        }
        let at = out_map.pairs.len();
        out_map.by_vdb.insert(pair.binds[0].vdb, at);
        out_map.by_elm.insert(pair.elm, at);
        out_map.elms.insert(pair.elm);
        out_map.whole_copies.insert((comp, pair.elm));
        out_map.pairs.push(pair);
    }
}

/// The comprehension clause's ROW: the block that builds it, the vector local it builds, the
/// buffer that vector is declared through, and the vector's type.
struct RowBlock<'a> {
    inner: &'a Value,
    vec: u16,
    vdb: u16,
    vtp: i32,
}

/// The block a bind's value descends to through value blocks, when its value is a vector
/// declared through a `__vdb` buffer: `OpDatabase[NP](vdb, …) · vec = OpGetField(vdb, 0, vtp)`.
fn comprehension_block<'a>(
    data: &Data,
    vars: &crate::variables::Function,
    value: &'a Value,
) -> Option<RowBlock<'a>> {
    let mut inner = value.unspan();
    while let Value::Block(outer) = inner
        && let Some(Value::Block(_)) = outer.operators.last().map(Value::unspan)
    {
        inner = outer.operators.last().map(Value::unspan)?;
    }
    let Value::Block(vblock) = inner else {
        return None;
    };
    let vec = as_var(vblock.operators.last())?;
    let code: Vec<&Value> = vblock
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let di = code.iter().position(|o| {
        call_named(o, data, "OpDatabase").is_some() || call_named(o, data, "OpDatabaseNP").is_some()
    })?;
    let dargs = call_named(code[di], data, "OpDatabase")
        .or_else(|| call_named(code[di], data, "OpDatabaseNP"))?;
    let vdb = as_var(dargs.first())?;
    if !vars.name(vdb).starts_with("__vdb") {
        return None;
    }
    let Value::Set(t, x) = code.get(di + 1)?.unspan() else {
        return None;
    };
    let ga = call_named(x, data, "OpGetField")?;
    if *t != vec
        || as_var(ga.first()) != Some(vdb)
        || !matches!(ga.get(1).map(Value::unspan), Some(Value::Int(0)))
    {
        return None;
    }
    let Some(Value::Int(vtp)) = ga.get(2).map(Value::unspan) else {
        return None;
    };
    Some(RowBlock {
        inner,
        vec,
        vdb,
        vtp: *vtp,
    })
}

/// The append group that consumes the row bound to `comp` at `idx[ki]`, right after it:
/// `[OpPreAllocVector(out, 1, size)] · elm = OpNewRecord(out, etp, 65535) · [OpSetInt4(elm,
/// 0, 0)] · OpCopyRecord(comp, elm, vtp) · OpFinishRecord(out, elm, …)`.  The copy writes the
/// element itself — the element of a vector of vectors IS the inner vector's handle, at offset
/// 0 — and copies the row's own vector type.
struct RowGroup {
    out: u16,
    elm: u16,
    etp: i32,
    size: Option<i32>,
}

fn row_append_group(
    data: &Data,
    ops: &[Value],
    idx: &[usize],
    ki: usize,
    comp: u16,
    vtp: i32,
) -> Option<RowGroup> {
    let at = |g: usize| idx.get(g).map(|&p| &ops[p]);
    let mut gi = ki + 1;
    let mut size = None;
    if let Some(pa) = at(gi).and_then(|o| call_named(o, data, "OpPreAllocVector")) {
        if let Some(Value::Int(sz)) = pa.get(2).map(Value::unspan) {
            size = Some(*sz);
        }
        gi += 1;
    }
    let Value::Set(elm, mv) = at(gi)?.unspan() else {
        return None;
    };
    let margs = call_named(mv, data, "OpNewRecord")?;
    let (Some(out), Some(Value::Int(etp)), Some(Value::Int(65535))) = (
        as_var(margs.first()),
        margs.get(1).map(Value::unspan),
        margs.get(2).map(Value::unspan),
    ) else {
        return None;
    };
    gi += 1;
    if at(gi)
        .and_then(|o| call_named(o, data, "OpSetInt4"))
        .is_some_and(|a| {
            as_var(a.first()) == Some(*elm)
                && matches!(a.get(1).map(Value::unspan), Some(Value::Int(0)))
        })
    {
        gi += 1;
    }
    let copies = at(gi)
        .and_then(|o| call_named(o, data, "OpCopyRecord"))
        .is_some_and(|a| {
            as_var(a.first()) == Some(comp)
                && as_var(a.get(1)) == Some(*elm)
                && matches!(a.get(2).map(Value::unspan), Some(Value::Int(t)) if *t == vtp)
        });
    let finishes = at(gi + 1)
        .and_then(|o| call_named(o, data, "OpFinishRecord"))
        .is_some_and(|a| as_var(a.first()) == Some(out) && as_var(a.get(1)) == Some(*elm));
    (copies && finishes).then_some(RowGroup {
        out,
        elm: *elm,
        etp: *etp,
        size,
    })
}

/// Every mention of `w` in `within`: a read, or a bind of it.
fn var_mentions(within: &Value, w: u16) -> u32 {
    let mut n = 0;
    within.any_node(&mut |x| {
        match x {
            Value::Var(v) | Value::Set(v, _) if *v == w => n += 1,
            _ => {}
        }
        false
    });
    n
}

/// `w`'s null declarations and frees in `body` — the only mentions a never-minted buffer
/// answers as nothing.
fn decl_and_free_mentions(data: &Data, body: &Value, w: u16) -> u32 {
    let mut n = 0;
    body.any_node(&mut |x| {
        match x {
            Value::Set(v, init) if *v == w && matches!(init.unspan(), Value::Null) => n += 1,
            Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && matches!(
                        data.def(*d).name(),
                        "OpFreeRef" | "OpFreeRefIfDistinct" | "OpClear"
                    )
                    && as_var(args.first()) == Some(w) =>
            {
                n += 1;
            }
            _ => {}
        }
        false
    });
    n
}

/// The comprehension clause's admission, per `comprehension_rows`' doc: `Err` names the
/// condition that declines.
fn admit_row(
    data: &Data,
    vars: &crate::variables::Function,
    body: &Value,
    bind: &Value,
    row: &RowBlock,
    group: &RowGroup,
    comp: u16,
) -> Result<(), &'static str> {
    if !matches!(vars.tp(group.out).base(), Type::Vector(_, _)) || vars.is_argument(group.out) {
        return Err("the destination is not a local vector");
    }
    if u16::try_from(group.etp).is_err() {
        return Err("the element type is out of range");
    }
    if var_mentions(body, comp) != 2 {
        return Err("the block's value is read elsewhere");
    }
    if var_mentions(body, row.vec) != var_mentions(row.inner, row.vec) {
        return Err("the vector is named outside its block");
    }
    if var_mentions(body, row.vdb)
        != var_mentions(row.inner, row.vdb) + decl_and_free_mentions(data, body, row.vdb)
    {
        return Err("the buffer is named outside its block");
    }
    if !row.inner.any_node(&mut |x| matches!(x, Value::Loop(_))) {
        return Err("the block does not loop");
    }
    if row.inner.reads_var(group.out) || jumps_out(bind) {
        return Err("the block names the destination or jumps out");
    }
    let moved = destination_views(data, body, vars, group.out, None);
    if moved
        .iter()
        .any(|w| *w != group.elm && row.inner.reads_var(*w))
    {
        return Err("the block reads a view of the destination's elements");
    }
    Ok(())
}

pub(crate) use crate::opt::value_records::layout::tuple_param_vars;
pub use crate::opt::value_records::layout::{
    FnRefSites, LeafSource, NULLABLE_LAYOUT, TupleParams, VALUE_RECORD_MAX_FIELDS, VIEW_LEAF_PART,
    ValueRecords, ValueTuple, ViewOffsets, ViewPlan, fnref_value, is_nullable_layout, is_view_part,
    layout_record, nullable_layout,
};

/// The tuple of `fields` read out of the record `db` names (a `DbRef` expression) through
/// `stores` (a `&Stores` expression), each element guarded exactly as the field's getter
/// template guards it — a null record answers the field's null.  ONE spelling for the
/// live-reload arm, the cdylib bridge and any other site that must build a tuple out of a
/// record it holds as a `DbRef`; the emitter's own sites go through the IR getters instead
/// (`Output::output_record_tuple`).
#[must_use]
pub fn tuple_reads(fields: &[(i64, &'static str)], db: &str, stores: &str) -> String {
    let reads: Vec<String> = fields
        .iter()
        .map(|(off, rt)| match *rt {
            "f64" => format!(
                "if {db}.rec == 0 {{ f64::NAN }} else {{ {stores}.store(&{db}).get_float({db}.rec, {db}.pos + {off}u32) }}"
            ),
            "f32" => format!(
                "if {db}.rec == 0 {{ f32::NAN }} else {{ {stores}.store(&{db}).get_single({db}.rec, {db}.pos + {off}u32) }}"
            ),
            "bool" => format!(
                "if {db}.rec == 0 {{ false }} else {{ {stores}.store(&{db}).get_byte({db}.rec, {db}.pos + {off}u32, 0) == 1 }}"
            ),
            // @PLN164 C5 — a VIEW LEAF is a reference, and the record holds that field at
            // its own offset: the leaf is the field SLOT of that record.
            VIEW_LEAF_PART => format!(
                "DbRef {{ store_nr: {db}.store_nr, rec: {db}.rec, pos: {db}.pos + {off}u32 }}"
            ),
            _ => format!(
                "if {db}.rec == 0 {{ i64::MIN }} else {{ {stores}.store(&{db}).get_int({db}.rec, {db}.pos + {off}u32) }}"
            ),
        })
        .collect();
    // The 1-tuple's trailing comma: `(x)` is a parenthesised scalar, not a tuple.
    let tail = if reads.len() == 1 { "," } else { "" };
    format!("({}{tail})", reads.join(", "))
}

/// The tuple of `layout` read out of the record `db` names — [`tuple_reads`], and for a
/// nullable layout the presence word after the fields: the record is present exactly when
/// it names one (`rec != 0`, the test `OpConvBoolFromRef` makes), and an absent record's
/// fields already read as their nulls.
#[must_use]
pub fn layout_reads(layout: &ValueTuple, db: &str, stores: &str) -> String {
    let reads = tuple_reads(&layout.fields, db, stores);
    if !layout.nullable {
        return reads;
    }
    // `(a,)` or `(a, b)` → `(a, present)` / `(a, b, present)`.
    let inner = reads[1..reads.len() - 1].trim_end_matches(',');
    format!("({inner}, {db}.rec != 0)")
}

/// The setter call that writes tuple element `value` (a Rust expression) into field
/// `(off, rt)` of the record `db` names, to be called on `stores.store_mut(&db)`: the
/// twin of [`tuple_reads`], for a site that must write a tuple back into a record it
/// holds as a `DbRef`.
#[must_use]
pub fn tuple_field_setter(off: i64, rt: &str, db: &str, value: &str) -> String {
    match rt {
        "f64" => format!("set_float({db}.rec, {db}.pos + {off}u32, {value})"),
        "f32" => format!("set_single({db}.rec, {db}.pos + {off}u32, {value})"),
        // `set_byte` takes the value as `i32`, not as the `u8` a boolean's STORAGE form
        // suggests (@PLN17) — `u8::from` here compiled to "expected `i32`, found `u8`" and
        // broke a whole cdylib, which is the only reason a library with one boolean-field
        // record could not build.
        "bool" => format!("set_byte({db}.rec, {db}.pos + {off}u32, 0, i32::from({value}))"),
        _ => format!("set_int({db}.rec, {db}.pos + {off}u32, {value})"),
    }
}

pub(crate) use crate::opt::value_records::{fits, value_shape};
pub use crate::opt::value_records::{is_null_sentinel, value_locals_in, value_records};

pub(crate) use crate::opt::value_records::leaves::collect_leaves;
pub use crate::opt::value_records::leaves::{
    VALUE_RECORD_GETTERS, VALUE_RECORD_SETTERS, leaf_source, object_own_return,
};

pub(crate) use crate::opt::value_records::sites::{ShapeCtx, namings_avoid_place, tuple_arg_form};
pub use crate::opt::value_records::sites::{
    ValueLeaves, fnref_param, forward_site, mv_return_source, tuple_arg_ready,
};

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
        // The store survives the iteration only while nothing ELSE releases it.
        let freed_elsewhere = bound_to_a_releasing_local(data, body, v);
        if freed_elsewhere {
            if trace {
                eprintln!(
                    "[loop-buffer] {}: {} is bound to a local that releases its store",
                    def.name(),
                    vars.name(v)
                );
            }
            continue;
        }
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

/// `@FR-R-LoopBuffer` — is buffer `v`'s field bound to a local that releases the store itself?
/// Such a local (an owner: a value branch's binding whose other arm mints) frees the buffer's
/// store at its own scope end, so the store does not survive the iteration.
fn bound_to_a_releasing_local(data: &Data, body: &Value, v: u16) -> bool {
    let mut binders: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(w, rhs) = n
            && *w != v
            && let Value::Call(d, args) = rhs.unspan()
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpGetField"
            && matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if *b == v)
        {
            binders.insert(*w);
        }
        false
    });
    !binders.is_empty()
        && body.any_node(&mut |n| {
            matches!(n, Value::Call(d, args)
                if (*d as usize) < data.definitions.len()
                    && matches!(data.def(*d).name(),
                        "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefTag")
                    && matches!(args.first().map(Value::unspan),
                        Some(Value::Var(w)) if binders.contains(w)))
        })
}

pub use crate::opt::attrs::ret_buffer_attr;

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

pub use crate::opt::value_records::{dead_buffers, forward_sites};

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

/// The WIDENED leaves under `node`, which stands where the NULLABLE layout `key` is expected
/// (`@FR-R-ValueRecord`'s nullable clause, `@FR-N-Road`): every leaf whose own tuple is the
/// DENSE one — a `-> S` call, a dense value local, and a VIEW, whose tuple is its field reads
/// whatever its type — by both addresses.  The walk is [`value_shape`]'s own: through a value
/// block's last statement, a discharged-view return's copy source, an `Insert`'s last, a
/// `return`'s operand and BOTH arms of a branch, so a dense arm beside a nullable one is
/// widened where it stands.  A `null` exit emits the absent tuple itself, and a nullable
/// call or local already is one.
fn widen_marks(node: &Value, c: &ShapeCtx, key: u16, out: &mut HashMap<usize, u16>) {
    let mut mark = |leaf: &Value| {
        out.insert(std::ptr::from_ref(leaf) as usize, key);
        out.insert(std::ptr::from_ref(leaf.unspan()) as usize, key);
    };
    match node.unspan() {
        // A build is the tuple of the body's OWN layout: the nullable one in a body that
        // returns it, and in a dense body the dense one, present by construction.  It is
        // marked by the `Block`'s address, which is what the build's emitter is handed.
        Value::Block(bl) if bl.name == "Object" => {
            if value_shape(node, c).is_some_and(|k| !is_nullable_layout(k)) {
                out.insert(std::ptr::from_ref(&**bl) as usize, key);
            }
        }
        Value::Block(bl) if mv_return_source(bl, c.data).is_some() => {
            if let Some(src) = mv_return_source(bl, c.data) {
                widen_marks(src, c, key, out);
            }
        }
        Value::Block(bl) if !matches!(bl.result.base(), Type::Void) => {
            if let Some(last) = bl.operators.last() {
                widen_marks(last, c, key, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(last) = ops.last() {
                widen_marks(last, c, key, out);
            }
        }
        Value::Return(x) => widen_marks(x, c, key, out),
        Value::If(_, then_v, else_v) => {
            widen_marks(then_v, c, key, out);
            widen_marks(else_v, c, key, out);
        }
        // A view reads its fields and nothing else, so it is widened whatever key its type
        // names; a value local is widened when it holds the dense tuple.
        Value::Var(v)
            if c.locals.get(v).is_none_or(|k| !is_nullable_layout(*k)) && fits(node, c, key) =>
        {
            mark(node);
        }
        Value::Call(..) | Value::CallRef(..)
            if value_shape(node, c) == Some(layout_record(key)) =>
        {
            mark(node);
        }
        _ => {}
    }
}

/// Every leaf of `d_nr` the emitter widens ([`widen_marks`]), with the nullable key it is
/// widened to: under each result position of an admitted `-> S?` body, and under the right
/// of every `τ?` value local of any function.  A result an `Object` or a discharged-view
/// block returns itself is reached through the block, never through its own `return`.
fn widened_leaves(
    data: &Data,
    d_nr: u32,
    vr: &ValueRecords,
    locals: &HashMap<u16, u16>,
) -> HashMap<usize, u16> {
    let mut out = HashMap::new();
    let own = vr.fns.contains_key(&d_nr).then_some(d_nr);
    let c = ShapeCtx {
        data,
        def_nr: d_nr,
        admitted: &vr.fns,
        params: &vr.params,
        locals,
        own,
        fnref: &vr.fnref_sites,
        sp: &vr.sp,
    };
    let body = data.def(d_nr).code();
    let key = vr
        .fns
        .get(&d_nr)
        .copied()
        .filter(|k| is_nullable_layout(*k));
    let mut own_returns: HashSet<usize> = HashSet::new();
    body.any_node(&mut |n| {
        match n {
            Value::Block(bl)
                if own.is_some()
                    && (object_own_return(bl).is_some()
                        || mv_return_source(bl, data).is_some()) =>
            {
                if let Some(k) = key {
                    widen_marks(n, &c, k, &mut out);
                }
                if let Some(last) = bl.operators.last() {
                    own_returns.insert(std::ptr::from_ref(last.unspan()) as usize);
                }
            }
            Value::Return(x) if !own_returns.contains(&(std::ptr::from_ref(n) as usize)) => {
                if let Some(k) = key {
                    widen_marks(x, &c, k, &mut out);
                }
            }
            Value::Set(w, rhs) => {
                if let Some(k) = locals.get(w).copied().filter(|k| is_nullable_layout(*k)) {
                    widen_marks(rhs, &c, k, &mut out);
                }
            }
            _ => {}
        }
        false
    });
    if let Some(k) = key {
        widen_marks(body, &c, k, &mut out);
    }
    out
}

/// [`tuple_arg_form`] for the emitter: the form argument `arg` of a call in `def_nr` takes
/// at a tuple parameter of layout `tp`, over that function's value locals.
#[must_use]
pub fn arg_tuple_form(
    data: &Data,
    def_nr: u32,
    vr: &ValueRecords,
    locals: &HashMap<u16, u16>,
    arg: &Value,
    tp: u16,
) -> Option<bool> {
    let c = ShapeCtx {
        data,
        def_nr,
        admitted: &vr.fns,
        params: &vr.params,
        locals,
        own: vr.fns.contains_key(&def_nr).then_some(def_nr),
        fnref: &vr.fnref_sites,
        sp: &vr.sp,
    };
    tuple_arg_form(arg, tp, &c)
}

/// The value leaves of `def_nr` for the emitter ([`ValueLeaves`]), over its value locals
/// `locals` ([`value_locals_in`]).  A function that is not admitted has only the WIDENED
/// ones: only an admitted body converts a view or an `Object`, and any function may bind a
/// `τ?` value local from a dense tuple.
#[must_use]
pub fn value_leaves(
    data: &Data,
    def_nr: u32,
    vr: &ValueRecords,
    locals: &HashMap<u16, u16>,
) -> ValueLeaves {
    let mut leaves = if vr.fns.contains_key(&def_nr) {
        collect_leaves(data.def(def_nr).code(), locals, true, data)
    } else {
        ValueLeaves::default()
    };
    leaves.widen = widened_leaves(data, def_nr, vr, locals);
    leaves
}

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
pub fn invariant_chains(
    lp: &Block,
    data: &Data,
    vars: &crate::variables::Function,
) -> Vec<InvariantChain> {
    if lp
        .operators
        .iter()
        .any(|op| op.any_node(&mut |n| matches!(n, Value::Yield(_) | Value::Parallel(_))))
    {
        return Vec::new();
    }
    let mut banned = rebound_vars(lp, data, vars);
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

#[cfg(test)]
mod store_free_sentinel {
    use super::*;

    /// Every native op whose operands and result are all scalars — `const` ones included —
    /// that the store-free gate still reads as a writer.  Such an op reaches state only
    /// through a `const` slot, type id or jump, or it is a selector the list above forgot
    /// (`OpMathFunc2Float` was one, and declined every caller's hoist).  A new entry here is
    /// a question to answer: name it in `PURE_NULLARY_OPS`, or add it below with the reason.
    #[test]
    fn every_scalar_only_native_op_is_classified() {
        let mut p = crate::parser::Parser::new();
        p.parse_dir("default", true, false).unwrap();
        let scalar = |tp: &Type| is_scalar(tp) || matches!(tp.base(), Type::Void);
        let mut unclassified: Vec<String> = Vec::new();
        for def in &p.data.definitions {
            if !def.name().starts_with("Op") || !matches!(def.code(), Value::Null) {
                continue;
            }
            if def.attributes().iter().all(|a| is_scalar(&a.typedef))
                && scalar(def.returned())
                && !native_op_is_store_free(def)
                && !REVIEWED_WRITERS.contains(&def.name())
            {
                unclassified.push(def.name().to_string());
            }
        }
        assert!(
            unclassified.is_empty(),
            "scalar-only native ops the store-free gate reads as writers, unreviewed: {unclassified:?}"
        );
    }

    /// Scalar-signature ops that are writers, or never reach the native emitter, by group.
    const REVIEWED_WRITERS: &[&str] = &[
        // Control flow and frames — bytecode only; the native emitter never sees them as calls.
        "OpGoto",
        "OpGotoWord",
        "OpGotoFalse",
        "OpGotoFalseWord",
        "OpCall",
        "OpReturn",
        "OpFreeStack",
        "OpReserveFrame",
        "OpStaticCall",
        "OpInitCreateStack",
        // Literals: `Value::Int` and friends in the IR, an op only in bytecode.
        "OpConstInt",
        "OpConstShort",
        "OpConstTiny",
        "OpConstSingle",
        "OpConstFloat",
        "OpConstEnum",
        // Frame-slot reads and writes through a `const` position.
        "OpVarBool",
        "OpPutBool",
        "OpVarInt",
        "OpVarCharacter",
        "OpPutInt",
        "OpVarNarrow",
        "OpPutNarrow",
        "OpPutCharacter",
        "OpVarSingle",
        "OpPutSingle",
        "OpVarFloat",
        "OpPutFloat",
        "OpInitText",
        "OpVarEnum",
        "OpPutEnum",
        "OpInitRef",
        "OpInitRefSentinel",
        // The interpreter's fused superinstructions, emitted after the IR the walk reads.
        "OpIntVV",
        "OpIntVC",
        "OpCmpIntVV",
        "OpCmpIntVC",
        "OpIntVVPut",
        "OpIntVCPut",
        "OpCmpIntVVJump",
        "OpCmpIntVCJump",
        "OpTextWalkStep",
        "OpTextNullJump",
        "OpTextEndJump",
        "OpVecGetInt",
        "OpVecGetIntNullable",
        "OpVecSetInt",
        "OpVecEndJump",
        "OpTagVCJump",
        // Reach a store or another frame through the `const` channel.
        "OpDatabase",
        "OpDropFnRef",
        "OpFnRefDetachShared",
        "OpParallelBegin",
        "OpParallelArm",
        "OpParallelJoin",
        "OpCallRef",
        "OpCallRefStore",
        "OpCoroutineCreate",
        "OpCoroutineNext",
        "OpCoroutineReturn",
        "OpCoroutineYield",
    ];
}
