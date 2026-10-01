// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

//! What a body's closure BUILDS capture: the backing local each capture names at the build,
//! whether an adoption takes over a local's free, and the joins and loops a build sits in.

use super::backings::construction_work_refs;
use super::call;
use super::capture_adoption::{
    capture_attr_is_cascade_relevant, record_adopts_capture, record_leaves_frame,
};
use super::closure_keep::pass_confined_records;
use crate::data::{Data, Type, Value, v_if};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
use crate::variables::Function;

/// The `__ref_N` work-ref an inline record LITERAL built, when the literal's value is that
/// buffer itself — the shape `c: S? = S { x: 5 }` lowers to.
///
/// `None` for every other right-hand side, which is what keeps this narrow: a block whose
/// value is a work-ref it did not `OpDatabase` into is someone else's store, and a dense
/// local has no buffer at all.
fn inline_literal_work_ref(rhs: &Value, function: &Function, data: &Data) -> Option<u16> {
    let Value::Block(bl) = rhs.unspan() else {
        return None;
    };
    let last = bl
        .operators
        .iter()
        .rev()
        .find(|o| !matches!(o.unspan(), Value::Line(_)))?;
    let Value::Var(av) = last.unspan() else {
        return None;
    };
    let av = *av;
    if av >= function.count() {
        return None;
    }
    let name = function.name(av);
    if !name.starts_with("__ref_") && !name.starts_with("__rref_") {
        return None;
    }
    // The store has to be MINTED here, not merely named: `OpDatabase(av, …)` is the mint.
    let db = data.def_nr("OpDatabase");
    let built_here = bl.operators.iter().any(|o| {
        matches!(o.unspan(), Value::Call(d, args) if *d == db
            && matches!(args.first().map(Value::unspan), Some(Value::Var(t)) if *t == av))
    });
    built_here.then_some(av)
}

/// Every construction work-ref a right-hand side DELIVERS to the binding it is assigned to:
/// [`inline_literal_work_ref`] at the value's tail, reached through each `if` arm (a `match`
/// lowers to `if`s) and each block's last operator.  `if c { S { … } } else { S { … } }`
/// delivers one of two, and the binding adopts whichever arm ran.
pub(super) fn adopted_work_refs(rhs: &Value, function: &Function, data: &Data, out: &mut Vec<u16>) {
    let tail = |ops: &[Value]| {
        ops.iter()
            .rev()
            .find(|o| !matches!(o.unspan(), Value::Line(_)))
            .cloned()
    };
    match rhs.unspan() {
        Value::If(_, t, f) => {
            adopted_work_refs(t, function, data, out);
            adopted_work_refs(f, function, data, out);
        }
        Value::Block(bl) => {
            if let Some(w) = inline_literal_work_ref(rhs, function, data) {
                out.push(w);
            } else if let Some(last) = tail(&bl.operators) {
                adopted_work_refs(&last, function, data, out);
            }
        }
        Value::Insert(ops) => {
            if let Some(last) = tail(ops) {
                adopted_work_refs(&last, function, data, out);
            }
        }
        _ => {}
    }
}

/// Does a closure record's adoption take over the frame-exit free of local `v`?
///
/// One home for the question `get_free_vars` asks before emitting a free and
/// [`check_ref_leaks`] asks before calling an unfreed local a leak. They are the emitter and
/// its static mirror, so they have to answer identically: a suppression the mirror does not
/// know about reads as a leak, and a leak the mirror excuses reads as nothing at all.
///
/// Both spellings of "reaches the store" are needed. A struct capture names the local that
/// holds the store outright; a collection capture names a VIEW, and the store lives in the
/// backing local no closure captured by name — which is what
/// [`backs_an_adopted_capture`] asks about. `is_dbref(.base())` is the shape test both
/// halves share: a capture's store may be a `Vector` or a keyed collection, not only a
/// `Reference`.
///
/// ⚠ Restating this rule is how it drifts. loft#1308 was the free emitter and this mirror
/// disagreeing on the shape test; the mirror then kept the `is_captured` half alone and a
/// capturing closure over a local vector false-positived the leak assert. Three consumers
/// now share it — those two and `ownership_cfg`'s leak oracle.
pub(crate) fn capture_adoption_owns_free(
    data: &Data,
    function: &Function,
    built_with: &CaptureBuilds,
    v: u16,
) -> bool {
    // @FR-O-Latest — the record adopts the store the capture named AT THE BUILD, so the
    // handover is only sound while the local still names that store.  A local assigned again
    // after the build names a different one, which nothing else frees: the record's cascade
    // reaches the adopted store and the frame's free was suppressed on the strength of the
    // BINDING being captured.  loft#1324 closed this for the collection half, where the
    // backing local is found through `built_with`; the direct half asked `is_captured` and
    // kept suppressing whatever the local named LAST, so `s = S{…}; h = |i| { s.a + i };
    // s = build(h)` leaked the store `s` ends up holding, on both backends, once per
    // reassignment and once per pass of a loop (loft#1388).
    !built_with.reassigned_after_build.contains(&v)
        && ((function.is_captured(v) && !captured_only_by_confined(data, function, built_with, v))
            || backs_an_adopted_capture(data, function, built_with, v))
        && crate::data::is_dbref(function.tp(v).base())
}

/// loft#1610 — is every closure record that captures local `v` confined to one loop pass
/// ([`pass_confined_records`])?  Then no record adopts `v` and the frame's release stands.  A
/// local captured by no record this frame can see keeps the old answer (`false`).
fn captured_only_by_confined(
    data: &Data,
    function: &Function,
    built_with: &CaptureBuilds,
    v: u16,
) -> bool {
    if built_with.pass_confined.is_empty() {
        return false;
    }
    // A boxed `__cell_` a closure mutates is minted FOR the record, which adopts it confined
    // or not (it is the cell's only owner): the frame's release would be a second one, and
    // the next pass's displaced record released the store again after the frame had handed
    // its number to the new cell (LOFT_POISON: the pass read a poisoned counter).
    if matches!(function.tp(v).base(), Type::Reference(r, _) if data.def(*r).name.starts_with("__cell_"))
    {
        return false;
    }
    let name = function.name(v);
    let mut any = false;
    for w in 0..function.next_var() {
        if function.is_argument(w) {
            continue;
        }
        let Type::Reference(record, _) = function.tp(w).base() else {
            continue;
        };
        if !data.def(*record).name.starts_with("__closure_")
            || data.attr(*record, name) == usize::MAX
        {
            continue;
        }
        if !built_with.pass_confined.contains(record) {
            return false;
        }
        any = true;
    }
    any
}

/// Does a closure record that LEAVES this frame hold the store of local `witness`?
///
/// `@FR-L-CapOwn` — a captured heap store is freed once, by whichever of the record and the
/// frame outlives the other.
///
/// [`capture_adoption_owns_free`] says the record ADOPTED the capture, and that alone does not
/// decide who frees: a record built and left behind dies with the frame WITHOUT a free of its
/// own — the fn-ref type carries `Deps::frame1` precisely so the scope sweep skips it — so its
/// cascade never runs and the frame's release is the store's only one.  A record handed OUT is
/// the caller's, and its cascade frees what it adopted, so a frame release there is a second
/// free of one store.
///
/// The escaping records are the ones the declared return type names, `DepEntry::CalleeFrame`
/// being the fn-ref's own spelling of "this value carries that record".  Asking whether a
/// record holds THIS witness — its capture attributes are named after the locals they took —
/// is what keeps a function that returns one closure while keeping another from declining a
/// free the kept one still owes.
pub(super) fn escaping_record_holds(
    data: &Data,
    function: &Function,
    d_nr: u32,
    witness: u16,
) -> bool {
    if d_nr == u32::MAX || !function.is_captured(witness) {
        return false;
    }
    let name = function.name(witness);
    data.def(d_nr).returned().depend().iter().any(|raw| {
        let crate::data::DepEntry::CalleeFrame(w) = crate::data::DepEntry::decode(*raw) else {
            return false;
        };
        if w >= function.count() {
            return false;
        }
        let Type::Reference(record, _) = function.tp(w) else {
            return false;
        };
        let record = *record;
        if !data.def(record).name.starts_with("__closure_") {
            return false;
        }
        let a = data.attr(record, name);
        // …and the record's death has to REACH the store: the cascade follows an attribute
        // holding a 12-byte DbRef, which is what `capture_attr_is_cascade_relevant` asks.  A
        // capture it does not follow — a record `Enum`, whose attribute is not a `Reference` —
        // is adopted for the free-suppression's purposes and freed by nobody, so the frame's
        // release is still the only one.
        a != usize::MAX && capture_attr_is_cascade_relevant(data, record, a)
    })
}

/// Does a closure record that LEAVES this frame hold the store literal buffer `buffer` minted?
///
/// `@FR-L-CapOwn` — a captured heap store is freed once, by whichever of the record and the
/// frame outlives the other.  Where the answer is the record, the frame owes nothing for that
/// store and must emit no free for the buffer naming it.
///
/// Asked of the BUFFER, not of the capture's name, and that is the whole of it: a buffer names
/// one store for its entire life, while a capture local reassigned after the build names two.
/// A name-keyed question then answers about whichever store the local holds LAST, which is the
/// one nobody adopted — so the frame declines the free it owes and takes the one it does not
/// (loft#1446).  `@FR-O-Witness` is this same currency, store identity, for a mixed-ownership
/// local.
///
/// Both halves loft#1439 named are kept, per record.  The record must ADOPT the capture — one
/// that merely borrows leaves the frame's free as the store's only release, and suppressing it
/// leaks — and its cascade must REACH the store, which is what
/// [`capture_attr_is_cascade_relevant`] asks; a capture the cascade does not follow is freed by
/// nobody else.
pub(super) fn escaping_record_holds_buffer(
    data: &Data,
    function: &Function,
    d_nr: u32,
    built_with: &CaptureBuilds,
    buffer: u16,
) -> bool {
    if d_nr == u32::MAX {
        return false;
    }
    let Some(pairs) = built_with.buffer_adopted.get(&buffer) else {
        return false;
    };
    pairs.iter().any(|&(record_var, capture)| {
        if !record_leaves_frame(data, function, d_nr, record_var) {
            return false;
        }
        let Type::Reference(record, _) = function.tp(record_var) else {
            return false;
        };
        let record = *record;
        if !data.def(record).name.starts_with("__closure_") {
            return false;
        }
        let a = data.attr(record, function.name(capture));
        a != usize::MAX
            && record_adopts_capture(data, function, built_with, record, a)
            && capture_attr_is_cascade_relevant(data, record, a)
    })
}

/// The backing local a capture named AT THE CLOSURE BUILD — the store the record actually
/// holds — or `None` when the code does not settle it.
///
/// A capture's store is decided by @FR-O-Latest: ownership belongs to the LATEST assignment
/// *at that point*, and a type-level `deps` list cannot express a point.  For a local assigned
/// once the two coincide and the type is enough; for one REASSIGNED after the build they name
/// different stores, and reading the type then aims the frame-exit suppression at the store the
/// closure does NOT hold.  Both directions are wrong at once: the store the record adopted is
/// freed by the frame as well (an escaping closure reads a released store), and the store the
/// local now names is suppressed although nobody adopted it (it leaks) — loft#1324.
///
/// The build is `OpSetDbRef(___clos_N, <offset>, <capture>)`, so walking the body in order and
/// remembering each local's most recent backing root answers it directly.  [`Value::walk`] is
/// pre-order over the children in source order, which is the ordering this needs; a hand-rolled
/// descent here would be the fourth copy of one that has drifted before.
///
/// A capture that owns its store outright — a struct — has no backing root and is not in the
/// map, and neither is one whose build this body does not contain.  Both fall back to the type
/// dep, which is the only fact available for them and is right whenever the local is assigned
/// once.
pub(crate) fn capture_build_backings(
    data: &Data,
    function: &Function,
    code: &Value,
) -> CaptureBuilds {
    let set_dbref = data.def_nr("OpSetDbRef");
    let mut latest: HashMap<u16, u16> = HashMap::default();
    // The literal buffers a local's LATEST assignment minted — the store a build reached
    // through that local therefore adopts.  A value branch mints one per arm and the local
    // adopts whichever ran, so all of them are carried.
    let mut minted: HashMap<u16, Vec<u16>> = HashMap::default();
    // The join arms of each local's LATEST assignment ([`join_arm_stores`]), for a build to
    // record against the capture it reaches.
    let mut join_arms: HashMap<u16, Vec<u16>> = HashMap::default();
    // How many times each local has been ASSIGNED so far in this walk.  Two records hold the
    // SAME store only if they adopted a local at the same generation: a local assigned between
    // two builds gives them different stores (@FR-O-Latest), which the capture NAME cannot say
    // and which decides whether one of them may be made to borrow (@FR-L-CapOwn, loft#1440).
    let mut generation: HashMap<u16, u32> = HashMap::default();
    let mut out = CaptureBuilds::default();
    captures_built_in_a_loop(code, set_dbref, false, &mut out.rebuilt_in_loop);
    out.pass_confined = pass_confined_records(data, function, code);
    captures_built_conditionally(code, set_dbref, false, &mut out.built_conditionally);
    let mut built: HashSet<u16> = HashSet::default();
    // Capture vars already resolved at their enclosing statement, waiting for the walk to
    // reach the build node itself.  See the `Value::Set` arm.
    let mut resolved_in_rhs: HashSet<u16> = HashSet::default();
    code.walk(&mut |node: &Value| match node.unspan() {
        Value::Set(v, rhs) => {
            // A build inside this statement's OWN right-hand side captures the value the
            // local held BEFORE the assignment — `s = build(|i| { s.a + i })` hands the
            // closure the store `s` is about to stop naming.  `Value::walk` is pre-order, so
            // the build node is reached AFTER this one, by which time `latest` describes the
            // assignment rather than the capture.  Resolve those builds here, against
            // `latest` as it still stands, and let the walk skip them when it arrives.
            for (record, c, backing) in captures_built_in(data, function, rhs, set_dbref, &latest) {
                built.insert(c);
                if let Some(b) = backing {
                    out.backing.insert(c, b);
                    built.insert(b);
                }
                out.adopted
                    .entry(record)
                    .or_default()
                    .push((c, *generation.get(&c).unwrap_or(&0)));
                for &b in minted.get(&c).into_iter().flatten() {
                    out.buffer_adopted.entry(b).or_default().push((record, c));
                }
                if let Some(arms) = join_arms.get(&c) {
                    out.join_arms_at_build.insert(c, arms.clone());
                }
                resolved_in_rhs.insert(c);
                // @FR-O-Latest — this very assignment is the one that moves the local off
                // the store the record just adopted.
                if c == *v || backing == Some(*v) {
                    out.reassigned_after_build.insert(*v);
                }
            }
            // @FR-O-Latest, the OTHER half of loft#1324.  The record holds the store the
            // capture named at the BUILD; a local assigned again after that names a different
            // one, and the frame is the only thing left to free it.
            if built.contains(v) {
                out.reassigned_after_build.insert(*v);
            }
            match crate::use_analysis::view_root_slots(data, rhs).as_deref() {
                Some([root]) if bind_views_root(function, *v, rhs, *root) => {
                    latest.insert(*v, *root);
                }
                // A right-hand side that names no single root leaves no backing to remember,
                // and the stale one would be worse than none: drop it.  So does one whose
                // destination owns the store it ends up holding — see `bind_views_root`.
                _ => {
                    latest.remove(v);
                }
            }
            // …and the local now names a different store, so a build after this one adopts
            // something the builds before it never held.
            *generation.entry(*v).or_default() += 1;
            // The buffer this assignment minted is the store any LATER build adopts through
            // `v`.  An assignment that mints none leaves the local naming something this walk
            // cannot pin to a buffer, and a stale entry would name the wrong store outright.
            // Only a CAPTURED local can reach a build, and asking every assignment of every
            // function allocated once per branch right-hand side in the stdlib alone.
            let arms = if function.is_captured(*v) {
                join_arm_stores(rhs, function, data)
            } else {
                Vec::new()
            };
            if arms.is_empty() {
                join_arms.remove(v);
            } else {
                join_arms.insert(*v, arms);
            }
            let mut bufs = Vec::new();
            adopted_work_refs(rhs, function, data, &mut bufs);
            if bufs.is_empty() {
                minted.remove(v);
            } else {
                minted.insert(*v, bufs);
            }
        }
        Value::Call(d, args) if *d == set_dbref => {
            if let Some(Value::Var(c)) = args.get(2).map(Value::unspan) {
                if resolved_in_rhs.remove(c) {
                    return;
                }
                built.insert(*c);
                if let Some(&backing) = latest.get(c) {
                    out.backing.insert(*c, backing);
                    built.insert(backing);
                }
                if let Some(Value::Var(record)) = args.first().map(Value::unspan) {
                    out.adopted
                        .entry(*record)
                        .or_default()
                        .push((*c, *generation.get(c).unwrap_or(&0)));
                    for &b in minted.get(c).into_iter().flatten() {
                        out.buffer_adopted.entry(b).or_default().push((*record, *c));
                    }
                    if let Some(arms) = join_arms.get(c) {
                        out.join_arms_at_build.insert(*c, arms.clone());
                    }
                }
            }
        }
        _ => {}
    });
    out
}

/// The captures a right-hand side BUILDS, each with the backing local it names.
///
/// Two spellings, the pair [`capture_build_backings`] itself carries: a struct capture names
/// the local outright (no backing), a collection capture names a VIEW whose root is the local.
/// A view minted inside this same right-hand side is resolved from the statements walked here;
/// anything older comes from `outer`, which describes the program up to — and not including —
/// the assignment this right-hand side belongs to.
fn captures_built_in(
    data: &Data,
    function: &Function,
    rhs: &Value,
    set_dbref: u32,
    outer: &HashMap<u16, u16>,
) -> Vec<(u16, u16, Option<u16>)> {
    let mut latest = outer.clone();
    let mut found: Vec<(u16, u16, Option<u16>)> = Vec::new();
    rhs.walk(&mut |node: &Value| match node.unspan() {
        Value::Set(c, src) => match crate::use_analysis::view_root_slots(data, src).as_deref() {
            Some([root]) if bind_views_root(function, *c, src, *root) => {
                latest.insert(*c, *root);
            }
            _ => {
                latest.remove(c);
            }
        },
        Value::Call(d, args) if *d == set_dbref => {
            // The RECORD is args[0] and the capture args[2].  Both are needed: which local a
            // record adopted decides ownership, and one local may be adopted by several
            // records (@FR-L-CapOwn, loft#1440).
            if let (Some(Value::Var(record)), Some(Value::Var(c))) = (
                args.first().map(Value::unspan),
                args.get(2).map(Value::unspan),
            ) {
                found.push((*record, *c, latest.get(c).copied()));
            }
        }
        _ => {}
    });
    found
}

/// What a body's closure BUILDS say about the locals they capture.
///
/// Two facts, one walk, because both are read off the same `OpSetDbRef(___clos_N, …, capture)`
/// point and a second walk would be a copy of an ordering that has drifted before.
#[derive(Default, Debug, Clone)]
pub(crate) struct CaptureBuilds {
    /// The backing local a capture named AT THE BUILD — the store the record actually holds.
    /// A capture that owns its store outright (a struct) has no backing root and is absent.
    pub(crate) backing: HashMap<u16, u16>,
    /// Locals whose store the record adopted and which were ASSIGNED AGAIN afterwards, so the
    /// local no longer names the adopted store and the frame still owes its own free.
    pub(crate) reassigned_after_build: HashSet<u16>,
    /// Captures whose closure BUILD sits inside a loop, so the record's slot is rewritten on
    /// every pass and only the LAST adoption is the one it still holds.
    pub(crate) rebuilt_in_loop: HashSet<u16>,
    /// loft#1610, `@FR-L-CapOwn` — the closure record TYPES confined to one pass of a loop: built
    /// inside a loop body into a fn-ref local that appears nowhere outside that loop and there
    /// only as a call's callee, so no value of it survives the pass.  Such a record cannot
    /// outlive what it captured, so it BORROWS every capture and the frame keeps its release,
    /// at the local's own scope end ([`pass_confined_records`]).
    pub(crate) pass_confined: HashSet<u32>,
    /// Captures whose closure BUILD does not DOMINATE the scope exit — it sits inside an `if`
    /// arm, a `match` arm or a loop body, so on some runs no record is ever built.
    ///
    /// `(L-CapOwn)` hands the release to the record's cascade, and the cascade only happens if
    /// the build EXECUTES.  The suppression is a static fact about the function and the thing
    /// it trades away is a per-RUN one, so where the two can disagree the frame's free is kept
    /// and made conditional on the record existing (loft#1464).
    pub(crate) built_conditionally: HashSet<u16>,
    /// Per closure-record local: the `(capture local, generation)` pairs it adopted at ITS
    /// build, where the generation counts assignments to that local before the build.
    ///
    /// This is the STORE identity `(L-CapOwn)`'s "freed once" needs.  Two records may name one
    /// local and hold two different stores — `s = S{…}; k1 = |…| s.a; s = S{…}; k2 = |…| s.a`
    /// — so the capture NAME cannot decide which of them owns, and grouping by it made one
    /// borrow a store the other never held (loft#1440's first cut, measured as a leaked `S`).
    pub(crate) adopted: HashMap<u16, Vec<(u16, u32)>>,
    /// Per literal BUFFER: the `(closure record, capture local)` pairs that adopted the store
    /// THAT buffer minted.
    ///
    /// `adopted` above answers "which store" with a generation, which is the right currency
    /// between two builds; the frame's scope-exit free needs the store itself, because the
    /// thing it is about to release is a buffer and a buffer names exactly one store for its
    /// whole life.  A capture local does not: reassign it and the name covers two stores, so
    /// asking about the NAME answers about whichever the local happens to hold last
    /// (loft#1446).  `@FR-O-Witness` is the same currency for a mixed-ownership local.
    pub(crate) buffer_adopted: HashMap<u16, Vec<(u16, u16)>>,
    /// Per capture: the stores its value could be AT THE BUILD when that value was a JOIN
    /// ([`join_arm_stores`]).  The local's own deps cannot say it once the local is assigned
    /// again — a type's dep list is the whole body's, and the reassignment's store is on it —
    /// so the arms are read off the assignment that reached the build.  loft#1725.
    pub(crate) join_arms_at_build: HashMap<u16, Vec<u16>>,
}

/// The capture whose value may be `v`'s store among OTHERS — a join — when its record adopts
/// it: `v`'s scope-exit free is then released by store identity against that capture.
///
/// loft#1721.  A capture bound from a join (`c = f() ?? []`, a value branch) holds whichever
/// arm's store ran, so neither "the record adopted `v`" nor "it did not" is true of every run.
/// Freed plainly, the arm that ran was released while an escaping closure still read it
/// (`[3,3]` for `[1,2,3]`); spared, the arm that did not run leaked.  `OpFreeRefIfDistinct(v,
/// capture)` answers per run: the store the capture holds is the record's to release, every
/// other arm's is the frame's.
///
/// Only for a capture assigned ONCE before its build and not rebuilt in a loop.  Identity
/// against the LOCAL is loft#1446's hazard otherwise: a local reassigned after the build reads
/// "distinct" for the very store the record holds.
pub(super) fn join_capture_witness(
    data: &Data,
    function: &Function,
    built_with: &CaptureBuilds,
    v: u16,
) -> Option<u16> {
    (0..function.next_var()).find(|&c| {
        c != v
            && function.is_captured(c)
            && !built_with.rebuilt_in_loop.contains(&c)
            && !built_with.reassigned_after_build.contains(&c)
            && capture_join_candidates(function, c).contains(&v)
            && capture_is_adopted(data, function, built_with, c)
    })
}

/// [`join_capture_witness`] for the capture it excludes: one REASSIGNED after its build.  The
/// local no longer names the store the record adopted, so the witness is the record's own
/// capture SLOT — `(record local, slot position)` — which holds exactly that store for as long
/// as the record lives.  loft#1725.
///
/// Only where ONE closure record in the frame captures `c` (two records may hold two
/// different stores of one name, loft#1440), the record adopts it, and the record LEAVES the
/// frame — one that stays is released by the frame, possibly before this read; the caller
/// guards the build having run at all.
pub(super) fn reassigned_join_capture_slot(
    data: &Data,
    database: &crate::database::Stores,
    function: &Function,
    d_nr: u32,
    built_with: &CaptureBuilds,
    v: u16,
) -> Option<(u16, u16)> {
    let c = (0..function.next_var()).find(|&c| {
        c != v
            && function.is_captured(c)
            && built_with.reassigned_after_build.contains(&c)
            && !built_with.rebuilt_in_loop.contains(&c)
            && built_with
                .join_arms_at_build
                .get(&c)
                .is_some_and(|arms| arms.contains(&v))
            && capture_is_adopted(data, function, built_with, c)
    })?;
    let name = function.name(c);
    let mut slot = None;
    for w in 0..function.next_var() {
        if function.is_argument(w) {
            continue;
        }
        let Type::Reference(record, _) = function.tp(w).base() else {
            continue;
        };
        if !data.def(*record).name.starts_with("__closure_") {
            continue;
        }
        if (0..data.attributes(*record)).any(|a| data.attr_name(*record, a) == name) {
            // A record that stays in the frame is released by it, possibly before this read.
            if slot.is_some() || !record_leaves_frame(data, function, d_nr, w) {
                return None;
            }
            let pos = database.position(data.def(*record).known_type(), name);
            slot = Some((w, pos));
        }
    }
    slot
}

/// The stores a JOIN assigned by `rhs` may leave in its destination, or empty when `rhs` is no
/// join: a `??` temp's value chain with its default arm's owners (`Function::join_owners`,
/// loft#1721), else a value branch's per-arm constructions ([`construction_work_refs`]).
/// Read off the assignment, so it stays true of the build it reaches however often the
/// destination is assigned afterwards.  loft#1725.
fn join_arm_stores(rhs: &Value, function: &Function, data: &Data) -> Vec<u16> {
    let mut out: Vec<u16> = Vec::new();
    rhs.walk(&mut |node: &Value| {
        if let Value::Set(t, _) = node.unspan()
            && !function.join_owners(*t).is_empty()
        {
            let mut work: Vec<u16> = function.join_owners(*t).to_vec();
            work.extend(function.tp(*t).depend());
            while let Some(a) = work.pop() {
                if a == *t || a == u16::MAX || function.is_argument(a) || out.contains(&a) {
                    continue;
                }
                out.push(a);
                work.extend_from_slice(function.join_owners(a));
                work.extend(function.tp(a).depend());
            }
        }
    });
    // A branch arm beside a `??` is an arm too (`if … { f() ?? [] } else { [7] }`): the two
    // spellings of a join union, and one arm on its own is no join at all.
    let mut arms = construction_work_refs(rhs, function, data);
    branch_arm_stores(rhs, &mut arms);
    let branch = arms.len() > 1;
    for a in arms {
        if (branch || !out.is_empty()) && !out.contains(&a) {
            out.push(a);
        }
    }
    out
}

/// The stores the arms of a value branch name — each arm block's own result deps, through
/// nested branches — for the arms [`construction_work_refs`] does not list (a collection
/// literal is built into a `__vdb_N`, not a record work-ref).
fn branch_arm_stores(v: &Value, out: &mut Vec<u16>) {
    match v.unspan() {
        Value::If(_, t, e) => {
            branch_arm_stores(t, out);
            branch_arm_stores(e, out);
        }
        Value::Block(bl) => {
            if let Some(tail @ Value::If(..)) = bl.operators.last().map(Value::unspan) {
                branch_arm_stores(tail, out);
            } else {
                for d in bl.result.depend() {
                    if !out.contains(&d) {
                        out.push(d);
                    }
                }
            }
        }
        _ => {}
    }
}

/// The stores capture `c` may hold when its value is a JOIN — every dep and the backing chain
/// behind it, and at each step a `??` temp's default-arm owners (`Function::join_owners`) —
/// or empty when it is not a join (one dep chain and no `??` temp on it), which
/// [`backs_an_adopted_capture`]'s single-store answer already covers.  loft#1721.
fn capture_join_candidates(function: &Function, c: u16) -> Vec<u16> {
    // loft#1726 — a sunk branch bind records its arms' stores on the capture itself.
    let mut join = function.tp(c).depend().len() > 1 || !function.join_owners(c).is_empty();
    let mut out = Vec::new();
    let mut work = function.tp(c).depend();
    work.extend_from_slice(function.join_owners(c));
    let mut seen = HashSet::default();
    while let Some(v) = work.pop() {
        if v == c || v == u16::MAX || function.is_argument(v) || !seen.insert(v) {
            continue;
        }
        out.push(v);
        let owners = function.join_owners(v);
        if !owners.is_empty() {
            join = true;
            work.extend_from_slice(owners);
        }
        work.extend(function.tp(v).depend());
    }
    if join { out } else { Vec::new() }
}

/// Is `v` the store behind a capture whose closure record ADOPTS it?
///
/// `get_free_vars` suppresses a captured local's scope-exit free by asking `is_captured` of
/// the local it is about to free — but a collection capture names a VIEW, and the local
/// holding the store is the backing one, which no closure captured by name. This asks the
/// question the other way round: does some captured local in this frame reach `v`?
///
/// ⚠ It gates on `frame_owns_capture_store`, the SAME predicate `record_adopts_capture`
/// uses, and that is the whole point. Suppressing the free and adopting the store have to be
/// one decision: suppress without adopting and the store is never freed at all, adopt without
/// suppressing and it is freed twice. An earlier cut answered them in two places — a parse-time
/// mark for the free and this pass for the verdict — and a capture the verdict called BORROWED
/// had already had its backing free suppressed, so it leaked
/// (`1248-a-capture-that-cannot-be-borrowed-from`). Asking one function keeps them from
/// disagreeing by construction.
pub(crate) fn backs_an_adopted_capture(
    data: &Data,
    function: &Function,
    built_with: &CaptureBuilds,
    v: u16,
) -> bool {
    (0..function.next_var()).any(|c| {
        c != v
            && function.is_captured(c)
            // @FR-O-Latest, the collection half of the same sentence — and only where the
            // record REBUILDS.  A build inside a loop rewrites its capture slot on every pass,
            // so the backing the FIRST pass adopted is not what the record holds at the end
            // and suppressing its free hands the store to an adoption two passes stale
            // (measured: `__vdb_1` never freed in a vector-capture loop).  A build that runs
            // ONCE is the opposite case and must keep its suppression however often the
            // capture is reassigned afterwards — that is #323's escaping factory, whose
            // record outlives the frame; declining there frees the store the escaped closure
            // still reads, which `1324-a-reassigned-capture-suppresses-the-store-the-record-\
            // holds` catches as `null(oob)`.
            && !built_with.rebuilt_in_loop.contains(&c)
            && capture_is_adopted(data, function, built_with, c)
            && match built_with.backing.get(&c) {
                // @FR-O-Latest — the record holds the store this capture named AT THE BUILD, so
                // that is the one local whose free it takes over.  Reading the type dep instead
                // aims the suppression at whatever the local names LAST, which for a capture
                // reassigned after the build is a different store: the adopted one is then freed
                // by the frame as well and an escaping closure reads a released store, while the
                // one the local now names is suppressed although nobody adopted it and leaks
                // (loft#1324).
                Some(&backing) => backing == v,
                // No build point in this body, or a right-hand side naming no single root: the
                // type dep is the only fact there is, and it is right whenever the local is
                // assigned once.
                // A JOIN capture's stores are released by identity instead
                // (`join_capture_witness`), so none of them is suppressed wholesale.
                // So is one whose value WAS a join at the build and was assigned since: its
                // type names the later store, which the record never held (loft#1725).
                None => {
                    backing_chain(function, c).contains(&v)
                        && capture_join_candidates(function, c).is_empty()
                        && !built_with.join_arms_at_build.contains_key(&c)
                }
            }
    })
}

/// Would [`mark_borrowed_captures`] ADOPT the capture named by local `c`?
///
/// Asked through the record rather than off `c` alone, because that pass declines captures
/// this frame nonetheless owns: its attribute filter admits only a `Reference` attribute with
/// non-empty deps — the cascade-relevant share marker — and a capture outside that class keeps
/// its frame-exit free. `test_a_store_backed_capture_still_declines_and_still_answers` is one
/// on purpose (loft#1248's minting capture, which still declines the lift), and reading only
/// `frame_owns_capture_store` here suppressed its free while the record declined to adopt it,
/// so the store leaked at program exit.
fn capture_is_adopted(data: &Data, function: &Function, builds: &CaptureBuilds, c: u16) -> bool {
    let name = function.name(c);
    for w in 0..function.next_var() {
        if function.is_argument(w) {
            continue;
        }
        let Type::Reference(record, _) = function.tp(w) else {
            continue;
        };
        let record = *record;
        if !data.def(record).name.starts_with("__closure_") {
            continue;
        }
        for a in 0..data.attributes(record) {
            if data.attr_name(record, a) != name {
                continue;
            }
            if !capture_attr_is_cascade_relevant(data, record, a) {
                return false;
            }
            return record_adopts_capture(data, function, builds, record, a);
        }
    }
    false
}

/// The locals that BACK `start`, nearest first — the chain `frame_owns_capture_store` walks
/// to reach the one that owns the store.
///
/// Empty when `start` owns its store directly, which is the struct case: there is nothing
/// behind it to mark.
/// Does the bind `v = rhs`, whose right-hand side is rooted at `root`, leave `v` VIEWING
/// the store `root` holds?
///
/// [`CaptureBuilds::backing`] names the local that HOLDS the store a capture reaches, and a
/// capture owning its store outright has none.  A right-hand side naming a single root
/// answers neither question on its own: `mb = cap` names `cap` and still mints `mb` a store
/// of its own, so recording `cap` as the backing suppressed a frame-exit free that the
/// closure's cascade never took over, and the copied-from record was freed by nobody
/// (loft#1487, one store per copy-bound capture, both backends).
///
/// `binding.md` is what separates the two, and it separates them by the SHAPE of the
/// right-hand side rather than by any fact about `v`: `(B-View)` makes a PROJECTION —
/// `q = __vdb_1[…]`, `x = b.s`, an element read — name an interior place, so the root keeps
/// holding the store; `(B-Copy)` makes a plain bind of a whole heap value COPY, so a bare
/// local read leaves `v` owning a store of its own.  `(B-Ref-Alias)` is the one bare-name
/// exception, and it says so in the destination's type.
///
/// ⚠ **Asked of `v`'s DEPS instead, this is the wrong currency and the reassignment cells
/// fail.** The map is built by a per-ASSIGNMENT walk because `@FR-O-Latest` needs the store
/// the capture named AT THE BUILD; a dep read describes what the local names LAST, so a
/// capture reassigned after its build loses the backing the record still holds and the
/// frame frees it under the escaped closure — `1324-a-reassigned-capture-suppresses-the-\
/// store-the-record-holds` reads `null(oob)` on exactly that.  The right-hand side is a
/// fact about this assignment and stays true however often the local is assigned again.
///
/// `@FR-L-CapOwn` — a captured heap store is freed once, by whichever of the record and the
/// frame outlives the other.  A store no capture reaches is not in that trade at all, so
/// naming it here takes away the frame's release without giving the cascade anything.
fn bind_views_root(function: &Function, v: u16, rhs: &Value, root: u16) -> bool {
    // `@FR-N-Shape` — through `base()`, because "is this destination a `&` LINK" is a shape
    // question and a `&τ?` links exactly as its dense twin does.
    let dest = function.tp(v).base();
    root != v
        && !function.is_argument(root)
        && (!rhs_is_a_bare_local_read(rhs) || matches!(dest, Type::RefVar(_)))
}

/// Is this right-hand side a bare read of another local — the `(B-Copy)` shape, as opposed
/// to a projection naming a place inside one?
///
/// Peels the wrappers a right-hand side arrives in and nothing else: a `Span` is a source
/// position, and the parser wraps a one-expression arm in a `Block` with a single operator.
fn rhs_is_a_bare_local_read(rhs: &Value) -> bool {
    match rhs.unspan() {
        Value::Var(_) => true,
        Value::Block(bl) if bl.operators.len() == 1 => rhs_is_a_bare_local_read(&bl.operators[0]),
        Value::Insert(ops) if ops.len() == 1 => rhs_is_a_bare_local_read(&ops[0]),
        _ => false,
    }
}

fn backing_chain(function: &Function, start: u16) -> Vec<u16> {
    let mut chain = Vec::new();
    let mut v = start;
    for _ in 0..8 {
        let dep = function.tp(v).depend();
        if dep.len() != 1 || dep[0] == v || function.is_argument(dep[0]) {
            break;
        }
        v = dep[0];
        chain.push(v);
    }
    chain
}

/// The captures whose closure build sits inside a LOOP.
///
/// A build that runs once adopts one store and keeps it; a build in a loop rewrites its
/// record's capture slot on every pass, so only the LAST adoption is the one the record still
/// holds and every earlier backing is the frame's to free again.
/// The capture variables whose closure BUILD may not run — it sits under an `if`, a `match` or
/// a loop body rather than on the straight line from the function's entry to its exit.
///
/// `@FR-L-CapOwn` — the frame gives its release up to the record's cascade, and a cascade that
/// never happens is not a release.  Over-approximating costs a runtime test where a static
/// suppression would have done; under-approximating strands the store, so a construct this walk
/// does not recognise must read as CONDITIONAL rather than as straight-line.
fn captures_built_conditionally(
    node: &Value,
    set_dbref: u32,
    branched: bool,
    out: &mut HashSet<u16>,
) {
    if branched
        && let Value::Call(d, args) = node.unspan()
        && *d == set_dbref
        && let Some(Value::Var(c)) = args.get(2).map(Value::unspan)
    {
        out.insert(*c);
    }
    match node.unspan() {
        // An `if`'s CONDITION is on the straight line; only the two arms are conditional.
        // `match` lowers to nested `If`, so it needs no arm of its own here.
        Value::If(cond, then, alt) => {
            captures_built_conditionally(cond, set_dbref, branched, out);
            captures_built_conditionally(then, set_dbref, true, out);
            captures_built_conditionally(alt, set_dbref, true, out);
        }
        // A statement SEQUENCE is walked in order, because an early `return` earlier in it
        // takes everything after it off the straight line.  `if p { return … } return …;`
        // builds the second closure at the block's top level — inside no arm, so the walk read
        // it as unconditional — while the arm's return is exactly what skips it.  The frame had
        // then given its release away to a record that run never built, and the capture leaked
        // (one store per call).  This is the same bound `@FR-L-CapOne`'s terminating-arm clause
        // draws from the other side: an arm that returns is exclusive with what follows it.
        //
        // Keyed on a `Return` appearing ANYWHERE in a preceding operator, which over-
        // approximates — a `return` inside a nested lambda's body would count, and a run-time
        // test would be emitted where a static suppression would have done.  That is the
        // direction this walk's own contract asks for: over-approximating costs a test,
        // under-approximating strands the store.
        Value::Block(bl) => {
            let mut br = branched;
            for op in &bl.operators {
                captures_built_conditionally(op, set_dbref, br, out);
                br = br || subtree_has_return(op);
            }
        }
        Value::Insert(ops) => {
            let mut br = branched;
            for op in ops {
                captures_built_conditionally(op, set_dbref, br, out);
                br = br || subtree_has_return(op);
            }
        }
        other => other.for_each_child(&mut |ch| {
            captures_built_conditionally(
                ch,
                set_dbref,
                branched || matches!(other, Value::Loop(_)),
                out,
            );
        }),
    }
}

/// Does `node` contain a `return` anywhere within it?
///
/// The signal that everything AFTER `node` in a statement sequence is conditional: a run that
/// takes that return never reaches them.  Answers about the whole subtree rather than its tail,
/// because the return that matters here is the one inside a branch arm — a tail return would end
/// the sequence anyway.
fn subtree_has_return(node: &Value) -> bool {
    let mut found = false;
    node.walk(&mut |n| {
        if matches!(n.unspan(), Value::Return(_)) {
            found = true;
        }
    });
    found
}

fn captures_built_in_a_loop(node: &Value, set_dbref: u32, in_loop: bool, out: &mut HashSet<u16>) {
    let inner = in_loop || matches!(node.unspan(), Value::Loop(_));
    if in_loop
        && let Value::Call(d, args) = node.unspan()
        && *d == set_dbref
        && let Some(Value::Var(c)) = args.get(2).map(Value::unspan)
    {
        out.insert(*c);
    }
    node.unspan()
        .for_each_child(&mut |ch| captures_built_in_a_loop(ch, set_dbref, inner, out));
}

/// Does the build that adopted `v`'s store sit in a branch?
///
/// Both spellings of "the record took this local's store", the pair [`CaptureBuilds`] carries:
/// a STRUCT capture names the local outright, a COLLECTION capture names a view whose backing
/// local is this one and which no closure captured by name.
pub(super) fn adoption_build_is_conditional(builds: &CaptureBuilds, v: u16) -> bool {
    builds.built_conditionally.contains(&v)
        || builds
            .backing
            .iter()
            .any(|(c, b)| *b == v && builds.built_conditionally.contains(c))
}

/// Release `v`'s store unless the closure record in `record` was built.
///
/// `@FR-L-CapOwn` — the store is freed ONCE, and where the build is conditional only the run
/// knows by whom.  The record local is the witness: `emit_lambda_code` inits it to the empty
/// slot and the build is the only thing that mints into it, so "a record is there" and "the
/// cascade that replaces this free will run" are the same fact.
///
/// "A record is there" refuses BOTH spellings of absent, the pair `build_into_return_buffer`
/// names for the same reason: the never-built slot (`rec == 0`, `OpConvBoolFromRef`) and the
/// freed one (`store_nr == u16::MAX`, `OpRefIsNull`).  `if is_null { false } else { has_rec }`
/// is how `&&` lowers.
///
/// `witness` is the local's OWNER WITNESS where it has one (`@FR-O-Witness`): the sweep
/// releases that separately, so the release here declines by store identity where the two
/// would name one store.  Without the witness the free is plain.
///
/// `hook` is the release's drop (`@FR-H-Drop`), run ahead of the free on the same path.
pub(super) fn free_unless_record_built(
    v: u16,
    records: &[u16],
    witness: Option<u16>,
    hook: Option<Value>,
    data: &Data,
) -> Value {
    // ANY of them being there is the fact: where several records can hold this store they are
    // mutually exclusive builds, so on a given run at most one exists — and that one's cascade
    // is the release this free stands down for.  Folded right so the last record is the base
    // case and each earlier one short-circuits to `true`, which is how `||` lowers.
    let present = records
        .iter()
        .rev()
        .map(|&record| {
            v_if(
                call("OpRefIsNull", record, data),
                Value::Boolean(false),
                call("OpConvBoolFromRef", record, data),
            )
        })
        .reduce(|acc, one| v_if(one, Value::Boolean(true), acc))
        .unwrap_or(Value::Boolean(false));
    let release = match witness {
        Some(w) => Value::Call(
            data.def_nr("OpFreeRefIfDistinct"),
            vec![Value::Var(v), Value::Var(w)],
        ),
        None => call("OpFreeRef", v, data),
    };
    let release = match hook {
        Some(hook) => Value::Insert(vec![hook, release]),
        None => release,
    };
    v_if(present, Value::Null, release)
}
