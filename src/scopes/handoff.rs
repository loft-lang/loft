// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Which side of a copy keeps the DROP: whether a whole-value copy, a collection copy or a copy
//! into a container hands the release of what it copies over, and the per-path flags that record
//! a hand-off made on only some paths.

use super::Scopes;
use super::backings::construction_work_refs;
use super::drops::drop_hook;
use super::par_writes::accessor_root_var;
use crate::data::{Data, Type, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;
use std::collections::{BTreeMap, BTreeSet};

/// The variable whose scope-end DROP a plain whole-value copy `v = src` takes away, or
/// `None` where the copy moves nothing.  `@FR-H-Drop`: responsibility moves with a copy.
///
/// `t = s` deep-copies (`@FR-B-Copy`) and leaves two records holding one resource — the
/// failure C111 names for a container and answers with a MOVE — so the copy owns the
/// resource and the SOURCE stops dropping.  Only the drop moves; both stores are still freed
/// by the ordinary sweep.  A copy off a PARAMETER runs the other way: the caller owns the
/// resource (calls.md F-ParamHeap — the parameter aliases it), so it is the callee's copy,
/// `v`, that never drops.
///
/// The fact belongs to the ASSIGNMENT, not the variable (`@FR-O-Latest`): a source rebound
/// after the copy releases the record it displaces through [`Scopes::displaced_drop`] — no,
/// it does not: that record's resource is the copy's now, so the rebind SKIPS it and only
/// retires the hand-off, and the source's NEW record is its own again.  A copy rebound later
/// releases the record it displaces the same way.  The scan keeps that order
/// (`drop_transferred` is re-armed at every hand-off a statement makes and retired at an
/// unconditional reassignment), which is what lets this predicate ignore how often either
/// side is assigned.  A captured variable is left alone — the closure record shares its
/// slot.
///
/// A COMPILER BUFFER destination (`buffer_dst`: a `__ref_N` return buffer, the `__ref_p2_N`
/// a materialised branch arm is copied into) is exempt from the not-an-argument test — a
/// return buffer IS an argument (the caller's, adopted at the return), and its record is
/// released with the cascade at its own free or by the caller that adopts it.  That is how
/// `t = s; return t` releases once, in the caller.  A local PROMOTED onto the return buffer
/// is the same destination under the local's name ([`promoted_ret_buffer`]), so
/// `x = mk(1); x = a; return x` moves `a`'s release to the caller too.
///
/// One home for the three sites that see a whole-value copy: [`collect_drop_transferred`]
/// (the parser's `Set(v, Var(src))` and its `OpCopyRecord` into a buffer), the branch-arm
/// lift (`__lift_N = a`, built after the collector ran) and the double-move lint, so none
/// of them can disagree about which copies move the drop.
pub(crate) fn copy_moves_drop_from(
    function: &Function,
    data: &Data,
    v: u16,
    src: u16,
    buffer_dst: bool,
) -> Option<u16> {
    if v == src
        || (!buffer_dst
            && function.is_argument(v)
            && !promoted_ret_buffer(data, data.def_nr(&function.name), function, v))
        || function.is_captured(v)
        || function.is_captured(src)
    {
        return None;
    }
    if !copy_carries_drop(function, data, v, function.tp(src)) {
        return None;
    }
    // A copy of what the CALLER holds — a parameter, or a local holding the caller's record on
    // every path (`Function::holds_caller_record`) — moves no release: the rules refuse it
    // (`(H-Copy-Refuse)`) or it takes a lease of its own (@FR-H-Copy-Lease), and either way each
    // structure drops for itself.  What moves is a value this function OWNS (`(H-Move)`): the
    // copy releases it and the source stops.
    if function.is_argument(src) || function.holds_caller_record(src) {
        return None;
    }
    Some(src)
}

/// Is `v` a local PROMOTED onto function `d_nr`'s hidden return buffer — `x = …; return x`
/// becomes `fn f(…, x: H)`, so `x` occupies the argument slot the caller hands for the result?
///
/// An argument by slot only: this frame binds what it holds, and the caller adopts the record
/// at the return.  So its assignments displace a record as a plain local's do
/// ([`Scopes::displaced_drop`]), and a copy into it moves a release as a copy into any buffer
/// does ([`copy_moves_drop_from`]).
pub(super) fn promoted_ret_buffer(data: &Data, d_nr: u32, function: &Function, v: u16) -> bool {
    let n = function.name(v);
    n != "__retbuf"
        && d_nr != u32::MAX
        && data
            .def(d_nr)
            .attr_names
            .get(n)
            .is_some_and(|&a| data.def(d_nr).attributes()[a].hidden)
}

/// Does copying a value of type `src_tp` into `v` carry a DROP with it?
///
/// The TYPE half of [`copy_moves_drop_from`], shared with the tuple-member arm of
/// [`drop_handoff_node`], which reads its source's type off the tuple rather than off a
/// variable of its own — a tuple member has no variable in this frame when the tuple is a
/// PARAMETER. One home so the two cannot disagree about which copies move a release: the
/// destination must be a heap record, the source must copy AS that record (`copies_as` admits a
/// variant widening into its parent enum), and the record's type must actually have a cascade —
/// a type that `owns_droppable` says yes about with no cascade gets the answer of a type owning
/// nothing.
fn copy_carries_drop(function: &Function, data: &Data, v: u16, src_tp: &Type) -> bool {
    let Some(d) = function.tp(v).base().heap_def_nr() else {
        return false;
    };
    let Some(sd) = src_tp.base().heap_def_nr() else {
        return false;
    };
    data.copies_as(d, sd) && data.drop_cascade_nr(d) != u32::MAX
}

/// Does the plain bind `v = src` between two record locals make `v` the OWNER of its copy, so
/// that the binding names no store as a dep?  Both must be the same record — through `base()`,
/// because `S?` is the same storage behind a nullability marker (`@FR-L-Null`).
///
/// Not for a CAPTURED or never-free `v` (`@FR-L-CapHeap`): a captured heap value is SHARED — the
/// closure holds the store at capture time — so making it an owner frees a store the closure
/// still reads (`x: S? = a; f = fn(){ x }; x = x.next` then `f()` read null).
///
/// One home for the two sites that ask it about the same copy: the bind itself, and a
/// reassignment written out per arm, which strips its arm tails before the first arm reads the
/// binding's type.
///
/// Which record pairs copy is `Data::copies_as`'s question, as at every other site that decides
/// the copy: the same def, a variant widened into its enum, or the narrowing a variant cast
/// proves.  Asked as `d_nr == src_d`, `bl = if q is Circle { q as Circle } else { … }`
/// re-bound over an owned `bl` kept the join's borrow of `q`, and the interpreter's re-bind
/// then aliased it where native copied.
pub(super) fn var_copy_owns(function: &Function, data: &Data, v: u16, src: u16) -> bool {
    let (Type::Reference(d_nr, _) | Type::Enum(d_nr, true, _)) = function.tp(v).base() else {
        return false;
    };
    let (Type::Reference(src_d, _) | Type::Enum(src_d, true, _)) = function.tp(src).base() else {
        return false;
    };
    data.copies_as(*d_nr, *src_d) && !function.is_captured(v) && !function.is_skip_free(v)
}

/// The `(destination, source)` pairs of whole-value copies written inside a branch ARM.
///
/// A copy that only some runs perform cannot move a release on all of them
/// (`ownership.md (O-Complete)`), and this is how those copies are found: structurally, off
/// the pre-scan IR, before any arm lift exists — so every pair here is one an author wrote,
/// `if c { x = a; } else { x = b; }`, rather than one the branch lowering produced.
///
/// Only a bare `Var` right-hand side qualifies: anything else is a value with no source
/// variable whose release could have moved.
pub(super) fn per_path_handoffs(code: &Value) -> HashSet<(u16, u16)> {
    let mut out: HashSet<(u16, u16)> = HashSet::default();
    fn arm(a: &Value, out: &mut HashSet<(u16, u16)>) {
        a.walk(&mut |m| {
            if let Value::Set(v, rhs) = m.unspan()
                && let Value::Var(src) = rhs.unspan()
            {
                out.insert((*v, *src));
            }
        });
    }
    code.walk(&mut |n| {
        if let Value::If(_, t, e) = n.unspan() {
            arm(t, &mut out);
            arm(e, &mut out);
        }
    });
    out
}

/// The whole-value copies written inside a LOOP body that stop their DESTINATION — a copy off a
/// parameter, or off a local that holds the caller's record ([`per_path_stops`]).  In a loop the
/// record such a copy leaves in its destination is displaced by the next pass, so whether a
/// displaced record is the frame's to release is a per-iteration fact, answered by the same flag an
/// arm copy gets.  A copy that moves its SOURCE's release is not returned: the loop's early seed
/// keeps deciding that one.
pub(super) fn loop_self_stopping_copies(
    code: &Value,
    function: &Function,
    data: &Data,
) -> HashSet<(u16, u16)> {
    let mut out: HashSet<(u16, u16)> = HashSet::default();
    code.walk(&mut |n| {
        if let Value::Loop(lp) = n.unspan() {
            for op in &lp.operators {
                op.walk(&mut |m| {
                    if let Value::Set(v, rhs) = m.unspan()
                        && let Value::Var(src) = rhs.unspan()
                        && per_path_stops(function, data, *v, *src) == Some(*v)
                    {
                        out.insert((*v, *src));
                    }
                });
            }
        }
    });
    out
}

/// Which side does a whole-value copy `dst = src` written in a branch arm stop, on the path that
/// runs it?  The answer is [`copy_moves_drop_from`]'s: the SOURCE for a copy that takes its
/// release, the DESTINATION for a copy off a PARAMETER, whose caller owns what it holds.  Either
/// way the stopped variable still owes its own release on the paths that did not run the copy, so
/// every such copy gets loft#1515's per-path flag, keyed on the side named here.
pub(super) fn per_path_stops(function: &Function, data: &Data, dst: u16, src: u16) -> Option<u16> {
    copy_moves_drop_from(function, data, dst, src, false)
}

/// The store whose elements' release a whole-collection copy hands over — `d = v`
/// (`OpAppendVector(d, v)`), `w += v`, a nested `return v` copied into the caller's buffer
/// (`OpReplaceVector(buffer, v)`) — or `None` where the copy moves nothing.
///
/// `(H-Move)` moves a collection the function owns wherever it is placed, and its elements go
/// with it; the copy made a second structure over the same resources and both released them
/// (D-heap-23).  The source is a USER local the function owns — not a parameter, which is the
/// caller's, and not a compiler temp, which answers for itself — viewing the backing that holds
/// its elements (`__vdb_N`, or the `__ref_N` a call delivered it through).  Only a backing whose
/// elements carry a hook is an answer, and never the destination's own: `v += v` copies a
/// collection into itself.
fn collection_copy_handoff(
    d_nr: u32,
    args: &[Value],
    function: &Function,
    data: &Data,
) -> Option<u16> {
    let (dst, src) = collection_copy_ends(d_nr, args, function, data)?;
    let (dst, src) = (&dst, &src);
    if dst == src || function.is_argument(*src) || function.is_compiler_generated(*src) {
        return None;
    }
    let dst_deps = function.tp(*dst).depend();
    let backing = function.tp(*src).depend().iter().copied().find(|&b| {
        let name = function.name(b);
        (name.starts_with("__vdb_") || name.starts_with("__ref_"))
            && !dst_deps.contains(&b)
            && b != *dst
    })?;
    drop_hook(function, backing, data).map(|_| backing)
}

/// The `(destination, source)` locals of a whole-collection copy: `OpAppendVector(d, v)`,
/// `OpReplaceVector(buffer, v)`, and — loft#1597, @FR-H-Move — a collection copied into the new element of
/// a vector of vectors (`OpCopyRecord(v, _elm_N, …)`, `outer += [v]`), which the outer
/// vector's cascade now releases.
fn collection_copy_ends(
    d_nr: u32,
    args: &[Value],
    function: &Function,
    data: &Data,
) -> Option<(u16, u16)> {
    if d_nr == data.def_nr("OpAppendVector") || d_nr == data.def_nr("OpReplaceVector") {
        // `@FR-H-Move` — a FIELD destination (`T { v: u }`, `o.v += u`, an enum payload) is named by its root
        // record, whose cascade releases what the copy placed there.  Read as no hand-off, the
        // source kept its release too and every element was released twice.
        let Value::Var(src) = args.get(1)?.unspan() else {
            return None;
        };
        let dst = accessor_root_var(args.first()?, data)?;
        return Some((dst, *src));
    }
    if d_nr == data.def_nr("OpCopyRecord")
        && let (Value::Var(src), Value::Var(dst)) = (args.first()?.unspan(), args.get(1)?.unspan())
        && function.name(*dst).starts_with("_elm_")
        && matches!(function.tp(*dst).base(), Type::Vector(_, _))
    {
        return Some((*dst, *src));
    }
    None
}

/// Every store a whole-collection copy may move its elements out of: the one
/// [`collection_copy_handoff`] names, and every other literal backing its source is bound to
/// (`backings`, [`vector_literal_backings`]) whose elements carry a hook.  A source's type names
/// only the backing of its LAST binding, while at the copy it may still hold an earlier one — a
/// rebind after the copy is what makes them differ.  Only the one it holds is live, since a
/// rebind releases the others and sets them to the sentinel, so marking them all is exact.
fn collection_copy_backings(
    d_nr: u32,
    args: &[Value],
    function: &Function,
    data: &Data,
    backings: &HashMap<u16, Vec<u16>>,
) -> Vec<u16> {
    let Some(first) = collection_copy_handoff(d_nr, args, function, data) else {
        return Vec::new();
    };
    let mut out = vec![first];
    let Some((dst, src)) = collection_copy_ends(d_nr, args, function, data) else {
        return out;
    };
    let (dst, src) = (&dst, &src);
    let dst_deps = function.tp(*dst).depend();
    for &b in backings.get(src).map(Vec::as_slice).unwrap_or_default() {
        if b != *dst
            && !dst_deps.contains(&b)
            && !out.contains(&b)
            && drop_hook(function, b, data).is_some()
        {
            out.push(b);
        }
    }
    out
}

/// Every store [`collection_copy_handoff`] answers for a copy anywhere in `code`.  Each is decided
/// per path, whether or not the copy sits under a branch: the backing still holds the elements
/// the copy moved until a re-mint replaces them, so its release is skipped exactly on the paths
/// the copy ran, and a re-mint gives it back (`Scopes::in_place_rebuild`).
pub(super) fn collection_handoffs(
    code: &Value,
    function: &Function,
    data: &Data,
    backings: &HashMap<u16, Vec<u16>>,
) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    code.walk(&mut |n| {
        if let Value::Call(d, args) = n.unspan() {
            for backing in collection_copy_backings(*d, args, function, data, backings) {
                if !out.contains(&backing) {
                    out.push(backing);
                }
            }
        }
    });
    out.sort_unstable();
    out
}

/// The user variables a copy hands to a CONTAINER inside a branch ARM, whose type owns a
/// droppable — a field or element write, an append, a literal's field, a return buffer
/// ([`copy_record_handoff`]).
///
/// Such a copy runs on some paths only, so the source's release cannot stop statically: on the
/// path that did not take the arm the source still owns its record and must release it
/// (`heap.md (H-Spent)`'s per-path clause, D-heap-14).  A compiler temp is left to the static
/// answer — a construction's work-ref, a call's buffer — because it holds nothing on the paths
/// that did not fill it; a parameter is the caller's to release in the first place.
pub(super) fn arm_container_handoffs(code: &Value, function: &Function, data: &Data) -> Vec<u16> {
    let copy_d = data.def_nr("OpCopyRecord");
    let mut out: Vec<u16> = Vec::new();
    if copy_d == u32::MAX {
        return out;
    }
    let arm = |a: &Value, out: &mut Vec<u16>| {
        a.walk(&mut |m| {
            let Value::Call(d, args) = m.unspan() else {
                return;
            };
            let src = if *d == copy_d && args.len() >= 3 {
                copy_record_handoff(args, function, data)
                    .filter(|&src| data.type_owns_droppable_anywhere(function.tp(src)))
            } else {
                handle_handoff(*d, args, function, data)
            };
            if let Some(src) = src
                && !function.is_compiler_generated(src)
                && !function.is_argument(src)
                && !out.contains(&src)
            {
                out.push(src);
            }
        });
    };
    code.walk(&mut |n| {
        if let Value::If(_, t, e) = n.unspan() {
            arm(t, &mut out);
            arm(e, &mut out);
        }
    });
    out.sort_unstable();
    out
}

/// The generator handle an `OpSetDbRef(host, pos, g)` MOVES into a field or an element — `g`
/// when it is an `iterator` local (loft#1585), `None` for any other store of a pointer.
fn handle_handoff(d_nr: u32, args: &[Value], function: &Function, data: &Data) -> Option<u16> {
    if d_nr != data.def_nr("OpSetDbRef") {
        return None;
    }
    let Value::Var(g) = args.get(2)?.unspan() else {
        return None;
    };
    matches!(function.tp(*g).base(), Type::Iterator(_, _)).then_some(*g)
}

/// The locals that hold the CALLER's record on every path ([`Function::mark_caller_record`]).
///
/// A heap local qualifies when each assignment that binds a store is a whole-value copy off a
/// parameter or off another qualifying local, and nothing can change what its record holds:
/// no write place is rooted at it (a field or element write, an in-place rebuild, an append) and
/// it is never handed bare to a call, whose body could write through it.  A write after the copy
/// (`x = p; x.h = mk()`) puts a resource the frame made into the record, so that local keeps its
/// own release.  Compiler temps are left to the scan that built them, and a capture, a loop
/// variable and a parameter are never candidates.  A fixpoint, because a copy of a qualifying
/// local qualifies only once that local does.
pub(super) fn caller_record_locals(
    code: &Value,
    function: &Function,
    data: &Data,
) -> BTreeSet<u16> {
    let mut copies: BTreeMap<u16, Vec<u16>> = BTreeMap::new();
    let mut disqualified: HashSet<u16> = HashSet::default();
    code.walk(&mut |n| match n.unspan() {
        Value::Set(t, rhs) => match rhs.unspan() {
            Value::Var(src) => copies.entry(*t).or_default().push(*src),
            r if crate::use_analysis::holds_no_store(data, r) => {}
            _ => {
                disqualified.insert(*t);
            }
        },
        Value::Call(d, args) => {
            let name = data.def(*d).name();
            let place = if name == "OpCopyRecord" {
                args.get(1)
            } else if name.starts_with("OpSet")
                || name.starts_with("OpAppend")
                || name.starts_with("OpInsert")
                || name.starts_with("OpRemove")
                || name.starts_with("OpClear")
                || name == "OpDatabase"
            {
                args.first()
            } else {
                None
            };
            if let Some(root) = place.and_then(|p| accessor_root_var(p, data)) {
                disqualified.insert(root);
            }
            if !name.starts_with("Op") {
                for a in args {
                    if let Value::Var(x) = a.unspan() {
                        disqualified.insert(*x);
                    }
                }
            }
        }
        Value::CallRef(_, args) => {
            for a in args {
                if let Value::Var(x) = a.unspan() {
                    disqualified.insert(*x);
                }
            }
        }
        _ => {}
    });
    let candidate = |v: u16| {
        (v as usize) < function.count() as usize
            && !function.name(v).starts_with("__")
            && !function.is_argument(v)
            && !function.is_captured(v)
            && !function.was_loop_var(v)
            && !disqualified.contains(&v)
            // A local of a type that declares `OpCopy` never holds the caller's record: the copy
            // that filled it took a lease of its own (@FR-H-Copy-Lease), so what it holds is this
            // function's structure, and returning or placing it MOVES it.
            && !function
                .tp(v)
                .base()
                .heap_def_nr()
                .is_some_and(|d| crate::lease::leases_whole(data, d))
    };
    let mut marked: BTreeSet<u16> = BTreeSet::new();
    loop {
        let mut grew = false;
        for (&t, srcs) in &copies {
            if marked.contains(&t) || !candidate(t) {
                continue;
            }
            let all_caller = srcs.iter().all(|&s| {
                (s as usize) < function.count() as usize
                    && !function.name(s).starts_with("__")
                    && (function.is_argument(s) || marked.contains(&s))
                    && copy_carries_drop(function, data, t, function.tp(s))
            });
            if all_caller {
                marked.insert(t);
                grew = true;
            }
        }
        if !grew {
            return marked;
        }
    }
}

/// Every whole-value copy `dst = src` between two locals in `code` whose destination can own a
/// release: a heap record whose copy carries a drop, and not an argument, a capture, a loop
/// variable or a compiler temp.  Sorted, so what is minted from these is minted in one order.
pub(super) fn local_copy_pairs(
    code: &Value,
    function: &Function,
    data: &Data,
) -> BTreeSet<(u16, u16)> {
    let mut out = BTreeSet::new();
    code.walk(&mut |n| {
        if let Value::Set(dst, rhs) = n.unspan()
            && let Value::Var(src) = rhs.unspan()
            && dst != src
            && (*dst as usize) < function.count() as usize
            && (*src as usize) < function.count() as usize
            && !function.name(*dst).starts_with("__")
            && !function.is_argument(*dst)
            && !function.is_captured(*dst)
            && !function.was_loop_var(*dst)
            && copy_carries_drop(function, data, *dst, function.tp(*src))
        {
            out.insert((*dst, *src));
        }
    });
    out
}

/// Is `v` assigned, anywhere in `code`, a whole-value copy that stops `v` itself — a copy off a
/// PARAMETER ([`per_path_stops`])?  Asked of a witnessed local before the scan, so its flag
/// exists before the first `Set` that has to write it.
pub(super) fn assigns_a_self_stopping_copy(
    code: &Value,
    function: &Function,
    data: &Data,
    v: u16,
) -> bool {
    let mut found = false;
    code.walk(&mut |n| {
        if !found
            && let Value::Set(t, rhs) = n.unspan()
            && *t == v
            && let Value::Var(src) = rhs.unspan()
            && per_path_stops(function, data, v, *src) == Some(v)
        {
            found = true;
        }
    });
    found
}

/// Which side of a whole-value copy stops dropping — `None` where the copy moves nothing.
///
/// The ONE home for the DIRECTION, because two sites decide it about the same copy and a
/// second spelling could only agree by accident: the arm lift records it from the
/// construction, and the collector's `Set` arm meets the very same `__lift_N = a` again once
/// it exists.
///
/// `per_path` is what they have to agree about. `heap.md (H-Drop)` moves the release with a
/// copy, and `ownership.md (O-Complete)` makes that fact per binding and PER PATH — so a copy
/// that happens on some runs only cannot move the release on all of them. A value `if`/`match`
/// lifts each arm's value into a temp of its own, and each temp is null until its own arm
/// assigns it: one statement records as many hand-offs as it has arms and performs one.
/// Recorded against the SOURCE, the arm that did not run left its source with nothing to
/// release it and the resource was never released at all — silently, on both backends, and
/// once per iteration inside a loop (loft#1514).
///
/// So a per-path hand-off keeps the release with the source, which is live on every path, and
/// stops the DESTINATION instead. That is the same trade this makes for a copy off a
/// parameter and for the same reason — the side that outlives the other keeps the
/// responsibility — and it needs no runtime witness, because the temp whose release it
/// removes is exactly the one that is null on the paths that did not take it.
fn handoff_target(
    function: &Function,
    data: &Data,
    dst: u16,
    src: u16,
    buffer_dst: bool,
    per_path: bool,
) -> Option<u16> {
    let moved = copy_moves_drop_from(function, data, dst, src, buffer_dst)?;
    Some(if per_path { dst } else { moved })
}

/// @PLN139 stage C — the vars that HANDED OFF what they hold, so their scope end must not
/// drop it.  Two ways a value stops being its variable's to release, both an `OpCopyRecord`:
///
/// - **the source-free bit (`0x8000`)** — "deep-copy me, then FREE my source store", the
///   move a collection element-append does for a value it knows is dead after the copy.
///   The store is gone but the variable still names it, so the scope-exit drop ran the
///   author's release on a freed record — and once the slot was recycled by the next
///   allocation, on somebody else's LIVE one. Two elements closed the second's resource
///   twice and the first's never (loft#849); `LOFT_STRICT_STORES` called it a use-after-free.
/// - **a FIELD destination (`OpGetField`)** — construction copies a droppable into a
///   container, and @PLN139 makes that a MOVE: the container's copy is the owner now, and
///   the container's death releases it through the cascade. Without this the resource is
///   released twice — once by the source at its own scope end (early, while the container
///   still holds it, which is the use-after-free @PLN138 met) and once by the cascade.
///
/// Only the DROP is suppressed. The two cases differ in what happens to the store — the
/// first has already been freed, the second keeps its own copy — so the free is left to the
/// ordinary sweep, which is null-tolerant either way.
///
/// Only a plain `Var` source can be marked: any other expression names no slot that could
/// carry a scope-exit drop.
pub(super) fn collect_drop_transferred(
    code: &Value,
    function: &Function,
    data: &Data,
    pairs: &HashSet<(u16, u16)>,
) -> HashSet<u16> {
    let mut out: HashSet<u16> = HashSet::default();
    // No arm lift has been built yet at construction time, so nothing here is per path.
    let per_path = HashSet::default();
    code.walk(&mut |n| drop_handoff_node(n, function, data, &mut out, &per_path, pairs));
    out
}

/// The variable whose RELEASE a whole-value `OpCopyRecord(src, dest, tp)` hands over, or `None`
/// where the copy moves no release — the one answer [`drop_handoff_node`]'s collector and the
/// scan's per-path flag write both read, so the two cannot disagree about which copies stop
/// what.
fn copy_record_handoff(args: &[Value], function: &Function, data: &Data) -> Option<u16> {
    // A whole-value copy into a compiler BUFFER — the per-arm `__ref_p2_N` a
    // materialised branch arm is copied into, the `__ref_N` a return delivers
    // through — is the same move as `t = s`: the buffer is freed with its cascade
    // (or adopted by a caller who runs it), so the source stops dropping.  The
    // spelling is a parser `OpCopyRecord` rather than a `Set`, which is why the
    // arm below does not see it.
    //
    // A lambda's reserved `__retbuf` is such a buffer too — its return buffer is never renamed
    // onto the returned local — and a source that is a whole-value VIEW of one compiler work-ref
    // (`x = c` of a capture, built in `__ref_N` with `x` viewing it) hands off from that work-ref,
    // which is the variable its release is emitted on.  Returning it is a move (`(H-Move)`), so
    // the release goes with the value into the buffer instead of running at the lambda's end
    // (@PLN163: a leased capture copy returned from a lambda was released twice).
    if let Some(src) = drop_bearing_source(&args[0], function)
        && let Value::Var(dst) = args[1].unspan()
        && (function.name(*dst).starts_with("__ref")
            || function.name(*dst) == "__retbuf"
            // A local PROMOTED onto the return buffer is that buffer under its own name, so
            // a copy into it moves the release the same way (loft#1934: `return b` of a
            // `-> S?` whose other local took the buffer copied `b` into it and still ran
            // `b`'s drop).
            || promoted_ret_buffer(data, data.def_nr(&function.name), function, *dst))
    {
        let src = match function.tp(src).depend().as_slice() {
            [w] if *w != src
                && !function.is_argument(src)
                && function.name(*w).starts_with("__ref_") =>
            {
                *w
            }
            _ => src,
        };
        if let Some(moved) = copy_moves_drop_from(function, data, *dst, src, true) {
            return Some(moved);
        }
    }
    if !copy_record_moves_source(args, function, data) {
        return None;
    }
    drop_bearing_source(&args[0], function)
}

/// Does `OpCopyRecord(src, dest, tp)` hand its SOURCE's release over to the destination?  The
/// `0x8000` move into a collection element, a destination that is a container FIELD, or one that
/// is an element being appended — each is released by the container's cascade.  One home for the
/// collector ([`copy_record_handoff`]) and the lift site (`Scopes::scan_args`), which meets the
/// same copy after its source call became a `__lift_N`: asked at two sites, the lift once missed
/// the appended element and the temp dropped the resource the element now holds (loft#1934).
pub(super) fn copy_record_moves_source(args: &[Value], function: &Function, data: &Data) -> bool {
    matches!(args.get(2).map(Value::unspan), Some(Value::Int(tp)) if tp & 0x8000 != 0)
        || args.get(1).is_some_and(|d| {
            copy_hands_off(d, function, data) || appends_to_element(d, function, data)
        })
}

/// The hand-offs ONE node makes, added to `out` — the body of [`collect_drop_transferred`],
/// which the scan re-applies statement by statement so a variable handed off AFTER a
/// reassignment retired it is armed again in scan order (the fact belongs to the
/// assignment, `@FR-O-Latest`).
pub(super) fn drop_handoff_node(
    n: &Value,
    function: &Function,
    data: &Data,
    out: &mut HashSet<u16>,
    per_path: &HashSet<u16>,
    pairs: &HashSet<(u16, u16)>,
) {
    let copy_d = data.def_nr("OpCopyRecord");
    if copy_d == u32::MAX {
        return;
    }
    {
        match n {
            // A generator HANDLE placed into a field or an element (loft#1585) MOVES there
            // (`(H-Move)`): the container's cascade frees the generator now, so the local that
            // held it frees nothing at its scope end.  One placed in a branch ARM moves only on
            // the path that ran it, which its per-path flag records (`arm_container_handoffs`).
            Value::Call(d, args)
                if let Some(g) = handle_handoff(*d, args, function, data)
                    && !pairs.contains(&(u16::MAX, g)) =>
            {
                out.insert(g);
            }
            Value::Call(d, args) if *d == copy_d && args.len() >= 3 => {
                // A copy written in a branch ARM (`pairs` holds `(u16::MAX, source)`, D-heap-14)
                // stops its source only on the path that ran it — the source's flag, set right
                // after the copy by the scan — so it never enters this set.
                if let Some(stopped) = copy_record_handoff(args, function, data)
                    && !pairs.contains(&(u16::MAX, stopped))
                {
                    out.insert(stopped);
                }
            }
            // A CONSTRUCTION block delivers its work-ref's record to the binding rather than
            // copying it, so the two then name ONE record and only the binding owns it.
            // Without this both released it: a struct-enum literal always takes the work-ref
            // path (its declared type is the enum, the constructed one the variant, so the
            // record cannot be built in place) and `w: W = WH { h: c }` cascaded twice.
            Value::Set(v, rhs) => {
                // loft#1510 / D-heap-4 — hand the work-ref's drop over ONLY to a binding
                // that will actually run it.  The premise of this transfer is "only the
                // binding owns it", and for a view-typed (dep-carrying) binding that premise
                // fails: its scope end runs no drop, so the transfer loses the hook and the
                // resource is released by nobody's cascade.  Such a binding keeps the record
                // through the work-ref, whose own scope-end drop+free then covers it (or the
                // owned→view transition free releases it earlier and disarms the work-ref —
                // `Scopes::construction_backing`).
                //
                // @FR-O-Proxy asks free — the answer places the scope-end DROP (suppress the
                // work-ref's, because the binding's own release covers the record), and the
                // @FR-O-Override veto rides inside `proxy_says_owned` as one question.
                // Every arm's construction when the value is a branch join: the binding adopts
                // the one that ran, and the others hold nothing (`construction_work_refs`).
                if function.proxy_says_owned(*v) {
                    for w in construction_work_refs(rhs, function, data) {
                        if w != *v {
                            out.insert(w);
                        }
                    }
                }
                // A plain WHOLE-VALUE copy between two locals — `t = s`, `h2 = h` — moves
                // the drop to the copy.  [`handoff_target`] decides WHICH side stops, because
                // this arm also meets the branch-arm lift's own `__lift_N = a` once that
                // exists, and there the answer runs the other way.
                //
                // A copy written in a branch ARM (`pairs`, loft#1515) stops its source — or its
                // destination, off a parameter — only on the path that ran it, and that fact is
                // the stopped side's flag.  It never enters this set, which means "stopped on
                // every path": a later unconditional hand-off of the same variable still has to
                // stop it, and a later rebind still has to release what it displaces on the
                // path where the copy did not run.
                if let Value::Var(src) = rhs.unspan()
                    && !pairs.contains(&(*v, *src))
                    && let Some(moved) =
                        handoff_target(function, data, *v, *src, false, per_path.contains(v))
                {
                    out.insert(moved);
                }
            }
            _ => {}
        }
    }
}

/// Does a copy into `dest` hand the source's OWNERSHIP over — i.e. will something else
/// release it?  True for a PLACE reached from a root variable through field reads and
/// vector element reads, at any depth (`o.h`, `o.s.h`, `v[0].h`, `o.items[i]`), when the
/// root's type owns a droppable anywhere: its cascade recurses through fields and
/// elements, so it reaches that place.  Read one level only, `o.s = S {…}` copied into
/// the nested `o.s.h` and `v[0] = S {…}` into an element were not hand-offs, and the
/// literal's work-ref released the resource a second time beside the container's cascade.
///
/// A path through a KEYED read is never a hand-off: a keyed collection does not release
/// its records (`@FR-H-Drop-Not`), so the source keeps dropping there.
pub(crate) fn copy_hands_off(dest: &Value, function: &Function, data: &Data) -> bool {
    let get_field_d = data.def_nr("OpGetField");
    let get_vector_d = data.def_nr("OpGetVector");
    let vector_ref_d = data.def_nr("OpVectorRef");
    if get_field_d == u32::MAX {
        return false;
    }
    let mut cur = dest;
    loop {
        let Value::Call(d, args) = cur.unspan() else {
            return false;
        };
        if *d != get_field_d && *d != get_vector_d && *d != vector_ref_d {
            return false;
        }
        match args.first().map(Value::unspan) {
            Some(Value::Var(cv)) => {
                return data.type_owns_droppable_anywhere(function.tp(*cv).base());
            }
            Some(inner) => cur = inner,
            None => return false,
        }
    }
}

/// Does a copy into `dest` hand ownership to a COLLECTION element?
///
/// `_elm_N` is the element `OpNewRecord` hands back, so a copy into it is the element-append.
/// The releaser is not the element's own type but the COLLECTION's cascade, which walks every
/// element — and that loop is emitted exactly when the element type owns a droppable, so that
/// is the condition to test.
///
/// Needed beside the `0x8000` case, which only fires when the source is dead after the copy.
/// A NAMED local appended to a collection (`v: vector<H> = [h1, h2]`) stays live, so no move
/// bit is set — and once the collection releases its elements, leaving the local dropping too
/// means one resource released twice.
pub(crate) fn appends_to_element(dest: &Value, function: &Function, data: &Data) -> bool {
    let Value::Var(dv) = dest.unspan() else {
        return false;
    };
    if !function.name(*dv).starts_with("_elm_") {
        return false;
    }
    match function.tp(*dv).base() {
        Type::Reference(ed, _) | Type::Enum(ed, true, _) => data.owns_droppable(*ed),
        _ => false,
    }
}

/// The variable a copy SOURCE ultimately names, or `None` when it names no slot.
///
/// A plain `Var` is the named-local case. An `Object` construction reaches here as the block
/// that BUILDS it, whose tail is the work-ref holding the finished record — `Nest { s: S { … } }`
/// copies such a block into `Nest`'s field, and without peeling it the inner `S` temp kept a
/// scope-exit drop and released the payload a second time.
///
/// A tuple MEMBER read names a slot too, and it is the third spelling of a copy source rather
/// than a fourth kind of thing: `layout.md (L-Tuple)` makes a tuple a synthetic struct, and a
/// heap member's stack word is the handle of a work-ref the tuple's own type names
/// (`(ref(S)["__ref_1"], integer)`). So `u = t` — lowered onto the per-member copy since
/// loft#1361 — copies `t`'s member record into `u`'s, and the source it displaces is that
/// work-ref. Without this arm the copy named no slot, both members kept a scope-exit drop,
/// and one resource was released TWICE while `(B-Copy)` and `heap.md (H-Drop)` between them
/// say a copy MOVES the single release to the copy.
pub(crate) fn drop_bearing_source(src: &Value, function: &Function) -> Option<u16> {
    match src.unspan() {
        Value::Var(v) => Some(*v),
        Value::TupleGet(base, i) => tuple_member_backing(*base, *i, function),
        Value::Block(bl) => bl
            .operators
            .last()
            .and_then(|v| drop_bearing_source(v, function)),
        Value::Insert(ops) => ops.last().and_then(|v| drop_bearing_source(v, function)),
        _ => None,
    }
}

/// The work-ref backing the tuple member a copy reads, or `None` when that member is not a
/// heap record — a scalar member is stored inline and has no slot of its own to release.
///
/// The tuple's TYPE is where the pairing lives, and reading it takes two steps of care.
///
/// **Which tuple.** A nested tuple's copy reads each heap leaf through a HOLD, so the `base`
/// a copy names is not always the tuple whose type carries the pairing: `u = t` over
/// `t = ((s, 1), 2)` copies from `_tuphold_1.0` where `_tuphold_1 = t.0`. A hold's own type
/// cannot answer this — its dep list names the base variable and nothing else — so
/// [`tuple_copy_source_path`] walks `Function::tuphold_origin` to the tuple the chain starts
/// from and the PATH to the leaf inside it.
///
/// **Which dep.** Asked twice, because the pairing survives in two different states.
///
/// `Vars::tuple_backings` has it as the members themselves spelled it — one backing per heap
/// leaf, recorded before a tuple variable was given the union of its members' deps. That is
/// the answer wherever it exists, and it needs no reasoning about the list at all.
///
/// The dep LIST is the older read, for a tuple that already carried the union by the time its
/// variable was typed. The lists are UNIONED across the heap members and spread back into
/// every one of them, so each leaf carries the same list and `(WS, integer, WT)` prints as
/// `(ref(WS)["__ref_1", "__ref_2"], integer, ref(WT)["__ref_1", "__ref_2"])`. The list is in
/// MEMBER order and recurses into a nested tuple in that same order, so the backing of a leaf
/// is the dep at the number of heap leaves before it in pre-order — `__ref_2` for the `WT`
/// above, not the `__ref_1` that `first()` answers. Member order, not the order the work-refs
/// were minted in: `(a, (b, 2))` lists the OUTER member's dep first although the inner
/// literal's work-ref was created first, which is what says the index is positional and not a
/// happy accident of the numbering.
///
/// The count is what makes that positional read safe rather than a convention this function
/// hopes for: if the list is not exactly as long as the tuple's heap leaves, the order it
/// would be indexed by is not established, so this DECLINES instead of naming a work-ref it
/// guessed. Declining costs the hand-off (the pre-loft#1361 double release) and never
/// suppresses the release of a member that is still live. That is also why the count read
/// cannot be the ONLY one: a `text` or a value-enum member carries a dep without being a heap
/// leaf, so the list outruns the walk and every such tuple declined — which is the half
/// `tuple_backings` exists to answer.
fn tuple_member_backing(base: u16, i: u16, function: &Function) -> Option<u16> {
    let (root, path) = tuple_copy_source_path(base, i, function);
    // A PARAMETER's members are the CALLER's, and its deps are not frame variables of this
    // function at all — reading one as a local's number would suppress the release of
    // whatever local happens to wear that number.  The parameter rule is the one that
    // applies here anyway: a copy off an argument leaves the caller as the owner
    // (`copy_moves_drop_from`), which is a decision about the argument, not its member.
    if function.is_argument(root) {
        return None;
    }
    let root_tp = function.tp(root);
    let (leaf, ordinal) = tuple_leaf_at(root_tp, &path)?;
    if !crate::data::is_dbref(leaf.base()) {
        return None;
    }
    // The pairing as the members themselves spelled it, recorded before the union was
    // written over them (`Vars::tuple_backings`).  It is the same question the dep list is
    // read for below, answered without having to infer which entry belongs to this leaf.
    if let Some(dep) = function
        .tuple_backings
        .get(&root)
        .and_then(|b| b.get(ordinal).copied())
        && dep != u16::MAX
    {
        return Some(dep);
    }
    let deps = match leaf.base() {
        Type::Reference(_, deps) | Type::Enum(_, true, deps) => deps,
        _ => return None,
    };
    if deps.len() != crate::data::tuple_heap_leaves(root_tp) {
        return None;
    }
    let dep = *deps.get(ordinal)?;
    (dep != u16::MAX).then_some(dep)
}

/// The tuple a copy's member source ultimately reads, and the path of member indices from it
/// down to the leaf being copied.
///
/// One `_tuphold_N` per level of nesting, each projected from the level above, so the chain
/// is as deep as the tuple. It cannot loop: a hold is minted after the tuple it projects, so
/// its variable number is the larger of the two, and the walk requires that — a table saying
/// otherwise would be a cycle, and stopping there answers with the tuple reached so far,
/// whose own dep list is then checked like any other.
fn tuple_copy_source_path(base: u16, i: u16, function: &Function) -> (u16, Vec<u16>) {
    let mut path = vec![i];
    let mut cur = base;
    while let Some(&(parent, member)) = function.tuphold_origin.get(&cur) {
        if parent >= cur {
            break;
        }
        // A hold projected from a whole VARIABLE rather than a member is the same tuple one
        // name further out, so it adds no step to the path.
        if let Some(m) = member {
            path.push(m);
        }
        cur = parent;
    }
    path.reverse();
    (cur, path)
}

/// The member `path` names inside tuple type `tp`, with its ordinal among the tuple's heap
/// leaves in pre-order — `None` when the path does not reach a member.
fn tuple_leaf_at<'a>(tp: &'a Type, path: &[u16]) -> Option<(&'a Type, usize)> {
    let Type::Tuple(elems) = tp.base() else {
        return None;
    };
    let (i, rest) = path.split_first()?;
    let before: usize = elems
        .iter()
        .take(usize::from(*i))
        .map(crate::data::tuple_heap_leaves)
        .sum();
    let elem = elems.get(usize::from(*i))?;
    if rest.is_empty() {
        return Some((elem, before));
    }
    let (leaf, inner) = tuple_leaf_at(elem, rest)?;
    Some((leaf, before + inner))
}

impl Scopes<'_> {
    /// The per-path flags a copy sets when it runs, empty where the copy hands nothing over per
    /// path: an `OpCopyRecord` written in a branch arm (`arm_container_handoffs`, D-heap-14), or
    /// a whole-collection copy of a collection the function owns (`collection_handoffs`,
    /// D-heap-23), which sets one flag per store it may have moved out of
    /// ([`collection_copy_backings`]).
    pub(super) fn arm_handoff_flags(
        &self,
        d_nr: u32,
        args: &[Value],
        function: &Function,
        data: &Data,
    ) -> Vec<u16> {
        let stopped = if d_nr == data.def_nr("OpCopyRecord") && args.len() >= 3 {
            match copy_record_handoff(args, function, data) {
                Some(s) => vec![s],
                // A collection copied into a vector-of-vectors element (loft#1597).
                None => collection_copy_backings(d_nr, args, function, data, &self.vector_backings),
            }
        } else if let Some(g) = handle_handoff(d_nr, args, function, data) {
            vec![g]
        } else {
            collection_copy_backings(d_nr, args, function, data, &self.vector_backings)
        };
        stopped
            .into_iter()
            .filter(|s| self.per_path_pairs.contains(&(u16::MAX, *s)))
            .filter_map(|s| self.handed_off.get(&s).copied())
            .collect()
    }

    pub(super) fn mint_handoff_flag(&mut self, function: &mut Function, var: u16) -> u16 {
        if let Some(&flag) = self.handed_off.get(&var) {
            return flag;
        }
        let name = format!("__hoff_{}", function.name(var));
        let flag = function.add_temp_var(&name, &Type::Boolean);
        self.var_scope.insert(flag, 0);
        self.var_order.push(flag);
        self.handed_off.insert(var, flag);
        flag
    }
}
