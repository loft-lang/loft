// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Where a collection's elements LIVE: the hidden backing (`__vdb_N` literal store or `__ref_N`
//! call buffer) a vector or tuple member is bound to, and the work-ref a construction delivers to
//! its target.

use crate::data::{Data, DefType, Type, Value};
use crate::fxhash::FxHashMap as HashMap;
use crate::variables::Function;

/// The locals a value branch hands back as its arm tails — the sources the statement form
/// `if c { x = a } else { x = b }` copies from.
///
/// Walks exactly the shapes `Scopes::sink_set_into_arms` writes out: an `if`'s two arms, a
/// block's and an `Insert`'s last operator.  It is asked only about a value that function
/// accepted, so every block on a tail path is already known to be a value block and is not
/// tested again here.  Any other node is a tail that is not a variable (a call, `null`), so it
/// names no source whose release could move.
/// The literal backings (`__vdb_N`) each vector local is bound to, anywhere in `code`: the
/// `Set(v, OpGetField(__vdb_N, …))` a vector literal lowers to.  A rebind of `v` displaces
/// whichever of them it held.
pub(super) fn vector_literal_backings(
    code: &Value,
    function: &Function,
    data: &Data,
) -> HashMap<u16, Vec<u16>> {
    let mut out: HashMap<u16, Vec<u16>> = HashMap::default();
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n.unspan()
            && let Some(b) = literal_backing_of(rhs, function, data)
        {
            let list = out.entry(*v).or_default();
            if !list.contains(&b) {
                list.push(b);
            }
        }
    });
    out
}

/// The call buffers (`__ref_N`) each vector local is bound through, anywhere in `code`: the hidden
/// return buffer a `Set(v, f(…, __ref_N))` hands the callee.  A rebind of `v` displaces the
/// value the call delivered into whichever of them it held.
pub(super) fn vector_call_buffers(code: &Value, function: &Function) -> HashMap<u16, Vec<u16>> {
    let mut out: HashMap<u16, Vec<u16>> = HashMap::default();
    code.walk(&mut |n| {
        if let Value::Set(v, rhs) = n.unspan()
            && let Some(b) = call_buffer_of(rhs, function)
        {
            let list = out.entry(*v).or_default();
            if !list.contains(&b) {
                list.push(b);
            }
        }
    });
    out
}

/// The vector return buffer (`__ref_N`) a call right-hand side hands its callee, or `None`.
pub(super) fn call_buffer_of(rhs: &Value, function: &Function) -> Option<u16> {
    let Value::Call(_, args) = rhs.unspan() else {
        return None;
    };
    args.iter().find_map(|a| match a.unspan() {
        Value::Var(b)
            if (*b as usize) < function.count() as usize
                && function.name(*b).starts_with("__ref_")
                && matches!(function.tp(*b).base(), Type::Vector(_, _)) =>
        {
            Some(*b)
        }
        _ => None,
    })
}

/// The literal backing a vector local's right-hand side binds it to — `OpGetField(__vdb_N, …)` —
/// or `None` for any other value.
pub(super) fn literal_backing_of(rhs: &Value, function: &Function, data: &Data) -> Option<u16> {
    let Value::Call(d, args) = rhs.unspan() else {
        return None;
    };
    if *d != data.def_nr("OpGetField") {
        return None;
    }
    let Some(Value::Var(b)) = args.first().map(Value::unspan) else {
        return None;
    };
    ((*b as usize) < function.count() as usize && function.name(*b).starts_with("__vdb_"))
        .then_some(*b)
}

/// The work-ref a CONSTRUCTION block hands to its target, if that is what `rhs` is.
///
/// Deliberately not a bare `Var` at the top: `x = y` between two locals deep-copies, so both
/// keep their own store and both must release. Only a block/insert whose tail is a work-ref
/// delivers the record itself.
///
/// And the TAIL must be that work-ref by name, not merely resolve to one — which is why this
/// peels with [`block_tail_var`] rather than with [`drop_bearing_source`]. The two answer
/// different questions: `drop_bearing_source` says *which slot does this copy SOURCE name*, and
/// a tuple member read names the work-ref backing that member without the block having built
/// anything. Read through it, a PROJECTION block (`u = t.0.0`, materialised through a temp)
/// looked like a construction and the member's backing was marked handed-off — while the
/// binding it was handed to is a VIEW, which drops nothing, so the resource was released by
/// nobody.
pub(super) fn construction_work_ref(rhs: &Value, function: &Function) -> Option<u16> {
    match rhs.unspan() {
        Value::Block(_) | Value::Insert(_) => {
            let v = block_tail_var(rhs)?;
            let n = function.name(v);
            (n.starts_with("__ref_") || n.starts_with("__rref_")).then_some(v)
        }
        _ => None,
    }
}

/// The backing of vector local `v` when it lives in a scope OTHER than the ones being left:
/// `v`'s one dep, a literal's `__vdb_N` or the `__ref_N` buffer a call delivered it through.
/// A keyed collection is not asked — its records are the author's to release (`H-Drop-Not`),
/// and neither is a backing that is an ARGUMENT: that is the caller's buffer, a literal backing
/// renamed onto the return buffer included, and what it holds is being handed back.  Only a local
/// the program DECLARED is asked: a compiler temp with the same dep is a vector built INSIDE that
/// store — an inner vector of a nested literal — which the outer vector owns.
pub(super) fn outer_collection_backing(
    function: &Function,
    v: u16,
    exited: &std::collections::HashSet<u16>,
    on_this_path: Option<u16>,
) -> Option<u16> {
    if !matches!(function.tp(v).base(), Type::Vector(_, _)) || function.is_compiler_generated(v) {
        return None;
    }
    // The backing the latest bind on THIS path filled, where the scan recorded one (loft#1607);
    // else the one the type names.
    let b = if let Some(b) = on_this_path {
        b
    } else {
        let [b] = function.tp(v).depend()[..] else {
            return None;
        };
        b
    };
    (is_backing_name(function, b) && !exited.contains(&b) && !function.is_argument(b)).then_some(b)
}

/// Is `b` a hidden backing a vector local's elements live in — its `__vdb_N`, or the `__ref_N`
/// buffer a call delivered it through?  Both are minted at the function's head.
fn is_backing_name(function: &Function, b: u16) -> bool {
    let n = function.name(b);
    n.starts_with("__vdb_") || n.starts_with("__ref_")
}

/// The backing a vector bind fills: `OpGetField(<backing>, …)`, the shape the parser lowers a
/// vector copy into.  `None` for any other bind.
pub(super) fn bind_backing_of(value: &Value, function: &Function, data: &Data) -> Option<u16> {
    if let Value::Call(d, args) = value.unspan()
        && data.def(*d).name() == "OpGetField"
        && let Some(Value::Var(b)) = args.first().map(Value::unspan)
        && is_backing_name(function, *b)
    {
        return Some(*b);
    }
    None
}

/// The backing a tuple member's value lives in, where that is one `__vdb_N` or `__ref_N`: named in
/// the member's type directly, or through the compiler temp a whole-tuple bind copied a vector
/// member into (`_tupcopy_N`, loft#1361), whose own one dep is the copy's backing.
pub(super) fn member_backing(function: &Function, member: &Type) -> Option<u16> {
    let [b] = member.depend()[..] else {
        return None;
    };
    let n = function.name(b);
    if n.starts_with("__vdb_") || n.starts_with("__ref_") {
        return Some(b);
    }
    if function.is_compiler_generated(b) && matches!(function.tp(b).base(), Type::Vector(_, _)) {
        let [inner] = function.tp(b).depend()[..] else {
            return None;
        };
        let n = function.name(inner);
        return (n.starts_with("__vdb_") || n.starts_with("__ref_")).then_some(inner);
    }
    None
}

/// The work-ref whose record `rhs` delivers to its target: a construction bound directly
/// ([`construction_work_ref`]), or one handed to a function whose body hands that parameter
/// back WHOLE — `s = me(Bx { … })` with `fn me(self: Bx) -> Bx { self }`, where the call returns
/// the construction's own store and the binding adopts it.
///
/// Both deliver one record that the binding and the work-ref then both name, so every reader
/// that asks *"who owns what this value hands over?"* — the drop hand-off, the disarm, the view
/// test — asks it here.  Answered for the direct shape alone, the work-ref kept claiming the
/// store the binding adopted through the call: in a loop its reuse re-initialised that store
/// before a literal that read the binding had read it (loft#1575), and a droppable's hook ran
/// through both claimants.
pub(super) fn delivered_work_ref(rhs: &Value, function: &Function, data: &Data) -> Option<u16> {
    construction_work_ref(rhs, function).or_else(|| {
        let mut tail = rhs.unspan();
        loop {
            match tail {
                Value::Block(bl) => tail = bl.operators.last()?.unspan(),
                Value::Insert(ops) => tail = ops.last()?.unspan(),
                _ => break,
            }
        }
        let Value::Call(d, args) = tail else {
            return None;
        };
        if *d >= data.definitions() {
            return None;
        }
        let def = data.def(*d);
        // Only a function whose body hands a PARAMETER back WHOLE delivers that argument's
        // record.  A projection op (`OpGetField`) also names its argument in its return deps,
        // and so does a function whose promoted local IS the hidden return buffer — and read
        // as delivered, the first made a view of a member look like the construction and the
        // second disarmed the caller's own buffer, which then leaked.
        if def.def_type != DefType::Function || def.name().starts_with("Op") {
            return None;
        }
        let mut body = def.code.unspan();
        loop {
            match body {
                Value::Block(bl) => body = bl.operators.last()?.unspan(),
                Value::Insert(ops) => body = ops.last()?.unspan(),
                Value::Return(v) => body = v.unspan(),
                _ => break,
            }
        }
        let Value::Var(returned) = body else {
            return None;
        };
        // The argument is the construction itself while the scan reads it — its block, whose
        // tail is the work-ref — or, once lifted, that work-ref by name.
        def.returned.depend().iter().find_map(|&k| {
            if k != *returned
                || def
                    .attributes()
                    .get(usize::from(k))
                    .is_none_or(|a| a.hidden)
            {
                return None;
            }
            let arg = args.get(usize::from(k))?;
            if let Value::Var(w) = arg.unspan() {
                let n = function.name(*w);
                return (n.starts_with("__ref_") || n.starts_with("__rref_")).then_some(*w);
            }
            construction_work_ref(arg, function)
        })
    })
}

/// The work-refs whose records a value's CONSTRUCTIONS hand to its target: every arm's when
/// `rhs` is a branch join, and [`construction_work_ref`]'s single answer otherwise.
///
/// A join delivers the value of the ONE arm that ran.  On that path the binding adopts that
/// arm's store; on every other path that arm's construction never ran, so its work-ref holds
/// nothing — null, or the sentinel an earlier pass's disarm left.  So every work-ref listed may
/// be handed off, and disarmed after the join, whichever arm ran (`@FR-O-Complete`).  An arm
/// whose tail is not a construction — a call adopted through its own buffer, a lifted local —
/// contributes nothing: its release is decided where that spelling is.
pub(super) fn construction_work_refs(rhs: &Value, function: &Function, data: &Data) -> Vec<u16> {
    match rhs.unspan() {
        Value::If(_, t, e) => {
            let mut out = construction_work_refs(t, function, data);
            for w in construction_work_refs(e, function, data) {
                if !out.contains(&w) {
                    out.push(w);
                }
            }
            out
        }
        // A value block whose TAIL is a join — an arm wrapper, or a `match` lowered behind its
        // subject binding — is the same join one wrapper down.
        Value::Block(bl)
            if matches!(bl.operators.last().map(Value::unspan), Some(Value::If(..))) =>
        {
            bl.operators.last().map_or_else(Vec::new, |tail| {
                construction_work_refs(tail, function, data)
            })
        }
        _ => delivered_work_ref(rhs, function, data)
            .into_iter()
            .collect(),
    }
}

/// The tuple MEMBER a value reads — `(base variable, member index)` — looking through the
/// block wrappers a lowering may have put around it.
///
/// A projection reaches a site either as a bare `TupleGet` or as a lowered block whose TAIL is
/// one: `t.0.0.0` materialises each level into a temp, so the second level's source is a block
/// ending in the first level's read.  Both spellings name the same member, and a site that
/// matched only the bare one saw a nested projection as naming nothing (`formal/heap.md`
/// D-heap-1: a nested member RETURNED released twice).
///
/// Lives here rather than beside `Value` in `data.rs` so the `unspan` audit can see it — that
/// audit skips `data.rs`, and a shape-discriminating helper hidden from the instrument that
/// asks *who reads a `Value` shape without peeling `Span`* is the one place it should not sit.
/// `pub(crate)` because the PARSER records the projection and the scope pass reads it back, so
/// the two must agree about which shapes count as one.
pub(crate) fn tuple_projection_of(v: &Value) -> Option<(u16, u16)> {
    match v.unspan() {
        Value::TupleGet(base, i) => Some((*base, *i)),
        Value::Block(bl) => bl.operators.last().and_then(tuple_projection_of),
        Value::Insert(ops) => ops.last().and_then(tuple_projection_of),
        _ => None,
    }
}

/// The variable a block or insert ENDS in, looking through nesting — its tail read as a bare
/// `Var`, and `None` for a tail that is anything else.
fn block_tail_var(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(var) => Some(*var),
        Value::Block(bl) => bl.operators.last().and_then(block_tail_var),
        Value::Insert(ops) => ops.last().and_then(block_tail_var),
        _ => None,
    }
}
