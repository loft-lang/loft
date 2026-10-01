// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! Closure records that ADOPT a captured store: which record keeps a store several records
//! captured, which records leave the frame, and the frees a record owes on the arms that do not
//! hand it out.

use super::capture_builds::{CaptureBuilds, capture_build_backings};
use super::insert_free::expr_ends_in_return;
use super::returns::{collect_return_sources, last_non_free_result};
use crate::data::{Data, DefType, Type, Value};
use crate::fxhash::FxHashMap as HashMap;
use crate::variables::Function;

/// #682 — decide, per closure-record capture, whether the record ADOPTS the
/// captured store (`free_named`'s cascade reclaims it) or merely BORROWS it, and
/// mark the borrowed ones on the record's attribute.
///
/// Adoption exists for exactly one reason: a store the DEFINING frame owned and
/// would otherwise free at scope exit.  `get_free_vars` suppresses that free for
/// a captured reference and hands the store to the record, which is what keeps an
/// escaping factory closure's capture alive past the frame that minted it (#323).
/// A capture with no such free to hand over is a BORROW — a PARAMETER (whose
/// caller owns the store and outlives this frame) or a projection local viewing
/// into someone else's store — and cascading there is a second free of a live
/// store: the caller's value went dangling and the crash surfaced thousands of
/// ops later in whatever function next touched it.
///
/// Why here and not at record synthesis: a capture's ownership is not knowable
/// while parsing.  `ch = pick(w, 1)` parses as "borrows `w`" from the callee's
/// declared return, and only the scan above rewrites it to OWNED once it knows
/// the return ABI deep-copies into a fresh store (`make_independent`, the
/// `!adopts_fresh_store` arm).  Reading the parse-time dep leaked that copy.
///
/// The record is reached through the enclosing frame's `___clos_N` variable
/// rather than by walking the IR: `emit_lambda_code` always mints one, and it
/// lives in exactly the frame whose variables decide the verdict.
pub(super) fn mark_borrowed_captures(data: &mut Data, database: &crate::database::Stores) {
    let mut borrowed: Vec<(u32, usize)> = Vec::new();
    for d_nr in 0..data.definitions() {
        if !matches!(data.def(d_nr).def_type, DefType::Function) {
            continue;
        }
        let function = &data.def(d_nr).variables;
        let builds = capture_build_backings(data, function, data.def(d_nr).code());
        let (adopters, mut never_adopted) = capture_store_adopters(data, function, &builds);
        borrowed.append(&mut never_adopted);
        // @FR-L-CapOwn — one STORE, one owner.  Two closures over one store both adopted it,
        // and their deaths are independent: where one record escapes and the other is left
        // behind, the one left behind released the store the escaped record still holds and the
        // caller read a released record (loft#1440).  So among the records that adopted ONE
        // store, exactly one keeps it — the one that LEAVES the frame, because the frame is
        // gone by the time the question is asked; where none leaves, the first, which is the
        // single-record case unchanged.  The rest BORROW: their cascade stops there, and the
        // free-suppression is per LOCAL, so the store still has its one release.
        //
        // The grouping is by the store each record ADOPTED (`adopted_store_key`), not by the
        // capture's name: a local assigned between two builds hands the two records different
        // stores, and grouping by name made the second borrow one the first never held —
        // measured as a leaked `S` in
        // `a-captured-local-reassigned-after-the-build-frees-its-own-store.loft`.
        for (_, mut group) in adopters {
            if group.len() < 2 {
                continue;
            }
            // …unless the records CANNOT COEXIST (`@FR-L-CapOne`).  "One store, one owner"
            // answers loft#1440,
            // where two records are built in a straight line and both exist; records in opposite
            // arms of one branch are the other case, and there at most one is ever built, so each
            // is the sole owner on its own run.  Demoting either leaves the run that builds it
            // with no release at all — the frame gave its free away to a record that this run
            // never made (loft#1473).
            if group_is_pairwise_exclusive(data.def(d_nr).code(), &group)
                || group_every_record_leaves(data, function, d_nr, &group)
            {
                continue;
            }
            let owner = adoption_owner_index(data, function, d_nr, &group);
            group.remove(owner);
            for (_, record, a) in group {
                borrowed.push((record, a));
            }
        }
    }
    for (record, a) in borrowed {
        data.mark_capture_borrowed(record, a);
        strip_borrowed_capture_walk(data, database, record, a);
    }
}

/// `@FR-L-CapOne` for the HOOKS — a record that BORROWS a capture runs no drop over it.
///
/// The record's drop cascade was synthesized at parse time, before this pass decided which
/// record keeps a shared store, so it walks every capture slot.  Two closures over one
/// store then ran the captured elements' hooks once each (loft#1606).  The free-side
/// half reads the borrowed marker `mark_capture_borrowed` writes; this is the hook-side
/// half: every walk the record's cascade (and its Except variant) guards on that slot's
/// `OpGetDbRef(self, offset)` is removed.
fn strip_borrowed_capture_walk(
    data: &mut Data,
    database: &crate::database::Stores,
    record: u32,
    a: usize,
) {
    let kt = data.def(record).known_type();
    let off = database.position(kt, &data.attr_name(record, a));
    let get_dbref = data.def_nr("OpGetDbRef");
    if off == u16::MAX || get_dbref == u32::MAX {
        return;
    }
    fn guards_on_slot(cond: &Value, get_dbref: u32, off: u16) -> bool {
        let mut hit = false;
        cond.walk(&mut |n| {
            if let Value::Call(d, args) = n.unspan()
                && *d == get_dbref
                && matches!(args.get(1).map(Value::unspan), Some(Value::Int(o)) if *o == i32::from(off))
            {
                hit = true;
            }
        });
        hit
    }
    // Wherever the guarded walk sits — at the cascade's top, or under the Except variant's
    // `skip` test — the guard is replaced by nothing.
    fn scrub(v: &mut Value, get_dbref: u32, off: u16) {
        if matches!(v.unspan(), Value::If(cond, _, _) if guards_on_slot(cond, get_dbref, off)) {
            *v = Value::Null;
            return;
        }
        v.for_each_child_mut(&mut |c| scrub(c, get_dbref, off));
    }
    let cascade = data.drop_cascade_nr(record);
    if cascade != u32::MAX {
        scrub(&mut data.definitions[cascade as usize].code, get_dbref, off);
    }
}

/// Does this ARM VALUE hand out closure record `r`?
///
/// Answers `true` unless the shape is one this can read and that demonstrably names something
/// else.  The fallback is the SAFE direction and that is the whole of its design: a `true` here
/// costs a leak the caller already has, while a wrong `false` frees a record the run is handing
/// out, which is a use-after-free.  So only three shapes answer no — a bare def-number (a
/// non-capturing lambda), a `FnRef` naming a different record, and a fn-ref LOCAL whose own type
/// deps do not mention `r`.
fn arm_value_delivers_record(leaf: &Value, r: u16, function: &Function) -> bool {
    match leaf.unspan() {
        Value::Int(_) | Value::Long(_) => false,
        Value::FnRef(_, w, _) => *w == r,
        // Asked through `base()`: a `fn(…)?` local is the same twenty-byte fn-ref slot as a
        // `fn(…)`, so the wrapper is not a distinction "does this hand out record r" may make.
        // Read bare it would answer the conservative `true` for a nullable fn-ref and leave its
        // record unfreed on every omitting path.
        Value::Var(v) if (*v as usize) < function.count() as usize => {
            match function.tp(*v).base() {
                Type::Function(_, _, deps, ..) => deps.as_slice().contains(&r),
                _ => true,
            }
        }
        _ => true,
    }
}

/// Free closure record `r` inside each arm of `op` that does NOT hand it out.
///
/// `@FR-L-CapOwn` — the frame gives its release up to the record's cascade, and that handover is
/// only right on the runs where the record actually LEAVES.  Delivery through a branch is a
/// per-run fact: `if p { g1 } else { |…| 7 }` hands the record out on one path and drops it on
/// the other, while the suppression is one static decision for both, so the run that does not
/// deliver leaks the record AND the capture its cascade would have taken (measured: `n_f` emits
/// no frees at all).
///
/// Placing the free INSIDE the omitting arm is what makes it per-run without a runtime witness:
/// the arm IS the path.  On that path nothing escaped holding the record, so the cascade into
/// its capture is exactly right.
///
/// Sound only where `r` is the SOLE adopter of its store — see the caller.  Where several
/// records adopt one store, the one left behind may be the static OWNER while the delivered one
/// borrows, and freeing it would cascade into a capture the escaping record still holds.
pub(super) fn free_record_in_omitting_arms(
    op: &mut Value,
    r: u16,
    function: &Function,
    tp: &Type,
    release: &[Value],
) {
    match op {
        Value::Span(b) => free_record_in_omitting_arms(&mut b.1, r, function, tp, release),
        Value::Return(inner) | Value::Drop(inner) => {
            free_record_in_omitting_arms(inner, r, function, tp, release);
        }
        Value::If(_, t, f) => {
            free_record_in_omitting_arms(t, r, function, tp, release);
            free_record_in_omitting_arms(f, r, function, tp, release);
        }
        // A BLOCK takes the free as a STATEMENT before its value, never as a wrapper around it.
        // Wrapping produces an `Insert` standing in value position, and native then emitted the
        // free itself as the block's result (`let __ret_1: (u32, DbRef) = OpFreeRef(…)`).  A
        // preceding statement is the shape the emitters already handle everywhere.
        Value::Block(bl) => {
            let Some(last) = bl.operators.last() else {
                return;
            };
            if arm_value_delivers_record(last, r, function) {
                let idx = bl.operators.len() - 1;
                free_record_in_omitting_arms(&mut bl.operators[idx], r, function, tp, release);
            } else {
                let idx = bl.operators.len() - 1;
                for (n, op) in release.iter().enumerate() {
                    bl.operators.insert(idx + n, op.clone());
                }
            }
        }
        Value::Insert(ops) => {
            let Some(last) = ops.last() else {
                return;
            };
            if arm_value_delivers_record(last, r, function) {
                let idx = ops.len() - 1;
                free_record_in_omitting_arms(&mut ops[idx], r, function, tp, release);
            } else {
                let idx = ops.len() - 1;
                for (n, op) in release.iter().enumerate() {
                    ops.insert(idx + n, op.clone());
                }
            }
        }
        leaf => {
            if arm_value_delivers_record(leaf, r, function) {
                return;
            }
            let mut held = std::mem::replace(leaf, Value::Null);
            // WIDEN before wrapping.  The arm that omits the record is, in the common shape,
            // the one holding a NON-capturing lambda — a bare `Value::Int` def-number.  Placing
            // it inside an `Insert` makes the `Insert` the value the return hoist reads, and the
            // block-result path that used to complete the fn-ref pair no longer sees the bare
            // spelling underneath, so native emitted `let __ret_tail: (u32, DbRef) = 741_i64`.
            // One notion, two spellings, and this rewrite must hand on the complete one
            // (loft#1469's widening, applied here because this site creates the position).
            crate::parser::widen_bare_fn_ref(&mut held, tp);
            let mut ops = release.to_vec();
            ops.push(held);
            *leaf = Value::Insert(ops);
        }
    }
}

/// Is closure record `r` built anywhere inside `n`?
///
/// The build is the `FnRef(d_nr, r, _)` node `emit_lambda_code` leaves — the only thing that
/// mints into the record local — so the presence of that node IS the presence of the build.
fn subtree_builds_record(n: &Value, r: u16) -> bool {
    let mut hit = false;
    n.walk(&mut |m| {
        if let Value::FnRef(_, w, _) = m.unspan()
            && *w == r
        {
            hit = true;
        }
    });
    hit
}

/// Can closure records `a` and `b` ever exist on the SAME run?
///
/// `@FR-L-CapOne` — the coexistence condition on `@FR-L-CapOwn`'s single owner.  "One store,
/// one owner" is a statement about records that COEXIST: loft#1440
/// is two records built in a straight line, one escaping and one left behind, where the one left
/// behind released the store the escaped one still held.  Records in OPPOSITE arms of one branch
/// are the other case — at most one of them is ever built — so each is the sole owner on its own
/// run, and demoting either to a borrow leaves that run's store with no release at all.
///
/// Answers "yes, exclusive" only on a branch that builds `a` in one arm and `b` in the other and
/// NEITHER in both.  A `match` lowers to nested `If`s, so its arms are covered by the same walk;
/// two sequential `if`s are not, and must not be — `if p { k1 } if q { k2 }` over one store can
/// run both, and calling that exclusive would hand one store to two cascades.
///
/// The fallback is `false`, which is the conservative direction: an exclusive pair reported as
/// coexisting keeps today's single-owner behaviour, while a coexisting pair reported as exclusive
/// would double-free.
fn builds_are_mutually_exclusive(body: &Value, a: u16, b: u16) -> bool {
    let mut found = false;
    body.walk(&mut |n| {
        if found {
            return;
        }
        if let Value::If(_, t, f) = n.unspan() {
            let (ta, tb) = (subtree_builds_record(t, a), subtree_builds_record(t, b));
            let (fa, fb) = (subtree_builds_record(f, a), subtree_builds_record(f, b));
            if (ta && fb && !tb && !fa) || (tb && fa && !ta && !fb) {
                found = true;
                return;
            }
            // An arm that TERMINATES is exclusive with everything after the branch, not just
            // with its sibling.  `if p { return fn() { … e.a }; } return fn() { … e.a };` builds
            // one record inside the arm and one after it — never in opposite arms, so the test
            // above says nothing — yet the arm returns, so the code after it does not run on the
            // path that built the first.  Reading only sibling arms left this pair sharing an
            // owner, and the run that delivered the demoted one read a released capture
            // (loft#1477's `same_local` cell, visible only under `LOFT_POISON=1`).
            //
            // Exactly one of the two must be inside the terminating arm: two records BOTH built
            // inside it are ordinary coexisting siblings, and the arm's own termination says
            // nothing about them.
            for arm in [t, f] {
                if !expr_ends_in_return(arm) {
                    continue;
                }
                let (ia, ib) = (subtree_builds_record(arm, a), subtree_builds_record(arm, b));
                if ia != ib {
                    found = true;
                    return;
                }
            }
        }
    });
    found
}

/// Does EVERY record adopting this store leave the frame?
///
/// `@FR-L-CapOne`'s second admissible group.  The single owner exists because a record left
/// BEHIND would otherwise release what the escaping one still holds (loft#1440) — but that
/// premise needs the left-behind record to be FREED, and one the return delivers on some path is
/// exempt from the frame's sweep on all of them.  Where every member is delivered, none of them
/// is ever released by this frame, so no cascade can run here and each may own: the run hands
/// one out, its cascade frees the capture in the CALLER, and the others simply die with the
/// frame having released nothing.
///
/// Without this the group keeps a statically chosen owner, and the run that delivers a demoted
/// BORROWER leaves the capture with no cascade at all — it is freed by nobody (loft#1476).
fn group_every_record_leaves(
    data: &Data,
    function: &Function,
    d_nr: u32,
    group: &[(u16, u32, usize)],
) -> bool {
    group
        .iter()
        .all(|(local, _, _)| record_leaves_frame(data, function, d_nr, *local))
}

/// Are the records adopting one store pairwise unable to coexist?
///
/// `@FR-L-CapOne` — the condition under which every member of the group OWNS: on any given run at most one of
/// them is built, so each one's cascade is that run's only release.  Pairwise and not merely
/// "every build is conditional", because conditional is not exclusive — see
/// [`builds_are_mutually_exclusive`].
fn group_is_pairwise_exclusive(body: &Value, group: &[(u16, u32, usize)]) -> bool {
    group.iter().enumerate().all(|(i, (la, _, _))| {
        group[i + 1..]
            .iter()
            .all(|(lb, _, _)| builds_are_mutually_exclusive(body, *la, *lb))
    })
}

/// This function's closure records, grouped by the STORE each adopted.
///
/// One home for `@FR-L-CapOwn`'s "which records are in the running for this store", read by
/// [`mark_borrowed_captures`] (which makes the losers borrow) and by [`owning_record_locals`]
/// (which asks the winner whether it exists at run time).  The two have to agree by
/// construction: a record the marking makes borrow while the free-suppression still credits it
/// leaves the store with no release at all, which is the shape loft#1464 is.
///
/// Returns the groups keyed by [`adopted_store_key`], plus the `(record, attribute)` pairs that
/// adopt NOTHING — those borrow outright and never enter a group.
pub(super) fn capture_store_adopters(
    data: &Data,
    function: &Function,
    builds: &CaptureBuilds,
) -> (
    HashMap<(u16, u32), Vec<(u16, u32, usize)>>,
    Vec<(u32, usize)>,
) {
    let mut adopters: HashMap<(u16, u32), Vec<(u16, u32, usize)>> = HashMap::default();
    let mut never_adopted = Vec::new();
    for v in 0..function.next_var() {
        // The DEFINING frame holds the record in the `___clos_N` local `emit_lambda_code`
        // mints for it.  A frame that receives the record as an ARGUMENT is the closure BODY
        // (its hidden `__closure` parameter), whose own variable table knows nothing about who
        // owns the captures — reading it flipped the verdict depending on definition order.
        // …and a displaced-record SNAPSHOT (`__disp_N`, `displaced_drop`) is a transient copy
        // released where it is taken, never an adopter: named one, it became the witness the
        // frame's conditional release reads, and the frame freed the capture under the live
        // record (loft#1606).
        if function.is_argument(v) || function.name(v).starts_with("__disp_") {
            continue;
        }
        let Type::Reference(record, _) = function.tp(v) else {
            continue;
        };
        let record = *record;
        if !data.def(record).name.starts_with("__closure_") {
            continue;
        }
        for a in 0..data.attributes(record) {
            if !capture_attr_is_cascade_relevant(data, record, a) {
                continue;
            }
            if record_adopts_capture(data, function, builds, record, a) {
                adopters
                    .entry(adopted_store_key(data, function, builds, v, record, a))
                    .or_default()
                    .push((v, record, a));
            } else {
                never_adopted.push((record, a));
            }
        }
    }
    (adopters, never_adopted)
}

/// Which of one store's adopters KEEPS it — `@FR-L-CapOwn`'s "exactly one".
///
/// The record that LEAVES the frame, because the frame is gone by the time the question is
/// asked; where none leaves, the first, which is the single-record case unchanged.
fn adoption_owner_index(
    data: &Data,
    function: &Function,
    d_nr: u32,
    group: &[(u16, u32, usize)],
) -> usize {
    group
        .iter()
        .position(|(v, _, _)| record_leaves_frame(data, function, d_nr, *v))
        .unwrap_or(0)
}

/// The closure-record local whose cascade releases the store capture `v` names, if any.
///
/// `@FR-L-CapOwn` — the frame gives up its own release only because some record's cascade takes
/// it over, and this names that record.  [`capture_adoption_owns_free`] answers *whether* the
/// handover happened; this answers *to whom*, which is the fact a build that may not RUN needs:
/// the record local is null until its build executes, so it is the runtime witness for
/// "the cascade that replaces the frame's free will actually happen" (loft#1464).
///
/// Reads the same grouping the borrow-marking reads, so the record named here is the record
/// whose cascade still follows the capture.  Asking it any other way is how the two drift:
/// naming an adopter the marking made BORROW would decline the frame's free in favour of a
/// cascade that stops at that attribute.
pub(super) fn owning_record_locals(
    data: &Data,
    function: &Function,
    d_nr: u32,
    builds: &CaptureBuilds,
    v: u16,
) -> Vec<u16> {
    // Both spellings of "the record took this local's store", the pair `CaptureBuilds` carries:
    // a STRUCT capture names the local outright, a COLLECTION capture names a view whose
    // backing local is this one.
    let name = function.name(v).to_string();
    let (adopters, _) = capture_store_adopters(data, function, builds);
    for group in adopters.values() {
        // `@FR-L-CapOne` — every member owns where the records CANNOT COEXIST, the same condition
        // `mark_borrowed_captures` uses to leave them all owning.  The two must agree by
        // construction: a record the marking left owning while this side declined to name it
        // would have the frame free the store out from under that run's cascade, which is
        // loft#1473 in the other direction.
        let owners: Vec<usize> = if group_is_pairwise_exclusive(data.def(d_nr).code(), group) {
            (0..group.len()).collect()
        } else {
            vec![adoption_owner_index(data, function, d_nr, group)]
        };
        let mut locals = Vec::new();
        for owner in owners {
            let (local, record, a) = group[owner];
            let attr = data.attr_name(record, a).clone();
            if attr == name || builds.backing.get(&function.var(&attr)) == Some(&v) {
                locals.push(local);
            }
        }
        if !locals.is_empty() {
            return locals;
        }
    }
    Vec::new()
}

/// The STORE a record adopted for capture attribute `a`, as a grouping key.
///
/// `(L-CapOwn)` is about a store, and a capture's NAME is not one: `s = S{…}; k1 = |…| s.a;
/// s = S{…}; k2 = |…| s.a` gives the two records two different stores under one name, and
/// treating them as one made the second borrow what the first never held.  The build walk
/// counts assignments, so `(capture local, generation at the build)` identifies the store; a
/// record whose build this body does not contain — a relayed capture, a rebuilt loop slot —
/// gets a key of its own and is never grouped, which is the pre-loft#1440 behaviour.
fn adopted_store_key(
    data: &Data,
    function: &Function,
    builds: &CaptureBuilds,
    record_local: u16,
    record: u32,
    a: usize,
) -> (u16, u32) {
    // "Groups with nothing" has to be unique per FIELD, not per record: keyed by the record
    // local alone, two captures of ONE record collided, read as one store with two adopters,
    // and the second was demoted to a borrow its cascade then skipped — a leaked store per
    // build.  Reached where a build names no capture the field's name resolves to: a yielded
    // lambda's record names the COPY it took (loft#1676), and a record rebuilt in a loop.
    let unique = (
        u16::MAX,
        (u32::from(record_local) << 16) | (a as u32 & 0xFFFF),
    );
    let capture = function.var(&data.attr_name(record, a).clone());
    if capture == u16::MAX || builds.rebuilt_in_loop.contains(&capture) {
        return unique;
    }
    match builds
        .adopted
        .get(&record_local)
        .and_then(|pairs| pairs.iter().find(|(c, _)| *c == capture))
    {
        Some((_, generation)) => (capture, *generation),
        // No build for it in this body: key it uniquely so it groups with nothing.
        None => unique,
    }
}

/// Does the closure record held by local `v` LEAVE the frame that built it?
///
/// `@FR-L-CapOwn` — the record that outlives the frame is the one that must keep a store they
/// both adopted.  The fn-ref's own spelling of "this value carries that record" is a
/// `DepEntry::CalleeFrame` in the declared return type, which is the only route out TODAY: #318
/// refuses returning a struct that holds a capturing closure, and a `&fn()` parameter does not
/// compile at all (loft#1443, an ICE).  That second one is a gap rather than a decision —
/// `(B-Ref-Intro)` admits `&τ` for every τ with none excluded (binding.md, the paragraph that
/// closed D-bind-17) — so when it is implemented this predicate gains a second source and must
/// be told, or an escaping closure written out through a `&` parameter loses its capture the
/// way loft#1439 lost one.
///
/// The declared type's note is the FALLBACK only: it is published once per lambda and
/// OVERWRITTEN, so wherever a function builds more than one it names the last one BUILT rather
/// than the one the return delivers (loft#1444).  `returned_closure_records` reads the values
/// in RETURN POSITION instead, and the note is asked only where that finds nothing — the
/// `return fn() { … }` written straight out, where the two agree.
pub(super) fn record_leaves_frame(data: &Data, function: &Function, d_nr: u32, v: u16) -> bool {
    if function.is_argument(v) {
        return false;
    }
    let mut delivered = returned_closure_records(data, function, d_nr);
    // The second route out, the one this doc comment reserved a place for.  Both are
    // DELIVERY — the value leaves this frame and the caller holds it — so they union
    // rather than take turns: a function that both returns one closure and writes another
    // out through a link delivers both, and asking only the returns would free the one the
    // link handed over.
    delivered.extend(link_written_closure_records(data, function, d_nr));
    if !delivered.is_empty() {
        return delivered.contains(&v);
    }
    // Nothing in return position names a record — fall back to the declared type's note, which
    // is what a `return fn() { … }` written straight out publishes.
    data.def(d_nr).returned().depend().iter().any(|raw| {
        matches!(crate::data::DepEntry::decode(*raw), crate::data::DepEntry::CalleeFrame(w) if w == v)
    })
}

/// The closure records a function's RETURN can DELIVER, read off the values in return position.
///
/// `@FR-L-CapOwn` needs "which record outlives the frame", and the declared type's
/// `DepEntry::CalleeFrame` note cannot answer it: that note is published once per lambda and
/// OVERWRITTEN, so wherever a function builds more than one it names the last one BUILT rather
/// than the one the return hands out (loft#1444).  The values themselves do know — a fn-ref
/// local carries its record in its own type's deps, and a `return fn() { … }` written straight
/// out is a `FnRef` naming it — so this reads them instead.
///
/// Every arm counts: a branch may deliver either, and each of those records outlives the frame
/// on the path that returns it.
fn returned_closure_records(data: &Data, function: &Function, d_nr: u32) -> Vec<u16> {
    let mut out = Vec::new();
    let mut sources: Vec<u16> = Vec::new();
    let body = data.def(d_nr).code();
    // RETURN POSITION only: the body's tail, and the value of every `return`.  A `FnRef`
    // anywhere else is a closure this frame keeps — collecting those made the record a KEPT
    // lambda builds look delivered, which hands it a capture the escaping one owns.
    let mut delivered: Vec<&Value> = vec![body];
    body.walk(&mut |n| {
        if let Value::Return(inner) = n.unspan() {
            collect_return_sources(inner, data, &mut sources);
            // …and the `FnRef` spelling, which `collect_return_sources` has no arm for.  That
            // decoder answers in VARIABLES, and a capturing lambda is not one: it is a `FnRef`
            // naming its `___clos_N` directly.  So `return fn() { … }` written straight out
            // contributed nothing, and the frame freed the record it had just handed over.
            //
            // The `delivered` stack below does read a `FnRef` — but it only reaches a block's
            // LAST operator, so it sees the tail `return` and never one standing earlier in an
            // `if` arm.  Two decoders for one question, and the blind one is on the route an
            // explicit mid-body `return` takes (loft#1477, found by the loft2 session).
            //
            // Collected from the whole returned subtree: a `return` of an `if` delivers a
            // different record per arm and every one of them outlives the frame on its own
            // path.  A lambda BODY is a separate definition, so a closure the returned lambda
            // builds internally cannot be swept in.
            inner.walk(&mut |m| {
                if let Value::FnRef(_, w, _) = m.unspan()
                    && *w != u16::MAX
                    && !out.contains(w)
                {
                    out.push(*w);
                }
            });
        }
    });
    while let Some(v) = delivered.pop() {
        match v.unspan() {
            Value::FnRef(_, w, _) => {
                if !out.contains(w) {
                    out.push(*w);
                }
            }
            Value::Block(bl) => {
                if let Some(last) = last_non_free_result(&bl.operators, data) {
                    delivered.push(last);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = last_non_free_result(ops, data) {
                    delivered.push(last);
                }
            }
            Value::If(_, t, f) => {
                delivered.push(t);
                delivered.push(f);
            }
            Value::Return(inner) => delivered.push(inner),
            other => collect_return_sources(other, data, &mut sources),
        }
    }
    for v in sources {
        closure_records_of_source(data, function, v, &mut out);
    }
    out
}

/// The closure records a SOURCE VARIABLE stands for, appended to `out`.
///
/// Two spellings reach a record through a variable, and both count: a fn-ref LOCAL, whose
/// own type names the record it holds, and the record itself handed over directly.
///
/// One home because two deliveries ask it — [`returned_closure_records`] for a `return` and
/// [`link_written_closure_records`] for a write through a `&fn(…)` link — and a record the
/// two disagreed about would be freed by the frame on one route and kept on the other. That
/// is the `is_dbref` / `deps_mut` / `is_keyed` family's failure mode, and this list is
/// exactly the shape that drifts when it is written out twice.
pub(super) fn closure_records_of_source(
    data: &Data,
    function: &Function,
    v: u16,
    out: &mut Vec<u16>,
) {
    if v >= function.count() {
        return;
    }
    match function.tp(v) {
        // A fn-ref LOCAL: its own type names the record it holds.
        //
        // Read space-agnostically, which is what this position needs.  A fn-ref VARIABLE
        // holds a FRAME-space list when its value is a lambda built here, and it holds the
        // callee's DEF-space list verbatim when the value came from a call — a returned
        // fn-ref publishes its closure work var as a tagged `CalleeFrame` note, and
        // `call_dependencies` hands a `Type::Function` return back unchanged.  Asking
        // `frame_vars()` therefore trips the space assert on a list that is simply INERT
        // here: every entry of a def-space list names the CALLEE's attributes or the
        // CALLEE's frame, never a record of this function, so nothing it carries can pass
        // the caller's membership test (`85-closure-factory-discarded-free`).
        Type::Function(_, _, deps, ..) => {
            for w in deps.as_slice() {
                if !out.contains(w) {
                    out.push(*w);
                }
            }
        }
        // The record itself, delivered directly.
        Type::Reference(record, _)
            if data.def(*record).name.starts_with("__closure_") && !out.contains(&v) =>
        {
            out.push(v);
        }
        _ => {}
    }
}

/// The closure records a function writes OUT through a `&fn(…)` LINK.
///
/// [`record_leaves_frame`]'s second source, and its own doc comment predicted it: *"a
/// `&fn()` parameter does not compile at all (loft#1443, an ICE) … when it is implemented
/// this predicate gains a second source and must be told, or an escaping closure written
/// out through a `&` parameter loses its capture the way loft#1439 lost one."*  loft#1443
/// made the write compile and did not tell it, so the record the write handed to the caller
/// was still freed at the callee's scope exit — `OpFreeRef(___clos_N)` sits directly after
/// the write in the IR — and the caller then called a closure over its own poison
/// (`0xDEADBEEF`).  Silent without `LOFT_POISON=1`, because a freed arena slot still reads
/// back the bytes it held: every cell of the loft#1443 guard passed on stale data.
///
/// A write through a link IS a delivery, exactly as a `return` is: `(B-Ref-Uniform)` says a
/// `&τ` variable is used exactly like a τ variable, and the caller holds what the callee
/// wrote. So the reading is the one [`returned_closure_records`] already does — the records
/// the assigned value can YIELD, on every arm it may take — asked at each `Set` whose
/// destination is a `RefVar` over a function type.
///
/// Narrow on purpose. Only a `RefVar` destination counts: a plain `fn`-typed LOCAL
/// (`g = fn() { … }`) is a closure this frame keeps, and collecting those would hand a
/// kept lambda the capture an escaping one owns — the same trap `returned_closure_records`
/// documents for a `FnRef` outside return position.
pub(super) fn link_written_closure_records(
    data: &Data,
    function: &Function,
    d_nr: u32,
) -> Vec<u16> {
    let is_fn_link = |v: u16| matches!(function.tp(v), Type::RefVar(inner) if matches!(**inner, Type::Function(..)));
    // Almost no function has a `&fn(…)` at all, and this walks a whole body — so ask the
    // variable table first, which is a scan of the one thing already in hand.  The sweep that
    // calls this runs once per scope exit.
    if !(0..function.count()).any(is_fn_link) {
        return Vec::new();
    }
    let mut out: Vec<u16> = Vec::new();
    let mut superseded: Vec<u16> = Vec::new();
    let body = data.def(d_nr).code();
    body.walk(&mut |n| {
        if let Value::Set(v, value) = n.unspan()
            && is_fn_link(*v)
        {
            records_of_link_write(data, function, value, &mut out);
        }
        // A write DISPLACED by a later write to the same link, in the same straight line of
        // operators, never reaches the caller: the second write overwrites the slot before
        // the frame returns, so that record is this frame's to free after all. Only a
        // sibling supersedes — an `if`'s two arms are separate lists and BOTH deliver, on
        // the path that runs, which is the same reading `returned_closure_records` gives a
        // branching return.
        let ops: &[Value] = match n.unspan() {
            Value::Block(bl) => &bl.operators,
            Value::Insert(ops) => ops,
            _ => return,
        };
        let writes: Vec<(u16, &Value)> = ops
            .iter()
            .filter_map(|o| match o.unspan() {
                Value::Set(v, value) if is_fn_link(*v) => Some((*v, &**value)),
                _ => None,
            })
            .collect();
        for (idx, (dest, value)) in writes.iter().enumerate() {
            if writes[idx + 1..].iter().any(|(later, _)| later == dest) {
                records_of_link_write(data, function, value, &mut superseded);
            }
        }
    });
    out.retain(|r| !superseded.contains(r));
    out
}

/// The closure records ONE write through a `&fn(…)` link hands to the caller.
///
/// The value's own yield, on every arm it may take — a `FnRef` built in place, an `if` that
/// chooses between two, a block whose tail is one — plus the second stage for the records a
/// value only NAMES: `out = h`, where the record lives in the fn-ref local's type.
fn records_of_link_write(data: &Data, function: &Function, value: &Value, out: &mut Vec<u16>) {
    let mut sources: Vec<u16> = Vec::new();
    let mut delivered: Vec<&Value> = vec![value];
    while let Some(cur) = delivered.pop() {
        match cur.unspan() {
            Value::FnRef(_, w, _) => {
                if !out.contains(w) {
                    out.push(*w);
                }
            }
            Value::Block(bl) => {
                if let Some(last) = last_non_free_result(&bl.operators, data) {
                    delivered.push(last);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = last_non_free_result(ops, data) {
                    delivered.push(last);
                }
            }
            Value::If(_, t, f) => {
                delivered.push(t);
                delivered.push(f);
            }
            other => collect_return_sources(other, data, &mut sources),
        }
    }
    for v in sources {
        closure_records_of_source(data, function, v, out);
    }
}

/// Is capture attribute `a` of `record` one the record's death CASCADES through?
///
/// Only the two share markers are: the attribute holds a 12-byte DbRef for `free_named` to
/// follow. An inline-bytes capture — a `text` copy, empty deps — holds no reference, so there
/// is nothing to reclaim and nothing to suppress.
///
/// One home because two callers must agree exactly. `mark_borrowed_captures` uses it to decide
/// which captures get a verdict at all, and `capture_is_adopted` to decide whether a frame-exit
/// free may be suppressed; a capture the first skips must not be one the second adopts, or the
/// store is freed twice. Restating it in the second place is how loft#1308 was written the
/// first time.
pub(super) fn capture_attr_is_cascade_relevant(data: &Data, record: u32, a: usize) -> bool {
    matches!(data.attr_type(record, a).base(), Type::Reference(_, deps) if !deps.is_empty())
}

/// Does the closure record own the store behind capture `a` of `record`, as seen
/// from the defining frame `function`?  See [`mark_borrowed_captures`].
pub(super) fn record_adopts_capture(
    data: &Data,
    function: &Function,
    builds: &CaptureBuilds,
    record: u32,
    a: usize,
) -> bool {
    // A `__cell_<T>` is minted FOR this closure (plan-22 boxes a mutated scalar /
    // text capture into one), so the record is its only possible owner however the
    // original binding was reached — including from a parameter.
    //
    // ...as long as it was minted for a binding THIS frame has.  A lambda nested in a lambda
    // RELAYS the capture outward: the enclosing lambda holds no binding of the name at all and
    // reads the handle out of its own closure record, so the record it builds is a second
    // pointer at a cell that belongs further out.  Adopting it freed the cell when the
    // enclosing lambda returned — on its FIRST call, so the second one read a released store
    // (loft#1236).
    if let Type::Reference(cell, _) = data.attr_type(record, a)
        && data.def(cell).name.starts_with("__cell_")
        && function.var(&data.attr_name(record, a)) != u16::MAX
    {
        return true;
    }
    // loft#1610 — a record confined to one loop pass outlives nothing it captured, so the frame
    // keeps every release it HAS (`@FR-L-CapOwn`'s "whichever outlives the other").  Asked after
    // the `__cell_` arm above: a cell is minted for the closure, the frame has no release of it
    // to keep, and the record is its only owner wherever it lives.
    if builds.pass_confined.contains(&record) {
        return false;
    }
    let v = function.var(&data.attr_name(record, a));
    // An unresolvable name defaults to BORROW: an unfreed store is a leak the
    // store checker reports, while an extra free silently corrupts a caller.
    // A parameter never enters the scope-exit sweep at all (`variables()`:
    // "never return function arguments"), so it has no free to hand over.
    if v == u16::MAX || function.is_argument(v) {
        return false;
    }
    // The same test as `get_free_vars`' `owns`, so the two cannot drift: empty
    // deps means owned, and a keyed collection's self-dep is an ownership marker
    // rather than a borrow (@P302) — asked of the local the capture NAMES, and
    // then of whatever backs it.
    frame_owns_capture_store(function, v)
}

/// Does the defining frame own the store behind local `v` — directly, or through the
/// backing local a collection VIEW depends on?  See [`mark_borrowed_captures`].
///
/// A struct local owns its store outright (`s: ref(726) OWNS`, empty deps) and the plain
/// `dep.is_empty()` test saw that.  A vector local does not: `v = [7,2,3]` compiles to a
/// VIEW whose deps name a separate `__vdb_N` local — `v: vec<int> deps=[__vdb_1(2)]`
/// beside `__vdb_1: ref(467) OWNS` — and the frame frees the BACKING local at scope exit.
/// Reading only `v`'s own deps therefore answered "borrow" for a store this frame really
/// does own and really does free, so the record declined the handover and the escaped
/// closure read a released store (loft#1308).
///
/// Ownership is what is being followed, not merely a dep edge: the walk stops at an
/// ARGUMENT, whose store belongs to the caller and outlives this frame, so a capture that
/// projects into a parameter stays the BORROW that #682 made it. The bound is a guard
/// against a cyclic dep chain, not a depth the language imposes.
fn frame_owns_capture_store(function: &Function, start: u16) -> bool {
    // @FR-O-Proxy asks free — the answer decides whether the frame's scope-exit free stands
    // or the capture record's cascade takes it over, so a free follows either way and this is
    // the free question asked about the BACKING local rather than the binding.
    let mut v = start;
    for _ in 0..8 {
        // loft#1721 — a `??` temp may deliver its default arm's store instead: owned only
        // when that store is the frame's too.
        if function
            .join_owners(v)
            .iter()
            .any(|&o| o == v || function.is_argument(o) || !frame_owns_capture_store(function, o))
        {
            return false;
        }
        let tp = function.tp(v);
        let dep = tp.depend();
        if dep.is_empty() {
            return true;
        }
        // A keyed collection's self-dep is an ownership marker, not a borrow (@P302).
        if dep.len() == 1 && dep[0] == v && crate::parser::vectors::is_keyed(tp) {
            return true;
        }
        // loft#1721 — more than one dep is a JOIN: the value is whichever arm ran (`f() ??
        // []`, a value branch), so the frame owns its store when it owns EVERY arm's.  Which
        // one the capture ends up holding is a per-run fact, settled at the release
        // (`join_capture_witness`).
        if dep.len() > 1 {
            return dep.iter().all(|&d| {
                d != v && !function.is_argument(d) && frame_owns_capture_store(function, d)
            });
        }
        // A self-dep that is not the keyed marker names no backing store to follow.
        if dep[0] == v {
            return false;
        }
        // The caller owns a parameter's store and outlives this frame: no free to hand over.
        if function.is_argument(dep[0]) {
            return false;
        }
        v = dep[0];
    }
    false
}
